//! Optional current-thread CPU cycle attribution, entirely absent without
//! idle-probe. Set RSUI_NATIVE_CYCLES to a CSV path. Scopes are nested: inclusive
//! counts contain child scopes; exclusive counts subtract their inclusive cost.
//! Cycle counts are processor accounting, never elapsed time or nanoseconds.

use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

pub(crate) const SESSION: usize = 0;
pub(crate) const CALIBRATION: usize = 1;
pub(crate) const GET_MESSAGE: usize = 2;
pub(crate) const DISPATCH: usize = 3;
pub(crate) const PRESENT_DUE: usize = 4;
pub(crate) const WM_MOVE: usize = 5;
pub(crate) const MODIFIERS_READ: usize = 6;
pub(crate) const MODIFIERS_FEED: usize = 7;
pub(crate) const MOVE_FEED: usize = 8;
pub(crate) const UPDATE_FRAME: usize = 9;
pub(crate) const SCHEDULE_WAKE: usize = 10;
pub(crate) const HOVER: usize = 11;
pub(crate) const PATH_AT: usize = 12;
pub(crate) const POINTER_MOVED: usize = 13;
pub(crate) const DRAW_FRAME: usize = 14;
pub(crate) const NOTE_ACTIVITY: usize = 15;
pub(crate) const NEED_GPU: usize = 16;
pub(crate) const INPUT_QUEUE_CHECK: usize = 17;
pub(crate) const PATH_LAY_OUT: usize = 18;
pub(crate) const RAW_POINTER_PATH: usize = 19;
pub(crate) const ROUNDED_CORNERS: usize = 20;
pub(crate) const POINTER_INERT: usize = 21;
pub(crate) const PARAGRAPH_PART: usize = 22;
const LABELS: [&str; 23] = [
    "session", "calibration_empty_scope", "loop_GetMessageW", "loop_translate_dispatch",
    "present_due_windows", "WM_MOUSEMOVE", "modifiers_read", "modifiers_feed",
    "app_move_feed", "update_frame", "schedule_wake", "update_hover", "path_at",
    "pointer_moved", "draw_frame", "note_activity", "need_gpu", "more_continuous_input",
    "path_lay_out", "raw_pointer_path", "rounded_corners", "pointer_inert", "paragraph_part_at",
];

#[link(name = "kernel32")]
unsafe extern "system" {
    fn QueryThreadCycleTime(thread: *mut std::ffi::c_void, cycles: *mut u64) -> i32;
    fn GetCurrentThreadId() -> u32;
}

fn cycles() -> Option<u64> {
    let mut count = 0;
    // SAFETY: -2 is the documented current-thread pseudo handle. The output is
    // writable for the duration of the synchronous call; no handle is owned.
    (unsafe { QueryThreadCycleTime((-2isize) as *mut _, &mut count) } != 0).then_some(count)
}

fn output() -> Option<&'static PathBuf> {
    static OUTPUT: OnceLock<Option<PathBuf>> = OnceLock::new();
    OUTPUT.get_or_init(|| std::env::var_os("RSUI_NATIVE_CYCLES").filter(|path| !path.is_empty()).map(PathBuf::from)).as_ref()
}

#[derive(Clone, Copy, Default)]
struct Counter {
    count: u64,
    inclusive: u64,
    exclusive: u64,
    min: u64,
    max: u64,
}

#[derive(Clone, Copy, Default)]
struct Frame {
    label: usize,
    began: u64,
    children: u64,
}

struct Counters {
    counters: [Counter; LABELS.len()],
    stack: [Frame; 64],
    depth: usize,
    failed_queries: u64,
    overflow: u64,
    messages: [(u32, u64); 64],
    message_overflow: u64,
}

impl Default for Counters {
    fn default() -> Self {
        Self { counters: [Counter::default(); LABELS.len()], stack: [Frame::default(); 64], depth: 0, failed_queries: 0, overflow: 0,
            messages: [(0, 0); 64], message_overflow: 0 }
    }
}

thread_local! { static COUNTERS: RefCell<Counters> = RefCell::new(Counters::default()); }

/// Counts exactly the messages returned to the loop, without changing the queue
/// or querying a clock. Unusual IDs share a single overflow count after 64 IDs.
pub(crate) fn note_message(message: u32) {
    if output().is_none() { return; }
    COUNTERS.with(|state| {
        let mut state = state.borrow_mut();
        if let Some(entry) = state.messages.iter_mut().find(|(id, count)| *count == 0 || *id == message) {
            entry.0 = message;
            entry.1 += 1;
        } else {
            state.message_overflow += 1;
        }
    });
}

pub(crate) struct Scope(bool);

pub(crate) fn enter(label: usize) -> Scope {
    if output().is_none() { return Scope(false); }
    let Some(began) = cycles() else {
        COUNTERS.with(|state| state.borrow_mut().failed_queries += 1);
        return Scope(false);
    };
    Scope(COUNTERS.with(|state| {
        let mut state = state.borrow_mut();
        let at = state.depth;
        if at == state.stack.len() {
            state.overflow += 1;
            return false;
        }
        state.stack[at] = Frame { label, began, children: 0 };
        state.depth += 1;
        true
    }))
}

impl Drop for Scope {
    fn drop(&mut self) {
        if !self.0 { return; }
        let ended = cycles();
        COUNTERS.with(|state| {
            let mut state = state.borrow_mut();
            state.depth -= 1;
            let at = state.depth;
            let frame = state.stack[at];
            let Some(ended) = ended else {
                state.failed_queries += 1;
                return;
            };
            let elapsed = ended.saturating_sub(frame.began);
            let counter = &mut state.counters[frame.label];
            counter.count += 1;
            counter.inclusive = counter.inclusive.saturating_add(elapsed);
            counter.exclusive = counter.exclusive.saturating_add(elapsed.saturating_sub(frame.children));
            counter.min = if counter.count == 1 { elapsed } else { counter.min.min(elapsed) };
            counter.max = counter.max.max(elapsed);
            if at > 0 {
                state.stack[at - 1].children = state.stack[at - 1].children.saturating_add(elapsed);
            }
        });
    }
}

pub(crate) struct Session(Option<Scope>);

impl Session {
    pub(crate) fn start() -> Self {
        if output().is_none() { return Self(None); }
        COUNTERS.with(|state| *state.borrow_mut() = Counters::default());
        // Repeated empty scopes expose the actual query/bookkeeping floor in
        // this executable. Do not subtract an assumed cost from measurements.
        for _ in 0..1_000 { drop(enter(CALIBRATION)); }
        Self(Some(enter(SESSION)))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.0.is_none() { return; }
        drop(self.0.take());
        let Some(path) = output() else { return; };
        let result = (|| -> std::io::Result<()> {
            if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
            // SAFETY: this only identifies the calling thread.
            let thread = unsafe { GetCurrentThreadId() };
            writeln!(file, "pid,thread_id,label,count,inclusive_cycles,exclusive_cycles,min_cycles,max_cycles,failed_queries,stack_overflow")?;
            COUNTERS.with(|state| -> std::io::Result<()> {
                let state = state.borrow();
                for (label, count) in LABELS.iter().zip(&state.counters) {
                    writeln!(file, "{},{thread},{label},{},{},{},{},{},{},{}", std::process::id(), count.count,
                        count.inclusive, count.exclusive, count.min, count.max, state.failed_queries, state.overflow)?;
                }
                Ok(())
            })?;
            file.flush()?;
            let mut messages = std::io::BufWriter::new(std::fs::File::create(path.with_extension("messages.csv"))?);
            writeln!(messages, "pid,thread_id,message_hex,count")?;
            COUNTERS.with(|state| -> std::io::Result<()> {
                let state = state.borrow();
                for &(id, count) in &state.messages {
                    if count > 0 { writeln!(messages, "{},{thread},0x{id:04x},{count}", std::process::id())?; }
                }
                writeln!(messages, "{},{thread},overflow,{}", std::process::id(), state.message_overflow)
            })?;
            messages.flush()?;
            // Existing nested split scopes distinguish JS, GC, layout, paint,
            // and individual host calls. These are elapsed durations, separate
            // from thread cycles; write once at close, never in an input handler.
            if rsui_alloc::split::on() {
                let mut parts = std::io::BufWriter::new(std::fs::File::create(path.with_extension("split.csv"))?);
                writeln!(parts, "part,elapsed_ns,count")?;
                for (name, nanos, count) in rsui_alloc::split::take() {
                    let name = name.replace('"', "\"\"");
                    writeln!(parts, "\"{name}\",{nanos},{count}")?;
                }
                parts.flush()?;
            }
            Ok(())
        })();
        if let Err(error) = result { eprintln!("native cycle report: {error}"); }
    }
}
