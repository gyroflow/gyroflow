// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use serde::{ Serialize, Deserialize };

use telemetry_parser::Input;
use telemetry_parser::tags_impl::{ GetWithType, GroupId, TagId };
use std::io::Result;

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
pub struct CameraIdentifier {
    pub brand: String,
    pub model: String,
    pub lens_model: String,
    pub lens_info: String,
    pub camera_setting: String,
    pub fps: usize,
    pub video_width: usize,
    pub video_height: usize,
    pub additional: String,

    pub identifier: String
}

impl CameraIdentifier {
    pub fn from_telemetry_parser(input: &Input, video_width: usize, video_height: usize, fps: f64) -> Result<Self> {
        let fps = (fps * 1000.0).round() as usize;
        let brand = input.camera_type();
        let model = input.camera_model().cloned().unwrap_or_default();

        let mut id = Self {
            brand: brand.clone(),
            model,
            video_width,
            video_height,
            fps,

            ..Default::default()
        };

        match id.brand.to_ascii_lowercase().as_str() {
            "runcam" | "caddx" => id.lens_info = "wide".into(),
            _ => { }
        }

        if !id.brand.is_empty() {
            id.model = id.model.to_string().replace(&id.brand, "").trim().to_string();
        }

        match brand.as_str() {
            "GoPro" => {
                if let Some(ref samples) = input.samples {
                    for info in samples {
                        if let Some(ref tag_map) = info.tag_map {
                            if let Some(map) = tag_map.get(&GroupId::Default) {
                                if let Some(v) = map.get_t(TagId::Unknown(0x45495341/*EISA*/)) as Option<&String> {
                                    if v != "N/A" {
                                        id.additional = if v == "Y" || v == "N" {
                                            format!("EIS-{}", v)
                                        } else {
                                            v.clone()
                                        };
                                    }
                                }
                                if let Some(v) = map.get_t(TagId::Unknown(0x45495345/*EISE*/)) as Option<&String> {
                                    if id.additional.is_empty() {
                                        id.additional = format!("EIS-{}", v);
                                    }
                                }
                                if id.additional == "EIS-N" {
                                    id.additional = "NO-EIS".into();
                                }
                                if let Some(v) = map.get_t(TagId::Unknown(0x56464f56/*VFOV*/)) as Option<&String> {
                                    match v.as_str() {
                                        "X" => id.lens_info = "Max".into(),
                                        "W" => id.lens_info = "Wide".into(),
                                        "S" => id.lens_info = "Super".into(),
                                        "H" => id.lens_info = "Hyper".into(),
                                        "L" => id.lens_info = "Linear".into(),
                                        "N" => id.lens_info = "Narrow".into(),
                                        _ => id.lens_info = v.into()
                                    };
                                }
                                if let Some(v) = map.get_t(TagId::Unknown(0x5a464f56/*ZFOV*/)) as Option<&f32> {
                                    if id.lens_info == "Linear" && *v < 80.0 {
                                        id.lens_info = "Narrow".into();
                                    }
                                }
                                if let Some(v) = map.get_t(TagId::Unknown(0x50524a54/*PRJT*/)) as Option<&String> {
                                    if v.as_str() == "GPMW" {
                                        id.lens_info = "Max Wide".into();
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            },
            "Sony" => {
                if let Some(ref samples) = input.samples {
                    if let Some(info) = samples.iter().next() {
                        if let Some(ref tag_map) = info.tag_map {
                            if let Some(v) = tag_map.get(&GroupId::Lens).and_then(|map| map.get_t(TagId::LensZoomNative) as Option<&f32>) {
                                id.lens_info = format!("{:.2}mm", v);
                            }
                            if let Some(v) = tag_map.get(&GroupId::Custom("LensDistortion".into())).and_then(|map| map.get_t(TagId::Data) as Option<&serde_json::Value>) {
                                if id.lens_info.is_empty() {
                                    let mut hasher = crc32fast::Hasher::new();
                                    hasher.update(v.to_string().as_bytes());
                                    id.lens_info = format!("{:x}", hasher.finalize());
                                }
                            }
                        }
                    }
                }
            },
            "Insta360" => {
                if let Some(ref samples) = input.samples {
                    for info in samples {
                        if let Some(ref tag_map) = info.tag_map {
                            if let Some(map) = tag_map.get(&GroupId::Default) {
                                if let Some(v) = map.get_t(TagId::Metadata) as Option<&serde_json::Value> {
                                    if let Some(fov_type) = v.get("fov_type").and_then(|v| v.as_str()) {
                                        id.lens_info = fov_type.replace("FovType", "");
                                    }
                                    if let Some(fov) = v.get("fov").and_then(|v| v.as_f64()) {
                                        if fov > 0.0 {
                                            id.lens_info.push_str(&format!(" {:.0}", fov));
                                        }
                                    }
                                    if let Some(flowstate) = v.get("is_flowstate_online").and_then(|v| v.as_bool()) {
                                        id.additional = if flowstate { "EIS" } else { "NO-EIS" }.into();
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
            }
            _ => {
                if let Some(ref samples) = input.samples {
                    let mut try_again = false;
                    for info in samples {
                        if let Some(ref tag_map) = info.tag_map {
                            if let Some(v) = tag_map.get(&GroupId::Lens).and_then(|map| map.get_t(TagId::LensZoomNative) as Option<&f32>) {
                                id.lens_info = format!("{:.2}mm", v);
                            }
                            if let Some(v) = tag_map.get(&GroupId::Lens).and_then(|map| map.get_t(TagId::Name) as Option<&String>) {
                                id.lens_model = v.clone();
                            }
                            if let Some(map) = tag_map.get(&GroupId::Default) {
                                if let Some(v) = map.get_t(TagId::Metadata) as Option<&serde_json::Value> {
                                    log::debug!("Camera ID Brand: {}, Model: {}, Metadata: {:?}", id.brand, id.model, v);
                                    if let Some(v) = v.get("lens_info")             .and_then(|v| v.as_str()) { id.lens_info      = v.to_string(); }
                                    if let Some(v)  = v.get("focal_length")          .and_then(|v| v.as_f64()) { id.lens_info      = format!("{:.2}mm", v); }
                                    if let Some(v) = v.get("focal_length")          .and_then(|v| v.as_str()) { id.lens_info      = v.to_string(); }
                                    if let Some(v) = v.get("lens_type")             .and_then(|v| v.as_str()) { id.lens_model     = v.to_string(); }
                                    if let Some(v) = v.get("resolution_format_name").and_then(|v| v.as_str()) { id.camera_setting = v.to_string(); }
                                }
                            }
                        }
                        if id.lens_info.is_empty() && !try_again {
                            try_again = true;
                            continue;
                        }
                        break;
                    }
                }
            }
        }

        id.identifier = id.get_identifier();
        dbg!(&id);

        log::info!("Camera identifier string: {}", id.identifier);

        Ok(id)
    }

    fn get_identifier(&self) -> String {
        if self.brand.is_empty() || self.model.is_empty() || self.lens_info.is_empty() { return String::new(); }

        let mut id = format!("{}-{}-{}-{}-{}x{}@{}-{}", self.brand, self.model, self.lens_model, self.lens_info, self.video_width, self.video_height, self.fps, self.additional);
        id = id.replace(' ', "");
        id = id.replace("--", "-");
        id = id.replace("--", "-");
        let x: &[_] = &['-', ' '];
        id.trim_matches(x).to_lowercase()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}
