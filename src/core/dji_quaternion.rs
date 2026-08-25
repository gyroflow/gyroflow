// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026

//! Lossless in-place patching of DJI dvtm quaternion fields.
//!
//! The telemetry-parser crate intentionally exposes normalized telemetry, not
//! a protobuf writer. DJI's quaternion messages are fixed-width protobuf
//! fields, however, so we can safely patch only their four little-endian f32
//! payloads while copying every other byte of the MP4 unchanged.

use std::io::Cursor;
use std::sync::{Arc, atomic::AtomicBool};

use nalgebra::{Quaternion, UnitQuaternion, Vector3};

use crate::filesystem;
use crate::gyro_source::{GyroSource, Quat64, TimeQuat};
use crate::{GyroflowCoreError, StabilizationManager};

#[derive(Default, Clone)]
pub(crate) struct QuaternionEditState {
    edited: Option<TimeQuat>,
    undo: Vec<TimeQuat>,
    redo: Vec<TimeQuat>,
    revision: u64,
}

impl GyroSource {
    pub fn edited_quaternions(&self) -> Option<&TimeQuat> {
        self.quaternion_repair.edited.as_ref()
    }

    pub fn quaternion_edit_revision(&self) -> u64 {
        self.quaternion_repair.revision
    }

    pub fn clear_quaternion_edits(&mut self) {
        self.quaternion_repair = QuaternionEditState::default();
    }

    pub fn restore_original_quaternions(&mut self) -> bool {
        if self.quaternion_repair.edited.is_none()
            && self.quaternion_repair.undo.is_empty()
            && self.quaternion_repair.redo.is_empty()
        {
            return false;
        }
        self.quaternions = self.file_metadata.read().quaternions.clone();
        self.clear_quaternion_edits();
        true
    }

    fn editable_quaternions(&self) -> TimeQuat {
        self.edited_quaternions()
            .cloned()
            .unwrap_or_else(|| self.file_metadata.read().quaternions.clone())
    }

    pub fn smooth_quaternion_range(&mut self, start_ms: f64, end_ms: f64, strength: u8) -> usize {
        if self.integration_method != 0 || strength == 0 {
            return 0;
        }
        let mut edited = self.editable_quaternions();
        let before = edited.clone();
        let count = smooth_quaternion_range(
            &mut edited,
            (start_ms.min(end_ms) * 1000.0).round() as i64,
            (start_ms.max(end_ms) * 1000.0).round() as i64,
            strength.clamp(1, 3),
        );
        if count > 0 {
            self.quaternion_repair.undo.push(before);
            self.quaternion_repair.redo.clear();
            self.quaternion_repair.edited = Some(edited.clone());
            self.quaternions = edited;
            self.quaternion_repair.revision = self.quaternion_repair.revision.wrapping_add(1);
        }
        count
    }

    pub fn undo_quaternion_edit(&mut self) -> bool {
        let Some(previous) = self.quaternion_repair.undo.pop() else {
            return false;
        };
        self.quaternion_repair
            .redo
            .push(self.editable_quaternions());
        self.quaternion_repair.edited = Some(previous.clone());
        self.quaternions = previous;
        self.quaternion_repair.revision = self.quaternion_repair.revision.wrapping_add(1);
        true
    }

    pub fn redo_quaternion_edit(&mut self) -> bool {
        let Some(next) = self.quaternion_repair.redo.pop() else {
            return false;
        };
        self.quaternion_repair
            .undo
            .push(self.editable_quaternions());
        self.quaternion_repair.edited = Some(next.clone());
        self.quaternions = next;
        self.quaternion_repair.revision = self.quaternion_repair.revision.wrapping_add(1);
        true
    }
}

impl StabilizationManager {
    pub fn smooth_quaternion_range(&self, start_ms: f64, end_ms: f64, strength: u8) -> usize {
        let count = self
            .gyro
            .write()
            .smooth_quaternion_range(start_ms, end_ms, strength);
        if count > 0 {
            self.invalidate_smoothing();
        }
        count
    }

    pub fn undo_quaternion_edit(&self) -> bool {
        let changed = self.gyro.write().undo_quaternion_edit();
        if changed {
            self.invalidate_smoothing();
        }
        changed
    }

    pub fn redo_quaternion_edit(&self) -> bool {
        let changed = self.gyro.write().redo_quaternion_edit();
        if changed {
            self.invalidate_smoothing();
        }
        changed
    }

    pub fn clear_quaternion_edits(&self) -> bool {
        let changed = self.gyro.write().restore_original_quaternions();
        if changed {
            self.invalidate_smoothing();
        }
        changed
    }

    pub fn save_fixed_quaternion_video(&self) -> Result<String, GyroflowCoreError> {
        let source_url = self.input_file.read().url.clone();
        if source_url.is_empty() {
            return Err(invalid("cannot save without an input video"));
        }

        let (original, edited) = {
            let gyro = self.gyro.read();
            let edited = gyro
                .edited_quaternions()
                .cloned()
                .ok_or_else(|| invalid("cannot save before any edit"))?;
            let original = gyro.file_metadata.read().quaternions.clone();
            (original, edited)
        };
        let source_filename = filesystem::get_filename(&source_url);
        let output_filename = filesystem::filename_with_suffix(&source_filename, "_fixed_gyro");
        let output_url =
            filesystem::get_file_url(&filesystem::get_folder(&source_url), &output_filename, true);
        if output_url.is_empty() || output_url == source_url {
            return Err(invalid("cannot create a distinct output URL"));
        }

        let changed = write_fixed_mp4(&source_url, &output_url, &original, &edited)?;
        log::info!("Saved {changed} edited DJI quaternion samples to {output_url}");
        Ok(output_url)
    }
}

const SMOOTHING_PASSES: usize = 3;
const MAX_FADE_US: i64 = 200_000;

fn preset_radius_ms(strength: u8) -> f64 {
    match strength.clamp(1, 3) {
        1 => 50.0,
        2 => 180.0,
        _ => 300.0,
    }
}

fn median_sample_interval_us(keys: &[i64]) -> Option<f64> {
    let mut intervals: Vec<i64> = keys
        .windows(2)
        .filter_map(|pair| {
            pair[1]
                .checked_sub(pair[0])
                .filter(|interval| *interval > 0)
        })
        .collect();
    if intervals.is_empty() {
        return None;
    }
    intervals.sort_unstable();
    let middle = intervals.len() / 2;
    Some(if intervals.len() % 2 == 0 {
        (intervals[middle - 1] as f64 + intervals[middle] as f64) / 2.0
    } else {
        intervals[middle] as f64
    })
}

fn box_blur_pass(input: &[Quat64], radius: usize) -> Vec<Quat64> {
    let mut prefix = vec![[0.0; 4]; input.len() + 1];
    for (i, quaternion) in input.iter().enumerate() {
        let q = quaternion.quaternion();
        prefix[i + 1] = [
            prefix[i][0] + q.w,
            prefix[i][1] + q.i,
            prefix[i][2] + q.j,
            prefix[i][3] + q.k,
        ];
    }

    (0..input.len())
        .map(|i| {
            let first = i.saturating_sub(radius);
            let last = i.saturating_add(radius).saturating_add(1).min(input.len());
            let count = (last - first) as f64;
            let averaged = Quaternion::new(
                (prefix[last][0] - prefix[first][0]) / count,
                (prefix[last][1] - prefix[first][1]) / count,
                (prefix[last][2] - prefix[first][2]) / count,
                (prefix[last][3] - prefix[first][3]) / count,
            );
            if averaged.norm_squared() <= f64::EPSILON {
                input[i]
            } else {
                UnitQuaternion::new_normalize(averaged)
            }
        })
        .collect()
}

fn smootherstep(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    value * value * value * (value * (value * 6.0 - 15.0) + 10.0)
}

/// Apply the DJIGyroFix-style three-pass quaternion moving average to a
/// user-selected interval, with a short SLERP fade into the source on each side.
fn smooth_quaternion_range(data: &mut TimeQuat, start_us: i64, end_us: i64, strength: u8) -> usize {
    if data.is_empty() || strength == 0 || start_us == end_us {
        return 0;
    }

    let (start_us, end_us) = (start_us.min(end_us), start_us.max(end_us));
    let duration_us = end_us - start_us;
    let fade_us = MAX_FADE_US
        .min((duration_us as f64 * 0.15).round() as i64)
        .max(1);
    let repair_start = start_us.saturating_sub(fade_us);
    let repair_end = end_us.saturating_add(fade_us);
    let radius_ms = preset_radius_ms(strength);
    let context_us = ((radius_ms * 4.0).max(750.0) * 1000.0).round() as i64;
    let context_start = repair_start.saturating_sub(context_us);
    let context_end = repair_end.saturating_add(context_us);
    let context: Vec<(i64, Quat64)> = data
        .range(context_start..=context_end)
        .map(|(timestamp, quaternion)| (*timestamp, *quaternion))
        .collect();
    if context.len() < 2 {
        return 0;
    }

    let keys: Vec<i64> = context.iter().map(|(timestamp, _)| *timestamp).collect();
    let stored_quaternions: Vec<Quat64> = context
        .iter()
        .map(|(_, quaternion)| UnitQuaternion::new_normalize(quaternion.clone().into_inner()))
        .collect();
    let repaired_indices: Vec<usize> = keys
        .iter()
        .enumerate()
        .filter_map(|(i, timestamp)| {
            (*timestamp > repair_start && *timestamp < repair_end).then_some(i)
        })
        .collect();
    if repaired_indices.is_empty() {
        return 0;
    }

    let mut continuous = stored_quaternions.clone();
    for i in 1..continuous.len() {
        if continuous[i - 1]
            .quaternion()
            .coords
            .dot(&continuous[i].quaternion().coords)
            < 0.0
        {
            continuous[i] = UnitQuaternion::from_quaternion(-continuous[i].quaternion().clone());
        }
    }

    let Some(median_interval_us) = median_sample_interval_us(&keys) else {
        return 0;
    };
    let radius_samples = ((radius_ms * 1000.0) / median_interval_us).round().max(1.0) as usize;
    let mut filtered = continuous.clone();
    for _ in 0..SMOOTHING_PASSES {
        filtered = box_blur_pass(&filtered, radius_samples);
    }

    let mut repaired = Vec::with_capacity(repaired_indices.len());
    for &i in &repaired_indices {
        let timestamp = keys[i];
        let amount = if timestamp < start_us {
            smootherstep((timestamp - repair_start) as f64 / fade_us as f64)
        } else if timestamp <= end_us {
            1.0
        } else {
            1.0 - smootherstep((timestamp - end_us) as f64 / fade_us as f64)
        };
        let mut output =
            UnitQuaternion::new_normalize(continuous[i].slerp(&filtered[i], amount).into_inner());
        if output
            .quaternion()
            .coords
            .dot(&stored_quaternions[i].quaternion().coords)
            < 0.0
        {
            output = UnitQuaternion::from_quaternion(-output.quaternion().clone());
        }
        repaired.push((timestamp, output));
    }

    for (timestamp, quaternion) in repaired {
        data.insert(timestamp, quaternion);
    }
    repaired_indices.len()
}

#[derive(Clone, Copy)]
struct Fixed32Field {
    payload: usize,
}

#[derive(Clone, Copy)]
struct QuaternionFields {
    w: Fixed32Field,
    x: Fixed32Field,
    y: Fixed32Field,
    z: Fixed32Field,
}

#[derive(Clone, Copy)]
struct LengthDelimitedField<'a> {
    data: &'a [u8],
    payload: usize,
}

fn invalid(message: impl Into<String>) -> GyroflowCoreError {
    log::warn!("DJI quaternion metadata writer: {}", message.into());
    GyroflowCoreError::InvalidData
}

fn read_varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *data.get(*pos)?;
        *pos += 1;
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn length_delimited_fields<'a>(
    data: &'a [u8],
    wanted: u32,
    base: usize,
) -> Result<Vec<LengthDelimitedField<'a>>, GyroflowCoreError> {
    let mut pos = 0;
    let mut result = Vec::new();
    while pos < data.len() {
        let key =
            read_varint(data, &mut pos).ok_or_else(|| invalid("invalid protobuf field key"))?;
        let field = (key >> 3) as u32;
        let wire = (key & 7) as u8;
        match wire {
            0 => {
                read_varint(data, &mut pos).ok_or_else(|| invalid("invalid protobuf varint"))?;
            }
            1 => {
                pos = pos
                    .checked_add(8)
                    .ok_or_else(|| invalid("protobuf offset overflow"))?;
            }
            2 => {
                let len = read_varint(data, &mut pos)
                    .ok_or_else(|| invalid("invalid protobuf length"))?
                    as usize;
                let end = pos
                    .checked_add(len)
                    .ok_or_else(|| invalid("protobuf length overflow"))?;
                let value = data
                    .get(pos..end)
                    .ok_or_else(|| invalid("protobuf field exceeds sample"))?;
                if field == wanted {
                    let payload = base
                        .checked_add(pos)
                        .ok_or_else(|| invalid("protobuf offset overflow"))?;
                    result.push(LengthDelimitedField {
                        data: value,
                        payload,
                    });
                }
                pos = end;
            }
            5 => {
                pos = pos
                    .checked_add(4)
                    .ok_or_else(|| invalid("protobuf offset overflow"))?;
            }
            _ => return Err(invalid("unsupported protobuf wire type")),
        }
        if pos > data.len() {
            return Err(invalid("protobuf field exceeds sample"));
        }
    }
    Ok(result)
}

fn enabled_varint_field(data: &[u8], wanted: u32) -> Result<bool, GyroflowCoreError> {
    let mut pos = 0;
    while pos < data.len() {
        let key =
            read_varint(data, &mut pos).ok_or_else(|| invalid("invalid protobuf field key"))?;
        let field = (key >> 3) as u32;
        let wire = (key & 7) as u8;
        match wire {
            0 => {
                let value = read_varint(data, &mut pos)
                    .ok_or_else(|| invalid("invalid protobuf varint"))?;
                if field == wanted {
                    return Ok(value != 0);
                }
            }
            1 => {
                pos = pos
                    .checked_add(8)
                    .ok_or_else(|| invalid("protobuf offset overflow"))?;
            }
            2 => {
                let len = read_varint(data, &mut pos)
                    .ok_or_else(|| invalid("invalid protobuf length"))?
                    as usize;
                pos = pos
                    .checked_add(len)
                    .ok_or_else(|| invalid("protobuf length overflow"))?;
            }
            5 => {
                pos = pos
                    .checked_add(4)
                    .ok_or_else(|| invalid("protobuf offset overflow"))?;
            }
            _ => return Err(invalid("unsupported protobuf wire type")),
        }
        if pos > data.len() {
            return Err(invalid("protobuf field exceeds sample"));
        }
    }
    Ok(false)
}

fn quaternion_fields(data: &[u8], base: usize) -> Result<QuaternionFields, GyroflowCoreError> {
    let mut pos = 0;
    let mut fields = [None; 4];
    while pos < data.len() {
        let key =
            read_varint(data, &mut pos).ok_or_else(|| invalid("invalid quaternion field key"))?;
        let field = (key >> 3) as u32;
        let wire = (key & 7) as u8;
        match wire {
            0 => {
                read_varint(data, &mut pos).ok_or_else(|| invalid("invalid quaternion varint"))?;
            }
            1 => {
                pos = pos
                    .checked_add(8)
                    .ok_or_else(|| invalid("quaternion offset overflow"))?;
            }
            2 => {
                let len = read_varint(data, &mut pos)
                    .ok_or_else(|| invalid("invalid quaternion length"))?
                    as usize;
                pos = pos
                    .checked_add(len)
                    .ok_or_else(|| invalid("quaternion length overflow"))?;
            }
            5 if (1..=4).contains(&field) => {
                let payload = base
                    .checked_add(pos)
                    .ok_or_else(|| invalid("quaternion offset overflow"))?;
                pos = pos
                    .checked_add(4)
                    .ok_or_else(|| invalid("quaternion offset overflow"))?;
                fields[(field - 1) as usize] = Some(Fixed32Field { payload });
            }
            5 => {
                pos = pos
                    .checked_add(4)
                    .ok_or_else(|| invalid("quaternion offset overflow"))?;
            }
            _ => return Err(invalid("unsupported quaternion wire type")),
        }
        if pos > data.len() {
            return Err(invalid("quaternion field exceeds sample"));
        }
    }
    Ok(QuaternionFields {
        w: fields[0].ok_or_else(|| invalid("DJI quaternion has no w field"))?,
        x: fields[1].ok_or_else(|| invalid("DJI quaternion has no x field"))?,
        y: fields[2].ok_or_else(|| invalid("DJI quaternion has no y field"))?,
        z: fields[3].ok_or_else(|| invalid("DJI quaternion has no z field"))?,
    })
}

fn collect_quaternions(
    sample: &[u8],
    oq101: bool,
) -> Result<Vec<QuaternionFields>, GyroflowCoreError> {
    let frame = length_delimited_fields(sample, 3, 0)?
        .into_iter()
        .next()
        .ok_or_else(|| invalid("DJI sample has no frame metadata"))?;
    let frame_header = length_delimited_fields(frame.data, 1, frame.payload)?
        .into_iter()
        .next();
    if frame_header
        .map(|header| enabled_varint_field(header.data, 4))
        .transpose()?
        .unwrap_or(false)
    {
        return Err(invalid(
            "DJI metadata uses a check code; refusing to write an invalid checksum",
        ));
    }
    let imu = length_delimited_fields(frame.data, 3, frame.payload)?
        .into_iter()
        .next()
        .ok_or_else(|| invalid("DJI sample has no IMU metadata"))?;
    let attitude = length_delimited_fields(imu.data, 2, imu.payload)?
        .into_iter()
        .next()
        .ok_or_else(|| invalid("DJI sample has no fused attitude"))?;

    let devices = if oq101 {
        length_delimited_fields(attitude.data, 1, attitude.payload)?
    } else {
        vec![attitude]
    };
    let mut result = Vec::new();
    for device in devices {
        for quaternion in length_delimited_fields(device.data, 3, device.payload)? {
            result.push(quaternion_fields(quaternion.data, quaternion.payload)?);
        }
    }
    Ok(result)
}

fn inverse_dji_coordinate_transform(q: &Quat64) -> Quat64 {
    // telemetry-parser converts DJI metadata as:
    //   multiply(raw, (0.5, -0.5, -0.5, 0.5))
    //   multiply((0, 0, 1, 0), converted)
    // Undo those transforms before writing the protobuf fields back.
    let camera =
        UnitQuaternion::from_quaternion(Quaternion::from_parts(0.5, Vector3::new(-0.5, -0.5, 0.5)));
    let horizon =
        UnitQuaternion::from_quaternion(Quaternion::from_parts(0.0, Vector3::new(0.0, 1.0, 0.0)));
    horizon.inverse() * *q * camera.inverse()
}

fn differs(a: &Quat64, b: &Quat64) -> bool {
    let av = a.quaternion();
    let bv = b.quaternion();
    av.coords.dot(&bv.coords).abs() < 1.0 - 1e-12
}

/// Write an edited DJI quaternion stream without re-encoding the MP4.
///
/// Only quaternion f32 payloads whose rotation changed are patched. The
/// metadata sample size stays identical, so video, audio, indexes and all
/// unselected metadata bytes remain byte-for-byte unchanged.
pub fn write_fixed_mp4(
    input_url: &str,
    output_url: &str,
    original: &TimeQuat,
    edited: &TimeQuat,
) -> Result<usize, GyroflowCoreError> {
    if input_url.is_empty() || output_url.is_empty() || input_url == output_url {
        return Err(invalid("source and destination must be distinct files"));
    }
    let matching_keys = original.keys().zip(edited.keys()).all(|(a, b)| a == b);
    if original.is_empty() || original.len() != edited.len() || !matching_keys {
        return Err(invalid(
            "edited quaternion map does not match source metadata",
        ));
    }

    let mut replacements: Vec<Option<Quat64>> = Vec::with_capacity(original.len());
    let mut changed = 0;
    for ((_, source), (_, edited)) in original.iter().zip(edited.iter()) {
        if differs(source, edited) {
            replacements.push(Some(inverse_dji_coordinate_transform(edited)));
            changed += 1;
        } else {
            replacements.push(None);
        }
    }
    if changed == 0 {
        return Err(invalid("no quaternion edits to save"));
    }

    let mut bytes = filesystem::read(input_url)?;
    let file_size = bytes.len();
    let mut cursor = Cursor::new(&bytes);
    let cancel = Arc::new(AtomicBool::new(false));
    let mut metadata_index = 0usize;
    let mut patches: Vec<(usize, [u8; 4])> = Vec::new();
    let mut parse_error: Option<GyroflowCoreError> = None;

    telemetry_parser::util::get_metadata_track_samples(
        &mut cursor,
        file_size,
        false,
        |_, data, file_position, _| {
            if parse_error.is_some() {
                return;
            }
            let oq101 = data
                .get(..64)
                .map(|header| header.windows(5).any(|x| x == b"oq101"))
                .unwrap_or(false);
            match collect_quaternions(data, oq101) {
                Ok(fields) => {
                    for fields in fields {
                        let replacement = replacements.get(metadata_index).and_then(|q| q.as_ref());
                        if replacement.is_some() {
                            let q = replacement.unwrap().quaternion();
                            for (field, value) in [
                                (fields.w, q.w as f32),
                                (fields.x, q.i as f32),
                                (fields.y, q.j as f32),
                                (fields.z, q.k as f32),
                            ] {
                                if field.payload + 4 > data.len() {
                                    parse_error =
                                        Some(invalid("quaternion patch exceeds metadata sample"));
                                    return;
                                }
                                let mut bytes = [0u8; 4];
                                bytes.copy_from_slice(&value.to_le_bytes());
                                patches.push((file_position as usize + field.payload, bytes));
                            }
                        }
                        metadata_index += 1;
                    }
                }
                Err(error) => parse_error = Some(error),
            }
        },
        cancel,
    )
    .map_err(|_| invalid("unable to enumerate DJI metadata samples"))?;

    if let Some(error) = parse_error {
        return Err(error);
    }
    if metadata_index != replacements.len() {
        return Err(invalid(format!(
            "metadata quaternion count mismatch: found {metadata_index}, expected {}",
            replacements.len()
        )));
    }

    for (offset, value) in patches {
        let end = offset
            .checked_add(value.len())
            .ok_or_else(|| invalid("quaternion output offset overflow"))?;
        let target = bytes
            .get_mut(offset..end)
            .ok_or_else(|| invalid("quaternion output offset is outside MP4"))?;
        target.copy_from_slice(&value);
    }
    filesystem::write(output_url, &bytes)?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use nalgebra::{Quaternion, UnitQuaternion, Vector3};

    use super::{collect_quaternions, preset_radius_ms, smooth_quaternion_range};
    use crate::gyro_source::{FileMetadata, GyroSource, TimeQuat};

    fn delimited(tag: u8, payload: Vec<u8>) -> Vec<u8> {
        assert!(payload.len() < 128);
        let mut field = vec![tag, payload.len() as u8];
        field.extend(payload);
        field
    }

    #[test]
    fn quaternion_offsets_are_relative_to_the_metadata_sample() {
        let values = [1.0f32, 2.0, 3.0, 4.0];
        let mut quaternion = Vec::new();
        for (tag, value) in [0x0d, 0x15, 0x1d, 0x25].into_iter().zip(values) {
            quaternion.push(tag);
            quaternion.extend(value.to_le_bytes());
        }
        let sample = delimited(
            0x1a,
            delimited(0x1a, delimited(0x12, delimited(0x1a, quaternion))),
        );

        let fields = collect_quaternions(&sample, false).unwrap()[0];
        for (field, value) in [fields.w, fields.x, fields.y, fields.z]
            .into_iter()
            .zip(values)
        {
            assert_eq!(
                &sample[field.payload..field.payload + 4],
                &value.to_le_bytes()
            );
        }
    }

    #[test]
    fn strength_presets_use_light_recommended_and_strong_radii() {
        assert_eq!(
            (1..=3).map(preset_radius_ms).collect::<Vec<_>>(),
            vec![50.0, 180.0, 300.0]
        );
    }

    #[test]
    fn smoothing_uses_outer_fades_and_preserves_samples_beyond_them() {
        let mut data = TimeQuat::new();
        for index in 0..=100 {
            let quaternion = if index == 20 || index == 50 {
                let rotation = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), 1.0);
                UnitQuaternion::from_quaternion(-rotation.quaternion().clone())
            } else {
                UnitQuaternion::new_normalize(Quaternion::identity())
            };
            data.insert(index * 10_000, quaternion);
        }
        let original = data.clone();

        let changed = smooth_quaternion_range(&mut data, 200_000, 800_000, 1);

        assert_eq!(changed, 77);
        for (timestamp, quaternion) in &data {
            assert!((quaternion.norm() - 1.0).abs() < 1e-12);
            if *timestamp <= 110_000 || *timestamp >= 890_000 {
                assert_eq!(
                    quaternion.quaternion().coords,
                    original[timestamp].quaternion().coords
                );
            }
        }
        assert!(data[&190_000].angle() > original[&190_000].angle());
        assert!(data[&200_000].angle() < original[&200_000].angle());
        assert!(data[&500_000].angle() < original[&500_000].angle());
        assert!(
            data[&500_000]
                .quaternion()
                .coords
                .dot(&original[&500_000].quaternion().coords)
                > 0.0
        );
    }

    #[test]
    fn clear_restores_original_quaternions_and_discards_history() {
        let mut original = TimeQuat::new();
        for index in 0..=100 {
            let angle = if index == 50 { 1.0 } else { 0.0 };
            original.insert(
                index * 10_000,
                UnitQuaternion::from_axis_angle(&Vector3::z_axis(), angle),
            );
        }
        let mut gyro = GyroSource::default();
        gyro.integration_method = 0;
        gyro.quaternions = original.clone();
        gyro.file_metadata = FileMetadata {
            quaternions: original.clone(),
            ..Default::default()
        }
        .into();

        assert!(gyro.smooth_quaternion_range(200.0, 800.0, 1) > 0);
        assert_ne!(gyro.quaternions, original);
        assert!(gyro.restore_original_quaternions());
        assert_eq!(gyro.quaternions, original);
        assert!(gyro.edited_quaternions().is_none());
        assert!(!gyro.undo_quaternion_edit());
        assert!(!gyro.redo_quaternion_edit());
        assert!(!gyro.restore_original_quaternions());
    }
}
