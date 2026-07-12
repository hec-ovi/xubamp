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

#[test]
#[ignore = "needs a running PipeWire session"]
fn pause_holds_the_clock_and_resume_advances_it() {
    let path = std::env::temp_dir().join("xubamp_pause_test.wav");
    write_wav(&path, 48_000, 4); // long enough not to finish during the test

    let engine = AudioEngine::play(&path).expect("engine failed to start playback");
    let handle = engine.handle();

    // Wait until more than one second has clearly played (48 kHz -> 60_000 frames is ~1.25 s),
    // so elapsed_secs has a non-zero, advancing value to check. Below one second every possible
    // implementation reads back 0, so an assertion there would prove nothing.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && engine.position_frames() <= 60_000 {
        std::thread::sleep(Duration::from_millis(50));
    }
    let frames = engine.position_frames();
    assert!(frames > 60_000, "playback did not reach one second (got {frames})");

    // The handle reads the same clock and rate (48 kHz) as the engine. Its read lands a hair
    // after `frames`, so it can only equal frames/rate or be one second more: it must have
    // advanced past a second, and it must match the position clock within that gap. This fails
    // a broken elapsed_secs (hardcoded 0, wrong base, or wrong divisor).
    let secs = handle.elapsed_secs();
    assert!(secs >= 1, "elapsed_secs did not advance past a second (got {secs})");
    assert!(
        secs.abs_diff((frames / 48_000) as u32) <= 1,
        "elapsed_secs {secs} does not match the position clock ({} s)",
        frames / 48_000
    );

    // Pause, let the deactivation reach the loop, then measure that the clock holds.
    handle.set_active(false);
    std::thread::sleep(Duration::from_millis(300));
    let a = engine.position_frames();
    std::thread::sleep(Duration::from_millis(400));
    let b = engine.position_frames();
    // If still playing at 48 kHz this window would advance ~19k frames; allow a small margin
    // for at most one in-flight quantum after the pause takes effect.
    assert!(
        b - a < 4_096,
        "paused clock kept advancing: {a} -> {b} ({} frames)",
        b - a
    );

    // Resume and confirm the clock moves again.
    handle.set_active(true);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && engine.position_frames() <= b + 8_000 {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        engine.position_frames() > b + 8_000,
        "resume did not advance the clock (held at {b}, now {})",
        engine.position_frames()
    );

    drop(engine);
    let _ = std::fs::remove_file(&path);
}

#[test]
#[ignore = "needs a running PipeWire session"]
fn seek_jumps_the_clock_and_reports_duration() {
    let path = std::env::temp_dir().join("xubamp_seek_test.wav");
    write_wav(&path, 48_000, 4); // four seconds, so a mid-track seek has somewhere to land

    let engine = AudioEngine::play(&path).expect("engine failed to start playback");
    let handle = engine.handle();

    // The WAV header gives an exact length.
    assert_eq!(handle.duration_secs(), Some(4), "duration read from the WAV header");

    // Let ~1 second play so there is a clear "before" position.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && engine.position_frames() <= 48_000 {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(engine.position_frames() > 48_000, "playback did not reach one second");

    // Seek to 75% (~3 s / 144_000 frames). The producer repositions and rebases the clock.
    handle.seek_fraction(0.75);
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && engine.position_frames() < 140_000 {
        std::thread::sleep(Duration::from_millis(20));
    }
    let after = engine.position_frames();
    assert!(
        (140_000..=176_000).contains(&after),
        "seek did not land near 3 s (got {after} frames)"
    );
    // The clock keeps advancing from the new spot rather than freezing.
    std::thread::sleep(Duration::from_millis(300));
    let now = engine.position_frames();
    assert!(
        now > after,
        "clock did not advance after the seek: after={after}, now={now}, finished={}",
        handle.is_finished()
    );

    // Seek-to-start (Stop's rewind) returns the clock near 0 even while playing.
    handle.seek_to_start();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && engine.position_frames() > 20_000 {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        engine.position_frames() < 20_000,
        "seek-to-start did not rewind the clock (got {} frames)",
        engine.position_frames()
    );

    drop(engine);
    let _ = std::fs::remove_file(&path);
}

#[test]
#[ignore = "needs a running PipeWire session"]
fn restarts_a_finished_track_from_the_start() {
    let path = std::env::temp_dir().join("xubamp_restart_test.wav");
    write_wav(&path, 48_000, 1); // one second, so it finishes quickly

    let engine = AudioEngine::play(&path).expect("engine failed to start playback");
    let handle = engine.handle();

    // Play it through to the end.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !handle.is_finished() {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(handle.is_finished(), "track never finished");
    assert!(engine.position_frames() >= 47_000, "clock did not reach the end");

    // Restart-on-play: seek to the start, then reactivate. The producer, parked at the end,
    // revives, clears `finished`, and refills from 0; the RT plays again from the top.
    handle.seek_to_start();
    handle.set_active(true);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && (handle.is_finished() || engine.position_frames() > 30_000) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!handle.is_finished(), "restart did not clear the finished flag");
    assert!(
        engine.position_frames() < 40_000,
        "restart did not rewind and replay from the start (got {} frames)",
        engine.position_frames()
    );

    // And it plays through to the end a second time.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !handle.is_finished() {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(handle.is_finished(), "restarted track did not play through again");

    drop(engine);
    let _ = std::fs::remove_file(&path);
}

#[test]
#[ignore = "needs a running PipeWire session"]
fn freezes_the_clock_at_end_of_track_and_reports_finished() {
    let path = std::env::temp_dir().join("xubamp_eos_test.wav");
    write_wav(&path, 48_000, 1); // exactly one second of audio

    let engine = AudioEngine::play(&path).expect("engine failed to start playback");
    let handle = engine.handle();

    // Wait for the track to drain to its end (the producer flags it after the ring empties).
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !handle.is_finished() {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(handle.is_finished(), "engine never reported end of track");

    // The clock froze at the true one-second length (48_000 frames), not past it from counted
    // silence. A broken implementation that counts silence quanta would keep climbing.
    let at_end = engine.position_frames();
    assert!(
        (47_000..=48_100).contains(&at_end),
        "clock did not freeze near the true one-second end (got {at_end})"
    );
    assert!(
        handle.elapsed_secs() <= 1,
        "elapsed_secs overran the one-second track (got {})",
        handle.elapsed_secs()
    );

    // The realtime thread emits many more silent quanta over this window; the frozen clock must
    // not move a single frame.
    std::thread::sleep(Duration::from_millis(500));
    let later = engine.position_frames();
    assert_eq!(
        later, at_end,
        "clock kept advancing after end of track: {at_end} -> {later}"
    );

    drop(engine);
    let _ = std::fs::remove_file(&path);
}
