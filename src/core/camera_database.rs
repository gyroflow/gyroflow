// SPDX-License-Identifier: GPL-3.0-or-later
// Camera and lens catalog loader and validator

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CameraDatabase {
    pub schema_version: String,
    pub generated_at: Option<String>,
    pub sources: Vec<String>,
    pub mounts: Vec<String>,
    pub brands: BTreeMap<String, Brand>,
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Brand {
    pub cameras: Vec<Camera>,
    pub lens_models: Vec<String>,
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Camera {
    pub model: String,
    pub mount: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

impl CameraDatabase {
    const EMBEDDED_BYTES: &'static [u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../resources/camera_database.json"
    ));

    /// Load the database from disk, falling back to the embedded copy.
    /// The lookup order is user data dir (to allow auto-updates), cwd resources, then embedded.
    pub fn load() -> Self {
        for path in Self::candidate_paths() {
            if let Ok(bytes) = std::fs::read(&path) {
                if let Ok(db) = Self::from_slice(&bytes) {
                    log::info!("Loaded camera database from {}", path.display());
                    return db;
                } else {
                    log::warn!("Failed to parse camera database at {}", path.display());
                }
            }
        }
        Self::from_slice(Self::EMBEDDED_BYTES).unwrap_or_else(|e| {
            log::error!("Falling back to empty camera database: {}", e);
            Self::default()
        })
    }

    pub fn from_slice(data: &[u8]) -> Result<Self, String> {
        serde_json::from_slice::<CameraDatabase>(data).map_err(|e| e.to_string()).and_then(|db| {
            db.validate().map(|_| db)
        })
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| e.to_string())
    }

    pub fn candidate_paths() -> Vec<PathBuf> {
        let mut v = Vec::new();
        let data_dir = crate::settings::data_dir();
        v.push(data_dir.join("camera_database.json"));
        v.push(data_dir.join("camera_presets").join("camera_database.json"));
        v.push(PathBuf::from("./resources/camera_database.json"));
        v
    }

    /// Simple list for UI/CLI consumption: (brand, model, mount, aliases, lenses)
    pub fn list_entries(&self) -> Vec<(String, String, String, Vec<String>, Vec<String>)> {
        let mut out = Vec::new();
        for (brand, b) in &self.brands {
            for cam in &b.cameras {
                out.push((brand.clone(), cam.model.clone(), cam.mount.clone(), cam.aliases.clone(), b.lens_models.clone()));
            }
        }
        out
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version.trim().is_empty() {
            return Err("schema_version must not be empty".into());
        }

        let mut mount_set = HashSet::new();
        for m in &self.mounts {
            if !mount_set.insert(m.to_ascii_lowercase()) {
                return Err(format!("duplicate mount entry: {}", m));
            }
        }

        for (brand, data) in &self.brands {
            if brand.trim().is_empty() {
                return Err("brand name must not be empty".into());
            }
            for cam in &data.cameras {
                if cam.model.trim().is_empty() {
                    return Err(format!("camera model missing for brand {}", brand));
                }
                if cam.mount.trim().is_empty() {
                    return Err(format!("mount missing for brand {} model {}", brand, cam.model));
                }
                if !mount_set.contains(&cam.mount.to_ascii_lowercase()) {
                    return Err(format!("mount {} not declared in mounts list (brand {}, model {})", cam.mount, brand, cam.model));
                }
                let mut alias_seen = HashSet::new();
                for a in &cam.aliases {
                    if !alias_seen.insert(a.to_ascii_lowercase()) {
                        return Err(format!("duplicate alias {} for brand {} model {}", a, brand, cam.model));
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_embedded_and_has_required_mounts() {
        let db = CameraDatabase::from_slice(CameraDatabase::EMBEDDED_BYTES).expect("embedded db should parse");
        assert!(db.mounts.contains(&"Fixed".to_string()));
        assert!(!db.brands.is_empty());
    }
}
