// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 distort_point(float x, float y, float z) {
    float k1 = params.k1.x;
    float k2 = params.k1.y;
    float k3 = params.k1.z;
    float p1 = params.k1.w;

    float p2 = params.k2.x;
    float xi = params.k2.y;

    vec3 P = vec3(x, y, z);
    P /= length(P);

    x = P.x / (P.z + xi);
    y = P.y / (P.z + xi);

    float r2 = x*x + y*y;
    float r4 = r2 * r2;
    float r6 = r4 * r2;

    return vec2(
        x * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p1*x*y + p2*(r2 + 2.0*x*x),
        y * (1.0 + k1*r2 + k2*r4 + k3*r6) + 2.0*p2*x*y + p1*(r2 + 2.0*y*y)
    );
}

vec2 undistort_point(vec2 p) {
    vec2 P = p;

    for (int i = 0; i < 200; i++) {
        vec2 diff = distort_point(P.x, P.y, 1.0) - p;
        if (abs(diff.x) < 1e-6 && abs(diff.y) < 1e-6) {
            break;
        }
        P -= diff;
    }

    return P;
}
