use qmetaobject::*;
use nalgebra::{Vector2, Vector4};
use std::sync::Arc;
use std::time::Duration;
use parking_lot::RwLock;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::atomic::Ordering::SeqCst;

use qml_video_rs::video_item::MDKVideoItem;

use crate::core::StabilizationManager;
use crate::core::smoothing::*;
use crate::core::synchronization::PoseEstimator;
use crate::core::undistortion;
use crate::rendering;
use crate::rendering::FfmpegProcessor;
use crate::ui::components::TimelineGyroChart::TimelineGyroChart;

#[derive(Default, SimpleListItem)]
struct OffsetItem {
    pub timestamp_us: i64,
    pub offset_ms: f64,
}

// TODO: move this to core
lazy_static::lazy_static! {
    static ref THREAD_POOL: rayon::ThreadPool = rayon::ThreadPoolBuilder::new().build().unwrap();
    static ref CURRENT_COMPUTE_ID: AtomicU64 = AtomicU64::new(0);
}

#[derive(Default, QObject)]
pub struct Controller { 
    base: qt_base_class!(trait QObject),  
 
    init_player: qt_method!(fn(&mut self, player: QJSValue)),
    load_video: qt_method!(fn(&mut self, url: QUrl, player: QJSValue)),
    load_telemetry: qt_method!(fn(&mut self, url: QUrl, is_video: bool, player: QJSValue, chart: QJSValue)),
    load_lens_profile: qt_method!(fn(&mut self, path: QString)),

    sync_method: qt_property!(u32; WRITE set_sync_method),
    start_autosync: qt_method!(fn(&mut self, timestamps_fract: QString, initial_offset: f64, sync_search_size: f64, sync_duration_ms: f64, every_nth_frame: u32, player: QJSValue)), // QString is workaround for now
    update_chart: qt_method!(fn(&mut self, chart: QJSValue)),

    telemetry_loaded: qt_signal!(is_main_video: bool, filename: QString, camera: QString, imu_orientation: QString, contains_gyro: bool, contains_quats: bool, frame_readout_time: f64),
    lens_profile_loaded: qt_signal!(lens_info: QJsonObject),

    set_smoothing_method: qt_method!(fn(&mut self, index: usize) -> QJsonArray),
    set_smoothing_param: qt_method!(fn(&mut self, name: QString, val: f64)),
    set_preview_resolution: qt_method!(fn(&self, target_height: i32, player: QJSValue)),
    set_background_color: qt_method!(fn(&mut self, color: QString, player: QJSValue)),
    set_integration_method: qt_method!(fn(&mut self, index: usize)),

    set_offset: qt_method!(fn(&mut self, timestamp_us: i64, offset_ms: f64)),
    remove_offset: qt_method!(fn(&mut self, timestamp_us: i64)),
    offset_at_timestamp: qt_method!(fn(&self, timestamp_ms: f64) -> f64),
    offsets_model: qt_property!(RefCell<SimpleListModel<OffsetItem>>; NOTIFY offsets_updated),
    offsets_updated: qt_signal!(),

    update_lpf: qt_method!(fn(&mut self, lpf: f64)),
    update_sync_lpf: qt_method!(fn(&mut self, lpf: f64)),
    update_imu_rotation: qt_method!(fn(&mut self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64)),
    update_imu_orientation: qt_method!(fn(&mut self, orientation: String)),

    stab_enabled: qt_property!(bool; WRITE set_stab_enabled),
    show_detected_features: qt_property!(bool; WRITE set_show_detected_features),
    fov: qt_property!(f64; WRITE set_fov),
    frame_readout_time: qt_property!(f64; WRITE set_frame_readout_time),

    lens_loaded: qt_property!(bool; NOTIFY lens_changed),
    lens_changed: qt_signal!(),

    gyro_loaded: qt_property!(bool; NOTIFY gyro_changed),
    gyro_changed: qt_signal!(),

    stabilizer: Arc<RwLock<StabilizationManager>>, // TODO generic

    compute_progress: qt_signal!(id: u64, progress: f64),
    sync_progress: qt_signal!(progress: f64, status: QString),

    set_trim_start: qt_method!(fn(&mut self, trim_start: f64)),
    set_trim_end: qt_method!(fn(&mut self, trim_end: f64)),

    file_exists: qt_method!(fn(&mut self, path: QString) -> bool),

    chart_data_changed: qt_signal!(),

    render: qt_method!(fn(&self, codec: String, output_path: String, trim_start: f64, trim_end: f64, output_width: usize, output_height: usize, use_gpu: bool, audio: bool)),
    render_progress: qt_signal!(progress: f64, current_frame: usize, total_frames: usize),

    cancel_current_operation: qt_method!(fn(&mut self)),

    sync_in_progress: qt_property!(bool; NOTIFY sync_in_progress_changed),
    sync_in_progress_changed: qt_signal!(),

    export_gyroflow: qt_method!(fn(&self)),

    resolve_android_url: qt_method!(fn(&self, url: QString) -> QString),

    video_path: String,

    cancel_flag: Arc<AtomicBool>,
}
impl Controller {
    pub fn new() -> Self {
        Self {
            sync_method: 1,
            ..Default::default()
        }
    }
    fn update_offset_model(&mut self) {
        self.offsets_model = RefCell::new(self.stabilizer.read().gyro.offsets.iter().map(|(k, v)| OffsetItem {
            timestamp_us: *k, 
            offset_ms: *v
        }).collect());

        let qptr = QPointer::from(&*self);
        qmetaobject::queued_callback(move |_| {
            if let Some(this) = qptr.as_pinned() {
                let this = this.borrow();
                this.offsets_updated();
                this.chart_data_changed();
            }
        })(());
    }

    fn remove_offset(&mut self, timestamp_us: i64) {
        self.stabilizer.write().gyro.remove_offset(timestamp_us);

        self.update_offset_model();
        self.recompute_threaded();
    }
    fn set_offset(&mut self, timestamp_us: i64, offset_ms: f64) {
        self.stabilizer.write().gyro.set_offset(timestamp_us, offset_ms);

        self.update_offset_model();
        self.recompute_threaded();
    }
    
    fn offset_at_timestamp(&self, timestamp_ms: f64) -> f64 {
        self.stabilizer.read().gyro.offset_at_timestamp(timestamp_ms)
    }

    fn url_to_path(url: &str) -> &str {
        if url.starts_with("file://") {
            if cfg!(target_os = "windows") {
                url.strip_prefix("file:///").unwrap()
            } else {
                url.strip_prefix("file://").unwrap()
            }
        } else {
            url
        }
    }

    fn load_video(&mut self, url: QUrl, player: QJSValue) {
        self.stabilizer.write().pose_estimator.clear();
        self.chart_data_changed();
        self.video_path = Self::url_to_path(&QString::from(url.clone()).to_string()).to_string();

        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()
            vid.setUrl(url);
        }
    }

    fn start_autosync(&mut self, timestamps_fract: QString, initial_offset: f64, sync_search_size: f64, sync_duration_ms: f64, every_nth_frame: u32, player: QJSValue) {
        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()

            let frame_count = vid.frameCount;
            let fps = vid.frameRate;
            let method = self.sync_method;

            self.sync_in_progress = true;
            self.sync_in_progress_changed();
            
            {
                let stab = self.stabilizer.read(); 
                let duration_ms = stab.duration_ms;
                let ranges: Vec<(usize, usize)> = timestamps_fract.to_string().split(';').map(|x| {
                    let x = x.parse::<f64>().unwrap();
                    let range = (
                        ((x * duration_ms) - (sync_duration_ms / 2.0)).max(0.0), 
                        ((x * duration_ms) + (sync_duration_ms / 2.0)).min(duration_ms)
                    );
                    (range.0 as usize, range.1 as usize)
                }).collect();

                let frame_ranges: Vec<(usize, usize)> = ranges.iter().map(|(from, to)| (stab.frame_at_timestamp(*from as f64), stab.frame_at_timestamp(*to as f64))).collect();
                dbg!(&frame_ranges);
                let mut frame_status = HashMap::<usize, bool>::new();
                for x in &frame_ranges {
                    for frame in x.0..x.1-1 {
                        frame_status.insert(frame, false);
                    }
                }
                let frame_status = Arc::new(RwLock::new(frame_status));

                let estimator = stab.pose_estimator.clone();
                 
                let mut img_ratio = stab.lens.calib_dimension.0 / vid.surfaceWidth as f64;
                if img_ratio < 0.1 || !img_ratio.is_finite() {
                    img_ratio = 1.0;
                }
                let mtrx = stab.camera_matrix_or_default();
                estimator.set_lens_params(
                    Vector2::new(mtrx[0] / img_ratio, mtrx[4] / img_ratio),
                    Vector2::new(mtrx[2] / img_ratio, mtrx[5] / img_ratio)
                );
                estimator.every_nth_frame.store(every_nth_frame as usize, SeqCst);
                let stab_clone = self.stabilizer.clone();
                drop(stab);

                let qptr = QPointer::from(&*self);
                let frame_status2 = frame_status.clone();
                let progress = Arc::new(qmetaobject::queued_callback(move |_| {
                    if let Some(this) = qptr.as_pinned() {
                        let l = frame_status2.read();
                        let total = l.len();
                        let ready = l.iter().filter(|e| *e.1).count();
                        drop(l);

                        let mut this = this.borrow_mut();
                        this.sync_in_progress = ready < total;
                        this.sync_in_progress_changed();
                        this.chart_data_changed();
                        this.sync_progress(ready as f64 / total as f64, QString::from(format!("{}/{}", ready, total)));
                    }
                }));
                let qptr = QPointer::from(&*self);
                let set_offsets = Arc::new(qmetaobject::queued_callback(move |offsets: Vec<(f64, f64, f64)>| {
                    if let Some(this) = qptr.as_pinned() {
                        let mut this = this.borrow_mut();
                        {
                            let mut stab = this.stabilizer.write();
                            for x in offsets {
                                println!("Setting offset at {:.4}: {:.4} (cost {:.4})", x.0, x.1, x.2);
                                stab.gyro.set_offset((x.0 * 1000.0) as i64, x.1);
                            }
                        }
                        this.update_offset_model();
                        this.recompute_threaded();
                        this.chart_data_changed();
                        this.sync_progress(1.0, QString::default());
                        this.sync_in_progress = false;
                        this.sync_in_progress_changed();
                    }
                }));

                let total_read_frames = Arc::new(AtomicUsize::new(0));
                let total_detected_frames = Arc::new(AtomicUsize::new(0));
                
                let video_path = Self::url_to_path(&QString::from(vid.url.clone()).to_string()).to_string();
                let (sw, sh) = (vid.surfaceWidth, vid.surfaceHeight);

                self.cancel_flag.store(false, SeqCst);
                let cancel_flag = self.cancel_flag.clone();
                let cancel_flag2 = self.cancel_flag.clone();
                THREAD_POOL.spawn(move || {
                    let mut proc = FfmpegProcessor::from_file(&video_path, true).unwrap();
                    proc.on_frame(|timestamp_us, input_frame, converter| {
                        let frame = ((timestamp_us as f64 / 1000.0) * fps / 1000.0).round() as i32;

                        if let Some(current_range) = frame_ranges.iter().find(|(from, to)| (*from..*to).contains(&(frame as usize))).copied() {
                            if frame % every_nth_frame as i32 != 0 {
                                // Don't analyze this frame
                                frame_status.write().insert(frame as usize, true);
                                estimator.insert_empty_result(frame as usize, method);
                                return;
                            }
                            let mut small_frame = converter.scale(input_frame, ffmpeg_next::format::Pixel::GRAY8, sw, sh);
    
                            let (width, height, pixels) = (small_frame.plane_width(0), small_frame.plane_height(0), small_frame.data_mut(0));
    
                            total_read_frames.fetch_add(1, SeqCst);
                            println!("frame: {}, range: {}..{}", frame, current_range.0, current_range.1);

                            let img = PoseEstimator::yuv_to_gray(width, height, pixels);
        
                            let cancel_flag = cancel_flag2.clone();
                            let estimator = estimator.clone();
                            let progress = progress.clone();
                            let frame_status = frame_status.clone();
                            let total_detected_frames = total_detected_frames.clone();
                            THREAD_POOL.spawn(move || {
                                if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                                    total_detected_frames.fetch_add(1, SeqCst);
                                    return;
                                }
                                estimator.detect_features(frame as usize, method, img);
                                total_detected_frames.fetch_add(1, SeqCst);

                                if frame % 7 == 0 {
                                    estimator.process_detected_frames(frame_count as usize, duration_ms, fps);
                                }

                                let processed_frames = estimator.processed_frames(current_range.0..current_range.1);
                                for x in processed_frames { frame_status.write().insert(x, true); }
                                progress(());
                            });
                        }
                    });
                    if let Err(e) = proc.start_decoder_only(ranges, cancel_flag) {
                        eprintln!("ffmpeg error: {:?}", e);
                    }

                    while total_detected_frames.load(SeqCst) < total_read_frames.load(SeqCst) {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    println!("finished OF");
                    estimator.process_detected_frames(frame_count as usize, duration_ms, fps);
                    estimator.recalculate_gyro_data(frame_count as usize, duration_ms, fps, true);

                    for v in frame_status.write().values_mut() {
                        *v = true;
                    }
                    progress(());
                    let offsets = estimator.find_offsets(initial_offset, sync_search_size, &stab_clone.read().gyro);
                    set_offsets(offsets);
                });
            }
        }
    }

    fn update_chart(&mut self, chart: QJSValue) {
        if let Some(chart) = chart.to_qobject::<TimelineGyroChart>() {
            let chart = unsafe { &mut *chart.as_ptr() }; // _self.borrow_mut();
            
            let stab = self.stabilizer.read();
            stab.pose_estimator.recalculate_gyro_data(stab.frame_count, stab.duration_ms, stab.fps, false);
            chart.setSyncResults(&*stab.pose_estimator.estimated_gyro.read());

            chart.setFromGyroSource(&stab.gyro);
        }
    }

    fn load_telemetry(&mut self, url: QUrl, is_main_video: bool, player: QJSValue, chart: QJSValue) {
        let s = Self::url_to_path(&QString::from(url).to_string()).to_string();
        let stab = self.stabilizer.clone();
        let filename = QString::from(s.split('/').last().unwrap());

        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()
            let duration_ms = vid.duration;
            let fps = vid.frameRate;
            let frame_count = vid.frameCount as usize;
            let video_size = (vid.videoWidth as usize, vid.videoHeight as usize);

            if is_main_video {
                self.set_preview_resolution(720, player);
            }

            let qptr = QPointer::from(&*self);
            let finished = qmetaobject::queued_callback(move |params: (bool, QString, QString, QString, bool, bool, f64)| {
                if let Some(this) = qptr.as_pinned() { 
                    let mut this = this.borrow_mut();
                    this.gyro_loaded = params.4; // Contains gyro
                    this.gyro_changed();
                    
                    this.recompute_threaded();
                    this.update_offset_model();
                    this.telemetry_loaded(params.0, params.1, params.2, params.3, params.4, params.5, params.6);    
                }
            });
            
            if duration_ms > 0.0 && fps > 0.0 {
                THREAD_POOL.spawn(move || {
                    let detected = {
                        let mut stab = stab.write(); // TODO: this locks the mutex for too long, fix it
                        
                        if is_main_video {
                            stab.init_from_video_data(&s, duration_ms, fps, frame_count, video_size);
                        } else {
                            stab.load_gyro_data(&s);
                        }
                        stab.recompute_smoothness();

                        let detected = stab.gyro.detected_source.as_ref().map(String::clone).unwrap_or_default();
                        let orientation = stab.gyro.org_imu_orientation.as_ref().map(String::clone).unwrap_or("XYZ".into());
                        let has_gyro = !stab.gyro.quaternions.is_empty();
                        let has_quats = !stab.gyro.org_quaternions.is_empty();

                        if let Some(chart) = chart.to_qobject::<TimelineGyroChart>() {
                            let chart = unsafe { &mut *chart.as_ptr() }; // _self.borrow_mut();
                            chart.setDurationMs(duration_ms);
                            chart.setFromGyroSource(&stab.gyro);
                        }
                        
                        (detected, orientation, has_gyro, has_quats, stab.frame_readout_time)
                    };

                    finished((is_main_video, filename, QString::from(detected.0.trim()), QString::from(detected.1), detected.2, detected.3, detected.4));
                });
            }
        }
    }
    fn load_lens_profile(&mut self, path: QString) {
        let info = {
            let mut stab = self.stabilizer.write();
            stab.load_lens_profile(&Self::url_to_path(&path.to_string()).to_string()); // TODO errors
            QJsonObject::from(stab.lens.get_info())
        };
        self.lens_loaded = true;
        self.lens_changed();
        self.lens_profile_loaded(info);
        self.recompute_threaded();
    }
    
    fn set_preview_resolution(&self, target_height: i32, player: QJSValue) {
        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()

            fn aligned_to_8(mut x: u32) -> u32 { if x % 8 != 0 { x += 8 - x % 8; } x }

            let h = if target_height > 0 { target_height as u32 } else { vid.videoHeight };
            let ratio = vid.videoHeight as f64 / h as f64;
            let new_w = aligned_to_8((vid.videoWidth as f64 / ratio).floor() as u32);
            let new_h = aligned_to_8((vid.videoHeight as f64 / (vid.videoWidth as f64 / new_w as f64)).floor() as u32);
            println!("surface size: {}x{}", new_w, new_h);

            self.stabilizer.write().pose_estimator.clear();
            self.chart_data_changed();

            vid.setSurfaceSize(new_w, new_h);
            vid.setCurrentFrame(vid.currentFrame)
        }
    }

    fn update_lpf(&mut self, lpf: f64) {
        self.stabilizer.write().gyro.set_lowpass_filter(lpf);
        
        self.chart_data_changed();
        self.recompute_threaded();
    }

    fn update_imu_rotation(&mut self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64) {
        self.stabilizer.write().gyro.set_imu_rotation(pitch_deg, roll_deg, yaw_deg);

        self.chart_data_changed();
        self.recompute_threaded();
    }
    fn update_imu_orientation(&mut self, orientation: String) {
        self.stabilizer.write().gyro.set_imu_orientation(orientation);

        self.chart_data_changed();
        self.recompute_threaded();
    }
    fn set_integration_method(&mut self, index: usize) {
        println!("set_integration_method {}", index);
        
        let qptr = QPointer::from(&*self);
        let finished = qmetaobject::queued_callback(move |_| {
            if let Some(this) = qptr.as_pinned() { 
                let mut this = this.borrow_mut();
                this.chart_data_changed();
                this.recompute_threaded();
            }
        });
        let stab = self.stabilizer.clone();
        THREAD_POOL.spawn(move || {
            {
                let mut stab = stab.write();
                stab.gyro.integration_method = index;
                stab.gyro.integrate();
                stab.recompute_smoothness();
            }
            finished(());
        });
    }

    fn update_sync_lpf(&mut self, lpf: f64) {
        {
            let stab = self.stabilizer.write();
            stab.pose_estimator.lowpass_filter(lpf, stab.frame_count, stab.duration_ms, stab.fps);
        }
        
        self.chart_data_changed();
        self.recompute_threaded();
    }

    fn init_player(&mut self, player: QJSValue) {
        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()

            let bg_color = vid.getBackgroundColor().get_rgba_f();
            self.stabilizer.write().background = Vector4::new(bg_color.0 as f32 * 255.0, bg_color.1 as f32 * 255.0, bg_color.2 as f32 * 255.0, bg_color.3 as f32 * 255.0);

            let stab = self.stabilizer.clone();
            vid.onResize(Box::new(move |width, height| {
                stab.write().init_size(width as usize, height as usize);
            }));

            let stab = self.stabilizer.clone();
            vid.onProcessPixels(Box::new(move |frame, width, height, pixels: &mut [u8]| -> *mut u8 {
                // let _time = std::time::Instant::now();

                let ptr = stab.write().process_pixels(frame as usize, width as usize, height as usize, width as usize, pixels);

                //println!("Frame {}, {}x{}, {:.2} MB | OpenCL {:.3}ms", frame, width, height, pixels.len() as f32 / 1024.0 / 1024.0, _time.elapsed().as_micros() as f64 / 1000.0);
                ptr
            }));
        }
    }

    fn set_background_color(&mut self, color: QString, player: QJSValue) {
        if let Some(vid) = player.to_qobject::<MDKVideoItem>() {
            let vid = unsafe { &mut *vid.as_ptr() }; // vid.borrow_mut()

            let color = QColor::from_name(&color.to_string());
            vid.setBackgroundColor(color);

            let bg = color.get_rgba_f();
            let bg = Vector4::new(bg.0 as f32 * 255.0, bg.1 as f32 * 255.0, bg.2 as f32 * 255.0, bg.3 as f32 * 255.0);
            
            let mut stab = self.stabilizer.write();
            stab.background = bg;
            stab.undistortion.set_background(bg);
        }
    }

    fn set_smoothing_method(&mut self, index: usize) -> QJsonArray {
        let ret = {
            let mut stab = self.stabilizer.write();
            stab.smoothing_id = index;

            let algorithm = stab.smoothing_algs[index].as_ref();

            simd_json_to_qt(&algorithm.get_parameters_json())
        };
        self.recompute_threaded();
        self.chart_data_changed();
        ret
    }
    fn set_smoothing_param(&mut self, name: QString, val: f64) {
        {
            let mut stab = self.stabilizer.write();
            let id = stab.smoothing_id;
            let algorithm = stab.smoothing_algs[id].as_mut();

            algorithm.set_parameter(&name.to_string(), val);
        }
        self.chart_data_changed();
        self.recompute_threaded();
    }

    fn set_stab_enabled(&mut self, enabled: bool) {
        self.stabilizer.write().stab_enabled = enabled;
    }
    fn set_show_detected_features(&mut self, enabled: bool) {
        self.stabilizer.write().show_detected_features = enabled;
    }
    fn set_sync_method(&mut self, v: u32) {
        self.sync_method = v;

        self.stabilizer.write().pose_estimator.clear();
        self.chart_data_changed();
    }
    fn set_fov(&mut self, fov: f64) {
        self.stabilizer.write().fov = fov;
        self.recompute_threaded();
    }

    fn recompute_threaded(&mut self) {
        let stab = self.stabilizer.clone();
        let compute_id = fastrand::u64(..);
        CURRENT_COMPUTE_ID.store(compute_id, SeqCst);

        self.compute_progress(compute_id, 0.0);

        let qptr = QPointer::from(&*self);
        let finished = qmetaobject::queued_callback(move |cid| {
            if let Some(this) = qptr.as_pinned() { 
                println!("compute finish");
                this.borrow().compute_progress(cid, 1.0);
            }
        });

        THREAD_POOL.spawn(move || {
            let params = {
                let mut stab = stab.write();
                stab.recompute_smoothness();
                undistortion::ComputeParams::from_manager(&stab)
            };
            if let Ok(stab_data) = undistortion::Undistortion::<undistortion::RGBA8>::calculate_stab_data(&params, &CURRENT_COMPUTE_ID, compute_id) {
                let mut stab = stab.write();
                stab.undistortion.stab_data = stab_data;

                finished(compute_id);
            }
        });
    }
    fn set_frame_readout_time(&mut self, v: f64) {
        self.stabilizer.write().frame_readout_time = v;
        
        self.recompute_threaded();
    }

    fn render(&self, codec: String, output_path: String, trim_start: f64, trim_end: f64, output_width: usize, output_height: usize, use_gpu: bool, audio: bool) {
        let qptr = QPointer::from(&*self);
        let progress = qmetaobject::queued_callback(move |params: (f64, usize, usize)| {
            if let Some(this) = qptr.as_pinned() { 
                this.borrow().render_progress(params.0, params.1, params.2);
            }
        });

        let trim_ratio = trim_end - trim_start;
        let total_frame_count = self.stabilizer.read().frame_count;
        let video_path = self.video_path.clone();

        progress((0.0, 0, (total_frame_count as f64 * trim_ratio).round() as usize));

        self.cancel_flag.store(false, SeqCst);
        let cancel_flag = self.cancel_flag.clone();

        let stab = self.stabilizer.clone();
        THREAD_POOL.spawn(move || {
            let stab = stab.read().get_render_stabilizator();
            rendering::render(stab, progress, video_path, codec, output_path, trim_start, trim_end, output_width, output_height, use_gpu, audio, cancel_flag);
        });
    }
    
    fn set_trim_start(&mut self, v: f64) {
        self.stabilizer.write().trim_start = v;
        self.recompute_threaded();
    }
    fn set_trim_end(&mut self, v: f64) {
        self.stabilizer.write().trim_end = v;
        self.recompute_threaded();
    }
    fn file_exists(&self, path: QString) -> bool {
        std::path::Path::new(&path.to_string()).exists()
    }
    
    fn cancel_current_operation(&mut self) {
        self.cancel_flag.store(true, SeqCst);
    }

    fn export_gyroflow(&self) {
        // TODO
    }
}

// These are safe because we only have one Controller for the lifetime of the program
unsafe impl Send for Controller { }
unsafe impl Sync for Controller { }

fn simd_json_to_qt(v: &simd_json::owned::Value) -> QJsonArray {
    let mut arr = QJsonArray::default();
    use simd_json::ValueAccess;
    for param in v.as_array().unwrap() {
        let mut map = QJsonObject::default();
        for (k, v) in param.as_object().unwrap() {
            match v {
                simd_json::OwnedValue::Static(simd_json::StaticNode::F64(v)) => { map.insert(k, QJsonValue::from(*v)); },
                simd_json::OwnedValue::Static(simd_json::StaticNode::I64(v)) => { map.insert(k, QJsonValue::from(*v as f64)); },
                simd_json::OwnedValue::Static(simd_json::StaticNode::U64(v)) => { map.insert(k, QJsonValue::from(*v as f64)); },
                simd_json::OwnedValue::Static(simd_json::StaticNode::Bool(v)) => { map.insert(k, QJsonValue::from(*v)); },
                simd_json::OwnedValue::String(v) => { map.insert(k, QJsonValue::from(QString::from(v.clone()))); },
                _ => { println!("Unimplemented"); }
            };
        }
        arr.push(QJsonValue::from(map));
    }
    arr
}


use cpp::*;
cpp! {{
    #ifdef Q_OS_ANDROID
    #   include <QJniObject>
    #endif
}}
impl Controller {
    fn resolve_android_url(&mut self, url: QString) -> QString {
        QString::from(cpp!(unsafe [url as "QString"] -> QString as "QString" {
            #ifdef Q_OS_ANDROID
                QVariant res = QNativeInterface::QAndroidApplication::runOnAndroidMainThread([url] {
                    QJniObject jniPath = QJniObject::fromString(url);
                    QJniObject jniUri = QJniObject::callStaticObjectMethod("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", jniPath.object());

                    QJniObject activity(QNativeInterface::QAndroidApplication::context());

                    QString url = QJniObject::callStaticObjectMethod("org/ekkescorner/utils/QSharePathResolver", 
                        "getRealPathFromURI",
                        "(Landroid/content/Context;Landroid/net/Uri;)Ljava/lang/String;",
                        activity.object(), jniUri.object()
                    ).toString();
                    
                    return QVariant::fromValue(url);
                }).result();
                return res.toString();
            #else
                return url;
            #endif
        }))
    }
}
