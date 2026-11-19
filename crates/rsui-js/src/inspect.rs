//! What a debugger asks the engine: where a function was written, which lines are running, and what a value looks like.
//!
//! None of this runs any of the program's code, and none of it is used unless something's looking, so it costs nothing
//! otherwise.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crate::Value;
use crate::object::Kind;
use crate::op::NOT_COMPILED;
use crate::vm::Vm;

/// What changes a file's source before it runs, as a debugger's logpoints are put in.
pub type SourceFilter = Rc<dyn Fn(&Path, &mut String)>;

thread_local! {
    static SOURCE_FILTER: RefCell<Option<SourceFilter>> = const { RefCell::new(None) };
}

/// Has every file that's read from now on, on this thread, go through `filter` before it runs, or none for nothing to.
/// Files are read again when they're run again, as after `Engine::forget_files`.
pub fn set_source_filter(filter: Option<SourceFilter>) {
    SOURCE_FILTER.with(|kept| *kept.borrow_mut() = filter);
}

pub(crate) fn has_source_filter() -> bool {
    SOURCE_FILTER.with(|kept| kept.borrow().is_some())
}

pub(crate) fn filter_source(path: &Path, source: &mut String) {
    let filter = SOURCE_FILTER.with(|kept| kept.borrow().clone());
    if let Some(filter) = filter {
        filter(path, source);
    }
}

/// Where some code is: the function it's in, the file and the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    /// Empty for a function with no name, as an arrow function given straight to something has.
    pub function: String,
    pub file: String,
    pub line: u32,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.file, self.line)
    }
}

impl Vm {
    /// Where a function was written, if it was written in JavaScript. A bound function is where what it's bound to was.
    pub fn function_origin(&self, value: Value) -> Option<Origin> {
        if !value.is_object() {
            return None;
        }
        match &self.objects[value.index()].kind {
            Kind::Function { proto, .. } => Some(self.proto_origin(*proto, None)),
            Kind::Bound(bound) => self.function_origin(bound.target),
            _ => None,
        }
    }

    /// The lines running now, innermost first, up to `most` of them, leaving out what's in files `skip` says to. Only
    /// code written in JavaScript has a line, so what's running in Rust isn't here.
    pub fn callers(&self, most: usize, skip: impl Fn(&str) -> bool) -> Vec<Origin> {
        let mut found = Vec::new();
        let mut last_file = None;
        let mut skipping = false;
        for frame in self.frames.iter().rev() {
            if found.len() >= most {
                break;
            }
            let file = self.protos[frame.proto as usize].file;
            // Frames from the same file come together, and the file only has to be read once for them.
            if last_file != Some(file) {
                last_file = Some(file);
                skipping = skip(&self.atom_text(file));
            }
            if !skipping {
                found.push(self.proto_origin(frame.proto, Some(frame.pc)));
            }
        }
        found
    }

    pub(crate) fn proto_origin(&self, proto_id: u32, pc: Option<u32>) -> Origin {
        let proto = &self.protos[proto_id as usize];
        let line = if proto.start == NOT_COMPILED {
            // It's never run, so all there is is where it starts.
            self.uncompiled
                .get(&proto_id)
                .map_or(0, crate::compiler::Uncompiled::line)
        } else {
            match pc {
                Some(pc) => {
                    let at = pc.saturating_sub(proto.start).saturating_sub(1);
                    match proto.lines.partition_point(|entry| entry.0 <= at) {
                        0 => proto.lines.first().map_or(0, |entry| entry.1),
                        after => proto.lines[after - 1].1,
                    }
                }
                // The first line it marks is the one it starts on.
                None => proto.lines.first().map_or(0, |entry| entry.1),
            }
        };
        Origin {
            function: self.atom_text(proto.name),
            file: self.atom_text(proto.file),
            line,
        }
    }

    /// A value as `console.log` shows it, on one line, cut short after about `longest` characters.
    pub fn describe(&mut self, value: Value, longest: usize) -> String {
        let shown = crate::builtins::inspect_value(self, value);
        let mut one_line = String::with_capacity(shown.len().min(longest + 1));
        let mut last_space = false;
        for c in shown.chars() {
            let c = if c.is_whitespace() { ' ' } else { c };
            if c == ' ' && last_space {
                continue;
            }
            last_space = c == ' ';
            one_line.push(c);
        }
        if one_line.chars().count() > longest {
            let cut: String = one_line.chars().take(longest.saturating_sub(1)).collect();
            return cut + "…";
        }
        one_line
    }
}

// Pausing, for a debugger.
//
// A file the debugger says it's debugging (`set_debug_files`) is read with a `Stmt::Line` in front of each statement,
// which is compiled to `Op::Step`. Nothing else has one, so other code is as fast as ever, and the JIT leaves a function
// with one to the interpreter. A step is a load of `DEBUG_WANTED` until some thread has a breakpoint, a pause asked for or
// a step to take, and then it asks `Vm::at_step` whether to stop, which goes by what its own thread has. Stopping calls
// the pause handler, on this thread, with the program stopped where it is, and carries on as the handler says once it
// returns.

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Whether a step has anything to ask about: some thread has a breakpoint, a pause asked for, or a step being taken.
pub(crate) static DEBUG_WANTED: AtomicBool = AtomicBool::new(false);
/// How many threads have breakpoints or a step being taken, and how many have a pause asked for.
static THREADS_WANTING: AtomicUsize = AtomicUsize::new(0);
static PAUSES_ASKED: AtomicUsize = AtomicUsize::new(0);

/// What has the program on a thread stop at its next statement in a file being debugged, from any thread, as one that's
/// waiting for a program that's busy does. See [`pause_handle`].
#[derive(Clone, Debug)]
pub struct PauseHandle(Arc<AtomicBool>);

impl PauseHandle {
    /// Has the program stop at its next statement in a file being debugged.
    pub fn pause(&self) {
        if !self.0.swap(true, Ordering::SeqCst) {
            PAUSES_ASKED.fetch_add(1, Ordering::SeqCst);
        }
        DEBUG_WANTED.store(true, Ordering::SeqCst);
    }

    /// Takes back a pause that's been asked for and hasn't happened. Says whether there was one.
    fn take(&self) -> bool {
        let was = self.0.swap(false, Ordering::SeqCst);
        if was {
            PAUSES_ASKED.fetch_sub(1, Ordering::SeqCst);
        }
        was
    }
}

/// Which files are being debugged, so that they're read with places to stop in them.
pub type DebugFiles = Rc<dyn Fn(&Path) -> bool>;

/// What's called when the program stops. It looks at the program with what's here on [`Vm`], and says how to carry on.
pub type PauseHandler = Rc<dyn Fn(&mut Vm, &Pause) -> Resume>;

/// Why the program stopped, and where.
#[derive(Debug, Clone)]
pub struct Pause {
    pub reason: PauseReason,
    pub at: Origin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    /// It got to a line with a breakpoint.
    Breakpoint,
    /// It got to where a step said to stop.
    Step,
    /// A pause was asked for, and this is the first statement after that.
    Asked,
}

/// How to carry on after a pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    /// Until the next breakpoint.
    Continue,
    /// To the next statement in this function, or in what called it once it returns.
    Over,
    /// To the next statement anywhere, which is inside a function this one calls if it calls one.
    Into,
    /// To the next statement after this function returns.
    Out,
}

#[derive(Clone, Copy)]
enum Stepping {
    Into,
    Over(usize),
    Out(usize),
}

#[derive(Default)]
struct Debugger {
    handler: Option<PauseHandler>,
    /// Each file, written as [`same_file`] has it, and line.
    breakpoints: Vec<(String, u32)>,
    stepping: Option<Stepping>,
    /// How many frames there were, the function, the line and where in the function, at the last step, so that a line
    /// with more than one statement on it is only stopped at once each time it runs.
    last: (usize, u32, u32, u32),
    paused: bool,
    /// Whether this thread is counted in [`THREADS_WANTING`].
    wanting: bool,
}

impl Drop for Debugger {
    fn drop(&mut self) {
        if self.wanting {
            THREADS_WANTING.fetch_sub(1, Ordering::SeqCst);
        }
        refresh_wanted();
    }
}

thread_local! {
    static DEBUG_FILES: RefCell<Option<DebugFiles>> = const { RefCell::new(None) };
    static PARSING_FOR_DEBUGGER: Cell<bool> = const { Cell::new(false) };
    static DEBUGGER: RefCell<Debugger> = RefCell::new(Debugger::default());
    static PAUSE_HANDLE: PauseHandle = PauseHandle(Arc::new(AtomicBool::new(false)));
}

/// What has the program on this thread stop, from any thread.
pub fn pause_handle() -> PauseHandle {
    PAUSE_HANDLE.with(Clone::clone)
}

/// How a file's written for comparing it with another: with `/` between folders, and in lower case.
pub fn same_file(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

/// Has the files that `which` says read from now on, on this thread, with places for a debugger to stop, or none for no
/// file to. Files are read again when they're run again, as after `Engine::forget_files`.
pub fn set_debug_files(which: Option<DebugFiles>) {
    DEBUG_FILES.with(|kept| *kept.borrow_mut() = which);
}

/// Whether a file's to be read with places to stop in it.
pub(crate) fn debugs(path: &Path) -> bool {
    let which = DEBUG_FILES.with(|kept| kept.borrow().clone());
    which.is_some_and(|which| which(path))
}

/// Whether what's being read now is for the debugger, which the parser asks when it starts.
pub(crate) fn parsing_for_debugger() -> bool {
    PARSING_FOR_DEBUGGER.with(Cell::get)
}

/// While this lasts, what's read is read for the debugger, or not, as it says.
pub(crate) struct ParsingForDebugger(bool);

impl ParsingForDebugger {
    pub(crate) fn start(on: bool) -> Self {
        Self(PARSING_FOR_DEBUGGER.with(|parsing| parsing.replace(on)))
    }
}

impl Drop for ParsingForDebugger {
    fn drop(&mut self) {
        PARSING_FOR_DEBUGGER.with(|parsing| parsing.set(self.0));
    }
}

/// Has the program stop on this thread at the lines given, each as a file and a line. Only files being debugged have
/// anywhere to stop.
pub fn set_breakpoints(at: &[(String, u32)]) {
    DEBUGGER.with(|debugger| {
        debugger.borrow_mut().breakpoints = at
            .iter()
            .map(|(file, line)| (same_file(file), *line))
            .collect();
    });
    update_wanted();
}

/// Has `handler` called when the program stops on this thread, or nothing to stop it.
pub fn set_pause_handler(handler: Option<PauseHandler>) {
    DEBUGGER.with(|debugger| {
        let mut debugger = debugger.borrow_mut();
        if handler.is_none() {
            debugger.stepping = None;
        }
        debugger.handler = handler;
    });
    update_wanted();
}

/// Has the program on this thread stop at its next statement in a file that's being debugged. [`pause_handle`] asks it
/// from another thread.
pub fn pause_soon() {
    pause_handle().pause();
}

/// Whether the program is stopped on this thread now, which it is while the pause handler is running.
pub fn paused() -> bool {
    DEBUGGER.with(|debugger| debugger.try_borrow().is_ok_and(|debugger| debugger.paused))
}

/// Counts this thread in or out of those wanting steps looked at, as what it has says.
fn update_wanted() {
    DEBUGGER.with(|debugger| {
        let mut debugger = debugger.borrow_mut();
        let wanting = debugger.handler.is_some()
            && (!debugger.breakpoints.is_empty() || debugger.stepping.is_some());
        if wanting != debugger.wanting {
            debugger.wanting = wanting;
            if wanting {
                THREADS_WANTING.fetch_add(1, Ordering::SeqCst);
            } else {
                THREADS_WANTING.fetch_sub(1, Ordering::SeqCst);
            }
        }
    });
    refresh_wanted();
}

fn refresh_wanted() {
    let wanted = || {
        THREADS_WANTING.load(Ordering::SeqCst) > 0 || PAUSES_ASKED.load(Ordering::SeqCst) > 0
    };
    DEBUG_WANTED.store(wanted(), Ordering::SeqCst);
    // A pause asked for on another thread between looking and saying isn't lost.
    if wanted() {
        DEBUG_WANTED.store(true, Ordering::SeqCst);
    }
}

/// A function that's running, as a debugger sees it: where it is, and what its variables have in them now.
#[derive(Debug, Clone)]
pub struct FrameView {
    pub at: Origin,
    /// Its own variables that are there at this point in it, innermost first, then what it sees from the functions
    /// around it, then `this` if it has one. A variable that hasn't been given its value yet isn't here.
    pub locals: Vec<(String, Value)>,
    /// Whether it's in a file being debugged. Only those know their own variables.
    pub debugged: bool,
}

enum Decision {
    Go,
    Stop(PauseReason),
    /// It's on a line that has a breakpoint in some file, so whether it's this file is to be looked at.
    MaybeBreakpoint,
}

impl Vm {
    /// At a statement in code being debugged, while something's wanted: stops if it's to, and says how to go on.
    #[inline(never)]
    #[cold]
    pub(crate) fn at_step(&mut self, line: u32) -> crate::vm::Run<()> {
        let depth = self.frames.len();
        let Some(frame) = self.frames.last() else {
            return Ok(());
        };
        let (proto, pc) = (frame.proto, frame.pc);
        let decision = DEBUGGER.with(|debugger| {
            let Ok(mut debugger) = debugger.try_borrow_mut() else {
                return Decision::Go;
            };
            if debugger.paused || debugger.handler.is_none() {
                return Decision::Go;
            }
            let here = (depth, proto, line, pc);
            let last = debugger.last;
            let same_line = (last.0, last.1, last.2) == (depth, proto, line) && pc > last.3;
            debugger.last = here;
            if PAUSE_HANDLE.with(PauseHandle::take) {
                return Decision::Stop(PauseReason::Asked);
            }
            let stepped = match debugger.stepping {
                Some(Stepping::Into) => true,
                Some(Stepping::Over(at)) => depth <= at && !same_line,
                Some(Stepping::Out(at)) => depth < at && !same_line,
                None => false,
            };
            if stepped {
                return Decision::Stop(PauseReason::Step);
            }
            if !same_line && debugger.breakpoints.iter().any(|(_, at)| *at == line) {
                return Decision::MaybeBreakpoint;
            }
            Decision::Go
        });
        let reason = match decision {
            Decision::Go => return Ok(()),
            Decision::Stop(reason) => reason,
            Decision::MaybeBreakpoint => {
                let file = same_file(&self.atom_text(self.protos[proto as usize].file));
                let here = DEBUGGER.with(|debugger| {
                    debugger
                        .borrow()
                        .breakpoints
                        .iter()
                        .any(|(at_file, at)| *at == line && *at_file == file)
                });
                if !here {
                    return Ok(());
                }
                PauseReason::Breakpoint
            }
        };
        let at = self.proto_origin(proto, Some(pc));
        let handler = DEBUGGER.with(|debugger| {
            let mut debugger = debugger.borrow_mut();
            debugger.paused = true;
            debugger.stepping = None;
            debugger.handler.clone()
        });
        let Some(handler) = handler else {
            return Ok(());
        };
        let resume = handler(self, &Pause { reason, at });
        DEBUGGER.with(|debugger| {
            let mut debugger = debugger.borrow_mut();
            debugger.paused = false;
            debugger.stepping = match resume {
                Resume::Continue => None,
                Resume::Over => Some(Stepping::Over(depth)),
                Resume::Into => Some(Stepping::Into),
                Resume::Out => Some(Stepping::Out(depth)),
            };
            // Going on from here doesn't stop again on the line it's on.
            debugger.last = (depth, proto, line, pc);
        });
        update_wanted();
        Ok(())
    }

    /// The functions running now, innermost first, up to `most` of them, with their variables.
    pub fn paused_frames(&self, most: usize) -> Vec<FrameView> {
        let mut found = Vec::new();
        for frame in self.frames.iter().rev().take(most) {
            let proto = &self.protos[frame.proto as usize];
            if proto.start == NOT_COMPILED {
                continue;
            }
            let at = frame.pc.saturating_sub(proto.start).saturating_sub(1);
            let mut locals: Vec<(String, Value)> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let vars = self.debug_vars.get(&frame.proto);
            if let Some(vars) = vars {
                // The innermost block's are the ones a name means, and they start last.
                let mut here: Vec<_> = vars
                    .iter()
                    .filter(|var| var.start <= at && at < var.end)
                    .collect();
                here.sort_by_key(|var| std::cmp::Reverse(var.start));
                for var in here {
                    let name = self.atom_text(var.name);
                    if !seen.insert(name.clone()) {
                        continue;
                    }
                    let Some(&held) = self.stack.get(frame.base as usize + var.reg as usize) else {
                        continue;
                    };
                    let value = if var.cell {
                        if !held.is_cell() {
                            continue;
                        }
                        self.cells[held.index()]
                    } else {
                        held
                    };
                    if !value.is_empty() {
                        locals.push((name, value));
                    }
                }
            }
            if !frame.upvalues.is_null() {
                for (index, name) in proto.capture_names.iter().enumerate() {
                    let name = self.atom_text(*name);
                    if !seen.insert(name.clone()) {
                        continue;
                    }
                    // SAFETY: a frame's captures are its closure's, which has one for each of its function's names.
                    #[allow(unsafe_code)]
                    let cell = unsafe { *frame.upvalues.add(index) };
                    // A parameter that's never given another value is captured as its value.
                    let value = if proto.captures.get(index).is_some_and(|capture| capture.by_value) {
                        cell
                    } else if cell.is_cell() {
                        self.cells[cell.index()]
                    } else {
                        continue;
                    };
                    if !value.is_empty() {
                        locals.push((name, value));
                    }
                }
            }
            if proto.flags & crate::op::IS_ARROW == 0
                && let Some(&this) = self.stack.get(frame.base as usize)
                && this.is_object()
            {
                locals.push(("this".to_owned(), this));
            }
            found.push(FrameView {
                at: self.proto_origin(frame.proto, Some(frame.pc)),
                locals,
                debugged: vars.is_some(),
            });
        }
        found
    }

    /// Works out an expression with the variables a running function sees, the `frame`th from the innermost, as
    /// [`Vm::paused_frames`] gives them. What it gives a variable isn't kept in the variable.
    pub fn evaluate_in_frame(&mut self, frame: usize, expression: &str) -> crate::vm::Run<Value> {
        let view = self.paused_frames(frame + 1).into_iter().nth(frame);
        let locals = view.map(|view| view.locals).unwrap_or_default();
        let mut names = Vec::new();
        let mut values = Vec::new();
        let mut this = Value::UNDEFINED;
        for (name, value) in locals {
            if name == "this" {
                this = value;
            } else if is_plain_name(&name) {
                names.push(name);
                values.push(value);
            }
        }
        let source = format!(
            "(function ({}) {{ return (\n{expression}\n); }})",
            names.join(", ")
        );
        let function = self.run_eval_source(&source)?;
        self.call(function, this, &values)
    }
}

/// Whether a variable's name can be a parameter's.
fn is_plain_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        && !matches!(name, "arguments" | "eval" | "this")
}

// The clock and the random numbers, as a replay sets them, so the program sees what it saw the first time.

thread_local! {
    /// What the clock says if it's been stopped, and how far it's from the system's otherwise.
    static CLOCK: Cell<(Option<f64>, f64)> = const { Cell::new((None, 0.0)) };
    static RANDOM_SEED: Cell<Option<u64>> = const { Cell::new(None) };
}

/// The time by the program's clock on this thread, in milliseconds since 1970: what `Date.now()` and timers go by.
pub fn now_ms() -> f64 {
    Vm::now_ms()
}

pub(crate) fn clock(real: impl FnOnce() -> f64) -> f64 {
    match CLOCK.with(Cell::get) {
        (Some(stopped), _) => stopped,
        (None, offset) => real() + offset,
    }
}

/// Has the program's clock on this thread say `at` until it's told something else, or with none, go again from where
/// it was, at the system's speed.
pub fn set_time(at: Option<f64>) {
    let (stopped, offset) = CLOCK.with(Cell::get);
    let next = match (at, stopped) {
        (Some(at), _) => (Some(at), offset),
        (None, Some(was)) => {
            let real = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |time| time.as_secs_f64() * 1000.0);
            (None, was - real)
        }
        (None, None) => (None, offset),
    };
    CLOCK.with(|clock| clock.set(next));
}

/// What `Math.random` starts from in each program on this thread that hasn't used it yet, or none for the time.
pub fn set_random_seed(seed: Option<u64>) {
    RANDOM_SEED.with(|kept| kept.set(seed));
}

pub(crate) fn random_seed() -> Option<u64> {
    RANDOM_SEED.with(Cell::get)
}
