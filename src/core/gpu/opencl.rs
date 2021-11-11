use ocl::*;

pub struct OclWrapper<T: ocl::OclPrm> {
    kernel: Kernel,
    src: Buffer<T>,
    dst: Buffer<T>,
    pix_element_count: usize,

    params_buf: Buffer<f32>,
}

impl<T: ocl::OclPrm> OclWrapper<T> {
    pub fn new(width: usize, height: usize, stride: usize, pix_element_count: usize, ocl_names: (&str, &str, &str, &str), bg: nalgebra::Vector4<f32>) -> ocl::Result<Self> {
        let platform = Platform::default();
        let device = Device::first(platform)?;
        println!("Platform: {}, Device: {} {}", platform.name()?, device.vendor()?, device.name()?);

        let context = Context::builder()
            .platform(platform)
            .devices(device)
            .build()?;

        let queue = Queue::new(&context, device, None)?;

        let program = Program::builder()
            .src(include_str!("opencl_undistort.cl"))
            .bo(builders::BuildOpt::CmplrDefine { ident: "DATA_TYPE"    .into(), val: ocl_names.0.into() })
            .bo(builders::BuildOpt::CmplrDefine { ident: "DATA_CONVERT" .into(), val: ocl_names.1.into() })
            .bo(builders::BuildOpt::CmplrDefine { ident: "DATA_TYPEF"   .into(), val: ocl_names.2.into() })
            .bo(builders::BuildOpt::CmplrDefine { ident: "DATA_CONVERTF".into(), val: ocl_names.3.into() })
            .devices(device)
            .build(&context)?;

        let source_buffer = Buffer::builder().queue(queue.clone()).len(stride*height*pix_element_count)
            .flags(MemFlags::new().read_only().host_write_only()).build()?;

        let dest_buffer = Buffer::builder().queue(queue.clone()).len(stride*height*pix_element_count)
            .flags(MemFlags::new().write_only().host_read_only().alloc_host_ptr()).build()?;

        let params_len = 9 * (height + 1);
        let params_buf = Buffer::<f32>::builder().queue(queue.clone()).flags(MemFlags::new().read_only()).len(params_len).build()?;

        let mut builder = Kernel::builder();
        unsafe {
            builder.program(&program).name("undistort_image").queue(queue)
            .global_work_size((width, height))
            .disable_arg_type_check()
            .arg(&source_buffer)
            .arg(&dest_buffer)
            .arg(ocl::prm::Ushort::new(width as u16))
            .arg(ocl::prm::Ushort::new(height as u16))
            .arg(ocl::prm::Ushort::new(stride as u16))
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
            kernel: kernel,
            src: source_buffer,
            dst: dest_buffer,
            params_buf,
        })
    }
    
    pub fn set_background(&mut self, bg: nalgebra::Vector4<f32>) -> ocl::Result<()> {
        match self.pix_element_count {
            1 => self.kernel.set_arg(7, ocl::prm::Float::new(bg[0]))?,
            2 => self.kernel.set_arg(7, ocl::prm::Float2::new(bg[0], bg[1]))?,
            3 => self.kernel.set_arg(7, ocl::prm::Float3::new(bg[0], bg[1], bg[2]))?,
            4 => self.kernel.set_arg(7, ocl::prm::Float4::new(bg[0], bg[1], bg[2], bg[3]))?,
            _ => panic!("Unknown pix_element_count {}", self.pix_element_count)
        };
        Ok(())
    }
    pub fn undistort_image(&mut self, pixels: &mut [T], itm: &crate::core::undistortion::FrameTransform) -> ocl::Result<()> {
        self.src.write(pixels as &[T]).enq()?;

        let flattened_params = unsafe { std::slice::from_raw_parts(itm.params.as_ptr() as *const f32, itm.params.len() * 9 ) };

        self.params_buf.write(flattened_params).enq()?;

        self.kernel.set_arg(6, ocl::prm::Ushort::new(itm.params.len() as u16))?;

        unsafe { self.kernel.enq()?; }

        self.dst.read(pixels).enq()?;

        Ok(())
    }
}
