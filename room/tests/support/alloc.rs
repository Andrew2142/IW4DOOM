// SPDX-License-Identifier: GPL-3.0-or-later
//! Counting global allocator for allocation tests (sheet 13 section 5.2 and the `allocations`
//! bench in section 5.5).
//!
//! Opt-in per test binary: a binary that wants it installs the allocator itself,
//!
//! ```ignore
//! #[global_allocator]
//! static ALLOC: support::alloc::CountingAlloc = support::alloc::CountingAlloc;
//! ```
//!
//! and then measures a closure with [`measure`] or asserts [`assert_no_allocations`]. Binaries
//! that do not install it pay nothing: merely compiling this module has no runtime effect.
//!
//! Only the thread that is inside [`measure`] is counted, so the test runner's own threads
//! cannot disturb a measurement.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static DEALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

thread_local! {
    // const-initialised and without a destructor, so touching it from inside the allocator is
    // safe (no lazy initialisation that could itself allocate).
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}

fn counting() -> bool {
    COUNTING.try_with(Cell::get).unwrap_or(false)
}

/// Global allocator that forwards to [`System`] and counts events on the measuring thread.
#[derive(Debug, Default, Clone, Copy)]
pub struct CountingAlloc;

// SAFETY: every method forwards to `System` with the caller's arguments unchanged; the only
// extra work is relaxed atomic counter updates that never allocate.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if counting() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        // SAFETY: same layout contract as the caller's.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if counting() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        // SAFETY: same layout contract as the caller's.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if counting() {
            DEALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: `ptr` and `layout` come from a matching `alloc` on `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if counting() {
            // A growing or moving realloc is an allocation event for budget purposes.
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        // SAFETY: `ptr` and `layout` come from a matching `alloc` on `System`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Allocator activity observed during one [`measure`] call.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AllocStats {
    /// `alloc`, `alloc_zeroed` and `realloc` calls.
    pub allocations: u64,
    /// `dealloc` calls.
    pub deallocations: u64,
    /// Bytes requested by the allocation calls above.
    pub bytes: u64,
}

/// Runs `f` and returns its result with the allocator activity of the current thread.
///
/// Returns all-zero stats when the binary did not install [`CountingAlloc`]; use
/// [`installed`] to tell the two cases apart.
pub fn measure<R>(f: impl FnOnce() -> R) -> (R, AllocStats) {
    let before = (
        ALLOCATIONS.load(Ordering::Relaxed),
        DEALLOCATIONS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed),
    );
    COUNTING.with(|c| c.set(true));
    let result = f();
    COUNTING.with(|c| c.set(false));
    let stats = AllocStats {
        allocations: ALLOCATIONS.load(Ordering::Relaxed) - before.0,
        deallocations: DEALLOCATIONS.load(Ordering::Relaxed) - before.1,
        bytes: BYTES.load(Ordering::Relaxed) - before.2,
    };
    (result, stats)
}

/// `true` when [`CountingAlloc`] is the process's global allocator (probes with one real
/// allocation).
pub fn installed() -> bool {
    let (_, stats) = measure(|| std::hint::black_box(Box::new(0u64)));
    stats.allocations > 0
}

/// Fails the test if `f` allocates, or if the counting allocator is not installed (so a
/// forgotten `#[global_allocator]` cannot make the assertion pass vacuously).
pub fn assert_no_allocations<R>(label: &str, f: impl FnOnce() -> R) -> R {
    assert!(
        installed(),
        "{label}: CountingAlloc is not the global allocator of this test binary"
    );
    let (result, stats) = measure(f);
    assert_eq!(
        stats.allocations, 0,
        "{label}: expected zero allocations, saw {} ({} bytes)",
        stats.allocations, stats.bytes
    );
    result
}
