// SPDX-License-Identifier: GPL-3.0-or-later

// Run after building the two assets in lens_profiles. Use the same release number as the packer:
// GYROFLOW_CAMERA_DATABASE=/path/to/camera_database.json GYROFLOW_PROFILES_BUNDLE=/path/to/profiles.cbor.gz \
// GYROFLOW_PROFILE_RELEASE_VERSION=42 cargo test --manifest-path src/core/Cargo.toml --test producer_artifacts -- --ignored

use std::{env, error::Error, fs, path::PathBuf};
use gyroflow_core::database_update::{install, DatabaseFormat};

#[test]
#[ignore = "uses artifacts produced by lens_profiles"]
fn producer_artifacts_install_through_native_consumer() -> Result<(), Box<dyn Error>> {
    let catalog = PathBuf::from(env::var("GYROFLOW_CAMERA_DATABASE")?);
    let bundle = PathBuf::from(env::var("GYROFLOW_PROFILES_BUNDLE")?);
    let version: u32 = env::var("GYROFLOW_PROFILE_RELEASE_VERSION")?.parse()?;

    let dir = tempfile::tempdir()?;
    let cameras = dir.path().join("camera_database.json");
    let profiles = dir.path().join("profiles.cbor.gz");
    install(fs::File::open(&catalog)?, &cameras, DatabaseFormat::Cameras, None)?;
    install(fs::File::open(&bundle)?, &profiles, DatabaseFormat::Profiles(version), None)?;
    assert!(fs::read(cameras)? == fs::read(catalog)?, "catalog changed during installation");

    // A stale release manifest must leave the usable profile bundle in place.
    assert!(install(fs::File::open(&bundle)?, &profiles, DatabaseFormat::Profiles(version ^ 1), None).is_err());
    assert!(fs::read(profiles)? == fs::read(bundle)?, "bundle changed after a rejected update");
    assert_eq!(fs::read_dir(dir.path())?.count(), 2);
    Ok(())
}
