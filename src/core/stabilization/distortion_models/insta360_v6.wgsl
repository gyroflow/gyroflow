// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

// Insta360's offset_v6 lens: the unified sphere with a 13-term distortion block, k = d0..d12, xi
// (k1 = d0..d3, k2 = d4..d7, k3 = d8..d11, k4.x = d12, k4.y = xi). See insta360_v6.rs

// Radial part of the projection: image radius of a unified-sphere plane radius m
fn insta360_v6_radial(m: f32) -> f32 {
    let m2 = m * m;
    return m * (1.0 + m2 * (params.k1.x + m2 * (params.k1.y + m2 * (params.k1.z + m2 * (params.k1.w + m2 * params.k2.x)))));
}
// d(insta360_v6_radial)/dm
fn insta360_v6_radial_derivative(m: f32) -> f32 {
    let m2 = m * m;
    return 1.0 + m2 * (3.0*params.k1.x + m2 * (5.0*params.k1.y + m2 * (7.0*params.k1.z + m2 * (9.0*params.k1.w + m2 * 11.0*params.k2.x))));
}
// The terms that aren't radial: the decentering and the thin prism, at the sphere-plane point uv
fn insta360_v6_non_radial(uv: vec2<f32>) -> vec2<f32> {
    let r2 = dot(uv, uv);
    let px = params.k2.y + params.k2.w * r2; // d5 + d7 r2
    let py = params.k2.z + params.k3.x * r2; // d6 + d8 r2
    return vec2<f32>(
        (r2 + 2.0*uv.x*uv.x) * px + 2.0*uv.x*uv.y * py + params.k3.y * r2 + params.k3.w * r2*r2, // d9, d11
        (r2 + 2.0*uv.y*uv.y) * py + 2.0*uv.x*uv.y * px + params.k3.z * r2 + params.k4.x * r2*r2  // d10, d12
    );
}
// A bound on how far the terms that aren't radial move a point of plane radius sqrt(m2): past radial(m) + this no point
// of radius up to m lands. See insta360_v6.rs
fn insta360_v6_non_radial_bound(m2: f32) -> f32 {
    return m2 * (4.0 * (abs(params.k2.y) + abs(params.k2.z) + (abs(params.k2.w) + abs(params.k3.x)) * m2) + abs(params.k3.y) + abs(params.k3.z) + (abs(params.k3.w) + abs(params.k4.x)) * m2);
}
// How far the sphere reaches: it folds at theta = acos(-1/xi), and for xi <= 1 it never folds, which the placeholder
// stands for (1e3, not insta360's 1e4: a degree-11 polynomial of 1e4 is past f32). See insta360_v6.rs
fn insta360_v6_m_max(xi: f32) -> f32 {
    if (xi > 1.0) { return 1.0 / sqrt(xi * xi - 1.0); }
    return 1e3;
}
// Newton on insta360_v6_radial(m) = r, bracketed to the branch the camera projects with. See insta360.wgsl
fn insta360_v6_solve_radial(r: f32, guess: f32, m_max: f32) -> f32 {
    var lo = 0.0;
    var hi = m_max;
    var m = clamp(guess, 0.0, m_max);
    for (var i: i32 = 0; i < 20; i = i + 1) {
        let d = insta360_v6_radial_derivative(m);
        if (d <= 1e-9) {
            hi = m;
            m = 0.5 * (lo + hi);
        } else {
            let f = insta360_v6_radial(m) - r;
            if (abs(f) <= 2e-7 * (1.0 + r)) { break; }
            if (f < 0.0) { lo = m; } else { hi = m; }
            let next = m - f / d;
            if (next >= lo && next <= hi) { m = next; } else { m = 0.5 * (lo + hi); }
        }
    }
    return m;
}

// The projection of a unified-sphere plane point, as distort_point has it
fn insta360_v6_plane(uv: vec2<f32>) -> vec2<f32> {
    let r2 = dot(uv, uv);
    let rad = 1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * (params.k1.z + r2 * (params.k1.w + r2 * params.k2.x))));
    return uv * rad + insta360_v6_non_radial(uv);
}
// Its Jacobian: (dx/du, dx/dv, dy/du, dy/dv)
fn insta360_v6_plane_jacobian(uv: vec2<f32>) -> vec4<f32> {
    let u = uv.x; let v = uv.y;
    let r2 = u*u + v*v;
    let rad = 1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * (params.k1.z + r2 * (params.k1.w + r2 * params.k2.x))));
    let drad = params.k1.x + r2 * (2.0*params.k1.y + r2 * (3.0*params.k1.z + r2 * (4.0*params.k1.w + r2 * 5.0*params.k2.x))); // d(rad)/d(r2)
    let px = params.k2.y + params.k2.w * r2; // d5 + d7 r2
    let py = params.k2.z + params.k3.x * r2; // d6 + d8 r2
    // d(non-radial)/d(r2), with the u, v of its other factors held still
    let qx = (r2 + 2.0*u*u) * params.k2.w + 2.0*u*v * params.k3.x + params.k3.y + 2.0 * params.k3.w * r2;
    let qy = (r2 + 2.0*v*v) * params.k3.x + 2.0*u*v * params.k2.w + params.k3.z + 2.0 * params.k4.x * r2;
    return vec4<f32>(rad + 2.0*u*u*drad + 6.0*u*px + 2.0*v*py + 2.0*u*qx,
                     2.0*u*v*drad       + 2.0*v*px + 2.0*u*py + 2.0*v*qx,
                     2.0*u*v*drad       + 2.0*u*py + 2.0*v*px + 2.0*u*qy,
                     rad + 2.0*v*v*drad + 6.0*v*py + 2.0*u*px + 2.0*v*qy);
}

fn distort_point(px: f32, py: f32, pz: f32) -> vec2<f32> {
    let xi = params.k4.y;

    var p = vec3<f32>(px, py, pz);
    p /= length(p);
    let uv = vec2<f32>(p.x, p.y) / (p.z + xi);

    let r2 = dot(uv, uv);
    let radial = 1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * (params.k1.z + r2 * (params.k1.w + r2 * params.k2.x))));
    return uv * radial + insta360_v6_non_radial(uv);
}

fn undistort_point(p: vec2<f32>) -> vec2<f32> {
    let xi = params.k4.y;

    let r_d = length(p);
    if (r_d < 1e-9) { return p; }

    // Past the end of the projection's rising branch there is no ray - but only where the sphere's own end is that end
    let m_max = insta360_v6_m_max(xi);
    // And only past what the terms that aren't radial add to it: they carry the image of the sphere's rim out of that
    // circle in some directions, and the 2D solve below decides there (see insta360_v6.rs / insta360.rs)
    let sphere_ends = insta360_v6_radial_derivative(m_max) > 0.0;
    if (sphere_ends && insta360_v6_radial(m_max) + insta360_v6_non_radial_bound(m_max * m_max) < r_d) { return vec2<f32>(-99999.0, -99999.0); }
    // Between the two only a point the solve reaches is one: what it stalls at short of a point out there is the edge of
    // the image, and the point past it
    let past_the_rim = sphere_ends && insta360_v6_radial(m_max) < r_d;

    // Strip the terms that aren't radial, invert the radial polynomial, re-evaluate them
    var a = p;
    var uv = vec2<f32>(0.0, 0.0);
    var m = r_d;
    var r_s = r_d;
    for (var i: i32 = 0; i < 4; i = i + 1) {
        r_s = length(a);
        m = insta360_v6_solve_radial(r_s, m, m_max);
        if (r_s > 1e-12) { uv = a * (m / r_s); } else { uv = vec2<f32>(0.0, 0.0); }
        a = p - insta360_v6_non_radial(uv);
    }
    // Large ones leave that pixels off: Newton on the whole map finishes from there, inside the sphere's fold and never
    // from past a fold of the map (see polish_inverse in distortion_models/mod.rs)
    var e = insta360_v6_plane(uv) - p;
    var err = max(abs(e.x), abs(e.y));
    let tol = 5e-7 * (1.0 + r_d);
    var exact_only = past_the_rim;
    for (var attempt: i32 = 0; attempt < 2; attempt = attempt + 1) {
        for (var it: i32 = 0; it < 8; it = it + 1) {
            if (!(err > tol)) { break; }
            let j = insta360_v6_plane_jacobian(uv);
            let det = j.x * j.w - j.y * j.z;
            if (!(det > 0.0)) { break; }
            let step = vec2<f32>(j.w * e.x - j.y * e.y, j.x * e.y - j.z * e.x) / det;
            var lambda = 1.0;
            var moved = false;
            for (var b: i32 = 0; b < 4; b = b + 1) {
                let c = uv - lambda * step;
                if (dot(c, c) <= m_max * m_max) {
                    let ce = insta360_v6_plane(c) - p;
                    let cerr = max(abs(ce.x), abs(ce.y));
                    if (cerr < err) { uv = c; e = ce; err = cerr; moved = true; break; }
                }
                lambda = lambda * 0.5;
            }
            if (!moved) { break; }
        }
        let jf = insta360_v6_plane_jacobian(uv);
        if (jf.x * jf.w - jf.y * jf.z > 0.0) { break; }
        // A point past a fold of the map has no ray: the camera images the other branch. Where that fold is the edge of
        // the image, the point it images can be just short of it: back towards the axis until the map keeps its orientation,
        // and Newton from there; only a point it then reaches counts (see polish_inverse in distortion_models/mod.rs)
        err = 1e30;
        if (attempt == 1) { break; }
        let uv0 = uv;
        var s = 1e-3;
        var onto = false;
        for (var r: i32 = 0; r < 8; r = r + 1) {
            let c = uv0 * (1.0 - s);
            let cj = insta360_v6_plane_jacobian(c);
            if (cj.x * cj.w - cj.y * cj.z > 0.0) { uv = c; onto = true; break; }
            s = s * 2.0;
        }
        if (!onto) { break; }
        e = insta360_v6_plane(uv) - p;
        err = max(abs(e.x), abs(e.y));
        exact_only = true;
    }
    // How far the map lands from the point is the test of whether there was a ray here at all
    var accept_err = 1e-4 * (1.0 + r_d);
    if (exact_only) { accept_err = 1e-6 * (1.0 + r_d); }
    if (!(err <= accept_err)) { return vec2<f32>(-99999.0, -99999.0); }

    // Unified sphere -> ray angle: the sphere point (u*alpha, v*alpha, alpha - xi) is a unit vector
    let rho2 = dot(uv, uv);
    let disc = 1.0 + (1.0 - xi*xi) * rho2;
    if (disc < 0.0) { return vec2<f32>(-99999.0, -99999.0); }
    let alpha = (xi + sqrt(disc)) / (rho2 + 1.0);
    let rho = sqrt(rho2);
    if (rho < 1e-12) { return vec2<f32>(0.0, 0.0); }
    return uv * (atan2(rho * alpha, alpha - xi) / rho);
}
