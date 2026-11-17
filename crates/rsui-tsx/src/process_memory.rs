//! What this process takes from the system, by its own counters: for `memory` in steps.

/// Bytes: the working set now and at its peak, the part of it that's this process's alone, and what it has committed
/// (private bytes) now and at its peak.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ProcessMemory {
    pub working_set: usize,
    pub peak_working_set: usize,
    pub private_working_set: usize,
    pub private: usize,
    pub peak_private: usize,
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub(crate) fn now() -> ProcessMemory {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged: usize,
        quota_paged: usize,
        quota_peak_non_paged: usize,
        quota_non_paged: usize,
        pagefile: usize,
        peak_pagefile: usize,
        private: usize,
        private_working_set: usize,
        shared_commit: u64,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, size: u32) -> i32;
    }
    let mut counters = Counters { cb: size_of::<Counters>() as u32, ..Default::default() };
    // SAFETY: the counters are the size they say, which is PROCESS_MEMORY_COUNTERS_EX2's.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    if ok == 0 {
        return ProcessMemory::default();
    }
    ProcessMemory {
        working_set: counters.working_set,
        peak_working_set: counters.peak_working_set,
        private_working_set: counters.private_working_set,
        private: counters.private,
        peak_private: counters.peak_pagefile,
    }
}

#[cfg(not(windows))]
pub(crate) fn now() -> ProcessMemory {
    ProcessMemory::default()
}

/// Which heap the process heap is (the segment heap, when the program's manifest asks for it, or the older NT heap), by the
/// signature in its header, and what's allocated and committed in it.
#[cfg(windows)]
#[allow(unsafe_code)]
pub(crate) fn process_heap() -> (&'static str, usize, usize) {
    #[repr(C)]
    #[derive(Default)]
    struct Summary {
        cb: u32,
        allocated: usize,
        committed: usize,
        reserved: usize,
        max_reserve: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetProcessHeap() -> *const u8;
        fn HeapSummary(heap: *const u8, flags: u32, summary: *mut Summary) -> i32;
    }
    // SAFETY: the process heap's header is mapped for as long as the process is, and both kinds have a signature 16 bytes in:
    // 0xFFEEFFEE for the NT heap's first segment, 0xDDEEDDEE for the segment heap.
    let (kind, summary) = unsafe {
        let heap = GetProcessHeap();
        let signature = heap.add(16).cast::<u32>().read_unaligned();
        let kind = match signature {
            0xFFEE_FFEE => "nt",
            0xDDEE_DDEE => "segment",
            _ => "unknown",
        };
        let mut summary = Summary { cb: size_of::<Summary>() as u32, ..Default::default() };
        HeapSummary(heap, 0, &mut summary);
        (kind, summary)
    };
    (kind, summary.allocated, summary.committed)
}

#[cfg(not(windows))]
pub(crate) fn process_heap() -> (&'static str, usize, usize) {
    ("none", 0, 0)
}
