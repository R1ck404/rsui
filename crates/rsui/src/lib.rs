//! A fast, memory-frugal UI framework, with every part written from scratch.
//!
//! A component is a function from props to an element. Its state lives in hooks, and only
//! what changed renders again:
//!
//! ```no_run
//! use rsui::prelude::*;
//!
//! fn counter(cx: &mut Cx, _: &()) -> Element {
//!     let count = cx.state(|| 0);
//!     let label = Element::new(Role::Text)
//!         .name(text!("Clicked {} times", cx.get(count)));
//!     let button = Element::new(Role::Button).key("add").name("Add one").props(style!(
//!         Style::new().padding_xy(14.0, 8.0),
//!         Paint::new().background(Color::hex(0x1F6FEB)).text(Color::WHITE).corner_radius(6.0),
//!     ));
//!     let button = cx.on(button, ActionKind::Click, move |cx, _| {
//!         cx.update(count, |count| *count += 1);
//!     });
//!     Element::new(Role::Window)
//!         .props(style!(Style::new().padding(24.0).gap(12.0).align(Align::Start)))
//!         .child(label)
//!         .child(button)
//! }
//!
//! fn main() {
//!     rsui::run("Counter", counter, ()).unwrap();
//! }
//! ```
//!
//! The keyboard and pointer need nothing set up. Tab reaches anything that can be clicked, and
//! enter or space then clicks it. [`text_input`] gives you an editable line of text, with
//! selection, clipboard and undo. Give an element an [`ActionKind::KeyPress`](prelude::ActionKind)
//! handler for shortcuts, which hears keys like `"ctrl+s"` from anywhere inside it. Hover and
//! pressed colors are part of [`Paint`](prelude::Paint), and cost no rendering at all.
//!
//! Each crate this is made of can be used by itself. They're also here as modules.

pub mod access;
mod app;
#[cfg(feature = "mutation-pilot")]
pub use app::mutation_pilot::MutationPilot;
#[cfg(windows)]
pub mod desktop;
// Each system's own desktop services come in as they're written; until then, the same names that do nothing.
#[cfg(not(windows))]
#[path = "desktop_none.rs"]
pub mod desktop;
pub mod dialogs;
mod dock;
#[cfg(windows)]
mod drop;
pub mod edit;
pub mod embed;
pub mod geometry;
mod input;
mod input_log;
pub mod inspect;
mod pill;
mod marks;
#[cfg(feature = "idle-probe")]
mod native_cycles;
pub mod replay;
#[cfg(windows)]
mod system;
#[cfg(not(windows))]
#[path = "system_none.rs"]
mod system;
#[cfg(windows)]
mod uia;
mod widgets;
#[cfg(windows)]
mod shell_layer;
#[cfg(windows)]
mod window;
// Android's window is the activity's (see `rsui::android`).
#[cfg(target_os = "android")]
#[path = "window_android.rs"]
mod window;
// Linux's window is Wayland's (X11 comes later).
#[cfg(target_os = "linux")]
#[path = "window_wayland.rs"]
mod window;
// AppKit on macOS comes in here as it's written. Until then, what runs headless works, and opening a window says it can't.
#[cfg(not(any(windows, target_os = "android", target_os = "linux")))]
#[path = "window_none.rs"]
mod window;

/// What an app's library on Android uses: starting the app when rsui's activity is made, handing on what the activity
/// hears, and the app's own folder and files.
#[cfg(target_os = "android")]
pub mod android {
    pub use crate::window::{
        commit, compose, configuration, copy_assets, data_folder, delete, destroy, finish_composing, focus, key, low_memory, redraw, shown,
        start, surface, touch,
    };
}
mod windows;

pub use app::{
    App, DropPhase, EditSaid, FieldSelection, Frame, Quiet, RunError, default_text_engine, edit_in_progress, field_selection, on_quiet, window_scale,
};
pub use dock::{DockRequest, DockSide};
pub use input::{Clipboard, Cursor, KeyCode, KeyPress, MemoryClipboard, Modifiers, PointerButton};
pub use inspect::{FrameRecord, allow_inspector, inspect_from_start, summary};
pub use marks::mark;
pub use rsui_render::register_image;
pub use system::{can_open_url, open_url, system_is_dark};
pub use widgets::{MenuAction, dialog, image, menu_button, select, text_area, text_input};
pub use window::{capture_window, main_window_options, on_media_key, on_thumbnail_button, on_window_captured, on_window_state_change, run, run_sized, set_main_window_kind, set_main_window_options, set_smallest_window_size, window_handle, window_state};
pub use windows::{
    AppBar, MakeApp, ScreenEdge, ThumbnailButton, WantedWindow, WindowChange, WindowId, WindowLayer, WindowOptions, WindowRequests, WindowStateNow, change_window,
    close_window, open_window, set_window_title, take_window_requests,
};

pub use rsui_core as core;
pub use rsui_headless as headless;
pub use rsui_layout as layout;
pub use rsui_render as render;
pub use rsui_runtime as runtime;
pub use rsui_text as text;

/// The allocator rsui is built around, which is about twice as fast as the Windows heap for
/// UI work, and uses less memory. It's already chosen if you use [`run`] through a program
/// with this line, which every program should have:
///
/// ```
/// #[global_allocator]
/// static ALLOCATOR: rsui::UiAlloc = rsui::UiAlloc;
/// ```
pub use rsui_alloc::UiAlloc;
#[doc(hidden)]
pub use rsui_alloc::owners as alloc_owners;
#[doc(hidden)]
pub use rsui_alloc::regions as alloc_regions;
pub use rsui_alloc::split;

/// Everything most programs need.
pub mod prelude {
    pub use rsui_core::{ActionKind, Element, IntoNode, Node, PropValue, Role, Text, text};
    pub use rsui_layout::{Align, Justify, Placement, Style, style};
    pub use rsui_render::{Color, Paint};
    pub use rsui_runtime::{Cx, ListWindow, ScrollAxis, State, Updates};

    pub use crate::widgets::{
        MenuAction, dialog, image, menu_button, select, text_area, text_input,
    };
}
