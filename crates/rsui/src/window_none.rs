//! The main window, on systems whose window system isn't written yet (it's `window.rs` on Windows). What a program says
//! about its main window before it opens is kept, as it is there, so that code asking about it gets the same answers; and
//! opening one says there's no window system to open it with. What runs without a window, as `rsui-tsx --do` and the
//! tests do, needs none of this.

use std::cell::{Cell, RefCell};

use rsui_core::IntoNode;
use rsui_runtime::Cx;

use crate::{RunError, WindowId, WindowLayer, WindowOptions};

thread_local! {
    /// Whether the main window has no frame, is above other windows, and fills the screen.
    static MAIN_KIND: Cell<(bool, bool, bool)> = const { Cell::new((false, false, false)) };
    /// How small the main window can be made, inside its edges, in logical pixels.
    static SMALLEST_MAIN: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
    static MAIN_OPENED: Cell<bool> = const { Cell::new(false) };
    static MAIN_MORE: RefCell<WindowOptions> = RefCell::new(WindowOptions::default());
}

/// Says the main window's been opened, or that what's running has no window to open.
pub(crate) fn main_opened() {
    MAIN_OPENED.with(|opened| opened.set(true));
}

/// Makes a change to what kind of window the main one is, if it hasn't been opened yet, and says whether it did.
pub(crate) fn change_main_kind(change: &crate::WindowChange) -> bool {
    if MAIN_OPENED.with(Cell::get) {
        return false;
    }
    let (mut frameless, mut on_top, mut fullscreen) = MAIN_KIND.with(Cell::get);
    match *change {
        crate::WindowChange::Frameless(on) => frameless = on,
        crate::WindowChange::OnTop(on) => {
            on_top = on;
            MAIN_MORE.with(|more| {
                let mut more = more.borrow_mut();
                more.layer = more.layer.after_on_top(on);
            });
        }
        crate::WindowChange::Fullscreen(on) => fullscreen = on,
        crate::WindowChange::Layer(layer) => {
            on_top = layer == WindowLayer::Top;
            MAIN_MORE.with(|more| more.borrow_mut().layer = layer);
        }
        crate::WindowChange::Transparent(on) => MAIN_MORE.with(|more| more.borrow_mut().transparent = on),
        crate::WindowChange::SkipTaskbar(on) => MAIN_MORE.with(|more| more.borrow_mut().skip_taskbar = on),
        crate::WindowChange::AllDesktops(on) => MAIN_MORE.with(|more| more.borrow_mut().all_desktops = on),
        crate::WindowChange::AppBar(bar) => MAIN_MORE.with(|more| more.borrow_mut().app_bar = bar),
        _ => return false,
    }
    MAIN_KIND.with(|kind| kind.set((frameless, on_top, fullscreen)));
    true
}

/// Says what kind of window the main one is. Set this before opening it, on the same thread.
pub fn set_main_window_kind(frameless: bool, on_top: bool, fullscreen: bool) {
    MAIN_KIND.with(|kind| kind.set((frameless, on_top, fullscreen)));
}

/// How the main window opens, for what [`set_main_window_kind`] doesn't say.
pub fn set_main_window_options(options: &WindowOptions) {
    MAIN_MORE.with(|more| {
        let mut more = more.borrow_mut();
        more.layer = options.layer;
        more.transparent = options.transparent;
        more.skip_taskbar = options.skip_taskbar;
        more.all_desktops = options.all_desktops;
        more.app_bar = options.app_bar;
    });
}

/// How the main window will open, as far as what's been said so far.
pub fn main_window_options() -> WindowOptions {
    let (frameless, on_top, fullscreen) = MAIN_KIND.with(Cell::get);
    let mut options = MAIN_MORE.with(|more| more.borrow().clone());
    options.frameless = frameless;
    options.on_top = on_top;
    options.fullscreen = fullscreen;
    options.owned = false;
    options.smallest = SMALLEST_MAIN.with(Cell::get);
    options
}

/// Says how small the main window can be made, inside its edges, in logical pixels.
pub fn set_smallest_window_size(width: u32, height: u32) {
    SMALLEST_MAIN.with(|smallest| smallest.set(Some((width, height))));
}

/// There are no windows to hear about here yet.
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

/// Opens a window showing `render` as its root component, and runs until it's closed: here, says it can't yet.
pub fn run<P: 'static, R: IntoNode + 'static>(
    title: &str,
    render: fn(&mut Cx<'_>, &P) -> R,
    props: P,
) -> Result<(), RunError> {
    run_sized(title, None, render, props)
}

/// [`run`], with room for this much inside the window.
pub fn run_sized<P: 'static, R: IntoNode + 'static>(
    _title: &str,
    _size: Option<(u32, u32)>,
    _render: fn(&mut Cx<'_>, &P) -> R,
    _props: P,
) -> Result<(), RunError> {
    MAIN_OPENED.with(|opened| opened.set(true));
    Err(RunError::Window("windows can't be opened on this system yet: run it headless (--do) instead".to_owned()))
}
