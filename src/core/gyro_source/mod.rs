// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

mod file_metadata;
mod imu_transforms;
mod sony;
mod canon;
mod motioncam;
mod per_frame;
mod segments;
pub mod splines;
pub mod optical_correction;
pub use file_metadata::*;
pub use segments::*;
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
use telemetry_parser::util::ReadingOrigin;
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
    /// The video the motion data is loaded for, when the file loaded isn't the video's own (a telemetry log, another
    /// camera's recording): the file's segments are then the video's, by its segment table's rule for telemetry logs
    /// (telemetry-parser's `Input::read_video_segment_table`). The video's, so it isn't stored either
    #[serde(skip)]
    pub video_url: Option<String>,
    /// The presentation times of the video's frames, when it has them (`StabilizationParams::frame_times`: frames that
    /// aren't evenly spaced from its start, which it's then looked up by): a Gyroflow protobuf frame's offset is taken
    /// against its time there, or at `k / fps` without them (`gyroflow_proto_time_offset`). The video's, so it isn't
    /// stored either
    #[serde(skip)]
    pub video_frame_times: Option<Arc<crate::frame_times::FrameTimes>>,
}

#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct GyroSource {
    pub file_load_options: FileLoadOptions,

    pub duration_ms: f64,

    // The readings and the orientations are shared (`Arc`s), and only ever replaced, never changed where another copy
    // can see it: a copy of the motion data (a render queue job's, a smoothing's or an integration's, see
    // `integrate_shared`) costs next to nothing, and takes what it reads out of the lock

    raw_imu: Arc<Vec<TimeIMU>>,

    pub imu_transforms: IMUTransforms,

    pub integration_method: usize,

    pub quaternions: Arc<TimeQuat>,
    pub smoothed_quaternions: Arc<TimeQuat>,

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

    /// Correction measured from the video, composed onto `quaternions` at the end of `integrate`. Shared, like the
    /// orientations: a long clip's is megabytes
    #[serde(default)]
    pub optical_correction: Option<Arc<OpticalCorrection>>,
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
    /// Whether the last `integrate` took the motion from the video (`OpticalCorrection::video_base`): a file without
    /// motion data, or with its own set aside
    #[serde(skip)]
    motion_from_video: bool,
    /// The file's own motion data and sync points, set aside while the motion comes from the video instead, see
    /// `set_ignore_file_motion`
    #[serde(skip)]
    ignored_motion: Option<Arc<(FileMotion, BTreeMap<i64, f64>)>>,

    /// The video's frame rate scale (`StabilizationParams::fps_scale`): the video's times here are its file's over it
    #[serde(skip)]
    pub fps_scale: Option<f64>,
    /// The rate the motion data was captured at, as the load of its file found it, when the video presents its frames
    /// at another one: an off-speed recording's, which the load of the video's own file puts the video on the motion
    /// data's timeline at (`StabilizationManager::capture_frame_rate`). `None` otherwise
    #[serde(skip)]
    pub capture_frame_rate: Option<f64>,
    /// How long the video lasts in its time (`StabilizationParams::get_scaled_duration_ms`), as `init_from_params` was
    /// last given it, or as a caller that doesn't call it set `duration_ms` before loading the motion data. The clip
    /// lasts as long as the video, or as its motion data where that spans another time (`refresh_duration`)
    #[serde(skip)]
    video_duration_ms: f64,
    /// Where the segments of a video cut from recordings are (`FileMetadata::trim_segments`), when it has more than one,
    /// with its motion data at the frame rate scale `fps_scale` (`refresh_segments`)
    #[serde(skip)]
    segments: Option<Arc<SegmentRanges>>,
    /// The sync points of each segment, as `offsets` and `offsets_adjusted` (empty without segments)
    #[serde(skip)]
    offsets_by_segment: Vec<(BTreeMap<i64, f64>, BTreeMap<i64, f64>)>,
}

/// How the orientations are brought up to date with the motion data and the settings (`GyroSource::integrate_shared`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Integration {
    /// The IMU readings transformed and filtered again, then integrated (`GyroSource::apply_transforms`)
    ApplyTransforms,
    /// The IMU readings integrated as they are (`GyroSource::integrate`)
    Integrate,
}

/// Where a lookup of the motion data may read: the motion data of one segment of a video cut from recordings, the one
/// the frame being looked up is in, or all of it (`GyroSource::lookup_for_frame`)
#[derive(Clone, Debug)]
pub struct MotionLookup {
    pub segment: Option<usize>,
    pub motion: std::ops::Range<i64>,
}
impl MotionLookup {
    pub const ALL: Self = Self { segment: None, motion: i64::MIN..i64::MAX };
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
        let changed = self.set_ignore_file_motion_unintegrated(ignore);
        if changed { self.apply_transforms(); }
        changed
    }
    /// `set_ignore_file_motion` without the integration of the motion data there is then, which is the caller's
    /// (`Integration::ApplyTransforms`): the shared motion data is integrated with its lock released (`integrate_shared`)
    pub fn set_ignore_file_motion_unintegrated(&mut self, ignore: bool) -> bool {
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
        // The clip's duration and the segments' motion data are the motion data's there is now
        self.refresh_duration();
        self.refresh_segments();
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

    /// Takes the video's timing in (`set_video_timing`), and integrates the motion data again when that needs it
    pub fn init_from_params(&mut self, stabilization_params: &StabilizationParams) {
        if self.set_video_timing(stabilization_params) {
            self.apply_transforms();
        }
    }

    /// Takes the video's timing in: its duration and its frame rate scale (`StabilizationParams::get_scaled_duration_ms`,
    /// `fps_scale`). Cheap, and returns whether the motion data has to be integrated again for it (`apply_transforms`),
    /// which is the caller's to do: not under the parameters' lock, which every reader of them would wait on for the
    /// whole integration, and not at all when it integrates anyway next (a project import loads its motion data and its
    /// integration settings after the video's timing)
    #[must_use = "the motion data may have to be integrated again (`apply_transforms`)"]
    pub fn set_video_timing(&mut self, stabilization_params: &StabilizationParams) -> bool {
        self.video_duration_ms = stabilization_params.get_scaled_duration_ms();
        self.fps_scale = stabilization_params.fps_scale.filter(|x| StabilizationParams::is_valid_fps_scale(*x));
        // The clip lasts as long as its motion data whenever the video's duration is set, before the motion data is
        // loaded or after it (a frame rate overridden then): the integration, the filters and the smoothing take the
        // motion data's rate from it
        let duration_changed = self.refresh_duration();
        // The motion data is filtered and integrated piece by piece: pieces the scale moved (a clip-wide IMU's readings
        // change parts with it) need it again, or a part would start from the orientation and the filter state of
        // another moment, and the jump an integration restarts with would sit inside a part
        let moved = self.refresh_segments();
        (moved || duration_changed) && !self.quaternions.is_empty()
    }

    /// Sets the clip's duration (`duration_ms`) from the video's (`video_duration_ms`) and the motion data's, as a clip's
    /// is taken (`segment_duration_ms` of all of the motion data: the video's, or the span of the camera's orientations
    /// or of the IMU readings where they span another time), and returns whether it changed. Again whenever the video's
    /// duration or the motion data change, so the duration is the same whatever order they're set in
    fn refresh_duration(&mut self) -> bool {
        let duration = if self.video_duration_ms > 0.0 {
            segment_duration_ms(&self.file_metadata.read(), self.video_duration_ms, i64::MIN..i64::MAX)
        } else {
            self.video_duration_ms
        };
        let changed = !Self::same_duration(self.duration_ms, duration);
        self.duration_ms = duration;
        changed
    }
    /// Whether two durations (ms) are one but for floating point rounding: the same duration computed again from other
    /// values (the video's over the frame rate scale, a project's) differs in its last bits, and an integration started
    /// again for that would only take its time
    fn same_duration(a: f64, b: f64) -> bool {
        a.to_bits() == b.to_bits() || (a - b).abs() <= 1e-9 * a.abs().max(b.abs())
    }

    /// Takes the segments of the file's metadata in (`segments`), and the sync points' with them: where the segments are
    /// in the motion data, at the frame rate scale in force (`SegmentRanges::new`). Again whenever the scale or the
    /// motion data change. Whether the pieces the motion data is handled in moved (`SegmentRanges::same_pieces`)
    fn refresh_segments(&mut self) -> bool {
        let scale = self.fps_scale.unwrap_or(1.0);
        let md = self.file_metadata.read();
        let segments = md.trim_segments.as_ref().filter(|s| s.len() > 1).map(|s| Arc::new(SegmentRanges::new(s, &md, scale)));
        drop(md);
        let moved = match (&self.segments, &segments) {
            (None, None) => false,
            (Some(a), Some(b)) => !a.same_pieces(b),
            _ => true,
        };
        self.segments = segments;
        self.adjust_offsets();
        moved
    }
    /// How long the motion data of segment `r` lasts (`SegmentRanges::durations_ms`), of all of it for `None`
    fn piece_duration_ms(&self, r: Option<usize>) -> f64 {
        r.and_then(|r| self.segments.as_ref()?.durations_ms.get(r).copied()).unwrap_or(self.duration_ms)
    }
    /// Where the segments of a video cut from recordings are, when it has more than one
    pub fn segment_ranges(&self) -> Option<&SegmentRanges> {
        self.segments.as_deref()
    }
    /// A copy to look orientations up in (`org_quat_at_*`, the sync points, the segments), without what only the
    /// integration and the smoothing read: the filtered IMU readings and the smoothed orientations, the bulk of a long
    /// clip's motion data. Without the orientations too unless `orientations`. Every field is listed, so one added
    /// later is taken over or left out on purpose
    pub fn clone_for_lookups(&self, orientations: bool) -> Self {
        Self {
            file_load_options: self.file_load_options.clone(),
            duration_ms: self.duration_ms,
            raw_imu: Default::default(),
            imu_transforms: self.imu_transforms.clone(),
            integration_method: self.integration_method,
            quaternions: if orientations { self.quaternions.clone() } else { Default::default() },
            smoothed_quaternions: Default::default(),
            use_gravity_vectors: self.use_gravity_vectors,
            horizon_lock_integration_method: self.horizon_lock_integration_method,
            max_angles: self.max_angles,
            smoothing_status: self.smoothing_status.clone(),
            prevent_recompute: self.prevent_recompute,
            file_metadata: self.file_metadata.clone(),
            offsets: self.offsets.clone(),
            offsets_linear: self.offsets_linear.clone(),
            offsets_adjusted: self.offsets_adjusted.clone(),
            file_url: self.file_url.clone(),
            all_metadata: self.all_metadata,
            optical_correction: self.optical_correction.clone(),
            optical_correction_applied: self.optical_correction_applied,
            optical_uncorrected_checksum: self.optical_uncorrected_checksum,
            optical_context: self.optical_context,
            motion_from_video: self.motion_from_video,
            ignored_motion: self.ignored_motion.clone(),
            fps_scale: self.fps_scale,
            capture_frame_rate: self.capture_frame_rate,
            video_duration_ms: self.video_duration_ms,
            segments: self.segments.clone(),
            offsets_by_segment: self.offsets_by_segment.clone(),
        }
    }
    /// The motion data an optical analysis measures the picture against, from the shared `gyro`: a copy to look
    /// orientations up in (`clone_for_lookups`) without the optical correction, its orientations as they are without it,
    /// or without any unless `orientations`. Where the correction is composed onto them (`optical_correction_applied`)
    /// they're integrated again, from the filtered IMU readings (shared), once the lock is released; otherwise they're
    /// the ones there are
    pub fn uncorrected_for_lookups(gyro: &RwLock<GyroSource>, orientations: bool) -> Self {
        let (mut copy, corrected) = {
            let gyro = gyro.read();
            let corrected = orientations && gyro.optical_correction.is_some() && gyro.optical_correction_applied;
            let mut copy = gyro.clone_for_lookups(orientations && !corrected);
            if corrected { copy.raw_imu = gyro.raw_imu.clone(); }
            (copy, corrected)
        };
        copy.optical_correction = None;
        copy.optical_correction_applied = false;
        if corrected {
            copy.integrate();
            copy.raw_imu = Default::default();
        }
        copy
    }
    /// Whether the video's frames `a` and `b` are of one part of a video cut from recordings (`segment_ranges`), as all
    /// of a video's frames are without: a part's picture doesn't go on in the next one, which is of another moment
    pub fn frames_of_one_part(&self, a: usize, b: usize) -> bool {
        self.segments.as_ref().is_none_or(|s| s.of_frame(a) == s.of_frame(b))
    }
    /// What a lookup for the video's frame `frame` may read: the motion data of the frame's segment (`segment_lookup`)
    pub fn lookup_for_frame(&self, frame: usize) -> MotionLookup {
        match &self.segments {
            Some(s) => self.segment_lookup(s, s.of_frame(frame), s.video_us_of_frame(frame)),
            None => MotionLookup::ALL,
        }
    }
    /// What a lookup at the video's time `timestamp_ms` may read: the motion data of the segment the video shows then
    /// (`segment_lookup`)
    pub fn lookup_for_video_time(&self, timestamp_ms: f64) -> MotionLookup {
        match &self.segments {
            Some(s) => { let t_us = (timestamp_ms * 1000.0).round() as i64; self.segment_lookup(s, s.of_video_us(t_us), t_us) }
            None => MotionLookup::ALL,
        }
    }
    /// What a lookup of segment `r` at the video's time `video_us` (µs, over the frame rate scale) may read: the
    /// segment's motion data, where its orientations reach the time the lookup reads them at (the video's less the
    /// segment's sync offset), or are held there (`SegmentRanges::reach_us`). Where they don't, the segments don't
    /// describe the motion data's timeline there (a frame rate overridden, a segment whose sync doesn't put its motion
    /// data at its frames, a segment without any): the lookup reads all of the motion data, as in a video without
    /// segments, with the segment's sync offset
    fn segment_lookup(&self, s: &SegmentRanges, r: usize, video_us: i64) -> MotionLookup {
        let video_ms = video_us as f64 / 1000.0;
        let motion_us = (video_ms - self.offset_at_video_timestamp_in(video_ms, Some(r))) * 1000.0;
        MotionLookup { segment: Some(r), motion: self.part_motion(s, r, motion_us, Some(video_ms)).0 }
    }
    /// The motion data a lookup of segment `r` at the motion data's time `motion_us` reads (`segment_lookup`): the
    /// segment's, or all of it where the segment's orientations don't reach that time, reported once when `report` gives
    /// the video's time it was for. With the first and the last of the orientations (`quaternions`) it's held between,
    /// when it reads the segment's
    fn part_motion(&self, s: &SegmentRanges, r: usize, motion_us: f64, report: Option<f64>) -> (std::ops::Range<i64>, Option<(i64, i64)>) {
        let own = s.motion[r].clone();
        if self.quaternions.len() < 2 { return (own, None); }
        let bounds = Self::bounds(&self.quaternions, &own);
        if bounds.is_some_and(|(first, last)| motion_us >= first as f64 - s.reach_us(r) && motion_us <= last as f64 + s.reach_us(r)) {
            return (own, bounds);
        }
        if let Some(video_ms) = report && !s.reported.swap(true, std::sync::atomic::Ordering::Relaxed) {
            log::warn!("Segment {r} of the video (frames from {}) has no motion data at {:.3} ms, where its time {:.3} ms reads it: it's looked up in all of the motion data, as without segments. Synchronize the segment", s.first_frames[r], motion_us / 1000.0, video_ms);
        }
        (MotionLookup::ALL.motion, None)
    }
    /// The first and the last timestamp of `quats` in `motion`
    fn bounds(quats: &TimeQuat, motion: &std::ops::Range<i64>) -> Option<(i64, i64)> {
        let mut data = quats.range(motion.clone());
        let first = *data.next()?.0;
        Some((first, data.next_back().map_or(first, |(t, _)| *t)))
    }

    pub fn parse_telemetry_file<T: Read + Seek, P: AsRef<std::path::Path>, F: Fn(f64)>(stream: &mut T, filesize: usize, path: P, options: &FileLoadOptions, size: (usize, usize), fps: f64, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> Result<FileMetadata, crate::GyroflowCoreError> {
        // The video's frame times by their fingerprint (`FrameTimes::checksum`), not every one of them
        let frame_times = options.video_frame_times.as_ref().map(|t| t.checksum());
        let key = format!("{}{:?}{frame_times:?}{size:?}{fps}", path.as_ref().display(), FileLoadOptions { video_frame_times: None, ..options.clone() });
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
        // The motion data of another file than the video, which has no frames of the video's: its segments are the
        // video's, with the clocks of its table only when the table names this file as the log it was trimmed with
        if let Some(video_url) = options.video_url.as_deref() {
            match crate::filesystem::open_file(video_url, false, false) {
                Ok(mut video) => {
                    let video_size = video.size;
                    input.read_video_segment_table(video.get_file(), video_size, stream, filesize, &path);
                }
                Err(e) => log::warn!("The video {video_url} the motion data is loaded for can't be opened for its segment table: {e:?}"),
            }
        }
        // The video's segment table (`segments`), when it was cut from camera recordings and the table is valid for it
        let segment_table = input.segment_table().cloned();

        let camera_identifier = CameraIdentifier::from_telemetry_parser(&input, size.0, size.1, fps).ok();

        let mut detected_source = input.camera_type();
        if let Some(m) = input.camera_model() { detected_source.push(' '); detected_source.push_str(m); }

        let mut imu_orientation = None;
        let mut quaternions = TimeQuat::default();
        let mut gravity_vectors: Option<TimeVec> = None;
        let mut image_orientations = None;
        // The sample each quaternion came from (`TrimSegments::new`)
        let mut quat_origins: Vec<(i64, ReadingOrigin)> = Vec::new();
        // The times of each part's metadata samples, of a video cut from recordings (`TrimSegments::metadata_us`)
        let mut metadata_us: Vec<Option<(i64, i64)>> = Vec::new();
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
            let is_red = input.camera_type() == "RED";
            // A sample's part, of a video cut from recordings (`contiguous_parts`): by its time on the file's timeline, where
            // a container times its samples, and a RED frame's by its frame (`frame_of_sample`), as RED times its frames
            // by the camera clock, which an off-speed clip runs at its capture rate
            let parts = segment_table.as_ref().map(contiguous_parts).unwrap_or_default();
            let by_frame = input.parser_name() == "RedR3d";
            let sample_segment = |info: &util::SampleInfo| segment_table.as_ref().map(|t| {
                let segment = match by_frame.then(|| frame_of_sample(&input, info)).flatten() {
                    Some(frame) => t.segment_of_frame(frame as u64),
                    None => t.segment_of_time_us(((info.timestamp_ms + info.duration_ms.max(0.0) / 2.0) * 1000.0).round() as i64),
                };
                parts.partition_point(|p| p.end <= segment).min(parts.len().saturating_sub(1))
            });
            let sample_parts: Vec<Option<usize>> = if segment_table.is_some() { samples.iter().map(sample_segment).collect() } else { Vec::new() };
            // What each sample states of the lens, read once (`LensTags`). A video of more than one part reads all of them
            // ahead, for what each part's own metadata gives of the lens: at a part's start, that decides what's carried
            // into it (`lens_at_segment_start`)
            let mut sample_lens: Vec<Option<LensTags>> = Vec::new();
            let mut segment_lens: Vec<LensParams> = Vec::new();
            if parts.len() > 1 {
                segment_lens = vec![LensParams::default(); parts.len()];
                sample_lens.reserve_exact(samples.len());
                for (sample_pos, info) in samples.iter().enumerate() {
                    let tags = info.tag_map.as_ref().map(|tag_map| {
                        let timestamp_us = (info.timestamp_ms * 1000.0).round() as i64;
                        read_lens_tags(tag_map, FrameMapping::of(tag_map, size), size, is_red, is_gyroflow_proto, &mut frame_readout_direction_tag, &mut lens_profile, &mut lens_positions, timestamp_us)
                    });
                    if let (Some(tags), Some(Some(r))) = (&tags, sample_parts.get(sample_pos)) {
                        tags.apply(&mut segment_lens[*r]);
                    }
                    sample_lens.push(tags);
                }
            }
            let mut lens_segment = None;
            metadata_us = vec![None; parts.len()];
            for (sample_pos, info) in samples.iter().enumerate() {
                let timestamp_us = (info.timestamp_ms * 1000.0).round() as i64;
                // The lens values a sample lacks are the ones before it, but never another segment's of what the
                // segment's own metadata gives (`lens_at_segment_start`)
                if let Some(&Some(segment)) = sample_parts.get(sample_pos) {
                    if lens_segment.is_some_and(|s| s != segment) {
                        lens_info = lens_at_segment_start(&lens_info, &segment_lens[segment]);
                    }
                    lens_segment = Some(segment);
                    // A frame's lens metadata, keyed by its sample's time, is looked up among its part's samples only
                    if info.tag_map.is_some() {
                        let span = &mut metadata_us[segment];
                        *span = Some(span.map_or((timestamp_us, timestamp_us), |(a, b)| (a.min(timestamp_us), b.max(timestamp_us))));
                    }
                }
                if let Some(ref tag_map) = info.tag_map {
                    let frame_mapping = FrameMapping::of(tag_map, size);
                    if let FrameMapping::Mapped { .. } = frame_mapping {
                        video_mapping.get_or_insert(frame_mapping);
                    }
                    if let Some(map) = tag_map.get(&GroupId::Quaternion) {
                        if let Some(arr) = map.get_t(TagId::Data) as Option<&Vec<TimeQuaternion<f64>>> {
                            for v in arr {
                                quats.insert(motion_key(v.t), Quat64::from_quaternion(Quaternion::from_parts(
                                    v.v.w,
                                    Vector3::new(v.v.x, v.v.y, v.v.z)
                                )));
                                quat_origins.push((motion_key(v.t), ReadingOrigin { sample: sample_pos, frame: None }));
                            }
                        }
                    }
                    let tags = match sample_lens.get_mut(sample_pos).and_then(Option::take) {
                        Some(tags) => tags,
                        None => read_lens_tags(tag_map, frame_mapping, size, is_red, is_gyroflow_proto, &mut frame_readout_direction_tag, &mut lens_profile, &mut lens_positions, timestamp_us),
                    };
                    tags.apply(&mut lens_info);
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

        // The IMU readings, each sensor's on its own timeline with where each reading came from. In a video cut from
        // recordings, the accelerometer and the magnetometer are put at a gyroscope reading's time from their readings of
        // its part only (`TrimSegments::part_of_reading`): the readings of another part are another moment's, and a
        // value between the two would be no moment's, at a part's start where each part's integration starts from it.
        // A reading whose part its sample and frame don't tell is of the part it's timed in (`SegmentRanges`), which
        // the video's frame rate scale places: it's taken with every part's (`ImuReadings::interpolated`)
        let readings = util::imu_readings(&input, Some("XYZ".into())).unwrap_or_default();
        let reading_parts = segment_table.as_ref().map(|table| TrimSegments::new(table.clone(), &[], &[])).filter(|parts| parts.len() > 1).map(|parts| {
            let gyro: Vec<(i64, ReadingOrigin)> = readings.gyro.iter().map(|(t, (_, origin))| (*t, *origin)).collect();
            let sample_parts = parts.sample_parts(input.samples.as_deref().unwrap_or_default(), &[(MotionSeries::Imu, &gyro), (MotionSeries::Orientations, &quat_origins)]);
            (parts, sample_parts)
        });
        let (mut raw_imu, origins) = readings.interpolated(|origin| reading_parts.as_ref().and_then(|(parts, sample_parts)| parts.part_of_reading(sample_parts, origin)));
        // The values that came from readings of no known part are of their reading's part only if the parts' places put
        // those readings in it too (`SegmentRanges::cut_values`), which the video's frame rate scale decides
        let unplaced = reading_parts.as_ref().map(|(parts, sample_parts)| readings.unplaced_values(|origin| parts.part_of_reading(sample_parts, origin))).unwrap_or_default();
        drop(readings);
        let mut imu_origins: Vec<usize> = origins.iter().map(|x| x.sample).collect();

        // Sony: the metadata packets carry the gyro timing (offset and rate per frame), use it instead of a uniform spacing
        let sony_packet_timed = input.camera_type() == "Sony" && input.samples.as_ref().map(|s| sony::retime_imu_from_packets(&mut raw_imu, &mut imu_origins, s)).unwrap_or(false);
        if sony_packet_timed { log::debug!("Sony gyro samples re-timed from the metadata packets"); }
        // Where each reading came from, in the readings' order. Sony's readings come with their frame's sample, none
        // with a frame of its own, and re-timing them can reorder them, with their samples
        let imu_origins: Vec<ReadingOrigin> = if sony_packet_timed {
            imu_origins.into_iter().map(|sample| ReadingOrigin { sample, frame: None }).collect()
        } else {
            origins
        };

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

        // The segments of a video cut from recordings, and the parts of its readings that their samples and frames tell
        let trim_segments = segment_table.map(|table| {
            let imu: Vec<(i64, ReadingOrigin)> = raw_imu.iter().map(|x| motion_key(x.timestamp_ms)).zip(imu_origins.iter().copied()).collect();
            let mut segments = TrimSegments::new(table, input.samples.as_deref().unwrap_or_default(), &[(MotionSeries::Imu, &imu), (MotionSeries::Orientations, &quat_origins)]);
            segments.metadata_us = metadata_us;
            segments.unplaced = unplaced;
            segments.motion_unplaced = !input.readings_at_table_frames();
            segments
        });

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
            lens_variation:   Default::default(),
            trim_segments,
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
            // A video cut from recordings fills each part's frames from its own (`PerFrame::interpolate_within`): another
            // part's frames are of another moment, or another recording
            let part_frames: Vec<std::ops::Range<usize>> = md.trim_segments.as_ref().map(|t| (0..t.len()).map(|r| t.frames(r)).collect()).unwrap_or_default();
            for (info, &frame) in samples.iter().zip(&sample_frames) {
                if let Some(ref tag_map) = info.tag_map {
                    // --------------------------------- Timing -------------------------------
                    // One source for all frames: the format's own timing, else the camera's
                    if let Some(frame) = frame {
                        let offset = if is_gyroflow_proto {
                            // The time the video presents the frame at, which it's looked up at: by its frame times, else `k / fps`
                            let frame_i32 = i32::try_from(frame).unwrap_or(i32::MAX);
                            let presented_ms = match &options.video_frame_times {
                                Some(times) => Some(times.timestamp_at_frame(frame_i32, 1.0)),
                                None => (fps > 0.0).then(|| crate::timestamp_at_frame(frame_i32, fps)),
                            };
                            gyroflow_proto_time_offset(tag_map, &FrameMapping::of(tag_map, size), frame_readout_direction, info.timestamp_ms, presented_ms)
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
            md.per_frame_time_offsets = time_offsets.interpolate_within(&part_frames);
            if input.camera_type() == "Sony" {
                md.lens_breathing = sony::breathing::compute(samples, &sample_frames, &part_frames);
            }
            // Only a clip with corrections has the table (and misses any): without one on any frame, it would be all
            // empty frames
            if mesh_frames.values().any(|m| !m.is_empty()) {
                mesh_frames.report();
                md.mesh_correction.frames = mesh_frames.hold_within(&part_frames, MeshFrame::default());
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
                // Each segment of a video cut from recordings on its own, spaced by its source's first samples (the table's
                // `sony_ibis_template`): its splines are then its source's
                let stab_segments: Vec<sony::StabSegment> = match &md.trim_segments {
                    Some(ts) => (0..ts.len()).map(|r| sony::StabSegment {
                        frames: (if r == 0 { 0 } else { ts.frames(r).start })..(if r + 1 == ts.len() { usize::MAX } else { ts.frames(r).end }),
                        template: ts.source(r).sony_ibis_template(),
                    }).collect(),
                    None => vec![sony::StabSegment { frames: 0..usize::MAX, template: None }],
                };
                md.camera_stab_data = sony::stab_calc_splines(&md, &is_temp, &stab_segments, sample_rate, fps, size).unwrap_or_default();
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
        self.quaternions = Default::default();
        self.smoothed_quaternions = Default::default();
        self.raw_imu = Default::default();
        self.imu_transforms.imu_rotation = None;
        self.imu_transforms.acc_rotation = None;
        self.imu_transforms.imu_lpf = 0.0;
        self.imu_transforms.imu_mf = 0;
        self.file_metadata = Default::default();
        self.optical_correction = None;
        // What the last `integrate` made of the orientations cleared above: a file whose orientations are loaded as they
        // are (`load_from_telemetry` without IMU readings) isn't integrated, and would keep them
        self.optical_correction_applied = false;
        self.motion_from_video = false;
        self.ignored_motion = None;
        self.segments = None;
        self.capture_frame_rate = None;
        self.clear_offsets();
    }

    pub fn load_from_telemetry(&mut self, telemetry: FileMetadata) {
        match self.load_from_telemetry_unintegrated(telemetry) {
            Some(Integration::ApplyTransforms) => self.apply_transforms(),
            Some(Integration::Integrate) => self.integrate(),
            None => { }
        }
    }
    /// `load_from_telemetry` without the integration it takes, which it returns for the caller to run: the shared motion
    /// data is integrated with its lock released (`integrate_shared`)
    #[must_use = "the motion data has to be integrated (`integrate_shared`)"]
    pub fn load_from_telemetry_unintegrated(&mut self, mut telemetry: FileMetadata) -> Option<Integration> {
        // The video's duration, as `init_from_params` was given it, or as a caller set the clip's without it
        if self.video_duration_ms <= 0.0 { self.video_duration_ms = self.duration_ms; }
        if self.video_duration_ms <= 0.0 {
            ::log::error!("Invalid duration_ms {}", self.video_duration_ms);
            return None;
        }

        self.clear();
        sony::upgrade_legacy_metadata(&mut telemetry);

        self.imu_transforms.imu_orientation = telemetry.imu_orientation.clone();

        let has_quats = !telemetry.quaternions.is_empty();
        let has_raw_imu = !telemetry.raw_imu.is_empty();

        self.file_metadata = telemetry.into();
        // The clip lasts as long as its motion data where that spans another time than the video (`refresh_duration`)
        self.refresh_duration();
        if self.duration_ms != self.video_duration_ms {
            log::warn!("IMU duration {} is different than video duration ({})", self.duration_ms, self.video_duration_ms);
        }
        self.refresh_segments();

        if has_quats {
            self.quaternions = Arc::new(self.file_metadata.read().quaternions.clone());
            self.integration_method = 0;
        }

        if has_raw_imu {
            Some(Integration::ApplyTransforms)
        } else if self.quaternions.is_empty() {
            Some(Integration::Integrate)
        } else {
            None
        }
    }
    pub fn integrate(&mut self) {
        let file_metadata = self.file_metadata.read();
        // A video cut from recordings is integrated segment by segment, each from its own data: its motion is of another
        // moment than the segment's before it, and an integration across the boundary would carry that one's
        // orientation, bias and filter state into it (`segments`)
        let pieces = split_ranges(self.segments.as_deref());
        let raw_imu = self.raw_imu(&file_metadata);
        match self.integration_method {
            0 => {
                let gopro = file_metadata.detected_source.as_deref().unwrap_or("").starts_with("GoPro") && !file_metadata.quaternions.is_empty() && (file_metadata.gravity_vectors.is_none() || !self.use_gravity_vectors);
                if gopro { log::info!("No gravity vectors - using accelerometer"); }
                let empty = TimeQuat::default();
                let image_orientations = file_metadata.image_orientations.as_ref().unwrap_or(&empty);
                // The orientations of a piece: all of them as they are, a part's copied out (the converter takes a map)
                fn of_piece<'a>(all: &'a TimeQuat, range: &std::ops::Range<i64>, r: Option<usize>) -> std::borrow::Cow<'a, TimeQuat> {
                    match r {
                        None => std::borrow::Cow::Borrowed(all),
                        Some(_) => std::borrow::Cow::Owned(all.range(range.clone()).map(|(k, v)| (*k, *v)).collect()),
                    }
                }
                let mut integrated = per_piece(pieces, |range, r| {
                    let quats = of_piece(&file_metadata.quaternions, &range, r);
                    let duration_ms = self.piece_duration_ms(r);
                    let mut quats = if gopro {
                        QuaternionConverter::convert(self.horizon_lock_integration_method, &quats, &of_piece(image_orientations, &range, r), imu_slice(raw_imu, &range), duration_ms)
                    } else {
                        quats.into_owned()
                    };
                    if self.imu_transforms.imu_lpf > 0.0 && !quats.is_empty() && duration_ms > 0.0 {
                        let sample_rate = quats.len() as f64 / (duration_ms / 1000.0);
                        if let Err(e) = super::filtering::Lowpass::filter_quats_forward_backward(self.imu_transforms.imu_lpf, sample_rate, &mut quats) {
                            log::error!("Filter error {:?}", e);
                        }
                    }
                    quats
                });
                if let Some(rot) = self.imu_transforms.imu_rotation {
                    for (_ts, q) in &mut integrated {
                        *q = rot * *q;
                    }
                }
                self.quaternions = Arc::new(integrated);
            },
            1..=6 => {
                let integrate: fn(&[TimeIMU], f64) -> TimeQuat = match self.integration_method {
                    1 => ComplementaryIntegrator  ::integrate,
                    2 => VQFIntegrator            ::integrate,
                    3 => SimpleGyroIntegrator     ::integrate,
                    4 => SimpleGyroAccelIntegrator::integrate,
                    5 => MahonyIntegrator         ::integrate,
                    _ => MadgwickIntegrator       ::integrate,
                };
                self.quaternions = Arc::new(per_piece(pieces, |range, r| {
                    let imu = imu_slice(raw_imu, &range);
                    // A piece without two readings has no motion to integrate
                    if imu.len() < 2 && r.is_some() { return TimeQuat::new(); }
                    integrate(imu, self.piece_duration_ms(r))
                }));
            },
            _ => log::error!("Unknown integrator")
        }
        drop(file_metadata);
        self.apply_optical_correction();
    }

    /// Composes the correction measured from the video onto freshly integrated quaternions - when it was measured
    /// against these very ones, in this context, see `OpticalCorrection`
    fn apply_optical_correction(&mut self) {
        self.optical_correction_applied = false;
        self.motion_from_video = false;
        if let Some(c) = &self.optical_correction {
            let from_video = self.quaternions.len() < 2 && !c.video_base.is_empty();
            if from_video {
                // A file without motion data: what the analysis measured between the frames is all there is. It's
                // fingerprinted as the analysis fingerprints it (`optical_motion`'s `finish`), of all of it at once
                let base = c.base_quats();
                self.optical_uncorrected_checksum = optical_correction::checksum(&base);
                // A video cut from recordings has it part by part, each part's as its range trimmed alone has it, on the
                // grid from the part's start and without a stretch between two parts' frames
                self.quaternions = Arc::new(match self.segments.as_deref() {
                    Some(s) => c.base_quats_in(&(0..s.len()).map(|r| (s.motion[r].clone(), s.video_starts_us[r])).collect::<Vec<_>>()),
                    None => base,
                });
            } else {
                self.optical_uncorrected_checksum = optical_correction::checksum(&self.quaternions);
            }
            // The orientations `integrate` just made, which nothing else shares yet: changed in place, not copied
            if self.optical_correction_applies() {
                c.apply(Arc::make_mut(&mut self.quaternions));
                self.optical_correction_applied = true;
            } else if c.enabled {
                log::warn!("The optical correction was measured on other motion data (another integration method or filter), sync, lens or frame timing, not applying it");
            }
            if from_video {
                // Held over the clip's ends: the first part's from the start, the last part's to the end on its own grid
                let quats = Arc::make_mut(&mut self.quaternions);
                match self.segments.as_deref() {
                    Some(s) => {
                        optical_correction::hold_from(quats, 0);
                        optical_correction::hold_to(quats, s.video_starts_us[s.len() - 1], (self.duration_ms * 1000.0).round() as i64);
                    }
                    None => optical_correction::hold_over_clip(quats, self.duration_ms),
                }
            }
            self.motion_from_video = from_video;
        }
    }
    /// Whether `integrate` composes the correction onto the quaternions, as things stand
    pub fn optical_correction_applies(&self) -> bool {
        self.optical_correction.as_ref().is_some_and(|c| c.enabled && c.measured_on(self.optical_uncorrected_checksum, self.optical_context))
    }

    /// The smoothed orientations, as the correction from each of them to the camera's, and the largest angles between
    /// the two. `compute_params` are lent: what a pass of the smoothing takes from them differently (a part's frames, the
    /// trim left out) is set on them for that pass and put back, as a copy of them would copy every per-frame table they
    /// have, once more for each part of a video cut from recordings
    pub fn recompute_smoothness(&self, alg: &dyn SmoothingAlgorithm, horizon_lock: super::smoothing::horizon::HorizonLock, compute_params: &mut crate::ComputeParams) -> (TimeQuat, (f64, f64, f64)) {
        let file_metadata = self.file_metadata.read();
        // A video cut from recordings is smoothed segment by segment, each as a clip of its own: the smoothing of one
        // segment doesn't see the motion of another, of another moment, nor the jump in orientation between them
        let mut smoothed_quaternions = TimeQuat::new();
        match self.segments.as_deref().filter(|s| s.len() > 1 && !s.whole) {
            Some(s) => {
                // Each part is smoothed with the video's parameters, for the part's frames (`smoothing_frames`)
                let all_frames = compute_params.smoothing_frames.take();
                for (r, range) in s.motion.iter().enumerate() {
                    let quats: TimeQuat = self.quaternions.range(range.clone()).map(|(k, v)| (*k, *v)).collect();
                    let (Some((&first, _)), Some((&last, _))) = (quats.first_key_value(), quats.last_key_value()) else { continue };
                    let gravity = file_metadata.gravity_vectors.as_ref().map(|g| g.range(range.clone()).map(|(k, v)| (*k, *v)).collect());
                    let duration_ms = self.piece_duration_ms(Some(r));
                    compute_params.smoothing_frames = Some(s.frames(r));
                    let mut smoothed = if self.motion_from_video {
                        // Motion measured from the video is held over a clip's time (`optical_correction::hold_over_clip`),
                        // which the smoothing takes as part of the clip: a part's is held over the part's time too, as
                        // its range trimmed alone has it, and the hold is dropped again (the next part has that time):
                        // all of it is before the part's first orientation or after its last one
                        let mut held = quats;
                        let video = s.video_us(r);
                        optical_correction::hold_from(&mut held, video.start);
                        optical_correction::hold_to(&mut held, video.start, video.end);
                        let mut smoothed = self.smooth_piece(alg, &horizon_lock, &held, &gravity, duration_ms, compute_params);
                        smoothed.retain(|t, _| (first..=last).contains(t));
                        smoothed
                    } else {
                        self.smooth_piece(alg, &horizon_lock, &quats, &gravity, duration_ms, compute_params)
                    };
                    smoothed_quaternions.append(&mut smoothed);
                }
                compute_params.smoothing_frames = all_frames;
            }
            None => smoothed_quaternions = self.smooth_piece(alg, &horizon_lock, &self.quaternions, &file_metadata.gravity_vectors, self.duration_ms, compute_params),
        }
        drop(file_metadata);

        let max_angles = crate::Smoothing::get_max_angles(&self.quaternions, &smoothed_quaternions, compute_params);

        for (sq, q) in smoothed_quaternions.iter_mut().zip(self.quaternions.iter()) {
            // rotation quaternion from smooth motion -> raw motion to counteract it
            *sq.1 = sq.1.inverse() * q.1;
        }
        (smoothed_quaternions, max_angles)
    }

    /// The smoothed orientations of `quats`, the camera's orientations of a clip (or of a segment of one), lasting
    /// `duration_ms`, with the gravity vectors of the same time. `compute_params` are lent as `recompute_smoothness` lends them
    fn smooth_piece(&self, alg: &dyn SmoothingAlgorithm, horizon_lock: &super::smoothing::horizon::HorizonLock, quats: &TimeQuat, gravity_vectors: &Option<TimeVec>, duration_ms: f64, compute_params: &mut crate::ComputeParams) -> TimeQuat {
        let mut smoothed_quaternions = quats.clone();

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
            horizon_lock.lock(&mut smoothed_quaternions, quats, gravity_vectors, self.use_gravity_vectors, self.integration_method, compute_params);
            let locked_quaternions = smoothed_quaternions;
            smoothed_quaternions = alg.smooth_with_original(&locked_quaternions, quats, duration_ms, compute_params);

            // "Only within trim range" replaces the motion outside of the trim ranges before smoothing, so it doesn't affect
            // the smoothing inside of them. The result outside of the ranges has nothing to do with the real motion then,
            // and the preview there looks like the shaky original, drifting. Those frames are not rendered, so use the
            // smoothing of the whole video for them
            if !compute_params.trim_ranges.is_empty() && alg.get_parameter("trim_range_only") > 0.5 {
                let trim_ranges = std::mem::take(&mut compute_params.trim_ranges);
                let whole_smoothed = alg.smooth_with_original(&locked_quaternions, quats, duration_ms, compute_params);
                compute_params.trim_ranges = trim_ranges;
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
            smoothed_quaternions = alg.smooth(&smoothed_quaternions, duration_ms, compute_params);
            horizon_lock.lock(&mut smoothed_quaternions, quats, gravity_vectors, self.use_gravity_vectors, self.integration_method, compute_params);
        }
        smoothed_quaternions
    }

    pub fn raw_imu<'a>(&'a self, file_metadata: &'a FileMetadata) -> &'a Vec<TimeIMU> {
        if !self.raw_imu.is_empty() { return &*self.raw_imu }
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
    /// Removes every sync point, and everything made of them, at once (whatever `prevent_recompute` holds back): the
    /// lookups read the segments' own points before the whole video's (`offset_at_video_timestamp_in`)
    pub fn clear_offsets(&mut self) {
        self.offsets.clear();
        self.offsets_adjusted.clear();
        self.offsets_linear.clear();
        self.offsets_by_segment.clear();
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
        // The sync points of a video cut from recordings are fitted segment by segment: each segment is of its own
        // moment, and its camera clock can be off the next one's. A point is of the segment the video shows at the time
        // it was measured at: its key (the motion data's time) plus its offset, the video's time over the frame rate
        // scale (`offsets_adjusted`), as the segments' starts are (`SegmentRanges::video_starts_us`)
        self.offsets_by_segment = match &self.segments {
            Some(segments) => {
                let mut by_segment = vec![(BTreeMap::new(), BTreeMap::new()); segments.len()];
                for (k, v) in &self.offsets {
                    let video_us = *k + (*v * 1000.0).round() as i64;
                    let r = segments.of_video_us(video_us);
                    by_segment[r].0.insert(*k, *v);
                    by_segment[r].1.insert(video_us, *v);
                }
                by_segment
            }
            None => Vec::new(),
        };
        self.offsets_linear = if self.offsets_by_segment.is_empty() {
            Self::fit_offsets(&self.offsets)
        } else {
            self.offsets_by_segment.iter().flat_map(|(offsets, _)| Self::fit_offsets(offsets)).collect()
        };

        self.offsets_adjusted = self.offsets.iter().map(|(k, v)| (*k + (*v * 1000.0).round() as i64, *v)).collect::<BTreeMap<i64, f64>>();
    }

    /// The line fit of sync points (`offsets_linear`): the line through the most points within 5 ms of it
    fn fit_offsets(offsets: &BTreeMap<i64, f64>) -> BTreeMap<i64, f64> {
        let mut offsets_linear = BTreeMap::new();
        // Calculate line fit
        if offsets.len() > 1 {
            let len = offsets.len();
            let keys: Vec<i64> = offsets.keys().copied().collect();

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
                        let i_offset = offsets.get(&keys[i]).unwrap();
                        let j_offset = offsets.get(&keys[j]).unwrap();
                        let slope = (j_offset - i_offset) / (keys[j] - keys[i]) as f64;
                        let intersect = i_offset - keys[i] as f64 * slope;

                        let within_error: BTreeMap<i64, f64> = offsets.iter().filter_map(|(k, v)| {
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

            if !best.offsets.is_empty() {
                for (k, _) in offsets {
                    let fitted = *k as f64 * best.coeffs[0] + best.coeffs[1];
                    offsets_linear.insert(*k, fitted);
                }
            } else {
                if let Some(solution) = Self::line_fit(offsets) {
                    for (k, _) in offsets {
                        let fitted = *k as f64 * solution[0] + solution[1];
                        offsets_linear.insert(*k, fitted);
                    }
                }
            }
        } else {
            offsets_linear = offsets.clone();
        }
        offsets_linear
    }

    pub fn apply_transforms(&mut self) {
        let file_metadata = self.file_metadata.read();
        // In a video cut from recordings, the values interpolated across a part boundary as the parts are placed now
        // (`SegmentRanges::cut_values`): the reading's part has no reading of that sensor on that side
        let cut = self.segments.as_deref().map(|s| s.cut_values.as_slice()).unwrap_or_default();

        // Made anew, never changed in place: the readings there were may be shared with a copy of the motion data
        if self.imu_transforms.has_any() || !cut.is_empty() {
            let mut raw_imu = file_metadata.raw_imu.clone();
            for (key, sensor) in cut {
                let from = raw_imu.partition_point(|x| motion_key(x.timestamp_ms) < *key);
                for x in raw_imu[from..].iter_mut().take_while(|x| motion_key(x.timestamp_ms) == *key) {
                    match sensor {
                        telemetry_parser::util::ImuSensor::Accelerometer => x.accl = None,
                        telemetry_parser::util::ImuSensor::Magnetometer => x.magn = None,
                    }
                }
            }
            if self.imu_transforms.has_any() {
                for x in raw_imu.iter_mut() {
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
                // Filtered segment by segment in a video cut from recordings (`integrate`): no filter state crosses a boundary
                let whole_len = file_metadata.raw_imu.len();
                let pieces = split_ranges(self.segments.as_deref());
                let durations: Vec<f64> = pieces.into_iter().flatten().enumerate().map(|(r, _)| self.piece_duration_ms(Some(r))).collect();
                let whole_duration = self.duration_ms;
                for (piece, r) in imu_pieces(&mut raw_imu, pieces) {
                    let (len, duration_ms) = match r { None => (whole_len, whole_duration), Some(r) => (piece.len(), durations[r]) };
                    if self.imu_transforms.imu_lpf > 0.0 && len > 0 && duration_ms > 0.0 {
                        let sample_rate = len as f64 / (duration_ms / 1000.0);
                        if let Err(e) = super::filtering::Lowpass::filter_gyro_forward_backward(self.imu_transforms.imu_lpf, sample_rate, piece) {
                            log::error!("Filter error {:?}", e);
                        }
                    }
                    if self.imu_transforms.imu_mf > 0 && len > 0 && duration_ms > 0.0 {
                        let sample_rate = len as f64 / (duration_ms / 1000.0);
                        super::filtering::Median::filter_gyro_forward_backward(self.imu_transforms.imu_mf, sample_rate, piece);
                    }
                }
            }
            self.raw_imu = Arc::new(raw_imu);
        } else {
            self.raw_imu = Default::default();
        }

        drop(file_metadata);

        self.integrate();
    }

    /// Integrates the shared motion data `gyro` again (`integration`) on a copy, with the lock released, and puts the
    /// result in under a short write: the integration of a long high-rate log takes seconds, and every reader of the
    /// motion data (the preview's frames, the UI) would wait for it, behind the write lock or a writer queued after a
    /// reader. When what it integrates from changed meanwhile (`same_integration_inputs`: another file, filter,
    /// integration method, correction...), it's integrated again from a new copy: what's put in is always the integration
    /// of the motion data there is then, as an integration under the lock would have it
    pub fn integrate_shared(gyro: &RwLock<GyroSource>, integration: Integration) {
        // Copies a change made stale before it's integrated under the lock, as it was before this: a change that keeps
        // coming (another thread setting the motion data over and over) would otherwise integrate it again for as long
        // as it comes, seconds each time, on the caller's thread (the UI's, `StabilizationManager::recompute_gyro`)
        const ON_A_COPY: usize = 3;
        let integrate = |g: &mut GyroSource| match integration {
            Integration::ApplyTransforms => g.apply_transforms(),
            Integration::Integrate => g.integrate(),
        };
        for _ in 0..ON_A_COPY {
            // Shares the readings, the orientations and the metadata (`Arc`s): only the settings are copied
            let mut copy = gyro.read().clone();
            integrate(&mut copy);
            let mut gyro = gyro.write();
            if gyro.same_integration_inputs(&copy, integration) {
                gyro.take_integration(&mut copy);
                // What it replaced (now in `copy`) is freed with the lock released: a long clip's orientations take
                // milliseconds
                drop(gyro);
                return;
            }
        }
        log::debug!("The motion data changed under {ON_A_COPY} integrations of a copy of it, integrating it under the lock");
        integrate(&mut gyro.write());
    }
    /// Whether `self` integrates (`integration`) to what `copy`, a copy of it, did: everything `apply_transforms`,
    /// `integrate` and `apply_optical_correction` read is the same. The shared parts are compared by identity, which is
    /// exact while `copy` holds them: anything changing one of `self`'s replaces it (`Arc::make_mut` copies a shared one)
    fn same_integration_inputs(&self, copy: &Self, integration: Integration) -> bool {
        fn same<T>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (a, b) => a.is_none() && b.is_none(),
            }
        }
        Arc::ptr_eq(&self.file_metadata.0, &copy.file_metadata.0)
            && same(&self.segments, &copy.segments)
            && same(&self.optical_correction, &copy.optical_correction)
            && self.optical_context == copy.optical_context
            && self.imu_transforms == copy.imu_transforms
            && self.integration_method == copy.integration_method
            && self.horizon_lock_integration_method == copy.horizon_lock_integration_method
            && self.use_gravity_vectors == copy.use_gravity_vectors
            && self.duration_ms.to_bits() == copy.duration_ms.to_bits()
            // `integrate` reads the filtered readings there are, `apply_transforms` filters them anew
            && (integration == Integration::ApplyTransforms || Arc::ptr_eq(&self.raw_imu, &copy.raw_imu))
    }
    /// Takes in what `integrate_shared`'s copy integrated: all that `apply_transforms` and `integrate` set. The readings
    /// and orientations it replaces go to `from`
    fn take_integration(&mut self, from: &mut Self) {
        std::mem::swap(&mut self.raw_imu, &mut from.raw_imu);
        std::mem::swap(&mut self.quaternions, &mut from.quaternions);
        self.optical_correction_applied = from.optical_correction_applied;
        self.optical_uncorrected_checksum = from.optical_uncorrected_checksum;
        self.motion_from_video = from.motion_from_video;
    }

    /// The orientation at the video's time `timestamp_ms`, from `quats`, of the motion data `lookup` may read: its time
    /// in the motion data is the video's less the sync offset there, and it's clamped to that data (no orientation is
    /// interpolated across a segment boundary of a video cut from recordings). The identity where it has none
    fn quat_at_timestamp(&self, quats: &TimeQuat, timestamp_ms: f64, lookup: &MotionLookup) -> Quat64 {
        if quats.len() < 2 || self.duration_ms <= 0.0 { return Quat64::identity(); }
        let motion_ms = timestamp_ms - self.offset_at_video_timestamp_in(timestamp_ms, lookup.segment);
        self.quat_in(quats, motion_ms, &lookup.motion, None)
    }
    /// `quat_at_timestamp` of the motion data the video's time `timestamp_ms` alone may read (`lookup_for_video_time`),
    /// with the segment's sync offset and the bounds of its orientations found once, not for the lookup and again for
    /// the orientation
    fn quat_at_video_time(&self, quats: &TimeQuat, timestamp_ms: f64) -> Quat64 {
        if quats.len() < 2 || self.duration_ms <= 0.0 { return Quat64::identity(); }
        let Some(s) = self.segments.as_deref() else { return self.quat_at_timestamp(quats, timestamp_ms, &MotionLookup::ALL) };
        let r = s.of_video_us((timestamp_ms * 1000.0).round() as i64);
        let motion_ms = timestamp_ms - self.offset_at_video_timestamp_in(timestamp_ms, Some(r));
        let (motion, bounds) = self.part_motion(s, r, motion_ms * 1000.0, Some(timestamp_ms));
        self.quat_in(quats, motion_ms, &motion, bounds)
    }
    /// The orientation from `quats` at the motion data's time `motion_ms`, held within its timestamps `motion`: `bounds`
    /// are the first and the last of them, when they're known for `quaternions`
    fn quat_in(&self, quats: &TimeQuat, motion_ms: f64, motion: &std::ops::Range<i64>, bounds: Option<(i64, i64)>) -> Quat64 {
        let bounds = bounds.filter(|_| std::ptr::eq(quats, &*self.quaternions)).or_else(|| Self::bounds(quats, motion));
        let Some((first_ts, last_ts)) = bounds else { return Quat64::identity() };
        let lookup_ts = ((motion_ms * 1000.0).round() as i64).min(last_ts).max(first_ts);

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
        Quat64::identity()
    }

    pub fn      org_quat_at_timestamp(&self, timestamp_ms: f64) -> Quat64 { self.quat_at_video_time(&self.quaternions,          timestamp_ms) }
    pub fn smoothed_quat_at_timestamp(&self, timestamp_ms: f64) -> Quat64 { self.quat_at_video_time(&self.smoothed_quaternions, timestamp_ms) }
    /// The orientation at the motion data's own time `motion_ms` (no sync offset taken off it), as a frame of the part
    /// `segment` of a video cut from recordings reads it there (`lookup_for_frame`): of the part's motion data, held at
    /// its ends, or of all of it where the part's doesn't reach that time (`segment_lookup`); of all of it for `None`
    /// and without segments. What the synchronization compares a picture of the part against, at the offsets it tries
    pub fn org_quat_at_motion_time(&self, motion_ms: f64, segment: Option<usize>) -> Quat64 {
        if self.quaternions.len() < 2 || self.duration_ms <= 0.0 { return Quat64::identity(); }
        let (motion, bounds) = match (self.segments.as_deref(), segment) {
            (Some(s), Some(r)) if r < s.len() => self.part_motion(s, r, motion_ms * 1000.0, None),
            _ => (MotionLookup::ALL.motion, None),
        };
        self.quat_in(&self.quaternions, motion_ms, &motion, bounds)
    }
    /// The part of a video cut from recordings the video's frame `frame` is of (`segment_ranges`), `None` without
    pub fn part_of_frame(&self, frame: usize) -> Option<usize> {
        self.segments.as_ref().map(|s| s.of_frame(frame))
    }
    /// `org_quat_at_timestamp` of a frame's time: of the motion data of the frame's segment (`lookup_for_frame`), at any
    /// time the frame's rows are read at
    pub fn      org_quat_at(&self, timestamp_ms: f64, lookup: &MotionLookup) -> Quat64 { self.quat_at_timestamp(&self.quaternions,          timestamp_ms, lookup) }
    pub fn smoothed_quat_at(&self, timestamp_ms: f64, lookup: &MotionLookup) -> Quat64 { self.quat_at_timestamp(&self.smoothed_quaternions, timestamp_ms, lookup) }
    /// The orientation at the video's time `timestamp_ms` read out in the video's frame `frame` (a row of it: the frame's
    /// time, its per-frame offset and the row's place in the readout), of the motion data of the frame's own part of a
    /// video cut from recordings, with that part's sync offset (`lookup_for_frame`), as the render looks a frame up. A
    /// lookup by the time alone (`org_quat_at_timestamp`) takes the part the video presents then, and a frame is read
    /// out after it's presented, on some cameras by more than a frame (Sony's per-frame offset: 54 ms at 23.976 fps), so
    /// the last frame of a part would read the next part's motion
    pub fn      org_quat_at_frame(&self, frame: usize, timestamp_ms: f64) -> Quat64 { self.org_quat_at(timestamp_ms, &self.lookup_for_frame(frame)) }
    pub fn smoothed_quat_at_frame(&self, frame: usize, timestamp_ms: f64) -> Quat64 { self.smoothed_quat_at(timestamp_ms, &self.lookup_for_frame(frame)) }
    /// The sync offset at the video's time `timestamp_ms` read out in frame `frame`: of the frame's own part
    /// (`org_quat_at_frame`)
    pub fn offset_at_frame(&self, frame: usize, timestamp_ms: f64) -> f64 { self.offset_at_video_timestamp_in(timestamp_ms, self.part_of_frame(frame)) }
    /// The orientations frame `frame` may be looked up in (`lookup_for_frame`): its own part's in a video cut from
    /// recordings, keyed by the motion data's time (µs), all of them otherwise
    pub fn quaternions_of_frame(&self, frame: usize) -> std::collections::btree_map::Range<'_, i64, Quat64> { self.quaternions.range(self.lookup_for_frame(frame).motion) }

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
    /// The sync offset at the video's time `timestamp_ms`. In a video cut from recordings, a segment's own sync points
    /// give its offsets (one segment's clock can be off the next one's); a segment without any takes the offsets of the
    /// whole video there
    pub fn offset_at_video_timestamp(&self, timestamp_ms: f64) -> f64 {
        self.offset_at_video_timestamp_in(timestamp_ms, self.segments.as_ref().map(|s| s.of_video_us((timestamp_ms * 1000.0).round() as i64)))
    }
    /// `offset_at_video_timestamp` of the segment `segment` (`MotionLookup::segment`)
    pub fn offset_at_video_timestamp_in(&self, timestamp_ms: f64, segment: Option<usize>) -> f64 {
        match segment.and_then(|r| self.offsets_by_segment.get(r)).filter(|x| !x.1.is_empty()) {
            Some((_, adjusted)) => Self::offset_at_timestamp(adjusted, timestamp_ms),
            None => Self::offset_at_timestamp(&self.offsets_adjusted, timestamp_ms),
        }
    }
    /// The sync offset at the motion data's time `timestamp_ms`, of its segment's sync points in a video cut from
    /// recordings (`offset_at_video_timestamp`)
    pub fn offset_at_gyro_timestamp (&self, timestamp_ms: f64) -> f64 {
        // The motion data's time tells its part, but where it's one piece every part's (`SegmentRanges::whole`)
        let segment = self.segments.as_ref().filter(|s| !s.whole).map(|s| s.of_motion_us(motion_key(timestamp_ms)));
        match segment.and_then(|r| self.offsets_by_segment.get(r)).filter(|x| !x.0.is_empty()) {
            Some((offsets, _)) => Self::offset_at_timestamp(offsets, timestamp_ms),
            None => Self::offset_at_timestamp(&self.offsets, timestamp_ms),
        }
    }

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
        // Where the segments' motion data is, at the frame rate scale (`refresh_segments`)
        if let Some(segments) = &self.segments { segments.hash_into(&mut hasher); }
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

/// What a sample of metadata states of the lens (`read_lens_tags`), to put into the lens values before it (`apply`). Read
/// once, and put into as many as need it: the running values, and in a video of more than one part, its part's
#[derive(Default)]
struct LensTags {
    /// The values the sample gives
    given: LensParams,
    /// The sample's pixel quantities can't be put into the video's (`FrameMapping::Unusable`): no pixel focal length
    /// holds for its frame, not the one before it either
    pixel_focal_length_unusable: bool,
    /// The pixel focal length of Sony's lens distortion metadata, of a sample without a focal length of its own: it
    /// counts where there's no focal length before it either
    distortion_pixel_focal_length: Option<(f32, f32)>,
}
impl LensTags {
    /// Puts the values into `lens`, which keeps the ones the sample doesn't have. Field by field, every one named, so a
    /// field `LensParams` gains is put in too
    fn apply(&self, lens: &mut LensParams) {
        fn set<T: Clone>(to: &mut Option<T>, from: &Option<T>) { if from.is_some() { to.clone_from(from); } }
        let LensParams { focal_length, pixel_pitch, sensor_size_px, capture_area_origin, capture_area_size, pixel_focal_length, principal_point,
                         distortion_coefficients, focus_distance, iris_fstop, iris_tstop, zoom_ring_position, readout_span_rows } = &self.given;
        set(&mut lens.focal_length,        focal_length);
        set(&mut lens.pixel_pitch,         pixel_pitch);
        set(&mut lens.sensor_size_px,      sensor_size_px);
        set(&mut lens.capture_area_origin, capture_area_origin);
        set(&mut lens.capture_area_size,   capture_area_size);
        if self.pixel_focal_length_unusable { lens.pixel_focal_length = None; }
        set(&mut lens.pixel_focal_length,  pixel_focal_length);
        set(&mut lens.principal_point,     principal_point);
        if !distortion_coefficients.is_empty() { lens.distortion_coefficients.clone_from(distortion_coefficients); }
        set(&mut lens.focus_distance,      focus_distance);
        set(&mut lens.iris_fstop,          iris_fstop);
        set(&mut lens.iris_tstop,          iris_tstop);
        set(&mut lens.zoom_ring_position,  zoom_ring_position);
        set(&mut lens.readout_span_rows,   readout_span_rows);
        if lens.focal_length.is_none() && let Some(v) = self.distortion_pixel_focal_length {
            lens.pixel_focal_length = Some(v);
        }
    }
}

/// The lens values of a metadata sample (`tag_map`), `LensTags`. Also the readout direction, the lens profile and the
/// focal length at `timestamp_us` it gives
#[allow(clippy::too_many_arguments)]
fn read_lens_tags(tag_map: &GroupedTagMap, frame_mapping: FrameMapping, size: (usize, usize), is_red: bool, is_gyroflow_proto: bool,
                  frame_readout_direction_tag: &mut Option<i32>, lens_profile: &mut Option<serde_json::Value>, lens_positions: &mut BTreeMap<i64, f64>, timestamp_us: i64) -> LensTags {
    let mut tags = LensTags::default();
    let lens_info = &mut tags.given;
    if let Some(im) = tag_map.get(&GroupId::Imager) {
        if is_red {
            lens_info.capture_area_size = Some((size.0 as f32, size.1 as f32));
        }
        if let Some(v) = im.get_t(TagId::FrameReadoutDirection) as Option<&i32> { *frame_readout_direction_tag = Some(*v); }
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
            *lens_profile = Some(v.clone());
        }
        if let Some(v) = map.get_t(TagId::Name) as Option<&String> {
            *lens_profile = Some(serde_json::Value::String(v.clone()));
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
        tags.pixel_focal_length_unusable = !pfl_valid;
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
                tags.distortion_pixel_focal_length = Some((pfl, pfl));
            }
        }
    }
    tags
}

/// The lens values at the start of a segment of a video cut from recordings: the ones before it (`carried`) of the
/// fields its own metadata never gives (`given`: all of its samples' lens values, `LensTags`), which are the
/// file's (the clip-wide values some formats put into the file's first sample), and none of those it gives: until its
/// metadata gives them, its first frames have none, like a recording's first frames, rather than another moment's. Field
/// by field, every one named, so a field `LensParams` gains is decided here too
fn lens_at_segment_start(carried: &LensParams, given: &LensParams) -> LensParams {
    LensParams {
        focal_length:            carried.focal_length       .filter(|_| given.focal_length.is_none()),
        pixel_pitch:             carried.pixel_pitch        .filter(|_| given.pixel_pitch.is_none()),
        sensor_size_px:          carried.sensor_size_px     .filter(|_| given.sensor_size_px.is_none()),
        capture_area_origin:     carried.capture_area_origin.filter(|_| given.capture_area_origin.is_none()),
        capture_area_size:       carried.capture_area_size  .filter(|_| given.capture_area_size.is_none()),
        pixel_focal_length:      carried.pixel_focal_length .filter(|_| given.pixel_focal_length.is_none()),
        principal_point:         carried.principal_point    .filter(|_| given.principal_point.is_none()),
        distortion_coefficients: if given.distortion_coefficients.is_empty() { carried.distortion_coefficients.clone() } else { Vec::new() },
        focus_distance:          carried.focus_distance     .filter(|_| given.focus_distance.is_none()),
        iris_fstop:              carried.iris_fstop         .filter(|_| given.iris_fstop.is_none()),
        iris_tstop:              carried.iris_tstop         .filter(|_| given.iris_tstop.is_none()),
        zoom_ring_position:      carried.zoom_ring_position .filter(|_| given.zoom_ring_position.is_none()),
        readout_span_rows:       carried.readout_span_rows  .filter(|_| given.readout_span_rows.is_none()),
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
/// camera clock to the middle of its exposure and readout. The middle of the rows the video shows, which is the frame's
/// unless the video is the file's other picture (`FrameMapping`).
///
/// The start is `FirstFrameTimestamp` after the time of the frame's message (`message_ms`, the sample's time, which
/// telemetry-parser takes it against): the frame's ideal time in the video, its index over the file's frame rate, in a
/// metadata track (`gyroflow_proto_inject`) as in the JSONL, the camera clock's jitter being in `FirstFrameTimestamp`.
/// The video presents the frame at another time (`presented_ms`, in the file's time): its presentation time where its
/// frames aren't evenly spaced (D10), with that jitter in it too where it's the frame's capture, or `k / fps`, which is
/// the ideal time only where the video's rate is the file's frame rate and telemetry-parser times the sample exactly
/// (it truncates it to whole µs). The offset is taken against the time the frame is presented at, so that every frame
/// is looked up where its message puts it on the camera clock, however the video times its frames
fn gyroflow_proto_time_offset(tag_map: &GroupedTagMap, mapping: &FrameMapping, direction: ReadoutDirection, message_ms: f64, presented_ms: Option<f64>) -> Option<f64> {
    let imager = tag_map.get(&GroupId::Imager)?;
    let first_frame_ts = *(imager.get_t(TagId::FirstFrameTimestamp) as Option<&f64>)?;
    let exposure_time = *(imager.get_t(TagId::ExposureTime)     as Option<&f64>).unwrap_or(&0.0);
    let readout_time  = *(imager.get_t(TagId::FrameReadoutTime) as Option<&f64>).unwrap_or(&0.0);
    let against_presented = presented_ms.map_or(0.0, |presented| message_ms - presented);

    Some(first_frame_ts + against_presented - exposure_time / 2.0 + readout_time * (0.5 + mapping.centre_shift(direction)))
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

#[cfg(test)]
mod lens_tags_tests {
    use super::*;
    use telemetry_parser::tag;
    use telemetry_parser::tags_impl::*;

    /// A sample with the lens values given, and with `distortion` Sony's lens distortion metadata of a 24 mm lens on a
    /// 16 mm high sensor
    fn sample(focal_length: Option<f32>, pixel_focal_length: Option<(f32, f32)>, distortion: bool) -> GroupedTagMap {
        let mut map = GroupedTagMap::new();
        let options = InputOptions::default();
        if let Some(v) = focal_length {
            util::insert_tag(&mut map, tag!(parsed GroupId::Lens, TagId::FocalLength, "", f32, |v| format!("{v}"), v, vec![]), &options);
        }
        if let Some(v) = pixel_focal_length {
            util::insert_tag(&mut map, tag!(parsed GroupId::Lens, TagId::PixelFocalLength, "", f32x2, |v| format!("{v:?}"), v, vec![]), &options);
        }
        if distortion {
            let data = serde_json::json!({ "focal_length_nm": 24e6, "effective_sensor_height_nm": 16e6 });
            util::insert_tag(&mut map, tag!(parsed GroupId::Custom("LensDistortion".into()), TagId::Data, "", Json, |v| format!("{v:?}"), data, vec![]), &options);
        }
        map
    }
    fn read(tag_map: &GroupedTagMap, mapping: FrameMapping) -> LensTags {
        read_lens_tags(tag_map, mapping, (1920, 1080), false, false, &mut None, &mut None, &mut BTreeMap::new(), 0)
    }

    #[test]
    fn a_sample_keeps_the_values_it_does_not_give() {
        let mut lens = LensParams { focal_length: Some(35.0), iris_fstop: Some(2.8), pixel_focal_length: Some((1000.0, 1000.0)), ..Default::default() };
        read(&sample(None, Some((1200.0, 1200.0)), false), FrameMapping::AsIs).apply(&mut lens);
        assert_eq!((lens.focal_length, lens.iris_fstop, lens.pixel_focal_length), (Some(35.0), Some(2.8), Some((1200.0, 1200.0))));
    }

    #[test]
    fn a_frame_the_video_cant_show_drops_the_pixel_focal_length_before_it() {
        let mut lens = LensParams { pixel_focal_length: Some((1000.0, 1000.0)), ..Default::default() };
        read(&sample(Some(24.0), Some((1200.0, 1200.0)), false), FrameMapping::Unusable).apply(&mut lens);
        assert_eq!((lens.focal_length, lens.pixel_focal_length), (Some(24.0), None));
    }

    #[test]
    fn the_lens_distortion_focal_length_counts_only_without_a_focal_length() {
        // 24 mm on a 16 mm high sensor: 1.5 picture heights
        let tags = read(&sample(None, None, true), FrameMapping::AsIs);
        let mut alone = LensParams::default();
        tags.apply(&mut alone);
        assert_eq!(alone.pixel_focal_length, Some((1620.0, 1620.0)));
        // A focal length before the sample (one carried along, a clip-wide one) keeps it out, as the sample's own does:
        // the same sample read once gives what reading it into each of them gave
        let mut carried = LensParams { focal_length: Some(24.0), ..Default::default() };
        tags.apply(&mut carried);
        assert_eq!(carried.pixel_focal_length, None);
        let mut own = LensParams::default();
        read(&sample(Some(24.0), None, true), FrameMapping::AsIs).apply(&mut own);
        assert_eq!(own.pixel_focal_length, None);
    }
}
