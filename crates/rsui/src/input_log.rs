//! When a window's input was handled, for measuring how long a key or a click takes in a real window. Set
//! `RSUI_INPUT_LOG` to a file, and each key, character and click a window handles adds a line to it: what it was, then when
//! handling it started and ended, in `QueryPerformanceCounter` ticks, which every process on the machine shares, so a program
//! that sent the input can tell how long it waited as well. What a window does once it's quiet, and letting go of the GPU,
//! are written down the same way. Unset, this costs only looking once to see that.

use std::fs::File;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
}

static FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();

fn file() -> Option<&'static Mutex<File>> {
    FILE.get_or_init(|| {
        std::env::var_os("RSUI_INPUT_LOG")
            .and_then(|path| File::create(path).ok())
            .map(Mutex::new)
    })
    .as_ref()
}

/// Whether anything's being written down.
pub(crate) fn on() -> bool {
    file().is_some()
}

/// An optional nested CPU/API elapsed-time interval. No clock is read when logging is disabled.
/// These intervals can overlap (for example, prepare includes update); do not add their totals.
pub(crate) struct Span {
    what: &'static str,
    start: Option<i64>,
}

impl Span {
    pub(crate) fn new(what: &'static str) -> Self {
        Self { what, start: on().then(now) }
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            note(self.what, start, format_args!(""));
        }
    }
}

/// The time now, in performance counter ticks.
pub(crate) fn now() -> i64 {
    let mut ticks = 0;
    // SAFETY: `ticks` is a valid place for the count, and this can't fail on Windows XP and later.
    unsafe {
        QueryPerformanceCounter(&mut ticks);
    }
    ticks
}

/// Writes down that `what` was handled from `start` until now, with `extra` after it.
pub(crate) fn note(what: &str, start: i64, extra: std::fmt::Arguments) {
    let end = now();
    if let Some(file) = file()
        && let Ok(mut file) = file.lock()
    {
        let _ = writeln!(file, "{what} {start} {end} {extra}");
    }
}
