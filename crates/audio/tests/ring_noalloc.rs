//! Proof that the realtime `fill_output` path (and the visualizer scope tap) never allocate. This
//! file holds ONLY this test so the process-global allocation counter is not perturbed by other
//! tests running concurrently in the same binary.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use xubamp_audio::ring::{apply_gain, fill_output, new_ring, push_block, SharedState};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn fill_output_does_not_allocate() {
    let (mut p, mut c) = new_ring(64);
    // SharedState allocates the scope ring here, once, unmeasured.
    let shared = SharedState::new();
    let block: Vec<f32> = (0..128).map(|i| i as f32).collect();
    let mut out = [0.0f32; 32];
    let mut scope_out = [0.0f32; 16];

    // Prime the ring and warm any one-time lazy state on the RT path, unmeasured.
    push_block(&mut p, &block);
    fill_output(&mut c, &mut out, &shared);
    push_block(&mut p, &block);
    shared.push_scope(&out);
    shared.read_scope(&mut scope_out);

    let before = ALLOCS.load(Ordering::Relaxed);
    // Normal copy path, then the gain stage (non-unity so it does real work).
    fill_output(&mut c, &mut out, &shared);
    apply_gain(&mut out, 0.5, 0.25);
    // The visualizer scope tap the RT calls right after (mono downmix into the ring) and the UI
    // read-back, both alloc-free.
    shared.push_scope(&out);
    shared.read_scope(&mut scope_out);
    // Gapless-seek drop branch: publish a boundary ahead of the read cursor so fill_output drops
    // the stale tail (read_chunk + commit_all) before playing, which must not allocate.
    push_block(&mut p, &block);
    let cursor = shared.removed_frames.load(Ordering::Relaxed);
    shared.drop_before.store(cursor + 8, Ordering::Release); // drop 8 frames
    fill_output(&mut c, &mut out, &shared);
    // Underrun/silence path: drain the ring fully, then read it empty.
    while c.slots() > 0 {
        fill_output(&mut c, &mut out, &shared);
    }
    fill_output(&mut c, &mut out, &shared);
    let after = ALLOCS.load(Ordering::Relaxed);

    assert_eq!(after, before, "the realtime fill_output + apply_gain + scope-tap path allocated");
}
