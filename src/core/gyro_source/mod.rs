// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

mod file_metadata;
mod imu_transforms;
mod sony;
mod canon;
mod motioncam;
mod per_frame;
pub mod splines;
pub mod optical_correction;
pub use file_metadata::*;
pub use optical_correction::{ OpticalCorrection, OpticalCorrectionSettings, TrackingMethod };
pub use imu_transforms::*;
pub use sony::{ interpolate_mesh, MESH_REFINE_SKIP_PX, MESH_REFINE_THRESHOLD_PX };

use nalgebra::*;
use std::iter::zip;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::{ Arc, atomic::AtomicBool };
use parking_lot::RwLock;
use telemetry_parser::{ Input, util, InputOptions, TagFilter };
use telemetry_parser::tags_impl::{ GetWithType, GroupId, GroupedTagMap, TagId, TimeQuaternion, TimeVector3 };
use std::io::{ Read, Seek };

use crate::camera_identifier::CameraIdentifier;
use crate::stabilization_params::ReadoutDirection;

use super::imu_integration::*;
use super::smoothing::SmoothingAlgorithm;
use crate::StabilizationParams;

const DEG2RAD: f64 = std::f64::consts::PI / 180.0;

pub type Quat64 = UnitQuaternion<f64>;
pub type TimeIMU = telemetry_parser::util::IMUData;
pub type TimeQuat = BTreeMap<i64, Quat64>; // key is timestamp_us
pub type TimeVec = BTreeMap<i64, Vector3<f64>>; // key is timestamp_us

#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileLoadOptions {
    pub sample_index: Option<usize>,
    pub project_version: u64,
    /// Frames of the video the motion data is loaded for, when they're known. The per-frame tables are by its frames,
    /// and its frame count bounds the frame numbers the metadata gives them (`per_frame::frame_limit`). The video's,
    /// not an option of the file, so it isn't stored
    #[serde(skip)]
    pub video_frames: Option<usize>,
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

    pub file_url: String,
    /// The file was loaded with all of its metadata, as the video's own (it's the video, or "Load all metadata" was
    /// on): its per-frame tables (the frame timing) apply to the video's frames
    #[serde(default)]
    pub all_metadata: bool,

    /// Correction measured from the video, composed onto `quaternions` at the end of `integrate`
    #[serde(default)]
    pub optical_correction: Option<OpticalCorrection>,
    /// Whether the last `integrate` applied it: an enabled one that doesn't match the quaternions is stale
    #[serde(skip)]
    pub optical_correction_applied: bool,
    /// `optical_correction::checksum` of the quaternions the last `integrate` composed it onto, or would have
    #[serde(skip)]
    pub optical_uncorrected_checksum: u64,
    /// The context the correction applies in: what `optical_motion::context_checksum` makes of the current sync, lens
    /// and frame timing. Kept up to date by `StabilizationManager::refresh_optical_correction`
    #[serde(skip)]
    pub optical_context: u64,
    /// The file's own motion data and sync points, set aside while the motion comes from the video instead, see
    /// `set_ignore_file_motion`
    #[serde(skip)]
    ignored_motion: Option<Arc<(FileMotion, BTreeMap<i64, f64>)>>,
}

impl GyroSource {
    pub fn new() -> Self {
        Self {
            integration_method: 2, // VQF
            use_gravity_vectors: false,
            horizon_lock_integration_method: 1, // VQF
            ..Default::default()
        }
    }

    pub fn has_motion(&self) -> bool {
        self.file_metadata.read().has_motion()
    }

    /// Sets the file's own motion data aside, and its sync points with it, so the motion comes from the video instead:
    /// "Analyze image optically" then measures all of it, as for a file without any. For motion data too broken to
    /// correct. Meanwhile everything reading motion data sees a file without any; the lens, the frame timing and the
    /// rest of the metadata stay. False brings them back. Returns whether anything changed
    pub fn set_ignore_file_motion(&mut self, ignore: bool) -> bool {
        if ignore == self.ignored_motion.is_some() { return false; }
        if ignore {
            let motion = self.file_metadata.take_motion();
            let offsets = std::mem::take(&mut self.offsets);
            self.clear_offsets();
            self.ignored_motion = Some(Arc::new((motion, offsets)));
        } else if let Some(ignored) = self.ignored_motion.take() {
            let (motion, offsets) = Arc::unwrap_or_clone(ignored);
            self.file_metadata.restore_motion(motion);
            self.set_offsets(offsets);
        }
        self.apply_transforms();
        true
    }
    pub fn ignores_file_motion(&self) -> bool {
        self.ignored_motion.is_some()
    }
    /// The sync points that belong to the file's motion data, also while it's set aside
    pub fn file_offsets(&self) -> &BTreeMap<i64, f64> {
        self.ignored_motion.as_ref().map(|x| &x.1).unwrap_or(&self.offsets)
    }
    /// The file's metadata with its motion data, when that's set aside (None otherwise: `file_metadata` has it)
    pub fn file_metadata_with_ignored_motion(&self) -> Option<FileMetadata> {
        self.ignored_motion.as_ref().map(|x| self.file_metadata.with_motion(&x.0))
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

    pub fn parse_telemetry_file<T: Read + Seek, P: AsRef<std::path::Path>, F: Fn(f64)>(stream: &mut T, filesize: usize, path: P, options: &FileLoadOptions, size: (usize, usize), fps: f64, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> Result<FileMetadata, crate::GyroflowCoreError> {
        let key = format!("{}{options:?}{size:?}{fps}", path.as_ref().display());
        static CACHE: RwLock<BTreeMap<String, FileMetadata>> = RwLock::new(BTreeMap::new());
        {
            let cache = CACHE.read();
            if let Some(md) = cache.get(&key) {
                return Ok(md.clone());
            }
        }

        let tpoptions = InputOptions {
            blackbox_gyro_only: true,
            tag_blacklist: [
                TagFilter::EntireGroup(GroupId::UnknownGroup(0xf000)),
                TagFilter::EntireGroup(GroupId::UnknownGroup(0x0)),
                // Not used here, and they are most of a MotionCam clip's metadata
                TagFilter::EntireGroup(GroupId::Custom("FrameMetadata".into())),
                TagFilter::SpecificTag(GroupId::Lens, TagId::Shading),
            ].into(),
            ..Default::default()
        };
        let mut input = Input::from_stream_with_options(stream, filesize, &path, progress_cb, cancel_flag, tpoptions)?;

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
        let mut frame_readout_direction_tag: Option<i32> = None;
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

        let is_gyroflow_proto = input.parser_name() == "GyroflowProtobuf";
        // How the video shows the frame the file states its pixel quantities in, for the clip's readout time
        let mut video_mapping = None;

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
                    let frame_mapping = FrameMapping::of(tag_map, size);
                    if let FrameMapping::Mapped { .. } = frame_mapping {
                        video_mapping.get_or_insert(frame_mapping);
                    }
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
                        if let Some(v) = im.get_t(TagId::FrameReadoutDirection) as Option<&i32> { frame_readout_direction_tag = Some(*v); }
                        if let Some(v) = im.get_t(TagId::PixelPitch) as Option<&(u32, u32)> { lens_info.pixel_pitch = Some(*v); }
                        // The sensor area the video shows: the frame's, unless the video is the file's other picture
                        let capture_origin = (im.get_t(TagId::CaptureAreaOrigin) as Option<&(f32, f32)>).copied();
                        if let Some(v) = im.get_t(TagId::CaptureAreaSize) as Option<&(f32, f32)> {
                            let (origin, area) = frame_mapping.capture_area(capture_origin.unwrap_or_default(), *v);
                            lens_info.capture_area_size = Some(area);
                            if capture_origin.is_some() { lens_info.capture_area_origin = Some(origin); }
                            // The Gyroflow protobuf's readout time is that of the rows it shows (gyroflow.proto, and the
                            // clip's readout time is turned into the video's rows' below), not the sensor's
                            if is_gyroflow_proto { lens_info.readout_span_rows = Some(area.1); }
                        } else if let Some(v) = capture_origin {
                            lens_info.capture_area_origin = Some(v);
                        }
                        if let Some(v) = im.get_t(TagId::SensorSizePixels) as Option<&(u32, u32)> { lens_info.sensor_size_px = Some(*v); }
                        if let Some(w) = im.get_t(TagId::PixelWidth) as Option<&u32> {
                            if let Some(h) = im.get_t(TagId::PixelHeight) as Option<&u32> {
                                lens_info.capture_area_origin = Some((0.0, 0.0));
                                lens_info.sensor_size_px = Some((*w, *h));
                                lens_info.capture_area_size = Some((*w as f32, *h as f32));

                                if let Some(def) = tag_map.get(&GroupId::Default) {
                                    if let Some(sw) = def.get_t(TagId::SensorWidth) as Option<&f32> {
                                        if let Some(sh) = def.get_t(TagId::SensorHeight) as Option<&f32> {
                                            lens_info.pixel_pitch = Some(((*sw * 1000000.0 / *w as f32).round() as u32, (*sh * 1000000.0 / *h as f32).round() as u32));
                                        }
                                    }
                                }
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
                        if let Some(v) = map.get_t(TagId::FocalLength) as Option<&f32> {
                            lens_positions.insert(timestamp_us, *v as f64);
                            lens_info.focal_length = Some(*v);
                        }
                        if let Some(v) = map.get_t(TagId::FocusDistance) as Option<&f32> {
                            lens_info.focus_distance = Some(*v);
                        }
                        if let Some(v) = map.get_t(TagId::IrisFStop) as Option<&f32> {
                            lens_info.iris_fstop = Some(*v);
                        }
                        if let Some(v) = map.get_t(TagId::IrisTStop) as Option<&f32> {
                            lens_info.iris_tstop = Some(*v);
                        }
                        if let Some(v) = map.get_t(TagId::ZoomRingPosition) as Option<&f32> {
                            lens_info.zoom_ring_position = Some(*v);
                        }
                        // The frame's pixel quantities in the video's pixels (`FrameMapping`)
                        let (pfl_scale, pfl_valid) = (frame_mapping.scale() as f32, frame_mapping != FrameMapping::Unusable);
                        if !pfl_valid {
                            lens_info.pixel_focal_length = None;
                        }
                        if let Some(v) = map.get_t(TagId::PixelFocalLength) as Option<&(f32, f32)> {
                            if pfl_valid { lens_info.pixel_focal_length = Some((v.0 * pfl_scale, v.1 * pfl_scale)); }
                        }
                        if let Some(v) = map.get_t(TagId::PixelFocalLength) as Option<&f32> {
                            if pfl_valid { lens_info.pixel_focal_length = Some((*v * pfl_scale, *v * pfl_scale)); }
                        }
                        if let Some(v) = map.get_t(TagId::PixelFocalLength) as Option<&Vec<f32>> {
                            match v.as_slice() {
                                [fx]         if pfl_valid => lens_info.pixel_focal_length = Some((*fx * pfl_scale, *fx * pfl_scale)),
                                [fx, fy, ..] if pfl_valid => lens_info.pixel_focal_length = Some((*fx * pfl_scale, *fy * pfl_scale)),
                                _ => {}
                            }
                        }
                        if let Some(v) = map.get_t(TagId::PrincipalPoint) as Option<&(f32, f32)> {
                            lens_info.principal_point = Some(frame_mapping.point(*v));
                        } else if pfl_valid && map.contains_key(&TagId::PixelFocalLength) {
                            // Left out where it's the frame's centre, which isn't the video's when the video is the other picture
                            if let Some(v) = frame_mapping.default_principal_point() { lens_info.principal_point = Some(v); }
                        }
                        // Per-frame distortion coefficients for gyroflow.proto GenericPolynomial
                        if let Some(v) = map.get_t(TagId::DistortionCoefficients) as Option<&Vec<f64>> {
                            if !v.is_empty() {
                                lens_info.distortion_coefficients = v.clone();
                            }
                        }
                    }
                    if lens_info.focal_length.is_none() {
                        if let Some(md) = tag_map.get(&GroupId::Custom("LensDistortion".into())) {
                            if let Some(v) = md.get_t(TagId::Data) as Option<&serde_json::Value> {
                                // lens.focal_length = v.get("focal_length_nm").and_then(|x| x.as_f64()).map(|x| (x / 1000000.0) as f32);
                                let focal_length_nm = v.get("focal_length_nm").and_then(|x| x.as_f64()).unwrap_or_default();
                                let effective_sensor_height_nm = v.get("effective_sensor_height_nm").and_then(|x| x.as_f64()).unwrap_or(1.0);

                                let pfl = ((focal_length_nm as f64 / effective_sensor_height_nm as f64) * size.1 as f64) as f32;
                                lens_info.pixel_focal_length = Some((pfl, pfl));
                            }
                        }
                    }
                    if lens_info.has_geometry() || lens_info.has_descriptive_data() {
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

        let mut raw_imu = util::normalized_imu_interpolated(&input, Some("XYZ".into())).unwrap_or_default();

        // Sony: the metadata packets carry the gyro timing (offset and rate per frame), use it instead of a uniform spacing
        let sony_packet_timed = input.camera_type() == "Sony" && input.samples.as_ref().map(|s| sony::retime_imu_from_packets(&mut raw_imu, s)).unwrap_or(false);
        if sony_packet_timed { log::debug!("Sony gyro samples re-timed from the metadata packets"); }

        if (input.camera_type() == "RED" || input.camera_type() == "RED RAW") && options.project_version > 0 && options.project_version < 4 { // Legacy gyro offset
            let mut first_timestamp = None;
            log::debug!("Legacy project, removing new RED gyro offset");
            for x in raw_imu.iter_mut() {
                if first_timestamp.is_none() {
                    first_timestamp = Some(x.timestamp_ms);
                }
                x.timestamp_ms -= first_timestamp.unwrap();
            }
        }
        let mut has_accurate_timestamps = input.has_accurate_timestamps();
        if let serde_json::Value::Object(o) = &mut additional_data {
            match o.get("has_accurate_timestamps") {
                Some(serde_json::Value::String(x)) => { if x == "true" || x == "1" { has_accurate_timestamps = true; } }
                Some(serde_json::Value::Bool(x)) => { if *x { has_accurate_timestamps = true; } }
                _ => {}
            }
            o.remove("has_accurate_timestamps");
        }

        let fr = input.frame_readout_time().unwrap_or_default();
        let frame_readout_time = if fr != 0.0 { Some(if fr.abs() > 10000.0 { fr.abs() - 10000.0 } else { fr.abs() }) } else { None };
        let frame_readout_direction = match frame_readout_direction_tag {
            Some(v) => ReadoutDirection::from(v),
            None => if fr < 0.0 {
                if fr.abs() > 10000.0 { ReadoutDirection::RightToLeft } else { ReadoutDirection::BottomToTop }
            } else {
                if fr.abs() > 10000.0 { ReadoutDirection::LeftToRight } else { ReadoutDirection::TopToBottom }
            },
        };
        // The Gyroflow protobuf's readout time is its frame's rows': a video showing more or fewer of them (the file's
        // other picture) takes their time
        let frame_readout_time = match video_mapping {
            Some(m) if is_gyroflow_proto => frame_readout_time.map(|x| x * m.readout_extent(frame_readout_direction)),
            _ => frame_readout_time,
        };
        // Canon's cameras that don't write their readout time: corrected, and timed (`canon::get_time_offset`), with a typical one
        let frame_readout_time = frame_readout_time.or((input.camera_type() == "Canon").then_some(canon::DEFAULT_READOUT_MS));

        let mut md = FileMetadata {
            imu_orientation,
            detected_source: Some(detected_source),
            quaternions,
            image_orientations,
            gravity_vectors,
            lens_positions,
            lens_params,
            raw_imu,
            frame_readout_time,
            frame_readout_direction,
            frame_rate,
            lens_profile,
            camera_identifier,
            has_accurate_timestamps,
            additional_data,
            per_frame_time_offsets: Vec::new(),
            digital_zoom,
            camera_stab_data: Vec::new(),
            mesh_correction:  Default::default(),
            legacy_mesh_correction: Vec::new(),
            lens_breathing:   Vec::new(),
            focal_length_varies_cache: Default::default(),
        };

        let sample_rate = Self::get_sample_rate(&md);
        let mut original_sample_rate = sample_rate;
        let mut is_temp = sony::ISTemp::default();
        let mut mesh_cache = BTreeMap::new(); // mesh key -> table index, so the frames with the same mesh share one
        let is_red = input.camera_type() == "RED" || input.camera_type() == "RED RAW";
        let legacy_red_offset = options.project_version > 0 && options.project_version < 4;
        let (mut red_has_frames, mut red_clip_exposure) = (false, None);
        if let Some(ref samples) = input.samples {
            // The frame each sample describes (`frame_of_sample`), among the frames the clip can have: one numbered
            // past them is a broken one, which would size every table (`per_frame::frame_limit`)
            let mut sample_frames: Vec<Option<usize>> = samples.iter().map(|info| frame_of_sample(&input, info)).collect();
            per_frame::limit_frames(options.video_frames, &mut sample_frames);
            // The per-frame tables are put together by the frame each sample describes, never in the order the samples
            // come: a frame without a sample, or with a sample that lacks the data, stays empty until all of them are
            // in, and every other frame keeps its own entry (`per_frame::PerFrame`)
            let mut time_offsets = per_frame::PerFrame::<f64>::new("Per-frame time offsets");
            let mut mesh_frames = per_frame::PerFrame::<MeshFrame>::new("Mesh correction");
            for (info, &frame) in samples.iter().zip(&sample_frames) {
                if let Some(ref tag_map) = info.tag_map {
                    // --------------------------------- Timing -------------------------------
                    // One source for all frames: the format's own timing, else the camera's
                    if let Some(frame) = frame {
                        let offset = if is_gyroflow_proto {
                            gyroflow_proto_time_offset(tag_map, &FrameMapping::of(tag_map, size), frame_readout_direction)
                        } else if let Some((org_sample_rate, offset)) = sony::get_time_offset(&md, &input, tag_map, sample_rate, sony_packet_timed) {
                            original_sample_rate = org_sample_rate;
                            Some(offset)
                        } else if is_red {
                            red_has_frames = true;
                            // The middle of the frame's exposure
                            red_exposure_ms(tag_map).filter(|_| !legacy_red_offset).map(|exposure| -exposure / 2.0)
                        } else if input.camera_type() == "Canon" {
                            canon::get_time_offset(&md, &input, tag_map, sample_rate, fps)
                        } else if input.camera_type() == "MotionCam" {
                            motioncam::get_time_offset(&md, tag_map, frame, fps)
                        } else {
                            None
                        };
                        if let Some(offset) = offset {
                            time_offsets.set(frame, offset);
                        }
                    } else if is_red {
                        // The clip's metadata (the RED2 header), which the RED SDK defines as its first frame's
                        red_clip_exposure = red_exposure_ms(tag_map);
                    }
                    sony::init_lens_profile(&mut md, &input, tag_map, size, info);
                    if let Some(frame) = frame {
                        sony::stab_collect(&mut is_temp, tag_map, frame, fps);
                        // The correction is looked up by frame (`MeshCorrections::frame`): a frame the camera wrote no
                        // correction for has an empty one, a frame without the data is filled in below
                        if let Some(mesh_frame) = sony::get_mesh_correction(tag_map, size, &mut md.mesh_correction, &mut mesh_cache) {
                            mesh_frames.set(frame, mesh_frame);
                        }
                    }

                    if let Some(ois) = tag_map.get(&GroupId::LensOSS).and_then(|x| x.get_t(TagId::Data) as Option<&Vec<TimeVector3<i32>>>) {
                        if ois.len() == 1 && *ois.first().unwrap() == (TimeVector3 { t: -1, x: -1, y: -1, z: -1 }) {
                            if let serde_json::Value::Object(o) = &mut md.additional_data {
                                o.insert("unsupported_lens".into(), true.into());
                            }
                        }
                    }

                    // --------------------------------- Sony ---------------------------------

                    // --------------------------------- Sony ---------------------------------

                    // --------------------------------- Canon ---------------------------------
                    if input.camera_type() == "Canon" {
                        canon::init_lens_profile(&mut md, &input, tag_map, size, info);
                        if canon::lens_stabilizer_on(tag_map) {
                            if let serde_json::Value::Object(o) = &mut md.additional_data {
                                o.insert("lens_stabilizer_on".into(), true.into());
                            }
                        }
                    }
                    // --------------------------------- Canon ---------------------------------

                    // --------------------------------- MotionCam ---------------------------------
                    if input.camera_type() == "MotionCam" {
                        motioncam::init_lens_profile(&mut md, &input, tag_map, size, info);
                    }
                    // --------------------------------- MotionCam ---------------------------------

                    // --------------------------------- Insta360 ---------------------------------
                    // Timing
                    if input.camera_type() == "Insta360" {
                        telemetry_parser::try_block!({
                            use telemetry_parser::tags_impl::TimeScalar;
                            let exp = (tag_map.get(&GroupId::Exposure)?.get_t(TagId::Data) as Option<&Vec<TimeScalar<f64>>>)?;
                            let tm = (tag_map.get(&GroupId::Default)?.get_t(TagId::Custom("TimeMap".into())) as Option<&Vec<TimeScalar<f64>>>).cloned().unwrap_or_default();

                            let mut video_ts = 0.0;
                            let mut zero_ref = None;
                            let mut prev_t = 0.0;
                            let mut i = 0;
                            for x in exp {
                                if x.t > prev_t || x.t == 0.0 {
                                    if zero_ref.is_none() {
                                        zero_ref = Some(x.t * 1000.0);
                                        log::debug!("Insta360 first frame reference time: {:.4}", x.t * 1000.0);
                                    }
                                    let tm_diff = tm.get(i).map(|tm| tm.t - tm.v).unwrap_or_default();

                                    // The additional 0.9 ms is a mystery
                                    let diff = (video_ts - x.t) * 1000.0;

                                    // Record `i` is video frame `i`: the records are timed from the first frame on
                                    time_offsets.set(i, -(x.v * 1000.0 / 2.0) - 0.9 - diff - tm_diff - zero_ref.unwrap());

                                    video_ts += 1.0 / fps;
                                    prev_t = x.t;
                                    i += 1;
                                }
                            }
                        });
                    }
                    // --------------------------------- Insta360 ---------------------------------
                }
            }
            if let Some(exposure) = red_clip_exposure.filter(|_| red_has_frames && !legacy_red_offset && !time_offsets.has(0)) {
                // The clip's metadata is the first frame's, so the first frame's exposure is known without its own
                time_offsets.set(0, -exposure / 2.0);
            }
            // A frame without an offset of its own: the readers need one for every frame, and the neighbours' is the
            // offset the frame continues, where a zero would move it by the whole offset against them
            time_offsets.report();
            md.per_frame_time_offsets = time_offsets.interpolate();
            if input.camera_type() == "Sony" {
                md.lens_breathing = sony::breathing::compute(samples, &sample_frames);
            }
            // Only a clip with corrections has the table (and misses any): without one on any frame, it would be all
            // empty frames
            if mesh_frames.values().any(|m| !m.is_empty()) {
                mesh_frames.report();
                md.mesh_correction.frames = mesh_frames.hold();
            }
            if md.mesh_correction.is_empty() {
                md.mesh_correction.clear();
            } else {
                let tables = &md.mesh_correction.tables;
                log::debug!("Mesh correction: {} tables over {} frames, {} refined against the camera's mesh", tables.len(), md.mesh_correction.frames.len(), tables.iter().filter(|t| !t.refinement.is_empty()).count());
            }
            if !is_temp.ibis.t.is_empty() || !is_temp.ois.t.is_empty() {
                // Computed for the frames from the first collected one on (a frame the metadata misses among them across
                // its gap), the frames before it take that one's: laid out by frame as it's computed
                md.camera_stab_data = sony::stab_calc_splines(&md, &is_temp, sample_rate, fps, size).unwrap_or_default();
                if !sony_packet_timed {
                    // Uniform gyro timeline: rescale the metadata readout to the measured sample rate, like the per-frame offsets
                    if let Some(frt) = md.frame_readout_time {
                        md.frame_readout_time = Some(frt / original_sample_rate * sample_rate);
                    }
                }
            }
        }

        #[cfg(feature = "cache-gyro-metadata")]
        {
            let mut cache = CACHE.write();
            cache.insert(key, md.clone());
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
        self.imu_transforms.imu_mf = 0;
        self.file_metadata = Default::default();
        self.optical_correction = None;
        self.ignored_motion = None;
        self.clear_offsets();
    }

    pub fn load_from_telemetry(&mut self, mut telemetry: FileMetadata) {
        if self.duration_ms <= 0.0 {
            ::log::error!("Invalid duration_ms {}", self.duration_ms);
            return;
        }

        self.clear();
        sony::upgrade_legacy_metadata(&mut telemetry);

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
        drop(file_metadata);
        self.apply_optical_correction();
    }

    /// Composes the correction measured from the video onto freshly integrated quaternions - when it was measured
    /// against these very ones, in this context, see `OpticalCorrection`
    fn apply_optical_correction(&mut self) {
        self.optical_correction_applied = false;
        if let Some(c) = &self.optical_correction {
            let from_video = self.quaternions.len() < 2 && !c.video_base.is_empty();
            if from_video {
                // A file without motion data: what the analysis measured between the frames is all there is
                self.quaternions = c.base_quats();
            }
            self.optical_uncorrected_checksum = optical_correction::checksum(&self.quaternions);
            if self.optical_correction_applies() {
                c.apply(&mut self.quaternions);
                self.optical_correction_applied = true;
            } else if c.enabled {
                log::warn!("The optical correction was measured on other motion data (another integration method or filter), sync, lens or frame timing, not applying it");
            }
            if from_video {
                optical_correction::hold_over_clip(&mut self.quaternions, self.duration_ms);
            }
        }
    }
    /// Whether `integrate` composes the correction onto the quaternions, as things stand
    pub fn optical_correction_applies(&self) -> bool {
        self.optical_correction.as_ref().is_some_and(|c| c.enabled && c.measured_on(self.optical_uncorrected_checksum, self.optical_context))
    }

    pub fn recompute_smoothness(&self, alg: &dyn SmoothingAlgorithm, horizon_lock: super::smoothing::horizon::HorizonLock, compute_params: &crate::ComputeParams) -> (TimeQuat, (f64, f64, f64)) {
        let file_metadata = self.file_metadata.read();
        let mut smoothed_quaternions = self.quaternions.clone();

        for (ts, q) in smoothed_quaternions.iter_mut() {
            use crate::KeyframeType;
            let timestamp_ms = *ts as f64 / 1000.0;
            let additional_rotation_x = compute_params.keyframes.value_at_gyro_timestamp(&KeyframeType::AdditionalRotationX, timestamp_ms).unwrap_or(compute_params.additional_rotation.0) * DEG2RAD;
            let additional_rotation_y = compute_params.keyframes.value_at_gyro_timestamp(&KeyframeType::AdditionalRotationY, timestamp_ms).unwrap_or(compute_params.additional_rotation.1) * DEG2RAD;
            let additional_rotation_z = compute_params.keyframes.value_at_gyro_timestamp(&KeyframeType::AdditionalRotationZ, timestamp_ms).unwrap_or(compute_params.additional_rotation.2) * DEG2RAD;
            let additional_rotation   = Quat64::from_euler_angles(additional_rotation_y, additional_rotation_x, additional_rotation_z);

            *q *= additional_rotation;
        }

        if true {
            // Lock horizon, then smooth
            horizon_lock.lock(&mut smoothed_quaternions, &self.quaternions, &file_metadata.gravity_vectors, self.use_gravity_vectors, self.integration_method, compute_params);
            let locked_quaternions = smoothed_quaternions;
            smoothed_quaternions = alg.smooth_with_original(&locked_quaternions, &self.quaternions, self.duration_ms, compute_params);

            // "Only within trim range" replaces the motion outside of the trim ranges before smoothing, so it doesn't affect
            // the smoothing inside of them. The result outside of the ranges has nothing to do with the real motion then,
            // and the preview there looks like the shaky original, drifting. Those frames are not rendered, so use the
            // smoothing of the whole video for them
            if !compute_params.trim_ranges.is_empty() && alg.get_parameter("trim_range_only") > 0.5 {
                let mut whole_video = compute_params.clone();
                whole_video.trim_ranges.clear();
                let whole_smoothed = alg.smooth_with_original(&locked_quaternions, &self.quaternions, self.duration_ms, &whole_video);
                let duration_us = compute_params.scaled_duration_ms * 1000.0;
                let ranges = compute_params.trim_ranges.iter().map(|x| ((x.0 * duration_us).round() as i64, (x.1 * duration_us).round() as i64)).collect::<Vec<_>>();
                for (ts, q) in smoothed_quaternions.iter_mut() {
                    if !ranges.iter().any(|r| *ts >= r.0 && *ts <= r.1) {
                        if let Some(whole) = whole_smoothed.get(ts) { *q = *whole; }
                    }
                }
            }
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
                    self.imu_transforms.transform_gyro(g);
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
            if self.imu_transforms.imu_mf > 0 && !file_metadata.raw_imu.is_empty() && self.duration_ms > 0.0 {
                let sample_rate = file_metadata.raw_imu.len() as f64 / (self.duration_ms / 1000.0);
                super::filtering::Median::filter_gyro_forward_backward(self.imu_transforms.imu_mf, sample_rate, &mut self.raw_imu);
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
        hasher.write_i32(self.imu_transforms.imu_mf);
        hasher.write_usize(self.raw_imu.len());
        hasher.write_usize(file_metadata.raw_imu.len());
        hasher.write_usize(self.quaternions.len());
        hasher.write_usize(file_metadata.quaternions.len());
        hasher.write_usize(file_metadata.image_orientations.as_ref().map(|v| v.len()).unwrap_or_default());
        hasher.write_usize(file_metadata.lens_positions.len());
        hasher.write_usize(file_metadata.lens_params.len());
        hasher.write_u32(if self.use_gravity_vectors { 1 } else { 0 });
        hasher.write_usize(self.integration_method);
        if let Some(c) = &self.optical_correction { c.hash_into(&mut hasher); }
        // Switched on or off by the context alone (`refresh_optical_correction`), the correction is the same one, and
        // where it isn't measured (the ends of the clip, which the rest of this looks at) the quaternions are too
        hasher.write_u8(self.optical_correction_applied as u8);
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

    /// Readings per second of the motion data: the IMU's, else the quaternions' (`readings_per_second`)
    pub fn get_sample_rate(file_metadata: &FileMetadata) -> f64 {
        if file_metadata.raw_imu.len() > 2 {
            Self::readings_per_second(file_metadata.raw_imu.iter().map(|x| x.timestamp_ms))
        } else if file_metadata.quaternions.len() > 2 {
            Self::readings_per_second(file_metadata.quaternions.keys().map(|ts| *ts as f64 / 1000.0))
        } else {
            0.0
        }
    }

    /// Readings per second of a timeline of more than two readings, `times` in ms, in order: their count over their
    /// span. An evenly spaced timeline (telemetry-parser's, of the cameras whose readings it spaces by their sample's
    /// duration) keeps the time of the frames the metadata misses while their readings aren't there, and counted over
    /// that time too, the rate would come out low by the share of time the gaps take: Canon's frame offsets (half a
    /// reading's interval) and the metadata times rescaled to Sony's uniform timeline would move with it. Such a timeline
    /// tells its gaps apart: every step is its interval (the median step), to the microsecond its readings are timed at,
    /// or longer than one and a half intervals, nearer two of them than one, with a reading or more missing in it. Its
    /// rate is then the one over the other steps, as the readings would take one after another. A timeline whose steps
    /// vary otherwise (the readings' own times, which jitter) can't tell a gap from a late reading, and keeps the rate
    /// over its span
    fn readings_per_second(times: impl Iterator<Item = f64> + Clone) -> f64 {
        let steps = || times.clone().zip(times.clone().skip(1)).map(|(a, b)| b - a);
        let len = times.clone().count() as f64;
        let mut duration_ms = times.clone().last().unwrap_or_default() - times.clone().next().unwrap_or_default();
        // The median is at least the shortest step, so a timeline whose longest step isn't one and a half of its
        // shortest has no gap, and keeps the rate over its whole span
        let (shortest, longest) = steps().fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), step| (min.min(step), max.max(step)));
        if longest > shortest * 1.5 {
            let mut sorted: Vec<f64> = steps().collect();
            let middle = sorted.len() / 2;
            let interval = *sorted.select_nth_unstable_by(middle, f64::total_cmp).1;
            let gap = |step: f64| step > interval * 1.5;
            // The readings telemetry-parser spaces evenly are timed in whole microseconds (`normalized_imu_interpolated`)
            let even = |step: f64| ((step - interval) * 1000.0).abs().round() <= 1.0;
            if interval > 0.0 && steps().all(|step| gap(step) || even(step)) {
                let (gaps, gap_ms) = steps().filter(|step| gap(*step)).fold((0.0, 0.0), |(count, ms), step| (count + 1.0, ms + step));
                if gaps > 0.0 {
                    duration_ms = (duration_ms - gap_ms) * (len - 1.0) / (len - 1.0 - gaps);
                }
            }
        }
        let duration_ms = duration_ms * ((len + 1.0) / len.max(1.0));
        len / (duration_ms / 1000.0)
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

/// The video frame a sample of per-frame metadata describes, which its entry in the per-frame tables belongs to. The
/// parsers that write a sample per frame number the samples by their frames (`SampleInfo::sample_index`), so a frame
/// they have no sample of leaves a gap rather than moving the frames after it: Sony, Canon and the other metadata
/// tracks by the track's sample, MXF by the frame's ancillary data, RED by the frame atom, MotionCam by the decoded
/// frame and the Gyroflow protobuf by `frame_number` (its time on the video). `None` for a sample of the clip's
/// metadata, which RED puts ahead of its frames: those carry a frame timestamp
fn frame_of_sample(input: &Input, info: &util::SampleInfo) -> Option<usize> {
    if input.parser_name() == "RedR3d" && !info.tag_map.as_ref().and_then(|x| x.get(&GroupId::Default)).is_some_and(|x| x.contains_key(&TagId::TimestampUs)) {
        return None;
    }
    usize::try_from(info.sample_index).ok()
}

/// Offset of a frame's capture from its time in the video, by the gyroflow.proto contract: the frame's start on the
/// camera clock (`FirstFrameTimestamp`, against the video time) to the middle of its exposure and readout. The middle
/// of the rows the video shows, which is the frame's unless the video is the file's other picture (`FrameMapping`)
fn gyroflow_proto_time_offset(tag_map: &GroupedTagMap, mapping: &FrameMapping, direction: ReadoutDirection) -> Option<f64> {
    let imager = tag_map.get(&GroupId::Imager)?;
    let first_frame_ts = *(imager.get_t(TagId::FirstFrameTimestamp) as Option<&f64>)?;
    let exposure_time = *(imager.get_t(TagId::ExposureTime)     as Option<&f64>).unwrap_or(&0.0);
    let readout_time  = *(imager.get_t(TagId::FrameReadoutTime) as Option<&f64>).unwrap_or(&0.0);

    Some(first_frame_ts - exposure_time / 2.0 + readout_time * (0.5 + mapping.centre_shift(direction)))
}

/// How the video shows the frame a file states its per-frame pixel quantities in (the focal length and the principal
/// point in pixels, and where that frame is on the sensor): frame pixel `p` is video pixel `(p + origin) · scale`, in
/// corner coordinates. The video is the decoder's or the host's and the frame the file's, and they needn't agree: a
/// proxy or a host's own resolution scales the frame, and the frames of some files have two pictures, a decoder showing
/// either (a DNG's default crop, which Resolve applies and FFmpeg leaves out; the file names the other picture,
/// `AlternativeFrameSize`, and where the frame is in it, `FrameOriginInAlternative`)
#[derive(Clone, Copy, Debug, PartialEq)]
enum FrameMapping {
    /// The file doesn't say which frame its quantities are in, the video is that frame, or the video is none of the
    /// file's pictures at their aspect ratio: the quantities are taken as they are, the video's
    AsIs,
    Mapped {
        scale: f64,
        origin: (f64, f64),
        /// The frame's size, and how much of it the video shows (from `-origin`), in frame pixels
        frame: (f64, f64),
        shown: (f64, f64),
        /// The file leaves the principal point out where it's the frame's centre (the Gyroflow protobuf)
        centred_by_default: bool,
    },
    /// The video isn't the frame at its aspect ratio, and the quantities don't apply (Canon's recorded frame)
    Unusable,
}

impl FrameMapping {
    /// How far off its aspect ratio a scaled copy of a frame can be, from rounding the scaled size
    const ASPECT_TOLERANCE: f64 = 0.02;

    fn of(tag_map: &GroupedTagMap, video: (usize, usize)) -> Self {
        let (vw, vh) = (video.0 as f64, video.1 as f64);
        if !(vw > 0.0 && vh > 0.0) { return Self::AsIs; }
        let imager = tag_map.get(&GroupId::Imager);
        let size_tag = |name: &'static str| imager.and_then(|x| x.get_t(TagId::Custom(name.into())) as Option<&(u32, u32)>)
            .filter(|x| x.0 > 0 && x.1 > 0).map(|x| (x.0 as f64, x.1 as f64));
        let aspect_error = |(w, h): (f64, f64)| ((w / h) / (vw / vh) - 1.0).abs();
        let mapped = |frame: (f64, f64), picture: (f64, f64), origin: (f64, f64), centred_by_default: bool| {
            if picture == (vw, vh) && picture == frame { return Self::AsIs; }
            let scale = vw / picture.0;
            Self::Mapped { scale, origin, frame, shown: (vw / scale, vh / scale), centred_by_default }
        };
        if let Some(frame) = size_tag("FrameSize") {
            // The pictures the video can be: the frame, and the file's other one, with the frame's place in it
            let mut pictures = vec![(frame, (0.0, 0.0))];
            let other_origin = imager.and_then(|x| x.get_t(TagId::Custom("FrameOriginInAlternative".into())) as Option<&(f32, f32)>);
            if let (Some(other), Some(origin)) = (size_tag("AlternativeFrameSize"), other_origin) {
                pictures.push((other, (origin.0 as f64, origin.1 as f64)));
            }
            // The one the video is, else the one closest to its aspect ratio (the frame on a tie)
            let exact = pictures.iter().find(|(picture, _)| *picture == (vw, vh));
            let closest = pictures.iter().min_by(|a, b| aspect_error(a.0).total_cmp(&aspect_error(b.0)))
                .filter(|(picture, _)| aspect_error(*picture) < Self::ASPECT_TOLERANCE);
            match exact.or(closest) {
                Some(&(picture, origin)) => mapped(frame, picture, origin, true),
                None => Self::AsIs,
            }
        } else if let Some(frame) = canon::recorded_frame_size(tag_map).filter(|x| x.0 > 0 && x.1 > 0).map(|x| (x.0 as f64, x.1 as f64)) {
            // Canon's recorded frame: shown scaled, or not at all
            if aspect_error(frame) < Self::ASPECT_TOLERANCE { mapped(frame, frame, (0.0, 0.0), false) } else { Self::Unusable }
        } else {
            Self::AsIs
        }
    }

    fn scale(&self) -> f64 {
        if let Self::Mapped { scale, .. } = self { *scale } else { 1.0 }
    }

    /// A point of the frame, in the video
    fn point(&self, p: (f32, f32)) -> (f32, f32) {
        match self {
            Self::Mapped { scale, origin, .. } => (((p.0 as f64 + origin.0) * scale) as f32, ((p.1 as f64 + origin.1) * scale) as f32),
            _ => p,
        }
    }

    /// The principal point where the file leaves it out: the frame's centre
    fn default_principal_point(&self) -> Option<(f32, f32)> {
        match self {
            Self::Mapped { frame, centred_by_default: true, .. } => Some(self.point(((frame.0 / 2.0) as f32, (frame.1 / 2.0) as f32))),
            _ => None,
        }
    }

    /// The sensor area the video shows, from the frame's origin and size: as many sensor pixels per frame pixel, over
    /// the frame pixels the video shows
    fn capture_area(&self, origin: (f32, f32), size: (f32, f32)) -> ((f32, f32), (f32, f32)) {
        match self {
            Self::Mapped { origin: o, frame, shown, .. } => {
                let k = (size.0 as f64 / frame.0, size.1 as f64 / frame.1);
                ((origin.0 - (o.0 * k.0) as f32, origin.1 - (o.1 * k.1) as f32), ((shown.0 * k.0) as f32, (shown.1 * k.1) as f32))
            }
            _ => (origin, size),
        }
    }

    /// The video's extent along the readout over the frame's: the part of the frame's readout time the rows (or the
    /// columns) the video shows take
    fn readout_extent(&self, direction: ReadoutDirection) -> f64 {
        match self {
            Self::Mapped { frame, shown, .. } => if direction.is_horizontal() { shown.0 / frame.0 } else { shown.1 / frame.1 },
            _ => 1.0,
        }
    }

    /// How far the middle of what the video shows is from the frame's middle along the readout, in frame extents
    fn centre_shift(&self, direction: ReadoutDirection) -> f64 {
        match self {
            Self::Mapped { origin, frame, shown, .. } => {
                let shift = if direction.is_horizontal() {
                    (shown.0 / 2.0 - origin.0 - frame.0 / 2.0) / frame.0
                } else {
                    (shown.1 / 2.0 - origin.1 - frame.1 / 2.0) / frame.1
                };
                if direction.is_inverted() { -shift } else { shift }
            }
            _ => 0.0,
        }
    }
}

/// RED's exposure time of a frame (or of the clip), in ms
fn red_exposure_ms(tag_map: &GroupedTagMap) -> Option<f64> {
    let exposure_us = *(tag_map.get(&GroupId::Default)?.get_t(TagId::ExposureTime) as Option<&f32>)?;
    Some(exposure_us as f64 / 1000.0)
}

#[cfg(test)]
mod frame_mapping_tests {
    use super::*;
    use telemetry_parser::tag;
    use telemetry_parser::tags_impl::*;

    fn tags(frame: (u32, u32), other: Option<((u32, u32), (f32, f32))>) -> GroupedTagMap {
        let mut map = GroupedTagMap::new();
        let options = InputOptions::default();
        util::insert_tag(&mut map, tag!(parsed GroupId::Imager, TagId::Custom("FrameSize".into()), "", u32x2, |v| format!("{v:?}"), frame, vec![]), &options);
        if let Some((size, origin)) = other {
            util::insert_tag(&mut map, tag!(parsed GroupId::Imager, TagId::Custom("AlternativeFrameSize".into()), "", u32x2, |v| format!("{v:?}"), size, vec![]), &options);
            util::insert_tag(&mut map, tag!(parsed GroupId::Imager, TagId::Custom("FrameOriginInAlternative".into()), "", f32x2, |v| format!("{v:?}"), origin, vec![]), &options);
        }
        map
    }

    fn near(a: (f32, f32), b: (f64, f64)) -> bool { (a.0 as f64 - b.0).abs() < 1e-3 && (a.1 as f64 - b.1).abs() < 1e-3 }

    #[test]
    fn a_dng_frame_maps_onto_whichever_picture_the_decoder_shows() {
        // The default crop 3840x2160 at 8, 5 of the active area 3856x2170, on 1.556 sensor pixels a pixel
        let dng = tags((3840, 2160), Some(((3856, 2170), (8.0, 5.0))));
        let k = 6000.0 / 3856.0;
        let (origin, size) = ((8.0 * k, 311.72 + 5.0 * k), (3840.0 * k, 2160.0 * k));

        // Resolve shows the default crop: the frame itself
        assert_eq!(FrameMapping::of(&dng, (3840, 2160)), FrameMapping::AsIs);

        // FFmpeg shows the active area: the same pixels, the frame 8, 5 in, and the active area's sensor rows
        let full = FrameMapping::of(&dng, (3856, 2170));
        assert_eq!(full.scale(), 1.0);
        assert!(near(full.point((1920.0, 1080.0)), (1928.0, 1085.0)));
        assert!(near(full.default_principal_point().unwrap(), (1928.0, 1085.0)));
        let (o, s) = full.capture_area((origin.0 as f32, origin.1 as f32), (size.0 as f32, size.1 as f32));
        assert!(near(o, (0.0, 311.72)) && near(s, (6000.0, 2170.0 * k)), "{o:?} {s:?}");
        assert!((full.readout_extent(ReadoutDirection::TopToBottom) - 2170.0 / 2160.0).abs() < 1e-12);
        assert!(full.centre_shift(ReadoutDirection::TopToBottom).abs() < 1e-12, "the crop is centred");

        // Proxies of either: by their aspect ratio, the frame's on a tie
        let proxy = FrameMapping::of(&dng, (1920, 1080));
        assert_eq!((proxy.scale(), proxy.point((1920.0, 1080.0))), (0.5, (960.0, 540.0)));
        assert_eq!(proxy.readout_extent(ReadoutDirection::TopToBottom), 1.0);
        let half_full = FrameMapping::of(&dng, (1928, 1085));
        assert!(near(half_full.point((1920.0, 1080.0)), (964.0, 542.5)));

        // None of the pictures: as it is
        assert_eq!(FrameMapping::of(&dng, (1080, 1920)), FrameMapping::AsIs);
        assert_eq!(FrameMapping::of(&GroupedTagMap::new(), (1920, 1080)), FrameMapping::AsIs);
    }

    #[test]
    fn an_uneven_other_picture_moves_the_middle_of_the_readout() {
        // An 80 row frame at the top of a 100 row picture: the video's middle row is 10 rows below the frame's
        let mapping = FrameMapping::of(&tags((100, 80), Some(((100, 100), (0.0, 0.0)))), (100, 100));
        assert!((mapping.readout_extent(ReadoutDirection::TopToBottom) - 1.25).abs() < 1e-12);
        assert!((mapping.centre_shift(ReadoutDirection::TopToBottom) - 0.125).abs() < 1e-12);
        assert!((mapping.centre_shift(ReadoutDirection::BottomToTop) + 0.125).abs() < 1e-12);
        assert_eq!(mapping.centre_shift(ReadoutDirection::LeftToRight), 0.0);
        assert_eq!(mapping.readout_extent(ReadoutDirection::LeftToRight), 1.0);
    }

    #[test]
    fn canons_recorded_frame_scales_or_doesnt_apply() {
        let mut canon = GroupedTagMap::new();
        util::insert_tag(&mut canon, tag!(parsed GroupId::Imager, TagId::Custom("ActiveAreaAspectRatio".into()), "", u32x2, |v| format!("{v:?}"), (4096, 2160), vec![]), &InputOptions::default());
        let half = FrameMapping::of(&canon, (2048, 1080));
        assert_eq!((half.scale(), half.default_principal_point()), (0.5, None));
        assert_eq!(FrameMapping::of(&canon, (4096, 2160)), FrameMapping::AsIs);
        assert_eq!(FrameMapping::of(&canon, (1920, 1080)), FrameMapping::Unusable);
    }
}
