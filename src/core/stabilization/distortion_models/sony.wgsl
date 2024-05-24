// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)

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

fn undistort_point(p: vec2<f32>) -> vec2<f32> {
    var pp = p;

    for (var i: i32 = 0; i < 20; i = i + 1) {
        let diff = distort_point(pp.x, pp.y, 1.0) - p;
        if (abs(diff.x) < 1e-6 && abs(diff.y) < 1e-6) {
            break;
        }
        pp -= diff;
    }

    return pp;
}
