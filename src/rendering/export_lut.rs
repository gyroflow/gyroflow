// SPDX-License-Identifier: GPL-3.0-or-later

//! Export LUT and neutral-by-default brightness/contrast, after stabilization.
use ffmpeg_next::{Error, ffi, filter, format::Pixel, frame::Video};
use rayon::prelude::*;
use std::{
    ffi::{CStr, CString},
    io::Write,
};

pub struct ExportLut {
    file: Option<tempfile::NamedTempFile>,
    brightness: f64,
    contrast: f64,
    graph: Option<filter::Graph>,
    output_graph: Option<filter::Graph>,
    input: Option<(Pixel, u32, u32, i32, i32, i32, i32)>,
}

fn check(result: i32) -> Result<(), String> {
    if result < 0 {
        Err(Error::from(result).to_string())
    } else {
        Ok(())
    }
}

impl ExportLut {
    pub fn new(bytes: &[u8]) -> Result<Self, String> {
        Self::with_adjustments(Some(bytes), 0.0, 0.0)
    }

    pub fn with_adjustments(
        bytes: Option<&[u8]>,
        brightness: f64,
        contrast: f64,
    ) -> Result<Self, String> {
        if !brightness.is_finite()
            || !contrast.is_finite()
            || brightness.abs() > 0.5
            || contrast.abs() > 0.5
        {
            return Err("Brightness and contrast must be between −50% and +50%.".into());
        }
        let file = if let Some(bytes) = bytes {
            let cube = super::cube_lut::CubeLut::parse(bytes)?;
            let mut file = tempfile::Builder::new()
                .suffix(".cube")
                .tempfile()
                .map_err(|e| e.to_string())?;
            file.write_all(&cube.canonical_cube())
                .map_err(|e| e.to_string())?;
            file.flush().map_err(|e| e.to_string())?;
            Some(file)
        } else {
            None
        };
        Ok(Self {
            file,
            brightness,
            contrast,
            graph: None,
            output_graph: None,
            input: None,
        })
    }

    fn build_graph(
        &self,
        frame: &Video,
        output_pixel: Pixel,
        apply_lut: bool,
    ) -> Result<filter::Graph, String> {
        let mut graph = filter::Graph::new();
        let pixel: ffi::AVPixelFormat = frame.format().into();
        let (color_space, color_range) = unsafe {
            (
                (*frame.as_ptr()).colorspace as i32,
                (*frame.as_ptr()).color_range as i32,
            )
        };
        let aspect = frame.aspect_ratio();
        let (aspect_num, aspect_den) = if aspect.numerator() > 0 && aspect.denominator() > 0 {
            (aspect.numerator(), aspect.denominator())
        } else {
            (1, 1)
        };
        let mut source = graph.add(&filter::find("buffer").ok_or("FFmpeg buffer filter is unavailable")?, "in", &format!(
            "video_size={}x{}:pix_fmt={}:time_base=1/1000000:pixel_aspect={}/{}:colorspace={}:range={}",
            frame.width(), frame.height(), pixel as i32, aspect_num, aspect_den, color_space, color_range
        )).map_err(|e| e.to_string())?;
        let descriptor = unsafe { ffi::av_pix_fmt_desc_get(pixel) };
        if descriptor.is_null() {
            return Err("Unsupported LUT input pixel format".into());
        }
        let has_alpha = unsafe { (*descriptor).flags & ffi::AV_PIX_FMT_FLAG_ALPHA as u64 != 0 };
        let rgb_format = if has_alpha { "gbrapf32le" } else { "gbrpf32le" };
        let rgb = graph
            .add(
                &filter::find("format").ok_or("FFmpeg format filter is unavailable")?,
                "rgb",
                &format!("pix_fmts={rgb_format}"),
            )
            .map_err(|e| e.to_string())?;

        let mut last = rgb;
        if let Some(file) = self.file.as_ref().filter(|_| apply_lut) {
            // Pass the private filename through the option API, never a user expression.
            let lut_filter = filter::find("lut3d")
                .ok_or("This FFmpeg build does not include the lut3d filter")?;
            let path = CString::new(file.path().to_string_lossy().as_bytes())
                .map_err(|e| e.to_string())?;
            let mut lut = unsafe {
                let ptr = ffi::avfilter_graph_alloc_filter(
                    graph.as_mut_ptr(),
                    lut_filter.as_ptr(),
                    c"lut".as_ptr(),
                );
                if ptr.is_null() {
                    return Err("Unable to allocate LUT filter".into());
                }
                check(ffi::av_opt_set(
                    ptr.cast(),
                    c"file".as_ptr(),
                    path.as_ptr(),
                    ffi::AV_OPT_SEARCH_CHILDREN,
                ))?;
                check(ffi::av_opt_set_int(
                    ptr.cast(),
                    c"interp".as_ptr(),
                    2,
                    ffi::AV_OPT_SEARCH_CHILDREN,
                ))?;
                check(ffi::avfilter_init_str(ptr, std::ptr::null()))?;
                filter::Context::wrap(ptr)
            };
            unsafe {
                check(ffi::avfilter_link(
                    last.as_mut_ptr(),
                    0,
                    lut.as_mut_ptr(),
                    0,
                ))?;
            }
            last = lut;
        }
        let output_descriptor = unsafe { ffi::av_pix_fmt_desc_get(output_pixel.into()) };
        if output_descriptor.is_null() {
            return Err("Unsupported color output pixel format".into());
        }
        if !apply_lut
            && unsafe { (*output_descriptor).flags & ffi::AV_PIX_FMT_FLAG_RGB as u64 == 0 }
        {
            // Separate RGB graphs cannot negotiate the original YUV matrix/range
            // across their boundary. Specify them on the conversion itself.
            let mut scale = graph
                .add(
                    &filter::find("scale").ok_or("FFmpeg scale filter is unavailable")?,
                    "restore_yuv",
                    &format!("out_color_matrix={color_space}:out_range={color_range}"),
                )
                .map_err(|e| e.to_string())?;
            unsafe {
                check(ffi::avfilter_link(
                    last.as_mut_ptr(),
                    0,
                    scale.as_mut_ptr(),
                    0,
                ))?;
            }
            last = scale;
        }
        let pixel_name = unsafe { CStr::from_ptr(ffi::av_get_pix_fmt_name(output_pixel.into())) }
            .to_string_lossy();
        let mut output_format = graph
            .add(
                &filter::find("format").ok_or("FFmpeg format filter is unavailable")?,
                "format",
                &format!("pix_fmts={pixel_name}"),
            )
            .map_err(|e| e.to_string())?;
        let mut sink = graph
            .add(
                &filter::find("buffersink").ok_or("FFmpeg buffersink filter is unavailable")?,
                "out",
                "",
            )
            .map_err(|e| e.to_string())?;
        unsafe {
            check(ffi::avfilter_link(
                source.as_mut_ptr(),
                0,
                graph
                    .get("rgb")
                    .ok_or("LUT RGB input is unavailable")?
                    .as_mut_ptr(),
                0,
            ))?;
            check(ffi::avfilter_link(
                last.as_mut_ptr(),
                0,
                output_format.as_mut_ptr(),
                0,
            ))?;
            check(ffi::avfilter_link(
                output_format.as_mut_ptr(),
                0,
                sink.as_mut_ptr(),
                0,
            ))?;
        }
        graph.validate().map_err(|e| e.to_string())?;
        Ok(graph)
    }

    pub fn apply(&mut self, frame: &Video) -> Result<Video, String> {
        let signature = unsafe {
            (
                frame.format(),
                frame.width(),
                frame.height(),
                (*frame.as_ptr()).colorspace as i32,
                (*frame.as_ptr()).color_range as i32,
                frame.aspect_ratio().numerator(),
                frame.aspect_ratio().denominator(),
            )
        };
        let needs_adjustment = self.brightness != 0.0 || self.contrast != 0.0;
        if self.input != Some(signature) {
            let output_pixel = if needs_adjustment {
                let descriptor = unsafe { ffi::av_pix_fmt_desc_get(frame.format().into()) };
                if descriptor.is_null() {
                    return Err("Unsupported color input pixel format".into());
                }
                if unsafe { (*descriptor).flags & ffi::AV_PIX_FMT_FLAG_ALPHA as u64 != 0 } {
                    Pixel::GBRAPF32LE
                } else {
                    Pixel::GBRPF32LE
                }
            } else {
                frame.format()
            };
            self.graph = Some(self.build_graph(frame, output_pixel, true)?);
            self.output_graph = None;
            self.input = Some(signature);
        }
        let graph = self.graph.as_mut().ok_or("LUT filter is unavailable")?;
        let mut output = Self::run_graph(graph, frame)?;
        if needs_adjustment {
            self.adjust_rgb(&mut output)?;
            // Carry the original conversion matrix/range across the native
            // RGB stage, just as the previous single filter graph did.
            output.set_color_space(frame.color_space());
            output.set_color_range(frame.color_range());
            if self.output_graph.is_none() {
                self.output_graph = Some(self.build_graph(&output, frame.format(), false)?);
            }
            output = Self::run_graph(
                self.output_graph
                    .as_mut()
                    .ok_or("Color output filter is unavailable")?,
                &output,
            )?;
        }
        Ok(output)
    }

    fn adjust_rgb(&self, frame: &mut Video) -> Result<(), String> {
        if !matches!(frame.format(), Pixel::GBRPF32LE | Pixel::GBRAPF32LE) {
            return Err("Color adjustments require planar float RGB".into());
        }
        // The LUT-free graph may return a reference to the caller's frame.
        // Make it writable before changing RGB; alpha and padding are untouched.
        unsafe {
            check(ffi::av_frame_make_writable(frame.as_mut_ptr()))?;
        }
        let width = frame.width() as usize;
        let gain = 1.0 + self.contrast;
        let offset = 0.5 + self.brightness;
        for plane in 0..3 {
            let stride = frame.stride(plane);
            if stride < width * 4 || stride % 4 != 0 {
                return Err("Invalid float RGB row stride".into());
            }
            let samples: &mut [f32] = bytemuck::try_cast_slice_mut(frame.data_mut(plane))
                .map_err(|_| "Invalid float RGB plane alignment")?;
            samples.par_chunks_exact_mut(stride / 4).for_each(|row| {
                for sample in &mut row[..width] {
                    // Match geq's f64 evaluation and final float storage, without
                    // reassociation/FMA or per-pixel expression interpretation.
                    // The graph uses explicit little-endian float formats.
                    let input = f32::from_bits(u32::from_le(sample.to_bits()));
                    let output = (((input as f64 - 0.5) * gain + offset).clamp(0.0, 1.0)) as f32;
                    *sample = f32::from_bits(output.to_bits().to_le());
                }
            });
        }
        Ok(())
    }

    fn run_graph(graph: &mut filter::Graph, frame: &Video) -> Result<Video, String> {
        // The wrapper's source().add() consumes the AVFrame even though it takes a
        // shared reference. Keep the caller's stabilization buffers intact.
        let mut source = graph.get("in").ok_or("LUT input is unavailable")?;
        unsafe {
            check(ffi::av_buffersrc_add_frame_flags(
                source.as_mut_ptr(),
                frame.as_ptr().cast_mut(),
                ffi::AV_BUFFERSRC_FLAG_KEEP_REF as i32,
            ))?;
        }
        let mut output = Video::empty();
        graph
            .get("out")
            .ok_or("LUT output is unavailable")?
            .sink()
            .frame(&mut output)
            .map_err(|e| e.to_string())?;
        Ok(output)
    }
}
