// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 superview(vec2 uv) {
    // The polynomials are a fit over the recorded frame [-0.5, 0.5]; outside it the high-order terms run
    // away and the inversion below diverges to NaN. Clamp the argument to the frame and continue with slope
    // 1 past it - identical in-domain, monotonic outside it. See gopro_superview.rs
    float x = clamp(uv.x, -0.5, 0.5);
    float y = clamp(uv.y, -0.5, 0.5);
    float x2 = x * x;
    float y2 = y * y;
    return vec2(
        x * (1.2100393 + x2 * (-1.2758402 + x2 * 1.7751845)) + (uv.x - x),
        y * (0.9364505 + (0.4465308 - 0.7683315 * y2) * y2 + (-0.3574087 + 1.1584653 * y2 + 0.3529348 * x2) * x2) + (uv.y - y)
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
    vec2 n = (uv / size) - 0.5;
    vec2 target = vec2(n.x * 1.333333333, n.y);

    vec2 P = n; // seed inside the recorded domain [-0.5,0.5]
    for (int i = 0; i < 12; ++i) {
        vec2 diff = superview(P) - target;
        if (abs(diff.x) < 1e-6 && abs(diff.y) < 1e-6) {
            break;
        }
        P -= diff;
    }

    return (P + 0.5) * size;
}
