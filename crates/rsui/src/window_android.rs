//! The window on Android: the one rsui's own activity has (`RsuiActivity.java` in rsui-android), which hands over its
//! window's surface, and everything that comes in: touches, keys, and the soft keyboard's text and composing text.
//!
//! The activity's calls come on Android's main thread, through the app's library (`rsui-android`), which calls the
//! functions here. Each is queued for the app's thread, which a pipe wakes; a surface that's going away is waited for, as
//! Android needs nothing to draw on it once its call returns. The app's thread is the one that started the app, which runs
//! here in [`run_sized`] as it would in a window anywhere else: what comes in is fed to the app, and a frame is drawn when
//! it says one's needed.
//!
//! Drawing is with the CPU renderer for now, straight into the window's buffer; Vulkan comes in its place (plan 058, §5.7).

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
/// What's there already, at the same size, is left.
pub fn copy_assets(into: &std::path::Path) -> Result<usize, String> {
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
        if std::fs::metadata(&to).is_ok_and(|found| found.len() == bytes.len() as u64) {
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
    window: *mut c_void,
    renderer: Renderer,
    size: (u32, u32),
}

impl Drop for Shown {
    fn drop(&mut self) {
        // SAFETY: the window was acquired for this, and is let go of once.
        unsafe { ANativeWindow_release(self.window) };
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
    let (read, mut scale, night, keyboard) = lock()
        .as_ref()
        .map(|glue| (glue.read, glue.scale, glue.night, glue.keyboard))
        .ok_or_else(|| RunError::Window("the app hasn't started".to_owned()))?;
    let text = crate::default_text_engine()?;
    let mut app = App::new(render, props, text, (1, 1), scale);
    let _ = app.feed(&Input::DarkMode(night));
    let mut shown: Option<Shown> = None;
    let mut needs_draw = true;
    let mut wake_at: Option<Instant> = None;
    let mut blink_at: Option<Instant> = None;
    let mut composing = String::new();
    let mut keyboard_up = false;
    loop {
        let now = Instant::now();
        let deadline = [wake_at, blink_at].into_iter().flatten().min();
        let timeout = if app.animating() && shown.is_some() {
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
                Event::Surface(window) if shown.as_ref().is_some_and(|shown| shown.window == window) => {
                    // SAFETY: acquired again by whoever sent it, and held already.
                    unsafe { ANativeWindow_release(window) };
                    needs_draw = true;
                    false
                }
                Event::Surface(window) => {
                    shown = None;
                    if !window.is_null() {
                        let size = window_size(window);
                        if let Ok(renderer) = Renderer::offscreen(size) {
                            app.resize(size, scale);
                            app.renderer_replaced();
                            shown = Some(Shown { window, renderer, size });
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
                        match action {
                            0 => {
                                let _ = app.feed(&Input::Move { x, y });
                                let _ = app.feed(&Input::Down { x, y, clicks: 1 });
                            }
                            2 => {
                                let _ = app.feed(&Input::Move { x, y });
                            }
                            1 | 6 => {
                                let _ = app.feed(&Input::Up { x, y });
                                // A finger that's lifted isn't over anything any more.
                                let _ = app.feed(&Input::Leave);
                            }
                            3 => {
                                let _ = app.feed(&Input::CancelPress);
                                let _ = app.feed(&Input::Leave);
                            }
                            _ => {}
                        }
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
                    retype(&mut app, &mut composing, &text);
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
        }
        wake_at = app.next_wake().map(|wait| Instant::now() + wait);
        if app.caret_blinking() && blink_at.is_none() {
            blink_at = Some(Instant::now() + Duration::from_millis(530));
        }
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
    let size = window_size(shown.window);
    if size != shown.size {
        shown.size = size;
        let _ = shown.renderer.resize(size);
    }
    app.resize(size, scale);
}

/// Draws a frame with the CPU, and copies it into the window's buffer.
fn draw(app: &mut App, shown: &mut Shown) {
    if app.render(&mut shown.renderer).is_err() {
        return;
    }
    let Ok(pixels) = shown.renderer.read_pixels() else { return };
    let (width, height) = shown.size;
    // SAFETY: the window is held; the buffer it hands out is `stride` pixels a row, `height` rows, until it's posted.
    unsafe {
        ANativeWindow_setBuffersGeometry(shown.window, width as i32, height as i32, FORMAT_RGBA_8888);
        let mut buffer: WindowBuffer = std::mem::zeroed();
        if ANativeWindow_lock(shown.window, &raw mut buffer, std::ptr::null_mut()) != 0 {
            return;
        }
        let rows = (buffer.height.max(0) as usize).min(height as usize);
        let across = (buffer.width.max(0) as usize).min(width as usize) * 4;
        for row in 0..rows {
            let from = &pixels[row * width as usize * 4..row * width as usize * 4 + across];
            let to = buffer.bits.cast::<u8>().add(row * buffer.stride as usize * 4);
            std::ptr::copy_nonoverlapping(from.as_ptr(), to, across);
        }
        ANativeWindow_unlockAndPost(shown.window);
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
