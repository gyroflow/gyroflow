// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Anton Belkin

//! Against clips whose truth is known: points of a static scene seen by a camera that walks and shakes, drawn through
//! a lens with a rolling shutter (each point at the time its row is read, on the camera's continuous path), with noise.
//! Run with --release: the fits are slow in a debug build.

use super::*;
use nalgebra::{ Rotation3, Vector3 };
use crate::stabilization_params::ReadoutDirection;

const SIZE: (usize, usize) = (2704, 1520);
const FRAMES: usize = 100;
/// Of the frame interval: a HERO4 Black at 2.7K 24 fps reads a frame in 31 ms
const READOUT: f64 = 0.74;

/// The lens of a GoPro HERO4 Black in 2.7K Medium, and the profile of its Wide mode
fn medium() -> Lens { Lens::new(1766.95, 1773.07, 1311.74, 774.68, [0.05352, -0.00625, 0.02390, -0.01311]) }
fn wide() -> Lens { Lens::new(1382.0, 1382.0, 1352.0, 760.0, [0.0375, 0.0217, -0.0321, 0.0149]) }

/// xorshift64*, and normals by Box-Muller: the clips have to be the same on every run
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12; self.0 ^= self.0 << 25; self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn uniform(&mut self, a: f64, b: f64) -> f64 { a + (b - a) * self.next() }
    fn normal(&mut self) -> f64 { (-2.0 * self.next().max(1e-300).ln()).sqrt() * (2.0 * std::f64::consts::PI * self.next()).cos() }
}

/// How the camera of a clip moves
#[derive(Clone, Copy)]
struct Motion {
    /// Its shake, 1 for a hand's
    shake: f64,
    path: Path,
}
#[derive(Clone, Copy, PartialEq)]
enum Path { /** Stays on the spot */ Still, /** Walks: forward, swaying */ Walk, /** Straight ahead: a gimbal, a drone, a car mount */ Forward }
const WALKS: Motion = Motion { shake: 1.0, path: Path::Walk };
const TURNS: Motion = Motion { shake: 1.0, path: Path::Still };
/// The shake cut to a tenth on a straight path: the data pull on the lens slowly here
const STEADY: Motion = Motion { shake: 0.1, path: Path::Forward };

/// The camera at a time in frames: world → camera. Nothing faster than a turn in 12 frames: the fit knows the
/// camera's rates from the frames next to each one
fn pose(t: f64, m: Motion) -> (Rotation3<f64>, Vector3<f64>) {
    let r = Rotation3::new(Vector3::new(
        0.04 * (0.5 * t).sin() + 0.03 * (0.31 * t + 0.5).sin(),
        0.05 * (0.37 * t + 2.0).sin() + 0.03 * (0.55 * t).sin(),
        0.02 * (0.29 * t).sin()) * m.shake);
    let centre = match m.path {
        Path::Still => Vector3::zeros(),
        Path::Walk => Vector3::new(0.15 * (0.21 * t).sin(), 0.08 * (0.33 * t + 1.0).sin(), 0.04 * t),
        Path::Forward => Vector3::new(0.0, 0.0, 0.04 * t),
    };
    (r, -(r * centre))
}

fn clip(lens: &Lens, readout: f64, walks: bool, noise: f64) -> Fragment { clip_of(lens, readout, if walks { WALKS } else { TURNS }, noise) }

fn clip_of(lens: &Lens, readout: f64, motion: Motion, noise: f64) -> Fragment {
    clip_at(lens, readout, motion, noise, &(0..FRAMES).map(|i| i as f64).collect::<Vec<_>>())
}

/// `at`: when each frame is taken, in frame intervals. The fragment knows those times
fn clip_at(lens: &Lens, readout: f64, motion: Motion, noise: f64, at: &[f64]) -> Fragment {
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let mut fragment = Fragment::new(SIZE, 1000.0 / 24.0);
    for track in 0..1500u32 {
        let x = Vector3::new(rng.uniform(-9.0, 9.0), rng.uniform(-5.0, 5.0), rng.uniform(2.5, 28.0));
        let from = rng.uniform(-30.0, FRAMES as f64) as i64;
        let len = rng.uniform(25.0, 70.0) as i64;
        for f in from.max(0)..(from + len).min(FRAMES as i64) {
            // The row is read at a time that depends on the row
            let mut uv = [0.0, SIZE.1 as f64 / 2.0];
            let mut in_front = true;
            for _ in 0..6 {
                let (r, t) = pose(at[f as usize] + (uv[1] / SIZE.1 as f64 - 0.5) * readout, motion);
                let xc = r * x + t;
                in_front = xc.z > 0.5;
                uv = lens.project(&xc);
            }
            if !in_front || uv[0] < 8.0 || uv[1] < 8.0 || uv[0] > SIZE.0 as f64 - 8.0 || uv[1] > SIZE.1 as f64 - 8.0 { continue; }
            fragment.push(track, f as usize, uv[0] + noise * rng.normal(), uv[1] + noise * rng.normal());
        }
    }
    fragment.frames = FRAMES;
    fragment.times = at.iter().map(|t| t * fragment.frame_ms).collect();
    fragment
}

fn never() -> bool { false }

/// The synthetic fits are a few seconds each optimized and minutes not: a plain `cargo test` leaves them out
macro_rules! slow { ($(#[$m:meta])* fn $name:ident() $body:block) => {
    $(#[$m])* #[test] #[cfg_attr(debug_assertions, ignore = "a bundle adjustment: run with --release")] fn $name() $body
} }

fn off_by(lens: &Lens, scale: f64) -> Lens { Lens { fx: lens.fx * scale, fy: lens.fy * scale, ..*lens } }
fn candidate(name: &str, lens: Lens, loadable: bool) -> Candidate {
    Candidate { id: name.into(), how: How::AsItIs, loadable, rank: (0, 0, name.into(), name.into()), ..Candidate::plain(name, lens) }
}

slow! {
fn recovers_the_lens_and_the_readout_time() {
    let truth = medium();
    let tracks = clip(&truth, READOUT, true, 0.3).tracks();
    // From the profile of another mode of the camera: the focal length a quarter off, another distortion, the centre
    // in the middle of the frame
    let found = estimate(&tracks, &wide(), 1000.0 / 24.0, &never, &|_| ()).unwrap();
    let shift = truth.shift_after_tilt(&found.lens, SIZE);
    println!("{found:?} {shift:?}");
    assert!((found.lens.fx / truth.fx - 1.0).abs() < 0.01 && (found.lens.fy / truth.fy - 1.0).abs() < 0.01, "focal length {} {}", found.lens.fx, found.lens.fy);
    assert!((found.readout - READOUT).abs() < 0.05, "readout {}", found.readout);
    assert!((found.readout_ms - READOUT * 1000.0 / 24.0).abs() < 2.0);
    assert!(shift.median < 6.0 && shift.max < 15.0, "{shift:?}");
    assert!(found.error < 1.0 && found.error < found.error_before && found.firmness > 0.15, "{found:?}");
    assert!(doubts(&found).is_empty());

    let p = profile(&found, Some(found.readout_ms), SIZE, 24.0, false, ("GoPro", "HERO4 Black"), None, "Anton");
    // Marked as found from the video, and calibrated by whoever made it, as any profile is
    assert!(p.calibrated_by == "Anton" && p.is_estimated_from_footage() && p.asymmetrical && p.frame_readout_time.is_some_and(|x| (x - 30.8).abs() < 2.0));
    assert!(matches!(p.frame_readout_direction, Some(ReadoutDirection::TopToBottom)));
    // It loads as any profile saved to a file does, and gives the same lens back
    let mut loaded = LensProfile::default();
    loaded.load_from_data(&p.get_json().unwrap()).unwrap();
    assert!(loaded.is_estimated_from_footage());
    assert!(Lens::from_profile(&loaded).unwrap().shift(&found.lens, SIZE).max < 1e-6);
} }

/// A readout time goes into a profile as a length and a direction: from the bottom up when the fit found it negative
#[test]
fn the_readout_direction_is_the_sign() {
    let e = Estimate { lens: medium(), readout: 0.0, readout_ms: 0.0, scene: SceneInfo::default(), error_before: 0.0, error: 0.0, within_1px: 0.0, firmness: 0.0 };
    let of = |ms: Option<f64>| { let p = profile(&e, ms, SIZE, 24.0, false, ("", ""), None, ""); (p.frame_readout_time, p.frame_readout_direction.map(|d| d as i32)) };
    assert_eq!(of(Some(30.5)), (Some(30.5), Some(ReadoutDirection::TopToBottom as i32)));
    assert_eq!(of(Some(-30.5)), (Some(30.5), Some(ReadoutDirection::BottomToTop as i32)));
    assert_eq!(of(Some(0.01)), (None, None));
    assert_eq!(of(None), (None, None));
    assert!(readout_for_profile(Some(f64::NAN)).is_none());
}

slow! {
fn tells_the_right_profile_from_wrong_ones() {
    let truth = medium();
    let tracks = clip(&truth, READOUT, true, 0.3).tracks();
    let right = check_profile(&tracks, &truth, None, &never).unwrap();
    println!("right: {right:?}");
    assert!(right.fits && right.moved.median < 8.0 && (right.readout - READOUT).abs() < 0.05, "{right:?}");
    // The clip shakes: a lens 5% off walks away in the quick steps, so staying put through them meant something
    assert!(right.holds == Some(true) && !right.at_rest, "{right:?}");
    // With its readout time given
    assert!(check_profile(&tracks, &truth, Some(READOUT), &never).unwrap().fits);
    // The focal length 4% off, and the profile of another mode
    let off = check_profile(&tracks, &off_by(&truth, 1.04), None, &never).unwrap();
    println!("4% off: {off:?}");
    assert!(!off.fits && off.moved.median > 15.0, "{off:?}");
    let other = check_profile(&tracks, &wide(), None, &never).unwrap();
    println!("wide: {other:?}");
    assert!(!other.fits && !other.fits_loosely && other.moved.median > 40.0, "{other:?}");
    // The right lens with a wrong readout time
    let wrong_readout = check_profile(&tracks, &truth, Some(0.3), &never).unwrap();
    assert!(!wrong_readout.fits, "{wrong_readout:?}");
} }

slow! {
/// A steady clip - a tenth of the shake, straight ahead: a gimbal, a drone, a car mount - pulls on a wrong lens
/// slowly. In the quick steps a lens 15% off has moved under 4 px there, well within what "fits" allows, and so has
/// one 5% off: the fragment doesn't tell that way. It tells at rest: the lenses put off come back to one place, the
/// right lens is there and the wrong ones are not
fn a_steady_clip_still_tells() {
    let truth = medium();
    let tracks = clip_of(&truth, READOUT, STEADY, 0.3).tracks();
    let right = check_profile(&tracks, &truth, None, &never).unwrap();
    println!("right: {right:?}");
    assert!(right.fits && right.at_rest && right.holds == Some(true) && right.moved.median < 8.0, "{right:?}");
    for (name, lens) in [("+15%", off_by(&truth, 1.15)), ("-15%", off_by(&truth, 1.0 / 1.15)), ("+8%", off_by(&truth, 1.08)), ("+4%", off_by(&truth, 1.04)), ("the other mode's", wide())] {
        let check = check_profile(&tracks, &lens, None, &never).unwrap();
        println!("{name}: {check:?}");
        assert!(!check.fits && !check.fits_loosely && check.moved.median > 20.0, "{name}: {check:?}");
    }
    // Over a clip of such parts: found, and not the one 8% off that comes first
    let fragments = [clip_of(&truth, READOUT, STEADY, 0.3), clip_of(&truth, READOUT, STEADY, 0.5)];
    let known = find_known(&fragments, &[candidate("+8%", off_by(&truth, 1.08), true), candidate("right", truth, true)], &[1.0], 1.0, &never, &|_| ());
    assert_eq!(known.accepted, Some(1), "{known:?}");
    assert!(known.tried[0].matching().verdict == Verdict::Poor && known.tried[1].result().checks.iter().flatten().all(|c| c.at_rest && c.holds == Some(true)));
} }

slow! {
/// A camera that moves through space and never turns: the picture is the same for any focal length (the scene
/// stretches along the axis to match), so every lens stays put. Such a fragment has no say
fn a_fragment_that_cannot_tell_has_no_say() {
    let truth = medium();
    let slides = clip_of(&truth, 0.0, Motion { shake: 0.0, path: Path::Walk }, 0.3);
    let tracks = slides.tracks();
    let right = check_profile(&tracks, &truth, Some(0.0), &never).unwrap();
    let wrong = check_profile(&tracks, &off_by(&truth, 1.05), Some(0.0), &never).unwrap();
    println!("right: {right:?}\n5% off: {wrong:?}");
    assert!(right.fits_loosely && wrong.fits_loosely, "a lens stays put either way");
    assert!(right.holds == Some(false) && wrong.holds == Some(false), "and the fragment knows it can't tell");
    // Over a clip: the fragment is counted out, and with nothing else to go by the lens isn't found
    let known = find_known(&[slides.clone(), slides], &[candidate("off", off_by(&truth, 1.05), true)], &[1.0], 1.0, &never, &|_| ());
    let m = known.tried[0].matching();
    println!("{m:?}");
    assert!(known.accepted.is_none() && m.verdict == Verdict::Poor && m.fragments == 0 && m.fragments_unable == 2, "{m:?}");
} }

slow! {
fn finds_the_known_profile_over_fragments() {
    let truth = medium();
    // One fragment where the camera only turns: it has no say
    let fragments = [clip(&truth, READOUT, true, 0.3), clip(&truth, READOUT, false, 0.3), clip(&truth, READOUT, true, 0.5)];
    let known = find_known(&fragments, &[candidate("wide", wide(), true), candidate("medium", truth, true)], &[1.0], 1.0, &never, &|_| ());
    assert_eq!(known.accepted, Some(1), "{known:?}");
    println!("wide: {:?}", known.tried[0].result().checks);
    let medium = &known.tried[1];
    let m = medium.matching();
    assert!(medium.accepted && m.verdict == Verdict::Good && m.fragments_fitting == 2 && m.fragments == 2 && m.fragments_unable == 0 && medium.result().checks[1].is_none(), "{medium:?}");
    assert!(m.readout_measured && m.readout_ms.is_some_and(|x| (x - 30.8).abs() < 2.5), "{m:?}");
    assert!(medium.result().checks.iter().flatten().all(|c| c.holds == Some(true)));
    assert!(m.moved < m.limit && known.tried[0].matching().verdict == Verdict::Poor);
    assert_eq!(known.best(), Some(1));
    // A lens that isn't a profile of this frame as it is - one brought to the frame, or a plain start - still wins
    // when it fits; what it can't be is loaded by its id, and the caller makes a profile of it
    let known = find_known(&fragments, &[candidate("medium", truth, false)], &[1.0], 1.0, &never, &|_| ());
    assert_eq!(known.accepted, Some(0));
    // A profile's own readout time is tried as it is, and isn't called measured
    let with_readout = Candidate { readout_ms: Some(READOUT * 1000.0 / 24.0), ..candidate("medium", truth, true) };
    let known = find_known(&fragments, &[with_readout], &[1.0], 1.0, &never, &|_| ());
    assert!(known.accepted == Some(0) && !known.tried[0].matching().readout_measured, "{known:?}");
} }

slow! {
/// The first profile that fits isn't taken for the lens: one 1% off, earlier in the list, is within the limit, and
/// the exact one comes after it
fn the_nearest_profile_wins_not_the_first() {
    let truth = medium();
    let fragments = [clip(&truth, READOUT, true, 0.3), clip(&truth, READOUT, true, 0.5)];
    let near = off_by(&truth, 1.01);
    let alone = find_known(&fragments, &[candidate("near", near, true)], &[1.0], 1.0, &never, &|_| ());
    println!("1% off, alone: {:?}", alone.tried[0].matching());
    assert_eq!(alone.accepted, Some(0), "within the limit by itself: {alone:?}");
    let known = find_known(&fragments, &[candidate("near", near, true), candidate("wide", wide(), true), candidate("exact", truth, true)], &[1.0], 1.0, &never, &|_| ());
    println!("1% off: {:?}\nexact: {:?}", known.tried[0].matching(), known.tried[2].matching());
    assert_eq!(known.accepted, Some(2), "{known:?}");
    assert!(known.tried.len() == 3 && known.tried[2].matching().moved < known.tried[0].matching().moved);
} }

slow! {
/// One part of a clip is one fragment's word: the lens that fits it is offered, and not as found
fn one_part_is_not_enough() {
    let truth = medium();
    let known = find_known(&[clip(&truth, READOUT, true, 0.3)], &[candidate("medium", truth, true)], &[1.0], 1.0, &never, &|_| ());
    let m = known.tried[0].matching();
    assert!(known.accepted.is_none() && m.verdict == Verdict::Single && m.fragments_fitting == 1 && m.fragments == 1, "{m:?}");
    assert_eq!(known.best(), Some(0));
} }

slow! {
/// With the lens known, the picture tells water from air: refraction can no longer be traded for the focal length, and
/// the lens tried in the wrong medium walks away in the corners. Nobody has to say which it was
fn a_known_lens_finds_its_medium() {
    let truth = medium();
    let candidate = candidate("medium", truth, true);
    let wet = [clip(&truth.with_refraction(1.33), READOUT, true, 0.3), clip(&truth.with_refraction(1.33), READOUT, true, 0.5)];
    let known = find_known(&wet, &[candidate.clone()], &[1.0, 1.33], 1.0, &never, &|_| ());
    let r = &known.tried[0];
    println!("under water: {:?}", r.media.iter().map(|m| (m.refraction, m.matching)).collect::<Vec<_>>());
    assert!(r.accepted && r.refraction() == 1.33 && r.medium_sure, "{r:?}");
    let dry = [clip(&truth, READOUT, true, 0.3), clip(&truth, READOUT, true, 0.5)];
    // Whichever is in use
    for current in [1.0, 1.33] {
        let known = find_known(&dry, &[candidate.clone()], &[1.0, 1.33], current, &never, &|_| ());
        let r = &known.tried[0];
        println!("in air: {:?}", r.media.iter().map(|m| (m.refraction, m.matching)).collect::<Vec<_>>());
        assert!(r.accepted && r.refraction() == 1.0 && r.medium_sure, "{r:?}");
    }
    // An estimate is judged the same way, in the medium it was made for
    assert_eq!(check_lens(&dry, &truth, None, &never).verdict, Verdict::Good);
    assert_eq!(check_lens(&dry, &off_by(&truth, 1.05), None, &never).verdict, Verdict::Poor);
} }

/// An even vote between the media changes nothing: it goes to the one in use, and isn't called sure
#[test]
fn a_tie_goes_to_the_medium_in_use() {
    let check = |corner: f64| ProfileCheck {
        scene: SceneInfo { parallax: 2.0, points: 600, ..Default::default() }, moved: LensShift { median: 3.0, max: corner, corner },
        readout: 0.7, readout_free: 0.7, error: 0.5, error_free: 0.5, fits: true, fits_loosely: true, holds: Some(true), at_rest: false,
    };
    let medium = |n: f64, corners: &[f64]| MediumResult::new(n, corners.iter().map(|c| Some(check(*c))).collect(), &[41.7; 2], 1.0, true);
    // One fragment each way, the same weight
    let even = [medium(1.0, &[4.0, 9.0]), medium(1.33, &[9.0, 4.0])];
    assert_eq!(choose_medium(&even, Some(0)), (0, false));
    assert_eq!(choose_medium(&even, Some(1)), (1, false));
    assert_eq!(choose_medium(&even, None), (0, false));
    // Corners that moved the same are no vote at all
    let same = [medium(1.0, &[4.0, 4.0]), medium(1.33, &[4.0, 4.0])];
    assert_eq!(choose_medium(&same, Some(1)), (1, false));
    // A clear vote is a clear vote, whatever is in use
    let clear = [medium(1.0, &[9.0, 9.0]), medium(1.33, &[4.0, 4.0])];
    assert_eq!(choose_medium(&clear, Some(0)), (1, true));
}

slow! {
fn under_water_gives_the_lens_in_air() {
    let truth = medium();
    let tracks = clip(&truth.with_refraction(1.33), READOUT, true, 0.3).tracks();
    let wet = estimate(&tracks, &wide().with_refraction(1.33), 1000.0 / 24.0, &never, &|_| ()).unwrap();
    println!("{wet:?}");
    assert!((wet.lens.fx / truth.fx - 1.0).abs() < 0.015, "focal length {}", wet.lens.fx);
    let p = profile(&wet, Some(wet.readout_ms), SIZE, 24.0, true, ("", ""), None, "");
    assert!((p.fisheye_params.camera_matrix[0][0] / truth.fx - 1.0).abs() < 0.015);
    // The known air lens fits the underwater clip with the refraction in the model, and not without
    assert!(check_profile(&tracks, &truth.with_refraction(1.33), None, &never).unwrap().fits);
    assert!(!check_profile(&tracks, &truth, None, &never).unwrap().fits);
} }

slow! {
fn refuses_a_camera_that_only_turns() {
    let truth = medium();
    let tracks = clip(&truth, READOUT, false, 0.3).tracks();
    assert_eq!(check_profile(&tracks, &truth, None, &never).err(), Some(NoScene::NoTranslation));
    assert_eq!(estimate(&tracks, &wide(), 1000.0 / 24.0, &never, &|_| ()).err(), Some(NoScene::NoTranslation));
    // And a fragment with next to nothing tracked
    let mut few = Fragment::new(SIZE, 40.0);
    for f in 0..30 { for t in 0..20u32 { few.push(t, f, 100.0 + t as f64 * 50.0, 300.0 + f as f64); } }
    assert_eq!(check_profile(&few.tracks(), &truth, None, &never).err(), Some(NoScene::TooFewTracks));
} }

slow! {
/// The frames carry their own time, and the fit takes the camera's rates over the time between the frames it has:
/// told the frames are twice as far apart as they are (a frame of the video missing between each pair), it finds those
/// rates halved and so twice the readout time, in intervals that are themselves twice as long
fn the_frames_own_times_drive_the_rolling_shutter() {
    let truth = medium();
    let mut doubled = clip(&truth, READOUT, true, 0.3);
    doubled.times = (0..FRAMES).map(|i| 2.0 * i as f64 * doubled.frame_ms).collect();
    let check = check_profile(&doubled.tracks(), &truth, None, &never).unwrap();
    println!("{check:?}");
    // Within a few per cent of twice it: the camera's path is curved, and rates taken over twice the interval are a
    // slightly coarser measure of it
    assert!((check.readout / (2.0 * READOUT) - 1.0).abs() < 0.06, "{check:?}");
    assert!(check.fits, "{check:?}");
} }

slow! {
/// A clip whose frame rate varies, a phone's: its frames are where their timestamps say, up to a third of an
/// interval early or late. Given those times the fit knows how far the camera moved between two frames; told the
/// frames are even, its rates are wrong by that much, and the tracks it hasn't seen fit that much worse
fn a_varying_frame_rate_is_followed() {
    let truth = medium();
    let mut rng = Rng(0x2545F4914F6CDD1D);
    let jitter: Vec<f64> = (0..FRAMES).map(|i| i as f64 + rng.uniform(-0.33, 0.33)).collect();
    let mut uneven = clip_at(&truth, READOUT, WALKS, 0.3, &jitter);
    let check = check_profile(&uneven.tracks(), &truth, None, &never).unwrap();
    println!("with its times: {check:?}");
    assert!(check.fits && (check.readout - READOUT).abs() < 0.05, "{check:?}");
    uneven.times = (0..FRAMES).map(|i| i as f64 * uneven.frame_ms).collect();
    let even = check_profile(&uneven.tracks(), &truth, None, &never).unwrap();
    println!("taken for even: {even:?}");
    assert!(even.error > 1.3 * check.error, "{} px with its times, {} px taken for even", check.error, even.error);
} }

#[test]
fn fragments_of_a_clip() {
    // 24 fps: 100 frames are 4.17 s
    let f = automatic_fragments(60_000.0, 24.0);
    assert_eq!(f.len(), 3);
    assert!((f[0].0 - 1000.0).abs() < 1.0 && (f[1].0 + f[1].1 - 60_000.0).abs() < 1.0 && (f[2].0 + f[2].1) / 2.0 == 48_000.0, "{f:?}");
    assert!(f.iter().all(|x| ((x.1 - x.0) - 4166.7).abs() < 1.0));
    // 120 fps: every 4th frame, the same stretch of time
    assert_eq!(frame_step(120.0), 4);
    assert!(((automatic_fragments(60_000.0, 120.0)[0].1 - 1000.0) - 3333.3).abs() < 1.0);
    // Too short for three, and for more than one
    assert_eq!(automatic_fragments(9_000.0, 24.0).len(), 2);
    assert_eq!(automatic_fragments(4_500.0, 24.0), vec![(0.0, 4_500.0)]);
    // From the playhead: as many frames as asked for, moved back where the clip ends sooner
    let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
    assert!(near(fragment_from(2_000.0, 60_000.0, 24.0, MAX_FRAMES), (2_000.0, 14_500.0)));
    let len = CHECK_FRAMES as f64 * 1000.0 / 24.0;
    assert!(near(fragment_from(58_000.0, 60_000.0, 24.0, CHECK_FRAMES), (60_000.0 - len, 60_000.0)));
    assert_eq!(fragment_from(1_000.0, 5_000.0, 24.0, MAX_FRAMES), (0.0, 5_000.0));
}

/// An estimate is checked on parts of the clip it wasn't made from
#[test]
fn an_estimate_is_checked_elsewhere() {
    let parts = [(1_000.0, 5_166.7), (27_916.7, 32_083.3), (45_916.7, 50_083.3)];
    // Made from the first 12.5 s of a minute: the first part is inside it, the other two aren't
    assert_eq!(apart_from(&parts, (0.0, 12_500.0)), vec![1, 2]);
    // From the playhead in the middle
    assert_eq!(apart_from(&parts, (25_000.0, 37_500.0)), vec![0, 2]);
    // A clip no longer than the estimate takes: nothing is left to check it on
    assert_eq!(apart_from(&automatic_fragments(11_700.0, 24.0), fragment_from(0.0, 11_700.0, 24.0, MAX_FRAMES)), Vec::<usize>::new());
    // Touching is apart; a frame shared isn't
    assert_eq!(apart_from(&[(0.0, 1000.0), (1000.0, 2000.0), (1990.0, 3000.0)], (1000.0, 2000.0)), vec![0]);
}

#[test]
fn builder_keeps_whole_tracks() {
    use crate::synchronization::optical_motion::Observation;
    let mut b = FragmentBuilder::new((1920, 1080), (960, 540), 40.0);
    b.push_frame(1000.0, &[]);
    b.push_frame(1040.0, &[Observation { id: 7, a: [10.0, 20.0], b: [11.0, 21.0] }]);
    b.push_frame(1080.0, &[Observation { id: 7, a: [11.0, 21.0], b: [12.0, 22.0] }]);
    // A frame of the video missing: this one is two frame intervals after the one before
    b.push_frame(1160.0, &[Observation { id: 7, a: [12.0, 22.0], b: [13.0, 23.0] }, Observation { id: 9, a: [0.0, 0.0], b: [1.0, 1.0] }]);
    let f = b.finish();
    assert_eq!(f.frames, 4);
    assert_eq!(f.track, vec![7, 7, 7, 7, 9, 9]);
    assert_eq!(f.frame, vec![0, 1, 2, 3, 2, 3]);
    assert_eq!(f.uv[0], [20.5, 40.5]);
    // The interval is what's between most frames, and the times are counted in it from the first
    assert_eq!(f.frame_ms, 40.0);
    let mut long = Fragment::new((1920, 1080), 40.0);
    for t in 0..2u32 { for frame in 0..MIN_TRACK { long.push(t, frame, 10.0, 10.0); } }
    long.times = vec![1000.0, 1040.0, 1080.0, 1160.0, 1200.0, 1242.0, 1280.0, 1320.0, 1360.0, 1400.0];
    assert_eq!(long.tracks().times, vec![0.0, 1.0, 2.0, 4.0, 5.0, 6.05, 7.0, 8.0, 9.0, 10.0]);
}

/// What the render applies frame by frame and the fit doesn't model is refused, by name
#[test]
fn what_the_render_adds_is_refused() {
    use crate::gyro_source::{ FileMetadata, LensParams };
    let mut md = FileMetadata::default();
    assert_eq!(unmodelled(&md), None);
    // A GoPro's zoom at its widest is no zoom
    md.digital_zoom = Some(1.0);
    assert_eq!(unmodelled(&md), None);
    md.digital_zoom = Some(1.4);
    assert_eq!(unmodelled(&md), Some("digital_zoom"));
    md.digital_zoom = None;
    md.lens_params.insert(0, LensParams { pixel_focal_length: Some((1400.0, 1400.0)), ..Default::default() });
    assert_eq!(unmodelled(&md), Some("lens_metadata"));
    // Metadata that only describes the shot (the focus distance, the iris) changes nothing the render draws
    md.lens_params.clear();
    md.lens_params.insert(0, LensParams { focus_distance: Some(2.0), ..Default::default() });
    assert_eq!(unmodelled(&md), None);
}

/// Profiles made here keep to what they know: an estimate is its maker's and is named after the clip's camera; a
/// known lens brought to the frame is still its calibrator's
#[test]
fn profiles_say_where_they_come_from() {
    let mut source = wide().to_profile((3840, 2160));
    source.camera_brand = "GoPro".into(); source.camera_model = "HERO4 Black".into(); source.lens_model = "Wide".into();
    source.camera_setting = "4K Wide".into(); source.calibrated_by = "Somebody".into(); source.crop_factor = Some(5.6);
    source.name = "GoPro HERO4 Black 4K".into();
    let lens = Lens::from_profile(&source).unwrap();
    let cropped = Candidate { how: How::Cropped, profile: Some(source.clone()), ..candidate("GoPro HERO4 Black 4K", Lens { cx: lens.cx - 568.0, cy: lens.cy - 320.0, ..lens }, false) };
    let p = brought_profile(&cropped, SIZE, 24.0).unwrap();
    assert!(!p.is_estimated_from_footage() && p.calibrated_by == "Somebody" && p.note.contains("GoPro HERO4 Black 4K") && p.note.contains("3840x2160"), "{p:?}");
    assert!(p.camera_setting.is_empty() && (p.crop_factor.unwrap() - 5.6 * 3840.0 / 2704.0).abs() < 1e-9 && p.frame_readout_time.is_none());
    assert_eq!((p.calib_dimension.w, p.calib_dimension.h), SIZE);
    assert!(Lens::from_profile(&p).unwrap().shift(&cropped.lens, SIZE).max < 1e-6);
    let scaled = Candidate { how: How::Scaled, profile: Some(source.clone()), ..candidate("GoPro HERO4 Black 4K", lens.scaled(2704.0 / 3840.0), false) };
    assert_eq!(brought_profile(&scaled, SIZE, 24.0).unwrap().crop_factor, Some(5.6));
    // A profile as it is isn't brought from anywhere, and a plain start is no profile
    assert!(brought_profile(&candidate("as it is", lens, true), SIZE, 24.0).is_none());
    assert!(brought_profile(&Candidate::plain("Generic fisheye", lens), SIZE, 24.0).is_none());

    let e = Estimate { lens: medium(), readout: 0.74, readout_ms: 30.8, scene: SceneInfo::default(), error_before: 2.0, error: 0.5, within_1px: 0.9, firmness: 0.3 };
    // From a profile of the same camera: its lens name, not the setting of its mode, nor the crop factor of another frame
    let p = profile(&e, Some(30.8), SIZE, 24.0, false, ("GoPro", "HERO4 Black"), Some(&source), "Anton");
    assert!(p.is_estimated_from_footage() && p.calibrated_by == "Anton" && p.lens_model == "Wide" && p.camera_setting.is_empty() && p.crop_factor.is_none(), "{p:?}");
    // A loaded profile of some other camera says nothing of this one's lens
    let p = profile(&e, Some(30.8), SIZE, 24.0, false, ("DJI", "Osmo Action 6"), Some(&source), "Anton");
    assert!(p.camera_brand == "DJI" && p.camera_model == "Osmo Action 6" && p.lens_model.is_empty(), "{p:?}");
    // A file that names no camera takes the profile's word
    let p = profile(&e, Some(30.8), SIZE, 24.0, false, ("", ""), Some(&source), "Anton");
    assert!(p.camera_brand == "GoPro" && p.lens_model == "Wide");
}

/// The lens as the render draws it: a profile made here, loaded as any profile is, read back by
/// `FrameTransform::get_lens_data_at_timestamp` and drawn by `Stabilization::rotate_and_distort`, lands where
/// `Lens::project` says - in air, under water and without coefficients (a pinhole). The render draws in `f32`: at the
/// edge of a 2.7K frame its last bit is 0.0002 px
#[test]
fn the_render_draws_the_same_lens() {
    use crate::{ StabilizationManager, stabilization::{ ComputeParams, FrameTransform, KernelParams, Stabilization, projection } };
    let e = Estimate { lens: medium(), readout: 0.74, readout_ms: 30.8, scene: SceneInfo::default(), error_before: 2.0, error: 0.5, within_1px: 0.9, firmness: 0.3 };
    let f = (SIZE.0 as f64).hypot(SIZE.1 as f64) * 16.0 / 43.27;
    let pinhole = Lens::new_pinhole(f, f, 1340.0, 770.0);
    let cases = [
        ("an estimate, in air", profile(&e, Some(30.8), SIZE, 24.0, false, ("", ""), None, "test"), medium()),
        ("an estimate made under water", profile(&Estimate { lens: medium().with_refraction(1.33), ..e }, Some(30.8), SIZE, 24.0, true, ("", ""), None, "test"), medium().with_refraction(1.33)),
        ("no coefficients: a pinhole", { let mut p = pinhole.to_profile(SIZE); p.calibrated_by = "test".into(); p.finalize(); p }, pinhole),
        ("a pinhole under water", { let mut p = pinhole.to_profile(SIZE); p.calibrated_by = "test".into(); p.finalize(); p }, pinhole.with_refraction(1.33)),
    ];
    for (name, profile, lens) in cases {
        let stab = StabilizationManager::default();
        stab.init_from_video_data(10_000.0, 24.0, 240, SIZE);
        stab.set_output_size(SIZE.0, SIZE.1);
        stab.load_lens_profile(&profile.get_json().unwrap()).unwrap();
        stab.set_light_refraction_coefficient(lens.refraction);
        let params = ComputeParams::from_manager(&stab);
        let (camera_matrix, coeffs, field_limit, h_stretch, v_stretch, ..) = FrameTransform::get_lens_data_at_timestamp(&params, 0.0, false);
        let mut kp = KernelParams {
            f: [camera_matrix[(0, 0)] as f32, camera_matrix[(1, 1)] as f32],
            c: [camera_matrix[(0, 2)] as f32, camera_matrix[(1, 2)] as f32],
            field_limit: field_limit as f32,
            input_horizontal_stretch: h_stretch as f32, input_vertical_stretch: v_stretch as f32,
            light_refraction_coefficient: params.light_refraction_coefficient as f32,
            ..Default::default()
        };
        for (i, v) in coeffs.iter().enumerate().take(kp.k.len()) { kp.k[i] = *v as f32; }
        let mut identity = [0.0f32; 16];
        (identity[0], identity[4], identity[8]) = (1.0, 1.0, 1.0);
        let mut worst: f64 = 0.0;
        let mut drawn = 0;
        // Rays out to the corners of the frame
        for (x, y) in (0..63).map(|i| (-0.78 + 0.26 * (i % 7) as f64, -0.44 + 0.11 * (i / 7) as f64)) {
            let ray = nalgebra::Vector3::new(x, y, 1.0);
            let expected = lens.project(&ray);
            if expected[0] < 0.0 || expected[1] < 0.0 || expected[0] > SIZE.0 as f64 || expected[1] > SIZE.1 as f64 { continue; }
            let uv = Stabilization::rotate_and_distort(projection::dir_to_ray((x as f32, y as f32, 1.0)), 0, &kp, &[identity], &params.distortion_model, params.digital_lens.as_ref(), &[]).unwrap_or_else(|| panic!("{name}: the render has no image of {ray:?}"));
            worst = worst.max((uv.0 as f64 - expected[0]).hypot(uv.1 as f64 - expected[1]));
            drawn += 1;
        }
        println!("{name}: {worst:.5} px over {drawn} rays");
        assert!(drawn >= 15 && worst < 1e-3, "{name}: {worst} px apart over {drawn} rays");
    }
}
