// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Adapted from LensFun: https://github.com/lensfun/lensfun/blob/e78e7be448c81256cce36a5a37ddc229616c0db7/libs/lensfun/mod-coord.cpp#L562

use crate::stabilization::KernelParams;

#[derive(Default, Clone)]
pub struct Poly3 { }

const NEWTON_EPS: f32 = 0.00001;

impl Poly3 {
    pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        let inv_k1 = 1.0 / params.k[0];

        let rd = (point.0 * point.0 + point.1 * point.1).sqrt();
        if rd == 0.0 { return None; }

        let rd_div_k1 = rd * inv_k1;

        // Use Newton's method to avoid dealing with complex numbers.
        // When carefully tuned this works almost as fast as Cardano's method (and we don't use complex numbers in it, which is required for a full solution!)
        //
        // Original function: Rd = k1_ * Ru^3 + Ru
        // Target function:   k1_ * Ru^3 + Ru - Rd = 0
        // Divide by k1_:     Ru^3 + Ru/k1_ - Rd/k1_ = 0
        // Derivative:        3 * Ru^2 + 1/k1_
        let mut ru = rd;
        for i in 0..10 {
            let fru = ru * ru * ru + ru * inv_k1 - rd_div_k1;
            if fru >= -NEWTON_EPS && fru < NEWTON_EPS {
                break;
            }
            if i > 5 {
                // Does not converge, no real solution in this area?
                return None;
            }

            ru = ru - (fru / (3.0 * ru * ru + inv_k1));
        }
        if ru < 0.0 {
            return None;
        }

        ru = ru / rd;

        Some((
            point.0 * ru,
            point.1 * ru
        ))
    }

    pub fn distort_point(&self, x: f32, y: f32, z: f32, params: &KernelParams) -> (f32, f32) {
        let x = x / z;
        let y = y / z;
        let poly2 = params.k[0] * (x.powi(2) + y.powi(2)) + 1.0;

        (
            x * poly2,
            y * poly2
        )
    }
    pub fn adjust_lens_profile(&self, _profile: &mut crate::LensProfile) { }

    pub fn rescale_coeffs(k: &mut [f64], hugin_scaling: f64) {
        let d = 1.0 - k[0];
        k[0] *= hugin_scaling.powi(2) / d.powi(3);
    }

    pub fn id() -> &'static str { "poly3" }
    pub fn name() -> &'static str { "Poly3" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("poly3.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("poly3.wgsl") }
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

