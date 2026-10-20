//! A global allocator that counts allocations, so the benchmark can report
//! what one call allocates. A global allocator serves the whole binary, so it
//! cannot be set per command; it counts only after `start_counting`, which
//! only `bench-memory` calls. Other commands pay one relaxed load per call.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

struct Counting;

static ON: AtomicBool = AtomicBool::new(false);
static COUNT: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

#[global_allocator]
static GLOBAL: Counting = Counting;

// SAFETY: every call forwards to `System` with the caller's arguments; the
// counters only add to atomics.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn count(size: usize) {
    if ON.load(Ordering::Relaxed) {
        COUNT.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(size as u64, Ordering::Relaxed);
    }
}

pub fn start_counting() {
    ON.store(true, Ordering::Relaxed);
}

/// Allocations and allocated bytes since `start_counting`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Allocs {
    pub count: u64,
    pub bytes: u64,
}

impl Allocs {
    pub fn now() -> Allocs {
        Allocs {
            count: COUNT.load(Ordering::Relaxed),
            bytes: BYTES.load(Ordering::Relaxed),
        }
    }

    pub fn since(self, before: Allocs) -> Allocs {
        Allocs {
            count: self.count - before.count,
            bytes: self.bytes - before.bytes,
        }
    }
}
