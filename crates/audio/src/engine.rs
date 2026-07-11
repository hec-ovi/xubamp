//! Minimal audio engine: play one file to the default PipeWire output. It spawns the output
//! loop thread and a decode/producer thread and starts playing immediately, so the caller
//! (the binary) can run its Wayland window loop alongside. Dropping the engine stops playback
//! and joins both threads cleanly.
//!
//! Transport (pause/resume/stop/seek), a position clock in the UI, and playlists arrive with
//! the interactivity phase; this is the smallest path from a file path to sound.

use std::fmt;
use std::path::Path;
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
    loop_thread: Option<JoinHandle<()>>,
    producer_thread: Option<JoinHandle<()>>,
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

        // Producer: prime with the first block, decode the rest into the ring, then wait for
        // the realtime side to drain it. `push_all`/the drain loop both stop early if the
        // consumer is dropped (engine drop -> loop thread gone), so this never hangs.
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
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        });

        Ok(Self {
            control,
            shared,
            loop_thread: Some(loop_thread),
            producer_thread: Some(producer_thread),
        })
    }

    /// Frames played so far. Basis for a future time display.
    pub fn position_frames(&self) -> u64 {
        self.shared.position_frames()
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
