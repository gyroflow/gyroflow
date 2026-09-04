// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)
//
// Sony native lens model: the camera's lens curve (ray angle at equally spaced image radii) evaluated as a natural
// cubic spline radius -> angle. Layout of params->k: see sony.rs.

int sony_segments(__global KernelParams *params) {
    float n = params->k[0];
    if (n >= 1.0f && n <= 10.0f && params->k[1] > 0.0f && params->k[2] == 0.0f) return (int)n;
    return 0;
}
// (y_i, b_i, c_i, d_i) of spline segment i
float4 sony_segment(int i, __global KernelParams *params) {
    float h = params->k[1];
    float y0 = params->k[2 + i], y1 = params->k[3 + i];
    float c0 = i == 0 ? 0.0f : params->k[13 + i], c1 = params->k[14 + i]; // k[13] holds r_lin, c_0 is 0
    return (float4)(y0, (y1 - y0) / h - h * (c1 + 2.0f * c0) / 3.0f, c0, (c1 - c0) / (3.0f * h));
}
#define SONY_THETA0 0.052359879f // 3 deg, end of the linear region near the optical axis
// Ray angle at normalized image radius r (linear continuation with the end slope outside the knots)
float sony_angle_at(int n, float r, __global KernelParams *params) {
    float h = params->k[1];
    float r_lin = params->k[13];
    if (r_lin > 0.0f && r < r_lin) return SONY_THETA0 * r / r_lin;
    int i = min(max((int)(r / h), 0), n - 1);
    float4 s = sony_segment(i, params);
    float dx = clamp(r - (float)i * h, 0.0f, h);
    float slope = s.y + (2.0f * s.z + 3.0f * s.w * dx) * dx;
    return s.x + (s.y + (s.z + s.w * dx) * dx) * dx + slope * (r - (float)i * h - dx);
}
// Normalized image radius for ray angle theta (inverse of sony_angle_at)
float sony_radius_at(int n, float theta, __global KernelParams *params) {
    float h = params->k[1];
    if (theta <= 0.0f) return 0.0f;
    float r_lin = params->k[13];
    if (r_lin > 0.0f && theta < SONY_THETA0) return theta * r_lin / SONY_THETA0;
    int i = 0;
    while (i + 1 < n && theta >= params->k[3 + i]) ++i;
    float4 s = sony_segment(i, params);
    float y1 = params->k[3 + i];
    if (theta >= y1) { // past the last knot: linear continuation
        float slope = s.y + (2.0f * s.z + 3.0f * s.w * h) * h;
        return (float)n * h + (slope > 1e-9f ? (theta - y1) / slope : 0.0f);
    }
    // Newton on the segment's cubic, starting from the chord
    float dx = h * (theta - s.x) / max(y1 - s.x, 1e-12f);
    for (int it = 0; it < 8; ++it) {
        float f = s.x + (s.y + (s.z + s.w * dx) * dx) * dx - theta;
        float fp = s.y + (2.0f * s.z + 3.0f * s.w * dx) * dx;
        if (fabs(fp) < 1e-12f) break;
        float fix = f / fp;
        dx = clamp(dx - fix, 0.0f, h);
        if (fabs(fix) < 1e-7f) break;
    }
    return (float)i * h + dx;
}

float2 undistort_point(float2 pos, __global KernelParams *params) {
    int n = sony_segments(params);
    if (n == 0) return pos;
    float r = length(pos);
    if (r < 1e-9f) return pos;
    float theta = sony_angle_at(n, r, params);
    if (theta <= 0.0f) return (float2)(-99999.0f, -99999.0f);
    // Clamp the angle just under tan()'s 90° asymptote and continue the radius linearly past it, so over-FOV rays
    // stay large and monotonic (no fold back into the frame) and r_limit clips them. See sony.rs / gopro.rs.
    const float TMAX = 1.5533f, tt = 57.14902f; // TMAX ≈ 89°, tt = tan(TMAX)
    float rr = theta < TMAX ? tan(theta) : tt + (theta - TMAX) * (1.0f + tt * tt);
    if (params->r_limit > 0.0f && rr > params->r_limit) return (float2)(-99999.0f, -99999.0f);
    return pos * (rr / r);
}

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    float2 pos = (float2)(x, y) / z;
    int n = sony_segments(params);
    if (n == 0) return pos;
    float r = length(pos);
    if (r < 1e-9f) return pos;
    // Inverse of undistort_point's angle clamp
    const float TMAX = 1.5533f, tt = 57.14902f;
    float theta = r < tt ? atan(r) : TMAX + (r - tt) / (1.0f + tt * tt);
    return pos * (sony_radius_at(n, theta, params) / r);
}
