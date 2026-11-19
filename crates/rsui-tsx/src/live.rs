//! Looking at an app while it runs, from the viewer in another program: `rsui-tsx --view` with no file.
//!
//! An app that's being worked on listens on a named pipe for that, which only it and its user can write to, and says it's
//! there with a file in `%TEMP%\rsui-live` that goes when it ends. Listening is a thread waiting, and nothing's kept or
//! timed until a viewer asks, when the app starts keeping what ctrl+shift+I would. A packed app does none of this.
//!
//! A request and its answer are each a line of JSON, and what's asked is done on the app's own thread, at its next update:
//!
//! - `{"ask":"hello"}`: which app this is.
//! - `{"ask":"news","after":FRAME,"heard":LINE,"spans":SPAN,"labels":LABEL,"stacks":STACK}`: the frames kept after FRAME,
//!   every change of state kept and which one things are as they were after, what's been said after LINE, and the
//!   logpoints there are. With `spans`, the timed spans kept from number SPAN on (see `rsui-spans`), as `spans.recs`, six
//!   numbers each: kind, thread, start and length in nanoseconds, label and a number that goes with it, with the labels
//!   from LABEL on and the stacks of labels (what JavaScript was sampled doing) from STACK on. `close` says the viewer's
//!   been asked to go, as F12 in the app does when it's open.
//! - `{"ask":"record","on":true}`: starts sampling where JavaScript is, about every millisecond, or stops.
//! - `{"ask":"pick","on":true}`: has the next click on the app pick what it's on, rather than reach the app, which `news`
//!   says as `picked`, with what the pointer's on filled in until then.
//! - `{"ask":"heat","cells":[[PATH,HEAT,"LABEL"],..]}`: draws where time went over the app, each element as hot as HEAT
//!   says, from 0 to 1, or nothing with no cells.
//! - `{"ask":"goto","change":N}`: puts state back to how it was after change N.
//! - `{"ask":"logpoint","at":"FILE:LINE","expression":"EXPRESSION"}`, or with no expression to take it away.
//! - `{"ask":"show","on":true}`: shows the inspector in the app's window, or hides it.
//! - `{"ask":"summary"}`: what `profile` says about the frames kept.
//! - `{"ask":"save"}`: saves a trace next to the app.
//! - `{"ask":"breakpoint","at":"FILE:LINE"}` adds a breakpoint, and with `"take":true` takes it away. The first one starts
//!   debugging (see `debugger.rs`).
//! - `{"ask":"pause"}`: stops at the next statement of the app's own that runs.
//! - `{"ask":"inputs","after":N}`: what's come in to the app from input N on, which `replay` can go back to.
//! - `{"ask":"tree"}`: what the app's window has in it, each element with its path, tag, props and where it is.
//! - `{"ask":"element","path":[..]}`: the component that made it, what that's in, why it rendered and its hooks.
//! - `{"ask":"highlight","path":[..],"name":"..."}`: shows it filled in in the app's window, or nothing with no path.
//! - `{"ask":"evaluate","expression":"..."}`: works it out as a browser's console does.
//! - `{"ask":"dock","pid":N,"side":"right","size":480}`: keeps process N's window inside the app's, along its right or
//!   `bottom`, as a browser keeps its inspector, or with no side gives it back.
//! - `{"ask":"replay","to":N,"pause":true}`: starts the app again in a new process, replaying what came in up to input
//!   N, and stopping at the first statement that runs for it with `pause`. Says the new process's `pid`.
//!
//! While the app's stopped at a breakpoint, its thread waits here for what the viewer says, and `news` says where it's
//! stopped (`paused`), with what can be done there:
//!
//! - `{"ask":"evaluate","frame":N,"expression":"EXPRESSION"}`: works it out in the Nth function out from where it's
//!   stopped.
//! - `{"ask":"resume","how":"continue"}`, or `over`, `into` or `out` for a step.
//!
//! It carries on by itself if every viewer goes. An answer that went wrong is `{"error":"..."}`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rsui::App;
use serde_json::{Value, json};

/// How long a request waits for the app to get to it, which a busy app might not for a while.
const ANSWER_WITHIN: Duration = Duration::from_secs(5);
/// The most frames one answer has, the newest, so a viewer that's been away doesn't get them all at once.
const MOST_FRAMES: usize = 600;
/// How many lines of what's been said are kept for a viewer to ask about.
const MOST_HEARD: usize = 1000;
/// The most spans one answer has, the oldest of those asked for, so one that's been away catches up over a few answers.
const MOST_SPANS: usize = 40_000;
/// How often JavaScript is sampled while a viewer's recording.
const SAMPLE_EVERY: Duration = Duration::from_millis(1);

/// Whether the viewer F12 opened has been asked to go, by F12 again.
static CLOSE_ASKED: AtomicBool = AtomicBool::new(false);
/// The viewer F12 opened, while it's running.
static OPENED: Mutex<Option<std::process::Child>> = Mutex::new(None);

/// Opens the viewer docked to the app's window, as F12 and the pill do, or asks the one that's open to go.
pub(crate) fn open_viewer(app: &mut App) {
    if !listening() {
        // A packed app has nothing for a viewer to look at, so it has what's drawn over it instead.
        app.show_inspector(!app.inspector_showing());
        return;
    }
    let mut opened = OPENED.lock().expect("lock is not poisoned");
    let running = opened
        .as_mut()
        .is_some_and(|child| matches!(child.try_wait(), Ok(None)));
    if running || viewer_looking() {
        // F12 again puts it away, as a browser's does: it's told with its next news, gives its window back and goes.
        CLOSE_ASKED.store(true, Ordering::Relaxed);
        app.dock(None);
        *opened = None;
        return;
    }
    let Ok(program) = std::env::current_exe() else {
        return;
    };
    let mut command = std::process::Command::new(program);
    command
        .arg("--view")
        .arg(std::process::id().to_string())
        .env(crate::trace_file::DOCK_VAR, "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    {
        use std::os::windows::process::CommandExt;
        // It has a window of its own, and no console.
        command.creation_flags(0x0800_0000);
    }
    CLOSE_ASKED.store(false, Ordering::Relaxed);
    match command.spawn() {
        Ok(child) => *opened = Some(child),
        Err(error) => eprintln!("rsui-tsx: the inspector couldn't be started: {error}"),
    }
}

const PIPE_ACCESS_DUPLEX: u32 = 3;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const PIPE_BUFFER: u32 = 64 * 1024;
const ERROR_PIPE_CONNECTED: i32 = 535;
const INVALID: isize = -1;

#[allow(unsafe_code)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        instances: u32,
        out_buffer: u32,
        in_buffer: u32,
        timeout: u32,
        security: *const c_void,
    ) -> isize;
    fn ConnectNamedPipe(pipe: isize, overlapped: *mut c_void) -> i32;
    fn CloseHandle(handle: isize) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
    fn GetExitCodeProcess(process: isize, code: *mut u32) -> i32;
}

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const STILL_ACTIVE: u32 = 259;

/// What a viewer's told this app is, and the file that says it's there.
struct Listening {
    about: Mutex<Value>,
    file: PathBuf,
    /// What has the app's thread stop, which works even while it's too busy to get to what's asked.
    pause: rsui_js::PauseHandle,
}

static LISTENING: OnceLock<Listening> = OnceLock::new();

/// How many viewers are connected.
static VIEWERS: AtomicUsize = AtomicUsize::new(0);

/// A request, and where its answer goes.
type Asked = (Value, Sender<Value>);

/// Where requests go while the app's stopped at a breakpoint, which its thread is waiting for.
static PAUSED: Mutex<Option<Sender<Asked>>> = Mutex::new(None);

thread_local! {
    /// What's been said, by `console.log` or by rsui, numbered from 1, while listening.
    static HEARD: RefCell<Option<Heard>> = const { RefCell::new(None) };
}

#[derive(Default)]
struct Heard {
    said: u64,
    lines: VecDeque<(u64, String)>,
}

/// Where apps say they're there to be looked at.
pub(crate) fn folder() -> PathBuf {
    std::env::temp_dir().join("rsui-live")
}

/// Starts listening for a viewer, for the app in `main_file`, called `title`. It's to be called on the app's thread, which
/// is where what's said is heard.
pub(crate) fn start(main_file: &Path, title: &str) {
    let pid = std::process::id();
    let pipe = format!(r"\\.\pipe\rsui-live-{pid}");
    let file = folder().join(format!("{pid}.json"));
    let about = json!({
        "pid": pid,
        "pipe": pipe,
        "app": title,
        "file": std::path::absolute(main_file).unwrap_or_else(|_| main_file.to_owned()),
        "started": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs()),
    });
    let Some(first) = make_pipe(&pipe, true) else {
        // Something else has the name, which is only this process's, so there's nothing to do but go without.
        return;
    };
    if LISTENING
        .set(Listening {
            about: Mutex::new(about.clone()),
            file: file.clone(),
            pause: rsui_js::pause_handle(),
        })
        .is_err()
    {
        close(first);
        return;
    }
    let _ = std::fs::create_dir_all(folder());
    let _ = std::fs::write(&file, about.to_string());
    HEARD.with(|heard| *heard.borrow_mut() = Some(Heard::default()));
    let spawned = std::thread::Builder::new()
        .name("rsui-tsx live".to_owned())
        .stack_size(64 * 1024)
        .spawn(move || listen(&pipe, first));
    if spawned.is_err() {
        stop();
    }
}

/// Takes away what's said about apps that ended without saying so, as one that crashed does.
pub(crate) fn forget_gone() {
    let Ok(entries) = std::fs::read_dir(folder()) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let pid = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<u32>().ok());
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
            && pid.is_some_and(|pid| !running(pid))
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Whether a process is still running.
#[allow(unsafe_code)]
fn running(pid: u32) -> bool {
    // SAFETY: the handle, if there is one, is closed before this returns, and `code` outlives the call.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process == 0 {
            // It's not there, or it's not this user's, which isn't one to look at either.
            return false;
        }
        let mut code = 0;
        let asked = GetExitCodeProcess(process, &raw mut code);
        CloseHandle(process);
        asked == 0 || code == STILL_ACTIVE
    }
}

/// Says what the app's called, once that's known from its main file.
pub(crate) fn named(title: &str) {
    if let Some(listening) = LISTENING.get() {
        let mut about = listening.about.lock().expect("lock is not poisoned");
        about["app"] = json!(title);
        let _ = std::fs::write(&listening.file, about.to_string());
    }
}

/// Says the app's no longer there to be looked at, as it's ending.
pub(crate) fn stop() {
    if let Some(listening) = LISTENING.get() {
        let _ = std::fs::remove_file(&listening.file);
    }
}

/// Whether a viewer can look at this app.
pub(crate) fn listening() -> bool {
    LISTENING.get().is_some()
}

/// Keeps what's been said, for a viewer to ask about, if one can.
pub(crate) fn heard(text: &str) {
    HEARD.with(|heard| {
        let mut heard = heard.borrow_mut();
        let Some(heard) = heard.as_mut() else {
            return;
        };
        for line in text.lines() {
            heard.said += 1;
            heard.lines.push_back((heard.said, line.to_owned()));
        }
        while heard.lines.len() > MOST_HEARD {
            heard.lines.pop_front();
        }
    });
}

fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(Some(0))
        .collect()
}

/// A new end of the pipe for the next viewer to connect to. The first is made only if nothing has the name already.
#[allow(unsafe_code)]
fn make_pipe(name: &str, first: bool) -> Option<isize> {
    let name = wide(name);
    let open_mode = PIPE_ACCESS_DUPLEX
        | if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    // SAFETY: `name` ends in a nul, and there's no security asked for, which leaves the pipe to its maker and the system.
    let pipe = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            std::ptr::null(),
        )
    };
    (pipe != INVALID).then_some(pipe)
}

#[allow(unsafe_code)]
fn close(pipe: isize) {
    // SAFETY: it's a handle this made and nothing else has.
    unsafe {
        CloseHandle(pipe);
    }
}

/// Whether a viewer is connected, which a breakpoint waits for to say when to carry on.
pub(crate) fn viewer_looking() -> bool {
    VIEWERS.load(Ordering::Relaxed) > 0
}

/// Waits on the app's thread, which is stopped at a breakpoint, for what the viewers ask, until one says to carry on or
/// they've all gone. `shown` is where it's stopped and what's there, as `news` says it.
pub(crate) fn wait_while_paused(vm: &mut rsui_js::Vm, shown: Value) -> rsui_js::Resume {
    let (send, asked): (Sender<Asked>, Receiver<Asked>) = std::sync::mpsc::channel();
    *PAUSED.lock().expect("lock is not poisoned") = Some(send);
    let resume = loop {
        match asked.recv_timeout(Duration::from_millis(500)) {
            Ok((request, reply)) => {
                let (answer, resume) = answer_paused(vm, &shown, &request);
                let _ = reply.send(answer);
                if let Some(resume) = resume {
                    break resume;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) if viewer_looking() => {}
            Err(_) => break rsui_js::Resume::Continue,
        }
    };
    *PAUSED.lock().expect("lock is not poisoned") = None;
    resume
}

/// What the app says to a request while it's stopped, and how to carry on if that's what was asked.
fn answer_paused(
    vm: &mut rsui_js::Vm,
    shown: &Value,
    request: &Value,
) -> (Value, Option<rsui_js::Resume>) {
    let stopped_at = shown["at"].as_str().unwrap_or_default();
    let said = match request["ask"].as_str().unwrap_or_default() {
        "hello" => Ok(LISTENING.get().map_or(Value::Null, |listening| {
            listening
                .about
                .lock()
                .expect("lock is not poisoned")
                .clone()
        })),
        "news" => {
            let mut news = heard_news(request);
            news["paused"] = shown.clone();
            news["frames"] = json!([]);
            news["history"] = json!(crate::trace_file::history_json());
            news["now"] = json!(crate::why::history_now());
            Ok(news)
        }
        "evaluate" => {
            let frame = request["frame"].as_u64().unwrap_or(0) as usize;
            let expression = request["expression"].as_str().unwrap_or_default();
            Ok(match crate::debugger::evaluate(vm, frame, expression) {
                Ok(value) => json!({ "value": value }),
                Err(threw) => json!({ "value": threw, "threw": true }),
            })
        }
        "resume" => {
            let how = request["how"].as_str().unwrap_or("continue");
            return match crate::debugger::resume_of(how) {
                Some(resume) => (
                    json!({ "said": format!("Carried on from {stopped_at}.") }),
                    Some(resume),
                ),
                None => (
                    json!({ "error": format!("there's no {how:?} way to carry on. It's continue, over, into or out") }),
                    None,
                ),
            };
        }
        "breakpoint" => breakpoint(request),
        "pause" => Ok(json!({ "said": format!("It's stopped at {stopped_at} already.") })),
        "" => Err("say what's asked, as in {\"ask\":\"news\"}".to_owned()),
        other => Err(format!(
            "the app's stopped at {stopped_at}, so it can't do {other:?} until it carries on"
        )),
    };
    (said.unwrap_or_else(|error| json!({ "error": error })), None)
}

/// Waits for each viewer in turn, and serves each on a thread of its own.
#[allow(unsafe_code)]
fn listen(name: &str, first: isize) {
    let mut next = Some(first);
    loop {
        let Some(pipe) = next.take().or_else(|| make_pipe(name, false)) else {
            return;
        };
        // SAFETY: `pipe` is a pipe this made, waited on without overlapping.
        let connected = unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) } != 0
            || std::io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_CONNECTED);
        if !connected {
            close(pipe);
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        // SAFETY: the handle is this pipe's, and the file is all that has it from here.
        let viewer = unsafe { File::from_raw_handle(pipe as *mut c_void) };
        let _ = std::thread::Builder::new()
            .name("rsui-tsx viewer".to_owned())
            .stack_size(256 * 1024)
            .spawn(move || serve(&viewer));
    }
}

/// Answers what one viewer asks, until it goes.
fn serve(viewer: &File) {
    struct Gone;
    impl Drop for Gone {
        fn drop(&mut self) {
            VIEWERS.fetch_sub(1, Ordering::Relaxed);
        }
    }
    VIEWERS.fetch_add(1, Ordering::Relaxed);
    let _gone = Gone;
    let mut reader = BufReader::new(viewer);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(request) => {
                // An app that's busy running its own code stops at its next statement, if it's being debugged, without
                // waiting to get to what's asked.
                if request["ask"] == "pause" {
                    if let Some(listening) = LISTENING.get() {
                        listening.pause.pause();
                    }
                }
                ask_app(request)
            }
            Err(error) => json!({ "error": format!("that isn't JSON: {error}") }),
        };
        let mut writer = viewer;
        let mut text = answer.to_string();
        text.push('\n');
        if writer.write_all(text.as_bytes()).is_err() {
            return;
        }
    }
}

/// Hands a request to where the app's stopped, if it is, and waits for the answer.
fn ask_paused(request: &Value, send: &Sender<Value>, receive: &Receiver<Value>) -> Option<Value> {
    let paused = PAUSED.lock().expect("lock is not poisoned").clone()?;
    paused.send((request.clone(), send.clone())).ok()?;
    Some(receive.recv_timeout(ANSWER_WITHIN).unwrap_or_else(
        |_| json!({ "error": "the app hasn't answered while it's stopped" }),
    ))
}

/// Has the app answer a request on its own thread: at its next update, or straight away if it's stopped at a breakpoint,
/// when its thread is waiting for what's asked.
fn ask_app(request: Value) -> Value {
    let (send, receive) = std::sync::mpsc::channel();
    if let Some(answer) = ask_paused(&request, &send, &receive) {
        return answer;
    }
    // It can stop at a breakpoint while this waits, and then the request goes to where it's stopped instead, and isn't
    // done again once it carries on.
    let taken = Arc::new(AtomicBool::new(false));
    let not_yet = taken.clone();
    let wanted = request.clone();
    let answer_to = send.clone();
    rsui::inspect::run_in_app(Box::new(move |app| {
        if !not_yet.swap(true, Ordering::Relaxed) {
            let _ = answer_to.send(answer(app, &wanted));
        }
    }));
    let waited = std::time::Instant::now();
    while waited.elapsed() < ANSWER_WITHIN {
        match receive.recv_timeout(Duration::from_millis(50)) {
            Ok(answer) => return answer,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        if PAUSED.lock().expect("lock is not poisoned").is_some() && !taken.swap(true, Ordering::Relaxed) {
            if let Some(answer) = ask_paused(&request, &send, &receive) {
                return answer;
            }
        }
    }
    json!({ "error": "the app hasn't answered. It's busy, or it has no window yet" })
}

/// What the app says to a request.
pub(crate) fn answer(app: &mut App, request: &Value) -> Value {
    let said = match request["ask"].as_str().unwrap_or_default() {
        "hello" => Ok(LISTENING.get().map_or(Value::Null, |listening| {
            listening
                .about
                .lock()
                .expect("lock is not poisoned")
                .clone()
        })),
        "news" => Ok(news(app, request)),
        "goto" => match request["change"].as_u64() {
            Some(change) => go_to(app, change as u32),
            None => Err("say which change to go to".to_owned()),
        },
        "logpoint" => logpoint(
            request["at"].as_str().unwrap_or_default(),
            request["expression"].as_str(),
        ),
        "show" => {
            app.show_inspector(request["on"].as_bool().unwrap_or(true));
            Ok(json!({ "showing": app.inspector_showing() }))
        }
        "summary" => Ok(json!({ "summary": rsui::summary(app.frames()) })),
        "record" => {
            let on = request["on"].as_bool().unwrap_or(true);
            let recording = crate::bridge::with_engine(|engine| {
                if on {
                    engine.start_sampling(SAMPLE_EVERY);
                } else {
                    engine.stop_sampling();
                }
                engine.sampling()
            });
            match recording {
                Some(recording) => Ok(json!({ "recording": recording })),
                None => Err("the app's in the middle of running its code".to_owned()),
            }
        }
        "pick" => {
            app.set_picking(request["on"].as_bool().unwrap_or(true));
            Ok(json!({ "picking": app.picking() }))
        }
        "heat" => {
            let cells = request["cells"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|cell| {
                    Some((
                        path_of(cell.get(0)?),
                        cell.get(1)?.as_f64()? as f32,
                        cell.get(2).and_then(Value::as_str).unwrap_or_default().to_owned(),
                    ))
                })
                .collect();
            app.set_heat(cells);
            Ok(json!({}))
        }
        "breakpoint" => breakpoint(request),
        "tree" => Ok(tree(app)),
        "element" => {
            let path = path_of(&request["path"]);
            let made = crate::drive::made_by(app, &path);
            let rect = app.layout().rect(&path);
            Ok(json!({
                "component": made.name,
                "around": made.around,
                "why": made.why,
                "hooks": made.hooks,
                "rect": rect.map(|rect| [rect.x, rect.y, rect.width, rect.height]),
                "inspecting": app.inspecting(),
            }))
        }
        "dock" => {
            let side = match request["side"].as_str() {
                Some("right") => Some(rsui::DockSide::Right),
                Some("bottom") => Some(rsui::DockSide::Bottom),
                _ => None,
            };
            let pid = request["pid"].as_u64().unwrap_or(0) as u32;
            let size = request["size"].as_f64().unwrap_or(480.0) as f32;
            app.dock(side.map(|side| rsui::DockRequest { pid, side, size }));
            Ok(json!({ "docked": request["side"] }))
        }
        "highlight" => {
            let at = request["path"].as_array().map(|_| {
                (
                    path_of(&request["path"]),
                    request["name"].as_str().unwrap_or_default().to_owned(),
                )
            });
            app.highlight(at);
            Ok(json!({}))
        }
        "evaluate" => Ok(match crate::bridge::evaluate(request["expression"].as_str().unwrap_or_default()) {
            Ok(value) => json!({ "value": value }),
            Err(threw) => json!({ "value": threw, "threw": true }),
        }),
        "pause" => {
            crate::debugger::pause();
            Ok(json!({ "said": "It stops at the next statement of its own that runs." }))
        }
        "inputs" => Ok(crate::replay::listed(
            app,
            request["after"].as_u64().unwrap_or(0) as usize,
        )),
        "replay" => {
            let to = request["to"].as_u64().unwrap_or(0) as usize;
            let pause = request["pause"].as_bool().unwrap_or(false);
            crate::replay::again(app, to, pause).map(|pid| {
                json!({
                    "pid": pid,
                    "said": format!(
                        "Started it again as process {pid}, replaying what came in up to input {to}{}.",
                        if pause { ", and stopping in what that input does" } else { "" }
                    ),
                })
            })
        }
        "save" => crate::trace_file::save_beside_app(app)
            .map(|path| json!({ "said": format!("Saved in {}.", path.display()), "path": path })),
        "" => Err("say what's asked, as in {\"ask\":\"news\"}".to_owned()),
        other => Err(format!("there's no {other:?} to ask")),
    };
    said.unwrap_or_else(|error| json!({ "error": error }))
}

/// What's happened since the viewer last asked. Asking starts the app keeping it, as it would for ctrl+shift+I.
fn news(app: &mut App, request: &Value) -> Value {
    let started = !app.inspecting();
    app.set_inspecting(true);
    let after = request["after"].as_u64().unwrap_or(0);
    let newer: Vec<_> = app
        .frames()
        .rev()
        .take_while(|frame| frame.number > after)
        .collect();
    let frames: Vec<Value> = newer
        .iter()
        .take(MOST_FRAMES)
        .rev()
        .map(|frame| crate::trace_file::frame_json(frame))
        .collect();
    let mut news = heard_news(request);
    news["started"] = json!(started);
    news["frames"] = json!(frames);
    news["history"] = json!(crate::trace_file::history_json());
    news["now"] = json!(crate::why::history_now());
    news["showing"] = json!(app.inspector_showing());
    news["inputs"] = json!(app.recorded_inputs().0.len());
    news["close"] = json!(CLOSE_ASKED.swap(false, Ordering::Relaxed));
    news["picking"] = json!(app.picking());
    if let Some(picked) = app.take_picked() {
        news["picked"] = json!(picked);
    }
    if request.get("spans").is_some() {
        news["spans"] = spans_news(request, MOST_SPANS);
    }
    news
}

/// The spans kept from what the viewer asks for on, at most `most` of them, with the labels and stacks it doesn't have yet.
pub(crate) fn spans_news(request: &Value, most: usize) -> Value {
    let recording = crate::bridge::with_engine(|engine| {
        engine.take_samples();
        engine.sampling()
    })
    .unwrap_or(false);
    let after = request["spans"].as_u64().unwrap_or(0);
    let (recs, next) = rsui_spans::since(after);
    let skipped = recs.len().saturating_sub(most);
    let given = recs.len() - skipped;
    // The oldest of what's asked for, so the next answer goes on from where this one stops.
    let mut flat = Vec::with_capacity(given * 6);
    for rec in recs.iter().take(most) {
        flat.extend([
            u64::from(rec.kind),
            u64::from(rec.thread),
            rec.start,
            rec.end.saturating_sub(rec.start),
            u64::from(rec.label),
            u64::from(rec.arg),
        ]);
    }
    let next = next - skipped as u64;
    let labels_from = request["labels"].as_u64().unwrap_or(0) as usize;
    let stacks_from = request["stacks"].as_u64().unwrap_or(0) as usize;
    let mut said = json!({
        "next": next,
        "recs": flat,
        "labels": rsui_spans::labels_from(labels_from),
        "stacks": rsui_spans::stacks_from(stacks_from),
        "now": rsui_spans::now(),
        "threads": rsui_spans::threads(),
        "level": rsui_spans::level() as u8,
        "recording": recording,
        "dropped": rsui_spans::dropped(),
    });
    if after == 0 {
        said["kinds"] = rsui_spans::KINDS
            .iter()
            .map(|(name, category, _, about)| json!([name, category.name(), about]))
            .collect();
    }
    said
}

/// What `news` says whether or not the app's stopped: what's been said after the line the viewer's had, the logpoints and
/// the breakpoints.
fn heard_news(request: &Value) -> Value {
    let heard_after = request["heard"].as_u64().unwrap_or(0);
    let (heard, said) = HEARD.with(|heard| {
        let heard = heard.borrow();
        let Some(heard) = heard.as_ref() else {
            return (Vec::new(), 0);
        };
        let lines: Vec<Value> = heard
            .lines
            .iter()
            .filter(|(number, _)| *number > heard_after)
            .map(|(number, line)| json!({ "number": number, "line": line }))
            .collect();
        (lines, heard.said)
    });
    let logpoints: Vec<Value> = crate::logpoint::entries()
        .into_iter()
        .map(|(at, expression, hits)| json!({ "at": at, "expression": expression, "hits": hits }))
        .collect();
    let breakpoints: Vec<Value> = crate::debugger::entries()
        .into_iter()
        .map(|(at, hits)| json!({ "at": at, "hits": hits }))
        .collect();
    json!({
        "heard": heard,
        "said": said,
        "logpoints": logpoints,
        "breakpoints": breakpoints,
        "debugging": crate::debugger::debugging(),
        "paused": null,
    })
}

/// The most elements `tree` says, which is more than a window shows at once.
const MOST_ELEMENTS: usize = 6000;

fn path_of(value: &Value) -> Vec<u32> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|step| step.as_u64().map(|step| step as u32))
        .collect()
}

/// What the app's window has in it, as the viewer's Elements shows it.
fn tree(app: &App) -> Value {
    fn element(
        node: &rsui::core::Node,
        path: &mut Vec<u32>,
        layout: &rsui::layout::Layout,
        left: &mut usize,
    ) -> Value {
        *left = left.saturating_sub(1);
        let props: serde_json::Map<String, Value> = node
            .props
            .iter()
            .map(|(name, value)| {
                let shown = match value {
                    rsui::core::PropValue::Bool(on) => json!(on),
                    rsui::core::PropValue::Int(number) => json!(number),
                    rsui::core::PropValue::Float(number) => json!((number * 100.0).round() / 100.0),
                    rsui::core::PropValue::Str(text) => json!(text.to_string()),
                };
                (name.as_str().to_owned(), shown)
            })
            .collect();
        let rect = layout.rect(path);
        let mut children = Vec::new();
        for (index, child) in node.children.iter().enumerate() {
            if *left == 0 {
                break;
            }
            path.push(index as u32);
            children.push(element(child, path, layout, left));
            path.pop();
        }
        json!({
            "path": path.clone(),
            "tag": node.role.name(),
            "key": node.key.as_ref().map(ToString::to_string),
            "name": node.name.to_string(),
            "props": props,
            "actions": node.actions.iter().map(|action| action.name()).collect::<Vec<_>>(),
            "rect": rect.map(|rect| [rect.x, rect.y, rect.width, rect.height]),
            "children": children,
            "more": node.children.len() - children.len(),
        })
    }
    let mut left = MOST_ELEMENTS;
    let layout = app.layout();
    let root = element(app.runtime().root(), &mut Vec::new(), &layout, &mut left);
    json!({ "tree": root, "cut": left == 0 })
}

fn breakpoint(request: &Value) -> Result<Value, String> {
    let at = request["at"].as_str().unwrap_or_default();
    if request["take"].as_bool().unwrap_or(false) {
        let removed = crate::debugger::remove(at)?;
        Ok(json!({
            "said": format!("{removed} {} taken away", if removed == 1 { "breakpoint" } else { "breakpoints" })
        }))
    } else {
        let place = crate::debugger::add(at)?;
        Ok(json!({ "said": format!("It stops at {place} each time it gets there.") }))
    }
}

fn go_to(app: &mut App, change: u32) -> Result<Value, String> {
    if !app.inspecting() || !crate::why::tracing() {
        return Err("nothing's being kept about state yet".to_owned());
    }
    let put_back = crate::bridge::go_to_change(change)?;
    Ok(json!({ "said": crate::why::went_to(change, put_back), "now": crate::why::history_now() }))
}

fn logpoint(at: &str, expression: Option<&str>) -> Result<Value, String> {
    // The app takes in its code again for a logpoint, and keeps its state through that, as it would for a save.
    crate::bridge::keep_state_across_reloads();
    let said = match expression {
        Some(expression) => {
            let place = crate::logpoint::add(at, expression)?;
            format!("{place} says {expression} each time it runs")
        }
        None => {
            let removed = crate::logpoint::remove(at)?;
            format!(
                "{removed} {} taken away",
                if removed == 1 {
                    "logpoint"
                } else {
                    "logpoints"
                }
            )
        }
    };
    Ok(json!({ "said": said }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;
    use std::sync::mpsc;

    const APP: &str = "import { useState } from 'rsui';
export default function App() {
    const [count, setCount] = useState(0);
    console.log(`count is ${count}`);
    return (
        <window>
            <button key=\"more\" onClick={() => setCount(count + 1)}>More</button>
            <text key=\"count\">{`Count ${count}`}</text>
        </window>
    );
}
";

    /// What the test does in the viewer, as someone would, once the app's been asked about over its pipe: the state's gone
    /// back, a logpoint's said and taken away, the app's evaluated in, stopped at a breakpoint and stepped through, and its
    /// elements and what came in to it are looked at.
    const VIEWER_STEPS: &str = "wait \"running, process\" 5000
        click \"Elements\"
        wait \"<window\" 5000
        click \"Element 0\"
        shot elements.png
        click \"Layout\"
        wait \"padding\" 3000
        shot box.png
        click \"State\"
        wait \"1 change of state\" 5000
        click \"Go back to here\"
        wait \"before all of them\" 3000
        wait \"Go forward to here\" 3000
        shot state.png
        click \"Console\"
        wait \"count * 10 = 0\" 3000
        type \"Console prompt\" \"6 * 7\"
        press enter \"Console prompt\"
        wait \"42\" 3000
        shot console.png
        click \"Sources\"
        click \"Take away the logpoint at app.tsx:5\"
        wait no \"count * 10\" 3000
        click \"Line 4\"
        wait \"Paused on breakpoint\" 5000
        wait \"Paused in debugger\" 3000
        shot sources.png
        click \"Console\"
        type \"Console prompt\" \"count + 100\"
        press enter \"Console prompt\"
        wait \"101\" 3000
        click \"Sources\"
        click \"Step over\"
        wait \"Paused after a step\" 3000
        click \"Resume\"
        wait no \"Paused in debugger\" 3000
        click \"Performance\"
        shot performance.png
        wait \"Every frame kept\" 5000
        click \"Bottom-up\"
        click \"Treemap\"
        shot treemap.png
        click \"As text\"
        wait \"with something happening\" 3000
        click \"Rendering\"
        shot rendering.png
        click \"Replay\"
        wait \"Replay and stop in it\" 3000
        shot replay.png
        size 480 800
        click \"Sources\"
        shot narrow.png
        click \"Dock to the right of the app\"
        click \"Dock to the bottom of the app\"
        click \"Undock into a separate window\"";

    /// What a viewer does: a line of JSON asked, and a line given back.
    fn ask(pipe: &mut File, answers: &mut BufReader<File>, request: Value) -> Value {
        let mut line = request.to_string();
        line.push('\n');
        pipe.write_all(line.as_bytes()).unwrap();
        line.clear();
        answers.read_line(&mut line).unwrap();
        let answer: Value = serde_json::from_str(&line).unwrap();
        assert!(
            answer.get("error").is_none(),
            "{request} was answered with {answer}"
        );
        answer
    }

    fn heard(news: &Value) -> Vec<String> {
        news["heard"]
            .as_array()
            .unwrap()
            .iter()
            .map(|one| one["line"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn a_viewer_looks_at_an_app_over_its_pipe_and_goes_back_through_its_state() {
        let folder = std::env::temp_dir().join(format!("rsui-live-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("app.tsx");
        std::fs::write(&path, APP).unwrap();
        let pipe = format!(r"\\.\pipe\rsui-live-{}", std::process::id());

        // The viewer asks for a click by sending on this, which the app's thread does and says it's done.
        let (want_click, clicks) = mpsc::channel::<bool>();
        let (clicked, click_done) = mpsc::channel::<()>();
        let stop_app = want_click.clone();
        let click_for_breakpoint = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let breakpoint_clicks = click_for_breakpoint.clone();
        let viewer = std::thread::spawn(move || {
            let mut connected = None;
            let mut why = None;
            for _ in 0..500 {
                match std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&pipe)
                {
                    Ok(file) => {
                        connected = Some(file);
                        break;
                    }
                    Err(error) => {
                        why = Some(error);
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
            let mut file = connected.unwrap_or_else(|| panic!("the app doesn't listen: {why:?}"));
            let mut answers = BufReader::new(file.try_clone().unwrap());
            let mut asked = |request| ask(&mut file, &mut answers, request);
            let click = || {
                want_click.send(true).unwrap();
                click_done.recv().unwrap();
            };

            assert_eq!(asked(json!({ "ask": "hello" }))["app"], "counter");
            let first = asked(json!({ "ask": "news" }));
            assert_eq!(
                first["started"], true,
                "asking starts it keeping what happens"
            );

            click();
            let news = asked(json!({ "ask": "news", "after": 0 }));
            let frames = news["frames"].as_array().unwrap();
            assert!(
                frames
                    .iter()
                    .any(|frame| frame["handled"][0]["action"] == "click"),
                "{frames:?}"
            );
            assert_eq!(news["history"].as_array().unwrap().len(), 1, "{news}");
            assert_eq!(news["now"], 1);
            assert!(
                heard(&news).iter().any(|line| line == "count is 1"),
                "{news}"
            );
            let said = news["said"].as_u64().unwrap();
            let last = frames.last().unwrap()["number"].as_u64().unwrap();

            let went = asked(json!({ "ask": "goto", "change": 0 }));
            assert!(
                went["said"]
                    .as_str()
                    .unwrap()
                    .contains("before all of them"),
                "{went}"
            );
            assert_eq!(went["now"], 0);

            let added =
                asked(json!({ "ask": "logpoint", "at": "app.tsx:5", "expression": "count * 10" }));
            assert!(
                added["said"].as_str().unwrap().starts_with("app.tsx:5"),
                "{added}"
            );
            click();
            let news = asked(json!({ "ask": "news", "after": last, "heard": said }));
            let lines = heard(&news);
            assert!(
                lines
                    .iter()
                    .any(|line| line == "logpoint app.tsx:5: count * 10 = 10"),
                "going back to 0 and clicking makes it 1 again: {lines:?}"
            );
            assert_eq!(news["logpoints"][0]["at"], "app.tsx:5");
            assert!(
                news["frames"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|frame| frame["number"].as_u64().unwrap() > last),
                "only what's newer comes"
            );

            let wrong = {
                let mut line = json!({ "ask": "fly" }).to_string();
                line.push('\n');
                file.write_all(line.as_bytes()).unwrap();
                line.clear();
                answers.read_line(&mut line).unwrap();
                line
            };
            assert!(wrong.contains("error"), "{wrong}");

            // A breakpoint has the app's code taken in again with places to stop, which renders it again, and it stops
            // there, with its thread waiting for what the viewer says.
            let mut asked = |request| ask(&mut file, &mut answers, request);
            let set = asked(json!({ "ask": "breakpoint", "at": "app.tsx:4" }));
            assert!(set["said"].as_str().unwrap().contains("app.tsx:4"), "{set}");
            let stopped = |asked: &mut dyn FnMut(Value) -> Value, at: &str| {
                for _ in 0..200 {
                    let news = asked(json!({ "ask": "news", "after": u64::MAX, "heard": u64::MAX }));
                    if news["paused"]["at"] == at {
                        return news;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                panic!("it didn't stop at {at}");
            };
            let news = stopped(&mut asked, "app.tsx:4");
            let paused = &news["paused"];
            assert_eq!(paused["reason"], "breakpoint", "{paused}");
            assert_eq!(paused["function"], "App", "{paused}");
            let locals = paused["frames"][0]["locals"].as_array().unwrap();
            assert!(
                locals.iter().any(|local| local["name"] == "count" && local["value"] == "1"),
                "{locals:?}"
            );
            assert!(
                paused["source"].as_array().unwrap().iter().any(|line| line["line"] == 4
                    && line["text"].as_str().unwrap().contains("count is")),
                "{paused}"
            );
            assert!(news["breakpoints"][0]["hits"].as_u64().unwrap() >= 1, "{news}");
            let worked = asked(json!({ "ask": "evaluate", "frame": 0, "expression": "count * 3 + 1" }));
            assert_eq!(worked["value"], "4", "{worked}");
            let busy = {
                let mut line = json!({ "ask": "goto", "change": 0 }).to_string();
                line.push('\n');
                file.write_all(line.as_bytes()).unwrap();
                line.clear();
                answers.read_line(&mut line).unwrap();
                line
            };
            assert!(busy.contains("stopped at app.tsx:4"), "{busy}");
            let mut asked = |request| ask(&mut file, &mut answers, request);
            asked(json!({ "ask": "resume", "how": "over" }));
            let news = stopped(&mut asked, "app.tsx:5");
            assert_eq!(news["paused"]["reason"], "step", "{news}");
            asked(json!({ "ask": "breakpoint", "at": "app.tsx:4", "take": true }));
            asked(json!({ "ask": "resume", "how": "continue" }));
            let docking = asked(json!({ "ask": "dock", "pid": 1, "side": "right", "size": 400 }));
            assert_eq!(docking["docked"], "right", "{docking}");
            let inputs = asked(json!({ "ask": "inputs" }));
            assert!(
                inputs["inputs"].as_array().unwrap().iter().any(|input| input["kind"] == "buttonup"),
                "what was clicked was kept, to be replayed: {inputs}"
            );
        });

        let app = std::thread::Builder::new()
            .stack_size(crate::STACK)
            .spawn(move || {
                let click_for_breakpoint = breakpoint_clicks;
                rsui::allow_inspector(true);
                start(&path, "counter");
                let mut desk =
                    crate::drive::Desk::open(&path, crate::STACK / 8 * 7, (400, 300)).unwrap();
                desk.settle(Duration::from_secs(2)).unwrap();
                // A window gives its app a way to be woken, which is what makes it the one a viewer looks at.
                desk.windows[0].app.set_waker(|| {});
                let mut clicked_for_breakpoint = false;
                loop {
                    // The viewer's own breakpoint is stopped at by a click, as someone using the app would.
                    if click_for_breakpoint.load(std::sync::atomic::Ordering::Relaxed)
                        && !clicked_for_breakpoint
                        && crate::debugger::entries().iter().any(|(_, hits)| *hits == 0)
                    {
                        clicked_for_breakpoint = true;
                        desk.run("click #more").unwrap();
                    }
                    match clicks.try_recv() {
                        Ok(true) => {
                            desk.run("click #more").unwrap();
                            desk.settle(Duration::from_secs(2)).unwrap();
                            clicked.send(()).unwrap();
                        }
                        Ok(false) | Err(mpsc::TryRecvError::Disconnected) => break,
                        Err(mpsc::TryRecvError::Empty) => {
                            desk.pass().unwrap();
                            std::thread::sleep(Duration::from_millis(2));
                        }
                    }
                }
                stop();
            })
            .unwrap();
        let viewed = viewer.join();
        // Then the viewer itself looks at it, as `rsui-tsx --view` would, with no window: the app's as the asking above
        // left it, gone back to before its first change and clicked again, with a logpoint.
        click_for_breakpoint.store(true, std::sync::atomic::Ordering::Relaxed);
        let shown = viewed.is_ok().then(|| {
            // SAFETY: nothing else in these tests reads or sets these.
            unsafe {
                std::env::set_var(crate::trace_file::LIVE_VAR, super::folder());
                std::env::set_var(crate::trace_file::PID_VAR, std::process::id().to_string());
            }
            let viewer = crate::trace_file::viewer_file().unwrap();
            std::thread::Builder::new()
                .stack_size(crate::STACK)
                .spawn(move || {
                    // With RSUI_VIEWER_SHOTS set to a folder, it saves what each part looks like there, to look at.
                    let shots = std::env::var("RSUI_VIEWER_SHOTS").ok();
                    let steps = VIEWER_STEPS
                        .lines()
                        .filter_map(|line| match line.trim().strip_prefix("shot ") {
                            Some(name) => shots.as_ref().map(|folder| format!("shot {folder}\\{name}")),
                            None => Some(line.to_owned()),
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    crate::drive::run_steps(&viewer, crate::STACK / 8 * 7, &steps)
                })
                .unwrap()
                .join()
        });
        stop_app.send(false).unwrap();
        app.join().unwrap();
        let _ = std::fs::remove_dir_all(&folder);
        viewed.unwrap();
        shown.unwrap().unwrap().unwrap();
    }
}
