//! A real window, and the loop that runs it.
//!
//! The loop sleeps until something happens, so an idle app uses no CPU at all. After a
//! moment of nothing happening, it gives back the memory it isn't using.

#[cfg(feature = "idle-probe")]
#[path = "idle_probe.rs"]
mod idle_probe;

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rsui_core::IntoNode;
use rsui_render::{RenderError, Renderer, WorkingSetLimit, give_back_memory};
use rsui_runtime::Cx;

use crate::replay::Input;
use windows::Win32::Foundation::{
    COLORREF, GlobalFree, HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLACK_BRUSH, BeginPaint, CreateSolidBrush, DIB_RGB_COLORS,
    EndPaint, FillRect, GdiFlush, GetDC, GetMonitorInfoW, GetStockObject, GetUpdateRect, HBRUSH, HDC,
    InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, PAINTSTRUCT,
    ReleaseDC, ScreenToClient, SetDIBitsToDevice, UpdateWindow, ValidateRect,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::Accessibility::{IRawElementProviderSimple, UiaRootObjectId};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, GetDpiForWindow,
    GetSystemMetricsForDpi, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetFocus, SetFocus};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardState, MAPVK_VK_TO_CHAR, MapVirtualKeyW, ReleaseCapture, SetCapture,
    TME_LEAVE, TRACKMOUSEEVENT, ToUnicode, TrackMouseEvent, VIRTUAL_KEY, VK_ADD, VK_APPS, VK_BACK,
    VK_CAPITAL, VK_CONTROL, VK_DECIMAL, VK_DELETE, VK_DIVIDE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1,
    VK_F24, VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
    VK_MULTIPLY, VK_NEXT, VK_NUMLOCK, VK_NUMPAD0, VK_NUMPAD9, VK_PAUSE, VK_PRIOR, VK_PROCESSKEY,
    VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SCROLL, VK_SHIFT,
    VK_SNAPSHOT, VK_SPACE, VK_SUBTRACT, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HSTRING, PCWSTR, s, w};

use crate::app::{
    AccessFingerprint, App, BACKGROUND, Frame, RunError, clear_color, default_text_engine,
};
use crate::input::{Clipboard, Cursor, KeyCode, KeyPress, Modifiers, PointerButton};
use crate::windows::{MakeApp, WindowChange, WindowId, WindowLayer, WindowOptions};
use crate::{access, uia};

/// Sent by other threads when they've changed state.
const WAKE: u32 = WM_APP + 1;
/// Sent by the thread making a renderer when it's finished.
const RENDERER_READY: u32 = WM_APP + 2;
/// Sent by anything that's asked for something to be done with the windows, for once that can safely happen.
const WINDOWS_WANTED: u32 = WM_APP + 16;
/// A timer that fires once things have been quiet, to give memory back.
const IDLE_TIMER: usize = 1;
/// How long things must be quiet for.
const QUIET: Duration = Duration::from_secs(1);
/// How long they must be quiet for after a first frame. Starting up leaves far more memory in
/// use than anything else does, and nothing is likely to be using it yet.
const QUIET_AFTER_FIRST_FRAME: Duration = Duration::from_millis(100);
/// The most things have to be quiet before giving memory back, for a window in regular use. Giving it
/// back blocks for about a frame's time, and the next frame takes hundreds of page faults to get it
/// again, which is no use if it's about to be used.
const QUIET_LONGEST: Duration = Duration::from_secs(8);
/// Being used again this soon after giving memory back means waiting longer next time.
const USED_AGAIN_SOON: Duration = Duration::from_secs(5);
/// A timer that blinks the caret.
const CARET_TIMER: usize = 2;
/// A timer that lets go of the GPU once there's been no need for it for a while.
const RELEASE_TIMER: usize = 3;
/// How long a window stays minimized before letting go of the GPU. Getting it back takes about a
/// tenth of a second, so this leaves a quick look at something else unaffected, while the
/// threads and much of the memory the driver uses come back for as long as it stays minimized.
const RELEASE_MILLISECONDS: u32 = 10_000;
/// How long a window on screen stays quiet before doing the same, keeping its picture on screen
/// without the GPU. The driver's threads wake up even with nothing to draw, and most windows are
/// idle most of the time. Getting the GPU back starts as soon as the pointer moves in, so it's
/// usually ready before the first click.
const DEEP_IDLE: Duration = Duration::from_secs(30);
/// How long that wait is at first when nobody seems to be using the window: the pointer is
/// elsewhere, and nothing is being typed in it. Coming back to it gets the GPU back on the way in,
/// so nothing's waiting by the time they click. This is long enough that a quick trip to another
/// window doesn't pay for getting it back, which costs far more than these seconds of waiting do.
const DEEP_IDLE_UNATTENDED: Duration = Duration::from_secs(10);
/// Getting the GPU back takes about 450 million cycles, and staying idle with it costs about a
/// million a second, so letting go of it only saves CPU after this long. If someone comes back
/// sooner, they're still using the window on and off, so the wait doubles, and a window in regular
/// use soon stops paying for this at all. It still saves GPU memory and threads from the start.
const DEEP_IDLE_WORTHWHILE: Duration = Duration::from_secs(420);
/// The longest that wait gets.
const DEEP_IDLE_LONGEST: Duration = Duration::from_secs(300);
/// A timer for one early trim, even if things haven't gone quiet. Starting up leaves a lot of
/// memory in use that nothing will touch again, and an app that's always busy would keep it.
const STARTUP_TRIM_TIMER: usize = 4;
/// A timer for when the app has something it wants to do later.
const WAKE_TIMER: usize = 5;
/// For drawing the next frame of something that's moving behind another window.
const COVERED_TIMER: usize = 6;
/// A timer for the app's own quiet work ([`crate::Quiet::Moment`]), once things have been quiet for [`QUIET_WORK`].
const QUIET_WORK_TIMER: usize = 7;
/// A timer for looking where the pointer is while it goes through a transparent window, which hears nothing from it then.
const PASS_THROUGH_TIMER: usize = 8;
/// A timer for the app's work between frames ([`crate::Quiet::Frame`]), once a frame is up and nothing else has come for
/// [`FRAME_QUIET`]. A timer's message only comes when nothing else is waiting, so it's after any input and painting.
const FRAME_QUIET_TIMER: usize = 9;
/// How long after a frame nothing else must have happened. Someone typing or clicking is between two of them for a tenth of
/// a second or more, and an animation draws a frame sooner than this, which puts it off.
const FRAME_QUIET: Duration = Duration::from_millis(20);
/// How long things must be quiet before the app's own quiet work, which rsui-tsx uses to collect all of its JavaScript's
/// garbage. That takes about 30 ms for an editor with 10,000 paragraphs, and someone who's paused for a second is likely to
/// carry on; past a few seconds they're less so, and their coming back costs a moment anyway, for the memory given back.
/// A caret stops blinking ten seconds after the last key, so in an editor this is fifteen.
const QUIET_WORK: Duration = Duration::from_secs(5);
/// How often something that's moving gets drawn while it's completely covered, which is also how often
/// that's looked into. It carries on rather than stopping, for the sake of the previews the system shows
/// of windows, and it's this long at most before it's moving properly again once it can be seen.
const COVERED_WAIT: Duration = Duration::from_millis(250);
/// How often the rate of the screen a window's on is looked at again, as it can be moved to another.
const FRAME_PACE_CHECKED: Duration = Duration::from_secs(1);
/// How long to wait after a frame of something moving that nobody got to see.
const UNSEEN_FRAME: Duration = Duration::from_millis(8);
const STARTUP_TRIM_MILLISECONDS: u32 = 250;
/// How much more can be in RAM while the GPU is starting up. See [`WorkingSetLimit`]. Less than this takes noticeably
/// longer to reach the first frame, and more only makes the peak higher.
const GPU_STARTUP_ROOM: usize = 4 * 1024 * 1024;
/// The least that's allowed in RAM while the GPU is starting up. Coming back from deep idle starts with almost nothing
/// in RAM, and needs this much to do it without slowing down.
const GPU_STARTUP_AT_LEAST: usize = 18 * 1024 * 1024;
/// Sent when the pointer leaves the window, after asking with `TrackMouseEvent`.
const WM_MOUSELEAVE: u32 = 0x02A3;
/// Windows reports 120 units for one wheel notch, which moves the web page by 100 logical pixels.
/// Fractional reports keep their fraction instead of waiting for a complete text row.
const WHEEL_UNITS_PER_NOTCH: i32 = 120;
const WHEEL_PIXELS_PER_NOTCH: f32 = 100.0;
/// Let queued continuous input make progress before another overdue frame. If
/// drawing itself exceeds a refresh, painting after every message otherwise leaves
/// practically no time to drain that input. Both bounds preserve paint fairness.
const INPUT_MESSAGES_PER_PAINT: usize = 32;
const INPUT_SLICE_PER_PAINT: Duration = Duration::from_millis(2);
/// The bit in a wheel message that says shift is held.
const MOUSE_WHEEL_SHIFT: usize = 0x0004;

/// Where the thread making a renderer leaves the result.
type RendererSlot = Arc<Mutex<Option<Result<Renderer, RenderError>>>>;

struct WindowState {
    app: App,
    id: WindowId,
    /// How small it can be made inside, in logical pixels, if that's limited.
    smallest: Option<(u32, u32)>,
    /// How big it can be made inside, in logical pixels, if that's limited.
    largest: Option<(u32, u32)>,
    /// What this has stopped from being used until it's gone.
    /// What draws. There's none until the driver has loaded, or after a long time minimized.
    renderer: Option<Renderer>,
    renderer_slot: RendererSlot,
    /// The thread making a renderer, if one is.
    building: Option<JoinHandle<()>>,
    /// Keeps the driver from filling RAM while it starts up.
    gpu_startup_limit: Option<WorkingSetLimit>,
    /// When memory was last given back, until the window's next used.
    gave_back: Option<Instant>,
    /// Whether something was covering the window, and when that was looked into.
    covered: Option<(Instant, bool)>,
    /// Whether the caret's timer is running.
    blink_timer: bool,
    /// Where the pointer last was, as the system also says it moved when it didn't.
    pointer_at: Option<(f32, f32)>,
    hidden: bool,
    /// Whether the window is showing a picture left there for when the GPU was let go of. If
    /// not, and there's no renderer, it shows its background color.
    showing_picture: bool,
    /// When the last frame of something moving was asked for.
    last_moving_frame: Option<Instant>,
    /// When native painting last began. Input can keep WM_PAINT out of the queue, but mustn't keep moving this deadline.
    last_paint_started: Option<Instant>,
    /// How long the screen the window's on shows each frame for, and when that was found out.
    frame_pace: Option<(Instant, Duration)>,
    /// When something last came in or changed, leaving out what moves by itself, and whether the app's quiet work has been
    /// done since while something's kept moving (see `ask_for_next_frame`).
    used_at: Instant,
    quiet_while_moving: bool,
    /// How long to wait before letting go of the GPU when nobody seems to be using the window.
    unattended_wait: Duration,
    /// When the GPU was last let go of with the window on screen.
    deep_idle_since: Option<Instant>,
    /// When memory was last in use, and whether a timer is waiting to see if that's still so.
    /// This doesn't reset a timer per event, as moving the pointer sends a lot of them.
    last_activity: Instant,
    trim_waiting: bool,
    quiet: Duration,
    cursors: [HCURSOR; 3],
    shown_cursor: Cursor,
    /// Whether the window's been told to take dropped files.
    accepts_files: bool,
    /// When the app last asked to hear from a timer, to save asking the system again.
    wake_at: Option<Instant>,
    /// What assistive technology can see, once it's asked, and what that was worked out from.
    access: Option<Arc<uia::Shared>>,
    access_fingerprint: Option<AccessFingerprint>,
    /// Whether the system will say when the pointer leaves the window.
    tracking_pointer: bool,
    /// Whether a mouse button went down in the window's own area and is still down, with the pointer held for it (`SetCapture`),
    /// so that what was pressed hears where it goes, as on a page, until it's let go of (see `hold_pointer`).
    pressing: bool,
    /// The first half of a character that arrived in two messages.
    high_surrogate: Option<u16>,
    /// What's being put together by a way of typing came to, once it's done, to say when it ends.
    composed: Option<String>,
    /// Whether the system tells this window about what's dragged over it from other programs.
    drop_target: bool,
    /// How it was before it covered the screen, if it is.
    before_fullscreen: Option<(i32, WINDOWPLACEMENT)>,
    /// Another program's window kept inside this one, as an inspector docked to it is, and whether the strip between
    /// them is being dragged.
    dock: Option<crate::dock::Docked>,
    dock_dragging: bool,
    /// The buttons under its picture in the taskbar, and whether the taskbar has its button yet, which they can only go on
    /// once it has, and whether they've gone on.
    thumbnail_buttons: Vec<crate::ThumbnailButton>,
    taskbar_button: bool,
    thumbnail_buttons_added: bool,
    thumbnail_icons: Vec<HICON>,
    /// Whether it has a backdrop, and whether it was asked to be transparent, either of which has it seen through.
    backdrop: bool,
    transparent: bool,
}

impl WindowState {
    /// Has the system say when the pointer leaves the window (`WM_MOUSELEAVE`), if it isn't already going to.
    fn track_pointer(&mut self, window: HWND) {
        if self.tracking_pointer {
            return;
        }
        self.tracking_pointer = true;
        let mut track = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: window,
            dwHoverTime: 0,
        };
        // SAFETY: `track` is filled in as this asks.
        unsafe {
            let _ = TrackMouseEvent(&mut track);
        }
    }

    fn scale(window: HWND) -> f32 {
        // SAFETY: `window` is the window this is running in.
        unsafe { GetDpiForWindow(window) as f32 / 96.0 }
    }

    /// Applies any changes, and arranges a frame if they made a difference.
    ///
    /// This doesn't draw. Input is coalesced until the next display deadline; the message loop services a due dirty frame
    /// even while input keeps arriving, since Windows otherwise postpones WM_PAINT until the queue is quiet.
    fn update_and_draw(&mut self, window: HWND) {
        #[cfg(feature = "idle-probe")]
        let probe_update = Instant::now();
        self.used_at = Instant::now();
        self.quiet_while_moving = false;
        self.update_frame(window);
        self.note_activity(window);
        self.restart_blinking(window);
        self.schedule_wake(window);
        #[cfg(feature = "idle-probe")]
        idle_probe::updated(probe_update.elapsed());
    }

    /// Asks to hear when the app has something it wants to do later.
    fn schedule_wake(&mut self, window: HWND) {
        #[cfg(feature = "idle-probe")]
        let _native_cycles = crate::native_cycles::enter(crate::native_cycles::SCHEDULE_WAKE);
        // Nothing needs to happen on time if nothing can be seen. Being shown again draws, which asks
        // again.
        let next = if self.hidden && !another_window_shows(window) {
            None
        } else {
            self.app.next_wake()
        };
        let next = match next {
            Some(wait) if self.is_covered(window) => Some(wait.max(COVERED_WAIT)),
            next => next,
        };
        let wanted = next.map(|wait| Instant::now() + wait);
        // Asking the system to change a timer isn't free, and the pointer moves a lot.
        let same = match (wanted, self.wake_at) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.max(b).duration_since(a.min(b)) < std::time::Duration::from_millis(16)
            }
            _ => false,
        };
        if same {
            return;
        }
        self.wake_at = wanted;
        // SAFETY: `window` is the window this is running in.
        unsafe {
            match next {
                Some(wait) => {
                    SetTimer(
                        Some(window),
                        WAKE_TIMER,
                        wait.as_millis().max(1) as u32,
                        None,
                    );
                }
                None => {
                    let _ = KillTimer(Some(window), WAKE_TIMER);
                }
            }
        }
    }

    fn update_frame(&mut self, window: HWND) {
        #[cfg(feature = "idle-probe")]
        let _native_cycles = crate::native_cycles::enter(crate::native_cycles::UPDATE_FRAME);
        if self.app.update() == Frame::Needed && !self.hidden {
            self.need_gpu(window);
            // SAFETY: `window` is the window this is running in.
            unsafe {
                let _ = InvalidateRect(Some(window), None, false);
            }
        }
        if let Some(title) = self.app.take_title_wanted() {
            // SAFETY: `window` is the window this is running in, and the text outlives the call.
            unsafe {
                let _ = SetWindowTextW(window, &HSTRING::from(title.as_str()));
            }
        }
        for url in self.app.take_urls_wanted() {
            crate::system::open_url(&url);
        }
        if let Some(dark) = self.app.take_dark_title_bar_wanted() {
            crate::system::set_dark_title_bar(window, dark);
        }
        if let Some(wanted) = self.app.take_dock_wanted() {
            self.change_dock(window, wanted);
        }
        // What's dragged in from other programs is told about as it's dragged, once something wants that.
        if !self.drop_target && rsui_runtime::wants_drops() {
            self.drop_target = crate::drop::listen(window);
        }
        let accepts = self.app.accepts_files();
        if accepts != self.accepts_files {
            self.accepts_files = accepts;
            // SAFETY: `window` is the window this is running in.
            unsafe {
                let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
                let flag = WS_EX_ACCEPTFILES.0 as isize;
                let style = if accepts { style | flag } else { style & !flag };
                SetWindowLongPtrW(window, GWL_EXSTYLE, style);
            }
        }
        self.publish_access();
    }

    /// The provider for assistive technology to start from, setting up what it needs the first time.
    fn access_root(&mut self, window: HWND) -> Option<IRawElementProviderSimple> {
        if self.access.is_none() {
            self.access_fingerprint = Some(self.app.access_fingerprint());
            let tree = self.app.access_tree();
            self.access = Some(uia::Shared::new(window, tree));
        }
        uia::root_provider(self.access.as_ref()?)
    }

    /// Brings what assistive technology can see up to date, if anything's asked to see it. Nothing
    /// here costs anything for a program nobody's asked about.
    fn publish_access(&mut self) {
        let Some(shared) = self.access.clone() else {
            return;
        };
        let fingerprint = self.app.access_fingerprint();
        if self.access_fingerprint.as_ref() == Some(&fingerprint) {
            return;
        }
        self.access_fingerprint = Some(fingerprint);
        let tree = self.app.access_tree();
        let Ok(mut held) = shared.tree.write() else {
            return;
        };
        let focus_of = |tree: &access::AccessTree| {
            tree.focused
                .and_then(|at| tree.nodes.get(at as usize))
                .map(|node| node.id)
        };
        let focus_moved = focus_of(&held) != focus_of(&tree);
        let structure_changed = !held.same_structure(&tree);
        let focus = focus_of(&tree);
        *held = tree;
        drop(held);
        if structure_changed {
            uia::raise_structure_changed(&shared);
        }
        if focus_moved {
            if let Some(focus) = focus {
                uia::raise_focus_changed(&shared, focus);
            }
        }
    }

    /// Carries out what assistive technology has asked for.
    fn run_access_commands(&mut self, window: HWND) {
        let Some(shared) = self.access.clone() else {
            return;
        };
        let commands = match shared.commands.lock() {
            Ok(mut commands) => std::mem::take(&mut *commands),
            Err(_) => return,
        };
        for (id, command) in commands {
            let path = shared
                .tree
                .read()
                .ok()
                .and_then(|tree| tree.find(id).map(|node| node.path.clone()));
            if let Some(path) = path {
                // There's no one to report a refusal to, as the request was accepted when it was made.
                let _ = self.app.feed(&Input::Access { path, command });
            }
        }
        self.update_and_draw(window);
    }

    /// Gets the GPU back if it was let go of, as it's needed now or is about to be.
    fn need_gpu(&mut self, window: HWND) {
        #[cfg(feature = "idle-probe")]
        let _native_cycles = crate::native_cycles::enter(crate::native_cycles::NEED_GPU);
        if self.renderer.is_none() && !self.hidden {
            if let Some(since) = self.deep_idle_since.take() {
                self.unattended_wait = if since.elapsed() < DEEP_IDLE_WORTHWHILE {
                    (self.unattended_wait * 2).min(DEEP_IDLE_LONGEST)
                } else {
                    DEEP_IDLE_UNATTENDED
                };
            }
            self.build_renderer(window);
        }
    }

    /// How long things have to be quiet before letting go of the GPU, as things stand now.
    fn deep_idle_wait(&self) -> Duration {
        if self.app.attended() {
            DEEP_IDLE.max(self.unattended_wait)
        } else {
            self.unattended_wait
        }
    }

    /// Starts making a renderer on another thread, as the driver takes about a tenth of a second
    /// to load, and the window stays responsive meanwhile.
    fn build_renderer(&mut self, window: HWND) {
        if self.building.is_some() {
            return;
        }
        let (building, limit) =
            start_building_renderer(window, &self.renderer_slot, self.app.see_through());
        self.gpu_startup_limit = limit;
        self.building = Some(building);
    }

    /// Takes on the renderer that another thread has finished making.
    fn renderer_ready(&mut self, window: HWND) {
        // This lasts through the first frame, which is the last of the startup, however this ends.
        let _limit = self.gpu_startup_limit.take();
        crate::mark("renderer ready");
        if let Some(building) = self.building.take() {
            let _ = building.join();
        }
        let result = self
            .renderer_slot
            .lock()
            .expect("the slot is only ever assigned")
            .take();
        match result {
            // It's no use now, and a new one will be made if the window comes back.
            Some(Ok(_)) if self.hidden => {}
            Some(Ok(mut renderer)) => {
                // The window may have changed size while this was being made.
                if let Err(error) = renderer.resize(client_size(window)) {
                    self.fail(window, error.into());
                    return;
                }
                // Or been given a backdrop, or had it taken away.
                if let Err(error) = renderer.set_see_through(self.app.see_through()) {
                    self.handle_render_error(window, error);
                    return;
                }
                // Without DirectComposition it can't be seen through, so it's drawn as it would be without a backdrop.
                self.app.set_see_through(renderer.is_see_through());
                self.renderer = Some(renderer);
                self.app.renderer_replaced();
                self.quiet = QUIET_AFTER_FIRST_FRAME;
                // SAFETY: `window` is the window this is running in.
                unsafe {
                    let _ = KillTimer(Some(window), IDLE_TIMER);
                    SetTimer(
                        Some(window),
                        STARTUP_TRIM_TIMER,
                        STARTUP_TRIM_MILLISECONDS,
                        None,
                    );
                }
                self.trim_waiting = false;
                self.draw(window);
                crate::mark("drawn");
                self.showing_picture = false;
                // What the system kept of the window is under what's drawn now, and shows where that's seen through.
                if self.app.see_through() {
                    clear_under(window);
                }
            }
            Some(Err(error)) => self.fail(window, error.into()),
            None => {}
        }
    }

    /// Lets go of the GPU completely.
    fn release_renderer(&mut self) {
        let logged = crate::input_log::now();
        let gave_up = self.release_renderer_now();
        crate::input_log::note("release", logged, format_args!("gave_up={gave_up}"));
    }

    fn release_renderer_now(&mut self) -> bool {
        #[cfg(feature = "idle-probe")]
        let probe_start = Instant::now();
        self.renderer = None;
        self.app.renderer_replaced();
        // The app's own long-quiet work (rsui-tsx collects its JavaScript's garbage), unless someone's already back.
        let gave_up = !input_waiting() && crate::app::run_quiet_hook(crate::app::Quiet::Long);
        // Drop renderer-owned allocations before the bounded idle reclaim.
        #[cfg(feature = "idle-probe")]
        let reclaim_start = Instant::now();
        let _released = if gave_up {
            reclaim_all()
        } else {
            rsui_alloc::UiAlloc.reclaim_for_idle(32)
        };
        #[cfg(feature = "idle-probe")]
        let reclaim_ns = reclaim_start.elapsed().as_nanos();
        give_back_memory();
        #[cfg(feature = "idle-probe")]
        idle_probe::boundary(self, "release", probe_start, _released, reclaim_ns);
        gave_up
    }

    /// The timer for letting go of the GPU has fired.
    fn release_timer(&mut self, window: HWND) {
        if self.hidden {
            // SAFETY: `window` is the window this is running in.
            unsafe {
                let _ = KillTimer(Some(window), RELEASE_TIMER);
            }
            self.release_renderer();
            return;
        }
        let quiet_for = self.last_activity.elapsed();
        let wait = self.deep_idle_wait();
        if quiet_for < wait {
            // SAFETY: `window` is the window this is running in.
            unsafe {
                SetTimer(Some(window), RELEASE_TIMER, millis(wait - quiet_for), None);
            }
            return;
        }
        // SAFETY: `window` is the window this is running in.
        unsafe {
            let _ = KillTimer(Some(window), RELEASE_TIMER);
        }
        self.enter_deep_idle(window);
    }

    /// Hands the picture over to the system, and lets go of the GPU.
    fn enter_deep_idle(&mut self, window: HWND) {
        // A window with no surface of the system's own has nowhere to leave a picture, and would vanish.
        if has_no_surface(window) {
            return;
        }
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        let (width, height) = renderer.size();
        let see_through = self.app.see_through();
        let scene = self.app.scene();
        let clear = clear_color(scene.window_color, see_through);
        match renderer.draw_to_memory(scene, clear) {
            Ok(pixels) if !pixels.is_empty() => {
                show_picture(window, &pixels, width, height);
                self.showing_picture = true;
                self.deep_idle_since = Some(Instant::now());
                self.release_renderer();
            }
            Ok(_) => {}
            Err(error) => self.handle_render_error(window, error),
        }
    }

    /// Starts the caret's blinking over if it needs to blink, so it holds still while typing.
    fn restart_blinking(&mut self, window: HWND) {
        self.blink(window, true);
    }

    /// Blinks if the caret should be, and goes back to the start of a blink if `restart` says to.
    /// Something happening in the background isn't a reason to, or the caret would stop blinking
    /// in an app that's often busy.
    fn blink(&mut self, window: HWND, restart: bool) {
        // SAFETY: `window` is the window this is running in.
        unsafe {
            let time = GetCaretBlinkTime();
            // The system's setting for never blinking is the largest number.
            let blinking = self.app.caret_blinking() && time != 0 && time != u32::MAX;
            if blinking && (restart || !self.blink_timer) {
                SetTimer(Some(window), CARET_TIMER, time, None);
            } else if !blinking && self.blink_timer {
                let _ = KillTimer(Some(window), CARET_TIMER);
            }
            self.blink_timer = blinking;
        }
    }

    fn show_cursor(&mut self) {
        self.shown_cursor = self.app.cursor();
        let cursor = self.cursors[self.shown_cursor as usize];
        // SAFETY: this is a cursor that came with the system, which stays loaded.
        unsafe {
            SetCursor(Some(cursor));
        }
    }

    /// Notes that memory is in use, so that it can be given back once things have gone quiet.
    fn note_activity(&mut self, window: HWND) {
        #[cfg(feature = "idle-probe")]
        let _native_cycles = crate::native_cycles::enter(crate::native_cycles::NOTE_ACTIVITY);
        self.last_activity = Instant::now();
        LAST_USED.with(|last| last.set(Some(self.last_activity)));
        if !self.trim_waiting {
            self.trim_waiting = true;
            if let Some(gave_back) = self.gave_back.take() {
                self.quiet = if gave_back.elapsed() < USED_AGAIN_SOON {
                    (self.quiet * 2).min(QUIET_LONGEST)
                } else {
                    QUIET
                };
            }
            // SAFETY: `window` is the window this is running in.
            unsafe {
                SetTimer(Some(window), IDLE_TIMER, millis(self.quiet), None);
            }
        }
    }

    /// Gives memory back if things are quiet, or looks again when they might be.
    fn idle_timer(&mut self, window: HWND) {
        // This is for all the windows, and one being in use means none should give anything back.
        let last_used = LAST_USED.with(Cell::get).unwrap_or(self.last_activity);
        let quiet_for = last_used.elapsed();
        if quiet_for < self.quiet {
            // SAFETY: `window` is the window this is running in.
            unsafe {
                SetTimer(
                    Some(window),
                    IDLE_TIMER,
                    millis(self.quiet - quiet_for),
                    None,
                );
            }
            return;
        }
        // Someone's back already, so it isn't quiet after all, and this would only be in their way.
        if input_waiting() {
            // SAFETY: `window` is the window this is running in.
            unsafe {
                SetTimer(Some(window), IDLE_TIMER, millis(self.quiet), None);
            }
            return;
        }
        // SAFETY: `window` is the window this is running in.
        unsafe {
            let _ = KillTimer(Some(window), IDLE_TIMER);
        }
        self.trim_waiting = false;
        // What happens right after the first frame says nothing about how the window's being used.
        if self.quiet == QUIET_AFTER_FIRST_FRAME {
            self.quiet = QUIET;
        } else {
            self.gave_back = Some(Instant::now());
        }
        #[cfg(feature = "idle-probe")]
        let probe_start = Instant::now();
        let logged = crate::input_log::now();
        self.app.forget_own_caches();
        let forgot = crate::input_log::now();
        // The app's own quiet work waits until things have been quiet longer.
        // SAFETY: `window` is the window this is running in.
        unsafe {
            SetTimer(
                Some(window),
                QUIET_WORK_TIMER,
                millis(QUIET_WORK.saturating_sub(quiet_for)),
                None,
            );
        }
        if let Some(renderer) = &mut self.renderer {
            self.app.release_unused_images(renderer);
        }
        if !self.hidden {
            // A window that isn't changing only needs one buffer.
            match self.renderer.as_mut().map(Renderer::compact) {
                Some(Ok(true)) => {
                    self.draw_frame(window);
                }
                Some(Err(error)) => self.handle_render_error(window, error),
                _ => {}
            }
            if self.renderer.is_some() {
                // SAFETY: `window` is the window this is running in.
                unsafe {
                    SetTimer(
                        Some(window),
                        RELEASE_TIMER,
                        millis(
                            self.deep_idle_wait()
                                .saturating_sub(self.last_activity.elapsed()),
                        ),
                        None,
                    );
                }
            }
        }
        match &mut self.renderer {
            Some(renderer) => renderer.trim(),
            None => give_back_memory(),
        }
        // One bounded call after cache and renderer storage release: at most 2 MiB.
        // Safe when the application uses a different global allocator.
        #[cfg(feature = "idle-probe")]
        let reclaim_start = Instant::now();
        let _released = rsui_alloc::UiAlloc.reclaim_for_idle(32);
        crate::input_log::note(
            "quiet",
            logged,
            format_args!("forget={} released={}", forgot - logged, _released),
        );
        #[cfg(feature = "idle-probe")]
        idle_probe::boundary(
            self,
            "idle",
            probe_start,
            _released,
            reclaim_start.elapsed().as_nanos(),
        );
    }

    /// Does the app's own quiet work ([`crate::Quiet::Moment`]) if things are still quiet, and nobody's back.
    fn quiet_work_timer(&mut self, window: HWND) {
        // SAFETY: `window` is the window this is running in.
        unsafe {
            let _ = KillTimer(Some(window), QUIET_WORK_TIMER);
        }
        let last_used = LAST_USED.with(Cell::get).unwrap_or(self.last_activity);
        // Something happened since, which goes quiet again in its own time and comes back here then.
        if last_used.elapsed() < QUIET_WORK || input_waiting() {
            return;
        }
        let logged = crate::input_log::now();
        let gave_up = crate::app::run_quiet_hook(crate::app::Quiet::Moment);
        let worked = crate::input_log::now();
        let mut released = 0;
        if gave_up {
            // A full collection empties far more of the allocator's chunks than 2 MiB, and touches what's still in use, so
            // those go back now, and so does what it touched.
            released = reclaim_all();
            give_back_memory();
        }
        crate::input_log::note(
            "work",
            logged,
            format_args!("hook={} released={released} gave_up={gave_up}", worked - logged),
        );
    }

    /// While something's moving, asks to draw again. Showing a frame waits for the screen, which is
    /// what normally paces these. A frame that looked the same isn't shown, and a window that's
    /// covered may not wait, so those wait here, or this would use a whole core for nothing. It's a
    /// plain sleep, as asking the GPU to wait for the screen can come straight back. How long doesn't
    /// matter much, because nobody saw that frame.
    fn ask_for_next_frame(&mut self, window: HWND) {
        if self.hidden || !self.app.animating() {
            self.last_moving_frame = None;
            return;
        }
        // Shown where nobody can see it, as behind a game that has the screen to itself: that's being covered, whatever the
        // windows in front say, and showing it came straight back.
        if self.renderer.as_ref().is_some_and(Renderer::was_occluded) {
            self.covered = Some((Instant::now(), true));
        }
        if self.is_covered(window) {
            self.last_moving_frame = None;
            // SAFETY: `window` is the window this is running in.
            unsafe {
                SetTimer(Some(window), COVERED_TIMER, millis(COVERED_WAIT), None);
            }
            return;
        }
        self.quiet_work_while_moving();
        let unseen = self.renderer.as_ref().is_some_and(Renderer::showed_nothing);
        if unseen {
            let _logged = crate::input_log::Span::new("phase-pacing-sleep");
            std::thread::sleep(UNSEEN_FRAME);
        } else if let Some(last) = self.last_moving_frame {
            // Showing a frame normally waits for the screen. If it came back sooner than the screen shows frames, nothing
            // waited, and without this the next frame would start at once, using a whole core for frames nobody sees.
            let pace = self.frame_pace(window);
            let since = last.elapsed();
            if since < pace * 3 / 4 {
                let _logged = crate::input_log::Span::new("phase-pacing-sleep");
                std::thread::sleep(pace - since);
            }
        }
        self.last_moving_frame = Some(Instant::now());
        // SAFETY: `window` is the window this is running in.
        unsafe {
            let _ = InvalidateRect(Some(window), None, false);
        }
    }

    /// The app's own quiet work ([`crate::Quiet::Moment`]), once, when nothing's come in or changed for [`QUIET_WORK`] but
    /// something's kept moving by itself, as a spinner does. The timers that do it for a still window never fire then, as a
    /// window that's moving always has a frame to paint, which Windows sees to before any timer; and a page whose spinner
    /// had it collect JavaScript's garbage only now and then kept what it made at every frame for many minutes.
    /// It's what a still window does after the same wait, less emptying the working set, which the next frame would only
    /// fault back in.
    fn quiet_work_while_moving(&mut self) {
        if self.quiet_while_moving || self.used_at.elapsed() < QUIET_WORK || input_waiting() {
            return;
        }
        self.quiet_while_moving = true;
        let logged = crate::input_log::now();
        let gave_up = crate::app::run_quiet_hook(crate::app::Quiet::Moment);
        let worked = crate::input_log::now();
        let released = if gave_up { reclaim_all() } else { 0 };
        crate::input_log::note(
            "work-moving",
            logged,
            format_args!(
                "hook={} released={released} gave_up={gave_up}",
                worked - logged
            ),
        );
    }

    /// How long the screen the window's on shows each frame for, looked at again now and then.
    fn frame_pace(&mut self, window: HWND) -> Duration {
        match self.frame_pace {
            Some((when, pace)) if when.elapsed() < FRAME_PACE_CHECKED => pace,
            _ => {
                let pace = frame_time_of(window);
                self.frame_pace = Some((Instant::now(), pace));
                pace
            }
        }
    }

    /// Whether nobody can see any of this window, as of a moment ago.
    fn is_covered(&mut self, window: HWND) -> bool {
        // Nothing can be covering where the pointer is, which saves looking.
        if self.tracking_pointer {
            return false;
        }
        match self.covered {
            Some((when, covered)) if when.elapsed() < COVERED_WAIT => covered,
            _ => {
                let covered = nothing_of_it_shows(window);
                self.covered = Some((Instant::now(), covered));
                covered
            }
        }
    }

    /// Tells what hears that something's scrolled of what's still waiting once a frame is up: a frame that moves something tells
    /// it as it starts (`App::tick`), so this is only for what moved while the frame was worked out. What that changes is drawn
    /// in a frame of its own.
    fn tell_scrolled(&mut self, window: HWND) {
        if self.hidden {
            return;
        }
        let logged = crate::input_log::now();
        if self.app.tell_scrolled() {
            crate::input_log::note("scrolled-told", logged, format_args!(""));
            self.update_frame(window);
            self.schedule_wake(window);
        }
    }

    fn draw(&mut self, window: HWND) {
        if self.draw_frame(window) {
            self.note_activity(window);
        }
    }

    /// Draws, and says whether that worked.
    fn draw_frame(&mut self, window: HWND) -> bool {
        #[cfg(feature = "idle-probe")]
        let _native_cycles = crate::native_cycles::enter(crate::native_cycles::DRAW_FRAME);
        let Some(renderer) = &mut self.renderer else {
            return false;
        };
        #[cfg(feature = "idle-probe")]
        let probe_start = Instant::now();
        let logged = crate::input_log::now();
        // What's scrolled is told as the frame starts (`App::tick`), and what's left once it's up (`tell_scrolled`).
        let result = self.app.draw_only(renderer);
        crate::input_log::note(
            "frame",
            logged,
            format_args!("ok={} showed_nothing={} occluded={}", result.is_ok(), renderer.showed_nothing(), renderer.was_occluded()),
        );
        #[cfg(feature = "idle-probe")]
        idle_probe::rendered(self, probe_start, result.is_ok());
        match result {
            Ok(()) => {
                self.place_embedded(window);
                // SAFETY: `window` is the window this is running in.
                unsafe {
                    SetTimer(Some(window), FRAME_QUIET_TIMER, millis(FRAME_QUIET), None);
                }
                true
            }
            Err(error) => {
                self.handle_render_error(window, error);
                false
            }
        }
    }

    /// Puts what's kept where elements are, as a web view is, where they've just been drawn. A window of its own can't be
    /// drawn over, so what's in this one doesn't draw over what's inside it.
    fn place_embedded(&mut self, window: HWND) {
        if !crate::embed::any() {
            return;
        }
        let places = self.app.embedded_places();
        if !places.is_empty() {
            // SAFETY: `window` is the window this is running in.
            unsafe {
                let style = GetWindowLongPtrW(window, GWL_STYLE);
                if style & WS_CLIPCHILDREN.0 as isize == 0 {
                    SetWindowLongPtrW(window, GWL_STYLE, style | WS_CLIPCHILDREN.0 as isize);
                }
            }
        }
        crate::embed::place_in(window.0 as isize, places);
    }

    fn handle_render_error(&mut self, window: HWND, error: RenderError) {
        match error {
            // A driver update or a GPU reset. Everything on the GPU is gone, so start over.
            RenderError::DeviceLost => {
                self.renderer = None;
                self.app.renderer_replaced();
                self.build_renderer(window);
            }
            error => self.fail(window, error.into()),
        }
    }

    /// Has the window seen through where nothing's drawn over it, or not, as it is while it has a backdrop.
    fn set_see_through(&mut self, window: HWND, on: bool) {
        if on == self.app.see_through() {
            return;
        }
        self.app.set_see_through(on);
        if let Some(renderer) = &mut self.renderer {
            match renderer.set_see_through(on) {
                // Without DirectComposition it can't be seen through, so it's drawn as it would be without a backdrop.
                Ok(()) => self.app.set_see_through(renderer.is_see_through()),
                Err(error) => self.handle_render_error(window, error),
            }
        }
        // A picture left for when the GPU was let go of is out of date now, so the GPU's needed again.
        self.showing_picture = false;
        clear_under(window);
        // SAFETY: `window` is the window this is running in.
        unsafe {
            let _ = InvalidateRect(Some(window), None, false);
        }
    }

    /// Tells the app where the window is on the screen now.
    fn tell_screen_origin(&mut self, window: HWND) {
        let (x, y) = screen_origin(window);
        self.app.set_screen_origin(x, y);
    }

    fn window_resized(&mut self, window: HWND, minimized: bool, size: (u32, u32)) {
        self.hidden = minimized;
        self.tell_screen_origin(window);
        self.showing_picture = false;
        if minimized {
            if let Some(renderer) = &mut self.renderer {
                if let Err(error) = renderer.hide() {
                    self.fail(window, error.into());
                    return;
                }
            }
            // SAFETY: `window` is the window this is running in.
            unsafe {
                SetTimer(Some(window), RELEASE_TIMER, RELEASE_MILLISECONDS, None);
            }
            return;
        }
        // SAFETY: `window` is the window this is running in.
        unsafe {
            let _ = KillTimer(Some(window), RELEASE_TIMER);
        }
        let scale = Self::scale(window);
        // With another window docked inside, the app has what's left of it.
        if self.dock.as_ref().is_some_and(|dock| !dock.alive()) {
            self.dock = None;
        }
        let content = self
            .dock
            .as_ref()
            .map_or(size, |dock| dock.content_size(size, scale));
        if let Some(dock) = &self.dock {
            dock.place(size, scale);
        }
        let _ = self.app.feed(&crate::replay::Input::Resize {
            width: content.0,
            height: content.1,
            scale,
        });
        match &mut self.renderer {
            Some(renderer) => {
                if let Err(error) = renderer.resize(size) {
                    self.fail(window, error.into());
                    return;
                }
                // This draws right away, as the system is waiting to show the new size.
                self.draw(window);
            }
            None => self.build_renderer(window),
        }
    }

    /// Takes another program's window in, or gives back the one that's in, as the app's been asked to.
    fn change_dock(&mut self, window: HWND, wanted: Option<crate::dock::DockRequest>) {
        // The one that's already inside is found there, as it's no longer a window of its own.
        let inside = |pid: u32| {
            self.dock
                .as_ref()
                .filter(|dock| dock.belongs_to(pid))
                .map(|dock| dock.child)
        };
        let found = wanted.and_then(|wanted| {
            Some((
                inside(wanted.pid).or_else(|| crate::dock::window_of(wanted.pid))?,
                wanted,
            ))
        });
        match (self.dock.as_mut(), found) {
            // The same one, somewhere else or another size.
            (Some(dock), Some((child, wanted))) if dock.child == child => {
                dock.side = wanted.side;
                dock.size = wanted.size;
            }
            (_, found) => {
                if let Some(dock) = self.dock.take() {
                    dock.give_back();
                }
                self.dock = found.map(|(child, wanted)| {
                    crate::dock::adopt(window, child, wanted.side, wanted.size)
                });
            }
        }
        self.relayout(window);
    }

    /// Lays everything out again in the window as it is.
    fn relayout(&mut self, window: HWND) {
        let size = client_size(window);
        self.window_resized(window, self.hidden, size);
    }

    /// Gives back what's docked inside, as the window's going.
    fn undock(&mut self) {
        if let Some(dock) = self.dock.take() {
            dock.give_back();
        }
    }

    /// Whether a point is on the strip between the app and what's docked inside.
    fn on_dock_gap(&self, window: HWND, x: f32, y: f32) -> bool {
        self.dock
            .as_ref()
            .is_some_and(|dock| dock.on_gap(client_size(window), Self::scale(window), x, y))
    }

    fn fail(&mut self, window: HWND, error: RunError) {
        if self.id == WindowId::MAIN {
            FAILURE.with(|failure| *failure.borrow_mut() = Some(error));
        } else {
            // The rest of the program can carry on without it.
            eprintln!("A window had to close: {error}");
        }
        destroy_window(window);
    }
}

/// A window and what's kept for it.
struct OpenWindow {
    window: HWND,
    id: WindowId,
    /// The window it's modal to, which can't be used until it's gone.
    disabled_owner: Option<HWND>,
    state: Rc<RefCell<WindowState>>,
}

thread_local! {
    /// When any window was last used, as giving memory back is for the whole program.
    static LAST_USED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// Whether a key, a click or the wheel is being handled, which a GPU starting up for it isn't held to a few megabytes for.
    static HANDLING_INPUT: Cell<bool> = const { Cell::new(false) };
    /// Why the main window closed, if something went wrong.
    static FAILURE: RefCell<Option<RunError>> = const { RefCell::new(None) };
    /// The windows that are open, with the main one first. The system calls `window_procedure` with no way to pass
    /// anything through safely, and a window belongs to one thread, so this is the place for it.
    static WINDOWS: RefCell<Vec<OpenWindow>> = const { RefCell::new(Vec::new()) };
}

/// Whether someone has asked Windows for things not to move.
fn system_allows_animations() -> bool {
    let mut allowed = BOOL(1);
    // SAFETY: `allowed` is what this setting is read into.
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(std::ptr::from_mut(&mut allowed).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    allowed.as_bool()
}

fn with_state(window: HWND, run: impl FnOnce(&mut WindowState)) -> bool {
    // It isn't held on to the list while this runs, as what it does can open and close others.
    let found = WINDOWS.with(|windows| {
        windows
            .borrow()
            .iter()
            .find(|open| open.window == window)
            .map(|open| open.state.clone())
    });
    let Some(found) = found else {
        return false;
    };
    match found.try_borrow_mut() {
        Ok(mut state) => {
            run(&mut state);
            true
        }
        // Drawing can make the system call back in straight away, such as during a resize.
        // There's nothing to do in that case, as drawing is already underway.
        Err(_) => false,
    }
}

/// Gives dirty windows a frame opportunity between dispatched messages, even if input never leaves the queue empty.
/// The handle buffer is reused by the message loop, and no window-list or state borrow is held while UpdateWindow sends
/// WM_PAINT synchronously. The deadline comes from the previous paint, not the latest move, so input can't defer it forever.
fn present_due_windows(handles: &mut Vec<HWND>) -> bool {
    #[cfg(feature = "idle-probe")]
    let _native_cycles = crate::native_cycles::enter(crate::native_cycles::PRESENT_DUE);
    let mut presented = false;
    handles.clear();
    WINDOWS.with(|windows| handles.extend(windows.borrow().iter().map(|open| open.window)));
    for &window in handles.iter() {
        // SAFETY: this only asks whether a window has a dirty region, and harmlessly fails if it has just closed.
        if !unsafe { GetUpdateRect(window, None, false) }.as_bool() {
            continue;
        }
        let mut due = false;
        with_state(window, |state| {
            if !state.hidden {
                let pace = state.frame_pace(window);
                due = state.last_paint_started.is_none_or(|last| last.elapsed() >= pace);
            }
        });
        if due {
            // SAFETY: no WindowState borrow survives this call. The normal paint handler performs all rendering and
            // validation; it may safely open or close other windows while doing so.
            presented |= unsafe { UpdateWindow(window) }.as_bool();
        }
    }
    presented
}

#[derive(Default)]
struct InputPaintBatch {
    began: Option<Instant>,
    messages: usize,
}

impl InputPaintBatch {
    fn checkpoint(&mut self, began: Instant, now: Instant, continuous_input_waiting: bool) -> bool {
        let first = *self.began.get_or_insert(began);
        self.messages = self.messages.saturating_add(1);
        !continuous_input_waiting
            || self.messages >= INPUT_MESSAGES_PER_PAINT
            || now.saturating_duration_since(first) >= INPUT_SLICE_PER_PAINT
    }

    fn painted(&mut self) {
        *self = Self::default();
    }
}

fn continuous_input(message: u32) -> bool {
    // Timers/background jobs enqueue WAKE even when an intervening input update
    // has already consumed their work. Dispatch it in order within the same short
    // service slice rather than forcing another frame on both sides of that wake.
    matches!(message, WM_MOUSEMOVE | WM_MOUSEWHEEL | WM_MOUSEHWHEEL | WAKE)
}

fn more_continuous_input(message: &MSG) -> bool {
    #[cfg(feature = "idle-probe")]
    let _native_cycles = crate::native_cycles::enter(crate::native_cycles::INPUT_QUEUE_CHECK);
    if !continuous_input(message.message) {
        return false;
    }
    let mut next = MSG::default();
    // SAFETY: inspect without removing the next message. Filtering by input type
    // would skip an intervening command, so the whole thread queue is checked.
    unsafe { PeekMessageW(&mut next, None, 0, 0, PM_NOREMOVE) }.as_bool()
        && next.hwnd == message.hwnd
        && continuous_input(next.message)
}

/// Whether any other window can be seen.
fn another_window_shows(this: HWND) -> bool {
    WINDOWS.with(|windows| {
        windows.borrow().iter().any(|open| {
            // SAFETY: asking about a window can't go wrong, and only says no if it's gone.
            open.window != this
                && unsafe {
                    IsWindowVisible(open.window).as_bool() && !IsIconic(open.window).as_bool()
                }
        })
    })
}

/// Whether a window was made without a surface of the system's own, as one that opens transparent is, so that what
/// DirectComposition shows is all there is of it.
fn has_no_surface(window: HWND) -> bool {
    // SAFETY: this only asks about the window.
    unsafe { GetWindowLongW(window, GWL_EXSTYLE) as u32 & WS_EX_NOREDIRECTIONBITMAP.0 != 0 }
}

/// Whether anything's drawn at this point on the screen in a window, as far as what was last drawn says. While a button's
/// held, or the window can't be asked, it counts as drawn, so that the pointer doesn't go elsewhere in the middle of a drag.
fn drawn_at_screen(window: HWND, at: POINT) -> bool {
    // SAFETY: these only ask, and write to what's valid.
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetCapture, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON};
        if GetCapture() == window
            || [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON].iter().any(|button| GetAsyncKeyState(i32::from(button.0)) < 0)
        {
            return true;
        }
        let mut inside = at;
        let _ = ScreenToClient(window, &raw mut inside);
        let mut drawn = true;
        with_state(window, |state| drawn = state.app.drawn_at(inside.x as f32, inside.y as f32));
        drawn
    }
}

/// Whether a window is seen through where nothing's drawn, as it is while it has a backdrop.
fn is_see_through(window: HWND) -> bool {
    let mut on = false;
    with_state(window, |state| on = state.app.see_through());
    on
}

/// Fills what the system keeps of a window that's seen through with black, which is see-through there too, so that only
/// what's drawn now shows over its backdrop.
fn clear_under(window: HWND) {
    // SAFETY: the device context is given back straight after, and the brush is the system's, which stays.
    unsafe {
        let context = GetDC(Some(window));
        fill_black(window, context);
        ReleaseDC(Some(window), context);
    }
}

/// Fills a window with black, through a device context of its.
///
/// # Safety
///
/// `context` has to be a device context for `window`.
unsafe fn fill_black(window: HWND, context: HDC) {
    let mut area = RECT::default();
    // SAFETY: as above, and `area` is written by the call.
    unsafe {
        let _ = GetClientRect(window, &raw mut area);
        FillRect(context, &raw const area, HBRUSH(GetStockObject(BLACK_BRUSH).0));
    }
}

/// Puts a picture in the window, where the system keeps it without any help. For a window that's seen through, its
/// fourth byte is how opaque each pixel is, with the colors multiplied by that, which is what the system takes there.
fn show_picture(window: HWND, pixels: &[u8], width: u32, height: u32) {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            // Negative means the rows go from the top.
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: `pixels` holds `width` by `height` of what `info` describes, and the device context
    // is given back straight after.
    unsafe {
        let context = GetDC(Some(window));
        SetDIBitsToDevice(
            context,
            0,
            0,
            width,
            height,
            0,
            0,
            0,
            height,
            pixels.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
        );
        let _ = GdiFlush();
        ReleaseDC(Some(window), context);
    }
}

/// Gives the keyboard back to the window when it's clicked, if a window inside it had it, as a web view or something docked
/// does once it's been clicked. The system leaves it there otherwise.
fn take_keyboard_back(window: HWND) {
    // SAFETY: `window` is valid for a message sent to it, and it's this thread's.
    unsafe {
        if GetFocus() != window {
            let _ = SetFocus(Some(window));
        }
    }
}

fn millis(duration: Duration) -> u32 {
    (duration.as_millis() as u32).max(1)
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetTickCount() -> u32;
}

/// When the message being handled was sent, as an `Instant`: up to a second ago.
fn message_instant() -> Instant {
    // SAFETY: these only read the clock and what the system kept about this thread's message.
    let age = unsafe { GetTickCount().wrapping_sub(GetMessageTime() as u32) };
    let now = Instant::now();
    now.checked_sub(Duration::from_millis(u64::from(age.min(1000)))).unwrap_or(now)
}

fn wheel_delta(wparam: WPARAM) -> i32 {
    i32::from((wparam.0 >> 16) as u16 as i16)
}

fn precise_wheel_delta(delta: i32) -> bool {
    delta % WHEEL_UNITS_PER_NOTCH != 0
}

fn wheel_pixels(delta: i32, horizontal: bool) -> f32 {
    let pixels = delta as f32 * (WHEEL_PIXELS_PER_NOTCH / WHEEL_UNITS_PER_NOTCH as f32);
    if horizontal { pixels } else { -pixels }
}

/// Whether a key, a click or the pointer moving is waiting to be handled on this thread.
fn input_waiting() -> bool {
    // SAFETY: this only asks about this thread's queue. The high word is what's in it now.
    unsafe { GetQueueStatus(QS_INPUT) >> 16 != 0 }
}

/// Gives back every one of the allocator's empty chunks but the last emptied of each size, which stay for what comes next.
fn reclaim_all() -> usize {
    let mut released = 0;
    // 32 chunks a call is 2 MiB, so this is up to 1 GiB, far past what any app's quiet collection empties.
    for _ in 0..512 {
        let some = rsui_alloc::UiAlloc.reclaim_for_idle(32);
        released += some;
        if some == 0 {
            break;
        }
    }
    released
}

/// `DwmGetWindowAttribute`, which is only looked for once something's moving, so that a program that
/// never needs it doesn't load another library to start up.
type GetWindowAttribute = unsafe extern "system" fn(HWND, u32, *mut c_void, u32) -> i32;

fn get_window_attribute() -> Option<GetWindowAttribute> {
    static FOUND: OnceLock<Option<GetWindowAttribute>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        // SAFETY: the library stays loaded for good, and the function has this signature.
        unsafe {
            let library = LoadLibraryW(w!("dwmapi.dll")).ok()?;
            let function = GetProcAddress(library, s!("DwmGetWindowAttribute"))?;
            Some(std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                GetWindowAttribute,
            >(function))
        }
    })
}

/// Whether a window is on another virtual desktop, or hidden in some other way that it doesn't say itself.
fn is_cloaked(window: HWND, get: GetWindowAttribute) -> bool {
    const DWMWA_CLOAKED: u32 = 14;
    let mut cloaked = 0u32;
    // SAFETY: `cloaked` is the size that's said, and a closed window only gives an error.
    let result = unsafe {
        get(
            window,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            size_of::<u32>() as u32,
        )
    };
    result >= 0 && cloaked != 0
}

/// Whether `outer` has all of `inner` inside it.
fn contains(outer: RECT, inner: RECT) -> bool {
    outer.left <= inner.left
        && outer.top <= inner.top
        && outer.right >= inner.right
        && outer.bottom >= inner.bottom
}

/// Whether none of this window can be seen, as far as a quick look can tell. It only says yes if it's on
/// another desktop or one window covers it all, so several windows covering it between them don't count.
/// Saying no when it could have been yes only costs what it always did, while the other way around would
/// stop something that can be seen, so anything that could be see-through doesn't count.
fn nothing_of_it_shows(window: HWND) -> bool {
    const DWMWA_EXTENDED_FRAME_BOUNDS: u32 = 9;
    // SAFETY: all of these take a window that could have closed, which only gives an error, and what they
    // write to is valid.
    unsafe {
        if GetForegroundWindow() == window {
            return false;
        }
        let Some(get) = get_window_attribute() else {
            return false;
        };
        if is_cloaked(window, get) {
            return true;
        }
        let mut own = RECT::default();
        if GetClientRect(window, &mut own).is_err() {
            return false;
        }
        let mut corner = POINT::default();
        if !windows::Win32::Graphics::Gdi::ClientToScreen(window, &mut corner).as_bool() {
            return false;
        }
        let own = RECT {
            left: corner.x,
            top: corner.y,
            right: corner.x + own.right,
            bottom: corner.y + own.bottom,
        };
        let mut above = GetWindow(window, GW_HWNDPREV);
        while let Ok(other) = above {
            above = GetWindow(other, GW_HWNDPREV);
            let extended = GetWindowLongW(other, GWL_EXSTYLE) as u32;
            if !IsWindowVisible(other).as_bool()
                || IsIconic(other).as_bool()
                || extended & (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0 | WS_EX_NOREDIRECTIONBITMAP.0)
                    != 0
                || is_cloaked(other, get)
            {
                continue;
            }
            // Without the shadow, which is part of what its rectangle says it has.
            let mut frame = RECT::default();
            let got = get(
                other,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                (&raw mut frame).cast(),
                size_of::<RECT>() as u32,
            );
            if got >= 0 && contains(frame, own) {
                return true;
            }
        }
        false
    }
}

/// The cursor for the strip between the app and what's docked inside, if the pointer's on it.
fn dock_cursor(window: HWND) -> Option<PCWSTR> {
    let mut point = POINT::default();
    // SAFETY: `point` is valid to write to, and `window` is the window this is running in.
    unsafe {
        GetCursorPos(&mut point).ok()?;
        let _ = ScreenToClient(window, &mut point);
    }
    let mut cursor = None;
    with_state(window, |state| {
        if state.dock_dragging || state.on_dock_gap(window, point.x as f32, point.y as f32) {
            cursor = state.dock.as_ref().map(crate::dock::Docked::cursor);
        }
    });
    cursor
}

/// Whether the strip between the app and what's docked inside is being dragged.
fn dock_dragging(window: HWND) -> bool {
    let mut dragging = false;
    with_state(window, |state| dragging = state.dock_dragging);
    dragging
}

fn client_size(window: HWND) -> (u32, u32) {
    let mut client = RECT::default();
    // SAFETY: `client` is valid to write to. A closed window leaves it empty, which is harmless.
    unsafe {
        let _ = GetClientRect(window, &mut client);
    }
    (client.right.max(0) as u32, client.bottom.max(0) as u32)
}

/// Something from another program is being dragged over this window, or dropped on it, which the app hears about. Says
/// whether it can be dropped there.
pub(crate) fn drag_from_outside(
    window: HWND,
    phase: crate::app::DropPhase,
    x: f32,
    y: f32,
    held: Modifiers,
    files: &[String],
    text: Option<&str>,
) -> bool {
    let mut accepted = false;
    with_state(window, |state| {
        state.need_gpu(window);
        let _ = state.app.feed(&Input::Modifiers(held));
        accepted = state
            .app
            .feed(&Input::DragFromOutside {
                phase,
                x,
                y,
                files: files.to_vec(),
                text: text.map(str::to_owned),
            })
            .unwrap_or(false);
        state.update_and_draw(window);
    });
    accepted
}

/// Where the window's top left is on the screen, in physical pixels.
fn screen_origin(window: HWND) -> (f32, f32) {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` is valid to write to.
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::ClientToScreen(window, &raw mut point);
    }
    (point.x as f32, point.y as f32)
}

/// How long the main screen shows each frame for.
fn frame_time() -> Duration {
    frame_time_on(PCWSTR::null())
}

/// How long the screen a window's mostly on shows each frame for.
fn frame_time_of(window: HWND) -> Duration {
    use windows::Win32::Graphics::Gdi::MONITORINFOEXW;
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: `info` is the size it says, and asking about a window that's gone leaves it empty.
    let found = unsafe {
        GetMonitorInfoW(
            MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST),
            (&raw mut info).cast(),
        )
    }
    .as_bool();
    if found {
        frame_time_on(PCWSTR(info.szDevice.as_ptr()))
    } else {
        frame_time()
    }
}

/// How long a screen shows each frame for: the one named, or the main one for a null name.
fn frame_time_on(device: PCWSTR) -> Duration {
    use windows::Win32::Graphics::Gdi::{DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW};
    let mut mode = DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..DEVMODEW::default()
    };
    // SAFETY: `mode` is as big as it says, and `device` is null or a name that ends in a zero.
    let known =
        unsafe { EnumDisplaySettingsW(device, ENUM_CURRENT_SETTINGS, &raw mut mode) }.as_bool();
    let hertz = if known && mode.dmDisplayFrequency > 1 {
        mode.dmDisplayFrequency
    } else {
        60
    };
    Duration::from_micros(1_000_000 / u64::from(hertz))
}

/// Which button a message about one other than the main one is about.
fn other_button(message: u32, wparam: WPARAM) -> PointerButton {
    match message {
        WM_RBUTTONDOWN | WM_RBUTTONUP => PointerButton::Secondary,
        WM_MBUTTONDOWN | WM_MBUTTONUP => PointerButton::Middle,
        // The high word says which of the two extra buttons it is.
        _ if (wparam.0 >> 16) & 0xFFFF == 2 => PointerButton::Forward,
        _ => PointerButton::Back,
    }
}

fn pointer_position(parameter: LPARAM) -> (f32, f32) {
    let x = (parameter.0 & 0xFFFF) as u16 as i16;
    let y = ((parameter.0 >> 16) & 0xFFFF) as u16 as i16;
    (f32::from(x), f32::from(y))
}

/// The key this code is for, and `None` for any rsui doesn't use.
fn key_code(code: u16) -> Option<KeyCode> {
    let key = VIRTUAL_KEY(code);
    Some(match key {
        VK_TAB => KeyCode::Tab,
        VK_RETURN => KeyCode::Enter,
        VK_ESCAPE => KeyCode::Escape,
        VK_BACK => KeyCode::Backspace,
        VK_DELETE => KeyCode::Delete,
        VK_SPACE => KeyCode::Space,
        VK_LEFT => KeyCode::Left,
        VK_RIGHT => KeyCode::Right,
        VK_UP => KeyCode::Up,
        VK_DOWN => KeyCode::Down,
        VK_HOME => KeyCode::Home,
        VK_END => KeyCode::End,
        VK_PRIOR => KeyCode::PageUp,
        VK_NEXT => KeyCode::PageDown,
        VK_INSERT => KeyCode::Insert,
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => KeyCode::Shift,
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL => KeyCode::Control,
        VK_MENU | VK_LMENU | VK_RMENU => KeyCode::Alt,
        VK_LWIN | VK_RWIN => KeyCode::Meta,
        VK_CAPITAL => KeyCode::CapsLock,
        VK_NUMLOCK => KeyCode::NumLock,
        VK_SCROLL => KeyCode::ScrollLock,
        VK_PAUSE => KeyCode::Pause,
        VK_SNAPSHOT => KeyCode::PrintScreen,
        VK_APPS => KeyCode::ContextMenu,
        VK_PROCESSKEY => KeyCode::Process,
        VK_MULTIPLY => KeyCode::Character('*'),
        VK_ADD => KeyCode::Character('+'),
        VK_SUBTRACT => KeyCode::Character('-'),
        VK_DECIMAL => KeyCode::Character('.'),
        VK_DIVIDE => KeyCode::Character('/'),
        _ if (VK_F1.0..=VK_F24.0).contains(&code) => KeyCode::Function((code - VK_F1.0 + 1) as u8),
        _ if (VK_NUMPAD0.0..=VK_NUMPAD9.0).contains(&code) => {
            KeyCode::Character(char::from(b'0' + (code - VK_NUMPAD0.0) as u8))
        }
        // Letters and digits have the same codes as their capital ASCII characters.
        _ => match u8::try_from(code).map(char::from) {
            Ok(character @ ('A'..='Z' | '0'..='9')) => {
                KeyCode::Character(character.to_ascii_lowercase())
            }
            _ => {
                // Anything else that types something, as `,` or `ß` does, is what it types here, without shift.
                // SAFETY: this only asks the system what the keyboard says.
                let typed = unsafe { MapVirtualKeyW(u32::from(code), MAPVK_VK_TO_CHAR) };
                // The top bit says it's a key that only goes with the next, which is still what it shows.
                let typed =
                    char::from_u32(typed & 0x7FFF_FFFF).filter(|typed| !typed.is_control())?;
                KeyCode::Character(typed.to_lowercase().next().unwrap_or(typed))
            }
        },
    })
}

fn key_held(key: VIRTUAL_KEY) -> bool {
    // SAFETY: this only reads the state of the keyboard. The high bit means it's down.
    unsafe { GetKeyState(i32::from(key.0)) < 0 }
}

/// What's held on the keyboard right now, as far as this window's messages go.
fn modifiers_held() -> Modifiers {
    #[cfg(feature = "idle-probe")]
    let _native_cycles = crate::native_cycles::enter(crate::native_cycles::MODIFIERS_READ);
    let alt = key_held(VK_MENU);
    Modifiers {
        // The AltGr key reports both of these, and is used for typing.
        ctrl: key_held(VK_CONTROL) && !alt,
        shift: key_held(VK_SHIFT),
        alt,
        meta: key_held(VK_LWIN) || key_held(VK_RWIN),
    }
}

/// The key a message is about, with what's held, where it is, and what it types on this keyboard.
fn key_press(wparam: WPARAM, lparam: LPARAM, alt: bool) -> Option<KeyPress> {
    let virtual_key = wparam.0 as u16;
    let code = key_code(virtual_key)?;
    let held = modifiers_held();
    // Bits 16 to 23 are where it is, and bit 24 says it's one of the keys the system calls extended.
    let mut scan = ((lparam.0 >> 16) & 0xFF) as u16;
    if lparam.0 & (1 << 24) != 0 {
        scan |= 0xE000;
    }
    Some(KeyPress {
        code,
        ctrl: held.ctrl,
        shift: held.shift,
        // Alt by itself comes as the system's kind of key, and with control it's AltGr, which types.
        alt: alt && held.alt,
        // Bit 30 says whether the key was already down.
        repeat: lparam.0 & (1 << 30) != 0,
        meta: held.meta,
        scan,
        typed: typed_by(virtual_key, scan, held.ctrl),
    })
}

/// What this key types on this keyboard with what's held, apart from control, which doesn't change what a key means.
fn typed_by(virtual_key: u16, scan: u16, ctrl: bool) -> Option<char> {
    let mut state = [0u8; 256];
    let mut out = [0u16; 4];
    // SAFETY: both are as big as these ask. Bit 2 of the flags keeps what's being put together with a key like `^`, which
    // asking would otherwise use up.
    unsafe {
        GetKeyboardState(&mut state).ok()?;
        if ctrl {
            for key in [VK_CONTROL, VK_LCONTROL, VK_RCONTROL] {
                state[key.0 as usize] = 0;
            }
        }
        let count = ToUnicode(
            u32::from(virtual_key),
            u32::from(scan & 0xFF),
            Some(&state),
            &mut out,
            4,
        );
        if count != 1 {
            return None;
        }
    }
    char::from_u32(u32::from(out[0])).filter(|typed| !typed.is_control())
}

/// The system's clipboard. It can be in use by another program, and failing to reach it only
/// means that this copy or paste does nothing.
struct SystemClipboard(HWND);

impl Clipboard for SystemClipboard {
    fn read(&mut self) -> Option<String> {
        // SAFETY: the clipboard's memory holds a terminated string while it's locked, and
        // stays valid until the clipboard is closed.
        unsafe {
            OpenClipboard(Some(self.0)).ok()?;
            let text = GetClipboardData(u32::from(CF_UNICODETEXT.0))
                .ok()
                .and_then(|handle| {
                    let memory = HGLOBAL(handle.0);
                    let start = GlobalLock(memory).cast::<u16>();
                    if start.is_null() {
                        return None;
                    }
                    let mut length = 0;
                    while *start.add(length) != 0 {
                        length += 1;
                    }
                    let text = String::from_utf16_lossy(std::slice::from_raw_parts(start, length));
                    let _ = GlobalUnlock(memory);
                    Some(text)
                });
            let _ = CloseClipboard();
            text
        }
    }

    fn write(&mut self, text: &str) {
        let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: the allocation holds all of `units`, and the system takes ownership of it
        // only if `SetClipboardData` succeeds. It's freed otherwise.
        unsafe {
            if OpenClipboard(Some(self.0)).is_err() {
                return;
            }
            let _ = EmptyClipboard();
            if let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2) {
                let target = GlobalLock(memory).cast::<u16>();
                let mut stored = false;
                if !target.is_null() {
                    std::ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
                    let _ = GlobalUnlock(memory);
                    stored = SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(memory.0)))
                        .is_ok();
                }
                if !stored {
                    let _ = GlobalFree(Some(memory));
                }
            }
            let _ = CloseClipboard();
        }
    }
}

unsafe extern "system" fn window_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // With `RSUI_INPUT_LOG`, how long each key, character and click took is written down (see input_log.rs).
    let input = match message {
        WM_KEYDOWN | WM_SYSKEYDOWN => Some("keydown"),
        WM_KEYUP | WM_SYSKEYUP => Some("keyup"),
        WM_CHAR => Some("char"),
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => Some("down"),
        WM_LBUTTONUP => Some("up"),
        WM_MOUSEMOVE => Some("move"),
        WM_MOUSEWHEEL => Some("wheel"),
        WM_MOUSEHWHEEL => Some("hwheel"),
        WM_SIZE => Some("resize"),
        _ => None,
    };
    // While the GPU is starting up, what's in RAM is held to a few megabytes past what was there (`GPU_STARTUP_ROOM`), which
    // coming back from deep idle is almost nothing. Someone typing or clicking then has the app touch far more than that
    // (a big editor, hundreds of megabytes), which only thrashes against the limit: a key once took 1.8 s that way
    // in Lexical with 10,000 paragraphs. So the limit ends once there's input to handle, other than the pointer moving in,
    // and a GPU that starts up for that input has none.
    let handling_input = matches!(
        message,
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_CHAR | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_MOUSEWHEEL | WM_MOUSEHWHEEL
    );
    if handling_input {
        with_state(window, |state| state.gpu_startup_limit = None);
        let result = HANDLING_INPUT.with(|handling| {
            let was = handling.replace(true);
            // SAFETY: these are the arguments this was given.
            let result = unsafe { window_procedure_logged(window, message, wparam, lparam, input) };
            handling.set(was);
            result
        });
        return result;
    }
    // SAFETY: these are the arguments this was given.
    unsafe { window_procedure_logged(window, message, wparam, lparam, input) }
}

/// [`handle_message`], written down if `RSUI_INPUT_LOG` asks and it's `input`.
///
/// # Safety
///
/// As for a window procedure: the arguments are what the system passed one.
unsafe fn window_procedure_logged(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    input: Option<&str>,
) -> LRESULT {
    if let Some(what) = input
        && crate::input_log::on()
    {
        let start = crate::input_log::now();
        // This is the millisecond, wrapping Windows message clock, not the QPC clock above.
        // Sent messages (including synchronous resize) need not have a meaningful queued timestamp.
        let queue_age_ms = unsafe { GetTickCount().wrapping_sub(GetMessageTime() as u32) };
        // SAFETY: these are the arguments this was given.
        let result = unsafe { handle_message(window, message, wparam, lparam) };
        crate::input_log::note(what, start, format_args!("{} queue_age_ms_coarse={queue_age_ms}", wparam.0));
        return result;
    }
    // SAFETY: these are the arguments this was given.
    unsafe { handle_message(window, message, wparam, lparam) }
}

/// What [`window_procedure`] does with a message.
///
/// # Safety
///
/// As for a window procedure: the arguments are what the system passed one.
unsafe fn handle_message(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(result) = crate::shell_layer::before_message(window, message, wparam, lparam) {
        return result;
    }
    match message {
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            // SAFETY: `window` is valid for a message sent to it, and `paint` is written
            // by `BeginPaint` for `EndPaint` to read.
            unsafe {
                BeginPaint(window, &mut paint);
            }
            with_state(window, |state| {
                if !state.hidden {
                    state.last_paint_started = Some(Instant::now());
                    // Whatever's moving goes to where it should be by now, then gets drawn there.
                    let moving = state.app.animating();
                    if moving {
                        state.app.tick(Instant::now());
                        state.app.update();
                    }
                    state.draw(window);
                    // An image starts playing when it first gets painted, so this is the first that's known of it.
                    state.schedule_wake(window);
                    // Assistive technology hears where it ended up, not about every step.
                    if moving && !state.app.animating() {
                        state.publish_access();
                    }
                    // Something else may have disturbed what's on screen, which only a new
                    // frame can put right.
                    if state.renderer.is_none() {
                        state.need_gpu(window);
                    }
                }
            });
            // SAFETY: as above.
            unsafe {
                let _ = EndPaint(window, &paint);
                let _ = ValidateRect(Some(window), None);
            }
            with_state(window, |state| {
                state.tell_scrolled(window);
                state.ask_for_next_frame(window);
            });
            LRESULT(0)
        }
        WM_ERASEBKGND => {
            // With no renderer yet, this shows the background color rather than nothing.
            let (mut fill, mut see_through) = (false, false);
            with_state(window, |state| {
                fill = state.renderer.is_none() && !state.showing_picture;
                see_through = state.app.see_through() && !state.showing_picture;
            });
            if see_through {
                // Black is see-through on a window whose frame goes all the way in, and what the system keeps of a
                // window shows under where what's drawn is see-through, so that's all it can have.
                // SAFETY: `wparam` is this window's device context, for this message.
                unsafe { fill_black(window, HDC(wparam.0 as *mut c_void)) };
                LRESULT(1)
            } else if fill {
                // SAFETY: these are the arguments this was given.
                unsafe { DefWindowProcW(window, message, wparam, lparam) }
            } else {
                LRESULT(1)
            }
        }
        WM_SIZE => {
            let width = (lparam.0 & 0xFFFF) as u32;
            let height = ((lparam.0 >> 16) & 0xFFFF) as u32;
            let minimized = wparam.0 == SIZE_MINIMIZED as usize;
            with_state(window, |state| {
                // Minimized, or brought back from that, a page hears `visibilitychange`.
                if state.hidden != minimized {
                    let _ = state.app.feed(&Input::Shown(!minimized));
                }
                state.window_resized(window, minimized, (width, height))
            });
            tell_state_changed(window);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // The system suggests a rectangle for the new DPI, and `WM_SIZE` handles the rest.
            let suggested = lparam.0 as *const RECT;
            // SAFETY: for this message, `lparam` points to a valid rectangle.
            unsafe {
                let rect = *suggested;
                let _ = SetWindowPos(
                    window,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE if dock_dragging(window) => {
            let (x, y) = pointer_position(lparam);
            with_state(window, |state| {
                let client = client_size(window);
                let scale = WindowState::scale(window);
                if let Some(dock) = state.dock.as_mut() {
                    dock.drag_to(client, scale, x, y);
                }
                state.relayout(window);
            });
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            #[cfg(feature = "idle-probe")]
            let _native_cycles = crate::native_cycles::enter(crate::native_cycles::WM_MOVE);
            let (x, y) = pointer_position(lparam);
            with_state(window, |state| {
                // The system says this when something changes under a pointer that hasn't moved, which
                // shouldn't be enough to bring back a GPU that's been let go of.
                if state.pointer_at == Some((x, y)) && state.tracking_pointer {
                    return;
                }
                state.pointer_at = Some((x, y));
                state.track_pointer(window);
                state.need_gpu(window);
                let held = modifiers_held();
                {
                    #[cfg(feature = "idle-probe")]
                    let _native_cycles = crate::native_cycles::enter(crate::native_cycles::MODIFIERS_FEED);
                    let _ = state.app.feed(&Input::Modifiers(held));
                }
                {
                    #[cfg(feature = "idle-probe")]
                    let _native_cycles = crate::native_cycles::enter(crate::native_cycles::MOVE_FEED);
                    let _ = state.app.feed(&Input::Move { x, y });
                }
                if state.app.cursor() != state.shown_cursor {
                    state.show_cursor();
                }
                state.update_frame(window);
                state.note_activity(window);
                state.schedule_wake(window);
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_state(window, |state| {
                // While a press is held, the pointer's still the window's wherever it is, and what was pressed goes on hearing it,
                // as on a page: the system can say it's left as it goes over where the window's dragged by, or out of the window.
                // Letting go of the press asks to be told again (see `let_go_of_pointer`).
                if state.pressing {
                    return;
                }
                state.tracking_pointer = false;
                state.pointer_at = None;
                let _ = state.app.feed(&Input::Leave);
                state.update_and_draw(window);
            });
            LRESULT(0)
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT && dock_cursor(window).is_some() => {
            if let Some(cursor) = dock_cursor(window) {
                // SAFETY: a cursor the system has.
                unsafe {
                    if let Ok(cursor) = LoadCursorW(None, cursor) {
                        SetCursor(Some(cursor));
                    }
                }
            }
            LRESULT(1)
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            with_state(window, |state| state.show_cursor());
            LRESULT(1)
        }
        WM_LBUTTONDOWN if dock_cursor(window).is_some() => {
            with_state(window, |state| state.dock_dragging = true);
            // SAFETY: `window` is valid for a message sent to it.
            unsafe {
                SetCapture(window);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP if dock_dragging(window) => {
            with_state(window, |state| state.dock_dragging = false);
            // SAFETY: this only lets go of the pointer, which is always allowed.
            unsafe {
                let _ = ReleaseCapture();
            }
            LRESULT(0)
        }
        WM_PARENTNOTIFY if (wparam.0 & 0xFFFF) as u32 == WM_DESTROY => {
            // What was docked inside has gone, as its program ended: the app has all the room again.
            let gone = HWND(lparam.0 as *mut c_void);
            with_state(window, |state| {
                if state.dock.as_ref().is_some_and(|dock| dock.child == gone) {
                    state.dock = None;
                    state.relayout(window);
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            take_keyboard_back(window);
            let (x, y) = pointer_position(lparam);
            let clicks = if message == WM_LBUTTONDBLCLK { 2 } else { 1 };
            with_state(window, |state| {
                let _ = state.app.feed(&Input::Modifiers(modifiers_held()));
                let _ = state.app.feed(&Input::Down { x, y, clicks });
                state.update_and_draw(window);
            });
            hold_pointer(window);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = pointer_position(lparam);
            with_state(window, |state| {
                let _ = state.app.feed(&Input::Modifiers(modifiers_held()));
                let _ = state.app.feed(&Input::Up { x, y });
                state.update_and_draw(window);
            });
            let_go_of_pointer(window, wparam);
            LRESULT(0)
        }
        WINDOWS_WANTED => {
            do_window_requests();
            LRESULT(0)
        }
        WM_CLOSE => {
            let mut kept = false;
            with_state(window, |state| {
                kept = state.app.feed(&Input::Close).unwrap_or(false);
                if kept {
                    state.update_and_draw(window);
                }
            });
            if kept {
                LRESULT(0)
            } else {
                destroy_window(window);
                LRESULT(0)
            }
        }
        WM_COMMAND if (wparam.0 >> 16) & 0xFFFF == windows::Win32::UI::Shell::THBN_CLICKED as usize => {
            thumbnail_button_clicked(window, (wparam.0 & 0xFFFF) as usize);
            LRESULT(0)
        }
        WM_COMMAND => {
            crate::desktop::menu_command((wparam.0 & 0xFFFF) as u16);
            LRESULT(0)
        }
        WM_APPCOMMAND => {
            if media_key(lparam) {
                LRESULT(1)
            } else {
                // SAFETY: these are the arguments this was given.
                unsafe { DefWindowProcW(window, message, wparam, lparam) }
            }
        }
        WM_NCCALCSIZE if wparam.0 != 0 && is_frameless(window) => {
            // All of it is inside, with nothing left for a title bar or edges. A maximized window goes past the edges of the
            // screen by as much as its edges would've been, and what's in it mustn't.
            // SAFETY: `window` is the window this is running in, and `lparam` points to this for this message when `wparam` isn't 0.
            unsafe {
                if IsZoomed(window).as_bool() {
                    let inset = frame_thickness(window);
                    let area = &mut (*(lparam.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0];
                    area.left += inset;
                    area.top += inset;
                    area.right -= inset;
                    area.bottom -= inset;
                }
            }
            LRESULT(0)
        }
        // Where a transparent window has nothing drawn, the pointer goes through to what's behind, as a layered window has it.
        WM_NCHITTEST
            if crate::shell_layer::may_pass_through(window) && {
                let (x, y) = pointer_position(lparam);
                !drawn_at_screen(window, POINT { x: x as i32, y: y as i32 })
            } =>
        {
            crate::shell_layer::start_passing_through(window, PASS_THROUGH_TIMER);
            LRESULT(HTTRANSPARENT as isize)
        }
        // A window beneath the rest doesn't take the keyboard from what has it when it's clicked, unless what's clicked takes
        // keys, and it stays beneath them either way (see `shell_layer`).
        WM_MOUSEACTIVATE if crate::shell_layer::layer_of(window) == WindowLayer::Bottom => {
            let mut at = POINT::default();
            let mut keys = false;
            // SAFETY: these write to what's valid.
            unsafe {
                let _ = GetCursorPos(&raw mut at);
                let _ = ScreenToClient(window, &raw mut at);
            }
            with_state(window, |state| keys = state.app.takes_keys_at(at.x as f32, at.y as f32));
            LRESULT(if keys { MA_ACTIVATE } else { MA_NOACTIVATE } as isize)
        }
        WM_NCHITTEST if is_frameless(window) => {
            let (x, y) = pointer_position(lparam);
            let mut at = POINT {
                x: x as i32,
                y: y as i32,
            };
            let mut client = RECT::default();
            // SAFETY: `window` is the window this is running in, and what these write to is valid.
            let sized = unsafe {
                let _ = ScreenToClient(window, &raw mut at);
                let _ = GetClientRect(window, &raw mut client);
                !IsZoomed(window).as_bool()
            };
            let mut hit = HTCLIENT;
            with_state(window, |state| {
                let edges = sized && state.before_fullscreen.is_none();
                // A press is only held while the window has the pointer, so that one whose end was missed can't keep the window
                // from being dragged or resized.
                // SAFETY: this only asks.
                let pressing = state.pressing && unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetCapture() } == window;
                hit = frameless_hit(
                    pressing,
                    || edges.then(|| edge_at(at.x, at.y, client.right, client.bottom, frame_thickness(window))).flatten(),
                    || state.app.drags_window_at(at.x as f32, at.y as f32),
                );
            });
            LRESULT(hit as isize)
        }
        WM_GETMINMAXINFO => {
            let mut own = None;
            let mut most = None;
            let known = with_state(window, |state| {
                own = state.smallest;
                most = state.largest;
            });
            let smallest = if known {
                own
            } else {
                // It's still being made, and asking about this comes before it's known.
                None
            };
            if let (true, Some((width, height))) = (known, most) {
                // SAFETY: as below.
                unsafe {
                    let scale = GetDpiForWindow(window) as f32 / 96.0;
                    let mut outer = RECT { left: 0, top: 0, right: (width as f32 * scale) as i32, bottom: (height as f32 * scale) as i32 };
                    let style = WINDOW_STYLE(GetWindowLongW(window, GWL_STYLE) as u32);
                    let extra = WINDOW_EX_STYLE(GetWindowLongW(window, GWL_EXSTYLE) as u32);
                    let _ = AdjustWindowRectEx(&raw mut outer, style, false, extra);
                    let info = lparam.0 as *mut MINMAXINFO;
                    // A side that isn't limited keeps what the system says.
                    if width > 0 {
                        (*info).ptMaxTrackSize.x = outer.right - outer.left;
                    }
                    if height > 0 {
                        (*info).ptMaxTrackSize.y = outer.bottom - outer.top;
                    }
                }
            }
            if let Some((width, height)) = smallest {
                let (width, height) = (width as i32, height as i32);
                // SAFETY: `window` is the window this is running in, and `lparam` points to this for this message.
                unsafe {
                    let scale = GetDpiForWindow(window) as f32 / 96.0;
                    let mut outer = RECT {
                        left: 0,
                        top: 0,
                        right: (width as f32 * scale) as i32,
                        bottom: (height as f32 * scale) as i32,
                    };
                    let style = WINDOW_STYLE(GetWindowLongW(window, GWL_STYLE) as u32);
                    let extra = WINDOW_EX_STYLE(GetWindowLongW(window, GWL_EXSTYLE) as u32);
                    let _ = AdjustWindowRectEx(&raw mut outer, style, false, extra);
                    let info = lparam.0 as *mut MINMAXINFO;
                    if width > 0 {
                        (*info).ptMinTrackSize.x = outer.right - outer.left;
                    }
                    if height > 0 {
                        (*info).ptMinTrackSize.y = outer.bottom - outer.top;
                    }
                }
            }
            LRESULT(0)
        }
        WM_IME_STARTCOMPOSITION => {
            with_state(window, |state| {
                if let Some((x, y, height)) = state.app.caret_place() {
                    crate::system::place_composition(window, x, y, height);
                }
                let _ = state.app.feed(&Input::Composition {
                    phase: "start".to_owned(),
                    text: String::new(),
                });
                state.update_and_draw(window);
            });
            // SAFETY: this is the message as it came, for what it does anyway.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_IME_COMPOSITION => {
            // What it's come to so far, and what it came to once it's done, which is then typed, as the system does.
            const SO_FAR: u32 = 0x0008;
            const CAME_TO: u32 = 0x0800;
            let flags = lparam.0 as u32;
            with_state(window, |state| {
                if flags & CAME_TO != 0 {
                    state.composed = crate::system::composition_text(window, CAME_TO);
                }
                if flags & SO_FAR != 0 {
                    if let Some(text) = crate::system::composition_text(window, SO_FAR) {
                        let _ = state.app.feed(&Input::Composition {
                            phase: "update".to_owned(),
                            text,
                        });
                        state.update_and_draw(window);
                    }
                }
            });
            // SAFETY: as above.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_IME_ENDCOMPOSITION => {
            with_state(window, |state| {
                let came_to = state.composed.take().unwrap_or_default();
                let _ = state.app.feed(&Input::Composition {
                    phase: "end".to_owned(),
                    text: came_to,
                });
                state.update_and_draw(window);
            });
            // SAFETY: as above.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_DROPFILES => {
            // SAFETY: this is what came with the message, and nothing else uses it.
            let (paths, x, y) = unsafe { crate::system::take_dropped_files(wparam.0 as *mut _) };
            with_state(window, |state| {
                state.need_gpu(window);
                let _ = state.app.feed(&Input::Files { x, y, paths });
                state.update_and_draw(window);
            });
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            // Only the colors are of interest, among everything this is sent for.
            const COLORS: &[u16] = &[
                73, 109, 109, 101, 114, 115, 105, 118, 101, 67, 111, 108, 111, 114, 83, 101, 116, 0,
            ];
            // SAFETY: if there's a pointer here, it's to a string that ends in a terminator, and this
            // stops at the first character that isn't the same, which that would be at the latest.
            let about_colors = lparam.0 != 0
                && COLORS
                    .iter()
                    .enumerate()
                    .all(|(at, wanted)| unsafe { *(lparam.0 as *const u16).add(at) == *wanted });
            if about_colors {
                with_state(window, |state| {
                    let _ = state
                        .app
                        .feed(&Input::DarkMode(crate::system::system_is_dark()));
                    state.update_and_draw(window);
                });
            }
            LRESULT(0)
        }
        WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
            take_keyboard_back(window);
            let (x, y) = pointer_position(lparam);
            let button = other_button(message, wparam);
            with_state(window, |state| {
                state.need_gpu(window);
                let _ = state.app.feed(&Input::Modifiers(modifiers_held()));
                let _ = state.app.feed(&Input::ButtonDown { x, y, button });
                state.update_and_draw(window);
            });
            hold_pointer(window);
            // Back and forward say they were dealt with by giving back true.
            LRESULT(isize::from(message == WM_XBUTTONDOWN))
        }
        WM_RBUTTONUP | WM_MBUTTONUP | WM_XBUTTONUP => {
            let (x, y) = pointer_position(lparam);
            let button = other_button(message, wparam);
            with_state(window, |state| {
                state.need_gpu(window);
                let _ = state.app.feed(&Input::Modifiers(modifiers_held()));
                let _ = state.app.feed(&Input::ButtonUp { x, y, button });
                state.update_and_draw(window);
            });
            let_go_of_pointer(window, wparam);
            LRESULT(isize::from(message == WM_XBUTTONUP))
        }
        WM_MOVE => {
            with_state(window, |state| state.tell_screen_origin(window));
            tell_state_changed(window);
            if crate::embed::any() {
                crate::embed::window_moved(window.0 as isize);
            }
            LRESULT(0)
        }
        WM_ACTIVATE | WM_SHOWWINDOW => {
            tell_state_changed(window);
            // SAFETY: these are the arguments this was given.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_CAPTURECHANGED => {
            // Something else took the pointer while a press was held, as a resize from an edge (`startResize`) or another window
            // coming to the front does, so the press is called off. Letting go of it after the buttons came up says nothing new.
            with_state(window, |state| {
                if std::mem::take(&mut state.pressing) {
                    state.tracking_pointer = false;
                    let _ = state.app.feed(&Input::CancelPress);
                    state.update_and_draw(window);
                }
            });
            LRESULT(0)
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            with_state(window, |state| {
                if message == WM_SETFOCUS {
                    state.need_gpu(window);
                    // What a file dialog belongs to is what's in front when it's asked for.
                    crate::dialogs::set_current_window(window);
                }
                let _ = state.app.feed(&Input::Active(message == WM_SETFOCUS));
                state.update_and_draw(window);
                if message == WM_SETFOCUS {
                    // It's in front now, so what's moving doesn't have to wait to carry on.
                    state.covered = None;
                    // SAFETY: `window` is valid for a message sent to it.
                    unsafe {
                        let _ = KillTimer(Some(window), COVERED_TIMER);
                    }
                    state.ask_for_next_frame(window);
                }
            });
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let system = message == WM_SYSKEYDOWN;
            with_state(window, |state| {
                state.need_gpu(window);
                if let Some(press) = key_press(wparam, lparam, system) {
                    let _ = state.app.feed(&Input::Modifiers(press.modifiers()));
                    let _ = state.app.feed(&Input::Key(press));
                    state.update_and_draw(window);
                }
            });
            if system {
                // What the system does with alt and F10 still happens, as alt+F4 closing the window does.
                // SAFETY: these are the arguments this was given.
                unsafe { DefWindowProcW(window, message, wparam, lparam) }
            } else {
                LRESULT(0)
            }
        }
        WM_KEYUP | WM_SYSKEYUP => {
            let system = message == WM_SYSKEYUP;
            with_state(window, |state| {
                if let Some(press) = key_press(wparam, lparam, system) {
                    let press = KeyPress {
                        repeat: false,
                        ..press
                    };
                    let _ = state.app.feed(&Input::Modifiers(modifiers_held()));
                    let _ = state.app.feed(&Input::KeyUp(press));
                    state.update_and_draw(window);
                }
            });
            if system {
                // SAFETY: these are the arguments this was given.
                unsafe { DefWindowProcW(window, message, wparam, lparam) }
            } else {
                LRESULT(0)
            }
        }
        WM_CHAR => {
            let unit = wparam.0 as u16;
            with_state(window, |state| {
                let mut units = [0u16; 2];
                let count = match (state.high_surrogate.take(), unit) {
                    (_, 0xD800..=0xDBFF) => {
                        state.high_surrogate = Some(unit);
                        return;
                    }
                    (Some(high), _) => {
                        units = [high, unit];
                        2
                    }
                    (None, _) => {
                        units[0] = unit;
                        1
                    }
                };
                let text = String::from_utf16_lossy(&units[..count]);
                let _ = state.app.feed(&Input::Text(text));
                state.update_and_draw(window);
            });
            LRESULT(0)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let turned_at = message_instant();
            // Deliver this report only. A peek/remove coalescer is not atomic and
            // can overtake or remove an intervening command. The message loop batches
            // paint opportunities instead, preserving every input in native order.
            let turned = wheel_delta(wparam);
            // Fractional reports remain direct input; ordinary notches keep their glide.
            let precise = precise_wheel_delta(wheel_delta(wparam));
            // Shift held turns the wheel into one that goes across.
            let horizontal = message == WM_MOUSEHWHEEL;
            let across = horizontal || wparam.0 & MOUSE_WHEEL_SHIFT != 0;
            // The wheel reports its position from the screen's corner, not the window's.
            let (mut x, mut y) = pointer_position(lparam);
            let mut point = windows::Win32::Foundation::POINT {
                x: x as i32,
                y: y as i32,
            };
            // SAFETY: `point` is a valid point to convert.
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(window, &mut point);
            }
            x = point.x as f32;
            y = point.y as f32;
            with_state(window, |state| {
                if turned != 0 {
                    state.app.set_input_time(turned_at);
                    let _ = state.app.feed(&Input::ScrollPixels {
                        x,
                        y,
                        pixels: wheel_pixels(turned, horizontal),
                        across,
                        precise,
                    });
                    state.update_and_draw(window);
                }
            });
            LRESULT(0)
        }
        WAKE => {
            with_state(window, |state| {
                state.update_frame(window);
                state.note_activity(window);
                state.blink(window, false);
                state.schedule_wake(window);
            });
            LRESULT(0)
        }
        WM_GETOBJECT if lparam.0 as i32 == UiaRootObjectId => {
            let mut provider = None;
            with_state(window, |state| provider = state.access_root(window));
            provider
                .and_then(|provider| uia::return_provider(window, wparam, lparam, &provider))
                // SAFETY: These are the arguments this was given.
                .unwrap_or_else(|| unsafe { DefWindowProcW(window, message, wparam, lparam) })
        }
        uia::ACCESS_COMMAND => {
            with_state(window, |state| state.run_access_commands(window));
            LRESULT(0)
        }
        RENDERER_READY => {
            with_state(window, |state| state.renderer_ready(window));
            LRESULT(0)
        }
        #[cfg(feature = "idle-probe")]
        WM_TIMER if wparam.0 == idle_probe::TIMER => {
            idle_probe::tick(window);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == PASS_THROUGH_TIMER => {
            let mut at = POINT::default();
            let mut area = RECT::default();
            // SAFETY: these write to what's valid.
            let inside = unsafe {
                let _ = GetCursorPos(&raw mut at);
                let _ = GetWindowRect(window, &raw mut area);
                at.x >= area.left && at.x < area.right && at.y >= area.top && at.y < area.bottom
            };
            // Away from the window, it can hear the pointer again, as it'll be asked about it before it's over it.
            if !inside || !crate::shell_layer::is_passing_through(window) || drawn_at_screen(window, at) {
                crate::shell_layer::stop_passing_through(window);
                // SAFETY: `window` is valid for a message sent to it.
                unsafe {
                    let _ = KillTimer(Some(window), PASS_THROUGH_TIMER);
                }
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == IDLE_TIMER => {
            with_state(window, |state| state.idle_timer(window));
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == COVERED_TIMER => {
            // SAFETY: `window` is valid for a message sent to it.
            unsafe {
                let _ = KillTimer(Some(window), COVERED_TIMER);
                let _ = InvalidateRect(Some(window), None, false);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == RELEASE_TIMER => {
            with_state(window, |state| state.release_timer(window));
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == STARTUP_TRIM_TIMER => {
            // SAFETY: `window` is valid for a message sent to it.
            unsafe {
                let _ = KillTimer(Some(window), STARTUP_TRIM_TIMER);
            }
            with_state(window, |state| {
                // If things went quiet, this has already happened.
                if state.quiet == QUIET_AFTER_FIRST_FRAME {
                    state.quiet = QUIET;
                    if let Some(renderer) = &mut state.renderer {
                        renderer.trim();
                    }
                }
            });
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == FRAME_QUIET_TIMER => {
            // SAFETY: `window` is valid for a message sent to it.
            unsafe {
                let _ = KillTimer(Some(window), FRAME_QUIET_TIMER);
            }
            if !input_waiting() {
                let logged = crate::input_log::now();
                crate::app::run_quiet_hook(crate::app::Quiet::Frame);
                crate::input_log::note("frame-quiet", logged, format_args!(""));
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == QUIET_WORK_TIMER => {
            with_state(window, |state| state.quiet_work_timer(window));
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == WAKE_TIMER => {
            // SAFETY: `window` is valid for a message sent to it.
            unsafe {
                let _ = KillTimer(Some(window), WAKE_TIMER);
            }
            with_state(window, |state| {
                state.wake_at = None;
                state.app.wake();
                state.update_frame(window);
                state.schedule_wake(window);
            });
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == CARET_TIMER => {
            with_state(window, |state| {
                state.app.blink();
                if !state.app.caret_blinking() {
                    // SAFETY: `window` is valid for a message sent to it.
                    unsafe {
                        let _ = KillTimer(Some(window), CARET_TIMER);
                    }
                }
                state.update_frame(window);
            });
            LRESULT(0)
        }
        WM_DESTROY => {
            let mut listening = false;
            with_state(window, |state| {
                listening = state.drop_target;
                // What's docked inside is another program's, and goes back to being a window of its own.
                state.undock();
                for icon in state.thumbnail_icons.drain(..).filter(|icon| !icon.is_invalid()) {
                    // SAFETY: each was made for its buttons, which go with the window.
                    unsafe {
                        let _ = DestroyIcon(icon);
                    }
                }
            });
            if listening {
                crate::drop::stop_listening(window);
            }
            // What was kept inside it has gone with it.
            crate::embed::window_gone(window.0 as isize);
            window_destroyed(window);
            LRESULT(0)
        }
        _ if message == taskbar_button_created() => {
            // The taskbar has its button now, or again after Explorer started over, which forgot the buttons under its picture.
            with_state(window, |state| {
                state.taskbar_button = true;
                state.thumbnail_buttons_added = false;
            });
            put_thumbnail_buttons(window);
            LRESULT(0)
        }
        // SAFETY: these are the arguments this was given.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// A window handle that can be sent to another thread, for posting messages to and making a
/// swap chain for. Both are safe from any thread, and fail harmlessly if the window has closed.
#[derive(Clone, Copy)]
struct WindowHandle(*mut c_void);

// SAFETY: see above.
unsafe impl Send for WindowHandle {}
// SAFETY: as above.
unsafe impl Sync for WindowHandle {}

thread_local! {
    /// How small the main window can be made, as that's said before it's opened.
    static SMALLEST_MAIN: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
    /// Whether the main window has no frame, stays on top, and covers the screen, as that's said before it's opened too.
    static MAIN_KIND: Cell<(bool, bool, bool)> = const { Cell::new((false, false, false)) };
    /// Whether the main window's been opened, after which what kind it is can only be changed on it.
    static MAIN_OPENED: Cell<bool> = const { Cell::new(false) };
    /// The rest of how the main window opens, as that's said before it's opened too: its layer, whether it's transparent,
    /// left off the taskbar or on every desktop, and its app bar.
    static MAIN_MORE: RefCell<WindowOptions> = RefCell::new(WindowOptions::default());
}

/// Says the main window's been opened, or that what's running has no window to open, so that what's asked of it is done to it.
pub(crate) fn main_opened() {
    MAIN_OPENED.with(|opened| opened.set(true));
}

/// Makes a change to what kind of window the main one is, if it hasn't been opened yet, so that it opens that way rather
/// than changing once it's shown, and says whether it did.
pub(crate) fn change_main_kind(change: &crate::WindowChange) -> bool {
    if MAIN_OPENED.with(Cell::get) {
        return false;
    }
    let (mut frameless, mut on_top, mut fullscreen) = MAIN_KIND.with(Cell::get);
    match *change {
        crate::WindowChange::Frameless(on) => frameless = on,
        crate::WindowChange::OnTop(on) => {
            // As it would be once it's open: taking it off the top leaves another layer as it is.
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

/// Says what kind of window the main one is. Set this before opening it, on the same thread. See
/// [`WindowOptions`](crate::WindowOptions) for what these mean.
pub fn set_main_window_kind(frameless: bool, on_top: bool, fullscreen: bool) {
    MAIN_KIND.with(|kind| kind.set((frameless, on_top, fullscreen)));
}

/// How the main window opens, for what [`set_main_window_kind`] doesn't say: its layer, whether it's transparent, left off the
/// taskbar or on every desktop, and its app bar. Set this before opening it, on the same thread; the rest of `options` is
/// left out. Asking for these with [`change_window`](crate::change_window) before it's opened does the same.
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

/// How the main window will open, as far as what's been said so far, for checking what's been asked of it.
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

/// Says how small the main window can be made, inside its edges, in logical pixels. Set this before opening it, on
/// the same thread. See [`WindowOptions`](crate::WindowOptions) for the others.
pub fn set_smallest_window_size(width: u32, height: u32) {
    SMALLEST_MAIN.with(|smallest| smallest.set(Some((width, height))));
}

/// How wide a window's edges are, for taking hold of them.
fn frame_thickness(window: HWND) -> i32 {
    // SAFETY: `window` is the window this is running in, and these only ask.
    unsafe {
        let dpi = GetDpiForWindow(window);
        GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
    }
}

/// What a point in a window with no frame is to the system (`WM_NCHITTEST`): an edge it's resized by, where it's dragged by, or
/// the window's own area. While a press in its own area is held, all of it is its own area. The system asks this of the window that
/// holds the pointer too, and an answer of anything else there has it say the pointer left (`WM_MOUSELEAVE`) and can have its moves
/// come as moves over a frame (`WM_NCMOUSEMOVE`), which what was pressed doesn't hear: a tab dragged off itself onto the strip the
/// window's dragged by stopped following the pointer.
fn frameless_hit(pressing: bool, edge: impl FnOnce() -> Option<u32>, drags: impl FnOnce() -> bool) -> u32 {
    if pressing {
        return HTCLIENT;
    }
    edge().unwrap_or_else(|| if drags() { HTCAPTION } else { HTCLIENT })
}

/// Whether any mouse button is still down, as what comes with a button's message says (`MK_LBUTTON`, `MK_RBUTTON`, `MK_MBUTTON`,
/// `MK_XBUTTON1` and `MK_XBUTTON2`), which for a button coming up leaves that one out.
fn buttons_held(wparam: WPARAM) -> bool {
    const BUTTONS: usize = 0x0001 | 0x0002 | 0x0010 | 0x0020 | 0x0040;
    wparam.0 & BUTTONS != 0
}

/// Holds the pointer for a press in the window's own area, so that what was pressed hears where the pointer goes and the button
/// coming up, over where the window's dragged by, its edges, or outside it, until no button's down (Pointer Events 3, 9.5).
fn hold_pointer(window: HWND) {
    with_state(window, |state| state.pressing = true);
    // SAFETY: `window` is valid for a message sent to it.
    unsafe {
        SetCapture(window);
    }
}

/// Lets go of the pointer once no button's down, which isn't the press being called off, and asks to be told when the pointer
/// leaves again, which the system then says straight away if it's left already.
fn let_go_of_pointer(window: HWND, wparam: WPARAM) {
    if buttons_held(wparam) {
        return;
    }
    let mut held = false;
    with_state(window, |state| held = std::mem::take(&mut state.pressing));
    if held {
        // SAFETY: this only lets go of the pointer, which is always allowed.
        unsafe {
            let _ = ReleaseCapture();
        }
    }
    with_state(window, |state| {
        state.tracking_pointer = false;
        state.track_pointer(window);
    });
}

/// Which edge or corner of a window this is on, with how wide they are, if it's on one.
fn edge_at(x: i32, y: i32, width: i32, height: i32, thickness: i32) -> Option<u32> {
    let (left, right) = (x < thickness, x >= width - thickness);
    let (top, bottom) = (y < thickness, y >= height - thickness);
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(HTTOPLEFT),
        (_, true, true, _) => Some(HTTOPRIGHT),
        (true, _, _, true) => Some(HTBOTTOMLEFT),
        (_, true, _, true) => Some(HTBOTTOMRIGHT),
        (true, ..) => Some(HTLEFT),
        (_, true, ..) => Some(HTRIGHT),
        (_, _, true, _) => Some(HTTOP),
        (_, _, _, true) => Some(HTBOTTOM),
        _ => None,
    }
}

/// Whether a window has been told to have no frame. This is kept on the window, as it's asked about while it's being made.
fn is_frameless(window: HWND) -> bool {
    // SAFETY: this only asks, and a window that's gone says no.
    unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) != 0 }
}

const QUIETLY: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(
    SWP_NOMOVE.0 | SWP_NOSIZE.0 | SWP_NOZORDER.0 | SWP_NOACTIVATE.0 | SWP_NOOWNERZORDER.0,
);

/// The screen a window's on, and the part of it that's left once the taskbar's taken its place.
fn screen_of(window: HWND) -> (RECT, RECT) {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is the size it says, and asking about a window that's gone leaves it empty.
    unsafe {
        let _ = GetMonitorInfoW(
            MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST),
            &raw mut info,
        );
    }
    (info.rcMonitor, info.rcWork)
}

/// What's told when a window's been moved, resized, minimized, shown, hidden, or given or lost the keyboard.
type StateHeard = std::rc::Rc<dyn Fn(WindowId)>;

thread_local! {
    static STATE_HEARD: RefCell<Option<StateHeard>> = const { RefCell::new(None) };
}

/// Runs this, with the window's id, each time how a window is changes: where it is, how big, whether it's shown, minimized,
/// maximized or fullscreen, and whether it has the keyboard. See [`window_state`].
pub fn on_window_state_change(hear: impl Fn(WindowId) + 'static) {
    STATE_HEARD.with(|heard| *heard.borrow_mut() = Some(std::rc::Rc::new(hear)));
}

fn tell_state_changed(window: HWND) {
    let id = WINDOWS.with(|windows| windows.borrow().iter().find(|open| open.window == window).map(|open| open.id));
    let hear = STATE_HEARD.with(|heard| heard.borrow().clone());
    if let (Some(id), Some(hear)) = (id, hear) {
        hear(id);
    }
}

/// The system's handle for a window, as a number, for what needs one.
pub fn window_handle(id: WindowId) -> Option<isize> {
    window_with_id(id).map(|window| window.0 as isize)
}

/// How a window is now, if it's open.
pub fn window_state(id: WindowId) -> Option<crate::WindowStateNow> {
    let window = window_with_id(id)?;
    let mut fullscreen = false;
    with_state(window, |state| fullscreen = state.before_fullscreen.is_some());
    // SAFETY: the window's on this thread, and asking about it only reads.
    unsafe {
        let scale = WindowState::scale(window);
        let mut outer = RECT::default();
        let _ = GetWindowRect(window, &raw mut outer);
        let mut inner = RECT::default();
        let _ = GetClientRect(window, &raw mut inner);
        let length = GetWindowTextLengthW(window).max(0) as usize;
        let mut text = vec![0u16; length + 1];
        let got = GetWindowTextW(window, &mut text).max(0) as usize;
        Some(crate::WindowStateNow {
            title: String::from_utf16_lossy(&text[..got]),
            x: (outer.left as f32 / scale).round() as i32,
            y: (outer.top as f32 / scale).round() as i32,
            width: (inner.right as f32 / scale).round() as u32,
            height: (inner.bottom as f32 / scale).round() as u32,
            visible: IsWindowVisible(window).as_bool(),
            minimized: IsIconic(window).as_bool(),
            maximized: IsZoomed(window).as_bool(),
            fullscreen,
            focused: GetForegroundWindow() == window,
        })
    }
}

/// A picture of what's in a window, as it's drawn now: its size in physical pixels, and its pixels, four bytes each, red first.
/// It's drawn again for this, off the screen, so what's over the window doesn't come into it.
pub fn capture_window(id: WindowId) -> Result<(u32, u32, Vec<u8>), String> {
    let window = window_with_id(id).ok_or("there's no such window")?;
    let mut out = Err("the window couldn't be drawn".to_owned());
    with_state(window, |state| {
        let size = state.app.size();
        // A new renderer has none of what the painter's given the window's, as its glyphs, and the window's has to be
        // given it all again after. Without this, text the window had drawn was missing from the picture.
        state.app.renderer_replaced();
        out = Renderer::offscreen(size)
            .map_err(|error| error.to_string())
            .and_then(|mut renderer| {
                state.app.render(&mut renderer).map_err(|error| error.to_string())?;
                renderer.read_pixels().map_err(|error| error.to_string())
            })
            .map(|pixels| (size.0, size.1, pixels));
        state.app.renderer_replaced();
    });
    out
}

/// What the system says once the taskbar has a window's button, which the buttons under its picture have to wait for.
fn taskbar_button_created() -> u32 {
    static NUMBER: OnceLock<u32> = OnceLock::new();
    // SAFETY: the name ends in a terminator.
    *NUMBER.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) })
}

/// The most buttons the taskbar has room for under a window's picture.
const THUMBNAIL_BUTTONS: usize = 7;

/// Puts the buttons under a window's picture in the taskbar, once it has its button. All seven go on the first time, with
/// those that aren't wanted hidden, as the taskbar only lets them be added once and changed after that.
fn put_thumbnail_buttons(window: HWND) {
    use windows::Win32::UI::Shell::{THB_FLAGS, THB_ICON, THB_TOOLTIP, THBF_DISABLED, THBF_ENABLED, THBF_HIDDEN, THUMBBUTTON};
    let mut wanted = None;
    with_state(window, |state| {
        if state.taskbar_button && (state.thumbnail_buttons_added || !state.thumbnail_buttons.is_empty()) {
            wanted = Some((state.thumbnail_buttons.clone(), state.thumbnail_buttons_added));
        }
    });
    let Some((wanted, added)) = wanted else {
        return;
    };
    let Some(list) = taskbar() else {
        return;
    };
    let icons: Vec<HICON> = wanted.iter().map(|one| crate::desktop::icon_from_bytes(&one.icon).unwrap_or_default()).collect();
    let mut buttons = [THUMBBUTTON::default(); THUMBNAIL_BUTTONS];
    for (slot, button) in buttons.iter_mut().enumerate() {
        button.iId = slot as u32;
        button.dwMask = THB_FLAGS | THB_ICON | THB_TOOLTIP;
        match wanted.get(slot) {
            Some(one) => {
                button.hIcon = icons[slot];
                button.dwFlags = if one.disabled { THBF_DISABLED } else { THBF_ENABLED };
                for (to, from) in button.szTip.iter_mut().zip(one.tooltip.encode_utf16().take(259)) {
                    *to = from;
                }
            }
            None => button.dwFlags = THBF_HIDDEN,
        }
    }
    // SAFETY: the window's on this thread, and the buttons last through the call.
    let put = unsafe {
        if added {
            list.ThumbBarUpdateButtons(window, &buttons)
        } else {
            list.ThumbBarAddButtons(window, &buttons)
        }
    };
    let mut before = Vec::new();
    with_state(window, |state| {
        state.thumbnail_buttons_added |= put.is_ok();
        before = std::mem::replace(&mut state.thumbnail_icons, icons);
    });
    // The icons that were shown before these are let go of, now these are in their place.
    for icon in before.into_iter().filter(|icon| !icon.is_invalid()) {
        // SAFETY: each was made for these buttons, and nothing else has it.
        unsafe {
            let _ = DestroyIcon(icon);
        }
    }
}

type ThumbnailHeard = Rc<dyn Fn(WindowId, u32)>;
type MediaKeyHeard = Rc<dyn Fn(&'static str)>;
type CaptureHeard = Rc<dyn Fn(u32, Result<(u32, u32, Vec<u8>), String>)>;

thread_local! {
    static THUMBNAIL_HEARD: RefCell<Option<ThumbnailHeard>> = const { RefCell::new(None) };
    static MEDIA_KEY_HEARD: RefCell<Option<MediaKeyHeard>> = const { RefCell::new(None) };
    static CAPTURE_HEARD: RefCell<Option<CaptureHeard>> = const { RefCell::new(None) };
}

/// Runs this with the number a [`WindowChange::Capture`](crate::WindowChange::Capture) was asked for with, and the picture
/// of what's in the window, as [`capture_window`] gives it, or why there isn't one.
pub fn on_window_captured(hear: impl Fn(u32, Result<(u32, u32, Vec<u8>), String>) + 'static) {
    CAPTURE_HEARD.with(|heard| *heard.borrow_mut() = Some(Rc::new(hear)));
}

/// Runs this, with the window's id and the button's, when one of the buttons under a window's picture in the taskbar is
/// clicked. See [`WindowChange::ThumbnailButtons`](crate::WindowChange::ThumbnailButtons).
pub fn on_thumbnail_button(hear: impl Fn(WindowId, u32) + 'static) {
    THUMBNAIL_HEARD.with(|heard| *heard.borrow_mut() = Some(Rc::new(hear)));
}

fn thumbnail_button_clicked(window: HWND, slot: usize) {
    let mut found = None;
    with_state(window, |state| found = state.thumbnail_buttons.get(slot).map(|button| (state.id, button.id)));
    let hear = THUMBNAIL_HEARD.with(|heard| heard.borrow().clone());
    if let (Some((id, button)), Some(hear)) = (found, hear) {
        hear(id, button);
    }
}

/// Runs this with `play`, `pause`, `playPause`, `next`, `previous` or `stop` when one of those keys is pressed while one of
/// this program's windows has the keyboard. Nothing's taken from other programs, which still get them when they have it.
pub fn on_media_key(hear: impl Fn(&'static str) + 'static) {
    MEDIA_KEY_HEARD.with(|heard| *heard.borrow_mut() = Some(Rc::new(hear)));
}

/// Tells what's listening about a media key, from what `WM_APPCOMMAND` says, and whether anything was.
fn media_key(lparam: LPARAM) -> bool {
    // The command is the top word, without the bits that say where it came from.
    let command = ((lparam.0 as usize >> 16) & 0x0FFF) as u32;
    let key = match command {
        11 => "next",
        12 => "previous",
        13 => "stop",
        14 => "playPause",
        46 => "play",
        47 => "pause",
        _ => return false,
    };
    let hear = MEDIA_KEY_HEARD.with(|heard| heard.borrow().clone());
    match hear {
        Some(hear) => {
            hear(key);
            true
        }
        None => false,
    }
}

/// The taskbar's buttons, as the system has them, made the first time they're wanted.
fn taskbar() -> Option<windows::Win32::UI::Shell::ITaskbarList3> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::UI::Shell::{ITaskbarList3, TaskbarList};
    thread_local! {
        static TASKBAR: RefCell<Option<ITaskbarList3>> = const { RefCell::new(None) };
    }
    TASKBAR.with(|kept| {
        if kept.borrow().is_none() {
            // SAFETY: COM's started for this thread, which it may already be, and the list is made as the system says.
            let made = unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER).ok().filter(|list| list.HrInit().is_ok())
            };
            *kept.borrow_mut() = made;
        }
        kept.borrow().clone()
    })
}

fn set_ex_style(window: HWND, bits: WINDOW_EX_STYLE, on: bool) {
    // SAFETY: the window's on this thread.
    unsafe {
        let now = GetWindowLongPtrW(window, GWL_EXSTYLE);
        let next = if on { now | bits.0 as isize } else { now & !(bits.0 as isize) };
        SetWindowLongPtrW(window, GWL_EXSTYLE, next);
    }
}

fn apply_window_change(window: HWND, change: WindowChange) {
    // Nothing here happens inside `with_state`, as these all send messages back that need it.
    // SAFETY: the window's on this thread, what's given lasts through each call, and a window that's gone only gives errors.
    unsafe {
        match change {
            WindowChange::Frameless(on) => {
                SetWindowLongPtrW(window, GWLP_USERDATA, isize::from(on));
                let _ = SetWindowPos(window, None, 0, 0, 0, 0, QUIETLY | SWP_FRAMECHANGED);
                let mut fullscreen = false;
                with_state(window, |state| {
                    fullscreen = state.before_fullscreen.is_some();
                });
                crate::system::set_frameless_look(window, on, fullscreen, is_see_through(window));
            }
            WindowChange::OnTop(on) => {
                let layer = crate::shell_layer::layer_of(window).after_on_top(on);
                crate::shell_layer::set_layer(window, layer);
            }
            WindowChange::Layer(layer) => crate::shell_layer::set_layer(window, layer),
            WindowChange::Transparent(on) => {
                crate::shell_layer::set_transparent(window, on);
                let (mut fullscreen, mut see_through) = (false, on);
                with_state(window, |state| {
                    fullscreen = state.before_fullscreen.is_some();
                    state.transparent = on;
                    see_through = on || state.backdrop;
                    state.set_see_through(window, see_through);
                });
                crate::system::set_frameless_look(window, is_frameless(window), fullscreen, see_through);
                // With nothing of the system's under it, all there is is what's drawn, without even a shadow round it.
                if has_no_surface(window) {
                    crate::system::set_bare_look(window, on);
                }
            }
            WindowChange::AppBar(bar) => crate::shell_layer::set_app_bar(window, bar),
            WindowChange::AllDesktops(on) => crate::shell_layer::set_tool_window(window, None, Some(on)),
            WindowChange::Fullscreen(on) => set_fullscreen(window, on),
            WindowChange::Minimize => {
                let _ = ShowWindow(window, SW_MINIMIZE);
            }
            WindowChange::Maximize => {
                let _ = ShowWindow(window, SW_MAXIMIZE);
            }
            WindowChange::Restore => {
                let _ = ShowWindow(window, SW_RESTORE);
            }
            WindowChange::Hide => {
                let _ = ShowWindow(window, SW_HIDE);
            }
            WindowChange::Show => {
                let show = if this_program_is_in_front() {
                    SW_SHOW
                } else {
                    SW_SHOWNOACTIVATE
                };
                #[cfg(feature = "idle-probe")]
                if idle_probe::active() {
                    let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
                    SetWindowLongPtrW(window, GWL_EXSTYLE, style | WS_EX_NOACTIVATE.0 as isize);
                }
                let _ = ShowWindow(window, show);
            }
            WindowChange::Focus => {
                let _ = SetForegroundWindow(window);
            }
            WindowChange::MoveTo(x, y) => {
                let scale = WindowState::scale(window);
                let (x, y) = crate::shell_layer::from_screen(window, (x as f32 * scale) as i32, (y as f32 * scale) as i32);
                let _ = SetWindowPos(
                    window,
                    None,
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            WindowChange::Resize(width, height) => {
                let (width, height) = outer_size(
                    window,
                    (width, height),
                    WindowState::scale(window),
                    is_frameless(window),
                );
                let _ = SetWindowPos(
                    window,
                    None,
                    0,
                    0,
                    width,
                    height,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            WindowChange::Opacity(how) => {
                set_ex_style(window, WS_EX_LAYERED, true);
                let alpha = (how.clamp(0.0, 1.0) * 255.0).round() as u8;
                let _ = SetLayeredWindowAttributes(window, windows::Win32::Foundation::COLORREF(0), alpha, LWA_ALPHA);
            }
            WindowChange::Resizable(on) => {
                let now = GetWindowLongPtrW(window, GWL_STYLE);
                let bits = (WS_THICKFRAME.0 | WS_MAXIMIZEBOX.0) as isize;
                SetWindowLongPtrW(window, GWL_STYLE, if on { now | bits } else { now & !bits });
                let _ = SetWindowPos(window, None, 0, 0, 0, 0, QUIETLY | SWP_FRAMECHANGED);
            }
            WindowChange::SizeLimits(smallest, largest) => {
                with_state(window, |state| {
                    state.smallest = smallest;
                    state.largest = largest;
                });
                // What it is now is brought within them.
                let _ = SetWindowPos(window, None, 0, 0, 0, 0, QUIETLY | SWP_FRAMECHANGED);
            }
            WindowChange::SkipTaskbar(on) => crate::shell_layer::set_tool_window(window, Some(on), None),
            WindowChange::ClickThrough(on) => {
                crate::shell_layer::set_click_through(window, on);
                set_ex_style(window, WS_EX_LAYERED, true);
                set_ex_style(window, WS_EX_TRANSPARENT, on);
                if on {
                    let _ = SetLayeredWindowAttributes(window, windows::Win32::Foundation::COLORREF(0), 255, LWA_ALPHA);
                }
            }
            WindowChange::ContentProtection(on) => {
                const WDA_EXCLUDEFROMCAPTURE: u32 = 0x11;
                let _ = SetWindowDisplayAffinity(window, WINDOW_DISPLAY_AFFINITY(if on { WDA_EXCLUDEFROMCAPTURE } else { 0 }));
            }
            WindowChange::ShowWithoutActivation => {
                let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
            }
            WindowChange::StartResize(edge) => {
                let hit = match edge.as_str() {
                    "left" => HTLEFT,
                    "right" => HTRIGHT,
                    "top" => HTTOP,
                    "bottom" => HTBOTTOM,
                    "top-left" => HTTOPLEFT,
                    "top-right" => HTTOPRIGHT,
                    "bottom-left" => HTBOTTOMLEFT,
                    _ => HTBOTTOMRIGHT,
                };
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
                let _ = SendMessageW(window, WM_NCLBUTTONDOWN, Some(WPARAM(hit as usize)), Some(LPARAM(0)));
            }
            WindowChange::Icon(bytes) => {
                if let Some(icon) = crate::desktop::icon_from_bytes(&bytes) {
                    for which in [ICON_BIG, ICON_SMALL] {
                        let _ = SendMessageW(window, WM_SETICON, Some(WPARAM(which as usize)), Some(LPARAM(icon.0 as isize)));
                    }
                }
            }
            WindowChange::Progress(done, how) => {
                use windows::Win32::UI::Shell::{TBPF_ERROR, TBPF_INDETERMINATE, TBPF_NOPROGRESS, TBPF_NORMAL, TBPF_PAUSED};
                if let Some(list) = taskbar() {
                    let state = match how.as_str() {
                        "none" => TBPF_NOPROGRESS,
                        "paused" => TBPF_PAUSED,
                        "error" => TBPF_ERROR,
                        "indeterminate" => TBPF_INDETERMINATE,
                        _ => TBPF_NORMAL,
                    };
                    let _ = list.SetProgressState(window, state);
                    if state != TBPF_NOPROGRESS && state != TBPF_INDETERMINATE {
                        let _ = list.SetProgressValue(window, (done.clamp(0.0, 1.0) * 10_000.0) as u64, 10_000);
                    }
                }
            }
            WindowChange::Overlay(bytes, words) => {
                if let Some(list) = taskbar() {
                    let icon = bytes.as_deref().and_then(crate::desktop::icon_from_bytes).unwrap_or_default();
                    let _ = list.SetOverlayIcon(window, icon, &HSTRING::from(words.as_str()));
                }
            }
            WindowChange::ThumbnailButtons(buttons) => {
                with_state(window, |state| state.thumbnail_buttons = buttons);
                put_thumbnail_buttons(window);
            }
            WindowChange::Backdrop(kind) => {
                // The system only draws a backdrop under what's seen through, so with one the window's frame goes all
                // the way in, and its swap chain has alpha. Without one, or on a Windows that doesn't have them, it's
                // as it was.
                let on = crate::system::set_backdrop(window, &kind) && kind != "none";
                let (mut fullscreen, mut see_through) = (false, on);
                with_state(window, |state| {
                    fullscreen = state.before_fullscreen.is_some();
                    state.backdrop = on;
                    see_through = on || state.transparent;
                    state.set_see_through(window, see_through);
                });
                crate::system::set_frameless_look(window, is_frameless(window), fullscreen, see_through);
            }
            WindowChange::Capture(number) => {
                let mut id = None;
                with_state(window, |state| id = Some(state.id));
                let picture = id.map_or_else(|| Err("the window's gone".to_owned()), capture_window);
                let hear = CAPTURE_HEARD.with(|heard| heard.borrow().clone());
                if let Some(hear) = hear {
                    hear(number, picture);
                }
            }
            WindowChange::Flash(how) => {
                let flags = match how.as_str() {
                    "stop" => FLASHW_STOP,
                    "informational" => FLASHW_TRAY,
                    _ => FLASHW_TRAY | FLASHW_TIMERNOFG,
                };
                let info = FLASHWINFO {
                    cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
                    hwnd: window,
                    dwFlags: flags,
                    uCount: if how == "informational" { 3 } else { 0 },
                    dwTimeout: 0,
                };
                let _ = FlashWindowEx(&raw const info);
            }
            WindowChange::Center => {
                let (_, work) = screen_of(window);
                let mut now = RECT::default();
                let _ = GetWindowRect(window, &raw mut now);
                let (width, height) = (now.right - now.left, now.bottom - now.top);
                let (x, y) = crate::shell_layer::from_screen(
                    window,
                    work.left + (work.right - work.left - width) / 2,
                    work.top + (work.bottom - work.top - height) / 2,
                );
                let _ = SetWindowPos(
                    window,
                    None,
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }
}

/// How big a window has to be for this much room inside it.
fn outer_size(window: HWND, inside: (u32, u32), scale: f32, frameless: bool) -> (i32, i32) {
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: (inside.0 as f32 * scale) as i32,
        bottom: (inside.1 as f32 * scale) as i32,
    };
    if !frameless {
        // SAFETY: `outer` is valid to write to, and asking about a window that's gone gives nothing.
        unsafe {
            let style = if window.is_invalid() {
                WS_OVERLAPPEDWINDOW
            } else {
                WINDOW_STYLE(GetWindowLongW(window, GWL_STYLE) as u32)
            };
            let _ = AdjustWindowRectEx(&raw mut outer, style, false, WINDOW_EX_STYLE::default());
        }
    }
    (outer.right - outer.left, outer.bottom - outer.top)
}

fn set_fullscreen(window: HWND, on: bool) {
    let mut before = None;
    with_state(window, |state| before = state.before_fullscreen);
    // SAFETY: as in `apply_window_change`, and `placement` is the size it says.
    unsafe {
        if on && before.is_none() {
            let style = GetWindowLongW(window, GWL_STYLE);
            let mut placement = WINDOWPLACEMENT {
                length: size_of::<WINDOWPLACEMENT>() as u32,
                ..Default::default()
            };
            let _ = GetWindowPlacement(window, &raw mut placement);
            with_state(window, |state| {
                state.before_fullscreen = Some((style, placement));
            });
            let (screen, _) = screen_of(window);
            SetWindowLongW(
                window,
                GWL_STYLE,
                (style as u32 & !WS_OVERLAPPEDWINDOW.0 | WS_POPUP.0) as i32,
            );
            let _ = SetWindowPos(
                window,
                Some(HWND_TOP),
                screen.left,
                screen.top,
                screen.right - screen.left,
                screen.bottom - screen.top,
                SWP_FRAMECHANGED | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
            );
        } else if let (false, Some((style, placement))) = (on, before) {
            with_state(window, |state| state.before_fullscreen = None);
            SetWindowLongW(window, GWL_STYLE, style);
            let _ = SetWindowPlacement(window, &raw const placement);
            let _ = SetWindowPos(window, None, 0, 0, 0, 0, QUIETLY | SWP_FRAMECHANGED);
        }
    }
    crate::system::set_frameless_look(window, is_frameless(window), on, is_see_through(window));
}

/// Does what's been asked of the windows.
fn do_window_requests() {
    let requests = crate::windows::take_window_requests();
    for wanted in requests.open {
        let owner = main_window().filter(|_| wanted.options.owned || wanted.options.modal);
        // SAFETY: this makes a window on this thread, with what it's given lasting through the calls.
        if let Err(error) = unsafe { open_a_window(wanted.id, &wanted.options, owner, wanted.make) }
        {
            eprintln!("A window couldn't open: {error}");
        }
    }
    for (id, title) in requests.titles {
        if let Some(window) = window_with_id(id) {
            // SAFETY: the window's on this thread, and the text outlives the call.
            unsafe {
                let _ = SetWindowTextW(window, &HSTRING::from(title.as_str()));
            }
        }
    }
    for (id, change) in requests.changes {
        if let Some(window) = window_with_id(id) {
            apply_window_change(window, change);
        }
    }
    for id in requests.close {
        if let Some(window) = window_with_id(id) {
            // What's still going on has to reach what's about to go, or its components never hear about it.
            with_state(window, |state| {
                state.app.update();
            });
            destroy_window(window);
        }
    }
}

/// Lets the window that a modal one was keeping from being used go back to being used, unless another modal one still is.
fn free_owner_of(window: HWND) {
    let owner = WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        let owner = windows
            .iter_mut()
            .find(|open| open.window == window)?
            .disabled_owner
            .take()?;
        let still_kept = windows
            .iter()
            .any(|open| open.disabled_owner == Some(owner));
        (!still_kept).then_some(owner)
    });
    if let Some(owner) = owner {
        // SAFETY: this does nothing if the window's gone.
        unsafe {
            let _ = EnableWindow(owner, true);
        }
    }
}

/// Destroys a window of this thread. Whatever it was keeping from being used has to be usable first: once it has left
/// the screen, the system looks for what to give the keyboard to, and if nothing in this program can take it, another
/// program comes to the front, with this one behind it.
fn destroy_window(window: HWND) {
    free_owner_of(window);
    // SAFETY: the window's on this thread, and doing this twice only gives an error.
    unsafe {
        let _ = DestroyWindow(window);
    }
}

fn window_with_id(id: WindowId) -> Option<HWND> {
    WINDOWS.with(|windows| {
        windows
            .borrow()
            .iter()
            .find(|open| open.id == id)
            .map(|open| open.window)
    })
}

fn main_window() -> Option<HWND> {
    window_with_id(WindowId::MAIN)
}

/// Whether the window someone's using is one of this program's.
fn this_program_is_in_front() -> bool {
    let mut process = 0;
    // SAFETY: asking about a window only gives an answer, and with no window there, this says no process.
    unsafe {
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut process));
    }
    process == std::process::id()
}

/// A window's on its way out, along with everything it has. Its components hear about that as this lets go of them.
fn window_destroyed(window: HWND) {
    // It may have gone along with its owner, without being asked to.
    free_owner_of(window);
    let gone = WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        windows
            .iter()
            .position(|open| open.window == window)
            .map(|at| windows.remove(at))
    });
    let Some(gone) = gone else {
        return;
    };
    if gone.id == WindowId::MAIN {
        // SAFETY: this ends the message loop on this thread, as intended.
        unsafe { PostQuitMessage(0) };
    }
}

/// Makes the kind of window these all are, the first time.
fn window_class() -> Result<(PCWSTR, [HCURSOR; 3], HINSTANCE), RunError> {
    static MADE: std::sync::Once = std::sync::Once::new();
    let class = w!("rsui-window");
    // SAFETY: these are given what they say they take, and what they give back is checked.
    unsafe {
        let instance: HINSTANCE = GetModuleHandleW(None)?.into();
        let arrow = LoadCursorW(None, IDC_ARROW)?;
        let cursors = [
            arrow,
            LoadCursorW(None, IDC_HAND)?,
            LoadCursorW(None, IDC_IBEAM)?,
        ];
        MADE.call_once(|| {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(window_procedure),
                hInstance: instance,
                lpszClassName: class,
                style: CS_DBLCLKS,
                hCursor: arrow,
                // The icon that was put in the program, if one was. See `rsui_tsx::pack`.
                hIcon: LoadIconW(Some(instance), PCWSTR(std::ptr::without_provenance(1)))
                    .unwrap_or_default(),
                // What shows until the GPU is ready, which should go with what's around it. COLORREF is
                // blue, green, red.
                hbrBackground: CreateSolidBrush(COLORREF(if crate::system::system_is_dark() {
                    0x0017_110D
                } else {
                    u32::from(BACKGROUND.b) << 16
                        | u32::from(BACKGROUND.g) << 8
                        | u32::from(BACKGROUND.r)
                })),
                ..Default::default()
            });
        });
        Ok((class, cursors, instance))
    }
}

/// Opens a window, with what's in it following when the GPU's ready.
///
/// # Safety
///
/// This has to be on the thread that runs the message loop.
/// Starts making a renderer for a window on another thread, into `slot`, which the window hears about with `RENDERER_READY`
/// when it's done, along with what keeps the memory the GPU starts with in check until then.
fn start_building_renderer(
    window: HWND,
    slot: &RendererSlot,
    see_through: bool,
) -> (JoinHandle<()>, Option<WorkingSetLimit>) {
    let size = client_size(window);
    let slot = slot.clone();
    let handle = WindowHandle(window.0);
    // Not while input's being handled, which needs what it needs (see `window_procedure`).
    let limit = (!HANDLING_INPUT.with(Cell::get))
        .then(|| WorkingSetLimit::with_room(GPU_STARTUP_ROOM, GPU_STARTUP_AT_LEAST));
    let building = std::thread::spawn(move || {
        let handle = handle;
        let result = Renderer::for_window_seen_through(HWND(handle.0), size, see_through);
        crate::mark("renderer made");
        *slot.lock().expect("the slot is only ever assigned") = Some(result);
        // SAFETY: posting is safe from any thread, and does nothing if the window has closed.
        unsafe {
            let _ = PostMessageW(Some(HWND(handle.0)), RENDERER_READY, WPARAM(0), LPARAM(0));
        }
    });
    (building, limit)
}

unsafe fn open_a_window(
    id: WindowId,
    options: &WindowOptions,
    owner: Option<HWND>,
    make: MakeApp,
) -> Result<HWND, RunError> {
    let text = default_text_engine()?;
    let (class, cursors, instance) = window_class()?;
    // SAFETY: these create a window on this thread, with arguments that outlive the calls, and each result is checked.
    unsafe {
        let (outer_width, outer_height) = match options.size {
            // The window's edges and title come on top of what's inside it.
            Some(size) => outer_size(
                HWND::default(),
                size,
                GetDpiForSystem() as f32 / 96.0,
                options.frameless,
            ),
            None => (960, 640),
        };
        // A transparent window has nothing of the system's own under what DirectComposition shows, so nothing else shows
        // before its first frame, or where that's see-through.
        let extended = if options.transparent { WS_EX_NOREDIRECTIONBITMAP } else { WINDOW_EX_STYLE::default() };
        let window = CreateWindowExW(
            extended,
            class,
            &HSTRING::from(options.title.as_str()),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            outer_width,
            outer_height,
            owner,
            None,
            Some(instance),
            None,
        )?;
        if id == WindowId::MAIN {
            crate::dialogs::set_current_window(window);
            crate::desktop::attach_menu_bar(window);
        }
        if options.frameless {
            apply_window_change(window, WindowChange::Frameless(true));
        }
        crate::mark("made window");
        // What it's drawn with is made while what's in it is worked out, rather than after.
        let renderer_slot = RendererSlot::default();
        let (building, gpu_startup_limit) = start_building_renderer(window, &renderer_slot, options.transparent);
        let size = client_size(window);
        let mut app = make(text, size, WindowState::scale(window));
        crate::mark("app");
        app.set_clipboard(SystemClipboard(window));

        let handle = WindowHandle(window.0);
        app.set_animations(system_allows_animations());
        // Code that hears the pointer move hears it once a frame at most, as a browser has it.
        app.set_move_pace(frame_time());
        app.set_frame_time(frame_time());
        let origin = screen_origin(window);
        app.set_screen_origin(origin.0, origin.1);
        app.set_waker(move || {
            let handle = handle;
            let _ = PostMessageW(Some(HWND(handle.0)), WAKE, WPARAM(0), LPARAM(0));
        });
        // The first window of a program that's being worked on has the pill in its corner, and keeps where frames'
        // time goes from the start.
        if crate::inspect::inspector_allowed() {
            crate::inspect::keep_spans_lightly();
            static GIVEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !options.modal && !GIVEN.swap(true, std::sync::atomic::Ordering::Relaxed) {
                app.set_pill(true);
            }
        }
        let disabled_owner = owner.filter(|_| options.modal);
        if let Some(owner) = disabled_owner {
            let _ = EnableWindow(owner, false);
        }
        let state = WindowState {
            app,
            id,
            smallest: options.smallest,
            largest: None,
            renderer: None,
            renderer_slot,
            building: Some(building),
            hidden: false,
            showing_picture: false,
            gpu_startup_limit,
            gave_back: None,
            covered: None,
            blink_timer: false,
            pointer_at: None,
            last_moving_frame: None,
            last_paint_started: None,
            frame_pace: None,
            used_at: Instant::now(),
            quiet_while_moving: false,
            unattended_wait: DEEP_IDLE_UNATTENDED,
            deep_idle_since: None,
            last_activity: Instant::now(),
            trim_waiting: false,
            quiet: QUIET,
            cursors,
            shown_cursor: Cursor::Arrow,
            accepts_files: false,
            wake_at: None,
            access: None,
            access_fingerprint: None,
            tracking_pointer: false,
            pressing: false,
            high_surrogate: None,
            composed: None,
            drop_target: false,
            before_fullscreen: None,
            dock: None,
            dock_dragging: false,
            thumbnail_buttons: Vec::new(),
            taskbar_button: false,
            thumbnail_buttons_added: false,
            thumbnail_icons: Vec::new(),
            backdrop: false,
            transparent: false,
        };
        WINDOWS.with(|windows| {
            windows.borrow_mut().push(OpenWindow {
                window,
                id,
                disabled_owner,
                state: Rc::new(RefCell::new(state)),
            });
        });
        // Its layer, transparency, app bar and the rest, before it's seen, as it should look from the start.
        for change in options.startup_changes() {
            apply_window_change(window, change);
        }
        // The window goes up right away, and what's in it follows when the GPU is ready. The main one does what it was
        // started to do. Any other only takes the keyboard from this program, never from what someone's using instead,
        // as an app can open a window at any time, and they could be typing or playing something.
        #[cfg(feature = "idle-probe")]
        idle_probe::opened(window);
        let show = if !cfg!(feature = "idle-probe")
            && (id == WindowId::MAIN || this_program_is_in_front())
        {
            SW_SHOW
        } else {
            SW_SHOWNOACTIVATE
        };
        let _ = ShowWindow(window, show);
        crate::mark("shown");
        with_state(window, |state| {
            state.build_renderer(window);
            // Anything it was asked before it was ready to say is said now.
            state.update_and_draw(window);
        });
        Ok(window)
    }
}

/// Opens a window showing `render` as its root component, and runs until it's closed.
pub fn run<P: 'static, R: IntoNode + 'static>(
    title: &str,
    render: fn(&mut Cx<'_>, &P) -> R,
    props: P,
) -> Result<(), RunError> {
    run_sized(title, None, render, props)
}

/// [`run`], with room for this much inside the window, in logical pixels, if that's given.
pub fn run_sized<P: 'static, R: IntoNode + 'static>(
    title: &str,
    size: Option<(u32, u32)>,
    render: fn(&mut Cx<'_>, &P) -> R,
    props: P,
) -> Result<(), RunError> {
    // The graphics driver loads while the first frame's worked out, rather than after.
    #[cfg(feature = "idle-probe")]
    let _native_cycles = crate::native_cycles::Session::start();
    rsui_render::start_gpu_early();
    MAIN_OPENED.with(|opened| opened.set(true));
    let options = WindowOptions {
        title: title.to_owned(),
        size,
        ..main_window_options()
    };
    // SAFETY: this is the thread that goes on to run the message loop. What it makes is let go of before this returns.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        open_a_window(
            WindowId::MAIN,
            &options,
            None,
            Box::new(move |text, size, scale| App::new(render, props, text, size, scale)),
        )?;
        // From here on, this hears about anything that's asked for, at a moment when nothing's going on.
        crate::windows::tell_with(|| {
            if let Some(window) =
                WINDOWS.with(|windows| windows.borrow().first().map(|open| open.window))
            {
                // SAFETY: posting is safe, and does nothing if the window has closed.
                let _ = PostMessageW(Some(window), WINDOWS_WANTED, WPARAM(0), LPARAM(0));
            }
        });
        do_window_requests();
        let mut message = MSG::default();
        let mut frame_windows = Vec::new();
        let mut input_batch = InputPaintBatch::default();
        // This sleeps until there's a message, so an idle app costs nothing.
        while {
            #[cfg(feature = "idle-probe")]
            let _native_cycles = crate::native_cycles::enter(crate::native_cycles::GET_MESSAGE);
            GetMessageW(&mut message, None, 0, 0).as_bool()
        } {
            #[cfg(feature = "idle-probe")]
            crate::native_cycles::note_message(message.message);
            let input_began = Instant::now();
            {
                #[cfg(feature = "idle-probe")]
                let _native_cycles = crate::native_cycles::enter(crate::native_cycles::DISPATCH);
                if !crate::desktop::translate_menu_keys(&message) {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            // A naturally delivered paint already had its opportunity. Under a
            // backlog, allow a bounded slice of continuous input between forced
            // paints; a slow frame must not reduce input service to one message.
            if message.message == WM_PAINT {
                input_batch.painted();
            } else if input_batch.checkpoint(input_began, Instant::now(), more_continuous_input(&message))
                && present_due_windows(&mut frame_windows)
            {
                input_batch.painted();
            }
        }
        // Whatever's left goes now, with those belonging to the main one already gone along with it.
        let left: Vec<HWND> =
            WINDOWS.with(|windows| windows.borrow().iter().map(|open| open.window).collect());
        for window in left {
            let _ = DestroyWindow(window);
        }
    }
    WINDOWS.with(|windows| windows.borrow_mut().clear());
    crate::desktop::let_go_of_everything();
    crate::windows::tell_with(|| {});
    match FAILURE.with(RefCell::take) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_press_has_all_of_a_frameless_window_be_its_own() {
        let never = || -> bool { panic!("nothing more is asked while a press is held") };
        assert_eq!(frameless_hit(true, || panic!("nor its edges"), never), HTCLIENT, "a press held over where it's dragged by");
        assert_eq!(frameless_hit(false, || None, || true), HTCAPTION, "where it's dragged by, with nothing held");
        assert_eq!(frameless_hit(false, || None, || false), HTCLIENT);
        assert_eq!(frameless_hit(false, || Some(HTTOPLEFT), || true), HTTOPLEFT, "an edge comes first");
        assert_eq!(edge_at(2, 300, 800, 600, 8), Some(HTLEFT));
        assert_eq!(edge_at(400, 300, 800, 600, 8), None);
    }

    #[test]
    fn the_pointer_is_let_go_of_once_no_button_is_down() {
        // As each button's message has it: what's still down, which for one coming up is the others.
        assert!(!buttons_held(WPARAM(0)));
        assert!(!buttons_held(WPARAM(0x0004 | 0x0008)), "shift and ctrl aren't buttons");
        assert!(buttons_held(WPARAM(0x0001)), "the main button's still down while another comes up");
        assert!(buttons_held(WPARAM(0x0002)));
        assert!(buttons_held(WPARAM(0x0010)));
        assert!(buttons_held(WPARAM(0x0020 | (1 << 16))), "the high word of an extra button's says which it was");
        assert!(buttons_held(WPARAM(0x0040)));
    }

    #[test]
    fn input_paint_batches_end_at_a_message_time_or_queue_boundary() {
        let start = Instant::now();
        let mut batch = InputPaintBatch::default();
        for _ in 1..INPUT_MESSAGES_PER_PAINT {
            assert!(!batch.checkpoint(start, start, true));
        }
        assert!(batch.checkpoint(start, start, true), "paint must get a turn under an unlimited producer");
        batch.painted();
        assert!(!batch.checkpoint(start, start, true));
        assert!(batch.checkpoint(start, start + INPUT_SLICE_PER_PAINT, true));
        batch.painted();
        assert!(batch.checkpoint(start, start + Duration::from_millis(5), true), "a slow handler consumes the time budget");
        batch.painted();
        assert!(batch.checkpoint(start, start, false), "an empty queue or command must not wait for more input");
        batch.painted();
        let later = start + Duration::from_secs(1);
        assert!(!batch.checkpoint(later, later, true), "a completed paint starts a fresh input slice");
    }

    #[test]
    fn paint_batch_inspection_keeps_mixed_continuous_input_and_commands_in_order() {
        use windows::Win32::System::Threading::GetCurrentThreadId;
        let mut next = MSG::default();
        let wheel = MSG { message: WM_MOUSEWHEEL, ..MSG::default() };
        // SAFETY: this test creates, inspects and drains only its own thread queue.
        unsafe {
            let _ = PeekMessageW(&mut next, None, 0, 0, PM_NOREMOVE);
            let thread = GetCurrentThreadId();
            PostThreadMessageW(thread, WM_MOUSEMOVE, WPARAM(0), LPARAM(1)).unwrap();
            PostThreadMessageW(thread, WAKE, WPARAM(0), LPARAM(0)).unwrap();
            PostThreadMessageW(thread, WM_LBUTTONDOWN, WPARAM(0), LPARAM(1)).unwrap();
            PostThreadMessageW(thread, WM_MOUSEWHEEL, WPARAM(0), LPARAM(1)).unwrap();
            assert!(more_continuous_input(&wheel), "mixed hover/wheel input may share a service slice");
            let key = MSG { message: WM_KEYDOWN, ..MSG::default() };
            assert!(!more_continuous_input(&key), "keys checkpoint even when more input is queued");
            let other_window = MSG { hwnd: HWND(1usize as *mut c_void), ..wheel };
            assert!(!more_continuous_input(&other_window), "another window must get its checkpoint");
            assert!(PeekMessageW(&mut next, None, 0, 0, PM_REMOVE).as_bool());
            assert_eq!(next.message, WM_MOUSEMOVE, "inspection must not remove or reorder input");
            assert!(more_continuous_input(&next), "internal wake work stays in the same bounded service slice");
            assert!(PeekMessageW(&mut next, None, 0, 0, PM_REMOVE).as_bool());
            assert_eq!(next.message, WAKE, "wake work must be dispatched rather than skipped");
            assert!(!more_continuous_input(&next), "a click after a wake still ends the service slice");
            assert!(!more_continuous_input(&wheel), "a click ends a continuous-input slice");
            assert!(PeekMessageW(&mut next, None, 0, 0, PM_REMOVE).as_bool());
            assert_eq!(next.message, WM_LBUTTONDOWN);
            assert!(PeekMessageW(&mut next, None, 0, 0, PM_REMOVE).as_bool());
            assert_eq!(next.message, WM_MOUSEWHEEL);
            assert!(!more_continuous_input(&wheel), "an empty queue asks for a checkpoint immediately");
        }
    }

    #[test]
    fn fine_wheel_reports_keep_each_fraction_and_notch_direction() {
        assert_eq!(wheel_pixels(120, false), -100.0);
        assert_eq!(wheel_pixels(120, true), 100.0);
        assert_eq!(wheel_pixels(-120, false), 100.0);
        assert!((wheel_pixels(8, false) + 100.0 / 15.0).abs() < 0.00001);
        for delta in -119..120 {
            if delta != 0 {
                assert!(precise_wheel_delta(delta));
                assert_ne!(wheel_pixels(delta, false), 0.0, "fine input must not wait for a row");
            }
        }
        assert!(!precise_wheel_delta(120));
        assert!(!precise_wheel_delta(240));
        assert_eq!(wheel_delta(WPARAM(usize::from((-8_i16) as u16) << 16)), -8);
    }

    #[test]
    fn what_is_asked_of_the_main_window_before_it_opens_is_how_it_opens() {
        // Each test has a thread of its own, and so a main window of its own that hasn't opened.
        let _ = crate::take_window_requests();
        let bar = crate::AppBar { edge: crate::ScreenEdge::Bottom, size: 28, monitor: None };
        for change in [
            WindowChange::Frameless(true),
            WindowChange::Transparent(true),
            WindowChange::Layer(WindowLayer::Desktop),
            // As the `Window` props say it whenever the frame changes, which mustn't take it off the desktop.
            WindowChange::OnTop(false),
            WindowChange::SkipTaskbar(true),
            WindowChange::AllDesktops(true),
            WindowChange::AppBar(Some(bar)),
        ] {
            crate::change_window(WindowId::MAIN, change);
        }
        assert!(crate::take_window_requests().is_empty(), "none of it waits for the window");
        let options = main_window_options();
        assert!(options.frameless && options.transparent && options.skip_taskbar && options.all_desktops);
        assert_eq!(options.effective_layer(), WindowLayer::Desktop);
        assert_eq!(options.app_bar, Some(bar));
        assert_eq!(
            options.startup_changes(),
            [
                WindowChange::Transparent(true),
                WindowChange::SkipTaskbar(true),
                WindowChange::AllDesktops(true),
                WindowChange::Layer(WindowLayer::Desktop),
                WindowChange::AppBar(Some(bar)),
            ]
        );
        // On top and back is the top layer and back, as it would be once it's open.
        crate::change_window(WindowId::MAIN, WindowChange::OnTop(true));
        assert_eq!(main_window_options().effective_layer(), WindowLayer::Top);
        crate::change_window(WindowId::MAIN, WindowChange::OnTop(false));
        assert_eq!(main_window_options().effective_layer(), WindowLayer::Normal);
        crate::change_window(WindowId::MAIN, WindowChange::Layer(WindowLayer::Top));
        assert!(main_window_options().on_top, "the top layer is on top");
        crate::change_window(WindowId::MAIN, WindowChange::AppBar(None));
        assert_eq!(main_window_options().app_bar, None);
        // A script's exports say all of these at once.
        set_main_window_options(&WindowOptions { layer: WindowLayer::Bottom, ..WindowOptions::default() });
        let options = main_window_options();
        assert_eq!(options.effective_layer(), WindowLayer::Bottom);
        assert!(!options.transparent && !options.skip_taskbar && !options.all_desktops);
        assert!(options.frameless, "what `set_main_window_kind` says stays");
    }

    #[test]
    fn once_the_main_window_is_open_what_is_asked_of_it_waits_for_it() {
        let _ = crate::take_window_requests();
        main_opened();
        crate::change_window(WindowId::MAIN, WindowChange::Layer(WindowLayer::Bottom));
        crate::change_window(WindowId::MAIN, WindowChange::Transparent(true));
        assert_eq!(
            crate::take_window_requests().changes,
            [
                (WindowId::MAIN, WindowChange::Layer(WindowLayer::Bottom)),
                (WindowId::MAIN, WindowChange::Transparent(true)),
            ]
        );
        assert_eq!(main_window_options().layer, WindowLayer::Normal);
    }

    const fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn the_edges_of_a_window_with_no_frame_can_be_taken_hold_of() {
        let at = |x, y| edge_at(x, y, 400, 300, 8);
        assert_eq!(at(200, 150), None, "the middle");
        assert_eq!(at(8, 8), None, "just inside the corner");
        assert_eq!(at(391, 291), None, "just inside the other corner");
        assert_eq!(at(0, 0), Some(HTTOPLEFT));
        assert_eq!(at(399, 0), Some(HTTOPRIGHT));
        assert_eq!(at(0, 299), Some(HTBOTTOMLEFT));
        assert_eq!(at(399, 299), Some(HTBOTTOMRIGHT));
        assert_eq!(at(0, 150), Some(HTLEFT));
        assert_eq!(at(399, 150), Some(HTRIGHT));
        assert_eq!(at(200, 0), Some(HTTOP));
        assert_eq!(at(200, 299), Some(HTBOTTOM));
    }

    #[test]
    fn a_window_only_covers_what_is_all_inside_it() {
        let own = rect(100, 100, 200, 200);
        assert!(contains(rect(0, 0, 1000, 1000), own));
        assert!(contains(own, own));
        assert!(!contains(rect(0, 0, 199, 1000), own), "one column shows");
        assert!(!contains(rect(101, 0, 1000, 1000), own), "one column shows");
        assert!(!contains(rect(0, 0, 1000, 150), own), "half of it shows");
    }
}
