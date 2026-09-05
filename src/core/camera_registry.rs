// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 the gyroflow contributors

//! Derived camera/lens registry.
//!
//! Aggregates the unique `(camera_brand, camera_model)` entries that appear
//! across the loaded [`LensProfile`]s into a flat, sorted list that the rest
//! of the application (calibrator UI, profile selectors, future LensFun
//! integration) can use without re-iterating the profile map itself.
//!
//! This is a partial step toward
//! <https://github.com/gyroflow/gyroflow/issues/742>. It deliberately
//! contains **no UI changes** — the calibrator and lens-profile selectors
//! stay as-is. Wiring this registry into the QML side is a separate,
//! reviewable follow-up.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::LensProfile;
use crate::lens_profile_database::LensProfileDatabase;

/// One camera as derived from the lens profile database.
///
/// Built from every loaded [`LensProfile`] that has a non-empty
/// `camera_brand` and `camera_model`. The `lens_models` and other aggregate
/// fields are deduplicated across all profiles that share the same brand+model.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CameraEntry {
    /// Camera manufacturer, e.g. `"Sony"`, `"GoPro"`. The casing of the
    /// first profile that contributed to this entry is preserved.
    pub brand: String,
    /// Camera model, e.g. `"a7s III"`, `"HERO11 Black"`. Same casing rule
    /// as `brand`.
    pub model: String,
    /// Distinct, case-insensitive lens model names that have ever been
    /// calibrated for this camera. Sorted alphabetically (case-insensitive).
    pub lens_models: Vec<String>,
    /// First non-`None` `crop_factor` encountered across the profiles for
    /// this camera. `None` if no profile carried one.
    pub crop_factor: Option<f64>,
    /// How many lens profiles in the database reference this camera. Useful
    /// for ranking / popularity in UI selectors.
    pub profile_count: usize,
    /// Whether at least one of the contributing profiles is marked
    /// `official`.
    pub has_official_profile: bool,
}

impl CameraEntry {
    /// Stable key used for deduplication: lowercased `brand|model`.
    fn key(brand: &str, model: &str) -> (String, String) {
        (brand.to_lowercase(), model.to_lowercase())
    }
}

/// Build the registry from any iterator of borrowed lens profiles.
///
/// The function is generic over iterators so it can be unit-tested with
/// synthetic profiles without having to load the bundled CBOR database.
///
/// Empty `camera_brand` or `camera_model` profiles are skipped (the
/// calibrator-version stub profiles that ship alongside the real ones).
/// Output is sorted by `(brand, model)` case-insensitively for stability.
pub fn build_camera_registry<'a, I>(profiles: I) -> Vec<CameraEntry>
where
    I: IntoIterator<Item = &'a LensProfile>,
{
    let mut acc: BTreeMap<(String, String), CameraEntry> = BTreeMap::new();

    for profile in profiles {
        let brand = profile.camera_brand.trim();
        let model = profile.camera_model.trim();
        if brand.is_empty() || model.is_empty() {
            continue;
        }

        let key = CameraEntry::key(brand, model);
        let entry = acc.entry(key).or_insert_with(|| CameraEntry {
            brand: brand.to_string(),
            model: model.to_string(),
            ..Default::default()
        });

        entry.profile_count += 1;

        if entry.crop_factor.is_none() {
            entry.crop_factor = profile.crop_factor;
        }
        if profile.official {
            entry.has_official_profile = true;
        }

        let lens = profile.lens_model.trim();
        if !lens.is_empty()
            && !entry
                .lens_models
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(lens))
        {
            entry.lens_models.push(lens.to_string());
        }
    }

    let mut out: Vec<CameraEntry> = acc.into_values().collect();
    for entry in &mut out {
        entry
            .lens_models
            .sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    }
    out
}

/// Convenience wrapper: build the registry directly from a loaded database.
pub fn build_camera_registry_from_database(db: &LensProfileDatabase) -> Vec<CameraEntry> {
    build_camera_registry(db.iter_profiles())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(brand: &str, model: &str, lens: &str, crop: Option<f64>, official: bool) -> LensProfile {
        // `LensProfile` has private fields, so we can't use struct-literal
        // syntax with `..Default::default()` from outside its module.
        let mut p = LensProfile::default();
        p.camera_brand = brand.into();
        p.camera_model = model.into();
        p.lens_model = lens.into();
        p.crop_factor = crop;
        p.official = official;
        p
    }

    #[test]
    fn empty_input_yields_empty_registry() {
        let profiles: Vec<LensProfile> = Vec::new();
        assert!(build_camera_registry(profiles.iter()).is_empty());
    }

    #[test]
    fn profiles_without_brand_or_model_are_skipped() {
        let profiles = vec![
            fixture("", "a7s III", "FE 24-70", None, true),
            fixture("Sony", "", "FE 24-70", None, true),
            fixture("  ", "  ", "FE 24-70", None, true),
        ];
        assert!(build_camera_registry(profiles.iter()).is_empty());
    }

    #[test]
    fn aggregates_unique_cameras_and_deduplicates_lenses() {
        let profiles = vec![
            fixture("Sony", "a7s III", "FE 24-70 GM", Some(1.0), true),
            fixture("Sony", "a7s III", "FE 24-70 GM", Some(1.0), true), // dup lens, same crop
            fixture("Sony", "a7s III", "FE 16-35 GM", None, false),
            fixture("GoPro", "HERO11 Black", "fixed", Some(1.0), true),
        ];

        let reg = build_camera_registry(profiles.iter());
        assert_eq!(reg.len(), 2);

        let sony = reg.iter().find(|c| c.brand == "Sony" && c.model == "a7s III").unwrap();
        assert_eq!(sony.lens_models, vec!["FE 16-35 GM".to_string(), "FE 24-70 GM".to_string()]);
        assert_eq!(sony.profile_count, 3);
        assert_eq!(sony.crop_factor, Some(1.0));
        assert!(sony.has_official_profile);

        let gopro = reg.iter().find(|c| c.brand == "GoPro").unwrap();
        assert_eq!(gopro.lens_models, vec!["fixed".to_string()]);
        assert_eq!(gopro.profile_count, 1);
    }

    #[test]
    fn brand_and_model_match_is_case_insensitive() {
        let profiles = vec![
            fixture("sony", "A7S III", "FE 24-70", None, false),
            fixture("Sony", "a7s III", "FE 24-70", None, true),
        ];
        let reg = build_camera_registry(profiles.iter());
        assert_eq!(reg.len(), 1, "case differences must collapse to a single entry");
        assert_eq!(reg[0].profile_count, 2);
        assert!(reg[0].has_official_profile);
    }

    #[test]
    fn lens_model_dedup_is_case_insensitive() {
        let profiles = vec![
            fixture("Sony", "a7s III", "FE 24-70 GM", None, false),
            fixture("Sony", "a7s III", "fe 24-70 gm", None, false),
            fixture("Sony", "a7s III", "FE 24-70 GM II", None, false),
        ];
        let reg = build_camera_registry(profiles.iter());
        assert_eq!(reg[0].lens_models.len(), 2);
    }

    #[test]
    fn output_is_sorted_by_brand_then_model() {
        let profiles = vec![
            fixture("Zorki", "1", "Industar-22", None, false),
            fixture("Canon", "EOS R5", "RF 24-70", None, false),
            fixture("Canon", "EOS R3", "RF 24-70", None, false),
            fixture("DJI", "Mavic 3", "fixed", None, false),
        ];
        let reg = build_camera_registry(profiles.iter());
        let order: Vec<(&str, &str)> = reg
            .iter()
            .map(|c| (c.brand.as_str(), c.model.as_str()))
            .collect();
        assert_eq!(
            order,
            vec![
                ("Canon", "EOS R3"),
                ("Canon", "EOS R5"),
                ("DJI", "Mavic 3"),
                ("Zorki", "1"),
            ]
        );
    }

    #[test]
    fn crop_factor_takes_first_non_none_then_keeps_it() {
        let profiles = vec![
            fixture("Sony", "a7s III", "FE 24-70", None, false),
            fixture("Sony", "a7s III", "FE 16-35", Some(1.0), false),
            fixture("Sony", "a7s III", "FE 70-200", Some(2.0), false),
        ];
        let reg = build_camera_registry(profiles.iter());
        assert_eq!(reg[0].crop_factor, Some(1.0));
    }
}
