use std::sync::atomic::Ordering::Relaxed;

use nalgebra::*;
use rayon::prelude::*;

use super::GyroSource;
use super::gpu::opencl;
use super::gpu::wgpu;
use super::StabilizationManager;

#[derive(Clone)]
pub struct ComputeParams {
    gyro: GyroSource, // TODO: ref? threading?

    frame_count: usize, 
    fps: f64,
    fov_scale: f64,
    width: usize,
    height: usize, 
    calib_width: f64,
    calib_height: f64,
    camera_matrix: Matrix3<f64>,
    distortion_coeffs: [f32; 4],
    frame_readout_time: f64
}
impl ComputeParams {
    pub fn from_manager(mgr: &StabilizationManager) -> Self {
        let camera_matrix = Matrix3::from_row_slice(&mgr.camera_matrix_or_default());

        let distortion_coeffs = if mgr.lens.distortion_coeffs.len() >= 4 {
            [
                mgr.lens.distortion_coeffs[0] as f32, 
                mgr.lens.distortion_coeffs[1] as f32, 
                mgr.lens.distortion_coeffs[2] as f32, 
                mgr.lens.distortion_coeffs[3] as f32
            ]
        } else {
            [0.0, 0.0, 0.0, 0.0]
        };
        let (calib_width, calib_height) = if mgr.lens.calib_dimension.0 > 0.0 && mgr.lens.calib_dimension.1 > 0.0 {
            mgr.lens.calib_dimension
        } else {
            (mgr.size.0 as f64, mgr.size.1 as f64)
        };

        Self {
            gyro: mgr.gyro.clone(), // TODO: maybe not clone?

            frame_count: mgr.frame_count,
            fps: mgr.fps,
            fov_scale: mgr.fov / (mgr.size.0 as f64 / calib_width),
            width: mgr.size.0,
            height: mgr.size.1,
            calib_width,
            calib_height,
            camera_matrix,
            distortion_coeffs,
            frame_readout_time: mgr.frame_readout_time
        }
    }
}

pub struct FrameTransform {
    pub params: Vec<[f32; 9]>,
}

impl FrameTransform {
    pub fn at_timestamp(params: &ComputeParams, timestamp_ms: f64) -> Self {
        let img_dim_ratio = params.width as f64 / params.calib_width;
    
        let k = params.camera_matrix;
        let scaled_k = k * img_dim_ratio;
        
        let out_dim = (params.width as f64, params.height as f64);
        let focal_center = (params.calib_width / 2.0, params.calib_height / 2.0);

        let mut new_k = k;
        new_k[(0, 0)] = new_k[(0, 0)] * 1.0 / params.fov_scale;
        new_k[(1, 1)] = new_k[(1, 1)] * 1.0 / params.fov_scale;
        new_k[(0, 2)] = (params.calib_width  / 2.0 - focal_center.0) * img_dim_ratio / params.fov_scale + out_dim.0 / 2.0;
        new_k[(1, 2)] = (params.calib_height / 2.0 - focal_center.1) * img_dim_ratio / params.fov_scale + out_dim.1 / 2.0;

        // ----------- Rolling shutter correction -----------
        let mut frame_readout_time = params.frame_readout_time;
        frame_readout_time *= params.fov_scale;
        frame_readout_time /= 2.0;
        //frame_readout_time *= params.height as f64 / params.calib_height; // org_height
        frame_readout_time *= img_dim_ratio;

		let row_readout_time = frame_readout_time / params.height as f64;
		let start_ts = timestamp_ms - (frame_readout_time / 2.0);

        // ----------- Rolling shutter correction -----------

        let quat1 = params.gyro.org_quat_at_timestamp(timestamp_ms).inverse();

        // Only compute 1 matrix if not using rolling shutter correction
        let rows = if frame_readout_time > 0.0 { params.height } else { 1 };

        let mut transform_params = (0..rows).into_par_iter().map(|y| {
            let quat_time = if frame_readout_time > 0.0 && timestamp_ms > 0.0 {
                start_ts + row_readout_time * y as f64
            } else {
                timestamp_ms
            };
            let quat = quat1
                     * params.gyro.org_quat_at_timestamp(quat_time)
                     * params.gyro.smoothed_quat_at_timestamp(quat_time);

            let mut r = *quat.to_rotation_matrix().matrix();
            r[(0, 1)] *= -1.0; r[(0, 2)] *= -1.0;
            r[(1, 0)] *= -1.0; r[(2, 0)] *= -1.0;
            
            let i_r: Matrix3<f32> = nalgebra::convert((new_k * r).pseudo_inverse(0.000001).unwrap());
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
}

#[derive(Default)]
pub struct Undistortion<T: Default + Copy + Send + Sync + FloatPixel> {
    pub stab_data: Vec<FrameTransform>,

    size: (usize, usize, usize), // width, height, stride
    pub background: Vector4<f32>,

    cl: Option<opencl::OclWrapper<T::Scalar>>,
    wgpu: Option<wgpu::WgpuWrapper<T::Scalar>>,

    tmp_buffer: Vec<T>,
}

impl<T: Default + Copy + Send + Sync + FloatPixel> Undistortion<T> {
    pub fn calculate_stab_data(params: &ComputeParams, current_compute_id: &std::sync::atomic::AtomicU64, compute_id: u64) -> Result<Vec<FrameTransform>, ()> {
        if params.frame_count <= 0 || params.width <= 0 || params.height <= 0 {
            println!("no params {} {} {} ", params.frame_count, params.width, params.height);
            return Ok(Vec::new());
        }

        assert!(params.width > 0);
        assert!(params.height > 0);
        assert!(params.calib_width > 0.0);
        assert!(params.calib_height > 0.0);
        assert!(params.fov_scale > 0.0);
        assert!(params.fps > 0.0);

        let frame_middle = 0.0;//if params.frame_readout_time > 0.0 { params.frame_readout_time / (1000.0 / params.fps) } else { 0.5 }; // TODO: +0.5?
        //dbg!(frame_middle);

        let mut vec = Vec::with_capacity(params.frame_count);
        for i in 0..params.frame_count {
            if current_compute_id.load(Relaxed) != compute_id { return Err(()); }
            vec.push(FrameTransform::at_timestamp(params, (i as f64 + frame_middle) * 1000.0 / params.fps));
        }
        Ok(vec)
    }
    pub fn recompute(&mut self, params: &ComputeParams) {
        let _time = std::time::Instant::now();

        let a = std::sync::atomic::AtomicU64::new(0);
        self.stab_data = Self::calculate_stab_data(params, &a, 0).unwrap();

        println!("Computed in {:.3}ms", _time.elapsed().as_micros() as f64 / 1000.0);
    }
    pub fn init_size(&mut self, bg: Vector4<f32>, params: &ComputeParams, stride: usize) {
        self.background = bg;

        //self.wgpu = wgpu::WgpuWrapper::new(params.width, params.height, self.background);
        self.cl = Some(opencl::OclWrapper::new(params.width, params.height, stride, T::COUNT, T::ocl_names(), self.background).unwrap()); // TODO ok()

        self.size = (params.width, params.height, stride);

        self.recompute(params);
    }

    pub fn set_background(&mut self, bg: Vector4<f32>) {
        self.background = bg;
        if let Some(ref mut wgpu) = self.wgpu {
            wgpu.set_background(bg);
        } else if let Some(ref mut cl) = self.cl {
            let _ = cl.set_background(bg);
        }
    }

    pub fn process_pixels(&mut self, frame: usize, width: usize, height: usize, stride: usize, pixels: &mut [T::Scalar]) -> *mut T::Scalar {
        if self.stab_data.is_empty() || frame >= self.stab_data.len() || self.size.0 != width || self.size.1 != height { return pixels.as_mut_ptr(); }

        let itm = &self.stab_data[frame];

        if let Some(ref mut wgpu) = self.wgpu {
            wgpu.undistort_image(pixels, itm);
        } else if let Some(ref mut cl) = self.cl {
            cl.undistort_image(pixels, itm).unwrap();
        } else {
            Self::undistort_image_cpu( unsafe { std::mem::transmute(pixels) }, &mut self.tmp_buffer, width, height, stride, &itm.params, self.background);
            return self.tmp_buffer.as_mut_ptr() as *mut T::Scalar;
        }

        pixels.as_mut_ptr()
    }

    // TODO: optimize further with SIMD
    fn undistort_image_cpu(pixels: &mut [T], out_pixels: &mut Vec<T>, width: usize, height: usize, stride: usize, undistortion_params: &[[f32; 9]], bg: Vector4<f32>) {
        out_pixels.resize_with(stride*height, T::default);

        let bg_t = FloatPixel::from_float(bg);
        
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

        out_pixels.par_chunks_mut(stride).enumerate().for_each(|(y, row)| {
            row.iter_mut().enumerate().for_each(|(x, pix_out)| {
                if x < width {
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
                        let mut src_index = (sy * stride as i32 + sx) as usize;
                
                        for yp in 0..2 {
                            if sy + yp >= 0 && sy + yp < height as i32 {
                                let xsum = if sx >= 0 && sx < width as i32 { FloatPixel::to_float(pixels[src_index]) } else { bg } * coeffs_x[0]
                                        + if sx + 1 >= 0 && sx + 1 < width as i32 { FloatPixel::to_float(pixels[src_index + 1]) } else { bg } * coeffs_x[1];

                                sum += xsum * coeffs_y[yp as usize];
                            } else {
                                sum += bg * coeffs_y[yp as usize];
                            }
                            src_index += stride;
                        }
                        *pix_out = FloatPixel::from_float(sum);
                    } else {
                        *pix_out = bg_t;
                    }
                } else {
                    *pix_out = bg_t;
                }
            });
        });
    }
}

pub trait FloatPixel {
    const COUNT: usize = 1;
    type Scalar: ocl::OclPrm + bytemuck::Pod;
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

impl FloatPixel for Luma8 {
    const COUNT: usize = 1;
    type Scalar = u8;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, 0.0, 0.0, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize]) -> Vector4<f32> { Vector4::new(rgb_to_yuv(v)[ind[0]] * Self::Scalar::MAX as f32, 0.0, 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("uchar", "convert_uchar", "float", "convert_float") }
}
impl FloatPixel for Luma16 {
    const COUNT: usize = 1;
    type Scalar = u16;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, 0.0, 0.0, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize]) -> Vector4<f32> { Vector4::new(rgb_to_yuv(v)[ind[0]] * Self::Scalar::MAX as f32, 0.0, 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("ushort", "convert_ushort", "float", "convert_float") }
}
impl FloatPixel for RGBA8 {
    const COUNT: usize = 4;
    type Scalar = u8;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, v.2 as f32, v.3 as f32) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar, v[3] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize]) -> Vector4<f32> { v }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("uchar4", "convert_uchar4", "float4", "convert_float4") }
}
impl FloatPixel for RGBAf {
    const COUNT: usize = 4;
    type Scalar = f32;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0, v.1, v.2, v.3) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0], v[1], v[2], v[3]) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize]) -> Vector4<f32> { v }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("float4", "convert_float4", "float4", "convert_float4") }
}
impl FloatPixel for UV8 {
    const COUNT: usize = 2;
    type Scalar = u8;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, 0.0, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize]) -> Vector4<f32> { let yuv = rgb_to_yuv(v); Vector4::new(yuv[ind[0]] * Self::Scalar::MAX as f32, yuv[ind[1]] * Self::Scalar::MAX as f32, 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("uchar2", "convert_uchar2", "float2", "convert_float2") }
}
impl FloatPixel for UV16 {
    const COUNT: usize = 2;
    type Scalar = u16;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, 0.0, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize]) -> Vector4<f32> { let yuv = rgb_to_yuv(v); Vector4::new(yuv[ind[0]] * Self::Scalar::MAX as f32, yuv[ind[1]] * Self::Scalar::MAX as f32, 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("ushort2", "convert_ushort2", "float2", "convert_float2") }
}
