// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Adapted from OpenCV: https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/calib3d/src/fisheye.cpp#L257-L460

use crate::stabilization::KernelParams;

#[derive(Default, Clone)]
pub struct OpenCVFisheye { }

impl OpenCVFisheye {
    pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        if params.k[0] == 0.0 && params.k[1] == 0.0 && params.k[2] == 0.0 && params.k[3] == 0.0 { return Some(point); }

        const EPS: f32 = 1e-6;

        let mut theta_d = (point.0 * point.0 + point.1 * point.1).sqrt();

        // the current camera model is only valid up to 180 FOV
        // for larger FOV the loop below does not converge
        // clip values so we still get plausible results for super fisheye images > 180 grad
        theta_d = theta_d.max(-std::f32::consts::PI).min(std::f32::consts::PI);

        let mut converged = false;
        let mut theta = theta_d;

        let mut scale = 0.0;

        if theta_d.abs() > EPS {
            theta = 0.0;

            // compensate distortion iteratively
            for _ in 0..10 {
                let theta2 = theta*theta;
                let theta4 = theta2*theta2;
                let theta6 = theta4*theta2;
                let theta8 = theta6*theta2;
                let k0_theta2 = params.k[0] * theta2;
                let k1_theta4 = params.k[1] * theta4;
                let k2_theta6 = params.k[2] * theta6;
                let k3_theta8 = params.k[3] * theta8;
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
        let theta4 = theta2*theta2;
        let theta6 = theta4*theta2;
        let theta8 = theta4*theta4;

        let theta_d = theta * (1.0 + params.k[0]*theta2 + params.k[1]*theta4 + params.k[2]*theta6 + params.k[3]*theta8);

        let scale = if r == 0.0 { 1.0 } else { theta_d / r };

        (
            x * scale,
            y * scale
        )
    }

    pub fn adjust_lens_profile(&self, _profile: &mut crate::LensProfile) { }

    pub fn distortion_derivative(&self, theta: f64, k: &[f64]) -> Option<f64> {
        let theta2 = theta * theta;
        let theta4 = theta2 * theta2;
        let theta6 = theta4 * theta2;
        let theta8 = theta6 * theta2;
        Some(
            1.0 + 3.0 * k[0] * theta2 + 5.0 * k[1] * theta4 + 7.0 * k[2] * theta6 + 9.0 * k[3] * theta8
        )
    }

    pub fn id() -> &'static str { "opencv_fisheye" }
    pub fn name() -> &'static str { "OpenCV Fisheye" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("opencv_fisheye.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("opencv_fisheye.wgsl") }
}
