// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

mod opencv_fisheye;
mod opencv_standard;
pub mod poly3;
pub mod poly5;
pub mod ptlens;
mod insta360;
pub mod sony;
mod generic_polynomial;
mod gopro;

mod gopro_superview;
mod gopro6_superview;
mod gopro_hyperview;
mod gopro_warp;
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
            /// `d(image radius)/dθ` at ray angle `theta`, ≤ 0 where the model's curve folds back
            pub fn distortion_derivative(&self, theta: f64, k: &[f64]) -> Option<f64> {
                match &self.inner {
                    $(DistortionModels::$name(x) => x.distortion_derivative(theta, k),)*
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
    Insta360          => insta360::Insta360,
    Sony              => sony::Sony,
    GenericPolynomial => generic_polynomial::GenericPolynomial,
    GoPro             => gopro::GoPro,

    // Digital lenses (ie. post-processing)
    GoProSuperview  => gopro_superview::GoProSuperview,
    GoPro6Superview => gopro6_superview::GoPro6Superview,
    GoProHyperview  => gopro_hyperview::GoProHyperview,
    GoProWarp       => gopro_warp::GoProWarp,
    DigitalStretch  => digital_stretch::DigitalStretch,
}

impl DistortionModel {
    /// Largest usable ray radius `tan θ` before the curve folds back (its derivative stops being positive),
    /// `None` when it rises all the way to 90° or the model has no derivative. The generic way samples the
    /// derivative up to 90° and bisects the first non-positive step; the Sony spline solves its fold from the
    /// coefficients, since each of its derivative samples would be a Newton solve and the renderer asks per frame
    /// when the file records a lens curve per frame
    pub fn radial_distortion_limit(&self, k: &[f64]) -> Option<f64> {
        if let DistortionModels::Sony(m) = &self.inner {
            return m.radial_distortion_limit(k);
        }
        let max_theta = std::f64::consts::FRAC_PI_2; // PI/2

        const STEPS: usize = 256;
        let mut low = 0.0;
        let mut high = max_theta;
        let mut found = false;
        for i in 1..=STEPS {
            let theta = i as f64 / STEPS as f64 * max_theta;
            if self.distortion_derivative(theta, k)? <= 0.0 {
                high = theta;
                found = true;
                break;
            }
            low = theta;
        }
        if !found { return None; }

        while high - low > 1e-6 {
            let mid = (low + high) / 2.0;
            if self.distortion_derivative(mid, k)? > 0.0 {
                low = mid;
            } else {
                high = mid;
            }
        }

        let theta_max = (low + high) / 2.0;
        if (theta_max - max_theta).abs() > 0.001 {
            Some(theta_max.tan())
        } else {
            None
        }
    }

    /// Whether `undistort_point` keeps every point on its ray from the principal point, ie. the map is a pure
    /// function of the radius. The lens-correction solve in `cpu_undistort::invert_lens_correction_blend`
    /// bisects along that ray when it is, and has to solve in two dimensions when it isn't: tangential and
    /// thin-prism terms move a point off its ray, and the digital warps are not radial at all
    pub fn is_radial(&self, params: &KernelParams) -> bool {
        use DistortionModels as M;
        match &self.inner {
            M::OpenCVFisheye(_) | M::Poly3(_) | M::Poly5(_) | M::PtLens(_) | M::Sony(_) | M::GenericPolynomial(_) | M::GoPro(_) => true,
            // k = [k1 k2 p1 p2 k3 k4 k5 k6 s1 s2 s3 s4 ...]: p1 p2 tangential, s1..s4 thin prism
            M::OpenCVStandard(_) => params.k[2] == 0.0 && params.k[3] == 0.0 && params.k[8..12].iter().all(|k| *k == 0.0),
            // k = [k1 k2 k3 p1 p2 xi]
            M::Insta360(_) => params.k[3] == 0.0 && params.k[4] == 0.0,
            M::GoProSuperview(_) | M::GoPro6Superview(_) | M::GoProHyperview(_) | M::GoProWarp(_) | M::DigitalStretch(_) => false,
        }
    }
}
