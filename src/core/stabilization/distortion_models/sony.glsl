// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)

float get_param(float row, float idx);
float get_mesh_data(int idx);
float map_coord(float x, float in_min, float in_max, float out_min, float out_max);

// Sony native lens model: the camera's lens curve (ray angle at equally spaced image radii) evaluated as a natural
// cubic spline radius -> angle. Layout of params.k1..k6: see sony.rs.
float sony_k(int i) {
    int v = i / 4;
    vec4 q = v == 0 ? params.k1 : v == 1 ? params.k2 : v == 2 ? params.k3 : v == 3 ? params.k4 : v == 4 ? params.k5 : params.k6;
    int c = i - v * 4;
    return c == 0 ? q.x : c == 1 ? q.y : c == 2 ? q.z : q.w;
}
int sony_segments() {
    float n = params.k1.x;
    if (n >= 1.0 && n <= 10.0 && params.k1.y > 0.0 && params.k1.z == 0.0) return int(n);
    return 0;
}
// (y_i, b_i, c_i, d_i) of spline segment i
vec4 sony_segment(int i) {
    float h = params.k1.y;
    float y0 = sony_k(2 + i), y1 = sony_k(3 + i);
    float c0 = i == 0 ? 0.0 : sony_k(13 + i), c1 = sony_k(14 + i); // k[13] holds r_lin, c_0 is 0
    return vec4(y0, (y1 - y0) / h - h * (c1 + 2.0 * c0) / 3.0, c0, (c1 - c0) / (3.0 * h));
}
const float SONY_THETA0 = 0.052359879; // 3 deg, end of the linear region near the optical axis
// Ray angle at normalized image radius r (linear continuation with the end slope outside the knots)
float sony_angle_at(int n, float r) {
    float h = params.k1.y;
    float r_lin = sony_k(13);
    if (r_lin > 0.0 && r < r_lin) return SONY_THETA0 * r / r_lin;
    int i = int(clamp(floor(r / h), 0.0, float(n - 1)));
    vec4 s = sony_segment(i);
    float dx = clamp(r - float(i) * h, 0.0, h);
    float slope = s.y + (2.0 * s.z + 3.0 * s.w * dx) * dx;
    return s.x + (s.y + (s.z + s.w * dx) * dx) * dx + slope * (r - float(i) * h - dx);
}
// Normalized image radius for ray angle theta (inverse of sony_angle_at)
float sony_radius_at(int n, float theta) {
    float h = params.k1.y;
    if (theta <= 0.0) return 0.0;
    float r_lin = sony_k(13);
    if (r_lin > 0.0 && theta < SONY_THETA0) return theta * r_lin / SONY_THETA0;
    int i = 0;
    while (i + 1 < n && theta >= sony_k(3 + i)) { i++; }
    vec4 s = sony_segment(i);
    float y1 = sony_k(3 + i);
    if (theta >= y1) { // past the last knot: linear continuation
        float slope = s.y + (2.0 * s.z + 3.0 * s.w * h) * h;
        return float(n) * h + (slope > 1e-9 ? (theta - y1) / slope : 0.0);
    }
    // Newton on the segment's cubic, starting from the chord
    float dx = h * (theta - s.x) / max(y1 - s.x, 1e-12);
    for (int it = 0; it < 8; ++it) {
        float f = s.x + (s.y + (s.z + s.w * dx) * dx) * dx - theta;
        float fp = s.y + (2.0 * s.z + 3.0 * s.w * dx) * dx;
        if (abs(fp) < 1e-12) break;
        float fix = f / fp;
        dx = clamp(dx - fix, 0.0, h);
        if (abs(fix) < 1e-7) break;
    }
    return float(i) * h + dx;
}

vec2 undistort_point(vec2 pos) {
    int n = sony_segments();
    if (n == 0) return pos;
    float r = length(pos);
    if (r < 1e-9) return pos;
    float theta = sony_angle_at(n, r);
    if (theta <= 0.0) return vec2(-99999.0, -99999.0);
    // Clamp the angle just under tan()'s 90° asymptote and continue the radius linearly past it, so over-FOV rays
    // stay large and monotonic (no fold back into the frame) and r_limit clips them. See sony.rs / gopro.rs.
    const float TMAX = 1.5533; const float tt = 57.14902; // TMAX ≈ 89°, tt = tan(TMAX)
    float rr = theta < TMAX ? tan(theta) : tt + (theta - TMAX) * (1.0 + tt * tt);
    if (params.r_limit > 0.0 && rr > params.r_limit) return vec2(-99999.0, -99999.0);
    return pos * (rr / r);
}

vec2 distort_point(float x, float y, float z) {
    vec2 pos = vec2(x, y) / z;
    int n = sony_segments();
    if (n == 0) return pos;
    float r = length(pos);
    if (r < 1e-9) return pos;
    // Inverse of undistort_point's angle clamp
    const float TMAX = 1.5533; const float tt = 57.14902;
    float theta = r < tt ? atan(r) : TMAX + (r - tt) / (1.0 + tt * tt);
    return pos * (sony_radius_at(n, theta) / r);
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
float bivariate_spline_interpolate(int base, float size_x, float size_y, int mesh_offset, int n, float x, float y) {
    float intermediate_values[GRID_SIZE];

    int i = int(max(0.0, min(float(GRID_SIZE - 2), (float(GRID_SIZE - 1) * x / size_x))));
    float dx = x - size_x * float(i) / float(GRID_SIZE - 1);
    float dx2 = dx * dx;
    int block_ = GRID_SIZE * 4;
    int offs = base + 9 + GRID_SIZE * GRID_SIZE * 2 + (block_ * GRID_SIZE * mesh_offset) + i;

    for (int j = 0; j < GRID_SIZE; j++) {
        intermediate_values[j] = get_mesh_data(offs + GRID_SIZE * 0 + (j * block_))
                               + get_mesh_data(offs + GRID_SIZE * 1 + (j * block_)) * dx
                               + get_mesh_data(offs + GRID_SIZE * 2 + (j * block_)) * dx2
                               + get_mesh_data(offs + GRID_SIZE * 3 + (j * block_)) * dx2 * dx;
    }

    cubic_spline_coefficients(intermediate_values, 1, 0, size_y, GRID_SIZE);
    return cubic_spline_interpolate2(GRID_SIZE, y, size_y);
}
vec2 interpolate_mesh(int base, int width, int height, vec2 pos) {
    return vec2(
        bivariate_spline_interpolate(base, float(width), float(height), 0, GRID_SIZE, pos.x, pos.y),
        bivariate_spline_interpolate(base, float(width), float(height), 1, GRID_SIZE, pos.x, pos.y)
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

        vec2 q = uv;
        uv = interpolate_mesh(0, int(mesh_size.x), int(mesh_size.y), q);
        // The 9x9 inverse mesh is only approximate for large warps, refine against the camera's forward mesh (fwd(p) = q).
        // The block is only there when the mesh needs it (sony::MESH_REFINE_THRESHOLD_PX), and a first correction that
        // is already tiny leaves nothing for a second one (sony::MESH_REFINE_SKIP_PX, squared here)
        int o = int(get_mesh_data(0));
        int fwd = o + 4 + 2 * int(max(get_mesh_data(o), 0.0));
        if (get_mesh_data(fwd) > 10.0) {
            for (int it = 0; it < 2; it++) {
                vec2 delta = q - interpolate_mesh(fwd, int(mesh_size.x), int(mesh_size.y), uv);
                uv += delta;
                if (dot(delta, delta) < 0.0625) { break; }
            }
        }

        uv.x = map_coord(uv.x, origin.x, origin.x + crop_size.x, 0.0, params.width);
        uv.y = map_coord(uv.y, origin.y, origin.y + crop_size.y, 0.0, params.height);

        if (bool(params.flags & 128)) { uv.y = params.height - uv.y; } // framebuffer inverted
    }

    // FocalPlaneDistortion
    if (get_mesh_data(0) > 0.0 && get_mesh_data(int(get_mesh_data(0))) > 0.0) {
        int o = int(get_mesh_data(0)); // offset to focal plane distortion data

        vec2 mesh_size = vec2(get_mesh_data(3), get_mesh_data(4));
        vec2 origin    = vec2(get_mesh_data(5), get_mesh_data(6));
        vec2 crop_size = vec2(get_mesh_data(7), get_mesh_data(8));
        float stblz_grid = get_mesh_data(o + 2) > 0.0 ? get_mesh_data(o + 2) : mesh_size.y / 8.0; // band height comes with the table

        if (bool(params.flags & 128)) { uv.y = params.height - uv.y; } // framebuffer inverted

        uv.x = map_coord(uv.x, 0.0, params.width,  origin.x, origin.x + crop_size.x);
        uv.y = map_coord(uv.y, 0.0, params.height, origin.y, origin.y + crop_size.y);

        int idx = min(7, max(0, int(floor(uv.y / stblz_grid))));
        float delta = uv.y - stblz_grid * float(idx);
        uv.x -= get_mesh_data(o + 4 + idx * 2 + 0) * delta;
        uv.y -= get_mesh_data(o + 4 + idx * 2 + 1) * delta;
        for (int j = 0; j < idx; j++) {
            uv.x -= get_mesh_data(o + 4 + j * 2 + 0) * stblz_grid;
            uv.y -= get_mesh_data(o + 4 + j * 2 + 1) * stblz_grid;
        }

        uv.x = map_coord(uv.x, origin.x, origin.x + crop_size.x, 0.0, params.width);
        uv.y = map_coord(uv.y, origin.y, origin.y + crop_size.y, 0.0, params.height);

        if (bool(params.flags & 128)) { uv.y = params.height - uv.y; } // framebuffer inverted
    }

    return uv;
}
