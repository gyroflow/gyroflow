// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use parking_lot::RwLock;
use std::sync::Arc;

use crate::camera_identifier::CameraIdentifier;
use crate::stabilization_params::ReadoutDirection;
use crate::util::MapClosest;
use super::{ TimeIMU, TimeQuat, TimeVec, splines };

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LensParams {
    pub focal_length: Option<f32>, // millimeters
    pub pixel_pitch: Option<(u32, u32)>, // nanometers
    pub sensor_size_px: Option<(u32, u32)>, // pixels
    pub capture_area_origin: Option<(f32, f32)>, // pixels
    pub capture_area_size: Option<(f32, f32)>, // pixels
    // Old projects stored this as a single f32 instead of (fx, fy)
    #[serde(deserialize_with = "deserialize_pixel_focal_length")]
    pub pixel_focal_length: Option<(f32, f32)>, // (fx, fy) pixels
    pub principal_point: Option<(f32, f32)>, // (cx, cy) pixels (output pixel units)
    pub distortion_coefficients: Vec<f64>,
    pub focus_distance: Option<f32>, // meters
    pub iris_fstop: Option<f32>, // f-number
    pub iris_tstop: Option<f32>, // T-number
    pub zoom_ring_position: Option<f32>, // percent of the zoom ring travel
    /// Sensor rows the clip's frame readout time spans, when not the sensor's whole height (`sensor_size_px`, which Sony
    /// and Android report the readout time for): the Gyroflow protobuf's readout time is the picture's, the capture area
    /// it shows (`gyroflow.proto`)
    pub readout_span_rows: Option<f32>,
}
impl LensParams {
    /// Whether the imager geometry is complete enough to be used for the stabilization
    pub fn has_geometry(&self) -> bool {
        self.pixel_pitch.is_some() && self.capture_area_size.is_some() && (self.pixel_focal_length.is_some() || self.focal_length.is_some())
    }
    pub fn has_descriptive_data(&self) -> bool {
        self.focus_distance.is_some() || self.iris_fstop.is_some() || self.iris_tstop.is_some()
    }
    pub fn has_projection_data(&self) -> bool {
        self.pixel_focal_length.is_some()
            || (self.focal_length.is_some() && self.pixel_pitch.is_some() && self.capture_area_size.is_some())
            || !self.distortion_coefficients.is_empty()
    }
    pub fn has_readout_scale(&self) -> bool {
        self.capture_area_size.is_some() && (self.readout_span_rows.is_some() || self.sensor_size_px.is_some())
    }
    /// The part of the clip's readout time the capture area's rows take
    pub fn readout_scale(&self) -> Option<f64> {
        let span = self.readout_span_rows.or(self.sensor_size_px.map(|x| x.1 as f32))?;
        (span > 0.0).then(|| self.capture_area_size.map(|x| x.1 as f64 / span as f64)).flatten()
    }
}

fn deserialize_pixel_focal_length<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<(f32, f32)>, D::Error> {
    use serde::Deserialize;
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Compat { Pair((f32, f32)), Single(f32) }
    Ok(Option::<Compat>::deserialize(d)?.map(|v| match v {
        Compat::Pair(p)   => p,
        Compat::Single(f) => (f, f),
    }))
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CameraStabData {
    pub offset: f64,
    pub ois_offset: Option<f64>,
    pub sensor_size: (u32, u32),
    pub crop_area: (f32, f32, f32, f32),
    pub pixel_pitch: (u32, u32),
    pub ibis_spline: splines::CatmullRom<nalgebra::Vector3<f64>>,
    pub ois_spline: splines::CatmullRom<nalgebra::Vector3<f64>>
}

// Lens breathing of one frame, see gyro_source::sony::breathing. `focal` is the lens's true focal length relative to
// the one its metadata states (the focus position's state against the lens tables' reference), per band of the
// capture area rows, linearly interpolated between the bands, as many as the frame's focus motion during the readout
// needs (a single entry when the rows agree or the frame has no rolling-shutter table). `applied` is the
// magnification the camera's own breathing compensation took out of the recorded picture by cropping it, `None`
// when it didn't: a camera matrix made from the frame's capture area already has that crop in it, a fixed one hasn't
#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BreathingFrame {
    pub focal: Vec<f32>,
    pub applied: Option<f32>,
    pub crop_y: f32,
    pub crop_h: f32,
}
impl BreathingFrame {
    /// `focal` at a sensor row
    pub fn focal_at_row(&self, sensor_row: f64) -> f64 {
        let n = self.focal.len();
        if n < 2 { return self.focal.first().copied().unwrap_or(1.0) as f64; }
        let x = ((sensor_row - self.crop_y as f64) * (n - 1) as f64 / (self.crop_h as f64 - 1.0).max(1.0)).clamp(0.0, (n - 1) as f64);
        let i = (x as usize).min(n - 2);
        let t = x - i as f64;
        self.focal[i] as f64 * (1.0 - t) + self.focal[i + 1] as f64 * t
    }
    /// `focal` at the centre row of the capture area, the frame's own: its camera matrix is scaled by it, and each
    /// row by its ratio to it (`FrameTransform::at_timestamp`)
    pub fn focal_at_centre(&self) -> f64 {
        self.focal_at_row(self.crop_y as f64 + (self.crop_h as f64 - 1.0).max(0.0) / 2.0)
    }
    /// The largest `focal` of the frame's rows
    pub fn focal_max(&self) -> f64 {
        self.focal.iter().copied().fold(f32::MIN, f32::max) as f64
    }
    /// The scale of a camera matrix of the frame for the breathing: `focal` as it is when the matrix was made from the
    /// frame's capture area (`capture_area`, see `FrameTransform::get_lens_data_at_lens_timestamp`), which carries the
    /// crop of the camera's own compensation. A matrix that doesn't (a fixed lens profile) shows the picture the camera
    /// recorded, with that much of the breathing already taken out
    pub fn camera_scale(&self, capture_area: bool) -> f64 {
        if capture_area { 1.0 } else { self.applied.filter(|a| a.is_finite() && *a > 0.0).unwrap_or(1.0) as f64 }
    }
}

/// What `FileMetadata::lens_focal_length_varies`, `has_per_frame_focal_length` and `lens_breathing_varies` found, each
/// the first time it's asked: they scan all of the lens metadata (the breathing every row of every frame), for the
/// renderer per frame and for the UI, the chart and every recompute, and the lens metadata never changes once the file
/// is read (`ReadOnlyFileMetadata`: only the motion data is replaced, in a copy of the rest)
#[derive(Default, Clone, Debug)]
pub struct LensVariation {
    focal_length_mm: std::sync::OnceLock<bool>,
    per_frame_focal_length: std::sync::OnceLock<bool>,
    breathing: std::sync::OnceLock<bool>,
}

#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FileMetadata {
    pub imu_orientation:     Option<String>,
    pub raw_imu:             Vec<TimeIMU>,
    pub quaternions:         TimeQuat,
    pub gravity_vectors:     Option<TimeVec>,
    pub image_orientations:  Option<TimeQuat>,
    pub detected_source:     Option<String>,
    pub frame_readout_time:  Option<f64>,
    pub frame_readout_direction: ReadoutDirection,
    pub frame_rate:          Option<f64>,
    pub camera_identifier:   Option<CameraIdentifier>,
    pub lens_profile:        Option<serde_json::Value>,
    pub lens_positions:      BTreeMap<i64, f64>,
    pub lens_params:         BTreeMap<i64, LensParams>,
    pub digital_zoom:        Option<f64>,
    pub has_accurate_timestamps: bool,
    pub additional_data:     serde_json::Value,
    // The per-frame tables below (and `MeshCorrections::frames`) have entry k for video frame k, every frame up to the
    // last one the metadata describes: a frame without data of its own has its neighbours' (`gyro_source::per_frame`).
    // They're read through `frame_time_offset`, `camera_stab_at`, `lens_breathing_at` and `MeshCorrections::frame`
    /// Offset of each frame's capture (the middle of its exposure and readout, in the motion data's time) from its time
    /// in the video, in ms
    pub per_frame_time_offsets: Vec<f64>,
    pub camera_stab_data:    Vec<CameraStabData>,
    #[serde(rename = "mesh_corrections")]
    pub mesh_correction:     MeshCorrections,
    /// What older project files stored instead: one `(forward, inverse)` buffer pair per frame. Read only, and folded
    /// into `mesh_correction` on load (`sony::upgrade_legacy_mesh_buffers`)
    #[serde(rename = "mesh_correction", skip_serializing)]
    pub legacy_mesh_correction: Vec<(Vec<f64>, Vec<f32>)>,
    /// The residual pass' per-frame mesh, built from the optical measurements when the user turns it on. Kept here
    /// because the render path reads it beside `mesh_correction` (see `frame_transform`), but never serialized: a
    /// project keeps the correction and the settings, and rebuilds the mesh from the measurements in milliseconds
    #[serde(skip)]
    pub residual_mesh: Option<crate::synchronization::optical_motion::residual::ResidualMesh>,
    pub lens_breathing:      Vec<BreathingFrame>,
    /// What the lens metadata's checks for a focal length that changes found (`LensVariation`)
    #[serde(skip)]
    pub lens_variation:      LensVariation,
    /// The segments of a video cut from camera recordings, from its segment table (`TrimSegments`): every segment
    /// boundary is a hard boundary for the motion data
    pub trim_segments:       Option<super::TrimSegments>,
}
impl FileMetadata {
    pub fn thin(&self) -> Self {
        Self {
            imu_orientation:         self.imu_orientation.clone(),
            raw_imu:                 Default::default(),
            quaternions:             Default::default(),
            gravity_vectors:         Default::default(),
            image_orientations:      Default::default(),
            detected_source:         self.detected_source.clone(),
            frame_readout_time:      self.frame_readout_time.clone(),
            frame_readout_direction: self.frame_readout_direction.clone(),
            frame_rate:              self.frame_rate.clone(),
            camera_identifier:       self.camera_identifier.clone(),
            lens_profile:            self.lens_profile.clone(),
            lens_positions:          Default::default(),
            lens_params:             Default::default(),
            digital_zoom:            self.digital_zoom.clone(),
            has_accurate_timestamps: self.has_accurate_timestamps.clone(),
            additional_data:         self.additional_data.clone(),
            per_frame_time_offsets:  Default::default(),
            camera_stab_data:        Default::default(),
            mesh_correction:         Default::default(),
            legacy_mesh_correction:  Default::default(),
            residual_mesh:           Default::default(),
            lens_breathing:          Default::default(),
            lens_variation:          Default::default(),
            trim_segments:           self.trim_segments.clone(),
        }
    }
    pub fn has_motion(&self) -> bool {
        !self.raw_imu.is_empty() || !self.quaternions.is_empty()
    }
    /// Number of `lens_params` samples that feed the projection. The map also holds entries with
    /// nothing but the descriptive values, for cameras that report those but no geometry at all
    pub fn lens_geometry_count(&self) -> usize {
        self.lens_params.values().filter(|x| x.has_projection_data()).count()
    }
    /// Whether any frame has a mesh or focal plane correction
    pub fn has_mesh_correction(&self) -> bool {
        !self.mesh_correction.is_empty()
    }
    /// Offset of a frame's capture from its time in the video (`per_frame_time_offsets`), 0 without the table
    pub fn frame_time_offset(&self, frame: usize) -> f64 {
        per_frame_entry(&self.per_frame_time_offsets, frame).copied().unwrap_or(0.0)
    }
    /// The in-camera stabilizer's sensor and lens shift of a frame (`camera_stab_data`)
    pub fn camera_stab_at(&self, frame: usize) -> Option<&CameraStabData> {
        per_frame_entry(&self.camera_stab_data, frame)
    }
    /// The lens breathing of a frame (`lens_breathing`), `None` where it has none
    pub fn lens_breathing_at(&self, frame: usize) -> Option<&BreathingFrame> {
        per_frame_entry(&self.lens_breathing, frame).filter(|b| !b.focal.is_empty())
    }
    /// Whether the effective focal length changes over the clip: the metadata's (`has_per_frame_focal_length`), or the
    /// lens's own through its focus breathing (`lens_breathing_varies`)
    pub fn has_varying_focal_length(&self) -> bool {
        self.has_per_frame_focal_length() || self.lens_breathing_varies()
    }
    /// Whether the lens breathing changes over the clip: the focus moves (the `focal` of any row of any frame, a frame
    /// without any taken as 1, as the focal length sweep takes it), or the crop of the camera's own compensation does
    /// (`applied`). A clip that keeps its focus breathes by as much all through, a constant scale of its focal length:
    /// nothing changes over it, and any one frame's lens is every frame's. Cached (`LensVariation`)
    pub fn lens_breathing_varies(&self) -> bool {
        *self.lens_variation.breathing.get_or_init(|| {
            Self::varies(&mut self.lens_breathing.iter().flat_map(|b| b.focal.iter().map(|f| *f as f64).chain(b.focal.is_empty().then_some(1.0))))
                || Self::varies(&mut self.lens_breathing.iter().filter(|b| !b.focal.is_empty()).map(|b| b.camera_scale(false)))
        })
    }
    /// More than 0.2% between the smallest and the largest value
    fn varies(it: &mut dyn Iterator<Item = f64>) -> bool {
        let (mut min, mut max, mut count) = (f64::MAX, 0.0f64, 0usize);
        for f in it.filter(|f| f.is_finite() && *f > 0.0) {
            min = min.min(f);
            max = max.max(f);
            count += 1;
        }
        count > 1 && max > min * 1.002
    }
    /// Whether the lens metadata reports a focal length in millimetres that changes over the clip (a zoom lens
    /// on a body that records it: Blackmagic, RED, Nikon, Z CAM, Sony). Cached (`LensVariation`), the projection asks
    /// per frame, see `FrameTransform::get_lens_data_at_timestamp`
    pub fn lens_focal_length_varies(&self) -> bool {
        *self.lens_variation.focal_length_mm.get_or_init(|| Self::varies(&mut self.lens_params.values().filter_map(|x| x.focal_length.map(|f| f as f64))))
    }
    /// Whether the lens metadata describes a focal length that changes over the clip (zoom lens, dynamic
    /// crop, interpolated lens profiles). A fixed lens reports the same value every frame and has nothing
    /// to stabilize or to chart. The pixel and the millimetre values are compared separately: an entry may
    /// carry one without the other, and they're not in the same unit. Cached (`LensVariation`)
    pub fn has_per_frame_focal_length(&self) -> bool {
        *self.lens_variation.per_frame_focal_length.get_or_init(|| {
            Self::varies(&mut self.lens_params.values().filter_map(|x| x.pixel_focal_length.map(|(fx, fy)| (fx as f64 * fy as f64).sqrt())))
                || self.lens_focal_length_varies()
                || Self::varies(&mut self.lens_positions.values().copied())
        })
    }
    pub fn lens_params_closest(&self, timestamp_us: i64, max_diff: i64, pred: impl Fn(&LensParams) -> bool) -> Option<&LensParams> {
        self.lens_params_closest_in(i64::MIN..=i64::MAX, timestamp_us, max_diff, pred)
    }
    /// The keys (µs) of the lens metadata frame `frame` takes its own from: in a video cut from recordings its part's
    /// (`TrimSegments::lens_keys_of_frame`), as the same frames trimmed on their own have only theirs; all of them
    /// otherwise
    pub fn lens_keys_of_frame(&self, frame: usize) -> RangeInclusive<i64> {
        self.trim_segments.as_ref().map_or(i64::MIN..=i64::MAX, |t| t.lens_keys_of_frame(frame))
    }
    /// `lens_params_closest` among the entries of the keys `keys` (`lens_keys_of_frame`), `MapClosest::get_closest_in`
    pub fn lens_params_closest_in(&self, keys: RangeInclusive<i64>, timestamp_us: i64, max_diff: i64, pred: impl Fn(&LensParams) -> bool) -> Option<&LensParams> {
        self.lens_params.get_closest_in(keys, timestamp_us, max_diff, pred)
    }
    /// The lens position (`lens_positions`) closest to `timestamp_us`, less than `max_diff` away, among the entries of
    /// the keys `keys` (`lens_keys_of_frame`), as `MapClosest::get_closest` finds it among all of them
    pub fn lens_position_closest_in(&self, keys: RangeInclusive<i64>, timestamp_us: i64, max_diff: i64) -> Option<f64> {
        self.lens_positions.get_closest_in(keys, timestamp_us, max_diff, |_| true).copied()
    }
}

/// Entry `frame` of a per-frame table. The readers ask for every frame of the video, which can go on past the last
/// frame the metadata describes (a camera that stops writing it a frame or two early, a video longer than the motion
/// file): those frames continue the last one, like a frame without metadata inside the table continues its
/// neighbours'. `None` for an empty table
pub fn per_frame_entry<T>(table: &[T], frame: usize) -> Option<&T> {
    table.get(frame).or_else(|| table.last())
}

// ------------- Mesh correction -------------

/// Length of the header every mesh block starts with: `[block length, divisions x, y, mesh size x, y, capture area
/// origin x, y, size x, y]`, followed by the grid positions (x, y per node, row by row) and the row spline coefficients
/// (`splines::BivariateSpline`, a, b, c, d per row, for x then for y). The kernels read `[0]` as the offset of the
/// focal plane table that follows the block, and a block longer than the header as a mesh
pub const MESH_HEADER: usize = 9;

/// Sony's per-frame distortion correction, from the `MeshCorrection` and `FocalPlaneDistortion` metadata (built by
/// `gyro_source::sony::get_mesh_correction`). The mesh depends on the lens state alone, so every frame the camera wrote
/// the same one for shares a single table; what differs from frame to frame, the capture area (which moves with the
/// in-camera stabilization) and the focal plane table, is kept per frame and folded into the buffers on request.
/// Project files with embedded metadata store the same thing, tables once and a few values per frame
#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MeshCorrections {
    pub tables: Vec<MeshTable>,
    /// One per frame, `MeshFrame::is_empty` where the frame has none; the whole vector is empty when no frame has any
    pub frames: Vec<MeshFrame>,
}

/// One distortion mesh, in the block layout described at [`MESH_HEADER`]. The capture area slots of the headers hold
/// zeros, the frame supplies them
#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MeshTable {
    /// The camera's mesh, what the CPU point path (`undistort_points`) applies
    pub forward: Vec<f64>,
    /// Its numeric inverse on the same grid, what the kernels look up
    pub inverse: Vec<f32>,
    /// `forward` for the kernels, which refine every inverse lookup against it; empty when the inverse alone is
    /// accurate enough (`sony::MESH_REFINE_THRESHOLD_PX`), which spares two spline evaluations per pixel
    pub refinement: Vec<f32>,
}

/// The correction of one frame
#[derive(Default, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MeshFrame {
    /// Index into `MeshCorrections::tables`, `None` when the frame has no mesh
    pub table: Option<u32>,
    /// Extent of the mesh coordinates (the sensor), in sensor pixels
    pub mesh_size: (f64, f64),
    /// Capture area within the sensor, in sensor pixels: the video maps onto it
    pub crop_origin: (f64, f64),
    pub crop_size: (f64, f64),
    /// Focal plane distortion table `[band count, unk1, band height, scale, count × (x, y)]`, empty when the frame has none
    pub focal_plane: Vec<f64>,
}
impl MeshFrame {
    pub fn is_empty(&self) -> bool {
        self.table.is_none() && !self.has_focal_plane()
    }
    pub fn has_focal_plane(&self) -> bool {
        self.focal_plane.first().map_or(false, |count| *count > 0.0)
    }
}

impl MeshCorrections {
    /// Whether no frame has any correction
    pub fn is_empty(&self) -> bool {
        !self.frames.iter().any(|f| !f.is_empty())
    }
    /// No corrections, and none of the storage they took
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    /// The correction of a frame, `None` when it has none
    pub fn frame(&self, frame: usize) -> Option<&MeshFrame> {
        per_frame_entry(&self.frames, frame).filter(|f| !f.is_empty())
    }
    pub fn has_mesh(&self, frame: usize) -> bool {
        self.frame(frame).map_or(false, |f| f.table.is_some())
    }
    pub fn has_focal_plane(&self, frame: usize) -> bool {
        self.frame(frame).map_or(false, |f| f.has_focal_plane())
    }
    fn table_of(&self, f: &MeshFrame) -> Option<&MeshTable> {
        self.tables.get(f.table? as usize)
    }
    /// Adds a table unless an equal one (by `key`) is already there, and returns its index. `cache` maps the keys of
    /// the tables added so far
    pub fn intern(&mut self, key: u32, cache: &mut BTreeMap<u32, u32>, build: impl FnOnce() -> MeshTable) -> u32 {
        *cache.entry(key).or_insert_with(|| {
            self.tables.push(build());
            (self.tables.len() - 1) as u32
        })
    }

    /// Appends a mesh block with the frame's geometry in its header, a bare header when the frame has no mesh
    fn push_block<T: Copy>(buf: &mut Vec<T>, block: &[T], f: &MeshFrame, conv: impl Fn(f64) -> T) {
        let start = buf.len();
        if block.len() >= MESH_HEADER {
            buf.extend_from_slice(block);
        } else {
            buf.push(conv(MESH_HEADER as f64));
            buf.extend(std::iter::repeat(conv(0.0)).take(MESH_HEADER - 1));
        }
        let geometry = [f.mesh_size.0, f.mesh_size.1, f.crop_origin.0, f.crop_origin.1, f.crop_size.0, f.crop_size.1];
        for (slot, v) in buf[start + 3..start + MESH_HEADER].iter_mut().zip(geometry) { *slot = conv(v); }
    }
    /// The focal plane table, or the 4-value header with a zero count the kernels probe when the frame has none
    fn push_focal_plane<T: Copy>(buf: &mut Vec<T>, f: &MeshFrame, conv: impl Fn(f64) -> T) {
        if f.focal_plane.len() >= 4 {
            buf.extend(f.focal_plane.iter().map(|v| conv(*v)));
        } else {
            buf.extend(std::iter::repeat(conv(0.0)).take(4));
        }
    }

    /// The kernels' buffer of a frame: `[inverse mesh][focal plane table][forward mesh]`, the forward mesh reduced to a
    /// single zero when the table doesn't need refining. The kernels probe that slot, and the GPU buffers keep stale
    /// data past what was uploaded, so it's always there. Empty when the frame has no correction
    pub fn kernel_buffer(&self, frame: usize) -> Vec<f32> {
        let Some(f) = self.frame(frame) else { return Vec::new() };
        let table = self.table_of(f);
        let inverse: &[f32] = table.map_or(&[], |t| t.inverse.as_slice());
        let refinement: &[f32] = table.map_or(&[], |t| t.refinement.as_slice());
        let mut buf = Vec::with_capacity(inverse.len().max(MESH_HEADER) + f.focal_plane.len().max(4) + refinement.len().max(1));
        Self::push_block(&mut buf, inverse, f, |v| v as f32);
        Self::push_focal_plane(&mut buf, f, |v| v as f32);
        if refinement.is_empty() {
            buf.push(0.0);
        } else {
            Self::push_block(&mut buf, refinement, f, |v| v as f32);
        }
        buf
    }
    /// The camera's mesh of a frame with the frame's geometry in its header, followed by the focal plane table: what
    /// `undistort_points` applies. `None` when the frame has no correction
    pub fn forward_mesh(&self, frame: usize) -> Option<Vec<f64>> {
        let f = self.frame(frame)?;
        let forward: &[f64] = self.table_of(f).map_or(&[], |t| t.forward.as_slice());
        let mut buf = Vec::with_capacity(forward.len().max(MESH_HEADER) + f.focal_plane.len().max(4));
        Self::push_block(&mut buf, forward, f, |v| v);
        Self::push_focal_plane(&mut buf, f, |v| v);
        Some(buf)
    }

    /// Project files of older versions stored one buffer pair per frame: the camera's mesh as `[mesh block][focal plane
    /// table]` and the kernels' buffer as `[inverse block][focal plane table][forward block, in later versions]`, both
    /// with the frame's capture area in their headers. The frames whose mesh is the same share one table again. A pair
    /// that ends before the slots the kernels probe (the oldest files, a one-value placeholder of an intermediate build)
    /// is no correction, and a truncated focal plane table is dropped rather than read past
    pub fn from_legacy(frames: Vec<(Vec<f64>, Vec<f32>)>) -> Self {
        let mut out = Self::default();
        let mut cache = BTreeMap::new();
        for (forward, inverse) in frames {
            let frame = Self::legacy_frame(&forward, &inverse, &mut out, &mut cache).unwrap_or_default();
            out.frames.push(frame);
        }
        if out.is_empty() { out.clear(); }
        out
    }
    fn legacy_frame(fwd: &[f64], inv: &[f32], out: &mut Self, cache: &mut BTreeMap<u32, u32>) -> Option<MeshFrame> {
        let block_len = |first: f64, len: usize| { let o = first as usize; (first >= MESH_HEADER as f64 && o <= len).then_some(o) };
        let of = block_len(*fwd.first()?, fwd.len())?;
        let oi = block_len(*inv.first()? as f64, inv.len())?;
        let focal_plane: Vec<f64> = match fwd.get(of) {
            Some(count) if *count > 0.0 => {
                let n = 4 + 2 * (*count as usize);
                if fwd.len() >= of + n { fwd[of..of + n].to_vec() } else {
                    log::warn!("Truncated focal plane table in a project file, dropped");
                    Vec::new()
                }
            }
            _ => Vec::new()
        };
        let table = if of > MESH_HEADER && oi > MESH_HEADER {
            // The capture area is the frame's, not the table's
            let mut forward = fwd[..of].to_vec();
            for v in &mut forward[5..MESH_HEADER] { *v = 0.0; }
            let mut inverse = inv[..oi].to_vec();
            for v in &mut inverse[5..MESH_HEADER] { *v = 0.0; }
            // The forward block after the focal plane table, when the file has one
            let count = inv.get(oi).copied().unwrap_or(0.0).max(0.0) as usize;
            let at = oi + 4 + 2 * count;
            let refinement = match inv.get(at) {
                Some(len) if *len > MESH_HEADER as f32 && at + *len as usize <= inv.len() => {
                    let mut r = inv[at..at + *len as usize].to_vec();
                    for v in &mut r[5..MESH_HEADER] { *v = 0.0; }
                    r
                }
                _ => Vec::new()
            };
            let mut hasher = crc32fast::Hasher::new();
            for v in &forward { hasher.update(&v.to_bits().to_le_bytes()); }
            hasher.update(&[refinement.is_empty() as u8]);
            let key = hasher.finalize();
            Some(out.intern(key, cache, || MeshTable { forward, inverse, refinement }))
        } else {
            None
        };
        Some(MeshFrame {
            table,
            mesh_size: (fwd[3], fwd[4]),
            crop_origin: (fwd[5], fwd[6]),
            crop_size: (fwd[7], fwd[8]),
            focal_plane,
        })
    }
}

// ------------- ReadOnlyFileMetadata -------------
/// The motion data of a file, taken out of its metadata while it's ignored, see `GyroSource::set_ignore_file_motion`
#[derive(Default, Clone, Debug)]
pub struct FileMotion {
    pub raw_imu:            Vec<TimeIMU>,
    pub quaternions:        TimeQuat,
    pub gravity_vectors:    Option<TimeVec>,
    pub image_orientations: Option<TimeQuat>,
}
impl FileMotion {
    fn put_into(self, md: &mut FileMetadata) {
        md.raw_imu = self.raw_imu;
        md.quaternions = self.quaternions;
        md.gravity_vectors = self.gravity_vectors;
        md.image_orientations = self.image_orientations;
    }
}

// Make a thread-safe read-only wrapper for FileMetadata, because once it's read, it's never changed
#[derive(Clone)]
pub struct ReadOnlyFileMetadata(pub Arc<RwLock<FileMetadata>>);
impl Default for ReadOnlyFileMetadata {
    fn default() -> Self {
        Self(Arc::new(RwLock::new(Default::default())))
    }
}
impl From<FileMetadata> for ReadOnlyFileMetadata {
    fn from(v: FileMetadata) -> Self {
        Self(Arc::new(RwLock::new(v)))
    }
}
impl ReadOnlyFileMetadata {
    pub fn read(&self) -> parking_lot::RwLockReadGuard<'_, FileMetadata> {
        self.0.read()
    }
    /// Changes the metadata into a copy of its own, never in place: every clone of a `GyroSource` shares the one it
    /// was cloned with (a render queue job's, a running analysis's), and a change is only ever meant for this one
    fn modify(&mut self, f: impl FnOnce(&mut FileMetadata)) {
        let mut md = self.0.read().clone();
        f(&mut md);
        self.0 = Arc::new(RwLock::new(md));
    }
    pub fn set_raw_imu(&mut self, v: Vec<TimeIMU>) {
        self.modify(|md| md.raw_imu = v);
    }
    /// Takes the motion data out: from then on it's metadata of a file without any
    pub fn take_motion(&mut self) -> FileMotion {
        let mut motion = FileMotion::default();
        self.modify(|md| motion = FileMotion {
            raw_imu:            std::mem::take(&mut md.raw_imu),
            quaternions:        std::mem::take(&mut md.quaternions),
            gravity_vectors:    md.gravity_vectors.take(),
            image_orientations: md.image_orientations.take(),
        });
        motion
    }
    /// Puts back what `take_motion` took out
    pub fn restore_motion(&mut self, motion: FileMotion) {
        self.modify(|md| motion.put_into(md));
    }
    /// A copy of the metadata with `motion` back in
    pub fn with_motion(&self, motion: &FileMotion) -> FileMetadata {
        let mut md = self.0.read().clone();
        motion.clone().put_into(&mut md);
        md
    }
    /// The residual pass' per-frame mesh, see `FileMetadata::residual_mesh`. Never serialized with the file
    pub fn set_residual_mesh(&mut self, v: Option<crate::synchronization::optical_motion::residual::ResidualMesh>) {
        self.modify(|md| md.residual_mesh = v);
    }
}
impl serde::Serialize for ReadOnlyFileMetadata {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: serde::Serializer {
        self.0.read().serialize(serializer)
    }
}
impl<'de> serde::Deserialize<'de> for ReadOnlyFileMetadata {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: serde::Deserializer<'de> {
        Ok(Self(Arc::new(RwLock::new(FileMetadata::deserialize(deserializer)?))))
    }
}
// ------------- ReadOnlyFileMetadata -------------

#[cfg(test)]
mod tests {
    use super::*;

    fn breathing(frames: &[&[f32]]) -> FileMetadata {
        FileMetadata { lens_breathing: frames.iter().map(|f| BreathingFrame { focal: f.to_vec(), applied: None, crop_y: 0.0, crop_h: 100.0 }).collect(), ..Default::default() }
    }

    #[test]
    fn a_clip_that_keeps_its_focus_breathes_by_as_much_all_through() {
        // A lens breathing 4%, its focus held: its focal length is a constant scale, nothing that changes over the clip
        let held = breathing(&[&[1.04, 1.04], &[1.04, 1.04], &[1.04, 1.04]]);
        assert!(!held.lens_breathing_varies() && !held.has_varying_focal_length());
        // Focused through the readout of a frame, or from one frame to the next
        assert!(breathing(&[&[1.04, 1.04], &[1.04, 1.02], &[1.02, 1.02]]).has_varying_focal_length());
        // A frame without breathing is the lens as it's stated, and the frames around it aren't
        assert!(breathing(&[&[1.04, 1.04], &[], &[1.04, 1.04]]).lens_breathing_varies());
        // The camera's own compensation cropping more as the focus moves
        let mut cropped = breathing(&[&[1.04], &[1.04]]);
        cropped.lens_breathing[1].applied = Some(1.03);
        assert!(cropped.lens_breathing_varies());
    }
}
