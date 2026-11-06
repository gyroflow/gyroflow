// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>
//
// GoPro Superview/Hyperview digital warp, data-driven from MAPX/MAPY in
// params.digital_lens_params[1..4]. Companion digital lens to the `gopro` radial model.

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3 };

pub struct GoProWarp { }

impl GoProWarp {
    fn gopro_map(uv: Vec2, params: &KernelParams) -> Vec2 {
        let x = uv.x;
        let y = uv.y;
        let x2 = x * x;
        let y2 = y * y;
        let c0 = params.digital_lens_params.x;  let c1 = params.digital_lens_params.y;  let c2 = params.digital_lens_params.z;  let c3 = params.digital_lens_params.w;
        let c4 = params.digital_lens_params2.x; let c5 = params.digital_lens_params2.y; let c6 = params.digital_lens_params2.z; let c7 = params.digital_lens_params2.w;
        let d0 = params.digital_lens_params3.x; let d1 = params.digital_lens_params3.y; let d2 = params.digital_lens_params3.z; let d3 = params.digital_lens_params3.w;
        let d4 = params.digital_lens_params4.x; let d5 = params.digital_lens_params4.y;
        let poly_x = c0 + x2 * (c1 + x2 * (c2 + x2 * (c3 + x2 * (c4 + x2 * (c5 + x2 * c6)))));
        vec2(
            x * (poly_x + c7 * y2),
            y * (d0 + d1 * y2 + d2 * y2 * y2 + x2 * (d3 + d4 * y2 + d5 * x2))
        )
    }

    /// From recorded (GoPro warped) to wide
    pub fn undistort_point(mut point: Vec2, params: &KernelParams) -> Vec2 {
        let mut factor = params.digital_lens_params4.z;
        if factor == 0.0 { factor = 1.0; }
        let out_c2 = vec2(params.output_width as f32, params.output_height as f32);
        point = (point / out_c2) - 0.5;
        point = Self::gopro_map(point, params);
        point.x = point.x / factor;
        (point + 0.5) * out_c2
    }

    /// From wide to recorded (GoPro warped)
    pub fn distort_point(point: Vec3, params: &KernelParams) -> Vec2 {
        let mut factor = params.digital_lens_params4.z;
        if factor == 0.0 { factor = 1.0; }
        let size = vec2(params.width as f32, params.height as f32);
        let mut uv = (vec2(point.x, point.y) / size) - 0.5;
        uv.x = uv.x * factor;
        let mut p = uv;
        for _ in 0..12 {
            let diff = Self::gopro_map(p, params) - uv;
            if diff.x.abs() < 1e-6 && diff.y.abs() < 1e-6 { break; }
            p -= diff;
        }
        (p + 0.5) * size
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(_calib_w: &mut usize, _calib_h: &mut usize/*, lens_model: &mut String*/) { }
}
