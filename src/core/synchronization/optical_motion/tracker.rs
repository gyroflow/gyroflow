// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! Persistent KLT feature tracks from one frame to the next. Every pair of consecutive frames yields the points that
//! survived a forward-backward check, with the id of the track they belong to: the analysis needs whole tracks, to
//! take the slow parallax of each point out of its motion

use opencv::{ core::{ self, Mat, Point, Point2f, Rect, Scalar, Size, TermCriteria, Vector, CV_8UC1 }, prelude::*, imgproc, video };
use super::Observation;

/// Grid the new corners are spread over, see `KltTracker::replenish`
const GRID_ROWS: i32 = 6;
const GRID_COLS: i32 = 4;
/// What counts as a corner: its smaller eigenvalue against the strongest one in the frame
const QUALITY: f64 = 0.002;
/// ... or, in a cell with nothing that strong (cloudy sky), against the noise of flat areas, see `replenish`
const NOISE_MULTIPLE: f64 = 8.0;
/// No new corners within this many radii of a compact saturated blob (the sun): its glare is a smooth ring the tracker
/// slides along and its flare is fixed to the lens, neither moves like the scene. Blobs from this many pixels up, at
/// 960 px wide - glints on water are smaller, and the robust fit takes care of them among everything else
const GLARE_RADII: f64 = 2.5;
const GLARE_MIN_AREA: f64 = 30.0;

pub struct KltTracker {
    prev: Option<Mat>,
    size: (i32, i32),
    points: Vec<Point2f>,
    ids: Vec<u32>,
    next_id: u32,
    max_points: usize,
}

impl KltTracker {
    pub fn new(max_points: usize) -> Self {
        Self { prev: None, size: (0, 0), points: Vec::new(), ids: Vec::new(), next_id: 0, max_points }
    }

    /// Forget everything: the next frame starts new tracks (a gap in the decoded frames, a cut)
    pub fn reset(&mut self) {
        self.prev = None;
        self.points.clear();
        self.ids.clear();
    }

    /// Tracks the points of the previous frame into this one. Empty for the first frame
    pub fn track(&mut self, width: u32, height: u32, stride: usize, pixels: &[u8]) -> Result<Vec<Observation>, opencv::Error> {
        let (w, h) = (width as i32, height as i32);
        if pixels.len() < stride * height as usize { return Ok(Vec::new()); }
        let cur = unsafe { Mat::new_size_with_data_unsafe(Size::new(w, h), CV_8UC1, pixels.as_ptr() as *mut std::ffi::c_void, stride) }?;
        if self.size != (w, h) { self.reset(); }
        self.size = (w, h);

        let mut out = Vec::new();
        if let Some(prev) = self.prev.take() {
            self.replenish(&prev)?;
            if !self.points.is_empty() {
                let pa: Vector<Point2f> = Vector::from_slice(&self.points);
                let mut pb = Vector::<Point2f>::new();
                let mut pa2 = Vector::<Point2f>::new();
                let mut st = Vector::<u8>::new();
                let mut st2 = Vector::<u8>::new();
                let mut err = Vector::<f32>::new();
                let criteria = TermCriteria::new(3 /* COUNT | EPS */, 50, 0.001)?;
                video::calc_optical_flow_pyr_lk(&prev, &cur, &pa, &mut pb, &mut st, &mut err, Size::new(21, 21), 3, criteria, 0, 1e-4)?;
                video::calc_optical_flow_pyr_lk(&cur, &prev, &pb, &mut pa2, &mut st2, &mut err, Size::new(21, 21), 3, criteria, 0, 1e-4)?;

                let mut points = Vec::with_capacity(self.points.len());
                let mut ids = Vec::with_capacity(self.points.len());
                for i in 0..pa.len() {
                    let (a, b, a2) = (pa.get(i)?, pb.get(i)?, pa2.get(i)?);
                    let fb = ((a2.x - a.x).powi(2) + (a2.y - a.y).powi(2)).sqrt();
                    let inside = b.x > 2.0 && b.y > 2.0 && b.x < (w - 3) as f32 && b.y < (h - 3) as f32;
                    if st.get(i)? == 1 && st2.get(i)? == 1 && fb < 0.1 && inside {
                        out.push(Observation { id: self.ids[i], a: [a.x, a.y], b: [b.x, b.y] });
                        points.push(b);
                        ids.push(self.ids[i]);
                    }
                }
                self.points = points;
                self.ids = ids;
            }
        }
        self.prev = Some(cur.try_clone()?);
        Ok(out)
    }

    /// Tops the tracks up with new corners, away from the ones already tracked: first each cell of a grid up to its
    /// share, then with whatever the cells without enough corners (clear sky) left, the strongest anywhere. All at once
    /// the strongest corners (buildings, boats, glints on water) took every track, and rows of cloudy sky - as good a
    /// reference for the rotation as anything, and the only one in their rows - got none: nothing measured them. A
    /// corner is one that stands out from the strongest in the frame, or in a cell with nothing that strong, from the
    /// noise of flat areas: clouds track as well as anything at a hundredth of a pixel back and forth, and flat sky
    /// still gets nothing
    fn replenish(&mut self, img: &Mat) -> Result<(), opencv::Error> {
        if self.points.len() >= self.max_points { return Ok(()); }
        let (w, h) = self.size;
        let min_distance = (w.max(h) as f64 / 120.0).max(4.0);
        let mut mask = Mat::new_rows_cols_with_default(h, w, CV_8UC1, Scalar::all(255.0))?;
        for p in &self.points {
            imgproc::circle(&mut mask, Point::new(p.x as i32, p.y as i32), min_distance as i32, Scalar::all(0.0), -1, imgproc::LINE_8, 0)?;
        }
        self.mask_glare(img, &mut mask)?;
        let mut eig = Mat::default();
        imgproc::corner_min_eigen_val(img, &mut eig, 7, 3, core::BORDER_DEFAULT)?;
        let mut strongest = 0.0;
        core::min_max_loc(&eig, None, Some(&mut strongest), None, None, &mask)?;
        if strongest <= 0.0 { return Ok(()); } // A flat frame: black, a fade
        let threshold = QUALITY * strongest;
        // The floor of what a corner is where nothing stronger is around: a few times the noise of flat areas (the 10th
        // percentile of the frame's corner strengths - sky, water - and higher still in a frame without any). Areas
        // with none at all (clipped highlights, black bars) aren't noise: a blown-out sky took it to 0 and with it
        // the floor
        let noise = {
            let mut v: Vec<f32> = Vec::with_capacity(((w / 8 + 1) * (h / 8 + 1)) as usize);
            for y in (0..h).step_by(8) { for x in (0..w).step_by(8) { let e = *eig.at_2d::<f32>(y, x)?; if e > 0.0 { v.push(e); } } }
            if v.is_empty() { return Ok(()); }
            let i = v.len() / 10;
            *v.select_nth_unstable_by(i, |a, b| a.total_cmp(b)).1 as f64
        };
        let floor = NOISE_MULTIPLE * noise;
        let share = self.max_points / (GRID_ROWS * GRID_COLS) as usize;
        for r in 0..GRID_ROWS {
            for c in 0..GRID_COLS {
                let (x0, y0) = (w * c / GRID_COLS, h * r / GRID_ROWS);
                let cell = Rect::new(x0, y0, w * (c + 1) / GRID_COLS - x0, h * (r + 1) / GRID_ROWS - y0);
                let has = self.points.iter().filter(|p| cell.contains(Point::new(p.x as i32, p.y as i32))).count();
                if has >= share { continue; }
                let mut corners = Vector::<Point2f>::new();
                {
                    let (eig_cell, mask_cell, img_cell) = (Mat::roi(&eig, cell)?, Mat::roi(&mask, cell)?, Mat::roi(img, cell)?);
                    let mut cell_strongest = 0.0;
                    core::min_max_loc(&eig_cell, None, Some(&mut cell_strongest), None, None, &mask_cell)?;
                    let cell_threshold = threshold.min(floor.max(QUALITY * cell_strongest));
                    if cell_threshold <= 0.0 || cell_strongest < cell_threshold { continue; }
                    imgproc::good_features_to_track(&img_cell, &mut corners, (share - has) as i32, cell_threshold / cell_strongest, min_distance, &mask_cell, 7, false, 0.04)?;
                }
                for p in corners {
                    let p = Point2f::new(p.x + x0 as f32, p.y + y0 as f32);
                    imgproc::circle(&mut mask, Point::new(p.x as i32, p.y as i32), min_distance as i32, Scalar::all(0.0), -1, imgproc::LINE_8, 0)?;
                    self.add(p);
                }
            }
        }
        if self.points.len() >= self.max_points { return Ok(()); }
        let mut corners = Vector::<Point2f>::new();
        imgproc::good_features_to_track(img, &mut corners, (self.max_points - self.points.len()) as i32, QUALITY, min_distance, &mask, 7, false, 0.04)?;
        for p in corners { self.add(p); }
        Ok(())
    }

    /// Keeps new corners away from the sun and the like, see `GLARE_RADII`. Only compact blobs, not too large: the edges
    /// of an overexposed white wall are as good as any
    fn mask_glare(&self, img: &Mat, mask: &mut Mat) -> Result<(), opencv::Error> {
        let w = self.size.0 as f64;
        let mut bright = Mat::default();
        imgproc::threshold(img, &mut bright, 249.0, 255.0, imgproc::THRESH_BINARY)?;
        let (mut labels, mut stats, mut centroids) = (Mat::default(), Mat::default(), Mat::default());
        let n = imgproc::connected_components_with_stats(&bright, &mut labels, &mut stats, &mut centroids, 8, core::CV_32S)?;
        for i in 1..n {
            let area = *stats.at_2d::<i32>(i, imgproc::CC_STAT_AREA)? as f64;
            let at = |c: i32| -> Result<f64, opencv::Error> { Ok(*stats.at_2d::<i32>(i, c)? as f64) };
            let (bx, by, bw, bh) = (at(imgproc::CC_STAT_LEFT)?, at(imgproc::CC_STAT_TOP)?, at(imgproc::CC_STAT_WIDTH)?, at(imgproc::CC_STAT_HEIGHT)?);
            // The frame's edge may cut it (a half disc is twice as wide as tall): its size is the larger side then
            let cut = bx <= 0.0 || by <= 0.0 || bx + bw >= w || by + bh >= self.size.1 as f64;
            let compact = area >= 0.5 * bw * bh && bw.max(bh) <= if cut { 3.0 } else { 2.0 } * bw.min(bh);
            let radius = bw.max(bh) / 2.0;
            if area < GLARE_MIN_AREA * (w / 960.0).powi(2) || !compact || radius > w / 12.0 { continue; }
            let (cx, cy) = (bx + bw / 2.0, by + bh / 2.0);
            imgproc::circle(mask, Point::new(cx as i32, cy as i32), (GLARE_RADII * radius) as i32, Scalar::all(0.0), -1, imgproc::LINE_8, 0)?;
        }
        Ok(())
    }

    fn add(&mut self, p: Point2f) {
        self.points.push(p);
        self.ids.push(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
    }
}
