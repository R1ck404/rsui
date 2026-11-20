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

/// Where the process's memory is, region by region, for `regions` in steps: each reservation of addresses is put down to
/// what it's for (a module, a heap, a thread's stack, one of rsui-js's lists, rsui-alloc's chunks), with how much of it is
/// committed, how much is in the working set and how much of that is the process's alone. Then each heap's blocks in
/// use by size.
#[cfg(windows)]
#[allow(unsafe_code)]
pub(crate) fn regions() -> Vec<String> {
    use std::collections::HashMap;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Basic {
        base: usize,
        allocation_base: usize,
        allocation_protect: u32,
        partition: u16,
        region_size: usize,
        state: u32,
        protect: u32,
        kind: u32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct HeapEntry {
        data: usize,
        size: u32,
        overhead: u8,
        region_index: u8,
        flags: u16,
        rest: [usize; 3],
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WsEx {
        address: usize,
        attributes: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn VirtualQuery(address: usize, info: *mut Basic, size: usize) -> usize;
        fn GetProcessHeaps(count: u32, heaps: *mut isize) -> u32;
        fn HeapLock(heap: isize) -> i32;
        fn HeapUnlock(heap: isize) -> i32;
        fn HeapWalk(heap: isize, entry: *mut HeapEntry) -> i32;
        fn GetProcessHeap() -> *const u8;
        fn GetModuleFileNameW(module: usize, name: *mut u16, size: u32) -> u32;
        fn K32GetMappedFileNameW(process: isize, address: usize, name: *mut u16, size: u32) -> u32;
        fn K32QueryWorkingSetEx(process: isize, info: *mut WsEx, size: u32) -> i32;
    }
    const MEM_COMMIT: u32 = 0x1000;
    const MEM_IMAGE: u32 = 0x100_0000;
    const MEM_MAPPED: u32 = 0x4_0000;
    const PAGE_GUARD: u32 = 0x100;
    const PAGE: usize = 4096;

    #[derive(Default)]
    struct Sum {
        committed: usize,
        resident: usize,
        own: usize,
        reservations: usize,
        writable: usize,
    }

    let query = |address: usize| -> Option<Basic> {
        let mut info = Basic::default();
        // SAFETY: the buffer is MEMORY_BASIC_INFORMATION's size.
        let got = unsafe { VirtualQuery(address, &mut info, size_of::<Basic>()) };
        (got != 0).then_some(info)
    };
    let wide = |buffer: &[u16], length: u32| -> String {
        let name = String::from_utf16_lossy(&buffer[..length as usize]);
        name.rsplit('\\').next().unwrap_or(&name).to_owned()
    };

    // Which reservations are which heap's, and each heap's blocks in use, by size.
    let mut labels: HashMap<usize, String> = HashMap::new();
    let mut heap_lines = Vec::new();
    let mut heaps = vec![0isize; 64];
    // SAFETY: the buffer has room for as many as it says.
    let count = unsafe { GetProcessHeaps(heaps.len() as u32, heaps.as_mut_ptr()) } as usize;
    // SAFETY: no preconditions.
    let process_heap = unsafe { GetProcessHeap() } as isize;
    let names = ["<=1K", "<=4K", "<=16K", "<=64K", "<=256K", "<=1M", "<=4M", ">4M"];
    for (index, heap) in heaps.iter().copied().take(count.min(64)).enumerate() {
        let name = if heap == process_heap { "process heap".to_owned() } else { format!("heap {index}") };
        let mut buckets = [(0usize, 0usize); 8];
        let mut overhead = 0usize;
        let mut last_base = 0usize;
        // SAFETY: a heap of the process's, locked while it's walked, with an entry zeroed to start.
        unsafe {
            if HeapLock(heap) == 0 {
                continue;
            }
            let mut entry: HeapEntry = std::mem::zeroed();
            while HeapWalk(heap, &mut entry) != 0 {
                if entry.data != 0 && (entry.data < last_base || entry.data >= last_base + 64 * 1024) {
                    if let Some(info) = query(entry.data) {
                        labels.entry(info.allocation_base).or_insert_with(|| name.clone());
                        last_base = info.allocation_base;
                    }
                }
                if entry.flags & 0x4 != 0 {
                    let size = entry.size as usize;
                    let bucket = match size {
                        0..=1024 => 0,
                        1025..=4096 => 1,
                        4097..=16384 => 2,
                        16385..=65536 => 3,
                        65537..=262_144 => 4,
                        262_145..=1_048_576 => 5,
                        1_048_577..=4_194_304 => 6,
                        _ => 7,
                    };
                    buckets[bucket].0 += 1;
                    buckets[bucket].1 += size;
                    overhead += entry.overhead as usize;
                }
            }
            HeapUnlock(heap);
        }
        let total: usize = buckets.iter().map(|bucket| bucket.1).sum();
        let mut line = format!("{name}: {} KB in use, {} KB overhead;", total / 1024, overhead / 1024);
        for (bucket, (blocks, bytes)) in buckets.iter().enumerate() {
            if *blocks > 0 {
                line += &format!(" {} {}x {} KB", names[bucket], blocks, bytes / 1024);
            }
        }
        heap_lines.push(line);
    }
    for (base, label) in rsui::alloc_regions::noted() {
        if let Some(info) = query(base) {
            labels.insert(info.allocation_base, format!("rsui-js {}", label.rsplit("::").next().unwrap_or(label)));
        }
    }

    // Each committed region, with its pages' working-set bits.
    let mut by_label: HashMap<String, Sum> = HashMap::new();
    let mut seen_bases = std::collections::HashSet::new();
    let mut address = 0usize;
    let mut name = vec![0u16; 1024];
    while let Some(info) = query(address) {
        let next = info.base + info.region_size;
        if info.state == MEM_COMMIT {
            let label = if let Some(label) = labels.get(&info.allocation_base).filter(|_| info.kind == MEM_MAPPED) {
                label.clone()
            } else if info.kind == MEM_IMAGE {
                // SAFETY: the buffer is as long as it says.
                let length = unsafe { GetModuleFileNameW(info.allocation_base, name.as_mut_ptr(), name.len() as u32) };
                format!("image {}", wide(&name, length))
            } else if info.kind == MEM_MAPPED {
                // SAFETY: as above.
                let length = unsafe {
                    K32GetMappedFileNameW(GetCurrentProcess(), info.base, name.as_mut_ptr(), name.len() as u32)
                };
                if length == 0 { "mapped (pagefile section)".to_owned() } else { format!("mapped {}", wide(&name, length)) }
            } else if let Some(label) = labels.get(&info.allocation_base) {
                label.clone()
            } else {
                // A reservation with a guard page in it is a thread's stack; 64 KB ones on their own are rsui-alloc's chunks.
                let mut at = info.allocation_base;
                let mut size = 0;
                let mut guard = false;
                while let Some(part) = query(at) {
                    if part.allocation_base != info.allocation_base {
                        break;
                    }
                    guard |= part.protect & PAGE_GUARD != 0;
                    size += part.region_size;
                    at = part.base + part.region_size;
                }
                let label = if guard {
                    "thread stacks".to_owned()
                } else if size == 64 * 1024 && info.allocation_base % (64 * 1024) == 0 {
                    "rsui-alloc chunks (64 KB reservations)".to_owned()
                } else {
                    let mb = size / (1024 * 1024);
                    let size = if mb == 0 { format!("{} KB", size / 1024) } else { format!("{mb} MB") };
                    format!("other private, reservations of {size}")
                };
                labels.insert(info.allocation_base, label.clone());
                label
            };
            let sum = by_label.entry(label).or_default();
            sum.committed += info.region_size;
            if seen_bases.insert(info.allocation_base) {
                sum.reservations += 1;
            }
            if info.protect & 0xCC != 0 {
                // PAGE_READWRITE, PAGE_WRITECOPY, PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY.
                sum.writable += info.region_size;
            }
            let pages = info.region_size / PAGE;
            let mut ws: Vec<WsEx> = (0..pages).map(|page| WsEx { address: info.base + page * PAGE, attributes: 0 }).collect();
            // SAFETY: the buffer is as long as it says.
            let ok = unsafe {
                K32QueryWorkingSetEx(GetCurrentProcess(), ws.as_mut_ptr(), (ws.len() * size_of::<WsEx>()) as u32)
            };
            if ok != 0 {
                for page in &ws {
                    if page.attributes & 1 != 0 {
                        sum.resident += PAGE;
                        if page.attributes & (1 << 15) == 0 {
                            sum.own += PAGE;
                        }
                    }
                }
            }
        }
        if next <= address {
            break;
        }
        address = next;
    }
    let mut rows: Vec<(String, Sum)> = by_label.into_iter().collect();
    rows.sort_by(|a, b| b.1.committed.cmp(&a.1.committed));
    let kb = |bytes: usize| bytes / 1024;
    let mut lines = Vec::new();
    let (mut committed, mut resident, mut own, mut private) = (0, 0, 0, 0);
    for (label, sum) in &rows {
        committed += sum.committed;
        resident += sum.resident;
        own += sum.own;
        if !label.starts_with("image") && !label.starts_with("mapped ") {
            private += sum.committed;
        }
        lines.push(format!(
            "region {label}: {} KB committed ({} KB writable), {} KB in the working set, {} KB of it private, {} reservations",
            kb(sum.committed),
            kb(sum.writable),
            kb(sum.resident),
            kb(sum.own),
            sum.reservations
        ));
    }
    lines.push(format!(
        "regions in all: {} KB committed, {} KB of it private regions, {} KB in the working set, {} KB of it private",
        kb(committed),
        kb(private),
        kb(resident),
        kb(own)
    ));
    lines.extend(heap_lines);
    lines
}

#[cfg(not(windows))]
pub(crate) fn regions() -> Vec<String> {
    Vec::new()
}
