// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use crate::{ stabilization::KernelParams, lens_profile::LensProfile };

#[derive(Default, Clone)]
pub struct GoProHyperview { }

impl GoProHyperview {
    fn hyperview(uv: (f32, f32)) -> (f32, f32) {
        let x2 = uv.0 * uv.0;
        let y2 = uv.1 * uv.1;
        (
            uv.0 * (1.5805143 + x2 * (-8.1668825 + x2 * (74.5198746 + x2 * (-451.5002441 + x2 * (1551.2922363 + x2 * (-2735.5422363 + x2 * 1923.1572266))))) + y2 * -0.1086027),
            uv.1 * (1.0238225 + y2 * -0.1025671 + x2 * (-0.2639930 + x2 * 0.2979266))
        )
    }

    /// `uv` range: (0,0)...(width, height)
    /// From hyperview to wide
    pub fn undistort_point(&self, mut uv: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        let out_c2 = (params.output_width as f32, params.output_height as f32);
        uv = ((uv.0 / out_c2.0) - 0.5,
              (uv.1 / out_c2.1) - 0.5);

        uv = Self::hyperview(uv);

        uv.0 = uv.0 / 1.555555555;

        Some(((uv.0 + 0.5) * out_c2.0,
              (uv.1 + 0.5) * out_c2.1))
    }

    /// `uv` range: (0,0)...(width, height)
    /// From wide to hyperview
    pub fn distort_point(&self, mut x: f32, mut y: f32, _z: f32, params: &KernelParams) -> (f32, f32) {
        let size = (params.width as f32, params.height as f32);
        x = (x / size.0) - 0.5;
        y = (y / size.1) - 0.5;

        x = x * 1.555555555;

        let mut pp = (x, y);
        for _ in 0..12 {
            let dp = Self::hyperview(pp);
            let diff = (dp.0 - x, dp.1 - y);
            if diff.0.abs() < 1e-6 && diff.1.abs() < 1e-6 {
                break;
            }
            pp.0 -= diff.0;
            pp.1 -= diff.1;
        }

        ((pp.0 + 0.5) * size.0,
         (pp.1 + 0.5) * size.1)
    }
    pub fn adjust_lens_profile(&self, profile: &mut LensProfile) {
        let aspect = (profile.calib_dimension.w as f64 / profile.calib_dimension.h as f64 * 100.0) as usize;
        if aspect == 114 { // It's 8:7
            profile.calib_dimension.w = (profile.calib_dimension.w as f64 * 1.55555555555).round() as usize;
        }
        profile.lens_model = "Hyperview".into();
    }
    pub fn distortion_derivative(&self, _theta: f64, _k: &[f64]) -> Option<f64> {
        None
    }

    pub fn id()   -> &'static str { "gopro_hyperview" }
    pub fn name() -> &'static str { "GoPro Hyperview" }

    pub fn opencl_functions(&self) -> &'static str {
        r#"
        float2 hyperview(float2 uv) {
            float x2 = uv.x * uv.x;
            float y2 = uv.y * uv.y;
            return (float2)(
                uv.x * (1.5805143f + x2 * (-8.1668825f + x2 * (74.5198746f + x2 * (-451.5002441f + x2 * (1551.2922363f + x2 * (-2735.5422363f + x2 * 1923.1572266f))))) + y2 * -0.1086027f),
                uv.y * (1.0238225f + y2 * -0.1025671f + x2 * (-0.2639930f + x2 * 0.2979266f))
            );
        }

        float2 digital_undistort_point(float2 uv, __global KernelParams *params) {
            float2 out_c2 = (float2)(params->output_width, params->output_height);
            uv = (uv / out_c2) - 0.5f;

            uv = hyperview(uv);

            uv.x = uv.x / 1.555555555f;
            uv = (uv + 0.5f) * out_c2;
            return uv;
        }
        float2 digital_distort_point(float2 uv, __global KernelParams *params) {
            float2 size = (float2)(params->width, params->height);
            uv = (uv / size) - 0.5f;
            uv.x = uv.x * 1.555555555f;

            float2 P = uv;
            for (int i = 0; i < 12; ++i) {
                float2 diff = hyperview(P) - uv;
                if (fabs(diff.x) < 1e-6f && fabs(diff.y) < 1e-6f) {
                    break;
                }
                P -= diff;
            }

            uv = (P + 0.5f) * size;

            return uv;
        }"#
    }
    pub fn wgsl_functions(&self) -> &'static str {
        r#"
        fn hyperview(uv: vec2<f32>) -> vec2<f32> {
            let x2 = uv.x * uv.x;
            let y2 = uv.y * uv.y;
            return vec2<f32>(
                uv.x * (1.5805143 + x2 * (-8.1668825 + x2 * (74.5198746 + x2 * (-451.5002441 + x2 * (1551.2922363 + x2 * (-2735.5422363 + x2 * 1923.1572266))))) + y2 * -0.1086027),
                uv.y * (1.0238225 + y2 * -0.1025671 + x2 * (-0.2639930 + x2 * 0.2979266))
            );
        }
        fn digital_undistort_point(_uv: vec2<f32>) -> vec2<f32> {
            let out_c2 = vec2<f32>(f32(params.output_width), f32(params.output_height));
            var uv = _uv;
            uv = (uv / out_c2) - 0.5;

            uv = hyperview(uv);

            uv.x = uv.x / 1.555555555;
            uv = (uv + 0.5) * out_c2;

            return uv;
        }
        fn digital_distort_point(_uv: vec2<f32>) -> vec2<f32> {
            let size = vec2<f32>(f32(params.width), f32(params.height));
            var uv = _uv;
            uv = (uv / size) - 0.5;

            uv.x = uv.x * 1.555555555;

            var P = uv;
            for (var i: i32 = 0; i < 12; i = i + 1) {
                let diff = hyperview(P) - uv;
                if (abs(diff.x) < 1e-6 && abs(diff.y) < 1e-6) {
                    break;
                }
                P -= diff;
            }

            uv = (P + 0.5) * size;

            return uv;
        }"#
    }
}
