// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Gyroflow contributors

use std::path::PathBuf;

use gyroflow_core::{
    camera_registry::CameraRegistry,
    lens_profile_database::LensProfileDatabase,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/camera_registry.json")
        });

    let mut profiles = LensProfileDatabase::default();
    profiles.load_all();

    let registry = CameraRegistry::from_lens_profile_database(&profiles);
    let data = serde_json::to_string_pretty(&registry.to_catalog())?;

    std::fs::write(&output, format!("{data}\n"))?;
    eprintln!("Wrote {}", output.display());

    Ok(())
}
