// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::BTreeMap;

use nalgebra::Vector4;

#[cfg(feature = "use-opencl")]
use super::gpu::opencl;
use super::gpu::wgpu;
use super::StabilizationManager;

mod compute_params;
mod frame_transform;
mod cpu_undistort;
pub use compute_params::ComputeParams;
pub use frame_transform::FrameTransform;
pub use cpu_undistort::{ undistort_points, undistort_points_with_rolling_shutter };

#[derive(Clone, Copy)]
pub enum Interpolation {
    Bilinear = 2,
    Bicubic = 4, 
    Lanczos4 = 8
}
impl Default for Interpolation {
    fn default() -> Self { Interpolation::Bilinear }
}

#[derive(Default)]
pub struct Undistortion<T: PixelType> {
    stab_data: BTreeMap<i64, FrameTransform>,

    size: (usize, usize, usize), // width, height, stride
    output_size: (usize, usize, usize), // width, height, stride
    pub background: Vector4<f32>,

    pub interpolation: Interpolation,

    #[cfg(feature = "use-opencl")]
    cl: Option<opencl::OclWrapper>,

    wgpu: Option<wgpu::WgpuWrapper>,

    backend_initialized: bool,

    pub current_fov: f64,
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
        self.stab_data.clear();
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
        let interp = self.interpolation as u32;
        if !self.backend_initialized {
            let mut _opencl_initialized = false;
            #[cfg(feature = "use-opencl")]
            {
                let cl = std::panic::catch_unwind(|| {
                    opencl::OclWrapper::new(self.size.0, self.size.1, self.size.2, T::COUNT * T::SCALAR_BYTES, self.output_size.0, self.output_size.1, self.output_size.2, T::COUNT, T::ocl_names(), self.background, interp)
                });
                match cl {
                    Ok(Ok(cl)) => { self.cl = Some(cl); _opencl_initialized = true; },
                    Ok(Err(e)) => { log::error!("OpenCL error: {:?}", e); },
                    Err(e) => { log::error!("OpenCL error: {:?}", e); }
                }
            }

            // TODO: Support other pixel types
            if !_opencl_initialized && T::COUNT == 4 && T::SCALAR_BYTES == 1 {
                let wgpu = std::panic::catch_unwind(|| {
                    wgpu::WgpuWrapper::new(self.size.0, self.size.1, self.size.2, T::COUNT * T::SCALAR_BYTES, self.output_size.0, self.output_size.1, self.output_size.2, T::COUNT, self.background, interp)
                });
                match wgpu {
                    Ok(Some(wgpu)) => self.wgpu = Some(wgpu),
                    Err(e) => {
                        log::error!("Failed to initialize wgpu {:?}", e);
                    },
                    _ => {
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

        self.current_fov = itm.fov;

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
        match self.interpolation {
            Interpolation::Bilinear => { Self::undistort_image_cpu::<2>(pixels, out_pixels, width, height, stride, output_width, output_height, output_stride, &itm.params, self.background); },
            Interpolation::Bicubic  => { Self::undistort_image_cpu::<4>(pixels, out_pixels, width, height, stride, output_width, output_height, output_stride, &itm.params, self.background); },
            Interpolation::Lanczos4 => { Self::undistort_image_cpu::<8>(pixels, out_pixels, width, height, stride, output_width, output_height, output_stride, &itm.params, self.background); },
        }

        true
    }
}

pub trait PixelType: Default + Copy + Send + Sync + bytemuck::Pod {
    const COUNT: usize = 1;
    const SCALAR_BYTES: usize = 1;
    type Scalar: Default + bytemuck::Pod;

    fn to_float(v: Self) -> Vector4<f32>;
    fn from_float(v: Vector4<f32>) -> Self;
    fn from_rgb_color(v: Vector4<f32>, ind: &[usize], max_val: f32) -> Vector4<f32>;
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
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct RGB8(u8, u8, u8);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct RGBA8(u8, u8, u8, u8);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct RGB16(u16, u16, u16);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct RGBA16(u16, u16, u16, u16);
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
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize], max_val: f32) -> Vector4<f32> { Vector4::new(rgb_to_yuv(v)[ind[0]] * max_val, 0.0, 0.0, 0.0) }
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
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize], max_val: f32) -> Vector4<f32> { Vector4::new(rgb_to_yuv(v)[ind[0]] * max_val, 0.0, 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("ushort", "convert_ushort", "float", "convert_float") }
}
unsafe impl bytemuck::Zeroable for RGB8 { }
unsafe impl bytemuck::Pod for RGB8 { }
impl PixelType for RGB8 {
    const COUNT: usize = 3;
    const SCALAR_BYTES: usize = 1;
    type Scalar = u8;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, v.2 as f32, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize], _max_val: f32) -> Vector4<f32> { v }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("uchar3", "convert_uchar3", "float4", "convert_float4") }
}
unsafe impl bytemuck::Zeroable for RGBA8 { }
unsafe impl bytemuck::Pod for RGBA8 { }
impl PixelType for RGBA8 {
    const COUNT: usize = 4;
    const SCALAR_BYTES: usize = 1;
    type Scalar = u8;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, v.2 as f32, v.3 as f32) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar, v[3] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize], _max_val: f32) -> Vector4<f32> { v }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("uchar4", "convert_uchar4", "float4", "convert_float4") }
}
unsafe impl bytemuck::Zeroable for RGB16 { }
unsafe impl bytemuck::Pod for RGB16 { }
impl PixelType for RGB16 {
    const COUNT: usize = 3;
    const SCALAR_BYTES: usize = 1;
    type Scalar = u16;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, v.2 as f32, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize], _max_val: f32) -> Vector4<f32> { v }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("ushort3", "convert_ushort3", "float4", "convert_float4") }
}
unsafe impl bytemuck::Zeroable for RGBA16 { }
unsafe impl bytemuck::Pod for RGBA16 { }
impl PixelType for RGBA16 {
    const COUNT: usize = 4;
    const SCALAR_BYTES: usize = 1;
    type Scalar = u16;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, v.2 as f32, v.3 as f32) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar, v[3] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize], _max_val: f32) -> Vector4<f32> { v }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("ushort4", "convert_ushort4", "float4", "convert_float4") }
}
unsafe impl bytemuck::Zeroable for RGBAf { }
unsafe impl bytemuck::Pod for RGBAf { }
impl PixelType for RGBAf {
    const COUNT: usize = 4;
    const SCALAR_BYTES: usize = 4;
    type Scalar = f32;
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0, v.1, v.2, v.3) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0], v[1], v[2], v[3]) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize], _max_val: f32) -> Vector4<f32> { v }
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
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize], max_val: f32) -> Vector4<f32> { let yuv = rgb_to_yuv(v); Vector4::new(yuv[ind[0]] * max_val, yuv[ind[1]] * max_val, 0.0, 0.0) }
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
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize], max_val: f32) -> Vector4<f32> { let yuv = rgb_to_yuv(v); Vector4::new(yuv[ind[0]] * max_val, yuv[ind[1]] * max_val, 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("ushort2", "convert_ushort2", "float2", "convert_float2") }
}

unsafe impl<T: PixelType> Send for Undistortion<T> { }
unsafe impl<T: PixelType> Sync for Undistortion<T> { }
