
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Adapted from LensFun: https://github.com/lensfun/lensfun/blob/e78e7be448c81256cce36a5a37ddc229616c0db7/libs/lensfun/mod-coord.cpp#L696

use crate::stabilization::KernelParams;

#[derive(Default, Clone)]
pub struct PtLens { }

const NEWTON_EPS: f32 = 0.00001;

impl PtLens {
    pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        let rd = (point.0 * point.0 + point.1 * point.1).sqrt();
        if rd == 0.0 { return None; }

        let mut ru = rd;
        for i in 0..10 {
            let fru = ru * (params.k[0] * ru * ru * ru + params.k[1] * ru * ru + params.k[2] * ru + 1.0) - rd;
            if fru >= -NEWTON_EPS && fru < NEWTON_EPS {
                break;
            }
            if i > 5 {
                // Does not converge, no real solution in this area?
                return None;
            }

            ru = ru - (fru / (4.0 * params.k[0] * ru * ru * ru + 3.0 * params.k[1] * ru * ru + 2.0 * params.k[2] * ru + 1.0));
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
        let r = ru2.sqrt();
        let poly3 = params.k[0] * ru2 * r + params.k[1] * ru2 + params.k[2] * r + 1.0;

        (
            x * poly3,
            y * poly3
        )
    }
    pub fn adjust_lens_profile(&self, _profile: &mut crate::LensProfile) { }

    pub fn rescale_coeffs(k: &mut [f64], hugin_scaling: f64) {
        let d = 1.0 - k[0] - k[1] - k[2];
        k[0] *= hugin_scaling.powi(3) / d.powi(4);
        k[1] *= hugin_scaling.powi(2) / d.powi(3);
        k[2] *= hugin_scaling / d.powi(2);
    }

    pub fn id() -> &'static str { "ptlens" }
    pub fn name() -> &'static str { "PTLens" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("ptlens.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("ptlens.wgsl") }
}
