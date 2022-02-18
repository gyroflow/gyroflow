// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use ocl::*;
use parking_lot::RwLock;

pub struct OclWrapper {
    kernel: Kernel,
    src: Buffer<u8>,
    dst: Buffer<u8>,
    pix_element_count: usize,

    params_buf: Buffer<f32>,
}

struct CtxWrapper {
    device: Device,
    context: Context,
}

lazy_static::lazy_static! {
    static ref CONTEXT: RwLock<Option<CtxWrapper>> = RwLock::new(None);
}

impl OclWrapper {
    pub fn initialize_context() -> ocl::Result<String> {
        let platform = Platform::default();
        let device = Device::first(platform)?;
        ::log::info!("OpenCL Platform: {}, Device: {} {}", platform.name()?, device.vendor()?, device.name()?);

        let context = Context::builder()
            .platform(platform)
            .devices(device)
            .build()?;

        let name = format!("{} {}", device.vendor()?, device.name()?);

        *CONTEXT.write() = Some(CtxWrapper { device, context });
        
        Ok(name)
    }

    pub fn new(width: usize, height: usize, stride: usize, bytes_per_pixel: usize, output_width: usize, output_height: usize, output_stride: usize, pix_element_count: usize, ocl_names: (&str, &str, &str, &str), bg: nalgebra::Vector4<f32>, interpolation: u32) -> ocl::Result<Self> {
        if height < 4 || output_height < 4 || stride < 1 { return Err(ocl::BufferCmdError::AlreadyMapped.into()); }
        
        let context_initialized = CONTEXT.read().is_some();
        if !context_initialized { Self::initialize_context()?; }
        let lock = CONTEXT.read();
        if let Some(ref ctx) = *lock {
            let queue = Queue::new(&ctx.context, ctx.device, None)?;

            let program = Program::builder()
                .src(include_str!("opencl_undistort.cl"))
                .bo(builders::BuildOpt::CmplrDefine { ident: "DATA_TYPE"    .into(), val: ocl_names.0.into() })
                .bo(builders::BuildOpt::CmplrDefine { ident: "DATA_CONVERT" .into(), val: ocl_names.1.into() })
                .bo(builders::BuildOpt::CmplrDefine { ident: "DATA_TYPEF"   .into(), val: ocl_names.2.into() })
                .bo(builders::BuildOpt::CmplrDefine { ident: "DATA_CONVERTF".into(), val: ocl_names.3.into() })
                .bo(builders::BuildOpt::CmplrDefine { ident: "PIXEL_BYTES"  .into(), val: format!("{}", bytes_per_pixel) })
                .bo(builders::BuildOpt::CmplrDefine { ident: "INTERPOLATION".into(), val: format!("{}", interpolation) })
                .devices(ctx.device)
                .build(&ctx.context)?;

            let source_buffer = Buffer::builder().queue(queue.clone()).len(stride*height)
                .flags(MemFlags::new().read_only().host_write_only()).build()?;

            let dest_buffer = Buffer::builder().queue(queue.clone()).len(output_stride*output_height)
                .flags(MemFlags::new().write_only().host_read_only().alloc_host_ptr()).build()?;

            let params_len = 9 * (height + 1);
            let params_buf = Buffer::<f32>::builder().queue(queue.clone()).flags(MemFlags::new().read_only()).len(params_len).build()?;

            let mut builder = Kernel::builder();
            unsafe {
                builder.program(&program).name("undistort_image").queue(queue)
                .global_work_size((output_width, output_height))
                .disable_arg_type_check()
                .arg(&source_buffer)
                .arg(&dest_buffer)
                .arg(ocl::prm::Ushort::new(width as u16))
                .arg(ocl::prm::Ushort::new(height as u16))
                .arg(ocl::prm::Ushort::new(stride as u16))
                .arg(ocl::prm::Ushort::new(output_width as u16))
                .arg(ocl::prm::Ushort::new(output_height as u16))
                .arg(ocl::prm::Ushort::new(output_stride as u16))
                .arg(&params_buf)
                .arg(ocl::prm::Ushort::new(2));
            }

            match pix_element_count {
                1 => builder.arg(ocl::prm::Float::new(bg[0])),
                2 => builder.arg(ocl::prm::Float2::new(bg[0], bg[1])),
                3 => builder.arg(ocl::prm::Float3::new(bg[0], bg[1], bg[2])),
                4 => builder.arg(ocl::prm::Float4::new(bg[0], bg[1], bg[2], bg[3])),
                _ => panic!("Unknown pix_element_count {}", pix_element_count)
            };
            let kernel = builder.build()?;
        
            Ok(Self {
                pix_element_count,
                kernel,
                src: source_buffer,
                dst: dest_buffer,
                params_buf,
            })
        } else {
            Err(ocl::BufferCmdError::AlreadyMapped.into())
        }
    }
    
    pub fn set_background(&mut self, bg: nalgebra::Vector4<f32>) -> ocl::Result<()> {
        match self.pix_element_count {
            1 => self.kernel.set_arg(10, ocl::prm::Float::new(bg[0]))?,
            2 => self.kernel.set_arg(10, ocl::prm::Float2::new(bg[0], bg[1]))?,
            3 => self.kernel.set_arg(10, ocl::prm::Float3::new(bg[0], bg[1], bg[2]))?,
            4 => self.kernel.set_arg(10, ocl::prm::Float4::new(bg[0], bg[1], bg[2], bg[3]))?,
            _ => panic!("Unknown pix_element_count {}", self.pix_element_count)
        };
        Ok(())
    }
    pub fn undistort_image(&mut self, pixels: &mut [u8], out_pixels: &mut [u8], itm: &crate::undistortion::FrameTransform) -> ocl::Result<()> {
        let flattened_params = unsafe { std::slice::from_raw_parts(itm.params.as_ptr() as *const f32, itm.params.len() * 9 ) };

        if self.src.len() != pixels.len()                 { log::error!("Buffer size mismatch! {} vs {}", self.src.len(), pixels.len()); return Ok(()); }
        if self.dst.len() != out_pixels.len()             { log::error!("Buffer size mismatch! {} vs {}", self.dst.len(), out_pixels.len()); return Ok(()); }
        if self.params_buf.len() < flattened_params.len() { log::error!("Buffer size mismatch! {} vs {}", self.params_buf.len(), flattened_params.len()); return Ok(()); }

        self.src.write(pixels as &[u8]).enq()?;

        self.params_buf.write(flattened_params).enq()?;

        self.kernel.set_arg(9, ocl::prm::Ushort::new(itm.params.len() as u16))?;

        unsafe { self.kernel.enq()?; }

        self.dst.read(out_pixels).enq()?;
        Ok(())
    }
}
