// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

// Adapted from OpenCV: initUndistortRectifyMap
// https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/fisheye.cpp#L454

#version 420

layout(location = 0) in vec2 v_texcoord;
layout(location = 0) out vec4 fragColor;

layout(binding = 1) uniform sampler2D texIn;

layout(std140, binding = 2) uniform UniformBuffer {
    uint params_count;
    uint width;
    uint height;
    uint output_width;
    uint output_height;
    uint _padding;
    uint _padding2;
    uint _padding3;
    vec4 bg;
} uniforms;

layout(binding = 3) uniform sampler2D texParams;

float get_param(int row, int idx) {
    return texture(texParams, vec2(idx / float(8), (row / float(uniforms.height - 2)))).r;
}

void main() {
    ivec2 texPos = ivec2(v_texcoord.xy * vec2(uniforms.output_width, uniforms.output_height));

    vec2 f = vec2(get_param(0, 0), get_param(0, 1));
    vec2 c = vec2(get_param(0, 2), get_param(0, 3));
    vec4 k = vec4(get_param(0, 4), get_param(0, 5), get_param(0, 6), get_param(0, 7));
    float r_limit = get_param(0, 8);

    ///////////////////////////////////////////////////////////////////
    // Calculate source `y` for rolling shutter
    int sy = texPos.y;
    if (uniforms.params_count > 2) {
        int idx = 1 + int(uniforms.params_count / 2); // Use middle matrix
        float _x = (float(texPos.y) * get_param(idx, 1)) + get_param(idx, 2) + (float(texPos.x) * get_param(idx, 0));
        float _y = (float(texPos.y) * get_param(idx, 4)) + get_param(idx, 5) + (float(texPos.x) * get_param(idx, 3));
        float _w = (float(texPos.y) * get_param(idx, 7)) + get_param(idx, 8) + (float(texPos.x) * get_param(idx, 6));
        if (_w > 0) {
            vec2 pos = vec2(_x, _y) / _w;
            float r = length(pos);
            float theta = atan(r);
            float theta2 = theta*theta; float theta4 = theta2*theta2; float theta6 = theta4*theta2; float theta8 = theta4*theta4;
            float theta_d = theta * (1.0 + dot(k, vec4(theta2, theta4, theta6, theta8)));
            float scale = r == 0? 1.0 : theta_d / r;
            vec2 uv = f * pos * scale + c;
            sy = int(min(uniforms.height, max(0, floor(0.5 + uv.y))));
        }
    }
    ///////////////////////////////////////////////////////////////////

    int idx = int(min(sy + 1, uniforms.params_count));

    float _x = (float(texPos.y) * get_param(idx, 1)) + get_param(idx, 2) + (float(texPos.x) * get_param(idx, 0));
    float _y = (float(texPos.y) * get_param(idx, 4)) + get_param(idx, 5) + (float(texPos.x) * get_param(idx, 3));
    float _w = (float(texPos.y) * get_param(idx, 7)) + get_param(idx, 8) + (float(texPos.x) * get_param(idx, 6));

    if (_w > 0) {
        vec2 pos = vec2(_x, _y) / _w;

        float r = length(pos);
        
        if (r_limit > 0.0 && r > r_limit) {
            fragColor = uniforms.bg;
            return;
        }

        float theta = atan(r);
        float theta2 = theta*theta;
        float theta4 = theta2*theta2;
        float theta6 = theta4*theta2;
        float theta8 = theta4*theta4;
        float theta_d = theta * (1.0 + dot(k, vec4(theta2, theta4, theta6, theta8)));

        float scale = r == 0? 1.0 : theta_d / r;
        vec2 uv = f * pos * scale + c;
        
        if ((uv.x >= 0 && uv.x < uniforms.width) && (uv.y >= 0 && uv.y < uniforms.height)) {
            fragColor = texture(texIn, vec2(uv.x / uniforms.width, uv.y / uniforms.height));
            return;
        }
    }
    fragColor = uniforms.bg;
}
