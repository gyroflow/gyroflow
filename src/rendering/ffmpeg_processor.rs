// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use std::collections::HashMap;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::error;

use ffmpeg_next::{ ffi, codec, encoder, format, frame, media, Dictionary, Rational, Stream, rescale, rescale::Rescale };

use gyroflow_core::filesystem::{ self, EngineBase, FilesystemError, FfmpegPathWrapper };
use super::*;
use super::ffmpeg_video::*;
use super::ffmpeg_audio::*;
#[cfg(target_os = "android")]
use super::ffmpeg_android::*;

pub struct FfmpegProcessor<'a> {
    pub gpu_decoding: bool,
    pub gpu_device: Option<String>,
    pub video_codec: Option<String>,

    pub audio_codec: codec::Id,

    input_context: format::context::Input,

    pub video: VideoTranscoder<'a>,

    pub start_ms: Option<f64>,
    pub end_ms: Option<f64>,

    pub decoder_fps: f64,

    pub preserve_other_tracks: bool,

    #[cfg(target_os = "android")]
    pub android_handles: Option<AndroidHWHandles>,

    ost_time_bases: Vec<Rational>,

    _file: FfmpegPathWrapper<'a>
}

#[derive(PartialEq)]
pub enum Status {
    Continue,
    Finish
}

#[derive(Debug)]
pub enum FFmpegError {
    EncoderNotFound,
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
    PixelFormatNotSupported((format::Pixel, Vec<format::Pixel>)),
    UnknownPixelFormat(format::Pixel),
    InternalError(ffmpeg_next::Error),
    CannotOpenInputFile((String, FilesystemError)),
    CannotOpenOutputFile((String, FilesystemError)),
}

impl std::fmt::Display for FFmpegError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            FFmpegError::EncoderNotFound             => write!(f, "Encoder not found"),
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
            FFmpegError::PixelFormatNotSupported(v) => write!(f, "Pixel format {:?} is not supported. Supported ones: {:?}", v.0, v.1),
            FFmpegError::InternalError(e)     => write!(f, "ffmpeg error: {:?}", e),
            FFmpegError::CannotOpenInputFile((url, e))   => write!(f, "Cannot open input file {url}: {e:?}"),
            FFmpegError::CannotOpenOutputFile((url, e))   => write!(f, "Cannot open output file {url}: {e:?}"),
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
    pub width: u32,
    pub height: u32,
    pub bitrate: f64, // in Mbps
    pub rotation: i32,
}

impl<'a> FfmpegProcessor<'a> {
    pub fn from_file(base: &'a EngineBase, url: &str, mut gpu_decoding: bool, gpu_decoder_index: usize, mut decoder_options: Option<Dictionary>) -> Result<Self, FFmpegError> {
        let mut file = FfmpegPathWrapper::new(base, url, false).map_err(|e| FFmpegError::CannotOpenInputFile((url.to_string(), e)))?;

        ffmpeg_next::init()?;
        crate::rendering::init_log();

        let hwaccel_device = decoder_options.as_ref().and_then(|x| x.get("hwaccel_device").map(|x| x.to_string()));
        if file.path.starts_with("fd:") {
            match &mut decoder_options {
                Some(ref mut dict) => { dict.set("fd", &file.path[3..]); file.path = "fd:".into(); }
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
        decoder_ctx.set_threading(ffmpeg_next::threading::Config { kind: ffmpeg_next::threading::Type::Frame, count: 3 });

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

            ost_time_bases: Vec::new(),

            start_ms: None,
            end_ms: None,

            preserve_other_tracks: false,

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
        })
    }

    pub fn render(&mut self, base: &'a EngineBase, output_folder: &str, output_filename: &str, output_size: (u32, u32), bitrate: Option<f64>, cancel_flag: Arc<AtomicBool>, pause_flag: Arc<AtomicBool>) -> Result<(), FFmpegError> {
        let output_url = filesystem::get_file_url(output_folder, output_filename, true);
        let mut file = FfmpegPathWrapper::new(base, &output_url, true).map_err(|e| FFmpegError::CannotOpenOutputFile((output_url.to_string(), e)))?;

        let mut stream_mapping: Vec<isize> = vec![0; self.input_context.nb_streams() as _];
        let mut ist_time_bases = vec![Rational(0, 0); self.input_context.nb_streams() as _];
        self.ost_time_bases.resize(self.input_context.nb_streams() as _, Rational(0, 0));
        let mut atranscoders = HashMap::new();
        let mut output_index = 0usize;

        if let Some(start_ms) = self.start_ms {
            let position = (start_ms as i64).rescale((1, 1000), rescale::TIME_BASE);
            self.input_context.seek(position, ..position)?;
        }
        let mut output_options = Dictionary::new();
        let output_format = if let Some(pos) = output_filename.rfind('.') { &output_filename[pos+1..] } else { "mp4" }.to_ascii_lowercase();
        if file.path.starts_with("fd:") {
            output_options.set("fd", &file.path[3..]);
            file.path = "fd:".into();
        }

        let mut octx = if output_format == "exr" || output_format == "png" {
            format::output_with(&file.path, output_options)
        } else {
            format::output_as_with(&file.path, &output_format, output_options)
        }?;

        for (i, stream) in self.input_context.streams().enumerate() {
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
                        self.video.codec_supported_formats = super::ffmpeg_hw::pix_formats_to_vec((*codec.as_ptr()).pix_fmts);
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
                if self.preserve_other_tracks/*stream.codec().id() == self.audio_codec*/ {
                    // Direct stream copy
                    let mut ost = octx.add_stream(encoder::find(codec::Id::None))?;
                    ost.set_parameters(stream.parameters());
                    // We need to set codec_tag to 0 lest we run into incompatible codec tag issues when muxing into a different container format.
                    unsafe { (*ost.parameters().as_mut_ptr()).codec_tag = 0; }
                } else {
                    // Transcode audio
                    atranscoders.insert(i, AudioTranscoder::new(self.audio_codec, &stream, &mut octx, output_index as _)?);
                }
                output_index += 1;
            } else if self.preserve_other_tracks && medium == media::Type::Data {
                // Direct stream copy
                let mut ost = octx.add_stream(encoder::find(codec::Id::None))?;
                ost.set_parameters(stream.parameters());
                ost.set_avg_frame_rate(stream.avg_frame_rate());
                output_index += 1;
            }
        }
        let mut metadata = self.input_context.metadata().to_owned();
        for (k, v) in self.video.encoder_params.metadata.iter() {
            metadata.set(k, v);
        }
        log::debug!("Output metadata: {:?}", &metadata);
        octx.set_metadata(metadata);
        // Header will be written after video encoder is initalized, in ffmpeg_video.rs:init_encoder

        let mut video_inited = false;

        let mut pending_packets: Vec<(Stream, ffmpeg_next::Packet, usize, isize)> = Vec::new();

        // let mut copied_stream_first_pts = None;
        // let mut copied_stream_first_dts = None;

        let mut process_stream = |octx: &mut format::context::Output, stream: Stream, mut packet: ffmpeg_next::Packet, ist_index: usize, ost_index: isize, ost_time_base: Rational| -> Result<(), Error> {
            match atranscoders.get_mut(&ist_index) {
                Some(atranscoder) => {
                    packet.rescale_ts(stream.time_base(), atranscoder.decoder.time_base());
                    atranscoder.decoder.send_packet(&packet)?;
                    atranscoder.receive_and_process_decoded_frames(octx, ost_time_base, self.start_ms)?;
                }
                None => {
                    // Direct stream copy
                    // TODO: Wrong pts, shifted by length of packet, would need to synchronize with first video frame pts
                    // if copied_stream_first_pts.is_none() {
                    //     copied_stream_first_pts = packet.pts();
                    //     copied_stream_first_dts = packet.dts();
                    // }

                    packet.rescale_ts(ist_time_bases[ist_index], ost_time_base);
                    packet.set_position(-1);
                    packet.set_stream(ost_index as _);
                    // packet.set_pts(packet.pts().map(|x| x - copied_stream_first_pts.unwrap_or_default()));
                    // packet.set_dts(packet.dts().map(|x| x - copied_stream_first_dts.unwrap_or_default()));
                    packet.write_interleaved(octx)?;
                }
            }
            Ok(())
        };

        let mut any_encoded = false;
        for (stream, mut packet) in self.input_context.packets() {
            let ist_index = stream.index();
            let ost_index = stream_mapping[ist_index];
            if ost_index < 0 {
                continue;
            }

            if ist_index == self.video.input_index {
                {
                    let decoder = self.video.decoder.as_mut().ok_or(Error::DecoderNotFound)?;
                    packet.rescale_ts(stream.time_base(), (1, 1000000)); // rescale to microseconds
                    if let Err(err) = decoder.send_packet(&packet) {
                        if self.gpu_decoding && !*GPU_DECODING.read() {
                            return Err(FFmpegError::GPUDecodingFailed);
                        }
                        if !any_encoded {
                            return Err(err.into());
                        }
                    }
                }

                match self.video.receive_and_process_video_frames(output_size, bitrate, Some(&mut octx), &mut self.ost_time_bases, self.start_ms, self.end_ms) {
                    Ok(encoding_status) => {
                        if self.video.encoder.is_some() {
                            video_inited = true;
                            if !pending_packets.is_empty() {
                                for (stream, packet, ist_index, ost_index) in pending_packets.drain(..) {
                                    let ost_time_base = self.ost_time_bases[ost_index as usize];
                                    process_stream(&mut octx, stream, packet, ist_index, ost_index, ost_time_base)?;
                                }
                            }
                            any_encoded = true;
                        }
                        if encoding_status == Status::Finish || cancel_flag.load(Relaxed) {
                            break;
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
            } else if self.audio_codec != codec::Id::None || self.preserve_other_tracks {
                if !video_inited {
                    pending_packets.push((stream, packet, ist_index, ost_index));
                    continue;
                }
                let ost_time_base = self.ost_time_bases[ost_index as usize];
                process_stream(&mut octx, stream, packet, ist_index, ost_index, ost_time_base)?;
            }
        }

        // Flush encoders and decoders.
        {
            let ost_time_base = self.ost_time_bases[self.video.output_index.unwrap_or_default()];
            self.video.decoder.as_mut().ok_or(Error::DecoderNotFound)?.send_eof()?;
            // self.video.decoder.as_mut().ok_or(Error::DecoderNotFound)?.flush();
            self.video.receive_and_process_video_frames(output_size, bitrate, Some(&mut octx), &mut self.ost_time_bases, self.start_ms, self.end_ms)?;
            self.video.encoder.as_mut().ok_or(Error::EncoderNotFound)?.send_eof()?;
            self.video.receive_and_process_encoded_packets(&mut octx, ost_time_base)?;
        }
        if self.audio_codec != codec::Id::None {
            for (ost_index, transcoder) in atranscoders.iter_mut() {
                let ost_time_base = self.ost_time_bases[*ost_index];
                transcoder.flush(&mut octx, ost_time_base, self.start_ms)?;
            }
        }

        octx.write_trailer()?;

        Ok(())
    }

    pub fn start_decoder_only(&mut self, mut ranges: Vec<(f64, f64)>, cancel_flag: Arc<AtomicBool>) -> Result<(), FFmpegError> {
        if !ranges.is_empty() {
            let next_range = ranges.remove(0);
            self.start_ms = Some(next_range.0);
            self.end_ms   = Some(next_range.1);
        }

        if let Some(start_ms) = self.start_ms {
            let position = (start_ms as i64).rescale((1, 1000), rescale::TIME_BASE);
            self.input_context.seek(position, ..position)?;
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
                        if self.gpu_decoding && !*GPU_DECODING.read() {
                            return Err(FFmpegError::GPUDecodingFailed);
                        }
                        if !any_encoded {
                            return Err(err.into());
                        }
                    }
                    match self.video.receive_and_process_video_frames((0, 0), None, None, &mut self.ost_time_bases, self.start_ms, self.end_ms) {
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
            if !ranges.is_empty() {
                let next_range = ranges.remove(0);
                let position = (next_range.0 as i64).rescale((1, 1000), rescale::TIME_BASE);
                self.input_context.seek(position, ..position)?;
                self.end_ms = Some(next_range.1);
                continue;
            } else {
                break;
            }
        }

        // Flush decoder.
        self.video.decoder.as_mut().ok_or(Error::DecoderNotFound)?.send_eof()?;
        self.video.receive_and_process_video_frames((0, 0), None, None, &mut self.ost_time_bases, self.start_ms, self.end_ms)?;

        Ok(())
    }

    pub fn on_frame<F>(&mut self, cb: F) where F: FnMut(i64, &mut frame::Video, Option<&mut frame::Video>, &mut ffmpeg_video_converter::Converter, &mut ffmpeg_video::RateControl) -> Result<(), FFmpegError> + 'a {
        self.video.on_frame_callback = Some(Box::new(cb));
    }
    pub fn on_encoder_initialized<F>(&mut self, cb: F) where F: FnMut(&encoder::video::Video) -> Result<(), FFmpegError> + 'a {
        self.video.on_encoder_initialized = Some(Box::new(cb));
    }

    pub fn get_video_info(url: &str) -> Result<VideoInfo, ffmpeg_next::Error> {
        let base = filesystem::get_engine_base();
        let mut file = FfmpegPathWrapper::new(&base, url, false).map_err(|_| ffmpeg_next::Error::ProtocolNotFound)?;
        let mut dict = Dictionary::new();
        if file.path.starts_with("fd:") {
            dict.set("fd", &file.path[3..]);
            file.path = "fd:".into();
        }

        let context = format::input_with_dictionary(&file.path, dict)?;
        if let Some(stream) = context.streams().best(media::Type::Video) {
            let codec = codec::context::Context::from_parameters(stream.parameters())?;
            if let Ok(video) = codec.decoder().video() {
                let mut bitrate = video.bit_rate();
                if bitrate == 0 { bitrate = context.bit_rate() as usize; }

                let mut frames = stream.frames() as usize;
                if frames == 0 { frames = (stream.duration() as f64 * f64::from(stream.time_base()) * f64::from(stream.rate())) as usize; }

                let rotation = {
                    let mut theta = 0.0;
                    if let Some(rotate_tag) = stream.metadata().get("rotate") {
                        if let Ok(num) = rotate_tag.parse::<f64>() {
                            theta = num;
                        }
                    }
                    if theta == 0.0 {
                        for side_data in stream.side_data() {
                            if side_data.kind() == codec::packet::side_data::Type::DisplayMatrix {
                                let display_matrix = side_data.data();
                                if display_matrix.len() == 9*4 {
                                    theta = -unsafe { ffi::av_display_rotation_get(display_matrix.as_ptr() as *const i32) };
                                }
                            }
                        }
                    }

                    theta -= 360.0 * (theta / 360.0 + 0.9 / 360.0).floor();
                    theta as i32
                };

                return Ok(VideoInfo {
                    duration_ms: stream.duration() as f64 * f64::from(stream.time_base()) * 1000.0,
                    frame_count: frames,
                    fps: f64::from(stream.rate()), // or avg_frame_rate?
                    width: video.width(),
                    height: video.height(),
                    bitrate: bitrate as f64 / 1024.0 / 1024.0,
                    rotation
                });
            }
        }
        Err(ffmpeg_next::Error::StreamNotFound)
    }
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
