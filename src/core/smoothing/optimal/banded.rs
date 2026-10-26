// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Adrian <adrian.eddy at gmail>

//! Symmetric positive definite banded matrices: the normal equations of a path smoothing problem couple each
//! sample only to its few neighbours, so the system is solved in `O(n·p²)` with a banded Cholesky factorization

/// The lower band of a symmetric `n×n` matrix with `p` sub-diagonals, row by row: `(i, j)` for `i - p <= j <= i`
/// lives at `data[i * (p + 1) + (i - j)]`
#[derive(Clone)]
pub struct BandedMatrix {
    pub n: usize,
    pub p: usize,
    data: Vec<f64>,
}

impl BandedMatrix {
    pub fn zeros(n: usize, p: usize) -> Self {
        Self { n, p, data: vec![0.0; n * (p + 1)] }
    }

    /// Adds `v` to `(i, j)` and, as the matrix is symmetric, to `(j, i)`. Only the lower band is stored, so the pair
    /// is written once; `|i - j|` has to be within the band
    #[inline]
    pub fn add(&mut self, i: usize, j: usize, v: f64) {
        let (r, c) = if i >= j { (i, j) } else { (j, i) };
        debug_assert!(r - c <= self.p);
        self.data[r * (self.p + 1) + (r - c)] += v;
    }

    #[inline]
    pub fn get(&self, i: usize, j: usize) -> f64 {
        let (r, c) = if i >= j { (i, j) } else { (j, i) };
        if r - c > self.p { return 0.0; }
        self.data[r * (self.p + 1) + (r - c)]
    }

    /// `y = A·x`
    pub fn mul_vec(&self, x: &[f64], y: &mut [f64]) {
        let (n, p) = (self.n, self.p);
        y.iter_mut().for_each(|v| *v = 0.0);
        for i in 0..n {
            let row = &self.data[i * (p + 1)..(i + 1) * (p + 1)];
            y[i] += row[0] * x[i];
            for d in 1..=p.min(i) {
                let a = row[d];
                if a != 0.0 {
                    y[i] += a * x[i - d];
                    y[i - d] += a * x[i];
                }
            }
        }
    }

    /// The Cholesky factor `L` (`A = L·Lᵀ`), in the same band layout. `None` when the matrix isn't positive definite
    pub fn cholesky(&self) -> Option<BandedCholesky> {
        let (n, p) = (self.n, self.p);
        let w = p + 1;
        let mut l = self.data.clone();
        for i in 0..n {
            let j0 = i.saturating_sub(p);
            for j in j0..=i {
                // s = A(i, j) - Σ_k L(i, k)·L(j, k) over the columns both rows have
                let k0 = j0.max(j.saturating_sub(p));
                let mut s = l[i * w + (i - j)];
                for k in k0..j {
                    s -= l[i * w + (i - k)] * l[j * w + (j - k)];
                }
                if i == j {
                    if !(s > 0.0) || !s.is_finite() { return None; }
                    l[i * w] = s.sqrt();
                } else {
                    l[i * w + (i - j)] = s / l[j * w];
                }
            }
        }
        Some(BandedCholesky { n, p, l })
    }
}

pub struct BandedCholesky {
    n: usize,
    p: usize,
    l: Vec<f64>,
}

impl BandedCholesky {
    /// Solves `A·x = b` in place
    pub fn solve_in_place(&self, b: &mut [f64]) {
        let (n, p) = (self.n, self.p);
        let w = p + 1;
        // L·y = b
        for i in 0..n {
            let mut s = b[i];
            for k in i.saturating_sub(p)..i {
                s -= self.l[i * w + (i - k)] * b[k];
            }
            b[i] = s / self.l[i * w];
        }
        // Lᵀ·x = y
        for i in (0..n).rev() {
            let mut s = b[i];
            for k in (i + 1)..(i + p + 1).min(n) {
                s -= self.l[k * w + (k - i)] * b[k];
            }
            b[i] = s / self.l[i * w];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_a_banded_system() {
        // A pentadiagonal SPD matrix: the second difference operator squared plus the identity
        let n = 50;
        let mut a = BandedMatrix::zeros(n, 2);
        for i in 0..n - 2 {
            let c = [1.0, -2.0, 1.0];
            for r in 0..3 { for s in 0..=r { a.add(i + r, i + s, c[r] * c[s] * 1e4); } }
        }
        for i in 0..n { a.add(i, i, 1.0); }
        let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.37).sin() + i as f64 * 0.01).collect();
        let mut b = vec![0.0; n];
        a.mul_vec(&x, &mut b);
        a.cholesky().unwrap().solve_in_place(&mut b);
        let err = x.iter().zip(&b).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(err < 1e-9, "max error {err}");
    }
}
