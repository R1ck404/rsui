//! Asking for more windows. Each has an [`App`] of its own, all on one thread.
//!
//! Asking only notes what's wanted, as this can be asked for in the middle of something a window's doing, and what's
//! done about it happens once that's over. With no windows, as in tests, it's all there to be taken with
//! [`take_window_requests`] and done however suits.

use std::cell::{Cell, RefCell};

use rsui_core::IntoNode;
use rsui_runtime::Cx;
use rsui_text::TextEngine;

use crate::app::App;

/// Which window this is. The first one, which the program ends with, is always [`WindowId::MAIN`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub u32);

impl WindowId {
    pub const MAIN: Self = Self(0);
}

/// How a window should be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowOptions {
    pub title: String,
    /// How much room there is inside it, in logical pixels, or what the system usually gives one.
    pub size: Option<(u32, u32)>,
    /// How small it can be made inside, in logical pixels.
    pub smallest: Option<(u32, u32)>,
    /// Whether it stays above the main window, and goes down with it when that's minimized.
    pub owned: bool,
    /// Whether the main window can't be used until this has gone.
    pub modal: bool,
    /// Whether it has no title bar or edges of its own. What's in it says where it can be dragged from, with `drags_window`.
    pub frameless: bool,
    /// Whether it stays above other programs' windows.
    pub on_top: bool,
    /// Whether it covers the whole screen.
    pub fullscreen: bool,
    /// Which of the system's layers it's in: with other windows, above them, beneath them, or on the desktop itself. `on_top`
    /// is the same as [`WindowLayer::Top`], for a window that says either.
    pub layer: WindowLayer,
    /// Whether only what's drawn in it shows, with what's behind the window everywhere else. See
    /// [`WindowChange::Transparent`].
    pub transparent: bool,
    /// Whether it's left off the taskbar and what Alt+Tab shows.
    pub skip_taskbar: bool,
    /// Whether it shows on every virtual desktop. See [`WindowChange::AllDesktops`].
    pub all_desktops: bool,
    /// A strip along an edge of a screen that it keeps for itself, as the taskbar does, or none.
    pub app_bar: Option<AppBar>,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            title: String::new(),
            size: None,
            smallest: None,
            owned: true,
            modal: false,
            frameless: false,
            on_top: false,
            fullscreen: false,
            layer: WindowLayer::Normal,
            transparent: false,
            skip_taskbar: false,
            all_desktops: false,
            app_bar: None,
        }
    }
}

impl WindowOptions {
    /// The layer it's in, with `on_top` counting as [`WindowLayer::Top`] when `layer` doesn't say otherwise.
    pub fn effective_layer(&self) -> WindowLayer {
        match self.layer {
            WindowLayer::Normal if self.on_top => WindowLayer::Top,
            layer => layer,
        }
    }

    /// What's done to the window once it's made and before it's shown, in order, for what these say that isn't how a window
    /// is anyway. Its frame comes off before any of this, as what's inside is worked out from that.
    pub fn startup_changes(&self) -> Vec<WindowChange> {
        let mut changes = Vec::new();
        if self.transparent {
            changes.push(WindowChange::Transparent(true));
        }
        if self.skip_taskbar {
            changes.push(WindowChange::SkipTaskbar(true));
        }
        if self.all_desktops {
            changes.push(WindowChange::AllDesktops(true));
        }
        let layer = self.effective_layer();
        if layer != WindowLayer::Normal {
            changes.push(WindowChange::Layer(layer));
        }
        if let Some(bar) = self.app_bar {
            changes.push(WindowChange::AppBar(Some(bar)));
        }
        if self.fullscreen {
            changes.push(WindowChange::Fullscreen(true));
        }
        changes
    }
}

/// Which of the system's layers a window is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WindowLayer {
    /// With other windows, coming to the front when it's used.
    #[default]
    Normal,
    /// Above other programs' windows, as `always on top` is.
    Top,
    /// Beneath every other window, staying there when it's clicked, as a widget on the desktop is. Clicking it doesn't take the
    /// keyboard from what has it, unless what's clicked takes keys. Show Desktop (Win+D) hides it along with the rest.
    Bottom,
    /// On the desktop itself, behind its icons and over its wallpaper, where Show Desktop leaves it. The icons are over it, so
    /// the pointer and keys don't reach it: it's for what's only looked at, as a wallpaper or a clock is.
    Desktop,
}

impl WindowLayer {
    /// From its name: `normal`, `top`, `bottom` or `desktop`.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "normal" => Some(Self::Normal),
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            "desktop" => Some(Self::Desktop),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Desktop => "desktop",
        }
    }

    /// What it is after [`WindowChange::OnTop`]: taking that away only moves a window that's on top.
    pub fn after_on_top(self, on: bool) -> Self {
        match (on, self) {
            (true, _) => Self::Top,
            (false, Self::Top) => Self::Normal,
            (false, other) => other,
        }
    }
}

/// An edge of a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenEdge {
    Top,
    Bottom,
    Left,
    Right,
}

impl ScreenEdge {
    /// From its name: `top`, `bottom`, `left` or `right`.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// A strip along an edge of a screen that a window keeps for itself, as the taskbar does: other windows are maximized and
/// snapped beside it rather than over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AppBar {
    pub edge: ScreenEdge,
    /// How thick it is, across from that edge, in logical pixels.
    pub size: u32,
    /// Which screen, counting from 0 for the main one and then in the order the system has them, or the main one.
    pub monitor: Option<u32>,
}

impl AppBar {
    /// From what a script says: an edge's name, a size, and which screen, if any. A size that isn't a number, or is less than 1,
    /// is 1, and an edge that isn't one is no app bar at all.
    pub fn parse(edge: &str, size: Option<f64>, monitor: Option<f64>) -> Option<Self> {
        let edge = ScreenEdge::parse(edge)?;
        let size = size.filter(|size| size.is_finite()).unwrap_or(1.0).clamp(1.0, 100_000.0).round() as u32;
        let monitor = monitor.filter(|at| at.is_finite() && *at >= 0.0).map(|at| at as u32);
        Some(Self { edge, size, monitor })
    }

    /// The rectangle it asks for on a screen (left, top, right, bottom, in physical pixels), `size` physical pixels thick.
    pub fn wanted(edge: ScreenEdge, screen: [i32; 4], size: i32) -> [i32; 4] {
        let [left, top, right, bottom] = screen;
        match edge {
            ScreenEdge::Top => [left, top, right, (top + size).min(bottom)],
            ScreenEdge::Bottom => [left, (bottom - size).max(top), right, bottom],
            ScreenEdge::Left => [left, top, (left + size).min(right), bottom],
            ScreenEdge::Right => [(right - size).max(left), top, right, bottom],
        }
    }

    /// What it takes once the system has moved what it asked for clear of what's already along that edge, as the taskbar is:
    /// `size` thick again from the outer edge the system settled on.
    pub fn settled(edge: ScreenEdge, proposed: [i32; 4], size: i32) -> [i32; 4] {
        let [left, top, right, bottom] = proposed;
        match edge {
            ScreenEdge::Top => [left, top, right, top + size],
            ScreenEdge::Bottom => [left, bottom - size, right, bottom],
            ScreenEdge::Left => [left, top, left + size, bottom],
            ScreenEdge::Right => [right - size, top, right, bottom],
        }
    }
}

/// Something to do to a window that's open.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowChange {
    Frameless(bool),
    /// Above other programs' windows, or back with them. This is [`Layer`](Self::Layer) with [`WindowLayer::Top`], and taking
    /// it away only does something to a window that's on top, leaving one on the desktop or beneath the rest where it is.
    OnTop(bool),
    /// Into one of the system's layers. See [`WindowLayer`].
    Layer(WindowLayer),
    /// Whether only what's drawn in it shows, at the opacity it's drawn with, with what's behind the window everywhere else:
    /// leave the root's background out, or make it transparent. Where nothing's drawn, the pointer goes through to what's
    /// behind, as it does for a layered window. A window that opens like this is made without a surface of the system's own
    /// (`WS_EX_NOREDIRECTIONBITMAP`), and keeps its GPU while it's on screen, as it has nothing else to show then; one that's
    /// changed later has its frame go all the way in, as one with a backdrop does.
    Transparent(bool),
    /// Keeps a strip along an edge of a screen for itself, as the taskbar does, and moves there, or gives it back with none.
    AppBar(Option<AppBar>),
    /// Whether it shows on every virtual desktop. Windows only has a stable way to have that for a tool window, which is left
    /// off the taskbar and Alt+Tab, so this makes it one, as [`SkipTaskbar`](Self::SkipTaskbar) does.
    AllDesktops(bool),
    Fullscreen(bool),
    Minimize,
    Maximize,
    /// Back from being minimized or maximized.
    Restore,
    /// Off the screen and the taskbar, while it carries on running, as for something that lives in the tray.
    Hide,
    Show,
    /// To the front. The system only allows that if this program is what's being used, and otherwise it flashes in the
    /// taskbar instead, which is as it should be, as someone could be typing or playing something.
    Focus,
    /// To here on the screen, in logical pixels.
    MoveTo(i32, i32),
    /// To this much room inside it, in logical pixels.
    Resize(u32, u32),
    /// To the middle of the screen it's on.
    Center,
    /// How much of it shows, from 0, which is nothing, to 1, all of it.
    Opacity(f32),
    /// Whether its edges can be dragged to make it bigger or smaller, and it can be maximized.
    Resizable(bool),
    /// How small and how big it can be made inside, in logical pixels, each if that's limited, with 0 for a side that
    /// isn't.
    SizeLimits(Option<(u32, u32)>, Option<(u32, u32)>),
    /// Whether it's left off the taskbar and what Alt+Tab shows.
    SkipTaskbar(bool),
    /// Whether the pointer goes straight through it, to what's behind, as for something laid over the screen.
    ClickThrough(bool),
    /// Whether it's left out of screenshots and recordings, and shows as black in them.
    ContentProtection(bool),
    /// Shown, without being given the keyboard or coming to the front of the program that's being used.
    ShowWithoutActivation,
    /// Starts making it bigger or smaller from this edge, as if that were dragged: `left`, `right`, `top`, `bottom`,
    /// `top-left`, `top-right`, `bottom-left` or `bottom-right`. Only while a button's held.
    StartResize(String),
    /// Its own icon, in its title bar and on the taskbar: an `.ico` file's bytes or a picture's.
    Icon(Vec<u8>),
    /// How far along something is, on its button on the taskbar, from 0 to 1, as `normal`, `paused`, `error` or
    /// `indeterminate`, or nothing with `none`.
    Progress(f64, String),
    /// A small picture over its button on the taskbar, as a count or a status, with words for screen readers, or none.
    Overlay(Option<Vec<u8>>, String),
    /// Its button on the taskbar flashes to say it wants the user: `critical` until they come to it, `informational` a
    /// few times, or `stop`.
    Flash(String),
    /// The buttons under its picture in the taskbar, which shows when the pointer's over its button: up to seven, in
    /// order, or none. A click is told to [`on_thumbnail_button`](crate::on_thumbnail_button).
    ThumbnailButtons(Vec<ThumbnailButton>),
    /// What the system draws behind it, through what isn't drawn over: `mica`, `acrylic`, `tabbed` or `none`. Only
    /// Windows 11 has these.
    Backdrop(String),
    /// A picture of what's in it, as [`capture_window`](crate::capture_window) takes, which is told to
    /// [`on_window_captured`](crate::on_window_captured) with this number. It's taken once nothing's drawing the window,
    /// as a program's own code is run while something is, and can't take one then.
    Capture(u32),
}

/// A button under a window's picture in the taskbar.
#[derive(Clone, PartialEq, Default)]
pub struct ThumbnailButton {
    /// What it's told apart by when it's clicked.
    pub id: u32,
    /// An `.ico` file's bytes or a picture's.
    pub icon: Vec<u8>,
    pub tooltip: String,
    pub disabled: bool,
}

impl std::fmt::Debug for ThumbnailButton {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The icon's bytes would be most of it, and say nothing.
        write!(out, "{:?} ({} bytes of icon{})", self.tooltip, self.icon.len(), if self.disabled { ", disabled" } else { "" })
    }
}

/// How a window is now, in logical pixels.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WindowStateNow {
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub visible: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub focused: bool,
}

/// What makes the [`App`] for a window, given its text, its size in physical pixels, and its scale.
pub type MakeApp = Box<dyn FnOnce(TextEngine, (u32, u32), f32) -> App>;

/// A window that's been asked for.
pub struct WantedWindow {
    pub id: WindowId,
    pub options: WindowOptions,
    pub make: MakeApp,
}

/// What's been asked of the windows, in the order it should be done in.
#[derive(Default)]
pub struct WindowRequests {
    pub open: Vec<WantedWindow>,
    pub titles: Vec<(WindowId, String)>,
    pub close: Vec<WindowId>,
    pub changes: Vec<(WindowId, WindowChange)>,
}

impl WindowRequests {
    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
            && self.titles.is_empty()
            && self.close.is_empty()
            && self.changes.is_empty()
    }
}

thread_local! {
    static REQUESTS: RefCell<WindowRequests> = RefCell::default();
    static NEXT_ID: Cell<u32> = const { Cell::new(1) };
    /// Called when something's been asked for, for whatever's running the windows.
    static TELL: Cell<Option<fn()>> = const { Cell::new(None) };
}

fn tell() {
    if let Some(tell) = TELL.with(Cell::get) {
        tell();
    }
}

pub(crate) fn tell_with(function: fn()) {
    TELL.with(|tell| tell.set(Some(function)));
}

/// Asks for a window showing `render` as its root component. It has a state of its own, as any [`App`] does. What goes
/// between windows goes through what they share, or a [`Remote`](rsui_runtime::Remote).
pub fn open_window<P: 'static, R: IntoNode + 'static>(
    options: WindowOptions,
    render: fn(&mut Cx<'_>, &P) -> R,
    props: P,
) -> WindowId {
    let id = WindowId(NEXT_ID.with(|next| next.replace(next.get() + 1)));
    let make: MakeApp =
        Box::new(move |text, size, scale| App::new(render, props, text, size, scale));
    REQUESTS.with(|requests| {
        requests
            .borrow_mut()
            .open
            .push(WantedWindow { id, options, make });
    });
    tell();
    id
}

/// Asks for a window to close. Closing the main one ends the program, along with the rest. Unlike clicking its close
/// button, this doesn't ask what the window thinks about it.
pub fn close_window(id: WindowId) {
    REQUESTS.with(|requests| requests.borrow_mut().close.push(id));
    tell();
}

/// Asks for a window to be called something else.
pub fn set_window_title(id: WindowId, title: &str) {
    REQUESTS.with(|requests| requests.borrow_mut().titles.push((id, title.to_owned())));
    tell();
}

/// Asks for something to be done to a window that's open, or that's just been asked for.
pub fn change_window(id: WindowId, change: WindowChange) {
    if id == WindowId::MAIN && crate::window::change_main_kind(&change) {
        return;
    }
    REQUESTS.with(|requests| requests.borrow_mut().changes.push((id, change)));
    tell();
}

/// Everything that's been asked of the windows so far, for doing something about it, which [`run`](crate::run) does
/// by itself.
pub fn take_window_requests() -> WindowRequests {
    REQUESTS.with(RefCell::take)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_and_edges_go_by_their_names() {
        for layer in [WindowLayer::Normal, WindowLayer::Top, WindowLayer::Bottom, WindowLayer::Desktop] {
            assert_eq!(WindowLayer::parse(layer.name()), Some(layer));
        }
        assert_eq!(WindowLayer::parse("wallpaper"), None);
        assert_eq!(WindowLayer::parse("Top"), None, "names are as they're written");
        assert_eq!(WindowLayer::default(), WindowLayer::Normal);
        for edge in [ScreenEdge::Top, ScreenEdge::Bottom, ScreenEdge::Left, ScreenEdge::Right] {
            assert_eq!(ScreenEdge::parse(edge.name()), Some(edge));
        }
        assert_eq!(ScreenEdge::parse("middle"), None);
    }

    #[test]
    fn on_top_is_the_top_layer_and_taking_it_away_only_moves_a_window_that_is_on_top() {
        assert_eq!(WindowLayer::Normal.after_on_top(true), WindowLayer::Top);
        assert_eq!(WindowLayer::Desktop.after_on_top(true), WindowLayer::Top);
        assert_eq!(WindowLayer::Top.after_on_top(false), WindowLayer::Normal);
        assert_eq!(WindowLayer::Desktop.after_on_top(false), WindowLayer::Desktop);
        assert_eq!(WindowLayer::Bottom.after_on_top(false), WindowLayer::Bottom);
        assert_eq!(WindowLayer::Normal.after_on_top(false), WindowLayer::Normal);

        let on_top = WindowOptions { on_top: true, ..WindowOptions::default() };
        assert_eq!(on_top.effective_layer(), WindowLayer::Top);
        let beneath = WindowOptions { on_top: true, layer: WindowLayer::Bottom, ..WindowOptions::default() };
        assert_eq!(beneath.effective_layer(), WindowLayer::Bottom, "a layer that's said wins");
        assert_eq!(WindowOptions::default().effective_layer(), WindowLayer::Normal);
    }

    #[test]
    fn a_plain_window_needs_nothing_done_before_it_shows() {
        assert_eq!(WindowOptions::default().startup_changes(), []);
        let framed = WindowOptions { frameless: true, title: "Only no frame".into(), ..WindowOptions::default() };
        assert_eq!(framed.startup_changes(), [], "the frame comes off before these");
    }

    #[test]
    fn options_combine_into_what_is_done_before_it_shows() {
        use WindowChange::*;
        let widget = WindowOptions {
            frameless: true,
            transparent: true,
            layer: WindowLayer::Desktop,
            skip_taskbar: true,
            all_desktops: true,
            ..WindowOptions::default()
        };
        assert_eq!(
            widget.startup_changes(),
            [Transparent(true), SkipTaskbar(true), AllDesktops(true), Layer(WindowLayer::Desktop)]
        );
        let bar = super::AppBar { edge: ScreenEdge::Top, size: 32, monitor: Some(1) };
        let docked = WindowOptions { on_top: true, app_bar: Some(bar), fullscreen: true, ..WindowOptions::default() };
        assert_eq!(docked.startup_changes(), [Layer(WindowLayer::Top), AppBar(Some(bar)), Fullscreen(true)]);
        let overlay = WindowOptions {
            transparent: true,
            layer: WindowLayer::Bottom,
            ..WindowOptions::default()
        };
        assert_eq!(overlay.startup_changes(), [Transparent(true), Layer(WindowLayer::Bottom)]);
    }

    #[test]
    fn an_app_bar_is_read_from_what_a_script_says() {
        assert_eq!(
            AppBar::parse("left", Some(48.4), None),
            Some(AppBar { edge: ScreenEdge::Left, size: 48, monitor: None })
        );
        assert_eq!(
            AppBar::parse("bottom", Some(30.0), Some(2.0)),
            Some(AppBar { edge: ScreenEdge::Bottom, size: 30, monitor: Some(2) })
        );
        assert_eq!(AppBar::parse("sideways", Some(30.0), None), None);
        assert_eq!(AppBar::parse("top", None, None).map(|bar| bar.size), Some(1));
        assert_eq!(AppBar::parse("top", Some(-5.0), None).map(|bar| bar.size), Some(1));
        assert_eq!(AppBar::parse("top", Some(f64::NAN), None).map(|bar| bar.size), Some(1));
        assert_eq!(AppBar::parse("top", Some(10.0), Some(-1.0)).map(|bar| bar.monitor), Some(None));
    }

    #[test]
    fn an_app_bar_asks_for_its_edge_and_keeps_its_size_where_the_system_puts_it() {
        let screen = [0, 0, 1920, 1080];
        assert_eq!(AppBar::wanted(ScreenEdge::Top, screen, 40), [0, 0, 1920, 40]);
        assert_eq!(AppBar::wanted(ScreenEdge::Bottom, screen, 40), [0, 1040, 1920, 1080]);
        assert_eq!(AppBar::wanted(ScreenEdge::Left, screen, 40), [0, 0, 40, 1080]);
        assert_eq!(AppBar::wanted(ScreenEdge::Right, screen, 40), [1880, 0, 1920, 1080]);
        assert_eq!(AppBar::wanted(ScreenEdge::Top, screen, 5000), [0, 0, 1920, 1080], "never past the screen");
        // A screen to the left of the main one.
        assert_eq!(AppBar::wanted(ScreenEdge::Right, [-1280, 0, 0, 1024], 30), [-30, 0, 0, 1024]);
        // The taskbar's at the bottom, so the system moves a bar there up above it, and the bar keeps its size from there.
        assert_eq!(AppBar::settled(ScreenEdge::Bottom, [0, 1000, 1920, 1032], 40), [0, 992, 1920, 1032]);
        assert_eq!(AppBar::settled(ScreenEdge::Top, [0, 48, 1920, 88], 40), [0, 48, 1920, 88]);
        assert_eq!(AppBar::settled(ScreenEdge::Left, [60, 0, 100, 1080], 40), [60, 0, 100, 1080]);
        assert_eq!(AppBar::settled(ScreenEdge::Right, [1820, 0, 1870, 1080], 40), [1830, 0, 1870, 1080]);
    }
}
