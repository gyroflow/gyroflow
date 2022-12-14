// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// See https://github.com/gyroflow/gyroflow/issues/43 for research details

use crate::{ stabilization::KernelParams, lens_profile::LensProfile };

#[derive(Default, Clone)]
pub struct GoProSuperview { }

impl GoProSuperview {
    /// `uv` range: (0,0)...(width, height)
    /// From superview to wide
    pub fn undistort_point(&self, mut uv: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        let out_c2 = (params.output_width as f32, params.output_height as f32);
        uv = ((uv.0 / out_c2.0) - 0.5,
              (uv.1 / out_c2.1) - 0.5);

        uv.0 *= 1.0 - 0.45 * uv.0.abs();
        uv.0 *= 0.168827 * (5.53572 + uv.0.abs());
        uv.1 *= 0.130841 * (7.14285 + uv.1.abs());

        Some(((uv.0 + 0.5) * out_c2.0,
              (uv.1 + 0.5) * out_c2.1))
    }

    /// `uv` range: (0,0)...(width, height)
    /// From wide to superview
    pub fn distort_point(&self, mut x: f32, mut y: f32, _z: f32, params: &KernelParams) -> (f32, f32) {
        let size = (params.width as f32, params.height as f32);
        x = (x / size.0) - 0.5;
        y = (y / size.1) - 0.5;

        let xs = if x < 0.0 { -1.0 } else { 1.0 };
        let ys = if y < 0.0 { -1.0 } else { 1.0 };

        y = ys * (3.57143 * ((0.5992 * y.abs() + 1.0).sqrt() - 1.0));
        x = xs * (3.57143 * (0.880341 * (0.5992 * x.abs() + 0.775).sqrt() - 0.775));
        x = xs * (-1.11111 * ((1.0 - 1.8 * x.abs()).sqrt() - 1.0));

        ((x + 0.5) * size.0,
         (y + 0.5) * size.1)
    }
    pub fn adjust_lens_profile(&self, profile: &mut LensProfile) {
        let aspect = (profile.calib_dimension.w as f64 / profile.calib_dimension.h as f64 * 100.0) as usize;
        if aspect == 133 { // It's 4:3
            profile.calib_dimension.w = (profile.calib_dimension.w as f64 * 1.3333333333333).round() as usize;
        }
        profile.lens_model = "Superview".into();
    }

    pub fn id()   -> &'static str { "gopro_superview" }
    pub fn name() -> &'static str { "GoPro Superview" }

    pub fn opencl_functions(&self) -> &'static str {
        r#"
        float2 digital_undistort_point(float2 uv, __global KernelParams *params) {
            float2 out_c2 = (float2)(params->output_width, params->output_height);
            uv = (uv / out_c2) - 0.5f;

            uv.x *= 1.0f - 0.45f * fabs(uv.x);
            uv.x *= 0.168827f * (5.53572f + fabs(uv.x));
            uv.y *= 0.130841f * (7.14285f + fabs(uv.y));

            uv = (uv + 0.5f) * out_c2;

            return uv;
        }
        float2 digital_distort_point(float2 uv, __global KernelParams *params) {
            float2 size = (float2)(params->width, params->height);
            uv = (uv / size) - 0.5f;

            float xs = uv.x < 0.0f? -1.0f : 1.0f;
            float ys = uv.y < 0.0f? -1.0f : 1.0f;

            uv.y = ys * (3.57143f * (sqrt(0.5992f * fabs(uv.y) + 1.0f) - 1.0f));
            uv.x = xs * (3.57143f * (0.880341f * sqrt(0.5992f * fabs(uv.x) + 0.775f) - 0.775f));
            uv.x = xs * (-1.11111f * (sqrt(1.0f - 1.8f * fabs(uv.x)) - 1.0f));

            uv = (uv + 0.5f) * size;

            return uv;
        }"#
    }
    pub fn wgsl_functions(&self) -> &'static str {
        r#"
        fn digital_undistort_point(uv: vec2<f32>) -> vec2<f32> {
            let out_c2 = vec2<f32>(f32(params.output_width), f32(params.output_height));
            var uv = uv;
            uv = (uv / out_c2) - 0.5;

            uv.x = uv.x * (1.0 - 0.45 * abs(uv.x));
            uv.x = uv.x * (0.168827 * (5.53572 + abs(uv.x)));
            uv.y = uv.y * (0.130841 * (7.14285 + abs(uv.y)));

            uv = (uv + 0.5) * out_c2;

            return uv;
        }
        fn digital_distort_point(uv: vec2<f32>) -> vec2<f32> {
            let size = vec2<f32>(f32(params.width), f32(params.height));
            var uv = uv;
            uv = (uv / size) - 0.5;

            let xs = uv.x / max(0.000001, abs(uv.x));
            let ys = uv.y / max(0.000001, abs(uv.y));

            uv.y = ys * (3.57143 * (sqrt(0.5992 * abs(uv.y) + 1.0) - 1.0));
            uv.x = xs * (3.57143 * (0.880341 * sqrt(0.5992 * abs(uv.x) + 0.775) - 0.775));
            uv.x = xs * (-1.11111 * (sqrt(1.0 - 1.8 * abs(uv.x)) - 1.0));

            uv = (uv + 0.5) * size;

            return uv;
        }"#
    }
}
