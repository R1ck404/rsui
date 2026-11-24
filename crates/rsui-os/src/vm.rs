//! Memory by the page: a range of addresses set aside first, with real memory asked for inside it as it's wanted and
//! given back when it isn't, as Windows' `VirtualAlloc` and `VirtualFree` do it.
//!
//! The one promise everything above relies on is that memory given back with [`decommit`] reads as zeros once it's
//! [`commit`]ted again. Windows does that by itself. Linux and Android do it with `MADV_DONTNEED`, which also drops the
//! pages from what the process is counted as using at once. macOS's `MADV_FREE` doesn't promise zeros, so there the
//! range is mapped again over itself, which does.
//!
//! What's given back stays readable and writable on Unix, as zeros, where Windows would fault on it. Taking that away
//! as well would cost a call to the system each time and split the mapping in two, and a program that gives back
//! many scattered pages, as the collector does, would soon have more mappings than Linux allows one process.
//!
//! Pages aren't always 4 KB: Apple's are 16 KB, and so are some Android phones' and the Raspberry Pi 5's. Callers think
//! in 4 KB steps, so [`commit`] widens a range to whole pages, and [`decommit`] gives back the whole pages inside a
//! range and writes zeros over the rest of it, which keeps the promise for any size of page.

/// The size of a page, the most that a range is ever widened by.
#[inline]
pub fn page_size() -> usize {
    imp::page_size()
}

/// Sets aside `size` bytes of addresses, with no memory behind them yet. `None` if there are no addresses left.
#[inline]
pub fn reserve(size: usize) -> Option<*mut u8> {
    imp::reserve(size)
}

/// Gives real memory to `size` bytes from `at`, inside what [`reserve`] set aside. What's there already is left as it
/// is; what's new reads as zeros. Says whether there was memory for it.
///
/// # Safety
/// `at..at + size` must be inside one range from [`reserve`] that hasn't been [`release`]d.
#[inline]
pub unsafe fn commit(at: *mut u8, size: usize) -> bool {
    // SAFETY: as the caller promises.
    unsafe { imp::commit(at, size) }
}

/// Gives back the memory of `size` bytes from `at`, which read as zeros from then on.
///
/// # Safety
/// `at..at + size` must be inside one range from [`reserve`], and nothing may still be using what's in it.
#[inline]
pub unsafe fn decommit(at: *mut u8, size: usize) {
    // SAFETY: as the caller promises.
    unsafe { imp::decommit(at, size) }
}

/// Gives back the whole of a range from [`reserve`], addresses and all.
///
/// # Safety
/// `at` and `size` must be what [`reserve`] was given and gave, and nothing may use the range again.
#[inline]
pub unsafe fn release(at: *mut u8, size: usize) {
    // SAFETY: as the caller promises.
    unsafe { imp::release(at, size) }
}

/// `size` bytes of memory at once, starting at a multiple of `align` (a power of two), as [`reserve`] and [`commit`]
/// together would give. It's given back with [`release_aligned`].
#[inline]
pub fn alloc_aligned(size: usize, align: usize) -> Option<*mut u8> {
    imp::alloc_aligned(size, align)
}

/// Gives back what [`alloc_aligned`] gave.
///
/// # Safety
/// `at` and `size` must be what [`alloc_aligned`] was given and gave, and nothing may use it again.
#[inline]
pub unsafe fn release_aligned(at: *mut u8, size: usize) {
    // SAFETY: as the caller promises.
    unsafe { imp::release_aligned(at, size) }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RESERVE: u32 = 0x2000;
    const MEM_DECOMMIT: u32 = 0x4000;
    const MEM_RELEASE: u32 = 0x8000;
    const PAGE_NOACCESS: u32 = 1;
    const PAGE_READWRITE: u32 = 4;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
        fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
    }

    pub fn page_size() -> usize {
        4096
    }

    pub fn reserve(size: usize) -> Option<*mut u8> {
        // SAFETY: asking for a new range of addresses has no preconditions.
        let at = unsafe { VirtualAlloc(std::ptr::null_mut(), size, MEM_RESERVE, PAGE_NOACCESS) };
        (!at.is_null()).then_some(at.cast())
    }

    pub unsafe fn commit(at: *mut u8, size: usize) -> bool {
        // SAFETY: the caller says the range is inside a reservation.
        !unsafe { VirtualAlloc(at.cast(), size, MEM_COMMIT, PAGE_READWRITE) }.is_null()
    }

    pub unsafe fn decommit(at: *mut u8, size: usize) {
        // SAFETY: the caller says the range is inside a reservation and unused.
        unsafe { VirtualFree(at.cast(), size, MEM_DECOMMIT) };
    }

    pub unsafe fn release(at: *mut u8, _size: usize) {
        // SAFETY: the caller says this is a whole reservation.
        unsafe { VirtualFree(at.cast(), 0, MEM_RELEASE) };
    }

    pub fn alloc_aligned(size: usize, align: usize) -> Option<*mut u8> {
        // Windows hands out addresses in steps of 64 KB, so anything up to that comes aligned already.
        assert!(align <= 64 * 1024, "Windows aligns to 64 KB at most by itself");
        // SAFETY: a fresh range has no preconditions.
        let at = unsafe { VirtualAlloc(std::ptr::null_mut(), size, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
        (!at.is_null()).then_some(at.cast())
    }

    pub unsafe fn release_aligned(at: *mut u8, size: usize) {
        // SAFETY: as for `release`.
        unsafe { release(at, size) }
    }
}

#[cfg(unix)]
mod imp {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static PAGE: AtomicUsize = AtomicUsize::new(0);

    #[inline]
    pub fn page_size() -> usize {
        let page = PAGE.load(Ordering::Relaxed);
        if page != 0 {
            return page;
        }
        // SAFETY: `sysconf` has no preconditions.
        let page = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap_or(4096).max(4096);
        PAGE.store(page, Ordering::Relaxed);
        page
    }

    /// The whole pages that `at..at + size` touches.
    fn outward(at: *mut u8, size: usize) -> (usize, usize) {
        let page = page_size();
        let start = at as usize & !(page - 1);
        let end = (at as usize + size).next_multiple_of(page);
        (start, end)
    }

    /// The whole pages inside `at..at + size`, which may be none.
    fn inward(at: *mut u8, size: usize) -> (usize, usize) {
        let page = page_size();
        let start = (at as usize).next_multiple_of(page);
        let end = (at as usize + size) & !(page - 1);
        (start, end.max(start))
    }

    pub fn reserve(size: usize) -> Option<*mut u8> {
        // SAFETY: a new anonymous mapping has no preconditions.
        let at = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANON | NORESERVE,
                -1,
                0,
            )
        };
        (at != libc::MAP_FAILED).then_some(at.cast())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    const NORESERVE: libc::c_int = libc::MAP_NORESERVE;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const NORESERVE: libc::c_int = 0;

    pub unsafe fn commit(at: *mut u8, size: usize) -> bool {
        if size == 0 {
            return true;
        }
        let (start, end) = outward(at, size);
        // SAFETY: the caller says the range is inside a reservation, and a reservation is whole pages, so widening to
        // whole pages stays inside it.
        unsafe { libc::mprotect(start as *mut libc::c_void, end - start, libc::PROT_READ | libc::PROT_WRITE) == 0 }
    }

    pub unsafe fn decommit(at: *mut u8, size: usize) {
        if size == 0 {
            return;
        }
        let (start, end) = inward(at, size);
        if end > start {
            // SAFETY: whole pages inside a range the caller says is unused.
            unsafe { forget(start as *mut u8, end - start) };
            // The parts of pages at either end are still in use by something else on the same page, so they're
            // written over instead.
            // SAFETY: as above; the range was committed, so it can be written.
            unsafe {
                std::ptr::write_bytes(at, 0, start - at as usize);
                std::ptr::write_bytes(end as *mut u8, 0, at as usize + size - end);
            }
        } else {
            // SAFETY: as above.
            unsafe { std::ptr::write_bytes(at, 0, size) };
        }
    }

    /// Drops whole pages, which read as zeros from then on.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    unsafe fn forget(at: *mut u8, size: usize) {
        // SAFETY: the caller passes whole unused pages of a mapping.
        unsafe { libc::madvise(at.cast(), size, libc::MADV_DONTNEED) };
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    unsafe fn forget(at: *mut u8, size: usize) {
        // A new mapping over the old one, which reads as zeros and has no pages yet.
        // SAFETY: the caller passes whole unused pages of a mapping of ours; `MAP_FIXED` replaces just those.
        let again = unsafe {
            libc::mmap(
                at.cast(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_FIXED,
                -1,
                0,
            )
        };
        if again == libc::MAP_FAILED {
            // Couldn't map again, so the zeros are written instead.
            // SAFETY: the pages are still ours and writable.
            unsafe { std::ptr::write_bytes(at, 0, size) };
        }
    }

    pub unsafe fn release(at: *mut u8, size: usize) {
        // SAFETY: the caller says this is a whole reservation.
        unsafe { libc::munmap(at.cast(), size) };
    }

    pub fn alloc_aligned(size: usize, align: usize) -> Option<*mut u8> {
        assert!(align.is_power_of_two());
        let page = page_size();
        let size = size.next_multiple_of(page);
        if align <= page {
            // SAFETY: a new anonymous mapping has no preconditions.
            let at = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    size,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                )
            };
            return (at != libc::MAP_FAILED).then_some(at.cast());
        }
        // More than is wanted, then what's before and after the aligned part is given back.
        let over = size + align - page;
        // SAFETY: as above.
        let at = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                over,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if at == libc::MAP_FAILED {
            return None;
        }
        let start = at as usize;
        let aligned = start.next_multiple_of(align);
        // SAFETY: both ranges are whole pages of the mapping just made, outside the part kept.
        unsafe {
            if aligned > start {
                libc::munmap(at, aligned - start);
            }
            let tail = start + over - (aligned + size);
            if tail > 0 {
                libc::munmap((aligned + size) as *mut libc::c_void, tail);
            }
        }
        Some(aligned as *mut u8)
    }

    pub unsafe fn release_aligned(at: *mut u8, size: usize) {
        // SAFETY: the caller says this is what `alloc_aligned` gave, which was rounded to whole pages the same way.
        unsafe { libc::munmap(at.cast(), size.next_multiple_of(page_size())) };
    }
}

/// Windows' own two calls for this, as Unix can do them, for code written against them first. That code keeps calling
/// `VirtualAlloc` and `VirtualFree` as it always has, so what it compiles to on Windows doesn't change at all, and
/// elsewhere it takes these instead. Only what rsui uses is here: reserving, committing as read and write, decommitting
/// and releasing. Releasing goes by where a reservation starts alone, as on Windows, so each one's size is kept until
/// then.
#[cfg(unix)]
#[allow(non_snake_case)]
pub mod windows_like {
    use std::collections::BTreeMap;
    use std::ffi::c_void;
    use std::sync::Mutex;

    pub const MEM_COMMIT: u32 = 0x1000;
    pub const MEM_RESERVE: u32 = 0x2000;
    pub const MEM_DECOMMIT: u32 = 0x4000;
    pub const MEM_RELEASE: u32 = 0x8000;

    /// Where each reservation starts, and how big it is.
    static SIZES: Mutex<BTreeMap<usize, usize>> = Mutex::new(BTreeMap::new());

    /// # Safety
    /// As Windows says for `VirtualAlloc`, for the kinds above.
    pub unsafe fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, _protect: u32) -> *mut c_void {
        if kind & MEM_RESERVE != 0 {
            let Some(at) = super::reserve(size) else { return std::ptr::null_mut() };
            // SAFETY: the whole of what was just reserved.
            if kind & MEM_COMMIT != 0 && !unsafe { super::commit(at, size) } {
                // SAFETY: as above.
                unsafe { super::release(at, size) };
                return std::ptr::null_mut();
            }
            SIZES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(at as usize, size);
            return at.cast();
        }
        if kind & MEM_COMMIT != 0 {
            // SAFETY: the caller says it's inside a reservation.
            return if unsafe { super::commit(address.cast(), size) } { address } else { std::ptr::null_mut() };
        }
        std::ptr::null_mut()
    }

    /// # Safety
    /// As Windows says for `VirtualFree`, for the kinds above.
    pub unsafe fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32 {
        if kind & MEM_DECOMMIT != 0 {
            // SAFETY: the caller says it's inside a reservation and unused.
            unsafe { super::decommit(address.cast(), size) };
            return 1;
        }
        if kind & MEM_RELEASE != 0 {
            let size = SIZES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&(address as usize));
            if let Some(size) = size {
                // SAFETY: that's the reservation, whole.
                unsafe { super::release(address.cast(), size) };
                return 1;
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_given_back_reads_as_zeros_when_it_is_wanted_again() {
        let size = 1 << 20;
        let at = reserve(size).expect("addresses");
        // SAFETY: inside the reservation, which nothing else uses.
        unsafe {
            assert!(commit(at, size));
            std::ptr::write_bytes(at, 0xab, size);
            decommit(at.add(65536), 3 * 65536);
            assert!(commit(at, size));
            let all = std::slice::from_raw_parts(at, size);
            assert!(all[..65536].iter().all(|&b| b == 0xab));
            assert!(all[65536..4 * 65536].iter().all(|&b| b == 0));
            assert!(all[4 * 65536..].iter().all(|&b| b == 0xab));
            release(at, size);
        }
    }

    /// What's given back can start and end partway through a page, as the callers' 4 KB steps can on bigger pages. Only
    /// that range reads as zeros after. Windows always gives back whole pages, so this is for the others.
    #[test]
    #[cfg(unix)]
    fn part_of_a_page_given_back_reads_as_zeros_and_the_rest_stays() {
        let size = 1 << 20;
        let at = reserve(size).expect("addresses");
        // SAFETY: inside the reservation, which nothing else uses.
        unsafe {
            assert!(commit(at, size));
            std::ptr::write_bytes(at, 0xab, size);
            decommit(at.add(4096 + 100), 3 * 65536);
            assert!(commit(at, size));
            let all = std::slice::from_raw_parts(at, size);
            assert!(all[..4196].iter().all(|&b| b == 0xab));
            assert!(all[4196..4196 + 3 * 65536].iter().all(|&b| b == 0));
            assert!(all[4196 + 3 * 65536..].iter().all(|&b| b == 0xab));
            release(at, size);
        }
    }

    #[test]
    fn aligned_memory_is_aligned_and_usable() {
        for _ in 0..8 {
            let at = alloc_aligned(64 * 1024, 64 * 1024).expect("memory");
            assert_eq!(at as usize % (64 * 1024), 0);
            // SAFETY: just given, and given back after.
            unsafe {
                at.write_bytes(1, 64 * 1024);
                release_aligned(at, 64 * 1024);
            }
        }
    }
}
