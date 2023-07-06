// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3, vec3 };

pub struct Insta360 { }

impl Insta360 {
    pub fn undistort_point(point: Vec2, params: &KernelParams) -> Vec2 {
        let mut px = point.x;
        let mut py = point.y;

        let mut i = 0; while i < 200 {
        // for _ in 0..200 {
            let dp = Self::distort_point(vec3(px, py, 1.0), params);
            px -= dp.x - point.x;
            py -= dp.y - point.y;
            i += 1;
        }

        vec2(px, py)
    }

    pub fn distort_point(point: Vec3, params: &KernelParams) -> Vec2 {
        let k1 = params.k1.x;
        let k2 = params.k1.y;
        let k3 = params.k1.z;
        let p1 = params.k1.w;
        let p2 = params.k2.x;
        let xi = params.k2.y;

        let len = point.length();

        let x = (point.x / len) / ((point.z / len) + xi);
        let y = (point.y / len) / ((point.z / len) + xi);

        let r2 = x*x + y*y;
        let r4 = r2 * r2;
        let r6 = r4 * r2;

        vec2(
            x * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p1*x*y + p2*(r2 + 2.0*x*x),
            y * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p2*x*y + p1*(r2 + 2.0*y*y)
        )
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(_calib_w: &mut usize, _calib_h: &mut usize/*, lens_model: &mut String*/) { }
}
