#![allow(dead_code)]
/*
use cpp::*;

cpp! {{
    #include <memory>
    #include "core_private.hpp"

    struct SyncProblemWrapper {
        SyncProblemWrapper() { inner = std::make_unique<SyncProblemPrivate>(); }
        std::unique_ptr<SyncProblemPrivate> inner;
    };
}}

cpp_class! { pub unsafe struct SyncProblemWrapper as "SyncProblemWrapper" }

impl SyncProblemWrapper {
    pub fn new() -> Self { Self::default() }

    #[allow(dead_code)]
    pub fn set_gyro_quaternions_fixed(&mut self, data: &[(f64, f64, f64, f64)], sample_rate: f64, first_timestamp: f64) {
        let data = data.into_iter().flat_map(|v| [v.0, v.1, v.2, v.3]).collect::<Vec<_>>();
        let count = data.len();

        let data = data.as_ptr();
        cpp!(unsafe [self as "SyncProblemWrapper *", data as "double *", count as "size_t", sample_rate as "double", first_timestamp as "double"] {
            self->inner->SetGyroQuaternions(data, count, sample_rate, first_timestamp);
        });
    }

    pub fn set_gyro_quaternions(&mut self, timestamps_us: &[i64], quats: &[(f64, f64, f64, f64)]) {
        let quats = quats.into_iter().flat_map(|v| [v.0, v.1, v.2, v.3]).collect::<Vec<_>>();

        let quat_count = timestamps_us.len() as u32;
        let quats = quats.as_ptr();
        let timestamps = timestamps_us.as_ptr();
        cpp!(unsafe [self as "SyncProblemWrapper *", quats as "double *", timestamps as "int64_t *", quat_count as "uint32_t"] {
            self->inner->SetGyroQuaternions(timestamps, quats, quat_count);
        });
    }

    pub fn set_track_result(&mut self, timestamp_us: i64, ts_a: &[f64], ts_b: &[f64], rays_a: &[(f64, f64, f64)], rays_b: &[(f64, f64, f64)]) {
        let rays_a = rays_a.into_iter().flat_map(|v| [v.0, v.1, v.2]).collect::<Vec<_>>();
        let rays_b = rays_b.into_iter().flat_map(|v| [v.0, v.1, v.2]).collect::<Vec<_>>();

        let points_len = ts_a.len() as u32;
        let tss_a = ts_a.as_ptr();
        let tss_b = ts_b.as_ptr();
        let points3d_a = rays_a.as_ptr();
        let points3d_b = rays_b.as_ptr();
        cpp!(unsafe [self as "SyncProblemWrapper *", timestamp_us as "int64_t", tss_a as "double *", tss_b as "double *", points3d_a as "double *", points3d_b as "double *", points_len as "uint32_t"] {
            self->inner->SetTrackResult(timestamp_us, tss_a, tss_b, points3d_a, points3d_b, points_len);
        });
    }

    pub fn pre_sync(&self, initial_delay: f64, ts_from: i64, ts_to: i64, search_step: f64, search_radius: f64) -> (f64, f64) {
        cpp!(unsafe [self as "SyncProblemWrapper *", ts_from as "int64_t", ts_to as "int64_t", search_step as "double", search_radius as "double", initial_delay as "double"] -> (f64, f64) as "std::pair<double, double>" {
            return self->inner->PreSync(initial_delay, ts_from, ts_to, search_step, search_radius);
        })
    }

    pub fn sync(&self, initial_delay: f64, ts_from: i64, ts_to: i64) -> (f64, f64) {
        cpp!(unsafe [self as "SyncProblemWrapper *", ts_from as "int64_t", ts_to as "int64_t", initial_delay as "double"] -> (f64, f64) as "std::pair<double, double>" {
            return self->inner->Sync(initial_delay, ts_from, ts_to);
        })
    }

    pub fn debug_pre_sync(&self, initial_delay: f64, ts_from: i64, ts_to: i64, search_radius: f64, delays: &mut [f64], costs: &mut [f64], point_count: usize) {
        let delays = delays.as_mut_ptr();
        let costs = costs.as_mut_ptr();
        cpp!(unsafe [self as "SyncProblemWrapper *", ts_from as "int64_t", ts_to as "int64_t", search_radius as "double", delays as "double *", costs as "double *", point_count as "size_t", initial_delay as "double"] {
            self->inner->DebugPreSync(initial_delay, ts_from, ts_to, search_radius, delays, costs, point_count);
        });
    }
}
*
pub type SyncProblem = SyncProblemWrapper;

*/

#[derive(Default, Debug, ::serde::Serialize, ::serde::Deserialize)]
pub struct PerFrame {
    pub timestamp_us: i64,
    pub pointsa: Vec<(f64, f64, f64)>,
    pub pointsb: Vec<(f64, f64, f64)>,
    pub tsa: Vec<f64>,
    pub tsb: Vec<f64>,
}
#[derive(Default, Debug, ::serde::Serialize, ::serde::Deserialize)]
pub struct Serialized {
    pub timestamps: Vec<i64>,
    pub quats: Vec<(f64, f64, f64, f64)>,
    pub perframe: Vec<PerFrame>,

    pub frame_ro: f64,

    pub from_ts: i64,
    pub to_ts: i64,
    pub presync_step: f64,
    pub presync_radius: f64,
    pub initial_delay: f64,
}

pub fn save_data_to_file(data: &Serialized, path: &str) {
    std::fs::write(path, bincode::serialize(&data).unwrap()).unwrap();
}

pub fn load_data_from_file(path: &str) -> Serialized {
    bincode::deserialize(&std::fs::read(path).unwrap()).unwrap()
}
