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
