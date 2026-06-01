// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>
//
// GoPro native radial lens model. Raw POLY radial coeffs r0..r6 in params.k1/k2.
// The Superview/Hyperview MAPX/MAPY warp is a separate digital lens (gopro_warp).

fn gopro_poly_eval(p: f32) -> f32 {
    return params.k1.x + p * (params.k1.y + p * (params.k1.z + p * (params.k1.w + p * (params.k2.x + p * (params.k2.y + p * params.k2.z)))));
}
fn gopro_poly_deriv(p: f32) -> f32 {
    return params.k1.y + p * (2.0 * params.k1.z + p * (3.0 * params.k1.w + p * (4.0 * params.k2.x + p * (5.0 * params.k2.y + p * (6.0 * params.k2.z)))));
}
fn gopro_poly_invert(theta: f32) -> f32 {
    var p = (theta - params.k1.x) / params.k1.y;
    for (var i: i32 = 0; i < 10; i = i + 1) {
        let d = gopro_poly_deriv(p);
        if (abs(d) < 1e-12) { break; }
        let fix = (gopro_poly_eval(p) - theta) / d;
        p -= fix;
        if (abs(fix) < 1e-7) { break; }
    }
    return p;
}

fn undistort_point(pos: vec2<f32>) -> vec2<f32> {
    if (params.k1.y == 0.0) { return pos; }
    let r_norm = length(pos);
    if (r_norm < 1e-9) { return pos; }
    let p = r_norm / params.k1.y;
    let theta = gopro_poly_eval(p);
    // Clamp the angle just under tan()'s 90° asymptote and continue the radius linearly past it so over-FOV
    // rays stay large & monotonic (no wrap/fold -> r_limit clips them to background). See gopro.rs.
    let TMAX = 1.5533; let tt = 57.14902; // TMAX ≈ 89°, tt = tan(TMAX)
    var rr: f32; if (theta < TMAX) { rr = tan(theta); } else { rr = tt + (theta - TMAX) * (1.0 + tt * tt); }
    let scale = rr / r_norm;
    return pos * scale;
}

fn distort_point(x: f32, y: f32, z: f32) -> vec2<f32> {
    let pos = vec2<f32>(x, y) / z;
    if (params.k1.y == 0.0) { return pos; }
    let r = length(pos);
    // Inverse of undistort_point's angle clamp (see gopro.rs / gopro.wgsl undistort_point).
    let TMAX = 1.5533; let tt = 57.14902;
    var theta: f32; if (r < tt) { theta = atan(r); } else { theta = TMAX + (r - tt) / (1.0 + tt * tt); }
    let p = gopro_poly_invert(theta);
    let r_norm = params.k1.y * p;
    var scale: f32 = 1.0;
    if (r != 0.0) { scale = r_norm / r; }
    return pos * scale;
}
