//! Lock-free SPSC ring between the decode/producer thread and the real-time output
//! callback, plus the small block of atomics they share.
//!
//! The producer (heavy, non-realtime) writes decoded, channel-mapped, resampled interleaved
//! stereo f32 into the ring with [`push_block`]. The PipeWire realtime callback drains it
//! with [`fill_output`], which only copies and silence-pads: no allocation, no lock, no
//! syscall, so it is safe on the audio RT thread. The ring itself (rtrb) allocates once at
//! construction and never again.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

/// Interleaved stereo: two samples per frame. The whole engine works in this fixed layout.
pub const CHANNELS: usize = 2;

/// Counters shared between the app thread (reads position/state), the producer thread
/// (writes seek/flush) and the RT callback (writes `frames_consumed`). All lock-free.
#[derive(Debug)]
pub struct SharedState {
    /// Frames the RT callback has consumed. Written only by the callback (Relaxed).
    pub frames_consumed: AtomicU64,
    /// Playback position in frames at the last seek or track start. Producer writes it.
    pub seek_base: AtomicU64,
    /// `frames_consumed` snapshot captured at that seek, so the clock does not jump.
    pub consumed_base: AtomicU64,
    /// Producer -> RT: drop any queued audio on the next callback (seek/stop/track change).
    pub flush: AtomicBool,
    /// The graph rate PipeWire actually negotiated, published from `param_changed`.
    pub stream_rate: AtomicU32,
    /// Producer -> app: the track fully drained after a clean end of decode. Set once, so the
    /// UI can show the stopped state and a future playlist can advance to the next track.
    pub finished: AtomicBool,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            frames_consumed: AtomicU64::new(0),
            seek_base: AtomicU64::new(0),
            consumed_base: AtomicU64::new(0),
            flush: AtomicBool::new(false),
            stream_rate: AtomicU32::new(0),
            finished: AtomicBool::new(false),
        }
    }

    /// Playback position in frames: the seek base plus frames consumed since that seek.
    /// `saturating_sub` guards the brief window where the callback has not yet caught up to
    /// a freshly written `consumed_base`.
    pub fn position_frames(&self) -> u64 {
        let consumed = self.frames_consumed.load(Ordering::Relaxed);
        let base = self.consumed_base.load(Ordering::Relaxed);
        self.seek_base.load(Ordering::Relaxed) + consumed.saturating_sub(base)
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
/// shortfall, advance `consumed` by the real frames copied (before padding), and return that
/// same count. RT-safe: only atomic loads/stores and memcpy. Trailing silence after a track's
/// last frame copies nothing, so the clock stops there and the time display freezes at the true
/// end. The count is published *before* the ring slots are freed (`commit_all`), so the producer
/// draining the ring sees the final frames the instant it observes the ring empty; without that
/// order it could flag end-of-track with the last quantum still uncounted and the clock would
/// tick up afterward. If `flush` is set, drop all queued audio first (seek/stop/track change) and
/// emit silence for this quantum (counting and returning 0).
pub fn fill_output(
    c: &mut Consumer<f32>,
    out: &mut [f32],
    flush: &AtomicBool,
    consumed: &AtomicU64,
) -> usize {
    if flush.swap(false, Ordering::AcqRel) {
        // read_chunk(slots()) never errors (n <= readable) and does not allocate.
        if let Ok(chunk) = c.read_chunk(c.slots()) {
            chunk.commit_all();
        }
    }

    let avail = c.slots().min(out.len());
    let mut written = 0;
    if let Ok(chunk) = c.read_chunk(avail) {
        let (a, b) = chunk.as_slices();
        out[..a.len()].copy_from_slice(a);
        out[a.len()..a.len() + b.len()].copy_from_slice(b);
        written = a.len() + b.len();
        // Count the real frames before `commit_all` frees the ring: the producer's drain loop
        // waits on `slots()` (the read index that `commit_all` releases), so counting first gives
        // it a happens-before view of this final count the moment it sees the ring drained.
        // Relaxed is enough; the release/acquire on the ring's read index carries the store.
        consumed.fetch_add((written / CHANNELS) as u64, Ordering::Relaxed);
        chunk.commit_all();
    }
    for s in &mut out[written..] {
        *s = 0.0;
    }
    written / CHANNELS
}
