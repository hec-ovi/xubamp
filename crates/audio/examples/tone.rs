//! Sub-unit (c) live check: generate a 440 Hz sine on the main thread, push it through the
//! SPSC ring, and play it via PipeWire for a few seconds. Exercises the real producer -> ring
//! -> realtime-callback path the engine will use, with a fixed tone standing in for the
//! decoder that lands in the next sub-unit.
//!
//! Run it in the dev container (the host has no libpipewire dev deps):
//!   scripts/dev-docker.sh run -- cargo run -p xubamp-audio --features output --example tone
//! or, inside a shell that already has the build deps:
//!   cargo run -p xubamp-audio --features output --example tone
//!
//! Verify while it runs: `pw-top -b -n 5` shows an "xubamp" node with the ERR/xrun column at
//! 0. For a headless capture, route it at a null sink and record the monitor (see
//! tests/live_playback.rs).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use xubamp_audio::command::Control;
use xubamp_audio::output::{control_channel, run_loop, RtData};
use xubamp_audio::ring::{new_ring, push_block, SharedState, CHANNELS};

const RATE: u32 = 48_000;
const FREQ: f64 = 440.0;
const AMPLITUDE: f32 = 0.3;
const SECONDS: u64 = 3;
const BLOCK_FRAMES: usize = 1024;

fn main() {
    // ~0.5 s of headroom in the ring.
    let (mut producer, consumer) = new_ring(RATE as usize / 2);
    let shared = Arc::new(SharedState::new());
    let (control, rx) = control_channel();

    let rt = RtData {
        consumer,
        shared: Arc::clone(&shared),
    };
    let loop_thread = thread::spawn(move || {
        if let Err(e) = run_loop(rx, rt, RATE) {
            eprintln!("tone: PipeWire loop error: {e}");
        }
    });

    // Producer: keep the ring full of a continuous sine until the deadline.
    let step = std::f64::consts::TAU * FREQ / RATE as f64;
    let mut phase = 0.0f64;
    let mut block = vec![0.0f32; BLOCK_FRAMES * CHANNELS];
    let deadline = Instant::now() + Duration::from_secs(SECONDS);

    while Instant::now() < deadline {
        for frame in block.chunks_exact_mut(CHANNELS) {
            let s = (phase.sin() as f32) * AMPLITUDE;
            phase += step;
            if phase >= std::f64::consts::TAU {
                phase -= std::f64::consts::TAU;
            }
            frame[0] = s;
            frame[1] = s;
        }
        // Push the whole block, retrying the remainder while the RT side drains. Bound the
        // retry by the deadline so we terminate even if run_loop failed to connect (no daemon)
        // and the ring never drains, instead of spinning on push_block == 0 forever.
        let mut off = 0;
        while off < block.len() && Instant::now() < deadline {
            let n = push_block(&mut producer, &block[off..]);
            off += n;
            if n == 0 {
                thread::sleep(Duration::from_millis(2));
            }
        }
        if off < block.len() {
            break; // consumer stalled or loop thread gone: stop rather than spin
        }
    }

    let rate = shared.stream_rate.load(Ordering::Acquire);
    let frames = shared.frames_consumed.load(Ordering::Relaxed);
    println!("tone: negotiated rate {rate} Hz, {frames} frames played");

    let _ = control.send(Control::Quit);
    let _ = loop_thread.join();
}
