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

use crate::channels::to_stereo;
use crate::command::Control;
use crate::decode::Source;
use crate::output::{control_channel, run_loop, ControlSender, RtData};
use crate::ring::{new_ring, push_all, SharedState, CHANNELS};

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

        // Clones for the producer thread so it can flag end-of-track and stop the stream.
        let shared_producer = Arc::clone(&shared);
        let control_producer = control.clone();
        // Producer: prime with the first block, decode the rest into the ring, then wait for
        // the realtime side to drain it. `push_all`/the drain loop both stop early if the
        // consumer is dropped (engine drop -> loop thread gone), so this never hangs. On a
        // clean end it flags the track finished and deactivates the stream.
        let producer_thread = thread::spawn(move || {
            let mut stereo = Vec::new();
            to_stereo(&first, channels, &mut stereo);
            if !push_all(&mut producer, &stereo) {
                return;
            }
            loop {
                match src.next_interleaved() {
                    Ok(Some(block)) => {
                        stereo.clear();
                        to_stereo(block, channels, &mut stereo);
                        if !push_all(&mut producer, &stereo) {
                            return;
                        }
                    }
                    Ok(None) => break, // clean end of stream
                    Err(e) => {
                        eprintln!("xubamp-audio: decode error: {e}");
                        break;
                    }
                }
            }
            while producer.slots() < ring_slots {
                if producer.is_abandoned() {
                    return; // engine dropped mid-drain: not a natural end, leave `finished` unset
                }
                thread::sleep(Duration::from_millis(10));
            }
            // The ring emptied after a clean end of decode: every real frame has been handed to
            // the graph. Flag the track finished (the RT side counts only real frames, so the
            // clock is already frozen at the true end) and deactivate the stream so the realtime
            // thread stops waking to emit silence.
            shared_producer.finished.store(true, Ordering::Release);
            let _ = control_producer.send(Control::Active(false));
        });

        Ok(Self {
            control,
            shared,
            rate,
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

    /// A cloneable remote control (pause/resume, elapsed time) that can outlive borrows of the
    /// engine.
    pub fn handle(&self) -> EngineHandle {
        EngineHandle {
            control: self.control.clone(),
            shared: Arc::clone(&self.shared),
            rate: self.rate,
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
