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

use lensfun::Database;
use lensfun::Lens;
use lensfun::calib::DistortionModel as LfDistortion;

use crate::LensProfile;
use crate::lens_profile::{CameraParams, Dimensions};
use crate::stabilization::distortion_models::DistortionModel;

/// Resolve a lens from the Lensfun database by fuzzy `lens_model` match,
/// optionally constrained to a specific maker.
///
/// When `maker` is empty, the top-scoring fuzzy hit is returned. When a
/// non-empty `maker` is given, only a lens whose maker matches
/// case-insensitively is returned — there is no silent fallback to a
/// wrong-maker candidate, because a Canon query must never resolve to a
/// Nikon lens.
fn find_lens<'a>(db: &'a Database, maker: &str, lens_model: &str) -> Option<&'a Lens> {
    let lenses = db.find_lenses(None, lens_model);
    if maker.is_empty() {
        lenses.into_iter().next()
    } else {
        lenses
            .into_iter()
            .find(|l| l.maker.eq_ignore_ascii_case(maker))
    }
}

/// Import a `LensProfile` from the bundled Lensfun database.
///
/// - `maker` is matched case-insensitively. When non-empty, it is required
///   — a Canon query will never resolve to a Nikon lens. Pass `""` to
///   accept the top fuzzy-match regardless of maker.
/// - `lens_model` is matched with Lensfun's fuzzy search.
/// - `focal_mm` picks the focal length to interpolate the calibration at.
/// - `(width, height)` is the target image dimension used to synthesise a
///   pinhole `camera_matrix` from the lens's crop factor.
///
/// # Errors
///
/// - `LensfunDbLoadFailed` — bundled Lensfun database could not be loaded.
/// - `LensNotFound(query)` — no lens in the database matched the query
///   (or no lens matched the requested maker).
/// - `NoCalibrationForFocal(focal)` — lens exists but has no distortion
///   calibration at (or near) the requested focal length.
/// - `LensHasNoDistortion` — the interpolated calibration returned
///   `DistortionModel::None`, meaning the lens is flagged as producing no
///   distortion; synthesising a profile would be meaningless.
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

    let lens = find_lens(&db, maker, lens_model).ok_or_else(|| {
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
    //
    // LensFun's polynomials keep `Ru = 1 → Rd = 1` (the corner is fixed).
    // Gyroflow's underlying polynomials are in the Hugin convention
    // `Rd = 1 + k1` at the corner, so `real_focal` shrinks by the
    // zero-frequency coefficient sum — this shift is model-specific (Poly5
    // has no `(1 - k)` term, so the marketed focal is used verbatim). The
    // fallback formulas mirror the TODO block in
    // `src/core/stabilization/distortion_models/poly3.rs`.
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
    //
    // Lensfun's `crop_factor` is defined as the ratio of the full-frame
    // diagonal (sqrt(36² + 24²) mm) to the calibration sensor's diagonal,
    // so the sensor *width* depends on the image's aspect ratio — dividing
    // 36 mm by the crop factor would only be correct for a 3:2 sensor
    // (CodeRabbit review, PR #1). Derive fx/fy from the diagonal instead.
    //
    // For square pixels this reduces to
    //   fx = fy = focal_mm · image_diag_pixels / sensor_diag_mm
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

#[cfg(test)]
mod tests {
    use super::*;
    use lensfun::{Database, Modifier};

    const CANON_24_70: &str = "Canon EF 24-70mm f/2.8L II USM";

    /// Lensfun's bundled database must contain Canon EF 24-70mm f/2.8L II USM.
    /// If this test fails with `LensNotFound`, the bundled `lensfun` crate
    /// version no longer ships this calibration — pick a different fixture
    /// rather than silently skipping.
    #[test]
    fn imports_canon_ef_24_70() {
        let profile = import_from_lensfun("Canon", CANON_24_70, 35.0, 6720, 4480)
            .expect("bundled lensfun DB must include Canon EF 24-70mm");

        assert_eq!(profile.camera_brand, "Canon");
        assert_eq!(profile.calib_dimension.w, 6720);
        assert_eq!(profile.calib_dimension.h, 4480);
        assert_eq!(profile.focal_length, Some(35.0));
        assert!(matches!(
            profile.distortion_model.as_deref(),
            Some("poly3" | "poly5" | "ptlens")
        ));
        assert_eq!(profile.fisheye_params.camera_matrix.len(), 3);
        assert!(!profile.fisheye_params.distortion_coeffs.is_empty());

        let fx = profile.fisheye_params.camera_matrix[0][0];
        let cx = profile.fisheye_params.camera_matrix[0][2];
        let cy = profile.fisheye_params.camera_matrix[1][2];
        assert!(fx > 0.0, "fx must be positive");
        assert!((cx - 3360.0).abs() < 1e-6, "cx should be width/2");
        assert!((cy - 2240.0).abs() < 1e-6, "cy should be height/2");
    }

    /// `init()` must leave `radial_distortion_limit` in a consistent state:
    /// either `None` (no limit found in the polynomial's valid range) or a
    /// finite positive value.
    #[test]
    fn init_populates_radial_limit_when_applicable() {
        let profile = import_from_lensfun("", CANON_24_70, 35.0, 1920, 1280)
            .expect("bundled lensfun DB must include Canon EF 24-70mm");
        if let Some(limit) = profile.fisheye_params.radial_distortion_limit {
            assert!(limit.is_finite(), "radial limit must be finite when set");
            assert!(limit > 0.0, "radial limit must be positive");
        }
    }

    /// A/B comparison: the distortion produced by gyroflow's rescaled
    /// coefficients must match lensfun's own `Modifier::apply_geometry_distortion`
    /// for the same lens/focal/frame-size. If this test fails, the Hugin ↔
    /// gyroflow coefficient conversion (or the pinhole camera-matrix
    /// synthesis) is wrong.
    ///
    /// `Modifier::new(..., reverse=false)` simulates the lens's forward
    /// distortion, which matches the semantics of gyroflow's
    /// `distort_point`: a pinhole-projected pixel is mapped to where a real
    /// lens would actually record it.
    #[test]
    fn distortion_matches_lensfun_modifier() {
        const W: u32 = 6720;
        const H: u32 = 4480;
        const FOCAL: f32 = 35.0;

        let db = Database::load_bundled().expect("bundled DB must load");
        let lens = find_lens(&db, "Canon", CANON_24_70)
            .expect("bundled DB must include Canon EF 24-70mm");

        let profile = import_from_lensfun("Canon", CANON_24_70, FOCAL, W as usize, H as usize)
            .expect("import must succeed");

        // Pull the rescaled polynomial out of the imported profile.
        let model_id = profile
            .distortion_model
            .as_deref()
            .expect("distortion_model must be set on import");
        let k: Vec<f64> = profile.fisheye_params.distortion_coeffs.clone();
        let fx = profile.fisheye_params.camera_matrix[0][0] as f32;
        let fy = profile.fisheye_params.camera_matrix[1][1] as f32;
        let cx = profile.fisheye_params.camera_matrix[0][2] as f32;
        let cy = profile.fisheye_params.camera_matrix[1][2] as f32;

        // Applies the forward (simulate) polynomial in gyroflow's normalised
        // camera-coordinate frame. Mirrors the `distort_point` implementations
        // in `src/core/stabilization/distortion_models/{poly3,poly5,ptlens}.rs`.
        let gyroflow_distort = |px: f32, py: f32| -> (f32, f32) {
            let x = (px - cx) / fx;
            let y = (py - cy) / fy;
            let r2 = (x * x + y * y) as f64;
            let r = r2.sqrt();
            let poly = match model_id {
                "poly3" => k[0] * r2 + 1.0,
                "poly5" => 1.0 + k[0] * r2 + k[1] * r2 * r2,
                "ptlens" => k[0] * r2 * r + k[1] * r2 + k[2] * r + 1.0,
                other => panic!("unexpected distortion model {other}"),
            };
            (
                (x as f64 * poly) as f32 * fx + cx,
                (y as f64 * poly) as f32 * fy + cy,
            )
        };

        // Build a lensfun Modifier that simulates the same lens (reverse=false).
        let mut modifier = Modifier::new(lens, FOCAL, lens.crop_factor, W, H, false);
        assert!(
            modifier.enable_distortion_correction(lens),
            "lensfun Modifier should have a usable distortion calibration"
        );

        // Test points spread across the frame, including near-corner.
        let test_points = [
            (W as f32 * 0.5, H as f32 * 0.5),   // centre (should be ~identity)
            (W as f32 * 0.25, H as f32 * 0.5),  // mid-left
            (W as f32 * 0.75, H as f32 * 0.5),  // mid-right
            (W as f32 * 0.5, H as f32 * 0.9),   // near-bottom
            (W as f32 * 0.1, H as f32 * 0.1),   // upper-left
            (W as f32 * 0.95, H as f32 * 0.95), // near-bottom-right
        ];

        let mut coords = [0.0_f32; 2];
        for (i, &(px, py)) in test_points.iter().enumerate() {
            // Lensfun single-pixel pass.
            assert!(modifier.apply_geometry_distortion(px, py, 1, 1, &mut coords));
            let (lx, ly) = (coords[0], coords[1]);

            // Gyroflow polynomial pass.
            let (gx, gy) = gyroflow_distort(px, py);

            let dx = (gx - lx).abs();
            let dy = (gy - ly).abs();
            // Tolerance: 2 pixels on a 6720-wide frame = ~0.03 % of width.
            // Any mis-rescaled coefficient would blow this by orders of
            // magnitude (e.g. a factor-of-two error at the corner = ~2000 px).
            assert!(
                dx < 2.0 && dy < 2.0,
                "point {i}: px=({px}, {py}) gyroflow=({gx}, {gy}) lensfun=({lx}, {ly}) diff=({dx}, {dy})"
            );
        }
    }
}
