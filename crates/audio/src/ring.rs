//! Lock-free SPSC ring between the decode/producer thread and the real-time output
//! callback, plus the small block of atomics they share.
//!
//! The producer (heavy, non-realtime) writes decoded, channel-mapped, resampled interleaved
//! stereo f32 into the ring with [`push_block`]. The PipeWire realtime callback drains it
//! with [`fill_output`], which only copies and silence-pads: no allocation, no lock, no
//! syscall, so it is safe on the audio RT thread. The ring itself (rtrb) allocates once at
//! construction and never again.

use std::sync::atomic::{
    AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering,
};

use rtrb::{Consumer, Producer, RingBuffer};

/// Interleaved stereo: two samples per frame. The whole engine works in this fixed layout.
pub const CHANNELS: usize = 2;

/// Length of the visualizer scope ring, a power of two so the write index masks cheaply. The RT
/// callback appends each quantum's downmixed-mono output here and the UI reads the most recent
/// window to draw the spectrum/oscilloscope. ~43 ms at 48 kHz: ample for a 512-point FFT and far
/// more than the UI reads between frames, so the reader never sees the writer lap it.
pub const SCOPE_LEN: usize = 2048;

/// Per-channel linear gains (left, right) for a volume and balance setting. Volume scales both
/// channels linearly (0..=100 -> 0.0..=1.0, matching classic Winamp's linear taper); balance
/// attenuates the *opposite* channel by `(100 - |balance|)/100` while the near channel stays
/// full. At volume 100, balance 0 this is `(1.0, 1.0)` (unity), so the realtime fast path can
/// skip scaling entirely.
pub fn mix_gains(volume: u8, balance: i8) -> (f32, f32) {
    let v = volume.min(100) as f32 / 100.0;
    let b = balance.clamp(-100, 100) as f32;
    let left = if b > 0.0 { (100.0 - b) / 100.0 } else { 1.0 };
    let right = if b < 0.0 { (100.0 + b) / 100.0 } else { 1.0 };
    (v * left, v * right)
}

/// Realtime side: scale an interleaved stereo buffer in place by per-channel gains. RT-safe (a
/// bounded multiply, no alloc/lock/syscall). Unity gains short-circuit, so full-volume centered
/// playback costs nothing. Scaling the trailing silence padding is harmless (`0 * g == 0`).
pub fn apply_gain(out: &mut [f32], gain_l: f32, gain_r: f32) {
    if gain_l == 1.0 && gain_r == 1.0 {
        return;
    }
    for frame in out.chunks_exact_mut(CHANNELS) {
        frame[0] *= gain_l;
        frame[1] *= gain_r;
    }
}

/// Counters shared between the app thread (reads position/state), the producer thread
/// (writes seek/flush) and the RT callback (writes `frames_consumed`). All lock-free.
#[derive(Debug)]
pub struct SharedState {
    /// Frames the RT callback has PLAYED (copied to the output). Drives the position clock, so
    /// dropped (skipped) frames deliberately do not count here. Written only by the callback.
    pub frames_consumed: AtomicU64,
    /// Frames the RT callback has REMOVED from the ring, whether played or dropped. This is the
    /// ring's read cursor in absolute frames; the callback compares it against [`Self::drop_before`]
    /// to know how many stale frames are still queued ahead of the fresh post-seek audio. Written
    /// only by the callback.
    pub removed_frames: AtomicU64,
    /// Position clock offset: `position = base_offset + frames_consumed`. It folds the seek target
    /// and the `frames_consumed` snapshot at that seek into a single value (`seek_target -
    /// consumed_at_seek`), so the producer publishes a rebase with ONE atomic store and the UI
    /// reads the clock without a two-variable invariant that could tear mid-seek. Signed because a
    /// backward seek makes it negative; the position is clamped at 0.
    pub base_offset: AtomicI64,
    /// Producer -> RT: absolute ring-read index (in frames) below which queued audio is STALE and
    /// must be dropped. A seek sets it to the frame count pushed up to the seek point, after the
    /// producer has already staged fresh audio behind that boundary, so the callback drops the
    /// stale tail and finds the fresh audio underneath in the same quantum: the ring never empties,
    /// so the stream never underruns (which some sinks, notably Bluetooth, suspend on). See
    /// [`SharedState::commit_seek`].
    pub drop_before: AtomicU64,
    /// The graph rate PipeWire actually negotiated, published from `param_changed`.
    pub stream_rate: AtomicU32,
    /// Producer -> app: the track fully drained after a clean end of decode. Set once, so the
    /// UI can show the stopped state and a future playlist can advance to the next track.
    pub finished: AtomicBool,
    /// Loop thread -> app: whether the output stream is currently active (playing) rather than
    /// paused/stopped. The visualizer animates from live audio only while this is set and settles
    /// to baseline otherwise; also a basis for a play indicator. Written on the loop thread.
    pub playing: AtomicBool,
    /// The current volume (0..=100) and balance (-100..=100), written by the UI thread. Kept so
    /// setting one recomputes the mix from both; not read on the RT path.
    pub volume: AtomicU32,
    pub balance: AtomicI32,
    /// Left/right realtime gains as `f32` bits, derived from `volume`/`balance` by `refresh_mix`
    /// and read by the RT callback. Default unity (1.0) so playback is full until the UI moves a
    /// slider.
    pub gain_l: AtomicU32,
    pub gain_r: AtomicU32,
    /// UI -> producer: a pending seek target in frames, or `-1` for none. The UI thread writes it
    /// (a newer request overwrites an unhandled older one, coalescing), the producer thread reads
    /// and clears it. Not on the RT path.
    pub seek_request: AtomicI64,
    /// RT -> UI: a [`SCOPE_LEN`] ring of the most recent downmixed-mono output samples (as `f32`
    /// bits) for the visualizer, `scope_write` the running write index. The RT appends each
    /// quantum (post-gain, so the scope shows what is heard); the UI reads the newest window. A
    /// torn read is invisible in a visualizer, so it is unsynchronised (Relaxed, no lock).
    pub scope: Box<[AtomicU32]>,
    pub scope_write: AtomicUsize,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            frames_consumed: AtomicU64::new(0),
            removed_frames: AtomicU64::new(0),
            base_offset: AtomicI64::new(0),
            drop_before: AtomicU64::new(0),
            stream_rate: AtomicU32::new(0),
            finished: AtomicBool::new(false),
            // Set true once the stream connects (it starts active); toggled by pause/resume.
            playing: AtomicBool::new(false),
            // Default to full volume, centered: unity gains, so playback is at full level before
            // the UI ever touches a slider (and it never opens silent).
            volume: AtomicU32::new(100),
            balance: AtomicI32::new(0),
            gain_l: AtomicU32::new(1.0f32.to_bits()),
            gain_r: AtomicU32::new(1.0f32.to_bits()),
            // No seek pending until the UI requests one.
            seek_request: AtomicI64::new(-1),
            // The scope ring starts silent; the single allocation here never repeats.
            scope: (0..SCOPE_LEN).map(|_| AtomicU32::new(0)).collect::<Vec<_>>().into_boxed_slice(),
            scope_write: AtomicUsize::new(0),
        }
    }

    /// RT: append this quantum's output (interleaved stereo, post-gain) to the scope ring as
    /// downmixed mono `(L + R) / 2`. Wait-free and RT-safe: a bounded loop of Relaxed atomic
    /// stores, no allocation, lock, or syscall. Silence (a padded underrun) writes zeros, so the
    /// visualizer falls to the baseline when nothing plays.
    pub fn push_scope(&self, interleaved: &[f32]) {
        let mask = SCOPE_LEN - 1;
        let mut w = self.scope_write.load(Ordering::Relaxed);
        for frame in interleaved.chunks_exact(CHANNELS) {
            let mono = (frame[0] + frame[1]) * 0.5;
            self.scope[w & mask].store(mono.to_bits(), Ordering::Relaxed);
            w = w.wrapping_add(1);
        }
        self.scope_write.store(w, Ordering::Relaxed);
    }

    /// UI: copy the most recent `out.len()` scope samples into `out`, oldest first (so `out` reads
    /// left-to-right in time). Reads behind the RT's write index; a torn sample is harmless for a
    /// visualizer. `out` longer than [`SCOPE_LEN`] has its excess head zero-filled.
    pub fn read_scope(&self, out: &mut [f32]) {
        let mask = SCOPE_LEN - 1;
        let n = out.len().min(SCOPE_LEN);
        let w = self.scope_write.load(Ordering::Relaxed);
        let start = w.wrapping_sub(n);
        let head = out.len() - n;
        for o in out.iter_mut().take(head) {
            *o = 0.0;
        }
        for (i, o) in out.iter_mut().skip(head).enumerate() {
            *o = f32::from_bits(self.scope[start.wrapping_add(i) & mask].load(Ordering::Relaxed));
        }
    }

    /// UI -> producer: request a seek to `target_frames` from the start of the track. Coalescing:
    /// a newer request overwrites an unhandled older one, since only the latest target matters.
    /// The producer picks it up with [`take_seek`] between decode steps.
    pub fn request_seek(&self, target_frames: u64) {
        // Frame counts this large never occur (a track longer than the age of the universe), so
        // the i64 cast cannot collide with the -1 sentinel.
        self.seek_request.store(target_frames as i64, Ordering::Relaxed);
    }

    /// Producer: is a seek pending? A cheap non-consuming peek, so the push/drain loops can bail
    /// out to handle it without swallowing the request.
    pub fn has_seek(&self) -> bool {
        self.seek_request.load(Ordering::Relaxed) >= 0
    }

    /// Producer: take the pending seek target in frames, clearing it, or `None` if none is set.
    pub fn take_seek(&self) -> Option<u64> {
        let v = self.seek_request.swap(-1, Ordering::Relaxed);
        (v >= 0).then_some(v as u64)
    }

    /// Producer: commit a seek that has landed the decoder at `landed_frames` and staged fresh
    /// audio in the ring. `stale_boundary` is [`Self::removed_frames`] + the frames that were
    /// already queued when the seek arrived (equivalently: the total frames pushed up to the seek
    /// point), so the callback drops exactly the pre-seek tail and starts playing the fresh audio
    /// staged behind it. Republishes the clock to the new spot and clears any end-of-track flag so
    /// a seek revives a finished track. Called only from the producer thread.
    ///
    /// Unlike a naive flush this never empties the ring: the producer stages the new-position audio
    /// BEFORE calling this, so when the callback drops the stale tail the fresh audio is already
    /// underneath it. A stream that underruns is suspended by some sinks (notably Bluetooth) and
    /// never resumes; keeping the ring non-empty across the drop is what makes the seek gapless AND
    /// safe.
    pub fn commit_seek(&self, landed_frames: u64, stale_boundary: u64) {
        // Rebase the clock in a single store so position = base_offset + frames_consumed reads back
        // `landed` at the first fresh frame and advances from there. Normally the callback drops the
        // remaining stale tail (not counted in frames_consumed) and the first fresh frame plays at
        // frames_consumed == consumed, so base_offset = landed - consumed. But if the buffer ran low
        // during the priming window and the callback already drained PAST the boundary, it has been
        // PLAYING the freshly-staged frames under the old clock; `removed_frames - stale_boundary`
        // is how many, and adding it keeps the clock from trailing the audio by that much.
        let consumed = self.frames_consumed.load(Ordering::Relaxed) as i64;
        let removed = self.removed_frames.load(Ordering::Relaxed);
        let overshoot = removed.saturating_sub(stale_boundary) as i64;
        self.base_offset
            .store(landed_frames as i64 - consumed + overshoot, Ordering::Relaxed);
        self.finished.store(false, Ordering::Release);
        // Publish the drop boundary last, with Release, so the fresh audio the producer just pushed
        // is visible to the callback before it acts on the new boundary (the callback loads it with
        // Acquire).
        self.drop_before.store(stale_boundary, Ordering::Release);
    }

    /// Producer: rebase the clock to `landed_frames` WITHOUT dropping the queued tail. Used as the
    /// safe fallback when a seek lands so close to the end of the track that too little fresh audio
    /// can be staged to keep the ring non-empty across a drop: rather than risk an underrun, the
    /// buffered tail plays out (a short stale tail, bounded by the ring latency) while the decoder
    /// refills. Clears the end-of-track flag so a seek still revives a finished track.
    pub fn rebase_clock_only(&self, landed_frames: u64) {
        let consumed = self.frames_consumed.load(Ordering::Relaxed) as i64;
        self.base_offset.store(landed_frames as i64 - consumed, Ordering::Relaxed);
        self.finished.store(false, Ordering::Release);
    }

    /// The realtime left/right gains most recently published by [`refresh_mix`].
    pub fn gains(&self) -> (f32, f32) {
        (
            f32::from_bits(self.gain_l.load(Ordering::Relaxed)),
            f32::from_bits(self.gain_r.load(Ordering::Relaxed)),
        )
    }

    /// Recompute the realtime gains from the current `volume` and `balance` and publish them for
    /// the RT callback. Called after the UI changes either value; both are only written from the
    /// single UI thread, so reading the pair here is race-free.
    pub fn refresh_mix(&self) {
        let volume = self.volume.load(Ordering::Relaxed).min(100) as u8;
        let balance = self.balance.load(Ordering::Relaxed).clamp(-100, 100) as i8;
        let (gl, gr) = mix_gains(volume, balance);
        self.gain_l.store(gl.to_bits(), Ordering::Relaxed);
        self.gain_r.store(gr.to_bits(), Ordering::Relaxed);
    }

    /// Playback position in frames: `base_offset + frames_consumed`, clamped at 0. Both are single
    /// atomic loads, so there is no multi-variable tear; `max(0)` covers the moment right after a
    /// backward seek before the callback has advanced `frames_consumed` to the new base.
    pub fn position_frames(&self) -> u64 {
        let consumed = self.frames_consumed.load(Ordering::Relaxed) as i64;
        (self.base_offset.load(Ordering::Relaxed) + consumed).max(0) as u64
    }

    /// Whether the track has played to its end (the producer sets `finished` once the ring has
    /// drained after a clean end of decode).
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

impl Default for SharedState {
    fn default() -> Self {
        Self::new()
    }
}

/// Allocate the ring, sized for `capacity_frames` interleaved stereo frames. This is the
/// only allocation; neither half allocates afterwards.
pub fn new_ring(capacity_frames: usize) -> (Producer<f32>, Consumer<f32>) {
    RingBuffer::<f32>::new(capacity_frames * CHANNELS)
}

/// Producer side (decode thread, not realtime): copy as much of `block` into the ring as
/// there is room for, returning the number of samples accepted. The caller retries the
/// remainder once the RT side has drained some. Never blocks.
pub fn push_block(p: &mut Producer<f32>, block: &[f32]) -> usize {
    let n = p.slots().min(block.len());
    if n == 0 {
        return 0;
    }
    // f32 is Copy + Default, so the (pre-zeroing) write_chunk cannot fail for n <= slots().
    match p.write_chunk(n) {
        Ok(mut chunk) => {
            let (a, b) = chunk.as_mut_slices();
            a.copy_from_slice(&block[..a.len()]);
            b.copy_from_slice(&block[a.len()..n]);
            chunk.commit_all();
            n
        }
        Err(_) => 0,
    }
}

/// Producer side: push the whole of `block` into the ring, retrying the remainder while the
/// realtime side drains, with a short sleep when it is full. Returns `false` if the consumer
/// was dropped (the output loop exited), so the caller can stop instead of spinning forever.
/// Not realtime: the sleep is a syscall, so this only runs on the producer thread.
pub fn push_all(p: &mut Producer<f32>, mut block: &[f32]) -> bool {
    while !block.is_empty() {
        let n = push_block(p, block);
        if n == 0 {
            if p.is_abandoned() {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(3));
        } else {
            block = &block[n..];
        }
    }
    true
}

/// Realtime side (PipeWire callback): fill `out` with queued audio, silence-padding any
/// shortfall, advance the clock by the real frames PLAYED (before padding), and return that
/// count. RT-safe: only atomic loads/stores and memcpy. Trailing silence after a track's last
/// frame copies nothing, so the clock stops there and the time display freezes at the true end.
///
/// Before reading, it drops any STALE tail a seek left queued: `shared.drop_before` is the ring
/// read index below which frames are pre-seek audio. Because the producer stages the fresh audio
/// BEHIND the tail before publishing the boundary, dropping the tail reveals the fresh audio in the
/// same quantum, so the ring never empties and the stream never underruns (which some sinks
/// suspend on). Dropped frames advance `removed_frames` (the read cursor) but NOT `frames_consumed`
/// (the clock), so the skipped tail does not tick the time display.
///
/// The played-frame count is published *before* the ring slots are freed (`commit_all`), so the
/// producer draining the ring sees the final frames the instant it observes the ring empty; without
/// that order it could flag end-of-track with the last quantum still uncounted and the clock would
/// tick up afterward.
pub fn fill_output(c: &mut Consumer<f32>, out: &mut [f32], shared: &SharedState) -> usize {
    // 1. Drop the stale pre-seek tail, if any. `drop_before` (frames) minus the read cursor is how
    //    much stale audio is still queued ahead of the fresh audio.
    let removed = shared.removed_frames.load(Ordering::Relaxed);
    let drop_before = shared.drop_before.load(Ordering::Acquire);
    let to_drop = drop_before.saturating_sub(removed) as usize;
    if to_drop > 0 {
        let drop_samples = (to_drop * CHANNELS).min(c.slots());
        if drop_samples > 0 {
            if let Ok(chunk) = c.read_chunk(drop_samples) {
                chunk.commit_all();
            }
            shared
                .removed_frames
                .store(removed + (drop_samples / CHANNELS) as u64, Ordering::Relaxed);
        }
    }

    // 2. Play the fresh audio (the only audio left after the drop).
    let avail = c.slots().min(out.len());
    let mut written = 0;
    if let Ok(chunk) = c.read_chunk(avail) {
        let (a, b) = chunk.as_slices();
        out[..a.len()].copy_from_slice(a);
        out[a.len()..a.len() + b.len()].copy_from_slice(b);
        written = a.len() + b.len();
        let played = (written / CHANNELS) as u64;
        // Count played frames before `commit_all` frees the ring: the producer's drain loop waits
        // on `slots()` (the read index `commit_all` releases), so counting first gives it a
        // happens-before view of this final count the moment it sees the ring drained. Relaxed is
        // enough; the release/acquire on the ring's read index carries the store.
        shared.frames_consumed.fetch_add(played, Ordering::Relaxed);
        shared
            .removed_frames
            .fetch_add(played, Ordering::Relaxed);
        chunk.commit_all();
    }
    for s in &mut out[written..] {
        *s = 0.0;
    }
    written / CHANNELS
}
