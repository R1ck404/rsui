//! What `net`, `dns` and `dgram` come down to: TCP, the system's named pipes, and UDP. No thread waits on any one of these:
//! what's started on them is finished by the one thread in [`iocp`](crate::iocp), and a read or write that can be done
//! straight away is. Sockets don't wait when they're read or written that way, so a read that's waiting for something
//! to arrive asks the system to say when there is, with nothing to put it in, and reads it once it's there. That way an
//! idle connection holds no buffer at all. None of it's reached by a program that doesn't use it.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock};

use crate::background::{Later, Native, Outcome};
use crate::builtins::host::bytes_of;
use crate::iocp::{self, Handle, Op, Overlapped, Work, aborted, lock};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_BROKEN_PIPE: i32 = 109;
const ERROR_PIPE_BUSY: i32 = 231;
const ERROR_NO_DATA: i32 = 232;
const ERROR_PIPE_NOT_CONNECTED: i32 = 233;
const ERROR_PIPE_CONNECTED: i32 = 535;
const WSAEWOULDBLOCK: i32 = 10035;
const WSAECONNABORTED: i32 = 10053;
const WSAECONNRESET: i32 = 10054;

const INVALID: isize = -1;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const PIPE_ACCESS_INBOUND: u32 = 1;
const PIPE_ACCESS_OUTBOUND: u32 = 2;
const PIPE_ACCESS_DUPLEX: u32 = 3;
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const PIPE_BUFFER: u32 = 64 * 1024;

const AF_INET: u16 = 2;
const AF_INET6: u16 = 23;
const SOCK_STREAM: i32 = 1;
const SOCK_DGRAM: i32 = 2;
const IPPROTO_TCP: i32 = 6;
const IPPROTO_UDP: i32 = 17;
const WSA_FLAG_OVERLAPPED: u32 = 0x01;
const WSA_FLAG_NO_HANDLE_INHERIT: u32 = 0x80;
const SOL_SOCKET: i32 = 0xffff;
const SO_UPDATE_ACCEPT_CONTEXT: i32 = 0x700B;
const SO_UPDATE_CONNECT_CONTEXT: i32 = 0x7010;
const TCP_NODELAY: i32 = 1;
const FIONBIO: i32 = 0x8004_667E_u32 as i32;
const SIO_GET_EXTENSION_FUNCTION_POINTER: u32 = 0xC800_0006;
const SIO_UDP_CONNRESET: u32 = 0x9800_000C;
const MSG_PEEK: u32 = 2;
const SD_SEND: i32 = 1;
const SD_BOTH: i32 = 2;
/// As many as Node lets wait to be accepted.
const BACKLOG: i32 = 511;

/// The most that's read at once, which is as much as Node reads.
const CHUNK: usize = 64 * 1024;
/// Room for an address, as `AcceptEx` wants it: the biggest there is, and sixteen more.
const ADDRESS_ROOM: usize = 128 + 16;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const c_void,
        disposition: u32,
        flags: u32,
        template: isize,
    ) -> isize;
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        instances: u32,
        out_size: u32,
        in_size: u32,
        timeout: u32,
        security: *const c_void,
    ) -> isize;
    fn ConnectNamedPipe(pipe: isize, overlapped: *mut Overlapped) -> i32;
    fn WaitNamedPipeW(name: *const u16, timeout: u32) -> i32;
    fn ReadFile(
        file: isize,
        buffer: *mut c_void,
        size: u32,
        read: *mut u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn WriteFile(
        file: isize,
        buffer: *const c_void,
        size: u32,
        written: *mut u32,
        overlapped: *mut Overlapped,
    ) -> i32;
}

#[repr(C)]
struct WsaBuf {
    len: u32,
    buf: *mut u8,
}

#[link(name = "ws2_32")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut c_void) -> i32;
    fn WSASocketW(
        family: i32,
        kind: i32,
        protocol: i32,
        info: *const c_void,
        group: u32,
        flags: u32,
    ) -> usize;
    fn WSAGetLastError() -> i32;
    fn WSAIoctl(
        socket: usize,
        code: u32,
        input: *const c_void,
        input_size: u32,
        output: *mut c_void,
        output_size: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
        routine: *const c_void,
    ) -> i32;
    fn WSARecv(
        socket: usize,
        buffers: *mut WsaBuf,
        count: u32,
        received: *mut u32,
        flags: *mut u32,
        overlapped: *mut Overlapped,
        routine: *const c_void,
    ) -> i32;
    fn WSASend(
        socket: usize,
        buffers: *const WsaBuf,
        count: u32,
        sent: *mut u32,
        flags: u32,
        overlapped: *mut Overlapped,
        routine: *const c_void,
    ) -> i32;
    fn WSASendTo(
        socket: usize,
        buffers: *const WsaBuf,
        count: u32,
        sent: *mut u32,
        flags: u32,
        to: *const c_void,
        to_size: i32,
        overlapped: *mut Overlapped,
        routine: *const c_void,
    ) -> i32;
    fn bind(socket: usize, address: *const c_void, size: i32) -> i32;
    fn listen(socket: usize, backlog: i32) -> i32;
    fn shutdown(socket: usize, how: i32) -> i32;
    fn setsockopt(socket: usize, level: i32, name: i32, value: *const c_void, size: i32) -> i32;
    fn getsockname(socket: usize, address: *mut c_void, size: *mut i32) -> i32;
    fn getpeername(socket: usize, address: *mut c_void, size: *mut i32) -> i32;
    fn ioctlsocket(socket: usize, command: i32, argument: *mut u32) -> i32;
    fn recv(socket: usize, buffer: *mut c_void, size: i32, flags: i32) -> i32;
    fn send(socket: usize, buffer: *const c_void, size: i32, flags: i32) -> i32;
    fn recvfrom(
        socket: usize,
        buffer: *mut c_void,
        size: i32,
        flags: i32,
        from: *mut c_void,
        from_size: *mut i32,
    ) -> i32;
    fn sendto(
        socket: usize,
        buffer: *const c_void,
        size: i32,
        flags: i32,
        to: *const c_void,
        to_size: i32,
    ) -> i32;
}

/// How `AcceptEx` is called, which the system has to be asked for.
type AcceptEx = unsafe extern "system" fn(
    listening: usize,
    accepted: usize,
    buffer: *mut c_void,
    receive: u32,
    here_size: u32,
    there_size: u32,
    received: *mut u32,
    overlapped: *mut Overlapped,
) -> i32;

/// The same, for `ConnectEx`.
type ConnectEx = unsafe extern "system" fn(
    socket: usize,
    to: *const c_void,
    to_size: i32,
    send: *const c_void,
    send_size: u32,
    sent: *mut u32,
    overlapped: *mut Overlapped,
) -> i32;

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

const WSAID_ACCEPTEX: Guid = Guid(
    0xb536_7df1,
    0xcbac,
    0x11cf,
    [0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92],
);
const WSAID_CONNECTEX: Guid = Guid(
    0x25a2_07b9,
    0xddf3,
    0x4660,
    [0x8e, 0xe9, 0x76, 0xe5, 0x8c, 0x74, 0x06, 0x3e],
);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_socket_error() -> io::Error {
    // SAFETY: this only reads the thread's last error.
    io::Error::from_raw_os_error(unsafe { WSAGetLastError() })
}

/// Sockets have to be asked for once before any can be made. It's never undone, as they could be needed again at any time.
fn start_sockets() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let mut data = [0u64; 64];
        // SAFETY: that's more room than the system's description of itself takes, and it lasts through the call.
        unsafe {
            WSAStartup(0x0202, data.as_mut_ptr().cast());
        }
    });
}

/// An address as the system has it.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct RawAddress {
    bytes: [u8; 128],
}

impl RawAddress {
    fn empty() -> Self {
        Self { bytes: [0; 128] }
    }

    fn of(address: SocketAddr) -> (Self, i32) {
        let mut out = Self::empty();
        let bytes = &mut out.bytes;
        bytes[2..4].copy_from_slice(&address.port().to_be_bytes());
        match address {
            SocketAddr::V4(address) => {
                bytes[0..2].copy_from_slice(&AF_INET.to_ne_bytes());
                bytes[4..8].copy_from_slice(&address.ip().octets());
                (out, 16)
            }
            SocketAddr::V6(address) => {
                bytes[0..2].copy_from_slice(&AF_INET6.to_ne_bytes());
                bytes[4..8].copy_from_slice(&address.flowinfo().to_ne_bytes());
                bytes[8..24].copy_from_slice(&address.ip().octets());
                bytes[24..28].copy_from_slice(&address.scope_id().to_ne_bytes());
                (out, 28)
            }
        }
    }

    fn address(&self) -> Option<SocketAddr> {
        let bytes = &self.bytes;
        let port = u16::from_be_bytes([bytes[2], bytes[3]]);
        match u16::from_ne_bytes([bytes[0], bytes[1]]) {
            AF_INET => {
                let ip = Ipv4Addr::new(bytes[4], bytes[5], bytes[6], bytes[7]);
                Some(SocketAddr::new(ip.into(), port))
            }
            AF_INET6 => {
                let mut ip = [0; 16];
                ip.copy_from_slice(&bytes[8..24]);
                let flow = u32::from_ne_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                let scope = u32::from_ne_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
                Some(SocketAddr::V6(std::net::SocketAddrV6::new(
                    ip.into(),
                    port,
                    flow,
                    scope,
                )))
            }
            _ => None,
        }
    }
}

/// Where a socket is, or what it's connected to.
fn name_of(raw: isize, there: bool) -> Option<SocketAddr> {
    let mut address = RawAddress::empty();
    let mut size = address.bytes.len() as i32;
    let pointer = (&raw mut address.bytes).cast();
    // SAFETY: the caller holds the socket open, and the address has as much room as it says.
    let found = unsafe {
        if there {
            getpeername(raw as usize, pointer, &raw mut size)
        } else {
            getsockname(raw as usize, pointer, &raw mut size)
        }
    };
    if found == 0 { address.address() } else { None }
}

/// So that reading and writing it straight away never waits. What's started overlapped isn't changed by this.
fn dont_wait(raw: isize) {
    let mut on = 1u32;
    // SAFETY: the caller holds the socket open.
    unsafe {
        ioctlsocket(raw as usize, FIONBIO, &raw mut on);
    }
}

/// A function the system has for sockets, which isn't one it exports.
fn extension(raw: isize, which: &Guid) -> io::Result<usize> {
    let mut found = 0usize;
    let mut returned = 0;
    // SAFETY: the caller holds the socket open, and what goes in and out is as big as it says.
    let failed = unsafe {
        WSAIoctl(
            raw as usize,
            SIO_GET_EXTENSION_FUNCTION_POINTER,
            std::ptr::from_ref(which).cast(),
            size_of::<Guid>() as u32,
            (&raw mut found).cast(),
            size_of::<usize>() as u32,
            &raw mut returned,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if failed != 0 {
        return Err(last_socket_error());
    }
    if found == 0 {
        return Err(io::Error::other("the system has no such function"));
    }
    Ok(found)
}

fn new_socket(v6: bool, kind: i32) -> io::Result<Arc<Handle>> {
    start_sockets();
    let protocol = if kind == SOCK_STREAM {
        IPPROTO_TCP
    } else {
        IPPROTO_UDP
    };
    let family = if v6 { AF_INET6 } else { AF_INET };
    // SAFETY: this only makes a socket, which the handle closes.
    let raw = unsafe {
        WSASocketW(
            i32::from(family),
            kind,
            protocol,
            std::ptr::null(),
            0,
            WSA_FLAG_OVERLAPPED | WSA_FLAG_NO_HANDLE_INHERIT,
        )
    };
    if raw == usize::MAX {
        return Err(last_socket_error());
    }
    Handle::socket(raw)
}

fn bind_to(raw: isize, address: SocketAddr) -> io::Result<()> {
    let (address, size) = RawAddress::of(address);
    // SAFETY: the caller holds the socket open, and the address is as long as it says.
    if unsafe { bind(raw as usize, (&raw const address.bytes).cast(), size) } != 0 {
        return Err(last_socket_error());
    }
    Ok(())
}

fn unspecified(v6: bool) -> SocketAddr {
    if v6 {
        SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), 0)
    } else {
        SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0)
    }
}

/// One end of something that carries bytes both ways: a TCP connection, a named pipe, or one of another program's streams.
pub(crate) struct Stream {
    handle: Arc<Handle>,
    tcp: bool,
    /// For another program's input and output, which end quietly rather than failing, as there's nobody to tell.
    child: bool,
    /// How many writes the thread has. Until there are none, a new one waits its turn behind them.
    writing: AtomicUsize,
}

impl Stream {
    fn new(handle: Arc<Handle>, tcp: bool, child: bool) -> Arc<Self> {
        Arc::new(Self {
            handle,
            tcp,
            child,
            writing: AtomicUsize::new(0),
        })
    }

    /// Stops what's going on with it, and closes it.
    pub(crate) fn close(&self) {
        if self.tcp {
            // SAFETY: the socket's held open through this.
            self.handle
                .with(|raw| unsafe { shutdown(raw as usize, SD_BOTH) });
        }
        self.handle.close();
    }
}

/// What's been heard from a stream.
enum Heard {
    Bytes(Vec<u8>),
    End,
    /// There's nothing yet.
    Nothing,
    Failed(io::Error),
}

thread_local! {
    /// What a socket's read into on this thread, before it's copied out at the length it turned out to be.
    static SPACE: RefCell<Vec<u8>> = RefCell::new(vec![0; CHUNK]);
}

/// Reads what's there already from a socket that doesn't wait.
fn read_now(raw: isize) -> Heard {
    SPACE.with_borrow_mut(|space| {
        // SAFETY: the caller holds the socket open, and the space is as long as it says.
        let got = unsafe { recv(raw as usize, space.as_mut_ptr().cast(), CHUNK as i32, 0) };
        match got {
            0 => Heard::End,
            length if length > 0 => Heard::Bytes(space[..length as usize].to_vec()),
            _ => {
                let error = last_socket_error();
                if error.raw_os_error() == Some(WSAEWOULDBLOCK) {
                    Heard::Nothing
                } else {
                    Heard::Failed(error)
                }
            }
        }
    })
}

impl Stream {
    fn heard(&self, heard: Heard) -> Outcome {
        match heard {
            Heard::Bytes(bytes) => Outcome::Bytes(bytes),
            Heard::End | Heard::Nothing => Outcome::Null,
            Heard::Failed(error) => self.read_failed(&error),
        }
    }

    fn read_failed(&self, error: &io::Error) -> Outcome {
        // Closing it from here looks like this, and so does the other end going away from a pipe.
        let ended = matches!(
            error.raw_os_error(),
            Some(
                ERROR_BROKEN_PIPE
                    | ERROR_NO_DATA
                    | ERROR_PIPE_NOT_CONNECTED
                    | iocp::ERROR_OPERATION_ABORTED
            )
        );
        if ended || self.child || self.handle.is_closed() {
            Outcome::Null
        } else {
            failed(error, "read", "")
        }
    }

    fn write_failed(&self, error: &io::Error) -> Outcome {
        if self.child {
            // One that's gone away isn't listening, which isn't for here to complain about.
            Outcome::Nothing
        } else if self.handle.is_closed() {
            Outcome::NetFailed {
                code: "EPIPE",
                message: "write EPIPE".to_owned(),
            }
        } else {
            failed(error, "write", "")
        }
    }
}

/// A read from a socket that's waiting for something to arrive. It's started with nothing to put it in, so it holds
/// nothing while it waits, and what's arrived is read once it's said to be there.
struct WaitingToRead {
    stream: Arc<Stream>,
    flags: u32,
    later: Later,
}

impl Work for WaitingToRead {
    unsafe fn start(op: *mut Op<Self>, raw: isize) -> io::Result<()> {
        let mut nothing = WsaBuf {
            len: 0,
            buf: std::ptr::null_mut(),
        };
        // SAFETY: as `Work::start` says, `op` is whole and `raw` is open. There's nowhere to put anything, and the system
        // copies what describes that before this returns.
        let started = unsafe {
            WSARecv(
                raw as usize,
                &raw mut nothing,
                1,
                std::ptr::null_mut(),
                &raw mut (*op).work.flags,
                Op::overlapped(op),
                std::ptr::null(),
            )
        };
        iocp::started_socket(started)
    }

    fn finish(op: Box<Op<Self>>, _: io::Result<u32>) {
        // Whatever it says, reading straight away says it too, and that's what to go by.
        match op.handle.with(read_now) {
            None => op.work.later.settle(Outcome::Null),
            Some(Heard::Nothing) => wait_to_read(op),
            Some(heard) => {
                let outcome = op.work.stream.heard(heard);
                op.work.later.settle(outcome);
            }
        }
    }
}

fn wait_to_read(op: Box<Op<WaitingToRead>>) {
    if let Err((op, error)) = iocp::start(op) {
        let outcome = op.work.stream.read_failed(&error);
        op.work.later.settle(outcome);
    }
}

/// A read from a pipe, which has a place to put what it reads from the start.
struct Reading {
    stream: Arc<Stream>,
    buffer: Vec<u8>,
    later: Later,
}

impl Work for Reading {
    unsafe fn start(op: *mut Op<Self>, raw: isize) -> io::Result<()> {
        // SAFETY: as `Work::start` says. The buffer's on the heap, owned by `op`, and as long as it says.
        let started = unsafe {
            let buffer = &raw mut (*op).work.buffer;
            ReadFile(
                raw,
                (*buffer).as_mut_ptr().cast(),
                (*buffer).len() as u32,
                std::ptr::null_mut(),
                Op::overlapped(op),
            )
        };
        iocp::started(started)
    }

    fn finish(op: Box<Op<Self>>, done: io::Result<u32>) {
        match done {
            // Something wrote nothing.
            Ok(0) => read_pipe(op),
            Ok(length) => {
                let bytes = op.work.buffer[..length as usize].to_vec();
                op.work.later.settle(Outcome::Bytes(bytes));
            }
            Err(error) => {
                let outcome = op.work.stream.read_failed(&error);
                op.work.later.settle(outcome);
            }
        }
    }
}

fn read_pipe(op: Box<Op<Reading>>) {
    if let Err((op, error)) = iocp::start(op) {
        let outcome = op.work.stream.read_failed(&error);
        op.work.later.settle(outcome);
    }
}

/// A promise for the next bytes from a stream, or `null` at the end.
pub(crate) fn read_stream(vm: &mut Vm, stream: &Arc<Stream>) -> Value {
    if stream.tcp {
        let heard = match stream.handle.with(read_now) {
            None => return vm.settled_promise(Outcome::Null),
            Some(Heard::Nothing) => None,
            Some(heard) => Some(stream.heard(heard)),
        };
        if let Some(outcome) = heard {
            return vm.settled_promise(outcome);
        }
    }
    // Nothing has to be going on for a socket to be waiting, so that doesn't count as something to wait for.
    let (promise, later) = vm.promise_for_later(!stream.child);
    if stream.tcp {
        wait_to_read(Op::new(
            stream.handle.clone(),
            WaitingToRead {
                stream: stream.clone(),
                flags: 0,
                later,
            },
        ));
    } else {
        read_pipe(Op::new(
            stream.handle.clone(),
            Reading {
                stream: stream.clone(),
                buffer: vec![0; CHUNK],
                later,
            },
        ));
    }
    promise
}

/// Writes what's left of some bytes, however long that takes.
struct Writing {
    stream: Arc<Stream>,
    bytes: Vec<u8>,
    done: usize,
    later: Later,
}

impl Work for Writing {
    unsafe fn start(op: *mut Op<Self>, raw: isize) -> io::Result<()> {
        // SAFETY: as `Work::start` says. The bytes are on the heap, owned by `op`, and `done` is never past their end. The
        // system copies what describes them before this returns.
        unsafe {
            let work = &raw mut (*op).work;
            let left = (*work).bytes.len() - (*work).done;
            let size = left.min(1 << 30) as u32;
            let from = (*work).bytes.as_mut_ptr().add((*work).done);
            let stream = &(*work).stream;
            if stream.tcp {
                let buffer = WsaBuf {
                    len: size,
                    buf: from,
                };
                let started = WSASend(
                    raw as usize,
                    &raw const buffer,
                    1,
                    std::ptr::null_mut(),
                    0,
                    Op::overlapped(op),
                    std::ptr::null(),
                );
                iocp::started_socket(started)
            } else {
                let started = WriteFile(
                    raw,
                    from.cast(),
                    size,
                    std::ptr::null_mut(),
                    Op::overlapped(op),
                );
                iocp::started(started)
            }
        }
    }

    fn finish(mut op: Box<Op<Self>>, done: io::Result<u32>) {
        let outcome = match done {
            Ok(wrote) => {
                op.work.done += wrote as usize;
                if wrote > 0 && op.work.done < op.work.bytes.len() {
                    write_rest(op);
                    return;
                }
                Outcome::Nothing
            }
            Err(error) => op.work.stream.write_failed(&error),
        };
        // What's next can go straight away now, as it's no longer behind this.
        op.work.stream.writing.fetch_sub(1, Ordering::AcqRel);
        op.work.later.settle(outcome);
    }
}

fn write_rest(op: Box<Op<Writing>>) {
    if let Err((op, error)) = iocp::start(op) {
        let outcome = op.work.stream.write_failed(&error);
        op.work.stream.writing.fetch_sub(1, Ordering::AcqRel);
        op.work.later.settle(outcome);
    }
}

/// Writes what a socket will take straight away, which can be none of it.
fn write_now(raw: isize, bytes: &[u8]) -> io::Result<usize> {
    let size = bytes.len().min(i32::MAX as usize) as i32;
    // SAFETY: the caller holds the socket open, and the bytes are as long as it says.
    let sent = unsafe { send(raw as usize, bytes.as_ptr().cast(), size, 0) };
    if sent >= 0 {
        return Ok(sent as usize);
    }
    let error = last_socket_error();
    if error.raw_os_error() == Some(WSAEWOULDBLOCK) {
        Ok(0)
    } else {
        Err(error)
    }
}

/// A promise for some bytes having gone to a stream.
pub(crate) fn write_stream(vm: &mut Vm, stream: &Arc<Stream>, bytes: Vec<u8>) -> Value {
    if bytes.is_empty() {
        return vm.settled_promise(Outcome::Nothing);
    }
    let mut done = 0;
    if stream.tcp && stream.writing.load(Ordering::Acquire) == 0 {
        match stream.handle.with(|raw| write_now(raw, &bytes)) {
            None => return vm.settled_promise(stream.write_failed(&aborted())),
            Some(Ok(wrote)) if wrote == bytes.len() => {
                return vm.settled_promise(Outcome::Nothing);
            }
            Some(Ok(wrote)) => done = wrote,
            Some(Err(error)) => return vm.settled_promise(stream.write_failed(&error)),
        }
    }
    let (promise, later) = vm.promise_for_later(false);
    stream.writing.fetch_add(1, Ordering::AcqRel);
    write_rest(Op::new(
        stream.handle.clone(),
        Writing {
            stream: stream.clone(),
            bytes,
            done,
            later,
        },
    ));
    promise
}

fn connect_pipe(path: &str) -> io::Result<Arc<Handle>> {
    let name = wide(path);
    for _ in 0..4 {
        // SAFETY: the name ends in a zero, and lasts through the calls.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                0,
            )
        };
        if handle != INVALID {
            return Handle::file(handle);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_PIPE_BUSY) {
            return Err(error);
        }
        // Every end of it is in use, and one may come free.
        // SAFETY: as above.
        unsafe {
            WaitNamedPipeW(name.as_ptr(), 1000);
        }
    }
    Err(io::Error::from_raw_os_error(ERROR_PIPE_BUSY))
}

fn new_pipe_end(path: &str, access: u32, first: bool) -> io::Result<Arc<Handle>> {
    let name = wide(path);
    let flags = FILE_FLAG_OVERLAPPED
        | if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    // SAFETY: the name ends in a zero, and lasts through the call. Only this machine can reach it.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            access | flags,
            PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            std::ptr::null(),
        )
    };
    if handle == INVALID {
        return Err(io::Error::last_os_error());
    }
    Handle::file(handle)
}

/// A pipe for another program's input or output: this end, and the end to give it, which doesn't overlap, as a program
/// expects of what it's given. `reading` is whether this end is the one that reads.
pub(crate) fn child_pipe(
    reading: bool,
) -> io::Result<(Arc<Stream>, std::os::windows::io::OwnedHandle)> {
    use std::os::windows::io::FromRawHandle;
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let path = format!(
        r"\\.\pipe\rsui-child-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let access = if reading {
        PIPE_ACCESS_INBOUND
    } else {
        PIPE_ACCESS_OUTBOUND
    };
    let ours = new_pipe_end(&path, access, true)?;
    let name = wide(&path);
    // SAFETY: the name ends in a zero, and lasts through the call.
    let theirs = unsafe {
        CreateFileW(
            name.as_ptr(),
            if reading { GENERIC_WRITE } else { GENERIC_READ },
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            0,
        )
    };
    if theirs == INVALID {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: it was just opened, and nothing else has it.
    let theirs =
        unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(theirs as *mut c_void) };
    Ok((Stream::new(ours, false, true), theirs))
}

enum Listening {
    Tcp(Arc<TcpListening>),
    Pipe(Arc<PipeListening>),
}

struct TcpListening {
    handle: Arc<Handle>,
    v6: bool,
    accept_ex: AcceptEx,
}

struct PipeListening {
    path: String,
    ends: Mutex<PipeEnds>,
}

#[derive(Default)]
struct PipeEnds {
    /// The first, made with the pipe, which hasn't been waited on yet.
    first: Option<Arc<Handle>>,
    /// The one that's waiting for something to connect to it.
    waiting: Option<Arc<Handle>>,
    closed: bool,
}

struct Datagrams {
    handle: Arc<Handle>,
    v6: bool,
}

#[derive(Default)]
struct Sockets {
    connections: HashMap<u32, Arc<Stream>>,
    listening: HashMap<u32, Listening>,
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
        Some(10061) => "ECONNREFUSED",
        Some(10060) => "ETIMEDOUT",
        Some(10054) => "ECONNRESET",
        Some(10053) => "ECONNABORTED",
        Some(10048) => "EADDRINUSE",
        Some(10049) => "EADDRNOTAVAIL",
        Some(10013) | Some(ERROR_ACCESS_DENIED) => "EACCES",
        Some(10051) => "ENETUNREACH",
        Some(10065) => "EHOSTUNREACH",
        Some(11001) | Some(11004) => "ENOTFOUND",
        Some(11002) => "EAI_AGAIN",
        Some(ERROR_FILE_NOT_FOUND) => "ENOENT",
        Some(ERROR_PIPE_BUSY) => "EBUSY",
        Some(ERROR_BROKEN_PIPE) | Some(ERROR_NO_DATA) | Some(ERROR_PIPE_NOT_CONNECTED) => "EPIPE",
        _ => "EIO",
    }
}

fn failed(error: &io::Error, doing: &str, target: &str) -> Outcome {
    let code = code_of(error);
    Outcome::NetFailed {
        code,
        message: format!("{doing} {code} {target}"),
    }
}

fn not_found(host: &str) -> Outcome {
    Outcome::NetFailed {
        code: "ENOTFOUND",
        message: format!("getaddrinfo ENOTFOUND {host}"),
    }
}

fn address_names(number: u32, here: Option<SocketAddr>, there: Option<SocketAddr>) -> Outcome {
    let part = |address: Option<SocketAddr>| match address {
        Some(address) => (address.ip().to_string(), address.port().to_string()),
        None => (String::new(), String::new()),
    };
    let (here_address, here_port) = part(here);
    let (there_address, there_port) = part(there);
    Outcome::Names(vec![
        number.to_string(),
        here_address,
        here_port,
        there_address,
        there_port,
    ])
}

fn number_arg(vm: &mut Vm, args: &Args, at: usize) -> Run<u32> {
    let value = vm.arg(args, at);
    Ok(vm.to_number(value)? as u32)
}

fn text_arg(vm: &mut Vm, args: &Args, at: usize) -> Run<String> {
    let value = vm.arg(args, at);
    vm.to_rust_string(value)
}

/// Looks a host up, and does something with where it is. That's done straight away when it's written as an address, and
/// otherwise on one of the threads for files, as looking a name up waits for the answer.
fn find_then(
    vm: &mut Vm,
    host: String,
    port: u16,
    then: impl FnOnce(io::Result<Vec<SocketAddr>>) + Send + 'static,
) {
    if let Ok(ip) = host.parse::<IpAddr>() {
        then(Ok(vec![SocketAddr::new(ip, port)]));
        return;
    }
    vm.in_pool(move || {
        then(
            (host.as_str(), port)
                .to_socket_addrs()
                .map(Iterator::collect),
        )
    });
}

/// Connecting to each address a host has in turn, until one answers.
struct Connecting {
    addresses: Vec<SocketAddr>,
    next: usize,
    target: String,
    to: RawAddress,
    to_size: i32,
    connect_ex: Option<ConnectEx>,
    last: Option<io::Error>,
    later: Later,
}

impl Work for Connecting {
    unsafe fn start(op: *mut Op<Self>, raw: isize) -> io::Result<()> {
        // SAFETY: as `Work::start` says. The address is owned by `op`, and as long as it says.
        unsafe {
            let work = &raw mut (*op).work;
            let Some(connect_ex) = (*work).connect_ex else {
                return Err(io::Error::other("there's no way to connect"));
            };
            let started = connect_ex(
                raw as usize,
                (&raw const (*work).to.bytes).cast(),
                (*work).to_size,
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                Op::overlapped(op),
            );
            iocp::started_socket_bool(started)
        }
    }

    fn finish(mut op: Box<Op<Self>>, done: io::Result<u32>) {
        if let Err(error) = done {
            op.handle.close();
            op.work.last = Some(error);
            connect_next(op.work);
            return;
        }
        let names = op.handle.with(|raw| {
            // SAFETY: the socket's held open through this. That says it's connected, which the other calls need to know.
            unsafe {
                setsockopt(
                    raw as usize,
                    SOL_SOCKET,
                    SO_UPDATE_CONNECT_CONTEXT,
                    std::ptr::null(),
                    0,
                );
            }
            dont_wait(raw);
            (name_of(raw, false), name_of(raw, true))
        });
        let Some((here, there)) = names else {
            op.work
                .later
                .settle(failed(&aborted(), "connect", &op.work.target));
            return;
        };
        let number = keep_connection(Stream::new(op.handle.clone(), true, false));
        op.work.later.settle(address_names(number, here, there));
    }
}

/// Starts connecting to the next address there is, and says what went wrong once there are none left.
fn connect_next(mut work: Connecting) {
    while let Some(&address) = work.addresses.get(work.next) {
        work.next += 1;
        let made = new_socket(address.is_ipv6(), SOCK_STREAM).and_then(|handle| {
            let found = handle.with(|raw| {
                bind_to(raw, unspecified(address.is_ipv6()))?;
                extension(raw, &WSAID_CONNECTEX)
            });
            match found {
                Some(Ok(pointer)) => Ok((handle, pointer)),
                Some(Err(error)) => Err(error),
                None => Err(aborted()),
            }
        });
        let (handle, pointer) = match made {
            Ok(made) => made,
            Err(error) => {
                work.last = Some(error);
                continue;
            }
        };
        // SAFETY: that's what the system gave for `ConnectEx`, which is called this way.
        work.connect_ex = Some(unsafe { std::mem::transmute::<usize, ConnectEx>(pointer) });
        (work.to, work.to_size) = RawAddress::of(address);
        match iocp::start(Op::new(handle, work)) {
            Ok(()) => return,
            Err((op, error)) => {
                op.handle.close();
                work = op.work;
                work.last = Some(error);
            }
        }
    }
    let outcome = match &work.last {
        Some(error) => failed(error, "connect", &work.target),
        None => not_found(&work.target),
    };
    work.later.settle(outcome);
}

/// `netConnect(host, port)`: a promise for its number, and the addresses at each end.
fn net_connect(vm: &mut Vm, args: Args) -> Run<Value> {
    let host = text_arg(vm, &args, 0)?;
    let port = number_arg(vm, &args, 1)? as u16;
    let (promise, later) = vm.promise_for_later(false);
    let target = format!("{host}:{port}");
    let named = host.clone();
    find_then(vm, host, port, move |found| match found {
        Ok(addresses) => connect_next(Connecting {
            addresses,
            next: 0,
            target,
            to: RawAddress::empty(),
            to_size: 0,
            connect_ex: None,
            last: None,
            later,
        }),
        Err(error) => later.settle(failed(&error, "getaddrinfo", &named)),
    });
    Ok(promise)
}

/// `pipeConnect(path)`: the same, for a named pipe. Waiting for one to be free is done on one of the threads for files.
fn pipe_connect(vm: &mut Vm, args: Args) -> Run<Value> {
    let path = text_arg(vm, &args, 0)?;
    Ok(vm.in_background(move || match connect_pipe(&path) {
        Ok(handle) => address_names(
            keep_connection(Stream::new(handle, false, false)),
            None,
            None,
        ),
        Err(error) => failed(&error, "connect", &path),
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
        None => vm.settled_promise(Outcome::NetFailed {
            code: "EPIPE",
            message: "write EPIPE".to_owned(),
        }),
    })
}

/// `netEnd(number)`: this end has nothing more to say, though it can still hear. A pipe doesn't have half of that, so it goes, which is
/// the only way the other end can find out that it's over.
fn net_end(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    match connection(number) {
        Some(stream) if stream.tcp => {
            // SAFETY: the socket's held open through this.
            stream
                .handle
                .with(|raw| unsafe { shutdown(raw as usize, SD_SEND) });
        }
        Some(stream) => {
            stream.close();
            lock(sockets()).connections.remove(&number);
        }
        None => {}
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
    let on = i32::from(vm.arg(&args, 1) != Value::FALSE);
    if let Some(stream) = connection(number).filter(|stream| stream.tcp) {
        stream.handle.with(|raw| {
            // SAFETY: the socket's held open through this, and the value is as long as it says.
            unsafe {
                setsockopt(
                    raw as usize,
                    IPPROTO_TCP,
                    TCP_NODELAY,
                    (&raw const on).cast(),
                    4,
                )
            }
        });
    }
    Ok(Value::UNDEFINED)
}

fn listen_tcp(addresses: Vec<SocketAddr>, target: &str) -> Outcome {
    let mut last = None;
    for address in addresses {
        let v6 = address.is_ipv6();
        let made = new_socket(v6, SOCK_STREAM).and_then(|handle| {
            let found = handle.with(|raw| {
                bind_to(raw, address)?;
                // SAFETY: the socket's held open through this.
                if unsafe { listen(raw as usize, BACKLOG) } != 0 {
                    return Err(last_socket_error());
                }
                let pointer = extension(raw, &WSAID_ACCEPTEX)?;
                Ok((pointer, name_of(raw, false)))
            });
            match found {
                Some(Ok((pointer, at))) => Ok((handle, pointer, at)),
                Some(Err(error)) => Err(error),
                None => Err(aborted()),
            }
        });
        match made {
            Ok((handle, pointer, at)) => {
                let number = next_number();
                let listening = TcpListening {
                    handle,
                    v6,
                    // SAFETY: that's what the system gave for `AcceptEx`, which is called this way.
                    accept_ex: unsafe { std::mem::transmute::<usize, AcceptEx>(pointer) },
                };
                lock(sockets())
                    .listening
                    .insert(number, Listening::Tcp(Arc::new(listening)));
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
    Ok(
        vm.in_background(move || match (host.as_str(), port).to_socket_addrs() {
            Ok(addresses) => listen_tcp(addresses.collect(), &target),
            Err(error) => failed(&error, "getaddrinfo", &host),
        }),
    )
}

/// `pipeListen(path)`.
fn pipe_listen(vm: &mut Vm, args: Args) -> Run<Value> {
    let path = text_arg(vm, &args, 0)?;
    let outcome = match new_pipe_end(&path, PIPE_ACCESS_DUPLEX, true) {
        Ok(first) => {
            let number = next_number();
            let listening = PipeListening {
                path,
                ends: Mutex::new(PipeEnds {
                    first: Some(first),
                    ..PipeEnds::default()
                }),
            };
            lock(sockets())
                .listening
                .insert(number, Listening::Pipe(Arc::new(listening)));
            address_names(number, None, None)
        }
        Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED) => Outcome::NetFailed {
            code: "EADDRINUSE",
            message: format!("listen EADDRINUSE {path}"),
        },
        Err(error) => failed(&error, "listen", &path),
    };
    Ok(vm.settled_promise(outcome))
}

/// Waiting for the next to connect to a TCP socket that's listening, with a socket made for it ahead of time.
struct Accepting {
    listening: Arc<TcpListening>,
    accepted: Arc<Handle>,
    addresses: [u8; 2 * ADDRESS_ROOM],
    received: u32,
    later: Later,
}

impl Work for Accepting {
    unsafe fn start(op: *mut Op<Self>, raw: isize) -> io::Result<()> {
        // SAFETY: as `Work::start` says. The socket it's accepted into, and the room for addresses, are owned by `op`, and
        // last until it's finished. Only `op` has that socket, so it can't be closed until `op` is finished with, and
        // everything that's needed of `op` is read before it's started.
        unsafe {
            let work = &raw mut (*op).work;
            let listening = &(*work).listening;
            let accept_ex = listening.accept_ex;
            let Some(accepted) = (*work).accepted.with(|accepted| accepted) else {
                return Err(aborted());
            };
            let addresses = (&raw mut (*work).addresses).cast();
            let received = &raw mut (*work).received;
            let started = accept_ex(
                raw as usize,
                accepted as usize,
                addresses,
                0,
                ADDRESS_ROOM as u32,
                ADDRESS_ROOM as u32,
                received,
                Op::overlapped(op),
            );
            iocp::started_socket_bool(started)
        }
    }

    fn finish(op: Box<Op<Self>>, done: io::Result<u32>) {
        let work = op.work;
        if let Err(error) = done {
            if work.listening.handle.is_closed() {
                work.later.settle(Outcome::Null);
            } else if matches!(error.raw_os_error(), Some(WSAECONNRESET | WSAECONNABORTED)) {
                // What connected went away before it was accepted, which is nothing to say anything about.
                accept_next(work.listening, work.later);
            } else {
                work.later.settle(failed(&error, "accept", ""));
            }
            return;
        }
        let names = work.listening.handle.with(|listening| {
            work.accepted.with(|accepted| {
                // SAFETY: both sockets are held open through this, and the value is as long as it says. That has the new
                // one take on what the one listening has, which the other calls need.
                unsafe {
                    setsockopt(
                        accepted as usize,
                        SOL_SOCKET,
                        SO_UPDATE_ACCEPT_CONTEXT,
                        (&raw const listening).cast(),
                        size_of::<usize>() as i32,
                    );
                }
                dont_wait(accepted);
                (name_of(accepted, false), name_of(accepted, true))
            })
        });
        match names.flatten() {
            Some((here, there)) => {
                let number = keep_connection(Stream::new(work.accepted, true, false));
                work.later.settle(address_names(number, here, there));
            }
            None => work.later.settle(Outcome::Null),
        }
    }
}

fn accept_next(listening: Arc<TcpListening>, later: Later) {
    let accepted = match new_socket(listening.v6, SOCK_STREAM) {
        Ok(accepted) => accepted,
        Err(error) => return later.settle(failed(&error, "accept", "")),
    };
    let op = Op::new(
        listening.handle.clone(),
        Accepting {
            listening,
            accepted,
            addresses: [0; 2 * ADDRESS_ROOM],
            received: 0,
            later,
        },
    );
    if let Err((op, error)) = iocp::start(op) {
        let work = op.work;
        if work.listening.handle.is_closed() {
            work.later.settle(Outcome::Null);
        } else {
            work.later.settle(failed(&error, "accept", ""));
        }
    }
}

/// Waiting for the next to connect to a pipe that's listening.
struct AcceptingPipe {
    listening: Arc<PipeListening>,
    later: Later,
}

impl Work for AcceptingPipe {
    unsafe fn start(op: *mut Op<Self>, raw: isize) -> io::Result<()> {
        // SAFETY: as `Work::start` says.
        let started = unsafe { ConnectNamedPipe(raw, Op::overlapped(op)) };
        iocp::started(started)
    }

    fn finish(op: Box<Op<Self>>, done: io::Result<u32>) {
        let closed = {
            let mut ends = lock(&op.work.listening.ends);
            ends.waiting = None;
            ends.closed
        };
        if closed {
            op.handle.close();
            op.work.later.settle(Outcome::Null);
            return;
        }
        match done {
            Ok(_) => pipe_accepted(op.handle, op.work.later),
            Err(error) => {
                op.handle.close();
                op.work
                    .later
                    .settle(failed(&error, "accept", &op.work.listening.path));
            }
        }
    }
}

fn pipe_accepted(end: Arc<Handle>, later: Later) {
    let number = keep_connection(Stream::new(end, false, false));
    later.settle(address_names(number, None, None));
}

fn accept_pipe(listening: Arc<PipeListening>, later: Later) {
    let mut ends = lock(&listening.ends);
    if ends.closed {
        drop(ends);
        return later.settle(Outcome::Null);
    }
    let end = match ends.first.take() {
        Some(end) => end,
        None => match new_pipe_end(&listening.path, PIPE_ACCESS_DUPLEX, false) {
            Ok(end) => end,
            Err(error) => {
                drop(ends);
                return later.settle(failed(&error, "accept", &listening.path));
            }
        },
    };
    // What stops this takes the same lock, so it either comes first and this sees it's closed, or comes after and finds
    // this one to close.
    ends.waiting = Some(end.clone());
    let op = Op::new(
        end.clone(),
        AcceptingPipe {
            listening: listening.clone(),
            later,
        },
    );
    if let Err((op, error)) = iocp::start(op) {
        ends.waiting = None;
        drop(ends);
        if error.raw_os_error() == Some(ERROR_PIPE_CONNECTED) {
            // It connected between being made and being waited on.
            pipe_accepted(end, op.work.later);
        } else {
            end.close();
            op.work
                .later
                .settle(failed(&error, "accept", &listening.path));
        }
    }
}

/// `netAccept(number)`: a promise for the next that connects, with its addresses, or `null` once it's stopped listening. It
/// doesn't count as something to wait for.
fn net_accept(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    let found = match lock(sockets()).listening.get(&number) {
        Some(Listening::Tcp(listening)) => Some(Listening::Tcp(listening.clone())),
        Some(Listening::Pipe(listening)) => Some(Listening::Pipe(listening.clone())),
        None => None,
    };
    let Some(found) = found else {
        return Ok(vm.settled_promise(Outcome::Null));
    };
    let (promise, later) = vm.promise_for_later(true);
    match found {
        Listening::Tcp(listening) => accept_next(listening, later),
        Listening::Pipe(listening) => accept_pipe(listening, later),
    }
    Ok(promise)
}

/// `netStopListening(number)`. What's waiting for a connection ends with `null`.
fn net_stop_listening(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    let found = lock(sockets()).listening.remove(&number);
    match found {
        Some(Listening::Tcp(listening)) => listening.handle.close(),
        Some(Listening::Pipe(listening)) => {
            let mut ends = lock(&listening.ends);
            ends.closed = true;
            if let Some(end) = ends.waiting.take() {
                end.close();
            }
            if let Some(end) = ends.first.take() {
                end.close();
            }
        }
        None => {}
    }
    Ok(Value::UNDEFINED)
}

/// `dnsLookup(host)`: a promise for its addresses.
fn dns_lookup(vm: &mut Vm, args: Args) -> Run<Value> {
    let host = text_arg(vm, &args, 0)?;
    Ok(
        vm.in_background(move || match (host.as_str(), 0).to_socket_addrs() {
            Ok(addresses) => {
                let mut names: Vec<String> = addresses.map(|one| one.ip().to_string()).collect();
                names.dedup();
                Outcome::Names(names)
            }
            Err(_) => not_found(&host),
        }),
    )
}

fn bind_udp(addresses: Vec<SocketAddr>, target: &str) -> Outcome {
    let mut last = None;
    for address in addresses {
        let v6 = address.is_ipv6();
        let made = new_socket(v6, SOCK_DGRAM).and_then(|handle| {
            let found = handle.with(|raw| {
                bind_to(raw, address)?;
                // Windows would otherwise end what's waiting to hear when something sent to had nothing listening.
                let mut off = 0u32;
                let mut returned = 0;
                // SAFETY: the socket's held open through this, and what goes in is as long as it says.
                unsafe {
                    WSAIoctl(
                        raw as usize,
                        SIO_UDP_CONNRESET,
                        (&raw mut off).cast(),
                        4,
                        std::ptr::null_mut(),
                        0,
                        &raw mut returned,
                        std::ptr::null_mut(),
                        std::ptr::null(),
                    );
                }
                dont_wait(raw);
                Ok(name_of(raw, false))
            });
            match found {
                Some(Ok(at)) => Ok((handle, at)),
                Some(Err(error)) => Err(error),
                None => Err(aborted()),
            }
        });
        match made {
            Ok((handle, at)) => {
                let number = next_number();
                lock(sockets())
                    .datagrams
                    .insert(number, Arc::new(Datagrams { handle, v6 }));
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
    Ok(
        vm.in_background(move || match (host.as_str(), port).to_socket_addrs() {
            Ok(addresses) => bind_udp(addresses.collect(), &target),
            Err(error) => failed(&error, "bind", &target),
        }),
    )
}

fn datagrams(number: u32) -> Option<Arc<Datagrams>> {
    lock(sockets()).datagrams.get(&number).cloned()
}

fn no_datagrams() -> Outcome {
    Outcome::NetFailed {
        code: "EBADF",
        message: "send EBADF".to_owned(),
    }
}

/// Sending one message, when the socket couldn't take it straight away.
struct Sending {
    bytes: Vec<u8>,
    to: RawAddress,
    to_size: i32,
    target: String,
    later: Later,
}

impl Work for Sending {
    unsafe fn start(op: *mut Op<Self>, raw: isize) -> io::Result<()> {
        // SAFETY: as `Work::start` says. The bytes and the address are owned by `op`, and as long as they say. The system
        // copies what describes the bytes before this returns.
        unsafe {
            let work = &raw mut (*op).work;
            let buffer = WsaBuf {
                len: (*work).bytes.len() as u32,
                buf: (*work).bytes.as_mut_ptr(),
            };
            let started = WSASendTo(
                raw as usize,
                &raw const buffer,
                1,
                std::ptr::null_mut(),
                0,
                (&raw const (*work).to.bytes).cast(),
                (*work).to_size,
                Op::overlapped(op),
                std::ptr::null(),
            );
            iocp::started_socket(started)
        }
    }

    fn finish(op: Box<Op<Self>>, done: io::Result<u32>) {
        let outcome = match done {
            Ok(_) => Outcome::Nothing,
            Err(_) if op.handle.is_closed() => no_datagrams(),
            Err(error) => failed(&error, "send", &op.work.target),
        };
        op.work.later.settle(outcome);
    }
}

fn send_datagram(
    socket: &Arc<Datagrams>,
    bytes: Vec<u8>,
    to: SocketAddr,
    target: String,
    later: Later,
) {
    let (address, size) = RawAddress::of(to);
    let length = bytes.len().min(i32::MAX as usize) as i32;
    let sent = socket.handle.with(|raw| {
        // SAFETY: the socket's held open through this, and the bytes and address are as long as they say.
        let sent = unsafe {
            sendto(
                raw as usize,
                bytes.as_ptr().cast(),
                length,
                0,
                (&raw const address.bytes).cast(),
                size,
            )
        };
        if sent >= 0 {
            Ok(())
        } else {
            Err(last_socket_error())
        }
    });
    match sent {
        None => later.settle(no_datagrams()),
        Some(Ok(())) => later.settle(Outcome::Nothing),
        Some(Err(error)) if error.raw_os_error() == Some(WSAEWOULDBLOCK) => {
            let op = Op::new(
                socket.handle.clone(),
                Sending {
                    bytes,
                    to: address,
                    to_size: size,
                    target,
                    later,
                },
            );
            if let Err((op, error)) = iocp::start(op) {
                let outcome = failed(&error, "send", &op.work.target);
                op.work.later.settle(outcome);
            }
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
    let host = if host == "localhost" {
        if socket.v6 { "::1" } else { "127.0.0.1" }.to_owned()
    } else {
        host
    };
    find_then(vm, host, port, move |found| {
        let addresses = match found {
            Ok(addresses) => addresses,
            Err(error) => return later.settle(failed(&error, "send", &target)),
        };
        let to = addresses
            .iter()
            .find(|address| address.is_ipv6() == socket.v6)
            .or(addresses.first());
        match to {
            Some(&to) => send_datagram(&socket, bytes, to, target, later),
            None => later.settle(not_found(&target)),
        }
    });
    Ok(promise)
}

/// What's come to a UDP socket.
enum Received {
    Message(Vec<u8>, SocketAddr),
    Nothing,
    Failed(io::Error),
}

fn receive_now(raw: isize) -> Received {
    SPACE.with_borrow_mut(|space| {
        loop {
            let mut from = RawAddress::empty();
            let mut from_size = from.bytes.len() as i32;
            // SAFETY: the caller holds the socket open, and the space and address are as long as they say.
            let got = unsafe {
                recvfrom(
                    raw as usize,
                    space.as_mut_ptr().cast(),
                    CHUNK as i32,
                    0,
                    (&raw mut from.bytes).cast(),
                    &raw mut from_size,
                )
            };
            if got >= 0 {
                let from = from
                    .address()
                    .unwrap_or_else(|| SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0));
                return Received::Message(space[..got as usize].to_vec(), from);
            }
            let error = last_socket_error();
            match error.raw_os_error() {
                Some(WSAEWOULDBLOCK) => return Received::Nothing,
                // Windows says this when what was sent to had nothing listening, and that doesn't stop this hearing from others.
                Some(WSAECONNRESET) => {}
                _ => return Received::Failed(error),
            }
        }
    })
}

fn received(received: Received) -> Outcome {
    match received {
        Received::Message(bytes, from) => Outcome::Datagram {
            bytes,
            address: from.ip().to_string(),
            port: from.port(),
        },
        Received::Nothing => Outcome::Null,
        Received::Failed(error) => failed(&error, "recv", ""),
    }
}

/// Waiting for a message, as a read from a socket does, by looking without taking it.
struct WaitingToReceive {
    flags: u32,
    later: Later,
}

impl Work for WaitingToReceive {
    unsafe fn start(op: *mut Op<Self>, raw: isize) -> io::Result<()> {
        let mut nothing = WsaBuf {
            len: 0,
            buf: std::ptr::null_mut(),
        };
        // SAFETY: as for `WaitingToRead`. Only looking at what's there leaves the message there to be received.
        let started = unsafe {
            (*op).work.flags = MSG_PEEK;
            WSARecv(
                raw as usize,
                &raw mut nothing,
                1,
                std::ptr::null_mut(),
                &raw mut (*op).work.flags,
                Op::overlapped(op),
                std::ptr::null(),
            )
        };
        iocp::started_socket(started)
    }

    fn finish(op: Box<Op<Self>>, _: io::Result<u32>) {
        match op.handle.with(receive_now) {
            None => op.work.later.settle(Outcome::Null),
            Some(Received::Nothing) => wait_to_receive(op),
            Some(got) => op.work.later.settle(received(got)),
        }
    }
}

fn wait_to_receive(op: Box<Op<WaitingToReceive>>) {
    if let Err((op, error)) = iocp::start(op) {
        let outcome = if op.handle.is_closed() {
            Outcome::Null
        } else {
            failed(&error, "recv", "")
        };
        op.work.later.settle(outcome);
    }
}

/// `udpReceive(number)`: a promise for what's next, or `null` once it's closed. It doesn't count as something to wait for.
fn udp_receive(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    let Some(socket) = datagrams(number) else {
        return Ok(vm.settled_promise(Outcome::Null));
    };
    match socket.handle.with(receive_now) {
        None => return Ok(vm.settled_promise(Outcome::Null)),
        Some(Received::Nothing) => {}
        Some(got) => return Ok(vm.settled_promise(received(got))),
    }
    let (promise, later) = vm.promise_for_later(true);
    wait_to_receive(Op::new(
        socket.handle.clone(),
        WaitingToReceive { flags: 0, later },
    ));
    Ok(promise)
}

/// `udpClose(number)`. What's waiting for a message ends with `null`.
fn udp_close(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = number_arg(vm, &args, 0)?;
    let found = lock(sockets()).datagrams.remove(&number);
    if let Some(socket) = found {
        socket.handle.close();
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
