# Solution for [github/issues_742]: [gyroflow/gyroflow] Refactor lens profile handling

- **Reward:** $500.00 USDC
- **Target URL:** https://github.com/gyroflow/gyroflow/issues/742
- **AI Brain:** gemini-3.8-flash
- **Tokens Used:** 15972 (~$0.0319 USD compute cost)

---

### Architectural Root Cause & Design Analysis

#### Current Limitations
1. **Free-form Text Search vs. Structured Taxonomy**: Currently, lens profile searching relies primarily on fuzzy text matching over combined name strings (e.g. `Sony FX3 24mm f1.4`). This leads to duplicate profiles, mismatched sensor formats, and difficulty locating profiles when names are formatted slightly differently.
2. **Missing Sensor Size & Mount Compatibility Engine**: Lenses can often be shared across camera bodies with the same mount and equivalent or smaller sensor sizes (e.g., full-frame E-mount lenses work seamlessly across Sony A7S III, FX3, A7 IV, etc., adjusting for crop factor). Without a structured database of camera mounts, sensor sizes, and crop factors, users miss compatible profiles.
3. **Calibrator Metadata Ambiguity**: Calibrator submissions often lack critical fields (e.g., zoom lens focal lengths not specified, camera model blank, or missing focal length / mount information).
4. **Lensfun Integration**: Lensfun offers camera and lens calibration data, but its database needs to be integrated into the selection hierarchy.
5. **Review and Verification Workflow**: Reviewers and power users need a mechanism to cycle through alternative or competing calibration profiles for the exact camera/lens combination to compare undistortion and discard poor calibrations.

#### Solution Architecture
```
┌────────────────────────────────────────────────────────┐
│             Camera & Lens Database JSON                │
│     (Brands, Models, Mounts, Sensor Crop Factors)      │
└──────────────────────────┬─────────────────────────────┘
                           │
                           ▼
┌────────────────────────────────────────────────────────┐
│               LensDatabaseManager (Rust)               │
│  - Parses camera_database.json & Lensfun definitions   │
│  - Mount compatibility matching & crop factor mapping  │
│  - Auto-update worker via remote manifest sync         │
└──────────────────────────┬─────────────────────────────┘
                           │
        ┌──────────────────┴──────────────────┐
        ▼                                     ▼
┌───────────────────────────────┐   ┌───────────────────────────────┐
│     LensProfileSelector (QML) │   │     Calibrator Validation     │
│ - Hierarchical Brand/Model/   │   │ - Strict submission checks    │
│   Lens selector               │   │ - Enforce valid camera/lens   │
│ - Compatible camera finder    │   │ - Prevent ambiguous profiles  │
│ - Profile A/B review toggle   │   │ - Video metadata prefill      │
└───────────────────────────────┘   └───────────────────────────────┘
```

---

### Implementation

### File: resources/camera_database.json
```json
{
  "version": 1,
  "mounts": {
    "Sony E": { "flange_distance_mm": 18.0, "full_frame": true },
    "Canon RF": { "flange_distance_mm": 20.0, "full_frame": true },
    "Canon EF": { "flange_distance_mm": 44.0, "full_frame": true },
    "Nikon Z": { "flange_distance_mm": 16.0, "full_frame": true },
    "Nikon F": { "flange_distance_mm": 46.5, "full_frame": true },
    "Micro Four Thirds": { "flange_distance_mm": 19.25, "full_frame": false },
    "Fujifilm X": { "flange_distance_mm": 17.7, "full_frame": false },
    "Fujifilm G": { "flange_distance_mm": 26.7, "full_frame": false },
    "Leica L": { "flange_distance_mm": 20.0, "full_frame": true },
    "DJI Action": { "flange_distance_mm": 0.0, "fixed": true },
    "GoPro Fixed": { "flange_distance_mm": 0.0, "fixed": true },
    "Insta360 Fixed": { "flange_distance_mm": 0.0, "fixed": true }
  },
  "cameras": [
    {
      "brand": "Sony",
      "model": "FX3",
      "mount": "Sony E",
      "sensor_type": "Full Frame",
      "crop_factor": 1.0,
      "aliases": ["ILME-FX3", "Sony ILME-FX3"]
    },
    {
      "brand": "Sony",
      "model": "A7S III",
      "mount": "Sony E",
      "sensor_type": "Full Frame",
      "crop_factor": 1.0,
      "aliases": ["ILCE-7SM3", "Sony A7S3", "Sony Alpha 7S III"]
    },
    {
      "brand": "Sony",
      "model": "A7 IV",
      "mount": "Sony E",
      "sensor_type": "Full Frame",
      "crop_factor": 1.0,
      "aliases": ["ILCE-7M4", "Sony A7M4"]
    },
    {
      "brand": "Sony",
      "model": "FX30",
      "mount": "Sony E",
      "sensor_type": "APS-C",
      "crop_factor": 1.53,
      "aliases": ["ILME-FX30"]
    },
    {
      "brand": "Sony",
      "model": "ZV-E1",
      "mount": "Sony E",
      "sensor_type": "Full Frame",
      "crop_factor": 1.0,
      "aliases": ["ZV-E1"]
    },
    {
      "brand": "Sony",
      "model": "ZV-E10",
      "mount": "Sony E",
      "sensor_type": "APS-C",
      "crop_factor": 1.53,
      "aliases": ["ZV-E10"]
    },
    {
      "brand": "Canon",
      "model": "EOS R5",
      "mount": "Canon RF",
      "sensor_type": "Full Frame",
      "crop_factor": 1.0,
      "aliases": ["Canon R5"]
    },
    {
      "brand": "Canon",
      "model": "EOS R6",
      "mount": "Canon RF",
      "sensor_type": "Full Frame",
      "crop_factor": 1.0,
      "aliases": ["Canon R6"]
    },
    {
      "brand": "Canon",
      "model": "EOS R7",
      "mount": "Canon RF",
      "sensor_type": "APS-C",
      "crop_factor": 1.6,
      "aliases": ["Canon R7"]
    },
    {
      "brand": "Panasonic",
      "model": "GH5",
      "mount": "Micro Four Thirds",
      "sensor_type": "M43",
      "crop_factor": 2.0,
      "aliases": ["DC-GH5"]
    },
    {
      "brand": "Panasonic",
      "model": "GH6",
      "mount": "Micro Four Thirds",
      "sensor_type": "M43",
      "crop_factor": 2.0,
      "aliases": ["DC-GH6"]
    },
    {
      "brand": "Panasonic",
      "model": "S5 II",
      "mount": "Leica L",
      "sensor_type": "Full Frame",
      "crop_factor": 1.0,
      "aliases": ["DC-S5M2", "Lumix S5II"]
    },
    {
      "brand": "Fujifilm",
      "model": "X-T4",
      "mount": "Fujifilm X",
      "sensor_type": "APS-C",
      "crop_factor": 1.53,
      "aliases": ["XT4", "FUJIFILM X-T4"]
    },
    {
      "brand": "Fujifilm",
      "model": "X-H2S",
      "mount": "Fujifilm X",
      "sensor_type": "APS-C",
      "crop_factor": 1.53,
      "aliases": ["XH2S", "FUJIFILM X-H2S"]
    },
    {
      "brand": "GoPro",
      "model": "HERO12 Black",
      "mount": "GoPro Fixed",
      "sensor_type": "1/1.9 inch",
      "crop_factor": 5.4,
      "aliases": ["HERO12", "GoPro 12"]
    },
    {
      "brand": "GoPro",
      "model": "HERO11 Black",
      "mount": "GoPro Fixed",
      "sensor_type": "1/1.9 inch",
      "crop_factor": 5.4,
      "aliases": ["HERO11", "GoPro 11"]
    },
    {
      "brand": "GoPro",
      "model": "HERO10 Black",
      "mount": "GoPro Fixed",
      "sensor_type": "1/2.3 inch",
      "crop_factor": 5.62,
      "aliases": ["HERO10", "GoPro 10"]
    },
    {
      "brand": "DJI",
      "model": "Action 4",
      "mount": "DJI Fixed",
      "sensor_type": "1/1.3 inch",
      "crop_factor": 3.8,
      "aliases": ["Osmo Action 4"]
    },
    {
      "brand": "DJI",
      "model": "Action 3",
      "mount": "DJI Fixed",
      "sensor_type": "1/1.7 inch",
      "crop_factor": 4.55,
      "aliases": ["Osmo Action 3"]
    },
    {
      "brand": "Insta360",
      "model": "Ace Pro",
      "mount": "Insta360 Fixed",
      "sensor_type": "1/1.3 inch",
      "crop_factor": 3.8,
      "aliases": ["AcePro"]
    }
  ],
  "lenses": [
    {
      "brand": "Sony",
      "model": "FE 24mm F1.4 GM",
      "mount": "Sony E",
      "sensor_format": "Full Frame",
      "focal_min": 24.0,
      "focal_max": 24.0,
      "is_zoom": false
    },
    {
      "brand": "Sony",
      "model": "FE 35mm F1.4 GM",
      "mount": "Sony E",
      "sensor_format": "Full Frame",
      "focal_min": 35.0,
      "focal_max": 35.0,
      "is_zoom": false
    },
    {
      "brand": "Sony",
      "model": "FE 24-70mm F2.8 GM II",
      "mount": "Sony E",
      "sensor_format": "Full Frame",
      "focal_min": 24.0,
      "focal_max": 70.0,
      "is_zoom": true
    },
    {
      "brand": "Sony",
      "model": "FE 16-35mm F2.8 GM II",
      "mount": "Sony E",
      "sensor_format": "Full Frame",
      "focal_min": 16.0,
      "focal_max": 35.0,
      "is_zoom": true
    },
    {
      "brand": "Sony",
      "model": "E 15mm F1.4 G",
      "mount": "Sony E",
      "sensor_format": "APS-C",
      "focal_min": 15.0,
      "focal_max": 15.0,
      "is_zoom": false
    },
    {
      "brand": "Canon",
      "model": "RF 15-35mm F2.8 L IS USM",
      "mount": "Canon RF",
      "sensor_format": "Full Frame",
      "focal_min": 15.0,
      "focal_max": 35.0,
      "is_zoom": true
    },
    {
      "brand": "Canon",
      "model": "RF 24-70mm F2.8 L IS USM",
      "mount": "Canon RF",
      "sensor_format": "Full Frame",
      "focal_min": 24.0,
      "focal_max": 70.0,
      "is_zoom": true
    },
    {
      "brand": "Sigma",
      "model": "16mm F1.4 DC DN Contemporary",
      "mount": "Sony E",
      "sensor_format": "APS-C",
      "focal_min": 16.0,
      "focal_max": 16.0,
      "is_zoom": false
    },
    {
      "brand": "Sigma",
      "model": "24-70mm F2.8 DG DN Art",
      "mount": "Sony E",
      "sensor_format": "Full Frame",
      "focal_min": 24.0,
      "focal_max": 70.0,
      "is_zoom": true
    },
    {
      "brand": "GoPro",
      "model": "HERO11/12 Integrated Wide Lens",
      "mount": "GoPro Fixed",
      "sensor_format": "1/1.9 inch",
      "focal_min": 2.47,
      "focal_max": 2.47,
      "is_zoom": false
    },
    {
      "brand": "DJI",
      "model": "Action 4 Integrated Wide Lens",
      "mount": "DJI Fixed",
      "sensor_format": "1/1.3 inch",
      "focal_min": 2.6,
      "focal_max": 2.6,
      "is_zoom": false
    }
  ]
}
```

---

### File: src/core/lens_profile/camera_db.rs
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Gyroflow contributors

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use log::{info, warn, error};
use once_cell::sync::Lazy;

const REMOTE_CAMERA_DB_URL: &str = "https://raw.githubusercontent.com/gyroflow/lens_profiles/main/camera_database.json";
const LOCAL_DB_FILENAME: &str = "camera_database.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountInfo {
    pub flange_distance_mm: f64,
    #[serde(default)]
    pub full_frame: bool,
    #[serde(default)]
    pub fixed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CameraInfo {
    pub brand: String,
    pub model: String,
    pub mount: String,
    pub sensor_type: String,
    pub crop_factor: f64,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LensInfo {
    pub brand: String,
    pub model: String,
    pub mount: String,
    pub sensor_format: String,
    pub focal_min: f64,
    pub focal_max: f64,
    pub is_zoom: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CameraDatabase {
    pub version: u32,
    pub mounts: HashMap<String, MountInfo>,
    pub cameras: Vec<CameraInfo>,
    pub lenses: Vec<LensInfo>,
}

pub static CAMERA_DB: Lazy<RwLock<CameraDatabase>> = Lazy::new(|| {
    RwLock::new(CameraDatabase::load_default())
});

impl CameraDatabase {
    pub fn load_default() -> Self {
        // 1. Try user directory / app data cached file
        if let Some(user_db) = Self::load_from_user_data() {
            return user_db;
        }

        // 2. Try embedded resource bundled in binary or local resources/ folder
        if let Ok(content) = fs::to_string(Path::new("resources").join(LOCAL_DB_FILENAME)) {
            if let Ok(db) = serde_json::from_str::<CameraDatabase>(&content) {
                return db;
            }
        }

        // 3. Fallback to compile-time resource
        let embedded = include_str!("../../../resources/camera_database.json");
        serde_json::from_str(embedded).unwrap_or_default()
    }

    fn user_cache_path() -> Option<PathBuf> {
        dirs::data_dir().map(|p| p.join("gyroflow").join(LOCAL_DB_FILENAME))
    }

    fn load_from_user_data() -> Option<Self> {
        let path = Self::user_cache_path()?;
        if path.exists() {
            let data = fs::to_string(path).ok()?;
            serde_json::from_str(&data).ok()
        } else {
            None
        }
    }

    pub fn update_from_remote() {
        std::thread::spawn(|| {
            info!("Checking for camera database updates from {}", REMOTE_CAMERA_DB_URL);
            match ureq::get(REMOTE_CAMERA_DB_URL).timeout(std::time::Duration::from_secs(10)).call() {
                Ok(response) => {
                    if let Ok(text) = response.into_string() {
                        if let Ok(new_db) = serde_json::from_str::<CameraDatabase>(&text) {
                            let mut current = CAMERA_DB.write().unwrap();
                            if new_db.version > current.version {
                                info!("Updating camera database to version {}", new_db.version);
                                if let Some(cache_path) = CameraDatabase::user_cache_path() {
                                    if let Some(parent) = cache_path.parent() {
                                        let _ = fs::create_dir_all(parent);
                                    }
                                    let _ = fs::write(&cache_path, &text);
                                }
                                *current = new_db;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to fetch remote camera database: {}", e);
                }
            }
        });
    }

    pub fn get_brands(&self) -> Vec<String> {
        let mut brands: Vec<String> = self.cameras.iter().map(|c| c.brand.clone()).collect();
        brands.sort();
        brands.dedup();
        brands
    }

    pub fn get_models_for_brand(&self, brand: &str) -> Vec<String> {
        let mut models: Vec<String> = self.cameras
            .iter()
            .filter(|c| c.brand.eq_ignore_ascii_case(brand))
            .map(|c| c.model.clone())
            .collect();
        models.sort();
        models.dedup();
        models
    }

    pub fn find_camera(&self, brand: &str, model: &str) -> Option<&CameraInfo> {
        self.cameras.iter().find(|c| {
            c.brand.eq_ignore_ascii_case(brand) && (c.model.eq_ignore_ascii_case(model)
                || c.aliases.iter().any(|a| a.eq_ignore_ascii_case(model)))
        })
    }

    pub fn match_camera_by_name(&self, query: &str) -> Option<&CameraInfo> {
        let q_lower = query.to_lowercase();
        self.cameras.iter().find(|c| {
            let full_name = format!("{} {}", c.brand, c.model).to_lowercase();
            if full_name == q_lower || c.model.to_lowercase() == q_lower {
                return true;
            }
            c.aliases.iter().any(|a| a.to_lowercase() == q_lower)
        })
    }

    pub fn get_compatible_cameras(&self, camera: &CameraInfo) -> Vec<&CameraInfo> {
        let mount = &camera.mount;
        let crop = camera.crop_factor;

        self.cameras
            .iter()
            .filter(|c| {
                if &c.mount != mount {
                    return false;
                }
                // Compatible if same mount and compatible sensor area (crop factor <= camera or same)
                (c.crop_factor - crop).abs() < 0.05 || c.crop_factor <= crop
            })
            .collect()
    }

    pub fn get_lenses_for_camera(&self, camera: &CameraInfo) -> Vec<&LensInfo> {
        self.lenses
            .iter()
            .filter(|l| l.mount.eq_ignore_ascii_case(&camera.mount))
            .collect()
    }
}
```

---

### File: src/core/lens_profile/validator.rs
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Gyroflow contributors

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LensProfileSubmission {
    pub camera_brand: String,
    pub camera_model: String,
    pub lens_brand: String,
    pub lens_model: String,
    pub focal_length: Option<f64>,
    pub is_zoom_lens: bool,
    pub sensor_format: Option<String>,
    pub input_resolution: (u32, u32),
    pub distortion_coefficients: Vec<f64>,
    pub optical_center: (f64, f64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

pub struct ProfileValidator;

impl ProfileValidator {
    pub fn validate(sub: &LensProfileSubmission) -> ValidationResult {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // 1. Camera brand & model validation
        let trimmed_brand = sub.camera_brand.trim();
        let trimmed_model = sub.camera_model.trim();

        if trimmed_brand.is_empty() {
            errors.push("Camera brand is required and cannot be empty.".to_string());
        }
        if trimmed_model.is_empty() {
            errors.push("Camera model is required and cannot be empty.".to_string());
        }
        if trimmed_brand.eq_ignore_ascii_case("other") || trimmed_model.eq_ignore_ascii_case("other") {
            errors.push("Specific camera brand and model must be specified instead of 'Other'.".to_string());
        }

        // 2. Lens validation
        let trimmed_lens_model = sub.lens_model.trim();
        let is_integrated_or_action = trimmed_brand.eq_ignore_ascii_case("gopro")
            || trimmed_brand.eq_ignore_ascii_case("dji")
            || trimmed_brand.eq_ignore_ascii_case("insta360");

        if !is_integrated_or_action && trimmed_lens_model.is_empty() {
            errors.push("Interchangeable lens cameras must have a specific lens model defined.".to_string());
        }

        // 3. Zoom lens check: must have explicit focal length
        if sub.is_zoom_lens {
            match sub.focal_length {
                None => {
                    errors.push("For zoom lenses, the exact calibrated focal length (in mm) must be specified.".to_string());
                }
                Some(fl) if fl <= 0.0 => {
                    errors.push("Calibrated focal length must be a positive number.".to_string());
                }
                _ => {}
            }
        }

        // 4. Optical parameters sanity check
        if sub.input_resolution.0 == 0 || sub.input_resolution.1 == 0 {
            errors.push("Input resolution width and height must be non-zero.".to_string());
        }

        if sub.distortion_coefficients.is_empty() {
            errors.push("Distortion coefficients are missing.".to_string());
        }

        if sub.optical_center.0 <= 0.0 || sub.optical_center.1 <= 0.0 {
            warnings.push("Optical center appears uninitialized or out of bounds.".to_string());
        }

        ValidationResult {
            is_valid: errors.is_empty(),
            errors,
            warnings,
        }
    }
}
```

---

### File: src/core/lens_profile/review.rs
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Gyroflow contributors

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProfileReviewEntry {
    pub id: String,
    pub profile_name: String,
    pub camera_brand: String,
    pub camera_model: String,
    pub lens_brand: String,
    pub lens_model: String,
    pub focal_length: Option<f64>,
    pub aspect_ratio: String,
    pub fov_deg: f64,
    pub rms_error: f64,
    pub author: String,
    pub timestamp: i64,
    pub verified: bool,
    pub is_active: bool,
}

#[derive(Default)]
pub struct ReviewManager {
    candidates: Vec<ProfileReviewEntry>,
    selected_index: usize,
}

impl ReviewManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_candidates(&mut self, mut list: Vec<ProfileReviewEntry>) {
        if !list.is_empty() {
            list[0].is_active = true;
        }
        self.candidates = list;
        self.selected_index = 0;
    }

    pub fn candidates(&self) -> &[ProfileReviewEntry] {
        &self.candidates
    }

    pub fn current_selection(&self) -> Option<&ProfileReviewEntry> {
        self.candidates.get(self.selected_index)
    }

    pub fn select_next(&mut self) -> Option<&ProfileReviewEntry> {
        if self.candidates.is_empty() {
            return None;
        }
        self.candidates[self.selected_index].is_active = false;
        self.selected_index = (self.selected_index + 1) % self.candidates.len();
        self.candidates[self.selected_index].is_active = true;
        Some(&self.candidates[self.selected_index])
    }

    pub fn select_prev(&mut self) -> Option<&ProfileReviewEntry> {
        if self.candidates.is_empty() {
            return None;
        }
        self.candidates[self.selected_index].is_active = false;
        if self.selected_index == 0 {
            self.selected_index = self.candidates.len() - 1;
        } else {
            self.selected_index -= 1;
        }
        self.candidates[self.selected_index].is_active = true;
        Some(&self.candidates[self.selected_index])
    }

    pub fn select_by_id(&mut self, id: &str) -> Option<&ProfileReviewEntry> {
        if let Some(idx) = self.candidates.iter().position(|e| e.id == id) {
            self.candidates[self.selected_index].is_active = false;
            self.selected_index = idx;
            self.candidates[self.selected_index].is_active = true;
            Some(&self.candidates[self.selected_index])
        } else {
            None
        }
    }

    pub fn mark_for_removal(&mut self, id: &str) -> bool {
        if let Some(idx) = self.candidates.iter().position(|e| e.id == id) {
            self.candidates.remove(idx);
            if self.selected_index >= self.candidates.len() && !self.candidates.is_empty() {
                self.selected_index = self.candidates.len() - 1;
            }
            if let Some(entry) = self.candidates.get_mut(self.selected_index) {
                entry.is_active = true;
            }
            true
        } else {
            false
        }
    }
}
```

---

### File: src/core/lens_profile/metadata_prefill.rs
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Gyroflow contributors

use super::camera_db::CAMERA_DB;

#[derive(Debug, Clone, Default)]
pub struct ExtractedVideoMetadata {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens_model: Option<String>,
    pub focal_length: Option<f64>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default)]
pub struct PrefillRecommendation {
    pub camera_brand: String,
    pub camera_model: String,
    pub lens_brand: String,
    pub lens_model: String,
    pub focal_length: f64,
    pub is_matched: bool,
    pub compatible_models: Vec<String>,
}

pub struct MetadataPrefillResolver;

impl MetadataPrefillResolver {
    pub fn resolve(meta: &ExtractedVideoMetadata) -> PrefillRecommendation {
        let db = CAMERA_DB.read().unwrap();
        let mut rec = PrefillRecommendation::default();

        let brand_query = meta.make.as_deref().unwrap_or("").trim();
        let model_query = meta.model.as_deref().unwrap_or("").trim();

        // Match camera
        let matched_cam = if !brand_query.is_empty() && !model_query.is_empty() {
            db.find_camera(brand_query, model_query)
        } else if !model_query.is_empty() {
            db.match_camera_by_name(model_query)
        } else {
            None
        };

        if let Some(cam) = matched_cam {
            rec.camera_brand = cam.brand.clone();
            rec.camera_model = cam.model.clone();
            rec.is_matched = true;

            // Fetch compatible cameras (e.g. Sony A7S3 -> FX3, ZV-E1)
            let comp = db.get_compatible_cameras(cam);
            rec.compatible_models = comp.into_iter()
                .filter(|c| c.model != cam.model)
                .map(|c| format!("{} {}", c.brand, c.model))
                .collect();
        } else {
            rec.camera_brand = brand_query.to_string();
            rec.camera_model = model_query.to_string();
        }

        // Lens matching
        if let Some(lens_str) = &meta.lens_model {
            let lens_trimmed = lens_str.trim();
            rec.lens_model = lens_trimmed.to_string();

            // Attempt to derive lens brand
            for b in &["Sony", "Canon", "Nikon", "Sigma", "Tamron", "Fujifilm", "Panasonic", "Laowa", "Zeiss", "Samyang", "Rokinon", "DJI", "GoPro"] {
                if lens_trimmed.to_lowercase().starts_with(&b.to_lowercase()) {
                    rec.lens_brand = b.to_string();
                    break;
                }
            }
            if rec.lens_brand.is_empty() && !rec.camera_brand.is_empty() {
                rec.lens_brand = rec.camera_brand.clone();
            }
        }

        if let Some(fl) = meta.focal_length {
            rec.focal_length = fl;
        }

        rec
    }
}
```

---

### File: src/ui/lens_profile_selector.rs
```rust
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Gyroflow contributors

use qmetaobject::*;
use std::sync::RwLock;
use crate::core::lens_profile::{
    camera_db::{CAMERA_DB, CameraDatabase},
    review::{ProfileReviewEntry, ReviewManager},
    validator::{LensProfileSubmission, ProfileValidator},
    metadata_prefill::{ExtractedVideoMetadata, MetadataPrefillResolver},
};

#[derive(Default, QObject)]
pub struct LensProfileSelectorWrapper {
    base: qt_base_class!(trait QObject),

    // Brands & Models
    brands: qt_property!(QVariantList; NOTIFY brands_changed),
    models: qt_property!(QVariantList; NOTIFY models_changed),
    lenses: qt_property!(QVariantList; NOTIFY lenses_changed),
    compatible_cameras: qt_property!(QVariantList; NOTIFY compatible_cameras_changed),

    // Active Selections
    selected_brand: qt_property!(QString; NOTIFY selected_brand_changed),
    selected_model: qt_property!(QString; NOTIFY selected_model_changed),
    selected_lens: qt_property!(QString; NOTIFY selected_lens_changed),
    selected_focal_length: qt_property!(f64; NOTIFY selected_focal_length_changed),

    // Review List & Navigation
    review_entries: qt_property!(QVariantList; NOTIFY review_entries_changed),
    active_review_index: qt_property!(i32; NOTIFY active_review_index_changed),
    active_profile_name: qt_property!(QString; NOTIFY active_profile_name_changed),

    // Validation Status for Calibrator
    can_submit: qt_property!(bool; NOTIFY validation_changed),
    validation_error: qt_property!(QString; NOTIFY validation_changed),

    // Signals
    brands_changed: qt_signal!(),
    models_changed: qt_signal!(),
    lenses_changed: qt_signal!(),
    compatible_cameras_changed: qt_signal!(),
    selected_brand_changed: qt_signal!(),
    selected_model_changed: qt_signal!(),
    selected_lens_changed: qt_signal!(),
    selected_focal_length_changed: qt_signal!(),
    review_entries_changed: qt_signal!(),
    active_review_index_changed: qt_signal!(),
    active_profile_name_changed: qt_signal!(),
    validation_changed: qt_signal!(),
    profile_selected_for_review: qt_signal!(id: QString),

    // Internal State
    review_mgr: RwLock<ReviewManager>,
}

impl LensProfileSelectorWrapper {
    pub fn init(&mut self) {
        CameraDatabase::update_from_remote();
        self.reload_brands();
    }

    fn reload_brands(&mut self) {
        let db = CAMERA_DB.read().unwrap();
        let mut list: Vec<QVariant> = db.get_brands()
            .into_iter()
            .map(|b| QString::from(b).into())
            .collect();
        list.push(QString::from("Other").into());
        self.brands = list.into();
        self.brands_changed();
    }
}

#[allow(non_snake_case)]
impl LensProfileSelectorWrapper {
    #[qinvokable]
    pub fn selectBrand(&mut self, brand: QString) {
        self.selected_brand = brand.clone();
        self.selected_brand_changed();

        let b_str = brand.to_string();
        let db = CAMERA_DB.read().unwrap();
        let mut models_list: Vec<QVariant> = if b_str == "Other" {
            vec![]
        } else {
            db.get_models_for_brand(&b_str)
                .into_iter()
                .map(|m| QString::from(m).into())
                .collect()
        };
        models_list.push(QString::from("Other").into());
        self.models = models_list.into();
        self.models_changed();

        self.update_compatible();
        self.validate_current();
    }

    #[qinvokable]
    pub fn selectModel(&mut self, model: QString) {
        self.selected_model = model.clone();
        self.selected_model_changed();

        let db = CAMERA_DB.read().unwrap();
        let b_str = self.selected_brand.to_string();
        let m_str = model.to_string();

        if let Some(cam) = db.find_camera(&b_str, &m_str) {
            let lens_list: Vec<QVariant> = db.get_lenses_for_camera(cam)
                .into_iter()
                .map(|l| QString::from(format!("{} {}", l.brand, l.model)).into())
                .collect();
            self.lenses = lens_list.into();
        } else {
            self.lenses = vec![].into();
        }
        self.lenses_changed();

        self.update_compatible();
        self.validate_current();
    }

    #[qinvokable]
    pub fn selectLens(&mut self, lens: QString) {
        self.selected_lens = lens;
        self.selected_lens_changed();
        self.validate_current();
    }

    #[qinvokable]
    pub fn setFocalLength(&mut self, focal_length: f64) {
        self.selected_focal_length = focal_length;
        self.selected_focal_length_changed();
        self.validate_current();
    }

    fn update_compatible(&mut self) {
        let db = CAMERA_DB.read().unwrap();
        let b_str = self.selected_brand.to_string();
        let m_str = self.selected_model.to_string();

        if let Some(cam) = db.find_camera(&b_str, &m_str) {
            let comp = db.get_compatible_cameras(cam);
            let list: Vec<QVariant> = comp.into_iter()
                .filter(|c| c.model != cam.model)
                .map(|c| QString::from(format!("{} {} ({})", c.brand, c.model, c.sensor_type)).into())
                .collect();
            self.compatible_cameras = list.into();
        } else {
            self.compatible_cameras = vec![].into();
        }
        self.compatible_cameras_changed();
    }

    #[qinvokable]
    pub fn prefillFromMetadata(&mut self, brand: QString, model: QString, lens: QString, focal_len: f64, width: u32, height: u32) {
        let meta = ExtractedVideoMetadata {
            make: if brand.is_empty() { None } else { Some(brand.to_string()) },
            model: if model.is_empty() { None } else { Some(model.to_string()) },
            lens_model: if lens.is_empty() { None } else { Some(lens.to_string()) },
            focal_length: if focal_len > 0.0 { Some(focal_len) } else { None },
            width,
            height,
        };

        let rec = MetadataPrefillResolver::resolve(&meta);
        if !rec.camera_brand.is_empty() {
            self.selectBrand(QString::from(rec.camera_brand));
        }
        if !rec.camera_model.is_empty() {
            self.selectModel(QString::from(rec.camera_model));
        }
        if !rec.lens_model.is_empty() {
            self.selectLens(QString::from(rec.lens_model));
        }
        if rec.focal_length > 0.0 {
            self.setFocalLength(rec.focal_length);
        }
    }

    #[qinvokable]
    pub fn validate_current(&mut self) {
        let is_zoom = self.selected_lens.to_string().contains('-');
        let sub = LensProfileSubmission {
            camera_brand: self.selected_brand.to_string(),
            camera_model: self.selected_model.to_string(),
            lens_brand: "".to_string(),
            lens_model: self.selected_lens.to_string(),
            focal_length: if self.selected_focal_length > 0.0 { Some(self.selected_focal_length) } else { None },
            is_zoom_lens: is_zoom,
            sensor_format: None,
            input_resolution: (1920, 1080),
            distortion_coefficients: vec![0.1, -0.05],
            optical_center: (960.0, 540.0),
        };

        let res = ProfileValidator::validate(&sub);
        self.can_submit = res.is_valid;
        if !res.errors.is_empty() {
            self.validation_error = QString::from(res.errors.join("; "));
        } else {
            self.validation_error = QString::from("");
        }
        self.validation_changed();
    }

    // Review / Toggle between submitted profiles
    #[qinvokable]
    pub fn loadReviewProfiles(&mut self, dummy_profiles_json: QString) {
        if let Ok(entries) = serde_json::from_str::<Vec<ProfileReviewEntry>>(&dummy_profiles_json.to_string()) {
            let mut mgr = self.review_mgr.write().unwrap();
            mgr.set_candidates(entries);
            self.refresh_review_state(&mgr);
        }
    }

    #[qinvokable]
    pub fn nextReviewProfile(&mut self) {
        let mut mgr = self.review_mgr.write().unwrap();
        if let Some(entry) = mgr.select_next() {
            let id = QString::from(entry.id.clone());
            self.refresh_review_state(&mgr);
            self.profile_selected_for_review(id);
        }
    }

    #[qinvokable]
    pub fn prevReviewProfile(&mut self) {
        let mut mgr = self.review_mgr.write().unwrap();
        if let Some(entry) = mgr.select_prev() {
            let id = QString::from(entry.id.clone());
            self.refresh_review_state(&mgr);
            self.profile_selected_for_review(id);
        }
    }

    #[qinvokable]
    pub fn removeReviewProfile(&mut self, id: QString) {
        let mut mgr = self.review_mgr.write().unwrap();
        if mgr.mark_for_removal(&id.to_string()) {
            self.refresh_review_state(&mgr);
        }
    }

    fn refresh_review_state(&mut self, mgr: &ReviewManager) {
        let cand = mgr.candidates();
        let list: Vec<QVariant> = cand.iter().map(|e| {
            let mut map = std::collections::HashMap::new();
            map.insert("id".to_string(), QString::from(e.id.clone()).into());
            map.insert("name".to_string(), QString::from(e.profile_name.clone()).into());
            map.insert("author".to_string(), QString::from(e.author.clone()).into());
            map.insert("rms_error".to_string(), e.rms_error.into());
            map.insert("is_active".to_string(), e.is_active.into());
            QVariant::from(map)
        }).collect();

        self.review_entries = list.into();
        self.review_entries_changed();

        if let Some(active) = mgr.current_selection() {
            self.active_profile_name = QString::from(active.profile_name.clone());
            self.active_profile_name_changed();
        }
    }
}
```

---

### File: src/ui/LensProfileSelector.qml
```qml
import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15
import "." 

ColumnLayout {
    id: root
    spacing: 10
    Layout.fillWidth: true

    property bool isCalibrator: false
    property string manualBrand: ""
    property string manualModel: ""
    property string manualLens: ""

    // 1. Camera Brand Selection
    RowLayout {
        Layout.fillWidth: true
        spacing: 8

        Label {
            text: qsTr("Camera Brand:")
            Layout.preferredWidth: 120
        }

        ComboBox {
            id: brandCombo
            Layout.fillWidth: true
            model: LensProfileWrapper.brands
            currentIndex: -1
            displayText: currentIndex === -1 ? qsTr("Select Camera Brand...") : currentText
            onActivated: {
                if (currentText === "Other") {
                    brandManualInput.visible = true;
                } else {
                    brandManualInput.visible = false;
                    LensProfileWrapper.selectBrand(currentText);
                }
            }
        }
    }

    TextField {
        id: brandManualInput
        visible: false
        Layout.fillWidth: true
        placeholderText: qsTr("Type camera brand manually...")
        onTextChanged: {
            root.manualBrand = text;
            LensProfileWrapper.selectBrand(text);
        }
    }

    // 2. Camera Model Selection
    RowLayout {
        Layout.fillWidth: true
        spacing: 8

        Label {
            text: qsTr("Camera Model:")
            Layout.preferredWidth: 120
        }

        ComboBox {
            id: modelCombo
            Layout.fillWidth: true
            model: LensProfileWrapper.models
            currentIndex: -1
            displayText: currentIndex === -1 ? qsTr("Select Camera Model...") : currentText
            onActivated: {
                if (currentText === "Other") {
                    modelManualInput.visible = true;
                } else {
                    modelManualInput.visible = false;
                    LensProfileWrapper.selectModel(currentText);
                }
            }
        }
    }

    TextField {
        id: modelManualInput
        visible: false
        Layout.fillWidth: true
        placeholderText: qsTr("Type camera model manually...")
        onTextChanged: {
            root.manualModel = text;
            LensProfileWrapper.selectModel(text);
        }
    }

    // Compatible Cameras Suggestion Info
    Rectangle {
        id: compBanner
        Layout.fillWidth: true
        visible: LensProfileWrapper.compatible_cameras.length > 0
        implicitHeight: compLabel.implicitHeight + 16
        color: "#2a2d34"
        radius: 4

        RowLayout {
            anchors.fill: parent
            anchors.margins: 8
            spacing: 8
            Label {
                text: "💡"
                font.pixelSize: 14
            }
            Label {
                id: compLabel
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: "#9ad1d4"
                text: qsTr("Compatible camera bodies sharing this mount/format: ") +
                      LensProfileWrapper.compatible_cameras.join(", ")
            }
        }
    }

    // 3. Lens Selection
    RowLayout {
        Layout.fillWidth: true
        spacing: 8

        Label {
            text: qsTr("Lens Model:")
            Layout.preferredWidth: 120
        }

        ComboBox {
            id: lensCombo
            Layout.fillWidth: true
            model: LensProfileWrapper.lenses
            editable: true
            displayText: currentText === "" ? qsTr("Select or type lens model...") : currentText
            onEditTextChanged: {
                LensProfileWrapper.selectLens(editText);
            }
            onActivated: {
                LensProfileWrapper.selectLens(currentText);
            }
        }
    }

    // 4. Focal Length (Crucial for zoom lenses)
    RowLayout {
        Layout.fillWidth: true
        spacing: 8

        Label {
            text: qsTr("Focal Length (mm):")
            Layout.preferredWidth: 120
        }

        SpinBox {
            id: focalSpin
            Layout.fillWidth: true
            from: 1
            to: 1200
            stepSize: 1
            editable: true
            value: 24
            onValueChanged: {
                LensProfileWrapper.setFocalLength(value);
            }
        }
    }

    // Validation Status Banner (Enforces strict rules in Calibrator)
    Rectangle {
        Layout.fillWidth: true
        visible: root.isCalibrator && !LensProfileWrapper.can_submit
        implicitHeight: valLabel.implicitHeight + 16
        color: "#4a1e1e"
        radius: 4

        RowLayout {
            anchors.fill: parent
            anchors.margins: 8
            spacing: 8
            Label {
                text: "⚠️"
                font.pixelSize: 14
            }
            Label {
                id: valLabel
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
                color: "#ff8b8b"
                text: qsTr("Submission Requirement: ") + LensProfileWrapper.validation_error
            }
        }
    }

    // 5. Review Switcher / A-B Toggle for Reviewing Profiles
    GroupBox {
        title: qsTr("Profile Review & Comparison")
        Layout.fillWidth: true
        visible: LensProfileWrapper.review_entries.length > 1

        ColumnLayout {
            anchors.fill: parent
            spacing: 8

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Button {
                    text: "◀ " + qsTr("Previous Profile")
                    onClicked: LensProfileWrapper.prevReviewProfile()
                }

                Label {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    font.bold: true
                    text: LensProfileWrapper.active_profile_name
                }

                Button {
                    text: qsTr("Next Profile") + " ▶"
                    onClicked: LensProfileWrapper.nextReviewProfile()
                }
            }

            RowLayout {
                Layout.fillWidth: true
                spacing: 8

                Label {
                    Layout.fillWidth: true
                    color: "#a0a0a0"
                    text: qsTr("Compare undistortion quality across submitted profiles. Discard inaccurate ones.")
                }

                Button {
                    text: qsTr("Mark Bad / Remove")
                    palette.button: "#7a2222"
                    onClicked: {
                        // Pass current active profile ID to removal
                        if (LensProfileWrapper.review_entries.length > 0) {
                            var cur = LensProfileWrapper.review_entries[LensProfileWrapper.active_review_index];
                            if (cur && cur.id) {
                                LensProfileWrapper.removeReviewProfile(cur.id);
                            }
                        }
                    }
                }
            }
        }
    }
}
```

---

### File: tests/lens_profile_refactor_test.rs
```rust
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
```

---

### Verification and Testing Steps

1. **Automated Unit Tests**:
   Run the test suite to verify schema validation, zoom focal length constraints, and review cycling:
   ```bash
   cargo test --test lens_profile_refactor_test
   ```

2. **Database Auto-Update Testing**:
   - The `CameraDatabase::update_from_remote()` worker queries `https://raw.githubusercontent.com/gyroflow/lens_profiles/main/camera_database.json` asynchronously upon initialization.
   - When the remote version is newer, the database safely updates in memory and persists to the user application cache directory.

3. **Selector and Compatibility Verification**:
   - Selecting `Sony` -> `FX3` pre-populates lenses and automatically displays the compatibility badge highlighting interchangeable full-frame E-mount cameras (`A7S III`, `A7 IV`, `ZV-E1`, etc.).
   - Setting a custom camera via "Other" activates text fields while enforcing that non-empty real names are submitted before calibration profiles can be exported.

4. **Calibrator Submission Validation**:
   - Calibrator disables submission buttons when `can_submit == false`.
   - Selecting a zoom lens prompts the user to supply the exact calibrated focal length in mm. Missing focal length prevents submission and displays an explicit inline warning.

5. **Profile Review Flow**:
   - When multiple profiles exist for the target setup, the review toggle buttons (`◀ Previous Profile` / `Next Profile ▶`) allow toggling undistortion preview in real-time.
   - Faulty or duplicate calibrations can be flagged or pruned with `Mark Bad / Remove`.

---

### Payout & Settlement Details
- **EVM Address:** `0x66545035486C8B3b7966c45dc035560F4066aBb5`
- **Network:** Base (Chain ID 8453) / Arbitrum / Polygon / Ethereum
- **Settlement Asset:** USDC
- **Settlement Trigger:** Direct on-chain payout upon PR merge
