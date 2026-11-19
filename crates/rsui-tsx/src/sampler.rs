//! A small native sampling profiler for one thread, for Windows x64, with nothing to install and no admin rights: another
//! thread suspends the one being measured about every millisecond, reads where it is, walks its stack with the unwind
//! tables (and past code that has none, as the JIT's hasn't: see `walk`), and lets it go. At the end, addresses are turned into function names with dbghelp, which needs a build with
//! debug info (`CARGO_PROFILE_RELEASE_DEBUG=line-tables-only CARGO_PROFILE_RELEASE_STRIP=none`; the code is the same).
//!
//! Used by `rsui-tsx native-prof`. It's for measuring only.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

/// The most frames kept of a sample, the innermost: React's renders, with compiled functions walked too, go past 48.
const MOST_FRAMES: usize = 128;

#[repr(C, align(16))]
struct Context([u8; 1232]);

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
    name: [u8; 512],
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcess() -> isize;
    fn OpenThread(access: u32, inherit: i32, id: u32) -> isize;
    fn CloseHandle(handle: isize) -> i32;
    fn SuspendThread(thread: isize) -> u32;
    fn ResumeThread(thread: isize) -> u32;
    fn GetThreadContext(thread: isize, context: *mut Context) -> i32;
    fn RtlLookupFunctionEntry(pc: u64, image_base: *mut u64, history: *mut u8) -> *const u8;
    fn RtlVirtualUnwind(
        kind: u32,
        image_base: u64,
        pc: u64,
        entry: *const u8,
        context: *mut Context,
        handler_data: *mut *mut u8,
        establisher: *mut u64,
        pointers: *mut u8,
    ) -> *const u8;
    fn GetModuleHandleExW(flags: u32, address: *const u8, module: *mut isize) -> i32;
    fn GetModuleFileNameW(module: isize, name: *mut u16, size: u32) -> u32;
    fn VirtualQuery(address: *const u8, info: *mut MemoryInfo, size: usize) -> usize;
    fn ReadProcessMemory(process: isize, address: *const u8, buffer: *mut u8, size: usize, read: *mut usize) -> i32;
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationThread(
        thread: isize,
        class: u32,
        info: *mut u8,
        size: u32,
        returned: *mut u32,
    ) -> i32;
}

/// `MEMORY_BASIC_INFORMATION` on x64.
#[repr(C)]
#[derive(Default)]
struct MemoryInfo {
    base: usize,
    allocation_base: usize,
    allocation_protect: u32,
    partition: u16,
    region_size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}

// From the library itself rather than its import library, as it's loaded late (`/DELAYLOAD` in .cargo/config.toml): the
// import library would give it a second table of what's called in it, besides the one rsui-alloc's declarations give.
// See tests/imports.rs.
#[link(name = "dbghelp.dll", kind = "raw-dylib", modifiers = "+verbatim")]
unsafe extern "system" {
    fn SymInitializeW(process: isize, search: *const u16, invade: i32) -> i32;
    fn SymSetOptions(options: u32) -> u32;
    fn SymFromAddr(
        process: isize,
        address: u64,
        displacement: *mut u64,
        symbol: *mut SymbolInfo,
    ) -> i32;
    fn SymGetLineFromAddr64(
        process: isize,
        address: u64,
        displacement: *mut u32,
        line: *mut LineInfo,
    ) -> i32;
}

/// `IMAGEHLP_LINE64` on x64.
#[repr(C)]
struct LineInfo {
    size_of_struct: u32,
    key: usize,
    line_number: u32,
    file_name: *const u8,
    address: u64,
}

const CONTEXT_FULL: u32 = 0x10000B;
const RIP: usize = 0xF8;
const RSP: usize = 0x98;

/// Where the general registers start in a `CONTEXT`, `rax` first, in the order x64 numbers them.
const REGISTERS: usize = 0x78;
/// How far up the stack a return address is looked for, above a frame with no unwind information.
const MOST_WORDS: u64 = 1024;
/// The most of the stack copied for a sample, from where it's at up: what a deep render uses fits.
const MOST_COPIED: usize = 1 << 20;

fn read(context: &Context, at: usize) -> u64 {
    u64::from_le_bytes(context.0[at..at + 8].try_into().unwrap())
}

fn write(context: &mut Context, at: usize, value: u64) {
    context.0[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

/// A copy of the part of the stack that's in use, from the stack pointer up, which is walked once the thread's going
/// again: `low..high` is the copy, and `real` is where it was copied from. Where the stack is comes from the thread's
/// TEB, which `SwitchToFiber` keeps right for the stack it's on, so it's read again for each sample (layout and drawing go
/// deep on a fiber of their own: see `rsui_core::stack::deep`).
struct Bounds {
    low: u64,
    high: u64,
    real: u64,
}

impl Bounds {
    /// Whether 8 bytes at `at` are in the copy.
    fn holds(&self, at: u64) -> bool {
        at % 8 == 0 && at >= self.low && at + 8 <= self.high
    }

    /// Where an address on the thread's stack is in the copy; anything else is as it was.
    fn copied(&self, at: u64) -> u64 {
        if at >= self.real && at - self.real < self.high - self.low {
            at - self.real + self.low
        } else {
            at
        }
    }
}

/// Readable code: the memory's committed and can be run. Each address costs a call into the system, so the last region
/// found is kept.
struct Code {
    /// The region, whether it's code, and whether it's mapped rather than an image, as the JIT's is.
    last: (u64, u64, bool, bool),
}

impl Code {
    fn runs(&mut self, at: u64) -> bool {
        if !(0x10000..0x7FFF_FFFF_0000).contains(&at) {
            return false;
        }
        self.find(at).0
    }

    /// Whether `at` is in what the JIT compiled: code that's mapped, as the JIT's is (crates/rsui-js/src/jit/memory.rs),
    /// rather than in an image.
    fn compiled(&mut self, at: u64) -> bool {
        (0x10000..0x7FFF_FFFF_0000).contains(&at) && self.find(at) == (true, true)
    }

    fn find(&mut self, at: u64) -> (bool, bool) {
        let (start, end, runs, mapped) = self.last;
        if (start..end).contains(&at) {
            return (runs, mapped);
        }
        let mut info = MemoryInfo::default();
        // SAFETY: a question about this process's own memory, answered into memory of this thread's own; it takes no lock
        // the stopped thread could hold.
        if unsafe { VirtualQuery(at as *const u8, &mut info, size_of::<MemoryInfo>()) } == 0 {
            return (false, false);
        }
        const MEM_COMMIT: u32 = 0x1000;
        // PAGE_EXECUTE, _READ, _READWRITE, _WRITECOPY, with no PAGE_GUARD.
        let runs =
            info.state == MEM_COMMIT && info.protect & 0xF0 != 0 && info.protect & 0x100 == 0;
        const MEM_MAPPED: u32 = 0x40000;
        let mapped = info.kind == MEM_MAPPED;
        self.last = (
            info.base as u64,
            (info.base + info.region_size) as u64,
            runs,
            mapped,
        );
        (runs, mapped)
    }

    /// Whether `at` is just after a call, as a return address is: `call rel32`, or `call` through a register or memory
    /// (`FF /2`), with or without a prefix in front, which can't be told apart and doesn't need to be.
    fn after_call(&mut self, at: u64) -> bool {
        if !self.runs(at) || !self.runs(at - 7) {
            return false;
        }
        // SAFETY: the 7 bytes before are in code that's committed, so they can be read.
        let before: [u8; 7] = unsafe { std::ptr::read_unaligned((at - 7) as *const [u8; 7]) };
        if before[2] == 0xE8 {
            return true;
        }
        (2..=7).any(|back: usize| {
            let (op, modrm) = (before[7 - back], before[8 - back]);
            if op != 0xFF || (modrm >> 3) & 7 != 2 {
                return false;
            }
            let (mode, rm) = (modrm >> 6, modrm & 7);
            let sib_base_is_rbp = rm == 4 && back >= 3 && before[9 - back] & 7 == 5;
            let length = match mode {
                3 => 2,
                0 if rm == 5 => 6,
                0 if rm == 4 => {
                    if sib_base_is_rbp {
                        7
                    } else {
                        3
                    }
                }
                0 => 2,
                1 => {
                    if rm == 4 {
                        4
                    } else {
                        3
                    }
                }
                _ => {
                    if rm == 4 {
                        7
                    } else {
                        6
                    }
                }
            };
            length == back
        })
    }
}

/// Which register a function whose unwind information says it sets up a frame pointer keeps it in (its place in a
/// `CONTEXT`), and how far above the stack pointer it was set, if it does, following chained information: the unwinder
/// takes the stack pointer from it, and if that's wrong it reads wherever it points.
///
/// # Safety
/// `entry` is a function entry `RtlLookupFunctionEntry` gave for an image at `base`.
unsafe fn frame_register(base: u64, entry: *const u8) -> Option<(usize, u64)> {
    let mut entry = entry;
    for _ in 0..8 {
        // SAFETY: a RUNTIME_FUNCTION is three RVAs, and what the third names is the image's own unwind information.
        unsafe {
            let rva = std::ptr::read_unaligned(entry.add(8).cast::<u32>());
            // With the low bit set it names another RUNTIME_FUNCTION, whose information this one shares.
            if rva & 1 != 0 {
                entry = (base + u64::from(rva & !1)) as *const u8;
                continue;
            }
            let info = (base + u64::from(rva)) as *const u8;
            let (flags, codes, frame) = (*info >> 3, *info.add(2), *info.add(3));
            if frame & 0xF != 0 {
                return Some((
                    REGISTERS + usize::from(frame & 0xF) * 8,
                    u64::from(frame >> 4) * 16,
                ));
            }
            // UNW_FLAG_CHAININFO: another RUNTIME_FUNCTION follows the codes, which are an even number of 2-byte slots.
            if flags & 0x4 == 0 {
                return None;
            }
            entry = info.add(4 + usize::from(codes.next_multiple_of(2)) * 2);
        }
    }
    None
}

/// Walks a copy of a thread's stack from `context` (with its stack pointer in the copy) into `stack`, innermost first.
/// Code with no unwind information (what the JIT compiles, and its way in from Rust) has its return address looked for
/// on the stack above it: the first word that's just after a call in code that's committed. Every address read is checked
/// to be in the copy first, and what the unwinder would read is checked as far as it can be, as the walk can go wrong,
/// and reading where it then pointed stopped the program (an access violation in ntdll, in a JIT build).
///
/// It's a copy, walked with the thread going again, because finding a function's unwind information can wait for a lock
/// (the table of what's loaded, or of code made at run time, as WARP's shaders are) that the stopped thread can be
/// holding, as it is while it loads a library: walking it in place sometimes hung a program as it started.
///
/// # Safety
/// `bounds` is the copy.
unsafe fn walk(context: &mut Context, bounds: &Bounds, code: &mut Code, stack: &mut Vec<u64>) {
    // Whether a frame's been gone past by looking for its return address, which leaves the registers it saved as they
    // were in it: the JIT keeps its list of cells in `rbp` (crates/rsui-js/JIT.md), which the interpreter's loop under it
    // keeps its frame in.
    let mut guessed = false;
    for _ in 0..MOST_FRAMES {
        let pc = read(context, RIP);
        let rsp = read(context, RSP);
        if pc == 0 || !bounds.holds(rsp) {
            return;
        }
        stack.push(pc);
        let mut base = 0;
        // SAFETY: a lookup in the tables of what's loaded.
        let entry = unsafe { RtlLookupFunctionEntry(pc, &mut base, std::ptr::null_mut()) };
        if entry.is_null() {
            // A leaf function, whose return address is on top of the stack, or code with no unwind information. A compiled
            // function's body keeps its return address 40 bytes up (crates/rsui-js/JIT.md), which is looked at first, as
            // below it are the 32 bytes a callee can keep what it likes in, where a stale return address can be.
            let body = rsp + 40;
            // SAFETY: on the stack, which is committed.
            let known = (code.compiled(pc)
                && bounds.holds(body)
                && code.after_call(unsafe { *(body as *const u64) }))
            .then_some(body);
            let Some(at) = known.or_else(|| {
                (0..MOST_WORDS)
                    .map(|word| rsp + word * 8)
                    .take_while(|at| bounds.holds(*at))
                    // SAFETY: on the stack, which is committed.
                    .find(|at| code.after_call(unsafe { *(*at as *const u64) }))
            }) else {
                return;
            };
            // SAFETY: as above.
            write(context, RIP, unsafe { *(at as *const u64) });
            write(context, RSP, at + 8);
            guessed = true;
            continue;
        }
        // SAFETY: the entry is for an image at `base`.
        let frame = unsafe { frame_register(base, entry) };
        // A frame pointer is an address on the thread's stack, which the unwinder is to read from the copy.
        if let Some((register, _)) = frame {
            write(context, register, bounds.copied(read(context, register)));
        }
        if let Some((register, offset)) =
            frame.filter(|&(register, _)| !bounds.holds(read(context, register) & !7))
        {
            // Under a frame gone past, the frame pointer is where it was set from the stack pointer, a little above
            // where that is now if the function lined its stack up after (as the interpreter's loop does), or allocates
            // on it later. The first place from which unwinding comes to just after a call is taken.
            if !guessed {
                return;
            }
            let tried = context.0;
            let found = (0..GUESSES).any(|word| {
                context.0 = tried;
                let at = rsp + offset + word * 8;
                // SAFETY: as below, with a frame pointer on the stack.
                bounds.holds(at)
                    && unsafe {
                        write(context, register, at);
                        unwind(base, pc, entry, context, rsp, bounds, code)
                            && code.after_call(read(context, RIP))
                    }
            });
            if !found {
                return;
            }
            continue;
        }
        // SAFETY: the stack pointer, and the frame pointer if there is one, are on the stack.
        if !unsafe { unwind(base, pc, entry, context, rsp, bounds, code) } {
            return;
        }
    }
}

/// How many places a frame pointer is looked for at, 8 bytes apart: see `walk`.
const GUESSES: u64 = 16;

/// Unwinds one frame of a function with unwind information, and says whether where it comes to looks right: further up
/// the stack, at code (or at nothing, at the end).
///
/// # Safety
/// The context's stack pointer, and its frame pointer if the function has one, are on the stack.
unsafe fn unwind(
    base: u64,
    pc: u64,
    entry: *const u8,
    context: &mut Context,
    rsp: u64,
    bounds: &Bounds,
    code: &mut Code,
) -> bool {
    let mut data = std::ptr::null_mut();
    let mut frame = 0;
    // SAFETY: the caller's.
    unsafe {
        RtlVirtualUnwind(
            0,
            base,
            pc,
            entry,
            context,
            &mut data,
            &mut frame,
            std::ptr::null_mut(),
        );
    }
    let (next, next_rsp) = (read(context, RIP), read(context, RSP));
    next_rsp > rsp && bounds.holds(next_rsp - 8) && (next == 0 || code.runs(next))
}

pub struct Sampler {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Vec<Vec<u64>>>>,
}

impl Sampler {
    /// Starts sampling the thread that calls this.
    pub fn start(every: Duration) -> Self {
        Self::start_after(every, Duration::ZERO)
    }

    /// Samples this caller after a warmup delay, without delaying the caller.
    /// Stopping during warmup joins promptly and produces no samples.
    pub fn start_after(every: Duration, delay: Duration) -> Self {
        // SAFETY: plain Win32 calls on this thread's own id.
        let target = unsafe { OpenThread(0x0002 | 0x0008 | 0x0040, 0, GetCurrentThreadId()) };
        assert!(target != 0, "can't open this thread to sample it");
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut samples = Vec::new();
            // Where the thread's TEB is, which says where its stack is.
            let mut basic = [0u8; 48];
            // SAFETY: ThreadBasicInformation, into room of its size, for a handle opened to query.
            let teb = unsafe {
                if NtQueryInformationThread(target, 0, basic.as_mut_ptr(), 48, std::ptr::null_mut())
                    < 0
                {
                    0
                } else {
                    u64::from_le_bytes(basic[8..16].try_into().unwrap())
                }
            };
            let mut code = Code {
                last: (0, 0, false, false),
            };
            let mut copy = vec![0u64; MOST_COPIED / 8];
            // SAFETY: the pseudo handle for this process needs nothing closed.
            let process = unsafe { GetCurrentProcess() };
            let begins = std::time::Instant::now() + delay;
            while !stopping.load(Ordering::Relaxed) {
                let Some(left) = begins.checked_duration_since(std::time::Instant::now()) else {
                    break;
                };
                std::thread::sleep(left.min(Duration::from_millis(10)));
            }
            while !stopping.load(Ordering::Relaxed) {
                // Windows sleeps a millisecond at the least, so a shorter time between samples is waited out on this
                // thread's own core.
                if every < Duration::from_micros(500) {
                    let until = std::time::Instant::now() + every;
                    while std::time::Instant::now() < until {
                        std::hint::spin_loop();
                    }
                } else {
                    std::thread::sleep(every);
                }
                // Once it's been asked to stop, the thread is only waiting for this one, which isn't worth a sample.
                if stopping.load(Ordering::Relaxed) {
                    break;
                }
                let mut stack = Vec::with_capacity(MOST_FRAMES);
                let mut context = Context([0; 1232]);
                context.0[0x30..0x34].copy_from_slice(&CONTEXT_FULL.to_le_bytes());
                let mut copied = 0;
                let mut real = 0;
                // SAFETY: while the target is stopped, only its registers are read, and the part of its stack in use
                // copied, which `ReadProcessMemory` does only if it can; no lock is taken nor anything allocated (it
                // could be holding the allocator's lock).
                unsafe {
                    if SuspendThread(target) == u32::MAX {
                        break;
                    }
                    if teb != 0 && GetThreadContext(target, &mut context) != 0 {
                        let (high, low) = (*((teb + 8) as *const u64), *((teb + 16) as *const u64));
                        real = read(&context, RSP);
                        if (low..high).contains(&real) {
                            let size = ((high - real) as usize).min(MOST_COPIED);
                            let into = copy.as_mut_ptr().cast();
                            if ReadProcessMemory(process, real as *const u8, into, size, &mut copied) == 0 {
                                copied = 0;
                            }
                        }
                    }
                    ResumeThread(target);
                }
                if copied > 0 {
                    let low = copy.as_ptr() as u64;
                    let bounds = Bounds {
                        low,
                        high: low + (copied as u64 & !7),
                        real,
                    };
                    write(&mut context, RSP, low);
                    // SAFETY: the bounds are the copy's.
                    unsafe { walk(&mut context, &bounds, &mut code, &mut stack) };
                }
                samples.push(stack);
            }
            // SAFETY: it was opened above.
            unsafe { CloseHandle(target) };
            samples
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }

    /// Stops, and gives every sample's stack, innermost first.
    pub fn finish(mut self) -> Vec<Vec<u64>> {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap_or_default()
    }
}

/// Names for addresses, as `module!function`.
pub struct Names {
    known: HashMap<u64, String>,
}

impl Names {
    pub fn new() -> Self {
        // SAFETY: dbghelp, set up once for this process.
        unsafe {
            SymSetOptions(0x2 | 0x4); // undecorated names, modules loaded as they're wanted
            // The program's own folder, where its .pdb is, then wherever the system would look.
            let folder = std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.to_string_lossy().into_owned()))
                .unwrap_or_default();
            let search: Vec<u16> = format!(
                "{folder};{}",
                std::env::var("_NT_SYMBOL_PATH").unwrap_or_default()
            )
            .encode_utf16()
            .chain(Some(0))
            .collect();
            SymInitializeW(GetCurrentProcess(), search.as_ptr(), 1);
        }
        Self {
            known: HashMap::new(),
        }
    }

    pub fn name(&mut self, address: u64) -> String {
        if let Some(found) = self.known.get(&address) {
            return found.clone();
        }
        // SAFETY: the struct is the size dbghelp is told, with room for the name.
        let name = unsafe {
            let mut module = 0;
            let mut module_name = [0u16; 260];
            let module_text =
                if GetModuleHandleExW(0x4 | 0x2, address as *const u8, &mut module) != 0 {
                    let length = GetModuleFileNameW(module, module_name.as_mut_ptr(), 260) as usize;
                    let full = String::from_utf16_lossy(&module_name[..length]);
                    full.rsplit(['\\', '/'])
                        .next()
                        .unwrap_or("?")
                        .trim_end_matches(".exe")
                        .trim_end_matches(".dll")
                        .to_owned()
                } else {
                    "?".to_owned()
                };
            let mut symbol: SymbolInfo = std::mem::zeroed();
            // What `sizeof(SYMBOL_INFO)` is in C on x64, with its one character of name.
            symbol.size_of_struct = 88;
            symbol.max_name_len = 511;
            let mut displacement = 0;
            if SymFromAddr(GetCurrentProcess(), address, &mut displacement, &mut symbol) != 0 {
                let length = (symbol.name_len as usize).min(511);
                format!(
                    "{module_text}!{}",
                    simplify(&String::from_utf8_lossy(&symbol.name[..length]))
                )
            } else {
                format!("{module_text}!?")
            }
        };
        self.known.insert(address, name.clone());
        name
    }
}

impl Names {
    /// The source file and line an address is from, as `file:line`, or nothing if it isn't known.
    pub fn line(&mut self, address: u64) -> Option<String> {
        // SAFETY: the struct is the size dbghelp is told, and the name it points at is a C string dbghelp keeps.
        unsafe {
            let mut line: LineInfo = std::mem::zeroed();
            line.size_of_struct = size_of::<LineInfo>() as u32;
            let mut displacement = 0;
            if SymGetLineFromAddr64(GetCurrentProcess(), address, &mut displacement, &mut line) == 0
            {
                return None;
            }
            let name = std::ffi::CStr::from_ptr(line.file_name.cast()).to_string_lossy();
            let short = name.rsplit(['\\', '/']).next().unwrap_or("?").to_owned();
            Some(format!("{short}:{}", line.line_number))
        }
    }
}

/// Drops generic arguments and hashes, which only make names long.
fn simplify(name: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in name.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// The functions samples were in, by themselves (self) and anywhere on the stack (total), most first.
pub fn report(samples: &[Vec<u64>], names: &mut Names, top: usize) -> String {
    let total = samples.len().max(1) as f64;
    let mut own: HashMap<String, u32> = HashMap::new();
    let mut inclusive: HashMap<String, u32> = HashMap::new();
    let mut modules: HashMap<String, u32> = HashMap::new();
    for stack in samples {
        let Some(&leaf) = stack.first() else { continue };
        let leaf_name = names.name(leaf);
        *modules
            .entry(leaf_name.split('!').next().unwrap_or("?").to_owned())
            .or_default() += 1;
        *own.entry(leaf_name).or_default() += 1;
        let mut seen = std::collections::HashSet::new();
        for &pc in stack {
            let name = names.name(pc);
            if seen.insert(name.clone()) {
                *inclusive.entry(name).or_default() += 1;
            }
        }
    }
    let sorted = |map: HashMap<String, u32>| {
        let mut rows: Vec<(String, u32)> = map.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        rows
    };
    let mut out = format!("{} samples\n\nby module (self)\n", samples.len());
    for (name, count) in sorted(modules) {
        out.push_str(&format!(
            "{:6.1}%  {name}\n",
            f64::from(count) * 100.0 / total
        ));
    }
    out.push_str("\nself\n");
    for (name, count) in sorted(own).into_iter().take(top) {
        out.push_str(&format!(
            "{:6.1}%  {name}\n",
            f64::from(count) * 100.0 / total
        ));
    }
    out.push_str("\ntotal (on the stack)\n");
    for (name, count) in sorted(inclusive).into_iter().take(top) {
        out.push_str(&format!(
            "{:6.1}%  {name}\n",
            f64::from(count) * 100.0 / total
        ));
    }
    // Who calls the functions with the most self time, as small helpers (memcmp, alloc) say little on their own.
    let mut callers: HashMap<String, HashMap<String, u32>> = HashMap::new();
    for stack in samples {
        if let [leaf, parent, ..] = stack.as_slice() {
            let (leaf, parent) = (names.name(*leaf), names.name(*parent));
            *callers.entry(leaf).or_default().entry(parent).or_default() += 1;
        }
    }
    // With `RSUI_PROF_LINES` set, the source lines the samples were on, which says where in a big function such as the
    // interpreter's loop the time went. With inlining, a line can be in a function inlined into the one sampled.
    if std::env::var_os("RSUI_PROF_LINES").is_some() {
        let mut lines: HashMap<String, u32> = HashMap::new();
        for stack in samples {
            let Some(&leaf) = stack.first() else { continue };
            let line = names.line(leaf).unwrap_or_else(|| "?".to_owned());
            *lines.entry(line).or_default() += 1;
        }
        out.push_str("\nself, by source line\n");
        for (line, count) in sorted(lines).into_iter().take(60) {
            out.push_str(&format!(
                "{:6.1}%  {line}\n",
                f64::from(count) * 100.0 / total
            ));
        }
    }
    // With `RSUI_PROF_FOLDED=<file>`, each stack as one line of what called what, outermost first, with how many samples were in
    // it, as flame graph tools read them: which of the callers of something its time came from.
    if let Some(file) = std::env::var_os("RSUI_PROF_FOLDED").filter(|file| !file.is_empty()) {
        let mut folded: HashMap<String, u32> = HashMap::new();
        for stack in samples {
            let line: Vec<String> = stack
                .iter()
                .rev()
                .map(|&pc| names.name(pc).replace(';', ":"))
                .collect();
            *folded.entry(line.join(";")).or_default() += 1;
        }
        let text: String = sorted(folded)
            .into_iter()
            .map(|(line, count)| format!("{line} {count}\n"))
            .collect();
        if let Err(error) = std::fs::write(&file, text) {
            eprintln!(
                "folded stacks {}: {error}",
                std::path::Path::new(&file).display()
            );
        }
    }
    out.push_str("\ncallers of the top self functions\n");
    let mut own_sorted: Vec<(String, u32)> = callers
        .iter()
        .map(|(k, v)| (k.clone(), v.values().sum()))
        .collect();
    own_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (leaf, _) in own_sorted.into_iter().take(12) {
        out.push_str(&format!("  {leaf}\n"));
        for (parent, count) in sorted(callers.remove(&leaf).unwrap_or_default())
            .into_iter()
            .take(4)
        {
            out.push_str(&format!(
                "    {:6.1}%  <- {parent}\n",
                f64::from(count) * 100.0 / total
            ));
        }
    }
    out
}
