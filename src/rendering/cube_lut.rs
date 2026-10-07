// SPDX-License-Identifier: GPL-3.0-or-later

//! The bounded, standard .cube subset supported by the live preview.
//! Entries remain float32; the GPU atlas packs their bytes into opaque RGB texels.
pub struct CubeLut {
    pub size: usize,
    pub entries: Vec<[f32; 3]>,
}

impl CubeLut {
    pub fn read_bounded(reader: impl std::io::Read) -> Result<Vec<u8>, String> {
        use std::io::Read;
        let mut bytes = Vec::new();
        reader
            .take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("The LUT is too large (maximum 64 MB).".into());
        }
        Ok(bytes)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("The LUT is too large (maximum 64 MB).".into());
        }
        let text = std::str::from_utf8(bytes).map_err(|_| "The LUT must be a text .cube file.")?;
        let mut size = None;
        let mut entries = Vec::new();
        for line in text.trim_start_matches('\u{feff}').lines() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() || line.starts_with("TITLE") {
                continue;
            }
            let words: Vec<_> = line.split_whitespace().collect();
            match words[0] {
                "LUT_3D_SIZE" => {
                    if size.is_some() || !entries.is_empty() || words.len() != 2 {
                        return Err("Invalid 3D LUT size.".into());
                    }
                    let n: usize = words[1].parse().map_err(|_| "Invalid 3D LUT size.")?;
                    if !(2..=128).contains(&n) {
                        return Err("Live preview supports 3D LUT sizes from 2 to 128.".into());
                    }
                    size = Some(n);
                }
                "DOMAIN_MIN" | "DOMAIN_MAX" => {
                    let expected = if words[0] == "DOMAIN_MIN" { 0.0 } else { 1.0 };
                    if words.len() != 4
                        || words[1..]
                            .iter()
                            .any(|v| v.parse::<f32>().ok() != Some(expected))
                    {
                        return Err("Live preview requires a standard 0–1 LUT input domain.".into());
                    }
                }
                _ => {
                    let n = size.ok_or("Choose a standard 3D .cube LUT (no 1D shaper).")?;
                    if words.len() != 3 || entries.len() >= n * n * n {
                        return Err("Invalid number of LUT entries.".into());
                    }
                    let mut entry = [0.0; 3];
                    for i in 0..3 {
                        entry[i] = words[i].parse::<f32>().map_err(|_| "Invalid LUT value.")?;
                        if !entry[i].is_finite() {
                            return Err("LUT values must be finite.".into());
                        }
                    }
                    entries.push(entry);
                }
            }
        }
        let size = size.ok_or("The file contains no 3D LUT.")?;
        if entries.len() != size * size * size {
            return Err("The LUT is incomplete.".into());
        }
        Ok(Self { size, entries })
    }

    pub fn canonical_cube(&self) -> Vec<u8> {
        let mut text = format!("LUT_3D_SIZE {}\n", self.size);
        for v in &self.entries {
            text.push_str(&format!("{} {} {}\n", v[0], v[1], v[2]));
        }
        text.into_bytes()
    }

    pub fn atlas(&self) -> (usize, usize, Vec<u8>) {
        // Six RGB texels per lattice point. Alpha is never used to store data,
        // so Qt's premultiplied image upload cannot alter any float bits.
        let width = 1536;
        let height = (self.entries.len() * 6).div_ceil(width);
        let mut bytes = vec![0; width * height * 3];
        for (i, entry) in self.entries.iter().enumerate() {
            for (c, value) in entry.iter().enumerate() {
                let b = value.to_le_bytes();
                let offset = (i * 6 + c * 2) * 3;
                bytes[offset..offset + 3].copy_from_slice(&b[..3]);
                bytes[offset + 3] = b[3];
            }
        }
        (width, height, bytes)
    }
}
