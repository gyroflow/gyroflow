// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>
//
// Generic polynomial fisheye projection — proto's `GenericPolynomial` variant.
//   r_normalized = k0·θ + k1·θ² + k2·θ³ + ... + k11·θ¹²
// dimensionless. Pixel scaling by f_x / f_y is done by the main kernel.
// 12 coefficients packed across params.k1.xyzw, k2.xyzw, k3.xyzw. Calibrations
// shorter than 12 terms ride on zero-padded trailing slots.

fn undistort_point(pos_param: vec2<f32>) -> vec2<f32> {
    if (params.k1.x == 0.0 && params.k1.y == 0.0 && params.k1.z == 0.0 && params.k1.w == 0.0
     && params.k2.x == 0.0 && params.k2.y == 0.0 && params.k2.z == 0.0 && params.k2.w == 0.0
     && params.k3.x == 0.0 && params.k3.y == 0.0 && params.k3.z == 0.0 && params.k3.w == 0.0) {
        // No calibration: a pinhole, so the image radius is `tan θ`
        let r = length(pos_param);
        if (r < 1e-12) { return pos_param; }
        return pos_param * (atan(r) / r);
    }

    var pos = pos_param;

    let theta_d = length(pos);

    var converged = false;
    var theta = theta_d;

    var scale = 0.0;

    if (abs(theta_d) > 1e-6) {
        theta = 0.0;
        for (var i: i32 = 0; i < 15; i = i + 1) {
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
            let theta_fix = clamp((theta * (k0 + k1_theta1 + k2_theta2 + k3_theta3 + k4_theta4 + k5_theta5 + k6_theta6 + k7_theta7 + k8_theta8 + k9_theta9 + k10_theta10 + k11_theta11) - theta_d)
                                  /
                                  (k0 + 2.0 * k1_theta1 + 3.0 * k2_theta2 + 4.0 * k3_theta3 + 5.0 * k4_theta4 + 6.0 * k5_theta5 + 7.0 * k6_theta6 + 8.0 * k7_theta7 + 9.0 * k8_theta8 + 10.0 * k9_theta9 + 11.0 * k10_theta10 + 12.0 * k11_theta11), -0.9, 0.9);

            theta -= theta_fix;
            if (abs(theta_fix) < 1e-6) {
                converged = true;
                break;
            }
        }

        scale = theta / theta_d;
    } else {
        converged = true;
    }
    let theta_flipped = (theta_d < 0.0 && theta > 0.0) || (theta_d > 0.0 && theta < 0.0);

    // Nothing past 180 degrees is a ray the pipeline can carry: the direction is built out of sin(theta),
    // which turns over there, so an angle the Newton wandered past it comes back from behind the camera.
    // See generic_polynomial.rs
    if (abs(theta) >= 3.14159265) { return vec2<f32>(-99999.0, -99999.0); }

    if (converged && !theta_flipped) {
        return pos * scale;
    }
    return vec2<f32>(-99999.0, -99999.0);
}

fn distort_point(x: f32, y: f32, z: f32) -> vec2<f32> {
    if (params.k1.x == 0.0 && params.k1.y == 0.0 && params.k1.z == 0.0 && params.k1.w == 0.0
     && params.k2.x == 0.0 && params.k2.y == 0.0 && params.k2.z == 0.0 && params.k2.w == 0.0
     && params.k3.x == 0.0 && params.k3.y == 0.0 && params.k3.z == 0.0 && params.k3.w == 0.0) {
        // No calibration: a pinhole, which has no image of a ray at or past 90°
        if (z > 1e-9) { return vec2<f32>(x, y) / z; }
        return vec2<f32>(x, y) * 1e9;
    }
    let pos = vec2<f32>(x, y);
    let r = length(pos);

    // atan2 against the ray's own z, so the angle is right past 90° too
    let theta = atan2(r, z);

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

    var scale: f32 = 1.0;
    if (r > 1e-12) {
        scale = theta_d / r;
    }

    return pos * scale;
}
