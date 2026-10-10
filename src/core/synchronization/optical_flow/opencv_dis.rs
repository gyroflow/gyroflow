// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#![allow(unused_variables, dead_code)]
use super::super::{ OpticalFlowPair, OpticalFlowPoints };
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
#[derive(Clone, Copy)]
struct TrackCandidate {
    start: Point2f,
    end: Point2f,
    score: f32,
}

type TrackPair = (OpticalFlowPoints, OpticalFlowPoints);

#[derive(Clone)]
pub struct OFOpenCVDis {
    features: Vec<(f32, f32)>,
    img: Arc<image::GrayImage>,
    matched_points: Arc<RwLock<BTreeMap<i64, TrackPair>>>,
    timestamp_us: i64,
    size: (i32, i32),
    used: Arc<AtomicU32>,
}

impl OFOpenCVDis {
    pub fn detect_features(timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32) -> Self {
        #[cfg(feature = "use-opencv")]
        let features = detect_spatial_features(&img, width, height).unwrap_or_else(|e| {
            log::error!("OpenCV feature detection error: {e:?}");
            Vec::new()
        });
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

#[cfg(feature = "use-opencv")]
fn grid_dimensions(width: i32, height: i32) -> (usize, usize) {
    const LONG_SIDE_CELLS: usize = 16;
    const MIN_SHORT_SIDE_CELLS: usize = 8;

    if width >= height {
        let rows = ((LONG_SIDE_CELLS as f32 * height as f32 / width.max(1) as f32).round() as usize)
            .clamp(MIN_SHORT_SIDE_CELLS, LONG_SIDE_CELLS);
        (LONG_SIDE_CELLS, rows)
    } else {
        let columns = ((LONG_SIDE_CELLS as f32 * width as f32 / height.max(1) as f32).round() as usize)
            .clamp(MIN_SHORT_SIDE_CELLS, LONG_SIDE_CELLS);
        (columns, LONG_SIDE_CELLS)
    }
}

#[cfg(feature = "use-opencv")]
fn grid_cell(point: Point2f, width: i32, height: i32, columns: usize, rows: usize) -> usize {
    let column = ((point.x / width.max(1) as f32) * columns as f32).floor() as usize;
    let row = ((point.y / height.max(1) as f32) * rows as f32).floor() as usize;
    row.min(rows - 1) * columns + column.min(columns - 1)
}

#[cfg(feature = "use-opencv")]
fn detect_spatial_features(img: &image::GrayImage, width: u32, height: u32) -> Result<Vec<(f32, f32)>, opencv::Error> {
    let (width, height) = (width as i32, height as i32);
    if img.is_empty() || width <= 0 || height <= 0 || img.width() < width as u32 || img.height() < height as u32 {
        return Ok(Vec::new());
    }

    let input = unsafe {
        Mat::new_size_with_data_unsafe(
            Size::new(width, height),
            CV_8UC1,
            img.as_raw().as_ptr() as *mut std::ffi::c_void,
            img.width() as usize,
        )
    }?;
    let mut detected = Mat::default();
    opencv::imgproc::good_features_to_track(
        &input,
        &mut detected,
        800,
        0.005,
        5.0,
        &Mat::default(),
        5,
        false,
        0.04,
    )?;

    let (columns, rows) = grid_dimensions(width, height);
    let mut cells = vec![0usize; columns * rows];
    let mut features = Vec::with_capacity((columns * rows * 2).min(detected.rows() as usize));
    for index in 0..detected.rows() {
        let point = *detected.at::<Point2f>(index)?;
        if !point.x.is_finite() || !point.y.is_finite() || point.x < 0.0 || point.y < 0.0 || point.x >= width as f32 || point.y >= height as f32 {
            continue;
        }
        let cell = grid_cell(point, width, height, columns, rows);
        if cells[cell] < 2 {
            cells[cell] += 1;
            features.push((point.x, point.y));
        }
    }
    Ok(features)
}

#[cfg(feature = "use-opencv")]
fn select_spatial_tracks(mut candidates: Vec<TrackCandidate>, width: i32, height: i32) -> Vec<TrackCandidate> {
    let (columns, rows) = grid_dimensions(width, height);
    let mut occupied = vec![false; columns * rows];
    candidates.sort_by(|a, b| a.score.total_cmp(&b.score));
    candidates.into_iter().filter(|candidate| {
        let cell = grid_cell(candidate.start, width, height, columns, rows);
        if occupied[cell] {
            false
        } else {
            occupied[cell] = true;
            true
        }
    }).collect()
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
            if self.img.is_empty() || next.img.is_empty() || w <= 0 || h <= 0 || self.size != next.size
            || self.img.width() < w as u32 || self.img.height() < h as u32
            || next.img.width() < w as u32 || next.img.height() < h as u32 { return None; }


            let result = || -> Result<TrackPair, opencv::Error> {
                let a1_view = unsafe { Mat::new_size_with_data_unsafe(Size::new(w, h), CV_8UC1, self.img.as_raw().as_ptr() as *mut std::ffi::c_void, self.img.width() as usize) }?;
                let a2_view = unsafe { Mat::new_size_with_data_unsafe(Size::new(w, h), CV_8UC1, next.img.as_raw().as_ptr() as *mut std::ffi::c_void, next.img.width() as usize) }?;
                // DIS requires continuous input. Decoder padding takes this copy-only fallback.
                let a1_img = if a1_view.is_continuous() { a1_view } else { a1_view.try_clone()? };
                let a2_img = if a2_view.is_continuous() { a2_view } else { a2_view.try_clone()? };

                let mut of = Mat::default();
                let mut optflow = opencv::video::DISOpticalFlow::create(opencv::video::DISOpticalFlow_PRESET_FAST)?;
                optflow.calc(&a1_img, &a2_img, &mut of)?;

                let mut starts = Vec::with_capacity(self.features.len());
                let mut initial_ends = Vec::with_capacity(self.features.len());
                for &(x, y) in &self.features {
                    let sample_x = x.round().clamp(0.0, (w - 1) as f32) as i32;
                    let sample_y = y.round().clamp(0.0, (h - 1) as f32) as i32;
                    let flow = of.at_2d::<Vec2f>(sample_y, sample_x)?;
                    let end = Point2f::new(x + flow[0], y + flow[1]);
                    if flow[0].is_finite() && flow[1].is_finite()
                    && end.x >= 0.0 && end.x < w as f32 && end.y >= 0.0 && end.y < h as f32 {
                        starts.push(Point2f::new(x, y));
                        initial_ends.push(end);
                    }
                }
                if starts.len() < 10 {
                    return Ok((Vec::new(), Vec::new()));
                }

                // DIS handles large displacement; bidirectional LK refines it and exposes occlusions.
                let start_points = Mat::from_slice(&starts)?;
                let mut end_points = Mat::from_slice(&initial_ends)?.try_clone()?;
                let mut forward_status = Mat::default();
                let mut forward_error = Mat::default();
                opencv::video::calc_optical_flow_pyr_lk(
                    &a1_img,
                    &a2_img,
                    &start_points,
                    &mut end_points,
                    &mut forward_status,
                    &mut forward_error,
                    Size::new(21, 21),
                    3,
                    TermCriteria::new(3, 30, 0.01)?,
                    opencv::video::OPTFLOW_USE_INITIAL_FLOW,
                    1e-4,
                )?;

                let mut returned_points = Mat::from_slice(&starts)?.try_clone()?;
                let mut backward_status = Mat::default();
                let mut backward_error = Mat::default();
                opencv::video::calc_optical_flow_pyr_lk(
                    &a2_img,
                    &a1_img,
                    &end_points,
                    &mut returned_points,
                    &mut backward_status,
                    &mut backward_error,
                    Size::new(21, 21),
                    3,
                    TermCriteria::new(3, 30, 0.01)?,
                    opencv::video::OPTFLOW_USE_INITIAL_FLOW,
                    1e-4,
                )?;

                let diagonal = (w as f32).hypot(h as f32);
                let max_round_trip_error = (diagonal / 1000.0).clamp(0.75, 2.5);
                let mut candidates = Vec::with_capacity(starts.len());
                for index in 0..forward_status.rows().min(backward_status.rows()) {
                    if *forward_status.at::<u8>(index)? != 1 || *backward_status.at::<u8>(index)? != 1 {
                        continue;
                    }
                    let start = *start_points.at::<Point2f>(index)?;
                    let end = *end_points.at::<Point2f>(index)?;
                    let returned = *returned_points.at::<Point2f>(index)?;
                    let forward_error = *forward_error.at::<f32>(index)?;
                    let backward_error = *backward_error.at::<f32>(index)?;
                    let round_trip_error = (returned.x - start.x).hypot(returned.y - start.y);

                    if !start.x.is_finite() || !start.y.is_finite()
                    || !end.x.is_finite() || !end.y.is_finite()
                    || !returned.x.is_finite() || !returned.y.is_finite()
                    || !forward_error.is_finite() || !backward_error.is_finite()
                    || end.x < 0.0 || end.x >= w as f32 || end.y < 0.0 || end.y >= h as f32
                    || round_trip_error > max_round_trip_error {
                        continue;
                    }

                    candidates.push(TrackCandidate {
                        start,
                        end,
                        score: round_trip_error + (forward_error + backward_error) / 510.0,
                    });
                }

                let selected = select_spatial_tracks(candidates, w, h);
                Ok((
                    selected.iter().map(|track| (track.start.x, track.start.y)).collect(),
                    selected.iter().map(|track| (track.end.x, track.end.y)).collect(),
                ))
            }();

            match result {
                Ok(res) => {
                    // Only store and return if we have enough valid points.
                    if res.0.len() >= 10 && res.1.len() >= 10 {
                        self.used.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        next.used.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        self.matched_points.write().insert(next.timestamp_us, res.clone());
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
        self.used.load(std::sync::atomic::Ordering::SeqCst) == 2
    }
    fn cleanup(&mut self) {
        self.img = Arc::new(image::GrayImage::default());
    }
}

#[cfg(all(test, feature = "use-opencv"))]
mod tests {
    use super::*;

    fn textured_image(width: u32, height: u32) -> image::GrayImage {
        image::GrayImage::from_fn(width, height, |x, y| {
            let checker = (((x / 11) + (y / 7)) & 1) as u8 * 71;
            let hash = (x.wrapping_mul(37) ^ y.wrapping_mul(101) ^ x.wrapping_mul(y).wrapping_mul(13)) as u8;
            image::Luma([hash.wrapping_add(checker)])
        })
    }

    fn transformed_image(
        source: &image::GrayImage,
        angle_radians: f32,
        translation: (f32, f32),
        occlusion: Option<(u32, u32, u32, u32)>,
    ) -> image::GrayImage {
        let (width, height) = source.dimensions();
        let center = (width as f32 / 2.0, height as f32 / 2.0);
        let (sin, cos) = angle_radians.sin_cos();

        image::GrayImage::from_fn(width, height, |x, y| {
            if occlusion.is_some_and(|(left, top, right, bottom)| {
                x >= left && x < right && y >= top && y < bottom
            }) {
                return image::Luma([127]);
            }

            let dx = x as f32 - center.0 - translation.0;
            let dy = y as f32 - center.1 - translation.1;
            let source_x = cos * dx + sin * dy + center.0;
            let source_y = -sin * dx + cos * dy + center.1;

            if source_x >= 0.0 && source_x < width as f32 && source_y >= 0.0 && source_y < height as f32 {
                *source.get_pixel(source_x.round().min(width as f32 - 1.0) as u32, source_y.round().min(height as f32 - 1.0) as u32)
            } else {
                image::Luma([0])
            }
        })
    }

    fn expected_point(
        point: (f32, f32),
        size: (u32, u32),
        angle_radians: f32,
        translation: (f32, f32),
    ) -> (f32, f32) {
        let center = (size.0 as f32 / 2.0, size.1 as f32 / 2.0);
        let (sin, cos) = angle_radians.sin_cos();
        let dx = point.0 - center.0;
        let dy = point.1 - center.1;
        (
            cos * dx - sin * dy + center.0 + translation.0,
            sin * dx + cos * dy + center.1 + translation.1,
        )
    }

    fn track_with_size(first: image::GrayImage, second: image::GrayImage, size: (u32, u32)) -> TrackPair {
        let first = OFOpenCVDis::detect_features(0, Arc::new(first), size.0, size.1);
        let second = OpticalFlowMethod::OFOpenCVDis(OFOpenCVDis::detect_features(
            1_000_000,
            Arc::new(second),
            size.0,
            size.1,
        ));
        first.optical_flow_to(&second).expect("optical flow returned no tracks")
    }

    fn track(first: image::GrayImage, second: image::GrayImage) -> TrackPair {
        let size = first.dimensions();
        track_with_size(first, second, size)
    }

    fn padded_image(source: &image::GrayImage, padded_width: u32) -> image::GrayImage {
        image::GrayImage::from_fn(padded_width, source.height(), |x, y| {
            if x < source.width() {
                *source.get_pixel(x, y)
            } else {
                image::Luma([(x.wrapping_mul(17) ^ y.wrapping_mul(43)) as u8])
            }
        })
    }

    fn percentile(mut values: Vec<f32>, percentile: f32) -> f32 {
        values.sort_by(f32::total_cmp);
        let index = ((values.len() - 1) as f32 * percentile).round() as usize;
        values[index]
    }

    fn assert_spatially_distributed(points: &[(f32, f32)], size: (u32, u32)) {
        let (columns, rows) = grid_dimensions(size.0 as i32, size.1 as i32);
        let mut occupied = vec![false; columns * rows];
        for &(x, y) in points {
            let cell = grid_cell(Point2f::new(x, y), size.0 as i32, size.1 as i32, columns, rows);
            assert!(!occupied[cell], "multiple tracks occupied grid cell {cell}");
            occupied[cell] = true;
        }
    }

    #[test]
    fn recovers_rotation_and_translation() {
        let size = (320, 192);
        let angle = 3.5_f32.to_radians();
        let translation = (24.0, -14.0);
        let first = textured_image(size.0, size.1);
        let second = transformed_image(&first, angle, translation, None);
        let (points_a, points_b) = track(first, second);

        let errors = points_a.iter().zip(&points_b).map(|(&point_a, &point_b)| {
            let expected = expected_point(point_a, size, angle, translation);
            (point_b.0 - expected.0).hypot(point_b.1 - expected.1)
        }).collect::<Vec<_>>();

        println!("rotation + translation: {} tracks, p50 {:.3}px, p90 {:.3}px", errors.len(), percentile(errors.clone(), 0.5), percentile(errors.clone(), 0.9));
        assert_spatially_distributed(&points_a, size);
        assert!(errors.len() >= 20, "too few tracks: {}", errors.len());
        assert!(percentile(errors.clone(), 0.5) < 1.0, "median endpoint error was too high");
        assert!(percentile(errors, 0.9) < 2.0, "p90 endpoint error was too high");
    }

    #[test]
    fn rejects_occluded_and_out_of_frame_tracks() {
        let size = (320, 192);
        let translation = (11.0, -7.0);
        let occlusion = (96, 38, 224, 154);
        let first = textured_image(size.0, size.1);
        let second = transformed_image(&first, 0.0, translation, Some(occlusion));
        let (points_a, points_b) = track(first, second);

        let errors = points_a.iter().zip(&points_b).map(|(&point_a, &point_b)| {
            let expected = expected_point(point_a, size, 0.0, translation);
            (point_b.0 - expected.0).hypot(point_b.1 - expected.1)
        }).collect::<Vec<_>>();

        println!("translation + occlusion: {} tracks, p50 {:.3}px, p90 {:.3}px", errors.len(), percentile(errors.clone(), 0.5), percentile(errors.clone(), 0.9));
        assert!(points_b.iter().all(|&(x, y)| {
            x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0 && x < size.0 as f32 && y < size.1 as f32
        }), "tracker returned an invalid or out-of-frame endpoint");
        assert!(errors.len() >= 20, "too few tracks: {}", errors.len());
        assert!(percentile(errors.clone(), 0.5) < 1.0, "median endpoint error was too high");
        assert!(percentile(errors, 0.9) < 2.0, "p90 endpoint error was too high");
    }

    #[test]
    fn tracks_padded_rows_at_the_logical_width() {
        let size = (320, 192);
        let padded_width = 336;
        let angle = 1.5_f32.to_radians();
        let translation = (9.0, -5.0);
        let first = textured_image(size.0, size.1);
        let second = transformed_image(&first, angle, translation, None);
        let (points_a, points_b) = track_with_size(
            padded_image(&first, padded_width),
            padded_image(&second, padded_width),
            size,
        );

        let errors = points_a.iter().zip(&points_b).map(|(&point_a, &point_b)| {
            let expected = expected_point(point_a, size, angle, translation);
            (point_b.0 - expected.0).hypot(point_b.1 - expected.1)
        }).collect::<Vec<_>>();

        assert!(errors.len() >= 20, "too few tracks: {}", errors.len());
        assert!(points_a.iter().chain(&points_b).all(|&(x, y)| {
            x >= 0.0 && y >= 0.0 && x < size.0 as f32 && y < size.1 as f32
        }), "tracker entered row padding");
        assert!(percentile(errors, 0.9) < 2.0, "p90 endpoint error was too high");
    }

    #[test]
    fn featureless_and_undersized_frames_fail_closed() {
        let blank = image::GrayImage::from_pixel(320, 192, image::Luma([127]));
        let first = OFOpenCVDis::detect_features(0, Arc::new(blank.clone()), 320, 192);
        let second = OpticalFlowMethod::OFOpenCVDis(OFOpenCVDis::detect_features(
            1_000_000,
            Arc::new(blank),
            320,
            192,
        ));
        assert!(first.features().is_empty());
        assert!(first.optical_flow_to(&second).is_none());

        let undersized = image::GrayImage::new(10, 10);
        let first = OFOpenCVDis::detect_features(0, Arc::new(undersized.clone()), 20, 20);
        let second = OpticalFlowMethod::OFOpenCVDis(OFOpenCVDis::detect_features(
            1_000_000,
            Arc::new(undersized),
            20,
            20,
        ));
        assert!(first.features().is_empty());
        assert!(first.optical_flow_to(&second).is_none());
    }

}
