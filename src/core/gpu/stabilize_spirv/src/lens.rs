// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2023 Adrian <adrian.eddy at gmail>

use glam::{ Vec2, vec2, Vec3 };
use super::DistortionModel;

use super::types::*;

#[inline(never)]
pub fn lens_undistort(point: Vec2, params: &KernelParams, distortion_model: u32) -> Vec2 {
    if params.k1.x == 0.0 && params.k1.y == 0.0 && params.k1.z == 0.0 && params.k1.w == 0.0 { return point; }

    let model: DistortionModel = unsafe { core::mem::transmute(distortion_model as i32) };
    model.undistort_point(point, params)
}
#[inline(never)]
pub fn lens_distort(point: Vec3, params: &KernelParams, distortion_model: u32) -> Vec2 {
    if params.k1.x == 0.0 && params.k1.y == 0.0 && params.k1.z == 0.0 && params.k1.w == 0.0 { return vec2(point.x / point.z, point.y / point.z); }

    let model: DistortionModel = unsafe { core::mem::transmute(distortion_model as i32) };
    model.distort_point(point, params)
}

#[inline(never)]
pub fn digital_lens_undistort(point: Vec2, params: &KernelParams, digital_distortion_model: u32) -> Vec2 {
    let model: DistortionModel = unsafe { core::mem::transmute(digital_distortion_model as i32) };
    model.undistort_point(point, params)
}
#[inline(never)]
pub fn digital_lens_distort(point: Vec3, params: &KernelParams, digital_distortion_model: u32) -> Vec2 {
    let model: DistortionModel = unsafe { core::mem::transmute(digital_distortion_model as i32) };
    model.distort_point(point, params)
}
