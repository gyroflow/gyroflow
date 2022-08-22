// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 undistort_point(vec2 pos, vec4 k1, vec4 k2, vec4 k3, float amount) {
    float NEWTON_EPS = 0.00001;

    float rd = length(pos);
    if (rd == 0.0) { return vec2(0.0, 0.0); }

    float ru = rd;
    for (int i = 0; i < 10; ++i) {
        float fru = ru * (k1.x * ru * ru * ru + k1.y * ru * ru + k1.z * ru + 1.0) - rd;
        if (fru >= -NEWTON_EPS && fru < NEWTON_EPS) {
            break;
        }
        if (i > 5) {
            // Does not converge, no real solution in this area?
            return vec2(0.0, 0.0);
        }

        ru -= fru / (4.0 * k1.x * ru * ru * ru + 3.0 * k1.y * ru * ru + 2.0 * k1.z * ru + 1.0);
    }
    if (ru < 0.0) {
        return vec2(0.0, 0.0);
    }

    ru /= rd;

    // Apply only requested amount
    ru = 1.0 + (ru - 1.0) * (1.0 - amount);

    return pos * ru;
}

vec2 distort_point(vec2 pos, vec4 k1, vec4 k2, vec4 k3) {
    float ru2 = (pos.x * pos.x + pos.y * pos.y);
    float r = sqrt(ru2);
    float poly3 = k1.x * ru2 * r + k1.y * ru2 + k1.z * r + 1.0;
    return pos * poly3;
}
