// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use nalgebra::{Vector2, Rotation3};
use std::ops::Range;
use std::sync::atomic::Ordering::SeqCst;
use std::vec::Vec;
use parking_lot::{RwLock};
use std::sync::Arc;
use std::collections::BTreeMap;
use rayon::iter::{ ParallelIterator, IntoParallelRefIterator };

use crate::gyro_source::{Quat64, TimeQuat};
use crate::undistortion::ComputeParams;

#[cfg(feature = "use-opencv")]
use self::opencv::ItemOpenCV;
use self::akaze::ItemAkaze;

use super::gyro_source::TimeIMU;

#[cfg(feature = "use-opencv")]
mod opencv;
mod akaze;
mod find_offset;
mod find_offset_visually;
mod autosync;
pub use autosync::AutosyncProcess;

#[derive(Clone)]
enum EstimatorItem {
    #[cfg(feature = "use-opencv")]
    OpenCV(ItemOpenCV),
    Akaze(ItemAkaze)
}

pub type GrayImage = image::GrayImage;
pub struct FrameResult {
    item: EstimatorItem,
    pub rotation: Option<Rotation3<f64>>,
    pub quat: Option<Quat64>,
    pub euler: Option<(f64, f64, f64)>
}
unsafe impl Send for FrameResult {}
unsafe impl Sync for FrameResult {}

#[derive(Default)]
pub struct PoseEstimator {
    pub sync_results: Arc<RwLock<BTreeMap<usize, FrameResult>>>,
    pub lens_params: Arc<RwLock<(Vector2<f64>, Vector2<f64>)>>,
    pub estimated_gyro: Arc<RwLock<Vec<TimeIMU>>>,
    pub estimated_quats: Arc<RwLock<TimeQuat>>,
    pub lpf: std::sync::atomic::AtomicU32,
    pub every_nth_frame: std::sync::atomic::AtomicUsize
}

impl PoseEstimator {
    pub fn set_lens_params(&self, focal: Vector2<f64>, principal: Vector2<f64>) {
        *self.lens_params.write() = (focal, principal);
    }
    pub fn clear(&self) {
        self.sync_results.write().clear();
        self.estimated_gyro.write().clear();
        self.estimated_quats.write().clear();
        #[cfg(feature = "use-opencv")]
        let _ = opencv::init();
    }

    pub fn insert_empty_result(&self, frame: usize, method: u32) {
        let item = match method {
            0 => EstimatorItem::Akaze(ItemAkaze::default()),
            #[cfg(feature = "use-opencv")]
            1 => EstimatorItem::OpenCV(ItemOpenCV::default()),
            _ => panic!("Invalid method {}", method) // TODO change to Result<>
        };
        {
            let mut l = self.sync_results.write();
            l.entry(frame).or_insert(FrameResult {
                item,
                rotation: None,
                quat: None,
                euler: None
            });
        }
    }
    pub fn detect_features(&self, frame: usize, method: u32, img: image::GrayImage) {
        let item = match method {
            0 => EstimatorItem::Akaze(ItemAkaze::detect_features(frame, img)),
            #[cfg(feature = "use-opencv")]
            1 => EstimatorItem::OpenCV(ItemOpenCV::detect_features(frame, img)),
            _ => panic!("Invalid method {}", method) // TODO change to Result<>
        };
        {
            let mut l = self.sync_results.write();
            l.entry(frame).or_insert(FrameResult {
                item,
                rotation: None,
                quat: None,
                euler: None
            });
        }
    }

    pub fn processed_frames(&self, range: Range<usize>) -> Vec<usize> {
        self.sync_results.read()
            .iter()
            .filter_map(|x| if range.contains(x.0) && x.1.rotation.is_some() { Some(*x.0) } else { None })
            .collect()
    }

    pub fn process_detected_frames(&self, frame_count: usize, duration_ms: f64, fps: f64) {
        let every_nth_frame = self.every_nth_frame.load(SeqCst);
        let mut frames_to_process = Vec::new();
        {
            let l = self.sync_results.read();
            for frame in 0..frame_count {
                if l.contains_key(&frame) && l.contains_key(&(frame + every_nth_frame)) {
                    let curr_entry = l.get(&frame).unwrap();
                    if curr_entry.rotation.is_none() {
                        frames_to_process.push(frame);
                    }
                }
            }
        }

        let results = self.sync_results.clone();
        frames_to_process.par_iter().for_each(move |frame| {
            let l = results.read();
            if let Some(curr) = l.get(frame) {
                if curr.rotation.is_none() {
                    let curr = curr.item.clone();
                    if let Some(next) = l.get(&(frame + every_nth_frame)) {
                        let next = next.item.clone();
                        let (focal, principal) = *self.lens_params.read();

                        // Unlock the mutex for estimate_pose
                        drop(l);

                        let r = match (curr, next) {
                            #[cfg(feature = "use-opencv")]
                            (EstimatorItem::OpenCV(mut curr), EstimatorItem::OpenCV(mut next)) => { curr.estimate_pose(&mut next, focal, principal) }
                            (EstimatorItem::Akaze (mut curr),  EstimatorItem::Akaze (mut next))  => { curr.estimate_pose(&mut next, focal, principal) }
                            _ => None
                        };

                        if let Some(rot) = r {
                            let mut l = results.write(); 
                            if let Some(x) = l.get_mut(frame) {
                                x.rotation = Some(rot);
                                x.quat = Some(Quat64::from(rot));
                                let rotvec = rot.scaled_axis() * fps;
                                x.euler = Some((rotvec[0], rotvec[1], rotvec[2]));
                            } else {
                                log::warn!("Failed to get frame {}", frame);
                            }
                        }
                    }
                }
            }
        });
        self.recalculate_gyro_data(frame_count, duration_ms, false);
    }

    pub fn get_points_for_frame(&self, frame: &usize) -> (Vec<f32>, Vec<f32>) {
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        {
            if let Some(l) = self.sync_results.try_read() {
                if let Some(entry) = l.get(frame) {
                    let count = match &entry.item {
                        #[cfg(feature = "use-opencv")]
                        EstimatorItem::OpenCV(x) => x.get_features_count(),
                        EstimatorItem::Akaze(x) => x.get_features_count()
                    };
                    for i in 0..count {
                        let pt = match &entry.item {
                            #[cfg(feature = "use-opencv")]
                            EstimatorItem::OpenCV(x) => x.get_feature_at_index(i),
                            EstimatorItem::Akaze(x) => x.get_feature_at_index(i)
                        };
                        xs.push(pt.0);
                        ys.push(pt.1);
                    }
                }
            }
        }
        (xs, ys)
    }

    pub fn filter_of_lines(lines: Option<(Vec<(f64, f64)>, Vec<(f64, f64)>)>) -> Option<(Vec<(f64, f64)>, Vec<(f64, f64)>)> {
        let lines = lines?;

        let mut sum_angles = 0.0;
        lines.0.iter().zip(lines.1.iter()).for_each(|(p1, p2)| {
            sum_angles += (p2.1 - p1.1).atan2(p2.0 - p1.0)
        });
        let avg_angle = sum_angles / lines.0.len() as f64;

        Some(lines.0.iter().zip(lines.1.iter()).filter(|(p1, p2)| {
            let angle = (p2.1 - p1.1).atan2(p2.0 - p1.0);
            let diff = (angle - avg_angle).abs();
            diff < 30.0 * (std::f64::consts::PI / 180.0) // 30 degrees 
        }).unzip())
    }

    pub fn get_of_lines_for_frame(&self, frame: &usize, scale: f64, num_frames: usize) -> Option<(Vec<(f64, f64)>, Vec<(f64, f64)>)> {
        if let Some(l) = self.sync_results.try_read() {
            if let Some(curr) = l.get(frame) {
                if let Some(next) = l.get(&(frame + num_frames)) {
                    let mut curr = curr.item.clone();
                    let mut next = next.item.clone();
                    drop(l);

                    return match (&mut curr, &mut next) {
                        #[cfg(feature = "use-opencv")]
                        (EstimatorItem::OpenCV(curr), EstimatorItem::OpenCV(next)) => { Self::filter_of_lines(curr.get_matched_features_pair(next, scale)) }
                        (EstimatorItem::Akaze (curr),  EstimatorItem::Akaze (next))  => { Self::filter_of_lines(curr.get_matched_features_pair(next, scale)) }
                        _ => None
                    };
                }
            }
        }
        None
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
    pub fn lowpass_filter(&self, freq: f64, frame_count: usize, duration_ms: f64) {
        self.lpf.store((freq * 100.0) as u32, SeqCst);
        self.recalculate_gyro_data(frame_count, duration_ms, false);
    }

    pub fn recalculate_gyro_data(&self, frame_count: usize, duration_ms: f64, final_pass: bool) {
        let every_nth_frame = self.every_nth_frame.load(SeqCst);
        let mut is_akaze = false;
        for v in self.sync_results.read().values() {
            if let EstimatorItem::Akaze(_) = v.item {
                is_akaze = true;
                break;
            }
        }

        let timestamp_at_frame = |frame: f64| -> f64 {
            (frame as f64 / frame_count as f64) * duration_ms
        };

        let lpf = self.lpf.load(SeqCst) as f64 / 100.0;
        
        let mut vec = Vec::new();
        let mut quats = TimeQuat::new();
        if !self.sync_results.read().is_empty() {
            vec.resize(frame_count, TimeIMU::default());
            for frame in 0..frame_count {
                // Analyzed motion in reality happened during the transition from this frame to the next frame
                // So we can't use the detected motion to distort `this` frame, we need to set the timestamp in between the frames 
                // TODO: figure out if rolling shutter time can be used to make better calculation here
                // TODO: figure out why AKAZE and OpenCV have slight difference
                let next_frame = frame + every_nth_frame;
                if is_akaze {
                    let halfway = (next_frame as f64 - frame as f64) / 2.0;
                    vec[frame].timestamp_ms = timestamp_at_frame(frame as f64 + halfway);
                } else {
                    let halfway = (next_frame as f64 - frame as f64) / 2.5;
                    vec[frame].timestamp_ms = timestamp_at_frame(frame as f64 + halfway);
                }
            }
        }

        let mut update_eulers = BTreeMap::<usize, Option<(f64, f64, f64)>>::new();
        {
            let sync_results = self.sync_results.read();
            for (k, v) in sync_results.iter() {
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
                    let frame = *k;
                    if frame < vec.len() {
                        // Swap X and Y
                        vec[frame].gyro = Some([
                            e.1 * 180.0 / std::f64::consts::PI,
                            e.0 * 180.0 / std::f64::consts::PI,
                            e.2 * 180.0 / std::f64::consts::PI
                        ]);
                        let quat = v.quat.unwrap_or_else(|| Quat64::identity());
                        quats.insert((vec[frame].timestamp_ms * 1000.0) as i64, quat);
                    }
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
        }

        if lpf > 0.0 && frame_count > 0 && duration_ms > 0.0 {
            let sample_rate = frame_count as f64 / (duration_ms / 1000.0);
            if let Err(e) = crate::filtering::Lowpass::filter_gyro_forward_backward(lpf, sample_rate, &mut vec) {
                log::error!("Filter error {:?}", e);
            }
        }

        *self.estimated_gyro.write() = vec;
        *self.estimated_quats.write() = quats;
    }

    pub fn get_ranges(&self) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        let mut prev_frame = 0;
        let mut curr_range_start = 0;
        for f in self.sync_results.read().keys() {
            if f - prev_frame > 5 {
                if curr_range_start != prev_frame {
                    ranges.push((curr_range_start, prev_frame));
                }
                curr_range_start = *f;
            }
            prev_frame = *f;
        }
        if curr_range_start != prev_frame {
            ranges.push((curr_range_start, prev_frame));
        }
        ranges
    }

    pub fn find_offsets(&self, ranges: &[(i32, i32)], initial_offset: f64, search_size: f64, params: &ComputeParams) -> Vec<(f64, f64, f64)> { // Vec<(timestamp, offset, cost)>
        find_offset::find_offsets(ranges, &self.estimated_gyro.read().clone(), initial_offset, search_size, params)
    }

    pub fn find_offsets_visually(&self, ranges: &[(i32, i32)], initial_offset: f64, search_size: f64, params: &ComputeParams, for_rs: bool) -> Vec<(f64, f64, f64)> { // Vec<(timestamp, offset, cost)>
        find_offset_visually::find_offsets(ranges, self, initial_offset, search_size, params, for_rs)
    }
}
