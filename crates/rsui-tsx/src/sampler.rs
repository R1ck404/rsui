//! A small native sampling profiler for one thread, for Windows x64, with nothing to install and no admin rights: another
//! thread suspends the one being measured about every millisecond, reads where it is, walks its stack with the unwind
//! tables, and lets it go. At the end, addresses are turned into function names with dbghelp, which needs a build with
//! debug info (`CARGO_PROFILE_RELEASE_DEBUG=line-tables-only CARGO_PROFILE_RELEASE_STRIP=none`; the code is the same).
//!
//! Used by `rsui-tsx native-prof`. It's for measuring only.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

const MOST_FRAMES: usize = 48;

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
}

#[link(name = "dbghelp")]
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

fn read(context: &Context, at: usize) -> u64 {
    u64::from_le_bytes(context.0[at..at + 8].try_into().unwrap())
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
            let begins = std::time::Instant::now() + delay;
            while !stopping.load(Ordering::Relaxed) {
                let Some(left) = begins.checked_duration_since(std::time::Instant::now()) else { break };
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
                // SAFETY: the target is suspended while its context is read and its stack walked; that's memory of this
                // same process, and the unwinder only reads it.
                unsafe {
                    if SuspendThread(target) == u32::MAX {
                        break;
                    }
                    let mut context = Context([0; 1232]);
                    context.0[0x30..0x34].copy_from_slice(&CONTEXT_FULL.to_le_bytes());
                    if GetThreadContext(target, &mut context) != 0 {
                        for _ in 0..MOST_FRAMES {
                            let pc = read(&context, RIP);
                            if pc == 0 {
                                break;
                            }
                            stack.push(pc);
                            let mut base = 0;
                            let entry = RtlLookupFunctionEntry(pc, &mut base, std::ptr::null_mut());
                            if entry.is_null() {
                                // A leaf function: its return address is on top of the stack.
                                let rsp = read(&context, RSP);
                                if rsp == 0 {
                                    break;
                                }
                                let ret = *(rsp as *const u64);
                                context.0[RIP..RIP + 8].copy_from_slice(&ret.to_le_bytes());
                                context.0[RSP..RSP + 8].copy_from_slice(&(rsp + 8).to_le_bytes());
                            } else {
                                let mut data = std::ptr::null_mut();
                                let mut frame = 0;
                                RtlVirtualUnwind(
                                    0,
                                    base,
                                    pc,
                                    entry,
                                    &mut context,
                                    &mut data,
                                    &mut frame,
                                    std::ptr::null_mut(),
                                );
                            }
                        }
                    }
                    ResumeThread(target);
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
            if SymGetLineFromAddr64(GetCurrentProcess(), address, &mut displacement, &mut line) == 0 {
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
