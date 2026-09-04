// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)
//
// Sony native lens model: the camera's lens curve (ray angle at equally spaced image radii) evaluated as a natural
// cubic spline radius -> angle. Layout of params.k1..k6: see sony.rs.

fn sony_k(i: i32) -> f32 {
    let v = i / 4;
    var q = params.k6;
    if (v == 0) { q = params.k1; } else if (v == 1) { q = params.k2; } else if (v == 2) { q = params.k3; } else if (v == 3) { q = params.k4; } else if (v == 4) { q = params.k5; }
    return q[i - v * 4];
}
fn sony_segments() -> i32 {
    let n = params.k1.x;
    if (n >= 1.0 && n <= 10.0 && params.k1.y > 0.0 && params.k1.z == 0.0) { return i32(n); }
    return 0;
}
// (y_i, b_i, c_i, d_i) of spline segment i
fn sony_segment(i: i32) -> vec4<f32> {
    let h = params.k1.y;
    let y0 = sony_k(2 + i); let y1 = sony_k(3 + i);
    var c0 = 0.0; if (i > 0) { c0 = sony_k(13 + i); } // k[13] holds r_lin, c_0 is 0
    let c1 = sony_k(14 + i);
    return vec4<f32>(y0, (y1 - y0) / h - h * (c1 + 2.0 * c0) / 3.0, c0, (c1 - c0) / (3.0 * h));
}
const SONY_THETA0: f32 = 0.052359879; // 3 deg, end of the linear region near the optical axis
// Ray angle at normalized image radius r (linear continuation with the end slope outside the knots)
fn sony_angle_at(n: i32, r: f32) -> f32 {
    let h = params.k1.y;
    let r_lin = sony_k(13);
    if (r_lin > 0.0 && r < r_lin) { return SONY_THETA0 * r / r_lin; }
    let i = clamp(i32(r / h), 0, n - 1);
    let s = sony_segment(i);
    let dx = clamp(r - f32(i) * h, 0.0, h);
    let slope = s.y + (2.0 * s.z + 3.0 * s.w * dx) * dx;
    return s.x + (s.y + (s.z + s.w * dx) * dx) * dx + slope * (r - f32(i) * h - dx);
}
// Normalized image radius for ray angle theta (inverse of sony_angle_at)
fn sony_radius_at(n: i32, theta: f32) -> f32 {
    let h = params.k1.y;
    if (theta <= 0.0) { return 0.0; }
    let r_lin = sony_k(13);
    if (r_lin > 0.0 && theta < SONY_THETA0) { return theta * r_lin / SONY_THETA0; }
    var i: i32 = 0;
    loop {
        if (!(i + 1 < n && theta >= sony_k(3 + i))) { break; }
        i = i + 1;
    }
    let s = sony_segment(i);
    let y1 = sony_k(3 + i);
    if (theta >= y1) { // past the last knot: linear continuation
        let slope = s.y + (2.0 * s.z + 3.0 * s.w * h) * h;
        if (slope > 1e-9) { return f32(n) * h + (theta - y1) / slope; }
        return f32(n) * h;
    }
    // Newton on the segment's cubic, starting from the chord
    var dx = h * (theta - s.x) / max(y1 - s.x, 1e-12);
    for (var it: i32 = 0; it < 8; it = it + 1) {
        let f = s.x + (s.y + (s.z + s.w * dx) * dx) * dx - theta;
        let fp = s.y + (2.0 * s.z + 3.0 * s.w * dx) * dx;
        if (abs(fp) < 1e-12) { break; }
        let fix = f / fp;
        dx = clamp(dx - fix, 0.0, h);
        if (abs(fix) < 1e-7) { break; }
    }
    return f32(i) * h + dx;
}

fn undistort_point(pos: vec2<f32>) -> vec2<f32> {
    let n = sony_segments();
    if (n == 0) { return pos; }
    let r = length(pos);
    if (r < 1e-9) { return pos; }
    let theta = sony_angle_at(n, r);
    if (theta <= 0.0) { return vec2<f32>(-99999.0, -99999.0); }
    // Clamp the angle just under tan()'s 90° asymptote and continue the radius linearly past it, so over-FOV rays
    // stay large and monotonic (no fold back into the frame) and r_limit clips them. See sony.rs / gopro.rs.
    let TMAX = 1.5533; let tt = 57.14902; // TMAX ≈ 89°, tt = tan(TMAX)
    var rr: f32; if (theta < TMAX) { rr = tan(theta); } else { rr = tt + (theta - TMAX) * (1.0 + tt * tt); }
    if (params.r_limit > 0.0 && rr > params.r_limit) { return vec2<f32>(-99999.0, -99999.0); }
    return pos * (rr / r);
}

fn distort_point(x: f32, y: f32, z: f32) -> vec2<f32> {
    let pos = vec2<f32>(x, y) / z;
    let n = sony_segments();
    if (n == 0) { return pos; }
    let r = length(pos);
    if (r < 1e-9) { return pos; }
    // Inverse of undistort_point's angle clamp
    let TMAX = 1.5533; let tt = 57.14902;
    var theta: f32; if (r < tt) { theta = atan(r); } else { theta = TMAX + (r - tt) / (1.0 + tt * tt); }
    return pos * (sony_radius_at(n, theta) / r);
}
