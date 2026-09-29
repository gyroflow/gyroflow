// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 undistort_point(vec2 pos) {
    // No calibration: a pinhole, so the image radius is `tan θ`
    if (params.k1 == vec4(0.0, 0.0, 0.0, 0.0)) {
        float r = length(pos);
        return r < 1e-12? pos : pos * (atan(r) / r);
    }

    float theta_d = min(max(length(pos), -3.141592653589793), 3.141592653589793); // PI

    bool converged = false;
    float theta = theta_d;

    float scale = 0.0;

    if (abs(theta_d) > 1e-6) {
        theta = 0.0;
        for (int i = 0; i < 15; ++i) {
            float theta2 = theta*theta;
            float theta4 = theta2*theta2;
            float theta6 = theta4*theta2;
            float theta8 = theta6*theta2;
            float k0_theta2 = params.k1.x * theta2;
            float k1_theta4 = params.k1.y * theta4;
            float k2_theta6 = params.k1.z * theta6;
            float k3_theta8 = params.k1.w * theta8;
            // new_theta = theta - theta_fix, theta_fix = f0(theta) / f0'(theta)
            float theta_fix = clamp((theta * (1.0 + k0_theta2 + k1_theta4 + k2_theta6 + k3_theta8) - theta_d)
                                    /
                                    (1.0 + 3.0 * k0_theta2 + 5.0 * k1_theta4 + 7.0 * k2_theta6 + 9.0 * k3_theta8), -0.9, 0.9);

            theta -= theta_fix;
            if (abs(theta_fix) < 1e-6) {
                converged = true;
                break;
            }
        }

        scale = theta / theta_d;
    } else {
        converged = true;
    }
    bool theta_flipped = (theta_d < 0.0 && theta > 0.0) || (theta_d > 0.0 && theta < 0.0);

    // Nothing past 180 degrees is a ray the pipeline can carry: the direction is built out of sin(theta),
    // which turns over there, so an angle the Newton wandered past it comes back from behind the camera.
    // See opencv_fisheye.rs
    if (abs(theta) >= 3.14159265) { return vec2(-99999.0, -99999.0); }

    if (converged && !theta_flipped) {
        return pos * scale;
    }
    return vec2(-99999.0, -99999.0);
}

vec2 distort_point(float x, float y, float z) {
    // No calibration: a pinhole, which has no image of a ray at or past 90°
    if (params.k1 == vec4(0.0, 0.0, 0.0, 0.0)) return z > 1e-9? vec2(x, y) / z : vec2(x, y) * 1e9;
    vec2 pos = vec2(x, y);

    float r = length(pos);

    // atan2 against the ray's own z, so the angle is right past 90° too
    float theta = atan(r, z);
    float theta2 = theta*theta,
          theta4 = theta2*theta2,
          theta6 = theta4*theta2,
          theta8 = theta4*theta4;

    float theta_d = theta * (1.0 + dot(params.k1, vec4(theta2, theta4, theta6, theta8)));

    float scale = r < 1e-12? 1.0 : theta_d / r;
    return pos * scale;
}
