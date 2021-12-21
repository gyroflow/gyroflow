use nalgebra::{Vector2, Rotation3};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::SeqCst;
use std::vec::Vec;
use parking_lot::{RwLock};
use std::sync::Arc;
use std::collections::BTreeMap;
use rayon::iter::{ ParallelIterator, IntoParallelRefIterator };

use crate::undistortion::ComputeParams;

#[cfg(feature = "use-opencv")]
use self::opencv::ItemOpenCV;
use self::akaze::ItemAkaze;

use super::StabilizationManager;
use super::gyro_source::{ GyroSource, TimeIMU };

#[cfg(feature = "use-opencv")]
mod opencv;
mod akaze;
mod find_offset;
mod find_offset_visually;

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
        self.estimated_gyro.write().clear();
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
        self.recalculate_gyro_data(frame_count, duration_ms, fps, false);
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
            if let Some(curr) = l.get(&frame) {
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
            if let Err(e) = crate::filtering::Lowpass::filter_gyro_forward_backward(lpf, sample_rate, &mut vec) {
                log::error!("Filter error {:?}", e);
            }
        }

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

    pub fn find_offsets(&self, ranges: &[(usize, usize)], initial_offset: f64, search_size: f64, gyro: &GyroSource) -> Vec<(f64, f64, f64)> { // Vec<(timestamp, offset, cost)>
        find_offset::find_offsets(&ranges, &self.estimated_gyro.read().clone(), initial_offset, search_size, gyro)
    }

    pub fn find_offsets_visually(&self, ranges: &[(usize, usize)], initial_offset: f64, search_size: f64, params: &ComputeParams, for_rs: bool) -> Vec<(f64, f64, f64)> { // Vec<(timestamp, offset, cost)>
        find_offset_visually::find_offsets(&ranges, &self, initial_offset, search_size, params, for_rs)
    }
}

pub struct AutosyncProcess {
    method: u32,
    initial_offset: f64,
    sync_search_size: f64,
    frame_count: usize,
    duration_ms: f64,
    fps: f64,
    for_rs: bool, // for rolling shutter estimation
    ranges_ms: Vec<(f64, f64)>,
    frame_ranges: Vec<(usize, usize)>,
    estimator: Arc<PoseEstimator>,
    frame_status: Arc<RwLock<HashMap<usize, bool>>>,
    total_read_frames: Arc<AtomicUsize>,
    total_detected_frames: Arc<AtomicUsize>,
    compute_params: Arc<RwLock<ComputeParams>>,
    progress_cb: Option<Arc<Box<dyn Fn(usize, usize) + Send + Sync + 'static>>>,
    finished_cb: Option<Arc<Box<dyn Fn(Vec<(f64, f64, f64)>) + Send + Sync + 'static>>>,
}

impl AutosyncProcess {
    pub fn from_manager<T: crate::undistortion::PixelType>(stab: &StabilizationManager<T>, method: u32, timestamps_fract: &[f64], initial_offset: f64, sync_search_size: f64, sync_duration_ms: f64, every_nth_frame: u32, for_rs: bool) -> Result<Self, ()> {
        let params = stab.params.read(); 
        let frame_count = params.frame_count;
        let fps = params.fps;
        let size = params.size;
        let duration_ms = params.duration_ms;
        drop(params);

        if duration_ms < 10.0 || frame_count < 2 || sync_duration_ms < 10.0 || sync_search_size < 10.0 { return Err(()); }

        let ranges_ms: Vec<(f64, f64)> = timestamps_fract.iter().map(|x| {
            let range = (
                ((x * duration_ms) - (sync_duration_ms / 2.0)).max(0.0), 
                ((x * duration_ms) + (sync_duration_ms / 2.0)).min(duration_ms)
            );
            (range.0, range.1)
        }).collect();

        let frame_ranges: Vec<(usize, usize)> = ranges_ms.iter().map(|(from, to)| (stab.frame_at_timestamp(*from, fps), stab.frame_at_timestamp(*to, fps))).collect();
        log::debug!("frame_ranges: {:?}", &frame_ranges);
        let mut frame_status = HashMap::<usize, bool>::new();
        for x in &frame_ranges {
            for frame in x.0..x.1-1 {
                frame_status.insert(frame, false);
            }
        }
        let frame_status = Arc::new(RwLock::new(frame_status));

        let estimator = stab.pose_estimator.clone();
         
        let mut img_ratio = stab.lens.read().calib_dimension.0 / size.0 as f64;
        if img_ratio < 0.1 || !img_ratio.is_finite() {
            img_ratio = 1.0;
        }
        let mtrx = stab.camera_matrix_or_default();
        estimator.set_lens_params(
            Vector2::new(mtrx[0] / img_ratio, mtrx[4] / img_ratio),
            Vector2::new(mtrx[2] / img_ratio, mtrx[5] / img_ratio)
        );
        estimator.every_nth_frame.store(every_nth_frame as usize, SeqCst);
        
        let mut comp_params = ComputeParams::from_manager(stab);
        if !for_rs {
            comp_params.gyro.offsets.clear();
        }

        Ok(Self {
            frame_count,
            duration_ms,
            fps,
            for_rs,
            method,
            ranges_ms,
            frame_ranges,
            frame_status,
            estimator,
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
    pub fn is_frame_wanted(&self, frame: i32) -> bool {
        if let Some(_current_range) = self.frame_ranges.iter().find(|(from, to)| (*from..*to).contains(&(frame as usize))) {
            if frame % self.estimator.every_nth_frame.load(SeqCst) as i32 != 0 {
                // Don't analyze this frame
                self.frame_status.write().insert(frame as usize, true);
                self.estimator.insert_empty_result(frame as usize, self.method);
                return false;
            }
            return true;    
        }
        return false;
    }
    pub fn feed_frame(&self, frame: i32, width: u32, height: u32, stride: usize, pixels: &[u8], cancel_flag: Arc<AtomicBool>) {
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
        let fps = self.fps;
        if let Some(current_range) = self.frame_ranges.iter().find(|(from, to)| (*from..*to).contains(&(frame as usize))).copied() {
            crate::THREAD_POOL.spawn(move || {
                if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    total_detected_frames.fetch_add(1, SeqCst);
                    return;
                }
                if let Some(img) = img {
                    estimator.detect_features(frame as usize, method, img);
                    total_detected_frames.fetch_add(1, SeqCst);

                    if frame % 7 == 0 {
                        estimator.process_detected_frames(frame_count as usize, duration_ms, fps);
                        estimator.recalculate_gyro_data(frame_count, duration_ms, fps, false);
                    }

                    let processed_frames = estimator.processed_frames(current_range.0..current_range.1);
                    for x in processed_frames { frame_status.write().insert(x, true); }

                    if total_detected_frames.load(SeqCst) < total_read_frames.load(SeqCst) {
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
        self.estimator.process_detected_frames(self.frame_count as usize, self.duration_ms, self.fps);
        self.estimator.recalculate_gyro_data(self.frame_count as usize, self.duration_ms, self.fps, true);

        if let Some(cb) = &self.finished_cb {
            if self.for_rs {
                cb(self.estimator.find_offsets_visually(&self.frame_ranges, self.initial_offset, self.sync_search_size, &self.compute_params.read(), true));
            } else {
                let offsets = match method {
                    0 => self.estimator.find_offsets(&self.frame_ranges, self.initial_offset, self.sync_search_size, &self.compute_params.read().gyro),
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
