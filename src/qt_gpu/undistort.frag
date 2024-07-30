// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

// Adapted from OpenCV: initUndistortRectifyMap
// https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/calib3d/src/fisheye.cpp#L465-L567

#version 420

layout(location = 0) in vec2 v_texcoord;
layout(location = 0) out vec4 fragColor;

layout(binding = 1) uniform sampler2D texIn;

layout(std140, binding = 2) uniform KernelParams {
    int width;             // 4
    int height;            // 8
    int stride;            // 12
    int output_width;      // 16
    int output_height;     // 4
    int output_stride;     // 8
    int matrix_count;      // 12 - for rolling shutter correction. 1 = no correction, only main matrix
    int interpolation;     // 16
    int background_mode;   // 4
    int flags;             // 8
    int bytes_per_pixel;   // 12
    int pix_element_count; // 16
    vec4 background;    // 16
    vec2 f;             // 8  - focal length in pixels
    vec2 c;             // 16 - lens center
    vec4 k1, k2, k3;    // 16, 16, 16 - distortion coefficients
    float fov;          // 4
    float r_limit;      // 8
    float lens_correction_amount;   // 12
    float input_vertical_stretch;   // 16
    float input_horizontal_stretch; // 4
    float background_margin;        // 8
    float background_margin_feather;// 12
    float canvas_scale;             // 16
    float input_rotation;           // 4
    float output_rotation;          // 8
    vec2 translation2d;             // 16
    vec4 translation3d;             // 16
    ivec4 source_rect;              // 16 - x, y, w, h - unused in this kernel
    ivec4 output_rect;              // 16 - x, y, w, h - unused in this kernel
    vec4 digital_lens_params;       // 16
    vec4 safe_area_rect;            // 16
    float max_pixel_value;          // 4
    int distortion_model;           // 8
    int digital_lens;               // 12
    float pixel_value_limit;        // 16
    float light_refraction_coefficient; // 4
    int plane_index;                // 8
    float reserved1;                // 12
    float reserved2;                // 16
} params;

LENS_MODEL_FUNCTIONS;

layout(binding = 3) uniform sampler2D texParams;
layout(binding = 4) uniform sampler2D texCanvas;
layout(binding = 5) uniform sampler2D texMeshData;

const vec4 colors[9] = vec4[9](
    vec4(0.0,   0.0,   0.0,     0.0), // None
    vec4(255.0, 0.0,   0.0,   255.0), // Red
    vec4(0.0,   255.0, 0.0,   255.0), // Green
    vec4(0.0,   0.0,   255.0, 255.0), // Blue
    vec4(254.0, 251.0, 71.0,  255.0), // Yellow
    vec4(200.0, 200.0, 0.0,   255.0), // Yellow2
    vec4(255.0, 0.0,   255.0, 255.0), // Magenta
    vec4(0.0,   128.0, 255.0, 255.0), // Blue2
    vec4(0.0,   200.0, 200.0, 255.0)  // Blue3
);
const float alphas[4] = float[4](1.0, 0.75, 0.50, 0.25);
void draw_pixel(inout vec4 out_pix, float x, float y, bool isInput) {
    if (!bool(params.flags & 8)) { // Drawing not enabled
        return;
    }
    int width = max(params.width, params.output_width);
    int height = max(params.height, params.output_height);

    int data = int(ceil(texture(texCanvas, vec2(x / width, y / height)).r * 255.0));
    if (data > 0) {
        int color = (data & 0xF8) >> 3;
        int alpha = (data & 0x06) >> 1;
        int stage = data & 1;
        if (((stage == 0 && isInput) || (stage == 1 && !isInput)) && color < 9) {
            vec4 colorf = colors[color] / 255.0;
            float alphaf = alphas[alpha];
            out_pix = colorf * alphaf + out_pix * (1.0 - alphaf);
            out_pix.a = 1.0;
        }
    }
}
void draw_safe_area(inout vec4 out_pix, float x, float y) {
    bool isSafeArea = x >= params.safe_area_rect.x && x <= params.safe_area_rect.z &&
                      y >= params.safe_area_rect.y && y <= params.safe_area_rect.w;
    if (!isSafeArea) {
        out_pix.x *= 0.5;
        out_pix.y *= 0.5;
        out_pix.z *= 0.5;
        bool isBorder = x >= params.safe_area_rect.x - 5.0 && x <= params.safe_area_rect.z + 5.0 &&
                        y >= params.safe_area_rect.y - 5.0 && y <= params.safe_area_rect.w + 5.0;
        if (isBorder) {
            out_pix.x *= 0.5;
            out_pix.y *= 0.5;
            out_pix.z *= 0.5;
        }
    }
}

float get_param(float row, float idx) {
    int size = bool(params.flags & 16)? params.width : params.height;
    return texture(texParams, vec2(idx / 13.0, row / float(size - 1))).r;
}

float get_mesh_data(int idx) {
    return texture(texMeshData, vec2(0, idx / 1023.0)).r;
}
const int GRID_SIZE = 9;
float a[GRID_SIZE]; float b[GRID_SIZE]; float c[GRID_SIZE]; float d[GRID_SIZE];
float h[GRID_SIZE]; float alpha[GRID_SIZE]; float l[GRID_SIZE]; float mu[GRID_SIZE]; float z[GRID_SIZE];
void cubic_spline_coefficients(float mesh[GRID_SIZE], int step_, int offset, float size, int n) {
    for (int i = 0; i < n; i++) { a[i] = mesh[(i + offset) * step_]; }
    for (int i = 0; i < n - 1; i++) { h[i] = size * (i + 1) / (n - 1) - size * i / (n - 1); }
    for (int i = 1; i < n - 1; i++) { alpha[i] = (3.0 / h[i] * (a[i + 1] - a[i])) - (3.0 / h[i - 1] * (a[i] - a[i - 1])); }

    l[0] = 1.0; mu[0] = 0.0; z[0] = 0.0;

    for (int i = 1; i < n - 1; i++) {
        l[i] = 2.0 * (size * (i + 1) / (n - 1) - size * (i - 1) / (n - 1)) - h[i - 1] * mu[i - 1];
        mu[i] = h[i] / l[i];
        z[i] = (alpha[i] - h[i - 1] * z[i - 1]) / l[i];
    }

    l[n - 1] = 1.0; z[n - 1] = 0.0; c[n - 1] = 0.0;

    for (int j = n - 2; j >= 0; j--) {
        c[j] = z[j] - mu[j] * c[j + 1];
        b[j] = (a[j + 1] - a[j]) / h[j] - h[j] * (c[j + 1] + 2.0 * c[j]) / 3.0;
        d[j] = (c[j + 1] - c[j]) / (3.0 * h[j]);
    }
}
float cubic_spline_interpolate1(int aa, int bb, int cc, int dd, int n, float x, float size) {
    int i = int(max(0.0, min(float(n - 2), (float(n - 1) * x / size))));
    float dx = x - size * float(i) / float(n - 1);
    return get_mesh_data(aa + i) + get_mesh_data(bb + i) * dx + get_mesh_data(cc + i) * dx * dx + get_mesh_data(dd + i) * dx * dx * dx;
}
float cubic_spline_interpolate2(int n, float x, float size) {
    int i = int(max(0.0, min(float(n - 2), (float(n - 1) * x / size))));
    float dx = x - size * float(i) / float(n - 1);
    return a[i] + b[i] * dx + c[i] * dx * dx + d[i] * dx * dx * dx;
}
float bivariate_spline_interpolate(float size_x, float size_y, int mesh_offset, int n, float x, float y) {
    float intermediate_values[GRID_SIZE];

    for (int j = 0; j < GRID_SIZE; j++) {
        int block_ = GRID_SIZE * 4;
        int aa = 9 + GRID_SIZE * GRID_SIZE * 2 + GRID_SIZE * 0 + (j * block_) + (block_ * GRID_SIZE * mesh_offset);
        int bb = 9 + GRID_SIZE * GRID_SIZE * 2 + GRID_SIZE * 1 + (j * block_) + (block_ * GRID_SIZE * mesh_offset);
        int cc = 9 + GRID_SIZE * GRID_SIZE * 2 + GRID_SIZE * 2 + (j * block_) + (block_ * GRID_SIZE * mesh_offset);
        int dd = 9 + GRID_SIZE * GRID_SIZE * 2 + GRID_SIZE * 3 + (j * block_) + (block_ * GRID_SIZE * mesh_offset);
        intermediate_values[j] = cubic_spline_interpolate1(aa, bb, cc, dd, GRID_SIZE, x, size_x);
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
float map_coord(float x, float in_min, float in_max, float out_min, float out_max) {
    return (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min;
}

vec2 rotate_and_distort(vec2 pos, float idx) {
    float _x = (float(pos.x) * get_param(idx, 0)) + (float(pos.y) * get_param(idx, 1)) + get_param(idx, 2) + params.translation3d.x;
    float _y = (float(pos.x) * get_param(idx, 3)) + (float(pos.y) * get_param(idx, 4)) + get_param(idx, 5) + params.translation3d.y;
    float _w = (float(pos.x) * get_param(idx, 6)) + (float(pos.y) * get_param(idx, 7)) + get_param(idx, 8) + params.translation3d.z;

    if (_w > 0.0) {
        if (params.r_limit > 0.0 && length(vec2(_x, _y) / _w) > params.r_limit) {
            return vec2(-99999.0, -99999.0);
        }

        if (params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0) {
            float r = length(vec2(_x, _y)) / _w;
            float sin_theta_d = (r / sqrt(1.0 + r * r)) * params.light_refraction_coefficient;
            float r_d = sin_theta_d / sqrt(1.0 - sin_theta_d * sin_theta_d);
            if (r_d != 0.0) {
                _w *= r / r_d;
            }
        }

        vec2 uv = params.f * distort_point(_x, _y, _w);

        if (get_param(idx, 9) != 0.0 || get_param(idx, 10) != 0.0 || get_param(idx, 11) != 0.0 || get_param(idx, 12) != 0.0 || get_param(idx, 13) != 0.0) {
            float ang_rad = get_param(idx, 11);
            float cos_a = cos(-ang_rad);
            float sin_a = sin(-ang_rad);
            uv = vec2(
                cos_a * uv.x - sin_a * uv.y - get_param(idx, 9)  + get_param(idx, 12),
                sin_a * uv.x + cos_a * uv.y - get_param(idx, 10) + get_param(idx, 13)
            );
        }

        uv += params.c;

        if (get_mesh_data(0) > 9.0) {
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

        // FocalPlaneDistortion
        if (get_mesh_data(0) > 0.0 && get_mesh_data(int(get_mesh_data(0))) > 0.0) {
            int o = int(get_mesh_data(0)); // offset to focal plane distortion data

            vec2 mesh_size = vec2(get_mesh_data(3), get_mesh_data(4));
            vec2 origin    = vec2(get_mesh_data(5), get_mesh_data(6));
            vec2 crop_size = vec2(get_mesh_data(7), get_mesh_data(8));
            float stblz_grid = mesh_size.y / 8.0;

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

        if (bool(params.flags & 2)) { // Has digital lens
            uv = digital_distort_point(uv);
        }

        if (params.input_horizontal_stretch > 0.001) { uv.x /= params.input_horizontal_stretch; }
        if (params.input_vertical_stretch   > 0.001) { uv.y /= params.input_vertical_stretch; }

        return uv;
    }
    return vec2(-99999.0, -99999.0);
}

void main() {
    vec2 texPos = v_texcoord.xy * vec2(params.output_width, params.output_height) + params.translation2d;
    vec2 outPos = v_texcoord.xy * vec2(params.output_width, params.output_height);

    if (bool(params.flags & 4)) { // Fill with background
        fragColor = params.background;
        return;
    }

    ///////////////////////////////////////////////////////////////////
    // Add lens distortion back
    if (params.lens_correction_amount < 1.0) {
        float factor = max(1.0 - params.lens_correction_amount, 0.001); // FIXME: this is close but wrong
        vec2 out_c = vec2(params.output_width / 2.0, params.output_height / 2.0);
        vec2 out_f = (params.f / params.fov) / factor;

        vec2 new_out_pos = texPos;

        if (bool(params.flags & 2)) { // Has digital lens
            new_out_pos = digital_undistort_point(new_out_pos);
        }

        new_out_pos = (new_out_pos - out_c) / out_f;
        new_out_pos = undistort_point(new_out_pos);
        if (params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0) {
            float r = length(new_out_pos);
            if (r != 0.0) {
                float sin_theta_d = (r / sqrt(1.0 + r * r)) / params.light_refraction_coefficient;
                float r_d = sin_theta_d / sqrt(1.0 - sin_theta_d * sin_theta_d);
                new_out_pos *= r_d / r;
            }
        }
        new_out_pos = out_f * new_out_pos + out_c;

        texPos = new_out_pos * (1.0 - params.lens_correction_amount) + (texPos * params.lens_correction_amount);
    }
    ///////////////////////////////////////////////////////////////////

    ///////////////////////////////////////////////////////////////////
    // Calculate source `y` for rolling shutter
    float sy = texPos.y;
    if (bool(params.flags & 16)) { // Horizontal RS
        sy = texPos.x;
    }
    if (params.matrix_count > 1) {
        float idx = params.matrix_count / 2.0; // Use middle matrix
        vec2 uv = rotate_and_distort(texPos, idx);
        if (uv.x > -99998.0) {
            if (bool(params.flags & 16)) { // Horizontal RS
                sy = min(params.width, max(0, floor(0.5 + uv.x)));
            } else {
                sy = min(params.height, max(0, floor(0.5 + uv.y)));
            }
        }
    }
    ///////////////////////////////////////////////////////////////////

    float idx = min(sy, params.matrix_count - 1.0);

    vec2 uv = rotate_and_distort(texPos, idx);
    if (uv.x > -99998.0) {
        if (params.background_mode == 1) { // edge repeat
            uv = max(vec2(0, 0), min(vec2(params.width - 1, params.height - 1), uv));
        } else if (params.background_mode == 2) { // edge mirror
            float width3 = (params.width - 2);
            float height3 = (params.height - 2);
            if (uv.x > width3)  uv.x = width3  - (uv.x - width3);
            if (uv.x < 2)       uv.x = 2 + params.width - (width3  + uv.x);
            if (uv.y > height3) uv.y = height3 - (uv.y - height3);
            if (uv.y < 2)       uv.y = 2 + params.height - (height3 + uv.y);
        } else if (params.background_mode == 3) { // margin with feather
            float widthf  = (params.width  - 1);
            float heightf = (params.height - 1);

            float feather = max(0.0001, params.background_margin_feather * heightf);
            vec2 pt2 = uv;
            float alpha = 1.0;
            if ((uv.x > widthf - feather) || (uv.x < feather) || (uv.y > heightf - feather) || (uv.y < feather)) {
                alpha = max(0.0, min(1.0, min(min(widthf - uv.x, heightf - uv.y), min(uv.x, uv.y)) / feather));
                pt2 /= vec2(widthf, heightf);
                pt2 = ((pt2 - 0.5) * (1.0 - params.background_margin)) + 0.5;
                pt2 *= vec2(widthf, heightf);
            }

            vec4 c1 = texture(texIn, vec2(uv.x / params.width, uv.y / params.height));
            vec4 c2 = texture(texIn, vec2(pt2.x / params.width, pt2.y / params.height));
            fragColor = c1 * alpha + c2 * (1.0 - alpha);
            fragColor.a = 1.0;
            if (!((pt2.x >= 0 && pt2.x < params.width) && (pt2.y >= 0 && pt2.y < params.height))) {
                fragColor = params.background;
            }
            draw_pixel(fragColor, uv.x, uv.y, true);
            draw_pixel(fragColor, outPos.x, outPos.y, false);
            draw_safe_area(fragColor, outPos.x, outPos.y);
            return;
        }

        if ((uv.x >= 0 && uv.x < params.width) && (uv.y >= 0 && uv.y < params.height)) {
            fragColor = texture(texIn, vec2(uv.x / params.width, uv.y / params.height));
            draw_pixel(fragColor, uv.x, uv.y, true);
            draw_pixel(fragColor, outPos.x, outPos.y, false);
            draw_safe_area(fragColor, outPos.x, outPos.y);
            return;
        }
    }

    fragColor = params.background;
    draw_pixel(fragColor, outPos.x, outPos.y, false);
    draw_safe_area(fragColor, outPos.x, outPos.y);
}
