// SPDX-License-Identifier: GPL-3.0-or-later

/// Manual GPU integration check, including the 30-second idle timeout.
#[cfg(feature = "use-gmflow")]
fn main() {
    use gyroflow_core::synchronization::{OpticalFlowContext, OpticalFlowMethod, OpticalFlowTrait};
    use std::sync::{Arc, atomic::AtomicBool};
    let image = Arc::new(image::GrayImage::from_fn(576, 320, |x, y| {
        image::Luma([((x * 31 + y * 17 + x * y % 127) % 256) as u8])
    }));
    let frame = |timestamp, context| {
        OpticalFlowMethod::detect_features_with_context(
            3,
            timestamp,
            image.clone(),
            576,
            320,
            context,
        )
    };
    let cancelled = Arc::new(OpticalFlowContext::new(Arc::new(AtomicBool::new(true))));
    assert!(
        frame(0, cancelled.clone())
            .optical_flow_to(&frame(1, cancelled.clone()))
            .is_none()
    );
    assert!(cancelled.error().is_none());

    let context = Arc::new(OpticalFlowContext::default());
    let a = frame(2, context.clone());
    let b = frame(3, context.clone());
    let pairs = std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| a.optical_flow_to(&b)))
            .collect();
        jobs.into_iter()
            .map(|job| job.join().unwrap().expect("identity flow"))
            .collect::<Vec<_>>()
    });
    assert!(pairs.iter().all(|pair| pair == &pairs[0]));
    assert!(pairs[0].0.len() > 100);
    let error = pairs[0]
        .0
        .iter()
        .zip(&pairs[0].1)
        .map(|(a, b)| (a.0 - b.0).hypot(a.1 - b.1))
        .sum::<f32>()
        / pairs[0].0.len() as f32;
    assert!(error < 0.2, "identity endpoint error: {error}");
    println!("Cancellation and concurrent cache checks passed; waiting for idle release.");

    std::thread::sleep(std::time::Duration::from_secs(32));
    let reloaded = frame(4, context.clone()).optical_flow_to(&frame(5, context.clone()));
    assert!(reloaded.is_some(), "{:?}", context.error());
    assert!(!context.is_cancelled());
    println!("Inference after idle release passed.");
}

#[cfg(not(feature = "use-gmflow"))]
fn main() {
    eprintln!("gmflow_lifecycle requires --features use-gmflow and a working GPU");
    std::process::exit(1);
}
