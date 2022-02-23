// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

struct PixelsData {
    data: [[stride(4)]] array<u32>;
};

[[group(0), binding(0)]]
var<storage, read> pixels: PixelsData;

struct UndistortionData {
    data: [[stride(4)]] array<f32>;
};
[[group(0), binding(1)]]
var<storage, read> undistortion_params: UndistortionData;

[[group(0), binding(2)]]
var<storage, read_write> pixels_out: PixelsData;

let INTER_BITS: u32 = 5u;
let INTER_TAB_SIZE: i32 = 32; // (1u << INTER_BITS);

struct Locals {
    width: u32;
    height: u32;
    stride: u32;
    output_width: u32;
    output_height: u32;
    output_stride: u32;
    bytes_per_pixel: u32;
    pix_element_count: u32;
    params_count: u32;
    interpolation: u32;
    background: array<f32, 4>;
};
[[group(0), binding(3)]]
var<uniform> params: Locals;

fn get_pixel(pos: u32) -> vec4<f32> {
    let px: u32 = pixels.data[pos / params.bytes_per_pixel];
    return vec4<f32>(
        f32(px & 0xffu),
        f32((px & 0xff00u) >> 8u),
        f32((px & 0xff0000u) >> 16u),
        f32((px & 0xff000000u) >> 24u),
    );
}
fn put_pixel(pos: u32, px: vec4<f32>) {
    pixels_out.data[pos / params.bytes_per_pixel] = u32(
        (u32(px[0]) << 0u) |
        (u32(px[1]) << 8u) |
        (u32(px[2]) << 16u) |
        (u32(px[3]) << 24u) 
    );
}

fn interpolate(src_index1: i32, width_u: i32, height_u: i32, sx: i32, sy: i32, sx0: i32, sy0: i32) -> vec4<f32> {
    let bg = vec4<f32>(params.background[0], params.background[1], params.background[2], params.background[3]);
    var sum = vec4<f32>(0.0);
    var src_index: i32 = src_index1;
    var COEFFS: array<f32, 448> = array<f32, 448>(
        // Bilinear
        1.000000, 0.000000, 0.968750, 0.031250, 0.937500, 0.062500, 0.906250, 0.093750, 0.875000, 0.125000, 0.843750, 0.156250,
        0.812500, 0.187500, 0.781250, 0.218750, 0.750000, 0.250000, 0.718750, 0.281250, 0.687500, 0.312500, 0.656250, 0.343750,
        0.625000, 0.375000, 0.593750, 0.406250, 0.562500, 0.437500, 0.531250, 0.468750, 0.500000, 0.500000, 0.468750, 0.531250,
        0.437500, 0.562500, 0.406250, 0.593750, 0.375000, 0.625000, 0.343750, 0.656250, 0.312500, 0.687500, 0.281250, 0.718750,
        0.250000, 0.750000, 0.218750, 0.781250, 0.187500, 0.812500, 0.156250, 0.843750, 0.125000, 0.875000, 0.093750, 0.906250,
        0.062500, 0.937500, 0.031250, 0.968750,

        // Bicubic
         0.000000, 1.000000, 0.000000,  0.000000, -0.021996, 0.997841, 0.024864, -0.000710, -0.041199, 0.991516, 0.052429, -0.002747,
        -0.057747, 0.981255, 0.082466, -0.005974, -0.071777, 0.967285, 0.114746, -0.010254, -0.083427, 0.949837, 0.149040, -0.015450,
        -0.092834, 0.929138, 0.185120, -0.021423, -0.100136, 0.905418, 0.222755, -0.028038, -0.105469, 0.878906, 0.261719, -0.035156,
        -0.108971, 0.849831, 0.301781, -0.042641, -0.110779, 0.818420, 0.342712, -0.050354, -0.111031, 0.784904, 0.384285, -0.058159,
        -0.109863, 0.749512, 0.426270, -0.065918, -0.107414, 0.712471, 0.468437, -0.073494, -0.103821, 0.674011, 0.510559, -0.080750,
        -0.099220, 0.634361, 0.552406, -0.087547, -0.093750, 0.593750, 0.593750, -0.093750, -0.087547, 0.552406, 0.634361, -0.099220,
        -0.080750, 0.510559, 0.674011, -0.103821, -0.073494, 0.468437, 0.712471, -0.107414, -0.065918, 0.426270, 0.749512, -0.109863,
        -0.058159, 0.384285, 0.784904, -0.111031, -0.050354, 0.342712, 0.818420, -0.110779, -0.042641, 0.301781, 0.849831, -0.108971,
        -0.035156, 0.261719, 0.878906, -0.105469, -0.028038, 0.222755, 0.905418, -0.100136, -0.021423, 0.185120, 0.929138, -0.092834,
        -0.015450, 0.149040, 0.949837, -0.083427, -0.010254, 0.114746, 0.967285, -0.071777, -0.005974, 0.082466, 0.981255, -0.057747,
        -0.002747, 0.052429, 0.991516, -0.041199, -0.000710, 0.024864, 0.997841, -0.021996,

        // Lanczos4
         0.000000,  0.000000,  0.000000,  1.000000,  0.000000,  0.000000,  0.000000,  0.000000, -0.002981,  0.009625, -0.027053,  0.998265, 
         0.029187, -0.010246,  0.003264, -0.000062, -0.005661,  0.018562, -0.051889,  0.993077,  0.060407, -0.021035,  0.006789, -0.000250, 
        -0.008027,  0.026758, -0.074449,  0.984478,  0.093543, -0.032281,  0.010545, -0.000567, -0.010071,  0.034167, -0.094690,  0.972534, 
         0.128459, -0.043886,  0.014499, -0.001012, -0.011792,  0.040757, -0.112589,  0.957333,  0.165004, -0.055744,  0.018613, -0.001582, 
        -0.013191,  0.046507, -0.128145,  0.938985,  0.203012, -0.067742,  0.022845, -0.002271, -0.014275,  0.051405, -0.141372,  0.917621, 
         0.242303, -0.079757,  0.027146, -0.003071, -0.015054,  0.055449, -0.152304,  0.893389,  0.282684, -0.091661,  0.031468, -0.003971, 
        -0.015544,  0.058648, -0.160990,  0.866453,  0.323952, -0.103318,  0.035754, -0.004956, -0.015761,  0.061020, -0.167496,  0.836995, 
         0.365895, -0.114591,  0.039949, -0.006011, -0.015727,  0.062590, -0.171900,  0.805208,  0.408290, -0.125335,  0.043992, -0.007117, 
        -0.015463,  0.063390, -0.174295,  0.771299,  0.450908, -0.135406,  0.047823, -0.008254, -0.014995,  0.063460, -0.174786,  0.735484, 
         0.493515, -0.144657,  0.051378, -0.009399, -0.014349,  0.062844, -0.173485,  0.697987,  0.535873, -0.152938,  0.054595, -0.010527, 
        -0.013551,  0.061594, -0.170517,  0.659039,  0.577742, -0.160105,  0.057411, -0.011613, -0.012630,  0.059764, -0.166011,  0.618877, 
         0.618877, -0.166011,  0.059764, -0.012630, -0.011613,  0.057411, -0.160105,  0.577742,  0.659039, -0.170517,  0.061594, -0.013551, 
        -0.010527,  0.054595, -0.152938,  0.535873,  0.697987, -0.173485,  0.062844, -0.014349, -0.009399,  0.051378, -0.144657,  0.493515, 
         0.735484, -0.174786,  0.063460, -0.014995, -0.008254,  0.047823, -0.135406,  0.450908,  0.771299, -0.174295,  0.063390, -0.015463, 
        -0.007117,  0.043992, -0.125336,  0.408290,  0.805208, -0.171900,  0.062590, -0.015727, -0.006011,  0.039949, -0.114591,  0.365895, 
         0.836995, -0.167496,  0.061020, -0.015761, -0.004956,  0.035754, -0.103318,  0.323952,  0.866453, -0.160990,  0.058648, -0.015544, 
        -0.003971,  0.031468, -0.091661,  0.282684,  0.893389, -0.152304,  0.055449, -0.015054, -0.003071,  0.027146, -0.079757,  0.242303, 
         0.917621, -0.141372,  0.051405, -0.014275, -0.002271,  0.022845, -0.067742,  0.203012,  0.938985, -0.128145,  0.046507, -0.013191, 
        -0.001582,  0.018613, -0.055744,  0.165004,  0.957333, -0.112589,  0.040757, -0.011792, -0.001012,  0.014499, -0.043886,  0.128459, 
         0.972534, -0.094690,  0.034167, -0.010071, -0.000567,  0.010545, -0.032281,  0.093543,  0.984478, -0.074449,  0.026758, -0.008027, 
        -0.000250,  0.006789, -0.021035,  0.060407,  0.993077, -0.051889,  0.018562, -0.005661, -0.000062,  0.003264, -0.010246,  0.029187, 
         0.998265, -0.027053,  0.009625, -0.002981
    );
    
    // TODO: Bicubic and Lanczos are not working

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
                    pixel = get_pixel(u32(src_index + (xp * i32(params.bytes_per_pixel))));
                } else {
                    pixel = bg;
                }
                xsum = xsum + (pixel * COEFFS[coeffs_x + xp]);
            }
            sum = sum + xsum * COEFFS[coeffs_y + yp];
        } else {
            sum = sum + bg * COEFFS[coeffs_y + yp];
        }
        src_index = src_index + i32(params.stride);
    }
    
    return sum;
}

// Adapted from OpenCV: initUndistortRectifyMap + remap 
// https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/fisheye.cpp#L454
// https://github.com/opencv/opencv/blob/4.x/modules/imgproc/src/opencl/remap.cl#L390
[[stage(compute), workgroup_size(8, 8)]]
fn undistort([[builtin(global_invocation_id)]] global_id: vec3<u32>) {
    let width = params.width;
    let height = params.height;
    let params_count = params.params_count;
    let bg = vec4<f32>(params.background[0], params.background[1], params.background[2], params.background[3]);

    let x: f32 = f32(global_id.x);
    let y: f32 = f32(global_id.y);

    let width_u = i32(width);
    let height_u = i32(height);

    if (global_id.x < params.output_width && global_id.y < params.output_height) {
        let f = vec2<f32>(undistortion_params.data[0], undistortion_params.data[1]);
        let c = vec2<f32>(undistortion_params.data[2], undistortion_params.data[3]);
        let k = vec4<f32>(undistortion_params.data[4], undistortion_params.data[5], undistortion_params.data[6], undistortion_params.data[7]);
        let r_limit = undistortion_params.data[8];

        ///////////////////////////////////////////////////////////////////
        // Calculate source `y` for rolling shutter
        var sy = global_id.y;
        if (params_count > 2u) {
            let params_idx: u32 = (params_count / 2u) * 9u; // Use middle matrix
            let x_y_ = vec2<f32>(y * undistortion_params.data[params_idx + 1u] + undistortion_params.data[params_idx + 2u] + (x * undistortion_params.data[params_idx + 0u]),
                                 y * undistortion_params.data[params_idx + 4u] + undistortion_params.data[params_idx + 5u] + (x * undistortion_params.data[params_idx + 3u]));
            let w_ = y * undistortion_params.data[params_idx + 7u] + undistortion_params.data[params_idx + 8u] + (x * undistortion_params.data[params_idx + 6u]);
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

        let x_y_ = vec2<f32>(y * undistortion_params.data[params_idx + 1u] + undistortion_params.data[params_idx + 2u] + (x * undistortion_params.data[params_idx + 0u]),
                             y * undistortion_params.data[params_idx + 4u] + undistortion_params.data[params_idx + 5u] + (x * undistortion_params.data[params_idx + 3u]));
        let w_ = y * undistortion_params.data[params_idx + 7u] + undistortion_params.data[params_idx + 8u] + (x * undistortion_params.data[params_idx + 6u]);
        
        let dst_index = global_id.x * params.bytes_per_pixel + global_id.y * params.output_stride;

        if (w_ > 0.0) {
            let pos = x_y_ / w_;
            let r = length(pos);
        
            if (r_limit > 0.0 && r > r_limit) {
                put_pixel(dst_index, bg);
                return;
            }
            
            let theta = atan(r);
            let theta2 = theta*theta;
            let theta4 = theta2*theta2;
            let theta6 = theta4*theta2;
            let theta8 = theta4*theta4;

            let theta_d = theta * (1.0 + dot(k, vec4<f32>(theta2, theta4, theta6, theta8)));
            //let theta_d = theta * (1.0 + k[0]*theta2 + k[1]*theta4 + k[2]*theta6 + k[3]*theta8);
        
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
        
            let src_index = sy * i32(params.stride) + sx * i32(params.bytes_per_pixel);

            let sum = interpolate(src_index, width_u, height_u, sx, sy, sx0, sy0);

            put_pixel(dst_index, sum);
        } else {
            put_pixel(dst_index, bg);
        }
    }
}

