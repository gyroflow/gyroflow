// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

mod opencv_fisheye;
mod gopro_superview;
pub use gopro_superview::GoProSuperview;

macro_rules! impl_models {
    ($($name:ident => $class:ty,)*) => {
        #[derive(Clone)]
        pub enum DistortionModels {
            $($name($class),)*
        }
        impl Default for DistortionModels {
            fn default() -> Self { DistortionModels::OpenCVFisheye(opencv_fisheye::OpenCVFisheye { }) }
        }
        #[derive(Default, Clone)]
        pub struct DistortionModel {
            inner: DistortionModels
        }
        impl DistortionModel {
            pub fn undistort_point<T: num_traits::Float>(&self, point: (T, T), k: &[T], amount: T) -> Option<(T, T)> {
                match &self.inner {
                    $(DistortionModels::$name(x) => x.undistort_point(point, k, amount),)*
                }
            }
            pub fn distort_point<T: num_traits::Float>(&self, point: (T, T), k: &[T], amount: T) -> (T, T) {
                match &self.inner {
                    $(DistortionModels::$name(x) => x.distort_point(point, k, amount),)*
                }
            }

            pub fn id(&self)               -> i32          { match &self.inner { $(DistortionModels::$name(x) => x.id(),)* } }
            pub fn name(&self)             -> &'static str { match &self.inner { $(DistortionModels::$name(x) => x.name(),)* } }
            pub fn opencl_functions(&self) -> &'static str { match &self.inner { $(DistortionModels::$name(x) => x.opencl_functions(),)* } }
            pub fn wgsl_functions(&self)   -> &'static str { match &self.inner { $(DistortionModels::$name(x) => x.wgsl_functions(),)* } }
            pub fn glsl_shader_path(&self) -> &'static str { match &self.inner { $(DistortionModels::$name(x) => x.glsl_shader_path(),)* } }

            pub fn from_id(_id: i32) -> Self {
                // TODO
                DistortionModel::default()
            }
        }
    };
}

impl_models! {
    OpenCVFisheye => opencv_fisheye::OpenCVFisheye,
}
