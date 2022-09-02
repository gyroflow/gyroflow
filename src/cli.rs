// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use argh::FromArgs;
use cpp::*;
use gyroflow_core::*;
use std::sync::Arc;
use std::time::Instant;
use qmetaobject::{ QString, QStringList };
use std::cell::RefCell;
use std::collections::{ HashMap, HashSet };
use crate::rendering;
use crate::rendering::render_queue::*;
use indicatif::{ProgressBar, MultiProgress, ProgressState, ProgressStyle};

cpp! {{
    #include <QCoreApplication>
}}

/** Gyroflow v1.2.0
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
}

pub fn run() -> bool {
    if std::env::args().len() > 1 {
        let opts: Opts = argh::from_env();

        let (videos, mut lens_profiles, mut presets) = detect_types(&opts.input);

        for file in videos.iter().chain(lens_profiles.iter()) {
            if !std::path::Path::new(&file).exists() {
                log::error!("File {} doesn't exist.", file);
                return true;
            }
        }
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

        let m = MultiProgress::new();
        m.set_draw_target(indicatif::ProgressDrawTarget::hidden());
        let sty = ProgressStyle::with_template("{elapsed_precise} [{bar:50.cyan/blue}] {pos:>7}/{len:7} {eta:11} {prefix:.magenta}\x1B[1m{msg}\x1B[0m")
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
        let pbh = m.add(ProgressBar::new(1)); pbh.set_style(ProgressStyle::with_template("{spinner:.green} {msg}:").unwrap().tick_strings(&spinner)); pbh.set_message("Queue"); pbh.enable_steady_tick(std::time::Duration::from_millis(70));

        log::set_max_level(log::LevelFilter::Info);

        let _time = Instant::now();
        let mut queue_printed = false;

        let stab = Arc::new(StabilizationManager::<stabilization::RGBA8>::default());
        stab.lens_profile_db.write().load_all();

        let mut queue = RenderQueue::new(stab.clone());

        rendering::init().unwrap();
        if let Some((name, _list_name)) = gyroflow_core::gpu::initialize_contexts() {
            rendering::set_gpu_type_from_name(&name);
        }
        let mut additional_data = setup_defaults(stab);

        if let Some(mut outp) = opts.out_params {
            outp = outp.replace('\'', "\"");
            gyroflow_core::util::merge_json(additional_data.get_mut("output").unwrap(), &serde_json::from_str(&outp).expect("Invalid json"));
        }

        queue.set_parallel_renders(opts.parallel_renders.max(1));
        queue.set_when_done(opts.when_done);

        let mut jobs_added = HashSet::new();
        let mut pbs = HashMap::<u32, ProgressBar>::new();

        let obj = RefCell::new(queue);
        let obj_ptr = unsafe { qmetaobject::QObjectPinned::new(&obj).get_or_create_cpp_object() };
        unsafe {
            qmetaobject::connect(obj_ptr, obj.borrow().status_changed.to_cpp_representation(&*obj.borrow()), || {
                let q = &mut *obj.as_ptr();
                // log::info!("Status: {}", q.status.to_string());

                if q.status.to_string() == "stopped" && q.get_pending_count() == 0 && q.get_active_render_count() == 0 {
                    cpp!(unsafe [] { qApp->quit(); });
                }
            });
            // qmetaobject::connect(obj_ptr, obj.borrow().progress_changed.to_cpp_representation(&*obj.borrow()), || {
            //     let q = &mut *obj.as_ptr();
            //     let c = q.get_current_frame();
            //     let t = q.get_total_frames();
            //     println!("\rRendering {:.2}% ({c}/{t})", c as f64 / t as f64 * 100.0);
            //     std::io::stdout().flush().unwrap();
            // });
            qmetaobject::connect(obj_ptr, obj.borrow().render_progress.to_cpp_representation(&*obj.borrow()), |job_id: &u32, _progress: &f64, current_frame: &usize, total_frames: &usize, _finished: &bool| {
                //let q = obj.borrow();

                let pb = pbs.get(job_id).unwrap();
                if *current_frame >= *total_frames {
                    pb.set_message(format!("\x1B[1;32m{}\x1B[0m", pb.message()));
                    m.set_draw_target(indicatif::ProgressDrawTarget::hidden());
                } else if *current_frame > 0 && m.is_hidden() {
                    let q = &mut *obj.as_ptr();
                    pbh.set_message("Rendering");
                    let qi = q.queue.borrow();

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
            qmetaobject::connect(obj_ptr, obj.borrow().processing_progress.to_cpp_representation(&*obj.borrow()), |job_id: &u32, progress: &f64| {
                let mut any_other_in_progress = false;
                {
                    let q = &mut *obj.as_ptr();
                    let qi = q.queue.borrow();
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
                    pbh.set_message("Synchronizing");
                    m.set_draw_target(indicatif::ProgressDrawTarget::stdout());
                }

                let pb = pbs.get(job_id).unwrap();
                if *progress < 0.999 {
                    pb.set_length(100);
                    pb.set_position((*progress * 100.0).round() as u64);
                }
            });
            // qmetaobject::connect(obj_ptr, obj.borrow().queue_changed.to_cpp_representation(&*obj.borrow()), || {
            //     log::info!("Current queue:");
            //     let q = &mut *obj.as_ptr();
            //     let qi = q.queue.borrow();
            //     for item in qi.iter() {
            //         log::info!("- [{:08x}] {} -> {}, {}, Frames: {}, Status: {:?} {}", item.job_id, item.input_file, item.output_path, item.export_settings, item.total_frames, item.get_status(), item.error_string);
            //     }
            // });
            qmetaobject::connect(obj_ptr, obj.borrow().convert_format.to_cpp_representation(&*obj.borrow()), |job_id: &u32, format: &QString, supported: &QString| {
                log::error!("[{:08x}] Pixel format {} is not supported. Supported are: {}", job_id, format.to_string(), supported.to_string());
            });
            qmetaobject::connect(obj_ptr, obj.borrow().error.to_cpp_representation(&*obj.borrow()), |job_id: &u32, text: &QString, arg: &QString, callback: &QString| {
                if opts.overwrite && text.to_string().starts_with("file_exists") {
                    let q = &mut *obj.as_ptr();
                    q.reset_job(*job_id);
                }
                log::error!("[{:08x}] Error: {}, callback: {}", job_id, text.to_string().replace("%1", &arg.to_string()), callback.to_string());
            });
            qmetaobject::connect(obj_ptr, obj.borrow().added.to_cpp_representation(&*obj.borrow()), |job_id: &u32| {
                let q = &mut *obj.as_ptr();
                let fname = std::path::Path::new(&q.get_job_output_path(*job_id).to_string()).file_name().map(|x| x.to_string_lossy().to_string()).unwrap();
                //log::info!("[{:08x}] Job added: {}", job_id, q.get_job_output_path(*job_id));
                let pb = m.add(ProgressBar::new(1));
                pb.set_style(sty.clone());
                pb.set_message(fname);
                pbs.insert(*job_id, pb);
            });
            qmetaobject::connect(obj_ptr, obj.borrow().processing_done.to_cpp_representation(&*obj.borrow()), |job_id: &u32| {
                let q = &mut *obj.as_ptr();
                // log::info!("[{:08x}] Processing done", job_id);

                if !lens_profiles.is_empty() {
                    // Apply lens profile
                    let file = lens_profiles.first().unwrap();
                    log::info!("Loading lens profile {}", file);
                    let stab = q.get_stab_for_job(*job_id).unwrap();
                    stab.load_lens_profile(file).expect("Loading lens profile");
                    stab.recompute_blocking();
                }

                jobs_added.remove(job_id);

                if jobs_added.is_empty() {
                    // All jobs added and completed processing

                    lens_profiles.clear(); // Apply lens profiles only once

                    // Apply presets
                    for preset in presets.drain(..) {
                        if let Ok(data) = std::fs::read_to_string(&preset) {
                            log::info!("Applying preset {}", preset);
                            q.apply_to_all(data, additional_data.to_string());
                        }
                    }

                    qmetaobject::single_shot(std::time::Duration::from_millis(500), move || {
                        q.start(); // Start the rendering queue
                    });
                }
            });
        }

        {
            let mut q = obj.borrow_mut();
            for file in &videos {
                let job_id = q.add_file(file.clone(), additional_data.to_string());
                jobs_added.insert(job_id);
            }
        }

        // Run the event loop
        cpp!(unsafe [] {
            int argc = 0;
            QCoreApplication(argc, nullptr).exec();
        });

        log::info!("Done in {:.3}s", _time.elapsed().as_millis() as f64 / 1000.0);

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
                let data = std::fs::read(&file).ok()?;
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

fn setup_defaults(stab: Arc<StabilizationManager<stabilization::RGBA8>>) -> serde_json::Value {
    let settings = get_saved_settings();
    dbg!(&settings);

    let codecs = [
        "H.264/AVC",
        "H.265/HEVC",
        "ProRes",
        "DNxHD",
        "EXR Sequence",
        "PNG Sequence",
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

    let codec = settings.get("defaultCodec").unwrap_or(&"0".into()).parse::<usize>().unwrap().min(codecs.len() - 1);
    let codec_name = codecs[codec];

    // Sync and export settings
    serde_json::json!({
        "output": {
            "codec":          codec_name,
            "codec_options":  "",
            // "output_path":    "C:/test.mp4",
            // "output_width":   3840,
            // "output_height":  2160,
            // "bitrate":        150,
            "use_gpu":        settings.get(&format!("exportGpu-{}", codec)).unwrap_or(&"true".into()).parse::<bool>().unwrap(),
            "audio":          settings.get("exportAudio").unwrap_or(&"true".into()).parse::<bool>().unwrap(),
            "pixel_format":   "",

            // Advanced
            "encoder_options":       settings.get(&format!("encoderOptions-{}", codec)).unwrap_or(&"".into()),
            "keyframe_distance":     settings.get("keyframeDistance").unwrap_or(&"1".into()).parse::<u32>().unwrap(),
            "preserve_other_tracks": settings.get("preserveOtherTracks").unwrap_or(&"false".into()).parse::<bool>().unwrap(),
            "pad_with_black":        settings.get("padWithBlack").unwrap_or(&"false".into()).parse::<bool>().unwrap(),
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
