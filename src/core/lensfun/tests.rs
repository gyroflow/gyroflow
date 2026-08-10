// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2025 Adrian <adrian.eddy at gmail>

use super::*;
use crate::stabilization::KernelParams;
use crate::stabilization::distortion_models::DistortionModel;

// Vendored sample of the lensfun database (https://github.com/lensfun/lensfun/tree/master/data/db).
// The EF-M lenses and cameras are copied from mil-canon.xml (the 18-55mm zoom is trimmed to
// 4 of its 6 distortion points). The EF poly3 and EF-S poly5 lenses use synthetic but
// representative values with clean numbers for hand-computed verification below.
const FIXTURE_XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<lensfun>
    <camera>
        <maker>Canon</maker>
        <model>Canon EOS M50</model>
        <model lang="en">EOS M50</model>
        <mount>Canon EF-M</mount>
        <cropfactor>1.613</cropfactor>
    </camera>
    <camera>
        <maker>Canon</maker>
        <model>Canon EOS 5D Mark III</model>
        <model lang="en">EOS 5D Mark III</model>
        <mount>Canon EF</mount>
        <cropfactor>1</cropfactor>
    </camera>
    <camera>
        <maker>Canon</maker>
        <model>Canon EOS 80D</model>
        <model lang="en">EOS 80D</model>
        <mount>Canon EF</mount>
        <mount>Canon EF-S</mount>
        <cropfactor>1.6</cropfactor>
    </camera>

    <lens>
        <maker>Canon</maker>
        <model>Canon EF-M 18-55mm f/3.5-5.6 IS STM</model>
        <mount>Canon EF-M</mount>
        <cropfactor>1.613</cropfactor>
        <calibration>
            <distortion model="ptlens" focal="18" a="0.0208" b="-0.06707" c="0.02864"/>
            <distortion model="ptlens" focal="24" a="0.01074" b="-0.03245" c="0.01496"/>
            <distortion model="ptlens" focal="35" a="0.00738" b="-0.0241" c="0.0299"/>
            <distortion model="ptlens" focal="55" a="0.00824" b="-0.0265" c="0.0378"/>
            <tca model="poly3" focal="18" br="-0.0001552" vr="1.0005722" bb="0.0001298" vb="0.9997515"/>
            <vignetting model="pa" focal="18" aperture="3.5" distance="10" k1="-1.3322" k2="1.1067" k3="-0.4495"/>
        </calibration>
    </lens>

    <lens>
        <maker>Canon</maker>
        <model>Canon EF 50mm f/1.8 (test poly3)</model>
        <mount>Canon EF</mount>
        <cropfactor>1</cropfactor>
        <calibration>
            <distortion model="poly3" focal="50" k1="0.01"/>
        </calibration>
    </lens>

    <lens>
        <maker>Canon</maker>
        <model>Canon EF-S 12mm f/2.8 (test poly5)</model>
        <mount>Canon EF-S</mount>
        <cropfactor>1.6</cropfactor>
        <calibration>
            <distortion model="poly5" focal="12" k1="0.005" k2="-0.0002" real-focal="11.94"/>
        </calibration>
    </lens>
</lensfun>
"#;

fn fixture_db() -> LensfunDatabase {
    LensfunDatabase::from_xml(FIXTURE_XML).expect("fixture XML should parse")
}

fn approx(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn lensfun_parse_fixture() {
    let db = fixture_db();

    assert_eq!(db.cameras.len(), 3);
    assert_eq!(db.lenses.len(), 3);

    // First <model> (without lang attribute) wins
    assert_eq!(db.cameras[0].model, "Canon EOS M50");
    assert_eq!(db.cameras[0].crop_factor, 1.613);

    // Multiple mounts per camera
    assert_eq!(db.cameras[2].mounts, vec!["Canon EF".to_string(), "Canon EF-S".to_string()]);

    // tca/vignetting entries are ignored, 4 distortion points parsed
    let zoom = &db.lenses[0];
    assert_eq!(zoom.calibrations.len(), 1);
    assert_eq!(zoom.calibrations[0].distortions.len(), 4);
    assert_eq!(zoom.calibrations[0].crop_factor, 1.613);
    assert_eq!(zoom.calibrations[0].aspect_ratio, 1.5); // default

    // real-focal attribute
    let poly5 = &db.lenses[2].calibrations[0].distortions[0];
    assert_eq!(poly5.real_focal, Some(11.94));
    assert_eq!(poly5.terms, vec![0.005, -0.0002]);
}

#[test]
fn lensfun_convert_poly3_coefficients() {
    // Hand-computed expectations for: poly3, focal=50, k1=0.01, calib crop=1.0,
    // aspect=1.5 (default), target crop=1.0 (EOS 5D Mark III):
    //   real_focal  = 50 * (1 - 0.01) = 49.5
    //   hugin_scale = hypot(36,24) / 1.0 / hypot(1.5,1) / 2 = 12 mm (36:24 == 1.5:1)
    //   hugin_scaling = 49.5 / 12 = 4.125
    //   d = 1 - k1 = 0.99
    //   k1' = k1 * hugin_scaling^2 / d^3 = 0.01 * 17.015625 / 0.970299 = 0.175364758698092
    //   frame = 3240x2160, diag_px = 3894, sensor_diag = 43.2666 -> fx = 49.5 * 90 = 4455
    let db = fixture_db();
    let profiles = db.to_lens_profiles();
    let p = profiles.iter().find(|p| p.identifier == "lensfun:Canon EOS 5D Mark III|Canon EF 50mm f/1.8 (test poly3)").expect("poly3 profile for 5D");

    assert_eq!(p.distortion_model.as_deref(), Some("poly3"));
    assert_eq!(p.camera_brand, "Canon");
    assert_eq!(p.camera_model, "EOS 5D Mark III");
    assert_eq!(p.lens_model, "EF 50mm f/1.8 (test poly3)");
    assert_eq!(p.calib_dimension.w, 3240);
    assert_eq!(p.calib_dimension.h, 2160);

    assert!(approx(p.focal_length.unwrap(), 49.5, 1e-9));
    assert_eq!(p.fisheye_params.distortion_coeffs.len(), 1);
    assert!(approx(p.fisheye_params.distortion_coeffs[0], 0.175364758698092, 1e-12));

    let m = &p.fisheye_params.camera_matrix;
    assert!(approx(m[0][0], 4455.0, 1e-9));
    assert!(approx(m[1][1], 4455.0, 1e-9));
    assert!(approx(m[0][2], 1620.0, 1e-9));
    assert!(approx(m[1][2], 1080.0, 1e-9));
}

#[test]
fn lensfun_convert_ptlens_coefficients() {
    // Hand-computed expectations for the EF-M 18-55mm at 18mm on the EOS M50
    // (calib crop = target crop = 1.613, aspect = 1.5):
    //   a=0.0208, b=-0.06707, c=0.02864, focal=18
    //   d = 1 - a - b - c = 1.01763
    //   real_focal = 18 * d = 18.31734
    //   hugin_scale = hypot(36,24) / 1.613 / hypot(1.5,1) / 2 = 7.439771428571429
    //   hugin_scaling = 18.31734 / 7.439771... = 2.462155785
    //   a' = a * hs^3 / d^4 = 0.2895011629663039
    //   b' = b * hs^2 / d^3 = -0.38582437169452544
    //   c' = c * hs   / d^2 = 0.06809398307832905
    //   fx = 18.31734 * 3894 / (43.2666.../1.613) = 2659.1282478
    let db = fixture_db();
    let profiles = db.to_lens_profiles();
    let p = profiles.iter().find(|p| p.identifier == "lensfun:Canon EOS M50|Canon EF-M 18-55mm f/3.5-5.6 IS STM").expect("ptlens profile for M50");

    assert_eq!(p.distortion_model.as_deref(), Some("ptlens"));
    assert!(approx(p.focal_length.unwrap(), 18.31734, 1e-9));

    let k = &p.fisheye_params.distortion_coeffs;
    assert_eq!(k.len(), 3);
    assert!(approx(k[0],  0.2895011629663039,  1e-12));
    assert!(approx(k[1], -0.38582437169452544, 1e-12));
    assert!(approx(k[2],  0.06809398307832905, 1e-12));

    assert!(approx(p.fisheye_params.camera_matrix[0][0], 2659.1282478, 1e-6));

    // Zoom lens: all 4 focal lengths exposed via the interpolations plumbing
    let interp = p.interpolations.as_ref().and_then(|x| x.as_object()).expect("interpolations");
    assert_eq!(interp.len(), 4);
    for f in ["18", "24", "35", "55"] {
        assert!(interp.contains_key(f), "missing interpolation at {}mm", f);
    }
}

#[test]
fn lensfun_crop_factor_rule() {
    let db = fixture_db();
    let profiles = db.to_lens_profiles();

    // EF-S lens calibrated on a 1.6x sensor: applicable to the 80D (1.6x),
    // but NOT to the 5D (1.0x): r = 1.0/1.6 = 0.625 < 0.96
    assert!(profiles.iter().any(|p| p.identifier == "lensfun:Canon EOS 80D|Canon EF-S 12mm f/2.8 (test poly5)"));
    assert!(!profiles.iter().any(|p| p.identifier == "lensfun:Canon EOS 5D Mark III|Canon EF-S 12mm f/2.8 (test poly5)"));

    // EF-M lens only matches EF-M cameras
    assert!(!profiles.iter().any(|p| p.identifier == "lensfun:Canon EOS 80D|Canon EF-M 18-55mm f/3.5-5.6 IS STM"));

    // Full-frame EF lens (calib crop 1.0) fits both the 5D (r = 1.0) and the 80D (r = 1.6)
    assert!(profiles.iter().any(|p| p.identifier == "lensfun:Canon EOS 5D Mark III|Canon EF 50mm f/1.8 (test poly3)"));
    assert!(profiles.iter().any(|p| p.identifier == "lensfun:Canon EOS 80D|Canon EF 50mm f/1.8 (test poly3)"));

    // real-focal attribute is used when present
    let p = profiles.iter().find(|p| p.identifier == "lensfun:Canon EOS 80D|Canon EF-S 12mm f/2.8 (test poly5)").unwrap();
    assert_eq!(p.distortion_model.as_deref(), Some("poly5"));
    assert!(approx(p.focal_length.unwrap(), 11.94, 1e-9));
}

#[test]
fn lensfun_hermite_interpolation() {
    // Port of lensfun's InterpolateDistortion (_lf_interpolate from auxfun.cpp).
    // Expected values computed with lensfun's own formulas:
    // terms are preconditioned by multiplying with the point's focal length,
    // cubic Hermite between the nearest focals, then divided by the target focal.
    let db = fixture_db();
    let points = &db.lenses[0].calibrations[0].distortions;

    // Exact match returns the calibration point unchanged
    let exact = interpolate_distortion(points, 24.0).unwrap();
    assert_eq!(exact.terms, vec![0.01074, -0.03245, 0.01496]);

    // Halfway between 18 and 24 (t = 0.5, 35mm point used as the outer spline point):
    //   a: hermite(None, 18*0.0208, 24*0.01074, 35*0.00738, 0.5) / 21 = 0.014702678571428571
    //   b: hermite(None, 18*-0.06707, 24*-0.03245, 35*-0.0241, 0.5) / 21 = -0.045819404761904756
    //   c: hermite(None, 18*0.02864, 24*0.01496, 35*0.0299, 0.5) / 21 = 0.018311130952380954
    let mid = interpolate_distortion(points, 21.0).unwrap();
    assert!(approx(mid.terms[0],  0.014702678571428571,  1e-12));
    assert!(approx(mid.terms[1], -0.045819404761904756, 1e-12));
    assert!(approx(mid.terms[2],  0.018311130952380954, 1e-12));

    // Between 24 and 35 at 28mm with outer points 18 and 55 (t = 4/11):
    //   a: hermite(18*0.0208, 24*0.01074, 35*0.00738, 55*0.00824, 4/11) / 28 = 0.008612539444027048
    //   real_focal: hermite over 18*(1-a-b-c) etc. = 27.222577370398195
    let mid4 = interpolate_distortion(points, 28.0).unwrap();
    assert!(approx(mid4.terms[0], 0.008612539444027048, 1e-12));
    assert!(approx(mid4.real_focal.unwrap(), 27.222577370398195, 1e-9));

    // Outside the calibrated range: nearest point wins (no extrapolation)
    let low = interpolate_distortion(points, 10.0).unwrap();
    assert_eq!(low.focal, 18.0);
    let high = interpolate_distortion(points, 80.0).unwrap();
    assert_eq!(high.focal, 55.0);
}

#[test]
fn lensfun_interpolated_profile_lookup() {
    // The converted profile must resolve interpolations and interpolate at
    // intermediate focal lengths through the standard LensProfile plumbing
    let db = fixture_db();
    let mut profiles = db.to_lens_profiles();
    let p = profiles.iter_mut().find(|p| p.identifier == "lensfun:Canon EOS M50|Canon EF-M 18-55mm f/3.5-5.6 IS STM").unwrap();

    let empty_db = crate::lens_profile_database::LensProfileDatabase::default();
    p.resolve_interpolations(&empty_db);

    // Exact calibration focal returns that calibration's coefficients
    let at18 = p.get_interpolated_lens_at(18.0);
    assert!(approx(at18.fisheye_params.distortion_coeffs[0], 0.2895011629663039, 1e-12));

    // Intermediate focal returns something finite between the neighbors
    let at21 = p.get_interpolated_lens_at(21.0);
    let k = &at21.fisheye_params.distortion_coeffs;
    assert!(k.iter().all(|x| x.is_finite()));
    let k18 = &p.get_interpolated_lens_at(18.0).fisheye_params.distortion_coeffs;
    let k24 = &p.get_interpolated_lens_at(24.0).fisheye_params.distortion_coeffs;
    for i in 0..3 {
        let (lo, hi) = (k18[i].min(k24[i]), k18[i].max(k24[i]));
        assert!(k[i] >= lo - 1e-9 && k[i] <= hi + 1e-9, "coeff {} out of range: {} not in [{}, {}]", i, k[i], lo, hi);
    }
}

/// Reference implementation of the raw lensfun/ptlens model (before Gyroflow's
/// coefficient rescaling), working in hugin-normalized units:
///   rd = ru * (a*ru^3 + b*ru^2 + c*ru + d),  d = 1-a-b-c
/// Solved for ru by Newton iterations, like lensfun's ModifyCoord_UnDist_PTLens.
fn reference_ptlens_undistort(rd: f64, a: f64, b: f64, c: f64) -> f64 {
    let d = 1.0 - a - b - c;
    let mut ru = rd;
    for _ in 0..50 {
        let fru = ru * (a * ru.powi(3) + b * ru.powi(2) + c * ru + d) - rd;
        if fru.abs() < 1e-12 { break; }
        ru -= fru / (4.0 * a * ru.powi(3) + 3.0 * b * ru.powi(2) + 2.0 * c * ru + d);
    }
    // lensfun scales the undistorted coordinates by d (see mod-coord.cpp header comment)
    ru * d
}

fn lensfun_undistort_sanity(model: &str, coeffs: &[f64], fx: f64, reference: &dyn Fn(f64) -> f64) {
    let distortion_model = DistortionModel::from_name(model);
    let mut params = KernelParams::default();
    let mut k = [0.0f32; 12];
    for (i, c) in coeffs.iter().enumerate() { k[i] = *c as f32; }
    params.k = k;

    // Test grid: distorted points at radii up to 0.45 (normalized), various angles
    let mut prev_ru = -1.0f64;
    for i in 1..20 {
        let rd = i as f64 * 0.025;
        let angle = i as f64 * 0.7;
        let (x, y) = ((rd * angle.cos()) as f32, (rd * angle.sin()) as f32);

        let res = distortion_model.undistort_point((x, y), &params);
        assert!(res.is_some(), "undistort_point failed at rd={}", rd);
        let (ux, uy) = res.unwrap();
        let ru = (ux as f64).hypot(uy as f64);
        assert!(ru.is_finite());

        // Monotonic radius mapping: undistorted radius grows with distorted radius
        assert!(ru > prev_ru, "radius mapping not monotonic at rd={}", rd);
        prev_ru = ru;

        // Compare against the lensfun reference behavior (converted to pixels, must be within a few px)
        let ru_ref = reference(rd);
        let diff_px = (ru - ru_ref).abs() * fx;
        assert!(diff_px < 1.0, "rd={}: gyroflow ru={} vs reference ru={} ({} px off)", rd, ru, ru_ref, diff_px);
    }
}

#[test]
fn lensfun_undistort_ptlens_matches_reference() {
    let (a, b, c, focal, crop) = (0.0208f64, -0.06707, 0.02864, 18.0, 1.613);
    let d = 1.0 - a - b - c;
    let real_focal = focal * d;
    let hugin_scale = 36.0f64.hypot(24.0) / crop / 1.5f64.hypot(1.0) / 2.0;
    let hugin_scaling = real_focal / hugin_scale;

    // Convert like the database converter does
    let mut coeffs = vec![a, b, c];
    PtLens::rescale_coeffs(&mut coeffs, hugin_scaling);

    let fx = 2659.1282478; // see lensfun_convert_ptlens_coefficients

    // Reference: hugin-normalized distorted radius -> undistorted radius in
    // real-focal-normalized units
    let reference = move |rd: f64| {
        let rd_hugin = rd * real_focal / hugin_scale;
        let ru_hugin = reference_ptlens_undistort(rd_hugin, a, b, c);
        ru_hugin * hugin_scale / real_focal
    };

    lensfun_undistort_sanity("ptlens", &coeffs, fx, &reference);
}

#[test]
fn lensfun_undistort_poly3_matches_reference() {
    let (k1, focal, crop) = (0.01f64, 50.0, 1.0);
    let real_focal = focal * (1.0 - k1);
    let hugin_scale = 36.0f64.hypot(24.0) / crop / 1.5f64.hypot(1.0) / 2.0;
    let hugin_scaling = real_focal / hugin_scale;

    let mut coeffs = vec![k1];
    Poly3::rescale_coeffs(&mut coeffs, hugin_scaling);

    let fx = 4455.0; // see lensfun_convert_poly3_coefficients

    // poly3 is ptlens with a=c=0, b=k1
    let reference = move |rd: f64| {
        let rd_hugin = rd * real_focal / hugin_scale;
        let ru_hugin = reference_ptlens_undistort(rd_hugin, 0.0, k1, 0.0);
        ru_hugin * hugin_scale / real_focal
    };

    lensfun_undistort_sanity("poly3", &coeffs, fx, &reference);
}

#[test]
fn lensfun_profiles_searchable() {
    // Converted profiles must satisfy the same invariants the UI list relies on
    let db = fixture_db();
    let profiles = db.to_lens_profiles();
    assert_eq!(profiles.len(), 4); // M50+EFM zoom, 5D+EF, 80D+EF, 80D+EF-S
    for p in &profiles {
        assert!(!p.camera_brand.is_empty() && !p.camera_model.is_empty());
        assert!(!p.identifier.is_empty());
        assert!(!p.get_display_name().is_empty() && p.get_display_name() != "---");
        assert!(p.fisheye_params.camera_matrix.len() == 3);
        assert!(p.calib_dimension.w > 0 && p.calib_dimension.h > 0);
    }
}

