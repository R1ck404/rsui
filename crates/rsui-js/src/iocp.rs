//! The one thread that waits on every socket and pipe there is, through what Windows calls an I/O completion port. A read,
//! a write, a connection or an accept is started on whatever thread asks for it, and the system tells this thread when it's
//! finished, which keeps the promise that's waiting on it. The thread's started when there's first something to wait for, and
//! goes once there's nothing left, so a program that doesn't use sockets never has it, and one with many open has one thread
//! for them all.
#![allow(unsafe_code)]

use std::io;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

pub(crate) const ERROR_OPERATION_ABORTED: i32 = 995;
const INVALID: isize = -1;
const INFINITE: u32 = u32::MAX;
/// How many finished things the thread can pick up at once.
const ENTRIES: usize = 64;
const STACK: usize = 256 * 1024;

/// What the system keeps for each thing that's started: how it went, and how much was done.
#[repr(C)]
#[derive(Default)]
pub(crate) struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: isize,
}

/// What the system hands back for each thing that's finished.
#[repr(C)]
#[derive(Clone, Copy)]
struct Entry {
    key: usize,
    overlapped: *mut Overlapped,
    internal: usize,
    bytes: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateIoCompletionPort(file: isize, port: isize, key: usize, threads: u32) -> isize;
    fn GetQueuedCompletionStatusEx(
        port: isize,
        entries: *mut Entry,
        count: u32,
        removed: *mut u32,
        wait: u32,
        alertable: i32,
    ) -> i32;
    fn PostQueuedCompletionStatus(
        port: isize,
        bytes: u32,
        key: usize,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn CancelIoEx(file: isize, overlapped: *mut Overlapped) -> i32;
    fn CloseHandle(handle: isize) -> i32;
}

#[link(name = "ws2_32")]
unsafe extern "system" {
    fn closesocket(socket: usize) -> i32;
    fn WSAGetOverlappedResult(
        socket: usize,
        overlapped: *mut Overlapped,
        bytes: *mut u32,
        wait: i32,
        flags: *mut u32,
    ) -> i32;
    fn WSAGetLastError() -> i32;
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlNtStatusToDosError(status: i32) -> u32;
}

/// A lock that's still usable after a thread panicked while holding it. Nothing here is left half done by that.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn aborted() -> io::Error {
    io::Error::from_raw_os_error(ERROR_OPERATION_ABORTED)
}

struct Port {
    handle: isize,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// How many things have been started that the system's yet to say are finished.
    pending: usize,
    running: bool,
}

fn port() -> io::Result<&'static Port> {
    static PORT: OnceLock<Option<Port>> = OnceLock::new();
    PORT.get_or_init(|| {
        // SAFETY: this only makes a port, which lasts for good. One thread at a time takes what's finished from it.
        let handle = unsafe { CreateIoCompletionPort(INVALID, 0, 0, 1) };
        (handle != 0).then(|| Port {
            handle,
            state: Mutex::new(State::default()),
        })
    })
    .as_ref()
    .ok_or_else(|| io::Error::other("the system wouldn't make a completion port"))
}

impl Port {
    /// Counts something that's about to start, with the thread there to hear about it.
    fn begin(&'static self) -> io::Result<()> {
        let mut state = lock(&self.state);
        if !state.running {
            std::thread::Builder::new()
                .name("rsui sockets".into())
                .stack_size(STACK)
                .spawn(move || self.serve())?;
            state.running = true;
        }
        state.pending += 1;
        Ok(())
    }

    /// For something that was counted, and then didn't start. If that's the last, the thread's told so that it can go.
    fn never_began(&self) {
        let mut state = lock(&self.state);
        state.pending = state.pending.saturating_sub(1);
        if state.pending == 0 && state.running {
            // SAFETY: the port lasts for good. Nothing's overlapped in this, so all it does is wake the thread to look.
            unsafe {
                PostQueuedCompletionStatus(self.handle, 0, 0, std::ptr::null_mut());
            }
        }
    }

    fn serve(&self) {
        let mut entries = [Entry {
            key: 0,
            overlapped: std::ptr::null_mut(),
            internal: 0,
            bytes: 0,
        }; ENTRIES];
        loop {
            let mut removed = 0;
            // SAFETY: the port lasts for good, and the entries are as many as it's told, and last through the call.
            let got = unsafe {
                GetQueuedCompletionStatusEx(
                    self.handle,
                    entries.as_mut_ptr(),
                    ENTRIES as u32,
                    &raw mut removed,
                    INFINITE,
                    0,
                )
            };
            let mut finished = 0;
            if got != 0 {
                for entry in &entries[..removed as usize] {
                    if entry.overlapped.is_null() {
                        continue;
                    }
                    finished += 1;
                    let header = entry.overlapped.cast::<Header>();
                    // SAFETY: everything started on a handle that's tied to this port was started by `start`, so what's
                    // overlapped is the header at the start of an `Op` it gave up, and the system gives each back once.
                    unsafe { ((*header).finish)(header, entry.bytes) };
                }
            }
            let mut state = lock(&self.state);
            debug_assert!(state.pending >= finished, "more finished than were started");
            // A panic here would end the whole program, as it's built to abort on one.
            state.pending = state.pending.saturating_sub(finished);
            // It can only fail to wait if the port's gone wrong, and then the next thing to start tries again with a new thread.
            if state.pending == 0 || got == 0 {
                state.running = false;
                return;
            }
        }
    }
}

/// A socket or a file that's tied to the port, with what it's called by the system for as long as it's open. Everything that
/// uses it holds the lock, so that once it's closed, nothing can use it again by mistake for whatever's given its number next.
pub(crate) struct Handle {
    raw: Mutex<isize>,
    socket: bool,
}

impl Handle {
    /// Takes over a socket, which is closed if it can't be tied to the port.
    pub(crate) fn socket(raw: usize) -> io::Result<Arc<Self>> {
        Self::tie(raw as isize, true)
    }

    /// Takes over a file or pipe that was opened to be overlapped, which is closed if it can't be tied to the port.
    pub(crate) fn file(raw: isize) -> io::Result<Arc<Self>> {
        Self::tie(raw, false)
    }

    fn tie(raw: isize, socket: bool) -> io::Result<Arc<Self>> {
        let made = Arc::new(Self {
            raw: Mutex::new(raw),
            socket,
        });
        let port = port()?;
        // SAFETY: the handle is open, and belongs to `made`. This only says where the system's to say what's finished.
        if unsafe { CreateIoCompletionPort(raw, port.handle, 0, 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(made)
    }

    /// Does something with it, if it's still open, and holds it open until that's done.
    pub(crate) fn with<R>(&self, using: impl FnOnce(isize) -> R) -> Option<R> {
        let raw = lock(&self.raw);
        (*raw != INVALID).then(|| using(*raw))
    }

    pub(crate) fn is_closed(&self) -> bool {
        *lock(&self.raw) == INVALID
    }

    /// Stops what's going on with it, and closes it. What was going on comes back to the thread as cancelled, and is
    /// finished from there.
    pub(crate) fn close(&self) {
        let mut raw = lock(&self.raw);
        if *raw == INVALID {
            return;
        }
        // SAFETY: the handle's open, and nothing else can use it while this holds it. Once this is done, nothing will,
        // as everything looks here first. What was started on it has its own memory, which lasts until it comes back.
        unsafe {
            CancelIoEx(*raw, std::ptr::null_mut());
            if self.socket {
                closesocket(*raw as usize);
            } else {
                CloseHandle(*raw);
            }
        }
        *raw = INVALID;
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.close();
    }
}

/// What the thread finds first in everything it's given back.
#[repr(C)]
struct Header {
    overlapped: Overlapped,
    finish: unsafe fn(*mut Header, u32),
}

/// Something that's been started, or is about to be, with what it needs until it's finished: the handle it's on, and the
/// buffers and promise in `work`. It's on the heap from being started to being finished, so it can't move while the system
/// is writing to it, and it's only freed once the system is done with it, even if the handle's closed first.
#[repr(C)]
pub(crate) struct Op<T> {
    header: Header,
    pub(crate) handle: Arc<Handle>,
    pub(crate) work: T,
}

/// Each kind of thing that can be started on a handle: how it's started, and what to do once it's finished.
pub(crate) trait Work: Sized + Send + 'static {
    /// Starts it on `raw`. It's `Ok` if the system's going to say when it's finished, which can be straight away.
    ///
    /// # Safety
    ///
    /// `op` is whole and isn't moved or freed until the system gives it back, and `raw` is its handle, held open through this.
    /// Once the system has it, it could be finished and freed on the thread at any moment, so nothing may touch `op` after
    /// starting it.
    unsafe fn start(op: *mut Op<Self>, raw: isize) -> io::Result<()>;

    /// Runs on the thread, with how it went: how many bytes were done, or what went wrong.
    fn finish(op: Box<Op<Self>>, done: io::Result<u32>);
}

impl<T: Work> Op<T> {
    pub(crate) fn new(handle: Arc<Handle>, work: T) -> Box<Self> {
        Box::new(Self {
            header: Header {
                overlapped: Overlapped::default(),
                finish: finish_op::<T>,
            },
            handle,
            work,
        })
    }

    /// What's handed to the system, which is at the very start of this.
    pub(crate) fn overlapped(op: *mut Self) -> *mut Overlapped {
        op.cast()
    }

    /// How it went, from what the system left in it.
    fn done(&mut self, bytes: u32) -> io::Result<u32> {
        // The low half is the system's own status, which is a failure when it's negative.
        let status = self.header.overlapped.internal as u32 as i32;
        if status >= 0 {
            return Ok(bytes);
        }
        if !self.handle.socket {
            // SAFETY: this only turns one number into another.
            let code = unsafe { RtlNtStatusToDosError(status) };
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        // A socket says what went wrong in its own words, as Node's codes do.
        let overlapped = &raw mut self.header.overlapped;
        let failed = self.handle.with(|raw| {
            let (mut done, mut flags) = (0, 0);
            // SAFETY: the socket's held open through this, and what's overlapped is what it was started with, which is finished.
            let finished = unsafe {
                WSAGetOverlappedResult(raw as usize, overlapped, &raw mut done, 0, &raw mut flags)
            };
            // SAFETY: this only reads the thread's last error.
            (finished == 0).then(|| unsafe { WSAGetLastError() })
        });
        match failed {
            Some(Some(code)) => Err(io::Error::from_raw_os_error(code)),
            Some(None) => Ok(bytes),
            None => Err(aborted()),
        }
    }
}

/// # Safety
///
/// `header` is the start of an `Op<T>` that `start` gave up, which the system has given back.
unsafe fn finish_op<T: Work>(header: *mut Header, bytes: u32) {
    // SAFETY: `finish_op::<T>` is only ever put in the header of an `Op<T>`, which is `repr(C)` with the header first, and
    // which `start` made from a box. The system is done with it, so this is its only owner.
    let mut op = unsafe { Box::from_raw(header.cast::<Op<T>>()) };
    let done = op.done(bytes);
    T::finish(op, done);
}

/// Starts something, which is finished on the thread. If it couldn't start, it comes back with what went wrong, and nothing
/// will be said about it.
pub(crate) fn start<T: Work>(mut op: Box<Op<T>>) -> Result<(), (Box<Op<T>>, io::Error)> {
    let handle = op.handle.clone();
    let raw = lock(&handle.raw);
    if *raw == INVALID {
        return Err((op, aborted()));
    }
    let port = match port() {
        Ok(port) => port,
        Err(error) => return Err((op, error)),
    };
    if let Err(error) = port.begin() {
        return Err((op, error));
    }
    op.header.overlapped = Overlapped::default();
    let pointer = Box::into_raw(op);
    // SAFETY: `pointer` is a whole `Op` that nothing else has, and the handle's held open until this returns. The box isn't
    // freed until the thread has it back, and nothing here touches it once it's started.
    match unsafe { T::start(pointer, *raw) } {
        Ok(()) => Ok(()),
        Err(error) => {
            port.never_began();
            // SAFETY: the system won't give back what didn't start, so this is still its only owner.
            Err((unsafe { Box::from_raw(pointer) }, error))
        }
    }
}

/// Whether a call that returns zero for having failed has started, or finished straight away. Either way the thread hears
/// when it's done.
pub(crate) fn started(result: i32) -> io::Result<()> {
    if result != 0 {
        return Ok(());
    }
    pending_or(io::Error::last_os_error())
}

/// The same, for the socket calls that return zero for having done it.
pub(crate) fn started_socket(result: i32) -> io::Result<()> {
    if result == 0 {
        return Ok(());
    }
    // SAFETY: this only reads the thread's last error.
    pending_or(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }))
}

/// The same, for the socket calls that return nonzero for having done it.
pub(crate) fn started_socket_bool(result: i32) -> io::Result<()> {
    if result != 0 {
        return Ok(());
    }
    // SAFETY: this only reads the thread's last error.
    pending_or(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }))
}

fn pending_or(error: io::Error) -> io::Result<()> {
    const ERROR_IO_PENDING: i32 = 997;
    if error.raw_os_error() == Some(ERROR_IO_PENDING) {
        Ok(())
    } else {
        Err(error)
    }
}
