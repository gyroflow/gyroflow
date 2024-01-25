// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use nalgebra::Rotation3;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{ AtomicBool, AtomicU32, Ordering::SeqCst };
use std::vec::Vec;
use parking_lot::RwLock;
use std::cell::RefCell;
use std::collections::BTreeMap;
use rayon::iter::{ ParallelIterator, IntoParallelRefIterator };

use crate::gyro_source::{ Quat64, TimeQuat };
use crate::stabilization::ComputeParams;

mod optical_flow; pub use optical_flow::*;
mod estimate_pose; pub use estimate_pose::*;
mod find_offset { pub mod rs_sync; pub mod essential_matrix; pub mod visual_features; }

use super::gyro_source::TimeIMU;

pub mod optimsync;
mod autosync;
pub use autosync::AutosyncProcess;
use crate::util::MapClosest;

pub type GrayImage = image::GrayImage;
pub type OpticalFlowPoints = Vec<(f32, f32)>; // timestamp_us, points
pub type OpticalFlowPair = Option<(OpticalFlowPoints, OpticalFlowPoints)>;
pub type OpticalFlowPairWithTs = Option<((i64, OpticalFlowPoints), (i64, OpticalFlowPoints))>;

#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SyncParams {
    pub initial_offset: f64,
    pub initial_offset_inv: bool,
    pub search_size: f64,
    pub calc_initial_fast: bool,
    pub max_sync_points: usize,
    pub every_nth_frame: usize,
    pub time_per_syncpoint: f64,
    pub of_method: usize,
    pub offset_method: usize,
    pub pose_method: usize,
    pub custom_sync_pattern: serde_json::Value
}

#[derive(Clone)]
pub struct FrameResult {
    pub of_method: OpticalFlowMethod,
    pub frame_no: usize,
    pub timestamp_us: i64,
    pub gyro_timestamp_us: i64,
    pub frame_size: (u32, u32),
    pub rotation: Option<Rotation3<f64>>,
    pub quat: Option<Quat64>,
    pub euler: Option<(f64, f64, f64)>,

    optical_flow: RefCell<BTreeMap<usize, OpticalFlowPairWithTs>>
}
unsafe impl Send for FrameResult {}
unsafe impl Sync for FrameResult {}

#[derive(Default)]
pub struct PoseEstimator {
    pub sync_results: Arc<RwLock<BTreeMap<i64, FrameResult>>>,
    pub estimated_gyro: Arc<RwLock<BTreeMap<i64, TimeIMU>>>,
    pub estimated_quats: Arc<RwLock<TimeQuat>>,
    pub lpf: AtomicU32,
    pub every_nth_frame: AtomicU32,
    pub pose_method: AtomicU32,
    pub offset_method: AtomicU32,
}

impl PoseEstimator {
    pub fn clear(&self) {
        self.sync_results.write().clear();
        self.estimated_gyro.write().clear();
        self.estimated_quats.write().clear();
    }

    pub fn detect_features(&self, frame_no: usize, timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32, of_method: u32) {
        let frame_size = (width, height);
        let contains = self.sync_results.read().contains_key(&timestamp_us);
        if !contains {
            let result = FrameResult {
                of_method: OpticalFlowMethod::detect_features(of_method, timestamp_us, img, width, height),
                frame_no,
                frame_size,
                timestamp_us,
                gyro_timestamp_us: 0,
                rotation: None,
                quat: None,
                euler: None,
                optical_flow: Default::default()
            };
            let mut l = self.sync_results.write();
            l.entry(timestamp_us).or_insert(result);
        }
    }

    pub fn processed_frames(&self, range: Range<i64>) -> Vec<i64> {
        self.sync_results.read()
            .iter()
            .filter_map(|x| if range.contains(x.0) && x.1.rotation.is_some() { Some(*x.0) } else { None })
            .collect()
    }

    pub fn process_detected_frames(&self, fps: f64, scaled_fps: f64, params: &ComputeParams) {
        let every_nth_frame = self.every_nth_frame.load(SeqCst) as f64;
        let mut frames_to_process = Vec::new();
        {
            let l = self.sync_results.read();
            for (k, v) in l.iter() {
                if v.rotation.is_none() && v.frame_size.0 > 0 {
                    if let Some((next_k, _)) = l.range(k..).find(|(_, next)| v.frame_no + 1 == next.frame_no && next.frame_size.0 > 0) {
                        frames_to_process.push((*k, *next_k));
                    }
                }
            }
        }

        let results = self.sync_results.clone();
        let mut pose = EstimatePoseMethod::from(self.pose_method.load(SeqCst));
        pose.init(params);
        frames_to_process.par_iter().for_each(move |(ts, next_ts)| {
            let l = results.read();
            if let Some(curr) = l.get(ts) {
                if curr.rotation.is_none() {
                    //let curr = curr.item.clone();
                    if let Some(next) = l.get(next_ts) {
                        // TODO estimate pose should be quick so test if instead of cloning it is faster just to keep the lock for longer
                        let curr_of = curr.of_method.clone();
                        let next_of = next.of_method.clone();

                        // Unlock the mutex for estimate_pose
                        drop(l);

                        if let Some(rot) = pose.estimate_pose(&curr_of.optical_flow_to(&next_of), curr_of.size(), params, *ts, *next_ts) {
                            let mut l = results.write();
                            if let Some(x) = l.get_mut(ts) {
                                x.rotation = Some(rot);
                                x.quat = Some(Quat64::from(rot));
                                let rotvec = rot.scaled_axis() * (scaled_fps / every_nth_frame);
                                x.euler = Some((rotvec[0], rotvec[1], rotvec[2]));
                            } else {
                                log::warn!("Failed to get ts {}", ts);
                            }
                        }
                    }
                }
            }
        });
        self.recalculate_gyro_data(fps, false);
    }

    pub fn filter_of_lines(lines: &OpticalFlowPairWithTs, scale: f64) -> OpticalFlowPairWithTs {
        if let Some(lines) = lines {
            let mut sum_angles = 0.0;
            lines.0.1.iter().zip(lines.1.1.iter()).for_each(|(p1, p2)| {
                sum_angles += (p2.1 - p1.1).atan2(p2.0 - p1.0)
            });
            let avg_angle = sum_angles / lines.0.1.len() as f32;

            let scale = scale as f32;

            let (lines0, lines1) = lines.0.1.iter().zip(lines.1.1.iter()).filter_map(|(p1, p2)| {
                let angle = (p2.1 - p1.1).atan2(p2.0 - p1.0);
                let diff = (angle - avg_angle).abs();
                if diff < 30.0 * (std::f32::consts::PI / 180.0) {  // 30 degrees
                    Some(((p1.0 * scale, p1.1 * scale), (p2.0 * scale, p2.1 * scale)))
                } else {
                    None
                }
            }).unzip();

            Some(((lines.0.0, lines0), (lines.1.0, lines1)))
        } else {
            None
        }
    }

    pub fn cache_optical_flow(&self, num_frames: usize) {
        let l = self.sync_results.read();
        let keys: Vec<i64> = l.keys().copied().collect();
        for (i, k) in keys.iter().enumerate() {
            if let Some(from_fr) = l.get(k) {
                if from_fr.optical_flow.try_borrow().map(|of| !of.is_empty()).unwrap_or_default() {
                    // We already have OF for this frame
                    continue;
                }
                for d in 1..=num_frames {
                    if let Some(to_key) = keys.get(i + d) {
                        if let Some(to_item) = l.get(to_key) {
                            if from_fr.frame_no + d == to_item.frame_no {
                                let of = from_fr.of_method.optical_flow_to(&to_item.of_method);
                                if let Ok(mut from_of) = from_fr.optical_flow.try_borrow_mut() {
                                    from_of.insert(d,
                                        of.map(|of| ((from_fr.timestamp_us, of.0), (to_item.timestamp_us, of.1)))
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    pub fn cleanup(&self) {
        let mut l = self.sync_results.write();
        for (_, i) in l.iter_mut(){
            i.of_method.cleanup();
        }
    }

    pub fn get_of_lines_for_timestamp(&self, timestamp_us: &i64, next_no: usize, scale: f64, num_frames: usize, filter: bool) -> (OpticalFlowPairWithTs, Option<(u32, u32)>) {
        if let Some(l) = self.sync_results.try_read() {
            if let Some(first_ts) = l.get_closest(timestamp_us, 2000).map(|v| v.timestamp_us) {
                let mut iter = l.range(first_ts..);
                for _ in 0..next_no { iter.next(); }
                if let Some((_, curr)) = iter.next() {
                    if let Ok(of) = curr.optical_flow.try_borrow() {
                        if let Some(opt_pts) = of.get(&num_frames) {
                            return (if filter {
                                Self::filter_of_lines(opt_pts, scale)
                            } else {
                                opt_pts.clone()
                            }, Some(curr.frame_size));
                        }
                    }
                }
            }
        }
        (None, None)
    }

    pub fn rgba_to_gray(width: u32, height: u32, stride: u32, slice: &[u8]) -> GrayImage {
        use image::Pixel;
        let mut img = image::GrayImage::new(width, height);
        for x in 0..width {
            for y in 0..height {
                let pix_pos = ((y * stride + x) * 4) as usize;
                img.put_pixel(x, y, image::Rgba::from_slice(&slice[pix_pos..pix_pos + 4]).to_luma());
            }
        }
        img
    }
    pub fn yuv_to_gray(_width: u32, height: u32, stride: u32, slice: &[u8]) -> Option<GrayImage> {
        // TODO: maybe a better way than using stride as width?
        image::GrayImage::from_raw(stride as u32, height, slice[0..(stride*height) as usize].to_vec())
    }
    pub fn lowpass_filter(&self, freq: f64, fps: f64) {
        self.lpf.store((freq * 100.0) as u32, SeqCst);
        self.recalculate_gyro_data(fps, false);
    }

    pub fn recalculate_gyro_data(&self, fps: f64, final_pass: bool) {
        let lpf = self.lpf.load(SeqCst) as f64 / 100.0;

        let mut gyro = BTreeMap::new();
        let mut quats = TimeQuat::new();
        let mut update_eulers = BTreeMap::<i64, Option<(f64, f64, f64)>>::new();
        let mut update_timestamps = BTreeMap::<i64, i64>::new();
        {
            let sync_results = self.sync_results.read();

            let mut iter = sync_results.iter().peekable();
            while let Some((k, v)) = iter.next() {
                let mut eul = v.euler;

                // ----------- Interpolation -----------
                if final_pass && eul.is_none() {
                    if let Some(prev_existing) = sync_results.range(..*k).rev().find(|x| x.1.euler.is_some()) {
                        if let Some(next_existing) = sync_results.range(*k..).find(|x| x.1.euler.is_some()) {
                            let ratio = (*k - prev_existing.0) as f64 / (next_existing.0 - prev_existing.0) as f64;

                            fn interpolate(prev: f64, next: f64, ratio: f64) -> f64 {
                                prev + (next - prev) * ratio
                            }

                            if let Some(prev_euler) = prev_existing.1.euler.as_ref() {
                                if let Some(next_euler) = next_existing.1.euler.as_ref() {
                                    eul = Some((
                                        interpolate(prev_euler.0, next_euler.0, ratio),
                                        interpolate(prev_euler.1, next_euler.1, ratio),
                                        interpolate(prev_euler.2, next_euler.2, ratio),
                                    ));
                                    update_eulers.insert(*k, eul);
                                }
                            }
                        }
                    }
                }
                // ----------- Interpolation -----------

                if let Some(e) = eul {
                    // Analyzed motion in reality happened during the transition from this frame to the next frame
                    // So we can't use the detected motion to distort `this` frame, we need to set the timestamp in between the frames
                    // TODO: figure out if rolling shutter time can be used to make better calculation here
                    let mut ts = *k as f64 / 1000.0;
                    if let Some(next_ts) = iter.peek().map(|(&k, _)| k as f64 / 1000.0) {
                        ts += (next_ts - ts) / 2.0;
                    }

                    let ts_us = (ts * 1000.0).round() as i64;
                    update_timestamps.insert(*k, ts_us);
                    gyro.insert(ts_us, TimeIMU {
                        timestamp_ms: ts,
                        gyro: Some([
                            // Swap X and Y
                            e.1 * 180.0 / std::f64::consts::PI,
                            e.0 * 180.0 / std::f64::consts::PI,
                            e.2 * 180.0 / std::f64::consts::PI
                        ]),
                        accl: None,
                        magn: None
                    });
                    let quat = v.quat.unwrap_or_else(|| Quat64::identity());
                    quats.insert(ts_us, quat);
                }
            }
        }
        {
            let mut sync_results = self.sync_results.write();
            for (k, e) in update_eulers {
                if let Some(entry) = sync_results.get_mut(&k) {
                    entry.euler = e;
                }
            }
            for (k, gyro_ts) in update_timestamps {
                if let Some(entry) = sync_results.get_mut(&k) {
                    entry.gyro_timestamp_us = gyro_ts;
                }
            }
        }

        if lpf > 0.0 && fps > 0.0 {
            let mut vals = gyro.values().cloned().collect::<Vec<_>>();
            if let Err(e) = crate::filtering::Lowpass::filter_gyro_forward_backward(lpf, fps, &mut vals) {
                log::error!("Filter error {:?}", e);
            }
            for ((_k, v), vec) in gyro.iter_mut().zip(vals.into_iter()) {
                *v = vec;
            }
        }

        *self.estimated_gyro.write() = gyro;
        *self.estimated_quats.write() = quats;
    }

    pub fn get_ranges(&self) -> Vec<(i64, i64)> {
        let mut ranges = Vec::new();
        let mut prev_ts = 0;
        let mut curr_range_start = 0;
        for f in self.sync_results.read().keys() {
            if f - prev_ts > 100000 { // 100ms
                if curr_range_start != prev_ts {
                    ranges.push((curr_range_start, prev_ts));
                }
                curr_range_start = *f;
            }
            prev_ts = *f;
        }
        if curr_range_start != prev_ts {
            ranges.push((curr_range_start, prev_ts));
        }
        ranges
    }

    pub fn find_offsets<F: Fn(f64) + Sync>(&self, ranges: &[(i64, i64)], sync_params: &SyncParams, params: &ComputeParams, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> Vec<(f64, f64, f64)> { // Vec<(timestamp, offset, cost)>
        match self.offset_method.load(SeqCst) {
            0 => find_offset::essential_matrix::find_offsets(&self, ranges, sync_params, params, progress_cb, cancel_flag),
            1 => find_offset::visual_features::find_offsets(&self, ranges,  sync_params, params, false, progress_cb, cancel_flag),
            2 => find_offset::rs_sync::find_offsets(&self, ranges, sync_params, params, progress_cb, cancel_flag),
            v => { log::error!("Unknown offset method: {v}"); Vec::new() }
        }
    }
}
