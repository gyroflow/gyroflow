// SPDX-License-Identifier: GPL-3.0-or-later

//! Print persisted track coverage and the prepared residual field of a project.
use gyroflow_core::{
    StabilizationManager,
    stabilization::{ComputeParams, optical},
};
use std::{path::PathBuf, sync::Arc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("Usage: optical_report project.gyroflow")?,
    );
    let manager = StabilizationManager::default();
    manager.import_gyroflow_data(
        &std::fs::read(path)?,
        true,
        None,
        |_| (),
        Arc::default(),
        &mut false,
        false,
    )?;
    let mut params = ComputeParams::from_manager(&manager);
    let start = std::time::Instant::now();
    optical::prepare(&mut params);
    let frames: Vec<_> = params
        .optical_corrections
        .iter()
        .map(|(time, grid)| {
            let max_px = grid
                .0
                .iter()
                .map(|v| {
                    (v[0] as f64 * params.width as f64).hypot(v[1] as f64 * params.height as f64)
                })
                .fold(0.0_f64, f64::max);
            serde_json::json!({"time_us":time,"max_displacement_px":max_px,"grid":grid.0.to_vec()})
        })
        .collect();
    let observations: Vec<_> = params
        .optical_motion
        .frames
        .iter()
        .map(|f| serde_json::json!({"time_us":f.timestamp_us,"tracks":f.points.len()}))
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"prepare_seconds":start.elapsed().as_secs_f64(),"source_size":[params.width,params.height],"strength":params.optical_stabilization_strength,"observations":observations,"frames":frames})
        )?
    );
    Ok(())
}
