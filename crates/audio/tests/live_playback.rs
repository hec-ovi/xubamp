//! Live end-to-end check of the PipeWire output path against a real user session. Ignored by
//! default because it needs a running PipeWire daemon; run it in the dev container with:
//!   cargo test -p xubamp-audio --features output --test live_playback -- --ignored --nocapture
//!
//! It drives the real entry point (`output::run_loop` -> `pw_stream_connect` -> the graph),
//! not a mock: a successful negotiation fires `param_changed` (proving the stream connected
//! and a format was agreed) and the RT `process` callback advances `frames_consumed` (proving
//! buffers actually flowed). No audio capture is needed, so it works headless over SSH.
#![cfg(feature = "output")]

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use xubamp_audio::command::Control;
use xubamp_audio::output::{control_channel, run_loop, RtData};
use xubamp_audio::ring::{new_ring, push_block, SharedState, CHANNELS};

const RATE: u32 = 48_000;

#[test]
#[ignore = "needs a running PipeWire session"]
fn connects_negotiates_a_rate_and_consumes_frames() {
    let (mut producer, consumer) = new_ring(RATE as usize / 2);
    let shared = Arc::new(SharedState::new());
    let (control, rx) = control_channel();

    let rt = RtData {
        consumer,
        shared: Arc::clone(&shared),
    };
    let loop_thread = thread::spawn(move || run_loop(rx, rt, RATE));

    // Keep the ring fed with a quiet sine so the RT callback always has data to pull.
    let step = std::f64::consts::TAU * 440.0 / RATE as f64;
    let mut phase = 0.0f64;
    let mut block = vec![0.0f32; 1024 * CHANNELS];

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut negotiated_rate = 0u32;
    while Instant::now() < deadline {
        for frame in block.chunks_exact_mut(CHANNELS) {
            let s = (phase.sin() as f32) * 0.1;
            phase += step;
            if phase >= std::f64::consts::TAU {
                phase -= std::f64::consts::TAU;
            }
            frame[0] = s;
            frame[1] = s;
        }
        // Bound the retry by the deadline: if the stream never connects (run_loop errored,
        // ring never drains) this must fail via the asserts below, not hang forever.
        let mut off = 0;
        while off < block.len() && Instant::now() < deadline {
            let n = push_block(&mut producer, &block[off..]);
            off += n;
            if n == 0 {
                thread::sleep(Duration::from_millis(2));
            }
        }
        if off < block.len() {
            break;
        }
        negotiated_rate = shared.stream_rate.load(Ordering::Acquire);
        // Stop once the graph has negotiated a rate and the RT callback has run a few quanta.
        if negotiated_rate != 0 && shared.frames_consumed.load(Ordering::Relaxed) > 4096 {
            break;
        }
    }

    let frames = shared.frames_consumed.load(Ordering::Relaxed);
    let _ = control.send(Control::Quit);
    let joined = loop_thread.join().expect("loop thread panicked");
    joined.expect("run_loop returned an error");

    assert!(
        negotiated_rate >= 8_000,
        "graph never negotiated a plausible rate (got {negotiated_rate})"
    );
    assert!(
        frames > 4_096,
        "RT process callback did not consume frames (got {frames})"
    );
}
