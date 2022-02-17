// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#![allow(non_snake_case)]

use std::collections::BTreeMap;

use qmetaobject::*;
use crate::core::gyro_source::{ GyroSource, TimeIMU, TimeQuat };
use crate::util;

#[derive(Default, Debug)]
pub struct ChartData {
    pub timestamp_us: i64,
    pub values: [f64; 4]
}

#[derive(Default)]
struct Series {
    data: BTreeMap<i64, f64>, // timestamp, value
    lines: Vec<Vec<QLineF>>,
    is_optflow: bool,
    visible: bool,
}

// We can have:
// viewMode 0: Gyro only
// viewMode 0: Gyro + sync results
// viewMode 1: Accel only
// viewMode 2: Magn only
// viewMode 3: Quaternions
// viewMode 3: Quaternions + smoothed quaternions

#[derive(Default, QObject)]
pub struct TimelineGyroChart {
    base: qt_base_class!(trait QQuickPaintedItem),

    visibleAreaLeft: qt_property!(f64; WRITE setVisibleAreaLeft),
    visibleAreaRight: qt_property!(f64; WRITE setVisibleAreaRight),
    vscale: qt_property!(f64; WRITE setVScale),

    setDurationMs: qt_method!(fn(&mut self, v: f64)),
    setVScaleToVisibleArea: qt_method!(fn(&mut self)),
    setAxisVisible: qt_method!(fn(&mut self, a: usize, v: bool)),
    getAxisVisible: qt_method!(fn(&self, a: usize) -> bool),
    axisVisibleChanged: qt_signal!(),
    viewModeChanged: qt_signal!(),

    viewMode: qt_property!(u32; WRITE setViewMode NOTIFY viewModeChanged),

    series: [Series; 4+4],

    gyro: Vec<ChartData>,
    accl: Vec<ChartData>,
    magn: Vec<ChartData>,
    quats: Vec<ChartData>,
    smoothed_quats: Vec<ChartData>,
    sync_results: Vec<ChartData>,
    sync_quats: Vec<ChartData>,

    gyro_max: Option<f64>,
    duration_ms: f64,
}

impl TimelineGyroChart {
    pub fn setDurationMs(&mut self, v: f64) { self.duration_ms = v; }
    fn setVisibleAreaLeft (&mut self, v: f64) { self.visibleAreaLeft = v; self.update(); }
    fn setVisibleAreaRight(&mut self, v: f64) { self.visibleAreaRight = v; self.update(); }
    fn setAxisVisible     (&mut self, a: usize, v: bool) { self.series[a].visible = v; self.update(); self.axisVisibleChanged(); }
    fn getAxisVisible     (&self, a: usize) -> bool { self.series[a].visible }
    fn setVScale          (&mut self, v: f64) { self.vscale = v.max(0.1); self.update(); }
    fn setViewMode        (&mut self, v: u32) { self.viewMode = v; self.update_data(); self.viewModeChanged(); }

    pub fn setVScaleToVisibleArea(&mut self) {
        let rect = (self as &dyn QQuickItem).bounding_rect();
        let mut min_height = f64::MAX;
        let mut max_height = 0.0;
        for serie in &mut self.series {
            if serie.visible && !serie.lines.is_empty() {
                for a in &serie.lines {
                    for b in a {
                        if b.pt1.x > 0.0 && b.pt1.x < rect.width &&
                           b.pt2.x > 0.0 && b.pt2.x < rect.width {
                            if b.pt1.y < min_height { min_height = b.pt1.y; }
                            if b.pt2.y < min_height { min_height = b.pt2.y; }
                            if b.pt1.y > max_height { max_height = b.pt1.y; }
                            if b.pt2.y > max_height { max_height = b.pt2.y; }
                        }
                    }
                }
            }
        }

        let min_element = (-(min_height / (rect.height / 2.0)) + 1.0) / self.vscale;
        let max_element = (-(max_height / (rect.height / 2.0)) + 1.0) / self.vscale;

        self.vscale = 0.9 / min_element.abs().max(max_element.abs());

        self.update();
    }

    pub fn update(&mut self) {
        self.calculate_lines();
        util::qt_queued_callback(self, |this, _| {
            (this as &dyn QQuickItem).update();
        })(());
    }
    fn calculate_lines(&mut self) {
        let rect = (self as &dyn QQuickItem).bounding_rect();
        let half_height = rect.height / 2.0;
        if rect.width <= 0.0 || rect.height <= 0.0 { return; }

        let map_to_visible_area = |v: f64| -> f64 { (v - self.visibleAreaLeft) / (self.visibleAreaRight - self.visibleAreaLeft) };

        let duration_us = self.duration_ms * 1000.0;

        for serie in &mut self.series {
            if serie.visible && !serie.data.is_empty() {
                let from_timestamp = ((self.visibleAreaLeft - 0.01) * duration_us).floor() as i64;
                let mut to_timestamp = ((self.visibleAreaRight + 0.01) * duration_us).ceil() as i64;
                if from_timestamp >= to_timestamp { to_timestamp = from_timestamp + 1; }

                let resolution = rect.width * 10.0;
                let mut range = serie.data.range(from_timestamp..=to_timestamp);
                let num_samples = range.clone().count();

                serie.lines.clear();
                if num_samples > 1 {
                    if let Some(first_item) = range.next() {
                        let mut line = Vec::new();
                        let mut prev_point = (*first_item.0, QPointF {
                            x: map_to_visible_area(*first_item.0 as f64 / duration_us) * rect.width,
                            y: (1.0 - *first_item.1 * self.vscale) * half_height
                        });
                        let step = (num_samples / resolution as usize).max(1);
                        for data in range.step_by(step) {
                            let point = QPointF {
                                x: map_to_visible_area(*data.0 as f64 / duration_us) * rect.width,
                                y: (1.0 - *data.1 * self.vscale) * half_height
                            };
                            
                            let new_line = serie.is_optflow && *data.0 - prev_point.0 > 100_000;
                            if new_line {
                                serie.lines.push(line);
                                line = Vec::new();
                            } else {
                                line.push(QLineF { pt1: prev_point.1, pt2: point });
                            }
                            prev_point = (*data.0, point);
                        }
                        serie.lines.push(line);
                    }
                }
            } else {
                serie.lines.clear();
            }
        }
    }

    fn drawAxis(&mut self, p: &mut QPainter, a: usize, color: &str) {
        let mut pen = QPen::from_color(QColor::from_name(color));
        pen.set_width_f(1.5); // TODO * dpiScale
        p.set_pen(pen);

        for l in &self.series[a].lines {
            if !l.is_empty() {
                p.draw_lines(l.as_slice());
            }
        }
    }

    pub fn setSyncResults(&mut self, data: &[TimeIMU]) {
        if self.viewMode == 0 {
            self.sync_results = Vec::with_capacity(data.len());

            for x in data {
                if let Some(g) = x.gyro.as_ref() {
                    self.sync_results.push(ChartData {
                        timestamp_us: (x.timestamp_ms * 1000.0) as i64,
                        values: [g[0], g[1], g[2], 0.0]
                    });
                }
            }
            Self::normalize_height(&mut self.sync_results, self.gyro_max);

            self.update_data();
        }
    }

    pub fn setSyncResultsQuats(&mut self, data: &TimeQuat) {
        if self.viewMode == 3 {
            self.sync_quats = Vec::with_capacity(data.len());

            for (ts, q) in data {
                let q = q.quaternion().as_vector();
                self.sync_quats.push(ChartData {
                    timestamp_us: *ts,
                    values: [q[0], q[1], q[2], q[3]]
                });
            }
            Self::normalize_height(&mut self.sync_quats, None);
            self.update_data();
        }
    }

    pub fn setFromGyroSource(&mut self, gyro: &GyroSource) {
        self.gyro = Vec::with_capacity(gyro.raw_imu.len());
        self.accl = Vec::with_capacity(gyro.raw_imu.len());
        self.magn = Vec::with_capacity(gyro.raw_imu.len());
        self.quats = Vec::with_capacity(gyro.quaternions.len());
        self.smoothed_quats = Vec::with_capacity(gyro.smoothed_quaternions.len());

        for x in &gyro.raw_imu {
            if self.viewMode == 0 {
                if let Some(g) = x.gyro.as_ref() {
                    self.gyro.push(ChartData {
                        timestamp_us: ((x.timestamp_ms + gyro.offset_at_timestamp(x.timestamp_ms)) * 1000.0) as i64,
                        values: [g[0], g[1], g[2], 0.0]
                    });
                }
            }
            if self.viewMode == 1 {
                if let Some(a) = x.accl.as_ref() {
                    self.accl.push(ChartData {
                        timestamp_us: ((x.timestamp_ms + gyro.offset_at_timestamp(x.timestamp_ms)) * 1000.0) as i64,
                        values: [a[0], a[1], a[2], 0.0]
                    });
                }
            }
            if self.viewMode == 2 {
                if let Some(m) = x.magn.as_ref() {
                    self.magn.push(ChartData {
                        timestamp_us: ((x.timestamp_ms + gyro.offset_at_timestamp(x.timestamp_ms)) * 1000.0) as i64,
                        values: [m[0], m[1], m[2], 0.0]
                    });
                }
            }
        }

        if self.viewMode == 3 {
            let add_quats = |quats: &TimeQuat, out_quats: &mut Vec<ChartData>| {
                for x in quats {
                    let mut ts = *x.0 as f64 / 1000.0;
                    ts += gyro.offset_at_timestamp(ts);

                    let q = x.1.quaternion().as_vector();

                    out_quats.push(ChartData {
                        timestamp_us: (ts * 1000.0) as i64,
                        values: [q[0], q[1], q[2], q[3]]
                    });
                }
            };
            add_quats(&gyro.quaternions, &mut self.quats);
            add_quats(&gyro.org_smoothed_quaternions, &mut self.smoothed_quats);
        }

        match self.viewMode {
            0 => { self.gyro_max = Self::normalize_height(&mut self.gyro, None); },
            1 => { Self::normalize_height(&mut self.accl, None); },
            2 => { Self::normalize_height(&mut self.magn, None); },
            3 => {
                let qmax = Self::normalize_height(&mut self.quats, None);
                Self::normalize_height(&mut self.smoothed_quats, qmax);
            },
            _ => { }
        }
        self.update_data();
    }
    fn get_serie_vector(vec: &[ChartData], i: usize) -> BTreeMap<i64, f64> {
        let mut ret = BTreeMap::new();
        for x in vec {
            ret.insert(x.timestamp_us, x.values[i]);
        }
        ret
    }
    pub fn update_data(&mut self) {
        for s in &mut self.series {
            s.data.clear();
        }
        match self.viewMode {
            0 => {  // Gyroscope
                self.series[0].data = Self::get_serie_vector(&self.gyro, 0);
                self.series[1].data = Self::get_serie_vector(&self.gyro, 1);
                self.series[2].data = Self::get_serie_vector(&self.gyro, 2);

                // + Sync results
                self.series[4].data = Self::get_serie_vector(&self.sync_results, 0);
                self.series[5].data = Self::get_serie_vector(&self.sync_results, 1);
                self.series[6].data = Self::get_serie_vector(&self.sync_results, 2);
                self.series[4].is_optflow = true;
                self.series[5].is_optflow = true;
                self.series[6].is_optflow = true;
            }
            1 => { // Accelerometer
                self.series[0].data = Self::get_serie_vector(&self.accl, 0);
                self.series[1].data = Self::get_serie_vector(&self.accl, 1);
                self.series[2].data = Self::get_serie_vector(&self.accl, 2);
            }
            2 => { // Magnetometer
                self.series[0].data = Self::get_serie_vector(&self.magn, 0);
                self.series[1].data = Self::get_serie_vector(&self.magn, 1);
                self.series[2].data = Self::get_serie_vector(&self.magn, 2);
            }
            3 => { // Quaternions
                self.series[0].data = Self::get_serie_vector(&self.quats, 0);
                self.series[1].data = Self::get_serie_vector(&self.quats, 1);
                self.series[2].data = Self::get_serie_vector(&self.quats, 2);
                self.series[3].data = Self::get_serie_vector(&self.quats, 3);

                // + Sync quaternions
                // self.series[4].data = Self::get_serie_vector(&self.sync_quats, 0);
                // self.series[5].data = Self::get_serie_vector(&self.sync_quats, 1);
                // self.series[6].data = Self::get_serie_vector(&self.sync_quats, 2);
                // self.series[7].data = Self::get_serie_vector(&self.sync_quats, 3);

                // + Smoothed quaternions
                self.series[4].data = Self::get_serie_vector(&self.smoothed_quats, 0);
                self.series[5].data = Self::get_serie_vector(&self.smoothed_quats, 1);
                self.series[6].data = Self::get_serie_vector(&self.smoothed_quats, 2);
                self.series[7].data = Self::get_serie_vector(&self.smoothed_quats, 3);
            }
            _ => panic!("Invalid view mode")
        }

        self.update();
    }

    fn normalize_height(data: &mut Vec<ChartData>, max: Option<f64>) -> Option<f64> {
        let max = max.unwrap_or_else(|| {
            let mut max = 0.0;
            for x in data.iter() {
                for i in 0..4 {
                    if x.values[i].abs() > max { max = x.values[i].abs(); }
                }
            }
            max
        });
        if max > 0.0 {
            for x in data.iter_mut() {
                for i in 0..4 {
                    x.values[i] /= max;
                }
            }
        }
        if max > 0.0 { Some(max) } else { None }
    }
}

impl QQuickItem for TimelineGyroChart {
    fn class_begin(&mut self) {
        self.duration_ms = 1.0;
        self.visibleAreaLeft = 0.0;
        self.visibleAreaRight = 1.0;
        self.vscale = 1.0;
        self.series[0].visible = true;
        self.series[1].visible = true;
        self.series[2].visible = true;

        self.series[4].visible = true;
        self.series[5].visible = true;
        self.series[6].visible = true;
    }

    fn geometry_changed(&mut self, _new: QRectF, _old: QRectF) {
        self.calculate_lines();
        (self as &dyn QQuickItem).update();
    }
}
impl QQuickPaintedItem for TimelineGyroChart {
    fn paint(&mut self, p: &mut QPainter) {
        p.set_render_hint(QPainterRenderHint::Antialiasing, true);

        if self.series[0].visible { self.drawAxis(p, 0, "#8f4c4c"); } // X
        if self.series[1].visible { self.drawAxis(p, 1, "#4c8f4d"); } // Y
        if self.series[2].visible { self.drawAxis(p, 2, "#4c7c8f"); } // Z
        if self.series[3].visible { self.drawAxis(p, 3, "#8f4c8f"); } // Angle

        if self.series[4].visible { self.drawAxis(p, 4, "#ff8888"); } // Sync X
        if self.series[5].visible { self.drawAxis(p, 5, "#88ff88"); } // Sync Y
        if self.series[6].visible { self.drawAxis(p, 6, "#88deff"); } // Sync Z
        if self.series[7].visible { self.drawAxis(p, 7, "#ff88ff"); } // Sync Angle

    }
}
