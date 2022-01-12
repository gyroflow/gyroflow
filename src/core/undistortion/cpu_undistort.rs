// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use super::{ PixelType, Undistortion, ComputeParams, FrameTransform };
use nalgebra::{ Vector4, Matrix3 };
use rayon::{ prelude::ParallelSliceMut, iter::{ ParallelIterator, IndexedParallelIterator } };

impl<T: PixelType> Undistortion<T> {
    // Adapted from OpenCV: initUndistortRectifyMap + remap 
    // https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/fisheye.cpp#L454
    // https://github.com/opencv/opencv/blob/4.x/modules/imgproc/src/opencl/remap.cl#L390
    pub fn undistort_image_cpu(pixels: &mut [u8], out_pixels: &mut [u8], width: usize, height: usize, stride: usize, output_width: usize, output_height: usize, output_stride: usize, undistortion_params: &[[f32; 9]], bg: Vector4<f32>) {
        let bg_t: T = PixelType::from_float(bg);
        
        const INTER_BITS: usize = 5;
        const INTER_TAB_SIZE: usize = 1 << INTER_BITS;
        
        const COEFFS: [f32; 64] = [
            1.000000, 0.000000, 0.968750, 0.031250, 0.937500, 0.062500, 0.906250, 0.093750, 0.875000, 0.125000, 0.843750, 0.156250,
            0.812500, 0.187500, 0.781250, 0.218750, 0.750000, 0.250000, 0.718750, 0.281250, 0.687500, 0.312500, 0.656250, 0.343750,
            0.625000, 0.375000, 0.593750, 0.406250, 0.562500, 0.437500, 0.531250, 0.468750, 0.500000, 0.500000, 0.468750, 0.531250,
            0.437500, 0.562500, 0.406250, 0.593750, 0.375000, 0.625000, 0.343750, 0.656250, 0.312500, 0.687500, 0.281250, 0.718750,
            0.250000, 0.750000, 0.218750, 0.781250, 0.187500, 0.812500, 0.156250, 0.843750, 0.125000, 0.875000, 0.093750, 0.906250,
            0.062500, 0.937500, 0.031250, 0.968750
        ];

        let f = &undistortion_params[0][0..2];
        let c = &undistortion_params[0][2..4];
        let k = &undistortion_params[0][4..];

        out_pixels.par_chunks_mut(output_stride).enumerate().for_each(|(y, row_bytes)| { // Parallel iterator over buffer rows
            row_bytes.chunks_mut(T::COUNT * T::SCALAR_BYTES).enumerate().for_each(|(x, pix_chunk)| { // iterator over row pixels
                if y < output_height && x < output_width {
                    assert!(pix_chunk.len() == std::mem::size_of::<T>());
                    let pix_out = bytemuck::from_bytes_mut(pix_chunk); // treat this byte chunk as `T`

                    let undistortion_params = undistortion_params[(y + 1).min(undistortion_params.len() - 1)];
                    let _x = y as f32 * undistortion_params[1] + undistortion_params[2] + (x as f32 * undistortion_params[0]);
                    let _y = y as f32 * undistortion_params[4] + undistortion_params[5] + (x as f32 * undistortion_params[3]);
                    let _w = y as f32 * undistortion_params[7] + undistortion_params[8] + (x as f32 * undistortion_params[6]);
                
                    if _w > 0.0 {
                        let posx = _x / _w;
                        let posy = _y / _w;
                
                        let r = (posx*posx + posy*posy).sqrt();
                        let theta = r.atan();

                        /*if r > 1.0 { // TODO add this maybe in lens profile?
                            *pix_out = bg_t;
                            return;
                        }*/
                
                        let theta2 = theta*theta;
                        let theta4 = theta2*theta2;
                        let theta6 = theta4*theta2;
                        let theta8 = theta4*theta4;
                
                        let theta_d = theta * (1.0 + k[0]*theta2 + k[1]*theta4 + k[2]*theta6 + k[3]*theta8);
                
                        let scale =  if r == 0.0 { 1.0 } else { theta_d / r };
                        let u = f[0] * posx * scale + c[0];
                        let v = f[1] * posy * scale + c[1];
                
                        let sx = ((0.5 + u * INTER_TAB_SIZE as f32).floor() as i32) >> INTER_BITS;
                        let sy = ((0.5 + v * INTER_TAB_SIZE as f32).floor() as i32) >> INTER_BITS;
                
                        let coeffs_x = &COEFFS[((u * INTER_TAB_SIZE as f32).round() as usize & (INTER_TAB_SIZE - 1)) << 1..];
                        let coeffs_y = &COEFFS[((v * INTER_TAB_SIZE as f32).round() as usize & (INTER_TAB_SIZE - 1)) << 1..];
                
                        let mut sum = Vector4::from_element(0.0);
                        let bytes_per_pixel = T::COUNT * T::SCALAR_BYTES;
                        let mut src_index = (sy * stride as i32 + sx * bytes_per_pixel as i32) as isize;

                        for yp in 0..2 {
                            if sy + yp >= 0 && sy + yp < height as i32 {
                                let xsum = 
                                    if sx >= 0 && sx < width as i32 {
                                        let px1: &T = bytemuck::from_bytes(&pixels[src_index as usize..src_index as usize + bytes_per_pixel]); 
                                        PixelType::to_float(*px1)
                                    } else { bg } * coeffs_x[0]
                                +  if sx + 1 >= 0 && sx + 1 < width as i32 {
                                        let px2: &T = bytemuck::from_bytes(&pixels[src_index as usize + bytes_per_pixel..src_index as usize + bytes_per_pixel*2]);
                                        PixelType::to_float(*px2)
                                    } else { bg } * coeffs_x[1];

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

    undistort_points(distorted, camera_matrix, &distortion_coeffs, rotations[0], Matrix3::identity(), Some(rotations))
}

// Ported from OpenCV: https://github.com/opencv/opencv/blob/4.x/modules/calib3d/src/fisheye.cpp#L321
pub fn undistort_points(distorted: &[(f64, f64)], camera_matrix: Matrix3<f64>, distortion_coeffs: &[f64], rotation: Matrix3<f64>, p: Matrix3<f64>, rot_per_point: Option<Vec<Matrix3<f64>>>) -> Vec<(f64, f64)> {
    let f = (camera_matrix[(0, 0)], camera_matrix[(1, 1)]);
    let c = (camera_matrix[(0, 2)], camera_matrix[(1, 2)]);
    let k = distortion_coeffs;
    
    let mut rr = rotation;
    if !p.is_empty() { // PP
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
