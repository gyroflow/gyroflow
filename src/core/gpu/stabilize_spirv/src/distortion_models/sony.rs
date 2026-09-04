// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)
//
// Sony native lens model: the camera's lens curve (ray angle at equally spaced image radii) evaluated as a natural
// cubic spline radius -> angle.

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3, Vec4 };
pub struct Sony { }

const TMAX: f32 = 1.5533; // ~89°
const TT: f32 = 57.14902; // tan(TMAX)
const THETA0: f32 = 0.052359879; // 3°, end of the linear region near the optical axis (k[13] = its radius r_lin)

impl Sony {
    #[inline] fn k(params: &KernelParams, i: i32) -> f32 {
        let v = i / 4;
        let q: Vec4 = match v { 0 => params.k1, 1 => params.k2, 2 => params.k3, 3 => params.k4, 4 => params.k5, _ => params.k6 };
        match i - v * 4 { 0 => q.x, 1 => q.y, 2 => q.z, _ => q.w }
    }
    #[inline] fn segments(params: &KernelParams) -> i32 {
        let n = params.k1.x;
        if n >= 1.0 && n <= 10.0 && params.k1.y > 0.0 && params.k1.z == 0.0 { n as i32 } else { 0 }
    }
    // (y_i, b_i, c_i, d_i) of spline segment i
    #[inline] fn segment(params: &KernelParams, i: i32) -> (f32, f32, f32, f32) {
        let h = params.k1.y;
        let (y0, y1) = (Self::k(params, 2 + i), Self::k(params, 3 + i));
        let (c0, c1) = (if i == 0 { 0.0 } else { Self::k(params, 13 + i) }, Self::k(params, 14 + i)); // k[13] holds r_lin, c_0 is 0
        (y0, (y1 - y0) / h - h * (c1 + 2.0 * c0) / 3.0, c0, (c1 - c0) / (3.0 * h))
    }
    // Ray angle at normalized image radius r (linear continuation with the end slope outside the knots)
    #[inline] fn angle_at(params: &KernelParams, n: i32, r: f32) -> f32 {
        let h = params.k1.y;
        let r_lin = Self::k(params, 13);
        if r_lin > 0.0 && r < r_lin { return THETA0 * r / r_lin; }
        let i = ((r / h) as i32).max(0).min(n - 1);
        let (y, b, c, d) = Self::segment(params, i);
        let dx = (r - i as f32 * h).max(0.0).min(h);
        let slope = b + (2.0 * c + 3.0 * d * dx) * dx;
        y + (b + (c + d * dx) * dx) * dx + slope * (r - i as f32 * h - dx)
    }
    // Normalized image radius for ray angle theta (inverse of angle_at)
    #[inline] fn radius_at(params: &KernelParams, n: i32, theta: f32) -> f32 {
        let h = params.k1.y;
        if theta <= 0.0 { return 0.0; }
        let r_lin = Self::k(params, 13);
        if r_lin > 0.0 && theta < THETA0 { return theta * r_lin / THETA0; }
        let mut i = 0;
        while i + 1 < n && theta >= Self::k(params, 3 + i) { i += 1; }
        let (y, b, c, d) = Self::segment(params, i);
        let y1 = Self::k(params, 3 + i);
        if theta >= y1 { // past the last knot: linear continuation
            let slope = b + (2.0 * c + 3.0 * d * h) * h;
            if slope > 1e-9 { return n as f32 * h + (theta - y1) / slope; }
            return n as f32 * h;
        }
        // Newton on the segment's cubic, starting from the chord
        let mut dx = h * (theta - y) / (y1 - y).max(1e-12);
        let mut it = 0; while it < 8 {
            let f = y + (b + (c + d * dx) * dx) * dx - theta;
            let fp = b + (2.0 * c + 3.0 * d * dx) * dx;
            if fp.abs() < 1e-12 { break; }
            let fix = f / fp;
            dx = (dx - fix).max(0.0).min(h);
            if fix.abs() < 1e-7 { break; }
            it += 1;
        }
        i as f32 * h + dx
    }

    pub fn undistort_point(point: Vec2, params: &KernelParams) -> Vec2 {
        let n = Self::segments(params);
        if n == 0 { return point; }
        let r = point.length();
        if r < 1e-9 { return point; }
        let theta = Self::angle_at(params, n, r);
        if theta <= 0.0 { return vec2(-99999.0, -99999.0); }
        let rr = if theta < TMAX { theta.tan() } else { TT + (theta - TMAX) * (1.0 + TT * TT) };
        if params.r_limit > 0.0 && rr > params.r_limit { return vec2(-99999.0, -99999.0); }
        point * (rr / r)
    }

    pub fn distort_point(point: Vec3, params: &KernelParams) -> Vec2 {
        let pt = vec2(point.x / point.z, point.y / point.z);
        let n = Self::segments(params);
        if n == 0 { return pt; }
        let r = pt.length();
        if r < 1e-9 { return pt; }
        let theta = if r < TT { r.atan() } else { TMAX + (r - TT) / (1.0 + TT * TT) };
        pt * (Self::radius_at(params, n, theta) / r)
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(_calib_w: &mut usize, _calib_h: &mut usize/*, lens_model: &mut String*/) { }
}
