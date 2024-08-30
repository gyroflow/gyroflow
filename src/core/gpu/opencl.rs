// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use ocl::*;
use ocl::core::{ ImageDescriptor, MemObjectType, GlTextureTarget };
use parking_lot::RwLock;
use std::ops::DerefMut;
use super::*;
use crate::stabilization::distortion_models::DistortionModel;
use crate::stabilization::KernelParams;

pub struct OclWrapper {
    kernel: Kernel,
    src: Buffer<u8>,
    dst: Buffer<u8>,

    queue: Queue,

    image_src: Option<(ocl::Image::<u8>, u64)>,
    image_dst: Option<(ocl::Image::<u8>, u64)>,

    buf_params: Buffer<u8>,
    buf_drawing: Buffer<u8>,
    buf_mesh_data: Buffer<f32>,
    buf_matrices: Buffer<f32>,
}

pub struct CtxWrapper {
    pub device: Device,
    pub context: Context,
    pub platform: Platform,

    pub surface_checksum: u32
}

lazy_static::lazy_static! {
    static ref CONTEXT: RwLock<Option<CtxWrapper>> = RwLock::new(None);
}

const EXCLUSIONS: &[&'static str] = &["Microsoft Basic Render Driver"];

impl OclWrapper {
    fn get_properties(buffers: Option<&Buffers>) -> ocl::builders::ContextProperties {
        let mut props = ocl::builders::ContextProperties::new();
        if let Some(buffers) = buffers {
            match &buffers.input.data {
                BufferSource::OpenGL { context: _context, .. } => {
                    props = ocl_interop::get_properties_list();
                }
                #[cfg(target_os = "windows")]
                BufferSource::DirectX11 { device, .. } => {
                    props.set_property_value(ocl::enums::ContextPropertyValue::D3d11DeviceKhr(*device));
                },
                _ => { }
            }
        }
        props
    }

    pub fn list_devices() -> Vec<String> {
        let devices = std::panic::catch_unwind(|| -> Vec<String> {
            let mut ret = Vec::new();
            for p in Platform::list() {
                if let Ok(devs) = Device::list(p, Some(ocl::flags::DeviceType::new().gpu().accelerator())) {
                    ret.extend(devs.into_iter().filter_map(|x| Some(format!("{} {}: {}", p.name().ok()?, x.name().ok()?, x.version().ok()?))));
                }
            }
            ret.drain(..).filter(|x| !EXCLUSIONS.iter().any(|e| x.contains(e))).collect()
        });
        match devices {
            Ok(devices) => { return devices; },
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
        Vec::new()
    }
    pub fn get_info() -> Option<String> {
        let lock = CONTEXT.read();
        if let Some(ref ctx) = *lock {
            ctx.device.name().ok()
        } else {
            None
        }
    }

    pub fn set_device(index: usize, buffers: &Buffers) -> ocl::Result<()> {
        let mut i = 0;
        for p in Platform::list() {
            if let Ok(devs) = Device::list(p, Some(ocl::flags::DeviceType::new().gpu().accelerator())) {
                for d in devs {
                    if EXCLUSIONS.iter().any(|x| d.name().unwrap_or_default().contains(x)) { continue; }
                    if i == index {
                        ::log::info!("OpenCL Platform: {}, Device: {} {}", p.name()?, d.vendor()?, d.name()?);

                        let context = Context::builder()
                            .properties(Self::get_properties(Some(buffers)))
                            .platform(p)
                            .devices(d)
                            .build()?;

                        *CONTEXT.write() = Some(CtxWrapper { device: d, context, platform: p, surface_checksum: buffers.get_checksum() });
                        return Ok(());
                    }
                    i += 1;
                }
            }
        }
        Err(ocl::BufferCmdError::MapUnavailable.into())
    }

    pub fn initialize_context(buffers: Option<&Buffers>) -> ocl::Result<(String, String)> {
        // List all devices
        Platform::list().iter().for_each(|p| {
            if let Ok(devs) = Device::list(p, Some(ocl::flags::DeviceType::new().gpu().accelerator())) {
                ::log::debug!("OpenCL devices: {:?} {:?} {:?}", p.name(), p.version(), devs.iter().filter_map(|x| x.name().ok()).collect::<Vec<String>>());
            }
        });

        let mut platform = None;
        let mut device = None;
        let preference = [ "nvidia", "quadro", "radeon", "geforce", "firepro", "accelerated parallel processing", "graphics" ];
        'outer: for pref in preference {
            for p in Platform::list() {
                if let Ok(devs) = Device::list(p, Some(ocl::flags::DeviceType::new().gpu().accelerator())) {
                    for d in devs.iter() {
                        let name = format!("{} {}", p.name().unwrap_or_default(),  d.name().unwrap_or_default());
                        if name.to_ascii_lowercase().contains(pref) {
                            platform = Some(p);
                            device = Some(*d);
                            break 'outer;
                        }
                    }
                }
            }
        }
        if device.is_none() {
            // Try first GPU
            'outer2: for p in Platform::list() {
                if let Ok(devs) = Device::list(p, Some(ocl::flags::DeviceType::new().gpu().accelerator())) {
                    for d in devs.iter() {
                        if let Ok(ocl::core::DeviceInfoResult::Type(typ)) = d.info(ocl::core::DeviceInfo::Type) {
                            if typ == ocl::DeviceType::GPU {
                                platform = Some(p);
                                device = Some(*d);
                                break 'outer2;
                            }
                        }
                    }
                }
            }
        }
        if device.is_none() { return Err(ocl::BufferCmdError::MapUnavailable.into()); }
        let platform = platform.unwrap();
        let device = device.unwrap();
        ::log::info!("OpenCL Platform: {}, ext: {:?} Device: {} {}", platform.name()?, platform.extensions()?, device.vendor()?, device.name()?);

        let context = Context::builder()
            .properties(Self::get_properties(buffers))
            .platform(platform)
            .devices(device)
            .build()?;

        let name = format!("{} {}", device.vendor()?, device.name()?);
        let list_name = format!("[OpenCL] {} {}", platform.name()?, device.name()?);

        *CONTEXT.write() = Some(CtxWrapper { device, context, platform, surface_checksum: buffers.map(|x| x.get_checksum()).unwrap_or_default() });

        Ok((name, list_name))
    }

    pub fn new(params: &KernelParams, ocl_names: (&str, &str, &str, &str), distortion_model: DistortionModel, digital_lens: Option<DistortionModel>, buffers: &Buffers, drawing_len: usize) -> ocl::Result<Self> {
        if params.height < 4 || params.output_height < 4 || params.stride < 1 { return Err(ocl::BufferCmdError::AlreadyMapped.into()); }

        let mut kernel = include_str!("opencl_undistort.cl").to_string();
        // let mut kernel = std::fs::read_to_string("D:/programowanie/projekty/Rust/gyroflow/src/core/gpu/opencl_undistort.cl").unwrap();

        let mut lens_model_functions = distortion_model.opencl_functions().to_string();
        let default_digital_lens = "float2 digital_undistort_point(float2 uv, __global KernelParams *p) { return uv; }
                                        float2 digital_distort_point(float2 uv, __global KernelParams *p) { return uv; }";
        lens_model_functions.push_str(digital_lens.as_ref().map(|x| x.opencl_functions()).unwrap_or(default_digital_lens));

        let mut extensions = String::new();
        if ocl_names.1 == "convert_half4" {
            extensions.push_str(r#"
                #pragma OPENCL EXTENSION cl_khr_fp16 : enable
                half4 convert_half4(float4 v) { half4 out = 0.0; vstore_half4_rte(v, 0, (half *)&out); return out; }
                float4 convert_half4_to_float4(half4 v) { return vload_half4(0, (half*)&v); }
            "#);
        }

        kernel = kernel.replace("LENS_MODEL_FUNCTIONS;", &lens_model_functions)
                       .replace("EXTENSIONS;", &extensions)
                       .replace("DATA_CONVERTF", ocl_names.3)
                       .replace("DATA_TYPEF", ocl_names.2)
                       .replace("DATA_CONVERT", ocl_names.1)
                       .replace("DATA_TYPE", ocl_names.0)
                       .replace("PIXEL_BYTES", &format!("{}", params.bytes_per_pixel))
                       .replace("INTERPOLATION", &format!("{}", params.interpolation));

        for i in 0..31 {
            let v = 1 << i;
            if v == 4 { continue; } // Fill with background can be different per frame
            kernel = kernel.replace(&format!("(params->flags & {v})"), if (params.flags & v) == v { "true" } else { "false" });
        }

        {
            let ctx = CONTEXT.read();
            let context_initialized = ctx.is_some();
            if !context_initialized || ctx.as_ref().unwrap().surface_checksum != buffers.get_checksum() {
                drop(ctx);
                Self::initialize_context(Some(buffers))?;
            }
        }
        let mut lock = CONTEXT.write();
        if let Some(ref mut ctx) = *lock {
            let mut ocl_queue = Queue::new(&ctx.context, ctx.device, None)?;

            let in_desc  = ImageDescriptor::new(MemObjectType::Image2d, buffers.input.size.0,  buffers.input.size.1,  1, 1, buffers.input.size.2,  0, None);
            let out_desc = ImageDescriptor::new(MemObjectType::Image2d, buffers.output.size.0, buffers.output.size.1, 1, 1, buffers.output.size.2, 0, None);

            let mut resolve_texture = |buf: &BufferDescription, is_in: bool, ocl_queue: &mut Queue, desc: ImageDescriptor, _other_img: Option<&(ocl::Image<u8>, u64)>| -> ocl::Result<(Buffer<u8>, Option<(ocl::Image<u8>, u64)>)> {
                match &buf.data {
                    BufferSource::Cpu { buffer } => {
                        let flags = if is_in { MemFlags::new().read_only().host_write_only() }
                                           else     { MemFlags::new().write_only().host_read_only().alloc_host_ptr() };
                        Ok((Buffer::builder().queue(ocl_queue.clone()).len(buffer.len()).flags(flags).build()?, None))
                    },
                    BufferSource::OpenCL { queue, .. } => {
                        if !queue.is_null() {
                            let queue_core = unsafe { core::CommandQueue::from_raw_copied_ptr(*queue) };
                            let device_core = queue_core.device()?;
                            let context_core = queue_core.context()?;
                            *ctx.device.deref_mut() = device_core;
                            *ctx.context.deref_mut() = context_core;

                            *ocl_queue = Queue::new(&ctx.context, ctx.device, None)?;
                            *ocl_queue.deref_mut() = queue_core;
                        }
                        let flags = if is_in { MemFlags::new().read_only().host_no_access() }
                                           else     { MemFlags::new().read_write().host_no_access() };
                        Ok((Buffer::builder().queue(ocl_queue.clone()).len(buf.size.1 * buf.size.2).flags(flags).build()?, None))
                    },
                    BufferSource::OpenGL { texture, .. } => {
                        let flags = if is_in { MemFlags::new().read_only() }
                                           else     { MemFlags::new().write_only() };

                        let img = Image::from_gl_texture(ocl_queue.clone(), flags, desc, GlTextureTarget::GlTexture2d, 0, *texture)?;

                        let flags = if is_in { MemFlags::new().read_only().host_no_access() }
                                           else     { MemFlags::new().read_write().host_no_access() };

                        Ok((Buffer::builder().queue(ocl_queue.clone()).len(buf.size.1 * buf.size.2).flags(flags).build()?, Some((img, *texture as u64))))
                    },
                    #[cfg(target_os = "windows")]
                    BufferSource::DirectX11 { texture, .. } => {
                        if is_in {
                            let img = Image::from_d3d11_texture2d(ocl_queue.clone(), MemFlags::new().read_only(), desc, *texture, 0)?;
                            Ok((Buffer::builder().queue(ocl_queue.clone()).len(img.pixel_count() * params.bytes_per_pixel as usize).flags(MemFlags::new().read_only().host_no_access()).build()?, Some((img, *texture as u64))))
                        } else {
                            let img = match &buffers.input.data {
                                BufferSource::DirectX11 { texture: in_texture, .. } if *texture == *in_texture => {
                                    Some((_other_img.unwrap().0.clone(), *texture as u64))
                                },
                                _ => Some((Image::from_d3d11_texture2d(ocl_queue.clone(), MemFlags::new().write_only(), desc, *texture, 0)?, *texture as u64))
                            };
                            Ok((Buffer::builder().queue(ocl_queue.clone()).len(img.as_ref().unwrap().0.pixel_count() * params.bytes_per_pixel as usize).flags(MemFlags::new().read_write().host_no_access()).build()?, img))
                        }
                    },
                    _ => panic!("Unsupported buffer {:?}", buf.data)
                }
            };
            let (source_buffer, image_src) = resolve_texture(&buffers.input, true, &mut ocl_queue, in_desc, None)?;
            let (dest_buffer, image_dst) = resolve_texture(&buffers.output, false, &mut ocl_queue, out_desc, image_src.as_ref())?;

            let program = Program::builder()
                .src(&kernel)
                .devices(ctx.device)
                .build(&ctx.context)?;

            let max_matrix_count = 14 * if (params.flags & 16) == 16 { params.width } else { params.height };
            let flags = MemFlags::new().read_only().host_write_only();

            let buf_params   = Buffer::builder().queue(ocl_queue.clone()).flags(flags).len(std::mem::size_of::<KernelParams>()).build()?;
            let buf_drawing  = Buffer::builder().queue(ocl_queue.clone()).flags(flags).len(drawing_len.max(4)).build()?;
            let buf_matrices = Buffer::builder().queue(ocl_queue.clone()).flags(flags).len(max_matrix_count).build()?;
            let buf_mesh_data = Buffer::builder().queue(ocl_queue.clone()).flags(flags).len(1024).build()?;

            let mut builder = Kernel::builder();
            unsafe {
                builder.program(&program).name("undistort_image").queue(ocl_queue.clone())
                    .global_work_size((buffers.output.size.0, buffers.output.size.1))
                    .disable_arg_type_check()
                    .arg(&source_buffer)
                    .arg(&dest_buffer)
                    .arg(&buf_params)
                    .arg(&buf_matrices)
                    .arg(&buf_drawing)
                    .arg(&buf_mesh_data);
            }

            let kernel = builder.build()?;

            Ok(Self {
                kernel,
                queue: ocl_queue,
                src: source_buffer,
                dst: dest_buffer,
                image_src,
                image_dst,
                buf_params,
                buf_drawing,
                buf_matrices,
                buf_mesh_data,
            })
        } else {
            Err(ocl::BufferCmdError::AlreadyMapped.into())
        }
    }

    pub fn undistort_image(&self, buffers: &mut Buffers, itm: &crate::stabilization::FrameTransform, drawing_buffer: &[u8]) -> ocl::Result<()> {
        let matrices = unsafe { std::slice::from_raw_parts(itm.matrices.as_ptr() as *const f32, itm.matrices.len() * 14 ) };

        let mut _temp1 = None;
        let mut _temp2 = None;

        if self.buf_matrices.len() < matrices.len() { log::error!("Buffer size mismatch matrices! {} vs {}", self.buf_matrices.len(), matrices.len()); return Ok(()); }

        if let Some(ref tex) = self.image_src {
            let len = tex.0.pixel_count() * itm.kernel_params.bytes_per_pixel as usize;
            if len != self.src.len() { log::error!("Buffer size mismatch image_src! {} vs {}", self.src.len(), len);  return Ok(()); }
        }
        if let Some(ref tex) = self.image_dst {
            let len = tex.0.pixel_count() * itm.kernel_params.bytes_per_pixel as usize;
            if len != self.dst.len() { log::error!("Buffer size mismatch image_dst! {} vs {}", self.dst.len(), len);  return Ok(()); }
        }

        if !drawing_buffer.is_empty() {
            if self.buf_drawing.len() != drawing_buffer.len() { log::error!("Buffer size mismatch drawing_buffer! {} vs {}", self.buf_drawing.len(), drawing_buffer.len()); return Ok(()); }
            self.buf_drawing.write(drawing_buffer).enq()?;
        }
        if !itm.mesh_data.is_empty() {
            if self.buf_mesh_data.len() < itm.mesh_data.len() { log::error!("Buffer size mismatch buf_mesh_data! {} vs {}", self.buf_mesh_data.len(), itm.mesh_data.len()); return Ok(()); }
            self.buf_mesh_data.write(&itm.mesh_data).enq()?;
        }
        match buffers.input.data {
            BufferSource::None => { },
            BufferSource::Cpu { ref buffer } => {
                if self.src.len() != buffer.len() { log::error!("Buffer size mismatch input! {} vs {}", self.src.len(), buffer.len());  return Ok(()); }
                self.src.write(buffer as &[u8]).enq()?;
            },
            BufferSource::OpenCL { texture, .. } => {
                unsafe {
                    let siz = std::mem::size_of::<ocl::ffi::cl_mem>() as usize;
                    self.kernel.set_arg_unchecked(0, core::ArgVal::from_raw(siz, &texture as *const _ as *const std::ffi::c_void, true))?;
                }
            },
            BufferSource::OpenGL { texture, .. } => {
                if let Some(ref tex) = self.image_src {
                    let mut img = &tex.0;
                    if tex.1 != texture as u64 {
                        let desc = ImageDescriptor::new(MemObjectType::Image2d, buffers.input.size.0,  buffers.input.size.1,  1, 1, buffers.input.size.2,  0, None);
                        _temp1 = Some(Image::<u8>::from_gl_texture(self.queue.clone(), MemFlags::new().read_only(), desc, GlTextureTarget::GlTexture2d, 0, texture)?);
                        img = _temp1.as_ref().unwrap();
                    }
                    img.cmd().gl_acquire().enq()?;
                    let _ = img.cmd().copy_to_buffer(&self.src, 0).enq();
                    img.cmd().gl_release().enq()?;
                }
            },
            #[cfg(target_os = "windows")]
            BufferSource::DirectX11 { .. } => {
                if let Some(ref tex) = self.image_src {
                    tex.0.cmd().d3d11_acquire().enq()?;
                    let _ = tex.0.cmd().copy_to_buffer(&self.src, 0).enq();
                    tex.0.cmd().d3d11_release().enq()?;
                }
            },
            _ => panic!("Unsupported input buffer {:?}", buffers.input.data)
        }
        match buffers.output.data {
            BufferSource::OpenCL { texture, .. } => {
                unsafe {
                    let siz = std::mem::size_of::<ocl::ffi::cl_mem>() as usize;
                    self.kernel.set_arg_unchecked(1, core::ArgVal::from_raw(siz, &texture as *const _ as *const std::ffi::c_void, true))?;
                }
            },
            _ => { }
        }

        self.buf_params.write(bytemuck::bytes_of(&itm.kernel_params)).enq()?;
        self.buf_matrices.write(matrices).enq()?;

        unsafe { self.kernel.enq()?; }

        match &mut buffers.output.data {
            BufferSource::None => { },
            BufferSource::Cpu { buffer, .. } => {
                self.dst.read(&mut **buffer).enq()?;
            },
            BufferSource::OpenGL { texture, .. } => {
                if let Some(ref tex) = self.image_dst {
                    let mut img = &tex.0;
                    if tex.1 != *texture as u64 {
                        let desc = ImageDescriptor::new(MemObjectType::Image2d, buffers.output.size.0,  buffers.output.size.1,  1, 1, buffers.output.size.2,  0, None);
                        _temp2 = Some(Image::<u8>::from_gl_texture(self.queue.clone(), MemFlags::new().write_only(), desc, GlTextureTarget::GlTexture2d, 0, *texture)?);
                        img = _temp2.as_ref().unwrap();
                    }

                    img.cmd().gl_acquire().enq()?;
                    if let SpatialDims::Three(w, h, d) = img.dims() {
                        let _ = self.dst.cmd().copy_to_image(&img, [0, 0, 0], [*w, *h, *d]).enq();
                    }
                    img.cmd().gl_release().enq()?;

                }
            },
            #[cfg(target_os = "windows")]
            BufferSource::DirectX11 { .. } => {
                if let Some(ref tex) = self.image_dst {
                    tex.0.cmd().d3d11_acquire().enq()?;
                    if let SpatialDims::Three(w, h, d) = tex.0.dims() {
                        let _ = self.dst.cmd().copy_to_image(&tex.0, [0, 0, 0], [*w, *h, *d]).enq();
                    }
                    tex.0.cmd().d3d11_release().enq()?;
                }
            }
            _ => { }
        }

        // self.queue.finish();

        Ok(())
    }
}

pub fn is_buffer_supported(buffers: &Buffers) -> bool {
    match buffers.input.data {
        BufferSource::None           => false,
        BufferSource::Cpu     { .. } => true,
        BufferSource::OpenGL  { .. } => true,
        BufferSource::OpenCL  { .. } => true,
        #[cfg(target_os = "windows")]
        BufferSource::DirectX11 { .. } => true,
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        BufferSource::Vulkan  { .. } => false,
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        BufferSource::CUDABuffer{ .. } => false,
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::Metal { .. } | BufferSource::MetalBuffer { .. } => false,
    }
}
