// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

float2 undistort_point(float2 pos, __global KernelParams *params) {
    // No calibration: a pinhole, so the image radius is `tan θ`
    if (params->k[0] == 0.0 && params->k[1] == 0.0 && params->k[2] == 0.0 && params->k[3] == 0.0) {
        float r = length(pos);
        return r < 1e-12f? pos : pos * (atan(r) / r);
    }

    float theta_d = fmin(fmax(length(pos), -3.141592653589793f), 3.141592653589793f); // PI

    bool converged = false;
    float theta = theta_d;

    float scale = 0.0f;

    if (fabs(theta_d) > 1e-6f) {
        theta = 0.0f;
        for (int i = 0; i < 15; ++i) {
            float theta2 = theta*theta;
            float theta4 = theta2*theta2;
            float theta6 = theta4*theta2;
            float theta8 = theta6*theta2;
            float k0_theta2 = params->k[0] * theta2;
            float k1_theta4 = params->k[1] * theta4;
            float k2_theta6 = params->k[2] * theta6;
            float k3_theta8 = params->k[3] * theta8;
            // new_theta = theta - theta_fix, theta_fix = f0(theta) / f0'(theta)
            float theta_fix = clamp((theta * (1.0f + k0_theta2 + k1_theta4 + k2_theta6 + k3_theta8) - theta_d)
                                    /
                                    (1.0f + 3.0f * k0_theta2 + 5.0f * k1_theta4 + 7.0f * k2_theta6 + 9.0f * k3_theta8), -0.9f, 0.9f);

            theta -= theta_fix;
            if (fabs(theta_fix) < 1e-6f) {
                converged = true;
                break;
            }
        }

        scale = theta / theta_d;
    } else {
        converged = true;
    }
    bool theta_flipped = (theta_d < 0.0f && theta > 0.0f) || (theta_d > 0.0f && theta < 0.0f);

    // Nothing past 180 degrees is a ray the pipeline can carry: the direction is built out of sin(theta),
    // which turns over there, so an angle the Newton wandered past it comes back from behind the camera.
    // See opencv_fisheye.rs
    if (fabs(theta) >= 3.14159265f) { return (float2)(-99999.0f, -99999.0f); }

    if (converged && !theta_flipped) {
        return pos * scale;
    }
    return (float2)(-99999.0f, -99999.0f);
}

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    // No calibration: a pinhole, which has no image of a ray at or past 90°
    if (params->k[0] == 0.0 && params->k[1] == 0.0 && params->k[2] == 0.0 && params->k[3] == 0.0) return z > 1e-9f? (float2)(x, y) / z : (float2)(x, y) * 1e9f;
    float2 pos = (float2)(x, y);
    float r = length(pos);

    // atan2 against the ray's own z, so the angle is right past 90° too
    float theta = atan2(r, z);
    float theta2 = theta*theta,
          theta4 = theta2*theta2,
          theta6 = theta4*theta2,
          theta8 = theta4*theta4;

    float theta_d = theta * (1.0f + theta2 * params->k[0] + theta4 * params->k[1] + theta6 * params->k[2] + theta8 * params->k[3]);

    float scale = r < 1e-12f? 1.0f : theta_d / r;

    return pos * scale;
}
