// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

#![allow(unused_variables)]
#![allow(unused_mut)]

use crate::gpu::{ BufferDescription, BufferSource };

#[cfg(not(any(target_os = "macos", target_os = "ios")))] use { super::wgpu_interop_vulkan::*, ash::vk };
#[cfg(any(target_os = "windows", target_os = "linux"))]  use super::wgpu_interop_cuda::*;
#[cfg(any(target_os = "macos", target_os = "ios"))]      use super::wgpu_interop_metal::*;
#[cfg(target_os = "windows")]                            use { super::wgpu_interop_directx::*, windows::{ Win32::Graphics::Direct3D11::*, core::Interface } };

#[cfg(any(target_os = "macos", target_os = "ios"))]
use metal::foreign_types::ForeignTypeRef;

use wgpu::{ Origin3d, Extent3d, TextureAspect, ImageCopyTexture, ImageCopyBuffer, ImageDataLayout };

pub enum NativeTexture {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    Cuda(CudaSharedMemory),
    #[cfg(target_os = "windows")]
    D3D11(DirectX11SharedTexture),
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    Metal(metal::Texture)
}

#[derive(Default)]
pub struct TextureHolder {
    pub native_texture: Option<NativeTexture>,
    pub wgpu_texture: Option<wgpu::Texture>,
    pub wgpu_buffer: Option<wgpu::Buffer>
}

pub fn init_texture(device: &wgpu::Device, backend: wgpu::Backend, buf: &BufferDescription, format: wgpu::TextureFormat, is_in: bool) -> TextureHolder {
    let usage = if is_in {
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST
    } else {
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC
    };
    let desc = wgpu::TextureDescriptor {
        label: None,
        size: Extent3d { width: buf.size.0 as u32, height: buf.size.1 as u32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[]
    };

    match buf.data {
        BufferSource::Cpu { .. } => {
            TextureHolder {
                wgpu_texture: Some(device.create_texture(&desc)),
                wgpu_buffer: None,
                native_texture: None
            }
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::Metal { texture, .. } => {
            if backend != wgpu::Backend::Metal { panic!("Unsupported backend!"); }
            TextureHolder {
                wgpu_texture: Some(if buf.texture_copy {
                    device.create_texture(&desc)
                } else {
                    create_texture_from_metal(&device, texture, buf.size.0 as u32, buf.size.1 as u32, format, usage)
                }),
                wgpu_buffer: None,
                native_texture: None
            }
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::MetalBuffer { buffer, .. } => {
            if backend != wgpu::Backend::Metal { panic!("Unsupported backend!"); }

            let use_buffer_directly = false;
            if use_buffer_directly {
                let usage = wgpu::BufferUsages::STORAGE | if is_in { wgpu::BufferUsages::COPY_SRC } else { wgpu::BufferUsages::COPY_DST };

                TextureHolder {
                    wgpu_texture: None,
                    wgpu_buffer: Some(create_buffer_from_metal(&device, buffer, (buf.size.2 * buf.size.1) as u64, usage)),
                    native_texture: None
                }
            } else {
                let native_texture = if buf.texture_copy {
                    None
                } else {
                    if check_metal_stride(device, format, buf.size.2) {
                        let metal_usage = if is_in { metal::MTLTextureUsage::ShaderRead }
                                          else     { metal::MTLTextureUsage::RenderTarget };
                        let texture = create_metal_texture_from_buffer(buffer, buf.size.0 as u32, buf.size.1 as u32, buf.size.2 as u32, format, metal_usage);
                        if texture.as_ptr().is_null() {
                            log::error!("Failed to create Metal texture from MTLBuffer!");
                            None
                        } else {
                            Some(texture)
                        }
                    } else {
                        // TODO: handle this case
                        None
                    }
                };
                TextureHolder {
                    wgpu_texture: Some(if buf.texture_copy || native_texture.is_none() {
                        device.create_texture(&desc)
                    } else {
                        create_texture_from_metal(&device, native_texture.as_ref().unwrap().as_ptr(), buf.size.0 as u32, buf.size.1 as u32, format, usage)
                    }),
                    wgpu_buffer: None,
                    native_texture: native_texture.map(NativeTexture::Metal)
                }
            }
        },
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        BufferSource::CUDABuffer { .. } => {
            match backend {
                wgpu::Backend::Vulkan => {
                    let use_buffer_directly = true;
                    if use_buffer_directly {
                        let (buffer, mem) = super::wgpu_interop_cuda::create_vk_buffer_backed_by_cuda_memory(&device, buf.size).unwrap(); // TODO: unwrap
                        TextureHolder {
                            wgpu_texture: None,
                            wgpu_buffer: Some(create_buffer_from_vk_buffer(&device, buffer, (buf.size.2 * buf.size.1) as u64, is_in)),
                            native_texture: Some(NativeTexture::Cuda(mem))
                        }
                    } else {
                        let (image, mem) = super::wgpu_interop_cuda::create_vk_image_backed_by_cuda_memory(&device, buf.size, format).unwrap(); // TODO: unwrap
                        TextureHolder {
                            wgpu_texture: Some(create_texture_from_vk_image(&device, image, desc.size.width, desc.size.height, format, is_in, true)),
                            wgpu_buffer: None,
                            native_texture: Some(NativeTexture::Cuda(mem))
                        }
                    }
                },
                #[cfg(target_os = "windows")]
                wgpu::Backend::Dx12 => {
                    let use_buffer_directly = false;
                    let size = if use_buffer_directly {
                        (buf.size.2 * buf.size.1) as usize
                    } else {
                        fn align(a: usize, b: usize) -> usize { ((a + b - 1) / b) * b }
                        buf.size.1 * align(buf.size.2, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
                    };
                    let (dx12_texture, shared_handle, mut actual_size) = create_native_shared_buffer_dx12(device, size).unwrap(); // TODO: unwrap
                    if use_buffer_directly {
                        actual_size = size;
                    }

                    unsafe {
                        let buffer = <wgpu::hal::api::Dx12 as wgpu::hal::Api>::Device::buffer_from_raw(dx12_texture, actual_size as u64);

                        let wgpu_buffer = device.create_buffer_from_hal::<wgpu::hal::api::Dx12>(buffer, &wgpu::BufferDescriptor {
                            label: None,
                            size: actual_size as u64,
                            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                            mapped_at_creation: false
                        });

                        let cuda_mem = import_external_d3d12_resource(shared_handle as *mut _, actual_size as usize).unwrap(); // TODO: unwrap

                        let _ = windows::Win32::Foundation::CloseHandle(windows::Win32::Foundation::HANDLE(shared_handle as *mut _));

                        TextureHolder {
                            wgpu_texture: if use_buffer_directly { None } else { Some(device.create_texture(&desc)) },
                            wgpu_buffer: Some(wgpu_buffer),
                            native_texture: Some(NativeTexture::Cuda(cuda_mem))
                        }
                    }
                },
                // TODO: Gl backend
                _ => {
                    panic!("Unsupported backend!");
                }
            }
        },
        #[cfg(target_os = "windows")]
        BufferSource::DirectX11 { texture, device: d3d11_device, device_context } => {
            unsafe {
                let d3d11_device = ID3D11Device::from_raw_borrowed(&d3d11_device).unwrap(); // TODO: unwrap
                // let device_context = ID3D11DeviceContext::from_raw_borrowed(device_context);
                let texture = ID3D11Texture2D::from_raw_borrowed(&texture).unwrap(); // TODO: unwrap
                let mut desc11 = D3D11_TEXTURE2D_DESC::default();
                texture.GetDesc(&mut desc11);
                let conv_format = format_dxgi_to_wgpu(desc11.Format);
                assert_eq!(format, conv_format);
                assert_eq!(desc11.Width, buf.size.0 as u32);
                assert_eq!(desc11.Height, buf.size.1 as u32);

                match backend {
                    wgpu::Backend::Vulkan => {
                        let (image, dx_tex) = create_vk_image_from_d3d11_texture(&device, d3d11_device, texture).unwrap(); // TODO: unwrap

                        TextureHolder {
                            wgpu_texture: Some(create_texture_from_vk_image(&device, image, desc11.Width, desc11.Height, conv_format, is_in, true)),
                            wgpu_buffer: None,
                            native_texture: dx_tex.map(|x| NativeTexture::D3D11(x))
                        }
                    },
                    wgpu::Backend::Dx12 => {
                        let (d3d12_resource, dx11_tex) = create_dx12_resource_from_d3d11_texture(&device, d3d11_device, texture).unwrap(); // TODO: unwrap

                        TextureHolder {
                            wgpu_texture: Some(create_texture_from_dx12_resource(&device, d3d12_resource, &desc)),
                            wgpu_buffer: None,
                            native_texture: dx11_tex.map(|x| NativeTexture::D3D11(x))
                        }
                    },
                    /*wgpu::Backend::Gl => {
                        // TODO
                    },*/
                    _ => {
                        panic!("Unsupported backend!")
                    }
                }
            }
        },
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        BufferSource::Vulkan { texture, device: vk_device, instance, .. } => {
            use ash::vk::Handle;

            match backend {
                wgpu::Backend::Vulkan => {
                    TextureHolder {
                        wgpu_texture: Some(if buf.texture_copy {
                            device.create_texture(&desc)
                        } else {
                            create_texture_from_vk_image(&device, vk::Image::from_raw(texture), buf.size.0 as u32, buf.size.1 as u32, format, is_in, false)
                        }),
                        wgpu_buffer: None,
                        native_texture: None
                    }
                },
                /*wgpu::Backend::Dx12 => {
                    let vk_image = vk::Image::from_raw(texture);
                    let vk_device = vk::Device::from_raw(vk_device);
                    let vk_instance = vk::Instance::from_raw(instance);
                    let d3d12_resource = create_dx12_resource_from_vk_image(&device, vk_image.clone(), vk_device.clone(), vk_instance.clone()).unwrap(); // TODO: unwrap
                    std::mem::forget(vk_image);
                    std::mem::forget(vk_device);
                    std::mem::forget(vk_instance);
                    TextureHolder {
                        wgpu_texture: Some(if buf.texture_copy {
                            device.create_texture(&desc)
                        } else {
                            create_texture_from_dx12_resource(&device, d3d12_resource, &desc)
                        }),
                        wgpu_buffer: None,
                        native_texture: None
                    }
                },*/
                /*wgpu::Backend::Gl => {
                    // TODO
                },*/
                _ => {
                    panic!("Unsupported backend!")
                }
            }
        },
        _ => {
            panic!("Unsupported buffer {:?}", buf.data);
        }
    }
}

pub fn handle_input_texture(device: &wgpu::Device, buf: &BufferDescription, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, in_texture: &TextureHolder, format: wgpu::TextureFormat, padded_stride: u32) -> Option<wgpu::Texture> {
    let mut temp_texture = None;

    let size = Extent3d { width: buf.size.0 as u32, height: buf.size.1 as u32, depth_or_array_layers: 1 };

    match &buf.data {
        BufferSource::Cpu { buffer } => {
            queue.write_texture(
                in_texture.wgpu_texture.as_ref().unwrap().as_image_copy(),
                bytemuck::cast_slice(buffer),
                ImageDataLayout { offset: 0, bytes_per_row: Some(buf.size.2 as u32), rows_per_image: None },
                size,
            );
        },
        #[cfg(target_os = "windows")]
        BufferSource::DirectX11 { texture, device_context, .. } => {
            unsafe {
                if let Some(NativeTexture::D3D11(i)) = &in_texture.native_texture {
                    i.synchronized_copy_from(ID3D11DeviceContext::from_raw_borrowed(device_context).unwrap(), ID3D11Texture2D::from_raw_borrowed(texture).unwrap()).unwrap(); // TODO: unwrap
                }
            }
        },
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        BufferSource::CUDABuffer { buffer } => {
            super::wgpu_interop_cuda::cuda_synchronize();
            if let Some(NativeTexture::Cuda(cuda_mem)) = &in_texture.native_texture {
                super::wgpu_interop_cuda::cuda_2d_copy_on_device(buf.size, cuda_mem.device_ptr, *buffer as CUdeviceptr, cuda_mem.vulkan_pitch_alignment, 1);
                super::wgpu_interop_cuda::cuda_synchronize();
            }
            if let Some(in_buf) = &in_texture.wgpu_buffer {
                if let Some(in_tex) = &in_texture.wgpu_texture {
                    encoder.copy_buffer_to_texture(
                        ImageCopyBuffer { buffer: in_buf, layout: ImageDataLayout { offset: 0, bytes_per_row: Some(padded_stride), rows_per_image: None } },
                        ImageCopyTexture { texture: in_tex, mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                        size
                    );
                }
            }
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::Metal { texture, .. } => {
            if buf.texture_copy {
                temp_texture = Some(create_texture_from_metal(device, *texture as *mut metal::MTLTexture, buf.size.0 as u32, buf.size.1 as u32, format, wgpu::TextureUsages::COPY_SRC));

                encoder.copy_texture_to_texture(
                    ImageCopyTexture { texture: temp_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                    ImageCopyTexture { texture: in_texture.wgpu_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                    size
                );
            }
        },
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        BufferSource::Vulkan { texture, .. } => {
            if buf.texture_copy {
                use ash::vk::Handle;
                temp_texture = Some(create_texture_from_vk_image(&device, vk::Image::from_raw(*texture), buf.size.0 as u32, buf.size.1 as u32, format, true, false));

                encoder.copy_texture_to_texture(
                    ImageCopyTexture { texture: temp_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                    ImageCopyTexture { texture: in_texture.wgpu_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                    size
                );
            }
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::MetalBuffer { .. } => {
            if buf.texture_copy {
                if let Some(NativeTexture::Metal(mtl_texture)) = &in_texture.native_texture {
                    if !mtl_texture.as_ptr().is_null() {
                        temp_texture = Some(create_texture_from_metal(device, mtl_texture.as_ptr(), buf.size.0 as u32, buf.size.1 as u32, format, wgpu::TextureUsages::COPY_SRC));

                        encoder.copy_texture_to_texture(
                            ImageCopyTexture { texture: temp_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                            ImageCopyTexture { texture: in_texture.wgpu_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                            size
                        );
                    }
                }
            }
        },
        _ => { }
    }

    temp_texture
}

pub fn handle_output_texture(device: &wgpu::Device, buf: &BufferDescription, _queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, out_texture: &TextureHolder, format: wgpu::TextureFormat, staging_buffer: &wgpu::Buffer, padded_stride: u32) -> Option<wgpu::Texture> {
    let mut temp_texture = None;

    let size = Extent3d { width: buf.size.0 as u32, height: buf.size.1 as u32, depth_or_array_layers: 1 };

    match &buf.data {
        BufferSource::Cpu { .. } => {
            encoder.copy_texture_to_buffer(
                ImageCopyTexture { texture: out_texture.wgpu_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                ImageCopyBuffer { buffer: staging_buffer, layout: ImageDataLayout { offset: 0, bytes_per_row: Some(padded_stride), rows_per_image: None } },
                size
            );
        },
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        BufferSource::CUDABuffer { buffer } => {
            if let Some(out_buf) = &out_texture.wgpu_buffer {
                if let Some(out_tex) = &out_texture.wgpu_texture {
                    encoder.copy_texture_to_buffer(
                        ImageCopyTexture { texture: out_tex, mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                        ImageCopyBuffer { buffer: out_buf, layout: ImageDataLayout { offset: 0, bytes_per_row: Some(padded_stride), rows_per_image: None } },
                        size
                    );
                }
            }
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::Metal { texture, .. } => {
            if buf.texture_copy {
                temp_texture = Some(create_texture_from_metal(&device, *texture as *mut metal::MTLTexture, buf.size.0 as u32, buf.size.1 as u32, format, wgpu::TextureUsages::COPY_DST));

                encoder.copy_texture_to_texture(
                    ImageCopyTexture { texture: out_texture.wgpu_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                    ImageCopyTexture { texture: temp_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                    size
                );
            }
        },
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        BufferSource::Vulkan { texture, .. } => {
            if buf.texture_copy {
                use ash::vk::Handle;
                temp_texture = Some(create_texture_from_vk_image(&device, vk::Image::from_raw(*texture), buf.size.0 as u32, buf.size.1 as u32, format, false, false));

                encoder.copy_texture_to_texture(
                    ImageCopyTexture { texture: out_texture.wgpu_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                    ImageCopyTexture { texture: temp_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                    size
                );
            }
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::MetalBuffer { .. } => {
            if buf.texture_copy {
                if let Some(NativeTexture::Metal(mtl_texture)) = &out_texture.native_texture {
                    if !mtl_texture.as_ptr().is_null() {
                        temp_texture = Some(create_texture_from_metal(device, mtl_texture.as_ptr(), buf.size.0 as u32, buf.size.1 as u32, format, wgpu::TextureUsages::COPY_DST));

                        encoder.copy_texture_to_texture(
                            ImageCopyTexture { texture: out_texture.wgpu_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                            ImageCopyTexture { texture: temp_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                            size
                        );
                    }
                }
            }
        }
        _ => { }
    }

    temp_texture
}

pub fn handle_output_texture_post(device: &wgpu::Device, buf: &BufferDescription, out_texture: &TextureHolder, format: wgpu::TextureFormat, sub_index: wgpu::SubmissionIndex) {
    match &buf.data {
        #[cfg(target_os = "windows")]
        BufferSource::DirectX11 { texture, device_context, .. } => {
            device.poll(wgpu::Maintain::WaitForSubmissionIndex(sub_index));

            unsafe {
                use windows::Win32::Graphics::Direct3D11::*;
                if let Some(NativeTexture::D3D11(o)) = &out_texture.native_texture {
                    o.synchronized_copy_to(ID3D11DeviceContext::from_raw_borrowed(device_context).unwrap(), ID3D11Texture2D::from_raw_borrowed(texture).unwrap()).unwrap(); // TODO: unwrap
                }
            }
        },
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        BufferSource::CUDABuffer { buffer } => {
            device.poll(wgpu::Maintain::WaitForSubmissionIndex(sub_index));
            if let Some(NativeTexture::Cuda(cuda_mem)) = &out_texture.native_texture {
                super::wgpu_interop_cuda::cuda_2d_copy_on_device(buf.size, *buffer as CUdeviceptr, cuda_mem.device_ptr, 1, cuda_mem.vulkan_pitch_alignment);
            }
            super::wgpu_interop_cuda::cuda_synchronize();
        },
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        BufferSource::Vulkan { .. } => {
            device.poll(wgpu::Maintain::WaitForSubmissionIndex(sub_index));
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::Metal { .. } | BufferSource::MetalBuffer { .. } => {
            device.poll(wgpu::Maintain::WaitForSubmissionIndex(sub_index));
        },
        _ => { }
    }
}
