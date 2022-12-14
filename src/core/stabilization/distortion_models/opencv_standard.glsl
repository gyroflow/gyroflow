// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 undistort_point(vec2 pos) {
    vec2 start_pos = pos;

    // compensate distortion iteratively
    for (int i = 0; i < 20; ++i) {
        float r2 = pos.x * pos.x + pos.y * pos.y;
        float r4 = r2 * r2;
        float r6 = r4 * r2;
        float a1 = 2.0 * pos.x * pos.y;
        float a2 = r2 + 2.0 * pos.x * pos.x;
        float a3 = r2 + 2.0 * pos.y * pos.y;
        float cdist = 1.0 + params.k1.x * r2 + params.k1.y * r4 + params.k2.x * r6;
        float icdist = (1.0 + params.k2.y * r2 + params.k2.z * r4 + params.k2.w * r6) / cdist;
        if (icdist < 0.0) {
            return vec2(0.0, 0.0);
        }
        float delta_x = params.k1.z * a1 + params.k1.w * a2 + params.k3.x * r2 + params.k3.y * r4;
        float delta_y = params.k1.z * a3 + params.k1.w * a1 + params.k3.z * r2 + params.k3.w * r4;
        pos = vec2(
            (start_pos.x - delta_x) * icdist,
            (start_pos.y - delta_y) * icdist
        );
    }

    return pos;
}

vec2 distort_point(float x, float y, float z) {
    vec2 pos = vec2(x, y) / z;
    float r2 = pos.x * pos.x + pos.y * pos.y;
    float r4 = r2 * r2;
    float r6 = r4 * r2;
    float a1 = 2.0 * pos.x * pos.y;
    float a2 = r2 + 2.0 * pos.x * pos.x;
    float a3 = r2 + 2.0 * pos.y * pos.y;
    float cdist = 1.0 + params.k1.x * r2 + params.k1.y * r4 + params.k2.x * r6;
    float icdist2 = 1.0 / (1.0 + params.k2.y * r2 + params.k2.z * r4 + params.k2.w * r6);

    return vec2(
        pos.x * cdist * icdist2 + params.k1.z * a1 + params.k1.w * a2 + params.k3.x * r2 + params.k3.y * r4,
        pos.y * cdist * icdist2 + params.k1.z * a3 + params.k1.w * a1 + params.k3.z * r2 + params.k3.w * r4
    );
}
