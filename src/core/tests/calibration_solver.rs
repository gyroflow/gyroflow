// SPDX-License-Identifier: GPL-3.0-or-later
//
// `calibration::solver` against lenses whose truth is known: chessboard views rendered through a model, with noise,
// and the lens it fits compared with the one they came from over the whole field.
// Run with --release: the solver is a bundle adjustment with finite differences, slow in a debug build.

use gyroflow_core::calibration::solver::{ self, Model };
use gyroflow_core::stabilization::KernelParams;
use gyroflow_core::stabilization::distortion_models::{ DistortionModel, OpenCVFisheye, OpenCVStandard, GenericPolynomial };
use nalgebra::{ Rotation3, Vector3 };
use std::f64::consts::PI;

const COLS: usize = 14;
const ROWS: usize = 8;

fn board() -> Vec<(f64, f64)> {
    (0..ROWS).flat_map(|y| (0..COLS).map(move |x| (x as f64, y as f64))).collect()
}

/// xorshift64*, and normals by Box-Muller: the views have to be the same on every run
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12; self.0 ^= self.0 << 25; self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545F4914F6CDD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn uniform(&mut self, a: f64, b: f64) -> f64 { a + (b - a) * self.next() }
    fn normal(&mut self) -> f64 { (-2.0 * self.next().max(1e-300).ln()).sqrt() * (2.0 * PI * self.next()).cos() }
}

/// A model with its focal length, principal point and coefficients (zero-padded to the 12 slots a model may read)
struct Lens<'a> { model: &'a dyn Model, f: (f64, f64), c: (f64, f64), k: Vec<f64> }
impl<'a> Lens<'a> {
    fn new(model: &'a dyn Model, f: (f64, f64), c: (f64, f64), k: &[f64]) -> Self {
        let mut k = k.to_vec();
        k.resize(12, 0.0);
        Self { model, f, c, k }
    }
    fn fitted(model: &'a dyn Model, fit: &solver::Calibration) -> Self { Self::new(model, fit.f, fit.c, &fit.k) }
    fn pixel(&self, ray: &Vector3<f64>) -> (f64, f64) {
        let (u, v) = self.model.project(&self.k, &ray.normalize());
        (self.f.0 * u + self.c.0, self.f.1 * v + self.c.1)
    }
}

struct Shots { n: usize, center_max: f64, corner_max: f64, distance: (f64, f64), noise: f64 }

/// Boards facing the camera from up to `center_max` degrees off the axis, every corner within `corner_max`.
/// Returns the views and the widest corner angle among them
fn views(lens: &Lens, s: &Shots, rng: &mut Rng) -> (Vec<Vec<(f64, f64)>>, f64) {
    let b = board();
    let ctr = Vector3::new((COLS - 1) as f64 / 2.0, (ROWS - 1) as f64 / 2.0, 0.0);
    let mut out = Vec::new();
    let mut widest = 0.0f64;
    while out.len() < s.n {
        let (tc, ac) = (rng.uniform(0.0, s.center_max).to_radians(), rng.uniform(0.0, 2.0 * PI));
        let dir = Vector3::new(tc.sin() * ac.cos(), tc.sin() * ac.sin(), tc.cos());
        let dist = rng.uniform(s.distance.0, s.distance.1);
        // The board's normal points back at the camera, tilted at random
        let tilt = Rotation3::new(Vector3::new(rng.normal(), rng.normal(), rng.normal()) * 0.35);
        let zb = tilt * -dir;
        let xb = Vector3::new(rng.normal(), rng.normal(), rng.normal()).cross(&zb).normalize();
        let yb = zb.cross(&xb);
        if zb.dot(&-dir) < 0.3 { continue; }
        let corners: Vec<Vector3<f64>> = b.iter().map(|p| {
            let q = Vector3::new(p.0, p.1, 0.0) - ctr;
            xb * q.x + yb * q.y + zb * q.z + dir * dist
        }).collect();
        let angle = corners.iter().map(|x| x.x.hypot(x.y).atan2(x.z).to_degrees()).fold(0.0, f64::max);
        if angle > s.corner_max { continue; }
        widest = widest.max(angle);
        out.push(corners.iter().map(|x| { let (u, v) = lens.pixel(x); (u + rng.normal() * s.noise, v + rng.normal() * s.noise) }).collect());
    }
    (out, widest)
}

/// The largest pixel distance between two lenses, over rays from the axis out to `field` degrees
fn difference(a: &Lens, b: &Lens, field: f64) -> f64 {
    let mut worst = 0.0f64;
    for i in 0..=110 {
        let t = (i as f64 / 110.0 * field).to_radians();
        for j in 0..16 {
            let az = j as f64 / 16.0 * 2.0 * PI;
            let ray = Vector3::new(t.sin() * az.cos(), t.sin() * az.sin(), t.cos());
            let (p, q) = (a.pixel(&ray), b.pixel(&ray));
            worst = worst.max((p.0 - q.0).hypot(p.1 - q.1));
        }
    }
    worst
}

fn report(fit: &solver::Calibration, n: usize, err: f64, field: f64) {
    println!("rms {:.3} px, kept {}/{n}, f ({:.2}, {:.2}), c ({:.2}, {:.2}), k {:?}, off by {err:.3} px at most over 0..{field}°",
        fit.rms, fit.kept.len(), fit.f.0, fit.f.1, fit.c.0, fit.c.1, &fit.k[..fit.fitted]);
}

#[test]
fn a_220_degree_lens_with_boards_past_90_degrees() {
    let model = OpenCVFisheye::default();
    let truth = Lens::new(&model, (1100.0, 1100.0), (2010.0, 1985.0), &[-0.02, 0.001, 0.0, 0.0]);
    let (views, widest) = views(&truth, &Shots { n: 60, center_max: 100.0, corner_max: 112.0, distance: (9.0, 14.0), noise: 0.5 }, &mut Rng(7));
    assert!(widest > 105.0, "the boards should reach well past 90°, they reach {widest:.1}°");

    let fit = solver::calibrate(&model, &board(), &views, (4000, 4000), &[]).expect("a calibration");
    let err = difference(&Lens::fitted(&model, &fit), &truth, 110.0);
    report(&fit, views.len(), err, 110.0);
    // 0.5 px per axis is 0.71 px per point
    assert!(fit.rms < 0.8, "rms {}", fit.rms);
    assert!(err < 1.5, "the fitted lens is off by {err} px");
    assert!(fit.max_corner_angle.to_degrees() > 105.0);
}

#[test]
fn an_ordinary_action_camera_lens() {
    let model = OpenCVFisheye::default();
    let truth = Lens::new(&model, (1850.0, 1850.0), (2003.0, 1497.0), &[0.04, -0.01, 0.003, 0.0]);
    let (views, _) = views(&truth, &Shots { n: 40, center_max: 55.0, corner_max: 75.0, distance: (9.0, 14.0), noise: 0.3 }, &mut Rng(11));
    let fit = solver::calibrate(&model, &board(), &views, (4000, 3000), &[]).expect("a calibration");
    let err = difference(&Lens::fitted(&model, &fit), &truth, 75.0);
    report(&fit, views.len(), err, 75.0);
    assert!(fit.rms < 0.5, "rms {}", fit.rms);
    assert!(err < 1.0, "the fitted lens is off by {err} px");
}

#[test]
fn a_rectilinear_lens_as_opencv_standard() {
    let model = OpenCVStandard::default();
    // k1, k2, p1, p2, k3: barrel distortion and a slightly decentred element
    let truth = Lens::new(&model, (2800.0, 2805.0), (1990.0, 1510.0), &[-0.12, 0.08, 0.0008, -0.0005, -0.02]);
    let (views, _) = views(&truth, &Shots { n: 40, center_max: 25.0, corner_max: 38.0, distance: (16.0, 26.0), noise: 0.3 }, &mut Rng(5));
    let fit = solver::calibrate(&model, &board(), &views, (4000, 3000), &[]).expect("a calibration");
    let err = difference(&Lens::fitted(&model, &fit), &truth, 38.0);
    report(&fit, views.len(), err, 38.0);
    assert_eq!(fit.fitted, 5, "only k1, k2, p1, p2, k3 are fitted");
    assert_eq!(fit.k.len(), 12, "and `k` is all the model reads");
    assert!(fit.rms < 0.5, "rms {}", fit.rms);
    // What's left is at the edge of the field, where only the corners of the outermost boards pin k3·r⁶ down
    assert!(err < 1.5, "the fitted lens is off by {err} px");
}

#[test]
fn a_lens_with_even_powers_as_generic_polynomial() {
    let model = GenericPolynomial::default();
    let truth = Lens::new(&model, (1100.0, 1100.0), (2010.0, 1985.0), &[1.0, 0.03, -0.06, 0.02, 0.004, -0.002]);
    let (views, _) = views(&truth, &Shots { n: 60, center_max: 90.0, corner_max: 100.0, distance: (9.0, 14.0), noise: 0.5 }, &mut Rng(31));
    let fit = solver::calibrate(&model, &board(), &views, (4000, 4000), &[]).expect("a calibration");
    let err = difference(&Lens::fitted(&model, &fit), &truth, 100.0);
    report(&fit, views.len(), err, 100.0);
    assert_eq!(fit.k[0], 1.0, "the linear term is the focal length's");
    assert_eq!(fit.fitted, 6);
    assert!(fit.rms < 0.8, "rms {}", fit.rms);
    assert!(err < 1.5, "the fitted lens is off by {err} px");

    // The same views through `opencv_fisheye`, which has no even powers: it can only come close
    let fisheye = OpenCVFisheye::default();
    let other = solver::calibrate(&fisheye, &board(), &views, (4000, 4000), &[]).expect("a calibration");
    println!("as opencv_fisheye: rms {:.3} px", other.rms);
    assert!(other.rms > fit.rms);
}

#[test]
fn bent_boards_are_dropped_unless_added_by_hand() {
    let model = OpenCVFisheye::default();
    let truth = Lens::new(&model, (1100.0, 1100.0), (2010.0, 1985.0), &[-0.02, 0.001, 0.0, 0.0]);
    let (mut views, _) = views(&truth, &Shots { n: 50, center_max: 90.0, corner_max: 105.0, distance: (9.0, 14.0), noise: 0.5 }, &mut Rng(23));
    // A fast pan under a rolling shutter: every row shifted in proportion to its height in the frame
    let bent = [3usize, 11, 19, 27, 35, 43];
    for &v in &bent {
        let mean_y = views[v].iter().map(|p| p.1).sum::<f64>() / views[v].len() as f64;
        for p in views[v].iter_mut() { p.0 += 0.03 * (p.1 - mean_y); }
    }
    let fit = solver::calibrate(&model, &board(), &views, (4000, 4000), &[]).expect("a calibration");
    let err = difference(&Lens::fitted(&model, &fit), &truth, 105.0);
    report(&fit, views.len(), err, 105.0);
    for &v in &bent {
        assert!(!fit.kept.contains(&v), "bent view {v} kept (its rms {:.2} px)", fit.view_rms[v]);
    }
    assert!(err < 1.5, "the fitted lens is off by {err} px");

    // One added by hand stays, bent or not - and pulls the lens with it, which is the user's call
    let fit = solver::calibrate(&model, &board(), &views, (4000, 4000), &[bent[0]]).expect("a calibration");
    assert!(fit.kept.contains(&bent[0]), "the view added by hand was dropped");
    assert!(bent[1..].iter().all(|v| !fit.kept.contains(v)));
}

/// The solver fits with the very projection the render draws with: the model's generic `distort`, in f64 there and
/// through `distort_point` in f32 here
#[test]
fn the_solver_projects_like_the_render() {
    let cases: [(&dyn Model, &str, Vec<f64>, f64); 3] = [
        (&OpenCVFisheye::default(),     "opencv_fisheye",     vec![0.05, -0.01, 0.003, -0.0004], 150.0),
        (&OpenCVStandard::default(),    "opencv_standard",    vec![-0.12, 0.08, 0.0008, -0.0005, -0.02, 0.01, -0.005, 0.002, 0.0001, -0.0002, 0.0003, 0.0001], 60.0),
        (&GenericPolynomial::default(), "generic_polynomial", vec![1.0, 0.03, -0.06, 0.02, 0.004, -0.002, 0.0005], 150.0),
    ];
    let mut rng = Rng(3);
    for (model, id, k, field) in cases {
        let render = DistortionModel::from_name(id);
        assert_eq!(render.id(), id);
        let mut kp = KernelParams::default();
        for (i, v) in k.iter().enumerate() { kp.k[i] = *v as f32; }
        let mut k12 = k.clone();
        k12.resize(12, 0.0);
        let mut worst = 0.0f64;
        for _ in 0..2000 {
            let (t, a) = (rng.uniform(0.0, field).to_radians(), rng.uniform(0.0, 2.0 * PI));
            let ray = Vector3::new(t.sin() * a.cos(), t.sin() * a.sin(), t.cos());
            let (u, v) = model.project(&k12, &ray);
            let (uf, vf) = render.distort_point(ray.x as f32, ray.y as f32, ray.z as f32, &kp);
            worst = worst.max((u - uf as f64).hypot(v - vf as f64) / u.hypot(v).max(1e-3));
        }
        println!("{id}: f32 and f64 agree to {worst:.2e} (relative)");
        assert!(worst < 1e-5, "{id}: the solver's projection and the render's differ by {worst:e}");
    }
}

/// The boards detected in a real clip, as JSON `{ "size": [w, h], "views": [[[x, y], ...], ...] }` (14x8 inner
/// corners, `findChessboardCornersSB`'s order), from `GYROFLOW_CALIB_POINTS`
#[test]
#[ignore = "needs a file of detected boards"]
fn detected_boards_from_a_file() {
    let path = std::env::var("GYROFLOW_CALIB_POINTS").expect("GYROFLOW_CALIB_POINTS");
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let size = (json["size"][0].as_u64().unwrap() as usize, json["size"][1].as_u64().unwrap() as usize);
    let views: Vec<Vec<(f64, f64)>> = json["views"].as_array().unwrap().iter().map(|v| {
        v.as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect()
    }).collect();
    let models: [(&dyn Model, &str); 3] = [
        (&OpenCVFisheye::default(), "opencv_fisheye"), (&OpenCVStandard::default(), "opencv_standard"), (&GenericPolynomial::default(), "generic_polynomial"),
    ];
    for (model, id) in models {
        let started = std::time::Instant::now();
        match solver::calibrate(model, &board(), &views, size, &[]) {
            Some(fit) => println!("{id}: {:.2}s, rms {:.4} px on {} of {} views, f ({:.2}, {:.2}), c ({:.2}, {:.2}), k {:?}, boards up to {:.1}°",
                started.elapsed().as_secs_f64(), fit.rms, fit.kept.len(), views.len(), fit.f.0, fit.f.1, fit.c.0, fit.c.1, &fit.k[..fit.fitted], fit.max_corner_angle.to_degrees()),
            None => println!("{id}: no calibration"),
        }
    }
}
