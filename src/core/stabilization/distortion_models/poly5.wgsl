// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

fn undistort_point(pos: vec2<f32>, k1: vec4<f32>, k2: vec4<f32>, k3: vec4<f32>, amount: f32) -> vec2<f32> {
    let NEWTON_EPS = 0.00001;

    let rd = length(pos);
    if (rd == 0.0) { return vec2<f32>(0.0, 0.0); }

    var ru = rd;
    for (var i: i32 = 0; i < 10; i = i + 1) {
        let ru2 = ru * ru;
        let fru = ru * (1.0 + k1.x * ru2 + k1.y * ru2 * ru2) - rd;
        if (fru >= -NEWTON_EPS && fru < NEWTON_EPS) {
            break;
        }
        if (i > 5) {
            // Does not converge, no real solution in this area?
            return vec2<f32>(0.0, 0.0);
        }

        ru = ru - (fru / (1.0 + 3.0 * k1.x * ru2 + 5.0 * k1.y * ru2 * ru2));
    }
    if (ru < 0.0) {
        return vec2<f32>(0.0, 0.0);
    }

    ru = ru / rd;

    // Apply only requested amount
    ru = 1.0 + (ru - 1.0) * (1.0 - amount);

    return pos * ru;
}

fn distort_point(pos: vec2<f32>, k1: vec4<f32>, k2: vec4<f32>, k3: vec4<f32>) -> vec2<f32> {
    let ru2 = (pos.x * pos.x + pos.y * pos.y);
    let poly4 = 1.0 + k1.x * ru2 + k1.y * ru2 * ru2;
    return pos * poly4;
}
