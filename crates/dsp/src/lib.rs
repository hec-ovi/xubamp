//! Realtime-safe equalizer DSP and Winamp EQF preset I/O.
//!
//! The audio path owns one [`Equalizer`] and mutates samples in place without allocation. The first
//! and last controls are low/high shelves and the eight middle controls are peaking filters, matching
//! Webamp's public implementation. Every control spans the classic -12..=12 dB range.

pub mod eqf;
mod filter;
pub mod presets;

use filter::{Biquad, FilterKind};

pub const BAND_FREQUENCIES: [u32; 10] = [
    60, 170, 310, 600, 1_000, 3_000, 6_000, 12_000, 14_000, 16_000,
];
pub const MIN_DB: f32 = -12.0;
pub const MAX_DB: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqSettings {
    pub enabled: bool,
    pub preamp_db: f32,
    pub bands_db: [f32; 10],
}

impl Default for EqSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            preamp_db: 0.0,
            bands_db: [0.0; 10],
        }
    }
}

impl EqSettings {
    pub fn sanitized(mut self) -> Self {
        self.preamp_db = sanitize_db(self.preamp_db);
        for value in &mut self.bands_db {
            *value = sanitize_db(*value);
        }
        self
    }
}

fn sanitize_db(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(MIN_DB, MAX_DB)
    } else {
        0.0
    }
}

/// Stateful stereo equalizer. Coefficients may be updated between audio blocks; filter delay state
/// is retained across ordinary slider changes and explicitly cleared on seek/track discontinuities.
pub struct Equalizer {
    sample_rate: f32,
    settings: EqSettings,
    preamp_gain: f32,
    left: [Biquad; 10],
    right: [Biquad; 10],
}

impl Equalizer {
    pub fn new(sample_rate: u32, settings: EqSettings) -> Self {
        let sample_rate = sample_rate.max(1) as f32;
        let settings = settings.sanitized();
        let mut equalizer = Self {
            sample_rate,
            settings,
            preamp_gain: 1.0,
            left: [Biquad::bypass(); 10],
            right: [Biquad::bypass(); 10],
        };
        equalizer.rebuild_coefficients();
        equalizer
    }

    pub fn settings(&self) -> EqSettings {
        self.settings
    }

    pub fn set_settings(&mut self, settings: EqSettings) {
        let settings = settings.sanitized();
        if settings == self.settings {
            return;
        }
        self.settings = settings;
        self.rebuild_coefficients();
    }

    pub fn set_band_db(&mut self, index: usize, db: f32) -> bool {
        let Some(current) = self.settings.bands_db.get_mut(index) else {
            return false;
        };
        let db = sanitize_db(db);
        if *current == db {
            return true;
        }
        *current = db;
        self.rebuild_band(index);
        true
    }

    pub fn reset(&mut self) {
        for filter in self.left.iter_mut().chain(self.right.iter_mut()) {
            filter.reset();
        }
    }

    /// Process interleaved stereo samples in place. A trailing odd sample is left untouched instead
    /// of panicking; the engine normally supplies exact stereo frames.
    pub fn process_interleaved(&mut self, samples: &mut [f32]) {
        if !self.settings.enabled {
            return;
        }
        for frame in samples.chunks_exact_mut(2) {
            let mut left = frame[0] * self.preamp_gain;
            let mut right = frame[1] * self.preamp_gain;
            for filter in &mut self.left {
                left = filter.process(left);
            }
            for filter in &mut self.right {
                right = filter.process(right);
            }
            frame[0] = left;
            frame[1] = right;
        }
    }

    fn rebuild_coefficients(&mut self) {
        self.preamp_gain = db_to_gain(self.settings.preamp_db);
        for i in 0..BAND_FREQUENCIES.len() {
            self.rebuild_band(i);
        }
    }

    fn rebuild_band(&mut self, index: usize) {
        let frequency = BAND_FREQUENCIES[index] as f32;
        let db = self.settings.bands_db[index];
        let kind = match index {
            0 => FilterKind::LowShelf,
            9 => FilterKind::HighShelf,
            _ => FilterKind::Peaking,
        };
        let coefficients = filter::Coefficients::design(kind, self.sample_rate, frequency, db);
        self.left[index].set_coefficients(coefficients);
        self.right[index].set_coefficients(coefficients);
    }
}

pub fn db_to_gain(db: f32) -> f32 {
    10.0f32.powf(sanitize_db(db) / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine(frequency: f32, seconds: f32, rate: u32) -> Vec<f32> {
        let frames = (seconds * rate as f32) as usize;
        let mut out = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let value = (TAU * frequency * i as f32 / rate as f32).sin() * 0.1;
            out.extend_from_slice(&[value, value]);
        }
        out
    }

    fn rms_left(samples: &[f32], skip_frames: usize) -> f32 {
        let (sum, n) = samples
            .chunks_exact(2)
            .skip(skip_frames)
            .fold((0.0f64, 0usize), |(sum, n), frame| {
                (sum + frame[0] as f64 * frame[0] as f64, n + 1)
            });
        (sum / n as f64).sqrt() as f32
    }

    #[test]
    fn neutral_equalizer_is_sample_exact() {
        let mut samples = sine(1_000.0, 0.05, 48_000);
        let before = samples.clone();
        Equalizer::new(48_000, EqSettings::default()).process_interleaved(&mut samples);
        assert_eq!(samples, before, "zero-dB filters use an exact bypass");
    }

    #[test]
    fn disabled_equalizer_bypasses_preamp_and_bands() {
        let mut settings = EqSettings {
            enabled: false,
            preamp_db: 12.0,
            ..EqSettings::default()
        };
        settings.bands_db[4] = 12.0;
        let mut samples = sine(1_000.0, 0.05, 48_000);
        let before = samples.clone();
        Equalizer::new(48_000, settings).process_interleaved(&mut samples);
        assert_eq!(samples, before);
    }

    #[test]
    fn preamp_uses_decibel_gain() {
        let mut settings = EqSettings {
            preamp_db: 6.0,
            ..EqSettings::default()
        };
        let mut samples = vec![0.25, -0.25, -0.5, 0.5];
        Equalizer::new(48_000, settings).process_interleaved(&mut samples);
        let gain = db_to_gain(6.0);
        assert!((samples[0] - 0.25 * gain).abs() < 1e-6);
        assert!((samples[1] + 0.25 * gain).abs() < 1e-6);
        settings.preamp_db = f32::NAN;
        assert_eq!(settings.sanitized().preamp_db, 0.0);
    }

    #[test]
    fn a_band_boost_has_the_expected_center_frequency_gain() {
        let rate = 48_000;
        let input = sine(1_000.0, 0.5, rate);
        let input_rms = rms_left(&input, 4_800);
        let mut output = input.clone();
        let mut settings = EqSettings::default();
        settings.bands_db[4] = 12.0;
        Equalizer::new(rate, settings).process_interleaved(&mut output);
        let ratio = rms_left(&output, 4_800) / input_rms;
        assert!(
            ratio > 3.7 && ratio < 4.3,
            "+12 dB center ratio was {ratio}"
        );
    }

    #[test]
    fn stereo_filter_state_is_independent() {
        let mut settings = EqSettings::default();
        settings.bands_db[4] = 12.0;
        let mut eq = Equalizer::new(48_000, settings);
        let mut samples = vec![0.0; 2_000];
        samples[0] = 1.0;
        eq.process_interleaved(&mut samples);
        assert!(samples.chunks_exact(2).any(|frame| frame[0].abs() > 1e-6));
        assert!(samples.chunks_exact(2).all(|frame| frame[1] == 0.0));
    }

    #[test]
    fn band_setters_clamp_and_reject_bad_indices() {
        let mut eq = Equalizer::new(44_100, EqSettings::default());
        assert!(eq.set_band_db(0, 99.0));
        assert_eq!(eq.settings().bands_db[0], MAX_DB);
        assert!(eq.set_band_db(1, f32::INFINITY));
        assert_eq!(eq.settings().bands_db[1], 0.0);
        assert!(!eq.set_band_db(10, 1.0));
    }
}
