
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Adapted from LensFun: https://github.com/lensfun/lensfun/blob/e78e7be448c81256cce36a5a37ddc229616c0db7/libs/lensfun/mod-coord.cpp#L696

#[derive(Default, Clone)]
pub struct PtLens { }

const NEWTON_EPS: f64 = 0.00001;

impl PtLens {
    pub fn undistort_point<T: num_traits::Float>(&self, point: (T, T), k: &[T], amount: T) -> Option<(T, T)> {
        let t_0 = T::from(0.0f32).unwrap();
        let t_1 = T::from(1.0f32).unwrap();
        let t_2 = T::from(2.0f32).unwrap();
        let t_3 = T::from(3.0f32).unwrap();
        let t_4 = T::from(4.0f32).unwrap();
        let t_eps = T::from(NEWTON_EPS).unwrap();

        let rd = (point.0 * point.0 + point.1 * point.1).sqrt();
        if rd == t_0 { return None; }

        let mut ru = rd;
        for i in 0..10 {
            let fru = ru * (k[0] * ru * ru * ru + k[1] * ru * ru + k[2] * ru + t_1) - rd;
            if fru >= -t_eps && fru < t_eps {
                break;
            }
            if i > 5 {
                // Does not converge, no real solution in this area?
                return None;
            }

            ru = ru - (fru / (t_4 * k[0] * ru * ru * ru + t_3 * k[1] * ru * ru + t_2 * k[2] * ru + t_1));
        }
        if ru < t_0 {
            return None;
        }

        ru = ru / rd;

        // Apply only requested amount
        ru = t_1 + (ru - t_1) * (t_1 - amount);

        Some((
            point.0 * ru,
            point.1 * ru
        ))
    }

    pub fn distort_point<T: num_traits::Float>(&self, point: (T, T), k: &[T], amount: T) -> (T, T) {
        let t_1 = T::from(1.0f32).unwrap();

        let ru2 = point.0 * point.0 + point.1 * point.1;
        let r = ru2.sqrt();
        let mut poly3 = k[0] * ru2 * r + k[1] * ru2 + k[2] * r + t_1;
        poly3 = t_1 + (poly3 - t_1) * (t_1 - amount);

        (
            point.0 * poly3,
            point.1 * poly3
        )
    }
    pub fn rescale_coeffs(k: &mut [f64], hugin_scaling: f64) {
        let d = 1.0 - k[0] - k[1] - k[2];
        k[0] *= hugin_scaling.powi(3) / d.powi(4);
        k[1] *= hugin_scaling.powi(2) / d.powi(3);
        k[2] *= hugin_scaling / d.powi(2);
    }

    pub fn id(&self) -> i32 { 4 }
    pub fn name(&self) -> &'static str { "PTLens" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("ptlens.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("ptlens.wgsl") }
    pub fn glsl_shader_path(&self) -> &'static str { ":/src/qt_gpu/compiled/undistort_ptlens.frag.qsb" }
}
