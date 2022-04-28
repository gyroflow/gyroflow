// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// https://github.com/gyroflow/gyroflow/issues/43

#[derive(Default, Clone)]
pub struct GoProSuperview { }

impl GoProSuperview {
    pub const ASPECT_SCALE: f32 = 1.33333333;
    pub const HORIZONTAL_SCALE: f32 = 1.46;
    
    /// `pt` range: [-0.5, 0.5]
    pub fn from_superview(mut pt: (f64, f64)) -> (f64, f64) {
        pt.0 *= 1.0 - 0.45 * pt.0.abs(); 
        pt.0 *= 0.168827 * (5.53572 + pt.0.abs());
        pt.1 *= 0.130841 * (7.14285 + pt.1.abs());

        pt
    }

    /// `pt` range: [-0.5, 0.5]
    pub fn to_superview(mut pt: (f32, f32)) -> (f32, f32) {
        pt.1 = (3.57143 - 0.5 * (51.0203 + 30.5714 * pt.1.abs()).sqrt()) * (-pt.1 / pt.1.abs().max(0.000001));
        pt.0 = (2.76785 - 0.5 * (30.6441 + 23.6928 * pt.0.abs()).sqrt()) * (-pt.0 / pt.0.abs().max(0.000001));
        pt.0 = (1.11111 - 0.5 * (4.93827 - 8.88889 * pt.0.abs()).sqrt()) * ( pt.0 / pt.0.abs().max(0.000001));

        pt
    }

    /// `pt` range: [-1.0, 1.0]
    pub fn transform_point_from_superview(pt: &mut (f64, f64)) {
        //pt.0 /= GoProSuperview::HORIZONTAL_SCALE as f64;
        pt.0 /= 2.0; pt.1 /= 2.0;

        let pt2 = Self::from_superview(*pt);

        pt.0 = pt2.0 * 2.0;// * GoProSuperview::HORIZONTAL_SCALE as f64;
        pt.1 = pt2.1 * 2.0;
    }

    /// `pt` range: [0, 1]
    pub fn from_superview_calib(pt: (f32, f32)) -> (f32, f32) {
        let mut pt2 = Self::from_superview((pt.0 as f64 - 0.5, pt.1 as f64 - 0.5));

        // Move from center to the left, because we trim the right part making it 4:3
        pt2.0 -= 0.125; // (3840 - 2880) / 2 / 3840

        (
            (pt2.0 as f32 + 0.5), 
            (pt2.1 as f32 + 0.5)
        )
    }

    pub fn opencl_functions() -> &'static str {
        r#"
        float2 from_superview(float2 uv) {
            uv.x *= 1.0f - 0.45f * fabs(uv.x); 
            uv.x *= 0.168827f * (5.53572f + fabs(uv.x));
            uv.y *= 0.130841f * (7.14285f + fabs(uv.y));

            return uv;
        }
        float2 to_superview(float2 uv) {            
            uv.y = (3.57143f - 0.5f * sqrt(51.0203f + 30.5714f * fabs(uv.y))) * (-uv.y / fmax(0.000001f, fabs(uv.y)));
            uv.x = (2.76785f - 0.5f * sqrt(30.6441f + 23.6928f * fabs(uv.x))) * (-uv.x / fmax(0.000001f, fabs(uv.x)));
            uv.x = (1.11111f - 0.5f * sqrt(4.93827f - 8.88889f * fabs(uv.x))) * ( uv.x / fmax(0.000001f, fabs(uv.x)));

            return uv;
        }"#
    }
    pub fn wgsl_functions() -> &'static str { 
        r#"
        fn from_superview(uv: vec2<f32>) -> vec2<f32> {
            var uv = uv;

            uv.x *= 1.0 - 0.45 * abs(uv.x); 
            uv.x *= 0.168827 * (5.53572 + abs(uv.x));
            uv.y *= 0.130841 * (7.14285 + abs(uv.y));

            return uv;
        }
        fn to_superview(uv: vec2<f32>) -> vec2<f32> {
            var uv = uv;
            
            uv.y = (3.57143 - 0.5 * sqrt(51.0203 + 30.5714 * abs(uv.y))) * (-uv.y / max(0.000001, abs(uv.y)));
            uv.x = (2.76785 - 0.5 * sqrt(30.6441 + 23.6928 * abs(uv.x))) * (-uv.x / max(0.000001, abs(uv.x)));
            uv.x = (1.11111 - 0.5 * sqrt(4.93827 - 8.88889 * abs(uv.x))) * ( uv.x / max(0.000001, abs(uv.x)));
            
            return uv;
        }"#
    }
}
