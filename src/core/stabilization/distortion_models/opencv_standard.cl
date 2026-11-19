// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// The projection of a z=1 plane point, as distort_point has it
float2 opencv_standard_plane(float2 pos, __global KernelParams *params) {
    float x = pos.x, y = pos.y;
    float r2 = x * x + y * y;
    float rad = (1.0f + r2 * (params->k[0] + r2 * (params->k[1] + r2 * params->k[4]))) / (1.0f + r2 * (params->k[5] + r2 * (params->k[6] + r2 * params->k[7])));
    return (float2)(x * rad + params->k[2] * 2.0f * x * y + params->k[3] * (r2 + 2.0f * x * x) + params->k[8]  * r2 + params->k[9]  * r2 * r2,
                    y * rad + params->k[2] * (r2 + 2.0f * y * y) + params->k[3] * 2.0f * x * y + params->k[10] * r2 + params->k[11] * r2 * r2);
}
// Its Jacobian: (dx/du, dx/dv, dy/du, dy/dv)
float4 opencv_standard_jacobian(float2 pos, __global KernelParams *params) {
    float x = pos.x, y = pos.y;
    float r2 = x * x + y * y;
    float cd  = 1.0f + r2 * (params->k[0] + r2 * (params->k[1] + r2 * params->k[4]));
    float cd2 = 1.0f + r2 * (params->k[5] + r2 * (params->k[6] + r2 * params->k[7]));
    float rad = cd / cd2;
    // d(rad)/d(r2)
    float drad = (params->k[0] + r2 * (2.0f * params->k[1] + 3.0f * params->k[4] * r2) - rad * (params->k[5] + r2 * (2.0f * params->k[6] + 3.0f * params->k[7] * r2))) / cd2;
    // d(thin prism)/d(r2)
    float sx = params->k[8]  + 2.0f * params->k[9]  * r2;
    float sy = params->k[10] + 2.0f * params->k[11] * r2;
    float p1 = params->k[2], p2 = params->k[3];
    return (float4)(rad + 2.0f * x * x * drad + 2.0f * p1 * y + 6.0f * p2 * x + 2.0f * x * sx,
                    2.0f * x * y * drad       + 2.0f * p1 * x + 2.0f * p2 * y + 2.0f * y * sx,
                    2.0f * x * y * drad       + 2.0f * p1 * x + 2.0f * p2 * y + 2.0f * x * sy,
                    rad + 2.0f * y * y * drad + 6.0f * p1 * y + 2.0f * p2 * x + 2.0f * y * sy);
}

// Newton on the whole map from pos (see polish_inverse in distortion_models/mod.rs): the point, and how far its image
// still is from p - 1e30 where the map has turned over at the point, a ray of the branch the camera doesn't image with
float3 opencv_standard_polish(float2 pos, float2 p, __global KernelParams *params) {
    float2 e = opencv_standard_plane(pos, params) - p;
    float err = fmax(fabs(e.x), fabs(e.y));
    float tol = 5e-7f * (1.0f + length(p));
    for (int attempt = 0; attempt < 2; ++attempt) {
        for (int it = 0; it < 8; ++it) {
            if (!(err > tol)) break;
            float4 j = opencv_standard_jacobian(pos, params);
            float det = j.x * j.w - j.y * j.z;
            if (!(det > 0.0f)) break;
            float2 step = (float2)(j.w * e.x - j.y * e.y, j.x * e.y - j.z * e.x) / det;
            float lambda = 1.0f;
            bool moved = false;
            for (int b = 0; b < 4; ++b) {
                float2 c = pos - lambda * step;
                float2 ce = opencv_standard_plane(c, params) - p;
                float cerr = fmax(fabs(ce.x), fabs(ce.y));
                if (cerr < err) { pos = c; e = ce; err = cerr; moved = true; break; }
                lambda *= 0.5f;
            }
            if (!moved) break;
        }
        float4 jf = opencv_standard_jacobian(pos, params);
        if (jf.x * jf.w - jf.y * jf.z > 0.0f) { if (attempt == 1 && !(err <= 1e-6f * (1.0f + length(p)))) { err = 1e30f; } break; }
        // A point past a fold of the map has no ray: the camera images the other branch. Where that fold is the edge of
        // the image, the point it images can be just short of it: back towards the axis until the map keeps its orientation,
        // and Newton from there; only a point it then reaches counts (see polish_inverse in distortion_models/mod.rs)
        err = 1e30f;
        if (attempt == 1) { break; }
        float2 pos0 = pos;
        float s = 1e-3f;
        bool onto = false;
        for (int r = 0; r < 8; ++r) {
            float2 c = pos0 * (1.0f - s);
            float4 cj = opencv_standard_jacobian(c, params);
            if (cj.x * cj.w - cj.y * cj.z > 0.0f) { pos = c; onto = true; break; }
            s *= 2.0f;
        }
        if (!onto) { break; }
        e = opencv_standard_plane(pos, params) - p;
        err = fmax(fabs(e.x), fabs(e.y));
    }
    return (float3)(pos, err);
}
float opencv_standard_radial(float r, __global KernelParams *params) {
    float r2 = r * r;
    return r * (1.0f + r2 * (params->k[0] + r2 * (params->k[1] + r2 * params->k[4]))) / (1.0f + r2 * (params->k[5] + r2 * (params->k[6] + r2 * params->k[7])));
}
// The plane radius whose radial image is r_d: bracketed from the axis outwards and bisected (see opencv_standard.rs).
// -1 where the curve turns back, or runs out, before it reaches r_d
float opencv_standard_solve_radial(float r_d, __global KernelParams *params) {
    float lo = 0.0f;
    float hi = r_d;
    float g_hi = opencv_standard_radial(hi, params);
    for (int n = 0; n < 31; ++n) {
        if (g_hi >= r_d) break;
        float g_next = opencv_standard_radial(hi * 2.0f, params);
        if (!(g_next > g_hi) || n == 30) return -1.0f;
        lo = hi;
        hi *= 2.0f;
        g_hi = g_next;
    }
    for (int i = 0; i < 40; ++i) {
        float mid = 0.5f * (lo + hi);
        if (opencv_standard_radial(mid, params) < r_d) lo = mid; else hi = mid;
    }
    return 0.5f * (lo + hi);
}

float2 undistort_point(float2 pos, __global KernelParams *params) {
    float2 start_pos = pos;

    // compensate distortion iteratively
    for (int i = 0; i < 20; ++i) {
        float r2 = pos.x * pos.x + pos.y * pos.y;
        float icdist = (1.0 + ((params->k[7] * r2 + params->k[6]) * r2 + params->k[5]) * r2)/(1.0 + ((params->k[4] * r2 + params->k[1]) * r2 + params->k[0]) * r2);
        if (icdist < 0.0) {
            return (float2)(-99999.0f, -99999.0f);
        }
        float delta_x = 2.0 * params->k[2] * pos.x * pos.y + params->k[3] * (r2 + 2.0 * pos.x * pos.x) + params->k[8]  * r2 + params->k[9]  * r2 * r2;
        float delta_y = params->k[2] * (r2 + 2.0 * pos.y * pos.y) + 2.0 * params->k[3] * pos.x * pos.y + params->k[10] * r2 + params->k[11] * r2 * r2;
        pos = (float2)(
            (start_pos.x - delta_x) * icdist,
            (start_pos.y - delta_y) * icdist
        );
    }

    // That is a fixed point with the tangential and thin-prism terms held still, and it only contracts while they and
    // the slope of the radial curve are small: a wide lens with large ones is left hundreds of pixels off in the
    // corners. Newton on the whole map from there (see polish_inverse in distortion_models/mod.rs); where neither
    // finds the point, the lens has no ray for it
    if (!(isfinite(pos.x) && isfinite(pos.y))) { pos = start_pos; }
    float tol = 1e-4f * (1.0f + length(start_pos));
    float3 q = opencv_standard_polish(pos, start_pos, params);
    if (!(q.z <= tol)) {
        // A steep radial curve throws the fixed point between two far-apart values, and Newton from either is too slow
        // to arrive: start again from the radial curve's own inverse, which for radial terms only is the answer
        float r_d = length(start_pos);
        float r = opencv_standard_solve_radial(r_d, params);
        if (r >= 0.0f && r_d > 1e-12f) { q = opencv_standard_polish(start_pos * (r / r_d), start_pos, params); }
    }
    if (!(q.z <= tol)) { return (float2)(-99999.0f, -99999.0f); }
    pos = q.xy;

    // The solve runs in the z=1 plane; the pipeline carries rays as angle vectors
    float r = length(pos);
    if (r < 1e-12f) return pos;
    return pos * (atan(r) / r);
}

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    // A rectilinear model has no image of a ray at or past 90°: send it far outside the frame
    float2 pos = z > 1e-9f? (float2)(x, y) / z : (float2)(x, y) * 1e9f;
    float r2 = pos.x * pos.x + pos.y * pos.y;
    float r4 = r2 * r2;
    float r6 = r4 * r2;
    float a1 = 2.0 * pos.x * pos.y;
    float a2 = r2 + 2.0 * pos.x * pos.x;
    float a3 = r2 + 2.0 * pos.y * pos.y;
    float cdist = 1.0 + params->k[0] * r2 + params->k[1] * r4 + params->k[4] * r6;
    float icdist2 = 1.0 / (1.0 + params->k[5] * r2 + params->k[6] * r4 + params->k[7] * r6);

    return (float2)(
        pos.x * cdist * icdist2 + params->k[2] * a1 + params->k[3] * a2 + params->k[8]  * r2 + params->k[9]  * r4,
        pos.y * cdist * icdist2 + params->k[2] * a3 + params->k[3] * a1 + params->k[10] * r2 + params->k[11] * r4
    );
}
