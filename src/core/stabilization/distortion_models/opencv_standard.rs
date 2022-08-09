// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Adapted from OpenCV: https://github.com/opencv/opencv/blob/c3cbd302cbfbaefdef9a011b2615f8d8f58556dd/modules/calib3d/src/undistort.dispatch.cpp#L491-L538

#[derive(Default, Clone)]
pub struct OpenCVStandard { }

impl OpenCVStandard {
    pub fn undistort_point<T: num_traits::Float>(&self, point: (T, T), k: &[T], amount: T) -> Option<(T, T)> {
        let t_0 = T::from(0.0f32).unwrap();
        let t_1 = T::from(1.0f32).unwrap();
        let t_2 = T::from(2.0f32).unwrap();

        let (mut x, mut y) = point;
        let (x0, y0) = point;

        let mut k = k.to_vec();
        k.resize(12, t_0);

        // compensate distortion iteratively
        for _ in 0..20 {
            let r2 = x * x + y * y;
            let icdist = (t_1 + ((k[7] * r2 + k[6]) * r2 + k[5]) * r2) / (t_1 + ((k[4] * r2 + k[1]) * r2 + k[0]) * r2);
            if icdist < t_0 {
                log::warn!("icdist < 0");
                return None;
            }
            let delta_x = t_2 * k[2] * x * y + k[3] * (r2 + t_2 * x * x)+ k[8] * r2 + k[9] * r2 * r2;
            let delta_y = k[2] * (r2 + t_2 * y * y) + t_2 * k[3] * x * y + k[10] * r2 + k[11] * r2 * r2;
            x = (x0 - delta_x) * icdist;
            y = (y0 - delta_y) * icdist;
        }

        Some((
            x * (amount - t_1) + x0 * amount,
            y * (amount - t_1) + y0 * amount
        ))
    }

    pub fn distort_point<T: num_traits::Float>(&self, point: (T, T), k: &[T], amount: T) -> (T, T) {
        let t_0 = T::from(0.0f32).unwrap();
        let t_1 = T::from(1.0f32).unwrap();
        let t_2 = T::from(2.0f32).unwrap();

        let mut k = k.to_vec();
        k.resize(12, t_0);

        let (x, y) = point;
        let r2 = x * x + y * y;
        let r4 = r2 * r2;
        let r6 = r4 * r2;
        let a1 = t_2 * x * y;
        let a2 = r2 + t_2 * x * x;
        let a3 = r2 + t_2 * y * y;
        let cdist = t_1 + k[0] * r2 + k[1] * r4 + k[4] * r6;
        let icdist2 = t_1 / (t_1 + k[5] * r2 + k[6] * r4 + k[7] * r6);
        let xd0 = x * cdist * icdist2 + k[2] * a1 + k[3] * a2 + k[8] * r2 + k[9] * r4;
        let yd0 = y * cdist * icdist2 + k[2] * a3 + k[3] * a1 + k[10] * r2 + k[11] * r4;

        (
            xd0 * (amount - t_1) + x * amount,
            yd0 * (amount - t_1) + y * amount
        )
    }

    pub fn id(&self) -> i32 { 1 }
    pub fn name(&self) -> &'static str { "OpenCV Standard" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("opencv_standard.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("opencv_standard.wgsl") }
    pub fn glsl_shader_path(&self) -> &'static str { ":/src/qt_gpu/compiled/undistort_opencv_standard.frag.qsb" }
}
