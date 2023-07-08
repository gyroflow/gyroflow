// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use glam::{ vec2, Vec2, Vec4 };
use super::types::*;

pub fn sample_input_at(uv: Vec2, _coeffs: &[f32], input: &ImageType, params: &KernelParams, _sampler: SamplerType) -> Vec4 {
    let max_value = if cfg!(all(target_arch = "spirv", not(feature = "texture_u32"))) { 1.0 } else { params.max_pixel_value };
    let bg = params.background * max_value;
    #[cfg(feature = "for_qtrhi")]
    {
        use spirv_std::image::{ ImageWithMethods, sample_with };
        if (uv.x >= 0.0 && uv.x < params.width as f32) && (uv.y >= 0.0 && uv.y < params.height as f32) {
            let size = vec2(params.width as f32, params.height as f32);
            input.sample_with(*_sampler, uv / size, sample_with::lod(0.0f32))
        } else {
            bg
        }
    }
    #[cfg(not(feature = "for_qtrhi"))]
    {
        const INTER_BITS: usize = 5;
        const INTER_TAB_SIZE: usize = 1 << INTER_BITS;

        let shift: i32 = (params.interpolation >> 2) + 1;
        let offset: f32 = ((params.interpolation >> 1) - 1) as f32;
        let ind: usize = [0, 64, 64 + 128][params.interpolation as usize >> 2];
        let mut uv = uv;

        if params.input_rotation != 0.0 {
            uv = rotate_point(uv, params.input_rotation * (core::f32::consts::PI / 180.0), vec2(params.width as f32 / 2.0, params.height as f32 / 2.0));
        }

        uv = vec2(
            map_coord(uv.x, 0.0, params.width  as f32, params.source_rect.x as f32, (params.source_rect.x + params.source_rect.z) as f32),
            map_coord(uv.y, 0.0, params.height as f32, params.source_rect.y as f32, (params.source_rect.y + params.source_rect.w) as f32)
        );

        let u = uv.x - offset;
        let v = uv.y - offset;

        let sx0 = fast_round(u * INTER_TAB_SIZE as f32);
        let sy0 = fast_round(v * INTER_TAB_SIZE as f32);

        let sx = sx0 >> INTER_BITS;
        let sy = sy0 >> INTER_BITS;

        let coeffs_x = ind + ((sx0 as usize & (INTER_TAB_SIZE - 1)) << shift);
        let coeffs_y = ind + ((sy0 as usize & (INTER_TAB_SIZE - 1)) << shift);

        let mut sum = Vec4::splat(0.0);
        let mut _src_index = sy as isize * params.stride as isize + sx as isize * params.bytes_per_pixel as isize;

        let mut yp = 0; while yp < params.interpolation {
        //for yp in 0..params.interpolation {
            if sy + yp >= params.source_rect.y as i32 && sy + yp < (params.source_rect.y + params.source_rect.w) as i32 {
                let mut xsum = Vec4::splat(0.0);
                let mut xp = 0; while xp < params.interpolation {
                // for xp in 0..params.interpolation {
                    let pixel = if sx + xp >= params.source_rect.x as i32 && sx + xp < (params.source_rect.x + params.source_rect.z) as i32 {
                        #[cfg(target_arch = "spirv")]
                        {
                            use spirv_std::image::{ ImageWithMethods, sample_with };
                            to_float(input.fetch_with(glam::IVec2::new((sx + xp) as i32, (sy + yp) as i32), sample_with::lod(0)))
                        }
                        #[cfg(not(target_arch = "spirv"))]
                        { input.1(&input.0[_src_index as usize + (params.bytes_per_pixel * xp) as usize.._src_index as usize + (params.bytes_per_pixel * (xp + 1)) as usize]) }
                    } else {
                        bg
                    };
                    xsum += pixel * _coeffs[coeffs_x + xp as usize];
                    xp += 1;
                    if xp >= params.interpolation { break; } // Bug in Dx12 backend, doesn't work without it for some strange reason
                }

                sum += xsum * _coeffs[coeffs_y + yp as usize];
            } else {
                sum += bg * Vec4::splat(_coeffs[coeffs_y + yp as usize]);
            }
            _src_index += params.stride as isize;
            yp += 1;
            if yp >= params.interpolation { break; } // Bug in Dx12 backend, doesn't work without it for some strange reason
        }
        glam::vec4(
            sum.x.min(max_value),
            sum.y.min(max_value),
            sum.z.min(max_value),
            sum.w.min(max_value),
        )
    }
}
