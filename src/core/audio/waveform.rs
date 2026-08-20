// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright © 2026 Rodrigo Sclosa

//! Waveform generation for the timeline.
//!
//! The UI is not fed sample by sample: one minute of 48 kHz stereo audio is 5.8
//! million values, while the timeline is only a few thousand pixels wide. The
//! signal is reduced to a (min, max) pair per bucket - typically one bucket per
//! pixel - which is enough to draw the classic waveform silhouette while keeping
//! transients visible.
//!
//! Peaks must be recomputed on zoom changes, since the number of samples that
//! fits in a pixel changes with it.

use super::AudioTrack;

/// (min, max) pair of the samples contained in one bucket.
pub type Peak = (f32, f32);

impl AudioTrack {
    /// Reduces the track to (min, max) pairs, one per bucket.
    ///
    /// `samples_per_bucket` is how many **frames** each bucket covers, usually
    /// `visible_frames / width_in_pixels`. 0 is treated as 1 to avoid dividing
    /// by zero.
    ///
    /// The extremes are taken across all channels, so a transient present in a
    /// single channel stays visible.
    ///
    /// Uses [`AudioTrack::samples`] (the export audio) because that is what the
    /// user expects to see; the analysis downmix is not involved in drawing.
    pub fn peaks(&self, samples_per_bucket: usize) -> Vec<Peak> {
        let channels = self.channels as usize;
        if self.samples.is_empty() || channels == 0 {
            return Vec::new();
        }

        let samples_per_bucket = samples_per_bucket.max(1);
        let total_frames = self.samples.len() / channels;
        let bucket_count = total_frames.div_ceil(samples_per_bucket);

        let mut peaks = Vec::with_capacity(bucket_count);
        for bucket in 0..bucket_count {
            let first_frame = bucket * samples_per_bucket;
            let last_frame = ((bucket + 1) * samples_per_bucket).min(total_frames);

            let mut min = f32::MAX;
            let mut max = f32::MIN;
            for value in &self.samples[first_frame * channels..last_frame * channels] {
                if *value < min {
                    min = *value;
                }
                if *value > max {
                    max = *value;
                }
            }

            // The arithmetic above prevents empty buckets; this guard only keeps
            // the sentinel values from leaking out.
            if min > max {
                peaks.push((0.0, 0.0));
            } else {
                peaks.push((min, max));
            }
        }

        peaks
    }

    /// Peaks computed for a target width in pixels.
    ///
    /// Convenience for the UI, which reasons in pixels rather than samples:
    /// splits the whole track into `width_px` buckets.
    pub fn peaks_for_width(&self, width_px: usize) -> Vec<Peak> {
        let channels = self.channels as usize;
        if width_px == 0 || channels == 0 || self.samples.is_empty() {
            return Vec::new();
        }
        let total_frames = self.samples.len() / channels;
        self.peaks(total_frames.div_ceil(width_px))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::SourceFormat;

    fn track_mono(samples: Vec<f32>) -> AudioTrack {
        AudioTrack {
            path: String::new(),
            samples,
            channels: 1,
            sample_rate: 48000,
            source_format: SourceFormat::F32,
            mono_analysis: Vec::new(),
            offset_seconds: 0.0,
            preserve_original_format: true,
        }
    }

    #[test]
    fn extremes_per_bucket() {
        let track = track_mono(vec![0.0, 1.0, -1.0, 0.5]);
        assert_eq!(track.peaks(2), vec![(0.0, 1.0), (-1.0, 0.5)]);
    }

    #[test]
    fn partial_last_bucket_is_not_dropped() {
        let track = track_mono(vec![0.2, 0.4, 0.9]);
        let peaks = track.peaks(2);
        assert_eq!(peaks.len(), 2);
        assert_eq!(peaks[1], (0.9, 0.9));
    }

    #[test]
    fn zero_bucket_size_does_not_divide_by_zero() {
        let track = track_mono(vec![0.1, 0.2]);
        assert_eq!(track.peaks(0).len(), 2);
    }

    #[test]
    fn stereo_considers_both_channels() {
        let mut track = track_mono(vec![0.0, 1.0, -1.0, 0.0]);
        track.channels = 2;
        // Single bucket over both frames: the extremes come from different
        // channels (max on the right, min on the left).
        assert_eq!(track.peaks(2), vec![(-1.0, 1.0)]);
    }

    #[test]
    fn empty_track_yields_no_peaks() {
        assert!(track_mono(Vec::new()).peaks(10).is_empty());
    }

    #[test]
    fn pixel_width_caps_the_peak_count() {
        let track = track_mono((0..1000).map(|i| i as f32 / 1000.0).collect());
        assert!(track.peaks_for_width(100).len() <= 100);
    }
}
