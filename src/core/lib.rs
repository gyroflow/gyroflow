pub mod gyro_source;
pub mod integration;
pub mod integration_complementary; // TODO: add this to `ahrs` crate
pub mod lens_profile;
pub mod synchronization;
pub mod undistortion;
pub mod adaptive_zoom;

pub mod smoothing;
pub mod filtering;

pub mod gpu;

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::SeqCst;
use parking_lot::RwLock;
pub use undistortion::PixelType;

use self::{ lens_profile::LensProfile, smoothing::Smoothing, undistortion::Undistortion, adaptive_zoom::AdaptiveZoom };

use simd_json::ValueAccess;
use nalgebra::{ Quaternion, Vector3, Vector4 };
use gyro_source::{ GyroSource, Quat64, TimeIMU };
use telemetry_parser::try_block;

lazy_static::lazy_static! {
    static ref THREAD_POOL: rayon::ThreadPool = rayon::ThreadPoolBuilder::new().build().unwrap();
    static ref CURRENT_COMPUTE_ID: AtomicU64 = AtomicU64::new(0);
}

#[derive(Clone)]
pub struct BasicParams {
    pub size: (usize, usize), // Processing input size
    pub output_size: (usize, usize), // Processing output size
    pub video_size: (usize, usize), // Full resolution input size
    pub video_output_size: (usize, usize), // Full resoution output size

    pub background: Vector4<f32>,

    pub frame_readout_time: f64,
    pub adaptive_zoom_window: f64,
    pub fov: f64,
    pub fovs: Vec<f64>,
    pub fps: f64,
    pub frame_count: usize,
    pub duration_ms: f64,

    pub trim_start: f64,
    pub trim_end: f64,

    pub video_rotation: f64,
    
    pub stab_enabled: bool,
    pub show_detected_features: bool,
}
impl Default for BasicParams {
    fn default() -> Self {
        Self {
            fov: 1.0,
            fovs: vec![],
            stab_enabled: true,
            show_detected_features: true,
            frame_readout_time: 0.0, 
            adaptive_zoom_window: 0.0, 

            size: (0, 0),
            output_size: (0, 0),
            video_size: (0, 0),
            video_output_size: (0, 0),

            video_rotation: 0.0,

            trim_start: 0.0,
            trim_end: 1.0,
        
            background: Vector4::new(0.0, 0.0, 0.0, 0.0),
    
            fps: 0.0,
            frame_count: 0,
            duration_ms: 0.0,
        }
    }
}
pub struct StabilizationManager<T: PixelType> {
    pub gyro: Arc<RwLock<GyroSource>>,
    pub lens: Arc<RwLock<LensProfile>>,
    pub smoothing: Arc<RwLock<Smoothing>>,

    pub undistortion: Arc<RwLock<Undistortion<T>>>,

    pub pose_estimator: Arc<synchronization::PoseEstimator>,

    pub params: Arc<RwLock<BasicParams>>
}

impl<T: PixelType> Default for StabilizationManager<T> {
    fn default() -> Self {
        Self {
            smoothing: Arc::new(RwLock::new(Smoothing::default())),

            params: Arc::new(RwLock::new(BasicParams::default())),
            
            undistortion: Arc::new(RwLock::new(Undistortion::<T>::default())),
            gyro: Arc::new(RwLock::new(GyroSource::new())),
            lens: Arc::new(RwLock::new(LensProfile::default())),
            
            pose_estimator: Arc::new(synchronization::PoseEstimator::default())
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

        self.load_gyro_data(path)?;
        
        Ok(())
    }

    pub fn load_gyro_data(&self, path: &str) -> std::io::Result<()> {
        {
            let params = self.params.read();
            let mut gyro = self.gyro.write(); 
            gyro.fps = params.fps;
            gyro.duration_ms = params.duration_ms;
            gyro.offsets.clear();
        }

        if path.ends_with(".gyroflow") {
            let mut data = std::fs::read(path)?;
            let v = simd_json::to_borrowed_value(&mut data)?;
    
            self.lens.write().load_from_json_value(&v["calibration_data"]);

            let to_f64_array = |x: &simd_json::borrowed::Value| -> Option<Vec<f64>> { Some(x.as_array()?.iter().filter_map(|x| x.as_f64()).collect()) };

            try_block!({
                let smoothed_quaternions = v["stab_transform"].as_array()?.iter().filter_map(to_f64_array)
                    .map(|x| ((x[0] * 1000.0) as i64, Quat64::from_quaternion(Quaternion::from_parts(x[3], Vector3::new(x[4], x[5], x[6])))))
                    .collect();
        
                let quaternions = v["frame_orientation"].as_array()?.iter().filter_map(to_f64_array)
                    .map(|x| ((x[0] * 1000.0) as i64, Quat64::from_quaternion(Quaternion::from_parts(x[3-1], Vector3::new(x[4-1], x[5-1], x[6-1])))))
                    .collect();
        
                let raw_imu = v["raw_imu"].as_array()?.iter().filter_map(to_f64_array)
                    .map(|x| TimeIMU { timestamp_ms: 0.0/*TODO*/, gyro: Some([x[0], x[1], x[2]]), accl: Some([x[3], x[4], x[6]]), magn: None }) // TODO IMU orientation
                    .collect();
                {
                    let mut gyro = self.gyro.write(); 
                    gyro.smoothed_quaternions = smoothed_quaternions;
                    gyro.quaternions = quaternions;
                    gyro.raw_imu = raw_imu;
                }
            });
        } else {
            let md = GyroSource::parse_telemetry_file(path)?;
            self.gyro.write().load_from_telemetry(&md);
            self.params.write().frame_readout_time = md.frame_readout_time.unwrap_or_default();
        }
        Ok(())
    }

    pub fn load_lens_profile(&self, path: &str) {
        self.lens.write().load_from_file(path); // TODO Result
    }

    pub fn camera_matrix_or_default(&self) -> Vec<f64> {
        let matrix = self.lens.read().camera_matrix.clone();
        if matrix.len() == 9 {
            matrix
        } else {
            let params = self.params.read();
            vec![
                params.size.0 as f64, 0.0, params.size.0 as f64 / 2.0,
                0.0, params.size.0 as f64, params.size.1 as f64 / 2.0,
                0.0, 0.0, 1.0
            ]
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
        }
    }

    pub fn set_size(&self, width: usize, height: usize) {
        {
            let mut params = self.params.write();
            params.size = (width, height);

            let ratio = params.size.0 as f64 / params.video_size.0 as f64;
            params.output_size = ((params.video_output_size.0 as f64 * ratio) as usize, (params.video_output_size.1 as f64 * ratio) as usize);
        }
        self.init_size();
    }
    pub fn set_output_size(&self, width: usize, height: usize) {
        {
            let mut params = self.params.write();
            let ratio = params.size.0 as f64 / params.video_size.0 as f64;
            params.output_size = ((width as f64 * ratio) as usize, (height as f64 * ratio) as usize);
            params.video_output_size = (width, height);
        }
        self.init_size();
    }

    pub fn recompute_adaptive_zoom(&self) {
        let (window, frames, video_size, ratio, fps, trim) = {
            let params = self.params.read();
            (params.adaptive_zoom_window, params.frame_count, params.output_size, params.size.0 as f64 / params.video_size.0 as f64, params.fps, (params.trim_start, params.trim_end))
        };
        if window > 0.0 || window < -0.9 {
            let mut quats = Vec::with_capacity(frames);
            {
                let g = self.gyro.read();
                for i in 0..frames {
                    quats.push(g.smoothed_quat_at_timestamp(i as f64 * 1000.0 / fps));
                }
            }
    
            let mode = if window < 0.0 {
                adaptive_zoom::Mode::StaticZoom
            } else {
                adaptive_zoom::Mode::DynamicZoom(window)
            };

            let zoom = AdaptiveZoom::from_manager(self);
            let fovs = zoom.compute(&quats, video_size, fps, mode, trim);
            self.params.write().fovs = fovs.iter().map(|v| v.0 * ratio).collect();
        } else {
            self.params.write().fovs.clear();
        }
    }

    pub fn recompute_smoothness(&self) {
        let params = self.params.read();
        self.gyro.write().recompute_smoothness(self.smoothing.write().current().as_ref(), params.duration_ms);
    }

    pub fn recompute_undistortion(&self) {
        let params = undistortion::ComputeParams::from_manager(self);
        self.undistortion.write().recompute(&params);
    }

    pub fn recompute_blocking(&self) {
        self.recompute_smoothness();
        self.recompute_adaptive_zoom();
        self.recompute_undistortion();
    }
    
    pub fn recompute_threaded<F: Fn(u64) + Send + Sync + Clone + 'static>(&self, cb: F) -> u64 {
        self.recompute_smoothness();
        self.recompute_adaptive_zoom();
        let params = undistortion::ComputeParams::from_manager(self);

        let compute_id = fastrand::u64(..);
        CURRENT_COMPUTE_ID.store(compute_id, SeqCst);

        let undistortion = self.undistortion.clone();
        THREAD_POOL.spawn(move || {
            if let Ok(stab_data) = undistortion::Undistortion::<T>::calculate_stab_data(&params, &CURRENT_COMPUTE_ID, compute_id) {
                undistortion.write().stab_data = stab_data;

                cb(compute_id);
            }
        });
        compute_id
    }

    pub fn process_pixels(&self, frame: usize, width: usize, height: usize, stride: usize, out_width: usize, out_height: usize, out_stride: usize, pixels: &mut [u8], out_pixels: &mut [u8]) -> bool { // TODO: generic
        let (enabled, show_features, ow, oh) = {
            let params = self.params.read();
            (params.stab_enabled, params.show_detected_features, params.output_size.0, params.output_size.1)
        };
        if enabled && ow == out_width && oh == out_height {
            if show_features {
                //////////////////////////// Draw detected features ////////////////////////////
                // TODO: maybe handle other types than RGBA8?
                if T::COUNT == 4 && T::SCALAR_BYTES == 1 {
                    let (xs, ys) = self.pose_estimator.get_points_for_frame(&frame);
                    for i in 0..xs.len() {
                        for xstep in -1..=1i32 {
                            for ystep in -1..=1i32 {
                                let pos = ((ys[i] as i32 + ystep) * stride as i32 + (xs[i] as i32 + xstep) * (T::COUNT * T::SCALAR_BYTES) as i32) as usize;
                                if pixels.len() > pos + 2 { 
                                    pixels[pos + 0] = 0x0c; // R
                                    pixels[pos + 1] = 0xff; // G
                                    pixels[pos + 2] = 0x00; // B
                                }
                            }
                        }
                    }
                }
                //////////////////////////// Draw detected features ////////////////////////////
            }
            
            self.undistortion.write().process_pixels(frame, width, height, stride, out_width, out_height, out_stride, pixels, out_pixels)
        } else {
            false
        }
    }

    pub fn set_video_rotation(&self, v: f64) { self.params.write().video_rotation = v; }

    pub fn set_trim_start(&self, v: f64) { self.params.write().trim_start = v; }
    pub fn set_trim_end  (&self, v: f64) { self.params.write().trim_end   = v; }

    pub fn set_show_detected_features(&self, v: bool) { self.params.write().show_detected_features = v; }
    pub fn set_stab_enabled          (&self, v: bool) { self.params.write().stab_enabled           = v; }
    pub fn set_frame_readout_time    (&self, v: f64)  { self.params.write().frame_readout_time     = v; }
    pub fn set_adaptive_zoom         (&self, v: f64)  { self.params.write().adaptive_zoom_window   = v; }
    pub fn set_fov                   (&self, v: f64)  { self.params.write().fov                    = v; }

    pub fn remove_offset      (&self, timestamp_us: i64)                 { self.gyro.write().remove_offset(timestamp_us); }
    pub fn set_offset         (&self, timestamp_us: i64, offset_ms: f64) { self.gyro.write().set_offset(timestamp_us, offset_ms); }
    pub fn offset_at_timestamp(&self, timestamp_us: i64) -> f64          { self.gyro.read() .offset_at_timestamp(timestamp_us as f64 / 1000.0) }

    pub fn set_imu_lpf(&self, lpf: f64) { self.gyro.write().set_lowpass_filter(lpf); }
    pub fn set_imu_rotation(&self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64) { self.gyro.write().set_imu_rotation(pitch_deg, roll_deg, yaw_deg); }
    pub fn set_imu_orientation(&self, orientation: String) { self.gyro.write().set_imu_orientation(orientation); }
    pub fn set_sync_lpf(&self, lpf: f64) {
        let params = self.params.read();
        self.pose_estimator.lowpass_filter(lpf, params.frame_count, params.duration_ms, params.fps);
    }

    pub fn timestamp_at_frame(&self, frame: usize, fps: f64) -> f64   { frame as f64 * fps * 1000.0 }
    pub fn frame_at_timestamp(&self, ts: f64,      fps: f64) -> usize { (ts / 1000.0 * fps).ceil() as usize }

    pub fn set_lens_param(&self, param: &str, value: f64) {
        let mut lens = self.lens.write();
        if lens.distortion_coeffs.len() >= 4 && lens.camera_matrix.len() >= 9 {
            match param{
                "fx" => lens.camera_matrix[0] = value,
                "fy" => lens.camera_matrix[4] = value,
                "k1" => lens.distortion_coeffs[0] = value,
                "k2" => lens.distortion_coeffs[1] = value,
                "k3" => lens.distortion_coeffs[2] = value,
                "k4" => lens.distortion_coeffs[3] = value,
                _ => { }
            }
        }
    }

    pub fn set_background_color(&self, bg: Vector4<f32>) {
        self.params.write().background = bg;
        self.undistortion.write().set_background(bg);
    }

    pub fn set_smoothing_method(&self, index: usize) -> simd_json::owned::Value {
        let mut smooth = self.smoothing.write();
        smooth.set_current(index);
        smooth.current().get_parameters_json()
    }
    pub fn set_smoothing_param(&self, name: &str, val: f64) {
        self.smoothing.write().current().as_mut().set_parameter(name, val);
    }
    pub fn get_smoothing_algs(&self) -> Vec<String> {
        self.smoothing.read().get_names()
    }

    pub fn get_render_stabilizator(&self, output_size: (usize, usize)) -> StabilizationManager<T> {
        let size = self.params.read().video_size;
        let stab = StabilizationManager {
            params: Arc::new(RwLock::new(self.params.read().clone())),
            gyro:   Arc::new(RwLock::new(self.gyro.read().clone())),
            lens:   Arc::new(RwLock::new(self.lens.read().clone())),
            ..Default::default()
        };
        stab.set_size(size.0, size.1);
        stab.set_output_size(output_size.0, output_size.1);

        stab.recompute_undistortion();

        stab
    }

    pub fn run_threaded<F>(cb: F) where F: FnOnce() + Send + 'static {
        THREAD_POOL.spawn(cb);
    }

    pub fn clear(&self) {
        *self.params.write() = BasicParams::default();
        *self.gyro.write() = GyroSource::new();
        self.pose_estimator.clear();
    }
}
