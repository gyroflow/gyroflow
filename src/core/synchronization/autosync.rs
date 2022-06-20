// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::sync::atomic::{ AtomicBool, AtomicUsize, Ordering::Relaxed, Ordering::SeqCst };
use std::sync::Arc;
use itertools::Either;
use parking_lot::RwLock;

use crate::StabilizationManager;
use crate::stabilization::ComputeParams;
use super::PoseEstimator;
use super::SyncParams;

pub struct AutosyncProcess {
    frame_count: usize,
    scaled_fps: f64,
    org_fps: f64,
    fps_scale: Option<f64>,
    mode: String, // synchronize, guess_imu_orientation, estimate_rolling_shutter
    ranges_us: Vec<(i64, i64)>,
    scaled_ranges_us: Vec<(i64, i64)>,
    estimator: Arc<PoseEstimator>,
    total_read_frames: Arc<AtomicUsize>,
    total_detected_frames: Arc<AtomicUsize>,
    compute_params: Arc<RwLock<ComputeParams>>,
    cancel_flag: Arc<AtomicBool>,
    progress_cb: Option<Arc<Box<dyn Fn(f64, usize, usize) + Send + Sync + 'static>>>,
    finished_cb: Option<Arc<Box<dyn Fn(Either<Vec<(f64, f64, f64)>, Option<(String, f64)>>) + Send + Sync + 'static>>>,

    sync_params: SyncParams,

    thread_pool: rayon::ThreadPool,
}

impl AutosyncProcess {
    pub fn from_manager<T: crate::stabilization::PixelType>(stab: &StabilizationManager<T>, timestamps_fract: &[f64], sync_params: SyncParams, mode: String, cancel_flag: Arc<AtomicBool>) -> Result<Self, ()> {
        let params = stab.params.read();
        let org_fps = params.fps;
        let scaled_fps = params.get_scaled_fps();
        let org_duration_ms = params.duration_ms;
        let fps_scale = params.fps_scale;
        let duration_ms = params.get_scaled_duration_ms();

        let SyncParams {
            search_size,
            mut time_per_syncpoint,
            every_nth_frame,
            ..
        } = sync_params;

        if let Some(scale) = &fps_scale {
            time_per_syncpoint *= scale;
        }
        let frame_count = ((timestamps_fract.len() as f64 * (time_per_syncpoint / 1000.0) * org_fps).ceil() as usize).min(params.frame_count) / every_nth_frame as usize;

        drop(params);

        if duration_ms < 10.0 || frame_count < 2 || time_per_syncpoint < 10.0 || search_size < 10.0 { return Err(()); }

        let ranges_us: Vec<(i64, i64)> = timestamps_fract.iter().map(|x| {
            let range = (
                ((x * org_duration_ms) - (time_per_syncpoint / 2.0)).max(0.0),
                ((x * org_duration_ms) + (time_per_syncpoint / 2.0)).min(org_duration_ms)
            );
            ((range.0 * 1000.0).round() as i64, (range.1 * 1000.0).round() as i64)
        }).collect();

        let scaled_ranges_us = ranges_us.iter().map(|(f, t)| (
            (*f as f64 / fps_scale.unwrap_or(1.0)) as i64,
            (*t as f64 / fps_scale.unwrap_or(1.0)) as i64)
        ).collect();

        let estimator = stab.pose_estimator.clone();

        estimator.every_nth_frame.store(every_nth_frame.max(1) as usize, SeqCst);

        let mut comp_params = ComputeParams::from_manager(stab, true);
        if mode == "synchronize" {
            comp_params.gyro.clear_offsets();
        }
        // Make sure we apply full correction for autosync
        comp_params.lens_correction_amount = 1.0;

        Ok(Self {
            frame_count,
            org_fps,
            scaled_fps,
            sync_params,
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
            thread_pool: rayon::ThreadPoolBuilder::new().build().unwrap()
        })
    }

    pub fn get_ranges(&self) -> Vec<(f64, f64)> {
        self.ranges_us.iter().map(|&v| (v.0 as f64 / 1000.0, v.1 as f64 / 1000.0)).collect()
    }

    pub fn feed_frame(&self, mut timestamp_us: i64, frame_no: usize, width: u32, height: u32, stride: usize, pixels: &[u8]) {
        let img = PoseEstimator::yuv_to_gray(width, height, stride as u32, pixels).map(|v| Arc::new(v));

        let method = self.sync_params.of_method;
        let estimator = self.estimator.clone();
        let total_detected_frames = self.total_detected_frames.clone();
        let total_read_frames = self.total_read_frames.clone();
        let progress_cb = self.progress_cb.clone();
        let frame_count = self.frame_count;
        let scaled_fps = self.scaled_fps;
        let org_fps = self.org_fps;
        let compute_params = self.compute_params.clone();
        let cancel_flag = self.cancel_flag.clone();
        if let Some(scale) = self.fps_scale {
            timestamp_us = (timestamp_us as f64 / scale) as i64;
        }

        if let Some(_current_range) = self.scaled_ranges_us.iter().find(|(from, to)| (*from..*to).contains(&timestamp_us)).copied() {
            self.total_read_frames.fetch_add(1, SeqCst);

            self.thread_pool.spawn(move || {
                if cancel_flag.load(Relaxed) {
                    total_detected_frames.fetch_add(1, SeqCst);
                    return;
                }
                if let Some(img) = img {
                    estimator.detect_features(frame_no, timestamp_us, method, img);
                    total_detected_frames.fetch_add(1, SeqCst);

                    if frame_no % 7 == 0 {
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

    pub fn finished_feeding_frames(&self) {
        while self.total_detected_frames.load(SeqCst) < self.total_read_frames.load(SeqCst) - 1 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        let offset_method = self.sync_params.offset_method;

        let progress_cb = self.progress_cb.clone();

        self.estimator.process_detected_frames(self.org_fps, self.scaled_fps, &self.compute_params.read());
        self.estimator.recalculate_gyro_data(self.org_fps, true);
        self.estimator.cache_optical_flow(if offset_method == 1 { 2 } else { 1 });
        self.estimator.cleanup();

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
                cb(Either::Left(self.estimator.find_offsets_visually(&self.scaled_ranges_us, &self.sync_params, &self.compute_params.read(), true, progress_cb2, self.cancel_flag.clone())));
            } else if self.mode == "guess_imu_orientation" {
                let guessed = self.estimator.guess_orientation_rssync(&self.scaled_ranges_us, &self.sync_params, &self.compute_params.read(), progress_cb2, self.cancel_flag.clone());
                if !self.cancel_flag.load(SeqCst) {
                    cb(Either::Right(guessed));
                }
            } else {
                let offsets = match offset_method {
                    0 => self.estimator.find_offsets(&self.scaled_ranges_us, &self.sync_params, &self.compute_params.read(), progress_cb2, self.cancel_flag.clone()),
                    1 => self.estimator.find_offsets_visually(&self.scaled_ranges_us, &self.sync_params, &self.compute_params.read(), false, progress_cb2, self.cancel_flag.clone()),
                    2 => self.estimator.find_offsets_rssync(&self.scaled_ranges_us, &self.sync_params, &self.compute_params.read(), progress_cb2, self.cancel_flag.clone()),
                    _ => { log::error!("Unsupported offset method: {}", offset_method); Vec::new() }
                };
                if check_negative {
                    for_negative.store(true, SeqCst);
                    // Try also negative rough offset
                    let mut sync_params = self.sync_params.clone();
                    sync_params.initial_offset = -sync_params.initial_offset;
                    let offsets2 = match offset_method {
                        0 => self.estimator.find_offsets(&self.scaled_ranges_us, &sync_params, &self.compute_params.read(), progress_cb2, self.cancel_flag.clone()),
                        1 => self.estimator.find_offsets_visually(&self.scaled_ranges_us, &sync_params, &self.compute_params.read(), false, progress_cb2, self.cancel_flag.clone()),
                        2 => self.estimator.find_offsets_rssync(&self.scaled_ranges_us, &sync_params, &self.compute_params.read(), progress_cb2, self.cancel_flag.clone()),
                        _ => { log::error!("Unsupported offset method: {}", offset_method); Vec::new() }
                    };
                    if offsets2.len() > offsets.len() {
                        cb(Either::Left(offsets2));
                    } else if offsets2.len() == offsets.len() {
                        let sum1: f64 = offsets.iter().map(|(_, _, cost)| *cost).sum();
                        let sum2: f64 = offsets2.iter().map(|(_, _, cost)| *cost).sum();
                        if sum1 < sum2 {
                            cb(Either::Left(offsets));
                        } else {
                            cb(Either::Left(offsets2));
                        }
                    }
                } else {
                    cb(Either::Left(offsets));
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
    pub fn on_finished<F>(&mut self, cb: F) where F:  Fn(Either<Vec<(f64, f64, f64)>, Option<(String, f64)>>) + Send + Sync + 'static {
        self.finished_cb = Some(Arc::new(Box::new(cb)));
    }
}
