// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;
use std::sync::atomic::Ordering;
use crate::core::StabilizationManager;
use crate::rendering::RenderQueue;

#[test]
fn queue_runs_autosync_when_no_gyro() {
    // Create a minimal StabilizationManager with a video that has no gyro data.
    let stab = StabilizationManager::new_dummy_without_gyro();
    let mut queue = RenderQueue::default();
    // Add a job to the queue (additional_data can be empty for this test).
    let job_id = queue.add_file("dummy.mp4".to_string(), "".to_string(), "{}".to_string());
    // Retrieve the job and ensure the gyro_outdated flag was set to true.
    let job = queue.jobs.get(&job_id).expect("job not found");
    assert_eq!(job.gyro_outdated.load(Ordering::SeqCst), true);
    // Starting the queue should not panic and should invoke autosync internally.
    queue.start();
    // After processing, the manager should now contain generated motion data.
    assert!(stab.gyro.read().has_motion(), "autosync did not generate gyro data");
}
