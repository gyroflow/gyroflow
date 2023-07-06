// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// See https://github.com/gyroflow/gyroflow/issues/43 for research details

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3 };

pub struct GoProSuperview { }

impl GoProSuperview {
    /// `uv` range: (0, 0)...(width, height)
    /// From superview to wide
    pub fn undistort_point(mut point: Vec2, params: &KernelParams) -> Vec2 {
        let out_c2 = vec2(params.output_width as f32, params.output_height as f32);
        point = (point / out_c2) - 0.5;

        point.x *= 1.0 - 0.45 * point.x.abs();
        point.x *= 0.168827 * (5.53572 + point.x.abs());
        point.y *= 0.130841 * (7.14285 + point.y.abs());

        (point + 0.5) * out_c2
    }

    /// `uv` range: (0, 0)...(width, height)
    /// From wide to superview
    pub fn distort_point(point: Vec3, params: &KernelParams) -> Vec2 {
        let mut point = vec2(point.x, point.y);
        let size = vec2(params.width as f32, params.height as f32);

        point = (point / size) - 0.5;

        let xs = if point.x < 0.0 { -1.0 } else { 1.0 };
        let ys = if point.y < 0.0 { -1.0 } else { 1.0 };

        point.y = ys * (3.57143 * ((0.5992 * point.y.abs() + 1.0).sqrt() - 1.0));
        point.x = xs * (3.57143 * (0.880341 * (0.5992 * point.x.abs() + 0.775).sqrt() - 0.775));
        point.x = xs * (-1.11111 * ((1.0 - 1.8 * point.x.abs()).sqrt() - 1.0));

        (point + 0.5) * size
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(calib_w: &mut usize, calib_h: &mut usize/*, lens_model: &mut String*/) {
        let aspect = (*calib_w as f64 / *calib_h as f64 * 100.0) as usize;
        if aspect == 133 { // It's 4:3
            *calib_w = (*calib_w as f64 * 1.3333333333333).round() as usize;
        }
        // *lens_model = "Superview".into();
    }
}
