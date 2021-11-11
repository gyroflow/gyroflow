[[block]]
struct PixelsData {
    data: [[stride(4)]] array<u32>;
};

[[group(0), binding(0)]]
var<storage, read_write> pixels: PixelsData;

[[block]]
struct UndistortionData {
    data: [[stride(4)]] array<f32>;
};
[[group(0), binding(1)]]
var<storage, read> undistortion_params: UndistortionData;

[[group(0), binding(2)]]
var<storage, read_write> pixels_out: PixelsData;

let INTER_BITS: u32 = 5u;
let INTER_TAB_SIZE: i32 = 32; // (1u << INTER_BITS);

fn get_pixel(pos: u32) -> vec4<f32> {
    let px: u32 = pixels.data[pos];
    return vec4<f32>(
        f32(px & 0xffu),
        f32((px & 0xff00u) >> 8u),
        f32((px & 0xff0000u) >> 16u),
        f32((px & 0xff000000u) >> 24u),
    );
}
fn put_pixel(pos: u32, px: vec4<f32>) {
    pixels_out.data[pos] = u32(
        (u32(px[0]) << 0u) |
        (u32(px[1]) << 8u) |
        (u32(px[2]) << 16u) |
        (u32(px[3]) << 24u) 
    );
}

[[block]]
struct Locals {
    width: f32;
    height: f32;
    params_count: u32;
    background: array<f32, 4>;
};
[[group(0), binding(3)]]
var<uniform> params: Locals;

[[stage(compute), workgroup_size(8, 8)]]
fn undistort([[builtin(global_invocation_id)]] global_id: vec3<u32>) {
    let width = params.width;
    let height = params.height;
    let params_count = params.params_count;
    let bg = vec4<f32>(params.background[0], params.background[1], params.background[2], params.background[3]);

    var COEFFS: array<f32, 64> = array<f32, 64>(
        1.000000, 0.000000, 0.968750, 0.031250, 0.937500, 0.062500, 0.906250, 0.093750, 0.875000, 0.125000, 0.843750, 0.156250,
        0.812500, 0.187500, 0.781250, 0.218750, 0.750000, 0.250000, 0.718750, 0.281250, 0.687500, 0.312500, 0.656250, 0.343750,
        0.625000, 0.375000, 0.593750, 0.406250, 0.562500, 0.437500, 0.531250, 0.468750, 0.500000, 0.500000, 0.468750, 0.531250,
        0.437500, 0.562500, 0.406250, 0.593750, 0.375000, 0.625000, 0.343750, 0.656250, 0.312500, 0.687500, 0.281250, 0.718750,
        0.250000, 0.750000, 0.218750, 0.781250, 0.187500, 0.812500, 0.156250, 0.843750, 0.125000, 0.875000, 0.093750, 0.906250,
        0.062500, 0.937500, 0.031250, 0.968750
    );

    let x: f32 = f32(global_id.x);
    let y: f32 = f32(global_id.y);

    let width_u = i32(width);
    let height_u = i32(height);

    if (x < width && y < height) {
        let f = vec2<f32>(undistortion_params.data[0], undistortion_params.data[1]);
        let c = vec2<f32>(undistortion_params.data[2], undistortion_params.data[3]);
        let k = vec4<f32>(undistortion_params.data[4], undistortion_params.data[5], undistortion_params.data[6], undistortion_params.data[7]);

        let params: u32 = min((global_id.y + 1u), (params_count - 1u)) * 9u;

        let x_y_ = vec2<f32>(y * undistortion_params.data[params + 1u] + undistortion_params.data[params + 2u] + (x * undistortion_params.data[params + 0u]),
                             y * undistortion_params.data[params + 4u] + undistortion_params.data[params + 5u] + (x * undistortion_params.data[params + 3u]));
        let w_ = y * undistortion_params.data[params + 7u] + undistortion_params.data[params + 8u] + (x * undistortion_params.data[params + 6u]);
        
        if (w_ > 0.0) {
            let pos = x_y_ / w_;
        
            let r = length(pos);

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
            let uv = f * pos * scale + c;
        
            let sx = i32(floor(0.5 + uv.x * f32(INTER_TAB_SIZE))) >> INTER_BITS;
            let sy = i32(floor(0.5 + uv.y * f32(INTER_TAB_SIZE))) >> INTER_BITS;
        
            let coeffs_x = i32(i32(round(uv.x * f32(INTER_TAB_SIZE))) & (INTER_TAB_SIZE - 1)) << 1u;
            let coeffs_y = i32(i32(round(uv.y * f32(INTER_TAB_SIZE))) & (INTER_TAB_SIZE - 1)) << 1u;
        
            var sum = vec4<f32>(0.0);
            var src_index = sy * width_u + sx;
        
            for (var yp: i32 = 0; yp < 2; yp = yp + 1) {
                if (sy + yp >= 0 && sy + yp < height_u) {
                    var xsum = vec4<f32>(0.0);
                    for (var xp: i32 = 0; xp < 2; xp = xp + 1) {
                        if (sx + xp >= 0 && sx + xp < width_u) {
                            xsum = xsum + get_pixel(u32(src_index)) * COEFFS[coeffs_x + xp];
                        } else {
                            xsum = xsum + bg * COEFFS[coeffs_x + xp];
                        }
                    }

                    sum = sum + xsum * COEFFS[coeffs_y + yp];
                } else {
                    sum = sum + bg * COEFFS[coeffs_y + yp];
                }

                src_index = src_index + width_u;
            }
            put_pixel(global_id.x + global_id.y * u32(width_u), sum);
        } else {
            put_pixel(global_id.x + global_id.y * u32(width_u), bg);
        }
    }
}
