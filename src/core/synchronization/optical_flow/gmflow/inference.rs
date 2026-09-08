// SPDX-License-Identifier: GPL-3.0-or-later

use super::OpticalFlowContext;
use super::geometry::Layout;
use burn::backend::wgpu::{RuntimeOptions, WgpuDevice, graphics::AutoGraphicsApi, init_setup};
use burn::tensor::backend::Backend;
use gyroflow_gmflow::{Gpu, HEIGHT, Model, WIDTH};
use parking_lot::Mutex;
use std::sync::{
    Arc, OnceLock,
    mpsc::{self, SyncSender},
};
use std::time::Duration;

type FlowResult = Result<Option<Vec<f32>>, String>;
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

struct Request {
    a: Arc<image::GrayImage>,
    b: Arc<image::GrayImage>,
    size: (u32, u32),
    reply: SyncSender<FlowResult>,
    context: Arc<OpticalFlowContext>,
}

// One model and one in-flight inference across Rayon jobs. The bounded queue
// retains image Arcs, rather than allocating full tensors for every queued pair.
static WORKER: OnceLock<Result<SyncSender<Request>, String>> = OnceLock::new();

pub(super) fn flow(
    a: Arc<image::GrayImage>,
    b: Arc<image::GrayImage>,
    size: (u32, u32),
    context: Arc<OpticalFlowContext>,
) -> FlowResult {
    if context.is_cancelled() {
        return Ok(None);
    }
    let worker = WORKER
        .get_or_init(|| {
            let (tx, rx) = mpsc::sync_channel(1);
            std::thread::Builder::new()
                .name("gmflow".into())
                .stack_size(16 * 1024 * 1024)
                .spawn(move || run(rx))
                .map_err(|e| e.to_string())?;
            Ok(tx)
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let (reply, result) = mpsc::sync_channel(1);
    worker
        .send(Request {
            a,
            b,
            size,
            reply,
            context,
        })
        .map_err(|_| "GMFlow worker has stopped".to_string())?;
    result
        .recv()
        .map_err(|_| "GMFlow worker stopped during inference".to_string())?
}

// Model construction overflows the default Windows thread stack. Keep it on
// this dedicated large-stack worker, including all lazy parameter initialization.
#[inline(never)]
fn run(rx: mpsc::Receiver<Request>) {
    let device = WgpuDevice::default();
    let mut network = None;
    let device_error = Arc::new(Mutex::new(None));
    let mut initialized = false;
    let mut failure = None;
    loop {
        let request = match rx.recv_timeout(IDLE_TIMEOUT) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if network.is_some() {
                    if let Err(error) = release_model(&mut network, &device) {
                        log::error!("GMFlow resource cleanup failed: {error}");
                        failure.get_or_insert(error);
                    } else {
                        log::debug!("GMFlow released idle model memory");
                    }
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if request.context.is_cancelled() {
            let _ = request.reply.send(Ok(None));
            continue;
        }
        if let Some(error) = &failure {
            let _ = request.reply.send(Err(error.clone()));
            continue;
        }
        // Driver/backend panics are converted into a failure of this analysis.
        // In-flight GPU kernels cannot be interrupted; cancellation is checked
        // again before returning their output. Queued pairs never launch work.
        let result = gpu_call(|| {
            if !initialized {
                initialize_device(&device, device_error.clone());
                initialized = true;
            }
            if network.is_none() {
                network = Some(Model::load(&device)?);
                log::debug!("GMFlow model loaded");
            }
            let layout = Layout::new(request.size).ok_or("invalid frame dimensions")?;
            let a = layout.prepare(&request.a)?;
            let b = layout.prepare(&request.b)?;
            if request.context.is_cancelled() {
                return Ok(None);
            }
            let result = network.as_ref().expect("model initialized above").flow(a, b, &device)?;
            // Keep model parameters, but release completed workspace buffers.
            // Holding the allocator's high-water mark can starve video encoding
            // on systems where GPU allocations also consume host commit memory.
            Gpu::memory_cleanup(&device);
            if result.len() != 2 * WIDTH * HEIGHT || !result.iter().all(|x| x.is_finite()) {
                return Err("model returned invalid optical flow".into());
            }
            if request.context.is_cancelled() {
                Ok(None)
            } else {
                Ok(Some(result))
            }
        })
        .map_err(|error| {
            let detail = device_error.lock().clone().unwrap_or(error);
            format!("GMFlow could not run on this device: {detail}. Select another optical flow method. Restart Gyroflow before trying GMFlow again.")
        });
        if let Err(error) = &result {
            request.context.fail(error.clone());
            // A failed compute client cannot safely be reused. Retain the first
            // error instead of repeatedly submitting work to a lost device.
            failure = Some(error.clone());
            if initialized {
                let _ = release_model(&mut network, &device);
            }
        }
        let _ = request.reply.send(result);
    }
}

fn initialize_device(device: &WgpuDevice, error: Arc<Mutex<Option<String>>>) {
    let setup = init_setup::<AutoGraphicsApi>(device, RuntimeOptions::default());
    log::info!("GMFlow adapter: {:?}", setup.adapter.get_info());
    let lost = error.clone();
    setup
        .device
        .set_device_lost_callback(move |reason, message| {
            lost.lock()
                .get_or_insert_with(|| format!("GPU device lost ({reason:?}): {message}"));
        });
    setup.device.on_uncaptured_error(Arc::new(move |cause| {
        error.lock().get_or_insert_with(|| cause.to_string());
        // Preserve wgpu's fail-fast behavior. The worker reports the original
        // error to the application rather than the downstream CubeCL CallError.
        panic!("GMFlow GPU operation failed");
    }));
}

fn gpu_call<T>(operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).unwrap_or_else(|panic| {
        let detail = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("unknown GPU backend panic");
        Err(detail.to_string())
    })
}

fn release_model(network: &mut Option<Model>, device: &WgpuDevice) -> Result<(), String> {
    gpu_call(|| {
        drop(network.take());
        Gpu::memory_cleanup(device);
        Ok(())
    })
}
