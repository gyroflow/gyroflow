// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>, Vladimir Pinchuk

use telemetry_parser::tags_impl::{ GroupedTagMap, GetWithType, GroupId, TagId };
use super::FileMetadata;

pub fn init_lens_profile(md: &mut FileMetadata, input: &telemetry_parser::Input, tag_map: &GroupedTagMap, size: (usize, usize), info: &telemetry_parser::util::SampleInfo) {
    if let Some(lmd) = tag_map.get(&GroupId::Custom("LensDistortion".into())) {
        let pixel_pitch = tag_map.get(&GroupId::Imager).and_then(|x| x.get_t(TagId::PixelPitch) as Option<&(u32, u32)>).cloned();
        let crop_size = tag_map.get(&GroupId::Imager).and_then(|x| x.get_t(TagId::CaptureAreaSize) as Option<&(f32, f32)>).cloned();
        let mut lens_compensation_enabled = false;

        if let Some(enabled) = lmd.get_t(TagId::Enabled) as Option<&bool> {
            lens_compensation_enabled = *enabled;
        }

        if let Some(v) = lmd.get_t(TagId::Data) as Option<&serde_json::Value> {
            telemetry_parser::try_block!({
                let pixel_pitch = pixel_pitch?;
                let crop_size = crop_size?;
                let sensor_height = v.get("effective_sensor_height_nm")?.as_f64()? / 1e9;
                let coeff_scale = v.get("coeff_scale")?.as_f64()?;
                // let focal_length_nm = v.get("focal_length_nm")?.as_f64()?;
                let mut lens_in_ray_angle: Vec<f64> = v.get("coeffs")?.as_array()?.into_iter().filter_map(|x| Some(x.as_f64()? / coeff_scale.max(1.0) / 180.0 * std::f64::consts::PI)).collect();
                lens_in_ray_angle.insert(0, 0.0);

                let lens_out_radius = nalgebra::DVector::from_iterator(11, (0..11).map(|i| (i as f64) / 10.0 * sensor_height));

                // Fit polynomial
                let mut matrix = nalgebra::DMatrix::<f64>::zeros(11, 6);
                for (i, angle) in lens_in_ray_angle.iter().enumerate() {
                    for power in 0..6 {
                        matrix[(i, power)] = angle.powf((power + 1) as f64);
                    }
                }
                match nalgebra::SVD::new(matrix.clone(), true, true).solve(&lens_out_radius, 1e-18f64) {
                    Ok(poly_coeffs) => {
                        assert_eq!(poly_coeffs.len(), 6);
                        //////////////////////////////////////////////////
                        fn a2y(a: f64, params: &nalgebra::DVector<f64>) -> f64 {
                            let mut sum = 0.0;
                            for i in 0..6 {
                                sum += a.powi(i + 1) * params[i as usize];
                            }
                            sum
                        }
                        fn a2y_diff(a: f64, params: &nalgebra::DVector<f64>) -> f64 {
                            let mut sum = 0.0;
                            for i in 0..6 {
                                sum += (i as f64 + 1.0) * a.powi(i) * params[i as usize];
                            }
                            sum
                        }
                        fn y2a(y: f64, params: &nalgebra::DVector<f64>) -> f64 {
                            let mut x = 0.01;
                            for _ in 0..50 {
                                x = x - (a2y(x, params) - y) / a2y_diff(x, params);
                            }
                            x
                        }

                        // Calculate max possible fov
                        let sensor_crop_px = nalgebra::Vector2::new(crop_size.0 as f64, crop_size.1 as f64);
                        let pixel_pitch = nalgebra::Vector2::new(pixel_pitch.0 as f64, pixel_pitch.1 as f64) / 1e9;
                        let video_res_px = nalgebra::Vector2::new(size.0 as f64, size.1 as f64);

                        let sensor_crop = pixel_pitch.component_mul(&sensor_crop_px);
                        let pixel_pitch_scaled = sensor_crop.component_div(&video_res_px);

                        let fov_hor = y2a(sensor_crop.x / 2.0, &poly_coeffs);
                        let fov_vert = y2a(sensor_crop.y / 2.0, &poly_coeffs);
                        let fov_diag = y2a(sensor_crop.norm() / 2.0, &poly_coeffs);

                        let focal_length = (video_res_px.x / fov_hor.tan())
                            .max(video_res_px.y / fov_vert.tan())
                            .max(video_res_px.norm() / fov_diag.tan())
                            / 2.0;
                        let post_scale = [
                            1.0 / pixel_pitch_scaled.x / focal_length,
                            1.0 / pixel_pitch_scaled.y / focal_length,
                        ];

                        let timestamp_us = (info.timestamp_ms * 1000.0).round() as i64;
                        if let Some(lp) = md.lens_params.get_mut(&timestamp_us) {
                            lp.focal_length = Some((focal_length * sensor_height / size.1 as f64 * 1000.0) as f32);
                            lp.pixel_focal_length = Some(focal_length as f32);
                            if !lens_compensation_enabled {
                                lp.distortion_coefficients = poly_coeffs.into_iter().cloned().chain(post_scale).collect();
                            }
                        }

                        if md.lens_profile.is_none() {
                            let focal_length = tag_map.get(&GroupId::Lens)
                                .and_then(|x| x.get_t(TagId::FocalLength) as Option<&f32>)
                                .map(|x| format!("{:.2} mm", *x));
                            md.lens_profile = Some(serde_json::json!({
                                "calibrated_by": "Sony",
                                "camera_brand": "Sony",
                                "camera_model": input.camera_model().map(|x| x.as_str()).unwrap_or(&""),
                                "lens_model":   focal_length.unwrap_or_default(),
                                "calib_dimension":  { "w": size.0, "h": size.1 },
                                "orig_dimension":   { "w": size.0, "h": size.1 },
                                "output_dimension": { "w": size.0, "h": size.1 },
                                "frame_readout_time": md.frame_readout_time,
                                "official": true,
                                "asymmetrical": false,
                                "note": format!("Distortion comp.: {}", if lens_compensation_enabled { "On" } else { "Off" }),
                                "fisheye_params": {
                                    "camera_matrix": [
                                        [ 0.0, 0.0, size.0 / 2 ],
                                        [ 0.0, 0.0, size.1 / 2 ],
                                        [ 0.0, 0.0, 1.0 ]
                                    ],
                                    "distortion_coeffs": []
                                },
                                "distortion_model": "sony",
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
                    },
                    Err(e) => {
                        log::error!("Error fitting polynomial: {e:?}");
                    }
                }
            });
        }
    }
}


pub fn get_time_offset(md: &FileMetadata, input: &telemetry_parser::Input, tag_map: &GroupedTagMap, sample_rate: f64) -> Option<(f64, f64)> {
    let model_offset = if input.camera_model().map(|x| x == "DSC-RX0M2").unwrap_or_default() { 1.5 } else { 0.0 };
    let imager = tag_map.get(&GroupId::Imager)?;
    let gyro   = tag_map.get(&GroupId::Gyroscope)?;

    let first_frame_ts     =  (imager.get_t(TagId::FirstFrameTimestamp) as Option<&f64>)?;
    let exposure_time      =  (imager.get_t(TagId::ExposureTime)        as Option<&f64>)?;
    let offset             =  (gyro  .get_t(TagId::TimeOffset)          as Option<&f64>)?;
    let sampling_frequency = *(gyro  .get_t(TagId::Frequency)           as Option<&i32>)? as f64;
    let scaler             = *(gyro  .get_t(TagId::Unknown(0xe436))     as Option<&i32>).unwrap_or(&1000000) as f64;
    let original_sample_rate = sampling_frequency;

    let rounded_offset = (offset * 1000.0 * (1000000.0 / scaler)).round();
    let offset_diff = ((rounded_offset - (1000000.0 / sampling_frequency) * (rounded_offset / (1000000.0 / sampling_frequency)).floor())).round() / 1000.0;

    let frame_offset = first_frame_ts - (exposure_time / 2.0) + (md.frame_readout_time.unwrap_or_default() / 2.0) + model_offset + offset_diff - offset;

    Some((original_sample_rate, frame_offset / sampling_frequency * sample_rate))
}
