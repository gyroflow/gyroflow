// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Anton Belkin

//! The known lens profiles a clip is tried against, nearest first:
//!
//! 1. the profile that's loaded;
//! 2. the camera's profiles in the database for this frame size (the nearest frame rate first);
//! 3. its profiles for another frame size of the same aspect ratio, brought to the clip's frame two ways: scaled (the
//!    whole sensor shrunk to the frame) and cropped (a window of that mode, pixel for pixel);
//! 4. the same of its sibling cameras of the same generation, which often share the lens (HERO4 Silver for a HERO4
//!    Black).
//!
//! Profiles that draw the same lens within 5 px, with the same readout time, are tried once. Without a profile to go
//! by, two plain lenses to start an estimate from: a wide fisheye, and one without distortion.

use crate::{ LensProfile, lens_profile_database::LensProfileDatabase, stabilization_params::ReadoutDirection };
use super::{ lens::Lens, px_scale };

/// Lenses nearer to each other than this (median over the frame, px at 2.7K) are one lens to try: well inside what
/// tells a lens that fits from one that doesn't (`FIT_MEDIAN`), or the right one could be dropped for a near miss
const SAME_LENS: f64 = 5.0;
/// ... unless their readout times differ by this, ms: the readout is part of what's tried
const SAME_READOUT: f64 = 0.5;
/// The most profiles tried
const LIMIT: usize = 8;

/// How a profile got to be a lens of the clip's frame
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum How {
    /// A profile of this very frame, or the loaded one as the render scales it
    AsItIs,
    /// A profile of another frame size: the whole sensor shrunk to the frame
    Scaled,
    /// ... or a window of that mode, pixel for pixel
    Cropped,
    /// No profile: a plain lens to start an estimate from
    Generic,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    /// The name of the profile
    pub name: String,
    pub how: How,
    /// What `StabilizationManager::load_lens_profile` loads it by; empty for a lens that isn't a profile as it is
    /// (scaled, cropped, generic)
    pub id: String,
    /// The lens in the clip's frame
    pub lens: Lens,
    /// The profile's readout time over the picture's rows, ms (negative: read from the bottom up), when the profile
    /// is for the clip's frame size and rate
    pub readout_ms: Option<f64>,
    /// Whether `id` loads exactly this lens: a profile of this very frame size. A profile of another size, which the
    /// render would scale (`FrameTransform::get_lens_data_at_timestamp`) or which this brought to the frame by
    /// cropping, is offered as a profile of its own instead, see `lens_from_video::brought_profile`
    pub loadable: bool,
    /// It's the profile that is loaded already: when it fits there's nothing to load, only to say so. It can have no
    /// id at all - one estimated from this video and not saved to a file
    pub is_loaded: bool,
    /// The profile it comes from
    pub profile: Option<LensProfile>,
    /// What orders the candidates: how near this lens is, then the distance in frame rate, then the profile itself -
    /// the database is a map, and two of its profiles can have the same name, so without the last the order of equally
    /// near candidates (and so which of a group of equal lenses is tried, and what an estimate starts from) would
    /// change from run to run
    pub(super) rank: (usize, i64, String, String),
}
impl Candidate {
    /// A lens that is no profile: one found some other way, or a start for an estimate
    pub fn plain(name: &str, lens: Lens) -> Self {
        Self { name: name.into(), how: How::Generic, id: String::new(), lens, readout_ms: None, loadable: false, is_loaded: false, profile: None, rank: (99, 0, name.into(), String::new()) }
    }
    /// For the log
    pub fn label(&self) -> String {
        match self.how { How::Scaled => format!("{} (scaled)", self.name), How::Cropped => format!("{} (cropped)", self.name), _ => self.name.clone() }
    }
}

pub(super) fn simplified(s: &str) -> String { s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect() }
fn first_word(s: &str) -> String { simplified(s.split_whitespace().next().unwrap_or("")) }

/// 0: the camera itself, 1: a sibling of its generation - the same first word, with its number: HERO4 Black and
/// HERO4 Silver. None: another camera, the next generation included (a HERO5 Black is no HERO4 Black, and every
/// "HERO n Black" there is would be hundreds of profiles)
fn kinship(brand: &str, model: &str, p: &LensProfile) -> Option<usize> {
    if brand.is_empty() || model.is_empty() || simplified(&p.camera_brand) != simplified(brand) { return None; }
    let (a, b) = (simplified(model), simplified(&p.camera_model));
    if a == b { return Some(0); }
    if b.is_empty() { return None; }
    if first_word(model) == first_word(&p.camera_model) && first_word(model).chars().any(|c| c.is_numeric()) { return Some(1); }
    None
}

/// Whether a profile is of this camera: one with no camera named is anybody's
pub(super) fn of_camera(camera: (&str, &str), p: &LensProfile) -> bool {
    (camera.0.is_empty() && camera.1.is_empty()) || (simplified(&p.camera_brand) == simplified(camera.0) && simplified(&p.camera_model) == simplified(camera.1))
}

/// A lens that could be the clip's at all: in the model this module fits, with square pixels and its centre near the
/// middle of the frame
fn plausible(lens: &Lens, size: (usize, usize)) -> bool {
    let (w, h) = (size.0 as f64, size.1 as f64);
    lens.fx > 0.2 * w.max(h) && (lens.fx / lens.fy - 1.0).abs() < 0.04 && (lens.cx / w - 0.5).abs() < 0.08 && (lens.cy / h - 0.5).abs() < 0.1
}

/// A profile's readout time as the fit counts it: over the picture's rows, negative from the bottom up. A profile
/// from before the direction was written has it in the sign (`StabilizationManager::load_lens_profile` reads it so).
/// None for a sensor read along its rows: the fit knows no such readout. `readout_scale`: picture rows over the
/// rows the profile's time is for (`FrameTransform::readout_scale`), 1 for most files
fn readout_of(p: &LensProfile, readout_scale: f64) -> Option<f64> {
    let t = p.frame_readout_time.filter(|x| x.abs() > 0.5)?;
    let down = match p.frame_readout_direction {
        Some(ReadoutDirection::TopToBottom) => true,
        Some(ReadoutDirection::BottomToTop) => false,
        Some(_) => return None,
        None => t > 0.0,
    };
    Some(if down { t.abs() } else { -t.abs() } * readout_scale)
}

fn same_lens(a: &Candidate, b: &Candidate, size: (usize, usize), within: f64) -> bool {
    let same_readout = match (a.readout_ms, b.readout_ms) { (Some(x), Some(y)) => (x - y).abs() < SAME_READOUT, (None, None) => true, _ => false };
    // 2% of focal length is three times `SAME_LENS` over the frame: no need to draw those
    same_readout && (a.lens.fx / b.lens.fx - 1.0).abs() < 0.02 && a.lens.shift(&b.lens, size).median <= within
}

/// The profiles to try for a clip of this camera (brand, model, and the name of its lens or field of view when it's
/// known), frame size and rate. `loaded`: the profile that is loaded, if any. `readout_scale`: see `readout_of`
pub fn candidates(db: &LensProfileDatabase, camera: (&str, &str, &str), size: (usize, usize), fps: f64, loaded: Option<&LensProfile>, readout_scale: f64) -> Vec<Candidate> {
    let (w, h) = (size.0 as f64, size.1 as f64);
    let mut out: Vec<Candidate> = Vec::new();
    if size.0 == 0 || size.1 == 0 { return out; }

    let add = |p: &LensProfile, id: &str, tier: usize, out: &mut Vec<Candidate>| {
        let Some(lens) = Lens::from_profile(p) else { return };
        let (pw, ph) = (p.calib_dimension.w as f64, p.calib_dimension.h as f64);
        let dfps = if p.fps > 0.0 && fps > 0.0 { ((p.fps - fps).abs() * 100.0) as i64 } else { 0 };
        let name = if p.name.is_empty() { p.get_name() } else { p.name.clone() };
        let mut push = |lens: Lens, how: How, rank: usize, as_it_is: bool| {
            if !plausible(&lens, size) { return; }
            out.push(Candidate {
                name: name.clone(),
                how,
                id: if as_it_is { id.to_string() } else { String::new() },
                lens,
                readout_ms: if as_it_is && dfps < 50 { readout_of(p, readout_scale) } else { None },
                // Loadable by its id, and only with one: `load_lens_profile` takes anything else for a file name
                loadable: as_it_is && !id.is_empty(),
                is_loaded: tier == usize::MAX,
                profile: Some(p.clone()),
                rank: (rank, dfps, name.clone(), format!("{id}|{how:?}")),
            });
        };
        if p.calib_dimension.w == size.0 && p.calib_dimension.h == size.1 {
            push(lens, How::AsItIs, if tier == usize::MAX { 0 } else { 1 + 3 * tier }, true);
        } else if (pw / ph - w / h).abs() < 0.01 {
            let rank = if tier == usize::MAX { 0 } else { 2 + 3 * tier };
            // The render scales a profile of another size to the frame: as it is, then
            push(lens.scaled(w / pw), if tier == usize::MAX { How::AsItIs } else { How::Scaled }, rank, tier == usize::MAX);
            if pw > w && tier != usize::MAX { push(Lens { cx: lens.cx - (pw - w) / 2.0, cy: lens.cy - (ph - h) / 2.0, ..lens }, How::Cropped, rank + 1, false); }
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
    let mut distinct: Vec<Candidate> = Vec::new();
    for c in out {
        if distinct.iter().all(|k| !same_lens(&c, k, size, same)) { distinct.push(c); }
    }
    let kept = limited(distinct);
    if kept.is_empty() { generic(size) } else { kept }
}

/// At most `LIMIT` of them: the nearer tiers whole, and of the first one that doesn't fit, lenses spread evenly over
/// its focal lengths. A body with interchangeable lenses has dozens of profiles equally near, and the first few by
/// name would all be of one maker's lenses
fn limited(distinct: Vec<Candidate>) -> Vec<Candidate> {
    if distinct.len() <= LIMIT { return distinct; }
    let mut kept: Vec<Candidate> = Vec::new();
    let mut rest = distinct.into_iter().peekable();
    while let Some(tier) = rest.peek().map(|c| c.rank.0) {
        let mut of_tier: Vec<Candidate> = Vec::new();
        while let Some(c) = rest.next_if(|c| c.rank.0 == tier) { of_tier.push(c); }
        let room = LIMIT - kept.len();
        if of_tier.len() <= room { kept.extend(of_tier); if kept.len() == LIMIT { break; } continue; }
        of_tier.sort_by(|a, b| a.lens.fx.total_cmp(&b.lens.fx).then_with(|| a.rank.cmp(&b.rank)));
        let n = of_tier.len();
        let picks: Vec<usize> = (0..room).map(|i| if room > 1 { i * (n - 1) / (room - 1) } else { n / 2 }).collect();
        kept.extend(of_tier.into_iter().enumerate().filter(|(i, _)| picks.contains(i)).map(|(_, c)| c));
        break;
    }
    kept
}

fn loaded_id(p: &LensProfile) -> String {
    if !p.identifier.is_empty() { p.identifier.clone() } else { p.path_to_file.clone() }
}

/// Nothing is known of the lens: a wide fisheye, and a lens without distortion of 24 mm (35 mm equivalent). Starts
/// for an estimate, never profiles to offer
pub fn generic(size: (usize, usize)) -> Vec<Candidate> {
    let (w, h) = (size.0 as f64, size.1 as f64);
    let f = 0.46 * w.max(h);
    let pinhole = (w * w + h * h).sqrt() * 24.0 / 43.27;
    [("Generic fisheye", Lens::new(f, f, w / 2.0, h / 2.0, [0.05, 0.0, 0.0, 0.0])), ("Generic rectilinear 24 mm", Lens::new_pinhole(pinhole, pinhole, w / 2.0, h / 2.0))]
        .into_iter().map(|(name, lens)| Candidate::plain(name, lens)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(model: &str, size: (usize, usize), f: f64) -> LensProfile {
        let mut p = Lens::new(f, f, size.0 as f64 / 2.0, size.1 as f64 / 2.0, [0.04, 0.01, -0.02, 0.01]).to_profile(size);
        p.camera_brand = "GoPro".into();
        p.camera_model = model.into();
        p.fps = 24.0;
        p
    }

    #[test]
    fn siblings_are_of_one_generation() {
        let p = |model: &str| profile(model, (2704, 1520), 1400.0);
        assert_eq!(kinship("GoPro", "HERO4 Black", &p("HERO4 Black")), Some(0));
        assert_eq!(kinship("GoPro", "HERO4 Black", &p("HERO4 Silver")), Some(1));
        assert_eq!(kinship("GoPro", "HERO4 Black", &p("HERO5 Black")), None);
        assert_eq!(kinship("GoPro", "HERO4 Black", &p("HERO12 Black")), None);
        assert_eq!(kinship("Sony", "HERO4 Black", &p("HERO4 Black")), None);
    }

    /// The sign of a readout time from before its direction was written is its direction; one read along the rows is
    /// none the fit can use; and the profile's time is brought to the picture's rows
    #[test]
    fn readout_of_a_profile() {
        let mut p = profile("HERO4 Black", (2704, 1520), 1400.0);
        assert_eq!(readout_of(&p, 1.0), None);
        p.frame_readout_time = Some(-16.0);
        assert_eq!(readout_of(&p, 1.0), Some(-16.0));
        p.frame_readout_time = Some(16.0);
        assert_eq!(readout_of(&p, 1.0), Some(16.0));
        p.frame_readout_direction = Some(ReadoutDirection::BottomToTop);
        assert_eq!(readout_of(&p, 1.0), Some(-16.0));
        p.frame_readout_direction = Some(ReadoutDirection::TopToBottom);
        p.frame_readout_time = Some(-16.0);
        assert_eq!(readout_of(&p, 1.0), Some(16.0));
        assert_eq!(readout_of(&p, 0.84375), Some(13.5));
        p.frame_readout_direction = Some(ReadoutDirection::LeftToRight);
        assert_eq!(readout_of(&p, 1.0), None);
    }

    /// Lenses 10 px apart are two lenses to try, as are equal lenses with different readout times; and of too many
    /// equally near, the ones kept span the focal lengths
    #[test]
    fn near_lenses_are_kept_apart() {
        let size = (2704, 1520);
        let candidate = |f: f64, readout: Option<f64>, rank: usize| Candidate { lens: Lens::new(f, f, 1352.0, 760.0, [0.04, 0.01, -0.02, 0.01]), readout_ms: readout, rank: (rank, 0, format!("{f}"), String::new()), ..Candidate::plain("", Lens::new(f, f, 0.0, 0.0, [0.0; 4])) };
        let (a, near, same) = (candidate(1400.0, None, 1), candidate(1414.0, None, 1), candidate(1401.0, None, 1));
        let px = a.lens.shift(&near.lens, size).median;
        assert!(px > SAME_LENS && px < 15.0, "{px}");
        assert!(!same_lens(&a, &near, size, SAME_LENS) && same_lens(&a, &same, size, SAME_LENS));
        assert!(!same_lens(&a, &candidate(1400.0, Some(16.0), 1), size, SAME_LENS));
        assert!(!same_lens(&candidate(1400.0, Some(12.0), 1), &candidate(1400.0, Some(16.0), 1), size, SAME_LENS));
        assert!(same_lens(&candidate(1400.0, Some(16.2), 1), &candidate(1400.0, Some(16.0), 1), size, SAME_LENS));

        let many: Vec<Candidate> = [candidate(1000.0, None, 0)].into_iter().chain((0..30).map(|i| candidate(1100.0 + 50.0 * i as f64, None, 1))).chain([candidate(900.0, None, 2)]).collect();
        let kept = limited(many);
        let f: Vec<f64> = kept.iter().map(|c| c.lens.fx).collect();
        assert_eq!(f.len(), LIMIT);
        assert_eq!((f[0], f[1], f[LIMIT - 1]), (1000.0, 1100.0, 2550.0));
        assert!(f[1..].windows(2).all(|x| x[1] - x[0] >= 200.0), "{f:?}");
    }
}
