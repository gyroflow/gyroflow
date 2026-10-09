// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

use telemetry_parser::tags_impl::{ GroupedTagMap, GetWithType, GroupId, TagId };
use telemetry_parser::util::SampleInfo;
use crate::gyro_source::FileMetadata;

/// Offset of the frame's capture time from its time in the video. The capture time (the middle of the exposure of
/// the frame's centre row, on the motion data's time base) follows the camera's real frame rate and its dropped
/// frames, while the video's frames are evenly spaced: at the video's frame rate, or the file's when the two differ
/// by more than 1 fps (`override_video_fps`)
pub fn get_time_offset(md: &FileMetadata, tag_map: &GroupedTagMap, info: &SampleInfo, fps: f64) -> Option<f64> {
    if !md.has_accurate_timestamps { return None; }
    let capture_time = *(tag_map.get(&GroupId::Imager)?.get_t(TagId::Custom("CaptureTime".into())) as Option<&f64>)?;
    let fps = md.frame_rate.filter(|x| (x - fps).abs() > 1.0).unwrap_or(fps);
    if fps <= 0.0 { return None; }
    Some(capture_time - crate::timestamp_at_frame(info.sample_index as i32, fps))
}

/// Without a lens profile, the camera's own calibration which MotionCam keeps with every frame: a pinhole camera
/// matrix and OpenCV's standard distortion (Android's model), in pixels of the recorded frame
pub fn init_lens_profile(md: &mut FileMetadata, input: &telemetry_parser::Input, tag_map: &GroupedTagMap, size: (usize, usize), info: &SampleInfo) {
    if md.lens_profile.is_some() { return; }
    let Some(calibration) = tag_map.get(&GroupId::Custom("LensCalibration".into())) else { return; };
    let (Some(f), Some(c), Some(&w), Some(&h)) = (
        calibration.get_t(TagId::PixelFocalLength) as Option<&(f32, f32)>,
        calibration.get_t(TagId::PrincipalPoint)   as Option<&(f32, f32)>,
        calibration.get_t(TagId::PixelWidth)       as Option<&u32>,
        calibration.get_t(TagId::PixelHeight)      as Option<&u32>,
    ) else { return; };
    if w == 0 || h == 0 || size.0 == 0 || size.1 == 0 { return; }
    // The loaded video can be a scaled rendering of the recorded frames, but not a cropped one
    if ((w as f64 / h as f64) / (size.0 as f64 / size.1 as f64) - 1.0).abs() > 0.02 { return; }
    let scale = size.0 as f64 / w as f64;
    let (fx, fy) = (f.0 as f64 * scale, f.1 as f64 * scale);
    let (cx, cy) = (c.0 as f64 * scale, c.1 as f64 * scale);
    let off_centre = (cx - size.0 as f64 / 2.0).abs() > 0.5 || (cy - size.1 as f64 / 2.0).abs() > 0.5;
    let distortion = (calibration.get_t(TagId::DistortionCoefficients) as Option<&Vec<f64>>).cloned().unwrap_or_default();

    let focal_length = tag_map.get(&GroupId::Lens).and_then(|x| x.get_t(TagId::FocalLength) as Option<&f32>).copied();
    let video_rotation = info.video_rotation.unwrap_or_default().abs();
    let is_vertical = video_rotation == 90 || video_rotation == 270;

    md.lens_profile = Some(serde_json::json!({
        "calibrated_by": "MotionCam",
        "camera_brand": "MotionCam",
        "camera_model": input.camera_model().map(|x| x.as_str()).unwrap_or(&""),
        "lens_model":   focal_length.map(|x| format!("{x:.2} mm")).unwrap_or_default(),
        "calib_dimension":  { "w": size.0, "h": size.1 },
        "orig_dimension":   { "w": size.0, "h": size.1 },
        "output_dimension": { "w": if is_vertical { size.1 } else { size.0 }, "h": if is_vertical { size.0 } else { size.1 } },
        "frame_readout_time": md.frame_readout_time,
        "focal_length": focal_length,
        "official": false,
        "asymmetrical": off_centre,
        "note": "The camera's own calibration, from the file's metadata",
        "fisheye_params": {
            "camera_matrix": [
                [ fx, 0.0, cx ],
                [ 0.0, fy, cy ],
                [ 0.0, 0.0, 1.0 ]
            ],
            "distortion_coeffs": distortion
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
