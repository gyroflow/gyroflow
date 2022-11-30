// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::super::OpticalFlowPair;
use super::EstimatePoseTrait;

use arrsac::Arrsac;
use cv_core::{ FeatureMatch, Pose, sample_consensus::Consensus };
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;
use crate::stabilization::*;

pub type Match = FeatureMatch;

#[derive(Default, Clone)]
pub struct PoseEightPoint;

impl EstimatePoseTrait for PoseEightPoint {
    fn init(&mut self, _: &ComputeParams) { }

    fn estimate_pose(&self, pairs: &OpticalFlowPair, size: (u32, u32), params: &ComputeParams, timestamp_us: i64, next_timestamp_us: i64) -> Option<nalgebra::Rotation3<f64>> {
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

        // Try different thresholds for best results
        let thresholds = [1e-10, 1e-8, 1e-6];

        let mut arrsac = Arrsac::new(1e-10, Xoshiro256PlusPlus::seed_from_u64(0));
            //.initialization_hypotheses(2048)
            //.max_candidate_hypotheses(512);
        for threshold in thresholds {
            arrsac = arrsac.inlier_threshold(threshold);

            let eight_point = eight_point::EightPoint::new();
            if let Some(out) = arrsac.model(&eight_point, matches.iter().copied()) {
                let rot = out.isometry().rotation;
                return Some(nalgebra::Rotation3::from_matrix_unchecked(nalgebra::Matrix3::from_column_slice(rot.matrix().as_slice())));
                /*let rotations = cv_pinhole::EssentialMatrix::from(out).possible_rotations(1e-12, 1000).unwrap();
                if rotations[0].angle() < rotations[1].angle() {
                    Some(rotations[0])
                } else {
                    Some(rotations[1])
                }*/
            }
        }
        ::log::warn!("couldn't find model");
        None
    }
}
