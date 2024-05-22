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
    pub fn detect_features(timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32) -> Self {
        let (w, h) = (width as i32, height as i32);

        #[cfg(feature = "use-opencv")]
        let features = {
            let inp = unsafe { Mat::new_size_with_data_unsafe(Size::new(w, h), CV_8UC1, img.as_raw().as_ptr() as *mut std::ffi::c_void, img.width() as usize) };

            // opencv::imgcodecs::imwrite("D:/test.jpg", &inp, &opencv::types::VectorOfi32::new());

            let mut pts = Mat::default();

            if let Err(e) = inp.and_then(|inp| {
                opencv::imgproc::good_features_to_track(&inp, &mut pts, 200, 0.01, 10.0, &Mat::default(), 3, false, 0.04)
            }) {
                log::error!("OpenCV error {:?}", e);
            }
            (0..pts.rows()).into_iter().filter_map(|i| { let x = pts.at::<Point2f>(i).ok()?; Some((x.x, x.y))}).collect()
        };
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
            if self.img.is_empty() || next.img.is_empty() || w <= 0 || h <= 0 { return None; }

            if let Some(matched) = self.matched_points.read().get(&next.timestamp_us) {
                return Some(matched.clone());
            }

            let result = || -> Result<(Vec<(f32, f32)>, Vec<(f32, f32)>), opencv::Error> {
                let a1_img = unsafe { Mat::new_size_with_data_unsafe(Size::new(w, h), CV_8UC1, self.img.as_raw().as_ptr() as *mut std::ffi::c_void, w as usize) }?;
                let a2_img = unsafe { Mat::new_size_with_data_unsafe(Size::new(w, h), CV_8UC1, next.img.as_raw().as_ptr() as *mut std::ffi::c_void, w as usize) }?;

                let pts1: Vec<Point2f> = self.features.iter().map(|(x, y)| Point2f::new(*x as f32, *y as f32)).collect();

                let a1_pts = Mat::from_slice(&pts1)?;
                //let a2_pts = a2.features;

                let mut a2_pts = Mat::default();
                let mut status = Mat::default();
                let mut err = Mat::default();

                opencv::video::calc_optical_flow_pyr_lk(&a1_img, &a2_img, &a1_pts, &mut a2_pts, &mut status, &mut err, Size::new(21, 21), 3, TermCriteria::new(3/*count+eps*/,30,0.01)?, 0, 1e-4)?;

                let mut pts1 = Vec::with_capacity(status.rows() as usize);
                let mut pts2 = Vec::with_capacity(status.rows() as usize);
                for i in 0..status.rows() {
                    if *status.at::<u8>(i)? == 1u8 {
                        let pt1 = a1_pts.at::<Point2f>(i)?;
                        let pt2 = a2_pts.at::<Point2f>(i)?;
                        if pt1.x >= 0.0 && pt1.x < w as f32 && pt1.y >= 0.0 && pt1.y < h as f32
                        && pt2.x >= 0.0 && pt2.x < w as f32 && pt2.y >= 0.0 && pt2.y < h as f32 {
                            pts1.push((pt1.x as f32, pt1.y as f32));
                            pts2.push((pt2.x as f32, pt2.y as f32));
                        }
                    }
                }
                Ok((pts1, pts2))
            }();
            
            self.used.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            next.used.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

            match result {
                Ok(res) => {
                    self.matched_points.write().insert(next.timestamp_us, res.clone());
                    return Some(res);
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
