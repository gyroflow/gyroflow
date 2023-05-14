// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use wgpu::Device;
use wgpu_hal::api::Metal;
use foreign_types::ForeignTypeRef;

pub fn create_metal_texture_from_buffer(buffer: *mut metal::MTLBuffer, width: u32, height: u32, stride: u32, format: wgpu::TextureFormat, usage: metal::MTLTextureUsage) -> metal::Texture {
    let buf = unsafe { metal::BufferRef::from_ptr(buffer) };
    let texture_descriptor = metal::TextureDescriptor::new();
    texture_descriptor.set_width(width as u64);
    texture_descriptor.set_height(height as u64);
    texture_descriptor.set_depth(1);
    texture_descriptor.set_texture_type(metal::MTLTextureType::D2);
    texture_descriptor.set_pixel_format(format_wgpu_to_metal(format));
    texture_descriptor.set_storage_mode(metal::MTLStorageMode::Private); // GPU only.
    texture_descriptor.set_usage(usage);

    buf.new_texture_with_descriptor(&texture_descriptor, 0, stride as u64)
}

pub fn create_texture_from_metal(device: &Device, image: *mut metal::MTLTexture, width: u32, height: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::Texture {
    let image = unsafe { metal::TextureRef::from_ptr(image) }.to_owned();

    let size = wgpu::Extent3d {
        width: width,
        height: height,
        depth_or_array_layers: 1,
    };

    let texture = unsafe {
        <Metal as wgpu_hal::Api>::Device::texture_from_raw(
            image,
            format,
            metal::MTLTextureType::D2,
            1,
            1,
            wgpu_hal::CopyExtent {
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

pub fn create_buffer_from_metal(device: &Device, buffer: *mut metal::MTLBuffer, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    let buffer = unsafe { metal::BufferRef::from_ptr(buffer) }.to_owned();
    let buffer = unsafe { <Metal as wgpu_hal::Api>::Device::buffer_from_raw(buffer, size) };
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

pub fn format_wgpu_to_metal(format: wgpu::TextureFormat) -> metal::MTLPixelFormat {
    use wgpu::TextureFormat as Tf;
    use wgpu::{AstcBlock, AstcChannel};
    use metal::MTLPixelFormat::*;
    match format {
        Tf::R8Unorm => R8Unorm,
        Tf::R8Snorm => R8Snorm,
        Tf::R8Uint => R8Uint,
        Tf::R8Sint => R8Sint,
        Tf::R16Uint => R16Uint,
        Tf::R16Sint => R16Sint,
        Tf::R16Unorm => R16Unorm,
        Tf::R16Snorm => R16Snorm,
        Tf::R16Float => R16Float,
        Tf::Rg8Unorm => RG8Unorm,
        Tf::Rg8Snorm => RG8Snorm,
        Tf::Rg8Uint => RG8Uint,
        Tf::Rg8Sint => RG8Sint,
        Tf::Rg16Unorm => RG16Unorm,
        Tf::Rg16Snorm => RG16Snorm,
        Tf::R32Uint => R32Uint,
        Tf::R32Sint => R32Sint,
        Tf::R32Float => R32Float,
        Tf::Rg16Uint => RG16Uint,
        Tf::Rg16Sint => RG16Sint,
        Tf::Rg16Float => RG16Float,
        Tf::Rgba8Unorm => RGBA8Unorm,
        Tf::Rgba8UnormSrgb => RGBA8Unorm_sRGB,
        Tf::Bgra8UnormSrgb => BGRA8Unorm_sRGB,
        Tf::Rgba8Snorm => RGBA8Snorm,
        Tf::Bgra8Unorm => BGRA8Unorm,
        Tf::Rgba8Uint => RGBA8Uint,
        Tf::Rgba8Sint => RGBA8Sint,
        Tf::Rgb10a2Unorm => RGB10A2Unorm,
        Tf::Rg11b10Float => RG11B10Float,
        Tf::Rg32Uint => RG32Uint,
        Tf::Rg32Sint => RG32Sint,
        Tf::Rg32Float => RG32Float,
        Tf::Rgba16Uint => RGBA16Uint,
        Tf::Rgba16Sint => RGBA16Sint,
        Tf::Rgba16Unorm => RGBA16Unorm,
        Tf::Rgba16Snorm => RGBA16Snorm,
        Tf::Rgba16Float => RGBA16Float,
        Tf::Rgba32Uint => RGBA32Uint,
        Tf::Rgba32Sint => RGBA32Sint,
        Tf::Rgba32Float => RGBA32Float,
        //Tf::Stencil8 => R8Unorm,
        Tf::Depth16Unorm => Depth16Unorm,
        Tf::Depth32Float => Depth32Float,
        Tf::Depth32FloatStencil8 => Depth32Float_Stencil8,
        Tf::Rgb9e5Ufloat => RGB9E5Float,
        Tf::Bc1RgbaUnorm => BC1_RGBA,
        Tf::Bc1RgbaUnormSrgb => BC1_RGBA_sRGB,
        Tf::Bc2RgbaUnorm => BC2_RGBA,
        Tf::Bc2RgbaUnormSrgb => BC2_RGBA_sRGB,
        Tf::Bc3RgbaUnorm => BC3_RGBA,
        Tf::Bc3RgbaUnormSrgb => BC3_RGBA_sRGB,
        Tf::Bc4RUnorm => BC4_RUnorm,
        Tf::Bc4RSnorm => BC4_RSnorm,
        Tf::Bc5RgUnorm => BC5_RGUnorm,
        Tf::Bc5RgSnorm => BC5_RGSnorm,
        Tf::Bc6hRgbFloat => BC6H_RGBFloat,
        Tf::Bc6hRgbUfloat => BC6H_RGBUfloat,
        Tf::Bc7RgbaUnorm => BC7_RGBAUnorm,
        Tf::Bc7RgbaUnormSrgb => BC7_RGBAUnorm_sRGB,
        Tf::Etc2Rgb8Unorm => ETC2_RGB8,
        Tf::Etc2Rgb8UnormSrgb => ETC2_RGB8_sRGB,
        Tf::Etc2Rgb8A1Unorm => ETC2_RGB8A1,
        Tf::Etc2Rgb8A1UnormSrgb => ETC2_RGB8A1_sRGB,
        Tf::Etc2Rgba8Unorm => EAC_RGBA8,
        Tf::Etc2Rgba8UnormSrgb => EAC_RGBA8_sRGB,
        Tf::EacR11Unorm => EAC_R11Unorm,
        Tf::EacR11Snorm => EAC_R11Snorm,
        Tf::EacRg11Unorm => EAC_RG11Unorm,
        Tf::EacRg11Snorm => EAC_RG11Snorm,
        Tf::Astc { block, channel } => match channel {
            AstcChannel::Unorm => match block {
                AstcBlock::B4x4 => ASTC_4x4_LDR,
                AstcBlock::B5x4 => ASTC_5x4_LDR,
                AstcBlock::B5x5 => ASTC_5x5_LDR,
                AstcBlock::B6x5 => ASTC_6x5_LDR,
                AstcBlock::B6x6 => ASTC_6x6_LDR,
                AstcBlock::B8x5 => ASTC_8x5_LDR,
                AstcBlock::B8x6 => ASTC_8x6_LDR,
                AstcBlock::B8x8 => ASTC_8x8_LDR,
                AstcBlock::B10x5 => ASTC_10x5_LDR,
                AstcBlock::B10x6 => ASTC_10x6_LDR,
                AstcBlock::B10x8 => ASTC_10x8_LDR,
                AstcBlock::B10x10 => ASTC_10x10_LDR,
                AstcBlock::B12x10 => ASTC_12x10_LDR,
                AstcBlock::B12x12 => ASTC_12x12_LDR,
            },
            AstcChannel::UnormSrgb => match block {
                AstcBlock::B4x4 => ASTC_4x4_sRGB,
                AstcBlock::B5x4 => ASTC_5x4_sRGB,
                AstcBlock::B5x5 => ASTC_5x5_sRGB,
                AstcBlock::B6x5 => ASTC_6x5_sRGB,
                AstcBlock::B6x6 => ASTC_6x6_sRGB,
                AstcBlock::B8x5 => ASTC_8x5_sRGB,
                AstcBlock::B8x6 => ASTC_8x6_sRGB,
                AstcBlock::B8x8 => ASTC_8x8_sRGB,
                AstcBlock::B10x5 => ASTC_10x5_sRGB,
                AstcBlock::B10x6 => ASTC_10x6_sRGB,
                AstcBlock::B10x8 => ASTC_10x8_sRGB,
                AstcBlock::B10x10 => ASTC_10x10_sRGB,
                AstcBlock::B12x10 => ASTC_12x10_sRGB,
                AstcBlock::B12x12 => ASTC_12x12_sRGB,
            },
            AstcChannel::Hdr => match block {
                AstcBlock::B4x4 => ASTC_4x4_HDR,
                AstcBlock::B5x4 => ASTC_5x4_HDR,
                AstcBlock::B5x5 => ASTC_5x5_HDR,
                AstcBlock::B6x5 => ASTC_6x5_HDR,
                AstcBlock::B6x6 => ASTC_6x6_HDR,
                AstcBlock::B8x5 => ASTC_8x5_HDR,
                AstcBlock::B8x6 => ASTC_8x6_HDR,
                AstcBlock::B8x8 => ASTC_8x8_HDR,
                AstcBlock::B10x5 => ASTC_10x5_HDR,
                AstcBlock::B10x6 => ASTC_10x6_HDR,
                AstcBlock::B10x8 => ASTC_10x8_HDR,
                AstcBlock::B10x10 => ASTC_10x10_HDR,
                AstcBlock::B12x10 => ASTC_12x10_HDR,
                AstcBlock::B12x12 => ASTC_12x12_HDR,
            },
        },
        _ => { panic!("Unsupported pixel format {:?}", format); }
    }
}
