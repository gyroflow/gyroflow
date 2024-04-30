// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use glam::{ Vec2, vec4, Vec4 };
use super::types::*;

const COLORS: [Vec4; 9] = [
    vec4(0.0   / 255.0, 0.0   / 255.0, 0.0    / 255.0, 0.0   / 255.0), // None
    vec4(255.0 / 255.0, 0.0   / 255.0, 0.0    / 255.0, 255.0 / 255.0), // Red
    vec4(0.0   / 255.0, 255.0 / 255.0, 0.0    / 255.0, 255.0 / 255.0), // Green
    vec4(0.0   / 255.0, 0.0   / 255.0, 255.0  / 255.0, 255.0 / 255.0), // Blue
    vec4(254.0 / 255.0, 251.0 / 255.0, 71.0   / 255.0, 255.0 / 255.0), // Yellow
    vec4(200.0 / 255.0, 200.0 / 255.0, 0.0    / 255.0, 255.0 / 255.0), // Yellow2
    vec4(255.0 / 255.0, 0.0   / 255.0, 255.0  / 255.0, 255.0 / 255.0), // Magenta
    vec4(0.0   / 255.0, 128.0 / 255.0, 255.0  / 255.0, 255.0 / 255.0), // Blue2
    vec4(0.0   / 255.0, 200.0 / 255.0, 200.0  / 255.0, 255.0 / 255.0)  // Blue3
];
const ALPHAS: [f32; 4] = [1.0, 0.75, 0.50, 0.25];

pub fn draw_pixel(in_pix: Vec4, x: f32, y: f32, is_input: bool, params: &KernelParams, _coeffs: &[f32], drawing: &DrawingType, _sampler: SamplerType, max_value: f32) -> Vec4 {
    let width = (params.width as f32).max(params.output_width as f32);

    #[cfg(feature = "for_qtrhi")]
    let data = {
        use spirv_std::image::{ ImageWithMethods, sample_with };
        let height = (params.height as f32).max(params.output_height as f32);
        (drawing.sample_with(*_sampler, glam::vec2(x / width, y / height), sample_with::lod(0.0f32)).x * 255.0).ceil() as u32
    };
    #[cfg(not(feature = "for_qtrhi"))]
    let data = {
        let pos_byte = fast_round(fast_floor(y / params.canvas_scale) as f32 * (width as f32 / params.canvas_scale) + fast_floor(x / params.canvas_scale) as f32) as usize;
        let pos_u32 = pos_byte / 4;
        let u32_offset = pos_byte - (pos_u32 * 4);
        (drawing[pos_u32] >> ((u32_offset) * 8)) & 0xFF
    };

    let mut pix = in_pix;
    if data > 0 {
        let color = ((data & 0xF8) >> 3) as usize;
        let alpha = ((data & 0x06) >> 1) as usize;
        let stage = data & 1;
        if ((stage == 0 && is_input) || (stage == 1 && !is_input)) && color < 9 {
            let colorf = COLORS[color] * max_value;
            let alphaf = ALPHAS[alpha];
            pix = colorf * alphaf + pix * (1.0 - alphaf);
            pix.w = colorf.w;
        }
    }
    pix
}
pub fn draw_safe_area(in_pix: Vec4, x: f32, y: f32, params: &KernelParams) -> Vec4 {
    let mut pix = in_pix;
    let is_safe_area = x >= params.safe_area_rect.x && x <= params.safe_area_rect.z &&
                       y >= params.safe_area_rect.y && y <= params.safe_area_rect.w;
    if !is_safe_area {
        pix.x *= 0.5;
        pix.y *= 0.5;
        pix.z *= 0.5;
        let is_border = x > params.safe_area_rect.x - 5.0 && x < params.safe_area_rect.z + 5.0 &&
                        y > params.safe_area_rect.y - 5.0 && y < params.safe_area_rect.w + 5.0;
        if is_border {
            pix.x *= 0.5;
            pix.y *= 0.5;
            pix.z *= 0.5;
        }
    }
    pix
}

// From 0-255(JPEG/Full) to 16-235(MPEG/Limited)
fn remap_colorrange(px: Vec4, is_y: bool, max_value: f32) -> Vec4 {
    if is_y { ((16.0 / 255.0) * max_value) + (px * ((235.0 - 16.0) / 255.0)) }
    else    { ((16.0 / 255.0) * max_value) + (px * ((240.0 - 16.0) / 255.0)) }
}

pub fn process_final_pixel(mut pixel: Vec4, src_pos: Vec2, out_pos: Vec2, params: &KernelParams, coeffs: &[f32], drawing: &DrawingType, sampler: SamplerType, flags: u32) -> Vec4 {
    if (flags & 1) == 1 {
        pixel = remap_colorrange(pixel, params.bytes_per_pixel == 1, params.max_pixel_value);
    }

    #[cfg(feature="for_qtrhi")]
    let drawing_enabled = (flags & 8) == 8;
    #[cfg(not(feature="for_qtrhi"))]
    let drawing_enabled = (flags & 8) == 8 && !drawing.is_empty();

    if drawing_enabled {
        if src_pos.y >= params.source_rect.y as f32 && src_pos.y < (params.source_rect.y + params.source_rect.w) as f32 {
            if src_pos.x >= params.source_rect.x as f32 && src_pos.x < (params.source_rect.x + params.source_rect.z) as f32 {
                pixel = draw_pixel(pixel, src_pos.x, src_pos.y, true, params, coeffs, drawing, sampler, params.max_pixel_value);
            }
        }
        pixel = draw_pixel(pixel, out_pos.x, out_pos.y, false, params, coeffs, drawing, sampler, params.max_pixel_value);
        pixel = draw_safe_area(pixel, out_pos.x, out_pos.y, params);
    }
    pixel
}
