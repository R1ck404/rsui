//! How long the steps of starting took, for seeing where the time goes. Set `RSUI_MARKS` to a file, and each [`mark`] adds a
//! line to it: milliseconds since the first mark, and what was reached. Unset, a mark costs only looking once to see that.

use std::fs::File;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();
static FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();

/// Says that the program has got this far, if `RSUI_MARKS` says where to write that. The first mark is the start.
pub fn mark(what: &str) {
    let start = *START.get_or_init(Instant::now);
    let file = FILE.get_or_init(|| {
        std::env::var_os("RSUI_MARKS")
            .and_then(|path| File::create(path).ok())
            .map(Mutex::new)
    });
    if let Some(file) = file {
        if let Ok(mut file) = file.lock() {
            let _ = writeln!(file, "{:.1} {what}", start.elapsed().as_secs_f64() * 1000.0);
        }
    }
}
