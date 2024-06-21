// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

mod file_metadata;
mod imu_transforms;
mod sony;
mod catmull_rom;
pub use file_metadata::*;
pub use imu_transforms::*;

use nalgebra::*;
use std::iter::zip;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::{ Arc, atomic::AtomicBool };
use telemetry_parser::{ Input, util };
use telemetry_parser::tags_impl::{ GetWithType, GroupId, TagId, TimeQuaternion };

use crate::camera_identifier::CameraIdentifier;
use crate::filesystem;

use super::imu_integration::*;
use super::smoothing::SmoothingAlgorithm;
use crate::StabilizationParams;

const DEG2RAD: f64 = std::f64::consts::PI / 180.0;

pub type Quat64 = UnitQuaternion<f64>;
pub type TimeIMU = telemetry_parser::util::IMUData;
pub type TimeQuat = BTreeMap<i64, Quat64>; // key is timestamp_us
pub type TimeVec = BTreeMap<i64, Vector3<f64>>; // key is timestamp_us

#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileLoadOptions {
    pub sample_index: Option<usize>
}

#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct GyroSource {
    pub file_load_options: FileLoadOptions,

    pub duration_ms: f64,

    raw_imu: Vec<TimeIMU>,

    pub imu_transforms: IMUTransforms,

    pub integration_method: usize,

    pub quaternions: TimeQuat,
    pub smoothed_quaternions: TimeQuat,

    pub use_gravity_vectors: bool,
    pub horizon_lock_integration_method: i32,

    pub max_angles: (f64, f64, f64), // (pitch, yaw, roll) in deg

    pub smoothing_status: serde_json::Value,

    pub prevent_recompute: bool,

    pub file_metadata: ReadOnlyFileMetadata, // Once this is set, it's never modified

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
        self.file_metadata.read().has_motion()
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
        let mut file = filesystem::open_file(&base, url, false, false)?;
        let filesize = file.size;
        let mut input = Input::from_stream(file.get_file(), filesize, &url, progress_cb, cancel_flag)?;

        let camera_identifier = CameraIdentifier::from_telemetry_parser(&input, size.0, size.1, fps).ok();

        let mut detected_source = input.camera_type();
        if let Some(m) = input.camera_model() { detected_source.push(' '); detected_source.push_str(m); }

        let mut imu_orientation = None;
        let mut quaternions = TimeQuat::default();
        let mut gravity_vectors: Option<TimeVec> = None;
        let mut image_orientations = None;
        let mut lens_profile = None;
        let mut frame_rate = None;
        let mut digital_zoom = None;
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
                            lens_info.capture_area_size = Some((size.0 as f32, size.1 as f32));
                        }
                        if let Some(v) = im.get_t(TagId::PixelPitch) as Option<&(u32, u32)> { lens_info.pixel_pitch = Some(*v); }
                        if let Some(v) = im.get_t(TagId::CaptureAreaSize) as Option<&(f32, f32)> { lens_info.capture_area_size = Some(*v); }
                        if let Some(v) = im.get_t(TagId::CaptureAreaOrigin) as Option<&(f32, f32)> { lens_info.capture_area_origin = Some(*v); }
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
                    if let Some(map) = tag_map.get(&GroupId::Default) {
                        if let Some(v) = map.get_t(TagId::Unknown(0x445a5354/*DZST*/)) as Option<&u32> {
                            if *v != 0 {
                                let max = *(map.get_t(TagId::Unknown(0x445a4d58/*DZMX*/)) as Option<&f32>).unwrap_or(&1.4) as f64;
                                digital_zoom = Some(1.0 + (*v as f64 / 100.0) * (max - 1.0));
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
                            Some(v) if v.len() == 3 => v.clone(),
                            _ => "XYZ".into()
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
                    let additional_data = additional_data.as_object_mut().unwrap();
                    if !additional_data.contains_key("recording_settings") {
                        let mut settings = serde_json::Map::new();
                        if let Some(map) = tag_map.get(&GroupId::Exposure) {
                            if let Some(v) = map.get(&TagId::ShutterAngle) { settings.insert(String::from("Shutter angle"), v.value.to_string().into()); }
                            if let Some(v) = map.get(&TagId::ShutterSpeed) { settings.insert(String::from("Shutter speed"), v.value.to_string().into()); }
                            if let Some(v) = map.get(&TagId::AutoExposureMode) { settings.insert(String::from("Exposure"), v.value.to_string().into()); }
                                 if let Some(v) = map.get(&TagId::Custom("ISOValue3".into())) { settings.insert(String::from("ISO"), v.value.to_string().into()); }
                            else if let Some(v) = map.get(&TagId::ISOValue) { settings.insert(String::from("ISO"), v.value.to_string().into()); }
                        }
                        if let Some(map) = tag_map.get(&GroupId::Colors) {
                            if let Some(v) = map.get(&TagId::ColorPrimaries)       { settings.insert(String::from("Color primaries"),      v.value.to_string().into()); }
                            if let Some(v) = map.get(&TagId::CaptureGammaEquation) { settings.insert(String::from("Gamma equation"),       v.value.to_string().into()); }
                            if let Some(v) = map.get(&TagId::AutoWBMode)           { settings.insert(String::from("White balance mode"), v.value.to_string().into()); }
                            if let Some(v) = map.get(&TagId::WhiteBalance)         { settings.insert(String::from("White balance"),      v.value.to_string().into()); }
                        }
                        if let Some(map) = tag_map.get(&GroupId::Lens) {
                                 if let Some(v) = map.get(&TagId::IrisTStop) { settings.insert(String::from("Iris"),         v.value.to_string().into()); }
                            else if let Some(v) = map.get(&TagId::IrisFStop) { settings.insert(String::from("Iris"),         v.value.to_string().into()); }
                            if let Some(v) = map.get(&TagId::FocalLength)    { settings.insert(String::from("Focal length"), v.value.to_string().into()); }
                        }
                        if let Some(map) = tag_map.get(&GroupId::Autofocus) {
                            if let Some(v) = map.get(&TagId::AutoFocusMode) { settings.insert(String::from("Focus mode"), v.value.to_string().into()); }
                        }
                        if !settings.is_empty() {
                            additional_data.insert("recording_settings".to_owned(), serde_json::Value::Object(settings));
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
            per_frame_time_offsets: Vec::new(),
            digital_zoom,
            camera_stab_data: Vec::new(),
            mesh_correction:  Vec::new(),
        };

        let sample_rate = Self::get_sample_rate(&md);
        let mut original_sample_rate = sample_rate;
        let mut is_temp = sony::ISTemp::default();
        let mut mesh_cache = BTreeMap::new();
        if let Some(ref samples) = input.samples {
            for info in samples {
                if let Some(ref tag_map) = info.tag_map {
                    // --------------------------------- Sony ---------------------------------
                    if let Some((org_sample_rate, offset)) = sony::get_time_offset(&md, &input, tag_map, sample_rate) {
                        original_sample_rate = org_sample_rate;
                        md.per_frame_time_offsets.push(offset);
                    }
                    sony::init_lens_profile(&mut md, &input, tag_map, size, info);
                    sony::stab_collect(&mut is_temp, tag_map, info, fps);
                    if let Some(mesh) = sony::get_mesh_correction(tag_map, &mut mesh_cache) {
                        md.mesh_correction.push(mesh);
                    }
                    // --------------------------------- Sony ---------------------------------

                    // --------------------------------- Sony ---------------------------------
                    // --------------------------------- Insta360 ---------------------------------
                    // Timing
                    if input.camera_type() == "Insta360" {
                        telemetry_parser::try_block!({
                            use telemetry_parser::tags_impl::TimeScalar;
                            let exp = (tag_map.get(&GroupId::Exposure)?.get_t(TagId::Data) as Option<&Vec<TimeScalar<f64>>>)?;

                            let mut video_ts = 0.0;
                            let mut zero_ref = None;
                            let mut prev_t = 0.0;
                            for x in exp {
                                if x.t > prev_t || x.t == 0.0 {
                                    if zero_ref.is_none() {
                                        zero_ref = Some(x.t * 1000.0);
                                        log::debug!("Insta360 first frame reference time: {:.4}", x.t * 1000.0);
                                    }
                                    // The additional 0.9 ms is a mystery
                                    let diff = (video_ts - x.t) * 1000.0;
                                    md.per_frame_time_offsets.push(-(x.v * 1000.0 / 2.0) - 0.9 - diff - zero_ref.unwrap());

                                    video_ts += 1.0 / fps;
                                    prev_t = x.t;
                                }
                            }
                        });
                    }
                    // --------------------------------- Insta360 ---------------------------------

                }
            }

            // IBIS
            telemetry_parser::try_block!({
            use
            telemetry_parser::tags_impl::*;
            let ibis = tag_map.get(&GroupId::IBIS)?;

            let shift = (ibis.get_t(TagID::Data) as Option<&Vec<TimeVector3<i32>>>)?;

            let angle = (ibis.get_t(TagId::Data2) as Option<&Vec<TimeVector3<i32>>>)?;

            let mut xs = BTreeMap::<i64,f64>::new();

            let mut ys = BTreeMap::i64,f64>::new();

            let mut th = BTreeMap::i64,f64>::new()

            let pixel_pitch = (8400.0,8400.0);

            let e406 = 1000000000.0;

            assert_eq!(shift.len(),angle.len());

            for (s,a) in shift.intro_iter().zip(angle.intro_iter())
            {
            let x = ((e406 / 1000000000.0) * (4096.0 / pixel_pitch.0)) * s.x as f64;

            let y = ((e406 / 1000000000.0) * (4096.0 / pixel_pitch.1)) * s.y as f64;

            if xx < 100 {
            dbg!((x, y, s.x, s.y, a.z));

            xx += 1;
            }

            xs.insert(s.t as i64, x);
            ys.insert(s.t as i64, y);
            th.insert(s.t as i64, a.z as f64);
            }

            let mut xs2 = Vec::new();
            let mut ys2 = Vec::new();
            let mut angles = Vec::new();

            for vid_y in 0..2160 {

            let x = telemetry_parser::util::interpolate_at_timestamp((vid_y as f64 * (8850.0 / 2160.0)).round() as i64, &xs);

            let y = telemetry_parser::util::interpolate_at_timestamp((vid_y as f64 * (8850.0 / 2160.0)).round() as i64, &ys);

            let t = telemetry_parser::util::interpolate_at_timestamp((vid_y as f64 * (8850.0 / 2160.0)).round() as i64, &th);

            angles.push(t / 1000.0);
            xs2.push((x / 4096.0));
            ys2.push((y / 4096.0));
            }


            md.per_frame_data.push(serde_json::json!({
            "translatex": xs2,
            "translatey":ys2,
            "angle": angles
            }));
            });






            if input.camera_type() == "Sony" {
                if let Some(frt) = md.frame_readout_time {
                    md.frame_readout_time = Some(frt / original_sample_rate * sample_rate);
                }
                md.camera_stab_data = sony::stab_calc_splines(&md, &is_temp, sample_rate, fps, size).unwrap_or_default();
            }
        }

        Ok(md)
    }

    pub fn clear(&mut self) {
        self.quaternions.clear();
        self.smoothed_quaternions.clear();
        self.raw_imu.clear();
        self.imu_transforms.imu_rotation = None;
        self.imu_transforms.acc_rotation = None;
        self.imu_transforms.imu_lpf = 0.0;
        self.file_metadata = Default::default();
        self.clear_offsets();
    }

    pub fn load_from_telemetry(&mut self, telemetry: FileMetadata) {
        if self.duration_ms <= 0.0 {
            ::log::error!("Invalid duration_ms {}", self.duration_ms);
            return;
        }

        self.clear();

        self.imu_transforms.imu_orientation = telemetry.imu_orientation.clone();

        let has_quats = !telemetry.quaternions.is_empty();
        let has_raw_imu = !telemetry.raw_imu.is_empty();

        self.file_metadata = telemetry.into();

        if has_quats {
            let file_metadata = self.file_metadata.read();
            self.quaternions = file_metadata.quaternions.clone();
            self.integration_method = 0;
            let len = file_metadata.quaternions.len() as f64;
            let first_ts = file_metadata.quaternions.iter().next()      .map(|x| *x.0 as f64 / 1000.0).unwrap_or_default();
            let last_ts  = file_metadata.quaternions.iter().next_back() .map(|x| *x.0 as f64 / 1000.0).unwrap_or_default();
            let imu_duration = (last_ts - first_ts) * ((len + 1.0) / len);
            if (imu_duration - self.duration_ms).abs() > 0.01 {
                log::warn!("IMU duration {imu_duration} is different than video duration ({})", self.duration_ms);
                if imu_duration > 0.0 {
                    self.duration_ms = imu_duration;
                }
            }
        }

        if has_raw_imu {
            {
                let file_metadata = self.file_metadata.read();
                let len = file_metadata.raw_imu.len() as f64;
                let first_ts = file_metadata.raw_imu.first().map(|x| x.timestamp_ms).unwrap_or_default();
                let last_ts  = file_metadata.raw_imu.last() .map(|x| x.timestamp_ms).unwrap_or_default();
                let imu_duration = (last_ts - first_ts) * ((len + 1.0) / len);
                if (imu_duration - self.duration_ms).abs() > 0.01 {
                    log::warn!("IMU duration {imu_duration} is different than video duration ({})", self.duration_ms);
                    if imu_duration > 0.0 {
                        self.duration_ms = imu_duration;
                    }
                }
            }
            self.apply_transforms();
        } else if self.quaternions.is_empty() {
            self.integrate();
        }
    }
    pub fn integrate(&mut self) {
        let file_metadata = self.file_metadata.read();
        match self.integration_method {
            0 => {
                self.quaternions = if file_metadata.detected_source.as_deref().unwrap_or("").starts_with("GoPro") && !file_metadata.quaternions.is_empty() && (file_metadata.gravity_vectors.is_none() || !self.use_gravity_vectors) {
                    log::info!("No gravity vectors - using accelerometer");
                    QuaternionConverter::convert(self.horizon_lock_integration_method, &file_metadata.quaternions, file_metadata.image_orientations.as_ref().unwrap_or(&TimeQuat::default()), self.raw_imu(&file_metadata), self.duration_ms)
                } else {
                    file_metadata.quaternions.clone()
                };
                if self.imu_transforms.imu_lpf > 0.0 && !self.quaternions.is_empty() && self.duration_ms > 0.0 {
                    let sample_rate = self.quaternions.len() as f64 / (self.duration_ms / 1000.0);
                    if let Err(e) = super::filtering::Lowpass::filter_quats_forward_backward(self.imu_transforms.imu_lpf, sample_rate, &mut self.quaternions) {
                        log::error!("Filter error {:?}", e);
                    }
                }
                if let Some(rot) = self.imu_transforms.imu_rotation {
                    for (_ts, q) in &mut self.quaternions {
                        *q = rot * *q;
                    }
                }
            },
            1 => self.quaternions = ComplementaryIntegrator  ::integrate(self.raw_imu(&file_metadata), self.duration_ms),
            2 => self.quaternions = VQFIntegrator            ::integrate(self.raw_imu(&file_metadata), self.duration_ms),
            3 => self.quaternions = SimpleGyroIntegrator     ::integrate(self.raw_imu(&file_metadata), self.duration_ms),
            4 => self.quaternions = SimpleGyroAccelIntegrator::integrate(self.raw_imu(&file_metadata), self.duration_ms),
            5 => self.quaternions = MahonyIntegrator         ::integrate(self.raw_imu(&file_metadata), self.duration_ms),
            6 => self.quaternions = MadgwickIntegrator       ::integrate(self.raw_imu(&file_metadata), self.duration_ms),
            _ => log::error!("Unknown integrator")
        }
    }

    pub fn recompute_smoothness(&self, alg: &dyn SmoothingAlgorithm, horizon_lock: super::smoothing::horizon::HorizonLock, compute_params: &crate::ComputeParams) -> (TimeQuat, (f64, f64, f64)) {
        let file_metadata = self.file_metadata.read();
        let mut smoothed_quaternions = self.quaternions.clone();
        if true {
            // Lock horizon, then smooth
            horizon_lock.lock(&mut smoothed_quaternions, &self.quaternions, &file_metadata.gravity_vectors, self.use_gravity_vectors, self.integration_method, compute_params);
            smoothed_quaternions = alg.smooth(&smoothed_quaternions, self.duration_ms, compute_params);
        } else {
            // Smooth, then lock horizon
            smoothed_quaternions = alg.smooth(&smoothed_quaternions, self.duration_ms, compute_params);
            horizon_lock.lock(&mut smoothed_quaternions, &self.quaternions, &file_metadata.gravity_vectors, self.use_gravity_vectors, self.integration_method, compute_params);
        }

        let max_angles = crate::Smoothing::get_max_angles(&self.quaternions, &smoothed_quaternions, compute_params);

        for (sq, q) in smoothed_quaternions.iter_mut().zip(self.quaternions.iter()) {
            // rotation quaternion from smooth motion -> raw motion to counteract it
            *sq.1 = sq.1.inverse() * q.1;
        }
        (smoothed_quaternions, max_angles)
    }

    pub fn raw_imu<'a>(&'a self, file_metadata: &'a FileMetadata) -> &'a Vec<TimeIMU> {
        if !self.raw_imu.is_empty() { return &self.raw_imu }
        return &file_metadata.raw_imu;
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
        let file_metadata = self.file_metadata.read();

        if self.imu_transforms.has_any() {
            self.raw_imu = file_metadata.raw_imu.clone();
            for x in self.raw_imu.iter_mut() {
                if let Some(g) = x.gyro.as_mut() {
                    self.imu_transforms.transform(g, false);
                }
                if let Some(a) = x.accl.as_mut() {
                    self.imu_transforms.transform(a, true);
                }
                if let Some(m) = x.magn.as_mut() {
                    self.imu_transforms.transform(m, false);
                }
            }
            if self.imu_transforms.imu_lpf > 0.0 && !file_metadata.raw_imu.is_empty() && self.duration_ms > 0.0 {
                let sample_rate = file_metadata.raw_imu.len() as f64 / (self.duration_ms / 1000.0);
                if let Err(e) = super::filtering::Lowpass::filter_gyro_forward_backward(self.imu_transforms.imu_lpf, sample_rate, &mut self.raw_imu) {
                    log::error!("Filter error {:?}", e);
                }
            }
        } else {
            self.raw_imu.clear();
        }

        drop(file_metadata);

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

    pub fn get_checksum(&self) -> u64 {
        use std::hash::Hasher;
        let file_metadata = self.file_metadata.read();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        if let Some(v) = &file_metadata.detected_source { hasher.write(v.as_bytes()); }
        if let Some(v) = &self.imu_transforms.imu_orientation { hasher.write(v.as_bytes()); }
        if let Some(v) = &self.imu_transforms.imu_rotation_angles { hasher.write_u64(v[0].to_bits()); hasher.write_u64(v[1].to_bits()); hasher.write_u64(v[2].to_bits()); }
        if let Some(v) = &self.imu_transforms.acc_rotation_angles { hasher.write_u64(v[0].to_bits()); hasher.write_u64(v[1].to_bits()); hasher.write_u64(v[2].to_bits()); }
        if let Some(v) = &self.imu_transforms.gyro_bias { hasher.write_u64(v[0].to_bits()); hasher.write_u64(v[1].to_bits()); hasher.write_u64(v[2].to_bits()); }
        hasher.write(self.file_url.as_bytes());
        hasher.write_u64(self.duration_ms.to_bits());
        hasher.write_u64(self.imu_transforms.imu_lpf.to_bits());
        hasher.write_usize(self.raw_imu.len());
        hasher.write_usize(file_metadata.raw_imu.len());
        hasher.write_usize(self.quaternions.len());
        hasher.write_usize(file_metadata.quaternions.len());
        hasher.write_usize(file_metadata.image_orientations.as_ref().map(|v| v.len()).unwrap_or_default());
        hasher.write_usize(file_metadata.lens_positions.len());
        hasher.write_usize(file_metadata.lens_params.len());
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

        let file_metadata = self.file_metadata.read();

        for x in &file_metadata.raw_imu {
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
