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
    float reserved2;                // 4
    float reserved3;                // 8
    vec2 translation2d;             // 16
    vec4 translation3d;             // 16
    ivec2 source_pos;               // 8
    ivec2 output_pos;               // 16
    vec2 source_stretch;            // 8
    vec2 output_stretch;            // 16
    vec4 digital_lens_params;       // 16
} params;

LENS_MODEL_FUNCTIONS;

layout(binding = 3) uniform sampler2D texParams;
layout(binding = 4) uniform sampler2D texCanvas;

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

float get_param(float row, float idx) {
    return texture(texParams, vec2(idx / 8.0, row / float(params.height - 1))).r;
}

vec2 rotate_and_distort(vec2 pos, float idx) {
    float _x = (float(pos.x) * get_param(idx, 0)) + (float(pos.y) * get_param(idx, 1)) + get_param(idx, 2) + params.translation3d.x;
    float _y = (float(pos.x) * get_param(idx, 3)) + (float(pos.y) * get_param(idx, 4)) + get_param(idx, 5) + params.translation3d.y;
    float _w = (float(pos.x) * get_param(idx, 6)) + (float(pos.y) * get_param(idx, 7)) + get_param(idx, 8) + params.translation3d.z;

    if (_w > 0) {
        vec2 pos = vec2(_x, _y) / _w;
        float r = length(pos);
        if (params.r_limit > 0.0 && r > params.r_limit) {
            return vec2(-99999.0, -99999.0);
        }
        vec2 uv = params.f * distort_point(pos) + params.c;

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
        fragColor = params.background / 255.0;
        return;
    }

    ///////////////////////////////////////////////////////////////////
    // Calculate source `y` for rolling shutter
    float sy = texPos.y;
    if (params.matrix_count > 1) {
        float idx = params.matrix_count / 2.0; // Use middle matrix
        vec2 uv = rotate_and_distort(texPos, idx);
        if (uv.x > -99998.0) {
            sy = min(params.height, max(0, floor(0.5 + uv.y)));
        }
    }
    ///////////////////////////////////////////////////////////////////

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
        new_out_pos = out_f * new_out_pos + out_c;

        texPos = new_out_pos * (1.0 - params.lens_correction_amount) + (texPos * params.lens_correction_amount);
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
                fragColor = params.background / 255.0;
            }
            draw_pixel(fragColor, uv.x, uv.y, true);
            draw_pixel(fragColor, outPos.x, outPos.y, false);
            return;
        }

        if ((uv.x >= 0 && uv.x < params.width) && (uv.y >= 0 && uv.y < params.height)) {
            fragColor = texture(texIn, vec2(uv.x / params.width, uv.y / params.height));
            draw_pixel(fragColor, uv.x, uv.y, true);
            draw_pixel(fragColor, outPos.x, outPos.y, false);
            return;
        }
    }

    fragColor = params.background / 255.0;
    draw_pixel(fragColor, outPos.x, outPos.y, false);
}
