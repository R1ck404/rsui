//! Network requests through WinHTTP, which comes with Windows and looks after TLS, proxies and redirects. It's only
//! loaded when something makes a request, so a program that doesn't has one library less, and no more code to speak of.
#![allow(unsafe_code)]

use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, SendError, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};

use crate::background::{Later, Native, Outcome, Response};
use crate::builtins::host::bytes_of;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

type Handle = *mut c_void;

const AUTOMATIC_PROXY: u32 = 4;
const SECURE: u32 = 0x0080_0000;
const QUERY_STATUS_CODE: u32 = 19;
const QUERY_RAW_HEADERS: u32 = 22;
const QUERY_NUMBER: u32 = 0x2000_0000;
const OPTION_DECOMPRESSION: u32 = 118;
const DECOMPRESS_ALL: u32 = 3;
/// How long each part can take, in milliseconds: finding the server, connecting, sending, and hearing back. Hearing back
/// can take as long as it does in Node, as a stream of events can go quiet for a while.
const TIMEOUTS: [i32; 4] = [10_000, 15_000, 30_000, 300_000];

struct Functions {
    open: unsafe extern "system" fn(*const u16, u32, *const u16, *const u16, u32) -> Handle,
    set_timeouts: unsafe extern "system" fn(Handle, i32, i32, i32, i32) -> i32,
    set_option: unsafe extern "system" fn(Handle, u32, *const c_void, u32) -> i32,
    connect: unsafe extern "system" fn(Handle, *const u16, u16, u32) -> Handle,
    open_request: unsafe extern "system" fn(
        Handle,
        *const u16,
        *const u16,
        *const u16,
        *const u16,
        *const *const u16,
        u32,
    ) -> Handle,
    send: unsafe extern "system" fn(Handle, *const u16, u32, *const c_void, u32, u32, usize) -> i32,
    receive: unsafe extern "system" fn(Handle, *mut c_void) -> i32,
    query:
        unsafe extern "system" fn(Handle, u32, *const u16, *mut c_void, *mut u32, *mut u32) -> i32,
    read: unsafe extern "system" fn(Handle, *mut c_void, u32, *mut u32) -> i32,
    query_available: unsafe extern "system" fn(Handle, *mut u32) -> i32,
    query_option: unsafe extern "system" fn(Handle, u32, *mut c_void, *mut u32) -> i32,
    set_callback: unsafe extern "system" fn(Handle, Option<StatusCallback>, u32, usize) -> isize,
    upgrade: unsafe extern "system" fn(Handle, usize) -> Handle,
    socket_send: unsafe extern "system" fn(Handle, i32, *const c_void, u32) -> u32,
    socket_receive: unsafe extern "system" fn(Handle, *mut c_void, u32, *mut u32, *mut i32) -> u32,
    socket_shutdown: unsafe extern "system" fn(Handle, u16, *const c_void, u32) -> u32,
    socket_status: unsafe extern "system" fn(Handle, *mut u16, *mut c_void, u32, *mut u32) -> u32,
    close: unsafe extern "system" fn(Handle) -> i32,
}

// SAFETY: these are only addresses of functions that can be called from any thread.
unsafe impl Send for Functions {}
// SAFETY: as above.
unsafe impl Sync for Functions {}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetLastError() -> u32;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn functions() -> Option<&'static Functions> {
    static FOUND: OnceLock<Option<Functions>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            // SAFETY: the library stays loaded for good, and each of these has the type it's being given.
            #[allow(clippy::missing_transmute_annotations)]
            unsafe {
                let module = LoadLibraryW(wide("winhttp.dll").as_ptr());
                if module.is_null() {
                    return None;
                }
                macro_rules! find {
                    ($name:literal) => {{
                        let found = GetProcAddress(module, concat!($name, "\0").as_ptr());
                        if found.is_null() {
                            return None;
                        }
                        std::mem::transmute(found)
                    }};
                }
                Some(Functions {
                    open: find!("WinHttpOpen"),
                    set_timeouts: find!("WinHttpSetTimeouts"),
                    set_option: find!("WinHttpSetOption"),
                    connect: find!("WinHttpConnect"),
                    open_request: find!("WinHttpOpenRequest"),
                    send: find!("WinHttpSendRequest"),
                    receive: find!("WinHttpReceiveResponse"),
                    query: find!("WinHttpQueryHeaders"),
                    read: find!("WinHttpReadData"),
                    query_available: find!("WinHttpQueryDataAvailable"),
                    query_option: find!("WinHttpQueryOption"),
                    set_callback: find!("WinHttpSetStatusCallback"),
                    upgrade: find!("WinHttpWebSocketCompleteUpgrade"),
                    socket_send: find!("WinHttpWebSocketSend"),
                    socket_receive: find!("WinHttpWebSocketReceive"),
                    socket_shutdown: find!("WinHttpWebSocketShutdown"),
                    socket_status: find!("WinHttpWebSocketQueryCloseStatus"),
                    close: find!("WinHttpCloseHandle"),
                })
            }
        })
        .as_ref()
}

/// Closes what it's given, however things end.
struct Closing(Handle, &'static Functions);

impl Drop for Closing {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this came from WinHTTP, and is only closed here.
            unsafe {
                (self.1.close)(self.0);
            }
        }
    }
}

/// The parts of a URL. Only `http` and `https` get this far.
#[derive(Debug, PartialEq)]
struct Parts<'a> {
    secure: bool,
    host: &'a str,
    port: u16,
    path: &'a str,
}

fn parse_url(url: &str) -> Option<Parts<'_>> {
    let (secure, rest) = if let Some(rest) = strip_scheme(url, "https://") {
        (true, rest)
    } else {
        (false, strip_scheme(url, "http://")?)
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    // What's after a `#` is for whoever has the page, and never goes to the server.
    let path = path.split('#').next().unwrap_or("");
    // Names and passwords in URLs aren't something to pass along by accident.
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.rfind(':') {
        Some(colon) if !authority.ends_with(']') => {
            (&authority[..colon], authority[colon + 1..].parse().ok()?)
        }
        _ => (authority, if secure { 443 } else { 80 }),
    };
    Some(Parts {
        secure,
        host,
        port,
        path: if path.is_empty() { "/" } else { path },
    })
}

fn strip_scheme<'a>(url: &'a str, scheme: &str) -> Option<&'a str> {
    let start = url.get(..scheme.len())?;
    start
        .eq_ignore_ascii_case(scheme)
        .then(|| &url[scheme.len()..])
}

/// A problem, with Windows's number for it.
fn last_failure(doing: &str) -> Outcome {
    // SAFETY: this has no preconditions.
    let code = unsafe { GetLastError() };
    Outcome::RequestFailed(format!("{doing} (error {code})"))
}

// A request runs on a thread of its own, as it's waiting on the network, but its answer comes a part at a time, each only
// when JavaScript asks for it, so nothing is read ahead that hasn't been asked for. WinHTTP works asynchronously here,
// saying what's happened from its own threads, so that the request's thread can wait for that and for what JavaScript
// asks, both at once, and so that a request can be stopped partway by closing its handle, which WinHTTP only allows then.

const ASYNC: u32 = 0x1000_0000;
const OPTION_CONTEXT_VALUE: u32 = 45;
const OPTION_URL: u32 = 34;
const STATUS_SENT: u32 = 0x0040_0000;
const STATUS_HEADERS: u32 = 0x0002_0000;
const STATUS_AVAILABLE: u32 = 0x0004_0000;
const STATUS_READ: u32 = 0x0008_0000;
const STATUS_FAILED: u32 = 0x0020_0000;
const STATUS_CLOSING: u32 = 0x0000_0800;
/// Everything that finishes, and handles coming and going.
const STATUS_WANTED: u32 = 0x007E_0000 | 0x0000_0C00;
const ERROR_CANCELLED: u32 = 12017;
const ERROR_TIMED_OUT: u32 = 12002;
/// The most that's read at once.
const PART: u32 = 64 * 1024;
const REQUEST_STACK: usize = 256 * 1024;

type StatusCallback = unsafe extern "system" fn(Handle, usize, u32, *mut c_void, u32);

/// What WinHTTP gives with a failure.
#[repr(C)]
struct AsyncResult {
    result: usize,
    error: u32,
}

enum Event {
    /// JavaScript wants the next part of the answer.
    Read(Later),
    /// JavaScript doesn't want any more of it.
    Cancel,
    /// WinHTTP has finished what it was last asked to do: sending, getting the headers, finding how much can be read
    /// (which this says), or reading (which this says too).
    Done(u32),
    Failed(u32),
    /// WinHTTP has let go of the request's handle, and won't say anything more about it.
    Closing,
}

/// What WinHTTP is given as the request's context, to call back with. It holds a count of this until it says it's closing.
struct Hearing {
    events: Sender<Event>,
}

/// Where WinHTTP says what's happened, on one of its threads.
unsafe extern "system" fn heard(
    _: Handle,
    context: usize,
    status: u32,
    info: *mut c_void,
    length: u32,
) {
    if context == 0 {
        return;
    }
    // SAFETY: a context that isn't 0 is only ever the request's `Hearing`, which lasts until WinHTTP says it's closing,
    // and each kind of news comes with what WinHTTP's documentation says it does.
    unsafe {
        let hearing = &*(context as *const Hearing);
        let event = match status {
            STATUS_SENT | STATUS_HEADERS => Event::Done(0),
            STATUS_AVAILABLE => Event::Done(*(info as *const u32)),
            STATUS_READ => Event::Done(length),
            STATUS_FAILED => Event::Failed((*(info as *const AsyncResult)).error),
            STATUS_CLOSING => Event::Closing,
            _ => return,
        };
        let closing = matches!(event, Event::Closing);
        let _ = hearing.events.send(event);
        if closing {
            drop(Arc::from_raw(context as *const Hearing));
        }
    }
}

/// Why a request stopped before it finished.
enum Stopped {
    Failed(u32),
    Cancelled,
}

impl Stopped {
    fn outcome(&self) -> Outcome {
        Outcome::RequestFailed(match self {
            Self::Failed(ERROR_TIMED_OUT) => "it took too long".to_owned(),
            Self::Failed(ERROR_CANCELLED) | Self::Cancelled => "it was stopped".to_owned(),
            Self::Failed(code) => format!("error {code}"),
        })
    }
}

/// The request's side of what it hears.
struct Exchange {
    events: Receiver<Event>,
    /// Reads that were asked for while something else was going on.
    asked: VecDeque<Later>,
}

impl Exchange {
    /// Waits for what WinHTTP was last asked to do, keeping what JavaScript asks for meanwhile.
    fn wait(&mut self) -> Result<u32, Stopped> {
        loop {
            match self.events.recv() {
                Ok(Event::Done(value)) => return Ok(value),
                Ok(Event::Read(later)) => self.asked.push_back(later),
                Ok(Event::Failed(code)) => return Err(Stopped::Failed(code)),
                Ok(Event::Cancel | Event::Closing) | Err(_) => return Err(Stopped::Cancelled),
            }
        }
    }

    /// Waits for JavaScript to ask for the next part.
    fn next_read(&mut self) -> Result<Later, Stopped> {
        if let Some(later) = self.asked.pop_front() {
            return Ok(later);
        }
        loop {
            match self.events.recv() {
                Ok(Event::Read(later)) => return Ok(later),
                Ok(Event::Failed(code)) => return Err(Stopped::Failed(code)),
                Ok(Event::Done(_)) => {}
                Ok(Event::Cancel | Event::Closing) | Err(_) => return Err(Stopped::Cancelled),
            }
        }
    }
}

fn exchanges() -> &'static Mutex<HashMap<u32, Sender<Event>>> {
    static ALL: OnceLock<Mutex<HashMap<u32, Sender<Event>>>> = OnceLock::new();
    ALL.get_or_init(Mutex::default)
}

/// What's asked for, which goes to the request's thread.
struct Asked {
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// The raw headers of what came back.
///
/// # Safety
///
/// `handle` must be a request whose headers are in.
unsafe fn raw_headers(api: &Functions, handle: Handle) -> String {
    // SAFETY: what's written to is as big as these calls are told it is. The first says how much room the headers want,
    // by not having enough.
    unsafe {
        let mut size = 0u32;
        (api.query)(
            handle,
            QUERY_RAW_HEADERS,
            std::ptr::null(),
            std::ptr::null_mut(),
            &raw mut size,
            std::ptr::null_mut(),
        );
        let mut raw = vec![0u16; size as usize / 2 + 1];
        let got = (api.query)(
            handle,
            QUERY_RAW_HEADERS,
            std::ptr::null(),
            raw.as_mut_ptr().cast(),
            &raw mut size,
            std::ptr::null_mut(),
        );
        if got != 0 {
            String::from_utf16_lossy(&raw[..size as usize / 2])
        } else {
            String::new()
        }
    }
}

/// The status of a request whose headers are in.
///
/// # Safety
///
/// As for [`raw_headers`].
unsafe fn status_of(api: &Functions, handle: Handle) -> u32 {
    let mut status = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: there's room for the number that's asked for.
    unsafe {
        (api.query)(
            handle,
            QUERY_STATUS_CODE | QUERY_NUMBER,
            std::ptr::null(),
            (&raw mut status).cast(),
            &raw mut size,
            std::ptr::null_mut(),
        );
    }
    status
}

/// Where a request ended up, after any redirects.
///
/// # Safety
///
/// As for [`raw_headers`].
unsafe fn final_url(api: &Functions, handle: Handle) -> String {
    // SAFETY: as in `raw_headers`, the first call says how much room it wants.
    unsafe {
        let mut size = 0u32;
        (api.query_option)(handle, OPTION_URL, std::ptr::null_mut(), &raw mut size);
        let mut url = vec![0u16; size as usize / 2 + 1];
        if (api.query_option)(handle, OPTION_URL, url.as_mut_ptr().cast(), &raw mut size) == 0 {
            return String::new();
        }
        String::from_utf16_lossy(&url[..size as usize / 2])
    }
}

/// Makes a request, says what came back once the headers are in, and then reads its answer a part at a time, each when
/// it's asked for, until it ends, fails, or is cancelled.
fn exchange(asked: Asked, head: Later, events: Receiver<Event>, hearing: Arc<Hearing>) {
    let mut head = Some(head);
    let mut exchange = Exchange {
        events,
        asked: VecDeque::new(),
    };
    let fail = |head: &mut Option<Later>, outcome: Outcome| {
        if let Some(head) = head.take() {
            head.settle(outcome);
        }
    };
    let Some(parts) = parse_url(&asked.url) else {
        let url = &asked.url;
        return fail(
            &mut head,
            Outcome::RequestFailed(format!("{url} isn't an http or https URL")),
        );
    };
    let Some(api) = functions() else {
        return fail(
            &mut head,
            Outcome::RequestFailed("WinHTTP isn't there".to_owned()),
        );
    };
    let Ok(body_length) = u32::try_from(asked.body.len()) else {
        return fail(
            &mut head,
            Outcome::RequestFailed("that's too much to send".to_owned()),
        );
    };
    let header_text: String = asked
        .headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect();
    let header_text = wide(&header_text);
    let mut buffer: Vec<u8> = Vec::new();
    // SAFETY: every string ends in a zero and outlives the calls using it, and each handle is checked before it's used and
    // closed once. What WinHTTP is still reading from or writing to, the headers, the body and the buffer, lasts until it
    // says it's let go of the request, and if it never does, they're never freed.
    unsafe {
        let session = (api.open)(
            wide("rsui").as_ptr(),
            AUTOMATIC_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            ASYNC,
        );
        if session.is_null() {
            return fail(&mut head, last_failure("couldn't start"));
        }
        let _session = Closing(session, api);
        if (api.set_callback)(session, Some(heard), STATUS_WANTED, 0) == -1 {
            return fail(&mut head, last_failure("couldn't start"));
        }
        // Finding the server can't have a time limit when this is asynchronous.
        (api.set_timeouts)(session, 0, TIMEOUTS[1], TIMEOUTS[2], TIMEOUTS[3]);
        let decompress = DECOMPRESS_ALL;
        (api.set_option)(
            session,
            OPTION_DECOMPRESSION,
            (&raw const decompress).cast(),
            size_of::<u32>() as u32,
        );
        let connection = (api.connect)(session, wide(parts.host).as_ptr(), parts.port, 0);
        if connection.is_null() {
            return fail(&mut head, last_failure("couldn't connect"));
        }
        let _connection = Closing(connection, api);
        let handle = (api.open_request)(
            connection,
            wide(&asked.method).as_ptr(),
            wide(parts.path).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            if parts.secure { SECURE } else { 0 },
        );
        if handle.is_null() {
            return fail(&mut head, last_failure("couldn't make the request"));
        }
        let context = Arc::into_raw(hearing) as usize;
        if (api.set_option)(
            handle,
            OPTION_CONTEXT_VALUE,
            (&raw const context).cast(),
            size_of::<usize>() as u32,
        ) == 0
        {
            let failure = last_failure("couldn't make the request");
            (api.close)(handle);
            drop(Arc::from_raw(context as *const Hearing));
            return fail(&mut head, failure);
        }
        let ended = (|| -> Result<(), Stopped> {
            let sent = (api.send)(
                handle,
                if asked.headers.is_empty() {
                    std::ptr::null()
                } else {
                    header_text.as_ptr()
                },
                if asked.headers.is_empty() {
                    0
                } else {
                    u32::MAX
                },
                if asked.body.is_empty() {
                    std::ptr::null()
                } else {
                    asked.body.as_ptr().cast()
                },
                body_length,
                body_length,
                context,
            );
            if sent == 0 {
                return Err(Stopped::Failed(GetLastError()));
            }
            exchange.wait()?;
            if (api.receive)(handle, std::ptr::null_mut()) == 0 {
                return Err(Stopped::Failed(GetLastError()));
            }
            exchange.wait()?;
            let (status_text, headers) = read_headers(&raw_headers(api, handle));
            if let Some(head) = head.take() {
                head.settle(Outcome::Response(Box::new(Response {
                    status: status_of(api, handle),
                    status_text,
                    headers,
                    url: final_url(api, handle),
                    body: Vec::new(),
                })));
            }
            loop {
                let later = exchange.next_read()?;
                let mut read = || {
                    if (api.query_available)(handle, std::ptr::null_mut()) == 0 {
                        return Err(Stopped::Failed(GetLastError()));
                    }
                    let available = exchange.wait()?;
                    if available == 0 {
                        return Ok(0);
                    }
                    buffer.resize(available.min(PART) as usize, 0);
                    let asked_for = buffer.len() as u32;
                    if (api.read)(
                        handle,
                        buffer.as_mut_ptr().cast(),
                        asked_for,
                        std::ptr::null_mut(),
                    ) == 0
                    {
                        return Err(Stopped::Failed(GetLastError()));
                    }
                    exchange.wait()
                };
                match read() {
                    Ok(0) => {
                        later.settle(Outcome::Null);
                        return Ok(());
                    }
                    Ok(got) => later.settle(Outcome::Bytes(buffer[..got as usize].to_vec())),
                    Err(stopped) => {
                        later.settle(stopped.outcome());
                        return Err(stopped);
                    }
                }
            }
        })();
        if let Err(stopped) = &ended {
            fail(&mut head, stopped.outcome());
        }
        (api.close)(handle);
        // Nothing WinHTTP has been given can be let go of until it says it's done with it.
        loop {
            match exchange
                .events
                .recv_timeout(std::time::Duration::from_secs(30))
            {
                Ok(Event::Closing) => break,
                Ok(Event::Read(later)) => exchange.asked.push_back(later),
                Ok(_) => {}
                Err(_) => {
                    std::mem::forget(buffer);
                    std::mem::forget(asked);
                    std::mem::forget(header_text);
                    break;
                }
            }
        }
        // What's asked for after the end is told it's the end, and after a failure, that it failed.
        for later in exchange.asked.drain(..) {
            match &ended {
                Ok(()) => later.settle(Outcome::Null),
                Err(stopped) => later.settle(stopped.outcome()),
            }
        }
    }
}

/// The pairs `[[name, value]...]` that JavaScript gives for headers.
fn header_pairs(vm: &mut Vm, given: Value) -> Run<Vec<(String, String)>> {
    let mut headers = Vec::new();
    if given.is_object() {
        let length = vm.get(given, crate::text::LENGTH)?;
        let length = vm.to_number(length)? as usize;
        for index in 0..length {
            let pair = vm.get_index(given, index)?;
            let name = vm.get_index(pair, 0)?;
            let value = vm.get_index(pair, 1)?;
            headers.push((vm.to_rust_string(name)?, vm.to_rust_string(value)?));
        }
    }
    Ok(headers)
}

/// `httpStart(url, method, [[name, value]...], body)`, which is what `fetch` and Node's `http` come down to: the
/// request's number, and a promise of its status and headers.
fn http_start(vm: &mut Vm, args: Args) -> Run<Value> {
    let url = vm.arg(&args, 0);
    let url = vm.to_rust_string(url)?;
    let method = vm.arg(&args, 1);
    let method = vm.to_rust_string(method)?;
    let given = vm.arg(&args, 2);
    let headers = header_pairs(vm, given)?;
    let given = vm.arg(&args, 3);
    let body = if given.is_string() {
        vm.to_rust_string(given)?.into_bytes()
    } else if given.is_undefined() {
        Vec::new()
    } else {
        bytes_of(vm, given)?
    };
    let asked = Asked {
        url,
        method,
        headers,
        body,
    };
    let (promise, head) = vm.promise_for_later(false);
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let (sender, events) = channel();
    if let Ok(mut all) = exchanges().lock() {
        all.insert(number, sender.clone());
    }
    let hearing = Arc::new(Hearing { events: sender });
    let started = std::thread::Builder::new()
        .name("rsui request".into())
        .stack_size(REQUEST_STACK)
        .spawn(move || {
            exchange(asked, head, events, hearing);
            if let Ok(mut all) = exchanges().lock() {
                all.remove(&number);
            }
        });
    if started.is_err() {
        if let Ok(mut all) = exchanges().lock() {
            all.remove(&number);
        }
    }
    Ok(vm.new_array(vec![Value::number(f64::from(number)), promise]))
}

/// `httpRead(number)`: a promise of the next part of what came back, as bytes, or `null` once that's all.
fn http_read(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = vm.arg(&args, 0);
    let number = vm.to_number(number)? as u32;
    // This only waits to hear something, as a stream of events can stay open for good.
    let (promise, later) = vm.promise_for_later(true);
    let sender = exchanges()
        .lock()
        .ok()
        .and_then(|all| all.get(&number).cloned());
    match sender {
        Some(sender) => {
            if let Err(SendError(Event::Read(later))) = sender.send(Event::Read(later)) {
                later.settle(Outcome::Null);
            }
        }
        None => later.settle(Outcome::Null),
    }
    Ok(promise)
}

/// `httpCancel(number)`: stops the request wherever it's got to, and lets go of it.
fn http_cancel(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = vm.arg(&args, 0);
    let number = vm.to_number(number)? as u32;
    let sender = exchanges()
        .lock()
        .ok()
        .and_then(|mut all| all.remove(&number));
    if let Some(sender) = sender {
        let _ = sender.send(Event::Cancel);
    }
    Ok(Value::UNDEFINED)
}

const OPTION_UPGRADE_TO_WEB_SOCKET: u32 = 114;
const STATUS_SWITCHING_PROTOCOLS: u32 = 101;
const BINARY_MESSAGE: i32 = 0;
const BINARY_FRAGMENT: i32 = 1;
const UTF8_MESSAGE: i32 = 2;
const UTF8_FRAGMENT: i32 = 3;
const CLOSE_MESSAGE: i32 = 4;

/// A web socket that's open, with what it came from.
struct WebSocket {
    session: Handle,
    connection: Handle,
    socket: Handle,
}

// SAFETY: these are only handles, and WinHTTP allows one send and one receive at a time from any thread, which is how they're used.
unsafe impl Send for WebSocket {}
// SAFETY: as above.
unsafe impl Sync for WebSocket {}

impl Drop for WebSocket {
    fn drop(&mut self) {
        if let Some(api) = functions() {
            // SAFETY: each is only closed here, and nothing's using them once the last of this has gone.
            unsafe {
                (api.close)(self.socket);
                (api.close)(self.connection);
                (api.close)(self.session);
            }
        }
    }
}

fn web_sockets() -> &'static Mutex<HashMap<u32, Arc<WebSocket>>> {
    static ALL: OnceLock<Mutex<HashMap<u32, Arc<WebSocket>>>> = OnceLock::new();
    ALL.get_or_init(Mutex::default)
}

fn web_socket(number: u32) -> Option<Arc<WebSocket>> {
    web_sockets().lock().ok()?.get(&number).cloned()
}

/// `ws://` as what it's asked for by, which starts as an ordinary request.
fn web_socket_url(url: &str) -> Option<String> {
    if let Some(rest) = strip_scheme(url, "wss://") {
        Some(format!("https://{rest}"))
    } else {
        strip_scheme(url, "ws://").map(|rest| format!("http://{rest}"))
    }
}

fn socket_failed(message: String) -> Outcome {
    Outcome::NetFailed {
        code: "ECONNREFUSED",
        message,
    }
}

fn connect_web_socket(url: &str, protocols: &str) -> Outcome {
    let Some(asked) = web_socket_url(url) else {
        return socket_failed(format!("{url} isn't a ws or wss URL"));
    };
    let Some(parts) = parse_url(&asked) else {
        return socket_failed(format!("{url} isn't a ws or wss URL"));
    };
    let Some(api) = functions() else {
        return socket_failed("WinHTTP isn't there".to_owned());
    };
    let headers = if protocols.is_empty() {
        String::new()
    } else {
        format!("Sec-WebSocket-Protocol: {protocols}\r\n")
    };
    let headers = wide(&headers);
    // SAFETY: every string ends in a zero and outlives the calls using it, and each handle is checked before it's used. What goes on
    // to be kept is closed when that's let go of, and everything else is closed here.
    unsafe {
        let session = (api.open)(
            wide("rsui").as_ptr(),
            AUTOMATIC_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        );
        if session.is_null() {
            return socket_failed("WinHTTP couldn't start".to_owned());
        }
        let connection = (api.connect)(session, wide(parts.host).as_ptr(), parts.port, 0);
        if connection.is_null() {
            (api.close)(session);
            return socket_failed(format!("couldn't connect to {url}"));
        }
        let close_both = || {
            (api.close)(connection);
            (api.close)(session);
        };
        let handle = (api.open_request)(
            connection,
            wide("GET").as_ptr(),
            wide(parts.path).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            if parts.secure { SECURE } else { 0 },
        );
        if handle.is_null() {
            close_both();
            return socket_failed(format!("couldn't connect to {url}"));
        }
        let _request = Closing(handle, api);
        (api.set_option)(handle, OPTION_UPGRADE_TO_WEB_SOCKET, std::ptr::null(), 0);
        let has_headers = !protocols.is_empty();
        let sent = (api.send)(
            handle,
            if has_headers {
                headers.as_ptr()
            } else {
                std::ptr::null()
            },
            if has_headers { u32::MAX } else { 0 },
            std::ptr::null(),
            0,
            0,
            0,
        );
        if sent == 0 || (api.receive)(handle, std::ptr::null_mut()) == 0 {
            let code = GetLastError();
            close_both();
            return socket_failed(format!("couldn't connect to {url} (error {code})"));
        }
        let status = status_of(api, handle);
        if status != STATUS_SWITCHING_PROTOCOLS {
            close_both();
            return socket_failed(format!(
                "{url} answered with {status} rather than a web socket"
            ));
        }
        let (_, answered) = read_headers(&raw_headers(api, handle));
        let protocol = answered
            .into_iter()
            .find(|(name, _)| name == "sec-websocket-protocol")
            .map_or(String::new(), |(_, value)| value);
        let socket = (api.upgrade)(handle, 0);
        if socket.is_null() {
            close_both();
            return socket_failed(format!("couldn't connect to {url}"));
        }
        static NEXT: AtomicU32 = AtomicU32::new(1);
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut all) = web_sockets().lock() {
            all.insert(
                number,
                Arc::new(WebSocket {
                    session,
                    connection,
                    socket,
                }),
            );
        }
        Outcome::Names(vec![number.to_string(), protocol])
    }
}

/// `wsConnect(url, protocols)`: a promise for its number, and the protocol the server chose.
fn ws_connect(vm: &mut Vm, args: Args) -> Run<Value> {
    let url = vm.arg(&args, 0);
    let url = vm.to_rust_string(url)?;
    let protocols = vm.arg(&args, 1);
    let protocols = vm.to_rust_string(protocols)?;
    Ok(vm.in_own_thread(move || connect_web_socket(&url, &protocols)))
}

/// `wsSend(number, text or bytes)`: a promise for it having gone.
fn ws_send(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = vm.arg(&args, 0);
    let number = vm.to_number(number)? as u32;
    let given = vm.arg(&args, 1);
    let (bytes, kind) = if given.is_string() {
        (vm.to_rust_string(given)?.into_bytes(), UTF8_MESSAGE)
    } else {
        (bytes_of(vm, given)?, BINARY_MESSAGE)
    };
    Ok(vm.in_own_thread(move || {
        let (Some(found), Some(api)) = (web_socket(number), functions()) else {
            return Outcome::Nothing;
        };
        let Ok(length) = u32::try_from(bytes.len()) else {
            return Outcome::Failed("that's too much to send".to_owned());
        };
        // SAFETY: the socket lasts as long as `found`, and the bytes are as long as it's told.
        let failed =
            unsafe { (api.socket_send)(found.socket, kind, bytes.as_ptr().cast(), length) };
        if failed == 0 {
            Outcome::Nothing
        } else {
            Outcome::NetFailed {
                code: "EPIPE",
                message: format!("the web socket couldn't send that (error {failed})"),
            }
        }
    }))
}

/// `wsReceive(number)`: a promise for the next message, as text or bytes, or for `["close", code, reason]` once it's over.
fn ws_receive(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = vm.arg(&args, 0);
    let number = vm.to_number(number)? as u32;
    Ok(vm.in_own_thread_quietly(move || {
        let lost = || Outcome::Names(vec!["close".to_owned(), "1006".to_owned(), String::new()]);
        let (Some(found), Some(api)) = (web_socket(number), functions()) else {
            return lost();
        };
        let mut message = Vec::new();
        let mut chunk = vec![0u8; 16 * 1024];
        loop {
            let (mut read, mut kind) = (0u32, 0i32);
            // SAFETY: the socket lasts as long as `found`, and there's as much room as it's told.
            let failed = unsafe {
                (api.socket_receive)(
                    found.socket,
                    chunk.as_mut_ptr().cast(),
                    chunk.len() as u32,
                    &raw mut read,
                    &raw mut kind,
                )
            };
            if failed != 0 {
                return close_status(api, &found).unwrap_or_else(lost);
            }
            message.extend_from_slice(&chunk[..read as usize]);
            match kind {
                BINARY_FRAGMENT | UTF8_FRAGMENT => {}
                BINARY_MESSAGE => return Outcome::Bytes(message),
                UTF8_MESSAGE => {
                    return Outcome::Text(String::from_utf8_lossy(&message).into_owned());
                }
                CLOSE_MESSAGE => {
                    return close_status(api, &found).unwrap_or_else(|| {
                        Outcome::Names(vec!["close".to_owned(), "1005".to_owned(), String::new()])
                    });
                }
                _ => return lost(),
            }
        }
    }))
}

/// What the other end said when it closed, if it's said anything.
fn close_status(api: &Functions, found: &WebSocket) -> Option<Outcome> {
    let mut status = 0u16;
    let mut reason = [0u8; 123];
    let mut length = 0u32;
    // SAFETY: the socket lasts as long as `found`, and there's as much room as it's told.
    let failed = unsafe {
        (api.socket_status)(
            found.socket,
            &raw mut status,
            reason.as_mut_ptr().cast(),
            reason.len() as u32,
            &raw mut length,
        )
    };
    (failed == 0).then(|| {
        Outcome::Names(vec![
            "close".to_owned(),
            status.to_string(),
            String::from_utf8_lossy(&reason[..length as usize]).into_owned(),
        ])
    })
}

/// `wsClose(number, code, reason)`: a promise for having said so. What's waiting to hear goes on to hear the other end's answer.
fn ws_close(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = vm.arg(&args, 0);
    let number = vm.to_number(number)? as u32;
    let code = vm.arg(&args, 1);
    let code = vm.to_number(code)? as u16;
    let reason = vm.arg(&args, 2);
    let reason = vm.to_rust_string(reason)?;
    Ok(vm.in_own_thread(move || {
        let (Some(found), Some(api)) = (web_socket(number), functions()) else {
            return Outcome::Nothing;
        };
        // SAFETY: as in `ws_send`. A reason can't be longer than this.
        unsafe {
            (api.socket_shutdown)(
                found.socket,
                code,
                reason.as_ptr().cast(),
                reason.len().min(123) as u32,
            );
        }
        Outcome::Nothing
    }))
}

/// `wsForget(number)`, for one that's over.
fn ws_forget(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = vm.arg(&args, 0);
    let number = vm.to_number(number)? as u32;
    if let Ok(mut all) = web_sockets().lock() {
        all.remove(&number);
    }
    Ok(Value::UNDEFINED)
}

pub(crate) const NATIVES: [Native; 8] = [
    ("httpStart", 4, http_start),
    ("httpRead", 1, http_read),
    ("httpCancel", 1, http_cancel),
    ("wsConnect", 2, ws_connect),
    ("wsSend", 2, ws_send),
    ("wsReceive", 1, ws_receive),
    ("wsClose", 3, ws_close),
    ("wsForget", 1, ws_forget),
];

/// `HTTP/1.1 200 OK` and the lines after it.
fn read_headers(raw: &str) -> (String, Vec<(String, String)>) {
    let mut lines = raw.split("\r\n");
    let status_text = lines
        .next()
        .and_then(|line| line.splitn(3, ' ').nth(2))
        .unwrap_or("")
        .to_owned();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect();
    (status_text, headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_socket_addresses_are_asked_for_as_requests() {
        assert_eq!(
            web_socket_url("ws://a.com/b").as_deref(),
            Some("http://a.com/b")
        );
        assert_eq!(
            web_socket_url("WSS://a.com:9/b").as_deref(),
            Some("https://a.com:9/b")
        );
        assert_eq!(web_socket_url("http://a.com/"), None);
    }

    #[test]
    fn urls_come_apart() {
        assert_eq!(
            parse_url("https://example.com/a/b?c=d#e"),
            Some(Parts {
                secure: true,
                host: "example.com",
                port: 443,
                path: "/a/b?c=d",
            })
        );
        assert_eq!(
            parse_url("http://127.0.0.1:8080"),
            Some(Parts {
                secure: false,
                host: "127.0.0.1",
                port: 8080,
                path: "/",
            })
        );
        assert_eq!(
            parse_url("HTTP://Example.com?x"),
            Some(Parts {
                secure: false,
                host: "Example.com",
                port: 80,
                path: "?x"
            })
        );
        assert_eq!(parse_url("ftp://example.com/"), None);
        assert_eq!(parse_url("file:///c:/secret"), None);
        assert_eq!(parse_url("https://user:password@example.com/"), None);
        assert_eq!(parse_url("https://"), None);
        assert_eq!(parse_url("http://example.com:notaport/"), None);
    }

    #[test]
    fn headers_come_apart() {
        let (text, headers) =
            read_headers("HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nX-A: b: c\r\n\r\n");
        assert_eq!(text, "Not Found");
        assert_eq!(
            headers,
            [
                ("content-type".to_owned(), "text/plain".to_owned()),
                ("x-a".to_owned(), "b: c".to_owned())
            ]
        );
    }
}
