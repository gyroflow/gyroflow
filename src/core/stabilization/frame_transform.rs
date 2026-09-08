// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use nalgebra::Matrix3;
use super::{ ComputeParams, KernelParams };
use rayon::iter::{ ParallelIterator, IntoParallelIterator };
use crate::gyro_source::FileMetadata;
use crate::keyframes::KeyframeType;
use crate::util::{ MapClosest, map_coord };

#[derive(Default, Clone)]
pub struct FrameTransform {
    pub matrices: Vec<[f32; 14]>,
    pub kernel_params: super::KernelParams,
    pub fov: f64,
    pub minimal_fov: f64,
    pub focal_length: Option<f64>,
    pub mesh_data: Vec<f32>,
}

impl FrameTransform {
    fn get_frame_readout_time(params: &ComputeParams, can_invert: bool, timestamp_ms: f64, file_metadata: &FileMetadata) -> f64 {
        let mut frame_readout_time = params.frame_readout_time.abs();
        let mut scale = 1.0;
        telemetry_parser::try_block!({
            let val = file_metadata.lens_params_closest((timestamp_ms * 1000.0).round() as i64, 100000, |v| v.has_readout_scale())?; // closest within 100ms
            scale = val.capture_area_size?.1 as f64 / val.sensor_size_px?.1 as f64;
        });
        if can_invert && params.framebuffer_inverted && !params.frame_readout_direction.is_horizontal() {
            frame_readout_time *= -1.0;
        }
        if params.frame_readout_direction.is_inverted() {
            frame_readout_time *= -1.0;
        }
        frame_readout_time * scale
    }
    fn get_new_k(params: &ComputeParams, camera_matrix: &Matrix3<f64>, fov: f64) -> Matrix3<f64> {
        let horizontal_ratio = if params.lens.input_horizontal_stretch > 0.01 { params.lens.input_horizontal_stretch } else { 1.0 };

        let img_dim_ratio = 1.0 / horizontal_ratio;

        let out_dim = (params.output_width as f64, params.output_height as f64);
        //let focal_center = (params.video_width as f64 / 2.0, params.video_height as f64 / 2.0);

        let mut new_k = *camera_matrix;
        new_k[(0, 0)] = new_k[(0, 0)] * img_dim_ratio / fov;
        new_k[(1, 1)] = new_k[(1, 1)] * img_dim_ratio / fov;
        new_k[(0, 2)] = /*(params.video_width  as f64 / 2.0 - new_k[(0, 2)]) * img_dim_ratio / fov + */out_dim.0 / 2.0;
        new_k[(1, 2)] = /*(params.video_height as f64 / 2.0 - new_k[(1, 2)]) * img_dim_ratio / fov + */out_dim.1 / 2.0;
        new_k
    }
    fn get_fov(params: &ComputeParams, frame: usize, use_fovs: bool, timestamp_ms: f64, for_ui: bool) -> f64 {
        let mut fov_scale = params.keyframes.value_at_video_timestamp(&KeyframeType::Fov, timestamp_ms).unwrap_or(params.fov_scale);
        fov_scale += if params.fov_overview && use_fovs && !for_ui { 1.0 } else { 0.0 };
        let mut fov = if use_fovs { params.fovs.get(frame).unwrap_or(if params.fovs.len() > 1 { params.fovs.last().unwrap() } else { &1.0 }) * fov_scale } else { 1.0 }.max(0.001);
        fov *= params.width as f64 / params.output_width.max(1) as f64;
        fov
    }

    /// The metadata focal length is often quantized (whole millimetres on many Sony lenses) while the optics
    /// zoom smoothly. Projecting with the stepped value makes the gyro correction jump at every step, because
    /// the correction shift scales with the focal length, so the camera matrix is rescaled to the dequantized
    /// per-frame focal length from `smoothing::focal_length` whenever the curve exists. Aspect and center are
    /// kept, and the distortion coefficients stay normalized as the camera delivered them. The curve stays within
    /// the dequantization band of the metadata (a fraction of a quantization step) everywhere except across a
    /// confirmed metadata glitch, where it bridges the levels around it (`focal_length::remove_outliers`), so this
    /// never moves the projection by more than a step on the strength of a heuristic; the clamp only guards
    /// against a curve that doesn't belong to this lens data at all. Returns the scale
    pub fn dequantize_camera_matrix(params: &ComputeParams, frame: usize, camera_matrix: &mut Matrix3<f64>) -> f64 {
        let Some(Some(dequantized)) = params.focal_lengths.get(frame).or(params.focal_lengths.last()).copied() else { return 1.0; };
        let raw = (camera_matrix[(0, 0)] * camera_matrix[(1, 1)]).sqrt();
        if !(raw > 0.0) || !(dequantized > 0.0) { return 1.0; }
        let scale = (dequantized / raw).clamp(0.1, 10.0);
        camera_matrix[(0, 0)] *= scale;
        camera_matrix[(1, 1)] *= scale;
        scale
    }

    /// Sensor row the picture row `y_source` sits at within the capture area `crop_y .. crop_y + crop_h`, for the
    /// per-row data (sensor and lens shift, lens breathing). `y_source` indexes the rows of the framebuffer the
    /// matrices are looked up by: inverted, row `y` holds picture row `height - y`, which sits at the mirrored
    /// position within the capture area, not within the sensor (the two differ as soon as the crop is off-centre,
    /// as it is with a moving EIS crop)
    fn sensor_row(params: &ComputeParams, y_source: f64, crop_y: f64, crop_h: f64) -> f64 {
        let y_sensor = map_coord(y_source, 0.0, params.height as f64, crop_y, crop_y + crop_h);
        if params.framebuffer_inverted { 2.0 * crop_y + crop_h - y_sensor } else { y_sensor }
    }

    /// The lens breathing compensation of one matrix row as a zoom of the output frame around its centre, by the
    /// row's magnification `k` (see `gyro_source::sony::breathing`). `None` when the row has no usable zoom: only a
    /// positive, finite one is a zoom at all, anything else makes the matrix singular and maps the whole output to
    /// the centre. `at_timestamp` post-multiplies its inverse transform by it and `at_timestamp_for_points`
    /// pre-multiplies its forward projection by the `inverse` of it, so the two directions stay exact inverses of
    /// each other - the STMap export writes one map from each and they only compose back to the identity if they do.
    /// The centre is the output frame's, in the coordinates of the caller's own output size: the two paths describe
    /// the same frame at different scales, and a zoom about a point survives that scaling unchanged
    fn breathing_matrix(params: &ComputeParams, k: f64, inverse: bool) -> Option<Matrix3<f64>> {
        if !(k.is_finite() && k > 0.0) { return None; }
        let k = if inverse { 1.0 / k } else { k };
        let (cx, cy) = (params.output_width as f64 / 2.0, params.output_height as f64 / 2.0);
        Some(Matrix3::new(k, 0.0, cx * (1.0 - k), 0.0, k, cy * (1.0 - k), 0.0, 0.0, 1.0))
    }

    /// Camera matrix, distortion coefficients, radial distortion limit, input stretches, focal length in millimetres,
    /// and whether the camera matrix's focal length came from per-frame lens metadata (see `get_lens_data_at_timestamp_with_metadata`)
    pub fn get_lens_data_at_timestamp(params: &ComputeParams, timestamp_ms: f64, invert_asym_lens: bool) -> (Matrix3<f64>, [f64; 24], f64, f64, f64, Option<f64>, bool) {
        let gyro = params.gyro.read();
        let file_metadata = gyro.file_metadata.read();
        Self::get_lens_data_at_timestamp_with_metadata(params, &file_metadata, timestamp_ms, invert_asym_lens)
    }

    /// `get_lens_data_at_timestamp` on file metadata the caller already holds. Anything that holds the `gyro` or
    /// the `file_metadata` read guard has to come through here: both are parking_lot locks, and a second `read()`
    /// of a lock this thread already reads is not a re-entry but a deadlock as soon as a writer has queued up
    /// behind the first guard (the lock is writer-fair: the new reader waits for the writer, the writer waits for
    /// the first guard, and the first guard waits for the new reader).
    ///
    /// The last element tells whether the focal length of the camera matrix came from the lens metadata of this frame
    /// (an interpolated lens profile, the camera's pixel focal length, or a millimetre focal length scaled into the
    /// profile) rather than from the static profile alone. The per-frame focal length curves (`smoothing::focal_length`)
    /// follow exactly this, so they can never disagree with the projection about which frames have a focal length of
    /// their own, whichever way the camera reports it
    pub fn get_lens_data_at_timestamp_with_metadata(params: &ComputeParams, file_metadata: &FileMetadata, timestamp_ms: f64, invert_asym_lens: bool) -> (Matrix3<f64>, [f64; 24], f64, f64, f64, Option<f64>, bool) {
        // The lens metadata may lag the picture by a few frames (per lens, see synchronization::lens_delay): every lookup uses the corrected time
        Self::get_lens_data_at_lens_timestamp(params, file_metadata, params.lens_timestamp_us(timestamp_ms), invert_asym_lens)
    }

    /// `get_lens_data_at_timestamp_with_metadata` at a lens metadata time already shifted by the delay
    /// (`ComputeParams::lens_timestamp_us`), for callers that apply a delay of their own choosing (the focal length
    /// curves are extracted without one and shifted by frames afterwards)
    pub fn get_lens_data_at_lens_timestamp(params: &ComputeParams, file_metadata: &FileMetadata, lens_timestamp_us: i64, invert_asym_lens: bool) -> (Matrix3<f64>, [f64; 24], f64, f64, f64, Option<f64>, bool) {
        let mut interpolated_lens = None;
        let mut per_frame = false;
        if !file_metadata.lens_positions.is_empty() && params.lens.has_interpolations() {
            if let Some(val) = file_metadata.lens_positions.get_closest(&lens_timestamp_us, 100000) { // closest within 100ms
                interpolated_lens = Some(params.lens.get_interpolated_lens_at(*val));
                per_frame = true;
            }
        }
        let lens = interpolated_lens.as_ref().unwrap_or(&params.lens);

        let mut focal_length = lens.focal_length;

        let mut camera_matrix = lens.get_camera_matrix((params.width, params.height), invert_asym_lens);
        let mut distortion_coeffs = lens.get_distortion_coeffs();

        let mut radial_distortion_limit = lens.fisheye_params.radial_distortion_limit.unwrap_or_default();

        let mut stretch_lens = true;
        let mut zoom_scale = 1.0;
        let digital_zoom = file_metadata.digital_zoom.unwrap_or_default();

        if lens.fisheye_params.distortion_coeffs.len() < 4 {
            if let Some(val) = file_metadata.lens_params_closest(lens_timestamp_us, 100000, |v| v.has_projection_data()) { // closest within 100ms
                let pixel_focal_length = val.pixel_focal_length.map(|f| (f.0 as f64, f.1 as f64)).or_else(|| {
                    let fl_mm = val.focal_length? as f64;
                    focal_length = Some(fl_mm);
                    let pp = val.pixel_pitch?;
                    let crop = val.capture_area_size?;
                    if pp.0 == 0 || pp.1 == 0 || crop.0 <= 0.0 || crop.1 <= 0.0 { return None; }
                    let fx = (fl_mm / ((pp.0 as f64 / 1_000_000.0) * crop.0 as f64)) * params.width  as f64;
                    let fy = (fl_mm / ((pp.1 as f64 / 1_000_000.0) * crop.1 as f64)) * params.height as f64;
                    Some((fx, fy))
                });
                if let Some((fx, fy)) = pixel_focal_length {
                    camera_matrix[(0, 0)] = fx;
                    camera_matrix[(1, 1)] = fy;
                    if let Some((cx, cy)) = val.principal_point {
                        camera_matrix[(0, 2)] = cx as f64;
                        camera_matrix[(1, 2)] = if invert_asym_lens { params.height as f64 - cy as f64 } else { cy as f64 };
                    }
                    stretch_lens = false;
                    per_frame = true;

                    if let Some(fl) = val.focal_length {
                        focal_length = Some(fl as f64);
                    }
                }
                if !val.distortion_coefficients.is_empty() && val.distortion_coefficients.len() <= 24 {
                    for (i, x) in val.distortion_coefficients.iter().enumerate() {
                        distortion_coeffs[i] = *x;
                    }

                    radial_distortion_limit = params.distortion_model.radial_distortion_limit(&distortion_coeffs).unwrap_or_default();
                }
            }
        } else if !params.lens.has_interpolations() && file_metadata.lens_focal_length_varies() {
            // A single calibration for a lens whose metadata records a changing focal length in millimetres (a zoom
            // lens on a Blackmagic, RED, Nikon or Z CAM body): the projection follows the zoom by scaling the
            // calibrated focal length with the metadata, relative to the focal length the profile declares or,
            // failing that, the one its camera matrix implies on this sensor. The distortion coefficients stay those
            // of the calibration. Cameras that also report the focal length in pixels (Canon) are left to that value.
            //
            // The profile asked here is `params.lens`, not the `lens` this frame projects with: a profile with
            // calibrations at several lens positions already follows the zoom through them, and scaling one of those
            // again would apply the zoom twice. `get_interpolated_lens_at` hands out the calibration of the position
            // itself - a profile of its own, with no interpolations left - whenever the lookup lands on a knot or
            // outside their range, and a blend that keeps them everywhere in between, so asking `lens` would turn
            // the branch on and off along the lens travel and jump the projection at every knot
            if let Some(val) = file_metadata.lens_params_closest(lens_timestamp_us, 100000, |v| v.focal_length.is_some() && v.pixel_focal_length.is_none()) {
                let mm = val.focal_length.unwrap_or_default() as f64;
                let calib_w = if lens.calib_dimension.w > 0 { lens.calib_dimension.w as f64 } else { params.width.max(1) as f64 };
                let reference = lens.focal_length.filter(|f| *f > 0.0).or_else(|| {
                    let (pp, crop) = (val.pixel_pitch?, val.capture_area_size?);
                    if pp.0 == 0 || crop.0 <= 0.0 { return None; }
                    Some(camera_matrix[(0, 0)] * (pp.0 as f64 / 1_000_000.0) * crop.0 as f64 / calib_w)
                });
                if let Some(reference) = reference {
                    if mm > 0.0 && reference > 0.0 {
                        zoom_scale = mm / reference;
                        focal_length = Some(mm);
                        per_frame = true;
                    }
                }
            }
        }

        let (calib_width, calib_height) = if lens.calib_dimension.w > 0 && lens.calib_dimension.h > 0 {
            (lens.calib_dimension.w as f64, lens.calib_dimension.h as f64)
        } else {
            (params.width.max(1) as f64, params.height.max(1) as f64)
        };

        let input_horizontal_stretch = if lens.input_horizontal_stretch > 0.01 { lens.input_horizontal_stretch } else { 1.0 };
        let input_vertical_stretch = if lens.input_vertical_stretch > 0.01 { lens.input_vertical_stretch } else { 1.0 };

        if stretch_lens {
            let lens_ratiox = (params.width as f64 / calib_width) * input_horizontal_stretch;
            let lens_ratioy = (params.height as f64 / calib_height) * input_vertical_stretch;
            camera_matrix[(0, 0)] *= lens_ratiox;
            camera_matrix[(1, 1)] *= lens_ratioy;
            camera_matrix[(0, 2)] *= lens_ratiox;
            camera_matrix[(1, 2)] *= lens_ratioy;
        }
        if digital_zoom > 0.0 {
            camera_matrix[(0, 0)] *= digital_zoom;
            camera_matrix[(1, 1)] *= digital_zoom;
        }
        if zoom_scale != 1.0 {
            camera_matrix[(0, 0)] *= zoom_scale;
            camera_matrix[(1, 1)] *= zoom_scale;
        }

        (camera_matrix, distortion_coeffs, radial_distortion_limit, input_horizontal_stretch, input_vertical_stretch, focal_length, per_frame)
    }

    pub fn at_timestamp(params: &ComputeParams, timestamp_ms: f64, frame: usize) -> Self {
        // ----------- Keyframes -----------
        let video_rotation = params.keyframes.value_at_video_timestamp(&KeyframeType::VideoRotation, timestamp_ms).unwrap_or(params.video_rotation);
        let background_margin = params.keyframes.value_at_video_timestamp(&KeyframeType::BackgroundMargin, timestamp_ms).unwrap_or(params.background_margin);
        let background_feather = params.keyframes.value_at_video_timestamp(&KeyframeType::BackgroundFeather, timestamp_ms).unwrap_or(params.background_margin_feather);
        let lens_correction_amount = params.keyframes.value_at_video_timestamp(&KeyframeType::LensCorrectionStrength, timestamp_ms).unwrap_or(params.lens_correction_amount);
        let adaptive_zoom_center_x = params.keyframes.value_at_video_timestamp(&KeyframeType::ZoomingCenterX, timestamp_ms).unwrap_or(params.adaptive_zoom_center_offset.0);
        let mut adaptive_zoom_center_y = params.keyframes.value_at_video_timestamp(&KeyframeType::ZoomingCenterY, timestamp_ms).unwrap_or(params.adaptive_zoom_center_offset.1);

        let light_refraction_coefficient = params.keyframes.value_at_video_timestamp(&KeyframeType::LightRefractionCoeff, timestamp_ms).unwrap_or(params.light_refraction_coefficient);

        // let additional_translation_x = params.keyframes.value_at_video_timestamp(&KeyframeType::AdditionalTranslationX, timestamp_ms).unwrap_or(params.additional_translation.0) as f32;
        // let additional_translation_y = params.keyframes.value_at_video_timestamp(&KeyframeType::AdditionalTranslationY, timestamp_ms).unwrap_or(params.additional_translation.1) as f32;
        // let additional_translation_z = params.keyframes.value_at_video_timestamp(&KeyframeType::AdditionalTranslationZ, timestamp_ms).unwrap_or(params.additional_translation.2) as f32;
        // ----------- Keyframes -----------

        // ----------- Lens -----------
        let (mut camera_matrix,
            distortion_coeffs,
            radial_distortion_limit,
            input_horizontal_stretch,
            input_vertical_stretch,
            focal_length, _) = Self::get_lens_data_at_timestamp(params, timestamp_ms, false);
        let focal_scale = Self::dequantize_camera_matrix(params, frame, &mut camera_matrix);
        let focal_length = focal_length.map(|f| f * focal_scale);
        // ----------- Lens -----------

        // Focal length stabilization: a uniform digital zoom (never above 1, so never past the frame) on
        // top of the adaptive zoom, see smoothing::focal_length. It's part of the applied zoom, so the UI
        // readout includes it too: the overlay then shows the true total zoom and the apparent focal length
        let fl_compensation = crate::smoothing::focal_length::compensation_at(params, frame);
        let mut fov = Self::get_fov(params, frame, true, timestamp_ms, false) * fl_compensation;
        let mut ui_fov = Self::get_fov(params, frame, true, timestamp_ms, true) * fl_compensation;
        if let Some(adj) = params.lens.optimal_fov {
            if params.fovs.is_empty() {
                fov *= adj;
            } else {
                ui_fov /= adj;
            }
        }

        let scaled_k = camera_matrix;
        let new_k = Self::get_new_k(&params, &camera_matrix, fov);

        let gyro = params.gyro.read();
        let file_metadata = gyro.file_metadata.read();

        // Undistorting mesh of the frame, empty when it has none (the kernel flags say so, the buffer is then not uploaded)
        let mesh_data = file_metadata.mesh_correction.kernel_buffer(frame);

        // ----------- Rolling shutter correction -----------
        let frame_readout_time = Self::get_frame_readout_time(&params, true, timestamp_ms, &file_metadata);

        let row_readout_time = frame_readout_time / if params.frame_readout_direction.is_horizontal() { params.width } else { params.height } as f64;
        let timestamp_ms = timestamp_ms + file_metadata.per_frame_time_offsets.get(frame).unwrap_or(&0.0);
        let start_ts = timestamp_ms - (frame_readout_time / 2.0);
        // ----------- Rolling shutter correction -----------

        // let frame_period = 1000.0 / params.scaled_fps as f64;
        // dbg!(frame_period);

        let is_scale = if let Some(is) = file_metadata.camera_stab_data.get(frame) {
            (
                params.width  as f64 / is.crop_area.2 as f64 / is.pixel_pitch.0 as f64,
                params.height as f64 / is.crop_area.3 as f64 / is.pixel_pitch.1 as f64 * (if params.framebuffer_inverted { -1.0 } else { 1.0 }),
            )
        } else {
            (1.0, 1.0)
        };
        // let height_scale = params.video_height as f64 / params.height.max(1) as f64;

        let image_rotation = Matrix3::new_rotation(video_rotation * (std::f64::consts::PI / 180.0));

        let quat1 = gyro.org_quat_at_timestamp(timestamp_ms).inverse();
        let smoothed_quat1 = gyro.smoothed_quat_at_timestamp(timestamp_ms);

        // Only compute 1 matrix if not using rolling shutter correction
        let rows = if frame_readout_time.abs() > 0.0 { if params.frame_readout_direction.is_horizontal() { params.width } else { params.height } } else { 1 };

        let breathing = if params.lens_breathing_enabled { file_metadata.lens_breathing.get(frame).filter(|b| !b.scale.is_empty()) } else { None };

        // Sensor row a matrix row is looked up at, for the per-row data (sensor and lens shift, lens breathing).
        // Without rolling shutter correction the single matrix stands for the whole frame and is evaluated at its
        // centre row
        let sensor_row = |y: usize, crop_y: f64, crop_h: f64| -> f64 {
            Self::sensor_row(params, if rows > 1 { y as f64 } else { params.height as f64 / 2.0 }, crop_y, crop_h)
        };

        let matrices = (0..rows).into_par_iter().map(|y| {
            let quat_time = if frame_readout_time.abs() > 0.0 {
                start_ts + row_readout_time * y as f64
            } else {
                start_ts
            };
            let quat = smoothed_quat1
                     * quat1
                     * gyro.org_quat_at_timestamp(quat_time);


            let mut r = image_rotation * *quat.to_rotation_matrix().matrix();
            if params.framebuffer_inverted {
                r[(0, 2)] *= -1.0; r[(1, 2)] *= -1.0;
                r[(2, 0)] *= -1.0; r[(2, 1)] *= -1.0;
            } else {
                r[(0, 1)] *= -1.0; r[(0, 2)] *= -1.0;
                r[(1, 0)] *= -1.0; r[(2, 0)] *= -1.0;
            }

            let (mut sx, mut sy, mut ra, mut ox, mut oy) = if let Some(is) = file_metadata.camera_stab_data.get(frame) {
                let y_sensor = sensor_row(y, is.crop_area.1 as f64, is.crop_area.3 as f64);

                let s = is.ibis_spline.interpolate(y_sensor + is.offset).unwrap_or_default();
                let sx = s.x * is_scale.0;
                let sy = s.y * is_scale.1;
                let ra = s.z / 1000.0 * (if params.framebuffer_inverted { -1.0 } else { 1.0 });

                let o = is.ois_spline.interpolate(y_sensor + is.ois_offset.unwrap_or(is.offset)).unwrap_or_default();
                let ox = o.x * is_scale.0;
                let oy = o.y * is_scale.1;

                // if y == 0 { log::debug!("IBIS data at frame: {frame}, ts: {ts}, sx: {sx:.3}, sy: {sy:.3}, ra: {ra:.3}, ox: {ox:.3}, oy: {oy:.3}"); }
                (sx as f32, sy as f32, ra.to_radians() as f32, ox as f32, oy as f32)
            } else {
                (0.0, 0.0, 0.0, 0.0, 0.0)
            };

            if params.suppress_rotation {
                r = Matrix3::identity();
                if params.frame_readout_time == 0.0 {
                    sx = 0.0; sy = 0.0; ra = 0.0; ox = 0.0; oy = 0.0;
                }
            }

            let i_r = (new_k * r).pseudo_inverse(0.000001);
            if let Err(err) = i_r {
                log::error!("Failed to multiply matrices: {:?} * {:?}: {}", new_k, r, err);
            }
            let mut i_r = i_r.unwrap_or_default();
            if let Some(b) = breathing {
                // Lens breathing: a zoom of the output around its centre, by the row's magnification
                if let Some(m) = Self::breathing_matrix(params, b.scale_at_row(sensor_row(y, b.crop_y as f64, b.crop_h as f64)), false) {
                    i_r *= m;
                }
            }
            let i_r: Matrix3<f32> = nalgebra::convert(i_r);
            [
                i_r[(0, 0)], i_r[(0, 1)], i_r[(0, 2)],
                i_r[(1, 0)], i_r[(1, 1)], i_r[(1, 2)],
                i_r[(2, 0)], i_r[(2, 1)], i_r[(2, 2)],
                sx, sy, ra,
                ox, oy
            ]
        }).collect::<Vec<[f32; 14]>>();
        drop(file_metadata);
        drop(gyro);

        let mut digital_lens_params = [0f32; 16];
        if let Some(p) = &params.digital_lens_params {
            for (i, v) in p.iter().take(16).enumerate() {
                digital_lens_params[i] = *v as f32;
            }
        }
        if params.framebuffer_inverted {
            adaptive_zoom_center_y *= -1.0;
        }

        let kernel_params = KernelParams {
            matrix_count:  matrices.len() as i32,
            f:             [scaled_k[(0, 0)] as f32, scaled_k[(1, 1)] as f32],
            c:             [scaled_k[(0, 2)] as f32, scaled_k[(1, 2)] as f32],
            k:             distortion_coeffs.iter().map(|x| *x as f32).collect::<Vec<f32>>().try_into().unwrap(),
            fov:           fov as f32,
            r_limit:       radial_distortion_limit as f32,
            lens_correction_amount:   lens_correction_amount as f32,
            input_vertical_stretch:   input_vertical_stretch as f32,
            input_horizontal_stretch: input_horizontal_stretch as f32,
            background_mode:          params.background_mode as i32,
            background_margin:        background_margin as f32,
            background_margin_feather:background_feather as f32,
            translation2d: [(adaptive_zoom_center_x * params.width as f64 / fov) as f32, (adaptive_zoom_center_y * params.height as f64 / fov) as f32],
            translation3d: [0.0, 0.0, 0.0, 0.0], // currently unused
            digital_lens_params,
            light_refraction_coefficient: light_refraction_coefficient as f32,
            ..Default::default()
        };

        Self {
            matrices,
            kernel_params,
            fov: ui_fov,
            minimal_fov: *params.minimal_fovs.get(frame).unwrap_or(&1.0),
            focal_length,
            mesh_data
        }
    }

    pub fn at_timestamp_for_points(params: &ComputeParams, points: &[(f32, f32)], timestamp_ms: f64, frame: Option<usize>, use_fovs: bool) -> (Matrix3<f64>, [f64; 24], Matrix3<f64>, Vec<Matrix3<f64>>, Option<Vec<(f32, f32, f32, f32, f32)>>, Option<Vec<f64>>, f64, f64) { // camera_matrix, dist_coeffs, p, rotations_per_point, shifts, mesh, fov, radial_distortion_limit
        // ----------- Keyframes -----------
        let video_rotation = params.keyframes.value_at_video_timestamp(&KeyframeType::VideoRotation, timestamp_ms).unwrap_or(params.video_rotation);
        // ----------- Keyframes -----------

        let frame = frame.unwrap_or_else(|| crate::frame_at_timestamp(timestamp_ms, params.scaled_fps) as usize);

        let (mut camera_matrix, distortion_coeffs, radial_distortion_limit, _, _, _, _) = Self::get_lens_data_at_timestamp(params, timestamp_ms, params.framebuffer_inverted);
        Self::dequantize_camera_matrix(params, frame, &mut camera_matrix);

        // The focal length compensation is part of the applied zoom, not of the base projection:
        // measurements at fov = 1 (zoom polygon, sync, features) must not include it, or the zoom would
        // fit the frame around the crop and undo it, see zooming::calculate_fovs
        let fl_compensation = if use_fovs { crate::smoothing::focal_length::compensation_at(params, frame) } else { 1.0 };
        let fov = Self::get_fov(params, frame, use_fovs, timestamp_ms, false) * fl_compensation;

        let scaled_k = camera_matrix;
        let new_k = Self::get_new_k(params, &camera_matrix, fov);

        let gyro = params.gyro.read();
        let file_metadata = gyro.file_metadata.read();

        let mesh_correction = file_metadata.mesh_correction.forward_mesh(frame); // distorting mesh, none when the frame has none

        // ----------- Rolling shutter correction -----------
        let frame_readout_time = Self::get_frame_readout_time(params, false, timestamp_ms, &file_metadata);

        let row_readout_time = frame_readout_time / if params.frame_readout_direction.is_horizontal() { params.width } else { params.height } as f64;
        let timestamp_ms = timestamp_ms + file_metadata.per_frame_time_offsets.get(frame).unwrap_or(&0.0);
        let start_ts = timestamp_ms - (frame_readout_time / 2.0);
        // ----------- Rolling shutter correction -----------

        let image_rotation = Matrix3::new_rotation(video_rotation * (std::f64::consts::PI / 180.0));

        let quat1 = gyro.org_quat_at_timestamp(timestamp_ms).inverse();
        let smoothed_quat1 = gyro.smoothed_quat_at_timestamp(timestamp_ms);

        // Only compute 1 matrix if not using rolling shutter correction; it stands for the whole frame, so the
        // per-row data (sensor and lens shift, lens breathing) is looked up at the centre row, like `at_timestamp` does
        let centre = [(params.width as f32 / 2.0, params.height as f32 / 2.0)];
        let points_iter: &[(f32, f32)] = if frame_readout_time.abs() > 0.0 { points } else { &centre };

        // Lens breathing, the zoom `at_timestamp` folds into its matrices, so this direction can undo it and the two
        // stay invertible (the STMap export writes a map from each). Like the focal length compensation above it's
        // part of the applied zoom and not of the base projection, so it follows `use_fovs` too: the measurements at
        // fov = 1 (zoom polygon, sync, features) describe the picture the zoom is fitted around, and a zoom folded
        // into them would only let the fit relax and undo it
        let breathing = if use_fovs && params.lens_breathing_enabled { file_metadata.lens_breathing.get(frame).filter(|b| !b.scale.is_empty()) } else { None };

        let rotations: Vec<Matrix3<f64>> = points_iter.iter().map(|&(x, y)| {
            let quat_time = if frame_readout_time.abs() > 0.0 {
                start_ts + row_readout_time * if params.frame_readout_direction.is_horizontal() { x } else { y } as f64
            } else {
                start_ts
            };
            let quat = smoothed_quat1
                     * quat1
                     * gyro.org_quat_at_timestamp(quat_time);

            let mut r = image_rotation * *quat.to_rotation_matrix().matrix();
            r[(0, 1)] *= -1.0; r[(0, 2)] *= -1.0;
            r[(1, 0)] *= -1.0; r[(2, 0)] *= -1.0;

            if params.suppress_rotation {
                r = Matrix3::identity();
            }

            let mut p = new_k * r;
            if let Some(b) = breathing {
                // Looked up by the same index `at_timestamp` looks its matrices up by: the point's readout position
                let readout_pos = if frame_readout_time.abs() > 0.0 {
                    (if params.frame_readout_direction.is_horizontal() { x } else { y }) as f64
                } else {
                    params.height as f64 / 2.0
                };
                if let Some(m) = Self::breathing_matrix(params, b.scale_at_row(Self::sensor_row(params, readout_pos, b.crop_y as f64, b.crop_h as f64)), true) {
                    p = m * p;
                }
            }
            p
        }).collect();

        let mut shifts: Option<Vec<(f32, f32, f32, f32, f32)>> = if let Some(is) = file_metadata.camera_stab_data.get(frame) {
            let is_scale = (
                params.width  as f64 / is.crop_area.2 as f64 / is.pixel_pitch.0 as f64,
                params.height as f64 / is.crop_area.3 as f64 / is.pixel_pitch.1 as f64,
            );
            Some(points_iter.iter().map(|&(_x, y)| {
                let y = map_coord(y as f64, 0.0, params.height as f64, is.crop_area.1 as f64, is.crop_area.1 as f64 + is.crop_area.3 as f64);
                let s = is.ibis_spline.interpolate(y + is.offset).unwrap_or_default();
                let sx = s.x * is_scale.0;
                let sy = s.y * is_scale.1;
                let ra = s.z / 1000.0;

                let o = is.ois_spline.interpolate(y + is.ois_offset.unwrap_or(is.offset)).unwrap_or_default();
                let ox = o.x * is_scale.0;
                let oy = o.y * is_scale.1;

                (sx as f32, sy as f32, ra.to_radians() as f32, ox as f32, oy as f32)
            }).collect())
        } else {
            None
        };
        if params.suppress_rotation && params.frame_readout_time == 0.0 {
            shifts = None;
        }

        (scaled_k, distortion_coeffs, new_k, rotations, shifts, mesh_correction, fov, radial_distortion_limit)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::gyro_source::{ BreathingFrame, FileMetadata, LensParams };
    use crate::lens_profile::{ Dimensions, LensProfile };
    use crate::stabilization::{ Stabilization, undistort_points };

    const W: usize = 1920;
    const H: usize = 1080;
    const POINTS: [(f32, f32); 5] = [(0.0, 0.0), (1919.0, 0.0), (960.0, 540.0), (300.0, 900.0), (1600.0, 1079.0)];

    /// A plain fisheye calibration on a still camera with one lens breathing table: everything the two transform
    /// paths need to describe the same frame, and nothing that could move between them
    fn params(scale: Vec<f32>, readout_time: f64) -> ComputeParams {
        let mut p = ComputeParams::default();
        p.width = W; p.height = H; p.output_width = W; p.output_height = H;
        p.frame_count = 1;
        p.scaled_fps = 30.0;
        p.fov_scale = 1.0;
        p.frame_readout_time = readout_time;
        p.suppress_rotation = true;
        p.lens_breathing_enabled = true;

        p.lens = LensProfile::default();
        p.lens.calib_dimension = Dimensions { w: W, h: H };
        p.lens.fisheye_params.camera_matrix = vec![[1400.0, 0.0, W as f64 / 2.0], [0.0, 1400.0, H as f64 / 2.0], [0.0, 0.0, 1.0]];
        p.lens.fisheye_params.distortion_coeffs = vec![0.05, -0.012, 0.003, -0.0004];

        let mut md = FileMetadata::default();
        md.lens_breathing = vec![BreathingFrame { scale, crop_y: 0.0, crop_h: H as f32 }];
        p.gyro.write().file_metadata = md.into();
        p
    }

    /// Output position of a source pixel: the direction the zoom, the sync and the STMap redistort map go
    fn to_output(p: &ComputeParams, pt: (f32, f32)) -> (f32, f32) {
        let (k, coeffs, _p, rotations, is, mesh, fov, r_limit) = FrameTransform::at_timestamp_for_points(p, &[pt], 0.0, Some(0), true);
        undistort_points(&[pt], k, &coeffs, rotations[0], None, Some(rotations), p, 1.0, fov, 0.0, is, mesh, r_limit)[0]
    }

    /// Source pixel an output position samples: the direction the render and the STMap undistort map go. `row` is
    /// the matrix the render resolves for the pixel, the source row it lands on
    fn to_source(p: &ComputeParams, pt: (f32, f32), row: usize) -> Option<(f32, f32)> {
        let t = FrameTransform::at_timestamp(p, 0.0, 0);
        let mut kp = t.kernel_params;
        kp.width = W as i32; kp.height = H as i32;
        kp.output_width = W as i32; kp.output_height = H as i32;
        Stabilization::rotate_and_distort(pt, row.min(t.matrices.len() - 1), &kp, &t.matrices, &p.distortion_model, None, kp.r_limit * kp.r_limit, &[])
    }

    fn assert_round_trip(p: &ComputeParams) {
        for &pt in &POINTS {
            let out = to_output(p, pt);
            let back = to_source(p, out, pt.1 as usize).unwrap_or_else(|| panic!("{pt:?} -> {out:?} has no source pixel"));
            assert!((back.0 - pt.0).abs() < 0.05 && (back.1 - pt.1).abs() < 0.05, "{pt:?} -> {out:?} -> {back:?}");
        }
    }

    #[test]
    fn breathing_zoom_inverts_itself() {
        // One magnification for the whole frame. The STMap export writes one map from each direction, and they
        // only compose back to the identity if both carry the zoom
        assert_round_trip(&params(vec![0.82], 0.0));
    }

    #[test]
    fn per_row_breathing_zoom_inverts_itself() {
        // The focus moving during the readout: every matrix row has a magnification of its own, and the forward
        // direction has to look its own up at the same row
        assert_round_trip(&params((0..9).map(|i| 0.75 + i as f32 * 0.02).collect(), 12.0));
    }

    /// A profile with calibrations at several lens positions, on a body that also records the focal length in
    /// millimetres: the projection follows the zoom through the calibrations themselves, and the metadata must
    /// not scale them on top of that. `get_interpolated_lens_at` hands out a profile with no interpolations left
    /// wherever the lookup lands on a knot, so a check on the profile of the frame instead of the one in
    /// `ComputeParams` would turn the scaling on at the knots only and jump the projection there
    #[test]
    fn interpolated_calibrations_are_not_scaled_by_the_metadata_focal_length() {
        let mut p = params(vec![1.0], 0.0);
        p.lens.focal_length = Some(24.0); // the calibration is a wide one; the metadata reaches 70mm
        p.lens.interpolations = Some(serde_json::json!({
            "0.0": { "camera_matrix": [[1000.0, 0.0, 960.0], [0.0, 1000.0, 540.0], [0.0, 0.0, 1.0]] },
            "1.0": { "camera_matrix": [[2000.0, 0.0, 960.0], [0.0, 2000.0, 540.0], [0.0, 0.0, 1.0]] },
        }));
        p.lens.resolve_interpolations(&crate::lens_profile_database::LensProfileDatabase::default());
        assert!(p.lens.has_interpolations());

        let mut md = FileMetadata::default();
        for (i, (position, mm)) in [(0.0, 24.0f32), (0.5, 47.0), (1.0, 70.0)].into_iter().enumerate() {
            let ts = i as i64 * 33333;
            md.lens_positions.insert(ts, position);
            md.lens_params.insert(ts, LensParams { focal_length: Some(mm), ..Default::default() });
        }
        assert!(md.lens_focal_length_varies());
        p.gyro.write().file_metadata = md.into();

        let fx = |ts: i64| {
            let gyro = p.gyro.read();
            let md = gyro.file_metadata.read();
            FrameTransform::get_lens_data_at_lens_timestamp(&p, &md, ts, false).0[(0, 0)]
        };
        // 1000 to 2000 across the lens travel: the calibrations at the ends and the blend in between, nothing else
        for (ts, expected) in [(0, 1000.0), (33333, 1500.0), (66666, 2000.0)] {
            assert!((fx(ts) - expected).abs() < 1e-6, "at {ts}: {} instead of {expected}", fx(ts));
        }
    }

    #[test]
    fn breathing_stays_out_of_the_fov_measurement() {
        // The measurements at fov = 1 (zoom polygon, sync, features) describe the picture the zoom is fitted
        // around; a zoom folded into them would only let the fit relax and undo it, like the focal length
        // compensation next to it
        let on = params(vec![0.82], 0.0);
        let mut off = params(vec![0.82], 0.0);
        off.lens_breathing_enabled = false;
        let at = |p: &ComputeParams, use_fovs: bool| FrameTransform::at_timestamp_for_points(p, &POINTS, 0.0, Some(0), use_fovs).3;
        assert_eq!(at(&on, false), at(&off, false));
        assert_ne!(at(&on, true),  at(&off, true));
    }
}
