// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Adrian <adrian.eddy at gmail>

// Adapted from OpenCV: https://github.com/opencv/opencv/blob/2b60166e5c65f1caccac11964ad760d847c536e4/modules/calib3d/src/fisheye.cpp#L257-L460

#[derive(Default, Clone)]
pub struct OpenCVFisheye { }

impl OpenCVFisheye {
    pub fn undistort_point<T: num_traits::Float>(&self, point: (T, T), k: &[T], amount: T) -> Option<(T, T)> {
        let t_0 = T::from(0.0f32).unwrap();
        let t_1 = T::from(1.0f32).unwrap();
        let t_3 = T::from(3.0f32).unwrap();
        let t_5 = T::from(5.0f32).unwrap();
        let t_7 = T::from(7.0f32).unwrap();
        let t_9 = T::from(9.0f32).unwrap();
        let t_fpi = T::from(std::f64::consts::PI).unwrap();
        let t_eps = T::from(1e-6f64).unwrap();
        
        let t_max_fix = T::from(0.9f32).unwrap();
    
        let mut theta_d = (point.0 * point.0 + point.1 * point.1).sqrt();
    
        // the current camera model is only valid up to 180 FOV
        // for larger FOV the loop below does not converge
        // clip values so we still get plausible results for super fisheye images > 180 grad
        theta_d = theta_d.max(-t_fpi).min(t_fpi);
    
        let mut converged = false;
        let mut theta = theta_d;
    
        let mut scale = t_0;
    
        if theta_d.abs() > t_eps {
            theta = t_0;
    
            // compensate distortion iteratively
            for _ in 0..10 {
                let theta2 = theta*theta;
                let theta4 = theta2*theta2;
                let theta6 = theta4*theta2;
                let theta8 = theta6*theta2;
                let k0_theta2 = k[0] * theta2;
                let k1_theta4 = k[1] * theta4;
                let k2_theta6 = k[2] * theta6;
                let k3_theta8 = k[3] * theta8;
                // new_theta = theta - theta_fix, theta_fix = f0(theta) / f0'(theta)
                let mut theta_fix = (theta * (t_1 + k0_theta2 + k1_theta4 + k2_theta6 + k3_theta8) - theta_d)
                                /
                                (t_1 + t_3 * k0_theta2 + t_5 * k1_theta4 + t_7 * k2_theta6 + t_9 * k3_theta8);
                
                theta_fix = theta_fix.max(-t_max_fix).min(t_max_fix);
    
                theta = theta - theta_fix;
                if theta_fix.abs() < t_eps {
                    converged = true;
                    break;
                }
            }
    
            scale = theta.tan() / theta_d;
        } else {
            converged = true;
        }
    
        // theta is monotonously increasing or decreasing depending on the sign of theta
        // if theta has flipped, it might converge due to symmetry but on the opposite of the camera center
        // so we can check whether theta has changed the sign during the optimization
        let theta_flipped = (theta_d < t_0 && theta > t_0) || (theta_d > t_0 && theta < t_0);
    
        if converged && !theta_flipped {
            // Apply only requested amount
            scale = t_1 + (scale - t_1) * (t_1 - amount);
    
            return Some((point.0 * scale, point.1 * scale));
        }
        None
    }
    
    pub fn distort_point<T: num_traits::Float>(&self, point: (T, T), k: &[T], amount: T) -> (T, T) {
        let t_0 = T::from(0.0f32).unwrap();
        let t_1 = T::from(1.0f32).unwrap();
    
        let r = (point.0 * point.0 + point.1 * point.1).sqrt();
    
        let theta = r.atan();
        let theta2 = theta*theta;
        let theta4 = theta2*theta2;
        let theta6 = theta4*theta2;
        let theta8 = theta4*theta4;
    
        let theta_d = theta * (t_1 + k[0]*theta2 + k[1]*theta4 + k[2]*theta6 + k[3]*theta8);
    
        let mut scale = if r == t_0 { t_1 } else { theta_d / r };
        scale = t_1 + (scale - t_1) * (t_1 - amount);
    
        (
            point.0 * scale,
            point.1 * scale
        )
    }

    pub fn id(&self) -> i32 { 0 }
    pub fn name(&self) -> &'static str { "OpenCV Fisheye" }

    pub fn opencl_functions(&self) -> &'static str { include_str!("opencv_fisheye.cl") }
    pub fn wgsl_functions(&self)   -> &'static str { include_str!("opencv_fisheye.wgsl") }
    pub fn glsl_shader_path(&self) -> &'static str { ":/src/qt_gpu/compiled/undistort_opencv_fisheye.frag.qsb" }
}
