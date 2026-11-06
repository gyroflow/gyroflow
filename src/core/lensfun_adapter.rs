// SPDX-License-Identifier: GPL-3.0-or-later
// Lensfun distortion model adapter for Gyroflow (#150)

#[derive(Debug, Clone, PartialEq)]
pub enum LensfunDistortionModel {
    Poly3 { k1: f64 },
    Poly5 { k1: f64, k2: f64 },
    PtLens { a: f64, b: f64, c: f64 },
}

#[derive(Debug, Clone)]
pub struct LensfunEntry {
    pub maker: String,
    pub model: String,
    pub mount: String,
    pub crop_factor: f64,
    pub focal_mm: f64,
    pub distortion: LensfunDistortionModel,
}

impl LensfunEntry {
    /// Converts Lensfun polynomial distortion parameters into a 4-term radial distortion array `[k1, k2, k3, k4]`.
    pub fn to_radial_k4(&self) -> [f64; 4] {
        match self.distortion {
            LensfunDistortionModel::Poly3 { k1 } => [k1, 0.0, 0.0, 0.0],
            LensfunDistortionModel::Poly5 { k1, k2 } => [k1, k2, 0.0, 0.0],
            LensfunDistortionModel::PtLens { a, b, c } => [b, a, c, 0.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_poly5_conversion() {
        let entry = LensfunEntry {
            maker: "Sony".into(),
            model: "FE 24mm F1.4 GM".into(),
            mount: "Sony E".into(),
            crop_factor: 1.0,
            focal_mm: 24.0,
            distortion: LensfunDistortionModel::Poly5 { k1: -0.0142, k2: 0.0031 },
        };
        assert_eq!(entry.to_radial_k4(), [-0.0142, 0.0031, 0.0, 0.0]);
    }
}
