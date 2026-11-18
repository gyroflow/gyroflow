// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022-2026 Adrian <adrian.eddy at gmail>

//! Lensfun XML database parser, Hugin polynomial coefficient rescaling, and `LensProfile` converter (#150).

use crate::lens_profile::{CameraParams, Dimensions, LensProfile};
use crate::stabilization::distortion_models::{poly3::Poly3, poly5::Poly5, ptlens::PtLens};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LensfunDistortionModel {
    Poly3,
    Poly5,
    PtLens,
}

impl LensfunDistortionModel {
    pub fn from_attr(model: &str) -> Option<Self> {
        match model.trim().to_ascii_lowercase().as_str() {
            "poly3" => Some(Self::Poly3),
            "poly5" => Some(Self::Poly5),
            "ptlens" => Some(Self::PtLens),
            _ => None,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Poly3 => Poly3::id(),
            Self::Poly5 => Poly5::id(),
            Self::PtLens => PtLens::id(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LensfunCalibration {
    pub model: LensfunDistortionModel,
    pub focal: f64,
    pub real_focal: Option<f64>,
    pub coeffs: [f64; 3],
}

impl LensfunCalibration {
    /// Computes the effective (real) focal length in millimeters according to liblensfun's rules.
    pub fn effective_focal(&self) -> f64 {
        self.real_focal.unwrap_or_else(|| match self.model {
            LensfunDistortionModel::PtLens => {
                self.focal * (1.0 - self.coeffs[0] - self.coeffs[1] - self.coeffs[2])
            }
            LensfunDistortionModel::Poly3 => self.focal * (1.0 - self.coeffs[0]),
            LensfunDistortionModel::Poly5 => self.focal,
        })
    }

    /// Computes Hugin coordinate normalization scaling (`real_focal / hugin_scale_in_mm`) and rescales
    /// polynomial coefficients into Gyroflow's `z = 1` plane convention.
    pub fn rescaled_coeffs(&self, crop_factor: f64, aspect_ratio: f64) -> Vec<f64> {
        let crop = if crop_factor > 0.0 { crop_factor } else { 1.0 };
        let aspect = if aspect_ratio > 0.0 { aspect_ratio } else { 3.0 / 2.0 };
        let real_focal = self.effective_focal();
        let hugin_scale_in_mm = 36.0_f64.hypot(24.0) / crop / aspect.hypot(1.0) / 2.0;
        let hugin_scaling = real_focal / hugin_scale_in_mm;

        match self.model {
            LensfunDistortionModel::Poly3 => {
                let mut k = vec![self.coeffs[0], 0.0, 0.0, 0.0];
                Poly3::rescale_coeffs(&mut k, hugin_scaling);
                k
            }
            LensfunDistortionModel::Poly5 => {
                let mut k = vec![self.coeffs[0], self.coeffs[1], 0.0, 0.0];
                Poly5::rescale_coeffs(&mut k, hugin_scaling);
                k
            }
            LensfunDistortionModel::PtLens => {
                let mut k = vec![self.coeffs[0], self.coeffs[1], self.coeffs[2], 0.0];
                PtLens::rescale_coeffs(&mut k, hugin_scaling);
                k
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct LensfunCamera {
    pub maker: String,
    pub model: String,
    pub mount: String,
    pub crop_factor: f64,
}

#[derive(Clone, Debug, Default)]
pub struct LensfunLens {
    pub maker: String,
    pub model: String,
    pub mounts: Vec<String>,
    pub crop_factor: f64,
    pub aspect_ratio: f64,
    pub calibrations: Vec<LensfunCalibration>,
}

impl LensfunLens {
    /// Checks whether this lens calibration sensor covers a camera with `target_crop_factor`
    /// (with Lensfun's 0.96 tolerance multiplier).
    pub fn supports_crop_factor(&self, target_crop_factor: f64) -> bool {
        if self.crop_factor <= 0.0 || target_crop_factor <= 0.0 {
            return true;
        }
        self.crop_factor <= target_crop_factor * 1.0416667
    }

    /// Interpolates distortion calibration at `focal_mm` across calibrated focal entries.
    pub fn interpolate_calibration(&self, focal_mm: f64) -> Option<LensfunCalibration> {
        if self.calibrations.is_empty() {
            return None;
        }
        let mut sorted = self.calibrations.clone();
        sorted.sort_by(|a, b| a.focal.partial_cmp(&b.focal).unwrap_or(std::cmp::Ordering::Equal));

        if focal_mm <= sorted[0].focal || sorted.len() == 1 {
            return Some(sorted[0].clone());
        }
        let last = sorted.last().unwrap();
        if focal_mm >= last.focal {
            return Some(last.clone());
        }

        for pair in sorted.windows(2) {
            let c0 = &pair[0];
            let c1 = &pair[1];
            if focal_mm >= c0.focal && focal_mm <= c1.focal {
                if c0.model != c1.model || (c1.focal - c0.focal).abs() < 1e-9 {
                    return Some(if (focal_mm - c0.focal).abs() <= (c1.focal - focal_mm).abs() {
                        c0.clone()
                    } else {
                        c1.clone()
                    });
                }
                let t = (focal_mm - c0.focal) / (c1.focal - c0.focal);
                let coeffs = [
                    c0.coeffs[0] * (1.0 - t) + c1.coeffs[0] * t,
                    c0.coeffs[1] * (1.0 - t) + c1.coeffs[1] * t,
                    c0.coeffs[2] * (1.0 - t) + c1.coeffs[2] * t,
                ];
                let real_focal = match (c0.real_focal, c1.real_focal) {
                    (Some(r0), Some(r1)) => Some(r0 * (1.0 - t) + r1 * t),
                    _ => None,
                };
                return Some(LensfunCalibration {
                    model: c0.model,
                    focal: focal_mm,
                    real_focal,
                    coeffs,
                });
            }
        }
        Some(sorted[0].clone())
    }

    /// Converts a Lensfun lens calibration at `focal_mm` into a Gyroflow `LensProfile`.
    pub fn to_lens_profile(
        &self,
        width: usize,
        height: usize,
        camera: Option<&LensfunCamera>,
        focal_mm: Option<f64>,
    ) -> Option<LensProfile> {
        let w = width.max(1920);
        let h = height.max(1080);
        let target_crop = camera
            .map(|c| c.crop_factor)
            .filter(|&c| c > 0.0)
            .unwrap_or_else(|| if self.crop_factor > 0.0 { self.crop_factor } else { 1.0 });

        if !self.supports_crop_factor(target_crop) {
            return None;
        }

        let target_focal = focal_mm.unwrap_or_else(|| {
            self.calibrations.first().map(|c| c.focal).unwrap_or(28.0)
        });
        let calib = self.interpolate_calibration(target_focal)?;
        let calib_crop = if self.crop_factor > 0.0 { self.crop_factor } else { target_crop };
        let calib_aspect = if self.aspect_ratio > 0.0 { self.aspect_ratio } else { 3.0 / 2.0 };
        let coeffs = calib.rescaled_coeffs(calib_crop, calib_aspect);

        let real_focal = calib.effective_focal();
        let sensor_diag_mm = 36.0_f64.hypot(24.0) / target_crop;
        let pixel_diag = (w as f64).hypot(h as f64);
        let f_px = real_focal * pixel_diag / sensor_diag_mm;

        let camera_brand = camera
            .map(|c| c.maker.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.maker.clone());
        let camera_model = camera
            .map(|c| c.model.clone())
            .unwrap_or_default();
        let identifier = format!(
            "lensfun:{}:{}:{}:{:.1}mm",
            camera_brand.to_ascii_lowercase().replace(' ', "_"),
            camera_model.to_ascii_lowercase().replace(' ', "_"),
            self.model.to_ascii_lowercase().replace(' ', "_"),
            calib.focal
        );

        let mut profile = LensProfile {
            name: format!("{} {} {} {:.1}mm", camera_brand, camera_model, self.model, calib.focal)
                .trim()
                .to_string(),
            calibrated_by: "Lensfun".to_string(),
            camera_brand,
            camera_model,
            lens_model: format!("{} ({:.1}mm)", self.model, calib.focal),
            calib_dimension: Dimensions { w, h },
            orig_dimension: Dimensions { w, h },
            input_horizontal_stretch: 1.0,
            input_vertical_stretch: 1.0,
            official: true,
            identifier,
            calibrator_version: env!("CARGO_PKG_VERSION").to_string(),
            distortion_model: Some(calib.model.id().to_string()),
            focal_length: Some(real_focal),
            crop_factor: Some(target_crop),
            fisheye_params: CameraParams {
                RMS_error: 0.0,
                camera_matrix: vec![
                    [f_px, 0.0, w as f64 / 2.0],
                    [0.0, f_px, h as f64 / 2.0],
                    [0.0, 0.0, 1.0],
                ],
                distortion_coeffs: coeffs,
                radial_distortion_limit: None,
            },
            ..Default::default()
        };
        profile.init();
        Some(profile)
    }
}

#[derive(Clone, Debug, Default)]
pub struct LensfunDatabase {
    pub cameras: Vec<LensfunCamera>,
    pub lenses: Vec<LensfunLens>,
}

fn extract_tag_blocks<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let open_pat = format!("<{}", tag);
    let close_pat = format!("</{}>", tag);
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start_idx) = rest.find(&open_pat) {
        let after_open = &rest[start_idx + open_pat.len()..];
        let valid_boundary = after_open
            .chars()
            .next()
            .map(|c| c == '>' || c.is_whitespace())
            .unwrap_or(false);
        if !valid_boundary {
            rest = after_open;
            continue;
        }
        let block_start = &rest[start_idx..];
        if let Some(end_idx) = block_start.find(&close_pat) {
            let full_end = end_idx + close_pat.len();
            out.push(&block_start[..full_end]);
            rest = &block_start[full_end..];
        } else {
            break;
        }
    }
    out
}

fn extract_child_text(block: &str, tag: &str) -> Option<String> {
    let open_pat = format!("<{}", tag);
    let close_pat = format!("</{}>", tag);
    let mut search = block;
    while let Some(pos) = search.find(&open_pat) {
        let after = &search[pos + open_pat.len()..];
        if let Some(gt) = after.find('>') {
            let tag_header = &after[..gt];
            // Prefer elements without lang= attribute (canonical English name)
            if !tag_header.contains("lang=") {
                let inner = &after[gt + 1..];
                if let Some(end) = inner.find(&close_pat) {
                    return Some(inner[..end].trim().to_string());
                }
            }
            search = &after[gt + 1..];
        } else {
            break;
        }
    }
    None
}

fn extract_all_child_texts(block: &str, tag: &str) -> Vec<String> {
    let open_pat = format!("<{}>", tag);
    let close_pat = format!("</{}>", tag);
    let mut out = Vec::new();
    let mut rest = block;
    while let Some(pos) = rest.find(&open_pat) {
        let inner = &rest[pos + open_pat.len()..];
        if let Some(end) = inner.find(&close_pat) {
            let val = inner[..end].trim();
            if !val.is_empty() {
                out.push(val.to_string());
            }
            rest = &inner[end + close_pat.len()..];
        } else {
            break;
        }
    }
    out
}

fn extract_attr(tag_text: &str, attr_name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let pattern = format!("{}={}", attr_name, quote);
        if let Some(idx) = tag_text.find(&pattern) {
            let rest = &tag_text[idx + pattern.len()..];
            if let Some(end) = rest.find(quote) {
                return Some(rest[..end].trim().to_string());
            }
        }
    }
    None
}

fn parse_aspect_ratio(text: &str) -> f64 {
    if let Some((num, den)) = text.split_once(':') {
        if let (Ok(n), Ok(d)) = (num.trim().parse::<f64>(), den.trim().parse::<f64>()) {
            if d > 0.0 {
                return n / d;
            }
        }
    }
    text.trim().parse::<f64>().unwrap_or(3.0 / 2.0)
}

impl LensfunDatabase {
    pub fn parse_xml(xml: &str) -> Self {
        let mut cameras = Vec::new();
        for cam_block in extract_tag_blocks(xml, "camera") {
            let maker = extract_child_text(cam_block, "maker").unwrap_or_default();
            let model = extract_child_text(cam_block, "model").unwrap_or_default();
            let mount = extract_child_text(cam_block, "mount").unwrap_or_default();
            let crop_factor = extract_child_text(cam_block, "cropfactor")
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(1.0);
            if !model.is_empty() {
                cameras.push(LensfunCamera {
                    maker,
                    model,
                    mount,
                    crop_factor,
                });
            }
        }

        let mut lenses = Vec::new();
        for lens_block in extract_tag_blocks(xml, "lens") {
            let maker = extract_child_text(lens_block, "maker").unwrap_or_default();
            let model = extract_child_text(lens_block, "model").unwrap_or_default();
            let mounts = extract_all_child_texts(lens_block, "mount");
            let crop_factor = extract_child_text(lens_block, "cropfactor")
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(1.0);
            let aspect_ratio = extract_child_text(lens_block, "aspect-ratio")
                .map(|s| parse_aspect_ratio(&s))
                .unwrap_or(3.0 / 2.0);

            let mut calibrations = Vec::new();
            let mut search = lens_block;
            while let Some(pos) = search.find("<distortion") {
                let after = &search[pos..];
                if let Some(end_gt) = after.find('>') {
                    let tag_str = &after[..=end_gt];
                    if let Some(model_str) = extract_attr(tag_str, "model") {
                        if let Some(dist_model) = LensfunDistortionModel::from_attr(&model_str) {
                            let focal = extract_attr(tag_str, "focal")
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(0.0);
                            let real_focal = extract_attr(tag_str, "real-focal")
                                .and_then(|s| s.parse::<f64>().ok());
                            let coeffs = match dist_model {
                                LensfunDistortionModel::Poly3 => [
                                    extract_attr(tag_str, "k1")
                                        .and_then(|s| s.parse::<f64>().ok())
                                        .unwrap_or(0.0),
                                    0.0,
                                    0.0,
                                ],
                                LensfunDistortionModel::Poly5 => [
                                    extract_attr(tag_str, "k1")
                                        .and_then(|s| s.parse::<f64>().ok())
                                        .unwrap_or(0.0),
                                    extract_attr(tag_str, "k2")
                                        .and_then(|s| s.parse::<f64>().ok())
                                        .unwrap_or(0.0),
                                    0.0,
                                ],
                                LensfunDistortionModel::PtLens => [
                                    extract_attr(tag_str, "a")
                                        .and_then(|s| s.parse::<f64>().ok())
                                        .unwrap_or(0.0),
                                    extract_attr(tag_str, "b")
                                        .and_then(|s| s.parse::<f64>().ok())
                                        .unwrap_or(0.0),
                                    extract_attr(tag_str, "c")
                                        .and_then(|s| s.parse::<f64>().ok())
                                        .unwrap_or(0.0),
                                ],
                            };
                            if focal > 0.0 {
                                calibrations.push(LensfunCalibration {
                                    model: dist_model,
                                    focal,
                                    real_focal,
                                    coeffs,
                                });
                            }
                        }
                    }
                    search = &after[end_gt + 1..];
                } else {
                    break;
                }
            }

            if !model.is_empty() && !calibrations.is_empty() {
                lenses.push(LensfunLens {
                    maker,
                    model,
                    mounts,
                    crop_factor,
                    aspect_ratio,
                    calibrations,
                });
            }
        }

        Self { cameras, lenses }
    }

    /// Generates Gyroflow `LensProfile` entries for all calibrated lenses in this Lensfun XML database.
    pub fn to_lens_profiles(&self, default_width: usize, default_height: usize) -> Vec<LensProfile> {
        let mut profiles = Vec::new();
        for lens in &self.lenses {
            let matching_camera = self
                .cameras
                .iter()
                .find(|cam| !cam.mount.is_empty() && lens.mounts.iter().any(|m| m == &cam.mount));
            for cal in &lens.calibrations {
                if let Some(profile) =
                    lens.to_lens_profile(default_width, default_height, matching_camera, Some(cal.focal))
                {
                    profiles.push(profile);
                }
            }
        }
        profiles
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_LENSFUN_XML: &str = r#"
<lensdatabase version="2">
    <camera>
        <maker>Sony</maker>
        <model>ILCE-7SM3</model>
        <mount>Sony E</mount>
        <cropfactor>1.0</cropfactor>
    </camera>
    <lens>
        <maker>Tamron</maker>
        <model>Tamron 28-200mm f/2.8-5.6 Di III RXD</model>
        <mount>Sony E</mount>
        <cropfactor>1.0</cropfactor>
        <aspect-ratio>3:2</aspect-ratio>
        <calibration>
            <distortion model="ptlens" focal="28" a="0.012" b="-0.038" c="0.015" />
            <distortion model="ptlens" focal="100" a="0.004" b="0.009" c="-0.002" />
            <distortion model="poly3" focal="50" k1="-0.018" />
            <distortion model="poly5" focal="70" k1="-0.011" k2="0.003" />
        </calibration>
    </lens>
</lensdatabase>
"#;

    #[test]
    fn parses_cameras_and_lenses_from_lensfun_xml() {
        let db = LensfunDatabase::parse_xml(SAMPLE_LENSFUN_XML);
        assert_eq!(db.cameras.len(), 1);
        assert_eq!(db.cameras[0].maker, "Sony");
        assert_eq!(db.cameras[0].model, "ILCE-7SM3");
        assert_eq!(db.lenses.len(), 1);
        assert_eq!(db.lenses[0].calibrations.len(), 4);
    }

    #[test]
    fn rescales_poly3_poly5_and_ptlens_coefficients_accurately() {
        let cal_poly3 = LensfunCalibration {
            model: LensfunDistortionModel::Poly3,
            focal: 28.0,
            real_focal: None,
            coeffs: [-0.02, 0.0, 0.0],
        };
        assert!((cal_poly3.effective_focal() - 28.0 * 1.02).abs() < 1e-9);
        let k_poly3 = cal_poly3.rescaled_coeffs(1.0, 1.5);
        assert!(k_poly3[0] < 0.0);

        let cal_ptlens = LensfunCalibration {
            model: LensfunDistortionModel::PtLens,
            focal: 28.0,
            real_focal: None,
            coeffs: [0.01, -0.03, 0.01],
        };
        assert!((cal_ptlens.effective_focal() - 28.0 * (1.0 - 0.01 + 0.03 - 0.01)).abs() < 1e-9);
        let k_ptlens = cal_ptlens.rescaled_coeffs(1.0, 1.5);
        assert_eq!(k_ptlens.len(), 4);
    }

    #[test]
    fn converts_lensfun_entries_into_gyroflow_lens_profiles() {
        let db = LensfunDatabase::parse_xml(SAMPLE_LENSFUN_XML);
        let profiles = db.to_lens_profiles(3840, 2160);
        assert_eq!(profiles.len(), 4);
        assert_eq!(profiles[0].distortion_model.as_deref(), Some("ptlens"));
        assert_eq!(profiles[0].calib_dimension.w, 3840);
        assert_eq!(profiles[0].calib_dimension.h, 2160);
        assert!(profiles[0].fisheye_params.camera_matrix[0][0] > 1000.0);
    }
}
