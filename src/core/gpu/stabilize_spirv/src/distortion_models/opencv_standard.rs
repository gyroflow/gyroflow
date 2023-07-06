// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Adapted from OpenCV: https://github.com/opencv/opencv/blob/c3cbd302cbfbaefdef9a011b2615f8d8f58556dd/modules/calib3d/src/undistort.dispatch.cpp#L491-L538

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3 };
pub struct OpenCVStandard { }

impl OpenCVStandard {
    pub fn undistort_point(point: Vec2, params: &KernelParams) -> Vec2 {
        let (mut x, mut y) = (point.x, point.y);
        let (x0, y0) = (point.x, point.y);

        // compensate distortion iteratively
        let mut i = 0; while i < 20 {
        // for _ in 0..20 {
            let r2 = x * x + y * y;
            let icdist = (1.0 + ((params.k2.w * r2 + params.k2.z) * r2 + params.k2.y) * r2) / (1.0 + ((params.k2.x * r2 + params.k1.y) * r2 + params.k1.x) * r2);
            if icdist < 0.0 {
                // log::warn!("icdist < 0");
                return vec2(-99999.0, -99999.0);
            }
            let delta_x = 2.0 * params.k1.z * x * y + params.k1.w * (r2 + 2.0 * x * x) + params.k3.x  * r2 + params.k3.y  * r2 * r2;
            let delta_y = params.k1.z * (r2 + 2.0 * y * y) + 2.0 * params.k1.w * x * y + params.k3.z * r2 + params.k3.w * r2 * r2;
            x = (x0 - delta_x) * icdist;
            y = (y0 - delta_y) * icdist;
            i += 1;
        }

        vec2(x, y)
    }

    pub fn distort_point(point: Vec3, params: &KernelParams) -> Vec2 {
        let x = point.x / point.z;
        let y = point.y / point.z;
        let r2 = x * x + y * y;
        let r4 = r2 * r2;
        let r6 = r4 * r2;
        let a1 = 2.0 * x * y;
        let a2 = r2 + 2.0 * x * x;
        let a3 = r2 + 2.0 * y * y;
        let cdist = 1.0 + params.k1.x * r2 + params.k1.y * r4 + params.k2.x * r6;
        let icdist2 = 1.0 / (1.0 + params.k2.y * r2 + params.k2.z * r4 + params.k2.w * r6);
        let xd0 = x * cdist * icdist2 + params.k1.z * a1 + params.k1.w * a2 + params.k3.x  * r2 + params.k3.y  * r4;
        let yd0 = y * cdist * icdist2 + params.k1.z * a3 + params.k1.w * a1 + params.k3.z * r2 + params.k3.w * r4;

        vec2(xd0, yd0)
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(_calib_w: &mut usize, _calib_h: &mut usize/*, lens_model: &mut String*/) { }
}
