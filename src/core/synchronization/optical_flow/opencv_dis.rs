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
use opencv::{ core::{ Mat, Size, CV_8UC1, Vec2f }, prelude::{ MatTraitConst, DenseOpticalFlowTrait } };

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
        Self {
            features: Vec::new(),
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


            let result = || -> Result<(Vec<(f32, f32)>, Vec<(f32, f32)>), opencv::Error> {
                let a1_img = unsafe { Mat::new_size_with_data_unsafe(Size::new(self.img.width() as i32, self.img.height() as i32), CV_8UC1, self.img.as_raw().as_ptr() as *mut std::ffi::c_void, 0) }?;
                let a2_img = unsafe { Mat::new_size_with_data_unsafe(Size::new(next.img.width() as i32, next.img.height() as i32), CV_8UC1, next.img.as_raw().as_ptr() as *mut std::ffi::c_void, 0) }?;

                let mut of = Mat::default();
                let mut optflow = opencv::video::DISOpticalFlow::create(opencv::video::DISOpticalFlow_PRESET_FAST)?;
                optflow.calc(&a1_img, &a2_img, &mut of)?;

                let mut points_a = Vec::new();
                let mut points_b = Vec::new();
                let step = w as usize / 15; // 15 points
                for i in (0..a1_img.cols()).step_by(step) {
                    for j in (0..a1_img.rows()).step_by(step) {
                        let pt = of.at_2d::<Vec2f>(j, i)?;
                        points_a.push((i as f32, j as f32));
                        points_b.push((i as f32 + pt[0] as f32, j as f32 + pt[1] as f32));
                    }
                }
                Ok((points_a, points_b))
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
