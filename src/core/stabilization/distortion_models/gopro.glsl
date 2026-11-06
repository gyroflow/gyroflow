// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>
//
// GoPro native radial lens model. Raw POLY radial coeffs r0..r6 in params.k1/k2.
// The Superview/Hyperview MAPX/MAPY warp is a separate digital lens (gopro_warp).

float gopro_poly_eval(float p) {
    return params.k1.x + p * (params.k1.y + p * (params.k1.z + p * (params.k1.w + p * (params.k2.x + p * (params.k2.y + p * params.k2.z)))));
}
float gopro_poly_deriv(float p) {
    return params.k1.y + p * (2.0 * params.k1.z + p * (3.0 * params.k1.w + p * (4.0 * params.k2.x + p * (5.0 * params.k2.y + p * (6.0 * params.k2.z)))));
}
float gopro_poly_invert(float theta) {
    float p = (theta - params.k1.x) / params.k1.y;
    for (int i = 0; i < 10; ++i) {
        float d = gopro_poly_deriv(p);
        if (abs(d) < 1e-12) break;
        float fix = (gopro_poly_eval(p) - theta) / d;
        p -= fix;
        if (abs(fix) < 1e-7) break;
    }
    return p;
}

vec2 undistort_point(vec2 pos) {
    float r_norm = length(pos);
    // No POLY block: a pinhole, so the image radius is `tan θ`
    if (params.k1.y == 0.0) return r_norm < 1e-12? pos : pos * (atan(r_norm) / r_norm);
    if (r_norm < 1e-9) return pos;
    float p = r_norm / params.k1.y;
    float theta = gopro_poly_eval(p);
    // Outside its fit range the POLY is free to run negative or past 180°, and neither is an angle this
    // pipeline can carry - a negative theta hands back the ray from the opposite side of the frame, and
    // past 180 degrees sin(theta) has turned over and the ray arrives from behind the camera. See gopro.rs
    if (!(theta > 0.0 && theta < 3.14159265)) { return vec2(-99999.0, -99999.0); }
    return pos * (theta / r_norm);
}

vec2 distort_point(float x, float y, float z) {
    // No POLY block: a pinhole, which has no image of a ray at or past 90°
    if (params.k1.y == 0.0) return z > 1e-9? vec2(x, y) / z : vec2(x, y) * 1e9;
    vec2 pos = vec2(x, y);
    float r = length(pos);
    if (r < 1e-12) return vec2(0.0, 0.0);
    // atan2 against the ray's own z: a GoPro's 150°+ field needs the angle, not a z=1 plane radius
    float theta = atan(r, z);
    return pos * (params.k1.y * gopro_poly_invert(theta) / r);
}
