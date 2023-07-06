// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Adapted from OpenCV: https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/calib3d/src/fisheye.cpp#L257-L460

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3 };
pub struct OpenCVFisheye { }

impl OpenCVFisheye {
    pub fn undistort_point(point: Vec2, params: &KernelParams) -> Vec2 {
        const EPS: f32 = 1e-6;

        let mut theta_d = point.length();

        // the current camera model is only valid up to 180 FOV
        // for larger FOV the loop below does not converge
        // clip values so we still get plausible results for super fisheye images > 180 grad
        theta_d = theta_d.max(-core::f32::consts::PI).min(core::f32::consts::PI);

        let mut converged = false;
        let mut theta = theta_d;

        let mut scale = 0.0;

        if theta_d.abs() > EPS {
            theta = 0.0;

            // compensate distortion iteratively
            let mut i = 0; while i < 10 {
            // for _ in 0..10 {
                let theta2 = theta*theta;
                let theta4 = theta2*theta2;
                let theta6 = theta4*theta2;
                let theta8 = theta6*theta2;
                let k0_theta2 = params.k1.x * theta2;
                let k1_theta4 = params.k1.y * theta4;
                let k2_theta6 = params.k1.z * theta6;
                let k3_theta8 = params.k1.w * theta8;
                // new_theta = theta - theta_fix, theta_fix = f0(theta) / f0'(theta)
                let mut theta_fix = (theta * (1.0 + k0_theta2 + k1_theta4 + k2_theta6 + k3_theta8) - theta_d)
                                /
                                (1.0 + 3.0 * k0_theta2 + 5.0 * k1_theta4 + 7.0 * k2_theta6 + 9.0 * k3_theta8);

                theta_fix = theta_fix.max(-0.9).min(0.9);

                theta = theta - theta_fix;
                if theta_fix.abs() < EPS {
                    converged = true;
                    break;
                }
                i += 1;
            }

            scale = theta.tan() / theta_d;
        } else {
            converged = true;
        }

        // theta is monotonously increasing or decreasing depending on the sign of theta
        // if theta has flipped, it might converge due to symmetry but on the opposite of the camera center
        // so we can check whether theta has changed the sign during the optimization
        let theta_flipped = (theta_d < 0.0 && theta > 0.0) || (theta_d > 0.0 && theta < 0.0);

        if converged && !theta_flipped {
            return point * scale;
        }
        vec2(-99999.0, -99999.0)
    }

    pub fn distort_point(point: Vec3, params: &KernelParams) -> Vec2 {
        let pt = vec2(point.x / point.z, point.y / point.z);

        let r = pt.length();

        let theta = r.atan();
        let theta2 = theta*theta;
        let theta4 = theta2*theta2;
        let theta6 = theta4*theta2;
        let theta8 = theta4*theta4;

        let theta_d = theta * (1.0 + params.k1.x * theta2 + params.k1.y * theta4 + params.k1.z * theta6 + params.k1.w * theta8);

        let scale = if r == 0.0 { 1.0 } else { theta_d / r };

        pt * scale
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(_calib_w: &mut usize, _calib_h: &mut usize/*, lens_model: &mut String*/) { }
}
