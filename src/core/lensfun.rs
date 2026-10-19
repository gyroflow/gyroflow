// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022-2026 Adrian <adrian.eddy at gmail> and Gyroflow Contributors

use std::collections::HashMap;
use crate::lens_profile::{ Dimensions, CameraParams, LensProfile };
use crate::stabilization::distortion_models::{ poly3::Poly3, poly5::Poly5, ptlens::PtLens };

#[derive(Debug, Clone, Default)]
pub struct LensfunDistortion {
    pub model: String,
    pub focal: f64,
    pub real_focal: Option<f64>,
    pub k1: Option<f64>,
    pub k2: Option<f64>,
    pub a: Option<f64>,
    pub b: Option<f64>,
    pub c: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct LensfunTca {
    pub model: String,
    pub focal: f64,
    pub vr: f64,
    pub vb: f64,
}

#[derive(Debug, Clone, Default)]
pub struct LensfunVignetting {
    pub model: String,
    pub focal: f64,
    pub aperture: f64,
    pub distance: f64,
    pub k1: f64,
    pub k2: f64,
    pub k3: f64,
}

#[derive(Debug, Clone, Default)]
pub struct LensfunCalibration {
    pub distortions: Vec<LensfunDistortion>,
    pub tca: Vec<LensfunTca>,
    pub vignetting: Vec<LensfunVignetting>,
}

#[derive(Debug, Clone, Default)]
pub struct LensfunLens {
    pub maker: String,
    pub model: String,
    pub mounts: Vec<String>,
    pub crop_factor: f64,
    pub aspect_ratio: Option<String>,
    pub calibrations: Vec<LensfunCalibration>,
}

#[derive(Debug, Clone, Default)]
pub struct LensfunCamera {
    pub maker: String,
    pub model: String,
    pub mount: String,
    pub crop_factor: f64,
}

#[derive(Debug, Clone, Default)]
pub struct LensfunDatabase {
    pub lenses: Vec<LensfunLens>,
    pub cameras: Vec<LensfunCamera>,
}

/// Helper to parse XML attributes and tags in Lensfun database files
pub fn parse_lensfun_xml(xml_content: &str) -> Result<Vec<LensProfile>, String> {
    let mut db = LensfunDatabase::default();

    let mut current_lens: Option<LensfunLens> = None;
    let mut current_camera: Option<LensfunCamera> = None;
    let mut current_calibration: Option<LensfunCalibration> = None;
    let mut current_tag_path: Vec<String> = Vec::new();
    let mut text_buf = String::new();

    let chars: Vec<char> = xml_content.chars().collect();
    let mut i = 0;
    let len = chars.len();

    while i < len {
        if chars[i] == '<' {
            // Check for XML comment <!-- ... -->
            if i + 3 < len && chars[i + 1] == '!' && chars[i + 2] == '-' && chars[i + 3] == '-' {
                i += 4;
                while i + 2 < len && !(chars[i] == '-' && chars[i + 1] == '-' && chars[i + 2] == '>') {
                    i += 1;
                }
                i += 3;
                continue;
            }

            // End tag </tag>
            if i + 1 < len && chars[i + 1] == '/' {
                i += 2;
                let mut tag_name = String::new();
                while i < len && chars[i] != '>' {
                    if !chars[i].is_whitespace() {
                        tag_name.push(chars[i]);
                    }
                    i += 1;
                }
                i += 1; // skip '>'

                let text = text_buf.trim().to_string();
                text_buf.clear();

                match tag_name.as_str() {
                    "maker" => {
                        if let Some(ref mut l) = current_lens { l.maker = text; }
                        else if let Some(ref mut c) = current_camera { c.maker = text; }
                    }
                    "model" => {
                        if let Some(ref mut l) = current_lens { l.model = text; }
                        else if let Some(ref mut c) = current_camera { c.model = text; }
                    }
                    "mount" => {
                        if let Some(ref mut l) = current_lens { l.mounts.push(text); }
                        else if let Some(ref mut c) = current_camera { c.mount = text; }
                    }
                    "cropfactor" => {
                        if let Ok(cf) = text.parse::<f64>() {
                            if let Some(ref mut l) = current_lens { l.crop_factor = cf; }
                            else if let Some(ref mut c) = current_camera { c.crop_factor = cf; }
                        }
                    }
                    "aspect-ratio" => {
                        if let Some(ref mut l) = current_lens { l.aspect_ratio = Some(text); }
                    }
                    "calibration" => {
                        if let Some(cal) = current_calibration.take() {
                            if let Some(ref mut l) = current_lens {
                                l.calibrations.push(cal);
                            }
                        }
                    }
                    "lens" => {
                        if let Some(mut l) = current_lens.take() {
                            if l.crop_factor <= 0.0 { l.crop_factor = 1.0; }
                            db.lenses.push(l);
                        }
                    }
                    "camera" => {
                        if let Some(c) = current_camera.take() {
                            db.cameras.push(c);
                        }
                    }
                    _ => {}
                }

                current_tag_path.pop();
                continue;
            }

            // Start tag or self-closing tag
            i += 1; // skip '<'
            let mut tag_content = String::new();
            while i < len && chars[i] != '>' {
                tag_content.push(chars[i]);
                i += 1;
            }
            i += 1; // skip '>'

            let is_self_closing = tag_content.ends_with('/');
            let trimmed = if is_self_closing {
                tag_content[..tag_content.len() - 1].trim()
            } else {
                tag_content.trim()
            };

            // Parse tag name and attributes
            let (tag_name, attrs) = parse_tag_and_attrs(trimmed);

            match tag_name.as_str() {
                "lens" => {
                    let mut lens = LensfunLens::default();
                    lens.crop_factor = 1.0;
                    current_lens = Some(lens);
                }
                "camera" => {
                    let mut cam = LensfunCamera::default();
                    cam.crop_factor = 1.0;
                    current_camera = Some(cam);
                }
                "calibration" => {
                    current_calibration = Some(LensfunCalibration::default());
                }
                "distortion" => {
                    let mut dist = LensfunDistortion::default();
                    dist.model = attrs.get("model").cloned().unwrap_or_default();
                    dist.focal = attrs.get("focal").and_then(|x| x.parse().ok()).unwrap_or(0.0);
                    dist.real_focal = attrs.get("real-focal").and_then(|x| x.parse().ok());
                    dist.k1 = attrs.get("k1").and_then(|x| x.parse().ok());
                    dist.k2 = attrs.get("k2").and_then(|x| x.parse().ok());
                    dist.a = attrs.get("a").and_then(|x| x.parse().ok());
                    dist.b = attrs.get("b").and_then(|x| x.parse().ok());
                    dist.c = attrs.get("c").and_then(|x| x.parse().ok());

                    if let Some(ref mut cal) = current_calibration {
                        cal.distortions.push(dist);
                    }
                }
                "tca" => {
                    let mut t = LensfunTca::default();
                    t.model = attrs.get("model").cloned().unwrap_or_default();
                    t.focal = attrs.get("focal").and_then(|x| x.parse().ok()).unwrap_or(0.0);
                    t.vr = attrs.get("vr").and_then(|x| x.parse().ok()).unwrap_or(1.0);
                    t.vb = attrs.get("vb").and_then(|x| x.parse().ok()).unwrap_or(1.0);
                    if let Some(ref mut cal) = current_calibration {
                        cal.tca.push(t);
                    }
                }
                "vignetting" => {
                    let mut v = LensfunVignetting::default();
                    v.model = attrs.get("model").cloned().unwrap_or_default();
                    v.focal = attrs.get("focal").and_then(|x| x.parse().ok()).unwrap_or(0.0);
                    v.aperture = attrs.get("aperture").and_then(|x| x.parse().ok()).unwrap_or(0.0);
                    v.distance = attrs.get("distance").and_then(|x| x.parse().ok()).unwrap_or(1000.0);
                    v.k1 = attrs.get("k1").and_then(|x| x.parse().ok()).unwrap_or(0.0);
                    v.k2 = attrs.get("k2").and_then(|x| x.parse().ok()).unwrap_or(0.0);
                    v.k3 = attrs.get("k3").and_then(|x| x.parse().ok()).unwrap_or(0.0);
                    if let Some(ref mut cal) = current_calibration {
                        cal.vignetting.push(v);
                    }
                }
                _ => {}
            }

            if !is_self_closing {
                current_tag_path.push(tag_name);
                text_buf.clear();
            }
        } else {
            text_buf.push(chars[i]);
            i += 1;
        }
    }

    // Convert parsed lenses into Gyroflow LensProfiles
    let mut profiles = Vec::new();
    for lens in &db.lenses {
        for cal in &lens.calibrations {
            for dist in &cal.distortions {
                let model = dist.model.to_lowercase();
                if model == "poly3" || model == "poly5" || model == "ptlens" {
                    let profile = convert_distortion_to_profile(lens, dist, 3840, 2160);
                    profiles.push(profile);
                }
            }
        }
    }

    Ok(profiles)
}

fn parse_tag_and_attrs(input: &str) -> (String, HashMap<String, String>) {
    let mut parts = input.split_whitespace();
    let tag = parts.next().unwrap_or_default().to_string();
    let mut attrs = HashMap::new();

    let rest = input[tag.len()..].trim();
    let mut chars = rest.chars().peekable();
    while chars.peek().is_some() {
        // Skip whitespace
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() { chars.next(); } else { break; }
        }
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c.is_whitespace() { break; }
            key.push(c);
            chars.next();
        }
        if key.is_empty() { break; }

        while let Some(&c) = chars.peek() {
            if c == '=' || c.is_whitespace() { chars.next(); } else { break; }
        }

        let quote = if let Some(&c) = chars.peek() {
            if c == '"' || c == '\'' { chars.next(); Some(c) } else { None }
        } else { None };

        let mut val = String::new();
        if let Some(q) = quote {
            while let Some(c) = chars.next() {
                if c == q { break; }
                val.push(c);
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() { break; }
                val.push(c);
                chars.next();
            }
        }
        attrs.insert(key.to_lowercase(), val);
    }

    (tag, attrs)
}

/// Converts Lensfun distortion parameters with sensor crop factor and aspect ratio
/// rescaling into a native Gyroflow LensProfile.
pub fn convert_distortion_to_profile(
    lens: &LensfunLens,
    dist: &LensfunDistortion,
    width: usize,
    height: usize,
) -> LensProfile {
    let crop_factor = if lens.crop_factor > 0.0 { lens.crop_factor } else { 1.0 };
    let aspect_ratio = width as f64 / height as f64;
    let nominal_focal = if dist.focal > 0.0 { dist.focal } else { 24.0 };

    // Initial coefficients from Lensfun
    let mut k = match dist.model.as_str() {
        "poly3" => vec![dist.k1.unwrap_or(0.0)],
        "poly5" => vec![dist.k1.unwrap_or(0.0), dist.k2.unwrap_or(0.0)],
        "ptlens" => vec![dist.a.unwrap_or(0.0), dist.b.unwrap_or(0.0), dist.c.unwrap_or(0.0)],
        _ => vec![0.0],
    };

    // Calculate real focal length
    let real_focal = dist.real_focal.unwrap_or_else(|| match dist.model.as_str() {
        "ptlens" => nominal_focal * (1.0 - k.get(0).unwrap_or(&0.0) - k.get(1).unwrap_or(&0.0) - k.get(2).unwrap_or(&0.0)),
        "poly3"  => nominal_focal * (1.0 - k.get(0).unwrap_or(&0.0)),
        _ => nominal_focal,
    });

    // Sensor diagonal of 35mm full-frame is hypot(36.0, 24.0) = 43.266615305567875 mm
    // Crop factor rescales diagonal: sensor_diag = 43.2666 / crop_factor
    // Sensor height = sensor_diag / hypot(aspect_ratio, 1.0)
    // hugin_scale_in_millimeters = Sensor height / 2.0
    let full_frame_diag = (36.0_f64.powi(2) + 24.0_f64.powi(2)).sqrt();
    let aspect_diag_ratio = (aspect_ratio.powi(2) + 1.0).sqrt();
    let hugin_scale_in_millimeters = full_frame_diag / crop_factor / aspect_diag_ratio / 2.0;

    let hugin_scaling = real_focal / hugin_scale_in_millimeters;

    // Rescale distortion coefficients according to model formulas
    match dist.model.as_str() {
        "poly3" => {
            Poly3::rescale_coeffs(&mut k, hugin_scaling);
        }
        "poly5" => {
            Poly5::rescale_coeffs(&mut k, hugin_scaling);
        }
        "ptlens" => {
            PtLens::rescale_coeffs(&mut k, hugin_scaling);
        }
        _ => {}
    }

    // Focal length in pixels: fx = fy = hugin_scaling * (height / 2.0)
    let fy = hugin_scaling * (height as f64 / 2.0);
    let fx = fy;
    let cx = width as f64 / 2.0;
    let cy = height as f64 / 2.0;

    let mut profile = LensProfile::default();
    profile.camera_brand = if !lens.maker.is_empty() { lens.maker.clone() } else { "Lensfun".into() };
    profile.camera_model = if !lens.mounts.is_empty() { lens.mounts[0].clone() } else { "Generic".into() };
    profile.lens_model = if !lens.model.is_empty() { lens.model.clone() } else { format!("{}mm", nominal_focal) };
    profile.camera_setting = "Lensfun DB".into();
    profile.calibrated_by = "Lensfun Database".into();
    profile.calibrator_version = "Lensfun-1.0".into();

    profile.calib_dimension = Dimensions { w: width, h: height };
    profile.orig_dimension = Dimensions { w: width, h: height };
    profile.fps = 0.0;
    profile.distortion_model = Some(dist.model.clone());
    profile.focal_length = Some(nominal_focal);
    profile.crop_factor = Some(crop_factor);
    profile.official = false;

    let id_maker = profile.camera_brand.replace(' ', "_");
    let id_lens = profile.lens_model.replace(' ', "_");
    profile.identifier = format!("lensfun_{}_{}_{:.0}mm_{}", id_maker, id_lens, nominal_focal, dist.model);
    profile.name = format!("{}_{}_{}_{}mm_{}", profile.camera_brand, profile.camera_model, profile.lens_model, nominal_focal as i32, dist.model);

    profile.fisheye_params = CameraParams {
        RMS_error: 0.0,
        camera_matrix: vec![
            [fx, 0.0, cx],
            [0.0, fy, cy],
            [0.0, 0.0, 1.0],
        ],
        distortion_coeffs: k,
        radial_distortion_limit: None,
    };

    profile.init();
    profile
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_LENSFUN_XML: &str = r#"
    <lensdatabase version="1">
        <lens>
            <maker>Sony</maker>
            <model>FE 28mm F2</model>
            <mount>Sony E</mount>
            <cropfactor>1.0</cropfactor>
            <calibration>
                <distortion model="poly3" focal="28" k1="-0.0185" />
                <tca model="poly3" focal="28" vr="1.0001" vb="0.9998" />
                <vignetting model="pa" focal="28" aperture="2.0" distance="1000" k1="-0.5" k2="0.2" k3="-0.1" />
            </calibration>
        </lens>
        <lens>
            <maker>Fujifilm</maker>
            <model>XF 23mm F2 R WR</model>
            <mount>Fujifilm X</mount>
            <cropfactor>1.5</cropfactor>
            <calibration>
                <distortion model="poly5" focal="23" k1="-0.012" k2="0.003" />
            </calibration>
        </lens>
        <lens>
            <maker>Sigma</maker>
            <model>16mm F1.4 DC DN</model>
            <mount>Micro 4/3</mount>
            <cropfactor>2.0</cropfactor>
            <calibration>
                <distortion model="ptlens" focal="16" a="0.004" b="-0.018" c="0.009" />
            </calibration>
        </lens>
    </lensdatabase>
    "#;

    #[test]
    fn test_parse_lensfun_xml_entries() {
        let profiles = parse_lensfun_xml(SAMPLE_LENSFUN_XML).expect("Failed to parse Lensfun XML");
        assert_eq!(profiles.len(), 3);

        // 1. Sony FE 28mm F2 (poly3)
        let p_sony = &profiles[0];
        assert_eq!(p_sony.camera_brand, "Sony");
        assert_eq!(p_sony.lens_model, "FE 28mm F2");
        assert_eq!(p_sony.distortion_model.as_deref(), Some("poly3"));
        assert_eq!(p_sony.focal_length, Some(28.0));
        assert_eq!(p_sony.crop_factor, Some(1.0));
        assert_eq!(p_sony.fisheye_params.camera_matrix.len(), 3);
        assert!(!p_sony.fisheye_params.distortion_coeffs.is_empty());

        // 2. Fujifilm XF 23mm F2 (poly5)
        let p_fuji = &profiles[1];
        assert_eq!(p_fuji.camera_brand, "Fujifilm");
        assert_eq!(p_fuji.distortion_model.as_deref(), Some("poly5"));
        assert_eq!(p_fuji.crop_factor, Some(1.5));
        assert_eq!(p_fuji.fisheye_params.distortion_coeffs.len(), 2);

        // 3. Sigma 16mm (ptlens)
        let p_sigma = &profiles[2];
        assert_eq!(p_sigma.camera_brand, "Sigma");
        assert_eq!(p_sigma.distortion_model.as_deref(), Some("ptlens"));
        assert_eq!(p_sigma.crop_factor, Some(2.0));
        assert_eq!(p_sigma.fisheye_params.distortion_coeffs.len(), 3);
    }

    #[test]
    fn test_crop_factor_rescaling_math() {
        let lens_ff = LensfunLens {
            maker: "TestMaker".into(),
            model: "Lens FF".into(),
            mounts: vec!["E-Mount".into()],
            crop_factor: 1.0,
            aspect_ratio: None,
            calibrations: vec![],
        };
        let dist = LensfunDistortion {
            model: "poly3".into(),
            focal: 28.0,
            real_focal: None,
            k1: Some(-0.02),
            k2: None,
            a: None,
            b: None,
            c: None,
        };

        let prof_ff = convert_distortion_to_profile(&lens_ff, &dist, 3840, 2160);
        let fx_ff = prof_ff.fisheye_params.camera_matrix[0][0];

        let mut lens_apsc = lens_ff.clone();
        lens_apsc.crop_factor = 1.5;
        let prof_apsc = convert_distortion_to_profile(&lens_apsc, &dist, 3840, 2160);
        let fx_apsc = prof_apsc.fisheye_params.camera_matrix[0][0];

        // APS-C (1.5x crop) has smaller sensor diagonal, so focal length in pixels must be 1.5x larger!
        let ratio = fx_apsc / fx_ff;
        assert!((ratio - 1.5).abs() < 1e-4, "Expected 1.5x scaling ratio for APS-C crop, got {}", ratio);
    }

    #[test]
    fn test_aspect_ratio_rescaling() {
        let lens = LensfunLens {
            maker: "Test".into(),
            model: "Lens 50mm".into(),
            mounts: vec![],
            crop_factor: 1.0,
            aspect_ratio: None,
            calibrations: vec![],
        };
        let dist = LensfunDistortion {
            model: "ptlens".into(),
            focal: 50.0,
            real_focal: None,
            k1: None,
            k2: None,
            a: Some(0.01),
            b: Some(-0.02),
            c: Some(0.005),
        };

        // 16:9 vs 4:3
        let p_16_9 = convert_distortion_to_profile(&lens, &dist, 3840, 2160);
        let p_4_3 = convert_distortion_to_profile(&lens, &dist, 2880, 2160);

        assert!(p_16_9.fisheye_params.camera_matrix[0][0] > 0.0);
        assert!(p_4_3.fisheye_params.camera_matrix[0][0] > 0.0);
        // Both center coordinates cx, cy should be half of dimensions
        assert_eq!(p_16_9.fisheye_params.camera_matrix[0][2], 1920.0);
        assert_eq!(p_16_9.fisheye_params.camera_matrix[1][2], 1080.0);
        assert_eq!(p_4_3.fisheye_params.camera_matrix[0][2], 1440.0);
        assert_eq!(p_4_3.fisheye_params.camera_matrix[1][2], 1080.0);
    }
}
