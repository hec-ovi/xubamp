//! Play a real audio file through the PipeWire output: decode + channel-map to stereo on a
//! producer thread, push into the SPSC ring, and stream at the file's native sample rate
//! (PipeWire converts to the device). This is the decode -> ring -> realtime-output path the
//! engine will use, exercised end to end with a real track instead of a synthetic tone.
//! Resampling to a fixed graph rate is a later sub-unit; here we just let the graph match the
//! file, which plays any rate correctly.
//!
//! Usage (dev container):
//!   scripts/dev-docker.sh run is for the binary; run the example directly, e.g.
//!   docker run ... xubamp-dev cargo run -p xubamp-audio --features output --example play -- <file>
//! or inside a shell with the build + PipeWire deps:
//!   cargo run -p xubamp-audio --features output --example play -- path/to/song.mp3

use std::env;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rtrb::Producer;

use xubamp_audio::channels::to_stereo;
use xubamp_audio::command::Control;
use xubamp_audio::decode::Source;
use xubamp_audio::output::{control_channel, run_loop, RtData};
use xubamp_audio::ring::{new_ring, push_block, SharedState, CHANNELS};

/// Push the whole buffer into the ring, retrying while the realtime side drains. Returns false
/// if the consumer was dropped (the loop thread exited), so the producer can stop instead of
/// spinning forever.
fn push_all(producer: &mut Producer<f32>, mut buf: &[f32]) -> bool {
    while !buf.is_empty() {
        let n = push_block(producer, buf);
        if n == 0 {
            if producer.is_abandoned() {
                return false;
            }
            thread::sleep(Duration::from_millis(3));
        } else {
            buf = &buf[n..];
        }
    }
    true
}

fn main() {
    let path = match env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: play <audio-file>");
            std::process::exit(2);
        }
    };

    let mut src = Source::open(Path::new(&path)).expect("failed to open/probe the file");

    // Decode the first block so the sample rate and channel count come from a real packet
    // (MP3 can report them as unknown until the first frame decodes). Copy it out so the
    // borrow of `src` ends and we can read its rate/channels; channels is constant per track,
    // so capture it once and reuse it for every later block.
    let first: Vec<f32> = match src.next_interleaved().expect("decode error") {
        Some(block) => block.to_vec(),
        None => {
            eprintln!("play: no audio frames in {path}");
            return;
        }
    };
    let rate = src.sample_rate;
    let channels = src.channels;
    println!("play: {path}\n  source {rate} Hz / {channels} ch -> stereo stream at {rate} Hz");

    let mut stereo: Vec<f32> = Vec::new();
    to_stereo(&first, channels, &mut stereo);

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
            eprintln!("play: PipeWire loop error: {e}");
        }
    });

    // Prime the ring with the first block, then decode the rest of the track into it.
    if push_all(&mut producer, &stereo) {
        loop {
            match src.next_interleaved() {
                Ok(Some(block)) => {
                    stereo.clear();
                    to_stereo(block, channels, &mut stereo);
                    if !push_all(&mut producer, &stereo) {
                        break; // loop thread gone
                    }
                }
                Ok(None) => break, // clean end of stream
                Err(e) => {
                    eprintln!("play: decode error: {e}");
                    break;
                }
            }
        }
    }

    // Wait for the realtime side to drain what we pushed, then let the last quantum flush.
    while producer.slots() < ring_slots {
        if producer.is_abandoned() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(80));

    let frames = shared.frames_consumed.load(Ordering::Relaxed);
    println!("play: done, {frames} frames played ({:.1} s)", frames as f64 / rate.max(1) as f64);

    let _ = control.send(Control::Quit);
    let _ = loop_thread.join();
}
