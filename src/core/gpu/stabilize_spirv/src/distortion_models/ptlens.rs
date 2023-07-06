
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Adapted from LensFun: https://github.com/lensfun/lensfun/blob/e78e7be448c81256cce36a5a37ddc229616c0db7/libs/lensfun/mod-coord.cpp#L696

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3 };

pub struct PtLens { }

const NEWTON_EPS: f32 = 0.00001;

impl PtLens {
    pub fn undistort_point(point: Vec2, params: &KernelParams) -> Vec2 {
        let rd = point.length();
        if rd == 0.0 { return vec2(-99999.0, -99999.0); }

        let mut ru = rd;
        let mut i = 0; while i < 6 {
        // for i in 0..10 {
            let fru = ru * (params.k1.x * ru * ru * ru + params.k1.y * ru * ru + params.k1.z * ru + 1.0) - rd;
            if fru >= -NEWTON_EPS && fru < NEWTON_EPS {
                break;
            }

            ru = ru - (fru / (4.0 * params.k1.x * ru * ru * ru + 3.0 * params.k1.y * ru * ru + 2.0 * params.k1.z * ru + 1.0));
            i += 1;
        }
        if i > 5 || ru < 0.0 {
            // Does not converge, no real solution in this area?
            return vec2(-99999.0, -99999.0);
        }

        ru = ru / rd;

        point * ru
    }

    pub fn distort_point(point: Vec3, params: &KernelParams) -> Vec2 {
        let x = point.x / point.z;
        let y = point.y / point.z;
        let ru2 = x.powi(2) + y.powi(2);
        let r = ru2.sqrt();
        let poly3 = params.k1.x * ru2 * r + params.k1.y * ru2 + params.k1.z * r + 1.0;

        vec2(
            x * poly3,
            y * poly3
        )
    }

    #[cfg(not(target_arch = "spirv"))]
    #[allow(unused)]
    pub fn rescale_coeffs(mut k1: crate::glam::Vec4, hugin_scaling: f32) -> crate::glam::Vec4 {
        let d = 1.0 - k1.x - k1.y - k1.z;
        k1.x *= hugin_scaling.powi(3) / d.powi(4);
        k1.y *= hugin_scaling.powi(2) / d.powi(3);
        k1.z *= hugin_scaling / d.powi(2);
        k1
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(_calib_w: &mut usize, _calib_h: &mut usize/*, lens_model: &mut String*/) { }
}
