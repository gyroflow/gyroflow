// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use glam::{ vec2, Vec2, Vec4 };
use super::types::*;
use super::interpolate::*;

pub fn sample_with_background_at(mut uv: Vec2, coeffs: &[f32], input: &ImageType, params: &KernelParams, sampler: SamplerType) -> Vec4 {
    let width_f = params.width as f32;
    let height_f = params.height as f32;
    match params.background_mode {
        1 => { // Edge repeat
            uv = vec2(
                uv.x.max(0.0).min(width_f  - 1.0),
                uv.y.max(0.0).min(height_f - 1.0),
            );
            sample_input_at(uv, coeffs, input, params, sampler)
        },
        2 => { // Edge mirror
            let rx = fast_round(uv.x) as f32;
            let ry = fast_round(uv.y) as f32;
            let width3 = width_f - 3.0;
            let height3 = height_f - 3.0;
            if rx > width3  { uv.x = width3  - (rx - width3); }
            if rx < 3.0     { uv.x = 3.0 + width_f - (width3  + rx); }
            if ry > height3 { uv.y = height3 - (ry - height3); }
            if ry < 3.0     { uv.y = 3.0 + height_f - (height3 + ry); }
            sample_input_at(uv, coeffs, input, params, sampler)
        },
        3 => { // Margin with feather
            let size = vec2(width_f - 1.0, height_f - 1.0);

            let feather = (params.background_margin_feather * size.y).max(0.0001);
            let mut pt2 = uv;
            let mut alpha = 1.0;
            if (uv.x > size.x - feather) || (uv.x < feather) || (uv.y > size.y - feather) || (uv.y < feather) {
                alpha = ((size.x - uv.x).min(size.y - uv.y).min(uv.x).min(uv.y) / feather).min(1.0).max(0.0);
                pt2 = ((((pt2 / size) - 0.5) * (1.0 - params.background_margin)) + 0.5) * size;
            }

            let c1 = sample_input_at(uv, coeffs, input, params, sampler);
            let c2 = sample_input_at(pt2, coeffs, input, params, sampler);
            c1 * alpha + c2 * (1.0 - alpha)
        },
        _ => { sample_input_at(uv, coeffs, input, params, sampler) }
    }
}
