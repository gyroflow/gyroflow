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
    float r_norm = length(pos);
    // No POLY block: a pinhole, so the image radius is `tan θ`
    if (params->k[1] == 0.0f) return r_norm < 1e-12f? pos : pos * (atan(r_norm) / r_norm);
    if (r_norm < 1e-9f) return pos;
    float p = r_norm / params->k[1];
    float theta = gopro_poly_eval(p, params);
    // Outside its fit range the POLY is free to run negative or past 180°, and neither is an angle this
    // pipeline can carry - a negative theta hands back the ray from the opposite side of the frame, and
    // past 180 degrees sin(theta) has turned over and the ray arrives from behind the camera. See gopro.rs
    if (!(theta > 0.0f && theta < 3.14159265f)) { return (float2)(-99999.0f, -99999.0f); }
    return pos * (theta / r_norm);
}

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    // No POLY block: a pinhole, which has no image of a ray at or past 90°
    if (params->k[1] == 0.0f) return z > 1e-9f? (float2)(x, y) / z : (float2)(x, y) * 1e9f;
    float2 pos = (float2)(x, y);
    float r = length(pos);
    if (r < 1e-12f) return (float2)(0.0f, 0.0f);
    // atan2 against the ray's own z: a GoPro's 150°+ field needs the angle, not a z=1 plane radius
    float theta = atan2(r, z);
    float p = gopro_poly_invert(theta, params);
    return pos * (params->k[1] * p / r);
}
