// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright Â© 2021-2022 Adrian <adrian.eddy at gmail>

#![cfg_attr(not(feature = "use-opencv"), allow(dead_code, unused_imports))]
use super::{OpticalFlowContext, OpticalFlowMethod, OpticalFlowPair, OpticalFlowTrait};
#[cfg(feature = "use-opencv")]
use opencv::{
    core::{CV_8UC1, Mat, Size, Vec2f},
    prelude::{DenseOpticalFlowTrait, MatTraitConst},
};
use parking_lot::Mutex;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

#[derive(Clone)]
pub struct OFOpenCVDis {
    features: Vec<(f32, f32)>,
    img: Arc<image::GrayImage>,
    matches: Arc<Mutex<BTreeMap<i64, OpticalFlowPair>>>,
    timestamp_us: i64,
    size: (u32, u32),
    used: Arc<AtomicU32>,
    context: Arc<OpticalFlowContext>,
}

impl OFOpenCVDis {
    pub fn detect_features(
        timestamp_us: i64,
        img: Arc<image::GrayImage>,
        width: u32,
        height: u32,
    ) -> Self {
        Self::with_context(timestamp_us, img, width, height, Arc::default())
    }

    pub fn with_context(
        timestamp_us: i64,
        img: Arc<image::GrayImage>,
        width: u32,
        height: u32,
        context: Arc<OpticalFlowContext>,
    ) -> Self {
        Self {
            features: Vec::new(),
            timestamp_us,
            size: (width, height),
            matches: Default::default(),
            img,
            used: Default::default(),
            context,
        }
    }

    #[cfg(feature = "use-opencv")]
    fn track(&self, next: &Self) -> opencv::Result<OpticalFlowPair> {
        let (w, h) = (self.size.0 as i32, self.size.1 as i32);
        // The Mat borrows immutable image storage for calc's read-only inputs.
        // Its row step includes padding, while its dimensions exclude padding.
        let view = |image: &image::GrayImage| unsafe {
            Mat::new_size_with_data_unsafe(
                Size::new(w, h),
                CV_8UC1,
                image.as_raw().as_ptr() as *mut std::ffi::c_void,
                image.width() as usize,
            )
        };
        let a = view(&self.img)?;
        let b = view(&next.img)?;
        // DIS requires contiguous input. Copy only when decoder row padding
        // makes the active image view non-contiguous.
        let a = if a.is_continuous() { a } else { a.try_clone()? };
        let b = if b.is_continuous() { b } else { b.try_clone()? };
        let mut algorithm =
            opencv::video::DISOpticalFlow::create(opencv::video::DISOpticalFlow_PRESET_FAST)?;
        let mut forward = Mat::default();
        let mut reverse = Mat::default();
        algorithm.calc(&a, &b, &mut forward)?;
        if self.context.is_cancelled() {
            return Ok(None);
        }
        algorithm.calc(&b, &a, &mut reverse)?;
        let mut source = Vec::new();
        let mut target = Vec::new();
        let step = (w as usize / 24).max(1);
        for y in (0..h).step_by(step) {
            for x in (0..w).step_by(step) {
                let flow = forward.at_2d::<Vec2f>(y, x)?;
                source.push((x as f32, y as f32));
                target.push((x as f32 + flow[0], y as f32 + flow[1]));
            }
        }
        let backward = |p: (f32, f32)| -> Option<(f32, f32)> {
            if !p.0.is_finite()
                || !p.1.is_finite()
                || p.0 < 0.0
                || p.1 < 0.0
                || p.0 + 1.0 >= w as f32
                || p.1 + 1.0 >= h as f32
            {
                return None;
            }
            let (x, y) = (p.0 as i32, p.1 as i32);
            let (ax, ay) = (p.0 - x as f32, p.1 - y as f32);
            let a = reverse.at_2d::<Vec2f>(y, x).ok()?;
            let b = reverse.at_2d::<Vec2f>(y, x + 1).ok()?;
            let c = reverse.at_2d::<Vec2f>(y + 1, x).ok()?;
            let d = reverse.at_2d::<Vec2f>(y + 1, x + 1).ok()?;
            let sample = |i: usize| {
                (a[i] * (1.0 - ax) + b[i] * ax) * (1.0 - ay) + (c[i] * (1.0 - ax) + d[i] * ax) * ay
            };
            Some((p.0 + sample(0), p.1 + sample(1)))
        };
        Ok(super::quality::filter_matches(
            &self.img,
            &next.img,
            self.size,
            Some((source, target)),
            backward,
        ))
    }
}

impl OpticalFlowTrait for OFOpenCVDis {
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn features(&self) -> &Vec<(f32, f32)> {
        &self.features
    }

    fn optical_flow_to(&self, to: &OpticalFlowMethod) -> OpticalFlowPair {
        if self.context.is_cancelled() {
            return None;
        }
        let OpticalFlowMethod::OFOpenCVDis(next) = to else {
            return None;
        };
        // Pose estimation and the chart may request the same pair concurrently.
        let mut cache = self.matches.lock();
        if let Some(matched) = cache.get(&next.timestamp_us) {
            return matched.clone();
        }
        let valid = |image: &image::GrayImage| {
            self.size.0 >= 12
                && self.size.1 >= 12
                && self.size.0 <= image.width()
                && self.size.1 <= image.height()
        };
        if self.size != next.size || !valid(&self.img) || !valid(&next.img) {
            return None;
        }
        #[cfg(feature = "use-opencv")]
        let result = match self.track(next) {
            Ok(result) => result,
            Err(error) => {
                self.context.fail(format!("OpenCV DIS: {error}"));
                None
            }
        };
        #[cfg(not(feature = "use-opencv"))]
        let result = None;
        cache.insert(next.timestamp_us, result.clone());
        self.used.fetch_add(1, Ordering::Relaxed);
        next.used.fetch_add(1, Ordering::Relaxed);
        result
    }

    fn can_cleanup(&self) -> bool {
        self.context.may_release_images() && self.used.load(Ordering::Relaxed) >= 2
    }
    fn cleanup(&mut self) {
        self.img = Arc::new(image::GrayImage::default());
    }
}

#[cfg(all(test, feature = "use-opencv"))]
mod tests {
    use super::*;

    #[test]
    fn padded_decoder_rows_do_not_enter_dis_or_photometric_checks() {
        let active = (130, 74);
        let a = image::GrayImage::from_fn(144, active.1, |x, y| {
            image::Luma([if x < active.0 {
                ((x * 17 + y * 11 + x * y) % 160 + 40) as u8
            } else {
                0
            }])
        });
        let b = image::GrayImage::from_fn(144, active.1, |x, y| {
            image::Luma([if (4..active.0).contains(&x) {
                a.get_pixel(x - 4, y).0[0]
            } else {
                255
            }])
        });
        let first = OFOpenCVDis::detect_features(0, Arc::new(a), active.0, active.1);
        let next = OpticalFlowMethod::OFOpenCVDis(OFOpenCVDis::detect_features(
            33_333,
            Arc::new(b),
            active.0,
            active.1,
        ));
        let (a, b) = first
            .optical_flow_to(&next)
            .expect("Padded frames should be trackable");
        assert!(a.len() >= 30);
        let mut errors: Vec<_> = a
            .iter()
            .zip(&b)
            .map(|(a, b)| ((b.0 - a.0) - 4.0).hypot(b.1 - a.1))
            .collect();
        errors.sort_by(f32::total_cmp);
        assert!(errors[errors.len() / 2] < 0.5);
        assert!(
            a.iter()
                .chain(&b)
                .all(|&(x, y)| x < active.0 as f32 && y < active.1 as f32)
        );
        assert_eq!(first.optical_flow_to(&next).unwrap().0, a);
    }
}
