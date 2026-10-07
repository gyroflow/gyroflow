// SPDX-License-Identifier: GPL-3.0-or-later

//! An export-only 3D LUT. Frames stay at their original precision and pixel format.
use ffmpeg_next::{Error, ffi, filter, format::Pixel, frame::Video};
use std::{
    ffi::{CStr, CString},
    io::Write,
};

pub struct ExportLut {
    file: tempfile::NamedTempFile,
    graph: Option<filter::Graph>,
    input: Option<(Pixel, u32, u32, i32, i32)>,
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
        if bytes.is_empty() {
            return Err("The selected LUT is empty.".into());
        }
        let mut file = tempfile::Builder::new()
            .suffix(".cube")
            .tempfile()
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
        Ok(Self {
            file,
            graph: None,
            input: None,
        })
    }

    fn build_graph(&self, frame: &Video) -> Result<filter::Graph, String> {
        let mut graph = filter::Graph::new();
        let pixel: ffi::AVPixelFormat = frame.format().into();
        let (color_space, color_range) = unsafe {
            (
                (*frame.as_ptr()).colorspace as i32,
                (*frame.as_ptr()).color_range as i32,
            )
        };
        let mut source = graph.add(&filter::find("buffer").ok_or("FFmpeg buffer filter is unavailable")?, "in", &format!(
            "video_size={}x{}:pix_fmt={}:time_base=1/1000000:pixel_aspect=1/1:colorspace={}:range={}",
            frame.width(), frame.height(), pixel as i32, color_space, color_range
        )).map_err(|e| e.to_string())?;
        let descriptor = unsafe { ffi::av_pix_fmt_desc_get(pixel) };
        if descriptor.is_null() {
            return Err("Unsupported LUT input pixel format".into());
        }
        let has_alpha = unsafe { (*descriptor).flags & ffi::AV_PIX_FMT_FLAG_ALPHA as u64 != 0 };
        let rgb_format = if has_alpha { "gbrapf32le" } else { "gbrpf32le" };
        let mut rgb = graph
            .add(
                &filter::find("format").ok_or("FFmpeg format filter is unavailable")?,
                "rgb",
                &format!("pix_fmts={rgb_format}"),
            )
            .map_err(|e| e.to_string())?;

        // Set the filename directly, rather than placing a user path into a filter expression.
        // This also works with spaces, apostrophes, Unicode and Windows drive letters.
        let lut_filter =
            filter::find("lut3d").ok_or("This FFmpeg build does not include the lut3d filter")?;
        let path = CString::new(self.file.path().to_string_lossy().as_bytes())
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
        let pixel_name =
            unsafe { CStr::from_ptr(ffi::av_get_pix_fmt_name(pixel)) }.to_string_lossy();
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
                rgb.as_mut_ptr(),
                0,
            ))?;
            check(ffi::avfilter_link(rgb.as_mut_ptr(), 0, lut.as_mut_ptr(), 0))?;
            check(ffi::avfilter_link(
                lut.as_mut_ptr(),
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
            )
        };
        if self.input != Some(signature) {
            self.graph = Some(self.build_graph(frame)?);
            self.input = Some(signature);
        }
        let graph = self.graph.as_mut().ok_or("LUT filter is unavailable")?;
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
