// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::BTreeMap;
use std::cell::RefCell;

use crate::GyroflowCoreError;

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

#[derive(Default, Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub enum Interpolation {
    #[default]
    Bilinear = 2,
    Bicubic  = 4,
    Lanczos4 = 8,
    RobidouxSharp = 10,
    Robidoux = 11,
    Mitchell = 12,
    CatmullRom = 13
}
impl From<&str> for Interpolation {
    fn from(s: &str) -> Self {
        match s {
            "Bilinear"           => Interpolation::Bilinear,
            "Bicubic"            => Interpolation::Bicubic,
            "Lanczos4"           => Interpolation::Lanczos4,
            "EWA: RobidouxSharp" => Interpolation::RobidouxSharp,
            "EWA: Robidoux"      => Interpolation::Robidoux,
            "EWA: Mitchell"      => Interpolation::Mitchell,
            "EWA: Catmull-Rom"   => Interpolation::CatmullRom,
            _ => Interpolation::Lanczos4
        }
    }
}

struct ThreadLocalWgpuCache(RefCell<lru::LruCache<u32, wgpu::WgpuWrapper>>);
impl Drop for ThreadLocalWgpuCache {
    fn drop(&mut self) {
        // Workaround for a Vulkan hang on device destroy (https://github.com/gfx-rs/wgpu/issues/4973)
        let inner = self.0.replace(lru::LruCache::new(std::num::NonZeroUsize::new(1).unwrap()));
        std::thread::spawn(move || drop(inner));
    }
}

lazy_static::lazy_static! {
    pub static ref GPU_LIST: parking_lot::RwLock<Vec<String>> = parking_lot::RwLock::new(Vec::new());
}
thread_local! {
    static CACHED_WGPU: ThreadLocalWgpuCache = ThreadLocalWgpuCache(RefCell::new(lru::LruCache::new(std::num::NonZeroUsize::new(15).unwrap())));
    #[cfg(feature = "use-opencl")]
    static CACHED_OPENCL: RefCell<lru::LruCache<u32, opencl::OclWrapper>> = RefCell::new(lru::LruCache::new(std::num::NonZeroUsize::new(15).unwrap()));
}

bitflags::bitflags! {
    #[derive(Default, Clone)]
    pub struct KernelParamsFlags: i32 {
        const FIX_COLOR_RANGE      = 1 << 0; // 1
        const HAS_DIGITAL_LENS     = 1 << 1; // 2
        const FILL_WITH_BACKGROUND = 1 << 2; // 4
        const DRAWING_ENABLED      = 1 << 3; // 8
        const HORIZONTAL_RS        = 1 << 4; // 16, right-to-left or left-to-right rolling shutter
        const HAS_SOURCE_RECT      = 1 << 5; // 32
        const HAS_OUTPUT_RECT      = 1 << 6; // 64
        const FRAMEBUFFER_INVERTED = 1 << 7; // 128
        const HAS_IBIS_DATA        = 1 << 8; // 256
        const HAS_MESH_DATA        = 1 << 9; // 512
        const HAS_FPD_DATA         = 1 << 10; // 1024
        const ANY_UNDERWATER       = 1 << 11; // 2048
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
    pub background:        [f32; 4], // 16
    pub f:                 [f32; 2], // 8  - focal length in pixels
    pub c:                 [f32; 2], // 16 - lens center
    pub k:                 [f32; 12], // 16,16,16 - distortion coefficients
    pub fov:               f32, // 4
    pub r_limit:           f32, // 8
    pub lens_correction_amount:   f32, // 12
    pub input_vertical_stretch:   f32, // 16
    pub input_horizontal_stretch: f32, // 4
    pub background_margin:        f32, // 8
    pub background_margin_feather:f32, // 12
    pub canvas_scale:             f32, // 16
    pub input_rotation:           f32, // 4
    pub output_rotation:          f32, // 8
    pub translation2d:            [f32; 2], // 16
    pub translation3d:            [f32; 4], // 16
    pub source_rect:              [i32; 4], // 16 - x, y, w, h
    pub output_rect:              [i32; 4], // 16 - x, y, w, h
    pub digital_lens_params:      [f32; 4], // 16
    pub safe_area_rect:           [f32; 4], // 16
    pub max_pixel_value:          f32, // 4
    pub distortion_model:         stabilize_spirv::DistortionModel, // 8
    pub digital_lens:             stabilize_spirv::DistortionModel, // 12
    pub pixel_value_limit:        f32, // 16
    pub light_refraction_coefficient: f32, // 4
    pub plane_index:              i32, // 8
    pub reserved1:                f32, // 12
    pub reserved2:                f32, // 16
    pub ewa_coeffs_p:             [f32; 4], // 16
    pub ewa_coeffs_q:             [f32; 4], // 16
}
unsafe impl bytemuck::Zeroable for KernelParams {}
unsafe impl bytemuck::Pod for KernelParams {}

#[derive(Default, Debug)]
pub enum BackendType {
    #[default]
    None,
    OpenCL(u32),
    Wgpu(u32),
    Cpu(u32)
}
impl BackendType {
    pub fn get_hash(&self) -> u32 {
        match self { BackendType::Cpu(x) => *x, BackendType::OpenCL(x) => *x, BackendType::Wgpu(x) => *x, _ => 0 }
    }
    pub fn is_none(&self) -> bool { matches!(self, Self::None) }
    pub fn is_wgpu(&self) -> bool { matches!(self, Self::Wgpu(_)) }
}

#[derive(Default)]
pub struct Stabilization {
    pub stab_data: BTreeMap<i64, FrameTransform>,

    pub size:        (usize, usize), // width, height
    pub output_size: (usize, usize), // width, height

    pub interpolation: Interpolation,
    pub kernel_flags: KernelParamsFlags,

    #[cfg(feature = "use-opencl")]
    cl: Option<opencl::OclWrapper>,

    pub wgpu: Option<wgpu::WgpuWrapper>,

    pub initialized_backend: BackendType,

    compute_params: ComputeParams,

    pub drawing: DrawCanvas,
    pub pending_device_change: Option<isize>,

    pub share_wgpu_instances: bool,
    pub cache_frame_transform: bool,
    next_backend: Option<&'static str>
}

#[derive(Debug)]
pub struct ProcessedInfo {
    pub fov: f64,
    pub minimal_fov: f64,
    pub focal_length: Option<f64>,
    pub backend: &'static str,
}

impl Stabilization {
    pub fn set_compute_params(&mut self, params: ComputeParams) {
        self.stab_data.clear();
        self.compute_params = params;
    }

    fn get_rect(desc: &BufferDescription) -> [i32; 4] {
        let mut ret = [0i32; 4];
        if let Some(r) = desc.rect {
            ret[0] = r.0 as i32;
            ret[1] = r.1 as i32;
            ret[2] = r.2 as i32;
            ret[3] = r.3 as i32;
        } else {
            // Stretch to the buffer by default
            ret[0] = 0;
            ret[1] = 0;
            ret[2] = desc.size.0 as i32;
            ret[3] = desc.size.1 as i32;
        }
        ret
    }

    pub fn get_kernel_flags(&self, frame: usize, buffers: &Buffers) -> KernelParamsFlags {
        let mut kernel_flags = self.kernel_flags.clone();
        kernel_flags.set(KernelParamsFlags::HAS_DIGITAL_LENS, self.compute_params.digital_lens.is_some());
        kernel_flags.set(KernelParamsFlags::HORIZONTAL_RS, self.compute_params.frame_readout_direction.is_horizontal());
        kernel_flags.set(KernelParamsFlags::HAS_SOURCE_RECT, buffers.input.rect.is_some() || self.size.0 != buffers.input.size.0 || self.size.1 != buffers.input.size.1);
        kernel_flags.set(KernelParamsFlags::HAS_OUTPUT_RECT, buffers.output.rect.is_some() || self.output_size.0 != buffers.output.size.0 || self.output_size.1 != buffers.output.size.1);
        kernel_flags.set(KernelParamsFlags::FRAMEBUFFER_INVERTED, self.compute_params.framebuffer_inverted);
        kernel_flags.set(KernelParamsFlags::ANY_UNDERWATER, (self.compute_params.light_refraction_coefficient != 1.0 && self.compute_params.light_refraction_coefficient > 0.0) || self.compute_params.keyframes.is_keyframed(&crate::KeyframeType::LightRefractionCoeff));

        {
            let gyro = self.compute_params.gyro.read();
            let file_metadata = gyro.file_metadata.read();
            if let Some(mc) = file_metadata.mesh_correction.get(frame) {
                if mc.1[0] > 10.0 {
                    kernel_flags.set(KernelParamsFlags::HAS_MESH_DATA, true);
                }
                if mc.1[0] > 0.0 && mc.1[mc.1[0] as usize] > 0.0 {
                    kernel_flags.set(KernelParamsFlags::HAS_FPD_DATA, true);
                }
            }
            if file_metadata.camera_stab_data.len() > frame {
                kernel_flags.set(KernelParamsFlags::HAS_IBIS_DATA, true);
            }
        }
        kernel_flags
    }

    pub fn get_frame_transform_at<T: PixelType>(&self, timestamp_us: i64, frame: Option<usize>, buffers: &Buffers) -> FrameTransform {
        let timestamp_ms = (timestamp_us as f64) / 1000.0;
        let frame = frame.unwrap_or_else(|| crate::frame_at_timestamp(timestamp_ms, self.compute_params.scaled_fps) as usize);

        let mut transform = FrameTransform::at_timestamp(&self.compute_params, timestamp_ms, frame);
        transform.kernel_params.pixel_value_limit = T::default_max_value().unwrap_or(f32::MAX);
        transform.kernel_params.max_pixel_value = T::default_max_value().unwrap_or(1.0);
        // If the pixel format gets converted to normalized 0-1 float in shader
        if self.initialized_backend.is_wgpu() && T::wgpu_format().map(|x| x.2).unwrap_or_default() {
            transform.kernel_params.pixel_value_limit = 1.0;
            transform.kernel_params.max_pixel_value = 1.0;
        }
        transform.kernel_params.interpolation = self.interpolation as i32;
        transform.kernel_params.width  = self.size.0 as i32;
        transform.kernel_params.height = self.size.1 as i32;
        transform.kernel_params.output_width  = self.output_size.0 as i32;
        transform.kernel_params.output_height = self.output_size.1 as i32;
        transform.kernel_params.background = [self.compute_params.background[0], self.compute_params.background[1], self.compute_params.background[2], self.compute_params.background[3]];
        transform.kernel_params.bytes_per_pixel = (T::COUNT * T::SCALAR_BYTES) as i32;
        transform.kernel_params.pix_element_count = T::COUNT as i32;
        transform.kernel_params.canvas_scale = self.drawing.scale as f32;
        transform.kernel_params.flags = self.get_kernel_flags(frame, buffers).bits();

        transform.kernel_params.stride        = buffers.input.size.2 as i32;
        transform.kernel_params.output_stride = buffers.output.size.2 as i32;

        if transform.kernel_params.interpolation > 8 {
            let (b, c) = match self.interpolation {
                Interpolation::RobidouxSharp => (0.2620145, 0.3689927),
                Interpolation::Robidoux      => (0.3782157, 0.3108921),
                Interpolation::Mitchell      => (0.3333333, 0.3333333),
                Interpolation::CatmullRom    => (0.0000000, 0.5000000),
                _ => (0.0, 0.0)
            };
            transform.kernel_params.ewa_coeffs_p[0] = (6.0 - 2.0 * b) / 6.0;
            transform.kernel_params.ewa_coeffs_p[1] = 0.0;
            transform.kernel_params.ewa_coeffs_p[2] = (-18.0 + 12.0 * b + 6.0 * c) / 6.0;
            transform.kernel_params.ewa_coeffs_p[3] = (12.0 - 9.0 * b - 6.0 * c) / 6.0;
            transform.kernel_params.ewa_coeffs_q[0] = (8.0 * b + 24.0 * c) / 6.0;
            transform.kernel_params.ewa_coeffs_q[1] = (-12.0 * b - 48.0 * c) / 6.0;
            transform.kernel_params.ewa_coeffs_q[2] = (6.0 * b + 30.0 * c) / 6.0;
            transform.kernel_params.ewa_coeffs_q[3] = (-1.0 * b - 6.0 * c) / 6.0;
        }

        let sa_fov =
            if self.compute_params.show_safe_area || self.compute_params.fov_overview  {
                let fov = self.compute_params.keyframes.value_at_video_timestamp(&crate::keyframes::KeyframeType::Fov, timestamp_ms).unwrap_or(self.compute_params.fov_scale) as f32;
                if self.compute_params.fov_overview {
                    (if self.compute_params.adaptive_zoom_window == 0.0 { 1.0 } else { 1.0 / fov }) + 1.0
                } else {
                    fov / (if self.compute_params.adaptive_zoom_window == 0.0 { transform.minimal_fov as f32 } else { 1.0 })
                }
            } else {
                1.0
            };
        let pos_x = (transform.kernel_params.output_width as f32 - (transform.kernel_params.output_width as f32 / sa_fov)) / 2.0;
        let pos_y = (transform.kernel_params.output_height as f32 - (transform.kernel_params.output_height as f32 / sa_fov)) / 2.0;
        transform.kernel_params.safe_area_rect[0] = pos_x;
        transform.kernel_params.safe_area_rect[1] = pos_y;
        transform.kernel_params.safe_area_rect[2] = transform.kernel_params.output_width as f32 - pos_x;
        transform.kernel_params.safe_area_rect[3] = transform.kernel_params.output_height as f32 - pos_y;

        if let Some(r) = buffers.input.rotation {
            transform.kernel_params.input_rotation = r;
        }
        if let Some(r) = buffers.output.rotation {
            transform.kernel_params.output_rotation = r;
        }

        transform.kernel_params.source_rect = Self::get_rect(&buffers.input);
        transform.kernel_params.output_rect = Self::get_rect(&buffers.output);

        transform
    }

    pub fn ensure_stab_data_at_timestamp<T: PixelType>(&mut self, timestamp_us: i64, frame: Option<usize>, buffers: &mut Buffers, is_pixel_normalized: bool) {
        let mut insert = true;
        if let Some(itm) = self.stab_data.get(&timestamp_us) {
            insert = false;
            if itm.kernel_params.stride        != buffers.input.size.2 as i32 ||
               itm.kernel_params.output_stride != buffers.output.size.2 as i32 {
                log::warn!("Stride mismatch ({} != {} || {} != {})", itm.kernel_params.stride, buffers.input.size.2, itm.kernel_params.output_stride, buffers.output.size.2);
                insert = true;
            }
            if itm.kernel_params.input_rotation != buffers.input.rotation.unwrap_or(0.0) ||
               itm.kernel_params.output_rotation != buffers.output.rotation.unwrap_or(0.0) ||
               itm.kernel_params.source_rect != Self::get_rect(&buffers.input) ||
               itm.kernel_params.output_rect != Self::get_rect(&buffers.output) {
                log::warn!("Updating stab params at {timestamp_us}");
                insert = true;
            }
        }
        if insert {
            let mut transform = self.get_frame_transform_at::<T>(timestamp_us, frame, buffers);
            if is_pixel_normalized {
                transform.kernel_params.max_pixel_value = 1.0;
                transform.kernel_params.pixel_value_limit = 1.0;
            }
            self.stab_data.insert(timestamp_us, transform);
        }
    }

    pub fn get_current_key(&self, buffers: &Buffers) -> String {
        let mut flags = self.get_kernel_flags(0, buffers);
        flags.set(KernelParamsFlags::FILL_WITH_BACKGROUND, false);
        format!(
            "{}|{}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}",
            buffers.get_checksum(),
            self.compute_params.distortion_model.id(),
            self.compute_params.digital_lens.as_ref().map(|x| x.id()).unwrap_or_default(),
            self.interpolation as u32,
            flags.bits(),
            self.size,
            self.output_size,
            self.interpolation,
            std::thread::current().id(),
        )
    }
    pub fn get_current_checksum(&self, buffers: &Buffers) -> u32 {
        crc32fast::hash(self.get_current_key(buffers).as_bytes())
    }

    pub fn init_size(&mut self, size: (usize, usize), output_size: (usize, usize)) {
        self.initialized_backend = BackendType::None;
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

    pub fn clear_stab_data(&mut self) {
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

    pub fn update_device(&mut self, i: isize, buffers: &Buffers) -> bool {
        self.stab_data.clear();
        self.next_backend = None;
        self.initialized_backend = BackendType::None;
        #[cfg(feature = "use-opencl")]
        { self.cl = None; }
        self.wgpu = None;

        let hash = self.get_current_checksum(buffers);
        if i < 0 { // CPU
            CACHED_WGPU.with(|x| x.0.borrow_mut().clear());
            #[cfg(feature = "use-opencl")] {
                CACHED_OPENCL.with(|x| x.borrow_mut().clear());
            }
            self.initialized_backend = BackendType::Cpu(hash);
            return true;
        }
        let gpu_list = GPU_LIST.read();
        if let Some(name) = gpu_list.get(i as usize) {
            if name.starts_with("[OpenCL]") {
                self.initialized_backend = BackendType::None;
                #[cfg(feature = "use-opencl")]
                match opencl::OclWrapper::set_device(i as usize, buffers) {
                    Ok(_) => {
                        self.next_backend = Some("opencl");
                        CACHED_WGPU.with(|x| x.0.borrow_mut().clear());
                        return true;
                    },
                    Err(e) => {
                        log::error!("Failed to set OpenCL device {}: {:?}", name, e);
                    }
                }
            } else if name.starts_with("[wgpu]") {
                self.initialized_backend = BackendType::None;
                let first_ind = gpu_list.iter().enumerate().find(|(_, m)| m.starts_with("[wgpu]")).map(|(idx, _)| idx).unwrap_or(0);
                let wgpu_ind = i - first_ind as isize;
                if wgpu_ind >= 0 {
                    match wgpu::WgpuWrapper::set_device(wgpu_ind as usize) {
                        Some(_) => {
                            self.next_backend = Some("wgpu");
                            #[cfg(feature = "use-opencl")] {
                                CACHED_OPENCL.with(|x| x.borrow_mut().clear());
                            }
                            return true;
                        },
                        None => {
                            log::error!("Failed to set wgpu device {}", name);
                        }
                    }
                }
            }
        }
        false
    }

    pub fn init_backends<T: PixelType>(&mut self, timestamp_us: i64, frame: Option<usize>, buffers: &Buffers) {
        let hash = self.get_current_checksum(buffers);
        let current_hash = self.initialized_backend.get_hash();

        if current_hash != hash {
            self.initialized_backend = BackendType::None;
            let canvas_len = self.drawing.get_buffer_len();
            #[allow(unused_mut)]
            let mut next_backend = self.next_backend.take().unwrap_or_default();

            #[cfg(feature = "use-opencl")]
            if std::env::var("NO_OPENCL").unwrap_or_default().is_empty() && next_backend != "wgpu" && opencl::is_buffer_supported(buffers) {
                if self.share_wgpu_instances && CACHED_OPENCL.with(|x| x.borrow().contains(&hash)) {
                    self.cl = None;
                    self.initialized_backend = BackendType::OpenCL(hash);
                } else {
                    self.cl = None;
                    let transform = self.get_frame_transform_at::<T>(timestamp_us, frame, buffers);
                    let params = transform.kernel_params;
                    let distortion_model = self.compute_params.distortion_model.clone();
                    let digital_lens = self.compute_params.digital_lens.clone();
                    let cl = std::panic::catch_unwind(|| {
                        opencl::OclWrapper::new(&params, T::ocl_names(), distortion_model, digital_lens, buffers, canvas_len)
                    });
                    match cl {
                        Ok(Ok(cl)) => {
                            if self.share_wgpu_instances {
                                CACHED_OPENCL.with(|x| x.borrow_mut().put(hash, cl));
                            } else {
                                self.cl = Some(cl);
                            }
                            self.initialized_backend = BackendType::OpenCL(hash);
                            log::info!("Initialized OpenCL for {:?} -> {:?}, key: {}", buffers.input.size, buffers.output.size, self.get_current_key(buffers));
                        },
                        Ok(Err(e)) => { next_backend = ""; log::error!("OpenCL error init_backends: {:?}", e); if self.share_wgpu_instances { CACHED_OPENCL.with(|x| x.borrow_mut().clear()) } },
                        Err(e) => {
                            next_backend = "";
                            if let Some(s) = e.downcast_ref::<&str>() {
                                log::error!("Failed to initialize OpenCL {}", s);
                            } else if let Some(s) = e.downcast_ref::<String>() {
                                log::error!("Failed to initialize OpenCL {}", s);
                            } else {
                                log::error!("Failed to initialize OpenCL {:?}", e);
                            }
                            if self.share_wgpu_instances { CACHED_OPENCL.with(|x| x.borrow_mut().clear()); }
                        }
                    }
                }
            }
            if self.initialized_backend.is_none() && T::wgpu_format().is_some() && next_backend != "opencl" && std::env::var("NO_WGPU").unwrap_or_default().is_empty() && wgpu::is_buffer_supported(buffers) {
                if self.share_wgpu_instances && CACHED_WGPU.with(|x| x.0.borrow().contains(&hash)) {
                    self.wgpu = None;
                    self.initialized_backend = BackendType::Wgpu(hash);
                } else {
                    self.wgpu = None;
                    let transform = self.get_frame_transform_at::<T>(timestamp_us, frame, buffers);
                    let params = transform.kernel_params;
                    let distortion_model = self.compute_params.distortion_model.clone();
                    let digital_lens = self.compute_params.digital_lens.clone();
                    let wgpu = std::panic::catch_unwind(|| {
                        wgpu::WgpuWrapper::new(&params, T::wgpu_format().unwrap(), distortion_model, digital_lens, buffers, canvas_len)
                    });
                    match wgpu {
                        Ok(Ok(wgpu)) => {
                            if self.share_wgpu_instances {
                                CACHED_WGPU.with(|x| x.0.borrow_mut().put(hash, wgpu));
                            } else {
                                self.wgpu = Some(wgpu);
                            }
                            self.initialized_backend = BackendType::Wgpu(hash);
                            log::info!("Initialized wgpu for {:?} -> {:?} | key: {}", buffers.input.size, buffers.output.size, self.get_current_key(buffers));
                        },
                        Ok(Err(e)) => { log::error!("Failed to initialize wgpu {:?}", e); if self.share_wgpu_instances { CACHED_WGPU.with(|x| x.0.borrow_mut().clear()) } },
                        Err(e) => {
                            if let Some(s) = e.downcast_ref::<&str>() {
                                log::error!("Failed to initialize wgpu {}", s);
                            } else if let Some(s) = e.downcast_ref::<String>() {
                                log::error!("Failed to initialize wgpu {}", s);
                            } else {
                                log::error!("Failed to initialize wgpu {:?}", e);
                            }
                            if self.share_wgpu_instances { CACHED_WGPU.with(|x| x.0.borrow_mut().clear()); }
                        },
                    }
                }
            }
        }
    }

    pub fn ensure_ready_for_processing<T: PixelType>(&mut self, timestamp_us: i64, frame: Option<usize>, buffers: &mut Buffers) {
        let pending_dev = self.pending_device_change.clone();
        if let Some(dev) = self.pending_device_change.take() {
            log::debug!("Setting device {dev}");
            self.update_device(dev, buffers);
        }

        self.init_backends::<T>(timestamp_us, frame, buffers);
        self.ensure_stab_data_at_timestamp::<T>(timestamp_us, frame, buffers, false);

        if self.share_wgpu_instances {
            if wgpu::is_buffer_supported(buffers) && CACHED_WGPU.with(|x| !x.0.borrow().is_empty()) {
                let hash = self.get_current_checksum(buffers);
                let has_cached = CACHED_WGPU.with(|x| x.0.borrow().contains(&hash));
                if !has_cached {
                    log::warn!("Cached wgpu not found, reinitializing. Key: {}", self.get_current_key(buffers));
                    self.initialized_backend = BackendType::None;
                    if let Some(dev) = pending_dev {
                        log::debug!("Setting device {dev}");
                        self.update_device(dev, buffers);
                    }
                    self.init_backends::<T>(timestamp_us, frame, buffers);
                } else {
                    self.initialized_backend = BackendType::Wgpu(hash);
                }
            } else {
                #[cfg(feature = "use-opencl")]
                if opencl::is_buffer_supported(buffers) && CACHED_OPENCL.with(|x| !x.borrow().is_empty()) {
                    let hash = self.get_current_checksum(buffers);
                    let has_cached = CACHED_OPENCL.with(|x| x.borrow().contains(&hash));
                    if !has_cached {
                        log::warn!("Cached OpenCL not found, reinitializing. Key: {}", self.get_current_key(buffers));
                        self.initialized_backend = BackendType::None;
                        if let Some(dev) = pending_dev {
                            log::debug!("Setting device {dev}");
                            self.update_device(dev, buffers);
                        }
                        self.init_backends::<T>(timestamp_us, frame, buffers);
                    } else {
                        self.initialized_backend = BackendType::OpenCL(hash);
                    }
                }
            }
        }
    }
    pub fn process_pixels<T: PixelType>(&self, timestamp_us: i64, frame: Option<usize>, buffers: &mut Buffers, frame_transform: Option<&FrameTransform>) -> Result<ProcessedInfo, GyroflowCoreError> {
        if buffers.input.size.1 < 4 || buffers.output.size.1 < 4 { return Err(GyroflowCoreError::SizeTooSmall); }

        let mut _tmp_transform = None;
        if frame_transform.is_none() && !self.cache_frame_transform {
            _tmp_transform = Some(self.get_frame_transform_at::<T>(timestamp_us, frame, buffers));
        }
        let itm = frame_transform.map(|x| Some(x)).unwrap_or_else(||
            if !self.cache_frame_transform {
                _tmp_transform.as_ref()
            } else {
                self.stab_data.get(&timestamp_us)
            }
        );

        if let Some(itm) = itm {
            let mut ret = ProcessedInfo {
                fov: itm.fov,
                minimal_fov: itm.minimal_fov,
                focal_length: itm.focal_length,
                backend: ""
            };
            let drawing_buffer = self.drawing.get_buffer();

            if self.size        != (itm.kernel_params.width        as usize, itm.kernel_params.height        as usize) { return Err(GyroflowCoreError::SizeMismatch(self.size, (itm.kernel_params.width        as usize, itm.kernel_params.height        as usize))); }
            if self.output_size != (itm.kernel_params.output_width as usize, itm.kernel_params.output_height as usize) { return Err(GyroflowCoreError::SizeMismatch(self.size, (itm.kernel_params.output_width as usize, itm.kernel_params.output_height as usize))); }

            if buffers.input.size.0  as i32 > itm.kernel_params.stride        { return Err(GyroflowCoreError::InvalidStride(itm.kernel_params.stride, buffers.input.size.0 as i32)); }
            if buffers.output.size.0 as i32 > itm.kernel_params.output_stride { return Err(GyroflowCoreError::InvalidStride(itm.kernel_params.output_stride, buffers.output.size.0 as i32)); }

            // OpenCL path
            #[cfg(feature = "use-opencl")]
            if !matches!(self.initialized_backend, BackendType::Cpu(_)) && opencl::is_buffer_supported(buffers) {
                if self.share_wgpu_instances {
                    let hash = self.get_current_checksum(buffers);
                    let has_cache = CACHED_OPENCL.with(|lru| lru.borrow().contains(&hash));
                    if has_cache {
                        return CACHED_OPENCL.with(|x| {
                            let mut cached = x.borrow_mut();
                            if let Some(cl) = cached.get(&hash) {
                                if let Err(err) = cl.undistort_image(buffers, &itm, drawing_buffer) {
                                    log::error!("OpenCL error undistort: {:?}", err);
                                }
                                ret.backend = "OpenCL";
                                Ok(ret)
                            } else {
                                Err(GyroflowCoreError::NoCachedWgpuInstance(self.get_current_key(buffers)))
                            }
                        });
                    }
                } else {
                    if let Some(ref cl) = self.cl {
                        if let Err(err) = cl.undistort_image(buffers, &itm, drawing_buffer) {
                            log::error!("OpenCL error undistort: {:?}", err);
                        } else {
                            ret.backend = "OpenCL";
                            return Ok(ret);
                        }
                    }
                }
            }

            // wgpu path
            if !matches!(self.initialized_backend, BackendType::Cpu(_)) && wgpu::is_buffer_supported(buffers) {
                if self.share_wgpu_instances {
                    let hash = self.get_current_checksum(buffers);
                    let has_any_cache = CACHED_WGPU.with(|x| !x.0.borrow().is_empty());
                    if has_any_cache {
                        return CACHED_WGPU.with(|x| {
                            let mut cached = x.0.borrow_mut();
                            if let Some(wgpu) = cached.get(&hash) {
                                wgpu.undistort_image(buffers, &itm, drawing_buffer);
                                ret.backend = "wgpu";
                                Ok(ret)
                            } else {
                                Err(GyroflowCoreError::NoCachedWgpuInstance(self.get_current_key(buffers)))
                            }
                        });
                    } else {
                        log::error!("No cached wgpu found for key: {}", self.get_current_key(buffers));
                    }
                } else {
                    if let Some(ref wgpu) = self.wgpu {
                        wgpu.undistort_image(buffers, &itm, drawing_buffer);
                        ret.backend = "wgpu";
                        return Ok(ret);
                    } else {
                        log::error!("No wgpu instance!");
                    }
                }
            }

            // CPU path
            let ok = match self.interpolation {
                Interpolation::Bilinear => { Self::undistort_image_cpu::<2, T>(buffers, &itm.kernel_params, &self.compute_params.distortion_model, self.compute_params.digital_lens.as_ref(), &itm.matrices, drawing_buffer, &itm.mesh_data) },
                Interpolation::Bicubic  => { Self::undistort_image_cpu::<4, T>(buffers, &itm.kernel_params, &self.compute_params.distortion_model, self.compute_params.digital_lens.as_ref(), &itm.matrices, drawing_buffer, &itm.mesh_data) },
                Interpolation::Lanczos4 => { Self::undistort_image_cpu::<8, T>(buffers, &itm.kernel_params, &self.compute_params.distortion_model, self.compute_params.digital_lens.as_ref(), &itm.matrices, drawing_buffer, &itm.mesh_data) },
                Interpolation::RobidouxSharp => { Self::undistort_image_cpu::<10, T>(buffers, &itm.kernel_params, &self.compute_params.distortion_model, self.compute_params.digital_lens.as_ref(), &itm.matrices, drawing_buffer, &itm.mesh_data) },
                Interpolation::Robidoux      => { Self::undistort_image_cpu::<11, T>(buffers, &itm.kernel_params, &self.compute_params.distortion_model, self.compute_params.digital_lens.as_ref(), &itm.matrices, drawing_buffer, &itm.mesh_data) },
                Interpolation::Mitchell      => { Self::undistort_image_cpu::<12, T>(buffers, &itm.kernel_params, &self.compute_params.distortion_model, self.compute_params.digital_lens.as_ref(), &itm.matrices, drawing_buffer, &itm.mesh_data) },
                Interpolation::CatmullRom    => { Self::undistort_image_cpu::<13, T>(buffers, &itm.kernel_params, &self.compute_params.distortion_model, self.compute_params.digital_lens.as_ref(), &itm.matrices, drawing_buffer, &itm.mesh_data) },
            };
            if ok {
                ret.backend = "CPU";
                return Ok(ret);
            }
        } else {
            log::warn!("No stab data at {timestamp_us}");
            return Err(GyroflowCoreError::NoStabilizationData(timestamp_us));
        }
        Err(GyroflowCoreError::Unknown)
    }
}

unsafe impl Send for Stabilization { }
unsafe impl Sync for Stabilization { }
