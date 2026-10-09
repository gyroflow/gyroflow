// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Radial part of the projection: image radius of a unified-sphere plane radius `m`
fn radial_projection(m: f32, k1: f32, k2: f32, k3: f32) -> f32 {
    let m2 = m * m;
    return m * (1.0 + k1*m2 + k2*m2*m2 + k3*m2*m2*m2);
}
// d(radial_projection)/dm
fn radial_derivative(m: f32, k1: f32, k2: f32, k3: f32) -> f32 {
    let m2 = m * m;
    return 1.0 + 3.0*k1*m2 + 5.0*k2*m2*m2 + 7.0*k3*m2*m2*m2;
}
// How far the sphere reaches: it folds at theta = acos(-1/xi) (120 degrees for the X5's xi = 2), and for
// xi <= 1 it never folds but runs to infinity, which the placeholder stands for. Never a bound on the image
// radius on its own - the radial polynomial folds where radial_derivative first reaches zero, which can come
// first. See insta360.rs
fn insta360_m_max(xi: f32) -> f32 {
    if (xi > 1.0) { return 1.0 / sqrt(xi * xi - 1.0); }
    return 1e4;
}
// Newton on `radial_projection(m) = r`, kept on the branch the camera projects with: a step that lands where
// the curve has stopped rising has stepped over its fold, and pulling the bracket in to there walks up to the
// fold instead of across it. Returns the end of the branch when `r` is past everything the lens images
fn solve_radial(r: f32, guess: f32, m_max: f32, k1: f32, k2: f32, k3: f32) -> f32 {
    var lo = 0.0;
    var hi = m_max;
    var m = clamp(guess, 0.0, m_max);
    for (var i: i32 = 0; i < 20; i = i + 1) { // Newton is done in a few; the rest are the fold's bisection
        let d = radial_derivative(m, k1, k2, k3);
        if (d <= 1e-9) {
            hi = m; // folded at or before m; the root, if there is one, is below it
            m = 0.5 * (lo + hi);
        } else {
            let f = radial_projection(m, k1, k2, k3) - r;
            // Solved, judged on the residual and not on the size of the step: a step below an ulp of m is
            // one a GPU computes exactly and then rounds away, and a test on it never passes. See insta360.rs
            if (abs(f) <= 2e-7 * (1.0 + r)) { break; }
            if (f < 0.0) { lo = m; } else { hi = m; }
            let next = m - f / d;
            // A step landing on an end of the bracket is a step of nothing - kept, not read as outside the
            // bracket and replaced by the midpoint of a bracket whose far end may still be m_max
            if (next >= lo && next <= hi) { m = next; } else { m = 0.5 * (lo + hi); } // outside (or NaN): bisect
        }
    }
    return m;
}

fn distort_point(px: f32, py: f32, pz: f32) -> vec2<f32> {
    let k1 = params.k1.x;
    let k2 = params.k1.y;
    let k3 = params.k1.z;
    let p1 = params.k1.w;

    let p2 = params.k2.x;
    let xi = params.k2.y;

    var p = vec3<f32>(px, py, pz);
    p /= length(p);

    let x = p.x / (p.z + xi);
    let y = p.y / (p.z + xi);

    let r2 = x*x + y*y;
    let r4 = r2 * r2;
    let r6 = r4 * r2;

    return vec2<f32>(
        x * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p1*x*y + p2*(r2 + 2.0*x*x),
        y * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p2*x*y + p1*(r2 + 2.0*y*y)
    );
}

fn undistort_point(p: vec2<f32>) -> vec2<f32> {
    let k1 = params.k1.x;
    let k2 = params.k1.y;
    let k3 = params.k1.z;
    let p1 = params.k1.w;
    let p2 = params.k2.x;
    let xi = params.k2.y;

    let r_d = length(p);
    if (r_d < 1e-9) { return p; }

    // Past the end of the projection's rising branch there is no ray at all - but only where the sphere's
    // own end is that end; where the polynomial folds first, radial_projection(m_max) is off the far side of
    // the fold and says nothing about the largest radius the lens images
    let m_max = insta360_m_max(xi);
    if (radial_derivative(m_max, k1, k2, k3) > 0.0 && radial_projection(m_max, k1, k2, k3) < r_d) { return vec2<f32>(-99999.0, -99999.0); }

    // Strip the tangential terms, invert the radial polynomial, re-evaluate them
    var a = p;
    var uv = vec2<f32>(0.0, 0.0);
    var m = r_d;
    var r_s = r_d;
    for (var i: i32 = 0; i < 3; i = i + 1) {
        r_s = length(a);
        m = solve_radial(r_s, m, m_max, k1, k2, k3);
        if (r_s > 1e-12) { uv = a * (m / r_s); } else { uv = vec2<f32>(0.0, 0.0); }
        let r2 = dot(uv, uv);
        a = p - vec2<f32>(2.0*p1*uv.x*uv.y + p2*(r2 + 2.0*uv.x*uv.x),
                          2.0*p2*uv.x*uv.y + p1*(r2 + 2.0*uv.y*uv.y));
    }
    // The radius the solve actually reached is the test of whether there was a ray here at all
    if (abs(radial_projection(m, k1, k2, k3) - r_s) > 1e-4 * (1.0 + r_s)) { return vec2<f32>(-99999.0, -99999.0); }

    // Unified sphere -> ray angle. The sphere point is (u*alpha, v*alpha, alpha - xi), already a unit
    // vector, so the angle falls out of atan2 - where the old x/z went to infinity at 90 degrees
    let rho2 = dot(uv, uv);
    let disc = 1.0 + (1.0 - xi*xi) * rho2;
    if (disc < 0.0) { return vec2<f32>(-99999.0, -99999.0); }
    let alpha = (xi + sqrt(disc)) / (rho2 + 1.0);
    let rho = sqrt(rho2);
    if (rho < 1e-12) { return vec2<f32>(0.0, 0.0); }
    return uv * (atan2(rho * alpha, alpha - xi) / rho);
}
