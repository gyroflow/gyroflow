// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 from_superview(vec2 uv) {
    uv.x *= 1.0 - 0.45 * abs(uv.x); 
    uv.x *= 0.168827 * (5.53572 + abs(uv.x));
    uv.y *= 0.130841 * (7.14285 + abs(uv.y));

    return uv;
}

vec2 to_superview(vec2 uv) {
    uv.y = (3.57143 - 0.5 * sqrt(51.0203 + 30.5714 * abs(uv.y))) * (-uv.y / max(0.000001, abs(uv.y)));
    uv.x = (2.76785 - 0.5 * sqrt(30.6441 + 23.6928 * abs(uv.x))) * (-uv.x / max(0.000001, abs(uv.x)));
    uv.x = (1.11111 - 0.5 * sqrt(4.93827 - 8.88889 * abs(uv.x))) * ( uv.x / max(0.000001, abs(uv.x)));

    return uv;
}
