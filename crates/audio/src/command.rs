//! Cross-thread control messages.
//!
//! `Control` goes from the app/producer thread to the PipeWire loop thread over a
//! [`pipewire::channel`](crate::output::control_channel), so pause/resume/quit are all
//! executed on the loop thread (the only thread allowed to call `pw_stream` methods and
//! `mainloop.quit()`). The heavier `Command` stream (Open/Seek/Stop) that drives the
//! producer thread lands with the engine in a later sub-unit.

/// App/producer -> PipeWire loop thread. Each variant runs on the loop thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// Activate (resume) or deactivate (pause) the stream. Deactivating stops the RT
    /// callbacks but keeps the buffered audio for a glitch-free resume.
    Active(bool),
    /// Quit the main loop so `run_loop` returns and its thread can be joined.
    Quit,
}
