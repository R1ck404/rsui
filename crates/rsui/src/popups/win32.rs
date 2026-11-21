//! The popups on Windows. Questions, messages and sheets are task dialogs, and the pickers a dialog made here around
//! Windows' own month calendar or time control. Each runs on a thread of its own, as the file dialogs do, belonging to the
//! app's window, which stays disabled until it's answered and goes on drawing meanwhile. The share sheet is the system's,
//! shown from the window's own thread. The toast is a little window of rsui's own on a thread of its own, as Windows has
//! nothing like it.
//!
//! Task dialogs and the date controls are in version 6 of the common controls, which a program only gets with a manifest
//! that asks for them. rsui-tsx's doesn't, as that would change every app's controls and starting, so here an activation
//! context that asks for them is made the first time one's wanted, and is only active while one's shown. Nothing is linked
//! from the common controls, as the version without them doesn't have what's used.

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{COLORREF, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SYSTEMTIME, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleDC, CreateFontIndirectW, CreateRoundRectRgn, CreateSolidBrush, DT_CALCRECT, DT_CENTER,
    DT_NOPREFIX, DT_WORDBREAK, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, GetMonitorInfoW, HFONT, HGDIOBJ,
    MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, MonitorFromWindow, PAINTSTRUCT,
    SelectObject, SetBkMode, SetTextColor, SetWindowRgn, TRANSPARENT,
};
use windows::Win32::System::ApplicationInstallationAndServicing::{ACTCTXW, ActivateActCtx, CreateActCtxW, DeactivateActCtx};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Controls::{
    DTM_GETSYSTEMTIME, DTM_SETFORMATW, DTM_SETSYSTEMTIME, DTS_TIMEFORMAT, DTS_UPDOWN, GDT_VALID, GDTR_MAX, GDTR_MIN,
    ICC_DATE_CLASSES, INITCOMMONCONTROLSEX, MCM_GETCURSEL, MCM_GETMINREQRECT, MCM_SETCURSEL, MCM_SETRANGE,
    TASKDIALOG_BUTTON, TASKDIALOG_COMMON_BUTTON_FLAGS, TASKDIALOG_FLAGS, TASKDIALOGCONFIG, TD_WARNING_ICON,
    TDCBF_CANCEL_BUTTON, TDCBF_OK_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW,
    TDF_USE_COMMAND_LINKS,
};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow, SystemParametersInfoForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DialogBoxIndirectParamW,
    DispatchMessageW, EndDialog, GWL_EXSTYLE, GWL_STYLE, GWLP_USERDATA, GetClientRect, GetDlgItem, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, HTTRANSPARENT, IDCANCEL, IDOK, IsIconic,
    IsWindowVisible, KillTimer, LWA_ALPHA, MSG, MapDialogRect, NONCLIENTMETRICSW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SPI_GETNONCLIENTMETRICS, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW,
    SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_INITDIALOG, WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{HSTRING, PCWSTR, w};

use super::{Answer, Answered, Day, Popup, TimeOfDay};

/// A sheet's options are the task dialog's buttons from this number on, in order.
pub const FIRST_OPTION: i32 = 100;

/// What's asked for while a version 6 control is shown.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;

type TaskDialogIndirect =
    unsafe extern "system" fn(*const TASKDIALOGCONFIG, *mut i32, *mut i32, *mut windows::core::BOOL) -> windows::core::HRESULT;
type InitCommonControlsEx = unsafe extern "system" fn(*const INITCOMMONCONTROLSEX) -> windows::core::BOOL;

/// Version 6 of the common controls, loaded once.
struct Controls {
    context: isize,
    task_dialog: TaskDialogIndirect,
}

fn controls() -> Result<&'static Controls, String> {
    static LOADED: OnceLock<Result<Controls, String>> = OnceLock::new();
    LOADED.get_or_init(load_controls).as_ref().map_err(Clone::clone)
}

fn load_controls() -> Result<Controls, String> {
    // The manifest is only read while the context is made, so it's a file of this process's own, and gone after.
    let file = std::env::temp_dir().join(format!("rsui-controls-{}.manifest", std::process::id()));
    std::fs::write(&file, MANIFEST).map_err(|error| format!("the system's dialogs couldn't be set up: {error}"))?;
    let source = HSTRING::from(file.as_os_str());
    let asked = ACTCTXW { cbSize: size_of::<ACTCTXW>() as u32, lpSource: PCWSTR(source.as_ptr()), ..Default::default() };
    // SAFETY: the description and the file's name last through the call.
    let context = unsafe { CreateActCtxW(&raw const asked) };
    let _ = std::fs::remove_file(&file);
    let context = context.map_err(|error| format!("the system's dialogs couldn't be set up: {error}"))?;
    let context = context.0 as isize;
    let _active = Active::new(context)?;
    // SAFETY: the library stays loaded for good, and each function has the type it's given.
    unsafe {
        let library = LoadLibraryW(w!("comctl32.dll")).map_err(|error| error.to_string())?;
        let task_dialog = GetProcAddress(library, windows::core::s!("TaskDialogIndirect"))
            .ok_or("this version of Windows has no task dialogs")?;
        if let Some(init) = GetProcAddress(library, windows::core::s!("InitCommonControlsEx")) {
            let init: InitCommonControlsEx = std::mem::transmute(init);
            let classes =
                INITCOMMONCONTROLSEX { dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_DATE_CLASSES };
            let _ = init(&raw const classes);
        }
        Ok(Controls { context, task_dialog: std::mem::transmute::<unsafe extern "system" fn() -> isize, TaskDialogIndirect>(task_dialog) })
    }
}

/// The version 6 controls, for as long as this lasts, on this thread.
struct Active(usize);

impl Active {
    fn new(context: isize) -> Result<Active, String> {
        let mut cookie = 0;
        // SAFETY: the context is made once and kept for good.
        unsafe { ActivateActCtx(Some(HANDLE(context as *mut c_void)), &raw mut cookie) }.map_err(|error| error.to_string())?;
        Ok(Active(cookie))
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        // SAFETY: undoes what `new` did, on the same thread.
        let _ = unsafe { DeactivateActCtx(0, self.0) };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// What a window says it's called, which a popup belonging to it says too.
fn title_of(window: HWND) -> Option<Vec<u16>> {
    if window.is_invalid() {
        return None;
    }
    // SAFETY: the room is as long as it says; a window that's gone has no text.
    unsafe {
        let length = GetWindowTextLengthW(window);
        if length <= 0 {
            return None;
        }
        let mut text = vec![0u16; length as usize + 1];
        let got = GetWindowTextW(window, &mut text) as usize;
        text.truncate(got);
        text.push(0);
        Some(text)
    }
}

pub(super) fn ask(popup: Popup, answered: Answered) {
    let owner = crate::dialogs::current_window();
    if let Popup::Share { .. } = popup {
        answered(share(&popup, owner));
        return;
    }
    // If the thread can't be started, what's waiting is told so.
    let waiting = Arc::new(Mutex::new(Some(answered)));
    let owner = owner.0 as isize;
    let started = std::thread::Builder::new().name("rsui popup".into()).stack_size(8 * 1024 * 1024).spawn({
        let waiting = waiting.clone();
        move || {
            let answer = ask_on_this_thread(&popup, HWND(owner as *mut c_void));
            let answered = waiting.lock().ok().and_then(|mut waiting| waiting.take());
            if let Some(answered) = answered {
                answered(answer);
            }
        }
    });
    if let Err(error) = started {
        let answered = waiting.lock().ok().and_then(|mut waiting| waiting.take());
        if let Some(answered) = answered {
            answered(Err(error.to_string()));
        }
    }
}

/// [`ask`], on this thread, which waits until it's answered.
#[doc(hidden)]
pub fn ask_on_this_thread(popup: &Popup, owner: HWND) -> Result<Answer, String> {
    match popup {
        Popup::Confirm { title, message, confirm, cancel, destructive } => {
            let buttons: Vec<(i32, &str)> =
                [(IDOK.0, confirm.as_deref()), (IDCANCEL.0, cancel.as_deref())].into_iter().filter_map(|(id, text)| Some((id, text?))).collect();
            let mut common = TASKDIALOG_COMMON_BUTTON_FLAGS(0);
            if confirm.is_none() {
                common |= TDCBF_OK_BUTTON;
            }
            if cancel.is_none() {
                common |= TDCBF_CANCEL_BUTTON;
            }
            let pressed = task_dialog(TaskDialog {
                owner,
                title,
                message,
                buttons: &buttons,
                common,
                // Going ahead with what can't be undone is never what Enter does by itself.
                default: if *destructive { IDCANCEL.0 } else { IDOK.0 },
                flags: TASKDIALOG_FLAGS(0),
                warning: *destructive,
            })?;
            Ok(Answer::Confirmed(pressed == IDOK.0))
        }
        Popup::Alert { title, message, ok } => {
            let buttons: Vec<(i32, &str)> = ok.as_deref().map(|ok| (IDOK.0, ok)).into_iter().collect();
            task_dialog(TaskDialog {
                owner,
                title,
                message,
                buttons: &buttons,
                common: if ok.is_none() { TDCBF_OK_BUTTON } else { TASKDIALOG_COMMON_BUTTON_FLAGS(0) },
                default: IDOK.0,
                flags: TASKDIALOG_FLAGS(0),
                warning: false,
            })?;
            Ok(Answer::Done)
        }
        Popup::Sheet { title, options, cancel: _ } => {
            if options.is_empty() {
                return Err("a sheet needs something to choose".to_owned());
            }
            let buttons: Vec<(i32, &str)> =
                options.iter().enumerate().map(|(at, option)| (FIRST_OPTION + at as i32, option.as_str())).collect();
            // The options are command links, and so would a button of the app's own for cancelling be, so it's the
            // system's own, which says Cancel in the system's language.
            let pressed = task_dialog(TaskDialog {
                owner,
                title,
                message: "",
                buttons: &buttons,
                common: TDCBF_CANCEL_BUTTON,
                default: FIRST_OPTION,
                flags: TDF_USE_COMMAND_LINKS,
                warning: false,
            })?;
            let chosen = usize::try_from(pressed - FIRST_OPTION).ok().filter(|chosen| *chosen < options.len());
            Ok(Answer::Chosen(chosen))
        }
        Popup::PickDate { value, min, max } => {
            let picked = picker(owner, Picking::Day { value: *value, min: *min, max: *max })?;
            Ok(Answer::Day(picked.map(|time| Day { year: i32::from(time.wYear), month: time.wMonth as u8, day: time.wDay as u8 })))
        }
        Popup::PickTime { value } => {
            let picked = picker(owner, Picking::Time { value: *value })?;
            Ok(Answer::Time(picked.map(|time| TimeOfDay { hours: time.wHour as u8, minutes: time.wMinute as u8 })))
        }
        Popup::Share { .. } => share(popup, owner),
    }
}

struct TaskDialog<'a> {
    owner: HWND,
    title: &'a str,
    message: &'a str,
    buttons: &'a [(i32, &'a str)],
    common: TASKDIALOG_COMMON_BUTTON_FLAGS,
    default: i32,
    flags: TASKDIALOG_FLAGS,
    warning: bool,
}

/// Shows a task dialog, and says which button was pressed: `IDCANCEL` if it was closed.
fn task_dialog(asked: TaskDialog<'_>) -> Result<i32, String> {
    let controls = controls()?;
    let _active = Active::new(controls.context)?;
    // These last until it's been answered.
    let window_title = title_of(asked.owner);
    let title = wide(asked.title);
    let message = wide(asked.message);
    let texts: Vec<Vec<u16>> = asked.buttons.iter().map(|(_, text)| wide(text)).collect();
    let buttons: Vec<TASKDIALOG_BUTTON> = asked
        .buttons
        .iter()
        .zip(&texts)
        .map(|((id, _), text)| TASKDIALOG_BUTTON { nButtonID: *id, pszButtonText: PCWSTR(text.as_ptr()) })
        .collect();
    let some = |text: &Vec<u16>| if text.len() > 1 { PCWSTR(text.as_ptr()) } else { PCWSTR::null() };
    let mut config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: asked.owner,
        dwFlags: asked.flags | TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW,
        dwCommonButtons: asked.common,
        pszWindowTitle: window_title.as_ref().map_or(PCWSTR::null(), |title| PCWSTR(title.as_ptr())),
        pszMainInstruction: some(&title),
        pszContent: some(&message),
        cButtons: buttons.len() as u32,
        pButtons: if buttons.is_empty() { std::ptr::null() } else { buttons.as_ptr() },
        nDefaultButton: asked.default,
        ..Default::default()
    };
    if asked.warning {
        config.Anonymous1.pszMainIcon = TD_WARNING_ICON;
    }
    let mut pressed = 0;
    // SAFETY: everything the description points to lasts through the call, which is the function's own type.
    let done = unsafe { (controls.task_dialog)(&raw const config, &raw mut pressed, std::ptr::null_mut(), std::ptr::null_mut()) };
    done.ok().map_err(|error| error.to_string())?;
    Ok(pressed)
}

/// What MessageBox's buttons say, in the system's language.
fn button_text(id: i32, otherwise: &str) -> Vec<u16> {
    type GetString = unsafe extern "system" fn(u32) -> PCWSTR;
    // SAFETY: user32 is loaded for good, the function has this type, and what it gives is the system's, which lasts.
    unsafe {
        let found = GetModuleHandleW(w!("user32.dll"))
            .ok()
            .and_then(|user| GetProcAddress(user, windows::core::s!("MB_GetString")));
        if let Some(found) = found {
            let get: GetString = std::mem::transmute(found);
            let text = get((id - 1) as u32);
            if !text.is_null()
                && let Ok(text) = text.to_string()
            {
                return wide(&text);
            }
        }
    }
    wide(otherwise)
}

/// What a picker starts from.
#[derive(Clone, Copy)]
enum Picking {
    Day { value: Option<Day>, min: Option<Day>, max: Option<Day> },
    Time { value: Option<TimeOfDay> },
}

/// The picker's state, which its dialog's procedure reaches through the window.
struct Picker {
    picking: Picking,
    picked: Option<SYSTEMTIME>,
}

const PICKER_CONTROL: i32 = 100;

fn system_day(day: Day) -> SYSTEMTIME {
    SYSTEMTIME { wYear: day.year.clamp(1601, 30827) as u16, wMonth: u16::from(day.month), wDay: u16::from(day.day), ..Default::default() }
}

/// Lays out a dialog's template, as `DialogBoxIndirectParamW` wants it: the dialog, then each control, each on a whole
/// number of 32-bit words.
fn picker_template(time: bool, title: &[u16]) -> Vec<u32> {
    let mut units: Vec<u16> = Vec::new();
    let long = |units: &mut Vec<u16>, value: u32| units.extend([value as u16, (value >> 16) as u16]);
    let text = |units: &mut Vec<u16>, text: &[u16]| {
        units.extend(text.iter().copied().take_while(|unit| *unit != 0));
        units.push(0);
    };
    let align = |units: &mut Vec<u16>| {
        if units.len() % 2 == 1 {
            units.push(0);
        }
    };
    const DS_SETFONT: u32 = 0x40;
    const DS_MODALFRAME: u32 = 0x80;
    const WS_CAPTION: u32 = 0x00C0_0000;
    const WS_SYSMENU: u32 = 0x0008_0000;
    const WS_CHILD: u32 = 0x4000_0000;
    const WS_VISIBLE: u32 = 0x1000_0000;
    const WS_TABSTOP: u32 = 0x0001_0000;
    const BS_DEFPUSHBUTTON: u32 = 1;
    long(&mut units, WS_POPUP.0 | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME | DS_SETFONT);
    long(&mut units, 0);
    // Three controls, and a place and size that are set once its controls have said how big they are.
    units.extend([3, 0, 0, 160, 120]);
    units.extend([0, 0]);
    text(&mut units, title);
    units.push(9);
    text(&mut units, &wide("Segoe UI"));
    let control = if time { wide("SysDateTimePick32") } else { wide("SysMonthCal32") };
    let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | if time { DTS_TIMEFORMAT | DTS_UPDOWN } else { 0 };
    let ok = button_text(IDOK.0, "OK");
    let cancel = button_text(IDCANCEL.0, "Cancel");
    for (id, class, style, title) in [
        (PICKER_CONTROL, Some(control.as_slice()), style, &[0u16][..]),
        (IDOK.0, None, WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON, ok.as_slice()),
        (IDCANCEL.0, None, WS_CHILD | WS_VISIBLE | WS_TABSTOP, cancel.as_slice()),
    ] {
        align(&mut units);
        long(&mut units, style);
        long(&mut units, 0);
        units.extend([0, 0, 50, 14, id as u16]);
        match class {
            Some(class) => text(&mut units, class),
            // A button, by the system's number for it.
            None => units.extend([0xFFFF, 0x0080]),
        }
        text(&mut units, title);
        units.push(0);
    }
    align(&mut units);
    units.chunks(2).map(|pair| u32::from(pair[0]) | u32::from(pair[1]) << 16).collect()
}

/// Shows a picker, and says what was picked, or nothing if it was cancelled.
fn picker(owner: HWND, picking: Picking) -> Result<Option<SYSTEMTIME>, String> {
    let controls = controls()?;
    let _active = Active::new(controls.context)?;
    let title = title_of(owner).unwrap_or_else(|| vec![0]);
    let template = picker_template(matches!(picking, Picking::Time { .. }), &title);
    let mut state = Picker { picking, picked: None };
    // SAFETY: the template's as Windows lays one out, and the state lasts until the dialog's gone.
    let done = unsafe {
        DialogBoxIndirectParamW(
            None,
            template.as_ptr().cast(),
            (!owner.is_invalid()).then_some(owner),
            Some(picker_procedure),
            LPARAM((&raw mut state) as isize),
        )
    };
    if done == -1 {
        return Err(windows::core::Error::from_thread().to_string());
    }
    Ok(state.picked)
}

unsafe extern "system" fn picker_procedure(dialog: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    // SAFETY: the state's what `picker` gave, which lasts while the dialog does, and each message is sent as Windows has it.
    unsafe {
        match message {
            WM_INITDIALOG => {
                SetWindowLongPtrW(dialog, GWLP_USERDATA, lparam.0);
                let state = &mut *(lparam.0 as *mut Picker);
                lay_out_picker(dialog, state.picking);
                1
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as i32;
                let state = GetWindowLongPtrW(dialog, GWLP_USERDATA) as *mut Picker;
                if id == IDOK.0 && !state.is_null() {
                    let state = &mut *state;
                    let control = GetDlgItem(Some(dialog), PICKER_CONTROL).unwrap_or_default();
                    let mut picked = SYSTEMTIME::default();
                    // The two have the same number, but not the same answer.
                    let time = matches!(state.picking, Picking::Time { .. });
                    let asked = if time { DTM_GETSYSTEMTIME } else { MCM_GETCURSEL };
                    let got = SendMessageW(control, asked, None, Some(LPARAM((&raw mut picked) as isize)));
                    let valid = if time { got.0 == GDT_VALID.0 as isize } else { got.0 != 0 };
                    state.picked = valid.then_some(picked);
                    let _ = EndDialog(dialog, 1);
                    1
                } else if id == IDCANCEL.0 || id == IDOK.0 {
                    let _ = EndDialog(dialog, 0);
                    1
                } else {
                    0
                }
            }
            _ => 0,
        }
    }
}

/// Starts the picker's control from what it was given, sizes it as it says it needs, puts the buttons under it, and the
/// dialog around them, over the window it belongs to.
///
/// # Safety
/// `dialog` is the picker's dialog, in its `WM_INITDIALOG`.
unsafe fn lay_out_picker(dialog: HWND, picking: Picking) {
    // SAFETY: as the caller promises; what's sent points to what lasts through each call.
    unsafe {
        let control = GetDlgItem(Some(dialog), PICKER_CONTROL).unwrap_or_default();
        let units = |x: i32, y: i32| {
            let mut rect = RECT { left: 0, top: 0, right: x, bottom: y };
            let _ = MapDialogRect(dialog, &raw mut rect);
            (rect.right, rect.bottom)
        };
        let (margin, gap) = (units(7, 7), units(4, 4).0);
        let button = units(50, 14);
        let size = match picking {
            Picking::Day { value, min, max } => {
                if min.is_some() || max.is_some() {
                    let range = [min.map(system_day).unwrap_or_default(), max.map(system_day).unwrap_or_default()];
                    let which = if min.is_some() { GDTR_MIN } else { 0 } | if max.is_some() { GDTR_MAX } else { 0 };
                    SendMessageW(control, MCM_SETRANGE, Some(WPARAM(which as usize)), Some(LPARAM(range.as_ptr() as isize)));
                }
                if let Some(value) = value {
                    let value = system_day(value);
                    SendMessageW(control, MCM_SETCURSEL, None, Some(LPARAM((&raw const value) as isize)));
                }
                let mut needed = RECT::default();
                SendMessageW(control, MCM_GETMINREQRECT, None, Some(LPARAM((&raw mut needed) as isize)));
                (needed.right - needed.left, needed.bottom - needed.top)
            }
            Picking::Time { value } => {
                if let Some(format) = short_time_format() {
                    SendMessageW(control, DTM_SETFORMATW, None, Some(LPARAM(format.as_ptr() as isize)));
                }
                if let Some(value) = value {
                    let value = SYSTEMTIME {
                        wYear: 2000,
                        wMonth: 1,
                        wDay: 1,
                        wHour: u16::from(value.hours),
                        wMinute: u16::from(value.minutes),
                        ..Default::default()
                    };
                    SendMessageW(control, DTM_SETSYSTEMTIME, Some(WPARAM(GDT_VALID.0 as usize)), Some(LPARAM((&raw const value) as isize)));
                }
                units(90, 14)
            }
        };
        let width = size.0.max(2 * button.0 + gap);
        let _ = SetWindowPos(control, None, margin.0, margin.1, width, size.1, SWP_NOZORDER | SWP_NOACTIVATE);
        let buttons_at = margin.1 + size.1 + margin.1;
        let right = margin.0 + width;
        for (id, left) in [(IDOK.0, right - 2 * button.0 - gap), (IDCANCEL.0, right - button.0)] {
            if let Ok(button_window) = GetDlgItem(Some(dialog), id) {
                let _ = SetWindowPos(button_window, None, left, buttons_at, button.0, button.1, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        let mut outside = RECT { left: 0, top: 0, right: right + margin.0, bottom: buttons_at + button.1 + margin.1 };
        let style = WINDOW_STYLE(GetWindowLongPtrW(dialog, GWL_STYLE) as u32);
        let extended = WINDOW_EX_STYLE(GetWindowLongPtrW(dialog, GWL_EXSTYLE) as u32);
        let _ = AdjustWindowRectEx(&raw mut outside, style, false, extended);
        let (width, height) = (outside.right - outside.left, outside.bottom - outside.top);
        let owner = windows::Win32::UI::WindowsAndMessaging::GetWindow(dialog, windows::Win32::UI::WindowsAndMessaging::GW_OWNER).unwrap_or_default();
        let (x, y) = centered_over(owner, width, height);
        let _ = SetWindowPos(dialog, None, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// Where something this big goes to be in the middle of `window`, on the screen it's on, or of the main screen.
fn centered_over(window: HWND, width: i32, height: i32) -> (i32, i32) {
    // SAFETY: these only look things up, into what lasts through each call.
    unsafe {
        let mut area = RECT::default();
        let monitor = if window.is_invalid() || IsIconic(window).as_bool() || !IsWindowVisible(window).as_bool() {
            MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY)
        } else {
            let _ = GetWindowRect(window, &raw mut area);
            MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST)
        };
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &raw mut info);
        let work = info.rcWork;
        if area.right <= area.left {
            area = work;
        }
        let x = (area.left + area.right - width) / 2;
        let y = (area.top + area.bottom - height) / 2;
        (x.clamp(work.left, (work.right - width).max(work.left)), y.clamp(work.top, (work.bottom - height).max(work.top)))
    }
}

/// How the person's set times to be written, without seconds, as a time control takes it.
fn short_time_format() -> Option<Vec<u16>> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocaleInfoEx(locale: PCWSTR, kind: u32, data: *mut u16, length: i32) -> i32;
    }
    const LOCALE_SSHORTTIME: u32 = 0x79;
    let mut format = [0u16; 80];
    // SAFETY: the room is as long as it says; no name is the person's own settings.
    let length = unsafe { GetLocaleInfoEx(PCWSTR::null(), LOCALE_SSHORTTIME, format.as_mut_ptr(), format.len() as i32) };
    (length > 1).then(|| format[..length as usize].to_vec())
}

// The share sheet

thread_local! {
    /// What the share sheet that was shown last is told when it asks what's being shared, which goes once another is shown.
    static SHARING: std::cell::RefCell<Option<(windows::ApplicationModel::DataTransfer::DataTransferManager, i64)>> =
        const { std::cell::RefCell::new(None) };
}

/// Brings up the system's share sheet for the window, which has to be shown from the window's own thread. It's done once
/// it's up; what's shared where isn't said.
fn share(popup: &Popup, owner: HWND) -> Result<Answer, String> {
    use windows::ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager};
    use windows::Foundation::{TypedEventHandler, Uri};
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::UI::Shell::IDataTransferManagerInterop;
    let Popup::Share { title, text, url } = popup else {
        return Err("that isn't something to share".to_owned());
    };
    if owner.is_invalid() {
        return Err("there's no window to share from".to_owned());
    }
    // The share sheet says what it's sharing, and has to have something to say.
    let title = if title.is_empty() {
        title_of(owner).map(|title| String::from_utf16_lossy(&title[..title.len() - 1])).unwrap_or_else(|| "Share".to_owned())
    } else {
        title.clone()
    };
    let (title, text, url) = (HSTRING::from(title), HSTRING::from(text.as_str()), url.clone());
    let failed = |error: windows::core::Error| format!("the share sheet couldn't be shown: {}", error.message());
    // SAFETY: the window is the app's, on this thread, and what's handed over lasts through each call.
    unsafe {
        // Already done on a thread that has the window, which is all that's wanted.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let interop = windows::core::factory::<DataTransferManager, IDataTransferManagerInterop>().map_err(failed)?;
        let manager: DataTransferManager = interop.GetForWindow(owner).map_err(failed)?;
        let token = manager
            .DataRequested(&TypedEventHandler::new(
                move |_, asked: windows::core::Ref<DataRequestedEventArgs>| {
                    let data = asked.ok()?.Request()?.Data()?;
                    data.Properties()?.SetTitle(&title)?;
                    if !text.is_empty() {
                        data.SetText(&text)?;
                    }
                    if !url.is_empty() {
                        data.SetWebLink(&Uri::CreateUri(&HSTRING::from(url.as_str()))?)?;
                    }
                    Ok(())
                },
            ))
            .map_err(failed)?;
        let before = SHARING.with(|sharing| sharing.borrow_mut().replace((manager, token)));
        if let Some((manager, token)) = before {
            let _ = manager.RemoveDataRequested(token);
        }
        interop.ShowShareUIForWindow(owner).map_err(failed)?;
    }
    Ok(Answer::Done)
}

// The toast

/// The toast that's showing, which goes when another comes.
static SHOWING: AtomicIsize = AtomicIsize::new(0);

pub(super) fn toast(text: &str, long: bool) -> Result<(), String> {
    let owner = crate::dialogs::current_window().0 as isize;
    let text = text.to_owned();
    let shown_for = Duration::from_millis(if long { 3500 } else { 2000 });
    std::thread::Builder::new()
        .name("rsui toast".into())
        .spawn(move || {
            if let Err(error) = toast_on_this_thread(&text, shown_for, HWND(owner as *mut c_void)) {
                eprintln!("rsui: the toast couldn't be shown: {error}");
            }
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
}

struct Toast {
    text: Vec<u16>,
    font: HFONT,
    padding: (i32, i32),
    started: Instant,
    shown_for: Duration,
}

const FADE_IN: Duration = Duration::from_millis(150);
const FADE_OUT: Duration = Duration::from_millis(300);
/// How opaque it is once it's faded in, out of 255.
const OPAQUE: f32 = 235.0;
const TICK: usize = 1;

/// Shows a toast, under the middle of `owner`, or of the main screen, and waits on this thread until it's gone, which is
/// once it's been shown this long and faded away, or another's come.
#[doc(hidden)]
pub fn toast_on_this_thread(text: &str, shown_for: Duration, owner: HWND) -> Result<(), String> {
    // SAFETY: the window's made, shown and let go of here, and its state is let go of once it's gone; each call has what
    // it points to for as long as it's needed.
    unsafe {
        let instance: HINSTANCE = GetModuleHandleW(None).map_err(|error| error.to_string())?.into();
        let class = w!("rsui toast");
        let description = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(toast_procedure),
            hInstance: instance,
            lpszClassName: class,
            ..Default::default()
        };
        // It's there already for a toast after the first.
        RegisterClassExW(&raw const description);
        let dpi = if owner.is_invalid() { GetDpiForSystem() } else { GetDpiForWindow(owner) }.max(96) as i32;
        let scaled = |pixels: i32| pixels * dpi / 96;
        let mut metrics = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
        let _ = SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS.0,
            metrics.cbSize,
            Some((&raw mut metrics).cast()),
            0,
            dpi as u32,
        );
        let font = CreateFontIndirectW(&raw const metrics.lfMessageFont);
        let mut said: Vec<u16> = text.encode_utf16().collect();
        // How big the text is, wrapped to at most this wide.
        let mut owner_area = RECT::default();
        let owned = !owner.is_invalid() && IsWindowVisible(owner).as_bool() && !IsIconic(owner).as_bool();
        if owned {
            let _ = GetWindowRect(owner, &raw mut owner_area);
        }
        let widest = if owned { (owner_area.right - owner_area.left - scaled(48)).clamp(scaled(120), scaled(560)) } else { scaled(560) };
        let measuring = CreateCompatibleDC(None);
        let before = SelectObject(measuring, HGDIOBJ(font.0));
        let mut room = RECT { left: 0, top: 0, right: widest, bottom: 0 };
        DrawTextW(measuring, &mut said, &raw mut room, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX | DT_CENTER);
        SelectObject(measuring, before);
        let _ = DeleteDC(measuring);
        let padding = (scaled(20), scaled(12));
        let (width, height) = (room.right + 2 * padding.0, room.bottom + 2 * padding.1);
        // Under the middle of the window it's for, a little up from its bottom, and on the screen it's on.
        let (x, _) = centered_over(if owned { owner } else { HWND::default() }, width, height);
        let y = if owned {
            owner_area.bottom - height - scaled(56)
        } else {
            let monitor = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(monitor, &raw mut info);
            info.rcWork.bottom - height - scaled(56)
        };
        let state = Box::into_raw(Box::new(Toast { text: said, font, padding, started: Instant::now(), shown_for }));
        let window = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class,
            &HSTRING::from(text),
            WS_POPUP,
            x,
            y,
            width,
            height,
            owned.then_some(owner),
            None,
            Some(instance),
            Some(state.cast()),
        );
        let window = match window {
            Ok(window) => window,
            Err(error) => {
                drop(Box::from_raw(state));
                let _ = DeleteObject(HGDIOBJ(font.0));
                return Err(error.to_string());
            }
        };
        SetWindowLongPtrW(window, GWLP_USERDATA, state as isize);
        let corner = scaled(16);
        SetWindowRgn(window, Some(CreateRoundRectRgn(0, 0, width + 1, height + 1, corner, corner)), false);
        let _ = SetLayeredWindowAttributes(window, COLORREF(0), 0, LWA_ALPHA);
        let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
        SetTimer(Some(window), TICK, 16, None);
        let before = SHOWING.swap(window.0 as isize, Ordering::AcqRel);
        if before != 0 {
            let _ = PostMessageW(Some(HWND(before as *mut c_void)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        let mut message = MSG::default();
        while GetMessageW(&raw mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&raw const message);
            DispatchMessageW(&raw const message);
        }
        let _ = SHOWING.compare_exchange(window.0 as isize, 0, Ordering::AcqRel, Ordering::Acquire);
        drop(Box::from_raw(state));
        let _ = DeleteObject(HGDIOBJ(font.0));
    }
    Ok(())
}

unsafe extern "system" fn toast_procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: the state is what `toast_on_this_thread` made, which lasts until the window's gone and its loop has ended.
    unsafe {
        let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *const Toast;
        match message {
            // Clicks go through it to what's under it.
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            WM_TIMER if !state.is_null() => {
                let state = &*state;
                let shown = state.started.elapsed();
                let out_from = FADE_IN + state.shown_for;
                let opacity = if shown < FADE_IN {
                    shown.as_secs_f32() / FADE_IN.as_secs_f32()
                } else if shown < out_from {
                    1.0
                } else {
                    1.0 - (shown - out_from).as_secs_f32() / FADE_OUT.as_secs_f32()
                };
                if opacity <= 0.0 {
                    let _ = DestroyWindow(window);
                } else {
                    let _ = SetLayeredWindowAttributes(window, COLORREF(0), (opacity.min(1.0) * OPAQUE) as u8, LWA_ALPHA);
                }
                LRESULT(0)
            }
            WM_PAINT if !state.is_null() => {
                let state = &*state;
                let mut paint = PAINTSTRUCT::default();
                let drawing = BeginPaint(window, &raw mut paint);
                let mut area = RECT::default();
                let _ = GetClientRect(window, &raw mut area);
                let brush = CreateSolidBrush(COLORREF(0x0030_3030));
                FillRect(drawing, &raw const area, brush);
                let _ = DeleteObject(HGDIOBJ(brush.0));
                SetBkMode(drawing, TRANSPARENT);
                SetTextColor(drawing, COLORREF(0x00FF_FFFF));
                let before = SelectObject(drawing, HGDIOBJ(state.font.0));
                let mut room = RECT {
                    left: area.left + state.padding.0,
                    top: area.top + state.padding.1,
                    right: area.right - state.padding.0,
                    bottom: area.bottom - state.padding.1,
                };
                let mut text = state.text.clone();
                DrawTextW(drawing, &mut text, &raw mut room, DT_WORDBREAK | DT_NOPREFIX | DT_CENTER);
                SelectObject(drawing, before);
                let _ = EndPaint(window, &raw const paint);
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(window);
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(window), TICK);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}
