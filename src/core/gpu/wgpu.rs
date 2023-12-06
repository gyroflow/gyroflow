// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::borrow::Cow;
use std::sync::atomic::{ AtomicUsize, Ordering::SeqCst };
use wgpu::Adapter;
use wgpu::BufferUsages;
use wgpu::util::DeviceExt;
use parking_lot::{ RwLock, Mutex };
use crate::gpu:: { Buffers, BufferSource };
use crate::stabilization::KernelParams;
use crate::stabilization::distortion_models::DistortionModel;
use super::wgpu_interop::*;

#[derive(Debug)]
pub enum WgpuError {
    RequestDevice(wgpu::RequestDeviceError),
    ParamCheck,
    NoAvailableAdapter,
}

enum PipelineType {
    None,
    Render(wgpu::RenderPipeline),
    Compute(wgpu::ComputePipeline)
}

pub struct WgpuWrapper  {
    staging_buffer: wgpu::Buffer,
    buf_matrices: wgpu::Buffer,
    buf_params: wgpu::Buffer,
    buf_lens_data: wgpu::Buffer,
    buf_drawing: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
    pipeline: PipelineType,
    pixel_format: wgpu::TextureFormat,

    in_texture: TextureHolder,
    out_texture: TextureHolder,

    queue: wgpu::Queue,
    pub device: wgpu::Device,

    padded_out_stride: u32,
    in_size: u64,
    out_size: u64,
    params_size: u64,
    drawing_size: u64,
}
impl Drop for WgpuWrapper {
    fn drop(&mut self) {
        // We need to delete all texture references and then call device.poll() to actually release them properly
        self.bind_group = None;
        self.pipeline = PipelineType::None;
        self.in_texture = TextureHolder::default();
        self.out_texture = TextureHolder::default();

        self.device.poll(wgpu::Maintain::Wait);
    }
}

lazy_static::lazy_static! {
    static ref INSTANCE: Mutex<wgpu::Instance> = Mutex::new(wgpu::Instance::new(wgpu::InstanceDescriptor::default()));
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

    pub fn set_device(index: usize) -> Option<()> {
        let mut i = 0;
        for a in ADAPTERS.read().iter() {
            if i == index {
                let info = a.get_info();
                log::debug!("WGPU adapter ({i}): {:?}", &info);

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

    pub fn new(params: &KernelParams, wgpu_format: (wgpu::TextureFormat, &str, bool), distortion_model: DistortionModel, digital_lens: Option<DistortionModel>, buffers: &Buffers, mut drawing_len: usize) -> Result<Self, WgpuError> {
        let max_matrix_count = 12 * if (params.flags & 16) == 16 { params.width } else { params.height } as usize;

        if params.height < 4 || params.output_height < 4 || buffers.input.size.0 < 16 || buffers.input.size.2 < 16 || buffers.output.size.0 < 16 || buffers.output.size.2 < 16 || params.width > 16384 || params.output_width > 16384 {
            return Err(WgpuError::ParamCheck);
        }

        let output_height = buffers.output.size.1 as i32;
        let output_stride = buffers.output.size.2 as i32;

        let in_size = (buffers.input.size.2 * buffers.input.size.1) as wgpu::BufferAddress;
        let out_size = (buffers.output.size.2 * buffers.output.size.1) as wgpu::BufferAddress;
        let params_size = (max_matrix_count * std::mem::size_of::<f32>()) as wgpu::BufferAddress;

        let drawing_enabled = (params.flags & 8) == 8;

        let mut adapter_id = ADAPTER.load(SeqCst);
        let adapter_initialized = ADAPTERS.read().get(adapter_id).is_some();
        if !adapter_initialized { Self::initialize_context(); }
        let lock = ADAPTERS.read();
        adapter_id = ADAPTER.load(SeqCst);

        #[cfg(any(target_os = "windows", target_os = "linux"))]
        if let BufferSource::CUDABuffer { .. } = buffers.input.data {
            adapter_id = super::wgpu_interop_cuda::get_current_cuda_device() as usize;
        }

        if let Some(adapter) = lock.get(adapter_id) {
            log::debug!("WGPU initializing adapter #{adapter_id}");
            let (device, queue) = match &buffers.input.data {
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                BufferSource::Metal { command_queue, .. } |
                BufferSource::MetalBuffer { command_queue, .. } if !(*command_queue).is_null() => {
                    unsafe {
                        use metal::foreign_types::ForeignType;
                        use wgpu_hal::api::Metal;

                        let mtl_cq = metal::CommandQueue::from_ptr(*command_queue);
                        let mtl_dev = mtl_cq.device();

                        adapter.create_device_from_hal(wgpu_hal::OpenDevice::<Metal> {
                            device: <Metal as wgpu_hal::Api>::Device::device_from_raw(mtl_dev.to_owned(), wgpu::Features::empty()),
                            queue: <Metal as wgpu_hal::Api>::Queue::queue_from_raw(mtl_cq, 1.0)
                        }, &wgpu::DeviceDescriptor {
                            label: None,
                            required_features: wgpu::Features::empty(),
                            required_limits: wgpu::Limits {
                                max_storage_buffers_per_shader_stage: 6,
                                max_storage_textures_per_shader_stage: 4,
                                max_buffer_size: (1 << 31) - 1,
                                max_storage_buffer_binding_size: (1 << 31) - 1,
                                ..wgpu::Limits::default()
                            },
                        }, None).map_err(|e| WgpuError::RequestDevice(e))?
                    }
                },
                _ => {
                    let max_buffer_bits = if cfg!(any(target_os = "android", target_os = "ios")) { 29 } else { 31 };
                    let max_storage_buffer_bits = if cfg!(any(target_os = "android", target_os = "ios")) { 27 } else { 31 };
                    let adapter_limits = adapter.limits();
                    let mut limits = wgpu::Limits {
                        max_storage_buffers_per_shader_stage: 6.min(adapter_limits.max_storage_buffers_per_shader_stage),
                        max_storage_textures_per_shader_stage: 4.min(adapter_limits.max_storage_textures_per_shader_stage),
                        max_buffer_size: ((1 << max_buffer_bits) - 1).min(adapter_limits.max_buffer_size),
                        max_storage_buffer_binding_size: ((1 << max_storage_buffer_bits) - 1+5).min(adapter_limits.max_storage_buffer_binding_size),
                        ..wgpu::Limits::default()
                    };
                    let mut result = Err(WgpuError::NoAvailableAdapter);
                    for _ in 0..4 {
                        let device = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                            label: None,
                            required_features: wgpu::Features::empty(),
                            required_limits: limits.clone(),
                        }, None));
                        if let Err(e) = &device {
                            let e_str = format!("{e:?}");
                            let re = regex::Regex::new("FailedLimit \\{ name: \"(.*?)\", requested: [0-9]+, allowed: ([0-9]+)").unwrap();
                            if let Some(captures) = re.captures(&e_str) {
                                log::debug!("Catching wgpu limit error: {e_str}");
                                let (_, [name, allowed]) = captures.extract();
                                match name {
                                    "max_storage_buffers_per_shader_stage"  => { limits.max_storage_buffers_per_shader_stage  = allowed.parse().unwrap(); continue; },
                                    "max_storage_textures_per_shader_stage" => { limits.max_storage_textures_per_shader_stage = allowed.parse().unwrap(); continue; },
                                    "max_buffer_size"                       => { limits.max_buffer_size                       = allowed.parse().unwrap(); continue; },
                                    "max_storage_buffer_binding_size"       => { limits.max_storage_buffer_binding_size       = allowed.parse().unwrap(); continue; },
                                    _ => { }
                                }
                            }
                        }
                        result = device.map_err(|e| WgpuError::RequestDevice(e));
                        break;
                    }
                    result?
                }
            };

            device.on_uncaptured_error(Box::new(|e| {
                log::error!("Uncaptured device error: {e:?}");
            }));

            let mut kernel = include_str!("wgpu_undistort.wgsl").to_string();
            //let mut kernel = std::fs::read_to_string("D:/programowanie/projekty/Rust/gyroflow/src/core/gpu/wgpu_undistort.wgsl").unwrap();

            let mut lens_model_functions = distortion_model.wgsl_functions().to_string();
            let default_digital_lens = "fn digital_undistort_point(uv: vec2<f32>) -> vec2<f32> { return uv; }
                                            fn digital_distort_point(uv: vec2<f32>) -> vec2<f32> { return uv; }";
            lens_model_functions.push_str(digital_lens.as_ref().map(|x| x.wgsl_functions()).unwrap_or(default_digital_lens));
            kernel = kernel.replace("LENS_MODEL_FUNCTIONS;", &lens_model_functions);
            kernel = kernel.replace("SCALAR", wgpu_format.1);
            // Replace it in source to allow for loop unrolling when compiling shader
            kernel = kernel.replace("params.interpolation", &format!("{}u", params.interpolation));

            let lens_data_len = 16; // TODO

            if !drawing_enabled {
                drawing_len = 16;
            }
            kernel = kernel.replace("params.pix_element_count >= 1", &format!("{}", params.pix_element_count >= 1));
            kernel = kernel.replace("params.pix_element_count >= 2", &format!("{}", params.pix_element_count >= 2));
            kernel = kernel.replace("params.pix_element_count >= 3", &format!("{}", params.pix_element_count >= 3));
            kernel = kernel.replace("params.pix_element_count >= 4", &format!("{}", params.pix_element_count >= 4));
            kernel = kernel.replace("bool(params.flags & 1)", &format!("{}", (params.flags & 1) > 0)); // fix_range
            kernel = kernel.replace("bool(params.flags & 2)", &format!("{}", (params.flags & 2) > 0)); // has_digital_lens
            kernel = kernel.replace("bool(params.flags & 8)", &format!("{}", (params.flags & 8) > 0)); // has_drawing

            let backend = adapter.get_info().backend;
            let in_texture = init_texture(&device, backend, &buffers.input, wgpu_format.0, true);
            let out_texture = init_texture(&device, backend, &buffers.output, wgpu_format.0, false);

            let uses_textures = in_texture.wgpu_texture.is_some();
            if uses_textures {
                while let Some(pos) = kernel.find("{buffer_input}") {
                    kernel.replace_range(pos..kernel.find("{/buffer_input}").unwrap() + 15, "");
                }
            } else {
                while let Some(pos) = kernel.find("{texture_input}") {
                    kernel.replace_range(pos..kernel.find("{/texture_input}").unwrap() + 16, "");
                }
            }
            // log::info!("Using kernel: {kernel}");

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
            let buf_lens_data = device.create_buffer(&wgpu::BufferDescriptor { size: drawing_len as u64, usage: BufferUsages::STORAGE | BufferUsages::COPY_DST, label: None, mapped_at_creation: false });

            let bind_group_layout = if uses_textures {
                let sample_type = match wgpu_format.1 {
                    "f32" => wgpu::TextureSampleType::Float { filterable: false },
                    "u32" => wgpu::TextureSampleType::Uint,
                    _ => { log::error!("Unknown texture scalar: {:?}", wgpu_format); wgpu::TextureSampleType::Float { filterable: false } }
                };
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    entries: &[
                        wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<KernelParams>() as _) }, count: None },
                        wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(params_size as _) }, count: None },
                        wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new((crate::stabilization::COEFFS.len() * std::mem::size_of::<f32>()) as _) }, count: None },
                        wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(lens_data_len as _) }, count: None },
                        wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(drawing_len as _) }, count: None },
                        wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                    ],
                    label: None,
                })
            } else {
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    entries: &[
                        wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<KernelParams>() as _) }, count: None },
                        wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(params_size as _) }, count: None },
                        wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new((crate::stabilization::COEFFS.len() * std::mem::size_of::<f32>()) as _) }, count: None },
                        wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(lens_data_len as _) }, count: None },
                        wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(drawing_len as _) }, count: None },
                        wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(in_size as _) }, count: None },
                        wgpu::BindGroupLayoutEntry { binding: 6, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: wgpu::BufferSize::new(out_size as _) }, count: None },
                    ],
                    label: None,
                })
            };

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

            let pipeline = if uses_textures {
                PipelineType::Render(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                }))
            } else {
                PipelineType::Compute(device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    module: &shader,
                    entry_point: "undistort_compute",
                    label: None,
                    layout: Some(&pipeline_layout),
                }))
            };

            let bind_group = match &pipeline {
                PipelineType::None => None,
                PipelineType::Render(p) => {
                    Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout: &p.get_bind_group_layout(0),
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: buf_params.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: buf_matrices.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 2, resource: buf_coeffs.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 3, resource: buf_lens_data.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 4, resource: buf_drawing.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&in_texture.wgpu_texture.as_ref().unwrap().create_view(&wgpu::TextureViewDescriptor::default())) },
                        ],
                    }))
                },
                PipelineType::Compute(p) => {
                    Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout: &p.get_bind_group_layout(0),
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: buf_params.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 1, resource: buf_matrices.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 2, resource: buf_coeffs.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 3, resource: buf_lens_data.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 4, resource: buf_drawing.as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 5, resource: in_texture.wgpu_buffer.as_ref().unwrap().as_entire_binding() },
                            wgpu::BindGroupEntry { binding: 6, resource: out_texture.wgpu_buffer.as_ref().unwrap().as_entire_binding() },
                        ],
                    }))
                }
            };

            Ok(Self {
                device,
                queue,
                staging_buffer,
                out_texture,
                in_texture,
                buf_matrices,
                buf_params,
                buf_drawing,
                buf_lens_data,
                bind_group,
                pipeline,
                in_size,
                out_size,
                params_size,
                drawing_size: drawing_len as u64,
                pixel_format: wgpu_format.0,
                padded_out_stride: padded_out_stride as u32
            })
        } else {
            Err(WgpuError::NoAvailableAdapter)
        }
    }

    pub fn undistort_image(&self, buffers: &mut Buffers, itm: &crate::stabilization::FrameTransform, drawing_buffer: &[u8]) -> bool {
        let matrices = bytemuck::cast_slice(&itm.matrices);

        let in_size = (buffers.input.size.2 * buffers.input.size.1) as u64;
        let out_size = (buffers.output.size.2 * buffers.output.size.1) as u64;
        if self.in_size  != in_size  { log::error!("Buffer size mismatch! {} vs {}", self.in_size,  in_size);  return false; }
        if self.out_size != out_size { log::error!("Buffer size mismatch! {} vs {}", self.out_size, out_size); return false; }

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        let _temp_texture = handle_input_texture(&self.device, &buffers.input, &self.queue, &mut encoder, &self.in_texture, self.pixel_format, self.padded_out_stride);

        if self.params_size < matrices.len() as u64    { log::error!("Buffer size mismatch! {} vs {}", self.params_size, matrices.len()); return false; }

        self.queue.write_buffer(&self.buf_matrices, 0, matrices);
        self.queue.write_buffer(&self.buf_params, 0, bytemuck::bytes_of(&itm.kernel_params));
        if !drawing_buffer.is_empty() {
            if self.drawing_size < drawing_buffer.len() as u64 { log::error!("Buffer size mismatch! {} vs {}", self.drawing_size, drawing_buffer.len()); return false; }
            self.queue.write_buffer(&self.buf_drawing, 0, drawing_buffer);
        }

        match &self.pipeline {
            PipelineType::None => { },
            PipelineType::Compute(p) => {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
                cpass.set_pipeline(p);
                cpass.set_bind_group(0, self.bind_group.as_ref().unwrap(), &[]);
                cpass.dispatch_workgroups((buffers.output.size.0 as f32 / 8.0).ceil() as u32, (buffers.output.size.1 as f32 / 8.0).ceil() as u32, 1);
            },
            PipelineType::Render(p) => {
                let view = self.out_texture.wgpu_texture.as_ref().unwrap().create_view(&wgpu::TextureViewDescriptor::default());
                let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                });
                rpass.set_pipeline(p);
                rpass.set_bind_group(0, self.bind_group.as_ref().unwrap(), &[]);
                rpass.draw(0..6, 0..1);
            }
        }

        let _temp_texture2 = handle_output_texture(&self.device, &buffers.output, &self.queue, &mut encoder, &self.out_texture, self.pixel_format, &self.staging_buffer, self.padded_out_stride);

        let sub_index = self.queue.submit(Some(encoder.finish()));

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
                        (&mut buffer[..buffers.output.size.1 * buffers.output.size.2]).copy_from_slice(data.as_ref());
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
            _ => { handle_output_texture_post(&self.device, &buffers.output, &self.out_texture, self.pixel_format, sub_index); }
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
        BufferSource::DirectX11 { .. } => true,
        #[cfg(feature = "use-opencl")]
        BufferSource::OpenCL  { .. } => false,
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        BufferSource::Vulkan  { .. } => true,
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        BufferSource::CUDABuffer{ .. } => true,
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::Metal { .. } | BufferSource::MetalBuffer { .. } => true,
    }
}
