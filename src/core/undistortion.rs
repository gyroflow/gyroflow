use std::collections::BTreeMap;

use nalgebra::{Matrix3, Vector4};
use rayon::prelude::*;

use super::GyroSource;
#[cfg(feature = "use-opencl")]
use super::gpu::opencl;
use super::gpu::wgpu;
use super::StabilizationManager;

#[derive(Default, Clone)]
pub struct ComputeParams {
    pub gyro: GyroSource,
    pub fovs: Vec<f64>,

    pub frame_count: usize,
    pub fov_scale: f64,
    pub width: usize,
    pub height: usize, 
    pub output_width: usize,
    pub output_height: usize,
    pub calib_width: f64,
    pub calib_height: f64,
    pub video_rotation: f64,
    pub camera_matrix: Matrix3<f64>,
    pub distortion_coeffs: [f64; 4],
    pub frame_readout_time: f64,
    pub trim_start_frame: usize,
    pub trim_end_frame: usize,
    pub framebuffer_inverted: bool,
}
impl ComputeParams {
    pub fn from_manager<T: PixelType>(mgr: &StabilizationManager<T>) -> Self {
        let params = mgr.params.read();
        let lens = mgr.lens.read();

        let camera_matrix = lens.get_camera_matrix(params.size);
        let distortion_coeffs = lens.get_distortion_coeffs();
        let distortion_coeffs = [distortion_coeffs[0], distortion_coeffs[1], distortion_coeffs[2], distortion_coeffs[3]];

        let (calib_width, calib_height) = if lens.calib_dimension.w > 0 && lens.calib_dimension.h > 0 {
            (lens.calib_dimension.w as f64, lens.calib_dimension.h as f64)
        } else {
            (params.size.0.max(1) as f64, params.size.1.max(1) as f64)
        };

        Self {
            gyro: mgr.gyro.read().clone(), // TODO: maybe not clone?

            frame_count: params.frame_count,
            fov_scale: params.fov / (params.size.0.max(1) as f64 / calib_width),
            fovs: params.fovs.clone(),
            width: params.size.0.max(1),
            height: params.size.1.max(1),
            output_width: params.output_size.0.max(1),
            output_height: params.output_size.1.max(1),
            calib_width,
            calib_height,
            camera_matrix,
            video_rotation: params.video_rotation,
            distortion_coeffs,
            framebuffer_inverted: params.framebuffer_inverted,
            frame_readout_time: params.frame_readout_time,
            trim_start_frame: (params.trim_start * params.frame_count as f64).floor() as usize,
            trim_end_frame: (params.trim_end * params.frame_count as f64).ceil() as usize,
        }
    }
}

#[derive(Default, Clone)]
pub struct FrameTransform {
    pub params: Vec<[f32; 9]>,
}

impl FrameTransform {
    pub fn at_timestamp(params: &ComputeParams, timestamp_ms: f64, frame: usize) -> Self {
        let img_dim_ratio = params.width as f64 / params.calib_width;
    
        let k = params.camera_matrix;
        let scaled_k = k * img_dim_ratio;
        
        let out_dim = (params.output_width as f64, params.output_height as f64);
        let focal_center = (params.calib_width / 2.0, params.calib_height / 2.0);

        let fov = if params.fovs.len() > frame { params.fovs[frame] * params.fov_scale } else { params.fov_scale }.max(0.001);

        let mut new_k = k;
        new_k[(0, 0)] = new_k[(0, 0)] * 1.0 / fov;
        new_k[(1, 1)] = new_k[(1, 1)] * 1.0 / fov;
        new_k[(0, 2)] = (params.calib_width  / 2.0 - focal_center.0) * img_dim_ratio / fov + out_dim.0 / 2.0;
        new_k[(1, 2)] = (params.calib_height / 2.0 - focal_center.1) * img_dim_ratio / fov + out_dim.1 / 2.0;
        
        // ----------- Rolling shutter correction -----------
        let mut frame_readout_time = params.frame_readout_time;
        if params.framebuffer_inverted {
            frame_readout_time *= -1.0;
        }
        frame_readout_time *= fov;
        frame_readout_time /= 2.0;
        frame_readout_time *= img_dim_ratio;

        let row_readout_time = frame_readout_time / params.height as f64;
        let start_ts = timestamp_ms - (frame_readout_time / 2.0);
        // ----------- Rolling shutter correction -----------

        let image_rotation = Matrix3::new_rotation(params.video_rotation * (std::f64::consts::PI / 180.0));

        let quat1 = params.gyro.org_quat_at_timestamp(timestamp_ms).inverse();

        // Only compute 1 matrix if not using rolling shutter correction
        let rows = if frame_readout_time.abs() > 0.0 { params.height } else { 1 };

        let mut transform_params = (0..rows).into_par_iter().map(|y| {
            let quat_time = if frame_readout_time.abs() > 0.0 && timestamp_ms > 0.0 {
                start_ts + row_readout_time * y as f64
            } else {
                timestamp_ms
            };
            let quat = quat1
                     * params.gyro.org_quat_at_timestamp(quat_time)
                     * params.gyro.smoothed_quat_at_timestamp(quat_time);

            let mut r = image_rotation * *quat.to_rotation_matrix().matrix();
            if params.framebuffer_inverted {
                r[(0, 2)] *= -1.0; r[(1, 2)] *= -1.0;
                r[(2, 0)] *= -1.0; r[(2, 1)] *= -1.0;
            } else {
                r[(0, 1)] *= -1.0; r[(0, 2)] *= -1.0;
                r[(1, 0)] *= -1.0; r[(2, 0)] *= -1.0;
            }
            
            let i_r = (new_k * r).pseudo_inverse(0.000001);
            if let Err(err) = i_r {
                log::error!("Failed to multiply matrices: {:?} * {:?}: {}", new_k, r, err);
            }
            let i_r: Matrix3<f32> = nalgebra::convert(i_r.unwrap_or_default());
            [
                i_r[(0, 0)], i_r[(0, 1)], i_r[(0, 2)], 
                i_r[(1, 0)], i_r[(1, 1)], i_r[(1, 2)], 
                i_r[(2, 0)], i_r[(2, 1)], i_r[(2, 2)],
            ]
        }).collect::<Vec<[f32; 9]>>();

        // Prepend lens params at the beginning
        transform_params.insert(0, [
            scaled_k[(0, 0)] as f32, scaled_k[(1, 1)] as f32, // 1, 2 - f
            scaled_k[(0, 2)] as f32, scaled_k[(1, 2)] as f32, // 3, 4 - c
    
            params.distortion_coeffs[0] as f32, // 5
            params.distortion_coeffs[1] as f32, // 6
            params.distortion_coeffs[2] as f32, // 7
            params.distortion_coeffs[3] as f32, // 8
            0.0 // pad to 9 values
        ]);

        Self { params: transform_params }
    }

    pub fn at_timestamp_for_points(params: &ComputeParams, points: &[(f64, f64)], timestamp_ms: f64) -> (Matrix3<f64>, [f64; 4], Matrix3<f64>, Vec<Matrix3<f64>>) { // camera_matrix, dist_coeffs, p, rotations_per_point
        let img_dim_ratio = params.width as f64 / params.calib_width;
    
        let k = params.camera_matrix;
        let scaled_k = k * img_dim_ratio;
        
        let out_dim = (params.output_width as f64, params.output_height as f64);
        let focal_center = (params.calib_width / 2.0, params.calib_height / 2.0);

        let fov = params.fov_scale;

        let mut new_k = k;
        new_k[(0, 0)] = new_k[(0, 0)] * 1.0 / fov;
        new_k[(1, 1)] = new_k[(1, 1)] * 1.0 / fov;
        new_k[(0, 2)] = (params.calib_width  / 2.0 - focal_center.0) * img_dim_ratio / fov + out_dim.0 / 2.0;
        new_k[(1, 2)] = (params.calib_height / 2.0 - focal_center.1) * img_dim_ratio / fov + out_dim.1 / 2.0;

        // ----------- Rolling shutter correction -----------
        let mut frame_readout_time = params.frame_readout_time;
        frame_readout_time *= fov;
        frame_readout_time /= 2.0;
        frame_readout_time *= img_dim_ratio;

        let row_readout_time = frame_readout_time / params.height as f64;
        let start_ts = timestamp_ms - (frame_readout_time / 2.0);
        // ----------- Rolling shutter correction -----------

        let image_rotation = Matrix3::new_rotation(params.video_rotation * (std::f64::consts::PI / 180.0));

        let quat1 = params.gyro.org_quat_at_timestamp(timestamp_ms).inverse();

        // Only compute 1 matrix if not using rolling shutter correction
        let points_iter = if frame_readout_time.abs() > 0.0 { points } else { &[(0.0, 0.0)] };

        let rotations: Vec<Matrix3<f64>> = points_iter.into_iter().map(|&(_, y)| {
            let quat_time = if frame_readout_time.abs() > 0.0 && timestamp_ms > 0.0 {
                start_ts + row_readout_time * y as f64
            } else {
                timestamp_ms
            };
            let quat = quat1
                     * params.gyro.org_quat_at_timestamp(quat_time)
                     * params.gyro.smoothed_quat_at_timestamp(quat_time);

            let mut r = image_rotation * *quat.to_rotation_matrix().matrix();
            r[(0, 1)] *= -1.0; r[(0, 2)] *= -1.0;
            r[(1, 0)] *= -1.0; r[(2, 0)] *= -1.0;
            
            new_k * r
        }).collect();

        (scaled_k, params.distortion_coeffs, new_k, rotations)
    }
}

#[derive(Default)]
pub struct Undistortion<T: PixelType> {
    stab_data: BTreeMap<i64, FrameTransform>,

    size: (usize, usize, usize), // width, height, stride
    output_size: (usize, usize, usize), // width, height, stride
    pub background: Vector4<f32>,

    #[cfg(feature = "use-opencl")]
    cl: Option<opencl::OclWrapper>,

    wgpu: Option<wgpu::WgpuWrapper>,

    backend_initialized: bool,

    empty_frame_transform: FrameTransform,
    compute_params: ComputeParams,

    _d: std::marker::PhantomData<T>
}

impl<T: PixelType> Undistortion<T> {
    pub fn set_compute_params(&mut self, params: ComputeParams) {
        self.stab_data.clear();
        self.compute_params = params;
    }

    pub fn get_stab_data_at_timestamp(&mut self, timestamp_us: i64) -> &FrameTransform {
        use std::collections::btree_map::Entry;

        if let Entry::Vacant(e) = self.stab_data.entry(timestamp_us) {
            let timestamp_ms = (timestamp_us as f64) / 1000.0;
            let frame = crate::frame_at_timestamp(timestamp_ms, self.compute_params.gyro.fps) as usize; // Only for FOVs
            e.insert(FrameTransform::at_timestamp(&self.compute_params, timestamp_ms, frame));
        }
        if let Some(e) = self.stab_data.get(&timestamp_us) {
            return e;
        } else {
            ::log::error!("Failed to get stab data at timestamp: {}, stab_data.len: {}", timestamp_us, self.stab_data.len());
        }
        &self.empty_frame_transform
    }

    pub fn init_size(&mut self, bg: Vector4<f32>, size: (usize, usize), stride: usize, output_size: (usize, usize), output_stride: usize) {
        self.background = bg;
        self.backend_initialized = false;

        self.size = (size.0, size.1, stride);
        self.output_size = (output_size.0, output_size.1, output_stride);
    }

    pub fn set_background(&mut self, bg: Vector4<f32>) {
        self.background = bg;
        if let Some(ref mut wgpu) = self.wgpu {
            wgpu.set_background(bg);
        }
        #[cfg(feature = "use-opencl")]
        if let Some(ref mut cl) = self.cl {
            let _ = cl.set_background(bg);
        }
    }

    pub fn get_undistortion_data(&mut self, timestamp_us: i64) -> Option<FrameTransform> {
        Some(self.get_stab_data_at_timestamp(timestamp_us).clone())
    }

    pub fn init_backends(&mut self) {
        if !self.backend_initialized {
            #[cfg(feature = "use-opencl")]
            {
                let cl = opencl::OclWrapper::new(self.size.0, self.size.1, self.size.2, T::COUNT * T::SCALAR_BYTES, self.output_size.0, self.output_size.1, self.output_size.2, T::COUNT, T::ocl_names(), self.background);
                match cl {
                    Ok(cl) => self.cl = Some(cl),
                    Err(err) => {
                        log::error!("OpenCL error: {:?}", err);
                    }
                }
            }
            
            // TODO: Support other pixel types
            if self.cl.is_none() && T::COUNT == 4 && T::SCALAR_BYTES == 1 {
                let wgpu = wgpu::WgpuWrapper::new(self.size.0, self.size.1, self.size.2, T::COUNT * T::SCALAR_BYTES, self.output_size.0, self.output_size.1, self.output_size.2, T::COUNT, self.background);
                match wgpu {
                    Some(wgpu) => self.wgpu = Some(wgpu),
                    None => {
                        log::error!("Failed to initialize wgpu");
                    }
                }
            }
            self.backend_initialized = true;
        }
    }

    pub fn process_pixels(&mut self, timestamp_us: i64, width: usize, height: usize, stride: usize, output_width: usize, output_height: usize, output_stride: usize, pixels: &mut [u8], out_pixels: &mut [u8]) -> bool {
        if self.size.0 != width || self.size.1 != height || self.output_size.0 != output_width || self.output_size.1 != output_height || height < 4 || output_height < 4 { return false; }

        let itm = self.get_stab_data_at_timestamp(timestamp_us).clone(); // TODO: get rid of this clone
        if itm.params.is_empty() { return false; }

        self.init_backends();

        // OpenCL path
        #[cfg(feature = "use-opencl")]
        if let Some(ref mut cl) = self.cl {
            if let Err(err) = cl.undistort_image(pixels, out_pixels, &itm) {
                log::error!("OpenCL error: {:?}", err);
            } else {
                return true;
            }
        }

        if let Some(ref mut wgpu) = self.wgpu {
            wgpu.undistort_image(pixels, out_pixels, &itm);
            return true;
        }

        // CPU path
        Self::undistort_image_cpu(pixels, out_pixels, width, height, stride, output_width, output_height, output_stride, &itm.params, self.background);

        true
    }

    // TODO: optimize further with SIMD
    fn undistort_image_cpu(pixels: &mut [u8], out_pixels: &mut [u8], width: usize, height: usize, stride: usize, output_width: usize, output_height: usize, output_stride: usize, undistortion_params: &[[f32; 9]], bg: Vector4<f32>) {
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

pub fn undistort_points(distorted: &[(f64, f64)], camera_matrix: Matrix3<f64>, distortion_coeffs: &[f64], rotation: Matrix3<f64>, p: Matrix3<f64>, rot_per_point: Option<Vec<Matrix3<f64>>>) -> Vec<(f64, f64)> {
    let _time = std::time::Instant::now();
    
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

pub trait PixelType: Default + Copy + Send + Sync + bytemuck::Pod {
    const COUNT: usize = 1;
    const SCALAR_BYTES: usize = 1;
    type Scalar: Default + bytemuck::Pod;

    fn to_float(v: Self) -> Vector4<f32>;
    fn from_float(v: Vector4<f32>) -> Self;
    fn from_rgb_color(v: Vector4<f32>, ind: &[usize]) -> Vector4<f32>;
    fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str);
}

fn rgb_to_yuv(v: Vector4<f32>) -> Vector4<f32> {
    Vector4::new(
         0.299 * (v[0] / 255.0) + 0.587 * (v[1] / 255.0) + 0.114 * (v[2] / 255.0)/* + 0.0627*/,
        -0.147 * (v[0] / 255.0) - 0.289 * (v[1] / 255.0) + 0.436 * (v[2] / 255.0) + 0.5000,
         0.615 * (v[0] / 255.0) - 0.515 * (v[1] / 255.0) - 0.100 * (v[2] / 255.0) + 0.5000,
         v[3] / 255.0
    )
}

#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct Luma8(u8);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct Luma16(u16);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct RGBA8(u8, u8, u8, u8);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct RGBAf(f32, f32, f32, f32);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct UV8(u8, u8);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct UV16(u16, u16);

unsafe impl bytemuck::Zeroable for Luma8 { }
unsafe impl bytemuck::Pod for Luma8 { }
impl PixelType for Luma8 {
    const COUNT: usize = 1;
    const SCALAR_BYTES: usize = 1;
    type Scalar = u8;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, 0.0, 0.0, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize]) -> Vector4<f32> { Vector4::new(rgb_to_yuv(v)[ind[0]] * Self::Scalar::MAX as f32, 0.0, 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("uchar", "convert_uchar", "float", "convert_float") }
}
unsafe impl bytemuck::Zeroable for Luma16 { }
unsafe impl bytemuck::Pod for Luma16 { }
impl PixelType for Luma16 {
    const COUNT: usize = 1;
    const SCALAR_BYTES: usize = 2;
    type Scalar = u16;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, 0.0, 0.0, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize]) -> Vector4<f32> { Vector4::new(rgb_to_yuv(v)[ind[0]] * Self::Scalar::MAX as f32, 0.0, 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("ushort", "convert_ushort", "float", "convert_float") }
}
unsafe impl bytemuck::Zeroable for RGBA8 { }
unsafe impl bytemuck::Pod for RGBA8 { }
impl PixelType for RGBA8 {
    const COUNT: usize = 4;
    const SCALAR_BYTES: usize = 1;
    type Scalar = u8;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, v.2 as f32, v.3 as f32) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar, v[3] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize]) -> Vector4<f32> { v }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("uchar4", "convert_uchar4", "float4", "convert_float4") }
}
unsafe impl bytemuck::Zeroable for RGBAf { }
unsafe impl bytemuck::Pod for RGBAf { }
impl PixelType for RGBAf {
    const COUNT: usize = 4;
    const SCALAR_BYTES: usize = 4;
    type Scalar = f32;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0, v.1, v.2, v.3) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0], v[1], v[2], v[3]) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize]) -> Vector4<f32> { v }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("float4", "convert_float4", "float4", "convert_float4") }
}
unsafe impl bytemuck::Zeroable for UV8 { }
unsafe impl bytemuck::Pod for UV8 { }
impl PixelType for UV8 {
    const COUNT: usize = 2;
    const SCALAR_BYTES: usize = 1;
    type Scalar = u8;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, 0.0, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize]) -> Vector4<f32> { let yuv = rgb_to_yuv(v); Vector4::new(yuv[ind[0]] * Self::Scalar::MAX as f32, yuv[ind[1]] * Self::Scalar::MAX as f32, 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("uchar2", "convert_uchar2", "float2", "convert_float2") }
}
unsafe impl bytemuck::Zeroable for UV16 { }
unsafe impl bytemuck::Pod for UV16 { }
impl PixelType for UV16 {
    const COUNT: usize = 2;
    const SCALAR_BYTES: usize = 2;
    type Scalar = u16;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, 0.0, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize]) -> Vector4<f32> { let yuv = rgb_to_yuv(v); Vector4::new(yuv[ind[0]] * Self::Scalar::MAX as f32, yuv[ind[1]] * Self::Scalar::MAX as f32, 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("ushort2", "convert_ushort2", "float2", "convert_float2") }
}

unsafe impl<T: PixelType> Send for Undistortion<T> { }
unsafe impl<T: PixelType> Sync for Undistortion<T> { }
