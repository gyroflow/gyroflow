// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// The projection of a z=1 plane point, as distort_point has it
fn opencv_standard_plane(pos: vec2<f32>) -> vec2<f32> {
    let x = pos.x; let y = pos.y;
    let r2 = x * x + y * y;
    let rad = (1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * params.k2.x))) / (1.0 + r2 * (params.k2.y + r2 * (params.k2.z + r2 * params.k2.w)));
    return vec2<f32>(x * rad + params.k1.z * 2.0 * x * y + params.k1.w * (r2 + 2.0 * x * x) + params.k3.x * r2 + params.k3.y * r2 * r2,
                     y * rad + params.k1.z * (r2 + 2.0 * y * y) + params.k1.w * 2.0 * x * y + params.k3.z * r2 + params.k3.w * r2 * r2);
}
// Its Jacobian: (dx/du, dx/dv, dy/du, dy/dv)
fn opencv_standard_jacobian(pos: vec2<f32>) -> vec4<f32> {
    let x = pos.x; let y = pos.y;
    let r2 = x * x + y * y;
    let cd  = 1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * params.k2.x));
    let cd2 = 1.0 + r2 * (params.k2.y + r2 * (params.k2.z + r2 * params.k2.w));
    let rad = cd / cd2;
    // d(rad)/d(r2)
    let drad = (params.k1.x + r2 * (2.0 * params.k1.y + 3.0 * params.k2.x * r2) - rad * (params.k2.y + r2 * (2.0 * params.k2.z + 3.0 * params.k2.w * r2))) / cd2;
    // d(thin prism)/d(r2)
    let sx = params.k3.x + 2.0 * params.k3.y * r2;
    let sy = params.k3.z + 2.0 * params.k3.w * r2;
    let p1 = params.k1.z; let p2 = params.k1.w;
    return vec4<f32>(rad + 2.0 * x * x * drad + 2.0 * p1 * y + 6.0 * p2 * x + 2.0 * x * sx,
                     2.0 * x * y * drad       + 2.0 * p1 * x + 2.0 * p2 * y + 2.0 * y * sx,
                     2.0 * x * y * drad       + 2.0 * p1 * x + 2.0 * p2 * y + 2.0 * x * sy,
                     rad + 2.0 * y * y * drad + 6.0 * p1 * y + 2.0 * p2 * x + 2.0 * y * sy);
}

// Newton on the whole map from pos (see polish_inverse in distortion_models/mod.rs): the point, and how far its image
// still is from p - 1e30 where the map has turned over at the point, a ray of the branch the camera doesn't image with
fn opencv_standard_polish(seed: vec2<f32>, p: vec2<f32>) -> vec3<f32> {
    var pos = seed;
    var e = opencv_standard_plane(pos) - p;
    var err = max(abs(e.x), abs(e.y));
    let tol = 5e-7 * (1.0 + length(p));
    for (var attempt: i32 = 0; attempt < 2; attempt = attempt + 1) {
        for (var it: i32 = 0; it < 8; it = it + 1) {
            if (!(err > tol)) { break; }
            let j = opencv_standard_jacobian(pos);
            let det = j.x * j.w - j.y * j.z;
            if (!(det > 0.0)) { break; }
            let step = vec2<f32>(j.w * e.x - j.y * e.y, j.x * e.y - j.z * e.x) / det;
            var lambda = 1.0;
            var moved = false;
            for (var b: i32 = 0; b < 4; b = b + 1) {
                let c = pos - lambda * step;
                let ce = opencv_standard_plane(c) - p;
                let cerr = max(abs(ce.x), abs(ce.y));
                if (cerr < err) { pos = c; e = ce; err = cerr; moved = true; break; }
                lambda = lambda * 0.5;
            }
            if (!moved) { break; }
        }
        let jf = opencv_standard_jacobian(pos);
        if (jf.x * jf.w - jf.y * jf.z > 0.0) { if (attempt == 1 && !(err <= 1e-6 * (1.0 + length(p)))) { err = 1e30; } break; }
        // A point past a fold of the map has no ray: the camera images the other branch. Where that fold is the edge of
        // the image, the point it images can be just short of it: back towards the axis until the map keeps its orientation,
        // and Newton from there; only a point it then reaches counts (see polish_inverse in distortion_models/mod.rs)
        err = 1e30;
        if (attempt == 1) { break; }
        let pos0 = pos;
        var s = 1e-3;
        var onto = false;
        for (var r: i32 = 0; r < 8; r = r + 1) {
            let c = pos0 * (1.0 - s);
            let cj = opencv_standard_jacobian(c);
            if (cj.x * cj.w - cj.y * cj.z > 0.0) { pos = c; onto = true; break; }
            s = s * 2.0;
        }
        if (!onto) { break; }
        e = opencv_standard_plane(pos) - p;
        err = max(abs(e.x), abs(e.y));
    }
    return vec3<f32>(pos, err);
}
fn opencv_standard_radial(r: f32) -> f32 {
    let r2 = r * r;
    return r * (1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * params.k2.x))) / (1.0 + r2 * (params.k2.y + r2 * (params.k2.z + r2 * params.k2.w)));
}
// The plane radius whose radial image is r_d: bracketed from the axis outwards and bisected (see opencv_standard.rs).
// -1 where the curve turns back, or runs out, before it reaches r_d
fn opencv_standard_solve_radial(r_d: f32) -> f32 {
    var lo = 0.0;
    var hi = r_d;
    var g_hi = opencv_standard_radial(hi);
    for (var n: i32 = 0; n < 31; n = n + 1) {
        if (g_hi >= r_d) { break; }
        let g_next = opencv_standard_radial(hi * 2.0);
        if (!(g_next > g_hi) || n == 30) { return -1.0; }
        lo = hi;
        hi = hi * 2.0;
        g_hi = g_next;
    }
    for (var i: i32 = 0; i < 40; i = i + 1) {
        let mid = 0.5 * (lo + hi);
        if (opencv_standard_radial(mid) < r_d) { lo = mid; } else { hi = mid; }
    }
    return 0.5 * (lo + hi);
}

fn undistort_point(pos_param: vec2<f32>) -> vec2<f32> {
    var pos = pos_param;

    let start_pos = pos;

    // compensate distortion iteratively
    for (var i: i32 = 0; i < 20; i = i + 1) {
        let r2 = pos.x * pos.x + pos.y * pos.y;
        let icdist = (1.0 + ((params.k2.w * r2 + params.k2.z) * r2 + params.k2.y) * r2)/(1.0 + ((params.k2.x * r2 + params.k1.y) * r2 + params.k1.x) * r2);
        if (icdist < 0.0) {
            return vec2<f32>(-99999.0, -99999.0);
        }
        let delta_x = 2.0 * params.k1.z * pos.x * pos.y + params.k1.w * (r2 + 2.0 * pos.x * pos.x)+ params.k3.x * r2 + params.k3.y * r2 * r2;
        let delta_y = params.k1.z * (r2 + 2.0 * pos.y * pos.y) + 2.0 * params.k1.w * pos.x * pos.y+ params.k3.z * r2 + params.k3.w * r2 * r2;
        pos = vec2<f32>(
            (start_pos.x - delta_x) * icdist,
            (start_pos.y - delta_y) * icdist
        );
    }

    // That is a fixed point with the tangential and thin-prism terms held still, and it only contracts while they and
    // the slope of the radial curve are small: a wide lens with large ones is left hundreds of pixels off in the
    // corners. Newton on the whole map from there (see polish_inverse in distortion_models/mod.rs); where neither
    // finds the point, the lens has no ray for it
    if (!(abs(pos.x) < 1e30 && abs(pos.y) < 1e30)) { pos = start_pos; } // not finite
    let tol = 1e-4 * (1.0 + length(start_pos));
    var q = opencv_standard_polish(pos, start_pos);
    if (!(q.z <= tol)) {
        // A steep radial curve throws the fixed point between two far-apart values, and Newton from either is too slow
        // to arrive: start again from the radial curve's own inverse, which for radial terms only is the answer
        let r_d = length(start_pos);
        let r = opencv_standard_solve_radial(r_d);
        if (r >= 0.0 && r_d > 1e-12) { q = opencv_standard_polish(start_pos * (r / r_d), start_pos); }
    }
    if (!(q.z <= tol)) { return vec2<f32>(-99999.0, -99999.0); }
    pos = q.xy;

    // The solve runs in the z=1 plane; the pipeline carries rays as angle vectors
    let r = length(pos);
    if (r < 1e-12) { return pos; }
    return pos * (atan(r) / r);
}

fn distort_point(x: f32, y: f32, z: f32) -> vec2<f32> {
    // A rectilinear model has no image of a ray at or past 90°: send it far outside the frame
    var pos = vec2<f32>(x, y) * 1e9;
    if (z > 1e-9) { pos = vec2<f32>(x, y) / z; }
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
