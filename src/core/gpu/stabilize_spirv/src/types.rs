// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

pub use spirv_std::glam;
use glam::{ Vec2, vec2, Vec4, IVec4 };

#[cfg(target_arch = "spirv")]
pub use spirv_std::num_traits::Float;

#[cfg(all(target_arch = "spirv", feature = "texture_u32"))]
pub type ImageType     = spirv_std::image::Image!(2D, type=u32, sampled);
#[cfg(all(target_arch = "spirv", not(feature = "texture_u32")))]
pub type ImageType     = spirv_std::image::Image!(2D, type=f32, sampled);
#[cfg(not(target_arch = "spirv"))]
pub type ImageType<'a> = (&'a [u8], fn(&[u8]) -> spirv_std::glam::Vec4);

#[cfg(not(feature = "for_qtrhi"))]
mod inner_types {
    pub type MatricesType  = [f32];
    pub type DrawingType   = [u32];
    pub type SamplerType   = f32;
}
#[cfg(feature = "for_qtrhi")]
mod inner_types {
    pub use spirv_std::image::Image;
    pub type MatricesType    = Image!(2D, type=f32, sampled);
    pub type DrawingType     = Image!(2D, type=f32, sampled);
    pub type SamplerType<'a> =  &'a spirv_std::Sampler;
}
pub use inner_types::*;

#[derive(Default, Copy, Clone)]
#[repr(C)]
pub struct KernelParams {
    pub width:             i32, // 4
    pub height:            i32, // 8
    pub stride:            i32, // 12
    pub output_width:      i32, // 16
    pub output_height:     i32, // 4
    pub output_stride:     i32, // 8
    pub matrix_count:      i32, // 12 - for rolling shutter correction. 1 = no correction, only main matrix
    pub interpolation:     i32, // 16
    pub background_mode:   i32, // 4
    pub flags:             i32, // 8
    pub bytes_per_pixel:   i32, // 12
    pub pix_element_count: i32, // 16
    pub background:        Vec4, // 16
    pub f:                 Vec2, // 8  - focal length in pixels
    pub c:                 Vec2, // 16 - lens center
    pub k1:                Vec4, // 16 - distortion coefficients
    pub k2:                Vec4, // 16 - distortion coefficients
    pub k3:                Vec4, // 16 - distortion coefficients
    pub fov:               f32, // 4
    pub r_limit:           f32, // 8
    pub lens_correction_amount:   f32, // 12
    pub input_vertical_stretch:   f32, // 16
    pub input_horizontal_stretch: f32, // 4
    pub background_margin:        f32, // 8
    pub background_margin_feather:f32, // 12
    pub canvas_scale:             f32, // 16
    pub input_rotation:           f32, // 4
    pub output_rotation:          f32, // 8
    pub translation2d:            Vec2, // 16
    pub translation3d:            Vec4, // 16
    pub source_rect:              IVec4, // 16 - x, y, w, h
    pub output_rect:              IVec4, // 16 - x, y, w, h
    pub digital_lens_params:      Vec4, // 16
    pub safe_area_rect:           Vec4, // 16
    pub max_pixel_value:          f32, // 4
    pub distortion_model:         crate::distortion_models::DistortionModel, // 8
    pub digital_lens:             crate::distortion_models::DistortionModel, // 12
    pub pixel_value_limit:        f32, // 16
}

// #[inline] pub fn fast_floor(x: f32) -> i32 { x as i32 }
// #[inline] pub fn fast_round(x: f32) -> i32 { fast_floor(x + 0.5) }
#[inline] pub fn fast_floor(x: f32) -> i32 { x.floor() as i32 }
#[inline] pub fn fast_round(x: f32) -> i32 { x.round() as i32 }

pub fn map_coord(x: f32, in_min: f32, in_max: f32, out_min: f32, out_max: f32) -> f32 {
    return (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min;
}
#[allow(unused)]
pub fn rotate_point(pos: Vec2, angle: f32, origin: Vec2) -> Vec2 {
    vec2(angle.cos() * (pos.x - origin.x) - angle.sin() * (pos.y - origin.y) + origin.x,
         angle.sin() * (pos.x - origin.x) + angle.cos() * (pos.y - origin.y) + origin.y)
}

#[cfg(feature = "texture_u32")]
mod inner_tex_type {
    use spirv_std::glam::{ Vec4, UVec4 };
    pub type ScalarVec4 = UVec4;
    pub fn to_float(v: UVec4) -> Vec4 { Vec4::new(v.x as f32, v.y as f32, v.z as f32, v.w as f32) }
    pub fn from_float(v: Vec4) -> UVec4 { UVec4::new(v.x as u32, v.y as u32, v.z as u32, v.w as u32)}
}
#[cfg(not(feature = "texture_u32"))]
mod inner_tex_type {
    use spirv_std::glam::Vec4;
    pub type ScalarVec4 = Vec4;
    #[inline] pub fn to_float(v: Vec4) -> Vec4 { v }
    #[inline] pub fn from_float(v: Vec4) -> Vec4 { v }
}
pub use inner_tex_type::*;
