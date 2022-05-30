// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::{ PixelType, Stabilization, ComputeParams, FrameTransform, KernelParams, distortion_models::DistortionModel };
use nalgebra::{ Vector4, Matrix3 };
use rayon::{ prelude::ParallelSliceMut, iter::{ ParallelIterator, IndexedParallelIterator } };
use super::distortion_models::GoProSuperview;

pub const COEFFS: [f32; 64+128+256] = [
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
];

impl<T: PixelType> Stabilization<T> {
    // Adapted from OpenCV: initUndistortRectifyMap + remap 
    // https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/calib3d/src/fisheye.cpp#L465-L567
    // https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/imgproc/src/opencl/remap.cl#L390-L498
    pub fn undistort_image_cpu<const I: i32>(pixels: &mut [u8], out_pixels: &mut [u8], params: &KernelParams, distortion_model: &DistortionModel, matrices: &[[f32; 9]]) {
        // From 0-255(JPEG/Full) to 16-235(MPEG/Limited)
        fn remap_colorrange(px: &mut Vector4<f32>, is_y: bool) {
            if is_y { *px *= 0.85882352; } // (235 - 16) / 255
            else    { *px *= 0.87843137; } // (240 - 16) / 255
            px[0] += 16.0;
            px[1] += 16.0;
        }

        fn rotate_and_distort(pos: (f32, f32), idx: usize, params: &KernelParams, matrices: &[[f32; 9]], distortion_model: &DistortionModel, r_limit: f32) -> Option<(f32, f32)> {
            let matrices = matrices[idx];
            let _x = pos.1 * matrices[1] + matrices[2] + (pos.0 * matrices[0]);
            let _y = pos.1 * matrices[4] + matrices[5] + (pos.0 * matrices[3]);
            let _w = pos.1 * matrices[7] + matrices[8] + (pos.0 * matrices[6]);
            if _w > 0.0 {
                let pos = (_x / _w, _y / _w);
                if params.r_limit > 0.0 && (pos.0 * pos.0 + pos.1 * pos.1) > r_limit {
                    return None;
                }
                let mut uv = distortion_model.distort_point(pos, &params.k, 0.0);
                uv = ((uv.0 * params.f[0]) + params.c[0], (uv.1 * params.f[1]) + params.c[1]);

                if (params.flags & 2) == 2 { // GoPro Superview
                    uv = GoProSuperview::to_superview((uv.0 / params.width as f32 - 0.5, uv.1 / params.height as f32 - 0.5));
                    uv = ((uv.0 + 0.5) * params.width as f32, (uv.1 + 0.5) * params.height as f32);
                }

                if params.input_horizontal_stretch > 0.001 { uv.0 /= params.input_horizontal_stretch; }
                if params.input_vertical_stretch   > 0.001 { uv.1 /= params.input_vertical_stretch; }

                return Some(uv);
            }
            return None;
        }

        fn sample_input_at<const I: i32, T: PixelType>(uv: (f32, f32), pixels: &[u8], params: &KernelParams, bg: &Vector4<f32>) -> Vector4<f32> {
            let fix_range = (params.flags & 1) == 1;

            const INTER_BITS: usize = 5;
            const INTER_TAB_SIZE: usize = 1 << INTER_BITS;
            let shift: i32 = (I >> 2) + 1;
            let offset: f32 = [0.0, 1.0, 3.0][I as usize >> 2];
            let ind: usize = [0, 64, 64 + 128][I as usize >> 2];
        
            let u = uv.0 - offset;
            let v = uv.1 - offset;
            
            let sx0 = (u * INTER_TAB_SIZE as f32).round() as i32;
            let sy0 = (v * INTER_TAB_SIZE as f32).round() as i32;
        
            let sx = sx0 >> INTER_BITS;
            let sy = sy0 >> INTER_BITS;
        
            let coeffs_x = &COEFFS[ind + ((sx0 as usize & (INTER_TAB_SIZE - 1)) << shift)..];
            let coeffs_y = &COEFFS[ind + ((sy0 as usize & (INTER_TAB_SIZE - 1)) << shift)..];
            
            let mut sum = Vector4::from_element(0.0);
            let mut src_index = (sy * params.stride + sx * params.bytes_per_pixel) as isize;
        
            for yp in 0..I {
                if sy + yp >= 0 && sy + yp < params.height {
                    let mut xsum = Vector4::<f32>::from_element(0.0);
                    for xp in 0..I {
                        let pixel = if sx + xp >= 0 && sx + xp < params.width {
                            let px1: &T = bytemuck::from_bytes(&pixels[src_index as usize + (params.bytes_per_pixel * xp) as usize..src_index as usize + (params.bytes_per_pixel * (xp + 1)) as usize]); 
                            let mut src_px = PixelType::to_float(*px1);
                            if fix_range {
                                remap_colorrange(&mut src_px, params.bytes_per_pixel == 1)
                            }
                            src_px
                        } else {
                            *bg
                        };
                        xsum += pixel * coeffs_x[xp as usize];
                    }
        
                    sum += xsum * coeffs_y[yp as usize];
                } else {
                    sum += bg * coeffs_y[yp as usize];
                }
                src_index += params.stride as isize;
            }
            sum
        }
        
        let r_limit = params.r_limit * params.r_limit; // Square it so we don't have to do sqrt on the point length

        let bg = Vector4::<f32>::new(params.background[0], params.background[1], params.background[2], params.background[3]);
        let bg_t: T = PixelType::from_float(bg);
        
        let factor = (1.0 - params.lens_correction_amount).max(0.001); // FIXME: this is close but wrong
        let out_c = (params.output_width as f32 / 2.0, params.output_height as f32 / 2.0);
        let out_c2 = (params.output_width as f64, params.output_height as f64);
        let out_f = ((params.f[0] / params.fov / factor), (params.f[1] / params.fov / factor));

        out_pixels.par_chunks_mut(params.output_stride as usize).enumerate().for_each(|(y, row_bytes)| { // Parallel iterator over buffer rows
            row_bytes.chunks_mut(params.bytes_per_pixel as usize).enumerate().for_each(|(x, pix_chunk)| { // iterator over row pixels
                if y < params.output_height as usize && x < params.output_width as usize {
                    assert!(pix_chunk.len() == std::mem::size_of::<T>());

                    let mut out_pos = (x as f32, y as f32);

                    ///////////////////////////////////////////////////////////////////
                    // Calculate source `y` for rolling shutter
                    let mut sy = y;
                    if params.matrix_count > 1 {
                        let idx = params.matrix_count as usize / 2;
                        if let Some(pt) = rotate_and_distort(out_pos, idx, params, matrices, distortion_model, r_limit) {
                            sy = (pt.1.round() as i32).min(params.height).max(0) as usize;
                        }
                    }
                    ///////////////////////////////////////////////////////////////////

                    ///////////////////////////////////////////////////////////////////
                    // Add lens distortion back
                    if params.lens_correction_amount < 1.0 {
                        if (params.flags & 2) == 2 { // Re-add GoPro Superview
                            let mut pt2 = GoProSuperview::from_superview((out_pos.0 as f64 / out_c2.0 - 0.5, out_pos.1 as f64 / out_c2.1 - 0.5));
                            pt2 = ((pt2.0 + 0.5) * out_c2.0, (pt2.1 + 0.5) * out_c2.1);
                            out_pos = (
                                pt2.0 as f32 * (1.0 - params.lens_correction_amount) + (out_pos.0 * params.lens_correction_amount),
                                pt2.1 as f32 * (1.0 - params.lens_correction_amount) + (out_pos.1 * params.lens_correction_amount)
                            );
                        }

                        out_pos = ((out_pos.0 - out_c.0) / out_f.0, (out_pos.1 - out_c.1) / out_f.1);
                        out_pos = distortion_model.undistort_point(out_pos, &params.k, params.lens_correction_amount).unwrap_or_default();
                        out_pos = ((out_pos.0 * out_f.0) + out_c.0, (out_pos.1 * out_f.1) + out_c.1);
                    }
                    ///////////////////////////////////////////////////////////////////
                
                    let pix_out = bytemuck::from_bytes_mut(pix_chunk); // treat this byte chunk as `T`

                    let idx = sy.min(params.matrix_count as usize - 1);
                    if let Some(mut uv) = rotate_and_distort(out_pos, idx, params, matrices, distortion_model, r_limit) {
                        let width_f = params.width as f32;
                        let height_f = params.height as f32;
                        match params.background_mode {
                            1 => { // Edge repeat
                                uv = (
                                    uv.0.max(0.0).min(width_f  - 1.0),
                                    uv.1.max(0.0).min(height_f - 1.0),
                                );
                            },
                            2 => { // Edge mirror
                                let rx = uv.0.round();
                                let ry = uv.1.round();
                                let width3 = width_f - 3.0;
                                let height3 = height_f - 3.0;
                                if rx > width3  { uv.0 = width3  - (rx - width3); }
                                if rx < 3.0     { uv.0 = 3.0 + width_f - (width3  + rx); }
                                if ry > height3 { uv.1 = height3 - (ry - height3); }
                                if ry < 3.0     { uv.1 = 3.0 + height_f - (height3 + ry); }
                            },
                            3 => { // Margin with feather
                                let widthf  = width_f - 1.0;
                                let heightf = height_f - 1.0;

                                let feather = (params.background_margin_feather * heightf).max(0.0001);
                                let mut pt2 = uv;
                                let mut alpha = 1.0;
                                if (uv.0 > widthf - feather) || (uv.0 < feather) || (uv.1 > heightf - feather) || (uv.1 < feather) {
                                    alpha = ((widthf - uv.0).min(heightf - uv.1).min(uv.0).min(uv.1) / feather).min(1.0).max(0.0);
                                    pt2 = (pt2.0 / width_f, pt2.1 / height_f);
                                    pt2 = (
                                        ((pt2.0 - 0.5) * (1.0 - params.background_margin)) + 0.5,
                                        ((pt2.1 - 0.5) * (1.0 - params.background_margin)) + 0.5
                                    );
                                    pt2 = (pt2.0 * width_f, pt2.1 * height_f);
                                }

                                let c1 = sample_input_at::<I, T>(uv, pixels, params, &bg);
                                let c2 = sample_input_at::<I, T>(pt2, pixels, params, &bg);
                                *pix_out = PixelType::from_float(c1 * alpha + c2 * (1.0 - alpha));
                                return;
                            },
                            _ => { }
                        }

                        *pix_out = PixelType::from_float(sample_input_at::<I, T>(uv, pixels, params, &bg));
                    } else {
                        *pix_out = bg_t;
                    }
                }
            });
        });
    }
}

pub fn undistort_points_with_rolling_shutter(distorted: &[(f64, f64)], timestamp_ms: f64, params: &ComputeParams) -> Vec<(f64, f64)> {
    if distorted.is_empty() { return Vec::new(); }
    let (camera_matrix, distortion_coeffs, _p, rotations) = FrameTransform::at_timestamp_for_points(params, distorted, timestamp_ms);

    undistort_points(distorted, camera_matrix, &distortion_coeffs, rotations[0], Some(Matrix3::identity()), Some(rotations), params)
}
pub fn undistort_points_with_params(distorted: &[(f64, f64)], rotation: Matrix3<f64>, p: Option<Matrix3<f64>>, rot_per_point: Option<Vec<Matrix3<f64>>>, params: &ComputeParams) -> Vec<(f64, f64)> {
    let img_dim_ratio = FrameTransform::get_ratio(params);
    let scaled_k = params.camera_matrix * img_dim_ratio;
    
    undistort_points(distorted, scaled_k, &params.distortion_coeffs, rotation, p, rot_per_point, params)
}
// Ported from OpenCV: https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/fisheye.cpp#L321
pub fn undistort_points(distorted: &[(f64, f64)], camera_matrix: Matrix3<f64>, distortion_coeffs: &[f64], rotation: Matrix3<f64>, p: Option<Matrix3<f64>>, rot_per_point: Option<Vec<Matrix3<f64>>>, params: &ComputeParams) -> Vec<(f64, f64)> {
    let f = (camera_matrix[(0, 0)], camera_matrix[(1, 1)]);
    let c = (camera_matrix[(0, 2)], camera_matrix[(1, 2)]);
    let k = distortion_coeffs;
    
    let mut rr = rotation;
    if let Some(p) = p { // PP
        rr = p * rr;
    }

    // TODO: into_par_iter?
    distorted.iter().enumerate().map(|(index, pi)| {
        let mut x = pi.0;
        let mut y = pi.1;
        if params.input_horizontal_stretch > 0.001 { x *= params.input_horizontal_stretch; }
        if params.input_vertical_stretch   > 0.001 { y *= params.input_vertical_stretch; }

        if params.is_superview {
            let pt2 = GoProSuperview::from_superview((x / params.width as f64 - 0.5, y / params.height as f64 - 0.5));
            x = (pt2.0 + 0.5) * params.width as f64;
            y = (pt2.1 + 0.5) * params.height as f64;
        }

        let pw = ((x - c.0) / f.0, (y - c.1) / f.1); // world point

        let rot = rot_per_point.as_ref().and_then(|v| v.get(index)).unwrap_or(&rr);

        if let Some(mut pt) = params.distortion_model.undistort_point(pw, k, 0.0) {
            // reproject
            let pr = rot * nalgebra::Vector3::new(pt.0, pt.1, 1.0); // rotated point optionally multiplied by new camera matrix
            pt = (pr[0] / pr[2], pr[1] / pr[2]);

            if params.lens_correction_amount < 1.0 {
                let mut out_c = c; // (params.output_width as f64 / 2.0, params.output_height as f64 / 2.0);
                if params.input_horizontal_stretch > 0.001 { out_c.0 /= params.input_horizontal_stretch; }
                if params.input_vertical_stretch   > 0.001 { out_c.1 /= params.input_vertical_stretch; }

                pt = ((pt.0 - out_c.0) / f.0, (pt.1 - out_c.1) / f.1);
                pt = params.distortion_model.distort_point(pt, k, params.lens_correction_amount);
                pt = ((pt.0 * f.0) + out_c.0, (pt.1 * f.1) + out_c.1);

                if params.is_superview {
                    // TODO: This calculation is wrong but it somewhat works
                    let size = (params.width as f64, params.height as f64);
                    pt = (pt.0 / size.0 - 0.5, pt.1 / size.1 - 0.5);
                    pt.0 *= 1.0 + (0.15 * (1.0 - params.lens_correction_amount));
                    pt = ((pt.0 + 0.5) * size.0, (pt.1 + 0.5) * size.1);
                }
            }
            pt
        } else {
            (-1000000.0, -1000000.0)
        }
    }).collect()
}
