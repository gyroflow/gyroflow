// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::{ PixelType, Undistortion, ComputeParams, FrameTransform };
use nalgebra::{ Vector4, Matrix3 };
use rayon::{ prelude::ParallelSliceMut, iter::{ ParallelIterator, IndexedParallelIterator } };

const COEFFS: [f32; 64+128+256] = [
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

impl<T: PixelType> Undistortion<T> {
    // Adapted from OpenCV: initUndistortRectifyMap + remap 
    // https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/fisheye.cpp#L454
    // https://github.com/opencv/opencv/blob/4.x/modules/imgproc/src/opencl/remap.cl#L390
    pub fn undistort_image_cpu<const I: i32>(pixels: &mut [u8], out_pixels: &mut [u8], width: usize, height: usize, stride: usize, output_width: usize, output_height: usize, output_stride: usize, undistortion_params: &[[f32; 9]], bg: Vector4<f32>) {
        let bg_t: T = PixelType::from_float(bg);
        
        const INTER_BITS: usize = 5;
        const INTER_TAB_SIZE: usize = 1 << INTER_BITS;

        let f = &undistortion_params[0][0..2];
        let c = &undistortion_params[0][2..4];
        let k = &undistortion_params[0][4..8];
        let r_limit = undistortion_params[0][8];

        let bytes_per_pixel = T::COUNT * T::SCALAR_BYTES;
        let shift = (I >> 2) + 1;
        let offset = [0.0, 1.0, 3.0][I as usize >> 2];
        let ind = [0, 64, 64 + 128][I as usize >> 2];

        out_pixels.par_chunks_mut(output_stride).enumerate().for_each(|(y, row_bytes)| { // Parallel iterator over buffer rows
            row_bytes.chunks_mut(T::COUNT * T::SCALAR_BYTES).enumerate().for_each(|(x, pix_chunk)| { // iterator over row pixels
                if y < output_height && x < output_width {
                    assert!(pix_chunk.len() == std::mem::size_of::<T>());
                    ///////////////////////////////////////////////////////////////////
                    // Calculate source `y` for rolling shutter
                    let mut sy = y;
                    if undistortion_params.len() > 2 {
                        let undistortion_params = undistortion_params[undistortion_params.len() / 2]; // Use middle matrix
                        let _x = y as f32 * undistortion_params[1] + undistortion_params[2] + (x as f32 * undistortion_params[0]);
                        let _y = y as f32 * undistortion_params[4] + undistortion_params[5] + (x as f32 * undistortion_params[3]);
                        let _w = y as f32 * undistortion_params[7] + undistortion_params[8] + (x as f32 * undistortion_params[6]);
                        if _w > 0.0 {
                            let posx = _x / _w;
                            let posy = _y / _w;
                            let r = (posx*posx + posy*posy).sqrt();
                            let theta = r.atan();
                            let theta2 = theta*theta; let theta4 = theta2*theta2; let theta6 = theta4*theta2; let theta8 = theta4*theta4;
                            let theta_d = theta * (1.0 + k[0]*theta2 + k[1]*theta4 + k[2]*theta6 + k[3]*theta8);
                            let scale =  if r == 0.0 { 1.0 } else { theta_d / r };
                            let v = f[1] * posy * scale + c[1];
                            sy = (((0.5 + v * INTER_TAB_SIZE as f32).floor() as i32) >> INTER_BITS).min(height as i32).max(0) as usize;
                        }
                    }
                    ///////////////////////////////////////////////////////////////////

                    let undistortion_params = undistortion_params[(sy + 1).min(undistortion_params.len() - 1)];
                    let _x = y as f32 * undistortion_params[1] + undistortion_params[2] + (x as f32 * undistortion_params[0]);
                    let _y = y as f32 * undistortion_params[4] + undistortion_params[5] + (x as f32 * undistortion_params[3]);
                    let _w = y as f32 * undistortion_params[7] + undistortion_params[8] + (x as f32 * undistortion_params[6]);
                
                    let pix_out = bytemuck::from_bytes_mut(pix_chunk); // treat this byte chunk as `T`

                    if _w > 0.0 {
                        let posx = _x / _w;
                        let posy = _y / _w;
                
                        let r = (posx*posx + posy*posy).sqrt();
                        let theta = r.atan();

                        if r_limit > 0.0 && r > r_limit {
                            *pix_out = bg_t;
                            return;
                        }
                
                        let theta2 = theta*theta;
                        let theta4 = theta2*theta2;
                        let theta6 = theta4*theta2;
                        let theta8 = theta4*theta4;
                
                        let theta_d = theta * (1.0 + k[0]*theta2 + k[1]*theta4 + k[2]*theta6 + k[3]*theta8);
                
                        let scale =  if r == 0.0 { 1.0 } else { theta_d / r };
                        let u = f[0] * posx * scale + c[0] - offset;
                        let v = f[1] * posy * scale + c[1] - offset;
                
                        let sx0 = (u * INTER_TAB_SIZE as f32).round() as i32;
                        let sy0 = (v * INTER_TAB_SIZE as f32).round() as i32;

                        let sx = sx0 >> INTER_BITS;
                        let sy = sy0 >> INTER_BITS;

                        let coeffs_x = &COEFFS[ind + ((sx0 as usize & (INTER_TAB_SIZE - 1)) << shift)..];
                        let coeffs_y = &COEFFS[ind + ((sy0 as usize & (INTER_TAB_SIZE - 1)) << shift)..];
                
                        let mut sum = Vector4::from_element(0.0);
                        let mut src_index = (sy * stride as i32 + sx * bytes_per_pixel as i32) as isize;

                        for yp in 0..I {
                            if sy + yp >= 0 && sy + yp < height as i32 {
                                let mut xsum = Vector4::<f32>::from_element(0.0);
                                for xp in 0..I {
                                    let pixel = if sx + xp >= 0 && sx + xp < width as i32 {
                                        let px1: &T = bytemuck::from_bytes(&pixels[src_index as usize + (bytes_per_pixel * xp as usize)..src_index as usize + bytes_per_pixel * (xp as usize + 1)]); 
                                        PixelType::to_float(*px1)
                                    } else {
                                        bg
                                    };
                                    xsum += pixel * coeffs_x[xp as usize];
                                }

                                sum += xsum * coeffs_y[yp as usize];
                            } else {
                                sum += bg * coeffs_y[yp as usize];
                            }
                            src_index += stride as isize;
                        }
                        *pix_out = PixelType::from_float(sum);
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

    undistort_points(distorted, camera_matrix, &distortion_coeffs, rotations[0], Some(Matrix3::identity()), Some(rotations))
}

// Ported from OpenCV: https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/fisheye.cpp#L321
pub fn undistort_points(distorted: &[(f64, f64)], camera_matrix: Matrix3<f64>, distortion_coeffs: &[f64], rotation: Matrix3<f64>, p: Option<Matrix3<f64>>, rot_per_point: Option<Vec<Matrix3<f64>>>) -> Vec<(f64, f64)> {
    let f = (camera_matrix[(0, 0)], camera_matrix[(1, 1)]);
    let c = (camera_matrix[(0, 2)], camera_matrix[(1, 2)]);
    let k = distortion_coeffs;
    
    let mut rr = rotation;
    if let Some(p) = p { // PP
        rr = p * rr;
    }

    // TODO: into_par_iter?
    distorted.iter().enumerate().map(|(index, pi)| {
        let pw = ((pi.0 - c.0) / f.0, (pi.1 - c.1) / f.1); // world point

        let mut theta_d = (pw.0 * pw.0 + pw.1 * pw.1).sqrt();

        // the current camera model is only valid up to 180 FOV
        // for larger FOV the loop below does not converge
        // clip values so we still get plausible results for super fisheye images > 180 grad
        theta_d = theta_d.max(-std::f64::consts::FRAC_PI_2).min(std::f64::consts::FRAC_PI_2);

        let mut converged = false;
        let mut theta = theta_d;

        let mut scale = 0.0;

        if theta_d.abs() > 1e-8 {
            // compensate distortion iteratively
            for _ in 0..10 {
                let theta2 = theta*theta;
                let theta4 = theta2*theta2;
                let theta6 = theta4*theta2;
                let theta8 = theta6*theta2;
                let k0_theta2 = k[0] * theta2;
                let k1_theta4 = k[1] * theta4;
                let k2_theta6 = k[2] * theta6;
                let k3_theta8 = k[3] * theta8;
                // new_theta = theta - theta_fix, theta_fix = f0(theta) / f0'(theta)
                let theta_fix = (theta * (1.0 + k0_theta2 + k1_theta4 + k2_theta6 + k3_theta8) - theta_d)
                                /
                                (1.0 + 3.0 * k0_theta2 + 5.0 * k1_theta4 + 7.0 * k2_theta6 + 9.0 * k3_theta8);

                theta -= theta_fix;
                if theta_fix.abs() < 1e-8 {
                    converged = true;
                    break;
                }
            }

            scale = theta.tan() / theta_d;
        } else {
            converged = true;
        }

        // theta is monotonously increasing or decreasing depending on the sign of theta
        // if theta has flipped, it might converge due to symmetry but on the opposite of the camera center
        // so we can check whether theta has changed the sign during the optimization
        let theta_flipped = (theta_d < 0.0 && theta > 0.0) || (theta_d > 0.0 && theta < 0.0);

        if converged && !theta_flipped {
            let pu = (pw.0 * scale, pw.1 * scale); // undistorted point

            let rot = rot_per_point.as_ref().and_then(|v| v.get(index)).unwrap_or(&rr);
            // reproject
            let pr = rot * nalgebra::Vector3::new(pu.0, pu.1, 1.0); // rotated point optionally multiplied by new camera matrix

            (pr[0] / pr[2], pr[1] / pr[2])
        } else {
            (-1000000.0, -1000000.0)
        }
    }).collect()
}
