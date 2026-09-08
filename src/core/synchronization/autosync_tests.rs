// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::stabilization::optical::{MotionData, MotionFrame};
use crate::synchronization::{FrameResult, OpticalFlowMethod};
use std::{cell::RefCell, collections::BTreeMap};

fn setup(with_gyro: bool) -> (StabilizationManager, SyncParams) {
    let stab = StabilizationManager::default();
    {
        let mut p = stab.params.write();
        p.fps = 30.0;
        p.frame_count = 30;
        p.duration_ms = 1000.0;
        p.size = (160, 90);
        p.output_size = (160, 90);
        p.optical_stabilization_strength = 1.0;
        p.optical_motion = Arc::new(MotionData {
            frames: vec![MotionFrame::new(0, 33_333, (160, 90), &None)],
        });
        stab.gyro.write().init_from_params(&p);
    }
    if with_gyro {
        let mut gyro = stab.gyro.write();
        gyro.file_metadata = crate::gyro_source::FileMetadata {
            has_accurate_timestamps: true,
            quaternions: BTreeMap::from([
                (0, nalgebra::UnitQuaternion::identity()),
                (1_000_000, nalgebra::UnitQuaternion::identity()),
            ]),
            ..Default::default()
        }
        .into();
        gyro.set_offset(0, 12.0);
    }
    let sync = SyncParams {
        every_nth_frame: 1,
        time_per_syncpoint: 500.0,
        search_size: 100.0,
        ..Default::default()
    };
    (stab, sync)
}

/// Supply completed estimator observations so lifecycle tests need no decoder,
/// optional OpenCV library, model weights, or GPU.
fn observations(process: &AutosyncProcess) {
    let points: Vec<_> = (0..20)
        .map(|i| (20.0 + (i % 5) as f32 * 25.0, 15.0 + (i / 5) as f32 * 15.0))
        .collect();
    for i in 0..4 {
        let video_time = i * 33_333;
        let timestamp = video_time + 2_500;
        process
            .video_timestamps
            .write()
            .insert(timestamp, video_time);
        process.estimator.sync_results.write().insert(
            timestamp,
            FrameResult {
                of_method: OpticalFlowMethod::OFOpenCVDis(
                    super::super::OFOpenCVDis::detect_features(
                        timestamp,
                        Arc::new(image::GrayImage::new(160, 90)),
                        160,
                        90,
                    ),
                ),
                frame_no: i as usize,
                timestamp_us: timestamp,
                gyro_timestamp_us: 0,
                frame_size: (160, 90),
                rotation: Some(nalgebra::Rotation3::identity()),
                quat: Some(nalgebra::UnitQuaternion::identity()),
                euler: Some((0.001, 0.002, 0.0)),
                optical_flow: RefCell::new(BTreeMap::from([(
                    1,
                    Some((
                        (timestamp, points.clone()),
                        (timestamp + 33_333, points.clone()),
                    )),
                )])),
            },
        );
    }
}

#[test]
fn cancelled_analysis_preserves_both_existing_motion_and_camera_data() {
    let (stab, sync) = setup(false);
    let original = stab.params.read().optical_motion.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut process =
        AutosyncProcess::from_manager(&stab, &[0.5], sync, "synchronize".into(), cancel.clone())
            .unwrap();
    observations(&process);
    process.on_progress(move |progress, _, _| {
        if progress >= 0.6 {
            cancel.store(true, SeqCst);
        }
    });
    process.on_finished(|_| panic!("Cancelled analysis must not publish a result"));
    process.finished_feeding_frames().unwrap();
    assert!(Arc::ptr_eq(&original, &stab.params.read().optical_motion));
    assert!(!stab.gyro.read().has_motion());
}

#[test]
fn residual_analysis_preserves_sync_and_records_video_not_sensor_timestamps() {
    let (stab, sync) = setup(true);
    let offsets = stab.gyro.read().get_offsets().clone();
    let mut process = AutosyncProcess::from_manager(
        &stab,
        &[0.5],
        sync,
        "optical_stabilization".into(),
        Arc::default(),
    )
    .unwrap();
    observations(&process);
    process.on_finished(|result| match result {
        AutosyncResult::Offsets(offsets) => assert!(offsets.is_empty()),
        _ => panic!("Unexpected analysis result"),
    });
    process.finished_feeding_frames().unwrap();
    let p = stab.params.read();
    assert_eq!(p.optical_motion.frames.len(), 3);
    assert_eq!(p.optical_motion.frames[0].timestamp_us, 0);
    assert_eq!(p.optical_motion.frames[0].next_timestamp_us, 33_333);
    assert_eq!(&offsets, stab.gyro.read().get_offsets());
}

#[test]
fn zero_frame_step_is_clamped_before_counting_frames() {
    let (stab, mut sync) = setup(true);
    sync.every_nth_frame = 0;
    assert!(
        AutosyncProcess::from_manager(&stab, &[0.5], sync, "synchronize".into(), Arc::default())
            .is_ok()
    );
}

#[test]
fn an_empty_decode_does_not_replace_existing_optical_analysis() {
    let (stab, sync) = setup(true);
    let original = stab.params.read().optical_motion.clone();
    let process = AutosyncProcess::from_manager(
        &stab,
        &[0.5],
        sync,
        "optical_stabilization".into(),
        Arc::default(),
    )
    .unwrap();
    assert!(
        process
            .finished_feeding_frames()
            .unwrap_err()
            .contains("No consecutive video frames")
    );
    assert!(Arc::ptr_eq(&original, &stab.params.read().optical_motion));
}
