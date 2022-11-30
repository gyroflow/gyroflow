// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use nalgebra::Rotation3;
use crate::stabilization::ComputeParams;
use super::OpticalFlowPair;

mod almeida;     pub use self::almeida::*;
mod eight_point; pub use self::eight_point::*;
#[cfg(feature = "use-opencv")] mod find_essential_mat; pub use self::find_essential_mat::*;
#[cfg(feature = "use-opencv")] mod find_homography;    pub use self::find_homography::*;

#[enum_delegate::register]
pub trait EstimatePoseTrait {
    fn init(&mut self, params: &ComputeParams);
    fn estimate_pose(&self, pairs: &OpticalFlowPair, size: (u32, u32), params: &ComputeParams, timestamp_us: i64, next_timestamp_us: i64) -> Option<Rotation3<f64>>;
}

#[enum_delegate::implement(EstimatePoseTrait)]
#[derive(Clone)]
pub enum EstimatePoseMethod {
    #[cfg(feature = "use-opencv")] PoseFindEssentialMat(PoseFindEssentialMat),
    PoseAlmeida(PoseAlmeida),
    PoseEightPoint(PoseEightPoint),
    #[cfg(feature = "use-opencv")] PoseFindHomography(PoseFindHomography),
}
impl From<u32> for EstimatePoseMethod {
    fn from(v: u32) -> Self {
        match v {
            #[cfg(feature = "use-opencv")] 0 => Self::PoseFindEssentialMat(Default::default()),
            1 => Self::PoseAlmeida(Default::default()),
            2 => Self::PoseEightPoint(Default::default()),
            #[cfg(feature = "use-opencv")] 3 => Self::PoseFindHomography(Default::default()),
            _ => { log::error!("Unknown pose method {v}", ); Self::PoseAlmeida(Default::default()) }
        }
    }
}
