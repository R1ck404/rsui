//! Giving back the pages of the engine's lists that have nothing on them, with `RSUI_GIVE_BACK=1`. It's a
//! child of `gc.rs`, as it works on the free lists' and marks' insides.
//!
//! After a full collection, a page of the list of objects, strings or cells whose every place is free is decommitted: its
//! places come off the free list, so nothing is put there, and they become holes. A hole looks reached and old to the
//! collector (its mark is set before each collection marks anything), so a stale handle to one, which a register can still
//! hold, is never traced into it, and sweeping leaves it alone. When the free list runs out and the list would grow, the
//! lowest holes are committed again first, filled with free places, and go back on the free list. So nothing that runs
//! JavaScript reads a hole, and what walks the whole of a list (`Vm::memory`, the census, the checks of `RSUI_GC_VERIFY`)
//! skips them.
#![allow(unsafe_code)]

use super::FreeSlots;
use crate::arena::Arena;
#[cfg(windows)]
use std::ffi::c_void;

const PAGE: usize = 4096;
const MEM_COMMIT: u32 = 0x1000;
const MEM_DECOMMIT: u32 = 0x4000;
const PAGE_READWRITE: u32 = 4;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
}
#[cfg(not(windows))]
use rsui_os::vm::windows_like::{VirtualAlloc, VirtualFree};

thread_local! {
    /// Turns giving back on for the engines on this thread, whatever the environment says, for tests, and for the
    /// collection a quiet app runs ([`crate::Engine::collect_garbage_when_quiet`]), which also shrinks storage.
    pub(crate) static FORCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether `RSUI_GIVE_BACK` asks for empty pages to be given back after each full collection.
pub(crate) fn enabled() -> bool {
    if FORCE.with(std::cell::Cell::get) {
        return true;
    }
    use std::sync::atomic::{AtomicU8, Ordering};
    static ON: AtomicU8 = AtomicU8::new(0);
    match ON.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var_os("RSUI_GIVE_BACK").is_some_and(|value| value != "0");
            ON.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        known => known == 2,
    }
}

/// The places of a list that are on pages given back, one bit each, laid out as the free list's bits are.
#[derive(Default, Debug)]
pub(crate) struct Holes {
    bits: Vec<u64>,
    pages: usize,
}

impl Holes {
    /// Whether this place is on a page that's been given back.
    #[inline]
    pub(crate) fn has(&self, index: usize) -> bool {
        self.pages != 0 && super::is_marked(&self.bits, index)
    }

    pub(crate) fn pages(&self) -> usize {
        self.pages
    }

    /// Sets the marks of every hole, so a collection never traces into one and sweeping leaves them alone.
    pub(crate) fn mark(&self, marks: &mut Vec<u64>) {
        if self.pages == 0 {
            return;
        }
        if marks.len() < self.bits.len() {
            marks.resize(self.bits.len(), 0);
        }
        for (mark, hole) in marks.iter_mut().zip(&self.bits) {
            *mark |= hole;
        }
    }

    /// The bits of the holes in one word of marks, to leave them out of what's looked at as marked.
    #[inline]
    pub(crate) fn word(&self, at: usize) -> u64 {
        if self.pages == 0 {
            0
        } else {
            self.bits.get(at).copied().unwrap_or(0)
        }
    }
}

/// How a list's places sit on its pages.
fn layout<T>(list: &Arena<T>) -> Option<(usize, usize)> {
    let size = size_of::<T>();
    let offset = list.offset();
    // A place has to be on one page, so a page's places are only its own.
    (PAGE % size == 0 && offset % size == 0).then_some((size, offset))
}

impl FreeSlots {
    pub(crate) fn holes(&self) -> &Holes {
        &self.holes
    }

    /// Gives back each page of `list` whose every place is free, taking its places off the free list and marking them as
    /// reached, as holes. Says how many pages that was.
    pub(crate) fn give_back<T>(&mut self, list: &mut Arena<T>, marks: &mut Vec<u64>) -> usize {
        #[cfg(test)]
        report_faults();
        let Some((size, offset)) = layout(list) else {
            return 0;
        };
        let per_page = PAGE / size;
        let len = list.len();
        // The first whole page starts where the list's first page ends, unless the list starts on a page.
        let mut first = if offset == 0 { 0 } else { (PAGE - offset) / size };
        let start = list.page_start();
        let mut given = 0;
        let mut run: Option<(usize, usize)> = None;
        let flush = |run: &mut Option<(usize, usize)>| {
            if let Some((from, pages)) = run.take() {
                // SAFETY: these whole pages are inside what the list committed, and hold only free places, which nothing
                // reads now that they're holes.
                unsafe {
                    VirtualFree(start.add(offset + from * size).cast(), pages * PAGE, MEM_DECOMMIT);
                }
            }
        };
        while first + per_page <= len {
            let all_free = (first..first + per_page).all(|index| self.is_free(index) && !self.holes.has(index));
            if all_free {
                if self.holes.bits.len() < self.bits.len() {
                    self.holes.bits.resize(self.bits.len(), 0);
                }
                if marks.len() < self.bits.len() {
                    marks.resize(self.bits.len(), 0);
                }
                for index in first..first + per_page {
                    self.bits[index / 64] &= !(1u64 << (index % 64));
                    self.holes.bits[index / 64] |= 1u64 << (index % 64);
                    marks[index / 64] |= 1u64 << (index % 64);
                }
                self.holes.pages += 1;
                given += 1;
                run = match run {
                    Some((from, pages)) if from + pages * per_page == first => Some((from, pages + 1)),
                    other => {
                        let mut other = other;
                        flush(&mut other);
                        Some((first, 1))
                    }
                };
            }
            first += per_page;
        }
        flush(&mut run);
        given
    }

    /// Commits the lowest holes again, up to 16 pages of them in a row, fills their places with `fill`'s, and puts them on
    /// the free list. Says whether there were any.
    #[cold]
    #[inline(never)]
    pub(crate) fn take_back<T>(&mut self, list: &mut Arena<T>, marks: &mut [u64], fill: impl Fn() -> T) -> bool {
        if self.holes.pages == 0 {
            return false;
        }
        let Some((size, offset)) = layout(list) else {
            return false;
        };
        let per_page = PAGE / size;
        let Some(word) = self.holes.bits.iter().position(|word| *word != 0) else {
            self.holes.pages = 0;
            return false;
        };
        let first = word * 64 + self.holes.bits[word].trailing_zeros() as usize;
        let mut pages = 0;
        while pages < 16 && self.holes.has(first + pages * per_page) {
            pages += 1;
        }
        let start = list.page_start();
        // SAFETY: these pages are inside what the list reserved, and were committed before they were given back.
        let done = unsafe { VirtualAlloc(start.add(offset + first * size).cast(), pages * PAGE, MEM_COMMIT, PAGE_READWRITE) };
        assert!(!done.is_null(), "out of memory");
        let base = list.as_mut_ptr();
        for index in first..first + pages * per_page {
            // SAFETY: the place has memory again, of zeros, which is written over without being read or dropped.
            unsafe { base.add(index).write(fill()) };
            self.holes.bits[index / 64] &= !(1u64 << (index % 64));
            if let Some(mark) = marks.get_mut(index / 64) {
                *mark &= !(1u64 << (index % 64));
            }
            self.bits[index / 64] |= 1u64 << (index % 64);
        }
        self.holes.pages -= pages;
        self.next = self.next.min(first / 64);
        true
    }

    /// Drops what's in each place of `list` that isn't a hole, and empties the list, so dropping it reads no hole.
    fn drop_around_holes<T>(&self, list: &mut Arena<T>) {
        if self.holes.pages == 0 {
            return;
        }
        let base = list.as_mut_ptr();
        for index in 0..list.len() {
            if !self.holes.has(index) {
                // SAFETY: the place holds an item, which is dropped once, as the list is emptied without dropping next.
                unsafe { std::ptr::drop_in_place(base.add(index)) };
            }
        }
        // SAFETY: every item has been dropped, or is a hole with nothing to drop.
        unsafe { list.set_len(0) };
    }

    /// Takes back every hole, for what walks the whole list without knowing about them.
    pub(crate) fn take_back_all<T>(&mut self, list: &mut Arena<T>, marks: &mut [u64], fill: impl Fn() -> T) {
        while self.take_back(list, marks, &fill) {}
    }
}

impl Drop for crate::vm::Vm {
    /// The lists' own `Drop`s would drop every place, holes too.
    fn drop(&mut self) {
        #[cfg(feature = "jsprof")]
        crate::jsprof::flush();
        self.free_objects.drop_around_holes(&mut self.objects.list);
        self.strings.free.drop_around_holes(&mut self.strings.items);
    }
}

impl crate::vm::Vm {
    /// Gives back the empty pages of the lists of objects, strings and cells, after a full collection.
    pub(crate) fn give_back_pages(&mut self) {
        self.free_objects.give_back(&mut self.objects.list, &mut self.object_marks);
        self.strings.free.give_back(&mut self.strings.items, &mut self.strings.marks);
        self.free_cells.give_back(&mut self.cells.list, &mut self.cells.marks);
    }

    /// Takes back every page given back, for what walks a whole list (the census).
    pub(crate) fn take_back_pages(&mut self) {
        self.free_objects
            .take_back_all(&mut self.objects.list, &mut self.object_marks, crate::object::Object::free);
        self.strings.take_back_all();
        self.free_cells.take_back_all(&mut self.cells.list, &mut self.cells.marks, || crate::value::Value::UNDEFINED);
    }

    /// Where a new object goes when nothing's on the free list: in the lowest pages given back, if there are any, or else
    /// at the end of the list.
    #[cold]
    #[inline(never)]
    pub(crate) fn place_object_in_hole(&mut self, object: crate::object::Object) -> Option<u32> {
        if !self
            .free_objects
            .take_back(&mut self.objects.list, &mut self.object_marks, crate::object::Object::free)
        {
            return None;
        }
        let handle = self.free_objects.pop()?;
        // SAFETY: that place was just filled with a free place, which has nothing to drop.
        unsafe { self.objects.as_mut_ptr().add(handle as usize).write(object) };
        Some(handle)
    }

    /// The same for a new cell.
    #[cold]
    #[inline(never)]
    pub(crate) fn place_cell_in_hole(&mut self, value: crate::value::Value) -> Option<u32> {
        if !self
            .free_cells
            .take_back(&mut self.cells.list, &mut self.cells.marks, || crate::value::Value::UNDEFINED)
        {
            return None;
        }
        let handle = self.free_cells.pop()?;
        self.cells.list[handle as usize] = value;
        Some(handle)
    }
}

impl crate::text::Strings {
    /// Where a new string goes when nothing's on the free list, as for objects.
    #[cold]
    #[inline(never)]
    pub(crate) fn place_in_hole(&mut self, data: crate::text::StringData) -> Result<u32, crate::text::StringData> {
        if !self.free.take_back(&mut self.items, &mut self.marks, crate::text::StringData::free) {
            return Err(data);
        }
        let Some(handle) = self.free.pop() else {
            return Err(data);
        };
        // SAFETY: that place was just filled with a free place, which has only empty text in it to forget.
        unsafe { self.items.as_mut_ptr().add(handle as usize).write(data) };
        Ok(handle)
    }

    fn take_back_all(&mut self) {
        self.free.take_back_all(&mut self.items, &mut self.marks, crate::text::StringData::free);
    }
}

/// Elsewhere what's given back can still be read, as zeros (see `rsui_os::vm`), so there's no fault to report.
#[cfg(all(test, not(windows)))]
fn report_faults() {}

/// In tests, a read of a page that's been given back says where it came from, rather than ending the run with no word.
#[cfg(all(test, windows))]
fn report_faults() {
    #[repr(C)]
    struct Record {
        code: u32,
        flags: u32,
        inner: *mut Record,
        address: *mut c_void,
        count: u32,
        information: [usize; 15],
    }
    #[repr(C)]
    struct Pointers {
        record: *mut Record,
        context: *mut c_void,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AddVectoredExceptionHandler(first: u32, handler: unsafe extern "system" fn(*mut Pointers) -> i32) -> *mut c_void;
    }
    unsafe extern "system" fn handler(pointers: *mut Pointers) -> i32 {
        // SAFETY: Windows hands the handler a valid record.
        let record = unsafe { &*(*pointers).record };
        if record.code == 0xC000_0005 {
            eprintln!(
                "access violation at {:p}, reading {:#x}
{}",
                record.address,
                record.information[1],
                std::backtrace::Backtrace::force_capture()
            );
            std::process::exit(99);
        }
        0
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: the handler only reads what it's given.
    ONCE.call_once(|| unsafe {
        AddVectoredExceptionHandler(1, handler);
    });
}
