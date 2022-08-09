// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 undistort_point(vec2 pos, vec4 k, float amount) {
    float NEWTON_EPS = 0.00001;

    float rd = length(pos);
    if (rd == 0.0) { return vec2(0.0, 0.0); }

    float ru = rd;
    for (int i = 0; i < 10; ++i) {
        float ru2 = ru * ru;
        float fru = ru * (1.0 + k.x * ru2 + k.y * ru2 * ru2) - rd;
        if (fru >= -NEWTON_EPS && fru < NEWTON_EPS) {
            break;
        }
        if (i > 5) {
            // Does not converge, no real solution in this area?
            return vec2(0.0, 0.0);
        }

        ru -= fru / (1.0 + 3.0 * k.x * ru2 + 5.0 * k.y * ru2 * ru2);
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
    float ru2 = (pos.x * pos.x + pos.y * pos.y);
    float poly4 = 1.0 + k.x * ru2 + k.y * ru2 * ru2;
    return pos * poly4;
}
