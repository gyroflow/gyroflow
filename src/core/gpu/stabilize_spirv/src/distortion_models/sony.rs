// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3 };
pub struct Sony { }

impl Sony {
    pub fn undistort_point(mut point: Vec2, params: &KernelParams) -> Vec2 {
        const EPS: f32 = 1e-6;

        let post_scale = vec2(params.k2.z, params.k2.w);
        point /= post_scale;

        let theta_d = point.length();

        let mut converged = false;
        let mut theta = theta_d;

        let mut scale = 0.0;

        if theta_d.abs() > EPS {
            theta = 0.0;

            // compensate distortion iteratively
            let mut i = 0; while i < 10 {
            // for _ in 0..10 {
                let theta2 = theta*theta;
                let theta3 = theta2*theta;
                let theta4 = theta2*theta2;
                let theta5 = theta2*theta3;
                let k0 = params.k1.x;
                let k1_theta1 = params.k1.y * theta;
                let k2_theta2 = params.k1.z * theta2;
                let k3_theta3 = params.k1.w * theta3;
                let k4_theta4 = params.k2.x * theta4;
                let k5_theta5 = params.k2.y * theta5;
                // new_theta = theta - theta_fix, theta_fix = f0(theta) / f0'(theta)
                let theta_fix = (theta * (k0 + k1_theta1 + k2_theta2 + k3_theta3 + k4_theta4 + k5_theta5) - theta_d)
                                /
                                (k0 + 2.0 * k1_theta1 + 3.0 * k2_theta2 + 4.0 * k3_theta3 + 5.0 * k4_theta4 + 6.0 * k5_theta5);

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
        let theta3 = theta2*theta;
        let theta4 = theta2*theta2;
        let theta5 = theta2*theta3;
        let theta6 = theta3*theta3;

        let theta_d = theta * params.k1.x + theta2 * params.k1.y + theta3 * params.k1.z + theta4 * params.k1.w + theta5 * params.k2.x + theta6 * params.k2.y;

        let scale = if r == 0.0 { 1.0 } else { theta_d / r };

        let post_scale = vec2(params.k2.z, params.k2.w);

        pt * scale * post_scale
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(_calib_w: &mut usize, _calib_h: &mut usize/*, lens_model: &mut String*/) { }
}
