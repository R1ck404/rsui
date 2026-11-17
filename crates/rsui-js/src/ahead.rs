//! The files a packed program is going to run, parsed on other threads before they're wanted.
//!
//! A program's files are parsed one after another as it starts, each when something first imports it, and reading what
//! the code says takes about as long as everything else that's done with it. A packed program knows which files it
//! loaded, and in which order, from when it was packed, so other threads can be reading those while this one is running
//! the ones before them. Each is only parsed: what it means is still worked out on the thread that runs it, in the order
//! it's run, so a program does exactly what it would otherwise, just sooner.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, OnceLock};

use crate::ast::{AstMemory, Program};

/// How many files can be read ahead of the one that's being run, which bounds what they take while they wait.
const AHEAD: usize = 24;
/// The most threads that read ahead. More only helps when one file takes much longer to run than to read.
const MOST_THREADS: usize = 3;
/// How deep the parser can go on a thread that reads ahead, which is given a little more than this.
const STACK_ROOM: usize = 15 * 1024 * 1024;

/// A file that's been parsed, as its source and what the parser made of it.
pub(crate) struct Parsed {
    /// What's in `memory` and `source`, which this owns, so it's never seen for longer than this lasts. See
    /// [`Parsed::program`].
    program: Program<'static>,
    /// Only kept for what `program` has in it.
    #[allow(dead_code)]
    memory: AstMemory,
    /// Only kept for what `program` has in it.
    #[allow(dead_code)]
    source: String,
    /// Whether it's an ES module. If not, it's the source wrapped as CommonJS is, parsed as a script.
    pub(crate) is_module: bool,
}

// SAFETY: what's in it is its own, and it's only used by whichever thread has it, one at a time.
#[allow(unsafe_code)]
unsafe impl Send for Parsed {}

impl Parsed {
    /// What the parser made of the file, and what that's in, to be kept while any of its functions is waiting to be
    /// compiled. The program can only be used while that lasts.
    pub(crate) fn into_kept(self) -> (std::rc::Rc<crate::compiler::KeptAst>, Program<'static>) {
        // Moving the memory and the source doesn't move what's in them, which is what the program borrows.
        let kept = crate::compiler::KeptAst::of(self.memory, self.source);
        (kept, self.program)
    }

    /// Reads a file once, as [`crate::parser::parse_file`] does, or `None` if it wasn't clear what it is that way.
    #[allow(unsafe_code)]
    fn new_file(
        source: String,
        params: &[&'static str],
        allow_module: bool,
    ) -> Option<Option<Self>> {
        let memory = AstMemory::new();
        // SAFETY: as in `new`.
        let (text, place): (&'static str, &'static AstMemory) = unsafe {
            (
                &*std::ptr::from_ref::<str>(source.as_str()),
                &*std::ptr::from_ref(&memory),
            )
        };
        let (program, is_module) =
            match crate::parser::parse_file(text, place, STACK_ROOM, params, allow_module) {
                Ok(crate::parser::FileProgram::Wrapped(program)) => (program, false),
                Ok(crate::parser::FileProgram::Module(program)) => (program, true),
                Ok(crate::parser::FileProgram::Unsure) => return None,
                // Read again where it's run, which says why.
                Err(_) => return Some(None),
            };
        Some(Some(Self {
            program,
            memory,
            source,
            is_module,
        }))
    }

    #[allow(unsafe_code)]
    fn new(source: String, is_module: bool) -> Option<Self> {
        let memory = AstMemory::new();
        // SAFETY: the program is only ever seen through `program`, for no longer than this lasts, and what it borrows
        // from doesn't move when this does: the text of a `String` and the memory of an `AstMemory` stay where they are.
        let (text, place): (&'static str, &'static AstMemory) = unsafe {
            (
                &*std::ptr::from_ref::<str>(source.as_str()),
                &*std::ptr::from_ref(&memory),
            )
        };
        let program = if is_module {
            crate::parser::parse_module(text, place, STACK_ROOM).ok()?
        } else {
            crate::parser::parse(text, place, STACK_ROOM, true).ok()?
        };
        if is_module && !crate::modules::has_module_syntax(&program) {
            return None;
        }
        Some(Self {
            program,
            memory,
            source,
            is_module,
        })
    }
}

enum Slot {
    /// Not started.
    Waiting,
    /// A thread is reading it.
    Reading,
    /// Read, or `None` if that didn't work, in which case it's read again where it's run, which says why.
    Read(Option<Box<Parsed>>),
    /// Taken, or not wanted any more.
    Gone,
}

struct Queue {
    files: Vec<PathBuf>,
    slots: Vec<Slot>,
    /// Where each file is in `files`, by its path as [`crate::packed`] compares them.
    places: HashMap<String, usize>,
    /// The next file for a thread to read.
    next: usize,
    /// How many have been taken, or passed over.
    taken: usize,
}

struct Ahead {
    queue: Mutex<Queue>,
    changed: Condvar,
}

static AHEAD_OF_TIME: OnceLock<Ahead> = OnceLock::new();

/// Starts reading `files` on other threads, in this order, which is the order they'll be wanted in.
pub(crate) fn start(files: Vec<PathBuf>) {
    let files: Vec<PathBuf> = files.into_iter().filter(|path| is_code(path)).collect();
    if files.is_empty() {
        return;
    }
    let places = files
        .iter()
        .enumerate()
        .map(|(at, path)| (crate::packed::key(path), at))
        .collect();
    let slots = files.iter().map(|_| Slot::Waiting).collect();
    let ahead = Ahead {
        queue: Mutex::new(Queue {
            files,
            slots,
            places,
            next: 0,
            taken: 0,
        }),
        changed: Condvar::new(),
    };
    if AHEAD_OF_TIME.set(ahead).is_err() {
        return;
    }
    let threads = std::thread::available_parallelism()
        .map_or(1, |count| count.get().saturating_sub(2))
        .clamp(1, MOST_THREADS);
    for _ in 0..threads {
        let _ = std::thread::Builder::new()
            .name("rsui read ahead".to_owned())
            .stack_size(STACK_ROOM + 1024 * 1024)
            .spawn(read_ahead);
    }
}

/// Whether this is a file of code, rather than of data.
fn is_code(path: &Path) -> bool {
    builtin_name(path).is_some()
        || path.extension().is_some_and(|extension| {
            ["js", "mjs", "cjs", "ts", "mts", "cts", "tsx", "jsx"]
                .iter()
                .any(|code| extension.eq_ignore_ascii_case(code))
        })
}

fn read_ahead() {
    let Some(ahead) = AHEAD_OF_TIME.get() else {
        return;
    };
    loop {
        let (at, path) = {
            let Ok(mut queue) = ahead.queue.lock() else {
                return;
            };
            loop {
                if queue.next >= queue.files.len() {
                    return;
                }
                if queue.next - queue.taken.min(queue.next) < AHEAD {
                    break;
                }
                queue = match ahead.changed.wait(queue) {
                    Ok(queue) => queue,
                    Err(_) => return,
                };
            }
            let at = queue.next;
            queue.next += 1;
            if !matches!(queue.slots[at], Slot::Waiting) {
                continue;
            }
            queue.slots[at] = Slot::Reading;
            (at, queue.files[at].clone())
        };
        let parsed = parse(&path);
        let Ok(mut queue) = ahead.queue.lock() else {
            return;
        };
        queue.slots[at] = if matches!(queue.slots[at], Slot::Gone) {
            Slot::Gone
        } else {
            Slot::Read(parsed.map(Box::new))
        };
        drop(queue);
        ahead.changed.notify_all();
    }
}

/// The name of a module that comes with the engine, if that's what this is. See [`crate::modules::builtin_path`].
fn builtin_name(path: &Path) -> Option<&str> {
    path.to_str()?.strip_prefix(crate::modules::BUILTIN)
}

/// Reads a file as [`crate::vm::Vm::load_module`] would: as a module if it is one, and otherwise as CommonJS has it. A
/// module that comes with the engine is read as it's compiled, as a script.
fn parse(path: &Path) -> Option<Parsed> {
    if let Some(name) = builtin_name(path) {
        let source = crate::modules::builtin_source(name)?;
        return Parsed::new_file(source.to_owned(), &crate::modules::BUILTIN_PARAMS, false)
            .flatten();
    }
    let mut source = crate::packed::read_file(path).ok()?;
    if source.starts_with('\u{FEFF}') {
        source.drain(..'\u{FEFF}'.len_utf8());
    }
    if source.starts_with("#!") {
        source.insert_str(0, "//");
    }
    let allow_module = crate::modules::might_be_module(&source);
    if let Some(parsed) = Parsed::new_file(
        source.clone(),
        &crate::modules::COMMONJS_PARAMS,
        allow_module,
    ) {
        return parsed;
    }
    if allow_module {
        // What isn't a module after all is read again as CommonJS, as it is where it's run.
        if let Some(parsed) = Parsed::new(source.clone(), true) {
            return Some(parsed);
        }
    }
    Parsed::new(crate::modules::wrapped_as_commonjs(source), false)
}

/// What was read ahead of `path`, if it was, waiting for it if it's being read now.
pub(crate) fn take(path: &Path) -> Option<Parsed> {
    let ahead = AHEAD_OF_TIME.get()?;
    let _time = crate::startup::span(crate::startup::Phase::AheadWait, || None);
    let mut queue = ahead.queue.lock().ok()?;
    let at = *queue.places.get(&crate::packed::key(path))?;
    loop {
        match std::mem::replace(&mut queue.slots[at], Slot::Gone) {
            Slot::Reading => {
                queue.slots[at] = Slot::Reading;
                queue = ahead.changed.wait(queue).ok()?;
            }
            Slot::Read(parsed) => {
                queue.taken += 1;
                #[cfg(test)]
                if parsed.is_some() {
                    USED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                drop(queue);
                ahead.changed.notify_all();
                return parsed.map(|parsed| *parsed);
            }
            // It's wanted sooner than it was when the program was packed, so it's read where it's wanted.
            Slot::Waiting => {
                queue.taken += 1;
                drop(queue);
                ahead.changed.notify_all();
                return None;
            }
            Slot::Gone => return None,
        }
    }
}

/// How many files that were read ahead have been run.
#[cfg(test)]
pub(crate) static USED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Waits until all that's going to be read ahead has been.
#[cfg(test)]
pub(crate) fn wait_until_read() {
    let ahead = AHEAD_OF_TIME.get().expect("reading ahead was started");
    let mut queue = ahead.queue.lock().unwrap();
    while queue
        .slots
        .iter()
        .any(|slot| matches!(slot, Slot::Waiting | Slot::Reading))
    {
        queue = ahead.changed.wait(queue).unwrap();
    }
}

/// Stops reading ahead, and lets go of what was read and never wanted, as when the program has started, and anything
/// else it runs is something it didn't run when it was packed.
pub(crate) fn stop() {
    let Some(ahead) = AHEAD_OF_TIME.get() else {
        return;
    };
    let unwanted: Vec<Slot> = {
        let Ok(mut queue) = ahead.queue.lock() else {
            return;
        };
        queue.next = queue.files.len();
        queue
            .slots
            .iter_mut()
            // What's being read now is let go of by the thread reading it, when it sees it's gone.
            .map(|slot| std::mem::replace(slot, Slot::Gone))
            .collect()
    };
    ahead.changed.notify_all();
    drop(unwanted);
}
