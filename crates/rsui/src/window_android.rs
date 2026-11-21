//! The window on Android: the one an activity has, as Android's `NativeActivity` hands it over.
//!
//! The activity's callbacks come on Android's own main thread. Each is written to a pipe that the app's thread waits on
//! with its looper, beside the queue of touches and keys; a window that's going away is waited for, as Android needs
//! nothing to draw on it once its callback returns. The app's thread is the one that started the app, which runs here in
//! [`run_sized`] as it would in a window anywhere else: what comes in is fed to the app, and a frame is drawn when it says
//! one's needed.
//!
//! Drawing is with the CPU renderer for now, straight into the window's buffer. Vulkan comes later (plan 058, §5.7), as
//! do the soft keyboard's composing text and TalkBack, which need an activity of rsui's own rather than this one.

#![allow(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_int, c_void};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use rsui_core::IntoNode;
use rsui_render::Renderer;
use rsui_runtime::Cx;

use crate::replay::Input;
use crate::{App, Frame, KeyCode, KeyPress, RunError, WindowId, WindowLayer, WindowOptions};

// What the NDK has, as its headers lay it out.

#[repr(C)]
pub struct ANativeActivity {
    pub callbacks: *mut Callbacks,
    pub vm: *mut c_void,
    pub env: *mut c_void,
    pub clazz: *mut c_void,
    pub internal_data_path: *const c_char,
    pub external_data_path: *const c_char,
    pub sdk_version: i32,
    pub instance: *mut c_void,
    pub asset_manager: *mut c_void,
    pub obb_path: *const c_char,
}

type Activity = *mut ANativeActivity;

#[repr(C)]
pub struct Callbacks {
    on_start: Option<unsafe extern "C" fn(Activity)>,
    on_resume: Option<unsafe extern "C" fn(Activity)>,
    on_save_instance_state: Option<unsafe extern "C" fn(Activity, *mut usize) -> *mut c_void>,
    on_pause: Option<unsafe extern "C" fn(Activity)>,
    on_stop: Option<unsafe extern "C" fn(Activity)>,
    on_destroy: Option<unsafe extern "C" fn(Activity)>,
    on_window_focus_changed: Option<unsafe extern "C" fn(Activity, c_int)>,
    on_native_window_created: Option<unsafe extern "C" fn(Activity, *mut c_void)>,
    on_native_window_resized: Option<unsafe extern "C" fn(Activity, *mut c_void)>,
    on_native_window_redraw_needed: Option<unsafe extern "C" fn(Activity, *mut c_void)>,
    on_native_window_destroyed: Option<unsafe extern "C" fn(Activity, *mut c_void)>,
    on_input_queue_created: Option<unsafe extern "C" fn(Activity, *mut c_void)>,
    on_input_queue_destroyed: Option<unsafe extern "C" fn(Activity, *mut c_void)>,
    on_content_rect_changed: Option<unsafe extern "C" fn(Activity, *const c_void)>,
    on_configuration_changed: Option<unsafe extern "C" fn(Activity)>,
    on_low_memory: Option<unsafe extern "C" fn(Activity)>,
}

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
    fn ANativeWindow_acquire(window: *mut c_void);
    fn ANativeWindow_release(window: *mut c_void);
    fn ANativeWindow_getWidth(window: *mut c_void) -> i32;
    fn ANativeWindow_getHeight(window: *mut c_void) -> i32;
    fn ANativeWindow_setBuffersGeometry(window: *mut c_void, width: i32, height: i32, format: i32) -> i32;
    fn ANativeWindow_lock(window: *mut c_void, buffer: *mut WindowBuffer, dirty: *mut c_void) -> i32;
    fn ANativeWindow_unlockAndPost(window: *mut c_void) -> i32;
    fn ALooper_prepare(options: c_int) -> *mut c_void;
    fn ALooper_addFd(looper: *mut c_void, fd: c_int, ident: c_int, events: c_int, callback: *mut c_void, data: *mut c_void)
    -> c_int;
    fn ALooper_pollOnce(timeout: c_int, fd: *mut c_int, events: *mut c_int, data: *mut *mut c_void) -> c_int;
    fn AInputQueue_attachLooper(queue: *mut c_void, looper: *mut c_void, ident: c_int, callback: *mut c_void, data: *mut c_void);
    fn AInputQueue_detachLooper(queue: *mut c_void);
    fn AInputQueue_getEvent(queue: *mut c_void, event: *mut *mut c_void) -> i32;
    fn AInputQueue_preDispatchEvent(queue: *mut c_void, event: *mut c_void) -> i32;
    fn AInputQueue_finishEvent(queue: *mut c_void, event: *mut c_void, handled: c_int);
    fn AInputEvent_getType(event: *const c_void) -> i32;
    fn AMotionEvent_getAction(event: *const c_void) -> i32;
    fn AMotionEvent_getX(event: *const c_void, pointer: usize) -> f32;
    fn AMotionEvent_getY(event: *const c_void, pointer: usize) -> f32;
    fn AKeyEvent_getAction(event: *const c_void) -> i32;
    fn AKeyEvent_getKeyCode(event: *const c_void) -> i32;
    fn AKeyEvent_getMetaState(event: *const c_void) -> i32;
    fn AKeyEvent_getRepeatCount(event: *const c_void) -> i32;
    fn AConfiguration_new() -> *mut c_void;
    fn AConfiguration_fromAssetManager(config: *mut c_void, assets: *mut c_void);
    fn AConfiguration_getDensity(config: *const c_void) -> i32;
    fn AConfiguration_getUiModeNight(config: *const c_void) -> i32;
    fn AConfiguration_delete(config: *mut c_void);
    fn ANativeActivity_finish(activity: Activity);
    fn ANativeActivity_showSoftInput(activity: Activity, flags: u32);
    fn AAssetManager_open(manager: *mut c_void, name: *const c_char, mode: c_int) -> *mut c_void;
    fn AAsset_read(asset: *mut c_void, buffer: *mut c_void, count: usize) -> c_int;
    fn AAsset_close(asset: *mut c_void);
}

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

const LOOPER_ID_COMMAND: c_int = 1;
const LOOPER_ID_INPUT: c_int = 2;
const LOOPER_EVENT_INPUT: c_int = 1;
const FORMAT_RGBA_8888: i32 = 1;
const INPUT_KEY: i32 = 1;
const INPUT_MOTION: i32 = 2;

/// What the activity's callbacks say, as the app's thread hears it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Command {
    WindowCreated = 1,
    WindowResized,
    WindowRedraw,
    WindowDestroyed,
    InputCreated,
    InputDestroyed,
    Focused,
    Unfocused,
    Resumed,
    Paused,
    ConfigurationChanged,
    LowMemory,
    Destroy,
}

impl Command {
    fn from_byte(byte: u8) -> Option<Self> {
        use Command::*;
        [WindowCreated, WindowResized, WindowRedraw, WindowDestroyed, InputCreated, InputDestroyed, Focused, Unfocused, Resumed,
            Paused, ConfigurationChanged, LowMemory, Destroy]
            .into_iter()
            .find(|command| *command as u8 == byte)
    }
}

/// What's handed between the activity's thread and the app's.
struct Glue {
    activity: Activity,
    /// The pipe the callbacks write to and the app's thread reads.
    write: c_int,
    read: c_int,
    /// The window and input queue the callbacks were last given, which the app's thread takes when it's told.
    window: *mut c_void,
    queue: *mut c_void,
    /// The last command the app's thread has finished with, for a callback that has to wait for that.
    done: u64,
    sent: u64,
}

// SAFETY: the pointers are Android's, used only as its documentation says, under the lock.
unsafe impl Send for Glue {}

static GLUE: Mutex<Option<Glue>> = Mutex::new(None);
static DONE: Condvar = Condvar::new();

fn lock() -> std::sync::MutexGuard<'static, Option<Glue>> {
    GLUE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Tells the app's thread something, and waits for it to have dealt with it if `wait`.
fn send(command: Command, window: Option<*mut c_void>, queue: Option<*mut c_void>, wait: bool) {
    let mut glue = lock();
    let Some(state) = glue.as_mut() else { return };
    if let Some(window) = window {
        state.window = window;
    }
    if let Some(queue) = queue {
        state.queue = queue;
    }
    state.sent += 1;
    let number = state.sent;
    let byte = command as u8;
    // SAFETY: the pipe is open while the glue is.
    unsafe { libc::write(state.write, (&raw const byte).cast(), 1) };
    if wait {
        while glue.as_ref().is_some_and(|state| state.done < number) {
            glue = DONE.wait(glue).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

unsafe extern "C" fn on_resume(_: Activity) {
    send(Command::Resumed, None, None, false);
}
unsafe extern "C" fn on_pause(_: Activity) {
    send(Command::Paused, None, None, false);
}
unsafe extern "C" fn on_destroy(_: Activity) {
    send(Command::Destroy, None, None, true);
}
unsafe extern "C" fn on_focus(_: Activity, focused: c_int) {
    send(if focused != 0 { Command::Focused } else { Command::Unfocused }, None, None, false);
}
unsafe extern "C" fn on_window_created(_: Activity, window: *mut c_void) {
    send(Command::WindowCreated, Some(window), None, true);
}
unsafe extern "C" fn on_window_resized(_: Activity, _: *mut c_void) {
    send(Command::WindowResized, None, None, false);
}
unsafe extern "C" fn on_window_redraw(_: Activity, _: *mut c_void) {
    send(Command::WindowRedraw, None, None, true);
}
unsafe extern "C" fn on_window_destroyed(_: Activity, _: *mut c_void) {
    send(Command::WindowDestroyed, None, None, true);
}
unsafe extern "C" fn on_input_created(_: Activity, queue: *mut c_void) {
    send(Command::InputCreated, None, Some(queue), true);
}
unsafe extern "C" fn on_input_destroyed(_: Activity, _: *mut c_void) {
    send(Command::InputDestroyed, None, None, true);
}
unsafe extern "C" fn on_configuration(_: Activity) {
    send(Command::ConfigurationChanged, None, None, false);
}
unsafe extern "C" fn on_low_memory(_: Activity) {
    send(Command::LowMemory, None, None, false);
}

/// Sets up an activity Android's just made, and starts `main` on a thread of its own, which is the app's thread from then
/// on. This is what an app's library calls from `ANativeActivity_onCreate`.
///
/// # Safety
/// `activity` is the one Android passed to `ANativeActivity_onCreate`, and this is called from there.
pub unsafe fn on_create(activity: *mut ANativeActivity, main: fn()) {
    send_output_to_the_log();
    let mut ends = [0; 2];
    // SAFETY: room for two descriptors.
    if unsafe { libc::pipe(ends.as_mut_ptr()) } != 0 {
        return;
    }
    // SAFETY: Android hands over a whole activity, whose callbacks are set here, before it calls any.
    unsafe {
        let callbacks = &mut *(*activity).callbacks;
        callbacks.on_resume = Some(on_resume);
        callbacks.on_pause = Some(on_pause);
        callbacks.on_destroy = Some(on_destroy);
        callbacks.on_window_focus_changed = Some(on_focus);
        callbacks.on_native_window_created = Some(on_window_created);
        callbacks.on_native_window_resized = Some(on_window_resized);
        callbacks.on_native_window_redraw_needed = Some(on_window_redraw);
        callbacks.on_native_window_destroyed = Some(on_window_destroyed);
        callbacks.on_input_queue_created = Some(on_input_created);
        callbacks.on_input_queue_destroyed = Some(on_input_destroyed);
        callbacks.on_configuration_changed = Some(on_configuration);
        callbacks.on_low_memory = Some(on_low_memory);
    }
    *lock() = Some(Glue {
        activity,
        write: ends[1],
        read: ends[0],
        window: std::ptr::null_mut(),
        queue: std::ptr::null_mut(),
        done: 0,
        sent: 0,
    });
    let _ = std::thread::Builder::new().name("rsui app".into()).stack_size(8 * 1024 * 1024).spawn(main);
}

/// The app's own folder on the device, which only it can see.
pub fn data_folder() -> Option<std::path::PathBuf> {
    let glue = lock();
    let activity = glue.as_ref()?.activity;
    // SAFETY: the activity lasts as long as the glue does, and its path is a string that ends in a zero.
    unsafe {
        let path = (*activity).internal_data_path;
        (!path.is_null()).then(|| std::path::PathBuf::from(std::ffi::CStr::from_ptr(path).to_string_lossy().into_owned()))
    }
}

/// Copies what's in the app's package under `assets/`, as `rsui-assets.txt` there lists it, a path a line, to `into`,
/// which is where an app's files are read from on the device. What's there already, at the same size, is left.
pub fn copy_assets(into: &std::path::Path) -> Result<usize, String> {
    let manager = {
        let glue = lock();
        let activity = glue.as_ref().ok_or("there's no activity")?.activity;
        // SAFETY: as for `data_folder`.
        unsafe { (*activity).asset_manager }
    };
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

/// Has what's written to the standard output and error go to Android's log, which is where they can be read, with `adb
/// logcat -s rsui`, as nothing else reads them.
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

/// An app on Android has one window, which fills the screen; what kind of window it is doesn't change anything.
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
    let _ = WindowLayer::Top;
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

/// What the app's thread keeps while the app runs.
struct Shown {
    window: *mut c_void,
    renderer: Renderer,
    size: (u32, u32),
}

/// The screen's scale, from its density: 160 dots to the inch is 1.
fn scale_now() -> (f32, bool) {
    let glue = lock();
    let Some(activity) = glue.as_ref().map(|state| state.activity) else { return (1.0, false) };
    // SAFETY: the configuration is made, read and let go of here, from the activity's assets, which last as it does.
    unsafe {
        let config = AConfiguration_new();
        AConfiguration_fromAssetManager(config, (*activity).asset_manager);
        let density = AConfiguration_getDensity(config);
        let night = AConfiguration_getUiModeNight(config) == 2;
        AConfiguration_delete(config);
        ((if density > 0 && density < 0xfffe { density as f32 / 160.0 } else { 1.0 }), night)
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
    let read = lock().as_ref().map(|state| state.read).ok_or_else(|| RunError::Window("there's no activity".to_owned()))?;
    // SAFETY: this thread gets a looper of its own, which the command pipe is added to.
    let looper = unsafe { ALooper_prepare(0) };
    // SAFETY: as above.
    unsafe { ALooper_addFd(looper, read, LOOPER_ID_COMMAND, LOOPER_EVENT_INPUT, std::ptr::null_mut(), std::ptr::null_mut()) };
    let (scale, night) = scale_now();
    let text = crate::default_text_engine()?;
    let mut app = App::new(render, props, text, (1, 1), scale);
    let _ = app.feed(&Input::DarkMode(night));
    let mut shown: Option<Shown> = None;
    let mut queue: *mut c_void = std::ptr::null_mut();
    let mut pressed = false;
    let mut needs_draw = true;
    let mut wake_at: Option<Instant> = None;
    let mut blink_at: Option<Instant> = None;
    loop {
        let now = Instant::now();
        let animating = app.animating() && shown.is_some();
        let mut deadline = wake_at;
        if let Some(blink) = blink_at {
            deadline = Some(deadline.map_or(blink, |at| at.min(blink)));
        }
        let timeout = if animating {
            16
        } else {
            deadline.map_or(-1, |at| at.saturating_duration_since(now).as_millis().min(i32::MAX as u128) as c_int)
        };
        let (mut fd, mut events, mut data) = (0, 0, std::ptr::null_mut());
        // SAFETY: the looper is this thread's, and the outputs have room.
        let ident = unsafe { ALooper_pollOnce(timeout, &raw mut fd, &raw mut events, &raw mut data) };
        if ident == LOOPER_ID_COMMAND {
            let mut byte = 0u8;
            // SAFETY: the read end is this thread's, and there's a byte for it.
            if unsafe { libc::read(read, (&raw mut byte).cast(), 1) } == 1
                && let Some(command) = Command::from_byte(byte)
            {
                let stop = handle_command(command, &mut app, &mut shown, &mut queue, looper, &mut needs_draw);
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
        } else if ident == LOOPER_ID_INPUT && !queue.is_null() {
            read_input(queue, &mut app, &mut pressed, scale_now().0);
        }
        let now = Instant::now();
        if app.animating() {
            app.tick(now);
        }
        if wake_at.is_some_and(|at| at <= now) {
            wake_at = None;
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
        if needs_draw && let Some(shown) = shown.as_mut() {
            needs_draw = false;
            draw(&mut app, shown);
        }
        wake_at = app.next_wake().map(|wait| Instant::now() + wait);
        if app.caret_blinking() && blink_at.is_none() {
            blink_at = Some(Instant::now() + Duration::from_millis(530));
        }
    }
}

/// Deals with what the activity said, and says whether the app's to end.
fn handle_command(
    command: Command,
    app: &mut App,
    shown: &mut Option<Shown>,
    queue: &mut *mut c_void,
    looper: *mut c_void,
    needs_draw: &mut bool,
) -> bool {
    match command {
        Command::WindowCreated => {
            let window = lock().as_ref().map_or(std::ptr::null_mut(), |state| state.window);
            if !window.is_null() {
                // SAFETY: the window's the one the callback was given, which is kept until it says it's gone.
                unsafe { ANativeWindow_acquire(window) };
                let size = window_size(window);
                if let Ok(renderer) = Renderer::offscreen(size) {
                    let (scale, _) = scale_now();
                    app.resize(size, scale);
                    app.renderer_replaced();
                    *shown = Some(Shown { window, renderer, size });
                    *needs_draw = true;
                }
            }
        }
        Command::WindowResized | Command::ConfigurationChanged | Command::WindowRedraw => {
            if let Some(now) = shown.as_mut() {
                let size = window_size(now.window);
                if size != now.size {
                    now.size = size;
                    let _ = now.renderer.resize(size);
                }
                let (scale, night) = scale_now();
                app.resize(size, scale);
                let _ = app.feed(&Input::DarkMode(night));
                *needs_draw = true;
                if command == Command::WindowRedraw {
                    // Android waits for this one, and wants the frame drawn before it carries on.
                    draw(app, now);
                    *needs_draw = false;
                }
            }
        }
        Command::WindowDestroyed => {
            if let Some(gone) = shown.take() {
                // SAFETY: the window was acquired when it came.
                unsafe { ANativeWindow_release(gone.window) };
            }
        }
        Command::InputCreated => {
            let given = lock().as_ref().map_or(std::ptr::null_mut(), |state| state.queue);
            if !given.is_null() {
                // SAFETY: the queue's the one the callback was given, attached to this thread's looper.
                unsafe { AInputQueue_attachLooper(given, looper, LOOPER_ID_INPUT, std::ptr::null_mut(), std::ptr::null_mut()) };
                *queue = given;
            }
        }
        Command::InputDestroyed => {
            if !queue.is_null() {
                // SAFETY: it was attached here.
                unsafe { AInputQueue_detachLooper(*queue) };
                *queue = std::ptr::null_mut();
            }
        }
        Command::Focused => {
            let _ = app.feed(&Input::Active(true));
        }
        Command::Unfocused => {
            let _ = app.feed(&Input::Active(false));
        }
        Command::Resumed => {
            let _ = app.feed(&Input::Shown(true));
            *needs_draw = true;
        }
        Command::Paused => {
            let _ = app.feed(&Input::Shown(false));
        }
        Command::LowMemory => {
            if let Some(shown) = shown.as_mut() {
                shown.renderer.trim();
            }
        }
        Command::Destroy => {
            let _ = app.feed(&Input::Close);
            return true;
        }
    }
    false
}

fn window_size(window: *mut c_void) -> (u32, u32) {
    // SAFETY: the window is held.
    unsafe { (ANativeWindow_getWidth(window).max(1) as u32, ANativeWindow_getHeight(window).max(1) as u32) }
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

/// Takes what's waiting in the input queue: touches, as the pointer, and keys.
fn read_input(queue: *mut c_void, app: &mut App, pressed: &mut bool, _scale: f32) {
    loop {
        let mut event = std::ptr::null_mut();
        // SAFETY: the queue's attached to this thread, and each event is finished with once.
        unsafe {
            if AInputQueue_getEvent(queue, &raw mut event) < 0 {
                return;
            }
            if AInputQueue_preDispatchEvent(queue, event) != 0 {
                continue;
            }
            let handled = match AInputEvent_getType(event) {
                INPUT_MOTION => {
                    let action = AMotionEvent_getAction(event) & 0xff;
                    let (x, y) = (AMotionEvent_getX(event, 0), AMotionEvent_getY(event, 0));
                    match action {
                        0 => {
                            *pressed = true;
                            let _ = app.feed(&Input::Move { x, y });
                            let _ = app.feed(&Input::Down { x, y, clicks: 1 });
                        }
                        2 if *pressed => {
                            let _ = app.feed(&Input::Move { x, y });
                        }
                        1 => {
                            *pressed = false;
                            let _ = app.feed(&Input::Up { x, y });
                            // A finger that's lifted isn't over anything any more.
                            let _ = app.feed(&Input::Leave);
                        }
                        3 => {
                            *pressed = false;
                            let _ = app.feed(&Input::CancelPress);
                            let _ = app.feed(&Input::Leave);
                        }
                        _ => {}
                    }
                    true
                }
                INPUT_KEY => key(event, app),
                _ => false,
            };
            AInputQueue_finishEvent(queue, event, c_int::from(handled));
        }
    }
}

/// A key from a keyboard, or the soft keyboard's simple keys. Back isn't taken, so it does what it does in any app.
unsafe fn key(event: *const c_void, app: &mut App) -> bool {
    // SAFETY: the event's a key event, which the caller holds.
    let (action, code, meta, repeat) =
        unsafe { (AKeyEvent_getAction(event), AKeyEvent_getKeyCode(event), AKeyEvent_getMetaState(event), AKeyEvent_getRepeatCount(event)) };
    let shift = meta & 0x1 != 0;
    let (key, typed) = match code {
        7..=16 => (KeyCode::Character((b'0' + (code - 7) as u8) as char), Some((b'0' + (code - 7) as u8) as char)),
        29..=54 => {
            let letter = (b'a' + (code - 29) as u8) as char;
            (KeyCode::Character(letter), Some(if shift { letter.to_ascii_uppercase() } else { letter }))
        }
        55 => (KeyCode::Character(','), Some(',')),
        56 => (KeyCode::Character('.'), Some('.')),
        61 => (KeyCode::Tab, None),
        62 => (KeyCode::Space, Some(' ')),
        66 => (KeyCode::Enter, None),
        67 => (KeyCode::Backspace, None),
        112 => (KeyCode::Delete, None),
        19 => (KeyCode::Up, None),
        20 => (KeyCode::Down, None),
        21 => (KeyCode::Left, None),
        22 => (KeyCode::Right, None),
        111 => (KeyCode::Escape, None),
        _ => return false,
    };
    let press = KeyPress {
        shift,
        ctrl: meta & 0x1000 != 0,
        alt: meta & 0x2 != 0,
        repeat: repeat > 0,
        typed,
        ..KeyPress::new(key)
    };
    if action == 0 {
        let _ = app.feed(&Input::Key(press));
        if let Some(typed) = typed.filter(|_| !press.ctrl && !press.alt) {
            let _ = app.feed(&Input::Text(typed.to_string()));
        }
    } else {
        let _ = app.feed(&Input::KeyUp(press));
    }
    true
}

/// Asks for the soft keyboard, as a text field that's been touched wants.
pub fn show_keyboard() {
    if let Some(activity) = lock().as_ref().map(|state| state.activity) {
        // SAFETY: the activity's still there while the glue is.
        unsafe { ANativeActivity_showSoftInput(activity, 0) };
    }
}

/// Ends the activity, as closing the app's window would.
pub fn finish() {
    if let Some(activity) = lock().as_ref().map(|state| state.activity) {
        // SAFETY: as above.
        unsafe { ANativeActivity_finish(activity) };
    }
}
