// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// See https://github.com/gyroflow/gyroflow/issues/43 for research details

use crate::{ stabilization::KernelParams, lens_profile::LensProfile };

#[derive(Default, Clone)]
pub struct GoProSuperview { }

impl GoProSuperview {
    fn superview(uv: (f32, f32)) -> (f32, f32) {
        // The polynomials are a fit over the recorded frame, [-0.5, 0.5]; outside it the high-order terms
        // run away and the fixed-point inversion in `distort_point` leaves for infinity and then for NaN -
        // which the point path hands straight to the zoom search, where a frame corner that is NaN is a
        // corner the search never sees. Clamp the argument to the frame and continue with slope 1 past it:
        // identical inside it, and outside the map stays smooth and strictly increasing, so a coordinate
        // off the frame cleanly stays off it (background) instead of folding back in. Exactly the
        // continuation `gopro_warp` already gives the camera's own MAPX/MAPY
        let x = uv.0.clamp(-0.5, 0.5);
        let y = uv.1.clamp(-0.5, 0.5);
        let x2 = x * x;
        let y2 = y * y;
        (
            x * (1.2100393 + x2 * (-1.2758402 + x2 * 1.7751845)) + (uv.0 - x),
            y * (0.9364505 + (0.4465308 - 0.7683315 * y2) * y2 + (-0.3574087 + 1.1584653 * y2 + 0.3529348 * x2) * x2) + (uv.1 - y)
        )
    }

    /// `uv` range: (0,0)...(width, height)
    /// From superview to wide
    pub fn undistort_point(&self, mut uv: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        let out_c2 = (params.output_width as f32, params.output_height as f32);
        uv = ((uv.0 / out_c2.0) - 0.5,
              (uv.1 / out_c2.1) - 0.5);

        uv = Self::superview(uv);

        uv.0 = uv.0 / 1.333333333;

        Some(((uv.0 + 0.5) * out_c2.0,
              (uv.1 + 0.5) * out_c2.1))
    }

    /// `uv` range: (0,0)...(width, height)
    /// From wide to superview
    pub fn distort_point(&self, mut x: f32, mut y: f32, _z: f32, params: &KernelParams) -> (f32, f32) {
        let size = (params.width as f32, params.height as f32);
        x = (x / size.0) - 0.5;
        y = (y / size.1) - 0.5;

        // Solve `superview(pp) = target`, seeded at the un-stretched coordinate: it is inside the recorded
        // frame wherever the answer is, and already ~ the solution since `superview(x).x ~ 1.21*x`. Seeded
        // at `target` instead (up to +-0.667) the iteration starts outside the fit, which is where it used
        // to leave for infinity - `gopro_warp` seeds the same way for the same reason
        let target = (x * 1.333333333, y);

        let mut pp = (x, y);
        for _ in 0..12 {
            let dp = Self::superview(pp);
            let diff = (dp.0 - target.0, dp.1 - target.1);
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

    pub fn id()   -> &'static str { "gopro_superview" }
    pub fn name() -> &'static str { "GoPro Superview" }

    pub fn opencl_functions(&self) -> &'static str {
        r#"
        float2 superview(float2 uv) {
            // Clamp the polynomial argument to the recorded frame and continue linearly past it, so the map
            // stays smooth & monotonic everywhere (no divergence to NaN). See gopro_superview.rs
            float x = clamp(uv.x, -0.5f, 0.5f);
            float y = clamp(uv.y, -0.5f, 0.5f);
            float x2 = x * x;
            float y2 = y * y;
            return (float2)(
                x * (1.2100393f + x2 * (-1.2758402f + x2 * 1.7751845f)) + (uv.x - x),
                y * (0.9364505f + (0.4465308f - 0.7683315f * y2) * y2 + (-0.3574087f + 1.1584653f * y2 + 0.3529348f * x2) * x2) + (uv.y - y)
            );
        }

        float2 digital_undistort_point(float2 uv, __global KernelParams *params) {
            float2 out_c2 = (float2)(params->output_width, params->output_height);
            uv = (uv / out_c2) - 0.5f;

            uv = superview(uv);

            uv.x = uv.x / 1.333333333f;
            uv = (uv + 0.5f) * out_c2;
            return uv;
        }
        float2 digital_distort_point(float2 uv, __global KernelParams *params) {
            float2 size = (float2)(params->width, params->height);
            float2 n = (uv / size) - 0.5f;
            float2 target = (float2)(n.x * 1.333333333f, n.y);

            float2 P = n; // seed inside the recorded domain [-0.5,0.5]
            for (int i = 0; i < 12; ++i) {
                float2 diff = superview(P) - target;
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
        fn superview(uv: vec2<f32>) -> vec2<f32> {
            // Clamp the polynomial argument to the recorded frame and continue linearly past it, so the map
            // stays smooth & monotonic everywhere (no divergence to NaN). See gopro_superview.rs
            let x = clamp(uv.x, -0.5, 0.5);
            let y = clamp(uv.y, -0.5, 0.5);
            let x2 = x * x;
            let y2 = y * y;
            return vec2<f32>(
                x * (1.2100393 + x2 * (-1.2758402 + x2 * 1.7751845)) + (uv.x - x),
                y * (0.9364505 + (0.4465308 - 0.7683315 * y2) * y2 + (-0.3574087 + 1.1584653 * y2 + 0.3529348 * x2) * x2) + (uv.y - y)
            );
        }
        fn digital_undistort_point(_uv: vec2<f32>) -> vec2<f32> {
            let out_c2 = vec2<f32>(f32(params.output_width), f32(params.output_height));
            var uv = _uv;
            uv = (uv / out_c2) - 0.5;

            uv = superview(uv);

            uv.x = uv.x / 1.333333333;
            uv = (uv + 0.5) * out_c2;

            return uv;
        }
        fn digital_distort_point(_uv: vec2<f32>) -> vec2<f32> {
            let size = vec2<f32>(f32(params.width), f32(params.height));
            let n = (_uv / size) - 0.5;
            let want = vec2<f32>(n.x * 1.333333333, n.y); // not `target`: a WGSL reserved keyword

            var P = n; // seed inside the recorded domain [-0.5,0.5]
            for (var i: i32 = 0; i < 12; i = i + 1) {
                let diff = superview(P) - want;
                if (abs(diff.x) < 1e-6 && abs(diff.y) < 1e-6) {
                    break;
                }
                P -= diff;
            }

            return (P + 0.5) * size;
        }"#
    }
}
