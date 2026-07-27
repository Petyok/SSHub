//! Heap-allocation counting for tests.
//!
//! Rust permits exactly one `#[global_allocator]` per binary, so every test in
//! the lib test binary that wants to *prove* an allocation-free hot path has to
//! share this one. Add consumers here rather than declaring a second allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// Passthrough allocator that counts allocations made by the **current** thread
/// while that thread is armed. Counting per thread keeps a measurement honest
/// even though the test binary runs its cases in parallel.
struct CountingAllocator;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<usize> = const { Cell::new(0) };
}

fn note_allocation() {
    // `try_with` because TLS is gone during thread teardown, where an
    // allocation is none of this counter's business anyway.
    if ARMED.try_with(Cell::get).unwrap_or(false) {
        let _ = COUNT.try_with(|count| count.set(count.get() + 1));
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note_allocation();
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Number of heap allocations this thread made while running `body`.
pub(crate) fn allocations_during(body: impl FnOnce()) -> usize {
    COUNT.with(|count| count.set(0));
    ARMED.with(|armed| armed.set(true));
    body();
    ARMED.with(|armed| armed.set(false));
    COUNT.with(Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A green allocation-free assertion elsewhere is only meaningful if this
    /// counter can actually see an allocation.
    #[test]
    fn the_counter_sees_a_deliberate_allocation() {
        assert!(
            allocations_during(|| {
                std::hint::black_box(vec![0u8; 64]);
            }) > 0
        );
        assert_eq!(
            allocations_during(|| {
                std::hint::black_box(1u8 + 1);
            }),
            0
        );
    }
}
