// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use std::{ collections::BTreeMap, collections::btree_map::Entry, str::FromStr };
use crate::gyro_source::GyroSource;
use std::sync::{ Arc, Mutex }; // parking_lot::Mutex can't be used across catch_unwind

macro_rules! define_keyframes {
    ($($name:ident, $color:literal, $text:literal, $format:expr,)*) => {
        #[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Eq, Ord, ::serde::Serialize, ::serde::Deserialize)]
        pub enum KeyframeType {
            $($name),*
        }
        pub fn keyframe_color(kf: &KeyframeType) -> &'static str {
            match kf { $(KeyframeType::$name => $color),* }
        }
        pub fn keyframe_text(kf: &KeyframeType) -> &'static str {
            match kf { $(KeyframeType::$name => $text),* }
        }
        pub fn keyframe_format_value(kf: &KeyframeType, v: f64) -> String {
            match kf { $(KeyframeType::$name => $format(v)),* }
        }
    };
}

define_keyframes! {
    Fov,                         "#8ee6ea", "FOV",                              |v| format!("{:.2}", v),
    VideoRotation,               "#eae38e", "Video rotation",                   |v| format!("{:.1}°", v),
    ZoomingSpeed,                "#32e595", "Zooming speed",                    |v| format!("{:.2}s", v),
    ZoomingCenterX,              "#6fefb6", "Zooming center offset X",          |v| format!("{:.0}%", v * 100.0),
    ZoomingCenterY,              "#5ddba2", "Zooming center offset Y",          |v| format!("{:.0}%", v * 100.0),
    BackgroundMargin,            "#6e5ddb", "Background margin",                |v| format!("{:.0}%", v),
    BackgroundFeather,           "#9d93e1", "Background feather",               |v| format!("{:.0}%", v),
    LockHorizonAmount,           "#ed7789", "Horizon lock amount",              |v| format!("{:.0}%", v),
    LockHorizonRoll,             "#e86176", "Horizon lock roll correction",     |v| format!("{:.1}°", v),
    LensCorrectionStrength,      "#e8ae61", "Lens correction strength",         |v| format!("{:.0}%", v * 100.0),

    SmoothingParamTimeConstant,  "#94ea8e", "Max smoothness",                   |v| format!("{:.2}", v),
    SmoothingParamTimeConstant2, "#89df82", "Max smoothness at high velocity",  |v| format!("{:.2}", v),
    SmoothingParamSmoothness,    "#7ced74", "Smoothness",                       |v| format!("{:.2}", v),
    SmoothingParamPitch,         "#59c451", "Pitch smoothness",                 |v| format!("{:.2}", v),
    SmoothingParamRoll,          "#51c485", "Roll smoothness",                  |v| format!("{:.2}", v),
    SmoothingParamYaw,           "#88c451", "Yaw smoothness",                   |v| format!("{:.2}", v),

    VideoSpeed,                  "#f6e926", "Video speed",                      |v| format!("{:.1}%", v * 100.0),
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
    custom_provider: Option<Arc<Mutex<dyn FnMut(&KeyframeManager, &KeyframeType, f64) -> Option<f64> + Send + 'static>>>,
    pub timestamp_scale: Option<f64>,
}

impl KeyframeManager {
    pub fn new() -> Self { Self::default() }

    pub fn set_custom_provider(&mut self, cb: impl FnMut(&KeyframeManager, &KeyframeType, f64) -> Option<f64> + Send + 'static) {
        self.custom_provider = Some(Arc::new(Mutex::new(cb)));
    }
    pub fn set(&mut self, typ: &KeyframeType, timestamp_us: i64, value: f64) {
        let kf = Keyframe {
            value,
            easing: Easing::EaseInOut
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
    pub fn is_keyframed_internally(&self, typ: &KeyframeType) -> bool {
        if let Some(x) = self.keyframes.get(typ) {
            return x.len() > 0;
        }
        false
    }
    pub fn is_keyframed(&self, typ: &KeyframeType) -> bool {
        if let Some(custom) = &self.custom_provider {
            if let Ok(mut custom) = custom.lock() {
                if let Some(_) = (*custom)(self, typ, 0.0) {
                    return true;
                }
            }
        }
        self.is_keyframed_internally(typ)
    }
    pub fn value_at_video_timestamp(&self, typ: &KeyframeType, timestamp_ms: f64) -> Option<f64> {
        if let Some(custom) = &self.custom_provider {
            if let Ok(mut custom) = custom.lock() {
                if let Some(v) = (*custom)(self, typ, timestamp_ms * self.timestamp_scale.unwrap_or(1.0)) {
                    return Some(v);
                }
            }
        }
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

    pub fn next_keyframe(&self, ts: i64, typ: Option<KeyframeType>) -> Option<(KeyframeType, i64, Keyframe)> {
        if let Some(kf) = typ {
            let res = self.keyframes.get(&kf)?.range(ts+1..).next()?;
            Some((kf, *res.0, *res.1))
        } else {
            self.keyframes
                .iter()
                .filter_map(|(&k, _)| self.next_keyframe(ts, Some(k)) )
                .min_by_key(|(_nt, nts, _nk)| (nts - ts).abs())
        }
    }
    pub fn prev_keyframe(&self, ts: i64, typ: Option<KeyframeType>) -> Option<(KeyframeType, i64, Keyframe)> {
       if let Some(kf) = typ {
            let res = self.keyframes.get(&kf)?.range(..ts).next_back()?;
            Some((kf, *res.0, *res.1))
        } else {
            self.keyframes
                .iter()
                .filter_map(|(&k, _)| self.prev_keyframe(ts, Some(k)) )
                .min_by_key(|(_nt, nts, _nk)| (nts - ts).abs())
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
