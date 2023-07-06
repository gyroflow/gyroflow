// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3 };

pub struct DigitalStretch { }

impl DigitalStretch {
    /// `uv` range: (0,0)...(width, height)
    /// From processed to real
    pub fn undistort_point(point: Vec2, params: &KernelParams) -> Vec2 {
        vec2(point.x / params.digital_lens_params.x,
             point.y / params.digital_lens_params.y)
    }

    /// `uv` range: (0,0)..(width, height)
    /// From real to processed
    pub fn distort_point(point: Vec3, params: &KernelParams) -> Vec2 {
        vec2(point.x * params.digital_lens_params.x,
             point.y * params.digital_lens_params.y)
    }

    // TODO
    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(_calib_w: &mut usize, _calib_h: &mut usize/*, lens_model: &mut String*/) { }
}
