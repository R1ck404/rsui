//! Where a window sits in the shell's own arrangement of things: a layer beneath or above the rest, on the desktop behind its
//! icons, along an edge of a screen as the taskbar is, or on every virtual desktop. These are plain options of any window (see
//! [`WindowChange`](crate::WindowChange)), which a desktop widget, a live wallpaper, a bar or an overlay are made of.
//!
//! What's kept for each window is here rather than in the window's own state, as these messages can come while that's in use,
//! as when the system moves a window while it's drawing.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Mutex, Once};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint,
    ScreenToClient,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{
    ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_ACTIVATE, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS,
    ABM_WINDOWPOSCHANGED, ABN_FULLSCREENAPP, ABN_POSCHANGED, APPBARDATA, SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PCWSTR, w};

use crate::windows::{AppBar, ScreenEdge, WindowLayer};

/// What an app bar hears from the shell on: something else along its edge moved, or something went fullscreen.
pub(crate) const APP_BAR_MESSAGE: u32 = WM_APP + 20;
/// Sent to a window to have it go back where it should be in its layer, once what's going on has finished.
const REPLACE: u32 = WM_APP + 21;

/// What's been asked of a window here.
#[derive(Debug, Clone, Copy, Default)]
struct Shell {
    layer: WindowLayer,
    /// What it was inside of, and how it was, before it went on the desktop: its owner and its style.
    before_desktop: Option<(isize, isize)>,
    transparent: bool,
    click_through: bool,
    /// Whether the pointer's going through it for now, as it's over where nothing's drawn.
    passing_through: bool,
    skip_taskbar: bool,
    all_desktops: bool,
    app_bar: Option<AppBar>,
    app_bar_registered: bool,
    /// Whether something's fullscreen on its screen, which an app bar on top goes beneath for.
    fullscreen_app: bool,
}

thread_local! {
    static SHELL: RefCell<HashMap<isize, Shell>> = RefCell::new(HashMap::new());
    /// The window that hears when Explorer starts over, as only top-level windows do, and a window on the desktop isn't one.
    static WATCHER: Cell<Option<isize>> = const { Cell::new(None) };
}

/// The windows with app bars, for taking them away if the program fails, from whatever thread that's on.
static APP_BARS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

fn get(window: HWND) -> Shell {
    SHELL.with(|shell| shell.borrow().get(&(window.0 as isize)).copied().unwrap_or_default())
}

fn update(window: HWND, change: impl FnOnce(&mut Shell)) {
    SHELL.with(|shell| change(shell.borrow_mut().entry(window.0 as isize).or_default()));
}

pub(crate) fn layer_of(window: HWND) -> WindowLayer {
    get(window).layer
}

pub(crate) fn set_transparent(window: HWND, on: bool) {
    update(window, |shell| shell.transparent = on);
    if !on {
        stop_passing_through(window);
    }
}

pub(crate) fn set_click_through(window: HWND, on: bool) {
    update(window, |shell| {
        shell.click_through = on;
        // Clicking through for good is more than for now.
        shell.passing_through = false;
    });
}

/// Whether the pointer should go through this window where nothing's drawn, rather than it hearing it.
pub(crate) fn may_pass_through(window: HWND) -> bool {
    let shell = get(window);
    shell.transparent && !shell.click_through && shell.layer != WindowLayer::Desktop
}

pub(crate) fn is_passing_through(window: HWND) -> bool {
    get(window).passing_through
}

/// Has the pointer go through to what's behind, until it's over something that's drawn again. The system only does that for a
/// window that's layered and transparent to the pointer, and then sends it nothing at all, so `poll` has to look for where the
/// pointer is until it's back.
pub(crate) fn start_passing_through(window: HWND, poll: usize) {
    if is_passing_through(window) {
        return;
    }
    update(window, |shell| shell.passing_through = true);
    // SAFETY: the window's on this thread, and one that's gone only gives errors.
    unsafe {
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
        if style & WS_EX_LAYERED.0 as isize == 0 {
            SetWindowLongPtrW(window, GWL_EXSTYLE, style | WS_EX_LAYERED.0 as isize);
            // A layered window shows nothing until it's said how opaque it is.
            let _ = SetLayeredWindowAttributes(window, windows::Win32::Foundation::COLORREF(0), 255, LWA_ALPHA);
        }
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
        SetWindowLongPtrW(window, GWL_EXSTYLE, style | WS_EX_TRANSPARENT.0 as isize);
        SetTimer(Some(window), poll, 50, None);
    }
}

/// The pointer stops going through, as it's over something that's drawn, or it isn't over the window at all.
pub(crate) fn stop_passing_through(window: HWND) {
    let shell = get(window);
    if !shell.passing_through {
        return;
    }
    update(window, |shell| shell.passing_through = false);
    if !shell.click_through {
        // SAFETY: as above.
        unsafe {
            let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
            SetWindowLongPtrW(window, GWL_EXSTYLE, style & !(WS_EX_TRANSPARENT.0 as isize));
        }
    }
}

/// Whether it's left off the taskbar, for itself or so as to be on every virtual desktop, which are both a tool window.
pub(crate) fn set_tool_window(window: HWND, skip_taskbar: Option<bool>, all_desktops: Option<bool>) {
    update(window, |shell| {
        if let Some(on) = skip_taskbar {
            shell.skip_taskbar = on;
        }
        if let Some(on) = all_desktops {
            shell.all_desktops = on;
        }
    });
    let shell = get(window);
    let tool = shell.skip_taskbar || shell.all_desktops;
    // SAFETY: as above. The taskbar only looks at these as a window's shown, so it's hidden around the change.
    unsafe {
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
        let wanted = if tool {
            (style | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize)
        } else {
            (style & !(WS_EX_TOOLWINDOW.0 as isize)) | WS_EX_APPWINDOW.0 as isize
        };
        if wanted == style {
            return;
        }
        let shown = IsWindowVisible(window).as_bool();
        if shown {
            let _ = ShowWindow(window, SW_HIDE);
        }
        SetWindowLongPtrW(window, GWL_EXSTYLE, wanted);
        if shown {
            let _ = ShowWindow(window, SW_SHOWNA);
        }
    }
}

/// Puts a window in a layer, taking it out of the one it was in.
pub(crate) fn set_layer(window: HWND, layer: WindowLayer) {
    let was = get(window).layer;
    if was == WindowLayer::Desktop && layer != WindowLayer::Desktop {
        leave_desktop(window);
    }
    update(window, |shell| shell.layer = layer);
    // SAFETY: as above.
    unsafe {
        match layer {
            WindowLayer::Normal => {
                let _ = SetWindowPos(window, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                if was == WindowLayer::Bottom || was == WindowLayer::Desktop {
                    let _ = SetWindowPos(window, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
            }
            WindowLayer::Top => {
                let _ = SetWindowPos(window, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
            WindowLayer::Bottom => {
                let _ = SetWindowPos(window, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                let _ = SetWindowPos(window, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
            WindowLayer::Desktop => {
                if was != WindowLayer::Desktop {
                    let _ = SetWindowPos(window, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                    go_on_desktop(window);
                }
                watch_explorer();
            }
        }
    }
}

/// The window the desktop's wallpaper is drawn in, behind the icons, for a window to go inside, made if it isn't there yet.
///
/// Explorer makes it when Progman is sent 0x052C, as it does to animate a change of wallpaper. Up to Windows 11 23H2 it's a
/// `WorkerW` of its own, the one after the `WorkerW` (or Progman) that holds the icons' `SHELLDLL_DefView`. From 24H2 on it's a
/// child of Progman, under the icons. If neither turns up, it's Progman itself, where the icons may cover what's drawn.
fn desktop_host() -> Option<HWND> {
    // SAFETY: these only ask about windows, and send a message Explorer expects, giving up after a second if it's stuck.
    unsafe {
        let progman = FindWindowW(w!("Progman"), PCWSTR::null()).ok()?;
        let mut answer = 0usize;
        // Newer builds want these numbers, older ones none, and either is harmless to the other.
        let _ = SendMessageTimeoutW(progman, 0x052C, WPARAM(0xD), LPARAM(1), SMTO_NORMAL, 1000, Some(&raw mut answer));
        let _ = SendMessageTimeoutW(progman, 0x052C, WPARAM(0), LPARAM(0), SMTO_NORMAL, 1000, Some(&raw mut answer));
        if let Ok(worker) = FindWindowExW(Some(progman), None, w!("WorkerW"), PCWSTR::null()) {
            return Some(worker);
        }
        let mut found = HWND::default();
        let _ = EnumWindows(Some(find_worker), LPARAM((&raw mut found) as isize));
        if !found.is_invalid() {
            return Some(found);
        }
        Some(progman)
    }
}

/// Looks for the top-level window that holds the desktop's icons, and the `WorkerW` after it.
unsafe extern "system" fn find_worker(top: HWND, found: LPARAM) -> BOOL {
    // SAFETY: `found` is the `HWND` `desktop_host` gave, which lasts through `EnumWindows`.
    unsafe {
        if FindWindowExW(Some(top), None, w!("SHELLDLL_DefView"), PCWSTR::null()).is_ok() {
            if let Ok(worker) = FindWindowExW(None, Some(top), w!("WorkerW"), PCWSTR::null()) {
                *(found.0 as *mut HWND) = worker;
                return BOOL(0);
            }
        }
    }
    BOOL(1)
}

/// Puts a window inside the desktop's, where it stays where it was on the screen.
fn go_on_desktop(window: HWND) {
    let Some(host) = desktop_host() else {
        return;
    };
    // SAFETY: as above, and what these write to is valid.
    unsafe {
        let mut rect = RECT::default();
        let _ = GetWindowRect(window, &raw mut rect);
        let before = get(window).before_desktop;
        if before.is_none() {
            let owner = GetWindowLongPtrW(window, GWLP_HWNDPARENT);
            let style = GetWindowLongPtrW(window, GWL_STYLE);
            update(window, |shell| shell.before_desktop = Some((owner, style)));
            // A window inside another is a child, which the system expects of one before it's put in another.
            SetWindowLongPtrW(window, GWL_STYLE, (style & !(WS_POPUP.0 as isize)) | WS_CHILD.0 as isize);
        }
        if SetParent(window, Some(host)).is_err() {
            if let Some((_, style)) = get(window).before_desktop {
                SetWindowLongPtrW(window, GWL_STYLE, style);
            }
            update(window, |shell| shell.before_desktop = None);
            return;
        }
        let mut corner = POINT { x: rect.left, y: rect.top };
        let _ = ScreenToClient(host, &raw mut corner);
        let _ = SetWindowPos(
            window,
            Some(HWND_TOP),
            corner.x,
            corner.y,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// Takes a window off the desktop, back to being one of its own, where it was on the screen.
fn leave_desktop(window: HWND) {
    let Some((owner, style)) = get(window).before_desktop else {
        return;
    };
    update(window, |shell| shell.before_desktop = None);
    // SAFETY: as above.
    unsafe {
        let mut rect = RECT::default();
        let _ = GetWindowRect(window, &raw mut rect);
        SetWindowLongPtrW(window, GWL_STYLE, style);
        let _ = SetParent(window, None);
        if owner != 0 {
            SetWindowLongPtrW(window, GWLP_HWNDPARENT, owner);
        }
        let _ = SetWindowPos(
            window,
            Some(HWND_TOP),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// Where a point on the screen is in what the window's inside, for placing it: the same, unless it's on the desktop.
pub(crate) fn from_screen(window: HWND, x: i32, y: i32) -> (i32, i32) {
    if get(window).before_desktop.is_none() {
        return (x, y);
    }
    let mut point = POINT { x, y };
    // SAFETY: as above.
    unsafe {
        if let Ok(parent) = GetParent(window) {
            let _ = ScreenToClient(parent, &raw mut point);
        }
    }
    (point.x, point.y)
}

fn edge_number(edge: ScreenEdge) -> u32 {
    match edge {
        ScreenEdge::Top => ABE_TOP,
        ScreenEdge::Bottom => ABE_BOTTOM,
        ScreenEdge::Left => ABE_LEFT,
        ScreenEdge::Right => ABE_RIGHT,
    }
}

fn bar_data(window: HWND) -> APPBARDATA {
    APPBARDATA {
        cbSize: size_of::<APPBARDATA>() as u32,
        hWnd: window,
        uCallbackMessage: APP_BAR_MESSAGE,
        ..Default::default()
    }
}

/// The screens, the main one first and then the rest in the order the system has them.
fn monitors() -> Vec<HMONITOR> {
    unsafe extern "system" fn add(monitor: HMONITOR, _: HDC, _: *mut RECT, list: LPARAM) -> BOOL {
        // SAFETY: `list` is the vector `monitors` gave, which lasts through `EnumDisplayMonitors`.
        unsafe { (*(list.0 as *mut Vec<HMONITOR>)).push(monitor) };
        BOOL(1)
    }
    let mut list: Vec<HMONITOR> = Vec::new();
    // SAFETY: `add` only pushes onto `list`, which outlives the call.
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(add), LPARAM((&raw mut list) as isize));
        let primary = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
        list.retain(|monitor| *monitor != primary);
        list.insert(0, primary);
    }
    list
}

/// Keeps a strip along an edge of a screen for the window, or gives it back.
pub(crate) fn set_app_bar(window: HWND, bar: Option<AppBar>) {
    let shell = get(window);
    update(window, |shell| shell.app_bar = bar);
    match bar {
        None => {
            if shell.app_bar_registered {
                remove_app_bar(window);
            }
        }
        Some(bar) => {
            if !shell.app_bar_registered {
                let mut data = bar_data(window);
                // SAFETY: `data` is the size it says.
                let registered = unsafe { SHAppBarMessage(ABM_NEW, &raw mut data) } != 0;
                if !registered {
                    return;
                }
                update(window, |shell| shell.app_bar_registered = true);
                APP_BARS.lock().map(|mut bars| bars.push(window.0 as isize)).ok();
                remove_app_bars_on_panic();
                watch_explorer();
            }
            place_app_bar(window, bar);
        }
    }
}

fn remove_app_bar(window: HWND) {
    update(window, |shell| {
        shell.app_bar_registered = false;
        shell.fullscreen_app = false;
    });
    let mut data = bar_data(window);
    // SAFETY: `data` is the size it says, and a window that isn't one does nothing.
    unsafe {
        SHAppBarMessage(ABM_REMOVE, &raw mut data);
    }
    if let Ok(mut bars) = APP_BARS.lock() {
        bars.retain(|bar| *bar != window.0 as isize);
    }
}

/// Asks the shell where the bar can go, keeps that, and moves there.
fn place_app_bar(window: HWND, bar: AppBar) {
    let screens = monitors();
    let monitor = bar.monitor.and_then(|at| screens.get(at as usize).copied()).unwrap_or(screens[0]);
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let (mut dpi, mut dpi_y) = (96u32, 96u32);
    // SAFETY: what these write to is valid, and a screen that's gone only gives an error.
    unsafe {
        if !GetMonitorInfoW(monitor, &raw mut info).as_bool() {
            return;
        }
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &raw mut dpi, &raw mut dpi_y);
    }
    let size = (bar.size as f32 * dpi as f32 / 96.0).round() as i32;
    let screen = info.rcMonitor;
    let wanted = AppBar::wanted(bar.edge, [screen.left, screen.top, screen.right, screen.bottom], size);
    let mut data = bar_data(window);
    data.uEdge = edge_number(bar.edge);
    data.rc = RECT { left: wanted[0], top: wanted[1], right: wanted[2], bottom: wanted[3] };
    // SAFETY: `data` is the size it says.
    unsafe {
        SHAppBarMessage(ABM_QUERYPOS, &raw mut data);
    }
    let settled = AppBar::settled(bar.edge, [data.rc.left, data.rc.top, data.rc.right, data.rc.bottom], size);
    data.rc = RECT { left: settled[0], top: settled[1], right: settled[2], bottom: settled[3] };
    // SAFETY: as above.
    unsafe {
        SHAppBarMessage(ABM_SETPOS, &raw mut data);
        let (x, y) = from_screen(window, data.rc.left, data.rc.top);
        let _ = SetWindowPos(
            window,
            None,
            x,
            y,
            data.rc.right - data.rc.left,
            data.rc.bottom - data.rc.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Takes away every app bar if the program panics, so that the strips they kept don't stay empty. Explorer also gives one
/// back once its window's gone, the next time it works out what's left of the screen, which covers being ended outright.
fn remove_app_bars_on_panic() {
    static HOOKED: Once = Once::new();
    HOOKED.call_once(|| {
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            remove_every_app_bar();
            before(info);
        }));
    });
}

/// Gives back what every app bar has kept. This is safe from any thread.
pub(crate) fn remove_every_app_bar() {
    let bars = APP_BARS.lock().map(|mut bars| std::mem::take(&mut *bars)).unwrap_or_default();
    for bar in bars {
        let mut data = bar_data(HWND(bar as *mut _));
        // SAFETY: `data` is the size it says, and a window that's gone does nothing.
        unsafe {
            SHAppBarMessage(ABM_REMOVE, &raw mut data);
        }
    }
}

fn taskbar_created() -> u32 {
    static NUMBER: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    // SAFETY: this only registers a name.
    *NUMBER.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

/// Makes the hidden window that hears when Explorer starts over, once anything here needs that.
fn watch_explorer() {
    if WATCHER.with(Cell::get).is_some() {
        return;
    }
    static CLASS: Once = Once::new();
    let class = w!("rsui-shell-watcher");
    // SAFETY: this makes a hidden window on this thread with a class of its own, whose procedure is below.
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        CLASS.call_once(|| {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(watcher_procedure),
                hInstance: module.into(),
                lpszClassName: class,
                ..Default::default()
            });
        });
        if let Ok(watcher) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class,
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(module.into()),
            None,
        ) {
            WATCHER.with(|cell| cell.set(Some(watcher.0 as isize)));
        }
    }
}

unsafe extern "system" fn watcher_procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == taskbar_created() {
        explorer_started_over();
        return LRESULT(0);
    }
    if message == WM_DESTROY {
        WATCHER.with(|cell| cell.set(None));
    }
    // SAFETY: these are the arguments this was given.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// Explorer has started over, with a new desktop and taskbar, which know nothing of what was asked of the old ones: windows on
/// the desktop go back on it, and app bars keep their strips again.
fn explorer_started_over() {
    let windows: Vec<(isize, Shell)> = SHELL.with(|shell| shell.borrow().iter().map(|(at, shell)| (*at, *shell)).collect());
    for (at, shell) in windows {
        let window = HWND(at as *mut _);
        // SAFETY: this only asks.
        if !unsafe { IsWindow(Some(window)) }.as_bool() {
            SHELL.with(|all| all.borrow_mut().remove(&at));
            continue;
        }
        if shell.layer == WindowLayer::Desktop {
            go_on_desktop(window);
        }
        if let Some(bar) = shell.app_bar {
            update(window, |shell| shell.app_bar_registered = false);
            if let Ok(mut bars) = APP_BARS.lock() {
                bars.retain(|other| *other != at);
            }
            set_app_bar(window, Some(bar));
        }
    }
}

/// Handles what's for this module before the window does, and says what the result is if that's all there is to it.
pub(crate) fn before_message(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match message {
        WM_WINDOWPOSCHANGING => {
            let shell = get(window);
            // SAFETY: for this message, `lparam` points to where it's going, which can be changed.
            let place = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
            if place.flags.0 & SWP_NOZORDER.0 == 0 {
                let beneath = shell.layer == WindowLayer::Bottom
                    || (shell.fullscreen_app && shell.layer == WindowLayer::Top && shell.app_bar.is_some());
                if beneath {
                    // Whatever brings it forward, as being clicked does, it stays beneath the rest.
                    place.hwndInsertAfter = HWND_BOTTOM;
                }
            }
            None
        }
        WM_WINDOWPOSCHANGED | WM_ACTIVATE => {
            if get(window).app_bar_registered {
                let mut data = bar_data(window);
                let tell = if message == WM_ACTIVATE { ABM_ACTIVATE } else { ABM_WINDOWPOSCHANGED };
                // SAFETY: `data` is the size it says.
                unsafe {
                    SHAppBarMessage(tell, &raw mut data);
                }
            }
            None
        }
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            if get(window).app_bar.is_some() {
                // After the window's handled it, as it moves itself for a new DPI.
                // SAFETY: posting is safe.
                unsafe {
                    let _ = PostMessageW(Some(window), REPLACE, WPARAM(0), LPARAM(0));
                }
            }
            None
        }
        REPLACE => {
            if let Some(bar) = get(window).app_bar {
                place_app_bar(window, bar);
            }
            Some(LRESULT(0))
        }
        APP_BAR_MESSAGE => {
            let shell = get(window);
            match wparam.0 as u32 {
                ABN_POSCHANGED => {
                    if let Some(bar) = shell.app_bar {
                        place_app_bar(window, bar);
                    }
                }
                ABN_FULLSCREENAPP => {
                    // Something covering the whole screen, as a game or a video does, goes over a bar that's on top,
                    // which comes back once it's gone.
                    let opening = lparam.0 != 0;
                    update(window, |shell| shell.fullscreen_app = opening);
                    if shell.layer == WindowLayer::Top {
                        // SAFETY: as above.
                        unsafe {
                            if opening {
                                let _ = SetWindowPos(window, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                                let _ = SetWindowPos(window, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                            } else {
                                let _ = SetWindowPos(window, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                            }
                        }
                    }
                }
                _ => {}
            }
            Some(LRESULT(0))
        }
        WM_DESTROY => {
            let shell = get(window);
            if shell.app_bar_registered {
                remove_app_bar(window);
            }
            SHELL.with(|all| all.borrow_mut().remove(&(window.0 as isize)));
            None
        }
        _ => None,
    }
}
