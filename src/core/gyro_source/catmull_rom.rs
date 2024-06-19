// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2024 Vladimir Pinchuk (https://github.com/VladimirP1)

use serde::{ Deserialize, Serialize };
use std::ops::{ Add, Mul, Sub };

#[derive(Debug, Copy, Clone, Default, Deserialize, Serialize)]
pub struct Vector3f {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Mul<f64> for Vector3f {
    type Output = Vector3f;
    fn mul(self, rhs: f64) -> Self::Output {
        Vector3f {
            x: self.x * rhs,
            y: self.y * rhs,
            z: self.z * rhs,
        }
    }
}

impl Add<Vector3f> for Vector3f {
    type Output = Vector3f;
    fn add(self, rhs: Vector3f) -> Self::Output {
        Vector3f {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}

impl Sub<Vector3f> for Vector3f {
    type Output = Vector3f;
    fn sub(self, rhs: Vector3f) -> Self::Output {
        Vector3f {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CatmullRom<T> {
    pub points: Vec<(f64, T)>,
}

impl<T> CatmullRom<T> {
    pub fn new() -> CatmullRom<T>{
        CatmullRom { points: Vec::new() }
    }
}

impl<T: Mul<f64, Output = T> + Sub<T, Output = T> + Add<T, Output = T> + Copy> CatmullRom<T> {
    pub fn interpolate(&self, t: f64) -> Option<T> {
        let lower = self
            .search_lower_cp(t)
            .filter(|x| x + 1 < self.points.len())?;

        let lower_val = &self.points[lower];
        let next_val = &self.points[lower + 1];

        let k = Self::normalize(t, lower_val.0, next_val.0);

        let lower2_val = if lower <= 0 {
            lower_val.1 * 2.0 - next_val.1
        } else {
            self.points[lower - 1].1.clone()
        };
        let next2_val = if lower + 2 >= self.points.len() {
            next_val.1 * 2.0 - lower_val.1
        } else {
            self.points[lower + 2].1.clone()
        };

        Some(Self::catmull_rom(
            k,
            lower2_val,
            lower_val.1,
            next_val.1,
            next2_val,
        ))
    }

    fn normalize(val: f64, start: f64, end: f64) -> f64 {
        (val - start) / (end - start)
    }

    fn search_lower_cp(&self, t: f64) -> Option<usize> {
        let len = self.points.len();
        if len < 2 {
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
        return ((((a * 3.0 - x) - b * 3.0) + y) * 0.5) * t * t * t
            + ((b - x) * 0.5) * t
            + a
            + (((b * 4.0 + a * -5.0 + x + x) - y) * 0.5) * t * t;
    }
}
