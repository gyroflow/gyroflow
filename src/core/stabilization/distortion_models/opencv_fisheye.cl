// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

float2 undistort_point(float2 pos, float4 k, float amount) {
    float theta_d = fmin(fmax(length(pos), -1.5707963267948966f), 1.5707963267948966f); // PI/2

    bool converged = false;
    float theta = theta_d;

    float scale = 0.0f;

    if (fabs(theta_d) > 1e-6f) {
        for (int i = 0; i < 10; ++i) {
            float theta2 = theta*theta;
            float theta4 = theta2*theta2;
            float theta6 = theta4*theta2;
            float theta8 = theta6*theta2;
            float k0_theta2 = k.x * theta2;
            float k1_theta4 = k.y * theta4;
            float k2_theta6 = k.z * theta6;
            float k3_theta8 = k.w * theta8;
            // new_theta = theta - theta_fix, theta_fix = f0(theta) / f0'(theta)
            float theta_fix = (theta * (1.0f + k0_theta2 + k1_theta4 + k2_theta6 + k3_theta8) - theta_d)
                              /
                              (1.0f + 3.0f * k0_theta2 + 5.0f * k1_theta4 + 7.0f * k2_theta6 + 9.0f * k3_theta8);

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

    if (converged && !theta_flipped) {
        // Apply only requested amount
        scale = 1.0f + (scale - 1.0f) * (1.0f - amount);

        return pos * scale;
    }
    return (float2)(0.0f, 0.0f);
}

float2 distort_point(float2 pos, float4 k) {
    float r = length(pos);

    float theta = atan(r);
    float theta2 = theta*theta,
          theta4 = theta2*theta2,
          theta6 = theta4*theta2,
          theta8 = theta4*theta4;

    float theta_d = theta * (1.0 + dot(k, (float4)(theta2, theta4, theta6, theta8)));

    float scale = r == 0? 1.0 : theta_d / r;

    return pos * scale;
}
