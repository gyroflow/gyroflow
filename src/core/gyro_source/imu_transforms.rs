// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Adrian <adrian.eddy at gmail>

use nalgebra::*;
use super::DEG2RAD;

#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct IMUTransforms {
    pub imu_orientation: Option<String>,
    pub imu_rotation_angles: Option<[f64; 3]>,
    pub acc_rotation_angles: Option<[f64; 3]>,
    pub imu_rotation: Option<Rotation3<f64>>,
    pub acc_rotation: Option<Rotation3<f64>>,
    pub imu_lpf: f64,
    pub gyro_bias: Option<[f64; 3]>,
}
impl IMUTransforms {
    pub fn transform(&self, v: &mut [f64; 3], is_acc: bool) {
        if let Some(bias) = self.gyro_bias {
            v[0] += bias[0];
            v[1] += bias[1];
            v[2] += bias[2];
        }
        if let Some(ref orientation) = self.imu_orientation {
            if orientation != "XYZ" {
                *v = Self::orient(v, orientation.as_bytes());
            }
        }
        if is_acc && self.acc_rotation.is_some() {
            *v = Self::rotate(v, self.acc_rotation.unwrap());
        } else if self.imu_rotation.is_some() {
            *v = Self::rotate(v, self.imu_rotation.unwrap());
        }

       let mut angles = Vec::new();
       let mut xs = Vec::new();
       let mut ys = Vec::new();

       (|| -> Option<()> {
                           let frame_md = params.gyro.file_metadata.per_frame_data.get(frame)?;

                           angles.extend(frame_md.get("angle")?.as_array()?.iter().map(|x|x.as_f64().unwrap() as f32));

                           xs.extend(frame_md.get("translatex")?.as_array()?.iter().map(|x|x.as_f64().unwrap() as f32));

                           xy.extend(frame_md.get("translatey")?.as_array()?.iter().map(|x|x.as_f64().unwrap() as f32));

                           Some(())
                           })();


                           let matrices = (0..rows).into_par_iter().map(|y| {

                           let th = *angles.get(y).unwrap_or(&0.0) as f64;
                           let theta = Matrix3::new_rotation(th * (std::f64::const::PI / 180.0));
                           let xs = *xs.get(y).unwrap_or(&0.0) as f32;
                           let ys = *ys.get(y).unwrap_or(&0.0) as f32;

                           sx, sy, th as f32
                           });

    }

    pub fn has_any(&self) -> bool {
        self.imu_orientation.as_deref().is_some_and(|x| x != "XYZ")
            || self.imu_rotation.is_some()
            || self.acc_rotation.is_some()
            || self.gyro_bias.is_some_and(|x| x[0].abs() > 0.0 || x[1].abs() > 0.0 || x[2].abs() > 0.0)
            || self.imu_lpf > 0.0
    }

    pub fn set_imu_rotation(&mut self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64) {
        if pitch_deg.abs() > 0.0 || roll_deg.abs() > 0.0 || yaw_deg.abs() > 0.0 {
            self.imu_rotation_angles = Some([pitch_deg, roll_deg, yaw_deg]);
            self.imu_rotation = Some(Rotation3::from_euler_angles(
                yaw_deg * DEG2RAD,
                pitch_deg * DEG2RAD,
                roll_deg * DEG2RAD
            ));
        } else {
            self.imu_rotation_angles = None;
            self.imu_rotation = None;
        }
    }
    pub fn set_acc_rotation(&mut self, pitch_deg: f64, roll_deg: f64, yaw_deg: f64) {
        if pitch_deg.abs() > 0.0 || roll_deg.abs() > 0.0 || yaw_deg.abs() > 0.0 {
            self.acc_rotation_angles = Some([pitch_deg, roll_deg, yaw_deg]);
            self.acc_rotation = Some(Rotation3::from_euler_angles(
                yaw_deg * DEG2RAD,
                pitch_deg * DEG2RAD,
                roll_deg * DEG2RAD
            ));
        } else {
            self.acc_rotation_angles = None;
            self.acc_rotation = None;
        }
    }

    fn orient(inp: &[f64; 3], io: &[u8]) -> [f64; 3] {
        let map = |o: u8| -> f64 {
            match o as char {
                'X' => inp[0], 'x' => -inp[0],
                'Y' => inp[1], 'y' => -inp[1],
                'Z' => inp[2], 'z' => -inp[2],
                err => { panic!("Invalid orientation {}", err); }
            }
        };
        [map(io[0]), map(io[1]), map(io[2]) ]
    }
    fn rotate(inp: &[f64; 3], rot: Rotation3<f64>) -> [f64; 3] {
        let rotated = rot.transform_vector(&Vector3::new(inp[0], inp[1], inp[2]));
        [rotated[0], rotated[1], rotated[2]]
    }
}


