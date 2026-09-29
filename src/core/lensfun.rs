// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Gyroflow contributors

//! Lensfun XML database support.
//!
//! Parses Lensfun database files (`*.xml`, database version 1 and 2) for the
//! distortion models gyroflow already implements (`poly3`, `poly5`, `ptlens`)
//! and converts calibration entries into [`LensProfile`] records, so dropping
//! Lensfun XML files into the lens profiles directory makes them usable like
//! any other profile. TCA and vignetting calibrations are parsed past but not
//! converted; gyroflow does not model them.
//!
//! Two pieces of upstream Lensfun behavior are ported directly:
//! - focal-length interpolation: `lfLens::InterpolateDistortion`
//!   (4-point Hermite spline over focal length, `term * focal` parameter
//!   scaling for these models, nearest-entry clamping outside the calibrated
//!   range, and the `>= 0.96` crop-factor gate when picking a calibration set)
//! - the Hugin-to-gyroflow coefficient rescaling, using the formula already
//!   sketched in the TODO block at the bottom of
//!   `stabilization/distortion_models/poly3.rs`, dispatched to the existing
//!   (previously unused) `rescale_coeffs` functions on the distortion models.

use crate::lens_profile::{CameraParams, Dimensions, LensProfile};
use crate::stabilization::distortion_models::DistortionModel;

/// Diagonal of the full-frame reference sensor, in millimeters.
const FULL_FRAME_DIAGONAL_MM: f64 = 43.266615305567875; // 36.hypot(24)

#[derive(Debug, Clone, PartialEq)]
pub struct LensfunCalibDistortion {
    /// gyroflow distortion model id: "poly3" | "poly5" | "ptlens"
    pub model: &'static str,
    /// Nominal focal length of this calibration, in mm.
    pub focal: f64,
    /// Effective focal length, resolved exactly like Lensfun's `database.cpp`:
    /// `real-focal` attribute when present, otherwise `focal * (1 - a - b - c)`
    /// for ptlens, `focal * (1 - k1)` for poly3, `focal` for poly5.
    pub real_focal: f64,
    /// poly3: `[k1, 0, 0]`, poly5: `[k1, k2, 0]`, ptlens: `[a, b, c]`
    pub terms: [f64; 3],
}

impl LensfunCalibDistortion {
    pub fn num_terms(&self) -> usize {
        match self.model {
            "poly3" => 1,
            "poly5" => 2,
            "ptlens" => 3,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LensfunCalibrationSet {
    pub crop_factor: f64,
    pub aspect_ratio: f64,
    pub distortions: Vec<LensfunCalibDistortion>,
}

#[derive(Debug, Clone)]
pub struct LensfunLens {
    pub maker: String,
    pub model: String,
    pub mounts: Vec<String>,
    pub crop_factor: f64,
    pub aspect_ratio: f64,
    pub calibration_sets: Vec<LensfunCalibrationSet>,
}

#[derive(Debug, Clone)]
pub struct LensfunCamera {
    pub maker: String,
    pub model: String,
    pub mounts: Vec<String>,
    pub crop_factor: f64,
}

#[derive(Debug)]
pub enum LensfunError {
    Xml(roxmltree::Error),
    NotALensfunDatabase,
}

impl std::fmt::Display for LensfunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Xml(e) => write!(f, "XML parse error: {e}"),
            Self::NotALensfunDatabase => write!(f, "not a <lensdatabase> document"),
        }
    }
}
impl std::error::Error for LensfunError {}
impl From<roxmltree::Error> for LensfunError {
    fn from(e: roxmltree::Error) -> Self { Self::Xml(e) }
}

#[derive(Default, Debug)]
pub struct LensfunDatabase {
    pub cameras: Vec<LensfunCamera>,
    pub lenses: Vec<LensfunLens>,
}


/// Remove a leading `<!DOCTYPE ...>` declaration (with optional internal
/// subset `[...]`) so roxmltree accepts real lensfun database files.
fn strip_doctype(xml: &str) -> &str {
    let t = xml.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    if !t.starts_with("<?xml") {
        // no XML declaration - check for doctype directly
        if !t.starts_with("<!DOCTYPE") { return xml; }
    }
    let start = match t.find("<!DOCTYPE") {
        Some(i) => i,
        None => return xml,
    };
    let rest = &t[start..];
    // find the closing '>' that is not inside an internal subset
    let mut depth = 0usize;
    for (i, c) in rest.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            '>' if depth == 0 => {
                let offset = xml.len() - t.len() + start + i + 1;
                return &xml[offset..];
            }
            _ => {}
        }
    }
    xml
}

fn child_text(node: roxmltree::Node, tag: &str) -> Option<String> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == tag)
        .map(|n| n.text().unwrap_or_default().trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Lensfun allows localized duplicates like `<model lang="en">...</model>`;
/// prefer the element without a `lang` attribute, then fall back to the first.
fn child_text_unlocalized(node: roxmltree::Node, tag: &str) -> Option<String> {
    let mut first: Option<String> = None;
    for n in node.children().filter(|n| n.is_element() && n.tag_name().name() == tag) {
        let text = n.text().unwrap_or_default().trim().to_string();
        if text.is_empty() { continue; }
        if n.attribute("lang").is_none() { return Some(text); }
        if first.is_none() { first = Some(text); }
    }
    first
}

fn child_texts(node: roxmltree::Node, tag: &str) -> Vec<String> {
    node.children()
        .filter(|n| n.is_element() && n.tag_name().name() == tag)
        .filter_map(|n| {
            let t = n.text().unwrap_or_default().trim().to_string();
            if t.is_empty() { None } else { Some(t) }
        })
        .collect()
}

fn attr_f64(node: &roxmltree::Node, name: &str) -> Option<f64> {
    node.attribute(name).and_then(|v| v.trim().parse::<f64>().ok())
}

/// Aspect ratios appear either as a decimal (`1.5`) or as a ratio (`3:2`).
fn parse_aspect(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some((a, b)) = s.split_once(':') {
        let a = a.trim().parse::<f64>().ok()?;
        let b = b.trim().parse::<f64>().ok()?;
        if b != 0.0 { return Some(a / b); }
        return None;
    }
    s.parse::<f64>().ok()
}

fn parse_distortion(node: roxmltree::Node) -> Option<LensfunCalibDistortion> {
    let (model, terms): (&str, [f64; 3]) = match node.attribute("model")? {
        "poly3" => ("poly3", [attr_f64(&node, "k1")?, 0.0, 0.0]),
        "poly5" => ("poly5", [attr_f64(&node, "k1")?, attr_f64(&node, "k2")?, 0.0]),
        "ptlens" => ("ptlens", [attr_f64(&node, "a")?, attr_f64(&node, "b")?, attr_f64(&node, "c")?]),
        _ => return None, // fisheye models etc. - gyroflow has no matching kernel
    };
    let focal = attr_f64(&node, "focal")?;
    if focal <= 0.0 { return None; }
    // `center-x`/`center-y` are ignored on purpose: gyroflow's distortion
    // kernels assume the distortion center coincides with the principal point.
    let real_focal = attr_f64(&node, "real-focal").filter(|f| *f > 0.0).unwrap_or_else(|| match model {
        "ptlens" => focal * (1.0 - terms[0] - terms[1] - terms[2]),
        "poly3" => focal * (1.0 - terms[0]),
        _ => focal,
    });
    Some(LensfunCalibDistortion { model, focal, real_focal, terms })
}

fn parse_lens(node: roxmltree::Node) -> Option<LensfunLens> {
    // Non-rectilinear lenses (fisheye projections: equisolid, equidistant,
    // stereographic, orthographic, thoby) need a projection conversion that
    // gyroflow applies through its own fisheye pipeline; importing their
    // distortion calibrations as rectilinear profiles would be wrong.
    if let Some(t) = child_text(node, "type") {
        if t != "rectilinear" { return None; }
    }
    let maker = child_text_unlocalized(node, "maker")?;
    let model = child_text_unlocalized(node, "model")?;
    let mounts = child_texts(node, "mount");
    let crop_factor = child_text(node, "cropfactor").and_then(|s| s.parse::<f64>().ok()).filter(|c| *c > 0.0).unwrap_or(1.0);
    let aspect_ratio = child_text(node, "aspect-ratio").and_then(|s| parse_aspect(&s)).filter(|a| *a > 0.0).unwrap_or(1.5);

    let calibration_sets = node.children()
        .filter(|n| n.is_element() && n.tag_name().name() == "calibration")
        .map(|calib| {
            // Database v2 carries per-set overrides as attributes; v1 falls
            // back to the lens-level sensor attributes.
            let set_crop = attr_f64(&calib, "cropfactor").filter(|c| *c > 0.0).unwrap_or(crop_factor);
            let set_aspect = calib.attribute("aspect-ratio").and_then(parse_aspect).filter(|a| *a > 0.0).unwrap_or(aspect_ratio);
            let distortions = calib.children()
                .filter(|n| n.is_element() && n.tag_name().name() == "distortion")
                .filter_map(parse_distortion)
                .collect();
            LensfunCalibrationSet { crop_factor: set_crop, aspect_ratio: set_aspect, distortions }
        })
        .collect();
    Some(LensfunLens { maker, model, mounts, crop_factor, aspect_ratio, calibration_sets })
}

impl LensfunDatabase {
    pub fn parse(xml: &str) -> Result<Self, LensfunError> {
        // Real lensfun database files start with `<!DOCTYPE lensdatabase SYSTEM
        // "lensfun-database.dtd">`, which roxmltree refuses. The DTD carries no
        // information we need, so strip it (including any internal subset).
        let xml = strip_doctype(xml);
        let doc = roxmltree::Document::parse(xml)?;
        let root = doc.root_element();
        if root.tag_name().name() != "lensdatabase" { return Err(LensfunError::NotALensfunDatabase); }
        let mut db = LensfunDatabase::default();
        for node in root.children().filter(|n| n.is_element()) {
            match node.tag_name().name() {
                "camera" => {
                    if let (Some(maker), Some(model)) = (child_text_unlocalized(node, "maker"), child_text_unlocalized(node, "model")) {
                        db.cameras.push(LensfunCamera {
                            maker, model,
                            mounts: child_texts(node, "mount"),
                            crop_factor: child_text(node, "cropfactor").and_then(|s| s.parse().ok()).filter(|c| *c > 0.0).unwrap_or(1.0),
                        });
                    }
                }
                "lens" => {
                    if let Some(lens) = parse_lens(node) { db.lenses.push(lens); }
                }
                _ => {}
            }
        }
        Ok(db)
    }

    /// One profile per calibrated focal length of every lens that carries a
    /// supported distortion model. `source_name` is recorded for provenance
    /// (it also keeps identifiers unique between different XML files).
    pub fn to_lens_profiles(&self, source_name: &str) -> Vec<LensProfile> {
        self.lenses.iter().flat_map(|lens| lens.to_lens_profiles(source_name)).collect()
    }
}

pub struct InterpolatedDistortion {
    pub model: &'static str,
    pub focal: f64,
    pub real_focal: f64,
    pub terms: [f64; 3],
    pub calib_crop_factor: f64,
    pub calib_aspect_ratio: f64,
}

/// Port of `_lf_interpolate` (auxfun.cpp): 4-point cubic Hermite spline.
/// `None` plays the role of upstream's FLT_MAX "point does not exist" marker.
fn lf_interpolate(y1: Option<f64>, y2: f64, y3: f64, y4: Option<f64>, t: f64) -> f64 {
    let t2 = t * t;
    let t3 = t2 * t;
    let tg2 = match y1 { None => y3 - y2, Some(v) => (y3 - v) * 0.5 };
    let tg3 = match y4 { None => y3 - y2, Some(v) => (v - y2) * 0.5 };
    (2.0 * t3 - 3.0 * t2 + 1.0) * y2 + (t3 - 2.0 * t2 + t) * tg2 + (-2.0 * t3 + 3.0 * t2) * y3 + (t3 - t2) * tg3
}

/// Port of `__insert_spline` (lens.cpp): keeps the two nearest calibration
/// points below and the two nearest above the requested focal length.
fn insert_spline<'a>(spline: &mut [Option<&'a LensfunCalibDistortion>; 4], dists: &mut [f64; 4], dist: f64, val: &'a LensfunCalibDistortion) {
    if dist < 0.0 {
        if dist > dists[1] {
            dists[0] = dists[1]; dists[1] = dist;
            spline[0] = spline[1]; spline[1] = Some(val);
        } else if dist > dists[0] {
            dists[0] = dist;
            spline[0] = Some(val);
        }
    } else if dist < dists[2] {
        dists[3] = dists[2]; dists[2] = dist;
        spline[3] = spline[2]; spline[2] = Some(val);
    } else if dist < dists[3] {
        dists[3] = dist;
        spline[3] = Some(val);
    }
}

impl LensfunLens {
    /// Port of `lfLens::InterpolateDistortion` (lens.cpp).
    pub fn interpolate_distortion(&self, crop: f64, focal: f64) -> Option<InterpolatedDistortion> {
        // Calibration set with closest crop factor, gated at ratio >= 0.96 so a
        // calibration from a smaller sensor is never reused on a larger one.
        let mut calib_set: Option<&LensfunCalibrationSet> = None;
        let mut crop_ratio = 1e6_f64;
        for set in self.calibration_sets.iter().filter(|s| !s.distortions.is_empty()) {
            let r = crop / set.crop_factor;
            if r >= 0.96 && r < crop_ratio {
                crop_ratio = r;
                calib_set = Some(set);
            }
        }
        let set = calib_set?;

        // Take into account just the first encountered lens model, like upstream.
        let dm = set.distortions.first()?.model;

        let mut spline: [Option<&LensfunCalibDistortion>; 4] = [None; 4];
        let mut dists: [f64; 4] = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];
        for c in set.distortions.iter().filter(|c| c.model == dm) {
            let df = focal - c.focal;
            if df == 0.0 {
                // Exact match found, don't care to interpolate.
                return Some(InterpolatedDistortion {
                    model: dm, focal,
                    real_focal: c.real_focal, terms: c.terms,
                    calib_crop_factor: set.crop_factor, calib_aspect_ratio: set.aspect_ratio,
                });
            }
            insert_spline(&mut spline, &mut dists, df, c);
        }

        let (s0, s1, s2, s3) = (spline[0], spline[1], spline[2], spline[3]);
        let (s1, s2) = match (s1, s2) {
            (Some(a), Some(b)) => (a, b),
            // Outside the calibrated range: clamp to the nearest entry.
            (Some(a), None) | (None, Some(a)) => {
                return Some(InterpolatedDistortion {
                    model: dm, focal,
                    real_focal: a.real_focal, terms: a.terms,
                    calib_crop_factor: set.crop_factor, calib_aspect_ratio: set.aspect_ratio,
                });
            }
            (None, None) => return None,
        };

        let t = (focal - s1.focal) / (s2.focal - s1.focal);
        let real_focal = lf_interpolate(s0.map(|c| c.real_focal), s1.real_focal, s2.real_focal, s3.map(|c| c.real_focal), t);
        // For poly3/poly5/ptlens, `__parameter_scales` leaves the scale factor
        // equal to the focal length: precondition terms with `term * focal`
        // and undo the scaling with the destination focal afterwards.
        let mut terms = [0.0; 3];
        for (i, term) in terms.iter_mut().enumerate() {
            let g = |c: &LensfunCalibDistortion| c.terms[i] * c.focal;
            *term = lf_interpolate(s0.map(|c| g(c)), g(s1), g(s2), s3.map(|c| g(c)), t) / focal;
        }

        Some(InterpolatedDistortion {
            model: dm, focal, real_focal, terms,
            calib_crop_factor: set.crop_factor, calib_aspect_ratio: set.aspect_ratio,
        })
    }

    /// Build one [`LensProfile`] per calibrated focal length of this lens.
    pub fn to_lens_profiles(&self, source_name: &str) -> Vec<LensProfile> {
        let mut out = Vec::new();
        for set in &self.calibration_sets {
            for calib in &set.distortions {
                if let Some(p) = self.profile_at(calib, set, source_name) {
                    out.push(p);
                }
            }
        }
        out
    }

    fn profile_at(&self, calib: &LensfunCalibDistortion, set: &LensfunCalibrationSet, source_name: &str) -> Option<LensProfile> {
        let model = DistortionModel::from_name(calib.model);

        // Rescale coefficients from Lensfun's Hugin normalization (radius
        // normalized by the half-diagonal in mm) into gyroflow's convention
        // (x/z tan-space), using the calibration sensor's crop and aspect.
        // See the derivation at the bottom of `stabilization/distortion_models/poly3.rs`.
        let hugin_scale_in_millimeters = FULL_FRAME_DIAGONAL_MM / set.crop_factor / set.aspect_ratio.hypot(1.0) / 2.0;
        let hugin_scaling = calib.real_focal / hugin_scale_in_millimeters;
        let n = calib.num_terms();
        if n == 0 { return None; }
        let mut k = calib.terms[..n].to_vec();
        if !model.rescale_coeffs(&mut k, hugin_scaling) { return None; }

        // Lensfun calibrations are resolution-independent (aspect-ratio-only),
        // so an arbitrary reference resolution is used; gyroflow rescales the
        // profile to the actual video dimensions downstream.
        let h = 2000.0;
        let w = (h * set.aspect_ratio).round().max(1.0);

        // Pinhole camera matrix from the real focal length and the calibration
        // sensor's pixel pitch. This is also where a full-frame calibration
        // lands correctly on a crop sensor: the pixel pitch shrinks with the
        // sensor size, raising f_px proportionally.
        let sensor_diag_mm = FULL_FRAME_DIAGONAL_MM / set.crop_factor;
        let sensor_w_mm = sensor_diag_mm * set.aspect_ratio / set.aspect_ratio.hypot(1.0);
        let f_px = calib.real_focal / (sensor_w_mm / w);
        let camera_matrix = vec![
            [f_px, 0.0, w / 2.0],
            [0.0, f_px, h / 2.0],
            [0.0, 0.0, 1.0],
        ];

        let slug = |s: &str| s.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect::<String>();
        let mut profile = LensProfile::default();
        profile.camera_brand = format!("Lensfun ({})", self.maker);
        profile.camera_model = if self.mounts.is_empty() { "Any".to_string() } else { self.mounts.join(" / ") };
        profile.lens_model = self.model.clone();
        profile.note = format!("Lensfun database import ({source_name})");
        profile.calibrated_by = "Lensfun".to_string();
        profile.calib_dimension = Dimensions { w: w as usize, h: h as usize };
        profile.orig_dimension = Dimensions { w: w as usize, h: h as usize };
        profile.input_horizontal_stretch = 1.0;
        profile.input_vertical_stretch = 1.0;
        profile.distortion_model = Some(model.id().to_string());
        profile.fisheye_params = CameraParams { RMS_error: 0.0, camera_matrix, distortion_coeffs: k, radial_distortion_limit: None };
        profile.focal_length = Some(calib.focal);
        profile.crop_factor = Some(set.crop_factor);
        profile.identifier = format!("lensfun_{}_{}_{}_{}mm_crop{}", slug(source_name), slug(&self.maker), slug(&self.model), calib.focal, set.crop_factor);
        profile.finalize();
        profile.init();
        Some(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DB: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<lensdatabase version="2">
    <camera>
        <maker>Testcam</maker>
        <model>Crop Body</model>
        <mount>TestMount</mount>
        <cropfactor>1.5</cropfactor>
    </camera>
    <lens>
        <maker>TestMaker</maker>
        <model>18-35mm f/1.8 Test</model>
        <mount>TestMount</mount>
        <cropfactor>1.5</cropfactor>
        <calibration>
            <distortion model="ptlens" focal="18" a="0.004" b="-0.015" c="-0.001"/>
            <distortion model="ptlens" focal="24" a="0.001" b="-0.008" c="-0.002"/>
            <distortion model="ptlens" focal="35" a="-0.002" b="-0.003" c="0.001"/>
        </calibration>
    </lens>
    <lens>
        <maker>TestMaker</maker>
        <model>28mm f/2.8 Prime</model>
        <model lang="en">28mm f/2.8 Prime</model>
        <mount>TestMount</mount>
        <cropfactor>1</cropfactor>
        <aspect-ratio>3:2</aspect-ratio>
        <calibration cropfactor="1" aspect-ratio="3:2">
            <distortion model="poly3" focal="28" k1="-0.028" real-focal="28.8"/>
        </calibration>
        <calibration cropfactor="1.5">
            <distortion model="poly3" focal="28" k1="-0.028"/>
        </calibration>
    </lens>
    <lens>
        <maker>TestMaker</maker>
        <model>12mm f/2.0 Wide</model>
        <mount>TestMount</mount>
        <cropfactor>2</cropfactor>
        <calibration>
            <distortion model="poly5" focal="12" k1="-0.12" k2="0.04"/>
        </calibration>
    </lens>
    <lens>
        <maker>TestMaker</maker>
        <model>50mm f/1.2 NoDistortion</model>
        <mount>TestMount</mount>
        <cropfactor>1</cropfactor>
        <calibration>
            <vignetting model="pa" focal="50" aperture="1.2" distance="1" k1="-0.1" k2="0.0" k3="0.0"/>
        </calibration>
    </lens>
</lensdatabase>"#;

    fn db() -> LensfunDatabase {
        LensfunDatabase::parse(TEST_DB).expect("test db parses")
    }

    #[test]
    fn parses_cameras_lenses_and_calibrations() {
        let db = db();
        assert_eq!(db.cameras.len(), 1);
        assert_eq!(db.cameras[0].crop_factor, 1.5);
        assert_eq!(db.lenses.len(), 4);

        let zoom = &db.lenses[0];
        assert_eq!(zoom.maker, "TestMaker");
        assert_eq!(zoom.model, "18-35mm f/1.8 Test");
        assert_eq!(zoom.mounts, vec!["TestMount".to_string()]);
        assert_eq!(zoom.calibration_sets.len(), 1);
        let set = &zoom.calibration_sets[0];
        assert_eq!(set.crop_factor, 1.5);
        assert_eq!(set.aspect_ratio, 1.5); // default 3:2
        assert_eq!(set.distortions.len(), 3);
        assert_eq!(set.distortions[0].model, "ptlens");
        assert_eq!(set.distortions[0].terms, [0.004, -0.015, -0.001]);

        // v2 per-set attribute overrides
        let prime = &db.lenses[1];
        assert_eq!(prime.calibration_sets.len(), 2);
        assert_eq!(prime.calibration_sets[0].crop_factor, 1.0);
        assert_eq!(prime.calibration_sets[0].aspect_ratio, 1.5);
        assert_eq!(prime.calibration_sets[1].crop_factor, 1.5);
        assert_eq!(prime.calibration_sets[1].aspect_ratio, 1.5); // falls back to lens-level
    }

    #[test]
    fn resolves_real_focal_like_lensfun() {
        let db = db();
        // explicit real-focal wins
        let p1 = &db.lenses[1].calibration_sets[0].distortions[0];
        assert_eq!(p1.real_focal, 28.8);
        // poly3: focal * (1 - k1)
        let p2 = &db.lenses[1].calibration_sets[1].distortions[0];
        assert!((p2.real_focal - 28.0 * (1.0 + 0.028)).abs() < 1e-9);
        // ptlens: focal * (1 - a - b - c)
        let t = &db.lenses[0].calibration_sets[0].distortions[0];
        assert!((t.real_focal - 18.0 * (1.0 - 0.004 + 0.015 + 0.001)).abs() < 1e-9);
        // poly5: focal
        let w = &db.lenses[2].calibration_sets[0].distortions[0];
        assert_eq!(w.real_focal, 12.0);
    }

    #[test]
    fn exact_focal_match_returns_calibration() {
        let db = db();
        let zoom = &db.lenses[0];
        let r = zoom.interpolate_distortion(1.5, 24.0).unwrap();
        assert_eq!(r.model, "ptlens");
        assert_eq!(r.terms, [0.001, -0.008, -0.002]);
        assert_eq!(r.calib_crop_factor, 1.5);
    }

    #[test]
    fn hermite_interpolation_between_focals() {
        // Reference values computed independently from the Hermite formula and
        // the term*focal parameter scaling documented in lens.cpp.
        let db = db();
        let zoom = &db.lenses[0];
        let r = zoom.interpolate_distortion(1.5, 28.0).unwrap();
        // Bracketing entries are 35mm (below) and 24mm (above), with 18mm as
        // the second-above spline point: t = (28-35)/(24-35) = 7/11.
        let t = 7.0 / 11.0;
        let expected = |i: usize| {
            let y2 = [-0.002, -0.003, 0.001][i] * 35.0; // nearest below
            let y3 = [0.001, -0.008, -0.002][i] * 24.0; // nearest above
            let y4 = [0.004, -0.015, -0.001][i] * 18.0; // second above
            let tg2 = y3 - y2; // no point below y2: tangent falls back to endpoint difference
            let tg3 = (y4 - y2) * 0.5;
            let t2 = t * t; let t3 = t2 * t;
            ((2.0 * t3 - 3.0 * t2 + 1.0) * y2 + (t3 - 2.0 * t2 + t) * tg2 + (-2.0 * t3 + 3.0 * t2) * y3 + (t3 - t2) * tg3) / 28.0
        };
        for i in 0..3 {
            assert!((r.terms[i] - expected(i)).abs() < 1e-12, "term {i}: {} vs {}", r.terms[i], expected(i));
        }
    }

    #[test]
    fn clamps_to_nearest_focal_outside_range() {
        let db = db();
        let zoom = &db.lenses[0];
        let lo = zoom.interpolate_distortion(1.5, 10.0).unwrap();
        assert_eq!(lo.terms, [0.004, -0.015, -0.001]); // 18mm entry
        let hi = zoom.interpolate_distortion(1.5, 50.0).unwrap();
        assert_eq!(hi.terms, [-0.002, -0.003, 0.001]); // 35mm entry
    }

    #[test]
    fn crop_gate_rejects_larger_target_sensor() {
        let db = db();
        let wide = &db.lenses[2]; // calibrated only on crop 2
        assert!(wide.interpolate_distortion(2.0, 12.0).is_some());
        assert!(wide.interpolate_distortion(1.0, 12.0).is_none()); // 1/2 = 0.5 < 0.96
    }

    #[test]
    fn rescale_math_matches_hand_computed() {
        let db = db();
        let prime = &db.lenses[1];
        let set = &prime.calibration_sets[0];
        let calib = &set.distortions[0]; // poly3, k1 = -0.028, real-focal 28.8
        let profile = prime.profile_at(calib, set, "test").unwrap();

        // Hand computation from the TODO formula in poly3.rs:
        let hugin_scale_mm = 36.0f64.hypot(24.0) / 1.0 / 1.5f64.hypot(1.0) / 2.0;
        let hugin_scaling = 28.8 / hugin_scale_mm;
        let d: f64 = 1.0 + 0.028;
        let expected_k1 = -0.028 * hugin_scaling.powi(2) / d.powi(3);
        assert!((profile.fisheye_params.distortion_coeffs[0] - expected_k1).abs() < 1e-12);
        assert_eq!(profile.distortion_model.as_deref(), Some("poly3"));

        // Camera matrix: full-frame sensor, 3:2 at 2000px high.
        let f_px = 28.8 / (36.0 / 3000.0);
        assert!((profile.fisheye_params.camera_matrix[0][0] - f_px).abs() < 1e-9);
        assert_eq!(profile.calib_dimension.w, 3000);
        assert_eq!(profile.calib_dimension.h, 2000);
        assert_eq!(profile.focal_length, Some(28.0));
        assert_eq!(profile.crop_factor, Some(1.0));
        assert!(!profile.official);
    }

    #[test]
    fn profiles_have_unique_identifiers_and_required_fields() {
        let db = db();
        let profiles = db.to_lens_profiles("test_db.xml");
        // 3 zoom focals + 2 prime calib sets + 1 wide; lens without distortion skipped
        assert_eq!(profiles.len(), 6);
        let ids: std::collections::HashSet<_> = profiles.iter().map(|p| p.identifier.clone()).collect();
        assert_eq!(ids.len(), profiles.len(), "identifiers must be unique");
        for p in &profiles {
            assert!(!p.fisheye_params.camera_matrix.is_empty());
            assert!(!p.fisheye_params.distortion_coeffs.is_empty());
            assert!(p.calib_dimension.w > 0 && p.calib_dimension.h > 0);
            assert!(!p.calibrator_version.is_empty());
            assert_eq!(p.input_horizontal_stretch, 1.0);
        }
    }

    // Numeric A/B against the independent pure-Rust port of Lensfun
    // (`lensfun` crate, itself validated against upstream Lensfun git master):
    // the final distorted pixel position must match through the whole pipeline
    // (interpolate -> rescale -> camera matrix -> gyroflow kernel space).
    #[test]
    fn distorted_pixels_match_lensfun_reference() {
        let mut reference_db = lensfun::db::Database::new();
        reference_db.load_str(TEST_DB).expect("reference db parses");
        let reference_lens = reference_db.lenses.iter().find(|l| l.model.contains("18-35mm")).unwrap();

        let db = db();
        let zoom = &db.lenses[0];

        for &focal in &[18.0_f32, 24.0, 28.0, 35.0] {
            // Reference: lensfun's own modifier, forward (undistorted -> distorted).
            let (w, h) = (3000u32, 2000u32);
            let mut modifier = lensfun::modifier::Modifier::new(&reference_lens, focal, 1.5, w, h, false);
            assert!(modifier.enable_distortion_correction(&reference_lens));

            // This implementation.
            let interp = zoom.interpolate_distortion(1.5, focal as f64).unwrap();
            let set = zoom.calibration_sets.iter().find(|s| (s.crop_factor - 1.5).abs() < 1e-9).unwrap();
            let calib = LensfunCalibDistortion { model: interp.model, focal: interp.focal, real_focal: interp.real_focal, terms: interp.terms };
            let profile = zoom.profile_at(&calib, set, "test").unwrap();
            let gyro_model = DistortionModel::from_name(interp.model);
            let mut params = crate::stabilization::KernelParams::default();
            for (i, v) in profile.fisheye_params.distortion_coeffs.iter().enumerate() {
                params.k[i] = *v as f32;
            }
            let k_mat = &profile.fisheye_params.camera_matrix;
            let (fx, cx) = (k_mat[0][0] as f32, k_mat[0][2] as f32);
            let (fy, cy) = (k_mat[1][1] as f32, k_mat[1][2] as f32);

            for &(px, py) in &[(77.0f32, 51.0f32), (200.0, 150.0), (1500.0, 1000.0), (2900.0, 1950.0), (5.0, 1999.0)] {
                let mut coords = [px, py];
                assert!(modifier.apply_geometry_distortion(px, py, 1, 1, &mut coords));
                let (ref_x, ref_y) = (coords[0], coords[1]);

                // gyroflow: normalize with the camera matrix, distort in x/z space, back to pixels.
                let nx = (px - cx) / fx;
                let ny = (py - cy) / fy;
                let (dx, dy) = gyro_model.distort_point(nx, ny, 1.0, &params);
                let (my_x, my_y) = (dx * fx + cx, dy * fy + cy);

                let err = (my_x - ref_x).hypot(my_y - ref_y);
                assert!(err < 0.05, "focal {focal} px ({px},{py}): reference ({ref_x},{ref_y}) vs ({my_x},{my_y}), err {err}px");
            }
        }
    }


    #[test]
    fn parses_real_lensfun_files_with_doctype() {
        // Real database files carry a DTD doctype header
        let body = TEST_DB.split_once("?>").map(|(_, rest)| rest).unwrap_or(TEST_DB);
        let with_doctype = "<!DOCTYPE lensdatabase SYSTEM \"lensfun-database.dtd\">\n".to_string() + body;
        let db = LensfunDatabase::parse(&with_doctype).unwrap();
        assert_eq!(db.lenses.len(), LensfunDatabase::parse(TEST_DB).unwrap().lenses.len());
    }

    #[test]
    fn skips_non_rectilinear_lens_types() {
        let xml = r#"<?xml version="1.0"?>
<lensdatabase version="2">
  <lens>
    <maker>Canon</maker><model>Fisheye Lens</model><mount>Canon EF</mount>
    <type>equisolid</type><cropfactor>1</cropfactor>
    <calibration><distortion model="ptlens" focal="8" a="0.38" b="-0.71" c="0.40"/></calibration>
  </lens>
  <lens>
    <maker>Canon</maker><model>Rectilinear Lens</model><mount>Canon EF</mount>
    <type>rectilinear</type><cropfactor>1</cropfactor>
    <calibration><distortion model="ptlens" focal="28" a="0.01" b="-0.03" c="0.003"/></calibration>
  </lens>
</lensdatabase>"#;
        let db = LensfunDatabase::parse(xml).unwrap();
        assert_eq!(db.lenses.len(), 1);
        assert_eq!(db.lenses[0].model, "Rectilinear Lens");
    }

}
