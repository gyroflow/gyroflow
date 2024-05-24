// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    float2 pos = (float2)(x, y) / z;
    if (params->k[0] == 0.0 && params->k[1] == 0.0 && params->k[2] == 0.0 && params->k[3] == 0.0) return pos;

    float r = length(pos);
    float theta = atan(r);

    float theta2 = theta*theta,
          theta3 = theta2*theta,
          theta4 = theta2*theta2,
          theta5 = theta2*theta3,
          theta6 = theta3*theta3;

    float theta_d =  theta * params->k[0] + theta2 * params->k[1] + theta3 * params->k[2] + theta4 * params->k[3] + theta5 * params->k[4] + theta6 * params->k[5];

    float scale = r == 0.0f? 1.0f : theta_d / r;

    float2 post_scale = (float2)(params->k[6], params->k[7]);

    return pos * scale * post_scale;
}

float2 undistort_point(float2 p, __global KernelParams *params) {
    float2 P = p;
    if (params->k[0] == 0.0 && params->k[1] == 0.0 && params->k[2] == 0.0 && params->k[3] == 0.0) return p;

    for (int i = 0; i < 20; ++i) {
        float2 diff = distort_point(P.x, P.y, 1.0, params) - p;
        if (fabs(diff.x) < 1e-6f && fabs(diff.y) < 1e-6f) {
            break;
        }
        P -= diff;
    }

    return P;
}
