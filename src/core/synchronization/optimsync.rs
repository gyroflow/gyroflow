// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Vladimir Pinchuk (https://github.com/VladimirP1)

use crate::gyro_source::GyroSource;
use itertools::izip;
use nalgebra::{ComplexField, Vector3};
use rand::Rng;
use rustfft::{num_complex::Complex, FftPlanner};
use std::f32::consts::PI;
use std::iter::zip;
pub struct OptimSync {
    sample_rate: f64,
    gyro: [Vec<f64>; 3],
}

fn blackman(width: usize) -> Vec<f32> {
    let a0 = 7938.0 / 18608.0;
    let a1 = 9240.0 / 18608.0;
    let a2 = 1430.0 / 18608.0;
    let mut samples = vec![0.0; width];
    let size = (width - 1) as f32;
    for i in 0..width {
        let n = i as f32;
        let v = a0 - a1 * (2.0 * PI * n / size).cos() + a2 * (4.0 * PI * n / size).cos();
        samples[i] = v;
    }
    samples
}

impl OptimSync {
    pub fn new(gyro: &GyroSource) -> Option<OptimSync> {
        let duration_ms = gyro.raw_imu.last()?.timestamp_ms - gyro.raw_imu.first()?.timestamp_ms;
        let samples_total = gyro.raw_imu.iter().filter(|x| x.gyro.is_some()).count();
        let avg_sr = samples_total as f64 / duration_ms * 1000.0;

        let interp_gyro = |ts| {
            let i_r = gyro
                .raw_imu
                .partition_point(|sample| sample.timestamp_ms < ts)
                .min(gyro.raw_imu.len() - 1);
            let i_l = i_r.max(1) - 1;

            let left = &gyro.raw_imu[i_l];
            let right = &gyro.raw_imu[i_r];
            if i_l == i_r {
                return Vector3::from_column_slice(&left.gyro.unwrap_or_default());
            }
            (Vector3::from_column_slice(&left.gyro.unwrap_or_default()) * (right.timestamp_ms - ts)
                + Vector3::from_column_slice(&right.gyro.unwrap_or_default()) * (ts - left.timestamp_ms))
                / (right.timestamp_ms - left.timestamp_ms)
        };

        let mut gyr = [Vec::<f64>::new(), Vec::<f64>::new(), Vec::<f64>::new()];
        for i in 0..((duration_ms * avg_sr / 1000.0) as usize) {
            let s = interp_gyro(i as f64 * 1000.0 / avg_sr);
            for j in 0..3 {
                gyr[j].push(s[j]);
            }
        }

        Some(OptimSync {
            sample_rate: avg_sr,
            gyro: gyr,
        })
    }

    pub fn run(
        &mut self,
        target_sync_points: usize,
        trim_start_s: f64,
        trim_end_s: f64,
    ) -> Vec<f64> {
        let gyro_c32: Vec<Vec<Complex<f32>>> = self
            .gyro
            .iter()
            .map(|v| v.iter().map(|&x| Complex::from_real(x as f32)).collect())
            .collect();

        let step_size_samples = 16;
        let nms_radius = ((self.sample_rate / 16.0 / 2.0) * 8.0) as usize; // sync points no closer than 8 seconds

        let fft_size = self.sample_rate.round() as usize;
        let scale = (1.0 / fft_size as f32).sqrt() / fft_size as f32 * 256.0;
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);

        let win = blackman(fft_size);

        let ffts: Vec<Vec<_>> = gyro_c32
            .iter()
            .map(|gyro_c32_chan| {
                gyro_c32_chan
                    .windows(fft_size)
                    .step_by(step_size_samples)
                    .map(|chunk| {
                        let mut cm: Vec<_> = zip(chunk, &win).map(|(x, y)| x * y).collect();
                        fft.process(&mut cm);
                        zip(cm.iter(), cm.iter().rev())
                            .take(fft_size / 2)
                            .map(|(a, b)| a + b)
                            .map(|x| x.norm() * scale)
                            .collect::<Vec<_>>()
                    })
                    .collect()
            })
            .collect();

        let map_to_bin = |freq: f64| {
            (fft_size as f64 / self.sample_rate * freq)
                .round()
                .max(0.0)
                .min((fft_size / 2 - 1) as f64) as usize
        };

        let band_energy = |axis: &Vec<Vec<f32>>, begin, end| {
            let f: Vec<_> = axis
                .iter()
                .map(|bins| bins[map_to_bin(begin)..map_to_bin(end)].iter().sum::<f32>())
                .collect();
            f
        };

        fn sum_vec_f32(a: &[f32], b: &[f32]) -> Vec<f32> {
            zip(a, b).map(|(a, b)| a + b).collect()
        }
        let merged_ffts: Vec<_> = izip!(&ffts[0], &ffts[1], &ffts[2])
            .map(|(x, y, z)| sum_vec_f32(&sum_vec_f32(x, y), z))
            .collect();

        let lf = band_energy(&merged_ffts, 0.0, 2.0);
        let mf = band_energy(&merged_ffts, 2.0, 30.0);
        let hf = band_energy(&merged_ffts, 30.0, 2000.0);

        let mut rank: Vec<_> = izip!(&lf, &mf, &hf)
            .map(|(lf, mf, hf)| {
                // we do not like low freqs and high freqs, but mid freqs are good
                mf / (1.0 + nlfunc(*hf, 450.0) * 0.003) / (1.0 + nlfunc(*lf, 650.0) * 0.003)
            })
            .collect();

        for i in 0..rank.len() {
            if rank[i] < 100.0
                || (i * step_size_samples) as f64 / self.sample_rate < trim_start_s
                || (i * step_size_samples) as f64 / self.sample_rate > trim_end_s
            {
                rank[i] = 0.0;
            }
        }

        let mut rank_nms = rank.clone();
        for i in 0..rank.len() {
            for j in
                (i as i64 - nms_radius as i64).max(0) as usize..(i + nms_radius).min(rank.len() - 1)
            {
                if rank[j] < rank[i] {
                    rank_nms[j] = 0.0;
                }
            }
        }

        let mut sync_points = Vec::<f64>::new();
        for i in 0..rank.len() {
            if rank_nms[i] > 0.1 {
                sync_points.push(
                    (i as f64 * step_size_samples as f64 + fft_size as f64 / 2.0)
                        / self.sample_rate
                        * 1000.0,
                );
            }
        }

        let mut selected_sync_points = Vec::<f64>::new();
        let mut rng = rand::thread_rng();
        for _ in 0..target_sync_points {
            if sync_points.is_empty() { break; }
            let rnd = rng.gen_range(trim_start_s * 1000.0..trim_end_s * 1000.0);
            let mut p = sync_points.partition_point(|x| x < &rnd).min(sync_points.len() - 1);
            if (sync_points[(p as i64-1).max(0) as usize] - rnd).abs() < (sync_points[p] - rnd) {
                p -= 1;
            }
            selected_sync_points.push(sync_points[p]);
            sync_points.remove(p);
        }

        // use inline_python::python;
        // python! {
        //     import matplotlib.pyplot as plt
        //     import os

        //     plt.plot('lf, label = "lf", alpha = .3)
        //     plt.plot('mf, label = "mf", alpha = .3)
        //     plt.plot('hf, label = "hf", alpha = .3)

        //     plt.plot('rank, label = "rank")
        //     plt.plot('rank_nms, label = "rank_nms")

        //     plt.legend()
        //     plt.tight_layout()
        //     fig = plt.gcf()
        //     fig.set_size_inches(10, 5)
        //     plt.show()
        // }
        selected_sync_points.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        selected_sync_points
    }
}

pub fn nlfunc(arg: f32, trip_point: f32) -> f32 {
    if arg < trip_point {
        0.0
    } else {
        arg - trip_point
    }
}
