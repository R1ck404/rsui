//! More of what a program can ask of the desktop, for `rsui/desktop`: the shell, the clipboard's formats, secrets, what the system
//! says about itself, the program's links and kinds of file, and its windows' state. As in [`native`](crate::native), it only
//! really happens for a program running in a real window; otherwise it's said, and pretended. With `RSUI_SANDBOX` set to a
//! folder, what would last in the user's settings or credentials is kept in that folder instead, even for a real window.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use rsui::desktop as system;
use rsui_js::{Args, HostFunction, Value, Vm};

use crate::bridge;
use crate::native::{failed, number_at, read_menu, real, text_at};

/// What would have lasted, for a program that's being checked, or that's sandboxed.
#[derive(Default)]
struct Kept {
    clipboard: system::ClipboardContents,
    secrets: BTreeMap<String, String>,
    protocols: BTreeMap<String, String>,
    file_kinds: BTreeMap<String, String>,
    recent: Vec<String>,
    /// What the jump list was set to, as it was given.
    jump_list: Option<serde_json::Value>,
}

thread_local! {
    static KEPT: RefCell<Option<Kept>> = const { RefCell::new(None) };
}

fn with_kept<R>(run: impl FnOnce(&mut Kept) -> R) -> R {
    KEPT.with(|kept| {
        let mut kept = kept.borrow_mut();
        let kept = kept.get_or_insert_with(load_sandbox);
        let out = run(kept);
        save_sandbox(kept);
        out
    })
}

/// The folder a sandboxed program keeps what it would have kept in the user's settings in.
fn sandbox() -> Option<PathBuf> {
    std::env::var_os("RSUI_SANDBOX").filter(|folder| !folder.is_empty()).map(PathBuf::from)
}

/// Whether what's asked is done for real: in a real window, unless it's sandboxed.
fn for_real() -> bool {
    real() && sandbox().is_none()
}

fn sandbox_file() -> Option<PathBuf> {
    sandbox().map(|folder| folder.join("rsui-desktop.json"))
}

fn load_sandbox() -> Kept {
    let mut kept = Kept::default();
    let Some(file) = sandbox_file() else {
        return kept;
    };
    let Ok(text) = std::fs::read_to_string(file) else {
        return kept;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return kept;
    };
    let map = |name: &str| {
        json.get(name)
            .and_then(|one| one.as_object())
            .map(|one| one.iter().filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned()))).collect())
            .unwrap_or_default()
    };
    kept.secrets = map("secrets");
    kept.protocols = map("protocols");
    kept.file_kinds = map("fileKinds");
    kept.jump_list = json.get("jumpList").filter(|list| !list.is_null()).cloned();
    kept
}

fn save_sandbox(kept: &Kept) {
    let Some(file) = sandbox_file() else {
        return;
    };
    let json = serde_json::json!({ "secrets": kept.secrets, "protocols": kept.protocols, "fileKinds": kept.file_kinds, "recent": kept.recent, "jumpList": kept.jump_list });
    if let Some(folder) = file.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    let _ = std::fs::write(file, json.to_string());
}

fn set(vm: &mut Vm, object: Value, name: &str, value: Value) -> Result<(), Value> {
    let name = vm.atom(name);
    vm.set_property(object, name, value)
}

fn text_or_undefined(vm: &mut Vm, text: Option<&str>) -> Value {
    text.map_or(Value::UNDEFINED, |text| vm.new_string(text))
}

fn texts_at(vm: &mut Vm, args: Args, at: usize) -> Vec<String> {
    let list = args.get(vm, at);
    let Some(items) = vm.array_elements(list).map(<[Value]>::to_vec) else {
        return Vec::new();
    };
    items.into_iter().filter_map(|item| vm.plain_text(item)).collect()
}

fn path_at(vm: &mut Vm, args: Args, at: usize) -> PathBuf {
    PathBuf::from(text_at(vm, args, at))
}

/// Runs a shell call for real, on a thread of its own, as a promise, or says it. The shell waits for Explorer, or whatever opens
/// the file, to have started, and the window shouldn't stop for that, as it doesn't for `open_url`.
fn shell(vm: &mut Vm, what: &str, path: &str, run: impl FnOnce() -> Result<(), String> + Send + 'static) -> Result<Value, Value> {
    if for_real() {
        return Ok(vm.later(move || run().map(|()| None)));
    }
    bridge::say(format!("it asked to {what} {path}, which wasn't done"));
    Ok(Value::UNDEFINED)
}

fn host_shell_open_path(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let path = path_at(vm, args, 0);
    if !path.exists() {
        return Err(failed(vm, &format!("there's nothing at {}", path.display())));
    }
    shell(vm, "open", &path.display().to_string(), move || system::open_path(&path))
}

fn host_shell_reveal(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let path = path_at(vm, args, 0);
    if !path.exists() {
        return Err(failed(vm, &format!("there's nothing at {}", path.display())));
    }
    shell(vm, "show in its folder", &path.display().to_string(), move || system::reveal(&path))
}

fn host_shell_open_with(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let path = path_at(vm, args, 0);
    shell(vm, "choose what opens", &path.display().to_string(), move || system::open_with(&path))
}

fn host_shell_properties(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let path = path_at(vm, args, 0);
    shell(vm, "show the properties of", &path.display().to_string(), move || system::show_properties(&path))
}

fn host_shell_trash(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let paths: Vec<PathBuf> = texts_at(vm, args, 0).into_iter().map(PathBuf::from).collect();
    if let Some(missing) = paths.iter().find(|path| !path.exists()) {
        return Err(failed(vm, &format!("there's nothing at {}", missing.display())));
    }
    let named = paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ");
    shell(vm, "put in the Recycle Bin", &named, move || system::trash(&paths))
}

fn contents_value(vm: &mut Vm, contents: &system::ClipboardContents) -> Result<Value, Value> {
    let out = vm.new_object();
    let text = text_or_undefined(vm, contents.text.as_deref());
    set(vm, out, "text", text)?;
    let html = text_or_undefined(vm, contents.html.as_deref());
    set(vm, out, "html", html)?;
    if !contents.files.is_empty() {
        let files: Vec<Value> = contents.files.iter().map(|file| vm.new_string(file)).collect();
        let files = vm.new_array(files);
        set(vm, out, "files", files)?;
    }
    let made;
    let png = match (&contents.png, &contents.picture) {
        (Some(png), _) => Some(png),
        (None, Some((width, height, pixels))) => {
            made = crate::png::encode(*width as usize, *height as usize, pixels);
            Some(&made)
        }
        (None, None) => None,
    };
    if let Some(png) = png {
        let hex: String = png.iter().map(|byte| format!("{byte:02x}")).collect();
        let hex = vm.new_string(&hex);
        set(vm, out, "pngHex", hex)?;
    }
    Ok(out)
}

fn host_clipboard_read(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let contents = if for_real() {
        system::read_clipboard().map_err(|message| failed(vm, &message))?
    } else {
        with_kept(|kept| kept.clipboard.clone())
    };
    contents_value(vm, &contents)
}

/// `clipboardWrite(text, html, files, png as hex)`, each that's given.
fn host_clipboard_write(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let optional = |vm: &mut Vm, at: usize| {
        let value = args.get(vm, at);
        if value.is_undefined() || value.is_null() { None } else { vm.plain_text(value) }
    };
    let png = optional(vm, 3).map(|hex| (0..hex.len() / 2).filter_map(|at| u8::from_str_radix(&hex[at * 2..at * 2 + 2], 16).ok()).collect());
    let contents = system::ClipboardContents { text: optional(vm, 0), html: optional(vm, 1), files: texts_at(vm, args, 2), png, picture: None };
    if for_real() {
        system::write_clipboard(&contents).map_err(|message| failed(vm, &message))?;
    } else {
        let what: Vec<&str> = [("text", contents.text.is_some()), ("html", contents.html.is_some()), ("files", !contents.files.is_empty()), ("a picture", contents.png.is_some())]
            .into_iter().filter(|(_, there)| *there).map(|(name, _)| name).collect();
        bridge::say(format!("it put {} on the clipboard", what.join(" and ")));
        with_kept(|kept| kept.clipboard = contents);
    }
    Ok(Value::UNDEFINED)
}

fn host_clipboard_clear(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    if for_real() {
        system::clear_clipboard().map_err(|message| failed(vm, &message))?;
    } else {
        bridge::say("it cleared the clipboard".to_owned());
        with_kept(|kept| kept.clipboard = system::ClipboardContents::default());
    }
    Ok(Value::UNDEFINED)
}

fn host_clipboard_formats(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let formats = if for_real() {
        system::clipboard_formats().map_err(|message| failed(vm, &message))?
    } else {
        with_kept(|kept| {
            let contents = &kept.clipboard;
            [("text/plain", contents.text.is_some()), ("text/html", contents.html.is_some()), ("files", !contents.files.is_empty()), ("image/png", contents.png.is_some())]
                .into_iter().filter(|(_, there)| *there).map(|(name, _)| name.to_owned()).collect()
        })
    };
    let formats: Vec<Value> = formats.iter().map(|one| vm.new_string(one)).collect();
    Ok(vm.new_array(formats))
}

fn host_clipboard_on_change(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    if for_real() {
        system::on_clipboard_change(Rc::new(move || bridge::fire_desktop_event(id, None))).map_err(|message| failed(vm, &message))?;
    }
    Ok(Value::UNDEFINED)
}

/// The name a secret's kept under in the Credential Manager, which is the program's own.
fn secret_target(key: &str) -> String {
    format!("rsui/{}/{key}", bridge::program_name())
}

fn host_secret_get(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = text_at(vm, args, 0);
    let found = if for_real() { system::read_secret(&secret_target(&key)) } else { with_kept(|kept| kept.secrets.get(&key).cloned()) };
    Ok(text_or_undefined(vm, found.as_deref()))
}

fn host_secret_set(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (key, value) = (text_at(vm, args, 0), text_at(vm, args, 1));
    if for_real() {
        system::write_secret(&secret_target(&key), &bridge::program_name(), &value).map_err(|message| failed(vm, &message))?;
    } else {
        bridge::say(format!("it kept a secret as {key:?}, where only this program can read it"));
        with_kept(|kept| kept.secrets.insert(key, value));
    }
    Ok(Value::UNDEFINED)
}

fn host_secret_delete(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let key = text_at(vm, args, 0);
    let was = if for_real() { system::delete_secret(&secret_target(&key)) } else { with_kept(|kept| kept.secrets.remove(&key).is_some()) };
    Ok(Value::bool(was))
}

fn host_system_info(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let out = vm.new_object();
    let locale = system::locale();
    let locale = vm.new_string(&locale);
    set(vm, out, "locale", locale)?;
    set(vm, out, "idleTime", Value::number(f64::from(system::idle_time())))?;
    let accent = system::accent_color();
    let accent = text_or_undefined(vm, accent.as_deref());
    set(vm, out, "accentColor", accent)?;
    let zone = system::time_zone();
    let zone = text_or_undefined(vm, zone.as_deref());
    set(vm, out, "timeZone", zone)?;
    let access = system::accessibility();
    set(vm, out, "reducedMotion", Value::bool(access.reduced_motion))?;
    set(vm, out, "highContrast", Value::bool(access.high_contrast))?;
    set(vm, out, "textScale", Value::number(f64::from(access.text_scale)))?;
    Ok(out)
}

fn host_system_on_change(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    if real() {
        system::on_settings_change(Rc::new(move || bridge::fire_desktop_event(id, None))).map_err(|message| failed(vm, &message))?;
    }
    Ok(Value::UNDEFINED)
}

fn host_battery(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let battery = system::battery();
    let out = vm.new_object();
    set(vm, out, "onBattery", Value::bool(battery.on_battery))?;
    set(vm, out, "level", battery.level.map_or(Value::NULL, |level| Value::number(f64::from(level))))?;
    set(vm, out, "charging", Value::bool(battery.charging))?;
    Ok(out)
}

fn host_network_status(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let status = system::network_status();
    let out = vm.new_object();
    set(vm, out, "online", Value::bool(status.online))?;
    set(vm, out, "localOnly", Value::bool(status.local_only))?;
    set(vm, out, "metered", Value::bool(status.metered))?;
    Ok(out)
}

fn host_network_on_change(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    if real() {
        let tell = bridge::desktop_events_from_anywhere();
        system::on_network_change(Box::new(move |status| tell(id, Some(String::from(if status.online { "online" } else { "offline" })))))
            .map_err(|message| failed(vm, &message))?;
    }
    Ok(Value::UNDEFINED)
}

fn display_value(vm: &mut Vm, display: Option<system::Display>) -> Result<Value, Value> {
    let Some(display) = display else {
        return Ok(Value::NULL);
    };
    let one = vm.new_object();
    let (x, y, width, height) = display.bounds;
    let (work_x, work_y, work_width, work_height) = display.work_area;
    for (name, value) in [("x", x), ("y", y), ("width", width), ("height", height), ("workX", work_x), ("workY", work_y), ("workWidth", work_width), ("workHeight", work_height)] {
        set(vm, one, name, Value::int(value))?;
    }
    set(vm, one, "scale", Value::number(f64::from(display.scale)))?;
    set(vm, one, "primary", Value::bool(display.primary))?;
    Ok(one)
}

fn host_display_at(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (x, y) = (args.get(vm, 0), args.get(vm, 1));
    let found = system::display_at(x.as_number() as i32, y.as_number() as i32);
    display_value(vm, found)
}

fn host_display_of_window(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    let found = rsui::window_handle(rsui::WindowId(id)).and_then(system::display_of_window);
    // A window that's being checked, rather than shown, is on the main screen, as far as anyone can tell.
    let found = found.or_else(|| system::displays().into_iter().find(|one| one.primary));
    display_value(vm, found)
}

fn host_displays_on_change(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    if real() {
        system::on_displays_change(Rc::new(move || bridge::fire_desktop_event(id, None))).map_err(|message| failed(vm, &message))?;
    }
    Ok(Value::UNDEFINED)
}

fn host_recent_documents(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let paths = texts_at(vm, args, 0);
    if for_real() {
        system::add_recent_documents(&paths.iter().map(PathBuf::from).collect::<Vec<_>>());
    } else {
        bridge::say(format!("it said these were opened lately: {}", paths.join(", ")));
        with_kept(|kept| kept.recent.extend(paths));
    }
    Ok(Value::UNDEFINED)
}

fn host_clear_recent_documents(_: &mut Vm, _: Args) -> Result<Value, Value> {
    if for_real() {
        system::clear_recent_documents();
    } else {
        bridge::say("it cleared the files it opened lately".to_owned());
        with_kept(|kept| kept.recent.clear());
    }
    Ok(Value::UNDEFINED)
}

/// This program, and what it's started with before what it's to open, which is nothing once it's packed.
fn own_program() -> (String, String) {
    let program = std::env::current_exe().map(|path| path.display().to_string()).unwrap_or_default();
    if rsui_js::running_packed() {
        (program, String::new())
    } else {
        // Run as `rsui-tsx app.tsx`, with what's opened after the app.
        (program, format!("--once \"{}\"", bridge::app_file().display()))
    }
}

/// What starts this program again, with what's opened after it.
fn own_command() -> String {
    let (program, before) = own_program();
    if before.is_empty() { format!("\"{program}\"") } else { format!("\"{program}\" {before}") }
}

fn scheme_ok(scheme: &str) -> bool {
    !scheme.is_empty() && scheme.chars().next().is_some_and(|first| first.is_ascii_alphabetic())
        && scheme.chars().all(|one| one.is_ascii_alphanumeric() || matches!(one, '+' | '-' | '.'))
        && !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https" | "file" | "mailto" | "ftp" | "javascript")
}

fn host_protocol_register(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let scheme = text_at(vm, args, 0).to_ascii_lowercase();
    if !scheme_ok(&scheme) {
        return Err(failed(vm, &format!("{scheme:?} isn't a scheme a program can say it opens")));
    }
    let command = own_command();
    if for_real() {
        system::register_protocol(&scheme, &bridge::program_name(), &command).map_err(|message| failed(vm, &message))?;
    } else {
        bridge::say(format!("it said it opens {scheme}: links, with {command}"));
        with_kept(|kept| kept.protocols.insert(scheme, command));
    }
    Ok(Value::UNDEFINED)
}

fn host_protocol_unregister(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let scheme = text_at(vm, args, 0).to_ascii_lowercase();
    if !scheme_ok(&scheme) {
        return Err(failed(vm, &format!("{scheme:?} isn't a scheme a program can say it opens")));
    }
    if for_real() {
        // Only what this program said it opens is taken away.
        if system::protocol_command(&scheme).is_some_and(|said| said.starts_with(&own_command())) {
            system::unregister_protocol(&scheme).map_err(|message| failed(vm, &message))?;
        }
    } else {
        bridge::say(format!("it said it doesn't open {scheme}: links any more"));
        with_kept(|kept| kept.protocols.remove(&scheme));
    }
    Ok(Value::UNDEFINED)
}

fn host_protocol_is_registered(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let scheme = text_at(vm, args, 0).to_ascii_lowercase();
    let command = own_command();
    let said = if for_real() { system::protocol_command(&scheme) } else { with_kept(|kept| kept.protocols.get(&scheme).cloned()) };
    Ok(Value::bool(said.is_some_and(|said| said.starts_with(&command))))
}

/// `fileKindRegister(extension, description, icon)`.
fn host_file_kind_register(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let extension = text_at(vm, args, 0).trim_start_matches('.').to_ascii_lowercase();
    if extension.is_empty() || !extension.chars().all(|one| one.is_ascii_alphanumeric() || one == '-' || one == '_') {
        return Err(failed(vm, &format!("{extension:?} isn't an extension")));
    }
    let description = text_at(vm, args, 1);
    let icon = args.get(vm, 2);
    let icon = if icon.is_undefined() || icon.is_null() { None } else { vm.plain_text(icon) };
    let kind = format!("{}.{extension}", bridge::program_name());
    let command = own_command();
    if for_real() {
        system::register_file_kind(&extension, &kind, &description, &command, icon.as_deref()).map_err(|message| failed(vm, &message))?;
    } else {
        bridge::say(format!("it said it opens .{extension} files, as {description:?}"));
        with_kept(|kept| kept.file_kinds.insert(extension, kind));
    }
    Ok(Value::UNDEFINED)
}

fn host_file_kind_unregister(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let extension = text_at(vm, args, 0).trim_start_matches('.').to_ascii_lowercase();
    let kind = format!("{}.{extension}", bridge::program_name());
    if for_real() {
        system::unregister_file_kind(&extension, &kind).map_err(|message| failed(vm, &message))?;
    } else {
        bridge::say(format!("it said it doesn't open .{extension} files any more"));
        with_kept(|kept| kept.file_kinds.remove(&extension));
    }
    Ok(Value::UNDEFINED)
}

fn host_open_default_apps(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    shell(vm, "open", "the settings for which programs open what", system::open_default_apps_settings)
}

/// `popupMenu(menu, x, y)`, with what each item does as a number, as for the tray, at a point in logical pixels or where the
/// pointer is.
fn host_popup_menu(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let list = args.get(vm, 0);
    let mut names = Vec::new();
    let items = read_menu(vm, list, "", &mut names)?;
    let (x, y) = (args.get(vm, 1), args.get(vm, 2));
    let at = (x.is_number() && y.is_number()).then(|| (x.as_number() as i32, y.as_number() as i32));
    if real() {
        system::popup_menu(&items, at).map_err(|message| failed(vm, &message))?;
    } else {
        let listed: Vec<&str> = names.iter().map(|(name, _)| name.as_str()).collect();
        bridge::say(format!("it showed a menu of {}", listed.join(", ")));
        crate::native::set_pretend_menu(names);
    }
    Ok(Value::UNDEFINED)
}

fn host_drag_files(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let paths: Vec<PathBuf> = texts_at(vm, args, 0).into_iter().map(PathBuf::from).collect();
    if let Some(missing) = paths.iter().find(|path| !path.exists()) {
        return Err(failed(vm, &format!("there's nothing at {}", missing.display())));
    }
    let done = if real() {
        system::drag_files(&paths).map_err(|message| failed(vm, &message))?
    } else {
        bridge::say(format!("it started dragging {} file(s), which nothing dropped", paths.len()));
        "none"
    };
    Ok(vm.new_string(done))
}

fn host_window_state(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    let Some(state) = rsui::window_state(rsui::WindowId(id)) else {
        return Ok(Value::NULL);
    };
    let out = vm.new_object();
    let title = vm.new_string(&state.title);
    set(vm, out, "title", title)?;
    for (name, value) in [("x", state.x), ("y", state.y)] {
        set(vm, out, name, Value::int(value))?;
    }
    for (name, value) in [("width", state.width), ("height", state.height)] {
        set(vm, out, name, Value::number(f64::from(value)))?;
    }
    for (name, value) in [("visible", state.visible), ("minimized", state.minimized), ("maximized", state.maximized), ("fullscreen", state.fullscreen), ("focused", state.focused)] {
        set(vm, out, name, Value::bool(value))?;
    }
    Ok(out)
}

fn host_window_on_state(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    if id != 0 {
        rsui::on_window_state_change(move |window| bridge::fire_desktop_event(id, Some(window.0.to_string())));
    }
    Ok(Value::UNDEFINED)
}

fn host_app_paths(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let out = vm.new_object();
    let env = |name: &str| std::env::var(name).ok();
    let program = std::env::current_exe().ok().map(|path| path.display().to_string());
    let app = bridge::app_file().display().to_string();
    let name = bridge::program_name();
    let app_data = env("APPDATA").map(|folder| PathBuf::from(folder).join(&name).display().to_string());
    let temp = std::env::temp_dir().display().to_string();
    let mut paths: Vec<(&str, Option<String>)> = vec![
        ("home", env("USERPROFILE")),
        ("appData", env("APPDATA")),
        ("localAppData", env("LOCALAPPDATA")),
        ("userData", app_data),
        ("temp", Some(temp)),
        ("exe", program),
        ("app", Some(app)),
    ];
    for folder in ["desktop", "documents", "downloads", "music", "pictures", "videos"] {
        // A sandboxed program's own folders are in its sandbox.
        let found = match sandbox() {
            Some(sandbox) => Some(sandbox.join(folder).display().to_string()),
            None => crate::native::user_folder(folder).map(|path| path.display().to_string()),
        };
        paths.push((folder, found));
    }
    for (name, value) in paths {
        let value = text_or_undefined(vm, value.as_deref());
        set(vm, out, name, value)?;
    }
    Ok(out)
}

/// What this program was started with, after the program, for what it's to open, as a link or a file.
fn host_started_with(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let given: Vec<String> = if rsui_js::running_packed() {
        std::env::args().skip(1).filter(|one| !one.starts_with("--")).collect()
    } else {
        // `rsui-tsx --once app.tsx what...`: what's after the app.
        let app = bridge::app_file();
        let mut after = false;
        std::env::args().skip(1).filter(|one| {
            if after {
                return true;
            }
            if std::path::Path::new(one).canonicalize().ok() == app.canonicalize().ok() {
                after = true;
            }
            false
        }).collect()
    };
    let given: Vec<Value> = given.iter().map(|one| vm.new_string(one)).collect();
    Ok(vm.new_array(given))
}

fn host_tray_bounds(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let Some((x, y, width, height)) = (if real() { system::tray_bounds() } else { None }) else {
        return Ok(Value::NULL);
    };
    let out = vm.new_object();
    for (name, value) in [("x", x), ("y", y), ("width", width), ("height", height)] {
        set(vm, out, name, Value::int(value))?;
    }
    Ok(out)
}

fn host_notification_dismiss(_: &mut Vm, _: Args) -> Result<Value, Value> {
    if real() {
        system::dismiss_notification();
    } else {
        bridge::say("it took the notification away".to_owned());
        crate::native::forget_pretend_notification();
    }
    Ok(Value::UNDEFINED)
}

/// `windowCapture(id, action)`: a picture of what's in the window, told to `action` as `png ` and a PNG file's bytes as hex, or as
/// `error ` and why not. It's taken once the window isn't being drawn, which it is while this runs. A window that's being
/// checked rather than shown has nothing to take a picture of, as `shot` is for that.
fn host_window_capture(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (id, action) = (number_at(vm, args, 0), number_at(vm, args, 1));
    if !real() {
        return Err(failed(vm, "there's no window shown to take a picture of here, which the `shot` step is for"));
    }
    rsui::on_window_captured(|action, picture| {
        let said = match picture {
            Ok((width, height, pixels)) => {
                let png = crate::png::encode_drawn(width as usize, height as usize, &pixels);
                let hex: String = png.iter().map(|byte| format!("{byte:02x}")).collect();
                format!("png {hex}")
            }
            Err(why) => format!("error {why}"),
        };
        bridge::fire_desktop_event(action, Some(said));
    });
    rsui::change_window(rsui::WindowId(id), rsui::WindowChange::Capture(action));
    Ok(Value::UNDEFINED)
}

/// `updateConfirmHealthy()`: says this version of a packed program works, so it's kept rather than the one before it.
fn host_update_confirm_healthy(_: &mut Vm, _: Args) -> Result<Value, Value> {
    if !real() {
        bridge::say("it said this version of the program works".to_owned());
        return Ok(Value::UNDEFINED);
    }
    if let Ok(program) = std::env::current_exe() {
        let _ = std::fs::remove_file(crate::unconfirmed_marker(&program));
    }
    Ok(Value::UNDEFINED)
}

/// `updateUnconfirmed()`: whether this program's a new version that hasn't said it works yet.
fn host_update_unconfirmed(_: &mut Vm, _: Args) -> Result<Value, Value> {
    let waiting = std::env::current_exe().is_ok_and(|program| crate::unconfirmed_marker(&program).exists());
    Ok(Value::bool(waiting))
}

/// Whether what's asked of the desktop really happens, rather than being said, as it doesn't when an app's being checked.
fn host_desktop_real(_: &mut Vm, _: Args) -> Result<Value, Value> {
    Ok(Value::bool(for_real()))
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 41] = [
    ("desktopReal", host_desktop_real),
    ("updateConfirmHealthy", host_update_confirm_healthy),
    ("updateUnconfirmed", host_update_unconfirmed),
    ("windowCapture", host_window_capture),
    ("trayBounds", host_tray_bounds),
    ("notificationDismiss", host_notification_dismiss),
    ("shellOpenPath", host_shell_open_path),
    ("shellReveal", host_shell_reveal),
    ("shellOpenWith", host_shell_open_with),
    ("shellProperties", host_shell_properties),
    ("shellTrash", host_shell_trash),
    ("clipboardRead", host_clipboard_read),
    ("clipboardWrite", host_clipboard_write),
    ("clipboardClear", host_clipboard_clear),
    ("clipboardFormats", host_clipboard_formats),
    ("clipboardOnChange", host_clipboard_on_change),
    ("secretGet", host_secret_get),
    ("secretSet", host_secret_set),
    ("secretDelete", host_secret_delete),
    ("systemInfo", host_system_info),
    ("systemOnChange", host_system_on_change),
    ("battery", host_battery),
    ("networkStatus", host_network_status),
    ("networkOnChange", host_network_on_change),
    ("displayAt", host_display_at),
    ("displayOfWindow", host_display_of_window),
    ("displaysOnChange", host_displays_on_change),
    ("recentDocuments", host_recent_documents),
    ("clearRecentDocuments", host_clear_recent_documents),
    ("protocolRegister", host_protocol_register),
    ("protocolUnregister", host_protocol_unregister),
    ("protocolIsRegistered", host_protocol_is_registered),
    ("fileKindRegister", host_file_kind_register),
    ("fileKindUnregister", host_file_kind_unregister),
    ("openDefaultApps", host_open_default_apps),
    ("popupMenu", host_popup_menu),
    ("dragFiles", host_drag_files),
    ("windowState", host_window_state),
    ("windowOnState", host_window_on_state),
    ("appPaths", host_app_paths),
    ("startedWith", host_started_with),
];

// Pictures of files and of the screen, the jump list, the buttons under a window's picture in the taskbar, choosing a color
// or a font, sound devices and media keys.

/// Bytes as base64, as a `data:` URL has them.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for three in bytes.chunks(3) {
        let joined = three.iter().enumerate().fold(0u32, |all, (at, byte)| all | u32::from(*byte) << (16 - 8 * at));
        for at in 0..4 {
            if at <= three.len() {
                out.push(char::from(LETTERS[(joined >> (18 - 6 * at) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A picture as JavaScript gets it: its width and its height, then a PNG file's bytes as base64, with spaces between.
fn picture_text((width, height, rgba): system::Picture) -> String {
    let png = crate::png::encode(width as usize, height as usize, &rgba);
    format!("{width} {height} {}", base64(&png))
}

/// `filePicture(path, size, icon)`: a promise of what the shell shows for a file, its thumbnail or with `icon` its icon, as
/// `picture_text` has it, or `null` for a file with no thumbnail. It's only read, so it's real even while an app's being
/// checked, as `displays` is.
fn host_file_picture(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let path = path_at(vm, args, 0);
    let size = number_at(vm, args, 1).max(1);
    let icon = args.get(vm, 2) == Value::TRUE;
    if !path.exists() {
        return Err(failed(vm, &format!("there's nothing at {}", path.display())));
    }
    Ok(vm.later(move || system::file_picture(&path, size, icon).map(|picture| picture.map(picture_text))))
}

/// `captureScreen(x, y, width, height)`: a promise of what's on the screen there, in physical pixels, as `picture_text` has it.
/// While an app's being checked it's said instead, and the picture's blank, so nothing ever reads what's on someone's screen.
fn host_capture_screen(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let mut numbers = [0.0; 4];
    for (at, number) in numbers.iter_mut().enumerate() {
        let value = args.get(vm, at);
        *number = if value.is_number() { value.as_number() } else { 0.0 };
    }
    let (x, y) = (numbers[0] as i32, numbers[1] as i32);
    let (width, height) = (numbers[2].clamp(0.0, 16_384.0) as u32, numbers[3].clamp(0.0, 16_384.0) as u32);
    if width == 0 || height == 0 {
        return Err(failed(vm, "a picture of the screen needs a width and a height"));
    }
    if !real() {
        bridge::say(format!("it took a picture of the screen at {x}, {y}, {width} by {height}, which is blank here"));
        let blank = [0, 0, 0, 255].repeat(width as usize * height as usize);
        return Ok(vm.later(move || Ok(Some(picture_text((width, height, blank))))));
    }
    Ok(vm.later(move || system::capture_screen(x, y, width, height).map(|picture| Some(picture_text(picture)))))
}

fn jump_item(one: &serde_json::Value) -> system::JumpItem {
    let text = |name: &str| one.get(name).and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
    system::JumpItem {
        title: text("title"),
        args: text("args"),
        icon: one.get("icon").and_then(serde_json::Value::as_str).map(str::to_owned),
        description: text("description"),
    }
}

/// `jumpListSet(json)`, with `{ tasks: [{ title, args, icon, description }], categories: [{ name, items }] }` as JSON, the icons
/// as files.
fn host_jump_list_set(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let text = text_at(vm, args, 0);
    let list: serde_json::Value = serde_json::from_str(&text).map_err(|error| failed(vm, &format!("the jump list isn't right: {error}")))?;
    let items = |value: Option<&serde_json::Value>| value.and_then(serde_json::Value::as_array).map(|items| items.iter().map(jump_item).collect::<Vec<_>>()).unwrap_or_default();
    let tasks = items(list.get("tasks"));
    let categories: Vec<system::JumpCategory> = list
        .get("categories")
        .and_then(serde_json::Value::as_array)
        .map(|all| {
            all.iter()
                .map(|one| system::JumpCategory {
                    name: one.get("name").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned(),
                    items: items(one.get("items")),
                })
                .collect()
        })
        .unwrap_or_default();
    if for_real() {
        // Each starts this program as it was started, with what's after it.
        let (program, before) = own_program();
        let started = |mut item: system::JumpItem| {
            item.args = [before.as_str(), item.args.as_str()].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" ");
            item
        };
        let tasks: Vec<system::JumpItem> = tasks.into_iter().map(started).collect();
        let categories: Vec<system::JumpCategory> = categories
            .into_iter()
            .map(|category| system::JumpCategory { name: category.name, items: category.items.into_iter().map(started).collect() })
            .collect();
        system::set_jump_list(&program, &tasks, &categories).map_err(|message| failed(vm, &message))?;
    } else {
        let titles = |items: &[system::JumpItem]| items.iter().map(|item| item.title.as_str()).collect::<Vec<_>>().join(", ");
        let mut said: Vec<String> = Vec::new();
        if !tasks.is_empty() {
            said.push(format!("Tasks ({})", titles(&tasks)));
        }
        said.extend(categories.iter().map(|category| format!("{} ({})", category.name, titles(&category.items))));
        bridge::say(format!("it set its jump list to {}", if said.is_empty() { "nothing".to_owned() } else { said.join("; ") }));
        with_kept(|kept| kept.jump_list = Some(list));
    }
    Ok(Value::UNDEFINED)
}

fn host_jump_list_clear(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    if for_real() {
        system::clear_jump_list().map_err(|message| failed(vm, &message))?;
    } else {
        bridge::say("it cleared its jump list".to_owned());
        with_kept(|kept| kept.jump_list = None);
    }
    Ok(Value::UNDEFINED)
}

/// The buttons under each window's picture in the taskbar, for the `thumb` step: the window, each button's name, what it does,
/// and whether it's disabled.
type ThumbnailButtons = Vec<(u32, String, u32, bool)>;

thread_local! {
    static THUMBNAIL_BUTTONS: RefCell<ThumbnailButtons> = const { RefCell::new(Vec::new()) };
    /// What's told about media keys.
    static MEDIA_KEYS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// `thumbnailButtons(window, [{ name, action, icon, tooltip, disabled }])`, with what each does as a number and its icon's
/// bytes as hex.
fn host_thumbnail_buttons(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let window = number_at(vm, args, 0);
    let list = args.get(vm, 1);
    let items = vm.array_elements(list).map(<[Value]>::to_vec).unwrap_or_default();
    if items.len() > 7 {
        return Err(failed(vm, "the taskbar has room for 7 buttons under a window's picture, at most"));
    }
    let mut buttons = Vec::with_capacity(items.len());
    let mut named = Vec::with_capacity(items.len());
    for item in items {
        let get = |vm: &mut Vm, name: &str| {
            let name = vm.atom(name);
            vm.get_property(item, name)
        };
        let action = get(vm, "action")?;
        let action = if action.is_number() { action.as_number() as u32 } else { 0 };
        let name = get(vm, "name")?;
        let name = vm.plain_text(name).unwrap_or_default();
        let icon = get(vm, "icon")?;
        let icon = vm.plain_text(icon).unwrap_or_default();
        let icon: Vec<u8> = (0..icon.len() / 2).filter_map(|at| u8::from_str_radix(&icon[at * 2..at * 2 + 2], 16).ok()).collect();
        let tooltip = get(vm, "tooltip")?;
        let tooltip = vm.plain_text(tooltip).unwrap_or_default();
        let disabled = get(vm, "disabled")? == Value::TRUE;
        named.push((window, name, action, disabled));
        buttons.push(rsui::ThumbnailButton { id: action, icon, tooltip, disabled });
    }
    if real() {
        // Only a button that's still there is told, as what a button did goes with the code that put it there.
        rsui::on_thumbnail_button(|window, action| {
            let there = THUMBNAIL_BUTTONS.with(|kept| kept.borrow().iter().any(|(on, _, one, _)| *on == window.0 && *one == action && action != 0));
            if there {
                bridge::fire_desktop_event(action, None);
            }
        });
    }
    THUMBNAIL_BUTTONS.with(|kept| {
        let mut kept = kept.borrow_mut();
        kept.retain(|(on, ..)| *on != window);
        kept.extend(named);
    });
    rsui::change_window(rsui::WindowId(window), rsui::WindowChange::ThumbnailButtons(buttons));
    Ok(Value::UNDEFINED)
}

/// Clicks on the button under a window's picture in the taskbar that goes by this name.
pub(crate) fn click_thumbnail_button(name: &str) -> Result<(), String> {
    let found = THUMBNAIL_BUTTONS.with(|kept| kept.borrow().iter().find(|(_, one, ..)| one == name).map(|(_, _, action, disabled)| (*action, *disabled)));
    match found {
        Some((_, true)) => Err(format!("the button {name:?} is disabled, so a person couldn't click it either")),
        Some((0, _)) => Err(format!("nothing's listening to the button {name:?}")),
        Some((action, false)) => {
            bridge::fire_desktop_event(action, None);
            Ok(())
        }
        None => {
            let there: Vec<String> = THUMBNAIL_BUTTONS.with(|kept| kept.borrow().iter().map(|(_, one, ..)| one.clone()).collect());
            Err(format!("there's no button {name:?} under a window's picture in the taskbar. There's {there:?}"))
        }
    }
}

/// A color as `#rrggbb`, from that or `#rgb`, if it's one.
fn color_from(text: &str) -> Option<(u8, u8, u8)> {
    let hex = text.trim().strip_prefix('#')?;
    let digit = |at: usize, wide: usize| u8::from_str_radix(hex.get(at..at + wide)?, 16).ok();
    match hex.len() {
        6 => Some((digit(0, 2)?, digit(2, 2)?, digit(4, 2)?)),
        3 => Some((digit(0, 1)? * 17, digit(1, 1)? * 17, digit(2, 1)? * 17)),
        _ => None,
    }
}

fn color_text((red, green, blue): (u8, u8, u8)) -> String {
    format!("#{red:02x}{green:02x}{blue:02x}")
}

/// What the next dialog gets where they don't come up, as the `dialog` step says, which is being cancelled if nothing's said.
fn pretend_answer(what: &str) -> Option<Option<String>> {
    let answer = bridge::take_dialog_answer().or_else(|| (!real()).then_some(None))?;
    bridge::say(match &answer {
        Some(answer) => format!("{what} was asked for, and got {answer}"),
        None => format!("{what} was asked for, and was cancelled"),
    });
    Some(answer)
}

/// `pickColor(initial)`: a promise of the color the user chose, as `#rrggbb`, or `null`.
fn host_pick_color(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let initial = color_from(&text_at(vm, args, 0));
    if let Some(answer) = pretend_answer("a color") {
        let answer = match answer.as_deref().map(|text| color_from(text).ok_or(text)) {
            None => None,
            Some(Ok(color)) => Some(color_text(color)),
            Some(Err(text)) => return Err(failed(vm, &format!("{text:?} isn't a color, as #rrggbb"))),
        };
        return Ok(vm.later(move || Ok(answer)));
    }
    Ok(vm.later(move || Ok(system::pick_color(initial).map(color_text))))
}

/// `pickFont(initial)`: a promise of the font the user chose, as JSON of `{ family, size, weight, italic, color }`, or `null`.
/// What it starts from is JSON the same way.
fn host_pick_font(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let given = text_at(vm, args, 0);
    if let Some(answer) = pretend_answer("a font") {
        if let Some(text) = &answer {
            if !serde_json::from_str::<serde_json::Value>(text).is_ok_and(|font| font.is_object()) {
                return Err(failed(vm, &format!("{text:?} isn't a font, as JSON like {{\"family\":\"Arial\",\"size\":12}}")));
            }
        }
        return Ok(vm.later(move || Ok(answer)));
    }
    let initial = serde_json::from_str::<serde_json::Value>(&given).ok().filter(serde_json::Value::is_object).map(|font| system::FontChoice {
        family: font.get("family").and_then(serde_json::Value::as_str).unwrap_or("Segoe UI").to_owned(),
        size: font.get("size").and_then(serde_json::Value::as_f64).unwrap_or(12.0) as f32,
        weight: font.get("weight").and_then(serde_json::Value::as_u64).unwrap_or(400) as u32,
        italic: font.get("italic").and_then(serde_json::Value::as_bool).unwrap_or(false),
        color: font.get("color").and_then(serde_json::Value::as_str).and_then(color_from).unwrap_or((0, 0, 0)),
    });
    Ok(vm.later(move || {
        Ok(system::pick_font(initial).map(|font| {
            serde_json::json!({
                "family": font.family,
                "size": font.size,
                "weight": font.weight,
                "italic": font.italic,
                "color": color_text(font.color),
            })
            .to_string()
        }))
    }))
}

/// `audioDevices()`: the devices that play and record sound, as `{ id, name, kind, isDefault }`. It's only read, so it's real
/// even while an app's being checked. A machine with no sound has none.
fn host_audio_devices(vm: &mut Vm, _: Args) -> Result<Value, Value> {
    let devices = system::audio_devices().unwrap_or_default();
    let mut list = Vec::with_capacity(devices.len());
    for device in devices {
        let one = vm.new_object();
        let id = vm.new_string(&device.id);
        set(vm, one, "id", id)?;
        let name = vm.new_string(&device.name);
        set(vm, one, "name", name)?;
        let kind = vm.new_string(if device.output { "output" } else { "input" });
        set(vm, one, "kind", kind)?;
        set(vm, one, "isDefault", Value::bool(device.default))?;
        list.push(one);
    }
    Ok(vm.new_array(list))
}

fn host_audio_on_change(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    if real() {
        system::on_devices_change(Rc::new(move || bridge::fire_desktop_event(id, None))).map_err(|message| failed(vm, &message))?;
    }
    Ok(Value::UNDEFINED)
}

/// `mediaOnKey(id)`: what's told the media keys pressed while one of the program's windows has the keyboard.
fn host_media_on_key(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    MEDIA_KEYS.with(|kept| kept.set(id));
    if real() {
        rsui::on_media_key(|key| {
            let id = MEDIA_KEYS.with(std::cell::Cell::get);
            if id != 0 {
                bridge::fire_desktop_event(id, Some(key.to_owned()));
            }
        });
    }
    Ok(Value::UNDEFINED)
}

/// Takes away the buttons under the windows' pictures in the taskbar, and stops telling the media keys, for a new version of
/// the app's code to ask for them again. What they did was the old code's.
pub(crate) fn let_go_of_everything() {
    let mut windows: Vec<u32> = THUMBNAIL_BUTTONS.with(|kept| kept.borrow_mut().drain(..).map(|(window, ..)| window).collect());
    windows.sort_unstable();
    windows.dedup();
    for window in windows {
        rsui::change_window(rsui::WindowId(window), rsui::WindowChange::ThumbnailButtons(Vec::new()));
    }
    MEDIA_KEYS.with(|kept| kept.set(0));
}

/// The media keys, as they're told.
const MEDIA_KEY_NAMES: [&str; 6] = ["play", "pause", "playPause", "next", "previous", "stop"];

/// Presses a media key, as if the window had the keyboard.
pub(crate) fn press_media_key(key: &str) -> Result<(), String> {
    let name = MEDIA_KEY_NAMES
        .into_iter()
        .find(|name| name.eq_ignore_ascii_case(key))
        .ok_or_else(|| format!("{key:?} isn't a media key. They're {MEDIA_KEY_NAMES:?}"))?;
    let id = MEDIA_KEYS.with(std::cell::Cell::get);
    if id == 0 {
        return Err("nothing's listening for the media keys".to_owned());
    }
    bridge::fire_desktop_event(id, Some(name.to_owned()));
    Ok(())
}

/// `folderList(path)`: a promise of what's in a folder, a line for each thing: its name, then flags (1 a folder, 2 hidden,
/// 4 the system's, 8 read-only, 16 a link), its size, and when it was changed and made, in milliseconds, with tabs between.
/// No name can have a tab or a new line in it on Windows. It's only read, so it's real while an app's being checked too.
fn host_folder_list(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    use std::fmt::Write as _;
    let path = path_at(vm, args, 0);
    Ok(vm.later(move || {
        let entries = system::list_folder(&path)?;
        let mut text = String::with_capacity(entries.len() * 48);
        for one in entries {
            let flags = u32::from(one.folder)
                | u32::from(one.hidden) << 1
                | u32::from(one.system) << 2
                | u32::from(one.readonly) << 3
                | u32::from(one.link) << 4;
            let _ = writeln!(text, "{}\t{flags}\t{}\t{}\t{}", one.name, one.size, one.modified as i64, one.created as i64);
        }
        Ok(Some(text))
    }))
}

/// `drives()`: a promise of the drives there are, as JSON of `[{ path, label, kind, total, free }]`.
fn host_drives(vm: &mut Vm, _args: Args) -> Result<Value, Value> {
    Ok(vm.later(|| {
        let drives: Vec<serde_json::Value> = system::drives()
            .into_iter()
            .map(|drive| {
                serde_json::json!({ "path": drive.path, "label": drive.label, "kind": drive.kind, "total": drive.total, "free": drive.free })
            })
            .collect();
        Ok(Some(serde_json::Value::Array(drives).to_string()))
    }))
}

pub(crate) const MORE_FUNCTIONS: [(&str, HostFunction); 12] = [
    ("filePicture", host_file_picture),
    ("folderList", host_folder_list),
    ("drives", host_drives),
    ("captureScreen", host_capture_screen),
    ("jumpListSet", host_jump_list_set),
    ("jumpListClear", host_jump_list_clear),
    ("thumbnailButtons", host_thumbnail_buttons),
    ("pickColor", host_pick_color),
    ("pickFont", host_pick_font),
    ("audioDevices", host_audio_devices),
    ("audioOnChange", host_audio_on_change),
    ("mediaOnKey", host_media_on_key),
];

#[cfg(test)]
mod tests {
    use super::{base64, color_from};

    #[test]
    fn base64_is_as_the_web_has_it() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xFF, 0xEF]), "/+8=");
    }

    #[test]
    fn colors_are_read_as_css_writes_them() {
        assert_eq!(color_from("#ff8000"), Some((255, 128, 0)));
        assert_eq!(color_from("#F80"), Some((255, 136, 0)));
        assert_eq!(color_from("ff8000"), None);
        assert_eq!(color_from("#ff80"), None);
        assert_eq!(color_from("#gg0000"), None);
    }
}
