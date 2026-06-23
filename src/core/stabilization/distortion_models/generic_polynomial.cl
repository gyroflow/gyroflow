// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>
//
// Generic polynomial fisheye projection — proto's `GenericPolynomial` variant.
//   r_normalized = k0·θ + k1·θ² + k2·θ³ + ... + k11·θ¹²
// (dimensionless, k0 ≈ 1.0 for paraxial). Output pixels = r_normalized · f_px;
// the main kernel multiplies by params->f after this function returns.
// Calibrations shorter than 12 terms ride on zero-padded trailing slots.

float2 undistort_point(float2 pos, __global KernelParams *params) {
    if (params->k[0]  == 0.0 && params->k[1]  == 0.0 && params->k[2]  == 0.0 && params->k[3]  == 0.0
     && params->k[4]  == 0.0 && params->k[5]  == 0.0 && params->k[6]  == 0.0 && params->k[7]  == 0.0
     && params->k[8]  == 0.0 && params->k[9]  == 0.0 && params->k[10] == 0.0 && params->k[11] == 0.0) return pos;

    float theta_d = length(pos);

    bool converged = false;
    float theta = theta_d;
    float scale = 0.0f;

    if (fabs(theta_d) > 1e-6f) {
        for (int i = 0; i < 10; ++i) {
            float theta2  = theta*theta;
            float theta3  = theta2*theta;
            float theta4  = theta2*theta2;
            float theta5  = theta2*theta3;
            float theta6  = theta3*theta3;
            float theta7  = theta3*theta4;
            float theta8  = theta4*theta4;
            float theta9  = theta4*theta5;
            float theta10 = theta5*theta5;
            float theta11 = theta5*theta6;
            float k0          = params->k[0];
            float k1_theta1   = params->k[1]  * theta;
            float k2_theta2   = params->k[2]  * theta2;
            float k3_theta3   = params->k[3]  * theta3;
            float k4_theta4   = params->k[4]  * theta4;
            float k5_theta5   = params->k[5]  * theta5;
            float k6_theta6   = params->k[6]  * theta6;
            float k7_theta7   = params->k[7]  * theta7;
            float k8_theta8   = params->k[8]  * theta8;
            float k9_theta9   = params->k[9]  * theta9;
            float k10_theta10 = params->k[10] * theta10;
            float k11_theta11 = params->k[11] * theta11;
            float theta_fix = (theta * (k0 + k1_theta1 + k2_theta2 + k3_theta3 + k4_theta4 + k5_theta5 + k6_theta6 + k7_theta7 + k8_theta8 + k9_theta9 + k10_theta10 + k11_theta11) - theta_d)
                              /
                              (k0 + 2.0f * k1_theta1 + 3.0f * k2_theta2 + 4.0f * k3_theta3 + 5.0f * k4_theta4 + 6.0f * k5_theta5 + 7.0f * k6_theta6 + 8.0f * k7_theta7 + 9.0f * k8_theta8 + 10.0f * k9_theta9 + 11.0f * k10_theta10 + 12.0f * k11_theta11);

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
        return pos * scale;
    }
    return (float2)(0.0f, 0.0f);
}

float2 distort_point(float x, float y, float z, __global KernelParams *params) {
    float2 pos = (float2)(x, y) / z;
    if (params->k[0]  == 0.0 && params->k[1]  == 0.0 && params->k[2]  == 0.0 && params->k[3]  == 0.0
     && params->k[4]  == 0.0 && params->k[5]  == 0.0 && params->k[6]  == 0.0 && params->k[7]  == 0.0
     && params->k[8]  == 0.0 && params->k[9]  == 0.0 && params->k[10] == 0.0 && params->k[11] == 0.0) return pos;

    float r = length(pos);
    float theta = atan(r);

    float theta2  = theta*theta,
          theta3  = theta2*theta,
          theta4  = theta2*theta2,
          theta5  = theta2*theta3,
          theta6  = theta3*theta3,
          theta7  = theta3*theta4,
          theta8  = theta4*theta4,
          theta9  = theta4*theta5,
          theta10 = theta5*theta5,
          theta11 = theta5*theta6,
          theta12 = theta6*theta6;

    float theta_d = theta   * params->k[0]
                  + theta2  * params->k[1]
                  + theta3  * params->k[2]
                  + theta4  * params->k[3]
                  + theta5  * params->k[4]
                  + theta6  * params->k[5]
                  + theta7  * params->k[6]
                  + theta8  * params->k[7]
                  + theta9  * params->k[8]
                  + theta10 * params->k[9]
                  + theta11 * params->k[10]
                  + theta12 * params->k[11];

    float scale = r == 0.0f? 1.0f : theta_d / r;

    return pos * scale;
}
