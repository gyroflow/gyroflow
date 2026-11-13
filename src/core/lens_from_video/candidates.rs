// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Anton Belkin

//! The known lens profiles a clip is tried against, nearest first:
//!
//! 1. the profile that's loaded;
//! 2. the camera's profiles in the database for this frame size (the nearest frame rate first);
//! 3. its profiles for another frame size of the same aspect ratio, brought to the clip's frame two ways: scaled (the
//!    whole sensor shrunk to the frame) and cropped (a window of that mode, pixel for pixel);
//! 4. the same of its sibling cameras, which often share the lens (HERO4 Silver for a HERO4 Black, Osmo Action 5 for a 6).
//!
//! Profiles that draw the same lens within 25 px are tried once. Without a profile to go by, two plain lenses to start
//! an estimate from: a wide fisheye, and one without distortion.

use crate::{ LensProfile, lens_profile_database::LensProfileDatabase };
use super::{ lens::{ Lens, PINHOLE_K }, px_scale };

/// Lenses nearer to each other than this (median over the frame, px at 2.7K) are one lens to try
const SAME_LENS: f64 = 25.0;
/// The most profiles tried
const LIMIT: usize = 8;

#[derive(Clone, Debug)]
pub struct Candidate {
    /// For the user: the name of the profile, and how it was brought to the frame
    pub name: String,
    /// What `StabilizationManager::load_lens_profile` loads it by; empty for a lens that isn't a profile as it is
    /// (scaled, cropped, generic)
    pub id: String,
    /// The lens in the clip's frame
    pub lens: Lens,
    /// The profile's readout time, when it's for the clip's frame size and rate
    pub readout_ms: Option<f64>,
    /// Whether `id` loads exactly this lens: a profile of this very frame size. A profile of another size, which the
    /// render would scale (`FrameTransform::get_lens_data_at_timestamp`) or which this brought to the frame by
    /// cropping, is offered as a profile of its own instead, see `lens_from_video::profile_of`
    pub loadable: bool,
    /// It's the profile that is loaded already: when it fits there's nothing to load, only to say so. It can have no
    /// id at all - one estimated from this video and not saved to a file
    pub is_loaded: bool,
    /// The profile it comes from, for the names of the camera and the lens
    pub profile: Option<LensProfile>,
    /// What orders the candidates: how near this lens is, then the distance in frame rate, then the profile itself -
    /// the database is a map, and two of its profiles can have the same name, so without the last the order of equally
    /// near candidates (and so which of a group of equal lenses is tried, and what an estimate starts from) would
    /// change from run to run
    pub(super) rank: (usize, i64, String, String),
}

fn simplified(s: &str) -> String { s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect() }
fn without_numbers(s: &str) -> String { s.chars().filter(|c| c.is_alphabetic()).flat_map(|c| c.to_lowercase()).collect() }
fn first_word(s: &str) -> String { simplified(s.split_whitespace().next().unwrap_or("")) }

/// 0: the camera itself, 1: a sibling (the same name but for its numbers, or the same first word: HERO4 Black and
/// HERO4 Silver), none: another camera
fn kinship(brand: &str, model: &str, p: &LensProfile) -> Option<usize> {
    if brand.is_empty() || model.is_empty() || simplified(&p.camera_brand) != simplified(brand) { return None; }
    let (a, b) = (simplified(model), simplified(&p.camera_model));
    if a == b { return Some(0); }
    if b.is_empty() { return None; }
    if without_numbers(model) == without_numbers(&p.camera_model) || (first_word(model) == first_word(&p.camera_model) && first_word(model).chars().any(|c| c.is_numeric())) { return Some(1); }
    None
}

/// A lens that could be the clip's at all: in the model this module fits, with square pixels and its centre near the
/// middle of the frame
fn plausible(lens: &Lens, size: (usize, usize)) -> bool {
    let (w, h) = (size.0 as f64, size.1 as f64);
    lens.fx > 0.2 * w.max(h) && (lens.fx / lens.fy - 1.0).abs() < 0.04 && (lens.cx / w - 0.5).abs() < 0.08 && (lens.cy / h - 0.5).abs() < 0.1
}

/// The profiles to try for a clip of this camera (brand, model, and the name of its lens or field of view when it's
/// known), frame size and rate. `loaded`: the profile that is loaded, if any
pub fn candidates(db: &LensProfileDatabase, camera: (&str, &str, &str), size: (usize, usize), fps: f64, loaded: Option<&LensProfile>) -> Vec<Candidate> {
    let (w, h) = (size.0 as f64, size.1 as f64);
    let mut out: Vec<Candidate> = Vec::new();
    if size.0 == 0 || size.1 == 0 { return out; }

    let add = |p: &LensProfile, id: &str, tier: usize, out: &mut Vec<Candidate>| {
        let Some(lens) = Lens::from_profile(p) else { return };
        let (pw, ph) = (p.calib_dimension.w as f64, p.calib_dimension.h as f64);
        let dfps = if p.fps > 0.0 && fps > 0.0 { ((p.fps - fps).abs() * 100.0) as i64 } else { 0 };
        let name = if p.name.is_empty() { p.get_name() } else { p.name.clone() };
        let mut push = |lens: Lens, how: &str, rank: usize, as_it_is: bool| {
            if !plausible(&lens, size) { return; }
            out.push(Candidate {
                name: if how.is_empty() { name.clone() } else { format!("{name} ({how})") },
                id: if as_it_is { id.to_string() } else { String::new() },
                lens,
                readout_ms: p.frame_readout_time.filter(|x| as_it_is && dfps < 50 && x.abs() > 0.5).map(|x| if matches!(p.frame_readout_direction, Some(crate::stabilization_params::ReadoutDirection::BottomToTop)) { -x.abs() } else { x.abs() }),
                // Loadable by its id, and only with one: `load_lens_profile` takes anything else for a file name
                loadable: as_it_is && !id.is_empty(),
                is_loaded: tier == usize::MAX,
                profile: Some(p.clone()),
                rank: (rank, dfps, name.clone(), format!("{id}|{how}")),
            });
        };
        if p.calib_dimension.w == size.0 && p.calib_dimension.h == size.1 {
            push(lens, "", if tier == usize::MAX { 0 } else { 1 + 3 * tier }, true);
        } else if (pw / ph - w / h).abs() < 0.01 {
            let rank = if tier == usize::MAX { 0 } else { 2 + 3 * tier };
            // The render scales a profile of another size to the frame: as it is, then
            push(lens.scaled(w / pw), "scaled", rank, tier == usize::MAX);
            if pw > w && tier != usize::MAX { push(Lens { cx: lens.cx - (pw - w) / 2.0, cy: lens.cy - (ph - h) / 2.0, ..lens }, "cropped", rank + 1, false); }
        }
    };

    if let Some(p) = loaded { add(p, &loaded_id(p), usize::MAX, &mut out); }
    let fov = simplified(camera.2);
    for (key, p) in db.profiles() {
        if p.path_to_file.ends_with(".gyroflow") { continue; }
        let Some(tier) = kinship(camera.0, camera.1, p) else { continue };
        // A profile of another lens or field of view of the camera (Linear for a Wide clip) isn't near
        let lens_model = simplified(&p.lens_model);
        if !fov.is_empty() && !lens_model.is_empty() && !lens_model.contains(&fov) && !fov.contains(&lens_model) { continue; }
        add(p, key, tier, &mut out);
    }
    out.sort_by(|a, b| a.rank.cmp(&b.rank));

    let same = SAME_LENS * px_scale(size);
    let mut kept: Vec<Candidate> = Vec::new();
    for c in out {
        if kept.len() >= LIMIT { break; }
        if kept.iter().all(|k| c.lens.shift(&k.lens, size).median > same) { kept.push(c); }
    }
    if kept.is_empty() { kept = generic(size); }
    kept
}

fn loaded_id(p: &LensProfile) -> String {
    if !p.identifier.is_empty() { p.identifier.clone() } else { p.path_to_file.clone() }
}

/// Nothing is known of the lens: a wide fisheye, and a lens without distortion of 24 mm (35 mm equivalent)
pub fn generic(size: (usize, usize)) -> Vec<Candidate> {
    let (w, h) = (size.0 as f64, size.1 as f64);
    let f = 0.46 * w.max(h);
    let pinhole = (w * w + h * h).sqrt() * 24.0 / 43.27;
    [("Generic fisheye", Lens::new(f, f, w / 2.0, h / 2.0, [0.05, 0.0, 0.0, 0.0])), ("Generic rectilinear 24 mm", Lens::new(pinhole, pinhole, w / 2.0, h / 2.0, PINHOLE_K))]
        .into_iter().map(|(name, lens)| Candidate { name: name.into(), id: String::new(), lens, readout_ms: None, loadable: false, is_loaded: false, profile: None, rank: (99, 0, name.into(), String::new()) }).collect()
}
