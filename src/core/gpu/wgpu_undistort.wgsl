// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

@id(100) override interpolation:      u32 = 8u;
@id(101) override pix_element_count:  i32 = 0;
@id(102) override bytes_per_pixel:    i32 = 0;
@id(103) override flags:              i32 = 0;

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
    input_rotation:           f32, // 4
    output_rotation:          f32, // 8
    translation2d:      vec2<f32>, // 16
    translation3d:      vec4<f32>, // 16
    source_rect:        vec4<i32>, // 16 - x, y, w, h
    output_rect:        vec4<i32>, // 16 - x, y, w, h
    digital_lens_params:vec4<f32>, // 16
    safe_area_rect:     vec4<f32>, // 16
    max_pixel_value:          f32, // 4
    distortion_model:         i32, // 8
    digital_lens:             i32, // 12
    pixel_value_limit:        f32, // 16
    light_refraction_coefficient: f32, // 4
    plane_index:              i32, // 8
    reserved1:                f32, // 12
    reserved2:                f32, // 16
    ewa_coeffs_p:             vec4<f32>, // 16
    ewa_coeffs_q:             vec4<f32>, // 16
}

@group(0) @binding(0) @fragment var<uniform> params: KernelParams;
@group(0) @binding(1) @fragment var<storage, read> matrices: array<f32>;
@group(0) @binding(2) @fragment var<storage, read> coeffs: array<f32>;
@group(0) @binding(3) @fragment var<storage, read> mesh_data: array<f32>;
@group(0) @binding(4) @fragment var<storage, read> drawing: array<u32>;
// {texture_input}
@group(0) @binding(5) @fragment var input_texture: texture_2d<SCALAR>;
// {/texture_input}
// {buffer_input}
@group(0) @binding(5) @fragment var<storage, read> input_buffer: array<SCALAR>;
@group(0) @binding(6) @fragment var<storage, read_write> output_buffer: array<SCALAR>;
// {/buffer_input}

LENS_MODEL_FUNCTIONS;

const INTER_BITS: u32 = 5u;
const INTER_TAB_SIZE: i32 = 32; // (1u << INTER_BITS);

fn draw_pixel(in_pix: vec4<f32>, x: u32, y: u32, isInput: bool) -> vec4<f32> {
    if (!bool(flags & 8)) { // Drawing not enabled
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
            let colorf = vec4<f32>(coeffs[color_offs], coeffs[color_offs + 1u], coeffs[color_offs + 2u], coeffs[color_offs + 3u]) * params.max_pixel_value;
            let alphaf = coeffs[484u + alpha];
            pix = colorf * alphaf + pix * (1.0 - alphaf);
            pix.w = colorf.w;
        }
    }
    return pix;
}
fn draw_safe_area(in_pix: vec4<f32>, x: f32, y: f32) -> vec4<f32> {
    var pix = in_pix;
    let isSafeArea = x >= params.safe_area_rect.x && x <= params.safe_area_rect.z &&
                     y >= params.safe_area_rect.y && y <= params.safe_area_rect.w;
    if (!isSafeArea) {
        pix.x *= 0.5;
        pix.y *= 0.5;
        pix.z *= 0.5;
        let isBorder = x > params.safe_area_rect.x - 5.0 && x < params.safe_area_rect.z + 5.0 &&
                       y > params.safe_area_rect.y - 5.0 && y < params.safe_area_rect.w + 5.0;
        if (isBorder) {
            pix.x *= 0.5;
            pix.y *= 0.5;
            pix.z *= 0.5;
        }
    }
    return pix;
}

// From 0-255(JPEG/Full) to 16-235(MPEG/Limited)
fn remap_colorrange(px: vec4<f32>, isY: bool) -> vec4<f32> {
    if (isY) { return ((16.0 / 255.0) * params.max_pixel_value) + (px * 0.85882352); } // (235 - 16) / 255
    else     { return ((16.0 / 255.0) * params.max_pixel_value) + (px * 0.87843137); } // (240 - 16) / 255
}
fn map_coord(x: f32, in_min: f32, in_max: f32, out_min: f32, out_max: f32) -> f32 {
    return (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min;
}

fn read_input_at(uv: vec2<i32>) -> vec4<f32> {
    // {texture_input}
    return vec4<f32>(textureLoad(input_texture, uv, 0));
    // {/texture_input}
    // {buffer_input}
    let stride_px = params.stride / (bytes_per_pixel / pix_element_count);
    let buffer_pos = u32((uv.y * stride_px) + uv.x * pix_element_count);
    return vec4<f32>(
        f32(input_buffer[buffer_pos + 0u]),
        f32(input_buffer[buffer_pos + 1u]),
        f32(input_buffer[buffer_pos + 2u]),
        f32(input_buffer[buffer_pos + 3u])
    );
    // {/buffer_input}
}

fn rotate_point(pos: vec2<f32>, angle: f32, origin: vec2<f32>, origin2: vec2<f32>) -> vec2<f32> {
     return vec2<f32>(cos(angle) * (pos.x - origin.x) - sin(angle) * (pos.y - origin.y) + origin2.x,
                      sin(angle) * (pos.x - origin.x) + cos(angle) * (pos.y - origin.y) + origin2.y);
}

////////////////////////////// EWA (Elliptical Weighted Average) CubicBC sampling //////////////////////////////
// Keys Cubic Filter Family https://imagemagick.org/Usage/filter/#robidoux
// https://github.com/ImageMagick/ImageMagick/blob/main/MagickCore/resize.c

// Gives a bounding box in the source image containing pixels that cover a circle of radius 2 completely in both the source and destination images
fn affine_bbox(jac: vec4<f32>) -> vec2<f32> {
    return vec2<f32>(
        2.0 * max(1.0, max(abs(jac.x + jac.y), abs(jac.x - jac.y))),
        2.0 * max(1.0, max(abs(jac.z + jac.w), abs(jac.z - jac.w)))
    );
}
// Computes minimum area ellipse which covers a unit circle in both the source and destination image
fn clamped_ellipse(jac: vec4<f32>) -> vec3<f32> {
    // find ellipse
    let F0 = abs(jac.x * jac.w - jac.y * jac.z);
    let F = max(0.1, F0 * F0);
    let A = (jac.z * jac.z + jac.w * jac.w) / F;
    let B = -2.0 * (jac.x * jac.z + jac.y * jac.w) / F;
    let C = (jac.x * jac.x + jac.y * jac.y) / F;
    // find the angle to rotate ellipse
    let v = vec2<f32>(C - A, -B);
    let lv = length(v);
    var v0 = 1.0;
    //var v1 = 1.0;
    if (lv > 0.01) { v0 = v.x / lv; }
    //if (lv > 0.01) { v1 = v.y / lv; }
    let c = sqrt(max(0.0, 1.0 + v0) / 2.0);
    var s = sqrt(max(1.0 - v0, 0.0) / 2.0);
    // rotate the ellipse to align it with axes
    var A0 = (A * c * c - B * c * s + C * s * s);
    var C0 = (A * s * s + B * c * s + C * c * c);
    let Bt1 = B * (c * c - s * s);
    let Bt2 = 2.0 * (A - C) * c * s;
    var B0 = Bt1 + Bt2;
    let B0v2 = Bt1 - Bt2;
    if (abs(B0) > abs(B0v2)) {
        s = -s;
        B0 = B0v2;
    }
    // clamp A,C
    A0 = min(A0, 1.0);
    C0 = min(C0, 1.0);
    let sn = -s;
    // rotate it back
    return vec3<f32>(
        A0 * c * c - B0 * c * sn + C0 * sn * sn,
        2.0 * A0 * c * sn + B0 * c * c - B0 * sn * sn - 2.0 * C0 * c * sn,
        A0 * sn * sn + B0 * c * sn + C0 * c * c
    );
}
fn bc2(x_param: f32) -> f32 {
    let x = abs(x_param);
    let x2 = x * x;
    let x3 = x2 * x;
    let powers = vec4<f32>(1.0, x, x2, x3);
    if (x < 1.0) {
        return dot(params.ewa_coeffs_p, powers);
    } else if (x < 2.0) {
        return dot(params.ewa_coeffs_q, powers);
    }
    return 0.0;
}
////////////////////////////// EWA (Elliptical Weighted Average) CubicBC sampling //////////////////////////////

fn sample_input_at(uv_param: vec2<f32>, jac: vec4<f32>) -> vec4<f32> {
    var uv = uv_param;
    let fix_range = bool(flags & 1);

    let bg = params.background * params.max_pixel_value;
    var sum = vec4<f32>(0.0);

    if (interpolation > 8u) {
        // find how many pixels we need around that pixel in each direction
        let trans_size = affine_bbox(jac);
        let bounds = vec4<i32>(
            i32(floor(uv.x - trans_size.x)),
            i32(ceil(uv.x + trans_size.x)),
            i32(floor(uv.y - trans_size.y)),
            i32(ceil(uv.y + trans_size.y))
        );
        var sum_div = 0.0;

        // See: Andreas Gustafsson. "Interactive Image Warping", section 3.6 http://www.gson.org/thesis/warping-thesis.pdf
        let abc = clamped_ellipse(jac);
        for (var in_y: i32 = bounds.z; in_y <= bounds.w; in_y = in_y + 1) {
            let in_fy = f32(in_y) - uv.y;
            for (var in_x: i32 = bounds.x; in_x <= bounds.y; in_x = in_x + 1) {
                let in_fx = f32(in_x) - uv.x;
                let dr = in_fx * in_fx * abc.x + in_fx * in_fy * abc.y + in_fy * in_fy * abc.z;
                let k = bc2(sqrt(dr)); // cylindrical filtering
                if (k == 0.0) {
                    continue;
                }
                var pixel: vec4<f32>;
                if (in_y >= params.source_rect.y && in_y < params.source_rect.y + params.source_rect.w && in_x >= params.source_rect.x && in_x < params.source_rect.x + params.source_rect.z) {
                    pixel = read_input_at(vec2<i32>(in_x, in_y));
                    pixel = draw_pixel(pixel, u32(in_x), u32(in_y), true);
                } else {
                    pixel = bg;
                }
                sum += k * pixel;
                sum_div += k;
            }
        }
        sum /= sum_div;
    } else {
        let shift = (interpolation >> 2u) + 1u;
        var indices: array<i32, 6> = array<i32, 6>(0, 64, 192, 0, 0, 0);
        let ind = indices[interpolation >> 2u];
        var offsets: array<f32, 6> = array<f32, 6>(0.0, 1.0, 3.0, 0.0, 0.0, 0.0);
        let offset = offsets[interpolation >> 2u];

        uv = uv - offset;

        let sx0 = i32(round(uv.x * f32(INTER_TAB_SIZE)));
        let sy0 = i32(round(uv.y * f32(INTER_TAB_SIZE)));

        let sx = i32(sx0 >> INTER_BITS);
        let sy = i32(sy0 >> INTER_BITS);

        let coeffs_x = i32(ind + ((sx0 & (INTER_TAB_SIZE - 1)) << shift));
        let coeffs_y = i32(ind + ((sy0 & (INTER_TAB_SIZE - 1)) << shift));

        for (var yp: i32 = 0; yp < i32(interpolation); yp = yp + 1) {
            if (sy + yp >= params.source_rect.y && sy + yp < params.source_rect.y + params.source_rect.w) {
                var xsum = vec4<f32>(0.0, 0.0, 0.0, 0.0);
                for (var xp: i32 = 0; xp < i32(interpolation); xp = xp + 1) {
                    var pixel: vec4<f32>;
                    if (sx + xp >= params.source_rect.x && sx + xp < params.source_rect.x + params.source_rect.z) {
                        pixel = read_input_at(vec2<i32>(sx + xp, sy + yp));
                        pixel = draw_pixel(pixel, u32(sx + xp), u32(sy + yp), true);
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
    }

    if (fix_range) {
        sum = remap_colorrange(sum, params.plane_index == 0);
    }
    return vec4<f32>(
        min(sum.x, params.pixel_value_limit),
        min(sum.y, params.pixel_value_limit),
        min(sum.z, params.pixel_value_limit),
        min(sum.w, params.pixel_value_limit)
    );
}

const GRID_SIZE: i32 = 9;
var<private> a: array<f32, GRID_SIZE>; var<private> b: array<f32, GRID_SIZE>; var<private> c: array<f32, GRID_SIZE>; var<private> d: array<f32, GRID_SIZE>;
var<private> alpha: array<f32, GRID_SIZE>; var<private> mu: array<f32, GRID_SIZE>; var<private> z: array<f32, GRID_SIZE>;

fn cubic_spline_coefficients(mesh: ptr<function, array<f32, GRID_SIZE>>, step: i32, offset: i32, size: f32) {
    const n: i32 = GRID_SIZE;
    let h = size / f32(n - 1);
    let inv_h = 1.0 / h;
    let three_inv_h = 3.0 * inv_h;
    let h_over_3 = h / 3.0;
    let inv_3h = 1.0 / (3.0 * h);
    for (var i = 0; i < n; i++) { a[i] = (*mesh)[(i + offset) * step]; }
    for (var i = 1; i < n - 1; i++) {alpha[i] = three_inv_h * (a[i + 1] - 2.0 * a[i] + a[i - 1]); }

    mu[0] = 0.0;
    z[0] = 0.0;

    for (var i = 1; i < n - 1; i++) {
        mu[i] = 1.0 / (4.0 - mu[i - 1]);
        z[i] = (alpha[i] * inv_h - z[i - 1]) * mu[i];
    }

    c[n - 1] = 0.0;

    for (var j = n - 2; j >= 0; j--) {
        c[j] = z[j] - mu[j] * c[j + 1];
        b[j] = (a[j + 1] - a[j]) * inv_h - h_over_3 * (c[j + 1] + 2.0 * c[j]);
        d[j] = (c[j + 1] - c[j]) * inv_3h;
    }
}

fn cubic_spline_interpolate2(n: i32, x: f32, size: f32) -> f32 {
    let i = u32(max(0.0, min(f32(n - 2), (f32(n - 1) * x / size))));
    let dx = x - size * f32(i) / f32(n - 1);
    return a[i] + b[i] * dx + c[i] * dx * dx + d[i] * dx * dx * dx;
}

fn bivariate_spline_interpolate(size_x: f32, size_y: f32, mesh_offset: i32, n: i32, x: f32, y: f32) -> f32 {
    var intermediate_values: array<f32, GRID_SIZE>;

    let i = i32(max(0.0, min(f32(GRID_SIZE - 2), (f32(GRID_SIZE - 1) * x / size_x))));
    let dx = x - size_x * f32(i) / f32(GRID_SIZE - 1);
    let dx2 = dx * dx;
    let block_ = GRID_SIZE * 4;
    let offs = 9 + GRID_SIZE * GRID_SIZE * 2 + (block_ * GRID_SIZE * mesh_offset) + i;

    for (var j = 0; j < GRID_SIZE; j++) {
        intermediate_values[j] = mesh_data[offs + (GRID_SIZE * 0) + (j * block_)]
                               + mesh_data[offs + (GRID_SIZE * 1) + (j * block_)] * dx
                               + mesh_data[offs + (GRID_SIZE * 2) + (j * block_)] * dx2
                               + mesh_data[offs + (GRID_SIZE * 3) + (j * block_)] * dx2 * dx;
        // cubic_spline_coefficients(mesh[9 + mesh_offset..], 2, (j * GRID_SIZE), size_x);
        // intermediate_values[j] = cubic_spline_interpolate1(aa, bb, cc, dd, GRID_SIZE, x, size_x);
    }

    cubic_spline_coefficients(&intermediate_values, 1, 0, size_y);
    return cubic_spline_interpolate2(GRID_SIZE, y, size_y);
}

fn interpolate_mesh(width: f32, height: f32, pos: vec2<f32>) -> vec2<f32> {
    if (pos.x < 0.0 || pos.x > width || pos.y < 0.0 || pos.y > height) {
        return pos;
    }
    return vec2<f32>(
        bivariate_spline_interpolate(width, height, 0, GRID_SIZE, pos.x, pos.y),
        bivariate_spline_interpolate(width, height, 1, GRID_SIZE, pos.x, pos.y)
    );
}

fn rotate_and_distort(pos: vec2<f32>, idx: u32, f: vec2<f32>, c: vec2<f32>, k1: vec4<f32>, k2: vec4<f32>, k3: vec4<f32>) -> vec2<f32> {
    let _x = (pos.x * matrices[idx + 0u]) + (pos.y * matrices[idx + 1u]) + matrices[idx + 2u] + params.translation3d.x;
    let _y = (pos.x * matrices[idx + 3u]) + (pos.y * matrices[idx + 4u]) + matrices[idx + 5u] + params.translation3d.y;
    var _w = (pos.x * matrices[idx + 6u]) + (pos.y * matrices[idx + 7u]) + matrices[idx + 8u] + params.translation3d.z;

    if (_w > 0.0) {
        if (params.r_limit > 0.0 && length(vec2<f32>(_x, _y) / _w) > params.r_limit) {
            return vec2<f32>(-99999.0, -99999.0);
        }

        if (bool(flags & 2048) && params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0) {
            let r = length(vec2<f32>(_x, _y)) / _w;
            let sin_theta_d = (r / sqrt(1.0 + r * r)) * params.light_refraction_coefficient;
            let r_d = sin_theta_d / sqrt(1.0 - sin_theta_d * sin_theta_d);
            if (r_d != 0.0) {
                _w *= r / r_d;
            }
        }

        var uv = f * distort_point(_x, _y, _w);

        if (bool(flags & 256) && (matrices[idx + 9] != 0.0 || matrices[idx + 10] != 0.0 || matrices[idx + 11] != 0.0 || matrices[idx + 12] != 0.0 || matrices[idx + 13] != 0.0)) {
            let ang_rad = matrices[idx + 11];
            let cos_a = cos(-ang_rad);
            let sin_a = sin(-ang_rad);
            uv = vec2<f32>(
                cos_a * uv.x - sin_a * uv.y - matrices[idx + 9]  + matrices[idx + 12],
                sin_a * uv.x + cos_a * uv.y - matrices[idx + 10] + matrices[idx + 13]
            );
        }

        uv += c;

        if (bool(flags & 512) && mesh_data[0] > 10.0) {
            let mesh_size = vec2<f32>(mesh_data[3], mesh_data[4]);
            let origin    = vec2<f32>(mesh_data[5], mesh_data[6]);
            let crop_size = vec2<f32>(mesh_data[7], mesh_data[8]);

            if (bool(flags & 128)) { uv.y = f32(params.height) - uv.y; } // framebuffer inverted

            uv.x = map_coord(uv.x, 0.0, f32(params.width),  origin.x, origin.x + crop_size.x);
            uv.y = map_coord(uv.y, 0.0, f32(params.height), origin.y, origin.y + crop_size.y);

            uv = interpolate_mesh(mesh_size.x, mesh_size.y, uv);

            uv.x = map_coord(uv.x, origin.x, origin.x + crop_size.x, 0.0, f32(params.width));
            uv.y = map_coord(uv.y, origin.y, origin.y + crop_size.y, 0.0, f32(params.height));

            if (bool(flags & 128)) { uv.y = f32(params.height) - uv.y; } // framebuffer inverted
        }

        // FocalPlaneDistortion
        if (bool(flags & 1024) && mesh_data[0] > 0.0 && mesh_data[u32(mesh_data[0])] > 0.0) {
            let o = u32(mesh_data[0]); // offset to focal plane distortion data

            let mesh_size = vec2<f32>(mesh_data[3], mesh_data[4]);
            let origin    = vec2<f32>(mesh_data[5], mesh_data[6]);
            let crop_size = vec2<f32>(mesh_data[7], mesh_data[8]);
            let stblz_grid = mesh_size.y / 8.0;

            if (bool(flags & 128)) { uv.y = f32(params.height) - uv.y; } // framebuffer inverted

            uv.x = map_coord(uv.x, 0.0, f32(params.width),  origin.x, origin.x + crop_size.x);
            uv.y = map_coord(uv.y, 0.0, f32(params.height), origin.y, origin.y + crop_size.y);

            let idx2 = u32(min(7, max(0, i32(floor(uv.y / stblz_grid)))));
            let delta = uv.y - stblz_grid * f32(idx2);
            uv.x -= mesh_data[o + 4 + idx2 * 2 + 0] * delta;
            uv.y -= mesh_data[o + 4 + idx2 * 2 + 1] * delta;
            for (var j = 0u; j < idx2; j++) {
                uv.x -= mesh_data[o + 4 + j * 2 + 0] * stblz_grid;
                uv.y -= mesh_data[o + 4 + j * 2 + 1] * stblz_grid;
            }

            uv.x = map_coord(uv.x, origin.x, origin.x + crop_size.x, 0.0, f32(params.width));
            uv.y = map_coord(uv.y, origin.y, origin.y + crop_size.y, 0.0, f32(params.height));

            if (bool(flags & 128)) { uv.y = f32(params.height) - uv.y; } // framebuffer inverted
        }
        if (bool(flags & 2)) { // Has digital lens
            uv = digital_distort_point(uv);
        }

        if (params.input_horizontal_stretch > 0.001) { uv.x /= params.input_horizontal_stretch; }
        if (params.input_vertical_stretch   > 0.001) { uv.y /= params.input_vertical_stretch; }

        return uv;
    }
    return vec2<f32>(-99999.0, -99999.0);
}

fn undistort_coord(position: vec2<f32>) -> vec2<f32> {
    var out_pos = position;
    if (bool(flags & 64)) { // Uses output rect
        out_pos = vec2<f32>(
            map_coord(position.x, f32(params.output_rect.x), f32(params.output_rect.x + params.output_rect.z), 0.0, f32(params.output_width) ),
            map_coord(position.y, f32(params.output_rect.y), f32(params.output_rect.y + params.output_rect.w), 0.0, f32(params.output_height))
        );
    }
    out_pos += params.translation2d;

    ///////////////////////////////////////////////////////////////////
    // Add lens distortion back
    if (params.lens_correction_amount < 1.0) {
        let factor = max(1.0 - params.lens_correction_amount, 0.001); // FIXME: this is close but wrong
        let out_c = vec2<f32>(f32(params.output_width) / 2.0, f32(params.output_height) / 2.0);
        let out_f = (params.f / params.fov) / factor;

        var new_out_pos = out_pos;

        if (bool(flags & 2)) { // Has digital lens
            new_out_pos = digital_undistort_point(new_out_pos);
        }

        new_out_pos = (new_out_pos - out_c) / out_f;
        new_out_pos = undistort_point(new_out_pos);
        if (bool(flags & 2048) && params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0) {
            let r = length(new_out_pos);
            if (r != 0.0) {
                let sin_theta_d = (r / sqrt(1.0 + r * r)) / params.light_refraction_coefficient;
                let r_d = sin_theta_d / sqrt(1.0 - sin_theta_d * sin_theta_d);
                new_out_pos *= r_d / r;
            }
        }
        new_out_pos = out_f * new_out_pos + out_c;

        out_pos = new_out_pos * (1.0 - params.lens_correction_amount) + (out_pos * params.lens_correction_amount);
    }
    ///////////////////////////////////////////////////////////////////

    ///////////////////////////////////////////////////////////////////
    // Calculate source `y` for rolling shutter
    var sy = 0u;
    if (bool(flags & 16)) { // Horizontal RS
        sy = u32(min(params.width, max(0, i32(floor(0.5 + out_pos.x)))));
    } else {
        sy = u32(min(params.height, max(0, i32(floor(0.5 + out_pos.y)))));
    }
    if (params.matrix_count > 1) {
        let idx: u32 = u32((params.matrix_count / 2) * 14); // Use middle matrix
        let uv = rotate_and_distort(out_pos, idx, params.f, params.c, params.k1, params.k2, params.k3);
        if (uv.x > -99998.0) {
            if (bool(flags & 16)) { // Horizontal RS
                sy = u32(min(params.width, max(0, i32(floor(0.5 + uv.x)))));
            } else {
                sy = u32(min(params.height, max(0, i32(floor(0.5 + uv.y)))));
            }
        }
    }
    ///////////////////////////////////////////////////////////////////

    let idx: u32 = min(sy, u32(params.matrix_count - 1)) * 14u;
    var uv = rotate_and_distort(out_pos, idx, params.f, params.c, params.k1, params.k2, params.k3);

    var frame_size = vec2<f32>(f32(params.width), f32(params.height));
    if (params.input_rotation != 0.0) {
        let rotation = params.input_rotation * (3.14159265359 / 180.0);
        let size = frame_size;
        frame_size = abs(round(rotate_point(size, rotation, vec2<f32>(0.0, 0.0), vec2<f32>(0.0, 0.0))));
        uv = rotate_point(uv, rotation, size / 2.0, frame_size / 2.0);
    }

    let width_f = f32(params.width);
    let height_f = f32(params.height);
    if (params.background_mode == 1) { // edge repeat
        uv = max(vec2<f32>(3.0, 3.0), min(vec2<f32>(width_f - 3.0, height_f - 3.0), uv));
    } else if (params.background_mode == 2) { // edge mirror
        let rx = round(uv.x);
        let ry = round(uv.y);
        let width3 = (width_f - 3.0);
        let height3 = (height_f - 3.0);
        if (rx > width3)  { uv.x = width3  - (rx - width3); }
        if (rx < 3.0)     { uv.x = 3.0 + width_f - (width3 + rx); }
        if (ry > height3) { uv.y = height3 - (ry - height3); }
        if (ry < 3.0)     { uv.y = 3.0 + height_f - (height3 + ry); }
    }

    if (bool(flags & 32) && params.background_mode != 3) { // Uses source rect
        uv = vec2<f32>(
            map_coord(uv.x, 0.0, f32(frame_size.x), f32(params.source_rect.x), f32(params.source_rect.x + params.source_rect.z)),
            map_coord(uv.y, 0.0, f32(frame_size.y), f32(params.source_rect.y), f32(params.source_rect.y + params.source_rect.w))
        );
    }

    return uv;
}

// Adapted from OpenCV: initUndistortRectifyMap + remap
// https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/calib3d/src/fisheye.cpp#L465-L567
// https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/imgproc/src/opencl/remap.cl#L390-L498
fn undistort(position: vec2<f32>) -> vec4<SCALAR> {
    let bg = vec4<f32>(params.background.x, params.background.y, params.background.z, params.background.w) * params.max_pixel_value;

    if (bool(params.flags & 4)) { // Fill with background
        return vec4<SCALAR>(bg);
    }

    var out_pos = position;
    if (bool(flags & 64)) { // Uses output rect
        out_pos = vec2<f32>(
            map_coord(position.x, f32(params.output_rect.x), f32(params.output_rect.x + params.output_rect.z), 0.0, f32(params.output_width) ),
            map_coord(position.y, f32(params.output_rect.y), f32(params.output_rect.y + params.output_rect.w), 0.0, f32(params.output_height))
        );
    }

    let p = out_pos;

    if (out_pos.x < 0.0 || out_pos.y < 0.0 || out_pos.x > f32(params.output_width) || out_pos.y > f32(params.output_height)) { return vec4<SCALAR>(bg); }

    var uv = undistort_coord(position);
    var jac = vec4<f32>(1.0, 0.0, 0.0, 1.0);

    if (interpolation > 8u) {
        let eps = 0.01;
        let xyx = undistort_coord(position + vec2<f32>(eps, 0.0)) - uv;
        let xyy = undistort_coord(position + vec2<f32>(0.0, eps)) - uv;
        jac = vec4<f32>(xyx.x / eps, xyy.x / eps, xyx.y / eps, xyy.y / eps);
    }

    var pixel: vec4<f32> = bg;

    if (uv.x > -99998.0) {
        let width_f = f32(params.width);
        let height_f = f32(params.height);
        if (params.background_mode == 3) { // margin with feather
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

            var frame_size = vec2<f32>(f32(params.width), f32(params.height));
            if (params.input_rotation != 0.0) {
                let rotation = params.input_rotation * (3.14159265359 / 180.0);
                let size = frame_size;
                frame_size = abs(round(rotate_point(size, rotation, vec2<f32>(0.0, 0.0), vec2<f32>(0.0, 0.0))));
            }
            if (bool(flags & 32)) { // Uses source rect
                uv  = vec2<f32>(map_coord(uv.x,  0.0, f32(frame_size.x), f32(params.source_rect.x), f32(params.source_rect.x + params.source_rect.z)),
                                map_coord(uv.y,  0.0, f32(frame_size.y), f32(params.source_rect.y), f32(params.source_rect.y + params.source_rect.w)));
                pt2 = vec2<f32>(map_coord(pt2.x, 0.0, f32(frame_size.x), f32(params.source_rect.x), f32(params.source_rect.x + params.source_rect.z)),
                                map_coord(pt2.y, 0.0, f32(frame_size.y), f32(params.source_rect.y), f32(params.source_rect.y + params.source_rect.w)));
            }

            let c1 = sample_input_at(uv, jac);
            let c2 = sample_input_at(pt2, jac); // FIXME: jac should be adjusted for pt2
            pixel = c1 * alpha + c2 * (1.0 - alpha);
            pixel = draw_pixel(pixel, u32(p.x), u32(p.y), false);
            pixel = draw_safe_area(pixel, p.x, p.y);
            return vec4<SCALAR>(pixel);
        }

        pixel = sample_input_at(uv, jac);
    }
    pixel = draw_pixel(pixel, u32(p.x), u32(p.y), false);
    pixel = draw_safe_area(pixel, p.x, p.y);
    return vec4<SCALAR>(pixel);
}

// {texture_input}
@vertex
fn undistort_vertex(@builtin(vertex_index) in_vertex_index: u32) -> @builtin(position) @invariant vec4<f32> {
    var positions: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>( 1.0, -1.0), vec2<f32>( 1.0,  1.0),
        vec2<f32>( 1.0,  1.0), vec2<f32>(-1.0,  1.0), vec2<f32>(-1.0, -1.0),
    );
    return vec4<f32>(positions[in_vertex_index], 0.0, 1.0);
}
@fragment
fn undistort_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<SCALAR> {
    return undistort(position.xy);
}
// {/texture_input}

// {buffer_input}
@compute @workgroup_size(8, 8)
fn undistort_compute(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let final_px = undistort(vec2<f32>(f32(global_id.x), f32(global_id.y)));
    let stride_px = params.output_stride / (bytes_per_pixel / pix_element_count);
    let buffer_pos = (global_id.y * u32(stride_px) + global_id.x * u32(pix_element_count));
    if (pix_element_count >= 1) { output_buffer[buffer_pos + 0u] = final_px.x; }
    if (pix_element_count >= 2) { output_buffer[buffer_pos + 1u] = final_px.y; }
    if (pix_element_count >= 3) { output_buffer[buffer_pos + 2u] = final_px.z; }
    if (pix_element_count >= 4) { output_buffer[buffer_pos + 3u] = final_px.w; }
}
// {/buffer_input}
