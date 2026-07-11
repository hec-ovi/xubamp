//! xubamp audio engine: decode plus PipeWire output.
//!
//! Phase 3 builds this crate incrementally (decode -> ring -> PipeWire output -> engine);
//! see `docs/ARCHITECTURE.md`. All heavy work (decode, resample, channel map) runs on a
//! producer thread; the real-time callback only copies from a lock-free ring.

pub mod channels;
pub mod command;
pub mod decode;
pub mod ring;

// PipeWire realtime output. Behind the `output` feature so the pure decode/ring/channels
// build and test on a clean host; the dev container builds with `--features output`.
#[cfg(feature = "output")]
pub mod output;
