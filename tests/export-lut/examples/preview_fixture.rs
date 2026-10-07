// SPDX-License-Identifier: GPL-3.0-or-later
//! Generate a packed preview atlas and the production export reference for an RGB24 image.
use ffmpeg_next::{format::Pixel, frame::Video};
use gyroflow_export_lut_tests::{cube_lut::CubeLut, export_lut::ExportLut};
use std::{error::Error, fs};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 9 {
        return Err("preview_fixture lut.cube input.rgb output.rgb atlas.rgb width height brightness contrast".into());
    }
    ffmpeg_next::init()?;
    let bytes = fs::read(&args[1])?;
    let cube = CubeLut::parse(&bytes)?;
    let (aw, ah, atlas) = cube.atlas();
    fs::write(&args[4], atlas)?;
    let (w, h): (usize, usize) = (args[5].parse()?, args[6].parse()?);
    let input = fs::read(&args[2])?;
    if input.len() != w * h * 3 {
        return Err("Expected one RGB24 frame".into());
    }
    let mut frame = Video::new(Pixel::GBRPF32LE, w as u32, h as u32);
    for (plane, channel) in [1, 2, 0].iter().enumerate() {
        let stride = frame.stride(plane);
        for y in 0..h {
            for x in 0..w {
                let value = input[(y * w + x) * 3 + channel] as f32 / 255.0;
                frame.data_mut(plane)[y * stride + x * 4..][..4]
                    .copy_from_slice(&value.to_le_bytes());
            }
        }
    }
    let output = ExportLut::with_adjustments(Some(&bytes), args[7].parse()?, args[8].parse()?)?
        .apply(&frame)?;
    let mut pixels = Vec::new();
    for y in 0..h {
        for x in 0..w {
            for plane in [2, 0, 1] {
                let value = f32::from_le_bytes(
                    output.data(plane)[y * output.stride(plane) + x * 4..][..4]
                        .try_into()
                        .unwrap(),
                );
                pixels.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
    }
    fs::write(&args[3], pixels)?;
    println!("atlas {aw} {ah}, LUT {}", cube.size);
    Ok(())
}
