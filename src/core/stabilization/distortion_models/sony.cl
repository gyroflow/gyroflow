// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)

float2 undistort_point(float2 pos, __global KernelParams *params) {
    if (params->k[0] == 0.0 && params->k[1] == 0.0 && params->k[2] == 0.0 && params->k[3] == 0.0) return pos;

    // pos is a dimensionless normalized image-plane radius

    float theta_d = length(pos);

    bool converged = false;
    float theta = theta_d;

    float scale = 0.0f;

    if (fabs(theta_d) > 1e-6f) {
        theta = 0.0f;
        for (int i = 0; i < 15; ++i) {
            float theta2 = theta*theta,
                  theta3 = theta2*theta,
                  theta4 = theta2*theta2,
                  theta5 = theta2*theta3;
            float k0 = params->k[0];
            float k1_theta1 = params->k[1] * theta;
            float k2_theta2 = params->k[2] * theta2;
            float k3_theta3 = params->k[3] * theta3;
            float k4_theta4 = params->k[4] * theta4;
            float k5_theta5 = params->k[5] * theta5;
            float theta_fix = clamp((theta * (k0 + k1_theta1 + k2_theta2 + k3_theta3 + k4_theta4 + k5_theta5) - theta_d)
                                    /
                                    (k0 + 2.0f * k1_theta1 + 3.0f * k2_theta2 + 4.0f * k3_theta3 + 5.0f * k4_theta4 + 6.0f * k5_theta5), -0.9f, 0.9f);

            theta -= theta_fix;
            if (fabs(theta_fix) < 1e-6f) {
                converged = true;
                break;
            }
        }

        scale = tan(theta) / theta_d;
    } else {
        converged = true;
    }
    bool theta_flipped = (theta_d < 0.0f && theta > 0.0f) || (theta_d > 0.0f && theta < 0.0f);

    bool out_of_range = fabs(theta) >= 1.5707963267948966f || (params->r_limit > 0.0f && fabs(scale * theta_d) > params->r_limit);

    if (converged && !theta_flipped && !out_of_range) {
        return pos * scale;
    }
    return (float2)(-99999.0f, -99999.0f);
}

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    float2 pos = (float2)(x, y) / z;
    if (params->k[0] == 0.0 && params->k[1] == 0.0 && params->k[2] == 0.0 && params->k[3] == 0.0) return pos;

    float r = length(pos);
    float theta = atan(r);

    float theta2 = theta*theta,
          theta3 = theta2*theta,
          theta4 = theta2*theta2,
          theta5 = theta2*theta3,
          theta6 = theta3*theta3;

    float theta_d = theta  * params->k[0]
                  + theta2 * params->k[1]
                  + theta3 * params->k[2]
                  + theta4 * params->k[3]
                  + theta5 * params->k[4]
                  + theta6 * params->k[5];

    float scale = r == 0.0f? 1.0f : theta_d / r;

    return pos * scale;
}
