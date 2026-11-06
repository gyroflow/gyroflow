// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

float2 undistort_point(float2 pos, __global KernelParams *params) {
    float NEWTON_EPS = 0.00001;

    float inv_k1 = (1.0 / params->k[0]);

    float rd = length(pos);
    if (rd == 0.0) { return pos; }

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
            return (float2)(-99999.0f, -99999.0f);
        }

        ru -= fru / (3.0 * ru * ru + inv_k1);
    }
    if (ru < 0.0) {
        return (float2)(-99999.0f, -99999.0f);
    }

    // `ru` is the radius in the z=1 plane; the pipeline carries rays as angle vectors
    return pos * (atan(ru) / rd);
}

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    // A rectilinear model has no image of a ray at or past 90°: send it far outside the frame
    float2 pos = z > 1e-9f? (float2)(x, y) / z : (float2)(x, y) * 1e9f;
    float poly2 = params->k[0] * (pos.x * pos.x + pos.y * pos.y) + 1.0;
    return pos * poly2;
}
