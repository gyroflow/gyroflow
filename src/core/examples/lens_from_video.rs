// "Find the lens from the video" end to end on a video, without the GUI: the frames come from ffmpeg (scaled, gray),
// are tracked as the app tracks them, and go through the same steps - the known profiles of the camera on three
// fragments, then an estimate of a fragment, checked on the fragments it wasn't made from.
// Not the app's own path to the frames: the app decodes and scales them itself, and times each by the video's own
// timestamps, where the frames here are ffmpeg's (`scale=...:flags=area`) at the nominal frame rate. The numbers of
// the two differ by what that is worth.
// cargo run --release --manifest-path <abs>/src/core/Cargo.toml --example lens_from_video -- <video> [options]
//   --camera <brand> <model>   the camera, for its profiles in the database (resources/camera_presets of the working directory)
//   --lens <profile.json>      the profile that's loaded: tried first, and what the estimate starts from
//   --water                    shot under water behind a flat port
//   --from <s> --to <s>        the fragment to estimate (the first 300 tracked frames of the clip without)
//   --estimate                 estimate even if a known profile fits
//   --air                      shot in air: without either, a known profile is tried both ways
//   --width <px>               width the frames are tracked at (1920, `lens_from_video::TRACKED_WIDTH`)
//   --out <profile.json>       where to write the estimated profile

use gyroflow_core::lens_profile::LensProfile;
use gyroflow_core::gyro_source::TrackingMethod;
use gyroflow_core::lens_from_video::{ self as lfv, Fragment, FragmentBuilder, Lens };
use gyroflow_core::lens_profile_database::LensProfileDatabase;
use gyroflow_core::synchronization::optical_motion::{ KltDevice, Tracker };
use std::io::Read;
use std::process::{ Command, Stdio };
use std::time::Instant;

fn track(video: &str, size: (usize, usize), fps: f64, range_ms: (f64, f64), width: u32) -> Fragment {
    let step = lfv::frame_step(fps);
    let tw = width.min(size.0 as u32);
    let th = (((size.1 as f64 * tw as f64 / size.0 as f64) / 2.0).round() * 2.0) as u32;
    let frames = (((range_ms.1 - range_ms.0) / 1000.0 * fps) / step as f64).floor().max(1.0) as usize;
    let mut ff = Command::new(std::env::var("FFMPEG_EXE").unwrap_or("ffmpeg".into()))
        .args(["-v", "error", "-ss", &format!("{:.3}", range_ms.0 / 1000.0), "-i", video, "-vf", &format!("select=not(mod(n\\,{step})),scale={tw}:{th}:flags=area"), "-fps_mode", "passthrough", "-frames:v", &frames.to_string(), "-pix_fmt", "gray", "-f", "rawvideo", "-"])
        .stdout(Stdio::piped()).spawn().expect("ffmpeg");
    let mut out = ff.stdout.take().unwrap();
    let mut buf = vec![0u8; (tw * th) as usize];
    let mut tracker = Tracker::new(lfv::TRACKED_POINTS, TrackingMethod::Klt, if cfg!(feature = "use-opencv") && std::env::var("KLT_CPU").is_ok() { KltDevice::Cpu } else { KltDevice::Gpu(None) }).expect("tracker");
    let interval = step as f64 * 1000.0 / fps;
    let mut builder = FragmentBuilder::new(size, (tw, th), interval);
    let mut slot = 0;
    while out.read_exact(&mut buf).is_ok() {
        // ffmpeg hands the frames over without their times: taken at the nominal frame rate
        builder.push_frame(range_ms.0 + slot as f64 * interval, &tracker.track(tw, th, tw as usize, &buf).expect("tracking"));
        slot += 1;
    }
    let _ = ff.wait();
    eprintln!("  {:.1}-{:.1} s: {} frames, {}", range_ms.0 / 1000.0, range_ms.1 / 1000.0, builder.frames(), tracker.describe());
    builder.finish()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let video = &args[1];
    let opt = |name: &str, n: usize| args.iter().position(|a| a == name).map(|i| args[i + 1..=i + n].to_vec());
    let flag = |name: &str| args.iter().any(|a| a == name);
    let camera = opt("--camera", 2).unwrap_or(vec![String::new(), String::new()]);
    let refraction = if flag("--water") { 1.33 } else { 1.0 };
    let width: u32 = opt("--width", 1).map(|x| x[0].parse().unwrap()).unwrap_or(lfv::TRACKED_WIDTH);

    let probe = Command::new("ffprobe").args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,r_frame_rate,duration", "-of", "default=nw=1", video]).output().unwrap();
    let probe = String::from_utf8_lossy(&probe.stdout).to_string();
    let get = |k: &str| probe.lines().find_map(|l| l.trim().strip_prefix(&format!("{k}=")).map(|v| v.to_string())).unwrap_or_else(|| panic!("no {k} in {probe:?}"));
    let size: (usize, usize) = (get("width").parse().unwrap(), get("height").parse().unwrap());
    let fps = { let r: Vec<f64> = get("r_frame_rate").split('/').map(|x| x.parse().unwrap()).collect(); r[0] / r[1] };
    let duration_ms = get("duration").parse::<f64>().unwrap() * 1000.0;
    println!("{}x{} {fps:.3} fps, {:.1} s", size.0, size.1, duration_ms / 1000.0);

    let loaded = opt("--lens", 1).map(|p| {
        let mut l = LensProfile::default();
        let data = std::fs::read_to_string(&p[0]).unwrap();
        // A profile, or a project with one
        let v: serde_json::Value = serde_json::from_str(&data).unwrap();
        l.load_from_data(&v.get("calibration_data").map(|c| c.to_string()).unwrap_or(data)).unwrap();
        l
    });
    let mut db = LensProfileDatabase::default();
    if !camera[0].is_empty() { db.load_all(); }
    let candidates = lfv::candidates(&db, (&camera[0], &camera[1], ""), size, fps, loaded.as_ref(), 1.0);
    for c in &candidates { println!("candidate: {} - f {:.0} {:.0} c {:.0} {:.0} k {:.3?} readout {:?}", c.label(), c.lens.fx, c.lens.fy, c.lens.cx, c.lens.cy, c.lens.k, c.readout_ms); }

    let never = || false;
    let t = Instant::now();
    let automatic = lfv::automatic_fragments(duration_ms, fps);
    let fragments: Vec<Fragment> = automatic.iter().map(|r| track(video, size, fps, *r, width)).collect();
    println!("tracked {} fragments in {:.1} s", fragments.len(), t.elapsed().as_secs_f64());
    let t = Instant::now();
    // The medium is the user's to say only for an estimate: a known lens is tried both ways
    let media: Vec<f64> = if flag("--water") { vec![1.33] } else if flag("--air") { vec![1.0] } else { vec![1.0, 1.33] };
    let known = lfv::find_known(&fragments, &candidates, &media, 1.0, &never, &|_| ());
    for (r, c) in known.tried.iter().zip(&candidates) {
        let m = r.matching();
        println!("{}{} [{}{}]: {:?}, holds on {} of {} ({} more couldn't tell), moves {:.1} px (corner {:.1}, limit {:.0}), error {:.2}", if r.accepted { "FITS " } else { "" }, c.label(),
            if r.refraction() != 1.0 { "water" } else { "air" }, if r.medium_sure { "" } else { ", unsure" }, m.verdict, m.fragments_fitting, m.fragments, m.fragments_unable, m.moved, m.moved_corner, m.limit, r.result().error);
        for c in r.result().checks.iter() {
            match c {
                Some(c) => println!("    moved {:.1} / {:.1} px{}, readout {:.3} -> {:.3}, error {:.2} -> {:.2}, parallax {:.1}, {} points, weight {:.2}{}{}", c.moved.median, c.moved.max, if c.at_rest { " from the fragment's lens at rest" } else { "" }, c.readout, c.readout_free, c.error, c.error_free, c.scene.parallax, c.scene.points, c.weight(), if c.fits { ": fits" } else { "" }, if c.votes() { "" } else { " (the fragment can't tell)" }),
                None => println!("    no scene"),
            }
        }
    }
    println!("known profiles in {:.1} s: {}", t.elapsed().as_secs_f64(), match known.accepted { Some(i) => format!("{} fits{}, readout {:.1?} ms", known.tried[i].name, if candidates[i].loadable { "" } else { " (as a profile of its own)" }, known.tried[i].matching().readout_ms), None => "none fits".into() });
    if known.accepted.is_some() && !flag("--estimate") { return; }

    let start = known.best().map(|i| &candidates[i]).unwrap_or(&candidates[0]);
    // An estimate is made in the medium given, or the one the search found
    let refraction = if flag("--water") { 1.33 } else if flag("--air") { 1.0 } else { known.best().map(|i| known.tried[i].refraction()).unwrap_or(refraction) };
    let range = match (opt("--from", 1), opt("--to", 1)) {
        (Some(a), Some(b)) => (a[0].parse::<f64>().unwrap() * 1000.0, b[0].parse::<f64>().unwrap() * 1000.0),
        _ => (0.0, duration_ms),
    };
    let t = Instant::now();
    let fitted = lfv::fragment_from(range.0, range.1, fps, lfv::MAX_FRAMES);
    let fragment = track(video, size, fps, fitted, width);
    println!("estimating from {}, tracked in {:.1} s", start.label(), t.elapsed().as_secs_f64());
    let t = Instant::now();
    match lfv::estimate(&fragment.tracks(), &start.lens.with_refraction(refraction), fragment.frame_ms, &never, &|_| ()) {
        Ok(e) => {
            let l: Lens = e.lens;
            println!("lens fx {:.2} fy {:.2} cx {:.2} cy {:.2} k {:.5?} | readout {:.4} of the frame interval = {:.1} ms", l.fx, l.fy, l.cx, l.cy, l.k, e.readout, e.readout_ms);
            println!("error of the held-out tracks {:.2} -> {:.2} px (at 2.7K), within 1 px {:.0}%, focal firmness {:.2}, parallax {:.1}, {} points, {} frames; {:.1} s", e.error_before, e.error, e.within_1px * 100.0, e.firmness, e.scene.parallax, e.scene.points, e.scene.frames, t.elapsed().as_secs_f64());
            if let Some(reference) = opt("--ref", 8) {
                let r: Vec<f64> = reference.iter().map(|x| x.parse().unwrap()).collect();
                let d = Lens::new(r[0], r[1], r[2], r[3], [r[4], r[5], r[6], r[7]]).shift_after_tilt(&Lens { refraction: 1.0, ..l }, size);
                println!("against the reference lens: {:.2} / {:.2} px (median / max)", d.median, d.max);
            }
            // Judged as a known profile is, on the parts of the clip it wasn't made from
            let apart: Vec<Fragment> = lfv::apart_from(&automatic, fitted).into_iter().map(|i| fragments[i].clone()).collect();
            if apart.is_empty() {
                println!("not checked on other parts of the clip: it has none apart from the one the lens was estimated from");
            } else {
                let m = lfv::check_lens(&apart, &l, Some(e.readout_ms), &never);
                println!("on {} other part(s) of the clip: {:?}, holds on {} of {} ({} more couldn't tell), {:.1} px from the picture (corner {:.1}, limit {:.0})", apart.len(), m.verdict, m.fragments_fitting, m.fragments, m.fragments_unable, m.moved, m.moved_corner, m.limit);
            }
            println!("doubts: {:?}", lfv::doubts(&e));
            let profile = lfv::profile(&e, Some(e.readout_ms), size, fps, refraction != 1.0, (&camera[0], &camera[1]), start.profile.as_ref().or(loaded.as_ref()), "lens_from_video example");
            if let Some(out) = opt("--out", 1) { std::fs::write(&out[0], profile.get_json().unwrap()).unwrap(); }
        },
        Err(e) => println!("no estimate: {e:?}"),
    }
}
