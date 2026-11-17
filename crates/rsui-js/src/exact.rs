//! Exact-size storage, with `RSUI_EXACT=1`: after a full collection, the property values and array
//! elements that objects keep outside themselves are shrunk to their length, where the allocator would give them a smaller
//! block. It's a child of `gc.rs`, as it goes through the list of objects' insides.

thread_local! {
    /// Turns this on for the engines on this thread, whatever the environment says, for tests.
    pub(crate) static FORCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether `RSUI_EXACT` asks for objects' storage to be shrunk to fit after each full collection.
pub(crate) fn enabled() -> bool {
    if FORCE.with(std::cell::Cell::get) || super::giveback::FORCE.with(std::cell::Cell::get) {
        return true;
    }
    use std::sync::atomic::{AtomicU8, Ordering};
    static ON: AtomicU8 = AtomicU8::new(0);
    match ON.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var_os("RSUI_EXACT").is_some_and(|value| value != "0");
            ON.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        known => known == 2,
    }
}

impl crate::vm::Vm {
    /// Shrinks every live object's storage to fit, and says how many bytes of room that gave back. Only when nothing but
    /// JavaScript is running (see [`crate::vm::Vm::nested_runs`]), as a native that has called back into it could be
    /// holding where some elements were.
    pub(crate) fn shrink_storage(&mut self) -> usize {
        if self.nested_runs > 1 {
            return 0;
        }
        let mut saved = 0;
        for index in 0..self.objects.list.len() {
            if self.free_objects.is_free(index) || self.free_objects.holes().has(index) {
                continue;
            }
            saved += self.objects.list[index].shrink_to_fit();
        }
        saved
    }
}
