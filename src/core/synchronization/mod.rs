use nalgebra::{Vector2, Rotation3};
use std::ops::Range;
use std::sync::atomic::Ordering::SeqCst;
use std::vec::Vec;
use parking_lot::{RwLock};
use std::sync::Arc;
use std::collections::BTreeMap;
use rayon::iter::IntoParallelRefIterator;
use rayon::iter::ParallelIterator;

use self::opencv::ItemOpenCV;
use self::akaze::ItemAkaze;

use super::gyro_source::{ GyroSource, TimeIMU };

mod opencv;
mod akaze;
mod find_offset;

#[derive(Clone)]
enum EstimatorItem {
    OpenCV(ItemOpenCV),
    Akaze(ItemAkaze)
}

pub type GrayImage = image::GrayImage;
pub struct FrameResult {
    item: EstimatorItem,
    pub rotation: Option<Rotation3<f64>>,
    pub euler: Option<(f64, f64, f64)>
}
unsafe impl Send for FrameResult {}
unsafe impl Sync for FrameResult {}

#[derive(Default)]
pub struct PoseEstimator {
    pub sync_results: Arc<RwLock<BTreeMap<usize, FrameResult>>>,
    pub lens_params: Arc<RwLock<(Vector2<f64>, Vector2<f64>)>>,
    pub estimated_gyro: Arc<RwLock<Vec<TimeIMU>>>,
    pub lpf: std::sync::atomic::AtomicU32,
    pub every_nth_frame: std::sync::atomic::AtomicUsize
}

impl PoseEstimator {
    pub fn set_lens_params(&self, focal: Vector2<f64>, principal: Vector2<f64>) {
        *self.lens_params.write() = (focal, principal);
    }
    pub fn clear(&self) {
        self.sync_results.write().clear();
        let _ = opencv::init();
    }

    pub fn insert_empty_result(&self, frame: usize, method: u32) {
        let item = match method {
            0 => EstimatorItem::Akaze(ItemAkaze::default()),
            1 => EstimatorItem::OpenCV(ItemOpenCV::default()),
            _ => panic!("Inavalid method")
        };
        {
            let mut l = self.sync_results.write();
            l.entry(frame).or_insert(FrameResult {
                item,
                rotation: None,
                euler: None
            });
        }
    }
    pub fn detect_features(&self, frame: usize, method: u32, img: image::GrayImage) {
        let item = match method {
            0 => EstimatorItem::Akaze(ItemAkaze::detect_features(frame, img)),
            1 => EstimatorItem::OpenCV(ItemOpenCV::detect_features(frame, img)),
            _ => panic!("Inavalid method")
        };
        {
            let mut l = self.sync_results.write();
            l.entry(frame).or_insert(FrameResult {
                item,
                rotation: None,
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
            let curr = l.get(frame).unwrap();
            if curr.rotation.is_none() {
                let curr = curr.item.clone();
                let next = l.get(&(frame + every_nth_frame)).unwrap().item.clone();
                let (focal, principal) = *self.lens_params.read();

                // Unlock the mutex for estimate_pose
                drop(l);

                let r = match (curr, next) {
                    (EstimatorItem::OpenCV(mut curr), EstimatorItem::OpenCV(mut next)) => { curr.estimate_pose(&mut next, focal, principal) }
                    (EstimatorItem::Akaze (mut curr),  EstimatorItem::Akaze (mut next))  => { curr.estimate_pose(&mut next, focal, principal) }
                    _ => None
                };

                if let Some(rot) = r {
                    let mut l = results.write(); 
                    let mut x = l.get_mut(frame).unwrap();
                    x.rotation = Some(rot);
                    x.euler = Some(rot.euler_angles());
                }
            }
        });
        self.recalculate_gyro_data(frame_count, duration_ms, fps, false);
    }

    pub fn get_points_for_frame(&self, frame: &usize) -> (Vec<f32>, Vec<f32>) {
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        {
            let l = self.sync_results.read();
            if let Some(entry) = l.get(frame) {
                let count = match &entry.item {
                    EstimatorItem::OpenCV(x) => x.get_features_count(),
                    EstimatorItem::Akaze(x) => x.get_features_count()
                };
                for i in 0..count {
                    let pt = match &entry.item {
                        EstimatorItem::OpenCV(x) => x.get_feature_at_index(i),
                        EstimatorItem::Akaze(x) => x.get_feature_at_index(i)
                    };
                    xs.push(pt.0);
                    ys.push(pt.1);
                }
            }
        }
        (xs, ys)
    }

    pub fn rgba_to_gray(width: u32, height: u32, slice: &[u8]) -> GrayImage {
        use image::Pixel;
        let mut img = image::GrayImage::new(width, height);
        for x in 0..width {
            for y in 0..height {
                let pix_pos = ((y * width + x) * 4) as usize;
                img.put_pixel(x, y, image::Rgba::from_slice(&slice[pix_pos..pix_pos + 4]).to_luma());
            }
        }
        img
    }
    pub fn yuv_to_gray(width: u32, height: u32, slice: &[u8]) -> GrayImage {
        image::GrayImage::from_raw(width, height, slice[0..(width*height) as usize].to_vec()).unwrap()
    }
    pub fn lowpass_filter(&self, freq: f64, frame_count: usize, duration_ms: f64, fps: f64) {
        self.lpf.store((freq * 100.0) as u32, SeqCst);
        self.recalculate_gyro_data(frame_count, duration_ms, fps, false);
    }

    pub fn recalculate_gyro_data(&self, frame_count: usize, duration_ms: f64, _fps: f64, final_pass: bool) {
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
                if final_pass && eul.is_none() {
                    if let Some(prev_existing) = sync_results.range(..*k).rev().find(|x| x.1.euler.is_some()) {
                        if let Some(next_existing) = sync_results.range(*k..).find(|x| x.1.euler.is_some()) {
                            let ratio = (*k - prev_existing.0) as f64 / (next_existing.0 - prev_existing.0) as f64;

                            fn interpolate(prev: f64, next: f64, ratio: f64) -> f64 {
                                prev + (next - prev) * ratio
                            }

                            let prev_euler = prev_existing.1.euler.as_ref().unwrap();
                            let next_euler = next_existing.1.euler.as_ref().unwrap();
                            eul = Some((
                                interpolate(prev_euler.0, next_euler.0, ratio),
                                interpolate(prev_euler.1, next_euler.1, ratio),
                                interpolate(prev_euler.2, next_euler.2, ratio),
                            ));
                            update_eulers.insert(*k, eul);
                        }
                    }
                }
                if let Some(e) = eul {
                    let frame = *k;
                    if frame < vec.len() {
                        // Swap X and Y
                        vec[frame].gyro = Some([
                            e.1 * 180.0 / std::f64::consts::PI,
                            e.0 * 180.0 / std::f64::consts::PI,
                            e.2 * 180.0 / std::f64::consts::PI
                        ]);
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
            if let Err(e) = crate::core::filtering::Lowpass::filter_gyro_forward_backward(lpf, sample_rate, &mut vec) {
                eprintln!("Filter error {:?}", e);
            }
        }

        self.get_ranges();

        *self.estimated_gyro.write() = vec;
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

    pub fn find_offsets(&self, initial_offset: f64, search_size: f64, gyro: &GyroSource) -> Vec<(f64, f64, f64)> { // Vec<(timestamp, offset, cost)>
        find_offset::find_offsets(&self.get_ranges(), &self.estimated_gyro.read().clone(), initial_offset, search_size, gyro)
    }
}
