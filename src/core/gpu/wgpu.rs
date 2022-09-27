// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::borrow::Cow;
use wgpu::Adapter;
use wgpu::BufferUsages;
use wgpu::util::DeviceExt;
use parking_lot::RwLock;
use crate::gpu:: { BufferDescription, BufferSource };
use crate::stabilization::ComputeParams;
use crate::stabilization::KernelParams;

pub struct WgpuWrapper  {
    pub device: wgpu::Device,
    queue: wgpu::Queue,
    staging_buffer: wgpu::Buffer,
    out_pixels: wgpu::Texture,
    in_pixels: wgpu::Texture,
    buf_matrices: wgpu::Buffer,
    buf_params: wgpu::Buffer,
    buf_drawing: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    render_pipeline: wgpu::RenderPipeline,

    padded_out_stride: u32,
    in_size: u64,
    out_size: u64,
    params_size: u64,
    drawing_size: u64,
}

lazy_static::lazy_static! {
    static ref ADAPTER: RwLock<Option<Adapter>> = RwLock::new(None);
}

impl WgpuWrapper {
    pub fn list_devices() -> Vec<String> {
        let instance = wgpu::Instance::new(wgpu::Backends::all());

        let adapters = instance.enumerate_adapters(wgpu::Backends::all());
        adapters.map(|x| { let x = x.get_info(); format!("{} ({:?})", x.name, x.backend) }).collect()
    }

    pub fn set_device(index: usize, _buffers: &BufferDescription) -> Option<()> {
        let instance = wgpu::Instance::new(wgpu::Backends::all());

        let mut i = 0;
        for a in instance.enumerate_adapters(wgpu::Backends::all()) {
            if i == index {
                let info = a.get_info();
                log::debug!("WGPU adapter: {:?}", &info);

                *ADAPTER.write() = Some(a);
                return Some(());
            }
            i += 1;
        }
        None
    }
    pub fn get_info() -> Option<String> {
        let lock = ADAPTER.read();
        if let Some(ref adapter) = *lock {
            let info = adapter.get_info();
            Some(format!("{} ({:?})", info.name, info.backend))
        } else {
            None
        }
    }

    pub fn initialize_context() -> Option<(String, String)> {
        let instance = wgpu::Instance::new(wgpu::Backends::all());

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))?;
        let info = adapter.get_info();
        log::debug!("WGPU adapter: {:?}", &info);
        if info.device_type == wgpu::DeviceType::Cpu {
            return None;
        }

        let name = info.name.clone();
        let list_name = format!("[wgpu] {} ({:?})", info.name, info.backend);

        *ADAPTER.write() = Some(adapter);

        Some((name, list_name))
    }

    pub fn new(params: &KernelParams, wgpu_format: (wgpu::TextureFormat, &str, f64), compute_params: &ComputeParams, _buffers: &BufferDescription, drawing_len: usize) -> Option<Self> {
        let max_matrix_count = 9 * params.height as usize;

        if params.height < 4 || params.output_height < 4 || params.stride < 1 || params.width > 8192 || params.output_width > 8192 { return None; }

        let in_size = (params.stride * params.height) as wgpu::BufferAddress;
        let out_size = (params.output_stride * params.output_height) as wgpu::BufferAddress;
        let params_size = (max_matrix_count * std::mem::size_of::<f32>()) as wgpu::BufferAddress;

        let adapter_initialized = ADAPTER.read().is_some();
        if !adapter_initialized { Self::initialize_context(); }
        let lock = ADAPTER.read();
        if let Some(ref adapter) = *lock {
            let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: None,
                features: wgpu::Features::empty(),
                limits: wgpu::Limits {
                    max_storage_buffers_per_shader_stage: 8,
                    max_storage_textures_per_shader_stage: 8,
                    ..wgpu::Limits::default()
                },
            }, None)).ok()?;

            let mut kernel = include_str!("wgpu_undistort.wgsl").to_string();
            //let mut kernel = std::fs::read_to_string("D:/programowanie/projekty/Rust/gyroflow/src/core/gpu/wgpu_undistort.wgsl").unwrap();

            let mut lens_model_functions = compute_params.distortion_model.wgsl_functions().to_string();
            let default_digital_lens = "fn digital_undistort_point(uv: vec2<f32>) -> vec2<f32> { return uv; }
                                            fn digital_distort_point(uv: vec2<f32>) -> vec2<f32> { return uv; }";
            lens_model_functions.push_str(compute_params.digital_lens.as_ref().map(|x| x.wgsl_functions()).unwrap_or(default_digital_lens));
            kernel = kernel.replace("LENS_MODEL_FUNCTIONS;", &lens_model_functions);
            kernel = kernel.replace("SCALAR", wgpu_format.1);
            kernel = kernel.replace("bg_scaler", &format!("{:.6}", wgpu_format.2));
            // Replace it in source to allow for loop unrolling when compiling shader
            kernel = kernel.replace("params.interpolation", &format!("{}u", params.interpolation));

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                source: wgpu::ShaderSource::Wgsl(Cow::Owned(kernel)),
                label: None
            });

            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as i32;
            let padding = (align - params.output_stride % align) % align;
            let padded_out_stride = params.output_stride + padding;
            let staging_size = padded_out_stride * params.output_height;

            let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor { size: staging_size as u64, usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
            let buf_matrices  = device.create_buffer(&wgpu::BufferDescriptor { size: params_size, usage: BufferUsages::STORAGE | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
            let buf_params = device.create_buffer(&wgpu::BufferDescriptor { size: std::mem::size_of::<KernelParams>() as u64, usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
            let buf_drawing = device.create_buffer(&wgpu::BufferDescriptor { size: drawing_len as u64, usage: BufferUsages::STORAGE | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
            let buf_coeffs  = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&crate::stabilization::COEFFS), usage: wgpu::BufferUsages::STORAGE });

            let in_pixels = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: params.width as u32, height: params.height as u32, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu_format.0,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            });
            let out_pixels = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: params.output_width as u32, height: params.output_height as u32, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu_format.0,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            });

            let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: "undistort_vertex",
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: "undistort_fragment",
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu_format.0,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                multiview: None,
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
            });

            let view = in_pixels.create_view(&wgpu::TextureViewDescriptor::default());

            let bind_group_layout = render_pipeline.get_bind_group_layout(0);
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: buf_params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: buf_matrices.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 3, resource: buf_coeffs.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: buf_drawing.as_entire_binding() },
                ],
            });

            Some(Self {
                device,
                queue,
                staging_buffer,
                out_pixels,
                in_pixels,
                buf_matrices,
                buf_params,
                buf_drawing,
                bind_group,
                render_pipeline,
                in_size,
                out_size,
                params_size,
                drawing_size: drawing_len as u64,
                padded_out_stride: padded_out_stride as u32
            })
        } else {
            None
        }
    }

    pub fn undistort_image(&self, buffers: &mut BufferDescription, itm: &crate::stabilization::FrameTransform, drawing_buffer: &[u8]) -> bool {
        let matrices = bytemuck::cast_slice(&itm.matrices);

        match &buffers.buffers {
            BufferSource::Cpu { input, output } => {
                if self.in_size  != input.len()  as u64 { log::error!("Buffer size mismatch! {} vs {}", self.in_size,  input.len()); return false; }
                if self.out_size != output.len() as u64 { log::error!("Buffer size mismatch! {} vs {}", self.out_size, output.len()); return false; }

                self.queue.write_texture(
                    self.in_pixels.as_image_copy(),
                    bytemuck::cast_slice(input),
                    wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: std::num::NonZeroU32::new(itm.kernel_params.stride as u32),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width: itm.kernel_params.width as u32,
                        height: itm.kernel_params.height as u32,
                        depth_or_array_layers: 1,
                    },
                );
            },
            BufferSource::OpenCL { .. } => {
                return false;
            },
            BufferSource::DirectX { .. } => {
                return false;
            },
            BufferSource::OpenGL { .. } => {
                return false;
            }
        }

        if self.params_size < matrices.len() as u64    { log::error!("Buffer size mismatch! {} vs {}", self.params_size, matrices.len()); return false; }

        self.queue.write_buffer(&self.buf_matrices, 0, matrices);
        self.queue.write_buffer(&self.buf_params, 0, bytemuck::bytes_of(&itm.kernel_params));
        if !drawing_buffer.is_empty() {
            if self.drawing_size < drawing_buffer.len() as u64 { log::error!("Buffer size mismatch! {} vs {}", self.drawing_size, drawing_buffer.len()); return false; }
            self.queue.write_buffer(&self.buf_drawing, 0, drawing_buffer);
        }

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        let view = self.out_pixels.create_view(&wgpu::TextureViewDescriptor::default());
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: true,
                    },
                })],
                depth_stencil_attachment: None,
            });
            rpass.set_pipeline(&self.render_pipeline);
            rpass.set_bind_group(0, &self.bind_group, &[]);
            rpass.draw(0..6, 0..1);
        }

        if let BufferSource::Cpu { .. } = buffers.buffers {
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
                width: itm.kernel_params.output_width as u32,
                height: itm.kernel_params.output_height as u32,
                depth_or_array_layers: 1,
            });
        }

        self.queue.submit(Some(encoder.finish()));

        if let BufferSource::Cpu { output, .. } = &mut buffers.buffers {
            let buffer_slice = self.staging_buffer.slice(..);
            let (sender, receiver) = futures_intrusive::channel::shared::oneshot_channel();
            buffer_slice.map_async(wgpu::MapMode::Read, move |v| sender.send(v).unwrap());

            self.device.poll(wgpu::Maintain::Wait);

            if let Some(Ok(())) = pollster::block_on(receiver.receive()) {
                let data = buffer_slice.get_mapped_range();
                if self.padded_out_stride == itm.kernel_params.output_stride as u32 {
                    // Fast path
                    output.copy_from_slice(data.as_ref());
                } else {
                    // data.as_ref()
                    //     .chunks(self.padded_out_stride as usize)
                    //     .zip(output.chunks_mut(itm.kernel_params.output_stride as usize))
                    //     .for_each(|(src, dest)| {
                    //         dest.copy_from_slice(&src[0..itm.kernel_params.output_stride as usize]);
                    //     });
                    use rayon::prelude::{ ParallelSliceMut, ParallelSlice };
                    use rayon::iter::{ ParallelIterator, IndexedParallelIterator };
                    data.as_ref()
                        .par_chunks(self.padded_out_stride as usize)
                        .zip(output.par_chunks_mut(itm.kernel_params.output_stride as usize))
                        .for_each(|(src, dest)| {
                            dest.copy_from_slice(&src[0..itm.kernel_params.output_stride as usize]);
                        });
                }

                // We have to make sure all mapped views are dropped before we unmap the buffer.
                drop(data);
                self.staging_buffer.unmap();
            } else {
                // TODO change to Result
                log::error!("failed to run compute on wgpu!");
                return false;
            }
        }
        true
    }
}

pub fn is_buffer_supported(buffers: &BufferDescription) -> bool {
    match buffers.buffers {
        BufferSource::Cpu     { .. } => true,
        BufferSource::OpenGL  { .. } => false,
        BufferSource::DirectX { .. } => false,
        #[cfg(feature = "use-opencl")]
        BufferSource::OpenCL  { .. } => false,
    }
}
