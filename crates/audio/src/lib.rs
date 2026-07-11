//! xubamp audio engine: decode plus PipeWire output.
//!
//! Phase 3 builds this crate incrementally (decode -> ring -> PipeWire output -> engine);
//! see `docs/ARCHITECTURE.md`. All heavy work (decode, resample, channel map) runs on a
//! producer thread; the real-time callback only copies from a lock-free ring.

pub mod channels;
pub mod decode;
pub mod ring;
