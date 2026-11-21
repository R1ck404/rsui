//! The desktop's services on systems that don't have their own written yet: the same types and functions as on Windows
//! (`desktop.rs`), each of which says it can't be done here, or gives what's least surprising, so that a program written
//! for them still builds and runs, without them. This file is written by tools/gen_desktop_none.py from the Windows ones, so that the two
//! stay alike; each system's real ones replace these as they're written.
#![allow(unused_variables, dead_code, clippy::all)]

#[path = "desktop/keys.rs"]
mod keys;

use std::path::{Path, PathBuf};
use std::rc::Rc;

pub use keys::{Keys, parse_keys};

/// What each of these says when it can't be done here.
const NOT_HERE: &str = "this isn't on this system yet";

/// Something to run when something happens.
pub type Action = Rc<dyn Fn()>;

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

/// Not on this system yet.
pub fn add_global_shortcut(keys: &str, action: Action) -> Result<i32, String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn remove_global_shortcut(id: i32) {
}

/// Not on this system yet.
pub fn set_menu_bar(items: &[MenuItem]) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn set_tray(tray: Tray) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn remove_tray() {
}

/// Not on this system yet.
pub fn notify(title: &str, body: &str, on_click: Option<Action>) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn play_sound(id: SoundId, from_start: bool, ended: Option<Action>) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn beep(kind: &str) {
}

/// Not on this system yet.
pub fn login_command(program: &Path, arguments: &[String]) -> String {
    String::new()
}

/// Not on this system yet.
pub fn set_start_at_login(name: &str, command: Option<&str>) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn start_at_login_command(name: &str) -> Option<String> {
    None
}

/// Not on this system yet.
pub fn claim_single_instance(
    name: &str,
    arguments: &[String],
    hear: Rc<dyn Fn(Vec<String>)>,
) -> Result<bool, String> {
    Err(NOT_HERE.to_owned())
}

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

pub fn message_box(
    title: &str,
    text: &str,
    kind: MessageKind,
    buttons: MessageButtons,
) -> &'static str {
    // Nothing could be asked, so nothing was agreed to.
    "cancel"
}

/// Not on this system yet.
pub fn displays() -> Vec<Display> {
    Vec::new()
}

/// Not on this system yet.
pub fn pointer_on_screen() -> (i32, i32) {
    (0, 0)
}

pub fn set_app_id(id: &str) -> Result<(), String> {
    Ok(())
}

/// Not on this system yet.
pub fn keep_awake(on: bool) {
}

/// Not on this system yet.
pub fn on_battery() -> bool {
    false
}

/// Not on this system yet.
pub fn on_power(hear: Rc<dyn Fn(&'static str)>) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn replace_program(program: &Path, new: &Path) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn relaunch() -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn let_go_of_everything() {
}

pub fn open_path(path: &Path) -> Result<(), String> {
    open_with_system(path.as_os_str())
}

pub fn reveal(path: &Path) -> Result<(), String> {
    open_with_system(path.parent().unwrap_or(path).as_os_str())
}

/// Not on this system yet.
pub fn open_with(path: &Path) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn show_properties(path: &Path) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn trash(paths: &[PathBuf]) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn add_recent_documents(paths: &[PathBuf]) {
}

/// Not on this system yet.
pub fn clear_recent_documents() {
}

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

/// Not on this system yet.
pub fn read_clipboard() -> Result<ClipboardContents, String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn write_clipboard(contents: &ClipboardContents) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn clear_clipboard() -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn clipboard_formats() -> Result<Vec<String>, String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn on_clipboard_change(hear: Action) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn read_secret(target: &str) -> Option<String> {
    None
}

/// Not on this system yet.
pub fn write_secret(target: &str, user: &str, value: &str) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn delete_secret(target: &str) -> bool {
    false
}

pub fn locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty() && value != "C" && value != "POSIX")
        .map(|value| value.split(['.', '@']).next().unwrap_or_default().replace('_', "-"))
        .unwrap_or_else(|| "en-US".to_owned())
}

/// Not on this system yet.
pub fn idle_time() -> u32 {
    0
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
    Battery { on_battery: false, level: None, charging: false }
}

/// Not on this system yet.
pub fn accent_color() -> Option<String> {
    None
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

pub fn accessibility() -> Accessibility {
    Accessibility { reduced_motion: false, high_contrast: false, text_scale: 1.0 }
}

pub fn time_zone() -> Option<String> {
    if let Ok(zone) = std::env::var("TZ") {
        return Some(zone.trim_start_matches(':').to_owned()).filter(|zone| !zone.is_empty());
    }
    // `/etc/localtime` is a link into the zone files, whose path ends in the zone's name.
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let text = target.to_string_lossy();
    text.split_once("zoneinfo/").map(|(_, name)| name.to_owned())
}

/// Not on this system yet.
pub fn on_settings_change(hear: Action) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn on_displays_change(hear: Action) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn display_at(x: i32, y: i32) -> Option<Display> {
    None
}

/// Not on this system yet.
pub fn display_of_window(window: isize) -> Option<Display> {
    None
}

/// How the machine's connected: whether it can reach the internet, and whether the connection costs by how much is used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetworkStatus {
    pub online: bool,
    /// Only this machine, or a network with no way out of it, as the system says.
    pub local_only: bool,
    pub metered: bool,
}

pub fn network_status() -> NetworkStatus {
    // Not knowing, the network is taken to be there, as most programs would rather.
    NetworkStatus { online: true, local_only: false, metered: false }
}

/// Not on this system yet.
pub fn on_network_change(hear: Box<dyn Fn(NetworkStatus) + Send + Sync>) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn register_protocol(scheme: &str, description: &str, command: &str) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn unregister_protocol(scheme: &str) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn protocol_command(scheme: &str) -> Option<String> {
    None
}

/// Not on this system yet.
pub fn register_file_kind(extension: &str, kind: &str, description: &str, command: &str, icon: Option<&str>) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn unregister_file_kind(extension: &str, kind: &str) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn open_default_apps_settings() -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn popup_menu(items: &[MenuItem], at: Option<(i32, i32)>) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn drag_files(paths: &[PathBuf]) -> Result<&'static str, String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn tray_bounds() -> Option<(i32, i32, i32, i32)> {
    None
}

/// Not on this system yet.
pub fn dismiss_notification() {
}

pub fn temp_folder() -> PathBuf {
    std::env::temp_dir()
}

/// A picture: how wide and high it is, and its pixels, four bytes each, red first, with alpha as it is rather than multiplied in.
pub type Picture = (u32, u32, Vec<u8>);

/// Not on this system yet.
pub fn file_picture(path: &Path, size: u32, icon: bool) -> Result<Option<Picture>, String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn capture_screen(x: i32, y: i32, width: u32, height: u32) -> Result<Picture, String> {
    Err(NOT_HERE.to_owned())
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

/// Not on this system yet.
pub fn set_jump_list(program: &str, tasks: &[JumpItem], categories: &[JumpCategory]) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn clear_jump_list() -> Result<(), String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn pick_color(initial: Option<(u8, u8, u8)>) -> Option<(u8, u8, u8)> {
    None
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

/// Not on this system yet.
pub fn pick_font(initial: Option<FontChoice>) -> Option<FontChoice> {
    None
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

/// Not on this system yet.
pub fn audio_devices() -> Result<Vec<AudioDevice>, String> {
    Err(NOT_HERE.to_owned())
}

/// Not on this system yet.
pub fn on_devices_change(hear: Action) -> Result<(), String> {
    Err(NOT_HERE.to_owned())
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

pub fn list_folder(path: &Path) -> Result<Vec<FolderEntry>, String> {
    use std::os::unix::fs::PermissionsExt;
    let millis = |time: std::io::Result<std::time::SystemTime>| {
        time.ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0.0, |since| since.as_secs_f64() * 1000.0)
    };
    let entries = std::fs::read_dir(path).map_err(|error| format!("can't read {}: {error}", path.display()))?;
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(link) = entry.file_type() else { continue };
        // What a link points to is what's shown, as a file manager does, and a link that points nowhere is shown as itself.
        let Ok(metadata) = std::fs::metadata(entry.path()).or_else(|_| entry.metadata()) else { continue };
        found.push(FolderEntry {
            hidden: name.starts_with('.'),
            name,
            folder: metadata.is_dir(),
            size: metadata.len(),
            modified: millis(metadata.modified()),
            created: millis(metadata.created()),
            system: false,
            readonly: metadata.permissions().mode() & 0o222 == 0,
            link: link.is_symlink(),
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

pub fn drives() -> Vec<Drive> {
    // There are no drives by letter: the whole of the file system starts at `/`, and the person's own things are in home.
    let mut found = vec![Drive { path: "/".to_owned(), label: String::new(), kind: "fixed", total: 0, free: 0 }];
    if let Some(home) = std::env::var_os("HOME") {
        found.push(Drive { path: home.to_string_lossy().into_owned(), label: "Home".to_owned(), kind: "fixed", total: 0, free: 0 });
    }
    found
}

/// A sound that's been opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoundId(pub u32);

/// Not on this system yet.
pub fn open_sound(path: &Path) -> SoundId {
    SoundId(0)
}

/// Not on this system yet.
pub fn pause_sound(id: SoundId) {
}

/// Not on this system yet.
pub fn stop_sound(id: SoundId) {
}

/// Not on this system yet.
pub fn set_sound_volume(id: SoundId, volume: f32) {
}

/// Not on this system yet.
pub fn seek_sound(id: SoundId, at: u64) {
}

/// Not on this system yet.
pub fn close_sound(id: SoundId) {
}

/// Not on this system yet.
pub fn sound_length(id: SoundId) -> Option<u64> {
    None
}

/// Not on this system yet.
pub fn sound_position(id: SoundId) -> Option<u64> {
    None
}

/// Has the system open something with whatever it opens such things with: `xdg-open` on Linux, `open` on macOS.
fn open_with_system(what: &std::ffi::OsStr) -> Result<(), String> {
    let opener = if cfg!(target_vendor = "apple") { "open" } else { "xdg-open" };
    std::process::Command::new(opener)
        .arg(what)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("{opener} couldn't be started: {error}"))
}
