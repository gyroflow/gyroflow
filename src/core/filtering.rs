use biquad::{Biquad, Coefficients, Type, DirectForm2Transposed, ToHertz};

use super::gyro_source::TimeIMU;

pub struct Lowpass {
    filters: [DirectForm2Transposed<f64>; 4]
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
            ]
        })
    }
    pub fn run(&mut self, i: usize, data: f64) -> f64 {
        self.filters[i].run(data)
    }

    pub fn filter_gyro(&mut self, data: &mut [TimeIMU]) {
        for mut x in data {
            x.gyro[0] = self.run(0, x.gyro[0]);
            x.gyro[1] = self.run(1, x.gyro[1]);
            x.gyro[2] = self.run(2, x.gyro[2]);
        }
    }
    pub fn filter_gyro_forward_backward(freq: f64, sample_rate: f64, data: &mut [TimeIMU]) -> Result<(), biquad::Errors> {
        let mut forward = Self::new(freq, sample_rate)?;
        let mut backward = Self::new(freq, sample_rate)?;
        for mut x in data.iter_mut() {
            x.gyro[0] = forward.run(0, x.gyro[0]);
            x.gyro[1] = forward.run(1, x.gyro[1]);
            x.gyro[2] = forward.run(2, x.gyro[2]);
        }
        for mut x in data.iter_mut().rev() {
            x.gyro[0] = backward.run(0, x.gyro[0]);
            x.gyro[1] = backward.run(1, x.gyro[1]);
            x.gyro[2] = backward.run(2, x.gyro[2]);
        }
        Ok(())
    }
}
