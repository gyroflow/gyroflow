use gyroflow_core::undistortion;
use qmetaobject::*;
use nalgebra::Vector4;
use std::sync::Arc;
use std::cell::RefCell;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::SeqCst;

use qml_video_rs::video_item::MDKVideoItem;

use crate::core;
use crate::core::StabilizationManager;
use crate::core::synchronization::AutosyncProcess;
use crate::rendering;
use crate::util;
use crate::wrap_simple_method;
use crate::rendering::FfmpegProcessor;
use crate::ui::components::TimelineGyroChart::TimelineGyroChart;

#[derive(Default, SimpleListItem)]
struct OffsetItem {
    pub timestamp_us: i64,
    pub offset_ms: f64,
}

#[derive(Default, QObject)]
pub struct Controller { 
    base: qt_base_class!(trait QObject),  
 
    init_player: qt_method!(fn(&self, player: QJSValue)),
    load_video: qt_method!(fn(&self, url: QUrl, player: QJSValue)),
    load_telemetry: qt_method!(fn(&self, url: QUrl, is_video: bool, player: QJSValue, chart: QJSValue)),
    load_lens_profile: qt_method!(fn(&mut self, path: QString)),

    sync_method: qt_property!(u32; WRITE set_sync_method),
    offset_method: qt_property!(u32),
    start_autosync: qt_method!(fn(&self, timestamps_fract: QString, initial_offset: f64, sync_search_size: f64, sync_duration_ms: f64, every_nth_frame: u32, for_rs: bool)), // QString is workaround for now
    update_chart: qt_method!(fn(&self, chart: QJSValue)),
    estimate_rolling_shutter: qt_method!(fn(&mut self, timestamp_fract: f64, sync_duration_ms: f64, every_nth_frame: u32)),
    rolling_shutter_estimated: qt_signal!(rolling_shutter: f64),

    telemetry_loaded: qt_signal!(is_main_video: bool, filename: QString, camera: QString, imu_orientation: QString, contains_gyro: bool, contains_quats: bool, frame_readout_time: f64),
    lens_profile_loaded: qt_signal!(lens_info: QJsonObject),

    set_smoothing_method: qt_method!(fn(&self, index: usize) -> QJsonArray),
    set_smoothing_param: qt_method!(fn(&self, name: QString, val: f64)),
    set_preview_resolution: qt_method!(fn(&mut self, target_height: i32, player: QJSValue)),
    set_background_color: qt_method!(fn(&self, color: QString, player: QJSValue)),
    set_integration_method: qt_method!(fn(&self, index: usize)),

    set_offset: qt_method!(fn(&self, timestamp_us: i64, offset_ms: f64)),
    remove_offset: qt_method!(fn(&self, timestamp_us: i64)),
    offset_at_timestamp: qt_method!(fn(&self, timestamp_us: i64) -> f64),
    offsets_model: qt_property!(RefCell<SimpleListModel<OffsetItem>>; NOTIFY offsets_updated),
    offsets_updated: qt_signal!(),

    set_sync_lpf: qt_method!(fn(&self, lpf: f64)),
    set_imu_lpf: qt_method!(fn(&self, lpf: f64)),
    set_imu_rotation: qt_method!(fn(&self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64)),
    set_imu_orientation: qt_method!(fn(&self, orientation: String)),

    recompute_threaded: qt_method!(fn(&self)),
    request_recompute: qt_signal!(),

    stab_enabled: qt_property!(bool; WRITE set_stab_enabled),
    show_detected_features: qt_property!(bool; WRITE set_show_detected_features),
    show_optical_flow: qt_property!(bool; WRITE set_show_optical_flow),
    fov: qt_property!(f64; WRITE set_fov),
    frame_readout_time: qt_property!(f64; WRITE set_frame_readout_time),
    adaptive_zoom: qt_property!(f64; WRITE set_adaptive_zoom),

    lens_loaded: qt_property!(bool; NOTIFY lens_changed),
    set_lens_param: qt_method!(fn(&self, param: QString, value: f64)),
    lens_changed: qt_signal!(),

    gyro_loaded: qt_property!(bool; NOTIFY gyro_changed),
    gyro_changed: qt_signal!(),

    compute_progress: qt_signal!(id: u64, progress: f64),
    sync_progress: qt_signal!(progress: f64, status: QString),

    set_video_rotation: qt_method!(fn(&self, angle: f64)),

    set_trim_start: qt_method!(fn(&self, trim_start: f64)),
    set_trim_end: qt_method!(fn(&self, trim_end: f64)),

    set_output_size: qt_method!(fn(&self, width: usize, height: usize)),

    chart_data_changed: qt_signal!(),

    render: qt_method!(fn(&self, codec: String, codec_options: String, output_path: String, trim_start: f64, trim_end: f64, output_width: usize, output_height: usize, bitrate: f64, use_gpu: bool, audio: bool)),
    render_progress: qt_signal!(progress: f64, current_frame: usize, total_frames: usize, finished: bool),

    cancel_current_operation: qt_method!(fn(&mut self)),

    sync_in_progress: qt_property!(bool; NOTIFY sync_in_progress_changed),
    sync_in_progress_changed: qt_signal!(),

    export_gyroflow: qt_method!(fn(&self)),

    check_updates: qt_method!(fn(&self)),
    updates_available: qt_signal!(version: QString, changelog: QString),

    file_exists: qt_method!(fn(&self, path: QString) -> bool),
    resolve_android_url: qt_method!(fn(&self, url: QString) -> QString),
    open_file_externally: qt_method!(fn(&self, path: QString)),

    error: qt_signal!(text: QString, arg: QString, callback: QString),

    video_path: String,

    preview_resolution: i32,

    cancel_flag: Arc<AtomicBool>,

    pub stabilizer: Arc<StabilizationManager<undistortion::RGBA8>>,
}

impl Controller {
    pub fn new() -> Self {
        Self {
            sync_method: 1,
            offset_method: 0,
            preview_resolution: 720,
            ..Default::default()
        }
    }

    fn load_video(&mut self, url: QUrl, player: QJSValue) {
        self.stabilizer.clear();
        self.chart_data_changed();
        self.video_path = util::url_to_path(&QString::from(url.clone()).to_string()).to_string();

        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()
            vid.setUrl(url);
        }
    }

    fn start_autosync(&mut self, timestamps_fract: QString, initial_offset: f64, sync_search_size: f64, sync_duration_ms: f64, every_nth_frame: u32, for_rs: bool) {
        rendering::clear_log();

        let method = self.sync_method;
        let offset_method = self.offset_method;
        self.sync_in_progress = true;
        self.sync_in_progress_changed();

        let (fps, size, _duration_ms, _frame_count) = {
            let params = self.stabilizer.params.read(); 
            (params.fps, params.size, params.duration_ms, params.frame_count)
        };

        let timestamps_fract: Vec<f64> = timestamps_fract.to_string().split(';').filter_map(|x| x.parse::<f64>().ok()).collect();

        let progress = util::qt_queued_callback_mut(self, |this, (ready, total): (usize, usize)| {
            this.sync_in_progress = ready < total;
            this.sync_in_progress_changed();
            this.chart_data_changed();
            this.sync_progress(ready as f64 / total as f64, QString::from(format!("{}/{}", ready, total)));
        });
        let set_offsets = util::qt_queued_callback_mut(self, move |this, offsets: Vec<(f64, f64, f64)>| {
            if for_rs {
                if let Some(offs) = offsets.first() {
                    this.rolling_shutter_estimated(offs.1);
                }
            } else {
                let mut gyro = this.stabilizer.gyro.write();
                for x in offsets {
                    ::log::info!("Setting offset at {:.4}: {:.4} (cost {:.4})", x.0, x.1, x.2);
                    gyro.set_offset((x.0 * 1000.0) as i64, x.1);
                }
            }
            this.update_offset_model();
            this.request_recompute();
        });
        let err = util::qt_queued_callback_mut(self, |this, (msg, mut arg): (String, String)| {
            arg.push_str("\n\n");
            arg.push_str(&rendering::get_log());

            this.error(QString::from(msg), QString::from(arg), QString::default());

            this.sync_in_progress = false;
            this.sync_in_progress_changed();
            this.update_offset_model();
            this.request_recompute();
        });

        if let Ok(mut sync) = AutosyncProcess::from_manager(&self.stabilizer, method, &timestamps_fract, initial_offset, sync_search_size, sync_duration_ms, every_nth_frame, for_rs) {
            sync.on_progress(move |ready, total| {
                progress((ready, total));
            });
            sync.on_finished(move |offsets| {
                set_offsets(offsets);
            });

            let ranges = sync.get_ranges();

            self.cancel_flag.store(false, SeqCst);
            let cancel_flag = self.cancel_flag.clone();
            
            let video_path = self.video_path.clone();
            let (sw, sh) = (size.0 as u32, size.1 as u32);
            core::run_threaded(move || {
                match FfmpegProcessor::from_file(&video_path, true) {
                    Ok(mut proc) => {
                        proc.on_frame(|timestamp_us, input_frame, _output_frame, converter| {
                            let frame = core::timestamp_to_frame(timestamp_us as f64 / 1000.0, fps);
      
                            assert!(_output_frame.is_none());

                            if sync.is_frame_wanted(frame) {
                                match converter.scale(input_frame, ffmpeg_next::format::Pixel::GRAY8, sw, sh) {
                                    Ok(mut small_frame) => {
                                        let (width, height, stride, pixels) = (small_frame.plane_width(0), small_frame.plane_height(0), small_frame.stride(0), small_frame.data_mut(0));
            
                                        sync.feed_frame(frame, width, height, stride, pixels, cancel_flag.clone());
                                    },
                                    Err(e) => {
                                        err(("An error occured: %1".to_string(), e.to_string()))
                                    }
                                }
                            }
                            Ok(())
                        });
                        if let Err(e) = proc.start_decoder_only(ranges, cancel_flag.clone()) {
                            err(("An error occured: %1".to_string(), e.to_string()));
                        }
                        sync.finished_feeding_frames(offset_method);
                    }
                    Err(error) => {
                        err(("An error occured: %1".to_string(), error.to_string()));
                    }
                }
            });
        } else {
            err(("An error occured: %1".to_string(), "Invalid parameters".to_string()));
        }
    }

    fn update_chart(&mut self, chart: QJSValue) {
        if let Some(chart) = chart.to_qobject::<TimelineGyroChart>() {
            let chart = unsafe { &mut *chart.as_ptr() }; // _self.borrow_mut();

            chart.setSyncResults(&*self.stabilizer.pose_estimator.estimated_gyro.read());

            chart.setFromGyroSource(&self.stabilizer.gyro.read());
        }
    }

    fn update_offset_model(&mut self) {
        self.offsets_model = RefCell::new(self.stabilizer.gyro.read().offsets.iter().map(|(k, v)| OffsetItem {
            timestamp_us: *k, 
            offset_ms: *v
        }).collect());

        util::qt_queued_callback(self, |this, _| {
            this.offsets_updated();
            this.chart_data_changed();
        })(());
    }

    fn load_telemetry(&mut self, url: QUrl, is_main_video: bool, player: QJSValue, chart: QJSValue) {
        let s = util::url_to_path(&QString::from(url).to_string()).to_string();
        let stab = self.stabilizer.clone();
        let filename = QString::from(s.split('/').last().unwrap_or_default());

        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()
            let duration_ms = vid.duration;
            let fps = vid.frameRate;
            let frame_count = vid.frameCount as usize;
            let video_size = (vid.videoWidth as usize, vid.videoHeight as usize);

            if is_main_video {
                self.set_preview_resolution(self.preview_resolution, player);
            }

            let err = util::qt_queued_callback_mut(self, |this, (msg, arg): (String, String)| {
                this.error(QString::from(msg), QString::from(arg), QString::default());
            });

            let finished = util::qt_queued_callback_mut(self, move |this, params: (bool, QString, QString, QString, bool, bool, f64)| {
                this.gyro_loaded = params.4; // Contains gyro
                this.gyro_changed();
                
                this.request_recompute();
                this.update_offset_model();
                this.chart_data_changed();
                this.telemetry_loaded(params.0, params.1, params.2, params.3, params.4, params.5, params.6);    
            });
            
            if duration_ms > 0.0 && fps > 0.0 {
                core::run_threaded(move || {
                    let detected = {
                        if is_main_video {
                            if let Err(e) = stab.init_from_video_data(&s, duration_ms, fps, frame_count, video_size) {
                                err(("An error occured: %1".to_string(), e.to_string()));
                            } else {
                                stab.set_output_size(video_size.0, video_size.1);
                            }
                        } else if let Err(e) = stab.load_gyro_data(&s) {
                            err(("An error occured: %1".to_string(), e.to_string()));
                        }
                        stab.recompute_smoothness();

                        let gyro = stab.gyro.read();
                        let detected = gyro.detected_source.as_ref().map(String::clone).unwrap_or_default();
                        let orientation = gyro.imu_orientation.as_ref().map(String::clone).unwrap_or("XYZ".into());
                        let has_gyro = !gyro.quaternions.is_empty();
                        let has_quats = !gyro.org_quaternions.is_empty();
                        drop(gyro);

                        if let Some(chart) = chart.to_qobject::<TimelineGyroChart>() {
                            let chart = unsafe { &mut *chart.as_ptr() }; // _self.borrow_mut();
                            chart.setDurationMs(duration_ms);
                        }
                        
                        (detected, orientation, has_gyro, has_quats, stab.params.read().frame_readout_time)
                    };

                    finished((is_main_video, filename, QString::from(detected.0.trim()), QString::from(detected.1), detected.2, detected.3, detected.4));
                });
            }
        }
    }
    fn load_lens_profile(&mut self, path: QString) {
        let info = {
            self.stabilizer.load_lens_profile(&util::url_to_path(&path.to_string()).to_string()); // TODO errors
            QJsonObject::from(self.stabilizer.lens.read().get_info())
        };
        self.lens_loaded = true;
        self.lens_changed();
        self.lens_profile_loaded(info);
        self.request_recompute();
    }
    
    fn set_preview_resolution(&mut self, target_height: i32, player: QJSValue) {
        self.preview_resolution = target_height;
        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()

            // fn aligned_to_8(mut x: u32) -> u32 { if x % 8 != 0 { x += 8 - x % 8; } x }

            if !self.video_path.is_empty() {
                let h = if target_height > 0 { target_height as u32 } else { vid.videoHeight };
                let ratio = vid.videoHeight as f64 / h as f64;
                let new_w = (vid.videoWidth as f64 / ratio).floor() as u32;
                let new_h = (vid.videoHeight as f64 / (vid.videoWidth as f64 / new_w as f64)).floor() as u32;
                ::log::info!("surface size: {}x{}", new_w, new_h);

                self.stabilizer.pose_estimator.clear();
                self.chart_data_changed();

                vid.setSurfaceSize(new_w, new_h);
                vid.setRotation(vid.getRotation());
                vid.setCurrentFrame(vid.currentFrame);
            }
        }
    }

    fn set_integration_method(&mut self, index: usize) {
        let finished = util::qt_queued_callback(self, |this, _| {
            this.chart_data_changed();
            this.request_recompute();
        });

        let stab = self.stabilizer.clone();
        core::run_threaded(move || {
            {
                let mut gyro = stab.gyro.write();
                gyro.integration_method = index;
                gyro.integrate();
                stab.smoothing.write().update_quats_checksum(&gyro.quaternions);
            }
            stab.recompute_smoothness();
            finished(());
        });
    }

    fn init_player(&self, player: QJSValue) {
        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()

            let bg_color = vid.getBackgroundColor().get_rgba_f();
            self.stabilizer.params.write().background = Vector4::new(bg_color.0 as f32 * 255.0, bg_color.1 as f32 * 255.0, bg_color.2 as f32 * 255.0, bg_color.3 as f32 * 255.0);

            let stab = self.stabilizer.clone();
            vid.onResize(Box::new(move |width, height| {
                stab.set_size(width as usize, height as usize);
                stab.recompute_threaded(|_|());
            }));

            let stab = self.stabilizer.clone();
            let out_pixels = RefCell::new(Vec::new());
            vid.onProcessPixels(Box::new(move |_frame, timestamp_ms, width, height, stride, pixels: &mut [u8]| -> (u32, u32, u32, *mut u8) {
                // let _time = std::time::Instant::now();

                // TODO: cache in atomics instead of locking the mutex every time
                let (ow, oh) = {
                    let params = stab.params.read();
                    params.output_size
                };
                let os = ow * 4; // Assume RGBA8 - 4 bytes per pixel

                let mut out_pixels = out_pixels.borrow_mut();
                out_pixels.resize_with(os*oh, u8::default);

                let ret = stab.process_pixels((timestamp_ms * 1000.0) as i64, width as usize, height as usize, stride as usize, ow, oh, os, pixels, &mut out_pixels);
                
                // ::log::info!("Frame {}, {}x{}, {:.2} MB | OpenCL {:.3}ms", frame, width, height, pixels.len() as f32 / 1024.0 / 1024.0, _time.elapsed().as_micros() as f64 / 1000.0);
                if ret {
                    (ow as u32, oh as u32, os as u32, out_pixels.as_mut_ptr())
                } else {
                    (0, 0, 0, std::ptr::null_mut())
                }
            }));

            // crate::qt_gpu::qrhi_undistort::init_player(vid.get_mdkplayer(), self.stabilizer.clone());
        }
    }

    fn set_background_color(&mut self, color: QString, player: QJSValue) {
        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()

            let color = QColor::from_name(&color.to_string());
            vid.setBackgroundColor(color);

            let bg = color.get_rgba_f();
            self.stabilizer.set_background_color(Vector4::new(bg.0 as f32 * 255.0, bg.1 as f32 * 255.0, bg.2 as f32 * 255.0, bg.3 as f32 * 255.0));
        }
    }

    fn set_smoothing_method(&mut self, index: usize) -> QJsonArray {
        let params = util::simd_json_to_qt(&self.stabilizer.set_smoothing_method(index));
        self.request_recompute();
        self.chart_data_changed();
        params
    }
    fn set_smoothing_param(&mut self, name: QString, val: f64) {
        self.stabilizer.set_smoothing_param(&name.to_string(), val);
        self.chart_data_changed();
        self.request_recompute();
    }
    pub fn get_smoothing_algs(&self) -> QVariantList {
        self.stabilizer.get_smoothing_algs().into_iter().map(QString::from).collect()
    }

    fn set_sync_method(&mut self, v: u32) {
        self.sync_method = v;

        self.stabilizer.pose_estimator.clear();
        self.chart_data_changed();
    }

    fn recompute_threaded(&self) {
        let id = self.stabilizer.recompute_threaded(util::qt_queued_callback(self, |this, id: u64| {
            this.compute_progress(id, 1.0);
        }));
        self.compute_progress(id, 0.0);
    }

    fn render(&self, codec: String, codec_options: String, output_path: String, trim_start: f64, trim_end: f64, output_width: usize, output_height: usize, bitrate: f64, use_gpu: bool, audio: bool) {
        rendering::clear_log();

        let progress = util::qt_queued_callback(self, |this, params: (f64, usize, usize, bool)| {
            this.render_progress(params.0, params.1, params.2, params.3);
        });

        let err = util::qt_queued_callback_mut(self, |this, (msg, mut arg): (String, String)| {
            arg.push_str("\n\n");
            arg.push_str(&rendering::get_log());
            this.error(QString::from(msg), QString::from(arg), QString::default());
            this.render_progress(1.0, 0, 0, true);
        });

        let trim_ratio = trim_end - trim_start;
        let total_frame_count = self.stabilizer.params.read().frame_count;
        let video_path = self.video_path.clone();

        progress((0.0, 0, (total_frame_count as f64 * trim_ratio).round() as usize, false));

        self.cancel_flag.store(false, SeqCst);
        let cancel_flag = self.cancel_flag.clone();

        let stab = self.stabilizer.clone();
        core::run_threaded(move || {
            let stab = stab.get_render_stabilizator((output_width, output_height));
            if let Err(e) = rendering::render(stab, progress, video_path, codec, codec_options, output_path, trim_start, trim_end, output_width, output_height, bitrate, use_gpu, audio, cancel_flag) {
                err(("An error occured: %1".to_string(), e.to_string()))
            }
        });
    }

    fn estimate_rolling_shutter(&mut self, timestamp_fract: f64, sync_duration_ms: f64, every_nth_frame: u32) {
        self.start_autosync(QString::from(format!("{}", timestamp_fract)), 0.0, 11.0, sync_duration_ms, every_nth_frame, true);
    }
    
    fn cancel_current_operation(&mut self) {
        self.cancel_flag.store(true, SeqCst);
    }

    fn export_gyroflow(&self) {
        // TODO
    }

    wrap_simple_method!(set_output_size,            w: usize, h: usize; recompute);
    wrap_simple_method!(set_video_rotation,         v: f64; recompute);
    wrap_simple_method!(set_stab_enabled,           v: bool);
    wrap_simple_method!(set_show_detected_features, v: bool);
    wrap_simple_method!(set_show_optical_flow,      v: bool);
    wrap_simple_method!(set_fov,                v: f64; recompute);
    wrap_simple_method!(set_frame_readout_time, v: f64; recompute);
    wrap_simple_method!(set_adaptive_zoom,      v: f64; recompute);
    wrap_simple_method!(set_trim_start,         v: f64; recompute);
    wrap_simple_method!(set_trim_end,           v: f64; recompute);

    wrap_simple_method!(set_offset, timestamp_us: i64, offset_ms: f64; recompute; update_offset_model);
    wrap_simple_method!(remove_offset, timestamp_us: i64; recompute; update_offset_model);

    wrap_simple_method!(set_imu_lpf, v: f64; recompute; chart_data_changed);
    wrap_simple_method!(set_imu_rotation, pitch_deg: f64, roll_deg: f64, yaw_deg: f64; recompute; chart_data_changed);
    wrap_simple_method!(set_imu_orientation, v: String; recompute; chart_data_changed);
    wrap_simple_method!(set_sync_lpf, v: f64; recompute; chart_data_changed);

    fn offset_at_timestamp(&self, timestamp_us: i64) -> f64 {
        self.stabilizer.offset_at_timestamp(timestamp_us)
    }
    fn set_lens_param(&self, param: QString, value: f64) {
        self.stabilizer.set_lens_param(param.to_string().as_str(), value);
        self.request_recompute();
    }

    fn check_updates(&self) {
        let update = util::qt_queued_callback_mut(self, |this, (version, changelog): (String, String)| {
            this.updates_available(QString::from(version), QString::from(changelog))
        });
        core::run_threaded(move || {
            use simd_json::ValueAccess;
            if let Ok(Ok(body)) = ureq::get("https://api.github.com/repos/AdrianEddy/gyroflow/releases").call().map(|x| x.into_string()) {
                let mut slice = body.as_bytes().to_vec();
                let v = simd_json::to_borrowed_value(&mut slice).unwrap();
                if let Some(obj) = v.as_array().and_then(|x| x.first()).and_then(|x| x.as_object()) {
                    let name = obj.get("name").and_then(|x| x.as_str());
                    let body = obj.get("body").and_then(|x| x.as_str());

                    if let Some(name) = name {
                        ::log::info!("Latest version: {}, current version: v{}", name, env!("CARGO_PKG_VERSION"));
                        if name.trim_start_matches('v') != env!("CARGO_PKG_VERSION") {
                            update((name.to_owned(), body.unwrap_or_default().to_owned()));
                        }
                    }
                }
            }
        });
    }

    // Utilities
    fn file_exists(&self, path: QString) -> bool { std::path::Path::new(&path.to_string()).exists() }
    fn resolve_android_url(&mut self, url: QString) -> QString { util::resolve_android_url(url) }
    fn open_file_externally(&self, path: QString) { util::open_file_externally(path); }
}
