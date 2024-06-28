// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

#![deny(warnings)]
#![no_std]

// TODO: compute stage with buffer

mod types;       pub use types::*;
mod drawing;     pub use drawing::*;
mod stabilize;   pub use stabilize::*;
mod lens;        pub use lens::*;
mod background;  pub use background::*;
mod interpolate; pub use interpolate::*;
mod distortion_models; pub use distortion_models::*;

pub use spirv_std::glam;
use glam::{ vec2, vec4, Vec4 };
use spirv_std::spirv;

#[cfg(feature = "for_qtrhi")]
#[spirv(fragment)]
pub fn undistort_fragment(
    #[spirv(frag_coord)] in_frag_coord: Vec4,
    #[spirv(descriptor_set = 0, binding = 1)] input_texture: &Image!(2D, type=f32, sampled),
    #[spirv(uniform, descriptor_set = 0, binding = 2)] params: &KernelParams,
    #[spirv(descriptor_set = 0, binding = 3)] matrices: &Image!(2D, type=f32, sampled),
    #[spirv(descriptor_set = 0, binding = 4)] drawing: &Image!(2D, type=f32, sampled),
    #[spirv(descriptor_set = 0, binding = 5)] sampler: &spirv_std::Sampler,
    #[spirv(spec_constant(id = 100, default = 8))] interpolation: u32,
    #[spirv(spec_constant(id = 101, default = 1))] distortion_model: u32,
    #[spirv(spec_constant(id = 102))] digital_distortion_model: u32,
    #[spirv(spec_constant(id = 103))] flags: u32,
    output: &mut ScalarVec4,
) {
    *output = undistort(vec2(in_frag_coord.x, in_frag_coord.y), params, matrices, &[], &[], drawing, input_texture, sampler, interpolation, distortion_model, digital_distortion_model, flags);
}

#[cfg(not(feature = "for_qtrhi"))]
#[spirv(fragment)]
pub fn undistort_fragment(
    #[spirv(frag_coord)] in_frag_coord: Vec4,
    #[spirv(uniform, descriptor_set = 0, binding = 0)] params: &KernelParams,
    #[spirv(storage_buffer, descriptor_set = 0, binding = 1)] matrices: &[f32],
    #[spirv(storage_buffer, descriptor_set = 0, binding = 2)] coeffs: &[f32],
    #[spirv(storage_buffer, descriptor_set = 0, binding = 3)] mesh_data: &[f32],
    #[spirv(storage_buffer, descriptor_set = 0, binding = 4)] drawing: &[u32],
    #[spirv(descriptor_set = 0, binding = 5)] input_texture: &ImageType,
    #[spirv(spec_constant(id = 100, default = 8))] interpolation: u32,
    #[spirv(spec_constant(id = 101, default = 1))] distortion_model: u32,
    #[spirv(spec_constant(id = 102))] digital_distortion_model: u32,
    #[spirv(spec_constant(id = 103))] flags: u32,
    output: &mut ScalarVec4,
) {
    *output = from_float(undistort(vec2(in_frag_coord.x, in_frag_coord.y), params, matrices, coeffs, mesh_data, drawing, input_texture, 0.0, interpolation, distortion_model, digital_distortion_model, flags));
}

#[spirv(vertex)]
pub fn undistort_vertex(#[spirv(vertex_index)] vert_id: usize, #[spirv(position, invariant)] out_pos: &mut Vec4) {
    const POSITIONS: [Vec4; 6] = [
        vec4(-1.0, -1.0, 0.0, 1.0), vec4( 1.0, -1.0, 0.0, 1.0), vec4( 1.0,  1.0, 0.0, 1.0),
        vec4( 1.0,  1.0, 0.0, 1.0), vec4(-1.0,  1.0, 0.0, 1.0), vec4(-1.0, -1.0, 0.0, 1.0),
    ];
    *out_pos = POSITIONS[vert_id];
}
