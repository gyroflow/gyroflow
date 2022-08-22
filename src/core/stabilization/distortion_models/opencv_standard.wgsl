// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

fn undistort_point(pos_param: vec2<f32>, k: array<f32, 12>, amount: f32) -> vec2<f32> {
    var pos = pos_param;

    let start_pos = pos;

    // compensate distortion iteratively
    for (var i: i32 = 0; i < 20; i = i + 1) {
        let r2 = pos.x * pos.x + pos.y * pos.y;
        let icdist = (1.0 + ((k[7] * r2 + k[6]) * r2 + k[5]) * r2)/(1.0 + ((k[4] * r2 + k[1]) * r2 + k[0]) * r2);
        if (icdist < 0.0) {
            return vec2<f32>(0.0, 0.0);
        }
        let delta_x = 2.0 * k[2] * pos.x * pos.y + k[3] * (r2 + 2.0 * pos.x * pos.x)+ k[8] * r2 + k[9] * r2 * r2;
        let delta_y = k[2] * (r2 + 2.0 * pos.y * pos.y) + 2.0 * k[3] * pos.x * pos.y+ k[10] * r2 + k[11] * r2 * r2;
        pos = vec2<f32>(
            (start_pos.x - delta_x) * icdist,
            (start_pos.y - delta_y) * icdist
        );
    }

    return vec2<f32>(
        pos.x * (amount - 1.0) + start_pos.x * amount,
        pos.y * (amount - 1.0) + start_pos.y * amount
    );
}

fn distort_point(pos: vec2<f32>, k: array<f32, 12>) -> vec2<f32> {
    let r2 = pos.x * pos.x + pos.y * pos.y;
    let r4 = r2 * r2;
    let r6 = r4 * r2;
    let a1 = 2.0 * pos.x * pos.y;
    let a2 = r2 + 2.0 * pos.x * pos.x;
    let a3 = r2 + 2.0 * pos.y * pos.y;
    let cdist = 1.0 + k[0] * r2 + k[1] * r4 + k[4] * r6;
    let icdist2 = 1.0 / (1.0 + k[5] * r2 + k[6] * r4 + k[7] * r6);

    return vec2<f32>(
        pos.x * cdist * icdist2 + k[2] * a1 + k[3] * a2 + k[8]  * r2 + k[9]  * r4,
        pos.y * cdist * icdist2 + k[2] * a3 + k[3] * a1 + k[10] * r2 + k[11] * r4
    );
}
