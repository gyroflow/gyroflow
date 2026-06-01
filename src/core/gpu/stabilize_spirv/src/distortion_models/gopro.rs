// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>
//
// GoPro native radial lens model. Raw POLY radial coeffs r0..r6 in params.k1/k2.
// The Superview/Hyperview MAPX/MAPY warp is a separate digital lens (gopro_warp).

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3 };

pub struct GoPro { }

impl GoPro {
    fn poly_eval(p: f32, params: &KernelParams) -> f32 {
        params.k1.x + p * (params.k1.y + p * (params.k1.z + p * (params.k1.w + p * (params.k2.x + p * (params.k2.y + p * params.k2.z)))))
    }
    fn poly_deriv(p: f32, params: &KernelParams) -> f32 {
        params.k1.y + p * (2.0 * params.k1.z + p * (3.0 * params.k1.w + p * (4.0 * params.k2.x + p * (5.0 * params.k2.y + p * (6.0 * params.k2.z)))))
    }
    fn poly_invert(theta: f32, params: &KernelParams) -> f32 {
        let mut p = (theta - params.k1.x) / params.k1.y;
        for _ in 0..10 {
            let d = Self::poly_deriv(p, params);
            if d.abs() < 1e-12 { break; }
            let fix = (Self::poly_eval(p, params) - theta) / d;
            p -= fix;
            if fix.abs() < 1e-7 { break; }
        }
        p
    }

    /// From image to ray
    pub fn undistort_point(point: Vec2, params: &KernelParams) -> Vec2 {
        if params.k1.y == 0.0 { return point; }
        let r_norm = point.length();
        if r_norm < 1e-9 { return point; }
        let p = r_norm / params.k1.y;
        let theta = Self::poly_eval(p, params);
        let scale = theta.tan() / r_norm;
        point * scale
    }

    /// From ray to image
    pub fn distort_point(point: Vec3, params: &KernelParams) -> Vec2 {
        let pos = vec2(point.x / point.z, point.y / point.z);
        if params.k1.y == 0.0 { return pos; }
        let r = pos.length();
        let theta = r.atan();
        let p = Self::poly_invert(theta, params);
        let r_norm = params.k1.y * p;
        let scale = if r == 0.0 { 1.0 } else { r_norm / r };
        pos * scale
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(_calib_w: &mut usize, _calib_h: &mut usize/*, lens_model: &mut String*/) { }
}
