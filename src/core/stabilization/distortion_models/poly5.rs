// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Adapted from LensFun: https://github.com/lensfun/lensfun/blob/e78e7be448c81256cce36a5a37ddc229616c0db7/libs/lensfun/mod-coord.cpp#L636

#[derive(Default, Clone)]
pub struct Poly5 { }

const NEWTON_EPS: f64 = 0.00001;

impl Poly5 {
    pub fn undistort_point<T: num_traits::Float>(&self, point: (T, T), k: &[T], amount: T) -> Option<(T, T)> {
        let t_0 = T::from(0.0f32).unwrap();
        let t_1 = T::from(1.0f32).unwrap();
        let t_3 = T::from(3.0f32).unwrap();
        let t_5 = T::from(5.0f32).unwrap();
        let t_eps = T::from(NEWTON_EPS).unwrap();

        let rd = (point.0 * point.0 + point.1 * point.1).sqrt();
        if rd == t_0 { return None; }

        let mut ru = rd;
        for i in 0..10 {
            let ru2 = ru * ru;
            let fru = ru * (t_1 + k[0] * ru2 + k[1] * ru2 * ru2) - rd;
            if fru >= -t_eps && fru < t_eps {
                break;
            }
            if i > 5 {
                // Does not converge, no real solution in this area?
                return None;
            }

            ru = ru - (fru / (t_1 + t_3 * k[0] * ru2 + t_5 * k[1] * ru2 * ru2));
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
        let mut poly4 = t_1 + k[0] * ru2 + k[1] * ru2 * ru2;
        poly4 = t_1 + (poly4 - t_1) * (t_1 - amount);

        (
            point.0 * poly4,
            point.1 * poly4
        )
    }

    pub fn rescale_coeffs(k: &mut [f64], hugin_scaling: f64) {
        k[0] *= hugin_scaling.powi(2);
        k[1] *= hugin_scaling.powi(4);
    }

    pub fn id(&self) -> i32 { 3 }
    pub fn name(&self) -> &'static str { "Poly5" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("poly5.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("poly5.wgsl") }
    pub fn glsl_shader_path(&self) -> &'static str { ":/src/qt_gpu/compiled/undistort_poly5.frag.qsb" }
}
