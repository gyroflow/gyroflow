// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// The projection of a z=1 plane point, as distort_point has it
vec2 opencv_standard_plane(vec2 pos) {
    float x = pos.x, y = pos.y;
    float r2 = x * x + y * y;
    float rad = (1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * params.k2.x))) / (1.0 + r2 * (params.k2.y + r2 * (params.k2.z + r2 * params.k2.w)));
    return vec2(x * rad + params.k1.z * 2.0 * x * y + params.k1.w * (r2 + 2.0 * x * x) + params.k3.x * r2 + params.k3.y * r2 * r2,
                y * rad + params.k1.z * (r2 + 2.0 * y * y) + params.k1.w * 2.0 * x * y + params.k3.z * r2 + params.k3.w * r2 * r2);
}
// Its Jacobian: (dx/du, dx/dv, dy/du, dy/dv)
vec4 opencv_standard_jacobian(vec2 pos) {
    float x = pos.x, y = pos.y;
    float r2 = x * x + y * y;
    float cd  = 1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * params.k2.x));
    float cd2 = 1.0 + r2 * (params.k2.y + r2 * (params.k2.z + r2 * params.k2.w));
    float rad = cd / cd2;
    // d(rad)/d(r2)
    float drad = (params.k1.x + r2 * (2.0 * params.k1.y + 3.0 * params.k2.x * r2) - rad * (params.k2.y + r2 * (2.0 * params.k2.z + 3.0 * params.k2.w * r2))) / cd2;
    // d(thin prism)/d(r2)
    float sx = params.k3.x + 2.0 * params.k3.y * r2;
    float sy = params.k3.z + 2.0 * params.k3.w * r2;
    float p1 = params.k1.z, p2 = params.k1.w;
    return vec4(rad + 2.0 * x * x * drad + 2.0 * p1 * y + 6.0 * p2 * x + 2.0 * x * sx,
                2.0 * x * y * drad       + 2.0 * p1 * x + 2.0 * p2 * y + 2.0 * y * sx,
                2.0 * x * y * drad       + 2.0 * p1 * x + 2.0 * p2 * y + 2.0 * x * sy,
                rad + 2.0 * y * y * drad + 6.0 * p1 * y + 2.0 * p2 * x + 2.0 * y * sy);
}

// Newton on the whole map from pos (see polish_inverse in distortion_models/mod.rs): the point, and how far its image
// still is from p - 1e30 where the map has turned over at the point, a ray of the branch the camera doesn't image with
vec3 opencv_standard_polish(vec2 pos, vec2 p) {
    vec2 e = opencv_standard_plane(pos) - p;
    float err = max(abs(e.x), abs(e.y));
    float tol = 5e-7 * (1.0 + length(p));
    for (int attempt = 0; attempt < 2; ++attempt) {
        for (int it = 0; it < 8; ++it) {
            if (!(err > tol)) break;
            vec4 j = opencv_standard_jacobian(pos);
            float det = j.x * j.w - j.y * j.z;
            if (!(det > 0.0)) break;
            vec2 step = vec2(j.w * e.x - j.y * e.y, j.x * e.y - j.z * e.x) / det;
            float lambda = 1.0;
            bool moved = false;
            for (int b = 0; b < 4; ++b) {
                vec2 c = pos - lambda * step;
                vec2 ce = opencv_standard_plane(c) - p;
                float cerr = max(abs(ce.x), abs(ce.y));
                if (cerr < err) { pos = c; e = ce; err = cerr; moved = true; break; }
                lambda *= 0.5;
            }
            if (!moved) break;
        }
        vec4 jf = opencv_standard_jacobian(pos);
        if (jf.x * jf.w - jf.y * jf.z > 0.0) { if (attempt == 1 && !(err <= 1e-6 * (1.0 + length(p)))) { err = 1e30; } break; }
        // A point past a fold of the map has no ray: the camera images the other branch. Where that fold is the edge of
        // the image, the point it images can be just short of it: back towards the axis until the map keeps its orientation,
        // and Newton from there; only a point it then reaches counts (see polish_inverse in distortion_models/mod.rs)
        err = 1e30;
        if (attempt == 1) { break; }
        vec2 pos0 = pos;
        float s = 1e-3;
        bool onto = false;
        for (int r = 0; r < 8; ++r) {
            vec2 c = pos0 * (1.0 - s);
            vec4 cj = opencv_standard_jacobian(c);
            if (cj.x * cj.w - cj.y * cj.z > 0.0) { pos = c; onto = true; break; }
            s *= 2.0;
        }
        if (!onto) { break; }
        e = opencv_standard_plane(pos) - p;
        err = max(abs(e.x), abs(e.y));
    }
    return vec3(pos, err);
}
float opencv_standard_radial(float r) {
    float r2 = r * r;
    return r * (1.0 + r2 * (params.k1.x + r2 * (params.k1.y + r2 * params.k2.x))) / (1.0 + r2 * (params.k2.y + r2 * (params.k2.z + r2 * params.k2.w)));
}
// The plane radius whose radial image is r_d: bracketed from the axis outwards and bisected (see opencv_standard.rs).
// -1 where the curve turns back, or runs out, before it reaches r_d
float opencv_standard_solve_radial(float r_d) {
    float lo = 0.0;
    float hi = r_d;
    float g_hi = opencv_standard_radial(hi);
    for (int n = 0; n < 31; ++n) {
        if (g_hi >= r_d) break;
        float g_next = opencv_standard_radial(hi * 2.0);
        if (!(g_next > g_hi) || n == 30) return -1.0;
        lo = hi;
        hi *= 2.0;
        g_hi = g_next;
    }
    for (int i = 0; i < 40; ++i) {
        float mid = 0.5 * (lo + hi);
        if (opencv_standard_radial(mid) < r_d) lo = mid; else hi = mid;
    }
    return 0.5 * (lo + hi);
}

vec2 undistort_point(vec2 pos) {
    vec2 start_pos = pos;

    // compensate distortion iteratively
    for (int i = 0; i < 20; ++i) {
        float r2 = pos.x * pos.x + pos.y * pos.y;
        float r4 = r2 * r2;
        float r6 = r4 * r2;
        float a1 = 2.0 * pos.x * pos.y;
        float a2 = r2 + 2.0 * pos.x * pos.x;
        float a3 = r2 + 2.0 * pos.y * pos.y;
        float cdist = 1.0 + params.k1.x * r2 + params.k1.y * r4 + params.k2.x * r6;
        float icdist = (1.0 + params.k2.y * r2 + params.k2.z * r4 + params.k2.w * r6) / cdist;
        if (icdist < 0.0) {
            return vec2(-99999.0, -99999.0);
        }
        float delta_x = params.k1.z * a1 + params.k1.w * a2 + params.k3.x * r2 + params.k3.y * r4;
        float delta_y = params.k1.z * a3 + params.k1.w * a1 + params.k3.z * r2 + params.k3.w * r4;
        pos = vec2(
            (start_pos.x - delta_x) * icdist,
            (start_pos.y - delta_y) * icdist
        );
    }

    // That is a fixed point with the tangential and thin-prism terms held still, and it only contracts while they and
    // the slope of the radial curve are small: a wide lens with large ones is left hundreds of pixels off in the
    // corners. Newton on the whole map from there (see polish_inverse in distortion_models/mod.rs); where neither
    // finds the point, the lens has no ray for it
    if (!(abs(pos.x) < 1e30 && abs(pos.y) < 1e30)) { pos = start_pos; } // not finite
    float tol = 1e-4 * (1.0 + length(start_pos));
    vec3 q = opencv_standard_polish(pos, start_pos);
    if (!(q.z <= tol)) {
        // A steep radial curve throws the fixed point between two far-apart values, and Newton from either is too slow
        // to arrive: start again from the radial curve's own inverse, which for radial terms only is the answer
        float r_d = length(start_pos);
        float r = opencv_standard_solve_radial(r_d);
        if (r >= 0.0 && r_d > 1e-12) { q = opencv_standard_polish(start_pos * (r / r_d), start_pos); }
    }
    if (!(q.z <= tol)) { return vec2(-99999.0, -99999.0); }
    pos = q.xy;

    // The solve runs in the z=1 plane; the pipeline carries rays as angle vectors
    float r = length(pos);
    if (r < 1e-12) return pos;
    return pos * (atan(r) / r);
}

vec2 distort_point(float x, float y, float z) {
    // A rectilinear model has no image of a ray at or past 90°: send it far outside the frame
    vec2 pos = z > 1e-9? vec2(x, y) / z : vec2(x, y) * 1e9;
    float r2 = pos.x * pos.x + pos.y * pos.y;
    float r4 = r2 * r2;
    float r6 = r4 * r2;
    float a1 = 2.0 * pos.x * pos.y;
    float a2 = r2 + 2.0 * pos.x * pos.x;
    float a3 = r2 + 2.0 * pos.y * pos.y;
    float cdist = 1.0 + params.k1.x * r2 + params.k1.y * r4 + params.k2.x * r6;
    float icdist2 = 1.0 / (1.0 + params.k2.y * r2 + params.k2.z * r4 + params.k2.w * r6);

    return vec2(
        pos.x * cdist * icdist2 + params.k1.z * a1 + params.k1.w * a2 + params.k3.x * r2 + params.k3.y * r4,
        pos.y * cdist * icdist2 + params.k1.z * a3 + params.k1.w * a1 + params.k3.z * r2 + params.k3.w * r4
    );
}
