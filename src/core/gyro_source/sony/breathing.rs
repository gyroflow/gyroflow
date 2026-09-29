// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

// Lens breathing compensation.
// The focus and zoom positions recorded per frame are looked up in the lens tables from the MP4 meta box,
// weighted over each row's exposure window and turned into an output zoom that keeps the field of view
// constant while focusing. Values are magnifications relative to the lens' reference focus position.

use telemetry_parser::tags_impl::{ GroupedTagMap, GetWithType, GroupId, TagId };
use telemetry_parser::util::SampleInfo;
use rayon::iter::{ ParallelIterator, IntoParallelIterator };
use crate::gyro_source::BreathingFrame;

const ROWS: usize = 513;
const DECAY_DIVISOR: f32 = 5.0;
const ACTIVATION_FRAMES: u32 = 4;
/// How closely a stored row table has to reproduce the full one: a hundredth of an output pixel a thousand
/// pixels from the centre. The rows of a frame only differ while the focus moves during the readout, so most
/// tables collapse to one value and the rest to a few dozen bands; the full 513 rows would take 2 KB per frame,
/// 440 MB for an hour of 60p, kept for the session and written into projects with embedded metadata
const TABLE_TOLERANCE: f32 = 1e-5;

struct Profile {
    zoom_positions: usize,
    entries: usize,
    curve: Vec<u16>,
    magnification: Vec<u16>,
    usable: Vec<u16>, // entries per zoom row up to the first zero magnification
}

impl Profile {
    fn parse(v: &serde_json::Value, lens_id: u16) -> Option<Self> {
        let lens = v.as_array()?.iter().find(|l| l["lens_id"].as_u64() == Some(lens_id as u64))?;
        let u16s = |k: &str| -> Vec<u16> { lens[k].as_array().map(|a| a.iter().filter_map(|x| x.as_u64().map(|x| x as u16)).collect()).unwrap_or_default() };
        let zoom_positions = lens["zoom_positions"].as_u64()? as usize;
        let entries = lens["entries"].as_u64()? as usize;
        let (curve, magnification) = (u16s("focus_curve"), u16s("magnification"));
        let rows = zoom_positions.max(1);
        if entries == 0 || curve.len() < rows * entries || magnification.len() < rows * entries { return None; }
        let usable = (0..rows).map(|z| magnification[z * entries..(z + 1) * entries].iter().position(|&m| m == 0).unwrap_or(entries) as u16).collect();
        Some(Self { zoom_positions, entries, curve, magnification, usable })
    }

    // Zoom code: row index in the upper bits and a 7-bit fraction between two rows; bits 14 and 15 mark an unknown position
    fn factor(&self, zoom: u16, focus: u32) -> f32 {
        let n = self.entries;
        let (zi, zi1, fz) = if self.zoom_positions == 1 { (0, 0, 0u32) } else {
            let zi = (zoom >> 7) as usize;
            if zi > 128 { return 1.0; }
            (zi, if zi == 128 { 128 } else { zi + 1 }, (zoom & 0x7f) as u32)
        };
        let usable = {
            let (a, b) = if self.zoom_positions == 1 { (0, 0) } else if zoom & 0xc000 != 0 { (128, 128) } else { (zi, zi1) };
            let v = self.usable.get(a).copied().unwrap_or(0).min(self.usable.get(b).copied().unwrap_or(0)) as usize;
            if v == 0 || v >= n { n } else { v }
        };
        if usable < 2 { return 1.0; }
        let (r0, r1) = ((zi * n) & 0xffff, (zi1 * n) & 0xffff);
        let wz = 128 - fz;
        let at = |t: &[u16], r: usize, i: usize| t.get(r + i).copied().unwrap_or(0) as u32;
        let curve = |i: usize| (fz * at(&self.curve, r1, i) + wz * at(&self.curve, r0, i)) >> 7;

        let v0 = curve(0);
        let (lo, hi, base, step) = if focus < v0 {
            (0, 1, v0, curve(1) as i64 - v0 as i64)
        } else {
            let (mut i, mut prev, mut cur) = (1, v0, curve(1));
            while focus >= cur && i < usable - 1 {
                i += 1;
                prev = cur;
                cur = curve(i);
            }
            (i - 1, i, prev, cur as i64 - prev as i64)
        };
        let ff = if step != 0 {
            let d = (focus as i64 - base as i64).max(-10000);
            (if d < 0x2711 { d << 12 } else { 0x2710000 }) / step
        } else { 0 } as f32;

        let mag = |i: usize| (wz * at(&self.magnification, r0, i) + fz * at(&self.magnification, r1, i)) as f32 / 128.0;
        (mag(hi) * ff + (4096.0 - ff) * mag(lo)) / 4096.0 / 4096.0
    }
}

#[derive(Default)]
struct Frame {
    period: f32, // µs
    phase: u8,
    span_before: u8,
    span_after: u8,
    phase_den: u8,
    exposure: f32,
    exposure_offset: f32,
    readout: f32,
    sensor_h: u16,
    crop_y: i32,
    crop_h: u16,
    zoom: Vec<u16>,
    focus: Vec<u16>,
    focus_next: Vec<u16>,
    delay: u8,
    zoom_hi: Vec<u16>,
    focus_hi: Vec<u16>,
    position_valid: bool,
    restart: bool,
    in_camera: bool,
    scale: f32,
    camera_factor: f32,
    applied: f32,
}

impl Frame {
    fn parse(g: &GroupedTagMap) -> Option<Self> {
        let g = g.get(&GroupId::LensBreathing)?;
        let ms = |id: u32| (g.get_t(TagId::Unknown(id)) as Option<&f64>).map(|x| (x * 1000.0) as f32);
        let u8v = |id: u32| (g.get_t(TagId::Unknown(id)) as Option<&u8>).copied().unwrap_or(0);
        let vec = |id: u32| (g.get_t(TagId::Unknown(id)) as Option<&Vec<u16>>).cloned().unwrap_or_default();
        let f32v = |id: u32| (g.get_t(TagId::Unknown(id)) as Option<&f32>).copied().unwrap_or(1.0);
        let flag = |id: u32| (g.get_t(TagId::Unknown(id)) as Option<&bool>).copied().unwrap_or(false);
        let pair = |id: u32| (g.get_t(TagId::Unknown(id)) as Option<&(u32, u32)>).copied();
        let counter = (g.get_t(TagId::Unknown(0xe511)) as Option<&u16>).copied().unwrap_or(0);
        Some(Self {
            period: ms(0xe501)?,
            phase: (counter & 0xff) as u8,
            span_before: u8v(0xe512),
            span_after: u8v(0xe513),
            phase_den: u8v(0xe514),
            exposure: ms(0xe516).unwrap_or(0.0),
            exposure_offset: ms(0xe517).unwrap_or(0.0),
            readout: ms(0xe518).unwrap_or(0.0),
            sensor_h: pair(0xe519)?.1 as u16,
            crop_y: pair(0xe51a)?.1 as i16 as i32,
            crop_h: pair(0xe51b)?.1 as u16,
            zoom: vec(0xe523),
            focus: vec(0xe524),
            focus_next: vec(0xe525),
            delay: u8v(0xe526),
            zoom_hi: vec(0xe527),
            focus_hi: vec(0xe528),
            position_valid: u8v(0xe504) & 1 != 0,
            restart: u8v(0xe52a) != 0,
            in_camera: flag(0xe531),
            scale: f32v(0xe533),
            camera_factor: f32v(0xe536),
            applied: f32v(0xe537),
        })
    }

    // Time of a row's exposure window in units of the sample spacing. Samples are stored newest first,
    // so index 0 is the latest one and index n - 1 the earliest
    fn exposure_start(&self, row: i32) -> f32 {
        let t0 = if self.phase_den < 2 { self.period * self.span_after as f32 } else { self.period / self.phase_den as f32 };
        (t0 - self.exposure).max(0.0) + self.exposure_offset + self.readout * row as f32 / self.sensor_h.max(1) as f32
    }

    // Factor at the exposure centre of one row
    fn cog_point(&self, factors: &[f32], dt: f32, row: i32) -> f32 {
        let n = factors.len();
        if n == 0 { return 1.0; }
        let phase = if self.phase_den >= 2 && self.phase != 0 { self.period * self.phase as f32 / self.phase_den as f32 } else { 0.0 };
        let u = ((phase + self.exposure * 0.5 + self.exposure_start(row)) / dt).clamp(0.0, (n - 1) as f32);
        let i = n - 1 - u as usize;
        if i > 0 { factors[i] + u.fract() * (factors[i - 1] - factors[i]) } else { factors[i] }
    }

    // Factor averaged over the exposure window of one row
    fn cog_window(&self, factors: &[f32], dt: f32, row: i32) -> f32 {
        let n = factors.len();
        if n == 0 { return 1.0; }
        let start = self.exposure_start(row);
        let ua = (start / dt).clamp(0.0, (n - 1) as f32);
        let ub = ((start + self.exposure) / dt).clamp(0.0, (n - 1) as f32);
        let (ia, ib) = (n - 1 - ua as usize, n - 1 - ub as usize);
        let (fa, fb) = (ua.fract(), ub.fract());
        let at = |i: usize, f: f32| if i > 0 { factors[i] + f * (factors[i - 1] - factors[i]) } else { factors[i] };
        if ua as usize == ub as usize {
            return (at(ia, fa) + at(ib, fb)) * 0.5;
        }
        if ia < 1 { return factors[0]; }
        let (va, vb) = (at(ia, fa), at(ib, fb));
        let (mut weight, mut acc) = (1.0 - fa, (1.0 - fa) * (factors[ia - 1] + va) * 0.5);
        let mut j = ia - 1;
        while ib < j {
            acc += (factors[j - 1] + factors[j]) * 0.5;
            weight += 1.0;
            j -= 1;
        }
        if ib >= 1 {
            acc += fb * (factors[ib] + vb) * 0.5;
            weight += fb;
        }
        if weight > 0.0 { acc / weight } else { (va + vb) * 0.5 }
    }
}

#[derive(Default)]
struct Analysis {
    table: Vec<f32>, // per row band of the capture area (compacted, see `compact_table`), empty when unavailable
    max: f32,        // largest row value, the one the normalization has to make room for
    cog: f32,
    cog_valid: bool,
    center: f32,
    center_valid: bool,
}

fn analyze(profile: &Profile, fr: &Frame, frames: &[Option<Frame>], f: usize) -> Analysis {
    let mut a = Analysis::default();
    let (crop_y, crop_h) = (fr.crop_y, fr.crop_h as i32);
    let span = (fr.span_before + fr.span_after) as f32;

    // Row table from the high-rate samples
    if !fr.focus.is_empty() && !fr.focus_hi.is_empty() && fr.crop_h != 0xffff {
        let zoom = if fr.zoom_hi.is_empty() { &fr.zoom } else { &fr.zoom_hi };
        let (nf, nz) = (fr.focus_hi.len(), zoom.len());
        if nz > 0 {
            let span = span.max(1.0);
            let dt = fr.period * span / (nf as f32 - 1.0).max(1.0);
            let dt_zoom = fr.period * span / (nz as f32 - 1.0).max(1.0);
            let factors: Vec<f32> = (0..nf).map(|i| {
                let zoom_code = if nf == nz { zoom[i] } else {
                    // Zoom sampled at a different rate: interpolate it at the focus sample time, newest first
                    let mut t = dt * (nf - 1 - i) as f32;
                    if fr.phase_den > 1 && fr.phase != 0 { t += fr.period * fr.phase as f32 / fr.phase_den as f32; }
                    let u = (t / dt_zoom).min((nz - 1) as f32);
                    let j = (u as usize).min(nz.saturating_sub(2));
                    let (za, zb) = (zoom[nz - 1 - j] as f32, zoom[nz - 1 - (j + 1).min(nz - 1)] as f32);
                    (za + (u - j as f32) * (zb - za)) as u16
                };
                profile.factor(zoom_code, fr.focus_hi[i] as u32)
            }).collect();
            a.table = if fr.readout == 0.0 {
                vec![fr.cog_window(&factors, dt, crop_y); ROWS]
            } else {
                (0..ROWS as i32).map(|r| fr.cog_window(&factors, dt, crop_y + (((crop_h - 1) * r) >> 9))).collect()
            };
            a.max = a.table.iter().copied().fold(f32::MIN, f32::max);
        }
    }

    // Frame value from the low-rate samples
    let center = crop_y + crop_h / 2;
    let dt_low = |n: usize| fr.period * span.max(2.0) / (n as f32 - 1.0).max(1.0);
    if !fr.focus.is_empty() && fr.focus.len() == fr.zoom.len() {
        let factors: Vec<f32> = fr.zoom.iter().zip(&fr.focus).map(|(&z, &p)| profile.factor(z, p as u32)).collect();
        a.cog = fr.cog_point(&factors, dt_low(factors.len()), center);
        a.cog_valid = true;
    }

    // Value at the centre row: from the table, or from the delayed focus samples of a later frame
    if !a.table.is_empty() {
        let i = ((center - crop_y) * (ROWS as i32 - 1) / (crop_h - 1).max(1)).clamp(0, ROWS as i32 - 1) as usize;
        a.center = a.table[i];
        a.center_valid = true;
    } else if let Some(later) = frames[(f + fr.delay as usize).saturating_sub(1).min(frames.len() - 1)].as_ref() {
        if !later.focus_next.is_empty() && later.focus_next.len() == fr.zoom.len() {
            let factors: Vec<f32> = fr.zoom.iter().zip(&later.focus_next).map(|(&z, &p)| profile.factor(z, p as u32)).collect();
            a.center = fr.cog_point(&factors, dt_low(factors.len()), center);
            a.max = a.center;
            a.center_valid = true;
        }
    }
    // Stored, and carried through the temporal pass, no finer than the lookup needs
    a.table = compact_table(&a.table);
    a
}

// Largest difference between `table` and its linear interpolation from every `stride`-th row (the ends kept),
// the lookup `BreathingFrame::scale_at_row` does on the stored table
fn subsampling_error(table: &[f32], stride: usize) -> f32 {
    let last = table.len() - 1;
    table.iter().enumerate().fold(0.0f32, |worst, (i, &v)| {
        let a = i - i % stride;
        let b = (a + stride).min(last);
        let approx = table[a] + (i - a) as f32 / stride as f32 * (table[b] - table[a]);
        worst.max((approx - v).abs())
    })
}

// The coarsest subsampling of `table` that reproduces it within `TABLE_TOLERANCE`: one value when the rows agree
// to it, otherwise every 2^k-th row down to the full table. The subsampled rows keep their exact values, so the
// normalization over the largest row still holds
fn compact_table(table: &[f32]) -> Vec<f32> {
    let full = table.len().saturating_sub(1);
    if full == 0 { return table.to_vec(); }
    let (min, max) = table.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    if max - min <= TABLE_TOLERANCE { return vec![(min + max) * 0.5]; }
    let mut stride = full;
    while stride > 1 {
        if full % stride == 0 && subsampling_error(table, stride) <= TABLE_TOLERANCE {
            return table.iter().copied().step_by(stride).collect();
        }
        stride /= 2;
    }
    table.to_vec()
}

// The offset keeps the zoom continuous when the source of the value changes and decays with the motion
fn decay_offset(total: f32, last_total: f32, offset: f32) -> f32 {
    let d = ((total - last_total) / DECAY_DIVISOR).abs();
    if offset <= 0.0 { (offset + d).min(0.0) } else { (offset - d).max(0.0) }
}

pub fn compute(samples: &[SampleInfo]) -> Vec<BreathingFrame> {
    let profile = samples.first().and_then(|s| s.tag_map.as_ref()).and_then(|tm| {
        let g = tm.get(&GroupId::LensBreathing)?;
        let lens_id = *(g.get_t(TagId::Unknown(0xe502)) as Option<&u16>)?;
        Profile::parse((g.get_t(TagId::Data) as Option<&serde_json::Value>)?, lens_id)
    });
    let Some(profile) = profile else { return Vec::new() };

    let frames: Vec<Option<Frame>> = samples.iter().map(|s| s.tag_map.as_ref().and_then(Frame::parse)).collect();
    compute_frames(&profile, &frames)
}

/// `None`: a sample without a usable LensBreathing group. Such a frame has no magnification of its own and takes
/// the zoom of the nearest frame that has one (the previous one, or the first one for the leading frames), so the
/// picture stays continuous across it instead of popping to the raw framing for a frame. A frame whose zoom doesn't
/// come out as a positive finite number is treated the same way: `FrameTransform::at_timestamp` folds the zoom into
/// the projection, and a zero would collapse the whole frame to its centre
fn compute_frames(profile: &Profile, frames: &[Option<Frame>]) -> Vec<BreathingFrame> {
    let analyses: Vec<Analysis> = (0..frames.len()).into_par_iter().map(|f| frames[f].as_ref().map(|fr| analyze(profile, fr, frames, f)).unwrap_or_default()).collect();

    // Temporal pass: pick the source of each frame's value and let the offset decay with the motion.
    // (value, offset, per row): the frame's magnification, the carried offset, and whether the row table applies.
    // A frame without data leaves the carried state to the next one
    let (mut last_total, mut last_offset, mut counter) = (0.0f32, 0.0f32, 0u32);
    let per_frame: Vec<Option<(f32, f32, bool)>> = frames.iter().zip(&analyses).map(|(fr, a)| {
        let fr = fr.as_ref()?;
        if !a.center_valid || !a.cog_valid || fr.crop_h == 0xffff || fr.sensor_h == 0xffff {
            last_total = fr.camera_factor;
            last_offset = 0.0;
            return Some((fr.camera_factor, 0.0, false));
        }
        if fr.restart { counter = ACTIVATION_FRAMES; } else if counter > 0 { counter -= 1; }
        let delayed = !fr.position_valid || counter == 0;
        let value = if delayed { a.center } else { a.cog };
        let offset = decay_offset(value + last_offset, last_total, last_offset);
        last_total = value + offset;
        last_offset = offset;
        Some((value, offset, delayed && !a.table.is_empty() && fr.readout != 0.0))
    }).collect();

    // The zoom is relative to the largest magnification any applied value reaches, the largest row of a row
    // table included, so no row of any frame is zoomed out past its content: the output samples the source at
    // `scale`, a scale above 1 reaches outside the frame the adaptive zoom validated (it knows nothing of this
    // zoom, see `FrameTransform::at_timestamp`), and the rows of one frame differ by the focus motion during
    // the readout
    let scale = |fr: &Frame| if fr.in_camera && fr.applied > 0.0 { fr.scale / fr.applied } else { fr.scale };
    let max = frames.iter().zip(&analyses).zip(&per_frame)
        .filter_map(|((fr, a), pf)| Some((fr.as_ref()?, a, (*pf)?)))
        .map(|(fr, a, (v, o, per_row))| scale(fr) * (o + if per_row { a.max } else { v }))
        .filter(|m| m.is_finite())
        .fold(f32::MIN, f32::max);
    if !(max > 0.0) { return Vec::new(); }

    let zooms: Vec<Option<BreathingFrame>> = frames.iter().zip(&analyses).zip(&per_frame).map(|((fr, a), pf)| {
        let (fr, (value, offset, per_row)) = (fr.as_ref()?, (*pf)?);
        let s = scale(fr) / max;
        let zoom: Vec<f32> = if per_row { a.table.iter().map(|t| s * (t + offset)).collect() } else { vec![s * (value + offset)] };
        if zoom.iter().any(|k| !(k.is_finite() && *k > 0.0)) { return None; }
        Some(BreathingFrame { scale: zoom, crop_y: fr.crop_y as f32, crop_h: fr.crop_h as f32 })
    }).collect();
    let Some(first) = zooms.iter().find_map(|z| z.clone()) else { return Vec::new() };
    let mut last = first;
    zooms.into_iter().map(|z| { if let Some(z) = z { last = z; } last.clone() }).collect()
}
