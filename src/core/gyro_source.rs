// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use nalgebra::*;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fs::File;
use telemetry_parser::{Input, util};
use telemetry_parser::tags_impl::{GetWithType, GroupId, TagId, TimeQuaternion};

use crate::camera_identifier::CameraIdentifier;

use super::integration::*;
use super::smoothing::SmoothingAlgorithm;
use std::io::Result;
use crate::StabilizationParams;

pub type Quat64 = UnitQuaternion<f64>;
pub type TimeIMU = telemetry_parser::util::IMUData;
pub type TimeQuat = BTreeMap<i64, Quat64>; // key is timestamp_us
pub type TimeVec = BTreeMap<i64, Vector3<f64>>; // key is timestamp_us

#[derive(Default)]
pub struct FileMetadata {
    pub imu_orientation: Option<String>,
    pub raw_imu:  Option<Vec<TimeIMU>>,
    pub quaternions:  Option<TimeQuat>,
    pub gravity_vectors:  Option<TimeVec>,
    pub detected_source: Option<String>,
    pub frame_readout_time: Option<f64>,
    pub camera_identifier: Option<CameraIdentifier>,
    pub lens_profile: Option<serde_json::Value>
}

#[derive(Default, Clone)]
pub struct GyroSource {
    pub detected_source: Option<String>,

    pub duration_ms: f64,
    pub fps: f64,

    pub raw_imu: Vec<TimeIMU>,
    pub org_raw_imu: Vec<TimeIMU>,

    pub imu_orientation: Option<String>,

    pub imu_rotation_angles: Option<[f64; 3]>,
    pub imu_rotation: Option<Rotation3<f64>>,
    pub imu_lpf: f64,

    pub gyro_bias: Option<[f64; 3]>,

    pub integration_method: usize,

    pub quaternions: TimeQuat,
    pub org_quaternions: TimeQuat,

    pub smoothed_quaternions: TimeQuat,
    pub org_smoothed_quaternions: TimeQuat,

    pub gravity_vectors: Option<TimeVec>,

    pub max_angles: (f64, f64, f64), // (pitch, yaw, roll) in deg

    pub smoothing_status: serde_json::Value,
    
    pub offsets: BTreeMap<i64, f64>, // microseconds timestamp, offset in milliseconds

    pub file_path: String,

    pub prevent_next_load: bool
}

impl GyroSource {
    pub fn new() -> Self {
        Self {
            integration_method: 1,
            ..Default::default()
        }
    }
    pub fn init_from_params(&mut self, stabilization_params: &StabilizationParams) {
        self.fps = stabilization_params.get_scaled_fps();
        self.duration_ms = stabilization_params.get_scaled_duration_ms();
        self.offsets.clear();
    }
    pub fn parse_telemetry_file(path: &str, size: (usize, usize), fps: f64) -> Result<FileMetadata> {
        let mut stream = File::open(path)?;
        let filesize = stream.metadata()?.len() as usize;
    
        let input = Input::from_stream(&mut stream, filesize, &path)?;

        let camera_identifier = CameraIdentifier::from_telemetry_parser(&input, size.0, size.1, fps).ok();
    
        let mut detected_source = input.camera_type();
        if let Some(m) = input.camera_model() { detected_source.push(' '); detected_source.push_str(m); }

        let mut imu_orientation = None;
        let mut quaternions = None;
        let mut gravity_vectors: Option<TimeVec> = None;
        let mut lens_profile = None;

        // Get IMU orientation and quaternions
        if let Some(ref samples) = input.samples {
            let mut quats = TimeQuat::new();
            let mut grav = Vec::<Vector3<f64>>::new();
            let mut iori = Vec::<Quat64>::new();
            let mut grav_is_usable = false;
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
                    if let Some(map) = tag_map.get(&GroupId::Lens) {
                        if let Some(v) = map.get_t(TagId::Data) as Option<&serde_json::Value> {
                            lens_profile = Some(v.clone());
                        }
                        if let Some(v) = map.get_t(TagId::Name) as Option<&String> {
                            lens_profile = Some(serde_json::Value::String(v.clone()));
                        }
                    }
                    if let Some(map) = tag_map.get(&GroupId::GravityVector) {
                        let scale = *(map.get_t(TagId::Scale) as Option<&i16>).unwrap_or(&32767) as f64;
                        if scale > 0.0 {
                            if let Some(arr) = map.get_t(TagId::Data) as Option<&Vec<telemetry_parser::tags_impl::Vector3<i16>>> {
                                for v in arr {
                                    if v.x != 0 || v.y != 0 || v.z != 0 {
                                        grav_is_usable = true;
                                    }
                                    grav.push(Vector3::new(v.x as f64 / scale, v.y as f64 / scale, v.z as f64 / scale));
                                }
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
                    if let Some(map) = tag_map.get(&GroupId::ImageOrientation) {
                        let scale = *(map.get_t(TagId::Scale) as Option<&i16>).unwrap_or(&32767) as f64;
                        if let Some(arr) = map.get_t(TagId::Data) as Option<&Vec<telemetry_parser::tags_impl::Quaternion<i16>>> {
                            for v in arr.iter() {
                                iori.push(Quat64::from_quaternion(nalgebra::Quaternion::<f64>::from_vector(Vector4::new(
                                    v.x as f64 / scale,
                                    v.y as f64 / scale,
                                    v.z as f64 / scale,
                                    v.w as f64 / scale)
                                )));
                            }
                        }
                    }
                }
            }

            if !grav_is_usable { grav.clear(); }

            if !quats.is_empty() {
                if !grav.is_empty() && grav.len() == quats.len() {

                    if grav.len() == iori.len() {
                        for (g, q) in grav.iter_mut().zip(iori.iter()) {
                            *g = (*q) * (*g);
                        }
                    }

                    gravity_vectors = Some(quats.keys().copied().zip(grav.into_iter()).collect());
                }
                quaternions = Some(quats);
            }
        }

        let raw_imu = util::normalized_imu_interpolated(&input, Some("XYZ".into())).ok();

        Ok(FileMetadata {
            imu_orientation,
            detected_source: Some(detected_source),
            quaternions,
            gravity_vectors,
            raw_imu,
            frame_readout_time: input.frame_readout_time(),
            lens_profile,
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

        if let Some(quats) = &telemetry.quaternions {
            self.quaternions = quats.clone();
            self.org_quaternions = self.quaternions.clone();
        }
        if !self.quaternions.is_empty() {
            self.integration_method = 0;
        }

        self.gravity_vectors = telemetry.gravity_vectors.clone();
        
        if let Some(imu) = &telemetry.raw_imu {
            self.org_raw_imu = imu.clone();
            self.apply_transforms();
        } else if self.quaternions.is_empty() {
            self.integrate();
        }
    }
    pub fn integrate(&mut self) {
        match self.integration_method {
            0 => self.quaternions = if self.detected_source.as_ref().unwrap_or(&"".into()).starts_with("GoPro") { 
                    QuaternionConverter::convert(&self.org_quaternions, &self.raw_imu, self.duration_ms) 
                } else {
                    self.org_quaternions.clone()
                },
            1 => self.quaternions = ComplementaryIntegrator::integrate(&self.raw_imu, self.duration_ms),
            2 => self.quaternions = MadgwickIntegrator::integrate(&self.raw_imu, self.duration_ms),
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

    pub fn recompute_smoothness(&mut self, alg: &mut dyn SmoothingAlgorithm, horizon_lock: super::smoothing::horizon::HorizonLock, stabilization_params: &StabilizationParams) {
        self.smoothed_quaternions = alg.smooth(&self.quaternions, self.duration_ms, stabilization_params);
        horizon_lock.lock(&mut self.smoothed_quaternions, &mut self.quaternions, &self.gravity_vectors, self.integration_method);

        self.max_angles = crate::Smoothing::get_max_angles(&self.quaternions, &self.smoothed_quaternions, stabilization_params);
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
        self.imu_rotation_angles = Some([pitch_deg, roll_deg, yaw_deg]);
        const DEG2RAD: f64 = std::f64::consts::PI / 180.0;
        if pitch_deg.abs() > 0.0 || roll_deg.abs() > 0.0 || yaw_deg.abs() > 0.0 {
            self.imu_rotation = Some(Rotation3::from_euler_angles(
                yaw_deg * DEG2RAD, 
                pitch_deg * DEG2RAD, 
                roll_deg * DEG2RAD
            ))
        } else {
            self.imu_rotation = None;
        }
        self.apply_transforms();
    }
    pub fn set_bias(&mut self, bx: f64, by: f64, bz: f64) {
        self.gyro_bias = Some([bx, by, bz]);
        self.apply_transforms();
    }

    pub fn apply_transforms(&mut self) {
        self.raw_imu = self.org_raw_imu.clone();
        if self.imu_lpf > 0.0 && !self.org_raw_imu.is_empty() && self.duration_ms > 0.0 {
            let sample_rate = self.org_raw_imu.len() as f64 / (self.duration_ms / 1000.0);
            if let Err(e) = super::filtering::Lowpass::filter_gyro_forward_backward(self.imu_lpf, sample_rate, &mut self.raw_imu) {
                log::error!("Filter error {:?}", e);
            }
        }
        if let Some(bias) = self.gyro_bias {
            for x in &mut self.raw_imu {
                if let Some(g) = x.gyro.as_mut() {
                    *g = [
                        g[0] + bias[0], 
                        g[1] + bias[1], 
                        g[2] + bias[2]
                    ];
                }
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
        if quats.len() < 2 || self.duration_ms <= 0.0 { return Quat64::identity(); }

        timestamp_ms -= self.offset_at_timestamp(timestamp_ms);
    
        if let Some(&first_ts) = quats.keys().next() {
            if let Some(&last_ts) = quats.keys().next_back() {
                let lookup_ts = ((timestamp_ms * 1000.0) as i64).min(last_ts).max(first_ts);

                if let Some(quat1) = quats.range(..=lookup_ts).next_back() {
                    if *quat1.0 == lookup_ts {
                        return *quat1.1;
                    }
                    if let Some(quat2) = quats.range(lookup_ts..).next() {
                        let time_delta = (quat2.0 - quat1.0) as f64;
                        let fract = (lookup_ts - quat1.0) as f64 / time_delta;
                        return quat1.1.slerp(quat2.1, fract);
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
                if let Some(&first_ts) = self.offsets.keys().next() {
                    if let Some(&last_ts) = self.offsets.keys().next_back() {
                        let timestamp_us = (timestamp_ms * 1000.0) as i64; 
                        let lookup_ts = (timestamp_us).min(last_ts-1).max(first_ts+1);
                        if let Some(offs1) = self.offsets.range(..=lookup_ts).next_back() {
                            if *offs1.0 == lookup_ts {
                                return *offs1.1;
                            }
                            if let Some(offs2) = self.offsets.range(lookup_ts..).next() {
                                let time_delta = (offs2.0 - offs1.0) as f64;
                                let fract = (timestamp_us - offs1.0) as f64 / time_delta;
                                return offs1.1 + (offs2.1 - offs1.1) * fract;
                            }
                        }
                    }
                }

                0.0
            }
        }
    }

    pub fn clone_quaternions(&self) -> Self {
        Self {
            duration_ms:          self.duration_ms,
            fps:                  self.fps,
            quaternions:          self.quaternions.clone(),
            smoothed_quaternions: self.smoothed_quaternions.clone(),            
            offsets:              self.offsets.clone(),
            gravity_vectors:      self.gravity_vectors.clone(),
            integration_method:   self.integration_method,
            ..Default::default()
        }
    }

    pub fn find_bias(&self, timestamp_start: f64, timestamp_stop: f64) -> (f64, f64, f64) {
        let ts_start = timestamp_start - self.offset_at_timestamp(timestamp_start);
        let ts_stop = timestamp_stop - self.offset_at_timestamp(timestamp_stop);
        let mut bias_vals = [0.0, 0.0, 0.0];
        let mut n = 0;

        for x in &self.org_raw_imu {
            if let Some(g) = x.gyro {
                if x.timestamp_ms > ts_start && x.timestamp_ms < ts_stop {
                    bias_vals[0] -= g[0];
                    bias_vals[1] -= g[1];
                    bias_vals[2] -= g[2];
                    n += 1;
                }
            }
        }
        for b in bias_vals.iter_mut() {
            *b /= n.max(1) as f64;
        }
        
        (bias_vals[0], bias_vals[1], bias_vals[2])
    }
}
