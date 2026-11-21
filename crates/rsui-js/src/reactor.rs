//! The one thread that waits on every socket and pipe outside Windows, as [`iocp`](crate::iocp)'s does there. Unix says
//! when something can be read or written rather than doing it and saying when it's done, so what's waiting here is what
//! to do once it can: each read or write is tried straight away first, and only waits here if it couldn't be done yet.
//!
//! Linux and Android wait with `epoll`, one-shot, so each wait is asked for again when it's wanted again and nothing is
//! said twice. macOS waits with `kqueue`. Nothing about any of it is started until a program first uses a socket, a
//! pipe or another program.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::io;
use std::os::fd::RawFd;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// What to do once a file descriptor can be read or written. It's told `true` if that's because the descriptor is
/// being closed, so whatever was waiting can end.
pub(crate) type Then = Box<dyn FnOnce(bool) + Send>;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Default)]
struct Waiting {
    read: Option<Then>,
    write: Option<Then>,
}

struct Reactor {
    poller: RawFd,
    waiting: Mutex<HashMap<RawFd, Waiting>>,
}

fn reactor() -> io::Result<&'static Reactor> {
    static REACTOR: OnceLock<Result<Reactor, i32>> = OnceLock::new();
    let made = REACTOR.get_or_init(|| {
        let poller = sys::new_poller().map_err(|error| error.raw_os_error().unwrap_or(0))?;
        std::thread::Builder::new()
            .name("rsui-reactor".into())
            .stack_size(256 * 1024)
            .spawn(run)
            .map_err(|error| error.raw_os_error().unwrap_or(0))?;
        Ok(Reactor { poller, waiting: Mutex::new(HashMap::new()) })
    });
    made.as_ref().map_err(|&code| io::Error::from_raw_os_error(code))
}

/// Runs `then` once `fd` can be written (`write`) or read. Only one of each can wait on a descriptor at a time, which
/// is how everything here uses them: a stream's reads and its writes each go one after another. If it can't be waited
/// on, `then` is told it's closed, straight away, so whatever's waiting always hears.
pub(crate) fn when_ready(fd: RawFd, write: bool, then: Then) {
    if let Err(then) = try_when_ready(fd, write, then) {
        then(true);
    }
}

fn try_when_ready(fd: RawFd, write: bool, then: Then) -> Result<(), Then> {
    let Ok(reactor) = reactor() else { return Err(then) };
    let mut waiting = lock(&reactor.waiting);
    let fresh = !waiting.contains_key(&fd);
    let entry = waiting.entry(fd).or_default();
    let slot = if write { &mut entry.write } else { &mut entry.read };
    debug_assert!(slot.is_none(), "one wait of a kind at a time");
    *slot = Some(then);
    let (reading, writing) = (entry.read.is_some(), entry.write.is_some());
    // SAFETY: the descriptor is open, as the caller holds it, and the poller is ours.
    if unsafe { sys::arm(reactor.poller, fd, fresh, reading, writing) }.is_err() {
        let entry = waiting.get_mut(&fd).expect("just added");
        let slot = if write { entry.write.take() } else { entry.read.take() };
        if entry.read.is_none() && entry.write.is_none() {
            waiting.remove(&fd);
        }
        return Err(slot.expect("just put there"));
    }
    Ok(())
}

/// Ends whatever's waiting on `fd`, which is about to be closed. What was waiting is told so.
pub(crate) fn forget(fd: RawFd) {
    let Ok(reactor) = reactor() else { return };
    let gone = lock(&reactor.waiting).remove(&fd);
    // SAFETY: the descriptor is still open; it's closed by the caller after this.
    unsafe { sys::disarm(reactor.poller, fd) };
    if let Some(gone) = gone {
        for then in [gone.read, gone.write].into_iter().flatten() {
            then(true);
        }
    }
}

fn run() {
    // The poller is made before this thread is started, and this waits for it to be put where it's found.
    let Ok(reactor) = reactor() else { return };
    let mut ready = Vec::with_capacity(64);
    loop {
        ready.clear();
        // SAFETY: the poller is ours, and `ready` has room for what's asked.
        if unsafe { sys::wait(reactor.poller, &mut ready) }.is_err() {
            continue;
        }
        for &(fd, readable, writable) in &ready {
            let mut thens = Vec::with_capacity(2);
            {
                let mut waiting = lock(&reactor.waiting);
                let Some(entry) = waiting.get_mut(&fd) else { continue };
                if readable && let Some(then) = entry.read.take() {
                    thens.push(then);
                }
                if writable && let Some(then) = entry.write.take() {
                    thens.push(then);
                }
                let (read, write) = (entry.read.is_some(), entry.write.is_some());
                if read || write {
                    // What's still waiting is asked for again.
                    // SAFETY: the descriptor is still open, as something still waits on it.
                    let _ = unsafe { sys::arm(reactor.poller, fd, false, read, write) };
                } else {
                    waiting.remove(&fd);
                }
            }
            for then in thens {
                then(false);
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod sys {
    use std::io;
    use std::os::fd::RawFd;

    pub(super) fn new_poller() -> io::Result<RawFd> {
        // SAFETY: no preconditions.
        let fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if fd < 0 { Err(io::Error::last_os_error()) } else { Ok(fd) }
    }

    pub(super) unsafe fn arm(poller: RawFd, fd: RawFd, fresh: bool, read: bool, write: bool) -> io::Result<()> {
        let mut events = libc::EPOLLONESHOT as u32 | libc::EPOLLRDHUP as u32;
        if read {
            events |= libc::EPOLLIN as u32;
        }
        if write {
            events |= libc::EPOLLOUT as u32;
        }
        let mut event = libc::epoll_event { events, u64: fd as u64 };
        let op = if fresh { libc::EPOLL_CTL_ADD } else { libc::EPOLL_CTL_MOD };
        // SAFETY: as the caller says; the event is as long as the call wants.
        let mut done = unsafe { libc::epoll_ctl(poller, op, fd, &raw mut event) };
        if done != 0 {
            // It was added before and forgotten, or modified after being dropped by a close: try the other way.
            let error = io::Error::last_os_error();
            let other = match error.raw_os_error() {
                Some(libc::EEXIST) => libc::EPOLL_CTL_MOD,
                Some(libc::ENOENT) => libc::EPOLL_CTL_ADD,
                _ => return Err(error),
            };
            // SAFETY: as above.
            done = unsafe { libc::epoll_ctl(poller, other, fd, &raw mut event) };
            if done != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    pub(super) unsafe fn disarm(poller: RawFd, fd: RawFd) {
        // SAFETY: as the caller says. Linux before 2.6.9 wanted an event here, so one's given.
        unsafe {
            let mut event = libc::epoll_event { events: 0, u64: 0 };
            libc::epoll_ctl(poller, libc::EPOLL_CTL_DEL, fd, &raw mut event);
        }
    }

    pub(super) unsafe fn wait(poller: RawFd, ready: &mut Vec<(RawFd, bool, bool)>) -> io::Result<()> {
        let mut events = [libc::epoll_event { events: 0, u64: 0 }; 64];
        // SAFETY: as the caller says, and the events have room for as many as are asked for.
        let count = unsafe { libc::epoll_wait(poller, events.as_mut_ptr(), events.len() as i32, -1) };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        for event in &events[..count as usize] {
            let flags = event.events;
            let broken = flags & (libc::EPOLLERR as u32 | libc::EPOLLHUP as u32 | libc::EPOLLRDHUP as u32) != 0;
            ready.push((
                event.u64 as RawFd,
                broken || flags & libc::EPOLLIN as u32 != 0,
                broken || flags & libc::EPOLLOUT as u32 != 0,
            ));
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
mod sys {
    use std::io;
    use std::os::fd::RawFd;

    pub(super) fn new_poller() -> io::Result<RawFd> {
        // SAFETY: no preconditions.
        let fd = unsafe { libc::kqueue() };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the descriptor was just made.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        Ok(fd)
    }

    fn change(fd: RawFd, filter: i16, flags: u16) -> libc::kevent {
        // SAFETY: a kevent is plain data, and all zeros is a valid one.
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        event.ident = fd as _;
        event.filter = filter as _;
        event.flags = flags as _;
        event
    }

    pub(super) unsafe fn arm(poller: RawFd, fd: RawFd, _fresh: bool, read: bool, write: bool) -> io::Result<()> {
        let mut changes = Vec::with_capacity(2);
        let flags = (libc::EV_ADD | libc::EV_ONESHOT) as u16;
        if read {
            changes.push(change(fd, libc::EVFILT_READ as i16, flags));
        }
        if write {
            changes.push(change(fd, libc::EVFILT_WRITE as i16, flags));
        }
        // SAFETY: as the caller says; the changes are as many as it's told.
        let done = unsafe {
            libc::kevent(poller, changes.as_ptr(), changes.len() as _, std::ptr::null_mut(), 0, std::ptr::null())
        };
        if done < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    pub(super) unsafe fn disarm(poller: RawFd, fd: RawFd) {
        let flags = libc::EV_DELETE as u16;
        let changes = [change(fd, libc::EVFILT_READ as i16, flags), change(fd, libc::EVFILT_WRITE as i16, flags)];
        for one in &changes {
            // SAFETY: as the caller says. A filter that wasn't added is an error that doesn't matter.
            unsafe { libc::kevent(poller, one, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
        }
    }

    pub(super) unsafe fn wait(poller: RawFd, ready: &mut Vec<(RawFd, bool, bool)>) -> io::Result<()> {
        // SAFETY: as above.
        let mut events: [libc::kevent; 64] = unsafe { std::mem::zeroed() };
        // SAFETY: as the caller says, and the events have room for as many as are asked for.
        let count = unsafe {
            libc::kevent(poller, std::ptr::null(), 0, events.as_mut_ptr(), events.len() as _, std::ptr::null())
        };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        for event in &events[..count as usize] {
            let fd = event.ident as RawFd;
            let read = event.filter == libc::EVFILT_READ as _;
            ready.push((fd, read, !read));
        }
        Ok(())
    }
}
