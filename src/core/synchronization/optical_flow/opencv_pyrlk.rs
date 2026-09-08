// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#![allow(unused_variables, dead_code, unused_mut)]
use super::super::{ OpticalFlowPair, OpticalFlowPoints };
use super::{ OpticalFlowTrait, OpticalFlowMethod };

use std::collections::BTreeMap;
use std::sync::Arc;
use parking_lot::RwLock;
use std::sync::atomic::AtomicU32;
#[cfg(feature = "use-opencv")]
use opencv::{ core::{ Mat, Size, Point2f, CV_8UC1, TermCriteria }, prelude::MatTraitConst };

#[derive(Clone)]
pub struct OFOpenCVPyrLK {
    features: Vec<(f32, f32)>,
    img: Arc<image::GrayImage>,
    matched_points: Arc<RwLock<BTreeMap<i64, (OpticalFlowPoints, OpticalFlowPoints)>>>,
    timestamp_us: i64,
    size: (i32, i32),
    used: Arc<AtomicU32>,
}
impl OFOpenCVPyrLK {
    fn valid_layout(img: &image::GrayImage, size: (i32, i32)) -> bool {
        size.0 > 0 && size.1 > 0 && img.width() >= size.0 as u32 && img.height() >= size.1 as u32
    }

    pub fn detect_features(timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32) -> Self {
        let (w, h) = (width as i32, height as i32);

        #[cfg(feature = "use-opencv")]
        let features = if Self::valid_layout(&img, (w, h)) {
            let inp = unsafe { Mat::new_size_with_data_unsafe(Size::new(w, h), CV_8UC1, img.as_raw().as_ptr() as *mut std::ffi::c_void, img.width() as usize) };

            // opencv::imgcodecs::imwrite("D:/test.jpg", &inp, &opencv::types::VectorOfi32::new());

            let mut pts = Mat::default();

            if let Err(e) = inp.and_then(|inp| {
                opencv::imgproc::good_features_to_track(&inp, &mut pts, 200, 0.01, 10.0, &Mat::default(), 3, false, 0.04)
            }) {
                log::error!("OpenCV error {:?}", e);
            }
            (0..pts.rows()).into_iter().filter_map(|i| { let x = pts.at::<Point2f>(i).ok()?; Some((x.x, x.y))}).collect()
        } else { Vec::new() };
        #[cfg(not(feature = "use-opencv"))]
        let features = Vec::new();

        Self {
            features,
            size: (w, h),
            img,
            timestamp_us,
            matched_points: Default::default(),
            used: Default::default()
        }
    }
}

impl OpticalFlowTrait for OFOpenCVPyrLK {
    fn size(&self) -> (u32, u32) {
        (self.size.0 as u32, self.size.1 as u32)
    }
    fn features(&self) -> &Vec<(f32, f32)> { &self.features }

    fn optical_flow_to(&self, _to: &OpticalFlowMethod) -> OpticalFlowPair {
        #[cfg(feature = "use-opencv")]
        if let OpticalFlowMethod::OFOpenCVPyrLK(next) = _to {
            let (w, h) = self.size;
            if self.features.is_empty() || self.size != next.size ||
                !Self::valid_layout(&self.img, self.size) || !Self::valid_layout(&next.img, next.size) { return None; }

            if let Some(matched) = self.matched_points.read().get(&next.timestamp_us) {
                return Some(matched.clone());
            }

            let result = || -> Result<(Vec<(f32, f32)>, Vec<(f32, f32)>), opencv::Error> {
                let a1_img = unsafe { Mat::new_size_with_data_unsafe(Size::new(w, h), CV_8UC1, self.img.as_raw().as_ptr() as *mut std::ffi::c_void, self.img.width() as usize) }?;
                let a2_img = unsafe { Mat::new_size_with_data_unsafe(Size::new(w, h), CV_8UC1, next.img.as_raw().as_ptr() as *mut std::ffi::c_void, next.img.width() as usize) }?;

                let pts1: Vec<Point2f> = self.features.iter().map(|(x, y)| Point2f::new(*x as f32, *y as f32)).collect();

                let a1_pts = Mat::from_slice(&pts1)?;
                //let a2_pts = a2.features;

                let mut a2_pts = Mat::default();
                let mut status = Mat::default();
                let mut err = Mat::default();

                opencv::video::calc_optical_flow_pyr_lk(&a1_img, &a2_img, &a1_pts, &mut a2_pts, &mut status, &mut err, Size::new(21, 21), 3, TermCriteria::new(3/*count+eps*/,30,0.01)?, 0, 1e-4)?;

                let in_frame = |p: &Point2f| p.x >= 0.0 && p.x < w as f32 && p.y >= 0.0 && p.y < h as f32;
                let mut forward_from = Vec::with_capacity(status.rows() as usize);
                let mut forward_to = Vec::with_capacity(status.rows() as usize);
                for i in 0..status.rows() {
                    if *status.at::<u8>(i)? == 1u8 {
                        let pt1 = a1_pts.at::<Point2f>(i)?;
                        let pt2 = a2_pts.at::<Point2f>(i)?;
                        if in_frame(pt1) && in_frame(pt2) {
                            forward_from.push(*pt1);
                            forward_to.push(*pt2);
                        }
                    }
                }
                if forward_to.len() < 10 { return Ok((Vec::new(), Vec::new())); }

                // Forward success alone can include plausible but incorrect matches
                // at occlusions. Track valid endpoints back and require agreement in
                // analysis-image pixels (forward-backward error, Kalal et al. 2010).
                let reverse_input = Mat::from_slice(&forward_to)?;
                let (mut returned, mut reverse_status, mut reverse_error) = (Mat::default(), Mat::default(), Mat::default());
                opencv::video::calc_optical_flow_pyr_lk(&a2_img, &a1_img, &reverse_input, &mut returned,
                    &mut reverse_status, &mut reverse_error, Size::new(21, 21), 3,
                    TermCriteria::new(3, 30, 0.01)?, 0, 1e-4)?;

                let mut pts1 = Vec::with_capacity(forward_to.len());
                let mut pts2 = Vec::with_capacity(forward_to.len());
                for (i, (from, to)) in forward_from.iter().zip(&forward_to).enumerate() {
                    if *reverse_status.at::<u8>(i as i32)? != 1 { continue; }
                    let back = returned.at::<Point2f>(i as i32)?;
                    if in_frame(back) && (back.x - from.x).hypot(back.y - from.y) <= 1.5 {
                        pts1.push((from.x, from.y));
                        pts2.push((to.x, to.y));
                    }
                }
                Ok((pts1, pts2))
            }();
            
            match result {
                Ok(res) => {
                    // Keep at least ten reliable correspondences for pose estimation.
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

    fn texture(width: u32, height: u32) -> image::GrayImage {
        image::GrayImage::from_fn(width, height, |x, y| {
            let mut v = (x / 4).wrapping_mul(73_856_093) ^ (y / 4).wrapping_mul(19_349_663);
            v ^= v >> 13;
            v = v.wrapping_mul(1_274_126_177);
            image::Luma([(v >> 24) as u8])
        })
    }

    fn shifted(source: &image::GrayImage, occlude: bool) -> image::GrayImage {
        let other = texture(source.width() + 113, source.height() + 79);
        image::GrayImage::from_fn(source.width(), source.height(), |x, y| {
            if occlude && (70..245).contains(&x) && (55..190).contains(&y) {
                *other.get_pixel(x + 113, y + 79)
            } else if x >= 6 && y >= 4 {
                *source.get_pixel(x - 6, y - 4)
            } else {
                image::Luma([0])
            }
        })
    }

    fn track(source: image::GrayImage, target: image::GrayImage) -> (OpticalFlowPoints, OpticalFlowPoints) {
        let (w, h) = source.dimensions();
        let from = OpticalFlowMethod::OFOpenCVPyrLK(OFOpenCVPyrLK::detect_features(0, Arc::new(source), w, h));
        let to = OpticalFlowMethod::OFOpenCVPyrLK(OFOpenCVPyrLK::detect_features(33_333, Arc::new(target), w, h));
        from.optical_flow_to(&to).expect("Textured translated frames should retain usable tracks")
    }

    #[test]
    fn pyr_lk_recovers_translation_with_useful_support() {
        let source = texture(320, 240);
        let target = shifted(&source, false);
        let (from, to) = track(source, target);
        assert!(from.len() >= 60, "Only {} tracks survived", from.len());
        let accurate = from.iter().zip(&to).filter(|(a, b)| {
            (b.0 - a.0 - 6.0).abs() < 0.5 && (b.1 - a.1 - 4.0).abs() < 0.5
        }).count();
        assert!(accurate * 100 >= from.len() * 95, "{accurate}/{} tracks agree with the known translation", from.len());
    }

    #[test]
    fn pyr_lk_occlusion_tracks_return_to_their_source_location() {
        let source = texture(320, 240);
        let target = shifted(&source, true);
        let (from, to) = track(source.clone(), target.clone());
        assert!(from.len() >= 30, "Rejecting all motion is not a useful solution");
        let points: Vec<Point2f> = to.iter().map(|p| Point2f::new(p.0, p.1)).collect();
        let points = Mat::from_slice(&points).unwrap();
        let source_mat = unsafe { Mat::new_size_with_data_unsafe(Size::new(320, 240), CV_8UC1, source.as_raw().as_ptr() as *mut std::ffi::c_void, 320) }.unwrap();
        let target_mat = unsafe { Mat::new_size_with_data_unsafe(Size::new(320, 240), CV_8UC1, target.as_raw().as_ptr() as *mut std::ffi::c_void, 320) }.unwrap();
        let (mut returned, mut status, mut error) = (Mat::default(), Mat::default(), Mat::default());
        opencv::video::calc_optical_flow_pyr_lk(&target_mat, &source_mat, &points, &mut returned,
            &mut status, &mut error, Size::new(21, 21), 3, TermCriteria::new(3, 30, 0.01).unwrap(), 0, 1e-4).unwrap();
        let inconsistent = from.iter().enumerate().filter(|(i, original)| {
            let p = returned.at::<Point2f>(*i as i32).unwrap();
            *status.at::<u8>(*i as i32).unwrap() != 1 ||
                (p.x - original.0).hypot(p.y - original.1) > 1.501
        }).count();
        assert_eq!(inconsistent, 0, "Occlusion produced non-repeatable correspondences");
    }

    #[test]
    fn pyr_lk_textureless_frames_do_not_invent_motion() {
        let image = Arc::new(image::GrayImage::from_pixel(320, 240, image::Luma([127])));
        let from = OpticalFlowMethod::OFOpenCVPyrLK(OFOpenCVPyrLK::detect_features(0, image.clone(), 320, 240));
        let to = OpticalFlowMethod::OFOpenCVPyrLK(OFOpenCVPyrLK::detect_features(33_333, image, 320, 240));
        assert!(from.optical_flow_to(&to).is_none());
    }

    #[test]
    fn pyr_lk_handles_different_row_pitches() {
        let source = texture(320, 240);
        let target = shifted(&source, false);
        let padded_source = image::GrayImage::from_fn(336, 240, |x, y| if x < 320 { *source.get_pixel(x, y) } else { image::Luma([255]) });
        let padded_target = image::GrayImage::from_fn(352, 240, |x, y| if x < 320 { *target.get_pixel(x, y) } else { image::Luma([0]) });
        let from = OpticalFlowMethod::OFOpenCVPyrLK(OFOpenCVPyrLK::detect_features(0, Arc::new(padded_source), 320, 240));
        let to = OpticalFlowMethod::OFOpenCVPyrLK(OFOpenCVPyrLK::detect_features(33_333, Arc::new(padded_target), 320, 240));
        let (a, b) = from.optical_flow_to(&to).unwrap();
        assert!(a.len() >= 60);
        let accurate = a.iter().zip(&b).filter(|(a, b)| (b.0 - a.0 - 6.0).abs() < 0.5 && (b.1 - a.1 - 4.0).abs() < 0.5).count();
        assert!(accurate * 100 >= a.len() * 95);
    }

    #[test]
    fn pyr_lk_rejects_incompatible_frame_geometry_before_native_access() {
        let from = OpticalFlowMethod::OFOpenCVPyrLK(OFOpenCVPyrLK::detect_features(0, Arc::new(texture(320, 240)), 320, 240));
        let smaller = OpticalFlowMethod::OFOpenCVPyrLK(OFOpenCVPyrLK::detect_features(33_333, Arc::new(texture(160, 120)), 160, 120));
        assert!(from.optical_flow_to(&smaller).is_none());
        let invalid = OFOpenCVPyrLK::detect_features(0, Arc::new(texture(160, 120)), 320, 240);
        assert!(invalid.features().is_empty());
    }
}
