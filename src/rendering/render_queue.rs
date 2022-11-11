// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use qmetaobject::*;

use crate::{ core, rendering, util };
use crate::core::{ stabilization, StabilizationManager };
use std::sync::{ Arc, atomic::{ AtomicBool, AtomicUsize, Ordering::SeqCst } };
use std::cell::RefCell;
use std::collections::{ HashMap, HashSet };
use parking_lot::RwLock;
use regex::Regex;

#[derive(Default, Clone, SimpleListItem, Debug)]
pub struct RenderQueueItem {
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
    pub processing_progress: f64,

    status: JobStatus
}
impl RenderQueueItem {
    pub fn get_status(&self) -> &JobStatus { &self.status }
}

#[derive(Default, Clone, Debug, Eq, PartialEq)]
pub enum JobStatus {
    #[default]
    Queued,
    Rendering,
    Finished,
    Error
}
struct Job {
    queue_index: usize,
    render_options: RenderOptions,
    additional_data: String,
    cancel_flag: Arc<AtomicBool>,
    stab: Arc<StabilizationManager<stabilization::RGBA8>>
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RenderOptions {
    pub codec: String,
    pub codec_options: String,
    pub output_path: String,
    pub output_width: usize,
    pub output_height: usize,
    pub bitrate: f64,
    pub use_gpu: bool,
    pub audio: bool,
    pub pixel_format: String,

    // Advanced
    pub encoder_options: String,
    pub keyframe_distance: f64,
    pub preserve_other_tracks: bool,
    pub pad_with_black: bool,
}
impl RenderOptions {
    pub fn settings_string(&self, fps: f64) -> String {
        let codec_info = match self.codec.as_ref() {
            "H.264/AVC" | "H.265/HEVC" => format!("{} {:.0} Mbps", self.codec, self.bitrate),
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
            if let Some(v)  = obj.get("output_width")   .and_then(|x| x.as_u64())  { self.output_width = v as usize; }
            if let Some(v)  = obj.get("output_height")  .and_then(|x| x.as_u64())  { self.output_height = v as usize; }
            if let Some(v)  = obj.get("bitrate")        .and_then(|x| x.as_f64())  { self.bitrate = v; }
            if let Some(v) = obj.get("use_gpu")        .and_then(|x| x.as_bool()) { self.use_gpu = v; }
            if let Some(v) = obj.get("audio")          .and_then(|x| x.as_bool()) { self.audio = v; }
            if let Some(v) = obj.get("pixel_format")   .and_then(|x| x.as_str())  { self.pixel_format = v.to_string(); }

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

    pub queue: qt_property!(RefCell<SimpleListModel<RenderQueueItem>>; NOTIFY queue_changed),
    jobs: HashMap<u32, Job>,

    add: qt_method!(fn(&mut self, additional_data: String, thumbnail_url: QString) -> u32),
    remove: qt_method!(fn(&mut self, job_id: u32)),

    start: qt_method!(fn(&mut self)),
    pause: qt_method!(fn(&mut self)),
    stop: qt_method!(fn(&mut self)),

    render_job: qt_method!(fn(&mut self, job_id: u32)),
    cancel_job: qt_method!(fn(&self, job_id: u32)),
    reset_job: qt_method!(fn(&self, job_id: u32)),
    get_gyroflow_data: qt_method!(fn(&self, job_id: u32) -> QString),

    add_file: qt_method!(fn(&mut self, path: String, gyro_path: String, additional_data: String) -> u32),

    get_job_output_path: qt_method!(fn(&self, job_id: u32) -> QString),
    set_job_output_path: qt_method!(fn(&mut self, job_id: u32, new_path: String, start: bool)),

    set_pixel_format: qt_method!(fn(&mut self, job_id: u32, format: String)),
    set_error_string: qt_method!(fn(&mut self, job_id: u32, err: QString)),

    file_exists: qt_method!(fn(&self, path: QString) -> bool),

    render_queue_json: qt_method!(fn(&self) -> QString),
    restore_render_queue: qt_method!(fn(&mut self, json: String, additional_data: String)),

    main_job_id: qt_property!(u32),
    editing_job_id: qt_property!(u32; NOTIFY queue_changed),

    pub start_timestamp: qt_property!(u64; NOTIFY progress_changed),
    pub end_timestamp: qt_property!(u64; NOTIFY progress_changed),
    current_frame: qt_property!(u64; READ get_current_frame NOTIFY progress_changed),
    total_frames: qt_property!(u64; READ get_total_frames NOTIFY queue_changed),
    pub status: qt_property!(QString; NOTIFY status_changed),

    pub progress_changed: qt_signal!(),
    pub queue_changed: qt_signal!(),
    pub status_changed: qt_signal!(),

    pub render_progress: qt_signal!(job_id: u32, progress: f64, current_frame: usize, total_frames: usize, finished: bool, start_time: f64),
    pub encoder_initialized: qt_signal!(job_id: u32, encoder_name: String),

    pub convert_format: qt_signal!(job_id: u32, format: QString, supported: QString),
    pub error: qt_signal!(job_id: u32, text: QString, arg: QString, callback: QString),
    pub added: qt_signal!(job_id: u32),
    pub processing_done: qt_signal!(job_id: u32, by_preset: bool),
    pub processing_progress: qt_signal!(job_id: u32, progress: f64),

    get_encoder_options: qt_method!(fn(&self, encoder: String) -> String),
    get_default_encoder: qt_method!(fn(&self, codec: String, gpu: bool) -> String),

    apply_to_all: qt_method!(fn(&mut self, data: String, additional_data: String)),

    pause_flag: Arc<AtomicBool>,

    pub default_suffix: qt_property!(QString),

    when_done: qt_property!(i32; WRITE set_when_done),

    parallel_renders: qt_property!(i32; WRITE set_parallel_renders),
    pub export_project: qt_property!(u32),
    pub overwrite_mode: qt_property!(u32),

    pub request_close: qt_signal!(),

    pub queue_finished: qt_signal!(),

    pub jobs_added: HashSet<u32>,

    paused_timestamp: Option<u64>,

    stabilizer: Arc<StabilizationManager<stabilization::RGBA8>>,
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
    pub fn new(stabilizer: Arc<StabilizationManager<stabilization::RGBA8>>) -> Self {
        Self {
            status: QString::from("stopped"),
            default_suffix: QString::from("_stabilized"),
            stabilizer,
            ..Default::default()
        }
    }

    pub fn get_stab_for_job(&self, job_id: u32) -> Option<Arc<StabilizationManager<stabilization::RGBA8>>> {
        Some(self.jobs.get(&job_id)?.stab.clone())
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

    pub fn set_job_output_path(&mut self, job_id: u32, new_path: String, start: bool) {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.render_options.output_path = new_path.clone();
        }
        update_model!(self, job_id, itm {
            itm.output_path = QString::from(new_path);
            itm.error_string = QString::default();
            itm.status = JobStatus::Queued;
        });
        if start && self.status.to_string() != "active" {
            self.start();
        }
    }

    pub fn set_error_string(&mut self, job_id: u32, err: QString) {
        update_model!(self, job_id, itm {
            itm.error_string = err;
            itm.status = JobStatus::Error;
        });
    }

    pub fn add(&mut self, additional_data: String, thumbnail_url: QString) -> u32 {
        let job_id = if self.editing_job_id > 0 {
            self.editing_job_id
        } else {
            fastrand::u32(1..)
        };
        if self.editing_job_id > 0 {
            self.editing_job_id = 0;
            self.queue_changed();
        }

        if let Ok(obj) = serde_json::from_str(&additional_data) as serde_json::Result<serde_json::Value> {
            if let Some(out) = obj.get("output") {
                if let Ok(render_options) = serde_json::from_value(out.clone()) as serde_json::Result<RenderOptions> {
                    let project_path = self.stabilizer.input_file.read().project_file_path.clone();
                    if let Some(project_path) = project_path {
                        // Save project file on disk
                        if let Err(e) = self.stabilizer.export_gyroflow_file(&project_path, false, false, &additional_data) {
                            ::log::warn!("Failed to save project file: {}: {:?}", project_path, e);
                        }
                    }
                    let stab = self.stabilizer.get_cloned();
                    self.add_internal(job_id, Arc::new(stab), render_options, additional_data, thumbnail_url);
                }
            }
        }
        job_id
    }

    pub fn add_internal(&mut self, job_id: u32, stab: Arc<StabilizationManager<stabilization::RGBA8>>, render_options: RenderOptions, additional_data: String, thumbnail_url: QString) {
        let size = stab.params.read().video_size;
        stab.set_render_params(size, (render_options.output_width, render_options.output_height));

        let params = stab.params.read();
        let trim_ratio = params.trim_end - params.trim_start;
        let video_path = stab.input_file.read().path.clone();

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
                processing_progress: 0.0,
                error_string: QString::default(),
                status: JobStatus::Queued,
            });
        }

        self.jobs.insert(job_id, Job {
            queue_index: 0,
            render_options,
            additional_data,
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
            loop {
                if self.get_active_render_count() >= self.parallel_renders.max(1) as usize {
                    break;
                }

                let mut job_id = None;
                for v in self.queue.borrow().iter() {
                    if v.current_frame == 0 && v.total_frames > 0 && v.status == JobStatus::Queued && (v.processing_progress == 0.0 || v.processing_progress == 1.0) {
                        job_id = Some(v.job_id);
                        break;
                    }
                }
                if let Some(job_id) = job_id {
                    self.render_job(job_id);
                } else {
                    if self.get_active_render_count() == 0 {
                        self.post_render_action();
                        self.queue_finished();

                        self.start_timestamp = 0;
                        self.progress_changed();

                        self.status = QString::from("stopped");
                        self.status_changed();
                    }
                    break;
                }
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

    fn post_render_action(&self) {
        // If it was running for at least 1 minute
        if Self::current_timestamp() - self.start_timestamp > 60000 && self.when_done > 0 {
            self.request_close();

            #[cfg(not(any(target_os = "ios", target_os = "android")))]
            {
                fn system_shutdown(reboot: bool) {
                    #[cfg(target_os = "windows")]
                    {
                        let msg = util::tr("App", &format!("Gyroflow will {} the computer in 60 seconds because all tasks have been completed.", if reboot { "reboot" } else { "shut down" }));
                        let _ = if reboot {
                            system_shutdown::reboot_with_message(&msg, 60, false)
                        } else {
                            system_shutdown::shutdown_with_message(&msg, 60, false)
                        };
                    }

                    #[cfg(not(target_os = "windows"))]
                    let _ = if reboot { system_shutdown::reboot() } else { system_shutdown::shutdown() };
                }

                match self.when_done {
                    1 => { system_shutdown(false); }
                    2 => { system_shutdown(true); }
                    3 => { let _ = system_shutdown::sleep(); }
                    4 => { let _ = system_shutdown::hibernate(); }
                    5 => { let _ = system_shutdown::logout(); }
                    _ => { }
                }
            }
        }
    }

    pub fn set_when_done(&mut self, v: i32) {
        self.when_done = v;
        #[cfg(target_os = "macos")]
        if v > 0 && v != 6 {
            let _ = system_shutdown::request_permission_dialog();
        }
    }
    pub fn get_active_render_count(&self) -> usize {
        self.queue.borrow().iter().filter(|v| v.total_frames > 0 && v.status == JobStatus::Rendering).count()
    }
    pub fn get_pending_count(&self) -> usize {
        self.queue.borrow().iter().filter(|v| v.total_frames > 0 && v.status == JobStatus::Queued).count()
    }
    pub fn set_parallel_renders(&mut self, v: i32) {
        self.parallel_renders = v;

        if self.status.to_string() == "active" {
            self.start();
        }
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

    pub fn render_queue_json(&self) -> QString {
        let mut all = Vec::new();
        for v in self.queue.borrow().iter() {
            if v.total_frames > 0 && v.status != JobStatus::Finished {
                if let Ok(data) = serde_json::from_str(&self.get_gyroflow_data(v.job_id).to_string()) as serde_json::Result<serde_json::Value> {
                    all.push(data);
                }
            }
        }
        QString::from(serde_json::to_string(&all).unwrap_or_default())
    }

    pub fn restore_render_queue(&mut self, json: String, additional_data: String) {
        if let Ok(serde_json::Value::Array(val)) = serde_json::from_str(&json) as serde_json::Result<serde_json::Value> {
            for x in val {
                if let Some(project) = x.get("project_file").and_then(|x| x.as_str()) {
                    self.add_file(project.to_string(), String::new(), additional_data.clone());
                } else if let Ok(data) = serde_json::to_string(&x) {
                    self.add_file(data, String::new(), additional_data.clone());
                }
            }
        }
    }

    pub fn get_gyroflow_data(&self, job_id: u32) -> QString {
        if let Some(job) = self.jobs.get(&job_id) {
            if let Some(path) = job.stab.input_file.read().project_file_path.as_ref() {
                if std::path::Path::new(&path).exists() {
                    return QString::from(serde_json::json!({ "project_file": path }).to_string());
                }
            }
            let mut additional_data = job.additional_data.clone();
            if let Ok(serde_json::Value::Object(mut obj)) = serde_json::from_str(&additional_data) as serde_json::Result<serde_json::Value> {
                if let Ok(output) = serde_json::to_value(&job.render_options) {
                    obj.insert("output".into(), output);
                }
                additional_data = serde_json::to_string(&obj).unwrap_or_default();
            }
            if let Ok(data) = job.stab.export_gyroflow_data(true, false, &additional_data) {
                return QString::from(data);
            }
        }
        QString::default()
    }

    pub fn render_job(&mut self, job_id: u32) {
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

                let mut start_time = 0;

                update_model!(this, job_id, itm {
                    itm.current_frame = current_frame as u64;
                    itm.total_frames = total_frames as u64;
                    if itm.start_timestamp == 0 {
                        itm.start_timestamp = Self::current_timestamp();
                    }
                    start_time = itm.start_timestamp;
                    itm.end_timestamp = Self::current_timestamp();
                    if finished {
                        itm.status = JobStatus::Finished;
                    }
                });

                this.end_timestamp = Self::current_timestamp();
                this.render_progress(job_id, progress, current_frame, total_frames, finished, start_time as f64);
                this.progress_changed();

                if finished {
                    if this.get_pending_count() > 0 {
                        // Start the next one
                        this.start();
                    } else {
                        this.update_status();
                        this.post_render_action();
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
                this.render_progress(job_id, 1.0, 0, 0, true, 0.0);

                if this.get_pending_count() > 0 {
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
                this.render_progress(job_id, 1.0, 0, 0, true, 0.0);

                if this.get_pending_count() > 0 {
                    // Start the next one
                    this.start();
                }
                this.update_status();
            });
            let params = stab.params.read();
            let trim_ratio = params.trim_end - params.trim_start;
            let total_frame_count = params.frame_count;
            drop(params);
            let input_file = stab.input_file.read().clone();
            let render_options = job.render_options.clone();

            progress((0.0, 0, (total_frame_count as f64 * trim_ratio).round() as usize, false));

            job.cancel_flag.store(false, SeqCst);
            let cancel_flag = job.cancel_flag.clone();
            let pause_flag = self.pause_flag.clone();

            if self.export_project > 0 {
                let mut additional_data = job.additional_data.clone();
                if let Ok(serde_json::Value::Object(mut obj)) = serde_json::from_str(&additional_data) as serde_json::Result<serde_json::Value> {
                    if let Ok(output) = serde_json::to_value(&job.render_options) {
                        obj.insert("output".into(), output);
                    }
                    additional_data = serde_json::to_string(&obj).unwrap_or_default();
                }
                let path = std::path::Path::new(&render_options.output_path.replace(&self.default_suffix.to_string(), "")).with_extension("gyroflow");
                let result = match self.export_project {
                    1 => job.stab.export_gyroflow_file(&path, true, false, &additional_data),
                    2 => job.stab.export_gyroflow_file(&path, false, false, &additional_data),
                    3 => job.stab.export_gyroflow_file(&path, false, true, &additional_data),
                    _ => { Err(std::io::Error::new(std::io::ErrorKind::Other, "Unknown option")) }
                };
                if let Err(e) = result {
                    err((e.to_string(), String::new()));
                } else {
                    progress((1.0, 1, 1, true));
                }
                return;
            }

            core::run_threaded(move || {
                let mut i = 0;
                loop {
                    let result = rendering::render(stab.clone(), progress.clone(), &input_file, &render_options, i, cancel_flag.clone(), pause_flag.clone(), encoder_initialized.clone());
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

    fn get_output_path(suffix: &str, path: &str, codec: &str, ui_output_path: &str) -> String {
        use std::path::Path;

        let mut path = Path::new(path).with_extension("");

        if !ui_output_path.is_empty() {
            // Prefer output path of the currently opened file
            let org_filename = path.file_name().map(|x| x.to_owned()).unwrap_or_default();
            path = Path::new(ui_output_path).to_path_buf();
            if path.is_dir() || ui_output_path.ends_with('/') || ui_output_path.ends_with('\\') {
                path.push(&org_filename);
            } else {
                path = path.with_file_name(&org_filename);
            }
        }

        let ext = match codec {
            "ProRes"        => ".mov",
            "DNxHD"         => ".mov",
            "EXR Sequence"  => "_%05d.exr",
            "PNG Sequence"  => "_%05d.png",
            _ => ".mp4"
        };

        path.set_file_name(format!("{}{}{}", path.file_name().map(|v| v.to_string_lossy()).unwrap_or_default(), suffix, ext));

        path.to_string_lossy().replace('\\', "/")
    }

    pub fn add_file(&mut self, path: String, gyro_path: String, additional_data: String) -> u32 {
        let job_id = fastrand::u32(1..);

        let is_gf_data = path.starts_with('{');

        let err = util::qt_queued_callback_mut(self, move |this, (msg, arg): (String, String)| {
            ::log::warn!("[add_file]: {}", arg);
            update_model!(this, job_id, itm {
                itm.error_string = QString::from(arg.clone());
                itm.status = JobStatus::Error;
            });
            this.error(job_id, QString::from(msg), QString::from(arg), QString::default());
        });
        let processing = util::qt_queued_callback_mut(self, move |this, progress: f64| {
            update_model!(this, job_id, itm {
                itm.processing_progress = progress;
            });
            this.processing_progress(job_id, progress);
        });
        let processing_done = util::qt_queued_callback_mut(self, move |this, _: ()| {
            this.processing_done(job_id, false);
        });

        let suffix = self.default_suffix.to_string();

        let stabilizer = self.stabilizer.clone();

        let additional_data2 = additional_data.clone();
        let additional_data3 = additional_data.clone();
        if let Ok(additional_data) = serde_json::from_str(&additional_data) as serde_json::Result<serde_json::Value> {
            let mut sync_options = serde_json::Value::default();
            if let Some(sync) = additional_data.get("synchronization") {
                sync_options = sync.clone();
            }
            if let Some(out) = additional_data.get("output") {
                if let Ok(mut render_options) = serde_json::from_value(out.clone()) as serde_json::Result<RenderOptions> {
                    let (smoothing_name, smoothing_params) = {
                        let smoothing_lock = stabilizer.smoothing.read();
                        let smoothing = smoothing_lock.current();
                        (smoothing.get_name(), smoothing.get_parameters_json())
                    };
                    let params = stabilizer.params.read();

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
                        input_file: Arc::new(RwLock::new(gyroflow_core::InputFile { path: path.clone(), project_file_path: None, image_sequence_start: 0, image_sequence_fps: 0.0 })),
                        lens_profile_db: stabilizer.lens_profile_db.clone(),
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
                    let loaded = util::qt_queued_callback_mut(self, move |this, (render_options, ask_path): (RenderOptions, bool)| {
                        let out_path = render_options.output_path.clone();
                        this.add_internal(job_id, stab2.clone(), render_options, additional_data2.clone(), QString::default());

                        if ask_path && std::path::Path::new(&out_path).exists() {
                            let msg = QString::from(format!("file_exists:{}", out_path));
                            update_model!(this, job_id, itm {
                                itm.error_string = msg.clone();
                                itm.status = JobStatus::Error;
                            });
                            this.error(job_id, msg, QString::default(), QString::default());
                        }
                    });
                    let thumb_fetched = util::qt_queued_callback_mut(self, move |this, thumb: QString| {
                        update_model!(this, job_id, itm { itm.thumbnail_url = thumb; });
                    });
                    let apply_preset = util::qt_queued_callback_mut(self, move |this, preset: String| {
                        this.apply_to_all(preset, additional_data3.clone());
                        this.added(job_id);
                    });

                    core::run_threaded(move || {
                        let fetch_thumb = |video_path: &str, ratio: f64| -> Result<(), rendering::FFmpegError> {
                            let mut fetched = false;
                            if !crate::cli::will_run_in_console() { // Don't fetch thumbs in the CLI
                                let mut proc = rendering::VideoProcessor::from_file(video_path, false, 0, None)?;
                                proc.on_frame(move |_timestamp_us, input_frame, _output_frame, converter, _rate_control| {
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

                        if is_gf_data || path.ends_with(".gyroflow") {
                            if !is_gf_data {
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
                            }

                            let result = if is_gf_data {
                                stab.import_gyroflow_data(path.as_bytes(), true, None, |_|(), Arc::new(AtomicBool::new(false)))
                            } else {
                                stab.import_gyroflow_file(&path, true, |_|(), Arc::new(AtomicBool::new(false)))
                            };

                            match result {
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
                                    processing_done(());
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
                            render_options.output_path = Self::get_output_path(&suffix, &path, &render_options.codec, &render_options.output_path);

                            let ratio = info.width as f64 / info.height as f64;

                            if info.duration_ms > 0.0 && info.fps > 0.0 {

                                let video_size = (info.width as usize, info.height as usize);

                                if let Err(e) = stab.init_from_video_data(&path, info.duration_ms, info.fps, info.frame_count, video_size) {
                                    err(("An error occured: %1".to_string(), e.to_string()));
                                    return;
                                }
                                let gyro_path = if !gyro_path.is_empty() { &gyro_path } else { &path };
                                let _ = stab.load_gyro_data(&gyro_path, |_|(), Arc::new(AtomicBool::new(false)));

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

                                stab.recompute_blocking();

                                // println!("{}", stab.export_gyroflow_data(true, serde_json::to_string(&render_options).unwrap_or_default()));

                                loaded((render_options, true));

                                if let Err(e) = fetch_thumb(&path, ratio) {
                                    err(("An error occured: %1".to_string(), e.to_string()));
                                }

                                Self::do_autosync(&path, info.duration_ms, stab.clone(), processing, err.clone(), sync_options);

                                processing_done(());
                            }
                        } else {
                            err(("An error occured: %1".to_string(), "Unable to read the video file.".to_string()));
                        }
                    });
                }
            }
        }
        self.jobs_added.insert(job_id);

        job_id
    }

    fn do_autosync<F: Fn(f64) + Send + Sync + Clone + 'static, F2: Fn((String, String)) + Send + Sync + Clone + 'static>(path: &str, duration_ms: f64, stab: Arc<StabilizationManager<stabilization::RGBA8>>, processing_cb: F, err: F2, sync_options: serde_json::Value) {
        let (has_gyro, has_sync_points) = {
            let gyro = stab.gyro.read();
            (!gyro.quaternions.is_empty(), !gyro.get_offsets().is_empty())
        };

        let sync_settings = stab.lens.read().sync_settings.clone();
        if let Some(sync_settings) = sync_settings {
            if has_gyro && !has_sync_points && sync_settings.get("do_autosync").and_then(|v| v.as_bool()).unwrap_or_default() {
                // ----------------------------------------------------------------------------
                // --------------------------------- Autosync ---------------------------------
                processing_cb(0.01);
                use gyroflow_core::synchronization::AutosyncProcess;
                use gyroflow_core::synchronization;
                use crate::rendering::VideoProcessor;
                use itertools::Either;

                if let serde_json::Value::Object(mut sync_options) = sync_options {
                    for (k, v) in sync_settings.as_object().unwrap() {
                        sync_options.insert(k.clone(), v.clone());
                    }

                    if let Some(points) = sync_options.get("max_sync_points").and_then(|v| v.as_i64()) {
                        let chunks = 1.0 / points as f64;
                        let start = chunks / 2.0;
                        let mut timestamps_fract: Vec<f64> = (0..points).map(|i| start + (i as f64 * chunks)).collect();
                        if let Some(v) = sync_options.get("custom_sync_timestamps").and_then(|v| v.as_array()) {
                            timestamps_fract = v.iter().filter_map(|v| v.as_f64()).filter(|v| *v <= duration_ms).map(|v| v / duration_ms).collect();
                        }

                        if let Ok(mut sync_params) = serde_json::from_value(serde_json::Value::Object(sync_options)) as serde_json::Result<synchronization::SyncParams> {

                            let cancel_flag = Arc::new(AtomicBool::new(false));
                            sync_params.initial_offset     *= 1000.0; // s to ms
                            sync_params.time_per_syncpoint *= 1000.0; // s to ms
                            sync_params.search_size        *= 1000.0; // s to ms

                            let every_nth_frame = sync_params.every_nth_frame.max(1);

                            let size = stab.params.read().video_size;

                            if let Ok(mut sync) = AutosyncProcess::from_manager(&stab, &timestamps_fract, sync_params, "synchronize".into(), cancel_flag.clone()) {
                                let processing_cb2 = processing_cb.clone();
                                sync.on_progress(move |percent, _ready, _total| {
                                    processing_cb2(percent);
                                });
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

                                let (sw, sh) = ((720.0 * (size.0 as f64 / size.1 as f64)).round() as u32, 720);

                                let gpu_decoding = *rendering::GPU_DECODING.read();

                                let mut frame_no = 0;
                                let mut abs_frame_no = 0;
                                let sync = std::rc::Rc::new(sync);

                                match VideoProcessor::from_file(path, gpu_decoding, 0, None) {
                                    Ok(mut proc) => {
                                        let err2 = err.clone();
                                        let sync2 = sync.clone();
                                        proc.on_frame(move |timestamp_us, input_frame, _output_frame, converter, _rate_control| {
                                            if abs_frame_no % every_nth_frame == 0 {
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
                                        if let Err(e) = proc.start_decoder_only(sync.get_ranges(), cancel_flag) {
                                            err(("An error occured: %1".to_string(), e.to_string()));
                                        }
                                        sync.finished_feeding_frames();
                                    }
                                    Err(error) => {
                                        err(("An error occured: %1".to_string(), error.to_string()));
                                    }
                                }
                            } else {
                                err(("An error occured: %1".to_string(), "Invalid parameters".to_string()));
                            }

                            stab.recompute_blocking();
                        }
                    }
                }
                processing_cb(1.0);
                // --------------------------------- Autosync ---------------------------------
                // ----------------------------------------------------------------------------
            }
        }
    }

    pub fn apply_to_all(&mut self, data: String, additional_data: String) {
        ::log::debug!("Applying preset {}", &data);
        let mut new_output_options = None;
        if let Ok(obj) = serde_json::from_str(&data) as serde_json::Result<serde_json::Value> {
            if let Some(output) = obj.get("output") {
                new_output_options = Some(output.clone());
            }
        }
        let processing = util::qt_queued_callback_mut(self, |this, (progress, job_id): (f64, u32)| {
            update_model!(this, job_id, itm {
                itm.processing_progress = progress;
            });
            this.processing_progress(job_id, progress);
        });
        let processing_done = util::qt_queued_callback_mut(self, |this, job_id: u32| {
            this.processing_done(job_id, true);
        });
        let err = util::qt_queued_callback_mut(self, move |this, (job_id, msg): (u32, String)| {
            this.error(job_id, QString::from(msg), QString::default(), QString::default());
        });
        ::log::debug!("new_output_options: {:?}", &new_output_options);
        let data = data.as_bytes();
        let mut q = self.queue.borrow_mut();
        for (job_id, job) in self.jobs.iter_mut() {
            if job.queue_index < q.row_count() as usize {
                let mut itm = q[job.queue_index].clone();
                if itm.status == JobStatus::Queued {
                    let stab = job.stab.clone();
                    let data_vec = data.to_vec();
                    let processing2 = processing.clone();
                    let mut sync_options = serde_json::Value::default();
                    if let Ok(additional_data) = serde_json::from_str(&additional_data) as serde_json::Result<serde_json::Value> {
                        if let Some(sync) = additional_data.get("synchronization") {
                            sync_options = sync.clone();
                        }
                    }
                    let job_id = *job_id;
                    if let Some(ref new_output_options) = new_output_options {
                        job.render_options.update_from_json(new_output_options);
                        job.render_options.output_path = Self::get_output_path(&self.default_suffix.to_string(), &itm.input_file.to_string(), &job.render_options.codec, &job.render_options.output_path);
                        itm.export_settings = QString::from(job.render_options.settings_string(job.stab.params.read().fps));
                        itm.output_path = QString::from(job.render_options.output_path.as_str());
                        if std::path::Path::new(&job.render_options.output_path).exists() {
                            let msg = QString::from(format!("file_exists:{}", job.render_options.output_path));
                            itm.error_string = msg.clone();
                            itm.status = JobStatus::Error;
                            err((job_id, msg.to_string()));
                        }
                    }
                    let processing_done = processing_done.clone();
                    core::run_threaded(move || {
                        if let Err(e) = stab.import_gyroflow_data(&data_vec, true, None, |_|(), Arc::new(AtomicBool::new(false))) {
                            ::log::error!("Failed to update queue stab data: {:?}", e);
                        }
                        let (path, duration_ms, ) = {
                            let params = stab.params.read();
                            (stab.input_file.read().path.clone(), params.duration_ms)
                        };
                        Self::do_autosync(&path, duration_ms, stab, move |progress| processing2((progress, job_id)) , |_|{}, sync_options);
                        processing_done(job_id);
                    });

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
