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
}

impl<T: Mul<f64, Output = T> + Sub<T, Output = T> + Add<T, Output = T> + Copy> CatmullRom<T> {
    pub fn interpolate(&self, t: f64) -> Option<T> {
        if self.points.len() < 2 {
            return None;
        }

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
        let i = (n - 2).min(((n as f64 - 1.0) * x / size) as usize).max(0);
        let dx = x - size * i as f64 / (n - 1) as f64;
        a[i] + b[i] * dx + c[i] * dx * dx + d[i] * dx * dx * dx
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

        let i = (self.grid_size.0 - 2).min(((self.grid_size.0 as f64 - 1.0) * x / size_x) as usize).max(0);
        let dx = x - size_x * i as f64 / (self.grid_size.0 - 1) as f64;
        let dx2 = dx*dx;
        let gs = self.grid_size.1;
        let block = gs * 4;
        let offs = 9 + gs*gs*2 + (block * gs * mesh_offset) + i;

        for j in 0..self.grid_size.1 {
            intermediate_values[j] = mesh[offs + (gs * 0) + (j * block)]
                                   + mesh[offs + (gs * 1) + (j * block)] * dx
                                   + mesh[offs + (gs * 2) + (j * block)] * dx2
                                   + mesh[offs + (gs * 3) + (j * block)] * dx2 * dx;
            // Self::cubic_spline_coefficients(&mesh[9 + mesh_offset..], 2, j * self.grid_size.0, size_x, self.grid_size.0, &mut a, &mut b, &mut c, &mut d, &mut alpha, &mut mu, &mut z);
            // intermediate_values[j] = Self::cubic_spline_interpolate(&a, &b, &c, &d, self.grid_size.0, x, size_x, i, dx);
        }

        Self::cubic_spline_coefficients(&intermediate_values, 1, 0, size_y, self.grid_size.1, &mut a, &mut b, &mut c, &mut d, &mut alpha, &mut mu, &mut z);

        Self::cubic_spline_interpolate(&a, &b, &c, &d, self.grid_size.1, y, size_y)
    }
}
