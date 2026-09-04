// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>

use std::collections::BTreeMap;
use parking_lot::RwLock;
use std::sync::Arc;

use crate::camera_identifier::CameraIdentifier;
use crate::stabilization_params::ReadoutDirection;
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
    pub zoom_ring_position: Option<f32> // percent of the zoom ring travel
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
        self.capture_area_size.is_some() && self.sensor_size_px.is_some()
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

// Lens breathing compensation of one frame: output zoom per band of the capture area rows, linearly interpolated
// between the bands, as many as the frame's focus motion during the readout needs (a single entry when the rows
// agree or the frame has no rolling-shutter table), see gyro_source::sony::breathing
#[derive(Default, Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BreathingFrame {
    pub scale: Vec<f32>,
    pub crop_y: f32,
    pub crop_h: f32,
}
impl BreathingFrame {
    pub fn scale_at_row(&self, sensor_row: f64) -> f64 {
        let n = self.scale.len();
        if n < 2 { return self.scale.first().copied().unwrap_or(1.0) as f64; }
        let x = ((sensor_row - self.crop_y as f64) * (n - 1) as f64 / (self.crop_h as f64 - 1.0).max(1.0)).clamp(0.0, (n - 1) as f64);
        let i = (x as usize).min(n - 2);
        let t = x - i as f64;
        self.scale[i] as f64 * (1.0 - t) + self.scale[i + 1] as f64 * t
    }
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
    pub per_frame_time_offsets: Vec<f64>,
    pub camera_stab_data:    Vec<CameraStabData>,
    #[serde(rename = "mesh_corrections")]
    pub mesh_correction:     MeshCorrections,
    /// What older project files stored instead: one `(forward, inverse)` buffer pair per frame. Read only, and folded
    /// into `mesh_correction` on load (`sony::upgrade_legacy_mesh_buffers`)
    #[serde(rename = "mesh_correction", skip_serializing)]
    pub legacy_mesh_correction: Vec<(Vec<f64>, Vec<f32>)>,
    pub lens_breathing:      Vec<BreathingFrame>,
    /// Cache of `lens_focal_length_varies`, the renderer asks per frame
    #[serde(skip)]
    pub focal_length_varies_cache: std::sync::OnceLock<bool>,
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
            lens_breathing:          Default::default(),
            focal_length_varies_cache: Default::default(),
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
    /// on a body that records it: Blackmagic, RED, Nikon, Z CAM, Sony). Cached, the projection asks per frame,
    /// see `FrameTransform::get_lens_data_at_timestamp`
    pub fn lens_focal_length_varies(&self) -> bool {
        *self.focal_length_varies_cache.get_or_init(|| Self::varies(&mut self.lens_params.values().filter_map(|x| x.focal_length.map(|f| f as f64))))
    }
    /// Whether the lens metadata describes a focal length that changes over the clip (zoom lens, dynamic
    /// crop, interpolated lens profiles). A fixed lens reports the same value every frame and has nothing
    /// to stabilize or to chart. The pixel and the millimetre values are compared separately: an entry may
    /// carry one without the other, and they're not in the same unit
    pub fn has_per_frame_focal_length(&self) -> bool {
        Self::varies(&mut self.lens_params.values().filter_map(|x| x.pixel_focal_length.map(|(fx, fy)| (fx as f64 * fy as f64).sqrt())))
            || self.lens_focal_length_varies()
            || Self::varies(&mut self.lens_positions.values().copied())
    }
    pub fn lens_params_closest(&self, timestamp_us: i64, max_diff: i64, pred: impl Fn(&LensParams) -> bool) -> Option<&LensParams> {
        let max_diff = max_diff.max(0);
        let min_ts = timestamp_us.saturating_sub(max_diff);
        let max_ts = timestamp_us.saturating_add(max_diff);

        // The two ranges overlap on an exact key hit; the tie-break below then picks that same entry
        let before = self.lens_params.range(min_ts..=timestamp_us).rev().find(|(_, v)| pred(*v));
        let after  = self.lens_params.range(timestamp_us..=max_ts) .find(|(_, v)| pred(*v));

        // `abs_diff` returns an u64 and can't overflow, unlike `(key - other).abs()`
        let closest = match (before, after) {
            (Some(before), Some(after)) => if timestamp_us.abs_diff(*after.0) <= timestamp_us.abs_diff(*before.0) { after } else { before },
            (Some(before), None)        => before,
            (None,         Some(after)) => after,
            (None,         None)        => return None
        };
        // The ranges above are inclusive on both ends, so an entry exactly `max_diff` away still needs rejecting.
        // Anything farther than the closest matching entry is out of range as well, so one check is enough
        if timestamp_us.abs_diff(*closest.0) < max_diff as u64 {
            Some(closest.1)
        } else {
            None
        }
    }
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
    pub fn clear(&mut self) {
        self.tables.clear();
        self.frames.clear();
    }
    /// The correction of a frame, `None` when it has none
    pub fn frame(&self, frame: usize) -> Option<&MeshFrame> {
        self.frames.get(frame).filter(|f| !f.is_empty())
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
    pub fn set_raw_imu(&mut self, v: Vec<TimeIMU>) {
        self.0.write().raw_imu = v;
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
