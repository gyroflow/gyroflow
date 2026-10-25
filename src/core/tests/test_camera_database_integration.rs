// SPDX-License-Identifier: GPL-3.0-or-later

use gyroflow_core::camera_database::CameraDatabase;
use gyroflow_core::lens_profile_database::LensProfileDatabase;
use gyroflow_core::lens_profile::{LensProfile, Dimensions, CameraParams};
use gyroflow_core::camera_identifier::CameraIdentifier;

#[test]
fn test_camera_database_json_parsing() {
    let json_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/camera_database.json");
    let content = std::fs::read_to_string(json_path).expect("camera_database.json should exist");
    let db = CameraDatabase::from_json(&content).expect("camera_database.json should parse");

    assert!(db.mounts.len() >= 10);
    assert!(db.get_brands().len() >= 50);

    // Test Sony normalization
    assert_eq!(db.normalize_brand("sony"), "Sony");
    assert_eq!(db.normalize_brand("SONY"), "Sony");
    assert_eq!(db.normalize_brand("bmpcc"), "Blackmagic");
    assert_eq!(db.normalize_brand("panasonic"), "Panasonic");
    assert_eq!(db.normalize_brand("red"), "RED");

    // Test Model normalization
    assert_eq!(db.normalize_model("Sony", "ILCE-7M3"), "a7 III");
    assert_eq!(db.normalize_model("Sony", "a73"), "a7 III");
    assert_eq!(db.normalize_model("Blackmagic", "bmpcc4k"), "Pocket Cinema Camera 4K");
    assert_eq!(db.normalize_model("GoPro", "hero10"), "HERO10 Black");
}

#[test]
fn test_lens_profile_database_querying() {
    let mut db = LensProfileDatabase::default();
    db.load_all();

    let brands = db.get_brands();
    assert!(!brands.is_empty());
    assert!(brands.contains(&"Sony".to_string()));
    assert!(brands.contains(&"GoPro".to_string()));

    let sony_models = db.get_models("Sony");
    assert!(!sony_models.is_empty());

    let sony_lenses = db.get_lenses("Sony", "a7 III");
    assert!(!sony_lenses.is_empty());

    let found = db.find_profiles(Some("Sony"), None, None);
    assert!(!found.is_empty());
}

#[test]
fn test_lens_profile_database_load_lensfun() {
    let xml = r#"
    <lensdatabase version="1">
        <camera>
            <maker>Sony</maker>
            <model>ILCE-7M3</model>
            <mount>Sony E</mount>
            <cropfactor>1.0</cropfactor>
        </camera>
        <lens>
            <maker>Sony</maker>
            <model>FE 50mm F1.8</model>
            <mount>Sony E</mount>
            <cropfactor>1.0</cropfactor>
            <aspect-ratio>3:2</aspect-ratio>
            <calibration>
                <distortion model="ptlens" focal="50" a="0.002" b="-0.006" c="0.008" />
            </calibration>
        </lens>
    </lensdatabase>
    "#;

    let mut db = LensProfileDatabase::default();
    let count = db.load_from_lensfun(xml).expect("Should load lensfun xml");
    assert_eq!(count, 1);

    let profiles = db.find_profiles(Some("Sony"), None, Some("FE 50mm F1.8"));
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].distortion_model.as_deref(), Some("ptlens"));
    assert_eq!(profiles[0].camera_brand, "Sony");
}

#[test]
fn test_lens_profile_validation() {
    let mut valid_profile = LensProfile::default();
    valid_profile.camera_brand = "Sony".into();
    valid_profile.camera_model = "a7 III".into();
    valid_profile.lens_model = "FE 24-70mm F2.8 GM".into();
    valid_profile.calib_dimension = Dimensions { w: 3840, h: 2160 };
    valid_profile.fisheye_params = CameraParams {
        RMS_error: 0.5,
        camera_matrix: vec![[1000.0, 0.0, 1920.0], [0.0, 1000.0, 1080.0], [0.0, 0.0, 1.0]],
        distortion_coeffs: vec![0.1, 0.01, 0.001],
        radial_distortion_limit: None,
    };

    // Zoom lens without focal length info must be rejected
    assert!(valid_profile.validate_for_submission().is_err());

    // Zoom lens with specific focal length succeeds
    valid_profile.focal_length = Some(24.0);
    assert!(valid_profile.validate_for_submission().is_ok());

    // Vague lens name on mirrorless camera must be rejected
    valid_profile.lens_model = "Standard".into();
    assert!(valid_profile.validate_for_submission().is_err());

    valid_profile.lens_model = "FE 35mm F1.4 GM".into();
    assert!(valid_profile.validate_for_submission().is_ok());

    // Missing brand
    valid_profile.camera_brand = "".into();
    assert!(valid_profile.validate_for_submission().is_err());

    valid_profile.camera_brand = "Unknown".into();
    assert!(valid_profile.validate_for_submission().is_err());

    valid_profile.camera_brand = "Sony".into();
    valid_profile.camera_model = "None".into();
    assert!(valid_profile.validate_for_submission().is_err());

    valid_profile.camera_model = "a7 III".into();
    valid_profile.lens_model = "".into();
    assert!(valid_profile.validate_for_submission().is_err());
}

#[test]
fn test_camera_identifier_normalization() {
    let mut db = CameraDatabase::new();
    db.add_brand_alias("sony", "Sony");
    db.add_model_alias("Sony", "ilce-7m3", "a7 III");

    let mut id = CameraIdentifier {
        brand: "sony".into(),
        model: "ilce-7m3".into(),
        lens_info: "wide".into(),
        video_width: 3840,
        video_height: 2160,
        fps: 60,
        ..Default::default()
    };

    id.normalize_with_db(&db);
    assert_eq!(id.brand, "Sony");
    assert_eq!(id.model, "a7 III");
}
