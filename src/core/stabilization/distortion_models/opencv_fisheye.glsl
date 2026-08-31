// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

vec2 undistort_point(vec2 pos) {
    if (params.k1 == vec4(0.0, 0.0, 0.0, 0.0)) return pos;

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

        scale = tan(theta) / theta_d;
    } else {
        converged = true;
    }
    bool theta_flipped = (theta_d < 0.0 && theta > 0.0) || (theta_d > 0.0 && theta < 0.0);

    bool out_of_range = abs(theta) >= 1.5707963267948966 || (params.r_limit > 0.0 && abs(scale * theta_d) > params.r_limit);

    if (converged && !theta_flipped && !out_of_range) {
        return pos * scale;
    }
    return vec2(-99999.0, -99999.0);
}

vec2 distort_point(float x, float y, float z) {
    vec2 pos = vec2(x, y) / z;
    if (params.k1 == vec4(0.0, 0.0, 0.0, 0.0)) return pos;

    float r = length(pos);

    float theta = atan(r);
    float theta2 = theta*theta,
          theta4 = theta2*theta2,
          theta6 = theta4*theta2,
          theta8 = theta4*theta4;

    float theta_d = theta * (1.0 + dot(params.k1, vec4(theta2, theta4, theta6, theta8)));

    float scale = r == 0? 1.0 : theta_d / r;
    return pos * scale;
}
