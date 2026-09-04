// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>, Vladimir Pinchuk

use telemetry_parser::tags_impl::{ GroupedTagMap, TagMap, GetWithType, GroupId, TagId, TagValue, TimeVector3 };
use super::{ FileMetadata, CameraStabData, TimeIMU, splines, MeshCorrections, MeshTable, MeshFrame, MESH_HEADER };
use rayon::iter::{ ParallelIterator, IntoParallelIterator };
use std::collections::BTreeMap;
use nalgebra::Vector2;
use argmin::{ core::{ CostFunction, Error, Executor }, solver::neldermead::NelderMead };
use crate::stabilization::distortion_models::sony::{ Sony, MAX_SEGMENTS };

pub mod breathing;

pub fn init_lens_profile(md: &mut FileMetadata, input: &telemetry_parser::Input, tag_map: &GroupedTagMap, size: (usize, usize), info: &telemetry_parser::util::SampleInfo) {
    if let Some(lmd) = tag_map.get(&GroupId::Custom("LensDistortion".into())) {
        let pixel_pitch    = tag_map.get(&GroupId::Imager).and_then(|x| x.get_t(TagId::PixelPitch)       as Option<&(u32, u32)>).cloned();
        let crop_size      = tag_map.get(&GroupId::Imager).and_then(|x| x.get_t(TagId::CaptureAreaSize)  as Option<&(f32, f32)>).cloned();
        let crop_origin    = tag_map.get(&GroupId::Imager).and_then(|x| x.get_t(TagId::CaptureAreaOrigin) as Option<&(f32, f32)>).cloned();
        let sensor_size_px = tag_map.get(&GroupId::Imager).and_then(|x| x.get_t(TagId::SensorSizePixels) as Option<&(u32, u32)>).cloned();
        let mut lens_compensation_enabled = false;

        // The optical axis stays at the center of the sensor (the IBIS shift is handled separately), but the capture area
        // moves around the sensor when the in-camera electronic stabilization is active, so the principal point in video
        // pixels has to follow the capture area origin per frame.
        let principal_point = (|| -> Option<(f32, f32)> {
            let crop_size = crop_size?;
            let crop_origin = crop_origin?;
            let sensor_size = sensor_size_px?;
            if crop_size.0 <= 0.0 || crop_size.1 <= 0.0 { return None; }
            let sensor_size = (
                if crop_origin.0 + crop_size.0 > sensor_size.0 as f32 { crop_size.0 + crop_origin.0 * 2.0 } else { sensor_size.0 as f32 } as f64,
                if crop_origin.1 + crop_size.1 > sensor_size.1 as f32 { crop_size.1 + crop_origin.1 * 2.0 } else { sensor_size.1 as f32 } as f64
            );
            Some((
                ((sensor_size.0 / 2.0 - crop_origin.0 as f64) * size.0 as f64 / crop_size.0 as f64) as f32,
                ((sensor_size.1 / 2.0 - crop_origin.1 as f64) * size.1 as f64 / crop_size.1 as f64) as f32
            ))
        })();

        if let Some(enabled) = lmd.get_t(TagId::Enabled) as Option<&bool> {
            lens_compensation_enabled = *enabled;
        }

        if let Some(v) = lmd.get_t(TagId::Data) as Option<&serde_json::Value> {
            telemetry_parser::try_block!({
                let pixel_pitch = pixel_pitch?;
                let crop_size = crop_size?;

                let video_rotation = info.video_rotation.unwrap_or_default().abs();
                let is_vertical = video_rotation == 90 || video_rotation == 270;

                let focal_length_str = tag_map.get(&GroupId::Lens)
                    .and_then(|x| x.get_t(TagId::FocalLength) as Option<&f32>)
                    .map(|x| format!("{:.2} mm", *x));

                let focal_length_mm = v.get("focal_length_nm")?.as_f64()? / 1000000.0;
                let approx_focal_length_mm = tag_map.get(&GroupId::Lens).and_then(|x| x.get_t(TagId::FocalLength) as Option<&f32>).map(|x| *x as f64).unwrap_or(focal_length_mm);

                let ratio = approx_focal_length_mm / focal_length_mm.max(0.000001);

                let is_bad_focal_length = (ratio - 1.0).abs() > 0.5;
                if is_bad_focal_length {
                    log::error!("Bad focal length: {approx_focal_length_mm} -> {focal_length_mm}");
                }

                let sensor_height = v.get("effective_sensor_height_nm")?.as_f64()? / 1e9;
                let coeff_scale = v.get("coeff_scale")?.as_f64()?;
                let lens_in_ray_angle: Vec<f64> = v.get("coeffs")?.as_array()?.into_iter().filter_map(|x| Some(x.as_f64()? / coeff_scale.max(1.0) / 180.0 * std::f64::consts::PI)).collect();
                if lens_in_ray_angle.is_empty() || sensor_height == 0.0 || is_bad_focal_length {
                    let sensor_size_px = tag_map.get(&GroupId::Imager).and_then(|x| x.get_t(TagId::SensorSizePixels) as Option<&(u32, u32)>).cloned()?;

                    let focal_length_mm = if is_bad_focal_length { approx_focal_length_mm } else { v.get("focal_length_nm")?.as_f64()? / 1000000.0 };
                    let sws = crop_size.0 as f64 / (sensor_size_px.0 as f64).max(1.0);
                    let shs = crop_size.1 as f64 / (sensor_size_px.1 as f64).max(1.0);

                    let sw = tag_map.get(&GroupId::Default).and_then(|x| x.get_t(TagId::SensorWidth)  as Option<&f32>).map(|x| *x).unwrap_or_default() as f64 * sws;
                    let sh = tag_map.get(&GroupId::Default).and_then(|x| x.get_t(TagId::SensorHeight) as Option<&f32>).map(|x| *x).unwrap_or_default() as f64 * shs;

                    // Create fallback lens profile without distortion correction, but only with focal length
                    if focal_length_mm > 0.0 && sw > 0.0 && sh > 0.0 {
                        let fx = focal_length_mm / sw * size.0 as f64;
                        let fy = focal_length_mm / sh * size.1 as f64;
                        let timestamp_us = (info.timestamp_ms * 1000.0).round() as i64;
                        if let Some(lp) = md.lens_params.get_mut(&timestamp_us) {
                            lp.focal_length = Some(focal_length_mm as f32);
                            lp.pixel_focal_length = Some((fx as f32, fy as f32));
                            lp.principal_point = principal_point;
                        }
                        if md.lens_profile.is_none() {
                            let mut lens_name = String::new();
                            if let Some(v) = tag_map.get(&GroupId::Lens).and_then(|map| map.get_t(TagId::DisplayName) as Option<&String>) {
                                lens_name = v.clone();
                            }
                            md.lens_profile = Some(serde_json::json!({
                                "calibrated_by": "Not calibrated",
                                "camera_brand": "Sony",
                                "camera_model": input.camera_model().map(|x| x.as_str()).unwrap_or(&""),
                                "lens_model":   if !lens_name.is_empty() && focal_length_str.is_some() { format!("{lens_name} ({})", focal_length_str.unwrap()) } else if !lens_name.is_empty() { lens_name } else { focal_length_str.unwrap_or_default() },
                                "calib_dimension":  { "w": size.0, "h": size.1 },
                                "orig_dimension":   { "w": size.0, "h": size.1 },
                                "output_dimension": { "w": if is_vertical { size.1 } else { size.0 }, "h": if is_vertical { size.0 } else { size.1 } },
                                "frame_readout_time": md.frame_readout_time,
                                "official": false,
                                "asymmetrical": false,
                                "note": format!("Distortion comp.: {}", if lens_compensation_enabled { "On" } else { "Off" }),
                                "fisheye_params": {
                                    "camera_matrix": [
                                        [ fx, 0.0, size.0 / 2 ],
                                        [ 0.0, fy, size.1 / 2 ],
                                        [ 0.0, 0.0, 1.0 ]
                                    ],
                                    "distortion_coeffs": []
                                },
                                "sync_settings": { },
                                "calibrator_version": "---"
                            }));
                        }
                    }
                    return None;
                }
                let pixel_pitch_m  = nalgebra::Vector2::new(pixel_pitch.0 as f64, pixel_pitch.1 as f64) / 1e9;
                let sensor_crop_px = nalgebra::Vector2::new(crop_size.0 as f64, crop_size.1 as f64);
                let video_res_px   = nalgebra::Vector2::new(size.0 as f64, size.1 as f64);

                // Effective meters-per-output-pixel after the sensor → output resize.
                let pixel_pitch_scaled = pixel_pitch_m.component_mul(&sensor_crop_px).component_div(&video_res_px);

                // Single physical focal length (meters) used to normalize the lens curve.
                let f_meters = focal_length_mm / 1000.0;

                // Per-axis pixel focal length: f_meters / (meters per output pixel).
                let fx = f_meters / pixel_pitch_scaled.x;
                let fy = f_meters / pixel_pitch_scaled.y;

                // Sony's stabilizer evaluates the lens curve as a natural cubic spline through the ray angles at radii
                // i/N × effective sensor height; build the same spline in normalized units (radius / focal length)
                if lens_in_ray_angle.len() != MAX_SEGMENTS {
                    log::warn!("Sony lens curve has {} knots, expected {MAX_SEGMENTS}", lens_in_ray_angle.len());
                }
                let n = lens_in_ray_angle.len().min(MAX_SEGMENTS);
                let normalized = Sony::coefficients_from_lens_curve(&lens_in_ray_angle, sensor_height / n as f64 / f_meters);

                let timestamp_us = (info.timestamp_ms * 1000.0).round() as i64;
                if let Some(lp) = md.lens_params.get_mut(&timestamp_us) {
                    lp.focal_length = Some(focal_length_mm as f32);
                    lp.pixel_focal_length = Some((fx as f32, fy as f32));
                    lp.principal_point = principal_point;
                    lp.distortion_coefficients = normalized;
                }

                if md.lens_profile.is_none() {
                    let mut lens_name = String::new();
                    if let Some(v) = tag_map.get(&GroupId::Lens).and_then(|map| map.get_t(TagId::DisplayName) as Option<&String>) {
                        lens_name = v.clone();
                    }
                    md.lens_profile = Some(serde_json::json!({
                        "calibrated_by": "Sony",
                        "camera_brand": "Sony",
                        "camera_model": input.camera_model().map(|x| x.as_str()).unwrap_or(&""),
                        "lens_model":   if !lens_name.is_empty() && focal_length_str.is_some() { format!("{lens_name} ({})", focal_length_str.unwrap()) } else if !lens_name.is_empty() { lens_name } else { focal_length_str.unwrap_or_default() },
                        "calib_dimension":  { "w": size.0, "h": size.1 },
                        "orig_dimension":   { "w": size.0, "h": size.1 },
                        "output_dimension": { "w": if is_vertical { size.1 } else { size.0 }, "h": if is_vertical { size.0 } else { size.1 } },
                        "frame_readout_time": md.frame_readout_time,
                        "official": true,
                        "asymmetrical": false,
                        "note": format!("Distortion comp.: {}", if lens_compensation_enabled { "On" } else { "Off" }),
                        "fisheye_params": {
                            "camera_matrix": [
                                [ fx,  0.0, size.0 / 2 ],
                                [ 0.0, fy,  size.1 / 2 ],
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
            });
        }
    }
}


/// Fixes up per-frame data from project files written by older Gyroflow versions
pub fn upgrade_legacy_metadata(md: &mut FileMetadata) {
    upgrade_legacy_lens_params(md);
    upgrade_legacy_mesh_buffers(md);
}

/// Project files of older versions stored one `(forward, inverse)` buffer pair per frame (`FileMetadata::legacy_mesh_correction`),
/// fold them into the shared tables the renderer reads now, see `MeshCorrections::from_legacy`
pub fn upgrade_legacy_mesh_buffers(md: &mut FileMetadata) {
    if !md.legacy_mesh_correction.is_empty() {
        md.mesh_correction = MeshCorrections::from_legacy(std::mem::take(&mut md.legacy_mesh_correction));
    }
}

/// Older Gyroflow versions stored the Sony lens curve per frame as a 6-term polynomial r(θ) = Σ p_i·θ^(i+1) (normalized
/// units); project files with embedded metadata can still carry those. Convert them to the spline block the model uses now.
pub fn upgrade_legacy_lens_params(md: &mut FileMetadata) {
    let lens_profile = match md.lens_profile.as_ref() { Some(x) => x, None => return };
    if lens_profile.get("distortion_model").and_then(|x| x.as_str()) != Some("sony") { return; }
    let half_diagonal_px = lens_profile.get("calib_dimension").and_then(|d| Some((d.get("w")?.as_f64()?.powi(2) + d.get("h")?.as_f64()?.powi(2)).sqrt() / 2.0));
    for lp in md.lens_params.values_mut() {
        if lp.distortion_coefficients.len() == 6 {
            match (half_diagonal_px, lp.pixel_focal_length) {
                (Some(hd), Some((fx, _))) if fx > 0.0 => { lp.distortion_coefficients = Sony::coefficients_from_legacy_polynomial(&lp.distortion_coefficients, hd / fx as f64); }
                _ => lp.distortion_coefficients.clear()
            }
        }
    }
}

/// Timing of one gyro packet, from the `Gyroscope` tags of its frame: the packet's first sample is `offset_ms` after the frame's
/// timestamp and the samples are `period_ms` apart. Tag 0xe437 is the offset in ticks of 1/0xe436 s (the unit tag defaults to
/// microseconds and the parser reports the tick count divided by 1000, which is already ms for that unit) and 0xe435 is the
/// sample rate. The SDK keeps the offset in whole microseconds (`ImuGL::read_imu_data`): `offset_us` is that rounded value
/// and `offset_ms` the same number in ms. Both readers of the packet timing, the re-timed IMU samples and the per-frame time
/// offsets, decode the tags here so they can't drift apart. `None` when a tag is missing or the sample rate isn't positive
pub struct GyroPacketTiming { pub offset_us: f64, pub offset_ms: f64, pub frequency: f64 }
impl GyroPacketTiming {
    pub fn period_ms(&self) -> f64 { 1000.0 / self.frequency }
    pub fn period_us(&self) -> f64 { 1_000_000.0 / self.frequency }
}
pub fn gyro_packet_timing(gyro: &TagMap) -> Option<GyroPacketTiming> {
    let offset    = *(gyro.get_t(TagId::TimeOffset) as Option<&f64>)?;
    let frequency = *(gyro.get_t(TagId::Frequency)  as Option<&i32>)?;
    if frequency <= 0 { return None; }
    // A missing or invalid unit is microseconds, like the SDK assumes
    let ticks_per_second = (gyro.get_t(TagId::Unknown(0xe436)) as Option<&i32>).copied().filter(|x| *x > 0).unwrap_or(1_000_000) as f64;
    let offset_us = (offset * 1000.0 * (1_000_000.0 / ticks_per_second)).round();
    Some(GyroPacketTiming { offset_us, offset_ms: offset_us / 1000.0, frequency: frequency as f64 })
}

/// Puts the merged IMU samples on the camera's own packet timeline, like the reference SDK (`ImuGL::read_imu_data`): sample `i`
/// of the gyro packet of frame `N` is at `ts(N) + TimeOffset(N) + i / Frequency(N)` (see [`gyro_packet_timing`]).
/// `imu` comes from `normalized_imu_interpolated`, which keeps the packet order with one entry per gyro sample but spaces them
/// uniformly over the clip; that spacing drifts away from the packets by up to a few milliseconds and jitters by one sample
/// between frames. Returns false and leaves `imu` untouched when the packets don't add up to the sample list.
pub fn retime_imu_from_packets(imu: &mut Vec<TimeIMU>, samples: &[telemetry_parser::util::SampleInfo]) -> bool {
    let started = std::time::Instant::now();
    // Validate the packets frame by frame first, then write the timestamps in place in a single pass over the samples
    struct Packet { start_ms: f64, period_ms: f64, count: usize }
    let mut packets = Vec::with_capacity(samples.len());
    let mut total = 0usize;
    for info in samples {
        let Some(gyro) = info.tag_map.as_ref().and_then(|tm| tm.get(&GroupId::Gyroscope)) else { continue };
        let count = match gyro.get(&TagId::Data).map(|d| &d.value) {
            Some(TagValue::Vec_Vector3_i16(a)) => a.get().len(),
            Some(TagValue::Vec_Vector3_f32(a)) => a.get().len(),
            Some(TagValue::Vec_TimeVector3_f64(a)) => a.get().len(),
            _ => 0
        };
        if count == 0 { continue; }
        let Some(timing) = gyro_packet_timing(gyro) else { return false };
        packets.push(Packet { start_ms: info.timestamp_ms + timing.offset_ms, period_ms: timing.period_ms(), count });
        total += count;
    }
    if total == 0 || total != imu.len() {
        log::warn!("Sony gyro packets ({total}) don't match the IMU sample count ({}), keeping the uniform timing", imu.len());
        return false;
    }
    let mut monotonic = true;
    let mut prev = f64::NEG_INFINITY;
    let mut samples_it = imu.iter_mut();
    for p in &packets {
        for i in 0..p.count {
            let x = samples_it.next().unwrap(); // the counts were verified above
            x.timestamp_ms = p.start_ms + i as f64 * p.period_ms;
            monotonic &= x.timestamp_ms > prev;
            prev = x.timestamp_ms;
        }
    }
    // Consecutive packets can in principle touch or overlap; the list has to stay monotonic for the integrators
    if !monotonic {
        imu.sort_by(|a, b| a.timestamp_ms.total_cmp(&b.timestamp_ms));
    }
    log::debug!("Sony gyro samples re-timed from {} packets in {:?}{}", packets.len(), started.elapsed(), if monotonic { "" } else { " (sorted)" });
    true
}

/// Time offset of the frame centre relative to the frame's video timestamp, in gyro time.
/// `packet_timed`: the gyro samples carry the packet timestamps from `retime_imu_from_packets`.
pub fn get_time_offset(md: &FileMetadata, input: &telemetry_parser::Input, tag_map: &GroupedTagMap, sample_rate: f64, packet_timed: bool) -> Option<(f64, f64)> {
    let model_offset = if input.camera_model().map(|x| x == "DSC-RX0M2").unwrap_or_default() { 1.5 } else { 0.0 };
    let imager = tag_map.get(&GroupId::Imager)?;
    let gyro   = tag_map.get(&GroupId::Gyroscope)?;

    let first_frame_ts = (imager.get_t(TagId::FirstFrameTimestamp) as Option<&f64>)?;
    let exposure_time  = (imager.get_t(TagId::ExposureTime)        as Option<&f64>)?;
    let timing = gyro_packet_timing(gyro)?;
    let original_sample_rate = timing.frequency;
    let readout_time = md.frame_readout_time.unwrap_or_default();

    if packet_timed {
        // The frame is placed exactly like the reference SDK does (`ImuGL::get_sample_timings`): the first sensor row is exposed
        // at `first_ts - exposure/2` and the readout spans the whole sensor height. The frame centre used by the kernels is the
        // middle of the captured area, which is not the sensor centre when the capture area is off-centre (dynamic EIS).
        let centre = (|| -> Option<f64> {
            let origin = imager.get_t(TagId::CaptureAreaOrigin) as Option<&(f32, f32)>;
            let size   = imager.get_t(TagId::CaptureAreaSize)   as Option<&(f32, f32)>;
            let sensor = imager.get_t(TagId::SensorSizePixels)  as Option<&(u32, u32)>;
            let (origin, size, sensor) = (origin?, size?, sensor?);
            if size.1 <= 0.0 { return None; }
            // Some cameras report a sensor smaller than the capture area, the SDK then assumes the capture area is centered
            let sensor_h = if origin.1 + size.1 > sensor.1 as f32 { size.1 + origin.1 * 2.0 } else { sensor.1 as f32 } as f64;
            if sensor_h <= 0.0 { return None; }
            Some((origin.1 as f64 + size.1 as f64 / 2.0) / sensor_h)
        })().unwrap_or(0.5);
        let frame_offset = first_frame_ts - (exposure_time / 2.0) + readout_time * centre + model_offset;
        return Some((original_sample_rate, frame_offset));
    }

    // Uniformly spaced gyro timeline: the packet phase is approximated with the SDK's sample remainder (whole microseconds)
    // and the metadata times are rescaled to the measured sample rate
    let period_us = timing.period_us();
    let offset_diff = (timing.offset_us - period_us * (timing.offset_us / period_us).floor()).round() / 1000.0;

    let frame_offset = first_frame_ts - (exposure_time / 2.0) + (readout_time / 2.0) + model_offset + offset_diff - timing.offset_ms;

    Some((original_sample_rate, frame_offset / timing.frequency * sample_rate))
}

/// Sensor (IBIS) or lens (OIS) stabilizer position samples, with their own timing
#[derive(Default)]
pub struct ISSamples {
    pub per_frame_start_idx: Vec<usize>,
    pub t: Vec<i32>,
    pub x: Vec<i32>,
    pub y: Vec<i32>,
    pub a: Vec<i32>,
}
impl ISSamples {
    fn calc_time_diff(&self, frame_interval: i32, i1: usize, i2: usize) -> Option<i32> {
        if self.t.is_empty() { return None; }
        let a = i1.min(i2).min(self.t.len() - 1);
        let b = i1.max(i2).min(self.t.len() - 1);
        let mut dt = self.t.get(b)? - self.t.get(a)?;
        if dt <= 0 { // wrapped to the next frame, or a duplicated/clamped sample
            dt += frame_interval;
        }
        Some(dt)
    }

    fn search_idx(&self, frame_interval: i32, frame: usize, top_offset: f64, time_offset: f64) -> Option<(usize, f64)> {
        // Frames past the last sample (a stabilizer that stopped reporting) search from the last sample, like the reference clamps its lookups
        let start_idx = (*self.per_frame_start_idx.get(frame)?).min(self.t.len().checked_sub(1)?);
        let mut index = start_idx;
        let mut current_time = *self.t.get(start_idx)? as f64;
        if top_offset >= 0.0 {
            while current_time <= time_offset && index < self.t.len() - 1 {
                current_time += self.calc_time_diff(frame_interval, index, index + 1)? as f64;
                index += 1;
            }
        } else {
            while index > 0 && current_time > time_offset {
                current_time -= self.calc_time_diff(frame_interval, index - 1, index)? as f64;
                index -= 1;
            }
        }
        Some((index, current_time))
    }

    fn search_top_idx2(&self, frame_interval: i32, frame: usize, top_offset: f64) -> Option<(usize, f64)> {
        let (mut top_index, mut current_time) = self.search_idx(frame_interval, frame, top_offset, top_offset)?;
        let adj = if top_offset >= 0.0 { 2 } else { 1 };
        for _i in 0..adj {
            if top_index > 0 {
                current_time -= self.calc_time_diff(frame_interval, top_index - 1, top_index)? as f64;
                top_index -= 1;
            }
        }
        Some((top_index, current_time))
    }

    fn search_bot_idx2(&self, frame_interval: i32, frame: usize, top_offset: f64, bot_offset: f64) -> Option<(usize, f64)> {
        let (mut bot_index, mut current_time) = self.search_idx(frame_interval, frame, top_offset, bot_offset)?;
        let adj = if bot_offset >= 0.0 { 2 } else { 1 };
        for _i in 0..adj {
            if bot_index > 0 {
                current_time += self.calc_time_diff(frame_interval, bot_index, bot_index + 1)? as f64;
                bot_index += 1;
            }
        }
        Some((bot_index, current_time))
    }
    fn calc_ofs(&self, frame_interval: i32, idx: usize) -> Option<i32> {
        let mut acc_time = 0;
        for i in 0..idx {
            acc_time += self.calc_time_diff(frame_interval, i, i + 1)?;
        }
        Some(acc_time)
    }

    /// Builds the per-row spline for one frame, returns the spline and the row offset of its first point, like the reference `VibrationProofGL::set_vp_frame`
    fn calc_spline(&self, frame_interval: i32, frame: usize, top_offset: f64, readout_time: f64, entry_rate: f64) -> Option<(splines::CatmullRom<nalgebra::Vector3<f64>>, f64)> {
        let mut spline = splines::CatmullRom::new();
        if self.t.is_empty() { return Some((spline, 0.0)); }
        let bot_offset = top_offset + readout_time;
        let (top_index, time) = self.search_top_idx2(frame_interval, frame, top_offset)?;
        let (bot_index, bot_time) = self.search_bot_idx2(frame_interval, frame, top_offset, bot_offset)?;
        let n_entries = bot_index - top_index + 1;

        // The reference doesn't offset the rows when the found samples don't span the whole readout (only happens at the clip edges)
        let ofs_rows = if bot_time - time >= readout_time { ((time - top_offset).abs() * entry_rate) as i64 } else { 0 };

        for i in 0..n_entries {
            // Note: the accumulated time is intentionally summed from the first sample, the reference does the same
            let ts = self.calc_ofs(frame_interval, i)? as f64 * entry_rate;
            if top_index + i < self.x.len() {
                spline.add_point(ts, nalgebra::Vector3::new(
                    *self.x.get(top_index + i).unwrap_or(&0) as f64,
                    *self.y.get(top_index + i).unwrap_or(&0) as f64,
                    *self.a.get(top_index + i).unwrap_or(&0) as f64
                ));
            }
        }
        Some((spline, ofs_rows as f64))
    }
}

#[derive(Default)]
pub struct ISTemp {
    pub frame_interval: i32,
    pub original_sample_rate: f64,
    pub first_frame_ts: Vec<f64>,
    pub pixel_pitch: (u32, u32),
    pub sensor_size: (u32, u32),
    pub per_frame_exposure: Vec<f64>,
    pub per_frame_crop: Vec<(f32, f32, f32, f32)>,
    pub ibis: ISSamples,
    pub ois: ISSamples,
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

    // Nothing below may fail: the per-frame vectors (start indices, exposure, crop, timestamp) must stay the same length
    // The sensor and the lens stabilizers are sampled independently, keep separate timing for each of them
    is.ibis.per_frame_start_idx.push(is.ibis.t.len());
    is.ois.per_frame_start_idx.push(is.ois.t.len());

    if let Some(shift) = ibis.and_then(|x| x.get_t(TagId::Data) as Option<&Vec<TimeVector3<i32>>>) {
        // Rotation samples come in a second table; a missing or short one only loses the roll, not the frame
        let angle = ibis.and_then(|x| x.get_t(TagId::Data2) as Option<&Vec<TimeVector3<i32>>>);
        if angle.map_or(true, |a| a.len() != shift.len()) {
            log::warn!("IBIS position and rotation sample counts differ: {} vs {:?}", shift.len(), angle.map(|a| a.len()));
        }
        for (i, s) in shift.iter().enumerate() {
            is.ibis.t.push(s.t);
            is.ibis.x.push(s.x);
            is.ibis.y.push(s.y);
            is.ibis.a.push(angle.and_then(|a| a.get(i)).map(|a| a.z).unwrap_or(0));
        }
    }
    if let Some(ois) = ois {
        if let Some(shift) = ois.get_t(TagId::Data) as Option<&Vec<TimeVector3<i32>>> {
            // A single (-1, -1, -1, -1) entry means the lens doesn't report its stabilizer position
            let unsupported = shift.len() == 1 && *shift.first().unwrap() == (TimeVector3 { t: -1, x: -1, y: -1, z: -1 });
            if !unsupported {
                for s in shift.into_iter() {
                    is.ois.t.push(s.t);
                    is.ois.x.push(s.x);
                    is.ois.y.push(s.y);
                    is.ois.a.push(0);
                }
            }
        }
    }

    is.frame_interval = (1000000.0 / frame_rate) as i32;
    is.per_frame_exposure.push(exposure_time * 1000.0);
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
        let exposuretime = is_temp.per_frame_exposure.get(frame)?;
        let first_timestamp = is_temp.first_frame_ts.get(frame)?;
        let top_offset = first_timestamp - exposuretime / 2.0;
        let entry_rate = is_temp.sensor_size.1 as f64 / readout_time; // rows per microsecond

        // A stabilizer without usable samples for this frame gets an empty spline (no shift); the frame itself is kept so that
        // the per-frame data stays aligned with the video frames
        let (ibis_spline, offset) = is_temp.ibis.calc_spline(is_temp.frame_interval, frame, top_offset, readout_time, entry_rate).unwrap_or_else(|| (splines::CatmullRom::new(), 0.0));
        let (ois_spline, ois_offset) = is_temp.ois.calc_spline(is_temp.frame_interval, frame, top_offset, readout_time, entry_rate).unwrap_or_else(|| (splines::CatmullRom::new(), 0.0));

        Some(CameraStabData {
            offset,
            ois_offset: Some(ois_offset),
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

    if per_frame_data.len() != num_frames {
        log::error!("Sony stabilizer data is incomplete: {} of {num_frames} frames", per_frame_data.len());
        return None;
    }

    Some(per_frame_data)
}

/// Above this residual of the inverse mesh lookup, `|forward(inverse(q)) - q|` in video pixels at the worst of
/// `MESH_RESIDUAL_GRID`² sensor positions, the kernels refine every lookup against the camera's mesh
/// (`MeshTable::refinement`), two more spline evaluations per pixel; below it the 9x9 inverse is taken as it is. A
/// quarter pixel is within the blur of the resampling filter
pub const MESH_REFINE_THRESHOLD_PX: f64 = 0.25;
/// In the kernels: a first correction shorter than this (sensor pixels) leaves the second iteration nothing to do. The
/// error left after applying a correction `d` is about `|J - I|·|d|` with `J` the Jacobian of the mesh, a few percent
/// of `d` for a lens mesh. The kernels compare against its square
pub const MESH_REFINE_SKIP_PX: f64 = 0.25;
/// Positions per axis the residual is sampled at, four per cell of the 9x9 grid: a few dozen microseconds per table
/// next to the two milliseconds its inversion takes, and a zoom changes the mesh on every frame
const MESH_RESIDUAL_GRID: usize = 33;

/// The correction of one frame (`MeshCorrections`): the mesh, shared with every frame that has the same one (keyed by
/// its content and the capture area size in `cache`, which maps to the table index in `meshes`), and the frame's own
/// capture area and focal plane table. `None` when the frame has neither. `video_size` is the video's, which the
/// capture area is resized to: the refinement decision is made in its pixels
pub fn get_mesh_correction(tag_map: &GroupedTagMap, video_size: (usize, usize), meshes: &mut MeshCorrections, cache: &mut BTreeMap<u32, u32>) -> Option<MeshFrame> {
    let mesh_group = tag_map.get(&GroupId::Custom("MeshCorrection".into()));
    let focal_plane_group = tag_map.get(&GroupId::Custom("FocalPlaneDistortion".into()));
    let crop_origin = tag_map.get(&GroupId::Imager).and_then(|x| x.get_t(TagId::CaptureAreaOrigin) as Option<&(f32, f32)>).cloned()?;
    let crop_size   = tag_map.get(&GroupId::Imager).and_then(|x| x.get_t(TagId::CaptureAreaSize)   as Option<&(f32, f32)>).cloned()?;

    let mesh_data = mesh_group.and_then(|x| x.get_t(TagId::Data) as Option<&serde_json::Value>);
    let focal_plane_data = focal_plane_group.and_then(|x| x.get_t(TagId::Data) as Option<&serde_json::Value>);

    let mut has_any_mesh_value = false;
    if let Some(mesh_data) = mesh_data {
        for x in mesh_data.get("raw_mesh")?.as_array()? {
            let coord = x.as_array()?;
            if coord[0].as_f64()? != 0.0 || coord[1].as_f64()? != 0.0 {
                has_any_mesh_value = true;
                break;
            }
        }
    }
    let focal_plane = focal_plane_data.and_then(parse_focal_plane).unwrap_or_default();

    if !has_any_mesh_value && focal_plane.is_empty() {
        return None;
    }

    let size = (|| -> Option<(f64, f64)> {
        let size = mesh_data?.get("size")?.as_array()?;
        Some((size[0].as_f64()?, size[1].as_f64()?))
    })().unwrap_or((0.0, 0.0));
    let divisions = (|| -> Option<(usize, usize)> {
        let divisions = mesh_data?.get("divisions")?.as_array()?;
        Some((divisions[0].as_i64()? as usize, divisions[1].as_i64()? as usize))
    })().unwrap_or((0, 0));

    let table = if has_any_mesh_value {
        let mesh_data = mesh_data?;
        let key = mesh_key(mesh_data, size, divisions, crop_size)?;
        Some(match cache.get(&key) {
            Some(index) => *index,
            None => {
                let table = build_mesh_table(mesh_data, size, divisions, crop_size, video_size)?;
                meshes.tables.push(table);
                let index = (meshes.tables.len() - 1) as u32;
                cache.insert(key, index);
                index
            }
        })
    } else {
        None
    };
    Some(MeshFrame {
        table,
        mesh_size: size,
        crop_origin: (crop_origin.0 as f64, crop_origin.1 as f64),
        crop_size: (crop_size.0 as f64, crop_size.1 as f64),
        focal_plane,
    })
}

/// `[count, unk1, band height, scale, count × (x, y)]` of the frame's `FocalPlaneDistortion` table, empty when the table
/// is empty or not the 8 bands the kernels apply
fn parse_focal_plane(data: &serde_json::Value) -> Option<Vec<f64>> {
    let bands = data.get("unk4")?.as_array()?;
    if bands.is_empty() { return None; }
    let mut coords = vec![bands.len() as f64, data.get("unk1")?.as_i64()? as f64, data.get("unk2")?.as_i64()? as f64, data.get("scale")?.as_f64()?];
    for x in bands {
        let coord = x.as_array()?;
        coords.push(coord.get(0)?.as_f64()? / 32768.0);
        coords.push(coord.get(1)?.as_f64()? / 32768.0);
    }
    if coords[0] != 8.0 {
        log::error!("Invalid FocalPlaneDistortion data: {coords:?}");
        return None;
    }
    Some(coords)
}

/// Identity of a mesh table: the grid, its extent and the capture area size it's resized from (which sets the pixel
/// scale of the refinement decision). The capture area origin is not part of it, it moves with the in-camera
/// stabilization while the mesh stays the same
fn mesh_key(mesh_data: &serde_json::Value, size: (f64, f64), divisions: (usize, usize), crop_size: (f32, f32)) -> Option<u32> {
    let mut hasher = crc32fast::Hasher::new();
    let mut feed = |v: f64| hasher.update(&v.to_bits().to_le_bytes());
    feed(size.0); feed(size.1);
    feed(divisions.0 as f64); feed(divisions.1 as f64);
    feed(crop_size.0 as f64); feed(crop_size.1 as f64);
    for x in mesh_data.get("mesh")?.as_array()? {
        let coord = x.as_array()?;
        feed(coord.get(0)?.as_f64()?);
        feed(coord.get(1)?.as_f64()?);
    }
    Some(hasher.finalize())
}

/// The row spline coefficients of a mesh block whose header and grid are in place (see `MESH_HEADER`): a, b, c, d per
/// row, `MAX_GRID_SIZE` values each, for the x coordinates and then for the y coordinates
fn append_row_coefficients(mesh: &mut Vec<f64>, divisions: (usize, usize), size_x: f64) {
    let mut a = [0.0; splines::MAX_GRID_SIZE];
    let mut b = [0.0; splines::MAX_GRID_SIZE];
    let mut c = [0.0; splines::MAX_GRID_SIZE];
    let mut d = [0.0; splines::MAX_GRID_SIZE];
    let mut alpha = [0.0; splines::MAX_GRID_SIZE - 1];
    let mut mu = [0.0; splines::MAX_GRID_SIZE];
    let mut z = [0.0; splines::MAX_GRID_SIZE];
    for mesh_offset in 0..=1 {
        for j in 0..divisions.1 {
            splines::BivariateSpline::cubic_spline_coefficients(&mesh[MESH_HEADER + mesh_offset..], 2, j * divisions.0, size_x, divisions.0, &mut a, &mut b, &mut c, &mut d, &mut alpha, &mut mu, &mut z);
            mesh.extend_from_slice(&a);
            mesh.extend_from_slice(&b);
            mesh.extend_from_slice(&c);
            mesh.extend_from_slice(&d);
        }
    }
}

/// Worst `|forward(inverse(q)) - q|` over `MESH_RESIDUAL_GRID`² positions on the sensor, in sensor pixels: how far
/// the kernels' inverse lookup lands from the source position the camera's mesh maps back to it
pub fn inverse_residual(forward: &[f64], inverse: &[f64], size: (f64, f64)) -> f64 {
    let n = MESH_RESIDUAL_GRID;
    (0..n * n).into_par_iter().map(|i| {
        let q = (size.0 * (i % n) as f64 / (n - 1) as f64, size.1 * (i / n) as f64 / (n - 1) as f64);
        let p = interpolate_mesh(q.0, q.1, size, inverse);
        let r = interpolate_mesh(p.x, p.y, size, forward);
        let residual = ((r.x - q.0).powi(2) + (r.y - q.1).powi(2)).sqrt();
        if residual.is_finite() { residual } else { f64::INFINITY }
    }).reduce(|| 0.0, f64::max)
}

/// The forward and inverse blocks of one mesh (`MESH_HEADER` describes the layout; the capture area slots stay zero,
/// the frame supplies them), and whether the kernels have to refine their inverse lookups against the forward block:
/// only when the inverse alone is off by more than `MESH_REFINE_THRESHOLD_PX` somewhere in the picture
fn build_mesh_table(mesh_data: &serde_json::Value, size: (f64, f64), divisions: (usize, usize), crop_size: (f32, f32), video_size: (usize, usize)) -> Option<MeshTable> {
    if divisions.0 < 2 || divisions.1 < 2 || divisions.0 > splines::MAX_GRID_SIZE || divisions.1 > splines::MAX_GRID_SIZE || !(size.0 > 0.0) || !(size.1 > 0.0) {
        log::error!("Invalid mesh correction: {divisions:?} divisions over {size:?}");
        return None;
    }
    let nodes = mesh_data.get("mesh")?.as_array()?;
    if nodes.len() != divisions.0 * divisions.1 {
        log::error!("Invalid mesh correction: {} nodes for {divisions:?} divisions", nodes.len());
        return None;
    }
    let capacity = MESH_HEADER + divisions.0 * divisions.1 * 2 + divisions.1 * splines::MAX_GRID_SIZE * 4 * 2;
    let header = |mesh: &mut Vec<f64>| {
        mesh.extend([0.0, divisions.0 as f64, divisions.1 as f64, size.0, size.1, 0.0, 0.0, 0.0, 0.0]);
    };

    let mut mesh = Vec::with_capacity(capacity);
    header(&mut mesh);
    for x in nodes {
        let coord = x.as_array()?;
        mesh.push(coord.get(0)?.as_f64()?);
        mesh.push(coord.get(1)?.as_f64()?);
    }
    append_row_coefficients(&mut mesh, divisions, size.0);
    mesh[0] = mesh.len() as f64;

    // The inverse on the same grid: where every node position comes from through the camera's mesh
    let step = (size.0 / (divisions.0 - 1) as f64, size.1 / (divisions.1 - 1) as f64);
    let grid: Vec<(f64, f64)> = (0..divisions.1).flat_map(|y| (0..divisions.0).map(move |x| (step.0 * x as f64, step.1 * y as f64))).collect();
    let inverted: Option<Vec<(f64, f64)>> = grid.into_par_iter().map(|(x, y)| inverse_interpolate_mesh(x, y, size, &mesh).ok()).collect();
    let Some(inverted) = inverted else {
        log::error!("Mesh correction: the mesh could not be inverted");
        return None;
    };
    let mut inv_mesh = Vec::with_capacity(capacity);
    header(&mut inv_mesh);
    for (x, y) in inverted {
        inv_mesh.push(x);
        inv_mesh.push(y);
    }
    append_row_coefficients(&mut inv_mesh, divisions, size.0);
    inv_mesh[0] = inv_mesh.len() as f64;

    let residual = inverse_residual(&mesh, &inv_mesh, size);
    let video_px_per_sensor_px = if video_size.0 > 0 && video_size.1 > 0 && crop_size.0 > 0.0 && crop_size.1 > 0.0 {
        (video_size.0 as f64 / crop_size.0 as f64).max(video_size.1 as f64 / crop_size.1 as f64)
    } else {
        1.0
    };
    let refine = residual * video_px_per_sensor_px > MESH_REFINE_THRESHOLD_PX;

    Some(MeshTable {
        refinement: if refine { mesh.iter().map(|x| *x as f32).collect() } else { Vec::new() },
        inverse: inv_mesh.iter().map(|x| *x as f32).collect(),
        forward: mesh,
    })
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
            Vector2::new(x_prime + 0.0003, y_prime),
            Vector2::new(x_prime, y_prime + 0.0003),
        ])
        .with_sd_tolerance(1e-10)?;

    let res = Executor::new(operator, solver)
        .configure(|state| state.max_iters(400))
        .run()?;

    if let Some(coeffs) = res.state.best_param {
        Ok((coeffs[0], coeffs[1]))
    } else {
        Err(argmin::core::Error::new(argmin::core::ArgminError::InvalidParameter { text: String::new() }))
    }
}
