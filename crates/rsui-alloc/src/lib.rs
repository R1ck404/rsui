//! A global allocator for UI workloads: huge numbers of small blocks of a few
//! sizes, which are freed and reallocated all the time.
//!
//! Small blocks have no headers. Each aligned 64 KiB chunk has one header and
//! its own free list. Allocation and free take one class lock and do constant
//! work. Wholly unused chunks can be returned at an application idle boundary
//! with [`UiAlloc::reclaim`]; nothing runs periodically or in the background.
//!
//! Windows chunks use VirtualAlloc so releasing a chunk returns its commit,
//! without an over-aligned System allocation's extra padding. Other platforms
//! use System. Large or over-aligned individual allocations still use System.
//!
//! ```no_run
//! #[global_allocator]
//! static ALLOCATOR: rsui_alloc::UiAlloc = rsui_alloc::UiAlloc;
//! ```

pub mod owners;
pub mod split;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::UnsafeCell;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// How much memory a size class takes from the system at a time.
const CHUNK_SIZE: usize = 64 * 1024;
/// Every class size is a multiple of this, so every block has this alignment.
const GRANULE: usize = 16;

const CLASS_SIZES: [usize; 20] = [
    16, 32, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 640, 768, 896, 1024,
];
const LARGEST_CLASS: usize = CLASS_SIZES[CLASS_SIZES.len() - 1];

/// Maps a size in granules, rounded up, to the smallest class that fits it.
const CLASS_OF_GRANULES: [u8; LARGEST_CLASS / GRANULE + 1] = {
    let mut table = [0; LARGEST_CLASS / GRANULE + 1];
    let mut granules = 0;
    let mut class = 0;
    while granules < table.len() {
        if granules * GRANULE > CLASS_SIZES[class] {
            class += 1;
        }
        table[granules] = class as u8;
        granules += 1;
    }
    table
};

struct Class {
    locked: AtomicBool,
    state: UnsafeCell<ClassState>,
}

struct ClassState {
    available: *mut Chunk,
    empty: *mut Chunk,
    chunks: usize,
    /// The blocks of this class in use, for [`UiAlloc::live_bytes`].
    blocks: usize,
}

/// All links and counters are protected by the owning class lock. Full chunks
/// are on neither list, non-full chunks on `available`, zero-live chunks also
/// on `empty`. A chunk is detached from both lists before its storage is freed.
#[repr(C, align(16))]
struct Chunk {
    free: *mut u8,
    next_offset: usize,
    live: usize,
    capacity: usize,
    prev: *mut Chunk,
    next: *mut Chunk,
    empty_prev: *mut Chunk,
    empty_next: *mut Chunk,
}
const HEADER_SIZE: usize = size_of::<Chunk>();
const _: () = assert!(HEADER_SIZE % GRANULE == 0 && HEADER_SIZE < CHUNK_SIZE);

#[cfg(windows)]
mod backing {
    use super::CHUNK_SIZE;
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
        fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
    }
    pub unsafe fn alloc() -> *mut u8 {
        // SAFETY: fresh reserve+commit, read/write pages. Windows allocation
        // granularity is 64 KiB, so the result has CHUNK_SIZE alignment.
        unsafe { VirtualAlloc(std::ptr::null_mut(), CHUNK_SIZE, 0x3000, 0x04).cast() }
    }
    pub unsafe fn free(p: *mut u8) {
        // SAFETY: caller owns an entire detached VirtualAlloc reservation.
        unsafe {
            VirtualFree(p.cast(), 0, 0x8000);
        }
    }
}
#[cfg(not(windows))]
mod backing {
    use super::*;
    const LAYOUT: Layout = match Layout::from_size_align(CHUNK_SIZE, CHUNK_SIZE) {
        Ok(l) => l,
        Err(_) => panic!("invalid chunk layout"),
    };
    pub unsafe fn alloc() -> *mut u8 {
        // SAFETY: nonzero valid layout; directly using System cannot recurse.
        unsafe { System.alloc(LAYOUT) }
    }
    pub unsafe fn free(p: *mut u8) {
        // SAFETY: caller owns the detached chunk allocated with LAYOUT.
        unsafe { System.dealloc(p, LAYOUT) }
    }
}

// All methods below require the owning class lock and a valid chunk from it.
impl ClassState {
    unsafe fn remove_available(&mut self, c: *mut Chunk) {
        // SAFETY: c is on this class's doubly linked available list.
        unsafe {
            if (*c).prev.is_null() {
                self.available = (*c).next;
            } else {
                (*(*c).prev).next = (*c).next;
            }
            if !(*c).next.is_null() {
                (*(*c).next).prev = (*c).prev;
            }
        }
    }
    unsafe fn add_available(&mut self, c: *mut Chunk) {
        // SAFETY: c is not currently on the list; the lock excludes readers.
        unsafe {
            (*c).prev = ptr::null_mut();
            (*c).next = self.available;
            if !self.available.is_null() {
                (*self.available).prev = c;
            }
            self.available = c;
        }
    }
    unsafe fn remove_empty(&mut self, c: *mut Chunk) {
        // SAFETY: c is on this class's doubly linked empty list.
        unsafe {
            if (*c).empty_prev.is_null() {
                self.empty = (*c).empty_next;
            } else {
                (*(*c).empty_prev).empty_next = (*c).empty_next;
            }
            if !(*c).empty_next.is_null() {
                (*(*c).empty_next).empty_prev = (*c).empty_prev;
            }
        }
    }
    unsafe fn add_empty(&mut self, c: *mut Chunk) {
        // SAFETY: c just became empty and was not already on the list.
        unsafe {
            (*c).empty_prev = ptr::null_mut();
            (*c).empty_next = self.empty;
            if !self.empty.is_null() {
                (*self.empty).empty_prev = c;
            }
            self.empty = c;
        }
    }
}

// SAFETY: `Class::with` and `UiAlloc::reclaim` hold the lock while accessing state.
unsafe impl Sync for Class {}

static CLASSES: [Class; CLASS_SIZES.len()] = [const {
    Class {
        locked: AtomicBool::new(false),
        state: UnsafeCell::new(ClassState {
            available: ptr::null_mut(),
            empty: ptr::null_mut(),
            chunks: 0,
            blocks: 0,
        }),
    }
}; CLASS_SIZES.len()];

impl Class {
    #[inline]
    fn with<R>(&self, action: impl FnOnce(&mut ClassState) -> R) -> R {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            // Allocation/free do constant work (including a backing allocation on refill).
            std::thread::yield_now();
        }
        // SAFETY: the lock is held until the store below.
        let result = action(unsafe { &mut *self.state.get() });
        self.locked.store(false, Ordering::Release);
        result
    }
}

/// The class for a layout, if one can serve it.
#[inline]
fn class_of(layout: Layout) -> Option<usize> {
    if layout.size() <= LARGEST_CLASS && layout.align() <= GRANULE {
        let granules = layout.size().div_ceil(GRANULE);
        Some(CLASS_OF_GRANULES[granules] as usize)
    } else {
        None
    }
}

/// See the [crate documentation](crate).
pub struct UiAlloc;

impl UiAlloc {
    /// Release at most `min(max_chunks, 32)` wholly unused 64 KiB chunks.
    ///
    /// Call after caches/temporary views have been dropped, at an existing idle
    /// boundary. No timer is installed. This makes at most 20 nonblocking lock
    /// attempts and detaches at most 32 chunks; backing frees happen outside the
    /// locks. OS free latency is not a hard real-time guarantee. The return value
    /// is backing bytes released, not working-set bytes. A zero result can mean
    /// contention, not necessarily that there is nothing left to reclaim.
    ///
    /// Safe concurrently with allocation, free, reallocation and other reclaim
    /// calls, and when another allocator is global. Only UiAlloc-owned chunks
    /// are touched. Repeated idle calls may drain more; don't loop on a UI input
    /// path. Class rotation avoids starving later classes under repeated calls.
    pub fn reclaim(&self, max_chunks: usize) -> usize {
        self.reclaim_with_policy(max_chunks, false)
    }

    /// Like [`Self::reclaim`], but keeps the most recently emptied chunk in each
    /// class for subsequent input. It never allocates a reserve: at most twenty
    /// existing 64 KiB empty chunks remain (1.25 MiB), in addition to live chunks.
    ///
    /// Use at a short idle boundary when another interaction is likely. A later
    /// [`Self::reclaim`] can release the reserve too. The same per-call release
    /// budget and nonblocking class-lock policy apply.
    pub fn reclaim_for_idle(&self, max_chunks: usize) -> usize {
        self.reclaim_with_policy(max_chunks, true)
    }

    fn reclaim_with_policy(&self, max_chunks: usize, keep_warm: bool) -> usize {
        let limit = max_chunks.min(32);
        if limit == 0 {
            return 0;
        }
        static CURSOR: AtomicUsize = AtomicUsize::new(0);
        let start = CURSOR.fetch_add(1, Ordering::Relaxed) % CLASSES.len();
        let mut released = 0;
        for offset in 0..CLASSES.len() {
            let class = &CLASSES[(start + offset) % CLASSES.len()];
            if class
                .locked
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                continue;
            }
            let mut detached: *mut Chunk = ptr::null_mut();
            // SAFETY: lock exclusively owns this state. Only zero-live chunks
            // are detached, so no caller can access them after unlock.
            unsafe {
                let state = &mut *class.state.get();
                while released < limit && !state.empty.is_null() {
                    // Keep the most recently emptied chunk. Detaching the next
                    // one leaves the reserve linked in both existing lists.
                    let c = if keep_warm {
                        (*state.empty).empty_next
                    } else {
                        state.empty
                    };
                    if c.is_null() {
                        break;
                    }
                    state.remove_empty(c);
                    state.remove_available(c);
                    state.chunks -= 1;
                    (*c).empty_next = detached;
                    detached = c;
                    released += 1;
                }
            }
            class.locked.store(false, Ordering::Release);
            while !detached.is_null() {
                // SAFETY: privately owned detached chain, with no live blocks.
                unsafe {
                    let next = (*detached).empty_next;
                    backing::free(detached.cast());
                    detached = next;
                }
            }
            if released == limit {
                break;
            }
        }
        released * CHUNK_SIZE
    }

    /// Requested backing bytes currently owned by the small-block pools,
    /// including chunk headers, unused slots and rounding slack. Excludes
    /// System fallback allocations. Not live requested bytes or resident RAM.
    /// On Windows these are also committed/reserved chunk bytes. Sampling locks
    /// each class separately; concurrent changes can make this an approximate
    /// total. The counter changes only on chunk refill/release, not per block.
    /// What the blocks in use take, by their classes' sizes: what [`UiAlloc::retained_bytes`] holds less what's free in it.
    pub fn live_bytes(&self) -> usize {
        CLASSES
            .iter()
            .zip(CLASS_SIZES)
            .map(|(c, size)| c.with(|s| s.blocks * size))
            .sum()
    }

    pub fn retained_bytes(&self) -> usize {
        CLASSES
            .iter()
            .map(|c| c.with(|s| s.chunks * CHUNK_SIZE))
            .sum()
    }
}

// SAFETY: each small block belongs to exactly one CHUNK_SIZE-aligned chunk.
// Masking its address (preserving provenance) recovers that chunk's header.
// The class lock protects header links, free slots, carving offset and live
// count. Allocation increments live before unlock; free decrements only after
// the caller relinquishes access. Reclaim detaches only live==0 chunks under
// that same lock. No block headers, global allocator recursion, or scans occur
// in these operations. System fallbacks retain their original layouts.
unsafe impl GlobalAlloc for UiAlloc {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let Some(class) = class_of(layout) else {
            return unsafe { System.alloc(layout) };
        };
        let size = CLASS_SIZES[class];
        CLASSES[class].with(|state| {
            // SAFETY: class lock held; the available head has a free or uncarved
            // slot. New backing is aligned and initialized before publication.
            unsafe {
                let mut c = state.available;
                if c.is_null() {
                    c = backing::alloc().cast::<Chunk>();
                    if c.is_null() {
                        return ptr::null_mut();
                    }
                    c.write(Chunk {
                        free: ptr::null_mut(),
                        next_offset: HEADER_SIZE,
                        live: 0,
                        capacity: (CHUNK_SIZE - HEADER_SIZE) / size,
                        prev: ptr::null_mut(),
                        next: ptr::null_mut(),
                        empty_prev: ptr::null_mut(),
                        empty_next: ptr::null_mut(),
                    });
                    state.add_available(c);
                    state.chunks += 1;
                } else if (*c).live == 0 {
                    state.remove_empty(c);
                }
                let block = if !(*c).free.is_null() {
                    let b = (*c).free;
                    (*c).free = b.cast::<*mut u8>().read();
                    b
                } else {
                    let b = c.cast::<u8>().add((*c).next_offset);
                    (*c).next_offset += size;
                    b
                };
                (*c).live += 1;
                state.blocks += 1;
                if (*c).live == (*c).capacity {
                    state.remove_available(c);
                }
                block
            }
        })
    }

    #[inline]
    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        let Some(class) = class_of(layout) else {
            return unsafe { System.dealloc(block, layout) };
        };
        CLASSES[class].with(|state| {
            // SAFETY: caller provides a live small block and original layout.
            // The chunk alignment and provenance cover both header and block.
            unsafe {
                let c = block.map_addr(|a| a & !(CHUNK_SIZE - 1)).cast::<Chunk>();
                if (*c).live == (*c).capacity {
                    state.add_available(c);
                }
                block.cast::<*mut u8>().write((*c).free);
                (*c).free = block;
                (*c).live -= 1;
                state.blocks -= 1;
                if (*c).live == 0 {
                    state.add_empty(c);
                }
            }
        });
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if class_of(layout).is_none() {
            return unsafe { System.alloc_zeroed(layout) };
        }
        let block = unsafe { self.alloc(layout) };
        if !block.is_null() {
            unsafe { block.write_bytes(0, layout.size()) };
        }
        block
    }

    #[inline]
    unsafe fn realloc(&self, block: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller promises `new_size` is valid for this alignment.
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        match (class_of(layout), class_of(new_layout)) {
            (Some(old), Some(new)) if old == new => block,
            (None, None) => unsafe { System.realloc(block, layout, new_size) },
            _ => unsafe {
                let moved = self.alloc(new_layout);
                if !moved.is_null() {
                    ptr::copy_nonoverlapping(block, moved, layout.size().min(new_size));
                    self.dealloc(block, layout);
                }
                moved
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reclaim_skips_busy_class_without_waiting() {
        // This binary uses System globally. No live UiAlloc blocks exist.
        CLASSES[0].locked.store(true, Ordering::Release);
        let released = UiAlloc.reclaim(32);
        CLASSES[0].locked.store(false, Ordering::Release);
        assert_eq!(released, 0);
    }
}
