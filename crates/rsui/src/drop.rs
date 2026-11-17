//! What's dragged in from other programs, as the system's drag and drop has it: files from the file browser, and text or links
//! from anywhere, which code written for a page hears about while it's over the window as well as when it's dropped. This is
//! only set up for a window once something in the app says it wants that, and what it needs is only loaded then, as it
//! costs a little to start and most apps never need it.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::OnceLock;

use windows::Win32::Foundation::{HWND, POINT, POINTL};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, TYMED_HGLOBAL};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE,
    IDropTarget, IDropTarget_Impl,
};
use windows::Win32::System::SystemServices::{MK_CONTROL, MK_SHIFT, MODIFIERKEYS_FLAGS};
use windows::core::{Interface, Ref, implement, s, w};

use crate::app::DropPhase;
use crate::input::Modifiers;

/// What's needed from the system for this, which is loaded the first time it's wanted.
struct Ole {
    initialize: unsafe extern "system" fn(*const c_void) -> i32,
    register: unsafe extern "system" fn(HWND, *mut c_void) -> i32,
    revoke: unsafe extern "system" fn(HWND) -> i32,
    release_medium: unsafe extern "system" fn(*mut STGMEDIUM),
}

fn ole() -> Option<&'static Ole> {
    static FOUND: OnceLock<Option<Ole>> = OnceLock::new();
    type Found = unsafe extern "system" fn() -> isize;
    FOUND
        .get_or_init(|| {
            // SAFETY: the library stays loaded for good, and these functions have these signatures.
            unsafe {
                let library = LoadLibraryW(w!("ole32.dll")).ok()?;
                Some(Ole {
                    initialize: std::mem::transmute::<Found, unsafe extern "system" fn(*const c_void) -> i32>(
                        GetProcAddress(library, s!("OleInitialize"))?,
                    ),
                    register: std::mem::transmute::<Found, unsafe extern "system" fn(HWND, *mut c_void) -> i32>(
                        GetProcAddress(library, s!("RegisterDragDrop"))?,
                    ),
                    revoke: std::mem::transmute::<Found, unsafe extern "system" fn(HWND) -> i32>(GetProcAddress(
                        library,
                        s!("RevokeDragDrop"),
                    )?),
                    release_medium: std::mem::transmute::<Found, unsafe extern "system" fn(*mut STGMEDIUM)>(
                        GetProcAddress(library, s!("ReleaseStgMedium"))?,
                    ),
                })
            }
        })
        .as_ref()
}

thread_local! {
    /// Whether this thread has what drag and drop needs started, which it only ever does once.
    static STARTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Starts what drag and drop needs on this thread, the first time, both to have things dropped on a window and to drag them
/// out of one, and says whether it's there.
pub(crate) fn start_ole() -> bool {
    let Some(ole) = ole() else {
        return false;
    };
    if STARTED.with(|started| started.get()) {
        return true;
    }
    // SAFETY: this is called on the thread it's for, and is only done once on each.
    let started = unsafe { (ole.initialize)(std::ptr::null()) } >= 0;
    STARTED.with(|kept| kept.set(started));
    started
}

/// Has the system tell this window about what's dragged over it from other programs, and says whether it will.
pub(crate) fn listen(window: HWND) -> bool {
    let Some(ole) = ole() else {
        return false;
    };
    if !start_ole() {
        return false;
    }
    // SAFETY: this is the window's own thread, which drag and drop has been started on, and the target given lasts for as
    // long as it's registered, as the system holds on to it.
    unsafe {
        let target: IDropTarget = Target {
            window,
            what: RefCell::default(),
        }
        .into();
        (ole.register)(window, target.as_raw()) >= 0
    }
}

/// Stops that, before the window goes.
pub(crate) fn stop_listening(window: HWND) {
    if let Some(ole) = ole() {
        // SAFETY: it's only told about this window, which is still there.
        unsafe {
            (ole.revoke)(window);
        }
    }
}

/// What's being dragged over the window, read when it comes over it.
#[derive(Default)]
struct Dragged {
    files: Vec<String>,
    text: Option<String>,
}

#[implement(IDropTarget)]
struct Target {
    window: HWND,
    what: RefCell<Dragged>,
}

impl Target {
    /// Tells the app, and gives the system what it can do here: copy it, if the app says it can be dropped here, and
    /// nothing otherwise.
    fn tell(&self, phase: DropPhase, keys: MODIFIERKEYS_FLAGS, at: &POINTL, effect: *mut DROPEFFECT) {
        let mut point = POINT { x: at.x, y: at.y };
        // SAFETY: `point` is a valid point to convert, for a window that's there.
        unsafe {
            let _ = ScreenToClient(self.window, &raw mut point);
        }
        let held = Modifiers {
            ctrl: keys.0 & MK_CONTROL.0 != 0,
            shift: keys.0 & MK_SHIFT.0 != 0,
            ..Modifiers::default()
        };
        let what = self.what.borrow();
        let accepted = crate::window::drag_from_outside(
            self.window,
            phase,
            point.x as f32,
            point.y as f32,
            held,
            &what.files,
            what.text.as_deref(),
        );
        if effect.is_null() {
            return;
        }
        // SAFETY: the system gives somewhere to say this, which says what the program it's from allows.
        unsafe {
            let allowed = *effect;
            *effect = if !accepted {
                DROPEFFECT_NONE
            } else if allowed.0 & DROPEFFECT_COPY.0 != 0 {
                DROPEFFECT_COPY
            } else if allowed.0 & DROPEFFECT_MOVE.0 != 0 {
                DROPEFFECT_MOVE
            } else {
                allowed & DROPEFFECT_LINK
            };
        }
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        at: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        *self.what.borrow_mut() = data.as_ref().map(read).unwrap_or_default();
        self.tell(DropPhase::Enter, keys, at, effect);
        Ok(())
    }

    fn DragOver(&self, keys: MODIFIERKEYS_FLAGS, at: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        self.tell(DropPhase::Over, keys, at, effect);
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        let what = self.what.take();
        crate::window::drag_from_outside(self.window, DropPhase::Leave, 0.0, 0.0, Modifiers::default(), &what.files, None);
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<IDataObject>,
        keys: MODIFIERKEYS_FLAGS,
        at: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        if let Some(data) = data.as_ref() {
            *self.what.borrow_mut() = read(data);
        }
        self.tell(DropPhase::Drop, keys, at, effect);
        self.what.take();
        Ok(())
    }
}

/// The files and text in what's being dragged, as far as they can be read.
fn read(data: &IDataObject) -> Dragged {
    let Some(ole) = ole() else {
        return Dragged::default();
    };
    let ask = |format: u16| FORMATETC {
        cfFormat: format,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let mut dragged = Dragged::default();
    // SAFETY: what's given back is only read while it's locked, and given back after.
    unsafe {
        if let Ok(mut medium) = data.GetData(&ask(CF_HDROP.0)) {
            dragged.files = crate::system::dropped_file_names(medium.u.hGlobal.0);
            (ole.release_medium)(&raw mut medium);
        }
        if let Ok(mut medium) = data.GetData(&ask(CF_UNICODETEXT.0)) {
            let start = GlobalLock(medium.u.hGlobal).cast::<u16>();
            if !start.is_null() {
                let mut length = 0;
                while *start.add(length) != 0 {
                    length += 1;
                }
                dragged.text = Some(String::from_utf16_lossy(std::slice::from_raw_parts(start, length)));
                let _ = GlobalUnlock(medium.u.hGlobal);
            }
            (ole.release_medium)(&raw mut medium);
        }
    }
    dragged
}
