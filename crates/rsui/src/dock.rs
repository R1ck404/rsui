//! Another program's window kept inside this one's, along its right or bottom edge, as a browser keeps its inspector:
//! `rsui-tsx --view` asks for it with its dock buttons.
//!
//! The other window becomes a child of this one, with no frame, and this one's app is laid out in what's left, with a
//! strip between the two that can be dragged to share the room out differently. Giving it back, or this window closing,
//! makes it a window of its own again, as it was. It's the other program's still: it draws itself and takes its own input.

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::BOOL;

/// Which edge of the window the other one goes along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockSide {
    Right,
    Bottom,
}

/// What's asked for: the main window of process `pid`, along `side`, `size` logical pixels across.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DockRequest {
    pub pid: u32,
    pub side: DockSide,
    pub size: f32,
}

/// The strip between the two, in logical pixels, which is what's dragged.
const GAP: f32 = 5.0;
/// The least either side has, in logical pixels.
const LEAST: f32 = 160.0;

/// Another window that's been taken in, and how it was before.
pub(crate) struct Docked {
    pub(crate) child: HWND,
    pub(crate) side: DockSide,
    /// How far across it is, in logical pixels.
    pub(crate) size: f32,
    style: isize,
    ex_style: isize,
    rect: RECT,
}

/// The main window of a process: one that's shown, has no owner, and isn't a child.
pub(crate) fn window_of(pid: u32) -> Option<HWND> {
    struct Search {
        pid: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn each(window: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the search this was given, which outlives the enumeration.
        let search = unsafe { &mut *(data.0 as *mut Search) };
        let mut pid = 0;
        // SAFETY: `window` is one the system is enumerating, and `pid` is valid to write to.
        unsafe {
            GetWindowThreadProcessId(window, Some(&raw mut pid));
            if pid == search.pid
                && IsWindowVisible(window).as_bool()
                && GetWindow(window, GW_OWNER).is_err_or_null()
            {
                search.found = Some(window);
                return BOOL(0);
            }
        }
        BOOL(1)
    }
    let mut search = Search { pid, found: None };
    // SAFETY: the callback only uses `search`, which lives until this returns.
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&raw mut search as isize));
    }
    search.found
}

trait NullWindow {
    fn is_err_or_null(&self) -> bool;
}

impl NullWindow for windows::core::Result<HWND> {
    fn is_err_or_null(&self) -> bool {
        self.as_ref().map_or(true, |window| window.0.is_null())
    }
}

/// Takes `child` into `parent`, with no frame, along `side`.
pub(crate) fn adopt(parent: HWND, child: HWND, side: DockSide, size: f32) -> Docked {
    let mut rect = RECT::default();
    // SAFETY: both are windows that exist, and `rect` is valid to write to.
    unsafe {
        let _ = GetWindowRect(child, &mut rect);
        let style = GetWindowLongPtrW(child, GWL_STYLE);
        let ex_style = GetWindowLongPtrW(child, GWL_EXSTYLE);
        let framed = (WS_POPUP | WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX).0 as isize;
        SetWindowLongPtrW(child, GWL_STYLE, (style & !framed) | (WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS).0 as isize);
        let ex_framed = (WS_EX_APPWINDOW | WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_DLGMODALFRAME).0 as isize;
        SetWindowLongPtrW(child, GWL_EXSTYLE, ex_style & !ex_framed);
        let _ = SetParent(child, Some(parent));
        // What's under it isn't drawn over it.
        let parent_style = GetWindowLongPtrW(parent, GWL_STYLE);
        SetWindowLongPtrW(parent, GWL_STYLE, parent_style | WS_CLIPCHILDREN.0 as isize);
        Docked {
            child,
            side,
            size,
            style,
            ex_style,
            rect,
        }
    }
}

impl Docked {
    /// Whether it's the window of process `pid`, which once it's inside this one isn't found among the windows of their own.
    pub(crate) fn belongs_to(&self, pid: u32) -> bool {
        let mut owner = 0;
        // SAFETY: asking about a window that may be gone is harmless, and `owner` is valid to write to.
        unsafe {
            GetWindowThreadProcessId(self.child, Some(&raw mut owner));
        }
        owner == pid && self.alive()
    }

    /// Whether the other window's still there, which it isn't once its program has ended.
    pub(crate) fn alive(&self) -> bool {
        // SAFETY: asking about a window that may be gone is harmless.
        unsafe { IsWindow(Some(self.child)).as_bool() }
    }

    /// How far across it is, in physical pixels, kept to what leaves room for both.
    fn across(&self, client: (u32, u32), scale: f32) -> f32 {
        let whole = match self.side {
            DockSide::Right => client.0,
            DockSide::Bottom => client.1,
        } as f32;
        let most = (whole - (LEAST + GAP) * scale).max(0.0);
        (self.size * scale).clamp(LEAST * scale, most.max(LEAST * scale)).min(whole)
    }

    /// How big what's left for this window's own app is.
    pub(crate) fn content_size(&self, client: (u32, u32), scale: f32) -> (u32, u32) {
        let taken = (self.across(client, scale) + GAP * scale).round() as u32;
        match self.side {
            DockSide::Right => (client.0.saturating_sub(taken).max(1), client.1),
            DockSide::Bottom => (client.0, client.1.saturating_sub(taken).max(1)),
        }
    }

    /// Puts the other window where it goes in a window of this size.
    pub(crate) fn place(&self, client: (u32, u32), scale: f32) {
        let across = self.across(client, scale).round() as i32;
        let (x, y, width, height) = match self.side {
            DockSide::Right => (client.0 as i32 - across, 0, across, client.1 as i32),
            DockSide::Bottom => (0, client.1 as i32 - across, client.0 as i32, across),
        };
        // SAFETY: moving a child window, which is harmless if it's gone.
        unsafe {
            let _ = SetWindowPos(self.child, None, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
    }

    /// Whether a point in the window, in physical pixels, is on the strip between the two.
    pub(crate) fn on_gap(&self, client: (u32, u32), scale: f32, x: f32, y: f32) -> bool {
        let content = self.content_size(client, scale);
        let gap = GAP * scale;
        match self.side {
            DockSide::Right => x >= content.0 as f32 && x < content.0 as f32 + gap,
            DockSide::Bottom => y >= content.1 as f32 && y < content.1 as f32 + gap,
        }
    }

    /// Makes it as far across as dragging the strip to this point says, in physical pixels.
    pub(crate) fn drag_to(&mut self, client: (u32, u32), scale: f32, x: f32, y: f32) {
        let left = match self.side {
            DockSide::Right => client.0 as f32 - x,
            DockSide::Bottom => client.1 as f32 - y,
        };
        self.size = (left / scale - GAP / 2.0).max(LEAST);
    }

    /// The cursor for the strip.
    pub(crate) fn cursor(&self) -> windows::core::PCWSTR {
        match self.side {
            DockSide::Right => IDC_SIZEWE,
            DockSide::Bottom => IDC_SIZENS,
        }
    }

    /// Makes it a window of its own again, as it was before.
    pub(crate) fn give_back(self) {
        if !self.alive() {
            return;
        }
        // SAFETY: the window exists, and these put back what it had.
        unsafe {
            let _ = SetParent(self.child, None);
            SetWindowLongPtrW(self.child, GWL_STYLE, self.style);
            SetWindowLongPtrW(self.child, GWL_EXSTYLE, self.ex_style);
            let RECT { left, top, right, bottom } = self.rect;
            let _ = SetWindowPos(
                self.child,
                None,
                left,
                top,
                right - left,
                bottom - top,
                SWP_NOZORDER | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docked(side: DockSide, size: f32) -> Docked {
        Docked {
            child: HWND::default(),
            side,
            size,
            style: 0,
            ex_style: 0,
            rect: RECT::default(),
        }
    }

    #[test]
    fn the_app_has_what_the_docked_window_and_the_strip_leave() {
        let right = docked(DockSide::Right, 400.0);
        assert_eq!(right.content_size((1200, 800), 1.0), (795, 800));
        // At twice the pixels, it's twice as many of them.
        assert_eq!(right.content_size((2400, 1600), 2.0), (1590, 1600));
        let bottom = docked(DockSide::Bottom, 300.0);
        assert_eq!(bottom.content_size((1200, 800), 1.0), (1200, 495));
    }

    #[test]
    fn neither_side_gets_squeezed_out() {
        let wide = docked(DockSide::Right, 5000.0);
        let (width, _) = wide.content_size((1000, 600), 1.0);
        assert!(width >= 160, "{width}");
        let narrow = docked(DockSide::Right, 10.0);
        assert_eq!(narrow.content_size((1000, 600), 1.0).0, 1000 - 160 - 5);
    }

    #[test]
    fn the_strip_between_is_where_dragging_starts_and_dragging_it_moves_it() {
        let mut right = docked(DockSide::Right, 400.0);
        assert!(right.on_gap((1200, 800), 1.0, 797.0, 10.0));
        assert!(!right.on_gap((1200, 800), 1.0, 790.0, 10.0));
        assert!(!right.on_gap((1200, 800), 1.0, 900.0, 10.0));
        right.drag_to((1200, 800), 1.0, 600.0, 10.0);
        // The strip's middle is where it was let go.
        assert_eq!(right.content_size((1200, 800), 1.0).0, 597);
    }
}
