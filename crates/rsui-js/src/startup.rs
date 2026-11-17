//! Where a program's start goes, for measuring: set `RSUI_STARTUP_TIMES` to a file, and [`write`] puts in it how long was
//! spent reading files, parsing them, compiling them, running their top levels, collecting, and waiting for what was read
//! ahead, each as time of its own (what a module's top level spends loading another is the other's), in all and file by
//! file. Unset, a span costs looking at one flag.
//!
//! It's kept for measuring the code cache (codecache.rs),
//! which says here how many files it loaded and how many it couldn't.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// What a span of time was spent on.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Phase {
    /// Reading a file's text, from the disk or from inside the program.
    Read,
    /// Parsing it.
    Parse,
    /// Compiling what was parsed into the engine's code, or loading that from a cache.
    Compile,
    /// Running a module's top level.
    Run,
    /// Waiting for a file another thread is reading ahead.
    AheadWait,
    /// Finding which file an `import` or `require` names.
    Resolve,
    /// Linking ES modules' imports to what they import, and making their namespaces.
    Link,
    /// Reading the code cache's block from the program (codecache.rs).
    CacheRead,
    /// Making a cached file's names and constants again.
    CacheNames,
    /// Making a cached file's functions again.
    CacheCode,
}

const PHASES: [Phase; 10] = [
    Phase::Read,
    Phase::Parse,
    Phase::Compile,
    Phase::Run,
    Phase::AheadWait,
    Phase::Resolve,
    Phase::Link,
    Phase::CacheRead,
    Phase::CacheNames,
    Phase::CacheCode,
];

static CACHE_LOADED: AtomicU32 = AtomicU32::new(0);
static CACHE_REFUSED: AtomicU32 = AtomicU32::new(0);
static CACHE_RESOLVED: AtomicU32 = AtomicU32::new(0);

/// Counts a name the code cache said what it was found to be when packing.
pub(crate) fn count_resolved() {
    CACHE_RESOLVED.fetch_add(1, Ordering::Relaxed);
}

/// How many names the code cache has said what they were found to be, in every engine.
#[cfg(test)]
pub(crate) fn count_resolved_so_far() -> u32 {
    CACHE_RESOLVED.load(Ordering::Relaxed)
}

/// Counts a file the code cache loaded, or found in its block and couldn't.
pub(crate) fn count_cache(loaded: bool) {
    let counter = if loaded {
        &CACHE_LOADED
    } else {
        &CACHE_REFUSED
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

/// How many files the code cache has loaded, in every engine.
#[cfg(test)]
pub(crate) fn count_loaded() -> u32 {
    CACHE_LOADED.load(Ordering::Relaxed)
}

struct Open {
    phase: Phase,
    file: Option<String>,
    started: Instant,
    /// What spans inside this one took, which isn't its own.
    inside: Duration,
}

#[derive(Default)]
struct Kept {
    open: Vec<Open>,
    totals: HashMap<Phase, (Duration, u32)>,
    by_file: HashMap<(Phase, String), Duration>,
}

thread_local! {
    static KEPT: RefCell<Kept> = RefCell::new(Kept::default());
}

/// Whether `RSUI_STARTUP_TIMES` says to keep times.
pub(crate) fn on() -> bool {
    static ON: AtomicU8 = AtomicU8::new(0);
    match ON.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var_os("RSUI_STARTUP_TIMES").is_some();
            ON.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        known => known == 2,
    }
}

/// Time spent on `phase` until this is dropped, for `file` if it's said.
pub(crate) struct Span(bool);

/// Starts a span, if times are kept. `file` is only asked for then.
#[inline]
pub(crate) fn span(phase: Phase, file: impl FnOnce() -> Option<String>) -> Span {
    if !on() {
        return Span(false);
    }
    let file = file();
    KEPT.with(|kept| {
        kept.borrow_mut().open.push(Open {
            phase,
            file,
            started: Instant::now(),
            inside: Duration::ZERO,
        });
    });
    Span(true)
}

impl Drop for Span {
    fn drop(&mut self) {
        if !self.0 {
            return;
        }
        KEPT.with(|kept| {
            let mut kept = kept.borrow_mut();
            let Some(open) = kept.open.pop() else {
                return;
            };
            let took = open.started.elapsed();
            let own = took.saturating_sub(open.inside);
            if let Some(outer) = kept.open.last_mut() {
                outer.inside += took;
            }
            let total = kept.totals.entry(open.phase).or_default();
            total.0 += own;
            total.1 += 1;
            if let Some(file) = open.file {
                *kept.by_file.entry((open.phase, file)).or_default() += own;
            }
        });
    }
}

/// Writes what's been kept so far to the file `RSUI_STARTUP_TIMES` names, after a line saying `when`.
pub fn write(when: &str) {
    let Some(path) = std::env::var_os("RSUI_STARTUP_TIMES") else {
        return;
    };
    let text = KEPT.with(|kept| {
        let kept = kept.borrow();
        let mut out = format!("at {when}\n");
        for phase in PHASES {
            let (took, count) = kept.totals.get(&phase).copied().unwrap_or_default();
            out.push_str(&format!(
                "total {:?} {:.3} ms {count}\n",
                phase,
                took.as_secs_f64() * 1000.0
            ));
        }
        out.push_str(&format!(
            "cache {} loaded {} refused {} resolved\n",
            CACHE_LOADED.load(Ordering::Relaxed),
            CACHE_REFUSED.load(Ordering::Relaxed),
            CACHE_RESOLVED.load(Ordering::Relaxed)
        ));
        let mut rows: Vec<_> = kept.by_file.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1));
        for ((phase, file), took) in rows {
            out.push_str(&format!(
                "file {:?} {:.3} ms {file}\n",
                phase,
                took.as_secs_f64() * 1000.0
            ));
        }
        out
    });
    // The first time, the file's made again; after that each time's added to it.
    static WRITTEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let again = WRITTEN.swap(true, Ordering::Relaxed);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(again)
        .truncate(!again)
        .open(path);
    if let Ok(mut file) = file {
        use std::io::Write;
        let _ = file.write_all(text.as_bytes());
    }
}
