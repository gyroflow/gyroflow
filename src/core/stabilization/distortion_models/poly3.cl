// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

float2 undistort_point(float2 pos, __global KernelParams *params) {
    float NEWTON_EPS = 0.00001;

    float inv_k1 = (1.0 / params->k[0]);

    float rd = length(pos);
    if (rd == 0.0) { return (float2)(0.0f, 0.0f); }

    float rd_div_k1 = rd * inv_k1;

    // Use Newton's method to avoid dealing with complex numbers.
    // When carefully tuned this works almost as fast as Cardano's method (and we don't use complex numbers in it, which is required for a full solution!)
    //
    // Original function: Rd = k1_ * Ru^3 + Ru
    // Target function:   k1_ * Ru^3 + Ru - Rd = 0
    // Divide by k1_:     Ru^3 + Ru/k1_ - Rd/k1_ = 0
    // Derivative:        3 * Ru^2 + 1/k1_
    float ru = rd;
    for (int i = 0; i < 10; ++i) {
        float fru = ru * ru * ru + ru * inv_k1 - rd_div_k1;
        if (fru >= -NEWTON_EPS && fru < NEWTON_EPS) {
            break;
        }
        if (i > 5) {
            // Does not converge, no real solution in this area?
            return (float2)(0.0f, 0.0f);
        }

        ru -= fru / (3.0 * ru * ru + inv_k1);
    }
    if (ru < 0.0) {
        return (float2)(0.0f, 0.0f);
    }

    ru /= rd;

    return pos * ru;
}

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    float2 pos = (float2)(x, y) / z;
    float poly2 = params->k[0] * (pos.x * pos.x + pos.y * pos.y) + 1.0;
    return pos * poly2;
}
