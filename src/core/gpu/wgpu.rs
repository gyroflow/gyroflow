// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::borrow::Cow;
use bytemuck::Pod;
use bytemuck::Zeroable;
use wgpu::Adapter;
use wgpu::BufferUsages;
use parking_lot::RwLock;

#[repr(C, align(32))]
#[derive(Clone, Copy)]
struct Globals {
    width: u32,
    height: u32,
    stride: u32,
    output_width: u32,
    output_height: u32,
    output_stride: u32,
    bytes_per_pixel: u32,
    pix_element_count: u32,
    num_params: u32,
    interpolation: u32,
    bg: [f32; 4]
}
unsafe impl Zeroable for Globals {}
unsafe impl Pod for Globals {}

pub struct WgpuWrapper  {
    device: wgpu::Device,
    queue: wgpu::Queue,
    staging_buffer: wgpu::Buffer,
    out_pixels: wgpu::Buffer,
    in_pixels: wgpu::Buffer,
    params_buffer: wgpu::Buffer,
    params2_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    compute_pipeline: wgpu::ComputePipeline,

    in_size: u64,
    out_size: u64,
    params_size: u64,

    globals: Globals
}

lazy_static::lazy_static! {
    static ref ADAPTER: RwLock<Option<Adapter>> = RwLock::new(None);
}

impl WgpuWrapper {
    pub fn initialize_context() -> Option<String> {
        let instance = wgpu::Instance::new(wgpu::Backends::all());

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
        let info = adapter.get_info();
        log::debug!("WGPU adapter: {:?}", &info);

        let name = info.name.clone();

        *ADAPTER.write() = Some(adapter);
        
        Some(name)
    }

    pub fn new(width: usize, height: usize, stride: usize, bytes_per_pixel: usize, output_width: usize, output_height: usize, output_stride: usize, pix_element_count: usize, bg: nalgebra::Vector4<f32>, mut interpolation: u32) -> Option<Self> {
        let params_count = 9 * (height + 1);

        if height < 4 || output_height < 4 || stride < 1 { return None; }

        let in_size = (stride * height) as wgpu::BufferAddress;
        let out_size = (output_stride * output_height) as wgpu::BufferAddress;
        let params_size = (params_count * std::mem::size_of::<f32>()) as wgpu::BufferAddress;

        if interpolation != 2 {
            interpolation = 2; // TODO: Bicubic and Lanczos are not working
        }

        //let instance = wgpu::Instance::new(wgpu::Backends::all());

        //let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
        let adapter_initialized = ADAPTER.read().is_some();
        if !adapter_initialized { Self::initialize_context(); }
        let lock = ADAPTER.read();
        if let Some(ref adapter) = *lock {
            let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: None,
                features: wgpu::Features::empty(),
                limits: wgpu::Limits::default(),
            }, None)).ok()?;

            let info = adapter.get_info();
            log::debug!("WGPU adapter: {:?}", &info);

            let shader = device.create_shader_module(&wgpu::ShaderModuleDescriptor {
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("wgpu_undistort.wgsl"))),
                label: None
            });

            let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor { size: out_size, usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
            let in_pixels      = device.create_buffer(&wgpu::BufferDescriptor { size: in_size,  usage: BufferUsages::STORAGE | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
            let out_pixels     = device.create_buffer(&wgpu::BufferDescriptor { size: out_size, usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC, label: None, mapped_at_creation: false });
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
                width: width as u32,
                height: height as u32,
                stride: stride as u32,

                output_width: output_width as u32,
                output_height: output_height as u32,
                output_stride: output_stride as u32,
                bytes_per_pixel: bytes_per_pixel as u32,
                pix_element_count: pix_element_count as u32,
                interpolation,
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
                in_size,
                out_size,
                params_size,
                globals
            })
        } else {
            None
        }
    }

    pub fn set_background(&mut self, bg: nalgebra::Vector4<f32>) {
        self.globals.bg = [bg[0], bg[1], bg[2], bg[3]];
    }

    pub fn undistort_image(&mut self, pixels: &mut [u8], output_pixels: &mut [u8], itm: &crate::undistortion::FrameTransform) {
        let flattened_params = bytemuck::cast_slice(&itm.params);

        if self.in_size != pixels.len() as u64              { log::error!("Buffer size mismatch! {} vs {}", self.in_size, pixels.len()); return; }
        if self.out_size != output_pixels.len() as u64      { log::error!("Buffer size mismatch! {} vs {}", self.out_size, output_pixels.len()); return; }
        if self.params_size < flattened_params.len() as u64 { log::error!("Buffer size mismatch! {} vs {}", self.params_size, flattened_params.len()); return; }

        self.queue.write_buffer(&self.params_buffer, 0, flattened_params);
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
        encoder.copy_buffer_to_buffer(&self.out_pixels, 0, &self.staging_buffer, 0, output_pixels.len() as wgpu::BufferAddress);

        self.queue.submit(Some(encoder.finish()));

        let buffer_slice = self.staging_buffer.slice(..);
        let buffer_future = buffer_slice.map_async(wgpu::MapMode::Read);

        self.device.poll(wgpu::Maintain::Wait);

        if let Ok(()) = pollster::block_on(buffer_future) {
            let data = buffer_slice.get_mapped_range();
            output_pixels.copy_from_slice(data.as_ref());

            // We have to make sure all mapped views are dropped before we unmap the buffer.
            drop(data);
            self.staging_buffer.unmap();
        } else {
            // TODO change to Result
            log::error!("failed to run compute on wgpu!")
        }
    }
}
