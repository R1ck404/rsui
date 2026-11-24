//! What `net`, `dns` and `dgram` come down to outside Windows: TCP, Unix sockets (which a path given to `net` is, as in
//! Node), and UDP, with the same natives as `net.rs` has on Windows. No thread waits on any one of these: a read or write
//! is tried straight away, and only if it can't be done yet does it wait in [`reactor`](crate::reactor), holding no
//! buffer while it does.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr, TcpListener, ToSocketAddrs, UdpSocket};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::background::{Later, Native, Outcome};
use crate::builtins::host::bytes_of;
use crate::reactor::{self, lock};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

/// The most read at once.
const CHUNK: usize = 64 * 1024;

/// A file descriptor that's closed once, by whoever gets there first, and that whatever's waiting on it hears about.
pub(crate) struct Fd {
    raw: Mutex<Option<OwnedFd>>,
}

impl Fd {
    fn new(fd: OwnedFd) -> Arc<Self> {
        Arc::new(Self { raw: Mutex::new(Some(fd)) })
    }

    /// Does something with it while it's held open, if it still is.
    fn with<R>(&self, using: impl FnOnce(RawFd) -> R) -> Option<R> {
        lock(&self.raw).as_ref().map(|fd| using(fd.as_raw_fd()))
    }

    fn is_closed(&self) -> bool {
        lock(&self.raw).is_none()
    }

    fn close(&self) {
        let taken = lock(&self.raw).take();
        if let Some(fd) = taken {
            // What's waiting on it is told while its number is still this one's, so nothing new can have it yet.
            reactor::forget(fd.as_raw_fd());
            drop(fd);
        }
    }
}

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: the caller holds the descriptor open.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// One end of something that carries bytes both ways: a TCP connection, a Unix socket, or one of another program's
/// streams.
pub(crate) struct Stream {
    fd: Arc<Fd>,
    socket: bool,
    /// For another program's input and output, which end quietly rather than failing, as there's nobody to tell.
    child: bool,
    /// How many writes are waiting. Until there are none, a new one waits its turn behind them.
    writing: AtomicUsize,
}

impl Stream {
    fn new(fd: OwnedFd, socket: bool, child: bool) -> Arc<Self> {
        Arc::new(Self { fd: Fd::new(fd), socket, child, writing: AtomicUsize::new(0) })
    }

    /// Stops what's going on with it, and closes it.
    pub(crate) fn close(&self) {
        if self.socket {
            // SAFETY: the socket's held open through this.
            self.fd.with(|raw| unsafe { libc::shutdown(raw, libc::SHUT_RDWR) });
        }
        self.fd.close();
    }

    fn read_failed(&self, error: &io::Error) -> Outcome {
        let ended = matches!(error.raw_os_error(), Some(libc::EPIPE | libc::ECONNRESET | libc::EBADF));
        if (ended && !self.socket) || self.child || self.fd.is_closed() {
            Outcome::Null
        } else {
            failed(error, "read", "")
        }
    }

    fn write_failed(&self, error: &io::Error) -> Outcome {
        if self.child {
            Outcome::Nothing
        } else if self.fd.is_closed() || error.raw_os_error() == Some(libc::EPIPE) {
            Outcome::NetFailed { code: "EPIPE", message: "write EPIPE".to_owned() }
        } else {
            failed(error, "write", "")
        }
    }

    fn heard(&self, heard: Heard) -> Outcome {
        match heard {
            Heard::Bytes(bytes) => Outcome::Bytes(bytes),
            Heard::End | Heard::Nothing => Outcome::Null,
            Heard::Failed(error) => self.read_failed(&error),
        }
    }
}

enum Heard {
    Bytes(Vec<u8>),
    End,
    Nothing,
    Failed(io::Error),
}

thread_local! {
    /// What's read into on this thread, before it's copied out at the length it turned out to be.
    static SPACE: RefCell<Vec<u8>> = RefCell::new(vec![0; CHUNK]);
}

fn read_now(raw: RawFd) -> Heard {
    SPACE.with_borrow_mut(|space| loop {
        // SAFETY: the caller holds the descriptor open, and the space is as long as it says.
        let got = unsafe { libc::read(raw, space.as_mut_ptr().cast(), CHUNK) };
        match got {
            0 => return Heard::End,
            length if length > 0 => return Heard::Bytes(space[..length as usize].to_vec()),
            _ => {
                let error = io::Error::last_os_error();
                match error.kind() {
                    io::ErrorKind::WouldBlock => return Heard::Nothing,
                    io::ErrorKind::Interrupted => {}
                    _ => return Heard::Failed(error),
                }
            }
        }
    })
}

/// Waits for something to read, then reads it.
fn wait_to_read(stream: Arc<Stream>, later: Later) {
    let Some(raw) = stream.fd.with(|raw| raw) else {
        return later.settle(Outcome::Null);
    };
    let waiting = stream.clone();
    let mut later = Some(later);
    let then: reactor::Then = Box::new(move |closed| {
        let later = later.take().expect("once");
        if closed {
            return later.settle(Outcome::Null);
        }
        match waiting.fd.with(read_now) {
            None => later.settle(Outcome::Null),
            Some(Heard::Nothing) => wait_to_read(waiting, later),
            Some(heard) => {
                let outcome = waiting.heard(heard);
                later.settle(outcome);
            }
        }
    });
    reactor::when_ready(raw, false, then);
}

/// A promise for the next bytes from a stream, or `null` at the end.
pub(crate) fn read_stream(vm: &mut Vm, stream: &Arc<Stream>) -> Value {
    match stream.fd.with(read_now) {
        None => return vm.settled_promise(Outcome::Null),
        Some(Heard::Nothing) => {}
        Some(heard) => return vm.settled_promise(stream.heard(heard)),
    }
    // Nothing has to be going on for a socket to be waiting, so that doesn't count as something to wait for.
    let (promise, later) = vm.promise_for_later(!stream.child);
    wait_to_read(stream.clone(), later);
    promise
}

fn write_now(raw: RawFd, bytes: &[u8]) -> io::Result<usize> {
    loop {
        // SAFETY: the caller holds the descriptor open, and the bytes are as long as it says. A socket that's gone is
        // an error here rather than a signal that ends the program.
        let sent = unsafe {
            if is_socket(raw) {
                libc::send(raw, bytes.as_ptr().cast(), bytes.len(), NO_SIGNAL)
            } else {
                libc::write(raw, bytes.as_ptr().cast(), bytes.len())
            }
        };
        if sent >= 0 {
            return Ok(sent as usize);
        }
        let error = io::Error::last_os_error();
        match error.kind() {
            io::ErrorKind::WouldBlock => return Ok(0),
            io::ErrorKind::Interrupted => {}
            _ => return Err(error),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
const NO_SIGNAL: libc::c_int = libc::MSG_NOSIGNAL;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const NO_SIGNAL: libc::c_int = 0;

fn is_socket(raw: RawFd) -> bool {
    // SAFETY: a stat of an open descriptor into a zeroed buffer.
    unsafe {
        let mut stat: libc::stat = std::mem::zeroed();
        libc::fstat(raw, &raw mut stat) == 0 && (stat.st_mode & libc::S_IFMT) == libc::S_IFSOCK
    }
}

/// Writes what's left of some bytes, however long that takes.
fn write_rest(stream: Arc<Stream>, bytes: Vec<u8>, mut done: usize, later: Later) {
    loop {
        let wrote = match stream.fd.with(|raw| write_now(raw, &bytes[done..])) {
            None => Err(io::Error::from_raw_os_error(libc::EBADF)),
            Some(wrote) => wrote,
        };
        match wrote {
            Ok(0) => break,
            Ok(wrote) => {
                done += wrote;
                if done == bytes.len() {
                    stream.writing.fetch_sub(1, Ordering::AcqRel);
                    return later.settle(Outcome::Nothing);
                }
            }
            Err(error) => {
                let outcome = stream.write_failed(&error);
                stream.writing.fetch_sub(1, Ordering::AcqRel);
                return later.settle(outcome);
            }
        }
    }
    let Some(raw) = stream.fd.with(|raw| raw) else {
        let outcome = stream.write_failed(&io::Error::from_raw_os_error(libc::EBADF));
        stream.writing.fetch_sub(1, Ordering::AcqRel);
        return later.settle(outcome);
    };
    let waiting = stream.clone();
    let mut parts = Some((bytes, later));
    let then: reactor::Then = Box::new(move |closed| {
        let (bytes, later) = parts.take().expect("once");
        if closed {
            let outcome = waiting.write_failed(&io::Error::from_raw_os_error(libc::EBADF));
            waiting.writing.fetch_sub(1, Ordering::AcqRel);
            return later.settle(outcome);
        }
        write_rest(waiting, bytes, done, later);
    });
    let _ = reactor::when_ready(raw, true, then);
}

/// A promise for some bytes having gone to a stream.
pub(crate) fn write_stream(vm: &mut Vm, stream: &Arc<Stream>, bytes: Vec<u8>) -> Value {
    if bytes.is_empty() {
        return vm.settled_promise(Outcome::Nothing);
    }
    let mut done = 0;
    if stream.writing.load(Ordering::Acquire) == 0 {
        match stream.fd.with(|raw| write_now(raw, &bytes)) {
            None => return vm.settled_promise(stream.write_failed(&io::Error::from_raw_os_error(libc::EBADF))),
            Some(Ok(wrote)) if wrote == bytes.len() => return vm.settled_promise(Outcome::Nothing),
            Some(Ok(wrote)) => done = wrote,
            Some(Err(error)) => return vm.settled_promise(stream.write_failed(&error)),
        }
    }
    let (promise, later) = vm.promise_for_later(false);
    stream.writing.fetch_add(1, Ordering::AcqRel);
    write_rest(stream.clone(), bytes, done, later);
    promise
}

/// A pipe for another program's input or output: this end, which doesn't wait, and the end to give it, which does, as a
/// program expects of what it's given. `reading` is whether this end is the one that reads.
pub(crate) fn child_pipe(reading: bool) -> io::Result<(Arc<Stream>, OwnedFd)> {
    let mut ends = [0; 2];
    // SAFETY: room for two descriptors. Neither is inherited by anything started later, except as it's given.
    if unsafe { libc::pipe(ends.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both were just made, and nothing else has them.
    let (read_end, write_end) = unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) };
    for end in [&read_end, &write_end] {
        // SAFETY: the descriptor is open.
        unsafe { libc::fcntl(end.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    let (ours, theirs) = if reading { (read_end, write_end) } else { (write_end, read_end) };
    set_nonblocking(ours.as_raw_fd())?;
    Ok((Stream::new(ours, false, true), theirs))
}

enum Listening {
    Tcp(Arc<Fd>),
    Unix { fd: Arc<Fd>, path: String },
}

struct Datagrams {
    fd: Arc<Fd>,
    v6: bool,
}

#[derive(Default)]
struct Sockets {
    connections: HashMap<u32, Arc<Stream>>,
    listening: HashMap<u32, Arc<Listening>>,
    datagrams: HashMap<u32, Arc<Datagrams>>,
}

fn sockets() -> &'static Mutex<Sockets> {
    static ALL: OnceLock<Mutex<Sockets>> = OnceLock::new();
    ALL.get_or_init(Mutex::default)
}

fn next_number() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn keep_connection(stream: Arc<Stream>) -> u32 {
    let number = next_number();
    lock(sockets()).connections.insert(number, stream);
    number
}

fn connection(number: u32) -> Option<Arc<Stream>> {
    lock(sockets()).connections.get(&number).cloned()
}

/// The word Node has for a problem, which is what code looks at.
fn code_of(error: &io::Error) -> &'static str {
    match error.raw_os_error() {
        Some(libc::ECONNREFUSED) => "ECONNREFUSED",
        Some(libc::ETIMEDOUT) => "ETIMEDOUT",
        Some(libc::ECONNRESET) => "ECONNRESET",
        Some(libc::ECONNABORTED) => "ECONNABORTED",
        Some(libc::EADDRINUSE) => "EADDRINUSE",
        Some(libc::EADDRNOTAVAIL) => "EADDRNOTAVAIL",
        Some(libc::EACCES | libc::EPERM) => "EACCES",
        Some(libc::ENETUNREACH) => "ENETUNREACH",
        Some(libc::EHOSTUNREACH) => "EHOSTUNREACH",
        Some(libc::ENOENT) => "ENOENT",
        Some(libc::EPIPE) => "EPIPE",
        Some(libc::EBADF) => "EBADF",
        _ if error.kind() == io::ErrorKind::NotFound => "ENOTFOUND",
        _ => "EIO",
    }
}

fn failed(error: &io::Error, doing: &str, target: &str) -> Outcome {
    let code = code_of(error);
    Outcome::NetFailed { code, message: format!("{doing} {code} {target}") }
}

fn not_found(host: &str) -> Outcome {
    Outcome::NetFailed { code: "ENOTFOUND", message: format!("getaddrinfo ENOTFOUND {host}") }
}

fn address_names(number: u32, here: Option<SocketAddr>, there: Option<SocketAddr>) -> Outcome {
    let part = |address: Option<SocketAddr>| match address {
        Some(address) => (address.ip().to_string(), address.port().to_string()),
        None => (String::new(), String::new()),
    };
    let (here_address, here_port) = part(here);
    let (there_address, there_port) = part(there);
    Outcome::Names(vec![number.to_string(), here_address, here_port, there_address, there_port])
}

fn number_arg(vm: &mut Vm, args: &Args, at: usize) -> Run<u32> {
    let value = vm.arg(args, at);
    Ok(vm.to_number(value)? as u32)
}

fn text_arg(vm: &mut Vm, args: &Args, at: usize) -> Run<String> {
    let value = vm.arg(args, at);
    vm.to_rust_string(value)
}

/// Looks a host up, and does something with where it is: straight away for an address, and otherwise on one of the
/// threads for files, as looking a name up waits for the answer.
fn find_then(vm: &mut Vm, host: String, port: u16, then: impl FnOnce(io::Result<Vec<SocketAddr>>) + Send + 'static) {
    if let Ok(ip) = host.parse::<IpAddr>() {
        then(Ok(vec![SocketAddr::new(ip, port)]));
        return;
    }
    vm.in_pool(move || then((host.as_str(), port).to_socket_addrs().map(Iterator::collect)));
}

/// The system's form of an address.
fn raw_address(address: SocketAddr) -> (libc::sockaddr_storage, libc::socklen_t) {
    // SAFETY: all zeros is a valid address of any kind, and each kind is written into the room for any.
    unsafe {
        let mut storage: libc::sockaddr_storage = std::mem::zeroed();
        let size = match address {
            SocketAddr::V4(v4) => {
                let raw = (&raw mut storage).cast::<libc::sockaddr_in>();
                (*raw).sin_family = libc::AF_INET as _;
                (*raw).sin_port = v4.port().to_be();
                (*raw).sin_addr = libc::in_addr { s_addr: u32::from_ne_bytes(v4.ip().octets()) };
                size_of::<libc::sockaddr_in>()
            }
            SocketAddr::V6(v6) => {
                let raw = (&raw mut storage).cast::<libc::sockaddr_in6>();
                (*raw).sin6_family = libc::AF_INET6 as _;
                (*raw).sin6_port = v6.port().to_be();
                (*raw).sin6_addr = libc::in6_addr { s6_addr: v6.ip().octets() };
                (*raw).sin6_flowinfo = v6.flowinfo();
                (*raw).sin6_scope_id = v6.scope_id();
                size_of::<libc::sockaddr_in6>()
            }
        };
        (storage, size as libc::socklen_t)
    }
}

/// The address at one end of a socket: this one's, or (`there`) the other's.
fn name_of(raw: RawFd, there: bool) -> Option<SocketAddr> {
    // A borrowed socket, to ask std, which knows how to read the answer; it isn't closed after.
    // SAFETY: the caller holds the socket open, and `ManuallyDrop` keeps this from closing it.
    let socket = std::mem::ManuallyDrop::new(unsafe { std::net::TcpStream::from_raw_fd(raw) });
    if there { socket.peer_addr().ok() } else { socket.local_addr().ok() }
}

fn new_socket(v6: bool, kind: libc::c_int) -> io::Result<OwnedFd> {
    let family = if v6 { libc::AF_INET6 } else { libc::AF_INET };
    // SAFETY: no preconditions.
    let raw = unsafe { libc::socket(family, kind, 0) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: just made, and nothing else has it.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    // SAFETY: the descriptor is open.
    unsafe { libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) };
    #[cfg(target_vendor = "apple")]
    {
        let on: libc::c_int = 1;
        // SAFETY: as above; the value is as long as it says. A socket that's gone is an error, not a signal.
        unsafe {
            libc::setsockopt(raw, libc::SOL_SOCKET, libc::SO_NOSIGPIPE, (&raw const on).cast(), 4);
        }
    }
    set_nonblocking(raw)?;
    Ok(fd)
}

/// Connecting to each address a host has in turn, until one answers.
fn connect_next(mut addresses: std::vec::IntoIter<SocketAddr>, target: String, last: Option<io::Error>, later: Later) {
    let mut last = last;
    for address in addresses.by_ref() {
        let fd = match new_socket(address.is_ipv6(), libc::SOCK_STREAM) {
            Ok(fd) => fd,
            Err(error) => {
                last = Some(error);
                continue;
            }
        };
        let (raw_to, size) = raw_address(address);
        // SAFETY: the socket is open, and the address is as long as it says.
        let started = unsafe { libc::connect(fd.as_raw_fd(), (&raw const raw_to).cast(), size) };
        if started == 0 {
            return connected(fd, later);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINPROGRESS) {
            last = Some(error);
            continue;
        }
        let raw = fd.as_raw_fd();
        let mut parts = Some((fd, addresses, target, later));
        let then: reactor::Then = Box::new(move |closed| {
            let (fd, addresses, target, later) = parts.take().expect("once");
            if closed {
                return connect_next(addresses, target, Some(io::Error::from_raw_os_error(libc::EBADF)), later);
            }
            let mut code: libc::c_int = 0;
            let mut length = size_of::<libc::c_int>() as libc::socklen_t;
            // SAFETY: the socket is open, and the value is as long as it says.
            unsafe {
                libc::getsockopt(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_ERROR, (&raw mut code).cast(), &raw mut length);
            }
            if code == 0 {
                connected(fd, later);
            } else {
                connect_next(addresses, target, Some(io::Error::from_raw_os_error(code)), later);
            }
        });
        reactor::when_ready(raw, true, then);
        return;
    }
    let outcome = match &last {
        Some(error) => failed(error, "connect", &target),
        None => not_found(&target),
    };
    later.settle(outcome);
}

fn connected(fd: OwnedFd, later: Later) {
    let raw = fd.as_raw_fd();
    let (here, there) = (name_of(raw, false), name_of(raw, true));
    let number = keep_connection(Stream::new(fd, true, false));
    later.settle(address_names(number, here, there));
}

/// `netConnect(host, port)`: a promise for its number, and the addresses at each end.
fn net_connect(vm: &mut Vm, args: Args) -> Run<Value> {
    let host = text_arg(vm, &args, 0)?;
    let port = number_arg(vm, &args, 1)? as u16;
    let (promise, later) = vm.promise_for_later(false);
    let target = format!("{host}:{port}");
    let named = host.clone();
    find_then(vm, host, port, move |found| match found {
        Ok(addresses) => connect_next(addresses.into_iter(), target, None, later),
        Err(error) => later.settle(failed(&error, "getaddrinfo", &named)),
    });
    Ok(promise)
}

/// `pipeConnect(path)`: the same, for a Unix socket at a path.
fn pipe_connect(vm: &mut Vm, args: Args) -> Run<Value> {
    let path = text_arg(vm, &args, 0)?;
    Ok(vm.in_background(move || {
        match UnixStream::connect(&path).and_then(|stream| stream.set_nonblocking(true).map(|()| stream)) {
            Ok(stream) => address_names(keep_connection(Stream::new(stream.into(), true, false)), None, None),
            Err(error) => failed(&error, "connect", &path),
        }
    }))
}

/// `netRead(number)`: a promise for the next bytes, or `null` at the end.
fn net_read(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    Ok(match connection(number) {
        Some(stream) => read_stream(vm, &stream),
        None => vm.settled_promise(Outcome::Null),
    })
}

/// `netWrite(number, bytes)`: a promise for it having gone.
fn net_write(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    let given = vm.arg(&args, 1);
    let bytes = bytes_of(vm, given)?;
    Ok(match connection(number) {
        Some(stream) => write_stream(vm, &stream, bytes),
        None => vm.settled_promise(Outcome::NetFailed { code: "EPIPE", message: "write EPIPE".to_owned() }),
    })
}

/// `netEnd(number)`: this end has nothing more to say, though it can still hear.
fn net_end(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    if let Some(stream) = connection(number) {
        // SAFETY: the socket's held open through this.
        stream.fd.with(|raw| unsafe { libc::shutdown(raw, libc::SHUT_WR) });
    }
    Ok(Value::UNDEFINED)
}

/// `netClose(number)`.
fn net_close(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    let found = lock(sockets()).connections.remove(&number);
    if let Some(stream) = found {
        stream.close();
    }
    Ok(Value::UNDEFINED)
}

fn net_no_delay(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    let on: libc::c_int = i32::from(vm.arg(&args, 1) != Value::FALSE);
    if let Some(stream) = connection(number) {
        stream.fd.with(|raw| {
            // SAFETY: the socket's held open through this, and the value is as long as it says. A Unix socket says no,
            // which doesn't matter.
            unsafe { libc::setsockopt(raw, libc::IPPROTO_TCP, libc::TCP_NODELAY, (&raw const on).cast(), 4) }
        });
    }
    Ok(Value::UNDEFINED)
}

fn listen_tcp(addresses: Vec<SocketAddr>, target: &str) -> Outcome {
    let mut last = None;
    for address in addresses {
        match TcpListener::bind(address).and_then(|listener| listener.set_nonblocking(true).map(|()| listener)) {
            Ok(listener) => {
                let at = listener.local_addr().ok();
                let number = next_number();
                let fd = Fd::new(listener.into());
                lock(sockets()).listening.insert(number, Arc::new(Listening::Tcp(fd)));
                return address_names(number, at, None);
            }
            Err(error) => last = Some(error),
        }
    }
    match last {
        Some(error) => failed(&error, "listen", target),
        None => not_found(target),
    }
}

/// `netListen(host, port)`: a promise for its number, and where it's listening.
fn net_listen(vm: &mut Vm, args: Args) -> Run<Value> {
    let host = text_arg(vm, &args, 0)?;
    let port = number_arg(vm, &args, 1)? as u16;
    let target = format!("{host}:{port}");
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vm.settled_promise(listen_tcp(vec![SocketAddr::new(ip, port)], &target)));
    }
    Ok(vm.in_background(move || match (host.as_str(), port).to_socket_addrs() {
        Ok(addresses) => listen_tcp(addresses.collect(), &target),
        Err(error) => failed(&error, "getaddrinfo", &host),
    }))
}

/// `pipeListen(path)`: a Unix socket at a path.
fn pipe_listen(vm: &mut Vm, args: Args) -> Run<Value> {
    let path = text_arg(vm, &args, 0)?;
    let outcome = match UnixListener::bind(&path).and_then(|listener| listener.set_nonblocking(true).map(|()| listener)) {
        Ok(listener) => {
            let number = next_number();
            let fd = Fd::new(listener.into());
            lock(sockets()).listening.insert(number, Arc::new(Listening::Unix { fd, path }));
            address_names(number, None, None)
        }
        Err(error) => failed(&error, "listen", &path),
    };
    Ok(vm.settled_promise(outcome))
}

/// Takes the next that's connected, if one has.
fn accept_now(raw: RawFd) -> io::Result<Option<OwnedFd>> {
    loop {
        // SAFETY: the caller holds the socket open; the addresses aren't wanted here.
        let accepted = unsafe { libc::accept(raw, std::ptr::null_mut(), std::ptr::null_mut()) };
        if accepted >= 0 {
            // SAFETY: just made, and nothing else has it.
            let fd = unsafe { OwnedFd::from_raw_fd(accepted) };
            // SAFETY: the descriptor is open.
            unsafe { libc::fcntl(accepted, libc::F_SETFD, libc::FD_CLOEXEC) };
            set_nonblocking(accepted)?;
            return Ok(Some(fd));
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EAGAIN) => return Ok(None),
            #[allow(unreachable_patterns)]
            Some(libc::EWOULDBLOCK) => return Ok(None),
            // What connected went away before it was taken, which is nothing to say anything about.
            Some(libc::EINTR | libc::ECONNABORTED) => {}
            _ => return Err(error),
        }
    }
}

fn accept_next(listening: Arc<Listening>, later: Later) {
    let (fd, tcp) = match &*listening {
        Listening::Tcp(fd) => (fd.clone(), true),
        Listening::Unix { fd, .. } => (fd.clone(), false),
    };
    match fd.with(accept_now) {
        None => later.settle(Outcome::Null),
        Some(Ok(Some(accepted))) => {
            let raw = accepted.as_raw_fd();
            let (here, there) = if tcp { (name_of(raw, false), name_of(raw, true)) } else { (None, None) };
            let number = keep_connection(Stream::new(accepted, true, false));
            later.settle(address_names(number, here, there));
        }
        Some(Ok(None)) => {
            let Some(raw) = fd.with(|raw| raw) else { return later.settle(Outcome::Null) };
            let mut parts = Some((listening, later));
            let then: reactor::Then = Box::new(move |closed| {
                let (listening, later) = parts.take().expect("once");
                if closed { later.settle(Outcome::Null) } else { accept_next(listening, later) }
            });
            let _ = reactor::when_ready(raw, false, then);
        }
        Some(Err(_)) if fd.is_closed() => later.settle(Outcome::Null),
        Some(Err(error)) => later.settle(failed(&error, "accept", "")),
    }
}

/// `netAccept(number)`: a promise for the next that connects, with its addresses, or `null` once it's stopped listening. It
/// doesn't count as something to wait for.
fn net_accept(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    let Some(found) = lock(sockets()).listening.get(&number).cloned() else {
        return Ok(vm.settled_promise(Outcome::Null));
    };
    let (promise, later) = vm.promise_for_later(true);
    accept_next(found, later);
    Ok(promise)
}

/// `netStopListening(number)`. What's waiting for a connection ends with `null`.
fn net_stop_listening(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    let found = lock(sockets()).listening.remove(&number);
    match found.as_deref() {
        Some(Listening::Tcp(fd)) => fd.close(),
        Some(Listening::Unix { fd, path }) => {
            fd.close();
            // The socket's file stays behind otherwise, and the next to listen there would find it in use.
            let _ = std::fs::remove_file(path);
        }
        None => {}
    }
    Ok(Value::UNDEFINED)
}

/// `dnsLookup(host)`: a promise for its addresses.
fn dns_lookup(vm: &mut Vm, args: Args) -> Run<Value> {
    let host = text_arg(vm, &args, 0)?;
    Ok(vm.in_background(move || match (host.as_str(), 0).to_socket_addrs() {
        Ok(addresses) => {
            let mut names: Vec<String> = addresses.map(|one| one.ip().to_string()).collect();
            names.dedup();
            Outcome::Names(names)
        }
        Err(_) => not_found(&host),
    }))
}

fn bind_udp(addresses: Vec<SocketAddr>, target: &str) -> Outcome {
    let mut last = None;
    for address in addresses {
        match UdpSocket::bind(address).and_then(|socket| socket.set_nonblocking(true).map(|()| socket)) {
            Ok(socket) => {
                let at = socket.local_addr().ok();
                let number = next_number();
                let datagrams = Datagrams { fd: Fd::new(socket.into()), v6: address.is_ipv6() };
                lock(sockets()).datagrams.insert(number, Arc::new(datagrams));
                return address_names(number, at, None);
            }
            Err(error) => last = Some(error),
        }
    }
    match last {
        Some(error) => failed(&error, "bind", target),
        None => not_found(target),
    }
}

/// `udpBind(host, port)`.
fn udp_bind(vm: &mut Vm, args: Args) -> Run<Value> {
    let host = text_arg(vm, &args, 0)?;
    let port = number_arg(vm, &args, 1)? as u16;
    let target = format!("{host}:{port}");
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vm.settled_promise(bind_udp(vec![SocketAddr::new(ip, port)], &target)));
    }
    Ok(vm.in_background(move || match (host.as_str(), port).to_socket_addrs() {
        Ok(addresses) => bind_udp(addresses.collect(), &target),
        Err(error) => failed(&error, "bind", &target),
    }))
}

fn datagrams(number: u32) -> Option<Arc<Datagrams>> {
    lock(sockets()).datagrams.get(&number).cloned()
}

fn no_datagrams() -> Outcome {
    Outcome::NetFailed { code: "EBADF", message: "send EBADF".to_owned() }
}

fn send_datagram(socket: Arc<Datagrams>, bytes: Vec<u8>, to: SocketAddr, target: String, later: Later) {
    let (address, size) = raw_address(to);
    let sent = socket.fd.with(|raw| loop {
        // SAFETY: the socket's held open through this, and the bytes and address are as long as they say.
        let sent = unsafe { libc::sendto(raw, bytes.as_ptr().cast(), bytes.len(), 0, (&raw const address).cast(), size) };
        if sent >= 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        match error.kind() {
            io::ErrorKind::WouldBlock => return Ok(false),
            io::ErrorKind::Interrupted => {}
            _ => return Err(error),
        }
    });
    match sent {
        None => later.settle(no_datagrams()),
        Some(Ok(true)) => later.settle(Outcome::Nothing),
        Some(Ok(false)) => {
            let Some(raw) = socket.fd.with(|raw| raw) else { return later.settle(no_datagrams()) };
            let mut parts = Some((socket, bytes, target, later));
            let then: reactor::Then = Box::new(move |closed| {
                let (socket, bytes, target, later) = parts.take().expect("once");
                if closed { later.settle(no_datagrams()) } else { send_datagram(socket, bytes, to, target, later) }
            });
            let _ = reactor::when_ready(raw, true, then);
        }
        Some(Err(error)) => later.settle(failed(&error, "send", &target)),
    }
}

/// `udpSend(number, bytes, host, port)`.
fn udp_send(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    let given = vm.arg(&args, 1);
    let bytes = bytes_of(vm, given)?;
    let host = text_arg(vm, &args, 2)?;
    let port = number_arg(vm, &args, 3)? as u16;
    let Some(socket) = datagrams(number) else {
        return Ok(vm.settled_promise(no_datagrams()));
    };
    let (promise, later) = vm.promise_for_later(false);
    let target = format!("{host}:{port}");
    // This machine is the usual place to send to, and it's the same kind of address as the socket that's sending.
    let host = if host == "localhost" { if socket.v6 { "::1" } else { "127.0.0.1" }.to_owned() } else { host };
    find_then(vm, host, port, move |found| {
        let addresses = match found {
            Ok(addresses) => addresses,
            Err(error) => return later.settle(failed(&error, "send", &target)),
        };
        let to = addresses.iter().find(|address| address.is_ipv6() == socket.v6).or(addresses.first()).copied();
        match to {
            Some(to) => send_datagram(socket, bytes, to, target, later),
            None => later.settle(not_found(&target)),
        }
    });
    Ok(promise)
}

enum Received {
    Message(Vec<u8>, SocketAddr),
    Nothing,
    Failed(io::Error),
}

fn receive_now(raw: RawFd) -> Received {
    // SAFETY: a borrowed socket, as in `name_of`, which std reads the sender's address for.
    let socket = std::mem::ManuallyDrop::new(unsafe { UdpSocket::from_raw_fd(raw) });
    SPACE.with_borrow_mut(|space| loop {
        match socket.recv_from(space) {
            Ok((length, from)) => return Received::Message(space[..length].to_vec(), from),
            Err(error) => match error.kind() {
                io::ErrorKind::WouldBlock => return Received::Nothing,
                io::ErrorKind::Interrupted | io::ErrorKind::ConnectionRefused => {}
                _ => return Received::Failed(error),
            },
        }
    })
}

fn received(received: Received) -> Outcome {
    match received {
        Received::Message(bytes, from) => Outcome::Datagram { bytes, address: from.ip().to_string(), port: from.port() },
        Received::Nothing => Outcome::Null,
        Received::Failed(error) => failed(&error, "recv", ""),
    }
}

fn wait_to_receive(socket: Arc<Datagrams>, later: Later) {
    let Some(raw) = socket.fd.with(|raw| raw) else { return later.settle(Outcome::Null) };
    let mut parts = Some((socket, later));
    let then: reactor::Then = Box::new(move |closed| {
        let (socket, later) = parts.take().expect("once");
        if closed {
            return later.settle(Outcome::Null);
        }
        match socket.fd.with(receive_now) {
            None => later.settle(Outcome::Null),
            Some(Received::Nothing) => wait_to_receive(socket, later),
            Some(got) => later.settle(received(got)),
        }
    });
    let _ = reactor::when_ready(raw, false, then);
}

/// `udpReceive(number)`: a promise for what's next, or `null` once it's closed. It doesn't count as something to wait for.
fn udp_receive(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    let Some(socket) = datagrams(number) else {
        return Ok(vm.settled_promise(Outcome::Null));
    };
    match socket.fd.with(receive_now) {
        None => return Ok(vm.settled_promise(Outcome::Null)),
        Some(Received::Nothing) => {}
        Some(got) => return Ok(vm.settled_promise(received(got))),
    }
    let (promise, later) = vm.promise_for_later(true);
    wait_to_receive(socket, later);
    Ok(promise)
}

/// `udpClose(number)`. What's waiting for a message ends with `null`.
fn udp_close(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    let found = lock(sockets()).datagrams.remove(&number);
    if let Some(socket) = found {
        socket.fd.close();
    }
    Ok(Value::UNDEFINED)
}

pub(crate) const NATIVES: [Native; 16] = [
    ("netConnect", 2, net_connect),
    ("pipeConnect", 1, pipe_connect),
    ("netRead", 1, net_read),
    ("netWrite", 2, net_write),
    ("netEnd", 1, net_end),
    ("netClose", 1, net_close),
    ("netNoDelay", 2, net_no_delay),
    ("netListen", 2, net_listen),
    ("pipeListen", 1, pipe_listen),
    ("netAccept", 1, net_accept),
    ("netStopListening", 1, net_stop_listening),
    ("dnsLookup", 1, dns_lookup),
    ("udpBind", 2, udp_bind),
    ("udpSend", 4, udp_send),
    ("udpReceive", 1, udp_receive),
    ("udpClose", 1, udp_close),
];
