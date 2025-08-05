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
    pub pixel_focal_length: Option<f32>, // pixels
    pub distortion_coefficients: Vec<f64>,
    pub focus_distance: Option<f32>
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
        }
    }
    pub fn has_motion(&self) -> bool {
        !self.raw_imu.is_empty() || !self.quaternions.is_empty()
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
