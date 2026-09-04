// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use crate::gpu::{ Buffers, BufferSource };

use super::{ PixelType, Stabilization, ComputeParams, FrameTransform, KernelParams, distortion_models::DistortionModel };
use nalgebra::{ Vector2, Vector3, Vector4, Matrix3 };
use rayon::{ prelude::ParallelSliceMut, iter::{ ParallelIterator, IndexedParallelIterator } };
use crate::util::map_coord;

pub const INVALID_POINT: (f32, f32) = (-1_000_000.0, -1_000_000.0);

pub fn is_valid_point(p: (f32, f32)) -> bool {
    p.0.is_finite() && p.1.is_finite() && p.0 > INVALID_POINT.0 / 2.0 && p.1 > INVALID_POINT.1 / 2.0
}

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
            if params.r_limit > 0.0 && (_x.powi(2) + _y.powi(2)) > r_limit_sq * (_w * _w) {
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
                // The camera applies the sensor roll before the sensor/lens shift, so undo the shift first and then the roll
                let ang_rad = matrices[11];
                let cos_a = (-ang_rad).cos();
                let sin_a = (-ang_rad).sin();
                let shifted = (uv.0 - matrices[9] + matrices[12], uv.1 - matrices[10] + matrices[13]);
                uv = (
                    cos_a * shifted.0 - sin_a * shifted.1,
                    sin_a * shifted.0 + cos_a * shifted.1
                );
            }

            uv = (uv.0 + params.c[0], uv.1 + params.c[1]);

            if !mesh_data.is_empty() && mesh_data[0] > 10.0 {
                let mesh_size = (mesh_data[3], mesh_data[4]);
                let origin    = (mesh_data[5] as f32, mesh_data[6] as f32);
                let crop_size = (mesh_data[7] as f32, mesh_data[8] as f32);

                if (params.flags & 128) == 128 { uv.1 = params.height as f32 - uv.1; } // framebuffer inverted

                uv.0 = map_coord(uv.0, 0.0, params.width  as f32, origin.0, origin.0 + crop_size.0);
                uv.1 = map_coord(uv.1, 0.0, params.height as f32, origin.1, origin.1 + crop_size.1);

                let q = (uv.0 as f64, uv.1 as f64);
                let mut new_pos = crate::gyro_source::interpolate_mesh(q.0, q.1, (mesh_size.0, mesh_size.1), mesh_data);
                // The 9x9 inverse mesh is only approximate for large warps, refine against the camera's forward mesh (fwd(p) = q).
                // The block is only there when the mesh needs it (`sony::MESH_REFINE_THRESHOLD_PX`), and a first correction
                // that is already tiny leaves nothing for a second one (`sony::MESH_REFINE_SKIP_PX`)
                let o = mesh_data[0] as usize;
                let fwd = o + 4 + 2 * (mesh_data[o].max(0.0) as usize);
                if mesh_data.len() > fwd + 9 && mesh_data[fwd] > 10.0 {
                    let skip_sq = crate::gyro_source::MESH_REFINE_SKIP_PX * crate::gyro_source::MESH_REFINE_SKIP_PX;
                    for _ in 0..2 {
                        let f = crate::gyro_source::interpolate_mesh(new_pos.x, new_pos.y, (mesh_size.0, mesh_size.1), &mesh_data[fwd..]);
                        let (dx, dy) = (q.0 - f.x, q.1 - f.y);
                        new_pos.x += dx;
                        new_pos.y += dy;
                        if dx * dx + dy * dy < skip_sq { break; }
                    }
                }

                uv.0 = map_coord(new_pos.x as f32, origin.0, origin.0 + crop_size.0, 0.0, params.width  as f32);
                uv.1 = map_coord(new_pos.y as f32, origin.1, origin.1 + crop_size.1, 0.0, params.height as f32);

                if (params.flags & 128) == 128 { uv.1 = params.height as f32 - uv.1; } // framebuffer inverted
            }

            // FocalPlaneDistortion
            if !mesh_data.is_empty() && mesh_data[0] > 0.0 && mesh_data[mesh_data[0] as usize] > 0.0 {
                let o = mesh_data[0] as usize; // offset to focal plane distortion data

                let mesh_size = (mesh_data[3], mesh_data[4]);
                let origin    = (mesh_data[5] as f32, mesh_data[6] as f32);
                let crop_size = (mesh_data[7] as f32, mesh_data[8] as f32);
                let stblz_grid = if mesh_data[o + 2] > 0.0 { mesh_data[o + 2] } else { mesh_size.1 / 8.0 }; // band height comes with the table

                if (params.flags & 128) == 128 { uv.1 = params.height as f32 - uv.1; } // framebuffer inverted

                uv.0 = map_coord(uv.0, 0.0, params.width  as f32, origin.0, origin.0 + crop_size.0);
                uv.1 = map_coord(uv.1, 0.0, params.height as f32, origin.1, origin.1 + crop_size.1);

                let idx = (uv.1 as f64 / stblz_grid).floor().max(0.0).min(7.0) as usize;
                let delta = uv.1 as f64 - stblz_grid * idx as f64;
                uv.0 -= (mesh_data[o + 4 + idx * 2 + 0] * delta) as f32;
                uv.1 -= (mesh_data[o + 4 + idx * 2 + 1] * delta) as f32;
                for j in 0..idx {
                    uv.0 -= (mesh_data[o + 4 + j * 2 + 0] * stblz_grid) as f32;
                    uv.1 -= (mesh_data[o + 4 + j * 2 + 1] * stblz_grid) as f32;
                }

                uv.0 = map_coord(uv.0, origin.0, origin.0 + crop_size.0, 0.0, params.width  as f32);
                uv.1 = map_coord(uv.1, origin.1, origin.1 + crop_size.1, 0.0, params.height as f32);

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

        fn rotate_point(pos: (f32, f32), angle: f32, origin: (f32, f32), origin2: (f32, f32)) -> (f32, f32) {
             return (angle.cos() * (pos.0 - origin.0) - angle.sin() * (pos.1 - origin.1) + origin2.0,
                     angle.sin() * (pos.0 - origin.0) + angle.cos() * (pos.1 - origin.1) + origin2.1);
        }

        ////////////////////////////// EWA (Elliptical Weighted Average) CubicBC sampling //////////////////////////////
        // Keys Cubic Filter Family https://imagemagick.org/Usage/filter/#robidoux
        // https://github.com/ImageMagick/ImageMagick/blob/main/MagickCore/resize.c

        // Gives a bounding box in the source image containing pixels that cover a circle of radius 2 completely in both the source and destination images
        fn affine_bbox(jac: &Vector4<f32>) -> Vector2<f32> {
            const MAX_SUPPORT: f32 = 64.0;
            return Vector2::new(
                (2.0 * ((jac.x + jac.y).abs().max((jac.x - jac.y).abs()).max(1.0))).min(MAX_SUPPORT),
                (2.0 * ((jac.z + jac.w).abs().max((jac.z - jac.w).abs()).max(1.0))).min(MAX_SUPPORT)
            );
        }
        // Computes minimum area ellipse which covers a unit circle in both the source and destination image
        fn clamped_ellipse(jac: &Vector4<f32>) -> Vector3<f32> {
            // find ellipse
            let f0 = (jac.x * jac.w - jac.y * jac.z).abs();
            let f = (f0 * f0).max(0.1);
            let a = (jac.z * jac.z + jac.w * jac.w) / f;
            let b = -2.0 * (jac.x * jac.z + jac.y * jac.w) / f;
            let c = (jac.x * jac.x + jac.y * jac.y) / f;
            // find the angle to rotate ellipse
            let v = Vector2::<f32>::new(c - a, -b);
            let lv = v.norm();
            let v0 = if lv > 0.01 { v.x / lv } else { 1.0 };
            // let v1 = if lv > 0.01 { v.y / lv } else { 1.0 };
            let cc = ((1.0 + v0).max(0.0) / 2.0).sqrt();
            let mut s = ((1.0 - v0).max(0.0) / 2.0).sqrt();
            // rotate the ellipse to align it with axes
            let mut a0 = a * cc * cc - b * cc * s + c * s * s;
            let mut c0 = a * s * s + b * cc * s + c * cc * cc;
            let bt1 = b * (cc * cc - s * s);
            let bt2 = 2.0 * (a - c) * cc * s;
            let mut b0 = bt1 + bt2;
            let b0v2 = bt1 - bt2;
            if b0.abs() > b0v2.abs() {
                s = -s;
                b0 = b0v2;
            }
            // clamp A,C
            a0 = a0.min(1.0);
            c0 = c0.min(1.0);
            let sn = -s;
            // rotate it back
            Vector3::new(
                a0 * cc * cc - b0 * cc * sn + c0 * sn * sn,
                2.0 * a0 * cc * sn + b0 * cc * cc - b0 * sn * sn - 2.0 * c0 * cc * sn,
                a0 * sn * sn + b0 * cc * sn + c0 * cc * cc
            )
        }
        fn bc2(x: f32, params: &KernelParams) -> f32 {
            let x = x.abs();
            unsafe {
                let x2 = x * x;
                if x < 1.0 {
                    return params.ewa_coeffs_p.get_unchecked(0) + params.ewa_coeffs_p.get_unchecked(1) * x + params.ewa_coeffs_p.get_unchecked(2) * x2 + params.ewa_coeffs_p.get_unchecked(3) * x2 * x;
                } else if x < 2.0 {
                    return params.ewa_coeffs_q.get_unchecked(0) + params.ewa_coeffs_q.get_unchecked(1) * x + params.ewa_coeffs_q.get_unchecked(2) * x2 + params.ewa_coeffs_q.get_unchecked(3) * x2 * x;
                }
            }
            0.0
        }
        ////////////////////////////// EWA (Elliptical Weighted Average) CubicBC sampling //////////////////////////////

        fn sample_input_at<const I: i32, T: PixelType>(uv: Vector2<f32>, jac: &Vector4<f32>, input: &[u8], params: &KernelParams, bg: &Vector4<f32>, _drawing: &[u8]) -> Vector4<f32> {
            let mut sum = Vector4::from_element(0.0);
            if I > 8 {
                // find how many pixels we need around that pixel in each direction
                let trans_size = affine_bbox(jac);
                let bounds = (
                    (uv.x - trans_size.x).floor() as i32,
                    (uv.x + trans_size.x).ceil() as i32,
                    (uv.y - trans_size.y).floor() as i32,
                    (uv.y + trans_size.y).ceil() as i32
                );
                let mut sum_div = 0.0;
                let mut src_index = bounds.2 * params.stride;

                // See: Andreas Gustafsson. "Interactive Image Warping", section 3.6 http://www.gson.org/thesis/warping-thesis.pdf
                let abc = clamped_ellipse(jac);
                for in_y in bounds.2..=bounds.3 {
                    let in_fy = in_y as f32 - uv.y;
                    let in_fy2 = in_fy * abc.y;
                    let in_fy3 = in_fy * in_fy * abc.z;
                    for in_x in bounds.0..=bounds.1 {
                        let in_fx = in_x as f32 - uv.x;
                        let dr = in_fx * in_fx * abc.x + in_fx * in_fy2 + in_fy3;
                        let k = bc2(dr.sqrt(), params); // cylindrical filtering
                        if k == 0.0 {
                            continue;
                        }
                        let pixel = if in_y >= params.source_rect[1] && in_y < params.source_rect[1] + params.source_rect[3] && in_x >= params.source_rect[0] && in_x < params.source_rect[0] + params.source_rect[2] {
                            let px1: &T = bytemuck::from_bytes(&input[src_index as usize + (params.bytes_per_pixel * in_x) as usize..src_index as usize + (params.bytes_per_pixel * (in_x + 1)) as usize]);
                            let src_px = PixelType::to_float(*px1);
                            // draw_pixel(&mut src_px, sx + xp, sy + yp, true, params.width, params, drawing);
                            src_px
                        } else {
                            *bg
                        };
                        sum += k * pixel;
                        sum_div += k;
                    }
                    src_index += params.stride;
                }
                sum /= sum_div;
            } else {
                const INTER_BITS: usize = 5;
                const INTER_TAB_SIZE: usize = 1 << INTER_BITS;
                let shift: i32 = (I >> 2) + 1;
                let offset: f32 = [0.0, 1.0, 3.0][I as usize >> 2];
                let ind: usize = [0, 64, 64 + 128][I as usize >> 2];

                let u = uv.x - offset;
                let v = uv.y - offset;

                let sx0 = (u * INTER_TAB_SIZE as f32).round() as i32;
                let sy0 = (v * INTER_TAB_SIZE as f32).round() as i32;

                let sx = sx0 >> INTER_BITS;
                let sy = sy0 >> INTER_BITS;

                let coeffs_x = &COEFFS[ind + ((sx0 as usize & (INTER_TAB_SIZE - 1)) << shift)..];
                let coeffs_y = &COEFFS[ind + ((sy0 as usize & (INTER_TAB_SIZE - 1)) << shift)..];

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
            }
            Vector4::new(
                sum.x.min(params.pixel_value_limit),
                sum.y.min(params.pixel_value_limit),
                sum.z.min(params.pixel_value_limit),
                sum.w.min(params.pixel_value_limit),
            )
        }

        fn undistort_coord(mut out_pos: Vector2<f32>, params: &KernelParams, matrices: &[[f32; 14]], distortion_model: &DistortionModel, digital_lens: Option<&DistortionModel>, r_limit_sq: f32, mesh_data: &[f64], out_c: &Vector2<f32>, out_f: &Vector2<f32>) -> Option<Vector2<f32>> {
            out_pos.x = map_coord(out_pos.x, params.output_rect[0] as f32, (params.output_rect[0] + params.output_rect[2]) as f32, 0.0, params.output_width  as f32);
            out_pos.y = map_coord(out_pos.y, params.output_rect[1] as f32, (params.output_rect[1] + params.output_rect[3]) as f32, 0.0, params.output_height as f32);
            out_pos.x += params.translation2d[0];
            out_pos.y += params.translation2d[1];

            ///////////////////////////////////////////////////////////////////
            // Add lens distortion back
            if params.lens_correction_amount < 1.0 {
                let mut new_out_pos = out_pos;

                if (params.flags & 2) == 2 { // Has digial lens
                    if let Some(digital) = digital_lens {
                        // Apply the digital warp in the UN-zoomed (fov=1) frame so it's FOV-independent,
                        let uz = ((new_out_pos.x - out_c.x) * params.fov + out_c.x, (new_out_pos.y - out_c.y) * params.fov + out_c.y);
                        if let Some(pt) = digital.undistort_point(uz, params) {
                            new_out_pos.x = (pt.0 - out_c.x) / params.fov + out_c.x;
                            new_out_pos.y = (pt.1 - out_c.y) / params.fov + out_c.y;
                        }
                    }
                }

                new_out_pos = (new_out_pos - out_c).component_div(out_f);
                let pt = distortion_model.undistort_point((new_out_pos.x, new_out_pos.y), params)?;
                new_out_pos.x = pt.0;
                new_out_pos.y = pt.1;
                if params.light_refraction_coefficient != 1.0 && params.light_refraction_coefficient > 0.0 {
                    let r = new_out_pos.norm();
                    if r != 0.0 {
                        let sin_theta_d = (r / (1.0 + r * r).sqrt()) / params.light_refraction_coefficient;
                        let r_d = sin_theta_d / (1.0 - sin_theta_d * sin_theta_d).sqrt();
                        let factor = r_d / r;
                        new_out_pos *= factor;
                    }
                }
                new_out_pos = (new_out_pos.component_mul(out_f)) + out_c;

                out_pos = new_out_pos * (1.0 - params.lens_correction_amount) + (out_pos * params.lens_correction_amount);
            }
            ///////////////////////////////////////////////////////////////////

            ///////////////////////////////////////////////////////////////////
            // Calculate source `y` for rolling shutter
            let mut sy = if (params.flags & 16) == 16 { // Horizontal RS
                (out_pos.x.round() as i32).min(params.width).max(0) as usize
            } else {
                (out_pos.y.round() as i32).min(params.height).max(0) as usize
            };
            if params.matrix_count > 1 {
                let idx = params.matrix_count as usize / 2;
                if let Some(pt) = Stabilization::rotate_and_distort((out_pos.x, out_pos.y), idx, params, matrices, distortion_model, digital_lens, r_limit_sq, mesh_data) {
                    if (params.flags & 16) == 16 { // Horizontal RS
                        sy = (pt.0.round() as i32).min(params.width).max(0) as usize;
                    } else {
                        sy = (pt.1.round() as i32).min(params.height).max(0) as usize;
                    }
                }
            }
            ///////////////////////////////////////////////////////////////////

            let idx = sy.min(params.matrix_count as usize - 1);
            let mut uv = Stabilization::rotate_and_distort((out_pos.x, out_pos.y), idx, params, matrices, distortion_model, digital_lens, r_limit_sq, mesh_data)?;
            let mut frame_size = (params.width as f32, params.height as f32);
            if params.input_rotation != 0.0 {
                let rotation = params.input_rotation * (std::f32::consts::PI / 180.0);
                let size = frame_size;
                frame_size = rotate_point(size, rotation, (0.0, 0.0), (0.0, 0.0));
                frame_size = (frame_size.0.abs().round(), frame_size.1.abs().round());
                uv = rotate_point(uv, rotation, (size.0 / 2.0, size.1 / 2.0), (frame_size.0 / 2.0, frame_size.1 / 2.0));
            }

            let width_f = params.width as f32;
            let height_f = params.height as f32;
            if params.background_mode == 1 { // Edge repeat
                uv = (
                    uv.0.max(3.0).min(width_f  - 3.0),
                    uv.1.max(3.0).min(height_f - 3.0),
                );
            } else if params.background_mode == 2 { // Edge mirror
                let rx = uv.0.round();
                let ry = uv.1.round();
                let width3 = width_f - 3.0;
                let height3 = height_f - 3.0;
                if rx > width3  { uv.0 = width3  - (rx - width3); }
                if rx < 3.0     { uv.0 = 3.0 + width_f - (width3  + rx); }
                if ry > height3 { uv.1 = height3 - (ry - height3); }
                if ry < 3.0     { uv.1 = 3.0 + height_f - (height3 + ry); }
            }
            if params.background_mode != 3 {
                uv = (
                    map_coord(uv.0, 0.0, frame_size.0, params.source_rect[0] as f32, (params.source_rect[0] + params.source_rect[2]) as f32),
                    map_coord(uv.1, 0.0, frame_size.1, params.source_rect[1] as f32, (params.source_rect[1] + params.source_rect[3]) as f32)
                );
            }
            Some(Vector2::new(uv.0, uv.1))
        }

        if let BufferSource::Cpu { buffer: input } = &mut buffers.input.data {
            if let BufferSource::Cpu { buffer: output } = &mut buffers.output.data {
                let r_limit_sq = params.r_limit * params.r_limit; // Square it so we don't have to do sqrt on the point length

                let bg = Vector4::<f32>::new(params.background[0], params.background[1], params.background[2], params.background[3]) * params.max_pixel_value;
                let bg_t: T = PixelType::from_float(bg);

                let factor = (1.0 - params.lens_correction_amount).max(0.001); // FIXME: this is close but wrong
                let out_c = Vector2::new(params.output_width as f32 / 2.0, params.output_height as f32 / 2.0);
                let out_f = Vector2::new(params.f[0] / params.fov / factor, params.f[1] / params.fov / factor);

                // let drawing_enabled = !drawing.is_empty() && (params.flags & 8) == 8;
                let fill_bg = (params.flags & 4) == 4;
                let fix_range = (params.flags & 1) == 1;
                let is_y = params.plane_index == 0;
                if buffers.output.size.2 <= 0 {
                    log::error!("buffers.output_size: {:?}", buffers.output.size);
                    return false;
                }

                let mesh_data = mesh_data.iter().map(|x| *x as f64).collect::<Vec<f64>>();

                assert_eq!(params.bytes_per_pixel as usize, std::mem::size_of::<T>());

                output.par_chunks_mut(buffers.output.size.2).enumerate().for_each(|(y, row_bytes)| { // Parallel iterator over buffer rows
                    row_bytes.chunks_mut(params.bytes_per_pixel as usize).enumerate().for_each(|(x, pix_chunk)| { // iterator over row pixels

                        let out_pos = (
                            map_coord(x as f32, params.output_rect[0] as f32, (params.output_rect[0] + params.output_rect[2]) as f32, 0.0, params.output_width  as f32),
                            map_coord(y as f32, params.output_rect[1] as f32, (params.output_rect[1] + params.output_rect[3]) as f32, 0.0, params.output_height as f32)
                        );

                        if out_pos.0 >= 0.0 && out_pos.1 >= 0.0 && (out_pos.0 as i32) < params.output_width && (out_pos.1 as i32) < params.output_height {

                            // let p = out_pos;
                            let mut pixel = bg;

                            let pix_out = bytemuck::from_bytes_mut(pix_chunk); // treat this byte chunk as `T`

                            if fill_bg {
                                *pix_out = bg_t;
                                return;
                            }

                            let position = Vector2::new(x as f32, y as f32);

                            if let Some(mut uv) = undistort_coord(position, params, matrices, distortion_model, digital_lens, r_limit_sq, &mesh_data, &out_c, &out_f) {
                                let mut jac = Vector4::new(1.0, 0.0, 0.0, 1.0);
                                if I > 8 {
                                    let eps = 0.01;
                                    let nx = undistort_coord(position + Vector2::new(eps, 0.0), params, matrices, distortion_model, digital_lens, r_limit_sq, &mesh_data, &out_c, &out_f);
                                    let ny = undistort_coord(position + Vector2::new(0.0, eps), params, matrices, distortion_model, digital_lens, r_limit_sq, &mesh_data, &out_c, &out_f);
                                    if let (Some(nx), Some(ny)) = (nx, ny) {
                                        let xyx = nx - uv;
                                        let xyy = ny - uv;
                                        jac = Vector4::new(xyx.x / eps, xyy.x / eps, xyx.y / eps, xyy.y / eps);
                                    }
                                }

                                let width_f = params.width as f32;
                                let height_f = params.height as f32;
                                if params.background_mode == 3 { // Margin with feather
                                    let widthf  = width_f - 1.0;
                                    let heightf = height_f - 1.0;

                                    let feather = (params.background_margin_feather * heightf).max(0.0001);
                                    let mut pt2 = uv;
                                    let mut alpha = 1.0;
                                    if (uv.x > widthf - feather) || (uv.x < feather) || (uv.y > heightf - feather) || (uv.y < feather) {
                                        alpha = ((widthf - uv.x).min(heightf - uv.y).min(uv.x).min(uv.y) / feather).min(1.0).max(0.0);
                                        let size_f = Vector2::new(width_f, height_f);
                                        let half = Vector2::from_element(0.5);
                                        pt2.component_div_assign(&size_f);
                                        pt2 = ((pt2 - half) * (1.0 - params.background_margin)) + half;
                                        pt2.component_mul_assign(&size_f);
                                    }

                                    let mut frame_size = (params.width as f32, params.height as f32);
                                    if params.input_rotation != 0.0 {
                                        let rotation = params.input_rotation * (std::f32::consts::PI / 180.0);
                                        let size = frame_size;
                                        frame_size = rotate_point(size, rotation, (0.0, 0.0), (0.0, 0.0));
                                        frame_size = (frame_size.0.abs().round(), frame_size.1.abs().round());
                                    }
                                    uv  = Vector2::new(map_coord(uv.x,  0.0, frame_size.0, params.source_rect[0] as f32, (params.source_rect[0] + params.source_rect[2]) as f32),
                                                       map_coord(uv.y,  0.0, frame_size.1, params.source_rect[1] as f32, (params.source_rect[1] + params.source_rect[3]) as f32));
                                    pt2 = Vector2::new(map_coord(pt2.x, 0.0, frame_size.0, params.source_rect[0] as f32, (params.source_rect[0] + params.source_rect[2]) as f32),
                                                       map_coord(pt2.y, 0.0, frame_size.1, params.source_rect[1] as f32, (params.source_rect[1] + params.source_rect[3]) as f32));

                                    let c1 = sample_input_at::<I, T>(uv, &jac, input, params, &bg, drawing);
                                    let c2 = sample_input_at::<I, T>(pt2, &jac, input, params, &bg, drawing); // FIXME: jac should be adjusted for pt2
                                    pixel = c1 * alpha + c2 * (1.0 - alpha);
                                    // draw_pixel(&mut pixel, p.0 as i32, p.1 as i32, false, params.output_width, params, drawing);
                                    if fix_range {
                                        remap_colorrange(&mut pixel, is_y)
                                    }
                                    *pix_out = PixelType::from_float(pixel);
                                    return;
                                }

                                pixel = sample_input_at::<I, T>(uv, &jac, input, params, &bg, drawing);
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

pub fn undistort_points_with_rolling_shutter(distorted: &[(f32, f32)], timestamp_ms: f64, frame: Option<usize>, params: &ComputeParams, lens_correction_amount: f64, use_fovs: bool, clamp_to_image_circle: bool) -> Vec<(f32, f32)> {
    if distorted.is_empty() { return Vec::new(); }
    let (camera_matrix, distortion_coeffs, _p, rotations, is, mesh, fov, r_limit) = FrameTransform::at_timestamp_for_points(params, distorted, timestamp_ms, frame, use_fovs);

    undistort_points(distorted, camera_matrix, &distortion_coeffs, rotations[0], Some(Matrix3::identity()), Some(rotations), params, lens_correction_amount, fov, timestamp_ms, is, mesh, if clamp_to_image_circle { r_limit } else { 0.0 })
}
pub fn undistort_points_for_optical_flow(distorted: &[(f32, f32)], timestamp_us: i64, params: &ComputeParams, points_dims: (u32, u32)) -> Vec<(f32, f32)> {
    let img_dim_ratio = points_dims.0 as f64 / params.width.max(1) as f64;//FrameTransform::get_ratio(params);

    let (camera_matrix, distortion_coeffs, _, _, _, _, _) = FrameTransform::get_lens_data_at_timestamp(params, timestamp_us as f64 / 1000.0, false);

    let scaled_k = camera_matrix * img_dim_ratio;

    // Optical flow matches real image features, so no image-circle clamping here - see `undistort_points`.
    undistort_points(distorted, scaled_k, &distortion_coeffs, Matrix3::identity(), None, None, params, 1.0, 1.0, timestamp_us as f64 / 1000.0, None, None, 0.0)
}
// Ported from OpenCV: https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/fisheye.cpp#L321
pub fn undistort_points(distorted: &[(f32, f32)], camera_matrix: Matrix3<f64>, distortion_coeffs: &[f64; 24], rotation: Matrix3<f64>, p: Option<Matrix3<f64>>, rot_per_point: Option<Vec<Matrix3<f64>>>, params: &ComputeParams, lens_correction_amount: f64, fov: f64, timestamp_ms: f64, shift_per_point: Option<Vec<(f32, f32, f32, f32, f32)>>, mesh: Option<Vec<f64>>, r_limit: f64) -> Vec<(f32, f32)> {
    let f = (camera_matrix[(0, 0)] as f32, camera_matrix[(1, 1)] as f32);
    let c = (camera_matrix[(0, 2)] as f32, camera_matrix[(1, 2)] as f32);
    let r_limit = r_limit as f32;

    let mut rr = rotation;
    if let Some(p) = p { // PP
        rr = p * rr;
    }

    let light_refraction_coefficient = params.keyframes.value_at_video_timestamp(&crate::KeyframeType::LightRefractionCoeff, timestamp_ms).unwrap_or(params.light_refraction_coefficient) as f32;

    let mut digital_lens_params = [0f32; 16];
    if let Some(p) = &params.digital_lens_params {
        for (i, v) in p.iter().take(16).enumerate() {
            digital_lens_params[i] = *v as f32;
        }
    }

    // TODO more params
    let kernel_params = KernelParams {
        width : params.width as i32,
        height: params.height as i32,
        output_width: params.output_width as i32,
        output_height: params.output_height as i32,
        f: [f.0, f.1],
        c: [c.0, c.1],
        k: distortion_coeffs.iter().map(|x| *x as f32).collect::<Vec<_>>().try_into().unwrap(),
        digital_lens_params,
        light_refraction_coefficient,
        // The model has to reach exactly as far here as it does in the render, or the lens-correction
        // solve below measures a field the picture doesn't have
        r_limit,

        ..Default::default()
    };

    // Lens-correction blend constants — point-independent, so compute once instead of per point.
    let lens_correction = if lens_correction_amount < 1.0 {
        // Match the render's lens-correction EXACTLY: out_c = output/2 (no input-stretch division) and
        // out_f = (f/fov)/factor — see undistort.frag and the undistort_coord render mirror above.
        let out_c = (params.output_width as f32 / 2.0, params.output_height as f32 / 2.0);
        let amount = lens_correction_amount as f32;
        let factor = (1.0 - amount).max(0.001);
        let out_f = (f.0 / fov as f32 / factor, f.1 / fov as f32 / factor);
        // The largest image radius the model still corrects - its own field limit with `r_limit` on top.
        // An output position past `n_max * out_f` has no picture at all (the render's `undistort_point`
        // returns None there and the pixel becomes background), so the solve below must stay inside it.
        let (mut lo, mut hi) = (0.0f32, 8.0f32);
        for _ in 0..32 {
            let mid = 0.5 * (lo + hi);
            if params.distortion_model.undistort_point((mid, 0.0), &kernel_params).is_some() { lo = mid; } else { hi = mid; }
        }
        // R (`r_of` below) is radial about `out_c` only when nothing in it moves a point off its ray: a digital
        // lens, or tangential and thin-prism terms of the optical model, need the two-dimensional solve
        let radial = params.digital_lens.is_none() && params.distortion_model.is_radial(&kernel_params);
        Some((out_c, amount, factor, out_f, lo, radial))
    } else { None };

    // Not parallelized here: the only caller that reaches the lens-correction block below
    // (zooming::fov_iterative) already runs this across frames via into_par_iter, and the
    // stmap caller passes one point at a time — so a nested par_iter would only oversubscribe.
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

        if let Some(mesh_data) = mesh.as_ref().filter(|m| m.len() > 9) {
            // FocalPlaneDistortion
            let o = mesh_data[0] as usize; // offset to focal plane distortion data
            if o > 0 && mesh_data.get(o).map_or(false, |x| *x > 0.0) {

                let mesh_size = (mesh_data[3], mesh_data[4]);
                let origin    = (mesh_data[5] as f32, mesh_data[6] as f32);
                let crop_size = (mesh_data[7] as f32, mesh_data[8] as f32);
                let stblz_grid = if mesh_data[o + 2] > 0.0 { mesh_data[o + 2] } else { mesh_size.1 / 8.0 }; // band height comes with the table

                x = map_coord(x, 0.0, params.width  as f32, origin.0, origin.0 + crop_size.0);
                y = map_coord(y, 0.0, params.height as f32, origin.1, origin.1 + crop_size.1);

                let idx = (y as f64 / stblz_grid).floor().max(0.0).min(7.0) as usize;
                let delta = y as f64 - stblz_grid * idx as f64;
                x += (mesh_data[o + 4 + idx * 2 + 0] * delta) as f32;
                y += (mesh_data[o + 4 + idx * 2 + 1] * delta) as f32;
                for j in 0..idx {
                    x += (mesh_data[o + 4 + j * 2 + 0] * stblz_grid) as f32;
                    y += (mesh_data[o + 4 + j * 2 + 1] * stblz_grid) as f32;
                }

                x = map_coord(x, origin.0, origin.0 + crop_size.0, 0.0, params.width  as f32);
                y = map_coord(y, origin.1, origin.1 + crop_size.1, 0.0, params.height as f32);
            }

            if mesh_data[0] > 10.0 {
                let mesh_size = (mesh_data[3], mesh_data[4]);
                let origin    = (mesh_data[5] as f32, mesh_data[6] as f32);
                let crop_size = (mesh_data[7] as f32, mesh_data[8] as f32);

                x = map_coord(x, 0.0, params.width  as f32, origin.0, origin.0 + crop_size.0);
                y = map_coord(y, 0.0, params.height as f32, origin.1, origin.1 + crop_size.1);

                let new_pos = crate::gyro_source::interpolate_mesh(x as f64, y as f64, (mesh_size.0, mesh_size.1), &mesh_data);

                x = map_coord(new_pos.x as f32, origin.0, origin.0 + crop_size.0, 0.0, params.width  as f32);
                y = map_coord(new_pos.y as f32, origin.1, origin.1 + crop_size.1, 0.0, params.height as f32);
            }
        }
        if let Some(shift) = shift_per_point.as_ref().and_then(|v| v.get(index)) {
            // Sensor roll around the principal point first, then the sensor/lens shift (the inverse of `rotate_and_distort`)
            let ang_rad = shift.2;
            let cos_a = ang_rad.cos();
            let sin_a = ang_rad.sin();
            let (xr, yr) = (x - c.0, y - c.1);
            x = cos_a * xr - sin_a * yr - shift.3 + shift.0 + c.0;
            y = sin_a * xr + cos_a * yr - shift.4 + shift.1 + c.1;
        }

        let pw = ((x - c.0) / f.0, (y - c.1) / f.1); // world point

        let rot = nalgebra::convert::<nalgebra::Matrix3<f64>, nalgebra::Matrix3<f32>>(*rot_per_point.as_ref().and_then(|v| v.get(index)).unwrap_or(&rr));

        let ray = match params.distortion_model.undistort_point(pw, &kernel_params) {
            Some(mut pt) => {
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
                if r_limit > 0.0 {
                    let r = (pt.0.powi(2) + pt.1.powi(2)).sqrt();
                    if r > r_limit {
                        pt = (pt.0 * (r_limit / r), pt.1 * (r_limit / r));
                    }
                }
                Some(pt)
            },
            None if r_limit > 0.0 => {
                let r = (pw.0.powi(2) + pw.1.powi(2)).sqrt();
                if r > 0.0 { Some((pw.0 * (r_limit / r), pw.1 * (r_limit / r))) } else { None }
            },
            None => None
        };

        if let Some(ray) = ray {
            // reproject
            let pr = rot * nalgebra::Vector3::new(ray.0, ray.1, 1.0); // rotated point optionally multiplied by new camera matrix
            if !(pr[2] > 0.0) {
                return INVALID_POINT;
            }
            let mut pt = (pr[0] / pr[2], pr[1] / pr[2]);

            if let Some((out_c, amount, factor, out_f, n_max, radial)) = lens_correction {
                // The digital warp is applied FOV-independently (un-zoom -> warp -> re-zoom below), so the
                // corrected frame shape is a pure uniform scale in fov — the zoom search and the cached debug
                // overlay both use the use_fovs=false `fov`, and the live FOV is a uniform scale at draw time.
                //
                // The render samples at  texPos_used = lerp(R(o), o, amount), where R(o) is applied FORWARD to
                // the in-frame output pixel o:  R = digital_undistort -> /out_f -> radial undistort -> *out_f
                // (undistort.frag). `pt` is that texPos_used, so the output position o of this source point is the
                // solution of  amount*o + (1-amount)*R(o) = pt, see `invert_lens_correction_blend`. R is evaluated
                // FORWARD — digital_undistort is a direct polynomial (finite even out-of-domain) and radial undistort
                // is well-defined, so this never blows up. (The old closed-form inverse fed the rectilinear,
                // far-out-of-frame `pt` into the digital *inverse*, which diverged to NaN for lc>0.)
                let r_of = |o: (f32, f32)| -> (f32, f32) {
                    let mut q = o;
                    if let Some(digital) = &params.digital_lens {
                        // FOV-independent digital warp: un-zoom -> warp -> re-zoom.
                        let uz = ((q.0 - out_c.0) * fov as f32 + out_c.0, (q.1 - out_c.1) * fov as f32 + out_c.1);
                        if let Some(d) = digital.undistort_point(uz, &kernel_params) {
                            q = ((d.0 - out_c.0) / fov as f32 + out_c.0, (d.1 - out_c.1) / fov as f32 + out_c.1);
                        }
                    }
                    let mut n = ((q.0 - out_c.0) / out_f.0, (q.1 - out_c.1) / out_f.1);
                    // Past the model's field, hold the correction at the edge. Falling back to the identity
                    // out there (`R(o) = o`) collapses the blend to `o = pt`, ie. it hands the solve a
                    // spurious root at the FULLY corrected position - which is why every strength between
                    // 0 and 100% measured the 100% polygon and the zoom overlay disagreed with the picture.
                    let nr = (n.0 * n.0 + n.1 * n.1).sqrt();
                    if nr > n_max && nr > 0.0 {
                        let s = n_max / nr;
                        n = (n.0 * s, n.1 * s);
                    }
                    n = match params.distortion_model.undistort_point(n, &kernel_params) {
                        Some(d) => d,
                        None if r_limit > 0.0 && nr > 0.0 => (n.0 * (r_limit / nr), n.1 * (r_limit / nr)),
                        None => n,
                    };
                    if kernel_params.light_refraction_coefficient != 1.0 && kernel_params.light_refraction_coefficient > 0.0 {
                        let r = (n.0 * n.0 + n.1 * n.1).sqrt();
                        if r != 0.0 {
                            let sin_theta_d = (r / (1.0 + r * r).sqrt()) / kernel_params.light_refraction_coefficient;
                            let r_d = sin_theta_d / (1.0 - sin_theta_d * sin_theta_d).sqrt();
                            let s = r_d / r;
                            n = (n.0 * s, n.1 * s);
                        }
                    }
                    ((n.0 * out_f.0) + out_c.0, (n.1 * out_f.1) + out_c.1)
                };
                pt = invert_lens_correction_blend(pt, out_c, out_f, n_max, amount, factor, radial, &r_of);
            }
            pt
        } else {
            INVALID_POINT
        }
    }).collect()
}

/// Inverts the render's lens-correction blend for one output-space point: the `o` with
/// `amount * o + factor * R(o) = target`, where `R` is the forward lens-correction map of `undistort_points`
/// (its `r_of`), `out_c` the centre R works about, `out_f` its focal length and `n_max` the largest normalized
/// radius it still corrects (an `o` past `n_max * out_f` has no picture at all, so the answer stays inside it).
///
/// For a radial R the equation is scalar and monotone along the ray from `out_c` through `target`, and it is
/// bisected there: the residual projected on the ray is `-|target - out_c|` at the centre and increasing, so the
/// bracket holds from the start, and when it never reaches zero inside the field the picture of this source
/// point ends at the field edge. Newton on the 2D map could not be trusted on its own: R rises almost vertically
/// over a pixel at the field edge, and nothing stopped it from walking out of the domain and onto the spurious
/// root the identity fallback used to offer there.
///
/// When R is not radial (a digital lens, tangential or thin-prism terms of the optical model), the ray answer is
/// only the initial guess and a damped 2D Newton polishes it against the SAME `target`: a step is taken only
/// when it reduces the residual (halved up to three times if it doesn't), so the polish can never hand back
/// anything worse than the ray solve, and it stops on the first step that improves nothing. The Jacobian comes
/// from one-pixel forward differences; R is smooth at that scale everywhere but the field edge, and there the
/// damping holds.
pub(crate) fn invert_lens_correction_blend(target: (f32, f32), out_c: (f32, f32), out_f: (f32, f32), n_max: f32, amount: f32, factor: f32, radial: bool, r_of: impl Fn((f32, f32)) -> (f32, f32)) -> (f32, f32) {
    // ----- Along the ray -----
    let dir = (target.0 - out_c.0, target.1 - out_c.1);
    let dlen = (dir.0 * dir.0 + dir.1 * dir.1).sqrt();
    let mut o = if dlen > 1e-6 {
        let u = (dir.0 / dlen, dir.1 / dlen);
        let (nx, ny) = (u.0 / out_f.0, u.1 / out_f.1);
        let rho_max = n_max / (nx * nx + ny * ny).sqrt();
        let resid = |rho: f32| -> f32 {
            let o = (out_c.0 + u.0 * rho, out_c.1 + u.1 * rho);
            let r = r_of(o);
            (amount * o.0 + factor * r.0 - target.0) * u.0 + (amount * o.1 + factor * r.1 - target.1) * u.1
        };
        let (mut lo, mut hi) = (0.0f32, rho_max);
        if resid(hi) < 0.0 {
            lo = hi; // no solution inside the field: this source point's picture ends at its edge
        } else {
            for _ in 0..24 {
                let mid = 0.5 * (lo + hi);
                if resid(mid) < 0.0 { lo = mid; } else { hi = mid; }
            }
        }
        let rho = 0.5 * (lo + hi);
        (out_c.0 + u.0 * rho, out_c.1 + u.1 * rho)
    } else {
        out_c
    };
    if radial { return o; }

    // ----- Two-dimensional polish -----
    const TOLERANCE: f32 = 0.02; // pixels of residual
    const EPS: f32 = 1.0;        // pixels, the finite-difference step
    let clamp_o = |o: (f32, f32)| -> (f32, f32) {
        let n = ((o.0 - out_c.0) / out_f.0, (o.1 - out_c.1) / out_f.1);
        let r = (n.0 * n.0 + n.1 * n.1).sqrt();
        if r > n_max && r > 0.0 {
            let s = n_max / r;
            (out_c.0 + n.0 * s * out_f.0, out_c.1 + n.1 * s * out_f.1)
        } else { o }
    };
    // R(o), g(o) = amount*o + factor*R(o) - target, |g(o)|
    let eval = |o: (f32, f32)| -> ((f32, f32), (f32, f32), f32) {
        let r = r_of(o);
        let g = (amount * o.0 + factor * r.0 - target.0, amount * o.1 + factor * r.1 - target.1);
        (r, g, (g.0 * g.0 + g.1 * g.1).sqrt())
    };
    let (mut r, mut g, mut g_norm) = eval(o);
    for _ in 0..10 {
        if !(g_norm > TOLERANCE) { break; } // converged, or NaN
        let rx = r_of((o.0 + EPS, o.1));
        let ry = r_of((o.0, o.1 + EPS));
        let j11 = amount + factor * (rx.0 - r.0) / EPS; let j12 = factor * (ry.0 - r.0) / EPS;
        let j21 = factor * (rx.1 - r.1) / EPS;          let j22 = amount + factor * (ry.1 - r.1) / EPS;
        let det = j11 * j22 - j12 * j21;
        if !det.is_finite() || det.abs() < 1e-9 { break; }
        let dx = ( j22 * g.0 - j12 * g.1) / det;
        let dy = (-j21 * g.0 + j11 * g.1) / det;
        if !dx.is_finite() || !dy.is_finite() { break; }
        // Backtracking: the longest step that still reduces the residual
        let mut improved = false;
        for &t in &[1.0f32, 0.5, 0.25, 0.125] {
            let candidate = clamp_o((o.0 - t * dx, o.1 - t * dy));
            let (rc, gc, gc_norm) = eval(candidate);
            if gc_norm < g_norm {
                o = candidate; r = rc; g = gc; g_norm = gc_norm;
                improved = true;
                break;
            }
        }
        if !improved { break; }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::invert_lens_correction_blend;

    const OUT_C: (f32, f32) = (960.0, 540.0);
    const OUT_F: (f32, f32) = (1000.0, 1000.0);
    const N_MAX: f32 = 4.0;

    // R(o) = out_c + diag(s) (o - out_c): a uniform `s` is radial, an anisotropic one is a digital stretch
    fn stretch(s: (f32, f32)) -> impl Fn((f32, f32)) -> (f32, f32) {
        move |o| (OUT_C.0 + s.0 * (o.0 - OUT_C.0), OUT_C.1 + s.1 * (o.1 - OUT_C.1))
    }
    // amount*o + (1-amount)*R(o) = target then has the closed form o = out_c + (target - out_c) / (amount + (1-amount) s)
    fn closed_form(target: (f32, f32), s: (f32, f32), amount: f32) -> (f32, f32) {
        let factor = 1.0 - amount;
        (OUT_C.0 + (target.0 - OUT_C.0) / (amount + factor * s.0), OUT_C.1 + (target.1 - OUT_C.1) / (amount + factor * s.1))
    }
    fn residual(o: (f32, f32), target: (f32, f32), amount: f32, r_of: &dyn Fn((f32, f32)) -> (f32, f32)) -> f32 {
        let r = r_of(o);
        let factor = 1.0 - amount;
        ((amount * o.0 + factor * r.0 - target.0).powi(2) + (amount * o.1 + factor * r.1 - target.1).powi(2)).sqrt()
    }
    fn close(a: (f32, f32), b: (f32, f32)) -> bool { (a.0 - b.0).abs() < 0.05 && (a.1 - b.1).abs() < 0.05 }

    #[test]
    fn radial_map_is_solved_on_the_ray() {
        let target = (OUT_C.0 + 800.0, OUT_C.1 - 300.0);
        let r_of = stretch((1.4, 1.4));
        let o = invert_lens_correction_blend(target, OUT_C, OUT_F, N_MAX, 0.5, 0.5, true, &r_of);
        let e = closed_form(target, (1.4, 1.4), 0.5);
        assert!(close(o, e), "{o:?} vs {e:?}");
    }

    #[test]
    fn identity_map_returns_the_target() {
        let target = (OUT_C.0 + 800.0, OUT_C.1 + 300.0);
        let r_of = stretch((1.0, 1.0));
        for radial in [true, false] {
            let o = invert_lens_correction_blend(target, OUT_C, OUT_F, N_MAX, 0.5, 0.5, radial, &r_of);
            assert!(close(o, target), "{o:?} vs {target:?}");
        }
    }

    #[test]
    fn nonradial_map_is_polished_against_the_original_target() {
        // A digital stretch (x by 1.5, y by 0.8 about the centre) at 50% correction. The ray solve alone lands off
        // the 2D solution, and a polish that solved against the ray answer instead of the target (what the code
        // did before) returned a point satisfying neither equation
        let target = (OUT_C.0 + 800.0, OUT_C.1 + 300.0);
        let s = (1.5, 0.8);
        let r_of = stretch(s);
        let ray = invert_lens_correction_blend(target, OUT_C, OUT_F, N_MAX, 0.5, 0.5, true, &r_of);
        assert!(residual(ray, target, 0.5, &r_of) > 1.0, "the ray answer already solves the 2D equation, the map isn't anisotropic enough to test anything");
        let o = invert_lens_correction_blend(target, OUT_C, OUT_F, N_MAX, 0.5, 0.5, false, &r_of);
        let e = closed_form(target, s, 0.5);
        assert!(close(o, e), "{o:?} vs {e:?}");
        assert!(residual(o, target, 0.5, &r_of) < 0.05);
    }

    #[test]
    fn polish_never_worsens_the_ray_answer() {
        // A map with a kink inside the field and a jump past it, plus a shear: Newton's local model is wrong
        // across both, and the damping has to keep whatever the ray solve found
        let r_of = |o: (f32, f32)| -> (f32, f32) {
            let d = (o.0 - OUT_C.0, o.1 - OUT_C.1);
            let r = (d.0 * d.0 + d.1 * d.1).sqrt();
            let s = if r < 500.0 { 1.0 } else if r < 900.0 { 1.0 + (r - 500.0) / 200.0 } else { 6.0 };
            (OUT_C.0 + s * d.0 + 0.3 * d.1, OUT_C.1 + s * d.1 - 0.2 * d.0)
        };
        for &(dx, dy) in &[(300.0f32, 100.0f32), (700.0, -400.0), (1500.0, 900.0), (3500.0, 0.0), (0.0, 0.0)] {
            let target = (OUT_C.0 + dx, OUT_C.1 + dy);
            let ray = invert_lens_correction_blend(target, OUT_C, OUT_F, N_MAX, 0.3, 0.7, true, &r_of);
            let polished = invert_lens_correction_blend(target, OUT_C, OUT_F, N_MAX, 0.3, 0.7, false, &r_of);
            assert!(residual(polished, target, 0.3, &r_of) <= residual(ray, target, 0.3, &r_of) + 1e-3, "{target:?}");
        }
    }
}
