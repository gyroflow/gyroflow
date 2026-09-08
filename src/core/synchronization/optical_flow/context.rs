// SPDX-License-Identifier: GPL-3.0-or-later

use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Cancellation and the first fatal error for one optical-flow analysis.
/// This is shared by its frames, never by unrelated videos in the render queue.
#[derive(Default)]
pub struct OpticalFlowContext {
    cancel: Arc<AtomicBool>,
    error: Mutex<Option<String>>,
    retain_images: bool,
}

impl OpticalFlowContext {
    pub fn new(cancel: Arc<AtomicBool>) -> Self {
        Self {
            cancel,
            error: Mutex::new(None),
            retain_images: false,
        }
    }
    pub(crate) fn retaining_images(mut self) -> Self {
        self.retain_images = true;
        self
    }
    pub(crate) fn may_release_images(&self) -> bool {
        !self.retain_images
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    pub fn error(&self) -> Option<String> {
        self.error.lock().clone()
    }

    pub(crate) fn fail(&self, error: String) {
        let mut first = self.error.lock();
        if first.is_none() {
            log::error!("Optical flow failed: {error}");
            *first = Some(error);
        }
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_stops_its_job_and_retains_the_original_error() {
        let cancel = Arc::new(AtomicBool::new(false));
        let context = OpticalFlowContext::new(cancel.clone());
        let independent = OpticalFlowContext::default();
        context.fail("device lost".into());
        context.fail("queue disconnected".into());
        assert!(cancel.load(Ordering::Relaxed));
        assert_eq!(context.error().as_deref(), Some("device lost"));
        assert!(!independent.is_cancelled());
        assert!(independent.error().is_none());
    }

    #[test]
    fn user_cancel_does_not_report_a_backend_failure() {
        let cancel = Arc::new(AtomicBool::new(false));
        let context = OpticalFlowContext::new(cancel.clone());
        cancel.store(true, Ordering::Relaxed);
        assert!(context.is_cancelled());
        assert!(context.error().is_none());
    }
}
