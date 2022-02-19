// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Maik <myco at gmx>

use ffmpeg_next::{ format, software, frame, channel_layout::ChannelLayout, Error };


pub struct AudioResampler {
    resampler: software::resampling::Context,
    src_frame: frame::Audio,
    buffer_frame: frame::Audio,
    chunk_size: usize,
    src_frame_offset: usize,
    buffer_frame_offset: usize
}

impl AudioResampler {
    pub fn new(
        (in_format, in_layout, in_rate): (format::Sample, ChannelLayout, u32),
        (out_format, out_layout, out_rate): (format::Sample, ChannelLayout, u32),
        chunk_size: usize
    ) -> Result<Self, Error> {

        let resampler = software::resampler(
            (in_format, in_layout, in_rate),
            (out_format, out_layout, out_rate)
        )?;

        let src_frame = frame::Audio::empty();
        let buffer_frame = frame::Audio::new(out_format, chunk_size, out_layout);

        Ok(Self {
            resampler,
            src_frame,
            buffer_frame,
            chunk_size,
            src_frame_offset: 0,
            buffer_frame_offset: 0
        })
    }

    pub fn new_frame(&mut self, in_frame: &mut frame::Audio) -> Result<(), Error> {
        self.src_frame = frame::Audio::empty();
        self.src_frame.set_pts(in_frame.pts());

        in_frame.set_channel_layout(self.resampler.input().channel_layout);
        self.resampler.run(&in_frame, &mut self.src_frame)?;

        self.src_frame_offset = 0;
        Ok(())
    }

    pub fn run(&mut self) -> Option<&frame::Audio> {
        let in_frame_samples = self.src_frame.samples();
        if self.src_frame_offset < in_frame_samples {
            let buf_space = self.chunk_size - self.buffer_frame_offset;
            let copy_samples = buf_space.min(in_frame_samples - self.src_frame_offset);

            let bytes_per_sample = self.resampler.output().format.bytes();
            let dest_byte_offset = self.buffer_frame_offset * bytes_per_sample;
            let src_byte_offset = self.src_frame_offset * bytes_per_sample;
            
            let channels = self.resampler.output().channel_layout.channels().max(1) as usize;
            if self.resampler.output().format.is_planar() {
                for c in 0..channels {
                    unsafe {
                        let dst_ptr = (*self.buffer_frame.as_mut_ptr()).data[c].offset(dest_byte_offset as isize);
                        let src_ptr = (*self.src_frame.as_ptr()).data[c].offset(src_byte_offset as isize);
                        std::ptr::copy_nonoverlapping::<u8>(src_ptr, dst_ptr, copy_samples * bytes_per_sample);
                    }
                }
            } else {
                unsafe {
                    let dst_ptr = (*self.buffer_frame.as_mut_ptr()).data[0].offset((dest_byte_offset * channels) as isize);
                    let src_ptr = (*self.src_frame.as_ptr()).data[0].offset((src_byte_offset * channels) as isize);
                    std::ptr::copy_nonoverlapping::<u8>(src_ptr, dst_ptr, copy_samples * bytes_per_sample * channels);
                }
            }

            if self.buffer_frame_offset == 0 {
                self.buffer_frame.set_pts(Some(self.src_frame.pts().unwrap() + (self.src_frame_offset as i64)));
            }

            self.src_frame_offset += copy_samples;
            self.buffer_frame_offset += copy_samples;
            
            if self.buffer_frame_offset >= self.chunk_size {
                self.buffer_frame.set_samples(self.chunk_size);
                self.buffer_frame_offset = 0;
                return Some(&self.buffer_frame);
            }
        }

        None
    }

    pub fn flush(&mut self) -> Option<&frame::Audio> {
        if self.buffer_frame_offset > 0 {
            let missing_samples = self.chunk_size - self.buffer_frame_offset;
            if missing_samples > 0 {
                let bytes_per_sample = self.resampler.output().format.bytes();
                let dest_byte_offset = self.buffer_frame_offset * bytes_per_sample;

                let channels = self.resampler.output().channel_layout.channels().max(1) as usize;
                if self.resampler.output().format.is_planar() {
                    for c in 0..channels {
                        unsafe {
                            let dst_ptr = (*self.buffer_frame.as_mut_ptr()).data[c].offset(dest_byte_offset as isize);
                            std::ptr::write_bytes::<u8>(dst_ptr, 0,missing_samples * bytes_per_sample);
                        }
                    }
                } else {
                    unsafe {
                        let dst_ptr = (*self.buffer_frame.as_mut_ptr()).data[0].offset((dest_byte_offset * channels) as isize);
                        std::ptr::write_bytes::<u8>(dst_ptr, 0, missing_samples * bytes_per_sample * channels);
                    }
                }
            }

            self.buffer_frame.set_samples(self.chunk_size);

            self.buffer_frame_offset = 0;
            Some(&self.buffer_frame)
        } else {
            None
        }
    }
}