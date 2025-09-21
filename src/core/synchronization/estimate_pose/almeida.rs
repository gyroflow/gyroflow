// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use super::super::OpticalFlowPair;
use super::{ EstimateRelativePoseTrait, RelativePose };

use nalgebra as na;
use crate::stabilization::*;
const EPS: f32 = 0.001 * std::f32::consts::PI / 180.0;
const ALPHA: f32 = 0.5;

#[derive(Default, Clone)]
pub struct PoseAlmeida {
    cam: Camera
}

impl EstimateRelativePoseTrait for PoseAlmeida {
    fn init(&mut self, params: &ComputeParams) {
        self.cam = Camera { compute_params: params.clone() };
        self.cam.compute_params.lens_correction_amount = 0.0;
    }
    fn estimate_relative_pose(&self, pairs: &OpticalFlowPair, size: (u32, u32), params: &ComputeParams, timestamp_us: i64, next_timestamp_us: i64) -> Option<RelativePose> {
        let (pts1, pts2) = pairs.as_ref()?;

        let (w, h) = (size.0 as f32, size.1 as f32);

        let vectors: Vec<MotionEntry> = pts1.into_iter().zip(pts2.into_iter()).map(|(a, b)| {
            (na::Point2::<f32>::new(a.0 / w, a.1 / h), na::Vector2::<f32>::new((b.0 - a.0) / w, (b.1 - a.1) / h))
        }).collect();

        let timestamp_ms = timestamp_us as f64 / 1000.0;

        let rot = AlmeidaEstimator::default().estimate(&vectors, &self.cam, timestamp_ms);
        let rotation = na::convert(na::Rotation3::from(rot.inverse()));
        Some(RelativePose { rotation, translation_dir_cam: None, inlier_ratio: None, median_epi_err: None })
    }
}

#[derive(Default, Clone)]
pub struct Camera {
    compute_params: ComputeParams
}

impl Camera {
    pub fn point_angle(&self, p: na::Point2<f32>, timestamp_ms: f64) -> na::Vector2<f32> {
        let (intrinsics, _, _, _, _, _) = FrameTransform::get_lens_data_at_timestamp(&self.compute_params, timestamp_ms, false);

        // Center the point.
        let p = p - na::Vector2::new(intrinsics[(0, 2)] as f32, intrinsics[(1, 2)] as f32);

        let tan = p
            .coords
            .component_div(&na::matrix![intrinsics[(0, 0)] as f32; intrinsics[(1, 1)] as f32]);

        na::matrix![tan.x.atan(); tan.y.atan()]
    }
    fn delta(&self, coords: na::Point2<f32>, rotation: na::Matrix4<f32>, timestamp_ms: f64) -> na::Vector2<f32> {
        let vw = self.compute_params.width as f32;
        let vh = self.compute_params.height as f32;
        let (camera_matrix, distortion_coeffs, _, _, _, _) = FrameTransform::get_lens_data_at_timestamp(&self.compute_params, timestamp_ms, false);

        let rot = na::Matrix3::<f32>::from(rotation.fixed_view::<3, 3>(0, 0));

        let pt = undistort_points(&[(coords[0] * vw, coords[1] * vh)], camera_matrix, &distortion_coeffs, na::convert(rot), Some(camera_matrix), None, &self.compute_params, 1.0, timestamp_ms, None, None)[0];

        na::Point2::new(pt.0 / vw, pt.1 / vh) - coords
    }
    fn roll(&self, coords: na::Point2<f32>, eps: f32, timestamp_ms: f64) -> na::Vector2<f32> {
        self.delta(coords, na::Matrix4::from_euler_angles(0.0, eps, 0.0), timestamp_ms)
    }
    fn pitch(&self, coords: na::Point2<f32>, eps: f32, timestamp_ms: f64) -> na::Vector2<f32> {
        self.delta(coords, na::Matrix4::from_euler_angles(eps, 0.0, 0.0), timestamp_ms)
    }
    fn yaw(&self, coords: na::Point2<f32>, eps: f32, timestamp_ms: f64) -> na::Vector2<f32> {
        self.delta(coords, na::Matrix4::from_euler_angles(0.0, 0.0, -eps), timestamp_ms)
    }
}

pub type MotionEntry = (na::Point2<f32>, na::Vector2<f32>);

/// Copied from https://github.com/h33p/ofps/blob/main/almeida-estimator/src/lib.rs
/// Motion estimator built on a research paper titled "Robust Estimation
/// of Camera Motion Using Optical Flow Models".
///
/// Authors:
///
/// Jurandy Almeida, Rodrigo Minetto, Tiago A. Almeida, Ricardo da S. Torres, and Neucimar J. Leite.
///
/// This estimator only produces rotational output, and no translation.
pub struct AlmeidaEstimator {
    /// True if ransac is used. False to perform least
    /// squares minimisation solution.
    use_ransac: bool,
    /// Number of iterations for ransac.
    num_iters: usize,
    /// Target angle error in degrees for the sample to be considered as inlier.
    inlier_angle: f32,
    /// Number of samples per each ransac iteration.
    ransac_samples: usize,
}

impl Default for AlmeidaEstimator {
    fn default() -> Self {
        Self {
            use_ransac: true,
            num_iters: 200,
            inlier_angle: 0.05,
            ransac_samples: 1000,
        }
    }
}

impl AlmeidaEstimator {
    pub fn estimate(&mut self, motion_vectors: &[MotionEntry], camera: &Camera, timestamp_ms: f64) -> na::UnitQuaternion<f32> {
        if self.use_ransac {
            solve_ypr_ransac(motion_vectors, camera, timestamp_ms, self.num_iters, self.inlier_angle, self.ransac_samples )
        } else {
            solve_ypr_given(motion_vectors, camera, timestamp_ms)
        }
    }
}

fn solve_ypr_given(input: &[MotionEntry], camera: &Camera, timestamp_ms: f64) -> na::UnitQuaternion<f32> {
    let dot = |a: usize, b: usize| move |vecs: &[na::Vector2<f32>]| vecs[a].dot(&vecs[b]);

    fn dot_map<T: Fn(&[na::Vector2<f32>]) -> f32>(
        motion: &[(na::Point2<f32>, [na::Vector2<f32>; 4])],
    ) -> (impl Fn(T) -> f32 + '_) {
        move |dot| motion.iter().map(|(_, v)| dot(v)).sum::<f32>()
    }

    let limit = (15.0 / ALPHA).ceil() as usize;

    let mut rotation = na::UnitQuaternion::identity();

    // Iterative optimisation loop.
    for i in 0..limit {
        let alpha = if i == limit - 1 { 1.0 } else { ALPHA };

        let rotm = rotation.to_homogeneous();

        let motion = input
            .iter()
            .copied()
            .map(|(pos, motion)| {
                let delta = camera.delta(pos, rotm, timestamp_ms);
                (
                    pos + delta,
                    [
                        motion - delta,
                        camera.roll(pos, EPS, timestamp_ms),
                        camera.pitch(pos, EPS, timestamp_ms),
                        camera.yaw(pos, EPS, timestamp_ms),
                    ],
                )
            })
            .collect::<Vec<_>>();

        let a = na::Matrix3::from_iterator([
                dot(1, 1), dot(1, 2), dot(1, 3),
                dot(2, 1), dot(2, 2), dot(2, 3),
                dot(3, 1), dot(3, 2), dot(3, 3)
            ].iter().map(dot_map(&motion))
        );

        let b = na::Vector3::from_iterator(
            [dot(1, 0), dot(2, 0), dot(3, 0)]
                .iter()
                .map(dot_map(&motion)),
        );

        let decomp = a.lu();

        let model = decomp.solve(&b).unwrap_or_default();

        let model = model * EPS * alpha;

        // Apply rotation in YRP order, as it is more correct.

        let roll = na::UnitQuaternion::from_euler_angles(0.0, model.x, 0.0);
        let pitch = na::UnitQuaternion::from_euler_angles(model.y, 0.0, 0.0);
        let yaw = na::UnitQuaternion::from_euler_angles(0.0, 0.0, -model.z);

        let rot = pitch * roll * yaw;

        rotation *= rot;
    }

    // We estimated how points rotate, not how the camera rotates - take inverse.
    rotation.inverse()
}

fn solve_ypr_ransac(field: &[MotionEntry], camera: &Camera, timestamp_ms: f64, num_iters: usize, target_delta: f32, num_samples: usize) -> na::UnitQuaternion<f32> {
    use rand::prelude::*;
    let mut best_inliers = vec![];
    let target_delta = target_delta.to_radians();

    let rng = &mut rand::rng();

    for _ in 0..num_iters {
        let samples = field.choose_multiple(rng, 3).copied().collect::<Vec<_>>();

        let fit = solve_ypr_given(&samples, camera, timestamp_ms);

        let motion = field
            .choose_multiple(rng, num_samples)
            .copied()
            .collect::<Vec<_>>();

        let mat = fit.inverse().to_homogeneous();

        let inliers = motion
            .iter()
            .copied()
            .map(|(pos, vec)| {
                let delta = camera.delta(pos, mat, timestamp_ms);
                ((pos, vec), (pos + delta, vec - delta))
            })
            .filter(|(_, (sample, vec))| {
                let angle = camera.point_angle(*sample, timestamp_ms);
                let cosang = na::matrix![angle.x.cos(); angle.y.cos()];
                vec.component_mul(&cosang).magnitude_squared() <= target_delta * target_delta
            })
            .map(|(a, _)| a)
            .collect::<Vec<_>>();

        if inliers.len() > best_inliers.len() {
            best_inliers = inliers;
        }
    }

    if best_inliers.len() >= 3 {
        solve_ypr_given(&best_inliers, camera, timestamp_ms)
    } else {
        Default::default()
    }
}
