//! A baseline JIT: a function that's run often enough has its instructions turned into x86-64, one template per
//! instruction, which runs on the interpreter's own frames and registers, with its values as they are. So compiled code
//! can be entered and left between any two instructions: anything it doesn't do itself it calls the interpreter's own
//! functions for, and anything it can't do at all it leaves for the interpreter to carry on with, from that
//! instruction. See `JIT.md`.
//!
//! It's built with the `jit` feature (which rsui-tsx turns on by default), on 64-bit Windows, and `RSUI_JIT` says
//! what it does: `0` (or `off`) nothing,
//! `1` (the default) compiles a function once it's been called or gone round a loop [`THRESHOLD`] times, and `eager`
//! compiles every function it can the first time it's called, which is for testing. `RSUI_JIT_LOG=1` writes a line
//! about each function compiled, and what all of it came to when the engine goes.

#[cfg(all(windows, target_arch = "x86_64"))]
mod asm;
#[cfg(all(windows, target_arch = "x86_64", feature = "jit-opt"))]
pub(crate) mod feedback;
#[cfg(all(windows, target_arch = "x86_64"))]
mod helpers;
#[cfg(all(windows, target_arch = "x86_64"))]
mod memory;
#[cfg(all(test, windows, target_arch = "x86_64"))]
mod tests;
#[cfg(all(windows, target_arch = "x86_64"))]
mod x64;

use crate::op::Op;
use crate::value::Value;
use crate::vm::Vm;

/// How many calls and loop iterations a function runs in the interpreter before it's compiled. At 1000, invoices'
/// first forty interactions compiled twice as many functions, whose code (in an app, run about once between layout
/// and text) cost more than it saved, and 1 MB more memory; the benchmarks measure the same at either.
pub(crate) const THRESHOLD: u64 = 3000;

/// How much code an engine may have, at most. Past this, nothing more is compiled.
const CODE_LIMIT: usize = 64 * 1024 * 1024;

/// What a call from compiled code to a function that isn't compiled yet counts for (see [`Jit::count_called`]).
const CALLED_WEIGHT: u64 = 20;

/// The longest function that's compiled, in instructions.
const MAX_OPS: usize = 30_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    Off,
    On,
    Eager,
}

/// What `RSUI_JIT` asks for, read once.
pub(crate) fn mode() -> Mode {
    use std::sync::atomic::{AtomicU8, Ordering};
    static MODE: AtomicU8 = AtomicU8::new(0);
    match MODE.load(Ordering::Relaxed) {
        0 => {
            let available = cfg!(all(windows, target_arch = "x86_64"));
            let mode = match std::env::var("RSUI_JIT").as_deref() {
                _ if !available => Mode::Off,
                Ok("0" | "off") => Mode::Off,
                Ok("eager") => Mode::Eager,
                _ => Mode::On,
            };
            MODE.store(mode as u8 + 1, Ordering::Relaxed);
            mode
        }
        1 => Mode::Off,
        2 => Mode::On,
        _ => Mode::Eager,
    }
}

/// Whether `RSUI_JIT_STRICT=1` asks for a bail-out of the generic helper (an instruction it doesn't do, which is a
/// bug) to end the process, so that a test run shows each one as a crash.
pub(crate) fn strict() -> bool {
    use std::sync::atomic::{AtomicU8, Ordering};
    static STRICT: AtomicU8 = AtomicU8::new(0);
    match STRICT.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var_os("RSUI_JIT_STRICT").is_some_and(|value| value != "0");
            STRICT.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        known => known == 2,
    }
}

fn logging() -> bool {
    log_level() > 0
}

/// Whether `RSUI_JIT_LOG=2` asks for each function's instructions, and its calls compiled in line, as it's compiled.
fn listing() -> bool {
    log_level() > 1
}

/// What `RSUI_JIT_LOG` says, read once: 0, 1, or 2 for `2`.
fn log_level() -> u8 {
    use std::sync::atomic::{AtomicU8, Ordering};
    static LOG: AtomicU8 = AtomicU8::new(0);
    match LOG.load(Ordering::Relaxed) {
        0 => {
            let level = match std::env::var_os("RSUI_JIT_LOG") {
                Some(value) if value == "2" => 2,
                Some(value) if value != "0" => 1,
                _ => 0,
            };
            LOG.store(level + 1, Ordering::Relaxed);
            level
        }
        known => known - 1,
    }
}

/// What a new function's slot starts as (see [`Jit::slots`]) in this mode, with this threshold: how many times it has
/// to run first, or 0 if it never will be compiled.
fn first_slot(mode: Mode, threshold: u64) -> u64 {
    match mode {
        Mode::Off => 0,
        Mode::On => 2 * threshold + 1,
        Mode::Eager => 1,
    }
}


/// What the JIT keeps for one engine.
pub(crate) struct Jit {
    /// Where Rust's stack can go down to before compiled code stops calling other compiled code (which uses it),
    /// and leaves the call to the interpreter (which doesn't). Worked out each time compiled code is entered.
    pub(crate) floor: usize,
    /// What a helper hands back to compiled code: the error it threw, or a value it worked out.
    pub(crate) value: Value,
    /// For each function, by its proto, what the JIT knows of it:
    ///
    /// - 0: it's never to be compiled;
    /// - `2n + 1`, below `CODE_MIN`: it's to be compiled once it's run `n` more times, calls and loops;
    /// - the address of its code's entry, for compiled code to call: odd if the interpreter and Rust should go into
    ///   that code too, as the entry starts with a one-byte `nop`, and even if they're better off interpreting it.
    ///
    /// So the interpreter only has to look further when the low bit is set. The list is as long as the list of
    /// protos, and never moves, so that the interpreter and compiled code can read it without looking.
    pub(crate) slots: crate::arena::Arena<u64>,
    #[cfg(all(windows, target_arch = "x86_64"))]
    pub(crate) backend: Option<Box<Backend>>,
    /// The backend's way in from Rust, once there is one (which there is if anything's been compiled).
    #[cfg(all(windows, target_arch = "x86_64"))]
    trampoline: Option<x64::Trampoline>,
    /// Whether making the backend failed, so it isn't tried again.
    unavailable: bool,
    pub(crate) stats: Stats,
    /// What this engine does with functions compiled from now on: what `RSUI_JIT` says, unless a test says otherwise.
    pub(crate) mode: Mode,
    /// How many runs a function has before it's compiled: [`THRESHOLD`], unless a test asks for fewer.
    pub(crate) threshold: u64,
    /// Whether a function that's due is compiled on a thread of its own (see [`Worker`]), as it is unless a test asks
    /// for it to be compiled there and then, so that it runs the same way each time.
    pub(crate) background: bool,
    /// Whether to count which instructions go the generic way, for `RSUI_JIT_LOG`.
    pub(crate) counting: bool,
    pub(crate) generic: std::collections::HashMap<String, u64>,
    /// Where compiled code says the record is that `helpers::typed_set` is to fill in, or 0 (see `feedback::take`).
    #[cfg(feature = "jit-opt")]
    pub(crate) site: usize,
    /// The last functions helpers looked for records of (see `feedback::site_of`), by their protos' low bits: the proto,
    /// where its instructions start, its block, and where each instruction's record is.
    #[cfg(all(windows, target_arch = "x86_64", feature = "jit-opt"))]
    pub(crate) fb_last: [(u32, u32, usize, std::sync::Arc<[u32]>); 8],
}

#[derive(Default, Debug, Clone, Copy)]
pub(crate) struct Stats {
    pub(crate) compiled: u32,
    pub(crate) refused: u32,
    pub(crate) ops: u64,
    pub(crate) entries: u64,
    pub(crate) loop_entries: u64,
    /// Every time compiled code left a frame for the interpreter to carry on with.
    pub(crate) bails: u64,
    /// Of those, the ones where the generic helper met an instruction it doesn't do (`helpers::bail_at`), which
    /// shouldn't happen at all. The rest are calls made with Rust's stack already deep (see `Jit::floor`).
    pub(crate) unknown: u64,
    /// Frames compiled code left to the interpreter to their end, at a `for of` over a generator (see
    /// `helpers::iter_next`). Not bail-outs: the helper does it, and compiled code carries on from what it says.
    pub(crate) handed: u64,
    /// Generators carried on in their code, from Rust.
    pub(crate) resumes: u64,
    /// How long compiling took, in all, when `RSUI_JIT_LOG` asks.
    pub(crate) compiling: std::time::Duration,
    /// Calls compiled in line (see `x64::Gen::inline_call`), and how many times one of them made its callee's frame
    /// to go on in the callee's own code, which compiled code counts.
    pub(crate) inlined: u64,
    pub(crate) left_line: u64,
    /// Calls whose callee was guessed (see `x64::Look::known_callee`) that found another, which compiled code counts.
    pub(crate) missed: u64,
    /// Functions compiled again, as calls in them had nothing to go on the first time (see `Vm::jit_recompile`).
    pub(crate) recompiled: u64,
    /// Functions whose count towards the optimizing tier came to 0, and of those, the ones that were due then (see
    /// `feedback::due`).
    #[cfg(feature = "jit-opt")]
    pub(crate) counted_out: u64,
    #[cfg(feature = "jit-opt")]
    pub(crate) due: u64,
}

impl Jit {
    pub(crate) fn new() -> Self {
        Self {
            floor: 0,
            value: Value::UNDEFINED,
            slots: crate::arena::Arena::with_limit(1 << 24),
            #[cfg(all(windows, target_arch = "x86_64"))]
            backend: None,
            #[cfg(all(windows, target_arch = "x86_64"))]
            trampoline: None,
            unavailable: false,
            stats: Stats::default(),
            mode: mode(),
            threshold: THRESHOLD,
            background: true,
            counting: logging(),
            generic: std::collections::HashMap::new(),
            #[cfg(feature = "jit-opt")]
            site: 0,
            #[cfg(all(windows, target_arch = "x86_64", feature = "jit-opt"))]
            fb_last: std::array::from_fn(|_| (u32::MAX, 0, 0, std::sync::Arc::from([]))),
        }
    }

    /// How many bytes of code there are, and how much memory they take.
    pub(crate) fn code_bytes(&self) -> (usize, usize) {
        #[cfg(all(windows, target_arch = "x86_64"))]
        if let Some(backend) = &self.backend {
            return (backend.used, backend.committed);
        }
        (0, 0)
    }


    #[allow(dead_code)]
    pub(crate) fn stats(&self) -> Stats {
        self.stats
    }

    /// Where each compiled function's code is, and each of its instructions', for a profile (`Engine::jit_code`).
    pub(crate) fn code_map(vm: &Vm) -> Vec<crate::JitCode> {
        #[cfg(all(windows, target_arch = "x86_64"))]
        if let Some(backend) = &vm.jit.backend {
            return backend
                .code_map()
                // A function that's gone has no instructions to show.
                .filter(|&(proto_id, _)| {
                    vm.protos.get(proto_id as usize).is_some_and(|proto| proto.start != crate::compiled::FREED)
                })
                .map(|(proto_id, compiled)| {
                    let proto = &vm.protos[proto_id as usize];
                    let name = vm.atom_text(proto.name);
                    let line = proto.lines.first().map_or(0, |&(_, line)| line);
                    let start = proto.start as usize;
                    let ops = compiled
                        .offsets
                        .iter()
                        .zip(compiled.cold.iter())
                        .enumerate()
                        .map(|(at, (&hot, &cold))| (hot, cold, format!("{:?}", vm.code[start + at])))
                        .collect();
                    crate::JitCode {
                        name: format!(
                            "{} ({}:{line})",
                            if name.is_empty() { "<anonymous>" } else { &name },
                            vm.atom_text(proto.file)
                        ),
                        start: compiled.address,
                        end: compiled.address + compiled.bytes,
                        ops,
                    }
                })
                .collect();
        }
        let _ = vm;
        Vec::new()
    }

    /// Counts an instruction that compiled code had the generic helper do, by its name.
    #[cold]
    pub(crate) fn count_generic(&mut self, op: &Op) {
        let text = format!("{op:?}");
        let name = text.split([' ', '{']).next().unwrap_or_default();
        *self.generic.entry(name.to_owned()).or_default() += 1;
    }

    /// Counts a call that compiled code made the long way, through `helpers::call` and the interpreter, as the
    /// function isn't compiled yet. That costs about what twenty instructions in the interpreter do, and a function
    /// compiled code calls is likely called as often as its caller, so such a call counts for [`CALLED_WEIGHT`]: in an
    /// app, compiled functions called those that weren't yet about 2,700 times in the first rounds of using it, each
    /// starting the interpreter afresh, which made those rounds slower than with no compiling at all.
    pub(crate) fn count_called(&mut self, proto: u32) {
        let slot = self.slots[proto as usize];
        if slot.wrapping_sub(3) < COUNTING {
            // It stays odd, and at least 1 (due).
            self.slots[proto as usize] = slot.saturating_sub(2 * (CALLED_WEIGHT - 1)).max(1);
        }
    }

    /// A proto was added: its slot, so there's one for each.
    pub(crate) fn add_proto(&mut self, protos: usize) {
        self.slots.push(first_slot(self.mode, self.threshold));
        debug_assert_eq!(self.slots.len(), protos);
    }

    /// A proto that was waiting to be compiled has been, or its number is another's now: its count starts again.
    pub(crate) fn reset(&mut self, proto: u32) {
        self.slots[proto as usize] = first_slot(self.mode, self.threshold);
    }

    /// A proto has gone, as nothing could run it any more (see `compiled.rs`). What was waiting for its loop goes with
    /// it; what machine code it has is kept, as its number is never another's then.
    pub(crate) fn forget(&mut self, proto: u32) {
        #[cfg(all(windows, target_arch = "x86_64"))]
        if let Some(backend) = self.backend.as_deref_mut() {
            backend.deferred.remove(&proto);
        }
        let _ = proto;
    }
}


/// What the JIT keeps for an engine once it's compiling, on the engine's thread.
#[cfg(all(windows, target_arch = "x86_64"))]
#[derive(Default)]
pub(crate) struct Backend {
    /// What writes the code, here on the engine's thread, when every function is compiled the first time it's
    /// called (`RSUI_JIT=eager`), or if there's no thread to be had.
    writer: Option<x64::Writer>,
    /// Else the thread that writes it.
    worker: Option<Worker>,
    /// What keeps the memory the worker writes code in there, while this engine can run it.
    code: Option<std::sync::Arc<memory::Views>>,
    /// What's known of each function compiled: its newest code.
    pub(crate) compiled: std::collections::HashMap<u32, x64::Compiled, foldhash::fast::FixedState>,
    /// What's known of code that's been compiled again (see `Vm::jit_recompile`), which frames may still be running
    /// and whose counts it still counts down, with the function it's for.
    retired: Vec<(u32, x64::Compiled)>,
    /// The functions given to the worker that it hasn't finished, whose slots say they're due meanwhile.
    queued: std::collections::HashSet<u32, foldhash::fast::FixedState>,
    /// The functions with a loop that were due as they were called, and are compiled at their loop's next turn
    /// instead (see `Vm::jit_compile`).
    deferred: std::collections::HashSet<u32, foldhash::fast::FixedState>,
    /// How many bytes of code there are, and how much memory they have.
    used: usize,
    committed: usize,
    /// What's known of each function whose count towards the optimizing tier has come to 0 (see `feedback::due`).
    #[cfg(feature = "jit-opt")]
    waiting: std::collections::HashMap<u32, feedback::Waiting, foldhash::fast::FixedState>,
}

#[cfg(all(windows, target_arch = "x86_64"))]
impl Backend {
    /// For a profile: each function compiled, with where its code is and where each of its instructions' hot and
    /// cold code starts, from there.
    fn code_map(&self) -> impl Iterator<Item = (u32, &x64::Compiled)> {
        self.compiled
            .iter()
            .map(|(&proto, compiled)| (proto, compiled))
            .chain(self.retired.iter().map(|(proto, compiled)| (*proto, compiled)))
    }
}

/// The thread that compiles an engine's functions while the engine carries on (interpreting them meanwhile), so that
/// an app isn't held up for the 10-100 us each takes, in the middle of what someone's doing. It's given a job for
/// each, and hands back what came of it, which the engine puts in place the next time it calls the function. A job
/// has all it needs, so the engine never waits for the thread: when it goes, the thread finishes what it's doing and
/// ends. (Waiting for it there could hang: an app's engine goes as its thread ends, when Windows won't let a thread
/// start or end until that's done.)
#[cfg(all(windows, target_arch = "x86_64"))]
struct Worker {
    shared: std::sync::Arc<Shared>,
}

/// What the engine and its worker share: a lock and a condition variable, which the engine already has for reading
/// files ahead, rather than channels, which were most of the 68 KB this added to the program.
#[cfg(all(windows, target_arch = "x86_64"))]
#[derive(Default)]
struct Shared {
    queue: std::sync::Mutex<Queue>,
    /// Wakes the worker when there's a job, or the engine has gone.
    given: std::sync::Condvar,
    /// Whether there's anything in `Queue::done`, so the engine can look without taking the lock.
    finished: std::sync::atomic::AtomicBool,
}

#[cfg(all(windows, target_arch = "x86_64"))]
#[derive(Default)]
struct Queue {
    jobs: std::collections::VecDeque<x64::Job>,
    done: Vec<Message>,
    /// The engine has gone.
    closed: bool,
}

#[cfg(all(windows, target_arch = "x86_64"))]
impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Hands something back to the engine.
    fn finish(&self, message: Message) {
        self.lock().done.push(message);
        self.finished.store(true, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
enum Message {
    /// The worker has made its writer: the way into its code, and what keeps its memory there.
    Ready(x64::Trampoline, std::sync::Arc<memory::Views>),
    /// There's no JIT to be had here.
    Unavailable,
    Done(x64::Done),
}

#[cfg(all(windows, target_arch = "x86_64"))]
impl Worker {
    /// Starts the thread, which makes the writer first. `None` if there's no thread to be had.
    fn start(bases: x64::Bases, timed: bool) -> Option<Self> {
        let shared = std::sync::Arc::new(Shared::default());
        let theirs = shared.clone();
        std::thread::Builder::new()
            .name("rsui-js jit".into())
            .stack_size(1 << 20)
            .spawn(move || {
                let Some((mut writer, trampoline)) = x64::Writer::new(bases, CODE_LIMIT, timed) else {
                    theirs.finish(Message::Unavailable);
                    return;
                };
                theirs.finish(Message::Ready(trampoline, writer.keep()));
                loop {
                    let job = {
                        let mut queue = theirs.lock();
                        loop {
                            if let Some(job) = queue.jobs.pop_front() {
                                break job;
                            }
                            if queue.closed {
                                return;
                            }
                            queue = theirs.given.wait(queue).unwrap_or_else(std::sync::PoisonError::into_inner);
                        }
                    };
                    theirs.finish(Message::Done(writer.compile(&job)));
                }
            })
            .ok()?;
        Some(Self { shared })
    }

    fn send(&self, job: x64::Job) {
        self.shared.lock().jobs.push_back(job);
        self.shared.given.notify_one();
    }

    /// What the worker has handed back since the last time.
    fn take(&self) -> Vec<Message> {
        if !self.shared.finished.swap(false, std::sync::atomic::Ordering::Acquire) {
            return Vec::new();
        }
        std::mem::take(&mut self.shared.lock().done)
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
impl Drop for Worker {
    fn drop(&mut self) {
        // The thread finishes what it's doing and ends; nothing waits for it.
        self.shared.lock().closed = true;
        self.shared.given.notify_one();
    }
}

impl Drop for Jit {
    fn drop(&mut self) {
        if logging() && (self.stats.compiled > 0 || self.stats.refused > 0) {
            let (used, committed) = self.code_bytes();
            eprintln!(
                "jit: {} functions compiled ({} ops, {:.2} MB of code, {:.2} MB committed, in {:.2} ms), {} refused; {} entries, {} at loops, {} generators carried on, {} bail-outs ({} at an instruction the generic way doesn't do, {} deep in the stack), {} frames left to the interpreter at a generator",
                self.stats.compiled,
                self.stats.ops,
                used as f64 / 1048576.0,
                committed as f64 / 1048576.0,
                self.stats.compiling.as_secs_f64() * 1e3,
                self.stats.refused,
                self.stats.entries,
                self.stats.loop_entries,
                self.stats.resumes,
                self.stats.bails,
                self.stats.unknown,
                self.stats.bails - self.stats.unknown,
                self.stats.handed,
            );
            if self.stats.inlined > 0 || self.stats.recompiled > 0 {
                eprintln!(
                    "jit: {} calls compiled in line, which went on in the callee {} times; {} functions compiled again; calls that found another callee than guessed: {}",
                    self.stats.inlined, self.stats.left_line, self.stats.recompiled, self.stats.missed
                );
            }
            #[cfg(all(windows, target_arch = "x86_64", feature = "jit-opt"))]
            self.say_feedback();
            let mut generic: Vec<_> = self.generic.iter().collect();
            generic.sort_by(|a, b| b.1.cmp(a.1));
            let shown: Vec<String> = generic
                .iter()
                .filter(|(name, _)| !name.starts_with("call: "))
                .take(12)
                .map(|(name, count)| format!("{name} {count}"))
                .collect();
            if !shown.is_empty() {
                eprintln!("jit: the generic way most: {}", shown.join(", "));
            }
            let calls: Vec<String> = generic
                .iter()
                .filter_map(|(name, count)| Some(format!("{} {count}", name.strip_prefix("call: ")?)))
                .take(8)
                .collect();
            if !calls.is_empty() {
                eprintln!("jit: calls made, and reads done, the long way most: {}", calls.join(", "));
            }
        }
    }
}

/// For tests: which instruction has the record at `at`.
#[cfg(all(test, windows, target_arch = "x86_64", feature = "jit-opt"))]
fn at_of(layout: &feedback::Layout, at: u32) -> usize {
    layout.offsets.iter().position(|&offset| offset == at).expect("a record is an instruction's")
}

/// For tests: the records of the compiled function named `name`, each with the name of the instruction it's for.
#[cfg(all(test, windows, target_arch = "x86_64", feature = "jit-opt"))]
#[allow(unsafe_code)]
pub(crate) fn feedback_of(vm: &Vm, name: &str) -> Vec<(String, Vec<u8>)> {
    let Some(backend) = &vm.jit.backend else {
        return Vec::new();
    };
    let Some((&proto, compiled)) = backend
        .compiled
        .iter()
        .find(|(proto, _)| vm.atom_text(vm.protos[**proto as usize].name) == name)
    else {
        return Vec::new();
    };
    if compiled.feedback == 0 {
        return Vec::new();
    }
    let start = vm.protos[proto as usize].start as usize;
    let ops = &vm.code[start..start + compiled.offsets.len()];
    let layout = feedback::layout(ops);
    ops.iter()
        .zip(layout.offsets.iter())
        .filter(|&(_, &at)| at != feedback::NONE)
        .map(|(op, &at)| {
            let kind = feedback::kind_of(op).expect("a record has a kind");
            let text = format!("{op:?}");
            let name = text.split([' ', '{']).next().unwrap_or_default().to_owned();
            // SAFETY: the block is laid out for these instructions, and there while the code is.
            let bytes = unsafe {
                std::slice::from_raw_parts((compiled.feedback + at as usize) as *const u8, kind.size() as usize)
            };
            // As the record would be with every bit in one place, and a call's known callees as what they are.
            let index = (at_of(&layout, at)) as u32;
            let knowns = compiled
                .feedback_knowns
                .binary_search_by_key(&index, |&(at, _)| at)
                .map_or(&[][..], |found| &compiled.feedback_knowns[found].1[..]);
            (name, feedback::logical(kind, bytes, knowns))
        })
        .collect()
}

#[cfg(all(windows, target_arch = "x86_64", feature = "jit-opt"))]
impl Jit {
    /// For `RSUI_OPT_LOG`: what the functions' feedback came to (see `feedback::Summary`).
    #[allow(unsafe_code)]
    fn say_feedback(&self) {
        if !feedback::logging() {
            return;
        }
        let mut summary = feedback::Summary::default();
        if let Some(backend) = &self.backend {
            for compiled in backend.compiled.values() {
                if let Some(ops) = &compiled.feedback_ops {
                    // SAFETY: the block was laid out for these instructions, and is there while the code is.
                    unsafe { summary.add(ops, compiled.feedback, &compiled.feedback_knowns) };
                }
            }
        }
        eprintln!(
            "opt: {} functions with feedback ({} bytes), {} counted out, {} due; numbers: {:?} (never, ints, doubles or results that aren't ints, other); branches: {:?} (never, taken only, fell only, both); calls: {:?} (never recorded, one callee, more, something else too); new: {:?} (never, one shape, more); elements: {:?} (plain, holes or past the end, typed, by name, other)",
            summary.functions,
            summary.bytes,
            self.stats.counted_out,
            self.stats.due,
            summary.numbers,
            summary.branches,
            summary.calls,
            summary.news,
            summary.elems,
        );
    }
}

/// What the interpreter does after compiled code has run: carry on from the frame on top (which compiled code has
/// returned to, or left for it), hand back what the frame at `stop` returned, or deal with an error.
pub(crate) enum Next {
    Interpret,
    Return(Value),
    Throw(Value),
}

/// Where an instruction can jump to, relative to itself, if it's a jump.
pub(crate) fn jump_offset(op: &Op) -> Option<i32> {
    Some(match *op {
        Op::Jump { offset } | Op::Loop { offset } => offset,
        Op::JumpIfTrue { offset, .. }
        | Op::JumpIfFalse { offset, .. }
        | Op::JumpIfNullish { offset, .. }
        | Op::JumpIfNotNullish { offset, .. }
        | Op::JumpIfNotUndefined { offset, .. }
        | Op::JumpIfUndefined { offset, .. }
        | Op::JumpIfNull { offset, .. }
        | Op::JumpIfNotNull { offset, .. } => offset,
        Op::JumpLt { offset, .. }
        | Op::JumpLe { offset, .. }
        | Op::JumpGt { offset, .. }
        | Op::JumpGe { offset, .. }
        | Op::JumpNotLt { offset, .. }
        | Op::JumpNotLe { offset, .. }
        | Op::JumpNotGt { offset, .. }
        | Op::JumpNotGe { offset, .. }
        | Op::JumpStrictEq { offset, .. }
        | Op::JumpStrictNe { offset, .. }
        | Op::JumpEq { offset, .. }
        | Op::JumpNe { offset, .. }
        | Op::JumpLtInt { offset, .. }
        | Op::JumpLeInt { offset, .. }
        | Op::JumpGtInt { offset, .. }
        | Op::JumpGeInt { offset, .. }
        | Op::JumpNotLtInt { offset, .. }
        | Op::JumpNotLeInt { offset, .. }
        | Op::JumpNotGtInt { offset, .. }
        | Op::JumpNotGeInt { offset, .. }
        | Op::JumpStrictEqInt { offset, .. }
        | Op::JumpStrictNeInt { offset, .. }
        | Op::JumpTypeIs { offset, .. }
        | Op::JumpTypeIsNot { offset, .. }
        | Op::LoopLt { offset, .. }
        | Op::LoopLe { offset, .. }
        | Op::LoopGt { offset, .. }
        | Op::LoopGe { offset, .. }
        | Op::LoopLtInt { offset, .. }
        | Op::LoopLeInt { offset, .. }
        | Op::LoopGtInt { offset, .. }
        | Op::LoopGeInt { offset, .. }
        | Op::LoopStrictNe { offset, .. }
        | Op::LoopStrictNeInt { offset, .. }
        | Op::IncLoopLt { offset, .. }
        | Op::IncLoopLtInt { offset, .. }
        | Op::IncLoopLtLength { offset, .. }
        | Op::IterNext { offset, .. }
        | Op::IterResult { offset, .. } => i32::from(offset),
        _ => return None,
    })
}

/// How many instructions a function has that can be reached: up to the first return or throw that nothing jumps or
/// catches past. `None` if a jump goes somewhere odd, or it's too long.
pub(crate) fn extent(vm: &Vm, proto: &crate::op::Proto) -> Option<usize> {
    let start = proto.start as usize;
    // What a `catch` or `finally` covers and where it goes are only reached through the list of handlers.
    let mut furthest = proto
        .handlers
        .iter()
        .map(|handler| handler.target.max(handler.end) as usize)
        .max()
        .unwrap_or(0);
    let mut at = 0usize;
    loop {
        let op = vm.code.get(start + at)?;
        if let Some(offset) = jump_offset(op) {
            let target = at as i64 + 1 + i64::from(offset);
            if target < 0 {
                return None;
            }
            furthest = furthest.max(target as usize);
        }
        let leaves = matches!(op, Op::Return { .. } | Op::ReturnUndefined | Op::Throw { .. });
        if leaves && at >= furthest {
            return Some(at + 1);
        }
        at += 1;
        if at > MAX_OPS {
            return None;
        }
    }
}

/// Whether a function can be compiled: one that's compiled to bytecode, and not async (a generator is). If not, why
/// not.
fn compilable(vm: &Vm, proto_id: u32) -> Result<(), &'static str> {
    use crate::op::{IS_ASYNC, NOT_COMPILED};
    let proto = &vm.protos[proto_id as usize];
    if proto.start == NOT_COMPILED {
        return Err("not compiled to bytecode yet");
    }
    if proto.flags & IS_ASYNC != 0 {
        return Err("async");
    }
    let Some(length) = extent(vm, proto) else {
        return Err("too long, or a jump out of it");
    };
    let ops = &vm.code[proto.start as usize..proto.start as usize + length];
    for (at, op) in ops.iter().enumerate() {
        if matches!(op, Op::Await { .. }) {
            return Err("awaits");
        }
        if matches!(op, Op::Step { .. }) {
            return Err("it's being debugged");
        }
        if let Some(offset) = jump_offset(op) {
            let target = at as i64 + 1 + i64::from(offset);
            if target < 0 || target as usize >= length {
                return Err("a jump past its end");
            }
        }
    }
    if proto.handlers.iter().any(|handler| handler.target as usize >= length) {
        return Err("a handler past its end");
    }
    Ok(())
}

#[cfg(all(windows, target_arch = "x86_64"))]
use x64::CODE_MIN;

/// Whether a function's code is worth going into from the interpreter or Rust, rather than interpreting it, which is
/// cheaper for something short with no loop or call. Compiled code calls it directly either way.
#[inline(always)]
fn worth_entering(code: u64) -> bool {
    code & 1 != 0
}
#[cfg(not(all(windows, target_arch = "x86_64")))]
const CODE_MIN: u64 = u64::MAX;

/// A slot `s` with `s - 3` below this is counting, and not due yet: the interpreter takes 2 off it itself.
pub(crate) const COUNTING: u64 = CODE_MIN - 3;

/// Whether a function's slot says it has code.
#[inline(always)]
pub(crate) fn has_code(slot: u64) -> bool {
    slot >= CODE_MIN
}

impl Vm {
    /// The interpreter has just made a frame for a function whose `jit` slot isn't 0. Counts the call, compiles the
    /// function if it's due, and runs its code if it has some.
    #[inline(never)]
    pub(crate) fn jit_enter(&mut self, stop: usize) -> Next {
        let proto = self.frames.last().expect("a frame was just made").proto;
        match self.jit_code(proto, 1, false) {
            Some(code) if worth_entering(code) => self.jit_run_body(stop, code),
            _ => Next::Interpret,
        }
    }

    /// The interpreter has just jumped back to the top of a loop, with the frame's `pc` there. Counts it, compiles
    /// the function if it's due, and carries on in its code from there if it has some.
    #[inline(never)]
    pub(crate) fn jit_loop(&mut self, stop: usize) -> Next {
        let frame = *self.frames.last().expect("a loop runs in a frame");
        // A frame compiled code has left to the interpreter to its end stays there.
        if frame.flags & crate::vm::INTERPRETED != 0 {
            return Next::Interpret;
        }
        if self.jit_code(frame.proto, 1, true).is_none() {
            return Next::Interpret;
        }
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            let Some(target) = self.jit_entry(frame.proto, frame.pc) else {
                return Next::Interpret;
            };
            self.jit.stats.loop_entries += 1;
            return self.jit_run(stop, target);
        }
        #[allow(unreachable_code)]
        {
            let _ = stop;
            Next::Interpret
        }
    }

    /// The function's code, once it's been run often enough to be compiled; `weight` more runs are counted, at the top
    /// of one of its loops if `at_loop`, else as it's called.
    fn jit_code(&mut self, proto: u32, weight: u64, at_loop: bool) -> Option<u64> {
        let slot = self.jit.slots[proto as usize];
        if slot >= CODE_MIN {
            return Some(slot);
        }
        if slot == 0 {
            return None;
        }
        if slot > 2 * weight {
            self.jit.slots[proto as usize] = slot - 2 * weight;
            return None;
        }
        self.jit_compile(proto, at_loop)
    }

    /// Compiles a function that's due, or has the worker compile it, and says what its slot says then: its code, if
    /// it has some yet. A function the worker is compiling stays due meanwhile, so that each call of it looks again.
    ///
    /// A function with a loop that's due as it's called (not `at_loop`) is compiled at its loop's next turn instead,
    /// once, with `RSUI_JIT=1`: what the code knows is what's in its frame's registers then (see
    /// `x64::Gen::looks_double`). It stays due meanwhile; if that turn doesn't come in this call, it's compiled as it's
    /// next called. Whether it was due as it was called or at its loop came down to how its calls and turns added up
    /// to the threshold, and float_math's `advance` compiled as it was called took 15% longer.
    fn jit_compile(&mut self, proto: u32, at_loop: bool) -> Option<u64> {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            if self.jit.unavailable {
                self.jit_refuse(proto);
                return None;
            }
            let backend = self.jit.backend.get_or_insert_with(Box::default);
            // Compiled already, ahead of being due, as a callee of a function compiled before it.
            if let Some(code) = backend.compiled.get(&proto).map(x64::Compiled::slot) {
                self.jit_give(proto, code);
                return Some(code);
            }
            if backend.queued.contains(&proto) {
                return self.jit_collect(proto);
            }
            if let Err(why) = compilable(self, proto) {
                if logging() {
                    let name = self.atom_text(self.protos[proto as usize].name);
                    eprintln!("jit: not compiling {}: {why}", if name.is_empty() { "<anonymous>" } else { &name });
                }
                self.jit_refuse(proto);
                return None;
            }
            if !at_loop && self.jit.mode == Mode::On && self.has_loop(proto) {
                let backend = self.jit.backend.as_deref_mut()?;
                if backend.deferred.insert(proto) {
                    return None;
                }
            }
            let Some(jobs) = self.jit_jobs(proto, 0) else {
                self.jit_refuse(proto);
                return None;
            };
            self.jit_send(jobs, proto);
            self.jit_code_of(proto)
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        {
            let _ = (proto, at_loop, compilable as fn(&Vm, u32) -> Result<(), &'static str>);
            None
        }
    }

    /// Whether a function has a loop: a jump back.
    #[cfg(all(windows, target_arch = "x86_64"))]
    fn has_loop(&self, proto: u32) -> bool {
        let proto = &self.protos[proto as usize];
        let Some(length) = extent(self, proto) else {
            return false;
        };
        let ops = &self.code[proto.start as usize..proto.start as usize + length];
        ops.iter().any(|op| jump_offset(op).is_some_and(|offset| offset < 0))
    }

    /// The jobs of compiling a function for the `generation`th time (0 the first): first those of the callees worth
    /// compiling ahead of it that don't have code (see `x64::Look::inline_candidates`), which its calls can then call
    /// directly or compile in line, then its own. `None` if it can't be compiled.
    #[cfg(all(windows, target_arch = "x86_64"))]
    fn jit_jobs(&mut self, proto: u32, generation: u8) -> Option<Vec<x64::Job>> {
        let candidates = x64::Look::new(self, proto)?.inline_candidates(self.jit.threshold);
        let mut jobs = Vec::new();
        for callee in candidates {
            let backend = self.jit.backend.as_deref()?;
            if backend.compiled.contains_key(&callee)
                || backend.queued.contains(&callee)
                || self.jit.slots[callee as usize] == 0
                || compilable(self, callee).is_err()
            {
                continue;
            }
            if let Some(job) = x64::Job::new(self, callee, 0, false) {
                jobs.push(job);
            }
        }
        jobs.push(x64::Job::new(self, proto, generation, true)?);
        Some(jobs)
    }

    /// Compiles what the jobs are for: on the worker's thread, unless every function is compiled as it's first called
    /// (which is for tests), or a test asks for it to be there and then, or there's no thread to be had. `proto` is the
    /// function they're for, the last.
    #[cfg(all(windows, target_arch = "x86_64"))]
    fn jit_send(&mut self, jobs: Vec<x64::Job>, proto: u32) {
        if self.jit.mode != Mode::Eager && self.jit.background {
            let bases = x64::Bases::of(self);
            let Some(backend) = self.jit.backend.as_deref_mut() else {
                return;
            };
            if backend.writer.is_none() {
                if backend.worker.is_none() {
                    backend.worker = Worker::start(bases, logging());
                }
                if let Some(worker) = &backend.worker {
                    for job in jobs {
                        backend.queued.insert(job.proto_id);
                        worker.send(job);
                    }
                    self.jit_collect(proto);
                    return;
                }
            }
        }
        self.jit_compile_here(jobs);
    }

    /// Compiles what the jobs are for on the engine's thread, making the writer first if there isn't one.
    #[cfg(all(windows, target_arch = "x86_64"))]
    fn jit_compile_here(&mut self, jobs: Vec<x64::Job>) {
        let bases = x64::Bases::of(self);
        let backend = self.jit.backend.get_or_insert_with(Box::default);
        if backend.writer.is_none() {
            match x64::Writer::new(bases, CODE_LIMIT, logging()) {
                Some((writer, trampoline)) => {
                    backend.writer = Some(writer);
                    self.jit.trampoline = Some(trampoline);
                }
                None => {
                    self.jit.unavailable = true;
                    for job in jobs.iter().filter(|job| job.give) {
                        self.jit_refuse(job.proto_id);
                    }
                    return;
                }
            }
        }
        for job in jobs {
            let Some(writer) = self.jit.backend.as_deref_mut().and_then(|backend| backend.writer.as_mut()) else {
                return;
            };
            let done = writer.compile(&job);
            self.jit_install(done);
        }
    }

    /// Puts in place what the worker has finished, and says what function `proto`'s slot says then.
    #[cfg(all(windows, target_arch = "x86_64"))]
    fn jit_collect(&mut self, proto: u32) -> Option<u64> {
        let mut installed = false;
        let messages = self
            .jit
            .backend
            .as_ref()
            .and_then(|backend| backend.worker.as_ref())
            .map(Worker::take)
            .unwrap_or_default();
        for message in messages {
            match message {
                Message::Ready(trampoline, code) => {
                    self.jit.trampoline = Some(trampoline);
                    if let Some(backend) = self.jit.backend.as_deref_mut() {
                        backend.code = Some(code);
                    }
                }
                Message::Unavailable => {
                    self.jit.unavailable = true;
                    let queued: Vec<u32> = self
                        .jit
                        .backend
                        .as_mut()
                        .map(|backend| backend.queued.drain().collect())
                        .unwrap_or_default();
                    for waiting in queued {
                        if !has_code(self.jit.slots[waiting as usize]) {
                            self.jit_refuse(waiting);
                        }
                    }
                }
                Message::Done(done) => {
                    self.jit_install(done);
                    installed = true;
                }
            }
        }
        if installed {
            // Code another processor wrote is about to run on this one, which wants an instruction that has it see
            // those instructions as they are now (Intel's manual, on code one processor writes for another to run).
            // SAFETY: every x86-64 has CPUID.
            #[allow(unused_unsafe, unsafe_code)]
            unsafe {
                std::arch::x86_64::__cpuid(0);
            }
        }
        self.jit_code_of(proto)
    }

    /// Puts a compiled function in place. Its slot says where its code is from now on, if the job said to give it the
    /// code, or the function is due by now; a callee compiled ahead of its caller is given its code when it's due.
    /// Code compiled again takes the place of the function's old code, which is kept, as frames may be running it.
    #[cfg(all(windows, target_arch = "x86_64"))]
    fn jit_install(&mut self, done: x64::Done) {
        let proto = done.proto_id;
        self.jit.stats.compiling += done.took;
        let Some(backend) = self.jit.backend.as_deref_mut() else {
            return;
        };
        backend.queued.remove(&proto);
        backend.used = done.used;
        backend.committed = done.committed;
        let Some((slot, compiled)) = done.code else {
            if done.generation > 0 {
                // Compiling it again didn't work: its code stays, and counts towards that no more.
                if let Some(old) = backend.compiled.get(&proto) {
                    old.counter.store(u64::MAX, std::sync::atomic::Ordering::Relaxed);
                }
            } else if done.give {
                self.jit_refuse(proto);
            }
            return;
        };
        self.jit.stats.inlined += u64::from(compiled.inlined);
        if logging() {
            let name = self.atom_text(self.protos[proto as usize].name);
            let file = self.atom_text(self.protos[proto as usize].file);
            eprintln!(
                "jit: compiled {}{} ({file}), {} ops, {} bytes, in {} us",
                if name.is_empty() { "<anonymous>" } else { &name },
                if done.generation > 0 { " again" } else if done.give { "" } else { " ahead" },
                compiled.offsets.len(),
                compiled.bytes,
                done.took.as_micros()
            );
            if listing() {
                for &(callee, at) in &done.inlines {
                    let callee = self.atom_text(self.protos[callee as usize].name);
                    eprintln!("jit: {callee} compiled in line in {name} at {at}");
                }
            }
        }
        let backend = self.jit.backend.as_deref_mut().expect("the backend is there");
        if done.generation > 0 {
            if let Some(old) = backend.compiled.insert(proto, compiled) {
                backend.retired.push((proto, old));
            }
            self.jit.stats.recompiled += 1;
            if has_code(self.jit.slots[proto as usize]) {
                self.jit.slots[proto as usize] = slot;
            }
            return;
        }
        // (Code already there, which nothing compiles twice, would be kept: other code may call it, and it counts.)
        if let Some(old) = backend.compiled.insert(proto, compiled) {
            backend.retired.push((proto, old));
        }
        // A slot of 1 is due (see `jit_code`).
        if done.give || self.jit.slots[proto as usize] == 1 {
            self.jit_give(proto, slot);
        }
    }

    /// A function is given its code: from now on its slot says where that is.
    #[cfg(all(windows, target_arch = "x86_64"))]
    fn jit_give(&mut self, proto: u32, slot: u64) {
        self.jit.stats.compiled += 1;
        if let Some(compiled) = self.jit.backend.as_deref().and_then(|backend| backend.compiled.get(&proto)) {
            self.jit.stats.ops += compiled.offsets.len() as u64;
        }
        self.jit.slots[proto as usize] = slot;
    }

    /// A function won't be compiled.
    fn jit_refuse(&mut self, proto: u32) {
        self.jit.slots[proto as usize] = 0;
        self.jit.stats.refused += 1;
    }

    /// The function's code, if its slot says it has some.
    fn jit_code_of(&self, proto: u32) -> Option<u64> {
        let slot = self.jit.slots[proto as usize];
        (slot >= CODE_MIN).then_some(slot)
    }

    /// A compiled function's count towards the optimizing tier has come to 0, as its code took one off at its entry
    /// and each loop's jump back: it's due if its feedback has settled (see `feedback::due`), which with no tier yet
    /// is only counted, and said with `RSUI_OPT_LOG`.
    #[cfg(all(windows, target_arch = "x86_64", feature = "jit-opt"))]
    #[allow(unsafe_code)]
    pub(crate) fn opt_due(&mut self, proto: u32) {
        self.jit.stats.counted_out += 1;
        let Some(backend) = self.jit.backend.as_deref_mut() else {
            return;
        };
        let Some(compiled) = backend.compiled.get(&proto) else {
            return;
        };
        let (block, size) = (compiled.feedback, compiled.feedback_size);
        if block == 0 {
            return;
        }
        let waiting = backend.waiting.entry(proto).or_default();
        // SAFETY: the block is this function's, laid out for its instructions, and there while its code is.
        if unsafe { feedback::due(waiting, block, size) } {
            self.jit.stats.due += 1;
            if feedback::logging() {
                let name = self.atom_text(self.protos[proto as usize].name);
                let file = self.atom_text(self.protos[proto as usize].file);
                eprintln!(
                    "opt: due: {} ({file}), {} due so far",
                    if name.is_empty() { "<anonymous>" } else { &name },
                    self.jit.stats.due
                );
            }
        }
    }

    /// The calls of a compiled function that knew nothing of what they'd call when it was compiled have counted
    /// `counter` down to 0, at the call at `pc` in the frame on top, which is running the code that counts it. Gives
    /// back where to go on in the function's newest code, at that call, or 0 to go on where it is.
    ///
    /// - If that's code compiled before the function's newest, it goes on in the newest.
    /// - Else, if a call that knew nothing would know what it calls now (see `x64::Look::call_knows`), the function is
    ///   compiled again, with what its caches and its frame know now: there and then if that's where functions are
    ///   compiled (the tests, and `RSUI_JIT=eager`), and it goes on in the new code; else on the worker, and the
    ///   count is set to look again soon, when it'll go on in the new code if that's there by then. Calls made
    ///   meanwhile go the way that works for any callee, which is only slower.
    /// - Else (a call of callbacks, or of natives) the count starts again, longer each time.
    #[allow(unsafe_code)]
    pub(crate) fn jit_recompile(&mut self, proto: u32, pc: u32, counter: *mut u64) -> usize {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
            // SAFETY: the counter is a compiled function's, which is kept as long as the engine, and its code gave it.
            let count = unsafe { AtomicU64::from_ptr(counter) };
            let at = (pc - self.protos[proto as usize].start) as usize;
            let newest = |vm: &Vm| {
                let compiled = vm.jit.backend.as_deref()?.compiled.get(&proto)?;
                let offset = *compiled.offsets.get(at)?;
                Some((compiled.address + offset as usize, std::ptr::eq(&*compiled.counter, count)))
            };
            let Some((target, current)) = newest(self) else {
                count.store(u64::MAX, Relaxed);
                return 0;
            };
            if !current {
                count.store(u64::MAX, Relaxed);
                return target;
            }
            let Some(backend) = self.jit.backend.as_deref() else {
                return 0;
            };
            if backend.queued.contains(&proto) {
                // Being compiled again on the worker: on in the new code if it's there.
                self.jit_collect(proto);
                return match newest(self) {
                    Some((target, false)) => {
                        count.store(u64::MAX, Relaxed);
                        target
                    }
                    _ => {
                        if self.jit.backend.as_deref().is_some_and(|backend| backend.queued.contains(&proto)) {
                            count.store(x64::RECOMPILE_POLL, Relaxed);
                        }
                        0
                    }
                };
            }
            let learned = {
                let compiled = &backend.compiled[&proto];
                x64::Look::new(self, proto).is_some_and(|look| {
                    compiled
                        .unguessed
                        .iter()
                        .any(|&at| look.call_knows(at as usize, |known| backend.compiled.contains_key(&known)))
                })
            };
            if !learned {
                let backend = self.jit.backend.as_deref_mut().expect("the backend is there");
                let compiled = backend.compiled.get_mut(&proto).expect("the function has code");
                compiled.waits = (compiled.waits + 1).min(10);
                count.store(x64::RECOMPILE_AFTER << compiled.waits, Relaxed);
                return 0;
            }
            let generation = backend.compiled[&proto].generation + 1;
            let Some(jobs) = self.jit_jobs(proto, generation) else {
                count.store(u64::MAX, Relaxed);
                return 0;
            };
            count.store(x64::RECOMPILE_POLL, Relaxed);
            self.jit_send(jobs, proto);
            match newest(self) {
                Some((target, false)) => {
                    count.store(u64::MAX, Relaxed);
                    target
                }
                _ => 0,
            }
        }
        #[cfg(not(all(windows, target_arch = "x86_64")))]
        {
            let _ = (proto, pc, counter);
            0
        }
    }

    /// Runs the frame on top until it returns, as `run_loop(stop)` does, in its function's compiled code if it has
    /// some and the frame is at its start. For a call from Rust (a native calling back, a call compiled code leaves
    /// to a helper), which then needn't start the interpreter at all. Such a call is counted here, and the function
    /// compiled when it's due, so that the interpreter isn't left under the compiled code, which (in a debug build,
    /// where its frame is large) would use Rust's stack up for nothing.
    ///
    /// Going into compiled code from here costs about what starting the interpreter's loop does, so any function with
    /// code is run in it, however short.
    #[inline(always)]
    #[allow(unsafe_code)]
    pub(crate) fn run_frame(&mut self, stop: usize) -> crate::vm::Run<Value> {
        // SAFETY: there's a frame on top, its proto is in the list, and there's a slot for each proto.
        let (slot, fresh, proto) = unsafe {
            let frame = &*self.frames.as_ptr().add(self.frames.len() - 1);
            let proto = frame.proto as usize;
            (
                *self.jit.slots.as_ptr().add(proto),
                frame.pc == (*self.protos.as_ptr().add(proto)).start,
                frame.proto,
            )
        };
        if fresh {
            if slot >= CODE_MIN && slot & 1 != 0 {
                return self.jit_run_called(stop, slot);
            }
            if slot & 1 != 0 {
                if slot.wrapping_sub(3) >= COUNTING {
                    return self.run_counted(stop, proto);
                }
                // SAFETY: as above.
                unsafe { *self.jit.slots.as_mut_ptr().add(proto as usize) = slot - 2 };
            }
        } else if slot >= CODE_MIN {
            return self.jit_resume(stop);
        }
        self.run_loop(stop)
    }

    /// `run_frame` for a frame partway through a function with code: a generator carrying on where it stopped, or at
    /// a handler an error was thrown to. In its code, if it can be gone into there.
    #[inline(never)]
    fn jit_resume(&mut self, stop: usize) -> crate::vm::Run<Value> {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            let frame = *self.frames.last().expect("a frame is to run");
            // A frame compiled code has left to the interpreter to its end stays there.
            if frame.flags & crate::vm::INTERPRETED == 0
                && let Some(target) = self.jit_entry(frame.proto, frame.pc)
            {
                self.jit.stats.resumes += 1;
                return self.jit_run_from(stop, target);
            }
        }
        self.run_loop(stop)
    }

    /// Where in a function's code to go in at its instruction `pc`, if it can be gone into there.
    #[cfg(all(windows, target_arch = "x86_64"))]
    fn jit_entry(&self, proto: u32, pc: u32) -> Option<usize> {
        let start = self.protos[proto as usize].start;
        let compiled = self.jit.backend.as_ref()?.compiled.get(&proto)?;
        let at = pc.checked_sub(start)? as usize;
        let offset = *compiled.offsets.get(at)?;
        compiled.entries[at].then(|| compiled.address + offset as usize)
    }

    /// `run_frame` for a function whose slot is counting towards compiling it: counts the call, and if that compiles
    /// it, runs its code.
    #[inline(never)]
    fn run_counted(&mut self, stop: usize, proto: u32) -> crate::vm::Run<Value> {
        match self.jit_code(proto, 1, false) {
            Some(code) if worth_entering(code) => self.jit_run_called(stop, code),
            _ => self.run_loop(stop),
        }
    }

    /// `run_frame` for a function with code: runs it from its body.
    #[inline(never)]
    fn jit_run_called(&mut self, stop: usize, code: u64) -> crate::vm::Run<Value> {
        self.jit.stats.entries += 1;
        self.jit_run_from(stop, body_of(code))
    }

    /// Runs the frame on top from `target` in its code, until it's returned: as `run_loop(stop)` does.
    fn jit_run_from(&mut self, stop: usize, target: usize) -> crate::vm::Run<Value> {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            return match self.jit_go(target) {
                // The frame that was entered has returned.
                helpers::OK => Ok(self.jit.value),
                helpers::THROW => {
                    let error = self.jit.value;
                    if self.unwind(error, stop) {
                        self.run_loop(stop)
                    } else {
                        Err(error)
                    }
                }
                status => {
                    if status == helpers::BAIL {
                        self.jit.stats.bails += 1;
                    }
                    self.run_loop(stop)
                }
            };
        }
        #[allow(unreachable_code)]
        {
            let _ = target;
            self.run_loop(stop)
        }
    }

    /// Runs a function's code from its body, its frame being made already.
    fn jit_run_body(&mut self, stop: usize, code: u64) -> Next {
        self.jit.stats.entries += 1;
        self.jit_run(stop, body_of(code))
    }

    /// Runs compiled code from `target`, in the frame on top, and says what the interpreter does next.
    #[allow(unsafe_code)]
    fn jit_run(&mut self, stop: usize, target: usize) -> Next {
        #[cfg(all(windows, target_arch = "x86_64"))]
        {
            // SAFETY: there's a frame on top.
            let ret_dst = unsafe { (*self.frames.as_ptr().add(self.frames.len() - 1)).ret_dst };
            return match self.jit_go(target) {
                helpers::OK => {
                    let value = self.jit.value;
                    let depth = self.frames.len();
                    if depth == stop {
                        Next::Return(value)
                    } else {
                        // SAFETY: the frame returned to is on top.
                        unsafe {
                            let caller = &*self.frames.as_ptr().add(depth - 1);
                            *self.stack.as_mut_ptr().add(caller.base as usize + ret_dst as usize) = value;
                        }
                        Next::Interpret
                    }
                }
                helpers::BAIL => {
                    self.jit.stats.bails += 1;
                    Next::Interpret
                }
                helpers::NOT_ENTERED | helpers::LEFT => Next::Interpret,
                _ => Next::Throw(self.jit.value),
            };
        }
        #[allow(unreachable_code)]
        {
            let _ = (stop, target);
            Next::Interpret
        }
    }

    /// Runs compiled code from `target` in the frame on top, until that frame returns or the code leaves it: how it
    /// left (see `helpers`), with what it handed back in `jit.value`.
    #[cfg(all(windows, target_arch = "x86_64"))]
    #[inline(always)]
    #[allow(unsafe_code)]
    fn jit_go(&mut self, target: usize) -> u32 {
        let Some(trampoline) = self.jit.trampoline else {
            return helpers::NOT_ENTERED;
        };
        // SAFETY: the frame on top is inside the list of frames, and `target` is in code compiled for its function by
        // this engine's backend, which isn't changed or dropped while its code runs (compiling more only adds to its
        // code memory, which doesn't move).
        unsafe {
            let frame = self.frames.as_mut_ptr().add(self.frames.len() - 1);
            trampoline(self, frame, target)
        }
    }
}

/// Where a function's body starts, from its code's entry (its slot), as the header says (see `x64::HEADER`).
#[allow(unsafe_code)]
#[inline(always)]
fn body_of(code: u64) -> usize {
    // SAFETY: every function's code starts with the header, and `code` is the entry right after it.
    unsafe { (((code & !1) - 16) as *const u64).read() as usize }
}
