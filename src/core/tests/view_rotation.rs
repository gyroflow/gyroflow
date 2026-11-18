// SPDX-License-Identifier: GPL-3.0-or-later
//
// `ComputeParams::view_rotation`: the output camera turned after the smoothing, as a camera on a tripod (yaw, then
// pitch, then roll about where it looks). Checked on the CPU kernel with a still camera and an equidistant fisheye,
// where a ray `θ` from the lens's axis lands `F·θ` from the middle, so the expectations need no lens model.

use std::collections::BTreeMap;
use std::sync::Arc;
use nalgebra::{ UnitQuaternion, Vector3 };
use gyroflow_core::StabilizationManager;
use gyroflow_core::gpu::{ Buffers, BufferDescription, BufferSource };
use gyroflow_core::stabilization::{ ComputeParams, Interpolation, Luma16, Stabilization };

const SIZE: (usize, usize) = (1920, 1080);
const F: f64 = 1100.0;
const C: (f64, f64) = (960.0, 540.0);
/// Source samples are their own coordinate, in sixteenths of a pixel, from this offset: 0 is the background
const BASE: f64 = 8.0;

fn manager() -> StabilizationManager {
    let stab = StabilizationManager::default();
    stab.init_from_video_data(10_000.0, 30.0, 300, SIZE);
    stab.set_output_size(SIZE.0, SIZE.1);
    let lens = serde_json::json!({
        "name": "test", "calibrated_by": "test", "camera_brand": "test", "camera_model": "test",
        "calib_dimension": { "w": SIZE.0, "h": SIZE.1 }, "orig_dimension": { "w": SIZE.0, "h": SIZE.1 },
        "fps": 30.0, "calibrator_version": "---",
        "fisheye_params": {
            "camera_matrix": [[F, 0.0, C.0], [0.0, F, C.1], [0.0, 0.0, 1.0]],
            "distortion_coeffs": [1e-6, 0.0, 0.0, 0.0]
        }
    }).to_string();
    stab.load_lens_profile(&lens).unwrap();
    {
        // A still camera: the raw and the smoothed orientation the same
        let mut gyro = stab.gyro.write();
        gyro.duration_ms = 10_000.0;
        let still: BTreeMap<i64, UnitQuaternion<f64>> = (0..=10_000).map(|ms| (ms * 1000, UnitQuaternion::identity())).collect();
        gyro.quaternions = Arc::new(still.clone());
        gyro.smoothed_quaternions = Arc::new(still);
    }
    stab
}

fn params(stab: &StabilizationManager, view: (f64, f64, f64)) -> ComputeParams {
    let mut p = ComputeParams::from_manager(stab);
    p.background = nalgebra::Vector4::new(0.0, 0.0, 0.0, 0.0);
    p.adaptive_zoom_window = 0.0;
    p.view_rotation = view;
    p
}

/// The CPU kernel on a source whose samples are their own x (or y): which source pixel each output pixel read, None
/// for the background
fn render(p: &ComputeParams) -> Vec<Option<(f64, f64)>> {
    let ts = 500_000;
    let mut stab = Stabilization::default();
    stab.interpolation = Interpolation::Bilinear;
    stab.share_wgpu_instances = false;
    stab.set_device(-1);
    stab.init_size(SIZE, (p.output_width, p.output_height));
    stab.set_compute_params(p.clone());
    let mut read = |coord: usize| -> Vec<u16> {
        let mut src: Vec<u16> = (0..SIZE.0 * SIZE.1).map(|k| ((if coord == 0 { k % SIZE.0 } else { k / SIZE.0 }) as f64 + BASE) as u16 * 16).collect();
        let mut dst = vec![0u16; p.output_width * p.output_height];
        let mut buffers = Buffers {
            input:  BufferDescription { size: (SIZE.0, SIZE.1, SIZE.0 * 2), data: BufferSource::Cpu { buffer: bytemuck::cast_slice_mut(&mut src) }, ..Default::default() },
            output: BufferDescription { size: (p.output_width, p.output_height, p.output_width * 2), data: BufferSource::Cpu { buffer: bytemuck::cast_slice_mut(&mut dst) }, ..Default::default() },
        };
        stab.ensure_ready_for_processing::<Luma16>(ts, None, &mut buffers);
        let mut transform = stab.get_frame_transform_at::<Luma16>(ts, None, &buffers);
        transform.kernel_params.pixel_value_limit = 65535.0;
        transform.kernel_params.max_pixel_value = 65535.0;
        stab.process_pixels::<Luma16>(ts, None, &mut buffers, Some(&transform)).unwrap();
        dst
    };
    let (xs, ys) = (read(0), read(1));
    xs.iter().zip(&ys).map(|(&x, &y)| if x == 0 || y == 0 { None } else { Some((x as f64 / 16.0 - BASE, y as f64 / 16.0 - BASE)) }).collect()
}

fn at(rendered: &[Option<(f64, f64)>], x: usize, y: usize) -> (f64, f64) {
    rendered[y * SIZE.0 + x].unwrap_or_else(|| panic!("({x}, {y}) is background"))
}

/// Where the equidistant lens images a direction of the camera (x right, y down, z forward)
fn image(d: Vector3<f64>) -> (f64, f64) {
    let rxy = d.x.hypot(d.y);
    let theta = rxy.atan2(d.z);
    (C.0 + F * theta * d.x / rxy, C.1 + F * theta * d.y / rxy)
}

fn close(a: (f64, f64), b: (f64, f64), label: &str) {
    assert!((a.0 - b.0).abs() < 0.15 && (a.1 - b.1).abs() < 0.15, "{label}: read {a:?}, expected {b:?}");
}

#[test]
fn no_view_rotation_changes_nothing() {
    let stab = manager();
    let mut before = ComputeParams::from_manager(&stab);
    before.background = nalgebra::Vector4::new(0.0, 0.0, 0.0, 0.0);
    before.adaptive_zoom_window = 0.0;
    assert_eq!(render(&before), render(&params(&stab, (0.0, 0.0, 0.0))));
}

/// + yaw looks right and + pitch looks down: the middle of the output reads the source that far from its middle
#[test]
fn yaw_looks_right_and_pitch_looks_down() {
    let stab = manager();
    let mid = (SIZE.0 / 2, SIZE.1 / 2);
    close(at(&render(&params(&stab, (0.0, 0.0, 0.0))), mid.0, mid.1), C, "no turn");
    close(at(&render(&params(&stab, (10.0, 0.0, 0.0))), mid.0, mid.1), (C.0 + F * 10f64.to_radians(), C.1), "yaw 10°");
    close(at(&render(&params(&stab, (-10.0, 0.0, 0.0))), mid.0, mid.1), (C.0 - F * 10f64.to_radians(), C.1), "yaw -10°");
    close(at(&render(&params(&stab, (0.0, 10.0, 0.0))), mid.0, mid.1), (C.0, C.1 + F * 10f64.to_radians()), "pitch 10°");
}

/// A tripod: the pitch tilts the camera about its own horizontal axis after the yaw has turned it, so the middle
/// looks along (sin yaw · cos pitch, sin pitch, cos yaw · cos pitch)
#[test]
fn pitch_is_about_the_turned_camera() {
    let stab = manager();
    let (yaw, pitch) = (30f64.to_radians(), 20f64.to_radians());
    let want = image(Vector3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos()));
    close(at(&render(&params(&stab, (30.0, 20.0, 0.0))), SIZE.0 / 2, SIZE.1 / 2), want, "yaw 30°, pitch 20°");
}

/// The roll turns the picture about its middle, whatever the yaw and pitch: the middle reads what it read without
/// it, and a point to its right reads the source turned by the roll about that, counterclockwise on the picture
/// (+ roll turns the camera clockwise)
#[test]
fn roll_turns_about_the_middle() {
    let stab = manager();
    let mid = (SIZE.0 / 2, SIZE.1 / 2);
    for (yaw, pitch) in [(0.0, 0.0), (15.0, -10.0)] {
        let flat = render(&params(&stab, (yaw, pitch, 0.0)));
        let rolled = render(&params(&stab, (yaw, pitch, 10.0)));
        close(at(&rolled, mid.0, mid.1), at(&flat, mid.0, mid.1), &format!("the middle, yaw {yaw}, pitch {pitch}"));
    }
    let rolled = render(&params(&stab, (0.0, 0.0, 10.0)));
    let p = at(&rolled, mid.0 + 200, mid.1);
    let angle = (p.1 - C.1).atan2(p.0 - C.0).to_degrees();
    assert!((angle - 10.0).abs() < 0.05, "a point right of the middle reads the source {angle:.3}° from the horizontal");
}
