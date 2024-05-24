// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)

vec2 undistort_point(vec2 pos) {
    if (params.k1 == vec4(0.0, 0.0, 0.0, 0.0)) return pos;

    vec2 post_scale = vec2(params.k2.z, params.k2.w);
    pos /= post_scale;

    // now pos is in meters from center of sensor

    float theta_d = length(pos);

    bool converged = false;
    float theta = theta_d;

    float scale = 0.0;

    if (abs(theta_d) > 1e-6) {
        for (int i = 0; i < 10; ++i) {
            float theta2 = theta*theta;
            float theta3 = theta2*theta;
            float theta4 = theta2*theta2;
            float theta5 = theta2*theta3;
            float k0 = params.k1.x;
            float k1_theta1 = params.k1.y * theta;
            float k2_theta2 = params.k1.z * theta2;
            float k3_theta3 = params.k1.w * theta3;
            float k4_theta4 = params.k2.x * theta4;
            float k5_theta5 = params.k2.y * theta5;
            float theta_fix = (theta * (k0 + k1_theta1 + k2_theta2 + k3_theta3 + k4_theta4 + k5_theta5) - theta_d)
                              /
                              (k0 + 2.0 * k1_theta1 + 3.0 * k2_theta2 + 4.0 * k3_theta3 + 5.0 * k4_theta4 + 6.0 * k5_theta5);

            theta -= theta_fix;
            if (abs(theta_fix) < 1e-6) {
                converged = true;
                break;
            }
        }

        scale = tan(theta) / theta_d;
    } else {
        converged = true;
    }
    bool theta_flipped = (theta_d < 0.0 && theta > 0.0) || (theta_d > 0.0 && theta < 0.0);

    if (converged && !theta_flipped) {
        return pos * scale;
    }
    return vec2(0.0, 0.0);
}

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
