// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)

use crate::stabilization::KernelParams;

#[derive(Default, Clone)]
pub struct Sony { }

impl Sony {
    pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        if params.k[0] == 0.0 && params.k[1] == 0.0 && params.k[2] == 0.0 && params.k[3] == 0.0 { return Some(point); }

        let mut px = point.0;
        let mut py = point.1;

        for _ in 0..20 {
            let dp = self.distort_point(px, py, 1.0, params);
            let diff = (dp.0 - point.0, dp.1 - point.1);
            if diff.0.abs() < 1e-6 && diff.1.abs() < 1e-6 {
                break;
            }
            px -= diff.0;
            py -= diff.1;
        }

        Some((px, py))
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
