// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use serde::{ Serialize, Deserialize };

use telemetry_parser::Input;
use telemetry_parser::tags_impl::{ GetWithType, GroupId, TagId };
use std::io::Result;

use std::collections::HashMap;

pub const FULL_FRAME_DIAGONAL_MM: f64 = 43.2666153056; // sqrt(36.0^2 + 24.0^2)

pub fn compute_crop_factor(sensor_width_mm: f64, sensor_height_mm: f64) -> f64 {
    if sensor_width_mm <= 0.0 || sensor_height_mm <= 0.0 {
        return 1.0;
    }
    let diagonal = (sensor_width_mm * sensor_width_mm + sensor_height_mm * sensor_height_mm).sqrt();
    if diagonal <= 0.0 {
        1.0
    } else {
        (FULL_FRAME_DIAGONAL_MM / diagonal * 100.0).round() / 100.0
    }
}

pub fn are_mounts_compatible(lens_mount: &str, camera_mount: &str) -> bool {
    let lm = lens_mount.trim().to_ascii_lowercase();
    let cm = camera_mount.trim().to_ascii_lowercase();
    if lm == cm {
        return true;
    }
    match (lm.as_str(), cm.as_str()) {
        ("sony fe", "sony e") | ("sony e", "sony fe") => true,
        ("canon ef", "canon ef-s") => true,
        ("nikon fx", "nikon dx") => true,
        ("leica l", "leica l") => true,
        _ => false,
    }
}

pub fn normalize_camera_name(name: &str) -> String {
    name.to_ascii_lowercase()
        .replace([' ', '-', '_', '.', '/'], "")
        .replace("blackmagicpocketcinemacamera", "bmpcc")
        .replace("pocketcinemacamera", "bmpcc")
        .replace("blackmagic", "")
        .replace("black", "")
        .replace("edition", "")
        .replace("lumix", "")
        .replace("osmo", "")
}

pub fn are_cameras_compatible(brand1: &str, model1: &str, brand2: &str, model2: &str) -> bool {
    let b1 = brand1.trim().to_ascii_lowercase();
    let b2 = brand2.trim().to_ascii_lowercase();

    let is_fpv_cross = (b1.contains("runcam") || b1.contains("caddx")) && (b2.contains("runcam") || b2.contains("caddx"));

    if !b1.is_empty() && !b2.is_empty() && !b1.contains(&b2) && !b2.contains(&b1) && !is_fpv_cross {
        return false;
    }

    let m1 = normalize_camera_name(model1);
    let m2 = normalize_camera_name(model2);

    if m1 == m2 || (!m1.is_empty() && !m2.is_empty() && (m1.contains(&m2) || m2.contains(&m1))) {
        return true;
    }

    const CLUSTERS: &[&[&str]] = &[
        // Sony Full Frame
        &["fx3", "a7siii", "a7s3", "a7iv", "a74", "a1", "fx6", "a7cii", "a7c2", "a7r5", "a7rv"],
        // Sony APS-C
        &["fx30", "a6700", "zve10ii", "a6600", "a6500", "a6400", "a6100", "zve10"],
        // Sony 1-inch
        &["zv1", "zv1ii", "rx100vii", "rx100vi"],
        // GoPro Hero 11 / 12 / 13 (identical sensor and wide optics)
        &["hero13", "hero12", "hero11", "hero11mini"],
        // GoPro Hero 9 / 10
        &["hero10", "hero9"],
        // GoPro Hero 6 / 7 / 8
        &["hero8", "hero7", "hero6"],
        // BMPCC 6K variants
        &["bmpcc6k", "bmpcc6kpro", "bmpcc6kg2"],
        // BMPCC 4K variants
        &["bmpcc4k", "microcinemacamera4k"],
        // Blackmagic Cinema Camera 6K / Pyxis
        &["cinemacamera6k", "pyxis6k"],
        // Panasonic Full Frame
        &["s5", "s5ii", "s5iix", "s1h", "s1"],
        // Panasonic MFT
        &["gh6", "gh7", "gh5", "gh5s", "bgh1"],
        // DJI Action 4 & 5 Pro
        &["action4", "action5pro"],
        // DJI Action 2 / 3 / O3 / Avata
        &["action3", "action2", "o3airunit", "avata"],
        // Insta360 Ace
        &["acepro", "acepro2"],
        // Insta360 ONE R / RS
        &["oners4kboost", "oners", "oner4k", "oner"],
        // Insta360 X series
        &["x3", "x4"],
        // Nikon Z
        &["z8", "z9", "z6iii", "z6ii", "zf"],
        // Fujifilm APS-C
        &["xt5", "xh2", "xt4", "xs20", "xt50"],
        &["xh2s"],
        // RED
        &["komodo6k", "komodo", "komodox6k", "komodox"],
        &["vraptor8kvv", "vraptor"],
        // FPV
        &["thumbpro", "thumbprov2", "walnut"],
        &["peanut", "go2"],
    ];

    for cluster in CLUSTERS {
        let has_m1 = cluster.iter().any(|c| m1.contains(c) || c.contains(&m1.as_str()));
        let has_m2 = cluster.iter().any(|c| m2.contains(c) || c.contains(&m2.as_str()));
        if has_m1 && has_m2 {
            return true;
        }
    }

    false
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
#[serde(default)]
pub struct CameraIdentifier {
    pub brand: String,
    pub model: String,
    pub lens_model: String,
    pub lens_info: String,
    pub focal_length: Option<f64>,
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
                                        "M" => id.lens_info = "Medium".into(),
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
                            if let Some(v) = tag_map.get(&GroupId::Lens).and_then(|map| map.get_t(TagId::FocalLength) as Option<&f32>) {
                                id.lens_info = format!("{:.2} mm", v);
                                id.focal_length = Some(*v as f64);
                            }
                            if let Some(v) = tag_map.get(&GroupId::Lens).and_then(|map| map.get_t(TagId::DisplayName) as Option<&String>) {
                                id.lens_model = v.clone();
                            }
                            if let Some(v) = tag_map.get(&GroupId::Custom("LensDistortion".into())).and_then(|map| map.get_t(TagId::Data) as Option<&serde_json::Value>) {
                                if id.lens_info.is_empty() {
                                    let mut hasher = crc32fast::Hasher::new();
                                    // use previous json for hash to load previous lens profiles
                                    if let Some(v) = v.as_object() {
                                        if v.contains_key("focal_length_nm") {
                                            let s = serde_json::json!({
                                                "unk1": [v["focal_length_nm"], v["effective_sensor_height_nm"]],
                                                "unk2": v["unk1"],
                                                "unk3": v["coeff_scale"],
                                                "unk4": v["coeffs"]
                                            }).to_string();
                                            hasher.update(s.as_bytes());

                                            id.lens_info = format!("{:x}", hasher.finalize());
                                        }
                                    }
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
                            if let Some(v) = tag_map.get(&GroupId::Lens).and_then(|map| map.get_t(TagId::FocalLength) as Option<&f32>) {
                                id.lens_info = format!("{:.2} mm", v);
                                id.focal_length = Some(*v as f64);
                            }
                            if brand != "Runcam" {
                                if let Some(v) = tag_map.get(&GroupId::Lens).and_then(|map| map.get_t(TagId::Name) as Option<&String>) {
                                    id.lens_model = v.clone();
                                }
                            }
                            if let Some(map) = tag_map.get(&GroupId::Default) {
                                if let Some(v) = map.get_t(TagId::Metadata) as Option<&serde_json::Value> {
                                    log::debug!("Camera ID Brand: {}, Model: {}, Metadata: {:?}", id.brand, id.model, v);
                                    if let Some(v) = v.get("lens_info")             .and_then(|v| v.as_str()) { id.lens_info      = v.to_string(); }
                                    if let Some(v) = v.get("focal_length")          .and_then(|v| v.as_f64()) { id.lens_info      = format!("{:.2}mm", v); id.focal_length = Some(v); }
                                    if let Some(v) = v.get("focal_length")          .and_then(|v| v.as_str()) { id.lens_info      = v.to_string(); id.focal_length = v.replace("mm", "").parse::<f64>().ok(); }
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

        log::debug!("{:#?}", id);

        Ok(id)
    }

    pub fn get_identifier_for_autoload(&self) -> String {
        self.identifier.replace("hero12", "hero11")
                       .replace("hero13", "hero11")
                       .replace("hero11blackmini", "hero11black")
    }

    fn get_identifier(&self) -> String {
        if self.brand.is_empty() || self.model.is_empty() || self.lens_info.is_empty() { return String::new(); }
        let fps = match self.brand.as_ref() {
            "RED" | "RED RAW" => 0, // RED doesn't do any sensor crop while maintaining the resolution, so we can skip fps
            _ => self.fps
        };

        let mut id = format!("{}-{}-{}-{}-{}x{}@{}-{}", self.brand, self.model, self.lens_model, self.lens_info, self.video_width, self.video_height, fps, self.additional);
        id = id.replace(' ', "");
        id = id.replace("--", "-");
        id = id.replace("--", "-");
        let x: &[_] = &['-', ' '];
        id.trim_matches(x).to_lowercase()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn is_profile_compatible(&self, profile_brand: &str, profile_model: &str, profile_lens: &str, _profile_crop: Option<f64>) -> bool {
        if !are_cameras_compatible(&self.brand, &self.model, profile_brand, profile_model) {
            return false;
        }
        if !self.lens_model.is_empty() && !profile_lens.is_empty() {
            let l1 = self.lens_model.to_ascii_lowercase().replace([' ', '-', '_', '.'], "");
            let l2 = profile_lens.to_ascii_lowercase().replace([' ', '-', '_', '.'], "");
            if !l1.contains(&l2) && !l2.contains(&l1) {
                return false;
            }
        }
        true
    }

    pub fn from_metadata_map(map: &HashMap<String, String>, video_width: usize, video_height: usize, fps: f64) -> Self {
        let get_any = |keys: &[&str]| -> Option<String> {
            for k in keys {
                for (mk, mv) in map {
                    if mk.eq_ignore_ascii_case(k) && !mv.trim().is_empty() {
                        return Some(mv.trim().to_string());
                    }
                }
            }
            None
        };

        let brand = get_any(&["make", "camera_make", "camera_brand", "manufacturer", "brand"]).unwrap_or_default();
        let model = get_any(&["model", "camera_model", "device_model", "camera"]).unwrap_or_default();
        let lens_model = get_any(&["lens_model", "lens", "lens_make_model", "lens_id"]).unwrap_or_default();
        let camera_setting = get_any(&["shooting_mode", "camera_setting", "resolution_name", "fov"]).unwrap_or_default();
        let focal_length = get_any(&["focal_length", "focallength", "focal_length_in_35mm_format"])
            .and_then(|v| v.replace("mm", "").trim().parse::<f64>().ok());

        Self::from_video_metadata(
            if brand.is_empty() { None } else { Some(&brand) },
            if model.is_empty() { None } else { Some(&model) },
            if lens_model.is_empty() { None } else { Some(&lens_model) },
            focal_length,
            if camera_setting.is_empty() { None } else { Some(&camera_setting) },
            video_width,
            video_height,
            fps,
        )
    }

    pub fn from_video_metadata(
        camera_make: Option<&str>,
        camera_model: Option<&str>,
        lens_model: Option<&str>,
        focal_length: Option<f64>,
        camera_setting: Option<&str>,
        video_width: usize,
        video_height: usize,
        fps: f64,
    ) -> Self {
        let mut brand = camera_make.unwrap_or_default().trim().to_string();
        let mut model = camera_model.unwrap_or_default().trim().to_string();
        let lens_model = lens_model.unwrap_or_default().trim().to_string();
        let camera_setting = camera_setting.unwrap_or_default().trim().to_string();
        let fps_rounded = (fps * 1000.0).round() as usize;

        if !brand.is_empty() && model.to_ascii_lowercase().starts_with(&brand.to_ascii_lowercase()) {
            model = model[brand.len()..].trim().to_string();
        }

        if brand.is_empty() {
            let m_lower = model.to_ascii_lowercase();
            if m_lower.starts_with("gopro") || m_lower.starts_with("hero") {
                brand = "GoPro".to_string();
            } else if m_lower.starts_with("sony") || m_lower.starts_with("ilce") || m_lower.starts_with("fx") {
                brand = "Sony".to_string();
            } else if m_lower.starts_with("canon") || m_lower.starts_with("eos") {
                brand = "Canon".to_string();
            } else if m_lower.starts_with("dji") || m_lower.starts_with("osmo") || m_lower.starts_with("action") {
                brand = "DJI".to_string();
            } else if m_lower.starts_with("insta360") {
                brand = "Insta360".to_string();
            } else if m_lower.starts_with("bmpcc") || m_lower.contains("blackmagic") {
                brand = "Blackmagic Design".to_string();
            } else if m_lower.starts_with("nikon") || m_lower.starts_with("z8") || m_lower.starts_with("z9") {
                brand = "Nikon".to_string();
            } else if m_lower.starts_with("fujifilm") || m_lower.starts_with("x-t") || m_lower.starts_with("x-h") {
                brand = "Fujifilm".to_string();
            } else if m_lower.starts_with("komodo") || m_lower.starts_with("raptor") {
                brand = "RED".to_string();
            }
        }

        let lens_info = if let Some(fl) = focal_length {
            format!("{:.1}mm", fl)
        } else {
            String::new()
        };

        let mut id = Self {
            brand,
            model,
            lens_model,
            lens_info,
            focal_length,
            camera_setting,
            fps: fps_rounded,
            video_width,
            video_height,
            additional: String::new(),
            identifier: String::new(),
        };
        id.identifier = id.get_identifier();
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_crop_factor() {
        let full_frame = compute_crop_factor(36.0, 24.0);
        assert!((full_frame - 1.0).abs() < 0.05);

        let apsc = compute_crop_factor(23.5, 15.6);
        assert!((apsc - 1.5).abs() < 0.1);

        let mft = compute_crop_factor(17.3, 13.0);
        assert!((mft - 2.0).abs() < 0.1);

        let one_inch = compute_crop_factor(13.2, 8.8);
        assert!((one_inch - 2.7).abs() < 0.1);
    }

    #[test]
    fn test_are_cameras_compatible() {
        // Sony Full Frame cluster
        assert!(are_cameras_compatible("Sony", "FX3", "Sony", "A7S III"));
        assert!(are_cameras_compatible("Sony", "FX3", "Sony", "A7 IV"));
        assert!(are_cameras_compatible("Sony", "FX30", "Sony", "A6700"));
        assert!(!are_cameras_compatible("Sony", "FX3", "Sony", "FX30")); // FF vs APS-C

        // GoPro cluster
        assert!(are_cameras_compatible("GoPro", "HERO13 Black", "GoPro", "HERO11 Black"));
        assert!(are_cameras_compatible("GoPro", "HERO12 Black", "GoPro", "HERO11 Black Mini"));
        assert!(!are_cameras_compatible("GoPro", "HERO11 Black", "GoPro", "HERO10 Black"));

        // Blackmagic 6K cluster
        assert!(are_cameras_compatible("Blackmagic Design", "Pocket Cinema Camera 6K", "Blackmagic Design", "Pocket Cinema Camera 6K Pro"));
        assert!(are_cameras_compatible("Blackmagic Design", "BMPCC 6K", "Blackmagic Design", "BMPCC 6K G2"));

        // DJI Action cluster
        assert!(are_cameras_compatible("DJI", "Osmo Action 4", "DJI", "Osmo Action 5 Pro"));
        assert!(are_cameras_compatible("DJI", "Action 2", "DJI", "Osmo Action 3"));

        // FPV cross brand
        assert!(are_cameras_compatible("RunCam", "Thumb Pro", "Caddx", "Walnut"));
    }

    #[test]
    fn test_are_mounts_compatible() {
        assert!(are_mounts_compatible("Sony E", "Sony E"));
        assert!(are_mounts_compatible("Sony FE", "Sony E"));
        assert!(are_mounts_compatible("Canon EF", "Canon EF-S"));
        assert!(!are_mounts_compatible("Sony E", "Canon RF"));
    }

    #[test]
    fn test_from_metadata_map() {
        let mut map = HashMap::new();
        map.insert("make".to_string(), "Sony".to_string());
        map.insert("model".to_string(), "ILCE-7SM3".to_string());
        map.insert("lens_model".to_string(), "FE 24-70mm F2.8 GM II".to_string());
        map.insert("focal_length".to_string(), "24.0mm".to_string());

        let id = CameraIdentifier::from_metadata_map(&map, 3840, 2160, 59.94);
        assert_eq!(id.brand, "Sony");
        assert_eq!(id.model, "ILCE-7SM3");
        assert_eq!(id.lens_model, "FE 24-70mm F2.8 GM II");
        assert_eq!(id.focal_length, Some(24.0));
        assert_eq!(id.video_width, 3840);
        assert_eq!(id.video_height, 2160);
        assert_eq!(id.fps, 59940);
    }

    #[test]
    fn test_is_profile_compatible() {
        let id = CameraIdentifier::from_video_metadata(
            Some("Sony"),
            Some("FX3"),
            Some("FE 24-70mm F2.8 GM II"),
            Some(24.0),
            None,
            3840,
            2160,
            60.0,
        );

        // Compatible Sony A7S III with same lens
        assert!(id.is_profile_compatible("Sony", "A7S III", "FE 24-70mm F2.8 GM II", None));
        // Incompatible camera
        assert!(!id.is_profile_compatible("Canon", "EOS R5", "RF 24-70mm F2.8", None));
    }
}

