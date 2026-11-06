// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// See https://github.com/gyroflow/gyroflow/issues/43 for research details

use crate::{ stabilization::KernelParams, lens_profile::LensProfile };

#[derive(Default, Clone)]
pub struct GoPro6Superview { }

impl GoPro6Superview {
    fn superview(uv: (f32, f32)) -> (f32, f32) {
        // The polynomials are a fit over the recorded frame, [-0.5, 0.5]; outside it the high-order terms
        // run away and the fixed-point inversion in `distort_point` leaves for infinity and then for NaN -
        // which the point path hands straight to the zoom search, where a frame corner that is NaN is a
        // corner the search never sees. Clamp the argument to the frame and continue with slope 1 past it:
        // identical inside it, and outside the map stays smooth and strictly increasing, so a coordinate
        // off the frame cleanly stays off it (background) instead of folding back in. Exactly the
        // continuation `gopro_warp` already gives the camera's own MAPX/MAPY
        let (cx, cy) = (uv.0.clamp(-0.5, 0.5), uv.1.clamp(-0.5, 0.5));
        let mut x = cx;
        x *= 1.0 - 0.48 * x.abs();
        x *= 0.943396 * (1.0 + 0.157895 * x.abs());
        let mut y = cy;
        y *= 0.943396 * (1.0 + 0.060000 * (y * 2.0).abs());
        (x + (uv.0 - cx), y + (uv.1 - cy))
    }

    /// `uv` range: (0,0)...(width, height)
    /// From superview to wide
    pub fn undistort_point(&self, mut uv: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        let out_c2 = (params.output_width as f32, params.output_height as f32);
        uv = ((uv.0 / out_c2.0) - 0.5,
              (uv.1 / out_c2.1) - 0.5);

        uv = Self::superview(uv);

        Some(((uv.0 + 0.5) * out_c2.0,
              (uv.1 + 0.5) * out_c2.1))
    }

    /// `uv` range: (0,0)...(width, height)
    /// From wide to superview
    pub fn distort_point(&self, mut x: f32, mut y: f32, _z: f32, params: &KernelParams) -> (f32, f32) {
        let size = (params.width as f32, params.height as f32);
        x = (x / size.0) - 0.5;
        y = (y / size.1) - 0.5;

        let mut pp = (x, y);
        for _ in 0..12 {
            let dp = Self::superview(pp);
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
        if aspect == 133 { // It's 4:3
            profile.calib_dimension.w = (profile.calib_dimension.w as f64 * 1.3333333333333).round() as usize;
        }
        profile.lens_model = "Superview".into();
    }
    pub fn distortion_derivative(&self, _theta: f64, _k: &[f64]) -> Option<f64> {
        None
    }

    pub fn id()   -> &'static str { "gopro6_superview" }
    pub fn name() -> &'static str { "GoPro6 Superview" }

    pub fn opencl_functions(&self) -> &'static str {
        r#"
        float2 superview(float2 uv) {
            // Clamp the polynomial argument to the recorded frame and continue linearly past it, so the map
            // stays smooth & monotonic everywhere (no divergence to NaN). See gopro6_superview.rs
            float2 c = clamp(uv, -0.5f, 0.5f);
            float x = c.x, y = c.y;
            x *= 1.0f - 0.48f * fabs(x);
            x *= 0.943396f * (1.0f + 0.157895f * fabs(x));
            y *= 0.943396f * (1.0f + 0.060000f * fabs(y * 2.0f));
            return (float2)(x, y) + (uv - c);
        }

        float2 digital_undistort_point(float2 uv, __global KernelParams *params) {
            float2 out_c2 = (float2)(params->output_width, params->output_height);
            uv = (uv / out_c2) - 0.5f;

            uv = superview(uv);

            uv = (uv + 0.5f) * out_c2;
            return uv;
        }
        float2 digital_distort_point(float2 uv, __global KernelParams *params) {
            float2 size = (float2)(params->width, params->height);
            uv = (uv / size) - 0.5f;

            float2 P = uv;
            for (int i = 0; i < 12; ++i) {
                float2 diff = superview(P) - uv;
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
        fn superview(_uv: vec2<f32>) -> vec2<f32> {
            // Clamp the polynomial argument to the recorded frame and continue linearly past it, so the map
            // stays smooth & monotonic everywhere (no divergence to NaN). See gopro6_superview.rs
            let c = clamp(_uv, vec2<f32>(-0.5, -0.5), vec2<f32>(0.5, 0.5));
            var uv = c;
            uv.x *= 1.0 - 0.48 * abs(uv.x);
            uv.x *= 0.943396 * (1.0 + 0.157895 * abs(uv.x));
            uv.y *= 0.943396 * (1.0 + 0.060000 * abs(uv.y * 2.0));
            return uv + (_uv - c);
        }
        fn digital_undistort_point(_uv: vec2<f32>) -> vec2<f32> {
            let out_c2 = vec2<f32>(f32(params.output_width), f32(params.output_height));
            var uv = _uv;
            uv = (uv / out_c2) - 0.5;

            uv = superview(uv);

            uv = (uv + 0.5) * out_c2;

            return uv;
        }
        fn digital_distort_point(_uv: vec2<f32>) -> vec2<f32> {
            let size = vec2<f32>(f32(params.width), f32(params.height));
            var uv = _uv;
            uv = (uv / size) - 0.5;

            var P = uv;
            for (var i: i32 = 0; i < 12; i = i + 1) {
                let diff = superview(P) - uv;
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
