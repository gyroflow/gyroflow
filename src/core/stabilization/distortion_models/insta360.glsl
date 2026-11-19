// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Radial part of the projection: image radius of a unified-sphere plane radius `m`
float radial_projection(float m, float k1, float k2, float k3) {
    float m2 = m * m;
    return m * (1.0 + k1*m2 + k2*m2*m2 + k3*m2*m2*m2);
}
// d(radial_projection)/dm
float radial_derivative(float m, float k1, float k2, float k3) {
    float m2 = m * m;
    return 1.0 + 3.0*k1*m2 + 5.0*k2*m2*m2 + 7.0*k3*m2*m2*m2;
}
// How far the sphere reaches: it folds at theta = acos(-1/xi) (120 degrees for the X5's xi = 2), and for
// xi <= 1 it never folds but runs to infinity, which the placeholder stands for. Never a bound on the image
// radius on its own - the radial polynomial folds where radial_derivative first reaches zero, which can come
// first. See insta360.rs
float insta360_m_max(float xi) {
    return xi > 1.0? 1.0 / sqrt(xi * xi - 1.0) : 1e4;
}
// Newton on `radial_projection(m) = r`, kept on the branch the camera projects with: a step that lands where
// the curve has stopped rising has stepped over its fold, and pulling the bracket in to there walks up to the
// fold instead of across it. Returns the end of the branch when `r` is past everything the lens images
float solve_radial(float r, float guess, float m_max, float k1, float k2, float k3) {
    float lo = 0.0, hi = m_max;
    float m = clamp(guess, 0.0, m_max);
    for (int i = 0; i < 20; ++i) { // Newton is done in a few; the rest are the fold's bisection
        float d = radial_derivative(m, k1, k2, k3);
        if (d <= 1e-9) {
            hi = m; // folded at or before m; the root, if there is one, is below it
            m = 0.5 * (lo + hi);
        } else {
            float f = radial_projection(m, k1, k2, k3) - r;
            // Solved, judged on the residual and not on the size of the step: a step below an ulp of m is
            // one a GPU computes exactly and then rounds away, and a test on it never passes. See insta360.rs
            if (abs(f) <= 2e-7 * (1.0 + r)) { break; }
            if (f < 0.0) { lo = m; } else { hi = m; }
            float next = m - f / d;
            // A step landing on an end of the bracket is a step of nothing - kept, not read as outside the
            // bracket and replaced by the midpoint of a bracket whose far end may still be m_max
            m = (next >= lo && next <= hi)? next : 0.5 * (lo + hi); // outside (or NaN): bisect
        }
    }
    return m;
}

// The projection of a unified-sphere plane point, as distort_point has it
vec2 insta360_plane(vec2 uv) {
    float k1 = params.k1.x, k2 = params.k1.y, k3 = params.k1.z, p1 = params.k1.w, p2 = params.k2.x;
    float r2 = dot(uv, uv);
    float rad = 1.0 + r2 * (k1 + r2 * (k2 + r2 * k3));
    return vec2(uv.x * rad + 2.0*p1*uv.x*uv.y + p2*(r2 + 2.0*uv.x*uv.x),
                uv.y * rad + 2.0*p2*uv.x*uv.y + p1*(r2 + 2.0*uv.y*uv.y));
}
// Its Jacobian: (dx/du, dx/dv, dy/du, dy/dv)
vec4 insta360_plane_jacobian(vec2 uv) {
    float k1 = params.k1.x, k2 = params.k1.y, k3 = params.k1.z, p1 = params.k1.w, p2 = params.k2.x;
    float u = uv.x, v = uv.y;
    float r2 = u*u + v*v;
    float rad = 1.0 + r2 * (k1 + r2 * (k2 + r2 * k3));
    float drad = k1 + r2 * (2.0*k2 + 3.0*k3*r2); // d(rad)/d(r2)
    return vec4(rad + 2.0*u*u*drad + 2.0*p1*v + 6.0*p2*u,
                2.0*u*v*drad       + 2.0*p1*u + 2.0*p2*v,
                2.0*u*v*drad       + 2.0*p2*v + 2.0*p1*u,
                rad + 2.0*v*v*drad + 2.0*p2*u + 6.0*p1*v);
}

vec2 distort_point(float x, float y, float z) {
    float k1 = params.k1.x;
    float k2 = params.k1.y;
    float k3 = params.k1.z;
    float p1 = params.k1.w;

    float p2 = params.k2.x;
    float xi = params.k2.y;

    vec3 P = vec3(x, y, z);
    P /= length(P);

    x = P.x / (P.z + xi);
    y = P.y / (P.z + xi);

    float r2 = x*x + y*y;
    float r4 = r2 * r2;
    float r6 = r4 * r2;

    return vec2(
        x * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p1*x*y + p2*(r2 + 2.0*x*x),
        y * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p2*x*y + p1*(r2 + 2.0*y*y)
    );
}

vec2 undistort_point(vec2 p) {
    float k1 = params.k1.x;
    float k2 = params.k1.y;
    float k3 = params.k1.z;
    float p1 = params.k1.w;
    float p2 = params.k2.x;
    float xi = params.k2.y;

    float r_d = length(p);
    if (r_d < 1e-9) { return p; }

    // Past the end of the projection's rising branch there is no ray at all - but only where the sphere's
    // own end is that end; where the polynomial folds first, radial_projection(m_max) is off the far side of
    // the fold and says nothing about the largest radius the lens images
    float m_max = insta360_m_max(xi);
    // And only past what the terms that aren't radial add to it: they carry the image of the sphere's rim out of that
    // circle in some directions, and the 2D solve below decides there (see insta360_v6.rs / insta360.rs)
    bool sphere_ends = radial_derivative(m_max, k1, k2, k3) > 0.0;
    if (sphere_ends && radial_projection(m_max, k1, k2, k3) + 4.0 * m_max * m_max * (abs(p1) + abs(p2)) < r_d) { return vec2(-99999.0, -99999.0); }
    // Between the two only a point the solve reaches is one: what it stalls at short of a point out there is the edge of
    // the image, and the point past it
    bool past_the_rim = sphere_ends && radial_projection(m_max, k1, k2, k3) < r_d;

    // Strip the tangential terms, invert the radial polynomial, re-evaluate them
    vec2 a = p;
    vec2 uv = vec2(0.0, 0.0);
    float m = r_d;
    float r_s = r_d;
    for (int i = 0; i < 3; ++i) {
        r_s = length(a);
        m = solve_radial(r_s, m, m_max, k1, k2, k3);
        uv = r_s > 1e-12? a * (m / r_s) : vec2(0.0, 0.0);
        float r2 = dot(uv, uv);
        a = p - vec2(2.0*p1*uv.x*uv.y + p2*(r2 + 2.0*uv.x*uv.x),
                     2.0*p2*uv.x*uv.y + p1*(r2 + 2.0*uv.y*uv.y));
    }
    // Large tangential terms leave that pixels off: Newton on the whole map finishes from there, inside the sphere's
    // fold and never from past a fold of the map (see polish_inverse in distortion_models/mod.rs)
    vec2 e = insta360_plane(uv) - p;
    float err = max(abs(e.x), abs(e.y));
    float tol = 5e-7 * (1.0 + r_d);
    bool exact_only = past_the_rim;
    for (int attempt = 0; attempt < 2; ++attempt) {
        for (int it = 0; it < 8; ++it) {
            if (!(err > tol)) break;
            vec4 j = insta360_plane_jacobian(uv);
            float det = j.x * j.w - j.y * j.z;
            if (!(det > 0.0)) break;
            vec2 step = vec2(j.w * e.x - j.y * e.y, j.x * e.y - j.z * e.x) / det;
            float lambda = 1.0;
            bool moved = false;
            for (int b = 0; b < 4; ++b) {
                vec2 c = uv - lambda * step;
                if (dot(c, c) <= m_max * m_max) {
                    vec2 ce = insta360_plane(c) - p;
                    float cerr = max(abs(ce.x), abs(ce.y));
                    if (cerr < err) { uv = c; e = ce; err = cerr; moved = true; break; }
                }
                lambda *= 0.5;
            }
            if (!moved) break;
        }
        vec4 jf = insta360_plane_jacobian(uv);
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
            vec4 cj = insta360_plane_jacobian(c);
            if (cj.x * cj.w - cj.y * cj.z > 0.0) { uv = c; onto = true; break; }
            s *= 2.0;
        }
        if (!onto) { break; }
        e = insta360_plane(uv) - p;
        err = max(abs(e.x), abs(e.y));
        exact_only = true;
    }
    // How far the map lands from the point is the test of whether there was a ray here at all
    float accept_err = 1e-4 * (1.0 + r_d);
    if (exact_only) { accept_err = 1e-6 * (1.0 + r_d); }
    if (!(err <= accept_err)) { return vec2(-99999.0, -99999.0); }

    // Unified sphere -> ray angle. The sphere point is (u*alpha, v*alpha, alpha - xi), already a unit
    // vector, so the angle falls out of atan2 - where the old x/z went to infinity at 90 degrees
    float rho2 = dot(uv, uv);
    float disc = 1.0 + (1.0 - xi*xi) * rho2;
    if (disc < 0.0) { return vec2(-99999.0, -99999.0); }
    float alpha = (xi + sqrt(disc)) / (rho2 + 1.0);
    float rho = sqrt(rho2);
    if (rho < 1e-12) { return vec2(0.0, 0.0); }
    return uv * (atan(rho * alpha, alpha - xi) / rho);
}

/*vec2 undistort_point(vec2 p) {
    float xi = params.k2.y;

    vec2 P = p;

    for (int i = 0; i < 200; i++) {
        P -= distort_point(P.x, P.y, 1.0) - p;
    }

    vec2 pud = P;

    float r = pud.x * pud.x + pud.y * pud.y;
    float z = 1.0 - ((xi * (r + 1.0)) / (xi + sqrt(1.0 + r * (1.0 - xi*xi))));

    return pud * z;
}
vec2 project_to_2d(float x, float y, float z) {
    float r = length(vec3(x, y, z));
    float theta = atan2(y, x);
    // float phi = atan2(z, r);

    return vec2(
        r * cos(theta),
        r * sin(theta)
    );
}
*/
