// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>
//
// Generic polynomial fisheye projection — proto's `GenericPolynomial` variant.
// 12 terms (k0·θ + k1·θ² + ... + k11·θ¹²) packed across k1.xyzw, k2.xyzw, k3.xyzw.
// Trailing zero slots are a mathematical no-op for shorter calibrations.

use crate::types::*;
use crate::glam::{ Vec2, vec2, Vec3 };
pub struct GenericPolynomial { }

impl GenericPolynomial {
    pub fn undistort_point(point: Vec2, params: &KernelParams) -> Vec2 {
        const EPS: f32 = 1e-6;

        let theta_d = point.length();

        let mut converged = false;
        let mut theta = theta_d;

        let mut scale = 0.0;

        if theta_d.abs() > EPS {
            theta = 0.0;

            let mut i = 0; while i < 10 {
                let theta2  = theta*theta;
                let theta3  = theta2*theta;
                let theta4  = theta2*theta2;
                let theta5  = theta2*theta3;
                let theta6  = theta3*theta3;
                let theta7  = theta3*theta4;
                let theta8  = theta4*theta4;
                let theta9  = theta4*theta5;
                let theta10 = theta5*theta5;
                let theta11 = theta5*theta6;
                let k0          = params.k1.x;
                let k1_theta1   = params.k1.y * theta;
                let k2_theta2   = params.k1.z * theta2;
                let k3_theta3   = params.k1.w * theta3;
                let k4_theta4   = params.k2.x * theta4;
                let k5_theta5   = params.k2.y * theta5;
                let k6_theta6   = params.k2.z * theta6;
                let k7_theta7   = params.k2.w * theta7;
                let k8_theta8   = params.k3.x * theta8;
                let k9_theta9   = params.k3.y * theta9;
                let k10_theta10 = params.k3.z * theta10;
                let k11_theta11 = params.k3.w * theta11;
                let theta_fix = (theta * (k0 + k1_theta1 + k2_theta2 + k3_theta3 + k4_theta4 + k5_theta5 + k6_theta6 + k7_theta7 + k8_theta8 + k9_theta9 + k10_theta10 + k11_theta11) - theta_d)
                                /
                                (k0 + 2.0 * k1_theta1 + 3.0 * k2_theta2 + 4.0 * k3_theta3 + 5.0 * k4_theta4 + 6.0 * k5_theta5 + 7.0 * k6_theta6 + 8.0 * k7_theta7 + 9.0 * k8_theta8 + 10.0 * k9_theta9 + 11.0 * k10_theta10 + 12.0 * k11_theta11);

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
        let theta2  = theta*theta;
        let theta3  = theta2*theta;
        let theta4  = theta2*theta2;
        let theta5  = theta2*theta3;
        let theta6  = theta3*theta3;
        let theta7  = theta3*theta4;
        let theta8  = theta4*theta4;
        let theta9  = theta4*theta5;
        let theta10 = theta5*theta5;
        let theta11 = theta5*theta6;
        let theta12 = theta6*theta6;

        let theta_d = theta   * params.k1.x
                    + theta2  * params.k1.y
                    + theta3  * params.k1.z
                    + theta4  * params.k1.w
                    + theta5  * params.k2.x
                    + theta6  * params.k2.y
                    + theta7  * params.k2.z
                    + theta8  * params.k2.w
                    + theta9  * params.k3.x
                    + theta10 * params.k3.y
                    + theta11 * params.k3.z
                    + theta12 * params.k3.w;

        let scale = if r == 0.0 { 1.0 } else { theta_d / r };

        pt * scale
    }

    #[cfg(not(target_arch = "spirv"))]
    pub fn adjust_lens_profile(_calib_w: &mut usize, _calib_h: &mut usize/*, lens_model: &mut String*/) { }
}
