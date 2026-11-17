//! What a program can ask of the desktop. All of it can be seen, heard, or lasts, so it only really happens for a program that's
//! running in a real window. Otherwise, such as with `--do` and `--agent` and in tests, it's said instead, and what would have come
//! back from the system can be made to happen with the steps here. That way, an app can be checked without anything coming up.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use rsui::desktop as system;
use rsui::desktop::{Action, MenuItem, MessageButtons, MessageKind, SoundId, Tray};
use rsui_js::{Args, HostFunction, Value, Vm};

use crate::bridge;

/// What would've come back from the system, if this were real.
#[derive(Default)]
struct Pretend {
    shortcuts: HashMap<i32, (system::Keys, u32)>,
    next: i32,
    tray: Option<u32>,
    tray_items: Vec<(String, u32)>,
    menu_items: Vec<(String, u32)>,
    notification: u32,
    second_instance: u32,
    power: u32,
    /// What's playing, and what to tell when it's done.
    playing: HashMap<u32, u32>,
    start_at_login: Option<String>,
    /// What's in the menu that was last shown where the pointer is, by its path.
    popup_items: Vec<(String, u32)>,
}

thread_local! {
    static REAL: Cell<bool> = const { Cell::new(false) };
    static PRETEND: RefCell<Pretend> = RefCell::default();
    /// Whether this program's already found that it's the only one, as a new version of its code asks again.
    static CLAIMED: Cell<bool> = const { Cell::new(false) };
}

/// From here on, what's asked of the desktop really happens, which is only for a program that's really running.
pub(crate) fn allow_real_effects() {
    REAL.with(|real| real.set(true));
}

pub(crate) fn real() -> bool {
    REAL.with(Cell::get)
}

fn with_pretend<R>(run: impl FnOnce(&mut Pretend) -> R) -> R {
    PRETEND.with(|pretend| run(&mut pretend.borrow_mut()))
}

/// Everything goes, for a new version of the app's code to ask for it again.
pub(crate) fn let_go_of_everything() {
    if real() {
        system::let_go_of_everything();
    }
    with_pretend(|pretend| {
        let kept = pretend.start_at_login.take();
        *pretend = Pretend::default();
        pretend.start_at_login = kept;
    });
}

pub(crate) fn action(id: u32) -> Option<Action> {
    (id != 0).then(|| Rc::new(move || bridge::fire_desktop_event(id, None)) as Action)
}

pub(crate) fn number_at(vm: &mut Vm, args: Args, at: usize) -> u32 {
    let value = args.get(vm, at);
    if value.is_number() {
        value.as_number() as u32
    } else {
        0
    }
}

pub(crate) fn text_at(vm: &mut Vm, args: Args, at: usize) -> String {
    let value = args.get(vm, at);
    vm.plain_text(value).unwrap_or_default()
}

pub(crate) fn failed(vm: &mut Vm, message: &str) -> Value {
    vm.type_error(message)
}

fn host_global_shortcut(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let keys = text_at(vm, args, 0);
    let id = number_at(vm, args, 1);
    if real() {
        let handle = system::add_global_shortcut(&keys, action(id).expect("it always has one"))
            .map_err(|message| failed(vm, &message))?;
        return Ok(Value::int(handle));
    }
    let parsed = system::parse_keys(&keys).map_err(|message| failed(vm, &message))?;
    if with_pretend(|pretend| pretend.shortcuts.values().any(|(held, _)| *held == parsed)) {
        return Err(failed(
            vm,
            &format!("{keys} is already in use by something else"),
        ));
    }
    bridge::say(format!("it asked for the global shortcut {keys}"));
    let handle = with_pretend(|pretend| {
        pretend.next += 1;
        pretend.shortcuts.insert(pretend.next, (parsed, id));
        pretend.next
    });
    Ok(Value::int(handle))
}

fn host_remove_global_shortcut(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let handle = number_at(vm, args, 0) as i32;
    if real() {
        system::remove_global_shortcut(handle);
    } else {
        with_pretend(|pretend| pretend.shortcuts.remove(&handle));
    }
    Ok(Value::UNDEFINED)
}

/// Reads a menu from what the layer made of it, with each item's action as a number. `names` is told what's there, by its path.
pub(crate) fn read_menu(
    vm: &mut Vm,
    list: Value,
    before: &str,
    names: &mut Vec<(String, u32)>,
) -> Result<Vec<MenuItem>, Value> {
    let Some(items) = vm.array_elements(list).map(<[Value]>::to_vec) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let get = |vm: &mut Vm, name: &str| {
            let name = vm.atom(name);
            vm.get_property(item, name)
        };
        let label = get(vm, "label")?;
        let label = vm.plain_text(label).unwrap_or_default();
        let shortcut = get(vm, "shortcut")?;
        let shortcut = vm.string_text(shortcut).map(|keys| keys.into_owned());
        let disabled = get(vm, "disabled")? == Value::TRUE;
        let checked = get(vm, "checked")? == Value::TRUE;
        let separator = get(vm, "separator")? == Value::TRUE;
        let id = get(vm, "action")?;
        let id = if id.is_number() {
            id.as_number() as u32
        } else {
            0
        };
        let inner = get(vm, "items")?;
        let path = if before.is_empty() {
            label.clone()
        } else {
            format!("{before}/{label}")
        };
        let items = read_menu(vm, inner, &path, names)?;
        if !separator && items.is_empty() {
            names.push((path, id));
        }
        out.push(MenuItem {
            label,
            shortcut,
            disabled,
            checked,
            separator,
            items,
            action: action(id),
        });
    }
    Ok(out)
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

/// `setTray(tooltip, the icon's bytes as hex, what a click does, the menu)`.
fn host_set_tray(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let tooltip = text_at(vm, args, 0);
    let icon = text_at(vm, args, 1);
    let click = number_at(vm, args, 2);
    let menu = args.get(vm, 3);
    let mut names = Vec::new();
    let menu = read_menu(vm, menu, "", &mut names)?;
    if real() {
        system::set_tray(Tray {
            tooltip,
            icon: from_hex(&icon),
            on_click: action(click),
            menu,
        })
        .map_err(|message| failed(vm, &message))?;
    } else {
        let said: Vec<&str> = names.iter().map(|(name, _)| name.as_str()).collect();
        bridge::say(format!(
            "it asked for a tray icon, {tooltip:?}, with this in its menu: {said:?}"
        ));
        with_pretend(|pretend| {
            pretend.tray = Some(click);
            pretend.tray_items = names;
        });
    }
    Ok(Value::UNDEFINED)
}

fn host_remove_tray(_: &mut Vm, _: Args) -> Result<Value, Value> {
    if real() {
        system::remove_tray();
    } else {
        bridge::say("it took away its tray icon".to_owned());
        with_pretend(|pretend| {
            pretend.tray = None;
            pretend.tray_items.clear();
        });
    }
    Ok(Value::UNDEFINED)
}

fn host_set_menu_bar(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let menu = args.get(vm, 0);
    let mut names = Vec::new();
    let menu = read_menu(vm, menu, "", &mut names)?;
    if real() {
        system::set_menu_bar(&menu).map_err(|message| failed(vm, &message))?;
    } else {
        let said: Vec<&str> = names.iter().map(|(name, _)| name.as_str()).collect();
        bridge::say(format!("it asked for a menu bar with this in it: {said:?}"));
        with_pretend(|pretend| pretend.menu_items = names);
    }
    Ok(Value::UNDEFINED)
}

fn host_notify(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (title, body) = (text_at(vm, args, 0), text_at(vm, args, 1));
    let click = number_at(vm, args, 2);
    if real() {
        system::notify(&title, &body, action(click)).map_err(|message| failed(vm, &message))?;
    } else {
        bridge::say(format!("it asked for a notification: {title:?}, {body:?}"));
        with_pretend(|pretend| pretend.notification = click);
    }
    Ok(Value::UNDEFINED)
}

fn host_sound_open(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let path = text_at(vm, args, 0);
    if real() {
        let path = crate::pack::on_the_disk(std::path::Path::new(&path));
        return Ok(Value::number(f64::from(system::open_sound(&path).0)));
    }
    let id = with_pretend(|pretend| {
        pretend.next += 1;
        pretend.next
    });
    bridge::say(format!("it opened a sound: {path}"));
    Ok(Value::int(id))
}

/// `soundPlay(id, from the start, what to tell when it's over)`.
fn host_sound_play(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    let from_start = args.get(vm, 1) == Value::TRUE;
    let ended = number_at(vm, args, 2);
    if real() {
        system::play_sound(SoundId(id), from_start, action(ended))
            .map_err(|message| failed(vm, &message))?;
    } else {
        bridge::say(format!("it played sound {id}, which wasn't heard"));
        with_pretend(|pretend| pretend.playing.insert(id, ended));
    }
    Ok(Value::UNDEFINED)
}

/// `soundDo(id, what)`, for `pause`, `stop` and `close`.
fn host_sound_do(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    let what = text_at(vm, args, 1);
    if real() {
        let id = SoundId(id);
        match what.as_str() {
            "pause" => system::pause_sound(id),
            "stop" => system::stop_sound(id),
            _ => system::close_sound(id),
        }
    } else {
        with_pretend(|pretend| pretend.playing.remove(&id));
    }
    Ok(Value::UNDEFINED)
}

fn host_sound_volume(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    let volume = args.get(vm, 1);
    if real() && volume.is_number() {
        system::set_sound_volume(SoundId(id), volume.as_number() as f32);
    }
    Ok(Value::UNDEFINED)
}

fn host_sound_seek(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    let at = args.get(vm, 1);
    if real() && at.is_number() {
        system::seek_sound(SoundId(id), at.as_number().max(0.0) as u64);
    }
    Ok(Value::UNDEFINED)
}

/// `soundAsk(id, what)`, for `length` and `position`, in milliseconds.
fn host_sound_ask(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = SoundId(number_at(vm, args, 0));
    let what = text_at(vm, args, 1);
    if !real() {
        return Ok(Value::NULL);
    }
    let answer = if what == "length" {
        system::sound_length(id)
    } else {
        system::sound_position(id)
    };
    Ok(answer.map_or(Value::NULL, |answer| Value::number(answer as f64)))
}

fn host_beep(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let kind = text_at(vm, args, 0);
    if real() {
        system::beep(&kind);
    } else {
        bridge::say(format!("it beeped: {kind}, which wasn't heard"));
    }
    Ok(Value::UNDEFINED)
}

/// `setStartAtLogin(on, what to start it with)`.
fn host_set_start_at_login(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let on = args.get(vm, 0) == Value::TRUE;
    let extra = args.get(vm, 1);
    let extra = vm
        .array_elements(extra)
        .map(<[Value]>::to_vec)
        .unwrap_or_default();
    let mut arguments: Vec<String> = if rsui_js::running_packed() {
        Vec::new()
    } else {
        // What's running this is the tool, which has to be told which app.
        vec![bridge::app_file().display().to_string()]
    };
    arguments.extend(extra.into_iter().filter_map(|value| vm.plain_text(value)));
    let program = std::env::current_exe().map_err(|error| failed(vm, &error.to_string()))?;
    let command = on.then(|| system::login_command(&program, &arguments));
    if real() {
        system::set_start_at_login(&bridge::program_name(), command.as_deref())
            .map_err(|message| failed(vm, &message))?;
    } else {
        match &command {
            Some(command) => bridge::say(format!("it asked to start at login, with {command}")),
            None => bridge::say("it asked not to start at login".to_owned()),
        }
        with_pretend(|pretend| pretend.start_at_login = command);
    }
    Ok(Value::UNDEFINED)
}

fn host_starts_at_login(_: &mut Vm, _: Args) -> Result<Value, Value> {
    let is_on = if real() {
        system::start_at_login_command(&bridge::program_name()).is_some()
    } else {
        with_pretend(|pretend| pretend.start_at_login.is_some())
    };
    Ok(Value::bool(is_on))
}

/// `singleInstance(what to tell if another starts)`.
fn host_single_instance(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    if !real() {
        bridge::say("it asked to be the only one of this program, and it is".to_owned());
        with_pretend(|pretend| pretend.second_instance = id);
        return Ok(Value::TRUE);
    }
    // A new version of its code asks again.
    if CLAIMED.with(Cell::get) {
        return Ok(Value::TRUE);
    }
    // What's on the command line of something that isn't packed is for the tool.
    let arguments: Vec<String> = if rsui_js::running_packed() {
        std::env::args().skip(1).collect()
    } else {
        Vec::new()
    };
    let first = system::claim_single_instance(
        &bridge::program_name(),
        &arguments,
        Rc::new(move |arguments: Vec<String>| {
            if id != 0 {
                bridge::fire_desktop_event(id, Some(arguments.join("\n")));
            }
        }),
    )
    .map_err(|message| failed(vm, &message))?;
    CLAIMED.with(|claimed| claimed.set(first));
    Ok(Value::bool(first))
}

fn message_options(vm: &mut Vm, args: Args) -> (String, String, MessageKind, MessageButtons) {
    let (title, text) = (text_at(vm, args, 0), text_at(vm, args, 1));
    let kind = match text_at(vm, args, 2).as_str() {
        "warning" => MessageKind::Warning,
        "error" => MessageKind::Error,
        "question" => MessageKind::Question,
        _ => MessageKind::Info,
    };
    let buttons = match text_at(vm, args, 3).as_str() {
        "ok-cancel" => MessageButtons::OkCancel,
        "yes-no" => MessageButtons::YesNo,
        "yes-no-cancel" => MessageButtons::YesNoCancel,
        "retry-cancel" => MessageButtons::RetryCancel,
        _ => MessageButtons::Ok,
    };
    (title, text, kind, buttons)
}

/// What a message box gets when nobody's there. It's the answer that's been said to give, which is a good way to check what
/// happens on each, and otherwise it was cancelled.
fn pretend_answer(title: &str, text: &str) -> String {
    let answer = bridge::next_dialog_answer().unwrap_or("cancel".to_owned());
    bridge::say(format!("it asked {title:?}, {text:?}, and got {answer}"));
    answer
}

/// `messageBox(title, text, kind, buttons)`, which waits for the answer, and holds everything up meanwhile.
fn host_message_box_now(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (title, text, kind, buttons) = message_options(vm, args);
    let answer = if real() {
        system::message_box(&title, &text, kind, buttons).to_owned()
    } else {
        pretend_answer(&title, &text)
    };
    Ok(vm.new_string(&answer))
}

/// `messageBoxLater(title, text, kind, buttons)`, for a promise of the answer.
fn host_message_box_later(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (title, text, kind, buttons) = message_options(vm, args);
    if !real() {
        let answer = pretend_answer(&title, &text);
        return Ok(vm.later(move || Ok(Some(answer))));
    }
    Ok(vm.later(move || {
        Ok(Some(
            system::message_box(&title, &text, kind, buttons).to_owned(),
        ))
    }))
}

fn host_displays(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let mut list = Vec::new();
    for display in system::displays() {
        let one = vm.new_object();
        let set = |vm: &mut Vm, name: &str, value: Value| {
            let name = vm.atom(name);
            vm.set_property(one, name, value)
        };
        let (x, y, width, height) = display.bounds;
        let (work_x, work_y, work_width, work_height) = display.work_area;
        for (name, value) in [
            ("x", x),
            ("y", y),
            ("width", width),
            ("height", height),
            ("workX", work_x),
            ("workY", work_y),
            ("workWidth", work_width),
            ("workHeight", work_height),
        ] {
            set(vm, name, Value::int(value))?;
        }
        set(vm, "scale", Value::number(f64::from(display.scale)))?;
        set(vm, "primary", Value::bool(display.primary))?;
        list.push(one);
    }
    Ok(vm.new_array(list))
}

fn host_pointer_on_screen(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let (x, y) = system::pointer_on_screen();
    Ok(vm.new_array(vec![Value::int(x), Value::int(y)]))
}

fn host_keep_awake(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let on = args.get(vm, 0) == Value::TRUE;
    if real() {
        system::keep_awake(on);
    } else {
        bridge::say(format!("it asked to keep the machine awake: {on}"));
    }
    Ok(Value::UNDEFINED)
}

fn host_on_battery(_: &mut Vm, _: Args) -> Result<Value, Value> {
    Ok(Value::bool(system::on_battery()))
}

fn host_on_power(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    if real() {
        system::on_power(Rc::new(move |what: &'static str| {
            bridge::fire_desktop_event(id, Some(what.to_owned()));
        }))
        .map_err(|message| failed(vm, &message))?;
    } else {
        with_pretend(|pretend| pretend.power = id);
    }
    Ok(Value::UNDEFINED)
}

fn host_is_packed(_: &mut Vm, _: Args) -> Result<Value, Value> {
    Ok(Value::bool(rsui_js::running_packed()))
}

fn host_app_file(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let file = bridge::app_file().display().to_string();
    Ok(vm.new_string(&file))
}

/// A GUID, as Windows lays it out.
const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> [u8; 16] {
    let (a, b, c) = (a.to_le_bytes(), b.to_le_bytes(), c.to_le_bytes());
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}

/// The user's own folders that [`user_folder`] knows, by the names Electron gives them, with the ids Windows knows them by.
#[rustfmt::skip]
const USER_FOLDERS: [(&str, [u8; 16]); 7] = [
    ("desktop", guid(0xB4BF_CC3A, 0xDB2C, 0x424C, [0xB0, 0x29, 0x7F, 0xE9, 0x9A, 0x87, 0xC6, 0x41])),
    ("documents", guid(0xFDD3_9AD0, 0x238F, 0x46AF, [0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7])),
    ("downloads", guid(0x374D_E290, 0x123F, 0x4565, [0x91, 0x64, 0x39, 0xC4, 0x92, 0x5E, 0x46, 0x7B])),
    ("music", guid(0x4BD8_D571, 0x6D19, 0x48D3, [0xBE, 0x97, 0x42, 0x22, 0x20, 0x08, 0x0E, 0x43])),
    ("pictures", guid(0x33E2_8130, 0x4E1E, 0x4676, [0x83, 0x5A, 0x98, 0x39, 0x5C, 0x3B, 0xC3, 0xBB])),
    ("videos", guid(0x1898_9B1D, 0x99B5, 0x455B, [0x84, 0x1C, 0xAB, 0x7C, 0x74, 0xE4, 0xDD, 0xFC])),
    ("recent", guid(0xAE50_C081, 0xEBD2, 0x438A, [0x86, 0x55, 0x8A, 0x09, 0x2E, 0x34, 0x98, 0x7A])),
];

/// Where Windows says one of the user's own folders is, which can be somewhere else than in their home, as when OneDrive
/// keeps their documents: `desktop`, `documents`, `downloads`, `music`, `pictures`, `videos` or `recent`.
pub(crate) fn user_folder(name: &str) -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "shell32.dll", kind = "raw-dylib", modifiers = "+verbatim")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(
            id: *const [u8; 16],
            flags: u32,
            token: *mut std::ffi::c_void,
            path: *mut *mut u16,
        ) -> i32;
    }
    #[link(name = "ole32.dll", kind = "raw-dylib", modifiers = "+verbatim")]
    unsafe extern "system" {
        fn CoTaskMemFree(memory: *mut std::ffi::c_void);
    }
    let (_, id) = USER_FOLDERS.iter().find(|(named, _)| *named == name)?;
    let mut path: *mut u16 = std::ptr::null_mut();
    // SAFETY: the id is a GUID's 16 bytes, and what comes back is a string that ends in a zero, which is freed here, as
    // it has to be even when it fails.
    unsafe {
        let failed = SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut path) != 0;
        let found = (!failed && !path.is_null()).then(|| {
            let length = (0..).take_while(|&at| *path.add(at) != 0).count();
            std::ffi::OsString::from_wide(std::slice::from_raw_parts(path, length))
        });
        if !path.is_null() {
            CoTaskMemFree(path.cast());
        }
        found.map(std::path::PathBuf::from)
    }
}

fn host_user_folder(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let name = text_at(vm, args, 0);
    Ok(match user_folder(&name) {
        Some(folder) => vm.new_string(&folder.to_string_lossy()),
        None => Value::UNDEFINED,
    })
}

fn host_replace_program(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let new = text_at(vm, args, 0);
    if !real() {
        bridge::say(format!(
            "it asked to replace this program with {new}, which wasn't done"
        ));
        return Ok(Value::UNDEFINED);
    }
    if !rsui_js::running_packed() {
        return Err(failed(vm, "only a packed program can replace itself"));
    }
    let program = std::env::current_exe().map_err(|error| failed(vm, &error.to_string()))?;
    system::replace_program(&program, std::path::Path::new(&new))
        .map_err(|message| failed(vm, &message))?;
    // The new version has to say it works once it's running, or the one before it comes back: see
    // `roll_back_an_unconfirmed_update`.
    let _ = std::fs::write(crate::unconfirmed_marker(&program), "0");
    Ok(Value::UNDEFINED)
}

fn host_relaunch(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    if real() {
        system::relaunch().map_err(|message| failed(vm, &message))?;
    } else {
        bridge::say("it asked to start again, which wasn't done".to_owned());
    }
    Ok(Value::UNDEFINED)
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 28] = [
    ("globalShortcut", host_global_shortcut),
    ("removeGlobalShortcut", host_remove_global_shortcut),
    ("setTray", host_set_tray),
    ("removeTray", host_remove_tray),
    ("setMenuBar", host_set_menu_bar),
    ("notify", host_notify),
    ("soundOpen", host_sound_open),
    ("soundPlay", host_sound_play),
    ("soundDo", host_sound_do),
    ("soundVolume", host_sound_volume),
    ("soundSeek", host_sound_seek),
    ("soundAsk", host_sound_ask),
    ("beep", host_beep),
    ("setStartAtLogin", host_set_start_at_login),
    ("startsAtLogin", host_starts_at_login),
    ("singleInstance", host_single_instance),
    ("messageBoxNow", host_message_box_now),
    ("messageBoxLater", host_message_box_later),
    ("displays", host_displays),
    ("pointerOnScreen", host_pointer_on_screen),
    ("keepAwake", host_keep_awake),
    ("onBattery", host_on_battery),
    ("onPower", host_on_power),
    ("isPacked", host_is_packed),
    ("appFile", host_app_file),
    ("userFolder", host_user_folder),
    ("replaceProgram", host_replace_program),
    ("relaunch", host_relaunch),
];

// What a person or the system would do, for `--do`.

fn fire(id: u32, value: Option<String>) -> Result<(), String> {
    if id == 0 {
        return Err("nothing's listening for that".to_owned());
    }
    bridge::fire_desktop_event(id, value);
    Ok(())
}

/// Presses these keys, wherever they'd be pressed.
pub(crate) fn press_global(keys: &str) -> Result<(), String> {
    let parsed = system::parse_keys(keys)?;
    let id = with_pretend(|pretend| {
        pretend
            .shortcuts
            .values()
            .find(|(held, _)| *held == parsed)
            .map(|(_, id)| *id)
    });
    fire(
        id.ok_or_else(|| format!("nothing has asked for the global shortcut {keys}"))?,
        None,
    )
}

fn chosen(items: &[(String, u32)], name: &str, what: &str) -> Result<(), String> {
    let found = items.iter().find(|(item, _)| item == name);
    match found {
        Some((_, id)) => fire(*id, None),
        None => {
            let there: Vec<&str> = items.iter().map(|(item, _)| item.as_str()).collect();
            Err(format!("{what} doesn't have {name:?}. It has {there:?}"))
        }
    }
}

/// Clicks on the tray icon, or on this in its menu.
pub(crate) fn click_tray(item: Option<&str>) -> Result<(), String> {
    let (click, items) = with_pretend(|pretend| (pretend.tray, pretend.tray_items.clone()));
    let click = click.ok_or("there's no tray icon")?;
    match item {
        Some(item) => chosen(&items, item, "the tray's menu"),
        None => fire(click, None),
    }
}

/// Forgets the notification that was said to be showing, as it's been taken away.
pub(crate) fn forget_pretend_notification() {
    with_pretend(|pretend| pretend.notification = 0);
}

/// Keeps what's in a menu shown where the pointer is, for `popup` to choose from.
pub(crate) fn set_pretend_menu(items: Vec<(String, u32)>) {
    with_pretend(|pretend| pretend.popup_items = items);
}

/// Chooses this from the menu that was last shown where the pointer is, as `More/Deep`.
pub(crate) fn click_popup(item: &str) -> Result<(), String> {
    let items = with_pretend(|pretend| std::mem::take(&mut pretend.popup_items));
    if items.is_empty() {
        return Err("no menu is showing".to_owned());
    }
    chosen(&items, item, "the menu")
}

/// Clicks on this in the menu bar, as `File/Open`.
pub(crate) fn click_menu_bar(item: &str) -> Result<(), String> {
    let items = with_pretend(|pretend| pretend.menu_items.clone());
    chosen(&items, item, "the menu bar")
}

/// Clicks on the last notification.
pub(crate) fn click_notification() -> Result<(), String> {
    let id = with_pretend(|pretend| std::mem::take(&mut pretend.notification));
    fire(id, None)
}

/// Gets every sound to its end.
pub(crate) fn end_sounds() -> Result<(), String> {
    let playing: Vec<u32> =
        with_pretend(|pretend| pretend.playing.drain().map(|(_, id)| id).collect());
    for id in playing.into_iter().filter(|id| *id != 0) {
        fire(id, None)?;
    }
    Ok(())
}

/// Starts this program a second time, with these.
pub(crate) fn second_instance(arguments: &[String]) -> Result<(), String> {
    let id = with_pretend(|pretend| pretend.second_instance);
    fire(id, Some(arguments.join("\n")))
}

/// Says what's happening to the power.
pub(crate) fn power(what: &str) -> Result<(), String> {
    let id = with_pretend(|pretend| pretend.power);
    fire(id, Some(what.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::from_hex;

    #[test]
    fn hex_comes_back_as_bytes_and_anything_else_as_nothing() {
        assert_eq!(from_hex("00ff10"), Some(vec![0, 255, 16]));
        assert_eq!(from_hex(""), None);
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
    }
}
