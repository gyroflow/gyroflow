// SPDX-License-Identifier: GPL-3.0-or-later

//! Lensfun database import.
//!
//! Lensfun ships its lens calibration database as XML: `<camera>` entries with
//! a maker and model, `<lens>` entries with a model and crop factor, and
//! `<distortion>` entries that carry a model plus its coefficients at one focal
//! length.
//!
//! Those coefficients are expressed in the focal plane, in millimetres.
//! Gyroflow's distortion models work in normalised image radius, so they have to
//! be rescaled first â€” the conversion liblensfun applies in `mod-coord.cpp`, and
//! the one sketched in the TODO in `distortion_models/poly3.rs`.
//!
//! The module does not touch the network or the filesystem: it turns a database
//! string into `LensProfile`s and leaves the caller to decide where the XML came
//! from and whether the profile is offered in the UI.

use crate::lens_profile::{Dimensions, LensProfile};
use crate::stabilization::distortion_models::DistortionModel;

/// Distortion models this importer can translate, in database spelling.
pub const SUPPORTED_MODELS: [&str; 3] = ["poly3", "poly5", "ptlens"];

/// One distortion entry: coefficients for a model at a focal length.
#[derive(Clone, Debug, PartialEq)]
pub struct Calibration {
    pub model: String,
    /// Focal length in millimetres.
    pub focal: f64,
    /// Coefficients in model order, in database units (millimetres).
    pub coeffs: Vec<f64>,
}

/// One `<lens>` entry and the calibrations that belong to it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Lens {
    pub maker: String,
    pub model: String,
    /// Crop factor of the calibration camera; 1.0 when the database omits it.
    pub crop_factor: f64,
    pub calibrations: Vec<Calibration>,
}

/// A camera body entry, so callers can search by body as well as by lens.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Camera {
    pub maker: String,
    pub model: String,
}

/// The parsed database.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Database {
    pub cameras: Vec<Camera>,
    pub lenses: Vec<Lens>,
}

/// Errors returned while reading a database string.
#[derive(Clone, Debug, PartialEq)]
pub enum LensfunError {
    /// The input contained no usable `<lens>` entry.
    Empty,
}

impl std::fmt::Display for LensfunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "no lenses in the database"),
        }
    }
}

impl std::error::Error for LensfunError {}

/// Reads `name="value"` pairs from one tag body.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = tag.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] as char).is_whitespace() { i += 1; }
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && !(bytes[i] as char).is_whitespace() { i += 1; }
        let key = &tag[key_start..i];
        while i < bytes.len() && (bytes[i] as char).is_whitespace() { i += 1; }
        if i >= bytes.len() || bytes[i] != b'=' {
            if !key.is_empty() { continue; }
            i += 1;
            continue;
        }
        i += 1;
        while i < bytes.len() && (bytes[i] as char).is_whitespace() { i += 1; }
        if i >= bytes.len() { break; }
        let quote = bytes[i];
        if quote != b'"' && quote != b'\'' {
            let value_start = i;
            while i < bytes.len() && !(bytes[i] as char).is_whitespace() { i += 1; }
            out.push((key.to_string(), tag[value_start..i].to_string()));
            continue;
        }
        i += 1;
        let value_start = i;
        while i < bytes.len() && bytes[i] != quote { i += 1; }
        out.push((key.to_string(), tag[value_start..i].to_string()));
        if i < bytes.len() { i += 1; }
    }
    out
}

fn attribute<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn number(pairs: &[(String, String)], key: &str) -> Option<f64> {
    attribute(pairs, key).and_then(|v| v.trim().parse::<f64>().ok())
}

/// Coefficient keys per model, in the order the kernels expect.
fn coefficient_keys(model: &str) -> &'static [&'static str] {
    match model {
        "poly3" => &["k1", "a"],
        "poly5" => &["k1", "k2", "a", "b"],
        "ptlens" => &["a", "b", "c"],
        _ => &[],
    }
}

/// Parses a lensfun XML database.
///
/// Walks the document once, tracking the innermost `<camera>` and `<lens>`, and
/// attaches every `<distortion>` to the lens that contains it. Unknown elements
/// are ignored so additions in the database format do not break the parse.
pub fn parse(xml: &str) -> Result<Database, LensfunError> {
    let mut db = Database::default();
    let mut current_lens: Option<Lens> = None;
    let mut current_camera: Option<Camera> = None;
    // text field currently being read inside a lens or camera element
    let mut pending: Option<&'static str> = None;

    let mut rest = xml;
    loop {
        let Some(start) = rest.find('<') else { break };
        // text between the previous tag and this one
        if start > 0 {
            let text = &rest[..start];
            let value = text.trim();
            if !value.is_empty() {
                match (pending, current_lens.as_mut(), current_camera.as_mut()) {
                    (Some("maker"), Some(lens), _) => lens.maker = value.to_string(),
                    (Some("model"), Some(lens), _) => lens.model = value.to_string(),
                    (Some("crop_factor"), Some(lens), _) => {
                        if let Ok(v) = value.parse::<f64>() { if v > 0.0 { lens.crop_factor = v; } }
                    }
                    (Some("maker"), _, Some(camera)) => camera.maker = value.to_string(),
                    (Some("model"), _, Some(camera)) => camera.model = value.to_string(),
                    _ => {}
                }
            }
            pending = None;
        }
        rest = &rest[start..];

        if rest.starts_with("<!--") {
            match rest.find("-->") { Some(i) => { rest = &rest[i + 3..]; continue; } None => break }
        }
        if rest.starts_with("<?") {
            match rest.find("?>") { Some(i) => { rest = &rest[i + 2..]; continue; } None => break }
        }

        let Some(end) = rest.find('>') else { break };
        let raw = &rest[1..end];
        let closing = raw.starts_with('/');
        let self_closing = raw.trim_end().ends_with('/');
        let body = raw.trim_start_matches('/').trim_end_matches('/').trim();
        let name = body.split_whitespace().next().unwrap_or("");
        let attrs = attributes(body);
        rest = &rest[end + 1..];

        match (closing, name) {
            (false, "camera") => {
                current_camera = Some(Camera {
                    maker: attribute(&attrs, "maker").unwrap_or("").to_string(),
                    model: attribute(&attrs, "model").unwrap_or("").to_string(),
                });
                if self_closing {
                    if let Some(c) = current_camera.take() { db.cameras.push(c); }
                }
            }
            (true, "camera") => {
                if let Some(c) = current_camera.take() {
                    if !c.model.is_empty() || !c.maker.is_empty() { db.cameras.push(c); }
                }
            }
            (false, "lens") => {
                current_lens = Some(Lens {
                    maker: attribute(&attrs, "maker").unwrap_or("").to_string(),
                    model: attribute(&attrs, "model").unwrap_or("").to_string(),
                    crop_factor: number(&attrs, "crop_factor").filter(|v| *v > 0.0).unwrap_or(1.0),
                    calibrations: Vec::new(),
                });
                if self_closing {
                    if let Some(l) = current_lens.take() {
                        if !l.model.is_empty() { db.lenses.push(l); }
                    }
                }
            }
            (true, "lens") => {
                if let Some(l) = current_lens.take() {
                    if !l.model.is_empty() { db.lenses.push(l); }
                }
            }
            (false, "maker") => pending = Some("maker"),
            (false, "model") => pending = Some("model"),
            (false, "crop_factor") => pending = Some("crop_factor"),
            (false, "distortion") | (false, "calibration") => {
                let Some(lens) = current_lens.as_mut() else { continue };
                let Some(model) = attribute(&attrs, "model").or_else(|| Some("poly3")).filter(|m| SUPPORTED_MODELS.contains(m)) else { continue };
                let Some(focal) = number(&attrs, "focal") else { continue };
                let coeffs: Vec<f64> = coefficient_keys(model)
                    .iter()
                    .filter_map(|key| number(&attrs, key))
                    .collect();
                if coeffs.is_empty() { continue; }
                lens.calibrations.push(Calibration { model: model.to_string(), focal, coeffs });
            }
            _ => {}
        }
    }

    if db.lenses.is_empty() {
        return Err(LensfunError::Empty);
    }
    Ok(db)
}

/// Normalisation radius liblensfun uses: half the diagonal of a 36Ã—24 frame,
/// divided by the crop factor and by the frame's own diagonal ratio.
fn hugin_scale(crop_factor: f64, aspect_ratio: f64, focal: f64, model: &str, coeffs: &[f64]) -> f64 {
    // Lensfun stores coefficients against the *real* focal length, which the
    // distortion itself shortens for poly3 and ptlens (mod-coord.cpp).
    let real_focal = match model {
        "ptlens" => focal * (1.0 - coeffs.iter().take(3).sum::<f64>()),
        "poly3" => focal * (1.0 - coeffs.first().copied().unwrap_or(0.0)),
        _ => focal,
    };
    let scale_mm = 36.0_f64.hypot(24.0) / crop_factor / aspect_ratio.hypot(1.0) / 2.0;
    if scale_mm <= 0.0 { 1.0 } else { real_focal / scale_mm }
}

/// Applies liblensfun's `rescale_coeffs` for the given model.
pub fn rescale_coeffs(model: &str, coeffs: &[f64], scaling: f64) -> Vec<f64> {
    let mut k = coeffs.to_vec();
    match model {
        "poly3" => {
            if k.is_empty() { return k; }
            let d = 1.0 - k[0];
            if d.abs() > f64::EPSILON {
                k[0] *= scaling.powi(2) / d.powi(3);
            }
        }
        "poly5" => {
            if k.len() < 2 { return k; }
            k[0] *= scaling.powi(2);
            k[1] *= scaling.powi(4);
        }
        "ptlens" => {
            if k.len() < 3 { return k; }
            let d = 1.0 - k[0] - k[1] - k[2];
            if d.abs() > f64::EPSILON {
                k[0] *= scaling.powi(3) / d.powi(4);
                k[1] *= scaling.powi(2) / d.powi(3);
                k[2] *= scaling / d.powi(2);
            }
        }
        _ => {}
    }
    k
}

/// Picks the calibration for `focal`, preferring the closest focal length.
///
/// Lensfun interpolates between neighbouring entries; a nearest match keeps the
/// prime case exact and everything else predictable. Callers that want
/// interpolation can read `Lens::calibrations` directly.
pub fn calibration_at<'a>(lens: &'a Lens, focal: f64) -> Option<&'a Calibration> {
    lens.calibrations
        .iter()
        .min_by(|a, b| {
            (a.focal - focal).abs()
                .partial_cmp(&(b.focal - focal).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// Converts one calibration into a `LensProfile` ready for the rest of the pipeline.
pub fn to_lens_profile(
    lens: &Lens,
    focal: f64,
    dimension: Dimensions,
    aspect_ratio: Option<f64>,
) -> Option<LensProfile> {
    let calibration = calibration_at(lens, focal)?;
    if !SUPPORTED_MODELS.contains(&calibration.model.as_str()) {
        return None;
    }
    let aspect = aspect_ratio.unwrap_or_else(|| {
        if dimension.h > 0 { dimension.w as f64 / dimension.h as f64 } else { 1.5 }
    });

    let scale = hugin_scale(lens.crop_factor, aspect, calibration.focal, &calibration.model, &calibration.coeffs);
    let coeffs = rescale_coeffs(&calibration.model, &calibration.coeffs, scale);

    let mut profile = LensProfile::default();
    profile.name = format!("{} {} @ {}mm", lens.maker, lens.model, calibration.focal);
    profile.lens_model = lens.model.clone();
    profile.camera_brand = lens.maker.clone();
    profile.calib_dimension = dimension.clone();
    profile.orig_dimension = dimension;
    profile.distortion_model = Some(calibration.model.clone());
    profile.focal_length = Some(calibration.focal);
    profile.crop_factor = Some(lens.crop_factor);
    profile.calibrator_version = format!("lensfun-import/{}", env!("CARGO_PKG_VERSION"));
    profile.fisheye_params.distortion_coeffs = coeffs.clone();
    profile.digital_lens_params = Some(coeffs);
    profile.init();

    // Reject coefficients that the kernels cannot invert (e.g. a degenerate
    // radial limit), so a bad database entry never reaches the renderer.
    let model = DistortionModel::from_name(&calibration.model);
    if model.radial_distortion_limit(&profile.get_distortion_coeffs()).is_none() {
        return None;
    }
    Some(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DB: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<lensdatabase>
  <camera>
    <maker>Sony</maker>
    <model>ILCE-7M3</model>
  </camera>
  <lens>
    <maker>Sony</maker>
    <model>FE 24-70mm F2.8 GM</model>
    <crop_factor>1.0</crop_factor>
    <calibration>
      <distortion model="poly3" focal="24" k1="-0.08"/>
      <distortion model="poly3" focal="70" k1="0.02"/>
    </calibration>
    <calibration>
      <distortion model="ptlens" focal="35" a="0.01" b="-0.02" c="0.005"/>
    </calibration>
  </lens>
  <lens>
    <maker>GoPro</maker>
    <model>Hero 9 Black</model>
    <crop_factor>5.6</crop_factor>
    <calibration>
      <distortion model="opencv_fisheye" focal="3"/>
    </calibration>
  </lens>
</lensdatabase>
"#;

    #[test]
    fn parses_cameras_lenses_and_calibrations() {
        let db = parse(DB).expect("database parses");
        assert_eq!(db.cameras.len(), 1);
        assert_eq!(db.cameras[0].model, "ILCE-7M3");
        assert_eq!(db.lenses.len(), 2);
        assert_eq!(db.lenses[0].model, "FE 24-70mm F2.8 GM");
        assert_eq!(db.lenses[0].maker, "Sony");
        assert_eq!(db.lenses[0].crop_factor, 1.0);
        let models: Vec<_> = db.lenses[0].calibrations.iter().map(|c| c.model.as_str()).collect();
        assert_eq!(models, ["poly3", "poly3", "ptlens"]);
        assert_eq!(db.lenses[1].crop_factor, 5.6);
        // unsupported distortion model is dropped, not guessed at
        assert!(db.lenses[1].calibrations.is_empty());
    }

    #[test]
    fn picks_the_nearest_focal_length() {
        let db = parse(DB).unwrap();
        let lens = &db.lenses[0];
        assert_eq!(calibration_at(lens, 20.0).unwrap().focal, 24.0);
        assert_eq!(calibration_at(lens, 60.0).unwrap().focal, 70.0);
        assert_eq!(calibration_at(lens, 35.0).unwrap().model, "ptlens");
    }

    #[test]
    fn rescales_poly3_coefficients() {
        // k' = k * s^2 / (1 - k)^3
        let k = rescale_coeffs("poly3", &[-0.08], 2.0);
        let expected = -0.08 * 4.0 / (1.08_f64).powi(3);
        assert!((k[0] - expected).abs() < 1e-12);
    }

    #[test]
    fn rescales_poly5_and_ptlens() {
        let k5 = rescale_coeffs("poly5", &[0.1, -0.02], 2.0);
        assert!((k5[0] - 0.4).abs() < 1e-12);
        assert!((k5[1] + 0.32).abs() < 1e-12);

        let pt = rescale_coeffs("ptlens", &[0.01, -0.02, 0.005], 2.0);
        let d: f64 = 1.0 - 0.01 + 0.02 - 0.005;
        assert!((pt[0] - 0.01 * 8.0 / d.powi(4)).abs() < 1e-12);
        assert!((pt[1] + 0.02 * 4.0 / d.powi(3)).abs() < 1e-12);
        assert!((pt[2] - 0.005 * 2.0 / d.powi(2)).abs() < 1e-12);
    }

    #[test]
    fn converts_to_a_lens_profile() {
        let db = parse(DB).unwrap();
        let profile = to_lens_profile(&db.lenses[0], 24.0, Dimensions { w: 6000, h: 4000 }, None)
            .expect("poly3 converts");
        assert_eq!(profile.distortion_model.as_deref(), Some("poly3"));
        assert_eq!(profile.calib_dimension.w, 6000);
        assert_eq!(profile.calib_dimension.h, 4000);
        assert_eq!(profile.fisheye_params.distortion_coeffs.len(), 1);
        assert_eq!(profile.digital_lens_params.as_ref().unwrap().len(), 1);
        assert_eq!(profile.crop_factor, Some(1.0));
        assert!(profile.name.contains("24mm"));
    }

    #[test]
    fn rejects_unusable_input() {
        assert_eq!(parse("<lensdatabase></lensdatabase>"), Err(LensfunError::Empty));
        assert_eq!(parse(""), Err(LensfunError::Empty));

        let db = parse(DB).unwrap();
        assert!(to_lens_profile(&db.lenses[1], 3.0, Dimensions { w: 4000, h: 3000 }, None).is_none());
    }

    #[test]
    fn attribute_parsing_handles_quotes_and_spacing() {
        let attrs = attributes(r#"distortion model="poly3" focal='24' k1 = "-0.08""#);
        assert_eq!(attribute(&attrs, "model"), Some("poly3"));
        assert_eq!(attribute(&attrs, "focal"), Some("24"));
        assert_eq!(attribute(&attrs, "k1"), Some("-0.08"));
    }
}
