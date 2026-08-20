// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rodrigo Sclosa

#![allow(non_snake_case)]

//! Timeline lane that draws the external audio waveform.
//!
//! Follows the same pattern as [`super::TimelineGyroChart`]: a `QQuickPaintedItem`
//! that takes `visibleAreaLeft`/`visibleAreaRight` from QML and maps time to
//! pixels with the same formula, so that the waveform stays aligned with the
//! gyro chart right above it.
//!
//! The audio is reduced to (min, max) pairs per pixel column, so the amount of
//! data reaching the painting code doesn't depend on the track duration.

use gyroflow_core::audio::AudioTrack;
use qmetaobject::*;

use crate::util;

#[derive(Default, QObject)]
pub struct TimelineAudioWaveform {
    base: qt_base_class!(trait QQuickPaintedItem),

    /// Start of the timeline visible area, normalized to `0.0..=1.0`.
    visibleAreaLeft: qt_property!(f64; WRITE setVisibleAreaLeft),
    /// End of the timeline visible area, normalized to `0.0..=1.0`.
    visibleAreaRight: qt_property!(f64; WRITE setVisibleAreaRight),
    /// Vertical scale applied to the amplitude.
    vscale: qt_property!(f64; WRITE setVScale),
    /// `"light"` or `"dark"`, selects the stroke color.
    theme: qt_property!(String; WRITE setTheme),

    /// Duration of the *video* in milliseconds.
    ///
    /// This is the timeline time axis: the waveform is positioned against it,
    /// not against the audio duration itself.
    durationMs: qt_property!(f64; WRITE setDurationMs),

    /// Audio offset in seconds (`t_audio = t_video + offset`).
    ///
    /// Written by the offset slider; the drawing moves without the audio being
    /// decoded again.
    offsetSeconds: qt_property!(f64; WRITE setOffsetSeconds),

    /// Whether a track is loaded - QML uses it to show or hide the lane.
    hasAudio: qt_property!(bool; READ hasAudio NOTIFY audioChanged),
    audioChanged: qt_signal!(),

    /// Discards the drawn track, when another video is loaded and the previous
    /// clip's track no longer applies.
    clear: qt_method!(fn(&mut self)),

    track: Option<AudioTrack>,

    /// Peaks computed for the visible area: one (min, max) pair per pixel column.
    peaks: Vec<(f32, f32)>,

    /// Track duration in seconds, cached to avoid recomputing it on every repaint.
    audio_duration: f64,
}

impl TimelineAudioWaveform {
    fn setVisibleAreaLeft(&mut self, v: f64) { self.visibleAreaLeft = v; self.update(); }
    fn setVisibleAreaRight(&mut self, v: f64) { self.visibleAreaRight = v; self.update(); }
    fn setVScale(&mut self, v: f64) { self.vscale = v; self.update(); }
    fn setTheme(&mut self, v: String) { self.theme = v; self.update(); }
    fn setDurationMs(&mut self, v: f64) { self.durationMs = v; self.update(); }
    fn setOffsetSeconds(&mut self, v: f64) { self.offsetSeconds = v; self.update(); }

    fn hasAudio(&self) -> bool { self.track.is_some() }

    fn clear(&mut self) { self.set_track(None); }

    /// Installs the decoded track and redraws.
    pub fn set_track(&mut self, track: Option<AudioTrack>) {
        self.audio_duration = track.as_ref().map_or(0.0, |t| t.duration_seconds());
        self.track = track;
        self.audioChanged();
        self.update();
    }

    /// Recomputes the peaks and schedules the repaint.
    ///
    /// `qt_queued_callback` is the same mechanism the gyro chart uses: it makes
    /// sure the item's `update` happens on the UI thread.
    pub fn update(&mut self) {
        self.calculate_peaks();
        util::qt_queued_callback(QPointer::from(self as &Self), |this, _| {
            (this as &dyn QQuickItem).update();
        })(());
    }

    /// Reduces the visible audio range to one (min, max) pair per pixel.
    ///
    /// Only the visible range is processed: when zoomed in on a long clip, this
    /// avoids walking the whole file on every frame.
    fn calculate_peaks(&mut self) {
        self.peaks.clear();

        let rect = (self as &dyn QQuickItem).bounding_rect();
        if rect.width <= 0.0 || rect.height <= 0.0 || self.durationMs <= 0.0 {
            return;
        }

        let Some(track) = &self.track else { return };
        if track.is_empty() {
            return;
        }

        let channels = track.channels as usize;
        let sample_rate = track.sample_rate as f64;
        if channels == 0 || sample_rate <= 0.0 {
            return;
        }

        let width = rect.width as usize;
        let duration_s = self.durationMs / 1000.0;

        // Video timestamps at the left and right edges of the visible area.
        let visible_from_s = self.visibleAreaLeft * duration_s;
        let visible_to_s = self.visibleAreaRight * duration_s;
        if visible_to_s <= visible_from_s {
            return;
        }

        let seconds_per_pixel = (visible_to_s - visible_from_s) / rect.width;
        let total_frames = track.frame_count();

        self.peaks.reserve(width);
        for px in 0..width {
            // Pixel column -> video time -> audio time: `t_audio = t_video + offset`.
            let t_video = visible_from_s + px as f64 * seconds_per_pixel;
            let t_audio_start = t_video + self.offsetSeconds;
            let t_audio_end = t_audio_start + seconds_per_pixel;

            let first = (t_audio_start * sample_rate).floor();
            let last = (t_audio_end * sample_rate).ceil();

            // Outside the track bounds the drawing stays flat - that is how the
            // user sees where the audio starts and ends relative to the video.
            if last <= 0.0 || first >= total_frames as f64 {
                self.peaks.push((0.0, 0.0));
                continue;
            }

            let first_frame = (first.max(0.0) as usize).min(total_frames);
            let last_frame = (last.max(0.0) as usize).min(total_frames).max(first_frame + 1);

            let mut min = f32::MAX;
            let mut max = f32::MIN;
            let from = first_frame * channels;
            let to = (last_frame * channels).min(track.samples.len());
            for value in &track.samples[from..to] {
                if *value < min { min = *value; }
                if *value > max { max = *value; }
            }

            if min > max {
                self.peaks.push((0.0, 0.0));
            } else {
                self.peaks.push((min, max));
            }
        }
    }
}

impl QQuickItem for TimelineAudioWaveform {
    fn geometry_changed(&mut self, _new: QRectF, _old: QRectF) {
        self.update();
    }
}

impl QQuickPaintedItem for TimelineAudioWaveform {
    fn paint(&mut self, p: &mut QPainter) {
        if self.peaks.is_empty() {
            return;
        }

        let rect = (self as &dyn QQuickItem).bounding_rect();
        let half_height = rect.height / 2.0;

        // Teal: same family as the colors already used in the timeline, but
        // distinct from the gyro series so the lanes aren't confused.
        let color = if self.theme == "light" { "#3a7d7d" } else { "#63c5c5" };

        p.set_render_hint(QPainterRenderHint::Antialiasing, false);
        let mut pen = QPen::from_color(QColor::from_name(color));
        pen.set_width_f(1.0);
        p.set_pen(pen);
        p.set_brush(QBrush::default());

        let vscale = if self.vscale > 0.0 { self.vscale } else { 1.0 };

        let lines: Vec<QLineF> = self
            .peaks
            .iter()
            .enumerate()
            .map(|(px, (min, max))| {
                let x = px as f64;
                QLineF {
                    pt1: QPointF { x, y: half_height * (1.0 - (*max as f64 * vscale).clamp(-1.0, 1.0)) },
                    pt2: QPointF { x, y: half_height * (1.0 - (*min as f64 * vscale).clamp(-1.0, 1.0)) },
                }
            })
            .collect();

        p.draw_lines(lines.as_slice());
    }
}
