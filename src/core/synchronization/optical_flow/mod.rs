// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use super::OpticalFlowPair;
use std::sync::Arc;

mod akaze;        pub use self::akaze::*;
mod opencv_dis;   pub use opencv_dis::*;
mod opencv_pyrlk; pub use opencv_pyrlk::*;

/// Normalized patch correlation rejects reversible but unrelated matches at cuts
/// and occlusions. Centering and normalization tolerate local exposure gain/offset.
#[cfg(feature = "use-opencv")]
fn photometric_match(a: &image::GrayImage, b: &image::GrayImage, size: (i32, i32), from: (f32, f32), to: (f32, f32)) -> bool {
    let sample = |image: &image::GrayImage, x: f32, y: f32| -> Option<f64> {
        if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || x > (size.0 - 1) as f32 || y > (size.1 - 1) as f32 { return None; }
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(size.0 as u32 - 1), (y0 + 1).min(size.1 as u32 - 1));
        let (dx, dy) = ((x - x0 as f32) as f64, (y - y0 as f32) as f64);
        let at = |x, y| image.get_pixel(x, y).0[0] as f64;
        Some((at(x0, y0) * (1.0 - dx) + at(x1, y0) * dx) * (1.0 - dy) +
             (at(x0, y1) * (1.0 - dx) + at(x1, y1) * dx) * dy)
    };
    let (mut count, mut sa, mut sb, mut saa, mut sbb, mut sab) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for y in -5..=5 {
        for x in -5..=5 {
            if let (Some(a), Some(b)) = (sample(a, from.0 + x as f32, from.1 + y as f32), sample(b, to.0 + x as f32, to.1 + y as f32)) {
                count += 1.0; sa += a; sb += b; saa += a * a; sbb += b * b; sab += a * b;
            }
        }
    }
    if count < 25.0 { return false; }
    let (va, vb) = (saa - sa * sa / count, sbb - sb * sb / count);
    va > count * 3.0 && vb > count * 3.0 && (sab - sa * sb / count) / (va * vb).sqrt() >= 0.75
}

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

    fn scene(seed: u32) -> image::GrayImage {
        image::GrayImage::from_fn(320, 240, |x, y| {
            let mut v = (x / 4).wrapping_mul(73_856_093) ^ (y / 4).wrapping_mul(19_349_663) ^ seed.wrapping_mul(83_492_791);
            v ^= v >> 13; v = v.wrapping_mul(1_274_126_177);
            image::Luma([(v >> 24) as u8])
        })
    }

    #[test]
    fn unrelated_scene_cuts_do_not_create_usable_native_tracks() {
        for method in [1, 2] {
            for seed in [3, 17, 101] {
                let from = OpticalFlowMethod::detect_features(method, 0, Arc::new(scene(seed)), 320, 240);
                let to = OpticalFlowMethod::detect_features(method, 33_333, Arc::new(scene(seed + 41)), 320, 240);
                let tracks = from.optical_flow_to(&to);
                assert!(tracks.is_none(), "Method {method}, seed {seed} invented {} correspondences at a cut", tracks.as_ref().map_or(0, |p| p.0.len()));
            }
        }
    }

    #[test]
    fn native_translation_survives_an_exposure_change() {
        let source = scene(3);
        let target = image::GrayImage::from_fn(320, 240, |x, y| {
            let pixel = if x >= 6 && y >= 4 { source.get_pixel(x - 6, y - 4).0[0] } else { 0 };
            image::Luma([((pixel as u16 * 4) / 5 + 20) as u8])
        });
        for method in [1, 2] {
            let from = OpticalFlowMethod::detect_features(method, 0, Arc::new(source.clone()), 320, 240);
            let to = OpticalFlowMethod::detect_features(method, 33_333, Arc::new(target.clone()), 320, 240);
            let (a, b) = from.optical_flow_to(&to).expect("Exposure changes should not look like cuts");
            assert!(a.len() >= 50);
            let accurate = a.iter().zip(&b).filter(|(p, q)| (q.0 - p.0 - 6.0).hypot(q.1 - p.1 - 4.0) < 0.75).count();
            assert!(accurate * 100 >= a.len() * 95, "Method {method}: {accurate}/{} matches agree with the true translation", a.len());
        }
    }

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
