// SPDX-License-Identifier: GPL-3.0-or-later
// Test utility: apply the production filter to one packed YUV420P10LE frame.
use ffmpeg_next::{
    color::{Range, Space},
    format::Pixel,
    frame::Video,
};
use gyroflow_export_lut_tests::export_lut::ExportLut;
use std::{error::Error, fs};
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        return Err("Usage: apply_raw_10bit input.raw output.raw lut.cube width height".into());
    }
    ffmpeg_next::init()?;
    let (w, h): (usize, usize) = (args[4].parse()?, args[5].parse()?);
    let bytes = fs::read(&args[1])?;
    if w % 2 != 0 || h % 2 != 0 || bytes.len() != w * h * 3 {
        return Err("Expected one even-sized YUV420P10LE frame".into());
    }
    let mut frame = Video::new(Pixel::YUV420P10LE, w as u32, h as u32);
    frame.set_color_space(Space::BT709);
    frame.set_color_range(Range::MPEG);
    let mut offset = 0;
    for p in 0..3 {
        let (pw, ph) = if p == 0 { (w, h) } else { (w / 2, h / 2) };
        let stride = frame.stride(p);
        for y in 0..ph {
            frame.data_mut(p)[y * stride..][..pw * 2]
                .copy_from_slice(&bytes[offset..offset + pw * 2]);
            offset += pw * 2;
        }
    }
    let mut lut = ExportLut::new(&fs::read(&args[3])?)?;
    let output = lut.apply(&frame)?;
    let mut bytes = Vec::with_capacity(w * h * 3);
    for p in 0..3 {
        let (pw, ph) = if p == 0 { (w, h) } else { (w / 2, h / 2) };
        for y in 0..ph {
            bytes.extend_from_slice(&output.data(p)[y * output.stride(p)..][..pw * 2]);
        }
    }
    fs::write(&args[2], bytes)?;
    Ok(())
}
