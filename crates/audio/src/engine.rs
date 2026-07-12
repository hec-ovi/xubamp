//! Minimal audio engine: play one file to the default PipeWire output. It spawns the output
//! loop thread and a decode/producer thread and starts playing immediately, so the caller
//! (the binary) can run its Wayland window loop alongside. Dropping the engine stops playback
//! and joins both threads cleanly.
//!
//! Transport (pause/resume/stop/seek), a position clock in the UI, and playlists arrive with
//! the interactivity phase; this is the smallest path from a file path to sound.

use std::fmt;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rtrb::Producer;

use crate::channels::to_stereo;
use crate::command::Control;
use crate::decode::Source;
use crate::output::{control_channel, run_loop, ControlSender, RtData};
use crate::ring::{new_ring, push_block, SharedState, CHANNELS};

/// How a producer-thread push ended: the whole block was queued, the consumer was dropped (the
/// output loop exited, so the producer should stop), or a seek request arrived and supersedes
/// finishing this block (the caller loops back to handle it; the flush drops the ring anyway).
enum PushOutcome {
    Done,
    Abandoned,
    SeekPending,
}

/// Producer side: push all of `block` into the ring, retrying while the realtime side drains,
/// but bail out early if a seek request arrives so a seek issued while the ring is full (e.g.
/// paused) is honored promptly instead of waiting for the ring to drain. Not realtime.
fn push_until_seek(p: &mut Producer<f32>, block: &[f32], shared: &SharedState) -> PushOutcome {
    let mut remaining = block;
    while !remaining.is_empty() {
        if shared.has_seek() {
            return PushOutcome::SeekPending;
        }
        let n = push_block(p, remaining);
        if n == 0 {
            if p.is_abandoned() {
                return PushOutcome::Abandoned;
            }
            thread::sleep(Duration::from_millis(3));
        } else {
            remaining = &remaining[n..];
        }
    }
    PushOutcome::Done
}

/// Producer side: after a clean end of decode (or an unrecoverable decode error), wait for the
/// realtime side to drain the ring, then flag the track finished and deactivate the stream, and
/// park cheaply until a seek revives the track (returns `true`) or the consumer is dropped
/// (returns `false`, so the thread exits). Keeping the producer alive past the end is what lets
/// the seek bar scrub back into a finished track (and Play restart it from the top).
fn drain_and_park(
    p: &Producer<f32>,
    shared: &SharedState,
    ring_slots: usize,
    control: &ControlSender<Control>,
) -> bool {
    // Wait for every queued frame to reach the graph (the drain loop stops early on a seek so a
    // scrub near the end need not wait out the tail, or on abandonment so drop never hangs).
    while p.slots() < ring_slots {
        if shared.has_seek() {
            return true;
        }
        if p.is_abandoned() {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
    // The ring emptied after a clean end: the clock is already frozen at the true length (the RT
    // counts only real frames). Flag finished and deactivate so the RT stops waking to emit
    // silence, then park until a seek request (or shutdown).
    shared.finished.store(true, Ordering::Release);
    let _ = control.send(Control::Active(false));
    loop {
        if shared.has_seek() {
            return true;
        }
        if p.is_abandoned() {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Why [`AudioEngine::play`] could not start.
#[derive(Debug)]
pub enum EngineError {
    /// The file could not be opened, probed, or decoded.
    Decode(symphonia::core::errors::Error),
    /// The file opened but produced no audio frames (or no usable sample rate).
    Empty,
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::Decode(e) => write!(f, "decode error: {e}"),
            EngineError::Empty => write!(f, "no decodable audio in the file"),
        }
    }
}

impl std::error::Error for EngineError {}

/// A playing track. Holds the two worker threads and the control channel to the output loop.
pub struct AudioEngine {
    control: ControlSender<Control>,
    shared: Arc<SharedState>,
    /// The stream's frame rate, for turning the position clock into seconds. This is the file's
    /// native rate, which is the single format the output offers, so it is also the graph rate.
    rate: u32,
    /// Total track length in frames from the header, or `None` when the format reports none.
    duration_frames: Option<u64>,
    loop_thread: Option<JoinHandle<()>>,
    producer_thread: Option<JoinHandle<()>>,
}

/// A cheap, cloneable remote control for a running [`AudioEngine`]. It owns a clone of the
/// control channel, so the UI thread can pause and resume playback without holding the engine
/// itself (which stays with whoever keeps the worker threads alive). Cloned senders wake the
/// same PipeWire loop, so this coexists with the engine's own control (used for shutdown).
#[derive(Clone)]
pub struct EngineHandle {
    control: ControlSender<Control>,
    shared: Arc<SharedState>,
    rate: u32,
    duration_frames: Option<u64>,
}

impl EngineHandle {
    /// Resume (`true`) or pause (`false`) playback. Pausing deactivates the PipeWire stream, so
    /// the realtime callback stops pulling frames and the position clock holds; the decoder
    /// thread simply waits with the ring full until playback resumes.
    pub fn set_active(&self, active: bool) {
        // The only failure is a dropped receiver (the loop has already exited), which means
        // there is nothing to pause anyway, so ignoring the error is correct.
        let _ = self.control.send(Control::Active(active));
    }

    /// Set the output volume, 0..=100. Recomputes the realtime gains from this and the current
    /// balance; the RT callback picks them up on its next quantum (no stream round-trip needed).
    pub fn set_volume(&self, volume: u8) {
        self.shared.volume.store(volume.min(100) as u32, Ordering::Relaxed);
        self.shared.refresh_mix();
    }

    /// Set the stereo balance, -100..=100 (negative pans left). Recomputes the realtime gains
    /// from this and the current volume.
    pub fn set_balance(&self, balance: i8) {
        self.shared.balance.store(balance.clamp(-100, 100) as i32, Ordering::Relaxed);
        self.shared.refresh_mix();
    }

    /// Request a seek to `fraction` (0..=1) of the track. A no-op when the length is unknown (an
    /// unseekable stream), so the caller need not check. The target is clamped just short of the
    /// end, since formats that range-check a seek (WAV) reject a past-end target. The producer
    /// thread performs the actual decoder seek and rebases the clock on its next step.
    pub fn seek_fraction(&self, fraction: f32) {
        let Some(total) = self.duration_frames else {
            return;
        };
        if total == 0 {
            return;
        }
        let f = fraction.clamp(0.0, 1.0) as f64;
        let target = ((f * total as f64) as u64).min(total.saturating_sub(1));
        self.shared.request_seek(target);
    }

    /// Request a seek to the very start (frame 0): Stop's reset-to-start, and restarting a
    /// finished track on the next Play. Works even when the length is unknown.
    pub fn seek_to_start(&self) {
        self.shared.request_seek(0);
    }

    /// Total track length in whole seconds, or `None` when the format reports no length.
    pub fn duration_secs(&self) -> Option<u32> {
        match (self.duration_frames, self.rate) {
            (Some(n), rate) if rate > 0 => Some((n / rate as u64) as u32),
            _ => None,
        }
    }

    /// Current playback position as a 0..=1 fraction of the track, or `None` when the length is
    /// unknown. Drives the seek-bar thumb; holds while paused and jumps on a seek.
    pub fn position_fraction(&self) -> Option<f32> {
        let total = self.duration_frames?;
        if total == 0 {
            return None;
        }
        Some((self.shared.position_frames() as f64 / total as f64).clamp(0.0, 1.0) as f32)
    }

    /// Copy the most recent output samples (downmixed mono, oldest first) into `out` for the
    /// visualizer. Reads a lock-free snapshot of the RT scope ring; while paused or stopped the
    /// tap holds its last values (the RT is not writing), which the visualizer decays to baseline.
    pub fn read_scope(&self, out: &mut [f32]) {
        self.shared.read_scope(out);
    }

    /// Elapsed whole seconds of playback, for the MM:SS time display. Derived from the same
    /// position clock as [`AudioEngine::position_frames`], so it holds while paused and is 0
    /// before any frame has played.
    pub fn elapsed_secs(&self) -> u32 {
        if self.rate == 0 {
            0
        } else {
            (self.shared.position_frames() / self.rate as u64) as u32
        }
    }

    /// Whether the track has played to its end. Once true, the position clock is frozen at the
    /// track's length and the stream has been deactivated; a future playlist reads this to
    /// advance to the next track.
    pub fn is_finished(&self) -> bool {
        self.shared.is_finished()
    }

    /// Whether the output stream is currently active (playing) rather than paused or stopped. The
    /// visualizer animates from live audio only while this holds.
    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }
}

impl AudioEngine {
    /// Open `path`, connect a stereo output stream at the file's native rate (PipeWire
    /// converts to the device), and start decoding it into the ring on a background thread.
    /// Returns once playback has started; the two threads keep running until the engine drops.
    pub fn play(path: &Path) -> Result<Self, EngineError> {
        let mut src = Source::open(path).map_err(EngineError::Decode)?;

        // Decode the first packet so rate/channels come from real data (MP3 may report them
        // as unknown until then). Copy it out so the borrow of `src` ends.
        let first: Vec<f32> = match src.next_interleaved().map_err(EngineError::Decode)? {
            Some(block) => block.to_vec(),
            None => return Err(EngineError::Empty),
        };
        let rate = src.sample_rate;
        let channels = src.channels;
        let duration_frames = src.total_frames;
        if rate == 0 {
            return Err(EngineError::Empty);
        }

        let cap_frames = (rate as usize / 2).max(2048); // ~0.5 s of headroom
        let ring_slots = cap_frames * CHANNELS;
        let (mut producer, consumer) = new_ring(cap_frames);
        let shared = Arc::new(SharedState::new());
        let (control, rx) = control_channel();

        let rt = RtData {
            consumer,
            shared: Arc::clone(&shared),
        };
        let loop_thread = thread::spawn(move || {
            if let Err(e) = run_loop(rx, rt, rate) {
                eprintln!("xubamp-audio: PipeWire loop error: {e}");
            }
        });

        // Clones for the producer thread so it can flag end-of-track, seek, and stop the stream.
        let shared_producer = Arc::clone(&shared);
        let control_producer = control.clone();
        // Producer: prime with the first block, then loop decoding into the ring. Between blocks
        // it services seek requests (reposition the decoder, flush stale audio, rebase the clock);
        // on a clean end (or a decode error) it drains, flags the track finished, and parks so a
        // later seek can scrub back in and restart it. Every wait bails out if the consumer is
        // dropped (engine drop -> loop thread gone), so no path hangs on shutdown.
        let producer_thread = thread::spawn(move || {
            let mut stereo = Vec::new();
            // Prime with the first block (already decoded to learn the rate). A seek racing in
            // here discards it (the flush drops the ring), which is correct: we start from target.
            to_stereo(&first, channels, &mut stereo);
            if let PushOutcome::Abandoned = push_until_seek(&mut producer, &stereo, &shared_producer)
            {
                return;
            }
            loop {
                // Service a pending seek before decoding on, so a seek issued while the ring is
                // full (e.g. paused) is honored promptly. Flush only when audio is actually
                // queued; an empty ring (finished/stopped) needs no flush and must not lose the
                // fresh post-seek audio when the stream reactivates.
                if let Some(target) = shared_producer.take_seek() {
                    let secs = target as f64 / rate as f64;
                    match src.seek(secs) {
                        Ok(landed) => shared_producer.begin_seek(landed),
                        Err(e) => eprintln!("xubamp-audio: seek error: {e}"),
                    }
                    continue;
                }
                match src.next_interleaved() {
                    Ok(Some(block)) => {
                        stereo.clear();
                        to_stereo(block, channels, &mut stereo);
                        match push_until_seek(&mut producer, &stereo, &shared_producer) {
                            PushOutcome::Abandoned => return,
                            PushOutcome::SeekPending => continue,
                            PushOutcome::Done => {}
                        }
                    }
                    Ok(None) => {
                        // Clean end: drain, flag finished, and park until a seek revives us.
                        if !drain_and_park(&producer, &shared_producer, ring_slots, &control_producer)
                        {
                            return;
                        }
                    }
                    Err(e) => {
                        eprintln!("xubamp-audio: decode error: {e}");
                        if !drain_and_park(&producer, &shared_producer, ring_slots, &control_producer)
                        {
                            return;
                        }
                    }
                }
            }
        });

        Ok(Self {
            control,
            shared,
            rate,
            duration_frames,
            loop_thread: Some(loop_thread),
            producer_thread: Some(producer_thread),
        })
    }

    /// Frames played so far. Basis for a future time display.
    pub fn position_frames(&self) -> u64 {
        self.shared.position_frames()
    }

    /// Whether the track has played through to its end.
    pub fn is_finished(&self) -> bool {
        self.shared.is_finished()
    }

    /// Total track length in whole seconds, or `None` when the format reports no length.
    pub fn duration_secs(&self) -> Option<u32> {
        match (self.duration_frames, self.rate) {
            (Some(n), rate) if rate > 0 => Some((n / rate as u64) as u32),
            _ => None,
        }
    }

    /// A cloneable remote control (pause/resume, seek, elapsed time) that can outlive borrows of
    /// the engine.
    pub fn handle(&self) -> EngineHandle {
        EngineHandle {
            control: self.control.clone(),
            shared: Arc::clone(&self.shared),
            rate: self.rate,
            duration_frames: self.duration_frames,
        }
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        // Quit the loop first: it returns from `run_loop` and drops the ring Consumer, which
        // makes the producer's `push_all`/drain loop observe `is_abandoned()` and stop
        // promptly instead of playing the rest of the track. Then join both threads.
        let _ = self.control.send(Control::Quit);
        if let Some(h) = self.producer_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.loop_thread.take() {
            let _ = h.join();
        }
    }
}
