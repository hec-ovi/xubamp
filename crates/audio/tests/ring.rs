//! SPSC ring correctness: in-order round-trip, wrap-around, underrun silence, flush, and the
//! volume/balance gain stage.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use xubamp_audio::ring::{apply_gain, fill_output, mix_gains, new_ring, push_block, SharedState};

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
fn flush_drains_a_completely_full_ring_and_refills() {
    // The live seek path flushes a ring that is FULL (0.5 s buffered), unlike the partial-ring
    // flush above. Prove a full-ring flush actually empties it and that new audio pushed after is
    // readable (a desync here silently freezes playback after a seek).
    let (mut p, mut c) = new_ring(4); // 8 slots
    let flush = AtomicBool::new(false);
    let consumed = AtomicU64::new(0);
    assert_eq!(push_block(&mut p, &[1., 2., 3., 4., 5., 6., 7., 8.]), 8);
    assert_eq!(p.slots(), 0, "ring is completely full");

    flush.store(true, Ordering::Release);
    let mut out = [9.9f32; 4];
    fill_output(&mut c, &mut out, &flush, &consumed);
    assert_eq!(out, [0.0; 4], "flushed audio is dropped");
    assert_eq!(p.slots(), 8, "a full ring is fully drained by the flush");

    // New audio pushed after the flush must be visible to the reader.
    assert_eq!(push_block(&mut p, &[10., 11., 12., 13.]), 4);
    let mut out2 = [0.0f32; 4];
    fill_output(&mut c, &mut out2, &flush, &consumed);
    assert_eq!(out2, [10., 11., 12., 13.], "new audio reads back after a full-ring flush");
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
fn mix_gains_maps_volume_and_balance() {
    // Full volume, centered: unity on both channels.
    assert_eq!(mix_gains(100, 0), (1.0, 1.0));
    // Half volume, centered: both channels halved.
    assert_eq!(mix_gains(50, 0), (0.5, 0.5));
    // Volume 0: silence regardless of balance.
    assert_eq!(mix_gains(0, 0), (0.0, 0.0));
    // Balance panned fully right at full volume: left channel silenced, right full.
    assert_eq!(mix_gains(100, 100), (0.0, 1.0));
    // Panned fully left: right silenced.
    assert_eq!(mix_gains(100, -100), (1.0, 0.0));
    // Balance attenuates the opposite channel proportionally; near channel stays full.
    let (l, r) = mix_gains(100, 50); // panned right
    assert_eq!((l, r), (0.5, 1.0));
    // Volume and balance compound.
    assert_eq!(mix_gains(50, -100), (0.5, 0.0));
}

#[test]
fn apply_gain_scales_each_channel_and_short_circuits_unity() {
    // Interleaved L,R,L,R.
    let mut buf = [1.0f32, 1.0, 1.0, 1.0];
    apply_gain(&mut buf, 0.5, 0.25);
    assert_eq!(buf, [0.5, 0.25, 0.5, 0.25]);
    // Unity leaves the buffer untouched (the RT fast path).
    let mut buf2 = [0.3f32, -0.7, 0.9, -0.1];
    let before = buf2;
    apply_gain(&mut buf2, 1.0, 1.0);
    assert_eq!(buf2, before);
    // The common runtime case: full volume with a non-zero balance leaves exactly ONE channel at
    // unity (e.g. mix_gains(100, 50) = (0.5, 1.0)). This pins the short-circuit to `&&`: a `||`
    // would wrongly skip scaling here and leave balance inert at full volume.
    let mut pan_right = [1.0f32, 1.0, 1.0, 1.0];
    apply_gain(&mut pan_right, 0.5, 1.0);
    assert_eq!(pan_right, [0.5, 1.0, 0.5, 1.0], "left halved, right untouched");
    let mut pan_left = [1.0f32, 1.0, 1.0, 1.0];
    apply_gain(&mut pan_left, 1.0, 0.5);
    assert_eq!(pan_left, [1.0, 0.5, 1.0, 0.5], "right halved, left untouched");
}

#[test]
fn refresh_mix_publishes_gains_from_volume_and_balance() {
    let s = SharedState::new();
    // Defaults: full volume, centered -> unity gains.
    assert_eq!(s.gains(), (1.0, 1.0));
    // Set half volume, then refresh: both channels halved.
    s.volume.store(50, Ordering::Relaxed);
    s.refresh_mix();
    assert_eq!(s.gains(), (0.5, 0.5));
    // Pan fully left at half volume: right channel silenced, left half.
    s.balance.store(-100, Ordering::Relaxed);
    s.refresh_mix();
    assert_eq!(s.gains(), (0.5, 0.0));
}

#[test]
fn seek_request_round_trips_and_coalesces() {
    let s = SharedState::new();
    assert!(!s.has_seek(), "no seek pending on a fresh state");
    assert_eq!(s.take_seek(), None);
    s.request_seek(1000);
    assert!(s.has_seek());
    // A newer request overwrites an unhandled one: only the latest target matters.
    s.request_seek(2000);
    assert_eq!(s.take_seek(), Some(2000), "coalesces to the latest request");
    assert!(!s.has_seek(), "taking clears it");
    assert_eq!(s.take_seek(), None, "nothing left to take");
    // Seeking to frame 0 is a real request, distinct from the -1 "none" sentinel.
    s.request_seek(0);
    assert!(s.has_seek());
    assert_eq!(s.take_seek(), Some(0));
}

#[test]
fn begin_seek_rebases_the_clock_and_clears_finished() {
    let s = SharedState::new();
    s.frames_consumed.store(5_000, Ordering::Relaxed);
    s.finished.store(true, Ordering::Release);

    // Seeking rebases the clock to the landed frame and revives a finished track. It does NOT
    // raise the flush: the queued audio plays out (dropping it would underrun the stream, which
    // some sinks suspend on), so the seek carries a short tail before the new position.
    s.begin_seek(30_000);
    assert!(!s.flush.load(Ordering::Acquire), "the ring is not flushed on a seek");
    assert!(!s.is_finished(), "a seek revives a finished track");
    assert_eq!(s.position_frames(), 30_000, "clock jumps to the seek target, no drift");
    // The clock then advances from the target as the RT consumes new-position frames.
    s.frames_consumed.store(5_480, Ordering::Relaxed);
    assert_eq!(s.position_frames(), 30_480);

    // Seeking to the start rebases to 0 (base_offset is recomputed from the current consumed).
    s.begin_seek(0);
    assert_eq!(s.position_frames(), 0, "rebased to the start");
    s.frames_consumed.store(5_960, Ordering::Relaxed);
    assert_eq!(s.position_frames(), 480, "advances from 0 as new-position frames play");
}

#[test]
fn position_clock_tracks_consumption_and_seeks() {
    let s = SharedState::new();
    assert_eq!(s.position_frames(), 0);
    s.frames_consumed.store(1000, Ordering::Relaxed);
    assert_eq!(s.position_frames(), 1000);
    // Seek to frame 5000 with 1000 already consumed: the clock reports 5000 with no jump...
    s.begin_seek(5000);
    assert_eq!(s.position_frames(), 5000, "no jump right after a seek");
    // ...then advances from the seek target as more frames play.
    s.frames_consumed.store(1250, Ordering::Relaxed);
    assert_eq!(s.position_frames(), 5250, "advances from the seek target");
    // A backward seek to 0 with 1250 consumed clamps at 0 (never negative) and climbs from there.
    s.begin_seek(0);
    assert_eq!(s.position_frames(), 0, "backward seek rebases to the start");
    s.frames_consumed.store(1450, Ordering::Relaxed);
    assert_eq!(s.position_frames(), 200, "advances from 0 after the backward seek");
}
