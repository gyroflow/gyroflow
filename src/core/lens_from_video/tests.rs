// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Anton Belkin

//! Against clips whose truth is known: points of a static scene seen by a camera that walks and shakes, drawn through
//! a lens with a rolling shutter (each point at the time its row is read, on the camera's continuous path), with noise.
//! Run with --release: the fits are slow in a debug build.

use super::*;
use nalgebra::{ Rotation3, Vector3 };

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

/// The camera at a time in frames: world → camera. It shakes either way; `walks`: it also moves through space.
/// Nothing faster than a turn in 12 frames: the fit knows the camera's rates from the frames next to each one
fn pose(t: f64, walks: bool) -> (Rotation3<f64>, Vector3<f64>) {
    let r = Rotation3::new(Vector3::new(
        0.04 * (0.5 * t).sin() + 0.03 * (0.31 * t + 0.5).sin(),
        0.05 * (0.37 * t + 2.0).sin() + 0.03 * (0.55 * t).sin(),
        0.02 * (0.29 * t).sin()));
    let centre = if walks { Vector3::new(0.15 * (0.21 * t).sin(), 0.08 * (0.33 * t + 1.0).sin(), 0.04 * t) } else { Vector3::zeros() };
    (r, -(r * centre))
}

fn clip(lens: &Lens, readout: f64, walks: bool, noise: f64) -> Fragment {
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
                let (r, t) = pose(f as f64 + (uv[1] / SIZE.1 as f64 - 0.5) * readout, walks);
                let xc = r * x + t;
                in_front = xc.z > 0.5;
                uv = lens.project(&xc);
            }
            if !in_front || uv[0] < 8.0 || uv[1] < 8.0 || uv[0] > SIZE.0 as f64 - 8.0 || uv[1] > SIZE.1 as f64 - 8.0 { continue; }
            fragment.push(track, f as usize, uv[0] + noise * rng.normal(), uv[1] + noise * rng.normal());
        }
    }
    fragment.frames = FRAMES;
    fragment
}

fn never() -> bool { false }

#[test]
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

    let p = profile(&found, SIZE, 24.0, false, ("GoPro", "HERO4 Black"), None, "Anton");
    // Marked as found from the video, and calibrated by whoever made it, as any profile is
    assert!(p.calibrated_by == "Anton" && p.is_estimated_from_footage() && p.asymmetrical && p.frame_readout_time.is_some_and(|x| (x - 30.8).abs() < 2.0));
    // It loads as any profile saved to a file does, and gives the same lens back
    let mut loaded = LensProfile::default();
    loaded.load_from_data(&p.get_json().unwrap()).unwrap();
    assert!(loaded.is_estimated_from_footage());
    assert!(Lens::from_profile(&loaded).unwrap().shift(&found.lens, SIZE).max < 1e-6);
}

#[test]
fn tells_the_right_profile_from_wrong_ones() {
    let truth = medium();
    let tracks = clip(&truth, READOUT, true, 0.3).tracks();
    let right = check_profile(&tracks, &truth, None, &never).unwrap();
    println!("right: {right:?}");
    assert!(right.fits && right.moved.median < 6.0 && (right.readout - READOUT).abs() < 0.05, "{right:?}");
    // With its readout time given
    assert!(check_profile(&tracks, &truth, Some(READOUT), &never).unwrap().fits);
    // The focal length 4% off, and the profile of another mode
    let mut off = truth; off.fx *= 1.04; off.fy *= 1.04;
    let off = check_profile(&tracks, &off, None, &never).unwrap();
    println!("4% off: {off:?}");
    assert!(!off.fits && off.moved.median > 15.0, "{off:?}");
    let other = check_profile(&tracks, &wide(), None, &never).unwrap();
    println!("wide: {other:?}");
    assert!(!other.fits && !other.fits_loosely && other.moved.median > 40.0, "{other:?}");
    // The right lens with a wrong readout time
    let wrong_readout = check_profile(&tracks, &truth, Some(0.3), &never).unwrap();
    assert!(!wrong_readout.fits, "{wrong_readout:?}");
}

#[test]
fn finds_the_known_profile_over_fragments() {
    let truth = medium();
    // One fragment where the camera only turns: it has no say
    let fragments = [clip(&truth, READOUT, true, 0.3), clip(&truth, READOUT, false, 0.3), clip(&truth, READOUT, true, 0.5)];
    let candidate = |name: &str, lens: Lens, loadable: bool| Candidate { name: name.into(), id: name.into(), lens, readout_ms: None, loadable, is_loaded: false, profile: None, rank: (0, 0, name.into(), name.into()) };
    let known = find_known(&fragments, &[candidate("wide", wide(), true), candidate("medium", truth, true)], &[1.0], &never, &|_| ());
    assert_eq!(known.accepted, Some(1), "{known:?}");
    assert_eq!(known.fragments_with_scene, 2);
    let medium = &known.tried[1];
    let m = medium.matching();
    assert!(m.verdict == Verdict::Good && m.fragments_fitting == 2 && m.fragments == 2 && medium.result().checks[1].is_none() && m.readout_ms.is_some_and(|x| (x - 30.8).abs() < 2.5), "{medium:?}");
    assert!(m.moved < m.limit && known.tried[0].matching().verdict == Verdict::Poor);
    assert_eq!(known.best(), Some(1));
    // A lens that isn't a profile of this frame as it is - one brought to the frame, or a plain start - still wins
    // when it fits; what it can't be is loaded by its id, and the caller makes a profile of it
    let known = find_known(&fragments[..1], &[candidate("medium", truth, false)], &[1.0], &never, &|_| ());
    assert_eq!(known.accepted, Some(0));
    assert!(known.tried[0].matching().fragments_fitting == 1);
}

/// With the lens known, the picture tells water from air: refraction can no longer be traded for the focal length, and
/// the lens tried in the wrong medium walks away in the corners. Nobody has to say which it was
#[test]
fn a_known_lens_finds_its_medium() {
    let truth = medium();
    let candidate = Candidate { name: "medium".into(), id: "medium".into(), lens: truth, readout_ms: None, loadable: true, is_loaded: false, profile: None, rank: (0, 0, String::new(), String::new()) };
    let wet = [clip(&truth.with_refraction(1.33), READOUT, true, 0.3)];
    let known = find_known(&wet, &[candidate.clone()], &[1.0, 1.33], &never, &|_| ());
    let r = &known.tried[0];
    println!("under water: {:?}", r.media.iter().map(|m| (m.refraction, m.matching)).collect::<Vec<_>>());
    assert!(r.accepted && r.refraction() == 1.33 && r.medium_sure, "{r:?}");
    let dry = [clip(&truth, READOUT, true, 0.3)];
    let known = find_known(&dry, &[candidate], &[1.0, 1.33], &never, &|_| ());
    let r = &known.tried[0];
    println!("in air: {:?}", r.media.iter().map(|m| (m.refraction, m.matching)).collect::<Vec<_>>());
    assert!(r.accepted && r.refraction() == 1.0 && r.medium_sure, "{r:?}");
    // An estimate is judged the same way, in the medium it was made for
    assert_eq!(check_lens(&dry, &truth, None, &never).verdict, Verdict::Good);
    let mut off = truth; off.fx *= 1.05; off.fy *= 1.05;
    assert_eq!(check_lens(&dry, &off, None, &never).verdict, Verdict::Poor);
}

#[test]
fn under_water_gives_the_lens_in_air() {
    let truth = medium();
    let tracks = clip(&truth.with_refraction(1.33), READOUT, true, 0.3).tracks();
    let wet = estimate(&tracks, &wide().with_refraction(1.33), 1000.0 / 24.0, &never, &|_| ()).unwrap();
    println!("{wet:?}");
    assert!((wet.lens.fx / truth.fx - 1.0).abs() < 0.015, "focal length {}", wet.lens.fx);
    let p = profile(&wet, SIZE, 24.0, true, ("", ""), None, "");
    assert!((p.fisheye_params.camera_matrix[0][0] / truth.fx - 1.0).abs() < 0.015);
    // The known air lens fits the underwater clip with the refraction in the model, and not without
    assert!(check_profile(&tracks, &truth.with_refraction(1.33), None, &never).unwrap().fits);
    assert!(!check_profile(&tracks, &truth, None, &never).unwrap().fits);
}

#[test]
fn refuses_a_camera_that_only_turns() {
    let truth = medium();
    let tracks = clip(&truth, READOUT, false, 0.3).tracks();
    assert_eq!(check_profile(&tracks, &truth, None, &never).err(), Some(NoScene::NoTranslation));
    assert_eq!(estimate(&tracks, &wide(), 1000.0 / 24.0, &never, &|_| ()).err(), Some(NoScene::NoTranslation));
    // And a fragment with next to nothing tracked
    let mut few = Fragment::new(SIZE, 40.0);
    for f in 0..30 { for t in 0..20u32 { few.push(t, f, 100.0 + t as f64 * 50.0, 300.0 + f as f64); } }
    assert_eq!(check_profile(&few.tracks(), &truth, None, &never).err(), Some(NoScene::TooFewTracks));
}

/// The frames carry their own time, and the fit takes the camera's rates over the time between the frames it has:
/// told the frames are twice as far apart as they are (a frame of the video missing between each pair), it finds those
/// rates halved and so twice the readout time, in intervals that are themselves twice as long
#[test]
fn the_frames_own_times_drive_the_rolling_shutter() {
    let truth = medium();
    let mut doubled = clip(&truth, READOUT, true, 0.3);
    doubled.times = (0..FRAMES).map(|i| 2.0 * i as f64).collect();
    let check = check_profile(&doubled.tracks(), &truth, None, &never).unwrap();
    println!("{check:?}");
    // Within a few per cent of twice it: the camera's path is curved, and rates taken over twice the interval are a
    // slightly coarser measure of it
    assert!((check.readout / (2.0 * READOUT) - 1.0).abs() < 0.06, "{check:?}");
    assert!(check.fits, "{check:?}");
}

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

#[test]
fn builder_keeps_whole_tracks() {
    use crate::synchronization::optical_motion::Observation;
    let mut b = FragmentBuilder::new((1920, 1080), (960, 540), 40.0);
    b.push_frame(0, &[]);
    b.push_frame(1, &[Observation { id: 7, a: [10.0, 20.0], b: [11.0, 21.0] }]);
    // A frame of the video missing: this one is two frame intervals after the one before
    b.push_frame(3, &[Observation { id: 7, a: [11.0, 21.0], b: [12.0, 22.0] }, Observation { id: 9, a: [0.0, 0.0], b: [1.0, 1.0] }]);
    let f = b.finish();
    assert_eq!(f.frames, 3);
    assert_eq!(f.track, vec![7, 7, 7, 9, 9]);
    assert_eq!(f.frame, vec![0, 1, 2, 1, 2]);
    assert_eq!(f.uv[0], [20.5, 40.5]);
    assert_eq!(f.tracks().times, vec![0.0, 1.0, 3.0]);
}
