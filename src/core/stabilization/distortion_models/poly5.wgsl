// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

fn undistort_point(pos: vec2<f32>) -> vec2<f32> {
    let NEWTON_EPS = 0.00001;

    let rd = length(pos);
    if (rd == 0.0) { return pos; }

    var ru = rd;
    for (var i: i32 = 0; i < 10; i = i + 1) {
        let ru2 = ru * ru;
        let fru = ru * (1.0 + params.k1.x * ru2 + params.k1.y * ru2 * ru2) - rd;
        if (fru >= -NEWTON_EPS && fru < NEWTON_EPS) {
            break;
        }
        if (i > 5) {
            // Does not converge, no real solution in this area?
            return vec2<f32>(-99999.0, -99999.0);
        }

        ru = ru - (fru / (1.0 + 3.0 * params.k1.x * ru2 + 5.0 * params.k1.y * ru2 * ru2));
    }
    if (ru < 0.0) {
        return vec2<f32>(-99999.0, -99999.0);
    }

    // `ru` is the radius in the z=1 plane; the pipeline carries rays as angle vectors
    return pos * (atan(ru) / rd);
}

fn distort_point(x: f32, y: f32, z: f32) -> vec2<f32> {
    // A rectilinear model has no image of a ray at or past 90°: send it far outside the frame
    var pos = vec2<f32>(x, y) * 1e9;
    if (z > 1e-9) { pos = vec2<f32>(x, y) / z; }
    let ru2 = (pos.x * pos.x + pos.y * pos.y);
    let poly4 = 1.0 + params.k1.x * ru2 + params.k1.y * ru2 * ru2;
    return pos * poly4;
}
