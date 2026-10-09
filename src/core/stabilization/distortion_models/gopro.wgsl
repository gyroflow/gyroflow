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
    let r_norm = length(pos);
    // No POLY block: a pinhole, so the image radius is `tan θ`
    if (params.k1.y == 0.0) {
        if (r_norm < 1e-12) { return pos; }
        return pos * (atan(r_norm) / r_norm);
    }
    if (r_norm < 1e-9) { return pos; }
    let p = r_norm / params.k1.y;
    let theta = gopro_poly_eval(p);
    // Outside its fit range the POLY is free to run negative or past 180°, and neither is an angle this
    // pipeline can carry - a negative theta hands back the ray from the opposite side of the frame, and
    // past 180 degrees sin(theta) has turned over and the ray arrives from behind the camera. See gopro.rs
    if (!(theta > 0.0 && theta < 3.14159265)) { return vec2<f32>(-99999.0, -99999.0); }
    return pos * (theta / r_norm);
}

fn distort_point(x: f32, y: f32, z: f32) -> vec2<f32> {
    // No POLY block: a pinhole, which has no image of a ray at or past 90°
    if (params.k1.y == 0.0) {
        if (z > 1e-9) { return vec2<f32>(x, y) / z; }
        return vec2<f32>(x, y) * 1e9;
    }
    let pos = vec2<f32>(x, y);
    let r = length(pos);
    if (r < 1e-12) { return vec2<f32>(0.0, 0.0); }
    // atan2 against the ray's own z: a GoPro's 150°+ field needs the angle, not a z=1 plane radius
    let theta = atan2(r, z);
    return pos * (params.k1.y * gopro_poly_invert(theta) / r);
}
