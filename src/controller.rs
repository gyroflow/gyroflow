// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use gyroflow_core::stabilization;
use itertools::Either;
use qmetaobject::*;
use nalgebra::Vector4;
use std::sync::Arc;
use std::cell::RefCell;
use std::sync::atomic::{ AtomicBool, AtomicUsize, Ordering::SeqCst };
use std::collections::BTreeSet;

use qml_video_rs::video_item::MDKVideoItem;

use crate::core;
use crate::core::StabilizationManager;
#[cfg(feature = "opencv")]
use crate::core::calibration::LensCalibrator;
use crate::core::synchronization::AutosyncProcess;
use crate::rendering;
use crate::util;
use crate::wrap_simple_method;
use crate::rendering::FfmpegProcessor;
use crate::ui::components::TimelineGyroChart::TimelineGyroChart;
use crate::qt_gpu::qrhi_undistort;

#[derive(Default, SimpleListItem)]
struct OffsetItem {
    pub timestamp_us: i64,
    pub offset_ms: f64,
}

#[derive(Default, SimpleListItem)]
struct CalibrationItem {
    pub timestamp_us: i64,
    pub sharpness: f64,
    pub is_forced: bool,
}

#[derive(Default, QObject)]
pub struct Controller {
    base: qt_base_class!(trait QObject),

    init_player: qt_method!(fn(&self, player: QJSValue)),
    reset_player: qt_method!(fn(&self, player: QJSValue)),
    load_video: qt_method!(fn(&self, url: QUrl, player: QJSValue)),
    load_telemetry: qt_method!(fn(&self, url: QUrl, is_video: bool, player: QJSValue, chart: QJSValue)),
    load_lens_profile: qt_method!(fn(&mut self, path: String)),
    load_lens_profile_url: qt_method!(fn(&mut self, url: QUrl)),
    export_lens_profile: qt_method!(fn(&mut self, url: QUrl, info: QJsonObject, upload: bool)),
    export_lens_profile_filename: qt_method!(fn(&mut self, info: QJsonObject) -> QString),

    set_sync_method: qt_method!(fn(&self, v: u32)),
    offset_method: qt_property!(u32),
    start_autosync: qt_method!(fn(&self, timestamps_fract: QString, initial_offset: f64, check_negative_initial_offset: bool, sync_search_size: f64, sync_duration_ms: f64, every_nth_frame: u32, for_rs: bool, guess_orientation: bool, override_fps: f64)), // QString is workaround for now
    update_chart: qt_method!(fn(&self, chart: QJSValue)),
    estimate_rolling_shutter: qt_method!(fn(&mut self, timestamp_fract: f64, sync_duration_ms: f64, every_nth_frame: u32, override_fps: f64)),
    rolling_shutter_estimated: qt_signal!(rolling_shutter: f64),
    estimate_bias: qt_method!(fn(&self, timestamp_fract: QString)),
    bias_estimated: qt_signal!(bx: f64, by: f64, bz: f64),
    orientation_guessed: qt_signal!(orientation: QString),

    start_autocalibrate: qt_method!(fn(&self, max_points: usize, every_nth_frame: usize, iterations: usize, max_sharpness: f64, custom_timestamp_ms: f64, no_marker: bool)),

    telemetry_loaded: qt_signal!(is_main_video: bool, filename: QString, camera: QString, imu_orientation: QString, contains_gyro: bool, contains_quats: bool, frame_readout_time: f64, camera_id_json: QString),
    lens_profile_loaded: qt_signal!(lens_json: QString, filepath: QString),

    set_smoothing_method: qt_method!(fn(&self, index: usize) -> QJsonArray),
    get_smoothing_max_angles: qt_method!(fn(&self) -> QJsonArray),
    get_smoothing_status: qt_method!(fn(&self) -> QJsonArray),
    set_smoothing_param: qt_method!(fn(&self, name: QString, val: f64)),
    set_horizon_lock: qt_method!(fn(&self, lock_percent: f64, roll: f64)),
    set_preview_resolution: qt_method!(fn(&mut self, target_height: i32, player: QJSValue)),
    set_background_color: qt_method!(fn(&self, color: QString, player: QJSValue)),
    set_integration_method: qt_method!(fn(&self, index: usize)),

    set_offset: qt_method!(fn(&self, timestamp_us: i64, offset_ms: f64)),
    remove_offset: qt_method!(fn(&self, timestamp_us: i64)),
    clear_offsets: qt_method!(fn(&self)),
    offset_at_video_timestamp: qt_method!(fn(&self, timestamp_us: i64) -> f64),
    offsets_model: qt_property!(RefCell<SimpleListModel<OffsetItem>>; NOTIFY offsets_updated),
    offsets_updated: qt_signal!(),

    load_profiles: qt_method!(fn(&self)),
    all_profiles_loaded: qt_signal!(profiles: QVariantList),
    fetch_profiles_from_github: qt_method!(fn(&self)),
    lens_profiles_updated: qt_signal!(),

    set_sync_lpf: qt_method!(fn(&self, lpf: f64)),
    set_imu_lpf: qt_method!(fn(&self, lpf: f64)),
    set_imu_rotation: qt_method!(fn(&self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64)),
    set_imu_orientation: qt_method!(fn(&self, orientation: String)),
    set_imu_bias: qt_method!(fn(&self, bx: f64, by: f64, bz: f64)),

    override_video_fps: qt_method!(fn(&self, fps: f64)),
    get_scaled_duration_ms: qt_method!(fn(&self) -> f64),
    get_scaled_fps: qt_method!(fn(&self) -> f64),

    recompute_threaded: qt_method!(fn(&mut self)),
    request_recompute: qt_signal!(),

    stab_enabled: qt_property!(bool; WRITE set_stab_enabled),
    show_detected_features: qt_property!(bool; WRITE set_show_detected_features),
    show_optical_flow: qt_property!(bool; WRITE set_show_optical_flow),
    fov: qt_property!(f64; WRITE set_fov),
    frame_readout_time: qt_property!(f64; WRITE set_frame_readout_time),

    adaptive_zoom: qt_property!(f64; WRITE set_adaptive_zoom),
    zooming_center_x: qt_property!(f64; WRITE set_zooming_center_x),
    zooming_center_y: qt_property!(f64; WRITE set_zooming_center_y),

    lens_correction_amount: qt_property!(f64; WRITE set_lens_correction_amount),

    input_horizontal_stretch: qt_property!(f64; WRITE set_input_horizontal_stretch),
    input_vertical_stretch: qt_property!(f64; WRITE set_input_vertical_stretch),

    background_mode: qt_property!(i32; WRITE set_background_mode),
    background_margin: qt_property!(f64; WRITE set_background_margin),
    background_margin_feather: qt_property!(f64; WRITE set_background_margin_feather),

    lens_loaded: qt_property!(bool; NOTIFY lens_changed),
    set_lens_param: qt_method!(fn(&self, param: QString, value: f64)),
    lens_changed: qt_signal!(),

    gyro_loaded: qt_property!(bool; NOTIFY gyro_changed),
    gyro_changed: qt_signal!(),

    compute_progress: qt_signal!(id: u64, progress: f64),
    sync_progress: qt_signal!(progress: f64, ready: usize, total: usize),

    set_video_rotation: qt_method!(fn(&self, angle: f64)),

    set_trim_start: qt_method!(fn(&self, trim_start: f64)),
    set_trim_end: qt_method!(fn(&self, trim_end: f64)),

    set_output_size: qt_method!(fn(&self, width: usize, height: usize)),

    chart_data_changed: qt_signal!(),

    cancel_current_operation: qt_method!(fn(&mut self)),

    sync_in_progress: qt_property!(bool; NOTIFY sync_in_progress_changed),
    sync_in_progress_changed: qt_signal!(),

    calib_in_progress: qt_property!(bool; NOTIFY calib_in_progress_changed),
    calib_in_progress_changed: qt_signal!(),
    calib_progress: qt_signal!(progress: f64, rms: f64, ready: usize, total: usize, good: usize),

    loading_gyro_in_progress: qt_property!(bool; NOTIFY loading_gyro_in_progress_changed),
    loading_gyro_in_progress_changed: qt_signal!(),
    loading_gyro_progress: qt_signal!(progress: f64),

    calib_model: qt_property!(RefCell<SimpleListModel<CalibrationItem>>; NOTIFY calib_model_updated),
    calib_model_updated: qt_signal!(),

    add_calibration_point: qt_method!(fn(&mut self, timestamp_us: i64, no_marker: bool)),
    remove_calibration_point: qt_method!(fn(&mut self, timestamp_us: i64)),

    get_current_fov: qt_method!(fn(&self) -> f64),
    get_scaling_ratio: qt_method!(fn(&self) -> f64),
    get_min_fov: qt_method!(fn(&self) -> f64),

    init_calibrator: qt_method!(fn(&mut self)),

    get_thin_gyroflow_file: qt_method!(fn(&mut self, url: QUrl) -> QJsonObject),
    import_gyroflow_file: qt_method!(fn(&mut self, url: QUrl) -> QJsonObject),
    import_gyroflow_data: qt_method!(fn(&mut self, data: QString) -> QJsonObject),
    export_gyroflow_file: qt_method!(fn(&self, thin: bool, output_options: QJsonObject, override_location: QString, overwrite: bool)),

    check_updates: qt_method!(fn(&self)),
    updates_available: qt_signal!(version: QString, changelog: QString),
    rate_profile: qt_method!(fn(&self, name: QString, is_good: bool)),

    set_zero_copy: qt_method!(fn(&self, player: QJSValue, enabled: bool)),
    set_gpu_decoding: qt_method!(fn(&self, enabled: bool)),

    is_superview: qt_property!(bool; WRITE set_is_superview),

    file_exists: qt_method!(fn(&self, path: QString) -> bool),
    resolve_android_url: qt_method!(fn(&self, url: QString) -> QString),
    open_file_externally: qt_method!(fn(&self, path: QString)),
    get_username: qt_method!(fn(&self) -> QString),

    url_to_path: qt_method!(fn(&self, url: QUrl) -> QString),
    path_to_url: qt_method!(fn(&self, path: QString) -> QUrl),

    image_to_b64: qt_method!(fn(&self, img: QImage) -> QString),

    message: qt_signal!(text: QString, arg: QString, callback: QString),
    error: qt_signal!(text: QString, arg: QString, callback: QString),

    gyroflow_exists: qt_signal!(path: QString, thin: bool),
    request_location: qt_signal!(path: QString, thin: bool),

    preview_resolution: i32,

    cancel_flag: Arc<AtomicBool>,

    ongoing_computations: BTreeSet<u64>,

    pub stabilizer: Arc<StabilizationManager<stabilization::RGBA8>>,
}

impl Controller {
    pub fn new() -> Self {
        Self {
            offset_method: 0,
            preview_resolution: 720,
            ..Default::default()
        }
    }

    fn load_video(&mut self, url: QUrl, player: QJSValue) {
        self.stabilizer.clear();
        self.chart_data_changed();
        self.update_offset_model();
        *self.stabilizer.video_path.write() = util::url_to_path(url.clone());

        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()
            vid.setUrl(url);
        }
    }

    fn start_autosync(&mut self, timestamps_fract: QString, initial_offset: f64, check_negative_initial_offset: bool, sync_search_size: f64, sync_duration_ms: f64, mut every_nth_frame: u32, for_rs: bool, guess_orientation: bool, override_fps: f64) {
        rendering::clear_log();

        if every_nth_frame <= 0 { every_nth_frame = 1; }

        let offset_method = self.offset_method;
        self.sync_in_progress = true;
        self.sync_in_progress_changed();

        let (size, method) = {
            let params = self.stabilizer.params.read();
            (params.size, params.sync_method)
        };

        let timestamps_fract: Vec<f64> = timestamps_fract.to_string().split(';').filter_map(|x| x.parse::<f64>().ok()).collect();

        let progress = util::qt_queued_callback_mut(self, |this, (percent, ready, total): (f64, usize, usize)| {
            this.sync_in_progress = ready < total || percent < 1.0;
            this.sync_in_progress_changed();
            this.chart_data_changed();
            this.sync_progress(percent, ready, total);
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
                    let new_ts = ((x.0 - x.1) * 1000.0) as i64;
                    // Remove existing offsets within 100ms range
                    gyro.remove_offsets_near(new_ts, 100.0);
                    gyro.set_offset(new_ts, x.1);
                }
                this.stabilizer.invalidate_zooming();
            }
            this.update_offset_model();
            this.request_recompute();
        });
        let set_orientation = util::qt_queued_callback_mut(self, move |this, orientation: String| {
            ::log::info!("Setting orientation {}", &orientation);
            this.orientation_guessed(QString::from(orientation));
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
        self.sync_progress(0.0, 0, 0);

        self.cancel_flag.store(false, SeqCst);

        if let Ok(mut sync) = AutosyncProcess::from_manager(&self.stabilizer, method, &timestamps_fract, initial_offset, check_negative_initial_offset, sync_search_size, sync_duration_ms, every_nth_frame, for_rs, guess_orientation, self.cancel_flag.clone()) {
            sync.on_progress(move |percent, ready, total| {
                progress((percent, ready, total));
            });
            sync.on_finished(move |arg| {
                match arg {
                    Either::Left(offsets)=>set_offsets(offsets),
                    Either::Right(Some(orientation))=> set_orientation(orientation.0),
                    _=> ()
                };
            });

            let ranges = sync.get_ranges();
            let cancel_flag = self.cancel_flag.clone();

            let video_path = self.stabilizer.video_path.read().clone();
            let (sw, sh) = (size.0 as u32, size.1 as u32);
            core::run_threaded(move || {
                let gpu_decoding = *rendering::GPU_DECODING.read();

                let mut frame_no = 0;
                let mut abs_frame_no = 0;

                let mut decoder_options = None;
                if override_fps > 0.0 {
                    let fps = rendering::fps_to_rational(override_fps);
                    let mut dict = ffmpeg_next::Dictionary::new();
                    dict.set("framerate", &format!("{}/{}", fps.numerator(), fps.denominator()));
                    decoder_options = Some(dict);
                }

                match FfmpegProcessor::from_file(&video_path, gpu_decoding, 0, decoder_options) {
                    Ok(mut proc) => {
                        proc.on_frame(|timestamp_us, input_frame, _output_frame, converter| {
                            assert!(_output_frame.is_none());

                            if abs_frame_no % every_nth_frame == 0 {
                                match converter.scale(input_frame, ffmpeg_next::format::Pixel::GRAY8, sw, sh) {
                                    Ok(small_frame) => {
                                        let (width, height, stride, pixels) = (small_frame.plane_width(0), small_frame.plane_height(0), small_frame.stride(0), small_frame.data(0));

                                        sync.feed_frame(timestamp_us, frame_no, width, height, stride, pixels);
                                    },
                                    Err(e) => {
                                        err(("An error occured: %1".to_string(), e.to_string()))
                                    }
                                }
                                frame_no += 1;
                            }
                            abs_frame_no += 1;
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

    fn estimate_bias(&mut self, timestamps_fract: QString) {
        let timestamps_fract: Vec<f64> = timestamps_fract.to_string().split(';').filter_map(|x| x.parse::<f64>().ok()).collect();

        let org_duration_ms = self.stabilizer.params.read().duration_ms;

        // sample 400 ms
        let ranges_ms: Vec<(f64, f64)> = timestamps_fract.iter().map(|x| {
            let range = (
                ((x * org_duration_ms) - (200.0)).max(0.0),
                ((x * org_duration_ms) + (200.0)).min(org_duration_ms)
            );
            (range.0, range.1)
        }).collect();

        if !ranges_ms.is_empty() {
            let bias = self.stabilizer.gyro.read().find_bias(ranges_ms[0].0, ranges_ms[0].1);
            self.bias_estimated(bias.0, bias.1, bias.2);
        }
    }

    fn update_chart(&mut self, chart: QJSValue) {
        if let Some(chart) = chart.to_qobject::<TimelineGyroChart>() {
            let chart = unsafe { &mut *chart.as_ptr() }; // _self.borrow_mut();

            chart.setSyncResults(&*self.stabilizer.pose_estimator.estimated_gyro.read());
            chart.setSyncResultsQuats(&*self.stabilizer.pose_estimator.estimated_quats.read());

            chart.setFromGyroSource(&self.stabilizer.gyro.read());
        }
    }

    fn update_offset_model(&mut self) {
        self.offsets_model = RefCell::new(self.stabilizer.gyro.read().get_offsets().iter().map(|(k, v)| OffsetItem {
            timestamp_us: *k,
            offset_ms: *v
        }).collect());

        util::qt_queued_callback(self, |this, _| {
            this.offsets_updated();
            this.chart_data_changed();
        })(());
    }

    fn load_telemetry(&mut self, url: QUrl, is_main_video: bool, player: QJSValue, chart: QJSValue) {
        let s = util::url_to_path(url);
        let stab = self.stabilizer.clone();
        let filename = QString::from(s.split('/').last().unwrap_or_default());
        self.loading_gyro_in_progress = true;
        self.loading_gyro_in_progress_changed();

        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()
            let duration_ms = vid.duration;
            let fps = vid.frameRate;
            let frame_count = vid.frameCount as usize;
            let video_size = (vid.videoWidth as usize, vid.videoHeight as usize);
            self.cancel_flag.store(false, SeqCst);
            let cancel_flag = self.cancel_flag.clone();

            if is_main_video {
                self.set_preview_resolution(self.preview_resolution, player);
            }

            let err = util::qt_queued_callback_mut(self, |this, (msg, arg): (String, String)| {
                this.error(QString::from(msg), QString::from(arg), QString::default());
            });

            let progress = util::qt_queued_callback_mut(self, move |this, progress: f64| {
                this.loading_gyro_in_progress = progress < 1.0;
                this.loading_gyro_progress(progress);
                this.loading_gyro_in_progress_changed();
            });
            let finished = util::qt_queued_callback_mut(self, move |this, params: (bool, QString, QString, QString, bool, bool, f64, QString)| {
                this.gyro_loaded = params.4; // Contains gyro
                this.gyro_changed();

                this.loading_gyro_in_progress = false;
                this.loading_gyro_progress(1.0);
                this.loading_gyro_in_progress_changed();

                this.request_recompute();
                this.update_offset_model();
                this.chart_data_changed();
                this.telemetry_loaded(params.0, params.1, params.2, params.3, params.4, params.5, params.6, params.7);
            });
            let load_lens = util::qt_queued_callback_mut(self, move |this, path: String| {
                this.load_lens_profile(path);
            });
            let reload_lens = util::qt_queued_callback_mut(self, move |this, _| {
                let lens = this.stabilizer.lens.read();
                if this.lens_loaded || !lens.filename.is_empty() {
                    this.lens_loaded = true;
                    this.lens_changed();
                    let json = lens.get_json().unwrap_or_default();
                    this.lens_profile_loaded(QString::from(json), QString::from(lens.filename.as_str()));
                }
            });

            if duration_ms > 0.0 && fps > 0.0 {
                core::run_threaded(move || {
                    let last_progress = RefCell::new(std::time::Instant::now());
                    let progress = |p| {
                        let now = std::time::Instant::now();
                        if (now - *last_progress.borrow()).as_millis() > 100 {
                            progress(p);
                            *last_progress.borrow_mut() = now;
                        }
                    };

                    if is_main_video {
                        if let Err(e) = stab.init_from_video_data(&s, duration_ms, fps, frame_count, video_size) {
                            err(("An error occured: %1".to_string(), e.to_string()));
                        } else {
                            let _ = stab.load_gyro_data(&s, progress, cancel_flag); // Ignore the error here, video file may not contain the telemetry and it's ok

                            if stab.set_output_size(video_size.0, video_size.1) {
                                stab.recompute_undistortion();
                            }
                        }
                    } else if let Err(e) = stab.load_gyro_data(&s, progress, cancel_flag) {
                        err(("An error occured: %1".to_string(), e.to_string()));
                    }
                    stab.recompute_smoothness();

                    let gyro = stab.gyro.read();
                    let detected = gyro.detected_source.as_ref().map(String::clone).unwrap_or_default();
                    let orientation = gyro.imu_orientation.as_ref().map(String::clone).unwrap_or_else(|| "XYZ".into());
                    let has_gyro = !gyro.quaternions.is_empty();
                    let has_quats = !gyro.org_quaternions.is_empty();
                    drop(gyro);

                    if let Some(chart) = chart.to_qobject::<TimelineGyroChart>() {
                        let chart = unsafe { &mut *chart.as_ptr() }; // _self.borrow_mut();
                        chart.setDurationMs(duration_ms);
                    }
                    let camera_id = stab.camera_id.read();

                    let id_str = camera_id.as_ref().map(|v| v.identifier.clone()).unwrap_or_default();
                    if is_main_video && !id_str.is_empty() {
                        let db = stab.lens_profile_db.read();
                        if db.contains_id(&id_str) {
                            load_lens(id_str);
                        }
                    }
                    reload_lens(());

                    let frame_readout_time = stab.params.read().frame_readout_time;
                    let camera_id = camera_id.as_ref().map(|v| v.to_json()).unwrap_or_default();

                    finished((is_main_video, filename, QString::from(detected.trim()), QString::from(orientation), has_gyro, has_quats, frame_readout_time, QString::from(camera_id)));
                });
            }
        }
    }
    fn load_lens_profile_url(&mut self, url: QUrl) {
        self.load_lens_profile(util::url_to_path(url))
    }
    fn load_lens_profile(&mut self, path: String) {
        let (json, filepath) = {
            if let Err(e) = self.stabilizer.load_lens_profile(&path) {
                self.error(QString::from("An error occured: %1"), QString::from(e.to_string()), QString::default());
            }
            let lens = self.stabilizer.lens.read();
            (lens.get_json().unwrap_or_default(), lens.filename.clone())
        };
        self.lens_loaded = true;
        self.lens_changed();
        self.lens_profile_loaded(QString::from(json), QString::from(filepath));
        self.request_recompute();
    }

    fn set_preview_resolution(&mut self, target_height: i32, player: QJSValue) {
        self.preview_resolution = target_height;
        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()

            // fn aligned_to_8(mut x: u32) -> u32 { if x % 8 != 0 { x += 8 - x % 8; } x }

            if !self.stabilizer.video_path.read().is_empty() {
                let h = if target_height > 0 { target_height as u32 } else { vid.videoHeight };
                let ratio = vid.videoHeight as f64 / h as f64;
                let new_w = (vid.videoWidth as f64 / ratio).floor() as u32;
                let new_h = (vid.videoHeight as f64 / (vid.videoWidth as f64 / new_w as f64)).floor() as u32;
                ::log::info!("surface size: {}x{}", new_w, new_h);

                self.stabilizer.pose_estimator.rescale(new_w, new_h);
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
                stab.invalidate_ongoing_computations();

                let mut gyro = stab.gyro.write();
                gyro.integration_method = index;
                gyro.integrate();
                stab.smoothing.write().update_quats_checksum(&gyro.quaternions);
            }
            stab.recompute_smoothness();
            finished(());
        });
    }

    fn set_zero_copy(&self, player: QJSValue, enabled: bool) {
        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()

            if enabled {
                qrhi_undistort::init_player(vid.get_mdkplayer(), self.stabilizer.clone());
            } else {
                qrhi_undistort::deinit_player(vid.get_mdkplayer());
            }
            vid.setCurrentFrame(vid.currentFrame);
        }
    }

    fn set_gpu_decoding(&self, enabled: bool) {
        *rendering::GPU_DECODING.write() = enabled;
    }

    fn reset_player(&self, player: QJSValue) {
        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()
            vid.onResize(Box::new(|_, _| { }));
            vid.onProcessPixels(Box::new(|_, _, _, _, _, _| -> (u32, u32, u32, *mut u8) {
                (0, 0, 0, std::ptr::null_mut())
            }));
            qrhi_undistort::deinit_player(vid.get_mdkplayer());
        }
    }
    fn init_player(&self, player: QJSValue) {
        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()

            let bg_color = vid.getBackgroundColor().get_rgba_f();
            self.stabilizer.params.write().background = Vector4::new(bg_color.0 as f32 * 255.0, bg_color.1 as f32 * 255.0, bg_color.2 as f32 * 255.0, bg_color.3 as f32 * 255.0);

            let stab = self.stabilizer.clone();
            vid.onResize(Box::new(move |width, height| {
                let current_size = stab.params.read().size;
                if current_size.0 != width as usize || current_size.1 != height as usize {
                    stab.set_size(width as usize, height as usize);
                    stab.recompute_threaded(|_|());

                    qrhi_undistort::resize_player(stab.clone());
                }
            }));

            let stab = self.stabilizer.clone();
            let out_pixels = RefCell::new(Vec::new());
            vid.onProcessPixels(Box::new(move |_frame, timestamp_ms, width, height, stride, pixels: &mut [u8]| -> (u32, u32, u32, *mut u8) {
                // let _time = std::time::Instant::now();

                // TODO: cache in atomics instead of locking the mutex every time
                let (ow, oh) = stab.params.read().output_size;
                let os = ow * 4; // Assume RGBA8 - 4 bytes per pixel

                let mut out_pixels = out_pixels.borrow_mut();
                out_pixels.resize_with(os*oh, u8::default);

                let ret = stab.process_pixels((timestamp_ms * 1000.0) as i64, (width as usize, height as usize, stride as usize), (ow, oh, os), pixels, &mut out_pixels);

                // println!("Frame {:.3}, {}x{}, {:.2} MB | OpenCL {:.3}ms", timestamp_ms, width, height, pixels.len() as f32 / 1024.0 / 1024.0, _time.elapsed().as_micros() as f64 / 1000.0);
                if ret {
                    (ow as u32, oh as u32, os as u32, out_pixels.as_mut_ptr())
                } else {
                    (0, 0, 0, std::ptr::null_mut())
                }
            }));
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
        let params = util::serde_json_to_qt_array(&self.stabilizer.set_smoothing_method(index));
        self.request_recompute();
        self.chart_data_changed();
        params
    }
    fn set_smoothing_param(&mut self, name: QString, val: f64) {
        self.stabilizer.set_smoothing_param(&name.to_string(), val);
        self.chart_data_changed();
        self.request_recompute();
    }
    wrap_simple_method!(set_horizon_lock, lock_percent: f64, roll: f64; recompute; chart_data_changed);
    pub fn get_smoothing_algs(&self) -> QVariantList {
        self.stabilizer.get_smoothing_algs().into_iter().map(QString::from).collect()
    }
    fn get_smoothing_status(&self) -> QJsonArray {
        util::serde_json_to_qt_array(&self.stabilizer.get_smoothing_status())
    }
    fn get_smoothing_max_angles(&self) -> QJsonArray {
        let max_angles = self.stabilizer.get_smoothing_max_angles();
        util::serde_json_to_qt_array(&serde_json::json!([max_angles.0, max_angles.1, max_angles.2]))
    }

    fn recompute_threaded(&mut self) {
        let id = self.stabilizer.recompute_threaded(util::qt_queued_callback_mut(self, |this, (id, _discarded): (u64, bool)| {
            if !this.ongoing_computations.contains(&id) {
                ::log::error!("Unknown compute_id: {}", id);
            }
            this.ongoing_computations.remove(&id);
            let finished = this.ongoing_computations.is_empty();
            this.compute_progress(id, if finished { 1.0 } else { 0.0 });
        }));
        self.ongoing_computations.insert(id);

        self.compute_progress(id, 0.0);
    }

    fn estimate_rolling_shutter(&mut self, timestamp_fract: f64, sync_duration_ms: f64, every_nth_frame: u32, override_fps: f64) {
        self.start_autosync(QString::from(format!("{}", timestamp_fract)), 0.0, false, 11.0, sync_duration_ms, every_nth_frame, true, false, override_fps);
    }

    fn cancel_current_operation(&mut self) {
        self.cancel_flag.store(true, SeqCst);
    }

    fn export_gyroflow_file(&self, thin: bool, output_options: QJsonObject, override_location: QString, overwrite: bool) {
        let gf_path = if override_location.is_empty() {
            let video_path = self.stabilizer.video_path.read().clone();
            let video_path = std::path::Path::new(&video_path);
            video_path.with_extension("gyroflow").to_string_lossy().into()
        } else {
            override_location.to_string()
        };

        if !overwrite && std::path::Path::new(&gf_path).exists() {
            self.gyroflow_exists(QString::from(gf_path), thin);
        } else {
            match self.stabilizer.export_gyroflow_file(&gf_path, thin, output_options.to_json().to_string()) {
                Ok(_) => {
                    self.message(QString::from("Gyroflow file exported to %1."), QString::from(format!("<b>{}</b>", gf_path)), QString::default());
                },
                Err(ref e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    self.request_location(QString::from(gf_path), thin);
                },
                Err(e) => {
                    self.error(QString::from("An error occured: %1"), QString::from(e.to_string()), QString::default());
                }
            }
        }
    }

    fn get_thin_gyroflow_file(&mut self, url: QUrl) -> QJsonObject {
        let tmp_stab = StabilizationManager::<stabilization::RGBA8>::default();
        match tmp_stab.import_gyroflow_file(&util::url_to_path(url), false) {
            Ok(thin_obj) => {
                util::serde_json_to_qt_object(&thin_obj)
            },
            Err(e) => {
                self.error(QString::from("An error occured: %1"), QString::from(e.to_string()), QString::default());
                QJsonObject::default()
            }
        }
    }
    fn import_gyroflow_file(&mut self, url: QUrl) -> QJsonObject {
        self.import_gyroflow_internal(self.stabilizer.import_gyroflow_file(&util::url_to_path(url), false))
    }
    fn import_gyroflow_data(&mut self, data: QString) -> QJsonObject {
        self.import_gyroflow_internal(self.stabilizer.import_gyroflow_data(data.to_string().as_bytes(), false, None))
    }
    fn import_gyroflow_internal(&mut self, result: std::io::Result<serde_json::Value>) -> QJsonObject {
        match result {
            Ok(thin_obj) => {
                self.lens_loaded = true;
                self.lens_changed();
                let lens_json = self.stabilizer.lens.read().get_json().unwrap_or_default();
                self.lens_profile_loaded(QString::from(lens_json), QString::default());
                self.request_recompute();
                util::serde_json_to_qt_object(&thin_obj)
            },
            Err(e) => {
                self.error(QString::from("An error occured: %1"), QString::from(e.to_string()), QString::default());
                QJsonObject::default()
            }
        }
    }

    fn set_output_size(&self, w: usize, h: usize) {
        if self.stabilizer.set_output_size(w, h) {
            self.stabilizer.recompute_undistortion();
            self.request_recompute();
            qrhi_undistort::resize_player(self.stabilizer.clone());
        }
    }

    wrap_simple_method!(override_video_fps,         v: f64; recompute; update_offset_model);
    wrap_simple_method!(set_video_rotation,         v: f64; recompute);
    wrap_simple_method!(set_stab_enabled,           v: bool);
    wrap_simple_method!(set_show_detected_features, v: bool);
    wrap_simple_method!(set_show_optical_flow,      v: bool);
    wrap_simple_method!(set_is_superview,           v: bool);
    wrap_simple_method!(set_fov,                v: f64; recompute);
    wrap_simple_method!(set_frame_readout_time, v: f64; recompute);
    wrap_simple_method!(set_adaptive_zoom,      v: f64; recompute);
    wrap_simple_method!(set_zooming_center_x,   v: f64; recompute);
    wrap_simple_method!(set_zooming_center_y,   v: f64; recompute);
    wrap_simple_method!(set_trim_start,         v: f64; recompute; chart_data_changed);
    wrap_simple_method!(set_trim_end,           v: f64; recompute; chart_data_changed);
    wrap_simple_method!(set_sync_method,        v: u32; recompute; chart_data_changed);

    wrap_simple_method!(set_lens_correction_amount,    v: f64; recompute);
    wrap_simple_method!(set_input_horizontal_stretch,  v: f64; recompute);
    wrap_simple_method!(set_input_vertical_stretch,    v: f64; recompute);
    wrap_simple_method!(set_background_mode,           v: i32; recompute);
    wrap_simple_method!(set_background_margin,         v: f64; recompute);
    wrap_simple_method!(set_background_margin_feather, v: f64; recompute);

    wrap_simple_method!(set_offset, timestamp_us: i64, offset_ms: f64; recompute; update_offset_model);
    wrap_simple_method!(clear_offsets,; recompute; update_offset_model);
    wrap_simple_method!(remove_offset, timestamp_us: i64; recompute; update_offset_model);

    wrap_simple_method!(set_imu_lpf, v: f64; recompute; chart_data_changed);
    wrap_simple_method!(set_imu_rotation, pitch_deg: f64, roll_deg: f64, yaw_deg: f64; recompute; chart_data_changed);
    wrap_simple_method!(set_imu_orientation, v: String; recompute; chart_data_changed);
    wrap_simple_method!(set_sync_lpf, v: f64; recompute; chart_data_changed);
    wrap_simple_method!(set_imu_bias, bx: f64, by: f64, bz: f64; recompute; chart_data_changed);

    fn get_scaled_duration_ms(&self) -> f64 { self.stabilizer.params.read().get_scaled_duration_ms() }
    fn get_scaled_fps        (&self) -> f64 { self.stabilizer.params.read().get_scaled_fps() }
    fn get_current_fov       (&self) -> f64 { self.stabilizer.get_current_fov() }
    fn get_scaling_ratio     (&self) -> f64 { self.stabilizer.get_scaling_ratio() }
    fn get_min_fov           (&self) -> f64 { self.stabilizer.get_min_fov() }

    fn offset_at_video_timestamp(&self, timestamp_us: i64) -> f64 {
        self.stabilizer.offset_at_video_timestamp(timestamp_us)
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
            if let Ok(Ok(body)) = ureq::get("https://api.github.com/repos/gyroflow/gyroflow/releases").call().map(|x| x.into_string()) {
                if let Ok(v) = serde_json::from_str(&body) as serde_json::Result<serde_json::Value> {
                    if let Some(obj) = v.as_array().and_then(|x| x.first()).and_then(|x| x.as_object()) {
                        let name = obj.get("name").and_then(|x| x.as_str());
                        let body = obj.get("body").and_then(|x| x.as_str());

                        if let Some(name) = name {
                            ::log::info!("Latest version: {}, current version: {}", name, util::get_version());

                            if let Ok(latest_version) = semver::Version::parse(name.trim_start_matches('v')) {
                                if let Ok(this_version) = semver::Version::parse(env!("CARGO_PKG_VERSION")) {
                                    if latest_version > this_version {
                                        update((name.to_owned(), body.unwrap_or_default().to_owned()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    pub fn init_calibrator(&self) {
        #[cfg(feature = "opencv")]
        {
            self.stabilizer.params.write().is_calibrator = true;
            *self.stabilizer.lens_calibrator.write() = Some(LensCalibrator::new());
            self.stabilizer.set_smoothing_method(2); // Plain 3D
            self.stabilizer.set_smoothing_param("time_constant", 2.0);
        }
    }

    fn start_autocalibrate(&mut self, max_points: usize, every_nth_frame: usize, iterations: usize, max_sharpness: f64, custom_timestamp_ms: f64, no_marker: bool) {
        #[cfg(feature = "opencv")]
        {
            rendering::clear_log();

            self.calib_in_progress = true;
            self.calib_in_progress_changed();
            self.calib_progress(0.0, 0.0, 0, 0, 0);

            let stab = self.stabilizer.clone();

            let (fps, frame_count, trim_start_ms, trim_end_ms, trim_ratio, input_horizontal_stretch, input_vertical_stretch) = {
                let params = stab.params.read();
                let lens = stab.lens.read();
                let input_horizontal_stretch = if lens.input_horizontal_stretch > 0.01 { lens.input_horizontal_stretch } else { 1.0 };
                let input_vertical_stretch = if lens.input_vertical_stretch > 0.01 { lens.input_vertical_stretch } else { 1.0 };
                (params.fps, params.frame_count, params.trim_start * params.duration_ms, params.trim_end * params.duration_ms, params.trim_end - params.trim_start, input_horizontal_stretch, input_vertical_stretch)
            };

            let is_forced = custom_timestamp_ms > -0.5;
            let ranges = if is_forced {
                vec![(custom_timestamp_ms - 1.0, custom_timestamp_ms + 1.0)]
            } else {
                vec![(trim_start_ms, trim_end_ms)]
            };

            let cal = stab.lens_calibrator.clone();
            if max_points > 0 {
                let mut lock = cal.write();
                let cal = lock.as_mut().unwrap();
                let saved: std::collections::BTreeMap<i32, core::calibration::Detected> = {
                    let lock = cal.image_points.read();
                    cal.forced_frames.iter().filter_map(|f| Some((*f, lock.get(f)?.clone()))).collect()
                };
                *cal.image_points.write() = saved;
                cal.max_images = max_points;
                cal.iterations = iterations;
                cal.max_sharpness = max_sharpness;
            }

            let progress = util::qt_queued_callback_mut(self, |this, (ready, total, good, rms): (usize, usize, usize, f64)| {
                this.calib_in_progress = ready < total;
                this.calib_in_progress_changed();
                this.calib_progress(ready as f64 / total as f64, rms, ready, total, good);
                if rms > 0.0 {
                    this.update_calib_model();
                }
            });
            let err = util::qt_queued_callback_mut(self, |this, (msg, mut arg): (String, String)| {
                arg.push_str("\n\n");
                arg.push_str(&rendering::get_log());

                this.error(QString::from(msg), QString::from(arg), QString::default());

                this.calib_in_progress = false;
                this.calib_in_progress_changed();
            });

            self.cancel_flag.store(false, SeqCst);
            let cancel_flag = self.cancel_flag.clone();

            let total = ((frame_count as f64 * trim_ratio) / every_nth_frame as f64) as usize;
            let total_read = Arc::new(AtomicUsize::new(0));
            let processed = Arc::new(AtomicUsize::new(0));

            let video_path = stab.video_path.read().clone();
            core::run_threaded(move || {
                let gpu_decoding = *rendering::GPU_DECODING.read();
                match FfmpegProcessor::from_file(&video_path, gpu_decoding, 0, None) {
                    Ok(mut proc) => {
                        proc.on_frame(|timestamp_us, input_frame, _output_frame, converter| {
                            let frame = core::frame_at_timestamp(timestamp_us as f64 / 1000.0, fps);

                            if is_forced && total_read.load(SeqCst) > 0 {
                                return Ok(());
                            }

                            if (frame % every_nth_frame as i32) == 0 {
                                let mut width = (input_frame.width() as f64 * input_horizontal_stretch).round() as u32;
                                let mut height = (input_frame.height() as f64 * input_vertical_stretch).round() as u32;
                                let mut pt_scale = 1.0;
                                if height > 2160 {
                                    pt_scale = height as f32 / 2160.0;
                                    width = (width as f32 / pt_scale).round() as u32;
                                    height = (height as f32 / pt_scale).round() as u32;
                                }
                                match converter.scale(input_frame, ffmpeg_next::format::Pixel::GRAY8, width, height) {
                                    Ok(mut small_frame) => {
                                        let (width, height, stride, pixels) = (small_frame.plane_width(0), small_frame.plane_height(0), small_frame.stride(0), small_frame.data_mut(0));

                                        total_read.fetch_add(1, SeqCst);
                                        let mut lock = cal.write();
                                        let cal = lock.as_mut().unwrap();
                                        if is_forced {
                                            cal.forced_frames.insert(frame);
                                        }
                                        cal.no_marker = no_marker;
                                        cal.feed_frame(timestamp_us, frame, width, height, stride, pt_scale, pixels, cancel_flag.clone(), total, processed.clone(), progress.clone());
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
                    }
                    Err(error) => {
                        err(("An error occured: %1".to_string(), error.to_string()));
                    }
                }
                // Don't lock the UI trying to draw chessboards while we calibrate
                stab.params.write().is_calibrator = false;

                while processed.load(SeqCst) < total_read.load(SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }

                let mut lock = cal.write();
                let cal = lock.as_mut().unwrap();
                if let Err(e) = cal.calibrate(is_forced) {
                    err(("An error occured: %1".to_string(), format!("{:?}", e)));
                } else {
                    stab.lens.write().set_from_calibrator(cal);
                    ::log::debug!("rms: {}, used_frames: {:?}, camera_matrix: {}, coefficients: {}", cal.rms, cal.used_points.keys(), cal.k, cal.d);
                }

                progress((total, total, 0, cal.rms));

                stab.params.write().is_calibrator = true;
            });
        }
    }

    fn update_calib_model(&mut self) {
        #[cfg(feature = "opencv")]
        {
            let cal = self.stabilizer.lens_calibrator.clone();

            let used_points = cal.read().as_ref().map(|x| x.used_points.clone()).unwrap_or_default();

            self.calib_model = RefCell::new(used_points.iter().map(|(_k, v)| CalibrationItem {
                timestamp_us: v.timestamp_us,
                sharpness: v.avg_sharpness,
                is_forced: v.is_forced
            }).collect());

            util::qt_queued_callback(self, |this, _| {
                this.calib_model_updated();
            })(());
        }
    }

    fn add_calibration_point(&mut self, timestamp_us: i64, no_marker: bool) {
        dbg!(timestamp_us);

        self.start_autocalibrate(0, 1, 1, 1000.0, timestamp_us as f64 / 1000.0, no_marker);
    }
    fn remove_calibration_point(&mut self, timestamp_us: i64) {
        #[cfg(feature = "opencv")]
        {
            let cal = self.stabilizer.lens_calibrator.clone();
            let mut rms = 0.0;
            {
                let mut lock = cal.write();
                let cal = lock.as_mut().unwrap();
                let mut frame_to_remove = None;
                for x in &cal.used_points {
                    if x.1.timestamp_us == timestamp_us {
                        frame_to_remove = Some(*x.0);
                        break;
                    }
                }
                if let Some(f) = frame_to_remove {
                    cal.forced_frames.remove(&f);
                    cal.used_points.remove(&f);
                }
                if cal.calibrate(true).is_ok() {
                    rms = cal.rms;
                    self.stabilizer.lens.write().set_from_calibrator(cal);
                    ::log::debug!("rms: {}, used_frames: {:?}, camera_matrix: {}, coefficients: {}", cal.rms, cal.used_points.keys(), cal.k, cal.d);
                }
            }
            self.update_calib_model();
            if rms > 0.0 {
                self.calib_progress(1.0, rms, 1, 1, 1);
            }
        }
    }

    fn export_lens_profile_filename(&self, info: QJsonObject) -> QString {
        let info_json = info.to_json().to_string();

        if let Ok(mut profile) = core::lens_profile::LensProfile::from_json(&info_json) {
            #[cfg(feature = "opencv")]
            if let Some(ref cal) = *self.stabilizer.lens_calibrator.read() {
                profile.set_from_calibrator(cal);
            }
            return QString::from(format!("{}.json", profile.get_name()));
        }
        QString::default()
    }

    fn export_lens_profile(&mut self, url: QUrl, info: QJsonObject, upload: bool) {
        let path = util::url_to_path(url);
        let info_json = info.to_json().to_string();

        match core::lens_profile::LensProfile::from_json(&info_json) {
            Ok(mut profile) => {
                #[cfg(feature = "opencv")]
                if let Some(ref cal) = *self.stabilizer.lens_calibrator.read() {
                    profile.set_from_calibrator(cal);
                }

                match profile.save_to_file(&path) {
                    Ok(json) => {
                        ::log::debug!("Lens profile json: {}", json);
                        if upload {
                            core::run_threaded(move || {
                                if let Ok(Ok(body)) = ureq::post("https://api.gyroflow.xyz/upload_profile").set("Content-Type", "application/json; charset=utf-8").send_string(&json).map(|x| x.into_string()) {
                                    ::log::debug!("Lens profile uploaded: {}", body.as_str());
                                }
                            });
                        }
                    }
                    Err(e) => { self.error(QString::from("An error occured: %1"), QString::from(format!("{:?}", e)), QString::default()); }
                }
            },
            Err(e) => { self.error(QString::from("An error occured: %1"), QString::from(format!("{:?}", e)), QString::default()); }
        }
    }

    fn load_profiles(&self) {
        let loaded = util::qt_queued_callback_mut(self, |this, all_names: QVariantList| {
            this.all_profiles_loaded(all_names)
        });
        let db = self.stabilizer.lens_profile_db.clone();
        core::run_threaded(move || {
            let mut db = db.write();
            db.load_all();
            // db.list_all_metadata();
            // db.process_adjusted_metadata();
            let all_names = db.get_all_names().into_iter().map(|(name, file)| QVariantList::from_iter([QString::from(name), QString::from(file)].into_iter())).collect();
            loaded(all_names);
        });
    }

    fn fetch_profiles_from_github(&self) {
        #[cfg(target_os = "android")]
        {
            return;
        }

        use crate::core::lens_profile_database::LensProfileDatabase;

        let update = util::qt_queued_callback_mut(self, |this, _| {
            this.lens_profiles_updated();
        });

        core::run_threaded(move || {
            if let Ok(Ok(body)) = ureq::get("https://api.github.com/repos/gyroflow/gyroflow/git/trees/master?recursive=1").call().map(|x| x.into_string()) {
                (|| -> Option<()> {
                    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
                    for obj in v.get("tree")?.as_array()? {
                        let obj = obj.as_object()?;
                        let path = obj.get("path")?.as_str()?;
                        if path.contains("/camera_presets/") && path.contains(".json") {
                            let local_path = LensProfileDatabase::get_path().join(path.replace("resources/camera_presets/", ""));
                            if !local_path.exists() {
                                ::log::info!("Downloading lens profile {:?}", local_path.file_name()?);

                                let url = obj.get("url")?.as_str()?.to_string();
                                let _ = std::fs::create_dir_all(local_path.parent()?);
                                let update = update.clone();
                                core::run_threaded(move || {
                                    let content = ureq::get(&url)
                                        .set("Accept", "application/vnd.github.v3.raw")
                                        .call().map(|x| x.into_string());
                                    if let Ok(Ok(content)) = content {
                                        if std::fs::write(local_path, content.into_bytes()).is_ok() {
                                           update(());
                                        }
                                    }
                                });
                            }
                        }
                    }
                    Some(())
                }());
            }
        });
    }

    fn rate_profile(&self, name: QString, is_good: bool) {
        core::run_threaded(move || {
            if let Ok(Ok(body)) = ureq::post(&format!("https://api.gyroflow.xyz/rate?good={}", is_good)).set("Content-Type", "application/json; charset=utf-8").send_string(&name.to_string()).map(|x| x.into_string()) {
                ::log::debug!("Lens profile rated: {}", body.as_str());
            }
        });
    }

    // Utilities
    fn file_exists(&self, path: QString) -> bool { std::path::Path::new(&path.to_string()).exists() }
    fn resolve_android_url(&mut self, url: QString) -> QString { util::resolve_android_url(url) }
    fn open_file_externally(&self, path: QString) { util::open_file_externally(path); }
    fn get_username(&self) -> QString { let realname = whoami::realname(); QString::from(if realname.is_empty() { whoami::username() } else { realname }) }
    fn url_to_path(&self, url: QUrl) -> QString { QString::from(util::url_to_path(url)) }
    fn path_to_url(&self, path: QString) -> QUrl { util::path_to_url(path) }
    fn image_to_b64(&self, img: QImage) -> QString { util::image_to_b64(img) }
}
