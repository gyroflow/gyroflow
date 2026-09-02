// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#![allow(unused_variables, dead_code)]
use super::super::OpticalFlowPair;
use super::{ OpticalFlowTrait, OpticalFlowMethod };

use std::collections::BTreeMap;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use parking_lot::RwLock;
#[cfg(feature = "use-opencv")]
use opencv::{
    core::{ Mat, Point2f, Size, TermCriteria, Vec2f, CV_8UC1 },
    prelude::{ DenseOpticalFlowTrait, MatTraitConst },
};

#[cfg(feature = "use-opencv")]
const MAX_FEATURES: usize = 240;
#[cfg(feature = "use-opencv")]
const GRID_CELLS: usize = 64;
#[cfg(feature = "use-opencv")]
const MIN_MATCHES: usize = 10;
#[cfg(feature = "use-opencv")]
const LK_BACKTRACK_MAX_ERROR: f32 = 1.5;
#[cfg(feature = "use-opencv")]
const DIS_SEED_MAX_MAGNITUDE: f32 = 250.0;

#[cfg(feature = "use-opencv")]
struct GrayMat {
    mat: Mat,
    _owned: Option<Vec<u8>>,
}

#[cfg(feature = "use-opencv")]
fn gray_image_to_mat(img: &image::GrayImage, width: i32, height: i32) -> opencv::Result<GrayMat> {
    if width <= 0 || height <= 0 || img.width() < width as u32 || img.height() < height as u32 {
        return Err(opencv::Error::new(opencv::core::StsBadArg, "invalid gray image dimensions"));
    }

    let src_stride = img.width() as usize;
    let width = width as usize;
    let height = height as usize;
    let required = src_stride.saturating_mul(height);
    let raw = img.as_raw();

    if raw.len() < required {
        return Err(opencv::Error::new(opencv::core::StsBadArg, "gray image buffer is too small"));
    }

    if src_stride == width && img.height() as usize == height {
        let mat = unsafe {
            Mat::new_size_with_data_unsafe(
                Size::new(width as i32, height as i32),
                CV_8UC1,
                raw.as_ptr() as *mut std::ffi::c_void,
                src_stride,
            )
        }?;
        return Ok(GrayMat { mat, _owned: None });
    }

    let mut owned = vec![0; width * height];
    for y in 0..height {
        let src = y * src_stride;
        let dst = y * width;
        owned[dst..dst + width].copy_from_slice(&raw[src..src + width]);
    }

    let mat = unsafe {
        Mat::new_size_with_data_unsafe(
            Size::new(width as i32, height as i32),
            CV_8UC1,
            owned.as_mut_ptr() as *mut std::ffi::c_void,
            width,
        )
    }?;
    Ok(GrayMat { mat, _owned: Some(owned) })
}

#[cfg(feature = "use-opencv")]
fn aspect_grid(width: i32, height: i32) -> (usize, usize) {
    if width <= 0 || height <= 0 {
        return (8, 8);
    }

    let aspect = width as f32 / height as f32;
    [(2usize, 32usize), (4, 16), (8, 8), (16, 4), (32, 2)]
        .into_iter()
        .min_by(|(a_cols, a_rows), (b_cols, b_rows)| {
            let a_err = ((*a_cols as f32 / *a_rows as f32) / aspect).ln().abs();
            let b_err = ((*b_cols as f32 / *b_rows as f32) / aspect).ln().abs();
            a_err.partial_cmp(&b_err).unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or((4, 4))
}

#[cfg(feature = "use-opencv")]
fn detect_spatial_gftt_features(img: &image::GrayImage, width: i32, height: i32) -> Vec<(f32, f32)> {
    let result = || -> opencv::Result<Vec<(f32, f32)>> {
        let gray = gray_image_to_mat(img, width, height)?;
        let mut raw_pts = Mat::default();
        opencv::imgproc::good_features_to_track(
            &gray.mat,
            &mut raw_pts,
            (MAX_FEATURES * 4) as i32,
            0.01,
            8.0,
            &Mat::default(),
            3,
            false,
            0.04,
        )?;

        let (cols, rows) = aspect_grid(width, height);
        let per_cell = ((MAX_FEATURES + GRID_CELLS - 1) / GRID_CELLS).max(1);
        let mut buckets = vec![Vec::<(f32, f32)>::new(); cols * rows];

        for i in 0..raw_pts.rows() {
            let pt = *raw_pts.at::<Point2f>(i)?;
            if !in_bounds(pt.x, pt.y, width, height) {
                continue;
            }

            let cx = ((pt.x / width as f32) * cols as f32).floor().clamp(0.0, (cols - 1) as f32) as usize;
            let cy = ((pt.y / height as f32) * rows as f32).floor().clamp(0.0, (rows - 1) as f32) as usize;
            let bucket = &mut buckets[cy * cols + cx];
            if bucket.len() < per_cell {
                bucket.push((pt.x, pt.y));
            }
        }

        let mut features = Vec::with_capacity(MAX_FEATURES);
        let mut added = true;
        while added && features.len() < MAX_FEATURES {
            added = false;
            for bucket in &mut buckets {
                if !bucket.is_empty() {
                    let pt = bucket.remove(0);
                    features.push(pt);
                    added = true;
                    if features.len() == MAX_FEATURES {
                        break;
                    }
                }
            }
        }

        Ok(features)
    }();

    match result {
        Ok(features) => features,
        Err(e) => {
            log::error!("OpenCV error: {:?}", e);
            Vec::new()
        }
    }
}

#[cfg(feature = "use-opencv")]
fn in_bounds(x: f32, y: f32, width: i32, height: i32) -> bool {
    x.is_finite()
        && y.is_finite()
        && x >= 0.0
        && y >= 0.0
        && x < width as f32
        && y < height as f32
}

#[cfg(feature = "use-opencv")]
fn lk_backtrack_threshold(width: i32, height: i32) -> f32 {
    ((width.max(1) as f32).hypot(height.max(1) as f32) * 0.002)
        .clamp(LK_BACKTRACK_MAX_ERROR, 4.0)
}

#[cfg(feature = "use-opencv")]
fn grid_index(point: (f32, f32), width: i32, height: i32, cols: usize, rows: usize) -> usize {
    let x = ((point.0 / width.max(1) as f32) * cols as f32)
        .floor()
        .clamp(0.0, (cols - 1) as f32) as usize;
    let y = ((point.1 / height.max(1) as f32) * rows as f32)
        .floor()
        .clamp(0.0, (rows - 1) as f32) as usize;
    y * cols + x
}

#[cfg(feature = "use-opencv")]
fn spatially_balanced_matches(
    candidates: Vec<((f32, f32), (f32, f32), f32)>,
    width: i32,
    height: i32,
) -> (Vec<(f32, f32)>, Vec<(f32, f32)>) {
    let (cols, rows) = aspect_grid(width, height);
    let mut buckets = vec![None::<((f32, f32), (f32, f32), f32)>; cols * rows];
    for candidate in candidates {
        let idx = grid_index(candidate.0, width, height, cols, rows);
        if buckets[idx].map(|best| candidate.2 < best.2).unwrap_or(true) {
            buckets[idx] = Some(candidate);
        }
    }

    let mut sorted = buckets.into_iter().flatten().collect::<Vec<_>>();
    sorted.sort_by(|a, b| a.2.total_cmp(&b.2));
    let mut points_a = Vec::with_capacity(sorted.len());
    let mut points_b = Vec::with_capacity(sorted.len());
    for (start, end, _) in sorted.into_iter().take(MAX_FEATURES) {
        points_a.push(start);
        points_b.push(end);
    }
    (points_a, points_b)
}

#[cfg(feature = "use-opencv")]
fn dis_seeded_lk_matches(
    prev_img: &image::GrayImage,
    next_img: &image::GrayImage,
    features: &[(f32, f32)],
    width: i32,
    height: i32,
) -> opencv::Result<(Vec<(f32, f32)>, Vec<(f32, f32)>)> {
    if features.len() < MIN_MATCHES {
        return Ok((Vec::new(), Vec::new()));
    }

    let prev = gray_image_to_mat(prev_img, width, height)?;
    let next = gray_image_to_mat(next_img, width, height)?;

    let mut flow = Mat::default();
    let mut dis = opencv::video::DISOpticalFlow::create(opencv::video::DISOpticalFlow_PRESET_FAST)?;
    dis.calc(&prev.mat, &next.mat, &mut flow)?;

    let mut prev_pts = Vec::with_capacity(features.len());
    let mut seeded_next_pts = Vec::with_capacity(features.len());

    for &(x, y) in features {
        if !in_bounds(x, y, width, height) {
            continue;
        }

        let ix = x.floor().clamp(0.0, (width - 1) as f32) as i32;
        let iy = y.floor().clamp(0.0, (height - 1) as f32) as i32;
        let flow_pt = *flow.at_2d::<Vec2f>(iy, ix)?;
        let dx = flow_pt[0];
        let dy = flow_pt[1];
        if !dx.is_finite() || !dy.is_finite() || dx.hypot(dy) > DIS_SEED_MAX_MAGNITUDE {
            continue;
        }

        let nx = x + dx;
        let ny = y + dy;
        if in_bounds(nx, ny, width, height) {
            prev_pts.push(Point2f::new(x, y));
            seeded_next_pts.push(Point2f::new(nx, ny));
        }
    }

    if prev_pts.len() < MIN_MATCHES {
        return Ok((Vec::new(), Vec::new()));
    }

    let prev_pts_mat = Mat::from_slice(&prev_pts)?;
    let mut next_pts_mat = Mat::from_slice_mut(&mut seeded_next_pts)?;
    let mut fwd_status = Mat::default();
    let mut fwd_err = Mat::default();
    let criteria = TermCriteria::new(3, 30, 0.01)?;

    opencv::video::calc_optical_flow_pyr_lk(
        &prev.mat,
        &next.mat,
        &prev_pts_mat,
        &mut next_pts_mat,
        &mut fwd_status,
        &mut fwd_err,
        Size::new(21, 21),
        3,
        criteria,
        opencv::video::OPTFLOW_USE_INITIAL_FLOW,
        1e-4,
    )?;

    let next_pts = (0..next_pts_mat.total())
        .map(|i| next_pts_mat.at::<Point2f>(i as i32).copied())
        .collect::<opencv::Result<Vec<_>>>()?;
    let next_pts_mat_for_back = Mat::from_slice(&next_pts)?;
    let mut seeded_back_pts = prev_pts.clone();
    let mut back_pts_mat = Mat::from_slice_mut(&mut seeded_back_pts)?;
    let mut back_status = Mat::default();
    let mut back_err = Mat::default();

    opencv::video::calc_optical_flow_pyr_lk(
        &next.mat,
        &prev.mat,
        &next_pts_mat_for_back,
        &mut back_pts_mat,
        &mut back_status,
        &mut back_err,
        Size::new(21, 21),
        3,
        TermCriteria::new(3, 30, 0.01)?,
        opencv::video::OPTFLOW_USE_INITIAL_FLOW,
        1e-4,
    )?;

    let mut candidates = Vec::with_capacity(prev_pts.len());
    let backtrack_threshold = lk_backtrack_threshold(width, height);

    for i in 0..fwd_status.total().min(back_status.total()) {
        let idx = i as i32;
        if *fwd_status.at::<u8>(idx)? != 1 || *back_status.at::<u8>(idx)? != 1 {
            continue;
        }

        let prev_pt = *prev_pts_mat.at::<Point2f>(idx)?;
        let next_pt = next_pts[i as usize];
        let back_pt = *back_pts_mat.at::<Point2f>(idx)?;
        let fwd_lk_error = if i < fwd_err.total() { *fwd_err.at::<f32>(idx)? } else { 0.0 };
        let back_lk_error = if i < back_err.total() { *back_err.at::<f32>(idx)? } else { 0.0 };
        if !in_bounds(prev_pt.x, prev_pt.y, width, height) || !in_bounds(next_pt.x, next_pt.y, width, height) {
            continue;
        }
        if !fwd_lk_error.is_finite() || !back_lk_error.is_finite() {
            continue;
        }

        let backtrack_error = (prev_pt.x - back_pt.x).hypot(prev_pt.y - back_pt.y);
        if backtrack_error <= backtrack_threshold {
            candidates.push((
                (prev_pt.x, prev_pt.y),
                (next_pt.x, next_pt.y),
                backtrack_error + 0.01 * (fwd_lk_error + back_lk_error),
            ));
        }
    }

    Ok(spatially_balanced_matches(candidates, width, height))
}

#[derive(Clone)]
pub struct OFOpenCVDis {
    features: Vec<(f32, f32)>,
    img: Arc<image::GrayImage>,
    matched_points: Arc<RwLock<BTreeMap<i64, (Vec<(f32, f32)>, Vec<(f32, f32)>)>>>,
    timestamp_us: i64,
    size: (i32, i32),
    used: Arc<AtomicU32>,
}

impl OFOpenCVDis {
    pub fn detect_features(timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32) -> Self {
        #[cfg(feature = "use-opencv")]
        let features = detect_spatial_gftt_features(&img, width as i32, height as i32);
        #[cfg(not(feature = "use-opencv"))]
        let features = Vec::new();

        Self {
            features,
            timestamp_us,
            size: (width as i32, height as i32),
            matched_points: Default::default(),
            img,
            used: Default::default()
        }
    }
}

impl OpticalFlowTrait for OFOpenCVDis {
    fn size(&self) -> (u32, u32) {
        (self.size.0 as u32, self.size.1 as u32)
    }
    fn features(&self) -> &Vec<(f32, f32)> { &self.features }

    fn optical_flow_to(&self, _to: &OpticalFlowMethod) -> OpticalFlowPair {
        #[cfg(feature = "use-opencv")]
        if let OpticalFlowMethod::OFOpenCVDis(next) = _to {
            let (w, h) = self.size;
            if let Some(matched) = self.matched_points.read().get(&next.timestamp_us) {
                return Some(matched.clone());
            }
            if self.img.is_empty() || next.img.is_empty() || w <= 0 || h <= 0 { return None; }
            if self.size != next.size { return None; }


            let result = || -> Result<(Vec<(f32, f32)>, Vec<(f32, f32)>), opencv::Error> {
                dis_seeded_lk_matches(&self.img, &next.img, &self.features, w, h)
            }();

            match result {
                Ok(res) => {
                    if res.0.len() >= MIN_MATCHES && res.1.len() >= MIN_MATCHES {
                        let mut cache = self.matched_points.write();
                        if let Some(matched) = cache.get(&next.timestamp_us) {
                            return Some(matched.clone());
                        }
                        self.used.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        next.used.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        cache.insert(next.timestamp_us, res.clone());
                        return Some(res);
                    }
                },
                Err(e) => {
                    log::error!("OpenCV error: {:?}", e);
                }
            }
        }
        None
    }
    fn can_cleanup(&self) -> bool {
        self.used.load(std::sync::atomic::Ordering::SeqCst) >= 2
    }
    fn cleanup(&mut self) {
        self.img = Arc::new(image::GrayImage::default());
    }
}

#[cfg(all(test, feature = "use-opencv"))]
mod tests {
    use super::*;
    use image::{ GrayImage, Luma };

    fn textured_image(width: u32, height: u32) -> GrayImage {
        let mut img = GrayImage::new(width, height);
        for y in (10..height.saturating_sub(10)).step_by(14) {
            for x in (10..width.saturating_sub(10)).step_by(14) {
                let value = (((x * 7 + y * 11) % 180) + 50) as u8;
                for yy in y..(y + 5).min(height) {
                    for xx in x..(x + 5).min(width) {
                        img.put_pixel(xx, yy, Luma([value]));
                    }
                }
            }
        }
        img
    }

    fn translated_image(src: &GrayImage, dx: i32, dy: i32) -> GrayImage {
        let mut dst = GrayImage::new(src.width(), src.height());
        for y in 0..src.height() as i32 {
            for x in 0..src.width() as i32 {
                let sx = x - dx;
                let sy = y - dy;
                if sx >= 0 && sy >= 0 && sx < src.width() as i32 && sy < src.height() as i32 {
                    dst.put_pixel(x as u32, y as u32, *src.get_pixel(sx as u32, sy as u32));
                }
            }
        }
        dst
    }

    #[test]
    fn detects_spatial_gftt_features() {
        let img = Arc::new(textured_image(160, 96));
        let tracker = OFOpenCVDis::detect_features(1, img, 160, 96);
        assert!(tracker.features().len() >= MIN_MATCHES);
    }

    #[test]
    fn tracks_translation_with_dis_seeded_lk_validation() {
        let img1 = Arc::new(textured_image(160, 96));
        let img2 = Arc::new(translated_image(&img1, 4, 3));
        let tracker1 = OFOpenCVDis::detect_features(1, img1, 160, 96);
        let tracker2 = OFOpenCVDis::detect_features(2, img2, 160, 96);

        let (pts1, pts2) = tracker1
            .optical_flow_to(&OpticalFlowMethod::OFOpenCVDis(tracker2))
            .expect("expected validated DIS/LK matches");
        assert!(pts1.len() >= MIN_MATCHES);

        let mean_dx = pts1.iter().zip(&pts2).map(|(a, b)| b.0 - a.0).sum::<f32>() / pts1.len() as f32;
        let mean_dy = pts1.iter().zip(&pts2).map(|(a, b)| b.1 - a.1).sum::<f32>() / pts1.len() as f32;
        assert!((mean_dx - 4.0).abs() < 1.0, "mean dx was {mean_dx}");
        assert!((mean_dy - 3.0).abs() < 1.0, "mean dy was {mean_dy}");
    }

    #[test]
    fn fails_closed_on_blank_frames() {
        let img1 = Arc::new(GrayImage::new(80, 60));
        let img2 = Arc::new(GrayImage::new(80, 60));
        let tracker1 = OFOpenCVDis::detect_features(1, img1, 80, 60);
        let tracker2 = OFOpenCVDis::detect_features(2, img2, 80, 60);
        assert!(tracker1.optical_flow_to(&OpticalFlowMethod::OFOpenCVDis(tracker2)).is_none());
    }

    #[test]
    fn gray_mat_copies_padded_decoder_stride() {
        let mut padded = GrayImage::new(12, 5);
        for y in 0..5 {
            for x in 0..12 {
                padded.put_pixel(x, y, Luma([200]));
            }
            for x in 0..8 {
                padded.put_pixel(x, y, Luma([((x + y * 8) % 251) as u8]));
            }
        }

        let mat = gray_image_to_mat(&padded, 8, 5).unwrap();
        assert!(mat._owned.is_some());
        assert_eq!(mat.mat.cols(), 8);
        assert_eq!(mat.mat.rows(), 5);
        for y in 0..5 {
            for x in 0..8 {
                assert_eq!(*mat.mat.at_2d::<u8>(y, x).unwrap(), ((x + y * 8) % 251) as u8);
            }
        }
    }

}
