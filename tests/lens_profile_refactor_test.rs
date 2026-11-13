// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Gyroflow contributors

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    #[test]
    fn test_camera_database_schema_and_integrity() {
        let db_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/camera_database.json");
        let json_str = fs::to_string(&db_path).expect("Failed to read camera_database.json");
        let value: serde_json::Value = serde_json::from_str(&json_str).expect("Invalid JSON syntax");

        assert!(value.get("version").is_some());
        assert!(value.get("mounts").is_some());
        assert!(value.get("cameras").is_some());
        assert!(value.get("lenses").is_some());

        let cameras = value["cameras"].as_array().expect("cameras should be an array");
        assert!(!cameras.is_empty(), "Camera list must not be empty");

        for cam in cameras {
            assert!(cam["brand"].is_string(), "Missing camera brand");
            assert!(cam["model"].is_string(), "Missing camera model");
            assert!(cam["mount"].is_string(), "Missing camera mount");
            assert!(cam["crop_factor"].is_number(), "Missing camera crop factor");
        }
    }

    #[test]
    fn test_submission_validation_rules() {
        use gyroflow::core::lens_profile::validator::{LensProfileSubmission, ProfileValidator};

        // 1. Should fail when camera model is empty or 'Other'
        let invalid_cam = LensProfileSubmission {
            camera_brand: "Sony".to_string(),
            camera_model: "Other".to_string(),
            lens_brand: "Sony".to_string(),
            lens_model: "24mm f1.4 GM".to_string(),
            focal_length: Some(24.0),
            is_zoom_lens: false,
            sensor_format: Some("Full Frame".to_string()),
            input_resolution: (3840, 2160),
            distortion_coefficients: vec![0.12, -0.04],
            optical_center: (1920.0, 1080.0),
        };
        let res = ProfileValidator::validate(&invalid_cam);
        assert!(!res.is_valid);
        assert!(res.errors.iter().any(|e| e.contains("Other")));

        // 2. Should fail when zoom lens has no focal length specified
        let invalid_zoom = LensProfileSubmission {
            camera_brand: "Sony".to_string(),
            camera_model: "FX3".to_string(),
            lens_brand: "Sony".to_string(),
            lens_model: "24-70mm GM II".to_string(),
            focal_length: None,
            is_zoom_lens: true,
            sensor_format: Some("Full Frame".to_string()),
            input_resolution: (3840, 2160),
            distortion_coefficients: vec![0.12, -0.04],
            optical_center: (1920.0, 1080.0),
        };
        let res = ProfileValidator::validate(&invalid_zoom);
        assert!(!res.is_valid);
        assert!(res.errors.iter().any(|e| e.contains("focal length")));

        // 3. Valid submission should succeed
        let valid = LensProfileSubmission {
            camera_brand: "Sony".to_string(),
            camera_model: "FX3".to_string(),
            lens_brand: "Sony".to_string(),
            lens_model: "FE 24-70mm F2.8 GM II".to_string(),
            focal_length: Some(24.0),
            is_zoom_lens: true,
            sensor_format: Some("Full Frame".to_string()),
            input_resolution: (3840, 2160),
            distortion_coefficients: vec![0.12, -0.04],
            optical_center: (1920.0, 1080.0),
        };
        let res = ProfileValidator::validate(&valid);
        assert!(res.is_valid);
        assert!(res.errors.is_empty());
    }

    #[test]
    fn test_review_manager_cycling() {
        use gyroflow::core::lens_profile::review::{ProfileReviewEntry, ReviewManager};

        let mut mgr = ReviewManager::new();
        let entries = vec![
            ProfileReviewEntry {
                id: "p1".into(),
                profile_name: "Profile 1".into(),
                camera_brand: "Sony".into(),
                camera_model: "FX3".into(),
                lens_brand: "Sony".into(),
                lens_model: "24mm".into(),
                focal_length: Some(24.0),
                aspect_ratio: "16:9".into(),
                fov_deg: 84.0,
                rms_error: 0.25,
                author: "user1".into(),
                timestamp: 100,
                verified: false,
                is_active: false,
            },
            ProfileReviewEntry {
                id: "p2".into(),
                profile_name: "Profile 2".into(),
                camera_brand: "Sony".into(),
                camera_model: "FX3".into(),
                lens_brand: "Sony".into(),
                lens_model: "24mm".into(),
                focal_length: Some(24.0),
                aspect_ratio: "16:9".into(),
                fov_deg: 83.8,
                rms_error: 0.18,
                author: "user2".into(),
                timestamp: 200,
                verified: true,
                is_active: false,
            },
        ];

        mgr.set_candidates(entries);
        assert_eq!(mgr.current_selection().unwrap().id, "p1");

        let next = mgr.select_next().unwrap();
        assert_eq!(next.id, "p2");

        // Cycle wraps around
        let wrap = mgr.select_next().unwrap();
        assert_eq!(wrap.id, "p1");

        // Mark for removal
        let removed = mgr.mark_for_removal("p1");
        assert!(removed);
        assert_eq!(mgr.candidates().len(), 1);
        assert_eq!(mgr.current_selection().unwrap().id, "p2");
    }
}
