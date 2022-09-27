// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use nalgebra::*;
use std::iter::zip;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::{ Arc, atomic::AtomicBool };
use std::fs::File;
use telemetry_parser::{ Input, util };
use telemetry_parser::tags_impl::{ GetWithType, GroupId, TagId, TimeQuaternion };

use crate::camera_identifier::CameraIdentifier;
use crate::keyframes::KeyframeManager;

use super::imu_integration::*;
use super::smoothing::SmoothingAlgorithm;
use std::io::Result;
use crate::StabilizationParams;

pub type Quat64 = UnitQuaternion<f64>;
pub type TimeIMU = telemetry_parser::util::IMUData;
pub type TimeQuat = BTreeMap<i64, Quat64>; // key is timestamp_us
pub type TimeVec = BTreeMap<i64, Vector3<f64>>; // key is timestamp_us
pub type TimeFloat = BTreeMap<i64, f64>; // key is timestamp_us

#[derive(Default)]
pub struct FileMetadata {
    pub imu_orientation: Option<String>,
    pub raw_imu:  Option<Vec<TimeIMU>>,
    pub quaternions:  Option<TimeQuat>,
    pub gravity_vectors:  Option<TimeVec>,
    pub image_orientations:  Option<TimeQuat>,
    pub detected_source: Option<String>,
    pub frame_readout_time: Option<f64>,
    pub frame_rate: Option<f64>,
    pub camera_identifier: Option<CameraIdentifier>,
    pub lens_profile: Option<serde_json::Value>,
    pub lens_positions: Option<TimeFloat>
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
    pub acc_rotation_angles: Option<[f64; 3]>,
    pub acc_rotation: Option<Rotation3<f64>>,
    pub imu_lpf: f64,

    pub gyro_bias: Option<[f64; 3]>,

    pub integration_method: usize,

    pub quaternions: TimeQuat,
    pub org_quaternions: TimeQuat,

    pub smoothed_quaternions: TimeQuat,
    pub org_smoothed_quaternions: TimeQuat,

    pub image_orientations: TimeQuat,

    pub gravity_vectors: Option<TimeVec>,
    pub use_gravity_vectors: bool,

    pub lens_positions: Option<TimeFloat>,

    pub max_angles: (f64, f64, f64), // (pitch, yaw, roll) in deg

    pub smoothing_status: serde_json::Value,

    offsets: BTreeMap<i64, f64>, // <microseconds timestamp, offset in milliseconds>
    offsets_adjusted: BTreeMap<i64, f64>, // <timestamp + offset, offset>

    pub file_path: String
}

impl GyroSource {
    pub fn new() -> Self {
        Self {
            integration_method: 1,
            use_gravity_vectors: false,
            ..Default::default()
        }
    }
    pub fn set_use_gravity_vectors(&mut self, v: bool) {
        if self.use_gravity_vectors != v {
            self.use_gravity_vectors = v;
            self.integrate();
        }
        self.use_gravity_vectors = v;
    }
    pub fn init_from_params(&mut self, stabilization_params: &StabilizationParams) {
        self.fps = stabilization_params.get_scaled_fps();
        self.duration_ms = stabilization_params.get_scaled_duration_ms();
    }
    pub fn parse_telemetry_file<F: Fn(f64)>(path: &str, size: (usize, usize), fps: f64, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> Result<FileMetadata> {
        let mut stream = File::open(path)?;
        let filesize = stream.metadata()?.len() as usize;

        let input = Input::from_stream(&mut stream, filesize, &path, progress_cb, cancel_flag)?;

        let camera_identifier = CameraIdentifier::from_telemetry_parser(&input, size.0, size.1, fps).ok();

        let mut detected_source = input.camera_type();
        if let Some(m) = input.camera_model() { detected_source.push(' '); detected_source.push_str(m); }

        let mut imu_orientation = None;
        let mut quaternions = None;
        let mut gravity_vectors: Option<TimeVec> = None;
        let mut image_orientations = None;
        let mut lens_profile = None;
        let mut frame_rate = None;
        let mut lens_positions = None;

        // Get IMU orientation and quaternions
        if let Some(ref samples) = input.samples {
            let mut quats = TimeQuat::new();
            let mut grav = Vec::<Vector3<f64>>::new();
            let mut iori_map = TimeQuat::new();
            let mut iori = Vec::<Quat64>::new();
            let mut crop_score = Vec::<f64>::new();
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
                        if let Some(v) = map.get_t(TagId::LensZoomNative) as Option<&f32> {
                            if lens_positions.is_none() { lens_positions = Some(BTreeMap::new()) };
                            lens_positions.as_mut().unwrap().insert((info.timestamp_ms * 1000.0).round() as i64, *v as f64);
                        }
                    }
                    if let Some(map) = tag_map.get(&GroupId::Default) {
                        if let Some(v) = map.get_t(TagId::FrameRate) as Option<&f64> {
                            frame_rate = Some(*v);
                        }
                    }
                    if let Some(map) = tag_map.get(&GroupId::Custom("FovAdaptationScore".into())) {
                        if let Some(v) = map.get_t(TagId::Data) as Option<&Vec<f32>> {
                            for v in v {
                                crop_score.push(*v as f64);
                            }
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

            for ((ts, _quat), iori) in zip(&quats, &iori) {
                iori_map.insert(*ts, *iori);
            }
            if !iori_map.is_empty() {
                image_orientations = Some(iori_map);
            }

            if !quats.is_empty() {
                if !grav.is_empty() && grav.len() == quats.len() {

                    if grav.len() == iori.len() {
                        for (g, q) in grav.iter_mut().zip(iori.iter()) {
                            *g = (*q) * (*g);
                        }
                    }

                    gravity_vectors = Some(quats.keys().copied().zip(grav.into_iter()).collect());
                }

                if lens_positions.is_none() && !crop_score.is_empty() && crop_score.len() == quats.len() {
                    lens_positions = Some(quats.iter().zip(crop_score.iter()).map(|((ts, _), crop)| (*ts, *crop)).collect());
                }

                quaternions = Some(quats);
            }
        }

        let raw_imu = util::normalized_imu_interpolated(&input, Some("XYZ".into())).ok();

        Ok(FileMetadata {
            imu_orientation,
            detected_source: Some(detected_source),
            quaternions,
            image_orientations,
            gravity_vectors,
            lens_positions,
            raw_imu,
            frame_readout_time: input.frame_readout_time(),
            frame_rate,
            lens_profile,
            camera_identifier
        })
    }

    pub fn load_from_telemetry(&mut self, telemetry: &FileMetadata) {
        if self.duration_ms <= 0.0 {
            ::log::error!("Invalid duration_ms {}", self.duration_ms);
            return;
        }
        if self.fps <= 0.0 {
            ::log::error!("Invalid fps {}", self.fps);
            return;
        }

        self.quaternions.clear();
        self.org_quaternions.clear();
        self.smoothed_quaternions.clear();
        self.org_smoothed_quaternions.clear();
        self.offsets.clear();
        self.offsets_adjusted.clear();
        self.raw_imu.clear();
        self.org_raw_imu.clear();
        self.imu_rotation = None;
        self.acc_rotation = None;
        self.imu_lpf = 0.0;

        self.imu_orientation = telemetry.imu_orientation.clone();
        self.detected_source = telemetry.detected_source.clone();

        if let Some(quats) = &telemetry.quaternions {
            self.quaternions = quats.clone();
            self.org_quaternions = self.quaternions.clone();
        }
        if let Some(ioris) = &telemetry.image_orientations {
            self.image_orientations = ioris.clone();
        }
        if !self.org_quaternions.is_empty() {
            self.integration_method = 0;
        }

        self.gravity_vectors = telemetry.gravity_vectors.clone();
        self.lens_positions = telemetry.lens_positions.clone();

        if let Some(imu) = &telemetry.raw_imu {
            self.org_raw_imu = imu.clone();
            self.apply_transforms();
        } else if self.quaternions.is_empty() {
            self.integrate();
        }
    }
    pub fn integrate(&mut self) {
        match self.integration_method {
            0 => self.quaternions = if self.detected_source.as_ref().unwrap_or(&"".into()).starts_with("GoPro") && !self.org_quaternions.is_empty() && (self.gravity_vectors.is_none() || !self.use_gravity_vectors) {
                    log::info!("No gravity vectors - using accelerometer");
                    QuaternionConverter::convert(&self.org_quaternions, &self.image_orientations, &self.raw_imu, self.duration_ms)
                } else {
                    self.org_quaternions.clone()
                },
            1 => self.quaternions = ComplementaryIntegrator::integrate(&self.raw_imu, self.duration_ms),
            2 => self.quaternions = VQFIntegrator::integrate(&self.raw_imu, self.duration_ms),
            3 => self.quaternions = SimpleGyroIntegrator::integrate(&self.raw_imu, self.duration_ms),
            4 => self.quaternions = SimpleGyroAccelIntegrator::integrate(&self.raw_imu, self.duration_ms),
            5 => self.quaternions = MahonyIntegrator::integrate(&self.raw_imu, self.duration_ms),
            6 => self.quaternions = MadgwickIntegrator::integrate(&self.raw_imu, self.duration_ms),
            _ => log::error!("Unknown integrator")
        }
    }

    pub fn recompute_smoothness(&mut self, alg: &dyn SmoothingAlgorithm, horizon_lock: super::smoothing::horizon::HorizonLock, stabilization_params: &StabilizationParams, keyframes: &KeyframeManager) {
        if true {
            // Lock horizon, then smooth
            self.smoothed_quaternions = horizon_lock.lock(&self.quaternions, &self.quaternions, &self.gravity_vectors, self.use_gravity_vectors, self.integration_method, keyframes);
            self.smoothed_quaternions = alg.smooth(&self.smoothed_quaternions, self.duration_ms, stabilization_params, keyframes);
        } else {
            // Smooth, then lock horizon
            self.smoothed_quaternions = alg.smooth(&self.quaternions, self.duration_ms, stabilization_params, keyframes);
            self.smoothed_quaternions = horizon_lock.lock(&self.smoothed_quaternions, &self.quaternions, &self.gravity_vectors, self.use_gravity_vectors, self.integration_method, keyframes);
        }

        self.max_angles = crate::Smoothing::get_max_angles(&self.quaternions, &self.smoothed_quaternions, stabilization_params);
        self.org_smoothed_quaternions = self.smoothed_quaternions.clone();

        for (sq, q) in self.smoothed_quaternions.iter_mut().zip(self.quaternions.iter()) {
            // rotation quaternion from smooth motion -> raw motion to counteract it
            *sq.1 = sq.1.inverse() * q.1;
        }
    }

    pub fn set_offset(&mut self, timestamp_us: i64, offset_ms: f64) {
        if offset_ms.is_finite() && !offset_ms.is_nan() {
            match self.offsets.entry(timestamp_us) {
                Entry::Occupied(o) => { *o.into_mut() = offset_ms; }
                Entry::Vacant(v) => { v.insert(offset_ms); }
            }
            self.adjust_offsets();
        }
    }
    pub fn remove_offset(&mut self, timestamp_us: i64) {
        self.offsets.remove(&timestamp_us);
        self.adjust_offsets();
    }
    pub fn clear_offsets(&mut self) {
        self.offsets.clear();
        self.offsets_adjusted.clear();
    }
    pub fn get_offsets(&self) -> &BTreeMap<i64, f64> {
        &self.offsets
    }
    pub fn set_offsets(&mut self, offsets: BTreeMap<i64, f64>) {
        self.offsets = offsets;
        self.adjust_offsets();
    }
    pub fn remove_offsets_near(&mut self, ts: i64, range_ms: f64) {
        let range_us = (range_ms * 1000.0).round() as i64;
        self.offsets.retain(|k, _| !(ts-range_us..ts+range_us).contains(k));
        self.adjust_offsets();
    }
    fn adjust_offsets(&mut self) {
        self.offsets_adjusted = self.offsets.iter().map(|(k, v)| (*k + (*v * 1000.0).round() as i64, *v)).collect::<BTreeMap<i64, f64>>();
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
        const DEG2RAD: f64 = std::f64::consts::PI / 180.0;
        if let Some([pitch_deg, roll_deg, yaw_deg]) = self.imu_rotation_angles {
            if pitch_deg.abs() > 0.0 || roll_deg.abs() > 0.0 || yaw_deg.abs() > 0.0 {
                self.imu_rotation = Some(Rotation3::from_euler_angles(
                    yaw_deg * DEG2RAD,
                    pitch_deg * DEG2RAD,
                    roll_deg * DEG2RAD
                ));
            } else {
                self.imu_rotation = None;
            }
        }
        if let Some([pitch_deg, roll_deg, yaw_deg]) = self.acc_rotation_angles {
            if pitch_deg.abs() > 0.0 || roll_deg.abs() > 0.0 || yaw_deg.abs() > 0.0 {
                self.acc_rotation = Some(Rotation3::from_euler_angles(
                    yaw_deg * DEG2RAD,
                    pitch_deg * DEG2RAD,
                    roll_deg * DEG2RAD
                ));
            } else {
                self.acc_rotation = None;
            }
        }
        if self.imu_rotation.is_some() || self.acc_rotation.is_some() {
            let rotate = |inp: &[f64; 3], rot: Rotation3<f64>| -> [f64; 3] {
                let rotated = rot.transform_vector(&Vector3::new(inp[0], inp[1], inp[2]));
                [rotated[0], rotated[1], rotated[2]]
            };
            let grot = self.imu_rotation;
            let arot = if self.acc_rotation.is_some() { self.acc_rotation } else { self.imu_rotation };
            for x in &mut self.raw_imu {
                if let Some(g) = x.gyro.as_mut() { if let Some(grot) = grot { *g = rotate(g, grot); } }
                if let Some(a) = x.accl.as_mut() { if let Some(arot) = arot { *a = rotate(a, arot); } }
                if let Some(m) = x.magn.as_mut() { if let Some(grot) = grot { *m = rotate(m, grot); } }
            }
        }

        self.integrate();
    }

    fn quat_at_timestamp(&self, quats: &TimeQuat, mut timestamp_ms: f64) -> Quat64 {
        if quats.len() < 2 || self.duration_ms <= 0.0 { return Quat64::identity(); }

        timestamp_ms -= self.offset_at_video_timestamp(timestamp_ms);

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

    pub fn offset_at_timestamp(offsets: &BTreeMap<i64, f64>, timestamp_ms: f64) -> f64 {
        match offsets.len() {
            0 => 0.0,
            1 => *offsets.values().next().unwrap(),
            _ => {
                if let Some(&first_ts) = offsets.keys().next() {
                    if let Some(&last_ts) = offsets.keys().next_back() {
                        let timestamp_us = (timestamp_ms * 1000.0) as i64;
                        let lookup_ts = (timestamp_us).min(last_ts-1).max(first_ts+1);
                        if let Some(offs1) = offsets.range(..=lookup_ts).next_back() {
                            if *offs1.0 == lookup_ts {
                                return *offs1.1;
                            }
                            if let Some(offs2) = offsets.range(lookup_ts..).next() {
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
    pub fn offset_at_video_timestamp(&self, timestamp_ms: f64) -> f64 { Self::offset_at_timestamp(&self.offsets_adjusted, timestamp_ms) }
    pub fn offset_at_gyro_timestamp (&self, timestamp_ms: f64) -> f64 { Self::offset_at_timestamp(&self.offsets, timestamp_ms) }

    pub fn clone_quaternions(&self) -> Self {
        Self {
            duration_ms:          self.duration_ms,
            fps:                  self.fps,
            quaternions:          self.quaternions.clone(),
            smoothed_quaternions: self.smoothed_quaternions.clone(),
            offsets:              self.offsets.clone(),
            offsets_adjusted:     self.offsets_adjusted.clone(),
            gravity_vectors:      self.gravity_vectors.clone(),
            lens_positions:       self.lens_positions.clone(),
            use_gravity_vectors:  self.use_gravity_vectors,
            integration_method:   self.integration_method,
            ..Default::default()
        }
    }

    pub fn get_sample_rate(&self) -> f64 {
        if self.org_raw_imu.len() > 2 {
            let duration_ms = self.org_raw_imu.last().unwrap().timestamp_ms - self.org_raw_imu.first().unwrap().timestamp_ms;
            self.org_raw_imu.len() as f64 / (duration_ms / 1000.0)
        } else if self.org_quaternions.len() > 2 {
            let first = *self.org_quaternions.iter().next().unwrap().0 as f64 / 1000.0;
            let last = *self.org_quaternions.iter().next_back().unwrap().0 as f64 / 1000.0;
            let duration_ms = last - first;
            self.org_quaternions.len() as f64 / (duration_ms / 1000.0)
        } else {
            0.0
        }
    }

    pub fn find_bias(&self, timestamp_start: f64, timestamp_stop: f64) -> (f64, f64, f64) {
        let ts_start = timestamp_start - self.offset_at_video_timestamp(timestamp_start);
        let ts_stop = timestamp_stop - self.offset_at_video_timestamp(timestamp_stop);
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
