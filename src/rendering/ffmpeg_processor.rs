// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::HashMap;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::error;

use ffmpeg_next::{ ffi, codec, encoder, format, frame, media, Dictionary, Rational, Stream, rescale, rescale::Rescale };

use gyroflow_core::filesystem::{ self, FilesystemError, FfmpegPathWrapper };
use super::*;
use super::ffmpeg_video::*;
use super::ffmpeg_audio::*;
#[cfg(target_os = "android")]
use super::ffmpeg_android::*;

/// Maps the input timestamps of the current trim range to the output timeline. All streams share the same mapping
/// (`ts - first + range_offset`), so they stay in sync across trim ranges
#[derive(Debug, Default)]
pub struct FrameTimestamps {
    /// Input timestamp (µs) of the start of the current trim range, set by its first video frame
    pub first: Option<i64>,
    /// Output timestamp (µs) of the start of the current trim range: where the video of the previous one ended
    pub range_offset: i64,
    /// Output timestamp (µs) where the video of the current trim range ended, once it has. Its audio ends there too
    pub range_end: Option<i64>,
    pub last_video: Option<i64>,
    pub last_duration_video: i64,
    /// Audio streams and copied data streams, by input stream index
    streams: HashMap<usize, StreamTimeline>,
}

/// The part of a unit (packet or decoded frame) of an audio or data stream that goes in the output
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Output timestamp (µs) of the part
    pub ts_us: i64,
    /// Where the part starts in the unit, and its duration (µs). Only units of raw samples are cut, the others are written whole
    pub skip_us: i64,
    pub duration_us: i64,
}

/// Where an audio or data stream is in the output
#[derive(Debug, Default)]
struct StreamTimeline {
    /// Output timestamp (µs) of the last unit written
    last: Option<i64>,
    /// Output timestamp (µs) where the last unit written ends
    end: i64,
    /// A unit was written in the current trim range
    in_range: bool,
    /// The stream went past the end of the current trim range
    finished: bool,
}

impl FrameTimestamps {
    /// The video of the current trim range ended
    pub fn end_range(&mut self) {
        self.range_end = Some(self.last_video.unwrap_or_default() + self.last_duration_video);
    }

    /// The next trim range starts in the output where the video of the previous one ended
    pub fn start_next_range(&mut self) {
        self.range_offset = self.last_video.unwrap_or_default() + self.last_duration_video;
        self.range_end = None;
        self.first = None;
        for stream in self.streams.values_mut() {
            stream.in_range = false;
            stream.finished = false;
        }
    }

    pub fn is_finished(&self, stream_index: usize) -> bool {
        self.streams.get(&stream_index).is_some_and(|x| x.finished)
    }

    /// Places a unit (packet or decoded frame) of an audio or data stream in the output, from its input timestamp and duration (µs).
    /// A stream is written from where it left off (in the previous range, the audio runs a bit past the video), or from the start of the
    /// range, up to where the video of the range ends. `divisible` units (raw samples) are cut exactly there, the others are written whole
    /// if they're mostly inside. `None` if nothing of the unit is written, also when it has no timestamp and doesn't follow a written unit
    pub fn place(&mut self, stream_index: usize, timestamp_us: Option<i64>, duration_us: i64, divisible: bool, start_ms: Option<f64>) -> Option<Placement> {
        let stream = self.streams.entry(stream_index).or_default();
        if stream.finished {
            return None;
        }
        let duration_us = duration_us.max(0);
        let (out_ts, range_start) = match timestamp_us {
            Some(ts) => {
                let first = *self.first.get_or_insert(ts);
                // From the start of the trim range to its first video frame, the input maps to just before `range_offset`
                let range_start = start_ms.map_or(i64::MIN, |start| (start * 1000.0).round() as i64 - first + self.range_offset);
                (ts - first + self.range_offset, range_start)
            }
            // Continues the unit before it
            None if stream.in_range => (stream.end, i64::MIN),
            None => return None
        };
        // `end` starts at 0, the start of the output
        let from = stream.end.max(range_start);
        let to = self.range_end.unwrap_or(i64::MAX);

        let (skip, duration) = if divisible && duration_us > 0 {
            let skip = (from - out_ts).clamp(0, duration_us);
            let end = to.saturating_sub(out_ts).clamp(0, duration_us);
            stream.finished = out_ts + duration_us >= to;
            if end <= skip {
                return None;
            }
            (skip, end - skip)
        } else {
            // Half a unit of tolerance: rounded timestamps (to µs here, often to ms by the container) don't line up exactly,
            // and at a range boundary it keeps the unit closest to where the stream left off
            let middle = out_ts + duration_us / 2;
            if middle > to {
                stream.finished = true;
                return None;
            }
            if out_ts < 0 || middle < from || stream.last.is_some_and(|last| out_ts <= last) {
                return None;
            }
            (0, duration_us)
        };
        stream.last = Some(out_ts + skip);
        stream.end = out_ts + skip + duration;
        stream.in_range = true;
        Some(Placement { ts_us: out_ts + skip, skip_us: skip, duration_us: duration })
    }
}

pub struct FfmpegProcessor<'a> {
    pub gpu_decoding: bool,
    pub gpu_device: Option<String>,
    pub video_codec: Option<String>,

    pub audio_codec: codec::Id,
    pub audio_copy: bool,

    input_context: format::context::Input,
    input_url: String,

    pub video: VideoTranscoder<'a>,

    pub ranges_ms: Vec<(Option<f64>, Option<f64>)>,

    pub decoder_fps: f64,

    pub preserve_other_tracks: bool,
    /// Camera tracks left out of the render, to append to the output once it's written (set by `render`)
    pub camera_tracks: Option<super::camera_tracks::CameraTracks>,
    /// The data tracks to preserve that the output won't have, and why (set by `render`)
    pub tracks_not_copied: Vec<String>,

    #[cfg(target_os = "android")]
    pub android_handles: Option<AndroidHWHandles>,

    ost_time_bases: Vec<Rational>,

    frame_ts: FrameTimestamps,

    _file: FfmpegPathWrapper
}

#[derive(PartialEq)]
pub enum Status {
    Continue,
    Finish
}

#[derive(Debug)]
pub enum FFmpegError {
    EncoderNotFound,
    NoWorkingEncoder(String),
    DecoderNotFound,
    NoSupportedFormats,
    NoOutputContext,
    EncoderConverterEmpty,
    ConverterEmpty,
    FrameEmpty,
    NoGPUDecodingDevice,
    NoHWTransferFormats,
    FromHWTransferError(i32),
    ToHWTransferError(i32),
    CannotCreateGPUDecoding,
    NoFramesContext,
    GPUDecodingFailed,
    ToHWBufferError(i32),
    PixelFormatNotSupported((format::Pixel, Vec<format::Pixel>, Option<format::Pixel>)),
    UnknownPixelFormat(format::Pixel),
    InternalError(ffmpeg_next::Error),
    CannotOpenInputFile((String, FilesystemError)),
    CannotOpenOutputFile((String, FilesystemError)),
    /// The output, as it's shown, would write over the input file, or a frame of the input image sequence, shown next
    OutputIsInput(String, String),
    /// The output can't be written where it goes, or the complete render couldn't get its name: why
    Output(String),
}

impl std::fmt::Display for FFmpegError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            FFmpegError::EncoderNotFound             => write!(f, "Encoder not found"),
            FFmpegError::NoWorkingEncoder(codec)     => write!(f, "No working encoder for {codec}"),
            FFmpegError::DecoderNotFound             => write!(f, "Decoder not found"),
            FFmpegError::NoSupportedFormats          => write!(f, "No supported formats"),
            FFmpegError::NoOutputContext             => write!(f, "No output context"),
            FFmpegError::EncoderConverterEmpty       => write!(f, "Encoder converter is null"),
            FFmpegError::ConverterEmpty              => write!(f, "Converter is null"),
            FFmpegError::FrameEmpty                  => write!(f, "Frame is null"),
            FFmpegError::NoHWTransferFormats         => write!(f, "No hardware transfer formats"),
            FFmpegError::FromHWTransferError(i) => write!(f, "Error transferring frame from the GPU: {:?}", ffmpeg_next::Error::Other { errno: *i }),
            FFmpegError::ToHWTransferError(i)   => write!(f, "Error transferring frame to the GPU: {:?}", ffmpeg_next::Error::Other { errno: *i }),
            FFmpegError::ToHWBufferError(i)     => write!(f, "Error getting HW transfer buffer to the GPU: {:?}", ffmpeg_next::Error::Other { errno: *i }),
            FFmpegError::NoFramesContext             => write!(f, "Empty hw frames context"),
            FFmpegError::GPUDecodingFailed           => write!(f, "GPU decoding failed, please try again."),
            FFmpegError::CannotCreateGPUDecoding     => write!(f, "Unable to create HW devices context"),
            FFmpegError::NoGPUDecodingDevice         => write!(f, "Unable to create any HW decoding context"),
            FFmpegError::UnknownPixelFormat(v) => write!(f, "Unknown pixel format: {:?}", v),
            FFmpegError::PixelFormatNotSupported(v) => write!(f, "Pixel format {:?} is not supported. Supported ones: {:?}. Optimal choice: {:?}", v.0, v.1, v.2),
            FFmpegError::InternalError(e)     => write!(f, "ffmpeg error: {:?}", e),
            FFmpegError::CannotOpenInputFile((url, e))   => write!(f, "Cannot open input file {url}: {e:?}"),
            FFmpegError::CannotOpenOutputFile((url, e))   => write!(f, "Cannot open output file {url}: {e:?}"),
            FFmpegError::OutputIsInput(output, input) => write!(f, "Rendering to {output} would write over {input}, which it's rendered from. Choose another name or folder for the output."),
            FFmpegError::Output(why)                 => write!(f, "{why}"),
        }
    }
}
impl error::Error for FFmpegError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            FFmpegError::InternalError(ref e) => Some(e),
            _ => None
        }
    }
}
impl From<ffmpeg_next::Error> for FFmpegError {
    fn from(err: ffmpeg_next::Error) -> FFmpegError { FFmpegError::InternalError(err) }
}

#[derive(Debug, Clone, Default)]
pub struct VideoInfo {
    pub duration_ms: f64,
    pub frame_count: usize,
    pub fps: f64,
    /// The base frame rate of the video (ffmpeg's `r_frame_rate`, else `fps`), for comparing files, not for timing: the
    /// rate a recording runs at, which the frames it dropped don't change, where they lower its average (`fps`, which
    /// times the frames)
    pub base_fps: f64,
    pub width: u32,
    pub height: u32,
    pub bitrate: f64, // in Mbps
    pub rotation: i32,
    pub created_at: Option<u64>
}

impl<'a> FfmpegProcessor<'a> {
    pub fn from_file(url: &str, mut gpu_decoding: bool, gpu_decoder_index: usize, mut decoder_options: Option<Dictionary>) -> Result<Self, FFmpegError> {
        let mut file = FfmpegPathWrapper::new(url, false).map_err(|e| FFmpegError::CannotOpenInputFile((url.to_string(), e)))?;

        ffmpeg_next::init()?;
        crate::rendering::init_log();

        let hwaccel_device = decoder_options.as_ref().and_then(|x| x.get("hwaccel_device").map(|x| x.to_string()));
        if file.path.starts_with("fd:") {
            match &mut decoder_options {
                Some(dict) => { dict.set("fd", &file.path[3..]); file.path = "fd:".into(); }
                None => { let mut dict = Dictionary::new(); dict.set("fd", &file.path[3..]); file.path = "fd:".into(); decoder_options = Some(dict); }
            }
        }

        let mut input_context = decoder_options.map_or_else(|| format::input(&file.path), |dict| format::input_with_dictionary(&file.path, dict))?;

        // format::context::input::dump(&input_context, 0, Some(file.path));

        let best_video_stream = unsafe {
            let mut decoder: *const ffi::AVCodec = std::ptr::null();
            let index = ffi::av_find_best_stream(input_context.as_mut_ptr(), media::Type::Video.into(), -1i32, -1i32, &mut decoder, 0);
            if index >= 0 && !decoder.is_null() {
                if gpu_decoding && cfg!(target_os = "android") {
                    let decoder_name = match (*decoder).id {
                        ffi::AVCodecID::AV_CODEC_ID_H264 => Some("h264_mediacodec"),
                        ffi::AVCodecID::AV_CODEC_ID_HEVC => Some("hevc_mediacodec"),
                        ffi::AVCodecID::AV_CODEC_ID_VP8  => Some("vp8_mediacodec"),
                        ffi::AVCodecID::AV_CODEC_ID_VP9  => Some("vp9_mediacodec"),
                        ffi::AVCodecID::AV_CODEC_ID_AV1  => Some("av1_mediacodec"),
                        _ => None
                    };
                    if let Some(name) = decoder_name {
                        let name = std::ffi::CString::new(name).unwrap();
                        let mc_ptr = ffi::avcodec_find_decoder_by_name(name.as_ptr());
                        if !mc_ptr.is_null() {
                            decoder = mc_ptr;
                        }
                    }
                }
                Ok((Stream::wrap(&input_context, index as usize), decoder))
            } else {
                Err(Error::StreamNotFound)
            }
        };

        let strm = best_video_stream?;
        let stream = strm.0;
        let decoder = strm.1;

        let decoder_fps = stream.rate().into();

        let mut decoder_ctx = unsafe { codec::context::Context::wrap(ffi::avcodec_alloc_context3(decoder), None) };
        unsafe {
            if ffi::avcodec_parameters_to_context(decoder_ctx.as_mut_ptr(), stream.parameters().as_ptr()) < 0 {
                ::log::error!("avcodec_parameters_to_context failed");
                return Err(FFmpegError::DecoderNotFound);
            }
        }
        decoder_ctx.set_threading(ffmpeg_next::threading::Config { kind: ffmpeg_next::threading::Type::Frame, count: 5 });

        let codec = decoder_ctx.codec().ok_or(FFmpegError::DecoderNotFound)?;

        let mut hw_backend = String::new();
        if gpu_decoding {
            let hw = ffmpeg_hw::init_device_for_decoding(gpu_decoder_index, unsafe { codec.as_ptr() }, &mut decoder_ctx, hwaccel_device.as_deref())?;
            log::debug!("Selected HW backend {:?} ({}) with format {:?}", hw.1, hw.2, hw.3);
            hw_backend = hw.2;
        }
        gpu_decoding = !hw_backend.is_empty();

        Ok(Self {
            _file: file,
            gpu_decoding,
            gpu_device: if !gpu_decoding { None } else { Some(hw_backend) },
            video_codec: None,

            audio_codec: codec::Id::AAC,
            audio_copy: false,

            ost_time_bases: Vec::new(),

            frame_ts: Default::default(),

            ranges_ms: Vec::new(),

            preserve_other_tracks: false,
            camera_tracks: None,
            tracks_not_copied: Vec::new(),

            decoder_fps,

            #[cfg(target_os = "android")]
            android_handles: None,//if gpu_decoding { AndroidHWHandles::init_with_context(&mut decoder_ctx).ok() } else { None },

            video: VideoTranscoder {
                gpu_encoding: true,
                gpu_decoding,
                input_index: stream.index(),
                encoder_params: EncoderParams {
                    options: Dictionary::new(),
                    ..EncoderParams::default()
                },
                decoder: Some(decoder_ctx.decoder().open_as(codec)?.video()?),
                ..VideoTranscoder::default()
            },

            input_context,
            input_url: url.to_owned(),
        })
    }

    pub fn render(&mut self, output_folder: &str, output_filename: &str, output_size: (u32, u32), bitrate: Option<f64>, cancel_flag: Arc<AtomicBool>, pause_flag: Arc<AtomicBool>) -> Result<(), FFmpegError> {
        let output_url = filesystem::get_file_url(output_folder, output_filename, true);
        let mut file = FfmpegPathWrapper::new(&output_url, true).map_err(|e| FFmpegError::CannotOpenOutputFile((output_url.to_string(), e)))?;

        let mut stream_mapping: Vec<isize> = vec![0; self.input_context.nb_streams() as _];
        let mut ist_time_bases = vec![Rational(0, 0); self.input_context.nb_streams() as _];
        // Copied audio streams of raw samples, which are cut at the exact sample: size of a sample (all channels) and the rate
        let mut raw_pcm_streams = vec![None; self.input_context.nb_streams() as _];
        self.ost_time_bases.resize(self.input_context.nb_streams() as _, Rational(0, 0));
        let mut atranscoders = HashMap::new();
        let mut audio_streams = Vec::new();
        let mut output_index = 0usize;

        let mut start_ms = None;
        let mut end_ms = None;

        if let Some(first_range) = self.ranges_ms.first() {
            if let Some(start) = first_range.0 {
                start_ms = Some(start);
                let position = (start as i64).rescale((1, 1000), rescale::TIME_BASE);
                self.input_context.seek(position, ..position)?;
            }
            if let Some(end) = first_range.1 {
                end_ms = Some(end);
            }
            self.ranges_ms.remove(0);
        }

        let output_filename = output_filename.strip_suffix(".tmp").unwrap_or(output_filename);

        let mut output_options = Dictionary::new();
        let mut output_format = if let Some(pos) = output_filename.rfind('.') { &output_filename[pos+1..] } else { "mp4" }.to_ascii_lowercase();
        if file.path.starts_with("fd:") {
            output_options.set("fd", &file.path[3..]);
            file.path = "fd:".into();
        }
        if output_format == "mkv" { output_format = String::from("matroska"); }

        let mut octx = if output_format == "exr" || output_format == "png" {
            format::output_with(&file.path, output_options)
        } else {
            format::output_as_with(&file.path, &output_format, output_options)
        }?;

        // Copy metadata
        let mut metadata = self.input_context.metadata().to_owned();
        for (k, v) in self.video.encoder_params.metadata.iter() {
            metadata.set(k, v);
        }

        // FFmpeg writes the data tracks it doesn't know with an empty sample description, which nothing can read. From an MP4/MOV
        // to an MP4/MOV, they're left out, and the camera's tracks are appended to the output once it's written (`camera_tracks`)
        let is_mp4_input = self.input_context.format().name().split(',').any(|x| x == "mov" || x == "mp4");
        let mut copy_camera_tracks = self.preserve_other_tracks && is_mp4_input && (output_format == "mov" || output_format == "mp4");
        let codec_tag = |stream: &Stream| unsafe { (*stream.parameters().as_ptr()).codec_tag }.to_le_bytes();
        // The only data tracks FFmpeg writes a sample description for
        let ffmpeg_writes = |tag: &[u8; 4]| tag == b"tmcd" || tag == b"gpmd";
        // A timecode track stores its frames per second in one byte: above 255 fps, the mov/mp4 muxer fails as it writes the moov,
        // once the whole video is rendered (`mov_write_tmcd_tag`, "fps 400 is too large"), so the output gets no timecode then
        let tmcd_fps_too_high = |rate: Rational| (output_format == "mov" || output_format == "mp4") && rate.denominator() != 0 && f64::from(rate).round() > 255.0;
        // Only the tracks that can be appended are left out, FFmpeg writes the others as before. Why they can't be
        let not_copyable: HashMap<u32, String> = if copy_camera_tracks {
            let track_ids: Vec<u32> = self.input_context.streams()
                .filter(|x| x.parameters().medium() == media::Type::Data && !ffmpeg_writes(&codec_tag(x)))
                .map(|x| x.id() as u32).collect();
            super::camera_tracks::check_tracks(&self.input_url, &track_ids).unwrap_or_else(|e| {
                // Nor its metadata
                log::warn!("Can't read the input to copy its camera tracks: {e:?}");
                copy_camera_tracks = false;
                track_ids.iter().map(|id| (*id, e.to_string())).collect()
            })
        } else {
            HashMap::new()
        };
        let mut camera_track_ids = Vec::new();

        for (i, stream) in self.input_context.streams().enumerate() {
            // Copy timecode from stream if global metadata doesn't have it
            if let Some(timecode) = stream.metadata().get("timecode") {
                if metadata.get("timecode").is_none() {
                    metadata.set("timecode", timecode);
                }
            }

            let medium = stream.parameters().medium();
            if medium != media::Type::Audio && medium != media::Type::Video && (!self.preserve_other_tracks || medium != media::Type::Data) {
                stream_mapping[i] = -1;
                continue;
            }
            // Limit to first video stream
            if medium == media::Type::Video && self.video.output_index.is_some() {
                stream_mapping[i] = -1;
                continue;
            }
            stream_mapping[i] = output_index as isize;
            ist_time_bases[i] = stream.time_base();

            if medium == media::Type::Video {
                self.video.input_index = i;
                self.video.output_index = Some(output_index);

                let codec = encoder::find_by_name(self.video_codec.as_ref().ok_or(Error::EncoderNotFound)?).ok_or(Error::EncoderNotFound)?;
                unsafe {
                    if !codec.as_ptr().is_null() {
                        self.video.codec_supported_formats = super::ffmpeg_hw::codec_pix_formats(std::ptr::null(), codec.as_ptr());
                        log::debug!("Codec formats: {:?}", self.video.codec_supported_formats);
                    }
                }
                let mut out_stream = octx.add_stream(codec)?;
                self.video.encoder_params.codec = Some(codec);

                self.video.encoder_params.frame_rate = Some(stream.avg_frame_rate());
                self.video.encoder_params.time_base = Some(stream.rate().invert());

                out_stream.set_rate(stream.rate());
                out_stream.set_time_base(stream.time_base());
                out_stream.set_avg_frame_rate(stream.avg_frame_rate());

                output_index += 1;
            } else if medium == media::Type::Audio && self.audio_codec != codec::Id::None {
                // Only "Copy" copies it, also with "Preserve other tracks" (which makes a .mov for it, see `RenderQueue::get_output_filename`)
                let mut copy_audio = self.audio_copy;
                if copy_audio && !Self::can_copy_stream(&octx, &stream.parameters()) {
                    log::warn!("Output format {} can't store audio codec {:?}, transcoding it to {:?} instead", octx.format().name(), stream.parameters().id(), self.audio_codec);
                    copy_audio = false;
                }
                if copy_audio {
                    // Direct stream copy
                    let mut ost = octx.add_stream(encoder::find(codec::Id::None))?;
                    ost.set_parameters(stream.parameters());
                    // We need to set codec_tag to 0 lest we run into incompatible codec tag issues when muxing into a different container format.
                    unsafe { (*ost.parameters().as_mut_ptr()).codec_tag = 0; }
                    raw_pcm_streams[i] = raw_pcm_layout(&stream.parameters());
                } else {
                    // Transcode audio
                    atranscoders.insert(i, AudioTranscoder::new(self.audio_codec, &stream, &mut octx, output_index as _)?);
                }
                audio_streams.push(i);
                output_index += 1;
            } else if self.preserve_other_tracks && medium == media::Type::Data {
                let tag = codec_tag(&stream);
                let id = stream.id() as u32;
                let not_copyable = not_copyable.get(&id);
                if copy_camera_tracks && !ffmpeg_writes(&tag) && not_copyable.is_none() {
                    // Appended once the output is written
                    camera_track_ids.push(id);
                    stream_mapping[i] = -1;
                } else if output_format == "mp4" && &tag != b"gpmd" {
                    // Not in the mp4 muxer's tags. It makes the timecode track from the `timecode` metadata
                    log::info!("Not copying data stream {i} ({}), the .mp4 format can't store it", String::from_utf8_lossy(&tag));
                    if &tag != b"tmcd" {
                        let why = not_copyable.cloned().unwrap_or_else(|| "the .mp4 format can't store it".into());
                        self.tracks_not_copied.push(format!("Track {id} ({}): {why}", String::from_utf8_lossy(&tag)));
                    }
                    stream_mapping[i] = -1;
                } else if &tag == b"tmcd" && tmcd_fps_too_high(stream.avg_frame_rate()) {
                    // In a .mov. The muxer makes the timecode track from the `timecode` metadata instead, at the video's frame rate
                    // (left out below if that's too high as well)
                    log::info!("Not copying data stream {i} (tmcd), its frame rate is above 255 fps");
                    stream_mapping[i] = -1;
                } else {
                    if let Some(why) = not_copyable {
                        log::warn!("Can't copy track {id} of the camera ({why}), FFmpeg writes it");
                    }
                    // Direct stream copy
                    let mut ost = octx.add_stream(encoder::find(codec::Id::None))?;
                    ost.set_parameters(stream.parameters());
                    ost.set_avg_frame_rate(stream.avg_frame_rate());
                    output_index += 1;
                }
            } else {
                // Not in the output (audio disabled)
                stream_mapping[i] = -1;
            }
        }
        if self.preserve_other_tracks && start_ms.is_none() {
            // The other tracks are kept as they are, so keep their timing relative to the video too: shift all streams by the start
            // of the file (like ffmpeg does) rather than by the first video frame, which would push earlier audio and data out
            let start_time = unsafe { (*self.input_context.as_ptr()).start_time };
            if start_time != ffi::AV_NOPTS_VALUE {
                self.frame_ts.first = Some(start_time.rescale(rescale::TIME_BASE, (1, 1000000)));
            }
        }
        if copy_camera_tracks {
            // The streams FFmpeg writes, in its order: they get the output indexes in the order of the input
            let ffmpeg_track_ids = self.input_context.streams().filter(|x| stream_mapping[x.index()] >= 0).map(|x| x.id() as u32).collect();
            self.camera_tracks = Some(super::camera_tracks::CameraTracks { track_ids: camera_track_ids, ffmpeg_track_ids, start_us: self.frame_ts.first.unwrap_or_default() });
        }
        let mut updated_creation_time = None;
        if let Some(start_ms) = start_ms {
            if start_ms > 0.0 {
                for (k, v) in metadata.iter() {
                    if k == "creation_time" {
                        if let Ok(v) = chrono::DateTime::parse_from_rfc3339(v) {
                            if let Some(v) = v.checked_add_signed(chrono::TimeDelta::try_milliseconds(start_ms.round() as i64).unwrap()) {
                                updated_creation_time = Some(v.to_rfc3339());
                            }
                        }
                        break;
                    }
                }
            }
        }
        if let Some(updated_creation_time) = updated_creation_time {
            metadata.set("creation_time", &updated_creation_time);
        }
        let video_fps = self.video.output_index.and_then(|i| octx.stream(i)).map(|x| x.avg_frame_rate());
        if let Some(fps) = video_fps.filter(|x| metadata.get("timecode").is_some() && tmcd_fps_too_high(*x)) {
            log::warn!("Not writing the timecode, a .{output_format} timecode track can't be above 255 fps (the video is {:.3} fps)", f64::from(fps));
            self.tracks_not_copied.push(format!("Timecode: the .{output_format} format can't store it above 255 fps"));
            metadata = metadata.iter().filter(|(k, _)| *k != "timecode").collect();
        }
        log::debug!("Output metadata: {:?}", &metadata);
        octx.set_metadata(metadata);
        // Header will be written after video encoder is initalized, in ffmpeg_video.rs:init_encoder, or with its first packet
        // for the encoders that need the parameter sets from the bitstream (see `VideoTranscoder::header_written`)

        let process_stream = |atranscoders: &mut HashMap<usize, AudioTranscoder>, octx: &mut format::context::Output, mut packet: ffmpeg_next::Packet, start_ms: Option<f64>, ist_index: usize, ost_index: isize, ost_time_base: Rational, frame_ts: &mut FrameTimestamps| -> Result<(), Error> {
            let ist_time_base = ist_time_bases[ist_index];
            match atranscoders.get_mut(&ist_index) {
                Some(atranscoder) => {
                    packet.rescale_ts(ist_time_base, atranscoder.decoder.time_base());
                    atranscoder.decoder.send_packet(&packet)?;
                    atranscoder.receive_and_process_decoded_frames(octx, ost_time_base, start_ms, frame_ts)?;
                }
                None => {
                    // Direct stream copy, on the same output timeline as the video
                    let timestamp_us = packet.pts().or(packet.dts()).map(|x| x.rescale(ist_time_base, (1, 1000000)));
                    let raw_samples = raw_pcm_streams[ist_index];
                    let duration_us = match raw_samples {
                        Some((sample_size, rate)) => ((packet.size() / sample_size) as i64).rescale((1, rate), (1, 1000000)),
                        None => packet.duration().rescale(ist_time_base, (1, 1000000))
                    };
                    let Some(placement) = frame_ts.place(ist_index, timestamp_us, duration_us, raw_samples.is_some(), start_ms) else {
                        return Ok(());
                    };
                    let mut sample_count = None;
                    if let Some((sample_size, rate)) = raw_samples {
                        // Raw samples, cut to the samples that are written
                        let samples = packet.size() / sample_size;
                        let skip = (placement.skip_us.rescale((1, 1000000), (1, rate)).max(0) as usize).min(samples);
                        let count = (placement.duration_us.rescale((1, 1000000), (1, rate)).max(0) as usize).min(samples - skip);
                        if count == 0 {
                            return Ok(());
                        }
                        unsafe {
                            use ffmpeg_next::packet::Mut;
                            let packet = packet.as_mut_ptr();
                            (*packet).data = (*packet).data.add(skip * sample_size);
                            (*packet).size = (count * sample_size) as i32;
                        }
                        sample_count = Some((count as i64, rate));
                    }

                    packet.rescale_ts(ist_time_base, ost_time_base);
                    if let Some(timestamp_us) = timestamp_us {
                        // The shift is the same for the whole range, so the packets keep their exact spacing (and pts-dts delay)
                        let shift = (placement.ts_us - timestamp_us).rescale((1, 1000000), ost_time_base);
                        packet.set_pts(packet.pts().map(|x| x + shift));
                        packet.set_dts(packet.dts().map(|x| x + shift));
                    } else {
                        let ts = placement.ts_us.rescale((1, 1000000), ost_time_base);
                        packet.set_pts(Some(ts));
                        packet.set_dts(Some(ts));
                    }
                    if let Some((count, rate)) = sample_count {
                        packet.set_duration(count.rescale((1, rate), ost_time_base));
                    }
                    packet.set_position(-1);
                    packet.set_stream(ost_index as _);
                    packet.write_interleaved(octx)?;
                }
            }
            Ok(())
        };

        // Audio and data packets wait for the muxer header (written once the video encoder is initialized, or with its first packet),
        // for the first video frame of the range, which sets where the range starts in the output, and past the end of the range,
        // for the video to end
        let must_wait = |frame_ts: &FrameTimestamps, header_written: bool, packet: &ffmpeg_next::Packet, ist_index: usize, end_ms: Option<f64>| -> bool {
            !header_written || frame_ts.first.is_none() || (frame_ts.range_end.is_none() && end_ms.is_some_and(|end| {
                packet.pts().or(packet.dts()).is_some_and(|ts| ts.rescale(ist_time_bases[ist_index], (1, 1000000)) as f64 / 1000.0 > end)
            }))
        };

        let mut any_encoded = false;
        let mut pending_packets: Vec<(ffmpeg_next::Packet, usize, isize)> = Vec::new();

        loop {
            let mut encoding_video = true;

            for (stream, mut packet) in self.input_context.packets() {
                let ist_index = stream.index();
                let ost_index = stream_mapping[ist_index];
                if ost_index < 0 {
                    continue;
                }

                if ist_index == self.video.input_index {
                    if encoding_video {
                        {
                            let decoder = self.video.decoder.as_mut().ok_or(Error::DecoderNotFound)?;
                            packet.rescale_ts(stream.time_base(), (1, 1000000)); // rescale to microseconds
                            if let Err(err) = decoder.send_packet(&packet) {
                                if self.gpu_decoding && FFMPEG_LOG.read().contains("failed to decode picture") {
                                    return Err(FFmpegError::GPUDecodingFailed);
                                }
                                if !any_encoded {
                                    return Err(err.into());
                                }
                            }
                        }

                        match self.video.receive_and_process_video_frames(output_size, bitrate, Some(&mut octx), &mut self.ost_time_bases, start_ms, end_ms, &mut self.frame_ts) {
                            Ok(encoding_status) => {
                                if encoding_status == Status::Finish {
                                    encoding_video = false;
                                    self.frame_ts.end_range();
                                }
                                // Decoding errors stop the export only until the first frame reached the encoder. Not tied to the header,
                                // which for some encoders waits for their first packet, a few frames later
                                if self.video.encoder.is_some() {
                                    any_encoded = true;
                                }
                                // The other streams can be written once the header is written
                                if self.video.header_written {
                                    for (packet, ist_index, ost_index) in std::mem::take(&mut pending_packets) {
                                        if must_wait(&self.frame_ts, self.video.header_written, &packet, ist_index, end_ms) {
                                            pending_packets.push((packet, ist_index, ost_index));
                                        } else if !self.frame_ts.is_finished(ist_index) {
                                            let ost_time_base = self.ost_time_bases[ost_index as usize];
                                            process_stream(&mut atranscoders, &mut octx, packet, start_ms, ist_index, ost_index, ost_time_base, &mut self.frame_ts)?;
                                        }
                                    }
                                }
                                while pause_flag.load(Relaxed) {
                                    std::thread::sleep(std::time::Duration::from_millis(100));
                                }
                            },
                            Err(e) => {
                                if !any_encoded {
                                    return Err(e);
                                }
                            }
                        }
                    }
                } else if !self.frame_ts.is_finished(ist_index) {
                    // Audio, or a data track kept by "Preserve other tracks"
                    if must_wait(&self.frame_ts, self.video.header_written, &packet, ist_index, end_ms) {
                        pending_packets.push((packet, ist_index, ost_index));
                        continue;
                    }
                    let ost_time_base = self.ost_time_bases[ost_index as usize];
                    process_stream(&mut atranscoders, &mut octx, packet, start_ms, ist_index, ost_index, ost_time_base, &mut self.frame_ts)?;
                }
                let audio_finished = audio_streams.iter().all(|x| self.frame_ts.is_finished(*x));
                if (!encoding_video && audio_finished) || cancel_flag.load(Relaxed) {
                    break;
                }
            }
            if !self.ranges_ms.is_empty() && !cancel_flag.load(Relaxed) {
                let next_range = self.ranges_ms.remove(0);
                if let Some(start) = next_range.0 {
                    start_ms = Some(start);
                    let position = (start as i64).rescale((1, 1000), rescale::TIME_BASE);
                    self.input_context.seek(position, ..position)?;
                    self.frame_ts.start_next_range();
                }
                end_ms = next_range.1;
                // Past the end of a range that the video didn't reach
                pending_packets.clear();
                continue;
            } else {
                break;
            }
        }

        // Flush encoders and decoders.
        {
            self.video.decoder.as_mut().ok_or(Error::DecoderNotFound)?.send_eof()?;
            // self.video.decoder.as_mut().ok_or(Error::DecoderNotFound)?.flush();
            let encoding_status = self.video.receive_and_process_video_frames(output_size, bitrate, Some(&mut octx), &mut self.ost_time_bases, start_ms, end_ms, &mut self.frame_ts)?;
            if encoding_status == Status::Finish && self.frame_ts.range_end.is_none() {
                self.frame_ts.end_range();
            }
            self.video.encoder.as_mut().ok_or(Error::EncoderNotFound)?.send_eof()?;
            if let Err(e) = self.video.receive_and_process_encoded_packets(&mut octx, &mut self.ost_time_bases) {
                log::error!("Failed to flush last packet: {e:?}");
            }
        }
        // The audio that waited for the video to reach the end of the last range, or for the header, when the video only got
        // there in the decoder or encoder flush
        for (packet, ist_index, ost_index) in pending_packets {
            if !must_wait(&self.frame_ts, self.video.header_written, &packet, ist_index, end_ms) && !self.frame_ts.is_finished(ist_index) {
                let ost_time_base = self.ost_time_bases[ost_index as usize];
                process_stream(&mut atranscoders, &mut octx, packet, start_ms, ist_index, ost_index, ost_time_base, &mut self.frame_ts)?;
            }
        }
        if self.audio_codec != codec::Id::None {
            for transcoder in atranscoders.values_mut() {
                let ost_time_base = self.ost_time_bases[transcoder.ost_index];
                transcoder.flush(&mut octx, ost_time_base, start_ms, &mut self.frame_ts)?;
            }
        }

        octx.write_trailer()?;

        Ok(())
    }

    pub fn start_decoder_only(&mut self, mut ranges: Vec<(f64, f64)>, cancel_flag: Arc<AtomicBool>) -> Result<(), FFmpegError> {
        let mut start_ms = None;
        let mut end_ms = None;

        if let Some(first_range) = ranges.first() {
            start_ms = Some(first_range.0);
            end_ms = Some(first_range.1);
            let position = (first_range.0 as i64).rescale((1, 1000), rescale::TIME_BASE);
            self.input_context.seek(position, ..position)?;
            ranges.remove(0);
        }

        self.video.decode_only = true;

        for (i, stream) in self.input_context.streams().enumerate() {
            if stream.parameters().medium() == media::Type::Video {
                self.video.input_index = i;

                // TODO this doesn't work for some reason
                // let c_name = CString::new("resize").unwrap();
                // let c_val = CString::new("1280x720").unwrap();
                // unsafe { ffi::av_opt_set((*codec.as_mut_ptr()).priv_data, c_name.as_ptr(), c_val.as_ptr(), 1); }

                self.video.encoder_params.frame_rate = self.video.decoder.as_ref().unwrap().frame_rate();
                self.video.encoder_params.time_base = Some(stream.rate().invert());
                break;
            }
        }

        let mut any_encoded = false;
        loop {
            for (stream, mut packet) in self.input_context.packets() {
                let ist_index = stream.index();

                if ist_index == self.video.input_index {
                    let decoder = self.video.decoder.as_mut().ok_or(Error::DecoderNotFound)?;
                    packet.rescale_ts(stream.time_base(), (1, 1000000)); // rescale to microseconds

                    if let Err(err) = decoder.send_packet(&packet) {
                        ::log::error!("Decoder error {:?}", err);
                        if self.gpu_decoding && FFMPEG_LOG.read().contains("failed to decode picture") {
                            return Err(FFmpegError::GPUDecodingFailed);
                        }
                        if !any_encoded {
                            return Err(err.into());
                        }
                    }
                    match self.video.receive_and_process_video_frames((0, 0), None, None, &mut self.ost_time_bases, start_ms, end_ms, &mut self.frame_ts) {
                        Ok(encoding_status) => {
                            any_encoded = true;
                            if encoding_status == Status::Finish || cancel_flag.load(Relaxed) {
                                break;
                            }
                        },
                        Err(e) => {
                            ::log::error!("Encoder error {:?}", e);
                            if !any_encoded {
                                return Err(e);
                            }
                        }
                    }
                }
            }
            if !ranges.is_empty() && !cancel_flag.load(Relaxed) {
                let next_range = ranges.remove(0);
                let position = (next_range.0 as i64).rescale((1, 1000), rescale::TIME_BASE);
                self.input_context.seek(position, ..position)?;
                end_ms = Some(next_range.1);
                continue;
            } else {
                break;
            }
        }

        // Flush decoder.
        self.video.decoder.as_mut().ok_or(Error::DecoderNotFound)?.send_eof()?;
        self.video.receive_and_process_video_frames((0, 0), None, None, &mut self.ost_time_bases, start_ms, end_ms, &mut self.frame_ts)?;

        Ok(())
    }

    pub fn on_frame<F>(&mut self, cb: F) where F: FnMut(i64, &mut frame::Video, Option<&mut frame::Video>, &mut ffmpeg_video_converter::Converter, &mut ffmpeg_video::RateControl) -> Result<(), FFmpegError> + 'a {
        self.video.on_frame_callback = Some(Box::new(cb));
    }
    pub fn on_encoder_initialized<F>(&mut self, cb: F) where F: FnMut(&encoder::video::Video) -> Result<(), FFmpegError> + 'a {
        self.video.on_encoder_initialized = Some(Box::new(cb));
    }

    /// Whether the output format can store `params` as a direct stream copy, in a form that's widely readable.
    /// The codec has to be in the muxer's own tag table (mov stores the others with Microsoft-style tags that hardly anything reads),
    /// but muxers also reject some of the codecs listed there when they're set up (mov takes FLAC, Opus and TrueHD only in .mp4,
    /// and TrueHD there only as experimental), so this sets up the same muxer for real, on a scratch context that writes nowhere
    fn can_copy_stream(octx: &format::context::Output, params: &codec::Parameters) -> bool {
        unsafe extern "C" fn write_nowhere(_: *mut std::ffi::c_void, _: *const u8, size: i32) -> i32 { size }
        // Seekable, mov refuses non-seekable output unless it's fragmented
        unsafe extern "C" fn seek_nowhere(_: *mut std::ffi::c_void, offset: i64, whence: i32) -> i64 {
            if whence & ffi::AVSEEK_SIZE != 0 { -1 } else { offset }
        }

        // 1: in the table, 0: not in it, negative: the muxer has no table, only setting it up can tell
        if unsafe { ffi::avformat_query_codec(octx.format().as_ptr(), params.id().into(), ffi::FF_COMPLIANCE_NORMAL) } == 0 {
            return false;
        }

        let mut ctx = std::ptr::null_mut();
        unsafe { ffi::avformat_alloc_output_context2(&mut ctx, (*octx.as_ptr()).oformat, std::ptr::null(), (*octx.as_ptr()).url); }
        !ctx.is_null() && unsafe {
            // The muxer logs why it refuses the codec, which isn't an error of the render
            (*ctx).opaque = &super::SILENT_LOG as *const u8 as *mut std::ffi::c_void;

            const BUFFER_SIZE: i32 = 4096;
            let buffer = ffi::av_malloc(BUFFER_SIZE as usize) as *mut u8;
            if !buffer.is_null() {
                (*ctx).pb = ffi::avio_alloc_context(buffer, BUFFER_SIZE, 1, std::ptr::null_mut(), None, Some(write_nowhere), Some(seek_nowhere));
                if (*ctx).pb.is_null() {
                    ffi::av_free(buffer as *mut _);
                }
            }
            let stream = ffi::avformat_new_stream(ctx, std::ptr::null());
            let mut can_copy = !(*ctx).pb.is_null() && !stream.is_null() && ffi::avcodec_parameters_copy((*stream).codecpar, params.as_ptr()) >= 0;
            if can_copy {
                // Same as the real output
                (*(*stream).codecpar).codec_tag = 0;
                can_copy = ffi::avformat_write_header(ctx, std::ptr::null_mut()) >= 0;
                // FFmpeg stores PCM in ISO MP4 as ISO/IEC 23003-5 'ipcm'/'fpcm', which few applications read yet
                // (QuickTime .mov has its own PCM sample entries, read everywhere)
                let tag = (*(*stream).codecpar).codec_tag;
                can_copy &= tag != u32::from_le_bytes(*b"ipcm") && tag != u32::from_le_bytes(*b"fpcm");
            }
            if !(*ctx).pb.is_null() {
                ffi::av_freep(&mut (*(*ctx).pb).buffer as *mut *mut u8 as *mut std::ffi::c_void);
                ffi::avio_context_free(&mut (*ctx).pb);
            }
            ffi::avformat_free_context(ctx);
            can_copy
        }
    }

    /// Rotation (degrees clockwise) to display the picture, and whether it's also mirrored, from an FFmpeg display matrix (stream or frame side data).
    /// Gyroflow can't mirror, so for a mirrored matrix this is the rotation that keeps the picture upright, only mirrored left-right.
    /// FFmpeg builds those as a rotation followed by a horizontal flip of the displayed picture (av_exif_orientation_to_matrix),
    /// so the flip is undone first: the rotation of the whole matrix reads e.g. EXIF orientation 2 (mirrored) as 180° (upside down)
    fn display_matrix_rotation(display_matrix: &[u8]) -> Option<(f64, bool)> {
        if display_matrix.len() != 9 * 4 { return None; }
        let mut matrix = [0i32; 9];
        for (x, bytes) in matrix.iter_mut().zip(display_matrix.chunks_exact(4)) {
            *x = i32::from_ne_bytes(bytes.try_into().ok()?);
        }
        let mirrored = (matrix[0] as i64) * (matrix[4] as i64) - (matrix[1] as i64) * (matrix[3] as i64) < 0;
        if mirrored {
            unsafe { ffi::av_display_matrix_flip(matrix.as_mut_ptr(), 1, 0); }
        }
        let rotation = -unsafe { ffi::av_display_rotation_get(matrix.as_ptr()) };
        rotation.is_finite().then_some((rotation, mirrored))
    }

    /// Rotation attached to the decoded frames rather than to the stream: the EXIF orientation of images (e.g. DNG/TIFF/JPEG image sequences)
    /// or an H.264/HEVC display orientation SEI, which FFmpeg exports as a per-frame display matrix.
    /// The player (MDK) applies it on its own, so it has to be known to keep everything else consistent.
    /// Decodes the first frame, but only for codecs that can have one, and for H.264/HEVC only when the first packet has that SEI
    fn first_frame_rotation(context: &mut format::context::Input, stream_index: usize, decoder: &mut ffmpeg_next::decoder::Video) -> Option<(f64, bool)> {
        use codec::Id;
        let codec_id = decoder.id();
        let is_video = matches!(codec_id, Id::H264 | Id::HEVC);
        if !is_video && !matches!(codec_id, Id::TIFF | Id::MJPEG | Id::PNG | Id::WEBP | Id::JPEGXL | Id::CRI) {
            return None;
        }
        let extradata = unsafe {
            let params = context.stream(stream_index)?.parameters().as_ptr();
            if (*params).extradata.is_null() { Vec::new() } else { std::slice::from_raw_parts((*params).extradata, (*params).extradata_size.max(0) as usize).to_vec() }
        };

        let get_rotation = |frame: &frame::Video| Self::display_matrix_rotation(frame.side_data(frame::side_data::Type::DisplayMatrix)?.data());
        let mut frame = frame::Video::empty();
        // Frame reordering can hold the first frame back by a few packets
        for (i, (_, packet)) in context.packets().filter(|(stream, _)| stream.index() == stream_index).take(16).enumerate() {
            if i == 0 && is_video && !has_display_orientation_sei(codec_id, &extradata, packet.data().unwrap_or_default()) {
                return None;
            }
            decoder.send_packet(&packet).ok()?;
            match decoder.receive_frame(&mut frame) {
                Ok(()) => return get_rotation(&frame),
                Err(ffmpeg_next::Error::Other { errno }) if errno == ffmpeg_next::util::error::EAGAIN => { }
                Err(_) => return None
            }
        }
        decoder.send_eof().ok()?;
        decoder.receive_frame(&mut frame).ok()?;
        get_rotation(&frame)
    }

    pub fn get_video_info(url: &str) -> Result<VideoInfo, ffmpeg_next::Error> {
        let mut file = FfmpegPathWrapper::new(url, false).map_err(|_| ffmpeg_next::Error::ProtocolNotFound)?;
        let mut dict = Dictionary::new();
        if file.path.starts_with("fd:") {
            dict.set("fd", &file.path[3..]);
            file.path = "fd:".into();
        }

        let mut context = format::input_with_dictionary(&file.path, dict)?;
        let created_at = context.metadata().get("creation_time").and_then(|x| chrono::DateTime::parse_from_rfc3339(x).ok()).map(|x| x.timestamp_millis() as u64 / 1000);
        if let Some(stream) = context.streams().best(media::Type::Video) {
            let codec = codec::context::Context::from_parameters(stream.parameters())?;
            if let Ok(mut video) = codec.decoder().video() {
                let mut bitrate = video.bit_rate();
                if bitrate == 0 { bitrate = context.bit_rate() as usize; }

                let fps = Self::average_frame_rate(&stream);
                let base_fps = Self::base_frame_rate(&stream).unwrap_or(fps);
                let duration_s = Self::video_duration_s(&context, &stream).unwrap_or_default();
                // Without a frame count in the container, its duration at its rate, as the player counts them (`MDKPlayer.cpp`)
                let mut frames = stream.frames() as usize;
                if frames == 0 { frames = (duration_s * fps) as usize; }

                let rotate_tag = stream.metadata().get("rotate").and_then(|x| x.parse::<f64>().ok()).map(|x| (x, false));
                let display_matrix = stream.side_data().find(|x| x.kind() == codec::packet::side_data::Type::DisplayMatrix).and_then(|x| Self::display_matrix_rotation(x.data()));
                let stream_rotation = rotate_tag.filter(|x| x.0 != 0.0).or(display_matrix).filter(|(theta, mirrored)| *theta != 0.0 || *mirrored);

                let stream_index = stream.index();
                let duration_ms = duration_s * 1000.0;
                let (width, height) = (video.width(), video.height());

                // Same as the player: the stream rotation, otherwise the one on the frames
                let (mut theta, mirrored) = stream_rotation.or_else(|| Self::first_frame_rotation(&mut context, stream_index, &mut video)).unwrap_or_default();
                if mirrored {
                    log::warn!("{url} is mirrored for display, which isn't supported, the output won't be mirrored");
                }
                theta = theta.round();
                theta -= 360.0 * (theta / 360.0 + 0.9 / 360.0).floor();

                return Ok(VideoInfo {
                    duration_ms,
                    frame_count: frames,
                    fps,
                    base_fps,
                    width,
                    height,
                    bitrate: bitrate as f64 / 1024.0 / 1024.0,
                    rotation: theta as i32,
                    created_at
                });
            }
        }
        Err(ffmpeg_next::Error::StreamNotFound)
    }

    /// The frame rate of a video stream as the player (MDK) and gyroflow-core read it: the average one, ffmpeg's
    /// `avg_frame_rate` (the stream's frames over their duration), so a clip added to the queue maps its frames to the
    /// same times as in the app. `r_frame_rate`, the base rate its timestamps are at, is the same for a constant frame
    /// rate, but slower or faster than the frames of a variable one (an iPhone clip: 30 against 30.0021), and can be
    /// the stream's time base (raw H.264/HEVC); it stands in only when ffmpeg can't tell the average (0/0). 0 when
    /// neither is known
    fn average_frame_rate(stream: &Stream) -> f64 {
        [stream.avg_frame_rate(), stream.rate()].into_iter().find(|rate| rate.numerator() > 0 && rate.denominator() > 0).map_or(0.0, f64::from)
    }

    /// The base frame rate of a video stream, ffmpeg's `r_frame_rate`: the rate its timestamps are at. For a recording
    /// it's the rate the camera records at, the same for every part of it. The MP4 demuxer takes it from the frame
    /// duration when all frames but the last have the same one, else ffmpeg from the first frames' durations (their
    /// common divisor, or the standard rate nearest them), else from the codec, so frames dropped from a part (longer
    /// durations) leave it as it is, where they lower the average. `None` when ffmpeg can't tell it
    fn base_frame_rate(stream: &Stream) -> Option<f64> {
        let rate = stream.rate();
        (rate.numerator() > 0 && rate.denominator() > 0).then(|| f64::from(rate))
    }

    /// The duration of a video stream in seconds, as the file declares it: the stream's own (MP4, MOV, MXF, MPEG-TS),
    /// else the duration its track's statistics give (the `DURATION` tag the Matroska and WebM muxers write per track,
    /// ffmpeg's and mkvmerge's, which is the player's, MDK's, value there), else the container's (FLV's `onMetaData`, a
    /// Matroska file without the tags), which spans all its streams. `None` when nothing declares one: a raw H.264/HEVC
    /// stream, or a recording that never got its duration written. ffmpeg doesn't know such a duration either, and the
    /// player has 0; a raw stream's frames have no time at all to be rendered by
    fn video_duration_s(context: &format::context::Input, stream: &Stream) -> Option<f64> {
        if stream.duration() > 0 {
            return Some(stream.duration() as f64 * f64::from(stream.time_base()));
        }
        let tag = stream.metadata().iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("DURATION") || key.get(..9).is_some_and(|key| key.eq_ignore_ascii_case("DURATION-")))
            .and_then(|(_, value)| Self::duration_tag_s(value));
        tag.filter(|s| *s > 0.0).or_else(|| (context.duration() > 0).then(|| context.duration() as f64 / f64::from(ffi::AV_TIME_BASE)))
    }

    /// A Matroska `DURATION` tag, `HH:MM:SS.nnnnnnnnn`, in seconds
    fn duration_tag_s(value: &str) -> Option<f64> {
        let mut parts = value.trim().splitn(3, ':');
        let (hours, minutes, seconds) = (parts.next()?.parse::<u64>().ok()?, parts.next()?.parse::<u64>().ok()?, parts.next()?.parse::<f64>().ok()?);
        (minutes < 60 && (0.0..60.0).contains(&seconds)).then(|| (hours * 3600 + minutes * 60) as f64 + seconds)
    }
}

/// Size of one sample (all channels) and sample rate of raw PCM, which can be cut at any sample
fn raw_pcm_layout(params: &codec::Parameters) -> Option<(usize, i32)> {
    use codec::Id;
    let bytes = match params.id() {
        Id::PCM_S8 | Id::PCM_U8 => 1,
        Id::PCM_S16LE | Id::PCM_S16BE | Id::PCM_U16LE | Id::PCM_U16BE => 2,
        Id::PCM_S24LE | Id::PCM_S24BE | Id::PCM_U24LE | Id::PCM_U24BE => 3,
        Id::PCM_S32LE | Id::PCM_S32BE | Id::PCM_U32LE | Id::PCM_U32BE | Id::PCM_F32LE | Id::PCM_F32BE => 4,
        Id::PCM_S64LE | Id::PCM_S64BE | Id::PCM_F64LE | Id::PCM_F64BE => 8,
        _ => return None
    };
    let (channels, rate) = unsafe { ((*params.as_ptr()).ch_layout.nb_channels, (*params.as_ptr()).sample_rate) };
    (channels > 0 && rate > 0).then_some((bytes * channels as usize, rate))
}

/// Whether an H.264/HEVC access unit has a display orientation SEI (payloadType 47), the only source of a frame display matrix in these streams.
/// Takes both packet formats: NAL units prefixed with their size (mp4, the size length is in the avcC/hvcC extradata) and Annex B start codes
fn has_display_orientation_sei(codec_id: codec::Id, extradata: &[u8], data: &[u8]) -> bool {
    let hevc = codec_id == codec::Id::HEVC;
    let size_length = match extradata {
        [1, ..] if !hevc && extradata.len() > 4  => Some((extradata[4]  & 3) as usize + 1),
        [1, ..] if  hevc && extradata.len() > 21 => Some((extradata[21] & 3) as usize + 1),
        _ => None
    };
    let mut nal_units = Vec::new();
    if let Some(size_length) = size_length {
        let mut rest = data;
        while rest.len() > size_length {
            let (size, tail) = rest.split_at(size_length);
            let size = size.iter().fold(0, |size, &x| (size << 8) | x as usize);
            if size > tail.len() { break; }
            nal_units.push(&tail[..size]);
            rest = &tail[size..];
        }
    } else {
        // Each unit runs up to the next 00 00 01. A 4-byte start code leaves a zero at the end of the unit before it, which doesn't matter here
        let starts: Vec<usize> = (2..data.len()).filter(|&i| data[i] == 1 && data[i - 1] == 0 && data[i - 2] == 0).map(|i| i + 1).collect();
        for (i, &start) in starts.iter().enumerate() {
            let end = starts.get(i + 1).map_or(data.len(), |next| next - 3);
            nal_units.push(&data[start..end.max(start)]);
        }
    }

    nal_units.into_iter().any(|nal| {
        let (is_sei, header_length) = match nal.first() {
            Some(x) if hevc => (matches!((x >> 1) & 0x3F, 39 | 40), 2), // Prefix and suffix SEI
            Some(x) => (x & 0x1F == 6, 1),
            None => (false, 0)
        };
        if !is_sei || nal.len() <= header_length { return false; }

        // Remove the emulation prevention bytes (00 00 03 -> 00 00)
        let mut rbsp = Vec::with_capacity(nal.len());
        let mut zeros = 0;
        for &x in &nal[header_length..] {
            if zeros >= 2 && x == 3 { zeros = 0; continue; }
            zeros = if x == 0 { zeros + 1 } else { 0 };
            rbsp.push(x);
        }
        // sei_message()s: payloadType and payloadSize, coded as a run of 0xFF bytes plus a last byte, then the payload
        let mut pos = 0;
        let read_value = |pos: &mut usize| -> Option<usize> {
            let mut value = 0;
            loop {
                let x = *rbsp.get(*pos)?;
                *pos += 1;
                value += x as usize;
                if x != 0xFF { return Some(value); }
            }
        };
        while let (Some(payload_type), Some(payload_size)) = (read_value(&mut pos), read_value(&mut pos)) {
            if payload_type == 47 { return true; }
            pos += payload_size;
        }
        false
    })
}

/* unsafe extern "C" fn get_hw_format(ctx: *mut ffi::AVCodecContext, pix_fmts: *const ffi::AVPixelFormat) -> ffi::AVPixelFormat {
    let mut i = 0;
    loop {
        let p = *pix_fmts.offset(i);
        if p == ffi::AVPixelFormat::AV_PIX_FMT_NONE {
            break;
        }
        if p == hw_format {
            return p;
        }
        i += 1;
    }

    ::log::error!("Failed to get HW surface format.");
    ffi::AVPixelFormat::AV_PIX_FMT_NONE
} */
