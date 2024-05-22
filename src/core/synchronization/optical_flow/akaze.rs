// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::super::OpticalFlowPair;
use super::{ OpticalFlowTrait, OpticalFlowMethod };

use akaze::Akaze;
use bitarray::{ BitArray, Hamming };
use std::{ vec::Vec, sync::Arc };

use space::{ Knn, LinearKnn };

const LOWES_RATIO: f32 = 0.5;

pub type Descriptor = BitArray<64>;

#[derive(Clone)]
pub struct OFAkaze {
    features: Vec<(f32, f32)>,
    descriptors: Vec<Descriptor>,
    img_size: (u32, u32)
}

impl OFAkaze {
    pub fn detect_features(_timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32) -> Self {
        let mut akz = Akaze::new(0.0007);
        akz.maximum_features = 200;
        let img_size = (width, height);
        let (points, descriptors) = akz.extract(&image::DynamicImage::ImageLuma8(Arc::try_unwrap(img).unwrap()));

        Self {
            features: points.into_iter().map(|x| x.point).collect(),
            descriptors,
            img_size
        }
    }
    pub fn match_descriptors(ds1: &[Descriptor], ds2: &[Descriptor]) -> Vec<(usize, usize)> {
        if ds1.len() < 2 || ds2.len() < 2 { return Vec::new() }
        let two_neighbors = ds1.iter().map(|d1| LinearKnn { metric: Hamming, iter: ds2.iter() }.knn(d1, 2)).enumerate();
        let satisfies_lowes_ratio = two_neighbors.filter(|(_, neighbors)| {
            (neighbors[0].distance as f32) < neighbors[1].distance as f32 * LOWES_RATIO
        });
        satisfies_lowes_ratio.map(|(ix1, neighbors)| (ix1, neighbors[0].index)).collect()
    }
}

impl OpticalFlowTrait for OFAkaze {
    fn optical_flow_to(&self, to: &OpticalFlowMethod) -> OpticalFlowPair {
        if let OpticalFlowMethod::OFAkaze(to) = to {
            return Some(Self::match_descriptors(&self.descriptors, &to.descriptors)
                .into_iter()
                .map(|(i1, i2)| {
                    (self.features[i1].clone(), to.features[i2].clone())
                }).unzip());
        }
        None
    }

    fn can_cleanup(&self) -> bool { true }
    fn features(&self) -> &Vec<(f32, f32)> { &self.features }
    fn size(&self) -> (u32, u32) { self.img_size }
    fn cleanup(&mut self) { }
}
