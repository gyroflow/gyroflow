// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#![allow(unused_variables, dead_code)]
use super::super::OpticalFlowPair;
use super::EstimatePoseTrait;

use nalgebra::Rotation3;
#[cfg(feature = "use-opencv")]
use opencv::{ core::{ Mat, Point2f }, prelude::MatTraitConst };

use crate::stabilization::*;

#[derive(Default, Clone)]
pub struct PoseFindEssentialMat;

impl EstimatePoseTrait for PoseFindEssentialMat {
    fn init(&mut self, _: &ComputeParams) { }

    fn estimate_pose(&self, pairs: &OpticalFlowPair, size: (u32, u32), params: &ComputeParams, timestamp_us: i64, next_timestamp_us: i64) -> Option<Rotation3<f64>> {
        let (pts1, pts2) = pairs.as_ref()?;

        #[cfg(feature = "use-opencv")]
        let result = || -> Result<Rotation3<f64>, opencv::Error> {
            let pts11 = undistort_points_for_optical_flow(&pts1, timestamp_us, params, size);
            let pts22 = undistort_points_for_optical_flow(&pts2, next_timestamp_us, params, size);

            let pts1 = pts11.into_iter().map(|(x, y)| Point2f::new(x, y)).collect::<Vec<Point2f>>();
            let pts2 = pts22.into_iter().map(|(x, y)| Point2f::new(x, y)).collect::<Vec<Point2f>>();

            let a1_pts = Mat::from_slice(&pts1)?;
            let a2_pts = Mat::from_slice(&pts2)?;

            let identity = Mat::eye(3, 3, opencv::core::CV_64F)?;

            let mut mask = Mat::default();
            let e = opencv::calib3d::find_essential_mat(&a1_pts, &a2_pts, &identity, opencv::calib3d::LMEDS, 0.999, 0.00001, 4000, &mut mask)?;

            let mut r1 = Mat::default();
            let mut t = Mat::default();

            let inliers = opencv::calib3d::recover_pose_triangulated(&e, &a1_pts, &a2_pts, &identity, &mut r1, &mut t, 100000.0, &mut mask, &mut Mat::default())?;
            if inliers < 10 {
                return Err(opencv::Error::new(0, "Model not found".to_string()));
            }

            cv_to_na(r1)
        }();
        #[cfg(not(feature = "use-opencv"))]
        let result = Err(());

        match result {
            Ok(res) => Some(res),
            Err(e) => {
                log::error!("OpenCV error: {:?}", e);
                None
            }
        }
    }
}

#[cfg(feature = "use-opencv")]
fn cv_to_na(r1: Mat) -> Result<Rotation3<f64>, opencv::Error> {
    if r1.typ() != opencv::core::CV_64FC1 {
        return Err(opencv::Error::new(0, "Invalid matrix type".to_string()));
    }
    Ok(Rotation3::from_matrix_unchecked(nalgebra::Matrix3::new(
        *r1.at_2d::<f64>(0, 0)?, *r1.at_2d::<f64>(0, 1)?, *r1.at_2d::<f64>(0, 2)?,
        *r1.at_2d::<f64>(1, 0)?, *r1.at_2d::<f64>(1, 1)?, *r1.at_2d::<f64>(1, 2)?,
        *r1.at_2d::<f64>(2, 0)?, *r1.at_2d::<f64>(2, 1)?, *r1.at_2d::<f64>(2, 2)?
    )))
}
