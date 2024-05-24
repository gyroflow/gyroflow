// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    float k1 = params->k[0];
    float k2 = params->k[1];
    float k3 = params->k[2];
    float p1 = params->k[3];
    float p2 = params->k[4];
    float xi = params->k[5];

    float3 P = (float3)(x, y, z);
    P /= length(P);

    x = P.x / (P.z + xi);
    y = P.y / (P.z + xi);

    float r2 = x*x + y*y;
    float r4 = r2 * r2;
    float r6 = r4 * r2;

    return (float2)(
        x * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p1*x*y + p2*(r2 + 2.0*x*x),
        y * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p2*x*y + p1*(r2 + 2.0*y*y)
    );
}

float2 undistort_point(float2 p, __global KernelParams *params) {
    float2 P = p;

    for (int i = 0; i < 200; ++i) {
        float2 diff = distort_point(P.x, P.y, 1.0, params) - p;
        if (fabs(diff.x) < 1e-6f && fabs(diff.y) < 1e-6f) {
            break;
        }
        P -= diff;
    }

    return P;
}
