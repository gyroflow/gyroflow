// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>
//
// GoPro Superview/Hyperview digital warp, data-driven from MAPX/MAPY in params.digital_lens_params.
// Companion digital lens to the `gopro` radial model. See gopro_warp.rs for the layout.

vec2 gopro_map(vec2 uv) {
    // Clamp the polynomial argument to the valid recorded-frame domain [-0.5,0.5] and continue linearly past
    // it, keeping the map smooth & monotonic everywhere (no edge-swirl / Newton divergence). See gopro_warp.rs.
    float x = clamp(uv.x, -0.5, 0.5);
    float y = clamp(uv.y, -0.5, 0.5);
    float x2 = x * x;
    float y2 = y * y;
    float c0 = params.digital_lens_params[0].x, c1 = params.digital_lens_params[0].y, c2 = params.digital_lens_params[0].z, c3 = params.digital_lens_params[0].w;
    float c4 = params.digital_lens_params[1].x, c5 = params.digital_lens_params[1].y, c6 = params.digital_lens_params[1].z, c7 = params.digital_lens_params[1].w;
    float d0 = params.digital_lens_params[2].x, d1 = params.digital_lens_params[2].y, d2 = params.digital_lens_params[2].z, d3 = params.digital_lens_params[2].w;
    float d4 = params.digital_lens_params[3].x, d5 = params.digital_lens_params[3].y;
    float poly_x = c0 + x2 * (c1 + x2 * (c2 + x2 * (c3 + x2 * (c4 + x2 * (c5 + x2 * c6)))));
    return vec2(
        x * (poly_x + c7 * y2) + (uv.x - x),
        y * (d0 + d1 * y2 + d2 * y2 * y2 + x2 * (d3 + d4 * y2 + d5 * x2)) + (uv.y - y)
    );
}

vec2 digital_undistort_point(vec2 uv) {
    float factor = params.digital_lens_params[3].z; if (factor == 0.0) factor = 1.0;
    vec2 out_c2 = vec2(params.output_width, params.output_height);
    uv = (uv / out_c2) - 0.5;
    uv = gopro_map(uv);
    uv.x = uv.x / factor;
    uv = (uv + 0.5) * out_c2;
    return uv;
}
vec2 digital_distort_point(vec2 uv) {
    float factor = params.digital_lens_params[3].z; if (factor == 0.0) factor = 1.0;
    vec2 size = vec2(params.width, params.height);
    vec2 n = (uv / size) - 0.5;
    vec2 target = vec2(n.x * factor, n.y);
    vec2 P = n; // seed inside the recorded domain [-0.5,0.5]
    for (int i = 0; i < 12; ++i) {
        vec2 diff = gopro_map(P) - target;
        if (abs(diff.x) < 1e-6 && abs(diff.y) < 1e-6) break;
        P -= diff;
    }
    vec2 res = gopro_map(P) - target; // reject out-of-domain (beyond recorded frame) -> background, no swirl
    if (abs(res.x) > 0.02 || abs(res.y) > 0.02) return vec2(-99999.0, -99999.0);
    return (P + 0.5) * size;
}
