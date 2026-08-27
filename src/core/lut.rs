// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal Adobe `.cube` LUT parser (1D and 3D), producing GPU-ready RGBA f32 data.
//!
//! A 3D LUT of size N is stored as an RGBA f32 table of N*N*N entries, in the
//! canonical .cube ordering: red varies fastest, then green, then blue.
//! For GPU upload it can be laid out as a 2D texture of width=N, height=N*N
//! (N slices of NxN stacked vertically), which matches the existing
//! `texMeshData` (RGBA32F) upload pattern used by the Qt RHI preview path.
//!
//! A 1D LUT of size N is stored as N RGBA f32 entries (one per input level),
//! applied per channel.

#[derive(Clone, Debug, PartialEq)]
pub enum LutKind {
    Dim1,
    Dim3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Lut {
    pub kind: LutKind,
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    /// RGBA f32. For 3D: size^3 entries (r fastest, then g, then b).
    /// For 1D: size entries.
    pub data: Vec<f32>,
}

impl Lut {
    /// Parse an Adobe `.cube` file from its text contents.
    pub fn parse_cube(text: &str) -> Result<Lut, String> {
        let mut size_3d: Option<usize> = None;
        let mut size_1d: Option<usize> = None;
        let mut domain_min = [0.0f32, 0.0, 0.0];
        let mut domain_max = [1.0f32, 1.0, 1.0];
        let mut table: Vec<[f32; 3]> = Vec::new();

        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let upper = line.to_ascii_uppercase();
            if upper.starts_with("TITLE") {
                continue;
            } else if upper.starts_with("LUT_3D_SIZE") {
                size_3d = Some(Self::parse_last_usize(line)?);
            } else if upper.starts_with("LUT_1D_SIZE") {
                size_1d = Some(Self::parse_last_usize(line)?);
            } else if upper.starts_with("DOMAIN_MIN") {
                domain_min = Self::parse_triplet(line)?;
            } else if upper.starts_with("DOMAIN_MAX") {
                domain_max = Self::parse_triplet(line)?;
            } else {
                // Data row: three floats
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() == 3 {
                    let r = parts[0].parse::<f32>().map_err(|e| format!("bad value '{}': {e}", parts[0]))?;
                    let g = parts[1].parse::<f32>().map_err(|e| format!("bad value '{}': {e}", parts[1]))?;
                    let b = parts[2].parse::<f32>().map_err(|e| format!("bad value '{}': {e}", parts[2]))?;
                    table.push([r, g, b]);
                }
                // ignore any other unknown keyword lines
            }
        }

        if table.is_empty() {
            return Err("No LUT data found".into());
        }

        let (kind, size) = if let Some(n) = size_3d {
            if table.len() != n * n * n {
                return Err(format!("LUT_3D_SIZE {n} expects {} entries, found {}", n * n * n, table.len()));
            }
            (LutKind::Dim3, n)
        } else if let Some(n) = size_1d {
            if table.len() != n {
                return Err(format!("LUT_1D_SIZE {n} expects {n} entries, found {}", table.len()));
            }
            (LutKind::Dim1, n)
        } else {
            return Err("Missing LUT_3D_SIZE / LUT_1D_SIZE".into());
        };

        // Flatten to RGBA f32 (alpha = 1.0).
        let mut data = Vec::with_capacity(table.len() * 4);
        for px in &table {
            data.push(px[0]);
            data.push(px[1]);
            data.push(px[2]);
            data.push(1.0);
        }

        Ok(Lut { kind, size, domain_min, domain_max, data })
    }

    fn parse_last_usize(line: &str) -> Result<usize, String> {
        line.split_whitespace()
            .last()
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or_else(|| format!("Cannot parse size from '{line}'"))
    }

    fn parse_triplet(line: &str) -> Result<[f32; 3], String> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            Ok([
                parts[1].parse::<f32>().map_err(|e| e.to_string())?,
                parts[2].parse::<f32>().map_err(|e| e.to_string())?,
                parts[3].parse::<f32>().map_err(|e| e.to_string())?,
            ])
        } else {
            Err(format!("Cannot parse triplet from '{line}'"))
        }
    }

    /// RGBA f32 data ready for a 2D texture of width = `tex_width()`, height = `tex_height()`.
    /// For a 3D LUT this is the cube laid out as N vertically-stacked NxN slices.
    /// For a 1D LUT this is a 1xN texture (width 1, height N).
    pub fn rgba_f32(&self) -> &[f32] {
        &self.data
    }

    pub fn tex_width(&self) -> usize {
        match self.kind {
            LutKind::Dim3 => self.size,
            LutKind::Dim1 => 1,
        }
    }

    pub fn tex_height(&self) -> usize {
        match self.kind {
            LutKind::Dim3 => self.size * self.size,
            LutKind::Dim1 => self.size,
        }
    }

    #[inline]
    fn entry(&self, idx: usize) -> [f32; 3] {
        let o = idx * 4;
        [self.data[o], self.data[o + 1], self.data[o + 2]]
    }

    /// CPU LUT sampling matching the GLSL `apply_lut()`. `c` in 0..1, returns the
    /// graded color blended with the original by `strength` (0..1).
    /// For 3D: trilinear. For 1D: per-channel linear curve.
    pub fn sample_rgb(&self, c: [f32; 3], strength: f32) -> [f32; 3] {
        let n = self.size;
        if n < 2 { return c; }
        let nf = (n - 1) as f32;
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| [a[0]+(b[0]-a[0])*t, a[1]+(b[1]-a[1])*t, a[2]+(b[2]-a[2])*t];
        let cl = |v: f32| v.max(0.0).min(1.0);
        let res = match self.kind {
            LutKind::Dim3 => {
                let sample = |ri: usize, gi: usize, bi: usize| self.entry(bi * n * n + gi * n + ri);
                let rf = cl(c[0]) * nf; let gf = cl(c[1]) * nf; let bf = cl(c[2]) * nf;
                let r0 = rf.floor() as usize; let g0 = gf.floor() as usize; let b0 = bf.floor() as usize;
                let r1 = (r0 + 1).min(n - 1); let g1 = (g0 + 1).min(n - 1); let b1 = (b0 + 1).min(n - 1);
                let dr = rf - r0 as f32; let dg = gf - g0 as f32; let db = bf - b0 as f32;
                let c000 = sample(r0, g0, b0); let c100 = sample(r1, g0, b0);
                let c010 = sample(r0, g1, b0); let c110 = sample(r1, g1, b0);
                let c001 = sample(r0, g0, b1); let c101 = sample(r1, g0, b1);
                let c011 = sample(r0, g1, b1); let c111 = sample(r1, g1, b1);
                let c00 = lerp(c000, c100, dr); let c10 = lerp(c010, c110, dr);
                let c01 = lerp(c001, c101, dr); let c11 = lerp(c011, c111, dr);
                let c0 = lerp(c00, c10, dg); let c1 = lerp(c01, c11, dg);
                lerp(c0, c1, db)
            }
            LutKind::Dim1 => {
                let mut out = [0.0f32; 3];
                for ch in 0..3 {
                    let f = cl(c[ch]) * nf;
                    let i0 = f.floor() as usize; let i1 = (i0 + 1).min(n - 1);
                    let t = f - i0 as f32;
                    out[ch] = self.entry(i0)[ch] + (self.entry(i1)[ch] - self.entry(i0)[ch]) * t;
                }
                out
            }
        };
        let s = strength.max(0.0).min(1.0);
        [c[0]+(res[0]-c[0])*s, c[1]+(res[1]-c[1])*s, c[2]+(res[2]-c[2])*s]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_3d() {
        // 2x2x2 identity-ish cube
        let cube = "\
LUT_3D_SIZE 2
0.0 0.0 0.0
1.0 0.0 0.0
0.0 1.0 0.0
1.0 1.0 0.0
0.0 0.0 1.0
1.0 0.0 1.0
0.0 1.0 1.0
1.0 1.0 1.0
";
        let lut = Lut::parse_cube(cube).unwrap();
        assert_eq!(lut.kind, LutKind::Dim3);
        assert_eq!(lut.size, 2);
        assert_eq!(lut.data.len(), 8 * 4);
        assert_eq!(lut.tex_width(), 2);
        assert_eq!(lut.tex_height(), 4);
        // first entry RGBA
        assert_eq!(&lut.data[0..4], &[0.0, 0.0, 0.0, 1.0]);
        // second entry red=1
        assert_eq!(&lut.data[4..8], &[1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn parse_1d() {
        let cube = "\
# comment
TITLE \"test\"
LUT_1D_SIZE 3
0.0 0.0 0.0
0.5 0.5 0.5
1.0 1.0 1.0
";
        let lut = Lut::parse_cube(cube).unwrap();
        assert_eq!(lut.kind, LutKind::Dim1);
        assert_eq!(lut.size, 3);
        assert_eq!(lut.data.len(), 3 * 4);
        assert_eq!(lut.tex_width(), 1);
        assert_eq!(lut.tex_height(), 3);
    }

    #[test]
    fn parse_domain() {
        let cube = "\
LUT_3D_SIZE 2
DOMAIN_MIN 0.0 0.0 0.0
DOMAIN_MAX 1.0 1.0 1.0
0 0 0
0 0 0
0 0 0
0 0 0
0 0 0
0 0 0
0 0 0
0 0 0
";
        let lut = Lut::parse_cube(cube).unwrap();
        assert_eq!(lut.domain_min, [0.0, 0.0, 0.0]);
        assert_eq!(lut.domain_max, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn wrong_count_errors() {
        let cube = "LUT_3D_SIZE 2\n0 0 0\n1 1 1\n";
        assert!(Lut::parse_cube(cube).is_err());
    }

    #[test]
    fn empty_errors() {
        assert!(Lut::parse_cube("# just a comment\n").is_err());
    }
}
