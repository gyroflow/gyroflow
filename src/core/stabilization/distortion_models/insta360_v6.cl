// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

// Insta360's offset_v6 lens: the unified sphere with a 13-term distortion block, k = d0..d12, xi. See insta360_v6.rs

// Radial part of the projection: image radius of a unified-sphere plane radius m
float insta360_v6_radial(float m, __global KernelParams *params) {
    float m2 = m * m;
    return m * (1.0f + m2 * (params->k[0] + m2 * (params->k[1] + m2 * (params->k[2] + m2 * (params->k[3] + m2 * params->k[4])))));
}
// d(insta360_v6_radial)/dm
float insta360_v6_radial_derivative(float m, __global KernelParams *params) {
    float m2 = m * m;
    return 1.0f + m2 * (3.0f*params->k[0] + m2 * (5.0f*params->k[1] + m2 * (7.0f*params->k[2] + m2 * (9.0f*params->k[3] + m2 * 11.0f*params->k[4]))));
}
// The terms that aren't radial: the decentering and the thin prism, at the sphere-plane point uv
float2 insta360_v6_non_radial(float2 uv, __global KernelParams *params) {
    float r2 = dot(uv, uv);
    float px = params->k[5] + params->k[7] * r2;
    float py = params->k[6] + params->k[8] * r2;
    return (float2)(
        (r2 + 2.0f*uv.x*uv.x) * px + 2.0f*uv.x*uv.y * py + params->k[9]  * r2 + params->k[11] * r2*r2,
        (r2 + 2.0f*uv.y*uv.y) * py + 2.0f*uv.x*uv.y * px + params->k[10] * r2 + params->k[12] * r2*r2
    );
}
// A bound on how far the terms that aren't radial move a point of plane radius sqrt(m2): past radial(m) + this no point
// of radius up to m lands. See insta360_v6.rs
float insta360_v6_non_radial_bound(float m2, __global KernelParams *params) {
    return m2 * (4.0f * (fabs(params->k[5]) + fabs(params->k[6]) + (fabs(params->k[7]) + fabs(params->k[8])) * m2) + fabs(params->k[9]) + fabs(params->k[10]) + (fabs(params->k[11]) + fabs(params->k[12])) * m2);
}
// How far the sphere reaches: it folds at theta = acos(-1/xi), and for xi <= 1 it never folds, which the placeholder
// stands for (1e3, not insta360's 1e4: a degree-11 polynomial of 1e4 is past float). See insta360_v6.rs
float insta360_v6_m_max(float xi) {
    if (xi > 1.0f) { return 1.0f / sqrt(xi * xi - 1.0f); }
    return 1e3f;
}
// Newton on insta360_v6_radial(m) = r, bracketed to the branch the camera projects with. See insta360.cl
float insta360_v6_solve_radial(float r, float guess, float m_max, __global KernelParams *params) {
    float lo = 0.0f;
    float hi = m_max;
    float m = clamp(guess, 0.0f, m_max);
    for (int i = 0; i < 20; i++) {
        float d = insta360_v6_radial_derivative(m, params);
        if (d <= 1e-9f) {
            hi = m;
            m = 0.5f * (lo + hi);
        } else {
            float f = insta360_v6_radial(m, params) - r;
            if (fabs(f) <= 2e-7f * (1.0f + r)) { break; }
            if (f < 0.0f) { lo = m; } else { hi = m; }
            float next = m - f / d;
            m = (next >= lo && next <= hi)? next : 0.5f * (lo + hi);
        }
    }
    return m;
}

// The projection of a unified-sphere plane point, as distort_point has it
float2 insta360_v6_plane(float2 uv, __global KernelParams *params) {
    float r2 = dot(uv, uv);
    float rad = 1.0f + r2 * (params->k[0] + r2 * (params->k[1] + r2 * (params->k[2] + r2 * (params->k[3] + r2 * params->k[4]))));
    return uv * rad + insta360_v6_non_radial(uv, params);
}
// Its Jacobian: (dx/du, dx/dv, dy/du, dy/dv)
float4 insta360_v6_plane_jacobian(float2 uv, __global KernelParams *params) {
    float u = uv.x, v = uv.y;
    float r2 = u*u + v*v;
    float rad = 1.0f + r2 * (params->k[0] + r2 * (params->k[1] + r2 * (params->k[2] + r2 * (params->k[3] + r2 * params->k[4]))));
    float drad = params->k[0] + r2 * (2.0f*params->k[1] + r2 * (3.0f*params->k[2] + r2 * (4.0f*params->k[3] + r2 * 5.0f*params->k[4]))); // d(rad)/d(r2)
    float px = params->k[5] + params->k[7] * r2;
    float py = params->k[6] + params->k[8] * r2;
    // d(non-radial)/d(r2), with the u, v of its other factors held still
    float qx = (r2 + 2.0f*u*u) * params->k[7] + 2.0f*u*v * params->k[8] + params->k[9]  + 2.0f * params->k[11] * r2;
    float qy = (r2 + 2.0f*v*v) * params->k[8] + 2.0f*u*v * params->k[7] + params->k[10] + 2.0f * params->k[12] * r2;
    return (float4)(rad + 2.0f*u*u*drad + 6.0f*u*px + 2.0f*v*py + 2.0f*u*qx,
                    2.0f*u*v*drad       + 2.0f*v*px + 2.0f*u*py + 2.0f*v*qx,
                    2.0f*u*v*drad       + 2.0f*u*py + 2.0f*v*px + 2.0f*u*qy,
                    rad + 2.0f*v*v*drad + 6.0f*v*py + 2.0f*u*px + 2.0f*v*qy);
}

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    float xi = params->k[13];

    float len = sqrt(x*x + y*y + z*z);
    float2 uv = (float2)(x / len, y / len) / ((z / len) + xi);

    float r2 = dot(uv, uv);
    float radial = 1.0f + r2 * (params->k[0] + r2 * (params->k[1] + r2 * (params->k[2] + r2 * (params->k[3] + r2 * params->k[4]))));
    return uv * radial + insta360_v6_non_radial(uv, params);
}

float2 undistort_point(float2 p, __global KernelParams *params) {
    float xi = params->k[13];

    float r_d = length(p);
    if (r_d < 1e-9f) { return p; }

    // Past the end of the projection's rising branch there is no ray - but only where the sphere's own end is that end
    float m_max = insta360_v6_m_max(xi);
    // And only past what the terms that aren't radial add to it: they carry the image of the sphere's rim out of that
    // circle in some directions, and the 2D solve below decides there (see insta360_v6.rs / insta360.rs)
    bool sphere_ends = insta360_v6_radial_derivative(m_max, params) > 0.0f;
    if (sphere_ends && insta360_v6_radial(m_max, params) + insta360_v6_non_radial_bound(m_max * m_max, params) < r_d) { return (float2)(-99999.0f, -99999.0f); }
    // Between the two only a point the solve reaches is one: what it stalls at short of a point out there is the edge of
    // the image, and the point past it
    bool past_the_rim = sphere_ends && insta360_v6_radial(m_max, params) < r_d;

    // Strip the terms that aren't radial, invert the radial polynomial, re-evaluate them
    float2 a = p;
    float2 uv = (float2)(0.0f, 0.0f);
    float m = r_d;
    float r_s = r_d;
    for (int i = 0; i < 4; i++) {
        r_s = length(a);
        m = insta360_v6_solve_radial(r_s, m, m_max, params);
        uv = r_s > 1e-12f? a * (m / r_s) : (float2)(0.0f, 0.0f);
        a = p - insta360_v6_non_radial(uv, params);
    }
    // Large ones leave that pixels off: Newton on the whole map finishes from there, inside the sphere's fold and never
    // from past a fold of the map (see polish_inverse in distortion_models/mod.rs)
    float2 e = insta360_v6_plane(uv, params) - p;
    float err = fmax(fabs(e.x), fabs(e.y));
    float tol = 5e-7f * (1.0f + r_d);
    bool exact_only = past_the_rim;
    for (int attempt = 0; attempt < 2; ++attempt) {
        for (int it = 0; it < 8; ++it) {
            if (!(err > tol)) break;
            float4 j = insta360_v6_plane_jacobian(uv, params);
            float det = j.x * j.w - j.y * j.z;
            if (!(det > 0.0f)) break;
            float2 step = (float2)(j.w * e.x - j.y * e.y, j.x * e.y - j.z * e.x) / det;
            float lambda = 1.0f;
            bool moved = false;
            for (int b = 0; b < 4; ++b) {
                float2 c = uv - lambda * step;
                if (dot(c, c) <= m_max * m_max) {
                    float2 ce = insta360_v6_plane(c, params) - p;
                    float cerr = fmax(fabs(ce.x), fabs(ce.y));
                    if (cerr < err) { uv = c; e = ce; err = cerr; moved = true; break; }
                }
                lambda *= 0.5f;
            }
            if (!moved) break;
        }
        float4 jf = insta360_v6_plane_jacobian(uv, params);
        if (jf.x * jf.w - jf.y * jf.z > 0.0f) { break; }
        // A point past a fold of the map has no ray: the camera images the other branch. Where that fold is the edge of
        // the image, the point it images can be just short of it: back towards the axis until the map keeps its orientation,
        // and Newton from there; only a point it then reaches counts (see polish_inverse in distortion_models/mod.rs)
        err = 1e30f;
        if (attempt == 1) { break; }
        float2 uv0 = uv;
        float s = 1e-3f;
        bool onto = false;
        for (int r = 0; r < 8; ++r) {
            float2 c = uv0 * (1.0f - s);
            float4 cj = insta360_v6_plane_jacobian(c, params);
            if (cj.x * cj.w - cj.y * cj.z > 0.0f) { uv = c; onto = true; break; }
            s *= 2.0f;
        }
        if (!onto) { break; }
        e = insta360_v6_plane(uv, params) - p;
        err = fmax(fabs(e.x), fabs(e.y));
        exact_only = true;
    }
    // How far the map lands from the point is the test of whether there was a ray here at all
    float accept_err = 1e-4f * (1.0f + r_d);
    if (exact_only) { accept_err = 1e-6f * (1.0f + r_d); }
    if (!(err <= accept_err)) { return (float2)(-99999.0f, -99999.0f); }

    // Unified sphere -> ray angle: the sphere point (u*alpha, v*alpha, alpha - xi) is a unit vector
    float rho2 = dot(uv, uv);
    float disc = 1.0f + (1.0f - xi*xi) * rho2;
    if (disc < 0.0f) { return (float2)(-99999.0f, -99999.0f); }
    float alpha = (xi + sqrt(disc)) / (rho2 + 1.0f);
    float rho = sqrt(rho2);
    if (rho < 1e-12f) { return (float2)(0.0f, 0.0f); }
    return uv * (atan2(rho * alpha, alpha - xi) / rho);
}
