// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

float2 undistort_point(float2 pos, float4 kk, float amount) {
    float k[12] = { kk.x, kk.y, kk.z, kk.w, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0 }; // TODO: allow more coefficients than 4?
    float2 start_pos = pos;

    // compensate distortion iteratively
    for (int i = 0; i < 20; ++i) {
        float r2 = pos.x * pos.x + pos.y * pos.y;
        float icdist = (1.0 + ((k[7] * r2 + k[6]) * r2 + k[5]) * r2)/(1.0 + ((k[4] * r2 + k[1]) * r2 + k[0]) * r2);
        if (icdist < 0.0) {
            return (float2)(0.0f, 0.0f);
        }
        float delta_x = 2.0 * k[2] * pos.x * pos.y + k[3] * (r2 + 2.0 * pos.x * pos.x)+ k[8] * r2 + k[9] * r2 * r2;
        float delta_y = k[2] * (r2 + 2.0 * pos.y * pos.y) + 2.0 * k[3] * pos.x * pos.y+ k[10] * r2 + k[11] * r2 * r2;
        pos = (float2)(
            (start_pos.x - delta_x) * icdist,
            (start_pos.y - delta_y) * icdist
        );
    }

    return (float2)(
        pos.x * (amount - 1.0) + start_pos.x * amount,
        pos.y * (amount - 1.0) + start_pos.y * amount
    );
}

float2 distort_point(float2 pos, float4 kk) {
    float k[12] = { kk.x, kk.y, kk.z, kk.w, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0 }; // TODO: allow more coefficients than 4?

    float r2 = pos.x * pos.x + pos.y * pos.y;
    float r4 = r2 * r2;
    float r6 = r4 * r2;
    float a1 = 2.0 * pos.x * pos.y;
    float a2 = r2 + 2.0 * pos.x * pos.x;
    float a3 = r2 + 2.0 * pos.y * pos.y;
    float cdist = 1.0 + k[0] * r2 + k[1] * r4 + k[4] * r6;
    float icdist2 = 1.0 / (1.0 + k[5] * r2 + k[6] * r4 + k[7] * r6);

    return (float2)(
        pos.x * cdist * icdist2 + k[2] * a1 + k[3] * a2 + k[8]  * r2 + k[9]  * r4,
        pos.y * cdist * icdist2 + k[2] * a3 + k[3] * a1 + k[10] * r2 + k[11] * r4
    );
}
