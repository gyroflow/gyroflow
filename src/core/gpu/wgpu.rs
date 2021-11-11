use std::borrow::Cow;
use bytemuck::Pod;
use bytemuck::Zeroable;
use wgpu::BufferUsages;

#[repr(C, align(32))]
#[derive(Clone, Copy)]
struct Globals {
    width: f32,
    height: f32,
    num_params: u32,
    bg: [f32; 4]
}
unsafe impl Zeroable for Globals {}
unsafe impl Pod for Globals {}

pub struct WgpuWrapper<T: bytemuck::Pod + std::marker::Sync>  {
    device: wgpu::Device,
    queue: wgpu::Queue,
    staging_buffer: wgpu::Buffer,
    out_pixels: wgpu::Buffer,
    in_pixels: wgpu::Buffer,
    params_buffer: wgpu::Buffer,
    params2_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    compute_pipeline: wgpu::ComputePipeline,

    globals: Globals,

    _d: std::marker::PhantomData<T>
}

impl<T: bytemuck::Pod + std::marker::Sync> WgpuWrapper<T>  {
    pub fn new(width: usize, height: usize, bg: nalgebra::Vector4<f32>) -> Option<Self> {
        let params_count = 9 * (height + 1);

        let size = (width * height * 4 * std::mem::size_of::<T>()) as wgpu::BufferAddress;
        let params_size = (params_count * std::mem::size_of::<f32>()) as wgpu::BufferAddress;

        let instance = wgpu::Instance::new(wgpu::Backends::all());

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: None,
            features: wgpu::Features::empty(),
            limits: wgpu::Limits::downlevel_defaults(),
        }, None)).ok()?;

        
        let info = adapter.get_info();
        dbg!(&info);

        let shader = device.create_shader_module(&wgpu::ShaderModuleDescriptor {
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("wgpu_undistort.wgsl"))),
            label: None
        });

        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor { size, usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
        let in_pixels      = device.create_buffer(&wgpu::BufferDescriptor { size, usage: BufferUsages::STORAGE | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
        let out_pixels     = device.create_buffer(&wgpu::BufferDescriptor { size, usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC, label: None, mapped_at_creation: false });
        let params_buffer  = device.create_buffer(&wgpu::BufferDescriptor { size: params_size, usage: BufferUsages::STORAGE | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
        
        let params2_buffer  = device.create_buffer(&wgpu::BufferDescriptor { size: std::mem::size_of::<Globals>() as u64, usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { module: &shader, entry_point: "undistort", label: None, layout: None });

        let bind_group_layout = compute_pipeline.get_bind_group_layout(0);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: in_pixels.as_entire_binding() }, 
                wgpu::BindGroupEntry { binding: 1, resource: params_buffer.as_entire_binding() }, 
                wgpu::BindGroupEntry { binding: 2, resource: out_pixels.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: params2_buffer.as_entire_binding() },
            ],
        });
        let globals = Globals {
            width: width as f32,
            height: height as f32,
            num_params: 2,
            bg: [bg[0], bg[1], bg[2], bg[3]]
        };

        Some(Self {
            device,
            queue,
            staging_buffer,
            out_pixels,
            in_pixels,
            params_buffer,
            params2_buffer,
            bind_group,
            compute_pipeline,
            globals,
            _d: Default::default()
        })
    }

    pub fn set_background(&mut self, bg: nalgebra::Vector4<f32>) {
        self.globals.bg = [bg[0], bg[1], bg[2], bg[3]];
    }

    pub fn undistort_image(&mut self, pixels: &mut [T], itm: &crate::core::undistortion::FrameTransform) {
        let size = (self.globals.width * self.globals.height * 4.0 * std::mem::size_of::<T>() as f32) as wgpu::BufferAddress;

        self.queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&itm.params));
        self.queue.write_buffer(&self.in_pixels, 0, bytemuck::cast_slice(pixels));

        self.globals.num_params = itm.params.len() as u32;
        self.queue.write_buffer(&self.params2_buffer, 0, bytemuck::bytes_of(&self.globals));

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None });
            cpass.set_pipeline(&self.compute_pipeline);
            cpass.set_bind_group(0, &self.bind_group, &[]);
            cpass.dispatch(self.globals.width as u32, self.globals.height as u32, 1);
        }

        // Will copy data from storage buffer on GPU to staging buffer on CPU.
        encoder.copy_buffer_to_buffer(&self.out_pixels, 0, &self.staging_buffer, 0, size);

        self.queue.submit(Some(encoder.finish()));

        let buffer_slice = self.staging_buffer.slice(..);
        let buffer_future = buffer_slice.map_async(wgpu::MapMode::Read);

        self.device.poll(wgpu::Maintain::Wait);

        if let Ok(()) = pollster::block_on(buffer_future) {
            let data = buffer_slice.get_mapped_range();
            bytemuck::cast_slice_mut(pixels).copy_from_slice(data.as_ref());

            // We have to make sure all mapped views are dropped before we unmap the buffer.
            drop(data);
            self.staging_buffer.unmap();
        } else {
            panic!("failed to run compute on gpu!")
        }
    }
}
