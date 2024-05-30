// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Adapted from LensFun: https://github.com/lensfun/lensfun/blob/e78e7be448c81256cce36a5a37ddc229616c0db7/libs/lensfun/mod-coord.cpp#L636

use crate::stabilization::KernelParams;

#[derive(Default, Clone)]
pub struct Poly5 { }

const NEWTON_EPS: f32 = 0.00001;

impl Poly5 {
    pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        let rd = (point.0 * point.0 + point.1 * point.1).sqrt();
        if rd == 0.0 { return None; }

        let mut ru = rd;
        for i in 0..10 {
            let ru2 = ru * ru;
            let fru = ru * (1.0 + params.k[0] * ru2 + params.k[1] * ru2 * ru2) - rd;
            if fru >= -NEWTON_EPS && fru < NEWTON_EPS {
                break;
            }
            if i > 5 {
                // Does not converge, no real solution in this area?
                return None;
            }

            ru = ru - (fru / (1.0 + 3.0 * params.k[0] * ru2 + 5.0 * params.k[1] * ru2 * ru2));
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
        let ru2 = x.powi(2) + y.powi(2);
        let poly4 = 1.0 + params.k[0] * ru2 + params.k[1] * ru2 * ru2;

        (
            x * poly4,
            y * poly4
        )
    }
    pub fn adjust_lens_profile(&self, _profile: &mut crate::LensProfile) { }

    pub fn distortion_derivative(&self, theta: f64, k: &[f64]) -> Option<f64> {
        if k.len() < 2 { return None; }
        let ru2 = theta * theta;
        Some(
            1.0 + 3.0 * k[0] * ru2 + 5.0 * k[1] * ru2 * ru2
        )
    }

    pub fn rescale_coeffs(k: &mut [f64], hugin_scaling: f64) {
        k[0] *= hugin_scaling.powi(2);
        k[1] *= hugin_scaling.powi(4);
    }

    pub fn id() -> &'static str { "poly5" }
    pub fn name() -> &'static str { "Poly5" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("poly5.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("poly5.wgsl") }
}
