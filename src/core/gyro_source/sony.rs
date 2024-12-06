// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>, Vladimir Pinchuk

use telemetry_parser::tags_impl::{ GroupedTagMap, GetWithType, GroupId, TagId, TimeVector3 };
use super::{ FileMetadata, CameraStabData, splines };
use rayon::iter::{ ParallelIterator, IntoParallelIterator };
use std::collections::BTreeMap;
use nalgebra::Vector2;
use argmin::{ core::{ CostFunction, Error, Executor }, solver::neldermead::NelderMead };

pub fn init_lens_profile(md: &mut FileMetadata, input: &telemetry_parser::Input, tag_map: &GroupedTagMap, size: (usize, usize), info: &telemetry_parser::util::SampleInfo) {
    if let Some(lmd) = tag_map.get(&GroupId::Custom("LensDistortion".into())) {
        let pixel_pitch    = tag_map.get(&GroupId::Imager).and_then(|x| x.get_t(TagId::PixelPitch)       as Option<&(u32, u32)>).cloned();
        let crop_size      = tag_map.get(&GroupId::Imager).and_then(|x| x.get_t(TagId::CaptureAreaSize)  as Option<&(f32, f32)>).cloned();
        // let sensor_size_px = tag_map.get(&GroupId::Imager).and_then(|x| x.get_t(TagId::SensorSizePixels) as Option<&(u32, u32)>).cloned();
        let mut lens_compensation_enabled = false;

        if let Some(enabled) = lmd.get_t(TagId::Enabled) as Option<&bool> {
            lens_compensation_enabled = *enabled;
        }

        if let Some(v) = lmd.get_t(TagId::Data) as Option<&serde_json::Value> {
            telemetry_parser::try_block!({
                let pixel_pitch = pixel_pitch?;
                let crop_size = crop_size?;
                // let sensor_size_px = sensor_size_px?;
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
                        let fx = focal_length;

                        let timestamp_us = (info.timestamp_ms * 1000.0).round() as i64;
                        if let Some(lp) = md.lens_params.get_mut(&timestamp_us) {
                            lp.focal_length = Some((focal_length * sensor_height / size.1 as f64 * 1000.0) as f32);
                            lp.pixel_focal_length = Some(focal_length as f32);
                            lp.distortion_coefficients = poly_coeffs.into_iter().cloned().chain(post_scale).collect();
                        }

                        if md.lens_profile.is_none() {
                            let video_rotation = info.video_rotation.unwrap_or_default().abs();
                            let is_vertical = video_rotation == 90 || video_rotation == 270;

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
                                "output_dimension": { "w": if is_vertical { size.1 } else { size.0 }, "h": if is_vertical { size.0 } else { size.1 } },
                                "frame_readout_time": md.frame_readout_time,
                                "official": true,
                                "asymmetrical": false,
                                "note": format!("Distortion comp.: {}", if lens_compensation_enabled { "On" } else { "Off" }),
                                "fisheye_params": {
                                    "camera_matrix": [
                                        [ fx, 0.0, size.0 / 2 ],
                                        [ 0.0, fx, size.1 / 2 ],
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
    let scaler             = *(gyro  .get_t(TagId::Unknown(0xe436))   as Option<&i32>).unwrap_or(&1000000) as f64;
    let original_sample_rate = sampling_frequency;

    let rounded_offset = (offset * 1000.0 * (1000000.0 / scaler)).round();
    let offset_diff = ((rounded_offset - (1000000.0 / sampling_frequency) * (rounded_offset / (1000000.0 / sampling_frequency)).floor())).round() / 1000.0;

    let frame_offset = first_frame_ts - (exposure_time / 2.0) + (md.frame_readout_time.unwrap_or_default() / 2.0) + model_offset + offset_diff - offset;

    Some((original_sample_rate, frame_offset / sampling_frequency * sample_rate))
}

#[derive(Default)]
pub struct ISTemp {
    pub frame_interval: i32,
    pub original_sample_rate: f64,
    pub first_frame_ts: Vec<f64>,
    pub pixel_pitch: (u32, u32),
    pub sensor_size: (u32, u32),
    pub per_frame_exposure: Vec<f64>,
    pub per_frame_start_idx: Vec<usize>,
    pub per_frame_crop: Vec<(f32, f32, f32, f32)>,
    pub t: Vec<i32>,
    pub ibis_x: Vec<i32>,
    pub ibis_y: Vec<i32>,
    pub ibis_a: Vec<i32>,
    pub ois_x: Vec<i32>,
    pub ois_y: Vec<i32>,
}
impl ISTemp {
    fn calc_time_diff(&self, i1: usize, i2: usize) -> Option<i32> {
        let a = i1.min(i2).min(self.t.len() - 1).max(0);
        let b = i1.max(i2).min(self.t.len() - 1).max(0);
        let mut dt = self.t.get(b)? - self.t.get(a)?;
        if dt < 0 {
            dt += self.frame_interval;
        }
        Some(dt)
    }

    fn search_idx(&self, frame: usize, top_offset: f64, time_offset: f64) -> Option<(usize, f64)> {
        let start_idx = *self.per_frame_start_idx.get(frame)?;
        let mut index = start_idx as usize;
        let mut current_time = *self.t.get(start_idx)? as f64;
        if top_offset >= 0.0 {
            while current_time <= time_offset && index < self.t.len() - 1 {
                current_time += self.calc_time_diff(index, index + 1)? as f64;
                index += 1;
            }
        } else {
            while index > 0 && current_time > time_offset {
                current_time -= self.calc_time_diff(index - 1, index)? as f64;
                index -= 1;
            }
        }
        Some((index, current_time))
    }

    fn search_top_idx2(&self, frame: usize, top_offset: f64) -> Option<(usize, f64)> {
        let (mut top_index, mut current_time) = self.search_idx(frame, top_offset, top_offset)?;
        let adj = if top_offset >= 0.0 { 2 } else { 1 };
        for _i in 0..adj{
            if top_index > 0 {
                current_time -= self.calc_time_diff(top_index - 1, top_index)? as f64;
                top_index -= 1;
            }
        }
        Some((top_index, current_time))
    }

    fn search_bot_idx2(&self, frame: usize, top_offset: f64, bot_offset: f64) -> Option<(usize, f64)> {
        let (mut bot_index, mut current_time) = self.search_idx(frame, top_offset, bot_offset)?;
        let adj = if bot_offset >= 0.0 { 2 } else { 1 };
        for _i in 0..adj{
            if bot_index > 0 {
                current_time += self.calc_time_diff(bot_index, bot_index + 1)? as f64;
                bot_index += 1;
            }
        }
        Some((bot_index, current_time))
    }
    fn calc_ofs(&self, idx: usize) -> Option<i32> {
        let mut acc_time = 0;
        for i in 0..idx {
            acc_time += self.calc_time_diff(i, i + 1)?;
        }
        Some(acc_time)
    }
}

pub fn stab_collect(is: &mut ISTemp, tag_map: &GroupedTagMap, _info: &telemetry_parser::util::SampleInfo, frame_rate: f64) -> Option<()> {
    let imager = tag_map.get(&GroupId::Imager)?;
    let ibis   = tag_map.get(&GroupId::IBIS);
    let ois    = tag_map.get(&GroupId::LensOSS);
    let gyro   = tag_map.get(&GroupId::Gyroscope)?;

    let original_sample_rate = *(gyro.get_t(TagId::Frequency) as Option<&i32>)? as f64;

    let first_frame_ts = (imager.get_t(TagId::FirstFrameTimestamp) as Option<&f64>)?;
    let exposure_time  = (imager.get_t(TagId::ExposureTime)        as Option<&f64>)?;

    let sensor_size = (imager.get_t(TagId::SensorSizePixels)  as Option<&(u32, u32)>)?;
    let pixel_pitch = (imager.get_t(TagId::PixelPitch)        as Option<&(u32, u32)>)?;
    let crop_origin = (imager.get_t(TagId::CaptureAreaOrigin) as Option<&(f32, f32)>)?;
    let crop_size   = (imager.get_t(TagId::CaptureAreaSize)   as Option<&(f32, f32)>)?;

    let start_idx = is.t.len();

    if let Some(ibis) = ibis {
        if let Some(shift) = ibis.get_t(TagId::Data) as Option<&Vec<TimeVector3<i32>>> {
            let angle = (ibis.get_t(TagId::Data2) as Option<&Vec<TimeVector3<i32>>>)?;

            assert_eq!(shift.len(), angle.len());

            // dbg!(&info.sample_index);
            // let ibis_offset = ((first_frame_ts - exposure_time / 2.0) * 1000.0 + 0.5) as i64;
            // let cur_time = ((info.sample_index as i32 as f64) * 1000000.0 / frame_rate) as i64;

            for (s, a) in shift.into_iter().zip(angle.into_iter()) {
                is.t.push(s.t);
                is.ibis_x.push(s.x);
                is.ibis_y.push(s.y);
                is.ibis_a.push(a.z);
            }
        }
    }
    if let Some(ois) = ois {
        if let Some(shift) = ois.get_t(TagId::Data) as Option<&Vec<TimeVector3<i32>>> {
            for s in shift.into_iter() {
                if is.ibis_x.is_empty() { // if `t` was not pushed by IBIS, this means we only have OIS, so push to `t` here
                    is.t.push(s.t);
                }
                is.ois_x.push(s.x);
                is.ois_y.push(s.y);
            }
        }
    }

    is.frame_interval = (1000000.0 / frame_rate) as i32;
    is.per_frame_exposure.push(exposure_time * 1000.0);
    is.per_frame_start_idx.push(start_idx);
    is.per_frame_crop.push((crop_origin.0, crop_origin.1, crop_size.0, crop_size.1));
    is.original_sample_rate = original_sample_rate;
    is.first_frame_ts.push(first_frame_ts * 1000.0);
    is.pixel_pitch = *pixel_pitch;
    is.sensor_size = *sensor_size;

    Some(())
}

pub fn stab_calc_splines(md: &FileMetadata, is_temp: &ISTemp, _sample_rate: f64, _frame_rate: f64, _size: (usize, usize)) -> Option<Vec<CameraStabData>> {
    let num_frames = is_temp.per_frame_exposure.len();

    let readout_time = (md.frame_readout_time.unwrap_or_default() * 1000.0).max(1.0);

    let per_frame_data: Vec<CameraStabData> = (0..num_frames).into_par_iter().filter_map(|frame| {
        let crop_area = *is_temp.per_frame_crop.get(frame)?; // (x, y, w, h)
        // let crop_scale = (crop_area.2 as f64 / is_temp.sensor_size.0 as f64, crop_area.3 as f64 / is_temp.sensor_size.1 as f64);
        let exposuretime = is_temp.per_frame_exposure.get(frame)?;
        let first_timestamp = is_temp.first_frame_ts.get(frame)?;
        let top_offset = first_timestamp - exposuretime / 2.0;
        let bot_offset = top_offset + readout_time;
        let entry_rate = is_temp.sensor_size.1 as f64 / readout_time; // 2166
        // dbg!(frame_interval, readout_time, first_timestamp, exposuretime, entry_rate);

        let (top_index, time) = is_temp.search_top_idx2(frame, top_offset)?;
        let n_entries = is_temp.search_bot_idx2(frame, top_offset, bot_offset)?.0 - top_index + 1;

        let ofs_rows = ((time - top_offset).abs() * entry_rate) as i64;

        // dbg!(frame, ofs_rows, is_temp.per_frame_crop.get(frame)?);

        let mut ibis_spline = splines::CatmullRom::new();
        let mut ois_spline = splines::CatmullRom::new();

        for i in 0..n_entries {
            let ts = is_temp.calc_ofs(i)? as f64 * entry_rate;
            if top_index + i < is_temp.ibis_x.len() {
                //if frame < 3 {
                //    dbg!(ts, is_temp.x[top_index + i], is_temp.y[top_index + i], is_temp.z[top_index + i]);
                //}
                ibis_spline.add_point(ts, nalgebra::Vector3::new(
                    *is_temp.ibis_x.get(top_index + i).unwrap_or(&0) as f64,
                    *is_temp.ibis_y.get(top_index + i).unwrap_or(&0) as f64,
                    *is_temp.ibis_a.get(top_index + i).unwrap_or(&0) as f64
                ));
            }
            if top_index + i < is_temp.ois_x.len() {
                ois_spline.add_point(ts, nalgebra::Vector3::new(
                    *is_temp.ois_x.get(top_index + i).unwrap_or(&0) as f64,
                    *is_temp.ois_y.get(top_index + i).unwrap_or(&0) as f64,
                    0.0
                ));
            }
        }

        Some(CameraStabData {
            offset: ofs_rows as f64,
            sensor_size: is_temp.sensor_size,
            crop_area,
            pixel_pitch: is_temp.pixel_pitch,
            ibis_spline,
            ois_spline
        })
    }).collect();

    if per_frame_data.is_empty() {
        return None;
    }

    assert_eq!(per_frame_data.len(), num_frames);

    Some(per_frame_data)
}

pub fn get_mesh_correction(tag_map: &GroupedTagMap, cache: &mut BTreeMap<u32, (Vec<f64>, Vec<f32>)>) -> Option<(Vec<f64>, Vec<f32>)> {
    let mesh_group = tag_map.get(&GroupId::Custom("MeshCorrection".into()));
    let focal_plane_group = tag_map.get(&GroupId::Custom("FocalPlaneDistortion".into()));
    let crop_origin = tag_map.get(&GroupId::Imager).and_then(|x| x.get_t(TagId::CaptureAreaOrigin) as Option<&(f32, f32)>).cloned()?;
    let crop_size   = tag_map.get(&GroupId::Imager).and_then(|x| x.get_t(TagId::CaptureAreaSize)   as Option<&(f32, f32)>).cloned()?;

    let mesh_data = mesh_group.and_then(|x| x.get_t(TagId::Data) as Option<&serde_json::Value>);
    let focal_plane_data = focal_plane_group.and_then(|x| x.get_t(TagId::Data) as Option<&serde_json::Value>);

    let crc = crc32fast::hash(serde_json::to_string(&[mesh_data.unwrap_or(&serde_json::Value::Null), focal_plane_data.unwrap_or(&serde_json::Value::Null), &crop_origin.0.into(), &crop_origin.1.into(), &crop_size.0.into(), &crop_size.1.into()]).unwrap().as_bytes());
    if cache.contains_key(&crc) {
        return cache.get(&crc).cloned();
    }

    let mut has_any_mesh_value = false;
    let mut has_any_focal_plane_value = false;
    if let Some(mesh_data) = mesh_data {
        for x in mesh_data.get("raw_mesh")?.as_array()? {
            let coord = x.as_array()?;
            if coord[0].as_f64()? != 0.0 || coord[1].as_f64()? != 0.0 {
                has_any_mesh_value = true;
                break;
            }
        }
    }
    let focal_plane_data = if let Some(focal_plane_data) = focal_plane_data {
        let unk1 = focal_plane_data.get("unk1")?.as_i64()? as f64;
        let unk2 = focal_plane_data.get("unk2")?.as_i64()? as f64;
        let scale = focal_plane_data.get("scale")?.as_f64()? as f64;
        let mut coords = vec![focal_plane_data.get("unk4")?.as_array()?.len() as f64, unk1, unk2, scale];
        for x in focal_plane_data.get("unk4")?.as_array()? {
            let coord = x.as_array()?;
            has_any_focal_plane_value = true;
            coords.push(coord[0].as_f64()? / 32768.0);
            coords.push(coord[1].as_f64()? / 32768.0);
        }
        if coords.len() == 4 { coords.clear(); coords.push(0.0); }
        else if coords[0] != 8.0 {
            log::error!("Invalid FocalPlaneDistortion data: {coords:?}");
            coords.clear();
            coords.push(0.0);
        }
        coords
    } else {
        vec![0.0]
    };

    if !has_any_mesh_value && !has_any_focal_plane_value {
        return None;
    }

    let size = (|| -> Option<(f64, f64)> {
        let mesh_data = mesh_data?;
        let size = mesh_data.get("size")?.as_array()?;
        Some((size[0].as_f64()?, size[1].as_f64()?))
    })().unwrap_or((0.0, 0.0));
    let divisions = (|| -> Option<(usize, usize)> {
        let mesh_data = mesh_data?;
        let divisions = mesh_data.get("divisions")?.as_array()?;
        Some((divisions[0].as_i64()? as usize, divisions[1].as_i64()? as usize))
    })().unwrap_or((0, 0));

    // Precompute spline coeffs for the y coordinate
    const MAX_GRID_SIZE: usize = 9;
    let mut a = [0.0; MAX_GRID_SIZE];
    let mut b = [0.0; MAX_GRID_SIZE];
    let mut c = [0.0; MAX_GRID_SIZE];
    let mut d = [0.0; MAX_GRID_SIZE];
    let mut alpha = [0.0; MAX_GRID_SIZE - 1];
    let mut mu = [0.0; MAX_GRID_SIZE];
    let mut z = [0.0; MAX_GRID_SIZE];

    let mut mesh = Vec::with_capacity(divisions.0 * divisions.1 * 2 + 9 + (divisions.1*4*2));
    mesh.push(0.0); // offset to focal_plane_data
    mesh.push(divisions.0 as f64);
    mesh.push(divisions.1 as f64);
    mesh.push(size.0 as f64);
    mesh.push(size.1 as f64);
    mesh.push(crop_origin.0 as f64);
    mesh.push(crop_origin.1 as f64);
    mesh.push(crop_size.0 as f64);
    mesh.push(crop_size.1 as f64);
    if has_any_mesh_value {
        let mesh_data = mesh_data?;
        for x in mesh_data.get("mesh")?.as_array()? {
            let coord = x.as_array()?;
            mesh.push(coord[0].as_f64()?);
            mesh.push(coord[1].as_f64()?);
        }

        for mesh_offset in 0..=1 {
            for j in 0..divisions.1 {
                splines::BivariateSpline::cubic_spline_coefficients(&mesh[9 + mesh_offset..], 2, j * divisions.0, size.0, divisions.0, &mut a, &mut b, &mut c, &mut d, &mut alpha, &mut mu, &mut z);
                for aa in a { mesh.push(aa); }
                for bb in b { mesh.push(bb); }
                for cc in c { mesh.push(cc); }
                for dd in d { mesh.push(dd); }
            }
        }
    }
    mesh[0] = mesh.len() as f64;
    mesh.extend(focal_plane_data.iter());

    let mut inv_mesh = Vec::with_capacity(mesh.len());
    inv_mesh.push(0.0); // offset to focal_plane_data
    inv_mesh.push(divisions.0 as f64);
    inv_mesh.push(divisions.1 as f64);
    inv_mesh.push(size.0 as f64);
    inv_mesh.push(size.1 as f64);
    inv_mesh.push(crop_origin.0 as f64);
    inv_mesh.push(crop_origin.1 as f64);
    inv_mesh.push(crop_size.0 as f64);
    inv_mesh.push(crop_size.1 as f64);
    if has_any_mesh_value {
        let step = ((size.0 / (divisions.0 as f64 - 1.0)), (size.1 / (divisions.1 as f64 - 1.0)));
        let grid: Vec<_> = (0..divisions.1).map(|y| {
            (0..divisions.0).map(move |x| (x as f64, y as f64))
        }).flatten().collect();

        let new_mesh: Vec<f64> = grid.into_par_iter().filter_map(|(x, y)| {
            let new_pos = inverse_interpolate_mesh(step.0 * x, step.1 * y, size, &mesh).ok()?;
            Some([new_pos.0 as f64, new_pos.1 as f64])
        }).flatten().collect();

        inv_mesh.extend(new_mesh);

        // Precompute spline coeffs for the y coordinate
        for mesh_offset in 0..=1 {
            for j in 0..divisions.1 {
                splines::BivariateSpline::cubic_spline_coefficients(&inv_mesh[9 + mesh_offset..], 2, j * divisions.0, size.0, divisions.0, &mut a, &mut b, &mut c, &mut d, &mut alpha, &mut mu, &mut z);
                for aa in a { inv_mesh.push(aa); }
                for bb in b { inv_mesh.push(bb); }
                for cc in c { inv_mesh.push(cc); }
                for dd in d { inv_mesh.push(dd); }
            }
        }
    }
    inv_mesh[0] = inv_mesh.len() as f64;
    inv_mesh.extend(focal_plane_data.iter());

    let inv_mesh = inv_mesh.iter().map(|x| *x as f32).collect::<Vec<_>>();

    cache.insert(crc, (mesh.clone(), inv_mesh.clone()));

    Some((mesh, inv_mesh))
}

pub fn interpolate_mesh(x: f64, y: f64, size: (f64, f64), mesh: &[f64]) -> Vector2<f64> {
    let grid_spline = splines::BivariateSpline::new(mesh[1] as usize, mesh[2] as usize);
    Vector2::new(
        grid_spline.interpolate(size.0, size.1, mesh, 0, x, y),
        grid_spline.interpolate(size.0, size.1, mesh, 1, x, y)
    )
}

struct Objective<'a> {
    x_prime: f64,
    y_prime: f64,
    size: (f64, f64),
    mesh: &'a [f64],
}
impl CostFunction for Objective<'_> {
    type Param = nalgebra::Vector2<f64>;
    type Output = f64;
    fn cost(&self, x: &Self::Param) -> Result<Self::Output, Error> {
        let interp_pos = interpolate_mesh(x[0], x[1], self.size, self.mesh);
        Ok((interp_pos[0] - self.x_prime).powi(2) + (interp_pos[1] - self.y_prime).powi(2))
    }
}
fn inverse_interpolate_mesh(x_prime: f64, y_prime: f64, size: (f64, f64), mesh: &[f64]) -> Result<(f64, f64), argmin::core::Error> {
    let operator = Objective { x_prime, y_prime, size, mesh };
    let solver = NelderMead::new(vec![
            Vector2::new(x_prime, y_prime),
            Vector2::new(x_prime + 0.0001, y_prime),
            Vector2::new(x_prime, y_prime + 0.0001),
        ])
        .with_sd_tolerance(1e-10)?;

    let res = Executor::new(operator, solver)
        .configure(|state| state.max_iters(200))
        .run()?;

    if let Some(coeffs) = res.state.best_param {
        Ok((coeffs[0], coeffs[1]))
    } else {
        Err(argmin::core::Error::new(argmin::core::ArgminError::InvalidParameter { text: String::new() }))
    }
}
