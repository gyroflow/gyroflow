// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Gyroflow contributors

use std::collections::HashSet;

use lensfun::DistortionModel as LensfunDistortionModel;
use lensfun::{CalibDistortion, Database, Lens};

use crate::lens_profile::{CameraParams, Dimensions};
use crate::stabilization::distortion_models::DistortionModel;
use crate::LensProfile;

pub const ID_PREFIX: &str = "lensfun://";

#[derive(Clone, Debug, PartialEq)]
pub struct LensfunImportSpec {
    pub maker: String,
    pub model: String,
    pub focal_mm: f32,
}

pub fn is_lensfun_identifier(value: &str) -> bool {
    value.starts_with(ID_PREFIX)
}

pub fn make_identifier(maker: &str, model: &str, focal_mm: f32) -> String {
    format!(
        "{}{}/{}?focal={:.3}",
        ID_PREFIX,
        urlencoding::encode(maker),
        urlencoding::encode(model),
        focal_mm
    )
}

pub fn parse_identifier(identifier: &str) -> Result<LensfunImportSpec, crate::GyroflowCoreError> {
    let rest = identifier.strip_prefix(ID_PREFIX).ok_or_else(|| {
        crate::GyroflowCoreError::InvalidLensfunIdentifier(identifier.to_string())
    })?;
    let (path, query) = rest.split_once("?focal=").ok_or_else(|| {
        crate::GyroflowCoreError::InvalidLensfunIdentifier(identifier.to_string())
    })?;
    let (maker, model) = path.split_once('/').ok_or_else(|| {
        crate::GyroflowCoreError::InvalidLensfunIdentifier(identifier.to_string())
    })?;
    let focal = query
        .split('&')
        .next()
        .unwrap_or_default()
        .parse::<f32>()
        .map_err(|_| crate::GyroflowCoreError::InvalidLensfunIdentifier(identifier.to_string()))?;

    let maker = decode_component(maker, identifier)?;
    let model = decode_component(model, identifier)?;
    if maker.trim().is_empty() || model.trim().is_empty() || focal <= 0.0 || !focal.is_finite() {
        return Err(crate::GyroflowCoreError::InvalidLensfunIdentifier(
            identifier.to_string(),
        ));
    }

    Ok(LensfunImportSpec {
        maker,
        model,
        focal_mm: focal,
    })
}

pub fn import_from_identifier(
    identifier: &str,
    width: usize,
    height: usize,
) -> Result<LensProfile, crate::GyroflowCoreError> {
    let spec = parse_identifier(identifier)?;
    import_from_bundled(&spec.maker, &spec.model, spec.focal_mm, width, height)
}

pub fn import_from_bundled(
    maker: &str,
    model: &str,
    focal_mm: f32,
    width: usize,
    height: usize,
) -> Result<LensProfile, crate::GyroflowCoreError> {
    let db = load_bundled_database()?;
    let lens = find_exact_lens(&db, maker, model)
        .or_else(|| {
            db.find_lenses(None, model)
                .into_iter()
                .find(|lens| lens.maker.eq_ignore_ascii_case(maker))
        })
        .ok_or_else(|| {
            crate::GyroflowCoreError::LensfunProfileNotFound(format!("{maker} {model}"))
        })?;

    profile_from_lens(lens, focal_mm, width, height)
}

pub fn bundled_profiles_for_ui(
    width: usize,
    height: usize,
) -> Result<Vec<LensProfile>, crate::GyroflowCoreError> {
    let db = load_bundled_database()?;
    let mut profiles = Vec::new();

    for lens in &db.lenses {
        let mut focals = HashSet::<String>::new();
        for calib in &lens.calib_distortion {
            if !is_supported_distortion(calib) {
                continue;
            }
            let focal_key = format!("{:.3}", calib.focal);
            if !focals.insert(focal_key) {
                continue;
            }
            if let Ok(profile) = profile_from_lens(lens, calib.focal, width, height) {
                profiles.push(profile);
            }
        }
    }

    Ok(profiles)
}

fn load_bundled_database() -> Result<Database, crate::GyroflowCoreError> {
    Database::load_bundled()
        .map_err(|err| crate::GyroflowCoreError::LensfunDbLoadFailed(err.to_string()))
}

fn find_exact_lens<'a>(db: &'a Database, maker: &str, model: &str) -> Option<&'a Lens> {
    db.lenses.iter().find(|lens| {
        lens.maker.eq_ignore_ascii_case(maker) && lens.model.eq_ignore_ascii_case(model)
    })
}

fn profile_from_lens(
    lens: &Lens,
    focal_mm: f32,
    width: usize,
    height: usize,
) -> Result<LensProfile, crate::GyroflowCoreError> {
    let (width, height) = normalize_dimensions(lens, width, height);
    let calib = lens.interpolate_distortion(focal_mm).ok_or_else(|| {
        crate::GyroflowCoreError::LensfunProfileUnsupported(format!(
            "{} {} at {:.3}mm",
            lens.maker, lens.model, focal_mm
        ))
    })?;
    let (model_id, coeffs, real_focal) = converted_coefficients(lens, &calib, focal_mm)?;

    let crop_factor = usable_positive(lens.crop_factor as f64).unwrap_or(1.0);
    let sensor_diag_mm = 36.0_f64.hypot(24.0) / crop_factor;
    let image_diag_px = (width as f64).hypot(height as f64);
    let fx = real_focal * image_diag_px / sensor_diag_mm;
    let fy = fx;

    let short_side = width.min(height) as f64;
    let cx = width as f64 / 2.0 + short_side / 2.0 * lens.center_x as f64;
    let cy = height as f64 / 2.0 + short_side / 2.0 * lens.center_y as f64;

    let identifier = make_identifier(&lens.maker, &lens.model, focal_mm);
    let checksum_src = format!(
        "{identifier}|{width}x{height}|{model_id}|{fx:.8}|{fy:.8}|{cx:.8}|{cy:.8}|{coeffs:?}"
    );

    let mut profile = LensProfile::default();
    profile.name = format!(
        "Lensfun: {} {} @ {:.1}mm",
        lens.maker.trim(),
        lens.model.trim(),
        focal_mm
    );
    profile.note = "Imported from the bundled Lensfun database".to_string();
    profile.calibrated_by = "Lensfun".to_string();
    profile.camera_brand = lens.maker.clone();
    profile.camera_model = "Lensfun database".to_string();
    profile.lens_model = lens.model.clone();
    profile.camera_setting = format!("{:.1}mm", focal_mm);
    profile.calib_dimension = Dimensions {
        w: width,
        h: height,
    };
    profile.orig_dimension = Dimensions {
        w: width,
        h: height,
    };
    profile.input_horizontal_stretch = 1.0;
    profile.input_vertical_stretch = 1.0;
    profile.num_images = 0;
    profile.fps = 0.0;
    profile.official = true;
    profile.fisheye_params = CameraParams {
        RMS_error: 0.0,
        camera_matrix: vec![[fx, 0.0, cx], [0.0, fy, cy], [0.0, 0.0, 1.0]],
        distortion_coeffs: coeffs,
        radial_distortion_limit: None,
    };
    profile.identifier = identifier.clone();
    profile.path_to_file = identifier;
    profile.calibrator_version = env!("CARGO_PKG_VERSION").to_string();
    profile.distortion_model = Some(model_id.to_string());
    profile.focal_length = Some(focal_mm as f64);
    profile.crop_factor = Some(crop_factor);
    profile.checksum = Some(format!("{:08x}", crc32fast::hash(checksum_src.as_bytes())));
    profile.init();

    Ok(profile)
}

fn converted_coefficients(
    lens: &Lens,
    calib: &CalibDistortion,
    focal_mm: f32,
) -> Result<(&'static str, Vec<f64>, f64), crate::GyroflowCoreError> {
    let (model_id, mut coeffs) = match calib.model {
        LensfunDistortionModel::Poly3 { k1 } => ("poly3", vec![k1 as f64]),
        LensfunDistortionModel::Poly5 { k1, k2 } => ("poly5", vec![k1 as f64, k2 as f64]),
        LensfunDistortionModel::Ptlens { a, b, c } => {
            ("ptlens", vec![a as f64, b as f64, c as f64])
        }
        LensfunDistortionModel::None => {
            return Err(crate::GyroflowCoreError::LensfunProfileUnsupported(
                format!("{} {} has no distortion model", lens.maker, lens.model),
            ));
        }
    };

    let crop_factor = usable_positive(lens.crop_factor as f64).unwrap_or(1.0);
    let aspect_ratio = usable_positive(lens.aspect_ratio as f64).unwrap_or(1.5);
    let real_focal =
        usable_positive(calib.real_focal.unwrap_or(focal_mm) as f64).unwrap_or(focal_mm as f64);
    let hugin_half_diag_mm = 36.0_f64.hypot(24.0) / crop_factor / aspect_ratio.hypot(1.0) / 2.0;
    let hugin_scaling = real_focal / hugin_half_diag_mm;

    if !DistortionModel::from_name(model_id).rescale_coeffs(&mut coeffs, hugin_scaling) {
        return Err(crate::GyroflowCoreError::LensfunProfileUnsupported(
            format!(
                "{} {} uses unsupported model {model_id}",
                lens.maker, lens.model
            ),
        ));
    }

    Ok((model_id, coeffs, real_focal))
}

fn is_supported_distortion(calib: &CalibDistortion) -> bool {
    matches!(
        calib.model,
        LensfunDistortionModel::Poly3 { .. }
            | LensfunDistortionModel::Poly5 { .. }
            | LensfunDistortionModel::Ptlens { .. }
    )
}

fn normalize_dimensions(lens: &Lens, width: usize, height: usize) -> (usize, usize) {
    if width >= 2 && height >= 2 {
        return (width, height);
    }

    let aspect_ratio = usable_positive(lens.aspect_ratio as f64).unwrap_or(1.5);
    let fallback_width = 3840_usize;
    let fallback_height = ((fallback_width as f64 / aspect_ratio).round() as usize).max(2);
    (fallback_width, fallback_height)
}

fn usable_positive(value: f64) -> Option<f64> {
    if value.is_finite() && value > 0.0 {
        Some(value)
    } else {
        None
    }
}

fn decode_component(component: &str, identifier: &str) -> Result<String, crate::GyroflowCoreError> {
    urlencoding::decode(component)
        .map(|value| value.into_owned())
        .map_err(|_| crate::GyroflowCoreError::InvalidLensfunIdentifier(identifier.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lensfun::Modifier;

    const CANON_24_70: &str = "Canon EF 24-70mm f/2.8L II USM";

    #[test]
    fn lensfun_identifier_round_trips() {
        let id = make_identifier("Canon", CANON_24_70, 35.0);
        let parsed = parse_identifier(&id).unwrap();

        assert_eq!(parsed.maker, "Canon");
        assert_eq!(parsed.model, CANON_24_70);
        assert!((parsed.focal_mm - 35.0).abs() < f32::EPSILON);
    }

    #[test]
    fn imports_bundled_lensfun_profile_for_current_dimensions() {
        let profile = import_from_bundled("Canon", CANON_24_70, 35.0, 6720, 4480)
            .expect("bundled Lensfun database should include the Canon 24-70 fixture");

        assert!(is_lensfun_identifier(&profile.identifier));
        assert_eq!(profile.calib_dimension.w, 6720);
        assert_eq!(profile.calib_dimension.h, 4480);
        assert_eq!(profile.focal_length, Some(35.0));
        assert!(matches!(
            profile.distortion_model.as_deref(),
            Some("poly3" | "poly5" | "ptlens")
        ));
        assert!(!profile.fisheye_params.distortion_coeffs.is_empty());
        assert_eq!(profile.fisheye_params.camera_matrix.len(), 3);
    }

    #[test]
    fn bundled_profiles_include_searchable_lensfun_entries() {
        let profiles =
            bundled_profiles_for_ui(3840, 2560).expect("bundled Lensfun database should load");
        assert!(profiles.iter().any(|profile| {
            profile.lens_model == CANON_24_70 && profile.focal_length == Some(35.0)
        }));
    }

    #[test]
    fn gyroflow_coefficients_match_lensfun_modifier_geometry() {
        const W: u32 = 6720;
        const H: u32 = 4480;
        const FOCAL: f32 = 35.0;

        let db = load_bundled_database().expect("bundled Lensfun database should load");
        let lens =
            find_exact_lens(&db, "Canon", CANON_24_70).expect("Canon 24-70 fixture should exist");
        let profile = import_from_bundled("Canon", CANON_24_70, FOCAL, W as usize, H as usize)
            .expect("profile import should succeed");

        let mut modifier = Modifier::new(lens, FOCAL, lens.crop_factor, W, H, false);
        assert!(modifier.enable_distortion_correction(lens));

        let model_id = profile.distortion_model.as_deref().unwrap();
        let coeffs = profile.fisheye_params.distortion_coeffs.clone();
        let fx = profile.fisheye_params.camera_matrix[0][0];
        let fy = profile.fisheye_params.camera_matrix[1][1];
        let cx = profile.fisheye_params.camera_matrix[0][2];
        let cy = profile.fisheye_params.camera_matrix[1][2];

        let gyroflow_distort = |px: f32, py: f32| -> (f32, f32) {
            let x = (px as f64 - cx) / fx;
            let y = (py as f64 - cy) / fy;
            let r2 = x * x + y * y;
            let r = r2.sqrt();
            let scale = match model_id {
                "poly3" => 1.0 + coeffs[0] * r2,
                "poly5" => 1.0 + coeffs[0] * r2 + coeffs[1] * r2 * r2,
                "ptlens" => 1.0 + coeffs[0] * r2 * r + coeffs[1] * r2 + coeffs[2] * r,
                other => panic!("unexpected model {other}"),
            };
            ((x * scale * fx + cx) as f32, (y * scale * fy + cy) as f32)
        };

        let test_points = [
            (W as f32 * 0.50, H as f32 * 0.50),
            (W as f32 * 0.25, H as f32 * 0.50),
            (W as f32 * 0.75, H as f32 * 0.50),
            (W as f32 * 0.50, H as f32 * 0.90),
            (W as f32 * 0.10, H as f32 * 0.10),
            (W as f32 * 0.95, H as f32 * 0.95),
        ];

        let mut coords = [0.0_f32; 2];
        for (index, (px, py)) in test_points.into_iter().enumerate() {
            assert!(modifier.apply_geometry_distortion(px, py, 1, 1, &mut coords));
            let (gx, gy) = gyroflow_distort(px, py);
            let dx = (gx - coords[0]).abs();
            let dy = (gy - coords[1]).abs();

            assert!(
                dx < 2.0 && dy < 2.0,
                "point {index}: gyroflow=({gx}, {gy}) lensfun=({}, {}) diff=({dx}, {dy})",
                coords[0],
                coords[1]
            );
        }
    }
}
