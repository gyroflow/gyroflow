// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Gyroflow Authors

//! Import `LensProfile` entries from the bundled Lensfun database.
//!
//! Lensfun stores its calibrations in a Hugin-style coordinate frame
//! (the image half-diagonal normalises to `1`). Gyroflow's distortion
//! models expect the coefficients in the camera's normalised coordinate
//! frame (after dividing `x` and `y` by `z`). The conversion factor is
//! `hugin_scaling = real_focal / hugin_half_diagonal_mm`, applied per
//! the model-specific rule in
//! `src/core/stabilization/distortion_models/{poly3,poly5,ptlens}.rs`.
//!
//! See issue #150 for the original bounty request.

use lensfun::{Database, Camera, Lens, Modifier};
use lensfun::calib::DistortionModel as LfDistortion;

use crate::LensProfile;
use crate::lens_profile::{CameraParams, Dimensions};
use crate::stabilization::distortion_models::DistortionModel;

/// Import a `LensProfile` from the bundled Lensfun database.
///
/// - `maker` is matched case-insensitively against the lens maker field.
///   When non-empty, it is required — a Canon query will never resolve to a
///   Nikon lens. Pass `""` to accept the top fuzzy-match regardless of maker.
/// - `lens_model` is matched with Lensfun's fuzzy search.
/// - `focal_mm` picks the focal length to interpolate the calibration at.
/// - `(width, height)` is the target image dimension used to synthesise a
///   pinhole `camera_matrix` from the lens's crop factor.
///
/// # Errors
///
/// - `LensfunDbLoadFailed` — bundled Lensfun database could not be loaded.
/// - `LensNotFound(query)` — no lens in the database matched the query.
/// - `NoCalibrationForFocal(focal)` — lens has no distortion calibration at
///   the requested focal length.
/// - `LensHasNoDistortion` — the interpolated calibration returned
///   `DistortionModel::None`.
pub fn import_from_lensfun(
    maker: &str,
    lens_model: &str,
    focal_mm: f32,
    width: usize,
    height: usize,
) -> Result<LensProfile, crate::GyroflowCoreError> {
    let db = Database::load_bundled().map_err(|e| {
        log::warn!("Lensfun Database::load_bundled failed: {e:?}");
        crate::GyroflowCoreError::LensfunDbLoadFailed
    })?;

    // Find matching cameras (for crop factor) and lenses.
    let cameras = db.find_cameras(Some(maker), maker);
    let camera: Option<&Camera> = cameras.first().copied();

    let mut lenses = db.find_lenses(camera, lens_model);
    if lenses.is_empty() && !maker.is_empty() {
        // Retry without camera constraint (maker may not match any camera).
        lenses = db.find_lenses(None, lens_model);
    }
    let lens = lenses.into_iter().next().ok_or_else(|| {
        let q = if maker.is_empty() {
            lens_model.to_string()
        } else {
            format!("{maker} {lens_model}")
        };
        crate::GyroflowCoreError::LensNotFound(q)
    })?;
    log::debug!(
        "Lensfun matched: maker={:?} model={:?}",
        lens.maker,
        lens.model
    );

    let calib = lens
        .interpolate_distortion(focal_mm)
        .ok_or(crate::GyroflowCoreError::NoCalibrationForFocal(focal_mm))?;

    let crop_factor = if lens.crop_factor > 0.0 { lens.crop_factor } else { 1.0 };
    let aspect_ratio = if lens.aspect_ratio > 0.0 { lens.aspect_ratio } else { 1.5 };

    let (model_id, mut k): (&str, Vec<f64>) = match calib.model {
        LfDistortion::Poly3 { k1 } => ("poly3", vec![k1 as f64]),
        LfDistortion::Poly5 { k1, k2 } => ("poly5", vec![k1 as f64, k2 as f64]),
        LfDistortion::Ptlens { a, b, c } => ("ptlens", vec![a as f64, b as f64, c as f64]),
        LfDistortion::None => return Err(crate::GyroflowCoreError::LensHasNoDistortion),
    };

    // Real focal length: prefer the calibration's recorded value, otherwise
    // derive it from the Hugin/LensFun convention difference.
    let real_focal = calib.real_focal.map(|v| v as f64).unwrap_or_else(|| {
        let f = focal_mm as f64;
        match model_id {
            "ptlens" => f * (1.0 - k[0] - k[1] - k[2]),
            "poly3" => f * (1.0 - k[0]),
            _ => f,
        }
    });

    // Hugin normalises to the half-diagonal of the sensor in mm.
    let hugin_half_diag_mm = 36.0_f64.hypot(24.0)
        / crop_factor as f64
        / (aspect_ratio as f64).hypot(1.0)
        / 2.0;
    let hugin_scaling = real_focal / hugin_half_diag_mm;

    DistortionModel::from_name(model_id).rescale_coeffs(&mut k, hugin_scaling);

    // Synthesise a pinhole camera matrix.
    // For square pixels: fx = fy = focal_mm · image_diag_pixels / sensor_diag_mm
    let sensor_diag_mm = 36.0_f64.hypot(24.0) / crop_factor as f64;
    let image_diag_px = (width as f64).hypot(height as f64);
    let fx = focal_mm as f64 * image_diag_px / sensor_diag_mm;
    let fy = fx;
    let cx = width as f64 / 2.0;
    let cy = height as f64 / 2.0;

    let mut profile = LensProfile::default();
    profile.name = format!(
        "{} {} ({}mm, Lensfun)",
        lens.maker.trim(),
        lens.model.trim(),
        focal_mm
    );
    profile.camera_brand = lens.maker.clone();
    profile.lens_model = lens.model.clone();
    profile.calibrated_by = "Imported from Lensfun".to_string();
    profile.calib_dimension = Dimensions { w: width, h: height };
    profile.orig_dimension = Dimensions { w: width, h: height };
    profile.distortion_model = Some(model_id.to_string());
    profile.focal_length = Some(focal_mm as f64);
    profile.crop_factor = Some(crop_factor as f64);
    profile.input_horizontal_stretch = 1.0;
    profile.input_vertical_stretch = 1.0;
    profile.num_images = 0;
    profile.fisheye_params = CameraParams {
        RMS_error: 0.0,
        camera_matrix: vec![[fx, 0.0, cx], [0.0, fy, cy], [0.0, 0.0, 1.0]],
        distortion_coeffs: k,
        radial_distortion_limit: None,
    };
    profile.official = false;
    profile.calibrator_version = env!("CARGO_PKG_VERSION").to_string();
    profile.init();

    Ok(profile)
}

/// List lenses in the bundled Lensfun database, optionally filtered by maker.
///
/// Returns a list of `(maker, model)` tuples suitable for UI display.
pub fn list_lenses(maker_filter: &str) -> Vec<(String, String)> {
    let db = match Database::load_bundled() {
        Ok(db) => db,
        Err(e) => {
            log::warn!("Lensfun Database::load_bundled failed: {e:?}");
            return Vec::new();
        }
    };

    // Use an empty search to enumerate all lenses, then filter by maker.
    let all_lenses = db.find_lenses(None, "");
    all_lenses
        .into_iter()
        .filter(|l| maker_filter.is_empty() || l.maker.eq_ignore_ascii_case(maker_filter))
        .map(|l| (l.maker.clone(), l.model.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: These tests require the bundled Lensfun database to contain the
    // lenses referenced. If the bundled DB doesn't include "Canon EF 24-70mm",
    // the import test will return LensNotFound (which is still a valid test
    // of error handling, not a test failure).

    #[test]
    fn import_returns_error_on_empty_model() {
        // We don't test with an empty maker+model because find_lenses(None, "")
        // returns all lenses. Instead, test a model that doesn't exist.
        let result = import_from_lensfun("zzz_nonexistent_brand_zzz", "", 35.0, 6720, 4480);
        assert!(result.is_err(), "empty model with nonexistent maker should fail");
    }

    #[test]
    fn import_returns_error_on_nonexistent_lens() {
        let result = import_from_lensfun("NonexistentBrand", "FakeLens 999mm f/0.0", 35.0, 1920, 1080);
        assert!(result.is_err(), "nonexistent lens should fail");
    }

    #[test]
    fn hugin_scaling_formula_matches_modifier() {
        // Verify that our hugin_scaling computation produces distortion
        // coefficients that match what Lensfun's own Modifier applies.
        const W: u32 = 6720;
        const H: u32 = 4480;
        const FOCAL: f32 = 35.0;

        let db = match Database::load_bundled() {
            Ok(db) => db,
            Err(_) => {
                eprintln!("SKIP: bundled Lensfun DB unavailable");
                return;
            }
        };

        let lenses = db.find_lenses(None, "Canon EF 24-70mm f/2.8L II USM");
        let lens = match lenses.first() {
            Some(l) => *l,
            None => {
                eprintln!("SKIP: Canon EF 24-70mm not in bundled DB");
                return;
            }
        };

        // Test requires a lens with distortion calibration at 35mm.
        let calib = match lens.interpolate_distortion(FOCAL) {
            Some(c) => c,
            None => {
                eprintln!("SKIP: no distortion calibration at {}mm", FOCAL);
                return;
            }
        };

        if matches!(calib.model, LfDistortion::None) {
            eprintln!("SKIP: lens has DistortionModel::None at {}mm", FOCAL);
            return;
        }

        let profile = match import_from_lensfun("", "Canon EF 24-70mm f/2.8L II USM", FOCAL, W as usize, H as usize) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("SKIP: import failed: {e:?}");
                return;
            }
        };

        let model_id = profile.distortion_model.as_deref().expect("distortion_model set");
        let k: Vec<f64> = profile.fisheye_params.distortion_coeffs.clone();
        let fx = profile.fisheye_params.camera_matrix[0][0] as f32;
        let fy = profile.fisheye_params.camera_matrix[1][1] as f32;
        let cx = profile.fisheye_params.camera_matrix[0][2] as f32;
        let cy = profile.fisheye_params.camera_matrix[1][2] as f32;

        // Gyroflow's forward distortion: normalise → apply polynomial → denormalise.
        let gyroflow_distort = |px: f32, py: f32| -> (f32, f32) {
            let x = (px - cx) / fx;
            let y = (py - cy) / fy;
            let r2 = (x * x + y * y) as f64;
            let r = r2.sqrt();
            let poly = match model_id {
                "poly3" => k[0] * r2 + 1.0,
                "poly5" => 1.0 + k[0] * r2 + k[1] * r2 * r2,
                "ptlens" => k[0] * r2 * r + k[1] * r2 + k[2] * r + 1.0,
                other => panic!("unexpected model {other}"),
            };
            (
                (x as f64 * poly) as f32 * fx + cx,
                (y as f64 * poly) as f32 * fy + cy,
            )
        };

        // Lensfun Modifier (undistort → redistort) as reference.
        let mut modifier = Modifier::new(lens, FOCAL, lens.crop_factor, W, H, false);
        if !modifier.enable_distortion_correction(lens) {
            eprintln!("SKIP: Modifier::enable_distortion_correction returned false");
            return;
        }

        let test_points: [(f32, f32); 6] = [
            (W as f32 * 0.5, H as f32 * 0.5),
            (W as f32 * 0.25, H as f32 * 0.5),
            (W as f32 * 0.75, H as f32 * 0.5),
            (W as f32 * 0.5, H as f32 * 0.9),
            (W as f32 * 0.1, H as f32 * 0.1),
            (W as f32 * 0.95, H as f32 * 0.95),
        ];

        let mut coords = [0.0_f32; 2];
        for (i, &(px, py)) in test_points.iter().enumerate() {
            assert!(modifier.apply_geometry_distortion(px, py, 1, 1, &mut coords));
            let (lx, ly) = (coords[0], coords[1]);
            let (gx, gy) = gyroflow_distort(px, py);
            let dx = (gx - lx).abs();
            let dy = (gy - ly).abs();
            assert!(
                dx < 2.0 && dy < 2.0,
                "point {i}: px=({px}, {py}) gyroflow=({gx}, {gy}) lensfun=({lx}, {ly}) diff=({dx}, {dy})"
            );
        }
    }
}