// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

float2 undistort_point(float2 pos, __global KernelParams *params) {
    float NEWTON_EPS = 0.00001;

    float rd = length(pos);
    if (rd == 0.0) { return (float2)(0.0f, 0.0f); }

    float ru = rd;
    for (int i = 0; i < 10; ++i) {
        float fru = ru * (params->k[0] * ru * ru * ru + params->k[1] * ru * ru + params->k[2] * ru + 1.0) - rd;
        if (fru >= -NEWTON_EPS && fru < NEWTON_EPS) {
            break;
        }
        if (i > 5) {
            // Does not converge, no real solution in this area?
            return (float2)(0.0f, 0.0f);
        }

        ru -= fru / (4.0 * params->k[0] * ru * ru * ru + 3.0 * params->k[1] * ru * ru + 2.0 * params->k[2] * ru + 1.0);
    }
    if (ru < 0.0) {
        return (float2)(0.0f, 0.0f);
    }

    ru /= rd;

    return pos * ru;
}

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    float2 pos = (float2)(x, y) / z;
    float ru2 = (pos.x * pos.x + pos.y * pos.y);
    float r = sqrt(ru2);
    float poly3 = params->k[0] * ru2 * r + params->k[1] * ru2 + params->k[2] * r + 1.0;
    return pos * poly3;
}
