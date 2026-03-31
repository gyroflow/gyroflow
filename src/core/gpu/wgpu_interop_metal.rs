// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use wgpu::Device;
use wgpu::hal::api::Metal;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::*;

pub fn check_metal_stride(device: &Device, format: wgpu::TextureFormat, stride: usize) -> bool {
    let alignment = unsafe { device.as_hal::<Metal>().and_then(|device| {
        let raw_device = device.raw_device();
        Some(raw_device.minimumLinearTextureAlignmentForPixelFormat(format_wgpu_to_metal(format)) as usize)
    }).unwrap_or(16) };

    if stride % alignment != 0 {
        log::error!("Invalid stride alignment: stride: {stride}, required alignment: {alignment}");
    }

    stride % alignment == 0
}

pub fn create_metal_texture_from_buffer(buffer: *mut std::ffi::c_void, width: u32, height: u32, stride: u32, format: wgpu::TextureFormat, usage: MTLTextureUsage) -> Option<Retained<ProtocolObject<dyn MTLTexture>>> {
    let buf: &ProtocolObject<dyn MTLBuffer> = unsafe { &*(buffer as *const ProtocolObject<dyn MTLBuffer>) };
    let texture_descriptor = MTLTextureDescriptor::new();
    unsafe { texture_descriptor.setWidth(width as usize) };
    unsafe { texture_descriptor.setHeight(height as usize) };
    unsafe { texture_descriptor.setDepth(1) };
    texture_descriptor.setTextureType(MTLTextureType::Type2D);
    texture_descriptor.setPixelFormat(format_wgpu_to_metal(format));
    texture_descriptor.setStorageMode(buf.storageMode());
    texture_descriptor.setUsage(usage | MTLTextureUsage::ShaderRead | MTLTextureUsage::ShaderWrite | MTLTextureUsage::RenderTarget);

    buf.newTextureWithDescriptor_offset_bytesPerRow(&texture_descriptor, 0, stride as usize)
}

fn retain_texture(ptr: *mut std::ffi::c_void) -> Retained<ProtocolObject<dyn MTLTexture>> {
    unsafe {
        Retained::retain(ptr as *mut ProtocolObject<dyn MTLTexture>)
    }.unwrap()
}

fn retain_buffer(ptr: *mut std::ffi::c_void) -> Retained<ProtocolObject<dyn MTLBuffer>> {
    unsafe {
        Retained::retain(ptr as *mut ProtocolObject<dyn MTLBuffer>)
    }.unwrap()
}

pub fn create_texture_from_metal(device: &Device, image: *mut std::ffi::c_void, width: u32, height: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::Texture {
    let image = retain_texture(image);

    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };

    let texture = unsafe {
        <Metal as wgpu::hal::Api>::Device::texture_from_raw(
            image,
            format,
            MTLTextureType::Type2D,
            1,
            1,
            wgpu::hal::CopyExtent {
                width,
                height,
                depth: 1,
            }
        )
    };

    unsafe {
        device.create_texture_from_hal::<Metal>(
            texture,
            &wgpu::TextureDescriptor {
                label: None,
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            },
        )
    }
}

pub fn create_buffer_from_metal(device: &Device, buffer: *mut std::ffi::c_void, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    let buffer = retain_buffer(buffer);
    let buffer = unsafe { <Metal as wgpu::hal::Api>::Device::buffer_from_raw(buffer, size) };
    unsafe {
        device.create_buffer_from_hal::<Metal>(
            buffer,
            &wgpu::BufferDescriptor {
                label: None,
                size,
                mapped_at_creation: false,
                usage,
            },
        )
    }
}

pub fn format_wgpu_to_metal(format: wgpu::TextureFormat) -> MTLPixelFormat {
    use wgpu::TextureFormat as Tf;
    use wgpu::{AstcBlock, AstcChannel};
    match format {
        Tf::R8Unorm => MTLPixelFormat::R8Unorm,
        Tf::R8Snorm => MTLPixelFormat::R8Snorm,
        Tf::R8Uint => MTLPixelFormat::R8Uint,
        Tf::R8Sint => MTLPixelFormat::R8Sint,
        Tf::R16Uint => MTLPixelFormat::R16Uint,
        Tf::R16Sint => MTLPixelFormat::R16Sint,
        Tf::R16Unorm => MTLPixelFormat::R16Unorm,
        Tf::R16Snorm => MTLPixelFormat::R16Snorm,
        Tf::R16Float => MTLPixelFormat::R16Float,
        Tf::Rg8Unorm => MTLPixelFormat::RG8Unorm,
        Tf::Rg8Snorm => MTLPixelFormat::RG8Snorm,
        Tf::Rg8Uint => MTLPixelFormat::RG8Uint,
        Tf::Rg8Sint => MTLPixelFormat::RG8Sint,
        Tf::Rg16Unorm => MTLPixelFormat::RG16Unorm,
        Tf::Rg16Snorm => MTLPixelFormat::RG16Snorm,
        Tf::R32Uint => MTLPixelFormat::R32Uint,
        Tf::R32Sint => MTLPixelFormat::R32Sint,
        Tf::R32Float => MTLPixelFormat::R32Float,
        Tf::Rg16Uint => MTLPixelFormat::RG16Uint,
        Tf::Rg16Sint => MTLPixelFormat::RG16Sint,
        Tf::Rg16Float => MTLPixelFormat::RG16Float,
        Tf::Rgba8Unorm => MTLPixelFormat::RGBA8Unorm,
        Tf::Rgba8UnormSrgb => MTLPixelFormat::RGBA8Unorm_sRGB,
        Tf::Bgra8UnormSrgb => MTLPixelFormat::BGRA8Unorm_sRGB,
        Tf::Rgba8Snorm => MTLPixelFormat::RGBA8Snorm,
        Tf::Bgra8Unorm => MTLPixelFormat::BGRA8Unorm,
        Tf::Rgba8Uint => MTLPixelFormat::RGBA8Uint,
        Tf::Rgba8Sint => MTLPixelFormat::RGBA8Sint,
        Tf::Rgb10a2Unorm => MTLPixelFormat::RGB10A2Unorm,
        Tf::Rgb10a2Uint => MTLPixelFormat::RGB10A2Uint,
        Tf::Rg11b10Ufloat => MTLPixelFormat::RG11B10Float,
        Tf::Rg32Uint => MTLPixelFormat::RG32Uint,
        Tf::Rg32Sint => MTLPixelFormat::RG32Sint,
        Tf::Rg32Float => MTLPixelFormat::RG32Float,
        Tf::Rgba16Uint => MTLPixelFormat::RGBA16Uint,
        Tf::Rgba16Sint => MTLPixelFormat::RGBA16Sint,
        Tf::Rgba16Unorm => MTLPixelFormat::RGBA16Unorm,
        Tf::Rgba16Snorm => MTLPixelFormat::RGBA16Snorm,
        Tf::Rgba16Float => MTLPixelFormat::RGBA16Float,
        Tf::Rgba32Uint => MTLPixelFormat::RGBA32Uint,
        Tf::Rgba32Sint => MTLPixelFormat::RGBA32Sint,
        Tf::Rgba32Float => MTLPixelFormat::RGBA32Float,
        //Tf::Stencil8 => MTLPixelFormat::R8Unorm,
        Tf::Depth16Unorm => MTLPixelFormat::Depth16Unorm,
        Tf::Depth32Float => MTLPixelFormat::Depth32Float,
        Tf::Depth32FloatStencil8 => MTLPixelFormat::Depth32Float_Stencil8,
        Tf::Rgb9e5Ufloat => MTLPixelFormat::RGB9E5Float,
        Tf::Bc1RgbaUnorm => MTLPixelFormat::BC1_RGBA,
        Tf::Bc1RgbaUnormSrgb => MTLPixelFormat::BC1_RGBA_sRGB,
        Tf::Bc2RgbaUnorm => MTLPixelFormat::BC2_RGBA,
        Tf::Bc2RgbaUnormSrgb => MTLPixelFormat::BC2_RGBA_sRGB,
        Tf::Bc3RgbaUnorm => MTLPixelFormat::BC3_RGBA,
        Tf::Bc3RgbaUnormSrgb => MTLPixelFormat::BC3_RGBA_sRGB,
        Tf::Bc4RUnorm => MTLPixelFormat::BC4_RUnorm,
        Tf::Bc4RSnorm => MTLPixelFormat::BC4_RSnorm,
        Tf::Bc5RgUnorm => MTLPixelFormat::BC5_RGUnorm,
        Tf::Bc5RgSnorm => MTLPixelFormat::BC5_RGSnorm,
        Tf::Bc6hRgbFloat => MTLPixelFormat::BC6H_RGBFloat,
        Tf::Bc6hRgbUfloat => MTLPixelFormat::BC6H_RGBUfloat,
        Tf::Bc7RgbaUnorm => MTLPixelFormat::BC7_RGBAUnorm,
        Tf::Bc7RgbaUnormSrgb => MTLPixelFormat::BC7_RGBAUnorm_sRGB,
        Tf::Etc2Rgb8Unorm => MTLPixelFormat::ETC2_RGB8,
        Tf::Etc2Rgb8UnormSrgb => MTLPixelFormat::ETC2_RGB8_sRGB,
        Tf::Etc2Rgb8A1Unorm => MTLPixelFormat::ETC2_RGB8A1,
        Tf::Etc2Rgb8A1UnormSrgb => MTLPixelFormat::ETC2_RGB8A1_sRGB,
        Tf::Etc2Rgba8Unorm => MTLPixelFormat::EAC_RGBA8,
        Tf::Etc2Rgba8UnormSrgb => MTLPixelFormat::EAC_RGBA8_sRGB,
        Tf::EacR11Unorm => MTLPixelFormat::EAC_R11Unorm,
        Tf::EacR11Snorm => MTLPixelFormat::EAC_R11Snorm,
        Tf::EacRg11Unorm => MTLPixelFormat::EAC_RG11Unorm,
        Tf::EacRg11Snorm => MTLPixelFormat::EAC_RG11Snorm,
        Tf::Astc { block, channel } => match channel {
            AstcChannel::Unorm => match block {
                AstcBlock::B4x4 => MTLPixelFormat::ASTC_4x4_LDR,
                AstcBlock::B5x4 => MTLPixelFormat::ASTC_5x4_LDR,
                AstcBlock::B5x5 => MTLPixelFormat::ASTC_5x5_LDR,
                AstcBlock::B6x5 => MTLPixelFormat::ASTC_6x5_LDR,
                AstcBlock::B6x6 => MTLPixelFormat::ASTC_6x6_LDR,
                AstcBlock::B8x5 => MTLPixelFormat::ASTC_8x5_LDR,
                AstcBlock::B8x6 => MTLPixelFormat::ASTC_8x6_LDR,
                AstcBlock::B8x8 => MTLPixelFormat::ASTC_8x8_LDR,
                AstcBlock::B10x5 => MTLPixelFormat::ASTC_10x5_LDR,
                AstcBlock::B10x6 => MTLPixelFormat::ASTC_10x6_LDR,
                AstcBlock::B10x8 => MTLPixelFormat::ASTC_10x8_LDR,
                AstcBlock::B10x10 => MTLPixelFormat::ASTC_10x10_LDR,
                AstcBlock::B12x10 => MTLPixelFormat::ASTC_12x10_LDR,
                AstcBlock::B12x12 => MTLPixelFormat::ASTC_12x12_LDR,
            },
            AstcChannel::UnormSrgb => match block {
                AstcBlock::B4x4 => MTLPixelFormat::ASTC_4x4_sRGB,
                AstcBlock::B5x4 => MTLPixelFormat::ASTC_5x4_sRGB,
                AstcBlock::B5x5 => MTLPixelFormat::ASTC_5x5_sRGB,
                AstcBlock::B6x5 => MTLPixelFormat::ASTC_6x5_sRGB,
                AstcBlock::B6x6 => MTLPixelFormat::ASTC_6x6_sRGB,
                AstcBlock::B8x5 => MTLPixelFormat::ASTC_8x5_sRGB,
                AstcBlock::B8x6 => MTLPixelFormat::ASTC_8x6_sRGB,
                AstcBlock::B8x8 => MTLPixelFormat::ASTC_8x8_sRGB,
                AstcBlock::B10x5 => MTLPixelFormat::ASTC_10x5_sRGB,
                AstcBlock::B10x6 => MTLPixelFormat::ASTC_10x6_sRGB,
                AstcBlock::B10x8 => MTLPixelFormat::ASTC_10x8_sRGB,
                AstcBlock::B10x10 => MTLPixelFormat::ASTC_10x10_sRGB,
                AstcBlock::B12x10 => MTLPixelFormat::ASTC_12x10_sRGB,
                AstcBlock::B12x12 => MTLPixelFormat::ASTC_12x12_sRGB,
            },
            AstcChannel::Hdr => match block {
                AstcBlock::B4x4 => MTLPixelFormat::ASTC_4x4_HDR,
                AstcBlock::B5x4 => MTLPixelFormat::ASTC_5x4_HDR,
                AstcBlock::B5x5 => MTLPixelFormat::ASTC_5x5_HDR,
                AstcBlock::B6x5 => MTLPixelFormat::ASTC_6x5_HDR,
                AstcBlock::B6x6 => MTLPixelFormat::ASTC_6x6_HDR,
                AstcBlock::B8x5 => MTLPixelFormat::ASTC_8x5_HDR,
                AstcBlock::B8x6 => MTLPixelFormat::ASTC_8x6_HDR,
                AstcBlock::B8x8 => MTLPixelFormat::ASTC_8x8_HDR,
                AstcBlock::B10x5 => MTLPixelFormat::ASTC_10x5_HDR,
                AstcBlock::B10x6 => MTLPixelFormat::ASTC_10x6_HDR,
                AstcBlock::B10x8 => MTLPixelFormat::ASTC_10x8_HDR,
                AstcBlock::B10x10 => MTLPixelFormat::ASTC_10x10_HDR,
                AstcBlock::B12x10 => MTLPixelFormat::ASTC_12x10_HDR,
                AstcBlock::B12x12 => MTLPixelFormat::ASTC_12x12_HDR,
            },
        },
        _ => { panic!("Unsupported pixel format {:?}", format); }
    }
}
