//! A global allocator that counts allocations, so the benchmark can report
//! what one call allocates.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

struct Counting;

static COUNT: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

#[global_allocator]
static GLOBAL: Counting = Counting;

// SAFETY: every call forwards to `System` with the caller's arguments; the
// counters only add to atomics.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        COUNT.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        COUNT.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Allocations and allocated bytes since the process started.
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
