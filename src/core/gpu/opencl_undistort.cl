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

float2 undistort_point(float2 pos, float2 f, float2 c, float4 k, float amount) {
    pos = (pos - c) / f;

    float theta_d = fmin(fmax(length(pos), -1.5707963267948966f), 1.5707963267948966f); // PI/2

    bool converged = false;
    float theta = theta_d;

    float scale = 0.0f;

    if (fabs(theta_d) > 1e-6f) {
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
            float theta_fix = (theta * (1.0f + k0_theta2 + k1_theta4 + k2_theta6 + k3_theta8) - theta_d)
                              /
                              (1.0f + 3.0f * k0_theta2 + 5.0f * k1_theta4 + 7.0f * k2_theta6 + 9.0f * k3_theta8);

            theta -= theta_fix;
            if (fabs(theta_fix) < 1e-6f) {
                converged = true;
                break;
            }
        }

        scale = tan(theta) / theta_d;
    } else {
        converged = true;
    }
    bool theta_flipped = (theta_d < 0.0f && theta > 0.0f) || (theta_d > 0.0f && theta < 0.0f);

    if (converged && !theta_flipped) {
        // Apply only requested amount
        scale = 1.0f + (scale - 1.0f) * (1.0f - amount);

        return f * pos * scale + c;
    }
    return (float2)(0.0f, 0.0f);
}

float2 distort_point(float2 pos, float2 f, float2 c, float4 k) {
    float r = length(pos);

    float theta = atan(r);
    float theta2 = theta*theta, 
          theta4 = theta2*theta2, 
          theta6 = theta4*theta2, 
          theta8 = theta4*theta4;

    float theta_d = theta * (1.0 + dot(k, (float4)(theta2, theta4, theta6, theta8)));

    float scale = r == 0? 1.0 : theta_d / r;
    return f * pos * scale + c;
}

// Adapted from OpenCV: initUndistortRectifyMap + remap 
// https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/fisheye.cpp#L454
// https://github.com/opencv/opencv/blob/4.x/modules/imgproc/src/opencl/remap.cl#L390
__kernel void undistort_image(__global const uchar *srcptr, __global uchar *dstptr, ushort width, ushort height, ushort stride, ushort output_width, ushort output_height, ushort output_stride, __global const float *undistortion_params, ushort params_count, DATA_TYPEF bg) {
    int x = get_global_id(0);
    int y = get_global_id(1);

    if (!undistortion_params || params_count - 1 < 1) return;

    float2 f = vload2(0, &undistortion_params[0]);
    float2 c = vload2(0, &undistortion_params[2]);
    float4 k = vload4(0, &undistortion_params[4]);
    float r_limit = undistortion_params[8];
    float lens_correction_amount = undistortion_params[9];
    float background_mode = undistortion_params[10];
    float fov = undistortion_params[11];
    bool edge_repeat = background_mode > 0.9 && background_mode < 1.1; // 1
    bool edge_mirror = background_mode > 1.9 && background_mode < 2.1; // 2

    if (x >= 0 && y >= 0 && x < output_width && y < output_height) {
        ///////////////////////////////////////////////////////////////////
        // Calculate source `y` for rolling shutter
        int sy = y;
        if (params_count > 3) {
            __global const float *params = &undistortion_params[(2 + ((params_count - 2) / 2)) * 9]; // Use middle matrix
            float _x = y * params[1] + params[2] + (x * params[0]);
            float _y = y * params[4] + params[5] + (x * params[3]);
            float _w = y * params[7] + params[8] + (x * params[6]);
            if (_w > 0) {
                float2 pos = (float2)(_x, _y) / _w;
                float2 uv = distort_point(pos, f, c, k);
                sy = min((int)height, max(0, (int)round(uv.y)));
            }
        }
        ///////////////////////////////////////////////////////////////////

        float2 dst_point = (float2)(x, y);
        if (lens_correction_amount < 1.0) {
            // Add lens distortion back
            float2 factor = (float2)max(1.0f - lens_correction_amount, 0.001f); // FIXME: this is close but wrong
            float2 out_c = (float2)(output_width / 2.0, output_height / 2.0);
            dst_point = undistort_point(dst_point, (f / fov) / factor, out_c, k, lens_correction_amount);
        }

        __global const float *params = &undistortion_params[min((sy + 2), params_count - 1) * 9];

        float _x = dst_point.y * params[1] + params[2] + (dst_point.x * params[0]);
        float _y = dst_point.y * params[4] + params[5] + (dst_point.x * params[3]);
        float _w = dst_point.y * params[7] + params[8] + (dst_point.x * params[6]);

        __global DATA_TYPE *out_pix = &dstptr[x * PIXEL_BYTES + y * output_stride];
        if (_w > 0) {
            float2 pos = (float2)(_x, _y) / _w;

            if (r_limit > 0.0 && length(pos) > r_limit) {
                *out_pix = DATA_CONVERT(bg);
                return;
            }
            float2 uv = distort_point(pos, f, c, k);

            if (edge_repeat) {
                uv = max((float2)(0, 0), min((float2)(width - 1, height - 1), uv));
            } else if (edge_mirror) {
                int rx = round(uv.x);
                int ry = round(uv.y);
                int width3 = (width - 3);
                int height3 = (height - 3);
                if (rx > width3)  uv.x = width3  - (rx - width3);
                if (rx < 3)       uv.x = 3 + width - (width3  + rx);
                if (ry > height3) uv.y = height3 - (ry - height3);
                if (ry < 3)       uv.y = 3 + height - (height3 + ry);
            }

            uv -= S_OFFSET;

            const int shift = (INTERPOLATION >> 2) + 1;
            
            int sx0 = convert_int_sat_rtz(0.5f + uv.x * INTER_TAB_SIZE);
            int sy0 = convert_int_sat_rtz(0.5f + uv.y * INTER_TAB_SIZE);

            int sx = sx0 >> INTER_BITS;
            int sy = sy0 >> INTER_BITS;

            __constant float *coeffs_x = &coeffs[(sx0 & (INTER_TAB_SIZE - 1)) << shift];
            __constant float *coeffs_y = &coeffs[(sy0 & (INTER_TAB_SIZE - 1)) << shift];

            DATA_TYPEF sum = 0;
            int src_index = sy * stride + sx * PIXEL_BYTES;

            #pragma unroll
            for (int yp = 0; yp < INTERPOLATION; ++yp) {
                if (sy + yp >= 0 && sy + yp < height) {
                    DATA_TYPEF xsum = 0.0f;
                    #pragma unroll
                    for (int xp = 0; xp < INTERPOLATION; ++xp) {
                        if (sx + xp >= 0 && sx + xp < width) {
                            xsum += DATA_CONVERTF(*(__global const DATA_TYPE *)&srcptr[src_index + PIXEL_BYTES * xp]) * coeffs_x[xp];
                        } else {
                            xsum += bg * coeffs_x[xp];
                        }
                    }
                    sum += xsum * coeffs_y[yp];
                } else {
                    sum += bg * coeffs_y[yp];
                }
                src_index += stride;
            }

            *out_pix = DATA_CONVERT(sum);
        } else {
            *out_pix = DATA_CONVERT(bg);
        }
    }
}
