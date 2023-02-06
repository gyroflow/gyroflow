// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

use biquad::{Biquad, Coefficients, Type, DirectForm2Transposed, ToHertz};

use super::gyro_source::{ TimeIMU, TimeQuat };

pub struct Lowpass {
    filters: [DirectForm2Transposed<f64>; 6]
}

impl Lowpass {
    pub fn new(freq: f64, sample_rate: f64) -> Result<Self, biquad::Errors> {
        let coeffs = Coefficients::<f64>::from_params(Type::LowPass, sample_rate.hz(), freq.hz(), biquad::Q_BUTTERWORTH_F64)?;
        Ok(Self {
            filters: [
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
                DirectForm2Transposed::<f64>::new(coeffs),
            ]
        })
    }
    pub fn run(&mut self, i: usize, data: f64) -> f64 {
        self.filters[i].run(data)
    }

    pub fn filter_gyro(&mut self, data: &mut [TimeIMU]) {
        for x in data {
            if let Some(g) = x.gyro.as_mut() {
                g[0] = self.run(0, g[0]);
                g[1] = self.run(1, g[1]);
                g[2] = self.run(2, g[2]);
            }

            if let Some(a) = x.accl.as_mut() {
                a[0] = self.run(3, a[0]);
                a[1] = self.run(4, a[1]);
                a[2] = self.run(5, a[2]);
            }
        }
    }
    pub fn filter_gyro_forward_backward(freq: f64, sample_rate: f64, data: &mut [TimeIMU]) -> Result<(), biquad::Errors> {
        let mut forward = Self::new(freq, sample_rate)?;
        let mut backward = Self::new(freq, sample_rate)?;
        for x in data.iter_mut() {
            if let Some(g) = x.gyro.as_mut() {
                g[0] = forward.run(0, g[0]);
                g[1] = forward.run(1, g[1]);
                g[2] = forward.run(2, g[2]);
            }
            if let Some(a) = x.accl.as_mut() {
                a[0] = forward.run(3, a[0]);
                a[1] = forward.run(4, a[1]);
                a[2] = forward.run(5, a[2]);
            }
        }
        for x in data.iter_mut().rev() {
            if let Some(g) = x.gyro.as_mut() {
                g[0] = backward.run(0, g[0]);
                g[1] = backward.run(1, g[1]);
                g[2] = backward.run(2, g[2]);
            }
            if let Some(a) = x.accl.as_mut() {
                a[0] = backward.run(3, a[0]);
                a[1] = backward.run(4, a[1]);
                a[2] = backward.run(5, a[2]);
            }
        }
        Ok(())
    }
    pub fn filter_quats_forward_backward(freq: f64, sample_rate: f64, data: &mut TimeQuat) -> Result<(), biquad::Errors> {
        let mut forward = Self::new(freq, sample_rate)?;
        let mut backward = Self::new(freq, sample_rate)?;
        for (_ts, uq) in data.iter_mut() {
            let mut q = uq.quaternion().clone();
            q.coords[0] = forward.run(0, q.coords[0]);
            q.coords[1] = forward.run(1, q.coords[1]);
            q.coords[2] = forward.run(2, q.coords[2]);
            q.coords[3] = forward.run(3, q.coords[3]);
            *uq = crate::Quat64::from_quaternion(q);
        }
        for (_ts, uq) in data.iter_mut().rev() {
            let mut q = uq.quaternion().clone();
            q.coords[0] = backward.run(0, q.coords[0]);
            q.coords[1] = backward.run(1, q.coords[1]);
            q.coords[2] = backward.run(2, q.coords[2]);
            q.coords[3] = backward.run(3, q.coords[3]);
            *uq = crate::Quat64::from_quaternion(q);
        }
        Ok(())
    }
}
