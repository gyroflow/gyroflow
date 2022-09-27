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

pub mod gpu;

pub mod util;
pub mod stabilization_params;

use std::sync::{ Arc, atomic::{ AtomicU64, AtomicBool, Ordering::SeqCst } };
use std::path::PathBuf;
use keyframes::*;
use parking_lot::{ RwLock, RwLockUpgradableReadGuard };
use nalgebra::Vector4;
use gyro_source::{ GyroSource, Quat64, TimeQuat, TimeVec };
use stabilization_params::StabilizationParams;
use lens_profile::LensProfile;
use lens_profile_database::LensProfileDatabase;
use smoothing::Smoothing;
use stabilization::Stabilization;
use zooming::ZoomingAlgorithm;
use camera_identifier::CameraIdentifier;
pub use stabilization::PixelType;
use gpu::BufferDescription;
use gpu::drawing::*;

#[cfg(feature = "opencv")]
use calibration::LensCalibrator;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

lazy_static::lazy_static! {
    static ref THREAD_POOL: rayon::ThreadPool = rayon::ThreadPoolBuilder::new().build().unwrap();
}

#[derive(Default, Clone, Debug)]
pub struct InputFile {
    pub path: String,
    pub image_sequence_fps: f64,
    pub image_sequence_start: i32
}

pub struct StabilizationManager<T: PixelType> {
    pub gyro: Arc<RwLock<GyroSource>>,
    pub lens: Arc<RwLock<LensProfile>>,
    pub smoothing: Arc<RwLock<Smoothing>>,

    pub stabilization: Arc<RwLock<Stabilization<T>>>,

    pub pose_estimator: Arc<synchronization::PoseEstimator>,
    #[cfg(feature = "opencv")]
    pub lens_calibrator: Arc<RwLock<Option<LensCalibrator>>>,

    pub current_compute_id: Arc<AtomicU64>,
    pub smoothing_checksum: Arc<AtomicU64>,
    pub zooming_checksum: Arc<AtomicU64>,

    pub camera_id: Arc<RwLock<Option<CameraIdentifier>>>,
    pub lens_profile_db: Arc<RwLock<LensProfileDatabase>>,

    pub input_file: Arc<RwLock<InputFile>>,

    pub keyframes: Arc<RwLock<KeyframeManager>>,

    pub params: Arc<RwLock<StabilizationParams>>
}

impl<T: PixelType> Default for StabilizationManager<T> {
    fn default() -> Self {
        Self {
            smoothing: Arc::new(RwLock::new(Smoothing::default())),

            params: Arc::new(RwLock::new(StabilizationParams::default())),

            stabilization: Arc::new(RwLock::new(Stabilization::<T>::default())),
            gyro: Arc::new(RwLock::new(GyroSource::new())),
            lens: Arc::new(RwLock::new(LensProfile::default())),

            current_compute_id: Arc::new(AtomicU64::new(0)),
            smoothing_checksum: Arc::new(AtomicU64::new(0)),
            zooming_checksum: Arc::new(AtomicU64::new(0)),

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

impl<T: PixelType> StabilizationManager<T> {
    pub fn init_from_video_data(&self, _path: &str, duration_ms: f64, fps: f64, frame_count: usize, video_size: (usize, usize)) -> std::io::Result<()> {
        {
            let mut params = self.params.write();
            params.fps = fps;
            params.frame_count = frame_count;
            params.duration_ms = duration_ms;
            params.video_size = video_size;
        }

        self.pose_estimator.sync_results.write().clear();
        self.keyframes.write().clear();

        Ok(())
    }

    pub fn load_gyro_data<F: Fn(f64)>(&self, path: &str, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> std::io::Result<gyro_source::FileMetadata> {
        {
            let params = self.params.read();
            let mut gyro = self.gyro.write();
            gyro.init_from_params(&params);
            gyro.clear_offsets();
            gyro.file_path = path.to_string();
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
        let mut md = GyroSource::parse_telemetry_file(path, size, fps, progress_cb, cancel_flag2)?;
        if md.detected_source.as_ref().map(|v| v.starts_with("GoPro ")).unwrap_or_default() {
            // If gopro reports rolling shutter value, it already applied it, ie. the video is already corrected
            md.frame_readout_time = None;
        }
        if !cancel_flag.load(SeqCst) {
            self.gyro.write().load_from_telemetry(&md);
        }
        self.params.write().frame_readout_time = md.frame_readout_time.unwrap_or_default();
        let quats = self.gyro.read().quaternions.clone();
        self.smoothing.write().update_quats_checksum(&quats);

        if let Some(ref lens) = md.lens_profile {
            let mut l = self.lens.write();
            if let Some(lens_str) = lens.as_str() {
                let db = self.lens_profile_db.read();
                if let Some(found) = db.find(lens_str) {
                    *l = found.clone();
                }
            } else {
                l.load_from_json_value(lens);
                l.filename = path.to_string();
                let db = self.lens_profile_db.read();
                l.resolve_interpolations(&db);
            }
        }
        if let Some(ref id) = md.camera_identifier {
            *self.camera_id.write() = Some(id.clone());
        }
        Ok(md)
    }

    pub fn load_lens_profile(&self, path: &str) -> Result<(), serde_json::Error> {
        let db = self.lens_profile_db.read();
        if let Some(lens) = db.get_by_id(path) {
            *self.lens.write() = lens.clone();
            Ok(())
        } else {
            let mut lens = self.lens.write();
            let result = lens.load_from_file(path);
            lens.resolve_interpolations(&db);
            result
        }
    }

    fn init_size(&self) {
        let (w, h, ow, oh, bg) = {
            let params = self.params.read();
            (params.size.0, params.size.1, params.output_size.0, params.output_size.1, params.background)
        };

        let s = w * T::COUNT * T::SCALAR_BYTES;
        let os = ow * T::COUNT * T::SCALAR_BYTES;

        if w > 0 && ow > 0 && h > 0 && oh > 0 {
            self.stabilization.write().init_size(bg, (w, h, s), (ow, oh, os));
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

    pub fn recompute_adaptive_zoom_static(zoom: &Box<dyn ZoomingAlgorithm>, params: &RwLock<StabilizationParams>, keyframes: &KeyframeManager) -> Vec<f64> {
        let (window, frames, fps) = {
            let params = params.read();
            (params.adaptive_zoom_window, params.frame_count, params.get_scaled_fps())
        };
        if window > 0.0 || window < -0.9 {
            let mut timestamps = Vec::with_capacity(frames);
            for i in 0..frames {
                timestamps.push(i as f64 * 1000.0 / fps);
            }

            let fovs = zoom.compute(&timestamps, &keyframes);
            fovs.iter().map(|v| v.0).collect()
        } else {
            Vec::new()
        }
    }
    pub fn recompute_adaptive_zoom(&self) {
        let params = stabilization::ComputeParams::from_manager(self, false);
        let lens_fov_adjustment = params.lens.optimal_fov.unwrap_or(1.0);
        let mut zoom = zooming::from_compute_params(params);
        let fovs = Self::recompute_adaptive_zoom_static(&mut zoom, &self.params, &self.keyframes.read());

        let mut stab_params = self.params.write();
        stab_params.set_fovs(fovs, lens_fov_adjustment);
        stab_params.zooming_debug_points = zoom.get_debug_points();
    }

    pub fn recompute_smoothness(&self) {
        let mut gyro = self.gyro.write();
        let params = self.params.read();
        let keyframes = self.keyframes.read().clone();
        let smoothing = self.smoothing.read();
        let horizon_lock = smoothing.horizon_lock.clone();

        gyro.recompute_smoothness(smoothing.current().as_ref(), horizon_lock, &params, &keyframes);
    }

    pub fn recompute_undistortion(&self) {
        let params = stabilization::ComputeParams::from_manager(self, false);
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
        let mut params = stabilization::ComputeParams::from_manager(self, false);

        let smoothing = self.smoothing.clone();
        let stabilization_params = self.params.clone();
        let keyframes = self.keyframes.read().clone();
        let gyro = self.gyro.clone();

        let compute_id = fastrand::u64(..);
        self.current_compute_id.store(compute_id, SeqCst);

        let current_compute_id = self.current_compute_id.clone();
        let smoothing_checksum = self.smoothing_checksum.clone();
        let zooming_checksum = self.zooming_checksum.clone();

        let stabilization = self.stabilization.clone();
        THREAD_POOL.spawn(move || {
            // std::thread::sleep(std::time::Duration::from_millis(20));
            if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

            let mut smoothing_changed = false;
            if smoothing.read().get_state_checksum() != smoothing_checksum.load(SeqCst) {
                let (mut smoothing, horizon_lock) = {
                    let lock = smoothing.read();
                    (lock.current().clone(), lock.horizon_lock.clone())
                };
                params.gyro.recompute_smoothness(smoothing.as_mut(), horizon_lock, &stabilization_params.read(), &keyframes);

                if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

                let mut lib_gyro = gyro.write();
                lib_gyro.quaternions = params.gyro.quaternions.clone();
                lib_gyro.smoothed_quaternions = params.gyro.smoothed_quaternions.clone();
                lib_gyro.max_angles = params.gyro.max_angles;
                lib_gyro.org_smoothed_quaternions = params.gyro.org_smoothed_quaternions.clone();
                lib_gyro.smoothing_status = smoothing.get_status_json();
                smoothing_changed = true;
            }

            if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

            let mut zoom = zooming::from_compute_params(params.clone());
            if smoothing_changed || zooming::get_checksum(&zoom) != zooming_checksum.load(SeqCst) {
                params.fovs = Self::recompute_adaptive_zoom_static(&mut zoom, &stabilization_params, &keyframes);

                if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

                let mut stab_params = stabilization_params.write();
                stab_params.set_fovs(params.fovs.clone(), params.lens.optimal_fov.unwrap_or(1.0));
                stab_params.zooming_debug_points = zoom.get_debug_points();
            }

            if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

            stabilization.write().set_compute_params(params);

            smoothing_checksum.store(smoothing.read().get_state_checksum(), SeqCst);
            zooming_checksum.store(zooming::get_checksum(&zoom), SeqCst);
            cb((compute_id, false));
        });
        compute_id
    }

    pub fn get_features_pixels(&self, timestamp_us: i64, size: (usize, usize)) -> Option<Vec<(i32, i32)>> { // (x, y, alpha)
        let mut ret = None;
        use crate::util::MapClosest;
        use synchronization::EstimatorItemInterface;

        if let Some(l) = self.pose_estimator.sync_results.try_read() {
            if let Some(entry) = l.get_closest(&timestamp_us, 2000) { // closest within 2ms
                let ratio = size.1 as f32 / entry.frame_size.1.max(1) as f32;
                for pt in entry.item.get_features() {
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
                        let fov = (p.fov * p.fovs.get(frame).unwrap_or(&1.0)).max(0.0001);
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

    pub fn process_pixels(&self, mut timestamp_us: i64, buffers: &mut BufferDescription) -> Option<stabilization::ProcessedInfo> {
        if let Some(scale) = self.params.read().fps_scale {
            timestamp_us = (timestamp_us as f64 / scale).round() as i64;
        }

        {
            let mut undist = self.stabilization.write();
            self.draw_overlays(&mut undist.drawing, timestamp_us);
            undist.ensure_ready_for_processing(timestamp_us, buffers);
        }

        let undist = self.stabilization.read();
        undist.process_pixels(timestamp_us, buffers)
    }

    pub fn set_video_rotation(&self, v: f64) { self.params.write().video_rotation = v; }

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
    pub fn set_fov                   (&self, v: f64)  { self.params.write().fov                    = v; }
    pub fn set_lens_correction_amount(&self, v: f64)  { self.params.write().lens_correction_amount = v; self.invalidate_zooming(); }
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

    pub fn get_scaling_ratio(&self) -> f64 { let params = self.params.read(); params.video_size.0 as f64 / params.video_output_size.0 as f64 }
    pub fn get_min_fov      (&self) -> f64 { self.params.read().min_fov }

    pub fn invalidate_smoothing(&self) { self.smoothing_checksum.store(0, SeqCst); self.invalidate_zooming(); }
    pub fn invalidate_zooming(&self) { self.zooming_checksum.store(0, SeqCst); }

    pub fn set_digital_lens_name(&self, v: String) {
        self.lens.write().digital_lens = Some(v.clone());
        #[cfg(feature = "opencv")]
        if let Some(ref mut calib) = *self.lens_calibrator.write() {
            calib.digital_lens = Some(v);
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
        self.smoothing.write().update_quats_checksum(&self.gyro.read().quaternions);
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

    pub fn set_background_color(&self, bg: Vector4<f32>) {
        self.params.write().background = bg;
        self.stabilization.write().set_background(bg);
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
    pub fn get_smoothing_max_angles(&self) -> (f64, f64, f64) {
        self.gyro.read().max_angles
    }
    pub fn get_smoothing_status(&self) -> serde_json::Value {
        self.gyro.read().smoothing_status.clone()
    }
    pub fn get_smoothing_algs(&self) -> Vec<String> {
        self.smoothing.read().get_names()
    }

    pub fn get_cloned(&self) -> StabilizationManager<T> {
        StabilizationManager {
            params: Arc::new(RwLock::new(self.params.read().clone())),
            gyro:   Arc::new(RwLock::new(self.gyro.read().clone())),
            lens:   Arc::new(RwLock::new(self.lens.read().clone())),
            keyframes:  Arc::new(RwLock::new(self.keyframes.read().clone())),
            smoothing:  Arc::new(RwLock::new(self.smoothing.read().clone())),
            input_file: Arc::new(RwLock::new(self.input_file.read().clone())),
            lens_profile_db: self.lens_profile_db.clone(),
            ..Default::default()
        }
    }
    pub fn set_render_params(&self, size: (usize, usize), output_size: (usize, usize)) {
        self.params.write().framebuffer_inverted = false;
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

    pub fn override_video_fps(&self, fps: f64) {
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

        self.stabilization.write().set_compute_params(stabilization::ComputeParams::from_manager(self, false));

        self.invalidate_smoothing();
    }

    pub fn list_gpu_devices<F: Fn(Vec<String>) + Send + Sync + 'static>(&self, cb: F) {
        let stab = self.stabilization.clone();
        run_threaded(move || {
            let lock = stab.upgradable_read();
            let list = lock.list_devices();

            {
                let mut lock = RwLockUpgradableReadGuard::upgrade(lock);
                lock.gpu_list = list.clone();
            }
            cb(list);
        });
    }

    pub fn export_gyroflow_file(&self, filepath: impl AsRef<std::path::Path>, thin: bool, extended: bool, additional_data: String) -> std::io::Result<()> {
        let data = self.export_gyroflow_data(thin, extended, additional_data)?;
        std::fs::write(filepath, data)?;

        Ok(())
    }
    pub fn export_gyroflow_data(&self, thin: bool, extended: bool, additional_data: String) -> std::io::Result<String> {
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
            "version": 2,
            "app_version": env!("CARGO_PKG_VERSION").to_string(),
            "videofile": input_file.path,
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
                // "adaptive_zoom_fovs":     if !thin { util::compress_to_base91(&params.fovs) } else { None },
                "lens_correction_amount": params.lens_correction_amount,
                "horizon_lock_amount":    horizon_amount,
                "horizon_lock_roll":      horizon_roll,
                "use_gravity_vectors":    gyro.use_gravity_vectors,
                "video_speed":                   params.video_speed,
                "video_speed_affects_smoothing": params.video_speed_affects_smoothing,
                "video_speed_affects_zooming":   params.video_speed_affects_zooming,
            },
            "gyro_source": {
                "filepath":           gyro.file_path,
                "lpf":                gyro.imu_lpf,
                "rotation":           gyro.imu_rotation_angles,
                "acc_rotation":       gyro.acc_rotation_angles,
                "imu_orientation":    gyro.imu_orientation,
                "gyro_bias":          gyro.gyro_bias,
                "integration_method": gyro.integration_method,
                "raw_imu":            if !thin { util::compress_to_base91(&gyro.org_raw_imu) } else { None },
                "quaternions":        if !thin && input_file.path != gyro.file_path { util::compress_to_base91(&gyro.org_quaternions) } else { None },
                "image_orientations": if !thin && input_file.path != gyro.file_path { util::compress_to_base91(&gyro.image_orientations) } else { None },
                "gravity_vectors":    if !thin && input_file.path != gyro.file_path && gyro.gravity_vectors.is_some() { util::compress_to_base91(gyro.gravity_vectors.as_ref().unwrap()) } else { None },
                // "smoothed_quaternions": smooth_quats
            },

            "offsets": gyro.get_offsets(), // timestamp, offset value
            "keyframes": self.keyframes.read().serialize(),

            "trim_start": params.trim_start,
            "trim_end":   params.trim_end,

            // "frame_orientation": {}, // timestamp, original frame quaternion
            // "stab_transform":    {} // timestamp, final quaternion
        });

        util::merge_json(&mut obj, &serde_json::from_str(&additional_data).unwrap_or_default());

        if extended {
            if let Some(serde_json::Value::Object(ref mut obj)) = obj.get_mut("gyro_source") {
                if let Some(q) = util::compress_to_base91(&gyro.quaternions) {
                    obj.insert("integrated_quaternions".into(), serde_json::Value::String(q));
                }
                if let Some(q) = util::compress_to_base91(&gyro.smoothed_quaternions) {
                    obj.insert("smoothed_quaternions".into(),   serde_json::Value::String(q));
                }
            }
        }

        Ok(serde_json::to_string_pretty(&obj)?)
    }

    pub fn get_new_videofile_path(file_path: &str, path: Option<std::path::PathBuf>) -> PathBuf {
        let mut file_path = std::path::Path::new(file_path).to_path_buf();
        if path.is_some() && !file_path.exists() {
            if let Some(filename) = file_path.file_name() {
                let new_path = path.as_ref().unwrap().with_file_name(filename);
                if new_path.exists() {
                    file_path = new_path;
                }
            }
        }
        file_path
    }

    pub fn import_gyroflow_file<F: Fn(f64)>(&self, path: &str, blocking: bool, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> std::io::Result<serde_json::Value> {
        let data = std::fs::read(path)?;
        self.import_gyroflow_data(&data, blocking, Some(std::path::Path::new(path).to_path_buf()), progress_cb, cancel_flag)
    }
    pub fn import_gyroflow_data<F: Fn(f64)>(&self, data: &[u8], blocking: bool, path: Option<std::path::PathBuf>, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> std::io::Result<serde_json::Value> {
        let mut obj: serde_json::Value = serde_json::from_slice(&data)?;
        if let serde_json::Value::Object(ref mut obj) = obj {
            let mut output_size = None;
            let org_video_path = obj.get("videofile").and_then(|x| x.as_str()).unwrap_or(&"").to_string();

            let video_path = Self::get_new_videofile_path(&org_video_path, path.clone());
            if let Some(videofile) = obj.get_mut("videofile") {
                *videofile = serde_json::Value::String(util::path_to_str(&video_path));
            }

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
            }
            if let Some(lens) = obj.get("calibration_data") {
                let mut l = self.lens.write();
                l.load_from_json_value(&lens);
                let db = self.lens_profile_db.read();
                l.resolve_interpolations(&db);
            }
            obj.remove("frame_orientation");
            obj.remove("stab_transform");
            if let Some(serde_json::Value::Object(ref mut obj)) = obj.get_mut("gyro_source") {
                let org_gyro_path = obj.get("filepath").and_then(|x| x.as_str()).unwrap_or(&"").to_string();
                let gyro_path = Self::get_new_videofile_path(&org_gyro_path, path.clone());
                if let Some(fp) = obj.get_mut("filepath") {
                    *fp = serde_json::Value::String(util::path_to_str(&gyro_path));
                }
                use crate::gyro_source::TimeIMU;

                let is_compressed = obj.get("raw_imu").map(|x| x.is_string()).unwrap_or_default();

                // Load IMU data only if it's from another file
                if !org_gyro_path.is_empty() && org_gyro_path != org_video_path {
                    let mut raw_imu = None;
                    let mut quaternions = None;
                    let mut image_orientations = None;
                    let mut gravity_vectors = None;
                    if is_compressed {
                        if let Some(bytes) = util::decompress_from_base91(obj.get("raw_imu").and_then(|x| x.as_str()).unwrap_or_default()) {
                            if let Ok(data) = bincode::deserialize(&bytes) as bincode::Result<Vec<TimeIMU>> {
                                raw_imu = Some(data);
                            }
                        }
                        if let Some(bytes) = util::decompress_from_base91(obj.get("quaternions").and_then(|x| x.as_str()).unwrap_or_default()) {
                            if let Ok(data) = bincode::deserialize(&bytes) as bincode::Result<TimeQuat> {
                                quaternions = Some(data);
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
                                raw_imu = serde_json::from_value(ri.clone()).ok();
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
                            });
                    }

                    if raw_imu.is_some() {
                        let md = crate::gyro_source::FileMetadata {
                            imu_orientation: obj.get("imu_orientation").and_then(|x| x.as_str().map(|x| x.to_string())),
                            detected_source: Some("Gyroflow file".to_string()),
                            quaternions,
                            gravity_vectors,
                            image_orientations,
                            raw_imu,
                            lens_profile: None,
                            frame_readout_time: None,
                            frame_rate: None,
                            camera_identifier: None,
                            lens_positions: None,
                        };

                        let mut gyro = self.gyro.write();
                        gyro.load_from_telemetry(&md);
                    } else if gyro_path.exists() && blocking {
                        if let Err(e) = self.load_gyro_data(&util::path_to_str(&gyro_path), progress_cb, cancel_flag) {
                            ::log::warn!("Failed to load gyro data from {:?}: {:?}", gyro_path, e);
                        }
                    }
                } else if gyro_path.exists() && blocking {
                    if let Err(e) = self.load_gyro_data(&util::path_to_str(&gyro_path), progress_cb, cancel_flag) {
                        ::log::warn!("Failed to load gyro data from {:?}: {:?}", gyro_path, e);
                    }
                }

                let mut gyro = self.gyro.write();
                if !org_gyro_path.is_empty() {
                    gyro.file_path = util::path_to_str(&gyro_path);
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
            }
            if let Some(serde_json::Value::Object(ref mut obj)) = obj.get_mut("stabilization") {
                let mut params = self.params.write();
                if let Some(v) = obj.get("fov")                   .and_then(|x| x.as_f64()) { params.fov                     = v; }
                if let Some(v) = obj.get("frame_readout_time")    .and_then(|x| x.as_f64()) { params.frame_readout_time      = v; }
                if let Some(v) = obj.get("adaptive_zoom_window")  .and_then(|x| x.as_f64()) { params.adaptive_zoom_window    = v; }
                if let Some(v) = obj.get("lens_correction_amount").and_then(|x| x.as_f64()) { params.lens_correction_amount  = v; }

                if let Some(v) = obj.get("video_speed").and_then(|x| x.as_f64()) { params.video_speed = v; }
                if let Some(v) = obj.get("video_speed_affects_smoothing").and_then(|x| x.as_bool()) { params.video_speed_affects_smoothing = v; }
                if let Some(v) = obj.get("video_speed_affects_zooming")  .and_then(|x| x.as_bool()) { params.video_speed_affects_zooming   = v; }

                if let Some(center_offs) = obj.get("adaptive_zoom_center_offset").and_then(|x| x.as_array()) {
                    params.adaptive_zoom_center_offset = (
                        center_offs.get(0).and_then(|x| x.as_f64()).unwrap_or_default(),
                        center_offs.get(1).and_then(|x| x.as_f64()).unwrap_or_default()
                    );
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

                obj.remove("adaptive_zoom_fovs");
            }
            if let Some(serde_json::Value::Object(ref obj)) = obj.get("output") {
                if let Some(w) =  obj.get("output_width").and_then(|x| x.as_u64()) {
                    if let Some(h) =  obj.get("output_height").and_then(|x| x.as_u64()) {
                        output_size = Some((w as usize, h as usize));
                    }
                }
            }

            if let Some(serde_json::Value::Object(offsets)) = obj.get("offsets") {
                let mut gyro = self.gyro.write();
                gyro.set_offsets(offsets.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_f64()?))).collect());
                self.keyframes.write().update_gyro(&gyro);
            }

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
                if !org_video_path.is_empty() {
                    input_file.path = util::path_to_str(&video_path);
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
