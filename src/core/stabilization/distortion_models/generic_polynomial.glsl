// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>
//
// Generic polynomial fisheye projection — proto's `GenericPolynomial` variant.
//   r_normalized = k0·θ + k1·θ² + k2·θ³ + ... + k11·θ¹²
// dimensionless. Pixel scaling by f_x / f_y is done by the main kernel.
// 12 coefficients packed across params.k1.xyzw, k2.xyzw, k3.xyzw. Calibrations
// shorter than 12 terms ride on zero-padded trailing slots.

float get_param(float row, float idx);
float get_mesh_data(int idx);
float map_coord(float x, float in_min, float in_max, float out_min, float out_max);

vec2 undistort_point(vec2 pos) {
    if (params.k1 == vec4(0.0) && params.k2 == vec4(0.0) && params.k3 == vec4(0.0)) return pos;

    float theta_d = length(pos);

    bool converged = false;
    float theta = theta_d;

    float scale = 0.0;

    if (abs(theta_d) > 1e-6) {
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
            float k0          = params.k1.x;
            float k1_theta1   = params.k1.y * theta;
            float k2_theta2   = params.k1.z * theta2;
            float k3_theta3   = params.k1.w * theta3;
            float k4_theta4   = params.k2.x * theta4;
            float k5_theta5   = params.k2.y * theta5;
            float k6_theta6   = params.k2.z * theta6;
            float k7_theta7   = params.k2.w * theta7;
            float k8_theta8   = params.k3.x * theta8;
            float k9_theta9   = params.k3.y * theta9;
            float k10_theta10 = params.k3.z * theta10;
            float k11_theta11 = params.k3.w * theta11;
            float theta_fix = (theta * (k0 + k1_theta1 + k2_theta2 + k3_theta3 + k4_theta4 + k5_theta5 + k6_theta6 + k7_theta7 + k8_theta8 + k9_theta9 + k10_theta10 + k11_theta11) - theta_d)
                              /
                              (k0 + 2.0 * k1_theta1 + 3.0 * k2_theta2 + 4.0 * k3_theta3 + 5.0 * k4_theta4 + 6.0 * k5_theta5 + 7.0 * k6_theta6 + 8.0 * k7_theta7 + 9.0 * k8_theta8 + 10.0 * k9_theta9 + 11.0 * k10_theta10 + 12.0 * k11_theta11);

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
    if (params.k1 == vec4(0.0) && params.k2 == vec4(0.0) && params.k3 == vec4(0.0)) return pos;

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

    float theta_d = theta   * params.k1.x
                  + theta2  * params.k1.y
                  + theta3  * params.k1.z
                  + theta4  * params.k1.w
                  + theta5  * params.k2.x
                  + theta6  * params.k2.y
                  + theta7  * params.k2.z
                  + theta8  * params.k2.w
                  + theta9  * params.k3.x
                  + theta10 * params.k3.y
                  + theta11 * params.k3.z
                  + theta12 * params.k3.w;

    float scale = r == 0? 1.0 : theta_d / r;

    return pos * scale;
}

const int GRID_SIZE = 9;
float a[GRID_SIZE]; float b[GRID_SIZE]; float c[GRID_SIZE]; float d[GRID_SIZE];
float alpha[GRID_SIZE]; float mu[GRID_SIZE]; float z[GRID_SIZE];
void cubic_spline_coefficients(float mesh[GRID_SIZE], int step_, int offset, float size, int n) {
    float h = size / float(n - 1);
    float inv_h = 1.0 / h;
    float three_inv_h = 3.0 * inv_h;
    float h_over_3 = h / 3.0;
    float inv_3h = 1.0 / (3.0 * h);
    for (int i = 0; i < n; i++) { a[i] = mesh[(i + offset) * step_]; }
    for (int i = 1; i < n - 1; i++) { alpha[i] = three_inv_h * (a[i + 1] - 2.0 * a[i] + a[i - 1]); }

    mu[0] = 0.0;
    z[0] = 0.0;

    for (int i = 1; i < n - 1; i++) {
        mu[i] = 1.0 / (4.0 - mu[i - 1]);
        z[i] = (alpha[i] * inv_h - z[i - 1]) * mu[i];
    }

    c[n - 1] = 0.0;

    for (int j = n - 2; j >= 0; j--) {
        c[j] = z[j] - mu[j] * c[j + 1];
        b[j] = (a[j + 1] - a[j]) * inv_h - h_over_3 * (c[j + 1] + 2.0 * c[j]);
        d[j] = (c[j + 1] - c[j]) * inv_3h;
    }
}
float cubic_spline_interpolate2(int n, float x, float size) {
    // Linear extrapolation outside the knot range, using the spline's own derivative
    // at the endpoint so the join is C¹-continuous (no slope kink at the boundary).
    if (x <= 0.0) {
        return a[0] + b[0] * x;
    }
    if (x >= size) {
        float h = size / float(n - 1);
        float slope = b[n - 2] + 2.0 * c[n - 2] * h + 3.0 * d[n - 2] * h * h;
        return a[n - 1] + slope * (x - size);
    }
    int i = int(max(0.0, min(float(n - 2), (float(n - 1) * x / size))));
    float dx = x - size * float(i) / float(n - 1);
    return a[i] + b[i] * dx + c[i] * dx * dx + d[i] * dx * dx * dx;
}
float bivariate_spline_interpolate(float size_x, float size_y, int mesh_offset, int n, float x, float y) {
    float intermediate_values[GRID_SIZE];

    int i = int(max(0.0, min(float(GRID_SIZE - 2), (float(GRID_SIZE - 1) * x / size_x))));
    float dx = x - size_x * float(i) / float(GRID_SIZE - 1);
    float dx2 = dx * dx;
    int block_ = GRID_SIZE * 4;
    int offs = 9 + GRID_SIZE * GRID_SIZE * 2 + (block_ * GRID_SIZE * mesh_offset) + i;

    for (int j = 0; j < GRID_SIZE; j++) {
        intermediate_values[j] = get_mesh_data(offs + GRID_SIZE * 0 + (j * block_))
                               + get_mesh_data(offs + GRID_SIZE * 1 + (j * block_)) * dx
                               + get_mesh_data(offs + GRID_SIZE * 2 + (j * block_)) * dx2
                               + get_mesh_data(offs + GRID_SIZE * 3 + (j * block_)) * dx2 * dx;
    }

    cubic_spline_coefficients(intermediate_values, 1, 0, size_y, GRID_SIZE);
    return cubic_spline_interpolate2(GRID_SIZE, y, size_y);
}
vec2 interpolate_mesh(int width, int height, vec2 pos) {
    if (pos.x < 0.0 || pos.x > float(width) || pos.y < 0.0 || pos.y > float(height)) {
        return pos;
    }
    return vec2(
        bivariate_spline_interpolate(float(width), float(height), 0, GRID_SIZE, pos.x, pos.y),
        bivariate_spline_interpolate(float(width), float(height), 1, GRID_SIZE, pos.x, pos.y)
    );
}

vec2 process_coord(vec2 uv, float idx) {
    if (get_mesh_data(0) > 10.0) {
        vec2 mesh_size = vec2(get_mesh_data(3), get_mesh_data(4));
        vec2 origin    = vec2(get_mesh_data(5), get_mesh_data(6));
        vec2 crop_size = vec2(get_mesh_data(7), get_mesh_data(8));

        if (bool(params.flags & 128)) { uv.y = params.height - uv.y; } // framebuffer inverted

        uv.x = map_coord(uv.x, 0.0, params.width,  origin.x, origin.x + crop_size.x);
        uv.y = map_coord(uv.y, 0.0, params.height, origin.y, origin.y + crop_size.y);

        uv = interpolate_mesh(int(mesh_size.x), int(mesh_size.y), uv);

        uv.x = map_coord(uv.x, origin.x, origin.x + crop_size.x, 0.0, params.width);
        uv.y = map_coord(uv.y, origin.y, origin.y + crop_size.y, 0.0, params.height);

        if (bool(params.flags & 128)) { uv.y = params.height - uv.y; } // framebuffer inverted
    }
    return uv;
}
