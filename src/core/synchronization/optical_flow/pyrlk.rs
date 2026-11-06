// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2026 Adrian <adrian.eddy at gmail>

//! Sparse optical flow by pyramidal Lucas-Kanade: the corners of each frame, followed into the frames after it. The
//! optical analysis' KLT (`optical_motion::PairKlt`): on the GPU, and with OpenCV without one

use super::super::{ OpticalFlowPair, OpticalFlowPoints };
use super::{ OpticalFlowTrait, OpticalFlowMethod };
use crate::synchronization::optical_motion::PairKlt;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{ AtomicU32, Ordering::SeqCst };
use parking_lot::RwLock;

/// The corners of a frame: at most `OFPyrLK::MAX_FEATURES`, above this much of the strongest one, at least this far
/// apart (px)
const QUALITY: f64 = 0.01;
const MIN_DISTANCE: f64 = 10.0;
/// How far a point tracked into the next frame and back may end up from where it started, px: further, it was lost
/// (occluded, a repeated texture). 2-6% of the points, which made most of rs-sync's cost: on a7S III footage 1143 at a
/// sync point without the check, 109 with it, the offsets the same within 0.1 ms. A precise point comes back within a
/// tenth of a pixel, but blur and noise take more than that, and the sync wants as many points as it can get
const FB_MAX: f32 = 1.0;
/// A pair with fewer points followed than this is none: not kept, and its frames not used up
const MIN_POINTS: usize = 10;

#[derive(Clone)]
pub struct OFPyrLK {
    features: Vec<(f32, f32)>,
    img: Arc<image::GrayImage>,
    matched_points: Arc<RwLock<BTreeMap<i64, (OpticalFlowPoints, OpticalFlowPoints)>>>,
    timestamp_us: i64,
    size: (i32, i32),
    used: Arc<AtomicU32>,
    /// Where it's tracked, one for all frames of a synchronization: let go of with the image
    klt: Option<Arc<PairKlt>>,
}

impl OFPyrLK {
    pub const MAX_FEATURES: usize = 200;

    pub fn detect_features(timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32, klt: Option<Arc<PairKlt>>) -> Self {
        // The image's rows are its width apart, which can be more than `width`
        let features = match &klt {
            Some(klt) => klt.corners(timestamp_us, img.as_raw(), (width, height), img.width() as usize, Self::MAX_FEATURES, QUALITY, MIN_DISTANCE)
                .unwrap_or_else(|e| { log::debug!("PyrLK: no corners at {timestamp_us} us: {e}"); Vec::new() }),
            None => Vec::new(),
        };
        Self {
            features: features.into_iter().map(|p| (p[0], p[1])).collect(),
            size: (width as i32, height as i32),
            img,
            timestamp_us,
            matched_points: Default::default(),
            used: Default::default(),
            klt,
        }
    }
}

impl OpticalFlowTrait for OFPyrLK {
    fn size(&self) -> (u32, u32) {
        (self.size.0 as u32, self.size.1 as u32)
    }
    fn features(&self) -> &Vec<(f32, f32)> { &self.features }

    fn optical_flow_to(&self, to: &OpticalFlowMethod) -> OpticalFlowPair {
        let OpticalFlowMethod::OFPyrLK(next) = to else { return None };
        // Before the images: those are let go of once both pairs of a frame are done
        if let Some(matched) = self.matched_points.read().get(&next.timestamp_us) {
            return Some(matched.clone());
        }
        let (w, h) = self.size;
        let klt = self.klt.as_ref()?;
        if self.img.is_empty() || next.img.is_empty() || self.features.is_empty() || next.size != self.size || next.img.width() != self.img.width() { return None; }

        let points: Vec<[f32; 2]> = self.features.iter().map(|p| [p.0, p.1]).collect();
        let moved = klt.track((self.timestamp_us, self.img.as_raw()), (next.timestamp_us, next.img.as_raw()), (w as u32, h as u32), self.img.width() as usize, &points, FB_MAX)
            .map_err(|e| log::debug!("PyrLK: nothing tracked from {} us to {} us: {e}", self.timestamp_us, next.timestamp_us))
            .ok()?;
        let inside = |p: [f32; 2]| p[0] >= 0.0 && p[0] < w as f32 && p[1] >= 0.0 && p[1] < h as f32;
        let (pts1, pts2): (OpticalFlowPoints, OpticalFlowPoints) = points.iter().zip(moved)
            .filter_map(|(&a, b)| b.filter(|&b| inside(a) && inside(b)).map(|b| ((a[0], a[1]), (b[0], b[1]))))
            .unzip();
        if pts1.len() < MIN_POINTS { return None; }
        // Used up once, also when two pose estimations got to the same pair at once: counted twice, a frame would let go
        // of its image before its other pair
        if self.matched_points.write().insert(next.timestamp_us, (pts1.clone(), pts2.clone())).is_none() {
            self.used.fetch_add(1, SeqCst);
            next.used.fetch_add(1, SeqCst);
        }
        Some((pts1, pts2))
    }
    fn can_cleanup(&self) -> bool {
        self.used.load(SeqCst) == 2
    }
    fn cleanup(&mut self) {
        self.img = Arc::new(image::GrayImage::default());
        self.klt = None;
    }
}
