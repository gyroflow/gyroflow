// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use nalgebra::{ Rotation3, Matrix3, Vector4 };
use std::collections::BTreeMap;
use std::ffi::c_void;
use opencv::core::{ Mat, Size, Point2f, TermCriteria, CV_8UC1 };
use opencv::prelude::MatTraitConst;

// use opencv::prelude::{PlatformInfoTraitConst, DeviceTraitConst, UMatTraitConst};
// use opencv::core::{UMat, UMatUsageFlags, AccessFlag::ACCESS_READ};

#[derive(Default, Clone)]
pub struct ItemOpenCV {
    features: Mat,
    img_bytes: Vec<u8>,
    size: (i32, i32),
    optical_flow: BTreeMap<usize, Option<(Vec<Point2f>, Vec<Point2f>)>>
}
unsafe impl Send for ItemOpenCV { }
unsafe impl Sync for ItemOpenCV { }

impl ItemOpenCV {
    pub fn detect_features(_frame: usize, img: image::GrayImage) -> Self {
        let (w, h) = (img.width() as i32, img.height() as i32);
        let mut bytes = img.into_raw();
        let inp = unsafe { Mat::new_size_with_data(Size::new(w, h), CV_8UC1, bytes.as_mut_ptr() as *mut c_void, w as usize) };
        
        // opencv::imgcodecs::imwrite("D:/test.jpg", &inp, &opencv::types::VectorOfi32::new());
        
        let mut pts = Mat::default();

        //let inp = inp.get_umat(ACCESS_READ, UMatUsageFlags::USAGE_DEFAULT).unwrap();
        //let mut pts = UMat::new(UMatUsageFlags::USAGE_DEFAULT);

        if let Err(e) = inp.and_then(|inp| {
            opencv::imgproc::good_features_to_track(&inp, &mut pts, 500, 0.01, 10.0, &Mat::default(), 3, false, 0.04)
        }) {
            log::error!("OpenCV error {:?}", e);
        }

        //let pts = pts.get_mat(ACCESS_READ).unwrap().clone();
        Self {
            features: pts,
            size: (w, h),
            img_bytes: bytes,
            optical_flow: BTreeMap::new()
        }
    }
    
    pub fn get_features_count(&self) -> usize {
        self.features.rows() as usize
    }
    pub fn get_feature_at_index(&self, i: usize) -> (f32, f32) {
        if let Ok(pt) = self.features.at::<Point2f>(i as i32) {
            (pt.x, pt.y)
        } else {
            (0.0, 0.0)
        }
    }
    pub fn rescale(&mut self, ratio: f32) {
        use opencv::prelude::MatTrait;
        for i in 0..self.features.rows() {
            let mut pt = self.features.at_mut::<Point2f>(i).unwrap();
            pt.x *= ratio;
            pt.y *= ratio;
        }
    }
    
    pub fn estimate_pose(&mut self, next: &mut Self, camera_matrix: Matrix3<f64>, coeffs: Vector4<f64>) -> Option<Rotation3<f64>> {
        let (pts1, pts2) = self.get_matched_features(next)?;

        let result = || -> Result<Rotation3<f64>, opencv::Error> {
            let pts11 = pts1.iter().map(|x| (x.x as f64, x.y as f64)).collect::<Vec<(f64, f64)>>();
            let pts22 = pts2.iter().map(|x| (x.x as f64, x.y as f64)).collect::<Vec<(f64, f64)>>();
            let pts11 = crate::undistortion::undistort_points(&pts11, camera_matrix, coeffs.as_slice(), Matrix3::identity(), None, None);
            let pts22 = crate::undistortion::undistort_points(&pts22, camera_matrix, coeffs.as_slice(), Matrix3::identity(), None, None);

            let pts1 = pts11.into_iter().map(|(x, y)| Point2f::new(x as f32, y as f32)).collect::<Vec<Point2f>>();
            let pts2 = pts22.into_iter().map(|(x, y)| Point2f::new(x as f32, y as f32)).collect::<Vec<Point2f>>();

            let a1_pts = Mat::from_slice(&pts1)?;
            let a2_pts = Mat::from_slice(&pts2)?;
            
            // let cam_matrix = Mat::from_slice_2d(&[
            //     [camera_matrix[(0, 0)], 0.0, camera_matrix[(0, 2)]],
            //     [0.0, camera_matrix[(1, 1)], camera_matrix[(1, 0)]],
            //     [0.0, 0.0, 1.0]
            // ])?;
            let identity = Mat::from_slice_2d(&[
                [1.0f64, 0.0f64, 0.0f64],
                [0.0f64, 1.0f64, 0.0f64],
                [0.0f64, 0.0f64, 1.0f64]
            ])?;

            // let e = opencv::calib3d::find_essential_mat(&a1_pts, &a2_pts, &cam_matrix, &Mat::default(), &scaled_k, &Mat::default(), opencv::calib3d::RANSAC, 0.999, 0.1, &mut Mat::default())?;
            let mut mask = Mat::default();
            let e = opencv::calib3d::find_essential_mat(&a1_pts, &a2_pts, &identity, opencv::calib3d::RANSAC, 0.999, 0.0005, 1000, &mut mask)?;
        
            let mut r1 = Mat::default();
            // let mut r2 = Mat::default();
            let mut t = Mat::default();
            
            let inliers = opencv::calib3d::recover_pose_triangulated(&e, &a1_pts, &a2_pts, &identity, &mut r1, &mut t, 100000.0, &mut mask, &mut Mat::default())?;
            if inliers < 20 {
                return Err(opencv::Error::new(0, "Model not found".to_string()));
            }
            
            cv_to_rot2(r1)
            // opencv::calib3d::decompose_essential_mat(&e, &mut r1, &mut r2, &mut t)?;
            // let r1 = cv_to_rot2(r1)?;
            // let r2 = cv_to_rot2(r2)?;
            // Ok(if r1.angle() < r2.angle() {
            //     r1
            // } else {
            //     r2
            // })
        }();

        match result {
            Ok(res) => Some(res),
            Err(e) => {
                log::error!("OpenCV error: {:?}", e);
                None
            }
        }
    }

    pub fn get_matched_features(&mut self, next: &mut Self) -> Option<(Vec<Point2f>, Vec<Point2f>)> {
        let (w, h) = self.size;
        if self.img_bytes.is_empty() || next.img_bytes.is_empty() || w <= 0 || h <= 0 { return None; }

        let result = || -> Result<(Vec<Point2f>, Vec<Point2f>), opencv::Error> {
            let a1_img = unsafe { Mat::new_size_with_data(Size::new(w, h), CV_8UC1, self.img_bytes.as_mut_ptr() as *mut c_void, w as usize) }?;
            let a2_img = unsafe { Mat::new_size_with_data(Size::new(w, h), CV_8UC1, next.img_bytes.as_mut_ptr() as *mut c_void, w as usize) }?;
            
            let a1_pts = &self.features;
            //let a2_pts = a2.features;
            
            let mut a2_pts = Mat::default();
            let mut status = Mat::default();
            let mut err = Mat::default();

            opencv::video::calc_optical_flow_pyr_lk(&a1_img, &a2_img, &a1_pts, &mut a2_pts, &mut status, &mut err, Size::new(21, 21), 3, TermCriteria::new(3/*count+eps*/,30,0.01)?, 0, 1e-4)?;

            let mut pts1: Vec<Point2f> = Vec::new();
            let mut pts2: Vec<Point2f> = Vec::new();
            for i in 0..status.rows() {
                if *status.at::<u8>(i)? == 1u8 {
                    let pt1 = a1_pts.at::<Point2f>(i)?;
                    let pt2 = a2_pts.at::<Point2f>(i)?;
                    if pt1.x >= 0.0 && pt1.x < w as f32 && pt1.y >= 0.0 && pt1.y < h as f32 
                    && pt2.x >= 0.0 && pt2.x < w as f32 && pt2.y >= 0.0 && pt2.y < h as f32 {
                        pts1.push(*pt1);
                        pts2.push(*pt2);
                    }
                }
            }
            Ok((pts1, pts2))
        }();

        match result {
            Ok(res) => Some(res),
            Err(e) => {
                log::error!("OpenCV error: {:?}", e);
                None
            }
        }
    }

    pub fn optical_flow_to_frame(&mut self, to: &mut Self, frame_offset: usize, force_update: bool) {
        if force_update || !self.optical_flow.contains_key(&frame_offset) {
            let pts = self.get_matched_features(to);
            self.optical_flow.insert(frame_offset, pts);
        }
    }

    pub fn get_optical_flow_lines(&self, frame_offset: usize, scale: f64) -> Option<(Vec<(f64, f64)>, Vec<(f64, f64)>)> {
        if let Some(&opt_pts) = self.optical_flow.get(&frame_offset).as_ref() {
            if let Some(pts) = opt_pts {
                return Some((
                    pts.0.iter().map(|x| (x.x as f64 * scale, x.y as f64 * scale )).collect::<Vec<(f64, f64)>>(),
                    pts.1.iter().map(|x| (x.x as f64 * scale, x.y as f64 * scale )).collect::<Vec<(f64, f64)>>()
                ))
            }
        }
        None
    }
}

pub fn init() -> Result<(), opencv::Error> {
    /*let opencl_have = opencv::core::have_opencl()?;
    if opencl_have {
        opencv::core::set_use_opencl(true)?;
        let mut platforms = opencv::types::VectorOfPlatformInfo::new();
        opencv::core::get_platfoms_info(&mut platforms)?;
        for (platf_num, platform) in platforms.into_iter().enumerate() {
            ::log::info!("Platform #{}: {}", platf_num, platform.name()?);
            for dev_num in 0..platform.device_number()? {
                let mut dev = opencv::core::Device::default();
                platform.get_device(&mut dev, dev_num)?;
                ::log::info!("  OpenCL device #{}: {}", dev_num, dev.name()?);
                ::log::info!("    vendor:  {}", dev.vendor_name()?);
                ::log::info!("    version: {}", dev.version()?);
            }
        }
    }
    let opencl_use = opencv::core::use_opencl()?;
    ::log::info!(
        "OpenCL is {} and {}",
        if opencl_have { "available" } else { "not available" },
        if opencl_use { "enabled" } else { "disabled" },
    );*/
    Ok(())
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
