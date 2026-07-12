//! SPSC ring correctness: in-order round-trip, wrap-around, underrun silence, the gapless-seek
//! drop-reveals-fresh-audio path, and the volume/balance gain stage.

use std::sync::atomic::Ordering;

use xubamp_audio::ring::{
    apply_gain, fill_output, mix_gains, new_ring, push_block, SharedState, SCOPE_LEN,
};

#[test]
fn round_trips_samples_in_order() {
    let (mut p, mut c) = new_ring(8); // 16 slots
    let s = SharedState::new();
    let block: Vec<f32> = (0..12).map(|i| i as f32).collect();
    assert_eq!(push_block(&mut p, &block), 12);
    let mut out = [0.0f32; 12];
    fill_output(&mut c, &mut out, &s);
    assert_eq!(out, [0., 1., 2., 3., 4., 5., 6., 7., 8., 9., 10., 11.]);
}

#[test]
fn push_block_reports_partial_accept_when_nearly_full() {
    let (mut p, mut c) = new_ring(4); // 8 slots
    let s = SharedState::new();
    let ten: Vec<f32> = (0..10).map(|i| i as f32).collect();
    // Only 8 slots exist, so only 8 samples are accepted.
    assert_eq!(push_block(&mut p, &ten), 8);
    let mut out = [0.0f32; 8];
    fill_output(&mut c, &mut out, &s);
    assert_eq!(out, [0., 1., 2., 3., 4., 5., 6., 7.]);
}

#[test]
fn underrun_pads_with_silence() {
    let (mut p, mut c) = new_ring(8);
    let s = SharedState::new();
    push_block(&mut p, &[1.0, 2.0, 3.0, 4.0]);
    let mut out = [9.9f32; 8];
    fill_output(&mut c, &mut out, &s);
    assert_eq!(&out[..4], &[1.0, 2.0, 3.0, 4.0]);
    assert_eq!(&out[4..], &[0.0; 4], "shortfall is silence, not stale data");
}

#[test]
fn wraps_around_the_physical_end() {
    let (mut p, mut c) = new_ring(4); // 8 slots
    let s = SharedState::new();
    let first: Vec<f32> = (0..8).map(|i| i as f32).collect();
    assert_eq!(push_block(&mut p, &first), 8);
    let mut out = [0.0f32; 6];
    fill_output(&mut c, &mut out, &s); // consume 6, leaving 6,7 near the end
    assert_eq!(out, [0., 1., 2., 3., 4., 5.]);
    let more: Vec<f32> = (100..104).map(|i| i as f32).collect();
    assert_eq!(push_block(&mut p, &more), 4); // write wraps past the end
    let mut out2 = [0.0f32; 6];
    fill_output(&mut c, &mut out2, &s);
    assert_eq!(out2, [6., 7., 100., 101., 102., 103.]);
}

#[test]
fn drop_before_skips_the_stale_tail_and_reveals_fresh_audio() {
    // The gapless-seek path: stale (pre-seek) audio is queued, the producer stages FRESH audio
    // behind it, then publishes the drop boundary. fill_output must drop exactly the stale frames
    // and play the fresh, and the ring must never go empty across the drop (fresh sits underneath
    // the tail) so the stream cannot underrun.
    let (mut p, mut c) = new_ring(8); // 16 slots = 8 stereo frames
    let s = SharedState::new();
    push_block(&mut p, &[1., 2., 3., 4., 5., 6.]); // 3 stale frames
    push_block(&mut p, &[100., 101., 102., 103., 104., 105.]); // 3 fresh frames staged behind
    // The producer pushed 3 frames before the seek point, so the drop boundary is 3 (in frames).
    s.drop_before.store(3, Ordering::Release);

    let mut out = [9.9f32; 6]; // one quantum = the 3 fresh frames
    let played = fill_output(&mut c, &mut out, &s);
    assert_eq!(played, 3, "only the fresh frames are played");
    assert_eq!(out, [100., 101., 102., 103., 104., 105.], "stale skipped, fresh revealed");
    assert_eq!(s.frames_consumed.load(Ordering::Relaxed), 3, "clock counts only played frames");
    assert_eq!(
        s.removed_frames.load(Ordering::Relaxed),
        6,
        "read cursor advanced past the 3 dropped + 3 played",
    );
    assert_eq!(p.slots(), 16, "ring drained; the fresh audio was never left starved");
    // The boundary is now behind the cursor, so a following quantum drops nothing.
    let mut out2 = [7.7f32; 2];
    assert_eq!(fill_output(&mut c, &mut out2, &s), 0, "nothing left, no spurious drop");
    assert_eq!(out2, [0.0, 0.0], "empty read is silence");
}

#[test]
fn drop_reveals_fresh_audio_across_a_full_ring() {
    // A seek when the ring is essentially full of stale: the boundary drops the whole stale block
    // in one quantum and the fresh audio staged behind it plays with no gap (a desync here would
    // empty the ring and freeze playback on a sink that suspends on underrun).
    let (mut p, mut c) = new_ring(4); // 8 slots = 4 stereo frames
    let s = SharedState::new();
    push_block(&mut p, &[1., 2., 3., 4.]); // 2 stale frames
    push_block(&mut p, &[10., 11., 12., 13.]); // 2 fresh frames -> ring full
    assert_eq!(p.slots(), 0, "ring is completely full");
    s.drop_before.store(2, Ordering::Release); // 2 stale frames pushed before the seek

    let mut out = [0.0f32; 4]; // request the 2 fresh frames
    let played = fill_output(&mut c, &mut out, &s);
    assert_eq!(played, 2, "the 2 fresh frames play");
    assert_eq!(out, [10., 11., 12., 13.], "stale dropped, fresh revealed from a full ring");
    assert_eq!(s.frames_consumed.load(Ordering::Relaxed), 2);
    assert_eq!(s.removed_frames.load(Ordering::Relaxed), 4, "2 dropped + 2 played");
}

#[test]
fn no_drop_once_the_boundary_is_behind_the_read_cursor() {
    // A stale drop boundary that the read cursor already passed must not eat fresh audio.
    let (mut p, mut c) = new_ring(8);
    let s = SharedState::new();
    push_block(&mut p, &[1., 2., 3., 4.]); // 2 frames
    s.removed_frames.store(5, Ordering::Relaxed); // cursor already past
    s.drop_before.store(3, Ordering::Release); // an old boundary
    let mut out = [0.0f32; 4];
    let played = fill_output(&mut c, &mut out, &s);
    assert_eq!(played, 2, "plays normally, drops nothing");
    assert_eq!(out, [1., 2., 3., 4.]);
    assert_eq!(s.removed_frames.load(Ordering::Relaxed), 7, "5 + 2 played");
    assert_eq!(s.frames_consumed.load(Ordering::Relaxed), 2, "all real frames counted");
}

#[test]
fn fill_output_counts_and_returns_real_audio_frames_only() {
    let (mut p, mut c) = new_ring(8); // 16 slots, 8 stereo frames
    let s = SharedState::new();
    // Four stereo frames (8 samples) into an eight-frame (16-sample) request: only the four real
    // frames are counted and returned; the silence padding is neither, so the clock advances by
    // played audio alone.
    push_block(&mut p, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    let mut out = [0.0f32; 16];
    assert_eq!(fill_output(&mut c, &mut out, &s), 4);
    assert_eq!(s.frames_consumed.load(Ordering::Relaxed), 4, "counts only the real frames");
    // Draining an empty ring counts and returns zero: this freezes the clock at a track's end
    // instead of counting trailing silence.
    let mut out2 = [0.0f32; 16];
    assert_eq!(fill_output(&mut c, &mut out2, &s), 0);
    assert_eq!(s.frames_consumed.load(Ordering::Relaxed), 4, "an empty read adds nothing");
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
fn commit_seek_rebases_the_clock_sets_the_drop_boundary_and_revives() {
    let s = SharedState::new();
    s.frames_consumed.store(5_000, Ordering::Relaxed);
    s.finished.store(true, Ordering::Release);

    // A seek that landed the decoder at frame 30_000, with 12_000 frames pushed up to the seek
    // point (the stale boundary). The clock jumps to the target and the drop boundary is published.
    s.commit_seek(30_000, 12_000);
    assert!(!s.is_finished(), "a seek revives a finished track");
    assert_eq!(s.position_frames(), 30_000, "clock jumps to the seek target, no drift");
    assert_eq!(s.drop_before.load(Ordering::Acquire), 12_000, "drop boundary published");
    // The clock advances from the target as PLAYED frames accrue; the dropped stale tail does not
    // count (frames_consumed only ever counts played audio).
    s.frames_consumed.store(5_480, Ordering::Relaxed);
    assert_eq!(s.position_frames(), 30_480);
}

#[test]
fn commit_seek_corrects_the_clock_when_the_rt_over_drained_during_priming() {
    // If the buffer ran low while staging, the realtime side can drain PAST the boundary and play
    // some freshly-staged frames before commit_seek runs. commit_seek must count those so the clock
    // does not trail the audio.
    let s = SharedState::new();
    s.frames_consumed.store(200, Ordering::Relaxed);
    // Read cursor is 50 frames past the stale boundary: 50 fresh frames already played early.
    s.removed_frames.store(12_050, Ordering::Relaxed);
    s.commit_seek(30_000, 12_000);
    assert_eq!(
        s.position_frames(),
        30_050,
        "clock accounts for the 50 fresh frames the RT already played past the boundary",
    );
    // With no over-drain (cursor at or before the boundary), the correction is zero.
    let s2 = SharedState::new();
    s2.frames_consumed.store(200, Ordering::Relaxed);
    s2.removed_frames.store(11_000, Ordering::Relaxed); // still short of the boundary
    s2.commit_seek(30_000, 12_000);
    assert_eq!(s2.position_frames(), 30_000, "no correction when the tail was not over-drained");
}

#[test]
fn rebase_clock_only_moves_the_clock_without_touching_the_drop_boundary() {
    // The near-end-of-track fallback: rebase the clock but leave the tail to play out (no drop), so
    // the ring is never risked empty when too little fresh audio could be staged.
    let s = SharedState::new();
    s.frames_consumed.store(1_000, Ordering::Relaxed);
    s.drop_before.store(7, Ordering::Release); // some earlier boundary
    s.finished.store(true, Ordering::Release);
    s.rebase_clock_only(50_000);
    assert_eq!(s.position_frames(), 50_000, "clock rebased to the target");
    assert!(!s.is_finished(), "still revives a finished track");
    assert_eq!(s.drop_before.load(Ordering::Acquire), 7, "boundary untouched: the tail plays out");
}

#[test]
fn scope_tap_writes_mono_and_reads_the_latest_window() {
    let s = SharedState::new();
    // Fresh: the scope reads back silence.
    let mut out = [9.9f32; 8];
    s.read_scope(&mut out);
    assert_eq!(out, [0.0; 8], "an untouched scope is silent");
    // Push interleaved stereo; the tap stores per-frame mono (L+R)/2.
    s.push_scope(&[2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0]); // frames -> 3, 7, 11, 15
    let mut out4 = [0.0f32; 4];
    s.read_scope(&mut out4);
    assert_eq!(out4, [3.0, 7.0, 11.0, 15.0], "mono of the four frames, oldest first");
    // Reading fewer than written returns the most recent, oldest first.
    let mut out2 = [0.0f32; 2];
    s.read_scope(&mut out2);
    assert_eq!(out2, [11.0, 15.0], "the two most recent frames");
    // Reading more history than has been written returns the ring's silence before the samples.
    let mut out6 = [0.0f32; 6];
    s.read_scope(&mut out6);
    assert_eq!(out6, [0.0, 0.0, 3.0, 7.0, 11.0, 15.0], "unwritten history reads as silence");
}

#[test]
fn scope_tap_wraps_around_the_ring() {
    let s = SharedState::new();
    // Write more than SCOPE_LEN frames so the index wraps; only the last SCOPE_LEN survive.
    let total = SCOPE_LEN + 100;
    let block: Vec<f32> = (0..total).flat_map(|i| [i as f32, i as f32]).collect(); // L==R so mono==i
    s.push_scope(&block);
    let mut out = [0.0f32; 4];
    s.read_scope(&mut out);
    let base = (total - 4) as f32; // the last four frames
    assert_eq!(out, [base, base + 1.0, base + 2.0, base + 3.0], "reads the newest across a wrap");
}

#[test]
fn position_clock_tracks_consumption_and_seeks() {
    let s = SharedState::new();
    assert_eq!(s.position_frames(), 0);
    s.frames_consumed.store(1000, Ordering::Relaxed);
    assert_eq!(s.position_frames(), 1000);
    // Seek to frame 5000 with 1000 already consumed: the clock reports 5000 with no jump...
    s.commit_seek(5000, 0);
    assert_eq!(s.position_frames(), 5000, "no jump right after a seek");
    // ...then advances from the seek target as more frames play.
    s.frames_consumed.store(1250, Ordering::Relaxed);
    assert_eq!(s.position_frames(), 5250, "advances from the seek target");
    // A backward seek to 0 with 1250 consumed clamps at 0 (never negative) and climbs from there.
    s.commit_seek(0, 0);
    assert_eq!(s.position_frames(), 0, "backward seek rebases to the start");
    s.frames_consumed.store(1450, Ordering::Relaxed);
    assert_eq!(s.position_frames(), 200, "advances from 0 after the backward seek");
}
