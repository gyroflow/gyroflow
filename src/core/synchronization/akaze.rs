// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use akaze::Akaze;
use arrsac::Arrsac;
use bitarray::{ BitArray, Hamming };
use nalgebra::Rotation3;
use cv_core::{ FeatureMatch, Pose, sample_consensus::Consensus };
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;
use std::{ vec::Vec, sync::Arc };
use crate::stabilization::ComputeParams;
use super::{ EstimatorItem, EstimatorItemInterface, OpticalFlowPair };

use space::{Knn, LinearKnn};

const LOWES_RATIO: f32 = 0.5;

// lazy_static::lazy_static! {
//     static ref THREAD_POOL: rayon::ThreadPool = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
// }

pub type Descriptor = BitArray<64>;
pub type Match = FeatureMatch;

#[derive(Default, Clone)]
pub struct ItemAkaze {
    features: Vec<(f32, f32)>,
    descriptors: Vec<Descriptor>,
    img_size: (u32, u32)
}

// TODO: add caching checkbox to the UI

impl EstimatorItemInterface for ItemAkaze {
    fn get_features(&self) -> &Vec<(f32, f32)> {
        &self.features
    }

    fn estimate_pose(&self, next: &EstimatorItem, params: &ComputeParams, timestamp_us: i64, next_timestamp_us: i64) -> Option<Rotation3<f64>> {
        if let EstimatorItem::ItemAkaze(next) = next {
            use cv_core::nalgebra::{ UnitVector3, Point2 };

            let pts1 = &self.features;
            let pts2 = &next.features;

            let pts1 = crate::stabilization::undistort_points_for_optical_flow(&pts1, timestamp_us, params, self.img_size);
            let pts2 = crate::stabilization::undistort_points_for_optical_flow(&pts2, next_timestamp_us, params, self.img_size);

            let matches: Vec<Match> = Self::match_descriptors(&self.descriptors, &next.descriptors).into_iter()
                .map(|(i1, i2)| {
                    FeatureMatch(
                        UnitVector3::new_normalize(Point2::new(pts1[i1].0 as f64, pts1[i1].1 as f64).to_homogeneous()),
                        UnitVector3::new_normalize(Point2::new(pts2[i2].0 as f64, pts2[i2].1 as f64).to_homogeneous())
                    )
                })
                .collect();

            // Try different thresholds for best results
            let thresholds = [1e-10, 1e-8, 1e-6];

            let mut arrsac = Arrsac::new(1e-10, Xoshiro256PlusPlus::seed_from_u64(0));
                //.initialization_hypotheses(2048)
                //.max_candidate_hypotheses(512);
            for threshold in thresholds {
                arrsac = arrsac.inlier_threshold(threshold);

                let eight_point = eight_point::EightPoint::new();
                if let Some(out) = arrsac.model(&eight_point, matches.iter().copied()) {
                    let rot = out.isometry().rotation;
                    return Some(nalgebra::Rotation3::from_matrix_unchecked(nalgebra::Matrix3::from_column_slice(rot.matrix().as_slice())));
                    /*let rotations = cv_pinhole::EssentialMatrix::from(out).possible_rotations(1e-12, 1000).unwrap();
                    if rotations[0].angle() < rotations[1].angle() {
                        Some(rotations[0])
                    } else {
                        Some(rotations[1])
                    }*/
                }
            }
        }
        ::log::warn!("couldn't find model");
        None
    }

    fn optical_flow_to(&self, to: &EstimatorItem) -> OpticalFlowPair {
        if let EstimatorItem::ItemAkaze(to) = to {
            return Some(Self::match_descriptors(&self.descriptors, &to.descriptors)
                .into_iter()
                .map(|(i1, i2)| {
                    (self.features[i1].clone(), to.features[i2].clone())
                }).unzip());
        }
        None
    }

    fn cleanup(&mut self) { }
}

impl ItemAkaze {
    pub fn match_descriptors(ds1: &[Descriptor], ds2: &[Descriptor]) -> Vec<(usize, usize)> {
        if ds1.len() < 2 || ds2.len() < 2 { return Vec::new() }
        let two_neighbors = ds1.iter().map(|d1| LinearKnn { metric: Hamming, iter: ds2.iter() }.knn(d1, 2)).enumerate();
        let satisfies_lowes_ratio = two_neighbors.filter(|(_, neighbors)| {
            (neighbors[0].distance as f32) < neighbors[1].distance as f32 * LOWES_RATIO
        });
        satisfies_lowes_ratio.map(|(ix1, neighbors)| (ix1, neighbors[0].index)).collect()
    }

    pub fn detect_features(_timestamp_us: i64, img: Arc<image::GrayImage>, width: u32, height: u32) -> Self {
        let mut akz = Akaze::new(0.0007);
        akz.maximum_features = 200;
        let img_size = (width, height);
        let (points, descriptors) = akz.extract(&image::DynamicImage::ImageLuma8(Arc::try_unwrap(img).unwrap()));

        /*let mut hasher = crc32fast::Hasher::new();
        hasher.update(img.as_bytes());
        let frame_path = format!("cache/{}-{}.bin", frame, hasher.finalize());

        let features = if let Ok(bytes) = std::fs::read(&frame_path) {
            deserialize_features(&bytes)
        } else {
            let mut akz = Akaze::new(0.0007);
            akz.maximum_features = 500;
            let features = akz.extract(&image::DynamicImage::ImageLuma8(img));
            let encoded: Vec<u8> = serialize_features(&features);
            THREAD_POOL.spawn(move || {
                let _ = std::fs::create_dir("cache");
                let _ = std::fs::write(frame_path, encoded);
            });

            features
        };*/

        Self {
            features: points.into_iter().map(|x| x.point).collect(),
            descriptors,
            img_size
        }
    }
}


/*#[derive(serde::Serialize, serde::Deserialize)]
struct SerializedKeypoint {
    pub point: (f32, f32),
    pub response: f32,
    pub size: f32,
    pub octave: usize,
    pub class_id: usize,
    pub angle: f32,
}
impl From<&akaze::KeyPoint> for SerializedKeypoint {
    fn from(v: &akaze::KeyPoint) -> Self {
        Self { point: v.point, response: v.response, size: v.size, octave: v.octave, class_id: v.class_id, angle: v.angle }
    }
}
impl Into<akaze::KeyPoint> for SerializedKeypoint {
    fn into(self) -> akaze::KeyPoint {
        akaze::KeyPoint { point: self.point, response: self.response, size: self.size, octave: self.octave, class_id: self.class_id, angle: self.angle }
    }
}
#[derive(serde::Serialize, serde::Deserialize)]
struct SerializedFeatures(Vec<SerializedKeypoint>, Vec<Vec<u8>>);

fn serialize_features(x: &DetectedFeatures) -> Vec<u8> {
    let out = SerializedFeatures(x.0.iter().map(SerializedKeypoint::from).collect(), x.1.iter().map(|v| v.bytes().to_vec()).collect());
    bincode::serialize(&out).unwrap()
}
fn deserialize_features(x: &[u8]) -> DetectedFeatures {
    let val: SerializedFeatures = bincode::deserialize(x).unwrap();
    (
        val.0.into_iter().map(SerializedKeypoint::into).collect(),
        val.1.into_iter().map(|x| { let mut a = [0u8; 64]; a.copy_from_slice(&x); BitArray::<64>::new(a) }).collect(),
    )
}*/
