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
        let r_norm = point.length();
        // No POLY block: a pinhole, so the image radius is `tan θ`
        if params.k1.y == 0.0 { return if r_norm < 1e-12 { point } else { point * (r_norm.atan() / r_norm) }; }
        if r_norm < 1e-9 { return point; }
        let p = r_norm / params.k1.y;
        let theta = Self::poly_eval(p, params);
        // Outside its fit range the POLY is free to run negative or past 180°, and neither is an angle
        // this pipeline can carry - a negative θ hands back the ray from the opposite side of the frame,
        // and past 180° `ray_to_dir`'s `sin θ` has turned over. See `distortion_models::gopro`
        if !(theta > 0.0 && theta < 3.1415927) { return vec2(-99999.0, -99999.0); }
        point * (theta / r_norm)
    }

    /// From ray to image
    pub fn distort_point(point: Vec3, params: &KernelParams) -> Vec2 {
        // No POLY block: a pinhole, which has no image of a ray at or past 90°
        if params.k1.y == 0.0 { return if point.z > 1e-9 { vec2(point.x / point.z, point.y / point.z) } else { vec2(point.x * 1e9, point.y * 1e9) }; }
        let pos = vec2(point.x, point.y);
        let r = pos.length();
        if r < 1e-12 { return vec2(0.0, 0.0); }
        // atan2 against the ray's own z: a GoPro's 150°+ field needs the angle, not a z=1 plane radius
        let theta = r.atan2(point.z);
        pos * (params.k1.y * Self::poly_invert(theta, params) / r)
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(_calib_w: &mut usize, _calib_h: &mut usize/*, lens_model: &mut String*/) { }
}
