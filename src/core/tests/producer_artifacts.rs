// SPDX-License-Identifier: GPL-3.0-or-later
//
// Producer/consumer contract check for the lens_profiles release artifacts.
//
// After generating camera_database.json and profiles.cbor.gz, run:
//
// GYROFLOW_CAMERA_DATABASE=/path/to/camera_database.json \
// GYROFLOW_PROFILES_BUNDLE=/path/to/profiles.cbor.gz \
// GYROFLOW_PROFILE_RELEASE_VERSION=<release-tag-number> \
// cargo test --manifest-path src/core/Cargo.toml --test producer_artifacts \
//   -- --ignored --exact producer_artifacts_install_through_native_consumer

use std::fs;
use std::path::Path;

use gyroflow_core::camera_database::CameraDatabase;
use gyroflow_core::database_update::{install, DatabaseFormat};

fn required_path(name: &str) -> std::path::PathBuf {
    std::env::var_os(name)
        .map(Into::into)
        .unwrap_or_else(|| panic!("set {name} to the producer artifact path"))
}

fn install_artifact(source: &Path, destination: &Path, format: DatabaseFormat) {
    let file = fs::File::open(source)
        .unwrap_or_else(|error| panic!("could not open {}: {error}", source.display()));
    install(file, destination, format)
        .unwrap_or_else(|error| panic!("could not install {}: {error}", source.display()));
}

#[test]
#[ignore = "requires generated camera database and profiles release artifact paths"]
fn producer_artifacts_install_through_native_consumer() {
    let camera_source = required_path("GYROFLOW_CAMERA_DATABASE");
    let profiles_source = required_path("GYROFLOW_PROFILES_BUNDLE");
    let release_version: u32 = std::env::var("GYROFLOW_PROFILE_RELEASE_VERSION")
        .expect("set GYROFLOW_PROFILE_RELEASE_VERSION to the release tag number")
        .parse()
        .expect("GYROFLOW_PROFILE_RELEASE_VERSION must be an unsigned integer");

    let camera_contents = fs::read_to_string(&camera_source).expect("read camera database artifact");
    CameraDatabase::parse_file(&camera_contents).expect("producer camera database parses in the app");

    let temp = tempfile::tempdir().expect("create isolated artifact install directory");
    let camera_destination = temp.path().join("lens_profiles/camera_database.json");
    let profiles_destination = temp.path().join("lens_profiles/profiles.cbor.gz");

    fs::create_dir_all(camera_destination.parent().unwrap()).expect("create install directory");
    fs::write(&camera_destination, b"previous camera database").expect("seed existing camera database");
    install_artifact(&camera_source, &camera_destination, DatabaseFormat::Cameras);
    let installed_camera = fs::read_to_string(&camera_destination).expect("read installed camera database");
    CameraDatabase::parse_file(&installed_camera).expect("installed camera database parses in the app");
    assert_eq!(installed_camera, camera_contents);

    fs::write(&profiles_destination, b"previous profile database").expect("seed existing profile database");
    install_artifact(&profiles_source, &profiles_destination, DatabaseFormat::Profiles(release_version));
    let installed_profiles = fs::read(&profiles_destination).expect("read installed profile bundle");
    assert_eq!(installed_profiles, fs::read(&profiles_source).expect("read source profile bundle"));

    let wrong_version = release_version.checked_add(1).expect("release version can be incremented");
    let wrong_release = fs::File::open(&profiles_source).expect("reopen profile bundle");
    assert!(install(wrong_release, &profiles_destination, DatabaseFormat::Profiles(wrong_version)).is_err());
    assert_eq!(
        fs::read(&profiles_destination).expect("previous profile database remains installed"),
        installed_profiles,
        "a bundle with a mismatched release version must not replace the installed database"
    );
    assert_eq!(fs::read_dir(temp.path().join("lens_profiles")).unwrap().count(), 2,
        "failed installation leaves no temporary files");
}
