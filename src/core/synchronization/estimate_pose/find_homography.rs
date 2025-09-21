// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#![allow(unused_variables, dead_code)]

//! Homography-based pose estimation for camera stabilization
//!
//! This module implements pose estimation using homography decomposition, which is particularly
//! effective for scenes with dominant planar surfaces (e.g., ground, walls, buildings).
//!
//! ## How it works:
//! 1. **Homography Estimation**: Finds the 3x3 homography matrix H that relates points between
//!    two camera views: x2 = H * x1
//! 2. **Decomposition**: Decomposes H into rotation R, translation t, and plane normal n:
//!    H = K * (R + t * n^T / d) * K^(-1)
//!    Where K is camera intrinsics, d is distance to plane
//! 3. **Solution Selection**: Chooses the solution with smallest translation magnitude
//! 4. **Translation Extraction**: Extracts translation direction (magnitude is ambiguous)
//!
//! ## Advantages:
//! - Works well with planar scenes (common in video stabilization)
//! - Provides both rotation and translation estimates
//! - Robust to outliers using RANSAC
//!
//! ## Limitations:
//! - Assumes dominant planar surface in scene
//! - Translation magnitude is not meaningful (scale ambiguity)
//! - May return multiple valid solutions
use super::super::OpticalFlowPair;
use super::{ EstimateRelativePoseTrait, RelativePose };

use nalgebra::Rotation3;
#[cfg(feature = "use-opencv")]
use opencv::{ core::{ Mat, Point2f, Vector }, prelude::MatTraitConst };

use crate::stabilization::*;

#[derive(Default, Clone)]
pub struct PoseFindHomography;


impl EstimateRelativePoseTrait for PoseFindHomography {
    fn init(&mut self, _: &ComputeParams) { }
    fn estimate_relative_pose(&self, pairs: &OpticalFlowPair, size: (u32, u32), params: &ComputeParams, timestamp_us: i64, next_timestamp_us: i64) -> Option<RelativePose> {
        let (pts1, pts2) = pairs.as_ref()?;

        #[cfg(feature = "use-opencv")]
        let result = || -> Result<RelativePose, opencv::Error> {
            // Step 1: Undistort feature points to remove lens distortion
            // This is crucial for accurate homography estimation
            let pts11 = undistort_points_for_optical_flow(&pts1, timestamp_us, params, size);
            let pts22 = undistort_points_for_optical_flow(&pts2, next_timestamp_us, params, size);

            // Convert to OpenCV Point2f format
            let pts1 = pts11.into_iter().map(|(x, y)| Point2f::new(x, y)).collect::<Vec<Point2f>>();
            let pts2 = pts22.into_iter().map(|(x, y)| Point2f::new(x, y)).collect::<Vec<Point2f>>();

            let a1_pts = Mat::from_slice(&pts1)?;
            let a2_pts = Mat::from_slice(&pts2)?;

            // Step 2: Use identity camera matrix for normalized coordinates
            // Since we're working with undistorted points, we can use identity matrix
            let cam_matrix = Mat::eye(3, 3, opencv::core::CV_64F)?;

            let mut inliers = Mat::default();

            // Step 3: Find homography matrix using RANSAC
            // Homography relates points between two views: x2 = H * x1
            // For planar scenes: H = K * (R + t * n^T / d) * K^(-1)
            // Where R=rotation, t=translation, n=plane normal, d=distance to plane
            let homography = opencv::calib3d::find_homography_ext(&a1_pts, &a2_pts, opencv::calib3d::RANSAC, 0.001, &mut inliers, 2000, 0.999)?;

            // Step 4: Decompose homography into rotation, translation, and plane normal
            // This can return multiple valid solutions due to the ambiguity in homography decomposition
            let mut r: Vector<Mat> = Default::default();  // Rotation matrices
            let mut t: Vector<Mat> = Default::default();  // Translation vectors
            let mut n: Vector<Mat> = Default::default();  // Plane normals

            opencv::calib3d::decompose_homography_mat(&homography, &cam_matrix, &mut r, &mut t, &mut n)?;

            // Step 5: Select the best solution based on translation magnitude
            // We choose the solution with the smallest translation magnitude as it's most likely
            // to represent the actual camera motion (camera movements are typically small)
            if let Some((r, t, _)) = r.iter().zip(t.iter()).fold(None, |cr, (r, t)| {
                    let dot = t.dot(&t).unwrap_or(0.0);  // Calculate ||t||^2
                    match cr {
                        Some((cr, ct, m)) if m < dot => Some((cr, ct, m)),  // Keep solution with smaller translation
                        _ => Some((r, t, dot)),  // First solution or better solution
                    }
                }) {
                let rotation = cv_to_na(r)?;
                
                // Step 6: Extract translation vector and convert to unit vector
                // The translation direction is what we need for stabilization
                // Magnitude is not meaningful due to scale ambiguity in homography
                let tx = *t.at_2d::<f64>(0, 0)?;
                let ty = *t.at_2d::<f64>(1, 0)?;
                let tz = *t.at_2d::<f64>(2, 0)?;
                let norm = (tx * tx + ty * ty + tz * tz).sqrt();
                let translation_dir_cam = if norm > 0.0 { 
                    Some(nalgebra::Unit::new_normalize(nalgebra::Vector3::new(tx, ty, tz))) 
                } else { 
                    None  // No meaningful translation if norm is zero
                };

                // Step 7: Calculate inlier ratio for quality assessment
                // Count non-zero entries in the inlier mask
                let inlier_count = opencv::core::count_non_zero(&inliers)? as f64;
                let total_points = a1_pts.rows().max(1) as f64;
                let inlier_ratio = Some((inlier_count / total_points).min(1.0));

                Ok(RelativePose {
                    rotation,
                    translation_dir_cam,
                    inlier_ratio,
                    median_epi_err: None,  // Not calculated for homography method
                })
            } else {
                Err(opencv::Error::new(0, "Model not found".to_string()))
            }
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
