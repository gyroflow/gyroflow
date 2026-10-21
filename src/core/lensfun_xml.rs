// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Contributors

//! Parser for the Lensfun XML lens database format (https://lensfun.github.io/).
//!
//! Lensfun stores distortion calibration as `poly3`/`poly5`/`ptlens` coefficients
//! (the same models gyroflow already implements in `stabilization::distortion_models`),
//! but those coefficients are normalized to the calibration's own "hugin scale" —
//! derived from the lens' crop factor, aspect ratio and (real) focal length. This
//! module parses the XML and rescales the coefficients into gyroflow's convention,
//! following the same math gyroflow's ptlens/poly3/poly5 models were ported from:
//! https://github.com/lensfun/lensfun/blob/master/libs/lensfun/mod-coord.cpp

use crate::LensProfile;
use crate::lens_profile::{ Dimensions, CameraParams };
use crate::stabilization::distortion_models::{ Poly3, Poly5, PtLens };
use xml::reader::{ EventReader, XmlEvent };

/// Full-frame (36x24mm) sensor diagonal in millimeters.
const FULL_FRAME_DIAGONAL_MM: f64 = 43.266615305567875;
/// Lensfun's default aspect ratio when `<aspect-ratio>` is not specified.
const DEFAULT_ASPECT_RATIO: f64 = 1.5; // 3:2

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LensfunDistortion {
    pub model: String,
    pub focal: f64,
    pub real_focal: Option<f64>,
    /// Coefficients in the model's own order: `[k1]` for poly3, `[k1, k2]` for poly5, `[a, b, c]` for ptlens.
    pub k: Vec<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LensfunLens {
    pub maker: String,
    pub model: String,
    pub mounts: Vec<String>,
    pub crop_factor: f64,
    pub aspect_ratio: f64,
    pub distortions: Vec<LensfunDistortion>,
}

/// Parse an `<aspect-ratio>` value, which is either a plain number (`1.5`) or a `w:h` ratio (`3:2`).
fn parse_aspect_ratio(s: &str) -> Option<f64> {
    if let Some((w, h)) = s.split_once(':') {
        let (w, h) = (w.trim().parse::<f64>().ok()?, h.trim().parse::<f64>().ok()?);
        if h != 0.0 { Some(w / h) } else { None }
    } else {
        s.trim().parse::<f64>().ok()
    }
}

/// Parse a Lensfun XML database (a `<lensdatabase>` document) and return every `<lens>`
/// entry that has at least one recognized (poly3/poly5/ptlens) distortion calibration.
pub fn parse_lenses(xml: &str) -> Vec<LensfunLens> {
    let mut lenses = Vec::new();

    let mut in_lens = false;
    let mut in_calibration = false;
    let mut cur = LensfunLens::default();
    let mut text_buf = String::new();

    for e in EventReader::new(xml.as_bytes()) {
        match e {
            Ok(XmlEvent::StartElement { name, attributes, .. }) => {
                text_buf.clear();
                match name.local_name.as_str() {
                    "lens" => {
                        in_lens = true;
                        cur = LensfunLens { aspect_ratio: DEFAULT_ASPECT_RATIO, ..Default::default() };
                    }
                    "calibration" if in_lens => in_calibration = true,
                    "distortion" if in_lens && in_calibration => {
                        let attr = |n: &str| attributes.iter().find(|a| a.name.local_name == n).map(|a| a.value.as_str());
                        let attr_f = |n: &str| attr(n).and_then(|v| v.parse::<f64>().ok());
                        let model = attr("model").unwrap_or_default().to_string();
                        let k = match model.as_str() {
                            "poly3" => attr_f("k1").map(|k1| vec![k1]),
                            "poly5" => attr_f("k1").zip(attr_f("k2")).map(|(k1, k2)| vec![k1, k2]),
                            "ptlens" => {
                                let (a, b, c) = (attr_f("a").unwrap_or(0.0), attr_f("b").unwrap_or(0.0), attr_f("c").unwrap_or(0.0));
                                Some(vec![a, b, c])
                            }
                            _ => None,
                        };
                        if let (Some(k), Some(focal)) = (k, attr_f("focal")) {
                            cur.distortions.push(LensfunDistortion { model, focal, real_focal: attr_f("real-focal"), k });
                        }
                    }
                    _ => {}
                }
            }
            Ok(XmlEvent::Characters(s)) => text_buf.push_str(&s),
            Ok(XmlEvent::EndElement { name }) => {
                let local = name.local_name.as_str();
                if in_lens && !in_calibration {
                    let text = text_buf.trim();
                    match local {
                        "maker" => cur.maker = text.to_string(),
                        "model" => cur.model = text.to_string(),
                        "mount" => cur.mounts.push(text.to_string()),
                        "cropfactor" => if let Ok(v) = text.parse() { cur.crop_factor = v; },
                        "aspect-ratio" => if let Some(v) = parse_aspect_ratio(text) { cur.aspect_ratio = v; },
                        _ => {}
                    }
                }
                match local {
                    "calibration" => in_calibration = false,
                    "lens" => {
                        in_lens = false;
                        if !cur.distortions.is_empty() && cur.crop_factor > 0.0 {
                            lenses.push(std::mem::take(&mut cur));
                        }
                    }
                    _ => {}
                }
                text_buf.clear();
            }
            Err(_) => break,
            _ => {}
        }
    }

    lenses
}

/// Compute Lensfun's "hugin scaling" factor for a calibration point: the ratio between
/// the lens' real focal length and the sensor half-height (in mm) implied by its crop
/// factor and aspect ratio. Ported from `AutoScaleLensParams`/mod-coord.cpp: coefficients
/// given at this scale must be rescaled by `rescale_coeffs` before use in gyroflow, whose
/// distortion models operate on `x/z, y/z` normalized by gyroflow's own camera matrix.
pub fn hugin_scaling(model: &str, k: &[f64], focal: f64, real_focal: Option<f64>, crop_factor: f64, aspect_ratio: f64) -> f64 {
    let real_focal = real_focal.unwrap_or_else(|| derive_real_focal(model, k, focal));
    let hugin_scale_in_millimeters = FULL_FRAME_DIAGONAL_MM / crop_factor / aspect_ratio.hypot(1.0) / 2.0;
    if hugin_scale_in_millimeters.abs() < f64::EPSILON {
        return 1.0;
    }
    real_focal / hugin_scale_in_millimeters
}

/// Derive a calibration point's real focal length from its polynomial coefficients, for the
/// (common) case where Lensfun's `real-focal` attribute isn't present. Ported from `database.cpp`.
fn derive_real_focal(model: &str, k: &[f64], focal: f64) -> f64 {
    match model {
        "ptlens" if k.len() >= 3 => focal * (1.0 - k[0] - k[1] - k[2]),
        "poly3"  if k.len() >= 1 => focal * (1.0 - k[0]),
        _ => focal,
    }
}

/// Rescale coefficients (in place) from Lensfun's hugin-normalized scale into gyroflow's.
pub fn rescale_coeffs(model: &str, k: &mut [f64], hugin_scaling: f64) {
    match model {
        "poly3" => Poly3::rescale_coeffs(k, hugin_scaling),
        "poly5" => Poly5::rescale_coeffs(k, hugin_scaling),
        "ptlens" => PtLens::rescale_coeffs(k, hugin_scaling),
        _ => {}
    }
}

/// Interpolate a lens' distortion coefficients at an arbitrary `focal` length, for a camera
/// with the given crop factor. Lensfun only calibrates a handful of discrete focal lengths per
/// zoom lens, so a video shot in between needs its coefficients derived rather than looked up.
/// Direct port of `lfLens::InterpolateDistortion` in Lensfun's `lens.cpp`.
///
/// Note: `distortion_to_profile` still builds one profile per exactly-calibrated focal length,
/// so this doesn't change what gets generated today — it's the piece a future "arbitrary focal"
/// lookup (e.g. matching a video's actual focal length instead of the nearest calibrated one)
/// would call into.
pub fn interpolate_distortion(lens: &LensfunLens, camera_crop: f64, focal: f64) -> Option<LensfunDistortion> {
    // A calibration from a smaller sensor than the target camera doesn't cover its whole
    // frame and can't be reused — same `>= 0.96` tolerance Lensfun itself uses.
    if lens.crop_factor <= 0.0 || camera_crop / lens.crop_factor < 0.96 {
        return None;
    }
    let model = lens.distortions.first()?.model.clone();

    // 2 nearest calibrated focals below and 2 above the requested one.
    let mut spline: [Option<&LensfunDistortion>; 4] = [None; 4];
    let mut spline_dist = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];
    for d in &lens.distortions {
        if d.model != model { continue; }
        let df = focal - d.focal;
        if df == 0.0 {
            return Some(d.clone());
        }
        if df < 0.0 {
            if df > spline_dist[1] {
                spline_dist[0] = spline_dist[1]; spline_dist[1] = df;
                spline[0] = spline[1]; spline[1] = Some(d);
            } else if df > spline_dist[0] {
                spline_dist[0] = df; spline[0] = Some(d);
            }
        } else if df < spline_dist[2] {
            spline_dist[3] = spline_dist[2]; spline_dist[2] = df;
            spline[3] = spline[2]; spline[2] = Some(d);
        } else if df < spline_dist[3] {
            spline_dist[3] = df; spline[3] = Some(d);
        }
    }

    let (s1, s2) = match (spline[1], spline[2]) {
        (Some(s1), Some(s2)) => (s1, s2),
        // Requested focal is outside the calibrated range, clamp to the nearest entry.
        (Some(s), None) | (None, Some(s)) => return Some(s.clone()),
        (None, None) => return None,
    };

    let t = (focal - s1.focal) / (s2.focal - s1.focal);
    let real_focal = |d: &LensfunDistortion| d.real_focal.unwrap_or_else(|| derive_real_focal(&d.model, &d.k, d.focal));
    let interpolated_real_focal = hermite_interpolate(spline[0].map(real_focal), real_focal(s1), real_focal(s2), spline[3].map(real_focal), t);

    let n = s1.k.len().min(s2.k.len());
    let mut k = vec![0.0; n];
    for (i, ki) in k.iter_mut().enumerate() {
        // Parameters are ~proportional to inverse focal length, so interpolate `term * focal`
        // (see `__parameter_scales` in Lensfun's lens.cpp), not the raw term.
        let scaled_edge = |d: Option<&LensfunDistortion>| d.and_then(|d| d.k.get(i).map(|v| v * d.focal));
        *ki = hermite_interpolate(scaled_edge(spline[0]), s1.k[i] * s1.focal, s2.k[i] * s2.focal, scaled_edge(spline[3]), t) / focal;
    }

    Some(LensfunDistortion { model, focal, real_focal: Some(interpolated_real_focal), k })
}

/// Hermite spline interpolation, direct port of `_lf_interpolate` in Lensfun's `auxfun.cpp`.
/// `y1`/`y4` are the values surrounding the interpolated segment `y2`..`y3` and may be missing
/// (at the edge of the calibrated range), falling back to a linear tangent in that case.
fn hermite_interpolate(y1: Option<f64>, y2: f64, y3: f64, y4: Option<f64>, t: f64) -> f64 {
    let t2 = t * t;
    let t3 = t2 * t;
    let tg2 = y1.map_or(y3 - y2, |y1| (y3 - y1) * 0.5);
    let tg3 = y4.map_or(y3 - y2, |y4| (y4 - y2) * 0.5);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y2 + (t3 - 2.0 * t2 + t) * tg2 + (-2.0 * t3 + 3.0 * t2) * y3 + (t3 - t2) * tg3
}

// ponytail: Lensfun calibrations are resolution-independent (only aspect ratio matters),
// so we bake them at an arbitrary reference pixel height. `get_all_matching_profiles`/the
// camera matrix scale proportionally with actual video size elsewhere in the UI, same as
// other formula-based (non-measured) profiles. Revisit if a specific target resolution
// ever needs bit-exact matching against Lensfun/Hugin's own output.
const REFERENCE_HEIGHT_PX: f64 = 2000.0;

/// Build a gyroflow `LensProfile` for one Lensfun lens + one of its distortion calibration points.
pub fn distortion_to_profile(lens: &LensfunLens, d: &LensfunDistortion) -> Option<LensProfile> {
    if d.k.is_empty() || lens.crop_factor <= 0.0 || lens.aspect_ratio <= 0.0 {
        return None;
    }

    let mut k = d.k.clone();
    let scaling = hugin_scaling(&d.model, &k, d.focal, d.real_focal, lens.crop_factor, lens.aspect_ratio);
    rescale_coeffs(&d.model, &mut k, scaling);

    let height = REFERENCE_HEIGHT_PX;
    let width = (height * lens.aspect_ratio).round();
    let f_px = scaling * height / 2.0;
    if !f_px.is_finite() || f_px <= 0.0 {
        return None;
    }

    let mut profile = LensProfile::default();
    profile.camera_brand = lens.maker.clone();
    profile.camera_model = lens.model.clone();
    profile.lens_model = lens.mounts.first().cloned().unwrap_or_default();
    profile.camera_setting = format!("{:.0}mm (Lensfun)", d.focal);
    profile.calib_dimension = Dimensions { w: width as usize, h: height as usize };
    profile.orig_dimension = Dimensions { w: width as usize, h: height as usize };
    profile.focal_length = Some(d.focal);
    profile.crop_factor = Some(lens.crop_factor);
    profile.distortion_model = Some(d.model.clone());
    profile.calibrated_by = "Lensfun".to_string();
    profile.official = true;
    profile.identifier = format!("lensfun_{}_{}_{}_{:.2}mm", lens.maker, lens.model, d.model, d.focal).replace(' ', "_");
    profile.fisheye_params = CameraParams {
        RMS_error: 0.0,
        camera_matrix: vec![
            [f_px, 0.0, width / 2.0],
            [0.0, f_px, height / 2.0],
            [0.0, 0.0, 1.0],
        ],
        distortion_coeffs: k,
        radial_distortion_limit: None,
    };
    profile.init();

    Some(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two lenses, mirroring real Lensfun DB entries (see lensfun/data/db/*.xml):
    // one ptlens (Canon full-frame prime, single calibration point) and one poly3
    // (APS-C zoom, two focal lengths) to exercise both models and multi-point lenses.
    const SAMPLE_XML: &str = r#"<!DOCTYPE lensdatabase SYSTEM "lensfun-database.dtd">
<lensdatabase version="2">
    <mount>
        <name>Canon EF</name>
    </mount>
    <lens>
        <maker>Canon</maker>
        <model>Canon EF 35mm f/2</model>
        <mount>Canon EF</mount>
        <cropfactor>1.0</cropfactor>
        <calibration>
            <distortion model="ptlens" focal="35" a="0.0070163863" b="-0.010468312" c="0.0030782963"/>
        </calibration>
    </lens>
    <lens>
        <maker>Sigma</maker>
        <model>Sigma 18-35mm f/1.8 DC HSM</model>
        <mount>Canon EF</mount>
        <cropfactor>1.6</cropfactor>
        <aspect-ratio>3:2</aspect-ratio>
        <calibration>
            <distortion model="poly3" focal="18" k1="-0.02127"/>
            <distortion model="poly3" focal="35" k1="0.00366"/>
        </calibration>
    </lens>
</lensdatabase>"#;

    #[test]
    fn parses_lens_metadata_and_mounts() {
        let lenses = parse_lenses(SAMPLE_XML);
        assert_eq!(lenses.len(), 2);

        assert_eq!(lenses[0].maker, "Canon");
        assert_eq!(lenses[0].model, "Canon EF 35mm f/2");
        assert_eq!(lenses[0].mounts, vec!["Canon EF".to_string()]);
        assert_eq!(lenses[0].crop_factor, 1.0);
        assert_eq!(lenses[0].aspect_ratio, DEFAULT_ASPECT_RATIO); // not specified -> default 3:2

        assert_eq!(lenses[1].crop_factor, 1.6);
        assert_eq!(lenses[1].aspect_ratio, 1.5); // explicit "3:2"
        assert_eq!(lenses[1].distortions.len(), 2);
    }

    #[test]
    fn parses_ptlens_coefficients() {
        let lenses = parse_lenses(SAMPLE_XML);
        let d = &lenses[0].distortions[0];
        assert_eq!(d.model, "ptlens");
        assert_eq!(d.focal, 35.0);
        assert_eq!(d.real_focal, None);
        assert_eq!(d.k, vec![0.0070163863, -0.010468312, 0.0030782963]);
    }

    #[test]
    fn parses_poly3_coefficients_at_multiple_focal_lengths() {
        let lenses = parse_lenses(SAMPLE_XML);
        let ds = &lenses[1].distortions;
        assert_eq!(ds[0].focal, 18.0);
        assert_eq!(ds[0].k, vec![-0.02127]);
        assert_eq!(ds[1].focal, 35.0);
        assert_eq!(ds[1].k, vec![0.00366]);
    }

    #[test]
    fn ignores_lens_without_recognized_distortion() {
        let xml = r#"<lensdatabase version="2">
            <lens>
                <maker>Nikon</maker>
                <model>Some lens with only vignetting data</model>
                <mount>Nikon F</mount>
                <cropfactor>1.5</cropfactor>
                <calibration>
                    <vignetting model="pa" focal="50" aperture="2.8" distance="1000" k1="-0.5" k2="0.2" k3="-0.1"/>
                </calibration>
            </lens>
        </lensdatabase>"#;
        assert!(parse_lenses(xml).is_empty());
    }

    #[test]
    fn hugin_scaling_is_one_at_reference_focal_length() {
        // At crop_factor=1, aspect=3:2, the sensor half-height in mm is
        // FULL_FRAME_DIAGONAL_MM / hypot(1.5, 1) / 2. Using exactly that value as the
        // real focal length must yield a hugin_scaling of 1.0 (no-op rescale).
        let half_height_mm = FULL_FRAME_DIAGONAL_MM / 1.5f64.hypot(1.0) / 2.0;
        let scaling = hugin_scaling("poly3", &[0.0], half_height_mm, Some(half_height_mm), 1.0, 1.5);
        assert!((scaling - 1.0).abs() < 1e-9, "expected 1.0, got {scaling}");
    }

    #[test]
    fn hugin_scaling_scales_inversely_with_crop_factor() {
        let a = hugin_scaling("poly3", &[0.0], 50.0, Some(50.0), 1.0, 1.5);
        let b = hugin_scaling("poly3", &[0.0], 50.0, Some(50.0), 2.0, 1.5);
        assert!((b - a * 2.0).abs() < 1e-9, "doubling crop_factor should double hugin_scaling: {a} vs {b}");
    }

    #[test]
    fn hugin_scaling_derives_real_focal_from_coefficients_when_absent() {
        // poly3: real_focal = focal * (1 - k1)
        let with_k = hugin_scaling("poly3", &[0.1], 50.0, None, 1.0, 1.5);
        let explicit = hugin_scaling("poly3", &[0.1], 45.0, Some(45.0), 1.0, 1.5); // 50*(1-0.1) = 45
        assert!((with_k - explicit).abs() < 1e-9, "{with_k} vs {explicit}");

        // ptlens: real_focal = focal * (1 - a - b - c)
        let with_k = hugin_scaling("ptlens", &[0.01, -0.02, 0.03], 35.0, None, 1.0, 1.5);
        let real_focal = 35.0 * (1.0 - 0.01 - (-0.02) - 0.03);
        let explicit = hugin_scaling("ptlens", &[0.01, -0.02, 0.03], real_focal, Some(real_focal), 1.0, 1.5);
        assert!((with_k - explicit).abs() < 1e-9, "{with_k} vs {explicit}");
    }

    #[test]
    fn rescale_coeffs_matches_poly3_formula() {
        let mut k = vec![0.05];
        rescale_coeffs("poly3", &mut k, 2.0);
        let d = 1.0 - 0.05_f64;
        let expected = 0.05 * 2.0f64.powi(2) / d.powi(3);
        assert!((k[0] - expected).abs() < 1e-12);
    }

    #[test]
    fn distortion_to_profile_produces_usable_profile() {
        let lenses = parse_lenses(SAMPLE_XML);
        let profile = distortion_to_profile(&lenses[0], &lenses[0].distortions[0]).expect("profile");

        assert_eq!(profile.camera_brand, "Canon");
        assert_eq!(profile.camera_model, "Canon EF 35mm f/2");
        assert_eq!(profile.distortion_model.as_deref(), Some("ptlens"));
        assert_eq!(profile.fisheye_params.distortion_coeffs.len(), 3);
        assert_eq!(profile.calib_dimension.h, REFERENCE_HEIGHT_PX as usize);
        assert_eq!(profile.calib_dimension.w, (REFERENCE_HEIGHT_PX * 1.5).round() as usize);

        let fx = profile.fisheye_params.camera_matrix[0][0];
        assert!(fx.is_finite() && fx > 0.0);
        assert_eq!(profile.fisheye_params.camera_matrix[0][2], profile.calib_dimension.w as f64 / 2.0);
        assert_eq!(profile.fisheye_params.camera_matrix[1][2], profile.calib_dimension.h as f64 / 2.0);
    }

    #[test]
    fn distortion_to_profile_rejects_lens_with_zero_crop_factor() {
        let lens = LensfunLens { crop_factor: 0.0, aspect_ratio: 1.5, distortions: vec![LensfunDistortion { model: "poly3".into(), focal: 50.0, k: vec![0.1], ..Default::default() }], ..Default::default() };
        assert!(distortion_to_profile(&lens, &lens.distortions[0]).is_none());
    }

    // Reference values below were generated by running actual Lensfun (git master, e78e7be4+)
    // through `lfLens::InterpolateDistortion` on an equivalent 4-focal ptlens zoom.
    const ZOOM_XML: &str = r#"<lensdatabase version="2">
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
    </lensdatabase>"#;

    #[test]
    fn interpolate_distortion_returns_exact_match_without_interpolating() {
        let lens = &parse_lenses(ZOOM_XML)[0];
        let d = interpolate_distortion(lens, 1.5, 24.0).unwrap();
        assert_eq!(d.k, vec![0.008, -0.021, 0.001]);
    }

    #[test]
    fn interpolate_distortion_matches_lensfun_hermite_spline() {
        let lens = &parse_lenses(ZOOM_XML)[0];
        // 28mm falls between the 24mm and 35mm calibration points.
        let d = interpolate_distortion(lens, 1.5, 28.0).unwrap();
        assert!((d.k[0] -  0.00630503381).abs()  < 1e-7, "a = {}", d.k[0]);
        assert!((d.k[1] - -0.0157351606).abs()   < 1e-7, "b = {}", d.k[1]);
        assert!((d.k[2] -  0.000774793385).abs() < 1e-7, "c = {}", d.k[2]);
        assert!((d.real_focal.unwrap() - 27.4955425).abs() < 1e-5, "real_focal = {}", d.real_focal.unwrap());
    }

    #[test]
    fn interpolate_distortion_clamps_outside_calibrated_range() {
        let lens = &parse_lenses(ZOOM_XML)[0];
        let d = interpolate_distortion(lens, 1.5, 100.0).unwrap();
        assert_eq!(d.k, vec![0.001, -0.002, 0.0]); // clamped to the 55mm entry
    }

    #[test]
    fn interpolate_distortion_rejects_calibration_from_smaller_sensor() {
        let lens = &parse_lenses(ZOOM_XML)[0]; // calibrated on a 1.5 crop sensor
        assert!(interpolate_distortion(lens, 1.0, 24.0).is_none()); // full-frame target can't use it
        assert!(interpolate_distortion(lens, 1.6, 24.0).is_some()); // smaller-sensor target can
    }
}
