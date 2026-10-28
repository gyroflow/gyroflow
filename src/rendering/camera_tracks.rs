// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! "Preserve other tracks" for MP4/MOV. FFmpeg's mov muxer writes the data tracks it doesn't know (DJI `djmd`/`dbgi`, Sony `rtmd`...)
//! with an empty sample description, which nothing can read, and replaces the camera's metadata (`encoder` with its own name).
//! So these tracks are left out of the render, and once the output is written, the camera's own `trak` boxes are appended to it with
//! their samples, and the camera's `udta` is merged in. Apps that read the camera's data need both: DJI Mimo's dashboard finds the
//! `djmd` track by its sample description, and its parser by the camera name in `encoder`

use std::collections::HashMap;
use std::io::{ self, Read, Seek, SeekFrom, Write };
use gyroflow_core::filesystem;

/// The input tracks to copy to the output as they are
#[derive(Debug, Default, Clone)]
pub struct CameraTracks {
    /// Their track ids (`tkhd`), which FFmpeg has as the stream id. Only those `check_tracks` passed
    pub track_ids: Vec<u32>,
    /// Track ids of the input streams FFmpeg wrote, in the order of the output tracks: what the references to them point to now
    pub ffmpeg_track_ids: Vec<u32>,
    /// Start of the input timeline (µs), where the output starts
    pub start_us: i64,
}

/// Boxes that only hold other boxes, of the ones read or changed here
const CONTAINERS: [&[u8; 4]; 7] = [b"moov", b"trak", b"mdia", b"minf", b"stbl", b"edts", b"udta"];
/// The `udta` and `ilst` items that FFmpeg reads as `encoder`
const ENCODER_KEYS: [&[u8; 4]; 3] = [b"\xa9too", b"\xa9swr", b"\xa9enc"];
/// A segment table in `moov/udta`: it maps the frames of the file it's in to the recordings they were cut from
const SEGMENT_TABLE: &[u8; 4] = b"MTRM";

/// A box: its payload, or for the containers, the boxes in it
#[derive(Debug, Clone)]
struct Atom {
    typ: [u8; 4],
    payload: Vec<u8>,
    children: Option<Vec<Atom>>,
}

impl Atom {
    fn new(typ: &[u8; 4], payload: Vec<u8>) -> Self {
        Self { typ: *typ, payload, children: None }
    }
    fn container(typ: &[u8; 4], children: Vec<Atom>) -> Self {
        Self { typ: *typ, payload: Vec::new(), children: Some(children) }
    }
    fn child(&self, typ: &[u8; 4]) -> Option<&Atom> {
        self.children.as_ref()?.iter().find(|x| &x.typ == typ)
    }
    fn child_mut(&mut self, typ: &[u8; 4]) -> Option<&mut Atom> {
        self.children.as_mut()?.iter_mut().find(|x| &x.typ == typ)
    }
    fn path(&self, path: &[&[u8; 4]]) -> Option<&Atom> {
        path.iter().try_fold(self, |atom, typ| atom.child(typ))
    }
    fn path_mut(&mut self, path: &[&[u8; 4]]) -> Option<&mut Atom> {
        path.iter().try_fold(self, |atom, typ| atom.child_mut(typ))
    }
    fn to_bytes(&self) -> Vec<u8> {
        let body: Vec<u8> = match &self.children {
            Some(children) => children.iter().flat_map(Atom::to_bytes).collect(),
            None => self.payload.clone()
        };
        let mut out = Vec::with_capacity(body.len() + 16);
        match u32::try_from(body.len() + 8) {
            Ok(size) => { out.extend(size.to_be_bytes()); out.extend(self.typ); }
            Err(_) => { out.extend(1u32.to_be_bytes()); out.extend(self.typ); out.extend((body.len() as u64 + 16).to_be_bytes()); }
        }
        out.extend(body);
        out
    }
}

fn invalid(what: String) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, what) }

fn be_u32(data: &[u8], at: usize) -> io::Result<u32> {
    data.get(at..at + 4).map(|x| u32::from_be_bytes(x.try_into().unwrap())).ok_or_else(|| invalid(format!("Truncated box at {at}")))
}
fn be_u64(data: &[u8], at: usize) -> io::Result<u64> {
    data.get(at..at + 8).map(|x| u64::from_be_bytes(x.try_into().unwrap())).ok_or_else(|| invalid(format!("Truncated box at {at}")))
}
/// Writes `value` at `at`, as u64 when `wide`, otherwise as a saturated u32
fn put(data: &mut [u8], at: usize, value: u64, wide: bool) {
    if wide {
        data[at..at + 8].copy_from_slice(&value.to_be_bytes());
    } else {
        data[at..at + 4].copy_from_slice(&(value.min(u32::MAX as u64) as u32).to_be_bytes());
    }
}

/// The boxes in `data`, and the boxes in the known containers
fn parse(data: &[u8]) -> io::Result<Vec<Atom>> {
    let mut atoms = Vec::new();
    let mut pos = 0;
    // Shorter remainders are padding (QuickTime ends `udta` with 4 zero bytes)
    while pos + 8 <= data.len() {
        let typ: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();
        let (size, header) = match be_u32(data, pos)? {
            0 => ((data.len() - pos) as u64, 8),
            1 => (be_u64(data, pos + 8)?, 16),
            size => (size as u64, 8)
        };
        if size < header as u64 || size > (data.len() - pos) as u64 {
            return Err(invalid(format!("Invalid size {size} of box {}", String::from_utf8_lossy(&typ))));
        }
        let body = &data[pos + header..pos + size as usize];
        atoms.push(if CONTAINERS.contains(&&typ) { Atom::container(&typ, parse(body)?) } else { Atom::new(&typ, body.to_vec()) });
        pos += size as usize;
    }
    Ok(atoms)
}

/// A file's `moov` (with its offset), its file-level `meta` boxes, and the size of the file
struct TopLevel {
    moov: Atom,
    moov_pos: u64,
    metas: Vec<Atom>,
    len: u64,
}

fn read_top_level<R: Read + Seek>(file: &mut R) -> io::Result<TopLevel> {
    let len = file.seek(SeekFrom::End(0))?;
    let mut moov = None;
    let mut metas = Vec::new();
    let mut pos = 0;
    while pos + 8 <= len {
        file.seek(SeekFrom::Start(pos))?;
        let mut header = [0u8; 16];
        file.read_exact(&mut header[..8])?;
        let size = match u32::from_be_bytes(header[..4].try_into().unwrap()) {
            0 => len - pos,
            1 => { file.read_exact(&mut header[8..])?; u64::from_be_bytes(header[8..].try_into().unwrap()) }
            size => size as u64
        };
        if size < 8 || size > len - pos {
            return Err(invalid(format!("Invalid size {size} of a top-level box at {pos}")));
        }
        // The first one, like the readers
        if (&header[4..8] == b"moov" && moov.is_none()) || &header[4..8] == b"meta" {
            let mut data = vec![0u8; size as usize];
            file.seek(SeekFrom::Start(pos))?;
            file.read_exact(&mut data)?;
            let atom = parse(&data)?.pop().ok_or_else(|| invalid("Empty box".into()))?;
            if &atom.typ == b"moov" { moov = Some((atom, pos)); } else { metas.push(atom); }
        }
        pos += size;
    }
    let (moov, moov_pos) = moov.ok_or_else(|| invalid("No moov box".into()))?;
    Ok(TopLevel { moov, moov_pos, metas, len })
}

/// The boxes in a `meta` and where they start in its payload: ISO `meta` is a full box, QuickTime's isn't
fn meta_children(meta: &Atom) -> Option<(usize, Vec<Atom>)> {
    let at = [4usize, 0].into_iter().find(|at| meta.payload.get(at + 4..at + 8) == Some(&b"hdlr"[..]))?;
    Some((at, parse(&meta.payload[at..]).ok()?))
}

/// Whether a file-level `meta` can be moved: its items are in it (`idat`), not at offsets in the file
fn is_self_contained(meta: &Atom) -> bool {
    let Some((_, boxes)) = meta_children(meta) else { return false };
    let Some(iloc) = boxes.iter().find(|x| &x.typ == b"iloc") else { return true };
    let p = &iloc.payload;
    let read = |pos: &mut usize, n: usize| -> Option<u64> {
        let value = p.get(*pos..*pos + n)?.iter().fold(0u64, |acc, x| acc << 8 | *x as u64);
        *pos += n;
        Some(value)
    };
    (|| {
        let version = *p.first()?;
        let (offset_size, length_size) = ((p.get(4)? >> 4) as usize, (p.get(4)? & 15) as usize);
        let (base_size, index_size) = ((p.get(5)? >> 4) as usize, if version > 0 { (p.get(5)? & 15) as usize } else { 0 });
        let id_size = if version < 2 { 2 } else { 4 };
        let mut pos = 6;
        for _ in 0..read(&mut pos, id_size)? {
            read(&mut pos, id_size)?;
            // 0: at an offset in the file, 1: in `idat`, 2: in another item
            let construction_method = if version > 0 { read(&mut pos, 2)? & 15 } else { 0 };
            // Data reference index, base offset
            read(&mut pos, 2)?;
            read(&mut pos, base_size)?;
            let extents = read(&mut pos, 2)?;
            if construction_method == 0 && extents > 0 { return Some(false); }
            pos += extents as usize * (index_size + offset_size + length_size);
        }
        Some(true)
    })().unwrap_or(false)
}

/// Timescale of a `mvhd` or `mdhd`
fn timescale(header: &Atom) -> io::Result<u32> {
    be_u32(&header.payload, if header.payload.first() == Some(&1) { 20 } else { 12 })
}

fn track_id(trak: &Atom) -> Option<u32> {
    let tkhd = &trak.child(b"tkhd")?.payload;
    be_u32(tkhd, if tkhd.first() == Some(&1) { 20 } else { 12 }).ok()
}

/// File offset and size of each chunk of a track, from its sample table. Errors when one isn't in the file (`file_len`)
fn chunks(stbl: &Atom, file_len: u64) -> io::Result<Vec<(u64, u64)>> {
    let payload = |typ: &[u8; 4]| stbl.child(typ).map(|x| x.payload.as_slice());
    let offsets: Vec<u64> = if let Some(stco) = payload(b"stco") {
        (0..be_u32(stco, 4)? as usize).map(|i| be_u32(stco, 8 + i * 4).map(u64::from)).collect::<io::Result<_>>()?
    } else if let Some(co64) = payload(b"co64") {
        (0..be_u32(co64, 4)? as usize).map(|i| be_u64(co64, 8 + i * 8)).collect::<io::Result<_>>()?
    } else {
        return Err(invalid("No chunk offsets".into()));
    };
    // The sample sizes, or the size of all of them
    let (fixed, count, sizes): (u64, usize, Vec<u64>) = if let Some(stsz) = payload(b"stsz") {
        let (fixed, count) = (be_u32(stsz, 4)? as u64, be_u32(stsz, 8)? as usize);
        if fixed != 0 { (fixed, count, Vec::new()) } else { (0, count, (0..count).map(|i| be_u32(stsz, 12 + i * 4).map(u64::from)).collect::<io::Result<_>>()?) }
    } else if let Some(stz2) = payload(b"stz2") {
        let (bits, count) = (stz2.get(7).copied().unwrap_or(0), be_u32(stz2, 8)? as usize);
        let entries = stz2.get(12..).unwrap_or_default();
        (0, count, (0..count).map(|i| match bits {
            4  => entries.get(i / 2).map(|x| (if i % 2 == 0 { x >> 4 } else { x & 15 }) as u64),
            8  => entries.get(i).map(|x| *x as u64),
            16 => entries.get(i * 2..i * 2 + 2).map(|x| u16::from_be_bytes([x[0], x[1]]) as u64),
            _  => None
        }.ok_or_else(|| invalid("Invalid stz2".into()))).collect::<io::Result<_>>()?)
    } else {
        return Err(invalid("No sample sizes".into()));
    };
    let stsc = payload(b"stsc").ok_or_else(|| invalid("No stsc".into()))?;
    let runs = (0..be_u32(stsc, 4)? as usize).map(|i| Ok((be_u32(stsc, 8 + i * 12)?, be_u32(stsc, 12 + i * 12)?))).collect::<io::Result<Vec<(u32, u32)>>>()?;

    let mut sample = 0usize;
    let mut run = 0;
    offsets.iter().enumerate().map(|(i, offset)| {
        // The last run starting at or before this chunk (1-based), the runs are in order
        let chunk = i as u64 + 1;
        while runs.get(run + 1).is_some_and(|x| x.0 as u64 <= chunk) { run += 1; }
        let per_chunk = runs.get(run).filter(|x| x.0 as u64 <= chunk).map_or(0, |x| x.1 as usize);
        let end = sample.checked_add(per_chunk).filter(|end| *end <= count).ok_or_else(|| invalid("Sample table out of range".into()))?;
        let size = if fixed != 0 { fixed * per_chunk as u64 } else { sizes[sample..end].iter().sum() };
        sample = end;
        if offset.checked_add(size).is_none_or(|end| end > file_len) {
            return Err(invalid(format!("Chunk at {offset} is past the end of the input")));
        }
        Ok((*offset, size))
    }).collect()
}

/// `value` in `from` units, in `to` units
fn rescale(value: u64, from: u32, to: u32) -> u64 {
    ((value as u128 * to as u128 + from as u128 / 2) / from.max(1) as u128) as u64
}

/// Edit list entries: duration (movie timescale), media time (track timescale, -1 for an empty edit) and rate
fn read_edits(trak: &Atom) -> io::Result<Option<Vec<(u64, i64, u32)>>> {
    let Some(elst) = trak.path(&[b"edts", b"elst"]) else { return Ok(None) };
    let p = &elst.payload;
    let wide = p.first() == Some(&1);
    let entry = if wide { 20 } else { 12 };
    (0..be_u32(p, 4)? as usize).map(|i| {
        let at = 8 + i * entry;
        Ok(if wide {
            (be_u64(p, at)?, be_u64(p, at + 8)? as i64, be_u32(p, at + 16)?)
        } else {
            (be_u32(p, at)? as u64, be_u32(p, at + 4)? as i32 as i64, be_u32(p, at + 8)?)
        })
    }).collect::<io::Result<_>>().map(Some)
}

fn write_edits(trak: &mut Atom, edits: &[(u64, i64, u32)]) {
    // Version 0 like the cameras write it, unless the values need 64 bits
    let wide = edits.iter().any(|(duration, media_time, _)| *duration > u32::MAX as u64 || i32::try_from(*media_time).is_err());
    let mut p = vec![wide as u8, 0, 0, 0];
    p.extend((edits.len() as u32).to_be_bytes());
    for (duration, media_time, rate) in edits {
        if wide {
            p.extend(duration.to_be_bytes());
            p.extend(media_time.to_be_bytes());
        } else {
            p.extend((*duration as u32).to_be_bytes());
            p.extend((*media_time as i32).to_be_bytes());
        }
        p.extend(rate.to_be_bytes());
    }
    let edts = Atom::container(b"edts", vec![Atom::new(b"elst", p)]);
    let children = trak.children.get_or_insert_with(Vec::new);
    match children.iter().position(|x| &x.typ == b"edts") {
        Some(i) => children[i] = edts,
        // After tkhd
        None => children.insert(children.iter().position(|x| &x.typ == b"tkhd").map_or(0, |i| i + 1), edts)
    }
}

/// Track references (`tref`) with the input track ids replaced by the output ones (`ids`), the others left out
fn remap_references(tref: &Atom, ids: &HashMap<u32, u32>) -> Option<Atom> {
    let references: Vec<Atom> = parse(&tref.payload).ok()?.into_iter().filter_map(|reference| {
        let targets: Vec<u8> = reference.payload.chunks_exact(4)
            .filter_map(|x| ids.get(&u32::from_be_bytes(x.try_into().unwrap())))
            .flat_map(|x| x.to_be_bytes()).collect();
        (!targets.is_empty()).then(|| Atom::new(&reference.typ, targets))
    }).collect();
    (!references.is_empty()).then(|| Atom::new(b"tref", references.iter().flat_map(Atom::to_bytes).collect()))
}

/// Makes a copied input track fit the output: its new id and references (`ids`), durations in the output movie timescale,
/// presented `shift` later (output movie timescale, earlier when negative), and its chunks at `offsets`. Returns its duration
fn adapt_trak(trak: &mut Atom, ids: &HashMap<u32, u32>, in_scale: u32, out_scale: u32, shift: i64, offsets: &[u64]) -> io::Result<u64> {
    let media_scale = trak.path(&[b"mdia", b"mdhd"]).map(timescale).transpose()?.unwrap_or(out_scale);
    let id = track_id(trak).and_then(|x| ids.get(&x).copied()).ok_or_else(|| invalid("No track id".into()))?;

    let tkhd = trak.child(b"tkhd").ok_or_else(|| invalid("No tkhd".into()))?;
    let wide = tkhd.payload.first() == Some(&1);
    let (id_at, duration_at) = if wide { (20, 28) } else { (12, 20) };
    let in_duration = if wide { be_u64(&tkhd.payload, duration_at)? } else { be_u32(&tkhd.payload, duration_at)? as u64 };
    let mut duration = rescale(in_duration, in_scale, out_scale);

    let edits = read_edits(trak)?;
    if edits.is_some() || shift != 0 {
        let mut edits: Vec<(u64, i64, u32)> = edits.unwrap_or_else(|| vec![(in_duration, 0, 0x10000)])
            .into_iter().map(|(duration, media_time, rate)| (rescale(duration, in_scale, out_scale), media_time, rate)).collect();
        if shift > 0 {
            edits.insert(0, (shift as u64, -1, 0x10000));
        } else {
            // Cut the start
            let mut cut = shift.unsigned_abs();
            for edit in edits.iter_mut() {
                let part = cut.min(edit.0);
                edit.0 -= part;
                if edit.1 >= 0 { edit.1 += rescale(part, out_scale, media_scale) as i64; }
                cut -= part;
            }
            edits.retain(|x| x.0 > 0);
        }
        duration = edits.iter().map(|x| x.0).sum();
        write_edits(trak, &edits);
    }

    let tkhd = trak.child_mut(b"tkhd").unwrap();
    put(&mut tkhd.payload, id_at, id as u64, false);
    put(&mut tkhd.payload, duration_at, duration, wide);

    // References to the other tracks, by their output ids
    let children = trak.children.get_or_insert_with(Vec::new);
    if let Some(i) = children.iter().position(|x| &x.typ == b"tref") {
        match remap_references(&children[i], ids) {
            Some(tref) => children[i] = tref,
            None => { children.remove(i); }
        }
    }

    let stbl = trak.path_mut(&[b"mdia", b"minf", b"stbl"]).ok_or_else(|| invalid("No stbl".into()))?;
    let children = stbl.children.get_or_insert_with(Vec::new);
    let wide = offsets.iter().any(|x| *x > u32::MAX as u64);
    let mut table = vec![0u8; 4];
    table.extend((offsets.len() as u32).to_be_bytes());
    for offset in offsets {
        if wide { table.extend(offset.to_be_bytes()); } else { table.extend((*offset as u32).to_be_bytes()); }
    }
    let at = children.iter().position(|x| &x.typ == b"stco" || &x.typ == b"co64").unwrap_or(children.len());
    children.retain(|x| &x.typ != b"stco" && &x.typ != b"co64");
    children.insert(at.min(children.len()), Atom::new(if wide { b"co64" } else { b"stco" }, table));
    Ok(duration)
}

/// The boxes of an iTunes-style (`mdir` handler) `meta`, and where they start in its payload
fn mdir_boxes(meta: &Atom) -> Option<(usize, Vec<Atom>)> {
    let (at, boxes) = meta_children(meta)?;
    let hdlr = boxes.iter().find(|x| &x.typ == b"hdlr")?;
    (hdlr.payload.get(8..12) == Some(&b"mdir"[..])).then_some((at, boxes))
}

/// The items of an `ilst`, none when there's no `ilst`, and `None` when it can't be read
fn ilst_items(boxes: &[Atom]) -> Option<Vec<Atom>> {
    boxes.iter().find(|x| &x.typ == b"ilst").map_or(Some(Vec::new()), |x| parse(&x.payload).ok())
}

/// `meta` with its `ilst` replaced by `items`
fn with_ilst(meta: &Atom, at: usize, mut boxes: Vec<Atom>, items: &[Atom]) -> Atom {
    let ilst = Atom::new(b"ilst", items.iter().flat_map(Atom::to_bytes).collect());
    match boxes.iter().position(|x| &x.typ == b"ilst") {
        Some(i) => boxes[i] = ilst,
        None => boxes.push(ilst)
    }
    let mut payload = meta.payload[..at].to_vec();
    payload.extend(boxes.iter().flat_map(Atom::to_bytes));
    Atom::new(b"meta", payload)
}

fn has_encoder(udta: &[Atom]) -> bool {
    udta.iter().any(|x| ENCODER_KEYS.contains(&&x.typ)) ||
    udta.iter().filter(|x| &x.typ == b"meta").filter_map(mdir_boxes).any(|(_, boxes)| ilst_items(&boxes).unwrap_or_default().iter().any(|x| ENCODER_KEYS.contains(&&x.typ)))
}

/// Merges `items` into `base`: the same keys are replaced, the others added. Without the encoder items when `keep_encoder`
fn merge_items(base: &mut Vec<Atom>, items: &[Atom], keep_encoder: bool) {
    for item in items {
        if keep_encoder && ENCODER_KEYS.contains(&&item.typ) { continue; }
        match base.iter().position(|x| x.typ == item.typ) {
            Some(i) => base[i] = item.clone(),
            None => base.push(item.clone())
        }
    }
}

/// The camera's `udta` with what FFmpeg wrote in the output's: the output's items win (like the comment), except the camera's `encoder`
fn merge_udta(camera: &[Atom], output: &[Atom]) -> Vec<Atom> {
    let keep_encoder = has_encoder(camera);
    let mut merged = camera.to_vec();
    for item in output {
        let output_mdir = if &item.typ == b"meta" { mdir_boxes(item) } else { None };
        let Some((output_at, output_boxes)) = output_mdir else {
            // Another kind of `meta`, when the camera has none
            if &item.typ != b"meta" || !merged.iter().any(|x| &x.typ == b"meta") {
                merge_items(&mut merged, std::slice::from_ref(item), keep_encoder);
            }
            continue;
        };
        let output_items = ilst_items(&output_boxes).unwrap_or_default();
        match merged.iter().enumerate().find_map(|(i, x)| mdir_boxes(x).map(|mdir| (i, mdir))) {
            Some((i, (at, boxes))) => {
                // An `ilst` that can't be read stays as it is
                if let Some(mut items) = ilst_items(&boxes) {
                    merge_items(&mut items, &output_items, keep_encoder);
                    merged[i] = with_ilst(&merged[i], at, boxes, &items);
                }
            }
            None => {
                let mut items = Vec::new();
                merge_items(&mut items, &output_items, keep_encoder);
                merged.push(with_ilst(item, output_at, output_boxes, &items));
            }
        }
    }
    merged
}

fn fs_err(e: filesystem::FilesystemError) -> io::Error { io::Error::other(format!("{e:?}")) }

/// Which of the input's `track_ids` `copy_camera_tracks` can't copy, and why. Before the render, so that only the others are left out of it:
/// once FFmpeg wrote the output without a track, it's lost when it can't be appended. Errors when the input can't be read
pub fn check_tracks(input_url: &str, track_ids: &[u32]) -> io::Result<HashMap<u32, String>> {
    let mut input = filesystem::open_file(input_url, false, false).map_err(fs_err)?;
    let TopLevel { moov, len, .. } = read_top_level(&mut input)?;
    let scale = moov.child(b"mvhd").map(timescale).ok_or_else(|| invalid("No mvhd in the input".into()))??;
    // A fragmented input has the samples in its fragments, not in the sample tables
    let fragmented = moov.child(b"mvex").is_some();
    Ok(track_ids.iter().filter_map(|&id| {
        let checked = if fragmented {
            Err(invalid("The samples are in fragments".into()))
        } else {
            moov.children.iter().flatten().find(|x| &x.typ == b"trak" && track_id(x) == Some(id)).ok_or_else(|| invalid("No trak".into())).and_then(|trak| {
                // What the copy reads of it
                let layout = chunks(trak.path(&[b"mdia", b"minf", b"stbl"]).ok_or_else(|| invalid("No stbl".into()))?, len)?;
                let offsets: Vec<u64> = layout.iter().map(|x| x.0).collect();
                adapt_trak(&mut trak.clone(), &HashMap::from([(id, id)]), scale, scale, 0, &offsets).map(|_| ())
            })
        };
        checked.err().map(|e| (id, e.to_string()))
    }).collect())
}

/// Appends the camera's `tracks` of `input_url` to the MP4/MOV output written by FFmpeg, and merges the camera's metadata (`udta`) into it,
/// except the segment table.
/// The output stays as it was until the new `moov` is written after it, then its `moov` becomes free space
pub fn copy_camera_tracks(output_url: &str, input_url: &str, tracks: &CameraTracks) -> io::Result<()> {
    if !filesystem::exists(output_url) {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("{output_url} doesn't exist")));
    }
    let mut input = filesystem::open_file(input_url, false, false).map_err(fs_err)?;
    let TopLevel { moov: in_moov, metas: in_metas, len: input_len, .. } = read_top_level(&mut input)?;

    let traks: Vec<Atom> = in_moov.children.iter().flatten()
        .filter(|x| &x.typ == b"trak" && track_id(x).is_some_and(|id| tracks.track_ids.contains(&id)))
        .cloned().collect();
    // The output's frames aren't the camera file's (the render's trim ranges, its frame rate), so the camera file's segment table
    // would describe frames that aren't there. A `udta` that held only the table has nothing to merge
    let camera_udta: Option<Vec<Atom>> = in_moov.child(b"udta").and_then(|x| x.children.as_ref()).and_then(|children| {
        let kept: Vec<Atom> = children.iter().filter(|x| &x.typ != SEGMENT_TABLE).cloned().collect();
        (kept.len() == children.len() || !kept.is_empty()).then_some(kept)
    });
    // File-level metadata (Sony's XML and lens profile)
    let file_metas: Vec<Atom> = in_metas.into_iter().filter(|meta| {
        let movable = is_self_contained(meta);
        if !movable { ::log::warn!("Not copying the camera's file-level metadata, it points to its media"); }
        movable
    }).collect();
    let camera_boxes = in_moov.children.iter().flatten().any(|x| &x.typ == b"meta" || &x.typ == b"uuid");
    if traks.is_empty() && camera_udta.is_none() && file_metas.is_empty() && !camera_boxes {
        return Ok(());
    }
    let layouts = traks.iter()
        .map(|trak| chunks(trak.path(&[b"mdia", b"minf", b"stbl"]).ok_or_else(|| invalid("No stbl".into()))?, input_len))
        .collect::<io::Result<Vec<_>>>()?;

    let mut output = filesystem::open_file(output_url, true, false).map_err(fs_err)?;
    let TopLevel { moov: mut out_moov, moov_pos: out_moov_pos, metas: out_metas, len: output_len } = read_top_level(&mut output)?;
    let in_scale = in_moov.child(b"mvhd").map(timescale).ok_or_else(|| invalid("No mvhd in the input".into()))??;
    let out_scale = out_moov.child(b"mvhd").map(timescale).ok_or_else(|| invalid("No mvhd in the output".into()))??;
    let shift = rescale(tracks.start_us.unsigned_abs(), 1_000_000, out_scale) as i64 * -tracks.start_us.signum();

    // Output track ids: of the copied tracks, after FFmpeg's, and of the input tracks FFmpeg wrote
    let out_traks: Vec<u32> = out_moov.children.iter().flatten().filter(|x| &x.typ == b"trak").filter_map(track_id).collect();
    let mut ids: HashMap<u32, u32> = tracks.ffmpeg_track_ids.iter().copied().zip(out_traks.iter().copied()).collect();
    let first_id = out_traks.iter().max().map_or(1, |x| x + 1);
    for (i, trak) in traks.iter().enumerate() {
        ids.insert(track_id(trak).unwrap(), first_id + i as u32);
    }

    // Everything is set before the file is changed. The new data goes after its end
    let mut pos = output_len + 16;
    let mut new_traks = Vec::with_capacity(traks.len());
    let mut duration = 0;
    for (mut trak, layout) in traks.into_iter().zip(layouts.iter()) {
        let offsets: Vec<u64> = layout.iter().map(|(_, size)| { pos += size; pos - size }).collect();
        ::log::debug!("Copying track {:?} of the camera as track {:?}", track_id(&trak), track_id(&trak).and_then(|x| ids.get(&x)));
        duration = duration.max(adapt_trak(&mut trak, &ids, in_scale, out_scale, shift, &offsets)?);
        new_traks.push(trak);
    }
    let total = pos - output_len - 16;

    let children = out_moov.children.get_or_insert_with(Vec::new);
    let after_traks = children.iter().rposition(|x| &x.typ == b"trak").map_or(children.len(), |i| i + 1);
    let next_id = first_id + new_traks.len() as u32;
    for (i, trak) in new_traks.into_iter().enumerate() {
        children.insert(after_traks + i, trak);
    }
    if let Some(mvhd) = children.iter_mut().find(|x| &x.typ == b"mvhd") {
        let wide = mvhd.payload.first() == Some(&1);
        let duration_at = if wide { 24 } else { 16 };
        let movie_duration = if wide { be_u64(&mvhd.payload, duration_at)? } else { be_u32(&mvhd.payload, duration_at)? as u64 };
        put(&mut mvhd.payload, duration_at, movie_duration.max(duration), wide);
        let at = mvhd.payload.len().checked_sub(4).ok_or_else(|| invalid("Truncated mvhd".into()))?;
        put(&mut mvhd.payload, at, next_id as u64, false);
    }
    if let Some(camera_udta) = &camera_udta {
        let output_udta = children.iter().find(|x| &x.typ == b"udta").and_then(|x| x.children.clone()).unwrap_or_default();
        let udta = Atom::container(b"udta", merge_udta(camera_udta, &output_udta));
        match children.iter().position(|x| &x.typ == b"udta") {
            Some(i) => children[i] = udta,
            None => children.push(udta)
        }
    }
    // Camera metadata in QuickTime keys (`mdta`), when FFmpeg didn't write any
    if let Some(meta) = in_moov.child(b"meta") {
        if !children.iter().any(|x| &x.typ == b"meta") {
            children.push(meta.clone());
        }
    }
    // The camera's own boxes (Sony `USMT`)
    for uuid in in_moov.children.iter().flatten().filter(|x| &x.typ == b"uuid") {
        if !children.iter().any(|x| &x.typ == b"uuid" && x.payload.get(..16) == uuid.payload.get(..16)) {
            children.push(uuid.clone());
        }
    }

    let file = output.get_file();
    file.seek(SeekFrom::Start(output_len))?;
    let written = (|| -> io::Result<()> {
        let mut writer = io::BufWriter::with_capacity(1 << 20, &mut *file);
        if !layouts.is_empty() {
            writer.write_all(&1u32.to_be_bytes())?;
            writer.write_all(b"mdat")?;
            writer.write_all(&(total + 16).to_be_bytes())?;
            for (offset, size) in layouts.iter().flatten() {
                input.seek(SeekFrom::Start(*offset))?;
                if io::copy(&mut (&mut input).take(*size), &mut writer)? != *size {
                    return Err(invalid(format!("Chunk at {offset} is past the end of the input")));
                }
            }
        }
        writer.write_all(&out_moov.to_bytes())?;
        if out_metas.is_empty() {
            for meta in &file_metas {
                writer.write_all(&meta.to_bytes())?;
            }
        }
        writer.flush()?;
        Ok(())
    })();
    if let Err(e) = written {
        // Back to how FFmpeg wrote it
        let _ = file.set_len(output_len);
        return Err(e);
    }
    file.sync_data()?;
    // The new `moov` is complete: the old one, the first for the readers, becomes free space
    file.seek(SeekFrom::Start(out_moov_pos + 4))?;
    file.write_all(b"free")?;
    file.flush()?;
    Ok(())
}
