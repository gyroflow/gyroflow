// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>
//
// GoPro native radial lens model. Raw POLY radial coeffs r0..r6 in params->k[0..7].
// The Superview/Hyperview MAPX/MAPY warp is a separate digital lens (gopro_warp).

float gopro_poly_eval(float p, __global KernelParams *params) {
    return params->k[0] + p * (params->k[1] + p * (params->k[2] + p * (params->k[3] + p * (params->k[4] + p * (params->k[5] + p * params->k[6])))));
}
float gopro_poly_deriv(float p, __global KernelParams *params) {
    return params->k[1] + p * (2.0f * params->k[2] + p * (3.0f * params->k[3] + p * (4.0f * params->k[4] + p * (5.0f * params->k[5] + p * (6.0f * params->k[6])))));
}
float gopro_poly_invert(float theta, __global KernelParams *params) {
    float p = (theta - params->k[0]) / params->k[1];
    for (int i = 0; i < 10; ++i) {
        float d = gopro_poly_deriv(p, params);
        if (fabs(d) < 1e-12f) break;
        float fix = (gopro_poly_eval(p, params) - theta) / d;
        p -= fix;
        if (fabs(fix) < 1e-7f) break;
    }
    return p;
}

float2 undistort_point(float2 pos, __global KernelParams *params) {
    if (params->k[1] == 0.0f) return pos;
    float r_norm = length(pos);
    if (r_norm < 1e-9f) return pos;
    float p = r_norm / params->k[1];
    float theta = gopro_poly_eval(p, params);
    // Clamp the angle just under tan()'s 90° asymptote and continue the radius linearly past it so over-FOV
    // rays stay large & monotonic (no wrap/fold -> r_limit clips them to background). See gopro.rs.
    const float TMAX = 1.5533f; const float tt = 57.14902f; // TMAX ≈ 89°, tt = tan(TMAX)
    float rr = theta < TMAX ? tan(theta) : tt + (theta - TMAX) * (1.0f + tt * tt);
    float scale = rr / r_norm;
    return pos * scale;
}

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    float2 pos = (float2)(x, y) / z;
    if (params->k[1] == 0.0f) return pos;
    float r = length(pos);
    // Inverse of undistort_point's angle clamp (see gopro.rs / gopro.cl undistort_point).
    const float TMAX = 1.5533f; const float tt = 57.14902f;
    float theta = r < tt ? atan(r) : TMAX + (r - tt) / (1.0f + tt * tt);
    float p = gopro_poly_invert(theta, params);
    float r_norm = params->k[1] * p;
    float scale = r == 0.0f ? 1.0f : r_norm / r;
    return pos * scale;
}
