// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

// Adapted from OpenCV: initUndistortRectifyMap
// https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/calib3d/src/fisheye.cpp#L465-L567

// #version 420

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
    vec4 k;             // 16 - distortion coefficients
    float fov;          // 4
    float r_limit;      // 8
    float lens_correction_amount;   // 12
    float input_vertical_stretch;   // 16
    float input_horizontal_stretch; // 4
    float background_margin;        // 8
    float background_margin_feather;// 12
    float reserved3;                // 16
} params;

layout(binding = 3) uniform sampler2D texParams;

float get_param(float row, float idx) {
    return texture(texParams, vec2(idx / 8.0, row / float(params.matrix_count - 1))).r;
}

vec2 rotate_and_distort(vec2 pos, float idx, vec2 f, vec2 c, vec4 k, float r_limit) {
    float _x = (float(pos.y) * get_param(idx, 1)) + get_param(idx, 2) + (float(pos.x) * get_param(idx, 0));
    float _y = (float(pos.y) * get_param(idx, 4)) + get_param(idx, 5) + (float(pos.x) * get_param(idx, 3));
    float _w = (float(pos.y) * get_param(idx, 7)) + get_param(idx, 8) + (float(pos.x) * get_param(idx, 6));

    if (_w > 0) {
        vec2 pos = vec2(_x, _y) / _w;
        float r = length(pos);
        if (r_limit > 0.0 && r > r_limit) {
            return vec2(-99999.0, -99999.0);
        }
        return f * distort_point(pos, k) + c;
    }
    return vec2(-99999.0, -99999.0);
}

void main() {
    vec2 texPos = v_texcoord.xy * vec2(params.output_width, params.output_height);

    ///////////////////////////////////////////////////////////////////
    // Calculate source `y` for rolling shutter
    float sy = texPos.y;
    if (params.matrix_count > 1) {
        float idx = params.matrix_count / 2.0; // Use middle matrix
        vec2 uv = rotate_and_distort(texPos, idx, params.f, params.c, params.k, params.r_limit);
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
        
        texPos = (texPos - out_c) / out_f;
        texPos = undistort_point(texPos, params.k, params.lens_correction_amount);
        texPos = out_f * texPos + out_c;
    }
    ///////////////////////////////////////////////////////////////////

    float idx = min(sy + 2.0, params.matrix_count - 1.0);

    vec2 uv = rotate_and_distort(texPos, idx, params.f, params.c, params.k, params.r_limit);
    if (params.input_horizontal_stretch > 0.001) { uv.x /= params.input_horizontal_stretch; }
    if (params.input_vertical_stretch   > 0.001) { uv.y /= params.input_vertical_stretch; }

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
            return;
        }

        if ((uv.x >= 0 && uv.x < params.width) && (uv.y >= 0 && uv.y < params.height)) {
            fragColor = texture(texIn, vec2(uv.x / params.width, uv.y / params.height));
            return;
        }
    }
    fragColor = params.background / 255.0;
}
