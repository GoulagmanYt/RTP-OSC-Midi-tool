//! Thread-local allocation counter used only by test binaries.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

thread_local! {
    static COUNT: Cell<Option<usize>> = const { Cell::new(None) };
}

struct CountingAllocator;
fn allocated() {
    let _ = COUNT.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}

// Every operation forwards the unchanged layout/pointer to the system allocator.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocated();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocated();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        allocated();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

pub(crate) fn count_allocations(work: impl FnOnce()) -> usize {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            COUNT.with(|count| count.set(self.0));
        }
    }
    let restore = Restore(COUNT.with(|count| count.replace(Some(0))));
    work();
    let result = COUNT.with(|count| count.get().unwrap());
    drop(restore);
    result
}
