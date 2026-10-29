// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::sync::atomic::{ AtomicBool, AtomicUsize, Ordering::Relaxed, Ordering::SeqCst };
use std::sync::Arc;
use std::borrow::Cow;
use parking_lot::RwLock;

use crate::StabilizationManager;
use crate::stabilization::ComputeParams;
use super::{ PoseEstimator, SyncParams, OpticalFlowMethod, OFPyrLK };
use super::optical_motion::{ PairKlt, KltDevice };
use super::optical_motion::OpticalMotionAnalysis;

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
    compute_params: Arc<RwLock<ComputeParams>>,
    cancel_flag: Arc<AtomicBool>,
    progress_cb: Option<Arc<Box<dyn Fn(f64, usize, usize) + Send + Sync + 'static>>>,
    finished_cb: Option<Arc<Box<dyn Fn(AutosyncResult) + Send + Sync + 'static>>>,

    sync_params: SyncParams,
    /// Where KLT tracks, for an optical flow method that uses it
    klt: Option<Arc<PairKlt>>,
    /// `estimate_lens_delay`: log focal length per frame of the metadata without any delay applied (`NaN` where
    /// unknown), the curve the analyzed windows were picked on and the estimate is aligned against
    lens_delay_meta_ln: Vec<f64>,
    /// `synchronize` by the optical analysis (offset method 3, `find_offset::bands`): the frames go to its tracking
    /// instead of to the pose estimator. None again once finished, or after it failed
    band_sync: bool,
    optical: parking_lot::Mutex<Option<OpticalMotionAnalysis>>,

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
        let klt_device = KltDevice::for_processing(params.current_device);

        let SyncParams {
            search_size,
            mut time_per_syncpoint,
            mut every_nth_frame,
            ..
        } = sync_params;

        if let Some(scale) = &fps_scale {
            time_per_syncpoint *= scale;
        }
        // Offset method 3 tracks every frame of a sync point, and needs them long enough for its tracks
        if mode == "synchronize" && sync_params.offset_method == 3 && stab.gyro.read().has_motion() {
            let min = super::find_offset::bands::min_window_ms(org_fps);
            if time_per_syncpoint < min {
                log::info!("Optical sync: {time_per_syncpoint:.0} ms per sync point is too short for its tracks, {min:.0} ms instead");
                time_per_syncpoint = min;
            }
            every_nth_frame = 1;
        }
        let mut frame_count = ((timestamps_fract.len() as f64 * (time_per_syncpoint / 1000.0) * org_fps).ceil() as usize).min(params.frame_count) / every_nth_frame as usize;

        drop(params);

        if duration_ms < 10.0 || frame_count < 2 || time_per_syncpoint < 10.0 || search_size < 10.0 { return Err(AutosyncError::InvalidParameters); }

        let mut ranges_us: Vec<(i64, i64)> = timestamps_fract.iter().map(|x| {
            let range = (
                ((x * org_duration_ms) - (time_per_syncpoint / 2.0)).max(0.0),
                ((x * org_duration_ms) + (time_per_syncpoint / 2.0)).min(org_duration_ms)
            );
            ((range.0 * 1000.0).round() as i64, (range.1 * 1000.0).round() as i64)
        }).collect();

        // The frames of a sync point of a video cut from recordings are those of its segment: across a segment boundary
        // the video and the motion jump to another moment. A window that reaches past the segment of the time asked for
        // is moved into it, with its length as far as the segment has it: the offsets are fitted per segment, and a window
        // cut short can be too short to be measured (offset method 3 lengthens its windows for its tracks). A frame is
        // taken by its time in the file (the optical analysis, `OpticalMotionAnalysis::feed_frame`) or by its time with
        // its time offset (the pose estimator, `feed_frame`), so a segment's frames are those between the midpoints to the
        // frames around it, by both times
        {
            let params = stab.params.read();
            let gyro = stab.gyro.read();
            let md = gyro.file_metadata.read();
            if let Some(segments) = md.trim_segments.as_ref().filter(|s| s.len() > 1) {
                let scale = params.get_fps_scale();
                // Frame `k`'s time in the file (ms), and with its time offset
                let at = |k: usize| { let t = params.timestamp_at_frame(k as i32); (t * scale, (t + md.frame_time_offset(k)) * scale) };
                for (range, x) in ranges_us.iter_mut().zip(timestamps_fract) {
                    let r = segments.segment_of_video_us((x * org_duration_ms * 1000.0).round() as i64);
                    let frames = segments.frames(r);
                    let (first, last) = (at(frames.start), at(frames.end - 1));
                    let from = if r == 0 { 0.0 } else { let p = at(frames.start - 1); ((p.0 + first.0) / 2.0).max((p.1 + first.1) / 2.0) };
                    let to = if r + 1 == segments.len() { org_duration_ms } else { let n = at(frames.end); ((last.0 + n.0) / 2.0).min((last.1 + n.1) / 2.0) };
                    let (from, to) = ((from * 1000.0).ceil() as i64, (to * 1000.0).floor() as i64);
                    let len = (range.1 - range.0).min(to - from).max(0);
                    let start = range.0.clamp(from, (to - len).max(from));
                    *range = (start, start + len);
                }
            }
        }

        if mode == "synchronize" && !stab.gyro.read().has_motion() {
            // If no gyro data in file, analyze the entire video
            ranges_us.clear();
            ranges_us.push((0, (org_duration_ms * 1000.0).round() as i64));
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
            let windows = super::lens_delay::zoom_ranges(&lens_delay_meta_ln, &comp_params, 1500.0, 3);
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

        // Offset method 3 for a file with motion data: the optical analysis tracks the sync points. Where it can't be
        // set up (DIS-only builds have none, a build without OpenCV no CPU tracker), rs-sync instead
        let mut sync_params = sync_params;
        let optical = if mode == "synchronize" && sync_params.offset_method == 3 && stab.gyro.read().has_motion() {
            let ranges_ms = ranges_us.iter().map(|(a, b)| (*a as f64 / 1000.0, *b as f64 / 1000.0)).collect();
            OpticalMotionAnalysis::for_sync(stab, ranges_ms, cancel_flag.clone()).map_err(|e| log::warn!("Optical sync can't be set up, rs-sync instead: {e}")).ok()
        } else {
            None
        };
        let band_sync = optical.is_some();
        // Without motion data the pose estimator's own path, for which method 3 is rs-sync
        if sync_params.offset_method == 3 && !band_sync { sync_params.offset_method = 2; }

        let estimator = stab.pose_estimator.clone();

        estimator.every_nth_frame.store(every_nth_frame.max(1) as u32, SeqCst);
        estimator.offset_method.store(sync_params.offset_method as u32, SeqCst);
        estimator.pose_method.store(sync_params.pose_method as u32, SeqCst);

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

        // Its GPU is looked for and set up while the decoder starts. Not for the optical analysis, which has its own: a
        // second set-up at the same time made a second device, and one hung in the driver held the other's lock
        let klt = (!band_sync && OpticalFlowMethod::uses_klt(sync_params.of_method as u32)).then(|| Arc::new(PairKlt::new(klt_device, OFPyrLK::MAX_FEATURES)));

        Ok(Self {
            frame_count,
            org_fps,
            scaled_fps,
            sync_params,
            klt,
            lens_delay_meta_ln,
            band_sync,
            optical: parking_lot::Mutex::new(optical),
            mode,
            ranges_us,
            scaled_ranges_us,
            estimator,
            fps_scale,
            total_read_frames: Arc::new(AtomicUsize::new(1)), // Start with 1 to keep the loader active until `finished_feeding_frames` overrides it with final value
            total_detected_frames: Arc::new(AtomicUsize::new(0)),
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
        if self.band_sync { return self.feed_optical(timestamp_us, width, height, stride, pixels); }
        let img = PoseEstimator::yuv_to_gray(width, height, stride as u32, pixels).map(Arc::new);
        if width > stride as u32 {
            width = stride as u32;
        }

        let method = self.sync_params.of_method as u32;
        let klt = self.klt.clone();
        let estimator = self.estimator.clone();
        let total_detected_frames = self.total_detected_frames.clone();
        let total_read_frames = self.total_read_frames.clone();
        let progress_cb = self.progress_cb.clone();
        let frame_count = self.frame_count;
        let scaled_fps = self.scaled_fps;
        let org_fps = self.org_fps;
        let compute_params = self.compute_params.clone();
        let cancel_flag = self.cancel_flag.clone();
        let needs_poses = self.mode != "estimate_lens_delay";
        if let Some(scale) = self.fps_scale {
            timestamp_us = (timestamp_us as f64 / scale) as i64;
        }

        {
            let compute_params = compute_params.read();
            let frame = compute_params.frame_at_timestamp(timestamp_us as f64 / 1000.0).max(0) as usize;
            timestamp_us += (compute_params.gyro.read().file_metadata.read().frame_time_offset(frame) * 1000.0).round() as i64;
        }

        if let Some(_current_range) = self.scaled_ranges_us.iter().find(|(from, to)| (*from..=*to).contains(&timestamp_us)) {
            self.total_read_frames.fetch_add(1, SeqCst);

            self.thread_pool.spawn(move || {
                if cancel_flag.load(Relaxed) {
                    total_detected_frames.fetch_add(1, SeqCst);
                    return;
                }
                if let Some(img) = img {
                    estimator.detect_features(frame_no, timestamp_us, img, width, height, method, klt);
                    total_detected_frames.fetch_add(1, SeqCst);

                    if needs_poses && frame_no % 7 == 0 {
                        estimator.process_detected_frames(org_fps, scaled_fps, &compute_params.read());
                        estimator.recalculate_gyro_data(org_fps, false);
                    }

                    if let Some(cb) = &progress_cb {
                        let d = total_detected_frames.load(SeqCst);
                        let t = total_read_frames.load(SeqCst).max(frame_count);
                        cb((d as f64 / t.max(1) as f64) * 0.58, d, t);
                    }
                } else {
                    log::warn!("Failed to get image {:?}", img);
                }
            });
        }
    }

    /// Offset method 3: the frame to the optical analysis, in decoding order on the caller's thread (it tracks each frame
    /// from the one before), at about the width it's made for
    fn feed_optical(&self, timestamp_us: i64, width: u32, height: u32, stride: usize, pixels: &[u8]) {
        if !self.cancel_flag.load(Relaxed) {
            let mut optical = self.optical.lock();
            if let Some(analysis) = optical.as_mut() {
                let fed = match super::find_offset::bands::shrink(width, height, stride, pixels) {
                    Some((w, h, small)) => analysis.feed_frame(timestamp_us, w, h, w as usize, &small),
                    None => analysis.feed_frame(timestamp_us, width, height, stride, pixels),
                };
                if let Err(e) = fed {
                    log::error!("Optical sync: {e}");
                    *optical = None;
                }
            }
        }
        // Read and done at once: `finished_feeding_frames` has nothing to wait for
        self.total_read_frames.fetch_add(1, SeqCst);
        let d = self.total_detected_frames.fetch_add(1, SeqCst) + 1;
        if let Some(cb) = &self.progress_cb {
            let t = self.frame_count.max(d);
            cb((d as f64 / t.max(1) as f64) * 0.58, d, t);
        }
    }

    /// Offset method 3: the offsets from what the optical analysis tracked, also from the negated initial offset when
    /// that's asked for (the one with more sync points found wins, then the one that fits better)
    fn finish_band_sync(&self) {
        let total = self.total_detected_frames.load(SeqCst);
        let progress = |p: f64| if let Some(cb) = &self.progress_cb { cb(0.6 + 0.4 * p, total, total) };
        progress(0.0);
        let analysis = self.optical.lock().take();
        let measured = analysis.filter(|_| !self.cancel_flag.load(SeqCst)).map(|a| a.finish());
        let offsets = match measured {
            Some(Ok(meas)) => {
                use super::find_offset::bands::find_offsets;
                let frame_ms = 1000.0 / self.scaled_fps.max(1e-9);
                let check_negative = self.sync_params.initial_offset_inv && self.sync_params.initial_offset.abs() > 1.0;
                let share = if check_negative { 0.5 } else { 1.0 };
                let offsets = find_offsets(&meas, &self.scaled_ranges_us, &self.sync_params, frame_ms, |p| progress(p * share), self.cancel_flag.clone());
                if check_negative {
                    let mut sync_params = self.sync_params.clone();
                    sync_params.initial_offset = -sync_params.initial_offset;
                    let negative = find_offsets(&meas, &self.scaled_ranges_us, &sync_params, frame_ms, |p| progress(0.5 + p * 0.5), self.cancel_flag.clone());
                    let cost = |o: &[(f64, f64, f64)]| o.iter().map(|x| x.2).sum::<f64>();
                    if negative.len() > offsets.len() || (negative.len() == offsets.len() && cost(&negative) < cost(&offsets)) { negative } else { offsets }
                } else {
                    offsets
                }
            },
            Some(Err(e)) => { log::warn!("Optical sync: {e}"); Vec::new() },
            None => Vec::new(),
        };
        if let Some(cb) = &self.finished_cb {
            cb(AutosyncResult::Offsets(offsets));
        }
        if let Some(cb) = &self.progress_cb {
            cb(1.0, total, total);
        }
    }

    pub fn finished_feeding_frames(&self) {
        while self.total_detected_frames.load(SeqCst) < self.total_read_frames.load(SeqCst) - 1 {
            std::thread::sleep(std::time::Duration::from_millis(100));
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
            return;
        }

        if self.band_sync { return self.finish_band_sync(); }

        self.estimator.process_detected_frames(self.org_fps, self.scaled_fps, &self.compute_params.read());
        self.estimator.recalculate_gyro_data(self.org_fps, true);
        self.estimator.cache_optical_flow(if offset_method == 1 { 2 } else { 1 });
        self.estimator.cleanup();

        let mut scaled_ranges_us = Cow::Borrowed(&self.scaled_ranges_us);

        if self.mode == "synchronize" && !self.compute_params.read().gyro.read().has_motion() {
            // If no gyro data in file, set the computed optical flow as gyro data
            let compute_params = self.compute_params.write();
            let estimated = self.estimator.estimated_gyro.read().values().cloned().collect::<Vec<_>>();
            compute_params.gyro.write().file_metadata.set_raw_imu(estimated);
            // The app's motion data: integrated with its lock released
            crate::gyro_source::GyroSource::integrate_shared(&compute_params.gyro, crate::gyro_source::Integration::ApplyTransforms);
            /*gyro.file_metadata.quaternions = self.estimator.estimated_quats.read().clone();
            gyro.integration_method = 0;
            gyro.integrate();*/
            let duration_ms = compute_params.gyro.read().duration_ms;

            let timestamps_fract = [0.5];
            let time_per_syncpoint = 500.0;

            scaled_ranges_us = Cow::Owned(timestamps_fract.into_iter().map(|x| (
                (((x * duration_ms) - (time_per_syncpoint / 2.0)).max(0.0)         * 1000.0 / self.fps_scale.unwrap_or(1.0)).round() as i64,
                (((x * duration_ms) + (time_per_syncpoint / 2.0)).min(duration_ms) * 1000.0 / self.fps_scale.unwrap_or(1.0)).round() as i64
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
    }

    pub fn on_progress<F>(&mut self, cb: F) where F: Fn(f64, usize, usize) + Send + Sync + 'static {
        self.progress_cb = Some(Arc::new(Box::new(cb)));
    }
    pub fn on_finished<F>(&mut self, cb: F) where F:  Fn(AutosyncResult) + Send + Sync + 'static {
        self.finished_cb = Some(Arc::new(Box::new(cb)));
    }
}
