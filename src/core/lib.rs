// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

pub mod gyro_source;
pub mod integration;
pub mod integration_complementary; // TODO: add this to `ahrs` crate
pub mod lens_profile;
pub mod lens_profile_database;
#[cfg(feature = "opencv")]
pub mod calibration;
pub mod synchronization;
pub mod undistortion;
pub mod camera_identifier;

pub mod zooming;
pub mod smoothing;
pub mod filtering;

pub mod gpu;

pub mod util;
pub mod stabilization_params;

use std::sync::{ Arc, atomic::{ AtomicU64, Ordering::SeqCst } };
use std::path::PathBuf;
use parking_lot::{ RwLock, RwLockUpgradableReadGuard };
use nalgebra::Vector4;
use gyro_source::{ GyroSource, Quat64, TimeQuat, TimeVec };
use stabilization_params::StabilizationParams;
use lens_profile::LensProfile;
use lens_profile_database::LensProfileDatabase;
use smoothing::Smoothing;
use undistortion::Undistortion;
use zooming::ZoomingAlgorithm;
use camera_identifier::CameraIdentifier;
pub use undistortion::PixelType;

#[cfg(feature = "opencv")]
use calibration::LensCalibrator;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

lazy_static::lazy_static! {
    static ref THREAD_POOL: rayon::ThreadPool = rayon::ThreadPoolBuilder::new().build().unwrap();
}

pub struct StabilizationManager<T: PixelType> {
    pub gyro: Arc<RwLock<GyroSource>>,
    pub lens: Arc<RwLock<LensProfile>>,
    pub smoothing: Arc<RwLock<Smoothing>>,

    pub undistortion: Arc<RwLock<Undistortion<T>>>,

    pub pose_estimator: Arc<synchronization::PoseEstimator>,
    #[cfg(feature = "opencv")]
    pub lens_calibrator: Arc<RwLock<Option<LensCalibrator>>>,

    pub current_compute_id: Arc<AtomicU64>,
    pub smoothing_checksum: Arc<AtomicU64>,
    pub zooming_checksum: Arc<AtomicU64>,
    pub current_fov_10000: Arc<AtomicU64>,

    pub camera_id: Arc<RwLock<Option<CameraIdentifier>>>,
    pub lens_profile_db: Arc<RwLock<LensProfileDatabase>>,

    pub video_path: Arc<RwLock<String>>,

    pub params: Arc<RwLock<StabilizationParams>>
}

impl<T: PixelType> Default for StabilizationManager<T> {
    fn default() -> Self {
        Self {
            smoothing: Arc::new(RwLock::new(Smoothing::default())),

            params: Arc::new(RwLock::new(StabilizationParams::default())),
            
            undistortion: Arc::new(RwLock::new(Undistortion::<T>::default())),
            gyro: Arc::new(RwLock::new(GyroSource::new())),
            lens: Arc::new(RwLock::new(LensProfile::default())),
            
            current_compute_id: Arc::new(AtomicU64::new(0)),
            smoothing_checksum: Arc::new(AtomicU64::new(0)),
            zooming_checksum: Arc::new(AtomicU64::new(0)),

            current_fov_10000: Arc::new(AtomicU64::new(0)),
            
            pose_estimator: Arc::new(synchronization::PoseEstimator::default()),

            lens_profile_db: Arc::new(RwLock::new(LensProfileDatabase::default())),

            video_path: Arc::new(RwLock::new(String::new())),

            #[cfg(feature = "opencv")]
            lens_calibrator: Arc::new(RwLock::new(None)),

            camera_id: Arc::new(RwLock::new(None)),
        }
    }
}

impl<T: PixelType> StabilizationManager<T> {
    pub fn init_from_video_data(&self, path: &str, duration_ms: f64, fps: f64, frame_count: usize, video_size: (usize, usize)) -> std::io::Result<()> {
        {
            let mut params = self.params.write();
            params.fps = fps;
            params.frame_count = frame_count;
            params.duration_ms = duration_ms;
            params.video_size = video_size;
        }

        self.pose_estimator.sync_results.write().clear();

        let _ = self.load_gyro_data(path); // Ignore the error here, video file may not contain the telemetry and it's ok
        
        Ok(())
    }

    pub fn load_gyro_data(&self, path: &str) -> std::io::Result<()> {
        {
            let params = self.params.read();
            let mut gyro = self.gyro.write();
            gyro.init_from_params(&params);
            gyro.file_path = path.to_string();
            if gyro.prevent_next_load {
                gyro.prevent_next_load = false;
                return Ok(());
            }
        }

        /*if path.ends_with(".gyroflow") {
            let data = std::fs::read_to_string(path)?;
            let v: serde_json::Value = serde_json::from_str(&data)?;
    
            self.lens.write().load_from_json_value(&v["calibration_data"]);

            let to_f64_array = |x: &serde_json::Value| -> Option<Vec<f64>> { Some(x.as_array()?.iter().filter_map(|x| x.as_f64()).collect()) };

            try_block!({
                //let smoothed_quaternions = v["stab_transform"].as_array()?.iter().filter_map(to_f64_array)
                //    .map(|x| ((x[0] * 1000.0) as i64, Quat64::from_quaternion(Quaternion::from_parts(x[3], Vector3::new(x[4], x[5], x[6])))))
                //    .collect();
        
                let quaternions = v["frame_orientation"].as_array()?.iter().filter_map(to_f64_array)
                    .map(|x| ((x[1] * 1000000.0) as i64, Quat64::from_quaternion(Quaternion::from_parts(x[3-1], Vector3::new(x[4-1], x[5-1], x[6-1])))))
                    .collect();
        
                let raw_imu = v["raw_imu"].as_array()?.iter().filter_map(to_f64_array)
                    .map(|x| TimeIMU { timestamp_ms: 0.0/*TODO*/, gyro: Some([x[0], x[1], x[2]]), accl: Some([x[3], x[4], x[6]]), magn: None })
                    .collect();

                let md = crate::gyro_source::FileMetadata {
                    imu_orientation: None, // TODO IMU orientation
                    detected_source: Some("Gyroflow file".to_string()),
                    quaternions: Some(quaternions),
                    raw_imu: Some(raw_imu),
                    frame_readout_time: None,
                    camera_identifier: None,
                };
                self.gyro.write().load_from_telemetry(&md);
                self.smoothing.write().update_quats_checksum(&self.gyro.read().quaternions);
            });
        } else {*/
            let (fps, size) = {
                let params = self.params.read();
                (params.fps, params.video_size)
            };

            let md = GyroSource::parse_telemetry_file(path, size, fps)?;
            self.gyro.write().load_from_telemetry(&md);
            self.params.write().frame_readout_time = md.frame_readout_time.unwrap_or_default();
            self.smoothing.write().update_quats_checksum(&self.gyro.read().quaternions);
            if let Some(id) = md.camera_identifier {
                *self.camera_id.write() = Some(id);
            }
        // }
        Ok(())
    }

    pub fn load_lens_profile(&self, path: &str) -> Result<(), serde_json::Error> {
        let db = self.lens_profile_db.read();
        if let Some(lens) = db.get_by_id(path) {
            *self.lens.write() = lens.clone();
            Ok(())
        } else {
            self.lens.write().load_from_file(path)
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
            self.undistortion.write().init_size(bg, (w, h), s, (ow, oh), os);
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

    pub fn recompute_adaptive_zoom_static(zoom: &Box<dyn ZoomingAlgorithm>, params: &RwLock<StabilizationParams>) -> Vec<f64> {
        let (window, frames, fps) = {
            let params = params.read();
            (params.adaptive_zoom_window, params.frame_count, params.get_scaled_fps())
        };
        if window > 0.0 || window < -0.9 {
            let mut timestamps = Vec::with_capacity(frames);
            for i in 0..frames {
                timestamps.push(i as f64 * 1000.0 / fps);
            }

            let fovs = zoom.compute(&timestamps);
            fovs.iter().map(|v| v.0).collect()
        } else {
            Vec::new()
        }
    }
    pub fn recompute_adaptive_zoom(&self) {
        let params = undistortion::ComputeParams::from_manager(self);
        let lens_fov_adjustment = params.lens_fov_adjustment;
        let mut zoom = zooming::from_compute_params(params);
        let fovs = Self::recompute_adaptive_zoom_static(&mut zoom, &self.params);

        let mut stab_params = self.params.write();
        stab_params.set_fovs(fovs, lens_fov_adjustment);
        stab_params.zooming_debug_points = zoom.get_debug_points();
    }

    pub fn recompute_smoothness(&self) {
        let mut gyro = self.gyro.write();
        let params = self.params.read();
        let mut smoothing = self.smoothing.write();
        let horizon_lock = smoothing.horizon_lock.clone();

        gyro.recompute_smoothness(smoothing.current_mut().as_mut(), horizon_lock, &params);
    }

    pub fn recompute_undistortion(&self) {
        let params = undistortion::ComputeParams::from_manager(self);
        self.undistortion.write().set_compute_params(params);
    }

    pub fn recompute_blocking(&self) {
        self.recompute_smoothness();
        self.recompute_adaptive_zoom();
        self.recompute_undistortion();
    }

    pub fn invalidate_ongoing_computations(&self) {
        self.current_compute_id.store(fastrand::u64(..), SeqCst);
    }
    
    pub fn recompute_threaded<F: Fn(u64) + Send + Sync + Clone + 'static>(&self, cb: F) -> u64 {
        //self.recompute_smoothness();
        //self.recompute_adaptive_zoom();
        let mut params = undistortion::ComputeParams::from_manager(self);

        let smoothing = self.smoothing.clone();
        let stabilization_params = self.params.clone();
        let gyro = self.gyro.clone();

        let compute_id = fastrand::u64(..);
        self.current_compute_id.store(compute_id, SeqCst);

        let current_compute_id = self.current_compute_id.clone();
        let smoothing_checksum = self.smoothing_checksum.clone();
        let zooming_checksum = self.zooming_checksum.clone();

        let undistortion = self.undistortion.clone();
        THREAD_POOL.spawn(move || {
            // std::thread::sleep(std::time::Duration::from_millis(20));
            if current_compute_id.load(SeqCst) != compute_id { return; }

            let mut smoothing_changed = false;
            if smoothing.read().get_state_checksum() != smoothing_checksum.load(SeqCst) {
                let (mut smoothing, horizon_lock) = {
                    let lock = smoothing.read();
                    (lock.current().clone(), lock.horizon_lock.clone())
                };
                params.gyro.recompute_smoothness(smoothing.as_mut(), horizon_lock, &stabilization_params.read());

                if current_compute_id.load(SeqCst) != compute_id { return; }

                let mut lib_gyro = gyro.write();
                lib_gyro.quaternions = params.gyro.quaternions.clone();
                lib_gyro.smoothed_quaternions = params.gyro.smoothed_quaternions.clone();
                lib_gyro.max_angles = params.gyro.max_angles;
                lib_gyro.org_smoothed_quaternions = params.gyro.org_smoothed_quaternions.clone();
                lib_gyro.smoothing_status = smoothing.get_status_json();
                smoothing_changed = true;
            }
            
            if current_compute_id.load(SeqCst) != compute_id { return; }

            let mut zoom = zooming::from_compute_params(params.clone());
            if smoothing_changed || zooming::get_checksum(&zoom) != zooming_checksum.load(SeqCst) {
                params.fovs = Self::recompute_adaptive_zoom_static(&mut zoom, &stabilization_params);

                if current_compute_id.load(SeqCst) != compute_id { return; }

                let mut stab_params = stabilization_params.write();
                stab_params.set_fovs(params.fovs.clone(), params.lens_fov_adjustment);
                stab_params.zooming_debug_points = zoom.get_debug_points();
            }
            
            if current_compute_id.load(SeqCst) != compute_id { return; }

            undistortion.write().set_compute_params(params);

            smoothing_checksum.store(smoothing.read().get_state_checksum(), SeqCst);
            zooming_checksum.store(zooming::get_checksum(&zoom), SeqCst);
            cb(compute_id);
        });
        compute_id
    }

    pub fn get_features_pixels(&self, frame: usize) -> Option<Vec<(i32, i32, f32)>> { // (x, y, alpha)
        let mut ret = None;
        if self.params.read().show_detected_features {
            let (xs, ys) = self.pose_estimator.get_points_for_frame(&frame);
            for i in 0..xs.len() {
                if ret.is_none() {
                    // Only allocate if we actually have any points
                    ret = Some(Vec::with_capacity(2048));
                }
                for xstep in -1..=1i32 {
                    for ystep in -1..=1i32 {
                        ret.as_mut().unwrap().push((xs[i] as i32 + xstep, ys[i] as i32 + ystep, 1.0));
                    }
                }
            }
        }
        ret
    }
    pub fn get_opticalflow_pixels(&self, frame: usize) -> Option<Vec<(i32, i32, f32)>> { // (x, y, alpha)
        let mut ret = None;
        if self.params.read().show_optical_flow {
            for i in 0..3 {
                let a = (3 - i) as f32 / 3.0;
                if let Some(lines) = self.pose_estimator.get_of_lines_for_frame(&(frame + i), 1.0, 1) {
                    lines.0.into_iter().zip(lines.1.into_iter()).for_each(|(p1, p2)| {
                        if ret.is_none() {
                            // Only allocate if we actually have any points
                            ret = Some(Vec::with_capacity(2048));
                        }
                        let line = line_drawing::Bresenham::new((p1.0 as isize, p1.1 as isize), (p2.0 as isize, p2.1 as isize)); 
                        for point in line {
                            ret.as_mut().unwrap().push((point.0 as i32, point.1 as i32, a));
                        }
                    });
                }
            }
        }
        ret
    }

    pub unsafe fn fill_undistortion_data(&self, timestamp_us: i64, out_ptr: *mut f32, out_size: usize) -> bool {
        if self.params.read().stab_enabled {
            let mut undist = self.undistortion.write();
            if let Some(itm) = undist.get_undistortion_data(timestamp_us) {

                let params_count = itm.params.len() * 9;
                if params_count <= out_size {
                    let src_ptr = itm.params.as_ptr() as *const f32;
                    std::ptr::copy_nonoverlapping(src_ptr, out_ptr, params_count);

                    drop(itm);

                    self.current_fov_10000.store((undist.current_fov * 10000.0) as u64, SeqCst);
                    return true;
                }
            }
        }
        false
    }

    pub fn process_pixels(&self, mut timestamp_us: i64, width: usize, height: usize, stride: usize, out_width: usize, out_height: usize, out_stride: usize, pixels: &mut [u8], out_pixels: &mut [u8]) -> bool {
        let (enabled, ow, oh, framebuffer_inverted, fps, fps_scale, is_calibrator, fov) = {
            let params = self.params.read();
            (params.stab_enabled, params.output_size.0, params.output_size.1, params.framebuffer_inverted, params.get_scaled_fps(), params.fps_scale, params.is_calibrator, params.fov)
        };
        if enabled && ow == out_width && oh == out_height {
            if let Some(scale) = fps_scale {
                timestamp_us = (timestamp_us as f64 / scale).round() as i64;
            }
            let frame = frame_at_timestamp(timestamp_us as f64 / 1000.0, fps) as usize; // used only to draw features and OF
            //////////////////////////// Draw detected features ////////////////////////////
            // TODO: maybe handle other types than RGBA8?
            if T::COUNT == 4 && T::SCALAR_BYTES == 1 {
                if let Some(pxs) = self.get_features_pixels(frame) {
                    for (x, mut y, _) in pxs {
                        if framebuffer_inverted { y = height as i32 - y; }
                        let pos = (y * stride as i32 + x * (T::COUNT * T::SCALAR_BYTES) as i32) as usize;
                        if pixels.len() > pos + 2 { 
                            pixels[pos + 0] = 0x0c; // R
                            pixels[pos + 1] = 0xff; // G
                            pixels[pos + 2] = 0x00; // B
                        }
                    }
                }
                if let Some(pxs) = self.get_opticalflow_pixels(frame) {
                    for (x, mut y, a) in pxs {
                        if framebuffer_inverted { y = height as i32 - y; }
                        let pos = (y * stride as i32 + x * (T::COUNT * T::SCALAR_BYTES) as i32) as usize;
                        if pixels.len() > pos + 2 { 
                            pixels[pos + 0] = (pixels[pos + 0] as f32 * (1.0 - a) + 0xfe as f32 * a) as u8; // R
                            pixels[pos + 1] = (pixels[pos + 1] as f32 * (1.0 - a) + 0xfb as f32 * a) as u8; // G
                            pixels[pos + 2] = (pixels[pos + 2] as f32 * (1.0 - a) + 0x47 as f32 * a) as u8; // B
                        }
                    }
                }

                #[cfg(feature = "opencv")]
                if is_calibrator {
                    let lock = self.lens_calibrator.read();
                    if let Some(ref cal) = *lock {
                        let points = cal.all_matches.read();
                        if let Some(entry) = points.get(&(frame as i32)) {
                            let (w, h, s) = (width as u32, height as u32, stride);
                            calibration::drawing::draw_chessboard_corners(cal.width, cal.height, w, h, s, pixels, (cal.columns, cal.rows), &entry.points, true);
                        }
                    }
                }
            }
            //////////////////////////// Draw detected features ////////////////////////////
            let mut undist = self.undistortion.write();
            let ret = undist.process_pixels(timestamp_us, width, height, stride, out_width, out_height, out_stride, pixels, out_pixels);
            if ret {
                //////////////////////////// Draw zooming debug pixels ////////////////////////////
                let p = self.params.read();
                if !p.zooming_debug_points.is_empty() {
                    if let Some((_, points)) = p.zooming_debug_points.range(timestamp_us..).next() {
                        for i in 0..points.len() {
                            let fov = (fov * p.fovs.get(frame).unwrap_or(&1.0)).max(0.0001);
                            let mut pt = points[i];
                            let width_ratio = width as f64 / out_width as f64;
                            let height_ratio = height as f64 / out_height as f64;
                            pt = (pt.0 - 0.5, pt.1 - 0.5);
                            pt = (pt.0 / fov * width_ratio, pt.1 / fov * height_ratio);
                            pt = (pt.0 + 0.5, pt.1 + 0.5);
                            for xstep in -2..=2i32 {
                                for ystep in -2..=2i32 {
                                    let (x, y) = ((pt.0 * out_width as f64) as i32 + xstep, (pt.1 * out_height as f64) as i32 + ystep);
                                    if x >= 0 && y >= 0 && x < out_width as i32 && y < out_height as i32 {
                                        let pos = (y * out_stride as i32 + x * (T::COUNT * T::SCALAR_BYTES) as i32) as usize;
                                        if out_pixels.len() > pos + 2 { 
                                            out_pixels[pos + 0] = 0xff; // R
                                            out_pixels[pos + 1] = 0x00; // G
                                            out_pixels[pos + 2] = 0x00; // B
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                //////////////////////////// Draw zooming debug pixels ////////////////////////////
            }
            self.current_fov_10000.store((undist.current_fov * 10000.0) as u64, SeqCst);
            ret
        } else {
            false
        }
    }

    pub fn set_video_rotation(&self, v: f64) { self.params.write().video_rotation = v; }

    pub fn set_trim_start(&self, v: f64) { self.params.write().trim_start = v; self.invalidate_smoothing(); }
    pub fn set_trim_end  (&self, v: f64) { self.params.write().trim_end   = v; self.invalidate_smoothing(); }

    pub fn set_show_detected_features(&self, v: bool) { self.params.write().show_detected_features = v; }
    pub fn set_show_optical_flow     (&self, v: bool) { self.params.write().show_optical_flow      = v; }
    pub fn set_stab_enabled          (&self, v: bool) { self.params.write().stab_enabled           = v; }
    pub fn set_frame_readout_time    (&self, v: f64)  { self.params.write().frame_readout_time     = v; }
    pub fn set_adaptive_zoom         (&self, v: f64)  { self.params.write().adaptive_zoom_window   = v; }
    pub fn set_fov                   (&self, v: f64)  { self.params.write().fov                    = v; }
    pub fn set_lens_correction_amount(&self, v: f64)  { self.params.write().lens_correction_amount = v; self.invalidate_zooming(); }
    pub fn set_background_mode       (&self, v: i32)  { self.params.write().background_mode = stabilization_params::BackgroundMode::from(v); }

    pub fn set_input_horizontal_stretch(&self, v: f64) { self.lens.write().input_horizontal_stretch = v; self.invalidate_zooming(); }
    pub fn set_input_vertical_stretch  (&self, v: f64) { self.lens.write().input_vertical_stretch   = v; self.invalidate_zooming(); }

    pub fn get_scaling_ratio         (&self) -> f64 { let params = self.params.read(); params.video_size.0 as f64 / params.video_output_size.0 as f64 }
    pub fn get_current_fov           (&self) -> f64 { self.current_fov_10000.load(SeqCst) as f64 / 10000.0 }
    pub fn get_min_fov               (&self) -> f64 { self.params.read().min_fov }

    pub fn invalidate_smoothing(&self) { self.smoothing_checksum.store(0, SeqCst); }
    pub fn invalidate_zooming(&self) { self.zooming_checksum.store(0, SeqCst); }

    pub fn remove_offset(&self, timestamp_us: i64) {
        self.gyro.write().remove_offset(timestamp_us);
        self.invalidate_zooming();
    }
    pub fn set_offset(&self, timestamp_us: i64, offset_ms: f64) {
        self.gyro.write().set_offset(timestamp_us, offset_ms);
        self.invalidate_zooming();
    }
    pub fn clear_offsets(&self) {
        self.gyro.write().offsets.clear();
        self.invalidate_zooming();
    }
    pub fn offset_at_timestamp(&self, timestamp_us: i64) -> f64 {
        self.gyro.read().offset_at_timestamp(timestamp_us as f64 / 1000.0)
    }

    pub fn set_imu_lpf(&self, lpf: f64) {
        self.gyro.write().set_lowpass_filter(lpf);
        self.smoothing.write().update_quats_checksum(&self.gyro.read().quaternions);
    }
    pub fn set_imu_rotation(&self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64) {
        self.gyro.write().set_imu_rotation(pitch_deg, roll_deg, yaw_deg);
        self.smoothing.write().update_quats_checksum(&self.gyro.read().quaternions);
    }
    pub fn set_imu_orientation(&self, orientation: String) {
        let mut gyro = self.gyro.write();
        let mut smoothing = self.smoothing.write();
        gyro.set_imu_orientation(orientation);
        smoothing.update_quats_checksum(&gyro.quaternions);
    }
    pub fn set_sync_lpf(&self, lpf: f64) {
        let params = self.params.read();
        self.pose_estimator.lowpass_filter(lpf, params.frame_count, params.duration_ms, params.fps);
    }
    pub fn set_imu_bias(&self, bx: f64, by: f64, bz: f64) {
        self.gyro.write().set_bias(bx, by, bz);
        self.smoothing.write().update_quats_checksum(&self.gyro.read().quaternions);
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
        self.undistortion.write().set_background(bg);
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
    pub fn get_smoothing_max_angles(&self) -> (f64, f64, f64) {
        self.gyro.read().max_angles
    }
    pub fn get_smoothing_status(&self) -> serde_json::Value {
        self.gyro.read().smoothing_status.clone()
    }
    pub fn get_smoothing_algs(&self) -> Vec<String> {
        self.smoothing.read().get_names()
    }

    pub fn get_render_stabilizer(&self, output_size: (usize, usize)) -> StabilizationManager<T> {
        let size = self.params.read().video_size;
        let stab = StabilizationManager {
            params: Arc::new(RwLock::new(self.params.read().clone())),
            gyro:   Arc::new(RwLock::new(self.gyro.read().clone())),
            lens:   Arc::new(RwLock::new(self.lens.read().clone())),
            video_path: Arc::new(RwLock::new(self.video_path.read().clone())),
            ..Default::default()
        };
        stab.params.write().framebuffer_inverted = false;
        stab.set_size(size.0, size.1);
        stab.set_output_size(output_size.0, output_size.1);

        stab.recompute_undistortion();

        stab
    }

    pub fn clear(&self) {
        let (stab_enabled, show_detected_features, show_optical_flow, background, adaptive_zoom_window, framebuffer_inverted, lens_correction_amount, background_mode) = {
            let params = self.params.read();
            (params.stab_enabled, params.show_detected_features, params.show_optical_flow, params.background, params.adaptive_zoom_window, params.framebuffer_inverted, params.lens_correction_amount, params.background_mode)
        };

        *self.params.write() = StabilizationParams {
            stab_enabled, show_detected_features, show_optical_flow, background, adaptive_zoom_window, framebuffer_inverted, lens_correction_amount, background_mode, ..Default::default()
        };
        if !self.gyro.read().prevent_next_load {
            *self.gyro.write() = GyroSource::new();
        }
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
        }

        self.undistortion.write().set_compute_params(undistortion::ComputeParams::from_manager(self));

        self.invalidate_smoothing();
    }

    pub fn export_gyroflow(&self, filepath: impl AsRef<std::path::Path>, thin: bool, output_options: String) -> std::io::Result<()> {
        use std::io::{ BufWriter, Write };

        let gyro = self.gyro.read();
        let params = self.params.read();

        let (smoothing_name, smoothing_params) = {
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

            (smoothing.get_name(), parameters)
        };

        let render_options: serde_json::Value = serde_json::from_str(&output_options).unwrap_or_default();

        let video_path = self.video_path.read().clone();
        
        let obj = serde_json::json!({
            "title": "Gyroflow data file",
            "version": 2,
            "app_version": env!("CARGO_PKG_VERSION").to_string(),
            "videofile": video_path,
            "calibration_data": self.lens.read().get_json_value().unwrap_or_else(|_| serde_json::json!({})),
            "date": chrono::Local::today().naive_local().to_string(),

            "background_color": params.background.as_slice(),
            "background_mode":  params.background_mode as i32,
    
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
                // "adaptive_zoom_fovs":     if !thin { util::compress_to_base91(&params.fovs) } else { None },
                "lens_correction_amount": params.lens_correction_amount,
            },
            "gyro_source": {
                "filepath":           gyro.file_path,
                "lpf":                gyro.imu_lpf,
                "rotation":           gyro.imu_rotation_angles,
                "imu_orientation":    gyro.imu_orientation,
                "gyro_bias":          gyro.gyro_bias,
                "integration_method": gyro.integration_method,
                "raw_imu":            if !thin { util::compress_to_base91(&gyro.org_raw_imu) } else { None },
                "quaternions":        if !thin && video_path != gyro.file_path { util::compress_to_base91(&gyro.org_quaternions) } else { None },
                "gravity_vectors":    if !thin && video_path != gyro.file_path && gyro.gravity_vectors.is_some() { util::compress_to_base91(gyro.gravity_vectors.as_ref().unwrap()) } else { None },
                // "smoothed_quaternions": smooth_quats
            },
            "output": render_options,
            /*"autosync": { // TODO
                // TODO: input settings?
                "camera_matrix": {}, // frame, Matrix3
                "euler_angles": {} // frame, rotation vector
            },*/
            "offsets": gyro.offsets, // timestamp, offset value

            "trim_start": params.trim_start,
            "trim_end":   params.trim_end,

            // "frame_orientation": {}, // timestamp, original frame quaternion
            // "stab_transform":    {} // timestamp, final quaternion
        });
        
        let file = std::fs::File::create(filepath)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &obj)?;
        writer.flush()?;

        Ok(())
    }
    pub fn import_gyroflow(&self, path: &str, blocking: bool) -> std::io::Result<serde_json::Value> {
        let get_new_path = |file_path: &str| -> PathBuf {
            let mut file_path = std::path::Path::new(file_path).to_path_buf();
            if !file_path.exists() {
                if let Some(filename) = file_path.file_name() {
                    let new_path = std::path::Path::new(path).with_file_name(filename);
                    if new_path.exists() {
                        file_path = new_path;
                    }
                }
            }
            file_path
        };

        let data = std::fs::read(path)?;
        let mut obj: serde_json::Value = serde_json::from_slice(&data)?;
        if let serde_json::Value::Object(ref mut obj) = obj {
            let mut output_size = None;
            let org_video_path = obj.get("videofile").and_then(|x| x.as_str()).unwrap_or(&"").to_string();

            let video_path = get_new_path(&org_video_path);
            obj["videofile"] = serde_json::Value::String(util::path_to_str(&video_path));

            if let Some(vid_info) = obj.get("video_info") {
                let mut params = self.params.write();
                params.video_size = (
                    vid_info.get("width") .and_then(|x| x.as_u64()).unwrap_or_default() as usize,
                    vid_info.get("height").and_then(|x| x.as_u64()).unwrap_or_default() as usize
                );
                output_size = Some(params.video_size);
                params.video_rotation = vid_info.get("rotation")   .and_then(|x| x.as_f64()).unwrap_or_default();
                params.frame_count    = vid_info.get("num_frames") .and_then(|x| x.as_u64()).unwrap_or_default() as usize;
                params.fps            = vid_info.get("fps")        .and_then(|x| x.as_f64()).unwrap_or_default();
                params.duration_ms    = vid_info.get("duration_ms").and_then(|x| x.as_f64()).unwrap_or_default();
                params.fps_scale      = vid_info.get("fps_scale")  .and_then(|x| x.as_f64());

                self.gyro.write().init_from_params(&params);
            }
            if let Some(lens) = obj.get("calibration_data") {
                self.lens.write().load_from_json_value(&lens);
            }
            obj.remove("frame_orientation");
            obj.remove("stab_transform");
            if let Some(serde_json::Value::Object(ref mut obj)) = obj.get_mut("gyro_source") {
                let org_gyro_path = obj.get("filepath").and_then(|x| x.as_str()).unwrap_or(&"").to_string();
                let gyro_path = get_new_path(&org_gyro_path);
                obj["filepath"] = serde_json::Value::String(util::path_to_str(&gyro_path));
                use crate::gyro_source::TimeIMU;

                let is_compressed = obj.get("raw_imu").map(|x| x.is_string()).unwrap_or_default();

                // Load IMU data only if it's from another file
                if org_gyro_path != org_video_path {
                    let mut raw_imu = None;
                    let mut quaternions = None;
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
                            raw_imu,
                            frame_readout_time: None,
                            camera_identifier: None,
                        };

                        let mut gyro = self.gyro.write();
                        gyro.prevent_next_load = true;
                        gyro.load_from_telemetry(&md);
                    } else if gyro_path.exists() {
                        if let Err(e) = self.load_gyro_data(&util::path_to_str(&gyro_path)) {
                            ::log::warn!("Failed to load gyro data from {:?}: {:?}", gyro_path, e);
                        }
                    }
                } else if gyro_path.exists() {
                    if let Err(e) = self.load_gyro_data(&util::path_to_str(&gyro_path)) {
                        ::log::warn!("Failed to load gyro data from {:?}: {:?}", gyro_path, e);
                    }
                }

                let mut gyro = self.gyro.write();
                gyro.file_path           = util::path_to_str(&gyro_path);
                gyro.imu_lpf             = obj.get("lpf").and_then(|x| x.as_f64()).unwrap_or_default();
                gyro.integration_method  = obj.get("integration_method").and_then(|x| x.as_u64()).unwrap_or(1) as usize;
                if let Some(v) = obj.get("rotation")  { gyro.imu_rotation_angles = serde_json::from_value(v.clone()).ok(); }
                if let Some(v) = obj.get("gyro_bias") { gyro.gyro_bias           = serde_json::from_value(v.clone()).ok(); }

                if blocking {
                    gyro.apply_transforms();
                    gyro.integrate();
                }
                self.smoothing.write().update_quats_checksum(&gyro.quaternions);

                obj.remove("raw_imu");
                obj.remove("quaternions");
                obj.remove("smoothed_quaternions");
                obj.remove("gravity_vectors");
            }
            if let Some(serde_json::Value::Object(ref mut obj)) = obj.get_mut("stabilization") {
                let mut params = self.params.write();
                params.fov                     = obj.get("fov")                   .and_then(|x| x.as_f64()).unwrap_or(1.0);
                params.frame_readout_time      = obj.get("frame_readout_time")    .and_then(|x| x.as_f64()).unwrap_or_default();
                params.adaptive_zoom_window    = obj.get("adaptive_zoom_window")  .and_then(|x| x.as_f64()).unwrap_or_default();
                params.lens_correction_amount  = obj.get("lens_correction_amount").and_then(|x| x.as_f64()).unwrap_or(1.0);

                let method = obj.get("method").and_then(|x| x.as_str()).unwrap_or_default();
                let method_idx = self.get_smoothing_algs()
                    .iter().enumerate()
                    .find(|(_, m)| method == m.as_str())
                    .map(|(idx, _)| idx)
                    .unwrap_or(1);

                let mut smoothing = self.smoothing.write();
                smoothing.set_current(method_idx);

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

                obj.remove("adaptive_zoom_fovs");
            }
            if let Some(serde_json::Value::Object(ref obj)) = obj.get("output") {
                output_size = Some((
                    obj.get("output_width") .and_then(|x| x.as_u64()).unwrap_or_default() as usize,
                    obj.get("output_height").and_then(|x| x.as_u64()).unwrap_or_default() as usize
                ));
            }

            if let Some(serde_json::Value::Object(offsets)) = obj.get("offsets") {
                self.gyro.write().offsets = offsets.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_f64()?))).collect();
            }

            *self.video_path.write() = util::path_to_str(&video_path);

            if blocking {
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
}

pub fn timestamp_at_frame(frame: i32, fps: f64) -> f64 { frame as f64 * 1000.0 / fps }
pub fn frame_at_timestamp(timestamp_ms: f64, fps: f64) -> i32 { (timestamp_ms * (fps / 1000.0)).round() as i32 }

pub fn run_threaded<F>(cb: F) where F: FnOnce() + Send + 'static {
    THREAD_POOL.spawn(cb);
}
