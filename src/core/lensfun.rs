// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Gyroflow contributors

//! Import lens profiles from the Lensfun database (<https://lensfun.github.io/>).
//!
//! Lensfun stores distortion calibrations as polynomial coefficients (`poly3`, `poly5`, `ptlens`)
//! normalized to the Hugin convention, together with the crop factor and aspect ratio of the
//! sensor used for calibration. To use them in Gyroflow they have to be:
//! 1. Interpolated to the requested focal length (Hermite spline, like Lensfun does),
//! 2. Rescaled from Hugin normalization to focal-length normalization (`rescale_coeffs`),
//! 3. Combined with a camera matrix derived from the real focal length and the pixel pitch
//!    of the target sensor (which is where the target camera's crop factor comes in).
//!
//! The math is a direct port of Lensfun:
//! - `rescale_polynomial_coefficients` in `libs/lensfun/mod-coord.cpp`
//! - `lfLens::InterpolateDistortion` and `__insert_spline` in `libs/lensfun/lens.cpp`
//! - `_lf_interpolate` in `libs/lensfun/auxfun.cpp`
//! - `NormScale` in `lfModifier::lfModifier` in `libs/lensfun/modifier.cpp`

use crate::LensProfile;
use crate::lens_profile::{ CameraParams, Dimensions };
use crate::stabilization::distortion_models::DistortionModel;

/// Diagonal of a full-frame 35mm sensor (36x24mm), used by Lensfun as crop factor reference
const FULL_FRAME_DIAGONAL: f64 = 43.266615305567875; // hypot(36, 24)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LensfunDistortionModel {
    Poly3,
    Poly5,
    PtLens,
}
impl LensfunDistortionModel {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "poly3"  => Some(Self::Poly3),
            "poly5"  => Some(Self::Poly5),
            "ptlens" => Some(Self::PtLens),
            _ => None
        }
    }
    /// Id of the matching Gyroflow distortion model
    pub fn id(&self) -> &'static str {
        match self {
            Self::Poly3  => "poly3",
            Self::Poly5  => "poly5",
            Self::PtLens => "ptlens",
        }
    }
    pub fn num_terms(&self) -> usize {
        match self {
            Self::Poly3  => 1,
            Self::Poly5  => 2,
            Self::PtLens => 3,
        }
    }
}

/// Sensor attributes of the camera used to calibrate a set of measurements
#[derive(Debug, Clone, Copy)]
pub struct CalibrationAttributes {
    pub crop_factor: f64,
    pub aspect_ratio: f64,
}

#[derive(Debug, Clone)]
pub struct LensfunDistortion {
    pub model: LensfunDistortionModel,
    pub focal: f64,
    /// Actual focal length of the image after distortion correction.
    /// If not measured, Lensfun derives it from the polynomial: `focal * (1 - a - b - c)` for
    /// ptlens, `focal * (1 - k1)` for poly3 (see `database.cpp` in Lensfun)
    pub real_focal: f64,
    pub terms: [f64; 3],
}

#[derive(Debug, Clone)]
pub struct LensfunCalibrationSet {
    pub attributes: CalibrationAttributes,
    pub distortions: Vec<LensfunDistortion>,
}

#[derive(Debug, Clone)]
pub struct LensfunCamera {
    pub maker: String,
    pub model: String,
    pub variant: String,
    pub mount: String,
    pub crop_factor: f64,
}

#[derive(Debug, Clone)]
pub struct LensfunLens {
    pub maker: String,
    pub model: String,
    pub mounts: Vec<String>,
    pub crop_factor: f64,
    pub aspect_ratio: f64,
    pub calibrations: Vec<LensfunCalibrationSet>,
}

#[derive(Debug, Default, Clone)]
pub struct LensfunDatabase {
    pub cameras: Vec<LensfunCamera>,
    pub lenses: Vec<LensfunLens>,
}

impl LensfunDatabase {
    pub fn parse(xml: &str) -> Result<Self, roxmltree::Error> {
        let doc = roxmltree::Document::parse(xml)?;
        let root = doc.root_element();
        if root.tag_name().name() != "lensdatabase" {
            return Err(roxmltree::Error::NoRootNode);
        }
        let mut db = Self::default();
        for node in root.children().filter(|x| x.is_element()) {
            match node.tag_name().name() {
                "camera" => {
                    db.cameras.push(LensfunCamera {
                        maker:       child_text(&node, "maker").unwrap_or_default(),
                        model:       child_text(&node, "model").unwrap_or_default(),
                        variant:     child_text(&node, "variant").unwrap_or_default(),
                        mount:       child_text(&node, "mount").unwrap_or_default(),
                        crop_factor: child_text(&node, "cropfactor").and_then(|x| x.parse().ok()).unwrap_or(1.0),
                    });
                }
                "lens" => {
                    // Defaults are from `_xml_start_element` in Lensfun's database.cpp
                    let crop_factor = child_text(&node, "cropfactor").and_then(|x| x.parse().ok()).unwrap_or(1.0);
                    let aspect_ratio = child_text(&node, "aspect-ratio").map(|x| parse_aspect_ratio(&x)).unwrap_or(1.5);
                    let lens_attrs = CalibrationAttributes { crop_factor, aspect_ratio };

                    let mut lens = LensfunLens {
                        maker: child_text(&node, "maker").unwrap_or_default(),
                        model: child_text(&node, "model").unwrap_or_default(),
                        mounts: node.children().filter(|x| x.has_tag_name("mount")).filter_map(|x| x.text().map(|x| x.trim().to_string())).collect(),
                        crop_factor,
                        aspect_ratio,
                        calibrations: Vec::new(),
                    };
                    for calib in node.children().filter(|x| x.has_tag_name("calibration")) {
                        let attributes = CalibrationAttributes {
                            crop_factor:  calib.attribute("cropfactor")  .and_then(|x| x.parse().ok()).unwrap_or(lens_attrs.crop_factor),
                            aspect_ratio: calib.attribute("aspect-ratio").and_then(|x| x.parse().ok()).unwrap_or(lens_attrs.aspect_ratio),
                        };
                        let mut distortions = Vec::new();
                        for dist in calib.children().filter(|x| x.has_tag_name("distortion")) {
                            let Some(model) = dist.attribute("model").and_then(LensfunDistortionModel::from_name) else { continue; };
                            let attr = |names: &[&str]| -> f64 {
                                names.iter().find_map(|n| dist.attribute(*n)).and_then(|x| x.parse().ok()).unwrap_or(0.0)
                            };
                            let focal = attr(&["focal"]);
                            if focal <= 0.0 { continue; }
                            let terms = [attr(&["a", "k1"]), attr(&["b", "k2"]), attr(&["c", "k3"])];
                            let mut real_focal = attr(&["real-focal"]);
                            if real_focal <= 0.0 {
                                real_focal = match model {
                                    LensfunDistortionModel::PtLens => focal * (1.0 - terms[0] - terms[1] - terms[2]),
                                    LensfunDistortionModel::Poly3  => focal * (1.0 - terms[0]),
                                    _ => focal
                                };
                            }
                            distortions.push(LensfunDistortion { model, focal, real_focal, terms });
                        }
                        if !distortions.is_empty() {
                            lens.calibrations.push(LensfunCalibrationSet { attributes, distortions });
                        }
                    }
                    if !lens.calibrations.is_empty() {
                        db.lenses.push(lens);
                    }
                }
                _ => { }
            }
        }
        Ok(db)
    }

    /// Convert every calibrated focal length of every lens to a Gyroflow lens profile
    pub fn to_lens_profiles(&self) -> Vec<LensProfile> {
        let mut ret = Vec::new();
        for lens in &self.lenses {
            for calib in &lens.calibrations {
                for dist in &calib.distortions {
                    // Target camera defaults to the calibration sensor.
                    // Videos with a different aspect ratio are treated as a sensor crop,
                    // which is consistent with how Gyroflow handles `compatible_settings`
                    let dims = dimensions_for_aspect(calib.attributes.aspect_ratio);
                    if let Some(profile) = lens.to_lens_profile(dist.focal, calib.attributes.crop_factor, dims) {
                        ret.push(profile);
                    }
                }
            }
        }
        ret
    }
}

impl LensfunLens {
    /// Interpolate distortion coefficients at `focal`, for a camera with crop factor `camera_crop`.
    /// Direct port of `lfLens::InterpolateDistortion` in Lensfun's lens.cpp
    pub fn interpolate_distortion(&self, camera_crop: f64, focal: f64) -> Option<(LensfunDistortion, CalibrationAttributes)> {
        // Find the calibration set with the closest crop factor.
        // Calibrations from a sensor smaller than the target (ratio < 0.96) can't be used,
        // because they don't cover the whole target sensor
        let mut calib_set: Option<&LensfunCalibrationSet> = None;
        let mut crop_ratio = 1e6;
        for c in &self.calibrations {
            let r = camera_crop / c.attributes.crop_factor;
            if !c.distortions.is_empty() && r >= 0.96 && r < crop_ratio {
                crop_ratio = r;
                calib_set = Some(c);
            }
        }
        let calib_set = calib_set?;

        // Take into account just the first encountered model
        let model = calib_set.distortions.first()?.model;

        // 2 nearest calibrated focals below and 2 above the requested one
        let mut spline: [Option<&LensfunDistortion>; 4] = [None; 4];
        let mut spline_dist = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];
        for c in &calib_set.distortions {
            if c.model != model { continue; }
            let df = focal - c.focal;
            if df == 0.0 {
                // Exact match found, don't care to interpolate
                return Some((c.clone(), calib_set.attributes));
            }
            if df < 0.0 {
                if df > spline_dist[1] {
                    spline_dist[0] = spline_dist[1];
                    spline_dist[1] = df;
                    spline[0] = spline[1];
                    spline[1] = Some(c);
                } else if df > spline_dist[0] {
                    spline_dist[0] = df;
                    spline[0] = Some(c);
                }
            } else {
                if df < spline_dist[2] {
                    spline_dist[3] = spline_dist[2];
                    spline_dist[2] = df;
                    spline[3] = spline[2];
                    spline[2] = Some(c);
                } else if df < spline_dist[3] {
                    spline_dist[3] = df;
                    spline[3] = Some(c);
                }
            }
        }

        let (s1, s2) = match (spline[1], spline[2]) {
            (Some(s1), Some(s2)) => (s1, s2),
            // Requested focal is outside of the calibrated range, clamp to the nearest entry
            (Some(s), None) | (None, Some(s)) => return Some((s.clone(), calib_set.attributes)),
            (None, None) => return None
        };

        let t = (focal - s1.focal) / (s2.focal - s1.focal);

        let real_focal = hermite_interpolate(spline[0].map(|x| x.real_focal), s1.real_focal, s2.real_focal, spline[3].map(|x| x.real_focal), t);
        let mut terms = [0.0; 3];
        for i in 0..terms.len() {
            // The parameters are approximately proportional to the inverse focal length,
            // so interpolate `term * focal` (see `__parameter_scales` in Lensfun's lens.cpp)
            terms[i] = hermite_interpolate(
                spline[0].map(|x| x.terms[i] * x.focal),
                s1.terms[i] * s1.focal,
                s2.terms[i] * s2.focal,
                spline[3].map(|x| x.terms[i] * x.focal),
                t
            ) / focal;
        }

        Some((LensfunDistortion { model, focal, real_focal, terms }, calib_set.attributes))
    }

    /// Create a Gyroflow lens profile for this lens at the given focal length,
    /// for a camera with the given crop factor, at the given pixel dimensions
    pub fn to_lens_profile(&self, focal: f64, camera_crop: f64, dimensions: (usize, usize)) -> Option<LensProfile> {
        let (dist, calib_attr) = self.interpolate_distortion(camera_crop, focal)?;

        let model = DistortionModel::from_name(dist.model.id());

        // Rescale the coefficients from Hugin normalization (radius = 1 at half of the smaller
        // sensor dimension of the calibration sensor) to focal length normalization
        // (radius = tan(angle), which is what Gyroflow kernels work in).
        // See `rescale_polynomial_coefficients` in Lensfun's mod-coord.cpp
        let hugin_scale_in_millimeters = FULL_FRAME_DIAGONAL / calib_attr.crop_factor / calib_attr.aspect_ratio.hypot(1.0) / 2.0;
        let hugin_scaling = dist.real_focal / hugin_scale_in_millimeters;
        let mut coeffs = dist.terms[..dist.model.num_terms()].to_vec();
        model.rescale_coeffs(&mut coeffs, hugin_scaling);

        let (w, h) = dimensions;
        if w == 0 || h == 0 || camera_crop <= 0.0 { return None; }

        // Pixel pitch of the target sensor in mm.
        // The sensor size is given for the outer rim of the pixel array, which spans
        // exactly `w` x `h` pixel pitches (see `lfModifier::lfModifier` in Lensfun's modifier.cpp)
        let pixel_pitch = (FULL_FRAME_DIAGONAL / camera_crop) / (w as f64).hypot(h as f64);
        let focal_px = dist.real_focal / pixel_pitch;

        let mut profile = LensProfile::default();
        profile.calibrated_by = "Lensfun".into();
        profile.camera_brand = self.maker.clone();
        profile.camera_model = self.model.strip_prefix(self.maker.as_str()).map(|x| x.trim().to_string()).unwrap_or_else(|| self.model.clone());
        profile.lens_model = format!("{:.4}", focal).trim_end_matches('0').trim_end_matches('.').to_string() + "mm";
        profile.note = "Imported from the Lensfun database".into();
        profile.calib_dimension = Dimensions { w, h };
        profile.orig_dimension  = Dimensions { w, h };
        profile.input_horizontal_stretch = 1.0;
        profile.input_vertical_stretch = 1.0;
        profile.fisheye_params = CameraParams {
            RMS_error: 0.0,
            camera_matrix: vec![
                [focal_px, 0.0, w as f64 / 2.0],
                [0.0, focal_px, h as f64 / 2.0],
                [0.0, 0.0, 1.0]
            ],
            distortion_coeffs: coeffs,
            radial_distortion_limit: None
        };
        profile.identifier = format!("lensfun-{}-{}-{}mm-crop{:.2}", self.maker, self.model, dist.focal, camera_crop)
            .to_lowercase().replace(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '.', "-");
        profile.calibrator_version = env!("CARGO_PKG_VERSION").to_string();
        profile.distortion_model = Some(dist.model.id().to_string());
        profile.focal_length = Some(dist.focal);
        profile.crop_factor = Some(camera_crop);
        profile.name = profile.get_name();
        profile.init();
        Some(profile)
    }
}

fn child_text(node: &roxmltree::Node, tag: &str) -> Option<String> {
    // Prefer the non-localized variant, like Lensfun does
    let mut children = node.children().filter(|x| x.has_tag_name(tag));
    children.clone().find(|x| !x.has_attribute("lang")).or_else(|| children.next())
        .and_then(|x| x.text().map(|x| x.trim().to_string()))
}

/// Aspect ratio can be either a decimal number or in "3:2" form (see `_xml_text` in Lensfun's database.cpp)
fn parse_aspect_ratio(text: &str) -> f64 {
    match text.split_once(':') {
        Some((a, b)) => {
            let a: f64 = a.trim().parse().unwrap_or(1.5);
            let b: f64 = b.trim().parse().unwrap_or(1.0);
            if b != 0.0 { a / b } else { 1.5 }
        }
        None => text.trim().parse().unwrap_or(1.5)
    }
}

/// Synthesize even pixel dimensions matching the given sensor aspect ratio
fn dimensions_for_aspect(aspect_ratio: f64) -> (usize, usize) {
    let w = 3840usize;
    let mut h = (w as f64 / aspect_ratio.max(0.1)).round() as usize;
    if h % 2 != 0 { h -= 1; }
    (w, h)
}

/// Hermite spline interpolation, direct port of `_lf_interpolate` in Lensfun's auxfun.cpp.
/// `y1`/`y4` are the values around the interpolated segment `y2`..`y3` and can be missing
fn hermite_interpolate(y1: Option<f64>, y2: f64, y3: f64, y4: Option<f64>, t: f64) -> f64 {
    let t2 = t * t;
    let t3 = t2 * t;

    let tg2 = match y1 { Some(y1) => (y3 - y1) * 0.5, None => y3 - y2 };
    let tg3 = match y4 { Some(y4) => (y4 - y2) * 0.5, None => y3 - y2 };

    (2.0 * t3 - 3.0 * t2 + 1.0) * y2 +
        (t3 - 2.0 * t2 + t) * tg2 +
        (-2.0 * t3 + 3.0 * t2) * y3 +
        (t3 - t2) * tg3
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference values in these tests were generated by running actual Lensfun
    // (git master, e78e7be4+) on the same XML through `lfLens::InterpolateDistortion` and
    // `lfModifier::ApplyGeometryDistortion` with a 4000x2666 image and crop factor 1.5

    const TEST_XML: &str = r#"<lensdatabase version="2">
        <mount><name>TestMount</name></mount>
        <camera>
            <maker>TestMaker</maker>
            <model>TestCam15</model>
            <mount>TestMount</mount>
            <cropfactor>1.5</cropfactor>
        </camera>
        <lens>
            <maker>TestMaker</maker>
            <model>Test PTLens Zoom 18-55mm</model>
            <mount>TestMount</mount>
            <cropfactor>1.5</cropfactor>
            <calibration>
                <distortion model="ptlens" focal="18" a="0.012" b="-0.035" c="0.002" />
                <distortion model="ptlens" focal="24" a="0.008" b="-0.021" c="0.001" />
                <distortion model="ptlens" focal="35" a="0.004" b="-0.009" c="0.0005" />
                <distortion model="ptlens" focal="55" a="0.001" b="-0.002" c="0.0" />
            </calibration>
        </lens>
        <lens>
            <maker>TestMaker</maker>
            <model>Test Poly3 30mm</model>
            <mount>TestMount</mount>
            <cropfactor>1.0</cropfactor>
            <aspect-ratio>3:2</aspect-ratio>
            <calibration>
                <distortion model="poly3" focal="30" k1="-0.015" />
            </calibration>
        </lens>
        <lens>
            <maker>TestMaker</maker>
            <model>Test Poly5 12mm</model>
            <mount>TestMount</mount>
            <cropfactor>1.5</cropfactor>
            <calibration>
                <distortion model="poly5" focal="12" k1="-0.021" k2="0.0033" />
            </calibration>
        </lens>
    </lensdatabase>"#;

    fn db() -> LensfunDatabase { LensfunDatabase::parse(TEST_XML).unwrap() }

    #[test]
    fn test_parse() {
        let db = db();
        assert_eq!(db.cameras.len(), 1);
        assert_eq!(db.cameras[0].crop_factor, 1.5);
        assert_eq!(db.lenses.len(), 3);

        let zoom = &db.lenses[0];
        assert_eq!(zoom.calibrations.len(), 1);
        assert_eq!(zoom.calibrations[0].distortions.len(), 4);
        // real_focal derived from the polynomial: 18 * (1 - 0.012 + 0.035 - 0.002)
        assert!((zoom.calibrations[0].distortions[0].real_focal - 18.378).abs() < 1e-9);

        let poly3 = &db.lenses[1];
        assert_eq!(poly3.crop_factor, 1.0);
        assert_eq!(poly3.aspect_ratio, 1.5);
        // real_focal derived from the polynomial: 30 * (1 - (-0.015))
        assert!((poly3.calibrations[0].distortions[0].real_focal - 30.45).abs() < 1e-9);
    }

    #[test]
    fn test_hermite_interpolation() {
        let db = db();
        let (dist, attr) = db.lenses[0].interpolate_distortion(1.5, 28.0).unwrap();
        // Reference from lfLens::InterpolateDistortion at 28mm (between 24mm and 35mm entries)
        assert!((dist.terms[0] -  0.00630503381).abs()  < 1e-7, "a = {}", dist.terms[0]);
        assert!((dist.terms[1] - -0.0157351606).abs()   < 1e-7, "b = {}", dist.terms[1]);
        assert!((dist.terms[2] -  0.000774793385).abs() < 1e-7, "c = {}", dist.terms[2]);
        assert!((dist.real_focal - 27.4955425).abs()    < 1e-5, "real_focal = {}", dist.real_focal);
        assert_eq!(attr.crop_factor, 1.5);
    }

    #[test]
    fn test_exact_focal_match() {
        let db = db();
        let (dist, _) = db.lenses[0].interpolate_distortion(1.5, 24.0).unwrap();
        assert_eq!(dist.terms[0], 0.008);
        assert_eq!(dist.terms[1], -0.021);
        assert_eq!(dist.terms[2], 0.001);
    }

    #[test]
    fn test_crop_factor_selection() {
        let db = db();
        // Calibration on a full-frame sensor (crop 1.0) is usable on a 1.5 crop camera...
        assert!(db.lenses[1].interpolate_distortion(1.5, 30.0).is_some());
        // ...but a calibration on a 1.5 crop sensor is not usable on a full-frame camera
        assert!(db.lenses[0].interpolate_distortion(1.0, 18.0).is_none());
    }

    fn check_distortion_map(profile: &LensProfile, expected: &[((f32, f32), (f32, f32))]) {
        let model = DistortionModel::from_name(profile.distortion_model.as_deref().unwrap());
        let mut params = crate::stabilization::KernelParams::default();
        for (i, k) in profile.fisheye_params.distortion_coeffs.iter().enumerate() {
            params.k[i] = *k as f32;
        }
        let m = &profile.fisheye_params.camera_matrix;
        let (fx, fy) = (m[0][0] as f32, m[1][1] as f32);
        // Lensfun addresses pixel centres, so its optical center is at (w-1)/2, not w/2.
        // The profile itself stores w/2 which is the Gyroflow convention
        // (`get_camera_matrix` forces it for non-asymmetrical profiles)
        let (cx, cy) = (m[0][2] as f32 - 0.5, m[1][2] as f32 - 0.5);
        for ((px, py), (ex, ey)) in expected {
            let (xd, yd) = model.distort_point((px - cx) / fx, (py - cy) / fy, 1.0, &params);
            let (ox, oy) = (xd * fx + cx, yd * fy + cy);
            assert!((ox - ex).abs() < 0.005 && (oy - ey).abs() < 0.005,
                "({}, {}) -> ({}, {}), expected ({}, {})", px, py, ox, oy, ex, ey);
        }
    }

    #[test]
    fn test_matches_lensfun_ptlens() {
        // Reference from lfModifier::ApplyGeometryDistortion, image 4000x2666, crop 1.5
        let profile = db().lenses[0].to_lens_profile(18.0, 1.5, (4000, 2666)).unwrap();
        assert_eq!(profile.distortion_model.as_deref(), Some("ptlens"));
        check_distortion_map(&profile, &[
            ((2000.0, 1333.0), (2000.000000, 1333.000000)),
            ((3000.0, 1333.0), (2987.581543, 1332.993774)),
            ((2000.0, 2000.0), (1999.997070, 1996.063965)),
            ((3500.0, 2400.0), (3453.502930, 2366.920654)),
            (( 200.0,  150.0), ( 265.065338,  192.756180)),
            ((3999.0, 2665.0), (3921.596924, 2613.417236)),
            ((   0.0,    0.0), (  77.403191,   51.582760)),
        ]);
    }

    #[test]
    fn test_matches_lensfun_ptlens_interpolated() {
        let profile = db().lenses[0].to_lens_profile(28.0, 1.5, (4000, 2666)).unwrap();
        check_distortion_map(&profile, &[
            ((2000.0, 1333.0), (2000.000000, 1333.000000)),
            ((3000.0, 1333.0), (2994.507812, 1332.997314)),
            ((2000.0, 2000.0), (1999.998657, 1998.199219)),
            ((3500.0, 2400.0), (3481.769043, 2387.030029)),
            (( 200.0,  150.0), ( 223.582275,  165.496429)),
            ((3999.0, 2665.0), (3973.458252, 2647.978271)),
            ((   0.0,    0.0), (  25.541855,   17.021618)),
        ]);
    }

    #[test]
    fn test_matches_lensfun_poly3_crop_mismatch() {
        // Lens calibrated on a full-frame sensor (crop 1.0), used on a 1.5 crop camera
        let profile = db().lenses[1].to_lens_profile(30.0, 1.5, (4000, 2666)).unwrap();
        assert_eq!(profile.distortion_model.as_deref(), Some("poly3"));
        check_distortion_map(&profile, &[
            ((2000.0, 1333.0), (2000.000000, 1333.000000)),
            ((3000.0, 1333.0), (2996.407715, 1332.998169)),
            ((2000.0, 2000.0), (1999.999146, 1998.933228)),
            ((3500.0, 2400.0), (3481.749512, 2387.016113)),
            (( 200.0,  150.0), ( 229.925522,  169.664902)),
            ((3999.0, 2665.0), (3957.593750, 2637.406250)),
            ((   0.0,    0.0), (  41.406223,   27.593697)),
        ]);
    }

    #[test]
    fn test_matches_lensfun_poly5() {
        let profile = db().lenses[2].to_lens_profile(12.0, 1.5, (4000, 2666)).unwrap();
        assert_eq!(profile.distortion_model.as_deref(), Some("poly5"));
        check_distortion_map(&profile, &[
            ((2000.0, 1333.0), (2000.000000, 1333.000000)),
            ((3000.0, 1333.0), (2989.215088, 1332.994629)),
            ((2000.0, 2000.0), (1999.997437, 1996.624756)),
            ((3500.0, 2400.0), (3457.907471, 2370.054199)),
            (( 200.0,  150.0), ( 258.167297,  188.223312)),
            ((3999.0, 2665.0), (3932.227539, 2620.501709)),
            ((   0.0,    0.0), (  66.772438,   44.498238)),
        ]);
    }

    #[test]
    fn test_to_lens_profiles() {
        let profiles = db().to_lens_profiles();
        assert_eq!(profiles.len(), 6); // 4 zoom focals + poly3 + poly5
        for p in &profiles {
            assert!(!p.identifier.is_empty());
            assert!(!p.fisheye_params.distortion_coeffs.is_empty());
            assert!(p.fisheye_params.camera_matrix.len() == 3);
        }
    }
}
