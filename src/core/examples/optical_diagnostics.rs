//! Inspect the derived optical crop and correction of a saved project.
use std::sync::{Arc, atomic::AtomicBool};
use gyroflow_core::{StabilizationManager, filesystem};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 { return Err("Usage: optical_diagnostics INPUT.gyroflow OUTPUT.json".into()); }
    let manager = StabilizationManager::default();
    let input = std::path::Path::new(&args[1]).canonicalize()?;
    let mut input = input.to_str().ok_or("Input path is not UTF-8")?.to_owned();
    if cfg!(target_os = "windows") { input = input.replace('\\', "/"); }
    manager.import_gyroflow_file(&filesystem::path_to_url(&input), true, |_| {}, Arc::new(AtomicBool::new(false)), false)?;
    let reserve = gyroflow_core::synchronization::residual_motion::zoom_reserve_factor(&gyroflow_core::stabilization::ComputeParams::from_manager(&manager));
    let params = manager.params.read();
    let stab = manager.stabilization.read();
    let grids = stab.optical_grids();
    let margins = stab.optical_crop_margins();
    let report = serde_json::json!({
        "frames": params.frame_count, "fps": params.get_scaled_fps(),
        "size": params.size, "output_size": params.output_size,
        "base_fovs": params.fovs, "optical_crop_margins": &*margins,
        "max_zoom": params.max_zoom, "manual_fov": params.fov,
        "requested_zoom_factor": reserve,
        "max_displacement_by_frame": grids.iter().map(|g| g.0.iter().flatten().fold(0.0f32, |a, b| a.max(b.abs()))).collect::<Vec<_>>(),
        "derivative_bound_by_frame": grids.iter().map(|g| g.derivative_bound()).collect::<Vec<_>>()
    });
    std::fs::write(&args[2], serde_json::to_vec_pretty(&report)?)?;
    println!("Inspected {} frames, {} correction grids", params.frame_count, grids.len());
    Ok(())
}
