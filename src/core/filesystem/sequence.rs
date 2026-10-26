// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! Image sequences as FFmpeg names their frames, and the input files that an output would write over

use super::*;
use std::ops::RangeInclusive;
use std::sync::atomic::{ AtomicBool, Ordering::Relaxed };

/// A path as FFmpeg's image2 muxer and demuxer read it to name each frame (`ff_bprint_get_frame_filename`, libavformat
/// `utils.c`; the muxer passes `AV_FRAME_FILENAME_FLAGS_MULTIPLE`, `img2enc.c`): `%d`, or `%Nd` padded with zeros to N digits,
/// is the frame number, every one the same; `%%` is a `%`. Any other `%`, or no number, makes it no pattern: the muxer then
/// writes the first frame to the path as it is, and refuses the second. The demuxer takes one number only
pub struct FramePath {
    /// The text before each number, and after the last one
    text: Vec<String>,
    /// The width of each number
    widths: Vec<usize>,
}
impl FramePath {
    pub fn parse(path: &str) -> Option<Self> {
        let (mut text, mut widths, mut current) = (Vec::new(), Vec::new(), String::new());
        let mut chars = path.chars();
        while let Some(x) = chars.next() {
            if x != '%' {
                current.push(x);
                continue;
            }
            let mut width = 0usize;
            let mut next = chars.next();
            while let Some(digit) = next.and_then(|x| x.to_digit(10)) {
                // FFmpeg's limit
                if width >= i32::MAX as usize / 10 - 255 { return None; }
                width = width * 10 + digit as usize;
                next = chars.next();
            }
            match next {
                Some('%') => current.push('%'),
                Some('d') => { text.push(std::mem::take(&mut current)); widths.push(width); },
                _ => return None
            }
        }
        if widths.is_empty() { return None; }
        text.push(current);
        Some(Self { text, widths })
    }
    /// Whether a number is longer than any system's paths (Windows' 32,767 characters): FFmpeg can't open the first frame, and
    /// nothing is read or written. Shorter ones are left to the system to refuse
    pub fn too_long(&self) -> bool {
        self.widths.iter().any(|&x| x > 32767)
    }
    /// The name of frame `n`
    pub fn frame(&self, n: usize) -> String {
        let mut ret = self.text[0].clone();
        for (&width, text) in self.widths.iter().zip(&self.text[1..]) {
            ret.push_str(&format!("{n:0width$}"));
            ret.push_str(text);
        }
        ret
    }
    /// The folder the frames are in, and their names in it, where only the file names hold the number. On Windows a name
    /// loses its dots and spaces at the end (`GetFullPathNameW`), as the path FFmpeg opens does
    fn in_folder(&self) -> Option<(String, Self)> {
        if self.text[1..].iter().any(|x| x.contains(std::path::is_separator)) { return None; }
        let at = self.text[0].rfind(std::path::is_separator)? + 1;
        let mut text = self.text.clone();
        let dir = text[0][..at].to_owned();
        text[0] = text[0][at..].to_owned();
        if cfg!(windows) && let Some(last) = text.last_mut() {
            last.truncate(last.trim_end_matches(['.', ' ']).len());
        }
        Some((dir, Self { text, widths: self.widths.clone() }))
    }
    /// Which frame of `numbers` the file `name` can be, and whether that frame's name is exactly `name`. Compared as a file
    /// system may compare names, so that no frame there is missed: where the names are ASCII, regardless of letter case and of
    /// the characters Unicode takes for ASCII ones (U+037E, U+1FEF, U+212A); otherwise by their digits alone, which no letter
    /// case or Unicode form changes
    fn number(&self, name: &str, numbers: &RangeInclusive<usize>) -> Option<(usize, bool)> {
        let fold = if self.text.iter().all(|x| x.is_ascii()) { fold_case } else { only_digits };
        let folded = fold(name);
        let rest = folded.strip_prefix(fold(&self.text[0]).as_str())?;
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        // The first number: as many digits as its width, or more for a larger number
        let width = self.widths[0];
        (width.max(1)..=width.max(20).min(digits)).find_map(|len| {
            let n = rest[..len].parse::<usize>().ok().filter(|x| numbers.contains(x))?;
            let frame = self.frame(n);
            (fold(&frame) == folded).then(|| (n, frame == name))
        })
    }
    /// The frames of `numbers` that may be there, by the names in their folder (`folder_url`), listed once rather than each name
    /// asked for, which over a network costs a round trip a frame: each one's number, whether it's there by that name exactly,
    /// and whether the listing tells it's a file there by that name, no symbolic link nor folder. In order, without repeats.
    /// `None` where a folder's name holds the number, or the folder can't be listed. Stops when `cancel` is up
    pub fn find(&self, numbers: &RangeInclusive<usize>, folder_url: &str, cancel: &AtomicBool) -> Option<Vec<(usize, bool, bool)>> {
        let (dir, names) = self.in_folder()?;
        let mut found = Vec::new();
        start_accessing_url(folder_url, true);
        let listed = std::fs::read_dir(&dir).and_then(|entries| {
            for entry in entries {
                if cancel.load(Relaxed) { break; }
                let entry = entry?;
                if let Some((n, exact)) = entry.file_name().to_str().and_then(|x| names.number(x, numbers)) {
                    found.push((n, exact, exact && entry.file_type().is_ok_and(|x| x.is_file())));
                }
            }
            Ok(())
        });
        stop_accessing_url(folder_url, true);
        listed.ok()?;
        // The exact name first
        found.sort_unstable_by_key(|&(n, exact, _)| (n, !exact));
        found.dedup_by_key(|x| x.0);
        Some(found)
    }
}
/// `name` as file systems that ignore letter case (Windows', and macOS's and Linux's that do), or Unicode's canonical forms
/// (macOS's), may take it, as far as it's ASCII or taken for ASCII
fn fold_case(name: &str) -> String {
    name.chars().map(|x| match x { '\u{37E}' => ';', '\u{1FEF}' => '`', x => x })
        .flat_map(char::to_lowercase).flat_map(char::to_uppercase).flat_map(char::to_lowercase).collect()
}
/// The digits of `name`, which no file system takes for anything else
fn only_digits(name: &str) -> String {
    name.chars().filter(char::is_ascii_digit).collect()
}

/// The input file that an output would destroy, writing over it, as it's shown: the file `input_url`, or one of the frames
/// `input_frames` of the image sequence that it names (as FFmpeg's image2 demuxer reads it, [`FramePath`]). The output is the
/// file `filename` in `folder`, or where `output_frames` is `Some`, the image sequence it names (as the image2 muxer writes it),
/// frames 1 to `output_frames`. Only the files that are there can be written over, each through its symbolic links.
///
/// By file identity ([`FileId`]), so another spelling, a letter case variant or a link of an input file is found too: an output
/// file with one name ([`path_id`]) is looked for among the input frames by that name, in its folder, and among the frames
/// there by other names (symbolic links, letter case variants), and one with more names among every input frame, each opened.
/// A file that can't be opened counts as none: FFmpeg can't read nor write it either. Stops when `cancel` is up
pub fn output_overwrites_input(folder: &str, filename: &str, output_frames: Option<usize>, input_url: &str, input_frames: RangeInclusive<usize>, cancel: &AtomicBool) -> Option<String> {
    #[cfg(target_os = "android")]
    {
        // Documents have no paths, nor image sequences
        let _ = (output_frames, input_frames, cancel);
        return is_same_file(&get_file_url(folder, filename, false), input_url).unwrap_or_default().then(|| display_url(input_url));
    }
    #[cfg(not(target_os = "android"))]
    {
        let input_folder = get_folder(input_url);
        start_accessing_url(folder, true);
        start_accessing_url(&input_folder, true);
        let ret = overwritten_input(&written_files(folder, filename, output_frames, cancel), input_url, &input_folder, input_frames, cancel);
        stop_accessing_url(&input_folder, true);
        stop_accessing_url(folder, true);
        dbg_call!(folder filename input_url -> ret);
        ret
    }
}

/// The files there that an output writes over, through their symbolic links: see [`output_overwrites_input`]
fn written_files(folder: &str, filename: &str, frames: Option<usize>, cancel: &AtomicBool) -> Vec<PathBuf> {
    let path = url_to_path(&get_file_url(folder, filename, false));
    let there = |path: &str| std::fs::metadata(path).ok().filter(|x| x.is_file()).and_then(|_| resolve_links(Path::new(path)).ok());
    let Some(pattern) = frames.and_then(|_| FramePath::parse(&path)) else {
        return there(&path).into_iter().collect();
    };
    if pattern.too_long() { return Vec::new(); }
    let numbers = 1..=frames.unwrap_or_default().max(1);
    match pattern.find(&numbers, folder, cancel) {
        // A file there by a frame's name is the frame
        Some(found) => found.into_iter().filter_map(|(n, _, file)| if file { Some(PathBuf::from(pattern.frame(n))) } else { there(&pattern.frame(n)) }).collect(),
        // A number in a folder's name, or a folder that can't be listed: each name is asked for
        None => numbers.take_while(|_| !cancel.load(Relaxed)).filter_map(|n| there(&pattern.frame(n))).collect()
    }
}

/// The input file of [`output_overwrites_input`] that one of the files `written` is
fn overwritten_input(written: &[PathBuf], input_url: &str, input_folder: &str, input_frames: RangeInclusive<usize>, cancel: &AtomicBool) -> Option<String> {
    if written.is_empty() { return None; }
    let id = |x: &Path| path_id(x).inspect_err(|e| log::warn!("Failed to identify {}: {e:?}", x.display())).ok();
    // Their own names, which on Windows are the long ones, as the input's are
    let written = written.iter().filter_map(|x| { let (file, links) = id(x)?; Some((std::fs::canonicalize(x).unwrap_or_else(|_| x.clone()), file, links)) }).collect::<Vec<_>>();
    if let Some(input) = file_id(input_url).ok().flatten() {
        return written.iter().any(|x| x.1 == input).then(|| display_url(input_url));
    }
    // An image sequence
    let pattern = FramePath::parse(&url_to_path(input_url)).filter(|x| !x.too_long())?;
    let frame_id = |n: usize| id(Path::new(&pattern.frame(n))).map(|x| (x.0, n));
    let found = |n: usize| Some(display_url(&path_to_url(&pattern.frame(n))));
    let Some(there) = pattern.find(&input_frames, input_folder, cancel) else {
        // A number in a folder's name, or a folder that can't be listed: each frame is opened, from the first one there (FFmpeg
        // looks for it among 5 numbers) to the last before one that isn't
        let mut started = false;
        for n in input_frames.clone() {
            if cancel.load(Relaxed) { break; }
            match frame_id(n) {
                Some((file, n)) => if written.iter().any(|x| x.1 == file) { return found(n); } else { started = true; },
                None if started || n >= input_frames.start().saturating_add(4) => break,
                None => {}
            }
        }
        return None;
    };
    let (dir_path, names) = pattern.in_folder()?;
    let dir = std::cell::OnceCell::new();
    let dir = || dir.get_or_init(|| id(Path::new(&dir_path)).map(|x| x.0));
    // The frames there by other names than exactly theirs, as files: symbolic links, letter case variants. And every frame there
    let others = std::cell::OnceCell::new();
    let others = || others.get_or_init(|| there.iter().filter(|x| !x.2).filter_map(|x| frame_id(x.0)).collect::<Vec<_>>());
    let every = std::cell::OnceCell::new();
    let every = || every.get_or_init(|| there.iter().take_while(|_| !cancel.load(Relaxed)).filter_map(|x| frame_id(x.0)).collect::<Vec<_>>());
    for (path, file, links) in &written {
        if cancel.load(Relaxed) { break; }
        if *links == Some(1) {
            // Its one name: a frame's, in the frames' folder
            let number = path.file_name().and_then(|x| x.to_str()).and_then(|x| names.number(x, &input_frames));
            if let Some((n, _)) = number && path.parent().and_then(id).map(|x| x.0) == *dir() && frame_id(n).is_some_and(|x| x.0 == *file) {
                return found(n);
            }
            if let Some(x) = others().iter().find(|x| x.0 == *file) {
                return found(x.1);
            }
        } else if let Some(x) = every().iter().find(|x| x.0 == *file) {
            return found(x.1);
        }
    }
    None
}
