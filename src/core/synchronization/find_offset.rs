// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use rayon::iter::{ ParallelIterator, IntoParallelIterator };
use std::sync::{ Arc, atomic::{ AtomicBool, Ordering::Relaxed } };
use std::collections::BTreeMap;
use crate::filtering::Lowpass;
use crate::stabilization::ComputeParams;
use super::SyncParams;

use crate::gyro_source::TimeIMU;

pub fn find_offsets<F: Fn(f64) + Sync>(ranges: &[(i64, i64)], estimated_gyro: &BTreeMap<i64, TimeIMU>, sync_params: &SyncParams, params: &ComputeParams, progress_cb: F, cancel_flag: Arc<AtomicBool>) -> Vec<(f64, f64, f64)> { // Vec<(timestamp, offset, cost)>
    let mut offsets = Vec::new();
    let gyro = &params.gyro;
    let ranges_len = ranges.len() as f64;
    if !estimated_gyro.is_empty() && gyro.duration_ms > 0.0 && !gyro.raw_imu.is_empty() {
        for (i, (from_ts, to_ts)) in ranges.iter().enumerate() {
            if cancel_flag.load(Relaxed) { break; }
            progress_cb(i as f64 / ranges_len);
            if to_ts <= from_ts { continue; }

            let mut of_item: Vec<TimeIMU> = estimated_gyro.range(from_ts..to_ts).map(|v| v.1.clone()).collect();
            if !of_item.is_empty() {
                let last_of_timestamp = of_item.last().map(|x| x.timestamp_ms).unwrap_or_default();
                let mut gyro_item: Vec<TimeIMU> = gyro.raw_imu.iter().filter_map(|x| {
                    let ts = x.timestamp_ms + sync_params.initial_offset;
                    if ts >= of_item[0].timestamp_ms - sync_params.search_size && ts <= last_of_timestamp + sync_params.search_size {
                        Some(x.clone())
                    } else {
                        None
                    }
                }).collect();

                let max_angle = get_max_angle(&of_item);
                if max_angle < 3.0 {
                    ::log::info!("No movement detected, max gyro angle: {}. Skipping sync point.", max_angle);
                    continue;
                }

                let sample_rate = gyro.raw_imu.len() as f64 / (gyro.duration_ms / 1000.0);
                let _ = Lowpass::filter_gyro_forward_backward(20.0, gyro.fps, &mut of_item);
                let _ = Lowpass::filter_gyro_forward_backward(20.0, sample_rate, &mut gyro_item);

                let gyro_bintree: BTreeMap<usize, TimeIMU> = gyro_item.into_iter().map(|x| ((x.timestamp_ms * 1000.0) as usize, x)).collect();

                let find_min = |a: (f64, f64), b: (f64, f64)| -> (f64, f64) { if a.1 < b.1 { a } else { b } };

                // First search every 1 ms
                let steps = sync_params.search_size as usize * 2;
                let lowest = (0..steps)
                    .into_par_iter()
                    .map(|i| {
                        let offs = sync_params.initial_offset - sync_params.search_size + (i as f64);
                        (offs, calculate_cost(offs, &of_item, &gyro_bintree))
                    })
                    .reduce_with(find_min)
                    .and_then(|lowest| {
                        // Then refine to 0.01 ms accuracy
                        let search_size = 2.0; // ms
                        let steps = (search_size * 100.0) as usize; // 100 times per ms
                        let step = search_size / steps as f64;
                        (0..steps)
                            .into_par_iter()
                            .map(|i| {
                                let offs = lowest.0 + (-search_size + (i as f64 * step));
                                (offs, calculate_cost(offs, &of_item, &gyro_bintree))
                            })
                            .reduce_with(find_min)
                    });

                if let Some(lowest) = lowest {
                    let middle_timestamp = (*from_ts as f64 + (to_ts - from_ts) as f64 / 2.0) / 1000.0;

                    // Only accept offsets that are within 90% of search size range
                    if (lowest.0 - sync_params.initial_offset).abs() < sync_params.search_size * 0.9 {
                        offsets.push((middle_timestamp, lowest.0, lowest.1));
                    } else {
                        log::warn!("Sync point out of acceptable range {} < {}", (lowest.0 - sync_params.initial_offset).abs(), sync_params.search_size * 0.9);
                    }
                }
            }
        }
    }
    offsets
}

fn get_max_angle(item: &[TimeIMU]) -> f64 {
    let mut max = 0.0;
    for x in item {
        if let Some(g) = x.gyro {
            if g[0].abs() > max { max = g[0].abs(); }
            if g[1].abs() > max { max = g[1].abs(); }
            if g[2].abs() > max { max = g[2].abs(); }
        }
    }
    max
}

fn gyro_at_timestamp(ts: f64, gyro: &BTreeMap<usize, TimeIMU>) -> Option<&TimeIMU> {
    gyro.range((ts * 1000.0) as usize..).next().map(|x| x.1)
}

fn calculate_cost(offs: f64, of: &[TimeIMU], gyro: &BTreeMap<usize, TimeIMU>) -> f64 {
    let mut sum = 0.0;
    let mut matches_count = 0;
    for o in of {
        if let Some(g) = gyro_at_timestamp(o.timestamp_ms - offs, gyro) {
            if let Some(gg) = g.gyro.as_ref() {
                if let Some(og) = o.gyro.as_ref() {
                    matches_count += 1;
                    sum += (gg[0] - og[0]).powi(2) * 70.0;
                    sum += (gg[1] - og[1]).powi(2) * 70.0;
                    sum += (gg[2] - og[2]).powi(2) * 100.0;
                }
            }
        }
    }
    if !of.is_empty() && matches_count > of.len() / 2 {
        // Return average sum per match, if we tested at least half of the samples
        sum / matches_count as f64
    } else {
        // Otherwise not a good match
        f64::MAX
    }
}

/*struct Translation(Vector2<f32>);
struct TranslationEstimator;

impl sample_consensus::Model<Vector2<f32>> for Translation {
    fn residual(&self, data: &Vector2<f32>) -> f64 {
        (self.0 - data).norm() as f64
    }
}

impl sample_consensus::Estimator<Vector2<f32>> for TranslationEstimator {
    type Model = Translation;
    type ModelIter = std::iter::Once<Translation>;
    const MIN_SAMPLES: usize = 1;
    fn estimate<I>(&self, mut data: I) -> Self::ModelIter
    where
        I: Iterator<Item = Vector2<f32>> + Clone,
    {
        let tr = data.next().unwrap();
        std::iter::once(Translation(tr))
    }
}

/// Return the estimated translation and the inlier matches.
fn estimate_translation(
    kp1: &[Vector2<f32>],
    kp2: &[Vector2<f32>],
    matches: &[(usize, usize)],
) -> (Vector2<f32>, Vec<usize>) {
    let mut arrsac = Arrsac::new(50.0, Xoshiro256PlusPlus::seed_from_u64(0));
    let data: Vec<_> = matches.iter().map(|(id1, id2)| {
        kp2[*id2] - kp1[*id1]
    }).collect();

    // Find inliers with RANSAC.
    let (_translation, inliers) = arrsac
        .model_inliers(&TranslationEstimator, data.iter().cloned())
        .unwrap();

    // Re-estimate translation with inliers only.
    let mut tr_sum = Vector2::zeros();
    inliers.iter().for_each(|&i| {
        tr_sum += data[i];
    });
    let tr = tr_sum / inliers.len() as f32;

    (tr, inliers)
}*/