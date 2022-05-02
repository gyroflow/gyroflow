// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Elvin Chen

use super::*;
use nalgebra::*;
use crate::gyro_source::TimeQuat;


pub fn from_euler_yxz(x: f64, y: f64, z: f64) -> UnitQuaternion<f64> {

    let x_axis = nalgebra::Vector3::<f64>::x_axis();
    let y_axis = nalgebra::Vector3::<f64>::y_axis();
    let z_axis = nalgebra::Vector3::<f64>::z_axis();
    
    let rot_x = Rotation3::from_axis_angle(&x_axis, x);
    let rot_y = Rotation3::from_axis_angle(&y_axis, y + std::f64::consts::FRAC_PI_2);
    let rot_z = Rotation3::from_axis_angle(&z_axis, z);

    let correction = Rotation3::from_axis_angle(&z_axis, std::f64::consts::FRAC_PI_2) * Rotation3::from_axis_angle(&y_axis, std::f64::consts::FRAC_PI_2);

    let combined_rot = rot_z * rot_x * rot_y * correction;
    UnitQuaternion::from_rotation_matrix(&combined_rot)
}

pub fn lock_horizon_angle(q: &UnitQuaternion<f64>, roll_correction: f64) -> UnitQuaternion<f64> {
    // z axis points in view direction, use as reference
    let axis = nalgebra::Vector3::<f64>::y_axis();

    // let x_axis = nalgebra::Vector3::<f64>::x_axis();
    let y_axis = nalgebra::Vector3::<f64>::y_axis();
    let z_axis = nalgebra::Vector3::<f64>::z_axis();

    let corrected_transform = q.to_rotation_matrix() * Rotation3::from_axis_angle(&y_axis, -std::f64::consts::FRAC_PI_2) * Rotation3::from_axis_angle(&z_axis, -std::f64::consts::FRAC_PI_2);
    // since this coincides with roll axis, the roll is neglected when transformed back
    let axis_transformed = corrected_transform * axis;

    let pitch = (axis_transformed.z).asin();
    let yaw = axis_transformed.y.simd_atan2(axis_transformed.x) - std::f64::consts::FRAC_PI_2;
    
    from_euler_yxz(pitch, roll_correction, yaw)
}

#[derive(Clone)]
pub struct HorizonLock {
    pub lock_enabled: bool,
    pub horizonlockpercent: f64,
    pub horizonroll: f64,
}

impl Default for HorizonLock {
    fn default() -> Self { Self {
        lock_enabled: false,
        horizonlockpercent: 100.0,
        horizonroll: 0.0,
    } }
}

impl HorizonLock {
    pub fn set_horizon(&mut self, lock_percent: f64, roll: f64) {
        self.horizonroll = roll;
        self.horizonlockpercent = lock_percent;
        self.lock_enabled = self.horizonlockpercent > 1e-6;
    }
    pub fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.horizonlockpercent.to_bits());
        hasher.write_u64(self.horizonroll.to_bits());
        hasher.finish()
    }

    pub fn lockquat(&self, q: &UnitQuaternion<f64>) -> UnitQuaternion<f64> {
        lock_horizon_angle(q, self.horizonroll * std::f64::consts::PI / 180.0).slerp(&q, 1.0 - self.horizonlockpercent / 100.0)
    }

    pub fn lock(&self, smoothed_quats: &mut TimeQuat, quats: &mut TimeQuat, grav: &Option<crate::gyro_source::TimeVec>, int_method: usize) {
        if self.lock_enabled {
            if int_method == 0 {
                // Only with "None" integration method
                if let Some(gvec) = grav {
                    let z_axis = nalgebra::Vector3::<f64>::z_axis();
                    let y_axis = nalgebra::Vector3::<f64>::y_axis();
                    //let corr = Rotation3::from_axis_angle(&z_axis, std::f64::consts::PI);

                    for (k, smoothed_ori) in smoothed_quats.iter_mut() {
                        let gv = gvec.get(k).unwrap_or(&y_axis);
                        let ori = quats.get(k).unwrap_or(&smoothed_ori).to_rotation_matrix();

                        // Correct for angle difference between original and smoothed orientation
                        let correction = ori.inverse() * smoothed_ori.to_rotation_matrix();
                        let angle_corr = (-correction[(0,1)]).simd_atan2(correction[(0,0)]);

                        //let gv_corrected = corr.inverse() * correction * corr * gv; // Alternative matrix approach
                        //let locked_ori =  smoothed_ori.to_rotation_matrix() * Rotation3::from_axis_angle(&z_axis, gv_corrected[0].simd_atan2(gv_corrected[1]) + self.horizonroll * std::f64::consts::PI / 180.0);
                        let locked_ori =  smoothed_ori.to_rotation_matrix() * Rotation3::from_axis_angle(&z_axis, -angle_corr + gv[0].simd_atan2(gv[1]) + self.horizonroll * std::f64::consts::PI / 180.0);
                        *smoothed_ori = UnitQuaternion::from_rotation_matrix(&locked_ori).slerp(&smoothed_ori, 1.0 - self.horizonlockpercent / 100.0);
                    }
                    return;
                }
            }

            for (_k, v) in smoothed_quats.iter_mut() {
                *v = self.lockquat(v);
            }
        }
    }
}
