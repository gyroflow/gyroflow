// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 superview(vec2 uv) {
    float x2 = uv.x * uv.x;
    float y2 = uv.y * uv.y;
    return vec2(
        uv.x * (1.2100393 + x2 * (-1.2758402 + x2 * 1.7751845)),
        uv.y * (0.9364505 + (0.4465308 - 0.7683315 * y2) * y2 + (-0.3574087 + 1.1584653 * y2 + 0.3529348 * x2) * x2)
    );
}

vec2 digital_undistort_point(vec2 uv) {
    vec2 out_c2 = vec2(params.output_width, params.output_height);
    uv = (uv / out_c2) - 0.5;

    uv = superview(uv);

    uv.x = uv.x / 1.333333333;
    uv = (uv + 0.5) * out_c2;
    return uv;
}
vec2 digital_distort_point(vec2 uv) {
    vec2 size = vec2(params.width, params.height);
    uv = (uv / size) - 0.5;
    uv.x = uv.x * 1.333333333;

    vec2 P = uv;
    for (int i = 0; i < 12; ++i) {
        vec2 diff = superview(P) - uv;
        if (abs(diff.x) < 1e-6 && abs(diff.y) < 1e-6) {
            break;
        }
        P -= diff;
    }

    uv = (P + 0.5) * size;

    return uv;
}
