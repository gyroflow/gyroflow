// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::Arc;
use parking_lot::RwLock;
use std::collections::HashMap;

use crate::StabilizationManager;
use crate::undistortion::ComputeParams;
use super::PoseEstimator;

pub struct AutosyncProcess {
    method: u32,
    initial_offset: f64,
    sync_search_size: f64,
    frame_count: usize,
    duration_ms: f64,
    scaled_fps: f64,
    org_fps: f64,
    fps_scale: Option<f64>,
    for_rs: bool, // for rolling shutter estimation
    ranges_ms: Vec<(f64, f64)>,
    frame_ranges: Vec<(i32, i32)>,
    estimator: Arc<PoseEstimator>,
    frame_status: Arc<RwLock<HashMap<usize, bool>>>,
    total_read_frames: Arc<AtomicUsize>,
    total_detected_frames: Arc<AtomicUsize>,
    compute_params: Arc<RwLock<ComputeParams>>,
    progress_cb: Option<Arc<Box<dyn Fn(usize, usize) + Send + Sync + 'static>>>,
    finished_cb: Option<Arc<Box<dyn Fn(Vec<(f64, f64, f64)>) + Send + Sync + 'static>>>,
}

impl AutosyncProcess {
    pub fn from_manager<T: crate::undistortion::PixelType>(stab: &StabilizationManager<T>, method: u32, timestamps_fract: &[f64], initial_offset: f64, sync_search_size: f64, mut sync_duration_ms: f64, every_nth_frame: u32, for_rs: bool) -> Result<Self, ()> {
        let params = stab.params.read();
        let frame_count = params.frame_count;
        let org_fps = params.fps;
        let scaled_fps = params.get_scaled_fps();
        let size = params.size;
        let video_size = params.video_size;
        let org_duration_ms = params.duration_ms;
        let fps_scale = params.fps_scale;
        let duration_ms = params.get_scaled_duration_ms();

        if let Some(scale) = &fps_scale {
            sync_duration_ms *= scale;
        }

        drop(params);

        if duration_ms < 10.0 || frame_count < 2 || sync_duration_ms < 10.0 || sync_search_size < 10.0 { return Err(()); }

        let ranges_ms: Vec<(f64, f64)> = timestamps_fract.iter().map(|x| {
            let range = (
                ((x * org_duration_ms) - (sync_duration_ms / 2.0)).max(0.0), 
                ((x * org_duration_ms) + (sync_duration_ms / 2.0)).min(org_duration_ms)
            );
            (range.0, range.1)
        }).collect();

        let frame_ranges: Vec<(i32, i32)> = ranges_ms.iter().map(|(from, to)| (crate::frame_at_timestamp(*from, org_fps), crate::frame_at_timestamp(*to, org_fps))).collect();
        let mut frame_status = HashMap::<usize, bool>::new();
        for x in &frame_ranges {
            for frame in x.0..x.1-1 {
                frame_status.insert(frame as usize, false);
            }
        }
        let frame_status = Arc::new(RwLock::new(frame_status));

        let estimator = stab.pose_estimator.clone();
         
        let mut img_ratio = stab.lens.read().calib_dimension.w as f64 / size.0 as f64;
        if img_ratio < 0.1 || !img_ratio.is_finite() {
            img_ratio = 1.0;
        }
        let mtrx = stab.lens.write().get_camera_matrix(size, video_size);
        estimator.set_lens_params(
            mtrx / img_ratio,
            stab.lens.read().get_distortion_coeffs()
        );
        estimator.every_nth_frame.store(every_nth_frame as usize, SeqCst);
        
        let mut comp_params = ComputeParams::from_manager(stab);
        comp_params.gyro.raw_imu = stab.gyro.read().raw_imu.clone();
        if !for_rs {
            comp_params.gyro.offsets.clear();
        }

        Ok(Self {
            frame_count,
            duration_ms,
            org_fps,
            scaled_fps,
            for_rs,
            method,
            ranges_ms,
            frame_ranges,
            frame_status,
            estimator,
            fps_scale,
            initial_offset,
            sync_search_size,
            total_read_frames: Arc::new(AtomicUsize::new(0)),
            total_detected_frames: Arc::new(AtomicUsize::new(0)),
            compute_params: Arc::new(RwLock::new(comp_params)),
            finished_cb: None,
            progress_cb: None,
        })
    }

    pub fn get_ranges(&self) -> Vec<(f64, f64)> {
        self.ranges_ms.clone()
    }
    pub fn is_frame_wanted(&self, frame: i32, mut timestamp_us: i64) -> bool {
        if let Some(_current_range) = self.frame_ranges.iter().find(|(from, to)| (*from..*to).contains(&frame)) {
            if let Some(scale) = self.fps_scale {
                timestamp_us = (timestamp_us as f64 / scale).round() as i64;
            }
            if frame % self.estimator.every_nth_frame.load(SeqCst) as i32 != 0 {
                // Don't analyze this frame
                self.frame_status.write().insert(frame as usize, true);
                self.estimator.insert_empty_result(frame as usize, timestamp_us, self.method);
                return false;
            }
            return true;
        }

        false
    }
    pub fn feed_frame(&self, mut timestamp_us: i64, frame: i32, width: u32, height: u32, stride: usize, pixels: &[u8], cancel_flag: Arc<AtomicBool>) {
        self.total_read_frames.fetch_add(1, SeqCst);

        let img = PoseEstimator::yuv_to_gray(width, height, stride as u32, pixels);
    
        let method = self.method;
        let estimator = self.estimator.clone();
        let frame_status = self.frame_status.clone();
        let total_detected_frames = self.total_detected_frames.clone();
        let total_read_frames = self.total_read_frames.clone();
        let progress_cb = self.progress_cb.clone();
        let frame_count = self.frame_count;
        let duration_ms = self.duration_ms;
        let scaled_fps = self.scaled_fps;
        let org_fps = self.org_fps;
        if let Some(scale) = self.fps_scale {
            timestamp_us = (timestamp_us as f64 / scale) as i64;
        }
        if let Some(current_range) = self.frame_ranges.iter().find(|(from, to)| (*from..*to).contains(&frame)).copied() {
            crate::THREAD_POOL.spawn(move || {
                if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    total_detected_frames.fetch_add(1, SeqCst);
                    return;
                }
                if let Some(img) = img {
                    estimator.detect_features(frame as usize, timestamp_us, method, img);
                    total_detected_frames.fetch_add(1, SeqCst);

                    if frame % 7 == 0 {
                        estimator.process_detected_frames(frame_count as usize, duration_ms, org_fps, scaled_fps);
                        estimator.recalculate_gyro_data(frame_count, duration_ms, org_fps, false);
                    }

                    let processed_frames = estimator.processed_frames(current_range.0 as usize..current_range.1 as usize);
                    for x in processed_frames { frame_status.write().insert(x, true); }

                    if total_detected_frames.load(SeqCst) <= total_read_frames.load(SeqCst) {
                        if let Some(cb) = &progress_cb {
                            let l = frame_status.read();
                            let total = l.len();
                            let ready = l.iter().filter(|e| *e.1).count();
                            drop(l);
                            cb(ready, total);
                        }
                    }
                } else {
                    log::warn!("Failed to get image {:?}", img);
                }
            });
        }
    }

    pub fn finished_feeding_frames(&self, method: u32) {
        while self.total_detected_frames.load(SeqCst) < self.total_read_frames.load(SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        self.estimator.process_detected_frames(self.frame_count as usize, self.duration_ms, self.org_fps, self.scaled_fps);
        self.estimator.recalculate_gyro_data(self.frame_count as usize, self.duration_ms, self.org_fps, true);
        self.estimator.optical_flow(1);

        if let Some(cb) = &self.finished_cb {
            if self.for_rs {
                cb(self.estimator.find_offsets_visually(&self.frame_ranges, self.initial_offset, self.sync_search_size, &self.compute_params.read(), true));
            } else {
                let offsets = match method {
                    0 => self.estimator.find_offsets(&self.frame_ranges, self.initial_offset, self.sync_search_size, &self.compute_params.read()),
                    1 => self.estimator.find_offsets_visually(&self.frame_ranges, self.initial_offset, self.sync_search_size, &self.compute_params.read(), false),
                    _ => { panic!("Unsupported offset method: {}", method); }
                };
                cb(offsets);
            }
        }
        if let Some(cb) = &self.progress_cb {
            let len = self.frame_status.read().len();
            cb(len, len);
        }
    }

    pub fn on_progress<F>(&mut self, cb: F) where F: Fn(usize, usize) + Send + Sync + 'static {
        self.progress_cb = Some(Arc::new(Box::new(cb)));
    }
    pub fn on_finished<F>(&mut self, cb: F) where F: Fn(Vec<(f64, f64, f64)>) + Send + Sync + 'static {
        self.finished_cb = Some(Arc::new(Box::new(cb)));
    }
}
