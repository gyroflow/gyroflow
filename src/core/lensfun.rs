// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright Ãƒâ€šÃ‚Â© 2024 Adrian <adrian.eddy at gmail>
// Lensfun XML database parser and converter for Gyroflow

use std::io::Read;
use serde::{Deserialize, Serialize};
use xml::reader::{EventReader, XmlEvent};

use crate::LensProfile;
use crate::lens_profile::{CameraParams, Dimensions};
use crate::stabilization::distortion_models::{PtLens, Poly3, Poly5};
use crate::camera_database::{CameraDatabase, CameraInfo, LensInfo, MountInfo};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LensfunCamera {
    pub maker: String,
    pub model: String,
    pub mount: String,
    pub crop_factor: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LensfunDistortion {
    pub model: String, // "ptlens", "poly3", "poly5"
    pub focal: f64,
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub k1: f64,
    pub k2: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LensfunLens {
    pub maker: String,
    pub model: String,
    pub mounts: Vec<String>,
    pub crop_factor: f64,
    pub aspect_ratio: String,
    pub min_focal: Option<f64>,
    pub max_focal: Option<f64>,
    pub distortions: Vec<LensfunDistortion>,
}

#[derive(Debug, Clone, Default)]
pub struct LensfunDatabase {
    pub mounts: Vec<MountInfo>,
    pub cameras: Vec<LensfunCamera>,
    pub lenses: Vec<LensfunLens>,
}

pub fn parse_aspect_ratio(ar: &str) -> Option<f64> {
    let parts: Vec<&str> = ar.split(':').collect();
    if parts.len() == 2 {
        let w = parts[0].trim().parse::<f64>().ok()?;
        let h = parts[1].trim().parse::<f64>().ok()?;
        if h > 0.0 {
            return Some(w / h);
        }
    }
    None
}

impl LensfunDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse Lensfun XML data
    pub fn parse_xml<R: Read>(&mut self, reader: R) -> Result<(), String> {
        let parser = EventReader::new(reader);

        let mut current_element = String::new();
        let mut in_camera = false;
        let mut in_lens = false;
        let mut in_mount = false;
        let mut in_calibration = false;

        let mut curr_camera = LensfunCamera::default();
        let mut curr_lens = LensfunLens::default();
        let mut curr_mount = MountInfo::default();
        let mut curr_compat_mount = String::new();

        for e in parser {
            match e {
                Ok(XmlEvent::StartElement { name, attributes, .. }) => {
                    current_element = name.local_name.clone();
                    match current_element.as_str() {
                        "camera" => {
                            in_camera = true;
                        },
                        "lens" => {
                            in_lens = true;
                        },
                        "mount" => {
                            in_mount = true;
                            curr_mount = MountInfo::default();
                            curr_compat_mount.clear();
                        },
                        "calibration" => {
                            in_calibration = true;
                        },
                        "distortion" if in_lens && in_calibration => {
                            let mut dist = LensfunDistortion::default();
                            for attr in attributes {
                                match attr.name.local_name.as_str() {
                                    "model" => dist.model = attr.value,
                                    "focal" => dist.focal = attr.value.parse().unwrap_or(0.0),
                                    "a"     => dist.a = attr.value.parse().unwrap_or(0.0),
                                    "b"     => dist.b = attr.value.parse().unwrap_or(0.0),
                                    "c"     => dist.c = attr.value.parse().unwrap_or(0.0),
                                    "k1"    => dist.k1 = attr.value.parse().unwrap_or(0.0),
                                    "k2"    => dist.k2 = attr.value.parse().unwrap_or(0.0),
                                    _ => {}
                                }
                            }
                            if !dist.model.is_empty() && dist.focal > 0.0 {
                                curr_lens.distortions.push(dist);
                            }
                        },
                        _ => {}
                    }
                },
                Ok(XmlEvent::Characters(content)) => {
                    let text = content.trim().to_string();
                    if text.is_empty() { continue; }

                    if in_camera {
                        match current_element.as_str() {
                            "maker" => curr_camera.maker = text,
                            "model" => if curr_camera.model.is_empty() { curr_camera.model = text; },
                            "mount" => curr_camera.mount = text,
                            "cropfactor" => curr_camera.crop_factor = text.parse().unwrap_or(1.0),
                            _ => {}
                        }
                    } else if in_lens {
                        match current_element.as_str() {
                            "maker" => curr_lens.maker = text,
                            "model" => curr_lens.model = text,
                            "mount" => curr_lens.mounts.push(text),
                            "cropfactor" => curr_lens.crop_factor = text.parse().unwrap_or(1.0),
                            "aspect-ratio" => curr_lens.aspect_ratio = text,
                            "min-focal" => curr_lens.min_focal = text.parse().ok(),
                            "max-focal" => curr_lens.max_focal = text.parse().ok(),
                            _ => {}
                        }
                    } else if in_mount {
                        match current_element.as_str() {
                            "name" => curr_mount.name = text,
                            "compat" => curr_compat_mount = text,
                            _ => {}
                        }
                    }
                },
                Ok(XmlEvent::EndElement { name }) => {
                    match name.local_name.as_str() {
                        "camera" => {
                            if !curr_camera.maker.is_empty() && !curr_camera.model.is_empty() {
                                self.cameras.push(curr_camera.clone());
                            }
                            in_camera = false;
                        },
                        "lens" => {
                            if !curr_lens.maker.is_empty() && !curr_lens.model.is_empty() {
                                self.lenses.push(curr_lens.clone());
                            }
                            in_lens = false;
                        },
                        "mount" => {
                            if !curr_mount.name.is_empty() {
                                curr_mount.id = curr_mount.name.to_ascii_lowercase().replace(' ', "_");
                                if !curr_compat_mount.is_empty() && curr_compat_mount != curr_mount.name {
                                    curr_mount.aliases.push(curr_compat_mount.clone());
                                }
                                self.mounts.push(curr_mount.clone());
                            }
                            in_mount = false;
                        },
                        "calibration" => {
                            in_calibration = false;
                        },
                        _ => {}
                    }
                    current_element.clear();
                },
                Err(e) => return Err(format!("XML parse error: {:?}", e)),
                _ => {}
            }
        }

        Ok(())
    }

    /// Import all cameras and lenses into the CameraDatabase
    pub fn populate_camera_database(&self, db: &mut CameraDatabase) {
        for m in &self.mounts {
            if !db.mounts.iter().any(|x| x.name.eq_ignore_ascii_case(&m.name)) {
                db.mounts.push(m.clone());
            }
        }

        for c in &self.cameras {
            let (sensor_w, sensor_h) = if c.crop_factor > 0.0 {
                (36.0 / c.crop_factor, 24.0 / c.crop_factor)
            } else {
                (36.0, 24.0)
            };
            let info = CameraInfo {
                brand: c.maker.clone(),
                model: c.model.clone(),
                mount: if !c.mount.is_empty() { Some(c.mount.clone()) } else { None },
                sensor_type: None,
                crop_factor: Some(c.crop_factor),
                sensor_width: Some(sensor_w),
                sensor_height: Some(sensor_h),
                aliases: Vec::new(),
            };
            db.add_camera(info);
        }

        for l in &self.lenses {
            let info = LensInfo {
                brand: l.maker.clone(),
                model: l.model.clone(),
                mount: l.mounts.first().cloned(),
                sensor_coverage: None,
                crop_factor: Some(l.crop_factor),
                min_focal: l.min_focal,
                max_focal: l.max_focal,
                aspect_ratio: if !l.aspect_ratio.is_empty() { Some(l.aspect_ratio.clone()) } else { None },
                aliases: Vec::new(),
            };
            db.add_lens(info);
        }

        db.rebuild_indices();
    }

    /// Convert all calibrations in the database into Gyroflow LensProfiles
    pub fn to_lens_profiles(&self) -> Vec<LensProfile> {
        let mut profiles = Vec::new();

        for lens in &self.lenses {
            if lens.distortions.is_empty() { continue; }

            // Create individual profile for each focal calibration
            let mut focal_profiles = Vec::new();
            for dist in &lens.distortions {
                if let Some(prof) = Self::distortion_to_profile(lens, dist, None) {
                    focal_profiles.push(prof);
                }
            }

            if focal_profiles.is_empty() { continue; }

            // If lens has multiple focal lengths, create an interpolated master profile
            if focal_profiles.len() > 1 {
                let mut master = focal_profiles[0].clone();
                master.name = format!("{} {}", lens.maker, lens.model);
                master.camera_setting = format!("{:.0}-{:.0}mm",
                    lens.min_focal.unwrap_or(lens.distortions.first().map(|x| x.focal).unwrap_or(0.0)),
                    lens.max_focal.unwrap_or(lens.distortions.last().map(|x| x.focal).unwrap_or(0.0))
                );

                let mut interp_map = serde_json::Map::new();
                for prof in &focal_profiles {
                    if let Some(focal) = prof.focal_length {
                        let obj = serde_json::json!({
                            "distortion_coeffs": prof.fisheye_params.distortion_coeffs,
                            "identifier": prof.identifier
                        });
                        interp_map.insert(format!("{:.1}", focal), obj);
                    }
                }
                master.interpolations = Some(serde_json::Value::Object(interp_map));
                let identifier = format!("{}-{}-{}", master.camera_brand, master.camera_model, master.lens_model)
                    .to_ascii_lowercase().replace(' ', "-");
                master.identifier = identifier.clone();
                master.checksum = Some(format!("{:08x}", crc32fast::hash(identifier.as_bytes())));

                profiles.push(master);
            }

            profiles.extend(focal_profiles);
        }

        profiles
    }

    /// Convert a single Lensfun distortion calibration into a Gyroflow LensProfile
    pub fn distortion_to_profile(lens: &LensfunLens, dist: &LensfunDistortion, camera: Option<&LensfunCamera>) -> Option<LensProfile> {
        let crop_factor = if let Some(cam) = camera {
            if cam.crop_factor > 0.0 { cam.crop_factor } else { lens.crop_factor }
        } else if lens.crop_factor > 0.0 {
            lens.crop_factor
        } else {
            1.0
        };

        let ar_str = if !lens.aspect_ratio.is_empty() { &lens.aspect_ratio } else { "3:2" };
        let aspect_ratio_val = parse_aspect_ratio(ar_str).unwrap_or(1.5);

        // Standard 4K calibration dimension
        let calib_w = 3840;
        let calib_h = ((calib_w as f64) / aspect_ratio_val).round() as usize;

        // Diagonal of 35mm full frame sensor is sqrt(36^2 + 24^2) mm
        let diag_full_frame = (36.0_f64).hypot(24.0);
        let hugin_scale_in_millimeters = diag_full_frame / crop_factor / aspect_ratio_val.hypot(1.0) / 2.0;

        let model_type = dist.model.to_ascii_lowercase();
        let (coeffs, hugin_scaling) = match model_type.as_str() {
            "ptlens" => {
                let real_focal = dist.focal * (1.0 - dist.a - dist.b - dist.c);
                let scaling = real_focal / hugin_scale_in_millimeters;
                let mut k = vec![dist.a, dist.b, dist.c];
                PtLens::rescale_coeffs(&mut k, scaling);
                (k, scaling)
            },
            "poly3" => {
                let real_focal = dist.focal * (1.0 - dist.k1);
                let scaling = real_focal / hugin_scale_in_millimeters;
                let mut k = vec![dist.k1];
                Poly3::rescale_coeffs(&mut k, scaling);
                (k, scaling)
            },
            "poly5" => {
                let real_focal = dist.focal;
                let scaling = real_focal / hugin_scale_in_millimeters;
                let mut k = vec![dist.k1, dist.k2];
                Poly5::rescale_coeffs(&mut k, scaling);
                (k, scaling)
            },
            _ => return None,
        };

        let fx = hugin_scaling * (calib_h as f64 / 2.0);
        let fy = fx;
        let cx = calib_w as f64 / 2.0;
        let cy = calib_h as f64 / 2.0;

        let camera_matrix = vec![
            [fx, 0.0, cx],
            [0.0, fy, cy],
            [0.0, 0.0, 1.0],
        ];

        let cam_brand = if let Some(cam) = camera { cam.maker.clone() } else { lens.maker.clone() };
        let cam_model = if let Some(cam) = camera { cam.model.clone() } else { "Lensfun".to_string() };

        let name = format!("{} {} ({:.0}mm)", lens.maker, lens.model, dist.focal);
        let mut profile = LensProfile::default();
        profile.name = name.clone();
        profile.note = "Imported from Lensfun database".to_string();
        profile.calibrated_by = "Lensfun".to_string();
        profile.camera_brand = cam_brand.clone();
        profile.camera_model = cam_model.clone();
        profile.lens_model = lens.model.clone();
        profile.camera_setting = format!("{:.0}mm", dist.focal);
        profile.calib_dimension = Dimensions { w: calib_w, h: calib_h };
        profile.orig_dimension = Dimensions { w: calib_w, h: calib_h };
        profile.calibrator_version = "Lensfun".to_string();
        profile.official = true;
        profile.distortion_model = Some(model_type);
        profile.focal_length = Some(dist.focal);
        profile.crop_factor = Some(crop_factor);
        profile.fisheye_params = CameraParams {
            RMS_error: 0.0,
            camera_matrix,
            distortion_coeffs: coeffs,
            radial_distortion_limit: None,
        };

        profile.init();

        let identifier = format!("{}-{}-{}-{:.0}mm-{}x{}",
            profile.camera_brand, profile.camera_model, profile.lens_model, dist.focal, calib_w, calib_h
        ).to_ascii_lowercase().replace(' ', "-");
        profile.identifier = identifier.clone();
        profile.checksum = Some(format!("{:08x}", crc32fast::hash(identifier.as_bytes())));

        Some(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lensfun_xml_parsing_and_conversion() {
        let xml = r#"
        <lensdatabase version="1">
            <mount>
                <name>Sony E</name>
                <compat>Sony E</compat>
            </mount>
            <camera>
                <maker>Sony</maker>
                <model>ILCE-7M3</model>
                <mount>Sony E</mount>
                <cropfactor>1.0</cropfactor>
            </camera>
            <lens>
                <maker>Sony</maker>
                <model>FE 24-70mm F2.8 GM</model>
                <mount>Sony E</mount>
                <cropfactor>1.0</cropfactor>
                <aspect-ratio>3:2</aspect-ratio>
                <calibration>
                    <distortion model="ptlens" focal="24" a="0.005" b="-0.015" c="0.02" />
                    <distortion model="poly3" focal="35" k1="-0.008" />
                    <distortion model="poly5" focal="70" k1="-0.005" k2="0.001" />
                </calibration>
            </lens>
        </lensdatabase>
        "#;

        let mut lf = LensfunDatabase::new();
        lf.parse_xml(std::io::Cursor::new(xml)).unwrap();

        assert_eq!(lf.mounts.len(), 1);
        assert_eq!(lf.cameras.len(), 1);
        assert_eq!(lf.lenses.len(), 1);

        let profiles = lf.to_lens_profiles();
        assert_eq!(profiles.len(), 4);

        let ptlens_prof = profiles.iter().find(|p| p.distortion_model.as_deref() == Some("ptlens")).unwrap();
        assert_eq!(ptlens_prof.camera_brand, "Sony");
        assert_eq!(ptlens_prof.lens_model, "FE 24-70mm F2.8 GM");
        assert_eq!(ptlens_prof.fisheye_params.camera_matrix.len(), 3);
        assert_eq!(ptlens_prof.fisheye_params.distortion_coeffs.len(), 3);
        assert!(ptlens_prof.fisheye_params.camera_matrix[0][0] > 0.0);

        let poly3_prof = profiles.iter().find(|p| p.distortion_model.as_deref() == Some("poly3")).unwrap();
        assert_eq!(poly3_prof.fisheye_params.distortion_coeffs.len(), 1);

        let poly5_prof = profiles.iter().find(|p| p.distortion_model.as_deref() == Some("poly5")).unwrap();
        assert_eq!(poly5_prof.fisheye_params.distortion_coeffs.len(), 2);

        let master = profiles.iter().find(|p| p.interpolations.is_some()).unwrap();
        assert!(master.interpolations.is_some());
    }
}
