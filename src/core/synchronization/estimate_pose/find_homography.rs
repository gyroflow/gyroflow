// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::super::OpticalFlowPair;
use super::EstimatePoseTrait;

use nalgebra::Rotation3;
use opencv::core::{ Mat, Point2f, Vector };
use opencv::prelude::MatTraitConst;

use crate::stabilization::*;

#[derive(Default, Clone)]
pub struct PoseFindHomography;

impl EstimatePoseTrait for PoseFindHomography {
    fn init(&mut self, _: &ComputeParams) { }

    fn estimate_pose(&self, pairs: &OpticalFlowPair, size: (u32, u32), params: &ComputeParams, timestamp_us: i64, next_timestamp_us: i64) -> Option<Rotation3<f64>> {
        let (pts1, pts2) = pairs.as_ref()?;

        let result = || -> Result<Rotation3<f64>, opencv::Error> {
            let pts11 = undistort_points_for_optical_flow(&pts1, timestamp_us, params, size);
            let pts22 = undistort_points_for_optical_flow(&pts2, next_timestamp_us, params, size);

            let pts1 = pts11.into_iter().map(|(x, y)| Point2f::new(x, y)).collect::<Vec<Point2f>>();
            let pts2 = pts22.into_iter().map(|(x, y)| Point2f::new(x, y)).collect::<Vec<Point2f>>();

            let a1_pts = Mat::from_slice(&pts1)?;
            let a2_pts = Mat::from_slice(&pts2)?;

            let cam_matrix = Mat::eye(3, 3, opencv::core::CV_64F)?;

            let mut inliers = Mat::default();

            let homography = opencv::calib3d::find_homography_ext(&a1_pts, &a2_pts, opencv::calib3d::RANSAC, 0.001, &mut inliers, 2000, 0.999)?;

            let mut r: Vector<Mat> = Default::default();
            let mut t: Vector<Mat> = Default::default();
            let mut n: Vector<Mat> = Default::default();

            opencv::calib3d::decompose_homography_mat(&homography, &cam_matrix, &mut r, &mut t, &mut n)?;

            if let Some((r, _)) = r.iter().zip(t.iter()).fold(None, |cr, (r, t)| {
                    let dot = t.dot(&t).unwrap_or(0.0);
                    match cr {
                        Some((cr, m)) if m < dot => Some((cr, m)),
                        _ => Some((r, dot)),
                    }
                }) {
                    cv_to_na(r)
            } else {
                Err(opencv::Error::new(0, "Model not found".to_string()))
            }
        }();

        match result {
            Ok(res) => Some(res),
            Err(e) => {
                log::error!("OpenCV error: {:?}", e);
                None
            }
        }
    }
}

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
