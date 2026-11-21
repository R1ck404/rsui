//! The window on Android: the one rsui's own activity has (`RsuiActivity.java` in rsui-android), which hands over its
//! window's surface, and everything that comes in: touches, keys, and the soft keyboard's text and composing text.
//!
//! The activity's calls come on Android's main thread, through the app's library (`rsui-android`), which calls the
//! functions here. Each is queued for the app's thread, which a pipe wakes; a surface that's going away is waited for, as
//! Android needs nothing to draw on it once its call returns. The app's thread is the one that started the app, which runs
//! here in [`run_sized`] as it would in a window anywhere else: what comes in is fed to the app, and a frame is drawn when
//! it says one's needed.
//!
//! Drawing is with Vulkan, on the phone's GPU, or with the CPU renderer, straight into the window's buffer, where Vulkan can't.

#![allow(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::{c_int, c_void};
use std::path::PathBuf;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use rsui_core::IntoNode;
use rsui_render::Renderer;
use rsui_runtime::Cx;

use crate::replay::Input;
use crate::{App, Frame, KeyCode, KeyPress, RunError, WindowId, WindowOptions};

#[repr(C)]
struct WindowBuffer {
    width: i32,
    height: i32,
    stride: i32,
    format: i32,
    bits: *mut c_void,
    reserved: [u32; 6],
}

#[link(name = "android")]
unsafe extern "C" {
    fn ANativeWindow_release(window: *mut c_void);
    fn ANativeWindow_getWidth(window: *mut c_void) -> i32;
    fn ANativeWindow_getHeight(window: *mut c_void) -> i32;
    fn ANativeWindow_setBuffersGeometry(window: *mut c_void, width: i32, height: i32, format: i32) -> i32;
    fn ANativeWindow_lock(window: *mut c_void, buffer: *mut WindowBuffer, dirty: *mut c_void) -> i32;
    fn ANativeWindow_unlockAndPost(window: *mut c_void) -> i32;
    fn AAssetManager_open(manager: *mut c_void, name: *const std::ffi::c_char, mode: c_int) -> *mut c_void;
    fn AAsset_read(asset: *mut c_void, buffer: *mut c_void, count: usize) -> c_int;
    fn AAsset_close(asset: *mut c_void);
}

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(priority: c_int, tag: *const std::ffi::c_char, text: *const std::ffi::c_char) -> c_int;
}

const FORMAT_RGBA_8888: i32 = 1;

/// What the activity says, as the app's thread hears it.
enum Event {
    /// A new surface, acquired by whoever sent it, or none.
    Surface(*mut c_void),
    Redraw,
    Shown(bool),
    Focus(bool),
    Configuration { scale: f32, night: bool },
    LowMemory,
    Destroy,
    Touch { action: i32, pointer: i32, x: f32, y: f32 },
    Key { code: i32, meta: i32, down: bool, repeat: i32, unicode: i32 },
    Compose(String),
    FinishComposing,
    Commit(String),
    Delete { before: i32, after: i32 },
}

// SAFETY: the surface pointer is Android's, handed from one thread to the app's, which alone uses it from then on.
unsafe impl Send for Event {}

/// What's handed between Android's main thread and the app's.
struct Glue {
    events: VecDeque<Event>,
    write: c_int,
    read: c_int,
    /// How many events the app's thread has finished with, for a call that has to wait for that.
    done: u64,
    sent: u64,
    data: PathBuf,
    assets: usize,
    scale: f32,
    night: bool,
    keyboard: fn(bool),
}

static GLUE: Mutex<Option<Glue>> = Mutex::new(None);
static DONE: Condvar = Condvar::new();

fn lock() -> std::sync::MutexGuard<'static, Option<Glue>> {
    GLUE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn send(event: Event, wait: bool) {
    let mut glue = lock();
    let Some(state) = glue.as_mut() else { return };
    state.events.push_back(event);
    state.sent += 1;
    let number = state.sent;
    // SAFETY: the pipe is open while the glue is; a full pipe already has a wake in it.
    unsafe { libc::write(state.write, [1u8].as_ptr().cast(), 1) };
    if wait {
        while glue.as_ref().is_some_and(|state| state.done < number) {
            glue = DONE.wait(glue).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

/// Starts the app on a thread of its own: `main`, with the app's own folder, its package's assets (an
/// `AAssetManager`), the screen's scale and whether it's set to dark colors, and what shows or hides the soft keyboard.
/// This is what the app's library calls when the activity's made.
pub fn start(main: fn(), data: PathBuf, assets: *mut c_void, scale: f32, night: bool, keyboard: fn(bool)) {
    let _ = STARTED.set(Instant::now());
    send_output_to_the_log();
    let mut ends = [0; 2];
    // SAFETY: room for two descriptors.
    if unsafe { libc::pipe(ends.as_mut_ptr()) } != 0 {
        return;
    }
    for end in ends {
        // SAFETY: both are open.
        unsafe { libc::fcntl(end, libc::F_SETFL, libc::fcntl(end, libc::F_GETFL) | libc::O_NONBLOCK) };
    }
    *lock() = Some(Glue {
        events: VecDeque::new(),
        write: ends[1],
        read: ends[0],
        done: 0,
        sent: 0,
        data,
        assets: assets as usize,
        scale,
        night,
        keyboard,
    });
    let _ = std::thread::Builder::new().name("rsui app".into()).stack_size(8 * 1024 * 1024).spawn(main);
}

/// What's told the colors at the top and bottom edges of what's drawn, as ARGB (0 for none that's solid), when they change,
/// for the status and navigation bars: see [`on_bar_colors`].
static BAR_COLORS: std::sync::OnceLock<fn(u32, u32)> = std::sync::OnceLock::new();

/// Has `tell` hear the colors at the top and bottom edges of what's drawn, as ARGB, or 0 where nothing there is solid, each
/// time they change, so the system's bars beside them can be the same. Said once, before the app starts.
pub fn on_bar_colors(tell: fn(u32, u32)) {
    let _ = BAR_COLORS.set(tell);
}

/// Tells [`on_bar_colors`]'s hearer the edges' colors, if they're not what they were.
fn tell_bar_colors(app: &mut App, told: &mut Option<(u32, u32)>) {
    let Some(tell) = BAR_COLORS.get() else { return };
    let argb = |color: Option<rsui_render::Color>| {
        color.map_or(0, |color| u32::from_be_bytes([color.a, color.r, color.g, color.b]))
    };
    let (top, bottom) = app.edge_colors();
    let now = (argb(top), argb(bottom));
    if *told != Some(now) {
        *told = Some(now);
        tell(now.0, now.1);
    }
}

/// The window's surface, acquired by the caller, which the app's thread releases once it's done with it; or none, which
/// is waited for.
pub fn surface(window: *mut c_void) {
    send(Event::Surface(window), window.is_null());
}
pub fn redraw() {
    send(Event::Redraw, true);
}
pub fn shown(on: bool) {
    send(Event::Shown(on), false);
}
pub fn focus(on: bool) {
    send(Event::Focus(on), false);
}
pub fn configuration(scale: f32, night: bool) {
    send(Event::Configuration { scale, night }, false);
}
/// Whether the activity said the system was set to dark colors when it started, for what's drawn before the app hears it.
pub(crate) fn night() -> bool {
    lock().as_ref().is_some_and(|glue| glue.night)
}
pub fn low_memory() {
    send(Event::LowMemory, false);
}
pub fn destroy() {
    send(Event::Destroy, true);
}
pub fn touch(action: i32, pointer: i32, x: f32, y: f32) {
    send(Event::Touch { action, pointer, x, y }, false);
}
pub fn key(code: i32, meta: i32, down: bool, repeat: i32, unicode: i32) {
    send(Event::Key { code, meta, down, repeat, unicode }, false);
}
pub fn compose(text: String) {
    send(Event::Compose(text), false);
}
pub fn finish_composing() {
    send(Event::FinishComposing, false);
}
pub fn commit(text: String) {
    send(Event::Commit(text), false);
}
pub fn delete(before: i32, after: i32) {
    send(Event::Delete { before, after }, false);
}

/// The app's own folder on the device, which only it can see.
pub fn data_folder() -> Option<PathBuf> {
    lock().as_ref().map(|glue| glue.data.clone())
}

/// Copies what's in the app's package under `assets/`, as `rsui-assets.txt` there lists it, a path a line, to `into`.
/// Unless it's `every` one, what's there already, at the same size, is left.
pub fn copy_assets(into: &std::path::Path, every: bool) -> Result<usize, String> {
    let manager = lock().as_ref().ok_or("the app hasn't started")?.assets as *mut c_void;
    let read = |name: &str| -> Option<Vec<u8>> {
        let name = std::ffi::CString::new(name).ok()?;
        // SAFETY: the manager is the activity's, which lasts as long as the app; each asset is closed after it's read.
        unsafe {
            let asset = AAssetManager_open(manager, name.as_ptr(), 2);
            if asset.is_null() {
                return None;
            }
            let mut bytes = Vec::new();
            let mut chunk = vec![0u8; 64 * 1024];
            loop {
                let got = AAsset_read(asset, chunk.as_mut_ptr().cast(), chunk.len());
                if got <= 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..got as usize]);
            }
            AAsset_close(asset);
            Some(bytes)
        }
    };
    let list = read("rsui-assets.txt").ok_or("the package has no rsui-assets.txt")?;
    let list = String::from_utf8_lossy(&list).into_owned();
    let mut copied = 0;
    for name in list.lines().map(str::trim).filter(|name| !name.is_empty() && !name.contains("..")) {
        let Some(bytes) = read(name) else { continue };
        let to = into.join(name);
        if !every && std::fs::metadata(&to).is_ok_and(|found| found.len() == bytes.len() as u64) {
            continue;
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&to, bytes).map_err(|error| format!("{}: {error}", to.display()))?;
        copied += 1;
    }
    Ok(copied)
}

/// Has what's written to the standard output and error go to Android's log, where `adb logcat -s rsui` reads it.
fn send_output_to_the_log() {
    let mut ends = [0; 2];
    // SAFETY: room for two descriptors; the write end becomes both standard streams.
    unsafe {
        if libc::pipe(ends.as_mut_ptr()) != 0 {
            return;
        }
        libc::dup2(ends[1], 1);
        libc::dup2(ends[1], 2);
    }
    let read = ends[0];
    let _ = std::thread::Builder::new().name("rsui log".into()).stack_size(64 * 1024).spawn(move || {
        use std::io::BufRead;
        use std::os::fd::FromRawFd;
        // SAFETY: the read end is this thread's alone.
        let file = unsafe { std::fs::File::from_raw_fd(read) };
        for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(text) = std::ffi::CString::new(line) else { continue };
            // SAFETY: both strings end in a zero.
            unsafe { __android_log_write(4, c"rsui".as_ptr(), text.as_ptr()) };
        }
    });
}

thread_local! {
    static MAIN_KIND: Cell<(bool, bool, bool)> = const { Cell::new((false, false, false)) };
    static SMALLEST_MAIN: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
    static MAIN_OPENED: Cell<bool> = const { Cell::new(false) };
    static MAIN_MORE: RefCell<WindowOptions> = RefCell::new(WindowOptions::default());
}

pub(crate) fn main_opened() {
    MAIN_OPENED.with(|opened| opened.set(true));
}

/// An app on Android has one window, which fills the screen; what kind of window it is changes nothing.
pub(crate) fn change_main_kind(change: &crate::WindowChange) -> bool {
    if MAIN_OPENED.with(Cell::get) {
        return false;
    }
    if let crate::WindowChange::Layer(layer) = *change {
        MAIN_MORE.with(|more| more.borrow_mut().layer = layer);
        return true;
    }
    false
}

pub fn set_main_window_kind(frameless: bool, on_top: bool, fullscreen: bool) {
    MAIN_KIND.with(|kind| kind.set((frameless, on_top, fullscreen)));
}

pub fn set_main_window_options(options: &WindowOptions) {
    MAIN_MORE.with(|more| more.borrow_mut().layer = options.layer);
}

pub fn main_window_options() -> WindowOptions {
    let (frameless, on_top, fullscreen) = MAIN_KIND.with(Cell::get);
    let mut options = MAIN_MORE.with(|more| more.borrow().clone());
    options.frameless = frameless;
    options.on_top = on_top;
    options.fullscreen = fullscreen;
    options.smallest = SMALLEST_MAIN.with(Cell::get);
    options
}

pub fn set_smallest_window_size(width: u32, height: u32) {
    SMALLEST_MAIN.with(|smallest| smallest.set(Some((width, height))));
}

pub fn on_window_state_change(_hear: impl Fn(WindowId) + 'static) {}

pub fn window_handle(_id: WindowId) -> Option<isize> {
    None
}

pub fn window_state(_id: WindowId) -> Option<crate::WindowStateNow> {
    None
}

pub fn capture_window(_id: WindowId) -> Result<(u32, u32, Vec<u8>), String> {
    Err("there's no such window".to_owned())
}

pub fn on_window_captured(_hear: impl Fn(u32, Result<(u32, u32, Vec<u8>), String>) + 'static) {}

pub fn on_thumbnail_button(_hear: impl Fn(WindowId, u32) + 'static) {}

pub fn on_media_key(_hear: impl Fn(&'static str) + 'static) {}

pub fn run<P: 'static, R: IntoNode + 'static>(title: &str, render: fn(&mut Cx<'_>, &P) -> R, props: P) -> Result<(), RunError> {
    run_sized(title, None, render, props)
}

/// The window the app's drawing on, while there is one.
struct Shown {
    // The renderer goes first, as it may be drawing on the window.
    renderer: Renderer,
    /// Whether the renderer draws on the window itself, with Vulkan, rather than the CPU into memory.
    gpu: bool,
    size: (u32, u32),
    window: Held,
}

/// A window acquired for drawing on, let go of once.
struct Held(*mut c_void);

impl Drop for Held {
    fn drop(&mut self) {
        // SAFETY: the window was acquired for this, and is let go of once.
        unsafe { ANativeWindow_release(self.0) };
    }
}

/// What draws on `window`: the GPU, with Vulkan, if there's one that can, or else the CPU. `RSUI_VULKAN=0` says to draw
/// with the CPU, and setting `RSUI_VULKAN` at all says which it is and why.
fn renderer_for(window: *mut c_void, size: (u32, u32)) -> Option<(Renderer, bool)> {
    let told = std::env::var_os("RSUI_VULKAN").is_some();
    // SAFETY: the window is held for as long as the renderer is, as `Shown` drops the renderer first.
    match unsafe { Renderer::for_android(window, size) } {
        Ok(renderer) => {
            if told {
                eprintln!("rsui: drawing with Vulkan");
            }
            Some((renderer, true))
        }
        Err(error) => {
            if told {
                eprintln!("rsui: drawing with the CPU, as {error}");
            }
            Renderer::offscreen(size).ok().map(|renderer| (renderer, false))
        }
    }
}

/// Runs the app in the activity's window, until the activity's destroyed.
pub fn run_sized<P: 'static, R: IntoNode + 'static>(
    _title: &str,
    _size: Option<(u32, u32)>,
    render: fn(&mut Cx<'_>, &P) -> R,
    props: P,
) -> Result<(), RunError> {
    MAIN_OPENED.with(|opened| opened.set(true));
    let (read, write, mut scale, night, keyboard) = lock()
        .as_ref()
        .map(|glue| (glue.read, glue.write, glue.scale, glue.night, glue.keyboard))
        .ok_or_else(|| RunError::Window("the app hasn't started".to_owned()))?;
    let text = crate::default_text_engine()?;
    // A phone's scrollbars show only while something scrolls.
    rsui_render::fade_scrollbars(true);
    let mut app = App::new(render, props, text, (1, 1), scale);
    // What another thread has for the app, as what a popup was answered with, wakes the loop as what the activity says does.
    app.set_waker(move || {
        // SAFETY: the pipe is open for as long as the process is; a full pipe already has a wake in it.
        unsafe { libc::write(write, [1u8].as_ptr().cast(), 1) };
    });
    let _ = app.feed(&Input::DarkMode(night));
    let mut shown: Option<Shown> = None;
    let mut needs_draw = true;
    let mut wake_at: Option<Instant> = None;
    let mut blink_at: Option<Instant> = None;
    let mut composing = String::new();
    let mut keyboard_up = false;
    let mut finger = Finger::default();
    let mut bars_told = None;
    loop {
        let now = Instant::now();
        let deadline = [wake_at, blink_at].into_iter().flatten().min();
        let timeout = if (app.animating() || finger.flinging()) && shown.is_some() {
            16
        } else {
            deadline.map_or(-1, |at| at.saturating_duration_since(now).as_millis().min(i32::MAX as u128) as c_int)
        };
        let mut poll = libc::pollfd { fd: read, events: libc::POLLIN, revents: 0 };
        // SAFETY: one descriptor, the pipe's, which is open.
        unsafe { libc::poll(&raw mut poll, 1, timeout) };
        let mut drain = [0u8; 64];
        // SAFETY: the pipe doesn't wait, and the room is as long as it says.
        while unsafe { libc::read(read, drain.as_mut_ptr().cast(), drain.len()) } > 0 {}
        loop {
            let event = lock().as_mut().and_then(|glue| glue.events.pop_front());
            let Some(event) = event else { break };
            let now = Instant::now();
            let stop = match event {
                // The same window, as it is when it's only changed size, keeps its renderer.
                Event::Surface(window) if shown.as_ref().is_some_and(|shown| shown.window.0 == window) => {
                    // SAFETY: acquired again by whoever sent it, and held already.
                    unsafe { ANativeWindow_release(window) };
                    needs_draw = true;
                    false
                }
                Event::Surface(window) => {
                    shown = None;
                    if !window.is_null() {
                        crate::mark("surface");
                        let size = window_size(window);
                        if let Some((renderer, gpu)) = renderer_for(window, size) {
                            crate::mark("renderer made");
                            app.resize(size, scale);
                            app.renderer_replaced();
                            shown = Some(Shown { renderer, gpu, size, window: Held(window) });
                        } else {
                            // SAFETY: acquired by whoever sent it, and not kept.
                            unsafe { ANativeWindow_release(window) };
                        }
                    }
                    needs_draw = true;
                    false
                }
                Event::Redraw => {
                    if let Some(now) = shown.as_mut() {
                        refit(&mut app, now, scale);
                        draw(&mut app, now);
                        tell_bar_colors(&mut app, &mut bars_told);
                        needs_draw = false;
                    }
                    false
                }
                Event::Shown(on) => {
                    let _ = app.feed(&Input::Shown(on));
                    needs_draw |= on;
                    false
                }
                Event::Focus(on) => {
                    let _ = app.feed(&Input::Active(on));
                    false
                }
                Event::Configuration { scale: given, night } => {
                    scale = given;
                    if let Some(now) = shown.as_mut() {
                        refit(&mut app, now, scale);
                    }
                    let _ = app.feed(&Input::DarkMode(night));
                    needs_draw = true;
                    false
                }
                Event::LowMemory => {
                    if let Some(shown) = shown.as_mut() {
                        shown.renderer.trim();
                    }
                    false
                }
                Event::Destroy => {
                    let _ = app.feed(&Input::Close);
                    true
                }
                Event::Touch { action, pointer, x, y } => {
                    app.set_input_time(now);
                    // The first finger is the pointer; others are left until rsui has more than one.
                    if pointer == 0 {
                        finger.touched(&mut app, action, x, y, scale, now);
                    }
                    false
                }
                Event::Key { code, meta, down, repeat, unicode } => {
                    app.set_input_time(now);
                    feed_key(&mut app, code, meta, down, repeat, unicode);
                    false
                }
                // The soft keyboard shows nothing of what's being composed itself, as a desktop's does: the word so far is
                // typed into the field, and each change to it takes back what was typed before and types it again.
                Event::Compose(text) => {
                    app.set_input_time(now);
                    retype(&mut app, &mut composing, &text);
                    composing = text;
                    false
                }
                Event::FinishComposing => {
                    // What was being composed stays as it was typed.
                    composing.clear();
                    false
                }
                Event::Commit(text) => {
                    app.set_input_time(now);
                    // Some keyboards send Enter as a new line typed, rather than as the key.
                    if text == "\n" {
                        composing.clear();
                        let _ = app.feed(&Input::Key(KeyPress::new(KeyCode::Enter)));
                    } else {
                        retype(&mut app, &mut composing, &text);
                    }
                    false
                }
                Event::Delete { before, after } => {
                    for _ in 0..before.max(0) {
                        let _ = app.feed(&Input::Key(KeyPress::new(KeyCode::Backspace)));
                    }
                    for _ in 0..after.max(0) {
                        let _ = app.feed(&Input::Key(KeyPress::new(KeyCode::Delete)));
                    }
                    false
                }
            };
            let mut glue = lock();
            if let Some(state) = glue.as_mut() {
                state.done += 1;
            }
            DONE.notify_all();
            drop(glue);
            if stop {
                return Ok(());
            }
        }
        let now = Instant::now();
        if app.animating() {
            app.tick(now);
        }
        if wake_at.is_some_and(|at| at <= now) {
            app.wake();
        }
        if blink_at.is_some_and(|at| at <= now) {
            blink_at = None;
            app.blink();
        }
        finger.fling_on(&mut app, Instant::now());
        if app.update() == Frame::Needed {
            needs_draw = true;
        }
        for url in app.take_urls_wanted() {
            crate::system::open_url(&url);
        }
        // The soft keyboard comes up for a field that takes text, and goes once nothing does.
        let typing = app.caret_place().is_some();
        if typing != keyboard_up {
            keyboard_up = typing;
            keyboard(typing);
        }
        if needs_draw && let Some(shown) = shown.as_mut() {
            needs_draw = false;
            refit(&mut app, shown, scale);
            draw(&mut app, shown);
            tell_bar_colors(&mut app, &mut bars_told);
        }
        wake_at = app.next_wake().map(|wait| Instant::now() + wait);
        if app.caret_blinking() && blink_at.is_none() {
            blink_at = Some(Instant::now() + Duration::from_millis(530));
        }
    }
}

/// How far a finger goes before it moves what's under it rather than pressing it, in logical pixels: Android's touch slop.
const TOUCH_SLOP: f32 = 8.0;
/// How quickly a fling slows: its speed falls to 1/e in this many seconds, which takes it `speed * FLING_FADE` in all.
const FLING_FADE: f32 = 0.5;
/// The slowest a lifted finger can be going and still fling, the fastest a fling starts, and the speed it stops at, in logical
/// pixels a second, as Android's 50 and 8,000 dp a second.
const FLING_LEAST: f32 = 50.0;
const FLING_MOST: f32 = 8000.0;
const FLING_STOP: f32 = 20.0;
/// How far back how fast a finger was going is worked out from.
const FLING_TRAIL: Duration = Duration::from_millis(100);

/// A finger on the screen, as a phone has them do: one that's put down and lifted where it was presses what's there; one that
/// goes further than the touch slop moves what's under it, across or down as it went, with it, and is let go with a fling,
/// which slows to a stop and which another finger stops without pressing anything.
#[derive(Default)]
struct Finger {
    /// Where it went down, in the window's pixels: what's moved is what's under there.
    down: Option<(f32, f32)>,
    /// Whether it's pressing what it went down on, as a mouse button would.
    pressing: bool,
    /// Which way it's moving what's under it, once it's gone past the slop: across (true) or down.
    panning: Option<bool>,
    /// Where it was last, in the window's pixels.
    last: (f32, f32),
    /// Where it's been lately, along the way it's moving, in logical pixels.
    trail: VecDeque<(Instant, f32)>,
    fling: Option<Fling>,
}

struct Fling {
    x: f32,
    y: f32,
    across: bool,
    /// Logical pixels a second, further on (down or right) when it's more than 0.
    speed: f32,
    at: Instant,
}

impl Finger {
    fn flinging(&self) -> bool {
        self.fling.is_some()
    }

    fn touched(&mut self, app: &mut App, action: i32, x: f32, y: f32, scale: f32, now: Instant) {
        match action {
            // Down.
            0 => {
                let caught = self.fling.take().is_some();
                self.down = Some((x, y));
                self.last = (x, y);
                self.panning = None;
                self.trail.clear();
                self.pressing = !caught;
                if self.pressing {
                    let _ = app.feed(&Input::Move { x, y });
                    let _ = app.feed(&Input::Down { x, y, clicks: 1 });
                }
            }
            // Moved.
            2 => {
                let Some((from_x, from_y)) = self.down else { return };
                let across = match self.panning {
                    Some(across) => across,
                    None => {
                        let (dx, dy) = (x - from_x, y - from_y);
                        let slop = TOUCH_SLOP * scale;
                        if dx.hypot(dy) < slop {
                            if self.pressing {
                                let _ = app.feed(&Input::Move { x, y });
                            }
                            return;
                        }
                        let across = dx.abs() > dy.abs();
                        self.panning = Some(across);
                        if self.pressing {
                            self.pressing = false;
                            let _ = app.feed(&Input::CancelPress);
                            let _ = app.feed(&Input::Leave);
                        }
                        // What it moves starts from where it went past the slop, as Android's own scrolling does, so nothing
                        // jumps by the slop as it starts.
                        self.last = if across {
                            (from_x + slop.copysign(dx), y)
                        } else {
                            (x, from_y + slop.copysign(dy))
                        };
                        across
                    }
                };
                let moved = if across { self.last.0 - x } else { self.last.1 - y };
                self.last = (x, y);
                self.trail_to(now, (if across { x } else { y }) / scale);
                let _ = app.feed(&Input::ScrollPixels { x: from_x, y: from_y, pixels: moved / scale, across, precise: true });
            }
            // Lifted.
            1 | 6 => {
                if self.pressing {
                    let _ = app.feed(&Input::Up { x, y });
                    // A finger that's lifted isn't over anything any more.
                    let _ = app.feed(&Input::Leave);
                } else if let (Some(across), Some((from_x, from_y))) = (self.panning, self.down) {
                    self.trail_to(now, (if across { x } else { y }) / scale);
                    if let (Some(&(first_at, first)), Some(&(last_at, last))) = (self.trail.front(), self.trail.back()) {
                        let took = last_at.saturating_duration_since(first_at).as_secs_f32();
                        // Moving up or left is moving further on.
                        let speed = if took > 0.004 { (first - last) / took } else { 0.0 };
                        if speed.abs() >= FLING_LEAST {
                            self.fling = Some(Fling {
                                x: from_x,
                                y: from_y,
                                across,
                                speed: speed.clamp(-FLING_MOST, FLING_MOST),
                                at: now,
                            });
                        }
                    }
                }
                self.down = None;
                self.pressing = false;
                self.panning = None;
            }
            // Taken away, as when a system gesture has it.
            3 => {
                if self.pressing {
                    let _ = app.feed(&Input::CancelPress);
                    let _ = app.feed(&Input::Leave);
                }
                self.down = None;
                self.pressing = false;
                self.panning = None;
            }
            _ => {}
        }
    }

    fn trail_to(&mut self, now: Instant, along: f32) {
        self.trail.push_back((now, along));
        while self.trail.front().is_some_and(|&(at, _)| now.saturating_duration_since(at) > FLING_TRAIL) {
            self.trail.pop_front();
        }
    }

    /// Moves what's being flung on by as far as it's gone since it last was, slowing.
    fn fling_on(&mut self, app: &mut App, now: Instant) {
        let Some(fling) = self.fling.as_mut() else { return };
        let took = now.saturating_duration_since(fling.at).as_secs_f32();
        if took <= 0.0 {
            return;
        }
        fling.at = now;
        // How far something going at `speed`, slowing as e^(-t/FLING_FADE), goes in `took`.
        let fade = (-took / FLING_FADE).exp();
        let pixels = fling.speed * FLING_FADE * (1.0 - fade);
        fling.speed *= fade;
        let (x, y, across) = (fling.x, fling.y, fling.across);
        if fling.speed.abs() < FLING_STOP {
            self.fling = None;
        }
        let _ = app.feed(&Input::ScrollPixels { x, y, pixels, across, precise: true });
    }
}

/// Takes back what was being composed, a Backspace a character, and types `text` in its place.
fn retype(app: &mut App, composing: &mut String, text: &str) {
    let common = composing.chars().zip(text.chars()).take_while(|(was, is)| was == is).count();
    for _ in common..composing.chars().count() {
        let _ = app.feed(&Input::Key(KeyPress::new(KeyCode::Backspace)));
    }
    let rest: String = text.chars().skip(common).collect();
    if !rest.is_empty() {
        let _ = app.feed(&Input::Text(rest));
    }
    composing.clear();
}

fn window_size(window: *mut c_void) -> (u32, u32) {
    // SAFETY: the window is held.
    unsafe { (ANativeWindow_getWidth(window).max(1) as u32, ANativeWindow_getHeight(window).max(1) as u32) }
}

/// Has the renderer and the app as big as the window is now.
fn refit(app: &mut App, shown: &mut Shown, scale: f32) {
    let size = window_size(shown.window.0);
    if size != shown.size {
        shown.size = size;
        let _ = shown.renderer.resize(size);
    }
    app.resize(size, scale);
}

/// Draws a frame with the CPU, and copies it into the window's buffer.
fn draw(app: &mut App, shown: &mut Shown) {
    draw_now(app, shown);
    // How long the app took to show, from the activity being made, which the log says once.
    static TOLD: std::sync::Once = std::sync::Once::new();
    if let Some(started) = STARTED.get() {
        TOLD.call_once(|| {
            crate::mark("first frame");
            eprintln!("rsui: first frame {} ms after the activity was made", started.elapsed().as_millis());
        });
    }
}

/// When the activity was made, for how long the first frame took.
static STARTED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn draw_now(app: &mut App, shown: &mut Shown) {
    if shown.gpu {
        if let Err(error) = app.render(&mut shown.renderer) {
            // The GPU's gone, or can't draw here: the CPU draws from now on, once Vulkan's let go of the window.
            if std::env::var_os("RSUI_VULKAN").is_some() {
                eprintln!("rsui: Vulkan stopped drawing ({error}); drawing with the CPU");
            }
            let Ok(renderer) = Renderer::offscreen(shown.size) else { return };
            shown.renderer = renderer;
            shown.gpu = false;
            app.renderer_replaced();
        } else {
            return;
        }
    }
    if app.render(&mut shown.renderer).is_err() {
        return;
    }
    let Ok(pixels) = shown.renderer.read_pixels() else { return };
    let (width, height) = shown.size;
    // SAFETY: the window is held; the buffer it hands out is `stride` pixels a row, `height` rows, until it's posted.
    unsafe {
        ANativeWindow_setBuffersGeometry(shown.window.0, width as i32, height as i32, FORMAT_RGBA_8888);
        let mut buffer: WindowBuffer = std::mem::zeroed();
        if ANativeWindow_lock(shown.window.0, &raw mut buffer, std::ptr::null_mut()) != 0 {
            return;
        }
        let rows = (buffer.height.max(0) as usize).min(height as usize);
        let across = (buffer.width.max(0) as usize).min(width as usize) * 4;
        for row in 0..rows {
            let from = &pixels[row * width as usize * 4..row * width as usize * 4 + across];
            let to = buffer.bits.cast::<u8>().add(row * buffer.stride as usize * 4);
            std::ptr::copy_nonoverlapping(from.as_ptr(), to, across);
        }
        ANativeWindow_unlockAndPost(shown.window.0);
    }
}

/// A key from a keyboard, or one the soft keyboard sends, as Enter and Backspace. What it types comes with it.
fn feed_key(app: &mut App, code: i32, meta: i32, down: bool, repeat: i32, unicode: i32) {
    let typed = char::from_u32(unicode as u32).filter(|typed| unicode > 0 && !typed.is_control());
    let named = match code {
        61 => Some(KeyCode::Tab),
        62 => Some(KeyCode::Space),
        66 | 160 => Some(KeyCode::Enter),
        67 => Some(KeyCode::Backspace),
        112 => Some(KeyCode::Delete),
        19 => Some(KeyCode::Up),
        20 => Some(KeyCode::Down),
        21 => Some(KeyCode::Left),
        22 => Some(KeyCode::Right),
        122 => Some(KeyCode::Home),
        123 => Some(KeyCode::End),
        92 => Some(KeyCode::PageUp),
        93 => Some(KeyCode::PageDown),
        111 => Some(KeyCode::Escape),
        59 | 60 => Some(KeyCode::Shift),
        113 | 114 => Some(KeyCode::Control),
        57 | 58 => Some(KeyCode::Alt),
        131..=142 => Some(KeyCode::Function((code - 130) as u8)),
        _ => None,
    };
    let Some(code) = named.or_else(|| typed.map(|typed| KeyCode::Character(typed.to_lowercase().next().unwrap_or(typed))))
    else {
        return;
    };
    let press = KeyPress {
        shift: meta & 0x1 != 0,
        alt: meta & 0x2 != 0,
        ctrl: meta & 0x1000 != 0,
        meta: meta & 0x10000 != 0,
        repeat: repeat > 0,
        typed,
        ..KeyPress::new(code)
    };
    if down {
        let _ = app.feed(&Input::Key(press));
        if let Some(typed) = typed.filter(|_| !press.ctrl && !press.alt && named.is_none_or(|named| named == KeyCode::Space)) {
            let _ = app.feed(&Input::Text(typed.to_string()));
        }
    } else {
        let _ = app.feed(&Input::KeyUp(press));
    }
}
