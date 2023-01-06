// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#[cfg(feature = "use-opencl")]
pub mod opencl;
pub mod wgpu;

pub mod wgpu_interop;
#[cfg(not(any(target_os = "macos", target_os = "ios")))] pub mod wgpu_interop_vulkan;
#[cfg(any(target_os = "macos", target_os = "ios"))]      pub mod wgpu_interop_metal;
#[cfg(target_os = "windows")]                            pub mod wgpu_interop_directx;
#[cfg(any(target_os = "windows", target_os = "linux"))]  pub mod wgpu_interop_cuda;

pub mod drawing;
use std::hash::Hasher;

#[derive(Default)]
pub struct BufferDescription<'a> {
    pub size: (usize, usize, usize), // width, height, stride
    pub rect: Option<(usize, usize, usize, usize)>, // x, y, width, height
    pub data: BufferSource<'a>,
    pub texture_copy: bool
}
pub struct Buffers<'a> {
    pub input: BufferDescription<'a>,
    pub output: BufferDescription<'a>
}

#[derive(Debug, Default)]
pub enum BufferSource<'a> {
    #[default]
    None,
    Cpu { buffer: &'a mut [u8] },
    #[cfg(feature = "use-opencl")]
    OpenCL {
        texture: ocl::ffi::cl_mem,
        queue: ocl::ffi::cl_command_queue
    },
    #[cfg(target_os = "windows")]
    DirectX {
        texture: *mut std::ffi::c_void, // ID3D11Texture2D*
        device: *mut std::ffi::c_void, // ID3D11Device*
        device_context: *mut std::ffi::c_void, // ID3D11DeviceContext*
    },
    OpenGL {
        texture: u32, // GLuint
        context: *mut std::ffi::c_void, // OpenGL context pointer
    },
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    Vulkan {
        texture: u64,
        device: u64,
        physical_device: u64,
        instance: u64,
    },
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    Metal {
        texture: *mut metal::MTLTexture,
        command_queue: *mut metal::MTLCommandQueue,
    },
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    MetalBuffer {
        buffer: *mut metal::MTLBuffer,
        command_queue: *mut metal::MTLCommandQueue,
    },
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    CUDABuffer {
        buffer: *mut std::ffi::c_void // Cudeviceptr
    },
}
impl<'a> BufferDescription<'a> {
    pub fn get_checksum(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        hasher.write_usize(self.size.0);
        hasher.write_usize(self.size.1);
        hasher.write_usize(self.size.2);
        if let Some(r) = self.rect {
            hasher.write_usize(r.0);
            hasher.write_usize(r.1);
            hasher.write_usize(r.2);
            hasher.write_usize(r.3);
        }
        match &self.data {
            BufferSource::None => { }
            BufferSource::Cpu { .. } => { }
            #[cfg(feature = "use-opencl")]
            BufferSource::OpenCL { texture, queue } => {
                if !self.texture_copy {
                    hasher.write_u64(*texture as u64);
                }
                hasher.write_u64(*queue as u64);
            }
            BufferSource::OpenGL { texture, context } => {
                if !self.texture_copy {
                    hasher.write_u32(*texture);
                }
                hasher.write_u64(*context as u64);
            }
            #[cfg(target_os = "windows")]
            BufferSource::DirectX { texture, device, device_context } => {
                if !self.texture_copy {
                    hasher.write_u64(*texture as u64);
                }
                hasher.write_u64(*device as u64);
                hasher.write_u64(*device_context as u64);
            },
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            BufferSource::Vulkan { texture, instance, device, physical_device } => {
                if !self.texture_copy {
                    hasher.write_u64(*texture);
                }
                hasher.write_u64(*instance);
                hasher.write_u64(*device);
                hasher.write_u64(*physical_device);
            },
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            BufferSource::CUDABuffer { buffer } => {
                if !self.texture_copy {
                    hasher.write_u64(*buffer as u64);
                }
            },
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            BufferSource::Metal { texture, command_queue } => {
                if !self.texture_copy {
                    hasher.write_u64(*texture as u64);
                }
                hasher.write_u64(*command_queue as u64);
            },
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            BufferSource::MetalBuffer { buffer, command_queue } => {
                if !self.texture_copy {
                    hasher.write_u64(*buffer as u64);
                }
                hasher.write_u64(*command_queue as u64);
            },
        }
        hasher.finalize()
    }
}
impl<'a> Buffers<'a> {
    pub fn get_checksum(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        hasher.write_u32(self.input.get_checksum());
        hasher.write_u32(self.output.get_checksum());
        hasher.finalize()
    }
}

pub fn initialize_contexts() -> Option<(String, String)> {
    #[cfg(feature = "use-opencl")]
    if std::env::var("NO_OPENCL").unwrap_or_default().is_empty() {
        let cl = std::panic::catch_unwind(|| {
            opencl::OclWrapper::initialize_context(None)
        });
        match cl {
            Ok(Ok(names)) => { return Some(names); },
            Ok(Err(e)) => { log::error!("OpenCL error init: {:?}", e); },
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
    }

    if std::env::var("NO_WGPU").unwrap_or_default().is_empty() {
        let wgpu = std::panic::catch_unwind(|| {
            wgpu::WgpuWrapper::initialize_context()
        });
        match wgpu {
            Ok(Some(names)) => { return Some(names); },
            Ok(None) => { log::error!("wgpu init error"); },
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

    None
}
