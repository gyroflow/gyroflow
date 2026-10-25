// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

mod opencv_fisheye;
mod opencv_standard;
mod poly3;
mod poly5;
mod ptlens;
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

// The models a chessboard calibration can fit (`calibration::solver`), which calls their `distort` directly
pub use self::{ opencv_fisheye::OpenCVFisheye, opencv_standard::OpenCVStandard, generic_polynomial::GenericPolynomial };
pub use self::{ poly3::Poly3, poly5::Poly5, ptlens::PtLens };

/// The float types a model's math is written for: `f32` in the render, `f64` in the calibration solver, whose
/// Jacobian is a finite difference that single precision would drown. A model writes its projection once, generic
/// over this, and `distort_point` calls it with `f32`.
///
/// The functions are std's own, so the render's `f32` math is what it was before it was generic. nalgebra's
/// `RealField` would route them through `libm` here, because `ahrs` builds nalgebra with `libm-force`: an `atan2`
/// a few ulp off std's, and a fisheye `distort_point` about twice as slow in a release build
pub trait Real: Copy + PartialOrd
    + std::ops::Add<Output = Self> + std::ops::Sub<Output = Self> + std::ops::Mul<Output = Self> + std::ops::Div<Output = Self>
{
    fn from_f64(v: f64) -> Self;
    fn sqrt(self) -> Self;
    fn atan2(self, x: Self) -> Self;
}
macro_rules! impl_real {
    ($($t:ty),*) => {$(
        impl Real for $t {
            #[inline(always)] fn from_f64(v: f64) -> Self { v as $t }
            #[inline(always)] fn sqrt(self) -> Self { <$t>::sqrt(self) }
            #[inline(always)] fn atan2(self, x: Self) -> Self { <$t>::atan2(self, x) }
        }
    )*};
}
impl_real!(f32, f64);

/// A constant in the model's precision
#[inline(always)]
pub fn c<T: Real>(v: f64) -> T { T::from_f64(v) }

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
            /// `d(image radius)/dÎ¸` at ray angle `theta`, â‰¤ 0 where the model's curve folds back
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
    /// The coefficients of the image mirrored vertically (`y` -> `-y`, a framebuffer upside down): the terms odd in
    /// `y` change sign, the tangential `p1` and the thin prism of the `y` offset. Radial models are symmetric
    pub fn mirror_coefficients_y(&self, k: &mut [f64]) {
        use DistortionModels as M;
        let odd: &[usize] = match &self.inner {
            M::OpenCVStandard(_) => &[2, 10, 11], // p1, s3, s4
            M::Insta360(_) => &[3],               // p1
            _ => &[],
        };
        for &i in odd {
            if let Some(v) = k.get_mut(i) { *v = -*v; }
        }
    }

    /// The largest ray angle the model itself can represent, in radians, `None` when it has no such end.
    /// Not a fold of the calibration - [`Self::radial_distortion_limit`] finds those - but the shape of the
    /// projection: a rectilinear model images the z=1 plane and has nothing at 90°, and the unified sphere
    /// of the Insta360 model turns over at `acos(-1/Î¾)`
    pub fn field_limit(&self, k: &[f64]) -> Option<f64> {
        use DistortionModels as M;
        match &self.inner {
            M::OpenCVStandard(_) | M::Poly3(_) | M::Poly5(_) | M::PtLens(_) => Some(std::f64::consts::FRAC_PI_2 - 1e-4),
            M::Insta360(_) => insta360::Insta360::field_limit(k),
            _ => None
        }
    }

    /// Largest usable ray angle (radians) before the curve folds back (its derivative stops being positive),
    /// `None` when it rises all the way. The generic way samples the derivative up to 180° and bisects the
    /// first non-positive step; the Sony spline solves its fold from the coefficients, since each of its
    /// derivative samples would be a Newton solve and the renderer asks per frame when the file records a
    /// lens curve per frame
    pub fn radial_distortion_limit(&self, k: &[f64]) -> Option<f64> {
        if let DistortionModels::Sony(m) = &self.inner {
            return m.radial_distortion_limit(k);
        }
        // A fisheye can see well past 90°, so the sweep runs to 180° - the pipeline carries rays as angles
        // and no longer stops at the old z=1 plane asymptote
        let max_theta = std::f64::consts::PI;

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
            Some(theta_max)
        } else {
            None
        }
    }
}
