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
    output_width: u32,
    output_height: u32,
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
    out_pixels: wgpu::Texture,
    in_pixels: wgpu::Texture,
    params_buffer: wgpu::Buffer,
    globals_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    compute_pipeline: wgpu::ComputePipeline,

    in_stride: u32,
    out_stride: u32,
    padded_out_stride: u32,
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

    pub fn new(width: usize, height: usize, stride: usize, _bytes_per_pixel: usize, output_width: usize, output_height: usize, output_stride: usize, _pix_element_count: usize, bg: nalgebra::Vector4<f32>, interpolation: u32) -> Option<Self> {
        let params_count = 9 * (height + 1);

        if height < 4 || output_height < 4 || stride < 1 || width > 8192 || output_width > 8192 { return None; }

        let in_size = (stride * height) as wgpu::BufferAddress;
        let out_size = (output_stride * output_height) as wgpu::BufferAddress;
        let params_size = (params_count * std::mem::size_of::<f32>()) as wgpu::BufferAddress;

        let adapter_initialized = ADAPTER.read().is_some();
        if !adapter_initialized { Self::initialize_context(); }
        let lock = ADAPTER.read();
        if let Some(ref adapter) = *lock {
            let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: None,
                features: wgpu::Features::empty(),
                limits: wgpu::Limits::default(),
            }, None)).ok()?;

            let mut shader_str = include_str!("wgpu_undistort.wgsl").to_string();
            shader_str = shader_str.replace("texture_2d<f32>", "texture_2d<f32>");
            shader_str = shader_str.replace("rgba8unorm", "rgba8unorm");

            let shader = device.create_shader_module(&wgpu::ShaderModuleDescriptor {
                source: wgpu::ShaderSource::Wgsl(Cow::Owned(shader_str)),
                label: None
            });

            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
            let padding = (align - output_stride % align) % align;
            let padded_out_stride = output_stride + padding;
            let staging_size = padded_out_stride * output_height;

            let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor { size: staging_size as u64, usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
            let params_buffer  = device.create_buffer(&wgpu::BufferDescriptor { size: params_size, usage: BufferUsages::STORAGE | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
            let globals_buffer  = device.create_buffer(&wgpu::BufferDescriptor { size: std::mem::size_of::<Globals>() as u64, usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });

            let in_pixels = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: width as u32, height: height as u32, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            });
            let out_pixels = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: output_width as u32, height: output_height as u32, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            });

            let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { module: &shader, entry_point: "undistort", label: None, layout: None });

            let view = in_pixels.create_view(&wgpu::TextureViewDescriptor::default());
            let out_view = out_pixels.create_view(&wgpu::TextureViewDescriptor::default());

            let bind_group_layout = compute_pipeline.get_bind_group_layout(0);
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: globals_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: params_buffer.as_entire_binding() }, 
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view) }, 
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&out_view) },
                ],
            });
            let globals = Globals {
                width: width as u32,
                height: height as u32,

                output_width: output_width as u32,
                output_height: output_height as u32,
                interpolation,
                num_params: 2,
                bg: [bg[0] / 255.0, bg[1] / 255.0, bg[2] / 255.0, bg[3] / 255.0]
            };

            Some(Self {
                device,
                queue,
                staging_buffer,
                out_pixels,
                in_pixels,
                params_buffer,
                globals_buffer,
                bind_group,
                compute_pipeline,
                in_size,
                out_size,
                params_size,
                globals,
                in_stride: stride as u32,
                out_stride: output_stride as u32,
                padded_out_stride: padded_out_stride as u32
            })
        } else {
            None
        }
    }

    pub fn set_background(&mut self, bg: nalgebra::Vector4<f32>) {
        self.globals.bg = [bg[0] / 255.0, bg[1] / 255.0, bg[2] / 255.0, bg[3] / 255.0];
    }

    pub fn undistort_image(&mut self, pixels: &mut [u8], output_pixels: &mut [u8], itm: &crate::undistortion::FrameTransform) {
        let flattened_params = bytemuck::cast_slice(&itm.params);

        if self.in_size != pixels.len() as u64              { log::error!("Buffer size mismatch! {} vs {}", self.in_size, pixels.len()); return; }
        if self.out_size != output_pixels.len() as u64      { log::error!("Buffer size mismatch! {} vs {}", self.out_size, output_pixels.len()); return; }
        if self.params_size < flattened_params.len() as u64 { log::error!("Buffer size mismatch! {} vs {}", self.params_size, flattened_params.len()); return; }

        self.queue.write_buffer(&self.params_buffer, 0, flattened_params);

        self.globals.num_params = itm.params.len() as u32;
        self.queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&self.globals));
        self.queue.write_texture(
            self.in_pixels.as_image_copy(),
            bytemuck::cast_slice(&pixels),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: std::num::NonZeroU32::new(self.in_stride),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: self.globals.width as u32,
                height: self.globals.height as u32,
                depth_or_array_layers: 1,
            },
        );

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None });
            cpass.set_pipeline(&self.compute_pipeline);
            cpass.set_bind_group(0, &self.bind_group, &[]);
            cpass.dispatch(self.globals.width as u32 / 16 + 1, self.globals.height as u32 / 16 + 1, 1);
        }

        encoder.copy_texture_to_buffer(wgpu::ImageCopyTexture {
            texture: &self.out_pixels,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        }, wgpu::ImageCopyBuffer {
            buffer: &self.staging_buffer,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: std::num::NonZeroU32::new(self.padded_out_stride),
                rows_per_image: None,
            },
        }, wgpu::Extent3d {
            width: self.globals.output_width as u32,
            height: self.globals.output_height as u32,
            depth_or_array_layers: 1,
        });

        self.queue.submit(Some(encoder.finish()));

        let buffer_slice = self.staging_buffer.slice(..);
        let buffer_future = buffer_slice.map_async(wgpu::MapMode::Read);

        self.device.poll(wgpu::Maintain::Wait);

        if let Ok(()) = pollster::block_on(buffer_future) {
            let data = buffer_slice.get_mapped_range();
            if self.padded_out_stride == self.out_stride {
                // Fast path
                output_pixels.copy_from_slice(data.as_ref());
            } else {
                // data.as_ref()
                //     .chunks(self.padded_out_stride as usize)
                //     .zip(output_pixels.chunks_mut(self.out_stride as usize))
                //     .for_each(|(src, dest)| {
                //         dest.copy_from_slice(&src[0..self.out_stride as usize]);
                //     });
                use rayon::prelude::{ ParallelSliceMut, ParallelSlice };
                use rayon::iter::{ ParallelIterator, IndexedParallelIterator };
                data.as_ref()
                    .par_chunks(self.padded_out_stride as usize)
                    .zip(output_pixels.par_chunks_mut(self.out_stride as usize))
                    .for_each(|(src, dest)| {
                        dest.copy_from_slice(&src[0..self.out_stride as usize]);
                    });
            }

            // We have to make sure all mapped views are dropped before we unmap the buffer.
            drop(data);
            self.staging_buffer.unmap();
        } else {
            // TODO change to Result
            log::error!("failed to run compute on wgpu!")
        }
    }
}
