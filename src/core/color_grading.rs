// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Gyroflow

//! Premiere-Pro-style color grading parameters.
//!
//! All values are stored NORMALIZED (shader-ready) so the GPU/CPU paths can
//! consume them directly without re-scaling:
//! - temperature / tint / exposure / contrast / highlights / shadows / whites
//!   / blacks / vibrance: `-1.0..1.0` (0.0 = neutral)
//! - basic_saturation / creative_saturation: `0.0..2.0` (1.0 = neutral)
//! - faded_film: `0.0..1.0` (0.0 = off)
//!
//! The UI sliders use human ranges (e.g. -100..100, 0..200) and the controller
//! divides by 100 before calling the setters, so the core only ever sees
//! normalized values.

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ColorGradingParams {
    pub basic_enabled: bool,
    pub creative_enabled: bool,

    // Basic - color
    pub temperature: f32,
    pub tint: f32,
    pub basic_saturation: f32,

    // Basic - light
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,

    // Creative
    pub faded_film: f32,
    pub vibrance: f32,
    pub creative_saturation: f32,

    // LUT (.cube). Path is persisted; strength 0..1; enabled toggles it.
    pub lut_enabled: bool,
    pub lut_strength: f32,
    pub lut_path: String,

    // Parsed LUT data, kept out of serde (re-loaded from lut_path on import).
    #[serde(skip)]
    pub lut: Option<std::sync::Arc<crate::lut::Lut>>,
}

impl Default for ColorGradingParams {
    fn default() -> Self {
        Self {
            basic_enabled: false,
            creative_enabled: false,
            temperature: 0.0,
            tint: 0.0,
            basic_saturation: 1.0,
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            faded_film: 0.0,
            vibrance: 0.0,
            creative_saturation: 1.0,
            lut_enabled: false,
            lut_strength: 1.0,
            lut_path: String::new(),
            lut: None,
        }
    }
}

impl ColorGradingParams {
    /// True when no enabled section would alter the image. Used to skip the
    /// color pass entirely (identity).
    pub fn is_identity(&self) -> bool {
        !self.basic_enabled && !self.creative_enabled && !(self.lut_enabled && self.lut.is_some())
    }

    /// Apply the full color grading chain to one RGB pixel in 0..1.
    /// MUST stay numerically identical to `apply_color_grading()`+`apply_lut()`
    /// in `src/qt_gpu/undistort.frag` so preview and export match.
    /// Used by all export backends (after YUV->RGB).
    pub fn apply_rgb(&self, mut x: [f32; 3]) -> [f32; 3] {
        const LR: f32 = 0.2126; const LG: f32 = 0.7152; const LB: f32 = 0.0722;
        let cl = |v: f32| v.max(0.0).min(1.0);
        let luma = |c: [f32; 3]| c[0] * LR + c[1] * LG + c[2] * LB;
        let smoothstep = |e0: f32, e1: f32, v: f32| { let t = ((v - e0) / (e1 - e0)).max(0.0).min(1.0); t * t * (3.0 - 2.0 * t) };

        if self.lut_enabled {
            if let Some(lut) = &self.lut {
                x = lut.sample_rgb([cl(x[0]), cl(x[1]), cl(x[2])], self.lut_strength);
            }
        }

        if self.basic_enabled {
            x[0] = cl(x[0]); x[1] = cl(x[1]); x[2] = cl(x[2]);
            x[0] += self.temperature * 0.2;
            x[2] -= self.temperature * 0.2;
            x[1] += self.tint * 0.2;
            let e = 2.0f32.powf(self.exposure * 2.0);
            x[0] *= e; x[1] *= e; x[2] *= e;
            for c in x.iter_mut() { *c = (*c - 0.5) * (1.0 + self.contrast) + 0.5; }
            let l = luma([cl(x[0]), cl(x[1]), cl(x[2])]);
            let add = self.highlights * 0.5 * smoothstep(0.5, 1.0, l)
                    + self.shadows    * 0.5 * (1.0 - smoothstep(0.0, 0.5, l))
                    + self.whites * 0.2 * l
                    + self.blacks * 0.2 * (1.0 - l);
            for c in x.iter_mut() { *c += add; }
            let g = luma(x);
            for c in x.iter_mut() { *c = g + (*c - g) * self.basic_saturation; }
        }

        if self.creative_enabled {
            for c in x.iter_mut() { *c = *c + ((*c * 0.85 + 0.15) - *c) * self.faded_film; }
            let g2 = luma(x);
            let sat = ((x[0]-g2).powi(2) + (x[1]-g2).powi(2) + (x[2]-g2).powi(2)).sqrt();
            let vib = self.vibrance * (1.0 - smoothstep(0.0, 0.6, sat));
            for c in x.iter_mut() { *c = g2 + (*c - g2) * (1.0 + vib); }
            let g3 = luma(x);
            for c in x.iter_mut() { *c = g3 + (*c - g3) * self.creative_saturation; }
        }

        [cl(x[0]), cl(x[1]), cl(x[2])]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_identity() {
        let p = ColorGradingParams::default();
        assert!(!p.basic_enabled);
        assert!(!p.creative_enabled);
        assert!(p.is_identity());
        assert_eq!(p.temperature, 0.0);
        assert_eq!(p.tint, 0.0);
        assert_eq!(p.basic_saturation, 1.0);
        assert_eq!(p.exposure, 0.0);
        assert_eq!(p.contrast, 0.0);
        assert_eq!(p.creative_saturation, 1.0);
        assert_eq!(p.faded_film, 0.0);
    }

    #[test]
    fn serde_roundtrip() {
        let mut p = ColorGradingParams::default();
        p.basic_enabled = true;
        p.exposure = 0.5;
        p.basic_saturation = 1.25;
        let s = serde_json::to_string(&p).unwrap();
        let p2: ColorGradingParams = serde_json::from_str(&s).unwrap();
        assert_eq!(p2, p);
        assert!(p2.basic_enabled);
        assert_eq!(p2.exposure, 0.5);
        assert_eq!(p2.basic_saturation, 1.25);
    }

    #[test]
    fn serde_partial_is_backward_compatible() {
        // Old projects without a color_grading object, or with a partial one,
        // must deserialize to defaults for missing fields (#[serde(default)]).
        let p: ColorGradingParams = serde_json::from_str("{}").unwrap();
        assert_eq!(p, ColorGradingParams::default());

        let p2: ColorGradingParams = serde_json::from_str(r#"{"exposure":0.3}"#).unwrap();
        assert_eq!(p2.exposure, 0.3);
        assert_eq!(p2.basic_saturation, 1.0); // default preserved
    }
}
