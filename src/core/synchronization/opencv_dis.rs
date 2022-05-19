// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use nalgebra::{ Rotation3, Matrix3, Vector4 };
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::sync::Arc;
use parking_lot::RwLock;
use opencv::core::{ Mat, Size, Point2f, CV_8UC1, Vec2f };
use opencv::prelude::MatTraitConst;
use opencv::prelude::DenseOpticalFlow;
use super::{ EstimatorItem, EstimatorItemInterface, OpticalFlowPair };

use crate::stabilization::ComputeParams;

#[derive(Clone)]
pub struct ItemOpenCVDis {
    features: Vec<(f64, f64)>,
    img: Arc<image::GrayImage>,
    matched_points: Arc<RwLock<BTreeMap<i64, (Vec<(f64, f64)>, Vec<(f64, f64)>)>>>,
    timestamp_us: i64,
    size: (i32, i32)
}

impl EstimatorItemInterface for ItemOpenCVDis {
    fn get_features(&self) -> &Vec<(f64, f64)> { &self.features }
    fn rescale(&mut self, _ratio: f32) { }
    
    fn estimate_pose(&self, next: &EstimatorItem, camera_matrix: Matrix3<f64>, coeffs: Vector4<f64>, params: &ComputeParams) -> Option<Rotation3<f64>> {
        let (pts1, pts2) = self.get_matched_features(next)?;

        let result = || -> Result<Rotation3<f64>, opencv::Error> {
            let pts11 = crate::stabilization::undistort_points(&pts1, camera_matrix, coeffs.as_slice(), Matrix3::identity(), None, None, params);
            let pts22 = crate::stabilization::undistort_points(&pts2, camera_matrix, coeffs.as_slice(), Matrix3::identity(), None, None, params);

            let pts1 = pts11.into_iter().map(|(x, y)| Point2f::new(x as f32, y as f32)).collect::<Vec<Point2f>>();
            let pts2 = pts22.into_iter().map(|(x, y)| Point2f::new(x as f32, y as f32)).collect::<Vec<Point2f>>();

            let a1_pts = Mat::from_slice(&pts1)?;
            let a2_pts = Mat::from_slice(&pts2)?;

            let identity = Mat::eye(3, 3, opencv::core::CV_64F)?;

            let mut mask = Mat::default();
            let e = opencv::calib3d::find_essential_mat(&a1_pts, &a2_pts, &identity, opencv::calib3d::RANSAC, 0.999, 0.0005, 1000, &mut mask)?;
        
            let mut r1 = Mat::default();
            let mut t = Mat::default();
            
            let inliers = opencv::calib3d::recover_pose_triangulated(&e, &a1_pts, &a2_pts, &identity, &mut r1, &mut t, 100000.0, &mut mask, &mut Mat::default())?;
            if inliers < 20 {
                return Err(opencv::Error::new(0, "Model not found".to_string()));
            }
            
            cv_to_rot2(r1)
        }();

        match result {
            Ok(res) => Some(res),
            Err(e) => {
                log::error!("OpenCV error: {:?}", e);
                None
            }
        }
    }
    
    fn optical_flow_to(&self, to: &EstimatorItem) -> OpticalFlowPair {
        self.get_matched_features(to)
    }
    fn cleanup(&mut self) {
        self.img = Arc::new(image::GrayImage::default());
    }
}

impl ItemOpenCVDis {
    pub fn detect_features(timestamp_us: i64, img: Arc<image::GrayImage>) -> Self {
        let (w, h) = (img.width() as i32, img.height() as i32);
        Self {
            features: Vec::new(),
            timestamp_us,
            size: (w, h),
            matched_points: Default::default(),
            img
        }
    }
    
    fn get_matched_features(&self, next: &EstimatorItem) -> Option<(Vec<(f64, f64)>, Vec<(f64, f64)>)> {
        if let EstimatorItem::ItemOpenCVDis(next) = next {
            let (w, h) = self.size;
            if self.img.is_empty() || next.img.is_empty() || w <= 0 || h <= 0 { return None; }

            if let Some(matched) = self.matched_points.read().get(&next.timestamp_us) {
                return Some(matched.clone());
            }

            let result = || -> Result<(Vec<(f64, f64)>, Vec<(f64, f64)>), opencv::Error> {
                let a1_img = unsafe { Mat::new_size_with_data(Size::new(w, h), CV_8UC1, self.img.as_raw().as_ptr() as *mut c_void, w as usize) }?;
                let a2_img = unsafe { Mat::new_size_with_data(Size::new(w, h), CV_8UC1, next.img.as_raw().as_ptr() as *mut c_void, w as usize) }?;
                
                let mut of = Mat::default();
                let mut optflow = <dyn opencv::video::DISOpticalFlow>::create(opencv::video::DISOpticalFlow_PRESET_FAST)?;
                optflow.calc(&a1_img, &a2_img, &mut of)?;

                let mut points_a = Vec::new();
                let mut points_b = Vec::new();
                let step = w as usize / 25; // 25 points
                for i in (0..a1_img.cols()).step_by(step) {
                    for j in (0..a1_img.rows()).step_by(step) {
                        let pt = of.at_2d::<Vec2f>(j, i)?;
                        points_a.push((i as f64, j as f64));
                        points_b.push((i as f64 + pt[0] as f64, j as f64 + pt[1] as f64));
                    }
                }
                Ok((points_a, points_b))
            }();

            match result {
                Ok(res) => {
                    self.matched_points.write().insert(next.timestamp_us, res.clone());
                    Some(res)
                },
                Err(e) => {
                    log::error!("OpenCV error: {:?}", e);
                    None
                }
            }
        } else {
            None
        }
    }
}

fn cv_to_rot2(r1: Mat) -> Result<Rotation3<f64>, opencv::Error> {
    if r1.typ() != opencv::core::CV_64FC1 {
        return Err(opencv::Error::new(0, "Invalid matrix type".to_string()));
    }
    Ok(Rotation3::from_matrix_unchecked(nalgebra::Matrix3::new(
        *r1.at_2d::<f64>(0, 0)?, *r1.at_2d::<f64>(0, 1)?, *r1.at_2d::<f64>(0, 2)?,
        *r1.at_2d::<f64>(1, 0)?, *r1.at_2d::<f64>(1, 1)?, *r1.at_2d::<f64>(1, 2)?,
        *r1.at_2d::<f64>(2, 0)?, *r1.at_2d::<f64>(2, 1)?, *r1.at_2d::<f64>(2, 2)?
    )))
}
