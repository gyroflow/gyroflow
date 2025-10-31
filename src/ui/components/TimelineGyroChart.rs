// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#![allow(non_snake_case)]

use std::collections::BTreeMap;

use gyroflow_core::stabilization_params::StabilizationParams;
use gyroflow_core::keyframes::{ KeyframeManager, KeyframeType };
use qmetaobject::*;
use crate::core::gyro_source::{ GyroSource, TimeIMU, TimeQuat };
use crate::util;

#[derive(Debug, Clone)]
pub struct ChartData<const I: usize> {
    pub timestamp_us: i64,
    pub values: [f64; I]
}

#[derive(Default)]
struct Series {
    data: BTreeMap<i64, f64>, // timestamp, value
    lines: Vec<Vec<QLineF>>,
    is_optflow: bool,
    is_fovs: bool,
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
    theme: qt_property!(String),

    setDurationMs: qt_method!(fn(&mut self, v: f64)),
    setVScaleToVisibleArea: qt_method!(fn(&mut self)),
    setAxisVisible: qt_method!(fn(&mut self, a: usize, v: bool)),
    getAxisVisible: qt_method!(fn(&self, a: usize) -> bool),
    axisVisibleChanged: qt_signal!(),
    viewModeChanged: qt_signal!(),

    viewMode: qt_property!(u32; WRITE setViewMode NOTIFY viewModeChanged),

    series: [Series; 4+4+1+1+2], // +2 for raw FL and smoothed FL

    sync_points: BTreeMap<i64, (f64, f64)>, // timestamp, (offset, fitted offset)

    gyro: Vec<ChartData<3>>,
    accl: Vec<ChartData<3>>,
    magn: Vec<ChartData<3>>,
    quats: Vec<ChartData<4>>,
    fovs: Vec<ChartData<1>>,
    minimal_fovs: Vec<ChartData<1>>,
    smoothed_quats: Vec<ChartData<4>>,
    sync_results: Vec<ChartData<3>>,
    org_sync_results: Vec<ChartData<3>>,
    sync_quats: Vec<ChartData<4>>,
    org_sync_quats: Vec<ChartData<4>>,
    focal_lengths: Vec<ChartData<1>>,
    smoothed_focal_lengths: Vec<ChartData<1>>,

    gyro_max: Option<f64>,
    duration_ms: f64,
}

impl TimelineGyroChart {
    pub fn setDurationMs(&mut self, v: f64) { self.duration_ms = v; }
    fn setVisibleAreaLeft (&mut self, v: f64) { self.visibleAreaLeft = v; self.update(); }
    fn setVisibleAreaRight(&mut self, v: f64) { self.visibleAreaRight = v; self.update(); }
    fn setAxisVisible     (&mut self, a: usize, v: bool) { if let Some(a) = self.series.get_mut(a) { a.visible = v; self.update(); self.axisVisibleChanged(); } }
    fn getAxisVisible     (&self, a: usize) -> bool { self.series.get(a).map(|x| x.visible).unwrap_or_default() }
    fn setVScale          (&mut self, v: f64) { self.vscale = v.max(0.1); self.update(); }
    fn setViewMode        (&mut self, v: u32) { self.viewMode = v; self.update_data(""); self.viewModeChanged(); }

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
        util::qt_queued_callback(QPointer::from(self as &Self), |this, _| {
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
                let vscale = if serie.is_fovs { 1.0 } else { self.vscale };
                let add_y =  if serie.is_fovs { half_height } else { 0.0 };
                if num_samples > 1 {
                    if let Some(first_item) = range.next() {
                        let mut line = Vec::new();
                        let mut prev_point = (*first_item.0, QPointF {
                            x: map_to_visible_area(*first_item.0 as f64 / duration_us) * rect.width,
                            y: (1.0 - *first_item.1 * vscale) * half_height + add_y
                        });
                        let step = (num_samples / resolution as usize).max(1);
                        for data in range.step_by(step) {
                            let point = QPointF {
                                x: map_to_visible_area(*data.0 as f64 / duration_us) * rect.width,
                                y: (1.0 - *data.1 * vscale) * half_height + add_y
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
        p.set_brush(QBrush::default());

        for l in &self.series[a].lines {
            if !l.is_empty() {
                p.draw_lines(l.as_slice());
            }
        }
    }
    fn drawOverlay(&mut self, p: &mut QPainter, a: usize, color: &str) {
        let rect = (self as &dyn QQuickItem).bounding_rect();

        let map_to_visible_area = |v: f64| -> f64 { (v - self.visibleAreaLeft) / (self.visibleAreaRight - self.visibleAreaLeft) };

        let mut pen = QPen::from_color(QColor::from_name(color));
        pen.set_width_f(1.5); // TODO * dpiScale
        p.set_pen(pen);

        let mut brush = QBrush::from_color(QColor::from_name(color));
        brush.set_style(BrushStyle::SolidPattern);
        p.set_brush(brush);

        for l in &self.series[a].lines {
            if !l.is_empty() {
                let mut points = Vec::with_capacity(l.len() * 2);
                points.push(QPointF { x: -2.0, y: rect.height });
                for line in l {
                    points.push(line.pt1);
                    points.push(line.pt2);
                }
                points.push(QPointF { x: rect.width + 2.0, y: rect.height });
                p.draw_polygon(&points);
            }
        }

        ////////////////////////////////////////////////////

        if self.gyro.is_empty() || self.quats.is_empty() {
            p.set_pen(QPen::from_style(PenStyle::NoPen));
            p.set_brush(QBrush::from_color(QColor::from_name("#30ff0000"))); // semi-transparent red

            let duration_us = self.duration_ms * 1000.0;

            let mut region: Option<QRectF> = None;
            for x in &self.minimal_fovs {
                if x.values[0] < 0.99 {
                    let x_pos = map_to_visible_area(x.timestamp_us as f64 / duration_us) * rect.width;
                    if let Some(region) = &mut region {
                        region.width = x_pos - region.x;
                    } else {
                        region = Some(QRectF {
                            x: map_to_visible_area(x.timestamp_us as f64 / duration_us) * rect.width,
                            y: 0.0,
                            width: 1.0 / (self.visibleAreaRight - self.visibleAreaLeft),
                            height: rect.height
                        });
                    }
                } else if let Some(region) = region.take() {
                    p.draw_rect(region);
                }
            }
            if let Some(region) = region.take() {
                p.draw_rect(region);
            }
        }
    }
    fn drawSyncPoints(&mut self, p: &mut QPainter) {
        p.set_pen(QPen::default());

        let rect = (self as &dyn QQuickItem).bounding_rect();
        if rect.width <= 0.0 || rect.height <= 0.0 || self.sync_points.is_empty() { return; }

        let map_to_visible_area = |v: f64| -> f64 { (v - self.visibleAreaLeft) / (self.visibleAreaRight - self.visibleAreaLeft) };

        let duration_us = self.duration_ms * 1000.0;

        let mut min =  999999999.0f64;
        let mut max = -999999999.0f64;
        for &(offset, _linear_offset) in self.sync_points.values() {
            min = min.min(offset);
            max = max.max(offset);
        }
        const MIN_RANGE: f64 = 30.0; // ms
        let range = max - min;
        let margin = (MIN_RANGE - range).max(0.0) / 2.0;

        let mut points = Vec::with_capacity(self.sync_points.len());
        for (ts, (offset, linear_offset)) in &self.sync_points {
            let y = 1.0 - ((linear_offset - min + margin) / range.max(MIN_RANGE));
            let x = map_to_visible_area((*ts as f64 + offset * 1000.0) / duration_us) * rect.width;
            points.push(QPointF {
                x,
                y: (8.0 + y * (rect.height - 16.0)),
            });

            let y = 1.0 - ((offset - min + margin) / range.max(MIN_RANGE));
            let pt = QPointF {
                x,
                y: (8.0 + y * (rect.height - 16.0)),
            };

            let bad_syncpoint_distance = 30.0;
            let validness = ((*offset - *linear_offset).abs()).min(bad_syncpoint_distance) / bad_syncpoint_distance; // 0 - valid (point near the line), 1 - invalid (30ms or more deviation from the line)
            p.set_brush(QBrush::from_color(QColor::from_hsv_f((112.0 * (1.0 - validness)) / 360.0, 0.84, 0.86)));
            p.draw_ellipse_with_center(pt, 3.0, 3.0);
        }

        if !points.is_empty() {
            let mut pen = QPen::from_color(QColor::from_name("#25e8d2"));
            pen.set_width_f(2.0); // TODO * dpiScale
            p.set_pen(pen);
            p.set_brush(QBrush::default());
            p.draw_polyline(points.as_slice());
        }
    }

    pub fn setSyncResults(&mut self, data: &BTreeMap<i64, TimeIMU>) {
        if self.viewMode == 0 {
            self.sync_results = Vec::with_capacity(data.len());

            for (k, x) in data {
                if let Some(g) = x.gyro.as_ref() {
                    self.sync_results.push(ChartData {
                        timestamp_us: *k,
                        values: [g[0], g[1], g[2]]
                    });
                }
            }
            self.org_sync_results = self.sync_results.clone();
            Self::normalize_height(&mut self.sync_results, self.gyro_max);
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
            self.org_sync_quats = self.sync_quats.clone();
            Self::normalize_height(&mut self.sync_quats, None);
        }
    }

    pub fn setFromGyroSource(&mut self, gyro: &GyroSource, params: &StabilizationParams, keyframes: &KeyframeManager, series: &str) {
        if series.is_empty() {
            self.gyro.clear();
            self.accl.clear();
            self.magn.clear();
            self.quats.clear();
            self.smoothed_quats.clear();
            self.sync_points = gyro.get_offsets_plus_linear();

            {
                let file_metadata = gyro.file_metadata.read();
                let raw_imu = gyro.raw_imu(&file_metadata);
                let imu_len = raw_imu.len();

                for x in raw_imu {
                    if self.viewMode == 0 {
                        if let Some(g) = x.gyro.as_ref() {
                            if self.gyro.is_empty() { self.gyro.reserve(imu_len); }
                            self.gyro.push(ChartData {
                                timestamp_us: ((x.timestamp_ms + gyro.offset_at_gyro_timestamp(x.timestamp_ms)) * 1000.0) as i64,
                                values: [g[0], g[1], g[2]]
                            });
                        }
                    }
                    if self.viewMode == 1 {
                        if let Some(a) = x.accl.as_ref() {
                            if self.accl.is_empty() { self.accl.reserve(imu_len); }
                            self.accl.push(ChartData {
                                timestamp_us: ((x.timestamp_ms + gyro.offset_at_gyro_timestamp(x.timestamp_ms)) * 1000.0) as i64,
                                values: [a[0], a[1], a[2]]
                            });
                        }
                    }
                    if self.viewMode == 2 {
                        if let Some(m) = x.magn.as_ref() {
                            if self.magn.is_empty() { self.magn.reserve(imu_len); }
                            self.magn.push(ChartData {
                                timestamp_us: ((x.timestamp_ms + gyro.offset_at_gyro_timestamp(x.timestamp_ms)) * 1000.0) as i64,
                                values: [m[0], m[1], m[2]]
                            });
                        }
                    }
                }
            }

            if self.viewMode == 3 {
                self.quats = Vec::with_capacity(gyro.quaternions.len());
                self.smoothed_quats = Vec::with_capacity(gyro.smoothed_quaternions.len());
                let add_quats = |quats: &TimeQuat, out_quats: &mut Vec<ChartData<4>>| {
                    for x in quats {
                        let mut ts = *x.0 as f64 / 1000.0;
                        ts += gyro.offset_at_gyro_timestamp(ts);
                        let q = x.1.as_vector();
                        out_quats.push(ChartData {
                            timestamp_us: (ts * 1000.0) as i64,
                            values: [q[0], q[1], q[2], q[3]]
                        });
                    }
                };
                add_quats(&gyro.quaternions, &mut self.quats);

                // Reverse the smoothed rotation to get original smoothed quaternions.
                // This is the inverse of gyro_source.rs:recompute_smoothness
                let mut org_smoothed_quats = gyro.smoothed_quaternions.clone();
                for (sq, q) in org_smoothed_quats.iter_mut().zip(gyro.quaternions.iter()) {
                    *sq.1 = (*sq.1 / q.1).inverse();
                }
                add_quats(&org_smoothed_quats, &mut self.smoothed_quats);
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

            self.sync_results = self.org_sync_results.clone();
            Self::normalize_height(&mut self.sync_results, self.gyro_max);
        }
        if series.is_empty() || series == "8" {
            let fps = params.get_scaled_fps();
            let max = *params.fovs.iter().max_by(|a, b| a.total_cmp(b)).unwrap_or(&1.0);
            self.fovs = params.fovs.iter().enumerate().map(|(i, x)| ChartData {
                timestamp_us: (gyroflow_core::timestamp_at_frame(i as i32, fps) * 1000.0).round() as i64,
                values: [max - *x]
            }).collect();
            Self::normalize_height(&mut self.fovs, None);

            self.minimal_fovs = params.minimal_fovs.iter().zip(params.fovs.iter()).enumerate().map(|(i, (min_fov, fov))| {
                let timestamp_us = (gyroflow_core::timestamp_at_frame(i as i32, fps) * 1000.0).round() as i64;

                let fov_scale = keyframes.value_at_video_timestamp(&KeyframeType::Fov, timestamp_us as f64 / 1000.0).unwrap_or(params.fov);

                ChartData {
                    timestamp_us,
                    values: [min_fov / (fov * fov_scale)]
                }
            }).collect();

            // Populate focal length series
            let ts = |i: usize| (gyroflow_core::timestamp_at_frame(i as i32, fps) * 1000.0).round() as i64;
            self.focal_lengths = params.focal_lengths.iter().enumerate()
                .filter_map(|(i, v)| v.map(|x| ChartData { timestamp_us: ts(i), values: [x] }))
                .collect();
            self.smoothed_focal_lengths = params.smoothed_focal_lengths.iter().enumerate()
                .filter_map(|(i, v)| v.map(|x| ChartData { timestamp_us: ts(i), values: [x] }))
                .collect();
            Self::normalize_height(&mut self.focal_lengths, None);
            Self::normalize_height(&mut self.smoothed_focal_lengths, None);
        }

        self.update_data(series);
    }
    fn get_serie_vector<const I: usize>(vec: &[ChartData<I>], i: usize) -> BTreeMap<i64, f64> {
        let mut ret = BTreeMap::new();
        for x in vec {
            ret.insert(x.timestamp_us, x.values[i]);
        }
        ret
    }
    pub fn update_data(&mut self, series: &str) {
        if series.is_empty() {
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
        }

        self.series[8].data = Self::get_serie_vector(&self.fovs, 0);
        self.series[8].is_fovs = true;

        // New focal length series
        self.series[10].data = Self::get_serie_vector(&self.focal_lengths, 0);
        self.series[10].is_optflow = false;  // Don't use optflow rendering
        self.series[11].data = Self::get_serie_vector(&self.smoothed_focal_lengths, 0);
        self.series[11].is_optflow = false;  // Don't use optflow rendering

        self.update();
    }

    fn normalize_height<const I: usize>(data: &mut [ChartData<I>], max: Option<f64>) -> Option<f64> {
        let max = max.unwrap_or_else(|| {
            let mut max = 0.0;
            for x in data.iter() {
                for i in 0..I {
                    if x.values[i].abs() > max { max = x.values[i].abs(); }
                }
            }
            max
        });
        if max > 0.0 {
            for x in data.iter_mut() {
                for i in 0..I {
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

        self.series[8].visible = true;

        // New: start hidden
        self.series[10].visible = false;
        self.series[11].visible = false;
    }

    fn geometry_changed(&mut self, _new: QRectF, _old: QRectF) {
        self.calculate_lines();
        (self as &dyn QQuickItem).update();
    }
}
impl QQuickPaintedItem for TimelineGyroChart {
    fn paint(&mut self, p: &mut QPainter) {
        p.set_render_hint(QPainterRenderHint::Antialiasing, true);

        let colors = if self.theme == "light" { // Light theme
            ["#8f4c4c", "#4c8f4d", "#4c7c8f", "#8f4c8f",
             "#ff8888", "#88ff88", "#88deff", "#ff88ff",
             "#10000000"
            ]
        } else { // Dark theme
            ["#8f4c4c", "#4c8f4d", "#4c7c8f", "#8f4c8f",
             "#ff8888", "#88ff88", "#88deff", "#ff88ff",
             "#10ffffff"
            ]
        };

        if self.series[0].visible { self.drawAxis(p, 0, colors[0]); } // X
        if self.series[1].visible { self.drawAxis(p, 1, colors[1]); } // Y
        if self.series[2].visible { self.drawAxis(p, 2, colors[2]); } // Z
        if self.series[3].visible { self.drawAxis(p, 3, colors[3]); } // Angle

        if self.series[4].visible { self.drawAxis(p, 4, colors[4]); } // Sync X
        if self.series[5].visible { self.drawAxis(p, 5, colors[5]); } // Sync Y
        if self.series[6].visible { self.drawAxis(p, 6, colors[6]); } // Sync Z
        if self.series[7].visible { self.drawAxis(p, 7, colors[7]); } // Sync Angle

        if self.series[8].visible { self.drawOverlay(p, 8, colors[8]); } // FOVs - zooming amount

        // New draws for focal length
        if self.series[10].visible { self.drawAxis(p, 10, "#ff6600"); } // Raw focal length (orange/darker)
        if self.series[11].visible { self.drawAxis(p, 11, "#cccc66"); } // Smoothed focal length (yellow/brighter, on top)

        if self.series[9].visible { self.drawSyncPoints(p); } // Sync points and line fit
    }
}
