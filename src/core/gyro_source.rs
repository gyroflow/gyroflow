use nalgebra::*;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fs::File;
use std::path::Path;
use telemetry_parser::{Input, util};
use telemetry_parser::tags_impl::{GetWithType, GroupId, TagId, TimeQuaternion};

use crate::BasicParams;
use crate::camera_identifier::CameraIdentifier;

use super::integration::*;
use super::smoothing::SmoothingAlgorithm;
use std::io::Result;

pub type Quat64 = UnitQuaternion<f64>;
pub type TimeIMU = telemetry_parser::util::IMUData;
pub type TimeQuat = BTreeMap<i64, Quat64>; // key is timestamp_us

#[derive(Default)]
pub struct FileMetadata {
    pub imu_orientation: Option<String>,
    pub raw_imu:  Option<Vec<TimeIMU>>,
    pub quaternions:  Option<TimeQuat>,
    pub detected_source: Option<String>,
    pub frame_readout_time: Option<f64>,
    pub camera_identifier: Option<CameraIdentifier>
}

#[derive(Default, Clone)]
pub struct GyroSource {
    pub detected_source: Option<String>,

    pub duration_ms: f64,
    pub fps: f64,

    pub raw_imu: Vec<TimeIMU>,
    pub org_raw_imu: Vec<TimeIMU>,

    pub imu_orientation: Option<String>,

    imu_rotation: Option<Rotation3<f64>>,
    imu_lpf: f64,

    pub integration_method: usize,

    pub quaternions: TimeQuat,
    pub org_quaternions: TimeQuat,

    pub smoothed_quaternions: TimeQuat,
    pub org_smoothed_quaternions: TimeQuat,

    pub smoothing_status: serde_json::Value,
    
    pub offsets: BTreeMap<i64, f64>, // microseconds timestamp, offset in milliseconds
}

impl GyroSource {
    pub fn new() -> Self {
        Self {
            integration_method: 1,
            ..Default::default()
        }
    }
    pub fn init_from_params(&mut self, params: &BasicParams) {
        self.fps = params.get_scaled_fps();
        self.duration_ms = params.get_scaled_duration_ms();
        self.offsets.clear();
    }
    pub fn parse_telemetry_file(path: &str, size: (usize, usize), fps: f64) -> Result<FileMetadata> {
        let mut stream = File::open(path)?;
        let filesize = stream.metadata()?.len() as usize;
    
        let filename = Path::new(&path).file_name().unwrap().to_str().unwrap();
    
        let input = Input::from_stream(&mut stream, filesize, filename)?;

        let camera_identifier = CameraIdentifier::from_telemetry_parser(&input, size.0, size.1, fps).ok();
    
        let mut detected_source = input.camera_type();
        if let Some(m) = input.camera_model() { detected_source.push(' '); detected_source.push_str(m); }

        let mut imu_orientation = None;
        let mut quaternions = None;

        // Get IMU orientation and quaternions
        if let Some(ref samples) = input.samples {
            let mut quats = TimeQuat::new();
            for info in samples {
                if let Some(ref tag_map) = info.tag_map {
                    if let Some(map) = tag_map.get(&GroupId::Quaternion) {
                        if let Some(arr) = map.get_t(TagId::Data) as Option<&Vec<TimeQuaternion<f64>>> {
                            for v in arr {
                                quats.insert((v.t * 1000.0) as i64, Quat64::from_quaternion(Quaternion::from_parts(
                                    v.v.w, 
                                    Vector3::new(v.v.x, v.v.y, v.v.z)
                                )));
                            }
                        }
                    }
                    if let Some(map) = tag_map.get(&GroupId::Gyroscope) {
                        let mut io = match map.get_t(TagId::Orientation) as Option<&String> {
                            Some(v) => v.clone(),
                            None => "XYZ".into()
                        };
                        io = input.normalize_imu_orientation(io);
                        imu_orientation = Some(io);
                    }
                }
            }
            if !quats.is_empty() {
                quaternions = Some(quats);
            }
        }

        let raw_imu = util::normalized_imu(&input, Some("XYZ".into())).ok();

        Ok(FileMetadata {
            imu_orientation,
            detected_source: Some(detected_source),
            quaternions,
            raw_imu,
            frame_readout_time: telemetry_parser::util::frame_readout_time(&input),
            camera_identifier
        })
    }

    pub fn load_from_telemetry(&mut self, telemetry: &FileMetadata) {
        assert!(self.duration_ms > 0.0);
        assert!(self.fps > 0.0);

        self.quaternions.clear();
        self.org_quaternions.clear();
        self.smoothed_quaternions.clear();
        self.org_smoothed_quaternions.clear();
        self.offsets.clear();
        self.raw_imu.clear();
        self.org_raw_imu.clear();
        self.imu_rotation = None;
        self.imu_lpf = 0.0;

        self.imu_orientation = telemetry.imu_orientation.clone();
        self.detected_source = telemetry.detected_source.clone();

        if let Some(imu) = &telemetry.raw_imu {
            self.org_raw_imu = imu.clone();
            self.apply_transforms();
        }
        if let Some(quats) = &telemetry.quaternions {
            self.quaternions = quats.clone();
            self.org_quaternions = self.quaternions.clone();
        }

        if self.quaternions.is_empty() {
            self.integrate();
        }
    }
    pub fn integrate(&mut self) {
        match self.integration_method {
            0 => self.quaternions = self.org_quaternions.clone(),
            1 => self.quaternions = MadgwickIntegrator::integrate(&self.raw_imu, self.duration_ms),
            2 => self.quaternions = ComplementaryIntegrator::integrate(&self.raw_imu, self.duration_ms),
            3 => self.quaternions = MahonyIntegrator::integrate(&self.raw_imu, self.duration_ms),
            4 => self.quaternions = GyroOnlyIntegrator::integrate(&self.raw_imu, self.duration_ms),
            _ => log::error!("Unknown integrator")
        }
    }

    pub fn set_offset(&mut self, timestamp_us: i64, offset_ms: f64) {
        if offset_ms.is_finite() && !offset_ms.is_nan() {
            match self.offsets.entry(timestamp_us) {
                Entry::Occupied(o) => { *o.into_mut() = offset_ms; }
                Entry::Vacant(v) => { v.insert(offset_ms); }
            }
        }
    }

    pub fn recompute_smoothness(&mut self, alg: &mut dyn SmoothingAlgorithm, params: &BasicParams) {
        self.smoothed_quaternions = alg.smooth(&self.quaternions, self.duration_ms, params);
        self.org_smoothed_quaternions = self.smoothed_quaternions.clone();

        for (sq, q) in self.smoothed_quaternions.iter_mut().zip(self.quaternions.iter()) {
            // rotation quaternion from smooth motion -> raw motion to counteract it
            *sq.1 = sq.1.inverse() * q.1;
        }
    }

    pub fn remove_offset(&mut self, timestamp_us: i64) {
        self.offsets.remove(&timestamp_us);
    }

    pub fn set_lowpass_filter(&mut self, freq: f64) {
        self.imu_lpf = freq;
        self.apply_transforms();
    }
    pub fn set_imu_orientation(&mut self, orientation: String) {
        self.imu_orientation = Some(orientation);
        self.apply_transforms();
    }
    pub fn set_imu_rotation(&mut self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64) {
        const DEG2RAD: f64 = std::f64::consts::PI / 180.0;
        if pitch_deg.abs() > 0.0 || roll_deg.abs() > 0.0 || yaw_deg.abs() > 0.0 {
            self.imu_rotation = Some(Rotation3::from_euler_angles(
                roll_deg * DEG2RAD, 
                pitch_deg * DEG2RAD, 
                yaw_deg * DEG2RAD
            ))
        } else {
            self.imu_rotation = None;
        }
        self.apply_transforms();
    }

    pub fn apply_transforms(&mut self) {
        self.raw_imu = self.org_raw_imu.clone();
        if self.imu_lpf > 0.0 {
            let sample_rate = self.org_raw_imu.len() as f64 / (self.duration_ms / 1000.0);
            if let Err(e) = super::filtering::Lowpass::filter_gyro_forward_backward(self.imu_lpf, sample_rate, &mut self.raw_imu) {
                log::error!("Filter error {:?}", e);
            }
        }
        if let Some(ref orientation) = self.imu_orientation {
            pub fn orient(inp: &[f64; 3], io: &[u8]) -> [f64; 3] {
                let map = |o: u8| -> f64 {
                    match o as char {
                        'X' => inp[0], 'x' => -inp[0],
                        'Y' => inp[1], 'y' => -inp[1],
                        'Z' => inp[2], 'z' => -inp[2], 
                        err => { panic!("Invalid orientation {}", err); }
                    }
                };
                [map(io[0]), map(io[1]), map(io[2]) ]
            }
            for x in &mut self.raw_imu {
                // Change orientation
                if let Some(g) = x.gyro.as_mut() { *g = orient(g, orientation.as_bytes()); }
                if let Some(a) = x.accl.as_mut() { *a = orient(a, orientation.as_bytes()); }
                if let Some(m) = x.magn.as_mut() { *m = orient(m, orientation.as_bytes()); }
            }
        }
        // Rotate
        if let Some(rotation) = self.imu_rotation {
            let rotate = |inp: &[f64; 3]| -> [f64; 3] {
                let rotated = rotation.transform_vector(&Vector3::new(inp[0], inp[1], inp[2]));
                [rotated[0], rotated[1], rotated[2]]
            };
            for x in &mut self.raw_imu {
                if let Some(g) = x.gyro.as_mut() { *g = rotate(g); } 
                if let Some(a) = x.accl.as_mut() { *a = rotate(a); } 
                if let Some(m) = x.magn.as_mut() { *m = rotate(m); } 
            }
        }

        self.integrate();
    }

    fn quat_at_timestamp(&self, quats: &TimeQuat, mut timestamp_ms: f64) -> Quat64 {
        if quats.len() < 2 { return Quat64::identity(); }
        assert!(self.duration_ms > 0.0);

        timestamp_ms -= self.offset_at_timestamp(timestamp_ms);
    
        if let Some(&first_ts) = quats.keys().next() {
            if let Some(&last_ts) = quats.keys().next_back() {
                let lookup_ts = ((timestamp_ms * 1000.0) as i64).min(last_ts).max(first_ts);

                if let Some(quat1) = quats.range(..=lookup_ts).next_back() {
                    if let Some(quat2) = quats.range(lookup_ts..).next() {

                        let time_delta = (quat2.0 - quat1.0) as f64;
                        if time_delta != 0.0 {
                            let fract = (lookup_ts - quat1.0) as f64 / time_delta;
                            return quat1.1.slerp(quat2.1, fract);
                        } else {
                            return *quat1.1;
                        }
                    }
                }
            }
        }
        Quat64::identity()
    }
    
    pub fn      org_quat_at_timestamp(&self, timestamp_ms: f64) -> Quat64 { self.quat_at_timestamp(&self.quaternions,          timestamp_ms) }
    pub fn smoothed_quat_at_timestamp(&self, timestamp_ms: f64) -> Quat64 { self.quat_at_timestamp(&self.smoothed_quaternions, timestamp_ms) }
    
    pub fn offset_at_timestamp(&self, timestamp_ms: f64) -> f64 {
        match self.offsets.len() {
            0 => 0.0,
            1 => *self.offsets.values().next().unwrap(),
            _ => {
                let first_ts = *self.offsets.keys().next().unwrap();
                let last_ts = *self.offsets.keys().next_back().unwrap();
        
                let lookup_ts = ((timestamp_ms * 1000.0) as i64).min(last_ts).max(first_ts);
        
                if let Some(offs1) = self.offsets.range(..=lookup_ts).next_back() {
                    if let Some(offs2) = self.offsets.range(lookup_ts..).next() {
                        let time_delta = (offs2.0 - offs1.0) as f64 / 1000.0;
                        if time_delta != 0.0 {
                            offs1.1 + ((offs2.1 - offs1.1) / time_delta) * ((lookup_ts - offs1.0) as f64 / 1000.0)
                        } else {
                            *offs1.1
                        }
                    } else {
                        0.0
                    }
                } else {
                    0.0
                }
            }
        }
    }
}
