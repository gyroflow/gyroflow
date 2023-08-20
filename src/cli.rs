// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use argh::FromArgs;
use cpp::*;
use gyroflow_core::*;
use std::sync::Arc;
use std::time::Instant;
use qmetaobject::{ QString, QStringList };
use std::cell::RefCell;
use std::collections::HashMap;
use crate::rendering;
use crate::rendering::render_queue::*;
use indicatif::{ProgressBar, MultiProgress, ProgressState, ProgressStyle};

cpp! {{
    struct TraitObject2 { void *data; void *vtable; };
    #include <QCoreApplication>
    #include <QFileSystemWatcher>
    #include <QTimer>
    #include <QDirIterator>
    #include <QMap>
}}
macro_rules! connect {
    ($obj_ptr:ident, $obj_borrowed:ident, $signal:ident, $cb:expr) => {
        qmetaobject::connect($obj_ptr, $obj_borrowed.$signal.to_cpp_representation(&*$obj_borrowed), $cb);
    };
}

/** Gyroflow v1.5.2
Video stabilization using gyroscope data
*/
#[derive(FromArgs)]
struct Opts {
    /// input files: videos, project files, lens profiles, presets
    #[argh(positional)]
    input: Vec<String>,

    /// overwrite if output file exists, default: false
    #[argh(switch, short = 'f')]
    overwrite: bool,

    /// number of parallel renders, default: 1
    #[argh(option, short = 'j', default = "1")]
    parallel_renders: i32,

    /// when done: 1 - shut down; 2 - reboot; 3 - sleep; 4 - hibernate; 5 - logout
    #[argh(option, short = 'd', default = "0")]
    when_done: i32,

    /// output parameters, eg. "{{ 'codec': 'H.265/HEVC', 'bitrate': 150, 'use_gpu': true, 'audio': true }}"
    #[argh(option, short = 'p')]
    out_params: Option<String>,

    /// export project file instead of rendering: 1 - default project, 2 - with gyro data, 3 - with processed gyro data, 4 - video + project file
    #[argh(option, default = "0")]
    export_project: u32,

    /// preset (file or content directly), eg. "{{ 'version': 2, 'stabilization': {{ 'fov': 1.5 }} }}"
    #[argh(option)]
    preset: Option<String>,

    /// open file in the GUI (video or project)
    #[argh(option)]
    open: Option<String>,

    /// watch folder for automated processing
    #[argh(option)]
    watch: Option<String>,

    /// gyro file path
    #[argh(option, short = 'g')]
    gyro_file: Option<String>,
}

pub fn will_run_in_console() -> bool {
    if std::env::args().len() > 1 {
        let opts: Opts = argh::from_env();
        if let Some(open) = opts.open {
            if !open.is_empty() {
                return false;
            }
        }
        return true;
    }
    false
}

pub fn run(open_file: &mut String) -> bool {
    if std::env::args().len() > 1 {
        let opts: Opts = argh::from_env();

        let (videos, mut lens_profiles, mut presets) = detect_types(&opts.input);
        if let Some(mut preset) = opts.preset {
            if !preset.is_empty() {
                if preset.starts_with('{') { preset = preset.replace('\'', "\""); }
                presets.push(preset);
            }
        }

        if let Some(open) = opts.open {
            if !open.is_empty() {
                *open_file = open;
                return false;
            }
        }

        for file in videos.iter().chain(lens_profiles.iter()) {
            if !std::path::Path::new(&file).exists() {
                log::error!("File {} doesn't exist.", file);
                return true;
            }
        }
        let mut watching = opts.watch.as_ref().map(|x| !x.is_empty()).unwrap_or_default();

        if !watching {
            if lens_profiles.len() > 1 {
                log::error!("More than one lens profile!");
                return true;
            }
            if videos.is_empty() {
                log::error!("No videos provided!");
                return true;
            }

            log::info!("Videos: {:?}", videos);
            if !lens_profiles.is_empty() { log::info!("Lens profiles: {:?}", lens_profiles); }
            if !presets.is_empty() { log::info!("Presets: {:?}", presets); }
        }

        let m = MultiProgress::new();
        m.set_draw_target(indicatif::ProgressDrawTarget::hidden());
        let sty = ProgressStyle::with_template("[{bar:50.cyan/blue}] {pos:>5}/{len:5} {eta:11} {prefix:.magenta}\x1B[37;1m{msg}\x1B[0m")
            .unwrap()
            .with_key("eta", |state: &ProgressState, w: &mut dyn std::fmt::Write| write!(w, "ETA {:.1}s", state.eta().as_secs_f64()).unwrap())
            .progress_chars("#>-");

        // let spinner = ["⠋","⠙","⠹","⠸","⠼","⠴","⠦","⠧","⠇","⠏"];
        // let spinner = ["◜","◠","◝","◞","◡","◟"];
        let spinner = [
            "⢀⠀","⡀⠀","⠄⠀","⢂⠀","⡂⠀","⠅⠀","⢃⠀","⡃⠀","⠍⠀","⢋⠀","⡋⠀","⠍⠁","⢋⠁","⡋⠁","⠍⠉","⠋⠉","⠋⠉","⠉⠙","⠉⠙","⠉⠩","⠈⢙","⠈⡙","⢈⠩","⡀⢙","⠄⡙","⢂⠩","⡂⢘","⠅⡘",
            "⢃⠨","⡃⢐","⠍⡐","⢋⠠","⡋⢀","⠍⡁","⢋⠁","⡋⠁","⠍⠉","⠋⠉","⠋⠉","⠉⠙","⠉⠙","⠉⠩","⠈⢙","⠈⡙","⠈⠩","⠀⢙","⠀⡙","⠀⠩","⠀⢘","⠀⡘","⠀⠨","⠀⢐","⠀⡐","⠀⠠","⠀⢀","⠀⡀"
        ];

        let pbh0 = m.add(ProgressBar::new(1)); pbh0.set_style(ProgressStyle::with_template("{msg}").unwrap()); pbh0.set_message(" ");
        let pbh = m.add(ProgressBar::new(1)); pbh.set_style(ProgressStyle::with_template("{spinner:.green} {msg:73} Elapsed: {elapsed_precise}").unwrap().tick_strings(&spinner)); pbh.set_message("Queue"); pbh.enable_steady_tick(std::time::Duration::from_millis(70));

        log::set_max_level(log::LevelFilter::Info);

        let time = Instant::now();
        let mut queue_printed = false;

        let stab = Arc::new(StabilizationManager::default());
        stab.lens_profile_db.write().load_all();

        let mut queue = RenderQueue::new(stab.clone());

        rendering::init().unwrap();
        if let Some((name, _list_name)) = gyroflow_core::gpu::initialize_contexts() {
            rendering::set_gpu_type_from_name(&name);
        }
        let mut additional_data = setup_defaults(stab, &mut queue);

        if let Some(mut outp) = opts.out_params {
            outp = outp.replace('\'', "\"");
            gyroflow_core::util::merge_json(additional_data.get_mut("output").unwrap(), &serde_json::from_str(&outp).expect("Invalid json"));
        }

        queue.set_parallel_renders(opts.parallel_renders.max(1));
        queue.set_when_done(opts.when_done);
        let suffix = format!("{}.", queue.default_suffix);

        if opts.export_project > 0 {
            queue.export_project = opts.export_project;
        }

        let mut pbs = HashMap::<u32, ProgressBar>::new();

        let queue = RefCell::new(queue);
        let queue_ptr = unsafe { qmetaobject::QObjectPinned::new(&queue).get_or_create_cpp_object() };

        if let Some(watch) = opts.watch {
            watching = watch_folder(watch, |path| {
                if !path.contains(&suffix) {
                    log::info!("New file detected: {}", path);
                    let extensions = [ "mp4", "mov", "mxf", "mkv", "webm", "insv", "gyroflow", "png", "exr", "dng", "braw" ];
                    let ext = std::path::Path::new(&path).extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                    if extensions.contains(&ext.as_str()) {
                        let queue = unsafe { &mut *queue.as_ptr() };
                        let additional_data2 = additional_data.to_string();
                        qmetaobject::single_shot(std::time::Duration::from_millis(1), move || {
                            queue.add_file(path.clone(), String::new(), additional_data2.clone());
                        });
                    }
                }
            });
        }

        unsafe {
            let q = queue.borrow();
            connect!(queue_ptr, q, status_changed, || {
                let queue = &mut *queue.as_ptr();
                // log::info!("Status: {}", q.status.to_string());

                if !watching && queue.status.to_string() == "stopped" && queue.get_pending_count() == 0 && queue.get_active_render_count() == 0 {
                    cpp!(unsafe [] { qApp->quit(); });
                }
            });
            connect!(queue_ptr, q, render_progress, |job_id: &u32, _progress: &f64, current_frame: &usize, total_frames: &usize, _finished: &bool, _start_time: &f64, _is_conversion: &bool| {
                let pb = pbs.get(job_id).unwrap();
                let queue = &mut *queue.as_ptr();
                let qi = queue.queue.borrow();
                if *current_frame >= *total_frames {
                    let mut ok = true;
                    for item in qi.iter() {
                        if item.job_id == *job_id {
                            ok = item.error_string.is_empty();
                            break;
                        }
                    }
                    if ok {
                        pb.set_message(format!("\x1B[1;32m{}\x1B[0m", pb.message())); // Green
                    } else {
                        pb.set_message(format!("\x1B[1;31m{}\x1B[0m", pb.message())); // Red
                    }
                    m.set_draw_target(indicatif::ProgressDrawTarget::hidden());
                } else if *current_frame > 0 && m.is_hidden() {
                    pbh.set_message("Rendering:");

                    if !queue_printed {
                        log::info!("Queue:");
                        for item in qi.iter() {
                            log::info!("- [{:08x}] {} -> {}, {}, Frames: {}, Status: {:?} {}", item.job_id, item.input_file, item.output_path, item.export_settings, item.total_frames, item.get_status(), item.error_string);
                        }
                        queue_printed = true;
                    }

                    for item in qi.iter() {
                        if let Some(pb2) = pbs.get(&item.job_id) {
                            pb2.set_position(item.current_frame);
                            pb2.set_length(item.total_frames);
                        }
                    }
                    m.set_draw_target(indicatif::ProgressDrawTarget::stdout());
                }

                pb.set_length(*total_frames as u64);
                pb.set_position(*current_frame as u64);
            });
            connect!(queue_ptr, q, processing_progress, |job_id: &u32, progress: &f64| {
                let mut any_other_in_progress = false;
                {
                    let queue = &mut *queue.as_ptr();
                    let qi = queue.queue.borrow();
                    for item in qi.iter() {
                        if item.job_id != *job_id && item.processing_progress > 0.0 && item.processing_progress < 1.0 {
                            any_other_in_progress = true;
                            break;
                        }
                    }
                }

                if *progress == 1.0 && !m.is_hidden() && !any_other_in_progress {
                    m.set_draw_target(indicatif::ProgressDrawTarget::hidden());
                } else if *progress > 0.01 && *progress < 1.0 && m.is_hidden() {
                    pbh.set_message("Synchronizing:");
                    m.set_draw_target(indicatif::ProgressDrawTarget::stdout());
                }

                let pb = pbs.get(job_id).unwrap();
                if *progress < 0.999 {
                    pb.set_length(100);
                    pb.set_position((*progress * 100.0).round() as u64);
                }
            });
            connect!(queue_ptr, q, convert_format, |job_id: &u32, format: &QString, supported: &QString| {
                log::error!("[{:08x}] Pixel format {} is not supported. Supported are: {}", job_id, format.to_string(), supported.to_string());
            });
            connect!(queue_ptr, q, error, |job_id: &u32, text: &QString, arg: &QString, _callback: &QString| {
                if opts.overwrite && text.to_string().starts_with("file_exists:") {
                    let queue = &mut *queue.as_ptr();
                    queue.reset_job(*job_id);
                    log::warn!("[{:08x}] File exists, overwriting: {}", job_id, text.to_string().strip_prefix("file_exists:").unwrap());
                    return;
                }
                log::error!("[{:08x}] Error: {}", job_id, text.to_string().replace("%1", &arg.to_string()));
            });
            connect!(queue_ptr, q, added, |job_id: &u32| {
                let queue = &mut *queue.as_ptr();
                let fname = std::path::Path::new(&queue.get_job_output_path(*job_id).to_string()).file_name().map(|x| x.to_string_lossy().to_string()).unwrap();
                //log::info!("[{:08x}] Job added: {}", job_id, q.get_job_output_path(*job_id));
                let pb = m.add(ProgressBar::new(1));
                pb.set_style(sty.clone());
                pb.set_message(fname);
                pbs.insert(*job_id, pb);
            });
            connect!(queue_ptr, q, processing_done, |job_id: &u32, by_preset: &bool| {
                let queue = &mut *queue.as_ptr();
                log::info!("[{:08x}] Processing done", job_id);

                if let Some(file) = lens_profiles.first() {
                    // Apply lens profile
                    log::info!("Loading lens profile {}", file);
                    let stab = queue.get_stab_for_job(*job_id).unwrap();
                    stab.load_lens_profile(file).expect("Loading lens profile");
                    stab.recompute_blocking();
                }

                let fname = std::path::Path::new(&queue.get_job_output_path(*job_id).to_string()).file_name().map(|x| x.to_string_lossy().to_string()).unwrap();
                pbs.get(job_id).unwrap().set_message(fname);

                queue.jobs_added.remove(job_id);

                let mut applying_preset = false;

                if queue.jobs_added.is_empty() {
                    // All jobs added and completed processing

                    if !by_preset {
                        // Apply presets
                        for preset in &presets {
                            log::info!("Applying preset {}", preset);
                            if preset.starts_with('{') {
                                queue.apply_to_all(preset.clone(), additional_data.to_string(), 0);
                                applying_preset = true;
                            } else if let Ok(data) = std::fs::read_to_string(preset) {
                                queue.apply_to_all(data, additional_data.to_string(), 0);
                                applying_preset = true;
                            }
                        }
                    }
                    if !watching {
                        lens_profiles.clear(); // Apply lens profiles only once
                        presets.clear();
                    }

                    if !applying_preset {
                        qmetaobject::single_shot(std::time::Duration::from_millis(500), move || {
                            queue.start(); // Start the rendering queue
                        });
                    }
                }
            });
        }

        if !watching {
            let mut queue = queue.borrow_mut();
            let gyro_file = opts.gyro_file.unwrap_or_default();
            for file in &videos {
                queue.add_file(file.clone(), gyro_file.clone(), additional_data.to_string());
            }
        }

        // Run the event loop
        cpp!(unsafe [] {
            int argc = 0;
            QCoreApplication(argc, nullptr).exec();
        });

        log::info!("Done in {:.3}s", time.elapsed().as_millis() as f64 / 1000.0);

        return true;
    }

    false
}

fn detect_types(all_files: &[String]) -> (Vec<String>, Vec<String>, Vec<String>) { // -> Videos/projects, lens profiles, presets
    let mut videos = Vec::new();
    let mut lens_profiles = Vec::new();
    let mut presets = Vec::new();
    for file in all_files {
        if file.ends_with(".json") { // Lens profile
            lens_profiles.push(file.clone());
        } else if file.ends_with(".gyroflow") {
            let video_path = || -> Option<String> {
                let data = std::fs::read(file).ok()?;
                let obj: serde_json::Value = serde_json::from_slice(&data).ok()?;
                Some(obj.get("videofile")?.as_str()?.to_string())
            }().unwrap_or_default();

            if video_path.is_empty() { // It's a preset
                presets.push(file.clone());
            } else {
                videos.push(file.clone());
            }
        } else {
            videos.push(file.clone());
        }
    }
    (videos, lens_profiles, presets)
}

fn get_saved_settings() -> HashMap<String, String> {
    let settings = cpp!(unsafe [] -> (QStringList, QStringList) as "std::pair<QStringList, QStringList>" {
        QSettings sett;
        QStringList keys, values;
        for (const auto &key : sett.allKeys()) {
            keys.append(key);
            values.append(sett.value(key).toString());
        }
        return { keys, values };
    });
    let mut map = HashMap::new();
    for (k, v) in settings.0.into_iter().zip(settings.1.into_iter()) {
        map.insert(k.to_string(), v.to_string());
    }
    map
}

fn setup_defaults(stab: Arc<StabilizationManager>, queue: &mut RenderQueue) -> serde_json::Value {
    let settings = get_saved_settings();
    dbg!(&settings);

    let codecs = [
        "H.264/AVC",
        "H.265/HEVC",
        "ProRes",
        "DNxHD",
        "CineForm",
        "EXR Sequence",
        "PNG Sequence",
        "AV1",
    ];

    // Default settings - project file will override this

    match settings.get("croppingMode").unwrap_or(&"1".into()).parse::<u32>() {
        Ok(0) => stab.set_adaptive_zoom(0.0), // No zooming
        Ok(1) => stab.set_adaptive_zoom(settings.get("adaptiveZoom").unwrap_or(&"4".into()).parse::<f64>().unwrap()),
        Ok(2) => stab.set_adaptive_zoom(-1.0), // Static zoom
        _ => { }
    }
    stab.set_lens_correction_amount(settings.get("correctionAmount").unwrap_or(&"1".into()).parse::<f64>().unwrap());
    let smoothing_method = settings.get("smoothingMethod").unwrap_or(&"1".into()).parse::<usize>().unwrap();
    let smoothing_method_prefix = format!("smoothing-{}-", smoothing_method);
    stab.set_smoothing_method(smoothing_method);
    for (k, v) in &settings {
        if k.starts_with(&smoothing_method_prefix) {
            stab.set_smoothing_param(k.strip_prefix(&smoothing_method_prefix).unwrap(), v.parse::<f64>().unwrap());
        }
    }

    // TODO: set more params from `settings`

    if let Some(gdec) = settings.get("gpudecode").and_then(|x| x.parse::<bool>().ok()) {
        *rendering::GPU_DECODING.write() = gdec;
    }
    if let Some(suffix) = settings.get("defaultSuffix") {
        queue.default_suffix = QString::from(suffix.as_str());
    }

    let codec = settings.get("defaultCodec").unwrap_or(&"0".into()).parse::<usize>().unwrap().min(codecs.len() - 1);
    let codec_name = codecs[codec];

    if let Some(processing_device) = settings.get("processingDeviceIndex").and_then(|x| x.parse::<i32>().ok()) {
        stab.set_device(processing_device);
    }

    // Sync and export settings
    serde_json::json!({
        "output": {
            "codec":          codec_name,
            "codec_options":  "",
            // "output_path":    "C:/test.mp4",
            // "output_width":   3840,
            // "output_height":  2160,
            // "bitrate":        150,
            "use_gpu":        settings.get(&format!("exportGpu-{}", codec)).unwrap_or(&"1".into()).parse::<u32>().unwrap() > 0,
            "audio":          settings.get("exportAudio").unwrap_or(&"true".into()).parse::<bool>().unwrap(),
            "pixel_format":   "",

            // Advanced
            "encoder_options":       settings.get(&format!("encoderOptions-{}", codec)).unwrap_or(&"".into()),
            "metadata":              { "comment": settings.get("metadataComment").unwrap_or(&"".into()) },
            "keyframe_distance":     settings.get("keyframeDistance").unwrap_or(&"1".into()).parse::<u32>().unwrap(),
            "preserve_other_tracks": settings.get("preserveOtherTracks").unwrap_or(&"false".into()).parse::<bool>().unwrap(),
            "pad_with_black":        settings.get("padWithBlack").unwrap_or(&"false".into()).parse::<bool>().unwrap(),
            "audio_codec":           settings.get("audioCodec").unwrap_or(&"AAC".into()),
        },
        "synchronization": {
            "initial_offset":     0,
            "initial_offset_inv": false,
            "search_size":        5,
            "calc_initial_fast":  false,
            "max_sync_points":    5,
            "every_nth_frame":    1,
            "time_per_syncpoint": 1,
            "of_method":          2,
            "offset_method":      2,
            "auto_sync_points":   true,
        }
    })
}

fn watch_folder<F: FnMut(String)>(path: String, cb: F) -> bool {
    if path.is_empty() { return false; }
    if !std::path::Path::new(&path).exists() { log::info!("{} doesn't exist.", path); return false; }

    let path = QString::from(path);
    let func: Box<dyn FnMut(String)> = Box::new(cb);
    let cb_ptr = Box::into_raw(func);
    cpp!(unsafe [path as "QString", cb_ptr as "TraitObject2"] -> bool as "bool" {
        auto w = new QFileSystemWatcher();
        auto existing = new QStringList();
        auto paths = new QMap<QString, QMap<QString, qint64> >();
        auto t = new QTimer();
        QObject::connect(t, &QTimer::timeout, [=] {
            bool anyWatching = false;
            for (const auto &file : paths->keys()) {
                auto &paths2 = (*paths)[file];
                for (const auto &path : paths2.keys()) {
                    anyWatching = true;
                    QFile f(path);
                    if (f.open(QFile::ReadOnly)) {
                        auto size = f.size();
                        f.close();
                        if (paths2[path] > 0 && paths2[path] == size) {
                            rust!(Rust_Gyroflow_cli_watch [cb_ptr: *mut dyn FnMut(String) as "TraitObject2", path: QString as "QString"] {
                                let mut cb = unsafe { Box::from_raw(cb_ptr) };
                                cb(path.to_string());
                                let _ = Box::into_raw(cb); // leak again so it doesn't get deleted here
                            });

                            existing->append(path);
                            paths2.remove(path);
                        } else {
                            paths2[path] = size;
                        }
                    }
                }
            }
            if (!anyWatching) {
                t->stop();
            }
        });

        QDirIterator it(path, QDirIterator::Subdirectories);
        while (it.hasNext()) {
            it.next();
            auto i = it.fileInfo();
            if (i.fileName() == "..") continue;
            if (i.isDir()) w->addPath(i.absoluteFilePath());
            if (i.isFile()) existing->append(i.absoluteFilePath());
        }
        QObject::connect(w, &QFileSystemWatcher::directoryChanged, [=](const QString &file) {
            auto &paths2 = (*paths)[file];

            for (const auto &i : QDir(file).entryInfoList(QDir::NoDotAndDotDot | QDir::AllEntries | QDir::Readable)) {
                if (i.fileName() == "..") continue;
                if (i.isDir()) w->addPath(i.absoluteFilePath());
                if (i.isFile() && !existing->contains(i.absoluteFilePath()))
                    paths2.insert(i.absoluteFilePath(), 0);
            }
            t->start(1000);
        });
        return !w->directories().isEmpty();
    })
}
