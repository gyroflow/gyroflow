// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

float2 undistort_point(float2 pos, __global KernelParams *params) {
    float2 start_pos = pos;

    // compensate distortion iteratively
    for (int i = 0; i < 20; ++i) {
        float r2 = pos.x * pos.x + pos.y * pos.y;
        float icdist = (1.0 + ((params->k[7] * r2 + params->k[6]) * r2 + params->k[5]) * r2)/(1.0 + ((params->k[4] * r2 + params->k[1]) * r2 + params->k[0]) * r2);
        if (icdist < 0.0) {
            return (float2)(0.0f, 0.0f);
        }
        float delta_x = 2.0 * params->k[2] * pos.x * pos.y + params->k[3] * (r2 + 2.0 * pos.x * pos.x) + params->k[8]  * r2 + params->k[9]  * r2 * r2;
        float delta_y = params->k[2] * (r2 + 2.0 * pos.y * pos.y) + 2.0 * params->k[3] * pos.x * pos.y + params->k[10] * r2 + params->k[11] * r2 * r2;
        pos = (float2)(
            (start_pos.x - delta_x) * icdist,
            (start_pos.y - delta_y) * icdist
        );
    }

    return pos;
}

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    float2 pos = (float2)(x, y) / z;
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
