// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 dan0v <dev@dan0v.com>

use crate::gyro_source::{ GyroReplacementRegion, Quat64, TimeQuat };

/// Integrate per-frame *relative* rotations `q_t` (camera-frame, `R_{t -> t+1}`)
/// into an absolute trajectory starting from `seed`. Convention: left-multiply,
/// `cumulative = cumulative * q_t`, matching `GyroSource::integrate()`.
pub fn accumulate_relative_quats_seeded(relative_quats: &TimeQuat, seed: Quat64) -> TimeQuat {
    if relative_quats.is_empty() { return TimeQuat::new(); }
    let mut result = TimeQuat::new();
    let mut cumulative = seed;
    for (ts, q) in relative_quats.iter() {
        cumulative = cumulative * *q;
        result.insert(*ts, cumulative);
    }
    result
}

/// Dual-anchor: align `of_quats` so that its endpoints match `start_quat`/`end_quat`,
/// slerping a correction term across the segment so the trajectory interpolates to
/// `end_quat` at `end_us`. If OF coverage stops short of `end_us`, the last available
/// sample is used as the anchor and the gap is logged.
pub fn dual_anchor_of_quats(
    of_quats: &TimeQuat,
    _start_quat: Quat64,
    end_quat: Quat64,
    start_us: i64,
    end_us: i64,
) -> TimeQuat {
    if of_quats.is_empty() { return TimeQuat::new(); }

    let last_of_ts = of_quats.keys().next_back().copied().unwrap_or(end_us);
    let of_end = of_quats.range(end_us..).next()
        .or_else(|| of_quats.iter().next_back())
        .map(|(_, q)| *q)
        .unwrap_or_else(Quat64::identity);
    if last_of_ts < end_us {
        log::info!("Gyro repair: OF coverage ({}) ends before region end ({}); anchoring on last sample", last_of_ts, end_us);
    }

    let end_correction = end_quat * of_end.inverse();

    let duration = (end_us - start_us) as f64;
    of_quats.iter().map(|(ts, q)| {
        let t = if duration > 0.0 {
            ((*ts - start_us) as f64 / duration).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let correction = Quat64::identity().slerp(&end_correction, t);
        (*ts, correction * *q)
    }).collect()
}

pub fn lowpass_filter_quats(quats: &mut TimeQuat, freq: f64, sample_rate: f64) {
    if quats.len() < 3 || freq <= 0.0 || sample_rate <= 0.0 { return; }
    let _ = crate::filtering::Lowpass::filter_quats_forward_backward(freq, sample_rate, quats);
}

/// Build a replacement region. `fps` is the video fps (already known by the caller
/// from `ComputeParams::lens.fps`); it drives the lowpass cutoff.
///
/// Cutoff rationale: `fps/4` sits well below the Nyquist frequency of the discrete
/// OF-derived rate signal, suppressing per-frame jitter while preserving real camera
/// motion up to ~`fps/8` Hz. Capped at 30 Hz because higher cutoffs on high-fps
/// footage pass through OF noise that the IMU would otherwise dampen.
pub fn build_replacement_region(
    start_us: i64,
    end_us: i64,
    blend_us: i64,
    blend_method: i32,
    blend_bias: f64,
    start_quat: Quat64,
    end_quat: Quat64,
    of_quats: TimeQuat,
    fps: f64,
) -> GyroReplacementRegion {
    let mut relative = of_quats;
    let duration_us = end_us - start_us;
    if duration_us > 0 && fps > 0.0 {
        lowpass_filter_quats(&mut relative, (fps / 4.0).min(30.0), fps);
    }
    let accumulated = accumulate_relative_quats_seeded(&relative, start_quat);

    let anchored = dual_anchor_of_quats(&accumulated, start_quat, end_quat, start_us, end_us);

    GyroReplacementRegion {
        start_us,
        end_us,
        blend_us: blend_us.max(1_000),
        blend_method,
        blend_bias,
        of_quats: anchored,
        enabled: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gyro_source::Quat64;
    use nalgebra::Vector3;

    fn from_axis_angle(axis: Vector3<f64>, angle: f64) -> Quat64 {
        Quat64::from_axis_angle(&nalgebra::Unit::new_normalize(axis), angle)
    }

    #[test]
    fn accumulate_seeded_starts_at_seed() {
        let seed = from_axis_angle(Vector3::y(), 0.5);
        let mut rel = TimeQuat::new();
        rel.insert(1000, from_axis_angle(Vector3::z(), 0.1));
        rel.insert(2000, from_axis_angle(Vector3::z(), 0.1));
        let acc = accumulate_relative_quats_seeded(&rel, seed);
        assert_eq!(acc.len(), 2);
        let first = acc.get(&1000).unwrap();
        assert!(first.angle_to(&(seed * from_axis_angle(Vector3::z(), 0.1))) < 1e-12,
            "first sample must be seed * q_0");
    }

    #[test]
    fn dual_anchor_endpoints_match() {
        let start = from_axis_angle(Vector3::x(), 0.2);
        let end   = from_axis_angle(Vector3::y(), 0.7);
        let mut of = TimeQuat::new();
        // relative rotations summing to a known trajectory
        of.insert(0,    from_axis_angle(Vector3::z(), 0.05));
        of.insert(1000, from_axis_angle(Vector3::z(), 0.05));
        of.insert(2000, from_axis_angle(Vector3::z(), 0.05));
        let acc = accumulate_relative_quats_seeded(&of, start);
        let anchored = dual_anchor_of_quats(&acc, start, end, 0, 2000);
        // first anchored sample == start (correction slerps from identity)
        let first = anchored.iter().next().unwrap().1;
        assert!(first.angle_to(&start) < 1e-9, "anchor start mismatch: {}", first.angle_to(&start));
        // last anchored sample == end
        let last = anchored.iter().next_back().unwrap().1;
        assert!(last.angle_to(&end) < 1e-9, "anchor end mismatch: {}", last.angle_to(&end));
    }

    #[test]
    fn build_region_always_enabled() {
        // Regions are always enabled after analysis; the convention guard was
        // removed because the first accumulated sample is `seed * q_0`, which is
        // never equal to `seed` for non-trivial OF rotations.
        let end   = from_axis_angle(Vector3::y(), 0.9);
        let mut of = TimeQuat::new();
        of.insert(0,    from_axis_angle(Vector3::z(), 0.2));
        of.insert(1000, from_axis_angle(Vector3::z(), 0.2));
        let wrong_seed = from_axis_angle(Vector3::y(), 1.7);
        let region = build_replacement_region(0, 1000, 100, 0, 0.5, wrong_seed, end, of, 50.0);
        assert!(region.enabled, "region must be enabled after analysis");
    }

    #[test]
    fn build_region_happy_path_enabled() {
        let start = from_axis_angle(Vector3::x(), 0.0);
        let end   = from_axis_angle(Vector3::x(), 0.0);
        let mut of = TimeQuat::new();
        of.insert(0,    Quat64::identity());
        of.insert(1000, Quat64::identity());
        let region = build_replacement_region(0, 1000, 100, 0, 0.5, start, end, of, 50.0);
        assert!(region.enabled);
        // 100 us < 1 ms minimum -> clamped
        assert_eq!(region.blend_us, 1_000);
    }

    #[test]
    fn build_region_clamps_blend_minimum() {
        let start = Quat64::identity();
        let end   = Quat64::identity();
        let mut of = TimeQuat::new();
        of.insert(0,    Quat64::identity());
        of.insert(1000, Quat64::identity());
        let region = build_replacement_region(0, 1000, 0, 0, 0.5, start, end, of, 50.0);
        assert!(region.blend_us >= 1_000, "blend_us must be clamped to >= 1ms, got {}", region.blend_us);
    }
}
