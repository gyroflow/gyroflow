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
mod pixel_formats;
pub use pixel_formats::*;
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
            self.current_fov = e.fov;
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

    pub fn get_undistortion_data(&mut self, timestamp_us: i64) -> Option<&FrameTransform> {
        let itm = self.get_stab_data_at_timestamp(timestamp_us);
        if itm.params.is_empty() { return None; }
        Some(itm)
    }

    pub fn init_backends(&mut self) {
        let interp = self.interpolation as u32;
        if !self.backend_initialized {
            let mut gpu_initialized = false;

            #[cfg(feature = "use-opencl")]
            {
                let cl = std::panic::catch_unwind(|| {
                    opencl::OclWrapper::new(self.size.0, self.size.1, self.size.2, T::COUNT * T::SCALAR_BYTES, self.output_size.0, self.output_size.1, self.output_size.2, T::COUNT, T::ocl_names(), self.background, interp)
                });
                match cl {
                    Ok(Ok(cl)) => { self.cl = Some(cl); gpu_initialized = true; },
                    Ok(Err(e)) => { log::error!("OpenCL error: {:?}", e); },
                    Err(e) => { log::error!("OpenCL error: {:?}", e); }
                }
            }
            if !gpu_initialized && T::wgpu_format().is_some() {
                let wgpu = std::panic::catch_unwind(|| {
                    wgpu::WgpuWrapper::new(self.size.0, self.size.1, self.size.2, self.output_size.0, self.output_size.1, self.output_size.2, self.background, interp, T::wgpu_format().unwrap())
                });
                match wgpu {
                    Ok(Some(wgpu)) => { self.wgpu = Some(wgpu); },
                    Err(e) => {
                        if let Some(s) = e.downcast_ref::<&str>() {
                            log::error!("Failed to initialize wgpu {}", s);
                        } else if let Some(s) = e.downcast_ref::<String>() {
                            log::error!("Failed to initialize wgpu {}", s);
                        } else {
                            log::error!("Failed to initialize wgpu {:?}", e);
                        }
                    },
                    _ => { log::error!("Failed to initialize wgpu"); }
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

        // wgpu path
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

unsafe impl<T: PixelType> Send for Undistortion<T> { }
unsafe impl<T: PixelType> Sync for Undistortion<T> { }
