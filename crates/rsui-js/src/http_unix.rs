//! Network requests outside Windows, through libcurl, which Linux distributions and macOS come with and which looks after
//! TLS, proxies (as `http_proxy` and the rest say), redirects and decompression, as WinHTTP does on Windows (`http.rs`).
//! It's loaded the first time something makes a request, so a program that doesn't has one library less.
//!
//! Each request is a thread that runs it with curl's easy interface. What comes back is handed over a part at a time,
//! each when JavaScript asks for it: curl's thread waits in its callback until it's asked, which is what keeps a stream
//! that never ends, or a body bigger than memory, from piling up.
//!
//! Android has no libcurl of its own; its requests will go through Java's, and until then they fail there as they would
//! with no network. WebSockets aren't here yet either.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_long, c_void};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};

use crate::background::{Later, Native, Outcome, Response};
use crate::builtins::host::bytes_of;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

const REQUEST_STACK: usize = 256 * 1024;

const CURLOPT_URL: c_int = 10002;
const CURLOPT_WRITEDATA: c_int = 10001;
const CURLOPT_WRITEFUNCTION: c_int = 20011;
const CURLOPT_HEADERDATA: c_int = 10029;
const CURLOPT_HEADERFUNCTION: c_int = 20079;
const CURLOPT_FOLLOWLOCATION: c_int = 52;
const CURLOPT_CUSTOMREQUEST: c_int = 10036;
const CURLOPT_HTTPHEADER: c_int = 10023;
const CURLOPT_POSTFIELDS: c_int = 10015;
const CURLOPT_POSTFIELDSIZE_LARGE: c_int = 30120;
const CURLOPT_NOBODY: c_int = 44;
const CURLOPT_ACCEPT_ENCODING: c_int = 10102;
const CURLOPT_NOPROGRESS: c_int = 43;
const CURLOPT_XFERINFOFUNCTION: c_int = 20219;
const CURLOPT_XFERINFODATA: c_int = 10057;
const CURLOPT_NOSIGNAL: c_int = 99;
const CURLOPT_CONNECTTIMEOUT_MS: c_int = 156;
const CURLOPT_MAXREDIRS: c_int = 68;
const CURLINFO_RESPONSE_CODE: c_int = 0x0020_0002;
const CURLINFO_EFFECTIVE_URL: c_int = 0x0010_0001;
const CURL_GLOBAL_DEFAULT: c_long = 3;
/// What curl's error for something a callback stopped is.
const CURLE_WRITE_ERROR: c_int = 23;
const CURLE_ABORTED_BY_CALLBACK: c_int = 42;

type SetOption = unsafe extern "C" fn(*mut c_void, c_int, ...) -> c_int;
type GetInfo = unsafe extern "C" fn(*mut c_void, c_int, ...) -> c_int;
type WriteFn = unsafe extern "C" fn(*const c_char, usize, usize, *mut c_void) -> usize;
type ProgressFn = unsafe extern "C" fn(*mut c_void, i64, i64, i64, i64) -> c_int;

/// libcurl's functions that are used here.
struct Curl {
    easy_init: unsafe extern "C" fn() -> *mut c_void,
    easy_setopt: SetOption,
    easy_perform: unsafe extern "C" fn(*mut c_void) -> c_int,
    easy_getinfo: GetInfo,
    easy_cleanup: unsafe extern "C" fn(*mut c_void),
    easy_strerror: unsafe extern "C" fn(c_int) -> *const c_char,
    slist_append: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    slist_free_all: unsafe extern "C" fn(*mut c_void),
}

/// libcurl, loaded the first time it's wanted, if it's on this system.
fn curl() -> Option<&'static Curl> {
    static CURL: OnceLock<Option<Curl>> = OnceLock::new();
    CURL.get_or_init(|| {
        let names: &[&CStr] = if cfg!(target_vendor = "apple") {
            &[c"/usr/lib/libcurl.4.dylib", c"libcurl.4.dylib", c"libcurl.dylib"]
        } else {
            &[c"libcurl.so.4", c"libcurl-gnutls.so.4", c"libcurl-nss.so.4", c"libcurl.so"]
        };
        // SAFETY: each name ends in a zero; a library that's found stays loaded for good.
        let library = names.iter().find_map(|name| {
            let found = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
            (!found.is_null()).then_some(found)
        })?;
        let find = |name: &CStr| {
            // SAFETY: as above.
            let found = unsafe { libc::dlsym(library, name.as_ptr()) };
            (!found.is_null()).then_some(found)
        };
        let global_init = find(c"curl_global_init")?;
        // SAFETY: these are libcurl's functions, with the types its header declares them with.
        unsafe {
            let global_init = std::mem::transmute::<*mut c_void, unsafe extern "C" fn(c_long) -> c_int>(global_init);
            if global_init(CURL_GLOBAL_DEFAULT) != 0 {
                return None;
            }
            Some(Curl {
                easy_init: std::mem::transmute::<*mut c_void, _>(find(c"curl_easy_init")?),
                easy_setopt: std::mem::transmute::<*mut c_void, SetOption>(find(c"curl_easy_setopt")?),
                easy_perform: std::mem::transmute::<*mut c_void, _>(find(c"curl_easy_perform")?),
                easy_getinfo: std::mem::transmute::<*mut c_void, GetInfo>(find(c"curl_easy_getinfo")?),
                easy_cleanup: std::mem::transmute::<*mut c_void, _>(find(c"curl_easy_cleanup")?),
                easy_strerror: std::mem::transmute::<*mut c_void, _>(find(c"curl_easy_strerror")?),
                slist_append: std::mem::transmute::<*mut c_void, _>(find(c"curl_slist_append")?),
                slist_free_all: std::mem::transmute::<*mut c_void, _>(find(c"curl_slist_free_all")?),
            })
        }
    })
    .as_ref()
}

/// What JavaScript says to a request while it runs.
enum Event {
    Read(Later),
    Cancel,
}

fn exchanges() -> &'static Mutex<HashMap<u32, Sender<Event>>> {
    static ALL: OnceLock<Mutex<HashMap<u32, Sender<Event>>>> = OnceLock::new();
    ALL.get_or_init(Mutex::default)
}

/// What's asked for.
struct Asked {
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// What curl's callbacks share with the request's thread, while it runs.
struct Exchange<'a> {
    curl: &'static Curl,
    handle: *mut c_void,
    /// The promise of the status and headers, until it's kept.
    head: Option<Later>,
    /// The header lines of the response that's coming, which a redirect starts again.
    lines: Vec<String>,
    events: &'a Receiver<Event>,
    cancelled: &'a AtomicBool,
}

impl Exchange<'_> {
    /// Keeps the promise of the head, with what's been heard so far.
    fn tell_head(&mut self) {
        let Some(head) = self.head.take() else { return };
        let mut status: c_long = 0;
        let mut url: *const c_char = std::ptr::null();
        // SAFETY: the handle's running this request; each answer has room for what it is.
        unsafe {
            (self.curl.easy_getinfo)(self.handle, CURLINFO_RESPONSE_CODE, &raw mut status);
            (self.curl.easy_getinfo)(self.handle, CURLINFO_EFFECTIVE_URL, &raw mut url);
        }
        let url = if url.is_null() {
            String::new()
        } else {
            // SAFETY: curl's string, which lasts as long as the handle.
            unsafe { CStr::from_ptr(url) }.to_string_lossy().into_owned()
        };
        let (status_text, headers) = read_head(&self.lines);
        head.settle(Outcome::Response(Box::new(Response {
            status: u32::try_from(status).unwrap_or(0),
            status_text,
            headers,
            url,
            body: Vec::new(),
        })));
    }

    /// Waits for JavaScript to ask for the next part, and gives it this. Says whether to go on.
    fn hand_over(&mut self, bytes: Vec<u8>) -> bool {
        loop {
            match self.events.recv() {
                Ok(Event::Read(later)) => {
                    later.settle(Outcome::Bytes(bytes));
                    return true;
                }
                Ok(Event::Cancel) | Err(_) => {
                    self.cancelled.store(true, Ordering::Relaxed);
                    return false;
                }
            }
        }
    }
}

/// `HTTP/1.1 200 OK` and the lines after it, as `(text, [(name, value)])`, the names in lower case.
fn read_head(lines: &[String]) -> (String, Vec<(String, String)>) {
    let status_text =
        lines.first().and_then(|line| line.splitn(3, ' ').nth(2)).unwrap_or("").trim().to_owned();
    let headers = lines
        .iter()
        .skip(1)
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect();
    (status_text, headers)
}

unsafe extern "C" fn heard_header(data: *const c_char, size: usize, count: usize, context: *mut c_void) -> usize {
    let length = size * count;
    // SAFETY: curl gives the exchange it was given, from the request's own thread, and a line `length` long.
    let (exchange, line) = unsafe {
        (&mut *context.cast::<Exchange>(), std::slice::from_raw_parts(data.cast::<u8>(), length))
    };
    let line = String::from_utf8_lossy(line).trim_end_matches(['\r', '\n']).to_owned();
    if line.starts_with("HTTP/") {
        // Each response, after a redirect or a `100 Continue`, starts its lines again.
        exchange.lines.clear();
        exchange.lines.push(line);
    } else if line.is_empty() {
        // The end of a response's head: it's the one that counts unless it's on its way somewhere else.
        let status: u32 = exchange.lines.first().and_then(|first| first.split(' ').nth(1)?.parse().ok()).unwrap_or(0);
        let moving = (300..400).contains(&status) && exchange.lines.iter().any(|line| line.to_ascii_lowercase().starts_with("location:"));
        if status >= 200 && !moving {
            exchange.tell_head();
        }
    } else {
        exchange.lines.push(line);
    }
    if exchange.cancelled.load(Ordering::Relaxed) { 0 } else { length }
}

unsafe extern "C" fn heard_body(data: *const c_char, size: usize, count: usize, context: *mut c_void) -> usize {
    let length = size * count;
    // SAFETY: as for `heard_header`.
    let (exchange, bytes) = unsafe {
        (&mut *context.cast::<Exchange>(), std::slice::from_raw_parts(data.cast::<u8>(), length))
    };
    exchange.tell_head();
    if exchange.hand_over(bytes.to_vec()) { length } else { 0 }
}

unsafe extern "C" fn going(context: *mut c_void, _: i64, _: i64, _: i64, _: i64) -> c_int {
    // SAFETY: the flag the request was given, which outlives it.
    let cancelled = unsafe { &*context.cast::<AtomicBool>() };
    // A request that's cancelled while nothing's coming stops at curl's next look, about once a second.
    c_int::from(cancelled.load(Ordering::Relaxed))
}

/// Makes a request, says what came back once the headers are in, and then hands over its answer a part at a time, each
/// when it's asked for, until it ends, fails, or is cancelled.
fn exchange(asked: Asked, head: Later, events: Receiver<Event>) {
    let Some(curl) = curl() else {
        head.settle(Outcome::RequestFailed("libcurl isn't on this system".to_owned()));
        return;
    };
    let lower = asked.url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        head.settle(Outcome::RequestFailed(format!("{} isn't an http or https URL", asked.url)));
        return;
    }
    let (Ok(url), Ok(method)) = (CString::new(asked.url.clone()), CString::new(asked.method.clone())) else {
        head.settle(Outcome::RequestFailed("the address has a zero in it".to_owned()));
        return;
    };
    let cancelled = AtomicBool::new(false);
    // SAFETY: the handle and the list of headers are made and freed here; everything curl's given lasts through
    // `perform`, which is when it uses them.
    unsafe {
        let handle = (curl.easy_init)();
        if handle.is_null() {
            head.settle(Outcome::RequestFailed("curl couldn't start".to_owned()));
            return;
        }
        let mut list: *mut c_void = std::ptr::null_mut();
        let lines: Vec<CString> = asked
            .headers
            .iter()
            .filter_map(|(name, value)| CString::new(format!("{name}: {value}")).ok())
            .collect();
        for line in &lines {
            list = (curl.slist_append)(list, line.as_ptr());
        }
        let mut exchange = Exchange {
            curl,
            handle,
            head: Some(head),
            lines: Vec::new(),
            events: &events,
            cancelled: &cancelled,
        };
        let set = curl.easy_setopt;
        set(handle, CURLOPT_URL, url.as_ptr());
        set(handle, CURLOPT_NOSIGNAL, 1 as c_long);
        set(handle, CURLOPT_FOLLOWLOCATION, 1 as c_long);
        set(handle, CURLOPT_MAXREDIRS, 20 as c_long);
        set(handle, CURLOPT_CONNECTTIMEOUT_MS, 60_000 as c_long);
        // What the server can squeeze, curl undoes.
        set(handle, CURLOPT_ACCEPT_ENCODING, c"".as_ptr());
        if asked.method.eq_ignore_ascii_case("HEAD") {
            set(handle, CURLOPT_NOBODY, 1 as c_long);
        } else {
            set(handle, CURLOPT_CUSTOMREQUEST, method.as_ptr());
        }
        if !asked.body.is_empty() {
            set(handle, CURLOPT_POSTFIELDSIZE_LARGE, asked.body.len() as i64);
            set(handle, CURLOPT_POSTFIELDS, asked.body.as_ptr());
        }
        if !list.is_null() {
            set(handle, CURLOPT_HTTPHEADER, list);
        }
        let context = (&raw mut exchange).cast::<c_void>();
        set(handle, CURLOPT_HEADERFUNCTION, heard_header as WriteFn);
        set(handle, CURLOPT_HEADERDATA, context);
        set(handle, CURLOPT_WRITEFUNCTION, heard_body as WriteFn);
        set(handle, CURLOPT_WRITEDATA, context);
        set(handle, CURLOPT_NOPROGRESS, 0 as c_long);
        set(handle, CURLOPT_XFERINFOFUNCTION, going as ProgressFn);
        set(handle, CURLOPT_XFERINFODATA, (&raw const cancelled).cast::<c_void>());
        let code = (curl.easy_perform)(handle);
        // Stopped by JavaScript, through a callback, which curl says as one of these.
        let stopped = cancelled.load(Ordering::Relaxed) && matches!(code, CURLE_WRITE_ERROR | CURLE_ABORTED_BY_CALLBACK);
        let ended = if code == 0 || stopped {
            // What had a head and no body, as a `204` or `HEAD` has, says so now.
            exchange.tell_head();
            Outcome::Null
        } else {
            let said = CStr::from_ptr((curl.easy_strerror)(code)).to_string_lossy().into_owned();
            let failed = Outcome::RequestFailed(format!("{said} (curl error {code})"));
            if let Some(head) = exchange.head.take() {
                head.settle(failed);
                Outcome::Null
            } else {
                failed
            }
        };
        (curl.easy_cleanup)(handle);
        if !list.is_null() {
            (curl.slist_free_all)(list);
        }
        // What's asked for after the end is told it's the end, and after a failure, that it failed. A read that's
        // already waiting gets that now; a failure with none waiting waits a while for the read that will hear of it.
        // Reads after this thread's gone find no request, which is the end.
        let failed = !matches!(ended, Outcome::Null);
        let mut ended = Some(ended);
        drop(exchange);
        while let Ok(event) = events.try_recv() {
            if let Event::Read(later) = event {
                later.settle(ended.take().unwrap_or(Outcome::Null));
            }
        }
        if failed
            && ended.is_some()
            && let Ok(Event::Read(later)) = events.recv_timeout(std::time::Duration::from_secs(30))
        {
            later.settle(ended.take().unwrap_or(Outcome::Null));
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

/// `httpStart(url, method, [[name, value]...], body)`: the request's number, and a promise of its status and headers.
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
    } else if given.is_undefined() || given.is_nullish() {
        Vec::new()
    } else {
        bytes_of(vm, given)?
    };
    let asked = Asked { url, method, headers, body };
    let (promise, head) = vm.promise_for_later(false);
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    let (sender, events) = channel();
    if let Ok(mut all) = exchanges().lock() {
        all.insert(number, sender);
    }
    let started = std::thread::Builder::new().name("rsui request".into()).stack_size(REQUEST_STACK).spawn(move || {
        exchange(asked, head, events);
        if let Ok(mut all) = exchanges().lock() {
            all.remove(&number);
        }
    });
    if started.is_err()
        && let Ok(mut all) = exchanges().lock()
    {
        all.remove(&number);
    }
    Ok(vm.new_array(vec![Value::number(f64::from(number)), promise]))
}

/// `httpRead(number)`: a promise of the next part of what came back, as bytes, or `null` once that's all.
fn http_read(vm: &mut Vm, args: Args) -> Run<Value> {
    let number = vm.arg(&args, 0);
    let number = vm.to_number(number)? as u32;
    // This only waits to hear something, as a stream of events can stay open for good.
    let (promise, later) = vm.promise_for_later(true);
    let sender = exchanges().lock().ok().and_then(|all| all.get(&number).cloned());
    match sender {
        Some(sender) => {
            if let Err(std::sync::mpsc::SendError(Event::Read(later))) = sender.send(Event::Read(later)) {
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
    let sender = exchanges().lock().ok().and_then(|mut all| all.remove(&number));
    if let Some(sender) = sender {
        let _ = sender.send(Event::Cancel);
    }
    Ok(Value::UNDEFINED)
}

fn no_web_sockets(vm: &mut Vm, _: Args) -> Run<Value> {
    Ok(vm.settled_promise(Outcome::RequestFailed("WebSockets aren't on this system yet".to_owned())))
}

fn nothing(_: &mut Vm, _: Args) -> Run<Value> {
    Ok(Value::UNDEFINED)
}

pub(crate) const NATIVES: [Native; 8] = [
    ("httpStart", 4, http_start),
    ("httpRead", 1, http_read),
    ("httpCancel", 1, http_cancel),
    ("wsConnect", 2, no_web_sockets),
    ("wsSend", 2, no_web_sockets),
    ("wsReceive", 1, no_web_sockets),
    ("wsClose", 3, nothing),
    ("wsForget", 1, nothing),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_response_head_is_read_as_a_status_and_headers() {
        let lines = ["HTTP/1.1 404 Not Found", "Content-Type: text/plain", "X-A: b: c"].map(str::to_owned);
        let (text, headers) = read_head(&lines);
        assert_eq!(text, "Not Found");
        assert_eq!(headers, [("content-type".to_owned(), "text/plain".to_owned()), ("x-a".to_owned(), "b: c".to_owned())]);
    }
}
