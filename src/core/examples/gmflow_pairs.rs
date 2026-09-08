// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(feature = "use-gmflow")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use gyroflow_core::synchronization::{
        GrayImage, OpticalFlowContext, OpticalFlowMethod, OpticalFlowTrait,
    };
    use std::{env, fs, sync::Arc, time::Instant};
    struct Logger;
    impl log::Log for Logger {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::Level::Info
        }
        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                eprintln!("{}: {}", record.level(), record.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: Logger = Logger;
    log::set_logger(&LOGGER).map_err(|error| error.to_string())?;
    log::set_max_level(log::LevelFilter::Info);
    let args: Vec<_> = env::args().collect();
    if args.len() != 6 {
        return Err(
            "usage: gmflow_pairs <width> <height> <stride> <gray8-sequence> <matches.json>".into(),
        );
    }
    let width = args[1].parse::<u32>()?;
    let height = args[2].parse::<u32>()?;
    let stride = args[3].parse::<u32>()?;
    if width == 0 || height == 0 || width > stride {
        return Err("invalid dimensions".into());
    }
    let frame_size = (stride as usize)
        .checked_mul(height as usize)
        .ok_or("frame size overflow")?;
    let bytes = fs::read(&args[4])?;
    if bytes.len() % frame_size != 0 || bytes.len() / frame_size < 2 {
        return Err("input must contain at least two complete gray8 frames".into());
    }
    let mut rows = Vec::new();
    let context = Arc::new(OpticalFlowContext::default());
    let mut previous: Option<OpticalFlowMethod> = None;
    for (i, frame) in bytes.chunks_exact(frame_size).enumerate() {
        let image =
            Arc::new(GrayImage::from_raw(stride, height, frame.to_vec()).ok_or("invalid image")?);
        let current = OpticalFlowMethod::detect_features_with_context(
            3,
            i as i64,
            image,
            width,
            height,
            context.clone(),
        );
        if let Some(old) = previous {
            let start = Instant::now();
            let matches = old.optical_flow_to(&current).ok_or_else(|| {
                context
                    .error()
                    .unwrap_or_else(|| "no valid correspondences".into())
            })?;
            let seconds = start.elapsed().as_secs_f64();
            let cached = old
                .optical_flow_to(&current)
                .ok_or("cache lost correspondences")?;
            assert_eq!(cached, matches);
            println!(
                "pair {}: {} matches, {:.3} s",
                i - 1,
                matches.0.len(),
                seconds
            );
            rows.push(serde_json::json!({"pair": i - 1, "seconds": seconds, "from": matches.0, "to": matches.1}));
        }
        previous = Some(current);
    }
    fs::write(&args[5], serde_json::to_vec_pretty(&rows)?)?;
    Ok(())
}

#[cfg(not(feature = "use-gmflow"))]
fn main() {
    eprintln!(
        "gmflow_pairs requires --features use-gmflow and a prepared model; see _scripts/gmflow/README.md"
    );
    std::process::exit(1);
}
