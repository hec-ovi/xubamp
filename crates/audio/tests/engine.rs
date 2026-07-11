//! End-to-end test of the audio engine: generate a small WAV, play it through the real
//! PipeWire output via `AudioEngine`, and assert the position clock advances (the realtime
//! callback consumed frames). Ignored by default because it needs a running PipeWire session.
//! In the dev container, route it to a silent null sink so it makes no noise:
//!   cargo test -p xubamp-audio --features output --test engine -- --ignored --nocapture
//! (set PIPEWIRE_NODE=<null-sink> first to stay silent; otherwise it plays a short 440 Hz tone
//! to the default sink).
#![cfg(feature = "output")]

use std::path::Path;
use std::time::{Duration, Instant};

use xubamp_audio::engine::AudioEngine;

/// Write a dependency-free 16-bit PCM stereo WAV of a 440 Hz sine.
fn write_wav(path: &Path, rate: u32, seconds: u32) {
    let channels: u16 = 2;
    let bits: u16 = 16;
    let frames = rate * seconds;
    let data_len = frames * channels as u32 * (bits / 8) as u32;
    let byte_rate = rate * channels as u32 * (bits / 8) as u32;
    let block_align = channels * (bits / 8);

    let mut buf = Vec::with_capacity(44 + data_len as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_len).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&bits.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());

    let step = std::f64::consts::TAU * 440.0 / rate as f64;
    for i in 0..frames {
        let s = ((i as f64 * step).sin() * 0.2 * 32767.0) as i16;
        buf.extend_from_slice(&s.to_le_bytes()); // L
        buf.extend_from_slice(&s.to_le_bytes()); // R
    }
    std::fs::write(path, buf).expect("write wav");
}

#[test]
#[ignore = "needs a running PipeWire session"]
fn plays_a_file_and_advances_the_clock() {
    let path = std::env::temp_dir().join("xubamp_engine_test.wav");
    write_wav(&path, 48_000, 1);

    let engine = AudioEngine::play(&path).expect("engine failed to start playback");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut frames = 0;
    while Instant::now() < deadline {
        frames = engine.position_frames();
        if frames > 4_096 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(
        frames > 4_096,
        "engine did not advance the position clock (got {frames})"
    );

    drop(engine); // clean shutdown: quit the loop, join both threads
    let _ = std::fs::remove_file(&path);
}
