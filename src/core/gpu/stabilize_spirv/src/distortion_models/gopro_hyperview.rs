// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3 };

pub struct GoProHyperview { }

impl GoProHyperview {
    /// `uv` range: (0, 0)...(width, height)
    /// From hyperview to wide
    pub fn undistort_point(mut point: Vec2, params: &KernelParams) -> Vec2 {
        let out_c2 = vec2(params.output_width as f32, params.output_height as f32);
        point = (point / out_c2) - 0.5;

        point.x *= 1.0 - 0.64 * point.x.abs();
        point.x *= 1.0101 * (1.0 - 0.0294118 * point.x.abs());
        point.y *= 1.0101 * (1.0 - 0.0200000 * point.y.abs());

        (point + 0.5) * out_c2
    }

    /// `uv` range: (0, 0)...(width, height)
    /// From wide to hyperview
    pub fn distort_point(point: Vec3, params: &KernelParams) -> Vec2 {
        let mut point = vec2(point.x, point.y);
        let size = vec2(params.width as f32, params.height as f32);

        point = (point / size) - 0.5;

        let xs = if point.x < 0.0 { -1.0 } else { 1.0 };
        let ys = if point.y < 0.0 { -1.0 } else { 1.0 };

        point.y = ys * (-25.0 * ((1.0 - 0.0792 * point.y.abs()).sqrt() - 1.0));
        point.x = xs * (-25.0 * (0.824621 * (0.68 - 0.0792 * point.x.abs()).sqrt() - 0.68));
        point.x = xs * (-0.78125 * ((1.0 - 2.56 * point.x.abs()).sqrt() - 1.0));

        (point + 0.5) * size
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(calib_w: &mut usize, calib_h: &mut usize/*, lens_model: &mut String*/) {
        let aspect = (*calib_w as f64 / *calib_h as f64 * 100.0) as usize;
        if aspect == 114 { // It's 8:7
            *calib_w = (*calib_w as f64 * 1.55555555555).round() as usize;
        }
        // *lens_model = "Hyperview".into();
    }
}
