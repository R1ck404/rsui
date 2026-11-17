//! Lives in its own test binary because it replaces the global allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use rsui_core::{Element, Node, Role, diff};

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAllocator;

// SAFETY: every call is forwarded unchanged to the system allocator.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|count| count.set(count.get() + 1));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn tree() -> Node {
    Element::new(Role::Window)
        .children((0..50usize).map(|section| {
            Element::new(Role::Group)
                .key(section)
                .children((0..20usize).map(|item| {
                    Element::new(Role::Button)
                        .key(item)
                        .name("item")
                        .prop("enabled", true)
                }))
        }))
        .into_node()
}

fn allocations_during(work: impl FnOnce()) -> usize {
    let before = ALLOCATIONS.with(Cell::get);
    work();
    ALLOCATIONS.with(Cell::get) - before
}

#[test]
fn diffing_equal_trees_does_not_allocate_per_node() {
    let (old, new) = (tree(), tree());
    // The only allocation is the buffer tracking the current path.
    let allocations = allocations_during(|| assert!(diff(&old, &new).is_empty()));
    assert!(allocations <= 1, "made {allocations} allocations");
}

#[test]
fn diffing_a_tree_with_itself_does_not_allocate() {
    let tree = tree();
    let allocations = allocations_during(|| assert!(diff(&tree, &tree).is_empty()));
    assert_eq!(allocations, 0);
}
