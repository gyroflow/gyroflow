// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 hyperview(vec2 uv) {
    // The polynomials are a fit over the recorded frame [-0.5, 0.5]; outside it the high-order terms run
    // away and the inversion below diverges to NaN. Clamp the argument to the frame and continue with slope
    // 1 past it - identical in-domain, monotonic outside it. See gopro_hyperview.rs
    float x = clamp(uv.x, -0.5, 0.5);
    float y = clamp(uv.y, -0.5, 0.5);
    float x2 = x * x;
    float y2 = y * y;
    return vec2(
        x * (1.5805143 + x2 * (-8.1668825 + x2 * (74.5198746 + x2 * (-451.5002441 + x2 * (1551.2922363 + x2 * (-2735.5422363 + x2 * 1923.1572266))))) + y2 * -0.1086027) + (uv.x - x),
        y * (1.0238225 + y2 * -0.1025671 + x2 * (-0.2639930 + x2 * 0.2979266)) + (uv.y - y)
    );
}

vec2 digital_undistort_point(vec2 uv) {
    vec2 out_c2 = vec2(params.output_width, params.output_height);
    uv = (uv / out_c2) - 0.5;

    uv = hyperview(uv);

    uv.x = uv.x / 1.555555555;
    uv = (uv + 0.5) * out_c2;
    return uv;
}
vec2 digital_distort_point(vec2 uv) {
    vec2 size = vec2(params.width, params.height);
    vec2 n = (uv / size) - 0.5;
    vec2 target = vec2(n.x * 1.555555555, n.y);

    vec2 P = n; // seed inside the recorded domain [-0.5,0.5]
    for (int i = 0; i < 12; ++i) {
        vec2 diff = hyperview(P) - target;
        if (abs(diff.x) < 1e-6 && abs(diff.y) < 1e-6) {
            break;
        }
        P -= diff;
    }

    return (P + 0.5) * size;
}
