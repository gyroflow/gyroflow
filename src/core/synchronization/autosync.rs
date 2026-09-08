// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::sync::atomic::{ AtomicBool, AtomicUsize, Ordering::Relaxed, Ordering::SeqCst };
use std::sync::Arc;
use std::borrow::Cow;
use parking_lot::RwLock;

use crate::StabilizationManager;
use crate::stabilization::ComputeParams;
use super::PoseEstimator;
use super::SyncParams;

/// What a finished process delivers, depending on its mode
pub enum AutosyncResult {
    /// (timestamp, offset, cost) per sync point: `synchronize` and `estimate_rolling_shutter`
    Offsets(Vec<(f64, f64, f64)>),
    /// `guess_imu_orientation`
    Orientation(Option<(String, f64)>),
    /// `estimate_lens_delay`, see `lens_delay`
    LensDelay(Option<super::lens_delay::LensDelayEstimate>),
}

/// Why a process could not be set up
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutosyncError {
    /// Too short a video, or sync parameters that leave nothing to analyze
    InvalidParameters,
    /// `estimate_lens_delay`: the lens metadata records no zoom to measure the delay on
    NoZoomInMetadata,
}

pub struct AutosyncProcess {
    frame_count: usize,
    scaled_fps: f64,
    org_fps: f64,
    fps_scale: Option<f64>,
    mode: String, // synchronize, guess_imu_orientation, estimate_rolling_shutter, estimate_lens_delay
    ranges_us: Vec<(i64, i64)>,
    scaled_ranges_us: Vec<(i64, i64)>,
    estimator: Arc<PoseEstimator>,
    total_read_frames: Arc<AtomicUsize>,
    total_detected_frames: Arc<AtomicUsize>,
    failed_input: Arc<AtomicBool>,
    estimated_gyro: bool,
    compute_params: Arc<RwLock<ComputeParams>>,
    cancel_flag: Arc<AtomicBool>,
    progress_cb: Option<Arc<Box<dyn Fn(f64, usize, usize) + Send + Sync + 'static>>>,
    finished_cb: Option<Arc<Box<dyn Fn(AutosyncResult) + Send + Sync + 'static>>>,

    sync_params: SyncParams,
    /// `estimate_lens_delay`: log focal length per frame of the metadata without any delay applied (`NaN` where
    /// unknown), the curve the analyzed windows were picked on and the estimate is aligned against
    lens_delay_meta_ln: Vec<f64>,

    thread_pool: rayon::ThreadPool,
}

impl AutosyncProcess {
    /// Sets the process up. For `estimate_lens_delay` this extracts the focal length curve of the whole clip to pick
    /// the frames to analyze, so call it off the UI thread
    pub fn from_manager(stab: &StabilizationManager, timestamps_fract: &[f64], sync_params: SyncParams, mode: String, cancel_flag: Arc<AtomicBool>) -> Result<Self, AutosyncError> {
        let params = stab.params.read();
        let org_fps = params.fps;
        let scaled_fps = params.get_scaled_fps();
        let org_duration_ms = params.duration_ms;
        let fps_scale = params.fps_scale;
        let duration_ms = params.get_scaled_duration_ms();

        let SyncParams {
            search_size,
            mut time_per_syncpoint,
            mut every_nth_frame,
            ..
        } = sync_params;
        every_nth_frame = every_nth_frame.max(1);

        if let Some(scale) = &fps_scale {
            time_per_syncpoint *= scale;
        }
        let mut frame_count = ((timestamps_fract.len() as f64 * (time_per_syncpoint / 1000.0) * org_fps).ceil() as usize).min(params.frame_count) / every_nth_frame as usize;

        drop(params);
        let estimated_gyro = {
            let gyro = stab.gyro.read();
            !gyro.has_motion() || gyro.file_metadata.read().optical_motion.as_ref().is_some_and(|d| d.estimated_gyro)
        };

        if duration_ms < 10.0 || frame_count < 2 || time_per_syncpoint < 10.0 || search_size < 10.0 { return Err(AutosyncError::InvalidParameters); }

        let mut ranges_us: Vec<(i64, i64)> = timestamps_fract.iter().map(|x| {
            let range = (
                ((x * org_duration_ms) - (time_per_syncpoint / 2.0)).max(0.0),
                ((x * org_duration_ms) + (time_per_syncpoint / 2.0)).min(org_duration_ms)
            );
            ((range.0 * 1000.0).round() as i64, (range.1 * 1000.0).round() as i64)
        }).collect();

        if mode == "synchronize" && (estimated_gyro || stab.params.read().optical_stabilization) {
            // If no gyro data in file, analyze the entire video
            ranges_us.clear();
            ranges_us.push((0, (org_duration_ms * 1000.0).round() as i64));
            frame_count = stab.params.read().frame_count / every_nth_frame.max(1);
        }

        let mut comp_params = ComputeParams::from_manager(stab);
        comp_params.keyframes.clear();
        // Make sure we apply full correction for autosync
        comp_params.lens_correction_amount = 1.0;

        let mut lens_delay_meta_ln = Vec::new();
        if mode == "estimate_lens_delay" {
            // The frames are picked by the lens metadata itself: the windows with the strongest zoom, every frame of
            // them, so the estimate doesn't depend on the synchronization (which files with accurate timestamps skip).
            // The curve is extracted once, from the process' own parameters without the delay under test and without
            // the adaptive zoom, and kept for the estimate
            comp_params.lens_metadata_delay_frames = 0;
            comp_params.fovs.clear();
            comp_params.minimal_fovs.clear();
            lens_delay_meta_ln = crate::smoothing::focal_length::compute_base_curve(&comp_params).iter().map(|x| x.ln()).collect();
            let windows = super::lens_delay::zoom_ranges(&lens_delay_meta_ln, scaled_fps, 1500.0, 3);
            if windows.is_empty() {
                log::warn!("Lens metadata delay: the lens metadata holds no zoom to estimate it from");
                return Err(AutosyncError::NoZoomInMetadata);
            }
            // The windows are in scaled time (frame index over the scaled fps), the ranges in the file's own time
            ranges_us = windows.iter().map(|(a, b)| ((a * fps_scale.unwrap_or(1.0) * 1000.0).round() as i64, (b * fps_scale.unwrap_or(1.0) * 1000.0).round() as i64)).collect();
            every_nth_frame = 1;
            frame_count = windows.iter().map(|(a, b)| ((b - a) / 1000.0 * scaled_fps).ceil() as usize).sum::<usize>().max(2);
        }

        let scaled_ranges_us = ranges_us.iter().map(|(f, t)| (
            (*f as f64 / fps_scale.unwrap_or(1.0)) as i64,
            (*t as f64 / fps_scale.unwrap_or(1.0)) as i64)
        ).collect();

        let estimator = stab.pose_estimator.clone();
        if mode == "synchronize" && (estimated_gyro || stab.params.read().optical_stabilization) {
            // A new full analysis cannot reuse matches from another sampling mode.
            estimator.clear();
        }

        estimator.every_nth_frame.store(every_nth_frame.max(1) as u32, SeqCst);
        estimator.offset_method.store(sync_params.offset_method as u32, SeqCst);
        estimator.pose_method.store(sync_params.pose_method as u32, SeqCst);
        estimator.optical_only.store(estimated_gyro, SeqCst);

        let thread_pool = rayon::ThreadPoolBuilder::new()
            .thread_name(move |i| format!("Sync {}", i))
            .stack_size(10 * 1024 * 1024) // 10 MB
            .panic_handler(move |e| {
                if let Some(s) = e.downcast_ref::<&str>() {
                    log::error!("Sync thread panic! {}", s);
                } else if let Some(s) = e.downcast_ref::<String>() {
                    log::error!("Sync thread panic! {}", s);
                } else {
                    log::error!("Sync thread panic! {:?}", e);
                }
            })
            .build().unwrap();

        Ok(Self {
            frame_count,
            org_fps,
            scaled_fps,
            sync_params,
            lens_delay_meta_ln,
            mode,
            ranges_us,
            scaled_ranges_us,
            estimator,
            fps_scale,
            total_read_frames: Arc::new(AtomicUsize::new(1)), // Start with 1 to keep the loader active until `finished_feeding_frames` overrides it with final value
            total_detected_frames: Arc::new(AtomicUsize::new(0)),
            failed_input: Arc::new(AtomicBool::new(false)),
            estimated_gyro,
            compute_params: Arc::new(RwLock::new(comp_params)),
            finished_cb: None,
            progress_cb: None,
            cancel_flag,
            thread_pool
        })
    }

    pub fn get_ranges(&self) -> Vec<(f64, f64)> {
        self.ranges_us.iter().map(|&v| (v.0 as f64 / 1000.0, v.1 as f64 / 1000.0)).collect()
    }

    pub fn feed_frame(&self, mut timestamp_us: i64, frame_no: usize, mut width: u32, height: u32, stride: usize, pixels: &[u8]) {
        let img = PoseEstimator::yuv_to_gray(width, height, stride as u32, pixels).map(Arc::new);
        if width > stride as u32 {
            width = stride as u32;
        }

        let method = self.sync_params.of_method as u32;
        let estimator = self.estimator.clone();
        let total_detected_frames = self.total_detected_frames.clone();
        let total_read_frames = self.total_read_frames.clone();
        let progress_cb = self.progress_cb.clone();
        let frame_count = self.frame_count;
        let scaled_fps = self.scaled_fps;
        let org_fps = self.org_fps;
        let compute_params = self.compute_params.clone();
        let cancel_flag = self.cancel_flag.clone();
        let failed_input = self.failed_input.clone();
        let needs_poses = self.mode != "estimate_lens_delay";
        if let Some(scale) = self.fps_scale {
            timestamp_us = (timestamp_us as f64 / scale) as i64;
        }
        let video_timestamp_us = timestamp_us;

        {
            let compute_params = compute_params.read();
            let frame = crate::frame_at_timestamp(timestamp_us as f64 / 1000.0, compute_params.scaled_fps) as usize;
            timestamp_us += (compute_params.gyro.read().file_metadata.read().per_frame_time_offsets.get(frame).unwrap_or(&0.0) * 1000.0).round() as i64;
        }

        if let Some(_current_range) = self.scaled_ranges_us.iter().find(|(from, to)| (*from..=*to).contains(&video_timestamp_us)) {
            self.total_read_frames.fetch_add(1, SeqCst);

            self.thread_pool.spawn(move || {
                struct Completed { count: Arc<AtomicUsize>, failed: Arc<AtomicBool>, success: bool }
                impl Drop for Completed {
                    fn drop(&mut self) {
                        if !self.success { self.failed.store(true, SeqCst); }
                        self.count.fetch_add(1, SeqCst);
                    }
                }
                // Count completion after all pose work and callbacks, including
                // rejected frames and unwinding. Finalization must not race them.
                let mut completed = Completed { count: total_detected_frames.clone(), failed: failed_input, success: false };
                if cancel_flag.load(Relaxed) { return; }
                if let Some(img) = img {
                    estimator.detect_features(frame_no, timestamp_us, img, width, height, method);

                    if needs_poses && frame_no % 7 == 0 {
                        estimator.process_detected_frames(org_fps, scaled_fps, &compute_params.read());
                        estimator.recalculate_gyro_data(org_fps, false);
                    }

                    if let Some(cb) = &progress_cb {
                        let d = total_detected_frames.load(SeqCst) + 1;
                        let t = total_read_frames.load(SeqCst).max(frame_count);
                        cb((d as f64 / t.max(1) as f64) * 0.58, d, t);
                    }
                    completed.success = true;
                } else {
                    log::warn!("Failed to get image {:?}", img);
                }
            });
        }
    }

    pub fn mark_decode_failed(&self) { self.failed_input.store(true, SeqCst); }

    /// False leaves the previously published motion untouched. Callers must not
    /// start a render after a failed/cancelled analysis.
    pub fn finished_feeding_frames(&self) -> bool {
        while self.total_detected_frames.load(SeqCst) < self.total_read_frames.load(SeqCst) - 1 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let optical_mode = self.mode == "synchronize" &&
            (self.compute_params.read().optical_stabilization || (self.estimated_gyro && self.sync_params.every_nth_frame == 1));
        let complete_frames = !optical_mode || {
            let frames = self.estimator.sync_results.read();
            let size = frames.values().next().map(|f| f.frame_size);
            frames.len() == self.frame_count && frames.len() == self.total_read_frames.load(SeqCst) - 1 &&
                frames.values().enumerate().all(|(i, f)| f.frame_no == i && Some(f.frame_size) == size)
        };
        if self.cancel_flag.load(SeqCst) || self.failed_input.load(SeqCst) || !complete_frames {
            self.estimator.cleanup();
            if let Some(cb) = &self.progress_cb { let n = self.total_detected_frames.load(SeqCst); cb(1.0, n, n); }
            return false;
        }

        let offset_method = self.sync_params.offset_method;

        let progress_cb = self.progress_cb.clone();

        if self.mode == "estimate_lens_delay" {
            // A cancelled analysis delivers no result, like `guess_imu_orientation`: `LensDelay(None)` means the
            // clip couldn't be analyzed, which the controller reports to the user. The final progress call below
            // is what ends the "in progress" state, so it's made either way
            let cancelled = self.cancel_flag.load(SeqCst);
            if !cancelled {
            // Only the feature tracks between consecutive frames are needed
            self.estimator.cache_optical_flow(1);
            }
            self.estimator.cleanup();
            if !cancelled {
            if let Some(cb) = &self.finished_cb {
                    cb(AutosyncResult::LensDelay(super::lens_delay::estimate(&self.estimator, &self.compute_params.read(), &self.lens_delay_meta_ln)));
                }
            }
            if let Some(cb) = &progress_cb {
                let len = self.total_detected_frames.load(SeqCst);
                cb(1.0, len, len);
            }
            return true;
        }

        self.estimator.process_detected_frames(self.org_fps, self.scaled_fps, &self.compute_params.read());
        self.estimator.recalculate_gyro_data(self.org_fps, true);
        self.estimator.cache_optical_flow(if offset_method == 1 { 2 } else { 1 });
        self.estimator.cleanup();

        let mut scaled_ranges_us = Cow::Borrowed(&self.scaled_ranges_us);

        if self.mode == "synchronize" && (self.compute_params.read().optical_stabilization || self.estimated_gyro) {
            use super::residual_motion::{ OpticalMotionData, OpticalMotionPair };
            let mut data = OpticalMotionData { version: 1,
                complete: self.sync_params.every_nth_frame == 1,
                estimated_gyro: self.estimated_gyro,
                discontinuities_us: Vec::new(), pairs: Vec::new() };
            let results = self.estimator.sync_results.read();
            let params = self.compute_params.read();
            let metadata = params.gyro.read().file_metadata.clone();
            let metadata = metadata.read();
            let file_time = |ts: i64| {
                let frame = crate::frame_at_timestamp(ts as f64 / 1000.0, self.scaled_fps) as usize;
                ((ts as f64 - metadata.per_frame_time_offsets.get(frame).unwrap_or(&0.0) * 1000.0) * self.fps_scale.unwrap_or(1.0)).round() as i64
            };
            for current in results.values() {
                if let Some(Some(((from_us, from), (to_us, to)))) = current.optical_flow.read().get(&1) {
                    data.pairs.push(OpticalMotionPair { from_us: file_time(*from_us), to_us: file_time(*to_us), size: current.frame_size, from: from.clone(), to: to.clone() });
                } else if let Some(next) = results.range((std::ops::Bound::Excluded(current.timestamp_us), std::ops::Bound::Unbounded)).next().map(|(_, v)| v) {
                    data.discontinuities_us.push(file_time(next.timestamp_us));
                }
            }
            drop(metadata);
            params.gyro.write().file_metadata.set_optical_motion(data);
        }

        if self.mode == "synchronize" && self.estimated_gyro {
            // If no gyro data in file, set the computed optical flow as gyro data
            let compute_params = self.compute_params.write();
            let mut gyro = compute_params.gyro.write();

            gyro.file_metadata.set_raw_imu(self.estimator.estimated_gyro.read().values().cloned().collect::<Vec<_>>());
            gyro.apply_transforms();

            let timestamps_fract = [0.5];
            let time_per_syncpoint = 500.0;

            scaled_ranges_us = Cow::Owned(timestamps_fract.into_iter().map(|x| (
                (((x * gyro.duration_ms) - (time_per_syncpoint / 2.0)).max(0.0)              * 1000.0 / self.fps_scale.unwrap_or(1.0)).round() as i64,
                (((x * gyro.duration_ms) + (time_per_syncpoint / 2.0)).min(gyro.duration_ms) * 1000.0 / self.fps_scale.unwrap_or(1.0)).round() as i64
            )).collect());
        }

        if let Some(cb) = &progress_cb {
            let d = self.total_detected_frames.load(SeqCst);
            let t = self.total_read_frames.load(SeqCst);
            cb(0.6, d, t);
        }

        let check_negative = self.sync_params.initial_offset_inv && self.sync_params.initial_offset.abs() > 1.0;

        let for_negative = AtomicBool::new(false);

        let progress_cb2 = |mut progress| {
            if let Some(cb) = &progress_cb {
                let d = self.total_detected_frames.load(SeqCst);
                let t = self.total_read_frames.load(SeqCst);
                if check_negative {
                    progress += if for_negative.load(SeqCst) { 1.0 } else { 0.0 };
                    progress /= 2.0;
                }
                cb(0.6 + (progress * 0.4), d, t);
            }
        };

        if let Some(cb) = &self.finished_cb {
            if self.mode == "estimate_rolling_shutter" {
                use super::find_offset::visual_features::find_offsets;
                cb(AutosyncResult::Offsets(find_offsets(&self.estimator, &scaled_ranges_us, &self.sync_params, &self.compute_params.read(), true, progress_cb2, self.cancel_flag.clone())));
            } else if self.mode == "guess_imu_orientation" {
                use super::find_offset::rs_sync::FindOffsetsRssync;
                let guessed = FindOffsetsRssync::new(&scaled_ranges_us, self.estimator.sync_results.clone(), &self.sync_params, &self.compute_params.read(), progress_cb2, self.cancel_flag.clone()).guess_orient();
                if !self.cancel_flag.load(SeqCst) {
                    cb(AutosyncResult::Orientation(guessed));
                }
            } else {
                let offsets = self.estimator.find_offsets(&scaled_ranges_us, &self.sync_params, &self.compute_params.read(), progress_cb2, self.cancel_flag.clone());
                if check_negative {
                    for_negative.store(true, SeqCst);
                    // Try also negative rough offset
                    let mut sync_params = self.sync_params.clone();
                    sync_params.initial_offset = -sync_params.initial_offset;
                    let offsets2 = self.estimator.find_offsets(&scaled_ranges_us, &sync_params, &self.compute_params.read(), progress_cb2, self.cancel_flag.clone());
                    if offsets2.len() > offsets.len() {
                        cb(AutosyncResult::Offsets(offsets2));
                    } else if offsets2.len() == offsets.len() {
                        let sum1: f64 = offsets.iter().map(|(_, _, cost)| *cost).sum();
                        let sum2: f64 = offsets2.iter().map(|(_, _, cost)| *cost).sum();
                        if sum1 < sum2 {
                            cb(AutosyncResult::Offsets(offsets));
                        } else {
                            cb(AutosyncResult::Offsets(offsets2));
                        }
                    }
                } else {
                    cb(AutosyncResult::Offsets(offsets));
                }
            }
        }
        if let Some(cb) = &self.progress_cb {
            let len = self.total_detected_frames.load(SeqCst);
            cb(1.0, len, len);
        }
        true
    }

    pub fn on_progress<F>(&mut self, cb: F) where F: Fn(f64, usize, usize) + Send + Sync + 'static {
        self.progress_cb = Some(Arc::new(Box::new(cb)));
    }
    pub fn on_finished<F>(&mut self, cb: F) where F:  Fn(AutosyncResult) + Send + Sync + 'static {
        self.finished_cb = Some(Arc::new(Box::new(cb)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synchronization::residual_motion::OpticalMotionData;

    fn manager() -> StabilizationManager {
        let manager = StabilizationManager::default();
        {
            let mut p = manager.params.write();
            p.fps = 30.0; p.duration_ms = 66.667; p.frame_count = 2;
            p.size = (32, 32); p.output_size = (32, 32); p.optical_stabilization = true;
        }
        manager.gyro.write().init_from_params(&manager.params.read());
        manager
    }
    fn sync(manager: &StabilizationManager, cancelled: Arc<AtomicBool>) -> AutosyncProcess {
        AutosyncProcess::from_manager(manager, &[0.5], SyncParams {
            search_size: 5000.0, time_per_syncpoint: 1000.0, every_nth_frame: 1, max_sync_points: 1, of_method: 2,
            ..Default::default()
        }, "synchronize".into(), cancelled).unwrap()
    }

    #[test]
    fn rejected_frames_complete_without_hanging_finalization() {
        let manager = manager();
        let process = sync(&manager, Arc::new(AtomicBool::new(false)));
        process.feed_frame(0, 0, 32, 32, 32, &[]);
        process.feed_frame(33_333, 1, 32, 32, 32, &[]);
        process.finished_feeding_frames();
        assert_eq!(process.total_detected_frames.load(SeqCst), 2);
        assert!(manager.gyro.read().file_metadata.read().optical_motion.is_none(), "Invalid frames must not publish completed analysis");
    }

    #[test]
    fn partial_and_duplicate_analysis_preserve_previous_project_motion() {
        for duplicate in [false, true] {
            let manager = manager();
            manager.gyro.write().file_metadata.set_optical_motion(OpticalMotionData { version: 42, complete: true, pairs: vec![], ..Default::default() });
            let process = sync(&manager, Arc::new(AtomicBool::new(false)));
            let pixels = [128u8; 32 * 32];
            process.feed_frame(0, 0, 32, 32, 32, &pixels);
            if duplicate { process.feed_frame(0, 1, 32, 32, 32, &pixels); }
            process.finished_feeding_frames();
            assert_eq!(manager.gyro.read().file_metadata.read().optical_motion.as_ref().unwrap().version, 42,
                "Partial or duplicated input must not replace the last complete analysis");
        }
    }

    #[test]
    fn cancelled_analysis_preserves_previous_project_motion() {
        let manager = manager();
        manager.gyro.write().file_metadata.set_optical_motion(OpticalMotionData { version: 42, complete: true, pairs: vec![], ..Default::default() });
        let cancelled = Arc::new(AtomicBool::new(false));
        let process = sync(&manager, cancelled.clone());
        process.feed_frame(0, 0, 32, 32, 32, &[]);
        cancelled.store(true, SeqCst);
        process.finished_feeding_frames();
        assert_eq!(manager.gyro.read().file_metadata.read().optical_motion.as_ref().unwrap().version, 42);
    }

    #[test]
    fn explicit_decoder_failure_does_not_publish_complete_frames() {
        let manager = manager();
        let process = sync(&manager, Arc::new(AtomicBool::new(false)));
        let pixels = [128u8; 32 * 32];
        process.feed_frame(0, 0, 32, 32, 32, &pixels);
        process.feed_frame(33_333, 1, 32, 32, 32, &pixels);
        process.mark_decode_failed();
        assert!(!process.finished_feeding_frames());
        assert!(manager.gyro.read().file_metadata.read().optical_motion.is_none());
    }

    #[test]
    fn complete_textureless_frames_are_valid_but_carry_no_invented_motion() {
        let manager = manager();
        let process = sync(&manager, Arc::new(AtomicBool::new(false)));
        let pixels = [128u8; 32 * 32];
        process.feed_frame(0, 0, 32, 32, 32, &pixels);
        process.feed_frame(33_333, 1, 32, 32, 32, &pixels);
        assert!(process.finished_feeding_frames());
        let gyro = manager.gyro.read();
        let md = gyro.file_metadata.read();
        let data = md.optical_motion.as_ref().unwrap();
        assert!(data.complete && data.estimated_gyro && data.pairs.is_empty());
        assert_eq!(data.discontinuities_us, vec![33_333]);
        assert!(md.raw_imu.iter().all(|s| s.gyro == Some([0.0; 3])));
    }

    #[test]
    fn negative_frame_metadata_offsets_do_not_drop_video_frames() {
        let manager = manager();
        manager.gyro.write().file_metadata = crate::gyro_source::FileMetadata {
            per_frame_time_offsets: vec![-3.0, -3.0], ..Default::default()
        }.into();
        let process = sync(&manager, Arc::new(AtomicBool::new(false)));
        let pixels = [128u8; 32 * 32];
        process.feed_frame(0, 0, 32, 32, 32, &pixels);
        process.feed_frame(33_333, 1, 32, 32, 32, &pixels);
        assert!(process.finished_feeding_frames());
        assert_eq!(process.total_detected_frames.load(SeqCst), 2);
        assert_eq!(manager.gyro.read().file_metadata.read().optical_motion.as_ref().unwrap().discontinuities_us, vec![33_333]);
    }

    #[test]
    fn reanalysis_replaces_old_estimated_gyro_instead_of_treating_it_as_sensor_data() {
        let manager = manager();
        manager.gyro.write().file_metadata = crate::gyro_source::FileMetadata {
            optical_motion: Some(OpticalMotionData { version: 1, complete: true, estimated_gyro: true, ..Default::default() }),
            raw_imu: vec![crate::gyro_source::TimeIMU { timestamp_ms: 0.0, gyro: Some([90.0, 0.0, 0.0]), accl: None, magn: None }],
            ..Default::default()
        }.into();
        let process = sync(&manager, Arc::new(AtomicBool::new(false)));
        let pixels = [128u8; 32 * 32];
        process.feed_frame(0, 0, 32, 32, 32, &pixels);
        process.feed_frame(33_333, 1, 32, 32, 32, &pixels);
        assert!(process.finished_feeding_frames());
        assert!(manager.gyro.read().file_metadata.read().raw_imu.iter().all(|s| s.gyro == Some([0.0; 3])));
    }
}
