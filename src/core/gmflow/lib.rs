// SPDX-License-Identifier: GPL-3.0-or-later
#![recursion_limit = "512"]

//! The generated network lives in a separate crate so application changes do
//! not recompile its graph and embedded weights. Call it on a large-stack thread.

use burn::backend::wgpu::WgpuDevice;
use burn::tensor::{Bytes, Tensor, TensorData};
use burn_store::{BurnpackStore, ModuleSnapshot};

mod bool_storage;

#[allow(warnings)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/gmflow/gmflow.rs"));
}

pub type Gpu = burn::backend::Wgpu<f32, i32, u32>;
pub const WIDTH: usize = 576;
pub const HEIGHT: usize = 320;

pub struct Model(generated::Model<Gpu>);

impl Model {
    pub fn load(device: &WgpuDevice) -> Result<Self, String> {
        let mut model = generated::Model::new(device);
        let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/gmflow/gmflow.bpk"));
        let mut store = BurnpackStore::from_bytes(Some(Bytes::from_bytes_vec(bytes.to_vec())))
            .with_from_adapter(bool_storage::GpuBoolAdapter);
        model
            .load_from(&mut store)
            .map_err(|e| format!("Cannot load GMFlow weights: {e}"))?;
        Ok(Self(model))
    }

    /// Infer planar (dx, dy) flow from two CHW, three-channel, 0–255 images.
    pub fn flow(&self, a: Vec<f32>, b: Vec<f32>, device: &WgpuDevice) -> Result<Vec<f32>, String> {
        if a.len() != 3 * WIDTH * HEIGHT || b.len() != a.len() {
            return Err("GMFlow requires two 3×320×576 input tensors".into());
        }
        let a = Tensor::<Gpu, 4>::from_data(TensorData::new(a, [1, 3, HEIGHT, WIDTH]), device);
        let b = Tensor::<Gpu, 4>::from_data(TensorData::new(b, [1, 3, HEIGHT, WIDTH]), device);
        self.0
            .forward(a, b)
            .into_data()
            .to_vec::<f32>()
            .map_err(|e| e.to_string())
    }
}
