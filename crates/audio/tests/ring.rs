//! SPSC ring correctness: in-order round-trip, wrap-around, underrun silence, flush.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use xubamp_audio::ring::{fill_output, new_ring, push_block, SharedState};

#[test]
fn round_trips_samples_in_order() {
    let (mut p, mut c) = new_ring(8); // 16 slots
    let flush = AtomicBool::new(false);
    let block: Vec<f32> = (0..12).map(|i| i as f32).collect();
    assert_eq!(push_block(&mut p, &block), 12);
    let mut out = [0.0f32; 12];
    fill_output(&mut c, &mut out, &flush, &AtomicU64::new(0));
    assert_eq!(out, [0., 1., 2., 3., 4., 5., 6., 7., 8., 9., 10., 11.]);
}

#[test]
fn push_block_reports_partial_accept_when_nearly_full() {
    let (mut p, mut c) = new_ring(4); // 8 slots
    let flush = AtomicBool::new(false);
    let ten: Vec<f32> = (0..10).map(|i| i as f32).collect();
    // Only 8 slots exist, so only 8 samples are accepted.
    assert_eq!(push_block(&mut p, &ten), 8);
    let mut out = [0.0f32; 8];
    fill_output(&mut c, &mut out, &flush, &AtomicU64::new(0));
    assert_eq!(out, [0., 1., 2., 3., 4., 5., 6., 7.]);
}

#[test]
fn underrun_pads_with_silence() {
    let (mut p, mut c) = new_ring(8);
    let flush = AtomicBool::new(false);
    push_block(&mut p, &[1.0, 2.0, 3.0, 4.0]);
    let mut out = [9.9f32; 8];
    fill_output(&mut c, &mut out, &flush, &AtomicU64::new(0));
    assert_eq!(&out[..4], &[1.0, 2.0, 3.0, 4.0]);
    assert_eq!(&out[4..], &[0.0; 4], "shortfall is silence, not stale data");
}

#[test]
fn wraps_around_the_physical_end() {
    let (mut p, mut c) = new_ring(4); // 8 slots
    let flush = AtomicBool::new(false);
    let first: Vec<f32> = (0..8).map(|i| i as f32).collect();
    assert_eq!(push_block(&mut p, &first), 8);
    let mut out = [0.0f32; 6];
    fill_output(&mut c, &mut out, &flush, &AtomicU64::new(0)); // consume 6, leaving 6,7 near the end
    assert_eq!(out, [0., 1., 2., 3., 4., 5.]);
    let more: Vec<f32> = (100..104).map(|i| i as f32).collect();
    assert_eq!(push_block(&mut p, &more), 4); // write wraps past the end
    let mut out2 = [0.0f32; 6];
    fill_output(&mut c, &mut out2, &flush, &AtomicU64::new(0));
    assert_eq!(out2, [6., 7., 100., 101., 102., 103.]);
}

#[test]
fn flush_drops_queued_audio_and_is_one_shot() {
    let (mut p, mut c) = new_ring(8);
    let flush = AtomicBool::new(false);
    push_block(&mut p, &[1.0, 2.0, 3.0, 4.0]);
    flush.store(true, Ordering::Release);
    let mut out = [5.5f32; 4];
    fill_output(&mut c, &mut out, &flush, &AtomicU64::new(0));
    assert_eq!(out, [0.0; 4], "flushed audio is dropped and replaced with silence");
    assert!(!flush.load(Ordering::Acquire), "flush clears itself");
    // Audio pushed after the flush plays normally.
    push_block(&mut p, &[7.0, 8.0]);
    let mut out2 = [0.0f32; 2];
    fill_output(&mut c, &mut out2, &flush, &AtomicU64::new(0));
    assert_eq!(out2, [7.0, 8.0]);
}

#[test]
fn fill_output_counts_and_returns_real_audio_frames_only() {
    let (mut p, mut c) = new_ring(8); // 16 slots, 8 stereo frames
    let flush = AtomicBool::new(false);
    let consumed = AtomicU64::new(0);
    // Four stereo frames (8 samples) into an eight-frame (16-sample) request: only the four real
    // frames are counted and returned; the silence padding is neither, so the clock advances by
    // played audio alone.
    push_block(&mut p, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    let mut out = [0.0f32; 16];
    assert_eq!(fill_output(&mut c, &mut out, &flush, &consumed), 4);
    assert_eq!(consumed.load(Ordering::Relaxed), 4, "counts only the real frames");
    // Draining an empty ring counts and returns zero: this freezes the clock at a track's end
    // instead of counting trailing silence.
    let mut out2 = [0.0f32; 16];
    assert_eq!(fill_output(&mut c, &mut out2, &flush, &consumed), 0);
    assert_eq!(consumed.load(Ordering::Relaxed), 4, "an empty read adds nothing");
}

#[test]
fn position_clock_tracks_consumption_and_seeks() {
    let s = SharedState::new();
    assert_eq!(s.position_frames(), 0);
    s.frames_consumed.store(1000, Ordering::Relaxed);
    assert_eq!(s.position_frames(), 1000);
    // Seek to frame 5000: base 5000, snapshot the current consumed count.
    s.seek_base.store(5000, Ordering::Relaxed);
    s.consumed_base.store(1000, Ordering::Relaxed);
    assert_eq!(s.position_frames(), 5000, "no jump right after a seek");
    s.frames_consumed.store(1250, Ordering::Relaxed);
    assert_eq!(s.position_frames(), 5250, "advances from the seek target");
}
