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

        // That is a fixed point with the tangential and thin-prism terms held still, and it only contracts while they
        // and the slope of the radial curve are small: a wide lens with large ones is left hundreds of pixels off in
        // the corners. Newton on the whole map from there; where neither finds the point, the lens has no ray for it
        if !(x.is_finite() && y.is_finite()) { (x, y) = (x0, y0); }
        let k = &params.k;
        let r_d = (x0 * x0 + y0 * y0).sqrt();
        let tol = 1e-4 * (1.0 + r_d);
        let ((mut x, mut y), mut err) = super::polish_inverse(point, (x, y), |x, y| Self::map_with_jacobian(x, y, k), |_, _| true);
        if !(err <= tol) {
            // A steep radial curve throws the fixed point between two far-apart values, and Newton from either is
            // too slow to arrive: start again from the radial curve's own inverse, which for a lens with radial
            // terms only is already the answer
            if let Some(r) = Self::solve_radial(r_d, k).filter(|_| r_d > 1e-12) {
                let s = r / r_d;
                ((x, y), err) = super::polish_inverse(point, (x0 * s, y0 * s), |x, y| Self::map_with_jacobian(x, y, k), |_, _| true);
            }
        }
        if !(err <= tol) { return None; }

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

    /// The plane radius whose radial image is `r_d`, `r·cd(r²)/cd2(r²) = r_d`: bracketed from the axis outwards and
    /// bisected. `None` where the curve turns back, or runs out, before it reaches `r_d`
    fn solve_radial(r_d: f32, k: &[f32]) -> Option<f32> {
        let g = |r: f32| { let r2 = r * r; r * (1.0 + r2 * (k[0] + r2 * (k[1] + r2 * k[4]))) / (1.0 + r2 * (k[5] + r2 * (k[6] + r2 * k[7]))) };
        let (mut lo, mut hi) = (0.0f32, r_d);
        let mut g_hi = g(hi);
        let mut n = 0;
        while !(g_hi >= r_d) {
            let g_next = g(hi * 2.0);
            if !(g_next > g_hi) || n == 30 { return None; }
            lo = hi;
            hi *= 2.0;
            g_hi = g_next;
            n += 1;
        }
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if g(mid) < r_d { lo = mid; } else { hi = mid; }
        }
        Some(0.5 * (lo + hi))
    }

    /// The projection of a z=1 plane point, as [`Self::distort`], and its Jacobian: `(x, y, ∂x/∂u, ∂x/∂v, ∂y/∂u, ∂y/∂v)`
    #[inline]
    fn map_with_jacobian(x: f32, y: f32, k: &[f32]) -> (f32, f32, f32, f32, f32, f32) {
        let r2 = x * x + y * y;
        let cd   = 1.0 + r2 * (k[0] + r2 * (k[1] + r2 * k[4]));
        let cd2  = 1.0 + r2 * (k[5] + r2 * (k[6] + r2 * k[7]));
        let rad  = cd / cd2;
        // d(rad)/d(r²)
        let drad = (k[0] + r2 * (2.0 * k[1] + 3.0 * k[4] * r2) - rad * (k[5] + r2 * (2.0 * k[6] + 3.0 * k[7] * r2))) / cd2;
        // d(thin prism)/d(r²)
        let (sx, sy) = (k[8] + 2.0 * k[9] * r2, k[10] + 2.0 * k[11] * r2);
        (
            x * rad + k[2] * 2.0 * x * y + k[3] * (r2 + 2.0 * x * x) + k[8]  * r2 + k[9]  * r2 * r2,
            y * rad + k[2] * (r2 + 2.0 * y * y) + k[3] * 2.0 * x * y + k[10] * r2 + k[11] * r2 * r2,
            rad + 2.0 * x * x * drad + 2.0 * k[2] * y + 6.0 * k[3] * x + 2.0 * x * sx,
            2.0 * x * y * drad       + 2.0 * k[2] * x + 2.0 * k[3] * y + 2.0 * y * sx,
            2.0 * x * y * drad       + 2.0 * k[2] * x + 2.0 * k[3] * y + 2.0 * x * sy,
            rad + 2.0 * y * y * drad + 6.0 * k[2] * y + 2.0 * k[3] * x + 2.0 * y * sy,
        )
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
