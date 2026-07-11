//! Proof that the realtime `fill_output` path never allocates. This file holds ONLY this
//! test so the process-global allocation counter is not perturbed by other tests running
//! concurrently in the same binary.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use xubamp_audio::ring::{fill_output, new_ring, push_block};

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
    let flush = AtomicBool::new(false);
    let block: Vec<f32> = (0..128).map(|i| i as f32).collect();
    let mut out = [0.0f32; 32];

    // Prime the ring and warm any one-time lazy state on the RT path, unmeasured.
    push_block(&mut p, &block);
    fill_output(&mut c, &mut out, &flush);
    push_block(&mut p, &block);

    let before = ALLOCS.load(Ordering::Relaxed);
    // Normal copy path.
    fill_output(&mut c, &mut out, &flush);
    // Flush-drain path.
    flush.store(true, Ordering::Release);
    fill_output(&mut c, &mut out, &flush);
    // Underrun/silence path (ring now empty).
    fill_output(&mut c, &mut out, &flush);
    let after = ALLOCS.load(Ordering::Relaxed);

    assert_eq!(after, before, "fill_output allocated on the realtime path");
}
