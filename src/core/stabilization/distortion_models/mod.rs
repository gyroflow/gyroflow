// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

mod opencv_fisheye;
mod opencv_standard;
mod poly3;
mod poly5;
mod ptlens;
mod insta360;

mod gopro_superview;
mod gopro_hyperview;
mod digital_stretch;

use super::KernelParams;

macro_rules! impl_models {
    ($($name:ident => $class:ty,)*) => {
        #[derive(Clone)]
        pub enum DistortionModels {
            $($name($class),)*
        }
        impl Default for DistortionModels {
            fn default() -> Self { Self::OpenCVFisheye(Default::default()) }
        }
        #[derive(Default, Clone)]
        pub struct DistortionModel {
            pub inner: DistortionModels
        }
        impl DistortionModel {
            pub fn undistort_point(&self, point: (f32, f32), params: &KernelParams) -> Option<(f32, f32)> {
                match &self.inner {
                    $(DistortionModels::$name(m) => m.undistort_point(point, params),)*
                }
            }
            pub fn distort_point(&self, x: f32, y: f32, z: f32, params: &KernelParams) -> (f32, f32) {
                match &self.inner {
                    $(DistortionModels::$name(m) => m.distort_point(x, y, z, params),)*
                }
            }
            pub fn adjust_lens_profile(&self, profile: &mut crate::LensProfile) {
                match &self.inner {
                    $(DistortionModels::$name(m) => m.adjust_lens_profile(profile),)*
                }
            }
            pub fn distort_for_light_refraction(&self, p: &[f64], theta: f64) -> f64 {
                match &self.inner {
                    $(DistortionModels::$name(m) => m.distort_for_light_refraction(p, theta),)*
                }
            }
            pub fn undistort_for_light_refraction_gradient(&self, p: &[f64], theta: f64) -> Vec<f64> {
                match &self.inner {
                    $(DistortionModels::$name(m) => m.undistort_for_light_refraction_gradient(p, theta),)*
                }
            }

            pub fn id(&self)               -> &'static str { match &self.inner { $(DistortionModels::$name(_) => <$class>::id(),)* } }
            pub fn name(&self)             -> &'static str { match &self.inner { $(DistortionModels::$name(_) => <$class>::name(),)* } }
            pub fn opencl_functions(&self) -> &'static str { match &self.inner { $(DistortionModels::$name(x) => x.opencl_functions(),)* } }
            pub fn wgsl_functions(&self)   -> &'static str { match &self.inner { $(DistortionModels::$name(x) => x.wgsl_functions(),)* } }

            pub fn from_name(id: &str) -> Self {
                $(
                    if <$class>::id() == id { return Self { inner: DistortionModels::$name(Default::default()) }; }
                )*
                DistortionModel::default()
            }
        }
    };
}

impl_models! {
    // Physical lenses
    OpenCVFisheye  => opencv_fisheye::OpenCVFisheye,
    OpenCVStandard => opencv_standard::OpenCVStandard,
    Poly3          => poly3::Poly3,
    Poly5          => poly5::Poly5,
    PtLens         => ptlens::PtLens,
    Insta360       => insta360::Insta360,

    // Digital lenses (ie. post-processing)
    GoProSuperview => gopro_superview::GoProSuperview,
    GoProHyperview => gopro_hyperview::GoProHyperview,
    DigitalStretch => digital_stretch::DigitalStretch,
}
