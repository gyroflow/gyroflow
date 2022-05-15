// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::Arc;
use parking_lot::RwLock;

use crate::StabilizationManager;
use crate::stabilization::ComputeParams;
use super::PoseEstimator;

pub struct AutosyncProcess {
    method: u32,
    initial_offset: f64,
    sync_search_size: f64,
    frame_count: usize,
    scaled_fps: f64,
    org_fps: f64,
    fps_scale: Option<f64>,
    for_rs: bool, // for rolling shutter estimation
    ranges_us: Vec<(i64, i64)>,
    scaled_ranges_us: Vec<(i64, i64)>,
    estimator: Arc<PoseEstimator>,
    total_read_frames: Arc<AtomicUsize>,
    total_detected_frames: Arc<AtomicUsize>,
    compute_params: Arc<RwLock<ComputeParams>>,
    progress_cb: Option<Arc<Box<dyn Fn(usize, usize) + Send + Sync + 'static>>>,
    finished_cb: Option<Arc<Box<dyn Fn(Vec<(f64, f64, f64)>) + Send + Sync + 'static>>>,

    thread_pool: rayon::ThreadPool,
}

impl AutosyncProcess {
    pub fn from_manager<T: crate::stabilization::PixelType>(stab: &StabilizationManager<T>, method: u32, timestamps_fract: &[f64], initial_offset: f64, sync_search_size: f64, mut sync_duration_ms: f64, every_nth_frame: u32, for_rs: bool) -> Result<Self, ()> {
        let params = stab.params.read();
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
        let frame_count = ((timestamps_fract.len() as f64 * (sync_duration_ms / 1000.0) * org_fps).ceil() as usize).min(params.frame_count) / every_nth_frame as usize;

        drop(params);

        if duration_ms < 10.0 || frame_count < 2 || sync_duration_ms < 10.0 || sync_search_size < 10.0 { return Err(()); }

        let ranges_us: Vec<(i64, i64)> = timestamps_fract.iter().map(|x| {
            let range = (
                ((x * org_duration_ms) - (sync_duration_ms / 2.0)).max(0.0), 
                ((x * org_duration_ms) + (sync_duration_ms / 2.0)).min(org_duration_ms)
            );
            ((range.0 * 1000.0).round() as i64, (range.1 * 1000.0).round() as i64)
        }).collect();

        let scaled_ranges_us = ranges_us.iter().map(|(f, t)| (
            (*f as f64 / fps_scale.unwrap_or(1.0)) as i64, 
            (*t as f64 / fps_scale.unwrap_or(1.0)) as i64)
        ).collect();

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
        estimator.every_nth_frame.store(every_nth_frame.max(1) as usize, SeqCst);
        
        let mut comp_params = ComputeParams::from_manager(stab);
        comp_params.gyro.raw_imu = stab.gyro.read().raw_imu.clone();
        if !for_rs {
            comp_params.gyro.offsets.clear();
        }
        // Make sure we apply full correction for autosync
        comp_params.lens_correction_amount = 1.0;

        Ok(Self {
            frame_count,
            org_fps,
            scaled_fps,
            for_rs,
            method,
            ranges_us,
            scaled_ranges_us,
            estimator,
            fps_scale,
            initial_offset,
            sync_search_size,
            total_read_frames: Arc::new(AtomicUsize::new(1)), // Start with 1 to keep the loader active until `finished_feeding_frames` overrides it with final value
            total_detected_frames: Arc::new(AtomicUsize::new(0)),
            compute_params: Arc::new(RwLock::new(comp_params)),
            finished_cb: None,
            progress_cb: None,
            thread_pool: rayon::ThreadPoolBuilder::new().build().unwrap()
        })
    }

    pub fn get_ranges(&self) -> Vec<(f64, f64)> {
        self.ranges_us.iter().map(|&v| (v.0 as f64 / 1000.0, v.1 as f64 / 1000.0)).collect()
    }
    
    pub fn feed_frame(&self, mut timestamp_us: i64, frame_no: usize, width: u32, height: u32, stride: usize, pixels: &[u8], cancel_flag: Arc<AtomicBool>) {
        let img = PoseEstimator::yuv_to_gray(width, height, stride as u32, pixels).map(|v| Arc::new(v));
    
        let method = self.method;
        let estimator = self.estimator.clone();
        let total_detected_frames = self.total_detected_frames.clone();
        let total_read_frames = self.total_read_frames.clone();
        let progress_cb = self.progress_cb.clone();
        let frame_count = self.frame_count;
        let scaled_fps = self.scaled_fps;
        let org_fps = self.org_fps;
        let compute_params = self.compute_params.clone();
        if let Some(scale) = self.fps_scale {
            timestamp_us = (timestamp_us as f64 / scale) as i64;
        }

        if let Some(_current_range) = self.scaled_ranges_us.iter().find(|(from, to)| (*from..*to).contains(&timestamp_us)).copied() {
            self.total_read_frames.fetch_add(1, SeqCst);

            self.thread_pool.spawn(move || {
                if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
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
                        cb(total_detected_frames.load(SeqCst), total_read_frames.load(SeqCst).max(frame_count));
                    }
                } else {
                    log::warn!("Failed to get image {:?}", img);
                }
            });
        }
    }

    pub fn finished_feeding_frames(&self, method: u32) {
        while self.total_detected_frames.load(SeqCst) < self.total_read_frames.load(SeqCst) - 1 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        self.estimator.process_detected_frames(self.org_fps, self.scaled_fps, &self.compute_params.read());
        self.estimator.recalculate_gyro_data(self.org_fps, true);
        self.estimator.cache_optical_flow(2);
        self.estimator.cleanup();

        if let Some(cb) = &self.finished_cb {
            if self.for_rs {
                cb(self.estimator.find_offsets_visually(&self.scaled_ranges_us, self.initial_offset, self.sync_search_size, &self.compute_params.read(), true));
            } else {
                let offsets = match method {
                    0 => self.estimator.find_offsets(&self.scaled_ranges_us, self.initial_offset, self.sync_search_size, &self.compute_params.read()),
                    1 => self.estimator.find_offsets_visually(&self.scaled_ranges_us, self.initial_offset, self.sync_search_size, &self.compute_params.read(), false),
                    _ => { panic!("Unsupported offset method: {}", method); }
                };
                cb(offsets);
            }
        }
        if let Some(cb) = &self.progress_cb {
            let len = self.total_detected_frames.load(SeqCst);
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
