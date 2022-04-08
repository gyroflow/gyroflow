// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

// Adapted from OpenCV: initUndistortRectifyMap
// https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/fisheye.cpp#L454

#version 420

layout(location = 0) in vec2 v_texcoord;
layout(location = 0) out vec4 fragColor;

layout(binding = 1) uniform sampler2D texIn;

layout(std140, binding = 2) uniform UniformBuffer {
    int params_count;
    int width;
    int height;
    int output_width;
    int output_height;
    int _padding;
    int _padding2;
    int _padding3;
    vec4 bg;
} uniforms;

layout(binding = 3) uniform sampler2D texParams;

float get_param(float row, float idx) {
    return texture(texParams, vec2(idx / 8.0, row / (float(uniforms.height + 2) - 3.0))).r;
}

vec2 undistort_point(vec2 pos, vec2 f, vec2 c, vec4 k, float amount) {
    pos = (pos - c) / f;

    float theta_d = min(max(length(pos), -1.5707963267948966), 1.5707963267948966); // PI/2

    bool converged = false;
    float theta = theta_d;

    float scale = 0.0;

    if (abs(theta_d) > 1e-6) {
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
            float theta_fix = (theta * (1.0 + k0_theta2 + k1_theta4 + k2_theta6 + k3_theta8) - theta_d)
                              /
                              (1.0 + 3.0 * k0_theta2 + 5.0 * k1_theta4 + 7.0 * k2_theta6 + 9.0 * k3_theta8);

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
        // Apply only requested amount
        scale = 1.0 + (scale - 1.0) * (1.0 - amount);

        return f * pos * scale + c;
    }
    return vec2(0.0, 0.0);
}

vec2 distort_point(vec2 pos, vec2 f, vec2 c, vec4 k) {
    float r = length(pos);

    float theta = atan(r);
    float theta2 = theta*theta, 
          theta4 = theta2*theta2, 
          theta6 = theta4*theta2, 
          theta8 = theta4*theta4;

    float theta_d = theta * (1.0 + dot(k, vec4(theta2, theta4, theta6, theta8)));

    float scale = r == 0? 1.0 : theta_d / r;
    return f * pos * scale + c;
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
        return distort_point(pos, f, c, k);
    }
    return vec2(-99999.0, -99999.0);
}

void main() {
    vec2 texPos = v_texcoord.xy * vec2(uniforms.output_width, uniforms.output_height);

    vec2 f = vec2(get_param(0, 0), get_param(0, 1));
    vec2 c = vec2(get_param(0, 2), get_param(0, 3));
    vec4 k = vec4(get_param(0, 4), get_param(0, 5), get_param(0, 6), get_param(0, 7));
    float r_limit = get_param(0, 8);
    float lens_correction_amount = get_param(1, 0);
    float background_mode = get_param(1, 1);
    float fov = get_param(1, 2);
    float input_horizontal_stretch = get_param(1, 3);
    float input_vertical_stretch = get_param(1, 4);
    bool edge_repeat = background_mode > 0.9 && background_mode < 1.1; // 1
    bool edge_mirror = background_mode > 1.9 && background_mode < 2.1; // 2

    ///////////////////////////////////////////////////////////////////
    // Calculate source `y` for rolling shutter
    float sy = texPos.y;
    if (uniforms.params_count > 3) {
        float idx = 2.0 + ((uniforms.params_count - 2.0) / 2.0); // Use middle matrix
        vec2 uv = rotate_and_distort(texPos, idx, f, c, k, r_limit);
        if (uv.x > -99998.0) {
            sy = min(uniforms.height, max(0, floor(0.5 + uv.y)));
        }
    }
    ///////////////////////////////////////////////////////////////////

    if (lens_correction_amount < 1.0) {
        // Add lens distortion back
        float factor = max(1.0 - lens_correction_amount, 0.001); // FIXME: this is close but wrong
        vec2 out_c = vec2(uniforms.output_width / 2.0, uniforms.output_height / 2.0);
        texPos = undistort_point(texPos, (f / fov) / factor, out_c, k, lens_correction_amount);
    }

    float idx = min(sy + 2.0, uniforms.params_count - 1.0);

    vec2 uv = rotate_and_distort(texPos, idx, f, c, k, r_limit);
    if (input_horizontal_stretch > 0.001) { uv.x /= input_horizontal_stretch; }
    if (input_vertical_stretch   > 0.001) { uv.y /= input_vertical_stretch; }

    if (uv.x > -99998.0) {
        if (edge_repeat) {
            uv = max(vec2(0, 0), min(vec2(uniforms.width - 1, uniforms.height - 1), uv));
        } else if (edge_mirror) {
            float width3 = (uniforms.width - 2);
            float height3 = (uniforms.height - 2);
            if (uv.x > width3)  uv.x = width3  - (uv.x - width3);
            if (uv.x < 2)       uv.x = 2 + uniforms.width - (width3  + uv.x);
            if (uv.y > height3) uv.y = height3 - (uv.y - height3);
            if (uv.y < 2)       uv.y = 2 + uniforms.height - (height3 + uv.y);
        } else if (false) {
            // margin with feather mode, looks good but not trivial to implement in OpenCL
            float width3 = (uniforms.width - 2);
            float height3 = (uniforms.height - 2);

            float margin = 100;
            float feather = 50;
            vec2 pt2 = uv;
            float alpha = 1.0;
            if (uv.x > width3 - margin)  { alpha = width3 - uv.x;  pt2 = vec2(uv.x - margin, uv.y); }
            if (uv.x < margin)           { alpha = uv.x;           pt2 = vec2(uv.x + margin, uv.y); }
            if (uv.y > height3 - margin) { alpha = height3 - uv.y; pt2 = vec2(uv.x, uv.y - margin); }
            if (uv.y < margin)           { alpha = uv.y;           pt2 = vec2(uv.x, uv.y + margin); }

            vec4 c1 = texture(texIn, vec2(uv.x / uniforms.width, uv.y / uniforms.height));
            vec4 c2 = texture(texIn, vec2(pt2.x / uniforms.width, pt2.y / uniforms.height));
            alpha = feather > 0.0? max(0, min(1, alpha / feather)) : 1.0;
            fragColor = c1 * alpha + c2 * (1.0 - alpha);
            fragColor.a = 1.0;
            return;
        }

        if ((uv.x >= 0 && uv.x < uniforms.width) && (uv.y >= 0 && uv.y < uniforms.height)) {
            fragColor = texture(texIn, vec2(uv.x / uniforms.width, uv.y / uniforms.height));
            return;
        }
    }
    fragColor = uniforms.bg;
}
