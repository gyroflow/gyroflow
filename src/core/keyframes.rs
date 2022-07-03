// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use std::{ collections::BTreeMap, collections::btree_map::Entry, str::FromStr };
use crate::gyro_source::GyroSource;

// TODO: points on timeline are rendered with slight horizontal offset

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Eq, Ord, ::serde::Serialize, ::serde::Deserialize)]
pub enum KeyframeType {
    Fov,
    VideoRotation,
    ZoomingSpeed,
    ZoomingCenterX,
    ZoomingCenterY,
    BackgroundMargin,
    BackgroundFeather,
    LockHorizonAmount,
    LockHorizonRoll,
    LensCorrectionStrength,

    SmoothingParamTimeConstant,
    SmoothingParamTimeConstant2,
    SmoothingParamSmoothness,
    SmoothingParamPitch,
    SmoothingParamRoll,
    SmoothingParamYaw,
}

#[derive(Default, Debug, Copy, Clone, PartialEq, PartialOrd, Eq, Ord, ::serde::Serialize, ::serde::Deserialize)]
pub enum Easing {
    #[default]
    NoEasing, // Linear
    EaseIn,
    EaseOut,
    EaseInOut
}

#[derive(Debug, Copy, Clone, Default, ::serde::Serialize, ::serde::Deserialize)]
pub struct Keyframe {
    pub value: f64,
    pub easing: Easing
}

#[derive(Default, Clone)]
pub struct KeyframeManager {
    keyframes: BTreeMap<KeyframeType, BTreeMap<i64, Keyframe>>,
    gyro_offsets: BTreeMap<i64, f64>,
    pub timestamp_scale: Option<f64>,
}

impl KeyframeManager {
    pub fn new() -> Self { Self::default() }

    pub fn set(&mut self, typ: &KeyframeType, timestamp_us: i64, value: f64) {
        let kf = Keyframe {
            value,
            ..Default::default()
        };
        if let Some(x) = self.keyframes.get_mut(typ) {
            match x.entry(timestamp_us) {
                Entry::Occupied(o) => { o.into_mut().value = value; }
                Entry::Vacant(v) => { v.insert(kf); }
            }
        } else {
            self.keyframes.insert(typ.clone(), BTreeMap::from([(timestamp_us, kf)]));
        }
    }
    pub fn set_easing(&mut self, typ: &KeyframeType, timestamp_us: i64, easing: Easing) {
        if let Some(x) = self.keyframes.get_mut(typ) {
            if let Some(kf) = x.get_mut(&timestamp_us) {
                kf.easing = easing;
            }
        }
    }
    pub fn easing(&self, typ: &KeyframeType, timestamp_us: i64) -> Option<Easing> {
        Some(self.keyframes.get(typ)?.get(&timestamp_us)?.easing)
    }
    pub fn remove(&mut self, typ: &KeyframeType, timestamp_us: i64) {
        if let Some(x) = self.keyframes.get_mut(typ) {
            x.remove(&timestamp_us);
        }
    }
    pub fn is_keyframed(&self, typ: &KeyframeType) -> bool {
        if let Some(x) = self.keyframes.get(typ) {
            return x.len() > 0;
        }
        false
    }
    pub fn value_at_video_timestamp(&self, typ: &KeyframeType, timestamp_ms: f64) -> Option<f64> {
        let keyframes = self.keyframes.get(typ)?;
        match keyframes.len() {
            0 => None,
            1 => Some(keyframes.values().next().unwrap().value),
            _ => {
                if let Some(&first_ts) = keyframes.keys().next() {
                    if let Some(&last_ts) = keyframes.keys().next_back() {
                        let timestamp_us = (timestamp_ms * 1000.0 * self.timestamp_scale.unwrap_or(1.0)).round() as i64;
                        let lookup_ts = timestamp_us.min(last_ts).max(first_ts);
                        if let Some(offs1) = keyframes.range(..=lookup_ts).next_back() {
                            if *offs1.0 == lookup_ts {
                                return Some(offs1.1.value);
                            }
                            if let Some(offs2) = keyframes.range(lookup_ts..).next() {
                                let time_delta = (offs2.0 - offs1.0) as f64;
                                let alpha = (timestamp_us - offs1.0) as f64 / time_delta;
                                let e = Easing::get(&offs1.1.easing, &offs2.1.easing, alpha);
                                return Some(e.interpolate(offs1.1.value, offs2.1.value, alpha));
                            }
                        }
                    }
                }

                None
            }
        }
    }

    pub fn value_at_gyro_timestamp(&self, typ: &KeyframeType, mut timestamp_ms: f64) -> Option<f64> {
        timestamp_ms += GyroSource::offset_at_timestamp(&self.gyro_offsets, timestamp_ms);
        self.value_at_video_timestamp(typ, timestamp_ms)
    }

    pub fn get_keyframes(&self, typ: &KeyframeType) -> Option<&BTreeMap<i64, Keyframe>> {
        self.keyframes.get(typ)
    }

    pub fn get_all_keys(&self) -> Vec<&KeyframeType> {
        self.keyframes.iter().filter(|(_, v)| !v.is_empty()).map(|(k, _)| k).collect()
    }

    pub fn update_gyro(&mut self, gyro: &GyroSource) {
        self.gyro_offsets = gyro.get_offsets().clone();
    }
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    pub fn clear_type(&mut self, key: &KeyframeType) {
        self.keyframes.remove(key);
    }

    pub fn serialize(&self) -> serde_json::Value {
        serde_json::to_value(&self.keyframes).unwrap_or(serde_json::Value::Null)
    }
    pub fn deserialize(&mut self, v: &serde_json::Value) {
        self.keyframes.clear();
        if let Ok(kf) = serde_json::from_value(v.clone()) {
            self.keyframes = kf;
        }
    }
}

impl FromStr for KeyframeType {
    type Err = serde_json::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> { serde_json::from_str(&format!("\"{}\"", s)) }
}
impl ToString for KeyframeType {
    fn to_string(&self) -> String { format!("{:?}", self) }
}
impl FromStr for Easing {
    type Err = serde_json::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> { serde_json::from_str(&format!("\"{}\"", s)) }
}
impl ToString for Easing {
    fn to_string(&self) -> String { format!("{:?}", self) }
}

pub fn color_for_keyframe(kf: &KeyframeType) -> &'static str {
    match kf {
        KeyframeType::Fov                         => "#8ee6ea",
        KeyframeType::VideoRotation               => "#eae38e",
        KeyframeType::ZoomingSpeed                => "#32e595",
        KeyframeType::ZoomingCenterX              => "#6fefb6",
        KeyframeType::ZoomingCenterY              => "#5ddba2",
        KeyframeType::BackgroundMargin            => "#6e5ddb",
        KeyframeType::BackgroundFeather           => "#9d93e1",
        KeyframeType::LockHorizonAmount           => "#ed7789",
        KeyframeType::LockHorizonRoll             => "#e86176",
        KeyframeType::LensCorrectionStrength      => "#e8ae61",

        KeyframeType::SmoothingParamTimeConstant  => "#94ea8e",
        KeyframeType::SmoothingParamTimeConstant2 => "#89df82",
        KeyframeType::SmoothingParamSmoothness    => "#7ced74",
        KeyframeType::SmoothingParamPitch         => "#59c451",
        KeyframeType::SmoothingParamRoll          => "#51c485",
        KeyframeType::SmoothingParamYaw           => "#88c451",
        // _ => { ::log::warn!("Unknown color for keyframe {:?}", kf); "#8e96ea" }
    }
}

impl Easing {
    pub fn get(a: &Self, b: &Self, _alpha: f64) -> Self {
        // let a_in  = a == &Self::EaseIn  || a == &Self::EaseInOut;
        // let b_out = b == &Self::EaseOut || b == &Self::EaseInOut;
        let a_out = a == &Self::EaseOut || a == &Self::EaseInOut;
        let b_in  = b == &Self::EaseIn  || b == &Self::EaseInOut;

        if a_out && b_in { return Self::EaseInOut; }
        if b_in { return Self::EaseOut; }
        if a_out { return Self::EaseIn; }

        Self::NoEasing
    }
    pub fn interpolate(&self, a: f64, b: f64, mut x: f64) -> f64 {
        x = match self {
            Self::EaseIn    => simple_easing::sine_in    (x as f32) as f64, // https://easings.net/#easeInSine
            Self::EaseOut   => simple_easing::sine_out   (x as f32) as f64, // https://easings.net/#easeOutSine
            Self::EaseInOut => simple_easing::sine_in_out(x as f32) as f64, // https://easings.net/#easeInOutSine
            _ => x
        };

        a * (1.0 - x) + b * x
    }
}
