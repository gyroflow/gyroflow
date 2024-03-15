// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

use std::sync::atomic::{ AtomicBool, Ordering::SeqCst };

// Up to 32 colors
#[repr(u8)]
#[derive(Copy, Clone)]
pub enum Color {
    None,

    Red,   // #ff0000
    Green, // #00ff00
    Blue,  // #0000ff
    Yellow, // #fefb47

    // for chessboard
    Yellow2, // #C8C800
    Magenta, // #ff00ff
    Blue2, // #0080ff
    Blue3, // #00C8C8
}

#[repr(u8)]
#[derive(Copy, Clone)]
pub enum Alpha { Alpha100, Alpha75, Alpha50, Alpha25 }
impl From<u8> for Alpha { fn from(v: u8) -> Self { match v { 1 => Alpha::Alpha75, 2 => Alpha::Alpha50, 3 => Alpha::Alpha25, _ => Alpha::Alpha100 } } }
#[repr(u8)]
#[derive(Copy, Clone)]
pub enum Stage { OnInput = 0, OnOutput = 1 }

#[derive(Default)]
pub struct DrawCanvas {
    pub width: usize,
    pub height: usize,
    pub output_width: usize,
    pub output_height: usize,
    pub scale: usize,
    pub has_any_pixels: bool,
    buffer: Vec<u8>,

    drawing_cleared: AtomicBool,
}

impl DrawCanvas {
    pub fn new(width: usize, height: usize, output_width: usize, output_height: usize, scale: usize) -> Self {
        // We can either draw on input or output, so we need a big enough canvas to cover both
        let scale = scale.max(1);
        let area = width.max(output_width) * height.max(output_height);
        let mut size = area / scale;
        if size % 16 != 0 { // Align to 16 bytes (for wgpu)
            size += 16 - size % 16;
        }
        Self {
            width, height,
            output_width, output_height,
            scale,
            buffer: vec![0; size],
            has_any_pixels: false,
            drawing_cleared: AtomicBool::new(false)
        }
    }

    pub fn clear(&mut self) {
        self.buffer.fill(0);
        self.has_any_pixels = false;
    }

    pub fn put_pixel(&mut self, x: i32, mut y: i32, color: Color, alpha: Alpha, stage: Stage, y_inverted: bool, size: usize) {
        let (w, h) = self.get_size();
        if y_inverted { y = match stage { Stage::OnInput => self.height, Stage::OnOutput => self.output_height } as i32 - y; }
        if x < 0 || y < 0 || x > w as i32 * self.scale as i32 || y > h as i32 * self.scale as i32 { return; }
        let adj = if size > 2 { size as f32 / -2.0 } else { 0.0 };
        for xstep in 0..size {
            for ystep in 0..size {
                let pos = (((y as f32 / self.scale as f32 + ystep as f32 + adj).floor()) * w as f32 + (x as f32 / self.scale as f32 + xstep as f32 + adj).floor()).round() as i32;
                if pos >= 0 && pos < self.buffer.len() as i32 {
                    self.has_any_pixels = true;
                    self.buffer[pos as usize] =
                        ((color as u8) << 3) |
                        ((alpha as u8) << 1) |
                        ((stage as u8));
                }
            }
        }
    }

    pub fn get_size(&self) -> (usize, usize) {
        (
            self.width.max(self.output_width) / self.scale.max(1),
            self.height.max(self.output_height) / self.scale.max(1)
        )
    }
    pub fn get_buffer_len(&self) -> usize {
        self.buffer.len()
    }
    pub fn get_buffer(&self) -> &[u8] {
        let buf = if self.has_any_pixels || !self.drawing_cleared.load(SeqCst) {
            self.buffer.as_slice()
        } else {
            &[]
        };
        self.drawing_cleared.store(!self.has_any_pixels, SeqCst);
        buf
    }
}
