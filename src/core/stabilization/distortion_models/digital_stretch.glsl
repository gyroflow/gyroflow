// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 digital_undistort_point(vec2 uv) {
    uv.x /= params.digital_lens_params.x;
    uv.y /= params.digital_lens_params.y;
    return uv;
}

vec2 digital_distort_point(vec2 uv) {
    uv.x *= params.digital_lens_params.x;
    uv.y *= params.digital_lens_params.y;
    return uv;
}
