pub mod gyro_source;
pub mod integration;
pub mod integration_complementary; // TODO add this to `ahrs` crate
pub mod lens_profile;
pub mod synchronization;
pub mod undistortion;

pub mod smoothing;
pub mod filtering;

pub mod gpu;

use std::sync::Arc;

use self::{lens_profile::LensProfile, smoothing::{SmoothingAlgorithm, get_smoothing_algorithms}, undistortion::Undistortion};

use simd_json::ValueAccess;
use nalgebra::{Quaternion, Vector3, Vector4};
use gyro_source::{GyroSource, Quat64, TimeIMU};

pub struct StabilizationManager {
    pub gyro: GyroSource,
    pub lens: LensProfile,
    pub smoothing_id: usize,
    pub smoothing_algs: Vec<Box<dyn SmoothingAlgorithm>>,

    pub undistortion: Undistortion<undistortion::RGBA8>, // TODO generic

    pub size: (usize, usize),
    pub video_size: (usize, usize),

    pub background: Vector4<f32>,

    pub frame_readout_time: f64,
    pub fov: f64,
    pub fps: f64,
    pub frame_count: usize,
    pub duration_ms: f64,

    pub trim_start: f64,
    pub trim_end: f64,

    pub pose_estimator: Arc<synchronization::PoseEstimator>,    

    pub stab_enabled: bool,
    pub show_detected_features: bool,
}

impl Default for StabilizationManager {
    fn default() -> Self {
        Self {
            smoothing_id: 0,
            smoothing_algs: get_smoothing_algorithms(),
            fov: 1.0,
            stab_enabled: true,
            show_detected_features: true,
            frame_readout_time: 0.0, 
            undistortion: Undistortion::<undistortion::RGBA8>::default(),
            gyro: GyroSource::default(),
            lens: LensProfile::default(),
            
            size: (0, 0),
            video_size: (0, 0),

            trim_start: 0.0,
            trim_end: 1.0,
        
            background: Vector4::new(0.0, 0.0, 0.0, 0.0),
    
            fps: 0.0,
            frame_count: 0,
            duration_ms: 0.0,

            pose_estimator: Arc::new(synchronization::PoseEstimator::default())
        }
    }
}

impl StabilizationManager {
    pub fn init_from_video_data(&mut self, path: &str, duration_ms: f64, fps: f64, frame_count: usize, video_size: (usize, usize)) {
        self.fps = fps;
        self.frame_count = frame_count;
        self.duration_ms = duration_ms;
        self.video_size = video_size;

        self.pose_estimator.sync_results.write().clear();

        self.load_gyro_data(path);
    }

    pub fn load_gyro_data(&mut self, path: &str) -> Option<()> {
        self.gyro.fps = self.fps;
        self.gyro.duration_ms = self.duration_ms;
        self.gyro.offsets.clear();

        if path.ends_with(".gyroflow") {
            let mut data = std::fs::read(path).ok()?;
            let v = simd_json::to_borrowed_value(&mut data).ok()?;
    
            self.lens.load_from_json_value(&v["calibration_data"]);

            let to_f64_array = |x: &simd_json::borrowed::Value| -> Option<Vec<f64>> { Some(x.as_array()?.iter().filter_map(|x| x.as_f64()).collect()) };

            self.gyro.smoothed_quaternions = v["stab_transform"].as_array()?.iter().filter_map(to_f64_array)
                .map(|x| ((x[0] * 1000.0) as i64, Quat64::from_quaternion(Quaternion::from_parts(x[3], Vector3::new(x[4], x[5], x[6])))))
                .collect();
    
            self.gyro.quaternions = v["frame_orientation"].as_array()?.iter().filter_map(to_f64_array)
                .map(|x| ((x[0] * 1000.0) as i64, Quat64::from_quaternion(Quaternion::from_parts(x[3-1], Vector3::new(x[4-1], x[5-1], x[6-1])))))
                .collect();
    
            self.gyro.raw_imu = v["raw_imu"].as_array()?.iter().filter_map(to_f64_array)
                .map(|x| TimeIMU { timestamp: 0.0/*TODO*/, gyro: [x[0], x[1], x[2]], accl: [x[3], x[4], x[6]] }) // TODO IMU orientation
                .collect();
            Some(())
        } else {
            self.gyro.load_from_file(path).ok()
        }
    }

    pub fn load_lens_profile(&mut self, path: &str) {
        self.lens.load_from_file(path); // TODO Result
    }

    pub fn camera_matrix_or_default(&self) -> Vec<f64> {
        if self.lens.camera_matrix.len() == 9 {
            self.lens.camera_matrix.clone()
        } else {
            vec![
                self.size.0 as f64, 0.0, self.size.0 as f64 / 2.0,
                0.0, self.size.0 as f64, self.size.1 as f64 / 2.0,
                0.0, 0.0, 1.0
            ]
        }
    }

    pub fn init_size(&mut self, width: usize, height: usize) {
        self.size = (width, height);

        let params = undistortion::ComputeParams::from_manager(self);
        self.undistortion.init_size(self.background, &params, self.size.0);
    }

    pub fn recompute_smoothness(&mut self) {
        let s = self.smoothing_algs[self.smoothing_id].as_ref();
        self.gyro.smoothed_quaternions = s.smooth(&self.gyro.quaternions, self.duration_ms);
        self.gyro.org_smoothed_quaternions = self.gyro.smoothed_quaternions.clone();

        for (sq, q) in self.gyro.smoothed_quaternions.iter_mut().zip(self.gyro.quaternions.iter()) {
            // rotation quaternion from smooth motion -> raw motion to counteract it
            *sq.1 = sq.1.inverse() * q.1;
        }
    }

    pub fn recompute(&mut self) {
        self.recompute_smoothness();
        self.recompute_undistortion();
    }

    pub fn recompute_undistortion(&mut self) {
        let params = undistortion::ComputeParams::from_manager(self);
        self.undistortion.recompute(&params);
    }

    pub fn process_pixels(&mut self, frame: usize, width: usize, height: usize, stride: usize, pixels: &mut [u8]) -> *mut u8 { // TODO: generic
        if self.stab_enabled {
            if self.show_detected_features {
                //////////////////////////// Draw detected features ////////////////////////////
                let (xs, ys) = self.pose_estimator.get_points_for_frame(&frame);
                for i in 0..xs.len() {
                    for xstep in -1..=1i32 {
                        for ystep in -1..=1i32 {
                            let pos = ((ys[i] as i32 + ystep) * stride as i32 + (xs[i] as i32 + xstep)) as usize * 4;
                            pixels[pos + 0] = 0x0c;
                            pixels[pos + 1] = 0xff;
                            pixels[pos + 2] = 0x00;
                        }
                    }
                }
                //////////////////////////// Draw detected features ////////////////////////////
            }

            self.undistortion.process_pixels(frame, width, height, stride, pixels)
        } else {
            pixels.as_mut_ptr()
        }
    }

    pub fn timestamp_at_frame(&self, frame: usize) -> f64 {
        frame as f64 * self.fps * 1000.0
    }
    pub fn frame_at_timestamp(&self, ts: f64) -> usize {
        (ts / 1000.0 * self.fps).ceil() as usize
    }

    pub fn get_render_stabilizator(&self) -> StabilizationManager {
        let mut stab = StabilizationManager {
            frame_readout_time: self.frame_readout_time,
            duration_ms: self.duration_ms,
            frame_count: self.frame_count,
            video_size:  self.video_size,
            fps:         self.fps,
            gyro:        self.gyro.clone(),
            fov:         self.fov,
            background:  self.background,
            lens:        self.lens.clone(),
            ..Default::default()
        };
        stab.init_size(self.video_size.0, self.video_size.1);

        stab.recompute_undistortion();

        stab
    }
}

unsafe impl Send for StabilizationManager { }
unsafe impl Sync for StabilizationManager { }
