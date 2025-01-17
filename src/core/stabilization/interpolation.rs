// This file is unused.
// It can be used to generate the interpolation coefficients for the GPU shaders

#[derive(Default, Debug, Clone, Copy)]
pub enum InterpolationType {
    #[default]
    Linear   = 2,
    Bicubic  = 4,
    Lanczos4 = 8
}

fn interpolate_linear(x: f64, coeffs: &mut [f64]) {
    coeffs[0] = 1.0 - x;
    coeffs[1] = x;
}

fn interpolate_cubic(x: f64, coeffs: &mut [f64]) {
    const A: f64 = -0.75;

    coeffs[0] = ((A * (x + 1.0) - 5.0 * A) * (x + 1.0) + 8.0 * A) * (x + 1.0) - 4.0 * A;
    coeffs[1] = ((A + 2.0) * x - (A + 3.0)) * x * x + 1.0;
    coeffs[2] = ((A + 2.0) * (1.0 - x) - (A + 3.0)) * (1.0 - x) * (1.0 - x) + 1.0;
    coeffs[3] = 1.0 - coeffs[0] - coeffs[1] - coeffs[2];
}

fn interpolate_lanczos4(x: f64, coeffs: &mut [f64]) {
    const FLT_EPSILON: f64 = 1.19209290E-07;
    const S45: f64 = 0.70710678118654752440084436210485;
    const CS: [[f64; 2]; 8] = [[1.0, 0.0], [-S45, -S45], [0.0, 1.0], [S45, -S45], [-1.0, 0.0], [S45, S45], [0.0, -1.0], [-S45, S45]];
    use std::f64::consts::FRAC_PI_4;

    if x < FLT_EPSILON {
        for i in 0..8 {
            coeffs[i] = 0.0;
        }
        coeffs[3] = 1.0;
        return;
    }

    let mut sum = 0.0;
    let y0 = -(x + 3.0) * FRAC_PI_4;
    let s0 = y0.sin();
    let c0 = y0.cos();
    for i in 0..8 {
        let y = -(x + 3.0 - i as f64) * FRAC_PI_4;
        coeffs[i] = (CS[i][0] * s0 + CS[i][1] * c0) / (y * y);
        sum += coeffs[i];
    }

    sum = 1.0 / sum;
    for i in 0..8 {
        coeffs[i] *= sum;
    }
}

pub fn get_interpolation_table(typ: InterpolationType) -> Vec<f64> {
    const TAB_SIZE: usize = 1 << 5;
    const SCALE: f64 = 1.0 / TAB_SIZE as f64;

    let num_coeffs = typ as usize;
    let mut tab: Vec<f64> = vec![0.0; TAB_SIZE * num_coeffs];
    for i in 0..TAB_SIZE {
        match typ {
            InterpolationType::Linear   => interpolate_linear  (i as f64 * SCALE, &mut tab[i * num_coeffs..i * num_coeffs + num_coeffs]),
            InterpolationType::Bicubic  => interpolate_cubic   (i as f64 * SCALE, &mut tab[i * num_coeffs..i * num_coeffs + num_coeffs]),
            InterpolationType::Lanczos4 => interpolate_lanczos4(i as f64 * SCALE, &mut tab[i * num_coeffs..i * num_coeffs + num_coeffs]),
        }
    }
    tab
}
