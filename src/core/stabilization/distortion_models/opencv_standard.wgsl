// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

fn undistort_point(pos_param: vec2<f32>) -> vec2<f32> {
    var pos = pos_param;

    let start_pos = pos;

    // compensate distortion iteratively
    for (var i: i32 = 0; i < 20; i = i + 1) {
        let r2 = pos.x * pos.x + pos.y * pos.y;
        let icdist = (1.0 + ((params.k2.w * r2 + params.k2.z) * r2 + params.k2.y) * r2)/(1.0 + ((k2.x * r2 + params.k1.y) * r2 + params.k1.x) * r2);
        if (icdist < 0.0) {
            return vec2<f32>(0.0, 0.0);
        }
        let delta_x = 2.0 * params.k1.z * pos.x * pos.y + params.k1.w * (r2 + 2.0 * pos.x * pos.x)+ params.k3.x * r2 + params.k3.y * r2 * r2;
        let delta_y = params.k1.z * (r2 + 2.0 * pos.y * pos.y) + 2.0 * params.k1.w * pos.x * pos.y+ params.k3.z * r2 + params.k3.w * r2 * r2;
        pos = vec2<f32>(
            (start_pos.x - delta_x) * icdist,
            (start_pos.y - delta_y) * icdist
        );
    }

    return pos;
}

fn distort_point(pos: vec2<f32>) -> vec2<f32> {
    let r2 = pos.x * pos.x + pos.y * pos.y;
    let r4 = r2 * r2;
    let r6 = r4 * r2;
    let a1 = 2.0 * pos.x * pos.y;
    let a2 = r2 + 2.0 * pos.x * pos.x;
    let a3 = r2 + 2.0 * pos.y * pos.y;
    let cdist = 1.0 + params.k1.x * r2 + params.k1.y * r4 + params.k2.x * r6;
    let icdist2 = 1.0 / (1.0 + params.k2.y * r2 + params.k2.z * r4 + params.k2.w * r6);

    return vec2<f32>(
        pos.x * cdist * icdist2 + params.k1.z * a1 + params.k1.w * a2 + params.k3.x * r2 + params.k3.y * r4,
        pos.y * cdist * icdist2 + params.k1.z * a3 + params.k1.w * a1 + params.k3.z * r2 + params.k3.w * r4
    );
}
