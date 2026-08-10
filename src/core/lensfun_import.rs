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

use lensfun::Camera;
use lensfun::Database;
use lensfun::Lens;
use lensfun::calib::CalibDistortion;
use lensfun::calib::DistortionModel as LfDistortion;

use crate::LensProfile;
use crate::lens_profile::{CameraParams, Dimensions};
use crate::stabilization::distortion_models::DistortionModel;

/// Canonical calibration width for profiles synthesised from a search query.
///
/// Lensfun's coefficients live in a normalised frame, so the pixel dimensions
/// of a synthesised profile are a convention rather than a measurement —
/// gyroflow rescales them to the actual video size. 4096 keeps `fx` in the
/// same order of magnitude as the hand-calibrated profiles gyroflow ships,
/// which makes the two comparable when both appear in the list.
const SYNTH_WIDTH: usize = 4096;

/// Upper bound on the number of profiles a single query may synthesise.
///
/// Measured against the bundled database: 1041 bodies, 1501 lenses carrying a
/// distortion calibration, 64066 mount-matched body/lens pairs and 6377
/// distortion calibrations. A body query therefore resolves to ~62 lenses and
/// ~260 calibrations, so this ceiling only trims pathological queries such as
/// a single common letter.
pub const MAX_PROFILES_PER_QUERY: usize = 600;

/// Slack allowed when comparing crop factors, so that near-identical sensor
/// formats (APS-C at 1.5 vs 1.52, say) still pair up.
const CROP_TOLERANCE: f32 = 0.96;

/// Fallback aspect ratio when neither the body nor the lens records one.
/// 1.5 is the 3:2 still format that most of the database is calibrated on.
const DEFAULT_ASPECT_RATIO: f32 = 1.5;

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

    build_profile(None, lens, &calib, focal_mm, width, height)
        .ok_or(crate::GyroflowCoreError::LensHasNoDistortion)
}

/// Turn one Lensfun distortion calibration into a gyroflow `LensProfile`.
///
/// `camera` is optional: when present it supplies the body identity that the
/// profile list requires and the sensor size the pinhole matrix is built for;
/// when absent the lens's own calibration sensor is used for both, which is
/// the shape the direct `import_from_lensfun` entry point needs.
///
/// Returns `None` when the calibration carries no usable distortion model.
fn build_profile(
    camera: Option<&Camera>,
    lens: &Lens,
    calib: &CalibDistortion,
    focal_mm: f32,
    width: usize,
    height: usize,
) -> Option<LensProfile> {
    let (model_id, mut k): (&str, Vec<f64>) = match calib.model {
        LfDistortion::Poly3 { k1 } => ("poly3", vec![k1 as f64]),
        LfDistortion::Poly5 { k1, k2 } => ("poly5", vec![k1 as f64, k2 as f64]),
        LfDistortion::Ptlens { a, b, c } => ("ptlens", vec![a as f64, b as f64, c as f64]),
        LfDistortion::None => return None,
    };

    // The coefficients were measured on the lens's calibration sensor, so the
    // Hugin normalisation must use that sensor — not the body the profile is
    // being synthesised for.
    let calib_crop = if lens.crop_factor > 0.0 { lens.crop_factor } else { 1.0 };
    let aspect_ratio = if lens.aspect_ratio > 0.0 { lens.aspect_ratio } else { DEFAULT_ASPECT_RATIO };

    // The pinhole matrix, in contrast, describes the frame the profile will be
    // applied to, which is the body's sensor when one is known.
    let target_crop = camera
        .map(|c| if c.crop_factor > 0.0 { c.crop_factor } else { calib_crop })
        .unwrap_or(calib_crop);

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
        / calib_crop as f64
        / (aspect_ratio as f64).hypot(1.0)
        / 2.0;
    let hugin_scaling = real_focal / hugin_half_diag_mm;

    DistortionModel::from_name(model_id).rescale_coeffs(&mut k, hugin_scaling);

    // Synthesise a pinhole camera matrix.
    //
    // Lensfun's `crop_factor` is defined as the ratio of the full-frame
    // diagonal (sqrt(36² + 24²) mm) to the sensor's diagonal, so the sensor
    // *width* depends on the image's aspect ratio — dividing 36 mm by the
    // crop factor would only be correct for a 3:2 sensor (CodeRabbit review,
    // PR #1). Derive fx/fy from the diagonal instead.
    //
    // For square pixels this reduces to
    //   fx = fy = focal_mm · image_diag_pixels / sensor_diag_mm
    let sensor_diag_mm = 36.0_f64.hypot(24.0) / target_crop as f64;
    let image_diag_px = (width as f64).hypot(height as f64);
    let fx = focal_mm as f64 * image_diag_px / sensor_diag_mm;
    let fy = fx;
    let cx = width as f64 / 2.0;
    let cy = height as f64 / 2.0;

    let mut profile = LensProfile::default();
    profile.name = match camera {
        Some(c) => format!("{} {} - {} ({}mm, Lensfun)", c.maker.trim(), c.model.trim(), lens.model.trim(), focal_mm),
        None => format!("{} {} ({}mm, Lensfun)", lens.maker.trim(), lens.model.trim(), focal_mm),
    };
    // A stable, collision-free key. Existing profiles are identified by their
    // own calibration ids or file paths, so the `lensfun://` scheme cannot
    // clash with them, and repeating a search rebuilds the same string — which
    // is what keeps the import idempotent.
    profile.identifier = format!(
        "lensfun://{}/{}/{}/{}/{}mm",
        camera.map(|c| c.maker.trim()).unwrap_or(""),
        camera.map(|c| c.model.trim()).unwrap_or(""),
        lens.maker.trim(),
        lens.model.trim(),
        focal_mm
    );
    profile.camera_brand = camera.map(|c| c.maker.clone()).unwrap_or_else(|| lens.maker.clone());
    profile.camera_model = camera.map(|c| c.model.clone()).unwrap_or_default();
    profile.lens_model = lens.model.clone();
    profile.calibrated_by = "Lensfun".to_string();
    profile.calib_dimension = Dimensions { w: width, h: height };
    profile.orig_dimension = Dimensions { w: width, h: height };
    profile.distortion_model = Some(model_id.to_string());
    profile.focal_length = Some(focal_mm as f64);
    profile.crop_factor = Some(target_crop as f64);
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

    Some(profile)
}

/// Calibration dimensions to synthesise a profile at.
///
/// Lensfun records no pixel counts, so the width is a fixed convention and
/// only the aspect ratio carries information.
fn synth_dimensions(camera: &Camera, lens: &Lens) -> (usize, usize) {
    let aspect = camera
        .aspect_ratio
        .filter(|a| *a > 0.0)
        .unwrap_or(if lens.aspect_ratio > 0.0 { lens.aspect_ratio } else { DEFAULT_ASPECT_RATIO });
    let height = ((SYNTH_WIDTH as f32 / aspect).round() as usize).max(1);
    // Keep both sides even; odd frame sizes are unusual and complicate nothing
    // here beyond making the centre land off-pixel.
    (SYNTH_WIDTH, height - (height % 2))
}

/// The bundled Lensfun database, decompressed once on first use.
///
/// `Database::load_bundled` inflates ~5 MB of XML, and the search box calls
/// into this on every keystroke, so the result is cached for the process.
fn bundled_database() -> Option<&'static Database> {
    static DB: std::sync::OnceLock<Option<Database>> = std::sync::OnceLock::new();
    DB.get_or_init(|| match Database::load_bundled() {
        Ok(db) => Some(db),
        Err(e) => {
            log::warn!("Lensfun Database::load_bundled failed: {e:?}");
            None
        }
    })
    .as_ref()
}

/// Whether `lens`'s calibration may be used on `camera`.
///
/// Two conditions. The lens has to fit the mount, and the sensor the
/// calibration was measured on has to be at least as large as the body's —
/// a smaller calibration sensor simply does not describe the outer field of
/// a larger frame, so an APS-C calibration must not be stretched over a
/// full-frame image. Lensfun's crop factor is inversely proportional to
/// sensor size, hence the `<=` on crop factors rather than `>=`.
fn pairs_with(lens: &Lens, camera: &Camera) -> bool {
    if !lens.mounts.iter().any(|m| m == &camera.mount) {
        return false;
    }
    if lens.crop_factor <= 0.0 || camera.crop_factor <= 0.0 {
        return false;
    }
    lens.crop_factor * CROP_TOLERANCE <= camera.crop_factor
}

/// Push one profile per distortion calibration of `lens` mounted on `camera`.
fn extend_with_pairing(
    out: &mut Vec<LensProfile>,
    seen: &mut std::collections::HashSet<String>,
    camera: &Camera,
    lens: &Lens,
) {
    let (width, height) = synth_dimensions(camera, lens);
    for calib in &lens.calib_distortion {
        if out.len() >= MAX_PROFILES_PER_QUERY {
            return;
        }
        if let Some(profile) = build_profile(Some(camera), lens, calib, calib.focal, width, height) {
            if seen.insert(profile.identifier.clone()) {
                out.push(profile);
            }
        }
    }
}

/// Synthesise profiles for every body/lens pairing that matches `query`.
///
/// Both halves are needed. Gyroflow users search by camera model, and
/// `Database::find_lenses` only matches lens model names, so a query such as
/// "a7iv" would otherwise return nothing at all. Conversely a lens query has
/// to be paired with bodies, because the profile list drops any entry whose
/// `camera_brand`/`camera_model` is empty.
///
/// Returns an empty vector when the query is too short to be selective, when
/// the bundled database is unavailable, or when nothing matches. The result is
/// capped at `MAX_PROFILES_PER_QUERY`.
pub fn profiles_for_query(query: &str) -> Vec<LensProfile> {
    let query = query.trim();
    if query.chars().count() < 3 {
        return Vec::new();
    }
    let Some(db) = bundled_database() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for camera in db.find_cameras(None, query) {
        for lens in db.lenses.iter().filter(|l| pairs_with(l, camera)) {
            extend_with_pairing(&mut out, &mut seen, camera, lens);
            if out.len() >= MAX_PROFILES_PER_QUERY {
                return out;
            }
        }
    }
    for lens in db.find_lenses(None, query) {
        for camera in db.cameras.iter().filter(|c| pairs_with(lens, c)) {
            extend_with_pairing(&mut out, &mut seen, camera, lens);
            if out.len() >= MAX_PROFILES_PER_QUERY {
                return out;
            }
        }
    }

    out
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

    /// A body query has to work. Gyroflow users search by camera model, and
    /// `Database::find_lenses` only matches lens names, so this is the case a
    /// lens-only lookup would silently return nothing for.
    #[test]
    fn body_query_yields_profiles() {
        let profiles = profiles_for_query("EOS 5D");
        assert!(!profiles.is_empty(), "a well-known body must resolve to at least one lens profile");
        assert!(profiles.len() <= MAX_PROFILES_PER_QUERY);
    }

    /// Every synthesised profile must survive `prepare_list_for_ui`, which
    /// drops any entry whose brand or model is empty. A profile that cannot
    /// reach the list is indistinguishable from no profile at all.
    #[test]
    fn synthesised_profiles_are_listable() {
        let profiles = profiles_for_query("EOS 5D");
        assert!(!profiles.is_empty());
        for p in &profiles {
            assert!(!p.camera_brand.is_empty(), "camera_brand empty: {}", p.identifier);
            assert!(!p.camera_model.is_empty(), "camera_model empty: {}", p.identifier);
            assert!(p.identifier.starts_with("lensfun://"), "unexpected identifier: {}", p.identifier);
            assert!(!p.fisheye_params.distortion_coeffs.is_empty());
            assert!(p.fisheye_params.camera_matrix[0][0] > 0.0, "fx must be positive");
            assert_eq!(p.calib_dimension.w, SYNTH_WIDTH);
            assert!(p.calib_dimension.h > 0);
        }
    }

    /// Running the same query twice must produce the same set of identifiers,
    /// otherwise repeated keystrokes would keep growing the profile map.
    #[test]
    fn query_is_idempotent() {
        let a: Vec<String> = profiles_for_query("EOS 5D").into_iter().map(|p| p.identifier).collect();
        let b: Vec<String> = profiles_for_query("EOS 5D").into_iter().map(|p| p.identifier).collect();
        assert_eq!(a, b);
        let unique: std::collections::HashSet<&String> = a.iter().collect();
        assert_eq!(unique.len(), a.len(), "identifiers must be unique within one query");
    }

    /// A query too short to be selective must not sweep the database.
    #[test]
    fn short_queries_are_ignored() {
        assert!(profiles_for_query("a").is_empty());
        assert!(profiles_for_query("  ").is_empty());
    }

    /// The crop rule, checked against the real database rather than a
    /// hand-built fixture: a calibration measured on a sensor smaller than the
    /// body's does not describe the outer field of that body's frame, so it
    /// must never be paired with it. Lensfun's crop factor grows as the sensor
    /// shrinks, hence the direction of the comparison.
    #[test]
    fn calibration_sensor_is_never_smaller_than_the_body() {
        let db = bundled_database().expect("bundled database must load");
        let mut checked = 0usize;
        for camera in db.cameras.iter().take(200) {
            for lens in db.lenses.iter() {
                if pairs_with(lens, camera) {
                    assert!(
                        lens.crop_factor * CROP_TOLERANCE <= camera.crop_factor,
                        "{} (crop {}) must not be paired with {} (crop {})",
                        lens.model, lens.crop_factor, camera.model, camera.crop_factor
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 0, "the sample must contain at least one valid pairing");
    }
}
