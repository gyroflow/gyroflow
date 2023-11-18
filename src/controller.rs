// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use itertools::{Either, Itertools};
use qmetaobject::*;
use nalgebra::Vector4;
use std::sync::Arc;
use std::cell::RefCell;
use std::sync::atomic::{ AtomicBool, AtomicUsize, Ordering::SeqCst };
use std::collections::{ BTreeSet, BTreeMap };
use std::str::FromStr;

use qml_video_rs::video_item::MDKVideoItem;

use crate::core;
use crate::core::StabilizationManager;
#[cfg(feature = "opencv")]
use crate::core::calibration::LensCalibrator;
use crate::core::synchronization::AutosyncProcess;
use crate::core::stabilization::KernelParamsFlags;
use crate::core::synchronization;
use crate::core::keyframes::*;
use crate::core::filesystem;
use crate::rendering;
use crate::util;
use crate::wrap_simple_method;
use crate::rendering::VideoProcessor;
use crate::ui::components::TimelineGyroChart::TimelineGyroChart;
use crate::ui::components::TimelineKeyframesView::TimelineKeyframesView;
use crate::ui::components::FrequencyGraph::FrequencyGraph;
use crate::qt_gpu::qrhi_undistort;

#[derive(Default, SimpleListItem)]
struct OffsetItem {
    pub timestamp_us: i64,
    pub offset_ms: f64,
    pub linear_offset_ms: f64,
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
    video_file_loaded: qt_method!(fn(&self, player: QJSValue)),
    load_telemetry: qt_method!(fn(&self, url: QUrl, is_video: bool, player: QJSValue, sample_index: i32)),
    load_lens_profile: qt_method!(fn(&mut self, url_or_id: QString)),
    export_lens_profile: qt_method!(fn(&mut self, url: QUrl, info: QJsonObject, upload: bool)),
    export_lens_profile_filename: qt_method!(fn(&mut self, info: QJsonObject) -> QString),

    set_of_method: qt_method!(fn(&self, v: u32)),
    start_autosync: qt_method!(fn(&mut self, timestamps_fract: String, sync_params: String, mode: String)),
    update_chart: qt_method!(fn(&self, chart: QJSValue, series: String) -> bool),
    update_frequency_graph: qt_method!(fn(&self, graph: QJSValue, idx: usize, ts: f64, sr: f64, fft_size: usize)),
    update_keyframes_view: qt_method!(fn(&self, kfview: QJSValue)),
    rolling_shutter_estimated: qt_signal!(rolling_shutter: f64),
    estimate_bias: qt_method!(fn(&self, timestamp_fract: QString)),
    bias_estimated: qt_signal!(bx: f64, by: f64, bz: f64),
    orientation_guessed: qt_signal!(orientation: QString),
    get_optimal_sync_points: qt_method!(fn(&mut self, target_sync_points: usize) -> QString),

    start_autocalibrate: qt_method!(fn(&self, max_points: usize, every_nth_frame: usize, iterations: usize, max_sharpness: f64, custom_timestamp_ms: f64, no_marker: bool)),

    telemetry_loaded: qt_signal!(is_main_video: bool, filename: QString, camera: QString, additional_data: QJsonObject),
    lens_profile_loaded: qt_signal!(lens_json: QString, filepath: QString, checksum: QString),

    set_smoothing_method: qt_method!(fn(&self, index: usize) -> QJsonArray),
    get_smoothing_max_angles: qt_method!(fn(&self) -> QJsonArray),
    get_smoothing_status: qt_method!(fn(&self) -> QJsonArray),
    set_smoothing_param: qt_method!(fn(&self, name: QString, val: f64)),
    set_horizon_lock: qt_method!(fn(&self, lock_percent: f64, roll: f64)),
    set_use_gravity_vectors: qt_method!(fn(&self, v: bool)),
    set_horizon_lock_integration_method: qt_method!(fn(&self, v: i32)),
    set_preview_resolution: qt_method!(fn(&mut self, target_height: i32, player: QJSValue)),
    set_processing_resolution: qt_method!(fn(&mut self, target_height: i32)),
    set_background_color: qt_method!(fn(&self, color: QString, player: QJSValue)),
    set_integration_method: qt_method!(fn(&self, index: usize)),

    set_offset: qt_method!(fn(&self, timestamp_us: i64, offset_ms: f64)),
    remove_offset: qt_method!(fn(&self, timestamp_us: i64)),
    clear_offsets: qt_method!(fn(&self)),
    offset_at_video_timestamp: qt_method!(fn(&self, timestamp_us: i64) -> f64),
    offsets_model: qt_property!(RefCell<SimpleListModel<OffsetItem>>; NOTIFY offsets_updated),
    offsets_updated: qt_signal!(),

    load_profiles: qt_method!(fn(&self, reload_from_disk: bool)),
    all_profiles_loaded: qt_signal!(profiles: QVariantList),
    fetch_profiles_from_github: qt_method!(fn(&self)),
    lens_profiles_updated: qt_signal!(reload_from_disk: bool),

    set_sync_lpf: qt_method!(fn(&self, lpf: f64)),
    set_imu_lpf: qt_method!(fn(&self, lpf: f64)),
    set_imu_rotation: qt_method!(fn(&self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64)),
    set_acc_rotation: qt_method!(fn(&self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64)),
    set_imu_orientation: qt_method!(fn(&self, orientation: String)),
    set_imu_bias: qt_method!(fn(&self, bx: f64, by: f64, bz: f64)),
    recompute_gyro: qt_method!(fn(&self)),

    override_video_fps: qt_method!(fn(&self, fps: f64, recompute: bool)),
    get_org_duration_ms: qt_method!(fn(&self) -> f64),
    get_scaled_duration_ms: qt_method!(fn(&self) -> f64),
    get_scaled_fps: qt_method!(fn(&self) -> f64),

    recompute_threaded: qt_method!(fn(&mut self)),
    request_recompute: qt_signal!(),

    stab_enabled: qt_property!(bool; WRITE set_stab_enabled),
    show_detected_features: qt_property!(bool; WRITE set_show_detected_features),
    show_optical_flow: qt_property!(bool; WRITE set_show_optical_flow),
    fov: qt_property!(f64; WRITE set_fov),
    fov_overview: qt_property!(bool; WRITE set_fov_overview),
    show_safe_area: qt_property!(bool; WRITE set_show_safe_area),
    frame_readout_time: qt_property!(f64; WRITE set_frame_readout_time),

    adaptive_zoom: qt_property!(f64; WRITE set_adaptive_zoom),
    zooming_center_x: qt_property!(f64; WRITE set_zooming_center_x),
    zooming_center_y: qt_property!(f64; WRITE set_zooming_center_y),
    zooming_method: qt_property!(i32; WRITE set_zooming_method),

    lens_correction_amount: qt_property!(f64; WRITE set_lens_correction_amount),
    set_video_speed: qt_method!(fn(&self, v: f64, s: bool, z: bool)),

    input_horizontal_stretch: qt_property!(f64; WRITE set_input_horizontal_stretch),
    input_vertical_stretch: qt_property!(f64; WRITE set_input_vertical_stretch),
    lens_is_asymmetrical: qt_property!(bool; WRITE set_lens_is_asymmetrical),

    background_mode: qt_property!(i32; WRITE set_background_mode),
    background_margin: qt_property!(f64; WRITE set_background_margin),
    background_margin_feather: qt_property!(f64; WRITE set_background_margin_feather),

    lens_loaded: qt_property!(bool; NOTIFY lens_changed),
    set_lens_param: qt_method!(fn(&self, param: QString, value: f64)),
    lens_changed: qt_signal!(),

    gyro_loaded: qt_property!(bool; NOTIFY gyro_changed),
    gyro_changed: qt_signal!(),

    has_gravity_vectors: qt_property!(bool; READ has_gravity_vectors NOTIFY gyro_changed),

    compute_progress: qt_signal!(id: u64, progress: f64),
    sync_progress: qt_signal!(progress: f64, ready: usize, total: usize),

    set_video_rotation: qt_method!(fn(&self, angle: f64)),

    set_trim_start: qt_method!(fn(&self, trim_start: f64)),
    set_trim_end: qt_method!(fn(&self, trim_end: f64)),

    set_output_size: qt_method!(fn(&self, width: usize, height: usize)),

    load_default_preset: qt_method!(fn(&mut self)),

    chart_data_changed: qt_signal!(),
    zooming_data_changed: qt_signal!(),
    keyframes_changed: qt_signal!(),

    cancel_current_operation: qt_method!(fn(&mut self)),

    sync_in_progress: qt_property!(bool; NOTIFY sync_in_progress_changed),
    sync_in_progress_changed: qt_signal!(),

    calib_in_progress: qt_property!(bool; NOTIFY calib_in_progress_changed),
    calib_in_progress_changed: qt_signal!(),
    calib_progress: qt_signal!(progress: f64, rms: f64, ready: usize, total: usize, good: usize, sharpness: f64),

    loading_gyro_in_progress: qt_property!(bool; NOTIFY loading_gyro_in_progress_changed),
    loading_gyro_in_progress_changed: qt_signal!(),
    loading_gyro_progress: qt_signal!(progress: f64),

    calib_model: qt_property!(RefCell<SimpleListModel<CalibrationItem>>; NOTIFY calib_model_updated),
    calib_model_updated: qt_signal!(),

    add_calibration_point: qt_method!(fn(&mut self, timestamp_us: i64, no_marker: bool)),
    remove_calibration_point: qt_method!(fn(&mut self, timestamp_us: i64)),

    quats_at_timestamp: qt_method!(fn(&self, timestamp_us: i64) -> QVariantList),
    get_scaling_ratio: qt_method!(fn(&self) -> f64),
    get_min_fov: qt_method!(fn(&self) -> f64),

    init_calibrator: qt_method!(fn(&mut self)),

    get_urls_from_gyroflow_file: qt_method!(fn(&mut self, url: QUrl) -> QStringList),
    import_gyroflow_file: qt_method!(fn(&mut self, url: QUrl)),
    import_gyroflow_data: qt_method!(fn(&mut self, data: QString)),
    gyroflow_file_loaded: qt_signal!(obj: QJsonObject),
    export_gyroflow_file: qt_method!(fn(&self, url: QUrl, typ: QString, additional_data: QJsonObject)),
    export_gyroflow_data: qt_method!(fn(&self, typ: QString, additional_data: QJsonObject) -> QString),

    input_file_url: qt_property!(QString; READ get_input_file_url NOTIFY input_file_url_changed),
    input_file_url_changed: qt_signal!(),

    project_file_url: qt_property!(QString; READ get_project_file_url NOTIFY project_file_url_changed),
    project_file_url_changed: qt_signal!(),

    check_updates: qt_method!(fn(&self)),
    updates_available: qt_signal!(version: QString, changelog: QString),
    rate_profile: qt_method!(fn(&self, name: QString, json: QString, checksum: QString, is_good: bool)),
    request_profile_ratings: qt_method!(fn(&self)),

    set_preview_pipeline: qt_method!(fn(&self, index: i32)),
    set_gpu_decoding: qt_method!(fn(&self, enabled: bool)),

    list_gpu_devices: qt_method!(fn(&self)),
    set_device: qt_method!(fn(&self, i: i32)),
    set_rendering_gpu_type_from_name: qt_method!(fn(&self, name: String)),
    gpu_list_loaded: qt_signal!(list: QJsonArray),

    set_digital_lens_name: qt_method!(fn(&self, name: String)),
    set_digital_lens_param: qt_method!(fn(&self, index: usize, value: f64)),

    get_username: qt_method!(fn(&self) -> QString),
    clear_settings: qt_method!(fn(&self)),
    copy_to_clipboard: qt_method!(fn(&self, text: QString)),

    image_to_b64: qt_method!(fn(&self, img: QImage) -> QString),
    export_preset: qt_method!(fn(&self, url: QUrl, data: QJsonObject)),

    message: qt_signal!(text: QString, arg: QString, callback: QString, id: QString),
    error: qt_signal!(text: QString, arg: QString, callback: QString),

    request_location: qt_signal!(url: QString, typ: QString),

    set_keyframe: qt_method!(fn(&self, typ: String, timestamp_us: i64, value: f64)),
    set_keyframe_easing: qt_method!(fn(&self, typ: String, timestamp_us: i64, easing: String)),
    keyframe_easing: qt_method!(fn(&self, typ: String, timestamp_us: i64) -> String),
    remove_keyframe: qt_method!(fn(&self, typ: String, timestamp_us: i64)),
    clear_keyframes_type: qt_method!(fn(&self, typ: String)),
    keyframe_value_at_video_timestamp: qt_method!(fn(&self, typ: String, timestamp_ms: f64) -> QJSValue),
    is_keyframed: qt_method!(fn(&self, typ: String) -> bool),
    set_prevent_recompute: qt_method!(fn(&self, v: bool)),

    keyframe_value_updated: qt_signal!(keyframe: String, value: f64),
    update_keyframe_values: qt_method!(fn(&self, timestamp_ms: f64)),

    check_external_sdk: qt_method!(fn(&self, filename: QString) -> bool),
    install_external_sdk: qt_method!(fn(&self, url: QString)),
    external_sdk_progress: qt_signal!(percent: f64, sdk_name: QString, error_string: QString, url: QString),

    mp4_merge: qt_method!(fn(&self, file_list: QStringList, output_folder: QUrl, output_filename: QString)),
    mp4_merge_progress: qt_signal!(percent: f64, error_string: QString, url: QString),

    // ---------- REDline conversion ----------
    find_redline: qt_method!(fn(&self) -> QString),
    // ---------- REDline conversion ----------

    play_sound: qt_method!(fn(&mut self, typ: String)),

    image_sequence_start: qt_property!(i32),
    image_sequence_fps: qt_property!(f64),

    preview_resolution: i32,
    processing_resolution: i32,

    current_fov: qt_property!(f64; NOTIFY processing_info_changed),
    current_minimal_fov: qt_property!(f64; NOTIFY processing_info_changed),
    current_focal_length: qt_property!(f64; NOTIFY processing_info_changed),
    processing_info: qt_property!(QString; NOTIFY processing_info_changed),
    processing_info_changed: qt_signal!(),

    cancel_flag: Arc<AtomicBool>,
    preview_pipeline: Arc<AtomicUsize>,

    ongoing_computations: BTreeSet<u64>,

    pub stabilizer: Arc<StabilizationManager>,
}

impl Controller {
    pub fn new() -> Self {
        Self {
            preview_resolution: -1,
            processing_resolution: 720,
            ..Default::default()
        }
    }

    fn load_video(&mut self, url: QUrl, player: QJSValue) {
        self.stabilizer.clear();
        let url = util::qurl_to_encoded(url.clone());
        let filename = filesystem::get_filename(&url);

        // Load current (clean) state to the UI
        if let Ok(current_state) = self.stabilizer.export_gyroflow_data(core::GyroflowProjectType::Simple, "{}", None) {
            if let Ok(current_state) = serde_json::from_str(current_state.as_str()) as serde_json::Result<serde_json::Value> {
                self.gyroflow_file_loaded(util::serde_json_to_qt_object(&current_state));
            }
        }

        self.chart_data_changed();
        self.keyframes_changed();
        self.update_offset_model();

        *self.stabilizer.input_file.write() = gyroflow_core::InputFile {
            url: url.clone(),
            project_file_url: None,
            image_sequence_start: self.image_sequence_start,
            image_sequence_fps: self.image_sequence_fps
        };
        self.input_file_url_changed();
        self.project_file_url_changed();

        let mut custom_decoder = String::new(); // eg. BRAW:format=rgba64le
        if self.image_sequence_start > 0 {
            custom_decoder = format!("FFmpeg:avformat_options=start_number={}", self.image_sequence_start);
        }

        let options = {
            let target_height = self.preview_resolution;
            if target_height > 0 {
                format!(":scale={}x{}", (target_height * 16) / 9, target_height)
            } else {
                "".to_owned()
            }
        };

        if filename.to_ascii_lowercase().ends_with("braw") {
            let gpu = if *rendering::GPU_DECODING.read() { "auto" } else { "no" }; // Disable GPU decoding for BRAW
            custom_decoder = format!("BRAW:gpu={}{}", gpu, options);
        }
        if filename.to_ascii_lowercase().ends_with("r3d") {
            custom_decoder = format!("R3D:gpu=auto{}", options);
        }
        if !custom_decoder.is_empty() {
            ::log::debug!("Custom decoder: {custom_decoder}");
        }

        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()
            filesystem::stop_accessing_url(&util::qurl_to_encoded(vid.url.clone()), false);
            filesystem::start_accessing_url(&url, false);
            vid.setUrl(QUrl::from(QString::from(url)), QString::from(custom_decoder));
        }
    }

    fn get_input_file_url(&self) -> QString {
        QString::from(self.stabilizer.input_file.read().url.clone())
    }
    fn get_project_file_url(&self) -> QString {
        QString::from(self.stabilizer.input_file.read().project_file_url.as_ref().cloned().unwrap_or_default())
    }

    fn start_autosync(&mut self, timestamps_fract: String, sync_params: String, mode: String) {
        rendering::clear_log();

        let sync_params = serde_json::from_str(&sync_params) as serde_json::Result<synchronization::SyncParams>;
        if let Err(e) = sync_params {
            self.sync_in_progress = false;
            self.sync_in_progress_changed();
            return self.error(QString::from("An error occured: %1"), QString::from(format!("JSON parse error: {}", e)), QString::default());
        }
        let mut sync_params = sync_params.unwrap();

        sync_params.initial_offset     *= 1000.0; // s to ms
        sync_params.time_per_syncpoint *= 1000.0; // s to ms
        sync_params.search_size        *= 1000.0; // s to ms
        sync_params.every_nth_frame     = sync_params.every_nth_frame.max(1);

        let for_rs = mode == "estimate_rolling_shutter";

        let every_nth_frame = sync_params.every_nth_frame;

        self.sync_in_progress = true;
        self.sync_in_progress_changed();

        let timestamps_fract: Vec<f64> = timestamps_fract.split(';').filter_map(|x| x.parse::<f64>().ok()).collect();

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
                gyro.prevent_recompute = true;
                for x in offsets {
                    ::log::info!("Setting offset at {:.4}: {:.4} (cost {:.4})", x.0, x.1, x.2);
                    let new_ts = ((x.0 - x.1) * 1000.0) as i64;
                    // Remove existing offsets within 100ms range
                    gyro.remove_offsets_near(new_ts, 100.0);
                    gyro.set_offset(new_ts, x.1);
                }
                gyro.prevent_recompute = false;
                gyro.adjust_offsets();
                this.stabilizer.keyframes.write().update_gyro(&gyro);
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

        if let Ok(mut sync) = AutosyncProcess::from_manager(&self.stabilizer, &timestamps_fract, sync_params, mode, self.cancel_flag.clone()) {
            sync.on_progress(move |percent, ready, total| {
                progress((percent, ready, total));
            });
            sync.on_finished(move |arg| {
                match arg {
                    Either::Left(offsets) => set_offsets(offsets),
                    Either::Right(Some(orientation)) => set_orientation(orientation.0),
                    _=> ()
                };
            });

            let ranges = sync.get_ranges();
            let cancel_flag = self.cancel_flag.clone();

            let input_file = self.stabilizer.input_file.read().clone();
            let proc_height = self.processing_resolution;
            core::run_threaded(move || {
                let gpu_decoding = *rendering::GPU_DECODING.read();

                let mut frame_no = 0;
                let mut abs_frame_no = 0;

                let mut decoder_options = ffmpeg_next::Dictionary::new();
                if input_file.image_sequence_fps > 0.0 {
                    let fps = rendering::fps_to_rational(input_file.image_sequence_fps);
                    decoder_options.set("framerate", &format!("{}/{}", fps.numerator(), fps.denominator()));
                }
                if input_file.image_sequence_start > 0 {
                    decoder_options.set("start_number", &format!("{}", input_file.image_sequence_start));
                }
                if proc_height > 0 {
                    decoder_options.set("scale", &format!("{}x{}", (proc_height * 16) / 9, proc_height));
                }
                ::log::debug!("Decoder options: {:?}", decoder_options);

                let sync = std::rc::Rc::new(sync);

                let fs_base = gyroflow_core::filesystem::get_engine_base();
                match VideoProcessor::from_file(&fs_base, &input_file.url, gpu_decoding, 0, Some(decoder_options)) {
                    Ok(mut proc) => {
                        let err2 = err.clone();
                        let sync2 = sync.clone();
                        proc.on_frame(move |timestamp_us, input_frame, _output_frame, converter, _rate_control| {
                            assert!(_output_frame.is_none());

                            if abs_frame_no % every_nth_frame == 0 {
                                let h = if proc_height > 0 { proc_height as u32 } else { input_frame.height() };
                                let ratio = input_frame.height() as f64 / h as f64;
                                let sw = (input_frame.width() as f64 / ratio).round() as u32;
                                let sh = (input_frame.height() as f64 / (input_frame.width() as f64 / sw as f64)).round() as u32;
                                match converter.scale(input_frame, ffmpeg_next::format::Pixel::GRAY8, sw, sh) {
                                    Ok(small_frame) => {
                                        let (width, height, stride, pixels) = (small_frame.plane_width(0), small_frame.plane_height(0), small_frame.stride(0), small_frame.data(0));

                                        sync2.feed_frame(timestamp_us, frame_no, width, height, stride, pixels);
                                    },
                                    Err(e) => {
                                        err2(("An error occured: %1".to_string(), e.to_string()))
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
                        sync.finished_feeding_frames();
                    }
                    Err(error) => {
                        err(("An error occured: %1".to_string(), error.to_string()));
                    }
                };
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

    fn get_optimal_sync_points(&mut self, target_sync_points: usize) -> QString {
        let dur_ms = self.stabilizer.params.read().get_scaled_duration_ms();
        let trim_start = self.stabilizer.params.read().trim_start * dur_ms / 1000.0;
        let trim_end = self.stabilizer.params.read().trim_end * dur_ms / 1000.0;
        if let Some(mut optsync) = core::synchronization::optimsync::OptimSync::new(&self.stabilizer.gyro.read()) {
            let s: String = optsync.run(target_sync_points, trim_start, trim_end).iter().map(|x| x / dur_ms).map(|x| x.to_string()).join(";").chars().collect();
            QString::from(s)
        } else {
            QString::default()
        }
    }

    fn update_chart(&mut self, chart: QJSValue, series: String) -> bool {
        if let Some(chart) = chart.to_qobject::<TimelineGyroChart>() {
            let chart = unsafe { &mut *chart.as_ptr() }; // _self.borrow_mut();

            if self.stabilizer.pose_estimator.estimated_gyro.is_locked() ||
               self.stabilizer.pose_estimator.estimated_quats.is_locked() ||
               self.stabilizer.gyro.is_locked() ||
               self.stabilizer.params.is_locked() {
                ::log::debug!("Chart mutex locked, retrying");
                return false;
            }

            if series.is_empty() {
                if let Some(est_gyro) = self.stabilizer.pose_estimator.estimated_gyro.try_read() {
                    chart.setSyncResults(&est_gyro);
                    if let Some(est_quats) = self.stabilizer.pose_estimator.estimated_quats.try_read() {
                        chart.setSyncResultsQuats(&est_quats);
                    }
                }
            }

            if let Some(gyro) = self.stabilizer.gyro.try_read() {
                if let Some(params) = self.stabilizer.params.try_read() {
                    if let Some(keyframes) = self.stabilizer.keyframes.try_read() {
                        chart.setFromGyroSource(&gyro, &params, &keyframes, &series);
                        return true;
                    }
                }
            }
        }
        false
    }

    fn update_frequency_graph(&mut self, graph: QJSValue, idx: usize, ts: f64, sr: f64, fft_size: usize) {
        if let Some(graph) = graph.to_qobject::<FrequencyGraph>() {
            let graph = unsafe { &mut *graph.as_ptr() }; // _self.borrow_mut();

            let gyro = &self.stabilizer.gyro.read();
            let raw_imu = &gyro.raw_imu;

            if !raw_imu.is_empty() {
                let dt_ms = 1000.0 / sr;
                let center_ts = ts - gyro.offset_at_video_timestamp(ts);
                let last_ts  = center_ts + dt_ms * (fft_size as f64)/2.0;
                let mut sample_ts = last_ts.min(raw_imu.last().unwrap().timestamp_ms) - (fft_size as f64) * dt_ms;
                sample_ts = sample_ts.max(0.0);

                let mut prev_ts = 0.0;
                let mut prev_val = 0.0;

                let mut samples: Vec<f64> = Vec::with_capacity(fft_size);
                for x in raw_imu {
                    let mut val = 0.0;
                    if idx < 3 {
                        if let Some(g) = x.gyro.as_ref() {
                            val = g[idx % 3];
                        }
                    } else {
                        if let Some(g) = x.accl.as_ref() {
                            val = g[idx % 3];
                        }
                    }

                    while x.timestamp_ms > sample_ts && samples.len() < fft_size {
                        let frac = (sample_ts - prev_ts) / (x.timestamp_ms - prev_ts);
                        let interpolated = prev_val + (val - prev_val) * frac.clamp(0.0, 1.0);
                        samples.push(interpolated /*+ samples.last().unwrap_or(&0.0)*/);
                        sample_ts += dt_ms;
                    }

                    if samples.len() >= fft_size {
                        break;
                    }

                    prev_ts = x.timestamp_ms;
                    prev_val = val;
                }

                if samples.len() == fft_size {
                    graph.setData(&samples, sr);
                } else {
                    graph.setData(&[], 0.0);
                }
            }
        }
    }

    fn update_keyframes_view(&mut self, view: QJSValue) {
        if let Some(view) = view.to_qobject::<TimelineKeyframesView>() {
            let view = unsafe { &mut *view.as_ptr() }; // _self.borrow_mut();

            view.setKeyframes(&self.stabilizer.keyframes.read());
        }
    }

    fn update_offset_model(&mut self) {
        self.offsets_model = RefCell::new(self.stabilizer.gyro.read().get_offsets_plus_linear().iter().map(|(k, v)| OffsetItem {
            timestamp_us: *k,
            offset_ms: v.0,
            linear_offset_ms: v.1
        }).collect());

        util::qt_queued_callback(self, |this, _| {
            this.offsets_updated();
            this.chart_data_changed();
        })(());
    }

    fn video_file_loaded(&mut self, player: QJSValue) {
        let stab = self.stabilizer.clone();

        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()
            let duration_ms = vid.duration;
            let fps = vid.frameRate;
            let frame_count = vid.frameCount as usize;
            let video_size = (vid.videoWidth as usize, vid.videoHeight as usize);

            self.set_preview_resolution(self.preview_resolution, player);

            if duration_ms > 0.0 && fps > 0.0 {
                stab.init_from_video_data(duration_ms, fps, frame_count, video_size);
                stab.set_output_size(video_size.0, video_size.1);
            }
        }
    }

    fn load_telemetry(&mut self, url: QUrl, is_main_video: bool, player: QJSValue, sample_index: i32) {
        let url = util::qurl_to_encoded(url);
        let stab = self.stabilizer.clone();
        let filename = filesystem::get_filename(&url);

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
            let stab2 = stab.clone();
            let finished = util::qt_queued_callback_mut(self, move |this, params: (bool, QString, QString, bool, serde_json::Value)| {
                this.gyro_loaded = params.3; // Contains motion
                this.gyro_changed();

                this.loading_gyro_in_progress = false;
                this.loading_gyro_progress(1.0);
                this.loading_gyro_in_progress_changed();

                this.update_offset_model();
                this.chart_data_changed();

                this.telemetry_loaded(params.0, params.1, params.2, util::serde_json_to_qt_object(&params.4));

                stab2.invalidate_ongoing_computations();
                stab2.invalidate_smoothing();
                this.request_recompute();
            });
            let load_lens = util::qt_queued_callback_mut(self, move |this, path: String| {
                this.load_lens_profile(path.into());
            });
            let reload_lens = util::qt_queued_callback_mut(self, move |this, _| {
                let lens = this.stabilizer.lens.read();
                if this.lens_loaded || !lens.path_to_file.is_empty() {
                    this.lens_loaded = true;
                    this.lens_changed();
                    let json = lens.get_json().unwrap_or_default();
                    this.lens_profile_loaded(QString::from(json), QString::from(lens.path_to_file.as_str()), QString::from(lens.checksum.clone().unwrap_or_default()));
                }
            });

            if duration_ms > 0.0 && fps > 0.0 {
                self.loading_gyro_in_progress = true;
                self.loading_gyro_in_progress_changed();
                core::run_threaded(move || {
                    let mut additional_data = serde_json::Value::Object(serde_json::Map::new());
                    let additional_obj = additional_data.as_object_mut().unwrap();
                    if is_main_video {
                        stab.init_from_video_data(duration_ms, fps, frame_count, video_size);
                        // Ignore the error here, video file may not contain the telemetry and it's ok
                        let _ = stab.load_gyro_data(&url, is_main_video, &Default::default(), progress, cancel_flag);

                        if stab.set_output_size(video_size.0, video_size.1) {
                            stab.recompute_undistortion();
                        }
                    } else {
                        let mut options = gyroflow_core::gyro_source::FileLoadOptions::default();
                        if sample_index > -1 {
                            options.sample_index = Some(sample_index as usize);
                        }

                        if let Err(e) = stab.load_gyro_data(&url, is_main_video, &options, progress, cancel_flag) {
                            err(("An error occured: %1".to_string(), e.to_string()));
                        }
                    }
                    stab.recompute_smoothness();

                    let gyro = stab.gyro.read();
                    let detected = gyro.file_metadata.detected_source.as_ref().map(String::clone).unwrap_or_default();
                    let has_raw_gyro = !gyro.file_metadata.raw_imu.is_empty();
                    let has_quats = !gyro.file_metadata.quaternions.is_empty();
                    let has_motion = has_raw_gyro || has_quats;
                    additional_obj.insert("imu_orientation".to_owned(),   serde_json::Value::String(gyro.imu_orientation.clone().unwrap_or_else(|| "XYZ".into())));
                    additional_obj.insert("contains_raw_gyro".to_owned(), serde_json::Value::Bool(has_raw_gyro));
                    additional_obj.insert("contains_quats".to_owned(),    serde_json::Value::Bool(has_quats));
                    additional_obj.insert("contains_motion".to_owned(),   serde_json::Value::Bool(has_motion));
                    additional_obj.insert("has_accurate_timestamps".to_owned(), serde_json::Value::Bool(gyro.file_metadata.has_accurate_timestamps));
                    additional_obj.insert("sample_rate".to_owned(),       serde_json::to_value(gyroflow_core::gyro_source::GyroSource::get_sample_rate(&gyro.file_metadata)).unwrap());
                    let has_builtin_profile = gyro.file_metadata.lens_profile.as_ref().map(|y| y.is_object()).unwrap_or_default();
                    let md_data = gyro.file_metadata.additional_data.clone();
                    if let Some(md_fps) = gyro.file_metadata.frame_rate {
                        let fps = stab.params.read().fps;
                        if (md_fps - fps).abs() > 1.0 {
                            additional_obj.insert("realtime_fps".to_owned(), serde_json::Number::from_f64(md_fps).unwrap().into());
                        }
                    }
                    drop(gyro);

                    let camera_id = stab.camera_id.read();

                    let id_str = camera_id.as_ref().map(|v| v.get_identifier_for_autoload()).unwrap_or_default();
                    if is_main_video && !id_str.is_empty() && !has_builtin_profile {
                        let mut db = stab.lens_profile_db.write();
                        db.on_loaded(move |db| {
                            if db.contains_id(&id_str) {
                                load_lens(id_str);
                            }
                        });
                    }
                    if is_main_video {
                        reload_lens(());
                    }

                    additional_obj.insert("frame_readout_time".to_owned(), serde_json::to_value(stab.params.read().frame_readout_time).unwrap());
                    if let Some(cam_id) = camera_id.as_ref() {
                        additional_obj.insert("camera_identifier".to_owned(), serde_json::to_value(cam_id).unwrap());
                    }

                    if md_data.is_object() {
                        gyroflow_core::util::merge_json(&mut additional_data, &md_data);
                    }

                    finished((is_main_video, filename.into(), QString::from(detected.trim()), has_motion, additional_data));
                });
            }
        }
    }
    fn load_lens_profile(&mut self, url_or_id: QString) {
        let (json, filepath, checksum) = {
            if let Err(e) = self.stabilizer.load_lens_profile(&url_or_id.to_string()) {
                self.error(QString::from("An error occured: %1"), QString::from(e.to_string()), QString::default());
            }
            let lens = self.stabilizer.lens.read();
            (lens.get_json().unwrap_or_default(), lens.path_to_file.clone(), lens.checksum.clone().unwrap_or_default())
        };
        self.lens_loaded = true;
        self.lens_changed();
        self.lens_profile_loaded(QString::from(json), QString::from(filepath), QString::from(checksum));
        self.request_recompute();
    }
    fn load_default_preset(&mut self) {
        // Assumes regular filesystem
        let local_path = gyroflow_core::lens_profile_database::LensProfileDatabase::get_path().join("default.gyroflow");
        if local_path.exists() {
            self.import_gyroflow_file(QUrl::from(QString::from(filesystem::path_to_url(&local_path.to_string_lossy()))));
        }
    }

    fn set_preview_resolution(&mut self, target_height: i32, player: QJSValue) {
        self.preview_resolution = target_height;
        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()

            // fn aligned_to_8(mut x: u32) -> u32 { if x % 8 != 0 { x += 8 - x % 8; } x }

            if !self.stabilizer.input_file.read().url.is_empty() {
                let h = if target_height > 0 { target_height as u32 } else { vid.videoHeight };
                let ratio = vid.videoHeight as f64 / h as f64;
                let new_w = (vid.videoWidth as f64 / ratio).floor() as u32;
                let new_h = (vid.videoHeight as f64 / (vid.videoWidth as f64 / new_w as f64)).floor() as u32;
                ::log::info!("surface size: {}x{}", new_w, new_h);

                self.chart_data_changed();

                vid.setSurfaceSize(new_w, new_h);
                vid.setRotation(vid.getRotation());
                // vid.setCurrentFrame(vid.currentFrame);
            }
        }
    }

    fn set_processing_resolution(&mut self, target_height: i32) {
        self.processing_resolution = target_height;
        self.stabilizer.pose_estimator.clear();
        self.chart_data_changed();
    }

    fn set_integration_method(&mut self, index: usize) {
        let finished = util::qt_queued_callback(self, |this, _| {
            this.chart_data_changed();
            this.request_recompute();
        });

        let stab = self.stabilizer.clone();

        if stab.gyro.read().integration_method == index {
            return;
        }

        core::run_threaded(move || {
            {
                stab.invalidate_ongoing_computations();

                let mut gyro = stab.gyro.write();
                gyro.integration_method = index;
                gyro.integrate();
            }
            stab.invalidate_smoothing();
            finished(());
        });
    }

    fn set_preview_pipeline(&self, index: i32) {
        self.preview_pipeline.store(index as usize, SeqCst);
    }

    fn set_prevent_recompute(&self, v: bool) {
        self.stabilizer.prevent_recompute.store(v, SeqCst);
    }

    fn set_gpu_decoding(&self, enabled: bool) {
        *rendering::GPU_DECODING.write() = enabled;
    }

    fn reset_player(&self, player: QJSValue) {
        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()
            vid.onResize(Box::new(|_, _| { }));
            vid.onProcessTexture(Box::new(|_, _, _, _, _, _, _, _, _, _| -> bool {
                false
            }));
            vid.onProcessPixels(Box::new(|_, _, _, _, _, _| -> (u32, u32, u32, *mut u8) {
                (0, 0, 0, std::ptr::null_mut())
            }));
            vid.readyForProcessing(Box::new(|| -> bool { false }));
        }
    }
    fn init_player(&self, player: QJSValue) {
        use gyroflow_core::stabilization::RGBA8;

        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid1 = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()

            let bg_color = vid.getBackgroundColor().get_rgba_f();
            self.stabilizer.params.write().background = Vector4::new(bg_color.0 as f32, bg_color.1 as f32, bg_color.2 as f32, bg_color.3 as f32);
            self.stabilizer.stabilization.write().kernel_flags.set(KernelParamsFlags::DRAWING_ENABLED, true);
            let request_recompute = util::qt_queued_callback_mut(self, move |this, _: ()| {
                this.request_recompute();
            });
            let stab = self.stabilizer.clone();
            vid.onResize(Box::new(move |width, height| {
                let current_size = stab.params.read().size;
                if current_size.0 != width as usize || current_size.1 != height as usize {
                    stab.set_size(width as usize, height as usize);
                    request_recompute(());
                }
            }));

            use gyroflow_core::gpu::{ BufferDescription, Buffers, BufferSource };

            let stab = self.stabilizer.clone();
            vid.readyForProcessing(Box::new(move || -> bool {
                !stab.params.is_locked_exclusive() && !stab.stabilization.is_locked_exclusive()
            }));
            let stab = self.stabilizer.clone();
            let preview_pipeline = self.preview_pipeline.clone();
            let out_pixels = RefCell::new(Vec::new());
            let update_info = util::qt_queued_callback_mut(self, move |this, (fov, minimal_fov, focal_length, info): (f64, f64, Option<f64>, QString)| {
                this.current_fov = fov;
                this.current_minimal_fov = minimal_fov;
                this.current_focal_length = focal_length.unwrap_or_default();
                this.processing_info = info;
                this.processing_info_changed();
            });
            let update_info2 = update_info.clone();

            #[allow(unused_variables)]
            vid.onProcessTexture(Box::new(move |_frame, timestamp_ms, width, height, backend_id, ptr1, ptr2, ptr3, ptr4, ptr5| -> bool {
                if width < 4 || height < 4 || backend_id == 0 { return false; }

                if !stab.params.read().stab_enabled { return false; }

                let _time = std::time::Instant::now();

                if preview_pipeline.load(SeqCst) == 0 {
                    let mut buffers = Buffers{
                        input:  BufferDescription { size: (width as usize, height as usize, width as usize * 4), ..Default::default() },
                        output: BufferDescription { size: (width as usize, height as usize, width as usize * 4), ..Default::default() },
                    };
                    if let Some(ret) = qrhi_undistort::render(vid1.get_mdkplayer(), timestamp_ms, width, height, stab.clone(), &mut buffers) {
                        update_info2((ret.fov, ret.minimal_fov, ret.focal_length, QString::from(format!("Processing {}x{} using {} took {:.2}ms", width, height, ret.backend, _time.elapsed().as_micros() as f64 / 1000.0))));
                    } else {
                        update_info2((1.0, 1.0, None, QString::from("---")));
                    }
                    return true;
                }

                if preview_pipeline.load(SeqCst) > 1 { return false; }

                let size = (width as usize, height as usize, width as usize * 4);

                let mut buffers =
                    match backend_id {
                        1 => { // OpenGL, ptr1: texture, ptr2: opengl context
                            Some((Buffers {
                                input: BufferDescription {
                                    size,
                                    data: BufferSource::OpenGL {
                                        texture: ptr1 as u32,
                                        context: ptr2 as *mut std::ffi::c_void
                                    }, ..Default::default()
                                },
                                output: BufferDescription {
                                    size,
                                    data: BufferSource::OpenGL {
                                        texture: ptr1 as u32,
                                        context: ptr2 as *mut std::ffi::c_void
                                    }, ..Default::default()
                                },
                            },
                            "OpenGL"))
                        },
                        #[cfg(any(target_os = "macos", target_os = "ios"))]
                        2 => { // Metal, ptr1: texture, ptr2: device, ptr3: command queue
                            Some((Buffers {
                                input: BufferDescription {
                                    size,
                                    data: BufferSource::Metal { texture: ptr1 as *mut metal::MTLTexture, command_queue: ptr3 as *mut metal::MTLCommandQueue }, ..Default::default()
                                },
                                output: BufferDescription {
                                    size,
                                    texture_copy: true,
                                    data: BufferSource::Metal { texture: ptr1 as *mut metal::MTLTexture, command_queue: ptr3 as *mut metal::MTLCommandQueue }, ..Default::default()
                                },
                            },
                            "Metal"))
                        },
                        #[cfg(target_os = "windows")]
                        3 => { // D3D11, ptr1: texture, ptr2: device, ptr3: device context
                            Some((Buffers {
                                input: BufferDescription {
                                    size,
                                    texture_copy: true,
                                    data: BufferSource::DirectX11 {
                                        texture: ptr1 as *mut std::ffi::c_void,
                                        device:  ptr2 as *mut std::ffi::c_void,
                                        device_context: ptr3 as *mut std::ffi::c_void
                                    }, ..Default::default()
                                },
                                output: BufferDescription {
                                    size,
                                    texture_copy: true,
                                    data: BufferSource::DirectX11 {
                                        texture: ptr1 as *mut std::ffi::c_void,
                                        device:  ptr2 as *mut std::ffi::c_void,
                                        device_context: ptr3 as *mut std::ffi::c_void
                                    }, ..Default::default()
                                },
                            },
                            "DirectX11"))
                        },
                        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
                        4 => { // Vulkan, ptr1: VkImage, ptr2: VkDevice, ptr3: VkCommandBuffer, ptr4: VkPhysicalDevice, ptr5: VkInstance
                            Some((Buffers {
                                input: BufferDescription {
                                    size,
                                    texture_copy: false,
                                    data: BufferSource::Vulkan { texture: ptr1, device: ptr2, physical_device: ptr4, instance: ptr5 },
                                    ..Default::default()
                                },
                                output: BufferDescription {
                                    size,
                                    texture_copy: true,
                                    data: BufferSource::Vulkan { texture: ptr1, device: ptr2, physical_device: ptr4, instance: ptr5 },
                                    ..Default::default()
                                },
                            },
                            "Vulkan"))
                        }
                        _ => None
                    };

                if let Some((ref mut buffers, backend)) = buffers {
                    match stab.process_pixels::<RGBA8>((timestamp_ms * 1000.0) as i64, buffers) {
                        Ok(ret) =>  {
                            update_info2((ret.fov, ret.minimal_fov, ret.focal_length, QString::from(format!("Processing {}x{} using {backend}->{} took {:.2}ms", width, height, ret.backend, _time.elapsed().as_micros() as f64 / 1000.0))));
                            return true;
                        },
                        Err(e) => {
                            ::log::error!("Failed to process pixels: {e:?}");
                        }
                    }
                }

                update_info2((1.0, 1.0, None, QString::from("---")));
                false
            }));

            let stab = self.stabilizer.clone();
            let update_info2 = update_info.clone();
            vid.onProcessPixels(Box::new(move |_frame, timestamp_ms, width, height, stride, pixels: &mut [u8]| -> (u32, u32, u32, *mut u8) {
                let _time = std::time::Instant::now();

                // TODO: cache in atomics instead of locking the mutex every time
                let params = stab.params.read();
                if !params.stab_enabled { return (0, 0, 0, std::ptr::null_mut()); }
                let (ow, oh) = params.output_size;
                let os = ow * 4; // Assume RGBA8 - 4 bytes per pixel
                drop(params);

                let mut out_pixels = out_pixels.borrow_mut();
                out_pixels.resize_with(os*oh, u8::default);

                let ret = stab.process_pixels::<RGBA8>((timestamp_ms * 1000.0) as i64, &mut Buffers {
                    input: BufferDescription {
                        size: (width as usize, height as usize, stride as usize),
                        data: BufferSource::Cpu { buffer: pixels },
                        ..Default::default()
                    },
                    output: BufferDescription {
                        size: (ow, oh, os),
                        data: BufferSource::Cpu { buffer: &mut out_pixels },
                        ..Default::default()
                    },
                });
                match ret {
                    Ok(bk) => {
                        update_info2((bk.fov, bk.minimal_fov, bk.focal_length, QString::from(format!("Processing {}x{} using {} took {:.2}ms", width, height, bk.backend, _time.elapsed().as_micros() as f64 / 1000.0))));
                        (ow as u32, oh as u32, os as u32, out_pixels.as_mut_ptr())
                    },
                    Err(_) => {
                        update_info2((1.0, 1.0, None, QString::from("---")));
                        (0, 0, 0, std::ptr::null_mut())
                    }
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
            self.stabilizer.set_background_color(Vector4::new(bg.0 as f32, bg.1 as f32, bg.2 as f32, bg.3 as f32));
            self.request_recompute();
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
    wrap_simple_method!(set_use_gravity_vectors, v: bool; recompute; chart_data_changed);
    wrap_simple_method!(set_horizon_lock_integration_method, v: i32; recompute; chart_data_changed);
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
        if self.stabilizer.params.read().duration_ms <= 0.0 { return; }
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

    fn cancel_current_operation(&mut self) {
        self.cancel_flag.store(true, SeqCst);
    }

    fn export_gyroflow_file(&self, url: QUrl, typ: QString, additional_data: QJsonObject) {
        let url = util::qurl_to_encoded(url);
        let typ_str = typ.clone();
        let typ = core::GyroflowProjectType::from_str(&typ.to_string()).unwrap();

        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            util::set_setting("lastProject", &filesystem::url_to_path(&url));
        }
        let finished = util::qt_queued_callback(self, move |this, (res, arg): (&str, String)| {
            match res {
                "ok" => this.message(QString::from("Gyroflow file exported to %1."), QString::from(format!("<b>{}</b>", filesystem::display_url(&arg))), QString::default(), QString::from("gyroflow-exported")),
                "location" => this.request_location(QString::from(arg), typ_str.clone()),
                "err" => this.error(QString::from("An error occured: %1"), QString::from(arg), QString::default()),
                _ => { }
            }
            this.request_recompute();
        });

        let stab = self.stabilizer.clone();
        core::run_threaded(move || {
            match stab.export_gyroflow_file(&url, typ, &additional_data.to_json().to_string()) {
                Ok(_) => finished(("ok", url.to_string())),
                Err(core::GyroflowCoreError::IOError(ref e)) if e.kind() == std::io::ErrorKind::PermissionDenied => finished(("location", url.to_string())),
                Err(e) => finished(("err", e.to_string()))
            }
        });
    }

    fn export_gyroflow_data(&self, typ: QString, additional_data: QJsonObject) -> QString {
        let typ = core::GyroflowProjectType::from_str(&typ.to_string()).unwrap();
        QString::from(self.stabilizer.export_gyroflow_data(typ, &additional_data.to_json().to_string(), None).unwrap_or_default())
    }

    fn get_urls_from_gyroflow_file(&mut self, url: QUrl) -> QStringList {
        let url = util::qurl_to_encoded(url);
        let mut ret = vec![QString::default(); 2];
        if let Ok(data) = filesystem::read(&url) {
            if let Ok(serde_json::Value::Object(obj)) = serde_json::from_slice(&data) {
                let mut org_video_url = obj.get("videofile").and_then(|x| x.as_str()).unwrap_or("").to_string();
                if !org_video_url.is_empty() && !org_video_url.contains("://") {
                    org_video_url = filesystem::path_to_url(&org_video_url);
                }
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                if let Some(v) = obj.get("videofile_bookmark").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) {
                    let (resolved, _is_stale) = filesystem::apple::resolve_bookmark(v, Some(&url));
                    if !resolved.is_empty() { org_video_url = resolved; }
                }

                if let Some(seq_start) = obj.get("image_sequence_start").and_then(|x| x.as_i64()) {
                    self.image_sequence_start = seq_start as i32;
                }
                if let Some(seq_fps) = obj.get("image_sequence_fps").and_then(|x| x.as_f64()) {
                    self.image_sequence_fps = seq_fps;
                }
                if !org_video_url.is_empty() {
                    let video_path = StabilizationManager::get_new_videofile_url(&org_video_url, Some(&url), self.image_sequence_start as u32);
                    ret[0] = QString::from(video_path);
                }

                if let Some(serde_json::Value::Object(gyro)) = obj.get("gyro_source") {
                    let mut gyro_url = gyro.get("filepath").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    if !gyro_url.is_empty() && !gyro_url.contains("://") {
                        gyro_url = filesystem::path_to_url(&gyro_url);
                    }
                    #[cfg(any(target_os = "macos", target_os = "ios"))]
                    if let Some(v) = obj.get("filepath_bookmark").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) {
                        let (resolved, _is_stale) = filesystem::apple::resolve_bookmark(v, Some(&url));
                        if !resolved.is_empty() { gyro_url = resolved; }
                    }

                    if !gyro_url.is_empty() {
                        let gyro_url = StabilizationManager::get_new_videofile_url(&gyro_url, Some(&url), self.image_sequence_start as u32);
                        ret[1] = QString::from(gyro_url);
                    }
                }
            } else {
                ::log::error!("Failed to parse json: {}", unsafe { std::str::from_utf8_unchecked(&data) });
            }
        }
        QStringList::from_iter(ret.into_iter())
    }

    fn import_gyroflow_file(&mut self, url: QUrl) {
        let url = util::qurl_to_encoded(url);
        let progress = util::qt_queued_callback_mut(self, move |this, progress: f64| {
            this.loading_gyro_in_progress = progress < 1.0;
            this.loading_gyro_progress(progress);
            this.loading_gyro_in_progress_changed();
        });
        let finished = util::qt_queued_callback_mut(self, move |this, obj: Result<serde_json::Value, gyroflow_core::GyroflowCoreError>| {
            this.loading_gyro_in_progress = false;
            this.loading_gyro_progress(1.0);
            this.loading_gyro_in_progress_changed();

            let obj = this.import_gyroflow_internal(obj);
            this.gyroflow_file_loaded(obj);
            this.project_file_url_changed();
        });

        let stab = self.stabilizer.clone();
        let cancel_flag = self.cancel_flag.clone();
        cancel_flag.store(true, SeqCst);
        core::run_threaded(move || {
            if Arc::strong_count(&cancel_flag) > 2 {
                // Wait for other tasks to finish
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            cancel_flag.store(false, SeqCst);
            finished(stab.import_gyroflow_file(&url, false, progress, cancel_flag));
        });
    }
    fn import_gyroflow_data(&mut self, data: QString) {
        let progress = util::qt_queued_callback_mut(self, move |this, progress: f64| {
            this.loading_gyro_in_progress = progress < 1.0;
            this.loading_gyro_progress(progress);
            this.loading_gyro_in_progress_changed();
        });
        let finished = util::qt_queued_callback_mut(self, move |this, obj: Result<serde_json::Value, gyroflow_core::GyroflowCoreError>| {
            this.loading_gyro_in_progress = false;
            this.loading_gyro_progress(1.0);
            this.loading_gyro_in_progress_changed();

            let obj = this.import_gyroflow_internal(obj);
            this.gyroflow_file_loaded(obj);
        });

        let stab = self.stabilizer.clone();
        let cancel_flag = self.cancel_flag.clone();
        cancel_flag.store(true, SeqCst);
        core::run_threaded(move || {
            if Arc::strong_count(&cancel_flag) > 2 {
                // Wait for other tasks to finish
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            cancel_flag.store(false, SeqCst);
            let mut is_preset = false;
            finished(stab.import_gyroflow_data(data.to_string().as_bytes(), false, None, progress, cancel_flag, &mut is_preset));
        });
    }
    fn import_gyroflow_internal(&mut self, result: Result<serde_json::Value, gyroflow_core::GyroflowCoreError>) -> QJsonObject {
        match result {
            Ok(thin_obj) => {
                if thin_obj.as_object().unwrap().contains_key("calibration_data") {
                    self.lens_loaded = true;
                    self.lens_changed();
                    let lens_json = self.stabilizer.lens.read().get_json().unwrap_or_default();
                    self.lens_profile_loaded(QString::from(lens_json), QString::default(), QString::default());
                }
                self.update_offset_model();
                self.request_recompute();
                self.chart_data_changed();
                self.keyframes_changed();
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
        }
    }

    wrap_simple_method!(override_video_fps,         v: f64, r: bool; recompute; update_offset_model);
    wrap_simple_method!(set_video_rotation,         v: f64; recompute; zooming_data_changed);
    wrap_simple_method!(set_stab_enabled,           v: bool);
    wrap_simple_method!(set_show_detected_features, v: bool);
    wrap_simple_method!(set_show_optical_flow,      v: bool);
    wrap_simple_method!(set_digital_lens_name,      v: String; recompute);
    wrap_simple_method!(set_digital_lens_param,     i: usize, v: f64; recompute);
    wrap_simple_method!(set_fov_overview,       v: bool; recompute);
    wrap_simple_method!(set_show_safe_area,     v: bool; recompute);
    wrap_simple_method!(set_fov,                v: f64; recompute; chart_data_changed);
    wrap_simple_method!(set_frame_readout_time, v: f64; recompute);
    wrap_simple_method!(set_adaptive_zoom,      v: f64; recompute; zooming_data_changed);
    wrap_simple_method!(set_zooming_center_x,   v: f64; recompute; zooming_data_changed);
    wrap_simple_method!(set_zooming_center_y,   v: f64; recompute; zooming_data_changed);
    wrap_simple_method!(set_zooming_method,     v: i32; recompute; zooming_data_changed);
    wrap_simple_method!(set_trim_start,         v: f64; recompute; chart_data_changed);
    wrap_simple_method!(set_trim_end,           v: f64; recompute; chart_data_changed);
    wrap_simple_method!(set_of_method,          v: u32; recompute; chart_data_changed);

    wrap_simple_method!(set_lens_correction_amount,    v: f64; recompute; zooming_data_changed);
    wrap_simple_method!(set_input_horizontal_stretch,  v: f64; recompute);
    wrap_simple_method!(set_lens_is_asymmetrical,      v: bool; recompute);
    wrap_simple_method!(set_input_vertical_stretch,    v: f64; recompute);
    wrap_simple_method!(set_background_mode,           v: i32; recompute);
    wrap_simple_method!(set_background_margin,         v: f64; recompute);
    wrap_simple_method!(set_background_margin_feather, v: f64; recompute);
    wrap_simple_method!(set_video_speed,               v: f64, s: bool, z: bool; recompute; zooming_data_changed);

    wrap_simple_method!(set_offset, timestamp_us: i64, offset_ms: f64; recompute; update_offset_model);
    wrap_simple_method!(clear_offsets,; recompute; update_offset_model);
    wrap_simple_method!(remove_offset, timestamp_us: i64; recompute; update_offset_model);

    wrap_simple_method!(set_imu_lpf, v: f64; recompute; chart_data_changed);
    wrap_simple_method!(set_imu_rotation, pitch_deg: f64, roll_deg: f64, yaw_deg: f64; recompute; chart_data_changed);
    wrap_simple_method!(set_acc_rotation, pitch_deg: f64, roll_deg: f64, yaw_deg: f64; recompute; chart_data_changed);
    wrap_simple_method!(set_imu_orientation, v: String; recompute; chart_data_changed);
    wrap_simple_method!(set_sync_lpf, v: f64; recompute; chart_data_changed);
    wrap_simple_method!(set_imu_bias, bx: f64, by: f64, bz: f64; recompute; chart_data_changed);
    wrap_simple_method!(recompute_gyro,; recompute; chart_data_changed);
    wrap_simple_method!(set_device, v: i32);

    fn get_org_duration_ms   (&self) -> f64 { self.stabilizer.params.read().duration_ms }
    fn get_scaled_duration_ms(&self) -> f64 { self.stabilizer.params.read().get_scaled_duration_ms() }
    fn get_scaled_fps        (&self) -> f64 { self.stabilizer.params.read().get_scaled_fps() }
    fn get_scaling_ratio     (&self) -> f64 { self.stabilizer.get_scaling_ratio() }
    fn get_min_fov           (&self) -> f64 { self.stabilizer.get_min_fov() }

    fn offset_at_video_timestamp(&self, timestamp_us: i64) -> f64 {
        self.stabilizer.offset_at_video_timestamp(timestamp_us)
    }
    fn quats_at_timestamp(&self, timestamp_us: i64) -> QVariantList {
        let gyro = self.stabilizer.gyro.read();
        let ts = timestamp_us as f64 / 1000.0 - gyro.offset_at_video_timestamp(timestamp_us as f64 / 1000.0);
        let sq = gyro.smoothed_quat_at_timestamp(ts);
        let q = gyro.org_quat_at_timestamp(ts);
        QVariantList::from_iter(&[q.w, q.i, q.j, q.k, sq.w, sq.i, sq.j, sq.k]) // scalar first
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
                    if let Some(v) = v.as_array() {
                        for itm in v {
                            if let Some(obj) = itm.as_object() {
                                let name = obj.get("name").and_then(|x| x.as_str());
                                let body = obj.get("body").and_then(|x| x.as_str());
                                let is_prerelease = obj.get("prerelease").and_then(|x| x.as_bool()).unwrap_or_default();
                                if is_prerelease { continue; }

                                if let Some(name) = name {
                                    ::log::info!("Latest version: {}, current version: {}", name, util::get_version());

                                    if let Ok(latest_version) = semver::Version::parse(name.trim_start_matches('v')) {
                                        if let Ok(this_version) = semver::Version::parse(env!("CARGO_PKG_VERSION")) {
                                            if latest_version > this_version {
                                                update((name.to_owned(), body.unwrap_or_default().to_owned()));
                                            }
                                        }
                                    }
                                    break;
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
            self.calib_progress(0.0, 0.0, 0, 0, 0, 0.0);

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
                let saved: BTreeMap<i32, core::calibration::Detected> = {
                    let lock = cal.image_points.read();
                    cal.forced_frames.iter().filter_map(|f| Some((*f, lock.get(f)?.clone()))).collect()
                };
                *cal.image_points.write() = saved;
                cal.max_images = max_points;
                cal.iterations = iterations;
                cal.max_sharpness = max_sharpness;
            }

            let progress = util::qt_queued_callback_mut(self, |this, (ready, total, good, rms, sharpness): (usize, usize, usize, f64, f64)| {
                this.calib_in_progress = ready < total;
                this.calib_in_progress_changed();
                this.calib_progress(ready as f64 / total as f64, rms, ready, total, good, sharpness);
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

            let processing_resolution = self.processing_resolution;

            let input_file = stab.input_file.read().clone();
            core::run_threaded(move || {

                let mut decoder_options = ffmpeg_next::Dictionary::new();
                if input_file.image_sequence_fps > 0.0 {
                    let fps = rendering::fps_to_rational(input_file.image_sequence_fps);
                    decoder_options.set("framerate", &format!("{}/{}", fps.numerator(), fps.denominator()));
                }
                if input_file.image_sequence_start > 0 {
                    decoder_options.set("start_number", &format!("{}", input_file.image_sequence_start));
                }
                if processing_resolution > 0 {
                    decoder_options.set("scale", &format!("{}x{}", (processing_resolution * 16) / 9, processing_resolution));
                }

                ::log::debug!("Decoder options: {:?}", decoder_options);
                let gpu_decoding = *rendering::GPU_DECODING.read();
                let fs_base = gyroflow_core::filesystem::get_engine_base();
                match VideoProcessor::from_file(&fs_base, &input_file.url, gpu_decoding, 0, Some(decoder_options)) {
                    Ok(mut proc) => {
                        let progress = progress.clone();
                        let err2 = err.clone();
                        let cal = cal.clone();
                        let total_read = total_read.clone();
                        let processed = processed.clone();
                        let cancel_flag2 = cancel_flag.clone();
                        let dims = proc.get_org_dimensions();

                        proc.on_frame(move |timestamp_us, input_frame, _output_frame, converter, _rate_control| {
                            let frame = core::frame_at_timestamp(timestamp_us as f64 / 1000.0, fps);

                            if is_forced && total_read.load(SeqCst) > 0 {
                                return Ok(());
                            }

                            if (frame % every_nth_frame as i32) == 0 {
                                let mut width = (input_frame.width() as f64 * input_horizontal_stretch).round() as u32;
                                let mut height = (input_frame.height() as f64 * input_vertical_stretch).round() as u32;
                                let mut org_size = (width, height);
                                let mut pt_scale = 1.0;
                                if processing_resolution > 0 && height > processing_resolution as u32 {
                                    pt_scale = height as f32 / processing_resolution as f32;
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

                                        if let Some(dims) = &dims {
                                            let (w, h) = (dims.0.load(SeqCst), dims.1.load(SeqCst));
                                            if w > 0 && h > 0 {
                                                pt_scale = h as f32 / height as f32;
                                                org_size = (w as u32, h as u32);
                                            }
                                        }
                                        cal.feed_frame(timestamp_us, frame, (width, height), org_size, stride, pt_scale, pixels, cancel_flag2.clone(), total, processed.clone(), progress.clone());
                                    },
                                    Err(e) => {
                                        err2(("An error occured: %1".to_string(), e.to_string()))
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
                    if cal.rms < 100.0 {
                        stab.lens.write().set_from_calibrator(cal);
                    }
                    ::log::debug!("rms: {}, used_frames: {:?}, camera_matrix: {}, coefficients: {}", cal.rms, cal.used_points.keys(), cal.k, cal.d);
                }

                let good = cal.image_points.read().len();
                progress((total, total, good, cal.rms, *cal.sum_sharpness.read() / good.max(1) as f64));

                stab.params.write().is_calibrator = true;
            });
        }
    }

    fn update_calib_model(&mut self) {
        #[cfg(feature = "opencv")]
        {
            let cal = self.stabilizer.lens_calibrator.clone();

            let used_points = cal.read().as_ref().map(|x| x.used_points.clone()).unwrap_or_default();

            self.calib_model = RefCell::new(used_points.values().map(|v| CalibrationItem {
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
                self.calib_progress(1.0, rms, 1, 1, 1, 0.0);
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
            let name = profile.get_name()
                .replace([':', '|', '*', ':'], "_")
                .replace(['<', '"', '>', '/', '\\'], "");
            return QString::from(format!("{}.json", name));
        }
        QString::default()
    }

    fn export_lens_profile(&mut self, url: QUrl, info: QJsonObject, upload: bool) {
        let url = util::qurl_to_encoded(url);
        let info_json = info.to_json().to_string();

        match core::lens_profile::LensProfile::from_json(&info_json) {
            Ok(mut profile) => {
                #[cfg(feature = "opencv")]
                if let Some(ref cal) = *self.stabilizer.lens_calibrator.read() {
                    profile.set_from_calibrator(cal);
                }

                match profile.save_to_file(&url) {
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

    fn load_profiles(&self, reload_from_disk: bool) {
        let loaded = util::qt_queued_callback_mut(self, |this, all_names: QVariantList| {
            this.all_profiles_loaded(all_names)
        });
        let db = self.stabilizer.lens_profile_db.clone();
        core::run_threaded(move || {
            if reload_from_disk {
                let mut new_db = core::lens_profile_database::LensProfileDatabase::default();
                new_db.load_all();
                // Important! Disable `fetch_profiles_from_github` before running these functions
                // new_db.list_all_metadata();
                // new_db.process_adjusted_metadata();

                db.write().set_from_db(new_db);
            }

            let all_names = db.read().get_all_info().into_iter().map(|(name, file, crc, official, rating, aspect_ratio)| {
                let mut list = QVariantList::from_iter([
                    QString::from(name),
                    QString::from(file),
                    QString::from(crc)
                ].into_iter());
                list.push(official.into());
                list.push(rating.into());
                list.push(aspect_ratio.into());
                list
            }).collect();

            loaded(all_names);
        });
    }

    #[allow(unreachable_code)]
    fn fetch_profiles_from_github(&self) {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            return;
        }

        use crate::core::lens_profile_database::LensProfileDatabase;

        if LensProfileDatabase::get_path().join("noupdate").exists() {
            ::log::info!("Skipping lens profile updates.");
            return;
        }

        let update = util::qt_queued_callback_mut(self, |this, _| {
            this.lens_profiles_updated(true);
        });

        core::run_threaded(move || {
            if let Ok(Ok(body)) = ureq::get("https://api.github.com/repos/gyroflow/lens_profiles/git/trees/main?recursive=1").call().map(|x| x.into_string()) {
                (|| -> Option<()> {
                    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
                    for obj in v.get("tree")?.as_array()? {
                        let obj = obj.as_object()?;
                        let path = obj.get("path")?.as_str()?;
                        if path.ends_with(".json") || path.ends_with(".gyroflow") {
                            let local_path = LensProfileDatabase::get_path().join(path);
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

    fn rate_profile(&self, name: QString, json: QString, checksum: QString, is_good: bool) {
        core::run_threaded(move || {
            let mut url = url::Url::parse(&format!("https://api.gyroflow.xyz/rate?good={}&checksum={}", is_good, checksum)).unwrap();
            url.query_pairs_mut().append_pair("filename", &name.to_string());

            if let Ok(Ok(body)) = ureq::request_url("POST", &url).set("Content-Type", "application/json; charset=utf-8").send_string(&json.to_string()).map(|x| x.into_string()) {
                ::log::debug!("Lens profile rated: {}", body.as_str());
            }
        });
    }
    fn request_profile_ratings(&self) {
        let update = util::qt_queued_callback_mut(self, |this, _| {
            this.lens_profiles_updated(false);
        });
        let db = self.stabilizer.lens_profile_db.clone();
        core::run_threaded(move || {
            if let Ok(Ok(body)) = ureq::get("https://api.gyroflow.xyz/rate?get_ratings=1").call().map(|x| x.into_string()) {
                db.write().set_profile_ratings(body.as_str());
                update(());
            }
        });
    }

    fn list_gpu_devices(&self) {
        let finished = util::qt_queued_callback(self, |this, list: Vec<String>| {
            this.gpu_list_loaded(util::serde_json_to_qt_array(&serde_json::json!(list)))
        });
        self.stabilizer.list_gpu_devices(finished);
    }
    fn set_rendering_gpu_type_from_name(&self, name: String) {
        rendering::set_gpu_type_from_name(&name);
    }

    fn export_preset(&self, url: QUrl, content: QJsonObject) {
        let contents = content.to_json_pretty();
        if let Err(e) = filesystem::write(&util::qurl_to_encoded(url), contents.to_slice()) {
            self.error(QString::from("An error occured: %1"), QString::from(e.to_string()), QString::default());
        }
    }

    fn set_keyframe(&self, typ: String, timestamp_us: i64, value: f64) {
        if let Ok(kf) = KeyframeType::from_str(&typ) {
            self.stabilizer.set_keyframe(&kf, timestamp_us, value);
            self.keyframes_changed();
            self.request_recompute();
            self.chart_data_changed();
        }
    }
    fn set_keyframe_easing(&self, typ: String, timestamp_us: i64, easing: String) {
        if let Ok(kf) = KeyframeType::from_str(&typ) {
            if let Ok(e) = Easing::from_str(&easing) {
                self.stabilizer.set_keyframe_easing(&kf, timestamp_us, e);
                self.keyframes_changed();
                self.request_recompute();
                self.chart_data_changed();
            }
        }
    }
    fn keyframe_easing(&self, typ: String, timestamp_us: i64) -> String {
        if let Ok(kf) = KeyframeType::from_str(&typ) {
            if let Some(e) = self.stabilizer.keyframe_easing(&kf, timestamp_us) {
                return e.to_string();
            }
        }
        String::new()
    }
    fn remove_keyframe(&self, typ: String, timestamp_us: i64) {
        if let Ok(kf) = KeyframeType::from_str(&typ) {
            self.stabilizer.remove_keyframe(&kf, timestamp_us);
            self.keyframes_changed();
            self.request_recompute();
            self.chart_data_changed();
        }
    }
    fn clear_keyframes_type(&self, typ: String) {
        if let Ok(kf) = KeyframeType::from_str(&typ) {
            self.stabilizer.clear_keyframes_type(&kf);
            self.keyframes_changed();
            self.request_recompute();
            self.chart_data_changed();
        }
    }
    fn keyframe_value_at_video_timestamp(&self, typ: String, timestamp_ms: f64) -> QJSValue {
        if let Ok(typ) = KeyframeType::from_str(&typ) {
            if let Some(v) = self.stabilizer.keyframe_value_at_video_timestamp(&typ, timestamp_ms) {
                return QJSValue::from(v);
            }
        }
        QJSValue::default()
    }
    fn is_keyframed(&self, typ: String) -> bool {
        if let Ok(typ) = KeyframeType::from_str(&typ) {
            return self.stabilizer.is_keyframed(&typ);
        }
        false
    }

    fn update_keyframe_values(&self, mut timestamp_ms: f64) {
        let keyframes = self.stabilizer.keyframes.read();
        timestamp_ms /= keyframes.timestamp_scale.unwrap_or(1.0);
        for kf in keyframes.get_all_keys() {
            if let Some(v) = keyframes.value_at_video_timestamp(kf, timestamp_ms) {
                self.keyframe_value_updated(kf.to_string(), v);
            }
        }
    }

    fn has_gravity_vectors(&self) -> bool {
        self.stabilizer.gyro.read().file_metadata.gravity_vectors.as_ref().map(|v| !v.is_empty()).unwrap_or_default()
    }

    fn check_external_sdk(&self, filename: QString) -> bool {
        crate::external_sdk::requires_install(&filename.to_string())
    }
    fn install_external_sdk(&self, url: QString) {
        let filename = if url.to_string() == "ffmpeg_gpl" { url.to_string() } else { filesystem::get_filename(&url.to_string()) };
        let progress = util::qt_queued_callback_mut(self, move |this, (percent, sdk_name, error_string): (f64, &'static str, String)| {
            this.external_sdk_progress(percent, QString::from(sdk_name), QString::from(error_string), QString::from(url.clone()));
        });
        crate::external_sdk::install(&filename, progress);
    }

    fn mp4_merge(&self, file_list: QStringList, output_folder: QUrl, output_filename: QString) {
        let output_folder = util::qurl_to_encoded(output_folder);
        let output_filename = output_filename.to_string();
        let output_url = filesystem::get_file_url(&output_folder, &output_filename, true);

        let mut file_list: Vec<String> = file_list.into_iter().map(QString::to_string).collect();
        file_list.sort_by(|a, b| human_sort::compare(a, b));

        ::log::debug!("Merging files: {:?}", &file_list);
        if file_list.len() < 2 {
            self.mp4_merge_progress(1.0, QString::from("Not enough files!"), QString::default());
            return;
        }
        if output_url.is_empty() {
            self.mp4_merge_progress(1.0, QString::from("Empty output path!"), QString::default());
            return;
        }
        let first_url = file_list.first().unwrap().clone();
        let out = output_url.clone();
        let progress = util::qt_queued_callback_mut(self, move |this, (percent, error_string): (f64, String)| {
            this.mp4_merge_progress(percent, QString::from(error_string), QString::from(out.as_str()));
        });
        core::run_threaded(move || {
            let base = filesystem::get_engine_base();
            let mut opened = Vec::with_capacity(file_list.len());
            for x in &file_list {
                match filesystem::open_file(&base, &x, false) {
                    Ok(x) => { opened.push(x); },
                    Err(e) => { progress((1.0, format!("Failed to open file: {x}: {e:?}"))); return; }
                }
            }
            let mut file_references: Vec<(&mut std::fs::File, usize)> = opened.iter_mut().map(|x| { let s = x.size; (x.get_file(), s) }).collect();
            let mut opened_output = match filesystem::open_file(&base, &output_url, true) {
                Ok(x) => { x },
                Err(e) => { progress((1.0, format!("Failed to create file: {output_url}: {e:?}"))); return; }
            };
            let res = mp4_merge::join_file_streams(&mut file_references, opened_output.get_file(), |p| progress((p.min(0.9999), String::default())));
            match res {
                Ok(_) => {
                    if let Err(e) = Self::merge_gcsv(&file_list, &output_folder, &output_filename) {
                        ::log::error!("Failed to merge .gcsv files: {:?}", e);
                    }

                    crate::util::update_file_times(&output_url, &first_url);

                    progress((1.0, String::default()))
                },
                Err(e) => progress((1.0, e.to_string()))
            }
        });
    }
    fn merge_gcsv(file_list: &[String], output_folder: &str, output_filename: &str) -> Result<(), gyroflow_core::GyroflowCoreError> {
        let base = filesystem::get_engine_base();

        use std::io::{ BufRead, Write, Seek, SeekFrom };
        let mut last_diff = 0.0;
        let mut last_timestamp = 0.0;
        let mut add_timestamp = 0.0;
        let mut output_gcsv = None;
        let mut first_file = true;
        let mut sync_points = Vec::new();
        let mut time_scale = 0.001; // default to millisecond
        let mut headers_end_position = None;
        for x in file_list {
            let filename = filesystem::get_filename(x);
            let folder = filesystem::get_folder(x);
            let gcsv_name = filesystem::filename_with_extension(&filename, "gcsv");
            let gcsv_url = filesystem::get_file_url(&folder, &gcsv_name, false);
            if filesystem::exists_in_folder(&folder, &gcsv_name) {
                let mut is_data = false;
                if let Ok(mut file) = filesystem::open_file(&base, &gcsv_url, false) {
                    if output_gcsv.is_none() {
                        let out_url = filesystem::get_file_url(&output_folder, &filesystem::filename_with_extension(output_filename, "gcsv"), true);
                        output_gcsv = Some(filesystem::open_file(&base, &out_url, true)?);
                    }
                    for (i, line) in std::io::BufReader::new(file.get_file()).lines().enumerate() {
                        let mut line = line?;
                        if i == 0 && !line.contains("GYROFLOW IMU LOG") && !line.contains("CAMERA IMU LOG") {
                            return Ok(()); // not a .gcsv file
                        }
                        if !is_data {
                            if line.starts_with("tscale,") {
                                if let Ok(ts) = line.strip_prefix("tscale,").unwrap().parse::<f64>() {
                                    time_scale = ts;
                                }
                            }
                            if line.starts_with("t,") || line.starts_with("time,") {
                                is_data = true;
                                if !first_file {
                                    sync_points.push((add_timestamp * time_scale - 0.5) * 1000.0);
                                    sync_points.push((add_timestamp * time_scale + 0.5) * 1000.0);
                                    sync_points.push((add_timestamp * time_scale + 1.0) * 1000.0);
                                    sync_points.push((add_timestamp * time_scale + 2.0) * 1000.0);
                                    sync_points.push((add_timestamp * time_scale + 2.5) * 1000.0);
                                    continue;
                                } else {
                                    headers_end_position = Some(output_gcsv.as_mut().unwrap().get_file().stream_position()?);
                                    writeln!(output_gcsv.as_mut().unwrap().get_file(), "additional_sync_points,{}", " ".repeat(1024))?; // 1kb of placeholder spaces
                                }
                            }
                        } else if line.contains(',') {
                            if let Ok(timestamp) = line.split(',').next().unwrap().parse::<f64>() {
                                last_diff = timestamp - last_timestamp;
                                last_timestamp = timestamp;
                                if timestamp >= add_timestamp {
                                    add_timestamp = 0.0;
                                }
                                let new_timestamp = timestamp + add_timestamp;
                                line = [new_timestamp.to_string()].into_iter().chain(line.split(',').skip(1).map(str::to_string)).join(",");
                            }
                        }
                        if first_file || is_data {
                            writeln!(output_gcsv.as_mut().unwrap().get_file(), "{}", line)?;
                        }
                    }
                }
                add_timestamp += last_timestamp + last_diff;
                last_timestamp = 0.0;
            }
            first_file = false;
        }
        if !sync_points.is_empty() && output_gcsv.is_some() && headers_end_position.is_some() {
            let output_gcsv = &mut output_gcsv.as_mut().unwrap().get_file();
            output_gcsv.seek(SeekFrom::Start(headers_end_position.unwrap()))?;
            write!(output_gcsv, "additional_sync_points,{}", sync_points.into_iter().map(|x| format!("{:.3}", x)).join(";"))?;
        }
        Ok(())
    }

    // ---------- REDline conversion ----------
    fn find_redline(&self) -> QString {
        QString::from(crate::external_sdk::r3d::REDSdk::find_redline())
    }
    // ---------- REDline conversion ----------

    fn play_sound(&self, typ: String) {
        core::run_threaded(move || {
            use std::io::{ Cursor, Error, ErrorKind };
            let _ = (|| -> Result<(), Box<dyn std::error::Error>> {
                let source = match typ.as_ref() {
                    "success" => include_bytes!("../resources/success.ogg") as &[u8],
                    "error"   => include_bytes!("../resources/error.ogg") as &[u8],
                    _ => { return Err(Error::new(ErrorKind::Other, "").into()) }
                };
                {
                    let (_stream, handle) = rodio::OutputStream::try_default()?;
                    let sink = rodio::Sink::try_new(&handle)?;
                    sink.append(rodio::Decoder::new(Cursor::new(source))?);
                    sink.sleep_until_end();
                }
                Ok(())
            })();
        });
    }

    // Utilities
    fn get_username(&self) -> QString { let realname = whoami::realname(); QString::from(if realname.is_empty() { whoami::username() } else { realname }) }
    fn image_to_b64(&self, img: QImage) -> QString { util::image_to_b64(img) }
    fn clear_settings(&self) { util::clear_settings() }
    fn copy_to_clipboard(&self, text: QString) { util::copy_to_clipboard(text) }
}

#[derive(Default, QObject)]
pub struct Filesystem {
    base: qt_base_class!(trait QObject),

    exists_in_folder:         qt_method!(fn(&self, folder: QUrl, filename: QString) -> bool),
    can_create_file:          qt_method!(fn(&self, folder: QUrl, filename: QString) -> bool),
    exists:                   qt_method!(fn(&self, url: QUrl) -> bool),
    get_filename:             qt_method!(fn(&self, url: QUrl) -> QString),
    get_folder:               qt_method!(fn(&self, url: QUrl) -> QString),
    filename_with_extension:  qt_method!(fn(&self, filename: QString, ext: QString) -> QString),
    filename_with_suffix:     qt_method!(fn(&self, filename: QString, suffix: QString) -> QString),
    open_file_externally:     qt_method!(fn(&self, url: QUrl)),
    path_to_url:              qt_method!(fn(&self, path: QString) -> QUrl),
    get_file_url:             qt_method!(fn(&self, folder: QUrl, filename: String, can_create: bool) -> QUrl),
    url_to_path:              qt_method!(fn(&self, url: QUrl) -> QString),
    display_url:              qt_method!(fn(&self, url: QUrl) -> QString),
    display_folder_filename:  qt_method!(fn(&self, folder: QUrl, filename: QString) -> QString),
    catch_url_open:           qt_method!(fn(&self, url: QUrl)),
    remove_file:              qt_method!(fn(&self, url: QUrl)),
    folder_access_granted:    qt_method!(fn(&self, url: QUrl)),
    save_allowed_folders:     qt_method!(fn(&self)),
    restore_allowed_folders:  qt_method!(fn(&self)),
    url_opened:               qt_signal!(url: QUrl),
}
impl Filesystem {
    fn exists_in_folder(&self, folder: QUrl, filename: QString) -> bool { filesystem::exists_in_folder(&util::qurl_to_encoded(folder), &filename.to_string()) }
    fn can_create_file(&self, folder: QUrl, filename: QString) -> bool { filesystem::can_create_file(&util::qurl_to_encoded(folder), &filename.to_string()) }
    fn exists(&self, url: QUrl) -> bool { filesystem::exists(&util::qurl_to_encoded(url)) }
    fn get_filename(&self, url: QUrl) -> QString { QString::from(filesystem::get_filename(&util::qurl_to_encoded(url))) }
    fn get_folder(&self, url: QUrl) -> QString { QString::from(filesystem::get_folder(&util::qurl_to_encoded(url))) }
    fn filename_with_extension(&self, filename: QString, ext: QString) -> QString { QString::from(filesystem::filename_with_extension(&filename.to_string(), &ext.to_string())) }
    fn filename_with_suffix(&self, filename: QString, suffix: QString) -> QString { QString::from(filesystem::filename_with_suffix(&filename.to_string(), &suffix.to_string())) }
    fn open_file_externally(&self, url: QUrl) { util::open_file_externally(url); }
    fn path_to_url(&self, path: QString) -> QUrl { QUrl::from(QString::from(filesystem::path_to_url(&path.to_string()))) }
    fn get_file_url(&self, folder: QUrl, filename: String, can_create: bool) -> QUrl { QUrl::from(QString::from(filesystem::get_file_url(&util::qurl_to_encoded(folder), &filename, can_create))) }
    fn url_to_path(&self, url: QUrl) -> QString { QString::from(filesystem::url_to_path(&util::qurl_to_encoded(url))) }
    fn display_url(&self, url: QUrl) -> QString { QString::from(filesystem::display_url(&util::qurl_to_encoded(url))) }
    fn display_folder_filename(&self, folder: QUrl, filename: QString) -> QString { QString::from(filesystem::display_folder_filename(&util::qurl_to_encoded(folder), &filename.to_string())) }
    fn catch_url_open(&self, url: QUrl) { util::dispatch_url_event(url.clone()); self.url_opened(url); }
    fn remove_file(&self, url: QUrl) { let _ = filesystem::remove_file(&util::qurl_to_encoded(url)); }
    fn folder_access_granted(&self, url: QUrl) { filesystem::folder_access_granted(&util::qurl_to_encoded(url)); }
    fn save_allowed_folders(&self) {
        let list = filesystem::get_allowed_folders();
        if !list.is_empty() {
            if let Ok(serialized) = serde_json::to_string((&list).into()) {
                util::set_setting("allowedUrls", &serialized);
            }
        }
    }
    fn restore_allowed_folders(&self) {
        let saved = util::get_setting("allowedUrls");
        if !saved.is_empty() {
            if let Ok(deserialized) = serde_json::from_str::<Vec<String>>(&saved) {
                filesystem::restore_allowed_folders(&deserialized);
            }
        }
    }
}
