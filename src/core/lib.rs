// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

pub mod gyro_source;
pub mod imu_integration;
pub mod lens_profile;
pub mod lens_profile_database;
#[cfg(feature = "opencv")]
pub mod calibration;
pub mod synchronization;
pub mod stabilization;
pub mod camera_identifier;
pub mod keyframes;

pub mod zooming;
pub mod smoothing;
pub mod filtering;
pub mod filesystem;

pub mod gpu;

pub mod util;
pub mod stabilization_params;

use std::sync::{ Arc, atomic::{ AtomicU64, AtomicBool, Ordering::SeqCst } };
use std::collections::BTreeMap;
use keyframes::*;
use parking_lot::{ RwLock, RwLockUpgradableReadGuard };
use nalgebra::Vector4;
use gyro_source::{ GyroSource, Quat64, TimeQuat, TimeVec };
use stabilization_params::StabilizationParams;
use lens_profile::LensProfile;
use lens_profile_database::LensProfileDatabase;
use smoothing::Smoothing;
use stabilization::{ Stabilization, KernelParamsFlags, ComputeParams };
use camera_identifier::CameraIdentifier;
pub use stabilization::PixelType;
pub use wgpu::TextureFormat as WgpuTextureFormat;
use gpu::Buffers;
use gpu::drawing::*;

pub use telemetry_parser;

#[cfg(feature = "opencv")]
use calibration::LensCalibrator;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

lazy_static::lazy_static! {
    static ref THREAD_POOL: rayon::ThreadPool = rayon::ThreadPoolBuilder::new().build().unwrap();
}

#[derive(Default, Clone, Debug)]
pub struct InputFile {
    pub url: String,
    pub project_file_url: Option<String>,
    pub image_sequence_fps: f64,
    pub image_sequence_start: i32
}

pub struct StabilizationManager {
    pub gyro: Arc<RwLock<GyroSource>>,
    pub lens: Arc<RwLock<LensProfile>>,
    pub smoothing: Arc<RwLock<Smoothing>>,

    pub stabilization: Arc<RwLock<Stabilization>>,

    pub pose_estimator: Arc<synchronization::PoseEstimator>,
    #[cfg(feature = "opencv")]
    pub lens_calibrator: Arc<RwLock<Option<LensCalibrator>>>,

    pub current_compute_id: Arc<AtomicU64>,
    pub smoothing_checksum: Arc<AtomicU64>,
    pub zooming_checksum: Arc<AtomicU64>,
    pub prevent_recompute: Arc<AtomicBool>,

    pub camera_id: Arc<RwLock<Option<CameraIdentifier>>>,
    pub lens_profile_db: Arc<RwLock<LensProfileDatabase>>,

    pub input_file: Arc<RwLock<InputFile>>,

    pub keyframes: Arc<RwLock<KeyframeManager>>,

    pub params: Arc<RwLock<StabilizationParams>>
}

impl Default for StabilizationManager {
    fn default() -> Self {
        std::env::set_var("IS_GYROFLOW", "1");
        Self {
            smoothing: Arc::new(RwLock::new(Smoothing::default())),

            params: Arc::new(RwLock::new(StabilizationParams::default())),

            stabilization: Arc::new(RwLock::new(Stabilization::default())),
            gyro: Arc::new(RwLock::new(GyroSource::new())),
            lens: Arc::new(RwLock::new(LensProfile::default())),

            current_compute_id: Arc::new(AtomicU64::new(0)),
            smoothing_checksum: Arc::new(AtomicU64::new(0)),
            zooming_checksum: Arc::new(AtomicU64::new(0)),
            prevent_recompute: Arc::new(AtomicBool::new(false)),

            pose_estimator: Arc::new(synchronization::PoseEstimator::default()),

            lens_profile_db: Arc::new(RwLock::new(LensProfileDatabase::default())),

            input_file: Arc::new(RwLock::new(InputFile::default())),

            #[cfg(feature = "opencv")]
            lens_calibrator: Arc::new(RwLock::new(None)),

            keyframes: Arc::new(RwLock::new(KeyframeManager::new())),

            camera_id: Arc::new(RwLock::new(None)),
        }
    }
}

impl StabilizationManager {
    pub fn init_from_video_data(&self, duration_ms: f64, fps: f64, frame_count: usize, video_size: (usize, usize)) {
        {
            let mut params = self.params.write();
            params.fps = fps;
            params.frame_count = frame_count;
            params.duration_ms = duration_ms;
            params.video_size = video_size;
        }

        self.pose_estimator.sync_results.write().clear();
        self.keyframes.write().clear();
    }

    pub fn load_gyro_data<F: Fn(f64)>(&self, url: &str, is_main_video: bool, options: &gyro_source::FileLoadOptions, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> std::result::Result<(), GyroflowCoreError> {
        {
            let params = self.params.read();
            let mut gyro = self.gyro.write();
            gyro.init_from_params(&params);
            gyro.clear();
            gyro.file_url = url.to_string();
            gyro.file_metadata = Default::default();
        }
        self.invalidate_smoothing();
        self.invalidate_zooming();

        let last_progress = std::cell::RefCell::new(std::time::Instant::now());
        let progress_cb = |p| {
            let now = std::time::Instant::now();
            if (now - *last_progress.borrow()).as_millis() > 100 {
                progress_cb(p);
                *last_progress.borrow_mut() = now;
            }
        };

        let (fps, size) = {
            let params = self.params.read();
            (params.fps, params.video_size)
        };

        let cancel_flag2 = cancel_flag.clone();
        let mut md = GyroSource::parse_telemetry_file(url, options, size, fps, progress_cb, cancel_flag2)?;
        if md.detected_source.as_ref().map(|v| v.starts_with("GoPro ")).unwrap_or_default() {
            // If gopro reports rolling shutter value, it already applied it, ie. the video is already corrected
            md.frame_readout_time = None;
        }
        if is_main_video {
            if let Some(ref lens) = md.lens_profile {
                let mut l = self.lens.write();
                if let Some(lens_str) = lens.as_str() {
                    let mut db = self.lens_profile_db.read();
                    if !db.loaded {
                        drop(db);
                        {
                            let mut db = self.lens_profile_db.write();
                            db.load_all();
                        }
                        db = self.lens_profile_db.read();
                    }
                    if let Some(found) = db.find(lens_str) {
                        *l = found.clone();
                    }
                } else if lens.is_object() {
                    l.load_from_json_value(lens);
                    l.path_to_file = filesystem::url_to_path(url);
                    let db = self.lens_profile_db.read();
                    l.resolve_interpolations(&db);
                }
            }
            if let Some(md_fps) = md.frame_rate {
                let fps = self.params.read().fps;
                if (md_fps - fps).abs() > 1.0 {
                    self.override_video_fps(md_fps, false);
                }
            }
            if md.detected_source.as_ref().map(|v| v.starts_with("Blackmagic ")).unwrap_or_default() {
                if let Some(rot) = md.additional_data.get("rotation").and_then(|x| x.as_u64()) {
                    if rot == 90 || rot == 270 {
                        log::info!("Using horizontal rolling shutter correction");
                        self.params.write().horizontal_rs = true;
                        if rot == 90 {
                            md.imu_orientation = Some("xYz".into());
                            md.frame_readout_time = md.frame_readout_time.map(|x| -x);
                        } else {
                            md.imu_orientation = Some("Xyz".into());
                        }
                    }
                    if rot == 180 {
                        md.frame_readout_time = md.frame_readout_time.map(|x| -x);
                        md.imu_orientation = Some("YXz".into());
                    }
                }
            }
            self.params.write().frame_readout_time = md.frame_readout_time.unwrap_or_default();
        } else {
            log::info!("Not a main video, clearing {} per-frame offsets", md.per_frame_time_offsets.len());
            md.per_frame_time_offsets.clear();
        }
        let camera_id = md.camera_identifier.clone();
        if !cancel_flag.load(SeqCst) {
            let mut gyro = self.gyro.write();
            gyro.load_from_telemetry(md);
            gyro.file_load_options = options.clone();
        }

        if let Some(id) = camera_id {
            *self.camera_id.write() = Some(id);
        }
        Ok(())
    }

    pub fn load_lens_profile(&self, url: &str) -> Result<(), crate::GyroflowCoreError> {
        let url = if (url.starts_with('/') || url.starts_with('\\') || (url.len() > 3 && &url[1..2] == ":")) && !url.contains("://") && !url.starts_with('{') {
            crate::filesystem::path_to_url(url)
        } else {
            url.to_owned()
        };
        let db = self.lens_profile_db.read();
        let (result, from_db) = if let Some(lens) = db.get_by_id(&url) {
            *self.lens.write() = lens.clone();
            (Ok(()), true)
        } else if url.starts_with('{') {
            (self.lens.write().load_from_data(&url), false)
        } else {
            (self.lens.write().load_from_file(&url), false)
        };
        let (width, height, aspect, id, fps) = {
            let params = self.params.read();
            (params.video_size.0, params.video_size.1, ((params.video_size.0 * 100) as f64 / params.video_size.1.max(1) as f64).round() as u32, self.camera_id.read().as_ref().map(|x| x.get_identifier_for_autoload()).unwrap_or_default(), (params.fps * 100.0).round() as i32)
        };

        let mut lens = self.lens.write();

        // Check if the lens profile needs to be swapped for vertical
        let lens_aspect_swapped = ((lens.calib_dimension.h * 100) as f64 / lens.calib_dimension.w.max(1) as f64).round() as u32;
        if (width == lens.calib_dimension.h && height == lens.calib_dimension.w) || lens_aspect_swapped == aspect {
            log::info!("Lens profile swapped from {}x{} to {}x{} to match the video aspect", lens.calib_dimension.w, lens.calib_dimension.h, lens.calib_dimension.h, lens.calib_dimension.w);
            *lens = lens.swapped();
        }

        let matching = lens.get_all_matching_profiles();
        if matching.len() > 1 {
            let mut found = false;
            if !id.is_empty() && lens.identifier == id {
                found = true;
            }
            // Find best match for:
            if !found {
                // 1. Identifier
                for x in &matching {
                    if !id.is_empty() && x.identifier == id {
                        *lens = x.clone(); found = true; break;
                    }
                }
            }
            if !found {
                // 2. Resolution and fps
                for x in &matching {
                    if width == x.calib_dimension.w && height == x.calib_dimension.h && fps == (x.fps * 100.0).round() as i32 {
                        *lens = x.clone(); found = true; break;
                    }
                }
            }
            if !found {
                // 3. Aspect ratio and fps
                for x in &matching {
                    let a = ((x.calib_dimension.w * 100) as f64 / x.calib_dimension.h.max(1) as f64).round() as u32;
                    if a == aspect && fps == (x.fps * 100.0).round() as i32 {
                        *lens = x.clone(); break;
                    }
                }
            }
            if !found {
                // 4. Resolution
                for x in &matching {
                    if width == x.calib_dimension.w && height == x.calib_dimension.h {
                        *lens = x.clone(); found = true; break;
                    }
                }
            }
            if !found {
                // 5. Aspect ratio
                for x in &matching {
                    let a = ((x.calib_dimension.w * 100) as f64 / x.calib_dimension.h.max(1) as f64).round() as u32;
                    if a == aspect {
                        *lens = x.clone(); break;
                    }
                }
            }
        }
        if !from_db {
            lens.resolve_interpolations(&db);
        }
        result
    }

    fn init_size(&self) {
        let (w, h, ow, oh) = {
            let params = self.params.read();
            (params.size.0, params.size.1, params.output_size.0, params.output_size.1)
        };

        if w > 0 && ow > 0 && h > 0 && oh > 0 {
            self.stabilization.write().init_size((w, h), (ow, oh));
            self.lens.write().optimal_fov = None;

            self.invalidate_smoothing();
        }
    }

    pub fn set_size(&self, width: usize, height: usize) {
        {
            let mut params = self.params.write();
            params.size = (width, height);

            let ratio = params.size.0 as f64 / params.video_output_size.0 as f64;
            params.output_size = ((params.video_output_size.0 as f64 * ratio) as usize, (params.video_output_size.1 as f64 * ratio) as usize);
        }
        self.init_size();
    }
    pub fn set_output_size(&self, width: usize, height: usize) -> bool {
        if width > 0 && height > 0 {
            let params = self.params.upgradable_read();

            let ratio = params.size.0 as f64 / width as f64;
            let output_size = ((width as f64 * ratio) as usize, (height as f64 * ratio) as usize);
            let video_output_size = (width, height);

            if params.output_size != output_size || params.video_output_size != video_output_size {
                {
                    let mut params = RwLockUpgradableReadGuard::upgrade(params);
                    params.output_size = output_size;
                    params.video_output_size = video_output_size;
                }
                self.init_size();

                return true;
            }
        }
        false
    }

    pub fn recompute_adaptive_zoom_static(compute_params: &ComputeParams, params: &RwLock<StabilizationParams>, keyframes: &KeyframeManager) -> (Vec<f64>, Vec<f64>, BTreeMap<i64, Vec<(f64, f64)>>) {
        let (frames, fps, method) = {
            let params = params.read();
            (params.frame_count, params.get_scaled_fps(), params.adaptive_zoom_method)
        };
        let timestamps = (0..frames).map(|i| i as f64 * 1000.0 / fps).collect::<Vec<f64>>();

        zooming::calculate_fovs(compute_params, &timestamps, &keyframes, method.into())
    }
    pub fn recompute_adaptive_zoom(&self) {
        let params = stabilization::ComputeParams::from_manager(self);
        let lens_fov_adjustment = params.lens.optimal_fov.unwrap_or(1.0);
        let (fovs, minimal_fovs, debug_points) = Self::recompute_adaptive_zoom_static(&params, &self.params, &self.keyframes.read());

        let mut stab_params = self.params.write();
        stab_params.set_fovs(fovs, lens_fov_adjustment);
        stab_params.minimal_fovs = minimal_fovs;
        stab_params.zooming_debug_points = debug_points;
    }

    pub fn recompute_smoothness(&self) {
        let params = self.params.read();
        let keyframes = self.keyframes.read().clone();
        let smoothing = self.smoothing.read();
        let horizon_lock = smoothing.horizon_lock.clone();

        let (quats, org_quats, max_angles) = self.gyro.read().recompute_smoothness(smoothing.current().as_ref(), horizon_lock, &params, &keyframes);
        let mut gyro = self.gyro.write();
        gyro.max_angles = max_angles;
        gyro.org_smoothed_quaternions = org_quats;
        gyro.smoothed_quaternions = quats;
    }

    pub fn recompute_undistortion(&self) {
        let params = stabilization::ComputeParams::from_manager(self);
        self.stabilization.write().set_compute_params(params);
    }

    pub fn recompute_blocking(&self) {
        self.recompute_smoothness();
        self.recompute_adaptive_zoom();
        self.recompute_undistortion();
    }

    pub fn invalidate_ongoing_computations(&self) {
        self.current_compute_id.store(fastrand::u64(..), SeqCst);
    }

    pub fn recompute_threaded<F: Fn((u64, bool)) + Send + Sync + Clone + 'static>(&self, cb: F) -> u64 {
        //self.recompute_smoothness();
        //self.recompute_adaptive_zoom();
        let mut params = stabilization::ComputeParams::from_manager(self);

        let smoothing = self.smoothing.clone();
        let stabilization_params = self.params.clone();
        let keyframes = self.keyframes.read().clone();
        let gyro = self.gyro.clone();

        let compute_id = fastrand::u64(..);
        self.current_compute_id.store(compute_id, SeqCst);

        let mut gyro_checksum = gyro.read().get_checksum();

        let prevent_recompute = self.prevent_recompute.clone();
        let current_compute_id = self.current_compute_id.clone();
        let smoothing_checksum = self.smoothing_checksum.clone();
        let zooming_checksum = self.zooming_checksum.clone();

        let stabilization = self.stabilization.clone();
        THREAD_POOL.spawn(move || {
            // std::thread::sleep(std::time::Duration::from_millis(20));
            if prevent_recompute.load(SeqCst) { return cb((compute_id, true)); } // we're still loading, don't recompute
            if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

            let mut smoothing_changed = false;
            if smoothing.read().get_state_checksum(gyro_checksum) != smoothing_checksum.load(SeqCst) {
                let (mut smoothing, horizon_lock) = {
                    let lock = smoothing.read();
                    (lock.current().clone(), lock.horizon_lock.clone())
                };
                let (quats, org_quats, max_angles) = gyro.read().recompute_smoothness(smoothing.as_mut(), horizon_lock, &stabilization_params.read(), &keyframes);

                if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }
                if gyro_checksum != gyro.read().get_checksum() { return cb((compute_id, true)); }

                let mut lib_gyro = gyro.write();
                lib_gyro.max_angles = max_angles;
                lib_gyro.org_smoothed_quaternions = org_quats;
                lib_gyro.smoothed_quaternions = quats;
                lib_gyro.smoothing_status = smoothing.get_status_json();
                gyro_checksum = lib_gyro.get_checksum();
                smoothing_changed = true;
            }
            smoothing_checksum.store(smoothing.read().get_state_checksum(gyro_checksum), SeqCst);

            if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

            if smoothing_changed || zooming::get_checksum(&params) != zooming_checksum.load(SeqCst) {
                let (fovs, minimal_fovs, debug_points) = Self::recompute_adaptive_zoom_static(&params, &stabilization_params, &keyframes);
                params.fovs = fovs;
                params.minimal_fovs = minimal_fovs;

                if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

                let mut stab_params = stabilization_params.write();
                stab_params.set_fovs(params.fovs.clone(), params.lens.optimal_fov.unwrap_or(1.0));
                stab_params.minimal_fovs = params.minimal_fovs.clone();
                stab_params.zooming_debug_points = debug_points;
                zooming_checksum.store(zooming::get_checksum(&params), SeqCst);
            }

            if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

            stabilization.write().set_compute_params(params);

            cb((compute_id, false));
        });
        compute_id
    }

    pub fn get_features_pixels(&self, timestamp_us: i64, size: (usize, usize)) -> Option<Vec<(i32, i32)>> { // (x, y, alpha)
        let mut ret = None;
        use crate::util::MapClosest;
        use synchronization::OpticalFlowTrait;

        if let Some(l) = self.pose_estimator.sync_results.try_read() {
            if let Some(entry) = l.get_closest(&timestamp_us, 2000) { // closest within 2ms
                let ratio = size.1 as f32 / entry.frame_size.1.max(1) as f32;
                for pt in entry.of_method.features() {
                    if ret.is_none() {
                        // Only allocate if we actually have any points
                        ret = Some(Vec::with_capacity(2048));
                    }
                    ret.as_mut().unwrap().push(((pt.0 * ratio) as i32, (pt.1 * ratio) as i32));
                }
            }
        }
        ret
    }
    pub fn get_opticalflow_pixels(&self, timestamp_us: i64, num_frames: usize, size: (usize, usize)) -> Option<Vec<(i32, i32, usize)>> { // (x, y, alpha)
        let mut ret = None;
        for i in 0..num_frames {
            match self.pose_estimator.get_of_lines_for_timestamp(&timestamp_us, i, 1.0, 1, false) {
                (Some(lines), Some(frame_size)) => {
                    let ratio = size.1 as f32 / frame_size.1.max(1) as f32;
                    lines.0.1.into_iter().zip(lines.1.1.into_iter()).for_each(|(p1, p2)| {
                        if ret.is_none() {
                            // Only allocate if we actually have any points
                            ret = Some(Vec::with_capacity(2048));
                        }
                        let line = line_drawing::Bresenham::new(((p1.0 * ratio) as isize, (p1.1 * ratio) as isize), ((p2.0 * ratio) as isize, (p2.1 * ratio) as isize));
                        for point in line {
                            ret.as_mut().unwrap().push((point.0 as i32, point.1 as i32, i));
                        }
                    });
                }
                _ => { }
            }
        }
        ret
    }

    pub fn draw_overlays(&self, drawing: &mut DrawCanvas, timestamp_us: i64) {
        drawing.clear();

        if let Some(p) = self.params.try_read() {
            let y_inverted = p.framebuffer_inverted;
            let size = p.size;
            let frame = frame_at_timestamp(timestamp_us as f64 / 1000.0, p.get_scaled_fps()) as usize; // used only to draw features and OF

            if p.show_optical_flow {
                let num_frames = if p.of_method == 2 { 1 } else { 3 };
                if let Some(pxs) = self.get_opticalflow_pixels(timestamp_us, num_frames, size) {
                    for (x, y, a) in pxs {
                        let a = Alpha::from(a as u8);
                        drawing.put_pixel(x, y, Color::Yellow, a, Stage::OnInput, y_inverted, 1);
                    }
                }
            }
            if p.show_detected_features {
                if let Some(pxs) = self.get_features_pixels(timestamp_us, size) {
                    for (x, y) in pxs {
                        drawing.put_pixel(x, y, Color::Green, Alpha::Alpha100, Stage::OnInput, y_inverted, 3);
                    }
                }
            }
            #[cfg(feature = "opencv")]
            if p.is_calibrator {
                let lock = self.lens_calibrator.read();
                if let Some(ref cal) = *lock {
                    let points = cal.all_matches.read();
                    if let Some(entry) = points.get(&(frame as i32)) {
                        calibration::drawing::draw_chessboard_corners(cal.width, cal.height, p.size.0, p.size.1, drawing, (cal.columns, cal.rows), &entry.points, true, y_inverted);
                    }
                }
            }
            if !p.zooming_debug_points.is_empty() {
                if let Some((_, points)) = p.zooming_debug_points.range(timestamp_us..).next() {
                    for i in 0..points.len() {
                        let fov = ((p.fov + if p.fov_overview { 1.0 } else { 0.0 }) * p.fovs.get(frame).unwrap_or(&1.0)).max(0.0001);
                        let mut pt = points[i];
                        let width_ratio = p.size.0 as f64 / p.output_size.0 as f64;
                        let height_ratio = p.size.1 as f64 / p.output_size.1 as f64;
                        pt = (pt.0 - 0.5, pt.1 - 0.5);
                        pt = (pt.0 / fov * width_ratio, pt.1 / fov * height_ratio);
                        pt = (pt.0 + 0.5, pt.1 + 0.5);
                        if pt.0 >= 0.0 && pt.1 >= 0.0 {
                            drawing.put_pixel((pt.0 * p.output_size.0 as f64) as i32, (pt.1 * p.output_size.1 as f64) as i32, Color::Red, Alpha::Alpha100, Stage::OnOutput, y_inverted, 4);
                        }
                    }
                }
            }
        }
    }

    pub fn process_pixels<T: PixelType>(&self, mut timestamp_us: i64, buffers: &mut Buffers) -> Result<stabilization::ProcessedInfo, GyroflowCoreError> {
        if let gpu::BufferSource::Cpu { buffer } = &buffers.input.data  { if buffer.is_empty() { return Err(GyroflowCoreError::InputBufferEmpty); } }
        if let gpu::BufferSource::Cpu { buffer } = &buffers.output.data { if buffer.is_empty() { return Err(GyroflowCoreError::OutputBufferEmpty); } }

        if let Some(scale) = self.params.read().fps_scale {
            timestamp_us = (timestamp_us as f64 / scale).round() as i64;
        }

        {
            let mut undist = self.stabilization.write();
            self.draw_overlays(&mut undist.drawing, timestamp_us);
            undist.ensure_ready_for_processing::<T>(timestamp_us, buffers);
        }

        let undist = self.stabilization.read();
        undist.process_pixels::<T>(timestamp_us, buffers, None)
    }

    pub fn set_video_rotation(&self, v: f64) { self.params.write().video_rotation = v; self.invalidate_smoothing(); }

    pub fn set_trim_start(&self, v: f64) { self.params.write().trim_start = v; self.invalidate_smoothing(); }
    pub fn set_trim_end  (&self, v: f64) { self.params.write().trim_end   = v; self.invalidate_smoothing(); }

    pub fn set_of_method(&self, v: u32) { self.params.write().of_method = v; self.pose_estimator.clear(); }
    pub fn set_show_detected_features(&self, v: bool) { self.params.write().show_detected_features = v; }
    pub fn set_show_optical_flow     (&self, v: bool) { self.params.write().show_optical_flow      = v; }
    pub fn set_stab_enabled          (&self, v: bool) { self.params.write().stab_enabled           = v; }
    pub fn set_frame_readout_time    (&self, v: f64)  { self.params.write().frame_readout_time     = v; }
    pub fn set_adaptive_zoom         (&self, v: f64)  { self.params.write().adaptive_zoom_window   = v; self.invalidate_zooming(); }
    pub fn set_zooming_center_x      (&self, v: f64)  { self.params.write().adaptive_zoom_center_offset.0 = v; self.invalidate_zooming(); }
    pub fn set_zooming_center_y      (&self, v: f64)  { self.params.write().adaptive_zoom_center_offset.1 = v; self.invalidate_zooming(); }
    pub fn set_zooming_method        (&self, v: i32)  { self.params.write().adaptive_zoom_method   = v;        self.invalidate_zooming(); }
    pub fn set_fov                   (&self, v: f64)  { self.params.write().fov                    = v; }
    pub fn set_fov_overview          (&self, v: bool) { self.params.write().fov_overview           = v; }
    pub fn set_show_safe_area        (&self, v: bool) { self.params.write().show_safe_area         = v; }
    pub fn set_lens_correction_amount(&self, v: f64)  { self.params.write().lens_correction_amount = v; self.invalidate_zooming(); }
    pub fn set_background_color      (&self, bg: Vector4<f32>) { self.params.write().background = bg; }
    pub fn set_background_mode       (&self, v: i32)  { self.params.write().background_mode = stabilization_params::BackgroundMode::from(v); }
    pub fn set_background_margin     (&self, v: f64)  { self.params.write().background_margin = v; }
    pub fn set_background_margin_feather(&self, v: f64) { self.params.write().background_margin_feather = v; }
    pub fn set_input_horizontal_stretch (&self, v: f64) { self.lens.write().input_horizontal_stretch = v; self.invalidate_zooming(); }
    pub fn set_input_vertical_stretch   (&self, v: f64) { self.lens.write().input_vertical_stretch   = v; self.invalidate_zooming(); }

    pub fn set_video_speed(&self, v: f64, link_with_smoothness: bool, link_with_zooming: bool) {
        let mut params = self.params.write();
        params.video_speed = v;
        params.video_speed_affects_smoothing = link_with_smoothness;
        params.video_speed_affects_zooming = link_with_zooming;
        self.invalidate_smoothing();
    }

    pub fn disable_lens_stretch(&self) {
        let (x_stretch, y_stretch) = {
            let lens = self.lens.read();
            (lens.input_horizontal_stretch, lens.input_vertical_stretch)
        };
        if (x_stretch > 0.01 && x_stretch != 1.0) || (y_stretch > 0.01 && y_stretch != 1.0) {
            {
                let mut params = self.params.write();
                params.video_size.0 = (params.video_size.0 as f64 * x_stretch).round() as usize;
                params.video_size.1 = (params.video_size.1 as f64 * y_stretch).round() as usize;
            }
            {
                let mut lens = self.lens.write();
                lens.input_horizontal_stretch = 1.0;
                lens.input_vertical_stretch = 1.0;
            }
        }
    }

    pub fn get_scaling_ratio(&self) -> f64 { let params = self.params.read(); params.video_size.0 as f64 / params.video_output_size.0 as f64 }
    pub fn get_min_fov      (&self) -> f64 { self.params.read().min_fov }

    pub fn invalidate_smoothing(&self) { self.invalidate_ongoing_computations(); self.smoothing_checksum.store(0, SeqCst); self.invalidate_zooming(); }
    pub fn invalidate_zooming(&self) { self.invalidate_ongoing_computations(); self.zooming_checksum.store(0, SeqCst); }

    pub fn set_digital_lens_name(&self, v: String) {
        self.lens.write().digital_lens =  if !v.is_empty() { Some(v.clone()) } else { None };
        #[cfg(feature = "opencv")]
        if let Some(ref mut calib) = *self.lens_calibrator.write() {
            calib.digital_lens = if !v.is_empty() { Some(v) } else { None };
        }
        self.invalidate_zooming();
    }
    pub fn set_digital_lens_param(&self, index: usize, value: f64) {
        let mut lens = self.lens.write();
        if lens.digital_lens_params.is_none() {
            lens.digital_lens_params = Some(vec![0f64; 4]);
        }
        lens.digital_lens_params.as_mut().unwrap()[index] = value;
        #[cfg(feature = "opencv")]
        if let Some(ref mut calib) = *self.lens_calibrator.write() {
            calib.digital_lens_params = lens.digital_lens_params.clone();
        }
        self.invalidate_zooming();
    }
    pub fn set_lens_is_asymmetrical(&self, v: bool) {
        self.lens.write().asymmetrical = v;
        #[cfg(feature = "opencv")]
        if let Some(ref mut calib) = *self.lens_calibrator.write() {
            calib.asymmetrical = v;
        }
        self.invalidate_zooming();
    }

    pub fn remove_offset(&self, timestamp_us: i64) {
        self.gyro.write().remove_offset(timestamp_us);
        self.keyframes.write().update_gyro(&self.gyro.read());
        self.invalidate_zooming();
    }
    pub fn set_offset(&self, timestamp_us: i64, offset_ms: f64) {
        self.gyro.write().set_offset(timestamp_us, offset_ms);
        self.keyframes.write().update_gyro(&self.gyro.read());
        self.invalidate_zooming();
    }
    pub fn clear_offsets(&self) {
        self.gyro.write().clear_offsets();
        self.keyframes.write().update_gyro(&self.gyro.read());
        self.invalidate_zooming();
    }
    pub fn offset_at_video_timestamp(&self, timestamp_us: i64) -> f64 {
        self.gyro.read().offset_at_video_timestamp(timestamp_us as f64 / 1000.0)
    }

    pub fn set_imu_lpf(&self, lpf: f64) {
        self.gyro.write().imu_lpf = lpf;
    }
    pub fn set_imu_rotation(&self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64) {
        self.gyro.write().imu_rotation_angles = Some([pitch_deg, roll_deg, yaw_deg]);
    }
    pub fn set_acc_rotation(&self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64) {
        self.gyro.write().acc_rotation_angles = Some([pitch_deg, roll_deg, yaw_deg]);
    }
    pub fn set_imu_orientation(&self, orientation: String) {
        self.gyro.write().imu_orientation = Some(orientation);
    }
    pub fn set_imu_bias(&self, bx: f64, by: f64, bz: f64) {
        self.gyro.write().gyro_bias = Some([bx, by, bz]);
    }
    pub fn recompute_gyro(&self) {
        self.gyro.write().apply_transforms();
        self.invalidate_smoothing();
    }
    pub fn set_sync_lpf(&self, lpf: f64) {
        let params = self.params.read();
        self.pose_estimator.lowpass_filter(lpf, params.fps);
    }

    pub fn set_lens_param(&self, param: &str, value: f64) {
        let mut lens = self.lens.write();
        if lens.fisheye_params.distortion_coeffs.len() >= 4 &&
           lens.fisheye_params.camera_matrix.len() == 3 &&
           lens.fisheye_params.camera_matrix[0].len() == 3 &&
           lens.fisheye_params.camera_matrix[1].len() == 3 &&
           lens.fisheye_params.camera_matrix[2].len() == 3 {
            match param {
                "fx" => lens.fisheye_params.camera_matrix[0][0] = value,
                "fy" => lens.fisheye_params.camera_matrix[1][1] = value,
                "cx" => lens.fisheye_params.camera_matrix[0][2] = value,
                "cy" => lens.fisheye_params.camera_matrix[1][2] = value,
                "k1" => lens.fisheye_params.distortion_coeffs[0] = value,
                "k2" => lens.fisheye_params.distortion_coeffs[1] = value,
                "k3" => lens.fisheye_params.distortion_coeffs[2] = value,
                "k4" => lens.fisheye_params.distortion_coeffs[3] = value,
                "r_limit" => {
                    #[cfg(feature = "opencv")]
                    if let Some(ref mut calib) = *self.lens_calibrator.write() {
                        calib.r_limit = value;
                    }
                    lens.fisheye_params.radial_distortion_limit = if value > 0.0 { Some(value) } else { None };
                }
                _ => { }
            }
        }
    }

    pub fn set_smoothing_method(&self, index: usize) -> serde_json::Value {
        let mut smooth = self.smoothing.write();
        smooth.set_current(index);

        self.invalidate_smoothing();

        smooth.current().get_parameters_json()
    }
    pub fn set_smoothing_param(&self, name: &str, val: f64) {
        self.smoothing.write().current_mut().as_mut().set_parameter(name, val);
        self.invalidate_smoothing();
    }
    pub fn set_horizon_lock(&self, lock_percent: f64, roll: f64) {
        self.smoothing.write().horizon_lock.set_horizon(lock_percent, roll);
        self.invalidate_smoothing();
    }
    pub fn set_use_gravity_vectors(&self, v: bool) {
        self.gyro.write().set_use_gravity_vectors(v);
        self.invalidate_smoothing();
    }
    pub fn set_horizon_lock_integration_method(&self, v: i32) {
        self.gyro.write().set_horizon_lock_integration_method(v);
        self.invalidate_smoothing();
    }
    pub fn get_smoothing_max_angles(&self) -> (f64, f64, f64) {
        self.gyro.read().max_angles
    }
    pub fn get_smoothing_status(&self) -> serde_json::Value {
        self.gyro.read().smoothing_status.clone()
    }
    pub fn get_smoothing_algs(&self) -> Vec<String> {
        self.smoothing.read().get_names()
    }

    pub fn get_cloned(&self) -> StabilizationManager {
        StabilizationManager {
            params: Arc::new(RwLock::new(self.params.read().clone())),
            gyro:   Arc::new(RwLock::new(self.gyro.read().clone())),
            lens:   Arc::new(RwLock::new(self.lens.read().clone())),
            keyframes:  Arc::new(RwLock::new(self.keyframes.read().clone())),
            smoothing:  Arc::new(RwLock::new(self.smoothing.read().clone())),
            input_file: Arc::new(RwLock::new(self.input_file.read().clone())),
            lens_profile_db: self.lens_profile_db.clone(),

            // NOT cloned:
            // stabilization
            // pose_estimator
            // lens_calibrator
            // current_compute_id
            // smoothing_checksum
            // zooming_checksum
            // prevent_recompute
            // camera_id
            ..Default::default()
        }
    }
    pub fn set_render_params(&self, size: (usize, usize), output_size: (usize, usize)) {
        self.params.write().framebuffer_inverted = false;
        self.params.write().fov_overview = false;
        self.params.write().show_safe_area = false;
        self.stabilization.write().kernel_flags.set(KernelParamsFlags::DRAWING_ENABLED, false);
        self.set_size(size.0, size.1);
        self.set_output_size(output_size.0, output_size.1);

        self.recompute_undistortion();
    }

    pub fn clear(&self) {
        self.params.write().clear();
        self.invalidate_ongoing_computations();
        self.invalidate_smoothing();
        *self.input_file.write() = InputFile::default();
        *self.camera_id.write() = None;

        *self.gyro.write() = GyroSource::new();
        self.keyframes.write().clear();

        self.pose_estimator.clear();
    }

    pub fn override_video_fps(&self, fps: f64, recompute: bool) {
        {
            let mut params = self.params.write();
            if (fps - params.fps).abs() > 0.001 {
                params.fps_scale = Some(fps / params.fps);
            } else {
                params.fps_scale = None;
            }
            self.gyro.write().init_from_params(&params);
            self.keyframes.write().timestamp_scale = params.fps_scale;
        }

        if recompute {
            self.stabilization.write().set_compute_params(stabilization::ComputeParams::from_manager(self));

            self.invalidate_smoothing();
        }
    }

    pub fn list_gpu_devices<F: Fn(Vec<String>) + Send + Sync + 'static>(&self, cb: F) {
        let stab = self.stabilization.clone();
        run_threaded(move || {
            let list = stab.read().list_devices();

            log::info!("GPU list: {:?}", &list);

            *stabilization::GPU_LIST.write() = list.clone();

            cb(list);
        });
    }

    pub fn export_gyroflow_file(&self, url: &str, typ: GyroflowProjectType, additional_data: &str) -> Result<(), GyroflowCoreError> {
        let data = self.export_gyroflow_data(typ, additional_data, Some(url))?;
        filesystem::write(url, data.as_bytes())?;

        self.input_file.write().project_file_url = Some(url.to_string());

        Ok(())
    }
    pub fn export_gyroflow_data(&self, typ: GyroflowProjectType, additional_data: &str, _project_url: Option<&str>) -> Result<String, GyroflowCoreError> {
        let gyro = self.gyro.read();
        let params = self.params.read();

        let (smoothing_name, smoothing_params, horizon_amount, horizon_roll) = {
            let smoothing_lock = self.smoothing.read();
            let smoothing = smoothing_lock.current();

            let mut parameters = smoothing.get_parameters_json();
            if let serde_json::Value::Array(ref mut arr) = parameters {
                for v in arr.iter_mut() {
                    if let serde_json::Value::Object(ref obj) = v {
                        *v = serde_json::json!({
                            "name": obj["name"],
                            "value": obj["value"]
                        });
                    }
                }
            }
            let mut horizon_amount = smoothing_lock.horizon_lock.horizonlockpercent;
            if !smoothing_lock.horizon_lock.lock_enabled {
                horizon_amount = 0.0;
            }

            (smoothing.get_name(), parameters, horizon_amount, smoothing_lock.horizon_lock.horizonroll)
        };

        let input_file = self.input_file.read().clone();

        let mut obj = serde_json::json!({
            "title": "Gyroflow data file",
            "version": 3,
            "app_version": env!("CARGO_PKG_VERSION").to_string(),
            "videofile": input_file.url,
            "calibration_data": self.lens.read().get_json_value().unwrap_or_else(|_| serde_json::json!({})),
            "date": time::OffsetDateTime::now_local().map(|v| v.date().to_string()).unwrap_or_default(),

            "image_sequence_start": input_file.image_sequence_start,
            "image_sequence_fps": input_file.image_sequence_fps,
            "background_color": params.background.as_slice(),
            "background_mode":  params.background_mode as i32,
            "background_margin":          params.background_margin,
            "background_margin_feather":  params.background_margin_feather,

            "video_info": {
                "width":       params.video_size.0,
                "height":      params.video_size.1,
                "rotation":    params.video_rotation,
                "num_frames":  params.frame_count,
                "fps":         params.fps,
                "duration_ms": params.duration_ms,
                "fps_scale":   params.fps_scale,
                "vfr_fps":     params.get_scaled_fps(),
                "vfr_duration_ms": params.get_scaled_duration_ms(),
            },
            "stabilization": {
                "fov":                    params.fov,
                "method":                 smoothing_name,
                "smoothing_params":       smoothing_params,
                "frame_readout_time":     params.frame_readout_time,
                "adaptive_zoom_window":   params.adaptive_zoom_window,
                "adaptive_zoom_center_offset": params.adaptive_zoom_center_offset,
                "adaptive_zoom_method":   params.adaptive_zoom_method,
                "lens_correction_amount": params.lens_correction_amount,
                "horizon_lock_amount":    horizon_amount,
                "horizon_lock_roll":      horizon_roll,
                "use_gravity_vectors":    gyro.use_gravity_vectors,
                "horizon_lock_integration_method": gyro.horizon_lock_integration_method,
                "video_speed":                   params.video_speed,
                "video_speed_affects_smoothing": params.video_speed_affects_smoothing,
                "video_speed_affects_zooming":   params.video_speed_affects_zooming,
                "horizontal_rs":          params.horizontal_rs,
            },
            "gyro_source": {
                "filepath":           gyro.file_url,
                "lpf":                gyro.imu_lpf,
                "rotation":           gyro.imu_rotation_angles,
                "acc_rotation":       gyro.acc_rotation_angles,
                "imu_orientation":    gyro.imu_orientation,
                "gyro_bias":          gyro.gyro_bias,
                "integration_method": gyro.integration_method,
                "sample_index":       gyro.file_load_options.sample_index,
                "detected_source":    gyro.file_metadata.detected_source,
            },

            "offsets": gyro.get_offsets(), // timestamp, offset value
            "keyframes": self.keyframes.read().serialize(),

            "trim_start": params.trim_start,
            "trim_end":   params.trim_end,
        });

        util::merge_json(&mut obj, &serde_json::from_str(additional_data).unwrap_or_default());

        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if let serde_json::Value::Object(ref mut obj) = obj {
            obj.insert("videofile_bookmark".into(), serde_json::Value::String(filesystem::apple::create_bookmark(&input_file.url, false, _project_url)));
            if let Some(serde_json::Value::Object(ref mut obj)) = obj.get_mut("gyro_source") {
                obj.insert("filepath_bookmark".into(), serde_json::Value::String(filesystem::apple::create_bookmark(&gyro.file_url, false, _project_url)));
            }
            if let Some(serde_json::Value::Object(ref mut obj)) = obj.get_mut("output") {
                if let Some(output_folder) = obj.get("output_folder").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) {
                    obj.insert("output_folder_bookmark".into(), serde_json::Value::String(filesystem::apple::create_bookmark(output_folder, true, _project_url)));
                }
            }
        }

        if let Some(serde_json::Value::Object(ref mut obj)) = obj.get_mut("gyro_source") {
            if typ == GyroflowProjectType::Simple {
                if let Ok(val) = serde_json::to_value(gyro.file_metadata.thin()) {
                    obj.insert("file_metadata".into(), val);
                }
            } else {
                if let Some(q) = util::compress_to_base91_cbor(&gyro.file_metadata) {
                    obj.insert("file_metadata".into(), serde_json::Value::String(q));
                }
            }

            if typ == GyroflowProjectType::WithProcessedData {
                let mut imu_timestamps = Vec::with_capacity(gyro.quaternions.len());
                let mut imu_timestamps_final = Vec::with_capacity(gyro.quaternions.len());
                for (t, _) in &gyro.quaternions {
                    let mut timestamp_ms = *t as f64 / 1000.0;
                    timestamp_ms += gyro.offset_at_gyro_timestamp(timestamp_ms);

                    imu_timestamps.push(timestamp_ms);

                    let frame = ((timestamp_ms - params.frame_readout_time / 2.0) * (params.get_scaled_fps() / 1000.0)).ceil() as usize;
                    imu_timestamps_final.push(timestamp_ms - gyro.file_metadata.per_frame_time_offsets.get(frame).unwrap_or(&0.0));
                }
                util::compress_to_base91_cbor(&imu_timestamps)           .and_then(|s| obj.insert("synced_imu_timestamps" .into(), serde_json::Value::String(s)));
                util::compress_to_base91_cbor(&imu_timestamps_final)     .and_then(|s| obj.insert("synced_imu_timestamps_with_per_frame_offset".into(), serde_json::Value::String(s)));
                util::compress_to_base91_cbor(&gyro.quaternions)         .and_then(|s| obj.insert("integrated_quaternions".into(), serde_json::Value::String(s)));
                util::compress_to_base91_cbor(&gyro.smoothed_quaternions).and_then(|s| obj.insert("smoothed_quaternions"  .into(), serde_json::Value::String(s)));
                util::compress_to_base91_cbor(&params.fovs)              .and_then(|s| obj.insert("adaptive_zoom_fovs"    .into(), serde_json::Value::String(s)));
            }
        }

        Ok(serde_json::to_string_pretty(&obj)?)
    }

    pub fn get_new_videofile_url(org_video_url: &str, gf_file_url: Option<&str>, sequence_start: u32) -> String {
        if gf_file_url.is_some() && !filesystem::exists(org_video_url) {
            ::log::debug!("get_new_videofile_url: {org_video_url}");
            let folder = filesystem::get_folder(gf_file_url.unwrap());
            let filename = filesystem::get_filename(org_video_url);
            let mut filename_replaced = filename.clone();

            if let Some(num_pos) = filename.find('%') {
                if let Some(d_pos) = filename[num_pos+1..].find('d') {
                    if d_pos <= 5 {
                        let num_str = &filename[num_pos+1..num_pos+1+d_pos];
                        if let Ok(num) = num_str.parse::<u32>() {
                            let new_num = format!("{:01$}", sequence_start, num as usize);
                            let from = format!("%{}d", num_str);
                            filename_replaced = filename.replace(&from, &new_num);
                        }
                    }
                }
            }
            if filesystem::exists_in_folder(&folder, &filename_replaced) {
                return filesystem::get_file_url(&folder, &filename, false);
            }
        }
        org_video_url.to_string()
    }

    pub fn import_gyroflow_file<F: Fn(f64)>(&self, url: &str, blocking: bool, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> std::result::Result<serde_json::Value, GyroflowCoreError> {
        let data = filesystem::read(url)?;

        let mut is_preset = false;
        let result = self.import_gyroflow_data(&data, blocking, Some(url), progress_cb, cancel_flag, &mut is_preset);
        if !is_preset && result.is_ok() {
            self.input_file.write().project_file_url = Some(url.to_string());
        }
        result
    }
    pub fn import_gyroflow_data<F: Fn(f64)>(&self, data: &[u8], blocking: bool, url: Option<&str>, progress_cb: F, cancel_flag: Arc<AtomicBool>, is_preset: &mut bool) -> std::result::Result<serde_json::Value, GyroflowCoreError> {
        let mut obj: serde_json::Value = serde_json::from_slice(&data)?;
        if let serde_json::Value::Object(ref mut obj) = obj {
            let mut output_size = None;
            let mut org_video_url = obj.get("videofile").and_then(|x| x.as_str()).unwrap_or(&"").to_string();
            if !org_video_url.is_empty() && !org_video_url.contains("://") {
                org_video_url = filesystem::path_to_url(&org_video_url);
                if let Some(videofile) = obj.get_mut("videofile") {
                    *videofile = serde_json::Value::String(org_video_url.clone());
                }
            }
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            if let Some(v) = obj.get("videofile_bookmark").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) {
                let (resolved, _is_stale) = filesystem::apple::resolve_bookmark(v, url);
                if !resolved.is_empty() { org_video_url = resolved; }
            }

            let sequence_start = obj.get("image_sequence_start").and_then(|x| x.as_i64()).unwrap_or_default() as u32;

            let video_url = Self::get_new_videofile_url(&org_video_url, url, sequence_start);
            if let Some(videofile) = obj.get_mut("videofile") {
                *videofile = serde_json::Value::String(video_url.clone());
            }
            *is_preset = org_video_url.is_empty();

            if let Some(vid_info) = obj.get("video_info") {
                let mut params = self.params.write();
                if let Some(w) = vid_info.get("width").and_then(|x| x.as_u64()) {
                    if let Some(h) = vid_info.get("height").and_then(|x| x.as_u64()) {
                        params.video_size = (w as usize, h as usize);
                    }
                }
                output_size = Some(params.video_size);
                if let Some(v) = vid_info.get("rotation")   .and_then(|x| x.as_f64()) { params.video_rotation = v; }
                if let Some(v) = vid_info.get("num_frames") .and_then(|x| x.as_u64()) { params.frame_count    = v as usize; }
                if let Some(v) = vid_info.get("fps")        .and_then(|x| x.as_f64()) { params.fps            = v; }
                if let Some(v) = vid_info.get("duration_ms").and_then(|x| x.as_f64()) { params.duration_ms    = v; }
                if let Some(v) = vid_info.get("fps_scale") { params.fps_scale = v.as_f64(); }

                self.gyro.write().init_from_params(&params);
                self.keyframes.write().timestamp_scale = params.fps_scale;
            }
            if let Some(serde_json::Value::Object(ref mut obj)) = obj.get_mut("gyro_source") {
                let mut org_gyro_url = obj.get("filepath").and_then(|x| x.as_str()).unwrap_or(&"").to_string();
                if !org_gyro_url.is_empty() && !org_gyro_url.contains("://") {
                    org_gyro_url = filesystem::path_to_url(&org_gyro_url);
                    if let Some(filepath) = obj.get_mut("filepath") {
                        *filepath = serde_json::Value::String(org_gyro_url.clone());
                    }
                }
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                if let Some(v) = obj.get("filepath_bookmark").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) {
                    let (resolved, _is_stale) = filesystem::apple::resolve_bookmark(v, url);
                    if !resolved.is_empty() { org_gyro_url = resolved; }
                }
                let gyro_url = Self::get_new_videofile_url(&org_gyro_url, url.clone(), sequence_start);
                if let Some(fp) = obj.get_mut("filepath") {
                    *fp = serde_json::Value::String(gyro_url.clone());
                }
                use crate::gyro_source::TimeIMU;

                let is_compressed = obj.get("raw_imu").map(|x| x.is_string()).unwrap_or_default();
                let is_main_video = org_gyro_url == org_video_url;

                // Load IMU data only if it's from another file or the gyro file is not accessible anymore
                if (!org_gyro_url.is_empty() && org_gyro_url != org_video_url) || !filesystem::can_open_file(&gyro_url) {
                    let mut raw_imu = Vec::new();
                    let mut quaternions = TimeQuat::default();
                    let mut image_orientations = None;
                    let mut gravity_vectors = None;
                    if is_compressed {
                        if let Some(bytes) = util::decompress_from_base91(obj.get("raw_imu").and_then(|x| x.as_str()).unwrap_or_default()) {
                            if let Ok(data) = bincode::deserialize(&bytes) as bincode::Result<Vec<TimeIMU>> {
                                raw_imu = data;
                            }
                        }
                        if let Some(bytes) = util::decompress_from_base91(obj.get("quaternions").and_then(|x| x.as_str()).unwrap_or_default()) {
                            if let Ok(data) = bincode::deserialize(&bytes) as bincode::Result<TimeQuat> {
                                quaternions = data;
                            }
                        }
                        if let Some(bytes) = util::decompress_from_base91(obj.get("image_orientations").and_then(|x| x.as_str()).unwrap_or_default()) {
                            if let Ok(data) = bincode::deserialize(&bytes) as bincode::Result<TimeQuat> {
                                image_orientations = Some(data);
                            }
                        }
                        if let Some(bytes) = util::decompress_from_base91(obj.get("gravity_vectors").and_then(|x| x.as_str()).unwrap_or_default()) {
                            if let Ok(data) = bincode::deserialize(&bytes) as bincode::Result<TimeVec> {
                                gravity_vectors = Some(data);
                            }
                        }
                    } else {
                        if let Some(ri) = obj.get("raw_imu") {
                            if ri.is_array() {
                                raw_imu = serde_json::from_value(ri.clone()).unwrap_or_default();
                            }
                        }
                        quaternions = obj.get("quaternions")
                            .and_then(|x| x.as_object())
                            .and_then(|x| {
                                let mut ret = TimeQuat::new();
                                for (k, v) in x {
                                    if let Ok(ts) = k.parse::<i64>() {
                                        if let Some(v) = v.as_array() {
                                            let v = v.into_iter().filter_map(|vv| vv.as_f64()).collect::<Vec<f64>>();
                                            if v.len() == 4 {
                                                let quat = Quat64::from_quaternion(nalgebra::Quaternion::from_vector(Vector4::new(v[0], v[1], v[2], v[3])));
                                                ret.insert(ts, quat);
                                            }
                                        }
                                    }
                                }
                                if !ret.is_empty() { Some(ret) } else { None }
                            })
                            .unwrap_or_default();
                    }

                    if !raw_imu.is_empty() {
                        let md = crate::gyro_source::FileMetadata {
                            imu_orientation: obj.get("imu_orientation").and_then(|x| x.as_str().map(|x| x.to_string())),
                            detected_source: Some(obj.get("detected_source").and_then(|x| x.as_str()).unwrap_or("Gyroflow file").to_string()),
                            quaternions,
                            gravity_vectors,
                            image_orientations,
                            raw_imu,
                            ..Default::default()
                        };

                        let mut gyro = self.gyro.write();
                        gyro.load_from_telemetry(md);
                    } else if let Ok(md) = util::decompress_from_base91_cbor(obj.get("file_metadata").and_then(|x| x.as_str()).unwrap_or_default()) as std::io::Result<crate::gyro_source::FileMetadata> {
                        let mut gyro = self.gyro.write();
                        gyro.load_from_telemetry(md);
                    } else if filesystem::exists(&gyro_url) && blocking {
                        if let Err(e) = self.load_gyro_data(&gyro_url, is_main_video, &Default::default(), progress_cb, cancel_flag) {
                            ::log::warn!("Failed to load gyro data from {:?}: {:?}", gyro_url, e);
                        }
                    }
                } else if filesystem::exists(&gyro_url) && blocking {
                    if let Err(e) = self.load_gyro_data(&gyro_url, is_main_video, &Default::default(), progress_cb, cancel_flag) {
                        ::log::warn!("Failed to load gyro data from {:?}: {:?}", gyro_url, e);
                    }
                }

                let mut gyro = self.gyro.write();
                if !org_gyro_url.is_empty() {
                    gyro.file_url = gyro_url.clone();
                }

                if let Some(v) = obj.get("lpf").and_then(|x| x.as_f64()) { gyro.imu_lpf = v; }
                if let Some(v) = obj.get("integration_method").and_then(|x| x.as_u64()) { gyro.integration_method = v as usize; }
                if let Some(v) = obj.get("imu_orientation").and_then(|x| x.as_str()) { gyro.imu_orientation = Some(v.to_string()); }
                if let Some(v) = obj.get("rotation")     { gyro.imu_rotation_angles = serde_json::from_value(v.clone()).ok(); }
                if let Some(v) = obj.get("acc_rotation") { gyro.acc_rotation_angles = serde_json::from_value(v.clone()).ok(); }
                if let Some(v) = obj.get("gyro_bias")    { gyro.gyro_bias           = serde_json::from_value(v.clone()).ok(); }

                obj.remove("raw_imu");
                obj.remove("quaternions");
                obj.remove("smoothed_quaternions");
                obj.remove("image_orientations");
                obj.remove("gravity_vectors");
                obj.remove("file_metadata");
            }
            if let Some(lens) = obj.get("calibration_data") {
                let mut l = self.lens.write();
                l.load_from_json_value(&lens);
                let db = self.lens_profile_db.read();
                l.resolve_interpolations(&db);
            }
            if let Some(serde_json::Value::Object(ref mut obj)) = obj.get_mut("stabilization") {
                let mut params = self.params.write();
                if let Some(v) = obj.get("fov")                   .and_then(|x| x.as_f64()) { params.fov                     = v; }
                if let Some(v) = obj.get("frame_readout_time")    .and_then(|x| x.as_f64()) { params.frame_readout_time      = v; }
                if let Some(v) = obj.get("adaptive_zoom_window")  .and_then(|x| x.as_f64()) { params.adaptive_zoom_window    = v; }
                if let Some(v) = obj.get("lens_correction_amount").and_then(|x| x.as_f64()) { params.lens_correction_amount  = v; }
                if let Some(v) = obj.get("horizontal_rs")        .and_then(|x| x.as_bool()) { params.horizontal_rs          = v; }

                if let Some(v) = obj.get("video_speed").and_then(|x| x.as_f64()) { params.video_speed = v; }
                if let Some(v) = obj.get("video_speed_affects_smoothing").and_then(|x| x.as_bool()) { params.video_speed_affects_smoothing = v; }
                if let Some(v) = obj.get("video_speed_affects_zooming")  .and_then(|x| x.as_bool()) { params.video_speed_affects_zooming   = v; }

                if let Some(center_offs) = obj.get("adaptive_zoom_center_offset").and_then(|x| x.as_array()) {
                    params.adaptive_zoom_center_offset = (
                        center_offs.get(0).and_then(|x| x.as_f64()).unwrap_or_default(),
                        center_offs.get(1).and_then(|x| x.as_f64()).unwrap_or_default()
                    );
                }
                if let Some(zooming_method) = obj.get("adaptive_zoom_method").and_then(|x| x.as_i64()) {
                    params.adaptive_zoom_method = zooming_method as i32;
                }

                if let Some(method) = obj.get("method").and_then(|x| x.as_str()) {
                    let method_idx = self.get_smoothing_algs()
                        .iter().enumerate()
                        .find(|(_, m)| method == m.as_str())
                        .map(|(idx, _)| idx)
                        .unwrap_or(1);

                    self.smoothing.write().set_current(method_idx);
                }

                let mut smoothing = self.smoothing.write();
                let empty_vec = Vec::new();
                let smoothing_params = obj.get("smoothing_params").and_then(|x| x.as_array()).unwrap_or(&empty_vec);
                let smoothing_alg = smoothing.current_mut();
                for param in smoothing_params {
                    (|| -> Option<()> {
                        let name = param.get("name").and_then(|x| x.as_str())?;
                        let value = param.get("value").and_then(|x| x.as_f64())?;
                        smoothing_alg.set_parameter(name, value);
                        Some(())
                    })();
                }
                if let Some(horizon_amount) = obj.get("horizon_lock_amount").and_then(|x| x.as_f64()) {
                    if let Some(horizon_roll) = obj.get("horizon_lock_roll").and_then(|x| x.as_f64()) {
                        smoothing.horizon_lock.set_horizon(horizon_amount, horizon_roll);
                    }
                }
                if let Some(v) = obj.get("use_gravity_vectors").and_then(|x| x.as_bool()) {
                    self.gyro.write().set_use_gravity_vectors(v);
                }
                if let Some(v) = obj.get("horizon_lock_integration_method").and_then(|x| x.as_i64()) {
                    self.gyro.write().set_horizon_lock_integration_method(v as i32);
                }

                obj.remove("adaptive_zoom_fovs");
            }
            if let Some(serde_json::Value::Object(ref mut obj)) = obj.get_mut("output") {
                if let Some(w) =  obj.get("output_width").and_then(|x| x.as_u64()) {
                    if let Some(h) =  obj.get("output_height").and_then(|x| x.as_u64()) {
                        output_size = Some((w as usize, h as usize));
                    }
                }
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                if let Some(v) = obj.get("output_folder_bookmark").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) {
                    let (resolved, _is_stale) = filesystem::apple::resolve_bookmark(v, url);
                    if !resolved.is_empty() {
                        filesystem::folder_access_granted(&resolved);
                        obj.insert("output_folder".into(), serde_json::Value::String(resolved));
                    }
                }
            }

            if let Some(serde_json::Value::Object(offsets)) = obj.get("offsets") {
                let mut gyro = self.gyro.write();
                gyro.set_offsets(offsets.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_f64()?))).collect());
                self.keyframes.write().update_gyro(&gyro);
            }
            obj.remove("offsets");

            if let Some(keyframes) = obj.get("keyframes") {
                self.keyframes.write().deserialize(keyframes);
            }

            if let Some(start) = obj.get("trim_start").and_then(|x| x.as_f64()) {
                if let Some(end) = obj.get("trim_end").and_then(|x| x.as_f64()) {
                    let mut params = self.params.write();
                    params.trim_start = start;
                    params.trim_end = end;
                }
            }

            {
                let mut params = self.params.write();
                if let Some(v) = obj.get("background_color").and_then(|x| x.as_array()) {
                    if v.len() == 4 {
                        params.background = nalgebra::Vector4::new(
                            v[0].as_f64().unwrap_or_default() as f32,
                            v[1].as_f64().unwrap_or_default() as f32,
                            v[2].as_f64().unwrap_or_default() as f32,
                            v[3].as_f64().unwrap_or_default() as f32
                        );
                    }
                }
                if let Some(v) = obj.get("background_mode").and_then(|x| x.as_i64()) { params.background_mode = stabilization_params::BackgroundMode::from(v as i32); }
                if let Some(v) = obj.get("background_margin").and_then(|x| x.as_f64()) { params.background_margin = v; }
                if let Some(v) = obj.get("background_margin_feather").and_then(|x| x.as_f64()) { params.background_margin_feather = v; }
            }

            {
                let mut input_file = self.input_file.write();
                if let Some(seq_start) = obj.get("image_sequence_start").and_then(|x| x.as_i64()) {
                    input_file.image_sequence_start = seq_start as i32;
                }
                if let Some(seq_fps) = obj.get("image_sequence_fps").and_then(|x| x.as_f64()) {
                    input_file.image_sequence_fps = seq_fps;
                }
                if !org_video_url.is_empty() && filesystem::can_open_file(&video_url) {
                    input_file.url = video_url;
                }
            }

            if blocking {
                self.recompute_gyro();

                if let Some(output_size) = output_size {
                    if output_size.0 > 0 && output_size.1 > 0 {
                        self.set_size(output_size.0, output_size.1);
                        self.set_output_size(output_size.0, output_size.1);
                    }
                }
                self.recompute_blocking();
            }
        }
        Ok(obj)
    }

    pub fn load_video_file(&self, url: &str, mut metadata: Option<telemetry_parser::util::VideoMetadata>) -> Result<telemetry_parser::util::VideoMetadata, GyroflowCoreError> {
        if metadata.is_none() {
            metadata = Some(util::get_video_metadata(url)?);
        }
        let metadata = metadata.unwrap();
        log::info!("Loading video file: {metadata:?}");

        if metadata.width > 0 && metadata.height > 0 && metadata.duration_s > 0.0 && metadata.fps > 0.0 {
            let video_size = (metadata.width as usize, metadata.height as usize);
            let frame_count = (metadata.duration_s * metadata.fps).ceil() as usize;

            self.init_from_video_data(metadata.duration_s * 1000.0, metadata.fps, frame_count, video_size);
            let _ = self.load_gyro_data(url, true, &Default::default(), |_|(), Arc::new(AtomicBool::new(false)));

            let camera_id = self.camera_id.read();

            let id_str = camera_id.as_ref().map(|v| v.get_identifier_for_autoload()).unwrap_or_default();
            if !id_str.is_empty() {
                let mut db = self.lens_profile_db.read();
                if !db.loaded {
                    drop(db);
                    {
                        let mut db = self.lens_profile_db.write();
                        db.load_all();
                    }
                    db = self.lens_profile_db.read();
                }
                if db.contains_id(&id_str) {
                    match self.load_lens_profile(&id_str) {
                        Ok(_) => {
                            if let Some(fr) = self.lens.read().frame_readout_time {
                                self.params.write().frame_readout_time = fr;
                            }
                        }
                        Err(e) => {
                            log::error!("An error occured: {e:?}");
                            return Err(e);
                        }
                    }
                }
            }
            let mut output_width = metadata.width;
            let mut output_height = metadata.height;
            if let Some(output_dim) = self.lens.read().output_dimension.clone() {
                output_width = output_dim.w;
                output_height = output_dim.h;
            }
            self.set_size(video_size.0, video_size.1);
            self.set_output_size(output_width, output_height);
        }
        Ok(metadata)
    }

    pub fn set_device(&self, i: i32) {
        self.params.write().current_device = i;
        let mut l = self.stabilization.write();
        l.set_device(i as isize);
    }

    pub fn set_keyframe(&self, typ: &KeyframeType, timestamp_us: i64, value: f64) {
        self.keyframes.write().set(typ, timestamp_us, value);
        self.keyframes_updated(typ);
    }
    pub fn set_keyframe_easing(&self, typ: &KeyframeType, timestamp_us: i64, easing: Easing) {
        self.keyframes.write().set_easing(typ, timestamp_us, easing);
        self.keyframes_updated(typ);
    }
    pub fn keyframe_easing(&self, typ: &KeyframeType, timestamp_us: i64) -> Option<Easing> {
        self.keyframes.read().easing(typ, timestamp_us)
    }
    pub fn remove_keyframe(&self, typ: &KeyframeType, timestamp_us: i64) {
        self.keyframes.write().remove(typ, timestamp_us);
        self.keyframes_updated(typ);
    }
    pub fn clear_keyframes_type(&self, typ: &KeyframeType) {
        self.keyframes.write().clear_type(typ);
        self.keyframes_updated(typ);
    }
    pub fn keyframe_value_at_video_timestamp(&self, typ: &KeyframeType, timestamp_ms: f64) -> Option<f64> {
        self.keyframes.read().value_at_video_timestamp(typ, timestamp_ms)
    }
    pub fn is_keyframed(&self, typ: &KeyframeType) -> bool {
        self.keyframes.read().is_keyframed(typ)
    }
    fn keyframes_updated(&self, typ: &KeyframeType) {
        match typ {
            KeyframeType::VideoRotation |
            KeyframeType::ZoomingSpeed |
            KeyframeType::ZoomingCenterX |
            KeyframeType::ZoomingCenterY => self.invalidate_zooming(),

            KeyframeType::LockHorizonAmount |
            KeyframeType::LockHorizonRoll |
            KeyframeType::SmoothingParamTimeConstant |
            KeyframeType::SmoothingParamTimeConstant2 |
            KeyframeType::SmoothingParamSmoothness |
            KeyframeType::SmoothingParamPitch |
            KeyframeType::SmoothingParamRoll |
            KeyframeType::SmoothingParamYaw => self.invalidate_smoothing(),
            _ => { }
        }
    }
}

pub fn timestamp_at_frame(frame: i32, fps: f64) -> f64 { frame as f64 * 1000.0 / fps }
pub fn frame_at_timestamp(timestamp_ms: f64, fps: f64) -> i32 { (timestamp_ms * (fps / 1000.0)).round() as i32 }

pub fn run_threaded<F>(cb: F) where F: FnOnce() + Send + 'static {
    THREAD_POOL.spawn(cb);
}

use std::str::FromStr;
#[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
pub enum GyroflowProjectType {
    Simple,
    WithGyroData,
    WithProcessedData
}
impl FromStr for GyroflowProjectType {
    type Err = serde_json::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> { serde_json::from_str(&format!("\"{}\"", s)) }
}
impl ToString for GyroflowProjectType {
    fn to_string(&self) -> String { format!("{:?}", self) }
}

#[derive(thiserror::Error, Debug)]
pub enum GyroflowCoreError {
    #[error("No stabilization data at {0}. Make sure you called `ensure_ready_for_processing`")]
    NoStabilizationData(i64),

    #[error("Buffer too small")]
    BufferTooSmall,

    #[error("Size too small")]
    SizeTooSmall,

    #[error("Size mismatch ({0:?} != ({1:?})")]
    SizeMismatch((usize, usize), (usize, usize)),

    #[error("Invalid stride: {0} must be greater than width ({1})")]
    InvalidStride(i32, i32),

    #[error("Input buffer is empty")]
    InputBufferEmpty,

    #[error("Output buffer is empty")]
    OutputBufferEmpty,

    #[error("Failed to find cached wgpu in process_pixels. Key: {0}")]
    NoCachedWgpuInstance(String),

    #[error("Unsupported file format {0}")]
    UnsupportedFormat(String),

    #[error("Invalid data")]
    InvalidData,

    #[error("JSON error {0:?}")]
    JSONError(#[from] serde_json::Error),

    #[error("Filesystem error {0:?}")]
    FilesystemError(#[from] crate::filesystem::FilesystemError),

    #[error("IO error {0:?}")]
    IOError(#[from] std::io::Error),

    #[error("Unknown error")]
    Unknown
}
