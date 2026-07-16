//! xubamp audio engine: decode plus PipeWire output.
//!
//! Phase 3 builds this crate incrementally (decode -> ring -> PipeWire output -> engine);
//! see `docs/ARCHITECTURE.md`. All heavy work (decode, resample, channel map, equalization) runs
//! on a producer thread; the real-time callback only copies from a lock-free ring.

pub mod channels;
pub mod command;
pub mod decode;
pub mod id3v1;
#[cfg(any(feature = "output", test))]
mod equalizer;
pub mod playlist;
pub mod playlist_file;
pub mod ring;

pub use xubamp_dsp::EqSettings;

// PipeWire realtime output + the engine that drives it. Behind the `output` feature so the
// pure decode/ring/channels build and test on a clean host; the dev container builds with
// `--features output`.
#[cfg(feature = "output")]
pub mod engine;
#[cfg(feature = "output")]
pub mod output;
