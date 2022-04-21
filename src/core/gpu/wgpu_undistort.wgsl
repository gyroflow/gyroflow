// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

struct Globals {
    width: u32,
    height: u32,
    output_width: u32,
    output_height: u32,
    params_count: u32,
    interpolation: u32,
    background0: f32,
    background1: f32,
    background2: f32,
    background3: f32,
};

@group(0) @binding(0) @fragment var<uniform> params: Globals;
@group(0) @binding(1) @fragment var<storage, read> undistortion_params: array<f32>;
@group(0) @binding(2) @fragment var input_tex: texture_2d<SCALAR>;
@group(0) @binding(3) @fragment var<storage, read> coeffs: array<f32>;

let INTER_BITS: u32 = 5u;
let INTER_TAB_SIZE: i32 = 32; // (1u << INTER_BITS);

fn interpolate(sx: i32, sy: i32, sx0: i32, sy0: i32, width_u: i32, height_u: i32) -> vec4<SCALAR> {
    let bg = vec4<f32>(params.background0, params.background1, params.background2, params.background3);
    var sum = vec4<f32>(0.0);
    
    let shift = (params.interpolation >> 2u) + 1u;
    var indices: array<i32, 3> = array<i32, 3>(0, 64, 192);
    let ind = indices[params.interpolation >> 2u];
    
    let coeffs_x = i32(ind + ((sx0 & (INTER_TAB_SIZE - 1)) << shift));
    let coeffs_y = i32(ind + ((sy0 & (INTER_TAB_SIZE - 1)) << shift));

    for (var yp: i32 = 0; yp < i32(params.interpolation); yp = yp + 1) {
        if (sy + yp >= 0 && sy + yp < height_u) {
            var xsum = vec4<f32>(0.0, 0.0, 0.0, 0.0);
            for (var xp: i32 = 0; xp < i32(params.interpolation); xp = xp + 1) {
                var pixel: vec4<f32>;
                if (sx + xp >= 0 && sx + xp < width_u) {
                    pixel = vec4<f32>(textureLoad(input_tex, vec2<i32>(sx + xp, sy + yp), 0));
                } else {
                    pixel = bg;
                }
                xsum = xsum + (pixel * coeffs[coeffs_x + xp]);
            }
            sum = sum + xsum * coeffs[coeffs_y + yp];
        } else {
            sum = sum + bg * coeffs[coeffs_y + yp];
        }
    }
    
    return vec4<SCALAR>(sum);
}

fn undistort_point(pos: vec2<f32>, f: vec2<f32>, c: vec2<f32>, k: vec4<f32>, amount: f32) -> vec2<f32> {
    let pos = (pos - c) / f;

    let theta_d = min(max(length(pos), -1.5707963267948966), 1.5707963267948966); // PI/2

    var converged = false;
    var theta = theta_d;

    var scale = 0.0;

    if (abs(theta_d) > 1e-6) {
        for (var i: i32 = 0; i < 10; i = i + 1) {
            let theta2 = theta*theta;
            let theta4 = theta2*theta2;
            let theta6 = theta4*theta2;
            let theta8 = theta6*theta2;
            let k0_theta2 = k.x * theta2;
            let k1_theta4 = k.y * theta4;
            let k2_theta6 = k.z * theta6;
            let k3_theta8 = k.w * theta8;
            // new_theta = theta - theta_fix, theta_fix = f0(theta) / f0'(theta)
            let theta_fix = (theta * (1.0 + k0_theta2 + k1_theta4 + k2_theta6 + k3_theta8) - theta_d)
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
    let theta_flipped = (theta_d < 0.0 && theta > 0.0) || (theta_d > 0.0 && theta < 0.0);

    if (converged && !theta_flipped) {
        // Apply only requested amount
        scale = 1.0 + (scale - 1.0) * (1.0 - amount);

        return f * pos * scale + c;
    }
    return vec2<f32>(0.0, 0.0);
}

fn distort_point(pos: vec2<f32>, f: vec2<f32>, c: vec2<f32>, k: vec4<f32>) -> vec2<f32> {
    let r = length(pos);

    let theta = atan(r);
    let theta2 = theta*theta;
    let theta4 = theta2*theta2;
    let theta6 = theta4*theta2;
    let theta8 = theta4*theta4;

    let theta_d = theta * (1.0 + dot(k, vec4<f32>(theta2, theta4, theta6, theta8)));

    var scale: f32 = 1.0;
    if (r != 0.0) {
        scale = theta_d / r;
    }
    return f * pos * scale + c;
}

fn rotate_and_distort(pos: vec2<f32>, idx: u32, f: vec2<f32>, c: vec2<f32>, k: vec4<f32>, r_limit: f32) -> vec2<f32> {
    let _x = (pos.y * undistortion_params[idx + 1u]) + undistortion_params[idx + 2u] + (pos.x * undistortion_params[idx + 0u]);
    let _y = (pos.y * undistortion_params[idx + 4u]) + undistortion_params[idx + 5u] + (pos.x * undistortion_params[idx + 3u]);
    let _w = (pos.y * undistortion_params[idx + 7u]) + undistortion_params[idx + 8u] + (pos.x * undistortion_params[idx + 6u]);

    if (_w > 0.0) {
        let pos = vec2<f32>(_x, _y) / _w;
        let r = length(pos);
        if (r_limit > 0.0 && r > r_limit) {
            return vec2<f32>(-99999.0, -99999.0);
        }
        return distort_point(pos, f, c, k);
    }
    return vec2<f32>(-99999.0, -99999.0);
}


@vertex
fn undistort_vertex(@builtin(vertex_index) in_vertex_index: u32) -> @builtin(position) vec4<f32> {
    var positions: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>( 1.0, -1.0), vec2<f32>( 1.0,  1.0),
        vec2<f32>( 1.0,  1.0), vec2<f32>(-1.0,  1.0), vec2<f32>(-1.0, -1.0),
    );
    return vec4<f32>(positions[in_vertex_index], 0.0, 1.0);
}

// Adapted from OpenCV: initUndistortRectifyMap + remap 
// https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/fisheye.cpp#L454
// https://github.com/opencv/opencv/blob/4.x/modules/imgproc/src/opencl/remap.cl#L390
@fragment
fn undistort_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<SCALAR> {
    let gx = i32(position.x);
    let gy = i32(position.y);

    let width = params.width;
    let height = params.height;
    let params_count = params.params_count;
    let bg = vec4<SCALAR>(SCALAR(params.background0), SCALAR(params.background1), SCALAR(params.background2), SCALAR(params.background3));

    var texPos = vec2<f32>(f32(gx), f32(gy));

    let width_u = i32(width);
    let height_u = i32(height);

    let f = vec2<f32>(undistortion_params[0], undistortion_params[1]);
    let c = vec2<f32>(undistortion_params[2], undistortion_params[3]);
    let k = vec4<f32>(undistortion_params[4], undistortion_params[5], undistortion_params[6], undistortion_params[7]);
    let r_limit = undistortion_params[8];
    let lens_correction_amount = undistortion_params[9];
    let background_mode = undistortion_params[10];
    let fov = undistortion_params[11];
    let input_horizontal_stretch = undistortion_params[12];
    let input_vertical_stretch = undistortion_params[13];
    let edge_repeat = background_mode > 0.9 && background_mode < 1.1; // 1
    let edge_mirror = background_mode > 1.9 && background_mode < 2.1; // 2

    ///////////////////////////////////////////////////////////////////
    // Calculate source `y` for rolling shutter
    var sy = u32(gy);
    if (params_count > 3u) {
        let idx: u32 = 2u + ((params_count - 2u) / 2u) * 9u; // Use middle matrix
        let uv = rotate_and_distort(texPos, idx, f, c, k, r_limit);
        if (uv.x > -99998.0) {
            sy = u32(min(height_u, max(0, i32(floor(0.5 + uv.y)))));
        }
    }
    ///////////////////////////////////////////////////////////////////
 
    if (lens_correction_amount < 1.0) {
        // Add lens distortion back
        let factor = max(1.0 - lens_correction_amount, 0.001); // FIXME: this is close but wrong
        let out_c = vec2<f32>(f32(params.output_width) / 2.0, f32(params.output_height) / 2.0);
        texPos = undistort_point(texPos, (f / fov) / factor, out_c, k, lens_correction_amount);
    }

    let idx: u32 = min((sy + 2u), (params_count - 1u)) * 9u;
 
    var uv = rotate_and_distort(texPos, idx, f, c, k, r_limit);
    if (input_horizontal_stretch > 0.001) { uv.x /= input_horizontal_stretch; }
    if (input_vertical_stretch   > 0.001) { uv.y /= input_vertical_stretch; }

    if (uv.x > -99998.0) {
        let width_f = f32(width);
        let height_f = f32(height);
        if (edge_repeat) {
            uv = max(vec2<f32>(0.0, 0.0), min(vec2<f32>(width_f - 1.0, height_f - 1.0), uv));
        } else if (edge_mirror) {
            let rx = round(uv.x);
            let ry = round(uv.y);
            let width3 = (width_f - 3.0);
            let height3 = (height_f - 3.0);
            if (rx > width3)  { uv.x = width3  - (rx - width3); }
            if (rx < 3.0)     { uv.x = 3.0 + width_f - (width3 + rx); }
            if (ry > height3) { uv.y = height3 - (ry - height3); }
            if (ry < 3.0)     { uv.y = 3.0 + height_f - (height3 + ry); }
        }

        let sx0 = i32(round(uv.x * f32(INTER_TAB_SIZE)));
        let sy0 = i32(round(uv.y * f32(INTER_TAB_SIZE)));

        let sx = i32(sx0 >> INTER_BITS);
        let sy = i32(sy0 >> INTER_BITS);

        return interpolate(sx, sy, sx0, sy0, width_u, height_u);
    }
    return bg;
}
