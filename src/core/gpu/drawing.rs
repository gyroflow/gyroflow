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

    // for the tracked points of "Analyze image optically"
    White, // #ffffff
    Sky,   // #3fb6ff
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

    /// A square of `size` pixels of the canvas at `(x, y)` (a pixel of `stage`), centred on it from 3 up: the part of it
    /// on the canvas. Each row and column is checked, not just the place in the buffer, which a column past the side
    /// would wrap into the row above or below
    pub fn put_pixel(&mut self, x: i32, y: i32, color: Color, alpha: Alpha, stage: Stage, y_inverted: bool, size: usize) {
        let (x, y) = self.to_canvas((x as f64, y as f64), stage, y_inverted);
        let adj = if size > 2 { size as f64 / -2.0 } else { 0.0 };
        let (x0, y0) = ((x + adj).floor() as i64, (y + adj).floor() as i64);
        let (w, h) = self.get_size();
        let cols = x0.clamp(0, w as i64) as usize..(x0 + size as i64).clamp(0, w as i64) as usize;
        let rows = y0.clamp(0, h as i64) as usize..(y0 + size as i64).clamp(0, h as i64) as usize;
        if cols.is_empty() { return; }
        let value = Self::value(color, alpha, stage);
        for row in rows {
            if let Some(pixels) = self.buffer.get_mut(row * w + cols.start..row * w + cols.end) {
                pixels.fill(value);
                self.has_any_pixels = true;
            }
        }
    }

    /// A line from `a` to `b` (pixels of `stage`), a pixel of the canvas wide: the part of it on the canvas. Clipped to
    /// the canvas first (Liang-Barsky), so an end however far off it (a point the lens throws far out of the picture) costs
    /// no more than one on it, and then rasterized (Bresenham)
    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), color: Color, alpha: Alpha, stage: Stage, y_inverted: bool) {
        let (w, h) = self.get_size();
        if w == 0 || h == 0 { return; }
        let a = self.to_canvas((a.0 as f64, a.1 as f64), stage, y_inverted);
        let b = self.to_canvas((b.0 as f64, b.1 as f64), stage, y_inverted);
        // Within the canvas's pixels, which end just before `w` and `h`, as `put_pixel`'s do: an end off the canvas is then in
        // the last pixel the line goes through, not one it touches at a corner
        const INSIDE: f64 = 1e-6;
        let Some((a, b)) = clip_segment(a, b, w as f64 - INSIDE, h as f64 - INSIDE) else { return };
        // The pixels the ends are in (clamped too: rounding can't take them off the canvas)
        let pixel = |p: (f64, f64)| ((p.0.floor() as i64).clamp(0, w as i64 - 1), (p.1.floor() as i64).clamp(0, h as i64 - 1));
        let value = Self::value(color, alpha, stage);
        for (x, y) in line_drawing::Bresenham::new(pixel(a), pixel(b)) {
            if let Some(px) = self.buffer.get_mut(y as usize * w + x as usize) {
                *px = value;
                self.has_any_pixels = true;
            }
        }
    }

    /// Where `(x, y)`, a pixel of `stage`, is on the canvas, in its pixels
    fn to_canvas(&self, (x, y): (f64, f64), stage: Stage, y_inverted: bool) -> (f64, f64) {
        let y = if y_inverted { (match stage { Stage::OnInput => self.height, Stage::OnOutput => self.output_height }) as f64 - y } else { y };
        let scale = self.scale.max(1) as f64;
        (x / scale, y / scale)
    }

    /// A pixel of the canvas: the colour, its alpha and the stage
    fn value(color: Color, alpha: Alpha, stage: Stage) -> u8 {
        ((color as u8) << 3) |
        ((alpha as u8) << 1) |
        ((stage as u8))
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

/// The part of the segment from `a` to `b` within `0..=w` x `0..=h` (Liang-Barsky): None when there's none, or an end
/// isn't a number
fn clip_segment(a: (f64, f64), b: (f64, f64), w: f64, h: f64) -> Option<((f64, f64), (f64, f64))> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    if !(a.0.is_finite() && a.1.is_finite() && dx.is_finite() && dy.is_finite()) { return None; }
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    // Each edge: which way the segment crosses it (`p` < 0 going in, > 0 going out) and how far inside it `a` is (`q`)
    for (p, q) in [(-dx, a.0), (dx, w - a.0), (-dy, a.1), (dy, h - a.1)] {
        if p == 0.0 {
            // Along the edge, outside of it
            if q < 0.0 { return None; }
        } else if p < 0.0 {
            t0 = t0.max(q / p);
        } else {
            t1 = t1.min(q / p);
        }
    }
    (t0 <= t1).then(|| ((a.0 + t0 * dx, a.1 + t0 * dy), (a.0 + t1 * dx, a.1 + t1 * dy)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(w: usize, h: usize, scale: usize) -> DrawCanvas { DrawCanvas::new(w, h, w, h, scale) }
    fn drawn(c: &DrawCanvas) -> Vec<(usize, usize)> {
        let (w, _) = c.get_size();
        c.buffer.iter().enumerate().filter(|(_, v)| **v != 0).map(|(i, _)| (i % w, i / w)).collect()
    }

    #[test]
    fn a_square_at_the_side_stays_on_its_rows() {
        let mut c = canvas(16, 8, 1);
        // A point at the last column, 6 wide: 4 columns of it on the canvas, none at the start of the next row
        c.put_pixel(15, 4, Color::Red, Alpha::Alpha100, Stage::OnOutput, false, 6);
        assert!(drawn(&c).iter().all(|&(x, y)| x >= 12 && (1..7).contains(&y)), "{:?}", drawn(&c));
        assert_eq!(drawn(&c).len(), 4 * 6);
        // ... and at the first one, none at the end of the row above
        let mut c = canvas(16, 8, 1);
        c.put_pixel(0, 4, Color::Red, Alpha::Alpha100, Stage::OnOutput, false, 6);
        assert!(drawn(&c).iter().all(|&(x, _)| x < 3), "{:?}", drawn(&c));
        // Off the canvas: nothing
        let mut c = canvas(16, 8, 1);
        c.put_pixel(16, 4, Color::Red, Alpha::Alpha100, Stage::OnOutput, false, 1);
        c.put_pixel(-1, 4, Color::Red, Alpha::Alpha100, Stage::OnOutput, false, 1);
        assert!(drawn(&c).is_empty() && !c.has_any_pixels);
    }

    #[test]
    fn a_line_far_off_the_canvas_is_clipped_to_it() {
        // From the middle to a point a hundred million pixels away: the part to the edge, a pixel of each column (of the
        // canvas, at half the frame's size)
        let mut c = canvas(64, 32, 2);
        c.line((32.0, 16.0), (1e8, 16.0), Color::Sky, Alpha::Alpha100, Stage::OnOutput, false);
        assert_eq!(drawn(&c), (16..32).map(|x| (x, 8)).collect::<Vec<_>>());
        // Both ends off it, across it
        let mut c = canvas(64, 32, 1);
        c.line((-1e9, -1e9), (1e9, 1e9), Color::Sky, Alpha::Alpha100, Stage::OnOutput, false);
        assert_eq!(drawn(&c), (0..32).map(|i| (i, i)).collect::<Vec<_>>());
        // Past it, not through it, and not a number
        let mut c = canvas(64, 32, 1);
        c.line((70.0, -5.0), (1e7, 40.0), Color::Sky, Alpha::Alpha100, Stage::OnOutput, false);
        c.line((10.0, 10.0), (f32::NAN, 10.0), Color::Sky, Alpha::Alpha100, Stage::OnOutput, false);
        c.line((10.0, 10.0), (f32::INFINITY, 10.0), Color::Sky, Alpha::Alpha100, Stage::OnOutput, false);
        assert!(drawn(&c).is_empty());
    }
}
