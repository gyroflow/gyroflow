// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Adapted from OpenCV: https://github.com/opencv/opencv/blob/c3cbd302cbfbaefdef9a011b2615f8d8f58556dd/modules/calib3d/src/undistort.dispatch.cpp#L491-L538

use crate::stabilization::KernelParams;
use super::{ Real, c };

#[derive(Default, Clone)]
pub struct OpenCVStandard { }

impl OpenCVStandard {
    pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        let (mut x, mut y) = point;
        let (x0, y0) = point;

        // compensate distortion iteratively
        for _ in 0..20 {
            let r2 = x * x + y * y;
            let icdist = (1.0 + ((params.k[7] * r2 + params.k[6]) * r2 + params.k[5]) * r2) / (1.0 + ((params.k[4] * r2 + params.k[1]) * r2 + params.k[0]) * r2);
            // Past where the radial curve folds: no ray (it's met for every pixel out there, so no log either)
            if icdist < 0.0 {
                return None;
            }
            let delta_x = 2.0 * params.k[2] * x * y + params.k[3] * (r2 + 2.0 * x * x) + params.k[8]  * r2 + params.k[9]  * r2 * r2;
            let delta_y = params.k[2] * (r2 + 2.0 * y * y) + 2.0 * params.k[3] * x * y + params.k[10] * r2 + params.k[11] * r2 * r2;
            x = (x0 - delta_x) * icdist;
            y = (y0 - delta_y) * icdist;
        }

        // The solve runs in the z=1 plane; the pipeline carries rays as angle vectors
        let r = (x * x + y * y).sqrt();
        if r < 1e-12 { return Some((x, y)); }
        let s = r.atan() / r;
        Some((x * s, y * s))
    }

    pub fn distort_point(&self, x: f32, y: f32, z: f32, params: &KernelParams) -> (f32, f32) {
        Self::distort(x, y, z, &params.k)
    }

    /// The projection itself, in either precision (see `Real`). `k`: k1, k2, p1, p2, k3, k4, k5, k6, s1, s2, s3, s4
    pub fn distort<T: Real>(x: T, y: T, z: T, k: &[T]) -> (T, T) {
        // A rectilinear model has no image of a ray at or past 90°: send it far outside the frame
        let (x, y) = if z > c(1e-9) { (x / z, y / z) } else { (x * c(1e9), y * c(1e9)) };
        let (one, two) = (c::<T>(1.0), c::<T>(2.0));
        let r2 = x * x + y * y;
        let r4 = r2 * r2;
        let r6 = r4 * r2;
        let a1 = two * x * y;
        let a2 = r2 + two * x * x;
        let a3 = r2 + two * y * y;
        let cdist = one + k[0] * r2 + k[1] * r4 + k[4] * r6;
        let icdist2 = one / (one + k[5] * r2 + k[6] * r4 + k[7] * r6);
        let xd0 = x * cdist * icdist2 + k[2] * a1 + k[3] * a2 + k[8]  * r2 + k[9]  * r4;
        let yd0 = y * cdist * icdist2 + k[2] * a3 + k[3] * a1 + k[10] * r2 + k[11] * r4;

        (xd0, yd0)
    }
    pub fn adjust_lens_profile(&self, _profile: &mut crate::LensProfile) { }

    pub fn distortion_derivative(&self, theta: f64, k: &[f64]) -> Option<f64> {
        if k.len() < 8 { return None; }
        let r2 = theta * theta;
        Some(
            (1.0 + ((k[7] * r2 + k[6]) * r2 + k[5]) * r2) / (1.0 + ((k[4] * r2 + k[1]) * r2 + k[0]) * r2)
        )
    }

    pub fn id() -> &'static str { "opencv_standard" }
    pub fn name() -> &'static str { "OpenCV Standard" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("opencv_standard.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("opencv_standard.wgsl") }
}
