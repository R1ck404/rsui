//! More of what a program can ask of the desktop: opening and showing files with the shell, the clipboard in all its formats,
//! the user's credentials, what the system says about itself, links and files the program opens, and dragging files out of it.
//! Like the rest of [`desktop`](super), each of these is real: what checks an app doesn't call them, it pretends.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use super::{Action, Display, MenuItem, build_menu, desk_window, displays, with_desk};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(text: &[u16]) -> String {
    let length = text.iter().position(|&one| one == 0).unwrap_or(text.len());
    String::from_utf16_lossy(&text[..length])
}

#[repr(C)]
struct ShellExecuteInfo {
    size: u32,
    mask: u32,
    window: isize,
    verb: *const u16,
    file: *const u16,
    parameters: *const u16,
    directory: *const u16,
    show: i32,
    instance: isize,
    id_list: *mut c_void,
    class: *const u16,
    class_key: isize,
    hot_key: u32,
    icon: isize,
    process: isize,
}

#[repr(C)]
struct FileOperation {
    window: isize,
    function: u32,
    from: *const u16,
    to: *const u16,
    flags: u16,
    aborted: i32,
    names: *mut c_void,
    title: *const u16,
}

#[link(name = "shell32.dll", kind = "raw-dylib", modifiers = "+verbatim")]
unsafe extern "system" {
    fn ShellExecuteExW(info: *mut ShellExecuteInfo) -> i32;
    fn SHFileOperationW(operation: *mut FileOperation) -> i32;
    fn SHAddToRecentDocs(flags: u32, path: *const c_void);
    fn SHChangeNotify(event: i32, flags: u32, first: *const c_void, second: *const c_void);
    fn SHParseDisplayName(name: *const u16, context: *mut c_void, list: *mut *mut c_void, wanted: u32, got: *mut u32) -> i32;
    fn SHCreateShellItemArrayFromIDLists(count: u32, lists: *const *const c_void, array: *mut *mut c_void) -> i32;
    fn SHDoDragDrop(window: isize, data: *mut c_void, source: *mut c_void, allowed: u32, done: *mut u32) -> i32;
    fn DragQueryFileW(drop: isize, index: u32, file: *mut u16, room: u32) -> u32;
    fn ILFree(list: *mut c_void);
}

const SEE_MASK_INVOKEIDLIST: u32 = 0x0C;
const SEE_MASK_NOASYNC: u32 = 0x100;
const FO_DELETE: u32 = 3;
const FOF_SILENT: u16 = 0x4;
const FOF_NOCONFIRMATION: u16 = 0x10;
const FOF_ALLOWUNDO: u16 = 0x40;
const FOF_NOERRORUI: u16 = 0x400;
const SHARD_PATHW: u32 = 3;
const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;

/// Has the shell do this to a file or folder, as the right-click menu would: `open`, `openas` or `properties`. It waits until
/// that's started, so it's for a thread other than a window's.
fn shell_verb(verb: &str, file: &str, parameters: Option<&str>) -> Result<(), String> {
    // What the shell does can be done by extensions it starts with COM.
    let _com = ComHere::start();
    let (verb, file) = (wide(verb), wide(file));
    let parameters = parameters.map(wide);
    let mut info = ShellExecuteInfo {
        size: std::mem::size_of::<ShellExecuteInfo>() as u32,
        mask: SEE_MASK_INVOKEIDLIST | SEE_MASK_NOASYNC,
        window: 0,
        verb: verb.as_ptr(),
        file: file.as_ptr(),
        parameters: parameters.as_ref().map_or(std::ptr::null(), |one| one.as_ptr()),
        directory: std::ptr::null(),
        show: 1,
        instance: 0,
        id_list: std::ptr::null_mut(),
        class: std::ptr::null(),
        class_key: 0,
        hot_key: 0,
        icon: 0,
        process: 0,
    };
    // SAFETY: the structure is the size it says, and what it points to lasts through the call.
    if unsafe { ShellExecuteExW(&raw mut info) } == 0 {
        return Err(format!("the shell couldn't do that: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Opens a file or folder with what the system has for it.
pub fn open_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("there's nothing at {}", path.display()));
    }
    shell_verb("open", &path.display().to_string(), None)
}

/// Shows a file in its folder, picked out, as Explorer does.
pub fn reveal(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("there's nothing at {}", path.display()));
    }
    shell_verb("open", "explorer.exe", Some(&format!("/select,\"{}\"", path.display())))
}

/// Asks which program to open a file with, as the system's "Open with" does.
pub fn open_with(path: &Path) -> Result<(), String> {
    shell_verb("openas", &path.display().to_string(), None)
}

/// Shows the system's properties of a file or folder.
pub fn show_properties(path: &Path) -> Result<(), String> {
    shell_verb("properties", &path.display().to_string(), None)
}

/// Puts these in the Recycle Bin, with nothing asked, as they can be brought back from there.
pub fn trash(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    // A list of paths, each ending in a zero, with another at the end.
    let mut from: Vec<u16> = Vec::new();
    for path in paths {
        if !path.exists() {
            return Err(format!("there's nothing at {}", path.display()));
        }
        from.extend(path.display().to_string().encode_utf16());
        from.push(0);
    }
    from.push(0);
    let mut operation = FileOperation {
        window: 0,
        function: FO_DELETE,
        from: from.as_ptr(),
        to: std::ptr::null(),
        flags: FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI,
        aborted: 0,
        names: std::ptr::null_mut(),
        title: std::ptr::null(),
    };
    // SAFETY: the list ends as the system wants it to, and lasts through the call.
    let failed = unsafe { SHFileOperationW(&raw mut operation) };
    if failed != 0 || operation.aborted != 0 {
        return Err(format!("they couldn't go in the Recycle Bin (error {failed})"));
    }
    Ok(())
}

/// Says these were opened, for the system's lists of recent files, as in the program's jump list. Nothing clears them all.
pub fn add_recent_documents(paths: &[PathBuf]) {
    for path in paths {
        let path = wide(&path.display().to_string());
        // SAFETY: the path ends in a zero and lasts through the call.
        unsafe { SHAddToRecentDocs(SHARD_PATHW, path.as_ptr().cast()) };
    }
}

pub fn clear_recent_documents() {
    // SAFETY: nothing, which is what clears them.
    unsafe { SHAddToRecentDocs(SHARD_PATHW, std::ptr::null()) };
}

// The clipboard.

/// What's on the clipboard, in the formats this knows.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct ClipboardContents {
    pub text: Option<String>,
    pub html: Option<String>,
    pub files: Vec<String>,
    /// A picture, as a PNG file's bytes.
    pub png: Option<Vec<u8>>,
    /// A picture that's only there as a bitmap, as most programs and Print Screen copy one, read rather than written.
    pub picture: Option<Picture>,
}

#[link(name = "user32.dll", kind = "raw-dylib", modifiers = "+verbatim")]
unsafe extern "system" {
    fn OpenClipboard(window: isize) -> i32;
    fn CloseClipboard() -> i32;
    fn EmptyClipboard() -> i32;
    fn GetClipboardData(format: u32) -> isize;
    fn SetClipboardData(format: u32, data: isize) -> isize;
    fn EnumClipboardFormats(format: u32) -> u32;
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
    fn GetClipboardFormatNameW(format: u32, name: *mut u16, room: i32) -> i32;
    fn AddClipboardFormatListener(window: isize) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalAlloc(flags: u32, size: usize) -> isize;
    fn GlobalLock(memory: isize) -> *mut c_void;
    fn GlobalUnlock(memory: isize) -> i32;
    fn GlobalSize(memory: isize) -> usize;
    fn GlobalFree(memory: isize) -> isize;
}

const CF_BITMAP: u32 = 2;
const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const CF_DIBV5: u32 = 17;
const GMEM_MOVEABLE: u32 = 2;

fn html_format() -> u32 {
    // SAFETY: the name ends in a zero.
    unsafe { RegisterClipboardFormatW(w!("HTML Format").as_ptr()) }
}

fn png_format() -> u32 {
    // SAFETY: as above.
    unsafe { RegisterClipboardFormatW(w!("PNG").as_ptr()) }
}

/// The clipboard, open for as long as this is, which it can only be for one thing at a time.
struct Opened;

impl Opened {
    fn new() -> Result<Self, String> {
        // It's held by something else for a moment as often as not, as when what's copying is still writing to it.
        for _ in 0..10 {
            // SAFETY: no window is the clipboard's owner, which is fine for reading it and writing it.
            if unsafe { OpenClipboard(0) } != 0 {
                return Ok(Opened);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Err("the clipboard is in use by another program".to_owned())
    }
}

impl Drop for Opened {
    fn drop(&mut self) {
        // SAFETY: it's open, as this was made.
        unsafe { CloseClipboard() };
    }
}

/// The bytes of what's on the clipboard in this format, if it's there.
fn bytes_in(format: u32) -> Option<Vec<u8>> {
    // SAFETY: what's given back belongs to the clipboard, which is open, and is only read while it's locked.
    unsafe {
        let data = GetClipboardData(format);
        if data == 0 {
            return None;
        }
        let size = GlobalSize(data);
        let at = GlobalLock(data);
        if at.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(at.cast::<u8>(), size).to_vec();
        GlobalUnlock(data);
        Some(bytes)
    }
}

/// Puts these bytes on the clipboard in this format. The clipboard owns them after that.
fn put(format: u32, bytes: &[u8]) -> Result<(), String> {
    // SAFETY: the memory's made the size it's filled to, and given to the clipboard, which frees it, unless that fails.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        if memory == 0 {
            return Err("there's no memory for the clipboard".to_owned());
        }
        let at = GlobalLock(memory);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), at.cast::<u8>(), bytes.len());
        GlobalUnlock(memory);
        if SetClipboardData(format, memory) == 0 {
            GlobalFree(memory);
            return Err("the clipboard didn't take it".to_owned());
        }
    }
    Ok(())
}

/// What a page's HTML is on the clipboard: a header that says where the fragment is in what follows.
fn html_for_clipboard(html: &str) -> Vec<u8> {
    let start = "<html><body><!--StartFragment-->";
    let end = "<!--EndFragment--></body></html>";
    let header_size = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n".len();
    let start_html = header_size;
    let start_fragment = start_html + start.len();
    let end_fragment = start_fragment + html.len();
    let end_html = end_fragment + end.len();
    format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n{start}{html}{end}"
    )
    .into_bytes()
}

/// The fragment of the clipboard's HTML, which is what was copied.
fn html_from_clipboard(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let number = |name: &str| {
        text.find(name)
            .and_then(|at| text[at + name.len()..].split(['\r', '\n']).next())
            .and_then(|digits| digits.trim().parse::<usize>().ok())
    };
    match (number("StartFragment:"), number("EndFragment:")) {
        (Some(from), Some(to)) if from <= to && to <= bytes.len() => String::from_utf8_lossy(&bytes[from..to]).into_owned(),
        _ => text.trim_end_matches('\0').to_owned(),
    }
}

/// Reads what's on the clipboard, in each format this knows.
pub fn read_clipboard() -> Result<ClipboardContents, String> {
    let _open = Opened::new()?;
    let mut out = ClipboardContents::default();
    if let Some(bytes) = bytes_in(CF_UNICODETEXT) {
        let units: Vec<u16> = bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
        out.text = Some(from_wide(&units));
    }
    if let Some(bytes) = bytes_in(html_format()) {
        out.html = Some(html_from_clipboard(&bytes));
    }
    if let Some(bytes) = bytes_in(png_format()) {
        out.png = Some(bytes);
    } else if let Some(bytes) = bytes_in(CF_DIB) {
        // The system makes this from a bitmap of any other kind that's there.
        out.picture = rsui_image::dib::decode(&bytes);
    }
    // SAFETY: the clipboard's open, and what it gives for files is read as the system says, with room for each name.
    unsafe {
        let drop = GetClipboardData(CF_HDROP);
        if drop != 0 {
            let count = DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0);
            for index in 0..count {
                let length = DragQueryFileW(drop, index, std::ptr::null_mut(), 0);
                let mut room = vec![0u16; length as usize + 1];
                DragQueryFileW(drop, index, room.as_mut_ptr(), room.len() as u32);
                out.files.push(from_wide(&room));
            }
        }
    }
    Ok(out)
}

/// Puts this on the clipboard in place of what's there: text, HTML, files and a PNG picture, each that's given.
pub fn write_clipboard(contents: &ClipboardContents) -> Result<(), String> {
    let _open = Opened::new()?;
    // SAFETY: it's open.
    unsafe { EmptyClipboard() };
    if let Some(text) = &contents.text {
        let units = wide(text);
        let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
        put(CF_UNICODETEXT, &bytes)?;
    }
    if let Some(html) = &contents.html {
        put(html_format(), &html_for_clipboard(html))?;
    }
    if let Some(png) = &contents.png {
        put(png_format(), png)?;
        // Most programs only paste a bitmap, as Chrome writes one too.
        if let Ok(picture) = rsui_image::decode(png, (u32::MAX, u32::MAX)) {
            put(CF_DIB, &rsui_image::dib::encode(picture.width, picture.height, &picture.pixels))?;
        }
    }
    if !contents.files.is_empty() {
        // DROPFILES, then each path ending in a zero, then another zero, in UTF-16.
        let mut bytes = Vec::new();
        bytes.extend(20u32.to_le_bytes());
        bytes.extend([0u8; 12]);
        bytes.extend(1u32.to_le_bytes());
        for file in &contents.files {
            for unit in file.encode_utf16().chain(std::iter::once(0)) {
                bytes.extend(unit.to_le_bytes());
            }
        }
        bytes.extend([0, 0]);
        put(CF_HDROP, &bytes)?;
    }
    Ok(())
}

pub fn clear_clipboard() -> Result<(), String> {
    let _open = Opened::new()?;
    // SAFETY: it's open.
    unsafe { EmptyClipboard() };
    Ok(())
}

/// What's on the clipboard, as the names a page uses for them where it has one: `text/plain`, `text/html`, `image/png`, `files`.
pub fn clipboard_formats() -> Result<Vec<String>, String> {
    let _open = Opened::new()?;
    let (html, png) = (html_format(), png_format());
    let mut out = Vec::new();
    let mut format = 0;
    loop {
        // SAFETY: the clipboard's open, and going through its formats is done this way.
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            break;
        }
        let name = match format {
            CF_UNICODETEXT => "text/plain".to_owned(),
            CF_HDROP => "files".to_owned(),
            _ if format == html => "text/html".to_owned(),
            // A bitmap is read as a PNG.
            CF_BITMAP | CF_DIB | CF_DIBV5 => "image/png".to_owned(),
            _ if format == png => "image/png".to_owned(),
            _ => {
                let mut room = [0u16; 128];
                // SAFETY: the room is as big as it says.
                let length = unsafe { GetClipboardFormatNameW(format, room.as_mut_ptr(), room.len() as i32) };
                if length <= 0 {
                    continue;
                }
                from_wide(&room)
            }
        };
        if !out.contains(&name) {
            out.push(name);
        }
    }
    Ok(out)
}

/// Runs this whenever something's put on the clipboard, by this program or another.
pub fn on_clipboard_change(hear: Action) -> Result<(), String> {
    let window = desk_window()?;
    let first = with_desk(|desk| desk.clipboard_changed.replace(hear).is_none());
    // SAFETY: the window's on this thread.
    if first && unsafe { AddClipboardFormatListener(window.0 as isize) } == 0 {
        return Err("the clipboard can't be listened to".to_owned());
    }
    Ok(())
}

// What the user's kept with Windows, in the Credential Manager.

#[repr(C)]
struct Credential {
    flags: u32,
    kind: u32,
    target: *mut u16,
    comment: *mut u16,
    written: u64,
    size: u32,
    blob: *mut u8,
    persist: u32,
    attribute_count: u32,
    attributes: *mut c_void,
    alias: *mut u16,
    user: *mut u16,
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn CredReadW(target: *const u16, kind: u32, flags: u32, credential: *mut *mut Credential) -> i32;
    fn CredWriteW(credential: *const Credential, flags: u32) -> i32;
    fn CredDeleteW(target: *const u16, kind: u32, flags: u32) -> i32;
    fn CredFree(buffer: *const c_void);
}

const CRED_TYPE_GENERIC: u32 = 1;
const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;

/// What's kept for this name in the Credential Manager, if anything is.
pub fn read_secret(target: &str) -> Option<String> {
    let target = wide(target);
    let mut found: *mut Credential = std::ptr::null_mut();
    // SAFETY: the name ends in a zero, and what comes back is read and then freed, as the system says.
    unsafe {
        if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &raw mut found) == 0 || found.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts((*found).blob, (*found).size as usize).to_vec();
        CredFree(found.cast());
        String::from_utf8(bytes).ok()
    }
}

/// Keeps this for the name, for this user, where only they can read it.
pub fn write_secret(target: &str, user: &str, value: &str) -> Result<(), String> {
    let (mut target, mut user) = (wide(target), wide(user));
    let mut bytes = value.as_bytes().to_vec();
    if bytes.len() > 2560 {
        return Err("a secret can be 2560 bytes at most".to_owned());
    }
    let credential = Credential {
        flags: 0,
        kind: CRED_TYPE_GENERIC,
        target: target.as_mut_ptr(),
        comment: std::ptr::null_mut(),
        written: 0,
        size: bytes.len() as u32,
        blob: bytes.as_mut_ptr(),
        persist: CRED_PERSIST_LOCAL_MACHINE,
        attribute_count: 0,
        attributes: std::ptr::null_mut(),
        alias: std::ptr::null_mut(),
        user: user.as_mut_ptr(),
    };
    // SAFETY: everything it points to lasts through the call.
    if unsafe { CredWriteW(&raw const credential, 0) } == 0 {
        return Err(format!("it couldn't be kept: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Forgets what's kept for this name, and says whether there was anything.
pub fn delete_secret(target: &str) -> bool {
    let target = wide(target);
    // SAFETY: the name ends in a zero.
    unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) != 0 }
}

// What the system says about itself.

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetUserDefaultLocaleName(name: *mut u16, room: i32) -> i32;
    fn GetTickCount() -> u32;
    fn GetCurrentThreadId() -> u32;
    fn GetSystemPowerStatus(status: *mut PowerStatus) -> i32;
}

#[repr(C)]
#[derive(Default)]
struct LastInput {
    size: u32,
    time: u32,
}

#[link(name = "user32.dll", kind = "raw-dylib", modifiers = "+verbatim")]
unsafe extern "system" {
    fn GetLastInputInfo(info: *mut LastInput) -> i32;
}

#[repr(C)]
#[derive(Default)]
struct PowerStatus {
    line: u8,
    battery_flag: u8,
    battery_percent: u8,
    saver: u8,
    life: u32,
    full_life: u32,
}

#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmGetColorizationColor(color: *mut u32, opaque: *mut i32) -> i32;
}

/// The language and region the user has set, as a page's `navigator.language` has it, like `en-US`.
pub fn locale() -> String {
    let mut room = [0u16; 85];
    // SAFETY: the room's as big as it says.
    let length = unsafe { GetUserDefaultLocaleName(room.as_mut_ptr(), room.len() as i32) };
    if length <= 1 {
        return "en-US".to_owned();
    }
    from_wide(&room)
}

/// How long it's been since the user last used the keyboard or the mouse, in milliseconds.
pub fn idle_time() -> u32 {
    let mut info = LastInput { size: std::mem::size_of::<LastInput>() as u32, time: 0 };
    // SAFETY: the structure's the size it says.
    unsafe {
        if GetLastInputInfo(&raw mut info) == 0 {
            return 0;
        }
        GetTickCount().wrapping_sub(info.time)
    }
}

/// What's running the machine: whether it's on its battery, how full that is, and whether it's charging.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Battery {
    pub on_battery: bool,
    /// How full it is, from 0 to 1, if there's one.
    pub level: Option<f32>,
    pub charging: bool,
}

pub fn battery() -> Battery {
    let mut status = PowerStatus::default();
    // SAFETY: the structure's the one this fills.
    if unsafe { GetSystemPowerStatus(&raw mut status) } == 0 {
        return Battery { on_battery: false, level: None, charging: false };
    }
    let none = status.battery_flag == 128 || status.battery_flag == 255;
    Battery {
        on_battery: status.line == 0,
        level: (!none && status.battery_percent <= 100).then(|| f32::from(status.battery_percent) / 100.0),
        charging: status.battery_flag & 8 != 0,
    }
}

/// The color the user's chosen for Windows, as `#rrggbb`.
pub fn accent_color() -> Option<String> {
    // What Settings calls the accent color, as 0xAABBGGRR. The colorization color, which `DwmGetColorizationColor` gives, is
    // what a title bar's tinted with, and can be another: a real window had red for it, where the accent was blue.
    if let Some(color) = super::user_number(r"Software\Microsoft\Windows\DWM", "AccentColor") {
        let [red, green, blue, _] = color.to_le_bytes();
        return Some(format!("#{red:02x}{green:02x}{blue:02x}"));
    }
    let (mut color, mut opaque) = (0u32, 0i32);
    // SAFETY: both are what this fills.
    if unsafe { DwmGetColorizationColor(&raw mut color, &raw mut opaque) } != 0 {
        return None;
    }
    Some(format!("#{:06x}", color & 0x00FF_FFFF))
}

/// What the user's asked of programs to make them easier to use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accessibility {
    /// Less moving about, as the system's "Animation effects" being off says.
    pub reduced_motion: bool,
    /// The system's high-contrast colors are on.
    pub high_contrast: bool,
    /// How much bigger text is wanted, from Windows' text size.
    pub text_scale: f32,
}

#[repr(C)]
struct HighContrast {
    size: u32,
    flags: u32,
    scheme: *mut u16,
}

pub fn accessibility() -> Accessibility {
    let mut animations = 1i32;
    let mut contrast = HighContrast { size: std::mem::size_of::<HighContrast>() as u32, flags: 0, scheme: std::ptr::null_mut() };
    // SAFETY: each is what its setting is read into, and as big as it says.
    unsafe {
        let _ = SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(std::ptr::from_mut(&mut animations).cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        let _ = SystemParametersInfoW(SPI_GETHIGHCONTRAST, contrast.size, Some(std::ptr::from_mut(&mut contrast).cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    let scale = super::user_number(r"Software\Microsoft\Accessibility", "TextScaleFactor").unwrap_or(100);
    Accessibility { reduced_motion: animations == 0, high_contrast: contrast.flags & 1 != 0, text_scale: scale as f32 / 100.0 }
}

/// The user's time zone, by its IANA name, like `Europe/Amsterdam`, as the ICU that comes with Windows 10 1903 and later
/// names the system's. ICU is only loaded once this is asked for.
pub fn time_zone() -> Option<String> {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    type DefaultZone = unsafe extern "C" fn(*mut u16, i32, *mut i32) -> i32;
    // SAFETY: the library stays loaded, the function has this signature in ICU's C API, and it's given room that's as big as
    // it's told, and an error code to set.
    unsafe {
        let library = LoadLibraryW(w!("icu.dll")).ok()?;
        let found = GetProcAddress(library, windows::core::s!("ucal_getDefaultTimeZone"))?;
        let default_zone = std::mem::transmute::<unsafe extern "system" fn() -> isize, DefaultZone>(found);
        let mut room = [0u16; 128];
        let mut error = 0i32;
        let length = default_zone(room.as_mut_ptr(), room.len() as i32, &raw mut error);
        // An ICU error code over 0 is a failure.
        (error <= 0 && length > 0 && (length as usize) < room.len()).then(|| String::from_utf16_lossy(&room[..length as usize]))
    }
}

/// Runs this when the user changes a setting of the system's, as its language, colors or what makes it easier to use.
pub fn on_settings_change(hear: Action) -> Result<(), String> {
    desk_window()?;
    with_desk(|desk| desk.settings_changed = Some(hear));
    Ok(())
}

/// Runs this when a screen's been added, taken away or changed.
pub fn on_displays_change(hear: Action) -> Result<(), String> {
    desk_window()?;
    with_desk(|desk| desk.displays_changed = Some(hear));
    Ok(())
}

/// The screen a point's on, in physical pixels, which is the nearest one if it's on none.
pub fn display_at(x: i32, y: i32) -> Option<Display> {
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
    // SAFETY: this only asks.
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    display_of_monitor(monitor.0 as isize)
}

/// The screen most of a window's on.
pub fn display_of_window(window: isize) -> Option<Display> {
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
    // SAFETY: as above, and a window that's gone gives the nearest to nothing.
    let monitor = unsafe { MonitorFromWindow(HWND(window as *mut c_void), MONITOR_DEFAULTTONEAREST) };
    display_of_monitor(monitor.0 as isize)
}

fn display_of_monitor(monitor: isize) -> Option<Display> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    // SAFETY: the structure's the size it says.
    unsafe {
        let _ = GetMonitorInfoW(HMONITOR(monitor as *mut c_void), &raw mut info);
    }
    let bounds = (info.rcMonitor.left, info.rcMonitor.top, info.rcMonitor.right - info.rcMonitor.left, info.rcMonitor.bottom - info.rcMonitor.top);
    displays().into_iter().find(|one| one.bounds == bounds)
}

// Whether the machine can reach the internet.

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct ConnectivityHint {
    level: i32,
    cost: i32,
    approaching_limit: u8,
    over_limit: u8,
    roaming: u8,
}

type HintChanged = unsafe extern "system" fn(context: *const c_void, hint: ConnectivityHint);

#[link(name = "iphlpapi")]
unsafe extern "system" {
    fn GetNetworkConnectivityHint(hint: *mut ConnectivityHint) -> u32;
    fn NotifyNetworkConnectivityHintChange(callback: HintChanged, context: *const c_void, initial: u8, handle: *mut isize) -> u32;
}

/// How the machine's connected: whether it can reach the internet, and whether the connection costs by how much is used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetworkStatus {
    pub online: bool,
    /// Only this machine, or a network with no way out of it, as the system says.
    pub local_only: bool,
    pub metered: bool,
}

fn status_of(hint: ConnectivityHint) -> NetworkStatus {
    // 0 is unknown, 1 none, 2 local, 3 internet, 4 behind a sign-in page, 5 hidden.
    NetworkStatus { online: hint.level == 3 || hint.level == 0, local_only: hint.level == 2 || hint.level == 4, metered: hint.cost == 2 || hint.cost == 3 }
}

pub fn network_status() -> NetworkStatus {
    let mut hint = ConnectivityHint::default();
    // SAFETY: the structure's what this fills.
    if unsafe { GetNetworkConnectivityHint(&raw mut hint) } != 0 {
        return NetworkStatus { online: true, local_only: false, metered: false };
    }
    status_of(hint)
}

/// What's told when the network changes, which is called on a thread of the system's, so it can only be something that can be
/// sent there.
static NETWORK_HEARD: std::sync::OnceLock<Box<dyn Fn(NetworkStatus) + Send + Sync>> = std::sync::OnceLock::new();

unsafe extern "system" fn network_changed(_: *const c_void, hint: ConnectivityHint) {
    if let Some(hear) = NETWORK_HEARD.get() {
        hear(status_of(hint));
    }
}

/// Runs this, on another thread, when whether the machine can reach the internet changes. Only one can be given.
pub fn on_network_change(hear: Box<dyn Fn(NetworkStatus) + Send + Sync>) -> Result<(), String> {
    if NETWORK_HEARD.set(hear).is_err() {
        return Ok(());
    }
    let mut handle = 0isize;
    // SAFETY: the function lasts for as long as the program, and nothing's given with it.
    let failed = unsafe { NotifyNetworkConnectivityHintChange(network_changed, std::ptr::null(), 0, &raw mut handle) };
    if failed != 0 {
        return Err(format!("the network can't be listened to (error {failed})"));
    }
    Ok(())
}

// Links and files the program is what opens.

/// Says this program opens links that start `scheme:`, for this user, with what opens them given the link after `command`.
pub fn register_protocol(scheme: &str, description: &str, command: &str) -> Result<(), String> {
    let key = format!(r"Software\Classes\{scheme}");
    super::set_user_value(&key, "", Some(&format!("URL:{description}")))?;
    super::set_user_value(&key, "URL Protocol", Some(""))?;
    super::set_user_value(&format!(r"{key}\shell\open\command"), "", Some(&format!("{command} \"%1\"")))?;
    Ok(())
}

pub fn unregister_protocol(scheme: &str) -> Result<(), String> {
    super::delete_user_key(&format!(r"Software\Classes\{scheme}"))
}

/// What opens links that start `scheme:` for this user, if this program said it does.
pub fn protocol_command(scheme: &str) -> Option<String> {
    super::user_value(&format!(r"Software\Classes\{scheme}\shell\open\command"), "")
}

/// Says this program opens files that end `.extension`, under `kind`, the name the system knows the kind of file by, for this user.
pub fn register_file_kind(extension: &str, kind: &str, description: &str, command: &str, icon: Option<&str>) -> Result<(), String> {
    let extension = extension.trim_start_matches('.');
    super::set_user_value(&format!(r"Software\Classes\.{extension}\OpenWithProgids"), kind, Some(""))?;
    super::set_user_value(&format!(r"Software\Classes\{kind}"), "", Some(description))?;
    if let Some(icon) = icon {
        super::set_user_value(&format!(r"Software\Classes\{kind}\DefaultIcon"), "", Some(icon))?;
    }
    super::set_user_value(&format!(r"Software\Classes\{kind}\shell\open\command"), "", Some(&format!("{command} \"%1\"")))?;
    // SAFETY: this only tells the shell, with nothing else given.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, 0, std::ptr::null(), std::ptr::null()) };
    Ok(())
}

pub fn unregister_file_kind(extension: &str, kind: &str) -> Result<(), String> {
    let extension = extension.trim_start_matches('.');
    super::set_user_value(&format!(r"Software\Classes\.{extension}\OpenWithProgids"), kind, None)?;
    // The extension's keys go too if nothing else is in them, as they're likely to have been made for this.
    super::delete_user_key_if_empty(&format!(r"Software\Classes\.{extension}\OpenWithProgids"));
    super::delete_user_key_if_empty(&format!(r"Software\Classes\.{extension}"));
    super::delete_user_key(&format!(r"Software\Classes\{kind}"))?;
    // SAFETY: as above.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, 0, std::ptr::null(), std::ptr::null()) };
    Ok(())
}

/// Opens the system's settings for which programs open what, as Windows only lets the user choose that.
pub fn open_default_apps_settings() -> Result<(), String> {
    shell_verb("open", "ms-settings:defaultapps", None)
}

// A menu where the pointer is.

/// Shows a menu at this point on the screen, in physical pixels, or where the pointer is, and runs what's chosen from it.
pub fn popup_menu(items: &[MenuItem], at: Option<(i32, i32)>) -> Result<(), String> {
    if items.is_empty() {
        return Ok(());
    }
    let window = desk_window()?;
    let mut actions: HashMap<u16, Action> = HashMap::new();
    let mut next = 0x6000;
    let mut unused = Vec::new();
    // SAFETY: the menu's made and let go of here, on this thread, as `show_tray_menu` does it.
    unsafe {
        let menu = CreatePopupMenu().map_err(|error| error.to_string())?;
        let built = build_menu(items, menu, &mut next, &mut actions, &mut unused);
        if built.is_ok() {
            let mut point = POINT::default();
            match at {
                Some((x, y)) => point = POINT { x, y },
                None => {
                    let _ = GetCursorPos(&raw mut point);
                }
            }
            let _ = SetForegroundWindow(window);
            let chosen = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, point.x, point.y, None, window, None);
            let _ = PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0));
            if chosen.0 != 0 {
                if let Some(action) = actions.get(&(chosen.0 as u16)) {
                    action();
                }
            }
        }
        let _ = DestroyMenu(menu);
        built
    }
}

// Dragging files out of the program.

use windows::core::GUID;

/// What the files are bound to, to be picked up: a data object, as Explorer gives. These are the `windows` crate's, as the
/// one typed in by hand here was wrong, and no file could be dragged out in a real window.
static BHID_DATA_OBJECT: GUID = windows::Win32::UI::Shell::BHID_DataObject;
static IID_IDATA_OBJECT: GUID = <windows::Win32::System::Com::IDataObject as windows::core::Interface>::IID;

/// Starts dragging these files, as if they'd been picked up in Explorer, so they can be dropped in another program. It only
/// makes sense while a button's held, and waits until they've been dropped. It says what was done: `copy`, `move`, `link`
/// or `none`.
pub fn drag_files(paths: &[PathBuf]) -> Result<&'static str, String> {
    if paths.is_empty() {
        return Ok("none");
    }
    let mut lists: Vec<*mut c_void> = Vec::new();
    let free = |lists: &[*mut c_void]| {
        for list in lists {
            // SAFETY: each was made by `SHParseDisplayName`.
            unsafe { ILFree(*list) };
        }
    };
    for path in paths {
        let name = wide(&path.display().to_string());
        let mut list = std::ptr::null_mut();
        // SAFETY: the name ends in a zero, and what comes back is freed below.
        if unsafe { SHParseDisplayName(name.as_ptr(), std::ptr::null_mut(), &raw mut list, 0, std::ptr::null_mut()) } != 0 {
            free(&lists);
            return Err(format!("there's nothing at {}", path.display()));
        }
        lists.push(list);
    }
    let mut array: *mut c_void = std::ptr::null_mut();
    // SAFETY: the lists last through the call, and the array that comes back is a COM object, released below.
    let made = unsafe { SHCreateShellItemArrayFromIDLists(lists.len() as u32, lists.as_ptr().cast(), &raw mut array) };
    free(&lists);
    if made != 0 || array.is_null() {
        return Err("the files couldn't be picked up".to_owned());
    }
    // IShellItemArray: QueryInterface, AddRef, Release, BindToHandler, ...
    type BindToHandler = unsafe extern "system" fn(this: *mut c_void, context: *mut c_void, handler: *const GUID, wanted: *const GUID, out: *mut *mut c_void) -> i32;
    type Release = unsafe extern "system" fn(this: *mut c_void) -> u32;
    // SAFETY: a COM object's first field points to its functions, in the order its interface lists them.
    unsafe {
        let table = *(array as *const *const usize);
        let bind: BindToHandler = std::mem::transmute(*table.add(3));
        let release: Release = std::mem::transmute(*table.add(2));
        let mut data: *mut c_void = std::ptr::null_mut();
        let bound = bind(array, std::ptr::null_mut(), &raw const BHID_DATA_OBJECT, &raw const IID_IDATA_OBJECT, &raw mut data);
        release(array);
        if bound != 0 || data.is_null() {
            return Err("the files couldn't be picked up".to_owned());
        }
        let mut done = 0u32;
        // Dragging needs OLE started on this thread, which a real window didn't have unless something could be dropped on it.
        if !crate::drop::start_ole() {
            let data_table = *(data as *const *const usize);
            let release_data: Release = std::mem::transmute(*data_table.add(2));
            release_data(data);
            return Err("drag and drop couldn't be started".to_owned());
        }
        // The window the button went down in, which the pointer's over as the drag starts, if it's one of this thread's.
        let mut at = POINT::default();
        let _ = GetCursorPos(&raw mut at);
        let under = GetAncestor(WindowFromPoint(at), GA_ROOT);
        let pressed_in = (!under.is_invalid() && GetWindowThreadProcessId(under, None) == GetCurrentThreadId()).then_some(under);
        // Copying, moving and linking are all allowed, and what's dropped on says which.
        let dragged = SHDoDragDrop(0, data, std::ptr::null_mut(), 1 | 2 | 4, &raw mut done);
        // The button came up in the drag's own loop, which the window never hears, so it's told its press is over, which ends
        // a drag of its own with `cancel`. It's posted, as its code is running now: a real window went on taking the
        // pointer's moves as a drag after this.
        if let Some(window) = pressed_in {
            let _ = PostMessageW(Some(window), WM_CAPTURECHANGED, WPARAM(0), LPARAM(0));
        }
        let data_table = *(data as *const *const usize);
        let release_data: Release = std::mem::transmute(*data_table.add(2));
        release_data(data);
        const DRAGDROP_S_DROP: i32 = 0x0004_0100;
        const DRAGDROP_S_CANCEL: i32 = 0x0004_0101;
        if dragged != 0 && dragged != DRAGDROP_S_DROP && dragged != DRAGDROP_S_CANCEL {
            return Err(format!("dragging didn't start (error {dragged:#x})"));
        }
        Ok(match done {
            1 => "copy",
            2 => "move",
            4 => "link",
            _ => "none",
        })
    }
}

/// Where the tray icon is on the screen, in physical pixels, if there is one: `(x, y, width, height)`.
pub fn tray_bounds() -> Option<(i32, i32, i32, i32)> {
    #[repr(C)]
    struct Identifier {
        size: u32,
        window: isize,
        id: u32,
        guid: [u8; 16],
    }
    #[link(name = "shell32.dll", kind = "raw-dylib", modifiers = "+verbatim")]
    unsafe extern "system" {
        fn Shell_NotifyIconGetRect(identifier: *const Identifier, rect: *mut windows::Win32::Foundation::RECT) -> i32;
    }
    let window = with_desk(|desk| desk.tray.as_ref().and(desk.window))?;
    let identifier = Identifier { size: std::mem::size_of::<Identifier>() as u32, window: window.0 as isize, id: super::TRAY_ICON, guid: [0; 16] };
    let mut rect = windows::Win32::Foundation::RECT::default();
    // SAFETY: both are what the call reads and fills.
    if unsafe { Shell_NotifyIconGetRect(&raw const identifier, &raw mut rect) } != 0 {
        return None;
    }
    Some((rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top))
}

/// Takes away the notification that's showing, if one is.
pub fn dismiss_notification() {
    let Some(notify) = super::notify_icon() else {
        return;
    };
    let (window, tray_icon, own) = with_desk(|desk| {
        desk.notification = None;
        (desk.window, desk.tray_icon, desk.notification_icon)
    });
    let Some(window) = window else {
        return;
    };
    let (id, icon) = match tray_icon {
        Some(icon) => (super::TRAY_ICON, icon),
        None if own => (super::NOTIFICATION_ICON, super::program_icon()),
        None => return,
    };
    // A notification with nothing in it takes the one that's showing away.
    let data = super::notification_data(window, id, "", "", icon);
    // SAFETY: as in `notify`.
    unsafe {
        let _ = notify(windows::Win32::UI::Shell::NIM_MODIFY, &raw const data);
    }
}

/// Where a program's own folders and the user's are, by the names a program uses for them.
pub fn temp_folder() -> PathBuf {
    std::env::temp_dir()
}

/// Says the screens have changed, or a setting, for what's listening, from the window the system tells.
pub(super) fn heard(message: u32) -> bool {
    let hear = with_desk(|desk| match message {
        WM_CLIPBOARDUPDATE => desk.clipboard_changed.clone(),
        WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED => desk.settings_changed.clone(),
        WM_DISPLAYCHANGE | WM_DPICHANGED => desk.displays_changed.clone(),
        WM_DEVICECHANGE => desk.devices_changed.clone(),
        _ => None,
    });
    if let Some(hear) = hear {
        hear();
        return true;
    }
    false
}

// Pictures of files and of the screen, the program's jump list, choosing a color or a font, and the sound devices.

/// COM, for as long as this is kept, on a thread that may not have it.
struct ComHere(bool);

impl ComHere {
    fn start() -> Self {
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
        // SAFETY: this is undone when it's dropped, on this same thread.
        Self(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
    }
}

impl Drop for ComHere {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: as above.
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

/// A picture: how wide and high it is, and its pixels, four bytes each, red first, with alpha as it is rather than multiplied in.
pub type Picture = (u32, u32, Vec<u8>);

/// What's in a bitmap, as the bytes Windows keeps it as: blue first.
///
/// # Safety
///
/// `bitmap` has to be a bitmap that isn't in a device context.
unsafe fn bitmap_pixels(bitmap: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, GetDC, GetDIBits, GetObjectW, ReleaseDC,
    };
    let mut about = BITMAP::default();
    // SAFETY: `about` is the size that's said.
    if unsafe { GetObjectW(bitmap.into(), size_of::<BITMAP>() as i32, Some((&raw mut about).cast())) } == 0 {
        return None;
    }
    let (width, height) = (about.bmWidth.max(0) as u32, about.bmHeight.unsigned_abs());
    if width == 0 || height == 0 {
        return None;
    }
    let mut header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            // From the top down.
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    // SAFETY: there's room for every row, as the header says, and the screen's context is given back.
    let got = unsafe {
        let screen = GetDC(None);
        let got = GetDIBits(screen, bitmap, 0, height, Some(pixels.as_mut_ptr().cast()), &raw mut header, DIB_RGB_COLORS);
        ReleaseDC(None, screen);
        got
    };
    (got != 0).then_some((width, height, pixels))
}

/// Blue-first pixels as red-first ones. With no alpha in any of them, as in a photo, they're all opaque; with `multiplied`,
/// the colors had the alpha multiplied in, which is taken out again.
fn red_first(mut pixels: Vec<u8>, multiplied: bool) -> Vec<u8> {
    let any_alpha = pixels.chunks_exact(4).any(|pixel| pixel[3] != 0);
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        if !any_alpha {
            pixel[3] = 255;
        } else if multiplied && pixel[3] != 0 && pixel[3] != 255 {
            let alpha = u32::from(pixel[3]);
            for color in &mut pixel[..3] {
                *color = ((u32::from(*color) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    pixels
}

/// What the shell shows for a file, at most `size` pixels across: its thumbnail, as Explorer shows a photo or a document, or
/// with `icon`, its icon. A file with no thumbnail gives none.
pub fn file_picture(path: &Path, size: u32, icon: bool) -> Result<Option<Picture>, String> {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::DeleteObject;
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY, SIIGBF_RESIZETOFIT, SIIGBF_THUMBNAILONLY,
    };
    let _com = ComHere::start();
    let name = windows::core::HSTRING::from(path.as_os_str());
    // SAFETY: the name lasts through the call.
    let factory: IShellItemImageFactory = unsafe { SHCreateItemFromParsingName(&name, None) }
        .map_err(|_| format!("there's nothing at {}", path.display()))?;
    let flags = if icon { SIIGBF_ICONONLY } else { SIIGBF_THUMBNAILONLY } | SIIGBF_RESIZETOFIT;
    let size = size.clamp(1, 2560) as i32;
    // SAFETY: the factory is the shell's, and the bitmap that comes back is this program's to let go of.
    let bitmap = match unsafe { factory.GetImage(SIZE { cx: size, cy: size }, flags) } {
        Ok(bitmap) => bitmap,
        Err(_) if !icon => return Ok(None),
        Err(error) => return Err(format!("the icon of {} couldn't be had: {error}", path.display())),
    };
    // SAFETY: it's in no device context, and it's let go of once it's read.
    let pixels = unsafe {
        let pixels = bitmap_pixels(bitmap);
        let _ = DeleteObject(bitmap.into());
        pixels
    };
    let (width, height, pixels) = pixels.ok_or_else(|| format!("the picture of {} couldn't be read", path.display()))?;
    Ok(Some((width, height, red_first(pixels, true))))
}

/// What's on the screen in this rectangle, in physical pixels from the main screen's top left, as the screens are laid out.
pub fn capture_screen(x: i32, y: i32, width: u32, height: u32) -> Result<Picture, String> {
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, ReleaseDC, SRCCOPY,
        SelectObject,
    };
    if width == 0 || height == 0 {
        return Err("there's nothing to take a picture of in a rectangle that size".to_owned());
    }
    let (wide, high) = (width.min(16_384) as i32, height.min(16_384) as i32);
    // SAFETY: what's made here is let go of here, and the bitmap's out of its context before it's read.
    let pixels = unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, wide, high);
        let before = SelectObject(memory, bitmap.into());
        let copied = BitBlt(memory, 0, 0, wide, high, Some(screen), x, y, SRCCOPY | CAPTUREBLT);
        SelectObject(memory, before);
        let pixels = copied.ok().and_then(|()| bitmap_pixels(bitmap));
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        pixels
    };
    let (width, height, mut pixels) = pixels.ok_or("the screen couldn't be read")?;
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    Ok((width, height, pixels))
}

/// Something in the program's jump list, which right-clicking its button on the taskbar shows: what it says, what the
/// program's started with when it's chosen, an icon file, and what's said when the pointer's over it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JumpItem {
    pub title: String,
    pub args: String,
    pub icon: Option<String>,
    pub description: String,
}

/// A heading in the jump list, with what's under it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JumpCategory {
    pub name: String,
    pub items: Vec<JumpItem>,
}

/// The name each link goes by in the jump list.
const PKEY_TITLE: windows::Win32::Foundation::PROPERTYKEY =
    windows::Win32::Foundation::PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0xF29F85E0_4FF9_1068_AB91_08002B27B3D9), pid: 2 };

/// Links that start `program` with each item's arguments, for the jump list.
///
/// # Safety
///
/// COM has to have been started on this thread.
unsafe fn jump_links(program: &str, items: &[JumpItem]) -> windows::core::Result<windows::Win32::UI::Shell::Common::IObjectCollection> {
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::Win32::UI::Shell::Common::IObjectCollection;
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{EnumerableObjectCollection, IShellLinkW, ShellLink};
    use windows::core::{HSTRING, Interface, PWSTR};
    // SAFETY: these are the shell's objects, made and used as it says, with what's given to them lasting through each call.
    unsafe {
        let links: IObjectCollection = CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
        for item in items {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(program))?;
            link.SetArguments(&HSTRING::from(item.args.as_str()))?;
            link.SetIconLocation(&HSTRING::from(item.icon.as_deref().unwrap_or(program)), 0)?;
            if !item.description.is_empty() {
                link.SetDescription(&HSTRING::from(item.description.as_str()))?;
            }
            let store: IPropertyStore = link.cast()?;
            let title = wide(&item.title);
            // The store copies the text, so it stays this program's. Dropping a `PROPVARIANT` clears it, which would give the
            // text to `CoTaskMemFree`, and the heap was corrupted the first time a jump list was set in a real window.
            let mut value = std::mem::ManuallyDrop::new(PROPVARIANT::default());
            (*value.Anonymous.Anonymous).vt = VT_LPWSTR;
            (*value.Anonymous.Anonymous).Anonymous.pwszVal = PWSTR(title.as_ptr().cast_mut());
            store.SetValue(&PKEY_TITLE, &raw const *value)?;
            store.Commit()?;
            links.AddObject(&link)?;
        }
        Ok(links)
    }
}

/// Puts these in the program's jump list, in place of what was there: `tasks` under Tasks, and each of `categories` under
/// its name. Each starts `program`.
pub fn set_jump_list(program: &str, tasks: &[JumpItem], categories: &[JumpCategory]) -> Result<(), String> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Shell::Common::IObjectArray;
    use windows::Win32::UI::Shell::{DestinationList, ICustomDestinationList};
    use windows::core::Interface;
    let _com = ComHere::start();
    let failed = |error: windows::core::Error| format!("the jump list couldn't be made: {error}");
    // SAFETY: COM's started, and the list is made and filled as the shell says.
    unsafe {
        let list: ICustomDestinationList = CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER).map_err(failed)?;
        let mut slots = 0u32;
        let _removed: IObjectArray = list.BeginList(&raw mut slots).map_err(failed)?;
        for category in categories.iter().filter(|category| !category.items.is_empty()) {
            let links: IObjectArray = jump_links(program, &category.items).and_then(|links| links.cast()).map_err(failed)?;
            if let Err(error) = list.AppendCategory(&windows::core::HSTRING::from(category.name.as_str()), &links) {
                let _ = list.AbortList();
                return Err(failed(error));
            }
        }
        if !tasks.is_empty() {
            let links: IObjectArray = jump_links(program, tasks).and_then(|links| links.cast()).map_err(failed)?;
            if let Err(error) = list.AddUserTasks(&links) {
                let _ = list.AbortList();
                return Err(failed(error));
            }
        }
        list.CommitList().map_err(failed)
    }
}

/// Takes everything the program put in its jump list out of it.
pub fn clear_jump_list() -> Result<(), String> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Shell::{DestinationList, ICustomDestinationList};
    let _com = ComHere::start();
    // SAFETY: as in `set_jump_list`.
    unsafe {
        let list: ICustomDestinationList = CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER).map_err(|error| error.to_string())?;
        list.DeleteList(windows::core::PCWSTR::null()).map_err(|error| format!("the jump list couldn't be cleared: {error}"))
    }
}

/// A function of the system's common dialogs, which are only loaded once one's wanted, so a program that never asks for a
/// color or a font doesn't load them to start up.
fn common_dialog(name: windows::core::PCSTR) -> Option<unsafe extern "system" fn() -> isize> {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    // SAFETY: the library stays loaded for good.
    unsafe {
        let library = LoadLibraryW(w!("comdlg32.dll")).ok()?;
        GetProcAddress(library, name)
    }
}

/// Runs a dialog on a thread of its own, as [`ask_for_path`](crate::dialogs::ask_for_path) does, with the window it belongs to.
fn on_dialog_thread<T: Send + 'static>(run: impl FnOnce(HWND) -> T + Send + 'static) -> Option<T> {
    let owner = crate::dialogs::current_window().0 as isize;
    std::thread::Builder::new()
        .name("rsui dialog".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || run(HWND(owner as *mut c_void)))
        .ok()?
        .join()
        .ok()
}

fn color_ref((red, green, blue): (u8, u8, u8)) -> windows::Win32::Foundation::COLORREF {
    windows::Win32::Foundation::COLORREF(u32::from(red) | u32::from(green) << 8 | u32::from(blue) << 16)
}

fn from_color_ref(color: windows::Win32::Foundation::COLORREF) -> (u8, u8, u8) {
    ((color.0 & 0xFF) as u8, (color.0 >> 8 & 0xFF) as u8, (color.0 >> 16 & 0xFF) as u8)
}

/// Asks the user to choose a color, starting from this one. It's the color, or nothing if they cancelled.
pub fn pick_color(initial: Option<(u8, u8, u8)>) -> Option<(u8, u8, u8)> {
    use windows::Win32::UI::Controls::Dialogs::{CC_ANYCOLOR, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW};
    type Choose = unsafe extern "system" fn(*mut CHOOSECOLORW) -> windows::core::BOOL;
    let found = common_dialog(windows::core::s!("ChooseColorW"))?;
    // SAFETY: the function has this signature.
    let choose: Choose = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, Choose>(found) };
    on_dialog_thread(move |owner| {
        let mut custom = [windows::Win32::Foundation::COLORREF(0x00FF_FFFF); 16];
        let mut asked = CHOOSECOLORW {
            lStructSize: size_of::<CHOOSECOLORW>() as u32,
            hwndOwner: owner,
            rgbResult: color_ref(initial.unwrap_or((0, 0, 0))),
            lpCustColors: custom.as_mut_ptr(),
            Flags: CC_RGBINIT | CC_FULLOPEN | CC_ANYCOLOR,
            ..Default::default()
        };
        // SAFETY: what it reads and fills lasts through the call.
        unsafe { choose(&raw mut asked) }.as_bool().then(|| from_color_ref(asked.rgbResult))
    })
    .flatten()
}

/// A font, as the user chose it: its family, its size in points, its weight from 100 to 900, whether it's italic, and its color.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FontChoice {
    pub family: String,
    pub size: f32,
    pub weight: u32,
    pub italic: bool,
    pub color: (u8, u8, u8),
}

/// Asks the user to choose a font, starting from this one. It's the font, or nothing if they cancelled.
pub fn pick_font(initial: Option<FontChoice>) -> Option<FontChoice> {
    use windows::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, LOGFONTW, LOGPIXELSY, ReleaseDC};
    use windows::Win32::UI::Controls::Dialogs::{
        CF_EFFECTS, CF_FORCEFONTEXIST, CF_INITTOLOGFONTSTRUCT, CF_SCREENFONTS, CHOOSEFONTW,
    };
    type Choose = unsafe extern "system" fn(*mut CHOOSEFONTW) -> windows::core::BOOL;
    let found = common_dialog(windows::core::s!("ChooseFontW"))?;
    // SAFETY: the function has this signature.
    let choose: Choose = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, Choose>(found) };
    on_dialog_thread(move |owner| {
        // SAFETY: this only asks, and gives the context back.
        let dots = unsafe {
            let screen = GetDC(None);
            let dots = GetDeviceCaps(Some(screen), LOGPIXELSY);
            ReleaseDC(None, screen);
            dots.max(1)
        };
        let mut font = LOGFONTW::default();
        let mut flags = CF_SCREENFONTS | CF_EFFECTS | CF_FORCEFONTEXIST;
        let mut color = (0, 0, 0);
        if let Some(initial) = &initial {
            flags |= CF_INITTOLOGFONTSTRUCT;
            font.lfHeight = -((initial.size * dots as f32 / 72.0).round() as i32);
            font.lfWeight = initial.weight as i32;
            font.lfItalic = u8::from(initial.italic);
            for (to, from) in font.lfFaceName.iter_mut().zip(initial.family.encode_utf16().take(31)) {
                *to = from;
            }
            color = initial.color;
        }
        let mut asked = CHOOSEFONTW {
            lStructSize: size_of::<CHOOSEFONTW>() as u32,
            hwndOwner: owner,
            lpLogFont: &raw mut font,
            Flags: flags,
            rgbColors: color_ref(color),
            ..Default::default()
        };
        // SAFETY: what it reads and fills lasts through the call.
        if !unsafe { choose(&raw mut asked) }.as_bool() {
            return None;
        }
        Some(FontChoice {
            family: from_wide(&font.lfFaceName),
            size: asked.iPointSize as f32 / 10.0,
            weight: if font.lfWeight <= 0 { 400 } else { font.lfWeight as u32 },
            italic: font.lfItalic != 0,
            color: from_color_ref(asked.rgbColors),
        })
    })
    .flatten()
}

/// A device that plays sound or records it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AudioDevice {
    /// What the system knows it by, which stays the same while it's plugged in and after.
    pub id: String,
    pub name: String,
    /// Whether it plays sound, rather than recording it.
    pub output: bool,
    /// Whether it's what the system uses for that unless a program says otherwise.
    pub default: bool,
}

/// What a device is called, as the sound settings show it.
const PKEY_DEVICE_FRIENDLY_NAME: windows::Win32::Foundation::PROPERTYKEY =
    windows::Win32::Foundation::PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0), pid: 14 };

fn device_id(device: &windows::Win32::Media::Audio::IMMDevice) -> Option<String> {
    // SAFETY: the id that comes back is this program's to free, which it is once it's copied.
    unsafe {
        let id = device.GetId().ok()?;
        let text = id.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(id.0 as *const c_void));
        text
    }
}

fn device_name(device: &windows::Win32::Media::Audio::IMMDevice) -> Option<String> {
    use windows::Win32::System::Com::STGM_READ;
    use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
    use windows::Win32::System::Variant::VT_LPWSTR;
    // SAFETY: the value that comes back is cleared once its text is copied, and its text is only read if that's what it is.
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let mut value = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME).ok()?;
        let inner = &*value.Anonymous.Anonymous;
        let text = (inner.vt == VT_LPWSTR).then(|| inner.Anonymous.pwszVal.to_string().ok()).flatten();
        let _ = PropVariantClear(&raw mut value);
        text
    }
}

/// The devices that play sound and record it that are there now, those that play first.
pub fn audio_devices() -> Result<Vec<AudioDevice>, String> {
    use windows::Win32::Media::Audio::{DEVICE_STATE_ACTIVE, IMMDeviceEnumerator, MMDeviceEnumerator, eCapture, eConsole, eRender};
    use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
    let _com = ComHere::start();
    let failed = |error: windows::core::Error| format!("the sound devices couldn't be listed: {error}");
    let mut out = Vec::new();
    // SAFETY: COM's started, and these are the system's objects, used as it says.
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(failed)?;
        for (flow, output) in [(eRender, true), (eCapture, false)] {
            let default = devices.GetDefaultAudioEndpoint(flow, eConsole).ok().and_then(|device| device_id(&device));
            let Ok(list) = devices.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) else {
                continue;
            };
            for at in 0..list.GetCount().unwrap_or(0) {
                let Ok(device) = list.Item(at) else {
                    continue;
                };
                let Some(id) = device_id(&device) else {
                    continue;
                };
                let name = device_name(&device).unwrap_or_default();
                out.push(AudioDevice { default: default.as_deref() == Some(id.as_str()), id, name, output });
            }
        }
    }
    Ok(out)
}

/// Runs this when a device is plugged in or taken out, which may be one that plays or records sound.
pub fn on_devices_change(hear: Action) -> Result<(), String> {
    desk_window()?;
    with_desk(|desk| desk.devices_changed = Some(hear));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_jump_list_s_links_are_made_and_let_go_of_without_freeing_the_title_twice() {
        use windows::Win32::UI::Shell::IShellLinkW;
        use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
        use windows::core::Interface;
        let _com = ComHere::start();
        let items = [JumpItem { title: "New note".to_owned(), args: "--new".to_owned(), icon: None, description: "Starts a note".to_owned() }];
        // Nothing's put in the system's jump list: the links are only made, read back and let go of. Freeing the title's text
        // when it was this program's, as it was, corrupted the heap here.
        // SAFETY: COM's started on this thread.
        let links = unsafe { jump_links(r"C:\rsui\app.exe", &items) }.unwrap();
        // SAFETY: the collection has the one link that was put in it.
        let link: IShellLinkW = unsafe { links.GetAt(0) }.unwrap();
        let store: IPropertyStore = link.cast().unwrap();
        // SAFETY: the value that comes back is the store's copy, which is the caller's to clear.
        let title = unsafe { store.GetValue(&PKEY_TITLE) }.unwrap();
        assert_eq!(title.to_string(), "New note");
    }
}

/// One thing in a folder, as Windows says while it reads the folder.
#[derive(Clone, Debug)]
pub struct FolderEntry {
    pub name: String,
    pub folder: bool,
    pub size: u64,
    /// Milliseconds since 1970.
    pub modified: f64,
    /// Milliseconds since 1970.
    pub created: f64,
    pub hidden: bool,
    pub system: bool,
    pub readonly: bool,
    /// A symbolic link or a junction.
    pub link: bool,
}

/// Names in the order Explorer shows them: letters without regard to case, and runs of digits by their value, so `file2`
/// comes before `file10`.
pub fn natural_order(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if a.is_ascii() && b.is_ascii() {
        return natural_order_ascii(a.as_bytes(), b.as_bytes());
    }
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let run = |chars: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(digit) = chars.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(digit);
                        chars.next();
                    }
                    digits
                };
                let (x, y) = (run(&mut a), run(&mut b));
                let (x, y) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let by_value = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
                if by_value != Ordering::Equal {
                    return by_value;
                }
            }
            (Some(x), Some(y)) => {
                let by_letter = x.to_lowercase().cmp(y.to_lowercase());
                if by_letter != Ordering::Equal {
                    return by_letter;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// [`natural_order`] for names that are all ASCII, as nearly all are, a byte at a time: sorting a folder of 25,000 names this way
/// takes a few milliseconds, where lowering each character on the way took ten times that.
fn natural_order_ascii(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let (x, y) = (a[i], b[j]);
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let run = |text: &[u8], mut at: usize| {
                while at < text.len() && text[at] == b'0' {
                    at += 1;
                }
                let start = at;
                while at < text.len() && text[at].is_ascii_digit() {
                    at += 1;
                }
                (start, at)
            };
            // Leading zeros are skipped, but the run starts where its digits do.
            let (x_start, x_end) = run(a, i);
            let (y_start, y_end) = run(b, j);
            let by_value = (x_end - x_start).cmp(&(y_end - y_start)).then_with(|| a[x_start..x_end].cmp(&b[y_start..y_end]));
            if by_value != Ordering::Equal {
                return by_value;
            }
            i = x_end;
            j = y_end;
            // `run` stopped at the zeros' end for a run that was only zeros, so step over any digits it didn't take.
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
        } else {
            let by_letter = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
            if by_letter != Ordering::Equal {
                return by_letter;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

/// What's in a folder, in one pass and in [`natural_order`]. Windows gives each name with its size, times and attributes as
/// it reads the folder, so nothing more is asked of each file, and a folder of 100,000 things lists in milliseconds rather
/// than seconds.
pub fn list_folder(path: &Path) -> Result<Vec<FolderEntry>, String> {
    use std::os::windows::fs::MetadataExt;
    let millis = |time: std::io::Result<std::time::SystemTime>| {
        time.ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0.0, |since| since.as_secs_f64() * 1000.0)
    };
    let entries = std::fs::read_dir(path).map_err(|error| format!("can't read {}: {error}", path.display()))?;
    let mut found = Vec::new();
    for entry in entries.flatten() {
        // On Windows this comes from what reading the folder gave, with no call of its own.
        let Ok(metadata) = entry.metadata() else { continue };
        let attributes = metadata.file_attributes();
        found.push(FolderEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            folder: attributes & 0x10 != 0,
            size: metadata.file_size(),
            modified: millis(metadata.modified()),
            created: millis(metadata.created()),
            hidden: attributes & 0x2 != 0,
            system: attributes & 0x4 != 0,
            readonly: attributes & 0x1 != 0,
            link: attributes & 0x400 != 0,
        });
    }
    found.sort_by(|a, b| natural_order(&a.name, &b.name).then_with(|| a.name.cmp(&b.name)));
    Ok(found)
}

/// A drive, as This PC shows it.
#[derive(Clone, Debug)]
pub struct Drive {
    /// Like `C:\`.
    pub path: String,
    pub label: String,
    /// `fixed`, `removable`, `network`, `optical`, `ram` or `unknown`.
    pub kind: &'static str,
    /// In bytes, or 0 when it can't be read, as an optical drive with nothing in it.
    pub total: u64,
    pub free: u64,
}

/// The drives there are. A network drive that isn't connected can take a while to answer, so this belongs on a thread.
pub fn drives() -> Vec<Drive> {
    use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
    use windows::core::PCWSTR;
    let mask = unsafe { GetLogicalDrives() };
    let mut found = Vec::new();
    for letter in 0..26u8 {
        if mask & (1 << letter) == 0 {
            continue;
        }
        let path = format!("{}:\\", (b'A' + letter) as char);
        let root = wide(&path);
        let at = PCWSTR(root.as_ptr());
        let kind = match unsafe { GetDriveTypeW(at) } {
            2 => "removable",
            3 => "fixed",
            4 => "network",
            5 => "optical",
            6 => "ram",
            _ => "unknown",
        };
        let mut label = [0u16; 261];
        let named = unsafe { GetVolumeInformationW(at, Some(&mut label), None, None, None, None) }.is_ok();
        let end = label.iter().position(|&unit| unit == 0).unwrap_or(label.len());
        let (mut free, mut total) = (0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(at, Some(&raw mut free), Some(&raw mut total), None) }.is_err() {
            (free, total) = (0, 0);
        }
        found.push(Drive {
            path,
            label: if named { String::from_utf16_lossy(&label[..end]) } else { String::new() },
            kind,
            total,
            free,
        });
    }
    found
}

#[cfg(test)]
mod listing_tests {
    use super::natural_order;
    use std::cmp::Ordering;

    #[test]
    fn names_sort_as_explorer_has_them() {
        let mut names = vec!["file10.txt", "File2.txt", "file1.txt", "a", "B", "file02b", "file2a", "x0010", "x9"];
        names.sort_by(|a, b| natural_order(a, b));
        assert_eq!(names, ["a", "B", "file1.txt", "File2.txt", "file2a", "file02b", "file10.txt", "x9", "x0010"]);
        assert_eq!(natural_order("abc", "ABC"), Ordering::Equal);
        assert_eq!(natural_order("a", "ab"), Ordering::Less);
    }

    #[test]
    fn a_folder_lists_in_one_pass() {
        let folder = std::env::temp_dir().join(format!("rsui-list-folder-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("sub")).unwrap();
        std::fs::write(folder.join("b10.txt"), "hello").unwrap();
        std::fs::write(folder.join("b9.txt"), "").unwrap();
        let found = super::list_folder(&folder).unwrap();
        let names: Vec<&str> = found.iter().map(|one| one.name.as_str()).collect();
        assert_eq!(names, ["b9.txt", "b10.txt", "sub"]);
        assert_eq!(found[1].size, 5);
        assert!(found[2].folder && !found[0].folder);
        assert!(found[1].modified > 0.0);
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
