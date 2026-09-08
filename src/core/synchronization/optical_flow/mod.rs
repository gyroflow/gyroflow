// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use super::OpticalFlowPair;
use std::sync::Arc;

mod akaze;        pub use self::akaze::*;
mod opencv_dis;   pub use opencv_dis::*;
mod opencv_pyrlk; pub use opencv_pyrlk::*;

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
    OFOpenCVPyrLK(OFOpenCVPyrLK),
    OFOpenCVDis(OFOpenCVDis),
}
impl OpticalFlowMethod {
    pub fn detect_features(method: u32, timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32) -> Self {
        match method {
            0 => Self::OFAkaze(OFAkaze::detect_features(timestamp_us, img, width, height)),
            1 => Self::OFOpenCVPyrLK(OFOpenCVPyrLK::detect_features(timestamp_us, img, width, height)),
            2 => Self::OFOpenCVDis(OFOpenCVDis::detect_features(timestamp_us, img, width, height)),
            _ => { log::error!("Unknown OF method {method}", ); Self::OFAkaze(OFAkaze::detect_features(timestamp_us, img, width, height)) }
        }
    }
}

#[cfg(all(test, feature = "use-opencv"))]
mod tests {
    use super::*;

    #[test]
    fn native_track_caches_survive_image_cleanup_for_later_analysis() {
        let image = Arc::new(image::GrayImage::from_fn(320, 240, |x, y| {
            let value = (x / 4).wrapping_mul(73_856_093) ^ (y / 4).wrapping_mul(19_349_663);
            image::Luma([(value >> 17) as u8])
        }));
        for method in [1, 2] {
            let mut from = OpticalFlowMethod::detect_features(method, 0, image.clone(), 320, 240);
            let mut to = OpticalFlowMethod::detect_features(method, 33_333, image.clone(), 320, 240);
            let expected = from.optical_flow_to(&to).expect("Identical textured images should match");
            from.cleanup();
            to.cleanup();
            assert_eq!(from.optical_flow_to(&to), Some(expected), "Method {method} lost cached tracks after dropping image storage");
        }
    }
}
