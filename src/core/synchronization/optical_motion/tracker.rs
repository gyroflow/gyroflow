// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! Persistent feature tracks from one frame to the next. Every pair of consecutive frames yields the points that
//! survived a forward-backward check, with the id of the track they belong to: the analysis needs whole tracks, to
//! take the slow parallax of each point out of its motion. KLT starts its points at corners, DIS on a regular grid
//! (see `TrackingMethod`); both away from glare

use opencv::{ core::{ self, Mat, Point, Point2f, Ptr, Rect, Scalar, Size, TermCriteria, Vec2f, Vector, CV_8UC1 }, prelude::*, imgproc, video };
use super::{ Observation, TrackingMethod };

/// Grid the new corners are spread over, see `Tracker::replenish`
const GRID_ROWS: i32 = 6;
const GRID_COLS: i32 = 4;
/// What counts as a corner: its smaller eigenvalue against the strongest one in the frame
const QUALITY: f64 = 0.002;
/// ... or, in a cell with nothing that strong (cloudy sky), against the noise of flat areas, see `replenish`
const NOISE_MULTIPLE: f64 = 8.0;
/// DIS's grid: where there's at least this many times the noise of flat areas around a cell's middle, see `replenish_grid`
const DIS_NOISE_MULTIPLE: f64 = 2.0;
/// No new corners within this many radii of a compact saturated blob (the sun): its glare is a smooth ring the tracker
/// slides along and its flare is fixed to the lens, neither moves like the scene. Blobs from this many pixels up, at
/// 960 px wide - glints on water are smaller, and the robust fit takes care of them among everything else
const GLARE_RADII: f64 = 2.5;
const GLARE_MIN_AREA: f64 = 30.0;
/// KLT's window, pixels: a point sees this much around it
const WINDOW: i32 = 21;
/// How far a point may end up from where it started, tracked forward and back again, pixels
const FB_MAX: f32 = 0.1;
/// ... with DIS. Its two flows are computed apart and smoothed over patches, and disagree by more than KLT's limit
/// at half of the points of water and sky (median 0.12 px on a lake, 0.06 over a forest): at 0.1 px most of its tracks
/// broke before the 15 frames that count, and the water didn't take part at all. How far a point is off grows with
/// the disagreement (0.06 px + 0.8 times it, against KLT where KLT is sure), so it stays the limit, just looser: at
/// 0.3 px five times the measurements count on the lake (twice over the forest), each a little worse, the
/// precision of a band better for it - what's left of the shake of the lake's water in the stabilized picture went from
/// 3.4 to 1.5 px, and down in every third of the frame on both clips. Not looser still, and not looser with the motion
/// (Sundaram et al. 2010): beyond that the points it adds measure less of the motion than there is - blur, ripples -
/// which no fit averages out
const DIS_FB_MAX: f32 = 0.3;
/// DIS: the preset, and the finest level of its pyramid the flow is computed at (0: full resolution; the presets'
/// own are 1 or 2, a half or a quarter, upsampled from there)
const DIS_PRESET: i32 = video::DISOpticalFlow_PRESET_MEDIUM;
const DIS_FINEST_SCALE: i32 = 1;

pub struct Tracker {
    /// DIS, when it follows the points through the dense flow; KLT without
    dis: Option<Ptr<video::DISOpticalFlow>>,
    prev: Option<Mat>,
    size: (i32, i32),
    points: Vec<Point2f>,
    ids: Vec<u32>,
    next_id: u32,
    max_points: usize,
}

impl Tracker {
    pub fn new(max_points: usize, method: TrackingMethod) -> Result<Self, opencv::Error> {
        let dis = match method {
            TrackingMethod::Klt => None,
            TrackingMethod::Dis => {
                let mut dis = video::DISOpticalFlow::create(DIS_PRESET)?;
                if DIS_FINEST_SCALE >= 0 { dis.set_finest_scale(DIS_FINEST_SCALE)?; }
                Some(dis)
            },
        };
        Ok(Self { dis, prev: None, size: (0, 0), points: Vec::new(), ids: Vec::new(), next_id: 0, max_points })
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
                let moved = match &mut self.dis {
                    None => Self::klt(&prev, &cur, &self.points)?,
                    Some(dis) => Self::dense(dis, &prev, &cur, &self.points)?,
                };
                let mut points = Vec::with_capacity(self.points.len());
                let mut ids = Vec::with_capacity(self.points.len());
                for (i, b) in moved.into_iter().enumerate() {
                    let Some(b) = b else { continue };
                    let a = self.points[i];
                    let inside = b.x > 2.0 && b.y > 2.0 && b.x < (w - 3) as f32 && b.y < (h - 3) as f32;
                    if inside {
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

    /// Where each point went, by pyramidal Lucas-Kanade: None where it lost it, or didn't come back to where it started
    fn klt(prev: &Mat, cur: &Mat, points: &[Point2f]) -> Result<Vec<Option<Point2f>>, opencv::Error> {
        let pa: Vector<Point2f> = Vector::from_slice(points);
        let mut pb = Vector::<Point2f>::new();
        let mut pa2 = Vector::<Point2f>::new();
        let mut st = Vector::<u8>::new();
        let mut st2 = Vector::<u8>::new();
        let mut err = Vector::<f32>::new();
        let criteria = TermCriteria::new(3 /* COUNT | EPS */, 50, 0.001)?;
        video::calc_optical_flow_pyr_lk(prev, cur, &pa, &mut pb, &mut st, &mut err, Size::new(WINDOW, WINDOW), 3, criteria, 0, 1e-4)?;
        video::calc_optical_flow_pyr_lk(cur, prev, &pb, &mut pa2, &mut st2, &mut err, Size::new(WINDOW, WINDOW), 3, criteria, 0, 1e-4)?;
        (0..pa.len()).map(|i| {
            let (a, b, a2) = (pa.get(i)?, pb.get(i)?, pa2.get(i)?);
            let fb = ((a2.x - a.x).powi(2) + (a2.y - a.y).powi(2)).sqrt();
            Ok((st.get(i)? == 1 && st2.get(i)? == 1 && fb < FB_MAX).then_some(b))
        }).collect()
    }

    /// Where each point went, by the dense flow of DIS (Kroeger et al. 2016) from this frame to the next and back:
    /// None where the two don't agree at the point (`DIS_FB_MAX`). Slower than KLT, and it smooths the flow over its
    /// patches, but it has a flow everywhere, also where KLT loses its corners
    fn dense(dis: &mut Ptr<video::DISOpticalFlow>, prev: &Mat, cur: &Mat, points: &[Point2f]) -> Result<Vec<Option<Point2f>>, opencv::Error> {
        // Fresh outputs every time: DIS takes a flow passed in with the right size and type as a first guess
        let (mut forward, mut backward) = (Mat::default(), Mat::default());
        dis.calc(prev, cur, &mut forward)?;
        dis.calc(cur, prev, &mut backward)?;
        let (w, h) = (forward.cols(), forward.rows());
        let (f, b) = (forward.data_typed::<Vec2f>()?, backward.data_typed::<Vec2f>()?);
        // Bilinear, within the frame
        let sample = |flow: &[Vec2f], p: Point2f| -> Option<Point2f> {
            if !(p.x >= 0.0 && p.y >= 0.0 && p.x <= (w - 1) as f32 && p.y <= (h - 1) as f32) { return None; }
            let (x0, y0) = (p.x.floor() as i32, p.y.floor() as i32);
            let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
            let (fx, fy) = (p.x - x0 as f32, p.y - y0 as f32);
            let at = |x: i32, y: i32| flow[(y * w + x) as usize];
            let (q00, q10, q01, q11) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
            let lerp = |k: usize| (q00[k] * (1.0 - fx) + q10[k] * fx) * (1.0 - fy) + (q01[k] * (1.0 - fx) + q11[k] * fx) * fy;
            Some(Point2f::new(p.x + lerp(0), p.y + lerp(1)))
        };
        Ok(points.iter().map(|&a| {
            let moved = sample(f, a)?;
            let back = sample(b, moved)?;
            (((back.x - a.x).powi(2) + (back.y - a.y).powi(2)).sqrt() < DIS_FB_MAX).then_some(moved)
        }).collect())
    }

    /// Tops the tracks up with new corners, away from the ones already tracked: first each cell of a grid up to its
    /// share, then with whatever the cells without enough corners (clear sky) left, the strongest anywhere. All at once
    /// the strongest corners (buildings, boats, glints on water) took every track, and rows of cloudy sky - as good a
    /// reference for the rotation as anything, and the only one in their rows - got none: nothing measured them. A
    /// corner is one that stands out from the strongest in the frame, or in a cell with nothing that strong, from the
    /// noise of flat areas: clouds track as well as anything at a hundredth of a pixel back and forth, and flat sky
    /// still gets nothing
    fn replenish(&mut self, img: &Mat) -> Result<(), opencv::Error> {
        if self.dis.is_some() { return self.replenish_grid(img); }
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
        // The floor of what a corner is where nothing stronger is around: a few times the noise of flat areas
        let Some(noise) = Self::noise_level(&eig, w, h)? else { return Ok(()) };
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

    /// DIS's points: a regular grid, a new point in the middle of every cell (of about the size `max_points` of them fill
    /// the frame with) that has none left. The dense flow doesn't need a corner there, only something to see: where
    /// there's no detail at all (clear sky, clipped highlights) it only has what it smoothed in from around, so those
    /// cells stay empty - a point there would count as a measurement of rows nothing was seen in
    fn replenish_grid(&mut self, img: &Mat) -> Result<(), opencv::Error> {
        let (w, h) = self.size;
        let spacing = (w as f64 * h as f64 / self.max_points.max(1) as f64).sqrt().max(4.0);
        let (cols, rows) = ((w as f64 / spacing).floor().max(1.0) as i32, (h as f64 / spacing).floor().max(1.0) as i32);
        let (sx, sy) = (w as f64 / cols as f64, h as f64 / rows as f64);
        let mut occupied = vec![false; (cols * rows) as usize];
        for p in &self.points {
            let (c, r) = ((p.x as f64 / sx).floor() as i32, (p.y as f64 / sy).floor() as i32);
            if c >= 0 && r >= 0 && c < cols && r < rows { occupied[(r * cols + c) as usize] = true; }
        }
        if occupied.iter().all(|o| *o) { return Ok(()); }
        let mut mask = Mat::new_rows_cols_with_default(h, w, CV_8UC1, Scalar::all(255.0))?;
        self.mask_glare(img, &mut mask)?;
        let mut eig = Mat::default();
        imgproc::corner_min_eigen_val(img, &mut eig, 7, 3, core::BORDER_DEFAULT)?;
        let Some(noise) = Self::noise_level(&eig, w, h)? else { return Ok(()) };
        let floor = DIS_NOISE_MULTIPLE * noise;
        // Detail anywhere around the middle of the cell, not just at it
        let around = (spacing / 4.0).max(2.0) as i32;
        for r in 0..rows {
            for c in 0..cols {
                if occupied[(r * cols + c) as usize] { continue; }
                let (x, y) = (((c as f64 + 0.5) * sx) as i32, ((r as f64 + 0.5) * sy) as i32);
                if x < 3 || y < 3 || x >= w - 3 || y >= h - 3 || *mask.at_2d::<u8>(y, x)? == 0 { continue; }
                let near = Rect::new((x - around).max(0), (y - around).max(0), (2 * around + 1).min(w - (x - around).max(0)), (2 * around + 1).min(h - (y - around).max(0)));
                let mut detail = 0.0;
                core::min_max_loc(&Mat::roi(&eig, near)?, None, Some(&mut detail), None, None, &core::no_array())?;
                if detail >= floor { self.add(Point2f::new(x as f32, y as f32)); }
            }
        }
        Ok(())
    }

    /// The noise of flat areas in a frame's corner strengths (`corner_min_eigen_val`): their 10th percentile - sky,
    /// water - and higher still in a frame without any. Areas with none at all (clipped highlights, black bars) aren't
    /// noise: a blown-out sky took it to 0, and with it every floor made of it. None for a frame with no detail at all
    fn noise_level(eig: &Mat, w: i32, h: i32) -> Result<Option<f64>, opencv::Error> {
        let mut v: Vec<f32> = Vec::with_capacity(((w / 8 + 1) * (h / 8 + 1)) as usize);
        for y in (0..h).step_by(8) { for x in (0..w).step_by(8) { let e = *eig.at_2d::<f32>(y, x)?; if e > 0.0 { v.push(e); } } }
        if v.is_empty() { return Ok(None); }
        let i = v.len() / 10;
        Ok(Some(*v.select_nth_unstable_by(i, |a, b| a.total_cmp(b)).1 as f64))
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
