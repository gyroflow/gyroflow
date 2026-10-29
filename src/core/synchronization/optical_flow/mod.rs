// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use super::OpticalFlowPair;
use super::optical_motion::PairKlt;
use std::sync::Arc;

mod akaze;        pub use self::akaze::*;
mod opencv_dis;   pub use opencv_dis::*;
mod pyrlk;        pub use pyrlk::*;

#[enum_delegate::register]
pub trait OpticalFlowTrait {
    fn size(&self) -> (u32, u32);
    fn features(&self) -> &Vec<(f32, f32)>;
    fn optical_flow_to(&self, to: &OpticalFlowMethod) -> OpticalFlowPair;
    fn cleanup(&mut self);
    fn can_cleanup(&self) -> bool;
}

#[enum_delegate::implement(OpticalFlowTrait)]
#[derive(Clone)]
pub enum OpticalFlowMethod {
    OFAkaze(OFAkaze),
    OFPyrLK(OFPyrLK),
    OFOpenCVDis(OFOpenCVDis),
}
impl OpticalFlowMethod {
    /// Whether `method` tracks with KLT, `OFPyrLK`: then `detect_features` takes the `PairKlt` of the synchronization.
    /// DIS is OpenCV's, a build without OpenCV has KLT in its place
    pub fn uses_klt(method: u32) -> bool {
        method == 1 || (method == 2 && !cfg!(feature = "use-opencv"))
    }

    pub fn detect_features(method: u32, timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32, klt: Option<Arc<PairKlt>>) -> Self {
        match method {
            0 => Self::OFAkaze(OFAkaze::detect_features(timestamp_us, img, width, height)),
            _ if Self::uses_klt(method) => Self::OFPyrLK(OFPyrLK::detect_features(timestamp_us, img, width, height, klt)),
            2 => Self::OFOpenCVDis(OFOpenCVDis::detect_features(timestamp_us, img, width, height)),
            _ => { log::error!("Unknown OF method {method}", ); Self::OFAkaze(OFAkaze::detect_features(timestamp_us, img, width, height)) }
        }
    }
}
