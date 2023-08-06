// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use glam::{ vec2, Vec2, vec3, Vec4 };
use super::drawing::*;
use super::types::*;
use super::lens::*;
use super::background::*;

#[inline(never)]
fn get_mtrx_param(_size_for_rs: f32, matrices: &MatricesType, _sampler: SamplerType, row: i32, idx: usize) -> f32 {
    #[cfg(not(feature = "for_qtrhi"))]
    { matrices[row as usize * 12 + idx] }
    #[cfg(feature = "for_qtrhi")]
    {
        use spirv_std::image::{ ImageWithMethods, sample_with };
        matrices.sample_with(*_sampler, vec2(idx as f32 / 11.0, row as f32 / (_size_for_rs - 1.0)), sample_with::lod(0.0f32)).x
    }
}

pub fn rotate_and_distort(pos: Vec2, idx: i32, params: &KernelParams, matrices: &MatricesType, sampler: SamplerType) -> Vec2 {
    let size_for_rs = if (params.flags & 16) == 16 { params.width as f32 } else { params.height as f32 };
    let point_3d = vec3(
        (pos.x * get_mtrx_param(size_for_rs, matrices, sampler, idx, 0)) + (pos.y * get_mtrx_param(size_for_rs, matrices, sampler, idx, 1)) + get_mtrx_param(size_for_rs, matrices, sampler, idx, 2) + params.translation3d.x,
        (pos.x * get_mtrx_param(size_for_rs, matrices, sampler, idx, 3)) + (pos.y * get_mtrx_param(size_for_rs, matrices, sampler, idx, 4)) + get_mtrx_param(size_for_rs, matrices, sampler, idx, 5) + params.translation3d.y,
        (pos.x * get_mtrx_param(size_for_rs, matrices, sampler, idx, 6)) + (pos.y * get_mtrx_param(size_for_rs, matrices, sampler, idx, 7)) + get_mtrx_param(size_for_rs, matrices, sampler, idx, 8) + params.translation3d.z
    );
    if point_3d.z > 0.0 {
        if params.r_limit > 0.0 && vec2(point_3d.x / point_3d.z, point_3d.y / point_3d.z).length_squared() > params.r_limit.powi(2) {
            return vec2(-99999.0, -99999.0);
        }
        let mut uv = params.f * lens_distort(point_3d, params) + params.c;

        if (params.flags & 2) == 2 { // Has digital lens
            uv = digital_lens_distort(vec3(uv.x, uv.y, 1.0), params);
        }

        if params.input_horizontal_stretch > 0.001 { uv.x /= params.input_horizontal_stretch; }
        if params.input_vertical_stretch   > 0.001 { uv.y /= params.input_vertical_stretch; }

        return uv;
    }
    vec2(-99999.0, -99999.0)
}

pub fn undistort(uv: Vec2, params: &KernelParams, matrices: &MatricesType, coeffs: &[f32], _lens_data: &[f32], drawing: &DrawingType, input: &ImageType, sampler: SamplerType) -> Vec4 {
    let bg = params.background * params.max_pixel_value;

    if (params.flags & 4) == 4 { // Fill with background
        return bg;
    }

    let mut out_pos = vec2(
        map_coord(uv.x as f32, params.output_rect.x as f32, (params.output_rect.x + params.output_rect.z) as f32, 0.0, params.output_width as f32 ),
        map_coord(uv.y as f32, params.output_rect.y as f32, (params.output_rect.y + params.output_rect.w) as f32, 0.0, params.output_height as f32)
    );

    if out_pos.x < 0.0 || out_pos.y < 0.0 || out_pos.x > params.output_width as f32 || out_pos.y > params.output_height as f32 { return bg; }

    let org_out_pos = out_pos;
    out_pos = out_pos + params.translation2d;

    ///////////////////////////////////////////////////////////////////
    // Add lens distortion back
    if params.lens_correction_amount < 1.0 {
        let factor = (1.0 - params.lens_correction_amount).max(0.001); // FIXME: this is close but wrong
        let out_c = vec2(params.output_width as f32 / 2.0, params.output_height as f32 / 2.0);
        let out_f = params.f / params.fov / factor;
        let mut new_out_pos = out_pos;

        if (params.flags & 2) == 2 { // Has digial lens
            let pt = digital_lens_undistort(new_out_pos, params);
            if pt.x > -99998.0 {
                new_out_pos = pt;
            }
        }

        new_out_pos = (new_out_pos - out_c) / out_f;
        new_out_pos = lens_undistort(new_out_pos, params);
        new_out_pos = new_out_pos * out_f + out_c;

        out_pos = new_out_pos * (1.0 - params.lens_correction_amount) + (out_pos * params.lens_correction_amount);
    }
    ///////////////////////////////////////////////////////////////////

    ///////////////////////////////////////////////////////////////////
    // Calculate source `y` for rolling shutter
    let mut sy = if (params.flags & 16) == 16 { // Horizontal RS
        (fast_round(out_pos.x) as f32).min(params.width as f32).max(0.0)
    } else {
        (fast_round(out_pos.y) as f32).min(params.height as f32).max(0.0)
    };
    if params.matrix_count > 1 {
        let idx = params.matrix_count / 2;
        let pt = rotate_and_distort(out_pos, idx, params, matrices, sampler);
        if pt.x > -99998.0 {
            if (params.flags & 16) == 16 { // Horizontal RS
                sy = (fast_round(pt.x) as f32).min(params.width as f32).max(0.0);
            } else {
                sy = (fast_round(pt.y) as f32).min(params.height as f32).max(0.0);
            }
        }
    }
    ///////////////////////////////////////////////////////////////////

    let mut pixel = bg;

    let idx = sy.min(params.matrix_count as f32 - 1.0) as i32;
    let uv = rotate_and_distort(out_pos, idx, params, matrices, sampler);
    if uv.x > -99998.0 {
        pixel = sample_with_background_at(uv, coeffs, input, params, sampler);
    }
    pixel = process_final_pixel(pixel, uv, org_out_pos, params, coeffs, drawing, sampler);

    pixel
}
