// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

fn undistort_point(pos: vec2<f32>) -> vec2<f32> {
    if (params.k1.x == 0.0 && params.k1.y == 0.0 && params.k1.z == 0.0 && params.k1.w == 0.0) { return pos; }
    let theta_d = min(max(length(pos), -1.5707963267948966), 1.5707963267948966); // PI/2

    var converged = false;
    var theta = theta_d;

    var scale = 0.0;

    if (abs(theta_d) > 1e-6) {
        for (var i: i32 = 0; i < 10; i = i + 1) {
            let theta2 = theta*theta;
            let theta4 = theta2*theta2;
            let theta6 = theta4*theta2;
            let theta8 = theta6*theta2;
            let k0_theta2 = params.k1.x * theta2;
            let k1_theta4 = params.k1.y * theta4;
            let k2_theta6 = params.k1.z * theta6;
            let k3_theta8 = params.k1.w * theta8;
            // new_theta = theta - theta_fix, theta_fix = f0(theta) / f0'(theta)
            let theta_fix = (theta * (1.0 + k0_theta2 + k1_theta4 + k2_theta6 + k3_theta8) - theta_d)
                            /
                            (1.0 + 3.0 * k0_theta2 + 5.0 * k1_theta4 + 7.0 * k2_theta6 + 9.0 * k3_theta8);

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
    let theta4 = theta2*theta2;
    let theta6 = theta4*theta2;
    let theta8 = theta4*theta4;

    let theta_d = theta * (1.0 + dot(params.k1, vec4<f32>(theta2, theta4, theta6, theta8)));

    var scale: f32 = 1.0;
    if (r != 0.0) {
        scale = theta_d / r;
    }
    return pos * scale;
}
