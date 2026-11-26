//! What programs expect from where they run, though it isn't part of the language: `process`, `crypto`, and text
//! encoding, and files for `node/fs.js`.

use crate::background::Outcome;
use crate::object::*;
use crate::text::{self, Text};
use crate::value::Value;
use crate::vm::{Args, Job, Run, Vm};

mod buffer;
pub(crate) mod file_stream;
mod path;
mod path_posix;
mod path_win32;
#[cfg(windows)]
mod watch;
#[cfg(not(windows))]
#[path = "host/watch_unix.rs"]
mod watch;
pub(crate) mod zlib;

// Files, for `node/fs.js`. Each thing it can do can happen right away, or on another thread with a promise for how it
// went, and it's the same code either way.

pub(crate) fn failed(error: &std::io::Error, doing: &'static str, path: String) -> Outcome {
    use std::io::ErrorKind;
    let (code, what) = match error.kind() {
        ErrorKind::NotFound => ("ENOENT", "no such file or directory"),
        ErrorKind::PermissionDenied => ("EACCES", "permission denied"),
        ErrorKind::AlreadyExists => ("EEXIST", "file already exists"),
        ErrorKind::IsADirectory => ("EISDIR", "illegal operation on a directory"),
        ErrorKind::NotADirectory => ("ENOTDIR", "not a directory"),
        ErrorKind::DirectoryNotEmpty => ("ENOTEMPTY", "directory not empty"),
        _ => ("EIO", "input or output error"),
    };
    Outcome::FileFailed {
        code,
        what,
        doing,
        path,
    }
}

/// What's in a file: the one inside this program, if it's packed and has it, as that's the app's, or the one on the disk.
fn file_bytes(path: &str) -> std::io::Result<Vec<u8>> {
    match crate::packed::read_packed_file(std::path::Path::new(path)) {
        Some(bytes) => Ok(bytes),
        None => std::fs::read(path),
    }
}

fn read_file(path: String) -> Outcome {
    match file_bytes(&path) {
        Ok(bytes) => Outcome::Text(String::from_utf8_lossy(&bytes).into_owned()),
        Err(error) => failed(&error, "open", path),
    }
}

fn read_bytes(path: String) -> Outcome {
    match file_bytes(&path) {
        Ok(bytes) => Outcome::Bytes(bytes),
        Err(error) => failed(&error, "open", path),
    }
}

fn copy_file(from: String, to: String) -> Outcome {
    match std::fs::copy(&from, &to) {
        Ok(_) => Outcome::Nothing,
        Err(error) => failed(&error, "copyfile", from),
    }
}

fn real_path(path: String) -> Outcome {
    match std::fs::canonicalize(&path) {
        Ok(found) => {
            let found = found.to_string_lossy();
            Outcome::Text(found.strip_prefix(r"\\?\").unwrap_or(&found).to_owned())
        }
        Err(error) => failed(&error, "realpath", path),
    }
}

fn write_file(path: String, text: impl AsRef<[u8]>, append: bool) -> Outcome {
    use std::io::Write as _;
    let written = if append {
        std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .and_then(|mut file| file.write_all(text.as_ref()))
    } else {
        std::fs::write(&path, text)
    };
    match written {
        Ok(()) => Outcome::Nothing,
        Err(error) => failed(&error, "open", path),
    }
}

fn file_exists(path: String) -> Outcome {
    let path = std::path::Path::new(&path);
    Outcome::Bool(
        crate::packed::is_packed_file(path)
            || crate::packed::is_packed_folder(path)
            || path.exists(),
    )
}

fn make_folder(path: String, recursive: bool) -> Outcome {
    let made = if recursive {
        std::fs::create_dir_all(&path)
    } else {
        std::fs::create_dir(&path)
    };
    match made {
        Ok(()) => Outcome::Nothing,
        Err(error) => failed(&error, "mkdir", path),
    }
}

fn remove_file(path: String, recursive: bool, force: bool) -> Outcome {
    let gone = match std::fs::symlink_metadata(&path) {
        Ok(found) if found.is_dir() && recursive => std::fs::remove_dir_all(&path),
        Ok(found) if found.is_dir() => std::fs::remove_dir(&path),
        Ok(_) => std::fs::remove_file(&path),
        Err(error) => Err(error),
    };
    match gone {
        Ok(()) => Outcome::Nothing,
        Err(error) if force && error.kind() == std::io::ErrorKind::NotFound => Outcome::Nothing,
        Err(error) => failed(&error, "unlink", path),
    }
}

fn rename_file(from: String, to: String) -> Outcome {
    match std::fs::rename(&from, to) {
        Ok(()) => Outcome::Nothing,
        Err(error) => failed(&error, "rename", from),
    }
}

fn list_folder(path: String) -> Outcome {
    let names: Result<Vec<String>, _> = std::fs::read_dir(&path).and_then(|entries| {
        entries
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect()
    });
    match names {
        Ok(mut names) => {
            names.sort();
            Outcome::Names(names)
        }
        Err(error) => failed(&error, "scandir", path),
    }
}

fn file_stat(path: String) -> Outcome {
    if let Some(size) = crate::packed::packed_size(std::path::Path::new(&path)) {
        return Outcome::Stat {
            file: true,
            folder: false,
            size: size as f64,
            modified: 0.0,
        };
    }
    if crate::packed::is_packed_folder(std::path::Path::new(&path)) {
        return Outcome::Stat {
            file: false,
            folder: true,
            size: 0.0,
            modified: 0.0,
        };
    }
    match std::fs::metadata(&path) {
        Ok(found) => Outcome::Stat {
            file: found.is_file(),
            folder: found.is_dir(),
            size: found.len() as f64,
            modified: found
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0.0, |since| since.as_secs_f64() * 1000.0),
        },
        Err(error) => failed(&error, "stat", path),
    }
}

fn text_argument(vm: &mut Vm, args: &Args, index: usize) -> Run<String> {
    let value = vm.arg(args, index);
    vm.to_rust_string(value)
}

/// Makes the two ways of calling something: right away, and later, which has `Later` on the end of its name.
macro_rules! file_operation {
    ($now:ident, $later:ident, |$vm:ident, $args:ident| { $($read:tt)* } => $run:expr) => {
        fn $now($vm: &mut Vm, $args: Args) -> Run<Value> {
            $($read)*
            let outcome = $run;
            $vm.settle_now(outcome)
        }

        fn $later($vm: &mut Vm, $args: Args) -> Run<Value> {
            $($read)*
            Ok($vm.in_background(move || $run))
        }
    };
}

file_operation!(file_read, file_read_later, |vm, args| {
    let path = text_argument(vm, &args, 0)?;
} => read_file(path));
file_operation!(file_read_bytes, file_read_bytes_later, |vm, args| {
    let path = text_argument(vm, &args, 0)?;
} => read_bytes(path));
file_operation!(file_copy, file_copy_later, |vm, args| {
    let from = text_argument(vm, &args, 0)?;
    let to = text_argument(vm, &args, 1)?;
} => copy_file(from, to));
file_operation!(file_realpath, file_realpath_later, |vm, args| {
    let path = text_argument(vm, &args, 0)?;
} => real_path(path));
file_operation!(file_write_bytes, file_write_bytes_later, |vm, args| {
    let path = text_argument(vm, &args, 0)?;
    let given = vm.arg(&args, 1);
    let bytes = bytes_of(vm, given)?;
    let append = vm.arg(&args, 2) == Value::TRUE;
} => write_file(path, bytes, append));
file_operation!(file_write, file_write_later, |vm, args| {
    let path = text_argument(vm, &args, 0)?;
    let text = text_argument(vm, &args, 1)?;
    let append = vm.arg(&args, 2) == Value::TRUE;
} => write_file(path, text, append));
file_operation!(file_exists_now, file_exists_later, |vm, args| {
    let path = text_argument(vm, &args, 0)?;
} => file_exists(path));
file_operation!(file_mkdir, file_mkdir_later, |vm, args| {
    let path = text_argument(vm, &args, 0)?;
    let recursive = vm.arg(&args, 1) == Value::TRUE;
} => make_folder(path, recursive));
file_operation!(file_remove, file_remove_later, |vm, args| {
    let path = text_argument(vm, &args, 0)?;
    let recursive = vm.arg(&args, 1) == Value::TRUE;
    let force = vm.arg(&args, 2) == Value::TRUE;
} => remove_file(path, recursive, force));
file_operation!(file_rename, file_rename_later, |vm, args| {
    let from = text_argument(vm, &args, 0)?;
    let to = text_argument(vm, &args, 1)?;
} => rename_file(from, to));
file_operation!(file_list, file_list_later, |vm, args| {
    let path = text_argument(vm, &args, 0)?;
} => list_folder(path));
file_operation!(file_stat_now, file_stat_later, |vm, args| {
    let path = text_argument(vm, &args, 0)?;
} => file_stat(path));

/// What `node/fs.js` and `node/fetch.js` are written with. It's only made for a program that asks for them.
pub(crate) fn file_natives(vm: &mut Vm) -> Value {
    let out = vm.new_plain();
    for (name, length, function) in [
        ("read", 1, file_read as NativeFn),
        ("readLater", 1, file_read_later),
        ("write", 3, file_write),
        ("writeLater", 3, file_write_later),
        ("exists", 1, file_exists_now),
        ("existsLater", 1, file_exists_later),
        ("mkdir", 2, file_mkdir),
        ("mkdirLater", 2, file_mkdir_later),
        ("remove", 3, file_remove),
        ("removeLater", 3, file_remove_later),
        ("rename", 2, file_rename),
        ("renameLater", 2, file_rename_later),
        ("list", 1, file_list),
        ("listLater", 1, file_list_later),
        ("stat", 1, file_stat_now),
        ("statLater", 1, file_stat_later),
        ("readBytes", 1, file_read_bytes),
        ("readBytesLater", 1, file_read_bytes_later),
        ("streamReader", 2, file_stream::create),
        ("streamRead", 2, file_stream::read),
        ("streamClose", 1, file_stream::close),
        ("writeBytes", 3, file_write_bytes),
        ("writeBytesLater", 3, file_write_bytes_later),
        ("copy", 2, file_copy),
        ("copyLater", 2, file_copy_later),
        ("realpath", 1, file_realpath),
        ("realpathLater", 1, file_realpath_later),
    ] {
        vm.method(out.handle(), name, length, function);
    }
    for (name, length, function) in zlib::NATIVES
        .into_iter()
        .chain(buffer::NATIVES)
        .chain(path::NATIVES)
        .chain(watch::NATIVES)
     {
        vm.method(out.handle(), name, length, function);
    }
    super::system::add_natives(vm, out);
    out
}

/// How long `source` is once each line has lost its indentation, and each line that's only a comment is left empty.
const fn squeezed_length(source: &[u8]) -> usize {
    let mut length = 0;
    let mut at = 0;
    while at < source.len() {
        let (start, end) = squeezed_line(source, at);
        length += end - start + 1;
        at = line_end(source, at) + 1;
    }
    length
}

/// Where the line starting at `at` ends, at its newline or the end of `source`.
const fn line_end(source: &[u8], mut at: usize) -> usize {
    while at < source.len() && source[at] != b'\n' {
        at += 1;
    }
    at
}

/// What's kept of the line starting at `at`, as where that starts and ends.
const fn squeezed_line(source: &[u8], at: usize) -> (usize, usize) {
    let mut end = line_end(source, at);
    if end > at && source[end - 1] == b'\r' {
        end -= 1;
    }
    let mut start = at;
    while start < end && (source[start] == b' ' || source[start] == b'\t') {
        start += 1;
    }
    if start + 1 < end && source[start] == b'/' && source[start + 1] == b'/' {
        start = end;
    }
    (start, end)
}

/// `source` with each line squeezed, as [`squeezed_length`] says, keeping every line, so that where an error says it
/// happened is still right. It's only for code with no text in it that goes over more than one line.
const fn squeeze<const LENGTH: usize>(source: &[u8]) -> [u8; LENGTH] {
    let mut out = [0u8; LENGTH];
    let mut written = 0;
    let mut at = 0;
    while at < source.len() {
        let (mut start, end) = squeezed_line(source, at);
        while start < end {
            out[written] = source[start];
            written += 1;
            start += 1;
        }
        out[written] = b'\n';
        written += 1;
        at = line_end(source, at) + 1;
    }
    out
}

/// A module's source, squeezed while this is being compiled, as a program carries every byte of it.
macro_rules! squeezed {
    ($path:literal) => {{
        const SOURCE: &[u8] = include_bytes!($path);
        const LENGTH: usize = squeezed_length(SOURCE);
        const BYTES: [u8; LENGTH] = squeeze::<LENGTH>(SOURCE);
        const TEXT: &str = match std::str::from_utf8(&BYTES) {
            Ok(text) => text,
            Err(_) => panic!("a module's source must be UTF-8"),
        };
        TEXT
    }};
}

/// The source of one of the modules that are only compiled when they're wanted, by its name as a module:
/// `internal/misc/os` and so on. Most are the small ones `node/misc.js` makes.
pub(crate) fn misc_source(name: &str) -> Option<&'static str> {
    Some(match name.strip_prefix("internal/misc/")? {
        "web_streams" => squeezed!("../node/web_streams.js"),
        "os" => include_str!("../node/misc/os.js"),
        "crypto" => include_str!("../node/misc/crypto.js"),
        "querystring" => include_str!("../node/misc/querystring.js"),
        "string_decoder" => include_str!("../node/misc/string_decoder.js"),
        "timers" => include_str!("../node/misc/timers.js"),
        "module" => include_str!("../node/misc/module.js"),
        "punycode" => include_str!("../node/misc/punycode.js"),
        "tty" => include_str!("../node/misc/tty.js"),
        "net" => include_str!("../node/misc/net.js"),
        "dns" => include_str!("../node/misc/dns.js"),
        "worker_threads" => include_str!("../node/misc/worker_threads.js"),
        "dgram" => include_str!("../node/misc/dgram.js"),
        "perf_hooks" => include_str!("../node/misc/perf_hooks.js"),
        "fs_watch" => include_str!("../node/misc/fs_watch.js"),
        _ => return None,
    })
}

/// The first time it's called, `fetch` and what goes with it are compiled, and then put in place of this.
fn first_fetch(vm: &mut Vm, args: Args) -> Run<Value> {
    let module = vm
        .load_builtin_module("internal/fetch")?
        .expect("it comes with the engine");
    let exports = vm.exports_of(module)?;
    let name = vm.intern_str("fetch");
    let fetch = vm.get(exports, name)?;
    let global = vm.global;
    vm.define(global, name, fetch, HIDDEN);
    let mut given = Vec::with_capacity(args.count);
    for index in 0..args.count {
        given.push(vm.arg(&args, index));
    }
    vm.call(fetch, Value::UNDEFINED, &given)
}

/// Names that are there as soon as anything looks at them, and only compiled then. Something can also set one of
/// these itself, as packages that bring their own do, which is then what it is from then on.
macro_rules! lazy_globals {
    ($($module:literal: $($name:literal $get:ident $set:ident),+;)+) => {
        $($(
            fn $get(vm: &mut Vm, _: Args) -> Run<Value> {
                let module = vm
                    .load_builtin_module($module)?
                    .expect("it comes with the engine");
                let exports = vm.exports_of(module)?;
                let atom = vm.intern_str($name);
                let found = vm.get(exports, atom)?;
                let global = vm.global;
                vm.define(global, atom, found, HIDDEN);
                Ok(found)
            }

            fn $set(vm: &mut Vm, args: Args) -> Run<Value> {
                let atom = vm.intern_str($name);
                let value = vm.arg(&args, 0);
                let global = vm.global;
                vm.define(global, atom, value, HIDDEN);
                Ok(Value::UNDEFINED)
            }
        )+)+

        fn install_lazy_globals(vm: &mut Vm) {
            $($(
                let atom = vm.intern_str($name);
                let get = vm.new_native(atom, 0, $get, Value::UNDEFINED, false);
                let set = vm.new_native(atom, 1, $set, Value::UNDEFINED, false);
                let global = vm.global;
                vm.define_accessor(global, atom, Some(get), Some(set), CONFIGURABLE);
            )+)+
        }
    };
}

lazy_globals! {
    "internal/fetch":
        "FormData" get_form_data set_form_data,
        "Headers" get_headers set_headers,
        "Request" get_request set_request,
        "Response" get_response set_response;
    "internal/url":
        "URL" get_url set_url,
        "URLSearchParams" get_url_search_params set_url_search_params;
    "internal/event-target":
        "Event" get_event set_event,
        "EventTarget" get_event_target set_event_target,
        "AbortController" get_abort_controller set_abort_controller,
        "AbortSignal" get_abort_signal set_abort_signal;
    "internal/buffer":
        "Buffer" get_buffer set_buffer;
    "internal/intl":
        "Intl" get_intl set_intl;
    "internal/more":
        "navigator" get_navigator set_navigator,
        "DOMException" get_dom_exception set_dom_exception,
        "Blob" get_blob set_blob,
        "File" get_file set_file,
        "MessageChannel" get_message_channel set_message_channel,
        "MessagePort" get_message_port set_message_port;
    "internal/websocket":
        "WebSocket" get_web_socket set_web_socket,
        "CloseEvent" get_close_event set_close_event,
        "MessageEvent" get_message_event set_message_event;
    "internal/worker-global":
        "Worker" get_worker set_worker;
    "internal/misc/web_streams":
        "ReadableStream" get_readable_stream set_readable_stream,
        "ReadableStreamDefaultReader" get_readable_stream_default_reader set_readable_stream_default_reader,
        "ReadableStreamBYOBReader" get_readable_stream_byob_reader set_readable_stream_byob_reader,
        "ReadableStreamDefaultController" get_readable_stream_default_controller set_readable_stream_default_controller,
        "ReadableByteStreamController" get_readable_byte_stream_controller set_readable_byte_stream_controller,
        "ReadableStreamBYOBRequest" get_readable_stream_byob_request set_readable_stream_byob_request,
        "WritableStream" get_writable_stream set_writable_stream,
        "WritableStreamDefaultWriter" get_writable_stream_default_writer set_writable_stream_default_writer,
        "WritableStreamDefaultController" get_writable_stream_default_controller set_writable_stream_default_controller,
        "TransformStream" get_transform_stream set_transform_stream,
        "TransformStreamDefaultController" get_transform_stream_default_controller set_transform_stream_default_controller,
        "ByteLengthQueuingStrategy" get_byte_length_queuing_strategy set_byte_length_queuing_strategy,
        "CountQueuingStrategy" get_count_queuing_strategy set_count_queuing_strategy,
        "TextEncoderStream" get_text_encoder_stream set_text_encoder_stream,
        "TextDecoderStream" get_text_decoder_stream set_text_decoder_stream,
        "CompressionStream" get_compression_stream set_compression_stream,
        "DecompressionStream" get_decompression_stream set_decompression_stream;
}

/// On Windows, `process.env.PATH` is the same as `Path`, which is how it's usually written down. What does that is only
/// made if something looks.
fn process_env_get(vm: &mut Vm, args: Args) -> Run<Value> {
    let module = vm
        .load_builtin_module("internal/env")?
        .expect("it comes with the engine");
    let make = vm.exports_of(module)?;
    let plain = Value::object(vm.realm.process_env);
    let found = vm.call(make, Value::UNDEFINED, &[plain])?;
    let atom = vm.intern_str("env");
    vm.define(args.this.handle(), atom, found, PLAIN);
    Ok(found)
}

fn process_env_set(vm: &mut Vm, args: Args) -> Run<Value> {
    let atom = vm.intern_str("env");
    let given = vm.arg(&args, 0);
    vm.define(args.this.handle(), atom, given, PLAIN);
    Ok(Value::UNDEFINED)
}

fn process_returns_itself(_: &mut Vm, args: Args) -> Run<Value> {
    Ok(args.this)
}

fn process_nothing_was_listening(_: &mut Vm, _: Args) -> Run<Value> {
    Ok(Value::FALSE)
}

fn process_exit(vm: &mut Vm, args: Args) -> Run<Value> {
    use std::io::Write as _;
    let code = vm.arg(&args, 0);
    let code = if code.is_undefined() {
        let atom = vm.intern_str("exitCode");
        vm.get(args.this, atom)?
    } else {
        code
    };
    let code = if code.is_undefined() {
        0.0
    } else {
        vm.to_number(code)?
    };
    let _ = std::io::stdout().flush();
    std::process::exit(code as i32);
}

fn process_uptime(_: &mut Vm, _: Args) -> Run<Value> {
    Ok(Value::number(Vm::now_ms() / 1000.0))
}

fn process_memory_usage(vm: &mut Vm, _: Args) -> Run<Value> {
    let out = vm.new_plain();
    let heap = vm.objects.len() as f64 * 64.0;
    for name in ["rss", "heapTotal", "heapUsed", "external", "arrayBuffers"] {
        let atom = vm.intern_str(name);
        vm.define(out.handle(), atom, Value::number(heap), PLAIN);
    }
    Ok(out)
}

/// `process.stdout` and the others are only made if they're looked for, which takes the module for streams.
fn process_stream(vm: &mut Vm, this: Value, name: &str) -> Run<Value> {
    let module = vm
        .load_builtin_module("internal/standard-streams")?
        .expect("it comes with the engine");
    let exports = vm.exports_of(module)?;
    let atom = vm.intern_str(name);
    let found = vm.get(exports, atom)?;
    vm.define(this.handle(), atom, found, PLAIN);
    Ok(found)
}

fn process_stdout(vm: &mut Vm, args: Args) -> Run<Value> {
    process_stream(vm, args.this, "stdout")
}

fn process_stderr(vm: &mut Vm, args: Args) -> Run<Value> {
    process_stream(vm, args.this, "stderr")
}

fn process_stdin(vm: &mut Vm, args: Args) -> Run<Value> {
    process_stream(vm, args.this, "stdin")
}

fn process_stream_set(_: &mut Vm, _: Args) -> Run<Value> {
    Ok(Value::UNDEFINED)
}

fn process_cwd(vm: &mut Vm, _: Args) -> Run<Value> {
    let dir = std::env::current_dir().unwrap_or_default();
    Ok(vm.new_str(&dir.to_string_lossy()))
}

fn process_next_tick(vm: &mut Vm, args: Args) -> Run<Value> {
    let function = vm.arg(&args, 0);
    if !vm.is_callable(function) {
        return Err(vm.type_error("The callback must be a function"));
    }
    let job = if args.count > 1 {
        let extra: Box<[Value]> = (1..args.count).map(|index| vm.arg(&args, index)).collect();
        vm.new_object_with_proto(
            vm.realm.function_proto,
            Kind::Bound(Box::new(BoundData {
                target: function,
                this: Value::UNDEFINED,
                args: extra,
            })),
        )
    } else {
        function
    };
    vm.jobs.push_back(Job::Call(job));
    Ok(Value::UNDEFINED)
}

fn process_hrtime(vm: &mut Vm, _: Args) -> Run<Value> {
    let now = Vm::now_ms();
    let seconds = (now / 1000.0).floor();
    let nanos = ((now - seconds * 1000.0) * 1e6).floor();
    Ok(vm.new_array(vec![Value::number(seconds), Value::number(nanos)]))
}

thread_local! {
    /// When this thread's program started, on the epoch's clock: `performance.timeOrigin`, which `performance.now()` counts from,
    /// as a page's clock counts from its navigation's start (HR Time 3 §4). Each thread's program, as a worker's, has its own.
    static TIME_ORIGIN: std::cell::Cell<f64> = const { std::cell::Cell::new(f64::NAN) };
}

/// This thread's time origin, which is now the first time it's asked.
fn time_origin() -> f64 {
    TIME_ORIGIN.with(|origin| {
        if origin.get().is_nan() {
            origin.set(Vm::now_ms());
        }
        origin.get()
    })
}

/// The milliseconds since the program started, as a page's `performance.now()` is since it was navigated to: what event
/// `timeStamp`s and animation frames are told too.
fn performance_now(_: &mut Vm, _: Args) -> Run<Value> {
    Ok(Value::number(Vm::now_ms() - time_origin()))
}

fn nothing(_: &mut Vm, _: Args) -> Run<Value> {
    Ok(Value::UNDEFINED)
}

fn get_random_values(vm: &mut Vm, args: Args) -> Run<Value> {
    let target = vm.arg(&args, 0);
    let Some(data) = vm.typed_data(target) else {
        return Err(vm.type_error("getRandomValues needs a typed array"));
    };
    let (buffer, offset, byte_length) = (
        data.buffer,
        data.offset,
        vm.typed_length(data) * data.element.size(),
    );
    if byte_length > 65536 {
        return Err(vm.range_error("getRandomValues can only fill 65536 bytes"));
    }
    let mut bytes = vec![0u8; byte_length];
    if getrandom(&mut bytes).is_err() {
        return Err(vm.type_error("The system has no random numbers to give"));
    }
    if let Kind::ArrayBuffer(store) = &mut vm.objects[buffer.index()].kind {
        store.bytes[offset..offset + byte_length].copy_from_slice(&bytes);
    }
    Ok(target)
}

/// Real randomness from the system, which on Windows and everywhere else is readable through hashing a new key.
fn getrandom(out: &mut [u8]) -> Result<(), ()> {
    use std::hash::{BuildHasher, Hasher};
    // Each `RandomState` is seeded by the operating system, which is what it's asked for here.
    for chunk in out.chunks_mut(8) {
        let value = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        chunk.copy_from_slice(&value.to_le_bytes()[..chunk.len()]);
    }
    Ok(())
}

fn random_uuid(vm: &mut Vm, _: Args) -> Run<Value> {
    let mut bytes = [0u8; 16];
    let _ = getrandom(&mut bytes);
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(vm.new_str(&format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )))
}

fn text_encoder_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    let proto = vm.proto_for(&args, vm.realm.text_encoder_proto)?;
    Ok(vm.new_object_with_proto(proto, Kind::Plain))
}

fn encoding_getter(vm: &mut Vm, _: Args) -> Run<Value> {
    Ok(vm.new_str("utf-8"))
}

pub(crate) fn new_bytes(vm: &mut Vm, bytes: Vec<u8>) -> Run<Value> {
    let buffer = vm.new_array_buffer(bytes);
    let constructor = vm.get_named(Value::object(vm.global), "Uint8Array")?;
    vm.construct(constructor, &[buffer])
}

fn text_encode(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let bytes = if value.is_undefined() {
        Vec::new()
    } else {
        vm.to_rust_string(value)?.into_bytes()
    };
    new_bytes(vm, bytes)
}

fn text_decoder_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    let proto = vm.proto_for(&args, vm.realm.text_decoder_proto)?;
    Ok(vm.new_object_with_proto(proto, Kind::Plain))
}

/// The bytes of an `ArrayBuffer` or anything that views one.
pub(crate) fn bytes_of(vm: &mut Vm, value: Value) -> Run<Vec<u8>> {
    if value.is_object() {
        match &vm.objects[value.index()].kind {
            Kind::ArrayBuffer(data) => return Ok(data.bytes.clone()),
            Kind::TypedArray(data) => {
                let (buffer, offset, length) = (
                    data.buffer,
                    data.offset,
                    vm.typed_length(data) * data.element.size(),
                );
                if let Kind::ArrayBuffer(store) = &vm.objects[buffer.index()].kind {
                    return Ok(store.bytes[offset..offset + length].to_vec());
                }
            }
            Kind::DataView(data) => {
                let (buffer, offset, length) = (data.buffer, data.offset, data.length);
                if let Kind::ArrayBuffer(store) = &vm.objects[buffer.index()].kind {
                    return Ok(store.bytes[offset..offset + length].to_vec());
                }
            }
            _ => {}
        }
    }
    Err(vm.type_error("Expected an ArrayBuffer or a view of one"))
}

fn text_decode(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    if value.is_undefined() {
        return Ok(vm.new_str(""));
    }
    let bytes = bytes_of(vm, value)?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(vm.new_str(text.strip_prefix('\u{FEFF}').unwrap_or(&text)))
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn btoa(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let units = vm.to_units(value)?;
    if units.iter().any(|unit| *unit > 255) {
        return Err(vm.make_error(
            vm.realm.error_proto,
            "Invalid character: btoa only takes characters up to 255",
        ));
    }
    let bytes: Vec<u8> = units.iter().map(|unit| *unit as u8).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(BASE64[(group >> 18) as usize & 63] as char);
        out.push(BASE64[(group >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            BASE64[(group >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            BASE64[group as usize & 63] as char
        } else {
            '='
        });
    }
    Ok(vm.new_str(&out))
}

fn atob(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let text = vm.to_rust_string(value)?;
    let mut group = 0u32;
    let mut bits = 0;
    let mut out: Vec<u16> = Vec::with_capacity(text.len() * 3 / 4);
    for byte in text.bytes() {
        if byte.is_ascii_whitespace() || byte == b'=' {
            continue;
        }
        let Some(digit) = BASE64.iter().position(|found| *found == byte) else {
            return Err(vm.make_error(
                vm.realm.error_proto,
                "The string to be decoded is not correctly encoded.",
            ));
        };
        group = group << 6 | digit as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((group >> bits) & 255) as u16);
        }
    }
    Ok(vm.new_text(Text::from_units(&out)))
}

/// Copies a value the way `structuredClone` does: everything plain is new, and cycles stay cycles.
fn clone_value(vm: &mut Vm, value: Value, seen: &mut Vec<(Value, Value)>) -> Run<Value> {
    if !value.is_object() {
        return Ok(value);
    }
    if let Some((_, copy)) = seen.iter().find(|(from, _)| *from == value) {
        return Ok(*copy);
    }
    if vm.is_callable(value) {
        return Err(vm.make_error(vm.realm.error_proto, "A function could not be cloned"));
    }
    match &vm.objects[value.index()].kind {
        Kind::Array(_) => {
            let items = vm.array_items(value);
            let copy = vm.new_array(vec![Value::UNDEFINED; items.len()]);
            seen.push((value, copy));
            for (index, item) in items.into_iter().enumerate() {
                let inner = clone_value(vm, item, seen)?;
                vm.set_index(copy, index, inner)?;
            }
            Ok(copy)
        }
        Kind::Date(time) => {
            let time = *time;
            let proto = vm.realm.date_proto;
            Ok(vm.new_object_with_proto(proto, Kind::Date(time)))
        }
        Kind::Boxed(inner) => {
            let inner = *inner;
            vm.to_object(inner)
        }
        Kind::Map(data) => {
            let entries = data.entries.clone();
            let constructor = vm.get_named(Value::object(vm.global), "Map")?;
            let copy = vm.construct(constructor, &[])?;
            seen.push((value, copy));
            let set = vm.get(copy, text::SET)?;
            for (key, item) in entries {
                if !key.is_empty() {
                    let new_key = clone_value(vm, key, seen)?;
                    let new_item = clone_value(vm, item, seen)?;
                    vm.call(set, copy, &[new_key, new_item])?;
                }
            }
            Ok(copy)
        }
        Kind::Set(data) => {
            let entries = data.entries.clone();
            let constructor = vm.get_named(Value::object(vm.global), "Set")?;
            let copy = vm.construct(constructor, &[])?;
            seen.push((value, copy));
            let add = vm.get_named(copy, "add")?;
            for (key, _) in entries {
                if !key.is_empty() {
                    let new_key = clone_value(vm, key, seen)?;
                    vm.call(add, copy, &[new_key])?;
                }
            }
            Ok(copy)
        }
        Kind::ArrayBuffer(data) => {
            if data.detached {
                return Err(vm.type_error("A detached ArrayBuffer could not be cloned"));
            }
            let bytes = data.bytes.clone();
            let copy = vm.new_array_buffer(bytes);
            seen.push((value, copy));
            Ok(copy)
        }
        // A view is a new view of the copy of its buffer, so that views of the same buffer still share one.
        Kind::TypedArray(data) => {
            let (name, buffer, offset) = (data.element.constructor_name(), data.buffer, data.offset);
            let length = (!data.follows_buffer).then(|| vm.typed_length(data));
            clone_view(vm, value, name, buffer, offset, length, seen)
        }
        Kind::DataView(data) => {
            let (buffer, offset) = (data.buffer, data.offset);
            let length = (!data.follows_buffer).then_some(data.length);
            clone_view(vm, value, "DataView", buffer, offset, length, seen)
        }
        _ => {
            let copy = vm.new_plain();
            seen.push((value, copy));
            for name in vm.own_enumerable_names(value.handle()) {
                let item = vm.get(value, name)?;
                let inner = clone_value(vm, item, seen)?;
                vm.define(copy.handle(), name, inner, PLAIN);
            }
            Ok(copy)
        }
    }
}

fn clone_view(
    vm: &mut Vm,
    value: Value,
    name: &str,
    buffer: Value,
    offset: usize,
    length: Option<usize>,
    seen: &mut Vec<(Value, Value)>,
) -> Run<Value> {
    let buffer = clone_value(vm, buffer, seen)?;
    let constructor = vm.get_named(Value::object(vm.global), name)?;
    let mut given = vec![buffer, Value::number(offset as f64)];
    given.extend(length.map(|length| Value::number(length as f64)));
    let copy = vm.construct(constructor, &given)?;
    seen.push((value, copy));
    Ok(copy)
}

/// `structuredClone(value, { transfer })`. Each buffer in `transfer` moves to the copy, and the one it came from is
/// left detached, as it would be if it had been sent somewhere.
fn structured_clone(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let options = vm.arg(&args, 1);
    let mut seen = Vec::new();
    let mut transferred = Vec::new();
    if options.is_object() {
        let list = vm.get_named(options, "transfer")?;
        if list.is_object() {
            for buffer in vm.array_items(list) {
                let bytes = match buffer.is_object().then(|| &vm.objects[buffer.index()].kind) {
                    Some(Kind::ArrayBuffer(data)) if !data.detached => data.bytes.clone(),
                    _ => {
                        return Err(vm.type_error(
                            "Only ArrayBuffers that aren't detached can be transferred",
                        ));
                    }
                };
                let moved = vm.new_array_buffer(bytes);
                seen.push((buffer, moved));
                transferred.push(buffer);
            }
        }
    }
    let copy = clone_value(vm, value, &mut seen)?;
    // The buffers are only detached once everything's been copied, as views of them need to know how long they are.
    for buffer in transferred {
        if let Kind::ArrayBuffer(data) = &mut vm.objects[buffer.index()].kind {
            data.detached = true;
            data.bytes = Vec::new();
        }
    }
    Ok(copy)
}

pub(super) fn install(vm: &mut Vm) {
    let process = vm.plain_proto();
    let atom = vm.intern_str("process");
    vm.add_property(vm.global, atom, Value::object(process), HIDDEN);
    let env = vm.new_plain();
    let env_atom = vm.intern_str("env");
    let env_get = vm.new_native(env_atom, 0, process_env_get, Value::UNDEFINED, false);
    let env_set = vm.new_native(env_atom, 1, process_env_set, Value::UNDEFINED, false);
    vm.define_accessor(
        process,
        env_atom,
        Some(env_get),
        Some(env_set),
        CONFIGURABLE,
    );
    vm.realm.process_env = env.handle();
    // Until something looks at `process.env`, this is all that's holding on to what's in it.
    let kept_atom = vm.intern_str("_environment");
    vm.add_property(process, kept_atom, env, HIDDEN);
    // `Object.prototype.toString` calls it `[object process]`, which is how packages such as axios know they're in Node.
    let tag = vm.new_str("process");
    vm.add_property(process, text::SYMBOL_TO_STRING_TAG, tag, CONFIGURABLE);
    // What Node says on each: macOS is `darwin`, and Android is `android`, though it's Linux underneath.
    let platform = vm.new_str(if cfg!(windows) {
        "win32"
    } else if cfg!(target_vendor = "apple") {
        "darwin"
    } else if cfg!(target_os = "android") {
        "android"
    } else {
        "linux"
    });
    let platform_atom = vm.intern_str("platform");
    vm.add_property(process, platform_atom, platform, PLAIN);
    let version = vm.new_str("v22.0.0");
    let version_atom = vm.intern_str("version");
    vm.add_property(process, version_atom, version, PLAIN);
    let versions = vm.new_plain();
    let versions_atom = vm.intern_str("versions");
    vm.add_property(process, versions_atom, versions, PLAIN);
    // Packages look at this to know what they can use, and this has what that version has that they look for.
    let node_version = vm.new_str("22.0.0");
    let node_atom = vm.intern_str("node");
    vm.add_property(versions.handle(), node_atom, node_version, PLAIN);
    let release = vm.new_plain();
    let release_atom = vm.intern_str("release");
    vm.add_property(process, release_atom, release, PLAIN);
    let release_name = vm.new_str("node");
    let name_atom = vm.intern_str("name");
    vm.add_property(release.handle(), name_atom, release_name, PLAIN);
    let argv = vm.new_array(Vec::new());
    let argv_atom = vm.intern_str("argv");
    vm.add_property(process, argv_atom, argv, PLAIN);
    vm.method(process, "cwd", 0, process_cwd);
    vm.method(process, "nextTick", 1, process_next_tick);
    vm.method(process, "hrtime", 0, process_hrtime);
    vm.method(process, "on", 2, process_returns_itself);
    vm.method(process, "once", 2, process_returns_itself);
    vm.method(process, "off", 2, process_returns_itself);
    vm.method(process, "addListener", 2, process_returns_itself);
    vm.method(process, "removeListener", 2, process_returns_itself);
    vm.method(process, "removeAllListeners", 1, process_returns_itself);
    vm.method(process, "emit", 1, process_nothing_was_listening);
    vm.method(process, "emitWarning", 1, nothing);
    vm.method(process, "exit", 1, process_exit);
    vm.method(process, "uptime", 0, process_uptime);
    vm.method(process, "memoryUsage", 0, process_memory_usage);
    let arch = vm.new_str("x64");
    let arch_atom = vm.intern_str("arch");
    vm.add_property(process, arch_atom, arch, PLAIN);
    let title = vm.new_str("rsui");
    let title_atom = vm.intern_str("title");
    vm.add_property(process, title_atom, title, PLAIN);
    let exec_path = std::env::current_exe().unwrap_or_default();
    let exec_path = vm.new_str(&exec_path.to_string_lossy());
    let exec_atom = vm.intern_str("execPath");
    vm.add_property(process, exec_atom, exec_path, PLAIN);
    let pid_atom = vm.intern_str("pid");
    vm.add_property(
        process,
        pid_atom,
        Value::number(f64::from(std::process::id())),
        PLAIN,
    );
    let exit_code_atom = vm.intern_str("exitCode");
    vm.add_property(process, exit_code_atom, Value::UNDEFINED, PLAIN);
    for (name, getter, setter) in [
        (
            "stdout",
            process_stdout as NativeFn,
            process_stream_set as NativeFn,
        ),
        ("stderr", process_stderr, process_stream_set),
        ("stdin", process_stdin, process_stream_set),
    ] {
        let atom = vm.intern_str(name);
        let get = vm.new_native(atom, 0, getter, Value::UNDEFINED, false);
        let set = vm.new_native(atom, 1, setter, Value::UNDEFINED, false);
        vm.define_accessor(process, atom, Some(get), Some(set), CONFIGURABLE);
    }

    vm.method(vm.global, "fetch", 2, first_fetch);
    install_lazy_globals(vm);

    let global_atom = vm.intern_str("global");
    vm.add_property(vm.global, global_atom, Value::object(vm.global), HIDDEN);

    let performance = vm.plain_proto();
    let atom = vm.intern_str("performance");
    vm.add_property(vm.global, atom, Value::object(performance), HIDDEN);
    vm.method(performance, "now", 0, performance_now);
    let atom = vm.intern_str("timeOrigin");
    vm.add_property(performance, atom, Value::number(time_origin()), HIDDEN);

    let crypto = vm.plain_proto();
    let atom = vm.intern_str("crypto");
    vm.add_property(vm.global, atom, Value::object(crypto), HIDDEN);
    vm.method(crypto, "getRandomValues", 1, get_random_values);
    vm.method(crypto, "randomUUID", 0, random_uuid);

    let encoder = vm.plain_proto();
    vm.realm.text_encoder_proto = encoder;
    vm.constructor("TextEncoder", 0, text_encoder_constructor, encoder);
    vm.method(encoder, "encode", 1, text_encode);
    vm.getter(encoder, "encoding", encoding_getter);
    let decoder = vm.plain_proto();
    vm.realm.text_decoder_proto = decoder;
    vm.constructor("TextDecoder", 0, text_decoder_constructor, decoder);
    vm.method(decoder, "decode", 1, text_decode);
    vm.getter(decoder, "encoding", encoding_getter);

    vm.global_function("btoa", 1, btoa);
    vm.global_function("atob", 1, atob);
    vm.global_function("structuredClone", 1, structured_clone);
}
