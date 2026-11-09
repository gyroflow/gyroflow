// SPDX-License-Identifier: GPL-3.0-or-later
//
// The equirectangular output projection and a rig lens's pose (`RigLens`): a flat clip rendered into a sphere,
// against the geometry worked out by hand here and against each other's backends - the CPU kernel, wgpu (the one a
// player renders with) and the point path the zoom and the sync use.
//
// The lens is an equidistant fisheye, OpenCV's model with next to no distortion terms, so where it images a ray is
// `F·θ` from the middle and the expectations below need no lens model of their own.

use std::collections::BTreeMap;
use nalgebra::{ Matrix3, UnitQuaternion, Vector3 };
use gyroflow_core::StabilizationManager;
use gyroflow_core::gpu::{ Buffers, BufferDescription, BufferSource };
use gyroflow_core::stabilization::{ ComputeParams, Interpolation, Luma16, RigLens, Stabilization, undistort_points_with_rolling_shutter };
use gyroflow_core::stabilization::projection::{ self, OutputProjection };

const SIZE: (usize, usize) = (1920, 1080);
const OUT: (usize, usize) = (720, 360);
const F: f64 = 1100.0;
const C: (f64, f64) = (960.0, 540.0);
/// Source samples are their own coordinate, in sixteenths of a pixel, from this offset: 0 is the background
const BASE: f64 = 8.0;

fn manager(moving: bool) -> StabilizationManager {
    let stab = StabilizationManager::default();
    stab.init_from_video_data(10_000.0, 30.0, 300, SIZE);
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

    // The camera turns about all three axes, the smoothed one follows it slower (as in vs-gyroflow's sphere.rs)
    let k = if moving { 1.0 } else { 0.0 };
    let camera = |t: f64| UnitQuaternion::from_euler_angles(k * 0.2 * (t * 1.3).sin(), k * (0.3 * (t * 0.7).sin() + 0.1), k * 0.15 * (t * 2.1).cos());
    let smoothed = |t: f64| UnitQuaternion::from_euler_angles(k * 0.05 * (t * 0.3).sin(), k * (0.3 * (t * 0.7 - 0.4).sin() + 0.1), k * 0.02);
    {
        let mut gyro = stab.gyro.write();
        gyro.duration_ms = 10_000.0;
        let ts: Vec<i64> = (0..=10_000).map(|ms| ms * 1000).collect();
        gyro.quaternions = std::sync::Arc::new(ts.iter().map(|&us| (us, camera(us as f64 / 1e6))).collect::<BTreeMap<_, _>>());
        gyro.smoothed_quaternions = std::sync::Arc::new(ts.iter().map(|&us| { let t = us as f64 / 1e6; (us, smoothed(t).inverse() * camera(t)) }).collect());
    }
    stab
}

fn equirect_params(stab: &StabilizationManager, readout_ms: f64, rig_lens: RigLens) -> ComputeParams {
    let mut p = ComputeParams::from_manager(stab);
    p.output_projection = OutputProjection::Equirect as i32;
    p.output_width = OUT.0;
    p.output_height = OUT.1;
    p.frame_readout_time = readout_ms;
    p.background = nalgebra::Vector4::new(0.0, 0.0, 0.0, 0.0);
    p.rig_lens = rig_lens;
    p
}

/// The rig lens turned to look along `yaw` (radians, + to the right), offset by `t` (metres, rig axes)
fn rig_lens(yaw: f64, t: [f64; 3], distance: f64, validity: f64) -> RigLens {
    RigLens { rotation: UnitQuaternion::from_axis_angle(&Vector3::y_axis(), yaw), translation: t, stitching_distance: distance, validity }
}

/// The direction an output pixel of the equirect stands for, in the output camera's axes: the kernel renders pixel
/// `i` at position `i` (not its middle)
fn out_dir(i: f64, j: f64) -> Vector3<f64> {
    let lon = (i - OUT.0 as f64 / 2.0) * std::f64::consts::TAU / OUT.0 as f64;
    let lat = (j - OUT.1 as f64 / 2.0) * std::f64::consts::PI / OUT.1 as f64;
    Vector3::new(lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos())
}

/// The equidistant lens: where it images a direction of its own axes, None behind it
fn image(d: &Vector3<f64>) -> Option<(f64, f64)> {
    let rxy = d.x.hypot(d.y);
    let theta = rxy.atan2(d.z);
    if rxy < 1e-12 { return if d.z > 0.0 { Some(C) } else { None }; }
    Some((C.0 + F * theta * d.x / rxy, C.1 + F * theta * d.y / rxy))
}

/// The source pixel an output direction is read from, worked out without the kernel: the output camera into the
/// world (the smoothed camera, `D` the axis flip between camera and gyro), back into the camera at the time its row
/// was read, then the rig lens's pose - the point `R·d` of the stitching sphere seen from `t` - and the lens
fn source_pixel(p: &ComputeParams, d_out: &Vector3<f64>, t_ms: f64) -> Option<(f64, f64)> {
    let dflip = Matrix3::from_diagonal(&Vector3::new(1.0, -1.0, -1.0));
    let (world, row_quat) = {
        let gyro = p.gyro.read();
        let sm = gyro.org_quat_at_timestamp(t_ms) * gyro.smoothed_quat_at_timestamp(t_ms).inverse();
        (sm.to_rotation_matrix().matrix() * dflip * d_out, |t: f64| p.gyro.read().org_quat_at_timestamp(t))
    };
    let lens = &p.rig_lens;
    let r = lens.stitching_distance;
    let mut y = SIZE.1 as f64 / 2.0;
    let mut px = None;
    for _ in 0..20 {
        let t = t_ms + p.frame_readout_time * (y / SIZE.1 as f64 - 0.5);
        let body = row_quat(t).inverse() * world;
        let rig = dflip * body;
        let seen = if r > 0.0 { rig * r - Vector3::from(lens.translation) } else { rig };
        let in_lens = lens.rotation.inverse() * seen;
        let theta = in_lens.x.hypot(in_lens.y).atan2(in_lens.z);
        if lens.validity > 0.0 && theta > lens.validity { return None; }
        px = image(&in_lens);
        match px { Some(q) => y = q.1, None => return None }
    }
    px
}

fn gpu_device() -> Option<isize> {
    gyroflow_core::gpu::initialize_contexts();
    let list = Stabilization::default().list_devices();
    *gyroflow_core::stabilization::GPU_LIST.write() = list.clone();
    list.iter().position(|n| n.starts_with("[wgpu]")).map(|i| i as isize)
}

/// The kernel on a source whose samples are their own x (or y): which source pixel each output pixel read, None
/// for the background. And the backend that drew it
fn render(p: &ComputeParams, t_ms: f64, device: isize) -> (Vec<Option<(f64, f64)>>, &'static str) {
    let ts = (t_ms * 1000.0) as i64;
    let mut stab = Stabilization::default();
    stab.interpolation = Interpolation::Bilinear;
    stab.share_wgpu_instances = false;
    stab.set_device(device);
    stab.init_size(SIZE, (p.output_width, p.output_height));
    stab.set_compute_params(p.clone());
    let mut backend = "";
    let mut read = |coord: usize| -> Vec<u16> {
        let mut src: Vec<u16> = (0..SIZE.0 * SIZE.1).map(|k| ((if coord == 0 { k % SIZE.0 } else { k / SIZE.0 }) as f64 + BASE) as u16 * 16).collect();
        let mut dst = vec![0u16; p.output_width * p.output_height];
        let mut buffers = Buffers {
            input:  BufferDescription { size: (SIZE.0, SIZE.1, SIZE.0 * 2), data: BufferSource::Cpu { buffer: bytemuck::cast_slice_mut(&mut src) }, ..Default::default() },
            output: BufferDescription { size: (p.output_width, p.output_height, p.output_width * 2), data: BufferSource::Cpu { buffer: bytemuck::cast_slice_mut(&mut dst) }, ..Default::default() },
        };
        stab.ensure_ready_for_processing::<Luma16>(ts, None, &mut buffers);
        let mut transform = stab.get_frame_transform_at::<Luma16>(ts, None, &buffers);
        let unorm = stab.initialized_backend.is_wgpu() && <Luma16 as gyroflow_core::stabilization::PixelType>::wgpu_format().map(|x| x.2).unwrap_or_default();
        transform.kernel_params.pixel_value_limit = if unorm { 1.0 } else { 65535.0 };
        transform.kernel_params.max_pixel_value = if unorm { 1.0 } else { 65535.0 };
        backend = stab.process_pixels::<Luma16>(ts, None, &mut buffers, Some(&transform)).unwrap().backend;
        dst
    };
    let (xs, ys) = (read(0), read(1));
    let px = xs.iter().zip(&ys).map(|(&x, &y)| if x == 0 || y == 0 { None } else { Some((x as f64 / 16.0 - BASE, y as f64 / 16.0 - BASE)) }).collect();
    (px, backend)
}

/// Inside the source by more than the interpolation's reach, where background can't creep in. wgpu's kernel reads
/// wider than the CPU's bilinear
const REACH: f64 = 8.0;
fn well_inside(q: (f64, f64)) -> bool {
    (REACH..SIZE.0 as f64 - REACH - 1.0).contains(&q.0) && (REACH..SIZE.1 as f64 - REACH - 1.0).contains(&q.1)
}
fn well_outside(q: Option<(f64, f64)>) -> bool {
    q.is_none_or(|q| !(-REACH..SIZE.0 as f64 + REACH).contains(&q.0) || !(-REACH..SIZE.1 as f64 + REACH).contains(&q.1))
}

/// Every output pixel against the hand geometry: where the picture is, the right source pixel; where it isn't,
/// the background. Returns how many pixels had picture
fn check_against_hand(p: &ComputeParams, t_ms: f64, rendered: &[Option<(f64, f64)>], label: &str) -> usize {
    check_against_hand_at(p, t_ms, rendered, 0.0, label)
}
fn check_against_hand_at(p: &ComputeParams, t_ms: f64, rendered: &[Option<(f64, f64)>], at: f64, label: &str) -> usize {
    let (mut worst, mut seen) = (0.0f64, 0);
    for j in 0..OUT.1 {
        for i in 0..OUT.0 {
            let want = source_pixel(p, &out_dir(i as f64 + at, j as f64 + at), t_ms);
            let got = rendered[j * OUT.0 + i];
            match want {
                Some(w) if well_inside(w) => {
                    let g = got.unwrap_or_else(|| panic!("{label}: ({i}, {j}) should read the source at {w:?}, it's background"));
                    worst = worst.max((g.0 - w.0).hypot(g.1 - w.1));
                    seen += 1;
                },
                w if well_outside(w) => assert!(got.is_none(), "{label}: ({i}, {j}) is past the source ({w:?}) and reads {got:?}"),
                _ => {}
            }
        }
    }
    // Sixteenths of a pixel, f32 arithmetic, the orientations interpolated per row
    assert!(worst < 0.25, "{label}: up to {worst:.3} source pixels off the hand geometry");
    seen
}

#[test]
fn the_plane_is_longitude_and_latitude() {
    use std::f32::consts::{ PI, FRAC_PI_2 };
    let p = projection::EQUIRECT;
    let dir = |n: (f32, f32)| projection::ray_to_dir(projection::unproject(n, p));
    let close = |a: (f32, f32, f32), b: (f32, f32, f32)| (a.0 - b.0).abs() < 2e-4 && (a.1 - b.1).abs() < 2e-4 && (a.2 - b.2).abs() < 2e-4;
    assert!(close(dir((0.0, 0.0)), (0.0, 0.0, 1.0)), "the middle looks forward");
    assert!(close(dir((FRAC_PI_2, 0.0)), (1.0, 0.0, 0.0)), "+90 degrees of longitude is to the right");
    assert!(close(dir((-FRAC_PI_2, 0.0)), (-1.0, 0.0, 0.0)));
    assert!(close(dir((0.0, -FRAC_PI_2 + 1e-4)), (0.0, -1.0, 0.0)), "the top row is up, -y");
    assert!(close(dir((0.0, FRAC_PI_2 - 1e-4)), (0.0, 1.0, 0.0)));
    assert!(close(dir((PI - 1e-4, 0.0)), (0.0, 0.0, -1.0)), "the edges are straight behind");
    // Behind is an angle vector of length π, carried as one
    assert!((projection::unproject((PI - 1e-3, 0.3), p).0.hypot(projection::unproject((PI - 1e-3, 0.3), p).1) - 2.84).abs() < 0.01);
    // And back, all over the plane
    for &(x, y) in &[(0.0, 0.0), (0.3, -0.2), (-2.9, 1.2), (3.1, -1.5), (1.7, 0.0), (-0.01, 1.55)] {
        let (x2, y2) = projection::project(projection::unproject((x, y), p), p);
        assert!((x - x2).abs() < 1e-4 && (y - y2).abs() < 1e-4, "({x}, {y}) -> ({x2}, {y2})");
    }
    assert_eq!(OutputProjection::from_i32(5), OutputProjection::Equirect);
    assert_eq!(projection::setting_from_name("equirect"), Some(5));
}

/// A still camera: the picture is in the middle of the sphere, the lens's field around it and nothing behind
#[test]
fn a_flat_clip_lands_where_its_directions_are() {
    let stab = manager(false);
    let p = equirect_params(&stab, 0.0, RigLens::default());
    let (rendered, _) = render(&p, 500.0, -1);
    let seen = check_against_hand(&p, 500.0, &rendered, "still");
    assert!(seen > 10_000, "only {seen} pixels of picture");
    // The middle of the sphere is the middle of the source
    let mid = rendered[OUT.1 / 2 * OUT.0 + OUT.0 / 2].unwrap();
    assert!((mid.0 - C.0).abs() < 0.1 && (mid.1 - C.1).abs() < 0.1, "{mid:?}");
    // 90 degrees to the right is F·π/2 to the right of the middle: past the frame, background
    assert!(rendered[OUT.1 / 2 * OUT.0 + OUT.0 * 3 / 4].is_none());
    assert!(rendered[OUT.1 / 2 * OUT.0].is_none(), "straight behind");
}

/// The camera turning and the rows read at different times: the same geometry, through the smoothing and the
/// rolling shutter; and wgpu, which a player renders with, the same as the CPU.
///
/// The two don't put an output pixel at the same place: the CPU kernel renders pixel `i` at position `i`, wgpu's
/// render pass at its middle, `i + 0.5` (that's gyroflow-core's, for every projection). For the equirect it's the
/// usual convention: pixel `i` of `W` is longitude `((i + 0.5) / W - 0.5) · 360°`. Each is held against the hand
/// geometry at its own positions
#[test]
fn a_moving_clip_and_the_gpu() {
    let stab = manager(true);
    let device = gpu_device();
    for (readout, t_ms) in [(0.0, 500.0), (25.0, 2345.0), (25.0, 7010.0)] {
        let p = equirect_params(&stab, readout, RigLens::default());
        let (cpu, _) = render(&p, t_ms, -1);
        check_against_hand(&p, t_ms, &cpu, &format!("CPU, readout {readout} ms, {t_ms} ms"));
        let Some(device) = device else { eprintln!("No wgpu device: the GPU half skipped"); continue };
        let (gpu, backend) = render(&p, t_ms, device);
        assert_eq!(backend, "wgpu");
        let seen = check_against_hand_at(&p, t_ms, &gpu, 0.5, &format!("wgpu, readout {readout} ms, {t_ms} ms"));
        assert!(seen > 10_000, "wgpu: only {seen} pixels of picture");
    }
}

/// A part of the sphere: `fov_scale` 0.5 is the middle 180°×90°, so the source's middle stays put and a pixel a
/// quarter of the way across is 45 degrees to the left
#[test]
fn the_zoom_is_a_part_of_the_sphere() {
    let stab = manager(false);
    let mut p = equirect_params(&stab, 0.0, RigLens::default());
    p.fov_scale = 0.5;
    let (rendered, _) = render(&p, 500.0, -1);
    let mid = rendered[OUT.1 / 2 * OUT.0 + OUT.0 / 2].unwrap();
    assert!((mid.0 - C.0).abs() < 0.1 && (mid.1 - C.1).abs() < 0.1);
    let quarter = rendered[OUT.1 / 2 * OUT.0 + OUT.0 / 4].unwrap();
    let want = C.0 - F * std::f64::consts::FRAC_PI_4;
    assert!((quarter.0 - want).abs() < 0.1, "{quarter:?} vs {want}");
}

/// The lens's offset against parallax worked out by hand. A lens 3 cm right of the rig's center, looking forward:
/// the point of the stitching sphere straight ahead of the center is `atan(0.03 / R)` to the lens's left
#[test]
fn the_offset_is_the_parallax_of_the_stitching_distance() {
    let stab = manager(false);
    let mid = OUT.1 / 2 * OUT.0 + OUT.0 / 2;
    for (distance, want_x) in [(1.0, 960.0 - 1100.0 * 0.03f64.atan()), (0.5, 960.0 - 1100.0 * 0.06f64.atan()), (0.0, 960.0)] {
        let p = equirect_params(&stab, 0.0, rig_lens(0.0, [0.03, 0.0, 0.0], distance, 0.0));
        let (rendered, _) = render(&p, 500.0, -1);
        let got = rendered[mid].unwrap();
        assert!((got.0 - want_x).abs() < 0.1 && (got.1 - C.1).abs() < 0.1, "R = {distance}: {got:?}, by hand {want_x}");
        check_against_hand(&p, 500.0, &rendered, &format!("offset, R = {distance}"));
    }
    // 927.0 and 894.1: the numbers themselves, so the formula above isn't the only thing checked
    assert!((960.0 - 1100.0 * 0.03f64.atan() - 927.01).abs() < 0.01);
    assert!((960.0 - 1100.0 * 0.06f64.atan() - 894.08).abs() < 0.01);
}

/// A rig lens looking right (an X5's back lens is the same with 180 degrees): its picture is around +90 degrees of
/// longitude. With its offset along its own axis and the sphere at 1 m, a point 30 degrees past its axis is seen at
/// `atan(0.5 / (cos 30° - 0.03))` = 30.88 degrees, not 30
#[test]
fn a_turned_lens_with_its_offset() {
    let stab = manager(true);
    let p = equirect_params(&stab, 25.0, rig_lens(std::f64::consts::FRAC_PI_2, [0.03, 0.0, 0.0], 1.0, 0.0));
    let (rendered, _) = render(&p, 2345.0, -1);
    check_against_hand(&p, 2345.0, &rendered, "turned lens, moving");
    // The offset and the turn on wgpu too: `translation3d` and the matrices are the kernel's own inputs there
    if let Some(device) = gpu_device() {
        let (gpu, backend) = render(&p, 2345.0, device);
        assert_eq!(backend, "wgpu");
        assert!(check_against_hand_at(&p, 2345.0, &gpu, 0.5, "turned lens, moving, wgpu") > 5_000);
    }

    let still = manager(false);
    let p = equirect_params(&still, 0.0, rig_lens(std::f64::consts::FRAC_PI_2, [0.03, 0.0, 0.0], 1.0, 0.0));
    let (rendered, _) = render(&p, 500.0, -1);
    // Longitude 90° + 30° is pixel 720 · (120/360) past the middle
    let at = |lon_deg: f64| rendered[OUT.1 / 2 * OUT.0 + (OUT.0 as f64 * (0.5 + lon_deg / 360.0)).round() as usize];
    let axis = at(90.0).unwrap();
    assert!((axis.0 - C.0).abs() < 0.1, "the lens's axis: {axis:?}");
    let past = at(120.0).unwrap();
    let theta = 0.5f64.atan2(0.75f64.sqrt() - 0.03);
    assert!((theta.to_degrees() - 30.88).abs() < 0.01, "{}", theta.to_degrees());
    assert!((past.0 - (C.0 + F * theta)).abs() < 0.1, "{past:?} vs {}", C.0 + F * theta);
    // In front of the rig is 90 degrees off this lens: past its frame, background
    assert!(at(0.0).is_none());
}

/// Past the rig's validity limit a ray is background, even where the lens still has picture: never its edge
#[test]
fn past_the_validity_limit_is_background() {
    let stab = manager(false);
    let p = equirect_params(&stab, 0.0, rig_lens(0.0, [0.0; 3], 0.0, 0.4));
    let (rendered, _) = render(&p, 500.0, -1);
    let row = OUT.1 / 2 * OUT.0;
    let at = |rad: f64| rendered[row + (OUT.0 as f64 * (0.5 + rad / std::f64::consts::TAU)).round() as usize];
    assert!(at(0.3).is_some());
    assert!(at(0.45).is_none() && at(-0.45).is_none() && at(0.8).is_none(), "{:?}", at(0.45));
    check_against_hand(&p, 500.0, &rendered, "validity");
}

/// No pose at all, and a stitching distance with nothing to move: a flat clip's render, bit for bit
#[test]
fn a_zero_pose_changes_nothing() {
    let stab = manager(true);
    let mut p = ComputeParams::from_manager(&stab);
    p.output_width = 480;
    p.output_height = 270;
    p.frame_readout_time = 25.0;
    p.background = nalgebra::Vector4::new(0.0, 0.0, 0.0, 0.0);
    let (a, _) = render(&p, 2345.0, -1);
    p.rig_lens = rig_lens(0.0, [0.0; 3], 2.0, 0.0);
    let (b, _) = render(&p, 2345.0, -1);
    assert!(a == b);
}

/// The point path (the zoom's, the sync's, the STMap's): a source pixel to the output, the inverse of the render,
/// offset and turn included
#[test]
fn points_land_where_the_render_reads_them() {
    let stab = manager(true);
    for lens in [RigLens::default(), rig_lens(0.4, [0.03, -0.01, 0.02], 0.7, 0.0)] {
        let p = equirect_params(&stab, 25.0, lens);
        for t_ms in [500.0, 2345.0] {
            for &(i, j) in &[(360.0, 180.0), (300.0, 150.0), (420.0, 220.0), (380.0, 170.0)] {
                let Some(src) = source_pixel(&p, &out_dir(i, j), t_ms) else { continue };
                if !well_inside(src) { continue; }
                let out = undistort_points_with_rolling_shutter(&[(src.0 as f32, src.1 as f32)], t_ms, None, &p, 1.0, true, false)[0];
                assert!((out.0 as f64 - i).hypot(out.1 as f64 - j) < 0.05, "{lens:?}, {t_ms} ms: ({i}, {j}) reads {src:?}, which the points put at {out:?}");
            }
        }
    }
}

/// A real clip's frame in the sphere, for looking at: `cargo test --test equirect -- --ignored a_clip_in_the_sphere`
/// with `EQUIRECT_CLIP` (the video, for its motion data and lens), `EQUIRECT_FRAME` (that frame as raw RGBA, its
/// size the video's: `ffmpeg -ss <t> -i clip -frames:v 1 -pix_fmt rgba -f rawvideo frame.rgba`), `EQUIRECT_TIME_MS`
/// (`<t>`) and `EQUIRECT_OUT` (a folder). Writes 2048×1024 raw RGBA, with a grid every 30°: `camera.rgba`, the
/// clip as it is, in its camera's axes at the middle of the frame (no smoothing, the rolling shutter taken out); and
/// `turned.rgba`, as if it were a rig's lens looking 90° right and 20° up, 5 cm from the rig's center, stitched at 1 m.
/// Rendered on wgpu, as a player would. `EQUIRECT_WATER=1` for a clip under water: the flat port's refraction
#[test]
#[ignore]
fn a_clip_in_the_sphere() {
    use gyroflow_core::stabilization::RGBA8;
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} not set"));
    let (clip, frame, out) = (env("EQUIRECT_CLIP"), env("EQUIRECT_FRAME"), std::path::PathBuf::from(env("EQUIRECT_OUT")));
    let t_ms: f64 = env("EQUIRECT_TIME_MS").parse().unwrap();

    let stab = StabilizationManager::default();
    let size = std::fs::metadata(&clip).unwrap().len();
    let url = gyroflow_core::filesystem::path_to_url(&clip);
    stab.load_video_file(&mut std::fs::File::open(&clip).unwrap(), size, &url, None, true).unwrap();
    assert!(stab.gyro.read().has_motion(), "{clip} has no motion data");
    stab.set_smoothing_method(0);
    stab.recompute_blocking();
    let (w, h) = stab.params.read().size;
    let src = std::fs::read(&frame).unwrap();
    assert_eq!(src.len(), w * h * 4, "{frame} isn't {w}x{h} RGBA");

    let ow = (2048, 1024);
    let device = gpu_device().expect("no wgpu device");
    let draw = |lens: RigLens, name: &str| {
        let mut p = ComputeParams::from_manager(&stab);
        p.output_projection = OutputProjection::Equirect as i32;
        p.output_width = ow.0;
        p.output_height = ow.1;
        p.background = nalgebra::Vector4::new(0.0, 0.0, 0.0, 255.0);
        p.rig_lens = lens;
        if std::env::var("EQUIRECT_WATER").is_ok_and(|v| v == "1") { p.light_refraction_coefficient = 1.33; }
        let ts = (t_ms * 1000.0) as i64;
        let mut s = Stabilization::default();
        s.interpolation = Interpolation::Lanczos4;
        s.share_wgpu_instances = false;
        s.set_device(device);
        s.init_size((w, h), ow);
        s.set_compute_params(p);
        let mut dst = vec![0u8; ow.0 * ow.1 * 4];
        let mut buffers = Buffers {
            input:  BufferDescription { size: (w, h, w * 4), data: BufferSource::Cpu { buffer: &mut src.clone() }, ..Default::default() },
            output: BufferDescription { size: (ow.0, ow.1, ow.0 * 4), data: BufferSource::Cpu { buffer: &mut dst }, ..Default::default() },
        };
        s.ensure_ready_for_processing::<RGBA8>(ts, None, &mut buffers);
        let mut transform = s.get_frame_transform_at::<RGBA8>(ts, None, &buffers);
        let unorm = s.initialized_backend.is_wgpu() && <RGBA8 as gyroflow_core::stabilization::PixelType>::wgpu_format().map(|x| x.2).unwrap_or_default();
        transform.kernel_params.pixel_value_limit = if unorm { 1.0 } else { 255.0 };
        transform.kernel_params.max_pixel_value = if unorm { 1.0 } else { 255.0 };
        let info = s.process_pixels::<RGBA8>(ts, None, &mut buffers, Some(&transform)).unwrap();
        assert_eq!(info.backend, "wgpu");
        // The grid: every 30° of longitude and latitude, the middle ones brighter
        for y in 0..ow.1 {
            for x in 0..ow.0 {
                let on = |v: usize, n: usize| { let step = n / if n == ow.0 { 12 } else { 6 }; (v % step == 0, v == n / 2) };
                let ((gx, mx), (gy, my)) = (on(x, ow.0), on(y, ow.1));
                if gx || gy {
                    let k = (y * ow.0 + x) * 4;
                    let v = if mx || my { 255 } else { 140 };
                    for c in 0..3 { dst[k + c] = ((dst[k + c] as u16 + v) / 2) as u8; }
                }
            }
        }
        std::fs::write(out.join(name), &dst).unwrap();
    };
    draw(RigLens::default(), "camera.rgba");
    draw(RigLens {
        rotation: UnitQuaternion::from_axis_angle(&Vector3::y_axis(), std::f64::consts::FRAC_PI_2) * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), 20f64.to_radians()),
        translation: [0.05, 0.0, 0.0], stitching_distance: 1.0, validity: 0.0,
    }, "turned.rgba");
}
