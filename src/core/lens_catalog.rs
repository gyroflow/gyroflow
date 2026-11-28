// SPDX-License-Identifier: GPL-3.0-or-later
// Camera & lens mount compatibility catalog (#742)

#[derive(Debug, Clone, PartialEq)]
pub struct CameraBodySpec {
    pub brand: String,
    pub model: String,
    pub mount: String,
    pub sensor_width_mm: f64,
    pub sensor_height_mm: f64,
    pub crop_factor: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LensSpec {
    pub brand: String,
    pub model: String,
    pub compatible_mounts: Vec<String>,
    pub min_focal_mm: f64,
    pub max_focal_mm: f64,
}

impl CameraBodySpec {
    pub fn is_compatible_with(&self, lens: &LensSpec) -> bool {
        let m = self.mount.to_ascii_lowercase();
        lens.compatible_mounts.iter().any(|lm| lm.to_ascii_lowercase() == m)
    }

    pub fn effective_focal_length_mm(&self, nominal_focal_mm: f64) -> f64 {
        nominal_focal_mm * self.crop_factor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mount_compatibility_and_crop() {
        let cam = CameraBodySpec {
            brand: "Sony".into(),
            model: "FX30".into(),
            mount: "Sony E".into(),
            sensor_width_mm: 23.3,
            sensor_height_mm: 15.5,
            crop_factor: 1.5,
        };
        let lens = LensSpec {
            brand: "Sigma".into(),
            model: "18-50mm F2.8 DC DN".into(),
            compatible_mounts: vec!["Sony E".into(), "L-Mount".into()],
            min_focal_mm: 18.0,
            max_focal_mm: 50.0,
        };
        assert!(cam.is_compatible_with(&lens));
        assert!((cam.effective_focal_length_mm(20.0) - 30.0).abs() < 1e-6);
    }
}
