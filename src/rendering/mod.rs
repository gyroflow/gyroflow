// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

mod ffmpeg_audio;
mod ffmpeg_video;
mod ffmpeg_video_converter;
mod audio_resampler;
pub mod ffmpeg_processor;
pub mod ffmpeg_hw;
pub mod render_queue;
pub mod mdk_processor;
pub mod video_processor;
pub mod zero_copy;
use zero_copy::*;
#[cfg(target_os = "android")]
pub mod ffmpeg_android;

pub use self::video_processor::VideoProcessor;
pub use self::ffmpeg_processor::{ FfmpegProcessor, FFmpegError };
use render_queue::RenderOptions;
use crate::core::{ StabilizationManager, stabilization::* };
use ffmpeg_next::{ format::Pixel, frame::Video, codec, Error, ffi };
use std::cell::RefCell;
use std::ffi::c_void;
use std::os::raw::c_char;
use std::os::raw::c_int;
use std::rc::Rc;
use std::sync::{ Arc, atomic::AtomicBool };
use parking_lot::RwLock;
use gyroflow_core::gpu::Buffers;

#[derive(Debug, PartialEq, Clone, Copy)]
enum GpuType {
    Nvidia, Amd, Intel, AppleSilicon, Android, Unknown
}
lazy_static::lazy_static! {
    static ref GPU_TYPE: RwLock<GpuType> = RwLock::new(GpuType::Unknown);
    pub static ref GPU_DECODING: RwLock<bool> = RwLock::new(true);
}
pub fn set_gpu_type_from_name(name: &str) {
    let name = name.to_ascii_lowercase();
         if name.contains("nvidia") || name.contains("quadro") { *GPU_TYPE.write() = GpuType::Nvidia; }
    else if name.contains("amd") || name.contains("advanced micro devices") { *GPU_TYPE.write() = GpuType::Amd; }
    else if name.contains("intel") && !name.contains("intel(r) core(tm)") { *GPU_TYPE.write() = GpuType::Intel; }
    else if name.contains("apple m") { *GPU_TYPE.write() = GpuType::AppleSilicon; }
    else if name.contains("adreno") { *GPU_TYPE.write() = GpuType::Android; }
    else {
        log::warn!("Unknown GPU {}", name);
    }

    let gpu_type = *GPU_TYPE.read();
    if gpu_type == GpuType::Nvidia {
        ffmpeg_hw::initialize_ctx(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA);
    }
    #[cfg(target_os = "windows")]
    if gpu_type == GpuType::Amd {
        ffmpeg_hw::initialize_ctx(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA);
    }
    #[cfg(target_os = "android")]
    {
        ffmpeg_hw::initialize_ctx(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_MEDIACODEC);
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        #[cfg(target_os = "macos")]
        if !name.contains("apple m") {
            // Disable GPU decoding on Intel macOS by default
            *GPU_DECODING.write() = false;
        }
        ffmpeg_hw::initialize_ctx(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX);
    }

    ::log::debug!("GPU type: {:?}, from name: {}", gpu_type, name);
}

pub fn get_possible_encoders(codec: &str, use_gpu: bool) -> Vec<(&'static str, bool)> { // -> (name, is_gpu)
    if codec.contains("PNG") || codec.contains("png") { return vec![("png", false)]; }
    if codec.contains("EXR") || codec.contains("exr") { return vec![("exr", false)]; }

    let mut encoders = if use_gpu {
        match codec {
            "H.264/AVC" => vec![
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                ("h264_videotoolbox", true),
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                ("h264_nvenc",        true),
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                ("h264_amf",          true),
                #[cfg(any(target_os = "linux"))]
                ("h264_vaapi",        true),
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                ("h264_qsv",          true),
                #[cfg(target_os = "windows")]
                ("h264_mf",           true),
                #[cfg(target_os = "linux")]
                ("h264_v4l2m2m",      true),
                #[cfg(target_os = "android")]
                ("h264_mediacodec",   true),
                ("libx264",           false),
            ],
            "H.265/HEVC" => vec![
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                ("hevc_videotoolbox", true),
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                ("hevc_nvenc",        true),
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                ("hevc_amf",          true),
                #[cfg(any(target_os = "linux"))]
                ("hevc_vaapi",        true),
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                ("hevc_qsv",          true),
                #[cfg(target_os = "windows")]
                ("hevc_mf",           true),
                #[cfg(target_os = "linux")]
                ("hevc_v4l2m2m",      true),
                #[cfg(target_os = "android")]
                ("hevc_mediacodec",   true),
                ("libx265",           false),
            ],
            "AV1" => vec![
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                ("av1_nvenc",        true),
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                ("av1_amf",          true),
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                ("av1_qsv",          true),
                #[cfg(any(target_os = "linux"))]
                ("av1_vaapi",         true),
                #[cfg(target_os = "android")]
                ("av1_mediacodec",   true),
                ("librav1e",         false),
                ("libaom-av1",       false),
                ("libsvtav1",        false),
            ],
            "ProRes" => vec![
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                ("prores_videotoolbox", true),
                ("prores_ks", false)
            ],
            "DNxHD"    => vec![("dnxhd", false)],
            "CineForm" => vec![("cfhd", false)],
            _          => vec![]
        }
    } else {
        match codec {
            "H.264/AVC"  => vec![("libx264", false)],
            "H.265/HEVC" => vec![("libx265", false)],
            "ProRes"     => vec![("prores_ks", false)],
            "DNxHD"      => vec![("dnxhd", false)],
            "CineForm"   => vec![("cfhd", false)],
            "AV1"        => vec![("librav1e", false), ("libaom-av1", false), ("libsvtav1", false)],
            _            => vec![]
        }
    };

    let gpu_type = *GPU_TYPE.read();
    if gpu_type != GpuType::Nvidia {
        encoders.retain(|x| !x.0.contains("nvenc"));
    }
    if gpu_type != GpuType::Amd {
        encoders.retain(|x| !x.0.contains("_amf"));
    }
    if gpu_type != GpuType::Intel {
        encoders.retain(|x| !x.0.contains("qsv"));
    }
    log::debug!("Possible encoders with {:?}: {:?}", gpu_type, encoders);
    encoders
}

pub fn render<F, F2>(stab: Arc<StabilizationManager>, progress: F, input_file: &gyroflow_core::InputFile, render_options: &RenderOptions, gpu_decoder_index: i32, cancel_flag: Arc<AtomicBool>, pause_flag: Arc<AtomicBool>, encoder_initialized: F2) -> Result<(), FFmpegError>
    where F: Fn((f64, usize, usize, bool, bool)) + Send + Sync + Clone,
          F2: Fn(String) + Send + Sync + Clone
{
    log::debug!("ffmpeg_hw::supported_gpu_backends: {:?}", ffmpeg_hw::supported_gpu_backends());

    let params = stab.params.read();
    let trim_ratio = if !render_options.pad_with_black && !render_options.preserve_other_tracks {
        params.trim_end - params.trim_start
    } else {
        1.0
    };
    let total_frame_count = params.frame_count;
    let fps_scale = params.fps_scale;
    let has_alpha = params.background[3] < 1.0;

    let mut pixel_format = render_options.pixel_format.clone();

    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let _prevent_system_sleep = keep_awake::inhibit_system("Gyroflow", "Rendering video");
    #[cfg(any(target_os = "ios", target_os = "android"))]
    let _prevent_system_sleep = keep_awake::inhibit_display("Gyroflow", "Rendering video");

    let mut output_width = render_options.output_width;
    let mut output_height = render_options.output_height;
    if cfg!(target_os = "android") {
        // Workaround for MediaCodec alignment requirement, until more proper fix is found
        // TODO: investigate and find proper fix in the MediaCodec encoder
        fn aligned_to_16(mut x: usize) -> usize { if (x % 16) != 0 { x += 16 - x % 16; } x }
        output_width = aligned_to_16(output_width);
        output_height = aligned_to_16(output_height);
    }

    let duration_ms = params.duration_ms;
    let fps = params.fps;
    let video_speed = params.video_speed;

    let render_duration = params.duration_ms * trim_ratio;
    let render_frame_count = (total_frame_count as f64 * trim_ratio).round() as usize;

    // Only use post-conversion processing when background is not opaque
    let order = if params.background[3] < 1.0 {
        ffmpeg_video::ProcessingOrder::PostConversion
    } else {
        ffmpeg_video::ProcessingOrder::PreConversion
    };

    let (trim_start, trim_end) = (params.trim_start, params.trim_end);

    drop(params);

    let mut decoder_options = ffmpeg_next::Dictionary::new();
    if input_file.image_sequence_fps > 0.0 {
        let fps = fps_to_rational(input_file.image_sequence_fps);
        decoder_options.set("framerate", &format!("{}/{}", fps.numerator(), fps.denominator()));
    }
    if input_file.image_sequence_start > 0 {
        decoder_options.set("start_number", &format!("{}", input_file.image_sequence_start));
    }
    if cfg!(target_os = "android") {
        decoder_options.set("ndk_codec", "1");
    }
    let gpu_decoding = *GPU_DECODING.read();
    let fs_base = gyroflow_core::filesystem::get_engine_base();
    let mut proc = FfmpegProcessor::from_file(&fs_base, &input_file.url, gpu_decoding && gpu_decoder_index >= 0, gpu_decoder_index as usize, Some(decoder_options))?;

    let render_options_dict = render_options.get_encoder_options_dict();
    let hwaccel_device = render_options_dict.get("hwaccel_device");

    match render_options.audio_codec.as_ref() {
        "AAC"         => proc.audio_codec = ffmpeg_next::codec::Id::AAC,
        "PCM (s16le)" => proc.audio_codec = ffmpeg_next::codec::Id::PCM_S16LE,
        "PCM (s16be)" => proc.audio_codec = ffmpeg_next::codec::Id::PCM_S16BE,
        "PCM (s24le)" => proc.audio_codec = ffmpeg_next::codec::Id::PCM_S24LE,
        "PCM (s24be)" => proc.audio_codec = ffmpeg_next::codec::Id::PCM_S24BE,
        _ => { }
    }

    log::debug!("proc.gpu_device: {:?}", &proc.gpu_device);
    let encoder = ffmpeg_hw::find_working_encoder(&get_possible_encoders(&render_options.codec, render_options.use_gpu), hwaccel_device);
    proc.video_codec = Some(encoder.0.to_owned());
    proc.video.gpu_encoding = encoder.1;
    proc.video.encoder_params.hw_device_type = encoder.2;
    proc.video.encoder_params.options.set("threads", "auto");
    proc.video.encoder_params.metadata = render_options.get_metadata_dict();
    proc.video.processing_order = order;
    log::debug!("video_codec: {:?}, processing_order: {:?}", &proc.video_codec, proc.video.processing_order);

    if !render_options.pad_with_black && !render_options.preserve_other_tracks {
        if trim_start > 0.0 { proc.start_ms = Some(trim_start * duration_ms); }
        if trim_end   < 1.0 { proc.end_ms   = Some(trim_end   * duration_ms); }
    }

    match proc.video_codec.as_deref() {
        Some("prores_ks") | Some("prores_videotoolbox") => {
            let profiles = ["Proxy", "LT", "Standard", "HQ", "4444", "4444XQ"];
            let pix_fmts = [Pixel::YUV422P10LE, Pixel::YUV422P10LE, Pixel::YUV422P10LE, Pixel::YUV422P10LE, Pixel::YUVA444P10LE, Pixel::YUVA444P10LE];
            if let Some(profile) = profiles.iter().position(|&x| x == render_options.codec_options) {
                proc.video.encoder_params.options.set("profile", &format!("{}", profile));
                if proc.video_codec.as_deref() == Some("prores_ks") {
                    proc.video.encoder_params.pixel_format = Some(pix_fmts[profile]);
                }
            }
            proc.video.clone_frames = proc.video_codec.as_deref() == Some("prores_ks");
        }
        Some("dnxhd") => {
            let profiles = ["DNxHD", "DNxHR LB", "DNxHR SQ", "DNxHR HQ", "DNxHR HQX", "DNxHR 444"];
            let pix_fmts = [Pixel::YUV422P, Pixel::YUV422P, Pixel::YUV422P, Pixel::YUV422P, Pixel::YUV422P10LE, Pixel::YUV444P10LE];
            if let Some(profile) = profiles.iter().position(|&x| x == render_options.codec_options) {
                proc.video.encoder_params.options.set("profile", &format!("{}", profile));
                proc.video.encoder_params.pixel_format = Some(pix_fmts[profile]);
            }
            proc.video.clone_frames = true;
        }
        Some("cfhd") => {
            proc.video.encoder_params.pixel_format = Some(Pixel::YUV422P10LE);
            proc.video.clone_frames = true;
        }
        Some("png") => {
            if render_options.codec_options.contains("16-bit") {
                proc.video.encoder_params.pixel_format = Some(if has_alpha { Pixel::RGBA64BE } else { Pixel::RGB48BE });
            } else {
                proc.video.encoder_params.pixel_format = Some(if has_alpha { Pixel::RGBA } else { Pixel::RGB24 });
            }
            proc.video.clone_frames = true;
        }
        Some("exr") => {
            proc.video.clone_frames = true;
            proc.video.encoder_params.options.set("compression", "1"); // RLE compression
            proc.video.encoder_params.options.set("gamma", "1.0");
            proc.video.encoder_params.pixel_format = Some(if has_alpha { Pixel::GBRAPF32LE } else { Pixel::GBRPF32LE });
            /*Decoder options:
                -layer             <string>     .D.V....... Set the decoding layer (default "")
                -part              <int>        .D.V....... Set the decoding part (from 0 to INT_MAX) (default 0)
                -gamma             <float>      .D.V....... Set the float gamma value when decoding (from 0.001 to FLT_MAX) (default 1)
                -apply_trc         <int>        .D.V....... color transfer characteristics to apply to EXR linear input (from 1 to 18) (default gamma)
                    bt709           1            .D.V....... BT.709
                    gamma           2            .D.V....... gamma
                    gamma22         4            .D.V....... BT.470 M
                    gamma28         5            .D.V....... BT.470 BG
                    smpte170m       6            .D.V....... SMPTE 170 M
                    smpte240m       7            .D.V....... SMPTE 240 M
                    linear          8            .D.V....... Linear
                    log             9            .D.V....... Log
                    log_sqrt        10           .D.V....... Log square root
                    iec61966_2_4    11           .D.V....... IEC 61966-2-4
                    bt1361          12           .D.V....... BT.1361
                    iec61966_2_1    13           .D.V....... IEC 61966-2-1
                    bt2020_10bit    14           .D.V....... BT.2020 - 10 bit
                    bt2020_12bit    15           .D.V....... BT.2020 - 12 bit
                    smpte2084       16           .D.V....... SMPTE ST 2084
                    smpte428_1      17           .D.V....... SMPTE ST 428-1
            */
        }
        _ => { }
    }

    if cfg!(any(target_os = "macos", target_os = "ios")) {
        proc.video.encoder_params.options.set("allow_sw", "1");
        proc.video.encoder_params.options.set("realtime", "0");

        // Test if `constant_bit_rate` is supported
        {
            let log = FFMPEG_LOG.read().clone();
            if let Some(enc) = ffmpeg_next::encoder::find_by_name("h264_videotoolbox") {
                let ctx_ptr = unsafe { ffi::avcodec_alloc_context3(enc.as_ptr()) };
                let context = unsafe { codec::context::Context::wrap(ctx_ptr, Some(Rc::new(0))) };
                let mut encoder = context.encoder().video()?;
                encoder.set_width(1920);
                encoder.set_height(1080);
                encoder.set_format(ffmpeg_next::format::Pixel::NV12);
                encoder.set_time_base(ffmpeg_next::Rational::new(30, 1));
                let mut options = ffmpeg_next::Dictionary::new();
                options.set("allow_sw", "1");
                options.set("constant_bit_rate", "1");
                if encoder.open_with(options).is_ok() {
                    proc.video.encoder_params.options.set("constant_bit_rate", "1");
                }
            }
            *FFMPEG_LOG.write() = log;
        }
    }
    if encoder.0.contains("nvenc") {
        proc.video.encoder_params.options.set("b_ref_mode", "disabled");
    }

    if cfg!(target_os = "android") {
        proc.video.encoder_params.options.set("ndk_codec", "1");
    }

    proc.video.encoder_params.keyframe_distance_s = render_options.keyframe_distance.max(0.0001);

    proc.preserve_other_tracks = render_options.preserve_other_tracks;

    for (key, value) in render_options_dict.iter() {
        log::info!("Setting encoder option {}: {}", key, value);
        if key == "pix_fmt" {
            pixel_format = value.to_string();
            continue;
        }
        proc.video.encoder_params.options.set(key, value);
    }

    if !pixel_format.is_empty() {
        use std::str::FromStr;
        match Pixel::from_str(&pixel_format.to_ascii_lowercase()) {
            Ok(px) => { proc.video.encoder_params.pixel_format = Some(px); },
            Err(e) => { ::log::debug!("Unknown requested pixel format: {}, {:?}", pixel_format, e); }
        }
    }

    let start_us = (proc.start_ms.unwrap_or_default() * 1000.0) as i64;

    if !render_options.audio {
        proc.audio_codec = codec::Id::None;
    }

    log::debug!("start_us: {}, render_duration: {}, render_frame_count: {}", start_us, render_duration, render_frame_count);

    let mut planes = Vec::<Box<dyn FnMut(i64, &mut Video, &mut Video, usize, bool)>>::new();

    let is_prores_videotoolbox = proc.video_codec.as_deref() == Some("prores_videotoolbox");

    let progress2 = progress.clone();
    let mut process_frame = 0;

    proc.on_encoder_initialized(|enc: &ffmpeg_next::encoder::video::Video| {
        encoder_initialized(enc.codec().map(|x| x.name().to_string()).unwrap_or_default());
        Ok(())
    });

    let mut prev_real_ts = 0;
    let mut ramped_ts = 0.0;
    let mut final_ts = 0;
    let interval = (1_000_000.0 / fps).round() as i64;
    let is_speed_changed = video_speed != 1.0 || stab.keyframes.read().is_keyframed(&gyroflow_core::keyframes::KeyframeType::VideoSpeed);
    if is_speed_changed {
        proc.audio_codec = codec::Id::None; // Audio not supported when changing speed
    }

    let render_globals = Rc::new(RefCell::new(zero_copy::RenderGlobals::default()));

    proc.on_frame(move |mut timestamp_us, input_frame, output_frame, converter, rate_control| {
        let fill_with_background = render_options.pad_with_black &&
            (timestamp_us < (trim_start * duration_ms * 1000.0).round() as i64 ||
             timestamp_us > (trim_end   * duration_ms * 1000.0).round() as i64);

        if let Some(scale) = fps_scale {
            timestamp_us = (timestamp_us as f64 / scale).round() as i64;
        }

        if is_speed_changed {
            let vid_speed = stab.keyframes.read().value_at_video_timestamp(&gyroflow_core::keyframes::KeyframeType::VideoSpeed, timestamp_us as f64 / 1000.0).unwrap_or(video_speed);
            let current_interval = ((rate_control.out_timestamp_us - prev_real_ts) as f64) / vid_speed;
            ramped_ts += current_interval;
            prev_real_ts = rate_control.out_timestamp_us;
            if ramped_ts < (final_ts as f64 + interval as f64 / 2.0) { // interval/2 because we want frame in the middle of the range, not in the end
                rate_control.repeat_times = 0; // skip this frame
                process_frame += 1;
                return Ok(());
            } else {
                let repeat_times = current_interval / interval as f64;
                if repeat_times >= 1.5 {
                    // Need to duplicate the frames
                    rate_control.repeat_times = repeat_times.round() as i64;
                    rate_control.repeat_interval = interval;
                }
            }
            rate_control.out_timestamp_us = final_ts;
            final_ts += interval * rate_control.repeat_times;
        }

        let output_frame = output_frame.unwrap();

        macro_rules! create_planes_proc {
            ($planes:ident, $(($t:tt, $in_frame:expr, $out_frame:expr, $ind:expr, $yuvi:expr, $max_val:expr), )*) => {
                $({
                    let in_size = zero_copy::get_plane_size($in_frame, $ind);
                    let out_size = zero_copy::get_plane_size($out_frame, $ind);
                    {
                        let mut params = stab.params.write();
                        params.plane_scale = Some(in_size.0 as f64 / params.video_size.0.max(1) as f64);
                        params.size        = (in_size.0,  in_size.1);
                        params.output_size = (out_size.0, out_size.1);
                        params.video_size  = params.size;
                        params.video_output_size = params.output_size;
                    }
                    let mut plane = Stabilization::default();
                    plane.interpolation = Interpolation::Lanczos4;
                    plane.share_wgpu_instances = true;
                    plane.set_device(stab.params.read().current_device as isize);

                    // Workaround for a bug in prores videotoolbox encoder
                    if $in_frame.format() == ffmpeg_next::format::Pixel::NV12 && is_prores_videotoolbox {
                        plane.kernel_flags.set(KernelParamsFlags::FIX_COLOR_RANGE, true);
                    }

                    let mut compute_params = ComputeParams::from_manager(&stab);

                    let is_limited_range = $out_frame.color_range() == ffmpeg_next::util::color::Range::MPEG;
                    compute_params.background = <$t as PixelType>::from_rgb_color(compute_params.background, &$yuvi, is_limited_range);

                    plane.init_size(in_size, out_size);
                    plane.set_compute_params(compute_params);
                    let render_globals = render_globals.clone();
                    $planes.push(Box::new(move |timestamp_us: i64, in_frame_data: &mut Video, out_frame_data: &mut Video, plane_index: usize, fill_with_background: bool| {
                        let mut g = render_globals.borrow_mut();
                        let wgpu_format = $t::wgpu_format().map(|x| x.0);

                        let mut buffers = Buffers {
                            input:  get_plane_buffer(in_frame_data, in_size, plane_index, &mut g, wgpu_format),
                            output: get_plane_buffer(out_frame_data, out_size, plane_index, &mut g, wgpu_format)
                        };

                        if plane.initialized_backend.is_none() || plane.pending_device_change.is_some() {
                            plane.ensure_ready_for_processing::<$t>(timestamp_us, &mut buffers);
                            plane.stab_data.clear();
                        }
                        let mut transform = plane.get_frame_transform_at::<$t>(timestamp_us, &mut buffers);
                        transform.kernel_params.pixel_value_limit = $max_val;
                        transform.kernel_params.max_pixel_value = $max_val;
                        if plane.initialized_backend.is_wgpu() && $t::wgpu_format().map(|x| x.2).unwrap_or_default() {
                            transform.kernel_params.pixel_value_limit = 1.0;
                            transform.kernel_params.max_pixel_value = 1.0;
                        }
                        if fill_with_background {
                            transform.kernel_params.flags |= KernelParamsFlags::FILL_WITH_BACKGROUND.bits();
                        }
                        if let Err(e) = plane.process_pixels::<$t>(timestamp_us, &mut buffers, Some(&transform)) {
                            ::log::error!("Failed to process pixels: {e:?}");
                        }
                    }));
                })*
            };
        }

        if planes.is_empty() {
            // Good reference about video formats: https://source.chromium.org/chromium/chromium/src/+/master:media/base/video_frame.cc
            // https://gist.github.com/Jim-Bar/3cbba684a71d1a9d468a6711a6eddbeb

            let mut format = input_frame.format();
            if let Some(underlying_format) = zero_copy::map_hardware_format(format, input_frame) {
                log::debug!("HW frame ({:?}) underlying format: {:?}", format, underlying_format);
                format = underlying_format;
            }
            match format {
                Pixel::NV12 => {
                    create_planes_proc!(planes,
                        (Luma8, input_frame, output_frame, 0, [0], 255.0),
                        (UV8,   input_frame, output_frame, 1, [1,2], 255.0),
                    );
                },
                Pixel::NV21 => {
                    create_planes_proc!(planes,
                        (Luma8, input_frame, output_frame, 0, [0], 255.0),
                        (UV8,   input_frame, output_frame, 1, [2,1], 255.0),
                    );
                },
                Pixel::P010LE | Pixel::P016LE |
                Pixel::P210LE | Pixel::P216LE |
                Pixel::P410LE | Pixel::P416LE => {
                    let max_val = match input_frame.format() {
                        // I'm not sure if this is correct but it appears that P010LE uses 16-bit values, even though it's 10-bit
                        //Pixel::P010LE | Pixel::P210LE | Pixel::P410LE => 1023.0,
                        _ => 65535.0
                    };
                    create_planes_proc!(planes,
                        (Luma16, input_frame, output_frame, 0, [0], max_val),
                        (UV16,   input_frame, output_frame, 1, [1,2], max_val),
                    );
                },
                Pixel::YUV420P | Pixel::YUVJ420P => {
                    create_planes_proc!(planes,
                        (Luma8, input_frame, output_frame, 0, [0], 255.0),
                        (Luma8, input_frame, output_frame, 1, [1], 255.0),
                        (Luma8, input_frame, output_frame, 2, [2], 255.0),
                    );
                },
                Pixel::YUV420P10LE | Pixel::YUV420P12LE | Pixel::YUV420P14LE | Pixel::YUV420P16LE |
                Pixel::YUV422P10LE | Pixel::YUV422P12LE | Pixel::YUV422P14LE | Pixel::YUV422P16LE |
                Pixel::YUV444P10LE | Pixel::YUV444P12LE | Pixel::YUV444P14LE | Pixel::YUV444P16LE => {
                    let max_val = match input_frame.format() {
                        Pixel::YUV420P10LE | Pixel::YUV422P10LE | Pixel::YUV444P10LE => 1023.0,
                        Pixel::YUV420P12LE | Pixel::YUV422P12LE | Pixel::YUV444P12LE => 4095.0,
                        Pixel::YUV420P14LE | Pixel::YUV422P14LE | Pixel::YUV444P14LE => 16383.0,
                        _ => 65535.0
                    };
                    create_planes_proc!(planes,
                        (Luma16, input_frame, output_frame, 0, [0], max_val),
                        (Luma16, input_frame, output_frame, 1, [1], max_val),
                        (Luma16, input_frame, output_frame, 2, [2], max_val),
                    );
                },
                Pixel::YUVA444P10LE | Pixel::YUVA444P12LE | Pixel::YUVA444P16LE => {
                    let max_val = match input_frame.format() {
                        Pixel::YUVA444P10LE => 1023.0,
                        Pixel::YUVA444P12LE => 4095.0,
                        _ => 65535.0
                    };
                    create_planes_proc!(planes,
                        (Luma16, input_frame, output_frame, 0, [0], max_val),
                        (Luma16, input_frame, output_frame, 1, [1], max_val),
                        (Luma16, input_frame, output_frame, 2, [2], max_val),
                        (Luma16, input_frame, output_frame, 3, [3], max_val),
                    );
                },
                Pixel::GBRAPF32LE => { create_planes_proc!(planes,
                    (R32f,  input_frame, output_frame, 0, [2], 255.0),
                    (R32f,  input_frame, output_frame, 0, [0], 255.0),
                    (R32f,  input_frame, output_frame, 0, [1], 255.0),
                    (R32f,  input_frame, output_frame, 0, [3], 255.0),
                ); },
                Pixel::GBRPF32LE => { create_planes_proc!(planes,
                    (R32f,  input_frame, output_frame, 0, [2], 255.0),
                    (R32f,  input_frame, output_frame, 0, [0], 255.0),
                    (R32f,  input_frame, output_frame, 0, [1], 255.0),
                ); },
                Pixel::AYUV64LE => { create_planes_proc!(planes, (AYUV16, input_frame, output_frame, 0, [3,0,1,2], 65535.0), ); },
                Pixel::RGB24    => { create_planes_proc!(planes, (RGB8,   input_frame, output_frame, 0, [], 255.0), ); },
                Pixel::RGBA     => { create_planes_proc!(planes, (RGBA8,  input_frame, output_frame, 0, [], 255.0), ); },
                Pixel::RGB48BE  => { create_planes_proc!(planes, (RGB16,  input_frame, output_frame, 0, [], 65535.0), ); },
                Pixel::RGBA64BE => { create_planes_proc!(planes, (RGBA16, input_frame, output_frame, 0, [], 65535.0), ); },
                format => { // All other convert to YUV444P16LE
                    ::log::info!("Unknown format {:?}, converting to YUV444P16LE", format);
                    // Go through 4:4:4 because of even plane dimensions
                    converter.convert_pixel_format(input_frame, output_frame, Pixel::YUV444P16LE, |converted_frame, converted_output| {
                        create_planes_proc!(planes,
                            (Luma16, converted_frame, converted_output, 0, [0], 65535.0),
                            (Luma16, converted_frame, converted_output, 1, [1], 65535.0),
                            (Luma16, converted_frame, converted_output, 2, [2], 65535.0),
                        );
                    })?;
                }
            }
        }
        if planes.is_empty() {
            return Err(FFmpegError::UnknownPixelFormat(input_frame.format()));
        }

        let mut undistort_frame = |frame: &mut Video, out_frame: &mut Video| {
            for (i, cb) in planes.iter_mut().enumerate() {
                (*cb)(timestamp_us, frame, out_frame, i, fill_with_background);
            }
            progress2((process_frame as f64 / render_frame_count as f64, process_frame, render_frame_count, false, false));
        };

        match input_frame.format() {
            Pixel::VIDEOTOOLBOX | // Pixel::D3D11 |
            Pixel::NV12 | Pixel::NV21 | Pixel::YUV420P | Pixel::YUVJ420P |
            Pixel::P010LE | Pixel::P016LE | Pixel::P210LE | Pixel::P216LE | Pixel::P410LE | Pixel::P416LE |
            Pixel::YUV420P10LE | Pixel::YUV420P12LE | Pixel::YUV420P14LE | Pixel::YUV420P16LE |
            Pixel::YUV422P10LE | Pixel::YUV422P12LE | Pixel::YUV422P14LE | Pixel::YUV422P16LE |
            Pixel::YUV444P10LE | Pixel::YUV444P12LE | Pixel::YUV444P14LE | Pixel::YUV444P16LE |
            Pixel::YUVA444P10LE | Pixel::YUVA444P12LE | Pixel::YUVA444P16LE |
            Pixel::AYUV64LE | Pixel::GBRAPF32LE | Pixel::GBRPF32LE |
            Pixel::RGB24 | Pixel::RGBA | Pixel::RGB48BE | Pixel::RGBA64BE => {
                undistort_frame(input_frame, output_frame)
            },
            _ => {
                converter.convert_pixel_format(input_frame, output_frame, Pixel::YUV444P16LE, |converted_frame, converted_output| {
                    undistort_frame(converted_frame, converted_output);
                })?;
            }
        }

        process_frame += 1;
        // log::debug!("process_frame: {}, timestamp_us: {}", process_frame, timestamp_us);

        Ok(())
    });

    let filename = &render_options.output_filename;
    let folder = &render_options.output_folder;
    if cfg!(not(any(target_os = "android", target_os = "ios"))) && !gyroflow_core::filesystem::exists(folder) {
        let path = gyroflow_core::filesystem::url_to_path(folder);
        if !path.is_empty() {
            let _ = std::fs::create_dir_all(path);
        }
    }

    proc.render(&fs_base, folder, filename, (output_width as u32, output_height as u32), if render_options.bitrate > 0.0 { Some(render_options.bitrate) } else { None }, cancel_flag, pause_flag)?;

    drop(proc);

    let output_url = gyroflow_core::filesystem::get_file_url(folder, filename, false);

    let re = regex::Regex::new(r#"%[0-9]+d"#).unwrap();
    if re.is_match(filename) {
        ::log::debug!("Removing {output_url}");
        let _ = gyroflow_core::filesystem::remove_file(&output_url);
    }
    progress((1.0, render_frame_count, render_frame_count, true, false));

    crate::util::update_file_times(&output_url, &input_file.url);

    Ok(())
}

pub fn init_log() {
	unsafe {
        ffi::av_log_set_level(ffi::AV_LOG_INFO);
        ffi::av_log_set_callback(Some(ffmpeg_log));
    }
}

pub fn fps_to_rational(fps: f64) -> ffmpeg_next::Rational {
    if fps.fract() > 0.1 {
        ffmpeg_next::Rational::new((fps * 1001.0).round() as i32, 1001)
    } else {
        ffmpeg_next::Rational::new(fps.round() as i32, 1)
    }
}

lazy_static::lazy_static! {
    pub static ref FFMPEG_LOG: Arc<RwLock<String>> = Arc::new(RwLock::new(String::new()));
    pub static ref LAST_PREFIX: Arc<RwLock<i32>> = Arc::new(RwLock::new(1));
}

#[cfg(not(any(target_os = "linux", all(target_os = "macos", target_arch = "x86_64"))))]
type VaList = ffi::va_list;
#[cfg(any(target_os = "linux", all(target_os = "macos", target_arch = "x86_64")))]
type VaList = *mut ffi::__va_list_tag;

#[allow(improper_ctypes_definitions)]
unsafe extern "C" fn ffmpeg_log(avcl: *mut c_void, level: i32, fmt: *const c_char, vl: VaList) {
    if level <= ffi::av_log_get_level() {
        let mut line = vec![0u8; 2048];
        let mut prefix: i32 = *LAST_PREFIX.read();

        ffi::av_log_default_callback(avcl, level, fmt, vl);
        #[cfg(target_os = "android")]
        let written = ffi::av_log_format_line2(avcl, level, fmt, vl, line.as_mut_ptr() as *mut u8, line.len() as i32, &mut prefix);
        #[cfg(not(target_os = "android"))]
        let written = ffi::av_log_format_line2(avcl, level, fmt, vl, line.as_mut_ptr() as *mut i8, line.len() as i32, &mut prefix);
        if written > 0 {
            line.resize(written as usize, 0u8);
        }

        *LAST_PREFIX.write() = prefix;

        if let Ok(mut line) = String::from_utf8(line) {
            match level {
                ffi::AV_LOG_PANIC | ffi::AV_LOG_FATAL | ffi::AV_LOG_ERROR => {
                    ::log::error!("{}", line.trim());
                    line = format!("<font color=\"#d82626\">{}</font>", line);
                },
                ffi::AV_LOG_WARNING => {
                    ::log::warn!("{}", line.trim());
                    line = format!("<font color=\"#f6a10c\">{}</font>", line);
                },
                _ => { ::log::debug!("{}", line.trim()); }
            }
            FFMPEG_LOG.write().push_str(&line);
        }
    }
}

pub fn append_log(msg: &str) { ::log::debug!("{}", msg); FFMPEG_LOG.write().push_str(msg); }
pub fn get_log() -> String { FFMPEG_LOG.read().clone() }
pub fn clear_log() { FFMPEG_LOG.write().clear() }

unsafe fn to_str<'a>(ptr: *const c_char) -> std::borrow::Cow<'a, str> {
    if ptr.is_null() { return std::borrow::Cow::Borrowed(""); }
    std::ffi::CStr::from_ptr(ptr).to_string_lossy()
}
unsafe fn codec_options(c: *const ffi::AVCodec) {
    if c.is_null() {
        log::warn!("Null pointer to encoder.");
        return;
    }

    use std::fmt::Write;
    let mut ret = String::new();
    let _ = writeln!(ret, "{} <b>{}</b>:\n", ["Decoder", "Encoder"][ffi::av_codec_is_encoder(c) as usize], to_str((*c).name));

    if !(*c).pix_fmts.is_null() {
        ret.push_str("Supported pixel formats (-pix_fmt): ");
        for i in 0..100 {
            let fmt = (*c).pix_fmts.offset(i);
            if fmt.is_null() { break; }
            let p = *fmt;
            if p == ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                break;
            }
            if i > 0 { ret.push_str(", "); }
            ret.push_str(&to_str(ffi::av_get_pix_fmt_name(p)));
        }
    }

    if !(*c).priv_class.is_null() {
        ret.push_str("<pre>");
        FFMPEG_LOG.write().push_str(&ret);
        show_help_children((*c).priv_class, ffi::AV_OPT_FLAG_ENCODING_PARAM | ffi::AV_OPT_FLAG_DECODING_PARAM);
        FFMPEG_LOG.write().push_str("</pre>Additional supported flags:<pre>-hwaccel_device\n-qscale</pre>");
    }
}

unsafe fn show_help_children(mut class: *const ffi::AVClass, flags: c_int) {
    if !(*class).option.is_null() {
        let ptr = std::ptr::null_mut();
        ffi::av_opt_show2((&mut class) as *mut *const _ as *mut _, ptr, flags, 0);
    }
    // let mut iter = std::ptr::null_mut();
    // loop {
    //     let child = ffi::av_opt_child_class_iterate(class, &mut iter);
    //     if child.is_null() {
    //         break;
    //     }
    //     show_help_children(child, flags);
    // }
}

pub fn get_default_encoder(codec: &str, gpu: bool) -> String {
    let encoder = ffmpeg_hw::find_working_encoder(&get_possible_encoders(codec, gpu), None);
    encoder.0.to_string()
}
pub fn get_encoder_options(name: &str) -> String {
	init_log();
    clear_log();
    match ffmpeg_next::encoder::find_by_name(name) {
        Some(encoder) => { unsafe { codec_options(encoder.as_ptr()); } },
        None => log::warn!("Failed to find codec by name: {name}")
    }
    let ret = get_log().replace("E..V.......", "").replace('\n', "<br>");
    clear_log();
    ret
}

/*
pub fn test() {
    log::debug!("FfmpegProcessor::supported_gpu_backends: {:?}", ffmpeg_hw::supported_gpu_backends());

    let stab = StabilizationManager::<crate::core::stabilization::RGBA8>::default();
    let duration_ms = 15015.0;
    let frame_count = 900;
    let fps = 60000.0/1001.0;
    //let video_size = (3840, 2160);
    let video_size = (5120, 3840);

    let vid = "/Users/eddy/Downloads/colors-GX029349.MP4";

    stab.init_from_video_data(duration_ms, fps, frame_count, video_size).unwrap();
    stab.load_gyro_data(vid, |_|(), Arc::new(AtomicBool::new(false)));
    {
        let mut gyro = stab.gyro.write();

        //gyro.set_offset(0, -26.0);
        gyro.integration_method = 1;
        gyro.integrate();
    }
    // stab.load_lens_profile("E:/clips/GoPro/GoPro_Hero_7_Black_4K_60_wide_16by9_1_120.json").unwrap();
    stab.set_size(video_size.0, video_size.1);
    stab.set_smoothing_method(0);
    //stab.smoothing_id = 1;
    //stab.smoothing_algs[1].as_mut().set_parameter("time_constant", 0.4);
    {
        let mut params = stab.params.write();
        // params.frame_readout_time = 8.9;
        params.fov = 1.0;
        params.background = nalgebra::Vector4::new(0.0, 0.0, 0.0, 255.0);
        params.lens_correction_amount = 0.0;
    }
    stab.recompute_blocking();

    render(
        Arc::new(stab),
        move |_params: (f64, usize, usize, bool)| {
            // ::log::debug!("frame {}/{}", params.1, params.2);
        },
        vid.into(),
        &RenderOptions {
            codec: "ProRes".into(),
            codec_options: "Standard".into(),
            output_width: video_size.0,
            output_height: video_size.1,
            bitrate: 100.0,
            use_gpu: true,
            audio: true,
            pixel_format: "".into(),
        },
        -1,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false))
    ).unwrap();
}
// use opencv::core::{Mat, Size, CV_8UC1};
// use std::os::raw::c_void;

pub fn test_decode() {
    let mut proc = FfmpegProcessor::from_file("/storage/self/primary/Download/gf/h8.MP4", true, 0, None).unwrap();

    let recv = proc.android_handles.as_mut().unwrap().receiver.take().unwrap();
    // TODO: gpu scaling in filters, example here https://github.com/zmwangx/rust-ffmpeg/blob/master/examples/transcode-audio.rs, filter scale_cuvid or scale_npp
    proc.on_frame(move |timestamp_us, input_frame, _output_frame, converter, _rate_control| {

        ffmpeg_android::release_frame(input_frame);
        let hw_buf = recv.recv().unwrap();

        unsafe {
            let desc = unsafe {
                let mut result = std::mem::MaybeUninit::uninit();
                ndk_sys::AHardwareBuffer_describe(hw_buf.as_ptr(), result.as_mut_ptr());
                result.assume_init()
            };
            ::log::debug!("recv: {:x}", desc.format);
        }

        ::log::debug!("recv: {:?}", hw_buf.as_ptr());

        ::log::debug!("ts: {} width: {}, format: {:?}", timestamp_us, input_frame.width(), input_frame.format());

        /*let (w, h) = (small_frame.plane_width(0) as i32, small_frame.plane_height(0) as i32);
        let mut bytes = small_frame.data_mut(0);
        let inp = unsafe { Mat::new_size_with_data(Size::new(w, h), CV_8UC1, bytes.as_mut_ptr() as *mut c_void, w as usize) }.unwrap();
        opencv::imgcodecs::imwrite("D:/test.jpg", &inp, &opencv::types::VectorOfi32::new());*/
        Ok(())
    });
    let _time = std::time::Instant::now();
    let _ = proc.start_decoder_only(vec![(0.0, 1000.0)], Arc::new(AtomicBool::new(false)));
    ::log::debug!("Done in {:.3} ms", _time.elapsed().as_micros() as f64 / 1000.0);
}*/
