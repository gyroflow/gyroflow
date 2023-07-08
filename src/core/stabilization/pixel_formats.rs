// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use nalgebra::Vector4;

pub trait PixelType: Default + Copy + Send + Sync + bytemuck::Pod {
    const COUNT: usize = 1;
    const SCALAR_BYTES: usize = 1;
    type Scalar: Default + bytemuck::Pod;

    fn to_float_glam(bytes: &[u8]) -> stabilize_spirv::glam::Vec4;
    fn from_float_glam(v: stabilize_spirv::glam::Vec4) -> Self;
    fn to_float(v: Self) -> Vector4<f32>;
    fn from_float(v: Vector4<f32>) -> Self;
    fn from_rgb_color(v: Vector4<f32>, ind: &[usize], is_limited: bool) -> Vector4<f32>;

    fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str);
    fn wgpu_format() -> Option<(wgpu::TextureFormat, &'static str, f64)>;
}

fn rgb_to_yuv(v: Vector4<f32>, is_limited: bool) -> Vector4<f32> {
    // From 0-255(JPEG/Full) to 16-235(MPEG/Limited)
    fn remap_to_limited(v: f32, is_y: bool) -> f32 {
        if is_y { ((16.0 / 255.0)) + (v * ((235.0 - 16.0) / 255.0)) }
        else    { ((16.0 / 255.0)) + (v * ((240.0 - 16.0) / 255.0)) }
    }

    const KR: f32 = 0.2126; // Rec709
    const KB: f32 = 0.0722; // Rec709
    const KG: f32 = 1.0 - KR - KB;
    const US: f32 = 1.0 / (2.0 - 2.0 * KB);
    const VS: f32 = 1.0 / (2.0 - 2.0 * KR);

    let mut ret = Vector4::new(
         (KR                * v[0] + KG         * v[1] + KB                * v[2]).max(0.0).min(1.0),
        ((-KR * US)         * v[0] + (-KG * US) * v[1] + ((1.0 - KB) * US) * v[2] + 0.5).max(0.0).min(1.0),
         (((1.0 - KR) * VS) * v[0] + (-KG * VS) * v[1] + (-KB * VS)        * v[2] + 0.5).max(0.0).min(1.0),
         v[3].max(0.0).min(1.0)
    );
    if is_limited {
        ret[0] = remap_to_limited(ret[0], true);
        ret[1] = remap_to_limited(ret[1], false);
        ret[2] = remap_to_limited(ret[2], false);
    }
    ret
}

#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct Luma8(u8);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct Luma16(u16);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct RGB8(u8, u8, u8);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct RGBA8(u8, u8, u8, u8);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct BGRA8(u8, u8, u8, u8);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct RGB16(u16, u16, u16);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct RGBA16(u16, u16, u16, u16);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct AYUV16(u16, u16, u16, u16);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct RGBAf(f32, f32, f32, f32);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct RGBAf16(Ff16, Ff16, Ff16, Ff16);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct R32f(f32);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct UV8(u8, u8);
#[derive(Default, Clone, Copy, PartialEq, PartialOrd)] pub struct UV16(u16, u16);

unsafe impl bytemuck::Zeroable for Luma8 { }
unsafe impl bytemuck::Pod for Luma8 { }
impl PixelType for Luma8 {
    const COUNT: usize = 1;
    const SCALAR_BYTES: usize = 1;
    type Scalar = u8;
    #[inline] fn to_float_glam(bytes: &[u8]) -> stabilize_spirv::glam::Vec4 {
        let v: &Self = bytemuck::from_bytes(&bytes);
        stabilize_spirv::glam::vec4(v.0 as f32, 0.0, 0.0, 0.0)
    }
    #[inline] fn from_float_glam(v: stabilize_spirv::glam::Vec4) -> Self { Self(v[0] as Self::Scalar) }
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, 0.0, 0.0, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize], is_limited: bool) -> Vector4<f32> { Vector4::new(rgb_to_yuv(v, is_limited)[ind[0]], 0.0, 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("uchar", "convert_uchar_sat", "float", "convert_float") }
    #[inline] fn wgpu_format() -> Option<(wgpu::TextureFormat, &'static str, f64)> { Some((wgpu::TextureFormat::R8Unorm, "f32", 255.0)) }
}
unsafe impl bytemuck::Zeroable for Luma16 { }
unsafe impl bytemuck::Pod for Luma16 { }
impl PixelType for Luma16 {
    const COUNT: usize = 1;
    const SCALAR_BYTES: usize = 2;
    type Scalar = u16;
    #[inline] fn to_float_glam(bytes: &[u8]) -> stabilize_spirv::glam::Vec4 {
        let v: &Self = bytemuck::from_bytes(&bytes);
        stabilize_spirv::glam::vec4(v.0 as f32, 0.0, 0.0, 0.0)
    }
    #[inline] fn from_float_glam(v: stabilize_spirv::glam::Vec4) -> Self { Self(v[0] as Self::Scalar) }
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, 0.0, 0.0, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize], is_limited: bool) -> Vector4<f32> { Vector4::new(rgb_to_yuv(v, is_limited)[ind[0]], 0.0, 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("ushort", "convert_ushort_sat", "float", "convert_float") }
    #[inline] fn wgpu_format() -> Option<(wgpu::TextureFormat, &'static str, f64)> { Some((wgpu::TextureFormat::R16Uint, "u32", 1.0)) }
}
unsafe impl bytemuck::Zeroable for RGB8 { }
unsafe impl bytemuck::Pod for RGB8 { }
impl PixelType for RGB8 {
    const COUNT: usize = 3;
    const SCALAR_BYTES: usize = 1;
    type Scalar = u8;
    #[inline] fn to_float_glam(bytes: &[u8]) -> stabilize_spirv::glam::Vec4 {
        let v: &Self = bytemuck::from_bytes(&bytes);
        stabilize_spirv::glam::vec4(v.0 as f32, v.1 as f32, v.2 as f32, 0.0)
    }
    #[inline] fn from_float_glam(v: stabilize_spirv::glam::Vec4) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar) }
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, v.2 as f32, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize], is_limited: bool) -> Vector4<f32> { v }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("uchar3", "convert_uchar3_sat", "float4", "convert_float4") } // FIXME: uchar3 can't be converted to float4
    #[inline] fn wgpu_format() -> Option<(wgpu::TextureFormat, &'static str, f64)> { None }
}
unsafe impl bytemuck::Zeroable for RGBA8 { }
unsafe impl bytemuck::Pod for RGBA8 { }
impl PixelType for RGBA8 {
    const COUNT: usize = 4;
    const SCALAR_BYTES: usize = 1;
    type Scalar = u8;
    #[inline] fn to_float_glam(bytes: &[u8]) -> stabilize_spirv::glam::Vec4 {
        let v: &Self = bytemuck::from_bytes(&bytes);
        stabilize_spirv::glam::vec4(v.0 as f32, v.1 as f32, v.2 as f32, v.3 as f32)
    }
    #[inline] fn from_float_glam(v: stabilize_spirv::glam::Vec4) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar, v[3] as Self::Scalar) }
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, v.2 as f32, v.3 as f32) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar, v[3] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize], is_limited: bool) -> Vector4<f32> { v }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("uchar4", "convert_uchar4_sat", "float4", "convert_float4") }
    #[inline] fn wgpu_format() -> Option<(wgpu::TextureFormat, &'static str, f64)> { Some((wgpu::TextureFormat::Rgba8Unorm, "f32", 255.0)) }
}
unsafe impl bytemuck::Zeroable for BGRA8 { }
unsafe impl bytemuck::Pod for BGRA8 { }
impl PixelType for BGRA8 {
    const COUNT: usize = 4;
    const SCALAR_BYTES: usize = 1;
    type Scalar = u8;
    #[inline] fn to_float_glam(bytes: &[u8]) -> stabilize_spirv::glam::Vec4 {
        let v: &Self = bytemuck::from_bytes(&bytes);
        stabilize_spirv::glam::vec4(v.0 as f32, v.1 as f32, v.2 as f32, v.3 as f32)
    }
    #[inline] fn from_float_glam(v: stabilize_spirv::glam::Vec4) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar, v[3] as Self::Scalar) }
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, v.2 as f32, v.3 as f32) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar, v[3] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize], is_limited: bool) -> Vector4<f32> { Vector4::new(v[2], v[1], v[0], v[3]) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("uchar4", "convert_uchar4_sat", "float4", "convert_float4") }
    #[inline] fn wgpu_format() -> Option<(wgpu::TextureFormat, &'static str, f64)> { Some((wgpu::TextureFormat::Bgra8Unorm, "f32", 255.0)) }
}
unsafe impl bytemuck::Zeroable for RGB16 { }
unsafe impl bytemuck::Pod for RGB16 { }
impl PixelType for RGB16 {
    const COUNT: usize = 3;
    const SCALAR_BYTES: usize = 2;
    type Scalar = u16;
    #[inline] fn to_float_glam(bytes: &[u8]) -> stabilize_spirv::glam::Vec4 {
        let v: &Self = bytemuck::from_bytes(&bytes);
        stabilize_spirv::glam::vec4(v.0 as f32, v.1 as f32, v.2 as f32, 0.0)
    }
    #[inline] fn from_float_glam(v: stabilize_spirv::glam::Vec4) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar) }
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, v.2 as f32, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize], is_limited: bool) -> Vector4<f32> { v }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("ushort3", "convert_ushort3_sat", "float4", "convert_float4") }
    #[inline] fn wgpu_format() -> Option<(wgpu::TextureFormat, &'static str, f64)> { None }
}
unsafe impl bytemuck::Zeroable for RGBA16 { }
unsafe impl bytemuck::Pod for RGBA16 { }
impl PixelType for RGBA16 {
    const COUNT: usize = 4;
    const SCALAR_BYTES: usize = 2;
    type Scalar = u16;
    #[inline] fn to_float_glam(bytes: &[u8]) -> stabilize_spirv::glam::Vec4 {
        let v: &Self = bytemuck::from_bytes(&bytes);
        stabilize_spirv::glam::vec4(v.0 as f32, v.1 as f32, v.2 as f32, v.3 as f32)
    }
    #[inline] fn from_float_glam(v: stabilize_spirv::glam::Vec4) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar, v[3] as Self::Scalar) }
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, v.2 as f32, v.3 as f32) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar, v[3] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize], is_limited: bool) -> Vector4<f32> { v }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("ushort4", "convert_ushort4_sat", "float4", "convert_float4") }
    #[inline] fn wgpu_format() -> Option<(wgpu::TextureFormat, &'static str, f64)> { Some((wgpu::TextureFormat::Rgba16Uint, "u32", 1.0)) }
}
unsafe impl bytemuck::Zeroable for AYUV16 { }
unsafe impl bytemuck::Pod for AYUV16 { }
impl PixelType for AYUV16 {
    const COUNT: usize = 4;
    const SCALAR_BYTES: usize = 2;
    type Scalar = u16;
    #[inline] fn to_float_glam(bytes: &[u8]) -> stabilize_spirv::glam::Vec4 {
        let v: &Self = bytemuck::from_bytes(&bytes);
        stabilize_spirv::glam::vec4(v.0 as f32, v.1 as f32, v.2 as f32, v.3 as f32)
    }
    #[inline] fn from_float_glam(v: stabilize_spirv::glam::Vec4) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar, v[3] as Self::Scalar) }
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, v.2 as f32, v.3 as f32) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar, v[2] as Self::Scalar, v[3] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize], is_limited: bool) -> Vector4<f32> { let yuv = rgb_to_yuv(v, is_limited); Vector4::new(yuv[ind[0]], yuv[ind[1]], yuv[ind[2]], yuv[ind[3]]) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("ushort4", "convert_ushort4_sat", "float4", "convert_float4") }
    #[inline] fn wgpu_format() -> Option<(wgpu::TextureFormat, &'static str, f64)> { Some((wgpu::TextureFormat::Rgba16Uint, "u32", 1.0)) }
}
unsafe impl bytemuck::Zeroable for RGBAf { }
unsafe impl bytemuck::Pod for RGBAf { }
impl PixelType for RGBAf {
    const COUNT: usize = 4;
    const SCALAR_BYTES: usize = 4;
    type Scalar = f32;
    #[inline] fn to_float_glam(bytes: &[u8]) -> stabilize_spirv::glam::Vec4 {
        let v: &Self = bytemuck::from_bytes(&bytes);
        stabilize_spirv::glam::vec4(v.0, v.1, v.2, v.3)
    }
    #[inline] fn from_float_glam(v: stabilize_spirv::glam::Vec4) -> Self { Self(v[0], v[1], v[2], v[3]) }
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0, v.1, v.2, v.3) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0], v[1], v[2], v[3]) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize], is_limited: bool) -> Vector4<f32> { v }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("float4", "convert_float4", "float4", "convert_float4") }
    #[inline] fn wgpu_format() -> Option<(wgpu::TextureFormat, &'static str, f64)> { Some((wgpu::TextureFormat::Rgba32Float, "f32", 255.0)) }
}
#[derive(Default, Copy, Clone, PartialEq, PartialOrd)]
pub struct Ff16(half::f16);
unsafe impl bytemuck::Zeroable for Ff16 { }
unsafe impl bytemuck::Pod for Ff16 { }

unsafe impl bytemuck::Zeroable for RGBAf16 { }
unsafe impl bytemuck::Pod for RGBAf16 { }
impl PixelType for RGBAf16 {
    const COUNT: usize = 4;
    const SCALAR_BYTES: usize = 2;
    type Scalar = Ff16;
    #[inline] fn to_float_glam(bytes: &[u8]) -> stabilize_spirv::glam::Vec4 {
        let v: &Self = bytemuck::from_bytes(&bytes);
        stabilize_spirv::glam::vec4(v.0.0.to_f32(), v.1.0.to_f32(), v.2.0.to_f32(), v.3.0.to_f32())
    }
    #[inline] fn from_float_glam(v: stabilize_spirv::glam::Vec4) -> Self { Self(Ff16(half::f16::from_f32(v[0])), Ff16(half::f16::from_f32(v[1])), Ff16(half::f16::from_f32(v[2])), Ff16(half::f16::from_f32(v[3]))) }
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0.0.to_f32(), v.1.0.to_f32(), v.2.0.to_f32(), v.3.0.to_f32()) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(Ff16(half::f16::from_f32(v[0])), Ff16(half::f16::from_f32(v[1])), Ff16(half::f16::from_f32(v[2])), Ff16(half::f16::from_f32(v[3]))) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, _ind: &[usize], is_limited: bool) -> Vector4<f32> { v }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("half4", "convert_half4", "float4", "convert_float4") }
    #[inline] fn wgpu_format() -> Option<(wgpu::TextureFormat, &'static str, f64)> { Some((wgpu::TextureFormat::Rgba16Float, "f32", 255.0)) }
}
unsafe impl bytemuck::Zeroable for R32f { }
unsafe impl bytemuck::Pod for R32f { }
impl PixelType for R32f {
    const COUNT: usize = 1;
    const SCALAR_BYTES: usize = 4;
    type Scalar = f32;
    #[inline] fn to_float_glam(bytes: &[u8]) -> stabilize_spirv::glam::Vec4 {
        let v: &Self = bytemuck::from_bytes(&bytes);
        stabilize_spirv::glam::vec4(v.0, 0.0, 0.0, 0.0)
    }
    #[inline] fn from_float_glam(v: stabilize_spirv::glam::Vec4) -> Self { Self(v[0]) }
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0, 0.0, 0.0, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0]) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize], is_limited: bool) -> Vector4<f32> { Vector4::new(v[ind[0]], 0.0, 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("float", "convert_float", "float", "convert_float") }
    #[inline] fn wgpu_format() -> Option<(wgpu::TextureFormat, &'static str, f64)> { Some((wgpu::TextureFormat::R32Float, "f32", 255.0)) }
}
unsafe impl bytemuck::Zeroable for UV8 { }
unsafe impl bytemuck::Pod for UV8 { }
impl PixelType for UV8 {
    const COUNT: usize = 2;
    const SCALAR_BYTES: usize = 1;
    type Scalar = u8;
    #[inline] fn to_float_glam(bytes: &[u8]) -> stabilize_spirv::glam::Vec4 {
        let v: &Self = bytemuck::from_bytes(&bytes);
        stabilize_spirv::glam::vec4(v.0 as f32, v.1 as f32, 0.0, 0.0)
    }
    #[inline] fn from_float_glam(v: stabilize_spirv::glam::Vec4) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar) }
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, 0.0, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize], is_limited: bool) -> Vector4<f32> { let yuv = rgb_to_yuv(v, is_limited); Vector4::new(yuv[ind[0]], yuv[ind[1]], 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("uchar2", "convert_uchar2_sat", "float2", "convert_float2") }
    #[inline] fn wgpu_format() -> Option<(wgpu::TextureFormat, &'static str, f64)> { Some((wgpu::TextureFormat::Rg8Unorm, "f32", 255.0)) }
}
unsafe impl bytemuck::Zeroable for UV16 { }
unsafe impl bytemuck::Pod for UV16 { }
impl PixelType for UV16 {
    const COUNT: usize = 2;
    const SCALAR_BYTES: usize = 2;
    type Scalar = u16;
    #[inline] fn to_float_glam(bytes: &[u8]) -> stabilize_spirv::glam::Vec4 {
        let v: &Self = bytemuck::from_bytes(&bytes);
        stabilize_spirv::glam::vec4(v.0 as f32, v.1 as f32, 0.0, 0.0)
    }
    #[inline] fn from_float_glam(v: stabilize_spirv::glam::Vec4) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar) }
    #[inline] fn to_float(v: Self) -> Vector4<f32> { Vector4::new(v.0 as f32, v.1 as f32, 0.0, 0.0) }
    #[inline] fn from_float(v: Vector4<f32>) -> Self { Self(v[0] as Self::Scalar, v[1] as Self::Scalar) }
    #[inline] fn from_rgb_color(v: Vector4<f32>, ind: &[usize], is_limited: bool) -> Vector4<f32> { let yuv = rgb_to_yuv(v, is_limited); Vector4::new(yuv[ind[0]], yuv[ind[1]], 0.0, 0.0) }
    #[inline] fn ocl_names() -> (&'static str, &'static str, &'static str, &'static str) { ("ushort2", "convert_ushort2_sat", "float2", "convert_float2") }
    #[inline] fn wgpu_format() -> Option<(wgpu::TextureFormat, &'static str, f64)> { Some((wgpu::TextureFormat::Rg16Uint, "u32", 1.0)) }
}
