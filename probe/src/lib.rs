//! An allocator wrapper that counts what a module asks its allocator for, so
//! that `peakmark` can report it next to the capacity of the linear memory.
//!
//! ```ignore
//! use std::alloc::System;
//! use peakmark_probe::Probe;
//!
//! #[global_allocator]
//! static ALLOCATOR: Probe<System> = Probe::new(System);
//! ```
//!
//! The probe observes four numbers and nothing else: the bytes that are
//! requested and not yet freed (*live*), the highest that value reached
//! (*peak*), and how many allocations and frees happened. It does not track
//! fragmentation, the allocator's own bookkeeping or where a request came from.
//!
//! The values are exported from the module as `peakmark_probe_live`,
//! `peakmark_probe_peak`, `peakmark_probe_allocations` and
//! `peakmark_probe_frees`, each a function without parameters that returns an
//! `i64`, and `peakmark_probe_version`, which returns the protocol version.
//! `peakmark` reads them after instantiation and after every run of a scenario.
//!
//! A `realloc` counts as one allocation and one free, and changes *live* by the
//! difference between the new and the old size. A request that fails is not
//! counted.

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicU64, Ordering};

/// The version of the export protocol. `peakmark` reads only the versions it knows.
pub const PROTOCOL_VERSION: u32 = 1;

static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static FREES: AtomicU64 = AtomicU64::new(0);

/// Wraps a global allocator and counts the requests that go through it.
#[derive(Debug)]
pub struct Probe<A> {
    inner: A,
}

impl<A> Probe<A> {
    /// Wraps `inner`. Install the result with `#[global_allocator]`.
    #[must_use]
    pub const fn new(inner: A) -> Self {
        Self { inner }
    }
}

/// The values observed so far, for a test or a program that reads them itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observed {
    /// Bytes requested and not yet freed.
    pub live: u64,
    /// The highest `live` reached.
    pub peak: u64,
    /// Allocations, including each `realloc`.
    pub allocations: u64,
    /// Frees, including each `realloc`.
    pub frees: u64,
}

/// Reads the values observed so far.
#[must_use]
pub fn observed() -> Observed {
    Observed {
        live: LIVE.load(Ordering::Relaxed),
        peak: PEAK.load(Ordering::Relaxed),
        allocations: ALLOCATIONS.load(Ordering::Relaxed),
        frees: FREES.load(Ordering::Relaxed),
    }
}

fn grow(bytes: u64) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

fn shrink(bytes: u64) {
    LIVE.fetch_sub(bytes, Ordering::Relaxed);
}

// The wrapper only forwards to the allocator it was given and counts around it.
unsafe impl<A: GlobalAlloc> GlobalAlloc for Probe<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { self.inner.alloc(layout) };
        if !pointer.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            grow(layout.size() as u64);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { self.inner.alloc_zeroed(layout) };
        if !pointer.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            grow(layout.size() as u64);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { self.inner.dealloc(pointer, layout) };
        FREES.fetch_add(1, Ordering::Relaxed);
        shrink(layout.size() as u64);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let moved = unsafe { self.inner.realloc(pointer, layout, new_size) };
        if !moved.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            FREES.fetch_add(1, Ordering::Relaxed);
            let (old, new) = (layout.size() as u64, new_size as u64);
            if new >= old {
                grow(new - old);
            } else {
                shrink(old - new);
            }
        }
        moved
    }
}

// The exports. `no_mangle` keeps them in the module's export section.

/// The protocol version of the exports below.
#[unsafe(no_mangle)]
pub extern "C" fn peakmark_probe_version() -> u32 {
    PROTOCOL_VERSION
}

/// Bytes requested and not yet freed.
#[unsafe(no_mangle)]
pub extern "C" fn peakmark_probe_live() -> u64 {
    LIVE.load(Ordering::Relaxed)
}

/// The highest value `peakmark_probe_live` reached.
#[unsafe(no_mangle)]
pub extern "C" fn peakmark_probe_peak() -> u64 {
    PEAK.load(Ordering::Relaxed)
}

/// Allocations so far, each `realloc` included.
#[unsafe(no_mangle)]
pub extern "C" fn peakmark_probe_allocations() -> u64 {
    ALLOCATIONS.load(Ordering::Relaxed)
}

/// Frees so far, each `realloc` included.
#[unsafe(no_mangle)]
pub extern "C" fn peakmark_probe_frees() -> u64 {
    FREES.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::System;

    // One test: the counters are process-wide.
    #[test]
    fn counts_requested_live_bytes_peak_and_events() {
        let probe = Probe::new(System);
        let small = Layout::from_size_align(100, 8).unwrap();
        let before = observed();
        unsafe {
            let a = probe.alloc(small);
            let b = probe.alloc_zeroed(small);
            let now = observed();
            assert_eq!(now.live - before.live, 200);
            assert_eq!(now.allocations - before.allocations, 2);

            let a = probe.realloc(a, small, 300);
            assert_eq!(observed().live - before.live, 400);

            let a = probe.realloc(a, Layout::from_size_align(300, 8).unwrap(), 50);
            assert_eq!(observed().live - before.live, 150);
            assert!(observed().peak >= before.live + 400);

            probe.dealloc(a, Layout::from_size_align(50, 8).unwrap());
            probe.dealloc(b, small);
        }
        let end = observed();
        assert_eq!(end.live, before.live);
        assert_eq!(end.allocations - before.allocations, 4);
        assert_eq!(end.frees - before.frees, 4);
    }

    #[test]
    fn the_exports_read_the_same_counters() {
        assert_eq!(peakmark_probe_version(), PROTOCOL_VERSION);
        assert_eq!(peakmark_probe_live(), observed().live);
    }
}
