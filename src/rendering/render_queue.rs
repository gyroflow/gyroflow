// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use qmetaobject::*;

use crate::{ core, rendering, util };
use crate::core::StabilizationManager;
use std::sync::{ Arc, atomic::{ AtomicBool, AtomicUsize, Ordering::SeqCst } };
use std::cell::RefCell;
use std::collections::{ HashMap, HashSet };
use parking_lot::RwLock;
use regex::Regex;

#[derive(Default, Clone, SimpleListItem, Debug)]
pub struct RenderQueueItem {
    pub job_id: u32,
    pub input_file: QString,
    pub input_filename: QString,
    pub output_filename: QString,
    pub output_folder: QString,
    pub display_output_path: QString,
    pub export_settings: QString,
    pub thumbnail_url: QString,
    pub current_frame: u64,
    pub start_timestamp_frame: u64,
    pub total_frames: u64,
    pub start_timestamp: u64,
    pub start_timestamp2: u64,
    pub end_timestamp: u64,
    pub error_string: QString,
    pub processing_progress: f64,

    frame_times: std::collections::VecDeque<(u64, u64)>,

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
    project_data: Option<String>,
    stab: Arc<StabilizationManager>
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RenderMetadata {
    pub comment: String,
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RenderOptions {
    pub codec: String,
    pub codec_options: String,
    pub output_folder: String,
    pub output_filename: String,
    pub output_width: usize,
    pub output_height: usize,
    pub input_filename: String,
    pub bitrate: f64,
    pub use_gpu: bool,
    pub audio: bool,
    pub pixel_format: String,

    // Advanced
    pub encoder_options: String,
    pub metadata: RenderMetadata,
    pub keyframe_distance: f64,
    pub preserve_other_tracks: bool,
    pub pad_with_black: bool,
    pub audio_codec: String,
}
impl RenderOptions {
    pub fn settings_string(&self, fps: f64) -> String {
        let codec_info = match self.codec.as_ref() {
            "H.264/AVC" | "H.265/HEVC" | "AV1" => format!("{} {:.0} Mbps", self.codec, self.bitrate),
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
    pub fn get_metadata_dict(&self) -> ffmpeg_next::Dictionary {
        let mut metadata = ffmpeg_next::Dictionary::new();
        metadata.set("comment", format!("Original filename: {}\n{}", self.input_filename, self.metadata.comment).trim());
        metadata
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
            if let Some(v) = obj.get("audio_codec")          .and_then(|x| x.as_str())  { self.audio_codec = v.to_string(); }

            if let Some(v) = obj.get("metadata").and_then(|x| x.as_object())  {
                if let Some(s) = v.get("comment").and_then(|x| x.as_str()) { self.metadata.comment = s.to_string(); }
            }

            // Backwards compatibility
            if let Some(v) = obj.get("output_path").and_then(|x| x.as_str()) {
                let url = core::filesystem::path_to_url(v);
                let folder = core::filesystem::get_folder(&url);
                if !folder.is_empty() {
                    self.output_folder = folder;
                }
                let filename = core::filesystem::get_filename(&url);
                if !filename.is_empty() {
                    self.output_filename = filename;
                }
            }
            if let Some(v) = obj.get("output_folder").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) {
                self.output_folder = v.to_string();
            }
            if let Some(v) = obj.get("output_filename").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) {
                self.output_filename = v.to_string();
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
    clear: qt_method!(fn(&mut self)),

    start: qt_method!(fn(&mut self)),
    pause: qt_method!(fn(&mut self)),
    stop: qt_method!(fn(&mut self)),

    render_job: qt_method!(fn(&mut self, job_id: u32)),
    cancel_job: qt_method!(fn(&self, job_id: u32)),
    reset_job: qt_method!(fn(&self, job_id: u32)),
    get_gyroflow_data: qt_method!(fn(&self, job_id: u32) -> QString),

    add_file: qt_method!(fn(&mut self, url: String, gyro_url: String, additional_data: String) -> u32),

    get_job_output_filename: qt_method!(fn(&self, job_id: u32) -> QString),
    get_job_output_folder: qt_method!(fn(&self, job_id: u32) -> QUrl),
    set_job_output_filename: qt_method!(fn(&mut self, job_id: u32, new_filename: QString, start: bool)),

    set_pixel_format: qt_method!(fn(&mut self, job_id: u32, format: String)),
    set_error_string: qt_method!(fn(&mut self, job_id: u32, err: QString)),
    set_processing_resolution: qt_method!(fn(&mut self, target_height: i32)),

    file_exists_in_folder: qt_method!(fn(&self, folder: QUrl, filename: QString) -> bool),

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

    pub render_progress: qt_signal!(job_id: u32, progress: f64, current_frame: usize, total_frames: usize, finished: bool, start_time: f64, is_conversion: bool),
    pub encoder_initialized: qt_signal!(job_id: u32, encoder_name: String),

    pub convert_format: qt_signal!(job_id: u32, format: QString, supported: QString),
    pub error: qt_signal!(job_id: u32, text: QString, arg: QString, callback: QString),
    pub added: qt_signal!(job_id: u32),
    pub processing_done: qt_signal!(job_id: u32, by_preset: bool),
    pub processing_progress: qt_signal!(job_id: u32, progress: f64),

    get_encoder_options: qt_method!(fn(&self, encoder: String) -> String),
    get_default_encoder: qt_method!(fn(&self, codec: String, gpu: bool) -> String),

    apply_to_all: qt_method!(fn(&mut self, data: String, additional_data: String, to_job_id: u32)),

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
    start_frame: u64,

    stabilizer: Arc<StabilizationManager>,

    processing_resolution: i32,
}

macro_rules! update_model {
    ($this:ident, $job_id:ident, $itm:ident $action:block) => {
        {
            if let Ok(mut q) = $this.queue.try_borrow_mut() {
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
        }
    };
}

impl RenderQueue {
    pub fn new(stabilizer: Arc<StabilizationManager>) -> Self {
        Self {
            status: QString::from("stopped"),
            default_suffix: QString::from("_stabilized"),
            processing_resolution: 720,
            stabilizer,
            ..Default::default()
        }
    }

    pub fn set_processing_resolution(&mut self, target_height: i32) {
        self.processing_resolution = target_height;
    }
    pub fn get_stab_for_job(&self, job_id: u32) -> Option<Arc<StabilizationManager>> {
        Some(self.jobs.get(&job_id)?.stab.clone())
    }

    pub fn get_total_frames(&self) -> u64 {
        self.queue.borrow().iter().map(|v| v.total_frames).sum::<u64>() - self.start_frame
    }
    pub fn get_current_frame(&self) -> u64 {
        self.queue.borrow().iter().map(|v| v.current_frame).sum::<u64>() - self.start_frame
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

    pub fn set_job_output_filename(&mut self, job_id: u32, new_filename: QString, start: bool) {
        if let Some(job) = self.jobs.get_mut(&job_id) {
            job.render_options.output_filename = new_filename.to_string();
            job.project_data = Self::get_gyroflow_data_internal(&job.stab, &job.additional_data, &job.render_options);
        }
        update_model!(self, job_id, itm {
            itm.output_filename = new_filename;
            itm.display_output_path = QString::from(core::filesystem::display_folder_filename(&itm.output_folder.to_string(), &itm.output_filename.to_string()));
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
                if let Ok(mut render_options) = serde_json::from_value(out.clone()) as serde_json::Result<RenderOptions> {
                    render_options.update_from_json(out);
                    let project_url = self.stabilizer.input_file.read().project_file_url.clone();
                    if let Some(project_url) = project_url {
                        // Save project file on disk
                        if let Err(e) = self.stabilizer.export_gyroflow_file(&project_url, core::GyroflowProjectType::WithGyroData, &additional_data) {
                            ::log::warn!("Failed to save project file: {}: {:?}", project_url, e);
                        }
                    }
                    let stab = self.stabilizer.get_cloned();

                    // If it's added from main UI, never do the additional autosync
                    if let Some(ref mut obj) = stab.lens.write().sync_settings { obj.as_object_mut().and_then(|x| x.remove("do_autosync")); }

                    self.add_internal(job_id, Arc::new(stab), render_options, additional_data, thumbnail_url);
                }
            }
        }
        job_id
    }

    pub fn add_internal(&mut self, job_id: u32, stab: Arc<StabilizationManager>, mut render_options: RenderOptions, additional_data: String, thumbnail_url: QString) {
        let size = stab.params.read().video_size;
        stab.set_render_params(size, (render_options.output_width, render_options.output_height));

        let params = stab.params.read();
        let trim_ratio = params.trim_end - params.trim_start;
        let video_url = stab.input_file.read().url.clone();

        let editing = self.jobs.contains_key(&job_id);

        if editing {
            update_model!(self, job_id, itm {
                itm.output_folder = QString::from(render_options.output_folder.as_str());
                itm.output_filename = QString::from(render_options.output_filename.as_str());
                itm.display_output_path = QString::from(core::filesystem::display_folder_filename(render_options.output_folder.as_str(), render_options.output_filename.as_str()));
                itm.export_settings = QString::from(render_options.settings_string(params.fps));
                itm.thumbnail_url = thumbnail_url;
                itm.current_frame = 0;
                itm.total_frames = (params.frame_count as f64 * trim_ratio).ceil() as u64;
                itm.start_timestamp = 0;
                itm.start_timestamp2 = 0;
                itm.start_timestamp_frame = 0;
                itm.end_timestamp = 0;
                itm.error_string = QString::default();
                itm.status = JobStatus::Queued;
                itm.frame_times.clear();
            });
        } else {
            let mut q = self.queue.borrow_mut();
            q.push(RenderQueueItem {
                job_id,
                input_file: QString::from(video_url.as_str()),
                input_filename: QString::from(core::filesystem::get_filename(&video_url)),
                output_folder: QString::from(render_options.output_folder.as_str()),
                output_filename: QString::from(render_options.output_filename.as_str()),
                display_output_path: QString::from(core::filesystem::display_folder_filename(render_options.output_folder.as_str(), render_options.output_filename.as_str())),
                export_settings: QString::from(render_options.settings_string(params.fps)),
                thumbnail_url,
                current_frame: 0,
                total_frames: (params.frame_count as f64 * trim_ratio).ceil() as u64,
                start_timestamp: 0,
                start_timestamp2: 0,
                start_timestamp_frame: 0,
                end_timestamp: 0,
                processing_progress: 0.0,
                error_string: QString::default(),
                frame_times: Default::default(),
                status: JobStatus::Queued,
            });
        }

        let project_data = Self::get_gyroflow_data_internal(&stab, &additional_data, &render_options);

        render_options.input_filename = core::filesystem::get_filename(&stab.input_file.read().url);

        self.jobs.insert(job_id, Job {
            queue_index: 0,
            render_options,
            additional_data,
            cancel_flag: Default::default(),
            project_data,
            stab: stab.clone()
        });
        self.update_queue_indices();

        self.queue_changed();
        self.added(job_id);
    }

    pub fn get_job_output_folder(&self, job_id: u32) -> QUrl {
        let q = self.queue.borrow();
        if let Some(job) = self.jobs.get(&job_id) {
            if job.queue_index < q.row_count() as usize {
                return QUrl::from(q[job.queue_index].output_folder.clone());
            }
        }
        QUrl::default()
    }
    pub fn get_job_output_filename(&self, job_id: u32) -> QString {
        let q = self.queue.borrow();
        if let Some(job) = self.jobs.get(&job_id) {
            if job.queue_index < q.row_count() as usize {
                return q[job.queue_index].output_filename.clone();
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
    pub fn clear(&mut self) {
        let mut to_delete = Vec::new();
        for v in self.queue.borrow().iter() {
            if v.status != JobStatus::Rendering {
                to_delete.push(v.job_id);
            }
        }
        for job_id in to_delete {
            self.remove(job_id);
        }
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
            self.start_frame = 0;
            self.start_timestamp = Self::current_timestamp();
            self.start_frame = self.get_current_frame();
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

                        self.start_frame = 0;
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
                    #[allow(unused_mut)]
                    let mut project = project.to_string();
                    #[cfg(any(target_os = "macos", target_os = "ios"))]
                    if let Some(bookmark) = x.get("project_file_bookmark").and_then(|x| x.as_str()).filter(|x| !x.is_empty()) {
                        let (resolved, _is_stale) = core::filesystem::apple::resolve_bookmark(bookmark, None);
                        if !resolved.is_empty() { project = resolved; }
                    }
                    self.add_file(project, String::new(), additional_data.clone());
                } else if let Ok(data) = serde_json::to_string(&x) {
                    self.add_file(data, String::new(), additional_data.clone());
                }
            }
        }
    }

    fn get_gyroflow_data_internal(stab: &StabilizationManager, additional_data: &str, render_options: &RenderOptions) -> Option<String> {
        if let Some(url) = stab.input_file.read().project_file_url.as_ref() {
            if core::filesystem::exists(url) {
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                {
                    return Some(serde_json::json!({ "project_file": url, "project_file_bookmark": core::filesystem::apple::create_bookmark(&url, false, None) }).to_string());
                }
                #[cfg(not(any(target_os = "macos", target_os = "ios")))]
                {
                    return Some(serde_json::json!({ "project_file": url }).to_string());
                }
            }
        }
        let mut additional_data = additional_data.to_owned();
        if let Ok(serde_json::Value::Object(mut obj)) = serde_json::from_str(&additional_data) as serde_json::Result<serde_json::Value> {
            if let Ok(output) = serde_json::to_value(&render_options) {
                obj.insert("output".into(), output);
            }
            additional_data = serde_json::to_string(&obj).unwrap_or_default();
        }
        if let Ok(data) = stab.export_gyroflow_data(core::GyroflowProjectType::Simple, &additional_data, None) {
            return Some(data);
        }
        None
    }

    pub fn get_gyroflow_data(&self, job_id: u32) -> QString {
        if let Some(job) = self.jobs.get(&job_id) {
            job.project_data.clone().map(QString::from).unwrap_or_default()
        } else {
            QString::default()
        }
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

            let stab = job.stab.clone();

            rendering::clear_log();

            let rendered_frames = Arc::new(AtomicUsize::new(0));
            let rendered_frames2 = rendered_frames.clone();
            let progress = util::qt_queued_callback_mut(self, move |this, (progress, current_frame, total_frames, finished, is_conversion): (f64, usize, usize, bool, bool)| {
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
                    itm.frame_times.push_back((itm.current_frame, itm.end_timestamp));
                    if itm.end_timestamp - itm.start_timestamp > 10000 { // 10s average
                        if let Some(el) = itm.frame_times.pop_front() {
                            itm.start_timestamp_frame = el.0;
                            itm.start_timestamp2 = el.1;
                        }
                    }
                    if finished {
                        itm.status = JobStatus::Finished;
                    }
                });

                this.end_timestamp = Self::current_timestamp();
                this.render_progress(job_id, progress, current_frame, total_frames, finished, start_time as f64, is_conversion);
                this.progress_changed();

                let is_queue_active = this.status == "active".into();
                if finished {
                    if this.get_pending_count() > 0 && is_queue_active {
                        // Start the next one
                        this.start();
                    } else {
                        this.start_timestamp = 0;
                        this.start_frame = 0;
                        this.update_status();
                        if is_queue_active {
                            this.post_render_action();
                        }
                    }
                }
            });
            let processing = util::qt_queued_callback_mut(self, move |this, progress: f64| {
                update_model!(this, job_id, itm {
                    itm.processing_progress = progress;
                });
                this.processing_progress(job_id, progress);
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
                this.render_progress(job_id, 1.0, 0, 0, true, 0.0, false);

                if this.get_pending_count() > 0 {
                    // Start the next one
                    this.start();
                } else {
                    this.start_timestamp = 0;
                    this.start_frame = 0;
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
                this.render_progress(job_id, 1.0, 0, 0, true, 0.0, false);

                if this.get_pending_count() > 0 {
                    // Start the next one
                    this.start();
                } else {
                    this.start_timestamp = 0;
                    this.start_frame = 0;
                }
                this.update_status();
            });
            let params = stab.params.read();
            let trim_ratio = params.trim_end - params.trim_start;
            let total_frame_count = params.frame_count;
            drop(params);
            let mut input_file = stab.input_file.read().clone();
            let filename = core::filesystem::get_filename(&input_file.url);
            let render_options = job.render_options.clone();

            progress((0.0, 0, (total_frame_count as f64 * trim_ratio).round() as usize, false, false));

            job.cancel_flag.store(false, SeqCst);
            let cancel_flag = job.cancel_flag.clone();
            let pause_flag = self.pause_flag.clone();
            let export_project = self.export_project;
            let default_suffix = self.default_suffix.to_string();
            let mut additional_data = job.additional_data.clone();
            let proc_height = self.processing_resolution;
            let err2 = err.clone();

            core::run_threaded(move || {
                Self::do_autosync(stab.clone(), processing, err2, proc_height);

                if export_project > 0 {
                    if let Ok(serde_json::Value::Object(mut obj)) = serde_json::from_str(&additional_data) as serde_json::Result<serde_json::Value> {
                        if let Ok(output) = serde_json::to_value(&render_options) {
                            obj.insert("output".into(), output);
                        }
                        additional_data = serde_json::to_string(&obj).unwrap_or_default();
                    }
                    let gf_folder = render_options.output_folder.to_owned();
                    let gf_file = core::filesystem::filename_with_extension(&render_options.output_filename.replace(&default_suffix, ""), "gyroflow");
                    let gf_url = core::filesystem::get_file_url(&gf_folder, &gf_file, true);
                    let result = match export_project {
                        1 => stab.export_gyroflow_file(&gf_url, core::GyroflowProjectType::Simple, &additional_data),
                        2 => stab.export_gyroflow_file(&gf_url, core::GyroflowProjectType::WithGyroData, &additional_data),
                        3 => stab.export_gyroflow_file(&gf_url, core::GyroflowProjectType::WithProcessedData, &additional_data),
                        4 => stab.export_gyroflow_file(&gf_url, core::GyroflowProjectType::WithGyroData, &additional_data),
                        _ => { Err(gyroflow_core::GyroflowCoreError::Unknown) }
                    };
                    if export_project != 4 {
                        if let Err(e) = result {
                            err((e.to_string(), String::new()));
                        } else {
                            progress((1.0, 1, 1, true, false));
                        }
                        return;
                    }
                }

                // Assumes regular filesystem
                if filename.to_ascii_lowercase().ends_with(".r3d") {
                    let mov_url = core::filesystem::get_file_url(&core::filesystem::get_folder(&input_file.url), &core::filesystem::filename_with_extension(&core::filesystem::get_filename(&input_file.url), "mov"), false);
                    if core::filesystem::exists(&mov_url) {
                        input_file.url = mov_url.clone();
                    } else {
                        let in_file = input_file.url.clone();

                        let mut frame = 0;
                        let r3d_progress = |(percent, error_str, out_url): (f64, String, String)| {
                            if !error_str.is_empty() {
                                err(("An error occured: %1".to_string(), error_str));
                            } else {
                                progress((percent * 0.98, frame, total_frame_count + 1, false, true));
                                input_file.url = out_url;
                                frame += 1;
                            }
                        };
                        let format = crate::util::get_setting("r3dConvertFormat").parse::<i32>().unwrap_or(0);
                        let force_primary = crate::util::get_setting("r3dColorMode").parse::<i32>().unwrap_or(0);

                        let gamma_curves = [-1, 1, 2, 3, 4, 5, 6, 14, 15, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37];
                        let color_spaces = [2, 0, 1, 14, 15, 5, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27];
                        let gamma = gamma_curves[crate::util::get_setting("r3dGammaCurve").parse::<usize>().unwrap_or(7)];
                        let space = color_spaces[crate::util::get_setting("r3dColorSpace").parse::<usize>().unwrap_or(0)];
                        let additional_params = crate::util::get_setting("r3dRedlineParams");
                        crate::external_sdk::r3d::REDSdk::convert_r3d(&in_file, format, force_primary > 0, gamma, space, &additional_params, r3d_progress, cancel_flag.clone());
                        if cancel_flag.load(SeqCst) {
                            std::thread::sleep(std::time::Duration::from_secs(2));
                            let _ = core::filesystem::remove_file(&mov_url);
                            err(("Conversion cancelled%1".to_string(), "".to_string()));
                            return;
                        }
                    }
                }

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

    fn get_output_folder(input_url: &str, ui_output_folder: &str) -> String {
        if !ui_output_folder.is_empty() {
            return ui_output_folder.to_owned();
        }
        core::filesystem::get_folder(input_url)
    }
    fn get_output_filename(input_url: &str, suffix: &str, render_options: &RenderOptions, override_ext: Option<&str>) -> String {
        let mut filename = core::filesystem::get_filename(input_url);

        let mut ext = override_ext.unwrap_or(match render_options.codec.as_ref() {
            "ProRes"        => ".mov",
            "DNxHD"         => ".mov",
            "CineForm"      => ".mov",
            "EXR Sequence"  => "_%05d.exr",
            "PNG Sequence"  => "_%05d.png",
            _ => ".mp4"
        });
        if ext == ".mp4" && render_options.preserve_other_tracks {
            ext = ".mov";
        }
        if let Some(pos) = filename.rfind('.') {
            filename = filename[..pos].to_owned();
        }

        format!("{filename}{suffix}{ext}")
    }

    pub fn add_file(&mut self, url: String, gyro_url: String, additional_data: String) -> u32 {
        let job_id = fastrand::u32(1..);

        let is_gf_data = url.starts_with('{');

        let err = util::qt_queued_callback_mut(self, move |this, (msg, arg): (String, String)| {
            ::log::warn!("[add_file]: {}", arg);
            update_model!(this, job_id, itm {
                itm.error_string = QString::from(arg.clone());
                itm.status = JobStatus::Error;
            });
            this.error(job_id, QString::from(msg), QString::from(arg), QString::default());
        });
        let processing_done = util::qt_queued_callback_mut(self, move |this, _: ()| {
            if let Some(job) = this.jobs.get(&job_id) {
                if core::filesystem::exists_in_folder(&job.render_options.output_folder, &job.render_options.output_filename.replace("_%05d", "_00001")) {
                    let msg = QString::from(format!("file_exists:{}", serde_json::json!({ "filename": job.render_options.output_filename, "folder": job.render_options.output_folder })));
                    update_model!(this, job_id, itm {
                        itm.error_string = msg.clone();
                        itm.status = JobStatus::Error;
                    });
                    this.error(job_id, msg, QString::default(), QString::default());
                }
            }

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
                let override_ext = out.get("output_extension").and_then(|x| x.as_str()).map(|x| x.to_owned());
                if let Ok(mut render_options) = serde_json::from_value(out.clone()) as serde_json::Result<RenderOptions> {
                    render_options.update_from_json(out);
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
                            current_device:            params.current_device,
                            video_speed:               params.video_speed,
                            video_speed_affects_smoothing: params.video_speed_affects_smoothing,
                            video_speed_affects_zooming:   params.video_speed_affects_zooming,
                            of_method:                 params.of_method,
                            ..Default::default()
                        })),
                        input_file: Arc::new(RwLock::new(gyroflow_core::InputFile { url: if is_gf_data { String::new() } else { url.clone() }, project_file_url: None, image_sequence_start: 0, image_sequence_fps: 0.0 })),
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
                    let loaded = util::qt_queued_callback_mut(self, move |this, render_options: RenderOptions| {
                        this.add_internal(job_id, stab2.clone(), render_options, additional_data2.clone(), QString::default());
                    });
                    let thumb_fetched = util::qt_queued_callback_mut(self, move |this, thumb: QString| {
                        update_model!(this, job_id, itm { itm.thumbnail_url = thumb; });
                    });
                    let apply_preset = util::qt_queued_callback_mut(self, move |this, (preset, to_job_id): (String, u32)| {
                        this.apply_to_all(preset, additional_data3.clone(), to_job_id);
                        this.added(job_id);
                    });

                    core::run_threaded(move || {
                        let fetch_thumb = |video_url: &str, ratio: f64| -> Result<(), rendering::FFmpegError> {
                            let mut fetched = false;
                            if !crate::cli::will_run_in_console() { // Don't fetch thumbs in the CLI
                                let fs_base = gyroflow_core::filesystem::get_engine_base();
                                let mut proc = rendering::VideoProcessor::from_file(&fs_base, video_url, false, 0, None)?;
                                proc.on_frame(move |_timestamp_us, input_frame, _output_frame, converter, _rate_control| {
                                    let sf = converter.scale(input_frame, ffmpeg_next::format::Pixel::RGBA, (50.0 * ratio).round() as u32, 50)?;

                                    if !fetched {
                                        thumb_fetched(util::image_data_to_base64(sf.plane_width(0), sf.plane_height(0), sf.stride(0) as u32, sf.data(0)));
                                        fetched = true;
                                    }

                                    Ok(())
                                });
                                proc.start_decoder_only(vec![(0.0, 50.0)], Arc::new(AtomicBool::new(true)))?;
                            }
                            Ok(())
                        };

                        if is_gf_data || core::filesystem::get_filename(&url).ends_with(".gyroflow") {
                            if !is_gf_data {
                                let video_url = || -> Option<String> {
                                    let data = core::filesystem::read(&url).ok()?;
                                    let obj: serde_json::Value = serde_json::from_slice(&data).ok()?;
                                    Some(obj.get("videofile")?.as_str()?.to_string())
                                }().unwrap_or_default();

                                if video_url.is_empty() {
                                    // It's a preset
                                    if let Ok(data) = core::filesystem::read_to_string(&url) {
                                        apply_preset((data, 0));
                                    }
                                    return;
                                }
                            }

                            let result = if is_gf_data {
                                let mut is_preset = false;
                                stab.import_gyroflow_data(url.as_bytes(), true, None, |_|(), Arc::new(AtomicBool::new(false)), &mut is_preset)
                            } else {
                                stab.import_gyroflow_file(&url, true, |_|(), Arc::new(AtomicBool::new(false)))
                            };

                            match result {
                                Ok(obj) => {
                                    if let Some(out) = obj.get("output") {
                                        if let Ok(mut render_options2) = serde_json::from_value(out.clone()) as serde_json::Result<RenderOptions> {
                                            render_options2.update_from_json(out);
                                            loaded(render_options2);
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

                                    Self::update_sync_settings(&stab, &sync_options);
                                    if let Some(sync) = obj.get("synchronization").and_then(|x| x.as_object()) {
                                        if !sync.is_empty() {
                                            Self::update_sync_settings(&stab, &serde_json::Value::Object(sync.clone()));
                                        }
                                    }

                                    processing_done(());
                                },
                                Err(e) => {
                                    err(("An error occured: %1".to_string(), format!("Error loading {}: {:?}", url, e)));
                                }
                            }
                        } else if let Ok(info) = rendering::VideoProcessor::get_video_info(&url) {
                            ::log::info!("Loaded {:?}", &info);

                            render_options.bitrate = render_options.bitrate.max(info.bitrate);
                            render_options.output_width = info.width as usize;
                            render_options.output_height = info.height as usize;
                            render_options.output_folder = Self::get_output_folder(&url, &render_options.output_folder);
                            render_options.output_filename = Self::get_output_filename(&url, &suffix, &render_options, override_ext.as_deref());

                            let ratio = info.width as f64 / info.height as f64;

                            if info.duration_ms > 0.0 && info.fps > 0.0 {

                                let video_size = (info.width as usize, info.height as usize);

                                stab.init_from_video_data(info.duration_ms, info.fps, info.frame_count, video_size);
                                stab.set_video_rotation(((360 - info.rotation) % 360) as f64);

                                stab.input_file.write().url = url.clone();

                                let is_main_video = gyro_url.is_empty();
                                let gyro_url = if !gyro_url.is_empty() { &gyro_url } else { &url };
                                let _ = stab.load_gyro_data(gyro_url, is_main_video, &Default::default(), |_|(), Arc::new(AtomicBool::new(false)));

                                let camera_id = stab.camera_id.read();

                                let id_str = camera_id.as_ref().map(|v| v.get_identifier_for_autoload()).unwrap_or_default();
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

                                loaded(render_options);

                                Self::update_sync_settings(&stab, &sync_options);

                                let default_preset = gyroflow_core::lens_profile_database::LensProfileDatabase::get_path().join("default.gyroflow");
                                if let Ok(data) = std::fs::read_to_string(default_preset) {
                                    // Apply default preset
                                    apply_preset((data, job_id));
                                }

                                if let Err(e) = fetch_thumb(&url, ratio) {
                                    err(("An error occured: %1".to_string(), e.to_string()));
                                }

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

    fn do_autosync<F: Fn(f64) + Send + Sync + Clone + 'static, F2: Fn((String, String)) + Send + Sync + Clone + 'static>(stab: Arc<StabilizationManager>, processing_cb: F, err: F2, proc_height: i32) {
        let (url, duration_ms) = {
            (stab.input_file.read().url.clone(), stab.params.read().duration_ms)
        };

        let (has_sync_points, has_accurate_timestamps) = {
            let gyro = stab.gyro.read();
            (!gyro.get_offsets().is_empty(), gyro.file_metadata.has_accurate_timestamps)
        };
        let fps = stab.params.read().fps;

        let sync_settings = stab.lens.read().sync_settings.clone().unwrap_or_default();
        if !has_sync_points && !has_accurate_timestamps && sync_settings.get("do_autosync").and_then(|v| v.as_bool()).unwrap_or_default() {
            // ----------------------------------------------------------------------------
            // --------------------------------- Autosync ---------------------------------
            processing_cb(0.01);
            use gyroflow_core::synchronization::AutosyncProcess;
            use gyroflow_core::synchronization;
            use crate::rendering::VideoProcessor;
            use itertools::Either;

            if let Ok(mut sync_params) = serde_json::from_value(sync_settings) as serde_json::Result<synchronization::SyncParams> {
                if sync_params.max_sync_points > 0 {
                    let chunks = 1.0 / sync_params.max_sync_points as f64;
                    let start = chunks / 2.0;
                    let mut timestamps_fract: Vec<f64> = (0..sync_params.max_sync_points).map(|i| start + (i as f64 * chunks)).collect();

                    if !sync_params.custom_sync_pattern.is_null() {
                        let v = Self::resolve_syncpoint_pattern(&sync_params.custom_sync_pattern, duration_ms, fps);
                        timestamps_fract = v.into_iter().filter(|v| *v <= duration_ms).map(|v| v / duration_ms).collect();
                    }

                    #[cfg(not(any(target_os = "ios", target_os = "android")))]
                    let _prevent_system_sleep = keep_awake::inhibit_system("Gyroflow", "Autosyncing");
                    #[cfg(any(target_os = "ios", target_os = "android"))]
                    let _prevent_system_sleep = keep_awake::inhibit_display("Gyroflow", "Autosyncing");

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
                                stab2.keyframes.write().update_gyro(&gyro);
                            }
                        });

                        let (sw, sh) = ((720.0 * (size.0 as f64 / size.1 as f64)).round() as u32, 720);

                        let gpu_decoding = *rendering::GPU_DECODING.read();

                        let mut frame_no = 0;
                        let mut abs_frame_no = 0;
                        let sync = Arc::new(sync);

                        let mut decoder_options = ffmpeg_next::Dictionary::new();
                        if proc_height > 0 {
                            decoder_options.set("scale", &format!("{}x{}", (proc_height * 16) / 9, proc_height));
                        }
                        ::log::debug!("Decoder options: {:?}", decoder_options);

                        let fs_base = gyroflow_core::filesystem::get_engine_base();
                        match VideoProcessor::from_file(&fs_base, &url, gpu_decoding, 0, Some(decoder_options)) {
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
                        };
                    } else {
                        err(("An error occured: %1".to_string(), "Invalid parameters".to_string()));
                    }

                    stab.recompute_blocking();
                }
            }
            processing_cb(1.0);
            // --------------------------------- Autosync ---------------------------------
            // ----------------------------------------------------------------------------
        }
    }

    pub fn apply_to_all(&mut self, data: String, additional_data: String, to_job_id: u32) {
        ::log::debug!("Applying preset {}", &data);
        let data_parsed: serde_json::Result<serde_json::Value> = serde_json::from_str(&data);
        let mut new_output_options = None;
        if let Ok(obj) = &data_parsed {
            if let Some(output) = obj.get("output") {
                new_output_options = Some(output.clone());
            }
        }
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
            if to_job_id > 0 && *job_id != to_job_id { continue; }
            if job.queue_index < q.row_count() as usize {
                let mut itm = q[job.queue_index].clone();
                if itm.status == JobStatus::Queued {
                    let stab = job.stab.clone();
                    let data_vec = data.to_vec();
                    let mut sync_options = serde_json::Value::default();
                    if let Ok(additional_data) = serde_json::from_str(&additional_data) as serde_json::Result<serde_json::Value> {
                        if let Some(sync) = additional_data.get("synchronization") {
                            sync_options = sync.clone();
                        }
                    }
                    if let Ok(obj) = &data_parsed {
                        if let Some(sync) = obj.get("synchronization") {
                            sync_options = sync.clone();
                        }
                    }
                    let job_id = *job_id;
                    if let Some(ref new_output_options) = new_output_options {
                        let override_ext = new_output_options.get("output_extension").and_then(|x| x.as_str());
                        job.render_options.update_from_json(new_output_options);
                        job.render_options.output_folder = Self::get_output_folder(&itm.input_file.to_string(), &job.render_options.output_folder);
                        job.render_options.output_filename = Self::get_output_filename(&itm.input_file.to_string(), &self.default_suffix.to_string(), &job.render_options, override_ext);
                        itm.export_settings = QString::from(job.render_options.settings_string(job.stab.params.read().fps));
                        itm.output_filename = QString::from(job.render_options.output_filename.as_str());
                        itm.output_folder   = QString::from(job.render_options.output_folder.as_str());
                        itm.display_output_path = QString::from(core::filesystem::display_folder_filename(job.render_options.output_folder.as_str(), job.render_options.output_filename.as_str()));
                        job.project_data = Self::get_gyroflow_data_internal(&job.stab, &job.additional_data, &job.render_options);
                        if core::filesystem::exists_in_folder(&job.render_options.output_folder, &job.render_options.output_filename.replace("_%05d", "_00001")) {
                            let msg = QString::from(format!("file_exists:{}", serde_json::json!({ "filename": job.render_options.output_filename, "folder": job.render_options.output_folder })));
                            itm.error_string = msg.clone();
                            itm.status = JobStatus::Error;
                            err((job_id, msg.to_string()));
                        }
                    }

                    let mut is_preset = false;
                    if let Err(e) = stab.import_gyroflow_data(&data_vec, true, None, |_|(), Arc::new(AtomicBool::new(false)), &mut is_preset) {
                        ::log::error!("Failed to update queue stab data: {:?}", e);
                    }

                    Self::update_sync_settings(&stab, &sync_options);
                    processing_done(job_id);

                    q.change_line(job.queue_index, itm);
                }
            }
        }
    }

    fn file_exists_in_folder(&self, folder: QUrl, filename: QString) -> bool {
        let folder = QString::from(folder).to_string();
        let filename = filename.to_string();
        for (_id, job) in self.jobs.iter() {
            if job.render_options.output_folder == folder && job.render_options.output_filename == filename {
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

    // Keep in sync with Synchronization.qml
    fn resolve_syncpoint_pattern(o: &serde_json::Value, duration: f64, fps: f64) -> Vec<f64> {
        fn resolve_duration_to_ms(d: &serde_json::Value, fps: f64) -> Option<f64> {
            if !d.is_number() && !d.is_string() { return None; }
                 if d.is_string() && d.as_str()?.ends_with("ms") { d.as_str()?.strip_suffix("ms")?.parse::<f64>().ok() }
            else if d.is_string() && d.as_str()?.ends_with('s')  { d.as_str()?.strip_suffix('s')?.parse::<f64>().ok().map(|x| x * 1000.0) }
            else if d.is_string() { d.as_str()?.parse::<f64>().ok().map(|x| (x / fps) * 1000.0) }
            else { d.as_f64().map(|x| (x / fps) * 1000.0) }
        }
        fn resolve_item(x: &serde_json::Value, duration: f64, fps: f64) -> Vec<f64> {
            if let Some(x) = x.as_object() {
                let start = x.get("start").and_then(|y| resolve_duration_to_ms(y, fps)).unwrap_or_default();
                let interval = x.get("interval").and_then(|y| resolve_duration_to_ms(y, fps)).unwrap_or(duration);
                let gap = x.get("gap").and_then(|y| resolve_duration_to_ms(y, fps)).unwrap_or_default();
                let mut out = Vec::new();
                let mut i = start;
                while i < duration {
                    out.push(i - gap / 2.0);
                    if gap > 0.0 {
                        out.push(i + gap / 2.0);
                    }
                    i += interval;
                }
                out
            } else {
                Vec::new()
            }
        }

        let mut timestamps = Vec::new();
        if let Some(array) = o.as_array() {
            for x in array {
                timestamps.append(&mut resolve_item(x, duration, fps));
            }
        } else if o.is_object() {
            timestamps.append(&mut resolve_item(o, duration, fps));
        }
        timestamps.sort_by(|a, b| a.total_cmp(b));

        timestamps
    }

    fn update_sync_settings(stab: &StabilizationManager, sync_options: &serde_json::Value) {
        let mut sync_settings = stab.lens.read().sync_settings.clone().unwrap_or(sync_options.clone());
        if sync_settings.is_object() && sync_options.is_object() {
            crate::core::util::merge_json(&mut sync_settings, sync_options);
        }
        if sync_settings.is_object() && !sync_settings.as_object().unwrap().is_empty() {
            stab.lens.write().sync_settings = Some(sync_settings);
        }
    }
}
