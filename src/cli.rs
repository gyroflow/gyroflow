// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use argh::FromArgs;
use cpp::*;
use gyroflow_core::*;
use std::sync::Arc;
use std::time::Instant;
use qmetaobject::{ QString, QStringList };
use std::cell::RefCell;
use std::io::Write;
use std::collections::HashMap;
use crate::rendering;
use crate::rendering::render_queue::*;

cpp! {{
    #include <QCoreApplication>
}}

/** Gyroflow v1.2.0
Video stabilization using gyroscope data
*/
#[derive(FromArgs)]
struct Opts {
    /// input file
    #[argh(positional)]
    input: String,
}

pub fn run() -> bool {
    if std::env::args().len() > 1 {
        let opts: Opts = argh::from_env();

        if !std::path::Path::new(&opts.input).exists() {
            println!("File {} doesn't exist.", opts.input);
            return false;
        }

        let _time = Instant::now();

        let stab = Arc::new(StabilizationManager::<stabilization::RGBA8>::default());
        stab.lens_profile_db.write().load_all();

        let mut queue = RenderQueue::new(stab.clone());

        rendering::init().unwrap();
        if let Some((name, _list_name)) = gyroflow_core::gpu::initialize_contexts() {
            rendering::set_gpu_type_from_name(&name);
        }

        dbg!(get_saved_settings());

        // Default settings - project file will override this
        // TODO: set this from saved settings
        // Set all default stabilization settings in `stab`
        stab.set_adaptive_zoom(4.0); // Default 4s
        // Sync and export settings
        let additional_data = serde_json::json!({
            "output": {
                "codec":          "H.265/HEVC",
                "codec_options":  "",
                // "output_path":    "E:/__test.mp4",
                // "output_width":   3840,
                // "output_height":  2160,
                // "bitrate":        150,
                "use_gpu":        true,
                "audio":          true,
                "pixel_format":   "",

                // Advanced
                "encoder_options":       "",
                "keyframe_distance":     1,
                "preserve_other_tracks": false,
                "pad_with_black":        false,
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
        });

        queue.set_parallel_renders(1);

        let obj = RefCell::new(queue);
        let obj_ptr = unsafe { qmetaobject::QObjectPinned::new(&obj).get_or_create_cpp_object() };
        unsafe {
            qmetaobject::connect(obj_ptr, obj.borrow().status_changed.to_cpp_representation(&*obj.borrow()), || {
                let q = &mut *obj.as_ptr();
                println!("Status: {}", q.status.to_string());

                if q.status.to_string() == "stopped" && q.get_pending_count() == 0 && q.get_active_render_count() == 0 {
                    println!("All done, quitting");
                    cpp!(unsafe [] { qApp->quit(); });
                }
            });
            qmetaobject::connect(obj_ptr, obj.borrow().progress_changed.to_cpp_representation(&*obj.borrow()), || {
                let q = &mut *obj.as_ptr();
                let c = q.get_current_frame();
                let t = q.get_total_frames();
                print!("\rProgress {:.2}% ({c}/{t})", c as f64 / t as f64 * 100.0);
                std::io::stdout().flush().unwrap();
            });
            // qmetaobject::connect(obj_ptr, obj.borrow().render_progress.to_cpp_representation(&*obj.borrow()), |job_id: &u32, progress: &f64, _current_frame: &usize, _total_frames: &usize, _finished: &bool| {
            //     let q = obj.borrow();
            //     println!("Item progress({}): {:.2}%", job_id, progress);
            // });
            qmetaobject::connect(obj_ptr, obj.borrow().queue_changed.to_cpp_representation(&*obj.borrow()), || {
                println!("Current queue:");
                let q = &mut *obj.as_ptr();
                let qi = q.queue.borrow();
                for item in qi.iter() {
                    println!("- {:?}", &item);
                }
            });
            qmetaobject::connect(obj_ptr, obj.borrow().convert_format.to_cpp_representation(&*obj.borrow()), |job_id: &u32, format: &QString, supported: &QString| {
                println!("Convert format needed job_id: {}, format: {}, supported: {}", job_id, format.to_string(), supported.to_string());
            });
            qmetaobject::connect(obj_ptr, obj.borrow().error.to_cpp_representation(&*obj.borrow()), |job_id: &u32, text: &QString, arg: &QString, callback: &QString| {
                // Always override files
                if text.to_string().starts_with("file_exists") {
                    let q = &mut *obj.as_ptr();
                    q.reset_job(*job_id);
                }
                println!("Error job_id: {}, text: {}, arg: {}, callback: {}", job_id, text.to_string(), arg.to_string(), callback.to_string());
            });
            qmetaobject::connect(obj_ptr, obj.borrow().added.to_cpp_representation(&*obj.borrow()), |job_id: &u32| {
                println!("Added job_id: {}", job_id);
            });
            qmetaobject::connect(obj_ptr, obj.borrow().processing_done.to_cpp_representation(&*obj.borrow()), |job_id: &u32| {
                let q = &mut *obj.as_ptr();
                println!("Processing done job_id: {}", job_id);

                // Modify job
                // let stab = q.get_stab_for_job(*job_id).unwrap();
                // stab.set_lens_correction_amount(0.5);
                // stab.recompute_blocking();

                // Apply preset
                // q.apply_to_all("{...preset data...}", additional_data.to_string());

                qmetaobject::single_shot(std::time::Duration::from_millis(500), move || {
                    q.start(); // Start the rendering queue
                });
            });
        }

        let _job_id = obj.borrow_mut().add_file(opts.input, additional_data.to_string());

        // Run the event loop
        cpp!(unsafe [] {
            int argc = 0;
            QCoreApplication(argc, nullptr).exec();
        });

        println!("Done in {:.3}s", _time.elapsed().as_millis() as f64 / 1000.0);

        return true;
    }

    false
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
