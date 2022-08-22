// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

float2 undistort_point(float2 pos, float k[12], float amount) {
    float NEWTON_EPS = 0.00001;

    float rd = length(pos);
    if (rd == 0.0) { return (float2)(0.0f, 0.0f); }

    float ru = rd;
    for (int i = 0; i < 10; ++i) {
        float ru2 = ru * ru;
        float fru = ru * (1.0 + k[0] * ru2 + k[1] * ru2 * ru2) - rd;
        if (fru >= -NEWTON_EPS && fru < NEWTON_EPS) {
            break;
        }
        if (i > 5) {
            // Does not converge, no real solution in this area?
            return (float2)(0.0f, 0.0f);
        }

        ru -= fru / (1.0 + 3.0 * k[0] * ru2 + 5.0 * k[1] * ru2 * ru2);
    }
    if (ru < 0.0) {
        return (float2)(0.0f, 0.0f);
    }

    ru /= rd;

    // Apply only requested amount
    ru = 1.0 + (ru - 1.0) * (1.0 - amount);

    return pos * ru;
}

float2 distort_point(float2 pos, float k[12]) {
    float ru2 = (pos.x * pos.x + pos.y * pos.y);
    float poly4 = 1.0 + k[0] * ru2 + k[1] * ru2 * ru2;
    return pos * poly4;
}
