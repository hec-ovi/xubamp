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
    connect_at(RATE);
}

/// The engine has no resampler: it opens the stream at the file's own rate and counts on the graph
/// agreeing to it. A track that is not at the graph rate is the case that would break, so check an
/// off-rate stream connects and gets exactly the rate it asked for.
#[test]
#[ignore = "needs a running PipeWire session"]
fn an_off_graph_rate_stream_gets_the_rate_it_asked_for() {
    connect_at(22_050);
    connect_at(44_100);
}

/// A stream that is fed as fast as it drains must never emit silence. When the realtime callback
/// fills PipeWire's whole mapped buffer instead of the frames the graph asked for, it empties the
/// ring into one cycle and pads the remainder, so every buffer queued to the sink ends in a gap.
/// A 22.05 kHz stream is where that bites: half a second of audio is 11025 frames against a
/// 12288-frame mapping, so the shortfall repeats forever instead of being a startup transient.
#[test]
#[ignore = "needs a running PipeWire session"]
fn a_low_rate_stream_never_pads_the_output_with_silence() {
    let rate = 22_050u32;
    // Half a second of audio, the engine's steady-state buffer. At this rate that is 11025 frames,
    // fewer than the 12288 the mapped buffer holds, which is the whole point: a callback that
    // fills the mapping cannot be satisfied from it and pads the difference every cycle.
    let (mut producer, consumer) = new_ring(rate as usize / 2);
    let shared = Arc::new(SharedState::new());
    let (control, rx) = control_channel();
    let rt = RtData {
        consumer,
        shared: Arc::clone(&shared),
    };
    let loop_thread = thread::spawn(move || run_loop(rx, rt, rate));

    // Keep the ring as full as it will go for two seconds, so any shortfall in frames played is
    // the output path's doing and not a starved producer.
    let block = vec![0.0f32; 1024 * CHANNELS];
    let started = Instant::now();
    let run_for = Duration::from_secs(2);
    while started.elapsed() < run_for {
        if push_block(&mut producer, &block) == 0 {
            thread::sleep(Duration::from_millis(2));
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    let frames = shared.frames_consumed.load(Ordering::Relaxed);
    let padded = shared.padded_frames.load(Ordering::Relaxed);
    let _ = control.send(Control::Quit);
    loop_thread
        .join()
        .expect("loop thread panicked")
        .expect("run_loop returned an error");

    assert!(frames > 0, "the stream never played anything");
    // A couple of quanta of slack for the very first cycles, before the producer has filled the
    // ring; a callback sized off the mapping instead pads on the order of a thousand frames per
    // cycle, tens of thousands over two seconds.
    assert!(
        padded < 4_096,
        "{padded} frames of silence padded into {frames} played over {elapsed:.2}s at {rate} Hz: \
         the callback is writing more than the graph asked for and filling the rest with zeros"
    );
}

/// Connect a stream at `rate`, feed it a quiet sine, and return once the graph has negotiated a
/// format and the realtime callback has pulled a few quanta (or the deadline passes).
fn connect_at(rate: u32) {
    let (mut producer, consumer) = new_ring(rate as usize / 2);
    let shared = Arc::new(SharedState::new());
    let (control, rx) = control_channel();

    let rt = RtData {
        consumer,
        shared: Arc::clone(&shared),
    };
    let loop_thread = thread::spawn(move || run_loop(rx, rt, rate));

    // Keep the ring fed with a quiet sine so the RT callback always has data to pull.
    let step = std::f64::consts::TAU * 440.0 / rate as f64;
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

    assert_eq!(
        negotiated_rate, rate,
        "the graph did not agree to the rate the stream offered, and nothing resamples"
    );
    assert!(
        frames > 4_096,
        "RT process callback did not consume frames (got {frames})"
    );
}
