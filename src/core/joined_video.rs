// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Gyroflow contributors

//! A video joined from consecutive files (eg. the chapters a camera splits a long recording into), without writing a
//! joined copy of them: an FFmpeg concat script (`.ffconcat`) next to the files lists them with their durations. The
//! preview and the renderer open it with FFmpeg's concat demuxer as one video, and the motion data is read from every
//! file and joined here.

use std::sync::{ Arc, atomic::{ AtomicBool, Ordering::SeqCst } };
use crate::filesystem;
use crate::gyro_source::{ FileLoadOptions, FileMetadata, GyroSource };

pub const EXTENSION: &str = "ffconcat";

#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub url: String,
    pub duration_ms: f64,
}

pub fn is_joined(url: &str) -> bool {
    filesystem::get_filename(url).to_ascii_lowercase().ends_with(&format!(".{EXTENSION}"))
}

/// Writes the script for `parts` (in this order) as `filename` in `folder` and returns its url. The parts have to be in
/// that folder: the script lists them by name, which is what FFmpeg opens without disabling its `safe` option
pub fn write(folder: &str, filename: &str, parts: &[Part]) -> Result<String, crate::GyroflowCoreError> {
    let url = filesystem::get_file_url(folder, filename, true);
    filesystem::write(&url, script(parts).as_bytes())?;
    Ok(url)
}

fn script(parts: &[Part]) -> String {
    let mut out = String::from("ffconcat version 1.0\n");
    for x in parts {
        // In single quotes, a quote is written as '\''
        out.push_str(&format!("file '{}'\nduration {:.6}\n", filesystem::get_filename(&x.url).replace('\'', "'\\''"), x.duration_ms / 1000.0));
    }
    out
}

/// The parts of a joined video, with their urls resolved against the folder of the script
pub fn read(url: &str) -> Result<Vec<Part>, crate::GyroflowCoreError> {
    let folder = filesystem::get_folder(url);
    let parts = parse(&filesystem::read_to_string(url)?, |name| filesystem::get_file_url(&folder, name, false));
    if parts.is_empty() {
        return Err(std::io::Error::other(format!("No files listed in {url}")).into());
    }
    Ok(parts)
}

fn parse<F: Fn(&str) -> String>(script: &str, resolve: F) -> Vec<Part> {
    let mut parts: Vec<Part> = Vec::new();
    for line in script.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix("file ") {
            let name = name.trim();
            let name = name.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')).map(|x| x.replace("'\\''", "'")).unwrap_or_else(|| name.to_string());
            parts.push(Part { url: resolve(&name), duration_ms: 0.0 });
        } else if let Some(duration) = line.strip_prefix("duration ") {
            if let (Some(last), Ok(v)) = (parts.last_mut(), duration.trim().parse::<f64>()) {
                last.duration_ms = v * 1000.0;
            }
        }
    }
    parts
}

/// Reads the motion data of every part as if it was loaded alone, and joins it: the timestamps of a part are shifted by
/// the durations of the parts before it, and the per frame data is appended
pub fn parse_telemetry<F: Fn(f64)>(url: &str, options: &FileLoadOptions, size: (usize, usize), fps: f64, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> Result<FileMetadata, crate::GyroflowCoreError> {
    let parts = read(url)?;
    let total_ms = parts.iter().map(|x| x.duration_ms).sum::<f64>().max(1.0);
    let mut joined: Option<FileMetadata> = None;
    let mut offset_ms = 0.0;
    for part in &parts {
        if cancel_flag.load(SeqCst) { break; }
        let mut file = filesystem::open_file(&part.url, false, false)?;
        let filesize = file.size;
        let start = offset_ms / total_ms;
        let md = GyroSource::parse_telemetry_file(file.get_file(), filesize, &part.url, options, size, fps, |p| progress_cb(start + p * part.duration_ms / total_ms), cancel_flag.clone())?;
        let frames = (part.duration_ms * fps / 1000.0).round() as usize;
        joined = Some(match joined {
            None => pad_frames(md, frames),
            Some(joined) => append(joined, md, offset_ms, frames),
        });
        offset_ms += part.duration_ms;
    }
    joined.ok_or_else(|| std::io::Error::other(format!("No motion data in {url}")).into())
}

/// The per frame time offsets are needed for every frame once one part has them, the other per frame data (from Sony
/// cameras) is used only when every part has it
fn pad_frames(mut md: FileMetadata, frames: usize) -> FileMetadata {
    if !md.per_frame_time_offsets.is_empty() { md.per_frame_time_offsets.resize(frames, 0.0); }
    md
}

fn append(mut a: FileMetadata, b: FileMetadata, offset_ms: f64, frames: usize) -> FileMetadata {
    let offset_us = (offset_ms * 1000.0).round() as i64;
    let a_frames = (offset_ms * a.frame_rate.unwrap_or(0.0) / 1000.0).round() as usize;

    a.raw_imu.extend(b.raw_imu.into_iter().map(|mut x| { x.timestamp_ms += offset_ms; x }));
    a.quaternions.extend(b.quaternions.into_iter().map(|(t, v)| (t + offset_us, v)));
    if let Some(bv) = b.gravity_vectors {
        a.gravity_vectors.get_or_insert_with(Default::default).extend(bv.into_iter().map(|(t, v)| (t + offset_us, v)));
    }
    if let Some(bv) = b.image_orientations {
        a.image_orientations.get_or_insert_with(Default::default).extend(bv.into_iter().map(|(t, v)| (t + offset_us, v)));
    }
    a.lens_positions.extend(b.lens_positions.into_iter().map(|(t, v)| (t + offset_us, v)));
    a.lens_params.extend(b.lens_params.into_iter().map(|(t, v)| (t + offset_us, v)));

    if !a.per_frame_time_offsets.is_empty() || !b.per_frame_time_offsets.is_empty() {
        a.per_frame_time_offsets.resize(a_frames.max(a.per_frame_time_offsets.len()), 0.0);
        let mut bv = b.per_frame_time_offsets;
        bv.resize(frames, 0.0);
        a.per_frame_time_offsets.extend(bv);
    }
    if !a.camera_stab_data.is_empty() && !b.camera_stab_data.is_empty() {
        a.camera_stab_data.extend(b.camera_stab_data);
    } else {
        a.camera_stab_data.clear();
    }
    if !a.lens_breathing.is_empty() && !b.lens_breathing.is_empty() {
        a.lens_breathing.extend(b.lens_breathing);
    } else {
        a.lens_breathing.clear();
    }
    if !a.mesh_correction.frames.is_empty() && !b.mesh_correction.frames.is_empty() {
        let table_offset = a.mesh_correction.tables.len() as u32;
        a.mesh_correction.tables.extend(b.mesh_correction.tables);
        a.mesh_correction.frames.extend(b.mesh_correction.frames.into_iter().map(|mut x| { x.table = x.table.map(|t| t + table_offset); x }));
    } else {
        a.mesh_correction = Default::default();
    }

    // The rest describes the camera and the recording settings, which the parts share
    if a.lens_profile.is_none() { a.lens_profile = b.lens_profile; }
    if a.camera_identifier.is_none() { a.camera_identifier = b.camera_identifier; }
    if a.frame_readout_time.is_none() { a.frame_readout_time = b.frame_readout_time; }
    if a.imu_orientation.is_none() { a.imu_orientation = b.imu_orientation; }
    if a.detected_source.is_none() { a.detected_source = b.detected_source; }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_round_trip() {
        let parts = vec![
            Part { url: "file:///card/GX012209.MP4".into(), duration_ms: 732160.0 },
            Part { url: "file:///card/it's GX022209.MP4".into(), duration_ms: 200000.0 },
        ];
        let script = script(&parts);
        assert!(script.starts_with("ffconcat version 1.0\n"));
        assert!(script.contains("file 'it'\\''s GX022209.MP4'\nduration 200.000000\n"));
        assert_eq!(parse(&script, |name| format!("file:///card/{name}")), parts);
    }

    #[test]
    fn timestamps_are_shifted_by_the_parts_before() {
        let mut a = FileMetadata::default();
        a.frame_rate = Some(25.0);
        a.quaternions.insert(1000, Default::default());
        a.raw_imu.push(crate::gyro_source::TimeIMU { timestamp_ms: 1.0, ..Default::default() });
        a.per_frame_time_offsets = vec![0.5; 25];
        let mut b = FileMetadata::default();
        b.quaternions.insert(1000, Default::default());
        b.raw_imu.push(crate::gyro_source::TimeIMU { timestamp_ms: 1.0, ..Default::default() });

        let joined = append(a, b, 1000.0, 50);
        assert_eq!(joined.quaternions.keys().copied().collect::<Vec<_>>(), vec![1000, 1_001_000]);
        assert_eq!(joined.raw_imu.iter().map(|x| x.timestamp_ms).collect::<Vec<_>>(), vec![1.0, 1001.0]);
        // The second part has no offsets, but its frames still need one each
        assert_eq!(joined.per_frame_time_offsets.len(), 75);
    }
}
