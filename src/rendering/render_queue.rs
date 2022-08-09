// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use qmetaobject::*;

use crate::{ core, rendering, util, controller::Controller };
use crate::core::{ stabilization, StabilizationManager };
use std::sync::{ Arc, atomic::{ AtomicBool, AtomicUsize, Ordering::SeqCst } };
use std::cell::RefCell;
use std::collections::HashMap;
use parking_lot::RwLock;
use regex::Regex;

#[derive(Default, Clone, SimpleListItem)]
struct RenderQueueItem {
    pub job_id: u32,
    pub input_file: QString,
    pub output_path: QString,
    pub export_settings: QString,
    pub thumbnail_url: QString,
    pub current_frame: u64,
    pub total_frames: u64,
    pub start_timestamp: u64,
    pub end_timestamp: u64,
    pub error_string: QString,

    status: JobStatus,
}

#[derive(Default, Clone, PartialEq)]
enum JobStatus {
    #[default]
    Queued,
    Rendering,
    Finished,
    Error
}
struct Job {
    queue_index: usize,
    input_file: String,
    render_options: RenderOptions,
    sync_options: String,
    cancel_flag: Arc<AtomicBool>,
    stab: Arc<StabilizationManager<stabilization::RGBA8>>
}

#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RenderOptions {
    pub codec: String,
    pub codec_options: String,
    pub output_path: String,
    pub trim_start: f64,
    pub trim_end: f64,
    pub output_width: usize,
    pub output_height: usize,
    pub bitrate: f64,
    pub use_gpu: bool,
    pub audio: bool,
    pub pixel_format: String,
    pub override_fps: f64,

    // Advanced
    pub encoder_options: String,
    pub keyframe_distance: f64,
    pub preserve_other_tracks: bool,
    pub pad_with_black: bool,
}
impl RenderOptions {
    pub fn settings_string(&self, fps: f64) -> String {
        let codec_info = match self.codec.as_ref() {
            "x264" | "x265" => format!("{} {:.0} Mbps", self.codec, self.bitrate),
            "DNxHD" => self.codec_options.clone(),
            "ProRes" => format!("{} {}", self.codec, self.codec_options),
            _ => self.codec.clone()
        };

        format!("{}x{} {:.3}fps | {}", self.output_width, self.output_height, fps, codec_info)
    }

    pub fn get_encoder_options_dict(&self) -> ffmpeg_next::Dictionary {
        let re = Regex::new(r#"-([^\s"]+)\s+("[^"]+"|[^\s"]+)"#).unwrap();

        let mut options = ffmpeg_next::Dictionary::new();
        for x in re.captures_iter(&self.encoder_options) {
            if let Some(k) = x.get(1) {
                if let Some(v) = x.get(2) {
                    let k = k.as_str();
                    let v = v.as_str().trim_matches('"');
                    options.set(k, v);
                }
            }
        }
        options
    }
    pub fn update_from_json(&mut self, obj: &serde_json::Value) {
        if let serde_json::Value::Object(obj) = obj {
            if let Some(v) = obj.get("codec")          .and_then(|x| x.as_str())  { self.codec = v.to_string(); }
            if let Some(v) = obj.get("codec_options")  .and_then(|x| x.as_str())  { self.codec_options = v.to_string(); }
            if let Some(v)  = obj.get("trim_start")     .and_then(|x| x.as_f64())  { self.trim_start = v; }
            if let Some(v)  = obj.get("trim_end")       .and_then(|x| x.as_f64())  { self.trim_end = v; }
            if let Some(v)  = obj.get("output_width")   .and_then(|x| x.as_u64())  { self.output_width = v as usize; }
            if let Some(v)  = obj.get("output_height")  .and_then(|x| x.as_u64())  { self.output_height = v as usize; }
            if let Some(v)  = obj.get("bitrate")        .and_then(|x| x.as_f64())  { self.bitrate = v; }
            if let Some(v) = obj.get("use_gpu")        .and_then(|x| x.as_bool()) { self.use_gpu = v; }
            if let Some(v) = obj.get("audio")          .and_then(|x| x.as_bool()) { self.audio = v; }
            if let Some(v) = obj.get("pixel_format")   .and_then(|x| x.as_str())  { self.pixel_format = v.to_string(); }
            if let Some(v)  = obj.get("override_fps")   .and_then(|x| x.as_f64())  { self.override_fps = v; }

            // Advanced
            if let Some(v) = obj.get("encoder_options")      .and_then(|x| x.as_str())  { self.encoder_options = v.to_string(); }
            if let Some(v)  = obj.get("keyframe_distance")    .and_then(|x| x.as_f64())  { self.keyframe_distance = v; }
            if let Some(v) = obj.get("preserve_other_tracks").and_then(|x| x.as_bool()) { self.preserve_other_tracks = v; }
            if let Some(v) = obj.get("pad_with_black")       .and_then(|x| x.as_bool()) { self.pad_with_black = v; }

            if let Some(v) = obj.get("output_path").and_then(|x| x.as_str()) {
                let cur_path = std::path::Path::new(&self.output_path);
                let mut new_path = std::path::Path::new(v).to_path_buf();
                if let Some(fname) = cur_path.file_name() {
                    new_path.push(fname.to_string_lossy().to_string());
                    self.output_path = new_path.to_string_lossy().replace('\\', "/");
                }
            }
        }
    }
}

#[derive(Default, QObject)]
pub struct RenderQueue {
    base: qt_base_class!(trait QObject),

    queue: qt_property!(RefCell<SimpleListModel<RenderQueueItem>>; NOTIFY queue_changed),
    jobs: HashMap<u32, Job>,

    add: qt_method!(fn(&mut self, controller: QJSValue, options_json: String, sync_options_json: String, thumbnail_url: QString) -> u32),
    remove: qt_method!(fn(&mut self, job_id: u32)),

    start: qt_method!(fn(&mut self)),
    pause: qt_method!(fn(&mut self)),
    stop: qt_method!(fn(&mut self)),

    render_job: qt_method!(fn(&mut self, job_id: u32, single: bool)),
    cancel_job: qt_method!(fn(&self, job_id: u32)),
    reset_job: qt_method!(fn(&self, job_id: u32)),
    get_gyroflow_data: qt_method!(fn(&self, job_id: u32) -> QString),

    add_file: qt_method!(fn(&mut self, url: QUrl, controller: QJSValue, options_json: String, sync_options_json: String) -> u32),

    get_job_output_path: qt_method!(fn(&self, job_id: u32) -> QString),
    set_job_output_path: qt_method!(fn(&mut self, job_id: u32, new_path: String)),

    set_pixel_format: qt_method!(fn(&mut self, job_id: u32, format: String)),
    set_error_string: qt_method!(fn(&mut self, job_id: u32, err: QString)),

    file_exists: qt_method!(fn(&self, path: QString) -> bool),

    main_job_id: qt_property!(u32),
    editing_job_id: qt_property!(u32; NOTIFY queue_changed),

    start_timestamp: qt_property!(u64; NOTIFY progress_changed),
    end_timestamp: qt_property!(u64; NOTIFY progress_changed),
    current_frame: qt_property!(u64; READ get_current_frame NOTIFY progress_changed),
    total_frames: qt_property!(u64; READ get_total_frames NOTIFY queue_changed),
    status: qt_property!(QString; NOTIFY status_changed),

    progress_changed: qt_signal!(),
    queue_changed: qt_signal!(),
    status_changed: qt_signal!(),

    render_progress: qt_signal!(job_id: u32, progress: f64, current_frame: usize, total_frames: usize, finished: bool),
    encoder_initialized: qt_signal!(job_id: u32, encoder_name: String),

    convert_format: qt_signal!(job_id: u32, format: QString, supported: QString),
    error: qt_signal!(job_id: u32, text: QString, arg: QString, callback: QString),
    added: qt_signal!(job_id: u32),

    get_encoder_options: qt_method!(fn(&self, encoder: String) -> String),
    get_default_encoder: qt_method!(fn(&self, codec: String, gpu: bool) -> String),

    apply_to_all: qt_method!(fn(&mut self, data: String)),

    pause_flag: Arc<AtomicBool>,

    default_suffix: qt_property!(QString),

    paused_timestamp: Option<u64>
}

macro_rules! update_model {
    ($this:ident, $job_id:ident, $itm:ident $action:block) => {
        {
            let mut q = $this.queue.borrow_mut();
            if let Some(job) = $this.jobs.get(&$job_id) {
                if job.queue_index < q.row_count() as usize {
                    //let mut $itm = &mut q[job.queue_index];
                    let mut $itm = q[job.queue_index].clone();
                    $action
                    q.change_line(job.queue_index, $itm);
                    //q.data_changed(job.queue_index);
                }
            }
        }
    };
}

impl RenderQueue {
    pub fn new() -> Self {
        Self {
            status: QString::from("stopped"),
            default_suffix: QString::from("_stabilized"),
            ..Default::default()
        }
    }
    pub fn get_total_frames(&self) -> u64 {
        self.queue.borrow().iter().map(|v| v.total_frames).sum()
    }
    pub fn get_current_frame(&self) -> u64 {
        self.queue.borrow().iter().map(|v| v.current_frame).sum()
    }

    pub fn set_pixel_format(&mut self, job_id: u32, format: String) {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            if format == "cpu" {
                job.render_options.use_gpu = false;
            } else {
                job.render_options.pixel_format = format;
            }
        }
        update_model!(self, job_id, itm {
            itm.error_string = QString::default();
            itm.status = JobStatus::Queued;
        });
        if self.status.to_string() != "active" {
            self.start();
        }
    }

    pub fn set_job_output_path(&mut self, job_id: u32, new_path: String) {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.render_options.output_path = new_path.clone();
        }
        update_model!(self, job_id, itm {
            itm.output_path = QString::from(new_path);
            itm.error_string = QString::default();
            itm.status = JobStatus::Queued;
        });
        if self.status.to_string() != "active" {
            self.start();
        }
    }

    pub fn set_error_string(&mut self, job_id: u32, err: QString) {
        update_model!(self, job_id, itm {
            itm.error_string = err;
            itm.status = JobStatus::Error;
        });
    }

    pub fn add(&mut self, controller: QJSValue, options_json: String, sync_options_json: String, thumbnail_url: QString) -> u32 {
        let job_id = if self.editing_job_id > 0 {
            self.editing_job_id
        } else {
            fastrand::u32(..) + 1
        };
        if self.editing_job_id > 0 {
            self.editing_job_id = 0;
            self.queue_changed();
        }

        if let Some(ctl) = controller.to_qobject::<Controller>() {
            let ctl = unsafe { &mut *ctl.as_ptr() }; // ctl.borrow_mut()
            if let Ok(render_options) = serde_json::from_str(&options_json) as serde_json::Result<RenderOptions> {
                self.add_internal(job_id, ctl.stabilizer.clone(), render_options, sync_options_json, thumbnail_url);
            }
        }
        job_id
    }

    pub fn add_internal(&mut self, job_id: u32, stab: Arc<StabilizationManager<stabilization::RGBA8>>, render_options: RenderOptions, sync_options_json: String, thumbnail_url: QString) {
        let stab = Arc::new(stab.get_render_stabilizer((render_options.output_width, render_options.output_height)));
        let params = stab.params.read();
        let trim_ratio = render_options.trim_end - render_options.trim_start;
        let video_path = stab.video_path.read().clone();

        let editing = self.jobs.contains_key(&job_id);

        if editing {
            update_model!(self, job_id, itm {
                itm.output_path = QString::from(render_options.output_path.as_str());
                itm.export_settings = QString::from(render_options.settings_string(params.fps));
                itm.thumbnail_url = thumbnail_url;
                itm.current_frame = 0;
                itm.total_frames = (params.frame_count as f64 * trim_ratio).ceil() as u64;
                itm.start_timestamp = 0;
                itm.end_timestamp = 0;
                itm.error_string = QString::default();
                itm.status = JobStatus::Queued;
            });
        } else {
            let mut q = self.queue.borrow_mut();
            q.push(RenderQueueItem {
                job_id,
                input_file: QString::from(video_path.as_str()),
                output_path: QString::from(render_options.output_path.as_str()),
                export_settings: QString::from(render_options.settings_string(params.fps)),
                thumbnail_url,
                current_frame: 0,
                total_frames: (params.frame_count as f64 * trim_ratio).ceil() as u64,
                start_timestamp: 0,
                end_timestamp: 0,
                error_string: QString::default(),
                status: JobStatus::Queued,
            });
        }

        self.jobs.insert(job_id, Job {
            queue_index: 0,
            input_file: video_path,
            render_options,
            sync_options: sync_options_json,
            cancel_flag: Default::default(),
            stab: stab.clone()
        });
        self.update_queue_indices();

        self.queue_changed();
        self.added(job_id);
    }

    pub fn get_job_output_path(&self, job_id: u32) -> QString {
        let q = self.queue.borrow();
        if let Some(job) = self.jobs.get(&job_id) {
            if job.queue_index < q.row_count() as usize {
                return q[job.queue_index].output_path.clone();
            }
        }
        QString::default()
    }
    pub fn remove(&mut self, job_id: u32) {
        if let Some(job) = self.jobs.get(&job_id) {
            job.cancel_flag.store(true, SeqCst);
            self.queue.borrow_mut().remove(job.queue_index);
            if self.editing_job_id == job_id {
                self.editing_job_id = 0;
            }
            self.queue_changed();
        }
        self.jobs.remove(&job_id);
        self.update_queue_indices();
    }
    fn update_queue_indices(&mut self) {
        for (i, v) in self.queue.borrow().iter().enumerate() {
            if let Some(job) = self.jobs.get_mut(&v.job_id) {
                job.queue_index = i;
            }
        }
    }
    fn current_timestamp() -> u64 {
        if let Ok(time) = std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH) {
            time.as_millis() as u64
        } else {
            0
        }
    }

    pub fn start(&mut self) {
        let paused = self.pause_flag.load(SeqCst);

        for (_id, job) in self.jobs.iter() {
            job.cancel_flag.store(false, SeqCst);
        }
        self.pause_flag.store(false, SeqCst);

        self.status = QString::from("active");
        self.status_changed();

        if !paused && self.start_timestamp == 0 {
            self.start_timestamp = Self::current_timestamp();
            self.progress_changed();
        } else if let Some(paused_timestamp) = self.paused_timestamp.take() {
            let diff =  Self::current_timestamp() - paused_timestamp;
            self.start_timestamp += diff;
            let mut q = self.queue.borrow_mut();
            for i in 0..q.row_count() as usize {
                //let mut v = &mut q[i];
                let mut v = q[i].clone();
                if v.start_timestamp > 0 && v.current_frame < v.total_frames {
                    v.start_timestamp += diff;
                    //q.data_changed(i);
                    q.change_line(i, v);
                }
            }
        }

        if !paused {
            let mut job_id = None;
            for v in self.queue.borrow().iter() {
                if v.current_frame == 0 && v.total_frames > 0 && v.status == JobStatus::Queued {
                    job_id = Some(v.job_id);
                    break;
                }
            }
            if let Some(job_id) = job_id {
                self.render_job(job_id, false);
            } else {
                self.start_timestamp = 0;
                self.progress_changed();

                self.status = QString::from("stopped");
                self.status_changed();
            }
        }
    }
    pub fn pause(&mut self) {
        self.pause_flag.store(true, SeqCst);
        self.paused_timestamp = Some(Self::current_timestamp());

        self.status = QString::from("paused");
        self.status_changed();
    }
    pub fn stop(&mut self) {
        self.pause_flag.store(false, SeqCst);
        for (_id, job) in self.jobs.iter() {
            job.cancel_flag.store(true, SeqCst);
        }
        self.status = QString::from("stopped");
        self.status_changed();
    }
    pub fn cancel_job(&self, job_id: u32) {
        if let Some(job) = self.jobs.get(&job_id) {
            job.cancel_flag.store(true, SeqCst);
        }
    }
    pub fn reset_job(&self, job_id: u32) {
        if let Some(job) = self.jobs.get(&job_id) {
            job.cancel_flag.store(true, SeqCst);
        }
        update_model!(self, job_id, itm {
            itm.error_string = QString::default();
            itm.current_frame = 0;
            itm.status = JobStatus::Queued;
        });
    }
    pub fn update_status(&mut self) {
        for v in self.queue.borrow().iter() {
            if v.total_frames > 0 && v.status == JobStatus::Rendering {
                self.status = QString::from("active");
                self.status_changed();
                return;
            }
        }

        self.status = QString::from("stopped");
        self.status_changed();
    }

    pub fn get_gyroflow_data(&self, job_id: u32) -> QString {
        if let Some(job) = self.jobs.get(&job_id) {
            if let Ok(data) = job.stab.export_gyroflow_data(true, false, serde_json::to_string(&job.render_options).unwrap_or_default(), job.sync_options.clone()) {
                return QString::from(data);
            }
        }
        QString::default()
    }

    pub fn render_job(&mut self, job_id: u32, single: bool) {
        if let Some(job) = self.jobs.get(&job_id) {
            {
                let mut q = self.queue.borrow_mut();
                if job.queue_index < q.row_count() as usize {
                    //let mut itm = &mut q[job.queue_index];
                    let mut itm = q[job.queue_index].clone();
                    if itm.status == JobStatus::Rendering || itm.status == JobStatus::Finished {
                        ::log::warn!("Job is already rendering {}", job_id);
                        return;
                    }
                    itm.status = JobStatus::Rendering;
                    //q.data_changed(job.queue_index);
                    q.change_line(job.queue_index, itm);
                }
            }
            job.cancel_flag.store(false, SeqCst);

            if self.start_timestamp == 0 {
                self.start_timestamp = Self::current_timestamp();
                self.status = QString::from("active");
                self.status_changed();
            }

            let stab = job.stab.clone();

            rendering::clear_log();

            let rendered_frames = Arc::new(AtomicUsize::new(0));
            let rendered_frames2 = rendered_frames.clone();
            let progress = util::qt_queued_callback_mut(self, move |this, (progress, current_frame, total_frames, finished): (f64, usize, usize, bool)| {
                rendered_frames2.store(current_frame, SeqCst);

                update_model!(this, job_id, itm {
                    itm.current_frame = current_frame as u64;
                    itm.total_frames = total_frames as u64;
                    if itm.start_timestamp == 0 {
                        itm.start_timestamp = Self::current_timestamp();
                    }
                    itm.end_timestamp = Self::current_timestamp();
                    if finished {
                        itm.status = JobStatus::Finished;
                    }
                });

                this.end_timestamp = Self::current_timestamp();
                this.render_progress(job_id, progress, current_frame, total_frames, finished);
                this.progress_changed();

                if finished {
                    if !single {
                        // Start the next one
                        this.start();
                    } else {
                        this.update_status();
                    }
                }
            });
            let encoder_initialized = util::qt_queued_callback_mut(self, move |this, encoder_name: String| {
                if let Some(job) = this.jobs.get(&job_id) {
                    if job.render_options.use_gpu && (encoder_name == "libx264" || encoder_name == "libx265" || encoder_name == "prores_ks") {
                        update_model!(this, job_id, itm {
                            itm.error_string = QString::from("uses_cpu");
                        });
                    }
                }
                this.encoder_initialized(job_id, encoder_name);
            });

            let err = util::qt_queued_callback_mut(self, move |this, (msg, mut arg): (String, String)| {
                arg.push_str("\n\n");
                arg.push_str(&rendering::get_log());

                update_model!(this, job_id, itm {
                    itm.error_string = QString::from(arg.clone());
                    itm.status = JobStatus::Error;
                });

                this.error(job_id, QString::from(msg), QString::from(arg), QString::default());
                this.render_progress(job_id, 1.0, 0, 0, true);

                if !single {
                    // Start the next one
                    this.start();
                }
                this.update_status();
            });

            let convert_format = util::qt_queued_callback_mut(self, move |this, (format, mut supported): (String, String)| {
                use itertools::Itertools;
                supported = supported
                    .split(',')
                    .filter(|v| !["CUDA", "D3D11", "BGRZ", "RGBZ", "BGRA", "UYVY422", "VIDEOTOOLBOX", "DXVA2", "MEDIACODEC", "VULKAN", "OPENCL", "QSV"].contains(v))
                    .join(",");

                update_model!(this, job_id, itm {
                    itm.error_string = QString::from(format!("convert_format:{};{}", format, supported));
                    itm.status = JobStatus::Error;
                });

                this.convert_format(job_id, QString::from(format), QString::from(supported));
                this.render_progress(job_id, 1.0, 0, 0, true);

                if !single {
                    // Start the next one
                    this.start();
                }
                this.update_status();
            });
            let trim_ratio = job.render_options.trim_end - job.render_options.trim_start;
            let total_frame_count = stab.params.read().frame_count;
            let video_path = job.input_file.clone();
            let render_options = job.render_options.clone();

            progress((0.0, 0, (total_frame_count as f64 * trim_ratio).round() as usize, false));

            job.cancel_flag.store(false, SeqCst);
            let cancel_flag = job.cancel_flag.clone();
            let pause_flag = self.pause_flag.clone();

            core::run_threaded(move || {
                let mut i = 0;
                loop {
                    let result = rendering::render(stab.clone(), progress.clone(), &video_path, &render_options, i, cancel_flag.clone(), pause_flag.clone(), encoder_initialized.clone());
                    if let Err(e) = result {
                        if let rendering::FFmpegError::PixelFormatNotSupported((fmt, supported)) = e {
                            convert_format((format!("{:?}", fmt), supported.into_iter().map(|v| format!("{:?}", v)).collect::<Vec<String>>().join(",")));
                            break;
                        }
                        if rendered_frames.load(SeqCst) == 0 {
                            if (0..4).contains(&i) {
                                // Try 4 times with different GPU decoders
                                i += 1;
                                continue;
                            }
                            if (0..5).contains(&i) {
                                // Try without GPU decoder
                                i = -1;
                                continue;
                            }
                        }
                        err(("An error occured: %1".to_string(), e.to_string()));
                        break;
                    } else {
                        // Render ok
                        break;
                    }
                }
            });
        }
    }

    fn get_output_path(suffix: &str, path: &str, codec: &str) -> String {
        let mut path = std::path::Path::new(path).with_extension("");

        let ext = match codec {
            "ProRes"        => ".mov",
            "DNxHD"         => ".mov",
            "EXR Sequence"  => "_%05d.exr",
            "PNG Sequence"  => "_%05d.png",
            _ => ".mp4"
        };

        path.set_file_name(format!("{}{}{}", path.file_name().map(|v| v.to_string_lossy()).unwrap_or_default(), suffix, ext));

        path.to_string_lossy().to_string()
    }

    pub fn add_file(&mut self, url: QUrl, controller: QJSValue, options_json: String, sync_options_json: String) -> u32 {
        let job_id = fastrand::u32(..);

        let path = util::url_to_path(url);

        let err = util::qt_queued_callback_mut(self, move |this, (msg, arg): (String, String)| {
            ::log::warn!("[add_file]: {}", arg);
            update_model!(this, job_id, itm {
                itm.error_string = QString::from(arg.clone());
                itm.status = JobStatus::Error;
            });
            this.error(job_id, QString::from(msg), QString::from(arg), QString::default());
        });

        let suffix = self.default_suffix.to_string();

        if let Some(ctl) = controller.to_qobject::<Controller>() {
            let ctl = unsafe { &mut *ctl.as_ptr() }; // ctl.borrow_mut()
            if let Ok(mut render_options) = serde_json::from_str(&options_json) as serde_json::Result<RenderOptions> {

                let (smoothing_name, smoothing_params) = {
                    let smoothing_lock = ctl.stabilizer.smoothing.read();
                    let smoothing = smoothing_lock.current();
                    (smoothing.get_name(), smoothing.get_parameters_json())
                };
                let params = ctl.stabilizer.params.read();

                let stab = StabilizationManager {
                    params: Arc::new(RwLock::new(core::stabilization_params::StabilizationParams {
                        fov:                    params.fov,
                        background:             params.background,
                        adaptive_zoom_window:   params.adaptive_zoom_window,
                        lens_correction_amount: params.lens_correction_amount,
                        background_mode:           params.background_mode,
                        background_margin:         params.background_margin,
                        background_margin_feather: params.background_margin_feather,
                        ..Default::default()
                    })),
                    video_path: Arc::new(RwLock::new(path.clone())),
                    lens_profile_db: ctl.stabilizer.lens_profile_db.clone(),
                    ..Default::default()
                };

                {
                    let method_idx = stab.get_smoothing_algs()
                        .iter().enumerate()
                        .find(|(_, m)| smoothing_name == m.as_str())
                        .map(|(idx, _)| idx)
                        .unwrap_or_default();

                    let mut smoothing = stab.smoothing.write();
                    smoothing.set_current(method_idx);

                    for param in smoothing_params.as_array().unwrap() {
                        (|| -> Option<()> {
                            let name = param.get("name").and_then(|x| x.as_str())?;
                            let value = param.get("value").and_then(|x| x.as_f64())?;
                            smoothing.current_mut().set_parameter(name, value);
                            Some(())
                        })();
                    }
                }

                let stab = Arc::new(stab);

                let stab2 = stab.clone();
                let sync_options_json2 = sync_options_json.clone();
                let loaded = util::qt_queued_callback_mut(self, move |this, (render_options, ask_path): (RenderOptions, bool)| {
                    let out_path = render_options.output_path.clone();
                    this.add_internal(job_id, stab2.clone(), render_options, sync_options_json2.clone(), QString::default());

                    if ask_path && std::path::Path::new(&out_path).exists() {
                        update_model!(this, job_id, itm {
                            itm.error_string = QString::from(format!("file_exists:{}", out_path));
                            itm.status = JobStatus::Error;
                        });
                    }
                });
                let thumb_fetched = util::qt_queued_callback_mut(self, move |this, thumb: QString| {
                    update_model!(this, job_id, itm { itm.thumbnail_url = thumb; });
                });
                let apply_preset = util::qt_queued_callback_mut(self, move |this, preset: String| {
                    this.apply_to_all(preset);
                    this.added(job_id);
                });

                core::run_threaded(move || {
                    let fetch_thumb = |video_path: &str, ratio: f64| -> Result<(), rendering::FFmpegError> {
                        let mut fetched = false;
                        {
                            let mut proc = rendering::VideoProcessor::from_file(video_path, false, 0, None)?;
                            proc.on_frame(move |_timestamp_us, input_frame, _output_frame, converter| {
                                let sf = converter.scale(input_frame, ffmpeg_next::format::Pixel::RGBA, (50.0 * ratio).round() as u32, 50)?;

                                if !fetched {
                                    thumb_fetched(util::image_data_to_base64(sf.plane_width(0), sf.plane_height(0), sf.stride(0) as u32, sf.data(0)));
                                    fetched = true;
                                }

                                Ok(())
                            });
                            proc.start_decoder_only(vec![(0.0, 0.0)], Arc::new(AtomicBool::new(false)))?;
                        }
                        Ok(())
                    };

                    if path.ends_with(".gyroflow") {
                        let video_path = || -> Option<String> {
                            let data = std::fs::read(&path).ok()?;
                            let obj: serde_json::Value = serde_json::from_slice(&data).ok()?;
                            Some(obj.get("videofile")?.as_str()?.to_string())
                        }().unwrap_or_default();

                        if video_path.is_empty() {
                            // It's a preset
                            if let Ok(data) = std::fs::read_to_string(&path) {
                                apply_preset(data);
                            }
                            return;
                        }

                        match stab.import_gyroflow_file(&path, true, |_|(), Arc::new(AtomicBool::new(false))) {
                            Ok(obj) => {
                                if let Some(out) = obj.get("output") {
                                    if let Ok(render_options2) = serde_json::from_value(out.clone()) as serde_json::Result<RenderOptions> {
                                        loaded((render_options2, true));
                                    }
                                }
                                if let Some(out) = obj.get("videofile").and_then(|x| x.as_str()) {
                                    let ratio = {
                                        let params = stab.params.read();
                                        params.video_size.0 as f64 / params.video_size.1 as f64
                                    };

                                    if let Err(e) = fetch_thumb(out, ratio) {
                                        err(("An error occured: %1".to_string(), e.to_string()));
                                    }
                                }
                            },
                            Err(e) => {
                                err(("An error occured: %1".to_string(), format!("Error loading {}: {:?}", path, e)));
                            }
                        }
                    } else if let Ok(info) = rendering::FfmpegProcessor::get_video_info(&path) {
                        ::log::info!("Loaded {:?}", &info);

                        render_options.bitrate = render_options.bitrate.max(info.bitrate);
                        render_options.output_width = info.width as usize;
                        render_options.output_height = info.height as usize;
                        render_options.output_path = Self::get_output_path(&suffix, &path, &render_options.codec);
                        render_options.trim_start = 0.0;
                        render_options.trim_end = 1.0;

                        let ratio = info.width as f64 / info.height as f64;

                        if info.duration_ms > 0.0 && info.fps > 0.0 {

                            let video_size = (info.width as usize, info.height as usize);

                            if let Err(e) = stab.init_from_video_data(&path, info.duration_ms, info.fps, info.frame_count, video_size) {
                                err(("An error occured: %1".to_string(), e.to_string()));
                                return;
                            }
                            let _ = stab.load_gyro_data(&path, |_|(), Arc::new(AtomicBool::new(false)));
                            let camera_id = stab.camera_id.read();

                            let id_str = camera_id.as_ref().map(|v| v.identifier.clone()).unwrap_or_default();
                            if !id_str.is_empty() {
                                let db = stab.lens_profile_db.read();
                                if db.contains_id(&id_str) {
                                    match stab.load_lens_profile(&id_str) {
                                        Ok(_) => {
                                            if let Some(fr) = stab.lens.read().frame_readout_time {
                                                stab.params.write().frame_readout_time = fr;
                                            }
                                        }
                                        Err(e) => {
                                            err(("An error occured: %1".to_string(), e.to_string()));
                                            return;
                                        }
                                    }
                                }
                            }
                            if let Some(output_dim) = stab.lens.read().output_dimension.clone() {
                                render_options.output_width = output_dim.w;
                                render_options.output_height = output_dim.h;
                            }

                            stab.set_size(video_size.0, video_size.1);
                            stab.set_output_size(render_options.output_width, render_options.output_height);

                            let contains_gyro = !stab.gyro.read().quaternions.is_empty();

                            let mut ask_path = true;

                            let sync_settings = stab.lens.read().sync_settings.clone();
                            if let Some(sync_settings) = sync_settings {
                                if contains_gyro && sync_settings.get("do_autosync").and_then(|v| v.as_bool()).unwrap_or_default() {
                                    // ----------------------------------------------------------------------------
                                    // --------------------------------- AUtosync ---------------------------------
                                    loaded((render_options.clone(), true));
                                    ask_path = false;
                                    use gyroflow_core::synchronization::AutosyncProcess;
                                    use gyroflow_core::synchronization;
                                    use crate::rendering::VideoProcessor;
                                    use itertools::Either;

                                    if let Ok(serde_json::Value::Object(mut sync_options)) = serde_json::from_str(&sync_options_json) {
                                        for (k, v) in sync_settings.as_object().unwrap() {
                                            sync_options.insert(k.clone(), v.clone());
                                        }

                                        if let Some(points) = sync_options.get("max_sync_points").and_then(|v| v.as_i64()) {
                                            let chunks = 1.0 / points as f64;
                                            let start = chunks / 2.0;
                                            let mut timestamps_fract: Vec<f64> = (0..points).map(|i| start + (i as f64 * chunks)).collect();
                                            if let Some(v) = sync_options.get("custom_sync_timestamps").and_then(|v| v.as_array()) {
                                                timestamps_fract = v.iter().filter_map(|v| v.as_f64()).filter(|v| *v <= info.duration_ms).map(|v| v / info.duration_ms).collect();
                                            }

                                            if let Ok(mut sync_params) = serde_json::from_value(serde_json::Value::Object(sync_options)) as serde_json::Result<synchronization::SyncParams> {

                                                let cancel_flag = Arc::new(AtomicBool::new(false));
                                                sync_params.initial_offset     *= 1000.0; // s to ms
                                                sync_params.time_per_syncpoint *= 1000.0; // s to ms
                                                sync_params.search_size        *= 1000.0; // s to ms

                                                let size = stab.params.read().size;
                                                let (sw, sh) = ((720.0 * (size.0 as f64 / size.1 as f64)) as u32, 720);
                                                stab.set_size(sw as usize, sh as usize);

                                                if let Ok(mut sync) = AutosyncProcess::from_manager(&stab, &timestamps_fract, sync_params, "synchronize".into(), cancel_flag.clone()) {
                                                    let stab2 = stab.clone();
                                                    sync.on_finished(move |arg| {
                                                        if let Either::Left(offsets) = arg {
                                                            let mut gyro = stab2.gyro.write();
                                                            for x in offsets {
                                                                ::log::info!("Setting offset at {:.4}: {:.4} (cost {:.4})", x.0, x.1, x.2);
                                                                let new_ts = ((x.0 - x.1) * 1000.0) as i64;
                                                                // Remove existing offsets within 100ms range
                                                                gyro.remove_offsets_near(new_ts, 100.0);
                                                                gyro.set_offset(new_ts, x.1);
                                                            }
                                                            stab2.keyframes.write().update_gyro(&gyro);
                                                        }
                                                    });

                                                    let (sw, sh) = ((720.0 * (size.0 as f64 / size.1 as f64)) as u32, 720);
                                                    let gpu_decoding = *rendering::GPU_DECODING.read();

                                                    let mut frame_no = 0;
                                                    let sync = std::rc::Rc::new(sync);

                                                    match VideoProcessor::from_file(&path, gpu_decoding, 0, None) {
                                                        Ok(mut proc) => {
                                                            let err2 = err.clone();
                                                            let sync2 = sync.clone();
                                                            proc.on_frame(move |timestamp_us, input_frame, _output_frame, converter| {
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
                                                                Ok(())
                                                            });
                                                            if let Err(e) = proc.start_decoder_only(sync.get_ranges(), cancel_flag) {
                                                                err(("An error occured: %1".to_string(), e.to_string()));
                                                            }
                                                            sync.finished_feeding_frames();
                                                        }
                                                        Err(error) => {
                                                            dbg!(&error.to_string());
                                                            err(("An error occured: %1".to_string(), error.to_string()));
                                                        }
                                                    }
                                                } else {
                                                    err(("An error occured: %1".to_string(), "Invalid parameters".to_string()));
                                                }

                                                stab.set_size(video_size.0, video_size.1);
                                            }
                                        }
                                    }
                                    // --------------------------------- AUtosync ---------------------------------
                                    // ----------------------------------------------------------------------------
                                }
                            }

                            stab.recompute_blocking();

                            // println!("{}", stab.export_gyroflow_data(true, serde_json::to_string(&render_options).unwrap_or_default()));

                            loaded((render_options, ask_path));

                            if let Err(e) = fetch_thumb(&path, ratio) {
                                err(("An error occured: %1".to_string(), e.to_string()));
                            }
                        }
                    }
                });
            }
        }

        job_id
    }

    fn apply_to_all(&mut self, data: String) {
        dbg!(&data);
        let mut new_output_options = None;
        if let Ok(obj) = serde_json::from_str(&data) as serde_json::Result<serde_json::Value> {
            if let Some(output) = obj.get("output") {
                new_output_options = Some(output.clone());
            }
        }
        dbg!(&new_output_options);
        let data = data.as_bytes();
        let mut q = self.queue.borrow_mut();
        for (_id, job) in self.jobs.iter_mut() {
            if job.queue_index < q.row_count() as usize {
                let mut itm = q[job.queue_index].clone();
                if itm.status == JobStatus::Queued {
                    let stab = job.stab.clone();
                    let data_vec = data.to_vec();
                    core::run_threaded(move || {
                        if let Err(e) = stab.import_gyroflow_data(&data_vec, true, None, |_|(), Arc::new(AtomicBool::new(false))) {
                            ::log::error!("Failed to update queue stab data: {:?}", e);
                        }
                    });
                    if let Some(ref new_output_options) = new_output_options {
                        job.render_options.update_from_json(new_output_options);
                        itm.export_settings = QString::from(job.render_options.settings_string(job.stab.params.read().fps));
                        itm.output_path = QString::from(job.render_options.output_path.as_str());
                    }

                    q.change_line(job.queue_index, itm);
                }
            }
        }
    }

    fn file_exists(&self, path: QString) -> bool {
        let path = std::path::PathBuf::from(path.to_string());
        for (_id, job) in self.jobs.iter() {
            let job_path = std::path::Path::new(&job.render_options.output_path);
            if job_path == path {
                return true;
            }
        }
        false
    }

    fn get_default_encoder(&self, codec: String, gpu: bool) -> String {
        rendering::get_default_encoder(&codec, gpu)
    }
    fn get_encoder_options(&self, encoder: String) -> String {
        rendering::get_encoder_options(&encoder)
    }
}
