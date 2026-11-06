// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 undistort_point(vec2 pos) {
    float NEWTON_EPS = 0.00001;

    float rd = length(pos);
    if (rd == 0.0) { return pos; }

    float ru = rd;
    for (int i = 0; i < 10; ++i) {
        float ru2 = ru * ru;
        float fru = ru * (1.0 + params.k1.x * ru2 + params.k1.y * ru2 * ru2) - rd;
        if (fru >= -NEWTON_EPS && fru < NEWTON_EPS) {
            break;
        }
        if (i > 5) {
            // Does not converge, no real solution in this area?
            return vec2(-99999.0, -99999.0);
        }

        ru -= fru / (1.0 + 3.0 * params.k1.x * ru2 + 5.0 * params.k1.y * ru2 * ru2);
    }
    if (ru < 0.0) {
        return vec2(-99999.0, -99999.0);
    }

    // `ru` is the radius in the z=1 plane; the pipeline carries rays as angle vectors
    return pos * (atan(ru) / rd);
}

vec2 distort_point(float x, float y, float z) {
    // A rectilinear model has no image of a ray at or past 90°: send it far outside the frame
    vec2 pos = z > 1e-9? vec2(x, y) / z : vec2(x, y) * 1e9;
    float ru2 = (pos.x * pos.x + pos.y * pos.y);
    float poly4 = 1.0 + params.k1.x * ru2 + params.k1.y * ru2 * ru2;
    return pos * poly4;
}
