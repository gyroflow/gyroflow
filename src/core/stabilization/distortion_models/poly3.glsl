// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 undistort_point(vec2 pos, vec4 k, float amount) {
    float NEWTON_EPS = 0.00001;

    float inv_k1 = (1.0 / k.x);

    float rd = length(pos);
    if (rd == 0.0) { return vec2(0.0, 0.0); }

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
            return vec2(0.0, 0.0);
        }

        ru -= fru / (3.0 * ru * ru + inv_k1);
    }
    if (ru < 0.0) {
        return vec2(0.0, 0.0);
    }

    ru /= rd;

    // Apply only requested amount
    ru = 1.0 + (ru - 1.0) * (1.0 - amount);

    return pos * ru;
}

vec2 distort_point(vec2 pos, vec4 k) {
    float poly2 = k.x * (pos.x * pos.x + pos.y * pos.y) + 1.0;
    return pos * poly2;
}
