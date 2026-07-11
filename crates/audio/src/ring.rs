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
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            frames_consumed: AtomicU64::new(0),
            seek_base: AtomicU64::new(0),
            consumed_base: AtomicU64::new(0),
            flush: AtomicBool::new(false),
            stream_rate: AtomicU32::new(0),
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

/// Realtime side (PipeWire callback): fill `out` with queued audio, silence-padding any
/// shortfall. RT-safe, only atomic loads/stores and memcpy. If `flush` is set, drop all
/// queued audio first (seek/stop/track change) and emit silence for this quantum.
pub fn fill_output(c: &mut Consumer<f32>, out: &mut [f32], flush: &AtomicBool) {
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
        chunk.commit_all();
    }
    for s in &mut out[written..] {
        *s = 0.0;
    }
}
