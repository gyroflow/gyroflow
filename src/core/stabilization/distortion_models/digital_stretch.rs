// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use crate::{ stabilization::KernelParams, lens_profile::LensProfile };

#[derive(Default, Clone)]
pub struct DigitalStretch { }

impl DigitalStretch {
    /// `uv` range: (0,0)...(width, height)
    /// From processed to real
    pub fn undistort_point(&self, uv: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        Some((uv.0 / params.digital_lens_params[0],
              uv.1 / params.digital_lens_params[1]))
    }

    /// `uv` range: (0,0)..(width, height)
    /// From real to processed
    pub fn distort_point(&self, x: f32, y: f32, _z: f32, params: &KernelParams) -> (f32, f32) {
        (x * params.digital_lens_params[0],
         y * params.digital_lens_params[1])
    }
    pub fn adjust_lens_profile(&self, _profile: &mut LensProfile) {
        // TODO
    }
    pub fn distortion_derivative(&self, _theta: f64, _k: &[f64]) -> Option<f64> {
        None
    }

    pub fn id()   -> &'static str { "digital_stretch" }
    pub fn name() -> &'static str { "Digital stretch" }

    pub fn opencl_functions(&self) -> &'static str {
        r#"
        float2 digital_undistort_point(float2 uv, __global KernelParams *params) {
            uv.x /= params->digital_lens_params.x;
            uv.y /= params->digital_lens_params.y;
            return uv;
        }
        float2 digital_distort_point(float2 uv, __global KernelParams *params) {
            uv.x *= params->digital_lens_params.x;
            uv.y *= params->digital_lens_params.y;
            return uv;
        }"#
    }
    pub fn wgsl_functions(&self) -> &'static str {
        r#"
        fn digital_undistort_point(uv: vec2<f32>) -> vec2<f32> {
            uv.x = uv.x / params.digital_lens_params.x;
            uv.y = uv.y / params.digital_lens_params.y;
            return uv;
        }
        fn digital_distort_point(uv: vec2<f32>) -> vec2<f32> {
            uv.x = uv.x * params.digital_lens_params.x;
            uv.y = uv.y * params.digital_lens_params.y;
            return uv;
        }"#
    }
}
