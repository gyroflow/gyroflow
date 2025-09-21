// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::super::OpticalFlowPair;
use super::{ EstimateRelativePoseTrait, RelativePose };

use arrsac::Arrsac;
use cv_core::{ FeatureMatch, Pose, sample_consensus::Consensus };
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;
use crate::stabilization::*;

pub type Match = FeatureMatch;

#[derive(Default, Clone)]
pub struct PoseEightPoint;

impl EstimateRelativePoseTrait for PoseEightPoint {
    fn init(&mut self, _: &ComputeParams) { }
    fn estimate_relative_pose(&self, pairs: &OpticalFlowPair, size: (u32, u32), params: &ComputeParams, timestamp_us: i64, next_timestamp_us: i64) -> Option<RelativePose> {
        use cv_core::nalgebra::{ UnitVector3, Point2 };

        let (pts1, pts2) = pairs.as_ref()?;

        let pts1 = crate::stabilization::undistort_points_for_optical_flow(&pts1, timestamp_us, params, size);
        let pts2 = crate::stabilization::undistort_points_for_optical_flow(&pts2, next_timestamp_us, params, size);

        let matches: Vec<Match> = pts1.into_iter().zip(pts2.into_iter()).map(|(i1, i2)| {
                FeatureMatch(
                    UnitVector3::new_normalize(Point2::new(i1.0 as f64, i1.1 as f64).to_homogeneous()),
                    UnitVector3::new_normalize(Point2::new(i2.0 as f64, i2.1 as f64).to_homogeneous())
                )
            })
            .collect();

        let thresholds = [1e-10, 1e-8, 1e-6];
        let mut arrsac = Arrsac::new(1e-10, Xoshiro256PlusPlus::seed_from_u64(0));
        for threshold in thresholds {
            arrsac = arrsac.inlier_threshold(threshold);

            let eight_point = eight_point::EightPoint::new();
            if let Some(out) = arrsac.model(&eight_point, matches.iter().copied()) {
                let iso = out.isometry();
                let rot = iso.rotation;
                let t = iso.translation.vector;
                let tdir = if t.norm() > 0.0 { Some(nalgebra::Unit::new_normalize(nalgebra::Vector3::new(t.x, t.y, t.z))) } else { None };
                return Some(RelativePose {
                    rotation: nalgebra::Rotation3::from_matrix_unchecked(nalgebra::Matrix3::from_column_slice(rot.matrix().as_slice())),
                    translation_dir_cam: tdir,
                    inlier_ratio: None,
                    median_epi_err: None,
                });
            }
        }
        None
    }
}
