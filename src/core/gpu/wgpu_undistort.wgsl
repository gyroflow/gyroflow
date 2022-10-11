// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

struct KernelParams {
    width:             i32, // 4
    height:            i32, // 8
    stride:            i32, // 12
    output_width:      i32, // 16
    output_height:     i32, // 4
    output_stride:     i32, // 8
    matrix_count:      i32, // 12 - for rolling shutter correction. 1 = no correction, only main matrix
    interpolation:     i32, // 16
    background_mode:   i32, // 4
    flags:             i32, // 8
    bytes_per_pixel:   i32, // 12
    pix_element_count: i32, // 16
    background:    vec4<f32>, // 16
    f:             vec2<f32>, // 8 - focal length in pixels
    c:             vec2<f32>, // 16 - lens center
    k1: vec4<f32>, k2: vec4<f32>, k3: vec4<f32>, // 16,16,16 - distortion coefficients
    fov:           f32, // 4
    r_limit:       f32, // 8
    lens_correction_amount:   f32, // 12
    input_vertical_stretch:   f32, // 16
    input_horizontal_stretch: f32, // 4
    background_margin:        f32, // 8
    background_margin_feather:f32, // 12
    canvas_scale:             f32, // 16
    reserved2:                f32, // 4
    reserved3:                f32, // 8
    translation2d:      vec2<f32>, // 16
    translation3d:      vec4<f32>, // 16
    source_rect:        vec4<i32>, // 16 - x, y, w, h
    output_rect:        vec4<i32>, // 16 - x, y, w, h
    digital_lens_params:vec4<f32>, // 16
}

@group(0) @binding(0) @fragment var<uniform> params: KernelParams;
@group(0) @binding(1) @fragment var<storage, read> matrices: array<f32>;
@group(0) @binding(2) @fragment var input_tex: texture_2d<SCALAR>;
@group(0) @binding(3) @fragment var<storage, read> coeffs: array<f32>;
@group(0) @binding(4) @fragment var<storage, read> drawing: array<u32>;

LENS_MODEL_FUNCTIONS;

let INTER_BITS: u32 = 5u;
let INTER_TAB_SIZE: i32 = 32; // (1u << INTER_BITS);

fn draw_pixel(in_pix: vec4<f32>, x: u32, y: u32, isInput: bool) -> vec4<f32> {
    if (!bool(params.flags & 8)) { // Drawing not enabled
        return in_pix;
    }

    let width = max(params.width, params.output_width);

    let pos_byte = u32(round(floor(f32(y) / params.canvas_scale) * (f32(width) / params.canvas_scale) + floor(f32(x) / params.canvas_scale)));
    let pos_u32 = pos_byte / 4u;
    let u32_offset = pos_byte - (pos_u32 * 4u);
    let data = (drawing[pos_u32] >> ((u32_offset) * 8u)) & 0xFFu;
    var pix = in_pix;
    if (data > 0u) {
        let color = (data & 0xF8u) >> 3u;
        let alpha = (data & 0x06u) >> 1u;
        let stage = data & 1u;
        if (((stage == 0u && isInput) || (stage == 1u && !isInput)) && color < 9u) {
            let color_offs = 448u + (color * 4u);
            let colorf = vec4<f32>(coeffs[color_offs], coeffs[color_offs + 1u], coeffs[color_offs + 2u], coeffs[color_offs + 3u]) / bg_scaler;
            let alphaf = coeffs[484u + alpha];
            pix = colorf * alphaf + pix * (1.0 - alphaf);
            pix.w = 255.0 / bg_scaler;
        }
    }
    return pix;
}

// From 0-255(JPEG/Full) to 16-235(MPEG/Limited)
fn remap_colorrange(px: vec4<f32>, isY: bool) -> vec4<f32> {
    if (isY) { return (16.0 / bg_scaler) + (px * 0.85882352); } // (235 - 16) / 255
    else     { return (16.0 / bg_scaler) + (px * 0.87843137); } // (240 - 16) / 255
}
fn map_coord(x: f32, in_min: f32, in_max: f32, out_min: f32, out_max: f32) -> f32 {
    return (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min;
}

fn sample_input_at(uv: vec2<f32>) -> vec4<f32> {
    let fix_range = bool(params.flags & 1);

    let bg = params.background / bg_scaler;
    var sum = vec4<f32>(0.0);

    let shift = (params.interpolation >> 2u) + 1u;
    var indices: array<i32, 3> = array<i32, 3>(0, 64, 192);
    let ind = indices[params.interpolation >> 2u];
    var offsets: array<f32, 3> = array<f32, 3>(0.0, 1.0, 3.0);
    let offset = offsets[params.interpolation >> 2u];

    var uv = vec2<f32>(
        map_coord(uv.x, 0.0, f32(params.width),  f32(params.source_rect.x), f32(params.source_rect.x + params.source_rect.z)),
        map_coord(uv.y, 0.0, f32(params.height), f32(params.source_rect.y), f32(params.source_rect.y + params.source_rect.w))
    );

    uv = uv - offset;

    let sx0 = i32(round(uv.x * f32(INTER_TAB_SIZE)));
    let sy0 = i32(round(uv.y * f32(INTER_TAB_SIZE)));

    let sx = i32(sx0 >> INTER_BITS);
    let sy = i32(sy0 >> INTER_BITS);

    let coeffs_x = i32(ind + ((sx0 & (INTER_TAB_SIZE - 1)) << shift));
    let coeffs_y = i32(ind + ((sy0 & (INTER_TAB_SIZE - 1)) << shift));

    for (var yp: i32 = 0; yp < i32(params.interpolation); yp = yp + 1) {
        if (sy + yp >= params.source_rect.y && sy + yp < params.source_rect.y + params.source_rect.w) {
            var xsum = vec4<f32>(0.0, 0.0, 0.0, 0.0);
            for (var xp: i32 = 0; xp < i32(params.interpolation); xp = xp + 1) {
                var pixel: vec4<f32>;
                if (sx + xp >= params.source_rect.x && sx + xp < params.source_rect.x + params.source_rect.z) {
                    pixel = vec4<f32>(textureLoad(input_tex, vec2<i32>(sx + xp, sy + yp), 0));
                    pixel = draw_pixel(pixel, u32(sx + xp), u32(sy + yp), true);
                    if (fix_range) {
                        pixel = remap_colorrange(pixel, params.bytes_per_pixel == 1);
                    }
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

    return sum;
}

fn rotate_and_distort(pos: vec2<f32>, idx: u32, f: vec2<f32>, c: vec2<f32>, k1: vec4<f32>, k2: vec4<f32>, k3: vec4<f32>) -> vec2<f32> {
    let _x = (pos.x * matrices[idx + 0u]) + (pos.y * matrices[idx + 1u]) + matrices[idx + 2u] + params.translation3d.x;
    let _y = (pos.x * matrices[idx + 3u]) + (pos.y * matrices[idx + 4u]) + matrices[idx + 5u] + params.translation3d.y;
    let _w = (pos.x * matrices[idx + 6u]) + (pos.y * matrices[idx + 7u]) + matrices[idx + 8u] + params.translation3d.z;

    if (_w > 0.0) {
        let pos = vec2<f32>(_x, _y) / _w;
        let r = length(pos);
        if (params.r_limit > 0.0 && r > params.r_limit) {
            return vec2<f32>(-99999.0, -99999.0);
        }
        var uv = f * distort_point(pos) + c;

        if (bool(params.flags & 2)) { // Has digital lens
            uv = digital_distort_point(uv);
        }

        if (params.input_horizontal_stretch > 0.001) { uv.x /= params.input_horizontal_stretch; }
        if (params.input_vertical_stretch   > 0.001) { uv.y /= params.input_vertical_stretch; }

        return uv;
    }
    return vec2<f32>(-99999.0, -99999.0);
}

@vertex
fn undistort_vertex(@builtin(vertex_index) in_vertex_index: u32) -> @builtin(position) @invariant vec4<f32> {
    var positions: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>( 1.0, -1.0), vec2<f32>( 1.0,  1.0),
        vec2<f32>( 1.0,  1.0), vec2<f32>(-1.0,  1.0), vec2<f32>(-1.0, -1.0),
    );
    return vec4<f32>(positions[in_vertex_index], 0.0, 1.0);
}

// Adapted from OpenCV: initUndistortRectifyMap + remap
// https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/calib3d/src/fisheye.cpp#L465-L567
// https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/imgproc/src/opencl/remap.cl#L390-L498
@fragment
fn undistort_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<SCALAR> {
    let bg = vec4<f32>(params.background.x / bg_scaler, params.background.y / bg_scaler, params.background.z / bg_scaler, params.background.w / bg_scaler);

    if (bool(params.flags & 4)) { // Fill with background
        return vec4<SCALAR>(bg);
    }

    var out_pos = vec2<f32>(
        map_coord(position.x, f32(params.output_rect.x), f32(params.output_rect.x + params.output_rect.z), 0.0, f32(params.output_width) ),
        map_coord(position.y, f32(params.output_rect.y), f32(params.output_rect.y + params.output_rect.w), 0.0, f32(params.output_height))
    );

    let p = out_pos;
    out_pos = out_pos + params.translation2d;

    if (out_pos.x < 0.0 || out_pos.y < 0.0 || out_pos.x > f32(params.output_width) || out_pos.y > f32(params.output_height)) { return vec4<SCALAR>(bg); }

    ///////////////////////////////////////////////////////////////////
    // Calculate source `y` for rolling shutter
    var sy = u32(position.y);
    if (params.matrix_count > 1) {
        let idx: u32 = u32((params.matrix_count / 2) * 9); // Use middle matrix
        let uv = rotate_and_distort(out_pos, idx, params.f, params.c, params.k1, params.k2, params.k3);
        if (uv.x > -99998.0) {
            sy = u32(min(params.height, max(0, i32(floor(0.5 + uv.y)))));
        }
    }
    ///////////////////////////////////////////////////////////////////

    ///////////////////////////////////////////////////////////////////
    // Add lens distortion back
    if (params.lens_correction_amount < 1.0) {
        let factor = max(1.0 - params.lens_correction_amount, 0.001); // FIXME: this is close but wrong
        let out_c = vec2<f32>(f32(params.output_width) / 2.0, f32(params.output_height) / 2.0);
        let out_f = (params.f / params.fov) / factor;

        var new_out_pos = out_pos;

        if (bool(params.flags & 2)) { // Has digital lens
            new_out_pos = digital_undistort_point(new_out_pos);
        }

        new_out_pos = (new_out_pos - out_c) / out_f;
        new_out_pos = undistort_point(new_out_pos);
        new_out_pos = out_f * new_out_pos + out_c;

        out_pos = new_out_pos * (1.0 - params.lens_correction_amount) + (out_pos * params.lens_correction_amount);
    }
    ///////////////////////////////////////////////////////////////////

    let idx: u32 = min(sy, u32(params.matrix_count - 1)) * 9u;

    var pixel: vec4<f32> = bg;

    var uv = rotate_and_distort(out_pos, idx, params.f, params.c, params.k1, params.k2, params.k3);
    if (uv.x > -99998.0) {
        let width_f = f32(params.width);
        let height_f = f32(params.height);
        if (params.background_mode == 1) { // edge repeat
            uv = max(vec2<f32>(0.0, 0.0), min(vec2<f32>(width_f - 1.0, height_f - 1.0), uv));
        } else if (params.background_mode == 2) { // edge mirror
            let rx = round(uv.x);
            let ry = round(uv.y);
            let width3 = (width_f - 3.0);
            let height3 = (height_f - 3.0);
            if (rx > width3)  { uv.x = width3  - (rx - width3); }
            if (rx < 3.0)     { uv.x = 3.0 + width_f - (width3 + rx); }
            if (ry > height3) { uv.y = height3 - (ry - height3); }
            if (ry < 3.0)     { uv.y = 3.0 + height_f - (height3 + ry); }
        } else if (params.background_mode == 3) { // margin with feather
            let widthf  = (width_f - 1.0);
            let heightf = (height_f - 1.0);

            let feather = max(0.0001, params.background_margin_feather * heightf);
            var pt2 = uv;
            var alpha = 1.0;
            if ((uv.x > widthf - feather) || (uv.x < feather) || (uv.y > heightf - feather) || (uv.y < feather)) {
                alpha = max(0.0, min(1.0, min(min(widthf - uv.x, heightf - uv.y), min(uv.x, uv.y)) / feather));
                pt2 /= vec2<f32>(width_f, height_f);
                pt2 = ((pt2 - 0.5) * (1.0 - params.background_margin)) + 0.5;
                pt2 *= vec2<f32>(width_f, height_f);
            }

            let c1 = sample_input_at(uv);
            let c2 = sample_input_at(pt2);
            pixel = c1 * alpha + c2 * (1.0 - alpha);
            pixel = draw_pixel(pixel, u32(p.x), u32(p.y), false);
            return vec4<SCALAR>(pixel);
        }

        pixel = sample_input_at(uv);
    }
    pixel = draw_pixel(pixel, u32(p.x), u32(p.y), false);
    return vec4<SCALAR>(pixel);
}
