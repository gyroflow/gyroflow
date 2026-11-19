// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

// Insta360's offset_v6 lens: the unified sphere with a 13-term distortion block, k = d0..d12, xi
// (k1 = d0..d3, k2 = d4..d7, k3 = d8..d11, k4.x = d12, k4.y = xi). See insta360_v6.rs

// Radial part of the projection: image radius of a unified-sphere plane radius m
float insta360_v6_radial(float m) {
    float m2 = m * m;
    return m * (1.0 + m2 * (params.k1.x + m2 * (params.k1.y + m2 * (params.k1.z + m2 * (params.k1.w + m2 * params.k2.x)))));
}
// d(insta360_v6_radial)/dm
float insta360_v6_radial_derivative(float m) {
    float m2 = m * m;
    return 1.0 + m2 * (3.0*params.k1.x + m2 * (5.0*params.k1.y + m2 * (7.0*params.k1.z + m2 * (9.0*params.k1.w + m2 * 11.0*params.k2.x))));
}
// The terms that aren't radial: the decentering and the thin prism, at the sphere-plane point uv
vec2 insta360_v6_non_radial(vec2 uv) {
    float r2 = dot(uv, uv);
    float px = params.k2.y + params.k2.w * r2; // d5 + d7 r2
    float py = params.k2.z + params.k3.x * r2; // d6 + d8 r2
    return vec2(
        (r2 + 2.0*uv.x*uv.x) * px + 2.0*uv.x*uv.y * py + params.k3.y * r2 + params.k3.w * r2*r2, // d9, d11
        (r2 + 2.0*uv.y*uv.y) * py + 2.0*uv.x*uv.y * px + params.k3.z * r2 + params.k4.x * r2*r2  // d10, d12
    );
}
// A bound on how far the terms that aren't radial move a point of plane radius sqrt(m2): past radial(m) + this no point
// of radius up to m lands. See insta360_v6.rs
float insta360_v6_non_radial_bound(float m2) {
    return m2 * (4.0 * (abs(params.k2.y) + abs(params.k2.z) + (abs(params.k2.w) + abs(params.k3.x)) * m2) + abs(params.k3.y) + abs(params.k3.z) + (abs(params.k3.w) + abs(params.k4.x)) * m2);
}
// How far the sphere reaches: it folds at theta = acos(-1/xi), and for xi <= 1 it never folds, which the placeholder
// stands for (1e3, not insta360's 1e4: a degree-11 polynomial of 1e4 is past float). See insta360_v6.rs
float insta360_v6_m_max(float xi) {
    return xi > 1.0? 1.0 / sqrt(xi * xi - 1.0) : 1e3;
}
// Newton on insta360_v6_radial(m) = r, bracketed to the branch the camera projects with. See insta360.glsl
float insta360_v6_solve_radial(float r, float guess, float m_max) {
    float lo = 0.0, hi = m_max;
    float m = clamp(guess, 0.0, m_max);
    for (int i = 0; i < 20; i++) {
        float d = insta360_v6_radial_derivative(m);
        if (d <= 1e-9) {
            hi = m;
            m = 0.5 * (lo + hi);
        } else {
            float f = insta360_v6_radial(m) - r;
            if (abs(f) <= 2e-7 * (1.0 + r)) { break; }
            if (f < 0.0) { lo = m; } else { hi = m; }
            float next = m - f / d;
            m = (next >= lo && next <= hi)? next : 0.5 * (lo + hi);
        }
    }
    return m;
}

// The projection of a unified-sphere plane point, as distort_point has it
vec2 insta360_v6_plane(vec2 uv) {
    float r2 = dot(uv, uv);
    float rad = 1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * (params.k1.z + r2 * (params.k1.w + r2 * params.k2.x))));
    return uv * rad + insta360_v6_non_radial(uv);
}
// Its Jacobian: (dx/du, dx/dv, dy/du, dy/dv)
vec4 insta360_v6_plane_jacobian(vec2 uv) {
    float u = uv.x, v = uv.y;
    float r2 = u*u + v*v;
    float rad = 1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * (params.k1.z + r2 * (params.k1.w + r2 * params.k2.x))));
    float drad = params.k1.x + r2 * (2.0*params.k1.y + r2 * (3.0*params.k1.z + r2 * (4.0*params.k1.w + r2 * 5.0*params.k2.x))); // d(rad)/d(r2)
    float px = params.k2.y + params.k2.w * r2; // d5 + d7 r2
    float py = params.k2.z + params.k3.x * r2; // d6 + d8 r2
    // d(non-radial)/d(r2), with the u, v of its other factors held still
    float qx = (r2 + 2.0*u*u) * params.k2.w + 2.0*u*v * params.k3.x + params.k3.y + 2.0 * params.k3.w * r2;
    float qy = (r2 + 2.0*v*v) * params.k3.x + 2.0*u*v * params.k2.w + params.k3.z + 2.0 * params.k4.x * r2;
    return vec4(rad + 2.0*u*u*drad + 6.0*u*px + 2.0*v*py + 2.0*u*qx,
                2.0*u*v*drad       + 2.0*v*px + 2.0*u*py + 2.0*v*qx,
                2.0*u*v*drad       + 2.0*u*py + 2.0*v*px + 2.0*u*qy,
                rad + 2.0*v*v*drad + 6.0*v*py + 2.0*u*px + 2.0*v*qy);
}

vec2 distort_point(float x, float y, float z) {
    float xi = params.k4.y;

    float len = length(vec3(x, y, z));
    vec2 uv = vec2(x / len, y / len) / ((z / len) + xi);

    float r2 = dot(uv, uv);
    float radial = 1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * (params.k1.z + r2 * (params.k1.w + r2 * params.k2.x))));
    return uv * radial + insta360_v6_non_radial(uv);
}

vec2 undistort_point(vec2 p) {
    float xi = params.k4.y;

    float r_d = length(p);
    if (r_d < 1e-9) { return p; }

    // Past the end of the projection's rising branch there is no ray - but only where the sphere's own end is that end
    float m_max = insta360_v6_m_max(xi);
    // And only past what the terms that aren't radial add to it: they carry the image of the sphere's rim out of that
    // circle in some directions, and the 2D solve below decides there (see insta360_v6.rs / insta360.rs)
    bool sphere_ends = insta360_v6_radial_derivative(m_max) > 0.0;
    if (sphere_ends && insta360_v6_radial(m_max) + insta360_v6_non_radial_bound(m_max * m_max) < r_d) { return vec2(-99999.0, -99999.0); }
    // Between the two only a point the solve reaches is one: what it stalls at short of a point out there is the edge of
    // the image, and the point past it
    bool past_the_rim = sphere_ends && insta360_v6_radial(m_max) < r_d;

    // Strip the terms that aren't radial, invert the radial polynomial, re-evaluate them
    vec2 a = p;
    vec2 uv = vec2(0.0, 0.0);
    float m = r_d;
    float r_s = r_d;
    for (int i = 0; i < 4; i++) {
        r_s = length(a);
        m = insta360_v6_solve_radial(r_s, m, m_max);
        uv = r_s > 1e-12? a * (m / r_s) : vec2(0.0, 0.0);
        a = p - insta360_v6_non_radial(uv);
    }
    // Large ones leave that pixels off: Newton on the whole map finishes from there, inside the sphere's fold and never
    // from past a fold of the map (see polish_inverse in distortion_models/mod.rs)
    vec2 e = insta360_v6_plane(uv) - p;
    float err = max(abs(e.x), abs(e.y));
    float tol = 5e-7 * (1.0 + r_d);
    bool exact_only = past_the_rim;
    for (int attempt = 0; attempt < 2; ++attempt) {
        for (int it = 0; it < 8; ++it) {
            if (!(err > tol)) break;
            vec4 j = insta360_v6_plane_jacobian(uv);
            float det = j.x * j.w - j.y * j.z;
            if (!(det > 0.0)) break;
            vec2 step = vec2(j.w * e.x - j.y * e.y, j.x * e.y - j.z * e.x) / det;
            float lambda = 1.0;
            bool moved = false;
            for (int b = 0; b < 4; ++b) {
                vec2 c = uv - lambda * step;
                if (dot(c, c) <= m_max * m_max) {
                    vec2 ce = insta360_v6_plane(c) - p;
                    float cerr = max(abs(ce.x), abs(ce.y));
                    if (cerr < err) { uv = c; e = ce; err = cerr; moved = true; break; }
                }
                lambda *= 0.5;
            }
            if (!moved) break;
        }
        vec4 jf = insta360_v6_plane_jacobian(uv);
        if (jf.x * jf.w - jf.y * jf.z > 0.0) { break; }
        // A point past a fold of the map has no ray: the camera images the other branch. Where that fold is the edge of
        // the image, the point it images can be just short of it: back towards the axis until the map keeps its orientation,
        // and Newton from there; only a point it then reaches counts (see polish_inverse in distortion_models/mod.rs)
        err = 1e30;
        if (attempt == 1) { break; }
        vec2 uv0 = uv;
        float s = 1e-3;
        bool onto = false;
        for (int r = 0; r < 8; ++r) {
            vec2 c = uv0 * (1.0 - s);
            vec4 cj = insta360_v6_plane_jacobian(c);
            if (cj.x * cj.w - cj.y * cj.z > 0.0) { uv = c; onto = true; break; }
            s *= 2.0;
        }
        if (!onto) { break; }
        e = insta360_v6_plane(uv) - p;
        err = max(abs(e.x), abs(e.y));
        exact_only = true;
    }
    // How far the map lands from the point is the test of whether there was a ray here at all
    float accept_err = 1e-4 * (1.0 + r_d);
    if (exact_only) { accept_err = 1e-6 * (1.0 + r_d); }
    if (!(err <= accept_err)) { return vec2(-99999.0, -99999.0); }

    // Unified sphere -> ray angle: the sphere point (u*alpha, v*alpha, alpha - xi) is a unit vector
    float rho2 = dot(uv, uv);
    float disc = 1.0 + (1.0 - xi*xi) * rho2;
    if (disc < 0.0) { return vec2(-99999.0, -99999.0); }
    float alpha = (xi + sqrt(disc)) / (rho2 + 1.0);
    float rho = sqrt(rho2);
    if (rho < 1e-12) { return vec2(0.0, 0.0); }
    return uv * (atan(rho * alpha, alpha - xi) / rho);
}
