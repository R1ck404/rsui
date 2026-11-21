//! Notices when an app's files change, using no CPU until they do.

#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[cfg(windows)]
const FILE_NOTIFY_CHANGE_LAST_WRITE: u32 = 0x10;
#[cfg(windows)]
const FILE_NOTIFY_CHANGE_FILE_NAME: u32 = 0x1;
#[cfg(windows)]
const INFINITE: u32 = u32::MAX;

#[cfg(windows)]
#[allow(unsafe_code)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn FindFirstChangeNotificationW(path: *const u16, subtree: i32, filter: u32) -> *mut c_void;
    fn FindNextChangeNotification(handle: *mut c_void) -> i32;
    fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
}

/// Saving a file is often a few writes in a row, which this waits to be over.
const SETTLE: Duration = Duration::from_millis(40);

/// Starts watching the folder of an app's main file. Anything below it can change.
pub(crate) fn start(path: &Path) {
    let folder = std::fs::canonicalize(path)
        .ok()
        .and_then(|path| path.parent().map(Path::to_owned))
        .unwrap_or_else(|| PathBuf::from("."));
    let main_file = path.to_owned();
    let notify = crate::bridge::reload_sender();
    let spawned = std::thread::Builder::new()
        .name("rsui-tsx watcher".to_owned())
        .stack_size(64 * 1024)
        .spawn(move || watch_folder(&folder, &main_file, &notify));
    if let Err(error) = spawned {
        eprintln!("not watching for changes: {error}");
    }
}

/// Elsewhere this looks at the folder's newest source file a few times a second, which is only done while developing
/// with `--watch`. inotify and FSEvents, as `rsui-js`'s `fs.watch` uses, would let it wait instead.
#[cfg(not(windows))]
fn watch_folder(folder: &Path, main_file: &Path, notify: &crate::bridge::ReloadSender) {
    let mut last = newest_change(folder);
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let now = newest_change(folder);
        if now > last {
            std::thread::sleep(SETTLE);
            last = newest_change(folder).max(now);
            notify.send(main_file.to_owned());
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn watch_folder(folder: &Path, main_file: &Path, notify: &crate::bridge::ReloadSender) {
    let wide: Vec<u16> = folder.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: the path ends in a zero and lasts through the call, and the result is checked.
    let handle = unsafe {
        FindFirstChangeNotificationW(
            wide.as_ptr(),
            1,
            FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME,
        )
    };
    if handle.is_null() || handle as isize == -1 {
        eprintln!("not watching for changes in {}", folder.display());
        return;
    }
    let mut last = newest_change(folder);
    loop {
        // SAFETY: the handle stays open for as long as this thread goes on.
        unsafe { WaitForSingleObject(handle, INFINITE) };
        std::thread::sleep(SETTLE);
        // SAFETY: as above.
        if unsafe { FindNextChangeNotification(handle) } == 0 {
            return;
        }
        // Lots of things change in a folder, and most of them aren't source code.
        let now = newest_change(folder);
        if now > last {
            last = now;
            notify.send(main_file.to_owned());
        }
    }
}

/// When the most recently changed source file below a folder changed, leaving out what's installed or built.
fn newest_change(folder: &Path) -> Option<SystemTime> {
    let mut newest = None;
    let mut to_visit = vec![folder.to_owned()];
    while let Some(at) = to_visit.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                if !name.starts_with('.') && name != "node_modules" && name != "target" {
                    to_visit.push(path);
                }
            } else if is_source(&name) {
                newest = newest.max(meta.modified().ok());
            }
        }
    }
    newest
}

fn is_source(name: &str) -> bool {
    [".tsx", ".ts", ".jsx", ".js", ".mjs", ".cjs", ".json"]
        .iter()
        .any(|extension| name.ends_with(extension))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_source_files_count() {
        assert!(is_source("app.tsx"));
        assert!(is_source("data.json"));
        assert!(!is_source("notes.txt"));
        assert!(!is_source("app.tsx.tmp"));
    }
}
