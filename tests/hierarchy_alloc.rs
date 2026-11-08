//! The warmed unique-component hierarchy lookup must not allocate.
//!
//! The counter covers only one successful lookup after the per-parent name
//! index exists. Cold setup, error paths and the branched fallback may allocate
//! and are outside this check.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

use ondas::HierarchyPath;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn count() {
    ALLOCATIONS.with(|count| count.set(count.get() + 1));
}

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn warmed_unique_lookup_does_not_allocate() {
    let input = b"$scope module tb $end\n\
        $scope module dut $end\n\
        $var wire 8 ! data $end\n\
        $upscope $end\n\
        $upscope $end\n\
        $enddefinitions $end\n\
        #0 b0 !\n";
    let wave = ondas::open_bytes("lookup.vcd", input.as_slice().into()).expect("open authored VCD");
    let hierarchy = wave.hierarchy();
    let path = HierarchyPath::from_components(["tb", "dut", "data"]);
    // Cold lookup builds the per-parent name index.
    assert!(hierarchy.signal_path(&path).is_ok());
    ALLOCATIONS.with(|count| count.set(0));
    let signal = hierarchy.signal_path(&path).expect("warm lookup");
    let allocations = ALLOCATIONS.with(Cell::get);
    assert_eq!(allocations, 0, "warm unique lookup allocated");
    assert_eq!(signal.width(), Some(8));
}
