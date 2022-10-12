// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::BTreeMap;
use nalgebra::Vector4;
use std::cell::RefCell;

#[cfg(feature = "use-opencl")]
use super::gpu::opencl;
use super::gpu::*;
use super::StabilizationManager;
use drawing::DrawCanvas;

mod compute_params;
mod frame_transform;
mod cpu_undistort;
mod pixel_formats;
pub mod distortion_models;
pub use pixel_formats::*;
pub use compute_params::ComputeParams;
pub use frame_transform::FrameTransform;
pub use cpu_undistort::*;

#[derive(Default, Clone, Copy)]
pub enum Interpolation {
    #[default]
    Bilinear = 2,
    Bicubic = 4,
    Lanczos4 = 8
}

lazy_static::lazy_static! {
    pub static ref GPU_LIST: parking_lot::RwLock<Vec<String>> = parking_lot::RwLock::new(Vec::new());
}

bitflags::bitflags! {
    #[derive(Default)]
    pub struct KernelParamsFlags: i32 {
        const FIX_COLOR_RANGE      = 1;
        const HAS_DIGITAL_LENS     = 2;
        const FILL_WITH_BACKGROUND = 4;
        const DRAWING_ENABLED      = 8;
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
    pub canvas_scale:             f32, // 16
    pub reserved2:                f32, // 4
    pub reserved3:                f32, // 8
    pub translation2d:         [f32; 2], // 16
    pub translation3d:         [f32; 4], // 16
    pub source_rect:           [i32; 4], // 16 - x, y, w, h
    pub output_rect:           [i32; 4], // 16 - x, y, w, h
    pub digital_lens_params:   [f32; 4], // 16
}
unsafe impl bytemuck::Zeroable for KernelParams {}
unsafe impl bytemuck::Pod for KernelParams {}

#[derive(Default)]
pub struct Stabilization<T: PixelType> {
    pub stab_data: BTreeMap<i64, FrameTransform>,
    last_frame_data: RefCell<FrameTransform>,

    size:        (usize, usize), // width, height
    output_size: (usize, usize), // width, height
    pub background: Vector4<f32>,

    pub interpolation: Interpolation,
    pub kernel_flags: KernelParamsFlags,

    #[cfg(feature = "use-opencl")]
    cl: Option<opencl::OclWrapper>,

    wgpu: Option<wgpu::WgpuWrapper>,

    backend_initialized: Option<(usize, usize, usize,   usize, usize, usize,   u32, u32)>, // (in_w, in_h, in_s,  out_w, out_h, out_s, buffer_checksum, lens models)

    compute_params: ComputeParams,

    pub drawing: DrawCanvas,
    pub pending_device_change: Option<isize>,
    next_backend: Option<&'static str>,

    _d: std::marker::PhantomData<T>
}

#[derive(Debug)]
pub struct ProcessedInfo {
    pub fov: f64,
    pub backend: &'static str,
}

impl<T: PixelType> Stabilization<T> {
    pub fn set_compute_params(&mut self, params: ComputeParams) {
        self.stab_data.clear();
        self.compute_params = params;
    }

    pub fn ensure_stab_data_at_timestamp(&mut self, timestamp_us: i64, buffers: Option<&mut BufferDescription>) {
        if !self.stab_data.contains_key(&timestamp_us) {
            // We need buffers to set the stab data, otherwise we have invalid sizes and strides
            if let Some(buffers) = buffers {
                let timestamp_ms = (timestamp_us as f64) / 1000.0;
                let frame = crate::frame_at_timestamp(timestamp_ms, self.compute_params.gyro.fps) as usize; // Only for FOVs

                self.kernel_flags.set(KernelParamsFlags::HAS_DIGITAL_LENS, self.compute_params.digital_lens.is_some());

                let mut transform = FrameTransform::at_timestamp(&self.compute_params, timestamp_ms, frame);
                transform.kernel_params.interpolation = self.interpolation as i32;
                transform.kernel_params.width  = self.size.0 as i32;
                transform.kernel_params.height = self.size.1 as i32;
                transform.kernel_params.output_width  = self.output_size.0 as i32;
                transform.kernel_params.output_height = self.output_size.1 as i32;
                transform.kernel_params.background = [self.background[0], self.background[1], self.background[2], self.background[3]];
                transform.kernel_params.bytes_per_pixel = (T::COUNT * T::SCALAR_BYTES) as i32;
                transform.kernel_params.pix_element_count = T::COUNT as i32;
                transform.kernel_params.canvas_scale = self.drawing.scale as f32;
                transform.kernel_params.flags = self.kernel_flags.bits();

                transform.kernel_params.stride        = buffers.input_size.2 as i32;
                transform.kernel_params.output_stride = buffers.output_size.2 as i32;

                if let Some(r) = buffers.input_rect {
                    transform.kernel_params.source_rect[0] = r.0 as i32;
                    transform.kernel_params.source_rect[1] = r.1 as i32;
                    transform.kernel_params.source_rect[2] = r.2 as i32;
                    transform.kernel_params.source_rect[3] = r.3 as i32;
                } else {
                    // Stretch to the buffer by default
                    transform.kernel_params.source_rect[0] = 0;
                    transform.kernel_params.source_rect[1] = 0;
                    transform.kernel_params.source_rect[2] = buffers.input_size.0  as i32;
                    transform.kernel_params.source_rect[3] = buffers.input_size.1  as i32;
                }
                if let Some(r) = buffers.output_rect {
                    transform.kernel_params.output_rect[0] = r.0 as i32;
                    transform.kernel_params.output_rect[1] = r.1 as i32;
                    transform.kernel_params.output_rect[2] = r.2 as i32;
                    transform.kernel_params.output_rect[3] = r.3 as i32;
                } else {
                    // Stretch to the buffer by default
                    transform.kernel_params.output_rect[0] = 0;
                    transform.kernel_params.output_rect[1] = 0;
                    transform.kernel_params.output_rect[2] = buffers.output_size.0 as i32;
                    transform.kernel_params.output_rect[3] = buffers.output_size.1 as i32;
                }

                self.stab_data.insert(timestamp_us, transform);
            }
        }
    }

    pub fn init_size(&mut self, bg: Vector4<f32>, size: (usize, usize), output_size: (usize, usize)) {
        self.background = bg;

        self.backend_initialized = None;
        #[cfg(feature = "use-opencl")]
        { self.cl = None; }
        self.wgpu = None;

        self.size = size;
        self.output_size = output_size;

        if self.kernel_flags.contains(KernelParamsFlags::DRAWING_ENABLED) {
            self.drawing = DrawCanvas::new(size.0, size.1, output_size.0, output_size.1, (size.1 as f64 / 720.0).max(1.0) as usize);
        }

        self.stab_data.clear();
    }

    pub fn set_background(&mut self, bg: Vector4<f32>) {
        self.background = bg;
        self.stab_data.clear();
    }

    pub fn get_undistortion_data(&self, timestamp_us: i64) -> Option<&FrameTransform> {
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

    pub fn set_device(&mut self, i: isize) {
        self.pending_device_change = Some(i);
    }

    pub fn update_device(&mut self, i: isize, buffers: &BufferDescription) -> bool {
        self.next_backend = None;
        self.backend_initialized = None;
        #[cfg(feature = "use-opencl")]
        { self.cl = None; }
        self.wgpu = None;

        let tuple = (
            buffers.input_size.0, buffers.input_size.1, buffers.input_size.2,
            buffers.output_size.0, buffers.output_size.1, buffers.output_size.2,
            buffers.buffers.get_checksum(),
            crc32fast::hash(format!("{}{}", self.compute_params.distortion_model.id(), self.compute_params.digital_lens.as_ref().map(|x| x.id()).unwrap_or_default()).as_bytes())
        );
        if i < 0 { // CPU
            #[cfg(feature = "use-opencl")]
            { self.cl = None; }
            self.wgpu = None;
            self.backend_initialized = Some(tuple);
            return true;
        }
        let gpu_list = GPU_LIST.read();
        if let Some(name) = gpu_list.get(i as usize) {
            if name.starts_with("[OpenCL]") {
                self.backend_initialized = None;
                #[cfg(feature = "use-opencl")]
                match opencl::OclWrapper::set_device(i as usize, buffers) {
                    Ok(_) => { self.next_backend = Some("opencl"); return true; },
                    Err(e) => {
                        log::error!("Failed to set OpenCL device {}: {:?}", name, e);
                    }
                }
            } else if name.starts_with("[wgpu]") {
                self.backend_initialized = None;
                let first_ind = gpu_list.iter().enumerate().find(|(_, m)| m.starts_with("[wgpu]")).map(|(idx, _)| idx).unwrap_or(0);
                let wgpu_ind = i - first_ind as isize;
                if wgpu_ind >= 0 {
                    match wgpu::WgpuWrapper::set_device(wgpu_ind as usize, buffers) {
                        Some(_) => { self.next_backend = Some("wgpu"); return true; },
                        None => {
                            log::error!("Failed to set wgpu device {}", name);
                        }
                    }
                }
            }
        }
        false
    }

    pub fn init_backends(&mut self, timestamp_us: i64, buffers: &BufferDescription) {
        let tuple = (
            buffers.input_size.0, buffers.input_size.1, buffers.input_size.2,
            buffers.output_size.0, buffers.output_size.1, buffers.output_size.2,
            buffers.buffers.get_checksum(),
            crc32fast::hash(format!("{}{}", self.compute_params.distortion_model.id(), self.compute_params.digital_lens.as_ref().map(|x| x.id()).unwrap_or_default()).as_bytes())
        );
        if self.backend_initialized.is_none() || self.backend_initialized.unwrap() != tuple {
            let mut gpu_initialized = false;
            if let Some(itm) = self.stab_data.get(&timestamp_us) {
                let params = itm.kernel_params;
                let canvas_len = self.drawing.get_buffer_len();
                let next_backend = self.next_backend.take().unwrap_or_default();

                #[cfg(feature = "use-opencl")]
                if std::env::var("NO_OPENCL").unwrap_or_default().is_empty() && next_backend != "wgpu" && opencl::is_buffer_supported(buffers) {
                    self.cl = None;
                    let cl = std::panic::catch_unwind(|| {
                        opencl::OclWrapper::new(&params, T::ocl_names(), &self.compute_params, buffers, canvas_len)
                    });
                    match cl {
                        Ok(Ok(cl)) => { self.cl = Some(cl); gpu_initialized = true; log::info!("Initialized OpenCL for {:?} -> {:?}", buffers.input_size, buffers.output_size); },
                        Ok(Err(e)) => { log::error!("OpenCL error init_backends: {:?}", e); },
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
                if !gpu_initialized && T::wgpu_format().is_some() && next_backend != "opencl" && std::env::var("NO_WGPU").unwrap_or_default().is_empty() && wgpu::is_buffer_supported(buffers) {
                    self.wgpu = None;
                    let wgpu = std::panic::catch_unwind(|| {
                        wgpu::WgpuWrapper::new(&params, T::wgpu_format().unwrap(), &self.compute_params, buffers, canvas_len)
                    });
                    match wgpu {
                        Ok(Some(wgpu)) => { self.wgpu = Some(wgpu); log::info!("Initialized wgpu for {:?} -> {:?}", buffers.input_size, buffers.output_size); },
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

    pub fn ensure_ready_for_processing(&mut self, timestamp_us: i64, buffers: &mut BufferDescription) {
        if let Some(dev) = self.pending_device_change.take() {
            log::debug!("Setting device {dev}");
            self.update_device(dev, buffers);
        }

        self.ensure_stab_data_at_timestamp(timestamp_us, Some(buffers));
        self.init_backends(timestamp_us, buffers);
    }
    pub fn process_pixels(&self, timestamp_us: i64, buffers: &mut BufferDescription) -> Option<ProcessedInfo> {
        if /*self.size != buffers.input_size || */buffers.input_size.1 < 4 || buffers.output_size.1 < 4 { return None; }

        let mut _last_frame_data = None;

        let itm = if self.stab_data.contains_key(&timestamp_us) {
            self.stab_data.get(&timestamp_us)
        } else {
            _last_frame_data = Some(self.last_frame_data.borrow().clone());
            _last_frame_data.as_ref()
        };

        if let Some(itm) = itm {
            let mut ret = ProcessedInfo {
                fov: itm.fov,
                backend: ""
            };
            let drawing_buffer = self.drawing.get_buffer();
            *self.last_frame_data.borrow_mut() = itm.clone();

            if self.size        != (itm.kernel_params.width as usize,        itm.kernel_params.height as usize) ||
               self.output_size != (itm.kernel_params.output_width as usize, itm.kernel_params.output_height as usize) {
                log::warn!("Size mismatch ({:?} != ({}, {}, {}) || ({:?} != ({}, {}, {})", self.size, itm.kernel_params.width, itm.kernel_params.height, itm.kernel_params.stride, self.output_size, itm.kernel_params.output_width, itm.kernel_params.output_height, itm.kernel_params.output_stride);
                return None;
            }

            // OpenCL path
            #[cfg(feature = "use-opencl")]
            if let Some(ref cl) = self.cl {
                if let Err(err) = cl.undistort_image(buffers, &itm, drawing_buffer) {
                    log::error!("OpenCL error undistort: {:?}", err);
                } else {
                    ret.backend = "OpenCL";
                    return Some(ret);
                }
            }

            // wgpu path
            if let Some(ref wgpu) = self.wgpu {
                wgpu.undistort_image(buffers, &itm, drawing_buffer);
                ret.backend = "wgpu";
                return Some(ret);
            }

            if let BufferSource::Cpu { input, output } = &mut buffers.buffers {
                // CPU path
                match self.interpolation {
                    Interpolation::Bilinear => { Self::undistort_image_cpu::<2>(input, output, &itm.kernel_params, &self.compute_params.distortion_model, self.compute_params.digital_lens.as_ref(), &itm.matrices, drawing_buffer); },
                    Interpolation::Bicubic  => { Self::undistort_image_cpu::<4>(input, output, &itm.kernel_params, &self.compute_params.distortion_model, self.compute_params.digital_lens.as_ref(), &itm.matrices, drawing_buffer); },
                    Interpolation::Lanczos4 => { Self::undistort_image_cpu::<8>(input, output, &itm.kernel_params, &self.compute_params.distortion_model, self.compute_params.digital_lens.as_ref(), &itm.matrices, drawing_buffer); },
                }
                ret.backend = "CPU";
                return Some(ret);
            }
        } else {
            log::warn!("No stab data at {timestamp_us}");
        }
        None
    }
}

unsafe impl<T: PixelType> Send for Stabilization<T> { }
unsafe impl<T: PixelType> Sync for Stabilization<T> { }
