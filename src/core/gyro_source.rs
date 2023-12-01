// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use nalgebra::*;
use std::iter::zip;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::{ Arc, atomic::AtomicBool };
use telemetry_parser::{ Input, util };
use telemetry_parser::tags_impl::{ GetWithType, GroupId, TagId, TimeQuaternion };

use crate::camera_identifier::CameraIdentifier;
use crate::keyframes::KeyframeManager;
use crate::filesystem;

use super::imu_integration::*;
use super::smoothing::SmoothingAlgorithm;
use crate::StabilizationParams;

pub type Quat64 = UnitQuaternion<f64>;
pub type TimeIMU = telemetry_parser::util::IMUData;
pub type TimeQuat = BTreeMap<i64, Quat64>; // key is timestamp_us
pub type TimeVec = BTreeMap<i64, Vector3<f64>>; // key is timestamp_us

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LensParams {
    pub focal_length: Option<f32>, // millimeters
    pub pixel_pitch: Option<(u32, u32)>, // nanometers
    pub capture_area_origin: Option<(u32, u32)>, // pixels
    pub capture_area_size: Option<(u32, u32)>, // pixels
    pub pixel_focal_length: Option<f32>, // pixels
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FileMetadata {
    pub imu_orientation:     Option<String>,
    pub raw_imu:             Vec<TimeIMU>,
    pub quaternions:         TimeQuat,
    pub gravity_vectors:     Option<TimeVec>,
    pub image_orientations:  Option<TimeQuat>,
    pub detected_source:     Option<String>,
    pub frame_readout_time:  Option<f64>,
    pub frame_rate:          Option<f64>,
    pub camera_identifier:   Option<CameraIdentifier>,
    pub lens_profile:        Option<serde_json::Value>,
    pub lens_positions:      BTreeMap<i64, f64>,
    pub lens_params:         BTreeMap<i64, LensParams>,
    pub has_accurate_timestamps: bool,
    pub additional_data:     serde_json::Value,
    pub per_frame_time_offsets: Vec<f64>,
    pub per_frame_data:      Vec<serde_json::Value>,
}
impl FileMetadata {
    pub fn thin(&self) -> Self {
        Self {
            imu_orientation:         self.imu_orientation.clone(),
            raw_imu:                 Default::default(),
            quaternions:             Default::default(),
            gravity_vectors:         Default::default(),
            image_orientations:      Default::default(),
            detected_source:         self.detected_source.clone(),
            frame_readout_time:      self.frame_readout_time.clone(),
            frame_rate:              self.frame_rate.clone(),
            camera_identifier:       self.camera_identifier.clone(),
            lens_profile:            self.lens_profile.clone(),
            lens_positions:          Default::default(),
            lens_params:             Default::default(),
            has_accurate_timestamps: self.has_accurate_timestamps.clone(),
            additional_data:         self.additional_data.clone(),
            per_frame_time_offsets:  Default::default(),
            per_frame_data:          Default::default(),
        }
    }
}

#[derive(Default, Clone)]
pub struct FileLoadOptions {
    pub sample_index: Option<usize>
}

#[derive(Default, Clone)]
pub struct GyroSource {
    pub file_load_options: FileLoadOptions,

    pub duration_ms: f64,

    pub raw_imu: Vec<TimeIMU>,

    pub imu_orientation: Option<String>,

    pub imu_rotation_angles: Option<[f64; 3]>,
    pub imu_rotation: Option<Rotation3<f64>>,
    pub acc_rotation_angles: Option<[f64; 3]>,
    pub acc_rotation: Option<Rotation3<f64>>,
    pub imu_lpf: f64,

    pub gyro_bias: Option<[f64; 3]>,

    pub integration_method: usize,

    pub quaternions: TimeQuat,

    pub smoothed_quaternions: TimeQuat,
    pub org_smoothed_quaternions: TimeQuat,

    pub use_gravity_vectors: bool,
    pub horizon_lock_integration_method: i32,

    pub max_angles: (f64, f64, f64), // (pitch, yaw, roll) in deg

    pub smoothing_status: serde_json::Value,

    pub prevent_recompute: bool,

    pub file_metadata: FileMetadata,

    offsets: BTreeMap<i64, f64>, // <microseconds timestamp, offset in milliseconds>
    offsets_linear: BTreeMap<i64, f64>, // <microseconds timestamp, offset in milliseconds> - linear fit
    offsets_adjusted: BTreeMap<i64, f64>, // <timestamp + offset, offset>

    pub file_url: String
}

impl GyroSource {
    pub fn new() -> Self {
        Self {
            integration_method: 1,
            use_gravity_vectors: false,
            horizon_lock_integration_method: 1, // VQF
            ..Default::default()
        }
    }
    pub fn has_motion(&self) -> bool {
        !self.file_metadata.raw_imu.is_empty() || !self.file_metadata.quaternions.is_empty()
    }
    pub fn set_use_gravity_vectors(&mut self, v: bool) {
        if self.use_gravity_vectors != v {
            self.use_gravity_vectors = v;
            self.integrate();
        }
        self.use_gravity_vectors = v;
    }
    pub fn set_horizon_lock_integration_method(&mut self, v: i32) {
        if self.horizon_lock_integration_method != v {
            self.horizon_lock_integration_method = v;
            self.integrate();
        }
        self.horizon_lock_integration_method = v;
    }
    pub fn init_from_params(&mut self, stabilization_params: &StabilizationParams) {
        self.duration_ms = stabilization_params.get_scaled_duration_ms();
    }
    pub fn parse_telemetry_file<F: Fn(f64)>(url: &str, options: &FileLoadOptions, size: (usize, usize), fps: f64, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> Result<FileMetadata, crate::GyroflowCoreError> {
        let base = filesystem::get_engine_base();
        let mut file = filesystem::open_file(&base, url, false)?;
        let filesize = file.size;
        let mut input = Input::from_stream(file.get_file(), filesize, &filesystem::url_to_path(url), progress_cb, cancel_flag)?;

        let camera_identifier = CameraIdentifier::from_telemetry_parser(&input, size.0, size.1, fps).ok();

        let mut detected_source = input.camera_type();
        if let Some(m) = input.camera_model() { detected_source.push(' '); detected_source.push_str(m); }

        let mut imu_orientation = None;
        let mut quaternions = TimeQuat::default();
        let mut gravity_vectors: Option<TimeVec> = None;
        let mut image_orientations = None;
        let mut lens_profile = None;
        let mut frame_rate = None;
        let mut lens_positions = BTreeMap::new();
        let mut lens_params = BTreeMap::new();
        let mut additional_data = serde_json::Value::Object(serde_json::Map::new());

        if input.camera_type() == "BlackBox" {
            if let Some(ref mut samples) = input.samples {
                let mut usable_logs = Vec::new();
                for info in samples.iter() {
                    log::info!("Blackbox log #{}: Timestamp {:.3} | Duration {:.3} | Data: {}", info.sample_index + 1, info.timestamp_ms / 1000.0, info.duration_ms / 1000.0, info.tag_map.is_some());
                    if info.tag_map.is_some() && info.duration_ms > 0.0 {
                        usable_logs.push(serde_json::Value::String(format!("{};{};{}", info.sample_index, info.timestamp_ms, info.duration_ms)));
                    }
                }
                if let Some(requested_index) = options.sample_index {
                    samples.retain(|x| x.sample_index as usize == requested_index);
                }
                additional_data.as_object_mut().unwrap().insert("usable_logs".to_owned(), serde_json::Value::Array(usable_logs));
            }
        }

        // Get IMU orientation and quaternions
        if let Some(ref samples) = input.samples {
            let mut quats = TimeQuat::new();
            let mut grav = Vec::<Vector3<f64>>::new();
            let mut iori_map = TimeQuat::new();
            let mut iori = Vec::<Quat64>::new();
            let mut crop_score = Vec::<f64>::new();
            let mut grav_is_usable = false;
            let mut lens_info = LensParams::default();
            for info in samples {
                let timestamp_us = (info.timestamp_ms * 1000.0).round() as i64;
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
                    if let Some(im) = tag_map.get(&GroupId::Imager) {
                        if input.camera_type() == "RED" {
                            lens_info.capture_area_size = Some((size.0 as u32, size.1 as u32));
                        }
                        if let Some(v) = im.get_t(TagId::PixelPitch) as Option<&(u32, u32)> { lens_info.pixel_pitch = Some(*v); }
                        if let Some(v) = im.get_t(TagId::CaptureAreaSize) as Option<&(u32, u32)> { lens_info.capture_area_size = Some(*v); }
                        if let Some(v) = im.get_t(TagId::CaptureAreaOrigin) as Option<&(u32, u32)> { lens_info.capture_area_origin = Some(*v); }
                    }
                    if let Some(map) = tag_map.get(&GroupId::Lens) {
                        if let Some(v) = map.get_t(TagId::Data) as Option<&serde_json::Value> {
                            lens_profile = Some(v.clone());
                        }
                        if let Some(v) = map.get_t(TagId::Name) as Option<&String> {
                            lens_profile = Some(serde_json::Value::String(v.clone()));
                        }
                        if let Some(v) = map.get_t(TagId::FocalLength) as Option<&f32> {
                            lens_positions.insert(timestamp_us, *v as f64);
                            lens_info.focal_length = Some(*v);
                        }
                    }
                    if lens_info.focal_length.is_none() {
                        if let Some(md) = tag_map.get(&GroupId::Custom("LensDistortion".into())) {
                            if let Some(v) = md.get_t(TagId::Data) as Option<&serde_json::Value> {
                                // lens.focal_length = v.get("focal_length_nm").and_then(|x| x.as_f64()).map(|x| (x / 1000000.0) as f32);
                                let focal_length_nm = v.get("focal_length_nm").and_then(|x| x.as_f64()).unwrap_or_default();
                                let effective_sensor_height_nm = v.get("effective_sensor_height_nm").and_then(|x| x.as_f64()).unwrap_or(1.0);

                                lens_info.pixel_focal_length = Some(((focal_length_nm as f64 / effective_sensor_height_nm as f64) * size.1 as f64) as f32);
                            }
                        }
                    }
                    if lens_info.pixel_pitch.is_some() && lens_info.capture_area_size.is_some() && (lens_info.pixel_focal_length.is_some() || lens_info.focal_length.is_some()) {
                        lens_params.insert(timestamp_us, lens_info.clone());
                    }

                    if let Some(map) = tag_map.get(&GroupId::Default) {
                        if let Some(v) = map.get_t(TagId::FrameRate) as Option<&f64> {
                            frame_rate = Some(*v);
                        }
                        if let Some(v) = map.get_t(TagId::Metadata) as Option<&serde_json::Value> {
                            crate::util::merge_json(&mut additional_data, v);
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

                if lens_positions.is_empty() && !crop_score.is_empty() && crop_score.len() == quats.len() {
                    lens_positions = quats.iter().zip(crop_score.iter()).map(|((ts, _), crop)| (*ts, *crop)).collect();
                }

                quaternions = quats;
            }
        }

        let raw_imu = util::normalized_imu_interpolated(&input, Some("XYZ".into())).unwrap_or_default();

        let mut has_accurate_timestamps = input.has_accurate_timestamps();
        if let serde_json::Value::Object(o) = &mut additional_data {
            match o.get("has_accurate_timestamps") {
                Some(serde_json::Value::String(x)) => { if x == "true" || x == "1" { has_accurate_timestamps = true; } }
                Some(serde_json::Value::Bool(x)) => { if *x { has_accurate_timestamps = true; } }
                _ => {}
            }
            o.remove("has_accurate_timestamps");
        }

        let mut md = FileMetadata {
            imu_orientation,
            detected_source: Some(detected_source),
            quaternions,
            image_orientations,
            gravity_vectors,
            lens_positions,
            lens_params,
            raw_imu,
            frame_readout_time: input.frame_readout_time(),
            frame_rate,
            lens_profile,
            camera_identifier,
            has_accurate_timestamps,
            additional_data,
            per_frame_time_offsets: Default::default(),
            per_frame_data: Default::default(),
        };

        let sample_rate = Self::get_sample_rate(&md);
        let mut original_sample_rate = sample_rate;
        if let Some(ref samples) = input.samples {
            for info in samples {
                if let Some(ref tag_map) = info.tag_map {
                    // --------------------------------- Sony ---------------------------------
                    telemetry_parser::try_block!({
                        let model_offset = if input.camera_model().map(|x| x == "DSC-RX0M2").unwrap_or_default() { 1.5 } else { 0.0 };
                        let imager = tag_map.get(&GroupId::Imager)?;
                        let gyro   = tag_map.get(&GroupId::Gyroscope)?;

                        let first_frame_ts     =  (imager.get_t(TagId::FirstFrameTimestamp) as Option<&f64>)?;
                        let exposure_time      =  (imager.get_t(TagId::ExposureTime)        as Option<&f64>)?;
                        let offset             =  (gyro  .get_t(TagId::TimeOffset)          as Option<&f64>)?;
                        let sampling_frequency = *(gyro  .get_t(TagId::Frequency)           as Option<&i32>)? as f64;
                        let scaler             = *(gyro  .get_t(TagId::Unknown(0xe436))     as Option<&i32>).unwrap_or(&1000000) as f64;
                        original_sample_rate = sampling_frequency;

                        let rounded_offset = (offset * 1000.0 * (1000000.0 / scaler)).round();
                        let offset_diff = ((rounded_offset - (1000000.0 / sampling_frequency) * (rounded_offset / (1000000.0 / sampling_frequency)).floor())).round() / 1000.0;

                        let frame_offset = first_frame_ts - (exposure_time / 2.0) + (md.frame_readout_time.unwrap_or_default() / 2.0) + model_offset + offset_diff - offset;

                        md.per_frame_time_offsets.push(frame_offset / sampling_frequency * sample_rate);
                    });
                    // --------------------------------- Sony ---------------------------------
                    // --------------------------------- Insta360 ---------------------------------
                    // Timing
                    if input.camera_type() == "Insta360" {
                        telemetry_parser::try_block!({
                            use telemetry_parser::tags_impl::TimeScalar;
                            let exp = (tag_map.get(&GroupId::Exposure)?.get_t(TagId::Data) as Option<&Vec<TimeScalar<f64>>>)?;

                            let mut video_ts = 0.0;
                            let mut zero_ref = None;
                            for x in exp {
                                if x.t >= 0.0 {
                                    if zero_ref.is_none() {
                                        zero_ref = Some(x.t * 1000.0);
                                        log::debug!("Insta360 first frame reference time: {:.4}", x.t * 1000.0);
                                    }
                                    // The additional 0.9 ms is a mystery
                                    let diff = (video_ts - x.t) * 1000.0;
                                    md.per_frame_time_offsets.push(-(x.v * 1000.0 / 2.0) - 0.9 - diff - zero_ref.unwrap());

                                    video_ts += 1.0 / fps;
                                }
                            }
                        });
                    }
                    // --------------------------------- Insta360 ---------------------------------
                }
            }
            if input.camera_type() == "Sony" {
                if let Some(frt) = md.frame_readout_time {
                    md.frame_readout_time = Some(frt / original_sample_rate * sample_rate);
                }
            }
        }

        Ok(md)
    }

    pub fn clear(&mut self) {
        self.quaternions.clear();
        self.smoothed_quaternions.clear();
        self.org_smoothed_quaternions.clear();
        self.raw_imu.clear();
        self.imu_rotation = None;
        self.acc_rotation = None;
        self.imu_lpf = 0.0;
        self.file_metadata = Default::default();
        self.clear_offsets();
    }

    pub fn load_from_telemetry(&mut self, telemetry: FileMetadata) {
        if self.duration_ms <= 0.0 {
            ::log::error!("Invalid duration_ms {}", self.duration_ms);
            return;
        }

        self.clear();

        self.imu_orientation = telemetry.imu_orientation.clone();

        self.file_metadata = telemetry;

        if !self.file_metadata.quaternions.is_empty() {
            self.quaternions = self.file_metadata.quaternions.clone();
            self.integration_method = 0;
            let len = self.file_metadata.quaternions.len() as f64;
            let first_ts = self.file_metadata.quaternions.iter().next()      .map(|x| *x.0 as f64 / 1000.0).unwrap_or_default();
            let last_ts  = self.file_metadata.quaternions.iter().next_back() .map(|x| *x.0 as f64 / 1000.0).unwrap_or_default();
            let imu_duration = (last_ts - first_ts) * ((len + 1.0) / len);
            if (imu_duration - self.duration_ms).abs() > 0.01 {
                log::warn!("IMU duration {imu_duration} is different than video duration ({})", self.duration_ms);
                if imu_duration > 0.0 {
                    self.duration_ms = imu_duration;
                }
            }
        }

        if !self.file_metadata.raw_imu.is_empty() {
            let len = self.file_metadata.raw_imu.len() as f64;
            let first_ts = self.file_metadata.raw_imu.first().map(|x| x.timestamp_ms).unwrap_or_default();
            let last_ts  = self.file_metadata.raw_imu.last() .map(|x| x.timestamp_ms).unwrap_or_default();
            let imu_duration = (last_ts - first_ts) * ((len + 1.0) / len);
            if (imu_duration - self.duration_ms).abs() > 0.01 {
                log::warn!("IMU duration {imu_duration} is different than video duration ({})", self.duration_ms);
                if imu_duration > 0.0 {
                    self.duration_ms = imu_duration;
                }
            }
            self.apply_transforms();
        } else if self.quaternions.is_empty() {
            self.integrate();
        }
    }
    pub fn integrate(&mut self) {
        match self.integration_method {
            0 => {
                self.quaternions = if self.file_metadata.detected_source.as_ref().unwrap_or(&"".into()).starts_with("GoPro") && !self.file_metadata.quaternions.is_empty() && (self.file_metadata.gravity_vectors.is_none() || !self.use_gravity_vectors) {
                    log::info!("No gravity vectors - using accelerometer");
                    QuaternionConverter::convert(self.horizon_lock_integration_method, &self.file_metadata.quaternions, self.file_metadata.image_orientations.as_ref().unwrap_or(&TimeQuat::default()), &self.raw_imu, self.duration_ms)
                } else {
                    self.file_metadata.quaternions.clone()
                };
                if self.imu_lpf > 0.0 && !self.quaternions.is_empty() && self.duration_ms > 0.0 {
                    let sample_rate = self.quaternions.len() as f64 / (self.duration_ms / 1000.0);
                    if let Err(e) = super::filtering::Lowpass::filter_quats_forward_backward(self.imu_lpf, sample_rate, &mut self.quaternions) {
                        log::error!("Filter error {:?}", e);
                    }
                }
                if let Some(rot) = self.imu_rotation {
                    for (_ts, q) in &mut self.quaternions {
                        *q = rot * *q;
                    }
                }
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

    pub fn recompute_smoothness(&self, alg: &dyn SmoothingAlgorithm, horizon_lock: super::smoothing::horizon::HorizonLock, stabilization_params: &StabilizationParams, keyframes: &KeyframeManager) -> (TimeQuat, TimeQuat, (f64, f64, f64)) {
        let mut smoothed_quaternions = self.quaternions.clone();
        if true {
            // Lock horizon, then smooth
            horizon_lock.lock(&mut smoothed_quaternions, &self.quaternions, &self.file_metadata.gravity_vectors, self.use_gravity_vectors, self.integration_method, keyframes, stabilization_params);
            smoothed_quaternions = alg.smooth(&smoothed_quaternions, self.duration_ms, stabilization_params, keyframes);
        } else {
            // Smooth, then lock horizon
            smoothed_quaternions = alg.smooth(&smoothed_quaternions, self.duration_ms, stabilization_params, keyframes);
            horizon_lock.lock(&mut smoothed_quaternions, &self.quaternions, &self.file_metadata.gravity_vectors, self.use_gravity_vectors, self.integration_method, keyframes, stabilization_params);
        }

        let max_angles = crate::Smoothing::get_max_angles(&self.quaternions, &smoothed_quaternions, stabilization_params);

        let org_smoothed_quaternions = smoothed_quaternions.clone();
        for (sq, q) in smoothed_quaternions.iter_mut().zip(self.quaternions.iter()) {
            // rotation quaternion from smooth motion -> raw motion to counteract it
            *sq.1 = sq.1.inverse() * q.1;
        }
        (smoothed_quaternions, org_smoothed_quaternions, max_angles)
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
    pub fn get_offsets_plus_linear(&self) -> BTreeMap<i64, (f64, f64)> {
        self.offsets.iter().map(|(k, v)| (*k, (*v, self.offsets_linear.get(k).copied().unwrap_or(*v)))).collect()
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

    fn line_fit(offsets: &BTreeMap<i64, f64>) -> Option<[f64; 3]> {
        let a = OMatrix::<f64, nalgebra::Dyn, U2>::from_row_iterator(offsets.len(), offsets.iter().flat_map(|(k, _)| [*k as f64, 1.0]));
        let b = OVector::<f64, nalgebra::Dyn>::from_iterator(offsets.len(), offsets.iter().map(|(_, v)| *v));

        let svd = nalgebra::linalg::SVD::new(a.clone(), true, true);
        let solution = svd.solve(&b, 1e-14).ok()?;
        if solution.len() >= 2 {
            let model: OVector<f64, nalgebra::Dyn> = a * &solution;
            let l1: OVector<f64, nalgebra::Dyn> = model - b;
            let residuals: f64 = l1.dot(&l1);

            Some([solution[0], solution[1], residuals])
        } else {
            None
        }
    }

    pub fn adjust_offsets(&mut self) {
        if self.prevent_recompute { return; }
        // Calculate line fit
        if self.offsets.len() > 1 {
            let len = self.offsets.len();
            let keys: Vec<i64> = self.offsets.keys().copied().collect();

            #[derive(Default)]
            struct Params {
                offsets: BTreeMap<i64, f64>,
                rsquared: f64,
                coeffs: [f64; 3]
            }
            let mut best = Params { rsquared: 1000.0, ..Default::default() };

            let max_fitting_error = 5.0; // max 5 ms

            for i in 0..len {
                for j in 0..len {
                    if i != j {
                        let i_offset = self.offsets.get(&keys[i]).unwrap();
                        let j_offset = self.offsets.get(&keys[j]).unwrap();
                        let slope = (j_offset - i_offset) / (keys[j] - keys[i]) as f64;
                        let intersect = i_offset - keys[i] as f64 * slope;

                        let within_error: BTreeMap<i64, f64> = self.offsets.iter().filter_map(|(k, v)| {
                            if ((*k as f64 * slope + intersect) - *v).abs() < max_fitting_error {
                                Some((*k, *v))
                            } else {
                                None
                            }
                        }).collect();

                        if within_error.len() >= best.offsets.len() && within_error != best.offsets {
                            if let Some(solution) = Self::line_fit(&within_error) {
                                let close_constant = solution[0].abs() < 0.1;
                                if within_error.len() > 2 && close_constant {
                                    if solution[2] < best.rsquared {
                                        best = Params {
                                            rsquared: solution[2],
                                            offsets: within_error.clone(),
                                            coeffs: solution.clone()
                                        };
                                    }
                                } else if close_constant {
                                    best = Params {
                                        rsquared: best.rsquared,
                                        offsets: within_error.clone(),
                                        coeffs: solution.clone()
                                    };
                                }
                            }
                        }
                    }
                }
            }

            self.offsets_linear.clear();
            if !best.offsets.is_empty() {
                for (k, _) in &self.offsets {
                    let fitted = *k as f64 * best.coeffs[0] + best.coeffs[1];
                    self.offsets_linear.insert(*k, fitted);
                }
            } else {
                if let Some(solution) = Self::line_fit(&self.offsets) {
                    for (k, _) in &self.offsets {
                        let fitted = *k as f64 * solution[0] + solution[1];
                        self.offsets_linear.insert(*k, fitted);
                    }
                }
            }
        } else {
            self.offsets_linear = self.offsets.clone();
        }

        self.offsets_adjusted = self.offsets.iter().map(|(k, v)| (*k + (*v * 1000.0).round() as i64, *v)).collect::<BTreeMap<i64, f64>>();
    }

    pub fn apply_transforms(&mut self) {
        self.raw_imu = self.file_metadata.raw_imu.clone();

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

        if self.imu_lpf > 0.0 && !self.file_metadata.raw_imu.is_empty() && self.duration_ms > 0.0 {
            let sample_rate = self.file_metadata.raw_imu.len() as f64 / (self.duration_ms / 1000.0);
            if let Err(e) = super::filtering::Lowpass::filter_gyro_forward_backward(self.imu_lpf, sample_rate, &mut self.raw_imu) {
                log::error!("Filter error {:?}", e);
            }
        }

        self.integrate();
    }

    fn quat_at_timestamp(&self, quats: &TimeQuat, mut timestamp_ms: f64) -> Quat64 {
        if quats.len() < 2 || self.duration_ms <= 0.0 { return Quat64::identity(); }

        timestamp_ms -= self.offset_at_video_timestamp(timestamp_ms);

        if let Some(&first_ts) = quats.keys().next() {
            if let Some(&last_ts) = quats.keys().next_back() {
                let lookup_ts = ((timestamp_ms * 1000.0).round() as i64).min(last_ts).max(first_ts);

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

    /// Partial clone with data necessary only for computations
    pub fn clone_quaternions(&self) -> Self {
        Self {
            duration_ms:          self.duration_ms,
            quaternions:          self.quaternions.clone(),
            smoothed_quaternions: self.smoothed_quaternions.clone(),
            offsets:              self.offsets.clone(),
            offsets_adjusted:     self.offsets_adjusted.clone(),
            file_metadata:        FileMetadata {
                gravity_vectors:        self.file_metadata.gravity_vectors.clone(),
                lens_positions:         self.file_metadata.lens_positions.clone(),
                lens_params:            self.file_metadata.lens_params.clone(),
                per_frame_time_offsets: self.file_metadata.per_frame_time_offsets.clone(),
                per_frame_data:         self.file_metadata.per_frame_data.clone(),
                additional_data:        self.file_metadata.additional_data.clone(),
                ..Default::default()
            },
            use_gravity_vectors:  self.use_gravity_vectors,
            integration_method:   self.integration_method,
            ..Default::default()
        }
    }

    pub fn get_checksum(&self) -> u64 {
        use std::hash::Hasher;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        if let Some(v) = &self.file_metadata.detected_source { hasher.write(v.as_bytes()); }
        if let Some(v) = &self.imu_orientation { hasher.write(v.as_bytes()); }
        if let Some(v) = &self.imu_rotation_angles { hasher.write_u64(v[0].to_bits()); hasher.write_u64(v[1].to_bits()); hasher.write_u64(v[2].to_bits()); }
        if let Some(v) = &self.acc_rotation_angles { hasher.write_u64(v[0].to_bits()); hasher.write_u64(v[1].to_bits()); hasher.write_u64(v[2].to_bits()); }
        if let Some(v) = &self.gyro_bias { hasher.write_u64(v[0].to_bits()); hasher.write_u64(v[1].to_bits()); hasher.write_u64(v[2].to_bits()); }
        hasher.write(self.file_url.as_bytes());
        hasher.write_u64(self.duration_ms.to_bits());
        hasher.write_u64(self.imu_lpf.to_bits());
        hasher.write_usize(self.raw_imu.len());
        hasher.write_usize(self.file_metadata.raw_imu.len());
        hasher.write_usize(self.quaternions.len());
        hasher.write_usize(self.file_metadata.quaternions.len());
        hasher.write_usize(self.file_metadata.image_orientations.as_ref().map(|v| v.len()).unwrap_or_default());
        hasher.write_usize(self.file_metadata.lens_positions.len());
        hasher.write_usize(self.file_metadata.lens_params.len());
        hasher.write_u32(if self.use_gravity_vectors { 1 } else { 0 });
        hasher.write_usize(self.integration_method);
        for (ts, v) in &self.offsets {
            hasher.write_i64(*ts);
            hasher.write_u64(v.to_bits());
        }
        if let Some((ts, q)) = self.quaternions.first_key_value() {
            let v = q.as_vector();
            hasher.write_i64(*ts);
            hasher.write_u64(v[0].to_bits());
            hasher.write_u64(v[1].to_bits());
            hasher.write_u64(v[2].to_bits());
            hasher.write_u64(v[3].to_bits());
        }
        if let Some((ts, q)) = self.quaternions.last_key_value() {
            let v = q.as_vector();
            hasher.write_i64(*ts);
            hasher.write_u64(v[0].to_bits());
            hasher.write_u64(v[1].to_bits());
            hasher.write_u64(v[2].to_bits());
            hasher.write_u64(v[3].to_bits());
        }

        hasher.finish()
    }

    pub fn get_sample_rate(file_metadata: &FileMetadata) -> f64 {
        if file_metadata.raw_imu.len() > 2 {
            let len = file_metadata.raw_imu.len() as f64;
            let duration_ms = file_metadata.raw_imu.last().unwrap().timestamp_ms - file_metadata.raw_imu.first().unwrap().timestamp_ms;
            let duration_ms = duration_ms * ((len + 1.0) / len.max(1.0));
            file_metadata.raw_imu.len() as f64 / (duration_ms / 1000.0)
        } else if file_metadata.quaternions.len() > 2 {
            let len = file_metadata.quaternions.len() as f64;
            let first = *file_metadata.quaternions.iter().next().unwrap().0 as f64 / 1000.0;
            let last = *file_metadata.quaternions.iter().next_back().unwrap().0 as f64 / 1000.0;
            let duration_ms = last - first;
            let duration_ms = duration_ms * ((len + 1.0) / len.max(1.0));
            file_metadata.quaternions.len() as f64 / (duration_ms / 1000.0)
        } else {
            0.0
        }
    }

    pub fn find_bias(&self, timestamp_start: f64, timestamp_stop: f64) -> (f64, f64, f64) {
        let ts_start = timestamp_start - self.offset_at_video_timestamp(timestamp_start);
        let ts_stop = timestamp_stop - self.offset_at_video_timestamp(timestamp_stop);
        let mut bias_vals = [0.0, 0.0, 0.0];
        let mut n = 0;

        for x in &self.file_metadata.raw_imu {
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
