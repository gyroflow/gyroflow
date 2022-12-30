// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::borrow::Cow;
use std::sync::atomic::{ AtomicUsize, Ordering::SeqCst };
use wgpu::Adapter;
use wgpu::BufferUsages;
use wgpu::util::DeviceExt;
use parking_lot::{ RwLock, Mutex };
use crate::gpu:: { Buffers, BufferSource };
use crate::stabilization::ComputeParams;
use crate::stabilization::KernelParams;
use super::wgpu_interop::*;

pub struct WgpuWrapper  {
    pub device: wgpu::Device,
    queue: wgpu::Queue,
    staging_buffer: wgpu::Buffer,
    buf_matrices: wgpu::Buffer,
    buf_params: wgpu::Buffer,
    buf_drawing: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    render_pipeline: wgpu::RenderPipeline,
    pixel_format: wgpu::TextureFormat,

    in_texture: TextureHolder,
    out_texture: TextureHolder,

    padded_out_stride: u32,
    in_size: u64,
    out_size: u64,
    params_size: u64,
    drawing_size: u64,
}

lazy_static::lazy_static! {
    static ref INSTANCE: Mutex<wgpu::Instance> = Mutex::new(wgpu::Instance::new(wgpu::Backends::all()));
    static ref ADAPTERS: RwLock<Vec<Adapter>> = RwLock::new(Vec::new());
    static ref ADAPTER: AtomicUsize = AtomicUsize::new(0);
}

const EXCLUSIONS: &[&'static str] = &["Microsoft Basic Render Driver"];

impl WgpuWrapper {
    pub fn list_devices() -> Vec<String> {
        if ADAPTERS.read().is_empty() {
            let devices = std::panic::catch_unwind(|| -> Vec<Adapter> {
                INSTANCE.lock().enumerate_adapters(wgpu::Backends::all()).filter(|x| !EXCLUSIONS.iter().any(|e| x.get_info().name.contains(e))).collect()
            });
            match devices {
                Ok(devices) => { *ADAPTERS.write() = devices; },
                Err(e) => {
                    if let Some(s) = e.downcast_ref::<&str>() {
                        log::error!("Failed to initialize wgpu {}", s);
                    } else if let Some(s) = e.downcast_ref::<String>() {
                        log::error!("Failed to initialize wgpu {}", s);
                    } else {
                        log::error!("Failed to initialize wgpu {:?}", e);
                    }
                }
            }
        }

        ADAPTERS.read().iter().map(|x| { let x = x.get_info(); format!("{} ({:?})", x.name, x.backend) }).collect()
    }

    pub fn set_device(index: usize, _buffers: &Buffers) -> Option<()> {
        let mut i = 0;
        for a in ADAPTERS.read().iter() {
            if i == index {
                let info = a.get_info();
                log::debug!("WGPU adapter: {:?}", &info);

                ADAPTER.store(i, SeqCst);
                return Some(());
            }
            i += 1;
        }
        None
    }
    pub fn get_info() -> Option<String> {
        let lock = ADAPTERS.read();
        if let Some(ref adapter) = lock.get(ADAPTER.load(SeqCst)) {
            let info = adapter.get_info();
            Some(format!("{} ({:?})", info.name, info.backend))
        } else {
            None
        }
    }

    pub fn initialize_context() -> Option<(String, String)> {
        let instance = INSTANCE.lock();

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

        ADAPTERS.write().push(adapter);
        ADAPTER.store(ADAPTERS.read().len() - 1, SeqCst);

        Some((name, list_name))
    }

    pub fn new(params: &KernelParams, wgpu_format: (wgpu::TextureFormat, &str, f64), compute_params: &ComputeParams, buffers: &Buffers, mut drawing_len: usize) -> Option<Self> {
        let max_matrix_count = 9 * params.height as usize;

        if params.height < 4 || params.output_height < 4 || buffers.input.size.2 < 1 || params.width > 8192 || params.output_width > 8192 { return None; }

        let output_height = buffers.output.size.1 as i32;
        let output_stride = buffers.output.size.2 as i32;

        let in_size = (buffers.input.size.2 * buffers.input.size.1) as wgpu::BufferAddress;
        let out_size = (buffers.output.size.2 * buffers.output.size.1) as wgpu::BufferAddress;
        let params_size = (max_matrix_count * std::mem::size_of::<f32>()) as wgpu::BufferAddress;

        let drawing_enabled = (params.flags & 8) == 8;

        let adapter_initialized = ADAPTERS.read().get(ADAPTER.load(SeqCst)).is_some();
        if !adapter_initialized { Self::initialize_context(); }
        let lock = ADAPTERS.read();
        if let Some(adapter) = lock.get(ADAPTER.load(SeqCst)) {
            let (device, queue) = match &buffers.input.data {
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                BufferSource::Metal { command_queue, .. } |
                BufferSource::MetalBuffer { command_queue, .. } if !(*command_queue).is_null() => {
                    unsafe {
                        use foreign_types::ForeignType;
                        use wgpu_hal::api::Metal;

                        let mtl_cq = metal::CommandQueue::from_ptr(*command_queue);
                        let mtl_dev = mtl_cq.device();

                        adapter.create_device_from_hal(wgpu_hal::OpenDevice::<Metal> {
                            device: <Metal as wgpu_hal::Api>::Device::device_from_raw(mtl_dev.to_owned(), wgpu::Features::empty()),
                            queue: <Metal as wgpu_hal::Api>::Queue::queue_from_raw(mtl_cq)
                        }, &wgpu::DeviceDescriptor {
                            label: None,
                            features: wgpu::Features::empty(),
                            limits: wgpu::Limits {
                                max_storage_buffers_per_shader_stage: 4,
                                max_storage_textures_per_shader_stage: 4,
                                ..wgpu::Limits::default()
                            },
                        }, None).unwrap()
                    }
                },
                _ => {
                    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                        label: None,
                        features: wgpu::Features::empty(),
                        limits: wgpu::Limits {
                            max_storage_buffers_per_shader_stage: 4,
                            max_storage_textures_per_shader_stage: 4,
                            ..wgpu::Limits::default()
                        },
                    }, None)).ok()?
                }
            };

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

            if !drawing_enabled {
                drawing_len = 16;
                kernel = kernel.replace("bool(params.flags & 8)", "false"); // It makes it much faster for some reason
            }

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                source: wgpu::ShaderSource::Wgsl(Cow::Owned(kernel)),
                label: None
            });

            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as i32;
            let padding = (align - output_stride % align) % align;
            let padded_out_stride = output_stride + padding;
            let staging_size = padded_out_stride * output_height;

            let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor { size: staging_size as u64, usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
            let buf_matrices  = device.create_buffer(&wgpu::BufferDescriptor { size: params_size, usage: BufferUsages::STORAGE | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
            let buf_params = device.create_buffer(&wgpu::BufferDescriptor { size: std::mem::size_of::<KernelParams>() as u64, usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
            let buf_drawing = device.create_buffer(&wgpu::BufferDescriptor { size: drawing_len as u64, usage: BufferUsages::STORAGE | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });
            let buf_coeffs  = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&crate::stabilization::COEFFS), usage: wgpu::BufferUsages::STORAGE });

            let backend = adapter.get_info().backend;
            let in_texture = init_texture(&device, backend, &buffers.input, wgpu_format.0, true);
            let out_texture = init_texture(&device, backend, &buffers.output, wgpu_format.0, false);

            let sample_type = match wgpu_format.1 {
                "f32" => wgpu::TextureSampleType::Float { filterable: false },
                "u32" => wgpu::TextureSampleType::Uint,
                _ => { log::error!("Unknown texture scalar: {:?}", wgpu_format); wgpu::TextureSampleType::Float { filterable: false } }
            };

            let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<KernelParams>() as _) }, count: None },
                    wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(params_size as _) }, count: None },
                    wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                    wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new((crate::stabilization::COEFFS.len() * std::mem::size_of::<f32>()) as _) }, count: None },
                    wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(drawing_len as _) }, count: None },
                ],
                label: None,
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

            let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&pipeline_layout),
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
                        write_mask: wgpu::ColorWrites::default(),
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..wgpu::PrimitiveState::default()
                },
                multiview: None,
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
            });

            let view = in_texture.wgpu_texture.create_view(&wgpu::TextureViewDescriptor::default());

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
                out_texture,
                in_texture,
                buf_matrices,
                buf_params,
                buf_drawing,
                bind_group,
                render_pipeline,
                in_size,
                out_size,
                params_size,
                drawing_size: drawing_len as u64,
                pixel_format: wgpu_format.0,
                padded_out_stride: padded_out_stride as u32
            })
        } else {
            None
        }
    }

    pub fn undistort_image(&self, buffers: &mut Buffers, itm: &crate::stabilization::FrameTransform, drawing_buffer: &[u8]) -> bool {
        let matrices = bytemuck::cast_slice(&itm.matrices);

        let in_size = (buffers.input.size.2 * buffers.input.size.1) as u64;
        let out_size = (buffers.output.size.2 * buffers.output.size.1) as u64;
        if self.in_size  != in_size  { log::error!("Buffer size mismatch! {} vs {}", self.in_size,  in_size);  return false; }
        if self.out_size != out_size { log::error!("Buffer size mismatch! {} vs {}", self.out_size, out_size); return false; }

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        let _temp_texture = handle_input_texture(&self.device, &buffers.input, &self.queue, &mut encoder, &self.in_texture, self.pixel_format);

        if self.params_size < matrices.len() as u64    { log::error!("Buffer size mismatch! {} vs {}", self.params_size, matrices.len()); return false; }

        self.queue.write_buffer(&self.buf_matrices, 0, matrices);
        self.queue.write_buffer(&self.buf_params, 0, bytemuck::bytes_of(&itm.kernel_params));
        if !drawing_buffer.is_empty() {
            if self.drawing_size < drawing_buffer.len() as u64 { log::error!("Buffer size mismatch! {} vs {}", self.drawing_size, drawing_buffer.len()); return false; }
            self.queue.write_buffer(&self.buf_drawing, 0, drawing_buffer);
        }

        let view = self.out_texture.wgpu_texture.create_view(&wgpu::TextureViewDescriptor::default());
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

        let _temp_texture2 = handle_output_texture(&self.device, &buffers.output, &self.queue, &mut encoder, &self.out_texture, self.pixel_format, &self.staging_buffer, self.padded_out_stride);

        self.queue.submit(Some(encoder.finish()));

        match &mut buffers.output.data {
            BufferSource::Cpu { buffer, .. } => {
                let buffer_slice = self.staging_buffer.slice(..);
                let (sender, receiver) = futures_intrusive::channel::shared::oneshot_channel();
                buffer_slice.map_async(wgpu::MapMode::Read, move |v| sender.send(v).unwrap());

                self.device.poll(wgpu::Maintain::Wait);

                if let Some(Ok(())) = pollster::block_on(receiver.receive()) {
                    let data = buffer_slice.get_mapped_range();
                    if self.padded_out_stride == buffers.output.size.2 as u32 {
                        // Fast path
                        buffer.copy_from_slice(data.as_ref());
                    } else {
                        // data.as_ref()
                        //     .chunks(self.padded_out_stride as usize)
                        //     .zip(output.chunks_mut(buffers.output_size.2))
                        //     .for_each(|(src, dest)| {
                        //         dest.copy_from_slice(&src[0..buffers.output_size.2]);
                        //     });
                        use rayon::prelude::{ ParallelSliceMut, ParallelSlice };
                        use rayon::iter::{ ParallelIterator, IndexedParallelIterator };
                        data.as_ref()
                            .par_chunks(self.padded_out_stride as usize)
                            .zip(buffer.par_chunks_mut(buffers.output.size.2))
                            .for_each(|(src, dest)| {
                                dest.copy_from_slice(&src[0..buffers.output.size.2]);
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
            #[cfg(target_os = "windows")]
            BufferSource::DirectX { texture, device_context, .. } => {
                self.device.poll(wgpu::Maintain::Wait); // TODO: is this needed?

                use windows::{ Win32::Graphics::Direct3D11::*, core::Vtable };
                unsafe {
                    let device_context = ID3D11DeviceContext::from_raw_borrowed(device_context);
                    let out_texture_d3d = ID3D11Texture2D::from_raw_borrowed(texture);
                    if let Some(o) = &self.out_texture.d3d11_texture {
                        device_context.CopyResource(out_texture_d3d, o);
                    }
                }
            },
            BufferSource::Vulkan { .. } => {
                self.device.poll(wgpu::Maintain::Wait);
            },
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            BufferSource::Metal { .. } | BufferSource::MetalBuffer { .. } => {
                self.device.poll(wgpu::Maintain::Wait);
            },
            _ => { }
        }

        true
    }
}

pub fn is_buffer_supported(buffers: &Buffers) -> bool {
    match buffers.input.data {
        BufferSource::None           => false,
        BufferSource::Cpu     { .. } => true,
        BufferSource::OpenGL  { .. } => false,
        #[cfg(target_os = "windows")]
        BufferSource::DirectX { .. } => false,
        #[cfg(feature = "use-opencl")]
        BufferSource::OpenCL  { .. } => false,
        BufferSource::Vulkan  { .. } => false,
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::Metal { .. } | BufferSource::MetalBuffer { .. } => true,
    }
}
