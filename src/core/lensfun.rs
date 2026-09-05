// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2025 Adrian <adrian.eddy at gmail>

// Parsing of the lensfun XML database (https://github.com/lensfun/lensfun/tree/master/data/db)
// and conversion of their lens calibrations to Gyroflow lens profiles.
//
// The coefficient rescaling and focal interpolation math is adapted from lensfun:
// - libs/lensfun/mod-coord.cpp (rescale_polynomial_coefficients)
// - libs/lensfun/lens.cpp      (lfLens::InterpolateDistortion)
// - libs/lensfun/auxfun.cpp    (_lf_interpolate)
//
// The lensfun database is expected as .xml files inside a `lensfun` directory within the
// lens profiles directory (see `LENSFUN_DIR_NAME`). This is meant to be provided by a
// git submodule of https://github.com/lensfun/lensfun data/db in gyroflow/lens_profiles.

use crate::LensProfile;
use crate::lens_profile::{ CameraParams, Dimensions };
use crate::stabilization::distortion_models::{ poly3::Poly3, poly5::Poly5, ptlens::PtLens };

/// Name of the directory inside the lens profiles directory that contains the lensfun XML files.
/// A follow-up PR to gyroflow/lens_profiles can vendor https://github.com/lensfun/lensfun
/// data/db as a submodule at this location.
pub const LENSFUN_DIR_NAME: &str = "lensfun";

/// Diagonal of the full-frame (36x24mm) reference sensor, in millimeters
const FULL_FRAME_DIAG: f64 = 43.26661530556787; // 36.0f64.hypot(24.0)

/// Canonical image height used for the synthesized pinhole camera matrix.
/// The profiles are resolution-independent (the camera matrix is rescaled to the
/// actual video size in `frame_transform`), only the aspect ratio matters.
const CANONICAL_HEIGHT: f64 = 2160.0;

#[derive(Debug, Clone, Default)]
pub struct LensfunCamera {
    pub maker: String,
    pub model: String,
    pub mounts: Vec<String>,
    pub crop_factor: f64,
}

#[derive(Debug, Clone)]
pub struct LensfunDistortion {
    pub model: String, // "ptlens", "poly3" or "poly5"
    pub focal: f64,
    pub real_focal: Option<f64>,
    pub terms: Vec<f64>, // raw hugin terms: [a, b, c] / [k1] / [k1, k2]
}

impl LensfunDistortion {
    /// Real focal length: the `real-focal` attribute if present, otherwise derived from
    /// the nominal focal and the distortion terms, as in lensfun's mod-coord.cpp
    pub fn effective_real_focal(&self) -> f64 {
        if let Some(rf) = self.real_focal {
            if rf > 0.01 { return rf; }
        }
        match self.model.as_str() {
            "ptlens" => self.focal * (1.0 - self.terms.iter().take(3).sum::<f64>()),
            "poly3"  => self.focal * (1.0 - self.terms.first().copied().unwrap_or(0.0)),
            _        => self.focal
        }
    }
}

#[derive(Debug, Clone)]
pub struct LensfunCalibSet {
    pub crop_factor: f64,
    pub aspect_ratio: f64,
    pub distortions: Vec<LensfunDistortion>,
}

#[derive(Debug, Clone, Default)]
pub struct LensfunLens {
    pub maker: String,
    pub model: String,
    pub mounts: Vec<String>,
    pub crop_factor: f64,
    pub aspect_ratio: f64,
    pub calibrations: Vec<LensfunCalibSet>,
}

#[derive(Default)]
pub struct LensfunDatabase {
    pub cameras: Vec<LensfunCamera>,
    pub lenses: Vec<LensfunLens>,
}

impl LensfunDatabase {
    pub fn from_xml(xml: &str) -> Result<Self, quick_xml::Error> {
        let mut db = Self::default();

        let mut reader = quick_xml::Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut cur_camera: Option<LensfunCamera> = None;
        let mut cur_lens: Option<LensfunLens> = None;
        let mut in_calibration = false;
        let mut new_calib_set = false;
        let mut calib_crop: Option<f64> = None;
        let mut calib_aspect: Option<f64> = None;
        let mut text_buf = String::new();

        #[derive(PartialEq, Clone, Copy)]
        enum Tag { None, Maker, Model, Mount, CropFactor, AspectRatio }
        let mut cur_tag = Tag::None;

        macro_rules! flush_text {
            () => {{
                let text = text_buf.trim();
                if !text.is_empty() {
                    if let Some(ref mut cam) = cur_camera {
                        match cur_tag {
                            Tag::Maker      => if cam.maker.is_empty() { cam.maker = text.to_string(); },
                            // Prefer the first <model>, which is the full name (later ones carry lang=... variants)
                            Tag::Model      => if cam.model.is_empty() { cam.model = text.to_string(); },
                            Tag::Mount      => cam.mounts.push(text.to_string()),
                            Tag::CropFactor => cam.crop_factor = text.parse().unwrap_or(0.0),
                            _ => { }
                        }
                    } else if let Some(ref mut lens) = cur_lens {
                        match cur_tag {
                            Tag::Maker      => if lens.maker.is_empty() { lens.maker = text.to_string(); },
                            Tag::Model      => if lens.model.is_empty() { lens.model = text.to_string(); },
                            Tag::Mount      => lens.mounts.push(text.to_string()),
                            Tag::CropFactor => {
                                if in_calibration { calib_crop = text.parse().ok(); }
                                else { lens.crop_factor = text.parse().unwrap_or(0.0); }
                            },
                            Tag::AspectRatio => {
                                if in_calibration { calib_aspect = text.parse().ok(); }
                                else { lens.aspect_ratio = text.parse().unwrap_or(0.0); }
                            },
                            _ => { }
                        }
                    }
                }
                text_buf.clear();
            }};
        }

        loop {
            match reader.read_event()? {
                quick_xml::events::Event::Start(e) => {
                    let name = e.name();
                    match name.as_ref() {
                        b"camera" => cur_camera = Some(LensfunCamera::default()),
                        b"lens" => cur_lens = Some(LensfunLens::default()),
                        b"calibration" => {
                            in_calibration = true;
                            new_calib_set = true;
                            calib_crop = None;
                            calib_aspect = None;
                        }
                        b"distortion" => {
                            if in_calibration {
                                if let Some(d) = parse_distortion(&e) {
                                    push_distortion(&mut cur_lens, &mut new_calib_set, calib_crop, calib_aspect, d);
                                }
                            }
                        }
                        b"maker" | b"model" | b"mount" | b"cropfactor" | b"aspect-ratio" => {
                            cur_tag = match name.as_ref() {
                                b"maker"        => Tag::Maker,
                                b"model"        => Tag::Model,
                                b"mount"        => Tag::Mount,
                                b"cropfactor"   => Tag::CropFactor,
                                b"aspect-ratio" => Tag::AspectRatio,
                                _ => unreachable!()
                            };
                        },
                        _ => { }
                    }
                    text_buf.clear();
                }
                quick_xml::events::Event::Empty(e) => {
                    if in_calibration && e.name().as_ref() == b"distortion" {
                        if let Some(d) = parse_distortion(&e) {
                            push_distortion(&mut cur_lens, &mut new_calib_set, calib_crop, calib_aspect, d);
                        }
                    }
                }
                quick_xml::events::Event::Text(e) => {
                    if let Ok(t) = e.decode() {
                        text_buf.push_str(&t);
                    }
                }
                quick_xml::events::Event::GeneralRef(e) => {
                    match e.as_ref() {
                        b"amp"  => text_buf.push('&'),
                        b"lt"   => text_buf.push('<'),
                        b"gt"   => text_buf.push('>'),
                        b"quot" => text_buf.push('"'),
                        b"apos" => text_buf.push('\''),
                        _ => if let Ok(Some(ch)) = e.resolve_char_ref() { text_buf.push(ch); }
                    }
                }
                quick_xml::events::Event::End(e) => {
                    match e.name().as_ref() {
                        b"camera" => {
                            if let Some(cam) = cur_camera.take() {
                                if !cam.model.is_empty() { db.cameras.push(cam); }
                            }
                        }
                        b"lens" => {
                            if let Some(lens) = cur_lens.take() {
                                if !lens.model.is_empty() && !lens.calibrations.is_empty() { db.lenses.push(lens); }
                            }
                        }
                        b"calibration" => {
                            in_calibration = false;
                            // Crop factor / aspect ratio may appear after the distortion entries
                            if let Some(ref mut lens) = cur_lens {
                                if let Some(set) = lens.calibrations.last_mut() {
                                    if let Some(c) = calib_crop { set.crop_factor = c; }
                                    if let Some(a) = calib_aspect { set.aspect_ratio = a; }
                                }
                            }
                        }
                        _ => { }
                    }
                    flush_text!();
                    cur_tag = Tag::None;
                }
                quick_xml::events::Event::Eof => break,
                _ => { }
            }
        }

        Ok(db)
    }

    /// Convert all parsed entries to Gyroflow lens profiles.
    /// One profile is generated per (lens, camera) pair where the camera has a matching
    /// mount and the calibration crop factor covers the camera sensor (see lensfun's
    /// `lfLens::InterpolateDistortion`: a calibration is applicable when
    /// `target_crop / calib_crop >= 0.96`; the calibration with the closest crop factor
    /// is preferred).
    pub fn to_lens_profiles(&self) -> Vec<LensProfile> {
        let mut ret = Vec::new();
        for lens in &self.lenses {
            for camera in &self.cameras {
                if camera.crop_factor <= 0.0 { continue; }
                if !lens.mounts.iter().any(|m| camera.mounts.contains(m)) { continue; }

                // Find the calibration set with the closest applicable crop factor
                let mut best: Option<&LensfunCalibSet> = None;
                let mut best_ratio = f64::MAX;
                for calib in &lens.calibrations {
                    let r = camera.crop_factor / calib.crop_factor;
                    if r >= 0.96 && r < best_ratio {
                        best_ratio = r;
                        best = Some(calib);
                    }
                }
                let Some(calib) = best else { continue };

                // Take into account just the first encountered distortion model (like lensfun does)
                let Some(model) = calib.distortions.first().map(|d| d.model.clone()) else { continue };
                let distortions: Vec<&LensfunDistortion> = calib.distortions.iter().filter(|d| d.model == model).collect();
                if distortions.is_empty() { continue; }

                if let Some(profile) = build_lens_profile(lens, camera, calib, &model, &distortions) {
                    ret.push(profile);
                }
            }
        }
        ret
    }
}

fn push_distortion(cur_lens: &mut Option<LensfunLens>, new_calib_set: &mut bool, calib_crop: Option<f64>, calib_aspect: Option<f64>, d: LensfunDistortion) {
    if let Some(lens) = cur_lens {
        // A <distortion> element belongs to the current <calibration> block. Crop factor
        // and aspect ratio can be specified either at lens level or per calibration block.
        if lens.calibrations.is_empty() || *new_calib_set {
            lens.calibrations.push(LensfunCalibSet {
                crop_factor:  if lens.crop_factor > 0.0 { lens.crop_factor } else { 1.0 },
                aspect_ratio: if lens.aspect_ratio > 0.0 { lens.aspect_ratio } else { 1.5 },
                distortions: Vec::new(),
            });
            *new_calib_set = false;
        }
        let set = lens.calibrations.last_mut().unwrap();
        if let Some(c) = calib_crop { set.crop_factor = c; }
        if let Some(a) = calib_aspect { set.aspect_ratio = a; }
        set.distortions.push(d);
    }
}

fn parse_distortion(e: &quick_xml::events::BytesStart) -> Option<LensfunDistortion> {
    let mut model = String::new();
    let mut focal = 0.0;
    let mut real_focal = None;
    let (mut a, mut b, mut c) = (0.0, 0.0, 0.0);
    let (mut k1, mut k2) = (0.0, 0.0);
    for attr in e.attributes().flatten() {
        let val = attr.unescape_value().unwrap_or_default().to_string();
        match attr.key.as_ref() {
            b"model"      => model = val,
            b"focal"      => focal = val.trim_end_matches("mm").trim().parse().unwrap_or(0.0),
            b"real-focal" => real_focal = val.trim_end_matches("mm").trim().parse().ok(),
            b"a" => a = val.parse().unwrap_or(0.0),
            b"b" => b = val.parse().unwrap_or(0.0),
            b"c" => c = val.parse().unwrap_or(0.0),
            b"k1" => k1 = val.parse().unwrap_or(0.0),
            b"k2" => k2 = val.parse().unwrap_or(0.0),
            _ => { }
        }
    }
    if focal <= 0.0 { return None; }
    let terms = match model.as_str() {
        "ptlens" => vec![a, b, c],
        "poly3"  => vec![k1],
        "poly5"  => vec![k1, k2],
        _ => return None // unsupported model (e.g. "acm"), skip
    };
    Some(LensfunDistortion { model, focal, real_focal, terms })
}

/// Convert a single distortion calibration point to Gyroflow space:
/// rescale the hugin coefficients (ported from lensfun's `rescale_polynomial_coefficients`,
/// see also `stabilization/distortion_models/*::rescale_coeffs`) and synthesize a pinhole
/// camera matrix from the real focal length.
fn convert_distortion(d: &LensfunDistortion, calib_crop: f64, aspect_ratio: f64, target_crop: f64) -> Option<(Vec<f64>, [[f64; 3]; 3], f64)> {
    let real_focal = d.effective_real_focal();
    if real_focal <= 0.0 { return None; }

    let hugin_scale = FULL_FRAME_DIAG / calib_crop / aspect_ratio.hypot(1.0) / 2.0;
    let hugin_scaling = real_focal / hugin_scale;

    let mut coeffs = d.terms.clone();
    match d.model.as_str() {
        "ptlens" => PtLens::rescale_coeffs(&mut coeffs, hugin_scaling),
        "poly3"  => Poly3::rescale_coeffs(&mut coeffs, hugin_scaling),
        "poly5"  => Poly5::rescale_coeffs(&mut coeffs, hugin_scaling),
        _ => return None
    }
    if !coeffs.iter().all(|x| x.is_finite()) { return None; }

    // Synthesize a pinhole camera matrix for a canonical frame with the calibration
    // aspect ratio. fx[px] = real_focal[mm] * image_diag[px] / sensor_diag[mm]
    let h = CANONICAL_HEIGHT;
    let w = ((h * aspect_ratio).round() as usize + 1) & !1; // round to even
    let sensor_diag = FULL_FRAME_DIAG / target_crop;
    let fx = real_focal * (w as f64).hypot(h) / sensor_diag;
    let camera_matrix = [
        [fx, 0.0, w as f64 / 2.0],
        [0.0, fx, h / 2.0],
        [0.0, 0.0, 1.0]
    ];

    Some((coeffs, camera_matrix, real_focal))
}

fn build_lens_profile(lens: &LensfunLens, camera: &LensfunCamera, calib: &LensfunCalibSet, model: &str, distortions: &[&LensfunDistortion]) -> Option<LensProfile> {
    let mut distortions: Vec<&LensfunDistortion> = distortions.to_vec();
    distortions.sort_by(|a, b| a.focal.partial_cmp(&b.focal).unwrap_or(std::cmp::Ordering::Equal));

    let h = CANONICAL_HEIGHT;
    let w = ((h * calib.aspect_ratio).round() as usize + 1) & !1;

    let base = convert_distortion(distortions[0], calib.crop_factor, calib.aspect_ratio, camera.crop_factor)?;

    let strip_maker = |maker: &str, model: &str| -> String {
        model.strip_prefix(maker).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).unwrap_or_else(|| model.trim().to_string())
    };

    let mut profile = LensProfile::default();
    profile.note = "Converted from the lensfun database".to_string();
    profile.calibrated_by = "lensfun".to_string();
    profile.camera_brand = camera.maker.clone();
    profile.camera_model = strip_maker(&camera.maker, &camera.model);
    profile.lens_model = strip_maker(&lens.maker, &lens.model);
    profile.calib_dimension = Dimensions { w, h: h as usize };
    profile.orig_dimension = Dimensions { w, h: h as usize };
    profile.fisheye_params = CameraParams {
        RMS_error: 0.0,
        camera_matrix: base.1.to_vec(),
        distortion_coeffs: base.0,
        radial_distortion_limit: None,
    };
    profile.distortion_model = Some(model.to_string());
    profile.focal_length = Some(base.2);
    profile.crop_factor = Some(camera.crop_factor);
    profile.official = true;
    profile.identifier = format!("lensfun:{}|{}", camera.model, lens.model);

    // Zoom lenses have calibration points at multiple focal lengths - expose them through
    // the standard `interpolations` mechanism, so `get_interpolated_lens_at` can pick or
    // interpolate the right coefficients based on the focal length from the file metadata
    if distortions.len() > 1 {
        let mut map = serde_json::Map::new();
        for d in &distortions {
            if let Some((coeffs, matrix, real_focal)) = convert_distortion(d, calib.crop_factor, calib.aspect_ratio, camera.crop_factor) {
                map.insert(format!("{}", d.focal), serde_json::json!({
                    "camera_matrix": matrix,
                    "distortion_coeffs": coeffs,
                    "focal_length": real_focal,
                }));
            }
        }
        if !map.is_empty() {
            profile.interpolations = Some(serde_json::Value::Object(map));
        }
    }

    profile.init();
    profile.name = profile.get_name();
    Some(profile)
}

/// Port of lensfun's `_lf_hermite`/`_lf_interpolate` (libs/lensfun/auxfun.cpp):
/// cubic Hermite spline interpolation. `None` marks a missing outer data point.
fn hermite_interpolate(y1: Option<f64>, y2: f64, y3: f64, y4: Option<f64>, t: f64) -> f64 {
    let tg2 = match y1 { Some(y1) => (y3 - y1) * 0.5, None => y3 - y2 };
    let tg3 = match y4 { Some(y4) => (y4 - y2) * 0.5, None => y3 - y2 };

    let t2 = t * t;
    let t3 = t2 * t;
    (2.0 * t3 - 3.0 * t2 + 1.0) * y2 +
    (t3 - 2.0 * t2 + t) * tg2 +
    (-2.0 * t3 + 3.0 * t2) * y3 +
    (t3 - t2) * tg3
}

/// Port of lensfun's `lfLens::InterpolateDistortion` (libs/lensfun/lens.cpp):
/// interpolate the raw (hugin) distortion terms of a single calibration set at an
/// arbitrary focal length. Before interpolation, each term is multiplied by the focal
/// length of its data point (distortion terms roughly follow a 1/f law, see
/// `__parameter_scales` in lens.cpp); the result is divided by the target focal length.
pub fn interpolate_distortion(points: &[LensfunDistortion], focal: f64) -> Option<LensfunDistortion> {
    let model = points.first()?.model.clone();
    let mut spline: [Option<&LensfunDistortion>; 4] = [None; 4];
    let mut spline_dist = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];

    for c in points.iter().filter(|c| c.model == model) {
        let df = focal - c.focal;
        if df == 0.0 {
            return Some(c.clone()); // Exact match, don't interpolate
        }
        // __insert_spline
        if df < 0.0 {
            if df > spline_dist[1] {
                spline_dist[0] = spline_dist[1]; spline_dist[1] = df;
                spline[0] = spline[1]; spline[1] = Some(c);
            } else if df > spline_dist[0] {
                spline_dist[0] = df; spline[0] = Some(c);
            }
        } else {
            if df < spline_dist[2] {
                spline_dist[3] = spline_dist[2]; spline_dist[2] = df;
                spline[3] = spline[2]; spline[2] = Some(c);
            } else if df < spline_dist[3] {
                spline_dist[3] = df; spline[3] = Some(c);
            }
        }
    }

    let (p1, p2) = match (spline[1], spline[2]) {
        (Some(p1), Some(p2)) => (p1, p2),
        (Some(p1), None) => return Some(p1.clone()),
        (None, Some(p2)) => return Some(p2.clone()),
        (None, None) => return None,
    };

    let t = (focal - p1.focal) / (p2.focal - p1.focal);

    let num_terms = p1.terms.len().max(p2.terms.len());
    let mut terms = Vec::with_capacity(num_terms);
    for i in 0..num_terms {
        let at = |p: &LensfunDistortion| p.terms.get(i).copied().unwrap_or(0.0);
        // Parameter preconditioning: scale by the focal length of the data point
        let y1 = spline[0].map(|p| at(p) * p.focal);
        let y2 = at(p1) * p1.focal;
        let y3 = at(p2) * p2.focal;
        let y4 = spline[3].map(|p| at(p) * p.focal);
        terms.push(hermite_interpolate(y1, y2, y3, y4, t) / focal);
    }

    let real_focal = hermite_interpolate(
        spline[0].map(|p| p.effective_real_focal()),
        p1.effective_real_focal(),
        p2.effective_real_focal(),
        spline[3].map(|p| p.effective_real_focal()),
        t
    );

    Some(LensfunDistortion { model, focal, real_focal: Some(real_focal), terms })
}

#[cfg(test)]
mod tests;
