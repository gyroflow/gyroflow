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
