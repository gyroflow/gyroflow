// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Adapted from LensFun: https://github.com/lensfun/lensfun/blob/e78e7be448c81256cce36a5a37ddc229616c0db7/libs/lensfun/mod-coord.cpp#L562

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3 };

pub struct Poly3 { }

const NEWTON_EPS: f32 = 0.00001;

impl Poly3 {
    pub fn undistort_point(point: Vec2, params: &KernelParams) -> Vec2 {
        let inv_k1 = 1.0 / params.k1.x;

        let rd = point.length();
        if rd == 0.0 { return vec2(-99999.0, -99999.0); }

        let rd_div_k1 = rd * inv_k1;

        // Use Newton's method to avoid dealing with complex numbers.
        // When carefully tuned this works almost as fast as Cardano's method (and we don't use complex numbers in it, which is required for a full solution!)
        //
        // Original function: Rd = k1_ * Ru^3 + Ru
        // Target function:   k1_ * Ru^3 + Ru - Rd = 0
        // Divide by k1_:     Ru^3 + Ru/k1_ - Rd/k1_ = 0
        // Derivative:        3 * Ru^2 + 1/k1_
        let mut ru = rd;
        let mut i = 0; while i < 6 {
        // for i in 0..10 {
            let fru = ru * ru * ru + ru * inv_k1 - rd_div_k1;
            if fru >= -NEWTON_EPS && fru < NEWTON_EPS {
                break;
            }

            ru = ru - (fru / (3.0 * ru * ru + inv_k1));
            i += 1;
        }
        if i > 5 || ru < 0.0 {
            // Does not converge, no real solution in this area?
            return vec2(-99999.0, -99999.0);
        }

        ru = ru / rd;

        vec2(
            point.x * ru,
            point.y * ru
        )
    }

    pub fn distort_point(point: Vec3, params: &KernelParams) -> Vec2 {
        let x = point.x / point.z;
        let y = point.y / point.z;
        let poly2 = params.k1.x * (x.powi(2) + y.powi(2)) + 1.0;

        vec2(
            x * poly2,
            y * poly2
        )
    }

    #[cfg(not(target_arch = "spirv"))]
    #[allow(unused)]
    pub fn rescale_coeffs(mut k1: crate::glam::Vec4, hugin_scaling: f32) -> crate::glam::Vec4 {
        let d = 1.0 - k1.x;
        k1.x *= hugin_scaling.powi(2) / d.powi(3);
        k1
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(_calib_w: &mut usize, _calib_h: &mut usize/*, lens_model: &mut String*/) { }
}

// TODO
// let focal = 28;
// let crop_factor = 1.0;
// let aspect_ratio = 4.0 / 3.0;

// let real_focal = real_focal.unwrap_or_else(|| match model {
//     "ptlens" => focal * (1.0 - k[0] - k[1] - k[2]),
//     "poly3"  => focal * (1.0 - k[0]),
//     _ => focal
// });
// let hugin_scale_in_millimeters = 36.0.hypot(24.0) / crop_factor / aspect_ratio.hypot(1.0) / 2.0;
// let hugin_scaling = real_focal / hugin_scale_in_millimeters;
// rescale_coeffs(k, hugin_scaling);

