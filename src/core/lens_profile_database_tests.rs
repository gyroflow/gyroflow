// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::camera_database::Selection;
use serde_json::{Value, json};
use std::io::Write;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("gyroflow-catalogue-{}-{}", std::process::id(), fastrand::u64(..)));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn bundle(&self, catalogue: Value, profiles: Vec<(String, Value)>) {
        let mut entries = vec![("__version".to_owned(), json!(42)), ("__camera_database".to_owned(), catalogue)];
        entries.extend(profiles);
        let mut data = Vec::new();
        ciborium::into_writer(&entries, &mut data).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&data).unwrap();
        std::fs::write(self.0.join("profiles.cbor.gz"), encoder.finish().unwrap()).unwrap();
    }
    fn load(&self) -> LensProfileDatabase {
        let mut db = LensProfileDatabase::default();
        db.load_directories(self.0.clone(), self.0.join("absent"));
        db.prepare_list_for_ui();
        db
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn catalogue() -> Value {
    json!({"schema_version":1, "mounts":[], "cameras":[
        {"id":"future/camera2","brand":"Future","model":"Camera 2","aliases":["Cam II"],"profile_lenses":["prime"]}
    ], "lenses":[{"id":"prime","brand":"Maker","model":"Prime 50mm"}]})
}
fn profile(id: &str, author: &str, focal: f64) -> Value {
    json!({"camera_brand":"Future","camera_model":"Cam II","lens_model":"Maker Prime 50mm",
        "calibrator_version":"1.6.3", "calibrated_by":author, "identifier":id,
        "fps":30, "calib_dimension":{"w":1920,"h":1080},
        "fisheye_params":{"camera_matrix":[[focal,0,960],[0,focal,540],[0,0,1]],"distortion_coeffs":[0,0,0,0]}})
}
fn selection() -> Selection {
    Selection { camera_brand:"Future".into(), camera_model:"Camera 2".into(), lens_model:"Maker Prime 50mm".into() }
}

#[test]
fn updated_bundle_preserves_duplicate_identifier_submissions_and_clones() {
    let fixture = Fixture::new();
    fixture.bundle(catalogue(), vec![
        ("Future/one.json".into(), profile("same-id", "First", 900.0)),
        ("Future/two.json".into(), profile("same-id", "Second", 950.0)),
    ]);
    let db = fixture.load();
    assert_eq!(db.version, 42);
    assert_eq!(db.map.len(), 2);
    assert_eq!(db.get_by_id("same-id").unwrap().calibrated_by, "First");
    let rows = db.browse(&selection(), false);
    assert_eq!(rows.as_array().unwrap().len(), 2);
    assert!(rows.as_array().unwrap().iter().all(|r| db.get_by_id(r["id"].as_str().unwrap()).is_some()));
    assert_eq!(db.camera_database.cameras.len(), 1); // Updated catalogue replaces the bundled fallback.
    let mut replacement = LensProfileDatabase::default();
    replacement.set_from_db(db.clone());
    replacement.prepare_list_for_ui();
    assert_eq!(replacement.version, 42);
    assert_eq!(replacement.browse(&selection(), false), rows);
    assert_eq!(replacement.camera_database.camera("Future", "Cam II").unwrap().model, "Camera 2");
}

#[test]
fn unsupported_catalogue_does_not_discard_profiles_or_bundled_metadata() {
    let fixture = Fixture::new();
    let mut invalid = catalogue();
    invalid["schema_version"] = json!(999);
    fixture.bundle(invalid, vec![("Future/one.json".into(), profile("one", "Author", 900.0))]);
    let db = fixture.load();
    assert_eq!(db.map.len(), 1);
    assert!(db.camera_database.camera("Sony", "a6000").is_some());
    assert!(db.camera_database.camera("Future", "Cam II").is_some());
}

#[test]
fn catalogue_json_is_not_a_calibration_and_all_review_results_are_returned() {
    let fixture = Fixture::new();
    std::fs::write(fixture.0.join("camera_database.json"), catalogue().to_string()).unwrap();
    std::fs::create_dir_all(fixture.0.join("lensfun/data/db")).unwrap();
    std::fs::write(fixture.0.join("lensfun/data/db/lensfun-database.dtd"), "").unwrap();
    std::fs::write(fixture.0.join("lensfun/fixture.json"), profile("source-fixture", "Not a submitted profile", 900.0).to_string()).unwrap();
    fixture.bundle(catalogue(), (0..205).map(|i| (format!("Future/{i}.json"), profile(&format!("id-{i}"), &format!("Author {i}"), 900.0 + i as f64))).collect());
    let db = fixture.load();
    assert_eq!(db.map.len(), 205);
    assert_eq!(db.browse(&selection(), false).as_array().unwrap().len(), 205);
    let no_match = Selection { lens_model:"Nonexistent lens".into(), ..selection() };
    assert!(db.browse(&no_match, true).as_array().unwrap().is_empty());
}
