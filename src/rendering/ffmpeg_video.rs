use ffmpeg_next::{ ffi, codec, decoder, encoder, format, frame, picture, software, Dictionary, Packet, Rational, Error, rescale::Rescale };

use super::ffmpeg_processor::Status;
use super::ffmpeg_processor::FFmpegError;

#[derive(Default)]
pub struct Converter {
    pub convert_to: Option<software::scaling::Context>,
    pub convert_from: Option<software::scaling::Context>,
    pub sw_frame_converted: Option<frame::Video>,
    pub sw_frame_converted_out: Option<frame::Video>,
}
impl<'a> Converter {
    pub fn convert_pixel_format<F>(&mut self, frame: &mut frame::Video, out_frame: &mut frame::Video, format: format::Pixel, mut cb: F) -> Result<(), FFmpegError> where F: FnMut(&mut frame::Video, &mut frame::Video) + 'a {
        if frame.format() != format {
            if self.sw_frame_converted.is_none() {
                self.sw_frame_converted = Some(frame::Video::new(format, frame.width(), frame.height()));
                self.convert_from = Some(software::converter((frame.width(), frame.height()), frame.format(), format)?);
            }

            if self.sw_frame_converted_out.is_none() {
                self.sw_frame_converted_out = Some(frame::Video::new(format, out_frame.width(), out_frame.height()));
                self.convert_to = Some(software::converter((out_frame.width(), out_frame.height()), format, out_frame.format())?);
            }

            let sw_frame_converted = self.sw_frame_converted.as_mut().ok_or(FFmpegError::FrameEmpty)?;
            let sw_frame_converted_out = self.sw_frame_converted_out.as_mut().ok_or(FFmpegError::FrameEmpty)?;
            let convert_from = self.convert_from.as_mut().ok_or(FFmpegError::ConverterEmpty)?;
            let convert_to = self.convert_to.as_mut().ok_or(FFmpegError::ConverterEmpty)?;

            convert_from.run(frame, sw_frame_converted)?;

            cb(sw_frame_converted, sw_frame_converted_out);
            
            convert_to.run(sw_frame_converted_out, out_frame)?;
        } else {
            cb(frame, out_frame);
        }
        Ok(())
    }
    pub fn scale(&mut self, frame: &mut frame::Video, format: format::Pixel, width: u32, height: u32) -> Result<frame::Video, FFmpegError> {
        if frame.width() != width || frame.height() != height || frame.format() != format {
            if self.sw_frame_converted.is_none() {
                self.sw_frame_converted = Some(frame::Video::new(format, width, height));
                self.convert_to = Some(
                    software::scaling::Context::get(
                        frame.format(), frame.width(), frame.height(), format, width, height, software::scaling::Flags::BILINEAR,
                    )?
                );
            }

            let sw_frame_converted = self.sw_frame_converted.as_mut().ok_or(FFmpegError::FrameEmpty)?;
            let convert_to = self.convert_to.as_mut().ok_or(FFmpegError::ConverterEmpty)?;

            convert_to.run(frame, sw_frame_converted)?;

            Ok(unsafe { frame::Video::wrap(ffi::av_frame_clone(sw_frame_converted.as_ptr())) })
        } else {
            Ok(unsafe { frame::Video::wrap(ffi::av_frame_clone(frame.as_ptr())) })
        }
    }
}

pub struct FrameBuffers {
    pub sw_frame: frame::Video,
    pub encoder_frame: frame::Video,
}
impl Default for FrameBuffers {
    fn default() -> Self { Self {
        sw_frame: frame::Video::empty(),
        encoder_frame: frame::Video::empty(),
    } }
}

#[derive(Default)]
pub struct VideoTranscoder<'a> {
    pub input_index: usize,
    pub output_index: usize,
    pub decoder: Option<decoder::Video>,
    pub encoder: Option<encoder::video::Video>,

    pub codec_options: Dictionary<'a>,

    pub hw_device_type: Option<ffi::AVHWDeviceType>,

    pub encoder_pixel_format: Option<format::Pixel>,
    pub encoder_converter: Option<software::scaling::Context>,

    pub decode_only: bool,
    pub gpu_decoding: bool,
    pub gpu_encoding: bool,

    pub converter: Converter,

    pub buffers: FrameBuffers,

    pub on_frame_callback: Option<Box<dyn FnMut(i64, &mut frame::Video, Option<&mut frame::Video>, &mut Converter) -> Result<(), FFmpegError> + 'a>>,

    pub first_frame_ts: Option<i64>,

    pub output_frame: Option<frame::Video>,
}

impl<'a> VideoTranscoder<'a> {
    fn init_encoder(frame: &mut frame::Video, decoder: &mut decoder::Video, size: (u32, u32), bitrate_mbps: Option<f64>, octx: &mut format::context::Output, hw_device_type: Option<ffi::AVHWDeviceType>, codec_options: Dictionary, format: Option<format::Pixel>) -> Result<encoder::video::Video, FFmpegError> {
        let global_header = octx.format().flags().contains(format::Flags::GLOBAL_HEADER);
        let mut ost = octx.stream_mut(0).unwrap();
        let mut encoder = ost.codec().encoder().video()?;
        let pixel_format = format.unwrap_or_else(|| decoder.format());
        encoder.set_width(size.0);
        encoder.set_height(size.1);
        encoder.set_aspect_ratio(decoder.aspect_ratio());
        encoder.set_format(pixel_format);
        encoder.set_frame_rate(decoder.frame_rate());
        encoder.set_time_base(decoder.frame_rate().unwrap().invert());
        encoder.set_bit_rate(bitrate_mbps.map(|x| (x * 1024.0*1024.0) as usize).unwrap_or_else(|| decoder.bit_rate()));
        encoder.set_color_range(decoder.color_range());
        encoder.set_colorspace(decoder.color_space());
        unsafe {
            (*encoder.as_mut_ptr()).color_trc = (*decoder.as_ptr()).color_trc;
            (*encoder.as_mut_ptr()).color_primaries = (*decoder.as_ptr()).color_primaries;
        }

        if global_header {
            encoder.set_flags(codec::Flags::GLOBAL_HEADER);
        }

        if let Some(hw_type) = hw_device_type {
            unsafe {
                if super::ffmpeg_hw::initialize_hwframes_context(encoder.as_mut_ptr(), frame.as_mut_ptr(), hw_type, pixel_format.into(), size).is_err() {
                    super::append_log("Failed to create encoder HW context.\n");
                }
            }
        }

        encoder.open_with(codec_options)?;
        encoder = ost.codec().encoder().video()?;
        ost.set_parameters(encoder);
        
        Ok(ost.codec().encoder().video()?)
    }
    
    pub fn receive_and_process_video_frames(&mut self, size: (u32, u32), bitrate: Option<f64>, mut octx: Option<&mut format::context::Output>, ost_time_bases: &mut Vec<Rational>, start_ms: Option<f64>, end_ms: Option<f64>) -> Result<Status, FFmpegError> {
        let mut status = Status::Continue;
        
        let decoder = self.decoder.as_mut().ok_or(FFmpegError::DecoderNotFound)?;
        
        let mut frame = frame::Video::empty();
        let mut sw_frame = &mut self.buffers.sw_frame;
        let mut hw_frame = frame::Video::empty();
        
        while decoder.receive_frame(&mut frame).is_ok() {
            if !self.decode_only && self.encoder.is_none() {
                let octx = octx.as_deref_mut().ok_or(FFmpegError::NoOutputContext)?;

                if self.gpu_decoding && self.encoder_pixel_format.is_none() {
                    unsafe {
                        let dl_format = *super::ffmpeg_hw::get_transfer_formats_from_gpu(frame.as_mut_ptr()).first().ok_or(FFmpegError::NoHWTransferFormats)?;
                        let codec = octx.stream(0).unwrap().codec().as_mut_ptr();
                        if !(*codec).codec.is_null() {
                            let sw_formats = super::ffmpeg_hw::pix_formats_to_vec((*(*codec).codec).pix_fmts);
                            let picked = super::ffmpeg_hw::find_best_matching_codec(dl_format, &sw_formats);
                            if picked != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                                self.encoder_pixel_format = Some(format::Pixel::from(picked));
                            }
                        }
                    }
                }
                log::debug!("hw_device_type: {:?}, encoder_pixel_format: {:?}", self.hw_device_type, self.encoder_pixel_format);

                // let mut stderr_buf  = gag::BufferRedirect::stderr().unwrap();

                let result = Self::init_encoder(&mut frame, decoder, size, bitrate, octx, self.hw_device_type, self.codec_options.to_owned(), self.encoder_pixel_format);

                // let mut output = String::new();
                // std::io::Read::read_to_string(stderr_buf, &mut output).unwrap();
                // drop(stderr_buf);
                // println!("output: {:?}", output);
                
                self.encoder = Some(result?);  

                octx.write_header()?;
                //format::context::output::dump(&octx, 0, Some(&output_path));
        
                for (ost_index, _) in octx.streams().enumerate() {
                    ost_time_bases[ost_index] = octx.stream(ost_index as _).ok_or(Error::StreamNotFound)?.time_base();
                }
            }

            if let Some(mut ts) = frame.timestamp() {
                let timestamp_us = ts.rescale(decoder.time_base(), (1, 1000000));
                let timestamp_ms = timestamp_us as f64 / 1000.0;

                if start_ms.is_none() || timestamp_ms >= start_ms.unwrap() {
                    if self.first_frame_ts.is_none() {
                        self.first_frame_ts = frame.timestamp();
                    }
                    ts -= self.first_frame_ts.unwrap();
        
                    let frame_timestamp = frame.timestamp();
                    let frame_color_primaries = frame.color_primaries();
                    let frame_color_range = frame.color_range();
                    let frame_color_space = frame.color_space();
                    let frame_color_transfer_characteristic = frame.color_transfer_characteristic();

                    let timestamp = Some(ts);

                    let raw_format: ffi::AVPixelFormat = frame.format().into();

                    let mut input_frame = if frame.format() == format::Pixel::CUDA || 
                       frame.format() == format::Pixel::DXVA2_VLD || 
                       frame.format() == format::Pixel::VDPAU || 
                       frame.format() == format::Pixel::D3D11 || 
                       frame.format() == format::Pixel::D3D11VA_VLD || 
                       frame.format() == format::Pixel::VIDEOTOOLBOX || 
                       frame.format() == format::Pixel::MEDIACODEC || 
                       frame.format() == format::Pixel::OPENCL || 
                       frame.format() == format::Pixel::QSV ||
                       frame.format() == format::Pixel::MMAL || 
                       raw_format == ffi::AVPixelFormat::AV_PIX_FMT_VAAPI ||
                       raw_format == ffi::AVPixelFormat::AV_PIX_FMT_VAAPI_VLD {
                        unsafe {
                            // retrieve data from GPU to CPU
                            let err = ffi::av_hwframe_transfer_data(sw_frame.as_mut_ptr(), frame.as_mut_ptr(), 0);
                            if err < 0 {
                                return Err(FFmpegError::FromHWTransferError(err));
                            }
                        }
                        &mut sw_frame
                    } else {
                        &mut frame
                    };

                    input_frame.set_pts(frame_timestamp);

                    if !self.decode_only && self.output_frame.is_none()  {
                        self.output_frame = Some(frame::Video::new(input_frame.format(), size.0, size.1));
                    }

                    // Process frame
                    if let Some(ref mut cb) = self.on_frame_callback {
                        cb(timestamp_us, input_frame, self.output_frame.as_mut(), &mut self.converter)?;
                    }

                    // Encode output frame
                    if !self.decode_only {
                        let mut final_sw_frame = if let Some(ref mut fr) = self.output_frame { fr } else { &mut input_frame };

                        if let Some(target_format) = self.encoder_pixel_format {
                            if final_sw_frame.format() != target_format {
                                if self.encoder_converter.is_none() {
                                    self.buffers.encoder_frame = frame::Video::new(target_format, final_sw_frame.width(), final_sw_frame.height());
                                    self.encoder_converter = Some(software::converter((final_sw_frame.width(), final_sw_frame.height()), final_sw_frame.format(), target_format)?);
                                }
                                let conv = self.encoder_converter.as_mut().ok_or(FFmpegError::EncoderConverterEmpty)?;
                                let buff = &mut self.buffers.encoder_frame;
                                conv.run(final_sw_frame, buff)?;
                                final_sw_frame = buff;
                            }
                        }

                        if self.gpu_encoding {
                            // Hardware encoder
                            let encoder = self.encoder.as_mut().ok_or(FFmpegError::EncoderNotFound)?;

                            let output_frame = self.output_frame.as_mut().ok_or(FFmpegError::FrameEmpty)?;
                            hw_frame.set_width(output_frame.width());
                            hw_frame.set_height(output_frame.height());

                            // Upload back to GPU
                            unsafe {
                                let err = ffi::av_hwframe_get_buffer((*encoder.as_mut_ptr()).hw_frames_ctx, hw_frame.as_mut_ptr(), 0);
                                if err < 0 {
                                    return Err(FFmpegError::ToHWBufferError(err));
                                }
                                if (*hw_frame.as_mut_ptr()).hw_frames_ctx.is_null() {
                                    return Err(FFmpegError::NoFramesContext);
                                }
                                let err = ffi::av_hwframe_transfer_data(hw_frame.as_mut_ptr(), output_frame.as_mut_ptr(), 0);
                                if err < 0 {
                                    return Err(FFmpegError::ToHWTransferError(err));
                                }
                            }
                            hw_frame.set_pts(timestamp);
                            hw_frame.set_kind(picture::Type::None);
                            hw_frame.set_color_primaries(frame_color_primaries);
                            hw_frame.set_color_range(frame_color_range);
                            hw_frame.set_color_space(frame_color_space);
                            hw_frame.set_color_transfer_characteristic(frame_color_transfer_characteristic);
                            encoder.send_frame(&hw_frame)?;
                        } else {
                            // Software encoder
                            let encoder = self.encoder.as_mut().ok_or(FFmpegError::EncoderNotFound)?;
                            final_sw_frame.set_pts(timestamp);
                            final_sw_frame.set_kind(picture::Type::None);
                            final_sw_frame.set_color_primaries(frame_color_primaries);
                            final_sw_frame.set_color_range(frame_color_range);
                            final_sw_frame.set_color_space(frame_color_space);
                            final_sw_frame.set_color_transfer_characteristic(frame_color_transfer_characteristic);
                            encoder.send_frame(final_sw_frame)?;
                        }                     
                    }
                    if end_ms.is_some() && timestamp_ms > end_ms.unwrap() {
                        status = Status::Finish;
                        break;
                    }
                }
            }
        }

        if !self.decode_only && self.encoder.is_some() {
            let ost_time_base = ost_time_bases[self.output_index];
            let octx = octx.unwrap();
            self.receive_and_process_encoded_packets(octx, ost_time_base)?;
        }

        Ok(status)
    }

    pub fn receive_and_process_encoded_packets(&mut self, octx: &mut format::context::Output, ost_time_base: Rational) -> Result<(), FFmpegError> {
        if !self.decode_only {
            let time_base = self.decoder.as_ref().ok_or(FFmpegError::DecoderNotFound)?.time_base();
            let mut encoded = Packet::empty();
            while self.encoder.as_mut().ok_or(FFmpegError::EncoderNotFound)?.receive_packet(&mut encoded).is_ok() {
                encoded.set_stream(self.output_index);
                encoded.rescale_ts(time_base, ost_time_base);
                if octx.format().name().contains("image") {
                    encoded.write(octx)?;
                } else {
                    encoded.write_interleaved(octx)?;
                }
            }
        }
        Ok(())
    }
}
