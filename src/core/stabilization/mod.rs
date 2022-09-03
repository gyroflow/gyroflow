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
pub mod distortion_models;
pub use pixel_formats::*;
pub use compute_params::ComputeParams;
pub use frame_transform::FrameTransform;
pub use cpu_undistort::{ undistort_points, undistort_points_with_params, undistort_points_with_rolling_shutter, COEFFS };

#[derive(Default, Clone, Copy)]
pub enum Interpolation {
    #[default]
    Bilinear = 2,
    Bicubic = 4,
    Lanczos4 = 8
}

bitflags::bitflags! {
    #[derive(Default)]
    pub struct KernelParamsFlags: i32 {
        const FIX_COLOR_RANGE      = 1;
        const IS_GOPRO_SUPERVIEW   = 2;
        const FILL_WITH_BACKGROUND = 4;
    }
}

// Each parameter must be aligned to 4 bytes and whole struct to 16 bytes
// Must be kept in sync with: opencl_undistort.cl, wgpu_undistort.wgsl and qt_gpu/undistort.frag
#[repr(C, packed(4))]
#[derive(Default, Copy, Clone)]
pub struct KernelParams {
    pub width:             i32, // 4
    pub height:            i32, // 8
    pub stride:            i32, // 12
    pub output_width:      i32, // 16
    pub output_height:     i32, // 4
    pub output_stride:     i32, // 8
    pub matrix_count:      i32, // 12 - for rolling shutter correction. 1 = no correction, only main matrix
    pub interpolation:     i32, // 16
    pub background_mode:   i32, // 4
    pub flags:             i32, // 8
    pub bytes_per_pixel:   i32, // 12
    pub pix_element_count: i32, // 16
    pub background:    [f32; 4], // 16
    pub f:             [f32; 2], // 8  - focal length in pixels
    pub c:             [f32; 2], // 16 - lens center
    pub k:             [f32; 12], // 16,16,16 - distortion coefficients
    pub fov:           f32, // 4
    pub r_limit:       f32, // 8
    pub lens_correction_amount:   f32, // 12
    pub input_vertical_stretch:   f32, // 16
    pub input_horizontal_stretch: f32, // 4
    pub background_margin:        f32, // 8
    pub background_margin_feather:f32, // 12
    pub reserved1:                f32, // 16
    pub reserved2:                f32, // 4
    pub reserved3:                f32, // 8
    pub translation2d:         [f32; 2], // 16
    pub translation3d:         [f32; 4], // 16
}
unsafe impl bytemuck::Zeroable for KernelParams {}
unsafe impl bytemuck::Pod for KernelParams {}

#[derive(Default)]
pub struct Stabilization<T: PixelType> {
    pub stab_data: BTreeMap<i64, FrameTransform>,

    size:        (usize, usize, usize), // width, height, stride
    output_size: (usize, usize, usize), // width, height, stride
    pub background: Vector4<f32>,

    pub interpolation: Interpolation,
    pub kernel_flags: KernelParamsFlags,

    #[cfg(feature = "use-opencl")]
    cl: Option<opencl::OclWrapper>,

    wgpu: Option<wgpu::WgpuWrapper>,

    backend_initialized: Option<(usize, usize, usize,   usize, usize, usize,   usize, usize)>, // (in_w, in_h, in_s,  out_w, out_h, out_s,  in_bytes, out_bytes)

    pub gpu_list: Vec<String>,

    pub current_fov: f64,
    compute_params: ComputeParams,

    _d: std::marker::PhantomData<T>
}

impl<T: PixelType> Stabilization<T> {
    pub fn set_compute_params(&mut self, params: ComputeParams) {
        self.stab_data.clear();
        self.compute_params = params;
        self.kernel_flags.set(KernelParamsFlags::IS_GOPRO_SUPERVIEW, self.compute_params.is_superview);
    }

    pub fn ensure_stab_data_at_timestamp(&mut self, timestamp_us: i64) {
        if !self.stab_data.contains_key(&timestamp_us) {
            let timestamp_ms = (timestamp_us as f64) / 1000.0;
            let frame = crate::frame_at_timestamp(timestamp_ms, self.compute_params.gyro.fps) as usize; // Only for FOVs

            let mut transform = FrameTransform::at_timestamp(&self.compute_params, timestamp_ms, frame);
            transform.kernel_params.interpolation = self.interpolation as i32;
            transform.kernel_params.width  = self.size.0 as i32;
            transform.kernel_params.height = self.size.1 as i32;
            transform.kernel_params.stride = self.size.2 as i32;
            transform.kernel_params.output_width  = self.output_size.0 as i32;
            transform.kernel_params.output_height = self.output_size.1 as i32;
            transform.kernel_params.output_stride = self.output_size.2 as i32;
            transform.kernel_params.background = [self.background[0], self.background[1], self.background[2], self.background[3]];
            transform.kernel_params.bytes_per_pixel = (T::COUNT * T::SCALAR_BYTES) as i32;
            transform.kernel_params.pix_element_count = T::COUNT as i32;
            transform.kernel_params.flags = self.kernel_flags.bits();

            self.stab_data.insert(timestamp_us, transform);
        }
    }

    pub fn init_size(&mut self, bg: Vector4<f32>, size: (usize, usize, usize), output_size: (usize, usize, usize)) {
        self.background = bg;

        #[cfg(feature = "use-opencl")]
        if self.cl  .is_some() { self.backend_initialized = None; }
        if self.wgpu.is_some() { self.backend_initialized = None; }

        self.size = size;
        self.output_size = output_size;
        self.stab_data.clear();
    }

    pub fn set_background(&mut self, bg: Vector4<f32>) {
        self.background = bg;
        self.stab_data.clear();
    }

    pub fn get_undistortion_data(&mut self, timestamp_us: i64) -> Option<&FrameTransform> {
        self.ensure_stab_data_at_timestamp(timestamp_us);
        self.stab_data.get(&timestamp_us)
    }

    pub fn list_devices(&self) -> Vec<String> {
        let mut ret = Vec::new();

        #[cfg(feature = "use-opencl")]
        if std::env::var("NO_OPENCL").unwrap_or_default().is_empty() {
            ret.extend(opencl::OclWrapper::list_devices().into_iter().map(|x| format!("[OpenCL] {x}")));
        }
        if std::env::var("NO_WGPU").unwrap_or_default().is_empty() {
            ret.extend(wgpu::WgpuWrapper::list_devices().into_iter().map(|x| format!("[wgpu] {x}")));
        }
        ret
    }

    pub fn set_device(&mut self, i: isize) -> bool {
        if i < 0 { // CPU
            #[cfg(feature = "use-opencl")]
            { self.cl = None; }
            self.wgpu = None;
            self.backend_initialized = Some(Default::default());
            return true;
        }
        if let Some(name) = self.gpu_list.get(i as usize) {
            if name.starts_with("[OpenCL]") {
                self.backend_initialized = None;
                #[cfg(feature = "use-opencl")]
                match opencl::OclWrapper::set_device(i as usize) {
                    Ok(_) => { return true; },
                    Err(e) => {
                        log::error!("Failed to set OpenCL device {}: {:?}", name, e);
                    }
                }
            } else if name.starts_with("[wgpu]") {
                self.backend_initialized = None;
                let first_ind = self.gpu_list.iter().enumerate().find(|(_, m)| m.starts_with("[wgpu]")).map(|(idx, _)| idx).unwrap_or(0);
                let wgpu_ind = i - first_ind as isize;
                if wgpu_ind >= 0 {
                    match wgpu::WgpuWrapper::set_device(wgpu_ind as usize) {
                        Some(_) => { return true; },
                        None => {
                            log::error!("Failed to set wgpu device {}", name);
                        }
                    }
                }
            }
        }
        return false;
    }

    pub fn init_backends(&mut self, timestamp_us: i64, size: (usize, usize, usize), output_size: (usize, usize, usize), in_len: usize, out_len: usize) {
        let tuple = (
            size.0, size.1, size.2,
            output_size.0, output_size.1, output_size.2,
            in_len, out_len
        );
        if self.backend_initialized.is_none() || self.backend_initialized.unwrap() != tuple {
            let mut gpu_initialized = false;
            if let Some(itm) = self.stab_data.get(&timestamp_us) {
                let params = itm.kernel_params;

                #[cfg(feature = "use-opencl")]
                if std::env::var("NO_OPENCL").unwrap_or_default().is_empty() {
                    let cl = std::panic::catch_unwind(|| {
                        opencl::OclWrapper::new(&params, T::ocl_names(), self.compute_params.distortion_model.opencl_functions(), size, output_size, in_len, out_len)
                    });
                    match cl {
                        Ok(Ok(cl)) => { self.cl = Some(cl); gpu_initialized = true; },
                        Ok(Err(e)) => { log::error!("OpenCL error: {:?}", e); },
                        Err(e) => {
                            if let Some(s) = e.downcast_ref::<&str>() {
                                log::error!("Failed to initialize OpenCL {}", s);
                            } else if let Some(s) = e.downcast_ref::<String>() {
                                log::error!("Failed to initialize OpenCL {}", s);
                            } else {
                                log::error!("Failed to initialize OpenCL {:?}", e);
                            }
                        }
                    }
                }
                if !gpu_initialized && T::wgpu_format().is_some() && std::env::var("NO_WGPU").unwrap_or_default().is_empty() {
                    let wgpu = std::panic::catch_unwind(|| {
                        wgpu::WgpuWrapper::new(&params, T::wgpu_format().unwrap(), self.compute_params.distortion_model.wgsl_functions(), size, output_size, in_len, out_len)
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

                self.backend_initialized = Some(tuple);
            }
        }
    }

    pub fn process_pixels(&mut self, timestamp_us: i64, size: (usize, usize, usize), output_size: (usize, usize, usize), pixels: &mut [u8], out_pixels: &mut [u8]) -> bool {
        if self.size != size || self.output_size != output_size || size.1 < 4 || output_size.1 < 4 { return false; }

        self.ensure_stab_data_at_timestamp(timestamp_us);
        self.init_backends(timestamp_us, size, output_size, pixels.len(), out_pixels.len());

        if let Some(itm) = self.stab_data.get(&timestamp_us) {
            self.current_fov = itm.fov;

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
                Interpolation::Bilinear => { Self::undistort_image_cpu::<2>(pixels, out_pixels, &itm.kernel_params, &self.compute_params.distortion_model, &itm.matrices); },
                Interpolation::Bicubic  => { Self::undistort_image_cpu::<4>(pixels, out_pixels, &itm.kernel_params, &self.compute_params.distortion_model, &itm.matrices); },
                Interpolation::Lanczos4 => { Self::undistort_image_cpu::<8>(pixels, out_pixels, &itm.kernel_params, &self.compute_params.distortion_model, &itm.matrices); },
            }

            return true;
        }
        false
    }
}

unsafe impl<T: PixelType> Send for Stabilization<T> { }
unsafe impl<T: PixelType> Sync for Stabilization<T> { }
