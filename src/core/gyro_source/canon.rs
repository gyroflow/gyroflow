// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2025 Adrian <adrian.eddy at gmail>

use telemetry_parser::tags_impl::{ GroupedTagMap, GetWithType, GroupId, TagId };
use crate::gyro_source::FileMetadata;

pub fn init_lens_profile(md: &mut FileMetadata, input: &telemetry_parser::Input, tag_map: &GroupedTagMap, size: (usize, usize), info: &telemetry_parser::util::SampleInfo) {
    if md.lens_profile.is_none() {
        if let Some(im) = tag_map.get(&GroupId::Imager) {
            if let Some(_w) = im.get_t(TagId::PixelWidth) as Option<&u32> {
                if let Some(_h) = im.get_t(TagId::PixelHeight) as Option<&u32> {
                    if let Some(map) = tag_map.get(&GroupId::Lens) {
                        if let Some(v) = map.get_t(TagId::PixelFocalLength) as Option<&Vec<f32>> {
                            if v.len() == 2 {
                                let (fx, fy) = (v[0], v[1]);

                                let video_rotation = info.video_rotation.unwrap_or_default().abs();
                                let is_vertical = video_rotation == 90 || video_rotation == 270;

                                let focal_length_str = tag_map.get(&GroupId::Lens)
                                    .and_then(|x| x.get_t(TagId::FocalLength) as Option<&f32>)
                                    .map(|x| format!("{:.2} mm", *x));

                                let mut lens_name = String::new();
                                if let Some(v) = tag_map.get(&GroupId::Lens).and_then(|map| map.get_t(TagId::DisplayName) as Option<&String>) {
                                    lens_name = v.clone();
                                }
                                md.lens_profile = Some(serde_json::json!({
                                    "calibrated_by": "Canon",
                                    "camera_brand": "Canon",
                                    "camera_model": input.camera_model().map(|x| x.as_str()).unwrap_or(&""),
                                    "lens_model":   if !lens_name.is_empty() && focal_length_str.is_some() { format!("{lens_name} ({})", focal_length_str.unwrap()) } else if !lens_name.is_empty() { lens_name } else { focal_length_str.unwrap_or_default() },
                                    "calib_dimension":  { "w": size.0, "h": size.1 },
                                    "orig_dimension":   { "w": size.0, "h": size.1 },
                                    "output_dimension": { "w": if is_vertical { size.1 } else { size.0 }, "h": if is_vertical { size.0 } else { size.1 } },
                                    "frame_readout_time": md.frame_readout_time,
                                    "official": true,
                                    "asymmetrical": false,
                                    "note": "",
                                    "fisheye_params": {
                                        "camera_matrix": [
                                            [ fx, 0.0, size.0 / 2 ],
                                            [ 0.0, fy, size.1 / 2 ],
                                            [ 0.0, 0.0, 1.0 ]
                                        ],
                                        "distortion_coeffs": []
                                    },
                                    "distortion_model": "opencv_standard",
                                    "sync_settings": {
                                        "initial_offset": 0,
                                        "initial_offset_inv": false,
                                        "search_size": 0.3,
                                        "max_sync_points": 5,
                                        "every_nth_frame": 1,
                                        "time_per_syncpoint": 0.5,
                                        "do_autosync": false
                                    },
                                    "calibrator_version": "---"
                                }));
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn get_time_offset(md: &FileMetadata, _input: &telemetry_parser::Input, tag_map: &GroupedTagMap, sample_rate: f64, fps: f64) -> Option<f64> {
    let exposure = (tag_map.get(&GroupId::Imager)?.get_t(TagId::ExposureTime) as Option<&f64>)?;
    // dbg!(&exposure);
    let frame_time = 1000.0 / md.frame_rate.unwrap_or(fps);
    let frame_readout_time = md.frame_readout_time?;
    Some(frame_time + frame_readout_time / 2.0 - (*exposure) / 2.0)
}
