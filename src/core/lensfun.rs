// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024-2026 Adrian <adrian.eddy at gmail>

use crate::lens_profile::{ CameraParams, Dimensions, LensProfile };
use crate::stabilization::distortion_models::{ poly3::Poly3, poly5::Poly5, ptlens::PtLens };
use serde::{ Deserialize, Serialize };

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LensFunMount {
    pub name: String,
    pub compat: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LensFunCamera {
    pub maker: String,
    pub model: String,
    pub variant: Option<String>,
    pub mount: String,
    pub crop_factor: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LensFunDistortion {
    pub focal: f64,
    pub real_focal: Option<f64>,
    pub model: String,
    pub coeffs: Vec<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LensFunLens {
    pub maker: String,
    pub model: String,
    pub mounts: Vec<String>,
    pub crop_factor: f64,
    pub aspect_ratio: f64,
    pub lens_type: String,
    pub min_focal: Option<f64>,
    pub max_focal: Option<f64>,
    pub distortions: Vec<LensFunDistortion>,
}

impl LensFunLens {
    pub fn is_zoom(&self) -> bool {
        match (self.min_focal, self.max_focal) {
            (Some(min_f), Some(max_f)) if (max_f - min_f).abs() > 1e-3 => true,
            _ => self.distortions.len() > 1,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LensFunDatabase {
    pub mounts: Vec<LensFunMount>,
    pub cameras: Vec<LensFunCamera>,
    pub lenses: Vec<LensFunLens>,
}

fn strip_doctype(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(pos) = rest.find("<!DOCTYPE") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        if let Some(end) = after.find('>') {
            rest = &after[end + 1..];
        } else {
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out
}

fn parse_aspect_ratio(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some((w, h)) = s.split_once(':') {
        let w: f64 = w.trim().parse().ok()?;
        let h: f64 = h.trim().parse().ok()?;
        if w > 0.0 && h > 0.0 {
            return Some(w / h);
        }
    }
    s.parse::<f64>().ok().filter(|v| *v > 0.0)
}

fn child_text<'a>(node: roxmltree::Node<'a, 'a>, tag: &str) -> Option<String> {
    let mut fallback = None;
    for child in node.children().filter(|c| c.is_element() && c.tag_name().name() == tag) {
        let lang = child.attribute("lang").or_else(|| child.attribute(("http://www.w3.org/XML/1998/namespace", "lang")));
        if lang.is_none() || lang == Some("en") {
            if let Some(t) = child.text() {
                return Some(t.trim().to_string());
            }
        } else if fallback.is_none() {
            fallback = child.text().map(|t| t.trim().to_string());
        }
    }
    fallback
}

/// Convert a single LensFun rectilinear distortion calibration to Gyroflow pixel space.
///
/// Mirrors the exact math used in `src/core/calibration/mod.rs`:
/// - `hugin_scale_in_millimeters = hypot(36.0, 24.0) / crop_factor / hypot(aspect_ratio, 1.0) / 2.0`
/// - `focal_px = real_focal / (2.0 * hugin_scale_in_millimeters) * calib_h`
/// - `hugin_scaling = real_focal / hugin_scale_in_millimeters / focal_px`
pub fn convert_lensfun_distortion(
    dist: &LensFunDistortion,
    crop_factor: f64,
    aspect_ratio: f64,
    calib_w: usize,
    calib_h: usize,
) -> Option<(String, [[f64; 3]; 3], Vec<f64>, f64)> {
    if crop_factor <= 0.0 || aspect_ratio <= 0.0 || calib_w == 0 || calib_h == 0 || dist.focal <= 0.0 {
        return None;
    }

    let (model_id, mut coeffs, fallback_real_focal) = match dist.model.as_str() {
        "ptlens" => {
            let a = *dist.coeffs.first().unwrap_or(&0.0);
            let b = *dist.coeffs.get(1).unwrap_or(&0.0);
            let c = *dist.coeffs.get(2).unwrap_or(&0.0);
            let rf = dist.focal * (1.0 - a - b - c);
            ("ptlens".to_string(), vec![a, b, c], rf)
        }
        "poly3" => {
            let k1 = *dist.coeffs.first().unwrap_or(&0.0);
            let rf = dist.focal * (1.0 - k1);
            ("poly3".to_string(), vec![k1], rf)
        }
        "poly5" => {
            let k1 = *dist.coeffs.first().unwrap_or(&0.0);
            let k2 = *dist.coeffs.get(1).unwrap_or(&0.0);
            let rf = dist.focal;
            ("poly5".to_string(), vec![k1, k2], rf)
        }
        _ => return None,
    };

    let real_focal = dist.real_focal.filter(|v| *v > 0.0).unwrap_or(fallback_real_focal);
    if real_focal <= 0.0 {
        return None;
    }

    let hugin_scale_in_millimeters = 36.0_f64.hypot(24.0) / crop_factor / aspect_ratio.hypot(1.0) / 2.0;
    let focal_px = real_focal / (2.0 * hugin_scale_in_millimeters) * (calib_h as f64);
    let hugin_scaling = real_focal / hugin_scale_in_millimeters;

    match model_id.as_str() {
        "ptlens" => PtLens::rescale_coeffs(&mut coeffs, hugin_scaling),
        "poly3" => Poly3::rescale_coeffs(&mut coeffs, hugin_scaling),
        "poly5" => Poly5::rescale_coeffs(&mut coeffs, hugin_scaling),
        _ => unreachable!(),
    }

    let cx = calib_w as f64 / 2.0;
    let cy = calib_h as f64 / 2.0;
    let camera_matrix = [
        [focal_px, 0.0, cx],
        [0.0, focal_px, cy],
        [0.0, 0.0, 1.0],
    ];

    Some((model_id, camera_matrix, coeffs, real_focal))
}

impl LensFunDatabase {
    pub fn parse_xml(xml: &str) -> Result<Self, String> {
        let cleaned = strip_doctype(xml);
        let doc = roxmltree::Document::parse(&cleaned).map_err(|e| e.to_string())?;
        let mut db = Self::default();

        for node in doc.descendants().filter(|n| n.is_element()) {
            match node.tag_name().name() {
                "mount" => {
                    if node.parent_element().map(|p| p.tag_name().name()) != Some("lensdatabase") {
                        continue;
                    }
                    if let Some(name) = child_text(node, "name") {
                        let compat = node
                            .children()
                            .filter(|c| c.is_element() && c.tag_name().name() == "compat")
                            .filter_map(|c| c.text().map(|t| t.trim().to_string()))
                            .filter(|s| !s.is_empty())
                            .collect();
                        db.mounts.push(LensFunMount { name, compat });
                    }
                }
                "camera" => {
                    let maker = child_text(node, "maker").unwrap_or_default();
                    let model = child_text(node, "model").unwrap_or_default();
                    let variant = child_text(node, "variant");
                    let mount = child_text(node, "mount").unwrap_or_default();
                    let crop_factor = child_text(node, "cropfactor")
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(1.0);
                    if !maker.is_empty() && !model.is_empty() {
                        db.cameras.push(LensFunCamera {
                            maker,
                            model,
                            variant,
                            mount,
                            crop_factor,
                        });
                    }
                }
                "lens" => {
                    let maker = child_text(node, "maker").unwrap_or_default();
                    let model = child_text(node, "model").unwrap_or_default();
                    if model.is_empty() {
                        continue;
                    }
                    let mounts: Vec<String> = node
                        .children()
                        .filter(|c| c.is_element() && c.tag_name().name() == "mount")
                        .filter_map(|c| c.text().map(|t| t.trim().to_string()))
                        .filter(|s| !s.is_empty())
                        .collect();
                    let crop_factor = child_text(node, "cropfactor")
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(1.0);
                    let aspect_ratio = child_text(node, "aspect-ratio")
                        .and_then(|s| parse_aspect_ratio(&s))
                        .unwrap_or(3.0 / 2.0);
                    let lens_type = child_text(node, "type").unwrap_or_else(|| "rectilinear".to_string());

                    let mut min_focal = None;
                    let mut max_focal = None;
                    if let Some(focal_node) = node.children().find(|c| c.is_element() && c.tag_name().name() == "focal") {
                        min_focal = focal_node.attribute("min").and_then(|v| v.parse::<f64>().ok());
                        max_focal = focal_node.attribute("max").and_then(|v| v.parse::<f64>().ok());
                        if min_focal.is_none() {
                            if let Some(val) = focal_node.attribute("value").and_then(|v| v.parse::<f64>().ok()) {
                                min_focal = Some(val);
                                max_focal = Some(val);
                            }
                        }
                    }

                    let mut distortions = Vec::new();
                    let mut real_focal_by_focal = std::collections::BTreeMap::<i64, f64>::new();
                    if let Some(calib_node) = node.children().find(|c| c.is_element() && c.tag_name().name() == "calibration") {
                        for rf_node in calib_node.children().filter(|c| c.is_element() && c.tag_name().name() == "real-focal-length") {
                            if let (Some(f), Some(rf)) = (
                                rf_node.attribute("focal").and_then(|v| v.parse::<f64>().ok()),
                                rf_node.attribute("real-focal").and_then(|v| v.parse::<f64>().ok()),
                            ) {
                                real_focal_by_focal.insert((f * 1000.0).round() as i64, rf);
                            }
                        }
                        for dist_node in calib_node.children().filter(|c| c.is_element() && c.tag_name().name() == "distortion") {
                            let model_attr = dist_node.attribute("model").unwrap_or("poly3").trim().to_string();
                            let focal = match dist_node.attribute("focal").and_then(|v| v.parse::<f64>().ok()) {
                                Some(f) if f > 0.0 => f,
                                _ => continue,
                            };
                            let real_focal = dist_node
                                .attribute("real-focal")
                                .and_then(|v| v.parse::<f64>().ok())
                                .or_else(|| real_focal_by_focal.get(&((focal * 1000.0).round() as i64)).copied());
                            let coeffs = match model_attr.as_str() {
                                "ptlens" => vec![
                                    dist_node.attribute("a").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0),
                                    dist_node.attribute("b").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0),
                                    dist_node.attribute("c").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0),
                                ],
                                "poly3" => vec![
                                    dist_node.attribute("k1").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0),
                                ],
                                "poly5" => vec![
                                    dist_node.attribute("k1").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0),
                                    dist_node.attribute("k2").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0),
                                ],
                                _ => continue,
                            };
                            distortions.push(LensFunDistortion {
                                focal,
                                real_focal,
                                model: model_attr,
                                coeffs,
                            });
                        }
                    }

                    distortions.sort_by(|a, b| a.focal.partial_cmp(&b.focal).unwrap_or(std::cmp::Ordering::Equal));
                    if min_focal.is_none() && !distortions.is_empty() {
                        min_focal = distortions.first().map(|d| d.focal);
                        max_focal = distortions.last().map(|d| d.focal);
                    }

                    db.lenses.push(LensFunLens {
                        maker,
                        model,
                        mounts,
                        crop_factor,
                        aspect_ratio,
                        lens_type,
                        min_focal,
                        max_focal,
                        distortions,
                    });
                }
                _ => {}
            }
        }

        Ok(db)
    }

    /// Convert all rectilinear lenses with supported distortion calibrations (`ptlens`, `poly3`, `poly5`)
    /// into Gyroflow `LensProfile`s.
    pub fn to_lens_profiles(&self, source_path: &str) -> Vec<LensProfile> {
        let mut profiles = Vec::new();

        for lens in &self.lenses {
            if !lens.lens_type.eq_ignore_ascii_case("rectilinear") || lens.distortions.is_empty() {
                continue;
            }

            let calib_w: usize = 3840;
            let mut calib_h: usize = ((calib_w as f64) / lens.aspect_ratio.max(0.1)).round() as usize;
            if calib_h % 2 != 0 {
                calib_h += 1;
            }

            let mut converted = Vec::new();
            for dist in &lens.distortions {
                if let Some((model_id, matrix, coeffs, _real_focal)) =
                    convert_lensfun_distortion(dist, lens.crop_factor, lens.aspect_ratio, calib_w, calib_h)
                {
                    converted.push((dist.focal, model_id, matrix, coeffs));
                }
            }
            if converted.is_empty() {
                continue;
            }

            let brand = if !lens.maker.is_empty() {
                lens.maker.clone()
            } else {
                lens.model.split_whitespace().next().unwrap_or("LensFun").to_string()
            };

            let clean_lens_model = if !lens.maker.is_empty()
                && lens.model.to_ascii_lowercase().starts_with(&lens.maker.to_ascii_lowercase())
            {
                lens.model[lens.maker.len()..].trim().to_string()
            } else {
                lens.model.clone()
            };
            let lens_label = if clean_lens_model.is_empty() {
                lens.model.clone()
            } else {
                clean_lens_model
            };

            let slug_brand: String = brand.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
            let slug_lens: String = lens_label.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();

            // Create individual focal length profiles AND a combined interpolated profile for zoom lenses
            if converted.len() == 1 {
                let (focal, model_id, matrix, coeffs) = &converted[0];
                let id = format!("lensfun:{}:{}:{:.1}mm", slug_brand, slug_lens, focal);
                let mut profile = LensProfile {
                    name: format!("{} {} ({:.0}mm)", brand, lens_label, focal),
                    camera_brand: brand.clone(),
                    camera_model: "LensFun".to_string(),
                    lens_model: format!("{} {:.0}mm", lens_label, focal),
                    camera_setting: format!("crop {:.2}x", lens.crop_factor),
                    note: format!("LensFun ({})", lens.mounts.join(", ")),
                    calibrated_by: "LensFun".to_string(),
                    calibrator_version: "lensfun".to_string(),
                    official: true,
                    calib_dimension: Dimensions { w: calib_w, h: calib_h },
                    orig_dimension: Dimensions { w: calib_w, h: calib_h },
                    input_horizontal_stretch: 1.0,
                    input_vertical_stretch: 1.0,
                    focal_length: Some(*focal),
                    crop_factor: Some(lens.crop_factor),
                    distortion_model: Some(model_id.clone()),
                    fisheye_params: CameraParams {
                        RMS_error: 0.0,
                        camera_matrix: matrix.to_vec(),
                        distortion_coeffs: coeffs.clone(),
                        radial_distortion_limit: None,
                    },
                    identifier: id.clone(),
                    path_to_file: format!("{}#{}", source_path, id),
                    ..Default::default()
                };
                profile.init();
                profiles.push(profile);
            } else {
                // Multi-focal zoom lens: create a master profile with interpolations if all share the same distortion model
                let first_model = converted[0].1.clone();
                if converted.iter().all(|(_, m, _, _)| *m == first_model) {
                    let mut interp_map = serde_json::Map::new();
                    for (focal, _, matrix, coeffs) in &converted {
                        let mut entry = serde_json::Map::new();
                        entry.insert("focal_length".to_string(), serde_json::json!(focal));
                        entry.insert("camera_matrix".to_string(), serde_json::json!(matrix));
                        entry.insert("distortion_coeffs".to_string(), serde_json::json!(coeffs));
                        interp_map.insert(format!("{:.2}", focal), serde_json::Value::Object(entry));
                    }
                    let (focal0, _, matrix0, coeffs0) = &converted[0];
                    let id = format!("lensfun:{}:{}:zoom", slug_brand, slug_lens);
                    let mut zoom_profile = LensProfile {
                        name: format!("{} {}", brand, lens_label),
                        camera_brand: brand.clone(),
                        camera_model: "LensFun".to_string(),
                        lens_model: lens_label.clone(),
                        camera_setting: format!("crop {:.2}x", lens.crop_factor),
                        note: format!("LensFun ({})", lens.mounts.join(", ")),
                        calibrated_by: "LensFun".to_string(),
                        calibrator_version: "lensfun".to_string(),
                        official: true,
                        calib_dimension: Dimensions { w: calib_w, h: calib_h },
                        orig_dimension: Dimensions { w: calib_w, h: calib_h },
                        input_horizontal_stretch: 1.0,
                        input_vertical_stretch: 1.0,
                        focal_length: Some(*focal0),
                        crop_factor: Some(lens.crop_factor),
                        distortion_model: Some(first_model),
                        interpolations: Some(serde_json::Value::Object(interp_map)),
                        fisheye_params: CameraParams {
                            RMS_error: 0.0,
                            camera_matrix: matrix0.to_vec(),
                            distortion_coeffs: coeffs0.clone(),
                            radial_distortion_limit: None,
                        },
                        identifier: id.clone(),
                        path_to_file: format!("{}#{}", source_path, id),
                        ..Default::default()
                    };
                    zoom_profile.init();
                    profiles.push(zoom_profile);
                }

                // Also emit per-focal-length profiles so users without dynamic focal length metadata can pick a specific focal length
                for (focal, model_id, matrix, coeffs) in &converted {
                    let id = format!("lensfun:{}:{}:{:.1}mm", slug_brand, slug_lens, focal);
                    let mut profile = LensProfile {
                        name: format!("{} {} ({:.0}mm)", brand, lens_label, focal),
                        camera_brand: brand.clone(),
                        camera_model: "LensFun".to_string(),
                        lens_model: format!("{} ({:.0}mm)", lens_label, focal),
                        camera_setting: format!("crop {:.2}x", lens.crop_factor),
                        note: format!("LensFun ({})", lens.mounts.join(", ")),
                        calibrated_by: "LensFun".to_string(),
                        calibrator_version: "lensfun".to_string(),
                        official: true,
                        calib_dimension: Dimensions { w: calib_w, h: calib_h },
                        orig_dimension: Dimensions { w: calib_w, h: calib_h },
                        input_horizontal_stretch: 1.0,
                        input_vertical_stretch: 1.0,
                        focal_length: Some(*focal),
                        crop_factor: Some(lens.crop_factor),
                        distortion_model: Some(model_id.clone()),
                        fisheye_params: CameraParams {
                            RMS_error: 0.0,
                            camera_matrix: matrix.to_vec(),
                            distortion_coeffs: coeffs.clone(),
                            radial_distortion_limit: None,
                        },
                        identifier: id.clone(),
                        path_to_file: format!("{}#{}", source_path, id),
                        ..Default::default()
                    };
                    profile.init();
                    profiles.push(profile);
                }
            }
        }

        profiles
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stabilization::KernelParams;
    use crate::stabilization::distortion_models::DistortionModel;

    const SAMPLE_LENSFUN_XML: &str = r#"<!DOCTYPE lensdatabase SYSTEM "lensfun-database.dtd">
<lensdatabase version="2">
    <mount>
        <name>Sony E</name>
        <compat>Minolta AF</compat>
    </mount>
    <camera>
        <maker>Sony</maker>
        <model>ILCE-7M4</model>
        <mount>Sony E</mount>
        <cropfactor>1.0</cropfactor>
    </camera>
    <lens>
        <maker>Sony</maker>
        <model>Sony FE 16-35mm F2.8 GM</model>
        <mount>Sony E</mount>
        <cropfactor>1.0</cropfactor>
        <aspect-ratio>3:2</aspect-ratio>
        <calibration>
            <distortion model="ptlens" focal="16" a="0.0102" b="-0.0412" c="0.0155" />
            <distortion model="ptlens" focal="24" a="0.0041" b="-0.0123" c="0.0032" />
            <distortion model="ptlens" focal="35" a="0.0012" b="0.0031" c="-0.0045" />
        </calibration>
    </lens>
    <lens>
        <maker>Sigma</maker>
        <model>Sigma 30mm F1.4 DC DN</model>
        <mount>Sony E</mount>
        <cropfactor>1.534</cropfactor>
        <calibration>
            <distortion model="poly3" focal="30" k1="-0.0125" real-focal="29.4" />
        </calibration>
    </lens>
    <lens>
        <maker>Samurai</maker>
        <model>Samurai 12mm F2.0</model>
        <mount>Sony E</mount>
        <cropfactor>1.5</cropfactor>
        <calibration>
            <distortion model="poly5" focal="12" k1="-0.018" k2="0.004" />
        </calibration>
    </lens>
</lensdatabase>
"#;

    #[test]
    fn test_parse_lensfun_xml_and_convert_profiles() {
        let db = LensFunDatabase::parse_xml(SAMPLE_LENSFUN_XML).expect("Failed to parse LensFun XML");
        assert_eq!(db.mounts.len(), 1);
        assert_eq!(db.mounts[0].name, "Sony E");
        assert_eq!(db.mounts[0].compat, vec!["Minolta AF"]);

        assert_eq!(db.cameras.len(), 1);
        assert_eq!(db.cameras[0].maker, "Sony");
        assert_eq!(db.cameras[0].model, "ILCE-7M4");
        assert_eq!(db.cameras[0].mount, "Sony E");
        assert!((db.cameras[0].crop_factor - 1.0).abs() < 1e-6);

        assert_eq!(db.lenses.len(), 3);
        assert!(db.lenses[0].is_zoom());
        assert!(!db.lenses[1].is_zoom());

        let profiles = db.to_lens_profiles("slr-sony.xml");
        // 1 zoom master + 3 focal lengths + 1 Sigma poly3 + 1 Samurai poly5 = 6 profiles
        assert_eq!(profiles.len(), 6);

        // Verify roundtrip distort -> undistort on converted PtLens, Poly3, Poly5 profiles
        for prof in &profiles {
            let model = DistortionModel::from_name(prof.distortion_model.as_deref().unwrap());
            let mut params = KernelParams::default();
            params.width = prof.calib_dimension.w as i32;
            params.height = prof.calib_dimension.h as i32;
            params.f = [
                prof.fisheye_params.camera_matrix[0][0] as f32,
                prof.fisheye_params.camera_matrix[1][1] as f32,
            ];
            params.c = [
                prof.fisheye_params.camera_matrix[0][2] as f32,
                prof.fisheye_params.camera_matrix[1][2] as f32,
            ];
            for (i, v) in prof.fisheye_params.distortion_coeffs.iter().enumerate() {
                params.k[i] = *v as f32;
            }

            // Test a ray in the z=1 plane: distort_point maps (x, y, 1) -> distorted z=1 coords,
            // and undistort_point maps distorted z=1 coords -> angle vector (u * atan(ru))
            let x_z1 = 0.15_f32;
            let y_z1 = -0.10_f32;
            let distorted = model.distort_point(x_z1, y_z1, 1.0, &params);
            let undist_angle = model.undistort_point(distorted, &params).expect("undistort failed");
            let theta = (undist_angle.0 * undist_angle.0 + undist_angle.1 * undist_angle.1).sqrt();
            let scale = theta.tan() / theta;
            let recovered_x = undist_angle.0 * scale;
            let recovered_y = undist_angle.1 * scale;
            assert!((recovered_x - x_z1).abs() < 1e-4, "x mismatch for {}: {} vs {}", prof.identifier, recovered_x, x_z1);
            assert!((recovered_y - y_z1).abs() < 1e-4, "y mismatch for {}: {} vs {}", prof.identifier, recovered_y, y_z1);
        }
    }
}
