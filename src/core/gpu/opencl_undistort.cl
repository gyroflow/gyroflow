// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

EXTENSIONS;

enum {
    INTER_BITS = 5,
    INTER_TAB_SIZE = 1 << INTER_BITS
};

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
    float output_rotation;           // 8
    float2 translation2d;            // 16
    float4 translation3d;            // 16
    int4 source_rect;                // 16
    int4 output_rect;                // 16
    float4 digital_lens_params;      // 16
    float4 safe_area_rect;           // 16
    float max_pixel_value;           // 4
    int distortion_model;            // 8
    int digital_lens;                // 12
    float pixel_value_limit;         // 16
    float light_refraction_coefficient; // 4
    int plane_index;                 // 8
    float shutter_speed;             // 12
    int shutter_samples;             // 16
    float4 ewa_coeffs_p;             // 16
    float4 ewa_coeffs_q;             // 16
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
            *pix = DATA_CONVERT(DATA_CONVERTF(*pix) * factorf);
        }
    }
}

// From 0-255(JPEG/Full) to 16-235(MPEG/Limited)
DATA_TYPEF remap_colorrange(DATA_TYPEF px, bool isY, __global KernelParams *params) {
    if (isY) { return ((16.0f / 255.0f) * params->max_pixel_value) + (px * 0.85882352f); } // (235 - 16) / 255
    else     { return ((16.0f / 255.0f) * params->max_pixel_value) + (px * 0.87843137f); } // (240 - 16) / 255
}
float map_coord(float x, float in_min, float in_max, float out_min, float out_max) {
    return (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min;
}

LENS_MODEL_FUNCTIONS;

float2 rotate_point(float2 pos, float angle, float2 origin, float2 origin2) {
     return (float2)(cos(angle) * (pos.x - origin.x) - sin(angle) * (pos.y - origin.y) + origin2.x,
                     sin(angle) * (pos.x - origin.x) + cos(angle) * (pos.y - origin.y) + origin2.y);
}

#define GRID_SIZE 9
void cubic_spline_coefficients(__private float *mesh, int step, int offset, float size, __private float *a, __private float *b, __private float *c, __private float *d, __private float *alpha, __private float *mu, __private float *z) {
    #define n GRID_SIZE
    float h = size / (float)(n - 1);
    float inv_h = 1.0f / h;
    float three_inv_h = 3.0f * inv_h;
    float h_over_3 = h / 3.0f;
    float inv_3h = 1.0f / (3.0f * h);
    for (int i = 0; i < n; i++) { a[i] = mesh[(i + offset) * step]; }
    for (int i = 1; i < n - 1; i++) { alpha[i] = three_inv_h * (a[i + 1] - 2.0f * a[i] + a[i - 1]); }

    mu[0] = 0.0f;
    z[0] = 0.0f;

    for (int i = 1; i < n - 1; i++) {
        mu[i] = 1.0f / (4.0f - mu[i - 1]);
        z[i] = (alpha[i] * inv_h - z[i - 1]) * mu[i];
    }

    c[n - 1] = 0.0f;

    for (int j = n - 2; j >= 0; j--) {
        c[j] = z[j] - mu[j] * c[j + 1];
        b[j] = (a[j + 1] - a[j]) * inv_h - h_over_3 * (c[j + 1] + 2.0f * c[j]);
        d[j] = (c[j + 1] - c[j]) * inv_3h;
    }
    #undef n
}
float cubic_spline_interpolate2(__private float *a, __private float *b, __private float *c, __private float *d, int n, float x, float size) {
    int i = max(0.0f, min(n - 2.0f, (n - 1.0f) * x / size));
    float dx = x - size * i / (n - 1.0f);
    return a[i] + b[i] * dx + c[i] * dx * dx + d[i] * dx * dx * dx;
}
float bivariate_spline_interpolate(float size_x, float size_y, __global const float *mesh, int mesh_offset, int n, float x, float y) {
    __private float intermediate_values[GRID_SIZE];
    __private float a[GRID_SIZE], b[GRID_SIZE], c[GRID_SIZE], d[GRID_SIZE];
    __private float alpha[GRID_SIZE - 1], mu[GRID_SIZE], z[GRID_SIZE];

    const int i = max(0.0f, min((float)(GRID_SIZE - 2), (float)(GRID_SIZE - 1) * x / size_x));
    const float dx = x - size_x * i / (float)(GRID_SIZE - 1);
    const float dx2 = dx * dx;
    const int block = GRID_SIZE * 4;
    const int offs = 9 + GRID_SIZE*GRID_SIZE*2 + (block * GRID_SIZE * mesh_offset) + i;

    #pragma unroll
    for (int j = 0; j < GRID_SIZE; j++) {
        intermediate_values[j] = mesh[offs + (GRID_SIZE * 0) + (j * block)]
                               + mesh[offs + (GRID_SIZE * 1) + (j * block)] * dx
                               + mesh[offs + (GRID_SIZE * 2) + (j * block)] * dx2
                               + mesh[offs + (GRID_SIZE * 3) + (j * block)] * dx2 * dx;
        // cubic_spline_coefficients(&mesh[9 + mesh_offset], 2, (j * GRID_SIZE), size_x, GRID_SIZE, a, b, c, d, alpha, mu, z);
        // intermediate_values[j] = cubic_spline_interpolate1(mesh, aa, bb, cc, dd, GRID_SIZE, x, size_x);
    }

    cubic_spline_coefficients(intermediate_values, 1, 0, size_y, a, b, c, d, alpha, mu, z);
    return cubic_spline_interpolate2(a, b, c, d, GRID_SIZE, y, size_y);
}
float2 interpolate_mesh(__global const float *mesh, int width, int height, float2 pos) {
    if (pos.x < 0 || pos.x > width || pos.y < 0 || pos.y > height) {
        return pos;
    }
    return (float2)(
        bivariate_spline_interpolate(width, height, mesh, 0, GRID_SIZE, pos.x, pos.y),
        bivariate_spline_interpolate(width, height, mesh, 1, GRID_SIZE, pos.x, pos.y)
    );
}

////////////////////////////// EWA (Elliptical Weighted Average) CubicBC sampling //////////////////////////////
// Keys Cubic Filter Family https://imagemagick.org/Usage/filter/#robidoux
// https://github.com/ImageMagick/ImageMagick/blob/main/MagickCore/resize.c
#if INTERPOLATION > 8
// Gives a bounding box in the source image containing pixels that cover a circle of radius 2 completely in both the source and destination images
float2 affine_bbox(float4 jac) {
    return (float2)(
        2.0f * fmax(1.0f, fmax(fabs(jac.x + jac.y), fabs(jac.x - jac.y))),
        2.0f * fmax(1.0f, fmax(fabs(jac.z + jac.w), fabs(jac.z - jac.w)))
    );
}
// Computes minimum area ellipse which covers a unit circle in both the source and destination image
float3 clamped_ellipse(float4 jac) {
    // find ellipse
    const float F0 = fabs(jac.x * jac.w - jac.y * jac.z);
    const float F = fmax(0.1f, F0 * F0);
    const float A = (jac.z * jac.z + jac.w * jac.w) / F;
    const float B = -2.0f * (jac.x * jac.z + jac.y * jac.w) / F;
    const float C = (jac.x * jac.x + jac.y * jac.y) / F;
    // find the angle to rotate ellipse
    const float2 v = (float2)(C - A, -B);
    const float lv = length(v);
    const float v0 = (lv > 0.01f) ? v.x / lv : 1.0f;
    //const float v1 = (lv > 0.01f) ? v.y / lv : 1.0f;
    const float c = sqrt(fmax(0.0f, 1.0f + v0) / 2.0f);
    float s = sqrt(fmax(1.0f - v0, 0.0f) / 2.0f);
    // rotate the ellipse to align it with axes
    float A0 = (A * c * c - B * c * s + C * s * s);
    float C0 = (A * s * s + B * c * s + C * c * c);
    const float Bt1 = B * (c * c - s * s);
    const float Bt2 = 2.0f * (A - C) * c * s;
    float B0 = Bt1 + Bt2;
    const float B0v2 = Bt1 - Bt2;
    if (fabs(B0) > fabs(B0v2)) {
        s = -s;
        B0 = B0v2;
    }
    // clamp A,C
    A0 = fmin(A0, 1.0f);
    C0 = fmin(C0, 1.0f);
    const float sn = -s;
    // rotate it back
    return (float3)(
        (A0 * c * c - B0 * c * sn + C0 * sn * sn),
        (2.0f * A0 * c * sn + B0 * c * c - B0 * sn * sn - 2.0f * C0 * c * sn),
        (A0 * sn * sn + B0 * c * sn + C0 * c * c)
    );
}
inline float bc2(float x, __global KernelParams *params) {
    x = fabs(x);
    if (x < 1.0f)
        return params->ewa_coeffs_p.x + params->ewa_coeffs_p.y * x + params->ewa_coeffs_p.z * x * x + params->ewa_coeffs_p.w * x * x * x;
    if (x < 2.0f)
        return params->ewa_coeffs_q.x + params->ewa_coeffs_q.y * x + params->ewa_coeffs_q.z * x * x + params->ewa_coeffs_q.w * x * x * x;
    return 0.0f;
}
#endif
////////////////////////////// EWA (Elliptical Weighted Average) CubicBC sampling //////////////////////////////

DATA_TYPEF sample_input_at(float2 uv, float4 jac, __global const uchar *srcptr, __global KernelParams *params, __global const uchar *drawing, DATA_TYPEF bg) {
    bool fix_range = (params->flags & 1);

    DATA_TYPEF sum = 0;

#   if INTERPOLATION > 8
        // find how many pixels we need around that pixel in each direction
        float2 trans_size = affine_bbox(jac);
        int4 bounds = (int4)(
            floor(uv.x - trans_size.x),
            ceil(uv.x + trans_size.x),
            floor(uv.y - trans_size.y),
            ceil(uv.y + trans_size.y)
        );
        float sum_div = 0.0f;
        int src_index = bounds.z * params->stride;

        // See: Andreas Gustafsson. "Interactive Image Warping", section 3.6 http://www.gson.org/thesis/warping-thesis.pdf
        float3 abc = clamped_ellipse(jac);
        #pragma unroll
        for (int in_y = bounds.z; in_y <= bounds.w; ++in_y) {
            const float in_fy = (float)in_y - uv.y;
            #pragma unroll
            for (int in_x = bounds.x; in_x <= bounds.y; ++in_x) {
                const float in_fx = (float)in_x - uv.x;
                const float dr = in_fx * in_fx * abc.x + in_fx * in_fy * abc.y + in_fy * in_fy * abc.z;
                const float k = bc2(sqrt(dr), params); // cylindrical filtering
                if (k == 0.0f)
                    continue;
                DATA_TYPEF srcpx;
                if (in_y >= params->source_rect.y && in_y < params->source_rect.y + params->source_rect.w && in_x >= params->source_rect.x && in_x < params->source_rect.x + params->source_rect.z) {
                    DATA_TYPE src_px = *(__global const DATA_TYPE *)&srcptr[src_index + in_x * PIXEL_BYTES];
                    draw_pixel(&src_px, in_x, in_y, true, max(params->width, params->output_width), params, drawing);
                    srcpx = DATA_CONVERTF(src_px);
                } else {
                    srcpx = bg;
                }
                sum += k * srcpx;
                sum_div += k;
            }
            src_index += params->stride;
        }
        sum /= sum_div;
#   else
        uv -= S_OFFSET;
        // uv -= (INTERPOLATION >> 1) - 1;

        const int shift = (INTERPOLATION >> 2) + 1;

        int sx0 = convert_int_sat_rtz(0.5f + uv.x * INTER_TAB_SIZE);
        int sy0 = convert_int_sat_rtz(0.5f + uv.y * INTER_TAB_SIZE);

        int sx = sx0 >> INTER_BITS;
        int sy = sy0 >> INTER_BITS;

        __constant float *coeffs_x = &coeffs[(sx0 & (INTER_TAB_SIZE - 1)) << shift];
        __constant float *coeffs_y = &coeffs[(sy0 & (INTER_TAB_SIZE - 1)) << shift];

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
#   endif

    if (fix_range) {
        sum = remap_colorrange(sum, params->plane_index == 0, params);
    }
    sum = min(sum, (DATA_TYPEF)(params->pixel_value_limit));
    return sum;
}

float2 rotate_and_distort(float2 pos, uint idx, __global KernelParams *params, __global const float *matrices, __global const float *mesh_data) {
    __global const float *matrix = &matrices[idx];
    float _x = (pos.x * matrix[0]) + (pos.y * matrix[1]) + matrix[2] + params->translation3d.x;
    float _y = (pos.x * matrix[3]) + (pos.y * matrix[4]) + matrix[5] + params->translation3d.y;
    float _w = (pos.x * matrix[6]) + (pos.y * matrix[7]) + matrix[8] + params->translation3d.z;
    if (_w > 0.0f) {
        if (params->r_limit > 0.0f && length((float2)(_x, _y) / _w) > params->r_limit) {
            return (float2)(-99999.0f, -99999.0f);
        }

        if ((params->flags & 2048) && params->light_refraction_coefficient != 1.0f && params->light_refraction_coefficient > 0.0f) {
            float r = length((float2)(_x, _y)) / _w;
            float sin_theta_d = (r / sqrt(1.0f + r * r)) * params->light_refraction_coefficient;
            float r_d = sin_theta_d / sqrt(1.0f - sin_theta_d * sin_theta_d);
            if (r_d != 0.0f) {
                _w *= r / r_d;
            }
        }

        float2 uv = params->f * distort_point(_x, _y, _w, params);

        if ((params->flags & 256) && (matrix[9] != 0.0f || matrix[10] != 0.0f || matrix[11] != 0.0f || matrix[12] != 0.0f || matrix[13] != 0.0f)) {
            float ang_rad = matrix[11];
            float cos_a = cos(-ang_rad);
            float sin_a = sin(-ang_rad);
            uv = (float2)(
                cos_a * uv.x - sin_a * uv.y - matrix[9]  + matrix[12],
                sin_a * uv.x + cos_a * uv.y - matrix[10] + matrix[13]
            );
        }

        uv += params->c;

        // MeshDistortion
        if ((params->flags & 512) && mesh_data && mesh_data[0] > 10.0f) {
            float2 mesh_size = (float2)(mesh_data[3], mesh_data[4]);
            float2 origin    = (float2)(mesh_data[5], mesh_data[6]);
            float2 crop_size = (float2)(mesh_data[7], mesh_data[8]);

            if ((params->flags & 128)) uv.y = (float)params->height - uv.y; // framebuffer inverted

            uv.x = map_coord(uv.x, 0.0f, (float)params->width,  origin.x, origin.x + crop_size.x);
            uv.y = map_coord(uv.y, 0.0f, (float)params->height, origin.y, origin.y + crop_size.y);

            uv = interpolate_mesh(mesh_data, mesh_size.x, mesh_size.y, uv);

            uv.x = map_coord(uv.x, origin.x, origin.x + crop_size.x, 0.0f, (float)params->width);
            uv.y = map_coord(uv.y, origin.y, origin.y + crop_size.y, 0.0f, (float)params->height);

            if ((params->flags & 128)) uv.y = (float)params->height - uv.y; // framebuffer inverted
        }

        // FocalPlaneDistortion
        if ((params->flags & 1024) && mesh_data && mesh_data[0] > 0.0f && mesh_data[(int)(mesh_data[0])] > 0.0f) {
            int o = (int)(mesh_data[0]); // offset to focal plane distortion data

            float2 mesh_size = (float2)(mesh_data[3], mesh_data[4]);
            float2 origin    = (float2)(mesh_data[5], mesh_data[6]);
            float2 crop_size = (float2)(mesh_data[7], mesh_data[8]);
            float stblz_grid = mesh_size.y / 8.0f;

            if ((params->flags & 128)) uv.y = (float)params->height - uv.y; // framebuffer inverted

            uv.x = map_coord(uv.x, 0.0f, (float)params->width,  origin.x, origin.x + crop_size.x);
            uv.y = map_coord(uv.y, 0.0f, (float)params->height, origin.y, origin.y + crop_size.y);

            int idx = min(7, max(0, (int)floor(uv.y / stblz_grid)));
            float delta = uv.y - stblz_grid * (float)idx;
            uv.x -= mesh_data[o + 4 + idx * 2 + 0] * delta;
            uv.y -= mesh_data[o + 4 + idx * 2 + 1] * delta;
            for (int j = 0; j < idx; j++) {
                uv.x -= mesh_data[o + 4 + j * 2 + 0] * stblz_grid;
                uv.y -= mesh_data[o + 4 + j * 2 + 1] * stblz_grid;
            }

            uv.x = map_coord(uv.x, origin.x, origin.x + crop_size.x, 0.0f, (float)params->width);
            uv.y = map_coord(uv.y, origin.y, origin.y + crop_size.y, 0.0f, (float)params->height);

            if ((params->flags & 128)) uv.y = (float)params->height - uv.y; // framebuffer inverted
        }

        if ((params->flags & 2)) { // Has digital lens
            uv = digital_distort_point(uv, params);
        }

        if (params->input_horizontal_stretch > 0.001f) { uv.x /= params->input_horizontal_stretch; }
        if (params->input_vertical_stretch   > 0.001f) { uv.y /= params->input_vertical_stretch; }

        return uv;
    }
    return (float2)(-99999.0f, -99999.0f);
}

float2 undistort_coord(int sample, float2 out_pos, __global KernelParams *params, __global const float *matrices, __global const float *mesh_data) {
    out_pos.x = map_coord(out_pos.x, (float)params->output_rect.x, (float)(params->output_rect.x + params->output_rect.z), 0.0f, (float)params->output_width ) + params->translation2d.x;
    out_pos.y = map_coord(out_pos.y, (float)params->output_rect.y, (float)(params->output_rect.y + params->output_rect.w), 0.0f, (float)params->output_height) + params->translation2d.y;

    ///////////////////////////////////////////////////////////////////
    // Add lens distortion back
    if (params->lens_correction_amount < 1.0f) {
        float2 factor = (float2)max(1.0f - params->lens_correction_amount, 0.001f); // FIXME: this is close but wrong
        float2 out_c = (float2)(params->output_width / 2.0f, params->output_height / 2.0f);
        float2 out_f = (params->f / params->fov) / factor;

        float2 new_out_pos = out_pos;

        if ((params->flags & 2)) { // Has digital lens
            new_out_pos = digital_undistort_point(new_out_pos, params);
        }
        new_out_pos = (new_out_pos - out_c) / out_f;
        new_out_pos = undistort_point(new_out_pos, params);
        if ((params->flags & 2048) && params->light_refraction_coefficient != 1.0f && params->light_refraction_coefficient > 0.0f) {
            float r = length(new_out_pos);
            if (r != 0.0f) {
                float sin_theta_d = (r / sqrt(1.0f + r * r)) / params->light_refraction_coefficient;
                float r_d = sin_theta_d / sqrt(1.0f - sin_theta_d * sin_theta_d);
                new_out_pos *= r_d / r;
            }
        }
        new_out_pos = out_f * new_out_pos + out_c;

        out_pos = new_out_pos * (1.0f - params->lens_correction_amount) + (out_pos * params->lens_correction_amount);
    }
    ///////////////////////////////////////////////////////////////////

    ///////////////////////////////////////////////////////////////////
    // Calculate source `y` for rolling shutter
    int sy = 0;
    if ((params->flags & 16)) { // Horizontal RS
        sy = min((int)params->width, max(0, (int)round(out_pos.x)));
    } else {
        sy = min((int)params->height, max(0, (int)round(out_pos.y)));
    }
    if (params->matrix_count > 1) {
        int idx = (params->matrix_count / 2) * 14 * params->shutter_samples + 14 * sample; // Use middle matrix
        float2 uv = rotate_and_distort(out_pos, idx, params, matrices, mesh_data);
        if (uv.x > -99998.0f) {
            if ((params->flags & 16)) { // Horizontal RS
                sy = min((int)params->width, max(0, (int)round(uv.x)));
            } else {
                sy = min((int)params->height, max(0, (int)round(uv.y)));
            }
        }
    }
    ///////////////////////////////////////////////////////////////////

    int idx = min(sy, params->matrix_count - 1) * 14 * params->shutter_samples + 14 * sample;
    float2 uv = rotate_and_distort(out_pos, idx, params, matrices, mesh_data);

    float2 frame_size = (float2)((float)params->width, (float)params->height);
    if (params->input_rotation != 0.0f) {
        float rotation = params->input_rotation * (M_PI_F / 180.0f);
        float2 size = frame_size;
        frame_size = fabs(round(rotate_point(size, rotation, (float2)(0.0f, 0.0f), (float2)(0.0f, 0.0f))));
        uv = rotate_point(uv, rotation, size / (float2)2.0f, frame_size / (float2)2.0f);
    }

    switch (params->background_mode) {
        case 1: { // edge repeat
            uv = max((float2)(3.0f, 3.0f), min((float2)(params->width - 3, params->height - 3), uv));
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
    }

    if (params->background_mode != 3) {
        uv.x = map_coord(uv.x, 0.0f, (float)frame_size.x, (float)params->source_rect.x, (float)(params->source_rect.x + params->source_rect.z));
        uv.y = map_coord(uv.y, 0.0f, (float)frame_size.y, (float)params->source_rect.y, (float)(params->source_rect.y + params->source_rect.w));
    }
    return uv;
}

// Adapted from OpenCV: initUndistortRectifyMap + remap
// https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/calib3d/src/fisheye.cpp#L465-L567
// https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/imgproc/src/opencl/remap.cl#L390-L498
__kernel void undistort_image(__global const uchar *srcptr, __global uchar *dstptr, __global const void *params_buf, __global const float *matrices, __global const uchar *drawing, __global const float *mesh_data) {
    int buf_x = get_global_id(0);
    int buf_y = get_global_id(1);

    __global KernelParams *params = (__global KernelParams *)params_buf;

    float x = map_coord((float)buf_x, (float)params->output_rect.x, (float)(params->output_rect.x + params->output_rect.z), 0.0f, (float)params->output_width );
    float y = map_coord((float)buf_y, (float)params->output_rect.y, (float)(params->output_rect.y + params->output_rect.w), 0.0f, (float)params->output_height);

    DATA_TYPEF bg = (*(__global DATA_TYPEF *)&params->background) * params->max_pixel_value;

    if (matrices == 0 || params->width < 1) return;

    if (x >= 0.0f && y >= 0.0f && x < (float)params->output_width && y < (float)params->output_height) {
        __global DATA_TYPE *out_pix = (__global DATA_TYPE *)&dstptr[buf_x * PIXEL_BYTES + buf_y * params->output_stride];

        if (params->flags & 4) { // Fill with background
            *out_pix = DATA_CONVERT(bg);
            return;
        }

        float weight = 1.0f / (float)params->shutter_samples;

        DATA_TYPEF tmp_pix;

        for (int sample = 0; sample < params->shutter_samples; ++sample) {

            float2 out_pos = (float2)((float)buf_x, (float)buf_y);
            float2 uv = undistort_coord(sample, out_pos, params, matrices, mesh_data);
            float4 jac = (float4)(1.0f, 0.0f, 0.0f, 1.0f);

    #       if INTERPOLATION > 8
                const float eps = 0.01f;
                float2 xyx = undistort_coord(sample, out_pos + (float2)(eps, 0.0f), params, matrices, mesh_data) - uv;
                float2 xyy = undistort_coord(sample, out_pos + (float2)(0.0f, eps), params, matrices, mesh_data) - uv;
                jac = (float4)(xyx.x / eps, xyy.x / eps, xyx.y / eps, xyy.y / eps);
    #       endif

            DATA_TYPEF sample_pix;
            if (uv.x > -99998.0f) {
                /*if (params->background_mode == 3) { // margin with feather
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

                    float2 frame_size = (float2)((float)params->width, (float)params->height);
                    if (params->input_rotation != 0.0f) {
                        float rotation = params->input_rotation * (M_PI_F / 180.0f);
                        float2 size = frame_size;
                        frame_size = fabs(round(rotate_point(size, rotation, (float2)(0.0f, 0.0f), (float2)(0.0f, 0.0f))));
                    }
                    uv.x  = map_coord(uv.x,  0.0f, (float)frame_size.x, (float)params->source_rect.x, (float)(params->source_rect.x + params->source_rect.z));
                    uv.y  = map_coord(uv.y,  0.0f, (float)frame_size.y, (float)params->source_rect.y, (float)(params->source_rect.y + params->source_rect.w));
                    pt2.x = map_coord(pt2.x, 0.0f, (float)frame_size.x, (float)params->source_rect.x, (float)(params->source_rect.x + params->source_rect.z));
                    pt2.y = map_coord(pt2.y, 0.0f, (float)frame_size.y, (float)params->source_rect.y, (float)(params->source_rect.y + params->source_rect.w));

                    DATA_TYPEF c1 = sample_input_at(uv,  jac, srcptr, params, drawing, bg);
                    DATA_TYPEF c2 = sample_input_at(pt2, jac, srcptr, params, drawing, bg); // FIXME: jac should be adjusted for pt2
                    final_pix = DATA_CONVERT(c1 * alpha + c2 * (1.0f - alpha));
                    draw_pixel(&final_pix, x, y, false, max(params->width, params->output_width), params, drawing);
                    draw_safe_area(&final_pix, x, y, params);
                    *out_pix = final_pix;
                    return;
                }*/

                sample_pix = sample_input_at(uv, jac, srcptr, params, drawing, bg);
            } else {
                sample_pix = bg;
            }

            tmp_pix += sample_pix * weight;
        }
        DATA_TYPE final_pix = DATA_CONVERT(tmp_pix);

        draw_pixel(&final_pix, x, y, false, max(params->width, params->output_width), params, drawing);
        draw_safe_area(&final_pix, x, y, params);

        *out_pix = final_pix;
    }
}
