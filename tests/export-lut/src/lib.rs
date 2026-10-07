// SPDX-License-Identifier: GPL-3.0-or-later
#[path = "../../../src/rendering/cube_lut.rs"]
pub mod cube_lut;
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
        assert!(ExportLut::new(b"not a cube file").is_err());
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
    #[test]
    fn lut_preserves_non_square_pixel_geometry() {
        let mut frame = floats(Pixel::GBRPF32LE, 8, 4);
        unsafe {
            (*frame.as_mut_ptr()).sample_aspect_ratio =
                ffmpeg_next::ffi::AVRational { num: 4, den: 3 };
        }
        let output = ExportLut::new(&cube(false)).unwrap().apply(&frame).unwrap();
        assert_eq!(output.aspect_ratio(), frame.aspect_ratio());
    }

    #[test]
    fn adjustments_match_preview_formula_after_lut_and_preserve_alpha() {
        let frame = floats(Pixel::GBRAPF32LE, 8, 4);
        let mut filter = ExportLut::with_adjustments(Some(&cube(true)), 0.1, 0.2).unwrap();
        let output = filter.apply(&frame).unwrap();
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
                    let expected = if p == 3 {
                        input
                    } else {
                        ((1.0 - input - 0.5) * 1.2 + 0.6).clamp(0.0, 1.0)
                    };
                    assert!(
                        (actual - expected).abs() < 0.000001,
                        "{actual} != {expected}"
                    );
                }
            }
        }
        assert_eq!(output.pts(), frame.pts());
    }

    #[test]
    fn neutral_adjustments_without_lut_are_identity() {
        let frame = floats(Pixel::GBRPF32LE, 8, 4);
        let output = ExportLut::with_adjustments(None, 0.0, 0.0)
            .unwrap()
            .apply(&frame)
            .unwrap();
        for p in 0..3 {
            assert_eq!(frame.data(p), output.data(p));
        }
        assert!(ExportLut::with_adjustments(None, f64::NAN, 0.0).is_err());
        assert!(ExportLut::with_adjustments(None, 0.0, 0.51).is_err());
    }

    #[test]
    fn preview_atlas_keeps_every_float_bit_without_alpha_storage() {
        let cube = super::cube_lut::CubeLut::parse(&cube(true)).unwrap();
        let (_, _, atlas) = cube.atlas();
        for (i, entry) in cube.entries.iter().enumerate() {
            for (c, value) in entry.iter().enumerate() {
                let start = (i * 6 + c * 2) * 3;
                let decoded = f32::from_le_bytes([
                    atlas[start],
                    atlas[start + 1],
                    atlas[start + 2],
                    atlas[start + 3],
                ]);
                assert_eq!(decoded.to_bits(), value.to_bits());
            }
        }
        for invalid in [
            b"LUT_3D_SIZE 9999\n".as_slice(),
            b"LUT_3D_SIZE 2\nNaN 0 0\n",
            b"LUT_1D_SIZE 2\n",
            b"LUT_3D_SIZE 2\nDOMAIN_MAX 2 2 2\n",
        ] {
            assert!(super::cube_lut::CubeLut::parse(invalid).is_err());
        }
    }
    #[test]
    fn oversized_stream_is_bounded_before_parsing() {
        use std::io::Read;
        let mut input = std::io::repeat(b'0').take(128 * 1024 * 1024);
        assert!(super::cube_lut::CubeLut::read_bounded(&mut input).is_err());
        assert_eq!(input.limit(), 64 * 1024 * 1024 - 1);
    }
    #[test]
    fn adjustment_extremes_without_lut_clamp_rgb_and_preserve_alpha() {
        let frame = floats(Pixel::GBRAPF32LE, 8, 4);
        for brightness in [-0.5_f64, 0.5] {
            for contrast in [-0.5_f64, 0.5] {
                let output = ExportLut::with_adjustments(None, brightness, contrast)
                    .unwrap()
                    .apply(&frame)
                    .unwrap();
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
                            let expected = if p == 3 {
                                input
                            } else {
                                ((input - 0.5) * (1.0 + contrast as f32) + 0.5 + brightness as f32)
                                    .clamp(0.0, 1.0)
                            };
                            assert!(
                                (actual - expected).abs() < 0.000001,
                                "{actual} != {expected}"
                            );
                        }
                    }
                }
            }
        }
    }

    // Independent FFmpeg geq oracle: the previous export adjustment pipeline.
    fn geq_reference(frame: &Video, brightness: f64, contrast: f64) -> Video {
        use ffmpeg_next::{ffi, filter};
        let mut graph = filter::Graph::new();
        let pixel: ffi::AVPixelFormat = frame.format().into();
        let aspect = frame.aspect_ratio();
        let source_args = format!(
            "video_size={}x{}:pix_fmt={}:time_base=1/1000000:pixel_aspect={}/{}:colorspace={}:range={}",
            frame.width(),
            frame.height(),
            pixel as i32,
            aspect.numerator().max(1),
            aspect.denominator().max(1),
            frame.color_space() as i32,
            frame.color_range() as i32
        );
        graph
            .add(&filter::find("buffer").unwrap(), "in", &source_args)
            .unwrap();
        graph
            .add(&filter::find("buffersink").unwrap(), "out", "")
            .unwrap();
        let channels = ['r', 'g', 'b']
            .map(|c| {
                format!(
                    "{c}='clip(({c}(X,Y)-0.5)*{}+{},0,1)'",
                    1.0 + contrast,
                    0.5 + brightness
                )
            })
            .join(":");
        let alpha = frame.format() == Pixel::GBRAPF32LE;
        let name = unsafe { std::ffi::CStr::from_ptr(ffi::av_get_pix_fmt_name(pixel)) }
            .to_str()
            .unwrap();
        let filters = format!(
            "format={},geq={channels}:a='alpha(X,Y)':interpolation=nearest,format={name}",
            if alpha { "gbrapf32le" } else { "gbrpf32le" }
        );
        graph
            .output("in", 0)
            .unwrap()
            .input("out", 0)
            .unwrap()
            .parse(&filters)
            .unwrap();
        graph.validate().unwrap();
        let mut source = graph.get("in").unwrap();
        assert_eq!(
            unsafe {
                ffi::av_buffersrc_add_frame_flags(
                    source.as_mut_ptr(),
                    frame.as_ptr().cast_mut(),
                    ffi::AV_BUFFERSRC_FLAG_KEEP_REF as i32,
                )
            },
            0
        );
        let mut output = Video::empty();
        graph.get("out").unwrap().sink().frame(&mut output).unwrap();
        output
    }

    fn assert_active_pixels_equal(a: &Video, b: &Video) {
        assert_eq!(
            (
                a.format(),
                a.width(),
                a.height(),
                a.pts(),
                a.aspect_ratio(),
                a.color_space(),
                a.color_range()
            ),
            (
                b.format(),
                b.width(),
                b.height(),
                b.pts(),
                b.aspect_ratio(),
                b.color_space(),
                b.color_range()
            )
        );
        for p in 0..a.planes() {
            let row_bytes = match a.format() {
                Pixel::GBRPF32LE | Pixel::GBRAPF32LE => a.width() as usize * 4,
                Pixel::P010LE => a.width() as usize * 2,
                Pixel::YUV420P10LE => a.plane_width(p) as usize * 2,
                _ => panic!("Unexpected fixture format"),
            };
            for y in 0..a.plane_height(p) as usize {
                assert_eq!(
                    &a.data(p)[y * a.stride(p)..][..row_bytes],
                    &b.data(p)[y * b.stride(p)..][..row_bytes],
                    "plane {p}, row {y}"
                );
            }
        }
    }

    #[test]
    fn native_adjustments_match_geq_bits_and_do_not_mutate_shared_odd_width_frames() {
        let mut frame = floats(Pixel::GBRAPF32LE, 17, 5);
        for p in 0..4 {
            let stride = frame.stride(p);
            frame.data_mut(p).fill(0xA5);
            for y in 0..5 {
                for x in 0..17 {
                    let value = (x as f32 - 3.0) / 10.0 + p as f32 * 0.03;
                    frame.data_mut(p)[y * stride + x * 4..][..4]
                        .copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        let saved: Vec<Vec<u8>> = (0..4).map(|p| frame.data(p).to_vec()).collect();
        for (brightness, contrast) in [
            (-0.5, -0.5),
            (0.5, 0.5),
            (0.0, 0.2),
            (0.1, 0.0),
            (0.1, 0.2),
            (-0.123, 0.234),
        ] {
            let reference = geq_reference(&frame, brightness, contrast);
            let mut filter = ExportLut::with_adjustments(None, brightness, contrast).unwrap();
            for _ in 0..2 {
                let output = filter.apply(&frame).unwrap();
                assert_active_pixels_equal(&reference, &output);
                for p in 0..4 {
                    assert_eq!(frame.data(p), saved[p]);
                }
            }
        }
    }

    #[test]
    fn adjusted_ten_bit_frames_match_geq_and_rebuild_for_format_size_and_color_changes() {
        ffmpeg_next::init().unwrap();
        let mut filter = ExportLut::with_adjustments(None, 0.1, 0.2).unwrap();
        for (pixel, w, h, space, range) in [
            (
                Pixel::YUV420P10LE,
                18,
                10,
                ffmpeg_next::color::Space::BT709,
                ffmpeg_next::color::Range::MPEG,
            ),
            (
                Pixel::P010LE,
                32,
                16,
                ffmpeg_next::color::Space::BT709,
                ffmpeg_next::color::Range::MPEG,
            ),
            (
                Pixel::YUV420P10LE,
                18,
                10,
                ffmpeg_next::color::Space::BT2020NCL,
                ffmpeg_next::color::Range::JPEG,
            ),
            (
                Pixel::YUV420P10LE,
                18,
                10,
                ffmpeg_next::color::Space::Unspecified,
                ffmpeg_next::color::Range::Unspecified,
            ),
        ] {
            let mut frame = Video::new(pixel, w, h);
            frame.set_pts(Some(98765));
            frame.set_color_space(space);
            frame.set_color_range(range);
            unsafe {
                (*frame.as_mut_ptr()).sample_aspect_ratio =
                    ffmpeg_next::ffi::AVRational { num: 4, den: 3 };
            }
            for p in 0..frame.planes() {
                let stride = frame.stride(p);
                let row_samples = if pixel == Pixel::P010LE {
                    w as usize
                } else {
                    frame.plane_width(p) as usize
                };
                for y in 0..frame.plane_height(p) as usize {
                    for x in 0..row_samples {
                        let value: u16 = ((x * 37 + y * 13 + p * 97) % 1024) as u16;
                        let stored = if pixel == Pixel::P010LE {
                            value << 6
                        } else {
                            value
                        };
                        frame.data_mut(p)[y * stride + x * 2..][..2]
                            .copy_from_slice(&stored.to_le_bytes());
                    }
                }
            }
            let saved: Vec<Vec<u8>> = (0..frame.planes())
                .map(|p| frame.data(p).to_vec())
                .collect();
            let reference = geq_reference(&frame, 0.1, 0.2);
            let output = filter.apply(&frame).unwrap();
            assert_active_pixels_equal(&reference, &output);
            for p in 0..frame.planes() {
                assert_eq!(frame.data(p), saved[p]);
            }
        }
    }
}
