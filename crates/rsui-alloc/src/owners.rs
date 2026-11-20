//! Which part of the program is holding how much memory, for finding out where it goes. With the `owners` feature, every
//! allocation is counted against what was running when it was made: the stage a [`Scope`] says, the innermost one. It's
//! counted back when it's freed, wherever that is. Without the feature, scopes are nothing and nothing's counted.
//!
//! It's only for measuring: [`Owned`] puts 16 bytes or more in front of every block, to say whose it is.

/// What a block was made for, by the stage that made it.
pub const OTHER: u8 = 0;
/// JavaScript running: the engine's objects, strings and tables.
pub const JS: u8 = 1;
/// rsui/web's natives in Rust, which JavaScript asks about CSS: `layout.rs` in rsui-tsx.
pub const WEB: u8 = 2;
/// The runtime turning what components gave into the tree of elements: instances, elements and their props.
pub const TREE: u8 = 3;
/// The window's layout: its boxes.
pub const LAYOUT: u8 = 4;
/// Measuring and shaping text for the window's layout.
pub const TEXT: u8 = 5;
/// The layout kept for code that asks where something is before the window has laid it out.
pub const GEOMETRY: u8 = 6;
/// Text measured for that layout, with its own text engine.
pub const GEOMETRY_TEXT: u8 = 7;
/// Turning the laid-out tree into what's drawn.
pub const SCENE: u8 = 8;

/// The names of the stages, by their number.
pub const NAMES: [&str; 9] = ["other", "js", "web natives", "tree", "layout", "text", "geometry", "geometry text", "scene"];

#[cfg(feature = "owners")]
mod counting {
    use std::alloc::{GlobalAlloc, Layout};
    use std::cell::Cell;
    use std::sync::atomic::{AtomicIsize, Ordering};

    use crate::UiAlloc;

    thread_local! {
        static NOW: Cell<u8> = const { Cell::new(0) };
    }

    #[allow(clippy::declare_interior_mutable_const)]
    const ZERO: AtomicIsize = AtomicIsize::new(0);
    static LIVE: [AtomicIsize; 16] = [ZERO; 16];

    pub(super) fn now() -> u8 {
        NOW.try_with(Cell::get).unwrap_or(0)
    }

    pub(super) fn replace(owner: u8) -> u8 {
        NOW.try_with(|now| now.replace(owner)).unwrap_or(0)
    }

    pub(super) fn live() -> [isize; 16] {
        std::array::from_fn(|at| LIVE[at].load(Ordering::Relaxed))
    }

    /// The allocator, with whose each block is in front of it.
    pub struct Owned;

    const HEADER: usize = 16;

    fn widened(layout: Layout) -> (Layout, usize) {
        let front = layout.align().max(HEADER);
        // SAFETY: the alignment is a power of two, and the size can't overflow for any block that could be made.
        (unsafe { Layout::from_size_align_unchecked(layout.size() + front, layout.align().max(HEADER)) }, front)
    }

    // SAFETY: each block is UiAlloc's with room in front, which is given back with the same layout.
    unsafe impl GlobalAlloc for Owned {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let (wide, front) = widened(layout);
            let block = unsafe { UiAlloc.alloc(wide) };
            if block.is_null() {
                return block;
            }
            let owner = now();
            unsafe {
                let given = block.add(front);
                let sampled = sites::maybe_sample(given, owner, layout.size());
                given.sub(HEADER).cast::<usize>().write(owner as usize | if sampled { SAMPLED } else { 0 });
                LIVE[owner as usize & 15].fetch_add(layout.size() as isize, Ordering::Relaxed);
                given
            }
        }

        unsafe fn dealloc(&self, given: *mut u8, layout: Layout) {
            let (wide, front) = widened(layout);
            unsafe {
                let word = given.sub(HEADER).cast::<usize>().read();
                if word & SAMPLED != 0 {
                    sites::forget(given);
                }
                LIVE[word & 15].fetch_sub(layout.size() as isize, Ordering::Relaxed);
                UiAlloc.dealloc(given.sub(front), wide);
            }
        }
    }

    const SAMPLED: usize = 1 << 8;

    /// Where a sample of blocks were made, by the calls that made them, for telling what holds memory by the code that made it:
    /// one block in every [`RATE`], from when [`start`] is called. Names come from the program's symbols, when they're there.
    pub(super) mod sites {
        use std::cell::Cell;
        use std::collections::HashMap;
        use std::ffi::c_void;
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        pub(crate) const RATE: usize = 61;
        /// Blocks this big or bigger are all kept, counted as themselves rather than as standing for [`RATE`] blocks.
        const BIG: usize = 64 * 1024;
        const DEPTH: usize = 28;

        static ON: AtomicBool = AtomicBool::new(false);
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        static KEPT: Mutex<Option<Kept>> = Mutex::new(None);

        thread_local! {
            static INSIDE: Cell<bool> = const { Cell::new(false) };
        }

        #[derive(Default)]
        struct Kept {
            live: HashMap<usize, (u32, usize, usize)>,
            stacks: HashMap<([usize; DEPTH], u8), u32>,
            /// Each stack, with its owner, the bytes it has live, and how many.
            each: Vec<([usize; DEPTH], u8, isize, isize)>,
        }

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn RtlCaptureStackBackTrace(skip: u32, count: u32, frames: *mut *mut c_void, hash: *mut u32) -> u16;
            fn GetCurrentProcess() -> *mut c_void;
            fn GetLastError() -> u32;
            fn GetModuleHandleW(name: *const u16) -> *mut c_void;
            fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, size: u32) -> u32;
        }

        // From the library itself rather than its import library, as it's loaded late: see tests/imports.rs in rsui-tsx.
        #[link(name = "dbghelp.dll", kind = "raw-dylib", modifiers = "+verbatim")]
        unsafe extern "system" {
            fn SymInitializeW(process: *mut c_void, path: *const u16, invade: i32) -> i32;
            fn SymSetOptions(options: u32) -> u32;
            fn SymLoadModuleExW(process: *mut c_void, file: *mut c_void, image: *const u16, module: *const u16, base: u64, size: u32, data: *mut c_void, flags: u32) -> u64;
            fn SymFromAddrW(process: *mut c_void, address: u64, displacement: *mut u64, symbol: *mut SymbolInfo) -> i32;
        }

        #[repr(C)]
        struct SymbolInfo {
            size_of_struct: u32,
            type_index: u32,
            reserved: [u64; 2],
            index: u32,
            size: u32,
            mod_base: u64,
            flags: u32,
            value: u64,
            address: u64,
            register: u32,
            scope: u32,
            tag: u32,
            name_len: u32,
            max_name_len: u32,
            name: [u16; 512],
        }

        pub(crate) fn start() {
            ON.store(true, Ordering::Relaxed);
        }

        fn inside<R>(run: impl FnOnce() -> R) -> Option<R> {
            let entered = INSIDE.try_with(|inside| !inside.replace(true)).unwrap_or(false);
            if !entered {
                return None;
            }
            let out = run();
            let _ = INSIDE.try_with(|inside| inside.set(false));
            Some(out)
        }

        pub(crate) fn maybe_sample(block: *mut u8, owner: u8, size: usize) -> bool {
            if !ON.load(Ordering::Relaxed) {
                return false;
            }
            let big = size >= BIG;
            if !big && COUNT.fetch_add(1, Ordering::Relaxed) % RATE != 0 {
                return false;
            }
            // What it stands for: a small one for RATE of them.
            let (weight, size) = if big { (1, size) } else { (RATE, size * RATE) };
            inside(|| {
                let mut frames = [std::ptr::null_mut::<c_void>(); DEPTH];
                // SAFETY: there's room for DEPTH frames.
                let got = unsafe { RtlCaptureStackBackTrace(2, DEPTH as u32, frames.as_mut_ptr(), std::ptr::null_mut()) } as usize;
                let mut stack = [0usize; DEPTH];
                for at in 0..got.min(DEPTH) {
                    stack[at] = frames[at] as usize;
                }
                let Ok(mut kept) = KEPT.lock() else {
                    return false;
                };
                let kept = kept.get_or_insert_with(Kept::default);
                let next = kept.each.len() as u32;
                let id = *kept.stacks.entry((stack, owner)).or_insert(next);
                if id == next {
                    kept.each.push((stack, owner, 0, 0));
                }
                kept.each[id as usize].2 += size as isize;
                kept.each[id as usize].3 += weight as isize;
                kept.live.insert(block as usize, (id, size, weight));
                true
            })
            .unwrap_or(false)
        }

        pub(crate) fn forget(block: *mut u8) {
            // A block that was sampled is always forgotten, even from inside, so it's done with the lock alone.
            let entered = INSIDE.try_with(|inside| inside.replace(true)).unwrap_or(true);
            if let Ok(mut kept) = KEPT.lock()
                && let Some(kept) = kept.as_mut()
                && let Some((id, size, weight)) = kept.live.remove(&(block as usize))
            {
                kept.each[id as usize].2 -= size as isize;
                kept.each[id as usize].3 -= weight as isize;
            }
            if !entered {
                let _ = INSIDE.try_with(|inside| inside.set(false));
            }
        }

        fn name_of(process: *mut c_void, address: usize, names: &mut HashMap<usize, String>) -> String {
            names
                .entry(address)
                .or_insert_with(|| {
                    // SAFETY: the struct is as dbghelp has it, with room for 512 characters of name.
                    unsafe {
                        let mut info: Box<SymbolInfo> = Box::new(std::mem::zeroed());
                        info.size_of_struct = 88;
                        info.max_name_len = 512;
                        let mut displacement = 0u64;
                        if SymFromAddrW(process, address as u64, &mut displacement, &mut *info) != 0 {
                            let length = (info.name_len as usize).min(512);
                            String::from_utf16_lossy(&info.name[..length])
                        } else {
                            // Where it is in the program, for a map of the program to say.
                            let base = GetModuleHandleW(std::ptr::null()) as usize;
                            if address > base { format!("rva:{:#x}", address - base) } else { format!("{address:#x}") }
                        }
                    }
                })
                .clone()
        }

        /// What's live of what was sampled, by owner and the first calls that made it that aren't the allocator's own or the
        /// standard library's, scaled up by [`RATE`], the most bytes first.
        pub(crate) fn report(top: usize, frames: usize) -> Vec<String> {
            // Off before the lock's taken: copying what's kept makes a block that would be sampled, which takes the lock.
            ON.store(false, Ordering::Relaxed);
            let each = {
                let Ok(kept) = KEPT.lock() else {
                    return Vec::new();
                };
                let Some(kept) = kept.as_ref() else {
                    return Vec::new();
                };
                kept.each.clone()
            };
            // SAFETY: dbghelp is asked about this process, once.
            let process = unsafe { GetCurrentProcess() };
            unsafe {
                SymSetOptions(0x2 | 0x10 | 0x4);
                // Where the program's symbols are, as `RSUI_ALLOC_SYMBOLS` says, as dbghelp doesn't always find them by itself.
                let path: Option<Vec<u16>> = std::env::var("RSUI_ALLOC_SYMBOLS").ok().map(|path| path.encode_utf16().chain([0]).collect());
                if SymInitializeW(process, path.as_ref().map_or(std::ptr::null(), |path| path.as_ptr()), 0) == 0 {
                    eprintln!("dbghelp didn't start: error {}", GetLastError());
                }
                // The program itself, where it's loaded, which is where nearly everything that allocates is.
                let module = GetModuleHandleW(std::ptr::null());
                let mut path = [0u16; 1024];
                let length = GetModuleFileNameW(module, path.as_mut_ptr(), 1023) as usize;
                path[length.min(1023)] = 0;
                if SymLoadModuleExW(process, std::ptr::null_mut(), path.as_ptr(), std::ptr::null(), module as u64, 0, std::ptr::null_mut(), 0) == 0 {
                    eprintln!("dbghelp didn't load the program: error {}", GetLastError());
                }
            }
            let mut names = HashMap::new();
            let mut grouped: HashMap<String, (isize, isize)> = HashMap::new();
            for (stack, owner, bytes, count) in each {
                if count <= 0 {
                    continue;
                }
                let mut said = Vec::new();
                for &address in stack.iter().take_while(|address| **address != 0) {
                    let name = name_of(process, address, &mut names);
                    let plain = ["alloc::", "core::", "std::", "hashbrown::", "rsui_alloc::", "thin_vec::", "triomphe::", "<alloc", "<core", "<std", "<hashbrown", "<thin_vec", "__rust", "rust_alloc", "rust_realloc"]
                        .iter()
                        .any(|start| name.starts_with(start));
                    if !plain {
                        said.push(name);
                    }
                    if said.len() >= frames {
                        break;
                    }
                }
                let key = format!("{} | {}", super::super::NAMES.get(owner as usize).copied().unwrap_or("?"), said.join(" < "));
                let entry = grouped.entry(key).or_default();
                entry.0 += bytes;
                entry.1 += count;
            }
            let mut out: Vec<(String, (isize, isize))> = grouped.into_iter().collect();
            out.sort_by(|a, b| b.1.0.cmp(&a.1.0));
            out.into_iter()
                .take(top)
                .map(|(key, (bytes, count))| format!("{:>12} B {:>9} blocks  {key}", bytes, count))
                .collect()
        }
    }
}

#[cfg(feature = "owners")]
pub use counting::Owned;

/// While it's there, what's made is counted as `owner`'s. Scopes inside it take over until they go.
pub struct Scope {
    #[cfg(feature = "owners")]
    before: u8,
}

/// Counts what's made from now until the scope goes as `owner`'s.
#[inline(always)]
#[must_use]
pub fn scope(owner: u8) -> Scope {
    #[cfg(feature = "owners")]
    {
        Scope { before: counting::replace(owner) }
    }
    #[cfg(not(feature = "owners"))]
    {
        let _ = owner;
        Scope {}
    }
}

/// The same, unless what's running now is the geometry's layout, whose own text is counted apart.
#[inline(always)]
#[must_use]
pub fn text_scope() -> Scope {
    #[cfg(feature = "owners")]
    {
        let owner = if matches!(counting::now(), GEOMETRY | GEOMETRY_TEXT) { GEOMETRY_TEXT } else { TEXT };
        scope(owner)
    }
    #[cfg(not(feature = "owners"))]
    {
        Scope {}
    }
}

#[cfg(feature = "owners")]
impl Drop for Scope {
    #[inline(always)]
    fn drop(&mut self) {
        counting::replace(self.before);
    }
}

/// Whether blocks are being counted, which is the `owners` feature.
pub const COUNTING: bool = cfg!(feature = "owners");

/// Starts keeping where a sample of blocks are made, for [`sites`]. Nothing without the `owners` feature.
pub fn start_sites() {
    #[cfg(feature = "owners")]
    counting::sites::start();
}

/// What's live of the blocks sampled since [`start_sites`], by owner and the `frames` calls that made them, scaled up to stand for
/// every block, the most bytes first. It stops sampling. Nothing without the `owners` feature.
pub fn sites(top: usize, frames: usize) -> Vec<String> {
    #[cfg(feature = "owners")]
    {
        counting::sites::report(top, frames)
    }
    #[cfg(not(feature = "owners"))]
    {
        let _ = (top, frames);
        Vec::new()
    }
}

/// The bytes asked for that each stage is holding now, by name. Nothing without the `owners` feature.
pub fn live() -> Vec<(&'static str, isize)> {
    #[cfg(feature = "owners")]
    {
        let live = counting::live();
        NAMES.iter().enumerate().map(|(at, name)| (*name, live[at])).collect()
    }
    #[cfg(not(feature = "owners"))]
    {
        Vec::new()
    }
}
