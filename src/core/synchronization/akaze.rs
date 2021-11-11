use akaze::Akaze;
use arrsac::Arrsac;
use bitarray::{BitArray, Hamming};
use image::EncodableLayout;
use nalgebra::{Vector2, Rotation3};
use cv_core::{CameraModel, FeatureMatch, Pose, sample_consensus::Consensus};
use rand_xoshiro::Xoshiro256PlusPlus;
use rand_xoshiro::rand_core::SeedableRng;
use std::vec::Vec;

use space::{Knn, LinearKnn};

const LOWES_RATIO: f32 = 0.5;

lazy_static::lazy_static! {
    static ref THREAD_POOL: rayon::ThreadPool = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
}

pub type Descriptor = BitArray<64>;
pub type Match = FeatureMatch;
pub type DetectedFeatures = (Vec<akaze::KeyPoint>, Vec<Descriptor>);

#[derive(Default, Clone)]
pub struct ItemAkaze {
    features: DetectedFeatures
}

impl ItemAkaze {
    pub fn detect_features(frame: usize, img: image::GrayImage) -> Self {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(img.as_bytes());
        let frame_path = format!("cache/{}-{}.bin", frame, hasher.finalize());

        let features = if let Ok(bytes) = std::fs::read(&frame_path) {
            deserialize_features(&bytes)
        } else {
            let features = Akaze::new(0.0007).extract(&image::DynamicImage::ImageLuma8(img));
            let encoded: Vec<u8> = serialize_features(&features);
            THREAD_POOL.spawn(move || {
                let _ = std::fs::create_dir("cache");
                let _ = std::fs::write(frame_path, encoded);
            });

            features
        };

        Self { features }
    }
    pub fn get_features_count(&self) -> usize {
        self.features.0.len()
    }
    pub fn get_feature_at_index(&self, i: usize) -> (f32, f32) {
        self.features.0[i].point
    }

    pub fn estimate_pose(&mut self, next: &mut Self, focal: Vector2<f64>, principal: Vector2<f64>) -> Option<Rotation3<f64>> {        
        let a1 = &self.features;
        let a2 = &next.features;

        let intrinsics = cv_pinhole::CameraIntrinsics {
            focals: cv_core::nalgebra::Vector2::<f64>::new(focal[0], focal[1]),
            principal_point: cv_core::nalgebra::Point2::new(principal[0], principal[1]),
            skew: 0.0,
        };

        let matches: Vec<Match> = Self::match_descriptors(&a1.1, &a2.1).into_iter()
            .map(|(i1, i2)| FeatureMatch(intrinsics.calibrate(a1.0[i1]), intrinsics.calibrate(a2.0[i2])))
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
                // return Some(out.isometry().rotation); TODO uncomment once Rust-CV updates to nalgebra 0.29
                return Some(nalgebra::Rotation3::from_matrix_unchecked(nalgebra::SMatrix::<f64, 3, 3>::from(out.isometry().rotation.into_inner().data.0)));
                /*let rotations = cv_pinhole::EssentialMatrix::from(out).possible_rotations(1e-12, 1000).unwrap();
                if rotations[0].angle() < rotations[1].angle() {
                    Some(rotations[0])
                } else {
                    Some(rotations[1])
                }*/
            }
        }
        println!("couldn't find model");
        None
    }

    fn match_descriptors(ds1: &[Descriptor], ds2: &[Descriptor]) -> Vec<(usize, usize)> {
        if ds1.len() < 2 || ds2.len() < 2 { return Vec::new() }
        let two_neighbors = ds1.iter().map(|d1| LinearKnn { metric: Hamming, iter: ds2.iter() }.knn(d1, 2)).enumerate();
        let satisfies_lowes_ratio = two_neighbors.filter(|(_, neighbors)| {
            (neighbors[0].distance as f32) < neighbors[1].distance as f32 * LOWES_RATIO
        });
        satisfies_lowes_ratio.map(|(ix1, neighbors)| (ix1, neighbors[0].index)).collect()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
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
fn deserialize_features(x: &Vec<u8>) -> DetectedFeatures {
    let val: SerializedFeatures = bincode::deserialize(&x).unwrap();
    (
        val.0.into_iter().map(SerializedKeypoint::into).collect(), 
        val.1.into_iter().map(|x| { let mut a = [0u8; 64]; a.copy_from_slice(&x); BitArray::<64>::new(a) }).collect(), 
    )
}
