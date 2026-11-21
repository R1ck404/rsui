//! Where compiled code lives on Linux and Android: as on Windows (`memory.rs`), one range of addresses set aside up front,
//! mapped twice, read-write where code is written and read-execute where it runs, so no address is ever both and adding
//! code changes no page's protection. The memory is a `memfd`, an anonymous file, which is what lets it be mapped twice;
//! its pages come into being as they're written, so nothing has to be committed first.
//!
//! A system that won't map that memory as executable (an SELinux policy can refuse it) gets no JIT, and the interpreter
//! runs everything, as with `RSUI_JIT=0`. The optimizing tier's feedback isn't here yet, so there's none recorded.
#![allow(unsafe_code)]

use std::ffi::c_void;

/// How much memory is counted as given to the code at a time, as on Windows.
const CHUNK: usize = 64 * 1024;

/// The memory itself, mapped twice, which goes once nothing holds it.
pub(crate) struct Views {
    fd: i32,
    base: *mut u8,
    writable: *mut u8,
    size: usize,
}

// SAFETY: it's only addresses, which whoever holds it can run code from; only the one `CodeMemory` writes through them.
unsafe impl Send for Views {}
unsafe impl Sync for Views {}

impl Drop for Views {
    fn drop(&mut self) {
        // SAFETY: the views and the file `CodeMemory::new` made, which nothing runs from once nothing holds them.
        unsafe {
            libc::munmap(self.base.cast(), self.size);
            libc::munmap(self.writable.cast(), self.size);
            rsui_alloc::regions::forget(self.base.cast());
            rsui_alloc::regions::forget(self.writable.cast());
            libc::close(self.fd);
        }
    }
}

pub(crate) struct CodeMemory {
    views: std::sync::Arc<Views>,
    base: *mut u8,
    writable: *mut u8,
    reserved: usize,
    /// How much of the memory has had code written to it, in chunks, as Windows counts what it's committed.
    committed: usize,
    used: usize,
}

impl CodeMemory {
    /// Sets aside room for this many bytes of code. `None` if the system won't have it.
    pub(crate) fn new(reserved: usize) -> Option<Self> {
        let reserved = reserved.next_multiple_of(CHUNK);
        // SAFETY: a new anonymous file, sized and mapped twice, each step checked and undone if a later one fails.
        // `memfd_create` is asked for by its number, as Android's C library only has it from Android 11.
        unsafe {
            let fd = libc::syscall(libc::SYS_memfd_create, c"rsui-jit".as_ptr(), libc::MFD_CLOEXEC) as i32;
            if fd < 0 {
                return None;
            }
            if libc::ftruncate(fd, reserved as libc::off_t) != 0 {
                libc::close(fd);
                return None;
            }
            let map = |protection| {
                let at = libc::mmap(std::ptr::null_mut(), reserved, protection, libc::MAP_SHARED, fd, 0);
                (at != libc::MAP_FAILED).then_some(at.cast::<u8>())
            };
            let writable = map(libc::PROT_READ | libc::PROT_WRITE);
            let base = map(libc::PROT_READ | libc::PROT_EXEC);
            let (Some(writable), Some(base)) = (writable, base) else {
                for view in [writable, base].into_iter().flatten() {
                    libc::munmap(view.cast::<c_void>(), reserved);
                }
                libc::close(fd);
                return None;
            };
            rsui_alloc::regions::note(writable.cast(), "jit code, written through");
            rsui_alloc::regions::note(base.cast(), "jit code, run from");
            Some(Self {
                views: std::sync::Arc::new(Views { fd, base, writable, size: reserved }),
                base,
                writable,
                reserved,
                committed: 0,
                used: 0,
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

    /// The offset code would go at, at the start of a cache line, as on Windows.
    pub(crate) fn next_offset(&self) -> usize {
        self.used.next_multiple_of(64)
    }

    pub(crate) fn has_room(&self, size: usize) -> bool {
        self.next_offset() + size <= self.reserved
    }

    /// Copies code in at [`next_offset`](Self::next_offset), and says where it went. x86-64 sees what's written through
    /// one view in the other at once, so nothing more is needed for it to run; Arm's instruction cache doesn't, so it's
    /// told (see [`sync_code`]).
    pub(crate) fn add(&mut self, code: &[u8]) -> Option<usize> {
        let at = self.next_offset();
        let end = at + code.len();
        if end > self.reserved {
            return None;
        }
        // SAFETY: inside the range both views map, past any code that's run.
        unsafe { std::ptr::copy_nonoverlapping(code.as_ptr(), self.writable.add(at), code.len()) };
        #[cfg(target_arch = "aarch64")]
        // SAFETY: both ranges are inside the views, and what's just been written.
        unsafe {
            sync_code(self.writable.add(at), self.base.add(at), code.len())
        };
        self.committed = self.committed.max(end.next_multiple_of(CHUNK).min(self.reserved));
        self.used = end;
        Some(at)
    }

    /// There's no feedback here yet.
    #[cfg(feature = "jit-opt")]
    #[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
    pub(crate) fn feedback(&mut self, _size: usize) -> Option<usize> {
        None
    }
}

/// Has the instruction cache see code just written at `written` (through the writable view), which runs at `run` (the
/// executable view): the data cache's lines are cleaned out to where instructions are fetched from, then the
/// instruction cache's lines for where it runs are thrown away, on every core (`ish`), as the Arm ARM has it for
/// self-modifying code. The core that runs it then wants an `isb` (which this one has here, and others in
/// `Vm::jit_collect`). The lines' sizes are what CTR_EL0 says, which Linux lets a program read.
///
/// # Safety
///
/// Both ranges have to be mapped.
#[cfg(target_arch = "aarch64")]
unsafe fn sync_code(written: *const u8, run: *const u8, length: usize) {
    let ctr: u64;
    // SAFETY: reading the cache type register, which the kernel allows (or emulates) at EL0.
    unsafe { std::arch::asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack, preserves_flags)) };
    let data_line = 4usize << ((ctr >> 16) & 0xF);
    let code_line = 4usize << (ctr & 0xF);
    // IDC: the data cache needn't be cleaned for the instruction cache to see it. DIC: nor the instruction cache
    // thrown away.
    let idc = ctr & (1 << 28) != 0;
    let dic = ctr & (1 << 29) != 0;
    // SAFETY: cache maintenance by address on memory that's mapped, and barriers.
    unsafe {
        if !idc {
            let mut at = written as usize & !(data_line - 1);
            while at < written as usize + length {
                std::arch::asm!("dc cvau, {}", in(reg) at, options(nostack, preserves_flags));
                at += data_line;
            }
        }
        std::arch::asm!("dsb ish", options(nostack, preserves_flags));
        if !dic {
            let mut at = run as usize & !(code_line - 1);
            while at < run as usize + length {
                std::arch::asm!("ic ivau, {}", in(reg) at, options(nostack, preserves_flags));
                at += code_line;
            }
            std::arch::asm!("dsb ish", options(nostack, preserves_flags));
        }
        std::arch::asm!("isb", options(nostack, preserves_flags));
    }
}
