// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

#![allow(unused_variables)]
#![allow(unused_mut)]

use crate::gpu::{ BufferDescription, BufferSource };

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
use { super::wgpu_interop_vulkan::*, ash::vk };
#[cfg(any(target_os = "macos", target_os = "ios"))]
use super::wgpu_interop_metal::*;
#[cfg(target_os = "windows")]
use { super::wgpu_interop_directx::*, windows::{ Win32::Graphics::Direct3D11::*, core::Vtable } };

#[cfg(any(target_os = "macos", target_os = "ios"))]
use foreign_types::ForeignTypeRef;

use std::num::NonZeroU32;
use wgpu::{ Origin3d, Extent3d, TextureAspect, ImageCopyTexture, ImageCopyBuffer, ImageDataLayout };

pub struct TextureHolder  {
    pub wgpu_texture: wgpu::Texture,

    #[cfg(target_os = "windows")]
    pub d3d11_texture: Option<ID3D11Texture2D>,

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    pub metal_texture: Option<metal::Texture>,
}

pub fn init_texture(device: &wgpu::Device, backend: wgpu::Backend, buf: &BufferDescription, format: wgpu::TextureFormat, is_in: bool) -> TextureHolder {
    let usage = if is_in {
        wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING
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
    };

    match buf.data {
        BufferSource::Cpu { .. } => {
            TextureHolder {
                wgpu_texture: device.create_texture(&desc),
                #[cfg(target_os = "windows")]
                d3d11_texture: None,
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                metal_texture: None
            }
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::Metal { texture, .. } => {
            if backend != wgpu::Backend::Metal { panic!("Unsupported backend!"); }
            TextureHolder {
                wgpu_texture: if buf.texture_copy {
                    device.create_texture(&desc)
                } else {
                    create_texture_from_metal(&device, texture, buf.size.0 as u32, buf.size.1 as u32, format, usage)
                },
                metal_texture: None
            }
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::MetalBuffer { buffer, .. } => {
            if backend != wgpu::Backend::Metal { panic!("Unsupported backend!"); }
            let metal_usage = if is_in { metal::MTLTextureUsage::ShaderRead }
                                          else     { metal::MTLTextureUsage::RenderTarget };
            let metal_texture = create_metal_texture_from_buffer(buffer, buf.size.0 as u32, buf.size.1 as u32, buf.size.2 as u32, format, metal_usage);
            if metal_texture.as_ptr().is_null() {
                log::error!("Failed to create Metal texture from MTLBuffer!");
            }

            TextureHolder {
                wgpu_texture: if buf.texture_copy {
                    device.create_texture(&desc)
                } else {
                    create_texture_from_metal(&device, metal_texture.as_ptr(), buf.size.0 as u32, buf.size.1 as u32, format, usage)
                },
                metal_texture: Some(metal_texture)
            }
        },
        #[cfg(target_os = "windows")]
        BufferSource::DirectX { texture, device: d3d11_device, device_context } => {
            unsafe {
                let d3d11_device = ID3D11Device::from_raw_borrowed(&d3d11_device);
                // let device_context = ID3D11DeviceContext::from_raw_borrowed(device_context);
                let texture = ID3D11Texture2D::from_raw_borrowed(&texture);
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                texture.GetDesc(&mut desc);
                let conv_format = format_dxgi_to_wgpu(desc.Format);
                assert_eq!(format, conv_format);
                assert_eq!(desc.Width, buf.size.0 as u32);
                assert_eq!(desc.Height, buf.size.1 as u32);

                match backend {
                    wgpu::Backend::Vulkan => {
                        let (image, dx_tex) = create_vk_image_from_d3d11_texture(&device, d3d11_device, texture).unwrap();

                        TextureHolder {
                            wgpu_texture: create_texture_from_vk_image(&device, image, desc.Width, desc.Height, conv_format, is_in),
                            d3d11_texture: dx_tex
                        }
                    },
                    /*wgpu::Backend::Dx12 => {
                        // TODO
                    },
                    wgpu::Backend::Dx11 => {
                        // TODO
                    },*/
                    _ => {
                        panic!("Unsupported backend!")
                    }
                }
            }
        },
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        BufferSource::Vulkan { texture, .. } => {
            if backend != wgpu::Backend::Vulkan { panic!("Unable to use Vulkan texture on non-Vulkan backend!"); }
            use ash::vk::Handle;
            TextureHolder {
                wgpu_texture: create_texture_from_vk_image(&device, vk::Image::from_raw(texture), buf.size.0 as u32, buf.size.1 as u32, format, is_in),

                #[cfg(target_os = "windows")]
                d3d11_texture: None,
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                metal_texture: None
            }
        },
        _ => {
            panic!("Unsupported buffer {:?}", buf.data);
        }
    }
}

pub fn handle_input_texture(device: &wgpu::Device, buf: &BufferDescription, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, in_texture: &TextureHolder, format: wgpu::TextureFormat) -> Option<wgpu::Texture> {
    let mut temp_texture = None;

    let size = Extent3d { width: buf.size.0 as u32, height: buf.size.1 as u32, depth_or_array_layers: 1 };

    match &buf.data {
        BufferSource::Cpu { buffer } => {
            queue.write_texture(
                in_texture.wgpu_texture.as_image_copy(),
                bytemuck::cast_slice(buffer),
                ImageDataLayout { offset: 0, bytes_per_row: NonZeroU32::new(buf.size.2 as u32), rows_per_image: None },
                size,
            );
        },
        #[cfg(target_os = "windows")]
        BufferSource::DirectX { texture, device_context, .. } => {
            unsafe {
                let device_context = ID3D11DeviceContext::from_raw_borrowed(device_context);
                let texture = ID3D11Texture2D::from_raw_borrowed(texture);
                if let Some(i) = &in_texture.d3d11_texture {
                    device_context.CopyResource(i, texture);
                }
            }
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::Metal { texture, .. } => {
            if buf.texture_copy {
                temp_texture = Some(create_texture_from_metal(device, *texture as *mut metal::MTLTexture, buf.size.0 as u32, buf.size.1 as u32, format, wgpu::TextureUsages::COPY_SRC));

                encoder.copy_texture_to_texture(
                    ImageCopyTexture { texture: temp_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                    ImageCopyTexture { texture: &in_texture.wgpu_texture, mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                    size
                );
            }
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::MetalBuffer { .. } => {
            if buf.texture_copy {
                let mtl_texture = in_texture.metal_texture.as_ref().unwrap();
                if !mtl_texture.as_ptr().is_null() {
                    temp_texture = Some(create_texture_from_metal(device, mtl_texture.as_ptr(), buf.size.0 as u32, buf.size.1 as u32, format, wgpu::TextureUsages::COPY_SRC));

                    encoder.copy_texture_to_texture(
                        ImageCopyTexture { texture: temp_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                        ImageCopyTexture { texture: &in_texture.wgpu_texture, mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                        size
                    );
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
                ImageCopyTexture { texture: &out_texture.wgpu_texture, mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                ImageCopyBuffer { buffer: staging_buffer, layout: ImageDataLayout { offset: 0, bytes_per_row: NonZeroU32::new(padded_stride), rows_per_image: None } },
                size
            );
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::Metal { texture, .. } => {
            if buf.texture_copy {
                temp_texture = Some(create_texture_from_metal(&device, *texture as *mut metal::MTLTexture, buf.size.0 as u32, buf.size.1 as u32, format, wgpu::TextureUsages::COPY_DST));

                encoder.copy_texture_to_texture(
                    ImageCopyTexture { texture: &out_texture.wgpu_texture, mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                    ImageCopyTexture { texture: temp_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                    size
                );
            }
        },
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        BufferSource::MetalBuffer { .. } => {
            let mtl_texture = out_texture.metal_texture.as_ref().unwrap();
            if !mtl_texture.as_ptr().is_null() {
                temp_texture = Some(create_texture_from_metal(device, mtl_texture.as_ptr(), buf.size.0 as u32, buf.size.1 as u32, format, wgpu::TextureUsages::COPY_DST));

                encoder.copy_texture_to_texture(
                    ImageCopyTexture { texture: &out_texture.wgpu_texture, mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                    ImageCopyTexture { texture: temp_texture.as_ref().unwrap(), mip_level: 0, origin: Origin3d::ZERO, aspect: TextureAspect::All },
                    size
                );
            }
        }
        _ => { }
    }

    temp_texture
}


