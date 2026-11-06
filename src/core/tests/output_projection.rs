// SPDX-License-Identifier: GPL-3.0-or-later
//
// The output projection is picked in the Lens profile section and is rectilinear unless picked otherwise, whatever
// the lens. "Automatic" is `OutputProjection::for_lens`: stereographic only for a lens that sees past 90° from its
// axis, which is where rectilinear runs out.

use gyroflow_core::StabilizationManager;
use gyroflow_core::stabilization::ComputeParams;
use gyroflow_core::stabilization::projection::{ self, OutputProjection };
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

fn lens(w: usize, h: usize, f: (f64, f64), c: (f64, f64), k: [f64; 4]) -> String {
    serde_json::json!({
        "name": "test", "calibrated_by": "test", "camera_brand": "test", "camera_model": "test",
        "calib_dimension": { "w": w, "h": h }, "orig_dimension": { "w": w, "h": h },
        "fps": 30.0, "calibrator_version": "---",
        "fisheye_params": {
            "camera_matrix": [[f.0, 0.0, c.0], [0.0, f.1, c.1], [0.0, 0.0, 1.0]],
            "distortion_coeffs": k
        }
    }).to_string()
}

/// HERO13 + Max Lens Mod 2.0 at 8:7 5.3K: the image circle runs past the curve's fold at 95.5°
fn max_lens_mod() -> (usize, usize, String) {
    (5312, 4648, lens(5312, 4648, (1161.619, 1173.635), (2649.377, 2247.909), [0.15238944, -0.01720013, 0.01529571, -0.00729304]))
}
/// An equidistant-ish fisheye whose 1080p frame corner sits at 87° from the axis: past the old 85° line, inside 90°
fn corner_at_87() -> (usize, usize, String) {
    let f = (960.0f64).hypot(540.0) / 87f64.to_radians();
    (1920, 1080, lens(1920, 1080, (f, f), (960.0, 540.0), [1e-6, 0.0, 0.0, 0.0]))
}

fn manager((w, h, json): (usize, usize, String)) -> StabilizationManager {
    let stab = StabilizationManager::default();
    stab.init_from_video_data(10_000.0, 30.0, 300, (w, h));
    stab.set_output_size(w, h);
    stab.load_lens_profile(&json).unwrap();
    stab
}

fn drawn_with(stab: &StabilizationManager) -> OutputProjection {
    OutputProjection::from_i32(ComputeParams::from_manager(stab).output_projection)
}

#[test]
fn rectilinear_unless_picked_otherwise() {
    let stab = manager(max_lens_mod());
    assert_eq!(drawn_with(&stab), OutputProjection::Rectilinear, "a lens past 90° still opens rectilinear");

    stab.set_output_projection(OutputProjection::Equidistant as i32);
    assert_eq!(drawn_with(&stab), OutputProjection::Equidistant);

    // A new file starts from the default again
    stab.clear();
    assert_eq!(stab.params.read().output_projection, OutputProjection::Rectilinear as i32);
}

#[test]
fn automatic_switches_only_past_90_degrees() {
    let stab = manager(max_lens_mod());
    stab.set_output_projection(projection::AUTOMATIC);
    assert_eq!(drawn_with(&stab), OutputProjection::Stereographic, "the Max Lens Mod reaches 95.5°");

    let stab = manager(corner_at_87());
    stab.set_output_projection(projection::AUTOMATIC);
    assert_eq!(drawn_with(&stab), OutputProjection::Rectilinear, "87° is inside rectilinear's reach");
}

/// A project opened over the video already loaded skips `clear`, so what the import does is all that decides it
#[test]
fn a_project_from_before_the_setting_is_rectilinear_and_a_preset_without_it_leaves_it() {
    let import = |stab: &StabilizationManager, json: serde_json::Value| -> bool {
        let mut is_preset = false;
        stab.import_gyroflow_data(json.to_string().as_bytes(), true, None, |_| {}, Arc::new(AtomicBool::new(false)), &mut is_preset, false).unwrap();
        is_preset
    };
    let current = |stab: &StabilizationManager| stab.params.read().output_projection;
    let stab = StabilizationManager::default();

    stab.set_output_projection(OutputProjection::Stereographic as i32);
    assert!(!import(&stab, serde_json::json!({ "videofile": "C:/clips/old.mp4", "stabilization": { "fov": 1.0 } })));
    assert_eq!(current(&stab), OutputProjection::Rectilinear as i32, "an old project");

    stab.set_output_projection(OutputProjection::Stereographic as i32);
    import(&stab, serde_json::json!({ "videofile": "C:/clips/new.mp4", "stabilization": { "output_projection": "a future one" } }));
    assert_eq!(current(&stab), OutputProjection::Rectilinear as i32, "a name this version doesn't know");

    import(&stab, serde_json::json!({ "videofile": "C:/clips/new.mp4", "stabilization": { "output_projection": "equidistant" } }));
    assert_eq!(current(&stab), OutputProjection::Equidistant as i32);

    assert!(import(&stab, serde_json::json!({ "stabilization": { "fov": 1.0 } })));
    assert_eq!(current(&stab), OutputProjection::Equidistant as i32, "a preset without it");
    import(&stab, serde_json::json!({ "stabilization": { "output_projection": "automatic" } }));
    assert_eq!(current(&stab), projection::AUTOMATIC, "a preset with it");
}

#[test]
fn the_project_file_names_round_trip() {
    for v in [projection::AUTOMATIC, 0, 1, 2, 3, 4] {
        let name = projection::setting_name(v);
        assert_eq!(projection::setting_from_name(name), Some(v), "{name}");
    }
    assert_eq!(projection::setting_from_name("fisheye"), None);
}
