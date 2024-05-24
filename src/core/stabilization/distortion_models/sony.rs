// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)

use crate::stabilization::KernelParams;

#[derive(Default, Clone)]
pub struct Sony { }

impl Sony {
    pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        if params.k[0] == 0.0 && params.k[1] == 0.0 && params.k[2] == 0.0 && params.k[3] == 0.0 { return Some(point); }

        const EPS: f32 = 1e-6;

        let post_scale = (params.k[6], params.k[7]);

        let point = (point.0 / post_scale.0, point.1 / post_scale.1);
        // now point is in meters from center of sensor

        let theta_d = (point.0 * point.0 + point.1 * point.1).sqrt();

        let mut converged = false;
        let mut theta = theta_d;

        let mut scale = 0.0;

        if theta_d.abs() > EPS {
            theta = 0.0;

            // compensate distortion iteratively
            for _ in 0..10 {
                let theta2 = theta*theta;
                let theta3 = theta2*theta;
                let theta4 = theta2*theta2;
                let theta5 = theta2*theta3;
                let theta6 = theta3*theta3;
                let k0  = params.k[0];
                let k1_theta1 = params.k[1] * theta;
                let k2_theta2 = params.k[2] * theta2;
                let k3_theta3 = params.k[3] * theta3;
                let k4_theta4 = params.k[4] * theta4;
                let k5_theta5 = params.k[5] * theta5;
                // new_theta = theta - theta_fix, theta_fix = f0(theta) / f0'(theta)
                let theta_fix = (theta * (k0 + k1_theta1 + k2_theta2 + k3_theta3 + k4_theta4 + k5_theta5) - theta_d)
                                /
                                (k0 + 2.0 * k1_theta1 + 3.0 * k2_theta2 + 4.0 * k3_theta3 + 5.0 * k4_theta4 + 6.0 * k5_theta5);

                theta = theta - theta_fix;
                if theta_fix.abs() < EPS {
                    converged = true;
                    break;
                }
            }

            scale = theta.tan() / theta_d;
        } else {
            converged = true;
        }

        let theta_flipped = (theta_d < 0.0 && theta > 0.0) || (theta_d > 0.0 && theta < 0.0);

        if converged && !theta_flipped {
            return Some((point.0 * scale, point.1 * scale));
        }
        None
    }

    pub fn distort_point(&self, x: f32, y: f32, z: f32, params: &KernelParams) -> (f32, f32) {
        let x = x / z;
        let y = y / z;
        if params.k[0] == 0.0 && params.k[1] == 0.0 && params.k[2] == 0.0 && params.k[3] == 0.0 { return (x, y); }

        let r = (x.powi(2) + y.powi(2)).sqrt();

        let theta = r.atan();

        let theta2 = theta*theta;
        let theta3 = theta2*theta;
        let theta4 = theta2*theta2;
        let theta5 = theta2*theta3;
        let theta6 = theta3*theta3;

        let theta_d = theta * params.k[0] + theta2 * params.k[1] + theta3 * params.k[2] + theta4 * params.k[3] + theta5 * params.k[4] + theta6 * params.k[5];

        let scale = if r == 0.0 { 1.0 } else { theta_d / r };

        let post_scale = (params.k[6], params.k[7]);

        (
            x * scale * post_scale.0,
            y * scale * post_scale.1
        )
    }

    pub fn distort_for_light_refraction(&self, p: &[f64], theta: f64) -> f64 {
        // FIXME
        let theta2 = theta*theta;
        let theta3 = theta2*theta;
        let theta4 = theta2*theta2;
        let theta5 = theta2*theta3;
        let theta6 = theta3*theta3;
        p[0] * (theta * p[1] + theta2 * p[2] + theta3 * p[3] + theta4 * p[4] + theta5 * p[5] + theta6 * p[6])
    }

    pub fn undistort_for_light_refraction_gradient(&self, p: &[f64], theta: f64) -> Vec<f64> {
        // FIXME
        let theta2 = theta*theta;
        let theta3 = theta2*theta;
        let theta4 = theta2*theta2;
        let theta5 = theta2*theta3;
        let theta6 = theta3*theta3;
        vec![
            theta * p[1] + theta2 * p[2] + theta3 * p[3] + theta4 * p[4] + theta5 * p[5] + theta6 * p[6],
            p[0] * theta * theta,
            p[0] * theta * theta2,
            p[0] * theta * theta3,
            p[0] * theta * theta4,
            p[0] * theta * theta5,
            p[0] * theta * theta6,
        ]
    }

    pub fn adjust_lens_profile(&self, _profile: &mut crate::LensProfile) { }

    pub fn id() -> &'static str { "sony" }
    pub fn name() -> &'static str { "Sony" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("sony.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("sony.wgsl") }
}
