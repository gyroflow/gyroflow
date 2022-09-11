// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2022 Maik Menz

#![allow(non_snake_case)]

use rustfft::algorithm::Radix4;
use nalgebra::ComplexField;
use rustfft::{num_complex::Complex, Fft, FftDirection};
use std::{f32::consts::PI};

use qmetaobject::*;
use crate::util;

#[derive(Default)]
struct Series {
    line: Vec<QPointF>,
    data: Vec<f64>,
    spectrum: Vec<f64>,
    window: Vec<f32>,
}


#[derive(Default, QObject)]
pub struct FrequencyGraph {
    base: qt_base_class!(trait QQuickPaintedItem),
    
    color: qt_property!(QColor; WRITE setColor  ),
    lineWidth: qt_property!(f64; WRITE setLineWidth),
    logY: qt_property!(bool; WRITE setLogY),
    min: qt_property!(f64; WRITE setMin),
    max: qt_property!(f64; WRITE setMax),

    samplerate: qt_property!(f64; NOTIFY samplerate_changed),
    samplerate_changed: qt_signal!(),

    series: Series,
}

impl FrequencyGraph {
    fn setColor    (&mut self, v: QColor) { self.color = v;     self.update(); }
    fn setLineWidth(&mut self, v: f64)    { self.lineWidth = v; self.update(); }
    fn setLogY     (&mut self, v: bool)   { self.logY = v;      self.update(); }
    fn setMin      (&mut self, v: f64)    { self.min = v;       self.update(); }
    fn setMax      (&mut self, v: f64)    { self.max = v;       self.update(); }

    pub fn update(&mut self) {
        self.calculate_lines();
        util::qt_queued_callback(self, |this, _| {
            (this as &dyn QQuickItem).update();
        })(());
    }
    
    pub fn setData(&mut self, vec: &[f64], sr: f64) {
        self.series.data = vec.to_vec();
        if self.samplerate != sr {
            self.samplerate = sr;
            self.samplerate_changed();
        }
        self.analyze_spectrum();
        self.update();
    }

    fn analyze_spectrum(&mut self) {
        self.series.spectrum.clear();
        
        let fft_size = self.series.data.len();
        if fft_size == 0 { return; }

        if self.series.window.len() != fft_size {
            // using blackmann-harris
            self.series.window = {
                let mut samples = vec![0.0; fft_size];
                let size = (fft_size - 1) as f32;
                for i in 0..fft_size {
                    let r = i as f32 / size;
                    samples[i] = 0.35875 -
						0.48829 * (2.0 * PI * r).cos() +
						0.14128 * (4.0 * PI * r).cos() -
						0.01168 * (6.0 * PI * r).cos();
                }
                samples
            }
        }

        let mut samples = self.series.data
            .iter()
            .map(|x| Complex::from_real(*x as f32))
            .zip(&self.series.window)
            .map(|(x,y)| x * y)
            .collect::<Vec<Complex<f32>>>();

        let fft = Radix4::new(fft_size, FftDirection::Forward);
        fft.process(&mut samples);

        let scale = 1.0 / (fft_size as f64);

        self.series.spectrum = samples
            .iter()
            .take(fft_size / 2)
            .map(|&complex| complex.norm() as f64 * scale)
            .collect();
    }

    fn calculate_lines(&mut self) {
        self.series.line.clear();
        
        if self.series.spectrum.is_empty() { return; }

        let rect = (self as &dyn QQuickItem).bounding_rect();
        if rect.width <= 0.0 || rect.height <= 0.0 { return; }

        let nrPoints = self.series.spectrum.len();
        self.series.line.reserve(nrPoints);

        let scaler = if self.logY { |x: f64| x.log10() } else { |x: f64| x };
        let rangeMax = scaler(self.max);
        let rangeMin = scaler(self.min);

        let dx = rect.width / (nrPoints-1) as f64;
        let mut x: f64 = 0.0;
        for v in &self.series.spectrum {
            let db = scaler(*v).clamp(rangeMin, rangeMax);
            let r = (db - rangeMin) / (rangeMax - rangeMin);

            self.series.line.push(QPointF{ x, y: rect.height * (1.0 - r)});
            x += dx;
        }
    }
}
impl QQuickItem for FrequencyGraph {
    fn class_begin(&mut self) {
        self.color     = QColor::from_name("white");
        self.lineWidth = 1.0;
        self.logY      = true;
        self.min       = 0.000001;
        self.max       = 1.0;
    }

    fn geometry_changed(&mut self, _new: QRectF, _old: QRectF) {
        self.calculate_lines();
        (self as &dyn QQuickItem).update();
    }
}
impl QQuickPaintedItem for FrequencyGraph {
    fn paint(&mut self, p: &mut QPainter) {
        if !self.series.line.is_empty() {
            let mut pen = QPen::from_color(self.color);
            pen.set_width_f(self.lineWidth); 

            p.set_pen(pen);
            p.set_render_hint(QPainterRenderHint::Antialiasing, true);
            p.draw_polyline(self.series.line.as_slice());
        }
    }
}
