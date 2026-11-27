//! Watching folders and files for changes, for `fs.watch`, `fs.watchFile` and `fs.promises.watch` in
//! `node/misc/fs_watch.js`. One thread looks after every watcher in the program: it waits on a completion port, which
//! Windows wakes when a folder it's reading changes, or when the next file to look at for `fs.watchFile` is due. While
//! nothing changes it uses no CPU, and once the last watcher's closed it goes away.
//!
//! What a watcher has seen waits here until its JavaScript asks for it, as a list of text: `r` and then a name for
//! something that came or went, `c` and a name for something that changed, `c` alone when so much changed that Windows
//! couldn't say what, `e` and a code for an error, and `s` and two stats for a file `fs.watchFile` saw change.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::background::{Later, Outcome};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

/// A handle, written as `net.rs` writes them, as the two declare the same functions.
type Handle = isize;

#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: Handle,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const c_void,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn CreateIoCompletionPort(file: Handle, port: Handle, key: usize, threads: u32) -> Handle;
    fn GetQueuedCompletionStatus(
        port: Handle,
        bytes: *mut u32,
        key: *mut usize,
        overlapped: *mut *mut Overlapped,
        milliseconds: u32,
    ) -> i32;
    fn PostQueuedCompletionStatus(port: Handle, bytes: u32, key: usize, overlapped: *mut Overlapped) -> i32;
    fn ReadDirectoryChangesW(
        folder: Handle,
        buffer: *mut c_void,
        length: u32,
        subtree: i32,
        filter: u32,
        returned: *mut u32,
        overlapped: *mut Overlapped,
        routine: *mut c_void,
    ) -> i32;
    fn CancelIoEx(file: Handle, overlapped: *mut Overlapped) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetLastError() -> u32;
}

const FILE_LIST_DIRECTORY: u32 = 1;
const SHARE_ALL: u32 = 7;
const OPEN_EXISTING: u32 = 3;
const BACKUP_SEMANTICS_AND_OVERLAPPED: u32 = 0x0200_0000 | 0x4000_0000;
/// Names, folders' names, attributes, size, writes, reads, creation and security: all that libuv, and so Node, asks for.
const EVERY_CHANGE: u32 = 0x17f;
const INFINITE: u32 = u32::MAX;
const WAIT_TIMEOUT: u32 = 258;
/// What a completion's key is when it's only to wake the thread, as no watcher's number is ever this.
const WAKE: usize = 0;
/// How many 4-byte words each folder has for Windows to say what changed in. A busy folder that fills this is told of
/// with a change with no name.
const ROOM: usize = 4096;
/// The most changes held for a watcher nothing's asking, so one that's forgotten can't grow for good.
const MOST_HELD: usize = 4096;

/// Things by their number. A program has a few watchers, so a list is quicker than a map, and costs less code.
struct Numbered<T>(Vec<(u32, T)>);

impl<T> Default for Numbered<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T> Numbered<T> {
    fn get_mut(&mut self, id: &u32) -> Option<&mut T> {
        self.0.iter_mut().find(|(found, _)| found == id).map(|(_, item)| item)
    }

    fn remove(&mut self, id: &u32) -> Option<T> {
        let at = self.0.iter().position(|(found, _)| found == id)?;
        Some(self.0.swap_remove(at).1)
    }

    fn insert(&mut self, id: u32, item: T) {
        self.remove(&id);
        self.0.push((id, item));
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A watcher, as the engine's thread and the watching thread both see it.
struct Watcher {
    seen: Vec<String>,
    /// The promise for what's seen next, while JavaScript is waiting for it.
    later: Option<Later>,
    /// Whether it keeps the program going: `persistent` and `ref()`.
    held: bool,
}

enum Command {
    Folder {
        id: u32,
        handle: usize,
        /// What it was asked to watch, which is what's said to have gone if the folder itself does.
        path: String,
        subtree: bool,
        /// The name of the one file in the folder to say anything about, in lower case.
        only: Option<Vec<u16>>,
    },
    Poll {
        id: u32,
        path: String,
        every: Duration,
    },
    Stop(u32),
}

#[derive(Default)]
struct Shared {
    watchers: Numbered<Watcher>,
    commands: Vec<Command>,
    /// The completion port the watching thread waits on, while there is that thread.
    port: usize,
    next: u32,
}

fn shared() -> &'static Mutex<Shared> {
    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
    SHARED.get_or_init(Mutex::default)
}

/// Where Windows writes what changed in a folder. The `Overlapped` comes first so that the two are one allocation that
/// doesn't move while Windows has it.
#[repr(C)]
struct Reading {
    overlapped: Overlapped,
    buffer: [u32; ROOM],
}

/// A folder the watching thread has open.
struct Folder {
    handle: Handle,
    path: String,
    reading: Box<Reading>,
    subtree: bool,
    only: Option<Vec<u16>>,
    /// Closed, with its last read yet to come back, after which `reading` is Windows' no more.
    closing: bool,
}

/// What `fs.watchFile` knows of a file the last time it looked.
#[derive(Clone, Copy, PartialEq)]
struct Look {
    /// 0 when there's nothing there, 1 for a file and 2 for a folder.
    kind: u8,
    size: f64,
    accessed: f64,
    modified: f64,
    born: f64,
}

impl Look {
    fn of(path: &str) -> Self {
        let ms = |time: std::io::Result<std::time::SystemTime>| {
            time.ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0.0, |since| since.as_secs_f64() * 1000.0)
        };
        match std::fs::metadata(path) {
            Ok(found) => Self {
                kind: if found.is_dir() { 2 } else { 1 },
                size: found.len() as f64,
                accessed: ms(found.accessed()),
                modified: ms(found.modified()),
                born: ms(found.created()),
            },
            Err(_) => Self {
                kind: 0,
                size: 0.0,
                accessed: 0.0,
                modified: 0.0,
                born: 0.0,
            },
        }
    }

    /// Whether it's changed in a way that's told of. Being read isn't one, as libuv doesn't count it either.
    fn differs(&self, other: &Self) -> bool {
        self.kind != other.kind || self.size != other.size || self.modified != other.modified || self.born != other.born
    }

    fn write(&self, out: &mut String) {
        use std::fmt::Write as _;
        let _ = write!(out, "{},{},{},{},{}", self.kind, self.size, self.accessed, self.modified, self.born);
    }
}

struct Poll {
    path: String,
    every: Duration,
    due: Instant,
    last: Look,
}

/// Tells a watcher what's been seen, and its JavaScript if it's waiting. It says whether the watcher's engine is gone.
fn tell(id: u32, seen: Vec<String>) -> bool {
    let Ok(mut shared) = shared().lock() else {
        return false;
    };
    let Some(watcher) = shared.watchers.get_mut(&id) else {
        return false;
    };
    watcher.seen.extend(seen);
    if watcher.seen.len() > MOST_HELD {
        let extra = watcher.seen.len() - MOST_HELD;
        watcher.seen.drain(..extra);
    }
    let Some(later) = watcher.later.take() else {
        return false;
    };
    let seen = std::mem::take(&mut watcher.seen);
    if later.settle_heard(Outcome::Names(seen)) {
        return false;
    }
    shared.watchers.remove(&id);
    true
}

/// Asks Windows for the next changes in a folder, to come to the port.
fn read_folder(folder: &mut Folder) -> bool {
    let reading = &mut *folder.reading;
    reading.overlapped = Overlapped {
        internal: 0,
        internal_high: 0,
        offset: 0,
        offset_high: 0,
        event: 0,
    };
    // SAFETY: the folder's handle is open, and the buffer and `Overlapped` are boxed, and stay where they are until the
    // port says this read's done, which `closing` waits for.
    unsafe {
        ReadDirectoryChangesW(
            folder.handle,
            reading.buffer.as_mut_ptr().cast(),
            (ROOM * 4) as u32,
            i32::from(folder.subtree),
            EVERY_CHANGE,
            std::ptr::null_mut(),
            &raw mut reading.overlapped,
            std::ptr::null_mut(),
        ) != 0
    }
}

/// What Windows wrote about a folder, as what's told of it.
fn changes(folder: &Folder, length: usize) -> Vec<String> {
    let words = &folder.reading.buffer;
    let length = length.min(ROOM * 4);
    let mut seen = Vec::new();
    let mut at = 0;
    // Each is a FILE_NOTIFY_INFORMATION: where the next is, what happened, and the name's length in bytes, then the name.
    while at + 12 <= length {
        let word = at / 4;
        let (next, action, size) = (words[word] as usize, words[word + 1], words[word + 2] as usize);
        let units: Vec<u16> = (0..size.min(length - at - 12) / 2)
            .map(|unit| {
                let byte = at + 12 + unit * 2;
                let shift = (byte % 4) * 8;
                (words[byte / 4] >> shift) as u16
            })
            .collect();
        let wanted = folder.only.as_ref().is_none_or(|only| {
            *only == String::from_utf16_lossy(&units).to_lowercase().encode_utf16().collect::<Vec<_>>()
        });
        if wanted {
            let kind = if action == 3 { 'c' } else { 'r' };
            seen.push(format!("{kind}{}", String::from_utf16_lossy(&units)));
        }
        if next == 0 {
            break;
        }
        at += next;
    }
    seen
}

/// What a Windows error is as the code Node gives it.
fn code_of(error: u32) -> &'static str {
    match error {
        2 | 3 => "ENOENT",
        5 => "EPERM",
        _ => "EIO",
    }
}

/// The watching thread, for as long as there's something to watch.
fn watch(port: usize) {
    let port = port as Handle;
    let mut folders: Numbered<Folder> = Numbered::default();
    let mut polls: Numbered<Poll> = Numbered::default();
    let mut gone = Vec::new();
    loop {
        let now = Instant::now();
        let wait = polls
            .0
            .iter()
            .map(|(_, poll)| poll.due.saturating_duration_since(now).as_millis().min(u128::from(INFINITE - 1)) as u32)
            .min()
            .unwrap_or(INFINITE);
        let (mut bytes, mut key, mut overlapped) = (0u32, 0usize, std::ptr::null_mut());
        // SAFETY: the port is open for as long as this thread is, and the three are written to and nothing else.
        let ok = unsafe { GetQueuedCompletionStatus(port, &raw mut bytes, &raw mut key, &raw mut overlapped, wait) };
        // SAFETY: it's read right after the call that set it.
        let error = if ok == 0 { unsafe { GetLastError() } } else { 0 };
        if overlapped.is_null() && ok != 0 && key == WAKE {
            let commands = shared().lock().map(|mut shared| std::mem::take(&mut shared.commands)).unwrap_or_default();
            for command in commands {
                match command {
                    Command::Folder { id, handle, path, subtree, only } => {
                        let handle = handle as Handle;
                        let mut folder = Folder {
                            handle,
                            path,
                            reading: Box::new(Reading {
                                overlapped: Overlapped {
                                    internal: 0,
                                    internal_high: 0,
                                    offset: 0,
                                    offset_high: 0,
                                    event: 0,
                                },
                                buffer: [0; ROOM],
                            }),
                            subtree,
                            only,
                            closing: false,
                        };
                        // SAFETY: both handles are open, and the folder's is this thread's to look after from now on.
                        let joined = unsafe { CreateIoCompletionPort(handle, port, id as usize, 0) } != 0;
                        if joined && read_folder(&mut folder) {
                            folders.insert(id, folder);
                        } else {
                            // SAFETY: it's read right after the call that failed.
                            let code = code_of(unsafe { GetLastError() });
                            // SAFETY: as above, and nothing was left reading it.
                            unsafe { CloseHandle(handle) };
                            gone.push((id, format!("e{code}")));
                        }
                    }
                    Command::Poll { id, path, every } => {
                        let last = Look::of(&path);
                        if last.kind == 0 {
                            // Node tells of a file that isn't there once to begin with, with both stats empty.
                            let mut told = String::from("s");
                            last.write(&mut told);
                            told.push(',');
                            last.write(&mut told);
                            gone.push((id, told));
                        }
                        polls.insert(id, Poll { path, every, due: Instant::now() + every, last });
                    }
                    Command::Stop(id) => {
                        polls.remove(&id);
                        if let Some(folder) = folders.get_mut(&id) {
                            folder.closing = true;
                            // SAFETY: the handle's open, and the read it cancels comes back to the port, which is
                            // when what it reads into goes.
                            unsafe {
                                CancelIoEx(folder.handle, std::ptr::null_mut());
                                CloseHandle(folder.handle);
                            }
                        }
                    }
                }
            }
        } else if !overlapped.is_null() {
            let id = key as u32;
            if let Some(folder) = folders.get_mut(&id) {
                if folder.closing {
                    folders.remove(&id);
                } else if ok == 0 {
                    // A folder that's deleted while it's watched can't be read any more. Node says it's been renamed,
                    // over and over, and this says so once.
                    gone.push((id, if error == 5 { format!("r{}", folder.path) } else { format!("e{}", code_of(error)) }));
                    // SAFETY: the read's over, so nothing else has the handle.
                    unsafe { CloseHandle(folder.handle) };
                    folders.remove(&id);
                } else {
                    let seen = if bytes == 0 { vec!["c".to_owned()] } else { changes(folder, bytes as usize) };
                    if !read_folder(folder) {
                        // SAFETY: as for the other times it's read.
                        let code = code_of(unsafe { GetLastError() });
                        // SAFETY: nothing's reading it any more.
                        unsafe { CloseHandle(folder.handle) };
                        folders.remove(&id);
                        gone.push((id, format!("e{code}")));
                    }
                    if !seen.is_empty() && tell(id, seen) {
                        if let Some(folder) = folders.get_mut(&id) {
                            folder.closing = true;
                            // SAFETY: as when it's stopped.
                            unsafe {
                                CancelIoEx(folder.handle, std::ptr::null_mut());
                                CloseHandle(folder.handle);
                            }
                        }
                    }
                }
            }
        } else if error != WAIT_TIMEOUT && ok == 0 {
            // The port itself has gone wrong, which nothing can be done about but stop.
            break;
        }
        let now = Instant::now();
        let mut ended = Vec::new();
        for (id, poll) in &mut polls.0 {
            if poll.due > now {
                continue;
            }
            poll.due = now + poll.every;
            let look = Look::of(&poll.path);
            if look.differs(&poll.last) {
                let mut told = String::from("s");
                look.write(&mut told);
                told.push(',');
                poll.last.write(&mut told);
                poll.last = look;
                if tell(*id, vec![told]) {
                    ended.push(*id);
                }
            }
        }
        for id in ended {
            polls.remove(&id);
        }
        for (id, told) in gone.drain(..) {
            tell(id, vec![told]);
        }
        if folders.is_empty() && polls.is_empty() {
            let Ok(mut shared) = shared().lock() else {
                break;
            };
            if shared.commands.is_empty() {
                shared.port = 0;
                break;
            }
        }
    }
    // SAFETY: nothing else has it, as the next watcher makes a port of its own.
    unsafe { CloseHandle(port) };
}

/// Starts watching, giving the new watcher's number, and the thread if it isn't there.
fn start(command: impl FnOnce(u32) -> Command, held: bool) -> Option<u32> {
    let mut shared = shared().lock().ok()?;
    shared.next = shared.next.wrapping_add(1).max(1);
    let id = shared.next;
    shared.watchers.insert(id, Watcher { seen: Vec::new(), later: None, held });
    shared.commands.push(command(id));
    if shared.port == 0 {
        // SAFETY: making a port of its own takes no handles but these.
        let port = unsafe { CreateIoCompletionPort(-1, 0, 0, 1) };
        if port == 0 {
            return None;
        }
        let made = port as usize;
        let started = std::thread::Builder::new()
            .name("rsui watching".into())
            .stack_size(128 * 1024)
            .spawn(move || watch(made));
        if started.is_err() {
            // SAFETY: nothing else has it.
            unsafe { CloseHandle(port) };
            return None;
        }
        shared.port = made;
    }
    // SAFETY: the port is open while it's in `shared`, which is locked.
    unsafe { PostQueuedCompletionStatus(shared.port as Handle, 0, WAKE, std::ptr::null_mut()) };
    Some(id)
}

fn number_argument(vm: &mut Vm, args: &Args, index: usize) -> Run<f64> {
    let value = vm.arg(args, index);
    vm.to_number(value)
}

/// `watchFolder(path, recursive, persistent)`, which gives a watcher's number, or throws as Node does if there's nothing
/// there. A file is watched by watching its folder for changes to it.
fn watch_folder(vm: &mut Vm, args: Args) -> Run<Value> {
    let path_value = vm.arg(&args, 0);
    let path = vm.to_rust_string(path_value)?;
    let subtree = vm.arg(&args, 1) == Value::TRUE;
    let held = vm.arg(&args, 2) != Value::FALSE;
    let found = match std::fs::metadata(&path) {
        Ok(found) => found,
        Err(error) => return vm.settle_now(super::failed(&error, "watch", path)),
    };
    let given = std::path::Path::new(&path);
    let (folder, only) = if found.is_dir() {
        (given.to_owned(), None)
    } else {
        let parent = given.parent().filter(|parent| !parent.as_os_str().is_empty());
        let name = given.file_name().map(|name| name.to_string_lossy().to_lowercase().encode_utf16().collect());
        (parent.map_or_else(|| std::path::PathBuf::from("."), std::path::Path::to_owned), name)
    };
    let wide: Vec<u16> = folder.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: the name ends in a zero and lasts through the call, and what comes back is checked.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_LIST_DIRECTORY,
            SHARE_ALL,
            std::ptr::null(),
            OPEN_EXISTING,
            BACKUP_SEMANTICS_AND_OVERLAPPED,
            0,
        )
    };
    if handle == -1 {
        let error = std::io::Error::last_os_error();
        return vm.settle_now(super::failed(&error, "watch", path));
    }
    let made = start(|id| Command::Folder { id, handle: handle as usize, path: path.clone(), subtree: subtree && only.is_none(), only }, held);
    match made {
        Some(id) => Ok(Value::number(f64::from(id))),
        None => {
            // SAFETY: the thread never had it.
            unsafe { CloseHandle(handle) };
            Err(vm.make_error(vm.realm.error_proto, "Couldn't start watching for changes"))
        }
    }
}

/// `watchPoll(path, interval, persistent)`, for `fs.watchFile`, which gives a watcher's number.
fn watch_poll(vm: &mut Vm, args: Args) -> Run<Value> {
    let path_value = vm.arg(&args, 0);
    let path = vm.to_rust_string(path_value)?;
    let every = number_argument(vm, &args, 1)?;
    let every = Duration::from_millis(if every.is_finite() && every >= 1.0 { every as u64 } else { 1 });
    let held = vm.arg(&args, 2) != Value::FALSE;
    match start(|id| Command::Poll { id, path, every }, held) {
        Some(id) => Ok(Value::number(f64::from(id))),
        None => Err(vm.make_error(vm.realm.error_proto, "Couldn't start watching for changes")),
    }
}

/// `watchNext(id)`: a promise for what's seen next, as a list, or for nothing once the watcher's closed.
fn watch_next(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = number_argument(vm, &args, 0)? as u32;
    let (promise, later) = vm.promise_for_later(true);
    let Ok(mut shared) = shared().lock() else {
        later.settle(Outcome::Nothing);
        return Ok(promise);
    };
    match shared.watchers.get_mut(&id) {
        None => {
            later.settle(Outcome::Nothing);
        }
        Some(watcher) => {
            vm.background.hold(later.id(), watcher.held);
            if watcher.seen.is_empty() {
                watcher.later = Some(later);
            } else {
                later.settle(Outcome::Names(std::mem::take(&mut watcher.seen)));
            }
        }
    }
    Ok(promise)
}

/// `watchHold(id, held)`, for `ref()` and `unref()`.
fn watch_hold(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = number_argument(vm, &args, 0)? as u32;
    let held = vm.arg(&args, 1) == Value::TRUE;
    if let Ok(mut shared) = shared().lock()
        && let Some(watcher) = shared.watchers.get_mut(&id)
    {
        watcher.held = held;
        if let Some(later) = &watcher.later {
            vm.background.hold(later.id(), held);
        }
    }
    Ok(Value::UNDEFINED)
}

/// `watchStop(id)`, which ends the watcher's promise with nothing.
fn watch_stop(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = number_argument(vm, &args, 0)? as u32;
    if let Ok(mut shared) = shared().lock()
        && let Some(watcher) = shared.watchers.remove(&id)
    {
        if let Some(later) = watcher.later {
            later.settle(Outcome::Nothing);
        }
        shared.commands.push(Command::Stop(id));
        if shared.port != 0 {
            // SAFETY: the port is open while it's in `shared`, which is locked.
            unsafe { PostQueuedCompletionStatus(shared.port as Handle, 0, WAKE, std::ptr::null_mut()) };
        }
    }
    Ok(Value::UNDEFINED)
}

pub(super) const NATIVES: [(&str, u8, crate::object::NativeFn); 5] = [
    ("watchFolder", 3, watch_folder),
    ("watchPoll", 3, watch_poll),
    ("watchNext", 1, watch_next),
    ("watchHold", 2, watch_hold),
    ("watchStop", 1, watch_stop),
];

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::{Mutex, MutexGuard};

    /// What's watched is kept for the whole process, and the first test ends by stopping all of it, which would stop
    /// the other's watcher too if they ran at the same time. So they take turns.
    fn one_at_a_time() -> MutexGuard<'static, ()> {
        static TURN: Mutex<()> = Mutex::new(());
        TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Runs a program in a folder of its own, which it's given as `dir`, and says what it printed.
    fn run(name: &str, source: &str) -> String {
        let dir = std::env::temp_dir().join(format!("rsui_js_watch_{name}_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("inside")).unwrap();
        let main = dir.join("main.js");
        std::fs::write(&main, format!("const dir = {:?};\n{source}", dir.join("inside"))).unwrap();
        let output = Rc::new(RefCell::new(String::new()));
        let mut engine = crate::Engine::new();
        let sink = output.clone();
        engine.set_printer(move |text| sink.borrow_mut().push_str(text));
        let result = engine.run_file(&main).map(|_| ()).and_then(|()| engine.run_until_idle());
        drop(engine);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(result.map_err(|error| error.message), Ok(()));
        let shown = output.borrow().replace("\r\n", "\n");
        shown.trim_end().to_owned()
    }

    #[test]
    fn changes_are_told_of_and_the_thread_goes_when_nothing_is_watched() {
        let _turn = one_at_a_time();
        let shown = run(
            "events",
            r#"
            const fs = require('fs'), path = require('path');
            const seen = [];
            const all = fs.watch(dir, { recursive: true }, (kind, name) => seen.push(`all ${kind} ${name}`));
            const one = fs.watch(path.join(dir, '..', 'main.js'), (kind, name) => seen.push(`one ${kind} ${name}`));
            const bytes = fs.watch(dir, { encoding: 'buffer' }, (kind, name) => seen.push(`bytes ${Buffer.isBuffer(name)}`));
            fs.watch(dir, { persistent: false }, () => {});
            fs.watch(dir, () => {}).unref();
            try { fs.watch(path.join(dir, 'missing')); } catch (error) { console.log(error.code, error.errno, error.syscall); }
            setTimeout(() => fs.mkdirSync(path.join(dir, 'deep')), 50);
            setTimeout(() => fs.writeFileSync(path.join(dir, 'deep', 'a.txt'), 'x'), 100);
            setTimeout(() => fs.appendFileSync(path.join(dir, '..', 'main.js'), '//'), 150);
            setTimeout(() => {
                all.close();
                one.close();
                bytes.on('close', () => console.log('closed'));
                bytes.close();
                // A folder's own change, as what's in it does, comes or doesn't depending on timing, in Node too.
                console.log([...new Set(seen)].filter((line) => line !== 'all change deep').sort().join('\n'));
            }, 400);
            "#,
        );
        assert_eq!(
            shown,
            "ENOENT -4058 watch\nall change deep\\a.txt\nall rename deep\nall rename deep\\a.txt\n\
             bytes true\none change main.js\nclosed"
        );
        // What wasn't closed was unref'd, so the program ended without it. Once those are closed too, the thread goes.
        if let Ok(mut shared) = super::shared().lock() {
            let open: Vec<u32> = shared.watchers.0.iter().map(|(id, _)| *id).collect();
            for id in open {
                shared.watchers.remove(&id);
                shared.commands.push(super::Command::Stop(id));
            }
            // SAFETY: the port is open while it's in `shared`, which is locked.
            #[allow(unsafe_code)]
            unsafe {
                super::PostQueuedCompletionStatus(shared.port as super::Handle, 0, super::WAKE, std::ptr::null_mut())
            };
        }
        let gone = (0..200).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(10));
            super::shared().lock().is_ok_and(|shared| shared.port == 0)
        });
        assert!(gone, "the watching thread is still there");
    }

    #[test]
    fn watch_file_says_what_the_file_was_and_is() {
        let _turn = one_at_a_time();
        let shown = run(
            "poll",
            r#"
            const fs = require('fs'), path = require('path');
            const file = path.join(dir, 'a.txt');
            fs.watchFile(file, { interval: 20 }, (now, before) => {
                // A look can come between the file being made and written to.
                if (now.isFile() && now.size === 0) {
                    return;
                }
                console.log(now.size, before.size, now.isFile(), before.mtimeMs === 0 || now.mtimeMs >= before.mtimeMs);
                if (now.size === 5) {
                    fs.unwatchFile(file);
                }
            });
            setTimeout(() => fs.writeFileSync(file, 'abc'), 100);
            setTimeout(() => fs.appendFileSync(file, 'de'), 200);
            fs.watchFile(path.join(dir, 'other.txt'), { persistent: false }, () => {});
            "#,
        );
        assert_eq!(shown, "0 0 false true\n3 0 true true\n5 3 true true");
    }
}
