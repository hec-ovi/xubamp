//! Lock-free UI-to-producer equalizer control and the producer-owned DSP instance.
//!
//! The decoder thread owns all stateful filters. UI handles publish a coherent [`EqSettings`]
//! snapshot through atomics; the producer checks the revision once per decoded block. Nothing on
//! the PipeWire realtime thread reads this state or runs the filters.

use std::hint::spin_loop;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use xubamp_dsp::{EqSettings, Equalizer};

/// A coherent atomic [`EqSettings`] snapshot shared by any number of UI handles and the single
/// decode producer. Writers briefly claim an odd revision while replacing the fields, then publish
/// the next even revision. This avoids a mutex in the audio path and prevents mixed presets when a
/// whole bank of sliders changes at once.
pub(crate) struct EqControl {
    revision: AtomicU64,
    enabled: AtomicBool,
    preamp_db: AtomicU32,
    bands_db: [AtomicU32; 10],
}

impl EqControl {
    pub(crate) fn new(settings: EqSettings) -> Self {
        let settings = settings.sanitized();
        Self {
            revision: AtomicU64::new(0),
            enabled: AtomicBool::new(settings.enabled),
            preamp_db: AtomicU32::new(settings.preamp_db.to_bits()),
            bands_db: std::array::from_fn(|i| AtomicU32::new(settings.bands_db[i].to_bits())),
        }
    }

    /// Publish all eleven controls and the enabled flag as one logical update. Multiple cloned
    /// handles may call this concurrently; writers serialize only against other writers, while the
    /// producer keeps taking lock-free snapshots between decoded blocks.
    pub(crate) fn publish(&self, settings: EqSettings) {
        let settings = settings.sanitized();
        let revision = loop {
            let revision = self.revision.load(Ordering::SeqCst);
            if revision & 1 != 0 {
                spin_loop();
                continue;
            }
            if self
                .revision
                .compare_exchange_weak(
                    revision,
                    revision.wrapping_add(1),
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                break revision;
            }
        };

        self.enabled.store(settings.enabled, Ordering::SeqCst);
        self.preamp_db
            .store(settings.preamp_db.to_bits(), Ordering::SeqCst);
        for (slot, value) in self.bands_db.iter().zip(settings.bands_db) {
            slot.store(value.to_bits(), Ordering::SeqCst);
        }
        self.revision
            .store(revision.wrapping_add(2), Ordering::SeqCst);
    }

    /// Read a whole preset without tearing across fields. The returned revision lets the producer
    /// skip coefficient rebuilds when no control changed.
    pub(crate) fn snapshot(&self) -> (u64, EqSettings) {
        loop {
            if let Some(snapshot) = self.try_snapshot() {
                return snapshot;
            }
            spin_loop();
        }
    }

    /// Producer-side nonblocking read. If a UI writer is between fields, keep processing with the
    /// prior coherent preset and try again on the next decoded block.
    fn try_snapshot(&self) -> Option<(u64, EqSettings)> {
        let before = self.revision.load(Ordering::SeqCst);
        if before & 1 != 0 {
            return None;
        }
        let settings = EqSettings {
            enabled: self.enabled.load(Ordering::SeqCst),
            preamp_db: f32::from_bits(self.preamp_db.load(Ordering::SeqCst)),
            bands_db: std::array::from_fn(|i| {
                f32::from_bits(self.bands_db[i].load(Ordering::SeqCst))
            }),
        };
        let after = self.revision.load(Ordering::SeqCst);
        (before == after).then_some((after, settings))
    }
}

/// The decode-thread side of the equalizer. It owns all delay state and applies settings only when
/// the atomic revision changes.
pub(crate) struct ProducerEqualizer {
    equalizer: Equalizer,
    revision: u64,
    control: Arc<EqControl>,
}

impl ProducerEqualizer {
    pub(crate) fn new(sample_rate: u32, control: Arc<EqControl>) -> Self {
        let (revision, settings) = control.snapshot();
        Self {
            equalizer: Equalizer::new(sample_rate, settings),
            revision,
            control,
        }
    }

    pub(crate) fn process(&mut self, stereo: &mut [f32]) {
        if let Some((revision, settings)) = self.control.try_snapshot() {
            if revision != self.revision {
                self.equalizer.set_settings(settings);
                self.revision = revision;
            }
        }
        self.equalizer.process_interleaved(stereo);
    }

    /// Clear filter delay history at a seek or another track discontinuity. Current slider values
    /// remain in force for the first fresh sample after the boundary.
    pub(crate) fn reset(&mut self) {
        self.equalizer.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring::{fill_output, new_ring, push_block, SharedState};
    use std::f32::consts::TAU;
    use std::thread;

    fn through_producer_and_ring(settings: EqSettings, input: &[f32]) -> Vec<f32> {
        let control = Arc::new(EqControl::new(settings));
        let mut equalizer = ProducerEqualizer::new(48_000, control);
        let mut block = input.to_vec();
        equalizer.process(&mut block);

        let frames = block.len() / 2;
        let (mut producer, mut consumer) = new_ring(frames);
        assert_eq!(push_block(&mut producer, &block), block.len());
        let shared = SharedState::new();
        let mut output = vec![0.0; block.len()];
        assert_eq!(fill_output(&mut consumer, &mut output, &shared), frames);
        output
    }

    fn rms_left(samples: &[f32], skip_frames: usize) -> f32 {
        let (sum, count) = samples
            .chunks_exact(2)
            .skip(skip_frames)
            .fold((0.0f64, 0usize), |(sum, count), frame| {
                (sum + frame[0] as f64 * frame[0] as f64, count + 1)
            });
        (sum / count as f64).sqrt() as f32
    }

    #[test]
    fn neutral_settings_are_sample_exact_through_the_output_ring() {
        let input = vec![0.25, -0.5, -0.125, 0.75, 0.0, -0.0, 1.0, -1.0];
        let output = through_producer_and_ring(EqSettings::default(), &input);
        assert_eq!(output, input);
    }

    #[test]
    fn enabled_band_boost_reaches_the_output_ring() {
        let frames = 12_000;
        let mut input = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let sample = (TAU * 1_000.0 * i as f32 / 48_000.0).sin() * 0.05;
            input.extend_from_slice(&[sample, sample]);
        }
        let mut settings = EqSettings::default();
        settings.bands_db[4] = 12.0;
        let output = through_producer_and_ring(settings, &input);
        let ratio = rms_left(&output, 2_400) / rms_left(&input, 2_400);
        assert!(
            ratio > 3.7 && ratio < 4.3,
            "+12 dB output ratio was {ratio}"
        );
    }

    #[test]
    fn a_published_setting_takes_effect_on_the_next_block() {
        let control = Arc::new(EqControl::new(EqSettings::default()));
        let mut equalizer = ProducerEqualizer::new(48_000, Arc::clone(&control));
        let mut before = [0.25, -0.25];
        equalizer.process(&mut before);
        assert_eq!(before, [0.25, -0.25]);

        control.publish(EqSettings {
            preamp_db: 6.0,
            ..EqSettings::default()
        });
        let mut after = [0.25, -0.25];
        equalizer.process(&mut after);
        let gain = xubamp_dsp::db_to_gain(6.0);
        assert!((after[0] - 0.25 * gain).abs() < 1e-6);
        assert!((after[1] + 0.25 * gain).abs() < 1e-6);
    }

    #[test]
    fn producer_never_waits_for_an_in_progress_ui_update() {
        let control = Arc::new(EqControl::new(EqSettings::default()));
        let mut equalizer = ProducerEqualizer::new(48_000, Arc::clone(&control));
        // Simulate a writer being descheduled after claiming the odd revision. The producer must
        // render this block with its last coherent settings instead of spinning behind the UI.
        control.revision.store(1, Ordering::SeqCst);
        let mut block = [0.25, -0.25];
        equalizer.process(&mut block);
        assert_eq!(block, [0.25, -0.25]);
        control.revision.store(2, Ordering::SeqCst);
    }

    #[test]
    fn reset_removes_filter_history_at_a_discontinuity() {
        let mut settings = EqSettings::default();
        settings.bands_db[4] = 12.0;
        let control = Arc::new(EqControl::new(settings));
        let mut equalizer = ProducerEqualizer::new(48_000, control);
        let mut impulse = vec![0.0; 512];
        impulse[0] = 1.0;
        equalizer.process(&mut impulse);

        equalizer.reset();
        let mut fresh_silence = [0.0; 512];
        equalizer.process(&mut fresh_silence);
        assert!(fresh_silence.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn concurrent_whole_preset_updates_never_tear() {
        let control = Arc::new(EqControl::new(EqSettings::default()));
        let writer = Arc::clone(&control);
        let thread = thread::spawn(move || {
            for i in 0..2_000 {
                let db = if i & 1 == 0 { -12.0 } else { 12.0 };
                writer.publish(EqSettings {
                    preamp_db: db,
                    bands_db: [db; 10],
                    ..EqSettings::default()
                });
            }
        });
        for _ in 0..4_000 {
            let (_, settings) = control.snapshot();
            assert!(
                settings
                    .bands_db
                    .iter()
                    .all(|value| *value == settings.preamp_db),
                "observed a torn equalizer preset: {settings:?}"
            );
        }
        thread.join().unwrap();
    }
}
