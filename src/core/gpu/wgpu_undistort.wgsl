// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

struct Globals {
    width: u32;
    height: u32;
    output_width: u32;
    output_height: u32;
    params_count: u32;
    interpolation: u32;
    background: array<f32, 4>;
};

@group(0) @binding(0) @stage(fragment) var<uniform> params: Globals;
@group(0) @binding(1) @stage(fragment) var<storage, read> undistortion_params: array<f32>;
@group(0) @binding(2) @stage(fragment) var input: texture_2d<SCALAR>;
@group(0) @binding(3) @stage(fragment) var<storage, read> coeffs: array<f32>;

let INTER_BITS: u32 = 5u;
let INTER_TAB_SIZE: i32 = 32; // (1u << INTER_BITS);

fn interpolate(sx: i32, sy: i32, sx0: i32, sy0: i32, width_u: i32, height_u: i32) -> vec4<SCALAR> {
    let bg = vec4<f32>(params.background[0], params.background[1], params.background[2], params.background[3]);
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
                    pixel = vec4<f32>(textureLoad(input, vec2<i32>(sx + xp, sy + yp), 0));
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

@stage(vertex)
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
@stage(fragment)
fn undistort_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<SCALAR> {
    let gx = i32(position.x);
    let gy = i32(position.y);

    let width = params.width;
    let height = params.height;
    let params_count = params.params_count;
    let bg = vec4<SCALAR>(SCALAR(params.background[0]), SCALAR(params.background[1]), SCALAR(params.background[2]), SCALAR(params.background[3]));

    let x: f32 = f32(gx);
    let y: f32 = f32(gy);

    let width_u = i32(width);
    let height_u = i32(height);

    let f = vec2<f32>(undistortion_params[0], undistortion_params[1]);
    let c = vec2<f32>(undistortion_params[2], undistortion_params[3]);
    let k = vec4<f32>(undistortion_params[4], undistortion_params[5], undistortion_params[6], undistortion_params[7]);
    let r_limit = undistortion_params[8];

    ///////////////////////////////////////////////////////////////////
    // Calculate source `y` for rolling shutter
    var sy = u32(gy);
    if (params_count > 2u) {
        let params_idx: u32 = (params_count / 2u) * 9u; // Use middle matrix
        let x_y_ = vec2<f32>(y * undistortion_params[params_idx + 1u] + undistortion_params[params_idx + 2u] + (x * undistortion_params[params_idx + 0u]),
                             y * undistortion_params[params_idx + 4u] + undistortion_params[params_idx + 5u] + (x * undistortion_params[params_idx + 3u]));
        let w_ = y * undistortion_params[params_idx + 7u] + undistortion_params[params_idx + 8u] + (x * undistortion_params[params_idx + 6u]);
        if (w_ > 0.0) {
            let pos = x_y_ / w_;
            let r = length(pos);
            let theta = atan(r);
            let theta2 = theta*theta; let theta4 = theta2*theta2; let theta6 = theta4*theta2; let theta8 = theta4*theta4;
            let theta_d = theta * (1.0 + dot(k, vec4<f32>(theta2, theta4, theta6, theta8)));            
            var scale: f32 = 1.0;
            if (r != 0.0) {
                scale = theta_d / r;
            }
            let uv = f * pos * scale + c;
            sy = u32(min(height_u, max(0, i32(floor(0.5 + uv.y * f32(INTER_TAB_SIZE))) >> INTER_BITS)));
        }
    }
    ///////////////////////////////////////////////////////////////////
 
    let params_idx: u32 = min((sy + 1u), (params_count - 1u)) * 9u;
 
    let x_y_ = vec2<f32>(y * undistortion_params[params_idx + 1u] + undistortion_params[params_idx + 2u] + (x * undistortion_params[params_idx + 0u]),
                         y * undistortion_params[params_idx + 4u] + undistortion_params[params_idx + 5u] + (x * undistortion_params[params_idx + 3u]));
    let w_ = y * undistortion_params[params_idx + 7u] + undistortion_params[params_idx + 8u] + (x * undistortion_params[params_idx + 6u]);
 
    if (w_ > 0.0) {
        let pos = x_y_ / w_;
        let r = length(pos);

        if (r_limit > 0.0 && r > r_limit) {
            return bg;
        }

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

        var offsets: array<f32, 3> = array<f32, 3>(0.0, 1.0, 3.0);
        let offset = offsets[params.interpolation >> 2u];

        let uv = f * pos * scale + c - offset;

        let sx0 = i32(round(uv.x * f32(INTER_TAB_SIZE)));
        let sy0 = i32(round(uv.y * f32(INTER_TAB_SIZE)));

        let sx = i32(sx0 >> INTER_BITS);
        let sy = i32(sy0 >> INTER_BITS);

        return interpolate(sx, sy, sx0, sy0, width_u, height_u);
    }
    return bg;
}
