// SPDX-License-Identifier: GPL-3.0-or-later

//! Camera/lens metadata, independent of usable distortion calibrations.
//! The generated catalogue and its attribution live in `resources/`.

use std::collections::{BTreeMap, HashMap};
use serde::{Deserialize, Serialize};
use crate::LensProfile;

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Camera {
    pub id: String,
    pub brand: String,
    pub model: String,
    #[serde(default)] pub aliases: Vec<String>,
    #[serde(default)] pub mounts: Vec<String>,
    pub crop_factor: Option<f64>,
    pub sensor_width_mm: Option<f64>,
    pub sensor_height_mm: Option<f64>,
    #[serde(default)] pub profile_lenses: Vec<String>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Lens {
    pub id: String,
    pub brand: String,
    pub model: String,
    #[serde(default)] pub aliases: Vec<String>,
    #[serde(default)] pub mounts: Vec<String>,
    pub min_focal: Option<f64>,
    pub max_focal: Option<f64>,
}

impl Lens {
    pub fn display_name(&self) -> String {
        if self.brand.is_empty() || normalized(&self.model).starts_with(&normalized(&self.brand)) {
            self.model.clone()
        } else {
            format!("{} {}", self.brand, self.model)
        }
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Mount {
    pub name: String,
    pub compatible_with: Vec<String>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct CameraDatabase {
    pub schema_version: u32,
    pub cameras: Vec<Camera>,
    pub lenses: Vec<Lens>,
    pub mounts: Vec<Mount>,
    #[serde(skip)] camera_index: HashMap<String, usize>,
    #[serde(skip)] lens_index: HashMap<String, usize>,
    #[serde(skip)] lens_aliases: HashMap<String, Vec<usize>>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Selection {
    pub camera_brand: String,
    pub camera_model: String,
    pub lens_model: String,
}

pub fn normalized(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).filter(|c| c.is_alphanumeric()).collect()
}

pub fn normalized_brand(value: &str) -> String {
    match normalized(value).as_str() {
        "nikoncorporation" => "nikon".to_owned(),
        "canoninc" => "canon".to_owned(),
        _ => normalized(value),
    }
}

fn camera_key(brand: &str, model: &str) -> String {
    let original_brand = normalized(brand);
    let brand = normalized_brand(brand);
    let model = normalized(model);
    let model = model.strip_prefix(&original_brand).unwrap_or(&model);
    let model = model.strip_prefix(&brand).unwrap_or(&model);
    let mut model = if brand == "sony" { model.strip_prefix("alpha").map(|x| format!("a{x}")).unwrap_or_else(|| model.to_owned()) } else { model.to_owned() };
    if brand == "canon" { model = model.strip_prefix("eos").unwrap_or(&model).to_owned(); }
    if brand == "panasonic" {
        for prefix in ["lumix", "dmc", "dc"] { model = model.strip_prefix(prefix).unwrap_or(&model).to_owned(); }
    }
    format!("{brand}/{model}")
}

impl CameraDatabase {
    pub fn from_value(value: serde_json::Value) -> serde_json::Result<Self> {
        let mut db: Self = serde_json::from_value(value)?;
        if db.schema_version != 1 {
            return Err(<serde_json::Error as serde::de::Error>::custom("Unsupported camera catalogue schema"));
        }
        db.reindex();
        Ok(db)
    }

    pub fn bundled() -> Self {
        serde_json::from_str(include_str!("../../resources/camera_database.json"))
            .and_then(Self::from_value).unwrap_or_else(|e| {
                log::error!("Unable to read bundled camera catalogue: {e}");
                Self::default()
            })
    }

    fn reindex(&mut self) {
        self.camera_index.clear();
        self.lens_index.clear();
        self.lens_aliases.clear();
        for (i, camera) in self.cameras.iter().enumerate() {
            self.camera_index.insert(camera.id.clone(), i);
            for name in std::iter::once(&camera.model).chain(&camera.aliases) {
                self.camera_index.entry(camera_key(&camera.brand, name)).or_insert(i);
            }
        }
        for (i, lens) in self.lenses.iter().enumerate() {
            self.lens_index.insert(lens.id.clone(), i);
            for name in std::iter::once(&lens.model).chain(&lens.aliases) {
                for alias in [normalized(name), normalized(&format!("{} {name}", lens.brand))] {
                    let items = self.lens_aliases.entry(alias).or_default();
                    if !items.contains(&i) { items.push(i); }
                }
            }
        }
    }

    pub fn camera(&self, brand: &str, model: &str) -> Option<&Camera> {
        self.camera_index.get(&camera_key(brand, model)).map(|i| &self.cameras[*i])
    }

    pub fn camera_by_id(&self, id: &str) -> Option<&Camera> {
        self.camera_index.get(id).map(|i| &self.cameras[*i])
    }

    pub fn lens(&self, camera: Option<&Camera>, model: &str) -> Option<&Lens> {
        let candidates = self.lens_aliases.get(&normalized(model))?;
        let own: Vec<_> = candidates.iter().filter(|i| camera.is_some_and(|c| c.profile_lenses.contains(&self.lenses[**i].id))).collect();
        if own.len() == 1 { return Some(&self.lenses[*own[0]]); }
        if candidates.len() == 1 && !self.lenses[candidates[0]].brand.is_empty() { return Some(&self.lenses[candidates[0]]); }
        None
    }

    /// Add locally installed or newly published profiles even when using an old
    /// catalogue. User-entered crop factors describe video modes, not sensors.
    pub fn add_profiles<'a>(&mut self, profiles: impl IntoIterator<Item = &'a LensProfile>) {
        for profile in profiles {
            let brand = profile.camera_brand.trim();
            let model = profile.camera_model.trim();
            if brand.is_empty() || model.is_empty() { continue; }
            let key = camera_key(brand, model);
            let camera_idx = if let Some(i) = self.camera_index.get(&key) { *i } else {
                let i = self.cameras.len();
                self.cameras.push(Camera { id: key.clone(), brand: brand.to_owned(), model: model.to_owned(), ..Default::default() });
                self.camera_index.insert(key, i);
                i
            };
            let name = profile.lens_model.trim();
            if name.is_empty() { continue; }
            let lens_id = if let Some(lens) = self.lens(Some(&self.cameras[camera_idx]), name) { lens.id.clone() } else {
                let id = format!("gyroflow/{}/{}", self.cameras[camera_idx].id, normalized(name));
                if !self.lens_index.contains_key(&id) {
                    let i = self.lenses.len();
                    let (min_focal, max_focal) = focal_range(name).map(|(a, b)| (Some(a), Some(b))).unwrap_or_default();
                    self.lenses.push(Lens { id: id.clone(), model: name.to_owned(), min_focal, max_focal, ..Default::default() });
                    self.lens_index.insert(id.clone(), i);
                    self.lens_aliases.entry(normalized(name)).or_default().push(i);
                }
                id
            };
            if !self.cameras[camera_idx].profile_lenses.contains(&lens_id) {
                self.cameras[camera_idx].profile_lenses.push(lens_id);
            }
        }
    }

    /// 0 = lens used in a profile or native mount, 1 = Lensfun adapter direction.
    /// No distortion calibration is implied by either kind of compatibility.
    fn lens_mount_match(&self, camera: &Camera, lens: &Lens) -> Option<u8> {
        if camera.profile_lenses.contains(&lens.id) || camera.mounts.iter().any(|m| lens.mounts.contains(m)) { return Some(0); }
        if self.mounts.iter().any(|m| camera.mounts.contains(&m.name) && m.compatible_with.iter().any(|c| lens.mounts.contains(c))) { return Some(1); }
        None
    }

    /// Conservative suggestions only. Physical mount and known sensor crop must
    /// agree; video crop, digital correction and recording mode still need review.
    pub fn compatible_cameras(&self, a: &Camera, b: &Camera) -> bool {
        if a.id == b.id { return true; }
        a.crop_factor.zip(b.crop_factor).is_some_and(|(a, b)| a.is_finite() && b.is_finite() && a > 0.0 && b > 0.0 && (a - b).abs() / a.max(b) <= 0.01)
            && a.mounts.iter().any(|m| b.mounts.contains(m))
    }

    pub fn options(&self, selection: &Selection) -> serde_json::Value {
        let mut brands = BTreeMap::new();
        for camera in &self.cameras { brands.entry(normalized_brand(&camera.brand)).or_insert(&camera.brand); }
        let camera = self.camera(&selection.camera_brand, &selection.camera_model);
        let brand = brands.get(&normalized_brand(&selection.camera_brand)).map(|s| s.as_str()).unwrap_or(&selection.camera_brand);
        let mut models: Vec<_> = self.cameras.iter().filter(|c| normalized_brand(&c.brand) == normalized_brand(&selection.camera_brand)).map(|c| &c.model).collect();
        models.sort_by_key(|m| normalized(m));
        let model_aliases: BTreeMap<_, _> = self.cameras.iter().filter(|c| normalized_brand(&c.brand) == normalized_brand(&selection.camera_brand)).map(|c| (&c.model, &c.aliases)).collect();
        let mut lenses: Vec<_> = camera.into_iter().flat_map(|c| self.lenses.iter().filter_map(move |l| {
            self.lens_mount_match(c, l).map(|adapted| serde_json::json!({
                "name": l.display_name(), "aliases": l.aliases, "adapted": adapted == 1,
                "has_profiles": c.profile_lenses.contains(&l.id), "zoom": l.min_focal.zip(l.max_focal).is_some_and(|(a,b)| b > a)
            }))
        })).collect();
        lenses.sort_by_key(|l| (l["adapted"].as_bool().unwrap_or(false), normalized(l["name"].as_str().unwrap_or_default())));
        lenses.dedup_by(|a, b| a["name"] == b["name"]);
        let lens = self.lens(camera, &selection.lens_model);
        serde_json::json!({
            "brands": brands.values().collect::<Vec<_>>(), "models": models, "model_aliases": model_aliases, "lenses": lenses, "camera": camera,
            "selection": { "camera_brand": camera.map(|c| c.brand.as_str()).unwrap_or(brand),
                "camera_model": camera.map(|c| c.model.as_str()).unwrap_or(&selection.camera_model),
                "lens_model": lens.map(Lens::display_name).unwrap_or_else(|| selection.lens_model.clone()) }
        })
    }

    pub fn submission_errors(&self, profile: &LensProfile) -> Vec<&'static str> {
        let mut errors = Vec::new();
        let specific = |s: &str| !["", "unknown", "other", "na", "none", "test", "camera", "lens", "brand", "model", "undefined"].contains(&normalized(s).as_str());
        if !specific(&profile.camera_brand) { errors.push("Enter the camera manufacturer."); }
        if !specific(&profile.camera_model) || normalized(&profile.camera_model) == normalized(&profile.camera_brand) { errors.push("Enter the exact camera model."); }
        if !specific(&profile.lens_model) { errors.push("Enter the lens model or the built-in lens mode."); }
        let camera = self.camera(&profile.camera_brand, &profile.camera_model);
        let interchangeable = camera.is_some_and(|c| c.mounts.iter().any(|mount| self.mounts.iter().any(|m| &m.name == mount)));
        let generic_mode = ["wide", "linear", "narrow", "normal", "ultrawide", "standard", "kit", "stock"].contains(&normalized(&profile.lens_model).as_str());
        lazy_static::lazy_static! { static ref ONLY_FOCAL: regex::Regex = regex::Regex::new(r"(?i)^\s*\d+(?:\.\d+)?\s*(?:[-–—]\s*\d+(?:\.\d+)?)?\s*(?:mm)?\s*(?:f/?\s*\d+(?:\.\d+)?)?\s*$").unwrap(); }
        if ONLY_FOCAL.is_match(&profile.lens_model) || (interchangeable && generic_mode) {
            errors.push("Specify the lens manufacturer and model, not only its focal length or field of view.");
        }
        let zoom = self.lens(camera, &profile.lens_model).and_then(|l| l.min_focal.zip(l.max_focal)).filter(|(a,b)| b > a)
            .or_else(|| focal_range(&profile.lens_model));
        if zoom.is_some() && !profile.focal_length.is_some_and(|f| f.is_finite() && f > 0.0) {
            errors.push("Enter the focal length used to calibrate this zoom lens.");
        }
        if profile.focal_length.is_some_and(|f| !f.is_finite() || f <= 0.0) { errors.push("Focal length must be a positive number."); }
        if profile.crop_factor.is_some_and(|f| !f.is_finite() || f <= 0.0) { errors.push("Crop factor must be a positive number."); }
        if profile.calib_dimension.w == 0 || profile.calib_dimension.h == 0 || !profile.fps.is_finite() || profile.fps <= 0.0 {
            errors.push("Load the calibration video to supply its dimensions and frame rate.");
        }
        errors
    }
}

fn focal_range(name: &str) -> Option<(f64, f64)> {
    lazy_static::lazy_static! { static ref RANGE: regex::Regex = regex::Regex::new(r"(?:^|[^\d.])(\d+(?:\.\d+)?)\s*[-–—]\s*(\d+(?:\.\d+)?)\s*(?:mm\b)?").unwrap(); }
    let captures = RANGE.captures(name)?;
    let low: f64 = captures[1].parse().ok()?;
    let high: f64 = captures[2].parse().ok()?;
    (low > 0.0 && low < high && high <= 3000.0).then_some((low, high))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn fixture() -> CameraDatabase {
        CameraDatabase::from_value(serde_json::json!({
            "schema_version": 1,
            "mounts": [{"name":"E", "compatible_with":["M42"]}],
            "cameras": [
                {"id":"sony/ilce6000","brand":"Sony","model":"ILCE-6000","aliases":["Alpha 6000"],"mounts":["E"],"crop_factor":1.5},
                {"id":"sony/ilce6300","brand":"Sony","model":"ILCE-6300","mounts":["E"],"crop_factor":1.5},
                {"id":"sony/fullframe","brand":"Sony","model":"Full frame","mounts":["E"],"crop_factor":1},
                {"id":"other/samecrop","brand":"Other","model":"Same crop","mounts":["X"],"crop_factor":1.5}
            ],
            "lenses": [
                {"id":"native","brand":"Sony","model":"E 16-50mm","mounts":["E"],"min_focal":16,"max_focal":50},
                {"id":"adapted","brand":"Maker","model":"Prime 50mm","mounts":["M42"]}
            ]
        })).unwrap()
    }

    #[test]
    fn aliases_and_mount_direction() {
        let db = fixture();
        let c = db.camera("SONY", "Sony a6000").unwrap();
        assert_eq!(c.model, "ILCE-6000");
        assert_eq!(db.lens_mount_match(c, &db.lenses[0]), Some(0));
        assert_eq!(db.lens_mount_match(c, &db.lenses[1]), Some(1));
        let reverse = Camera { mounts: vec!["M42".into()], ..Default::default() };
        assert_eq!(db.lens_mount_match(&reverse, &db.lenses[0]), None);
        assert!(db.compatible_cameras(c, &db.cameras[1]));
        assert!(!db.compatible_cameras(c, &db.cameras[2]));
        assert!(!db.compatible_cameras(c, &db.cameras[3]));
    }

    #[test]
    fn local_profiles_survive_catalogue_updates_without_invented_sensor_data() {
        let mut db = fixture();
        let mut p = LensProfile::default();
        p.camera_brand = "Future".into(); p.camera_model = "Camera 2".into(); p.lens_model = "18–55mm".into(); p.crop_factor = Some(2.0);
        db.add_profiles([&p, &p]);
        let c = db.camera("future", "Camera 2").unwrap();
        assert_eq!(c.profile_lenses.len(), 1);
        assert!(c.crop_factor.is_none());
        assert_eq!(db.lens(Some(c), "18–55mm").unwrap().max_focal, Some(55.0));
    }

    #[test]
    fn validates_manual_metadata_and_zoom_focal_length() {
        let db = fixture();
        let mut p = LensProfile::default();
        p.camera_brand = "Future".into(); p.camera_model = "Camera 2".into(); p.lens_model = "Maker 18-55mm".into();
        p.fps = 30.0; p.calib_dimension.w = 1920; p.calib_dimension.h = 1080;
        assert_eq!(db.submission_errors(&p).len(), 1);
        p.focal_length = Some(24.0);
        assert!(db.submission_errors(&p).is_empty());
        p.camera_model = "Other".into();
        assert!(!db.submission_errors(&p).is_empty());
        p.camera_model = "Camera 2".into(); p.lens_model = "Wide".into(); p.focal_length = None;
        assert!(db.submission_errors(&p).is_empty());
        p.crop_factor = Some(f64::NAN);
        assert!(!db.submission_errors(&p).is_empty());
        p.crop_factor = None; p.camera_brand = "Sony".into(); p.camera_model = "a6000".into();
        assert!(!db.submission_errors(&p).is_empty()); // Wide is not a lens model for a mirrorless body.
        p.lens_model = "50mm f/1.8".into();
        assert!(!db.submission_errors(&p).is_empty());
    }

    #[test]
    fn bundled_catalogue_has_both_sources_and_unique_ids() {
        let db = CameraDatabase::bundled();
        assert!(db.cameras.len() > 1000);
        assert!(db.lenses.len() > 1000);
        assert!(db.camera("Sony", "a6000").unwrap().crop_factor.is_some());
        assert!(db.camera("Nikon", "D5300").unwrap().crop_factor.is_some());
        assert_eq!(camera_key("Canon", "EOS R5"), camera_key("Canon", "R5"));
        assert_eq!(camera_key("Panasonic", "Lumix DC-GH5"), camera_key("Panasonic", "GH5"));
        assert!(db.cameras.iter().any(|c| c.brand == "RED"));
        assert_eq!(db.cameras.iter().map(|c| &c.id).collect::<BTreeSet<_>>().len(), db.cameras.len());
        assert_eq!(db.lenses.iter().map(|c| &c.id).collect::<BTreeSet<_>>().len(), db.lenses.len());
        assert!(db.cameras.iter().flat_map(|c| &c.profile_lenses).all(|id| db.lens_index.contains_key(id)));
        assert!(CameraDatabase::from_value(serde_json::json!({"schema_version":2,"cameras":[],"lenses":[],"mounts":[]})).is_err());
    }
}
