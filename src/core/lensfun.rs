// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Gyroflow contributors

//! Lensfun database parser and lens profile synthesis.
//!
//! Parses the Lensfun XML database format (https://lensfun.github.io/)
//! and synthesizes Gyroflow `LensProfile` entries from its distortion calibrations
//! (`poly3`, `poly5`, and `ptlens` models). Rescales coefficients from Lensfun's
//! Hugin-based normalization into Gyroflow's pinhole camera convention.

use crate::lens_profile::{CameraParams, Dimensions};
use crate::stabilization::distortion_models::DistortionModel;
use crate::LensProfile;
use std::collections::HashMap;

/// Full-frame (36x24mm) sensor diagonal in millimeters: hypot(36.0, 24.0)
pub const FULL_FRAME_DIAGONAL_MM: f64 = 43.266615305567875;

/// Lensfun's default aspect ratio when `<aspect-ratio>` is not specified: 3:2 = 1.5
pub const DEFAULT_ASPECT_RATIO: f64 = 1.5;

/// Reference frame height in pixels for synthesized lens profiles.
/// Calibration dimensions scale proportionally with target video dimensions.
pub const REFERENCE_HEIGHT_PX: f64 = 2000.0;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LensfunDistortion {
    pub model: String,
    pub focal: f64,
    pub real_focal: Option<f64>,
    /// Coefficients in the model's own order: `[k1]` for poly3; `[k1, k2]` for poly5; `[a, b, c]` for ptlens.
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

/// Parse an `<aspect-ratio>` value, either a ratio ("3:2", "4:3", "16:9") or a float ("1.5").
pub fn parse_aspect_ratio(s: &str) -> Option<f64> {
    if let Some((w, h)) = s.split_once(':') {
        let (w, h) = (w.trim().parse::<f64>().ok()?, h.trim().parse::<f64>().ok()?);
        if h > 0.0 && w > 0.0 {
            Some(w / h)
        } else {
            None
        }
    } else {
        s.trim().parse::<f64>().ok().filter(|v| *v > 0.0)
    }
}

/// Derive real focal length from model coefficients when not explicitly provided in XML.
/// Ported from Lensfun database logic (database.cpp).
pub fn derive_real_focal(model: &str, k: &[f64], focal: f64) -> f64 {
    match model {
        "poly3" => focal * (1.0 - k.first().copied().unwrap_or(0.0)),
        "ptlens" => {
            let a = k.first().copied().unwrap_or(0.0);
            let b = k.get(1).copied().unwrap_or(0.0);
            let c = k.get(2).copied().unwrap_or(0.0);
            focal * (1.0 - a - b - c)
        }
        _ => focal,
    }
}

/// Compute Hugin scaling factor:
/// ratio between real focal length and sensor half-height in mm.
pub fn hugin_scaling(
    model: &str,
    k: &[f64],
    focal: f64,
    real_focal: Option<f64>,
    crop_factor: f64,
    aspect_ratio: f64,
) -> f64 {
    let real_focal = real_focal.unwrap_or_else(|| derive_real_focal(model, k, focal));
    let crop = if crop_factor > 0.0 { crop_factor } else { 1.0 };
    let aspect = if aspect_ratio > 0.0 {
        aspect_ratio
    } else {
        DEFAULT_ASPECT_RATIO
    };
    let half_height_mm = FULL_FRAME_DIAGONAL_MM / crop / aspect.hypot(1.0) / 2.0;
    if half_height_mm.abs() < f64::EPSILON {
        return 1.0;
    }
    real_focal / half_height_mm
}

/// Rescale polynomial coefficients from Lensfun normalization to Gyroflow convention.
pub fn rescale_coeffs(model: &str, k: &mut [f64], hugin_scaling: f64) {
    DistortionModel::rescale_model_coeffs(model, k, hugin_scaling);
}

/// Parse a Lensfun XML database document and return every `<lens>` entry
/// having at least one recognized (`poly3`, `poly5`, or `ptlens`) distortion calibration.
pub fn parse_lenses(xml: &str) -> Vec<LensfunLens> {
    let opt = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = match roxmltree::Document::parse_with_options(xml, opt) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("Failed to parse Lensfun XML: {:?}", e);
            return Vec::new();
        }
    };

    // First pass: collect camera mounts to crop factors fallback
    let mut mount_crops: HashMap<String, f64> = HashMap::new();
    for node in doc.descendants() {
        if node.is_element() && node.tag_name().name() == "camera" {
            let mut crop = None;
            let mut mount = None;
            for child in node.children() {
                if child.is_element() {
                    match child.tag_name().name() {
                        "cropfactor" => {
                            if let Some(txt) = child.text() {
                                crop = txt.trim().parse::<f64>().ok();
                            }
                        }
                        "mount" => {
                            if let Some(txt) = child.text() {
                                mount = Some(txt.trim().to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
            if let (Some(m), Some(c)) = (mount, crop.filter(|&c| c > 0.0)) {
                mount_crops.entry(m).or_insert(c);
            }
        }
    }

    let mut lenses = Vec::new();

    for node in doc.descendants() {
        if node.is_element() && node.tag_name().name() == "lens" {
            let mut maker = String::new();
            let mut model = String::new();
            let mut mounts = Vec::new();
            let mut crop_factor = 0.0;
            let mut aspect_ratio = DEFAULT_ASPECT_RATIO;
            let mut distortions = Vec::new();

            for child in node.children() {
                if !child.is_element() {
                    continue;
                }
                let tag = child.tag_name().name();
                match tag {
                    "maker" => {
                        if let Some(t) = child.text() {
                            maker = t.trim().to_string();
                        }
                    }
                    "model" => {
                        if let Some(t) = child.text().filter(|_| model.is_empty()) {
                            model = t.trim().to_string();
                        }
                    }
                    "mount" => {
                        if let Some(t) = child.text() {
                            let m = t.trim().to_string();
                            if !m.is_empty() {
                                mounts.push(m);
                            }
                        }
                    }
                    "cropfactor" => {
                        if let Some(Ok(c)) = child.text().map(|t| t.trim().parse::<f64>()) {
                            crop_factor = c;
                        }
                    }
                    "aspect-ratio" => {
                        if let Some(ar) = child.text().and_then(|t| parse_aspect_ratio(t.trim())) {
                            aspect_ratio = ar;
                        }
                    }
                    "calibration" => {
                        for cal_child in child.children() {
                            if !cal_child.is_element() {
                                continue;
                            }
                            if cal_child.tag_name().name() == "distortion" {
                                let dist_model =
                                    cal_child.attribute("model").unwrap_or_default().to_string();
                                let focal = cal_child
                                    .attribute("focal")
                                    .and_then(|v| v.parse::<f64>().ok());
                                let real_focal = cal_child
                                    .attribute("real-focal")
                                    .and_then(|v| v.parse::<f64>().ok());
                                let k = match dist_model.as_str() {
                                    "poly3" => cal_child
                                        .attribute("k1")
                                        .and_then(|v| v.parse::<f64>().ok())
                                        .map(|k1| vec![k1]),
                                    "poly5" => {
                                        let k1 = cal_child
                                            .attribute("k1")
                                            .and_then(|v| v.parse::<f64>().ok());
                                        let k2 = cal_child
                                            .attribute("k2")
                                            .and_then(|v| v.parse::<f64>().ok());
                                        match (k1, k2) {
                                            (Some(k1), Some(k2)) => Some(vec![k1, k2]),
                                            (Some(k1), None) => Some(vec![k1, 0.0]),
                                            _ => None,
                                        }
                                    }
                                    "ptlens" => {
                                        let a = cal_child
                                            .attribute("a")
                                            .and_then(|v| v.parse::<f64>().ok())
                                            .unwrap_or(0.0);
                                        let b = cal_child
                                            .attribute("b")
                                            .and_then(|v| v.parse::<f64>().ok())
                                            .unwrap_or(0.0);
                                        let c = cal_child
                                            .attribute("c")
                                            .and_then(|v| v.parse::<f64>().ok())
                                            .unwrap_or(0.0);
                                        Some(vec![a, b, c])
                                    }
                                    _ => None,
                                };

                                if let (Some(focal), Some(k)) =
                                    (focal.filter(|&f| f > 0.0), k.filter(|k| !k.is_empty()))
                                {
                                    distortions.push(LensfunDistortion {
                                        model: dist_model,
                                        focal,
                                        real_focal,
                                        k,
                                    });
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            if crop_factor <= 0.0 {
                for m in &mounts {
                    if let Some(&c) = mount_crops.get(m) {
                        crop_factor = c;
                        break;
                    }
                }
                if crop_factor <= 0.0 {
                    crop_factor = 1.0;
                }
            }

            if !distortions.is_empty() && (!maker.is_empty() || !model.is_empty()) {
                distortions.sort_by(|a, b| {
                    a.focal
                        .partial_cmp(&b.focal)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                distortions.dedup_by(|a, b| (a.focal - b.focal).abs() < 1e-4 && a.model == b.model);
                lenses.push(LensfunLens {
                    maker,
                    model,
                    mounts,
                    crop_factor,
                    aspect_ratio,
                    distortions,
                });
            }
        }
    }

    lenses
}

/// Parse a Lensfun XML document and convert every calibrated focal length into a `LensProfile`.
pub fn parse_xml(xml: &str, file_path: &str) -> Vec<LensProfile> {
    let lenses = parse_lenses(xml);
    let mut profiles = Vec::new();
    for lens in &lenses {
        for d in &lens.distortions {
            if let Some(prof) = distortion_to_profile(lens, d, file_path) {
                profiles.push(prof);
            }
        }
    }
    profiles
}

/// Build a Gyroflow `LensProfile` for one Lensfun lens at a specific calibrated distortion point.
pub fn distortion_to_profile(
    lens: &LensfunLens,
    d: &LensfunDistortion,
    file_path: &str,
) -> Option<LensProfile> {
    if d.k.is_empty() || lens.crop_factor <= 0.0 || lens.aspect_ratio <= 0.0 {
        return None;
    }

    let aspect = lens.aspect_ratio;
    let scaling = hugin_scaling(
        &d.model,
        &d.k,
        d.focal,
        d.real_focal,
        lens.crop_factor,
        aspect,
    );
    let mut k = d.k.clone();
    rescale_coeffs(&d.model, &mut k, scaling);

    let height = REFERENCE_HEIGHT_PX;
    let width = (height * aspect).round();
    let f_px = scaling * height / 2.0;
    if !f_px.is_finite() || f_px <= 0.0 {
        return None;
    }

    let cx = width / 2.0;
    let cy = height / 2.0;

    let mut profile = LensProfile::default();
    profile.camera_brand = lens.maker.clone();
    let trimmed_model = if lens
        .model
        .to_lowercase()
        .starts_with(&lens.maker.to_lowercase())
    {
        lens.model[lens.maker.len()..].trim().to_string()
    } else {
        lens.model.clone()
    };
    profile.camera_model = if trimmed_model.is_empty() {
        lens.model.clone()
    } else {
        trimmed_model
    };
    profile.lens_model = lens.mounts.join(", ");
    profile.camera_setting = if d.focal.fract().abs() < 1e-4 {
        format!("{:.0}mm (Lensfun)", d.focal)
    } else {
        format!("{:.1}mm (Lensfun)", d.focal)
    };
    profile.calib_dimension = Dimensions {
        w: width as usize,
        h: height as usize,
    };
    profile.orig_dimension = Dimensions {
        w: width as usize,
        h: height as usize,
    };
    profile.focal_length = Some(d.focal);
    profile.crop_factor = Some(lens.crop_factor);
    profile.distortion_model = Some(d.model.clone());
    profile.calibrated_by = "Lensfun".to_string();
    profile.calibrator_version = "Lensfun".to_string();
    profile.official = true;
    profile.path_to_file = file_path.to_string();

    let id_str = format!(
        "lensfun_{}_{}_{}_{:.2}mm",
        lens.maker, lens.model, d.model, d.focal
    );
    profile.identifier = id_str
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();

    profile.fisheye_params = CameraParams {
        RMS_error: 0.0,
        camera_matrix: vec![[f_px, 0.0, cx], [0.0, f_px, cy], [0.0, 0.0, 1.0]],
        distortion_coeffs: k,
        radial_distortion_limit: None,
    };
    profile.init();

    // Populate interpolations if this lens has multiple calibration points
    if lens.distortions.len() > 1 {
        let mut map = serde_json::Map::new();
        for other in &lens.distortions {
            let other_scaling = hugin_scaling(
                &other.model,
                &other.k,
                other.focal,
                other.real_focal,
                lens.crop_factor,
                aspect,
            );
            let mut other_k = other.k.clone();
            rescale_coeffs(&other.model, &mut other_k, other_scaling);
            let other_f_px = other_scaling * height / 2.0;

            let mut entry = serde_json::Map::new();
            entry.insert("focal_length".into(), serde_json::json!(other.focal));
            entry.insert(
                "camera_matrix".into(),
                serde_json::json!([
                    [other_f_px, 0.0, cx],
                    [0.0, other_f_px, cy],
                    [0.0, 0.0, 1.0]
                ]),
            );
            entry.insert("distortion_coeffs".into(), serde_json::json!(other_k));
            map.insert(
                format!("{:.4}", other.focal),
                serde_json::Value::Object(entry),
            );
        }
        profile.interpolations = Some(serde_json::Value::Object(map));
    }

    let to_checksum = format!(
        "{}|{}{}|{:.8}{:.8}|{:.8}{:.8}|{:.8}{:.8}{:.8}{:.8}",
        profile.identifier,
        profile.calib_dimension.w,
        profile.calib_dimension.h,
        profile.fisheye_params.camera_matrix[0][0],
        profile.fisheye_params.camera_matrix[1][1],
        profile.fisheye_params.camera_matrix[0][2],
        profile.fisheye_params.camera_matrix[1][2],
        profile
            .fisheye_params
            .distortion_coeffs
            .first()
            .unwrap_or(&0.0),
        profile
            .fisheye_params
            .distortion_coeffs
            .get(1)
            .unwrap_or(&0.0),
        profile
            .fisheye_params
            .distortion_coeffs
            .get(2)
            .unwrap_or(&0.0),
        profile
            .fisheye_params
            .distortion_coeffs
            .get(3)
            .unwrap_or(&0.0)
    );
    profile.checksum = Some(format!("{:08x}", crc32fast::hash(to_checksum.as_bytes())));
    profile.name = profile.get_display_name();

    Some(profile)
}

/// Port of Lensfun's `lfLens::InterpolateDistortion`.
/// Interpolates distortion coefficients at an arbitrary focal length for zoom lenses.
pub fn interpolate_distortion(
    lens: &LensfunLens,
    camera_crop: f64,
    focal: f64,
) -> Option<LensfunDistortion> {
    if lens.distortions.is_empty() || focal <= 0.0 || camera_crop <= 0.0 {
        return None;
    }
    // Tolerance rule ported from Lensfun: calibration sensor must be at least as large
    // as target sensor, with a 4% margin (0.96 tolerance).
    if lens.crop_factor * 0.96 > camera_crop {
        return None;
    }

    let model = lens.distortions[0].model.clone();

    // 4-point Hermite spline window: [s0, s1, s2, s3]
    let mut spline: [Option<&LensfunDistortion>; 4] = [None; 4];
    let mut spline_dist = [f64::MIN, f64::MIN, f64::MAX, f64::MAX];

    for d in &lens.distortions {
        if d.model != model {
            continue;
        }
        let df = d.focal - focal;
        if df.abs() < 1e-6 {
            return Some(d.clone());
        }
        if df < 0.0 {
            if df > spline_dist[1] {
                spline_dist[0] = spline_dist[1];
                spline_dist[1] = df;
                spline[0] = spline[1];
                spline[1] = Some(d);
            } else if df > spline_dist[0] {
                spline_dist[0] = df;
                spline[0] = Some(d);
            }
        } else if df < spline_dist[2] {
            spline_dist[3] = spline_dist[2];
            spline_dist[2] = df;
            spline[3] = spline[2];
            spline[2] = Some(d);
        } else if df < spline_dist[3] {
            spline_dist[3] = df;
            spline[3] = Some(d);
        }
    }

    let (s1, s2) = match (spline[1], spline[2]) {
        (Some(s1), Some(s2)) => (s1, s2),
        // Clamping outside the calibrated range
        (Some(s), None) | (None, Some(s)) => return Some(s.clone()),
        (None, None) => return None,
    };

    let t = (focal - s1.focal) / (s2.focal - s1.focal);
    let real_focal = |d: &LensfunDistortion| {
        d.real_focal
            .unwrap_or_else(|| derive_real_focal(&d.model, &d.k, d.focal))
    };
    let interp_rf = hermite_interpolate(
        spline[0].map(real_focal),
        real_focal(s1),
        real_focal(s2),
        spline[3].map(real_focal),
        t,
    );

    let n = s1.k.len().min(s2.k.len());
    let mut k = vec![0.0; n];
    for (i, ki) in k.iter_mut().enumerate() {
        let scaled_edge =
            |d: Option<&LensfunDistortion>| d.and_then(|d| d.k.get(i).map(|v| v * d.focal));
        *ki = hermite_interpolate(
            scaled_edge(spline[0]),
            s1.k[i] * s1.focal,
            s2.k[i] * s2.focal,
            scaled_edge(spline[3]),
            t,
        ) / focal;
    }

    Some(LensfunDistortion {
        model,
        focal,
        real_focal: Some(interp_rf),
        k,
    })
}

fn hermite_interpolate(y1: Option<f64>, y2: f64, y3: f64, y4: Option<f64>, t: f64) -> f64 {
    let t2 = t * t;
    let t3 = t2 * t;
    let tg2 = y1.map_or(y3 - y2, |y1| (y3 - y1) * 0.5);
    let tg3 = y4.map_or(y3 - y2, |y4| (y4 - y2) * 0.5);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y2
        + (t3 - 2.0 * t2 + t) * tg2
        + (-2.0 * t3 + 3.0 * t2) * y3
        + (t3 - t2) * tg3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stabilization::KernelParams;

    const SAMPLE_XML: &str = r#"<!DOCTYPE lensdatabase SYSTEM "lensfun-database.dtd">
<lensdatabase version="2">
    <camera>
        <maker>Canon</maker>
        <model>EOS 5D Mark IV</model>
        <mount>Canon EF</mount>
        <cropfactor>1.0</cropfactor>
    </camera>
    <camera>
        <maker>Canon</maker>
        <model>EOS 80D</model>
        <mount>Canon EF-S</mount>
        <cropfactor>1.6</cropfactor>
    </camera>
    <mount>
        <name>Canon EF</name>
    </mount>
    <lens>
        <maker>Canon</maker>
        <model>Canon EF 35mm f/2</model>
        <mount>Canon EF</mount>
        <cropfactor>1.0</cropfactor>
        <calibration>
            <distortion model="ptlens" focal="35" a="0.007684346" b="-0.010468312" c="0.003928173"/>
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
    <lens>
        <maker>Sony</maker>
        <model>FE 50mm F1.8</model>
        <mount>Sony E</mount>
        <cropfactor>1.0</cropfactor>
        <aspect-ratio>16:9</aspect-ratio>
        <calibration>
            <distortion model="poly5" focal="50" real-focal="49.5" k1="0.001" k2="-0.0002"/>
        </calibration>
    </lens>
</lensdatabase>"#;

    #[test]
    fn test_parse_lens_metadata_and_models() {
        let lenses = parse_lenses(SAMPLE_XML);
        assert_eq!(lenses.len(), 3);

        // Lens 1: Canon EF 35mm (PTLens)
        assert_eq!(lenses[0].maker, "Canon");
        assert_eq!(lenses[0].model, "Canon EF 35mm f/2");
        assert_eq!(lenses[0].mounts, vec!["Canon EF".to_string()]);
        assert_eq!(lenses[0].crop_factor, 1.0);
        assert_eq!(lenses[0].aspect_ratio, DEFAULT_ASPECT_RATIO);
        assert_eq!(lenses[0].distortions.len(), 1);
        assert_eq!(lenses[0].distortions[0].model, "ptlens");
        assert_eq!(lenses[0].distortions[0].focal, 35.0);
        assert_eq!(
            lenses[0].distortions[0].k,
            vec![0.007684346, -0.010468312, 0.003928173]
        );

        // Lens 2: Sigma 18-35mm (Poly3)
        assert_eq!(lenses[1].maker, "Sigma");
        assert_eq!(lenses[1].model, "Sigma 18-35mm f/1.8 DC HSM");
        assert_eq!(lenses[1].crop_factor, 1.6);
        assert_eq!(lenses[1].aspect_ratio, 1.5);
        assert_eq!(lenses[1].distortions.len(), 2);
        assert_eq!(lenses[1].distortions[0].focal, 18.0);
        assert_eq!(lenses[1].distortions[0].k, vec![-0.02127]);
        assert_eq!(lenses[1].distortions[1].focal, 35.0);
        assert_eq!(lenses[1].distortions[1].k, vec![0.00366]);

        // Lens 3: Sony 50mm (Poly5)
        assert_eq!(lenses[2].maker, "Sony");
        assert_eq!(lenses[2].model, "FE 50mm F1.8");
        assert_eq!(lenses[2].aspect_ratio, 16.0 / 9.0);
        assert_eq!(lenses[2].distortions[0].model, "poly5");
        assert_eq!(lenses[2].distortions[0].real_focal, Some(49.5));
        assert_eq!(lenses[2].distortions[0].k, vec![0.001, -0.0002]);
    }

    #[test]
    fn test_fallback_crop_factor_from_camera() {
        let xml = r#"<lensdatabase version="2">
            <camera>
                <maker>Nikon</maker>
                <model>D500</model>
                <mount>Nikon F</mount>
                <cropfactor>1.5</cropfactor>
            </camera>
            <lens>
                <maker>Nikon</maker>
                <model>AF-S DX 35mm f/1.8G</model>
                <mount>Nikon F</mount>
                <calibration>
                    <distortion model="poly3" focal="35" k1="-0.01"/>
                </calibration>
            </lens>
        </lensdatabase>"#;
        let lenses = parse_lenses(xml);
        assert_eq!(lenses.len(), 1);
        assert_eq!(lenses[0].crop_factor, 1.5);
    }

    #[test]
    fn test_ignores_non_distortion_calibration() {
        let xml = r#"<lensdatabase version="2">
            <lens>
                <maker>Brand</maker>
                <model>No Distortion Lens</model>
                <mount>Generic</mount>
                <cropfactor>1.0</cropfactor>
                <calibration>
                    <tca model="poly3" focal="50" br="0.001" vr="1.0" bb="0.001" vb="1.0"/>
                    <vignetting model="pa" focal="50" aperture="2.8" distance="10" k1="-0.5" k2="0.2" k3="-0.1"/>
                </calibration>
            </lens>
        </lensdatabase>"#;
        assert!(parse_lenses(xml).is_empty());
    }

    #[test]
    fn test_hugin_scaling_math() {
        let half_height_mm = FULL_FRAME_DIAGONAL_MM / 1.5f64.hypot(1.0) / 2.0;
        let scaling = hugin_scaling(
            "poly3",
            &[0.0],
            half_height_mm,
            Some(half_height_mm),
            1.0,
            1.5,
        );
        assert!((scaling - 1.0).abs() < 1e-9);

        let a = hugin_scaling("poly3", &[0.0], 50.0, Some(50.0), 1.0, 1.5);
        let b = hugin_scaling("poly3", &[0.0], 50.0, Some(50.0), 2.0, 1.5);
        assert!((b - a * 2.0).abs() < 1e-9);

        // real_focal fallback derivation
        let with_k = hugin_scaling("poly3", &[0.1], 50.0, None, 1.0, 1.5);
        let explicit = hugin_scaling("poly3", &[0.1], 45.0, Some(45.0), 1.0, 1.5);
        assert!((with_k - explicit).abs() < 1e-9);

        let with_pt = hugin_scaling("ptlens", &[0.01, -0.02, 0.03], 35.0, None, 1.0, 1.5);
        let rf = 35.0 * (1.0 - 0.01 - (-0.02) - 0.03);
        let explicit_pt = hugin_scaling("ptlens", &[0.01, -0.02, 0.03], rf, Some(rf), 1.0, 1.5);
        assert!((with_pt - explicit_pt).abs() < 1e-9);
    }

    #[test]
    fn test_rescale_coeffs_formulas() {
        // Poly3: k0' = k0 * s^2 / (1 - k0)^3
        let mut k = vec![0.05];
        rescale_coeffs("poly3", &mut k, 2.0);
        let d = 1.0 - 0.05;
        let expected = 0.05 * 4.0 / (d * d * d);
        assert!((k[0] - expected).abs() < 1e-12);

        // Poly5: k0' = k0 * s^2, k1' = k1 * s^4
        let mut k5 = vec![0.01, -0.002];
        rescale_coeffs("poly5", &mut k5, 1.5);
        assert!((k5[0] - 0.01 * 1.5f64.powi(2)).abs() < 1e-12);
        assert!((k5[1] - -0.002 * 1.5f64.powi(4)).abs() < 1e-12);

        // PTLens: d = 1 - a - b - c; a' = a*s^3/d^4, b' = b*s^2/d^3, c' = c*s/d^2
        let mut k_pt = vec![0.01, -0.02, 0.005];
        let d_pt: f64 = 1.0 - 0.01 - (-0.02) - 0.005;
        rescale_coeffs("ptlens", &mut k_pt, 1.8);
        assert!((k_pt[0] - 0.01 * 1.8f64.powi(3) / d_pt.powi(4)).abs() < 1e-12);
        assert!((k_pt[1] - -0.02 * 1.8f64.powi(2) / d_pt.powi(3)).abs() < 1e-12);
        assert!((k_pt[2] - 0.005 * 1.8 / d_pt.powi(2)).abs() < 1e-12);
    }

    #[test]
    fn test_distortion_models_distort_undistort_roundtrip() {
        use crate::stabilization::distortion_models::{poly3::Poly3, poly5::Poly5, ptlens::PtLens};

        // Test Poly3
        let poly3 = Poly3::default();
        let mut kp = KernelParams::default();
        kp.k[0] = -0.015;
        let test_points = [(0.0f32, 0.0f32), (0.1, 0.1), (-0.2, 0.15), (0.3, -0.25)];
        for (x, y) in test_points {
            let (dx, dy) = poly3.distort_point(x, y, 1.0, &kp);
            let undist = poly3
                .undistort_point((dx, dy), &kp)
                .expect("poly3 undistort");
            let mag = (x * x + y * y).sqrt();
            let umag = (undist.0 * undist.0 + undist.1 * undist.1).sqrt();
            if mag > 1e-6 {
                let reconstructed_x = (undist.0 / umag) * umag.tan();
                let reconstructed_y = (undist.1 / umag) * umag.tan();
                assert!(
                    (reconstructed_x - x).abs() < 1e-4,
                    "poly3 x mismatch: {reconstructed_x} vs {x}"
                );
                assert!(
                    (reconstructed_y - y).abs() < 1e-4,
                    "poly3 y mismatch: {reconstructed_y} vs {y}"
                );
            }
        }

        // Test Poly5
        let poly5 = Poly5::default();
        let mut kp5 = KernelParams::default();
        kp5.k[0] = 0.01;
        kp5.k[1] = -0.002;
        for (x, y) in test_points {
            let (dx, dy) = poly5.distort_point(x, y, 1.0, &kp5);
            let undist = poly5
                .undistort_point((dx, dy), &kp5)
                .expect("poly5 undistort");
            let mag = (x * x + y * y).sqrt();
            let umag = (undist.0 * undist.0 + undist.1 * undist.1).sqrt();
            if mag > 1e-6 {
                let reconstructed_x = (undist.0 / umag) * umag.tan();
                let reconstructed_y = (undist.1 / umag) * umag.tan();
                assert!(
                    (reconstructed_x - x).abs() < 1e-4,
                    "poly5 x mismatch: {reconstructed_x} vs {x}"
                );
                assert!(
                    (reconstructed_y - y).abs() < 1e-4,
                    "poly5 y mismatch: {reconstructed_y} vs {y}"
                );
            }
        }

        // Test PtLens
        let ptlens = PtLens::default();
        let mut kpt = KernelParams::default();
        kpt.k[0] = 0.005;
        kpt.k[1] = -0.01;
        kpt.k[2] = 0.002;
        for (x, y) in test_points {
            let (dx, dy) = ptlens.distort_point(x, y, 1.0, &kpt);
            let undist = ptlens
                .undistort_point((dx, dy), &kpt)
                .expect("ptlens undistort");
            let mag = (x * x + y * y).sqrt();
            let umag = (undist.0 * undist.0 + undist.1 * undist.1).sqrt();
            if mag > 1e-6 {
                let reconstructed_x = (undist.0 / umag) * umag.tan();
                let reconstructed_y = (undist.1 / umag) * umag.tan();
                assert!(
                    (reconstructed_x - x).abs() < 1e-4,
                    "ptlens x mismatch: {reconstructed_x} vs {x}"
                );
                assert!(
                    (reconstructed_y - y).abs() < 1e-4,
                    "ptlens y mismatch: {reconstructed_y} vs {y}"
                );
            }
        }
    }

    #[test]
    fn test_distortion_to_profile_produces_usable_profile() {
        let lenses = parse_lenses(SAMPLE_XML);
        let profile = distortion_to_profile(&lenses[0], &lenses[0].distortions[0], "canon.xml")
            .expect("profile");

        assert_eq!(profile.camera_brand, "Canon");
        assert_eq!(profile.camera_model, "EF 35mm f/2");
        assert_eq!(profile.distortion_model.as_deref(), Some("ptlens"));
        assert_eq!(profile.fisheye_params.distortion_coeffs.len(), 3);
        assert_eq!(profile.calib_dimension.h, REFERENCE_HEIGHT_PX as usize);
        assert_eq!(
            profile.calib_dimension.w,
            (REFERENCE_HEIGHT_PX * 1.5).round() as usize
        );
        assert!(profile.official);
        assert_eq!(profile.calibrated_by, "Lensfun");
        assert!(profile.checksum.is_some());

        let fx = profile.fisheye_params.camera_matrix[0][0];
        let fy = profile.fisheye_params.camera_matrix[1][1];
        assert_eq!(fx, fy);
        assert!(fx > 0.0 && fx.is_finite());
        assert_eq!(
            profile.fisheye_params.camera_matrix[0][2],
            profile.calib_dimension.w as f64 / 2.0
        );
        assert_eq!(
            profile.fisheye_params.camera_matrix[1][2],
            profile.calib_dimension.h as f64 / 2.0
        );
    }

    #[test]
    fn test_multi_focal_populates_interpolations() {
        let lenses = parse_lenses(SAMPLE_XML);
        let profile = distortion_to_profile(&lenses[1], &lenses[1].distortions[0], "sigma.xml")
            .expect("profile");
        assert!(profile.interpolations.is_some());
        let interp = profile
            .interpolations
            .as_ref()
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(interp.len(), 2);
        assert!(interp.contains_key("18.0000"));
        assert!(interp.contains_key("35.0000"));
    }

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
    fn test_interpolate_distortion_exact_match() {
        let lens = &parse_lenses(ZOOM_XML)[0];
        let d = interpolate_distortion(lens, 1.5, 24.0).unwrap();
        assert_eq!(d.k, vec![0.008, -0.021, 0.001]);
    }

    #[test]
    fn test_interpolate_distortion_matches_lensfun_hermite_spline() {
        let lens = &parse_lenses(ZOOM_XML)[0];
        // 28mm falls between 24mm and 35mm
        let d = interpolate_distortion(lens, 1.5, 28.0).unwrap();
        assert!((d.k[0] - 0.0063050338).abs() < 1e-7, "a = {}", d.k[0]);
        assert!((d.k[1] - -0.015735162).abs() < 1e-7, "b = {}", d.k[1]);
        assert!((d.k[2] - 0.0007747934).abs() < 1e-7, "c = {}", d.k[2]);
        assert!(
            (d.real_focal.unwrap() - 27.4955424).abs() < 1e-5,
            "real_focal = {}",
            d.real_focal.unwrap()
        );
    }

    #[test]
    fn test_interpolate_distortion_clamps_outside_range() {
        let lens = &parse_lenses(ZOOM_XML)[0];
        let d = interpolate_distortion(lens, 1.5, 100.0).unwrap();
        assert_eq!(d.k, vec![0.001, -0.002, 0.0]); // clamped to 55mm
        let d_min = interpolate_distortion(lens, 1.5, 10.0).unwrap();
        assert_eq!(d_min.k, vec![0.012, -0.035, 0.002]); // clamped to 18mm
    }

    #[test]
    fn test_interpolate_distortion_crop_gate() {
        let lens = &parse_lenses(ZOOM_XML)[0]; // cropfactor 1.5
                                               // Full frame target sensor (cropfactor 1.0) cannot use 1.5 crop calibration
        assert!(interpolate_distortion(lens, 1.0, 24.0).is_none());
        // MFT sensor (cropfactor 2.0) can use 1.5 crop calibration
        assert!(interpolate_distortion(lens, 2.0, 24.0).is_some());
    }

    #[test]
    fn test_parse_xml_end_to_end() {
        let profiles = parse_xml(SAMPLE_XML, "sample.xml");
        // 1 from Canon + 2 from Sigma + 1 from Sony = 4 profiles
        assert_eq!(profiles.len(), 4);
        for p in profiles {
            assert!(!p.camera_brand.is_empty());
            assert!(!p.camera_model.is_empty());
            assert!(!p.name.is_empty());
            assert!(p.checksum.is_some());
        }
    }
}
