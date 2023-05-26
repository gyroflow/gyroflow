// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

enum {
    INTER_BITS = 5,
    INTER_TAB_SIZE = 1 << INTER_BITS
};
// #ifdef cl_amd_fp64
// #pragma OPENCL EXTENSION cl_amd_fp64:enable
// #elif defined (cl_khr_fp64)
// #pragma OPENCL EXTENSION cl_khr_fp64:enable
// #endif

typedef struct {
    int width;         // 4
    int height;        // 8
    int stride;        // 12
    int output_width;  // 16
    int output_height; // 4
    int output_stride; // 8
    int matrix_count;  // 12 - for rolling shutter correction. 1 = no correction, only main matrix
    int interpolation; // 16
    int background_mode;   // 4
    int flags;             // 8
    int bytes_per_pixel;   // 12
    int pix_element_count; // 16
    float4 background; // 16
    float2 f;          // 8  - focal length in pixels
    float2 c;          // 16 - lens center
    float k[12];       // 16, 16, 16 - distortion coefficients
    float fov;         // 4
    float r_limit;     // 8
    float lens_correction_amount;    // 12
    float input_vertical_stretch;    // 16
    float input_horizontal_stretch;  // 4
    float background_margin;         // 8
    float background_margin_feather; // 12
    float canvas_scale;              // 16
    float input_rotation;            // 4
    float reserved3;                 // 8
    float2 translation2d;            // 16
    float4 translation3d;            // 16
    int4 source_rect;                // 16
    int4 output_rect;                // 16
    float4 digital_lens_params;      // 16
    float4 safe_area_rect;           // 16
} KernelParams;

#if INTERPOLATION == 2 // Bilinear
#define S_OFFSET 0.0f
__constant float coeffs[64] = {
    1.000000f, 0.000000f, 0.968750f, 0.031250f, 0.937500f, 0.062500f, 0.906250f, 0.093750f, 0.875000f, 0.125000f, 0.843750f, 0.156250f,
    0.812500f, 0.187500f, 0.781250f, 0.218750f, 0.750000f, 0.250000f, 0.718750f, 0.281250f, 0.687500f, 0.312500f, 0.656250f, 0.343750f,
    0.625000f, 0.375000f, 0.593750f, 0.406250f, 0.562500f, 0.437500f, 0.531250f, 0.468750f, 0.500000f, 0.500000f, 0.468750f, 0.531250f,
    0.437500f, 0.562500f, 0.406250f, 0.593750f, 0.375000f, 0.625000f, 0.343750f, 0.656250f, 0.312500f, 0.687500f, 0.281250f, 0.718750f,
    0.250000f, 0.750000f, 0.218750f, 0.781250f, 0.187500f, 0.812500f, 0.156250f, 0.843750f, 0.125000f, 0.875000f, 0.093750f, 0.906250f,
    0.062500f, 0.937500f, 0.031250f, 0.968750f
};
#elif INTERPOLATION == 4 // Bicubic
#define S_OFFSET 1.0f
__constant float coeffs[128] = {
     0.000000f, 1.000000f, 0.000000f,  0.000000f, -0.021996f, 0.997841f, 0.024864f, -0.000710f, -0.041199f, 0.991516f, 0.052429f, -0.002747f,
    -0.057747f, 0.981255f, 0.082466f, -0.005974f, -0.071777f, 0.967285f, 0.114746f, -0.010254f, -0.083427f, 0.949837f, 0.149040f, -0.015450f,
    -0.092834f, 0.929138f, 0.185120f, -0.021423f, -0.100136f, 0.905418f, 0.222755f, -0.028038f, -0.105469f, 0.878906f, 0.261719f, -0.035156f,
    -0.108971f, 0.849831f, 0.301781f, -0.042641f, -0.110779f, 0.818420f, 0.342712f, -0.050354f, -0.111031f, 0.784904f, 0.384285f, -0.058159f,
    -0.109863f, 0.749512f, 0.426270f, -0.065918f, -0.107414f, 0.712471f, 0.468437f, -0.073494f, -0.103821f, 0.674011f, 0.510559f, -0.080750f,
    -0.099220f, 0.634361f, 0.552406f, -0.087547f, -0.093750f, 0.593750f, 0.593750f, -0.093750f, -0.087547f, 0.552406f, 0.634361f, -0.099220f,
    -0.080750f, 0.510559f, 0.674011f, -0.103821f, -0.073494f, 0.468437f, 0.712471f, -0.107414f, -0.065918f, 0.426270f, 0.749512f, -0.109863f,
    -0.058159f, 0.384285f, 0.784904f, -0.111031f, -0.050354f, 0.342712f, 0.818420f, -0.110779f, -0.042641f, 0.301781f, 0.849831f, -0.108971f,
    -0.035156f, 0.261719f, 0.878906f, -0.105469f, -0.028038f, 0.222755f, 0.905418f, -0.100136f, -0.021423f, 0.185120f, 0.929138f, -0.092834f,
    -0.015450f, 0.149040f, 0.949837f, -0.083427f, -0.010254f, 0.114746f, 0.967285f, -0.071777f, -0.005974f, 0.082466f, 0.981255f, -0.057747f,
    -0.002747f, 0.052429f, 0.991516f, -0.041199f, -0.000710f, 0.024864f, 0.997841f, -0.021996f
};
#elif INTERPOLATION == 8 // Lanczos4
#define S_OFFSET 3.0f
__constant float coeffs[256] = {
     0.000000f,  0.000000f,  0.000000f,  1.000000f,  0.000000f,  0.000000f,  0.000000f,  0.000000f, -0.002981f,  0.009625f, -0.027053f,  0.998265f,
     0.029187f, -0.010246f,  0.003264f, -0.000062f, -0.005661f,  0.018562f, -0.051889f,  0.993077f,  0.060407f, -0.021035f,  0.006789f, -0.000250f,
    -0.008027f,  0.026758f, -0.074449f,  0.984478f,  0.093543f, -0.032281f,  0.010545f, -0.000567f, -0.010071f,  0.034167f, -0.094690f,  0.972534f,
     0.128459f, -0.043886f,  0.014499f, -0.001012f, -0.011792f,  0.040757f, -0.112589f,  0.957333f,  0.165004f, -0.055744f,  0.018613f, -0.001582f,
    -0.013191f,  0.046507f, -0.128145f,  0.938985f,  0.203012f, -0.067742f,  0.022845f, -0.002271f, -0.014275f,  0.051405f, -0.141372f,  0.917621f,
     0.242303f, -0.079757f,  0.027146f, -0.003071f, -0.015054f,  0.055449f, -0.152304f,  0.893389f,  0.282684f, -0.091661f,  0.031468f, -0.003971f,
    -0.015544f,  0.058648f, -0.160990f,  0.866453f,  0.323952f, -0.103318f,  0.035754f, -0.004956f, -0.015761f,  0.061020f, -0.167496f,  0.836995f,
     0.365895f, -0.114591f,  0.039949f, -0.006011f, -0.015727f,  0.062590f, -0.171900f,  0.805208f,  0.408290f, -0.125335f,  0.043992f, -0.007117f,
    -0.015463f,  0.063390f, -0.174295f,  0.771299f,  0.450908f, -0.135406f,  0.047823f, -0.008254f, -0.014995f,  0.063460f, -0.174786f,  0.735484f,
     0.493515f, -0.144657f,  0.051378f, -0.009399f, -0.014349f,  0.062844f, -0.173485f,  0.697987f,  0.535873f, -0.152938f,  0.054595f, -0.010527f,
    -0.013551f,  0.061594f, -0.170517f,  0.659039f,  0.577742f, -0.160105f,  0.057411f, -0.011613f, -0.012630f,  0.059764f, -0.166011f,  0.618877f,
     0.618877f, -0.166011f,  0.059764f, -0.012630f, -0.011613f,  0.057411f, -0.160105f,  0.577742f,  0.659039f, -0.170517f,  0.061594f, -0.013551f,
    -0.010527f,  0.054595f, -0.152938f,  0.535873f,  0.697987f, -0.173485f,  0.062844f, -0.014349f, -0.009399f,  0.051378f, -0.144657f,  0.493515f,
     0.735484f, -0.174786f,  0.063460f, -0.014995f, -0.008254f,  0.047823f, -0.135406f,  0.450908f,  0.771299f, -0.174295f,  0.063390f, -0.015463f,
    -0.007117f,  0.043992f, -0.125336f,  0.408290f,  0.805208f, -0.171900f,  0.062590f, -0.015727f, -0.006011f,  0.039949f, -0.114591f,  0.365895f,
     0.836995f, -0.167496f,  0.061020f, -0.015761f, -0.004956f,  0.035754f, -0.103318f,  0.323952f,  0.866453f, -0.160990f,  0.058648f, -0.015544f,
    -0.003971f,  0.031468f, -0.091661f,  0.282684f,  0.893389f, -0.152304f,  0.055449f, -0.015054f, -0.003071f,  0.027146f, -0.079757f,  0.242303f,
     0.917621f, -0.141372f,  0.051405f, -0.014275f, -0.002271f,  0.022845f, -0.067742f,  0.203012f,  0.938985f, -0.128145f,  0.046507f, -0.013191f,
    -0.001582f,  0.018613f, -0.055744f,  0.165004f,  0.957333f, -0.112589f,  0.040757f, -0.011792f, -0.001012f,  0.014499f, -0.043886f,  0.128459f,
     0.972534f, -0.094690f,  0.034167f, -0.010071f, -0.000567f,  0.010545f, -0.032281f,  0.093543f,  0.984478f, -0.074449f,  0.026758f, -0.008027f,
    -0.000250f,  0.006789f, -0.021035f,  0.060407f,  0.993077f, -0.051889f,  0.018562f, -0.005661f, -0.000062f,  0.003264f, -0.010246f,  0.029187f,
     0.998265f, -0.027053f,  0.009625f, -0.002981f
};
#endif

__constant float4 colors[9] = {
    (float4)(0.0f,   0.0f,   0.0f,     0.0f), // None
    (float4)(255.0f, 0.0f,   0.0f,   255.0f), // Red
    (float4)(0.0f,   255.0f, 0.0f,   255.0f), // Green
    (float4)(0.0f,   0.0f,   255.0f, 255.0f), // Blue
    (float4)(254.0f, 251.0f, 71.0f,  255.0f), // Yellow
    (float4)(200.0f, 200.0f, 0.0f,   255.0f), // Yellow2
    (float4)(255.0f, 0.0f,   255.0f, 255.0f), // Magenta
    (float4)(0.0f,   128.0f, 255.0f, 255.0f), // Blue2
    (float4)(0.0f,   200.0f, 200.0f, 255.0f)  // Blue3
};
__constant float alphas[4] = { 1.0f, 0.75f, 0.50f, 0.25f };
void draw_pixel(DATA_TYPE *out_pix, int x, int y, bool isInput, int width, __global KernelParams *params, __global const uchar *drawing) {
    if (!(params->flags & 8)) { // Drawing not enabled
        return;
    }
    int pos = (int)round(floor((float)y / params->canvas_scale) * (width / params->canvas_scale) + floor((float)x / params->canvas_scale));
    uchar data = drawing[pos];
    if (data > 0) {
        uchar color = (data & 0xF8) >> 3;
        uchar alpha = (data & 0x06) >> 1;
        uchar stage = data & 1;
        if (((stage == 0 && isInput) || (stage == 1 && !isInput)) && color < 9 && alpha < 4) {
            float4 colorf4 = colors[color];
            DATA_TYPEF colorf = *(DATA_TYPEF *)&colorf4;

            float alphaf = alphas[alpha];

            *out_pix = DATA_CONVERT(colorf * alphaf + DATA_CONVERTF(*out_pix) * (1.0f - alphaf));
        }
    }
}
void draw_safe_area(DATA_TYPE *pix, float x, float y, __global KernelParams *params) {
    bool isSafeArea = x >= params->safe_area_rect.x && x <= params->safe_area_rect.z &&
                      y >= params->safe_area_rect.y && y <= params->safe_area_rect.w;
    if (!isSafeArea) {
        float4 factorf4 = (float4)(0.5, 0.5, 0.5, 1.0);
        DATA_TYPEF factorf = *(DATA_TYPEF *)&factorf4;
        *pix = DATA_CONVERT(DATA_CONVERTF(*pix) * factorf);
        bool isBorder = x >= params->safe_area_rect.x - 5.0 && x <= params->safe_area_rect.z + 5.0 &&
                        y >= params->safe_area_rect.y - 5.0 && y <= params->safe_area_rect.w + 5.0;
        if (isBorder) {
            float4 borderf4 = (float4)(40.0, 40.0, 40.0, 255.0);
            DATA_TYPEF borderf = *(DATA_TYPEF *)&borderf4;
            *pix = DATA_CONVERT(borderf);
        }
    }
}

// From 0-255(JPEG/Full) to 16-235(MPEG/Limited)
DATA_TYPEF remap_colorrange(DATA_TYPEF px, bool isY) {
    if (isY) { return 16.0f + (px * 0.85882352f); } // (235 - 16) / 255
    else     { return 16.0f + (px * 0.87843137f); } // (240 - 16) / 255
}
float map_coord(float x, float in_min, float in_max, float out_min, float out_max) {
    return (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min;
}

LENS_MODEL_FUNCTIONS;

float2 rotate_point(float2 pos, float angle, float2 origin) {
     return (float2)(cos(angle) * (pos.x - origin.x) - sin(angle) * (pos.y - origin.y) + origin.x,
                     sin(angle) * (pos.x - origin.x) + cos(angle) * (pos.y - origin.y) + origin.y);
}

DATA_TYPEF sample_input_at(float2 uv, __global const uchar *srcptr, __global KernelParams *params, __global const uchar *drawing, DATA_TYPEF bg) {
    bool fix_range = params->flags & 1;

    if (params->input_rotation != 0.0) {
        uv = rotate_point(uv, params->input_rotation * (M_PI_F / 180.0), (float2)((float)params->width / 2.0, (float)params->height / 2.0));
    }

    uv.x = map_coord(uv.x, 0.0f, (float)params->width,  (float)params->source_rect.x, (float)(params->source_rect.x + params->source_rect.z));
    uv.y = map_coord(uv.y, 0.0f, (float)params->height, (float)params->source_rect.y, (float)(params->source_rect.y + params->source_rect.w));

    uv -= S_OFFSET;

    const int shift = (INTERPOLATION >> 2) + 1;

    int sx0 = convert_int_sat_rtz(0.5f + uv.x * INTER_TAB_SIZE);
    int sy0 = convert_int_sat_rtz(0.5f + uv.y * INTER_TAB_SIZE);

    int sx = sx0 >> INTER_BITS;
    int sy = sy0 >> INTER_BITS;

    __constant float *coeffs_x = &coeffs[(sx0 & (INTER_TAB_SIZE - 1)) << shift];
    __constant float *coeffs_y = &coeffs[(sy0 & (INTER_TAB_SIZE - 1)) << shift];

    DATA_TYPEF sum = 0;
    int src_index = sy * params->stride + sx * PIXEL_BYTES;

    #pragma unroll
    for (int yp = 0; yp < INTERPOLATION; ++yp) {
        if (sy + yp >= params->source_rect.y && sy + yp < params->source_rect.y + params->source_rect.w) {
            DATA_TYPEF xsum = 0.0f;
            #pragma unroll
            for (int xp = 0; xp < INTERPOLATION; ++xp) {
                if (sx + xp >= params->source_rect.x && sx + xp < params->source_rect.x + params->source_rect.z) {
                    DATA_TYPE src_px = *(__global const DATA_TYPE *)&srcptr[src_index + PIXEL_BYTES * xp];
                    draw_pixel(&src_px, sx + xp, sy + yp, true, max(params->width, params->output_width), params, drawing);
                    DATA_TYPEF srcpx = DATA_CONVERTF(src_px);
                    if (fix_range) {
                        srcpx = remap_colorrange(srcpx, PIXEL_BYTES == 1);
                    }
                    xsum += srcpx * coeffs_x[xp];
                } else {
                    xsum += bg * coeffs_x[xp];
                }
            }
            sum += xsum * coeffs_y[yp];
        } else {
            sum += bg * coeffs_y[yp];
        }
        src_index += params->stride;
    }
    return sum;
}

float2 rotate_and_distort(float2 pos, uint idx, __global KernelParams *params, __global const float *matrices) {
    __global const float *matrix = &matrices[idx];
    float _x = (pos.x * matrix[0]) + (pos.y * matrix[1]) + matrix[2] + params->translation3d.x;
    float _y = (pos.x * matrix[3]) + (pos.y * matrix[4]) + matrix[5] + params->translation3d.y;
    float _w = (pos.x * matrix[6]) + (pos.y * matrix[7]) + matrix[8] + params->translation3d.z;
    if (_w > 0) {
        if (params->r_limit > 0.0f && length((float2)(_x, _y) / _w) > params->r_limit) {
            return (float2)(-99999.0f, -99999.0f);
        }
        float2 uv = params->f * distort_point(_x, _y, _w, params) + params->c;

        if (params->flags & 2) { // Has digital lens
            uv = digital_distort_point(uv, params);
        }

        if (params->input_horizontal_stretch > 0.001f) { uv.x /= params->input_horizontal_stretch; }
        if (params->input_vertical_stretch   > 0.001f) { uv.y /= params->input_vertical_stretch; }

        return uv;
    }
    return (float2)(-99999.0f, -99999.0f);
}

// Adapted from OpenCV: initUndistortRectifyMap + remap
// https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/calib3d/src/fisheye.cpp#L465-L567
// https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/imgproc/src/opencl/remap.cl#L390-L498
__kernel void undistort_image(__global const uchar *srcptr, __global uchar *dstptr, __global const void *params_buf, __global const float *matrices, __global const uchar *drawing) {
    int buf_x = get_global_id(0);
    int buf_y = get_global_id(1);

    __global KernelParams *params = (__global KernelParams *)params_buf;

    float x = map_coord((float)buf_x, (float)params->output_rect.x, (float)(params->output_rect.x + params->output_rect.z), 0.0f, (float)params->output_width );
    float y = map_coord((float)buf_y, (float)params->output_rect.y, (float)(params->output_rect.y + params->output_rect.w), 0.0f, (float)params->output_height);

    DATA_TYPEF bg = *(__global DATA_TYPEF *)&params->background;

    if (matrices == 0 || params->width < 1) return;

    if (x >= 0.0f && y >= 0.0f && x < (float)params->output_width && y < (float)params->output_height) {
        __global DATA_TYPE *out_pix = (__global DATA_TYPE *)&dstptr[buf_x * PIXEL_BYTES + buf_y * params->output_stride];

        if (params->flags & 4) { // Fill with background
            *out_pix = DATA_CONVERT(bg);
            return;
        }

        float2 out_pos = (float2)(x, y) + params->translation2d;

        ///////////////////////////////////////////////////////////////////
        // Add lens distortion back
        if (params->lens_correction_amount < 1.0f) {
            float2 factor = (float2)max(1.0f - params->lens_correction_amount, 0.001f); // FIXME: this is close but wrong
            float2 out_c = (float2)(params->output_width / 2.0f, params->output_height / 2.0f);
            float2 out_f = (params->f / params->fov) / factor;

            float2 new_out_pos = out_pos;

            if (params->flags & 2) { // Has digital lens
                new_out_pos = digital_undistort_point(new_out_pos, params);
            }
            new_out_pos = (new_out_pos - out_c) / out_f;
            new_out_pos = undistort_point(new_out_pos, params);
            new_out_pos = out_f * new_out_pos + out_c;

            out_pos = new_out_pos * (1.0f - params->lens_correction_amount) + (out_pos * params->lens_correction_amount);
        }
        ///////////////////////////////////////////////////////////////////

        ///////////////////////////////////////////////////////////////////
        // Calculate source `y` for rolling shutter
        int sy = min((int)params->height, max(0, (int)round(out_pos.y)));
        if (params->matrix_count > 1) {
            int idx = (params->matrix_count / 2) * 9; // Use middle matrix
            float2 uv = rotate_and_distort(out_pos, idx, params, matrices);
            if (uv.x > -99998.0f) {
                sy = min((int)params->height, max(0, (int)round(uv.y)));
            }
        }
        ///////////////////////////////////////////////////////////////////

        DATA_TYPE final_pix;

        int idx = min(sy, params->matrix_count - 1) * 9;
        float2 uv = rotate_and_distort(out_pos, idx, params, matrices);
        if (uv.x > -99998.0f) {
            switch (params->background_mode) {
                case 1: { // edge repeat
                    uv = max((float2)(0, 0), min((float2)(params->width - 1, params->height - 1), uv));
                } break;
                case 2: { // edge mirror
                    int rx = round(uv.x);
                    int ry = round(uv.y);
                    int width3 = (params->width - 3);
                    int height3 = (params->height - 3);
                    if (rx > width3)  uv.x = width3  - (rx - width3);
                    if (rx < 3)       uv.x = 3 + params->width - (width3  + rx);
                    if (ry > height3) uv.y = height3 - (ry - height3);
                    if (ry < 3)       uv.y = 3 + params->height - (height3 + ry);
                } break;
                case 3: { // margin with feather
                    float widthf  = (params->width  - 1);
                    float heightf = (params->height - 1);

                    float feather = max(0.0001f, params->background_margin_feather * heightf);
                    float2 pt2 = uv;
                    float alpha = 1.0f;
                    if ((uv.x > widthf - feather) || (uv.x < feather) || (uv.y > heightf - feather) || (uv.y < feather)) {
                        alpha = fmax(0.0f, fmin(1.0f, fmin(fmin(widthf - uv.x, heightf - uv.y), fmin(uv.x, uv.y)) / feather));
                        pt2 /= (float2)(widthf, heightf);
                        pt2 = ((pt2 - 0.5f) * (1.0f - params->background_margin)) + 0.5f;
                        pt2 *= (float2)(widthf, heightf);
                    }

                    DATA_TYPEF c1 = sample_input_at(uv,  srcptr, params, drawing, bg);
                    DATA_TYPEF c2 = sample_input_at(pt2, srcptr, params, drawing, bg);
                    final_pix = DATA_CONVERT(c1 * alpha + c2 * (1.0f - alpha));
                    draw_pixel(&final_pix, x, y, false, max(params->width, params->output_width), params, drawing);
                    draw_safe_area(&final_pix, x, y, params);
                    *out_pix = final_pix;
                    return;
                } break;
            }

            final_pix = DATA_CONVERT(sample_input_at(uv, srcptr, params, drawing, bg));
        } else {
            final_pix = DATA_CONVERT(bg);
        }
        draw_pixel(&final_pix, x, y, false, max(params->width, params->output_width), params, drawing);
        draw_safe_area(&final_pix, x, y, params);

        *out_pix = final_pix;
    }
}
