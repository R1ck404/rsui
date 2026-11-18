//! What a program asks of the desktop, apart from its windows: keys that work from anywhere, an icon in the tray, notifications, a
//! menu bar, sound, starting at login, and the like.
//!
//! These have effects that can be seen and heard, or that last, so nothing here happens by itself, and everything has to be on the
//! thread with the windows. What can be worked out without doing it is apart from what does it, so that tests can check it.
//! Nothing here is loaded, and no window's made for it, until something's asked for.

mod keys;
mod more;
mod sound;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::path::Path;
use std::rc::Rc;
use std::sync::OnceLock;

use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFY_ICON_MESSAGE, NOTIFYICONDATAW, SetCurrentProcessExplicitAppUserModelID,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HSTRING, PCWSTR, s, w};

pub use keys::{Keys, parse_keys};
pub use more::*;
pub use sound::{
    SoundId, close_sound, open_sound, pause_sound, seek_sound, set_sound_volume, sound_length,
    sound_position, stop_sound,
};

/// Something to run when something happens.
pub type Action = Rc<dyn Fn()>;

/// What the system sends about the tray icon.
const TRAY_MESSAGE: u32 = WM_APP + 20;
/// Which icon each is, as they're told apart by number.
const TRAY_ICON: u32 = 1;
const NOTIFICATION_ICON: u32 = 2;
/// Where the numbers for what's in the tray's menu start, with those for the menu bar below.
const TRAY_MENU_FROM: u16 = 0x8000;
/// What the system says when a sound's stopped, and that it got to the end.
const MM_MCINOTIFY: u32 = 0x3B9;
const MCI_NOTIFY_SUCCESSFUL: usize = 1;
const NIN_BALLOONHIDE: u32 = WM_USER + 3;
const NIN_BALLOONTIMEOUT: u32 = WM_USER + 4;
const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;
const PBT_APMSUSPEND: usize = 4;
const PBT_APMRESUMEAUTOMATIC: usize = 18;
const PBT_APMPOWERSTATUSCHANGE: usize = 10;

/// An item in a menu.
#[derive(Clone, Default)]
pub struct MenuItem {
    pub label: String,
    /// Keys that do the same as this, like `ctrl+o`, which only work in a menu bar. A tray's menu just shows them.
    pub shortcut: Option<String>,
    pub disabled: bool,
    pub checked: bool,
    pub separator: bool,
    /// What opens from this.
    pub items: Vec<MenuItem>,
    pub action: Option<Action>,
}

/// An icon in the tray.
#[derive(Clone, Default)]
pub struct Tray {
    pub tooltip: String,
    /// An `.ico` file or a picture, or else the program's own icon.
    pub icon: Option<Vec<u8>>,
    /// What clicking on it does. Clicking with the other button opens the menu.
    pub on_click: Option<Action>,
    pub menu: Vec<MenuItem>,
}

/// A screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Display {
    /// Where it is and how big, in physical pixels.
    pub bounds: (i32, i32, i32, i32),
    /// The same, without the taskbar.
    pub work_area: (i32, i32, i32, i32),
    /// How much bigger than 96 dots an inch things are drawn on it.
    pub scale: f32,
    pub primary: bool,
}

#[derive(Default)]
struct Desk {
    window: Option<HWND>,
    shortcuts: HashMap<i32, Action>,
    next_shortcut: i32,
    /// What's in the menus, by the number each item goes by.
    menu_actions: HashMap<u16, Action>,
    bar: Option<HMENU>,
    bar_shortcuts: Option<HACCEL>,
    /// The window the menu bar goes on, once it's there.
    bar_window: Option<HWND>,
    tray: Option<Tray>,
    tray_icon: Option<HICON>,
    /// What clicking on the last notification does.
    notification: Option<Action>,
    /// Whether there's an icon only for showing a notification, as one has to come from an icon.
    notification_icon: bool,
    second_instance: Option<Rc<dyn Fn(Vec<String>)>>,
    power: Option<Rc<dyn Fn(&'static str)>>,
    sounds: HashMap<SoundId, Action>,
    /// What's told when something's put on the clipboard, a setting of the system's changes, or the screens do.
    clipboard_changed: Option<Action>,
    settings_changed: Option<Action>,
    displays_changed: Option<Action>,
    /// What's told when a device is plugged in or taken out.
    devices_changed: Option<Action>,
}

thread_local! {
    static DESK: RefCell<Desk> = RefCell::default();
}

fn with_desk<R>(run: impl FnOnce(&mut Desk) -> R) -> R {
    DESK.with(|desk| run(&mut desk.borrow_mut()))
}

const CLASS: PCWSTR = w!("rsui-desktop");

/// A window that's never shown, which is what the system sends all of this to. It isn't one that only takes messages, as those
/// don't hear when the taskbar's come back, and can't own a menu.
fn desk_window() -> Result<HWND, String> {
    if let Some(window) = with_desk(|desk| desk.window) {
        return Ok(window);
    }
    static MADE: std::sync::Once = std::sync::Once::new();
    // SAFETY: these make a window on this thread, with what they're given lasting through the calls. It's never shown.
    let window = unsafe {
        let instance = GetModuleHandleW(None).map_err(|error| error.to_string())?;
        MADE.call_once(|| {
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                lpszClassName: CLASS,
                ..Default::default()
            });
        });
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS,
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(|error| error.to_string())?
    };
    with_desk(|desk| desk.window = Some(window));
    Ok(window)
}

fn taskbar_came_back() -> u32 {
    static NUMBER: OnceLock<u32> = OnceLock::new();
    // SAFETY: the name ends in a terminator.
    *NUMBER.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_HOTKEY => {
            let action = with_desk(|desk| desk.shortcuts.get(&(wparam.0 as i32)).cloned());
            if let Some(action) = action {
                action();
            }
            LRESULT(0)
        }
        TRAY_MESSAGE => {
            tray_message(wparam.0 as u32, lparam.0 as u32 & 0xFFFF);
            LRESULT(0)
        }
        WM_COMMAND => {
            menu_command((wparam.0 & 0xFFFF) as u16);
            LRESULT(0)
        }
        MM_MCINOTIFY => {
            if wparam.0 == MCI_NOTIFY_SUCCESSFUL {
                sound_ended(lparam.0 as u32);
            }
            LRESULT(0)
        }
        WM_COPYDATA => {
            let copied = lparam.0 as *const COPYDATASTRUCT;
            // SAFETY: this message comes with this, and what it points to lasts until this returns.
            let text = unsafe {
                if copied.is_null() || (*copied).lpData.is_null() {
                    String::new()
                } else {
                    let bytes = std::slice::from_raw_parts(
                        (*copied).lpData as *const u8,
                        (*copied).cbData as usize,
                    );
                    String::from_utf8_lossy(bytes).into_owned()
                }
            };
            let hear = with_desk(|desk| desk.second_instance.clone());
            if let Some(hear) = hear {
                hear(split_arguments(&text));
            }
            LRESULT(1)
        }
        WM_POWERBROADCAST => {
            let what = match wparam.0 {
                PBT_APMSUSPEND => Some("suspend"),
                PBT_APMRESUMEAUTOMATIC => Some("resume"),
                PBT_APMPOWERSTATUSCHANGE => Some("power-changed"),
                _ => None,
            };
            let hear = with_desk(|desk| desk.power.clone());
            if let (Some(what), Some(hear)) = (what, hear) {
                hear(what);
            }
            LRESULT(1)
        }
        WM_CLIPBOARDUPDATE | WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED | WM_DISPLAYCHANGE | WM_DPICHANGED
        | WM_DEVICECHANGE => {
            more::heard(message);
            // SAFETY: these are the arguments this was given.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        _ if message == taskbar_came_back() => {
            // Explorer started again, which has forgotten every icon.
            let _ = show_tray_icon(NIM_ADD);
            LRESULT(0)
        }
        // SAFETY: these are the arguments this was given.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn run_menu_action(id: u16) -> bool {
    let action = with_desk(|desk| desk.menu_actions.get(&id).cloned());
    let found = action.is_some();
    if let Some(action) = action {
        action();
    }
    found
}

/// What a click on something in the menu bar comes to, for the window it's on to pass on.
pub(crate) fn menu_command(id: u16) -> bool {
    run_menu_action(id)
}

fn sound_ended(device: u32) {
    if let Some(id) = sound::sound_of_device(device) {
        let action = with_desk(|desk| desk.sounds.get(&id).cloned());
        if let Some(action) = action {
            action();
        }
    }
}

fn split_arguments(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n').map(str::to_owned).collect()
}

// Global shortcuts

fn modifiers_of(keys: Keys) -> HOT_KEY_MODIFIERS {
    // Holding the keys down would otherwise say so over and over.
    let mut modifiers = MOD_NOREPEAT;
    if keys.control {
        modifiers |= MOD_CONTROL;
    }
    if keys.alt {
        modifiers |= MOD_ALT;
    }
    if keys.shift {
        modifiers |= MOD_SHIFT;
    }
    if keys.windows {
        modifiers |= MOD_WIN;
    }
    modifiers
}

/// Runs `action` whenever these keys are pressed, whatever's being used at the time, until it's removed. It gives what
/// [`remove_global_shortcut`] takes. Only one program can have any set of keys, so this can fail.
pub fn add_global_shortcut(keys: &str, action: Action) -> Result<i32, String> {
    let parsed = parse_keys(keys)?;
    let window = desk_window()?;
    let id = with_desk(|desk| {
        desk.next_shortcut += 1;
        desk.next_shortcut
    });
    // SAFETY: the window's on this thread.
    unsafe {
        RegisterHotKey(
            Some(window),
            id,
            modifiers_of(parsed),
            u32::from(parsed.key),
        )
    }
    .map_err(|_| format!("{keys} is already in use by something else"))?;
    with_desk(|desk| desk.shortcuts.insert(id, action));
    Ok(id)
}

pub fn remove_global_shortcut(id: i32) {
    let window = with_desk(|desk| {
        desk.shortcuts.remove(&id);
        desk.window
    });
    if let Some(window) = window {
        // SAFETY: the window's on this thread, and this only gives an error if it wasn't there.
        unsafe {
            let _ = UnregisterHotKey(Some(window), id);
        }
    }
}

// Menus

/// The text an item shows, with its keys lined up on the right.
fn menu_text(item: &MenuItem) -> String {
    match &item.shortcut {
        Some(keys) => format!("{}\t{keys}", item.label),
        None => item.label.clone(),
    }
}

/// Makes a menu from these, giving each item that does something a number, from `next`.
fn build_menu(
    items: &[MenuItem],
    menu: HMENU,
    next: &mut u16,
    actions: &mut HashMap<u16, Action>,
    table: &mut Vec<ACCEL>,
) -> Result<(), String> {
    for item in items {
        // SAFETY: the menu's one that's just been made, and the text lasts through each call.
        unsafe {
            if item.separator {
                AppendMenuW(menu, MF_SEPARATOR, 0, None).map_err(|error| error.to_string())?;
                continue;
            }
            let text = HSTRING::from(menu_text(item).as_str());
            let mut flags = MF_STRING;
            if item.disabled {
                flags |= MF_GRAYED;
            }
            if item.checked {
                flags |= MF_CHECKED;
            }
            if !item.items.is_empty() {
                let inner = CreatePopupMenu().map_err(|error| error.to_string())?;
                build_menu(&item.items, inner, next, actions, table)?;
                AppendMenuW(menu, flags | MF_POPUP, inner.0 as usize, &text)
                    .map_err(|error| error.to_string())?;
                continue;
            }
            let id = *next;
            *next = next.wrapping_add(1);
            if let Some(action) = &item.action {
                actions.insert(id, action.clone());
            }
            if let (Some(keys), false) = (&item.shortcut, item.disabled) {
                let parsed = parse_keys(keys)?;
                let mut flags = FVIRTKEY;
                if parsed.control {
                    flags |= FCONTROL;
                }
                if parsed.alt {
                    flags |= FALT;
                }
                if parsed.shift {
                    flags |= FSHIFT;
                }
                table.push(ACCEL {
                    fVirt: flags,
                    key: parsed.key,
                    cmd: id,
                });
            }
            AppendMenuW(menu, flags, usize::from(id), &text).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// Puts a menu bar on the main window, or takes it off with an empty list.
pub fn set_menu_bar(items: &[MenuItem]) -> Result<(), String> {
    let mut actions = HashMap::new();
    let mut table = Vec::new();
    let mut next = 1;
    let bar = if items.is_empty() {
        None
    } else {
        // SAFETY: this only makes a menu, which is let go of along with the old one when the next takes its place.
        let bar = unsafe { CreateMenu() }.map_err(|error| error.to_string())?;
        build_menu(items, bar, &mut next, &mut actions, &mut table)?;
        Some(bar)
    };
    // SAFETY: the table's as long as it says.
    let shortcuts = (!table.is_empty())
        .then(|| unsafe { CreateAcceleratorTableW(&table) }.ok())
        .flatten();
    let (old_bar, old_shortcuts, window) = with_desk(|desk| {
        desk.menu_actions.retain(|id, _| *id >= TRAY_MENU_FROM);
        desk.menu_actions.extend(actions);
        (
            std::mem::replace(&mut desk.bar, bar),
            std::mem::replace(&mut desk.bar_shortcuts, shortcuts),
            desk.bar_window,
        )
    });
    if let Some(window) = window {
        attach_menu_bar(window);
    }
    // SAFETY: nothing's using these now.
    unsafe {
        if let Some(old) = old_bar {
            let _ = DestroyMenu(old);
        }
        if let Some(old) = old_shortcuts {
            let _ = DestroyAcceleratorTable(old);
        }
    }
    Ok(())
}

/// Says which window the menu bar goes on, as it can be asked for before that window's there.
pub(crate) fn attach_menu_bar(window: HWND) {
    let bar = with_desk(|desk| {
        desk.bar_window = Some(window);
        desk.bar
    });
    // SAFETY: the window's on this thread, and doing this to one that's gone only gives an error.
    unsafe {
        let _ = SetMenu(window, bar);
        let _ = DrawMenuBar(window);
    }
}

/// Whether this was the keys of something in the menu bar, which has dealt with it.
pub(crate) fn translate_menu_keys(message: &MSG) -> bool {
    let found = DESK.with(|desk| {
        let desk = desk.borrow();
        desk.bar_shortcuts.zip(desk.bar_window)
    });
    let Some((shortcuts, window)) = found else {
        return false;
    };
    // SAFETY: the table and window are this thread's, and the message is as it came.
    unsafe { TranslateAcceleratorW(window, shortcuts, message) != 0 }
}

// The tray, and notifications

type NotifyIcon = unsafe extern "system" fn(NOTIFY_ICON_MESSAGE, *const NOTIFYICONDATAW) -> BOOL;

fn notify_icon() -> Option<NotifyIcon> {
    static FOUND: OnceLock<Option<NotifyIcon>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        // SAFETY: the library stays loaded for good, and the function has this signature.
        unsafe {
            let library = LoadLibraryW(w!("shell32.dll")).ok()?;
            let found = GetProcAddress(library, s!("Shell_NotifyIconW"))?;
            Some(std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                NotifyIcon,
            >(found))
        }
    })
}

/// Copies text into room for it, cutting it off at a whole character and leaving a terminator.
fn put_text(room: &mut [u16], text: &str) {
    let mut at = 0;
    for character in text.chars() {
        let mut units = [0u16; 2];
        let units = character.encode_utf16(&mut units);
        if at + units.len() >= room.len() {
            break;
        }
        room[at..at + units.len()].copy_from_slice(units);
        at += units.len();
    }
    room[at] = 0;
}

/// What says what an icon is.
fn icon_data(window: HWND, id: u32) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: id,
        uCallbackMessage: TRAY_MESSAGE,
        ..Default::default()
    }
}

fn tray_data(window: HWND, tooltip: &str, icon: HICON) -> NOTIFYICONDATAW {
    let mut data = icon_data(window, TRAY_ICON);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.hIcon = icon;
    put_text(&mut data.szTip, tooltip);
    data
}

fn notification_data(
    window: HWND,
    id: u32,
    title: &str,
    body: &str,
    icon: HICON,
) -> NOTIFYICONDATAW {
    let mut data = icon_data(window, id);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_INFO;
    data.hIcon = icon;
    data.dwInfoFlags = NIIF_INFO;
    put_text(&mut data.szInfoTitle, title);
    put_text(&mut data.szInfo, body);
    data
}

fn program_icon() -> HICON {
    // SAFETY: these only look things up, and the system's own icon is always there.
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or(HMODULE::default());
        LoadIconW(
            Some(instance.into()),
            PCWSTR(std::ptr::without_provenance(1)),
        )
        .or_else(|_| LoadIconW(None, IDI_APPLICATION))
        .unwrap_or_default()
    }
}

/// An icon from an `.ico` file, using the part closest to `size`.
fn icon_from_ico(bytes: &[u8], size: u32) -> Option<HICON> {
    let word = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
    let long = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    if word(0)? != 0 || word(2)? != 1 {
        return None;
    }
    let mut best: Option<(u32, usize, usize)> = None;
    for index in 0..usize::from(word(4)?) {
        let entry = 6 + index * 16;
        let width = u32::from(*bytes.get(entry)?);
        let width = if width == 0 { 256 } else { width };
        let (length, offset) = (long(entry + 8)? as usize, long(entry + 12)? as usize);
        let distance = width.abs_diff(size);
        if best.is_none_or(|(closest, ..)| distance < closest) {
            best = Some((distance, offset, length));
        }
    }
    let (_, offset, length) = best?;
    let part = bytes.get(offset..offset.checked_add(length)?)?;
    // SAFETY: this only reads the bytes it's given, which it checks.
    unsafe {
        CreateIconFromResourceEx(
            part,
            true,
            0x0003_0000,
            size as i32,
            size as i32,
            IMAGE_FLAGS(0),
        )
    }
    .ok()
}

/// An icon from a picture, whose colors are already multiplied by the alpha.
fn icon_from_picture(bytes: &[u8], size: u32) -> Option<HICON> {
    let picture = rsui_image::decode(bytes, (size, size)).ok()?;
    let (width, height) = (picture.width as i32, picture.height as i32);
    let header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Going from the top down, as the pixels are.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: what's made is the size that's asked for and written, and these are let go of once the icon has its own copy.
    unsafe {
        let mut bits: *mut c_void = std::ptr::null_mut();
        let color = CreateDIBSection(
            None,
            &raw const header,
            DIB_RGB_COLORS,
            &raw mut bits,
            None,
            0,
        )
        .ok()?;
        let out = std::slice::from_raw_parts_mut(bits.cast::<u8>(), picture.pixels.len());
        for (to, from) in out.chunks_exact_mut(4).zip(picture.pixels.chunks_exact(4)) {
            to.copy_from_slice(&[from[2], from[1], from[0], from[3]]);
        }
        let mask = CreateBitmap(width, height, 1, 1, None);
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            hbmMask: mask,
            hbmColor: color,
            ..Default::default()
        });
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon.ok()
    }
}

pub(crate) fn icon_from_bytes(bytes: &[u8]) -> Option<HICON> {
    // SAFETY: this only asks.
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    icon_from_ico(bytes, size).or_else(|| icon_from_picture(bytes, size))
}

fn show_tray_icon(how: NOTIFY_ICON_MESSAGE) -> Result<(), String> {
    let found = with_desk(|desk| {
        Some((
            desk.window?,
            desk.tray.as_ref()?.tooltip.clone(),
            desk.tray_icon?,
        ))
    });
    let Some((window, tooltip, icon)) = found else {
        return Ok(());
    };
    let notify = notify_icon().ok_or("the system's tray can't be reached")?;
    let data = tray_data(window, &tooltip, icon);
    // SAFETY: this is the size it says, and lasts through the call.
    let done = unsafe { notify(how, &raw const data) };
    if done.as_bool() {
        Ok(())
    } else {
        Err("the tray icon couldn't be put there".to_owned())
    }
}

/// Puts an icon in the tray, or changes the one that's there.
pub fn set_tray(tray: Tray) -> Result<(), String> {
    let window = desk_window()?;
    let icon = tray
        .icon
        .as_deref()
        .and_then(icon_from_bytes)
        .unwrap_or_else(program_icon);
    let (already, old_icon) = with_desk(|desk| {
        let already = desk.tray.is_some();
        desk.tray = Some(tray);
        (already, desk.tray_icon.replace(icon))
    });
    let result = show_tray_icon(if already { NIM_MODIFY } else { NIM_ADD });
    if let Some(old) = old_icon {
        let _ = window;
        // SAFETY: the system has what it needs of the new one by now. The program's own icon isn't affected by this.
        unsafe {
            let _ = DestroyIcon(old);
        }
    }
    result
}

/// Takes the icon out of the tray.
pub fn remove_tray() {
    let found = with_desk(|desk| {
        desk.tray = None;
        desk.menu_actions.retain(|id, _| *id < TRAY_MENU_FROM);
        desk.window.zip(desk.tray_icon.take())
    });
    if let (Some((window, icon)), Some(notify)) = (found, notify_icon()) {
        let data = icon_data(window, TRAY_ICON);
        // SAFETY: as in `show_tray_icon`, and nothing's using the icon now.
        unsafe {
            let _ = notify(NIM_DELETE, &raw const data);
            let _ = DestroyIcon(icon);
        }
    }
}

fn tray_message(icon: u32, what: u32) {
    match what {
        WM_LBUTTONUP if icon == TRAY_ICON => {
            let action = with_desk(|desk| desk.tray.as_ref()?.on_click.clone());
            if let Some(action) = action {
                action();
            }
        }
        WM_RBUTTONUP | WM_CONTEXTMENU if icon == TRAY_ICON => show_tray_menu(),
        NIN_BALLOONUSERCLICK => {
            let action = with_desk(|desk| desk.notification.take());
            if let Some(action) = action {
                action();
            }
            remove_notification_icon();
        }
        NIN_BALLOONTIMEOUT | NIN_BALLOONHIDE => remove_notification_icon(),
        _ => {}
    }
}

fn remove_notification_icon() {
    let window = with_desk(|desk| {
        std::mem::take(&mut desk.notification_icon)
            .then_some(desk.window)
            .flatten()
    });
    if let (Some(window), Some(notify)) = (window, notify_icon()) {
        let data = icon_data(window, NOTIFICATION_ICON);
        // SAFETY: as in `show_tray_icon`.
        unsafe {
            let _ = notify(NIM_DELETE, &raw const data);
        }
    }
}

fn show_tray_menu() {
    let items = with_desk(|desk| desk.tray.as_ref().map(|tray| tray.menu.clone()));
    let (Some(items), Ok(window)) = (items, desk_window()) else {
        return;
    };
    if items.is_empty() {
        return;
    }
    let mut actions = HashMap::new();
    let mut next = TRAY_MENU_FROM;
    let mut unused = Vec::new();
    // SAFETY: the menu's made here and let go of here, and the window's on this thread. The menu goes away by itself only if the
    // window it belongs to is the one in front, and the message after it is what the system says that needs.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        if build_menu(&items, menu, &mut next, &mut actions, &mut unused).is_ok() {
            with_desk(|desk| {
                desk.menu_actions.retain(|id, _| *id < TRAY_MENU_FROM);
                desk.menu_actions.extend(actions);
            });
            let mut at = POINT::default();
            let _ = GetCursorPos(&raw mut at);
            let _ = SetForegroundWindow(window);
            let chosen = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                at.x,
                at.y,
                None,
                window,
                None,
            );
            let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));
            if chosen.0 != 0 {
                run_menu_action(chosen.0 as u16);
            }
        }
        let _ = DestroyMenu(menu);
    }
}

/// Shows a notification. It comes from the tray icon if there is one, and otherwise from one that's there for as long as this is.
pub fn notify(title: &str, body: &str, on_click: Option<Action>) -> Result<(), String> {
    let window = desk_window()?;
    let notify = notify_icon().ok_or("the system's notifications can't be reached")?;
    let tray_icon = with_desk(|desk| {
        desk.notification = on_click;
        desk.tray_icon
    });
    let (id, icon, how) = match tray_icon {
        Some(icon) => (TRAY_ICON, icon, NIM_MODIFY),
        None => {
            let already = with_desk(|desk| std::mem::replace(&mut desk.notification_icon, true));
            let how = if already { NIM_MODIFY } else { NIM_ADD };
            (NOTIFICATION_ICON, program_icon(), how)
        }
    };
    let data = notification_data(window, id, title, body, icon);
    // SAFETY: as in `show_tray_icon`.
    let done = unsafe { notify(how, &raw const data) };
    if done.as_bool() {
        Ok(())
    } else {
        Err("the notification couldn't be shown".to_owned())
    }
}

// Sound

/// Plays a sound that was opened, from the start or from where it was paused. `ended` runs if it gets to the end.
pub fn play_sound(id: SoundId, from_start: bool, ended: Option<Action>) -> Result<(), String> {
    let window = desk_window()?;
    with_desk(|desk| match ended {
        Some(ended) => {
            desk.sounds.insert(id, ended);
        }
        None => {
            desk.sounds.remove(&id);
        }
    });
    sound::play(id, from_start, window.0 as isize);
    Ok(())
}

/// One of the sounds the system makes for things: `info`, `warning`, `error` or `question`, or else its usual one.
pub fn beep(kind: &str) {
    let style = match kind {
        "info" => MB_ICONINFORMATION,
        "warning" => MB_ICONWARNING,
        "error" => MB_ICONERROR,
        "question" => MB_ICONQUESTION,
        _ => MB_OK,
    };
    #[link(name = "user32.dll", kind = "raw-dylib", modifiers = "+verbatim")]
    unsafe extern "system" {
        fn MessageBeep(kind: u32) -> i32;
    }
    // SAFETY: this only asks for a sound.
    unsafe {
        MessageBeep(style.0);
    }
}

// Starting at login, and everything else that's kept in the registry

const CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
const REG_SZ: u32 = 1;
const ONLY_TEXT: u32 = 2;
/// Where programs are listed to start at login.
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegSetKeyValueW(
        key: isize,
        sub_key: *const u16,
        value: *const u16,
        kind: u32,
        data: *const c_void,
        size: u32,
    ) -> i32;
    fn RegDeleteKeyValueW(key: isize, sub_key: *const u16, value: *const u16) -> i32;
    fn RegDeleteTreeW(key: isize, sub_key: *const u16) -> i32;
    fn RegGetValueW(
        key: isize,
        sub_key: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut c_void,
        size: *mut u32,
    ) -> i32;
    fn RegOpenKeyExW(key: isize, sub_key: *const u16, options: u32, access: u32, opened: *mut isize) -> i32;
    fn RegQueryInfoKeyW(
        key: isize,
        class: *mut u16,
        class_size: *mut u32,
        reserved: *mut u32,
        sub_keys: *mut u32,
        longest_sub_key: *mut u32,
        longest_class: *mut u32,
        values: *mut u32,
        longest_value_name: *mut u32,
        longest_value: *mut u32,
        security: *mut u32,
        written: *mut u64,
    ) -> i32;
    fn RegCloseKey(key: isize) -> i32;
    fn RegDeleteKeyW(key: isize, sub_key: *const u16) -> i32;
}

/// Takes away a key of the user's settings if there's nothing in it, neither a value nor a key, as one a program made and then
/// emptied would be. One that something else has put anything in stays.
fn delete_user_key_if_empty(key: &str) {
    const KEY_QUERY_VALUE: u32 = 1;
    let key = HSTRING::from(key);
    let mut opened = 0isize;
    // SAFETY: the name ends in a terminator and lasts through the calls, the key that's opened is closed, and only the counts
    // are asked for.
    unsafe {
        if RegOpenKeyExW(CURRENT_USER, key.as_ptr(), 0, KEY_QUERY_VALUE, &raw mut opened) != 0 {
            return;
        }
        let (mut keys, mut values) = (0u32, 0u32);
        use std::ptr::null_mut;
        let asked = RegQueryInfoKeyW(opened, null_mut(), null_mut(), null_mut(), &raw mut keys, null_mut(), null_mut(), &raw mut values, null_mut(), null_mut(), null_mut(), null_mut());
        RegCloseKey(opened);
        if asked == 0 && keys == 0 && values == 0 {
            RegDeleteKeyW(CURRENT_USER, key.as_ptr());
        }
    }
}

fn set_user_value(key: &str, name: &str, text: Option<&str>) -> Result<(), String> {
    let (key, name) = (HSTRING::from(key), HSTRING::from(name));
    let failed = if let Some(text) = text {
        let text = HSTRING::from(text);
        // SAFETY: these all end in terminators and last through the call, and the size is that of the text with its terminator.
        unsafe {
            RegSetKeyValueW(
                CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                text.as_ptr().cast(),
                ((text.len() + 1) * 2) as u32,
            )
        }
    } else {
        const NOT_THERE: i32 = 2;
        // SAFETY: as above. Removing what isn't there is fine.
        let failed = unsafe { RegDeleteKeyValueW(CURRENT_USER, key.as_ptr(), name.as_ptr()) };
        if failed == NOT_THERE { 0 } else { failed }
    };
    if failed == 0 {
        Ok(())
    } else {
        Err(format!("the setting couldn't be changed (error {failed})"))
    }
}

fn user_value(key: &str, name: &str) -> Option<String> {
    let (key, name) = (HSTRING::from(key), HSTRING::from(name));
    let mut room = vec![0u16; 2048];
    let mut size = (room.len() * 2) as u32;
    // SAFETY: these end in terminators and last through the call, and the room is as big as `size` says.
    let failed = unsafe {
        RegGetValueW(
            CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            ONLY_TEXT,
            std::ptr::null_mut(),
            room.as_mut_ptr().cast(),
            &raw mut size,
        )
    };
    if failed != 0 {
        return None;
    }
    let length = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&room[..length]))
}

/// Takes away a key of the user's settings, with everything in it. It's fine if it isn't there.
fn delete_user_key(key: &str) -> Result<(), String> {
    let key = HSTRING::from(key);
    const NOT_THERE: i32 = 2;
    // SAFETY: the name ends in a terminator and lasts through the call.
    let failed = unsafe { RegDeleteTreeW(CURRENT_USER, key.as_ptr()) };
    if failed == 0 || failed == NOT_THERE {
        Ok(())
    } else {
        Err(format!("the setting couldn't be taken away (error {failed})"))
    }
}

/// A number in the user's settings, if it's there.
fn user_number(key: &str, name: &str) -> Option<u32> {
    const ONLY_NUMBER: u32 = 0x10;
    let (key, name) = (HSTRING::from(key), HSTRING::from(name));
    let mut number = 0u32;
    let mut size = 4u32;
    // SAFETY: as in `user_value`, with room for one number.
    let failed = unsafe {
        RegGetValueW(CURRENT_USER, key.as_ptr(), name.as_ptr(), ONLY_NUMBER, std::ptr::null_mut(), std::ptr::from_mut(&mut number).cast(), &raw mut size)
    };
    (failed == 0).then_some(number)
}

/// How a program's started, with quotes around what could have spaces in it.
pub fn login_command(program: &Path, arguments: &[String]) -> String {
    let mut command = format!("\"{}\"", program.display());
    for argument in arguments {
        command.push_str(" \"");
        command.push_str(argument);
        command.push('"');
    }
    command
}

/// Says whether a program under this name starts at login, with this as what starts it, or stops it.
pub fn set_start_at_login(name: &str, command: Option<&str>) -> Result<(), String> {
    set_user_value(RUN_KEY, name, command)
}

/// What starts this program at login, if something does.
pub fn start_at_login_command(name: &str) -> Option<String> {
    user_value(RUN_KEY, name)
}

// Only one of a program

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(attributes: *const c_void, owned: i32, name: *const u16) -> *mut c_void;
    fn SetThreadExecutionState(flags: u32) -> u32;
    fn GetLastError() -> u32;
}

fn instance_title(name: &str) -> String {
    format!("rsui-one-{name}")
}

/// Says whether this is the only one of this program running. If so, `hear` is told what any others were started with. If not, what
/// this was started with has gone to the one that's running, which is allowed to come to the front, and this one should end.
pub fn claim_single_instance(
    name: &str,
    arguments: &[String],
    hear: Rc<dyn Fn(Vec<String>)>,
) -> Result<bool, String> {
    const ALREADY_THERE: u32 = 183;
    let title = instance_title(name);
    let wide = HSTRING::from(format!("Local\\{title}"));
    // SAFETY: the name ends in a terminator and lasts through the call. This is never let go of, as it says this program's running
    // for as long as it is, and the system lets go of it then.
    let first = unsafe {
        let mutex = CreateMutexW(std::ptr::null(), 0, wide.as_ptr());
        if mutex.is_null() {
            return Err(
                "it couldn't be found out whether this program is already running".to_owned(),
            );
        }
        GetLastError() != ALREADY_THERE
    };
    if first {
        let window = desk_window()?;
        with_desk(|desk| desk.second_instance = Some(hear));
        // SAFETY: the window's on this thread, and this is how another finds it.
        unsafe {
            let _ = SetWindowTextW(window, &HSTRING::from(title.as_str()));
        }
    } else {
        tell_first_instance(&title, arguments);
    }
    Ok(first)
}

fn tell_first_instance(title: &str, arguments: &[String]) {
    let text = arguments.join("\n");
    let copied = COPYDATASTRUCT {
        dwData: 0,
        cbData: text.len() as u32,
        lpData: text.as_ptr() as *mut c_void,
    };
    // SAFETY: what's sent lasts through the call, which comes back after a second if that program's stuck.
    unsafe {
        let Ok(found) = FindWindowW(CLASS, &HSTRING::from(title)) else {
            return;
        };
        let mut process = 0;
        GetWindowThreadProcessId(found, Some(&raw mut process));
        // This one is what's just been started, so it's allowed to come to the front, and to pass that on.
        let _ = AllowSetForegroundWindow(process);
        let _ = SendMessageTimeoutW(
            found,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM((&raw const copied) as isize),
            SMTO_ABORTIFHUNG,
            1000,
            None,
        );
    }
}

// Message boxes

/// What kind of question a message box is asking, which says what picture it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Info,
    Warning,
    Error,
    Question,
}

/// Which buttons a message box has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageButtons {
    Ok,
    OkCancel,
    YesNo,
    YesNoCancel,
    RetryCancel,
}

fn message_style(kind: MessageKind, buttons: MessageButtons) -> MESSAGEBOX_STYLE {
    let kind = match kind {
        MessageKind::Info => MB_ICONINFORMATION,
        MessageKind::Warning => MB_ICONWARNING,
        MessageKind::Error => MB_ICONERROR,
        MessageKind::Question => MB_ICONQUESTION,
    };
    let buttons = match buttons {
        MessageButtons::Ok => MB_OK,
        MessageButtons::OkCancel => MB_OKCANCEL,
        MessageButtons::YesNo => MB_YESNO,
        MessageButtons::YesNoCancel => MB_YESNOCANCEL,
        MessageButtons::RetryCancel => MB_RETRYCANCEL,
    };
    kind | buttons
}

/// Asks with a message box, and says which button was clicked: `ok`, `cancel`, `yes`, `no` or `retry`. It waits for that, as the
/// system's own do, with the window it belongs to staying as it is until it's answered.
pub fn message_box(
    title: &str,
    text: &str,
    kind: MessageKind,
    buttons: MessageButtons,
) -> &'static str {
    let owner = crate::dialogs::current_window();
    let (title, text) = (HSTRING::from(title), HSTRING::from(text));
    // SAFETY: the text lasts through the call, and a window that's gone only means it belongs to nothing.
    let answer = unsafe {
        MessageBoxW(
            (!owner.is_invalid()).then_some(owner),
            &text,
            &title,
            message_style(kind, buttons),
        )
    };
    match answer {
        IDOK => "ok",
        IDYES => "yes",
        IDNO => "no",
        IDRETRY => "retry",
        _ => "cancel",
    }
}

// Screens

unsafe extern "system" fn add_display(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    list: LPARAM,
) -> BOOL {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is the size it says, and `list` is what `displays` gave, which lasts while this runs.
    unsafe {
        if GetMonitorInfoW(monitor, &raw mut info).as_bool() {
            let list = &mut *(list.0 as *mut Vec<(HMONITOR, MONITORINFO)>);
            list.push((monitor, info));
        }
    }
    true.into()
}

fn scale_of(monitor: HMONITOR) -> f32 {
    type GetDpi = unsafe extern "system" fn(HMONITOR, i32, *mut u32, *mut u32) -> i32;
    static FOUND: OnceLock<Option<GetDpi>> = OnceLock::new();
    let found = *FOUND.get_or_init(|| {
        // SAFETY: the library stays loaded for good, and the function has this signature.
        unsafe {
            let library = LoadLibraryW(w!("shcore.dll")).ok()?;
            let found = GetProcAddress(library, s!("GetDpiForMonitor"))?;
            Some(std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                GetDpi,
            >(found))
        }
    });
    let (mut x, mut y) = (96, 96);
    if let Some(get) = found {
        // SAFETY: these are valid to write to, and it's a monitor the system just gave.
        unsafe {
            let _ = get(monitor, 0, &raw mut x, &raw mut y);
        }
    }
    x as f32 / 96.0
}

/// The screens there are.
pub fn displays() -> Vec<Display> {
    let mut found: Vec<(HMONITOR, MONITORINFO)> = Vec::new();
    // SAFETY: the list lasts through the call, and the function's given it back.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(add_display),
            LPARAM((&raw mut found) as isize),
        );
    }
    let area = |rect: RECT| {
        (
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        )
    };
    found
        .into_iter()
        .map(|(monitor, info)| Display {
            bounds: area(info.rcMonitor),
            work_area: area(info.rcWork),
            scale: scale_of(monitor),
            primary: info.dwFlags & 1 != 0,
        })
        .collect()
}

/// Where the pointer is on the screen, in physical pixels.
pub fn pointer_on_screen() -> (i32, i32) {
    let mut at = POINT::default();
    // SAFETY: `at` is valid to write to, and it's left as it is if this fails.
    unsafe {
        let _ = GetCursorPos(&raw mut at);
    }
    (at.x, at.y)
}

// The app

/// Says which app this program is, as Windows knows apps by (its AppUserModelID): what its windows are grouped as on the
/// taskbar, and what Task Manager puts the processes it starts under, as a web view's browser processes. Without it, a program
/// is known by its file, so every app run by the same program is one app, and a web view's processes are its browser's. It's
/// for before the first window's made; `id` is made of what an id can have, letters, digits, `.` and `-`, and kept to its most.
pub fn set_app_id(id: &str) -> Result<(), String> {
    let id = app_id(id);
    // SAFETY: a call with a string that lives for it.
    unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(id.as_str())) }.map_err(|error| error.message())
}

/// `name` as an app id can have it: anything else becomes `-`, and it's 128 long at most.
fn app_id(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '-' }).take(128).collect()
}

// Power

/// Stops the machine going to sleep and the screen going off, for as long as this is on, which is only from this thread.
pub fn keep_awake(on: bool) {
    const STAY: u32 = 0x8000_0000;
    const NEEDED: u32 = 0x1 | 0x2;
    let flags = if on { STAY | NEEDED } else { STAY };
    // SAFETY: this only says what's needed.
    unsafe {
        SetThreadExecutionState(flags);
    }
}

/// Whether the machine's running on its battery.
pub fn on_battery() -> bool {
    battery().on_battery
}

/// Says what to tell when the machine's about to `suspend`, when it's come to `resume`, and when the `power-changed`.
pub fn on_power(hear: Rc<dyn Fn(&'static str)>) -> Result<(), String> {
    desk_window()?;
    with_desk(|desk| desk.power = Some(hear));
    Ok(())
}

// Updating itself

/// Puts `new` where `program` is, with the old one kept next to it as `.old`, as a program that's running can be moved but can't be
/// overwritten. If that can't all be done, it's put back as it was. It takes effect when the program's next started.
pub fn replace_program(program: &Path, new: &Path) -> Result<(), String> {
    let old = program.with_extension("old");
    // One left over from the last time, which isn't running by now.
    let _ = std::fs::remove_file(&old);
    std::fs::rename(program, &old).map_err(|error| format!("it couldn't be moved: {error}"))?;
    if let Err(error) = std::fs::copy(new, program) {
        let _ = std::fs::remove_file(program);
        let _ = std::fs::rename(&old, program);
        return Err(format!("the new one couldn't be put there: {error}"));
    }
    Ok(())
}

/// Starts this program again, with what it was started with, and closes this one.
pub fn relaunch() -> Result<(), String> {
    let program = std::env::current_exe().map_err(|error| error.to_string())?;
    std::process::Command::new(program)
        .args(std::env::args_os().skip(1))
        .spawn()
        .map_err(|error| error.to_string())?;
    crate::close_window(crate::WindowId::MAIN);
    Ok(())
}

// Ending

/// Takes away everything that would otherwise stay behind once the program's gone, or when an app's code is replaced.
pub fn let_go_of_everything() {
    remove_tray();
    remove_notification_icon();
    let shortcuts: Vec<i32> = with_desk(|desk| desk.shortcuts.keys().copied().collect());
    for id in shortcuts {
        remove_global_shortcut(id);
    }
    let _ = set_menu_bar(&[]);
    keep_awake(false);
    with_desk(|desk| {
        desk.notification = None;
        desk.power = None;
        desk.clipboard_changed = None;
        desk.settings_changed = None;
        desk.displays_changed = None;
        desk.devices_changed = None;
    });
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    /// Plays a sound, and looks at messages until it's said to have ended.
    pub(crate) fn expect_sound_to_end(id: SoundId) {
        let ended = Rc::new(Cell::new(false));
        let noted = ended.clone();
        play_sound(id, true, Some(Rc::new(move || noted.set(true)))).unwrap();
        let started = Instant::now();
        while !ended.get() {
            assert!(started.elapsed() < Duration::from_secs(5), "it never ended");
            pump();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub(crate) fn pump() {
        let mut message = MSG::default();
        // SAFETY: `message` is valid to write to, and this only handles what's for this thread.
        unsafe {
            while PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&raw const message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn an_app_id_is_what_windows_lets_one_have() {
        assert_eq!(app_id("rsui.browser-app"), "rsui.browser-app");
        assert_eq!(app_id("rsui.My App/2"), "rsui.My-App-2");
        assert_eq!(app_id(&"x".repeat(200)).len(), 128);
    }

    fn item(label: &str) -> MenuItem {
        MenuItem {
            label: label.to_owned(),
            ..MenuItem::default()
        }
    }

    fn label_of(menu: HMENU, at: u32) -> String {
        let mut room = [0u16; 64];
        // SAFETY: this only reads a menu of this thread, into room that's as long as it says.
        let length = unsafe { GetMenuStringW(menu, at, Some(&mut room), MF_BYPOSITION) } as usize;
        String::from_utf16_lossy(&room[..length])
    }

    #[test]
    fn a_menu_comes_out_as_it_was_described() {
        let ran = Rc::new(Cell::new(0));
        let noted = ran.clone();
        let items = vec![
            MenuItem {
                shortcut: Some("ctrl+o".to_owned()),
                action: Some(Rc::new(move || noted.set(noted.get() + 1))),
                ..item("&Open")
            },
            MenuItem {
                separator: true,
                ..MenuItem::default()
            },
            MenuItem {
                disabled: true,
                checked: true,
                action: Some(Rc::new(|| {})),
                ..item("Off")
            },
            MenuItem {
                items: vec![item("Inner"), item("Inner two")],
                ..item("More")
            },
        ];
        let mut next = 7;
        let mut actions = HashMap::new();
        let mut table = Vec::new();
        // SAFETY: this makes a menu that's never shown, and lets go of it after.
        unsafe {
            let menu = CreatePopupMenu().unwrap();
            build_menu(&items, menu, &mut next, &mut actions, &mut table).unwrap();
            assert_eq!(GetMenuItemCount(Some(menu)), 4);
            assert_eq!(label_of(menu, 0), "&Open\tctrl+o");
            assert_eq!(label_of(menu, 2), "Off");
            let state = GetMenuState(menu, 2, MF_BYPOSITION);
            assert_ne!(state & MF_GRAYED.0, 0);
            assert_ne!(state & MF_CHECKED.0, 0);
            let inner = GetSubMenu(menu, 3);
            assert_eq!(GetMenuItemCount(Some(inner)), 2);
            assert_eq!(label_of(inner, 1), "Inner two");
            let _ = DestroyMenu(menu);
        }
        assert_eq!(table.len(), 1);
        assert_eq!(table[0].key, u16::from(b'O'));
        assert_eq!(table[0].cmd, 7);
        assert_eq!(table[0].fVirt, FVIRTKEY | FCONTROL);
        actions[&7]();
        assert_eq!(ran.get(), 1);
    }

    #[test]
    fn a_menu_with_keys_that_mean_nothing_says_so() {
        let items = vec![MenuItem {
            shortcut: Some("ctrl+nothing".to_owned()),
            ..item("Bad")
        }];
        // SAFETY: as above.
        unsafe {
            let menu = CreatePopupMenu().unwrap();
            let failed = build_menu(&items, menu, &mut 1, &mut HashMap::new(), &mut Vec::new());
            assert!(failed.unwrap_err().contains("nothing"));
            let _ = DestroyMenu(menu);
        }
    }

    #[test]
    fn text_is_cut_off_to_fit_at_a_whole_character() {
        let mut room = [9u16; 4];
        put_text(&mut room, "ab");
        assert_eq!(room[..3], [97, 98, 0]);
        put_text(&mut room, "abcdef");
        assert_eq!(room, [97, 98, 99, 0]);
        // That last one takes two, and only one is left after these, along with the terminator.
        put_text(&mut room, "ab\u{1F600}");
        assert_eq!(room[..3], [97, 98, 0]);
    }

    #[test]
    fn what_says_what_an_icon_is_says_what_it_was_given() {
        let data = tray_data(HWND::default(), "A tip", HICON::default());
        assert_eq!(data.uID, TRAY_ICON);
        assert_eq!(data.uCallbackMessage, TRAY_MESSAGE);
        assert_eq!(data.uFlags, NIF_MESSAGE | NIF_ICON | NIF_TIP);
        assert_eq!(data.szTip[..6], [65, 32, 116, 105, 112, 0]);
        let long = "x".repeat(500);
        let data = notification_data(
            HWND::default(),
            NOTIFICATION_ICON,
            &long,
            &long,
            HICON::default(),
        );
        assert_eq!(data.uFlags, NIF_MESSAGE | NIF_ICON | NIF_INFO);
        assert_eq!(data.szInfoTitle[data.szInfoTitle.len() - 1], 0);
        assert_eq!(data.szInfo[data.szInfo.len() - 1], 0);
        assert_eq!(data.szInfo[0], 120);
    }

    fn a_png() -> Vec<u8> {
        // One pixel, as PNGs go.
        vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52,
            0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1F, 0x15, 0xC4, 0x89, 0, 0, 0, 0x0D, 0x49,
            0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0, 5, 0, 1, 0xFF,
            0x56, 0xC7, 0x2F, 0x0D, 0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ]
    }

    #[test]
    fn an_icon_can_be_made_from_a_picture_and_not_from_nonsense() {
        let icon = icon_from_bytes(&a_png()).expect("it's a picture");
        // SAFETY: it was just made, and nothing else has it.
        unsafe {
            DestroyIcon(icon).unwrap();
        }
        assert!(icon_from_bytes(b"nonsense").is_none());
        assert!(icon_from_bytes(&[]).is_none());
    }

    #[test]
    fn an_ico_file_is_read_with_its_nearest_size() {
        // An `.ico` with one part in it, which is a PNG, as they can be.
        let png = a_png();
        let mut ico = vec![0u8, 0, 1, 0, 1, 0, 1, 1, 0, 0, 1, 0, 32, 0];
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes());
        ico.extend_from_slice(&png);
        let icon = icon_from_ico(&ico, 16).expect("it has a part");
        // SAFETY: as above.
        unsafe {
            DestroyIcon(icon).unwrap();
        }
        // A part that says it goes past the end of the file is left alone.
        let mut cut = ico.clone();
        cut.truncate(30);
        assert!(icon_from_ico(&cut, 16).is_none());
    }

    #[test]
    fn a_shortcut_is_held_until_it_is_removed_and_its_message_runs_it() {
        // Nobody has these keys, and they can't be pressed on most keyboards. The message is sent by hand.
        let keys = "ctrl+alt+shift+f23";
        let ran = Rc::new(Cell::new(0));
        let noted = ran.clone();
        let Ok(id) = add_global_shortcut(keys, Rc::new(move || noted.set(noted.get() + 1))) else {
            // Something else has them, which is its business.
            return;
        };
        let window = desk_window().unwrap();
        // SAFETY: the window's on this thread.
        unsafe {
            SendMessageW(
                window,
                WM_HOTKEY,
                Some(WPARAM(id as usize)),
                Some(LPARAM(0)),
            );
        }
        assert_eq!(ran.get(), 1);
        // Having them already means nothing else can.
        assert!(
            add_global_shortcut(keys, Rc::new(|| {}))
                .unwrap_err()
                .contains("in use")
        );
        remove_global_shortcut(id);
        let again = add_global_shortcut(keys, Rc::new(|| {})).expect("they're free again");
        remove_global_shortcut(again);
    }

    #[test]
    fn a_shortcut_that_means_nothing_says_so_before_asking_for_anything() {
        assert!(add_global_shortcut("ctrl+nothing", Rc::new(|| {})).is_err());
    }

    #[test]
    fn the_program_starts_with_quotes_around_what_could_have_spaces() {
        assert_eq!(
            login_command(
                Path::new("C:\\a b\\app.exe"),
                &["C:\\c d\\app.tsx".to_owned()]
            ),
            "\"C:\\a b\\app.exe\" \"C:\\c d\\app.tsx\""
        );
    }

    #[test]
    fn a_setting_is_kept_changed_and_removed() {
        // This has nothing to do with what starts at login: it's a key of its own, which is gone at the end.
        let key = format!("Software\\rsui-test-{}", std::process::id());
        assert_eq!(user_value(&key, "one"), None);
        set_user_value(&key, "one", Some("C:\\a program.exe")).unwrap();
        assert_eq!(
            user_value(&key, "one").as_deref(),
            Some("C:\\a program.exe")
        );
        set_user_value(&key, "one", Some("\u{4F60}\u{597D}")).unwrap();
        assert_eq!(user_value(&key, "one").as_deref(), Some("\u{4F60}\u{597D}"));
        set_user_value(&key, "one", None).unwrap();
        assert_eq!(user_value(&key, "one"), None);
        // Removing what's already gone is fine.
        set_user_value(&key, "one", None).unwrap();
        let wide = HSTRING::from(key.as_str());
        // SAFETY: the name ends in a terminator and lasts through the call.
        assert_eq!(unsafe { RegDeleteKeyW(CURRENT_USER, wide.as_ptr()) }, 0);
    }

    #[test]
    fn a_key_is_only_taken_away_once_nothing_is_in_it() {
        // A key of its own, as unregistering a kind of file leaves `.extension\OpenWithProgids` with nothing in it, which a real
        // window showed. Each is gone at the end.
        let key = format!("Software\\rsui-test-{}-empty", std::process::id());
        let inner = format!("{key}\\OpenWithProgids");
        set_user_value(&inner, "app.kind", Some("")).unwrap();
        set_user_value(&key, "", Some("a value of its own")).unwrap();
        // It has a value, and the outer one a key as well, so both stay.
        delete_user_key_if_empty(&inner);
        delete_user_key_if_empty(&key);
        assert_eq!(user_value(&inner, "app.kind").as_deref(), Some(""));
        set_user_value(&inner, "app.kind", None).unwrap();
        delete_user_key_if_empty(&inner);
        assert!(user_value(&key, "").is_some(), "the outer key has its own value, so it stays");
        assert_eq!(user_value(&inner, "app.kind"), None);
        set_user_value(&key, "", None).unwrap();
        delete_user_key_if_empty(&key);
        let wide = HSTRING::from(key.as_str());
        // SAFETY: the name ends in a terminator and lasts through the call. It's gone, so it can't be opened.
        let mut opened = 0isize;
        assert_ne!(unsafe { RegOpenKeyExW(CURRENT_USER, wide.as_ptr(), 0, 1, &raw mut opened) }, 0);
    }

    #[test]
    fn the_second_of_a_program_tells_the_first_and_is_told_it_is_second() {
        let name = format!("test-{}", std::process::id());
        let heard = Rc::new(RefCell::new(Vec::new()));
        let noted = heard.clone();
        let first = claim_single_instance(
            &name,
            &[],
            Rc::new(move |arguments| noted.borrow_mut().push(arguments)),
        );
        assert_eq!(first, Ok(true));
        let second = claim_single_instance(
            &name,
            &["one".to_owned(), "two words".to_owned()],
            Rc::new(|_| panic!("the second one doesn't hear anything")),
        );
        assert_eq!(second, Ok(false));
        assert_eq!(
            *heard.borrow(),
            [vec!["one".to_owned(), "two words".to_owned()]]
        );
    }

    #[test]
    fn there_is_a_screen_with_a_size_and_one_of_them_is_the_main_one() {
        let screens = displays();
        // A machine with nothing attached can have none.
        if screens.is_empty() {
            return;
        }
        assert_eq!(screens.iter().filter(|one| one.primary).count(), 1);
        for one in screens {
            assert!(one.bounds.2 > 0 && one.bounds.3 > 0, "{one:?}");
            assert!(one.work_area.2 <= one.bounds.2, "{one:?}");
            assert!(one.scale >= 1.0, "{one:?}");
        }
    }

    #[test]
    fn a_message_box_has_the_picture_and_buttons_it_says() {
        assert_eq!(
            message_style(MessageKind::Warning, MessageButtons::YesNo),
            MB_ICONWARNING | MB_YESNO
        );
        assert_eq!(
            message_style(MessageKind::Info, MessageButtons::Ok),
            MB_ICONINFORMATION | MB_OK
        );
    }

    #[test]
    fn a_program_can_be_replaced_with_the_old_one_kept() {
        let folder = std::env::temp_dir().join(format!("rsui-replace-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let (program, new) = (folder.join("app.exe"), folder.join("downloaded.bin"));
        std::fs::write(&program, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        replace_program(&program, &new).unwrap();
        assert_eq!(std::fs::read_to_string(&program).unwrap(), "new");
        assert_eq!(
            std::fs::read_to_string(folder.join("app.old")).unwrap(),
            "old"
        );
        // Doing it again gets rid of the one kept from before.
        std::fs::write(&new, "newer").unwrap();
        replace_program(&program, &new).unwrap();
        assert_eq!(std::fs::read_to_string(&program).unwrap(), "newer");
        assert_eq!(
            std::fs::read_to_string(folder.join("app.old")).unwrap(),
            "new"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_program_that_cannot_be_replaced_stays_as_it_was() {
        let folder = std::env::temp_dir().join(format!("rsui-unreplaced-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let program = folder.join("app.exe");
        std::fs::write(&program, "old").unwrap();
        let failed = replace_program(&program, &folder.join("not there"));
        assert!(failed.is_err());
        assert_eq!(std::fs::read_to_string(&program).unwrap(), "old");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn being_told_about_power_reaches_what_is_listening() {
        let heard = Rc::new(RefCell::new(Vec::new()));
        let noted = heard.clone();
        on_power(Rc::new(move |what| noted.borrow_mut().push(what))).unwrap();
        let window = desk_window().unwrap();
        // SAFETY: the window's on this thread.
        unsafe {
            SendMessageW(
                window,
                WM_POWERBROADCAST,
                Some(WPARAM(PBT_APMSUSPEND)),
                Some(LPARAM(0)),
            );
            SendMessageW(
                window,
                WM_POWERBROADCAST,
                Some(WPARAM(PBT_APMRESUMEAUTOMATIC)),
                Some(LPARAM(0)),
            );
        }
        assert_eq!(*heard.borrow(), ["suspend", "resume"]);
    }

    #[test]
    fn clicks_on_a_tray_icon_and_a_notification_reach_what_they_should() {
        let clicks = Rc::new(Cell::new(0));
        let noted = clicks.clone();
        // There's no icon really there: this is what the system would say about one.
        with_desk(|desk| {
            desk.tray = Some(Tray {
                on_click: Some(Rc::new(move || noted.set(noted.get() + 1))),
                ..Tray::default()
            });
        });
        tray_message(TRAY_ICON, WM_LBUTTONUP);
        assert_eq!(clicks.get(), 1);
        let told = Rc::new(Cell::new(0));
        let noted = told.clone();
        with_desk(|desk| desk.notification = Some(Rc::new(move || noted.set(noted.get() + 1))));
        tray_message(NOTIFICATION_ICON, NIN_BALLOONUSERCLICK);
        assert_eq!(told.get(), 1);
        tray_message(NOTIFICATION_ICON, NIN_BALLOONUSERCLICK);
        assert_eq!(told.get(), 1, "it only happens once");
        with_desk(|desk| desk.tray = None);
    }

    #[test]
    fn a_click_in_a_menu_runs_what_it_should() {
        let ran = Rc::new(Cell::new(0));
        let noted = ran.clone();
        with_desk(|desk| {
            desk.menu_actions
                .insert(42, Rc::new(move || noted.set(noted.get() + 1)));
        });
        assert!(menu_command(42));
        assert_eq!(ran.get(), 1);
        assert!(!menu_command(43));
        with_desk(|desk| {
            desk.menu_actions.remove(&42);
        });
    }
}
