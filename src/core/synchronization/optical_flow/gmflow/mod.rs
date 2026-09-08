// SPDX-License-Identifier: GPL-3.0-or-later

use super::{OpticalFlowContext, OpticalFlowMethod, OpticalFlowPair, OpticalFlowTrait};
use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

// The non-AI build keeps a stub variant, matching the optional OpenCV backends.
#[cfg_attr(not(feature = "use-gmflow"), allow(dead_code))]
mod geometry;
#[cfg(feature = "use-gmflow")]
mod inference;

/// The variant is always declared because enum_delegate does not propagate cfg
/// attributes to generated match arms. Method 3 is only selectable in AI builds.
#[derive(Clone)]
pub struct OFGmflow {
    img: Arc<image::GrayImage>,
    size: (u32, u32),
    timestamp_us: i64,
    features: Vec<(f32, f32)>,
    matches: Arc<Mutex<BTreeMap<i64, OpticalFlowPair>>>,
    used: Arc<AtomicU32>,
    context: Arc<OpticalFlowContext>,
}

#[cfg(all(test, not(feature = "use-gmflow")))]
mod tests {
    use super::*;

    #[test]
    fn unavailable_method_reports_an_error_before_processing_frames() {
        let context = Arc::new(OpticalFlowContext::default());
        OFGmflow::detect_features(
            0,
            Arc::new(image::GrayImage::new(16, 16)),
            16,
            16,
            context.clone(),
        );
        assert!(context.is_cancelled());
        assert!(context.error().unwrap().contains("does not include GMFlow"));
    }
}

impl OFGmflow {
    pub fn detect_features(
        timestamp_us: i64,
        img: Arc<image::GrayImage>,
        width: u32,
        height: u32,
        context: Arc<OpticalFlowContext>,
    ) -> Self {
        #[cfg(not(feature = "use-gmflow"))]
        context
            .fail("This build does not include GMFlow. Select another optical flow method.".into());
        let size = (width, height);
        let features = geometry::Layout::new(size)
            .filter(|_| width <= img.width() && height <= img.height())
            .map(|layout| layout.features(&img))
            .unwrap_or_default();
        Self {
            img,
            size,
            timestamp_us,
            features,
            matches: Default::default(),
            used: Default::default(),
            context,
        }
    }
}

impl OpticalFlowTrait for OFGmflow {
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
        let OpticalFlowMethod::OFGmflow(next) = to else {
            return None;
        };
        // Keep the per-frame lock across inference to coalesce simultaneous
        // requests for the same pair from pose estimation and chart drawing.
        let mut cache = self.matches.lock();
        if let Some(cached) = cache.get(&next.timestamp_us) {
            return cached.clone();
        }
        if self.size != next.size
            || self.img.is_empty()
            || next.img.is_empty()
            || self.features.len() < geometry::MIN_MATCHES
        {
            return None;
        }
        #[cfg(feature = "use-gmflow")]
        let result = match inference::flow(
            self.img.clone(),
            next.img.clone(),
            self.size,
            self.context.clone(),
        ) {
            Ok(Some(flow)) => {
                match inference::flow(
                    next.img.clone(),
                    self.img.clone(),
                    self.size,
                    self.context.clone(),
                ) {
                    Ok(Some(reverse)) => geometry::Layout::new(self.size).and_then(|layout| {
                        super::quality::filter_matches(
                            &self.img,
                            &next.img,
                            self.size,
                            layout.correspondences(&flow, &self.features),
                            |p| layout.destination(&reverse, p),
                        )
                    }),
                    Ok(None) => None,
                    Err(error) => {
                        self.context.fail(error);
                        None
                    }
                }
            }
            Ok(None) => None,
            Err(error) => {
                self.context.fail(error);
                None
            }
        };
        #[cfg(not(feature = "use-gmflow"))]
        let result = {
            self.context.fail(
                "This build does not include GMFlow. Select another optical flow method.".into(),
            );
            None
        };
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
