// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#[cfg(feature = "use-opencl")]
pub mod opencl;
pub mod wgpu;

pub mod drawing;

pub struct BufferDescription<'a> {
    pub input_size:  (usize, usize, usize), // width, height, stride
    pub output_size: (usize, usize, usize), // width, height, stride

    pub input_rect:  Option<(usize, usize, usize, usize)>, // x, y, width, height
    pub output_rect: Option<(usize, usize, usize, usize)>, // x, y, width, height

    pub buffers: BufferSource<'a>
}
pub enum BufferSource<'a> {
    None,
    Cpu {
        input: &'a mut [u8],
        output: &'a mut [u8]
    },
    #[cfg(feature = "use-opencl")]
    OpenCL {
        input: ocl::ffi::cl_mem,
        output: ocl::ffi::cl_mem,
        queue: ocl::ffi::cl_command_queue
    },
    DirectX {
        input: *mut std::ffi::c_void, // ID3D11Texture2D*
        output: *mut std::ffi::c_void, // ID3D11Texture2D*
        device: *mut std::ffi::c_void, // ID3D11Device*
        device_context: *mut std::ffi::c_void, // ID3D11DeviceContext*
    },
    OpenGL {
        input: u32, // GLuint
        output: u32, // GLuint
        context: *mut std::ffi::c_void, // OpenGL context pointer
    },
    Vulkan {
        input: u64,
        output: u64,
        device: u64,
        physical_device: u64,
        instance: u64,
    }
    /*Cuda {
        input: u32,
        output: u32,
    },
    Metal {
        input: u32,
        output: u32,
    }*/
}
impl<'a> BufferSource<'a> {
    pub fn get_checksum(&self) -> u32 {
        use std::hash::Hasher;
        let mut hasher = crc32fast::Hasher::new();
        match &self {
            BufferSource::None => { }
            BufferSource::Cpu { .. } => { hasher.write_u64(1); }
            #[cfg(feature = "use-opencl")]
            BufferSource::OpenCL { queue, .. } => { hasher.write_u64(*queue as u64); }
            BufferSource::OpenGL { context, .. } => { hasher.write_u64(*context as u64); }
            BufferSource::DirectX { device, device_context, .. } => {
                hasher.write_u64(*device as u64);
                hasher.write_u64(*device_context as u64);
            },
            BufferSource::Vulkan { instance, device, physical_device, .. } => {
                hasher.write_u64(*instance);
                hasher.write_u64(*device);
                hasher.write_u64(*physical_device);
            },
        }
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
