//! Where compiled code lives: one range of addresses set aside up front, which is given real memory as code is added.
//! No address is ever writable and executable at once: the memory is mapped twice, at two ranges of addresses, one
//! read-write, which code is written through, and one read-execute, which it runs from. So adding code changes no
//! page's protection, which took two calls into the system for each function, each of which had every core forget
//! what it knew of those pages, and was about a third of what compiling a function cost.
#![allow(unsafe_code)]

use std::ffi::c_void;

const MEM_COMMIT: u32 = 0x1000;
const PAGE_READWRITE: u32 = 0x04;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const SEC_RESERVE: u32 = 0x0400_0000;
const FILE_MAP_WRITE: u32 = 0x02;
const FILE_MAP_READ: u32 = 0x04;
const FILE_MAP_EXECUTE: u32 = 0x20;
/// How much memory is given to the code at a time.
const CHUNK: usize = 64 * 1024;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileMappingW(
        file: isize,
        attributes: *const c_void,
        protect: u32,
        size_high: u32,
        size_low: u32,
        name: *const u16,
    ) -> isize;
    fn MapViewOfFile(mapping: isize, access: u32, offset_high: u32, offset_low: u32, size: usize) -> *mut c_void;
    fn UnmapViewOfFile(address: *const c_void) -> i32;
    fn CloseHandle(handle: isize) -> i32;
    fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    #[cfg(feature = "jit-opt")]
    fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
    #[cfg(feature = "jit-opt")]
    fn MapViewOfFileEx(
        mapping: isize,
        access: u32,
        offset_high: u32,
        offset_low: u32,
        size: usize,
        address: *mut c_void,
    ) -> *mut c_void;
    fn FlushInstructionCache(process: *mut c_void, address: *const c_void, size: usize) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
}

/// The memory itself, mapped twice, which goes once nothing holds it: both the engine, whose code runs from it, and
/// the thread that writes it, which can still be finishing a function when the engine goes.
pub(crate) struct Views {
    mapping: isize,
    /// Where the code runs from, read and execute only.
    base: *mut u8,
    /// The same memory, where it's written, read and write only.
    writable: *mut u8,
    /// Where the code's feedback is (see `jit::feedback`), within 2 GB of the code, or null if there's none.
    #[cfg(feature = "jit-opt")]
    feedback: *mut u8,
}

// SAFETY: it's only addresses, which whoever holds it can run code from; only the one `CodeMemory` writes through
// them.
unsafe impl Send for Views {}
unsafe impl Sync for Views {}

impl Drop for Views {
    fn drop(&mut self) {
        // SAFETY: these are the views and the mapping `CodeMemory::new` made, and nothing runs from them once nothing
        // holds them.
        unsafe {
            UnmapViewOfFile(self.base.cast());
            UnmapViewOfFile(self.writable.cast());
            CloseHandle(self.mapping);
            #[cfg(feature = "jit-opt")]
            if !self.feedback.is_null() {
                VirtualFree(self.feedback.cast(), 0, MEM_RELEASE);
            }
        }
    }
}

#[cfg(feature = "jit-opt")]
const MEM_RESERVE: u32 = 0x2000;
#[cfg(feature = "jit-opt")]
const MEM_RELEASE: u32 = 0x8000;
/// How much feedback there can be, at most: about a byte or two for each of the code's 170 an instruction.
#[cfg(feature = "jit-opt")]
const FEEDBACK_BYTES: usize = 8 << 20;

/// Maps the view code runs from with room for feedback just after it, so that code can name a byte of feedback by its
/// distance: finds a place for both with a reservation of their size, gives it back, maps the view at its start and
/// reserves the rest. Another thread can take the place in between, so it tries a few times. The view and the room, or
/// nulls. (With only the view mapped where the system likes, the room near it was often taken: views of code are
/// packed side by side, and a process with several engines can fill a gigabyte on either side.)
#[cfg(feature = "jit-opt")]
fn map_with_room(mapping: isize, reserved: usize) -> (*mut u8, *mut u8) {
    const PAGE_NOACCESS: u32 = 0x01;
    for _ in 0..8 {
        // SAFETY: a reservation of addresses only, given back at once; then a view of the mapping and a reservation
        // where it was, each checked.
        unsafe {
            let place = VirtualAlloc(std::ptr::null_mut(), reserved + FEEDBACK_BYTES, MEM_RESERVE, PAGE_NOACCESS);
            if place.is_null() {
                break;
            }
            VirtualFree(place, 0, MEM_RELEASE);
            let base = MapViewOfFileEx(mapping, FILE_MAP_READ | FILE_MAP_EXECUTE, 0, 0, reserved, place);
            if base.is_null() {
                continue;
            }
            let room = VirtualAlloc(base.cast::<u8>().add(reserved).cast(), FEEDBACK_BYTES, MEM_RESERVE, PAGE_READWRITE);
            if room.is_null() {
                UnmapViewOfFile(base);
                continue;
            }
            return (base.cast(), room.cast());
        }
    }
    (std::ptr::null_mut(), std::ptr::null_mut())
}

/// Sets aside room for feedback within 2 GB of the code, which is `reserved` bytes from `base`, if `map_with_room`
/// couldn't: just after the code if that's free, else the nearest place after or before it that is, a megabyte at a
/// time. Null if there's none within a gigabyte, and then nothing is recorded.
#[cfg(feature = "jit-opt")]
fn reserve_near(base: usize, reserved: usize) -> *mut u8 {
    const STEP: usize = 1 << 20;
    let after = (base + reserved).next_multiple_of(CHUNK);
    for step in 0..1024 {
        for at in [after + step * STEP, base.wrapping_sub(FEEDBACK_BYTES + step * STEP) & !(CHUNK - 1)] {
            if at == 0 || at > usize::MAX / 2 {
                continue;
            }
            let lowest = at.min(base);
            let highest = (at + FEEDBACK_BYTES).max(base + reserved);
            if highest - lowest >= (1 << 31) - (1 << 20) {
                continue;
            }
            // SAFETY: asking for addresses only; the system says no if they're in use.
            let got = unsafe { VirtualAlloc(at as *mut c_void, FEEDBACK_BYTES, MEM_RESERVE, PAGE_READWRITE) };
            if !got.is_null() {
                return got.cast();
            }
        }
    }
    std::ptr::null_mut()
}

pub(crate) struct CodeMemory {
    views: std::sync::Arc<Views>,
    base: *mut u8,
    writable: *mut u8,
    /// How many bytes there are addresses for, which is the most code there can be.
    reserved: usize,
    /// How many of those have real memory.
    committed: usize,
    /// How many bytes are in use.
    used: usize,
    /// The feedback's room (see `Views::feedback`), and how much of it has real memory and is in use.
    #[cfg(feature = "jit-opt")]
    feedback: *mut u8,
    #[cfg(feature = "jit-opt")]
    feedback_committed: usize,
    #[cfg(feature = "jit-opt")]
    feedback_used: usize,
}

impl CodeMemory {
    /// Sets aside room for this many bytes of code. Nothing but addresses is used until code is added. `None` if even
    /// that's refused.
    pub(crate) fn new(reserved: usize) -> Option<Self> {
        let reserved = reserved.next_multiple_of(CHUNK);
        // SAFETY: a mapping of the page file, of memory that's only reserved until it's committed, has no
        // preconditions; each view is checked, and what was made is undone if one wasn't.
        unsafe {
            let mapping = CreateFileMappingW(
                -1,
                std::ptr::null(),
                PAGE_EXECUTE_READWRITE | SEC_RESERVE,
                (reserved as u64 >> 32) as u32,
                reserved as u32,
                std::ptr::null(),
            );
            if mapping == 0 {
                return None;
            }
            let writable = MapViewOfFile(mapping, FILE_MAP_WRITE, 0, 0, reserved);
            #[cfg(feature = "jit-opt")]
            let (base, early) = match crate::jit::feedback::on() {
                true => map_with_room(mapping, reserved),
                false => (std::ptr::null_mut(), std::ptr::null_mut()),
            };
            #[cfg(feature = "jit-opt")]
            let base = if base.is_null() {
                MapViewOfFile(mapping, FILE_MAP_READ | FILE_MAP_EXECUTE, 0, 0, reserved)
            } else {
                base.cast()
            };
            #[cfg(not(feature = "jit-opt"))]
            let base = MapViewOfFile(mapping, FILE_MAP_READ | FILE_MAP_EXECUTE, 0, 0, reserved);
            if writable.is_null() || base.is_null() {
                for view in [writable, base] {
                    if !view.is_null() {
                        UnmapViewOfFile(view);
                    }
                }
                CloseHandle(mapping);
                return None;
            }
            #[cfg(feature = "jit-opt")]
            let feedback = if !early.is_null() {
                early
            } else if crate::jit::feedback::on() {
                reserve_near(base as usize, reserved)
            } else {
                std::ptr::null_mut()
            };
            Some(Self {
                views: std::sync::Arc::new(Views {
                    mapping,
                    base: base.cast(),
                    writable: writable.cast(),
                    #[cfg(feature = "jit-opt")]
                    feedback,
                }),
                base: base.cast(),
                writable: writable.cast(),
                reserved,
                committed: 0,
                used: 0,
                #[cfg(feature = "jit-opt")]
                feedback,
                #[cfg(feature = "jit-opt")]
                feedback_committed: 0,
                #[cfg(feature = "jit-opt")]
                feedback_used: 0,
            })
        }
    }

    /// Something that keeps the memory there while it's held, for whoever runs code from it.
    pub(crate) fn keep(&self) -> std::sync::Arc<Views> {
        self.views.clone()
    }

    pub(crate) fn used(&self) -> usize {
        self.used
    }

    pub(crate) fn committed(&self) -> usize {
        self.committed
    }

    /// Where code at this offset is.
    pub(crate) fn address(&self, offset: usize) -> usize {
        self.base as usize + offset
    }

    /// The offset code of this size would go at, if there's room. Nothing is written yet. It's at the start of a cache
    /// line, so that code written from another thread never shares one with code that's been run.
    pub(crate) fn next_offset(&self) -> usize {
        self.used.next_multiple_of(64)
    }

    pub(crate) fn has_room(&self, size: usize) -> bool {
        self.next_offset() + size <= self.reserved
    }

    /// Copies code in at [`next_offset`](Self::next_offset), and says where it went. The code has to have been made for
    /// that address, as it may have jumps to absolute ones. `None` if there's no room left or the system refused.
    pub(crate) fn add(&mut self, code: &[u8]) -> Option<usize> {
        let at = self.next_offset();
        let end = at + code.len();
        if end > self.reserved {
            return None;
        }
        // SAFETY: everything here is inside the range that `new` reserved, in both views, and the bytes written are
        // past any code that's run: the engine is on this thread, here.
        unsafe {
            if end > self.committed {
                let to = end.next_multiple_of(CHUNK).min(self.reserved);
                // Committed through one view, the memory is there in both.
                let done = VirtualAlloc(
                    self.writable.add(self.committed).cast(),
                    to - self.committed,
                    MEM_COMMIT,
                    PAGE_READWRITE,
                );
                if done.is_null() {
                    return None;
                }
                self.committed = to;
            }
            std::ptr::copy_nonoverlapping(code.as_ptr(), self.writable.add(at), code.len());
            FlushInstructionCache(GetCurrentProcess(), self.base.add(at).cast(), code.len());
        }
        self.used = end;
        Some(at)
    }

    /// Room for `size` bytes of feedback, which start as zeros: where it is, or `None` if there's no more (or none).
    #[cfg(feature = "jit-opt")]
    pub(crate) fn feedback(&mut self, size: usize) -> Option<usize> {
        if self.feedback.is_null() {
            return None;
        }
        let at = self.feedback_used.next_multiple_of(8);
        let end = at + size;
        if end > FEEDBACK_BYTES {
            return None;
        }
        if end > self.feedback_committed {
            let to = end.next_multiple_of(CHUNK).min(FEEDBACK_BYTES);
            // SAFETY: inside the room `map_with_room` or `reserve_near` set aside, which nothing else uses.
            let done = unsafe {
                VirtualAlloc(
                    self.feedback.add(self.feedback_committed).cast(),
                    to - self.feedback_committed,
                    MEM_COMMIT,
                    PAGE_READWRITE,
                )
            };
            if done.is_null() {
                return None;
            }
            self.feedback_committed = to;
        }
        self.feedback_used = end;
        Some(self.feedback as usize + at)
    }
}
