// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#![allow(non_snake_case)]

use std::{collections::BTreeMap, str::FromStr};

use gyroflow_core::keyframes::*;
use qmetaobject::*;
use crate::util;

struct Point {
    point: QPointF,
    timestamp: i64,
    value: f64
}
struct Series {
    line: Vec<QPointF>,
    points: Vec<Point>,
    playback_keyframe_idx: i32,
}

const POINT_SIZE: f64 = 3.5;
const TOP_BOTTOM_MARGIN: f64 = 5.0;

#[derive(Default, QObject)]
pub struct TimelineKeyframesView {
    base: qt_base_class!(trait QQuickPaintedItem),

    visibleAreaLeft: qt_property!(f64; WRITE setVisibleAreaLeft),
    visibleAreaRight: qt_property!(f64; WRITE setVisibleAreaRight),
    vscale: qt_property!(f64; WRITE setVScale),
    videoTimestamp: qt_property!(f64; WRITE setVideoTimestamp),

    setDurationMs: qt_method!(fn(&mut self, v: f64)),
    keyframeAtXY: qt_method!(fn(&self, x: f64, y: f64) -> QJSValue),

    nextKeyframe: qt_method!(fn(&self, typ: String) -> QJSValue),
    prevKeyframe: qt_method!(fn(&self, typ: String) -> QJSValue),

    series: BTreeMap<KeyframeType, Series>,

    mgr: KeyframeManager,

    duration_ms: f64,
}

impl TimelineKeyframesView {
    pub fn setDurationMs(&mut self, v: f64) { self.duration_ms = v; }
    fn setVisibleAreaLeft (&mut self, v: f64) { self.visibleAreaLeft = v; self.update(); }
    fn setVisibleAreaRight(&mut self, v: f64) { self.visibleAreaRight = v; self.update(); }
    fn setVScale          (&mut self, v: f64) { self.vscale = v.max(0.1); self.update(); }
    fn setVideoTimestamp  (&mut self, v: f64) { self.videoTimestamp = v; self.update_video_timestamp(true); }

    fn keyframeAtXY(&self, x: f64, y: f64) -> QJSValue {
        for (kf, v) in &self.series {
            for pt in &v.points {
                if x >= pt.point.x - 8.0 && x <= pt.point.x + 8.0 &&
                   y >= pt.point.y - 8.0 && y <= pt.point.y + 8.0 {
                    return QJSValue::from(QString::from(format!("{:?}:{}:{}:{}", kf, pt.timestamp, keyframe_text(kf), keyframe_format_value(kf, pt.value))));
                }
            }
        }

        QJSValue::default()
    }

    fn nextKeyframe(&self, typ: String) -> QJSValue {
        if let Some(res) = self.mgr.next_keyframe((self.videoTimestamp * 1000.0) as i64, KeyframeType::from_str(&typ).ok()) {
            dbg!("{:?}",res);
            return QJSValue::from(QString::from(format!("{:?}:{}:{}:{}", res.0, res.1, keyframe_text(&res.0), keyframe_format_value(&res.0, res.2.value))));
        }
        QJSValue::default()
    }
    fn prevKeyframe(&self, typ: String) -> QJSValue {
        if let Some(res) = self.mgr.prev_keyframe((self.videoTimestamp * 1000.0) as i64, KeyframeType::from_str(&typ).ok()) {
            dbg!("{:?}",res);
            return QJSValue::from(QString::from(format!("{:?}:{}:{}:{}", res.0, res.1, keyframe_text(&res.0), keyframe_format_value(&res.0, res.2.value))));
        }
        QJSValue::default()
    }

    fn update_video_timestamp(&mut self, redraw: bool) {
        let vid_ts = (self.videoTimestamp * 1000.0) as i64;
        let mut changed = false;
        for v in self.series.values_mut() {
            let mut new_idx: i32 = -1;

            if let Some(idx) = v.points.iter().position(|pt| pt.timestamp == vid_ts) {
                new_idx = idx as i32;
            }
            if new_idx != v.playback_keyframe_idx {
                v.playback_keyframe_idx = new_idx;
                changed = true;
            }
        }
        if redraw && changed { self.update(); }
    }

    pub fn update(&mut self) {
        self.calculate_lines();
        self.update_video_timestamp(false);
        util::qt_queued_callback(self, |this, _| {
            (this as &dyn QQuickItem).update();
        })(());
    }
    fn calculate_lines(&mut self) {
        let rect = (self as &dyn QQuickItem).bounding_rect();
        if rect.width <= 0.0 || rect.height <= 0.0 { return; }

        let map_to_visible_area = |v: f64| -> f64 { (v - self.visibleAreaLeft) / (self.visibleAreaRight - self.visibleAreaLeft) };
        let map_from_visible_area = |v: f64| -> f64 { v * (self.visibleAreaRight - self.visibleAreaLeft) + self.visibleAreaLeft };

        let duration_us = (self.duration_ms * 1000.0).round();

        self.series.clear();

        for kf in self.mgr.get_all_keys() {
            let mut line = Vec::with_capacity(rect.width as usize);
            let mut points = Vec::new();
            let mut max = 1.0;
            let mut min = -1.0;
            if let Some(all_keyframes) = self.mgr.get_keyframes(kf) {
                if let Some(v) = all_keyframes.values().max_by(|a, b| a.value.total_cmp(&b.value)) {
                    if v.value > max { max = v.value; }
                }
                if let Some(v) = all_keyframes.values().min_by(|a, b| a.value.total_cmp(&b.value)) {
                    if v.value < min { min = v.value; }
                }
                if max == min { max = 1.0; min = -1.0; }

                let both_max = max.abs().max(min.abs());
                max = both_max;
                min = -both_max;

                for (ts, v) in all_keyframes {
                    points.push(Point {
                        point: QPointF {
                            x: map_to_visible_area(*ts as f64 / duration_us) * rect.width,
                            y: TOP_BOTTOM_MARGIN + (1.0 - ((v.value - min) / (max - min)) * self.vscale) * (rect.height - TOP_BOTTOM_MARGIN*2.0)
                        },
                        timestamp: *ts,
                        value: v.value
                    });
                }
            }

            for x in 0..rect.width as i32 {
                let p = x as f64 / rect.width;
                let timestamp_ms = map_from_visible_area(p) * self.duration_ms / self.mgr.timestamp_scale.unwrap_or(1.0);
                if let Some(v) = self.mgr.value_at_video_timestamp(kf, timestamp_ms) {
                    let point = QPointF {
                        x: x as f64,
                        y: TOP_BOTTOM_MARGIN + (1.0 - ((v - min) / (max - min)) * self.vscale) * (rect.height - TOP_BOTTOM_MARGIN*2.0)
                    };
                    line.push(point);
                }
            }
            self.series.insert(*kf, Series { line, points, playback_keyframe_idx: -1 });
        }
    }

    fn drawKeyframe(&self, p: &mut QPainter, keyframe: &KeyframeType, color: &str) {
        let color = QColor::from_name(color);
        let mut pen = QPen::from_color(color);
        pen.set_width_f(1.0); // TODO * dpiScale

        p.set_brush(QBrush::from_style(BrushStyle::NoBrush));
        p.set_pen(pen);
        p.draw_polyline(self.series[keyframe].line.as_slice());

        p.set_brush(QBrush::from_color(color));
        p.set_pen(QPen::from_style(PenStyle::NoPen));
        for pt in &self.series[keyframe].points {
            p.draw_ellipse_with_center(pt.point, POINT_SIZE, POINT_SIZE);
        }

        let idx = &self.series[keyframe].playback_keyframe_idx;
        if *idx >= 0 && *idx < self.series[keyframe].points.len() as i32 {
            p.set_brush(QBrush::from_style(BrushStyle::NoBrush));

            let mut pen = QPen::from_color(QColor::from_name("white"));
            pen.set_width_f(1.0); // TODO * dpiScale
            p.set_pen(pen);

            let pt = &self.series[keyframe].points[*idx as usize];
            p.draw_ellipse_with_center(pt.point, POINT_SIZE*1.5, POINT_SIZE*1.5);
        }
    }

    pub fn setKeyframes(&mut self, mgr: &KeyframeManager) {
        self.mgr = mgr.clone();
        self.calculate_lines();
        self.update_video_timestamp(false);
        self.update();
    }
}

impl QQuickItem for TimelineKeyframesView {
    fn class_begin(&mut self) {
        self.duration_ms = 1.0;
        self.visibleAreaLeft = 0.0;
        self.visibleAreaRight = 1.0;
        self.vscale = 1.0;
    }

    fn geometry_changed(&mut self, _new: QRectF, _old: QRectF) {
        self.calculate_lines();
        self.update_video_timestamp(false);
        (self as &dyn QQuickItem).update();
    }
}

impl QQuickPaintedItem for TimelineKeyframesView {
    fn paint(&mut self, p: &mut QPainter) {
        p.set_render_hint(QPainterRenderHint::Antialiasing, true);

        for kf in self.mgr.get_all_keys() {
            self.drawKeyframe(p, kf, keyframe_color(kf));
        }
    }
}
