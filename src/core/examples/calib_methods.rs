// The lens calibrator end to end on a video, without the GUI - frames every `nth` like "Auto calibrate" (processing
// resolution 4k, sharpness limit 8 px), the boards found by `feed_frame`, then `calibrate` with each method and model.
// Needs the OpenCV environment `just` exports, and an ffmpeg with filters (`FFMPEG_EXE`, the bundled one is lite):
// just -f _scripts/windows.just --command cargo run --release --manifest-path <abs>/src/core/Cargo.toml --features use-opencv --example calib_methods -- <video> [nth]

#[cfg(not(feature = "use-opencv"))]
fn main() { eprintln!("needs --features use-opencv"); }

#[cfg(feature = "use-opencv")]
fn main() {
    use gyroflow_core::calibration::{ CalibrationMethod, CalibrationModel, LensCalibrator };
    use std::io::Read;
    use std::process::{ Command, Stdio };
    use std::sync::Arc;
    use std::sync::atomic::{ AtomicBool, AtomicUsize, Ordering::SeqCst };

    let args: Vec<String> = std::env::args().collect();
    let video = &args[1];
    let nth: usize = args.get(2).and_then(|x| x.parse().ok()).unwrap_or(10);

    let probe = Command::new("ffprobe").args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,r_frame_rate", "-of", "default=nw=1", video]).output().unwrap();
    let probe = String::from_utf8_lossy(&probe.stdout).to_string();
    let get = |k: &str| probe.lines().find_map(|l| l.trim().strip_prefix(&format!("{k}=")).map(|v| v.to_string())).unwrap_or_else(|| panic!("no {k} in {probe:?}"));
    let (w, h): (u32, u32) = (get("width").parse().unwrap(), get("height").parse().unwrap());
    let fps = { let r: Vec<f64> = get("r_frame_rate").split('/').map(|x| x.parse().unwrap()).collect(); r[0] / r[1] };
    let pt_scale = h as f32 / 2160.0;
    let (sw, sh) = ((w as f32 / pt_scale).round() as u32, 2160u32);
    println!("{w}x{h} -> {sw}x{sh}, every {nth}th frame");

    let mut cal = LensCalibrator::new();
    cal.max_images = 15;
    cal.iterations = 500;
    cal.max_sharpness = 8.0;

    let mut ff = Command::new(std::env::var("FFMPEG_EXE").unwrap_or("ffmpeg".into())).args(["-v", "error", "-i", video, "-vf", &format!("select=not(mod(n\\,{nth})),scale={sw}:{sh}"), "-fps_mode", "passthrough", "-pix_fmt", "gray", "-f", "rawvideo", "-"])
        .stdout(Stdio::piped()).spawn().unwrap();
    let mut out = ff.stdout.take().unwrap();
    let mut buf = vec![0u8; (sw * sh) as usize];
    let processed = Arc::new(AtomicUsize::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let mut fed = 0usize;
    while out.read_exact(&mut buf).is_ok() {
        let frame = (fed * nth) as i32;
        let ts = (frame as f64 / fps * 1_000_000.0) as i64;
        // Keep the queue short: feed_frame copies the frame into a pool job
        while fed - processed.load(SeqCst) > 24 { std::thread::sleep(std::time::Duration::from_millis(20)); }
        cal.feed_frame(ts, frame, (sw, sh), (w, h), sw as usize, pt_scale, &buf, cancel.clone(), 100000, processed.clone(), |_| {});
        fed += 1;
    }
    while processed.load(SeqCst) < fed { std::thread::sleep(std::time::Duration::from_millis(50)); }
    println!("{fed} frames, boards found in {}, under the sharpness limit {}", cal.all_matches.read().len(), cal.image_points.read().len());

    for (method, model) in [
        (CalibrationMethod::OpenCV,   CalibrationModel::OpenCVFisheye),
        (CalibrationMethod::OpenCV,   CalibrationModel::OpenCVStandard),
        (CalibrationMethod::Gyroflow, CalibrationModel::OpenCVFisheye),
        (CalibrationMethod::Gyroflow, CalibrationModel::OpenCVStandard),
        (CalibrationMethod::Gyroflow, CalibrationModel::GenericPolynomial),
    ] {
        cal.method = method;
        cal.model = model;
        let t = std::time::Instant::now();
        match cal.calibrate(false) {
            Ok(()) => println!("{method:?} {model:?}: {:.1}s, rms {:.4} px on {} frames, f ({:.2}, {:.2}), c ({:.2}, {:.2}), k {:?}",
                t.elapsed().as_secs_f64(), cal.rms, cal.used_points.len(), cal.k[(0, 0)], cal.k[(1, 1)], cal.k[(0, 2)], cal.k[(1, 2)], cal.d),
            Err(e) => println!("{method:?} {model:?}: {:.1}s, failed: {e:?}", t.elapsed().as_secs_f64()),
        }
    }
}
