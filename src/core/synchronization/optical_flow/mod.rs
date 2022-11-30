// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use super::OpticalFlowPair;
use std::sync::Arc;

mod akaze;    pub use self::akaze::*;
#[cfg(feature = "use-opencv")] mod opencv_dis;   pub use opencv_dis::*;
#[cfg(feature = "use-opencv")] mod opencv_pyrlk; pub use opencv_pyrlk::*;

#[enum_delegate::register]
pub trait OpticalFlowTrait {
    fn size(&self) -> (u32, u32);
    fn features(&self) -> &Vec<(f32, f32)>;
    fn optical_flow_to(&self, to: &OpticalFlowMethod) -> OpticalFlowPair;
    fn cleanup(&mut self);
}

#[enum_delegate::implement(OpticalFlowTrait)]
#[derive(Clone)]
pub enum OpticalFlowMethod {
    OFAkaze(OFAkaze),
    #[cfg(feature = "use-opencv")] OFOpenCVPyrLK(OFOpenCVPyrLK),
    #[cfg(feature = "use-opencv")] OFOpenCVDis(OFOpenCVDis),
}
impl OpticalFlowMethod {
    pub fn detect_features(method: u32, timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32) -> Self {
        match method {
            0 => Self::OFAkaze(OFAkaze::detect_features(timestamp_us, img, width, height)),
            #[cfg(feature = "use-opencv")] 1 => Self::OFOpenCVPyrLK(OFOpenCVPyrLK::detect_features(timestamp_us, img, width, height)),
            #[cfg(feature = "use-opencv")] 2 => Self::OFOpenCVDis(OFOpenCVDis::detect_features(timestamp_us, img, width, height)),
            _ => { log::error!("Unknown OF method {method}", ); Self::OFAkaze(OFAkaze::detect_features(timestamp_us, img, width, height)) }
        }
    }
}
