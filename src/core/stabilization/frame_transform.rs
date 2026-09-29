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
    pub matrices: Vec<[f32; 16]>,
    pub kernel_params: super::KernelParams,
    pub fov: f64,
    pub minimal_fov: f64,
    pub focal_length: Option<f64>,
    pub mesh_data: Vec<f32>,
}

impl FrameTransform {
    pub(crate) fn get_frame_readout_time(params: &ComputeParams, can_invert: bool, timestamp_ms: f64, file_metadata: &FileMetadata) -> f64 {
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

        let mut radial_distortion_limit = lens.radial_distortion_limit_rad().unwrap_or_default();

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

        // The largest ray angle this lens has an image for: where its calibration folds, and where the
        // model's own projection ends (`DistortionModel::field_limit`), whichever comes first
        let mut field_limit = radial_distortion_limit;
        if let Some(fl) = params.distortion_model.field_limit(&distortion_coeffs) {
            field_limit = if field_limit > 0.0 { field_limit.min(fl) } else { fl };
        }

        (camera_matrix, distortion_coeffs, field_limit, input_horizontal_stretch, input_vertical_stretch, focal_length, per_frame)
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
            field_limit,
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
        // No `get_new_k` here: the kernel builds the output plane itself, from `f`, `fov` and the output
        // size, because the lens-correction blend needs it before the rotation (`undistort_coord`)

        let gyro = params.gyro.read();
        let file_metadata = gyro.file_metadata.read();

        // Camera metadata mesh takes precedence. Otherwise use the transient optical residual mesh only when the
        // correction was actually applied in this lens/sync/timing context.
        let camera_mesh = file_metadata.mesh_correction.kernel_buffer(frame);
        let mesh_data = if !camera_mesh.is_empty() {
            camera_mesh
        } else if gyro.optical_correction_applied {
            gyro.optical_correction.as_ref()
                .and_then(|c| c.residual.kernel_buffer(frame, (params.width, params.height)))
                .unwrap_or_default()
        } else {
            Vec::new()
        };

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

            // The kernel carries rays as directions, not as points of the z=1 plane, so the output camera
            // matrix is no longer folded in here: this is the rotation alone, and `undistort_coord` turns the
            // output pixel into a ray before it. `r` is orthonormal (a rotation, at most conjugated by a
            // diagonal ±1 flip above), so its inverse is its transpose
            let i_r: Matrix3<f32> = nalgebra::convert(r.transpose());
            // Lens breathing: the row's magnification of the source image, applied where the lens images the
            // ray (`gyro_source::sony::breathing`). It used to be a zoom of the output plane folded into the
            // matrix, which is the same thing only paraxially - breathing is a change of the source lens's
            // focal length, so it belongs on the source side of the rotation
            let bz = breathing
                .map(|b| b.scale_at_row(sensor_row(y, b.crop_y as f64, b.crop_h as f64)))
                .filter(|k| k.is_finite() && *k > 0.0)
                .unwrap_or(1.0) as f32;
            [
                i_r[(0, 0)], i_r[(0, 1)], i_r[(0, 2)],
                i_r[(1, 0)], i_r[(1, 1)], i_r[(1, 2)],
                i_r[(2, 0)], i_r[(2, 1)], i_r[(2, 2)],
                sx, sy, ra,
                ox, oy,
                bz, 0.0
            ]
        }).collect::<Vec<[f32; 16]>>();
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
            field_limit:   field_limit as f32,
            output_projection: params.output_projection,
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

    pub fn at_timestamp_for_points(params: &ComputeParams, points: &[(f32, f32)], timestamp_ms: f64, frame: Option<usize>, use_fovs: bool) -> (Matrix3<f64>, [f64; 24], Matrix3<f64>, Vec<Matrix3<f64>>, Option<Vec<(f32, f32, f32, f32, f32)>>, Option<Vec<f64>>, f64, f64, Option<Vec<f32>>) { // camera_matrix, dist_coeffs, output camera matrix, rotations_per_point, shifts, mesh, fov, field_limit, breathing_per_point
        // ----------- Keyframes -----------
        let video_rotation = params.keyframes.value_at_video_timestamp(&KeyframeType::VideoRotation, timestamp_ms).unwrap_or(params.video_rotation);
        // ----------- Keyframes -----------

        let frame = frame.unwrap_or_else(|| crate::frame_at_timestamp(timestamp_ms, params.scaled_fps) as usize);

        let (mut camera_matrix, distortion_coeffs, field_limit, _, _, _, _) = Self::get_lens_data_at_timestamp(params, timestamp_ms, params.framebuffer_inverted);
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

        let mesh_correction = file_metadata.mesh_correction.forward_mesh(frame).or_else(|| {
            if gyro.optical_correction_applied {
                gyro.optical_correction.as_ref()
                    .and_then(|c| c.residual.forward_mesh(frame, (params.width, params.height)))
            } else {
                None
            }
        }); // distorting mesh, camera metadata first, then optical residual

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

        let rotations_breathing: Vec<(Matrix3<f64>, f32)> = points_iter.iter().map(|&(x, y)| {
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

            // The rotation alone: `undistort_points` projects the rotated ray itself and applies `new_k`
            // to the projected point, so the output camera matrix must not be folded in here. The breathing
            // magnification travels beside it and is undone on the source side, where `at_timestamp` applies it
            let p = r;
            let mut bz = 1.0f32;
            if let Some(b) = breathing {
                // Looked up by the same index `at_timestamp` looks its matrices up by: the point's readout position
                let readout_pos = if frame_readout_time.abs() > 0.0 {
                    (if params.frame_readout_direction.is_horizontal() { x } else { y }) as f64
                } else {
                    params.height as f64 / 2.0
                };
                let k = b.scale_at_row(Self::sensor_row(params, readout_pos, b.crop_y as f64, b.crop_h as f64));
                if k.is_finite() && k > 0.0 { bz = k as f32; }
            }
            (p, bz)
        }).collect();
        let rotations: Vec<Matrix3<f64>> = rotations_breathing.iter().map(|x| x.0).collect();
        // `None` when nothing breathes, so the common path carries no per-point vector at all
        let breathing_per_point: Option<Vec<f32>> = breathing.map(|_| rotations_breathing.iter().map(|x| x.1).collect());

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

        (scaled_k, distortion_coeffs, new_k, rotations, shifts, mesh_correction, fov, field_limit, breathing_per_point)
    }
}