// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>
//
// GoPro Superview/Hyperview digital warp, data-driven from the camera's in-camera MAPX/MAPY
// calibration (gopro/gpmf-parser#190) — the companion digital lens to the `gopro` radial model.
// Unlike the hardcoded gopro_superview/gopro_hyperview lenses, the warp polynomials come from
// `digital_lens_params`, so any current or future GoPro lens mode works without hardcoding.
//
// digital_lens_params layout (16 floats == 4 vec4):
//   [0..8)  MAPX coeffs c0..c7 -> new_x = x*(c0 + c1*x² + c2*x⁴ + c3*x⁶ + c4*x⁸ + c5*x¹⁰ + c6*x¹²) + c7*x*y²
//   [8..14) MAPY coeffs d0..d5 -> new_y = y*(d0 + d1*y² + d2*y⁴ + d3*x² + d4*y²*x² + d5*x⁴)
//   [14]    factor = ARWA/ARUW (Superview 1.3333, Hyperview 1.5556, Wide 1.0)
//   [15]    unused
// gopro_map is the recorded(warped) -> wide(sensor) map (direct eval in undistort, Newton-invert in distort).

use crate::{ stabilization::KernelParams, lens_profile::LensProfile };

#[derive(Default, Clone)]
pub struct GoProWarp { }

impl GoProWarp {
    fn gopro_map(uv: (f32, f32), p: &[f32; 16]) -> (f32, f32) {
        // The MAPX/MAPY polynomials are only valid/monotonic inside the recorded frame [-0.5, 0.5]; beyond it
        // the degree-12 MAPX oscillates and folds back (the FOV-overview edge swirl) and makes the Newton
        // inversions in distort_point / undistort_points diverge. Clamp the polynomial argument to the valid
        // domain and continue linearly (slope 1) past it, so the map stays smooth and strictly monotonic
        // everywhere — identical to the raw polynomial in-domain, and beyond the frame it grows outward so the
        // source coordinate cleanly leaves [0, w] (background) instead of folding back in.
        let x = uv.0.clamp(-0.5, 0.5);
        let y = uv.1.clamp(-0.5, 0.5);
        let x2 = x * x;
        let y2 = y * y;
        let poly_x = p[0] + x2 * (p[1] + x2 * (p[2] + x2 * (p[3] + x2 * (p[4] + x2 * (p[5] + x2 * p[6])))));
        (
            x * (poly_x + p[7] * y2) + (uv.0 - x),
            y * (p[8] + p[9] * y2 + p[10] * y2 * y2 + x2 * (p[11] + p[12] * y2 + p[13] * x2)) + (uv.1 - y)
        )
    }

    /// `uv` range: (0,0)...(width, height)
    /// From recorded (GoPro warped) to wide
    pub fn undistort_point(&self, mut uv: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        let p = &params.digital_lens_params;
        let factor = if p[14] != 0.0 { p[14] } else { 1.0 };
        let out_c2 = (params.output_width as f32, params.output_height as f32);
        uv = ((uv.0 / out_c2.0) - 0.5,
              (uv.1 / out_c2.1) - 0.5);

        uv = Self::gopro_map(uv, p);

        uv.0 = uv.0 / factor;

        Some(((uv.0 + 0.5) * out_c2.0,
              (uv.1 + 0.5) * out_c2.1))
    }

    /// `uv` range: (0,0)...(width, height)
    /// From wide to recorded (GoPro warped)
    pub fn distort_point(&self, mut x: f32, mut y: f32, _z: f32, params: &KernelParams) -> (f32, f32) {
        let p = &params.digital_lens_params;
        let factor = if p[14] != 0.0 { p[14] } else { 1.0 };
        let size = (params.width as f32, params.height as f32);
        x = (x / size.0) - 0.5;
        y = (y / size.1) - 0.5;

        // Solve gopro_map(pp) = target, where target is the wide coord (x stretched by `factor`).
        // Start the iteration at the UN-stretched normalized coord: it's inside the recorded domain
        // [-0.5, 0.5] (where the MAPX polynomial is valid) and already ≈ the solution since
        // gopro_map(x).x ≈ x·c0 ≈ x·factor. Starting at `target` (up to ±0.5·factor) lands outside
        // [-0.5, 0.5] for Hyperview (factor ≈ 1.556 → ±0.78), where the high-order MAPX polynomial
        // explodes and the fixed-point iteration diverges.
        let target = (x * factor, y);
        let mut pp = (x, y);
        for _ in 0..12 {
            let dp = Self::gopro_map(pp, p);
            let diff = (dp.0 - target.0, dp.1 - target.1);
            if diff.0.abs() < 1e-6 && diff.1.abs() < 1e-6 {
                break;
            }
            pp.0 -= diff.0;
            pp.1 -= diff.1;
        }

        // Reject out-of-domain (beyond the recorded frame): there's no valid inverse there and the high-order
        // MAPX polynomial oscillates — which renders as a swirl in the zoomed-out FOV overview. If the iteration
        // didn't actually land on `target`, return an off-frame sentinel so it samples background instead.
        let res = Self::gopro_map(pp, p);
        if (res.0 - target.0).abs() > 0.02 || (res.1 - target.1).abs() > 0.02 {
            return (-99999.0, -99999.0);
        }

        ((pp.0 + 0.5) * size.0,
         (pp.1 + 0.5) * size.1)
    }
    pub fn adjust_lens_profile(&self, _profile: &mut LensProfile) {
        // No-op: telemetry-parser emits a fully-resolved profile (calib_dimension = VRES, 16:9 output).
    }
    pub fn distortion_derivative(&self, _theta: f64, _k: &[f64]) -> Option<f64> {
        None
    }

    pub fn id()   -> &'static str { "gopro_warp" }
    pub fn name() -> &'static str { "GoPro warp" }

    pub fn opencl_functions(&self) -> &'static str {
        r#"
        float2 gopro_map(float2 uv, __global KernelParams *params) {
            // Clamp the polynomial argument to the valid recorded-frame domain [-0.5,0.5] and continue linearly
            // past it, keeping the map smooth & monotonic everywhere (no edge-swirl / Newton divergence). See gopro_warp.rs.
            float x = clamp(uv.x, -0.5f, 0.5f);
            float y = clamp(uv.y, -0.5f, 0.5f);
            float x2 = x * x;
            float y2 = y * y;
            float c0 = params->digital_lens_params[0].x, c1 = params->digital_lens_params[0].y, c2 = params->digital_lens_params[0].z, c3 = params->digital_lens_params[0].w;
            float c4 = params->digital_lens_params[1].x, c5 = params->digital_lens_params[1].y, c6 = params->digital_lens_params[1].z, c7 = params->digital_lens_params[1].w;
            float d0 = params->digital_lens_params[2].x, d1 = params->digital_lens_params[2].y, d2 = params->digital_lens_params[2].z, d3 = params->digital_lens_params[2].w;
            float d4 = params->digital_lens_params[3].x, d5 = params->digital_lens_params[3].y;
            float poly_x = c0 + x2 * (c1 + x2 * (c2 + x2 * (c3 + x2 * (c4 + x2 * (c5 + x2 * c6)))));
            return (float2)(
                x * (poly_x + c7 * y2) + (uv.x - x),
                y * (d0 + d1 * y2 + d2 * y2 * y2 + x2 * (d3 + d4 * y2 + d5 * x2)) + (uv.y - y)
            );
        }
        float2 digital_undistort_point(float2 uv, __global KernelParams *params) {
            float factor = params->digital_lens_params[3].z; if (factor == 0.0f) { factor = 1.0f; }
            float2 out_c2 = (float2)(params->output_width, params->output_height);
            uv = (uv / out_c2) - 0.5f;
            uv = gopro_map(uv, params);
            uv.x = uv.x / factor;
            uv = (uv + 0.5f) * out_c2;
            return uv;
        }
        float2 digital_distort_point(float2 uv, __global KernelParams *params) {
            float factor = params->digital_lens_params[3].z; if (factor == 0.0f) { factor = 1.0f; }
            float2 size = (float2)(params->width, params->height);
            float2 n = (uv / size) - 0.5f;
            float2 target = (float2)(n.x * factor, n.y);
            float2 P = n; // seed inside the recorded domain [-0.5,0.5]
            for (int i = 0; i < 12; ++i) {
                float2 diff = gopro_map(P, params) - target;
                if (fabs(diff.x) < 1e-6f && fabs(diff.y) < 1e-6f) { break; }
                P -= diff;
            }
            float2 res = gopro_map(P, params) - target; // reject out-of-domain (beyond recorded frame) -> background
            if (fabs(res.x) > 0.02f || fabs(res.y) > 0.02f) { return (float2)(-99999.0f, -99999.0f); }
            return (P + 0.5f) * size;
        }"#
    }
    pub fn wgsl_functions(&self) -> &'static str {
        r#"
        fn gopro_map(uv: vec2<f32>) -> vec2<f32> {
            // Clamp the polynomial argument to the valid recorded-frame domain [-0.5,0.5] and continue linearly
            // past it, keeping the map smooth & monotonic everywhere (no edge-swirl / Newton divergence). See gopro_warp.rs.
            let x = clamp(uv.x, -0.5, 0.5);
            let y = clamp(uv.y, -0.5, 0.5);
            let x2 = x * x;
            let y2 = y * y;
            let c0 = params.digital_lens_params[0].x; let c1 = params.digital_lens_params[0].y; let c2 = params.digital_lens_params[0].z; let c3 = params.digital_lens_params[0].w;
            let c4 = params.digital_lens_params[1].x; let c5 = params.digital_lens_params[1].y; let c6 = params.digital_lens_params[1].z; let c7 = params.digital_lens_params[1].w;
            let d0 = params.digital_lens_params[2].x; let d1 = params.digital_lens_params[2].y; let d2 = params.digital_lens_params[2].z; let d3 = params.digital_lens_params[2].w;
            let d4 = params.digital_lens_params[3].x; let d5 = params.digital_lens_params[3].y;
            let poly_x = c0 + x2 * (c1 + x2 * (c2 + x2 * (c3 + x2 * (c4 + x2 * (c5 + x2 * c6)))));
            return vec2<f32>(
                x * (poly_x + c7 * y2) + (uv.x - x),
                y * (d0 + d1 * y2 + d2 * y2 * y2 + x2 * (d3 + d4 * y2 + d5 * x2)) + (uv.y - y)
            );
        }
        fn digital_undistort_point(_uv: vec2<f32>) -> vec2<f32> {
            var factor = params.digital_lens_params[3].z; if (factor == 0.0) { factor = 1.0; }
            let out_c2 = vec2<f32>(f32(params.output_width), f32(params.output_height));
            var uv = _uv;
            uv = (uv / out_c2) - 0.5;
            uv = gopro_map(uv);
            uv.x = uv.x / factor;
            uv = (uv + 0.5) * out_c2;
            return uv;
        }
        fn digital_distort_point(_uv: vec2<f32>) -> vec2<f32> {
            var factor = params.digital_lens_params[3].z; if (factor == 0.0) { factor = 1.0; }
            let size = vec2<f32>(f32(params.width), f32(params.height));
            let n = (_uv / size) - 0.5;
            let target = vec2<f32>(n.x * factor, n.y);
            var P = n; // seed inside the recorded domain [-0.5,0.5]
            for (var i: i32 = 0; i < 12; i = i + 1) {
                let diff = gopro_map(P) - target;
                if (abs(diff.x) < 1e-6 && abs(diff.y) < 1e-6) { break; }
                P -= diff;
            }
            let res = gopro_map(P) - target; // reject out-of-domain (beyond recorded frame) -> background
            if (abs(res.x) > 0.02 || abs(res.y) > 0.02) { return vec2<f32>(-99999.0, -99999.0); }
            return (P + 0.5) * size;
        }"#
    }
}
