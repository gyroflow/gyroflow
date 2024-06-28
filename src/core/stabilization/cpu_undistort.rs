// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use crate::gpu::{ Buffers, BufferSource };

use super::{ PixelType, Stabilization, ComputeParams, FrameTransform, KernelParams, distortion_models::DistortionModel };
use nalgebra::{ Vector4, Matrix3 };
use rayon::{ prelude::ParallelSliceMut, iter::{ ParallelIterator, IndexedParallelIterator } };
use crate::util::map_coord;

pub const COEFFS: [f32; 64+128+256 + 9*4 + 4] = [
    // Bilinear
    // offset 0
    1.000000, 0.000000, 0.968750, 0.031250, 0.937500, 0.062500, 0.906250, 0.093750, 0.875000, 0.125000, 0.843750, 0.156250,
    0.812500, 0.187500, 0.781250, 0.218750, 0.750000, 0.250000, 0.718750, 0.281250, 0.687500, 0.312500, 0.656250, 0.343750,
    0.625000, 0.375000, 0.593750, 0.406250, 0.562500, 0.437500, 0.531250, 0.468750, 0.500000, 0.500000, 0.468750, 0.531250,
    0.437500, 0.562500, 0.406250, 0.593750, 0.375000, 0.625000, 0.343750, 0.656250, 0.312500, 0.687500, 0.281250, 0.718750,
    0.250000, 0.750000, 0.218750, 0.781250, 0.187500, 0.812500, 0.156250, 0.843750, 0.125000, 0.875000, 0.093750, 0.906250,
    0.062500, 0.937500, 0.031250, 0.968750,

    // Bicubic
    // offset 64
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
    // offset 192
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
     0.998265, -0.027053,  0.009625, -0.002981,

    // Colors
    // offset 448
    0.0,   0.0,   0.0,     0.0, // None
    255.0, 0.0,   0.0,   255.0, // Red
    0.0,   255.0, 0.0,   255.0, // Green
    0.0,   0.0,   255.0, 255.0, // Blue
    254.0, 251.0, 71.0,  255.0, // Yellow
    200.0, 200.0, 0.0,   255.0, // Yellow2
    255.0, 0.0,   255.0, 255.0, // Magenta
    0.0,   128.0, 255.0, 255.0, // Blue2
    0.0,   200.0, 200.0, 255.0, // Blue3

    // Alphas
    // offset 484
    1.0, 0.75, 0.50, 0.25,
];

// const COLORS: [Vector4<f32>; 9] = [
//     Vector4::new(0.0,   0.0,   0.0,     0.0), // None
//     Vector4::new(255.0, 0.0,   0.0,   255.0), // Red
//     Vector4::new(0.0,   255.0, 0.0,   255.0), // Green
//     Vector4::new(0.0,   0.0,   255.0, 255.0), // Blue
//     Vector4::new(254.0, 251.0, 71.0,  255.0), // Yellow
//     Vector4::new(200.0, 200.0, 0.0,   255.0), // Yellow2
//     Vector4::new(255.0, 0.0,   255.0, 255.0), // Magenta
//     Vector4::new(0.0,   128.0, 255.0, 255.0), // Blue2
//     Vector4::new(0.0,   200.0, 200.0, 255.0)  // Blue3
// ];
// const ALPHAS: [f32; 4] = [ 1.0, 0.75, 0.50, 0.25 ];

impl Stabilization {
    pub fn undistort_image_cpu_spirv<T: PixelType>(buffers: &mut Buffers, params: &KernelParams, distortion_model: &DistortionModel, digital_lens: Option<&DistortionModel>, matrices: &[[f32; 14]], drawing: &[u8]) -> bool {
        if let BufferSource::Cpu { buffer: input } = &mut buffers.input.data {
            if let BufferSource::Cpu { buffer: output } = &mut buffers.output.data {
                if buffers.output.size.2 <= 0 {
                    log::error!("buffers.output_size: {:?}", buffers.output.size);
                    return false;
                }

                output.par_chunks_mut(buffers.output.size.2).enumerate().for_each(|(y, row_bytes)| { // Parallel iterator over buffer rows
                    row_bytes.chunks_mut(params.bytes_per_pixel as usize).enumerate().for_each(|(x, pix_chunk)| { // iterator over row pixels
                        let matrices2: &[f32] = unsafe { std::slice::from_raw_parts(matrices.as_ptr() as *const f32, matrices.len() * 14 ) };
                        let params2: stabilize_spirv::KernelParams  = unsafe { std::mem::transmute(*params) };
                        let drawing2: &[u32]  = unsafe { std::slice::from_raw_parts(drawing.as_ptr() as *const u32, drawing.len() / 4 ) };

                        let color = stabilize_spirv::undistort(
                            stabilize_spirv::glam::vec2(x as f32, y as f32),
                            &params2,
                            matrices2,
                            &COEFFS,
                            &[],
                            drawing2,
                            &(input, T::to_float_glam),
                            0.0,
                            params.interpolation as _,
                            params.distortion_model as u32,
                            params.digital_lens as u32,
                            params.flags as u32
                        );

                        let pix_out: &mut T = bytemuck::from_bytes_mut(pix_chunk); // treat this byte chunk as `T`
                        *pix_out = PixelType::from_float_glam(color);
                    });
                });
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    pub fn rotate_and_distort(pos: (f32, f32), idx: usize, params: &KernelParams, matrices: &[[f32; 14]], distortion_model: &DistortionModel, digital_lens: Option<&DistortionModel>, r_limit_sq: f32, mesh_data: &[f64]) -> Option<(f32, f32)> {
        let matrices = matrices[idx];
        let _x = (pos.0 * matrices[0]) + (pos.1 * matrices[1]) + matrices[2] + params.translation3d[0];
        let _y = (pos.0 * matrices[3]) + (pos.1 * matrices[4]) + matrices[5] + params.translation3d[1];
        let mut _w = (pos.0 * matrices[6]) + (pos.1 * matrices[7]) + matrices[8] + params.translation3d[2];
        if _w > 0.0 {
            if r_limit_sq > 0.0 && (_x.powi(2) + _y.powi(2)) > r_limit_sq * _w {
                return None;
            }

            if params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0 {
                if _w != 0.0 {
                    let r = (_x.powi(2) + _y.powi(2)).sqrt() / _w;
                    let sin_theta_d = (r / (1.0 + r * r).sqrt()) * params.light_refraction_coefficient;
                    let r_d = sin_theta_d / (1.0 - sin_theta_d * sin_theta_d).sqrt();
                    if r_d != 0.0 {
                        _w *= r / r_d;
                    }
                }
            }

            let mut uv = distortion_model.distort_point(_x, _y, _w, &params);
            uv = (uv.0 * params.f[0], uv.1 * params.f[1]);

            if matrices[9] != 0.0 || matrices[10] != 0.0 || matrices[11] != 0.0 || matrices[12] != 0.0 || matrices[13] != 0.0 {
                let ang_rad = matrices[11];
                let cos_a = (-ang_rad).cos();
                let sin_a = (-ang_rad).sin();
                uv = (
                    cos_a * uv.0 - sin_a * uv.1 - matrices[9]  + matrices[12],
                    sin_a * uv.0 + cos_a * uv.1 - matrices[10] + matrices[13]
                );
            }

            uv = (uv.0 + params.c[0], uv.1 + params.c[1]);

            if !mesh_data.is_empty() && mesh_data[0] != 0.0 {
                let mesh_size = (mesh_data[2], mesh_data[3]);
                let origin    = (mesh_data[4] as f32, mesh_data[5] as f32);
                let crop_size = (mesh_data[6] as f32, mesh_data[7] as f32);

                if (params.flags & 128) == 128 { uv.1 = params.height as f32 - uv.1; } // framebuffer inverted

                uv.0 = map_coord(uv.0, 0.0, params.width  as f32, origin.0, origin.0 + crop_size.0);
                uv.1 = map_coord(uv.1, 0.0, params.height as f32, origin.1, origin.1 + crop_size.1);

                let new_pos = crate::gyro_source::interpolate_mesh(uv.0 as f64, uv.1 as f64, (mesh_size.0, mesh_size.1), mesh_data);

                uv.0 = map_coord(new_pos.x as f32, origin.0, origin.0 + crop_size.0, 0.0, params.width  as f32);
                uv.1 = map_coord(new_pos.y as f32, origin.1, origin.1 + crop_size.1, 0.0, params.height as f32);

                if (params.flags & 128) == 128 { uv.1 = params.height as f32 - uv.1; } // framebuffer inverted
            }

            if (params.flags & 2) == 2 { // Has digital lens
                if let Some(digital) = digital_lens {
                    uv = digital.distort_point(uv.0, uv.1, 1.0, params);
                }
            }

            if params.input_horizontal_stretch > 0.001 { uv.0 /= params.input_horizontal_stretch; }
            if params.input_vertical_stretch   > 0.001 { uv.1 /= params.input_vertical_stretch; }

            return Some(uv);
        }
        return None;
    }

    // Adapted from OpenCV: initUndistortRectifyMap + remap
    // https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/calib3d/src/fisheye.cpp#L465-L567
    // https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/imgproc/src/opencl/remap.cl#L390-L498
    pub fn undistort_image_cpu<const I: i32, T: PixelType>(buffers: &mut Buffers, params: &KernelParams, distortion_model: &DistortionModel, digital_lens: Option<&DistortionModel>, matrices: &[[f32; 14]], drawing: &[u8], mesh_data: &[f32]) -> bool {
        // #[cold]
        // fn draw_pixel(pix: &mut Vector4<f32>, x: i32, y: i32, is_input: bool, width: i32, params: &KernelParams, drawing: &[u8]) {
        //     if drawing.is_empty() || (params.flags & 8) == 0 { return; }
        //     let pos = ((y as f32 / params.canvas_scale).floor() * (width as f32) + (x as f32 / params.canvas_scale).floor()).round() as usize;
        //     if let Some(&data) = drawing.get(pos) {
        //         if data > 0 {
        //             let color = (data & 0xF8) >> 3;
        //             let alpha = (data & 0x06) >> 1;
        //             let stage = data & 1;
        //             if ((stage == 0 && is_input) || (stage == 1 && !is_input)) && color < 9 {
        //                 let colorf = COLORS[color as usize];
        //                 let alphaf = ALPHAS[alpha as usize];
        //                 *pix = colorf * alphaf + *pix * (1.0 - alphaf);
        //                 pix.w = 255.0;
        //             }
        //         }
        //     }
        // }

        // From 0-255(JPEG/Full) to 16-235(MPEG/Limited)
        #[cold]
        fn remap_colorrange(px: &mut Vector4<f32>, is_y: bool) {
            if is_y { *px *= 0.85882352; } // (235 - 16) / 255
            else    { *px *= 0.87843137; } // (240 - 16) / 255
            px[0] += 16.0;
            px[1] += 16.0;
        }

        fn rotate_point(pos: (f32, f32), angle: f32, origin: (f32, f32)) -> (f32, f32) {
             return (angle.cos() * (pos.0 - origin.0) - angle.sin() * (pos.1 - origin.1) + origin.0,
                     angle.sin() * (pos.0 - origin.0) + angle.cos() * (pos.1 - origin.1) + origin.1);
        }

        fn sample_input_at<const I: i32, T: PixelType>(mut uv: (f32, f32), input: &[u8], params: &KernelParams, bg: &Vector4<f32>, _drawing: &[u8]) -> Vector4<f32> {
            const INTER_BITS: usize = 5;
            const INTER_TAB_SIZE: usize = 1 << INTER_BITS;
            let shift: i32 = (I >> 2) + 1;
            let offset: f32 = [0.0, 1.0, 3.0][I as usize >> 2];
            let ind: usize = [0, 64, 64 + 128][I as usize >> 2];

            if params.input_rotation != 0.0 {
                uv = rotate_point(uv, params.input_rotation * (std::f32::consts::PI / 180.0), (params.width as f32 / 2.0, params.height as f32 / 2.0));
            }

            uv = (
                map_coord(uv.0, 0.0, params.width  as f32, params.source_rect[0] as f32, (params.source_rect[0] + params.source_rect[2]) as f32),
                map_coord(uv.1, 0.0, params.height as f32, params.source_rect[1] as f32, (params.source_rect[1] + params.source_rect[3]) as f32)
            );

            let u = uv.0 - offset;
            let v = uv.1 - offset;

            let sx0 = (u * INTER_TAB_SIZE as f32).round() as i32;
            let sy0 = (v * INTER_TAB_SIZE as f32).round() as i32;

            let sx = sx0 >> INTER_BITS;
            let sy = sy0 >> INTER_BITS;

            let coeffs_x = &COEFFS[ind + ((sx0 as usize & (INTER_TAB_SIZE - 1)) << shift)..];
            let coeffs_y = &COEFFS[ind + ((sy0 as usize & (INTER_TAB_SIZE - 1)) << shift)..];

            let mut sum = Vector4::from_element(0.0);
            let mut src_index = sy as isize * params.stride as isize + sx as isize * params.bytes_per_pixel as isize;

            for yp in 0..I {
                if sy + yp >= params.source_rect[1] && sy + yp < params.source_rect[1] + params.source_rect[3] {
                    let mut xsum = Vector4::<f32>::from_element(0.0);
                    for xp in 0..I {
                        let pixel = if sx + xp >= params.source_rect[0] && sx + xp < params.source_rect[0] + params.source_rect[2] {
                            let px1: &T = bytemuck::from_bytes(&input[src_index as usize + (params.bytes_per_pixel * xp) as usize..src_index as usize + (params.bytes_per_pixel * (xp + 1)) as usize]);
                            let src_px = PixelType::to_float(*px1);
                            // draw_pixel(&mut src_px, sx + xp, sy + yp, true, params.width, params, drawing);
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
            Vector4::new(
                sum.x.min(params.pixel_value_limit),
                sum.y.min(params.pixel_value_limit),
                sum.z.min(params.pixel_value_limit),
                sum.w.min(params.pixel_value_limit),
            )
        }

        if let BufferSource::Cpu { buffer: input } = &mut buffers.input.data {
            if let BufferSource::Cpu { buffer: output } = &mut buffers.output.data {
                let r_limit_sq = params.r_limit * params.r_limit; // Square it so we don't have to do sqrt on the point length

                let bg = Vector4::<f32>::new(params.background[0], params.background[1], params.background[2], params.background[3]) * params.max_pixel_value;
                let bg_t: T = PixelType::from_float(bg);

                let factor = (1.0 - params.lens_correction_amount).max(0.001); // FIXME: this is close but wrong
                let out_c = (params.output_width as f32 / 2.0, params.output_height as f32 / 2.0);
                let out_f = ((params.f[0] / params.fov / factor), (params.f[1] / params.fov / factor));

                // let drawing_enabled = !drawing.is_empty() && (params.flags & 8) == 8;
                let fill_bg = (params.flags & 4) == 4;
                let fix_range = (params.flags & 1) == 1;
                let is_y = params.bytes_per_pixel == 1;
                if buffers.output.size.2 <= 0 {
                    log::error!("buffers.output_size: {:?}", buffers.output.size);
                    return false;
                }

                let mesh_data = mesh_data.iter().map(|x| *x as f64).collect::<Vec<f64>>();

                output.par_chunks_mut(buffers.output.size.2).enumerate().for_each(|(y, row_bytes)| { // Parallel iterator over buffer rows
                    row_bytes.chunks_mut(params.bytes_per_pixel as usize).enumerate().for_each(|(x, pix_chunk)| { // iterator over row pixels

                        let mut out_pos = (
                            map_coord(x as f32, params.output_rect[0] as f32, (params.output_rect[0] + params.output_rect[2]) as f32, 0.0, params.output_width  as f32),
                            map_coord(y as f32, params.output_rect[1] as f32, (params.output_rect[1] + params.output_rect[3]) as f32, 0.0, params.output_height as f32)
                        );

                        if out_pos.0 >= 0.0 && out_pos.1 >= 0.0 && (out_pos.0 as i32) < params.output_width && (out_pos.1 as i32) < params.output_height {
                            assert!(pix_chunk.len() == std::mem::size_of::<T>());

                            // let p = out_pos;
                            let mut pixel = bg;

                            out_pos.0 += params.translation2d[0];
                            out_pos.1 += params.translation2d[1];

                            let pix_out = bytemuck::from_bytes_mut(pix_chunk); // treat this byte chunk as `T`

                            if fill_bg {
                                *pix_out = bg_t;
                                return;
                            }

                            ///////////////////////////////////////////////////////////////////
                            // Add lens distortion back
                            if params.lens_correction_amount < 1.0 {
                                let mut new_out_pos = out_pos;

                                if (params.flags & 2) == 2 { // Has digial lens
                                    if let Some(digital) = digital_lens {
                                        if let Some(pt) = digital.undistort_point(new_out_pos, params) {
                                            new_out_pos = pt;
                                        }
                                    }
                                }

                                new_out_pos = ((new_out_pos.0 - out_c.0) / out_f.0, (new_out_pos.1 - out_c.1) / out_f.1);
                                new_out_pos = distortion_model.undistort_point(new_out_pos, &params).unwrap_or_default();
                                if params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0 {
                                    let r = (new_out_pos.0.powi(2) + new_out_pos.1.powi(2)).sqrt();
                                    if r != 0.0 {
                                        let sin_theta_d = (r / (1.0 + r * r).sqrt()) / params.light_refraction_coefficient;
                                        let r_d = sin_theta_d / (1.0 - sin_theta_d * sin_theta_d).sqrt();
                                        let factor = r_d / r;
                                        new_out_pos.0 *= factor;
                                        new_out_pos.1 *= factor;
                                    }
                                }
                                new_out_pos = ((new_out_pos.0 * out_f.0) + out_c.0, (new_out_pos.1 * out_f.1) + out_c.1);

                                out_pos = (
                                    new_out_pos.0 * (1.0 - params.lens_correction_amount) + (out_pos.0 * params.lens_correction_amount),
                                    new_out_pos.1 * (1.0 - params.lens_correction_amount) + (out_pos.1 * params.lens_correction_amount),
                                );
                            }
                            ///////////////////////////////////////////////////////////////////

                            ///////////////////////////////////////////////////////////////////
                            // Calculate source `y` for rolling shutter
                            let mut sy = if (params.flags & 16) == 16 { // Horizontal RS
                                (out_pos.0.round() as i32).min(params.width).max(0) as usize
                            } else {
                                (out_pos.1.round() as i32).min(params.height).max(0) as usize
                            };
                            if params.matrix_count > 1 {
                                let idx = params.matrix_count as usize / 2;
                                if let Some(pt) = Self::rotate_and_distort(out_pos, idx, params, matrices, distortion_model, digital_lens, r_limit_sq, &mesh_data) {
                                    if (params.flags & 16) == 16 { // Horizontal RS
                                        sy = (pt.0.round() as i32).min(params.width).max(0) as usize;
                                    } else {
                                        sy = (pt.1.round() as i32).min(params.height).max(0) as usize;
                                    }
                                }
                            }
                            ///////////////////////////////////////////////////////////////////

                            let idx = sy.min(params.matrix_count as usize - 1);
                            if let Some(mut uv) = Self::rotate_and_distort(out_pos, idx, params, matrices, distortion_model, digital_lens, r_limit_sq, &mesh_data) {
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

                                        let c1 = sample_input_at::<I, T>(uv, input, params, &bg, drawing);
                                        let c2 = sample_input_at::<I, T>(pt2, input, params, &bg, drawing);
                                        pixel = c1 * alpha + c2 * (1.0 - alpha);
                                        // draw_pixel(&mut pixel, p.0 as i32, p.1 as i32, false, params.output_width, params, drawing);
                                        if fix_range {
                                            remap_colorrange(&mut pixel, is_y)
                                        }
                                        *pix_out = PixelType::from_float(pixel);
                                        return;
                                    },
                                    _ => { }
                                }

                                pixel = sample_input_at::<I, T>(uv, input, params, &bg, drawing);
                            }
                            // draw_pixel(&mut pixel, p.0 as i32, p.1 as i32, false, params.output_width, params, drawing);

                            if fix_range {
                                remap_colorrange(&mut pixel, is_y)
                            }
                            *pix_out = PixelType::from_float(pixel);
                        }
                    });
                });
                true
            } else {
                false
            }
        } else {
            false
        }
    }
}

pub fn undistort_points_with_rolling_shutter(distorted: &[(f32, f32)], timestamp_ms: f64, frame: Option<usize>, params: &ComputeParams, lens_correction_amount: f64, use_fovs: bool) -> Vec<(f32, f32)> {
    if distorted.is_empty() { return Vec::new(); }
    let (camera_matrix, distortion_coeffs, _p, rotations, is, mesh) = FrameTransform::at_timestamp_for_points(params, distorted, timestamp_ms, frame, use_fovs);

    undistort_points(distorted, camera_matrix, &distortion_coeffs, rotations[0], Some(Matrix3::identity()), Some(rotations), params, lens_correction_amount, timestamp_ms, is, mesh)
}
pub fn undistort_points_for_optical_flow(distorted: &[(f32, f32)], timestamp_us: i64, params: &ComputeParams, points_dims: (u32, u32)) -> Vec<(f32, f32)> {
    let img_dim_ratio = points_dims.0 as f64 / params.video_width.max(1) as f64;//FrameTransform::get_ratio(params);

    let (camera_matrix, distortion_coeffs, _, _, _, _) = FrameTransform::get_lens_data_at_timestamp(params, timestamp_us as f64 / 1000.0);

    let scaled_k = camera_matrix * img_dim_ratio;

    undistort_points(distorted, scaled_k, &distortion_coeffs, Matrix3::identity(), None, None, params, 1.0, timestamp_us as f64 / 1000.0, None, None)
}
// Ported from OpenCV: https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/fisheye.cpp#L321
pub fn undistort_points(distorted: &[(f32, f32)], camera_matrix: Matrix3<f64>, distortion_coeffs: &[f64; 12], rotation: Matrix3<f64>, p: Option<Matrix3<f64>>, rot_per_point: Option<Vec<Matrix3<f64>>>, params: &ComputeParams, lens_correction_amount: f64, timestamp_ms: f64, shift_per_point: Option<Vec<(f32, f32, f32, f32, f32)>>, mesh: Option<Vec<f64>>) -> Vec<(f32, f32)> {
    let f = (camera_matrix[(0, 0)] as f32, camera_matrix[(1, 1)] as f32);
    let c = (camera_matrix[(0, 2)] as f32, camera_matrix[(1, 2)] as f32);

    let mut rr = rotation;
    if let Some(p) = p { // PP
        rr = p * rr;
    }

    let light_refraction_coefficient = params.keyframes.value_at_video_timestamp(&crate::KeyframeType::LightRefractionCoeff, timestamp_ms).unwrap_or(params.light_refraction_coefficient) as f32;

    // TODO more params
    let kernel_params = KernelParams {
        width : params.width as i32,
        height: params.height as i32,
        output_width: params.output_width as i32,
        output_height: params.output_height as i32,
        f: [f.0, f.1],
        c: [c.0, c.1],
        k: distortion_coeffs.iter().map(|x| *x as f32).collect::<Vec<_>>().try_into().unwrap(),
        light_refraction_coefficient,

        ..Default::default()
    };

    // TODO: into_par_iter?
    distorted.iter().enumerate().map(|(index, pi)| {
        let mut x = pi.0;
        let mut y = pi.1;
        if params.lens.input_horizontal_stretch > 0.001 { x *= params.lens.input_horizontal_stretch as f32; }
        if params.lens.input_vertical_stretch   > 0.001 { y *= params.lens.input_vertical_stretch as f32; }

        if let Some(digital) = &params.digital_lens {
            if let Some(pt2) = digital.undistort_point((x, y), &kernel_params) {
                x = pt2.0;
                y = pt2.1;
            }
        }

        if let Some(mesh_data) = &mesh {
            let mesh_size = (mesh_data[2], mesh_data[3]);
            let origin    = (mesh_data[4] as f32, mesh_data[5] as f32);
            let crop_size = (mesh_data[6] as f32, mesh_data[7] as f32);

            x = map_coord(x, 0.0, params.width  as f32, origin.0, origin.0 + crop_size.0);
            y = map_coord(y, 0.0, params.height as f32, origin.1, origin.1 + crop_size.1);

            let new_pos = crate::gyro_source::interpolate_mesh(x as f64, y as f64, (mesh_size.0, mesh_size.1), &mesh_data);

            x = map_coord(new_pos.x as f32, origin.0, origin.0 + crop_size.0, 0.0, params.width  as f32);
            y = map_coord(new_pos.y as f32, origin.1, origin.1 + crop_size.1, 0.0, params.height as f32);
        }
        if let Some(shift) = shift_per_point.as_ref().and_then(|v| v.get(index)) {
            let ang_rad = shift.2;
            let cos_a = ang_rad.cos();
            let sin_a = ang_rad.sin();
            x = x - c.0 - shift.3 + shift.0;
            y = y - c.1 - shift.4 + shift.1;
            x = cos_a * x - sin_a * y + c.0;
            y = sin_a * x + cos_a * y + c.1;
        }

        let pw = ((x - c.0) / f.0, (y - c.1) / f.1); // world point

        let rot = nalgebra::convert::<nalgebra::Matrix3<f64>, nalgebra::Matrix3<f32>>(*rot_per_point.as_ref().and_then(|v| v.get(index)).unwrap_or(&rr));

        if let Some(mut pt) = params.distortion_model.undistort_point(pw, &kernel_params) {
            if kernel_params.light_refraction_coefficient != 1.0 && kernel_params.light_refraction_coefficient > 0.0 {
                let r = (pt.0.powi(2) + pt.1.powi(2)).sqrt();
                if r != 0.0 {
                    let sin_theta_d = (r / (1.0 + r * r).sqrt()) / kernel_params.light_refraction_coefficient;
                    let r_d = sin_theta_d / (1.0 - sin_theta_d * sin_theta_d).sqrt();
                    let factor = r_d / r;
                    pt.0 *= factor;
                    pt.1 *= factor;
                }
            }

            // reproject
            let pr = rot * nalgebra::Vector3::new(pt.0, pt.1, 1.0); // rotated point optionally multiplied by new camera matrix
            pt = (pr[0] / pr[2], pr[1] / pr[2]);

            if lens_correction_amount < 1.0 {
                let mut out_c = (params.output_width as f32 / 2.0, params.output_height as f32 / 2.0);
                if params.lens.input_horizontal_stretch > 0.001 { out_c.0 /= params.lens.input_horizontal_stretch as f32; }
                if params.lens.input_vertical_stretch   > 0.001 { out_c.1 /= params.lens.input_vertical_stretch as f32; }

                let mut new_pt = pt;
                new_pt = ((new_pt.0 - out_c.0) / f.0, (new_pt.1 - out_c.1) / f.1);
                let mut _w = 1.0;
                if kernel_params.light_refraction_coefficient != 1.0 && kernel_params.light_refraction_coefficient > 0.0 {
                    let r = (new_pt.0.powi(2) + new_pt.1.powi(2)).sqrt() / _w;
                    let sin_theta_d = (r / (1.0 + r * r).sqrt()) * kernel_params.light_refraction_coefficient;
                    let r_d = sin_theta_d / (1.0 - sin_theta_d * sin_theta_d).sqrt();
                    if r_d != 0.0 {
                        _w *= r / r_d;
                    }
                }
                new_pt = params.distortion_model.distort_point(new_pt.0, new_pt.1, _w, &kernel_params); // TODO: z?
                new_pt = ((new_pt.0 * f.0) + out_c.0, (new_pt.1 * f.1) + out_c.1);

                if let Some(digital) = &params.digital_lens {
                    new_pt = digital.distort_point(new_pt.0, new_pt.1, 1.0, &kernel_params);
                    if digital.id() == "gopro_superview" || digital.id() == "gopro_hyperview" {
                        // TODO: This calculation is wrong but it somewhat works
                        let size = (params.width as f32, params.height as f32);
                        new_pt = (new_pt.0 / size.0 - 0.5, new_pt.1 / size.1 - 0.5);
                        if digital.id() == "gopro_superview" {
                            new_pt.0 *= 0.91;
                        } else if digital.id() == "gopro_hyperview"{
                            new_pt.0 *= 0.81;
                        }
                        new_pt = ((new_pt.0 + 0.5) * size.0, (new_pt.1 + 0.5) * size.1);
                    }
                }

                pt = (
                    new_pt.0 * (1.0 - lens_correction_amount as f32) + (pt.0 * lens_correction_amount as f32),
                    new_pt.1 * (1.0 - lens_correction_amount as f32) + (pt.1 * lens_correction_amount as f32),
                );
            }
            pt
        } else {
            (-1000000.0, -1000000.0)
        }
    }).collect()
}
