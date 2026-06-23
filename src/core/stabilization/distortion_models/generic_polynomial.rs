// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>
//
// Generic polynomial fisheye projection — proto's `GenericPolynomial` variant.
// Maps θ (radians from optical axis) to a dimensionless normalized image radius:
//   r_normalized = k0·θ + k1·θ² + k2·θ³ + ... + k11·θ¹²
// k0 ≈ 1.0 for paraxial. Output pixels = r_normalized · f_x_pixels;
// the per-axis pixel scaling happens in the main kernel via `params.f`.
// Calibrations shorter than 12 terms ride on zero-padded trailing slots
// (mathematical no-op: 0·θⁿ = 0).

use crate::stabilization::KernelParams;

#[derive(Default, Clone)]
pub struct GenericPolynomial { }

impl GenericPolynomial {
    pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
        if params.k[0]  == 0.0 && params.k[1]  == 0.0 && params.k[2]  == 0.0 && params.k[3]  == 0.0
        && params.k[4]  == 0.0 && params.k[5]  == 0.0 && params.k[6]  == 0.0 && params.k[7]  == 0.0
        && params.k[8]  == 0.0 && params.k[9]  == 0.0 && params.k[10] == 0.0 && params.k[11] == 0.0 { return Some(point); }

        const EPS: f32 = 1e-6;

        let theta_d = (point.0 * point.0 + point.1 * point.1).sqrt();

        let mut converged = false;
        let mut theta = theta_d;

        let mut scale = 0.0;

        if theta_d.abs() > EPS {
            theta = 0.0;

            // Newton iteration on r_normalized(θ) - theta_d = 0
            for _ in 0..10 {
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
                let k0          = params.k[0];
                let k1_theta1   = params.k[1]  * theta;
                let k2_theta2   = params.k[2]  * theta2;
                let k3_theta3   = params.k[3]  * theta3;
                let k4_theta4   = params.k[4]  * theta4;
                let k5_theta5   = params.k[5]  * theta5;
                let k6_theta6   = params.k[6]  * theta6;
                let k7_theta7   = params.k[7]  * theta7;
                let k8_theta8   = params.k[8]  * theta8;
                let k9_theta9   = params.k[9]  * theta9;
                let k10_theta10 = params.k[10] * theta10;
                let k11_theta11 = params.k[11] * theta11;
                let theta_fix = (theta * (k0 + k1_theta1 + k2_theta2 + k3_theta3 + k4_theta4 + k5_theta5 + k6_theta6 + k7_theta7 + k8_theta8 + k9_theta9 + k10_theta10 + k11_theta11) - theta_d)
                                /
                                (k0 + 2.0 * k1_theta1 + 3.0 * k2_theta2 + 4.0 * k3_theta3 + 5.0 * k4_theta4 + 6.0 * k5_theta5 + 7.0 * k6_theta6 + 8.0 * k7_theta7 + 9.0 * k8_theta8 + 10.0 * k9_theta9 + 11.0 * k10_theta10 + 12.0 * k11_theta11);

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
        if params.k[0]  == 0.0 && params.k[1]  == 0.0 && params.k[2]  == 0.0 && params.k[3]  == 0.0
        && params.k[4]  == 0.0 && params.k[5]  == 0.0 && params.k[6]  == 0.0 && params.k[7]  == 0.0
        && params.k[8]  == 0.0 && params.k[9]  == 0.0 && params.k[10] == 0.0 && params.k[11] == 0.0 { return (x, y); }

        let r = (x.powi(2) + y.powi(2)).sqrt();

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

        let theta_d = theta   * params.k[0]
                    + theta2  * params.k[1]
                    + theta3  * params.k[2]
                    + theta4  * params.k[3]
                    + theta5  * params.k[4]
                    + theta6  * params.k[5]
                    + theta7  * params.k[6]
                    + theta8  * params.k[7]
                    + theta9  * params.k[8]
                    + theta10 * params.k[9]
                    + theta11 * params.k[10]
                    + theta12 * params.k[11];

        let scale = if r == 0.0 { 1.0 } else { theta_d / r };

        (x * scale, y * scale)
    }

    pub fn adjust_lens_profile(&self, _profile: &mut crate::LensProfile) { }

    pub fn distortion_derivative(&self, theta: f64, k: &[f64]) -> Option<f64> {
        // Evaluates d/dθ [Σ k[i]·θ^(i+1)] = Σ (i+1)·k[i]·θ^i over the supplied
        // coefficients (up to 12 terms). Tolerates short slices — 6- or 8-term
        // calibrations are valid inputs.
        if k.is_empty() { return None; }
        let n = k.len().min(12);
        let mut acc = 0.0_f64;
        let mut theta_pow = 1.0_f64; // θ^0
        for i in 0..n {
            acc += (i as f64 + 1.0) * k[i] * theta_pow;
            theta_pow *= theta;
        }
        Some(acc)
    }

    pub fn id() -> &'static str { "generic_polynomial" }
    pub fn name() -> &'static str { "Generic polynomial" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("generic_polynomial.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("generic_polynomial.wgsl") }
}
