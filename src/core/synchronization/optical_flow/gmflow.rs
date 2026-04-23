// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

use super::super::OpticalFlowPair;
use super::{ OpticalFlowTrait, OpticalFlowMethod };

use std::sync::Arc;

#[derive(Clone)]
pub struct OFGmflow {
    features: Vec<(f32, f32)>,
    img: Arc<image::GrayImage>,
    timestamp_us: i64,
    size: (u32, u32),
}

impl OFGmflow {
    pub fn detect_features(timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32) -> Self {
        Self {
            features: Vec::new(),
            img,
            timestamp_us,
            size: (width, height),
        }
    }
}

impl OpticalFlowTrait for OFGmflow {
    fn size(&self) -> (u32, u32) { self.size }
    fn features(&self) -> &Vec<(f32, f32)> { &self.features }

    fn optical_flow_to(&self, _to: &OpticalFlowMethod) -> OpticalFlowPair {
        // TODO: AI-based optical flow via burn::gmflow (task: #45)
        // Requires feature = \"use-burn\" and pre-downloaded gmflow weights (.bpk)
        None
    }

    fn can_cleanup(&self) -> bool { true }
    fn cleanup(&mut self) { self.img = Arc::new(image::GrayImage::default()); }
}
