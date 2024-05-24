// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)

fn undistort_point(pos_param: vec2<f32>) -> vec2<f32> {
    if (params.k1.x == 0.0 && params.k1.y == 0.0 && params.k1.z == 0.0 && params.k1.w == 0.0) { return pos_param; }

    let post_scale = vec2<f32>(params.k2.z, params.k2.w);
    var pos = pos_param / post_scale;

    // now pos is in meters from center of sensor

    let theta_d = length(pos);

    var converged = false;
    var theta = theta_d;

    var scale = 0.0;

    if (abs(theta_d) > 1e-6) {
        for (var i: i32 = 0; i < 10; i = i + 1) {
                let theta2 = theta*theta;
                let theta3 = theta2*theta;
                let theta4 = theta2*theta2;
                let theta5 = theta2*theta3;
                let k0  = params.k1.x;
                let k1_theta1 = params.k1.y * theta;
                let k2_theta2 = params.k1.z * theta2;
                let k3_theta3 = params.k1.w * theta3;
                let k4_theta4 = params.k2.x * theta4;
                let k5_theta5 = params.k2.y * theta5;
                // new_theta = theta - theta_fix, theta_fix = f0(theta) / f0'(theta)
                let theta_fix = (theta * (k0 + k1_theta1 + k2_theta2 + k3_theta3 + k4_theta4 + k5_theta5) - theta_d)
                                /
                                (k0 + 2.0 * k1_theta1 + 3.0 * k2_theta2 + 4.0 * k3_theta3 + 5.0 * k4_theta4 + 6.0 * k5_theta5);

            theta -= theta_fix;
            if (abs(theta_fix) < 1e-6) {
                converged = true;
                break;
            }
        }

        scale = tan(theta) / theta_d;
    } else {
        converged = true;
    }
    let theta_flipped = (theta_d < 0.0 && theta > 0.0) || (theta_d > 0.0 && theta < 0.0);

    if (converged && !theta_flipped) {
        return pos * scale;
    }
    return vec2<f32>(0.0, 0.0);
}

fn distort_point(x: f32, y: f32, z: f32) -> vec2<f32> {
    let pos = vec2<f32>(x, y) / z;
    if (params.k1.x == 0.0 && params.k1.y == 0.0 && params.k1.z == 0.0 && params.k1.w == 0.0) { return pos; }
    let r = length(pos);

    let theta = atan(r);

    let theta2 = theta*theta;
    let theta3 = theta2*theta;
    let theta4 = theta2*theta2;
    let theta5 = theta2*theta3;
    let theta6 = theta3*theta3;

    let theta_d = theta * params.k1.x + theta2 * params.k1.y + theta3 * params.k1.z + theta4 * params.k1.w + theta5 * params.k2.x + theta6 * params.k2.y;

    var scale: f32 = 1.0;
    if (r != 0.0) {
        scale = theta_d / r;
    }

    let post_scale = vec2<f32>(params.k2.z, params.k2.w);

    return pos * scale * post_scale;
}
