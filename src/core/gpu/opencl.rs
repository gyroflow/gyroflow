// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use ocl::*;
use parking_lot::RwLock;
use crate::stabilization::KernelParams;
use crate::stabilization::distortion_models::GoProSuperview;

pub struct OclWrapper {
    kernel: Kernel,
    src: Buffer<u8>,
    dst: Buffer<u8>,

    buf_params: Buffer<u8>,
    buf_matrices: Buffer<f32>,
}

struct CtxWrapper {
    device: Device,
    context: Context,
}

lazy_static::lazy_static! {
    static ref CONTEXT: RwLock<Option<CtxWrapper>> = RwLock::new(None);
}

const EXCLUSIONS: &[&'static str] = &["Microsoft Basic Render Driver"];

impl OclWrapper {
    pub fn list_devices() -> Vec<String> {
        let mut ret = Vec::new();
        for p in Platform::list() {
            if let Ok(devs) = Device::list_all(p) {
                ret.extend(devs.into_iter().filter_map(|x| Some(format!("{} {}", p.name().ok()?, x.name().ok()?))));
            }
        }
        ret.drain(..).filter(|x| !EXCLUSIONS.iter().any(|e| x.contains(e))).collect()
    }

    pub fn set_device(index: usize) -> ocl::Result<()> {
        let mut i = 0;
        for p in Platform::list() {
            if let Ok(devs) = Device::list_all(p) {
                for d in devs {
                    if EXCLUSIONS.iter().any(|x| d.name().unwrap_or_default().contains(x)) { continue; }
                    if i == index {
                        ::log::info!("OpenCL Platform: {}, Device: {} {}", p.name()?, d.vendor()?, d.name()?);

                        let context = Context::builder()
                            .platform(p)
                            .devices(d)
                            .build()?;

                        *CONTEXT.write() = Some(CtxWrapper { device: d, context });
                        return Ok(());
                    }
                    i += 1;
                }
            }
        }
        Err(ocl::BufferCmdError::MapUnavailable.into())
    }

    pub fn initialize_context() -> ocl::Result<(String, String)> {
        // List all devices
        Platform::list().iter().for_each(|p| {
            if let Ok(devs) = Device::list_all(p) {
                ::log::debug!("OpenCL devices: {:?} {:?}", p.name(), devs.iter().filter_map(|x| x.name().ok()).collect::<Vec<String>>());
            }
        });

        let mut platform = None;
        let mut device = None;
        let preference = [ "nvidia", "radeon", "geforce", "firepro", "accelerated parallel processing", "graphics" ];
        'outer: for pref in preference {
            for p in Platform::list() {
                if let Ok(devs) = Device::list_all(p) {
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
                if let Ok(devs) = Device::list_all(p) {
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
        ::log::info!("OpenCL Platform: {}, Device: {} {}", platform.name()?, device.vendor()?, device.name()?);

        let context = Context::builder()
            .platform(platform)
            .devices(device)
            .build()?;

        let name = format!("{} {}", device.vendor()?, device.name()?);
        let list_name = format!("[OpenCL] {} {}", platform.name()?, device.name()?);

        *CONTEXT.write() = Some(CtxWrapper { device, context });

        Ok((name, list_name))
    }

    pub fn new(params: &KernelParams, ocl_names: (&str, &str, &str, &str), lens_model_funcs: &str) -> ocl::Result<Self> {
        if params.height < 4 || params.output_height < 4 || params.stride < 1 { return Err(ocl::BufferCmdError::AlreadyMapped.into()); }

        let context_initialized = CONTEXT.read().is_some();
        if !context_initialized { Self::initialize_context()?; }
        let lock = CONTEXT.read();
        if let Some(ref ctx) = *lock {
            if ctx.device.name()?.to_ascii_lowercase().contains("core(tm)") {
                return Err(ocl::BufferCmdError::AlreadyMapped.into());
            }
            let queue = Queue::new(&ctx.context, ctx.device, None)?;

            let mut kernel = include_str!("opencl_undistort.cl").to_string();
            kernel.insert_str(0, GoProSuperview::opencl_functions());
            kernel.insert_str(0, lens_model_funcs);
            kernel = kernel.replace("DATA_CONVERTF", ocl_names.3)
                           .replace("DATA_TYPEF", ocl_names.2)
                           .replace("DATA_CONVERT", ocl_names.1)
                           .replace("DATA_TYPE", ocl_names.0)
                           .replace("PIXEL_BYTES", &format!("{}", params.bytes_per_pixel))
                           .replace("INTERPOLATION", &format!("{}", params.interpolation));

            let program = Program::builder()
                .src(&kernel)
                .devices(ctx.device)
                .build(&ctx.context)?;

            let source_buffer = Buffer::builder().queue(queue.clone()).len(params.stride*params.height)
                .flags(MemFlags::new().read_only().host_write_only()).build()?;

            let dest_buffer = Buffer::builder().queue(queue.clone()).len(params.output_stride*params.output_height)
                .flags(MemFlags::new().write_only().host_read_only().alloc_host_ptr()).build()?;

            let buf_params = Buffer::builder().queue(queue.clone()).len(std::mem::size_of::<KernelParams>())
                .flags(MemFlags::new().read_only().host_write_only()).build()?;

            let max_matrix_count = 9 * params.height;
            let buf_matrices = Buffer::<f32>::builder().queue(queue.clone()).flags(MemFlags::new().read_only()).len(max_matrix_count).build()?;

            let mut builder = Kernel::builder();
            unsafe {
                builder.program(&program).name("undistort_image").queue(queue)
                .global_work_size((params.output_width, params.output_height))
                .disable_arg_type_check()
                .arg(&source_buffer)
                .arg(&dest_buffer)
                .arg(&buf_params)
                .arg(&buf_matrices);
            }

            let kernel = builder.build()?;

            Ok(Self {
                kernel,
                src: source_buffer,
                dst: dest_buffer,
                buf_params,
                buf_matrices,
            })
        } else {
            Err(ocl::BufferCmdError::AlreadyMapped.into())
        }
    }

    pub fn undistort_image(&mut self, pixels: &mut [u8], out_pixels: &mut [u8], itm: &crate::stabilization::FrameTransform) -> ocl::Result<()> {
        let matrices = unsafe { std::slice::from_raw_parts(itm.matrices.as_ptr() as *const f32, itm.matrices.len() * 9 ) };

        if self.src.len() != pixels.len()           { log::error!("Buffer size mismatch! {} vs {}", self.src.len(), pixels.len()); return Ok(()); }
        if self.dst.len() != out_pixels.len()       { log::error!("Buffer size mismatch! {} vs {}", self.dst.len(), out_pixels.len()); return Ok(()); }
        if self.buf_matrices.len() < matrices.len() { log::error!("Buffer size mismatch! {} vs {}", self.buf_matrices.len(), matrices.len()); return Ok(()); }

        self.src.write(pixels as &[u8]).enq()?;

        self.buf_params.write(bytemuck::bytes_of(&itm.kernel_params)).enq()?;
        self.buf_matrices.write(matrices).enq()?;

        unsafe { self.kernel.enq()?; }

        self.dst.read(out_pixels).enq()?;
        Ok(())
    }
}
