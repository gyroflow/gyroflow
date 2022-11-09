// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Aphobius

// 1. Calculate velocity for each quaternion
// 2. Smooth the velocities
// 3. Multiply max velocity (500 deg/s) with slider value
// 4. Perform plain 3D smoothing with varying alpha, where each alpha is interpolated between 1s smoothness at 0 velocity, 0.1s smoothness at max velocity and extrapolated above that
// 5. This way, low velocities are smoothed using 1s smoothness, but high velocities are smoothed using 0.1s smoothness at max velocity (500 deg/s multiplied by slider) and gradually lower smoothness above that
// 6. Calculate distance from smoothed quaternions to raw quaternions
// 7. Normalize distance and set everything bellow 0.5 to 0.0
// 8. Smooth distance
// 9. Normalize distance again and change range to 0.5 - 1.0
// 10. Perform plain 3D smoothing, on the last smoothed quaternions, with varying alpha, interpolated between 1s and 0.1s smoothness based on previously calculated velocity multiplied by the distance

use std::collections::BTreeMap;

use super::*;
use crate::gyro_source::TimeQuat;
use nalgebra::*;
use crate::Quat64;
use crate::keyframes::*;

#[derive(Clone)]
pub struct DefaultAlgo {
    pub smoothness: f64,
    pub smoothness_pitch: f64,
    pub smoothness_yaw: f64,
    pub smoothness_roll: f64,
    pub per_axis: bool,
    pub second_pass: bool,
    pub trim_range_only: bool,
    pub max_smoothness: f64,
    pub alpha_0_1s: f64,
}

impl Default for DefaultAlgo {
    fn default() -> Self { Self {
        smoothness: 0.5,
        smoothness_pitch: 0.5,
        smoothness_yaw: 0.5,
        smoothness_roll: 0.5,
        per_axis: false,
        second_pass: true,
        trim_range_only: false,
        max_smoothness: 1.0,
        alpha_0_1s: 0.1
    } }
}

impl SmoothingAlgorithm for DefaultAlgo {
    fn get_name(&self) -> String { "Default".to_owned() }

    fn set_parameter(&mut self, name: &str, val: f64) {
        match name {
            "smoothness"       => self.smoothness = val,
            "smoothness_pitch" => self.smoothness_pitch = val,
            "smoothness_yaw"   => self.smoothness_yaw = val,
            "smoothness_roll"  => self.smoothness_roll = val,
            "per_axis"         => self.per_axis = val > 0.1,
            "second_pass"      => self.second_pass = val > 0.1,
            "trim_range_only"  => self.trim_range_only = val > 0.1,
            "max_smoothness"   => self.max_smoothness = val,
            "alpha_0_1s"       => self.alpha_0_1s = val,
            _ => log::error!("Invalid parameter name: {}", name)
        }
    }
    fn get_parameter(&self, name: &str) -> f64 {
        match name {
            "smoothness"       => self.smoothness,
            "smoothness_pitch" => self.smoothness_pitch,
            "smoothness_yaw"   => self.smoothness_yaw,
            "smoothness_roll"  => self.smoothness_roll,
            "per_axis"         => if self.per_axis { 1.0 } else { 0.0 },
            "second_pass"      => if self.second_pass { 1.0 } else { 0.0 },
            "trim_range_only"  => if self.trim_range_only { 1.0 } else { 0.0 },
            "max_smoothness"   => self.max_smoothness,
            "alpha_0_1s"       => self.alpha_0_1s,
            _ => 0.0
        }
    }

    fn get_parameters_json(&self) -> serde_json::Value {
        serde_json::json!([
            {
                "name": "smoothness",
                "description": "Smoothness",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 1.0,
                "value": self.smoothness,
                "default": 0.5,
                "unit": "",
                "precision": 3,
                "keyframe": "SmoothingParamSmoothness"
            },
            {
                "name": "smoothness_pitch",
                "description": "Pitch smoothness",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 1.0,
                "value": self.smoothness_pitch,
                "default": 0.5,
                "unit": "",
                "precision": 3,
                "keyframe": "SmoothingParamPitch"
            },
            {
                "name": "smoothness_yaw",
                "description": "Yaw smoothness",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 1.0,
                "value": self.smoothness_yaw,
                "default": 0.5,
                "unit": "",
                "precision": 3,
                "keyframe": "SmoothingParamYaw"
            },
            {
                "name": "smoothness_roll",
                "description": "Roll smoothness",
                "type": "SliderWithField",
                "from": 0.001,
                "to": 1.0,
                "value": self.smoothness_roll,
                "default": 0.5,
                "unit": "",
                "precision": 3,
                "keyframe": "SmoothingParamRoll"
            },
            {
                "name": "per_axis",
                "description": "Per axis",
                "advanced": true,
                "type": "CheckBox",
                "default": self.per_axis,
                "value": if self.per_axis { 1.0 } else { 0.0 },
                "custom_qml": "Connections { function onCheckedChanged() {
                    const checked = root.getParamElement('per_axis').checked;
                    root.getParamElement('smoothness-label').visible = !checked;
                    root.getParamElement('smoothness_pitch-label').visible = checked;
                    root.getParamElement('smoothness_yaw-label').visible = checked;
                    root.getParamElement('smoothness_roll-label').visible = checked;
                }}"
            },
            {
                "name": "second_pass",
                "description": "Second smoothing pass",
                "advanced": true,
                "type": "CheckBox",
                "default": self.second_pass,
                "value": if self.second_pass { 1.0 } else { 0.0 },
            },
            {
                "name": "trim_range_only",
                "description": "Only within trim range",
                "advanced": true,
                "type": "CheckBox",
                "default": self.trim_range_only,
                "value": if self.trim_range_only { 1.0 } else { 0.0 },
            },
            {
                "name": "max_smoothness",
                "description": "Max smoothness",
                "advanced": true,
                "type": "SliderWithField",
                "from": 0.1,
                "to": 5.0,
                "value": self.max_smoothness,
                "default": 1.0,
                "unit": "s",
                "keyframe": "SmoothingParamTimeConstant"
            },
            {
                "name": "alpha_0_1s",
                "description": "Max smoothness at high velocity",
                "advanced": true,
                "type": "SliderWithField",
                "from": 0.01,
                "to": 1.0,
                "value": self.alpha_0_1s,
                "default": 0.1,
                "unit": "s",
                "keyframe": "SmoothingParamTimeConstant2"
            }
        ])
    }

    fn get_status_json(&self) -> serde_json::Value {
        serde_json::json!([])
    }

    fn get_checksum(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        hasher.write_u64(self.smoothness.to_bits());
        hasher.write_u64(self.smoothness_pitch.to_bits());
        hasher.write_u64(self.smoothness_yaw.to_bits());
        hasher.write_u64(self.smoothness_roll.to_bits());
        hasher.write_u64(self.max_smoothness.to_bits());
        hasher.write_u64(self.alpha_0_1s.to_bits());
        hasher.write_u8(if self.per_axis { 1 } else { 0 });
        hasher.write_u8(if self.second_pass { 1 } else { 0 });
        hasher.finish()
    }

    fn smooth(&self, quats: &TimeQuat, duration: f64, stabilization_params: &StabilizationParams, keyframes: &KeyframeManager) -> TimeQuat { // TODO Result<>?
        if quats.is_empty() || duration <= 0.0 { return quats.clone(); }

        const MAX_VELOCITY: f64 = 500.0;
        const RAD_TO_DEG: f64 = 180.0 / std::f64::consts::PI;
        let sample_rate: f64 = quats.len() as f64 / (duration / 1000.0);
        let rad_to_deg_per_sec: f64 = sample_rate * RAD_TO_DEG;

        let get_alpha = |time_constant: f64| {
            1.0 - (-(1.0 / sample_rate) / time_constant).exp()
        };
        let noop = |v| v;

        let mut quats_copy = None;
        let quats = if self.trim_range_only && (stabilization_params.trim_start != 0.0 || stabilization_params.trim_end != 1.0) {
            let ts_start = ((duration * stabilization_params.trim_start) * 1000.0).round() as i64;
            let ts_end   = ((duration * stabilization_params.trim_end) * 1000.0).round() as i64;
            if quats.range(ts_start..ts_end).next().is_none() {
                &quats
            } else {
                let mut first_q = quats.range(ts_start..ts_end).next().unwrap().1.clone();
                let mut last_q = quats.range(ts_start..ts_end).next_back().unwrap().1.clone();
                quats_copy = Some(quats.clone());
                for (ts, q) in quats_copy.as_mut().unwrap().iter_mut() {
                    if *ts < ts_start {
                        *q = first_q.clone();
                    } else if *ts > ts_end {
                        *q = last_q.clone();
                    }
                }
                quats_copy.as_ref().unwrap()
            }
        } else {
            &quats
        };

        let get_keyframed_param = |typ: &KeyframeType, def: f64, cb: &dyn Fn(f64) -> f64| -> BTreeMap<i64, f64> {
            let mut ret = BTreeMap::<i64, f64>::new();
            if keyframes.is_keyframed(typ) || (stabilization_params.video_speed_affects_smoothing && (stabilization_params.video_speed != 1.0 || keyframes.is_keyframed(&KeyframeType::VideoSpeed))) {
                ret = quats.iter().map(|(ts, _)| {
                    let timestamp_ms = *ts as f64 / 1000.0;
                    let mut val = keyframes.value_at_gyro_timestamp(typ, timestamp_ms).unwrap_or(def);
                    if stabilization_params.video_speed_affects_smoothing {
                        let vid_speed = keyframes.value_at_gyro_timestamp(&KeyframeType::VideoSpeed, timestamp_ms).unwrap_or(stabilization_params.video_speed);
                        if typ == &KeyframeType::SmoothingParamTimeConstant || typ == &KeyframeType::SmoothingParamTimeConstant2 {
                            val *= vid_speed / 2.0;
                        } else {
                            val *= vid_speed;
                        }
                    }
                    (*ts, cb(val))
                }).collect();
            }
            ret
        };

        let alpha_smoothness_per_timestamp = get_keyframed_param(&KeyframeType::SmoothingParamTimeConstant, self.max_smoothness, &get_alpha);
        let alpha_0_1s_per_timestamp = get_keyframed_param(&KeyframeType::SmoothingParamTimeConstant2, self.alpha_0_1s, &get_alpha);
        let smoothness_per_timestamp = get_keyframed_param(&KeyframeType::SmoothingParamSmoothness, self.smoothness, &noop);
        let smoothness_pitch_per_timestamp = get_keyframed_param(&KeyframeType::SmoothingParamPitch, self.smoothness_pitch, &noop);
        let smoothness_yaw_per_timestamp = get_keyframed_param(&KeyframeType::SmoothingParamYaw, self.smoothness_yaw, &noop);
        let smoothness_roll_per_timestamp = get_keyframed_param(&KeyframeType::SmoothingParamRoll, self.smoothness_roll, &noop);

        let alpha_smoothness = get_alpha(self.max_smoothness);
        let alpha_0_1s = get_alpha(self.alpha_0_1s);

        // Calculate velocity
        let mut velocity = BTreeMap::<i64, Vector3<f64>>::new();

        let first_quat = quats.iter().next().unwrap(); // First quat
        velocity.insert(*first_quat.0, Vector3::from_element(0.0));

        let mut prev_quat = *quats.iter().next().unwrap().1; // First quat
        for (timestamp, quat) in quats.iter().skip(1) {
            let dist = prev_quat.inverse() * quat;
            if self.per_axis {
                let euler = dist.euler_angles();
                velocity.insert(*timestamp, Vector3::new(
                    euler.0.abs() * rad_to_deg_per_sec,
                    euler.1.abs() * rad_to_deg_per_sec,
                    euler.2.abs() * rad_to_deg_per_sec
                ));
            } else {
                velocity.insert(*timestamp, Vector3::from_element(dist.angle() * rad_to_deg_per_sec));
            }
            prev_quat = *quat;
        }

        // Smooth velocity
        let mut prev_velocity = *velocity.iter().next().unwrap().1; // First velocity
        for (_timestamp, vel) in velocity.iter_mut().skip(1) {
            *vel = prev_velocity * (1.0 - alpha_0_1s) + *vel * alpha_0_1s;
            prev_velocity = *vel;
        }
        for (_timestamp, vel) in velocity.iter_mut().rev().skip(1) {
            *vel = prev_velocity * (1.0 - alpha_0_1s) + *vel * alpha_0_1s;
            prev_velocity = *vel;
        }

        // Normalize velocity
        for (ts, vel) in velocity.iter_mut() {
            let smoothness_pitch = smoothness_pitch_per_timestamp.get(ts).unwrap_or(&self.smoothness_pitch);
            let smoothness_yaw   = smoothness_yaw_per_timestamp  .get(ts).unwrap_or(&self.smoothness_yaw);
            let smoothness_roll  = smoothness_roll_per_timestamp .get(ts).unwrap_or(&self.smoothness_roll);
            let smoothness       = smoothness_per_timestamp      .get(ts).unwrap_or(&self.smoothness);

            // Calculate max velocity
            let mut max_velocity = [MAX_VELOCITY, MAX_VELOCITY, MAX_VELOCITY];
            if self.per_axis {
                max_velocity[0] *= smoothness_pitch;
                max_velocity[1] *= smoothness_yaw;
                max_velocity[2] *= smoothness_roll;
            } else {
                max_velocity[0] *= smoothness;
            }

            // Doing this to get similar max zoom as without second pass
            if self.second_pass {
                max_velocity[0] *= 0.5;
                if self.per_axis {
                    max_velocity[1] *= 0.5;
                    max_velocity[2] *= 0.5;
                }
            }

            vel[0] /= max_velocity[0];
            if self.per_axis {
                vel[1] /= max_velocity[1];
                vel[2] /= max_velocity[2];
            }
        }

        // Plain 3D smoothing with varying alpha
        // Forward pass
        let mut q = *quats.iter().next().unwrap().1;
        let smoothed1: TimeQuat = quats.iter().map(|(ts, x)| {
            let ratio = velocity[ts];
            let alpha_smoothness = alpha_smoothness_per_timestamp.get(ts).unwrap_or(&alpha_smoothness);
            let alpha_0_1s = alpha_0_1s_per_timestamp.get(ts).unwrap_or(&alpha_0_1s);
            if self.per_axis {
                let pitch_factor = alpha_smoothness * (1.0 - ratio[0]) + alpha_0_1s * ratio[0];
                let yaw_factor = alpha_smoothness * (1.0 - ratio[1]) + alpha_0_1s * ratio[1];
                let roll_factor = alpha_smoothness * (1.0 - ratio[2]) + alpha_0_1s * ratio[2];

                let euler_rot = (q.inverse() * x).euler_angles();

                let quat_rot = Quat64::from_euler_angles(
                    euler_rot.0 * pitch_factor.min(1.0),
                    euler_rot.1 * yaw_factor.min(1.0),
                    euler_rot.2 * roll_factor.min(1.0),
                );
                q *= quat_rot;
            } else {
                let val = alpha_smoothness * (1.0 - ratio[0]) + alpha_0_1s * ratio[0];
                q = q.slerp(x, val.min(1.0));
            }
            (*ts, q)
        }).collect();

        // Reverse pass
        let mut q = *smoothed1.iter().next_back().unwrap().1;
        let smoothed2: TimeQuat = smoothed1.iter().rev().map(|(ts, x)| {
            let alpha_smoothness = alpha_smoothness_per_timestamp.get(ts).unwrap_or(&alpha_smoothness);
            let alpha_0_1s = alpha_0_1s_per_timestamp.get(ts).unwrap_or(&alpha_0_1s);
            let ratio = velocity[ts];
            if self.per_axis {
                let pitch_factor = alpha_smoothness * (1.0 - ratio[0]) + alpha_0_1s * ratio[0];
                let yaw_factor = alpha_smoothness * (1.0 - ratio[1]) + alpha_0_1s * ratio[1];
                let roll_factor = alpha_smoothness * (1.0 - ratio[2]) + alpha_0_1s * ratio[2];

                let euler_rot = (q.inverse() * x).euler_angles();

                let quat_rot = Quat64::from_euler_angles(
                    euler_rot.0 * pitch_factor.min(1.0),
                    euler_rot.1 * yaw_factor.min(1.0),
                    euler_rot.2 * roll_factor.min(1.0),
                );
                q *= quat_rot;
            } else {
                let val = alpha_smoothness * (1.0 - ratio[0]) + alpha_0_1s * ratio[0];
                q = q.slerp(x, val.min(1.0));
            }
            (*ts, q)
        }).collect();

        if !self.second_pass {
            return smoothed2;
        }

        // Calculate distance
        let mut distance = BTreeMap::<i64, Vector3<f64>>::new();
        let mut max_distance = Vector3::from_element(0.0);
        for (ts, quat) in smoothed2.iter() {
            let dist = quats[ts].inverse() * quat;
            if self.per_axis {
                let euler = dist.euler_angles();
                distance.insert(*ts, Vector3::new(
                    euler.0.abs(),
                    euler.1.abs(),
                    euler.2.abs()
                ));
                if euler.0.abs() > max_distance[0] { max_distance[0] = euler.0.abs(); }
                if euler.1.abs() > max_distance[1] { max_distance[1] = euler.1.abs(); }
                if euler.2.abs() > max_distance[2] { max_distance[2] = euler.2.abs(); }
            } else {
                distance.insert(*ts, Vector3::from_element(dist.angle()));
                if dist.angle() > max_distance[0] { max_distance[0] = dist.angle(); }
            }
        }

        // Normalize distance and discard under 0.5
        for (_ts, dist) in distance.iter_mut() {
            dist[0] /= max_distance[0];
            if dist[0] < 0.5 { dist[0] = 0.0; }
            if self.per_axis {
                dist[1] /= max_distance[1];
                if dist[1] < 0.5 { dist[1] = 0.0; }
                dist[2] /= max_distance[2];
                if dist[2] < 0.5 { dist[2] = 0.0; }
            }
        }

        // Smooth distance
        let mut prev_dist = *distance.iter().next().unwrap().1;
        for (_timestamp, dist) in distance.iter_mut().skip(1) {
            *dist = prev_dist * (1.0 - alpha_0_1s) + *dist * alpha_0_1s;
            prev_dist = *dist;
        }
        for (_timestamp, dist) in distance.iter_mut().rev().skip(1) {
            *dist = prev_dist * (1.0 - alpha_0_1s) + *dist * alpha_0_1s;
            prev_dist = *dist;
        }

        // Get max distance
        max_distance = Vector3::from_element(0.0);
        for (_ts, dist) in distance.iter_mut() {
            if dist[0] > max_distance[0] { max_distance[0] = dist[0]; }
            if self.per_axis {
                if dist[1] > max_distance[1] { max_distance[1] = dist[1]; }
                if dist[2] > max_distance[2] { max_distance[2] = dist[2]; }
            }
        }

        // Normalize distance and change range to 0.5 - 1.0
        for (_ts, dist) in distance.iter_mut() {
            dist[0] /= max_distance[0];
            dist[0] = (dist[0] + 1.0) / 2.0;
            if self.per_axis {
                dist[1] /= max_distance[1];
                dist[1] = (dist[1] + 1.0) / 2.0;
                dist[2] /= max_distance[2];
                dist[2] = (dist[2] + 1.0) / 2.0;
            }
        }

        // Plain 3D smoothing with varying alpha
        // Forward pass
        let mut q = *smoothed2.iter().next().unwrap().1;
        let smoothed1: TimeQuat = smoothed2.iter().map(|(ts, x)| {
            let alpha_smoothness = alpha_smoothness_per_timestamp.get(ts).unwrap_or(&alpha_smoothness);
            let alpha_0_1s = alpha_0_1s_per_timestamp.get(ts).unwrap_or(&alpha_0_1s);
            let vel_ratio = velocity[ts];
            let dist_ratio = distance[ts];
            if self.per_axis {
                let pitch_factor = alpha_smoothness * (1.0 - vel_ratio[0] * dist_ratio[0]) + alpha_0_1s * vel_ratio[0] * dist_ratio[0];
                let yaw_factor = alpha_smoothness * (1.0 - vel_ratio[1] * dist_ratio[1]) + alpha_0_1s * vel_ratio[1] * dist_ratio[1];
                let roll_factor = alpha_smoothness * (1.0 - vel_ratio[2] * dist_ratio[2]) + alpha_0_1s * vel_ratio[2] * dist_ratio[2];

                let euler_rot = (q.inverse() * x).euler_angles();

                let quat_rot = Quat64::from_euler_angles(
                    euler_rot.0 * pitch_factor.min(1.0),
                    euler_rot.1 * yaw_factor.min(1.0),
                    euler_rot.2 * roll_factor.min(1.0),
                );
                q *= quat_rot;
            } else {
                let val = alpha_smoothness * (1.0 - vel_ratio[0] * dist_ratio[0]) + alpha_0_1s * vel_ratio[0] * dist_ratio[0];
                q = q.slerp(x, val.min(1.0));
            }
            (*ts, q)
        }).collect();

        // Reverse pass
        let mut q = *smoothed1.iter().next_back().unwrap().1;
        smoothed1.iter().rev().map(|(ts, x)| {
            let alpha_smoothness = alpha_smoothness_per_timestamp.get(ts).unwrap_or(&alpha_smoothness);
            let alpha_0_1s = alpha_0_1s_per_timestamp.get(ts).unwrap_or(&alpha_0_1s);
            let vel_ratio = velocity[ts];
            let dist_ratio = distance[ts];
            if self.per_axis {
                let pitch_factor = alpha_smoothness * (1.0 - vel_ratio[0] * dist_ratio[0]) + alpha_0_1s * vel_ratio[0] * dist_ratio[0];
                let yaw_factor = alpha_smoothness * (1.0 - vel_ratio[1] * dist_ratio[1]) + alpha_0_1s * vel_ratio[1] * dist_ratio[1];
                let roll_factor = alpha_smoothness * (1.0 - vel_ratio[2] * dist_ratio[2]) + alpha_0_1s * vel_ratio[2] * dist_ratio[2];

                let euler_rot = (q.inverse() * x).euler_angles();

                let quat_rot = Quat64::from_euler_angles(
                    euler_rot.0 * pitch_factor.min(1.0),
                    euler_rot.1 * yaw_factor.min(1.0),
                    euler_rot.2 * roll_factor.min(1.0),
                );
                q *= quat_rot;
            } else {
                let val = alpha_smoothness * (1.0 - vel_ratio[0] * dist_ratio[0]) + alpha_0_1s * vel_ratio[0] * dist_ratio[0];
                q = q.slerp(x, val.min(1.0));
            }
            (*ts, q)
        }).collect()
    }
}