// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Gyroflow contributors

use std::collections::HashMap;
use std::io::Read;

use xml::attribute::OwnedAttribute;
use xml::reader::{ EventReader, XmlEvent };

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LensfunDatabase {
    pub cameras: Vec<LensfunCamera>,
    pub lenses: Vec<LensfunLens>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LensfunCamera {
    pub maker: String,
    pub model: String,
    pub mount: String,
    pub cropfactor: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LensfunLens {
    pub maker: String,
    pub model: String,
    pub mounts: Vec<String>,
    pub cropfactor: Option<f64>,
    pub distortions: Vec<LensfunDistortion>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LensfunDistortion {
    pub focal: f64,
    pub model: String,
    pub coefficients: Vec<f64>,
}

impl LensfunDatabase {
    pub fn from_reader<R: Read>(reader: R) -> Result<Self, xml::reader::Error> {
        let parser = EventReader::new(reader);
        let mut db = LensfunDatabase::default();
        let mut stack = Vec::<String>::new();
        let mut text = String::new();
        let mut camera: Option<LensfunCamera> = None;
        let mut lens: Option<LensfunLens> = None;

        for event in parser {
            match event? {
                XmlEvent::StartElement { name, attributes, .. } => {
                    text.clear();
                    let tag = name.local_name;
                    match tag.as_str() {
                        "camera" => camera = Some(LensfunCamera::default()),
                        "lens" => lens = Some(LensfunLens::default()),
                        "distortion" => {
                            if in_lens_calibration(&stack) {
                                if let Some(distortion) = LensfunDistortion::from_attributes(&attributes) {
                                    if let Some(lens) = lens.as_mut() {
                                        lens.distortions.push(distortion);
                                    }
                                }
                            }
                        },
                        _ => { }
                    }
                    stack.push(tag);
                },
                XmlEvent::Characters(value) | XmlEvent::CData(value) => {
                    text.push_str(&value);
                },
                XmlEvent::EndElement { name } => {
                    let tag = name.local_name;
                    let value = text.trim();

                    if let Some(lens) = lens.as_mut() {
                        match tag.as_str() {
                            "maker" => lens.maker = value.to_owned(),
                            "model" => lens.model = value.to_owned(),
                            "mount" if !value.is_empty() => lens.mounts.push(value.to_owned()),
                            "cropfactor" => lens.cropfactor = parse_f64(value),
                            _ => { }
                        }
                    } else if let Some(camera) = camera.as_mut() {
                        match tag.as_str() {
                            "maker" => camera.maker = value.to_owned(),
                            "model" => camera.model = value.to_owned(),
                            "mount" => camera.mount = value.to_owned(),
                            "cropfactor" => camera.cropfactor = parse_f64(value),
                            _ => { }
                        }
                    }

                    match tag.as_str() {
                        "camera" => {
                            if let Some(camera) = camera.take() {
                                if !camera.model.is_empty() {
                                    db.cameras.push(camera);
                                }
                            }
                        },
                        "lens" => {
                            if let Some(lens) = lens.take() {
                                if !lens.model.is_empty() {
                                    db.lenses.push(lens);
                                }
                            }
                        },
                        _ => { }
                    }

                    stack.pop();
                    text.clear();
                },
                _ => { }
            }
        }

        Ok(db)
    }

    pub fn from_xml_str(xml: &str) -> Result<Self, xml::reader::Error> {
        Self::from_reader(xml.as_bytes())
    }

    pub fn lens_metadata(&self) -> Vec<LensfunLensMetadata> {
        self.lenses
            .iter()
            .map(|lens| LensfunLensMetadata {
                maker: lens.maker.clone(),
                model: lens.model.clone(),
                mounts: lens.mounts.clone(),
                cropfactor: lens.cropfactor,
                focal_lengths: lens.distortions.iter().map(|x| x.focal).collect(),
                distortion_models: lens.distortions.iter().map(|x| x.model.clone()).collect(),
            })
            .collect()
    }

    pub fn extend(&mut self, other: Self) {
        self.cameras.extend(other.cameras);
        self.lenses.extend(other.lenses);
    }

    pub fn lenses_for_mount<'a>(&'a self, mount: &str) -> Vec<&'a LensfunLens> {
        self.lenses.iter().filter(|lens| lens.supports_mount(mount)).collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LensfunLensMetadata {
    pub maker: String,
    pub model: String,
    pub mounts: Vec<String>,
    pub cropfactor: Option<f64>,
    pub focal_lengths: Vec<f64>,
    pub distortion_models: Vec<String>,
}

impl LensfunDistortion {
    fn from_attributes(attributes: &[OwnedAttribute]) -> Option<Self> {
        let attrs = attributes
            .iter()
            .map(|attr| (attr.name.local_name.as_str(), attr.value.as_str()))
            .collect::<HashMap<_, _>>();

        let model = attrs.get("model")?.to_string();
        let focal = parse_f64(attrs.get("focal").copied().unwrap_or_default())?;
        let coefficients = match model.as_str() {
            "poly3" => vec![parse_attr(&attrs, "k1")?],
            "poly5" => vec![parse_attr(&attrs, "k1")?, parse_attr(&attrs, "k2")?],
            "ptlens" => vec![parse_attr(&attrs, "a")?, parse_attr(&attrs, "b")?, parse_attr(&attrs, "c")?],
            "none" => Vec::new(),
            _ => return None,
        };

        Some(Self { focal, model, coefficients })
    }

    pub fn is_supported(&self) -> bool {
        matches!(self.model.as_str(), "poly3" | "poly5" | "ptlens")
    }
}

impl LensfunLens {
    pub fn supports_mount(&self, mount: &str) -> bool {
        self.mounts.iter().any(|x| x.eq_ignore_ascii_case(mount))
    }

    pub fn display_name(&self) -> String {
        [self.maker.trim(), self.model.trim()]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn supported_focal_lengths(&self) -> Vec<f64> {
        self.distortions
            .iter()
            .filter(|x| x.is_supported())
            .map(|x| x.focal)
            .collect()
    }
}

fn in_lens_calibration(stack: &[String]) -> bool {
    stack.iter().any(|x| x == "lens") && stack.iter().any(|x| x == "calibration")
}

fn parse_attr(attrs: &HashMap<&str, &str>, key: &str) -> Option<f64> {
    parse_f64(attrs.get(key).copied().unwrap_or("0"))
}

fn parse_f64(value: &str) -> Option<f64> {
    value.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lensfun_cameras_lenses_and_distortion_entries() {
        let xml = r#"
            <lensdatabase>
                <camera>
                    <maker>Sony</maker>
                    <model>ILCE-7SM3</model>
                    <mount>Sony E</mount>
                    <cropfactor>1.0</cropfactor>
                </camera>
                <lens>
                    <maker>Sony</maker>
                    <model>FE 24mm F1.4 GM</model>
                    <mount>Sony E</mount>
                    <cropfactor>1.0</cropfactor>
                    <calibration>
                        <distortion model="ptlens" focal="24" a="0.012" b="-0.036" c="0.004" />
                        <distortion model="poly3" focal="35" k1="-0.015" />
                        <distortion model="acm" focal="50" k1="1" />
                    </calibration>
                </lens>
            </lensdatabase>
        "#;

        let db = LensfunDatabase::from_xml_str(xml).unwrap();

        assert_eq!(db.cameras.len(), 1);
        assert_eq!(db.cameras[0].maker, "Sony");
        assert_eq!(db.cameras[0].mount, "Sony E");
        assert_eq!(db.lenses.len(), 1);
        assert_eq!(db.lenses[0].model, "FE 24mm F1.4 GM");
        assert_eq!(db.lenses[0].distortions.len(), 2);
        assert_eq!(db.lenses[0].distortions[0].model, "ptlens");
        assert_eq!(db.lenses[0].distortions[0].coefficients, vec![0.012, -0.036, 0.004]);
        assert_eq!(db.lenses[0].distortions[1].coefficients, vec![-0.015]);
    }

    #[test]
    fn creates_lens_metadata_for_selectors() {
        let xml = r#"
            <lensdatabase>
                <lens>
                    <maker>Canon</maker>
                    <model>RF 15-35mm F2.8</model>
                    <mount>Canon RF</mount>
                    <calibration>
                        <distortion model="poly5" focal="15" k1="-0.02" k2="0.003" />
                    </calibration>
                </lens>
            </lensdatabase>
        "#;

        let db = LensfunDatabase::from_xml_str(xml).unwrap();
        let metadata = db.lens_metadata();

        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].maker, "Canon");
        assert_eq!(metadata[0].mounts, vec!["Canon RF"]);
        assert_eq!(metadata[0].focal_lengths, vec![15.0]);
        assert_eq!(metadata[0].distortion_models, vec!["poly5"]);
    }

    #[test]
    fn finds_lenses_for_matching_mounts() {
        let xml = r#"
            <lensdatabase>
                <lens>
                    <maker>Sony</maker>
                    <model>FE 24mm F1.4 GM</model>
                    <mount>Sony E</mount>
                    <calibration>
                        <distortion model="poly3" focal="24" k1="-0.01" />
                    </calibration>
                </lens>
                <lens>
                    <maker>Canon</maker>
                    <model>RF 24mm F1.8</model>
                    <mount>Canon RF</mount>
                    <calibration>
                        <distortion model="poly3" focal="24" k1="-0.01" />
                    </calibration>
                </lens>
            </lensdatabase>
        "#;

        let db = LensfunDatabase::from_xml_str(xml).unwrap();
        let lenses = db.lenses_for_mount("sony e");

        assert_eq!(lenses.len(), 1);
        assert_eq!(lenses[0].display_name(), "Sony FE 24mm F1.4 GM");
        assert_eq!(lenses[0].supported_focal_lengths(), vec![24.0]);
    }
}
