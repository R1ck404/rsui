//! Watching folders and files for changes outside Windows, with the same natives as `watch.rs` has there. One thread
//! looks after every watcher in the program: on Linux and Android it waits on `inotify`, which the system wakes when a
//! folder it watches changes, or until the next file to look at for `fs.watchFile` is due. While nothing changes it uses
//! no CPU, and once the last watcher's closed it goes away.
//!
//! `inotify` doesn't watch a folder's subfolders, so a recursive watch adds one for each folder in it, and for each new
//! one as it comes. macOS has no `inotify`; until its `FSEvents` is written there, a folder there is watched by looking
//! at what's in it twice a second.
//!
//! What a watcher has seen is written as `watch.rs` writes it: `r` and a name for something that came or went, `c` and a
//! name for something that changed, `c` alone when so much changed that the system couldn't say what, `e` and a code
//! for an error, and `s` and two stats for a file `fs.watchFile` saw change.

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::background::{Later, Outcome};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

/// The most changes held for a watcher nothing's asking, so one that's forgotten can't grow for good.
const MOST_HELD: usize = 4096;

struct Watcher {
    seen: Vec<String>,
    later: Option<Later>,
    held: bool,
}

enum Command {
    Folder { id: u32, folder: PathBuf, path: String, subtree: bool, only: Option<String> },
    Poll { id: u32, path: String, every: Duration },
    Stop(u32),
}

#[derive(Default)]
struct Shared {
    watchers: HashMap<u32, Watcher>,
    commands: Vec<Command>,
    /// What wakes the watching thread, while there is that thread.
    wake: Option<Wake>,
    next: u32,
}

fn shared() -> &'static Mutex<Shared> {
    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
    SHARED.get_or_init(Mutex::default)
}

/// A pipe whose reading end the watching thread waits on as well, so it can be told there's something to do.
#[derive(Clone, Copy)]
struct Wake {
    read: i32,
    write: i32,
}

impl Wake {
    fn new() -> Option<Self> {
        let mut ends = [0; 2];
        // SAFETY: room for two descriptors.
        if unsafe { libc::pipe(ends.as_mut_ptr()) } != 0 {
            return None;
        }
        for end in ends {
            // SAFETY: both are open.
            unsafe {
                libc::fcntl(end, libc::F_SETFD, libc::FD_CLOEXEC);
                libc::fcntl(end, libc::F_SETFL, libc::fcntl(end, libc::F_GETFL) | libc::O_NONBLOCK);
            }
        }
        Some(Self { read: ends[0], write: ends[1] })
    }

    fn wake(self) {
        // SAFETY: the pipe's open while it's in `shared`. A full pipe already has a wake waiting in it.
        unsafe { libc::write(self.write, [1u8].as_ptr().cast(), 1) };
    }

    fn drain(self) {
        let mut room = [0u8; 64];
        // SAFETY: the pipe doesn't wait, and the room is as long as it says.
        while unsafe { libc::read(self.read, room.as_mut_ptr().cast(), room.len()) } > 0 {}
    }

    fn close(self) {
        // SAFETY: nothing else has these once the thread's going.
        unsafe {
            libc::close(self.read);
            libc::close(self.write);
        }
    }
}

/// What `fs.watchFile` knows of a file the last time it looked.
#[derive(Clone, Copy, PartialEq)]
struct Look {
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
                // Not every file system knows when a file was made; Node says the change time then, near enough.
                born: ms(found.created().or_else(|_| found.modified())),
            },
            Err(_) => Self { kind: 0, size: 0.0, accessed: 0.0, modified: 0.0, born: 0.0 },
        }
    }

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
    let Ok(mut shared) = shared().lock() else { return false };
    let Some(watcher) = shared.watchers.get_mut(&id) else { return false };
    watcher.seen.extend(seen);
    if watcher.seen.len() > MOST_HELD {
        let extra = watcher.seen.len() - MOST_HELD;
        watcher.seen.drain(..extra);
    }
    let Some(later) = watcher.later.take() else { return false };
    let seen = std::mem::take(&mut watcher.seen);
    if later.settle_heard(Outcome::Names(seen)) {
        return false;
    }
    shared.watchers.remove(&id);
    true
}

fn code_of(error: &std::io::Error) -> &'static str {
    match error.raw_os_error() {
        Some(libc::ENOENT | libc::ENOTDIR) => "ENOENT",
        Some(libc::EACCES | libc::EPERM) => "EPERM",
        Some(libc::ENOSPC) => "ENOSPC",
        _ => "EIO",
    }
}

/// A folder being watched, by whichever means this system has.
struct Folder {
    root: PathBuf,
    path: String,
    subtree: bool,
    only: Option<String>,
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    listing: HashMap<PathBuf, (u64, f64)>,
}

impl Folder {
    /// Whether something by this name, inside the folder, is what's asked about.
    fn wanted(&self, name: &str) -> bool {
        self.only.as_ref().is_none_or(|only| only == name)
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod system {
    use super::*;

    pub(super) const IN_MODIFY: u32 = 0x2;
    pub(super) const IN_ATTRIB: u32 = 0x4;
    pub(super) const IN_CLOSE_WRITE: u32 = 0x8;
    pub(super) const IN_MOVED_FROM: u32 = 0x40;
    pub(super) const IN_MOVED_TO: u32 = 0x80;
    pub(super) const IN_CREATE: u32 = 0x100;
    pub(super) const IN_DELETE: u32 = 0x200;
    pub(super) const IN_DELETE_SELF: u32 = 0x400;
    pub(super) const IN_MOVE_SELF: u32 = 0x800;
    pub(super) const IN_Q_OVERFLOW: u32 = 0x4000;
    pub(super) const IN_IGNORED: u32 = 0x8000;
    pub(super) const IN_ISDIR: u32 = 0x4000_0000;
    const EVERY_CHANGE: u32 =
        IN_MODIFY | IN_ATTRIB | IN_CLOSE_WRITE | IN_MOVED_FROM | IN_MOVED_TO | IN_CREATE | IN_DELETE | IN_DELETE_SELF | IN_MOVE_SELF;

    /// What the thread keeps: the `inotify` descriptor, and for each of its watches, which watcher it's for and where
    /// it is under that watcher's folder.
    pub(super) struct Watching {
        pub(super) fd: i32,
        pub(super) watches: HashMap<i32, (u32, PathBuf)>,
    }

    impl Watching {
        pub(super) fn new() -> Option<Self> {
            // SAFETY: no preconditions.
            let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
            (fd >= 0).then(|| Self { fd, watches: HashMap::new() })
        }

        /// Watches one folder, as part of watcher `id`, at `under` inside its folder.
        pub(super) fn add(&mut self, id: u32, folder: &Path, under: PathBuf) -> std::io::Result<()> {
            use std::os::unix::ffi::OsStrExt;
            let mut name = folder.as_os_str().as_bytes().to_vec();
            name.push(0);
            // SAFETY: the name ends in a zero.
            let wd = unsafe { libc::inotify_add_watch(self.fd, name.as_ptr().cast(), EVERY_CHANGE) };
            if wd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            self.watches.insert(wd, (id, under));
            Ok(())
        }

        /// Watches a folder and everything in it.
        pub(super) fn add_all(&mut self, id: u32, folder: &Path, under: PathBuf) {
            if self.add(id, folder, under.clone()).is_err() {
                return;
            }
            let Ok(entries) = std::fs::read_dir(folder) else { return };
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    self.add_all(id, &entry.path(), under.join(entry.file_name()));
                }
            }
        }

        pub(super) fn remove(&mut self, id: u32) {
            let gone: Vec<i32> = self.watches.iter().filter(|(_, (of, _))| *of == id).map(|(wd, _)| *wd).collect();
            for wd in gone {
                self.watches.remove(&wd);
                // SAFETY: the descriptor's open; a watch that's gone already is an error that doesn't matter.
                // Android's C library takes the watch as unsigned; glibc's as signed.
                unsafe { libc::inotify_rm_watch(self.fd, wd as _) };
            }
        }

        /// What's come, as (watch, what happened, name) for each.
        pub(super) fn read(&mut self) -> Vec<(i32, u32, String)> {
            let mut events = Vec::new();
            let mut room = vec![0u8; 64 * 1024];
            loop {
                // SAFETY: the descriptor doesn't wait, and the room is as long as it says.
                let got = unsafe { libc::read(self.fd, room.as_mut_ptr().cast(), room.len()) };
                if got <= 0 {
                    break;
                }
                let mut at = 0;
                let got = got as usize;
                // Each is a `struct inotify_event`: the watch, what happened, a cookie, the name's length, then the name.
                while at + 16 <= got {
                    let word = |offset: usize| u32::from_ne_bytes(room[at + offset..at + offset + 4].try_into().unwrap());
                    let (wd, mask, length) = (word(0) as i32, word(4), word(12) as usize);
                    let name = room.get(at + 16..at + 16 + length).unwrap_or_default();
                    let name = String::from_utf8_lossy(name.split(|&byte| byte == 0).next().unwrap_or_default()).into_owned();
                    events.push((wd, mask, name));
                    at += 16 + length;
                }
            }
            events
        }
    }
}

/// The watching thread, for as long as there's something to watch.
fn watch(wake: Wake) {
    let mut folders: HashMap<u32, Folder> = HashMap::new();
    let mut polls: HashMap<u32, Poll> = HashMap::new();
    let mut gone: Vec<(u32, String)> = Vec::new();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let Some(mut watching) = system::Watching::new() else {
        if let Ok(mut shared) = shared().lock() {
            shared.wake = None;
        }
        wake.close();
        return;
    };
    loop {
        let now = Instant::now();
        #[cfg_attr(any(target_os = "linux", target_os = "android"), allow(unused_mut))]
        let mut wait = polls.values().map(|poll| poll.due.saturating_duration_since(now)).min();
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        if !folders.is_empty() {
            wait = Some(wait.map_or(Duration::from_millis(500), |wait| wait.min(Duration::from_millis(500))));
        }
        let timeout = wait.map_or(-1, |wait| wait.as_millis().min(i32::MAX as u128) as i32);
        let mut fds = vec![libc::pollfd { fd: wake.read, events: libc::POLLIN, revents: 0 }];
        #[cfg(any(target_os = "linux", target_os = "android"))]
        fds.push(libc::pollfd { fd: watching.fd, events: libc::POLLIN, revents: 0 });
        // SAFETY: the descriptors are open, and the list is as long as it says.
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
        if fds[0].revents != 0 {
            wake.drain();
        }
        let commands = shared().lock().map(|mut shared| std::mem::take(&mut shared.commands)).unwrap_or_default();
        for command in commands {
            match command {
                Command::Folder { id, folder, path, subtree, only } => {
                    #[cfg(any(target_os = "linux", target_os = "android"))]
                    {
                        let added = if subtree {
                            watching.add(id, &folder, PathBuf::new()).map(|()| {
                                watching.remove(id);
                                watching.add_all(id, &folder, PathBuf::new());
                            })
                        } else {
                            watching.add(id, &folder, PathBuf::new())
                        };
                        match added {
                            Ok(()) => {
                                folders.insert(id, Folder { root: folder, path, subtree, only });
                            }
                            Err(error) => gone.push((id, format!("e{}", code_of(&error)))),
                        }
                    }
                    #[cfg(not(any(target_os = "linux", target_os = "android")))]
                    {
                        let mut made = Folder { root: folder, path, subtree, only, listing: HashMap::new() };
                        made.listing = listing(&made.root, subtree);
                        folders.insert(id, made);
                    }
                }
                Command::Poll { id, path, every } => {
                    let last = Look::of(&path);
                    if last.kind == 0 {
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
                    if folders.remove(&id).is_some() {
                        #[cfg(any(target_os = "linux", target_os = "android"))]
                        watching.remove(id);
                    }
                }
            }
        }
        let mut seen: HashMap<u32, Vec<String>> = HashMap::new();
        #[cfg(any(target_os = "linux", target_os = "android"))]
        for (wd, mask, name) in watching.read() {
            use system::*;
            if mask & IN_Q_OVERFLOW != 0 {
                for id in folders.keys() {
                    seen.entry(*id).or_default().push("c".to_owned());
                }
                continue;
            }
            let Some((id, under)) = watching.watches.get(&wd).cloned() else { continue };
            let Some(folder) = folders.get(&id) else { continue };
            if mask & IN_IGNORED != 0 {
                watching.watches.remove(&wd);
                if under.as_os_str().is_empty() {
                    // The folder itself has gone. Node says it's been renamed, and this says so once.
                    gone.push((id, format!("r{}", folder.path)));
                    folders.remove(&id);
                    watching.remove(id);
                }
                continue;
            }
            if mask & (IN_DELETE_SELF | IN_MOVE_SELF) != 0 || name.is_empty() {
                continue;
            }
            if under.as_os_str().is_empty() && !folder.wanted(&name) {
                continue;
            }
            let relative = under.join(&name);
            if folder.subtree && mask & IN_ISDIR != 0 && mask & (IN_CREATE | IN_MOVED_TO) != 0 {
                let root = folder.root.clone();
                watching.add_all(id, &root.join(&relative), relative.clone());
            }
            let kind = if mask & (IN_CREATE | IN_DELETE | IN_MOVED_FROM | IN_MOVED_TO) != 0 { 'r' } else { 'c' };
            seen.entry(id).or_default().push(format!("{kind}{}", relative.to_string_lossy()));
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        for (id, folder) in &mut folders {
            let now = listing(&folder.root, folder.subtree);
            let mut changes = Vec::new();
            for (path, stamp) in &now {
                match folder.listing.get(path) {
                    None => changes.push(format!("r{}", path.to_string_lossy())),
                    Some(before) if before != stamp => changes.push(format!("c{}", path.to_string_lossy())),
                    Some(_) => {}
                }
            }
            for path in folder.listing.keys() {
                if !now.contains_key(path) {
                    changes.push(format!("r{}", path.to_string_lossy()));
                }
            }
            changes.retain(|change| folder.only.as_ref().is_none_or(|only| &change[1..] == only));
            folder.listing = now;
            if !changes.is_empty() {
                seen.entry(*id).or_default().extend(changes);
            }
        }
        for (id, changes) in seen {
            if tell(id, changes) {
                folders.remove(&id);
                #[cfg(any(target_os = "linux", target_os = "android"))]
                watching.remove(id);
            }
        }
        let now = Instant::now();
        let mut ended = Vec::new();
        for (id, poll) in &mut polls {
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
            let Ok(mut shared) = shared().lock() else { break };
            if shared.commands.is_empty() {
                shared.wake = None;
                break;
            }
        }
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    // SAFETY: nothing else has it.
    unsafe {
        libc::close(watching.fd)
    };
    wake.close();
}

/// What's in a folder (and its folders, for a `subtree`), with each one's size and when it last changed.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn listing(root: &Path, subtree: bool) -> HashMap<PathBuf, (u64, f64)> {
    fn walk(root: &Path, under: &Path, subtree: bool, into: &mut HashMap<PathBuf, (u64, f64)>) {
        let Ok(entries) = std::fs::read_dir(root.join(under)) else { return };
        for entry in entries.flatten() {
            let relative = under.join(entry.file_name());
            let Ok(found) = entry.metadata() else { continue };
            let modified = found
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0.0, |since| since.as_secs_f64());
            into.insert(relative.clone(), (found.len(), modified));
            if subtree && found.is_dir() {
                walk(root, &relative, subtree, into);
            }
        }
    }
    let mut all = HashMap::new();
    walk(root, Path::new(""), subtree, &mut all);
    all
}

/// Starts watching, giving the new watcher's number, and the thread if it isn't there.
fn start(command: impl FnOnce(u32) -> Command, held: bool) -> Option<u32> {
    let mut shared = shared().lock().ok()?;
    shared.next = shared.next.wrapping_add(1).max(1);
    let id = shared.next;
    shared.watchers.insert(id, Watcher { seen: Vec::new(), later: None, held });
    shared.commands.push(command(id));
    if shared.wake.is_none() {
        let wake = Wake::new()?;
        let started = std::thread::Builder::new().name("rsui watching".into()).stack_size(128 * 1024).spawn(move || watch(wake));
        if started.is_err() {
            wake.close();
            return None;
        }
        shared.wake = Some(wake);
    }
    if let Some(wake) = shared.wake {
        wake.wake();
    }
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
    let given = Path::new(&path);
    let (folder, only) = if found.is_dir() {
        (given.to_owned(), None)
    } else {
        let parent = given.parent().filter(|parent| !parent.as_os_str().is_empty());
        let name = given.file_name().map(|name| name.to_string_lossy().into_owned());
        (parent.map_or_else(|| PathBuf::from("."), Path::to_owned), name)
    };
    let subtree = subtree && only.is_none();
    match start(|id| Command::Folder { id, folder, path: path.clone(), subtree, only }, held) {
        Some(id) => Ok(Value::number(f64::from(id))),
        None => Err(vm.make_error(vm.realm.error_proto, "Couldn't start watching for changes")),
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
        None => later.settle(Outcome::Nothing),
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
        if let Some(wake) = shared.wake {
            wake.wake();
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
        output.borrow().trim_end().to_owned()
    }

    #[test]
    fn changes_in_a_folder_and_its_folders_are_told_of() {
        let shown = run(
            "events",
            r#"
            const fs = require('fs'), path = require('path');
            const seen = [];
            const all = fs.watch(dir, { recursive: true }, (kind, name) => seen.push(`all ${kind} ${name}`));
            try { fs.watch(path.join(dir, 'missing')); } catch (error) { console.log(error.code, error.syscall); }
            setTimeout(() => fs.mkdirSync(path.join(dir, 'deep')), 50);
            setTimeout(() => fs.writeFileSync(path.join(dir, 'deep', 'a.txt'), 'x'), 150);
            setTimeout(() => {
                all.close();
                console.log([...new Set(seen)].filter((line) => line !== 'all change deep').sort().join('\n'));
            }, 1400);
            "#,
        );
        assert!(shown.starts_with("ENOENT watch\n"), "{shown}");
        assert!(shown.contains("all rename deep\n"), "{shown}");
        assert!(shown.contains("deep/a.txt"), "{shown}");
    }
}
