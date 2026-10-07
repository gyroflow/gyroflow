// SPDX-License-Identifier: GPL-3.0-or-later
#[path = "../../../src/rendering/export_lut.rs"]
pub mod export_lut;

#[cfg(test)]
mod tests {
    use super::export_lut::ExportLut;
    use ffmpeg_next::{format::Pixel, frame::Video};

    fn cube(invert: bool) -> Vec<u8> {
        let mut text =
            String::from("TITLE \"Test\"\nLUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    let values = if invert {
                        [1 - r, 1 - g, 1 - b]
                    } else {
                        [r, g, b]
                    };
                    text.push_str(&format!("{} {} {}\n", values[0], values[1], values[2]));
                }
            }
        }
        text.into_bytes()
    }

    fn floats(format: Pixel, width: u32, height: u32) -> Video {
        ffmpeg_next::init().unwrap();
        let mut frame = Video::new(format, width, height);
        frame.set_pts(Some(123456));
        for plane in 0..frame.planes() {
            let stride = frame.stride(plane);
            for y in 0..height as usize {
                for x in 0..width as usize {
                    let value = if plane == 3 {
                        0.375
                    } else {
                        (plane as f32 + x as f32 + 1.0) / 10.0
                    };
                    let offset = y * stride + x * 4;
                    frame.data_mut(plane)[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        frame
    }

    #[test]
    fn identity_preserves_float_precision_dimensions_and_timestamp() {
        let frame = floats(Pixel::GBRPF32LE, 8, 4);
        let output = ExportLut::new(&cube(false)).unwrap().apply(&frame).unwrap();
        assert_eq!(
            (
                output.format(),
                output.width(),
                output.height(),
                output.pts()
            ),
            (frame.format(), 8, 4, frame.pts())
        );
        for p in 0..3 {
            for y in 0..4 {
                for x in 0..8 {
                    let input = f32::from_le_bytes(
                        frame.data(p)[y * frame.stride(p) + x * 4..][..4]
                            .try_into()
                            .unwrap(),
                    );
                    let actual = f32::from_le_bytes(
                        output.data(p)[y * output.stride(p) + x * 4..][..4]
                            .try_into()
                            .unwrap(),
                    );
                    assert!((input - actual).abs() < 0.000001, "{input} != {actual}");
                }
            }
        }
    }

    #[test]
    fn chosen_transform_changes_rgb_and_preserves_alpha() {
        let frame = floats(Pixel::GBRAPF32LE, 8, 4);
        let output = ExportLut::new(&cube(true)).unwrap().apply(&frame).unwrap();
        for p in 0..4 {
            for y in 0..4 {
                for x in 0..8 {
                    let input = f32::from_le_bytes(
                        frame.data(p)[y * frame.stride(p) + x * 4..][..4]
                            .try_into()
                            .unwrap(),
                    );
                    let actual = f32::from_le_bytes(
                        output.data(p)[y * output.stride(p) + x * 4..][..4]
                            .try_into()
                            .unwrap(),
                    );
                    let expected = if p == 3 { input } else { 1.0 - input };
                    assert!(
                        (expected - actual).abs() < 0.000001,
                        "{expected} != {actual}"
                    );
                }
            }
        }
    }

    #[test]
    fn invalid_or_empty_lut_fails_instead_of_exporting_uncorrected_video() {
        assert!(ExportLut::new(b"").is_err());
        let mut invalid = ExportLut::new(b"not a cube file").unwrap();
        assert!(invalid.apply(&floats(Pixel::GBRPF32LE, 8, 4)).is_err());
    }

    #[test]
    fn graph_rebuilds_when_frame_size_changes() {
        let mut lut = ExportLut::new(&cube(false)).unwrap();
        for (w, h) in [(8, 4), (16, 8), (8, 4)] {
            let output = lut.apply(&floats(Pixel::GBRPF32LE, w, h)).unwrap();
            assert_eq!((output.width(), output.height()), (w, h));
        }
    }

    #[test]
    fn ten_bit_yuv_stays_ten_bit_and_keeps_color_properties() {
        ffmpeg_next::init().unwrap();
        let mut frame = Video::new(Pixel::YUV420P10LE, 16, 8);
        frame.set_color_space(ffmpeg_next::color::Space::BT709);
        frame.set_color_range(ffmpeg_next::color::Range::MPEG);
        for p in 0..3 {
            let (w, h) = if p == 0 { (16, 8) } else { (8, 4) };
            let stride = frame.stride(p);
            for y in 0..h {
                for x in 0..w {
                    let value: u16 = if p == 0 { 128 + x as u16 * 24 } else { 512 };
                    frame.data_mut(p)[y * stride + x * 2..][..2]
                        .copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        let output = ExportLut::new(&cube(false)).unwrap().apply(&frame).unwrap();
        assert_eq!(output.format(), Pixel::YUV420P10LE);
        assert_eq!(output.color_space(), frame.color_space());
        assert_eq!(output.color_range(), frame.color_range());
        for x in 0..16 {
            let expected = u16::from_le_bytes(frame.data(0)[x * 2..][..2].try_into().unwrap());
            let actual = u16::from_le_bytes(output.data(0)[x * 2..][..2].try_into().unwrap());
            assert!(
                (expected as i32 - actual as i32).abs() <= 2,
                "{expected} != {actual}"
            );
        }
    }
}
