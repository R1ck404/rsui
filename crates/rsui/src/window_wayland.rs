//! The window on Linux, under Wayland: one xdg-shell toplevel, drawn with Vulkan on the GPU where there is one (plan 058,
//! §5.5), and otherwise into shared memory with the CPU renderer.
//!
//! It speaks Wayland through libwayland, loaded when the window opens, as Vulkan needs libwayland's own `wl_display` to
//! draw on the surface. It binds only what the first frame needs: the compositor, shared memory, xdg-shell and the seat,
//! with decorations and cursor shapes if the compositor has them. A compositor that draws no title bar (GNOME's) leaves
//! the app to draw its own, as a frameless window on Windows does, and dragging that moves the window. The keyboard's
//! keymap is compiled by libxkbcommon, which is loaded the first time a keymap arrives.
//!
//! What runs is as on Windows: what comes in is fed to the app, a frame is drawn when it says one's needed, and the
//! thread sleeps in `poll` until the compositor says something or the app's next timer is due.

#![allow(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

use rsui_core::IntoNode;
use rsui_render::Renderer;
use rsui_runtime::Cx;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface, wl_touch,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::wp::cursor_shape::v1::client::{wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1};
use wayland_protocols::xdg::decoration::zv1::client::{zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::replay::Input;
use crate::{App, Cursor, Frame, KeyCode, KeyPress, PointerButton, RunError, WindowId, WindowLayer, WindowOptions};

thread_local! {
    static MAIN_KIND: Cell<(bool, bool, bool)> = const { Cell::new((false, false, false)) };
    static SMALLEST_MAIN: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
    static MAIN_OPENED: Cell<bool> = const { Cell::new(false) };
    static MAIN_MORE: RefCell<WindowOptions> = RefCell::new(WindowOptions::default());
}

pub(crate) fn main_opened() {
    MAIN_OPENED.with(|opened| opened.set(true));
}

pub(crate) fn change_main_kind(change: &crate::WindowChange) -> bool {
    if MAIN_OPENED.with(Cell::get) {
        return false;
    }
    let (mut frameless, mut on_top, mut fullscreen) = MAIN_KIND.with(Cell::get);
    match *change {
        crate::WindowChange::Frameless(on) => frameless = on,
        crate::WindowChange::OnTop(on) => on_top = on,
        crate::WindowChange::Fullscreen(on) => fullscreen = on,
        crate::WindowChange::Layer(layer) => {
            on_top = layer == WindowLayer::Top;
            MAIN_MORE.with(|more| more.borrow_mut().layer = layer);
        }
        crate::WindowChange::Transparent(on) => MAIN_MORE.with(|more| more.borrow_mut().transparent = on),
        _ => return false,
    }
    MAIN_KIND.with(|kind| kind.set((frameless, on_top, fullscreen)));
    true
}

pub fn set_main_window_kind(frameless: bool, on_top: bool, fullscreen: bool) {
    MAIN_KIND.with(|kind| kind.set((frameless, on_top, fullscreen)));
}

pub fn set_main_window_options(options: &WindowOptions) {
    MAIN_MORE.with(|more| {
        let mut more = more.borrow_mut();
        more.layer = options.layer;
        more.transparent = options.transparent;
    });
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

// What the compositor says, as it's heard, for the loop to give the app once a round of it's been read.

enum Heard {
    Configure { size: Option<(u32, u32)>, maximized: bool, activated: bool },
    Close,
    Scale(i32),
    FrameDone,
    PointerEnter { serial: u32, x: f64, y: f64 },
    PointerLeave,
    PointerMove { x: f64, y: f64 },
    Button { serial: u32, button: u32, pressed: bool },
    Wheel { vertical: bool, value: f64 },
    Keymap(OwnedFd, u32),
    Key { key: u32, pressed: bool },
    Modifiers { depressed: u32, latched: u32, locked: u32, group: u32 },
    KeyboardFocus(bool),
    RepeatInfo { rate: i32, delay: i32 },
    TouchDown { serial: u32, x: f64, y: f64 },
    TouchMove { x: f64, y: f64 },
    TouchUp,
}

struct State {
    heard: Vec<Heard>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    touch: Option<wl_touch::WlTouch>,
    /// Which shared-memory buffers the compositor has let go of, and can be drawn into again.
    free: [bool; 2],
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

delegate_noop!(State: wl_compositor::WlCompositor);
delegate_noop!(State: wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: zxdg_decoration_manager_v1::ZxdgDecorationManagerV1);
delegate_noop!(State: ignore zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1);
delegate_noop!(State: wp_cursor_shape_manager_v1::WpCursorShapeManagerV1);
delegate_noop!(State: wp_cursor_shape_device_v1::WpCursorShapeDeviceV1);

impl Dispatch<wl_surface::WlSurface, ()> for State {
    fn event(state: &mut Self, _: &wl_surface::WlSurface, event: wl_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_surface::Event::PreferredBufferScale { factor } = event {
            state.heard.push(Heard::Scale(factor));
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(state: &mut Self, _: &wl_callback::WlCallback, event: wl_callback::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_callback::Event::Done { .. } = event {
            state.heard.push(Heard::FrameDone);
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, usize> for State {
    fn event(state: &mut Self, _: &wl_buffer::WlBuffer, event: wl_buffer::Event, which: &usize, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_buffer::Event::Release = event {
            state.free[*which] = true;
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(_: &mut Self, base: &xdg_wm_base::XdgWmBase, event: xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for State {
    fn event(_: &mut Self, surface: &xdg_surface::XdgSurface, event: xdg_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for State {
    fn event(state: &mut Self, _: &xdg_toplevel::XdgToplevel, event: xdg_toplevel::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            xdg_toplevel::Event::Configure { width, height, states } => {
                let states: Vec<u32> = states.chunks_exact(4).map(|bytes| u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])).collect();
                let size = (width > 0 && height > 0).then_some((width as u32, height as u32));
                state.heard.push(Heard::Configure {
                    size,
                    maximized: states.contains(&(xdg_toplevel::State::Maximized as u32)),
                    activated: states.contains(&(xdg_toplevel::State::Activated as u32)),
                });
            }
            xdg_toplevel::Event::Close => state.heard.push(Heard::Close),
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(state: &mut Self, seat: &wl_seat::WlSeat, event: wl_seat::Event, _: &(), _: &Connection, queue: &QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(capabilities) } = event {
            if capabilities.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(queue, ()));
            }
            if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(queue, ()));
            }
            if capabilities.contains(wl_seat::Capability::Touch) && state.touch.is_none() {
                state.touch = Some(seat.get_touch(queue, ()));
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(state: &mut Self, _: &wl_pointer::WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        state.heard.push(match event {
            wl_pointer::Event::Enter { serial, surface_x, surface_y, .. } => Heard::PointerEnter { serial, x: surface_x, y: surface_y },
            wl_pointer::Event::Leave { .. } => Heard::PointerLeave,
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => Heard::PointerMove { x: surface_x, y: surface_y },
            wl_pointer::Event::Button { serial, button, state: pressed, .. } => {
                Heard::Button { serial, button, pressed: pressed == WEnum::Value(wl_pointer::ButtonState::Pressed) }
            }
            wl_pointer::Event::Axis { axis, value, .. } => {
                Heard::Wheel { vertical: axis == WEnum::Value(wl_pointer::Axis::VerticalScroll), value }
            }
            _ => return,
        });
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(state: &mut Self, _: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        state.heard.push(match event {
            wl_keyboard::Event::Keymap { fd, size, .. } => Heard::Keymap(fd, size),
            wl_keyboard::Event::Enter { .. } => Heard::KeyboardFocus(true),
            wl_keyboard::Event::Leave { .. } => Heard::KeyboardFocus(false),
            wl_keyboard::Event::Key { key, state: pressed, .. } => {
                Heard::Key { key, pressed: pressed == WEnum::Value(wl_keyboard::KeyState::Pressed) }
            }
            wl_keyboard::Event::Modifiers { mods_depressed, mods_latched, mods_locked, group, .. } => {
                Heard::Modifiers { depressed: mods_depressed, latched: mods_latched, locked: mods_locked, group }
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => Heard::RepeatInfo { rate, delay },
            _ => return,
        });
    }
}

impl Dispatch<wl_touch::WlTouch, ()> for State {
    fn event(state: &mut Self, _: &wl_touch::WlTouch, event: wl_touch::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        // One finger is the pointer; others are left until rsui has more than one pointer.
        state.heard.push(match event {
            wl_touch::Event::Down { serial, id: 0, x, y, .. } => Heard::TouchDown { serial, x, y },
            wl_touch::Event::Motion { id: 0, x, y, .. } => Heard::TouchMove { x, y },
            wl_touch::Event::Up { id: 0, .. } | wl_touch::Event::Cancel => Heard::TouchUp,
            _ => return,
        });
    }
}

/// Two buffers in shared memory, drawn into in turn, which the compositor reads frames from: one mapping, which both
/// are halves of.
struct Frames {
    buffers: [wl_buffer::WlBuffer; 2],
    pixels: *mut u8,
    /// How long each buffer is.
    one: usize,
}

impl Frames {
    fn new(shm: &wl_shm::WlShm, queue: &QueueHandle<State>, size: (u32, u32)) -> Option<Self> {
        let stride = size.0 as usize * 4;
        let one = stride * size.1 as usize;
        // SAFETY: a new anonymous file, sized for both, and mapped; each step's checked.
        let (fd, pixels) = unsafe {
            let fd = libc::syscall(libc::SYS_memfd_create, c"rsui-frame".as_ptr(), libc::MFD_CLOEXEC) as c_int;
            if fd < 0 {
                return None;
            }
            let fd = OwnedFd::from_raw_fd(fd);
            if libc::ftruncate(fd.as_raw_fd(), (one * 2) as libc::off_t) != 0 {
                return None;
            }
            let at = libc::mmap(std::ptr::null_mut(), one * 2, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd.as_raw_fd(), 0);
            if at == libc::MAP_FAILED {
                return None;
            }
            (fd, at.cast::<u8>())
        };
        let pool = shm.create_pool(fd.as_fd(), (one * 2) as i32, queue, ());
        let buffer = |which: usize| {
            pool.create_buffer((which * one) as i32, size.0 as i32, size.1 as i32, stride as i32, wl_shm::Format::Argb8888, queue, which)
        };
        let buffers = [buffer(0), buffer(1)];
        pool.destroy();
        Some(Self { buffers, pixels, one })
    }

    /// The pixels of one of the two.
    fn pixels(&mut self, which: usize) -> &mut [u8] {
        // SAFETY: each half of the mapping is one buffer's, and this is borrowed mutably while it's written.
        unsafe { std::slice::from_raw_parts_mut(self.pixels.add(which * self.one), self.one) }
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        for buffer in &self.buffers {
            buffer.destroy();
        }
        // SAFETY: the mapping was made for these, which the compositor's done with once they're destroyed.
        unsafe { libc::munmap(self.pixels.cast(), self.one * 2) };
    }
}

/// libxkbcommon, for turning keys into what they type on the keyboard's layout.
struct Xkb {
    keymap_new_from_string: unsafe extern "C" fn(*mut c_void, *const c_char, c_int, c_int) -> *mut c_void,
    state_new: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    state_unref: unsafe extern "C" fn(*mut c_void),
    keymap_unref: unsafe extern "C" fn(*mut c_void),
    state_update_mask: unsafe extern "C" fn(*mut c_void, u32, u32, u32, u32, u32, u32) -> c_int,
    state_key_get_one_sym: unsafe extern "C" fn(*mut c_void, u32) -> u32,
    state_key_get_utf8: unsafe extern "C" fn(*mut c_void, u32, *mut c_char, usize) -> c_int,
    keymap_key_repeats: unsafe extern "C" fn(*mut c_void, u32) -> c_int,
    context: *mut c_void,
}

fn xkb() -> Option<&'static Xkb> {
    struct Found(Option<Xkb>);
    // SAFETY: the library's functions and its context are used from the window's thread only.
    unsafe impl Send for Found {}
    unsafe impl Sync for Found {}
    static XKB: std::sync::OnceLock<Found> = std::sync::OnceLock::new();
    XKB.get_or_init(|| {
        // SAFETY: the names end in zeros; the library stays loaded for good, and each function has the type its header
        // gives it.
        Found(unsafe {
            (|| {
                let library = [c"libxkbcommon.so.0", c"libxkbcommon.so"].iter().find_map(|name| {
                    let found = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
                    (!found.is_null()).then_some(found)
                })?;
                let find = |name: &CStr| {
                    let found = libc::dlsym(library, name.as_ptr());
                    (!found.is_null()).then_some(found)
                };
                let context_new: unsafe extern "C" fn(c_int) -> *mut c_void = std::mem::transmute(find(c"xkb_context_new")?);
                let context = context_new(0);
                if context.is_null() {
                    return None;
                }
                Some(Xkb {
                    keymap_new_from_string: std::mem::transmute(find(c"xkb_keymap_new_from_string")?),
                    state_new: std::mem::transmute(find(c"xkb_state_new")?),
                    state_unref: std::mem::transmute(find(c"xkb_state_unref")?),
                    keymap_unref: std::mem::transmute(find(c"xkb_keymap_unref")?),
                    state_update_mask: std::mem::transmute(find(c"xkb_state_update_mask")?),
                    state_key_get_one_sym: std::mem::transmute(find(c"xkb_state_key_get_one_sym")?),
                    state_key_get_utf8: std::mem::transmute(find(c"xkb_state_key_get_utf8")?),
                    keymap_key_repeats: std::mem::transmute(find(c"xkb_keymap_key_repeats")?),
                    context,
                })
            })()
        })
    })
    .0
    .as_ref()
}

/// The keyboard's layout, as libxkbcommon has it, and which modifiers are held.
struct Keyboard {
    keymap: *mut c_void,
    state: *mut c_void,
    shift: bool,
    ctrl: bool,
    alt: bool,
    meta: bool,
}

impl Keyboard {
    fn new(fd: OwnedFd, size: u32) -> Option<Self> {
        let xkb = xkb()?;
        // SAFETY: the compositor's keymap, a string of `size` bytes ending in a zero, mapped to read and let go of after.
        unsafe {
            let text = libc::mmap(std::ptr::null_mut(), size as usize, libc::PROT_READ, libc::MAP_PRIVATE, fd.as_raw_fd(), 0);
            if text == libc::MAP_FAILED {
                return None;
            }
            let keymap = (xkb.keymap_new_from_string)(xkb.context, text.cast(), 1, 0);
            libc::munmap(text, size as usize);
            if keymap.is_null() {
                return None;
            }
            let state = (xkb.state_new)(keymap);
            Some(Self { keymap, state, shift: false, ctrl: false, alt: false, meta: false })
        }
    }

    fn modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        let Some(xkb) = xkb() else { return };
        // SAFETY: the state's this keyboard's.
        unsafe { (xkb.state_update_mask)(self.state, depressed, latched, locked, 0, 0, group) };
        // The usual places of the usual modifiers in every keymap X has had.
        let held = depressed | latched;
        self.shift = held & 0x1 != 0;
        self.ctrl = held & 0x4 != 0;
        self.alt = held & 0x8 != 0;
        self.meta = held & 0x40 != 0;
    }

    /// The key a Linux key code is, what it types, and whether it repeats when held.
    fn key(&self, key: u32) -> (Option<KeyCode>, Option<String>, bool) {
        let Some(xkb) = xkb() else { return (None, None, false) };
        let code = key + 8;
        // SAFETY: the state and keymap are this keyboard's; the text has room for what's asked.
        let (symbol, typed, repeats) = unsafe {
            let symbol = (xkb.state_key_get_one_sym)(self.state, code);
            let mut text = [0 as c_char; 16];
            let length = (xkb.state_key_get_utf8)(self.state, code, text.as_mut_ptr(), text.len());
            let typed = (length > 0).then(|| CStr::from_ptr(text.as_ptr()).to_string_lossy().into_owned());
            (symbol, typed, (xkb.keymap_key_repeats)(self.keymap, code) != 0)
        };
        let named = match symbol {
            0xff09 | 0xfe20 => Some(KeyCode::Tab),
            0xff0d | 0xff8d => Some(KeyCode::Enter),
            0xff1b => Some(KeyCode::Escape),
            0xff08 => Some(KeyCode::Backspace),
            0xffff => Some(KeyCode::Delete),
            0x0020 => Some(KeyCode::Space),
            0xff51 => Some(KeyCode::Left),
            0xff52 => Some(KeyCode::Up),
            0xff53 => Some(KeyCode::Right),
            0xff54 => Some(KeyCode::Down),
            0xff50 => Some(KeyCode::Home),
            0xff57 => Some(KeyCode::End),
            0xff55 => Some(KeyCode::PageUp),
            0xff56 => Some(KeyCode::PageDown),
            0xff63 => Some(KeyCode::Insert),
            0xffe1 | 0xffe2 => Some(KeyCode::Shift),
            0xffe3 | 0xffe4 => Some(KeyCode::Control),
            0xffe9 | 0xffea => Some(KeyCode::Alt),
            0xffeb | 0xffec => Some(KeyCode::Meta),
            0xffe5 => Some(KeyCode::CapsLock),
            0xffbe..=0xffd5 => Some(KeyCode::Function((symbol - 0xffbe + 1) as u8)),
            _ => None,
        };
        let code = named.or_else(|| {
            let character = typed.as_deref()?.chars().next()?;
            (!character.is_control()).then(|| KeyCode::Character(character.to_lowercase().next().unwrap_or(character)))
        });
        // What a named key types (Enter's carriage return, Tab, Backspace) is the key, not text.
        let typed = typed.filter(|_| named.is_none() || named == Some(KeyCode::Space)).filter(|text| !text.chars().any(char::is_control));
        (code, typed, repeats)
    }
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        if let Some(xkb) = xkb() {
            // SAFETY: both are this keyboard's, let go of once.
            unsafe {
                (xkb.state_unref)(self.state);
                (xkb.keymap_unref)(self.keymap);
            }
        }
    }
}

/// Opens the main window, showing `render`, and runs until it's closed.
pub fn run_sized<P: 'static, R: IntoNode + 'static>(
    title: &str,
    size: Option<(u32, u32)>,
    render: fn(&mut Cx<'_>, &P) -> R,
    props: P,
) -> Result<(), RunError> {
    MAIN_OPENED.with(|opened| opened.set(true));
    let failed = |what: &str, error: &dyn std::fmt::Display| RunError::Window(format!("{what}: {error}"));
    let connection = Connection::connect_to_env().map_err(|error| failed("there's no Wayland compositor to talk to", &error))?;
    let (globals, mut queue) = registry_queue_init::<State>(&connection).map_err(|error| failed("Wayland", &error))?;
    let handle = queue.handle();
    let compositor: wl_compositor::WlCompositor =
        globals.bind(&handle, 4..=6, ()).map_err(|error| failed("the compositor", &error))?;
    let shm: wl_shm::WlShm = globals.bind(&handle, 1..=1, ()).map_err(|error| failed("shared memory", &error))?;
    let shell: xdg_wm_base::XdgWmBase = globals.bind(&handle, 1..=5, ()).map_err(|error| failed("xdg-shell", &error))?;
    let decorations: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1> = globals.bind(&handle, 1..=1, ()).ok();
    let cursors: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1> = globals.bind(&handle, 1..=1, ()).ok();
    let mut state = State { heard: Vec::new(), seat: None, pointer: None, keyboard: None, touch: None, free: [true; 2] };
    state.seat = globals.bind::<wl_seat::WlSeat, _, _>(&handle, 1..=8, ()).ok();

    let options = WindowOptions { title: title.to_owned(), size, ..main_window_options() };
    let surface = compositor.create_surface(&handle, ());
    let xdg = shell.get_xdg_surface(&surface, &handle, ());
    let toplevel = xdg.get_toplevel(&handle, ());
    toplevel.set_title(options.title.clone());
    toplevel.set_app_id(app_id());
    if let Some((width, height)) = options.smallest {
        toplevel.set_min_size(width as i32, height as i32);
    }
    // The compositor draws the title bar if it will; an app that draws its own (frameless) doesn't want one.
    if let Some(decorations) = &decorations {
        let decoration = decorations.get_toplevel_decoration(&toplevel, &handle, ());
        decoration.set_mode(if options.frameless {
            zxdg_toplevel_decoration_v1::Mode::ClientSide
        } else {
            zxdg_toplevel_decoration_v1::Mode::ServerSide
        });
    }
    if options.fullscreen {
        toplevel.set_fullscreen(None);
    }
    surface.commit();
    // The first configure says how big the compositor would like it.
    queue.roundtrip(&mut state).map_err(|error| failed("Wayland", &error))?;

    let mut scale: i32 = 1;
    let mut logical = options.size.unwrap_or((1100, 720));
    for heard in state.heard.drain(..) {
        match heard {
            Heard::Configure { size: Some(given), .. } => logical = given,
            Heard::Scale(factor) => scale = factor.max(1),
            _ => {}
        }
    }
    let physical = |logical: (u32, u32), scale: i32| (logical.0 * scale as u32, logical.1 * scale as u32);
    let mut size_now = physical(logical, scale);
    let text = crate::default_text_engine()?;
    let mut app = App::new(render, props, text, size_now, scale as f32);
    // What another thread wakes the loop with when the app has something to show, as a page's timer or an image that's
    // been read does: an eventfd, which the loop sleeps on beside the connection. The waker keeps it open for as long as
    // it might be called.
    // SAFETY: a new descriptor, owned from here on if it was made.
    let wake = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
    let wake = (wake >= 0).then(|| std::sync::Arc::new(unsafe { OwnedFd::from_raw_fd(wake) }));
    if let Some(wake) = wake.clone() {
        app.set_waker(move || {
            let one = 1u64;
            // SAFETY: eight bytes to an eventfd that's open, as this holds it.
            unsafe { libc::write(wake.as_raw_fd(), (&raw const one).cast(), 8) };
        });
    }
    // Declared after the surface and connection, so that it's dropped before them, as it draws on them.
    let mut renderer = match gpu_renderer(&connection, &surface, size_now) {
        Some(renderer) => renderer,
        None => Renderer::offscreen(size_now).map_err(RunError::Render)?,
    };
    let mut frames = if renderer.draws_to_window() { None } else { Frames::new(&shm, &handle, size_now) };
    // Whether a frame callback's been asked for that no commit has taken yet: Vulkan commits when it presents, and a frame
    // that turns out to be the same as the last presents nothing.
    let mut frame_asked = false;
    let mut next_buffer = 0usize;
    let mut cursor_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1> = None;
    let mut cursor_now: Option<Cursor> = None;
    let mut pointer_serial = 0u32;
    let mut pointer_at = (0.0f32, 0.0f32);
    let mut keyboard: Option<Keyboard> = None;
    let mut repeat = (25i32, 400i32);
    let mut repeating: Option<(u32, Instant)> = None;
    let mut waiting_for_frame = false;
    let mut needs_draw = true;
    let mut wake_at: Option<Instant> = None;
    let mut blink_at: Option<Instant> = None;
    let mut maximized = false;

    loop {
        queue.dispatch_pending(&mut state).map_err(|error| failed("Wayland", &error))?;
        let now = Instant::now();
        for heard in std::mem::take(&mut state.heard) {
            match heard {
                Heard::Configure { size, maximized: is, activated } => {
                    maximized = is;
                    if let Some(given) = size
                        && given != logical
                    {
                        logical = given;
                    }
                    let _ = app.feed(&Input::Active(activated));
                    needs_draw = true;
                }
                Heard::Scale(factor) => {
                    scale = factor.max(1);
                    needs_draw = true;
                }
                Heard::Close => {
                    if !app.feed(&Input::Close).unwrap_or(false) {
                        return Ok(());
                    }
                }
                Heard::FrameDone => waiting_for_frame = false,
                Heard::PointerEnter { serial, x, y } => {
                    pointer_serial = serial;
                    pointer_at = (x as f32 * scale as f32, y as f32 * scale as f32);
                    cursor_now = None;
                    if cursor_device.is_none()
                        && let (Some(cursors), Some(pointer)) = (&cursors, &state.pointer)
                    {
                        cursor_device = Some(cursors.get_pointer(pointer, &handle, ()));
                    }
                    let _ = app.feed(&Input::Move { x: pointer_at.0, y: pointer_at.1 });
                }
                Heard::PointerLeave => {
                    let _ = app.feed(&Input::Leave);
                }
                Heard::PointerMove { x, y } => {
                    pointer_at = (x as f32 * scale as f32, y as f32 * scale as f32);
                    app.set_input_time(now);
                    let _ = app.feed(&Input::Move { x: pointer_at.0, y: pointer_at.1 });
                }
                Heard::Button { serial, button, pressed } => {
                    pointer_serial = serial;
                    app.set_input_time(now);
                    let (x, y) = pointer_at;
                    // Linux's button numbers, from linux/input-event-codes.h.
                    match (button, pressed) {
                        (0x110, true) => {
                            if app.drags_window_at(x, y)
                                && let Some(seat) = &state.seat
                            {
                                toplevel._move(seat, serial);
                            } else {
                                let _ = app.feed(&Input::Down { x, y, clicks: 1 });
                            }
                        }
                        (0x110, false) => {
                            let _ = app.feed(&Input::Up { x, y });
                        }
                        (0x111 | 0x112 | 0x113 | 0x114, _) => {
                            let which = match button {
                                0x111 => PointerButton::Secondary,
                                0x112 => PointerButton::Middle,
                                0x113 => PointerButton::Back,
                                _ => PointerButton::Forward,
                            };
                            let _ = app.feed(&if pressed {
                                Input::ButtonDown { x, y, button: which }
                            } else {
                                Input::ButtonUp { x, y, button: which }
                            });
                        }
                        _ => {}
                    }
                }
                Heard::Wheel { vertical, value } => {
                    app.set_input_time(now);
                    let (x, y) = pointer_at;
                    let _ = app.feed(&Input::ScrollPixels { x, y, pixels: value as f32, across: !vertical, precise: false });
                }
                Heard::Keymap(fd, size) => keyboard = Keyboard::new(fd, size),
                Heard::Modifiers { depressed, latched, locked, group } => {
                    if let Some(keyboard) = keyboard.as_mut() {
                        keyboard.modifiers(depressed, latched, locked, group);
                        let _ = app.feed(&Input::Modifiers(crate::Modifiers {
                            shift: keyboard.shift,
                            ctrl: keyboard.ctrl,
                            alt: keyboard.alt,
                            meta: keyboard.meta,
                        }));
                    }
                }
                Heard::KeyboardFocus(on) => {
                    repeating = None;
                    let _ = app.feed(&Input::Active(on));
                }
                Heard::RepeatInfo { rate, delay } => repeat = (rate, delay),
                Heard::Key { key, pressed } => {
                    app.set_input_time(now);
                    if let Some(keyboard) = keyboard.as_ref() {
                        let repeats = press_key(&mut app, keyboard, key, pressed, false);
                        repeating = match (pressed, repeats, repeat.0 > 0) {
                            (true, true, true) => Some((key, now + Duration::from_millis(repeat.1.max(0) as u64))),
                            (false, _, _) if repeating.is_some_and(|(held, _)| held == key) => None,
                            (_, _, _) => repeating,
                        };
                    }
                }
                Heard::TouchDown { serial, x, y } => {
                    pointer_serial = serial;
                    pointer_at = (x as f32 * scale as f32, y as f32 * scale as f32);
                    let (x, y) = pointer_at;
                    let _ = app.feed(&Input::Move { x, y });
                    let _ = app.feed(&Input::Down { x, y, clicks: 1 });
                }
                Heard::TouchMove { x, y } => {
                    pointer_at = (x as f32 * scale as f32, y as f32 * scale as f32);
                    let _ = app.feed(&Input::Move { x: pointer_at.0, y: pointer_at.1 });
                }
                Heard::TouchUp => {
                    let (x, y) = pointer_at;
                    let _ = app.feed(&Input::Up { x, y });
                    let _ = app.feed(&Input::Leave);
                }
            }
        }
        let _ = maximized;
        // A held key types again and again, as fast as the compositor says.
        if let (Some((key, at)), Some(keyboard)) = (repeating, keyboard.as_ref())
            && at <= now
        {
            press_key(&mut app, keyboard, key, true, true);
            repeating = Some((key, now + Duration::from_millis(1000 / repeat.0.max(1) as u64)));
        }
        let wanted = physical(logical, scale);
        if wanted != size_now {
            size_now = wanted;
            let _ = renderer.resize(size_now);
            if !renderer.draws_to_window() {
                frames = Frames::new(&shm, &handle, size_now);
                state.free = [true; 2];
            }
            app.resize(size_now, scale as f32);
            needs_draw = true;
        }
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
        if let Some(title) = app.take_title_wanted() {
            toplevel.set_title(title.to_string());
        }
        for url in app.take_urls_wanted() {
            crate::system::open_url(&url);
        }
        if let Some(device) = &cursor_device
            && cursor_now != Some(app.cursor())
        {
            cursor_now = Some(app.cursor());
            device.set_shape(
                pointer_serial,
                match app.cursor() {
                    Cursor::Arrow => wp_cursor_shape_device_v1::Shape::Default,
                    Cursor::Hand => wp_cursor_shape_device_v1::Shape::Pointer,
                    Cursor::Text => wp_cursor_shape_device_v1::Shape::Text,
                },
            );
        }
        if needs_draw && !waiting_for_frame && renderer.draws_to_window() {
            // The frame callback and the scale go with the commit Vulkan makes when it presents.
            if !frame_asked {
                surface.frame(&handle, ());
                frame_asked = true;
            }
            surface.set_buffer_scale(scale);
            match app.render(&mut renderer) {
                Ok(()) => {
                    if !renderer.showed_nothing() {
                        waiting_for_frame = true;
                        frame_asked = false;
                    }
                    needs_draw = false;
                }
                Err(error) => {
                    // The GPU's gone, or never could draw here: the CPU draws from now on, into shared memory.
                    if std::env::var_os("RSUI_VULKAN").is_some() {
                        eprintln!("rsui: Vulkan stopped drawing ({error}); drawing with the CPU");
                    }
                    renderer = Renderer::offscreen(size_now).map_err(RunError::Render)?;
                    frames = Frames::new(&shm, &handle, size_now);
                    state.free = [true; 2];
                    app.renderer_replaced();
                }
            }
        } else if needs_draw && !waiting_for_frame && let Some(frames) = frames.as_mut() {
            let which = if state.free[next_buffer] { Some(next_buffer) } else if state.free[1 - next_buffer] { Some(1 - next_buffer) } else { None };
            if let Some(which) = which
                && app.render(&mut renderer).is_ok()
                && let Ok(pixels) = renderer.read_pixels()
            {
                // RGBA, colors multiplied by alpha, to what Wayland calls ARGB8888, which is B, G, R, A in memory. The
                // compositor isn't reading this buffer, as it's let go of it.
                for (to, from) in frames.pixels(which).chunks_exact_mut(4).zip(pixels.chunks_exact(4)) {
                    to.copy_from_slice(&[from[2], from[1], from[0], from[3]]);
                }
                state.free[which] = false;
                next_buffer = 1 - which;
                surface.set_buffer_scale(scale);
                surface.attach(Some(&frames.buffers[which]), 0, 0);
                surface.damage_buffer(0, 0, size_now.0 as i32, size_now.1 as i32);
                surface.frame(&handle, ());
                surface.commit();
                waiting_for_frame = true;
                needs_draw = false;
            }
        }
        wake_at = app.next_wake().map(|wait| Instant::now() + wait);
        if app.caret_blinking() && blink_at.is_none() {
            blink_at = Some(Instant::now() + Duration::from_millis(530));
        }
        let _ = connection.flush();
        // Sleep until the compositor says something, or the next thing that's due.
        let mut due = [wake_at, blink_at, repeating.map(|(_, at)| at)].into_iter().flatten().min();
        if app.animating() && !waiting_for_frame {
            due = Some(now);
        }
        let Some(guard) = queue.prepare_read() else { continue };
        let timeout = due.map_or(-1, |at| at.saturating_duration_since(Instant::now()).as_millis().min(i32::MAX as u128) as c_int);
        let mut polls = [
            libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: wake.as_ref().map_or(-1, |wake| wake.as_raw_fd()), events: libc::POLLIN, revents: 0 },
        ];
        // SAFETY: the connection's descriptor and the eventfd, which are open; a negative one is left out.
        let ready = unsafe { libc::poll(polls.as_mut_ptr(), polls.len() as libc::nfds_t, timeout) };
        if ready > 0 && polls[1].revents != 0 {
            let mut count = 0u64;
            // SAFETY: reading the eventfd's count, eight bytes, empties it.
            unsafe { libc::read(polls[1].fd, (&raw mut count).cast(), 8) };
        }
        if ready > 0 && polls[0].revents != 0 {
            guard.read().map_err(|error| failed("Wayland", &error))?;
        }
    }
}

/// A renderer that draws on the surface with Vulkan, or nothing if Vulkan can't, for the CPU to draw instead. Setting
/// `RSUI_VULKAN` (to anything) says which it is and why.
fn gpu_renderer(connection: &Connection, surface: &wl_surface::WlSurface, size: (u32, u32)) -> Option<Renderer> {
    let display = connection.backend().display_ptr().cast::<c_void>();
    let target = surface.id().as_ptr().cast::<c_void>();
    let told = std::env::var_os("RSUI_VULKAN").is_some();
    if display.is_null() || target.is_null() {
        return None;
    }
    // SAFETY: both are libwayland's, alive for as long as the window's loop runs, and the renderer is dropped before
    // either goes.
    match unsafe { Renderer::for_wayland(display, target, size) } {
        Ok(renderer) => {
            if told {
                eprintln!("rsui: drawing with Vulkan");
            }
            Some(renderer)
        }
        Err(error) => {
            if told {
                eprintln!("rsui: drawing with the CPU, as {error}");
            }
            None
        }
    }
}

/// Feeds a key to the app, and what it types; says whether it repeats when held.
fn press_key(app: &mut App, keyboard: &Keyboard, key: u32, pressed: bool, repeat: bool) -> bool {
    let (code, typed, repeats) = keyboard.key(key);
    let Some(code) = code else { return false };
    let press = KeyPress {
        ctrl: keyboard.ctrl,
        shift: keyboard.shift,
        alt: keyboard.alt,
        meta: keyboard.meta,
        repeat,
        typed: typed.as_deref().and_then(|text| text.chars().next()),
        ..KeyPress::new(code)
    };
    if pressed {
        let _ = app.feed(&Input::Key(press));
        if let Some(text) = typed.filter(|_| !keyboard.ctrl && !keyboard.alt) {
            let _ = app.feed(&Input::Text(text));
        }
    } else {
        let _ = app.feed(&Input::KeyUp(press));
    }
    repeats
}

/// What the desktop knows the program by: its file's name.
fn app_id() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "rsui".to_owned())
}
