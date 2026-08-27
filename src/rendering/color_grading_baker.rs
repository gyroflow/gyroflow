// SPDX-License-Identifier: GPL-3.0-or-later

//! Bakes color grading + LUT into the exported frame.
//!
//! The GPU undistort/stabilization writes the final stabilized frame in its
//! native (often YUV, planar, per-plane) format, where RGB-mixing color ops
//! can't be applied. So after stabilization we round-trip the output frame
//! through 16-bit RGBA, apply the SAME color math as the preview
//! (`ColorGradingParams::apply_rgb`), and convert back to the native format.
//!
//! This runs only when color grading is active (`!is_identity()`), so normal
//! exports are unaffected.

use ffmpeg_next::{ format, frame, software };
use gyroflow_core::color_grading::ColorGradingParams;
use crate::rendering::FFmpegError;

pub struct ColorGradingBaker {
    to_rgb: Option<software::scaling::Context>,
    from_rgb: Option<software::scaling::Context>,
    rgb: Option<frame::Video>,
    src_format: Option<format::Pixel>,
    size: (u32, u32),
}

impl Default for ColorGradingBaker {
    fn default() -> Self {
        Self { to_rgb: None, from_rgb: None, rgb: None, src_format: None, size: (0, 0) }
    }
}

impl ColorGradingBaker {
    /// Apply color grading to `out_frame` in place. No-op if `cg.is_identity()`.
    pub fn apply(&mut self, out_frame: &mut frame::Video, cg: &ColorGradingParams, interp: software::scaling::flag::Flags) -> Result<(), FFmpegError> {
        if cg.is_identity() { return Ok(()); }

        let w = out_frame.width();
        let h = out_frame.height();
        let fmt = out_frame.format();
        const RGB: format::Pixel = format::Pixel::RGBA64LE;

        // If the frame is already RGBA64LE we can grade it directly.
        let direct = fmt == RGB;

        // (Re)build the cached scaling contexts + scratch frame if anything changed.
        if !direct && (self.src_format != Some(fmt) || self.size != (w, h) || self.rgb.is_none()) {
            self.rgb = Some(frame::Video::new(RGB, w, h));
            self.to_rgb = Some(software::scaling::Context::get(fmt, w, h, RGB, w, h, interp)?);
            self.from_rgb = Some(software::scaling::Context::get(RGB, w, h, fmt, w, h, interp)?);
            self.src_format = Some(fmt);
            self.size = (w, h);
        }

        if direct {
            Self::grade_rgba64(out_frame, cg);
        } else {
            let rgb = self.rgb.as_mut().ok_or(FFmpegError::FrameEmpty)?;
            self.to_rgb.as_mut().ok_or(FFmpegError::ConverterEmpty)?.run(out_frame, rgb)?;
            Self::grade_rgba64(rgb, cg);
            self.from_rgb.as_mut().ok_or(FFmpegError::ConverterEmpty)?.run(rgb, out_frame)?;
        }
        Ok(())
    }

    /// Apply `apply_rgb` to every pixel of a packed RGBA64LE frame (4x u16 per pixel).
    fn grade_rgba64(f: &mut frame::Video, cg: &ColorGradingParams) {
        let w = f.width() as usize;
        let h = f.height() as usize;
        const MAX: f32 = 65535.0;
        let (ptr, stride) = unsafe {
            let p = f.as_mut_ptr();
            ((*p).data[0], (*p).linesize[0] as usize)
        };
        if ptr.is_null() || stride < w * 8 { return; }
        let data: &mut [u8] = unsafe { std::slice::from_raw_parts_mut(ptr, stride * h) };
        for y in 0..h {
            let row = &mut data[y * stride .. y * stride + w * 8];
            for px in row.chunks_exact_mut(8) {
                let r = u16::from_le_bytes([px[0], px[1]]) as f32 / MAX;
                let g = u16::from_le_bytes([px[2], px[3]]) as f32 / MAX;
                let b = u16::from_le_bytes([px[4], px[5]]) as f32 / MAX;
                let out = cg.apply_rgb([r, g, b]);
                let ri = (out[0] * MAX).round().clamp(0.0, MAX) as u16;
                let gi = (out[1] * MAX).round().clamp(0.0, MAX) as u16;
                let bi = (out[2] * MAX).round().clamp(0.0, MAX) as u16;
                px[0..2].copy_from_slice(&ri.to_le_bytes());
                px[2..4].copy_from_slice(&gi.to_le_bytes());
                px[4..6].copy_from_slice(&bi.to_le_bytes());
                // alpha (px[6..8]) left unchanged
            }
        }
    }
}
