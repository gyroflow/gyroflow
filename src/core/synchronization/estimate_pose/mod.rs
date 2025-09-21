// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use nalgebra::{ Rotation3, UnitVector3 };
use crate::stabilization::ComputeParams;
use super::PoseMethodKind;
use super::OpticalFlowPair;

mod almeida;            pub use self::almeida::*;
mod eight_point;        pub use self::eight_point::*;
mod find_essential_mat; pub use self::find_essential_mat::*;
mod find_homography;    pub use self::find_homography::*;

#[derive(Clone, Debug)]
pub struct RelativePose {
    pub rotation: Rotation3<f64>,
    pub translation_dir_cam: Option<UnitVector3<f64>>, // unit vector in camera frame (+Z forward convention)
    pub inlier_ratio: Option<f64>,
    pub median_epi_err: Option<f64>,
}

#[enum_delegate::register]
pub trait EstimateRelativePoseTrait {
    fn init(&mut self, params: &ComputeParams);
    fn estimate_relative_pose(&self, pairs: &OpticalFlowPair, size: (u32, u32), params: &ComputeParams, timestamp_us: i64, next_timestamp_us: i64) -> Option<RelativePose>;
}

#[enum_delegate::implement(EstimateRelativePoseTrait)]
#[derive(Clone)]
pub enum RelativePoseMethod {
    PoseFindEssentialMat(PoseFindEssentialMat),
    PoseAlmeida(PoseAlmeida),
    PoseEightPoint(PoseEightPoint),
    PoseFindHomography(PoseFindHomography),
}

impl From<&PoseMethodKind> for RelativePoseMethod {
    fn from(cfg: &PoseMethodKind) -> Self {
        match cfg {
            PoseMethodKind::EssentialLMEDS => Self::PoseFindEssentialMat(PoseFindEssentialMat { robust_method: find_essential_mat::RobustMethod::LMEDS }),
            PoseMethodKind::EssentialRANSAC => Self::PoseFindEssentialMat(PoseFindEssentialMat { robust_method: find_essential_mat::RobustMethod::RANSAC }),
            PoseMethodKind::Almeida => Self::PoseAlmeida(Default::default()),
            PoseMethodKind::EightPoint => Self::PoseEightPoint(Default::default()),
            PoseMethodKind::Homography => Self::PoseFindHomography(Default::default()),
        }
    }
}

