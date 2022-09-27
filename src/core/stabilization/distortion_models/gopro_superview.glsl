// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 digital_undistort_point(vec2 uv) {
    vec2 out_c2 = vec2(params.output_width, params.output_height);
    uv = (uv / out_c2) - 0.5;

    uv.x *= 1.0 - 0.45 * abs(uv.x);
    uv.x *= 0.168827 * (5.53572 + abs(uv.x));
    uv.y *= 0.130841 * (7.14285 + abs(uv.y));

    uv = (uv + 0.5f) * out_c2;

    return uv;
}

vec2 digital_distort_point(vec2 uv) {
    vec2 size = vec2(params.width, params.height);
    uv = (uv / size) - 0.5;

    float xs = uv.x / max(0.000001, abs(uv.x));
    float ys = uv.y / max(0.000001, abs(uv.y));

    uv.y = ys * (3.57143 * (sqrt(0.5992 * abs(uv.y) + 1.0) - 1.0));
    uv.x = xs * (3.57143 * (0.880341 * sqrt(0.5992 * abs(uv.x) + 0.775) - 0.775));
    uv.x = xs * (-1.11111 * (sqrt(1.0 - 1.8 * abs(uv.x)) - 1.0));

    uv = (uv + 0.5) * size;

    return uv;
}
