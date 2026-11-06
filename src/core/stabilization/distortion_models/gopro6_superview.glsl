// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 superview(vec2 _uv) {
    // The polynomials are a fit over the recorded frame [-0.5, 0.5]; outside it the high-order terms run
    // away and the inversion below diverges to NaN. Clamp the argument to the frame and continue with slope
    // 1 past it - identical in-domain, monotonic outside it. See gopro6_superview.rs
    vec2 c = clamp(_uv, -0.5, 0.5);
    vec2 uv = c;
    uv.x *= 1.0 - 0.48 * abs(uv.x);
    uv.x *= 0.943396 * (1.0 + 0.157895 * abs(uv.x));
    uv.y *= 0.943396 * (1.0 + 0.060000 * abs(uv.y * 2.0));
    return uv + (_uv - c);
}

vec2 digital_undistort_point(vec2 uv) {
    vec2 out_c2 = vec2(params.output_width, params.output_height);
    uv = (uv / out_c2) - 0.5;

    uv = superview(uv);

    uv = (uv + 0.5) * out_c2;
    return uv;
}
vec2 digital_distort_point(vec2 uv) {
    vec2 size = vec2(params.width, params.height);
    uv = (uv / size) - 0.5;

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
