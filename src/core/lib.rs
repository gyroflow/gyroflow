// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>
#![recursion_limit = "256"]

pub mod gyro_source;
pub mod imu_integration;
pub mod lens_profile;
pub mod lens_profile_database;
pub mod lensfun;
pub mod calibration;
pub mod synchronization;
pub mod stabilization;
pub mod camera_identifier;
pub mod keyframes;
pub mod stmap;

pub mod zooming;
pub mod smoothing;
pub mod filtering;
pub mod filesystem;
pub mod gyro_export;
pub mod settings;

pub mod gpu;

pub mod util;
pub mod stabilization_params;
pub mod frame_times;

use std::sync::{ Arc, atomic::{ AtomicU64, AtomicBool, Ordering::SeqCst } };
use std::collections::{ BTreeMap, BTreeSet, HashSet };
use keyframes::*;
use parking_lot::{ RwLock, RwLockUpgradableReadGuard };
use nalgebra::Vector4;
use gyro_source::{ GyroSource, Integration, Quat64, TimeQuat, TimeVec };
use stabilization_params::{ ReadoutDirection, StabilizationParams };
use lens_profile::LensProfile;
use lens_profile_database::LensProfileDatabase;
use smoothing::Smoothing;
use stabilization::{ Stabilization, KernelParamsFlags, ComputeParams };
use camera_identifier::CameraIdentifier;
pub use stabilization::PixelType;
pub use wgpu::TextureFormat as WgpuTextureFormat;
use gpu::Buffers;
use gpu::drawing::*;

pub use telemetry_parser;
use std::io::{ Read, Seek };

#[cfg(feature = "opencv")]
use calibration::LensCalibrator;

lazy_static::lazy_static! {
    static ref THREAD_POOL: rayon::ThreadPool = rayon::ThreadPoolBuilder::new().build().unwrap();
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct InputFile {
    pub url: String,
    pub project_file_url: Option<String>,
    pub image_sequence_fps: f64,
    pub image_sequence_start: i32,
    pub preset_name: Option<String>,
    pub preset_output_size: Option<(usize, usize)>,
}

#[derive(Clone)]
pub struct SyncData {
    pub rank: Vec<f32>,
    pub ratio: f64,
}
impl Default for SyncData {
    fn default() -> Self {
        Self {
            rank: vec![],
            ratio: 0.016,
        }
    }
}

#[derive(Clone)]
pub struct StabilizationManager {
    pub gyro: Arc<RwLock<GyroSource>>,
    pub lens: Arc<RwLock<LensProfile>>,
    pub smoothing: Arc<RwLock<Smoothing>>,

    pub stabilization: Arc<RwLock<Stabilization>>,

    pub pose_estimator: Arc<synchronization::PoseEstimator>,
    #[cfg(feature = "opencv")]
    pub lens_calibrator: Arc<RwLock<Option<LensCalibrator>>>,

    pub current_compute_id: Arc<AtomicU64>,
    pub smoothing_checksum: Arc<AtomicU64>,
    pub zooming_checksum: Arc<AtomicU64>,
    pub prevent_recompute: Arc<AtomicBool>,
    pub smoothing_invalidated: Arc<AtomicBool>,
    pub zooming_invalidated: Arc<AtomicBool>,
    pub undistortion_invalidated: Arc<AtomicBool>,
    pub gpu_decoding: Arc<AtomicBool>,

    pub camera_id: Arc<RwLock<Option<CameraIdentifier>>>,
    pub lens_profile_db: Arc<RwLock<LensProfileDatabase>>,

    pub input_file: Arc<RwLock<InputFile>>,

    pub keyframes: Arc<RwLock<KeyframeManager>>,

    pub params: Arc<RwLock<StabilizationParams>>,

    pub sync_data: Arc<RwLock<SyncData>>,

    /// What the last "Analyze image optically" measured, kept so a change of the strength refits the correction without
    /// another pass over the video. In memory only: a project keeps just the fitted correction. Its lock also orders
    /// what may replace the correction: a finished analysis or refit, and `invalidate_optical_measurements`
    pub optical_measurements: Arc<RwLock<Option<Arc<synchronization::optical_motion::OpticalMeasurements>>>>,
    pub optical_settings: Arc<RwLock<gyro_source::OpticalCorrectionSettings>>,
    /// Moved on by everything that makes an analysis still running measure the wrong thing (another file, a project,
    /// Clear): it stops, and what it measured is dropped, see `set_optical_measurements`
    pub optical_generation: Arc<AtomicU64>,
    /// How the next analysis follows the points: the user's choice, kept from one file to the next
    pub optical_tracking: Arc<RwLock<gyro_source::TrackingMethod>>,
    /// Tracks the user took out of what "Analyze image optically" measured, by where they start: out of the analysis
    /// kept, and of every analysis after it that tracks them again (see `TrackStart`). Per file, kept in the project
    pub optical_removed_tracks: Arc<RwLock<BTreeSet<synchronization::optical_motion::TrackStart>>>,
}

impl Default for StabilizationManager {
    fn default() -> Self {
        util::init_telemetry_parser();
        Self {
            smoothing: Arc::new(RwLock::new(Smoothing::default())),

            params: Arc::new(RwLock::new(StabilizationParams::default())),

            stabilization: Arc::new(RwLock::new(Stabilization::default())),
            gyro: Arc::new(RwLock::new(GyroSource::new())),
            lens: Arc::new(RwLock::new(LensProfile::default())),

            current_compute_id: Arc::new(AtomicU64::new(0)),
            smoothing_checksum: Arc::new(AtomicU64::new(0)),
            zooming_checksum: Arc::new(AtomicU64::new(0)),
            prevent_recompute: Arc::new(AtomicBool::new(false)),
            smoothing_invalidated: Arc::new(AtomicBool::new(false)),
            zooming_invalidated: Arc::new(AtomicBool::new(false)),
            undistortion_invalidated: Arc::new(AtomicBool::new(false)),

            gpu_decoding: Arc::new(AtomicBool::new(settings::get_bool("gpudecode", true))),

            pose_estimator: Arc::new(synchronization::PoseEstimator::default()),

            lens_profile_db: Arc::new(RwLock::new(LensProfileDatabase::default())),

            input_file: Arc::new(RwLock::new(InputFile::default())),

            #[cfg(feature = "opencv")]
            lens_calibrator: Arc::new(RwLock::new(None)),

            keyframes: Arc::new(RwLock::new(KeyframeManager::new())),

            camera_id: Arc::new(RwLock::new(None)),

            sync_data: Arc::new(RwLock::new(SyncData::default())),

            optical_measurements: Arc::new(RwLock::new(None)),
            optical_settings: Arc::new(RwLock::new(Default::default())),
            optical_generation: Arc::new(AtomicU64::new(0)),
            optical_tracking: Arc::new(RwLock::new(Default::default())),
            optical_removed_tracks: Arc::new(RwLock::new(Default::default())),
        }
    }
}

/// Where loading motion data takes the video's frame times (`StabilizationParams::frame_times`) from
enum FrameTimesFrom {
    /// The file loaded: it's the video, or its own metadata (`GyroSource::all_metadata`)
    LoadedFile,
    /// The video at this url, the one the file's motion data is for
    Video(String),
    /// Neither: the caller sets them (a project import, from the project's video)
    Caller,
}

/// A video's timing as its file states it (`StabilizationManager::video_timing_of`)
#[derive(Default)]
struct VideoTiming {
    /// Its frame times (`StabilizationParams::frame_times`): `None` for a video whose frames are evenly spaced from its
    /// start, which `round(t · fps)` maps exactly
    frame_times: Option<Arc<frame_times::FrameTimes>>,
    /// The frame rate it presents its frames at, exactly as the file states it: its most common frame duration's
    /// (`VideoFrameTimes::nominal`)
    frame_rate: Option<f64>,
}

impl StabilizationManager {
    pub fn init_from_video_data(&self, duration_ms: f64, fps: f64, frame_count: usize, video_size: (usize, usize)) {
        {
            let mut params = self.params.write();
            params.fps = fps;
            params.frame_count = frame_count;
            params.duration_ms = duration_ms;
            params.size = video_size;
            // The new video's frame times are read with its motion data (`load_gyro_data`) or its project
            // (`import_gyroflow_data`)
            params.frame_times = None;
        }

        self.pose_estimator.sync_results.write().clear();
        self.keyframes.write().clear();
    }

    /// Loads the motion data of the file `url` (`stream`) for the video: the video's own (`is_main_video`: the video itself,
    /// or a file loaded with all of its metadata as the video's), or an external motion log for the video `input_file`.
    /// The video's frame times (`StabilizationParams::frame_times`) come with it: the file's when it's the video's own,
    /// otherwise the input file's, so a video whose motion is another file's (a render queue job with a gyro file, a
    /// project with a log) has them too
    pub fn load_gyro_data<T: Read + Seek, F: Fn(f64)>(&self, stream: &mut T, filesize: usize, url: &str, is_main_video: bool, options: &gyro_source::FileLoadOptions, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> std::result::Result<(), GyroflowCoreError> {
        let frame_times = if is_main_video { FrameTimesFrom::LoadedFile } else { FrameTimesFrom::Video(self.input_file.read().url.clone()) };
        self.load_motion_file(stream, filesize, url, is_main_video, options, progress_cb, cancel_flag, frame_times)
    }

    /// A video's timing from its file: its frame times (`StabilizationParams::frame_times`) and its frame rate, as the
    /// file states them
    fn video_timing_of<T: Read + Seek>(stream: &mut T, filesize: u64) -> VideoTiming {
        let Some(v) = telemetry_parser::segment_table::video_frame_times(stream, filesize) else { return VideoTiming::default() };
        let times = frame_times::FrameTimes::of_video(&v);
        if let Some(times) = &times {
            log::info!("The video's {} frames aren't evenly spaced: they're mapped by their presentation times", times.frames());
        }
        VideoTiming { frame_times: times.map(Arc::new), frame_rate: (v.timescale > 0 && v.nominal > 0).then(|| v.timescale as f64 / v.nominal as f64) }
    }
    /// `video_timing_of` the video at `url`, nothing too when it can't be opened (an image sequence, a file that isn't
    /// there)
    fn read_video_timing(url: &str) -> VideoTiming {
        if url.is_empty() { return VideoTiming::default(); }
        let Ok(mut file) = filesystem::open_file(url, false, false) else { return VideoTiming::default() };
        let filesize = file.size as u64;
        Self::video_timing_of(file.get_file(), filesize)
    }
    /// The frame times of the video at `url` (`read_video_timing`)
    fn read_frame_times(url: &str) -> Option<Arc<frame_times::FrameTimes>> {
        Self::read_video_timing(url).frame_times
    }

    /// The rate a video's motion data was captured at, when the video presents its frames at another one: an off-speed
    /// recording's, whose motion data is timed by its capture, so the video is put on the motion data's timeline at that
    /// rate (`override_video_fps`). `rate` is the one the motion data's file states (`FileMetadata::frame_rate`), `fps`
    /// the video's frame rate as its decoder gives it, and `exact_rate` the video's as its file states it, where both are
    /// exact ratios of the file:
    /// - Blackmagic RAW states the rate each frame was captured at (`sensor_rate`) and the clip's frame rate (its frame
    ///   duration) as ratios, so any difference is an off-speed clip, however small: 25 fps in a 24 fps clip is 4 % slow,
    ///   its frame 96, presented at 4 s, captured at 3.84 s. They're compared as the file has them, not with the decoder's
    ///   rate, a float a few of its last bits off the clip's (Blackmagic's SDK and gyroflow's player on it: 29.970032 for
    ///   30000/1001, the sensor rate of such a clip)
    /// - Otherwise a rate is another one when it's more than a frame per second from the video's
    pub fn capture_frame_rate(rate: Option<f64>, fps: f64, exact_rate: Option<f64>) -> Option<f64> {
        let rate = rate?;
        match exact_rate {
            Some(video) => (rate != video).then_some(rate),
            None => ((rate - fps).abs() > 1.0).then_some(rate),
        }
    }

    /// `load_gyro_data`, the video's frame times taken as `frame_times` says
    #[allow(clippy::too_many_arguments)]
    fn load_motion_file<T: Read + Seek, F: Fn(f64)>(&self, stream: &mut T, filesize: usize, url: &str, is_main_video: bool, options: &gyro_source::FileLoadOptions, progress_cb: F, cancel_flag: Arc<AtomicBool>, frame_times: FrameTimesFrom) -> std::result::Result<(), GyroflowCoreError> {
        {
            let params = self.params.read();
            let mut gyro = self.gyro.write();
            // Cleared first: the scale taken in moves no segments of the file before, so nothing is integrated again for it
            gyro.clear();
            gyro.init_from_params(&params);
            gyro.file_url = url.to_string();
            gyro.file_metadata = Default::default();
        }
        self.invalidate_optical_measurements();
        self.invalidate_smoothing();
        self.invalidate_zooming();

        let last_progress = std::cell::RefCell::new(std::time::Instant::now());
        let progress_cb = |p| {
            let now = std::time::Instant::now();
            if (now - *last_progress.borrow()).as_millis() > 100 {
                progress_cb(p);
                *last_progress.borrow_mut() = now;
            }
        };

        let (fps, size, frame_count) = {
            let params = self.params.read();
            (params.fps, params.size, params.frame_count)
        };
        // The per-frame tables are the video's
        let parse_options = gyro_source::FileLoadOptions { video_frames: options.video_frames.or((frame_count > 0).then_some(frame_count)), ..options.clone() };

        let timing = match frame_times {
            FrameTimesFrom::LoadedFile => Some(Self::video_timing_of(stream, filesize as u64)),
            FrameTimesFrom::Video(video_url) => Some(Self::read_video_timing(&video_url)),
            FrameTimesFrom::Caller => None,
        };
        // Stored only by a load that wasn't replaced: the next video's frame times are its own load's to set. Checked under
        // the lock `init_from_video_data` clears them under, and the controller cancels a load before the next video's
        // init, so a load cancelled while it read can't write over the next one's
        let store_frame_times = || if let Some(timing) = &timing {
            let mut params = self.params.write();
            if !cancel_flag.load(SeqCst) { params.frame_times = timing.frame_times.clone(); }
        };

        let cancel_flag2 = cancel_flag.clone();
        let mut md = match GyroSource::parse_telemetry_file(stream, filesize, &url, &parse_options, size, fps, progress_cb, cancel_flag2) {
            Ok(md) => md,
            Err(e) => {
                // A video without motion data still has its frame times
                store_frame_times();
                // A video without motion data of its own still has its segments, when it was cut from recordings: the
                // motion measured from it is handled segment by segment like a file's (`gyro_source::TrimSegments`)
                if is_main_video && !cancel_flag.load(SeqCst)
                    && let Some(Ok(table)) = telemetry_parser::segment_table::read(stream, filesize as u64) {
                    let md = gyro_source::FileMetadata { trim_segments: Some(gyro_source::TrimSegments::new(table, &[], &[])), ..Default::default() };
                    let integration = self.gyro.write().load_from_telemetry_unintegrated(md);
                    if let Some(integration) = integration { GyroSource::integrate_shared(&self.gyro, integration); }
                }
                return Err(e);
            }
        };
        // The parsers return what they read until then, but a cancelled load (eg. replaced by the next video) mustn't set any of it
        if cancel_flag.load(SeqCst) { return Ok(()); }
        store_frame_times();
        // The rate the file's motion data was captured at, when the video presents its frames at another one. A
        // Blackmagic RAW clip's rates are both exact (`capture_frame_rate`), the video's as its file states it (the clip's
        // own, or its proxy's when the clip is another video's motion data)
        let braw = filesystem::get_filename(url).to_ascii_lowercase().ends_with(".braw");
        let exact_rate = timing.as_ref().and_then(|t| t.frame_rate).filter(|_| braw);
        let capture_rate = Self::capture_frame_rate(md.frame_rate, self.params.read().fps, exact_rate);
        if md.detected_source.as_ref().map(|v| v.starts_with("GoPro ")).unwrap_or_default() {
            // If gopro reports rolling shutter value, it already applied it, ie. the video is already corrected
            md.frame_readout_time = None;
        }

        // The profile of the database the file names (a .gcsv `lensprofile`, a Gyroflow protobuf `lens_profile`), set up
        // for the video at the end, once the file's camera is known
        let mut referenced_lens = None;
        if is_main_video {
            if let Some(ref lens) = md.lens_profile {
                if let Some(lens_str) = lens.as_str() {
                    let mut db = self.lens_profile_db.read();
                    if !db.loaded {
                        drop(db);
                        {
                            let mut db = self.lens_profile_db.write();
                            db.load_all();
                        }
                        db = self.lens_profile_db.read();
                    }
                    referenced_lens = db.find(lens_str).cloned();
                } else if lens.is_object() {
                    let mut l = self.lens.write();
                    l.load_from_json_value(lens);
                    l.path_to_file = filesystem::url_to_path(url);
                    let db = self.lens_profile_db.read();
                    l.resolve_interpolations(&db);
                }
            }
            if let Some(rate) = capture_rate {
                self.override_video_fps(rate, false);
            }
            self.params.write().frame_readout_direction = md.frame_readout_direction;
            if md.detected_source.as_ref().map(|v| v.starts_with("Blackmagic ")).unwrap_or_default() {
                if let Some(rot) = md.additional_data.get("rotation").and_then(|x| x.as_u64()) {
                    if rot == 90 || rot == 270 {
                        log::info!("Using horizontal rolling shutter correction");
                        if rot == 90 {
                            self.params.write().frame_readout_direction = ReadoutDirection::RightToLeft;
                            md.imu_orientation = Some("xYz".into());
                        } else {
                            self.params.write().frame_readout_direction = ReadoutDirection::LeftToRight;
                            md.imu_orientation = Some("Xyz".into());
                        }
                    }
                    if rot == 180 {
                        self.params.write().frame_readout_direction = ReadoutDirection::BottomToTop;
                        md.imu_orientation = Some("YXz".into());
                    }
                }
            }
            self.params.write().frame_readout_time = md.frame_readout_time.unwrap_or_default();
        } else {
            log::info!("Not a main video, clearing {} per-frame offsets", md.per_frame_time_offsets.len());
            md.per_frame_time_offsets.clear();
        }
        let camera_id = md.camera_identifier.clone();
        if !cancel_flag.load(SeqCst) {
            let integration = {
                let mut gyro = self.gyro.write();
                let integration = gyro.load_from_telemetry_unintegrated(md);
                gyro.file_load_options = options.clone();
                gyro.all_metadata = is_main_video;
                gyro.capture_frame_rate = capture_rate;
                integration
            };
            // With the lock released: every reader of the motion data would wait for the whole integration
            if let Some(integration) = integration { GyroSource::integrate_shared(&self.gyro, integration); }
        }

        if let Some(id) = camera_id {
            *self.camera_id.write() = Some(id);
        }
        if let Some(mut lens) = referenced_lens {
            // As `load_lens_profile` sets up every profile it loads: a profile gives the same lens whichever way it's
            // loaded, picked or named by the file
            let (size, fps, camera_id) = self.lens_video_context();
            lens.adapt_to_video(size, fps, &camera_id);
            *self.lens.write() = lens;
        }
        Ok(())
    }

    /// What a lens profile is set up for when it's loaded (`LensProfile::adapt_to_video`): the video's size and frame
    /// rate, and the autoload identifier of the camera of the motion data loaded last
    fn lens_video_context(&self) -> ((usize, usize), f64, String) {
        let params = self.params.read();
        (params.size, params.fps, self.camera_id.read().as_ref().map(|x| x.get_identifier_for_autoload()).unwrap_or_default())
    }

    pub fn load_lens_profile(&self, url: &str) -> Result<(), crate::GyroflowCoreError> {
        let url = if (url.starts_with('/') || url.starts_with('\\') || (url.len() > 3 && &url[1..2] == ":")) && !url.contains("://") && !url.starts_with('{') {
            crate::filesystem::path_to_url(url)
        } else {
            url.to_owned()
        };
        // The database's lock isn't held while the lens's is taken (the loads take them the other way round), nor over
        // the file read: a copy of its profile
        let in_db = self.lens_profile_db.read().get_by_id(&url).cloned();
        let (result, from_db) = if let Some(lens) = in_db {
            *self.lens.write() = lens;
            (Ok(()), true)
        } else if url.starts_with('{') {
            (self.lens.write().load_from_data(&url), false)
        } else {
            (self.lens.write().load_from_file(&url), false)
        };
        let (size, fps, camera_id) = self.lens_video_context();

        let mut lens = self.lens.write();
        lens.adapt_to_video(size, fps, &camera_id);
        if !from_db {
            lens.resolve_interpolations(&self.lens_profile_db.read());
        }
        result
    }

    pub fn init_size(&self) {
        let (w, h, ow, oh) = {
            let params = self.params.read();
            (params.size.0, params.size.1, params.output_size.0, params.output_size.1)
        };

        if w > 0 && ow > 0 && h > 0 && oh > 0 {
            self.stabilization.write().init_size((w, h), (ow, oh));
            self.lens.write().optimal_fov = None;

            self.invalidate_smoothing();
        }
    }

    pub fn set_size(&self, width: usize, height: usize) {
        {
            let mut params = self.params.write();
            params.size = (width, height);
        }
        self.init_size();
    }
    pub fn set_output_size(&self, width: usize, height: usize) -> bool {
        if width > 0 && height > 0 {
            let params = self.params.upgradable_read();

            let r = (params.video_rotation as f64).abs();
            let (ow, oh) = if r == 90.0 || r == 270.0 {
                (params.size.1, params.size.0)
            } else {
                params.size
            };

            let wp = width as f64;
            let hp = height as f64;

            let sw = ow as f64 / wp;
            let sh = oh as f64 / hp;
            let scale = sw.min(sh);

            let mut nw = (wp * scale).round() as usize;
            let mut nh = (hp * scale).round() as usize;

            if nw % 2 != 0 { nw -= 1; }
            if nh % 2 != 0 { nh -= 1; }

            let output_size = (nw, nh);

            if params.output_size != output_size {
                {
                    let mut params = RwLockUpgradableReadGuard::upgrade(params);
                    params.output_size = output_size;
                }
                self.init_size();

                return true;
            }
        }
        false
    }

    /// The focal length curves (`smoothing::focal_length`): the effective focal length the camera matrices are scaled to,
    /// and the target of the focal length smoothing and the lens breathing compensation. Before anything that projects
    /// with the lens: the crop lenses of `ComputeParams::calculate_camera_fovs`, the smoothing, the zoom
    fn apply_focal_length_smoothing(params: &mut ComputeParams, stabilization_params: &RwLock<StabilizationParams>) {
        let (enabled, max_zoom_rate, breathing_compensation) = {
            let sp = stabilization_params.read();
            (sp.focal_length_smoothing_enabled, sp.focal_length_max_zoom_rate, sp.lens_breathing_enabled)
        };
        // The base curve evaluates the projection for every frame (a third of a second per minute of 60 fps footage),
        // and a recompute is requested on every slider change, so it's reused while nothing it depends on changed:
        // the file and its lens metadata, the lens profile and the video geometry. The settings (the delay, the rate
        // limit, whether smoothing or the breathing compensation is on) and the trim only act on the curves derived from
        // it, in O(frames), so none of them is part of the key and a change of theirs never repeats the sweep. The
        // boundaries of the parts of a video cut from recordings are: the curve is made in each part on its own
        // (`focal_length::parts`)
        use crate::smoothing::focal_length::{ compute_base, derive_curves, parts, FocalLengthBase };
        let parts = parts(params);
        let base_key = {
            use std::hash::Hasher;
            let mut h = std::collections::hash_map::DefaultHasher::new();
            let gyro = params.gyro.read();
            let md = gyro.file_metadata.read();
            h.write(gyro.file_url.as_bytes());
            h.write_usize(md.lens_params.len());
            h.write_usize(md.lens_positions.len());
            h.write_usize(md.lens_breathing.len());
            h.write_u64(md.digital_zoom.unwrap_or_default().to_bits());
            h.write_u64(params.lens.get_checksum());
            h.write_usize(params.width);
            h.write_usize(params.height);
            h.write_u64(params.scaled_fps.to_bits());
            h.write_usize(params.frame_count);
            // Where each frame's lens is looked up: its time
            if let Some(times) = &params.frame_times { h.write_u64(times.checksum()); }
            for part in &parts { h.write_usize(part.start); }
            h.finish()
        };
        let derive = |base: &FocalLengthBase| derive_curves(base, &parts, params.lens_metadata_delay_frames, enabled, max_zoom_rate, params.scaled_fps, &params.trim_ranges, breathing_compensation);
        let cached = {
            let sp = stabilization_params.read();
            (sp.focal_length_base_key == base_key).then(|| derive(&sp.focal_length_base))
        };
        let (base, (focal_lengths, smoothed)) = match cached {
            Some(curves) => (None, curves),
            None => {
                let base = compute_base(params, &parts);
                let curves = derive(&base);
                (Some(base), curves)
            }
        };

        params.focal_length_smoothing_enabled = enabled && !smoothed.is_empty();
        params.focal_length_max_zoom_rate = max_zoom_rate;
        params.lens_breathing_enabled = breathing_compensation;
        params.focal_lengths = focal_lengths.clone();
        params.smoothed_focal_lengths = smoothed.clone();

        let mut sp = stabilization_params.write();
        sp.focal_lengths = focal_lengths;
        sp.smoothed_focal_lengths = smoothed;
        if let Some(base) = base {
            sp.focal_length_base = base;
            sp.focal_length_base_key = base_key;
        }
    }

    pub fn recompute_adaptive_zoom_static(compute_params: &ComputeParams, params: &RwLock<StabilizationParams>) -> (Vec<f64>, Vec<f64>, BTreeMap<i64, Vec<(f64, f64)>>, Vec<f64>) {
        let (timestamps, method) = {
            let params = params.read();
            ((0..params.frame_count).map(|i| (i, params.timestamp_at_frame(i as i32))).collect::<Vec<(usize, f64)>>(), params.adaptive_zoom_method)
        };

        zooming::calculate_fovs(compute_params, &timestamps, method.into())
    }
    /// One iteration of the zoom limit: lowers the smoothing of every frame zoomed in more than the limit allows, judged
    /// by `limit_fovs` (see `zooming::calculate_fovs`). Returns whether any frame was over the limit
    fn lower_smoothing_over_zoom_limit(params: &mut ComputeParams, limit_fovs: &[f64], iter: usize) -> bool {
        let thresholds = [0.95, 0.9, 0.85, 0.8];
        let threshold = *thresholds.get(iter).unwrap_or(thresholds.last().unwrap());
        let fov_limits = zooming::zoom_limit_fovs(params);
        let mut any_above_limit = false;
        for (i, fov) in limit_fovs.iter().enumerate() {
            let Some(fov_limit) = fov_limits.get(i).copied().flatten() else { continue; };
            if *fov < fov_limit {
                any_above_limit = true;
                if let Some(v) = params.smoothing_fov_limit_per_frame.get_mut(i) {
                    *v *= (*fov / fov_limit).min(threshold);
                }
            }
        }
        any_above_limit
    }
    pub fn recompute_adaptive_zoom(&self) {
        let mut params = stabilization::ComputeParams::from_manager(self);
        Self::apply_focal_length_smoothing(&mut params, &self.params);
        params.calculate_camera_fovs();

        let lens_fov_adjustment = params.lens.optimal_fov.unwrap_or(1.0);
        let (fovs, minimal_fovs, debug_points, mut limit_fovs) = Self::recompute_adaptive_zoom_static(&params, &self.params);
        params.fovs = fovs;
        params.minimal_fovs = minimal_fovs;

        {
            let mut stab_params = self.params.write();
            stab_params.set_fovs( params.fovs.clone(), lens_fov_adjustment);
            stab_params.minimal_fovs = params.minimal_fovs.clone();
            stab_params.zooming_debug_points = debug_points;
        }
        let max_zoom_iters = params.max_zoom_iterations;

        // Max zoom
        if zooming::zoom_limit_runs(&params) {
            params.smoothing_fov_limit_per_frame.clear();
            for _ in params.fovs.iter() {
                params.smoothing_fov_limit_per_frame.push(1.0);
            }
            for iter in 0..max_zoom_iters {
                let any_above_limit = Self::lower_smoothing_over_zoom_limit(&mut params, &limit_fovs, iter);
                log::debug!("Max zoom iteration {iter}/{max_zoom_iters}, any above limit: {any_above_limit}");
                if !any_above_limit {
                    if iter == 0 {
                        params.smoothing_fov_limit_per_frame.clear();
                    }
                    break;
                }

                // Smoothing
                self.smooth_blocking(&mut params);


                // Zooming
                let lens_fov_adjustment = params.lens.optimal_fov.unwrap_or(1.0);
                let (fovs, minimal_fovs, debug_points, new_limit_fovs) = Self::recompute_adaptive_zoom_static(&params, &self.params);
                params.fovs = fovs;
                params.minimal_fovs = minimal_fovs;
                limit_fovs = new_limit_fovs;
                {
                    let mut stab_params = self.params.write();
                    stab_params.set_fovs(params.fovs.clone(), lens_fov_adjustment);
                    stab_params.minimal_fovs = params.minimal_fovs.clone();
                    stab_params.zooming_debug_points = debug_points;
                }
            }
        }
    }

    pub fn recompute_smoothness(&self) {
        let mut params = stabilization::ComputeParams::from_manager(self);
        Self::apply_focal_length_smoothing(&mut params, &self.params);
        params.calculate_camera_fovs();

        self.smooth_blocking(&mut params);
    }
    /// Smooths the motion data (`GyroSource::recompute_smoothness`) and puts the result in. On a copy, with the lock
    /// released: the smoothing of a long clip takes seconds, and every reader of the motion data (the preview's frames)
    /// would wait for it behind the next writer. Smoothed again when the orientations were replaced meanwhile (another
    /// integration), so what's put in is always the smoothing of the orientations there are
    fn smooth_blocking(&self, params: &mut stabilization::ComputeParams) {
        // A copy of the algorithm, as `recompute_threaded` takes it: the smoothing's lock isn't held over the motion data's
        let (smoothing, horizon_lock) = {
            let lock = self.smoothing.read();
            (lock.current().clone(), lock.horizon_lock.clone())
        };
        loop {
            // Shares the orientations and the metadata (`Arc`s)
            let source = self.gyro.read().clone();
            let (quats, max_angles) = source.recompute_smoothness(smoothing.as_ref(), horizon_lock.clone(), params);
            let mut gyro = self.gyro.write();
            if !Arc::ptr_eq(&gyro.quaternions, &source.quaternions) { continue; }
            gyro.max_angles = max_angles;
            // The smoothing it replaces is freed with the lock released
            let _replaced = std::mem::replace(&mut gyro.smoothed_quaternions, Arc::new(quats));
            drop(gyro);
            return;
        }
    }

    pub fn recompute_undistortion(&self) {
        let params = stabilization::ComputeParams::from_manager(self);
        self.stabilization.write().set_compute_params(params);
    }

    pub fn recompute_blocking(&self) {
        self.refresh_optical_correction();
        self.recompute_smoothness();
        self.recompute_adaptive_zoom();
        self.recompute_undistortion();
    }

    pub fn invalidate_ongoing_computations(&self) {
        self.current_compute_id.store(fastrand::u64(..), SeqCst);
    }

    pub fn recompute_threaded<F: Fn((u64, bool)) + Send + Sync + Clone + 'static>(&self, cb: F) -> u64 {
        //self.recompute_smoothness();
        //self.recompute_adaptive_zoom();
        // Before the gyro checksum below, which says whether the correction is applied
        self.refresh_optical_correction();
        let mut params = stabilization::ComputeParams::from_manager(self);

        let smoothing = self.smoothing.clone();
        let stabilization_params = self.params.clone();
        let gyro = self.gyro.clone();

        let compute_id = fastrand::u64(..);
        self.current_compute_id.store(compute_id, SeqCst);

        let mut gyro_checksum = gyro.read().get_checksum();

        let prevent_recompute = self.prevent_recompute.clone();
        let current_compute_id = self.current_compute_id.clone();
        let smoothing_checksum = self.smoothing_checksum.clone();
        let zooming_checksum = self.zooming_checksum.clone();

        let stabilization = self.stabilization.clone();
        THREAD_POOL.spawn(move || {
            // std::thread::sleep(std::time::Duration::from_millis(20));
            if prevent_recompute.load(SeqCst) { return cb((compute_id, true)); } // we're still loading, don't recompute
            if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

            let commit = |checksum: &AtomicU64, value: u64| -> bool {
                checksum.store(value, SeqCst);
                if current_compute_id.load(SeqCst) != compute_id { checksum.store(0, SeqCst); return false; }
                true
            };

            // The focal length curves first: the camera matrices are scaled to them (the lens breathing is in them), and
            // the crop lenses the smoothing reads are made with those matrices. Here and not before spawning, as the
            // first sweep of a file projects every frame
            Self::apply_focal_length_smoothing(&mut params, &stabilization_params);
            params.calculate_camera_fovs();
            if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

            let mut smoothing_recomputed = false;
            if smoothing.read().get_state_checksum(gyro_checksum, &params) != smoothing_checksum.load(SeqCst) {
                let (mut smoothing, horizon_lock) = {
                    let lock = smoothing.read();
                    (lock.current().clone(), lock.horizon_lock.clone())
                };

                // On a copy, with the lock released (`smooth_blocking`). It shares the orientations and the metadata
                let source = gyro.read().clone();
                let (quats, max_angles) = source.recompute_smoothness(smoothing.as_mut(), horizon_lock, &mut params);

                if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

                // Only onto the motion data it smoothed: whatever changed it meanwhile asks for a recompute of its own. The
                // smoothing it replaces is freed with the lock released
                let replaced = {
                    let mut lib_gyro = gyro.write();
                    let same = gyro_checksum == lib_gyro.get_checksum() && Arc::ptr_eq(&lib_gyro.quaternions, &source.quaternions);
                    same.then(|| {
                        lib_gyro.max_angles = max_angles;
                        lib_gyro.smoothing_status = smoothing.get_status_json();
                        let replaced = std::mem::replace(&mut lib_gyro.smoothed_quaternions, Arc::new(quats));
                        gyro_checksum = lib_gyro.get_checksum();
                        replaced
                    })
                };
                if replaced.is_none() { return cb((compute_id, true)); }
                drop(replaced);
                smoothing_recomputed = true;
            }
            let smoothing_state = smoothing.read().get_state_checksum(gyro_checksum, &params);
            if smoothing_recomputed && !commit(&*smoothing_checksum, smoothing_state) { return cb((compute_id, true)); }

            // The zoom accounts for the focal length compensation, made above
            let zoom_key = zooming::get_checksum(&params, smoothing_state);
            // Freshly recomputed quaternions are the plain smoothing output, and the max zoom folds its limit back into
            // them inside the zoom pass, so a smoothing recompute needs the zoom pass again even when the zoom key still
            // matches the last commit: a run killed inside the max-zoom iterations leaves that key committed next to
            // quaternions that no longer carry the limit
            if smoothing_recomputed || zoom_key != zooming_checksum.load(SeqCst) {
                let (fovs, minimal_fovs, debug_points, mut limit_fovs) = Self::recompute_adaptive_zoom_static(&params, &stabilization_params);
                params.fovs = fovs;
                params.minimal_fovs = minimal_fovs;

                if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

                {
                    let mut stab_params = stabilization_params.write();
                    stab_params.set_fovs(params.fovs.clone(), params.lens.optimal_fov.unwrap_or(1.0));
                    stab_params.minimal_fovs = params.minimal_fovs.clone();
                    stab_params.zooming_debug_points = debug_points;
                }
                let max_zoom_iters = params.max_zoom_iterations;

                // Max zoom
                if zooming::zoom_limit_runs(&params) {
                    // The iterations rewrite the quaternions with the zoom limit folded in, and the fovs along with them.
                    // Until they finish, neither is the state its key describes, so a run killed in here has to redo both
                    smoothing_checksum.store(0, SeqCst);
                    zooming_checksum.store(0, SeqCst);

                    params.smoothing_fov_limit_per_frame.clear();
                    for _ in params.fovs.iter() {
                        params.smoothing_fov_limit_per_frame.push(1.0);
                    }
                    for iter in 0..max_zoom_iters {
                        let any_above_limit = Self::lower_smoothing_over_zoom_limit(&mut params, &limit_fovs, iter);
                        log::debug!("Max zoom iteration {iter}/{max_zoom_iters}, any above limit: {any_above_limit}");
                        if !any_above_limit {
                            if iter == 0 {
                                params.smoothing_fov_limit_per_frame.clear();
                            }
                            break;
                        }

                        if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

                        // Smoothing
                        let (mut smoothing, horizon_lock) = {
                            let lock = smoothing.read();
                            (lock.current().clone(), lock.horizon_lock.clone())
                        };
                        let source = gyro.read().clone();
                        let (quats, max_angles) = source.recompute_smoothness(smoothing.as_mut(), horizon_lock, &mut params);

                        if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

                        let replaced = {
                            let mut lib_gyro = gyro.write();
                            Arc::ptr_eq(&lib_gyro.quaternions, &source.quaternions).then(|| {
                                lib_gyro.max_angles = max_angles;
                                lib_gyro.smoothing_status = smoothing.get_status_json();
                                std::mem::replace(&mut lib_gyro.smoothed_quaternions, Arc::new(quats))
                            })
                        };
                        if replaced.is_none() { return cb((compute_id, true)); }
                        drop(replaced);

                        // The focal length curves don't change within max-zoom iterations:
                        // settings and raw metadata are the same, so the outer apply is reused

                        let (fovs, minimal_fovs, debug_points, new_limit_fovs) = Self::recompute_adaptive_zoom_static(&params, &stabilization_params);
                        params.fovs = fovs;
                        params.minimal_fovs = minimal_fovs;
                        limit_fovs = new_limit_fovs;

                        if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

                        {
                            let mut stab_params = stabilization_params.write();
                            stab_params.set_fovs(params.fovs.clone(), params.lens.optimal_fov.unwrap_or(1.0));
                            stab_params.minimal_fovs = params.minimal_fovs.clone();
                            stab_params.zooming_debug_points = debug_points;
                        }
                    }

                    // The quaternions are final again (zoom limit folded in), which is what the smoothing key describes
                    if !commit(&*smoothing_checksum, smoothing_state) { return cb((compute_id, true)); }
                }

                if !commit(&*zooming_checksum, zoom_key) { return cb((compute_id, true)); }
            }

            if current_compute_id.load(SeqCst) != compute_id { return cb((compute_id, true)); }

            stabilization.write().set_compute_params(params);

            cb((compute_id, false));
        });
        compute_id
    }

    pub fn get_features_pixels(&self, timestamp_us: i64, size: (usize, usize)) -> Option<Vec<(i32, i32)>> { // (x, y, alpha)
        let mut ret = None;
        use crate::util::MapClosest;
        use synchronization::OpticalFlowTrait;

        if let Some(l) = self.pose_estimator.sync_results.try_read() {
            if let Some(entry) = l.get_closest(&timestamp_us, 2000) { // closest within 2ms
                let ratio = size.1 as f32 / entry.frame_size.1.max(1) as f32;
                for pt in entry.of_method.features() {
                    if ret.is_none() {
                        // Only allocate if we actually have any points
                        ret = Some(Vec::with_capacity(2048));
                    }
                    ret.as_mut().unwrap().push(((pt.0 * ratio) as i32, (pt.1 * ratio) as i32));
                }
            }
        }
        ret
    }
    pub fn get_opticalflow_pixels(&self, timestamp_us: i64, num_frames: usize, size: (usize, usize)) -> Option<Vec<(i32, i32, usize)>> { // (x, y, alpha)
        let mut ret = None;
        for i in 0..num_frames {
            match self.pose_estimator.get_of_lines_for_timestamp(&timestamp_us, i, 1.0, 1, false) {
                (Some(lines), Some(frame_size)) => {
                    let ratio = size.1 as f32 / frame_size.1.max(1) as f32;
                    lines.0.1.into_iter().zip(lines.1.1.into_iter()).for_each(|(p1, p2)| {
                        if ret.is_none() {
                            // Only allocate if we actually have any points
                            ret = Some(Vec::with_capacity(2048));
                        }
                        let line = line_drawing::Bresenham::new(((p1.0 * ratio) as isize, (p1.1 * ratio) as isize), ((p2.0 * ratio) as isize, (p2.1 * ratio) as isize));
                        for point in line {
                            ret.as_mut().unwrap().push((point.0 as i32, point.1 as i32, i));
                        }
                    });
                }
                _ => { }
            }
        }
        ret
    }

    pub fn draw_overlays(&self, drawing: &mut DrawCanvas, timestamp_us: i64) {
        drawing.clear();

        if let Some(p) = self.params.try_read() {
            let y_inverted = p.framebuffer_inverted;
            let size = p.size;
            let frame = p.frame_at_timestamp(timestamp_us as f64 / 1000.0).max(0) as usize; // used only to draw features and OF

            if p.show_optical_flow {
                let num_frames = if p.of_method == 2 { 1 } else { 3 };
                if let Some(pxs) = self.get_opticalflow_pixels(timestamp_us, num_frames, size) {
                    for (x, y, a) in pxs {
                        let a = Alpha::from(a as u8);
                        drawing.put_pixel(x, y, Color::Yellow, a, Stage::OnInput, y_inverted, 1);
                    }
                }
            }
            if p.show_detected_features {
                if let Some(pxs) = self.get_features_pixels(timestamp_us, size) {
                    for (x, y) in pxs {
                        drawing.put_pixel(x, y, Color::Green, Alpha::Alpha100, Stage::OnInput, y_inverted, 3);
                    }
                }
            }
            #[cfg(feature = "opencv")]
            if p.is_calibrator {
                let lock = self.lens_calibrator.read();
                if let Some(ref cal) = *lock {
                    let points = cal.all_matches.read();
                    if let Some(entry) = points.get(&(frame as i32)) {
                        calibration::drawing::draw_chessboard_corners(cal.width, cal.height, p.size.0, p.size.1, drawing, (cal.columns, cal.rows), &entry.points, true, y_inverted);
                    }
                }
            }
            if !p.zooming_debug_points.is_empty() {
                if let Some((_, points)) = p.zooming_debug_points.range(timestamp_us - 1000..).next() {
                    for i in 0..points.len() {
                        // Same total zoom as FrameTransform::at_timestamp: the polygon is measured at fov = 1
                        let mut fov = ((p.fov + if p.fov_overview { 1.0 } else { 0.0 }) * p.fovs.get(frame).unwrap_or(&1.0)).max(0.0001);
                        fov *= p.size.0 as f64 / p.output_size.0.max(1) as f64;
                        fov *= smoothing::focal_length::compensation(&p.focal_lengths, &p.smoothed_focal_lengths, frame);
                        let mut pt = points[i];
                        let width_ratio = p.size.0 as f64 / p.output_size.0 as f64;
                        let height_ratio = p.size.1 as f64 / p.output_size.1 as f64;
                        pt = (pt.0 - 0.5, pt.1 - 0.5);
                        pt = (pt.0 / fov * width_ratio, pt.1 / fov * height_ratio);
                        pt = (pt.0 + 0.5, pt.1 + 0.5);
                        if pt.0 >= 0.0 && pt.1 >= 0.0 {
                            drawing.put_pixel((pt.0 * p.output_size.0 as f64) as i32, (pt.1 * p.output_size.1 as f64) as i32, Color::Red, Alpha::Alpha100, Stage::OnOutput, y_inverted, 4);
                        }
                    }
                }
            }
        }
    }

    pub fn process_pixels<T: PixelType>(&self, mut timestamp_us: i64, frame: Option<usize>, buffers: &mut Buffers) -> Result<stabilization::ProcessedInfo, GyroflowCoreError> {
        // A frame of a GPU which wgpu can't share the CUDA memory with: on a copy in the host memory
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        if gpu::wgpu_interop_cuda::needs_host_copy(buffers) {
            return gpu::wgpu_interop_cuda::through_host(buffers, |buffers| self.process_pixels::<T>(timestamp_us, frame, buffers));
        }

        if let gpu::BufferSource::Cpu { buffer } = &buffers.input.data  { if buffer.is_empty() { return Err(GyroflowCoreError::InputBufferEmpty); } }
        if let gpu::BufferSource::Cpu { buffer } = &buffers.output.data { if buffer.is_empty() { return Err(GyroflowCoreError::OutputBufferEmpty); } }

        let (offset, fps) = {
            let params = self.params.read();
            (params.frame_offset, params.fps)
        };
        let frame = frame.map(|x| (x as i32 + offset).max(0) as usize);
        timestamp_us += (offset as f64 / fps * 1000000.0).round() as i64;

        if let Some(scale) = self.params.read().fps_scale {
            timestamp_us = (timestamp_us as f64 / scale).round() as i64;
        }

        if self.smoothing_invalidated.load(SeqCst) {
            self.recompute_smoothness();
            self.smoothing_invalidated.store(false, SeqCst);
        }
        if self.zooming_invalidated.load(SeqCst) {
            self.recompute_adaptive_zoom();
            self.zooming_invalidated.store(false, SeqCst);
        }
        if self.undistortion_invalidated.load(SeqCst) {
            self.recompute_undistortion();
            self.undistortion_invalidated.store(false, SeqCst);
        }

        let (use_cache, hash, current_hash) = {
            let stab = self.stabilization.read();
            (stab.cache_frame_transform, stab.get_current_checksum(buffers), stab.initialized_backend.get_hash())
        };

        if use_cache || hash != current_hash {
            if let Some(mut undist) = self.stabilization.try_write_for(std::time::Duration::from_millis(30000)) {
                self.draw_overlays(&mut undist.drawing, timestamp_us);
                undist.ensure_ready_for_processing::<T>(timestamp_us, frame, buffers);
            } else {
                return Err(GyroflowCoreError::Unknown);
            }
        }

        if let Some(undist) = self.stabilization.try_read_for(std::time::Duration::from_millis(30000)) {
            undist.process_pixels::<T>(timestamp_us, frame, buffers, None)
        } else {
            Err(GyroflowCoreError::Unknown)
        }
    }

    pub fn set_video_rotation(&self, v: f64) { self.params.write().video_rotation = v; self.invalidate_smoothing(); }

    pub fn trim_ranges(&self) -> Vec<(f64, f64)> { self.params.read().trim_ranges.clone() }
    pub fn set_trim_ranges(&self, v: Vec<(f64, f64)>) {
        self.params.write().trim_ranges = if v.first() == Some(&(0.0, 1.0)) {
           Vec::new()
        } else {
            v
        };
        self.invalidate_smoothing();
    }

    pub fn set_of_method(&self, v: u32) { self.params.write().of_method = v; self.pose_estimator.clear(); }
    pub fn set_show_detected_features(&self, v: bool) { self.params.write().show_detected_features = v; }
    pub fn set_show_optical_flow     (&self, v: bool) { self.params.write().show_optical_flow      = v; }
    pub fn set_stab_enabled          (&self, v: bool) { self.params.write().stab_enabled           = v; }
    pub fn set_frame_readout_time    (&self, v: f64)  { self.params.write().frame_readout_time     = v; }
    pub fn set_frame_readout_direction(&self, v: impl Into<ReadoutDirection>) { self.params.write().frame_readout_direction = v.into(); }
    pub fn set_adaptive_zoom         (&self, v: f64)  { self.params.write().adaptive_zoom_window   = v; self.invalidate_zooming(); }
    pub fn set_zooming_center_x      (&self, v: f64)  { self.params.write().adaptive_zoom_center_offset.0 = v; self.invalidate_zooming(); }
    pub fn set_zooming_center_y      (&self, v: f64)  { self.params.write().adaptive_zoom_center_offset.1 = v; self.invalidate_zooming(); }
    pub fn set_additional_rotation_x (&self, v: f64)  { self.params.write().additional_rotation.0  = v; self.invalidate_smoothing(); }
    pub fn set_additional_rotation_y (&self, v: f64)  { self.params.write().additional_rotation.1  = v; self.invalidate_smoothing(); }
    pub fn set_additional_rotation_z (&self, v: f64)  { self.params.write().additional_rotation.2  = v; self.invalidate_smoothing(); }
    pub fn set_additional_translation_x(&self, v: f64){ self.params.write().additional_translation.0 = v; self.invalidate_zooming(); }
    pub fn set_additional_translation_y(&self, v: f64){ self.params.write().additional_translation.1 = v; self.invalidate_zooming(); }
    pub fn set_additional_translation_z(&self, v: f64){ self.params.write().additional_translation.2 = v; self.invalidate_zooming(); }
    pub fn set_zooming_method        (&self, v: i32)  { self.params.write().adaptive_zoom_method   = v;   self.invalidate_zooming(); }
    pub fn set_fov                   (&self, v: f64)  { self.params.write().fov                    = v; }
    pub fn set_fov_overview          (&self, v: bool) { self.params.write().fov_overview           = v; }
    pub fn set_show_safe_area        (&self, v: bool) { self.params.write().show_safe_area         = v; }
    pub fn set_lens_correction_amount(&self, v: f64)  { self.params.write().lens_correction_amount = v; self.invalidate_zooming(); }
    pub fn set_output_projection     (&self, v: i32)  { self.params.write().output_projection      = v; self.invalidate_zooming(); }
    pub fn set_frame_offset          (&self, v: i32)  { self.params.write().frame_offset           = v; }
    pub fn set_light_refraction_coefficient(&self, v: f64) { self.params.write().light_refraction_coefficient = v; self.invalidate_zooming(); }
    pub fn set_background_color      (&self, bg: Vector4<f32>) { self.params.write().background = bg; }
    pub fn set_background_mode       (&self, v: i32)  { self.params.write().background_mode = stabilization_params::BackgroundMode::from(v); }
    pub fn set_background_margin     (&self, v: f64)  { self.params.write().background_margin = v; }
    pub fn set_background_margin_feather(&self, v: f64) { self.params.write().background_margin_feather = v; }
    pub fn set_input_horizontal_stretch (&self, v: f64) { self.lens.write().input_horizontal_stretch = v; self.invalidate_zooming(); }
    pub fn set_input_vertical_stretch   (&self, v: f64) { self.lens.write().input_vertical_stretch   = v; self.invalidate_zooming(); }
    pub fn set_max_zoom(&self, v: f64, iters: usize)  {
        let mut params = self.params.write();
        params.max_zoom = if v > 50.0 { Some(v) } else { None };
        params.max_zoom_iterations = iters.max(1);
        self.invalidate_smoothing();
    }
    pub fn set_fixed_zoom(&self, v: bool) {
        self.params.write().fixed_zoom = v;
        self.invalidate_smoothing(); // the zoom limit folds into the smoothing
    }
    pub fn set_zoom_limit_whole_clip(&self, v: bool) {
        {
            let mut params = self.params.write();
            if params.zoom_limit_whole_clip == v { return; } // the UI showing what was loaded, it stays as it came
            params.zoom_limit_whole_clip = v;
            params.zoom_limit_whole_clip_from_old_file = false;
        }
        self.invalidate_smoothing();
    }
    /// With the manual zoom level: the zoom (%) it's set to, when the frame doesn't fit in it even with the smoothing lowered
    /// and the video had to be zoomed in further. `None` when it fits, or with another zoom level
    pub fn manual_zoom_missed(&self) -> Option<f64> {
        let params = stabilization::ComputeParams::from_manager(self);
        if !zooming::is_manual_zoom(&params) { return None; }
        let target = zooming::zoom_limit_fovs(&params).into_iter().flatten().next()?;
        let fov = params.fovs.iter().copied().reduce(f64::min)?;
        let scaling_factor = params.width as f64 / params.output_width.max(1) as f64;
        (fov < target - 1e-6).then(|| 100.0 / (target * scaling_factor))
    }

    pub fn set_video_speed(&self, v: f64, link_with_smoothness: bool, link_with_zooming: bool, link_with_zooming_limit: bool) {
        let mut params = self.params.write();
        params.video_speed = v;
        params.video_speed_affects_smoothing = link_with_smoothness;
        params.video_speed_affects_zooming = link_with_zooming;
        params.video_speed_affects_zooming_limit = link_with_zooming_limit;
        self.invalidate_smoothing();
    }

    pub fn disable_lens_stretch(&self, adjust_size: bool) {
        let (x_stretch, y_stretch) = {
            let lens = self.lens.read();
            (lens.input_horizontal_stretch, lens.input_vertical_stretch)
        };
        if (x_stretch > 0.01 && x_stretch != 1.0) || (y_stretch > 0.01 && y_stretch != 1.0) {
            if adjust_size {
                let mut params = self.params.write();
                params.size.0 = (params.size.0 as f64 * x_stretch).round() as usize;
                params.size.1 = (params.size.1 as f64 * y_stretch).round() as usize;
            }
            {
                let mut lens = self.lens.write();
                lens.input_horizontal_stretch = 1.0;
                lens.input_vertical_stretch = 1.0;
            }
        }
    }

    pub fn get_scaling_ratio(&self) -> f64 { let params = self.params.read(); params.size.0 as f64 / params.output_size.0 as f64 }
    pub fn get_min_fov      (&self) -> f64 { self.params.read().min_fov }

    pub fn invalidate_smoothing(&self) {
        self.invalidate_ongoing_computations();
        self.smoothing_checksum.store(0, SeqCst);
        self.invalidate_zooming();
    }
    pub fn invalidate_zooming(&self) {
        self.invalidate_ongoing_computations();
        self.zooming_checksum.store(0, SeqCst);
    }

    pub fn invalidate_blocking_smoothing(&self) { self.invalidate_ongoing_computations(); self.smoothing_invalidated.store(true, SeqCst); self.zooming_invalidated.store(true, SeqCst); self.undistortion_invalidated.store(true, SeqCst); }
    pub fn invalidate_blocking_zooming(&self) { self.invalidate_ongoing_computations(); self.zooming_invalidated.store(true, SeqCst); self.undistortion_invalidated.store(true, SeqCst); }
    pub fn invalidate_blocking_undistortion(&self) { self.invalidate_ongoing_computations(); self.undistortion_invalidated.store(true, SeqCst); }

    pub fn set_digital_lens_name(&self, v: String) {
        self.lens.write().digital_lens =  if !v.is_empty() { Some(v.clone()) } else { None };
        #[cfg(feature = "opencv")]
        if let Some(ref mut calib) = *self.lens_calibrator.write() {
            calib.digital_lens = if !v.is_empty() { Some(v) } else { None };
        }
        self.invalidate_zooming();
    }
    pub fn set_digital_lens_param(&self, index: usize, value: f64) {
        let mut lens = self.lens.write();
        if lens.digital_lens_params.is_none() {
            lens.digital_lens_params = Some(vec![0f64; 16]);
        }
        lens.digital_lens_params.as_mut().unwrap()[index] = value;
        #[cfg(feature = "opencv")]
        if let Some(ref mut calib) = *self.lens_calibrator.write() {
            calib.digital_lens_params = lens.digital_lens_params.clone();
        }
        self.invalidate_zooming();
    }
    pub fn set_lens_is_asymmetrical(&self, v: bool) {
        self.lens.write().asymmetrical = v;
        #[cfg(feature = "opencv")]
        if let Some(ref mut calib) = *self.lens_calibrator.write() {
            calib.asymmetrical = v;
        }
        self.invalidate_zooming();
    }

    // The motion data's lock before the keyframes', as everywhere: `self.keyframes.write().update_gyro(&self.gyro.read())`
    // takes them the other way round (the receiver first), and two threads taking them in opposite orders deadlock
    pub fn remove_offset(&self, timestamp_us: i64) {
        let mut gyro = self.gyro.write();
        gyro.remove_offset(timestamp_us);
        self.keyframes.write().update_gyro(&gyro);
        drop(gyro);
        self.invalidate_zooming();
    }
    pub fn set_offset(&self, timestamp_us: i64, offset_ms: f64) {
        let mut gyro = self.gyro.write();
        gyro.set_offset(timestamp_us, offset_ms);
        self.keyframes.write().update_gyro(&gyro);
        drop(gyro);
        self.invalidate_zooming();
    }
    pub fn clear_offsets(&self) {
        let mut gyro = self.gyro.write();
        gyro.clear_offsets();
        self.keyframes.write().update_gyro(&gyro);
        drop(gyro);
        self.invalidate_zooming();
    }
    pub fn offset_at_video_timestamp(&self, timestamp_us: i64) -> f64 {
        self.gyro.read().offset_at_video_timestamp(timestamp_us as f64 / 1000.0)
    }

    pub fn set_imu_lpf(&self, lpf: f64) {
        self.gyro.write().imu_transforms.imu_lpf = lpf;
    }
    pub fn set_imu_median_filter(&self, size: i32) {
        self.gyro.write().imu_transforms.imu_mf = size;
    }
    pub fn set_imu_rotation(&self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64) {
        self.gyro.write().imu_transforms.set_imu_rotation(pitch_deg, roll_deg, yaw_deg);
    }
    pub fn set_acc_rotation(&self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64) {
        self.gyro.write().imu_transforms.set_acc_rotation(pitch_deg, roll_deg, yaw_deg);
    }
    pub fn set_imu_orientation(&self, orientation: String) {
        self.gyro.write().imu_transforms.imu_orientation = Some(orientation);
    }
    pub fn set_imu_bias(&self, bx: f64, by: f64, bz: f64) {
        self.gyro.write().imu_transforms.gyro_bias = Some([bx, by, bz]);
    }
    pub fn recompute_gyro(&self) {
        GyroSource::integrate_shared(&self.gyro, Integration::ApplyTransforms);
        self.invalidate_smoothing();
    }
    /// Integrates the motion data again with another method (`GyroSource::integration_method`)
    pub fn set_integration_method(&self, method: usize) {
        self.gyro.write().integration_method = method;
        GyroSource::integrate_shared(&self.gyro, Integration::Integrate);
        self.invalidate_smoothing();
    }

    /// Installs a correction (or none) in the context as it is now, so it applies right away when it was measured in it
    pub fn set_optical_correction(&self, correction: Option<gyro_source::OpticalCorrection>) {
        let context = correction.is_some().then(|| self.optical_context());
        {
            let mut gyro = self.gyro.write();
            if let Some(context) = context { gyro.optical_context = context; }
            gyro.optical_correction = correction.map(Arc::new);
        }
        self.recompute_gyro();
    }
    pub fn set_optical_correction_enabled(&self, enabled: bool) {
        // A copy of the correction when another copy of the motion data shares it (`Arc::make_mut`), only when it changes
        let changed = self.gyro.write().optical_correction.as_mut().filter(|c| c.enabled != enabled).map(|c| Arc::make_mut(c).enabled = enabled).is_some();
        if changed { self.recompute_gyro(); }
    }
    pub fn clear_optical_correction(&self) {
        self.invalidate_optical_measurements();
        self.set_optical_correction(None);
    }
    /// Drops the kept measurements and moves `optical_generation` on: an analysis still running stops, and a refit
    /// still running finds nothing to replace. For everything after which what they measure is for something else
    fn invalidate_optical_measurements(&self) {
        let mut kept = self.optical_measurements.write();
        self.optical_generation.fetch_add(1, SeqCst);
        *kept = None;
    }
    /// The context a correction applies in, as things are now, see `optical_motion::context_checksum`
    fn optical_context(&self) -> u64 {
        use synchronization::optical_motion::{ context_checksum, measurement_params };
        context_checksum(&measurement_params(self))
    }
    /// Brings the correction up to date with its context: a sync, a lens profile or a frame timing other than the ones
    /// it was measured with switches it off (and back on, when they come back), without `integrate` hearing of it. The
    /// recomputes call this; true when it switched
    pub fn refresh_optical_correction(&self) -> bool {
        if self.gyro.read().optical_correction.is_none() { return false; }
        let context = self.optical_context();
        // Every recompute asks, on the UI thread: a write would wait for any smoothing still reading the motion data
        {
            let gyro = self.gyro.read();
            if gyro.optical_context == context && gyro.optical_correction_applies() == gyro.optical_correction_applied { return false; }
        }
        {
            let mut gyro = self.gyro.write();
            gyro.optical_context = context;
            if gyro.optical_correction_applies() == gyro.optical_correction_applied { return false; }
        }
        GyroSource::integrate_shared(&self.gyro, Integration::Integrate);
        self.invalidate_smoothing();
        true
    }
    /// See `GyroSource::set_ignore_file_motion`. True when anything changed
    pub fn set_ignore_file_motion(&self, ignore: bool) -> bool {
        {
            let mut gyro = self.gyro.write();
            if !gyro.set_ignore_file_motion_unintegrated(ignore) { return false; }
            self.keyframes.write().update_gyro(&gyro);
        }
        GyroSource::integrate_shared(&self.gyro, Integration::ApplyTransforms);
        self.invalidate_smoothing();
        true
    }
    /// Keeps what an analysis measured and fits the correction to it with the current settings. When what's loaded
    /// changed since the analysis started (another file, a project, Clear: `optical_generation`), they're dropped
    /// instead, as "Cancelled"
    pub fn set_optical_measurements(&self, m: synchronization::optical_motion::OpticalMeasurements) -> Result<(), String> {
        // The tracks taken out of an analysis before that this one tracked again: out of this one too
        let removed = m.tracks.ids_of(self.optical_removed_tracks.read().iter());
        let m = if removed.is_empty() { m } else {
            ::log::info!("Optical analysis: measuring again without the {} tracks removed before", removed.len());
            let generation = m.generation;
            match m.remeasure(removed, || self.optical_generation.load(SeqCst) != generation) {
                Ok(without) => without,
                Err(e) if e == "Cancelled" => return Err(e),
                Err(e) => {
                    // Too little left without them: the analysis is worth more than what was taken out of another one,
                    // so they're put back, like an edit that can't be measured (`remeasure_optical_correction`)
                    let mut removed = self.optical_removed_tracks.write();
                    if self.optical_generation.load(SeqCst) != generation { return Err("Cancelled".into()); }
                    ::log::warn!("Optical analysis: {e} without the tracks removed before, they're put back");
                    removed.retain(|s| m.tracks.ids_of(std::iter::once(s)).is_empty());
                    m
                }
            }
        };
        let m = Arc::new(m);
        let context = self.optical_context();
        loop {
            // A copy: the strength slider would wait for the fit on the settings lock
            let settings = *self.optical_settings.read();
            let correction = self.fit_optical_correction(&m, &settings)?;
            let mut kept = self.optical_measurements.write();
            if self.optical_generation.load(SeqCst) != m.generation { return Err("Cancelled".into()); }
            // Moved during the fit: the refit that asked for found no measurements to refit yet, so it's up to this one
            if *self.optical_settings.read() != settings { continue; }
            *kept = Some(m);
            let mut gyro = self.gyro.write();
            gyro.optical_context = context;
            gyro.optical_correction = Some(Arc::new(correction));
            break;
        }
        self.recompute_gyro();
        Ok(())
    }
    /// Fits the correction to the measurements with `settings` (`optical_motion::solve`): in a video cut from recordings,
    /// each part on its own, from the bands measured between its own frames (`optical_correction::solve_parts`), as the
    /// part trimmed on its own would have it
    fn fit_optical_correction(&self, m: &synchronization::optical_motion::OpticalMeasurements, settings: &gyro_source::OpticalCorrectionSettings) -> Result<gyro_source::OpticalCorrection, String> {
        match gyro_source::optical_correction::MeasuredParts::of(&synchronization::optical_motion::measurement_params(self)) {
            Some(parts) => gyro_source::optical_correction::solve_parts(m, settings, &parts),
            None => synchronization::optical_motion::solve(m, settings),
        }
    }
    /// The kept measurements, as long as the correction they gave still sits on what they were measured on: the same
    /// quaternions (uncorrected), in the same context
    fn valid_optical_measurements(&self) -> Option<Arc<synchronization::optical_motion::OpticalMeasurements>> {
        let m = self.optical_measurements.read().clone()?;
        let gyro = self.gyro.read();
        (gyro.optical_correction.is_some() && m.quats_checksum == gyro.optical_uncorrected_checksum && m.context_checksum == gyro.optical_context).then_some(m)
    }
    /// Sets the strength; true when the correction should be refitted to it, see `refit_optical_correction`
    pub fn set_optical_correction_strength(&self, strength: f64) -> bool {
        self.update_optical_settings(|s| s.strength = strength.clamp(0.0, 1.0))
    }
    /// One correction per frame instead of per band of rows, see `OpticalCorrectionSettings::per_frame`; true when the
    /// correction should be refitted to it, see `refit_optical_correction`
    pub fn set_optical_correction_per_frame(&self, per_frame: bool) -> bool {
        self.update_optical_settings(|s| s.per_frame = per_frame)
    }
    fn update_optical_settings(&self, update: impl FnOnce(&mut gyro_source::OpticalCorrectionSettings)) -> bool {
        let settings = { let mut s = self.optical_settings.write(); update(&mut s); *s };
        let differs = self.gyro.read().optical_correction.as_ref().map(|c| c.settings != settings).unwrap_or_default();
        differs && self.valid_optical_measurements().is_some()
    }
    /// Refits the correction to the kept measurements with the current settings (a fraction of a second for a long
    /// clip, so not on the UI thread). False when there's nothing to refit, or the settings moved on meanwhile. Only
    /// ever replaces the correction these very measurements gave: never one that Clear, another file or another
    /// analysis put there while it was fitting
    pub fn refit_optical_correction(&self) -> Result<bool, String> {
        let Some(m) = self.valid_optical_measurements() else { return Ok(false) };
        let settings = *self.optical_settings.read();
        let mut c = self.fit_optical_correction(&m, &settings)?;
        {
            let kept = self.optical_measurements.read();
            if !kept.as_ref().is_some_and(|k| Arc::ptr_eq(k, &m)) || *self.optical_settings.read() != settings { return Ok(false); }
            let mut gyro = self.gyro.write();
            let Some(fitted) = &gyro.optical_correction else { return Ok(false) };
            c.enabled = fitted.enabled;
            gyro.optical_correction = Some(Arc::new(c));
        }
        self.recompute_gyro();
        Ok(true)
    }
    /// How the next "Analyze image optically" follows the points, "klt" or "dis". False for neither
    pub fn set_optical_tracking(&self, name: &str) -> bool {
        let Some(method) = gyro_source::TrackingMethod::from_name(name) else { return false };
        *self.optical_tracking.write() = method;
        true
    }
    /// Takes the tracks `ids` of the kept analysis out of what it measured, or puts them back (`remove` false). Returns
    /// the ones that weren't already: when any, `remeasure_optical_correction` brings the correction up to date
    pub fn remove_optical_tracks(&self, ids: &[u32], remove: bool) -> Vec<u32> {
        let Some(m) = self.optical_measurements.read().clone() else { return Vec::new() };
        let mut removed = self.optical_removed_tracks.write();
        ids.iter().copied().filter(|id| {
            let Some(start) = m.tracks.start_of(*id) else { return false };
            if remove { removed.insert(start) } else { removed.remove(&start) }
        }).collect()
    }
    /// Puts back every track taken out. True when there were any
    pub fn restore_optical_tracks(&self) -> bool {
        let mut removed = self.optical_removed_tracks.write();
        let had = !removed.is_empty();
        removed.clear();
        had
    }
    /// Measures the kept analysis again without the tracks taken out now (`optical_removed_tracks`), and fits the
    /// correction to that: a second or so per minute of video, so not on the UI thread. False when there's nothing to
    /// do, or when it was overtaken - by another change of the tracks taken out (whose own call measures that), another
    /// analysis, another file. Like a refit, only ever replaces the correction the kept measurements gave.
    ///
    /// All or nothing: what can't be measured (too little of the image left without the tracks taken out) is undone -
    /// the tracks taken out go back to the ones the kept analysis left out - and the error says so. Without that the
    /// tracks would look taken out while the correction still had them, and nothing would ever measure them again
    pub fn remeasure_optical_correction(&self) -> Result<bool, String> {
        let Some(m) = self.valid_optical_measurements() else { return Ok(false) };
        let removed = self.optical_removed_tracks.read().clone();
        let excluded = m.tracks.ids_of(&removed);
        if excluded == *m.excluded { return Ok(false); }
        // One lock at a time: the fit below takes them the other way round
        let overtaken = || {
            let same = *self.optical_removed_tracks.read() == removed;
            !same || !self.optical_measurements.read().as_ref().is_some_and(|k| Arc::ptr_eq(k, &m))
        };
        let failed = |e: String| -> Result<bool, String> {
            if !self.revert_optical_tracks(&m, &removed) { return Ok(false); }
            Err(if e == "Not enough of the image could be tracked" {
                "Too little of the image is left to measure without the tracks taken out, so they were put back".into()
            } else {
                format!("{e}. The tracks taken out were put back")
            })
        };
        let new = Arc::new(match m.remeasure(excluded, overtaken) {
            Ok(new) => new,
            Err(e) if e == "Cancelled" => return Ok(false),
            Err(e) => return failed(e),
        });
        loop {
            // A copy: the strength slider would wait for the fit on the settings lock
            let settings = *self.optical_settings.read();
            let mut c = match self.fit_optical_correction(&new, &settings) {
                Ok(c) => c,
                Err(e) => return failed(e),
            };
            let mut kept = self.optical_measurements.write();
            if !kept.as_ref().is_some_and(|k| Arc::ptr_eq(k, &m)) || *self.optical_removed_tracks.read() != removed { return Ok(false); }
            // Moved during the fit: the refit that asked for it refitted the measurements replaced here
            if *self.optical_settings.read() != settings { continue; }
            let mut gyro = self.gyro.write();
            let Some(fitted) = &gyro.optical_correction else { return Ok(false) };
            c.enabled = fitted.enabled;
            gyro.optical_correction = Some(Arc::new(c));
            *kept = Some(new);
            break;
        }
        self.recompute_gyro();
        Ok(true)
    }
    /// Takes the tracks taken out back to the ones the kept analysis `m` left out, after `tried` couldn't be measured -
    /// unless they're no longer `tried` or `m` no longer the kept analysis: what overtook it measures its own. Tracks of
    /// other analyses (of another trim range) stay as they are. True when it did
    fn revert_optical_tracks(&self, m: &Arc<synchronization::optical_motion::OpticalMeasurements>, tried: &BTreeSet<synchronization::optical_motion::TrackStart>) -> bool {
        // In the order the fit takes them
        let kept = self.optical_measurements.read();
        let mut removed = self.optical_removed_tracks.write();
        if *removed != *tried || !kept.as_ref().is_some_and(|k| Arc::ptr_eq(k, m)) { return false; }
        removed.retain(|s| m.tracks.ids_of(std::iter::once(s)).is_empty());
        removed.extend(m.excluded.iter().filter_map(|id| m.tracks.start_of(*id)));
        true
    }
    /// Whether the kept analysis could be measured without the tracks taken out now and isn't: they were taken out (or
    /// put back) while it was out of date - another lens, sync, motion data - and it isn't any more. Then
    /// `remeasure_optical_correction` catches up
    pub fn optical_tracks_unmeasured(&self) -> bool {
        let Some(m) = self.valid_optical_measurements() else { return false };
        let removed = self.optical_removed_tracks.read();
        m.tracks.ids_of(removed.iter()) != *m.excluded
    }
    /// Lets go of what the last analysis kept to refit the correction and to take tracks out of it (the correction
    /// stays): a render job, which does neither, doesn't hold on to the tracks of a long clip
    pub fn release_optical_measurements(&self) {
        *self.optical_measurements.write() = None;
    }
    /// The frame at a moment of the file (µs), numbered as the analysis numbers them (`OpticalMotionAnalysis::feed_frame`)
    fn optical_frame_at(&self, file_timestamp_us: i64) -> usize {
        let p = self.params.read();
        p.frame_at_timestamp(file_timestamp_us as f64 / 1000.0 / p.get_fps_scale()).max(0) as usize
    }
    /// The tracks of the analysis `m` taken out now: until measuring again catches up, not quite the ones it left out
    fn optical_removed_ids(&self, m: &synchronization::optical_motion::OpticalMeasurements) -> HashSet<u32> {
        self.optical_removed_tracks.try_read().map(|removed| m.tracks.ids_of(removed.iter())).unwrap_or_else(|| (*m.excluded).clone())
    }
    /// The points the analysis `m` tracked in the frame `index` that the correction is measured with, as the user has
    /// them now: not the ones it didn't use (too short a track, too few points in their rows), not the tracks taken out,
    /// and the ones put back only once measuring again catches up
    fn optical_points_in_frame(&self, m: &synchronization::optical_motion::OpticalMeasurements, index: usize) -> Vec<(u32, f32, f32, synchronization::optical_motion::PointState)> {
        use synchronization::optical_motion::PointState;
        let removed = self.optical_removed_ids(m);
        m.points_at(index).into_iter()
            .filter(|p| !removed.contains(&p.0) && !matches!(p.3, PointState::Unused | PointState::Removed))
            .collect()
    }
    /// Where points of the input frame `index` (relative to it) are in the stabilized picture as the preview renders it,
    /// upright, relative to it: see `optical_motion::to_output`
    fn optical_placement(&self, stabilized: bool) -> impl Fn(usize, &[(f32, f32)]) -> Vec<Option<(f32, f32)>> + Send + Sync + 'static {
        let params = stabilized.then(|| {
            let mut params = ComputeParams::from_manager(self);
            params.framebuffer_inverted = false;
            params
        });
        move |index, points| match &params {
            Some(params) => synchronization::optical_motion::to_output(params, index, points),
            None => synchronization::optical_motion::as_tracked(index, points),
        }
    }
    /// Where the last "Analyze image optically" tracked points in the frame at a moment of the file (µs), but the tracks
    /// taken out: their track, where they are relative to the frame (0 to 1), and what became of them. With `stabilized`,
    /// where they are in the stabilized picture the preview shows, and only the ones in it
    pub fn optical_tracked_points(&self, file_timestamp_us: i64, stabilized: bool) -> Vec<(u32, f32, f32, synchronization::optical_motion::PointState)> {
        let Some(m) = self.optical_measurements.read().clone() else { return Vec::new() };
        let index = self.optical_frame_at(file_timestamp_us);
        let mut points = self.optical_points_in_frame(&m, index);
        if stabilized {
            let placed = self.optical_placement(true)(index, &points.iter().map(|p| (p.1, p.2)).collect::<Vec<_>>());
            points = points.into_iter().zip(placed)
                .filter_map(|(p, at)| at.filter(|(x, y)| (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y)).map(|(x, y)| (p.0, x, y, p.3)))
                .collect();
        }
        points
    }
    /// The tracks (but the ones taken out) with a point within `rect` (x0, y0, x1, y1, relative to the frame - the
    /// stabilized one with `stabilized`) in the frame at a moment of the file (µs), or with `all_frames`, within the
    /// part of the camera frame `rect` covers there in any frame. Every frame of a long clip placed in the stabilized
    /// picture takes seconds: not on the UI thread. None once `cancelled`
    pub fn optical_tracks_in_rect(&self, file_timestamp_us: i64, rect: [f32; 4], all_frames: bool, stabilized: bool, cancelled: impl Fn() -> bool + Sync) -> Option<Vec<u32>> {
        let Some(m) = self.optical_measurements.read().clone() else { return Some(Vec::new()) };
        let shown = self.optical_frame_at(file_timestamp_us);
        let frames = if all_frames { 0..=usize::MAX } else { shown..=shown };
        // Every frame's points placed the way the frame shown is: over the whole clip, what's fixed to the camera
        // (propellers, the drone's body, on-screen graphics) is what stays in the box, not what the stabilized
        // picture holds still there
        let place = self.optical_placement(stabilized);
        m.tracks.in_rect(frames, rect, &self.optical_removed_ids(&m), move |_, points: &[(f32, f32)]| place(shown, points), cancelled)
    }
    /// Where the tracks `ids` are in the frames up to `reach` away from the one at a moment of the file (µs): per track,
    /// how many frames from that one, and where, relative to the frame (the stabilized one with `stabilized`)
    pub fn optical_track_paths(&self, ids: &HashSet<u32>, file_timestamp_us: i64, reach: usize, stabilized: bool) -> Vec<(u32, Vec<(i64, f32, f32)>)> {
        let Some(m) = self.optical_measurements.read().clone() else { return Vec::new() };
        let i = self.optical_frame_at(file_timestamp_us);
        m.tracks.paths(ids, i.saturating_sub(reach)..=i + reach, self.optical_placement(stabilized)).into_iter()
            .map(|(id, path)| (id, path.into_iter().map(|(f, x, y)| (f as i64 - i as i64, x, y)).collect()))
            .collect()
    }
    /// State of the correction measured from the video, for the UI
    pub fn optical_correction_info(&self) -> serde_json::Value {
        // Read out under the lock, which is let go of before `valid_optical_measurements` takes it again: a second read
        // of a parking_lot lock this thread holds deadlocks as soon as a writer queues up in between
        let (mut info, fitted_with) = {
            let gyro = self.gyro.read();
            let (ignore_file_motion, has_motion) = (gyro.ignores_file_motion(), gyro.has_motion());
            match &gyro.optical_correction {
                Some(c) => (serde_json::json!({
                    "available": true,
                    "enabled": c.enabled,
                    // Enabled but not applied: measured on other quaternions (another integration method or filter) or
                    // in another context (sync, lens, frame timing) than the ones there now
                    "stale": c.enabled && !gyro.optical_correction_applied,
                    "frames": c.frames,
                    "measured_frames": c.measured_frames,
                    "rms_deg": c.rms_deg,
                    "strength": c.settings.strength,
                    "per_frame": c.settings.per_frame,
                    // The file had no motion data: the analysis measured all of it
                    "from_video": !c.video_base.is_empty(),
                    "ignore_file_motion": ignore_file_motion,
                    "has_motion": has_motion,
                }), Some((c.settings, c.tracking))),
                None => (serde_json::json!({ "available": false, "ignore_file_motion": ignore_file_motion, "has_motion": has_motion }), None),
            }
        };
        // What "Show/edit tracked points" can show: an analysis of this session, a project keeps only the correction
        let kept = self.optical_measurements.read().clone();
        info["tracked_points"] = kept.as_ref().is_some_and(|m| !m.tracks.is_empty()).into();
        // Which analysis's tracks they are: the same through measuring again, another for another analysis, which numbers
        // its tracks anew
        info["tracks_key"] = kept.as_ref().map(|m| Arc::as_ptr(&m.tracks) as usize as u64).unwrap_or_default().into();
        // Only measurements the correction still sits on can be measured again without some of the tracks
        let editable = kept.is_some() && self.valid_optical_measurements().is_some();
        info["tracks_editable"] = editable.into();
        {
            let removed = self.optical_removed_tracks.read();
            // Of this analysis when there is one, otherwise all the project remembers
            let (count, pending) = match &kept {
                Some(m) => { let ids = m.tracks.ids_of(removed.iter()); (ids.len(), editable && ids != *m.excluded) },
                None => (removed.len(), false),
            };
            info["removed_tracks"] = count.into();
            // Taken out or put back, and not measured again yet
            info["removed_tracks_pending"] = pending.into();
        }
        let tracking = *self.optical_tracking.read();
        info["tracking"] = tracking.name().into();
        if let Some((fitted, fitted_tracking)) = fitted_with {
            // Tracked another way than the one set now: still a correction, the new one applies from the next analysis
            info["tracking_changed"] = (fitted_tracking != tracking).into();
            // Fitted with other settings than the ones set, and the measurements to refit aren't here any more (a
            // project loaded from disk keeps only the correction): only another analysis applies the new ones. The
            // strength doesn't count without motion data to correct: the motion and its correction come from the same
            // tracks, and it changes next to nothing
            let set = *self.optical_settings.read();
            let differs = set.per_frame != fitted.per_frame || (info["has_motion"] == true && set.strength != fitted.strength);
            info["outdated"] = (differs && self.valid_optical_measurements().is_none()).into();
        }
        info
    }
    pub fn set_sync_lpf(&self, lpf: f64) {
        let params = self.params.read();
        self.pose_estimator.lowpass_filter(lpf, params.fps);
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
                _ => { }
            }
        }
    }

    pub fn set_gpu_decoding(&self, v: bool) {
        self.gpu_decoding.store(v, SeqCst);
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
    pub fn set_horizon_lock(&self, lock_percent: f64, roll: f64, lock_pitch: bool, pitch: f64, automatic_lock: bool, turn_threshold: f64, turn_smoothing_ms: f64, turn_multiplier: f64, tilt_accel_limit: f64) {
        self.smoothing.write().horizon_lock.set_horizon(lock_percent, roll, lock_pitch, pitch, automatic_lock, turn_threshold, turn_smoothing_ms, turn_multiplier, tilt_accel_limit);
        self.invalidate_smoothing();
    }
    // As `GyroSource::set_use_gravity_vectors` and `set_horizon_lock_integration_method`, integrated with the lock released
    pub fn set_use_gravity_vectors(&self, v: bool) {
        let changed = std::mem::replace(&mut self.gyro.write().use_gravity_vectors, v) != v;
        if changed { GyroSource::integrate_shared(&self.gyro, Integration::Integrate); }
        self.invalidate_smoothing();
    }
    pub fn set_horizon_lock_integration_method(&self, v: i32) {
        let changed = std::mem::replace(&mut self.gyro.write().horizon_lock_integration_method, v) != v;
        if changed { GyroSource::integrate_shared(&self.gyro, Integration::Integrate); }
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

    pub fn get_cloned(&self) -> StabilizationManager {
        StabilizationManager {
            params: Arc::new(RwLock::new(self.params.read().clone())),
            gyro:   Arc::new(RwLock::new(self.gyro.read().clone())),
            lens:   Arc::new(RwLock::new(self.lens.read().clone())),
            keyframes:  Arc::new(RwLock::new(self.keyframes.read().clone())),
            smoothing:  Arc::new(RwLock::new(self.smoothing.read().clone())),
            input_file: Arc::new(RwLock::new(self.input_file.read().clone())),
            lens_profile_db: self.lens_profile_db.clone(),
            // The strength the correction was fitted with: a project written from the clone (a render queue job's, which
            // "Edit" loads back) says it, and one that says another would refit the correction there
            optical_settings: Arc::new(RwLock::new(*self.optical_settings.read())),
            // How a render queue job's own analysis tracks, and what it leaves out of that
            optical_tracking: Arc::new(RwLock::new(*self.optical_tracking.read())),
            optical_removed_tracks: Arc::new(RwLock::new(self.optical_removed_tracks.read().clone())),

            // NOT cloned:
            // stabilization
            // pose_estimator
            // lens_calibrator
            // current_compute_id
            // smoothing_checksum
            // zooming_checksum
            // prevent_recompute
            // camera_id
            // optical_measurements: they take a lot of memory, and a clone has nothing to refit
            ..Default::default()
        }
    }
    pub fn set_render_params(&self, size: (usize, usize), output_size: (usize, usize)) {
        self.params.write().framebuffer_inverted = false;
        self.params.write().fov_overview = false;
        self.params.write().show_safe_area = false;
        self.stabilization.write().kernel_flags.set(KernelParamsFlags::DRAWING_ENABLED, false);
        self.set_size(size.0, size.1);
        self.set_output_size(output_size.0, output_size.1);

        self.recompute_undistortion();
    }

    pub fn clear(&self) {
        self.invalidate_optical_measurements();
        self.params.write().clear();
        self.invalidate_ongoing_computations();
        self.invalidate_smoothing();
        *self.input_file.write() = InputFile::default();
        *self.camera_id.write() = None;

        *self.gyro.write() = GyroSource::new();
        self.keyframes.write().clear();
        self.optical_removed_tracks.write().clear();

        self.pose_estimator.clear();
    }

    pub fn override_video_fps(&self, fps: f64, recompute: bool) {
        {
            let mut params = self.params.write();
            if (fps - params.fps).abs() > 0.001 {
                if params.fps > 0.0 {
                    let scale = fps / params.fps;
                    params.set_fps_scale(Some(scale));
                } else {
                    // The video frame rate is not known, so there's nothing to scale against.
                    // This can happen eg. when the file gets unloaded while its telemetry is still being parsed in the background.
                    log::warn!("Unable to override video fps to {fps}, because the source fps is unknown");
                    params.set_fps_scale(None);
                }
            } else {
                params.set_fps_scale(None);
            }
            self.keyframes.write().timestamp_scale = params.fps_scale;
        }
        // The motion data takes the new timing in while the parameters can't change (`set_video_timing`; the params locked
        // before the gyro, as everywhere), and is integrated again where that needs it (segments the scale moves) with
        // neither lock held: every reader of them would wait on the whole integration
        let integrate = {
            let params = self.params.read();
            self.gyro.write().set_video_timing(&params)
        };
        if integrate { GyroSource::integrate_shared(&self.gyro, Integration::ApplyTransforms); }

        if recompute {
            self.stabilization.write().set_compute_params(stabilization::ComputeParams::from_manager(self));

            self.invalidate_smoothing();
        }
    }

    pub fn list_gpu_devices<F: Fn(Vec<String>) + Send + Sync + 'static>(&self, cb: F) {
        let stab = self.stabilization.clone();
        run_threaded(move || {
            let list = stab.read().list_devices();

            log::info!("GPU list: {:?}", &list);

            *stabilization::GPU_LIST.write() = list.clone();

            cb(list);
        });
    }

    pub fn export_gyroflow_file(&self, url: &str, typ: GyroflowProjectType, additional_data: &str) -> Result<(), GyroflowCoreError> {
        let data = self.export_gyroflow_data(typ, additional_data, Some(url))?;
        filesystem::write(url, data.as_bytes())?;

        self.input_file.write().project_file_url = Some(url.to_string());

        Ok(())
    }
    pub fn export_gyroflow_data(&self, typ: GyroflowProjectType, additional_data: &str, _project_url: Option<&str>) -> Result<String, GyroflowCoreError> {
        // Copies of one moment (the parameters locked before the motion data, as everywhere), and not the locks: the motion
        // data is serialized and compressed below, which takes seconds for a long high-rate log, and every reader would
        // wait for that behind the next writer. The motion data's copy shares its readings and orientations (`Arc`s)
        let (params, gyro) = {
            let params = self.params.read();
            (params.clone(), self.gyro.read().clone())
        };

        let (smoothing_name, smoothing_params, horizon_amount, horizon_lock) = {
            let smoothing_lock = self.smoothing.read();
            let smoothing = smoothing_lock.current();

            let mut parameters = smoothing.get_parameters_json();
            if let serde_json::Value::Array(ref mut arr) = parameters {
                for v in arr.iter_mut() {
                    if let serde_json::Value::Object(obj) = v {
                        *v = serde_json::json!({
                            "name": obj["name"],
                            "value": obj["value"]
                        });
                    }
                }
            }
            let mut horizon_amount = smoothing_lock.horizon_lock.horizonlockpercent;
            if !smoothing_lock.horizon_lock.lock_enabled {
                horizon_amount = 0.0;
            }

            (smoothing.get_name(), parameters, horizon_amount, smoothing_lock.horizon_lock.clone())
        };

        let input_file = self.input_file.read().clone();

        let trim_ranges_ms = params.trim_ranges.iter().map(|(a, b)| (a * params.duration_ms, b * params.duration_ms)).collect::<Vec<_>>();

        let mut obj = serde_json::json!({
            "title": "Gyroflow data file",
            "version": 4,
            "app_version": env!("CARGO_PKG_VERSION").to_string(),
            "videofile": input_file.url,
            "calibration_data": self.lens.read().get_json_value().unwrap_or_else(|_| serde_json::json!({})),
            "date": time::OffsetDateTime::now_local().map(|v| v.date().to_string()).unwrap_or_default(),

            "image_sequence_start": input_file.image_sequence_start,
            "image_sequence_fps": input_file.image_sequence_fps,
            "background_color": params.background.as_slice(),
            "background_mode":  params.background_mode as i32,
            "background_margin":          params.background_margin,
            "background_margin_feather":  params.background_margin_feather,
            "light_refraction_coefficient": params.light_refraction_coefficient,

            "video_info": {
                "width":       params.size.0,
                "height":      params.size.1,
                "rotation":    params.video_rotation,
                "num_frames":  params.frame_count,
                "fps":         params.fps,
                "duration_ms": params.duration_ms,
                "fps_scale":   params.fps_scale,
                "vfr_fps":     params.get_scaled_fps(),
                "vfr_duration_ms": params.get_scaled_duration_ms(),
                "created_at"   : params.video_created_at,
            },
            "stabilization": {
                "fov":                    params.fov,
                "method":                 smoothing_name,
                "smoothing_params":       smoothing_params,
                "frame_readout_time":     params.frame_readout_time.abs(),
                "frame_readout_direction": params.frame_readout_direction,
                "adaptive_zoom_window":   params.adaptive_zoom_window,
                "adaptive_zoom_center_offset": params.adaptive_zoom_center_offset,
                "adaptive_zoom_method":   params.adaptive_zoom_method,
                "additional_rotation":    params.additional_rotation,
                "additional_translation": params.additional_translation,
                "lens_correction_amount": params.lens_correction_amount,
                "output_projection":      stabilization::projection::setting_name(params.output_projection),
                "horizon_lock_amount":    horizon_amount,
                "horizon_lock_roll":      horizon_lock.horizonroll,
                "horizon_lock_pitch_enabled": horizon_lock.lock_pitch,
                "horizon_lock_pitch":     horizon_lock.horizonpitch,
                "automatic_lock":         horizon_lock.automatic_lock,
                "turn_threshold":         horizon_lock.turn_threshold,
                "turn_smoothing_ms":      horizon_lock.turn_smoothing_ms,
                "turn_multiplier":        horizon_lock.turn_multiplier,
                "tilt_accel_limit":       horizon_lock.tilt_accel_limit.is_finite().then_some(horizon_lock.tilt_accel_limit), // null = no limit
                "use_gravity_vectors":    gyro.use_gravity_vectors,
                "horizon_lock_integration_method": gyro.horizon_lock_integration_method,
                "video_speed":                   params.video_speed,
                "video_speed_affects_smoothing": params.video_speed_affects_smoothing,
                "video_speed_affects_zooming":   params.video_speed_affects_zooming,
                "video_speed_affects_zooming_limit": params.video_speed_affects_zooming_limit,
                "max_zoom":               params.max_zoom,
                "max_zoom_iterations":    params.max_zoom_iterations,
                "fixed_zoom":             params.fixed_zoom,
                "zoom_limit_whole_clip":  params.zoom_limit_whole_clip,
                "frame_offset":           params.frame_offset,
                "focal_length_smoothing_enabled":  params.focal_length_smoothing_enabled,
                "focal_length_max_zoom_rate":      params.focal_length_max_zoom_rate,
                "lens_metadata_delay_frames":      params.lens_metadata_delay_frames,
                "lens_breathing_enabled":          params.lens_breathing_enabled,
            },
            "gyro_source": {
                "filepath":           gyro.file_url,
                "all_metadata":       gyro.all_metadata,
                "lpf":                gyro.imu_transforms.imu_lpf,
                "mf":                 gyro.imu_transforms.imu_mf,
                "rotation":           gyro.imu_transforms.imu_rotation_angles,
                "acc_rotation":       gyro.imu_transforms.acc_rotation_angles,
                "imu_orientation":    gyro.imu_transforms.imu_orientation,
                "gyro_bias":          gyro.imu_transforms.gyro_bias,
                "integration_method": gyro.integration_method,
                "sample_index":       gyro.file_load_options.sample_index,
                "detected_source":    gyro.file_metadata.read().detected_source,
                "optical_correction_enabled": gyro.optical_correction.as_ref().map(|c| c.enabled),
                "optical_correction_strength": self.optical_settings.read().strength,
                "optical_correction_per_frame": self.optical_settings.read().per_frame,
                "optical_tracking": self.optical_tracking.read().name(),
                "optical_removed_tracks": *self.optical_removed_tracks.read(),
                "ignore_file_motion": gyro.ignores_file_motion(),
            },

            "offsets": gyro.file_offsets(), // timestamp, offset value
            "keyframes": self.keyframes.read().serialize(),

            // "trim_ranges": params.trim_ranges,
            "trim_ranges_ms": trim_ranges_ms,
        });

        util::merge_json(&mut obj, &serde_json::from_str(additional_data).unwrap_or_default());

        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if let serde_json::Value::Object(ref mut obj) = obj {
            obj.insert("videofile_bookmark".into(), serde_json::Value::String(filesystem::apple::create_bookmark(&input_file.url, false, _project_url)));
            if let Some(serde_json::Value::Object(obj)) = obj.get_mut("gyro_source") {
                obj.insert("filepath_bookmark".into(), serde_json::Value::String(filesystem::apple::create_bookmark(&gyro.file_url, false, _project_url)));
            }
            if let Some(serde_json::Value::Object(obj)) = obj.get_mut("output") {
                if let Some(output_folder) = obj.get("output_folder").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) {
                    obj.insert("output_folder_bookmark".into(), serde_json::Value::String(filesystem::apple::create_bookmark(output_folder, true, _project_url)));
                }
            }
        }

        if let Some(serde_json::Value::Object(obj)) = obj.get_mut("gyro_source") {
            // Before the guard below: it reads the same lock, and a second read of a parking_lot lock this thread holds
            // deadlocks as soon as a writer queues up in between
            let with_motion = if typ == GyroflowProjectType::Simple { None } else { gyro.file_metadata_with_ignored_motion() };
            let file_metadata = gyro.file_metadata.read();

            // The analysis took a pass over every frame, so it's kept with the project whatever its type
            if let Some(c) = gyro.optical_correction.as_deref().and_then(util::compress_to_base91_cbor) {
                obj.insert("optical_correction".into(), serde_json::Value::String(c));
            }

            if typ == GyroflowProjectType::Simple {
                if let Ok(val) = serde_json::to_value(file_metadata.thin()) {
                    obj.insert("file_metadata".into(), val);
                }
            } else {
                // With the file's motion data also while it's set aside, for when it's used again
                if let Some(q) = util::compress_to_base91_cbor(with_motion.as_ref().unwrap_or(&*file_metadata)) {
                    obj.insert("file_metadata".into(), serde_json::Value::String(q));
                }
            }

            if typ == GyroflowProjectType::WithProcessedData {
                // Export focal length arrays for smoothing (for new plugins that support it)
                if !params.focal_lengths.is_empty() {
                    util::compress_to_base91_cbor(&params.focal_lengths).and_then(|s| obj.insert("focal_lengths".into(), serde_json::Value::String(s)));
                }
                if !params.smoothed_focal_lengths.is_empty() {
                    util::compress_to_base91_cbor(&params.smoothed_focal_lengths).and_then(|s| obj.insert("smoothed_focal_lengths".into(), serde_json::Value::String(s)));
                }

                let mut imu_timestamps = Vec::with_capacity(gyro.quaternions.len());
                let mut imu_timestamps_final = Vec::with_capacity(gyro.quaternions.len());
                for (t, _) in gyro.quaternions.iter() {
                    let mut timestamp_ms = *t as f64 / 1000.0;
                    timestamp_ms += gyro.offset_at_gyro_timestamp(timestamp_ms);

                    imu_timestamps.push(timestamp_ms);

                    let frame = params.first_frame_at_or_after(timestamp_ms - params.frame_readout_time / 2.0).max(0) as usize;
                    imu_timestamps_final.push(timestamp_ms - file_metadata.frame_time_offset(frame));
                }
                util::compress_to_base91_cbor(&imu_timestamps)           .and_then(|s| obj.insert("synced_imu_timestamps" .into(), serde_json::Value::String(s)));
                util::compress_to_base91_cbor(&imu_timestamps_final)     .and_then(|s| obj.insert("synced_imu_timestamps_with_per_frame_offset".into(), serde_json::Value::String(s)));
                util::compress_to_base91_cbor(&*gyro.quaternions)         .and_then(|s| obj.insert("integrated_quaternions".into(), serde_json::Value::String(s)));
                util::compress_to_base91_cbor(&*gyro.smoothed_quaternions).and_then(|s| obj.insert("smoothed_quaternions"  .into(), serde_json::Value::String(s)));
                util::compress_to_base91_cbor(&params.fovs)              .and_then(|s| obj.insert("adaptive_zoom_fovs"    .into(), serde_json::Value::String(s)));
            }
        }

        Ok(serde_json::to_string_pretty(&obj)?)
    }

    /// The project files of the recordings the video was cut from, when it has a segment table that names them and their
    /// projects are next to the video, as Gyroflow saves a video's project (`<name>.gyroflow`): `(source name, project
    /// url)`
    pub fn source_project_urls(&self, video_url: &str) -> Vec<(String, String)> {
        let gyro = self.gyro.read();
        let md = gyro.file_metadata.read();
        let Some(segments) = md.trim_segments.as_ref() else { return Vec::new() };
        let folder = filesystem::get_folder(video_url);
        let mut out = Vec::new();
        for source in &segments.table.sources {
            let stem = source.name.rsplit_once('.').map_or(source.name.as_str(), |x| x.0);
            let url = filesystem::get_file_url(&folder, &format!("{stem}.gyroflow"), false);
            if !url.is_empty() && filesystem::exists(&url) && !out.iter().any(|(n, _): &(String, String)| *n == source.name) {
                out.push((source.name.clone(), url));
            }
        }
        out
    }

    /// The sync points of `project`, a project made on the recording `source_name` the video was cut from (its data, as
    /// `export_gyroflow_data` writes it), as this video's: each point at its moment of the recording in the video, through
    /// the segments of the video's table that are of it, and left out where the video doesn't have that moment.
    ///
    /// A sync point pairs a moment of the picture, the video's time at its key plus its offset (the file's time over its
    /// project's frame rate scale, `video_info.fps_scale`), with the motion data's time there, its key. A cut keeps the
    /// file's own metadata with the picture: the picture moves by the segment's shift in the file's time, and the motion
    /// data by the same shift on its own timeline (the file's time over the scale its metadata states, which gyroflow
    /// applies by itself, `load_gyro_data`: the same in the recording and in the video cut from it). So each point keeps
    /// its picture and its motion sample, and its offset follows from the two, whatever scale either project uses.
    /// A motion log of another file isn't cut with the video, its points would be off by the cut, and the video's segments
    /// don't apply to it (`load_gyro_data`): `None` unless the motion data of both is the files' own, and `None` for a
    /// project of another video than the recording
    pub fn offsets_from_source(&self, source_name: &str, project: &serde_json::Value) -> Option<BTreeMap<i64, f64>> {
        let url_of = |v: Option<&serde_json::Value>| -> String {
            let s = v.and_then(|x| x.as_str()).unwrap_or_default();
            if !s.is_empty() && !s.contains("://") { filesystem::path_to_url(s) } else { s.to_string() }
        };
        let project_video = url_of(project.get("videofile"));
        if project_video.is_empty() || filesystem::get_filename(&project_video) != source_name { return None; }
        let project_gyro = project.get("gyro_source")?;
        let project_motion = url_of(project_gyro.get("filepath"));
        // As `import_gyroflow_data` tells the video's own metadata from another file's
        if project_motion != project_video && !project_gyro.get("all_metadata").and_then(|x| x.as_bool()).unwrap_or_default() { return None; }
        let source_scale = project.get("video_info").and_then(|x| x.get("fps_scale")).and_then(|x| x.as_f64())
            .filter(|x| StabilizationParams::is_valid_fps_scale(*x)).unwrap_or(1.0);
        let offsets = project.get("offsets")?.as_object()?.iter().filter_map(|(k, v)| Some((k.parse::<i64>().ok()?, v.as_f64()?)));

        let (scale, fps) = { let p = self.params.read(); (p.get_fps_scale(), p.fps) };
        let gyro = self.gyro.read();
        let md = gyro.file_metadata.read();
        if !gyro.all_metadata || !md.has_motion() { return None; }
        let segments = md.trim_segments.as_ref()?;
        // The motion data's timeline is the file's time over this, as `load_gyro_data` scales the video for it (motion
        // data a project holds, not loaded from its file, as a load without the file's own frame rate scales it)
        let motion_scale = gyro.capture_frame_rate.or_else(|| Self::capture_frame_rate(md.frame_rate, fps, None))
            .filter(|_| fps > 0.0).map(|r| r / fps).filter(|x| StabilizationParams::is_valid_fps_scale(*x)).unwrap_or(1.0);
        let mut out = BTreeMap::new();
        for (key_us, offset_ms) in offsets {
            // The point's picture, in the recording file's µs
            let picture_us = (key_us as f64 + offset_ms * 1000.0) * source_scale;
            for s in segments.table.segments.iter().filter(|s| segments.table.source_of(s).name == source_name) {
                if picture_us >= s.src_start_us as f64 && picture_us < (s.src_start_us + s.duration_us) as f64 {
                    let shift_us = (s.out_start_us - s.src_start_us) as f64;
                    let video_us = (picture_us + shift_us) / scale;
                    let motion_us = (key_us as f64 + shift_us / motion_scale).round();
                    out.insert(motion_us as i64, (video_us - motion_us) / 1000.0);
                }
            }
        }
        Some(out)
    }

    /// The lens profile of `project`, a project made on a recording the video was cut from (`offsets_from_source`), for this
    /// video: a cut keeps the recording's picture, so the lens calibrated for it is this video's, while the picture is the
    /// same size (the lens metadata per frame is the video's own). `None` when the project has no profile or its video is
    /// of another size
    pub fn lens_from_source(&self, project: &serde_json::Value) -> Option<serde_json::Value> {
        let lens = project.get("calibration_data")?;
        let calibrated = lens.get("fisheye_params").and_then(|x| x.get("camera_matrix")).and_then(|x| x.as_array()).is_some_and(|x| !x.is_empty());
        let info = project.get("video_info")?;
        let size = (info.get("width")?.as_u64()? as usize, info.get("height")?.as_u64()? as usize);
        (calibrated && size == self.params.read().size).then(|| lens.clone())
    }

    pub fn get_new_videofile_url(org_video_url: &str, gf_file_url: Option<&str>, sequence_start: u32) -> String {
        if gf_file_url.is_some() && !filesystem::exists(org_video_url) {
            ::log::debug!("get_new_videofile_url: {org_video_url}");
            let folder = filesystem::get_folder(gf_file_url.unwrap());
            let filename = filesystem::get_filename(org_video_url);
            let mut filename_replaced = filename.clone();

            if let Some(num_pos) = filename.find('%') {
                if let Some(d_pos) = filename[num_pos+1..].find('d') {
                    if d_pos <= 5 {
                        let num_str = &filename[num_pos+1..num_pos+1+d_pos];
                        if let Ok(num) = num_str.parse::<u32>() {
                            let new_num = format!("{:01$}", sequence_start, num as usize);
                            let from = format!("%{}d", num_str);
                            filename_replaced = filename.replace(&from, &new_num);
                        }
                    }
                }
            }
            if filesystem::exists_in_folder(&folder, &filename_replaced) {
                return filesystem::get_file_url(&folder, &filename, false);
            }
        }
        org_video_url.to_string()
    }

    pub fn import_gyroflow_file<F: Fn(f64)>(&self, url: &str, blocking: bool, progress_cb: F, cancel_flag: Arc<AtomicBool>, is_plugin: bool) -> std::result::Result<serde_json::Value, GyroflowCoreError> {
        let data = filesystem::read(url)?;

        let mut is_preset = false;
        let result = self.import_gyroflow_data(&data, blocking, Some(url), progress_cb, cancel_flag, &mut is_preset, is_plugin);
        if !is_preset && result.is_ok() {
            self.input_file.write().project_file_url = Some(url.to_string());
        }
        result
    }
    pub fn import_gyroflow_data<F: Fn(f64)>(&self, data: &[u8], blocking: bool, url: Option<&str>, progress_cb: F, cancel_flag: Arc<AtomicBool>, is_preset: &mut bool, is_plugin: bool) -> std::result::Result<serde_json::Value, GyroflowCoreError> {
        let mut obj: serde_json::Value = serde_json::from_slice(&data)?;
        let mut load_options = gyro_source::FileLoadOptions::default();
        if let serde_json::Value::Object(ref mut obj) = obj {
            let mut output_size = None;
            load_options.project_version = obj.get("version").and_then(|x| x.as_u64()).unwrap_or(2);
            let mut org_video_url = obj.get("videofile").and_then(|x| x.as_str()).unwrap_or(&"").to_string();
            if !org_video_url.is_empty() && !org_video_url.contains("://") {
                org_video_url = filesystem::path_to_url(&org_video_url);
                if let Some(videofile) = obj.get_mut("videofile") {
                    *videofile = serde_json::Value::String(org_video_url.clone());
                }
            }
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            if let Some(v) = obj.get("videofile_bookmark").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) {
                let (resolved, _is_stale) = filesystem::apple::resolve_bookmark(v, url);
                if !resolved.is_empty() { org_video_url = resolved; }
            }

            let full_data_included = blocking && Self::project_has_motion_data(&data);

            let sequence_start = obj.get("image_sequence_start").and_then(|x| x.as_i64()).unwrap_or_default() as u32;

            let video_url = if full_data_included {
                org_video_url.clone()
            } else {
                Self::get_new_videofile_url(&org_video_url, url, sequence_start)
            };
            if let Some(videofile) = obj.get_mut("videofile") {
                *videofile = serde_json::Value::String(video_url.clone());
            }
            *is_preset = org_video_url.is_empty();
            if !*is_preset {
                // A project replaces what an analysis still running measures, and what the last one measured
                self.invalidate_optical_measurements();
            }

            if let Some(vid_info) = obj.get("video_info") {
                let mut params = self.params.write();
                if let Some(w) = vid_info.get("width").and_then(|x| x.as_u64()) {
                    if let Some(h) = vid_info.get("height").and_then(|x| x.as_u64()) {
                        params.size = (w as usize, h as usize);
                    }
                }
                output_size = Some(params.size);
                if let Some(v) = vid_info.get("rotation")   .and_then(|x| x.as_f64()) { params.video_rotation = v; }
                if let Some(v) = vid_info.get("num_frames") .and_then(|x| x.as_u64()) { params.frame_count    = v as usize; }
                if let Some(v) = vid_info.get("fps")        .and_then(|x| x.as_f64()) { params.fps            = v; }
                if let Some(v) = vid_info.get("duration_ms").and_then(|x| x.as_f64()) { params.duration_ms    = v; }
                if let Some(v) = vid_info.get("fps_scale") { params.set_fps_scale(v.as_f64()); }

                // The motion data takes the project's timing in, but isn't integrated again for it here, under the lock
                // every reader of the parameters waits on: the project's own motion data and integration settings are
                // loaded below, and every import ends with an integration of all of it (`recompute_gyro`: here when
                // `blocking`, the app's and the render queue's after it)
                let _ = self.gyro.write().set_video_timing(&params);
                self.keyframes.write().timestamp_scale = params.fps_scale;
            }
            let mut ignore_file_motion = None;
            // Where the video's frame times are read from: the project's video, or the file loaded as its own metadata
            let mut frame_times_url = if full_data_included { org_video_url.clone() } else { video_url.clone() };
            if let Some(serde_json::Value::Object(obj)) = obj.get_mut("gyro_source") {
                let mut org_gyro_url = obj.get("filepath").and_then(|x| x.as_str()).unwrap_or(&"").to_string();
                if !org_gyro_url.is_empty() && !org_gyro_url.contains("://") {
                    org_gyro_url = filesystem::path_to_url(&org_gyro_url);
                    if let Some(filepath) = obj.get_mut("filepath") {
                        *filepath = serde_json::Value::String(org_gyro_url.clone());
                    }
                }
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                if let Some(v) = obj.get("filepath_bookmark").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) {
                    let (resolved, _is_stale) = filesystem::apple::resolve_bookmark(v, url);
                    if !resolved.is_empty() { org_gyro_url = resolved; }
                }
                let gyro_url = if full_data_included {
                    org_gyro_url.clone()
                } else {
                    Self::get_new_videofile_url(&org_gyro_url, url.clone(), sequence_start)
                };
                if let Some(fp) = obj.get_mut("filepath") {
                    *fp = serde_json::Value::String(gyro_url.clone());
                }
                use crate::gyro_source::TimeIMU;

                let is_compressed = obj.get("raw_imu").map(|x| x.is_string()).unwrap_or_default();
                // Another file loaded with all of its metadata is the video's own too, so its frame timing stays (a
                // CRM on its proxy): a project with no motion data of its own loads it again
                let is_main_video = org_gyro_url == org_video_url || obj.get("all_metadata").and_then(|x| x.as_bool()).unwrap_or_default();
                // The video's own where that file isn't there anymore (the project has its motion data)
                if is_main_video && !gyro_url.is_empty() && filesystem::exists(&gyro_url) {
                    frame_times_url = gyro_url.clone();
                }

                let built_in_gyro: std::io::Result<crate::gyro_source::FileMetadata> = util::decompress_from_base91_cbor(obj.get("file_metadata").and_then(|x| x.as_str()).unwrap_or_default());

                // Load IMU data only if it's from another file or we are sure that built_in_gyro contains motion data
                if (!org_gyro_url.is_empty() && org_gyro_url != org_video_url) || built_in_gyro.as_ref().map(|x| x.has_motion()).unwrap_or_default() {
                    let mut raw_imu = Vec::new();
                    let mut quaternions = TimeQuat::default();
                    let mut image_orientations = None;
                    let mut gravity_vectors = None;
                    if is_compressed {
                        if let Some(bytes) = util::decompress_from_base91(obj.get("raw_imu").and_then(|x| x.as_str()).unwrap_or_default()) {
                            if let Ok(data) = bincode::serde::decode_from_slice::<Vec<TimeIMU>, _>(&bytes, bincode::config::legacy()) {
                                raw_imu = data.0;
                            }
                        }
                        if let Some(bytes) = util::decompress_from_base91(obj.get("quaternions").and_then(|x| x.as_str()).unwrap_or_default()) {
                            if let Ok(data) = bincode::serde::decode_from_slice::<TimeQuat, _>(&bytes, bincode::config::legacy()) {
                                quaternions = data.0;
                            }
                        }
                        if let Some(bytes) = util::decompress_from_base91(obj.get("image_orientations").and_then(|x| x.as_str()).unwrap_or_default()) {
                            if let Ok(data) = bincode::serde::decode_from_slice::<TimeQuat, _>(&bytes, bincode::config::legacy()) {
                                image_orientations = Some(data.0);
                            }
                        }
                        if let Some(bytes) = util::decompress_from_base91(obj.get("gravity_vectors").and_then(|x| x.as_str()).unwrap_or_default()) {
                            if let Ok(data) = bincode::serde::decode_from_slice::<TimeVec, _>(&bytes, bincode::config::legacy()) {
                                gravity_vectors = Some(data.0);
                            }
                        }
                    } else {
                        if let Some(ri) = obj.get("raw_imu") {
                            if ri.is_array() {
                                raw_imu = serde_json::from_value(ri.clone()).unwrap_or_default();
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
                            })
                            .unwrap_or_default();
                    }

                    if !raw_imu.is_empty() {
                        let md = match built_in_gyro {
                            // The motion data is the arrays', the rest of the file's metadata (its segments, lens and
                            // per-frame data) is what the project has of it
                            Ok(mut md) => {
                                md.raw_imu = raw_imu;
                                md.quaternions = quaternions;
                                md.gravity_vectors = gravity_vectors;
                                md.image_orientations = image_orientations;
                                md
                            }
                            Err(_) => crate::gyro_source::FileMetadata {
                                imu_orientation: obj.get("imu_orientation").and_then(|x| x.as_str().map(|x| x.to_string())),
                                detected_source: Some(obj.get("detected_source").and_then(|x| x.as_str()).unwrap_or("Gyroflow file").to_string()),
                                quaternions,
                                gravity_vectors,
                                image_orientations,
                                raw_imu,
                                ..Default::default()
                            },
                        };

                        // Integrated with the lock released: every reader of the motion data would wait for it
                        let integration = self.gyro.write().load_from_telemetry_unintegrated(md);
                        if let Some(integration) = integration { GyroSource::integrate_shared(&self.gyro, integration); }
                    } else if let Ok(md) = built_in_gyro {
                        let integration = self.gyro.write().load_from_telemetry_unintegrated(md);
                        if let Some(integration) = integration { GyroSource::integrate_shared(&self.gyro, integration); }
                    } else if filesystem::exists(&gyro_url) && blocking {
                        let mut file = filesystem::open_file(&gyro_url, false, false)?;
                        let filesize = file.size;
                        if let Err(e) = self.load_motion_file(file.get_file(), filesize, &gyro_url, is_main_video, &load_options, progress_cb, cancel_flag.clone(), FrameTimesFrom::Caller) {
                            ::log::warn!("Failed to load gyro data from {:?}: {:?}", gyro_url, e);
                        }
                    }
                } else if filesystem::exists(&gyro_url) && blocking {
                    let mut file = filesystem::open_file(&gyro_url, false, false)?;
                    let filesize = file.size;
                    if let Err(e) = self.load_motion_file(file.get_file(), filesize, &gyro_url, is_main_video, &load_options, progress_cb, cancel_flag.clone(), FrameTimesFrom::Caller) {
                        ::log::warn!("Failed to load gyro data from {:?}: {:?}", gyro_url, e);
                    }
                }

                // The settings under the lock, the integration the correction needs with it released, and before the
                // parameters' lock is taken: the parameters are locked before the motion data, as everywhere
                let correction_loaded = {
                    let mut gyro = self.gyro.write();
                    if !org_gyro_url.is_empty() {
                        gyro.file_url = gyro_url.clone();
                        gyro.all_metadata = is_main_video;
                    }

                    if let Some(v) = obj.get("lpf").and_then(|x| x.as_f64()) { gyro.imu_transforms.imu_lpf = v; }
                    if let Some(v) = obj.get("mf").and_then(|x| x.as_i64()) { gyro.imu_transforms.imu_mf = v as _; }
                    if let Some(v) = obj.get("integration_method").and_then(|x| x.as_u64()) { gyro.integration_method = v as usize; }
                    if let Some(v) = obj.get("imu_orientation").and_then(|x| x.as_str()) { gyro.imu_transforms.imu_orientation = Some(v.to_string()); }
                    if let Some(v) = obj.get("rotation")     { let v: [f64; 3] = serde_json::from_value(v.clone()).unwrap_or_default(); gyro.imu_transforms.set_imu_rotation(v[0], v[1], v[2]); }
                    if let Some(v) = obj.get("acc_rotation") { let v: [f64; 3] = serde_json::from_value(v.clone()).unwrap_or_default(); gyro.imu_transforms.set_acc_rotation(v[0], v[1], v[2]); }
                    if let Some(v) = obj.get("gyro_bias")    { gyro.imu_transforms.gyro_bias = serde_json::from_value(v.clone()).ok(); }
                    if let Some(v) = obj.get("optical_correction_strength").and_then(|x| x.as_f64()) { self.optical_settings.write().strength = v; }
                    if let Some(v) = obj.get("optical_correction_per_frame").and_then(|x| x.as_bool()) { self.optical_settings.write().per_frame = v; }
                    if let Some(v) = obj.get("optical_tracking").and_then(|x| x.as_str()).and_then(gyro_source::TrackingMethod::from_name) { *self.optical_tracking.write() = v; }
                    if let Some(v) = obj.get("optical_removed_tracks") { *self.optical_removed_tracks.write() = serde_json::from_value(v.clone()).unwrap_or_default(); }
                    ignore_file_motion = obj.get("ignore_file_motion").and_then(|x| x.as_bool());
                    if let Ok(mut c) = util::decompress_from_base91_cbor::<crate::gyro_source::OpticalCorrection>(obj.get("optical_correction").and_then(|x| x.as_str()).unwrap_or_default()) {
                        if let Some(v) = obj.get("optical_correction_enabled").and_then(|x| x.as_bool()) { c.enabled = v; }
                        gyro.optical_correction = Some(Arc::new(c));
                        true
                    } else {
                        false
                    }
                };
                if correction_loaded {
                    // Integrated with the settings just read, for the checksum it's matched against. Whether it applies
                    // also depends on what the rest of the project sets (the sync, the lens, the frame timing): the next
                    // recompute brings that up to date, see `refresh_optical_correction`
                    GyroSource::integrate_shared(&self.gyro, Integration::ApplyTransforms);
                }

                {
                    // The curves of a project file only stand in until the next recompute: they may come from another build
                    // (older ones stored millimetres, the renderer projects with output pixels) or from other lens metadata
                    // or settings than the ones loaded now. The base curve they'd be derived from is not stored, so the
                    // next recompute extracts it (and derives the curves it computed itself) from the loaded data
                    let mut params = self.params.write();
                    if let Ok(fls) = util::decompress_from_base91_cbor::<Vec<Option<f64>>>(obj.get("focal_lengths").and_then(|x| x.as_str()).unwrap_or_default()) {
                        params.focal_lengths = fls;
                    }
                    if let Ok(fls) = util::decompress_from_base91_cbor::<Vec<Option<f64>>>(obj.get("smoothed_focal_lengths").and_then(|x| x.as_str()).unwrap_or_default()) {
                        params.smoothed_focal_lengths = fls;
                    }
                    params.focal_length_base = Default::default();
                    params.focal_length_base_key = 0;
                }

                obj.remove("raw_imu");
                obj.remove("quaternions");
                obj.remove("smoothed_quaternions");
                obj.remove("image_orientations");
                obj.remove("gravity_vectors");
                obj.remove("file_metadata");
                obj.remove("focal_lengths");
                obj.remove("smoothed_focal_lengths");
                obj.remove("optical_correction");
            }
            if let Some(lens) = obj.get("calibration_data") {
                let mut l = self.lens.write();
                l.load_from_json_value(&lens);
                let db = self.lens_profile_db.read();
                l.resolve_interpolations(&db);
            }
            // Set once the parameters' and the smoothing's locks are let go of: either may integrate the motion data again
            let (mut use_gravity_vectors, mut horizon_lock_integration_method) = (None, None);
            if let Some(serde_json::Value::Object(obj)) = obj.get_mut("stabilization") {
                let mut params = self.params.write();
                if let Some(v) = obj.get("fov")                   .and_then(|x| x.as_f64()) { params.fov                     = v; }
                if let Some(v) = obj.get("frame_readout_time")    .and_then(|x| x.as_f64()) { params.frame_readout_time      = v; if v < 0.0 { params.frame_readout_direction = ReadoutDirection::BottomToTop; } }
                if let Some(v) = obj.get("frame_readout_direction").and_then(|x| x.as_i64()) { params.frame_readout_direction = (v as i32).into(); }
                if let Some(v) = obj.get("frame_readout_direction").and_then(|x| x.as_str()) { params.frame_readout_direction = v.into(); }
                if let Some(v) = obj.get("adaptive_zoom_window")  .and_then(|x| x.as_f64()) { params.adaptive_zoom_window    = v; }
                if let Some(v) = obj.get("lens_correction_amount").and_then(|x| x.as_f64()) { params.lens_correction_amount  = v; }
                // A project from before the setting draws rectilinear, the way it was made, and so does one with a name this
                // version doesn't know, whatever the file open before it was set to. A preset without it leaves it alone
                match obj.get("output_projection").and_then(|x| x.as_str()).and_then(stabilization::projection::setting_from_name) {
                    Some(v) => params.output_projection = v,
                    None => if !*is_preset { params.output_projection = stabilization::projection::OutputProjection::Rectilinear as i32; }
                }
                if let Some(v) = obj.get("frame_offset")          .and_then(|x| x.as_i64()) { params.frame_offset            = v as i32; }
                if let Some(v) = obj.get("horizontal_rs")         .and_then(|x| x.as_bool()) { if v { params.frame_readout_direction = if params.frame_readout_time < 0.0 { ReadoutDirection::RightToLeft } else { ReadoutDirection::LeftToRight }; } }
                if let Some(v) = obj.get("max_zoom")              .and_then(|x| x.as_f64()) { params.max_zoom                = Some(v); }
                if let Some(v) = obj.get("max_zoom_iterations")   .and_then(|x| x.as_i64()) { params.max_zoom_iterations     = v as _; }
                // A project, or a preset with zoom settings, from before these keys: automatic zoom level, and the zoom limit
                // lowering the smoothing of the whole clip, so it renders the way it was made. A preset without zoom settings
                // leaves them alone
                let old_zoom_settings = !*is_preset || ["adaptive_zoom_window", "max_zoom", "max_zoom_iterations"].iter().any(|x| obj.get(*x).is_some());
                match obj.get("fixed_zoom").and_then(|x| x.as_bool()) {
                    Some(v) => params.fixed_zoom = v,
                    None => if old_zoom_settings { params.fixed_zoom = false; }
                }
                match obj.get("zoom_limit_whole_clip").and_then(|x| x.as_bool()) {
                    Some(v) => { params.zoom_limit_whole_clip = v; params.zoom_limit_whole_clip_from_old_file = false; }
                    None => if old_zoom_settings { params.zoom_limit_whole_clip = true; params.zoom_limit_whole_clip_from_old_file = true; }
                }

                if let Some(v) = obj.get("video_speed").and_then(|x| x.as_f64()) { params.video_speed = v; }
                if let Some(v) = obj.get("video_speed_affects_smoothing")    .and_then(|x| x.as_bool()) { params.video_speed_affects_smoothing     = v; }
                if let Some(v) = obj.get("video_speed_affects_zooming")      .and_then(|x| x.as_bool()) { params.video_speed_affects_zooming       = v; }
                if let Some(v) = obj.get("video_speed_affects_zooming_limit").and_then(|x| x.as_bool()) { params.video_speed_affects_zooming_limit = v; }

                if let Some(center_offs) = obj.get("adaptive_zoom_center_offset").and_then(|x| x.as_array()) {
                    params.adaptive_zoom_center_offset = (
                        center_offs.get(0).and_then(|x| x.as_f64()).unwrap_or_default(),
                        center_offs.get(1).and_then(|x| x.as_f64()).unwrap_or_default()
                    );
                }
                if let Some(x) = obj.get("additional_rotation").and_then(|x| x.as_array()) {
                    params.additional_rotation = (
                        x.get(0).and_then(|x| x.as_f64()).unwrap_or_default(),
                        x.get(1).and_then(|x| x.as_f64()).unwrap_or_default(),
                        x.get(2).and_then(|x| x.as_f64()).unwrap_or_default()
                    );
                }
                if let Some(x) = obj.get("additional_translation").and_then(|x| x.as_array()) {
                    params.additional_translation = (
                        x.get(0).and_then(|x| x.as_f64()).unwrap_or_default(),
                        x.get(1).and_then(|x| x.as_f64()).unwrap_or_default(),
                        x.get(2).and_then(|x| x.as_f64()).unwrap_or_default()
                    );
                }
                if let Some(zooming_method) = obj.get("adaptive_zoom_method").and_then(|x| x.as_i64()) {
                    params.adaptive_zoom_method = zooming_method as i32;
                }

                if let Some(method) = obj.get("method").and_then(|x| x.as_str()) {
                    let method_idx = self.get_smoothing_algs()
                        .iter().enumerate()
                        .find(|(_, m)| method == m.as_str())
                        .map(|(idx, _)| idx)
                        .unwrap_or(1);

                    self.smoothing.write().set_current(method_idx);
                }

                let mut smoothing = self.smoothing.write();
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
                if let Some(horizon_amount) = obj.get("horizon_lock_amount").and_then(|x| x.as_f64()) {
                    if let Some(horizon_roll) = obj.get("horizon_lock_roll").and_then(|x| x.as_f64()) {
                        let horizon_pitch_enabled = obj.get("horizon_lock_pitch_enabled").and_then(|x| x.as_bool()).unwrap_or(false);
                        let horizon_pitch         = obj.get("horizon_lock_pitch")        .and_then(|x| x.as_f64()) .unwrap_or(0.0);
                        let turn_threshold        = obj.get("turn_threshold")            .and_then(|x| x.as_f64()) .unwrap_or(5.0);
                        let turn_smoothing_ms     = obj.get("turn_smoothing_ms")         .and_then(|x| x.as_f64()) .unwrap_or(500.0);
                        let turn_multiplier       = obj.get("turn_multiplier")           .and_then(|x| x.as_f64()) .unwrap_or(1.0);
                        let tilt_accel_limit      = obj.get("tilt_accel_limit")          .and_then(|x| x.as_f64()) .unwrap_or(f64::INFINITY);
                        let automatic_lock        = obj.get("automatic_lock")            .and_then(|x| x.as_bool()).unwrap_or(false);
                        smoothing.horizon_lock.set_horizon(horizon_amount, horizon_roll, horizon_pitch_enabled, horizon_pitch, automatic_lock, turn_threshold, turn_smoothing_ms, turn_multiplier, tilt_accel_limit);
                    }
                }
                use_gravity_vectors = obj.get("use_gravity_vectors").and_then(|x| x.as_bool());
                horizon_lock_integration_method = obj.get("horizon_lock_integration_method").and_then(|x| x.as_i64()).map(|v| v as i32);

                if let Some(v) = obj.get("focal_length_smoothing_enabled") .and_then(|x| x.as_bool()) { params.focal_length_smoothing_enabled  = v; }
                if let Some(v) = obj.get("focal_length_max_zoom_rate").and_then(|x| x.as_f64()) { params.focal_length_max_zoom_rate = v.clamp(0.01, 10.0); }
                if let Some(v) = obj.get("lens_metadata_delay_frames").and_then(|x| x.as_i64()) { params.lens_metadata_delay_frames = v.clamp(-30, 30) as i32; }
                if let Some(v) = obj.get("lens_breathing_enabled").and_then(|x| x.as_bool()) { params.lens_breathing_enabled = v; }

                obj.remove("adaptive_zoom_fovs");
            }
            if let Some(v) = use_gravity_vectors { self.set_use_gravity_vectors(v); }
            if let Some(v) = horizon_lock_integration_method { self.set_horizon_lock_integration_method(v); }
            if let Some(serde_json::Value::Object(obj)) = obj.get_mut("output") {
                if let Some(w) =  obj.get("output_width").and_then(|x| x.as_u64()) {
                    if let Some(h) =  obj.get("output_height").and_then(|x| x.as_u64()) {
                        output_size = Some((w as usize, h as usize));
                        if *is_preset {
                            self.input_file.write().preset_output_size = Some((w as usize, h as usize));
                        }
                    }
                }
                if is_plugin {
                    if let Some(i) = obj.get("interpolation").and_then(|x| x.as_str()) {
                        self.stabilization.write().interpolation = i.into();
                    }
                }
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                if let Some(v) = obj.get("output_folder_bookmark").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) {
                    let (resolved, _is_stale) = filesystem::apple::resolve_bookmark(v, url);
                    if !resolved.is_empty() {
                        filesystem::folder_access_granted(&resolved);
                        obj.insert("output_folder".into(), serde_json::Value::String(resolved));
                    }
                }
            }

            if let Some(serde_json::Value::Object(offsets)) = obj.get("offsets") {
                let mut gyro = self.gyro.write();
                gyro.set_offsets(offsets.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_f64()?))).collect());
                self.keyframes.write().update_gyro(&gyro);
            }
            obj.remove("offsets");
            // After the sync points, which it sets aside with the motion data
            if let Some(v) = ignore_file_motion {
                self.set_ignore_file_motion(v);
            }

            if let Some(keyframes) = obj.get("keyframes") {
                self.keyframes.write().deserialize(keyframes);
            }

            if let Some(start) = obj.get("trim_start").and_then(|x| x.as_f64()) {
                if let Some(end) = obj.get("trim_end").and_then(|x| x.as_f64()) {
                    self.params.write().trim_ranges = vec![(start, end)];
                }
            }

            let duration_ms = self.params.read().duration_ms.max(1.0);
            if let Some(ranges) = obj.get("trim_ranges_ms").and_then(|x| x.as_array()) {
                let ranges = ranges.iter().filter_map(|x| {
                    let x = x.as_array()?;
                    if x.len() == 2 {
                        let mut end_range = x[1].as_f64()?;
                        if end_range < 0.0 {
                            end_range = duration_ms + end_range;
                        }
                        Some((x[0].as_f64()? / duration_ms, end_range / duration_ms))
                    } else {
                        None
                    }
                }).collect::<Vec<_>>();
                self.params.write().trim_ranges = ranges;
            } else if let Some(ranges) = obj.get("trim_ranges").and_then(|x| x.as_array()) {
                // Deprecated
                let ranges = ranges.iter().filter_map(|x| {
                    let x = x.as_array()?;
                    if x.len() == 2 {
                        Some((x[0].as_f64()?, x[1].as_f64()?))
                    } else {
                        None
                    }
                }).collect::<Vec<_>>();
                self.params.write().trim_ranges = ranges;
            }

            {
                let mut params = self.params.write();
                if let Some(v) = obj.get("background_color").and_then(|x| x.as_array()) {
                    if v.len() == 4 {
                        params.background = nalgebra::Vector4::new(
                            v[0].as_f64().unwrap_or_default() as f32,
                            v[1].as_f64().unwrap_or_default() as f32,
                            v[2].as_f64().unwrap_or_default() as f32,
                            v[3].as_f64().unwrap_or_default() as f32
                        );
                    }
                }
                if let Some(v) = obj.get("background_mode").and_then(|x| x.as_i64()) { params.background_mode = stabilization_params::BackgroundMode::from(v as i32); }
                if let Some(v) = obj.get("background_margin").and_then(|x| x.as_f64()) { params.background_margin = v; }
                if let Some(v) = obj.get("background_margin_feather").and_then(|x| x.as_f64()) { params.background_margin_feather = v; }
                if let Some(v) = obj.get("light_refraction_coefficient").and_then(|x| x.as_f64()) { params.light_refraction_coefficient = v; }
            }

            {
                let mut input_file = self.input_file.write();
                if *is_preset {
                    if let Some(name) = obj.get("name").and_then(|x| x.as_str()) {
                        input_file.preset_name = Some(name.into());
                    } else if let Some(url) = url {
                        input_file.preset_name = Some(filesystem::get_filename(url).replace(".gyroflow", ""));
                    } else {
                        input_file.preset_name = Some("Untitled".into());
                    }
                }
                if let Some(seq_start) = obj.get("image_sequence_start").and_then(|x| x.as_i64()) {
                    input_file.image_sequence_start = seq_start as i32;
                }
                if let Some(seq_fps) = obj.get("image_sequence_fps").and_then(|x| x.as_f64()) {
                    input_file.image_sequence_fps = seq_fps;
                }
                if !org_video_url.is_empty() {
                    if full_data_included {
                        input_file.url = org_video_url;
                    } else if filesystem::can_open_file(&video_url) {
                        input_file.url = video_url;
                    }
                }
            }
            // The project's video replaces the one before, and its frame times with it, read here however the project is
            // loaded: a blocking import is all a render of the project loads (the render queue, the CLI, a plugin), and the
            // app, which loads the project's video first, doesn't load the video's own metadata then (`load_gyro_data`
            // reads them with it), only the project's motion data file, when it has one that's there. Stored as
            // `load_motion_file` stores them: not by an import cancelled for the next video, whose own load sets them
            if !*is_preset {
                let times = Self::read_frame_times(&frame_times_url);
                let mut params = self.params.write();
                if !cancel_flag.load(SeqCst) { params.frame_times = times; }
            }

            if blocking {
                self.recompute_gyro();

                if let Some(output_size) = output_size {
                    if output_size.0 > 0 && output_size.1 > 0 {
                        self.set_output_size(output_size.0, output_size.1);
                    }
                }
                self.init_size();
                self.recompute_blocking();
            }
        }
        Ok(obj)
    }

    pub fn project_has_motion_data(data: &[u8]) -> bool {
        if let Ok(serde_json::Value::Object(obj)) = serde_json::from_slice(&data) {
            if let Some(serde_json::Value::Object(obj)) = obj.get("gyro_source") {
                let built_in_gyro: std::io::Result<crate::gyro_source::FileMetadata> = util::decompress_from_base91_cbor(obj.get("file_metadata").and_then(|x| x.as_str()).unwrap_or_default());
                if built_in_gyro.as_ref().map(|x| x.has_motion()).unwrap_or_default() {
                    return true;
                }

                // Compatibility with older formats
                let is_compressed = obj.get("raw_imu").map(|x| x.is_string()).unwrap_or_default();
                if is_compressed {
                    if let Some(bytes) = util::decompress_from_base91(obj.get("raw_imu").and_then(|x| x.as_str()).unwrap_or_default()) {
                        if let Ok(data) = bincode::serde::decode_from_slice::<Vec<crate::gyro_source::TimeIMU>, _>(&bytes, bincode::config::legacy()) {
                            if !data.0.is_empty() { return true; }
                        }
                    }
                    if let Some(bytes) = util::decompress_from_base91(obj.get("quaternions").and_then(|x| x.as_str()).unwrap_or_default()) {
                        if let Ok(data) = bincode::serde::decode_from_slice::<TimeQuat, _>(&bytes, bincode::config::legacy()) {
                            if !data.0.is_empty() { return true; }
                        }
                    }
                } else {
                    if let Some(ri) = obj.get("raw_imu") {
                        if let Some(ri) = ri.as_array() {
                            if !ri.is_empty() { return true; }
                        }
                    }
                    if let Some(x) = obj.get("quaternions").and_then(|x| x.as_object()) {
                        if !x.is_empty() { return true; }
                    }
                }
            }
        }
        false
    }

    pub fn load_video_file<T: Read + Seek>(&self, stream: &mut T, filesize: usize, url: &str, mut metadata: Option<telemetry_parser::util::VideoMetadata>, is_plugin: bool) -> Result<telemetry_parser::util::VideoMetadata, GyroflowCoreError> {
        if metadata.is_none() {
            metadata = Some(util::get_video_metadata(stream, filesize, url)?);
        }
        let metadata = metadata.unwrap();
        log::info!("Loading video file: {metadata:?}");

        if metadata.width > 0 && metadata.height > 0 && metadata.duration_s > 0.0 && metadata.fps > 0.0 {
            let video_size = (metadata.width as usize, metadata.height as usize);
            // The video's duration at its frame rate is a whole number of frames, which the product misses by the
            // rounding of the two (13.847166666666666 s at 29.970029970029973 fps is 415.00000000000006 frames): less
            // than a millionth of a frame over a whole one is that rounding, not a frame more
            let frames = metadata.duration_s * metadata.fps;
            let frame_count = (frames - 1e-6).ceil().max(1.0) as usize;

            self.init_from_video_data(metadata.duration_s * 1000.0, metadata.fps, frame_count, video_size);
            // The camera identifier is this video's, from its own metadata: a video without one (no metadata the parser
            // reads) isn't the previous video's camera, and mustn't autoload its lens profile below. The app starts every
            // video the same way (`clear`)
            *self.camera_id.write() = None;
            let _ = self.load_gyro_data(stream, filesize, url, true, &Default::default(), |_|(), Arc::new(AtomicBool::new(false)));

            let has_builtin_profile = {
                let gyro = self.gyro.read();
                let file_metadata = gyro.file_metadata.read();
                file_metadata.lens_profile.as_ref().map(|y| y.is_object()).unwrap_or_default()
            };

            let id_str = self.camera_id.read().as_ref().map(|v| v.get_identifier_for_autoload()).unwrap_or_default();
            if !id_str.is_empty() && !has_builtin_profile {
                if !self.lens_profile_db.read().loaded {
                    let mut db = self.lens_profile_db.write();
                    db.load_all();
                }
                // Not held over `load_lens_profile`, which reads the database again: a second read of a parking_lot lock
                // this thread holds deadlocks as soon as a writer queues up in between
                let in_db = self.lens_profile_db.read().contains_id(&id_str);
                if in_db {
                    match self.load_lens_profile(&id_str) {
                        Ok(_) => {
                            let (fr, frd) = { let lens = self.lens.read(); (lens.frame_readout_time, lens.frame_readout_direction) };
                            if let Some(fr) = fr {
                                let mut params = self.params.write();
                                params.frame_readout_time = fr.abs();
                                params.frame_readout_direction = frd.unwrap_or(if fr < 0.0 { ReadoutDirection::BottomToTop } else { ReadoutDirection::TopToBottom });
                            }
                        }
                        Err(e) => {
                            log::error!("An error occurred: {e:?}");
                            return Err(e);
                        }
                    }
                }
            }
            let mut output_width = metadata.width;
            let mut output_height = metadata.height;
            if let Some(output_dim) = self.lens.read().output_dimension.clone() {
                output_width = output_dim.w;
                output_height = output_dim.h;
            }
            self.set_size(video_size.0, video_size.1);
            self.set_output_size(output_width, output_height);

            // Apply default preset
            let local_path = lens_profile_database::LensProfileDatabase::get_path().join("default.gyroflow");
            let settings_path = settings::data_dir().join("lens_profiles").join("default.gyroflow");
            if settings_path.exists() {
                let _ = self.import_gyroflow_file(&settings_path.to_str().unwrap(), true, |_|(), Arc::new(AtomicBool::new(false)), is_plugin);
            } else if local_path.exists() {
                let _ = self.import_gyroflow_file(&local_path.to_str().unwrap(), true, |_|(), Arc::new(AtomicBool::new(false)), is_plugin);
            }
        }
        Ok(metadata)
    }

    pub fn set_device(&self, i: i32) {
        self.params.write().current_device = i;
        let mut l = self.stabilization.write();
        l.set_device(i as isize);
    }

    pub fn set_keyframe(&self, typ: &KeyframeType, timestamp_us: i64, value: f64) {
        self.keyframes.write().set(typ, timestamp_us, value);
        self.keyframes_updated(typ);
    }
    pub fn set_keyframe_easing(&self, typ: &KeyframeType, timestamp_us: i64, easing: Easing) {
        self.keyframes.write().set_easing(typ, timestamp_us, easing);
        self.keyframes_updated(typ);
    }
    pub fn keyframe_easing(&self, typ: &KeyframeType, timestamp_us: i64) -> Option<Easing> {
        self.keyframes.read().easing(typ, timestamp_us)
    }
    pub fn set_keyframe_timestamp(&self, typ: &KeyframeType, id: u32, timestamp_us: i64) {
        self.keyframes.write().set_timestamp(typ, id, timestamp_us);
        self.keyframes_updated(typ);
    }
    pub fn keyframe_id(&self, typ: &KeyframeType, timestamp_us: i64) -> Option<u32> {
        self.keyframes.read().id(typ, timestamp_us)
    }
    pub fn remove_keyframe(&self, typ: &KeyframeType, timestamp_us: i64) {
        self.keyframes.write().remove(typ, timestamp_us);
        self.keyframes_updated(typ);
    }
    pub fn clear_keyframes_type(&self, typ: &KeyframeType) {
        self.keyframes.write().clear_type(typ);
        self.keyframes_updated(typ);
    }
    pub fn keyframe_value_at_video_timestamp(&self, typ: &KeyframeType, timestamp_ms: f64) -> Option<f64> {
        self.keyframes.read().value_at_video_timestamp(typ, timestamp_ms)
    }
    pub fn is_keyframed(&self, typ: &KeyframeType) -> bool {
        self.keyframes.read().is_keyframed(typ)
    }
    fn keyframes_updated(&self, typ: &KeyframeType) {
        match typ {
            KeyframeType::VideoRotation |
            KeyframeType::ZoomingSpeed |
            KeyframeType::AdditionalTranslationX |
            KeyframeType::AdditionalTranslationY |
            KeyframeType::AdditionalTranslationZ |
            KeyframeType::ZoomingCenterX |
            KeyframeType::ZoomingCenterY => self.invalidate_zooming(),

            KeyframeType::LockHorizonAmount |
            KeyframeType::LockHorizonRoll |
            KeyframeType::LockHorizonPitchEnabled |
            KeyframeType::LockHorizonPitch |
            KeyframeType::AdditionalRotationX |
            KeyframeType::AdditionalRotationY |
            KeyframeType::AdditionalRotationZ |
            KeyframeType::SmoothingParamTimeConstant |
            KeyframeType::SmoothingParamTimeConstant2 |
            KeyframeType::SmoothingParamSmoothness |
            KeyframeType::SmoothingParamPitch |
            KeyframeType::SmoothingParamRoll |
            KeyframeType::SmoothingParamYaw => self.invalidate_smoothing(),
            _ => { }
        }
    }

    pub fn get_optimal_sync_points(&self, target_sync_points: usize, initial_offset: f64) -> Vec<f64> {
        let dur_ms = self.params.read().get_scaled_duration_ms();
        let mut trim_ranges: Vec<(f64, f64)> = {
            let params = self.params.read();
            if params.trim_ranges.is_empty() {
                vec![((0.0 - initial_offset)/1000.0, (dur_ms - initial_offset) / 1000.0)]
            } else {
                params.trim_ranges.iter().map(|x| ((x.0 * dur_ms - initial_offset) / 1000.0, (x.1 * dur_ms - initial_offset) / 1000.0)).collect()
            }
        };
        // In a video cut from recordings, the sync points are within its segments: the ranges are split at their boundaries
        if let Some(segments) = self.gyro.read().segment_ranges() {
            let boundaries: Vec<f64> = segments.video_starts_us.iter().skip(1).map(|t| (*t as f64 / 1000.0 - initial_offset) / 1000.0).collect();
            trim_ranges = trim_ranges.into_iter().flat_map(|(from, to)| {
                let cuts: Vec<f64> = std::iter::once(from).chain(boundaries.iter().copied().filter(|b| *b > from && *b < to)).chain(std::iter::once(to)).collect();
                cuts.windows(2).map(|w| (w[0], w[1])).collect::<Vec<_>>()
            }).collect();
        }

        if let Some(mut optsync) = synchronization::optimsync::OptimSync::new(&self.gyro.read()) {
            let (points, rank, ratio) = optsync.run(target_sync_points, trim_ranges);
            {
                let mut sync_data = self.sync_data.write();
                sync_data.rank = rank;
                sync_data.ratio = ratio;
            }
            points.iter().map(|x| (x + initial_offset) / dur_ms).filter(|&v| v >= 0.0 && v <= 1.0).collect()
        } else {
            Vec::new()
        }
    }
}

pub fn timestamp_at_frame(frame: i32, fps: f64) -> f64 { frame as f64 * 1000.0 / fps }
pub fn frame_at_timestamp(timestamp_ms: f64, fps: f64) -> i32 { (timestamp_ms * (fps / 1000.0)).round() as i32 }

pub fn run_threaded<F>(cb: F) where F: FnOnce() + Send + 'static {
    THREAD_POOL.spawn(cb);
}

/// Drop cached GPU pipelines on every worker thread of the render pool.
/// Used to reclaim VRAM after a render-queue job finishes.
pub fn clear_gpu_cache_on_all_workers() {
    THREAD_POOL.broadcast(|_| {
        stabilization::clear_gpu_cache_current_thread();
    });
}

use std::str::FromStr;
#[derive(Debug, Clone, PartialEq, ::serde::Serialize, ::serde::Deserialize)]
pub enum GyroflowProjectType {
    Simple,
    WithGyroData,
    WithProcessedData
}
impl FromStr for GyroflowProjectType {
    type Err = serde_json::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> { serde_json::from_str(&format!("\"{}\"", s)) }
}
impl ToString for GyroflowProjectType {
    fn to_string(&self) -> String { format!("{:?}", self) }
}

#[derive(thiserror::Error, Debug)]
pub enum GyroflowCoreError {
    #[error("No stabilization data at {0}. Make sure you called `ensure_ready_for_processing`")]
    NoStabilizationData(i64),

    #[error("Buffer too small")]
    BufferTooSmall,

    #[error("Size too small")]
    SizeTooSmall,

    #[error("Size mismatch ({0:?} != ({1:?})")]
    SizeMismatch((usize, usize), (usize, usize)),

    #[error("Invalid stride: {0} must be greater than width ({1})")]
    InvalidStride(i32, i32),

    #[error("Input buffer is empty")]
    InputBufferEmpty,

    #[error("Output buffer is empty")]
    OutputBufferEmpty,

    #[error("Failed to find cached wgpu in process_pixels. Key: {0}")]
    NoCachedWgpuInstance(String),

    #[error("No GPU backend for the GPU buffers, which the CPU can't read. Key: {0}")]
    NoBackendForBuffers(String),

    #[error("CUDA error {0}")]
    Cuda(String),

    #[error("Unsupported file format {0}")]
    UnsupportedFormat(String),

    #[error("Invalid data")]
    InvalidData,

    #[error("JSON error {0:?}")]
    JSONError(#[from] serde_json::Error),

    #[error("Filesystem error {0:?}")]
    FilesystemError(#[from] crate::filesystem::FilesystemError),

    #[error("IO error {0:?}")]
    IOError(#[from] std::io::Error),

    #[error("Unknown error")]
    Unknown
}
