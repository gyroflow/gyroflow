// SPDX-License-Identifier: GPL-3.0-or-later
// Tests for CameraDatabase and lens profile compatibility (#742)

use gyroflow_core::lens_profile_database::{CameraDatabase, LensCalibration, LensProfileDatabase};

#[test]
fn test_camera_database_loading() {
    let mut db = LensProfileDatabase::default();
    db.load_all();

    // Verify brands
    let brands = db.camera_db.get_brands();
    assert!(!brands.is_empty(), "Camera brands should not be empty");
    assert!(brands.iter().any(|b| b == "Sony"), "Should contain Sony");
    assert!(brands.iter().any(|b| b == "GoPro"), "Should contain GoPro");
    assert!(brands.iter().any(|b| b == "Canon"), "Should contain Canon");

    // Verify Sony models
    let sony_models = db.camera_db.get_models("Sony");
    assert!(!sony_models.is_empty(), "Sony models should not be empty");

    // Verify Sony E-mount compatibility
    let a7iii_model = sony_models
        .iter()
        .find(|m| m.contains("ILCE-7M3") || m.contains("A7 III") || m.contains("a7iii"));
    if let Some(model) = a7iii_model {
        let compats = db.camera_db.get_compatible_cameras("Sony", model);
        println!("Compatible cameras for Sony {}: {:?}", model, compats);
        // Compatible full frame E-mount cameras
        assert!(!compats.is_empty(), "Should find compatible Sony cameras");
    }

    // Verify lenses for camera
    let lenses = db
        .camera_db
        .get_lenses_for_camera("Sony", "ILCE-7M3 (A7 III)");
    assert!(!lenses.is_empty(), "Should return lenses for Sony camera");

    // Verify LensFun profile creation
    let calib = LensCalibration {
        model: "ptlens".to_string(),
        focal: 24.0,
        coeffs: vec![0.01, -0.02, 0.0],
    };
    let lp = CameraDatabase::create_lensfun_profile(
        "Sony",
        "ILCE-7M3",
        "Sony",
        "FE 24mm F1.4 GM",
        &calib,
        1.0,
    );
    assert_eq!(lp.distortion_model.as_deref(), Some("ptlens"));
    assert_eq!(lp.focal_length, Some(24.0));
    assert_eq!(lp.fisheye_params.distortion_coeffs, vec![0.01, -0.02, 0.0]);
}
