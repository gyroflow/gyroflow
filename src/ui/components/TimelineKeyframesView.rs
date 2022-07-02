// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2021-2022 Adrian <adrian.eddy at gmail>

#![allow(non_snake_case)]

use std::collections::BTreeMap;

use gyroflow_core::keyframes::*;
use qmetaobject::*;
use crate::util;
use cpp::*;

struct Series {
    line: Vec<QPointF>,
    points: Vec<QPointF>,
    timestamps_per_point: Vec<i64>,
}

const POINT_SIZE: f64 = 3.5;
const TOP_BOTTOM_MARGIN: f64 = 5.0;

#[derive(Default, QObject)]
pub struct TimelineKeyframesView {
    base: qt_base_class!(trait QQuickPaintedItem),

    visibleAreaLeft: qt_property!(f64; WRITE setVisibleAreaLeft),
    visibleAreaRight: qt_property!(f64; WRITE setVisibleAreaRight),
    vscale: qt_property!(f64; WRITE setVScale),

    setDurationMs: qt_method!(fn(&mut self, v: f64)),
    keyframeAtXY: qt_method!(fn(&self, x: f64, y: f64) -> QJSValue),

    series: BTreeMap<KeyframeType, Series>,

    mgr: KeyframeManager,

    duration_ms: f64,
}

impl TimelineKeyframesView {
    pub fn setDurationMs(&mut self, v: f64) { self.duration_ms = v; }
    fn setVisibleAreaLeft (&mut self, v: f64) { self.visibleAreaLeft = v; self.update(); }
    fn setVisibleAreaRight(&mut self, v: f64) { self.visibleAreaRight = v; self.update(); }
    fn setVScale          (&mut self, v: f64) { self.vscale = v.max(0.1); self.update(); }

    fn keyframeAtXY(&self, x: f64, y: f64) -> QJSValue {
        for (kf, v) in &self.series {
            for (pt, ts) in v.points.iter().zip(v.timestamps_per_point.iter()) {
                if x >= pt.x - 8.0 && x <= pt.x + 8.0 &&
                   y >= pt.y - 8.0 && y <= pt.y + 8.0 {
                    return QJSValue::from(QString::from(format!("{:?}:{}", kf, ts)));
                }
            }
        }

        QJSValue::default()
    }

    pub fn update(&mut self) {
        self.calculate_lines();
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
            let mut timestamps_per_point = Vec::new();
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
                    points.push(QPointF {
                        x: map_to_visible_area(*ts as f64 / duration_us) * rect.width,
                        y: TOP_BOTTOM_MARGIN + (1.0 - ((v.value - min) / (max - min)) * self.vscale) * (rect.height - TOP_BOTTOM_MARGIN*2.0)
                    });
                    timestamps_per_point.push(*ts);
                }
            }

            for x in 0..rect.width as i32 {
                let p = x as f64 / rect.width;
                let timestamp_ms = map_from_visible_area(p) * self.duration_ms / self.mgr.timestamp_scale.unwrap_or(1.0);
                if let Some(v) = self.mgr.value_at_video_timestamp(&kf, timestamp_ms) {
                    let point = QPointF {
                        x: x as f64,
                        y: TOP_BOTTOM_MARGIN + (1.0 - ((v - min) / (max - min)) * self.vscale) * (rect.height - TOP_BOTTOM_MARGIN*2.0)
                    };
                    line.push(point);
                }
            }
            self.series.insert(*kf, Series { line, points, timestamps_per_point });
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
            p.draw_ellipse_with_center(*pt, POINT_SIZE, POINT_SIZE);
        }
    }

    pub fn setKeyframes(&mut self, mgr: &KeyframeManager) {
        self.mgr = mgr.clone();
        self.calculate_lines();
        self.update();
    }
}

impl QQuickItem for TimelineKeyframesView {
    fn component_complete(&mut self) {
        let obj = self.get_cpp_object();
        cpp!(unsafe [obj as "QQuickItem *"] {
            obj->setAcceptedMouseButtons(Qt::AllButtons);
            obj->setAcceptHoverEvents(true);
        });
    }
    fn class_begin(&mut self) {
        self.duration_ms = 1.0;
        self.visibleAreaLeft = 0.0;
        self.visibleAreaRight = 1.0;
        self.vscale = 1.0;
    }

    fn geometry_changed(&mut self, _new: QRectF, _old: QRectF) {
        self.calculate_lines();
        (self as &dyn QQuickItem).update();
    }
    fn mouse_event(&mut self, event: QMouseEvent) -> bool {
        dbg!(event.position());
        let obj = self.get_cpp_object();
        cpp!(unsafe [obj as "QQuickItem *"] {
            obj->setCursor(Qt::PointingHandCursor);
        });
        false
    }
}

impl QQuickPaintedItem for TimelineKeyframesView {
    fn paint(&mut self, p: &mut QPainter) {
        p.set_render_hint(QPainterRenderHint::Antialiasing, true);

        for kf in self.mgr.get_all_keys() {
            self.drawKeyframe(p, kf, color_for_keyframe(kf));
        }
    }
}
