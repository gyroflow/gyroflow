// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

pub mod opencv_fisheye;
pub mod opencv_standard;
pub mod poly3;
pub mod poly5;
pub mod ptlens;
pub mod insta360;
pub mod sony;

pub mod gopro_superview;
pub mod gopro_hyperview;
pub mod digital_stretch;

use crate::KernelParams;
use crate::glam::{ Vec2, Vec3 };

macro_rules! impl_models {
    ($($id:tt::$name:tt,)*) => {
        #[derive(Clone, Copy)]
        #[repr(i32)]
        pub enum DistortionModel {
            $($name,)*
        }
        impl Default for DistortionModel {
            fn default() -> Self { Self::OpenCVFisheye }
        }
        impl DistortionModel {
            pub fn undistort_point(&self, point: Vec2, params: &KernelParams) -> Vec2 {
                match &self {
                    $(DistortionModel::$name => <$id::$name>::undistort_point(point, params),)*
                }
            }
            pub fn distort_point(&self, point: Vec3, params: &KernelParams) -> Vec2 {
                match &self {
                    $(DistortionModel::$name => <$id::$name>::distort_point(point, params),)*
                }
            }

            #[cfg(not(target_arch = "spirv"))]
            pub fn adjust_lens_profile(&self, calib_w: &mut usize, calib_h: &mut usize/*, lens_model: &mut String*/) {
                match &self {
                    $(DistortionModel::$name => <$id::$name>::adjust_lens_profile(calib_w, calib_h/*, lens_model*/),)*
                }
            }
            #[cfg(not(target_arch = "spirv"))]
            pub fn from_name(id: &str) -> Self {
                $(
                    if stringify!($id) == id { return Self::$name; }
                )*
                Self::default()
            }
        }
    };
}

impl_models! {
    none::None,

    // Physical lenses
    opencv_fisheye::OpenCVFisheye,
    opencv_standard::OpenCVStandard,
    poly3::Poly3,
    poly5::Poly5,
    ptlens::PtLens,
    insta360::Insta360,
    sony::Sony,

    // Digital lenses (ie. post-processing)
    gopro_superview::GoProSuperview,
    gopro_hyperview::GoProHyperview,
    digital_stretch::DigitalStretch,
}

mod none {
    use crate::glam::{ Vec2, Vec3 };
    pub struct None { }
    impl None {
        #[inline] pub fn undistort_point(p: Vec2, _: &crate::KernelParams) -> Vec2 { p }
        #[inline] pub fn distort_point(p: Vec3, _: &crate::KernelParams) -> Vec2 { Vec2::new(p.x, p.y) }
        #[cfg(not(target_arch = "spirv"))] pub fn adjust_lens_profile(_: &mut usize, _: &mut usize/*, _: &mut String*/) { }
    }
}
