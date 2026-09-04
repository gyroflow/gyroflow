// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1), Adrian <adrian.eddy at gmail>

use serde::{ Deserialize, Serialize };
use std::ops::{ Add, Mul, Sub };

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CatmullRom<T> {
    points: Vec<(f64, T)>,
}

impl<T> CatmullRom<T> {
    pub fn new() -> CatmullRom<T>{
        CatmullRom { points: Vec::new() }
    }
    pub fn add_point(&mut self, position: f64, value: T) {
        self.points.push((position, value));
    }
    pub fn points(&self) -> &[(f64, T)] {
        &self.points
    }
}

impl<T: Mul<f64, Output = T> + Sub<T, Output = T> + Add<T, Output = T> + Copy> CatmullRom<T> {
    pub fn interpolate(&self, t: f64) -> Option<T> {
        if self.points.len() < 2 {
            return None;
        }

        // Outside of the sampled range hold the end values, like the reference implementation does
        if t <= self.points[0].0 { return Some(self.points[0].1); }
        if t >= self.points[self.points.len() - 1].0 { return Some(self.points[self.points.len() - 1].1); }

        let lower = self
            .search_lower_cp(t)
            .filter(|x| x + 1 < self.points.len())?;

        let lower_val = &self.points[lower];
        let next_val = &self.points[lower + 1];

        let k = Self::normalize(t, lower_val.0, next_val.0);

        let lower2_val = if lower <= 0 {
            lower_val.1 * 2.0 - next_val.1
        } else {
            self.points[lower - 1].1
        };
        let next2_val = if lower + 2 >= self.points.len() {
            next_val.1 * 2.0 - lower_val.1
        } else {
            self.points[lower + 2].1
        };

        Some(Self::catmull_rom(
            k,
            lower2_val,
            lower_val.1,
            next_val.1,
            next2_val
        ))
    }

    fn normalize(val: f64, start: f64, end: f64) -> f64 {
        (val - start) / (end - start)
    }

    fn search_lower_cp(&self, t: f64) -> Option<usize> {
        let len = self.points.len();
        if len < 2 || t.is_nan() {
            return None;
        }
        match self
            .points
            .binary_search_by(|key| key.0.partial_cmp(&t).unwrap())
        {
            Err(i) if i >= len => None,
            Err(0) => None,
            Err(i) => Some(i - 1),
            Ok(i) if i == len - 1 => None,
            Ok(i) => Some(i),
        }
    }

    fn catmull_rom(t: f64, x: T, a: T, b: T, y: T) -> T {
        ((((a * 3.0 - x) - b * 3.0) + y) * 0.5) * t * t * t
            + ((b - x) * 0.5) * t
            + a
            + (((b * 4.0 + a * -5.0 + x + x) - y) * 0.5) * t * t
    }
}

// ----------------------------------------------------------------
// ----------------------------------------------------------------

pub const MAX_GRID_SIZE: usize = 9;
pub const MESH_BLOCK_SIZE: usize = 9 + MAX_GRID_SIZE * MAX_GRID_SIZE * 2 + (MAX_GRID_SIZE*MAX_GRID_SIZE*4*2);
/// Inverse mesh block + focal plane data + forward mesh block (used to refine the inverse lookup)
pub const MAX_BUFFER_SIZE: usize = MESH_BLOCK_SIZE * 2 + /*focal plane data*/20;
pub struct BivariateSpline {
    grid_size: (usize, usize)
}

impl BivariateSpline {
    pub fn new(width: usize, height: usize) -> Self {
        assert!(width <= MAX_GRID_SIZE && height <= MAX_GRID_SIZE, "Grid size is too large");
        Self { grid_size: (width, height) }
    }

    pub fn cubic_spline_coefficients(mesh: &[f64], step: usize, offset: usize, size: f64, n: usize, a: &mut [f64], b: &mut [f64], c: &mut [f64], d: &mut [f64], alpha: &mut [f64], mu: &mut [f64], z: &mut [f64]) {
        let h = size / (n - 1) as f64;
        let inv_h = 1.0 / h;
        let three_inv_h = 3.0 * inv_h;
        let h_over_3 = h / 3.0;
        let inv_3h = 1.0 / (3.0 * h);
        for i in 0..n { a[i] = mesh[(i + offset) * step]; }
        for i in 1..n - 1 { alpha[i] = three_inv_h * (a[i + 1] - 2.0 * a[i] + a[i - 1]); }

        mu[0] = 0.0;
        z[0] = 0.0;

        for i in 1..n - 1 {
            mu[i] = 1.0 / (4.0 - mu[i - 1]);
            z[i] = (alpha[i] * inv_h - z[i - 1]) * mu[i];
        }

        c[n - 1] = 0.0;

        for j in (0..n - 1).rev() {
            c[j] = z[j] - mu[j] * c[j + 1];
            b[j] = (a[j + 1] - a[j]) * inv_h - h_over_3 * (c[j + 1] + 2.0 * c[j]);
            d[j] = (c[j + 1] - c[j]) * inv_3h;
        }
    }

    fn cubic_spline_interpolate(a: &[f64], b: &[f64], c: &[f64], d: &[f64], n: usize, x: f64, size: f64) -> f64 {
        if x <= 0.0 {
            return a[0] + b[0] * x;
        }
        if x >= size {
            let h = size / (n - 1) as f64;
            return a[n - 1] + cubic_slope(b[n - 2], c[n - 2], d[n - 2], h) * (x - size);
        }

        let i = (n - 2).min(((n as f64 - 1.0) * x / size) as usize).max(0);
        let dx = x - size * i as f64 / (n - 1) as f64;
        cubic(a[i], b[i], c[i], d[i], dx)
    }

    pub fn interpolate(&self, size_x: f64, size_y: f64, mesh: &[f64], mesh_offset: usize, x: f64, y: f64) -> f64 {
        let mut intermediate_values = [0.0; MAX_GRID_SIZE];
        let mut a = [0.0; MAX_GRID_SIZE];
        let mut b = [0.0; MAX_GRID_SIZE];
        let mut c = [0.0; MAX_GRID_SIZE];
        let mut d = [0.0; MAX_GRID_SIZE];
        let mut alpha = [0.0; MAX_GRID_SIZE - 1];
        let mut mu = [0.0; MAX_GRID_SIZE];
        let mut z = [0.0; MAX_GRID_SIZE];

        let n_x = self.grid_size.0; // 9
        let n_y = self.grid_size.1; // 9

        let i = (n_x - 2).min(((n_x as f64 - 1.0) * x / size_x) as usize).max(0);
        let dx = x - size_x * i as f64 / (n_x - 1) as f64;
        let dx2 = dx * dx;

        let grid = MAX_GRID_SIZE;          // 9
        let raw_mesh_len = n_x * n_y * 2;  // x,y pairs
        let block = grid * 4;              // per-row stride for (a,b,c,d), each length 9

        let coeff_base = 9 + raw_mesh_len + (mesh_offset * n_y * block);
        let offs = coeff_base + i;

        for j in 0..n_y {
            let row_base = offs + j * block;
            intermediate_values[j] =
                mesh[row_base + (grid * 0)]
                + mesh[row_base + (grid * 1)] * dx
                + mesh[row_base + (grid * 2)] * dx2
                + mesh[row_base + (grid * 3)] * dx2 * dx;
        }

        Self::cubic_spline_coefficients(&intermediate_values, 1, 0, size_y, n_y, &mut a, &mut b, &mut c, &mut d, &mut alpha, &mut mu, &mut z);
        Self::cubic_spline_interpolate(&a, &b, &c, &d, n_y, y, size_y)
    }
}

/// Value of the segment cubic `a + b·dx + c·dx² + d·dx³`, in the form the mesh kernels evaluate it
#[inline]
fn cubic(a: f64, b: f64, c: f64, d: f64, dx: f64) -> f64 {
    a + b * dx + c * dx * dx + d * dx * dx * dx
}
/// Its slope `b + 2c·dx + 3d·dx²`
#[inline]
fn cubic_slope(b: f64, c: f64, d: f64, dx: f64) -> f64 {
    b + 2.0 * c * dx + 3.0 * d * dx * dx
}

/// Natural cubic spline through the knots `xs` (strictly increasing, any spacing) with the values `ys`: zero second
/// derivative at both ends, continued linearly with the end slopes outside the knots. These are the boundary
/// conditions of `BivariateSpline::cubic_spline_coefficients`, the uniform-spacing special case the mesh kernels
/// mirror, and on uniform knots the two agree to rounding (see the tests). Anything built from both, like the Sony
/// lens curve and its inverse (`distortion_models::sony`), therefore meets without a kink
#[derive(Debug, Clone)]
pub struct NaturalSpline {
    xs: Vec<f64>,
    a: Vec<f64>,
    b: Vec<f64>,
    c: Vec<f64>,
    d: Vec<f64>,
}
impl NaturalSpline {
    /// `None` with fewer than two knots, mismatched lengths, or knots that are not strictly increasing
    pub fn new(xs: &[f64], ys: &[f64]) -> Option<Self> {
        let n = xs.len();
        if n < 2 || ys.len() != n || xs.windows(2).any(|w| !(w[1] > w[0])) { return None; }
        let h: Vec<f64> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        // Thomas algorithm on the tridiagonal system for c (half the second derivative at the knots), c_0 = c_{n-1} = 0
        let (mut mu, mut z) = (vec![0.0; n], vec![0.0; n]);
        for i in 1..n - 1 {
            let alpha = 3.0 * ((ys[i + 1] - ys[i]) / h[i] - (ys[i] - ys[i - 1]) / h[i - 1]);
            let l = 2.0 * (h[i - 1] + h[i]) - h[i - 1] * mu[i - 1];
            mu[i] = h[i] / l;
            z[i] = (alpha - h[i - 1] * z[i - 1]) / l;
        }
        let (mut b, mut c, mut d) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
        for j in (0..n - 1).rev() {
            c[j] = z[j] - mu[j] * c[j + 1];
            b[j] = (ys[j + 1] - ys[j]) / h[j] - h[j] * (c[j + 1] + 2.0 * c[j]) / 3.0;
            d[j] = (c[j + 1] - c[j]) / (3.0 * h[j]);
        }
        Some(Self { xs: xs.to_vec(), a: ys.to_vec(), b, c, d })
    }
    /// Value at `t`
    pub fn at(&self, t: f64) -> f64 {
        let n = self.xs.len();
        if t <= self.xs[0] {
            return self.a[0] + self.b[0] * (t - self.xs[0]);
        }
        if t >= self.xs[n - 1] {
            let h = self.xs[n - 1] - self.xs[n - 2];
            return self.a[n - 1] + cubic_slope(self.b[n - 2], self.c[n - 2], self.d[n - 2], h) * (t - self.xs[n - 1]);
        }
        let i = self.xs.partition_point(|&k| k <= t).saturating_sub(1).min(n - 2);
        cubic(self.a[i], self.b[i], self.c[i], self.d[i], t - self.xs[i])
    }
    /// Half the second derivative at every knot (`c_i`), zero at both ends
    pub fn c(&self) -> &[f64] {
        &self.c
    }
}
