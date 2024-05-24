// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)

vec2 distort_point(float x, float y, float z) {
    vec2 pos = vec2(x, y) / z;
    if (params.k1 == vec4(0.0, 0.0, 0.0, 0.0)) return pos;

    float r = length(pos);
    float theta = atan(r);

    float theta2 = theta*theta,
          theta3 = theta2*theta,
          theta4 = theta2*theta2,
          theta5 = theta2*theta3,
          theta6 = theta3*theta3;

    float theta_d = theta * params.k1.x + theta2 * params.k1.y + theta3 * params.k1.z + theta4 * params.k1.w + theta5 * params.k2.x + theta6 * params.k2.y;

    float scale = r == 0? 1.0 : theta_d / r;

    vec2 post_scale = vec2(params.k2.z, params.k2.w);

    return pos * scale * post_scale;
}

vec2 undistort_point(vec2 p) {
    vec2 P = p;
    if (params.k1 == vec4(0.0, 0.0, 0.0, 0.0)) return p;

    for (int i = 0; i < 20; i++) {
        vec2 diff = distort_point(P.x, P.y, 1.0) - p;
        if (abs(diff.x) < 1e-6 && abs(diff.y) < 1e-6) {
            break;
        }
        P -= diff;
    }

    return P;
}
