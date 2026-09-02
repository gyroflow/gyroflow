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
    pub iris_tstop: Option<f32>  // T-number
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
    pub sensor_size: (u32, u32),
    pub crop_area: (f32, f32, f32, f32),
    pub pixel_pitch: (u32, u32),
    pub ibis_spline: splines::CatmullRom<nalgebra::Vector3<f64>>,
    pub ois_spline: splines::CatmullRom<nalgebra::Vector3<f64>>
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
    pub mesh_correction:     Vec<(Vec<f64>, Vec<f32>)>,
    /// Runtime-generated local stabilization warp. Kept separate from camera
    /// metadata meshes so optical-only analysis can be replaced or cleared
    /// without destroying embedded lens/ sensor corrections.
    #[serde(skip)]
    pub optical_flow_correction: Vec<(Vec<f64>, Vec<f32>)>,
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
            optical_flow_correction: Default::default(),
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
    pub fn has_per_frame_focal_length(&self) -> bool {
        self.lens_params.values().any(|x| x.focal_length.is_some())
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
    pub fn set_optical_flow_correction(&mut self, v: Vec<(Vec<f64>, Vec<f32>)>) {
        self.0.write().optical_flow_correction = v;
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
