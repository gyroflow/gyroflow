use std::collections::HashMap;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::error;

use ffmpeg_next::{ ffi, codec, encoder, format, frame, log, media, Dictionary, Rational, Stream, rescale, rescale::Rescale };

use super::*;
use super::ffmpeg_video::*;
use super::ffmpeg_audio::*;

pub struct FfmpegProcessor<'a> {
    pub gpu_decoding: bool,
    pub gpu_device: Option<String>,
    pub video_codec: Option<String>,

    pub audio_codec: codec::Id,

    input_context: format::context::Input,

    pub video: VideoTranscoder<'a>,

    pub start_ms: Option<f64>,
    pub end_ms: Option<f64>,

    ost_time_bases: Vec<Rational>,
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
    NoHWTransferFormats,
    FromHWTransferError(i32),
    ToHWTransferError(i32),
    NoFramesContext,
    ToHWBufferError(i32),
    UnknownPixelFormat(format::Pixel),
    InternalError(ffmpeg_next::Error),
}

impl std::fmt::Display for FFmpegError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            FFmpegError::EncoderNotFound             => write!(f, "Encoder not found"),
            FFmpegError::DecoderNotFound             => write!(f, "Decoder not found"),
            FFmpegError::NoSupportedFormats          => write!(f, "No supported formats"),
            FFmpegError::NoOutputContext             => write!(f, "No output context"),
            FFmpegError::EncoderConverterEmpty       => write!(f, "Encoder converter is null"),
            FFmpegError::ConverterEmpty              => write!(f, "Converter is null"),
            FFmpegError::FrameEmpty                  => write!(f, "Frame is null"),
            FFmpegError::NoHWTransferFormats         => write!(f, "No hardware transfer formats"),
            FFmpegError::FromHWTransferError(i)  => write!(f, "Error transferring frame from the GPU: {:?}", ffmpeg_next::Error::Other { errno: i }),
            FFmpegError::ToHWTransferError(i)    => write!(f, "Error transferring frame to the GPU: {:?}", ffmpeg_next::Error::Other { errno: i }),
            FFmpegError::ToHWBufferError(i)      => write!(f, "Error getting HW transfer buffer to the GPU: {:?}", ffmpeg_next::Error::Other { errno: i }),
            FFmpegError::NoFramesContext             => write!(f, "Empty hw frames context"),
            FFmpegError::UnknownPixelFormat(v) => write!(f, "Unknown pixel format: {:?}", v),
            FFmpegError::InternalError(e)      => write!(f, "ffmpeg error: {:?}", e),
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

impl<'a> FfmpegProcessor<'a> {
    pub fn from_file(path: &str, mut gpu_decoding: bool) -> Result<Self, FFmpegError> {
        ffmpeg_next::init()?;
        log::set_level(log::Level::Info);

        let mut input_context = format::input(&path)?;

        // format::context::input::dump(&input_context, 0, Some(path));

        let best_video_stream = unsafe {
            let mut decoder = std::ptr::null_mut();
            let index = ffi::av_find_best_stream(input_context.as_mut_ptr(), media::Type::Video.into(), -1i32, -1i32, &mut decoder, 0);
            if index >= 0 && !decoder.is_null() {
                Ok((Stream::wrap(&input_context, index as usize), decoder))
            } else {
                Err(Error::StreamNotFound)
            }
        };

        let strm = best_video_stream?;
        let mut stream = strm.0;
        let decoder = strm.1;

        let mut hw_backend = String::new();
        if gpu_decoding {
            // --------------------------- GPU ---------------------------
            let hw = ffmpeg_hw::init_device_for_decoding(decoder, &mut stream)?;
            hw_backend = hw.1;
            super::append_log(&format!("Selected HW backend {:?} with format {:?}\n", hw.0, hw.2));
            // --------------------------- GPU ---------------------------
        }
        gpu_decoding = !hw_backend.is_empty();

        Ok(Self {
            gpu_decoding,
            gpu_device: if !gpu_decoding { None } else { Some(hw_backend) },
            video_codec: None,

            audio_codec: codec::Id::AAC,

            ost_time_bases: Vec::new(),

            start_ms: None,
            end_ms: None,
        
            video: VideoTranscoder {
                gpu_encoding: true,
                gpu_decoding,
                input_index: stream.index(),
                codec_options: Dictionary::new(),
                ..VideoTranscoder::default()
            },

            input_context,
        })
    }

    pub fn render(&mut self, output_path: &str, output_size: (u32, u32), bitrate: Option<f64>, cancel_flag: Arc<AtomicBool>) -> Result<(), FFmpegError> {
        let mut stream_mapping: Vec<isize> = vec![0; self.input_context.nb_streams() as _];
        let mut ist_time_bases = vec![Rational(0, 0); self.input_context.nb_streams() as _];
        self.ost_time_bases.resize(self.input_context.nb_streams() as _, Rational(0, 0));
        let mut atranscoders = HashMap::new();
        let mut output_index = 0usize;

        if let Some(start_ms) = self.start_ms {
            let position = (start_ms as i64).rescale((1, 1000), rescale::TIME_BASE);
            self.input_context.seek(position, ..position)?;
        }

        let mut octx = format::output(&output_path)?;

        for (i, stream) in self.input_context.streams().enumerate() {
            let medium = stream.codec().medium();
            if medium != media::Type::Audio && medium != media::Type::Video {
                stream_mapping[i] = -1;
                continue;
            }
            stream_mapping[i] = output_index as isize;
            ist_time_bases[i] = stream.time_base();
            if medium == media::Type::Video { // TODO limit to first video stream
                self.video.input_index = i;
                self.video.output_index = output_index;

                octx.add_stream(encoder::find_by_name(self.video_codec.as_ref().ok_or(Error::EncoderNotFound)?))?;

                self.video.decoder = Some(stream.codec().decoder().video()?);

            } else if medium == media::Type::Audio && self.audio_codec != codec::Id::None {
                if stream.codec().id() == self.audio_codec {
                    // Direct stream copy
                    let mut ost = octx.add_stream(encoder::find(codec::Id::None))?;
                    ost.set_parameters(stream.parameters());
                    // We need to set codec_tag to 0 lest we run into incompatible codec tag issues when muxing into a different container format. 
                    unsafe { (*ost.parameters().as_mut_ptr()).codec_tag = 0; }
                } else {
                    // Transcode audio
                    atranscoders.insert(i, AudioTranscoder::new(self.audio_codec, &stream, &mut octx, output_index as _)?);
                }
            }
            output_index += 1;
        }

        octx.set_metadata(self.input_context.metadata().to_owned());
        // Header will be written after video encoder is initalized, in ffmpeg_video.rs:init_encoder

        let mut video_inited = false;

        let mut pending_packets: Vec<(Stream, ffmpeg_next::Packet, usize, isize)> = Vec::new();

        let mut copied_stream_first_pts = None;
        let mut copied_stream_first_dts = None;

        let mut process_stream = |octx: &mut format::context::Output, stream: Stream, mut packet: ffmpeg_next::Packet, ist_index: usize, ost_index: isize, ost_time_base: Rational| -> Result<(), Error> {
            match atranscoders.get_mut(&ist_index) {
                Some(atranscoder) => {
                    packet.rescale_ts(stream.time_base(), atranscoder.decoder.time_base());
                    atranscoder.decoder.send_packet(&packet)?;
                    atranscoder.receive_and_process_decoded_frames(octx, ost_time_base)?;
                }
                None => {
                    // Direct stream copy
                    if copied_stream_first_pts.is_none() {
                        copied_stream_first_pts = packet.pts();
                        copied_stream_first_dts = packet.dts();
                    }
        
                    packet.rescale_ts(ist_time_bases[ist_index], ost_time_base);
                    packet.set_position(-1);
                    packet.set_stream(ost_index as _);
                    packet.set_pts(packet.pts().map(|x| x - copied_stream_first_pts.unwrap_or_default()));
                    packet.set_dts(packet.dts().map(|x| x - copied_stream_first_dts.unwrap_or_default()));
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
                    packet.rescale_ts(stream.time_base(), decoder.time_base());
                    if let Err(err) = decoder.send_packet(&packet) {
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
                    },
                    Err(e) => {
                        if !any_encoded {
                            return Err(e);
                        }
                    }
                }
            } else if self.audio_codec != codec::Id::None {
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
            let ost_time_base = self.ost_time_bases[self.video.output_index];
            self.video.decoder.as_mut().ok_or(Error::DecoderNotFound)?.send_eof()?;
            self.video.receive_and_process_video_frames(output_size, bitrate, Some(&mut octx), &mut self.ost_time_bases, self.start_ms, self.end_ms)?;
            self.video.encoder.as_mut().ok_or(Error::EncoderNotFound)?.send_eof()?;
            self.video.receive_and_process_encoded_packets(&mut octx, ost_time_base)?;
        }
        if self.audio_codec != codec::Id::None {
            for (ost_index, transcoder) in atranscoders.iter_mut() {
                let ost_time_base = self.ost_time_bases[*ost_index];
                transcoder.decoder.send_eof()?;
                transcoder.receive_and_process_decoded_frames(&mut octx, ost_time_base)?;
                transcoder.encoder.send_eof()?;
                transcoder.receive_and_process_encoded_packets(&mut octx, ost_time_base)?;
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
            if stream.codec().medium() == media::Type::Video {
                self.video.input_index = i;

                let codec = stream.codec().decoder();
                // TODO this doesn't work for some reason
                // let c_name = CString::new("resize").unwrap();
                // let c_val = CString::new("1280x720").unwrap();
                // unsafe { ffi::av_opt_set((*codec.as_mut_ptr()).priv_data, c_name.as_ptr(), c_val.as_ptr(), 1); } 

                self.video.decoder = Some(codec.video()?);
                break;
            }
        }

        let mut any_encoded = false;
        loop {
            for (stream, mut packet) in self.input_context.packets() {
                let ist_index = stream.index();

                if ist_index == self.video.input_index {
                    let decoder = self.video.decoder.as_mut().ok_or(Error::DecoderNotFound)?;
                    packet.rescale_ts(stream.time_base(), decoder.time_base());

                    if let Err(err) = decoder.send_packet(&packet) {
                        ::log::error!("Decoder error {:?}", err);
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
                                return Err(e.into());
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

    pub fn on_frame<F>(&mut self, cb: F) where F: FnMut(i64, &mut frame::Video, Option<&mut frame::Video>, &mut ffmpeg_video::Converter) -> Result<(), FFmpegError> + 'a {
        self.video.on_frame_callback = Some(Box::new(cb));
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
