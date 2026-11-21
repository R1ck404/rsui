//! Making one program out of an app, to hand to someone with nothing else to install.

#[cfg(windows)]
use std::ffi::c_void;
use std::fs::OpenOptions;
#[cfg(windows)]
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn BeginUpdateResourceW(file: *const u16, delete_existing: i32) -> *mut c_void;
    fn UpdateResourceW(
        update: *mut c_void,
        kind: *const u16,
        name: *const u16,
        language: u16,
        data: *const c_void,
        size: u32,
    ) -> i32;
    fn EndUpdateResourceW(update: *mut c_void, discard: i32) -> i32;
    fn FindResourceW(module: *mut c_void, name: *const u16, kind: *const u16) -> *mut c_void;
    fn LoadResource(module: *mut c_void, found: *mut c_void) -> *mut c_void;
    fn LockResource(loaded: *mut c_void) -> *const c_void;
    fn SizeofResource(module: *mut c_void, found: *mut c_void) -> u32;
}

/// The kind of resource that's bytes of the program's own, and the name the app's config has among them.
#[cfg(windows)]
const RAW_DATA: usize = 10;
#[cfg(windows)]
const CONFIG_NAME: &str = "RSUICONFIG";

/// Writes the app's config (`config.rs`), as JSON, into the program, where [`packed_config`] finds it.
#[cfg(windows)]
pub(crate) fn set_config(program: &Path, config: &serde_json::Value) -> Result<(), String> {
    let text = config.to_string();
    let (path, name) = (wide(program), wide(CONFIG_NAME));
    // SAFETY: both strings end in a terminator; the data lasts until the update has ended, which is what it's copied by.
    unsafe {
        let update = BeginUpdateResourceW(path.as_ptr(), 0);
        if update.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let worked = UpdateResourceW(update, RAW_DATA as *const u16, name.as_ptr(), 0, text.as_ptr().cast(), text.len() as u32) != 0;
        if EndUpdateResourceW(update, i32::from(!worked)) == 0 || !worked {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}

/// Elsewhere a packed program carries no config yet: what it says goes into what the system reads, as an APK's manifest.
#[cfg(not(windows))]
pub(crate) fn set_config(_program: &Path, _config: &serde_json::Value) -> Result<(), String> {
    Ok(())
}

/// The config written into this program when it was packed, if it was. It's in the program's image, which is mapped
/// already, so this reads no file.
#[cfg(windows)]
pub(crate) fn packed_config() -> Option<serde_json::Value> {
    let name = wide(CONFIG_NAME);
    // SAFETY: a null module is this program; the resource stays mapped for as long as it runs, and is `size` bytes.
    unsafe {
        let found = FindResourceW(std::ptr::null_mut(), name.as_ptr(), RAW_DATA as *const u16);
        if found.is_null() {
            return None;
        }
        let size = SizeofResource(std::ptr::null_mut(), found) as usize;
        let bytes = LockResource(LoadResource(std::ptr::null_mut(), found));
        if bytes.is_null() {
            return None;
        }
        serde_json::from_slice(std::slice::from_raw_parts(bytes.cast::<u8>(), size)).ok()
    }
}

#[cfg(not(windows))]
pub(crate) fn packed_config() -> Option<serde_json::Value> {
    None
}

/// A `.ico` holding one PNG picture, as Windows has taken since Vista, for an app whose icon is a `.png`.
pub(crate) fn ico_from_png(png: &[u8]) -> Result<Vec<u8>, String> {
    if png.get(..8) != Some(b"\x89PNG\r\n\x1a\n".as_slice()) || png.get(12..16) != Some(b"IHDR".as_slice()) {
        return Err("that isn't a .png file".to_owned());
    }
    let side = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().expect("four bytes"));
    let (width, height) = (side(16), side(20));
    if width > 256 || height > 256 {
        return Err(format!(
            "the icon is {width} by {height}, and a Windows icon is at most 256 by 256: give a .ico, or a 256 by 256 .png"
        ));
    }
    // 256 is written as 0.
    let byte = |side: u32| (side % 256) as u8;
    let mut ico = vec![0, 0, 1, 0, 1, 0];
    ico.extend_from_slice(&[byte(width), byte(height), 0, 0, 1, 0, 32, 0]);
    ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes());
    ico.extend_from_slice(png);
    Ok(ico)
}

#[cfg(windows)]
#[link(name = "user32.dll", kind = "raw-dylib", modifiers = "+verbatim")]
unsafe extern "system" {
    fn MessageBoxW(window: *mut c_void, text: *const u16, title: *const u16, kind: u32) -> i32;
}

#[cfg(windows)]
fn wide(text: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain([0]).collect()
}

/// Shows that something went wrong. Elsewhere a program started from the desktop has its errors in the system's log, which
/// is where this goes, until the desktop's own message boxes are written.
#[cfg(not(windows))]
pub(crate) fn show_error(message: &str) {
    eprintln!("This app couldn't start: {message}");
}

/// Shows that something went wrong, for a program with no console to say so on.
#[cfg(windows)]
pub(crate) fn show_error(message: &str) {
    const ERROR_ICON: u32 = 0x10;
    let (text, title) = (wide(message), wide("This app couldn't start"));
    // SAFETY: both strings end in a terminator and last through the call, and no window is allowed.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            ERROR_ICON,
        );
    }
}

/// Everything a running app has loaded, with the whole of any package it's in. A package can load the
/// rest of itself later, while what's an app's own has been imported by the time it's started, or
/// it's something like a data file it reads for itself. It's kept in order, for the same result each time.
/// The kinds of file an app has for what it shows and plays, rather than as code, which it reads for itself.
const ASSETS: [&str; 20] = [
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg", "avif", "woff", "woff2", "ttf",
    "otf", "wav", "mp3", "ogg", "m4a", "flac", "css", "txt",
];

/// The kinds of file that are code, or data that code reads as it loads.
const CODE: [&str; 9] = ["js", "mjs", "cjs", "json", "ts", "mts", "cts", "tsx", "jsx"];

fn is_one_of(path: &Path, kinds: &[&str]) -> bool {
    path.extension().is_some_and(|extension| {
        kinds
            .iter()
            .any(|kind| extension.eq_ignore_ascii_case(kind))
    })
}

fn files_to_pack(
    app: &Path,
    loaded: &[PathBuf],
    also: &[PathBuf],
    graph: &[PathBuf],
) -> std::io::Result<Vec<PathBuf>> {
    fn add_folder(folder: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(folder)? {
            let entry = entry?;
            let path = entry.path();
            // A link in a package leads to another package, as pnpm makes them, which is packed where it really is if
            // it's used, and is found through the link from the list that `links_to` makes.
            if entry.file_type()?.is_symlink() {
                continue;
            }
            if path.is_dir() {
                // What a package needs for itself is in its own folder, and a nested
                // `node_modules` only has what it needs too.
                add_folder(&path, files)?;
            } else if is_one_of(&path, &CODE) || is_one_of(&path, &ASSETS) {
                files.push(path);
            }
        }
        Ok(())
    }
    /// The pictures, fonts and sounds in the app's own folders, which it shows and plays by their paths.
    fn add_assets(folder: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(folder)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                // Packages are only packed if they're used, and what starts with a dot is the tools' own, and so is what
                // Cargo builds, which it marks with a tag.
                if name != "node_modules"
                    && !name.starts_with('.')
                    && !path.join("CACHEDIR.TAG").is_file()
                {
                    add_assets(&path, files)?;
                }
            } else if is_one_of(&path, &ASSETS) {
                files.push(path);
            }
        }
        Ok(())
    }
    // What the app needs that isn't code, as what its CSS tools made, goes in as it is.
    let mut files = also.to_vec();
    let mut packages = Vec::new();
    for path in loaded.iter().chain(graph) {
        match package_folder(path) {
            Some(package) => {
                if !packages.contains(&package) {
                    packages.push(package);
                }
            }
            None => files.push(path.clone()),
        }
    }
    for package in packages {
        add_folder(&package, &mut files)?;
    }
    if let Some(folder) = app.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        add_assets(&plain_path(&std::fs::canonicalize(folder)?), &mut files)?;
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// The links in the `node_modules` folders that a packed file would look for packages in, as Node does, from its own
/// folder up, that lead to a folder with packed files in it, with where each leads. pnpm's `node_modules` is all links
/// into `node_modules/.pnpm`, and a package there finds what it needs in links next to it. A workspace's own packages
/// are links too, from npm, yarn and pnpm alike. `files` is sorted, and the paths it has are where files really are.
fn links_to(files: &[PathBuf]) -> Vec<(PathBuf, PathBuf)> {
    fn leads_to_packed(files: &[PathBuf], target: &Path) -> bool {
        let first = files.partition_point(|file| file.as_path() < target);
        files
            .get(first)
            .is_some_and(|file| file.starts_with(target))
    }
    fn look_in(
        folder: &Path,
        files: &[PathBuf],
        links: &mut Vec<(PathBuf, PathBuf)>,
        in_scope: bool,
    ) {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_symlink() {
                if let Ok(target) = std::fs::canonicalize(&path) {
                    let target = plain_path(&target);
                    if target.is_dir() && leads_to_packed(files, &target) {
                        links.push((path, target));
                    }
                }
            } else if kind.is_dir()
                && !in_scope
                && entry.file_name().to_string_lossy().starts_with('@')
            {
                look_in(&path, files, links, true);
            }
        }
    }
    let mut looked_at = std::collections::HashSet::new();
    let mut links = Vec::new();
    for file in files {
        for folder in file.ancestors().skip(1) {
            if !looked_at.insert(folder.to_owned()) {
                break;
            }
            // A `node_modules` isn't looked for inside one, as Node doesn't.
            if folder
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("node_modules"))
            {
                continue;
            }
            look_in(&folder.join("node_modules"), files, &mut links, false);
        }
    }
    links.sort();
    links.dedup();
    links
}

/// The files that `file` loads with `import("./...")` once they're wanted, which running it doesn't load: those that are
/// there, and said as a path from it.
pub(crate) fn dynamic_imports(file: &Path) -> Vec<PathBuf> {
    let Ok(source) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let Some(folder) = file.parent() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut rest = source.as_str();
    while let Some(at) = rest.find("import(") {
        rest = &rest[at + "import(".len()..];
        let said = rest.trim_start();
        let Some(quote) = said
            .chars()
            .next()
            .filter(|quote| matches!(quote, '"' | '\'' | '`'))
        else {
            continue;
        };
        let Some(end) = said[1..].find(quote) else {
            continue;
        };
        let wanted = &said[1..=end];
        if !wanted.starts_with('.') || wanted.contains("${") {
            continue;
        }
        let path = folder.join(wanted);
        if let Ok(path) = std::fs::canonicalize(&path) {
            let path = plain_path(&path);
            if path.is_file() && !found.contains(&path) {
                found.push(path);
            }
        }
    }
    found
}

/// The folder of the package this file's in, if it's in one: what comes after the last `node_modules`, or
/// the two parts after it for one with a scope.
pub(crate) fn package_folder(path: &Path) -> Option<PathBuf> {
    let parts: Vec<_> = path.components().collect();
    let at = parts
        .iter()
        .rposition(|part| part.as_os_str().eq_ignore_ascii_case("node_modules"))?;
    let name = parts.get(at + 1)?.as_os_str();
    let count = if name.to_string_lossy().starts_with('@') {
        3
    } else {
        2
    };
    if parts.len() <= at + count {
        return None;
    }
    Some(parts[..=at + count - 1].iter().collect())
}

/// The nearest folder that has all of these in it.
fn folder_around(files: &[PathBuf]) -> Option<PathBuf> {
    let mut around = files.first()?.parent()?.to_owned();
    for file in files {
        while !file.starts_with(&around) {
            around = around.parent()?.to_owned();
        }
    }
    Some(around)
}

/// Says that this doesn't want a console, so that none comes up with it. The field that says so is in the
/// program's header, as Windows lays it out.
#[cfg(windows)]
fn stop_wanting_a_console(file: &mut std::fs::File) -> std::io::Result<()> {
    const WINDOWS_GUI: u16 = 2;
    const SUBSYSTEM_FROM_SIGNATURE: u64 = 4 + 20 + 68;
    let mut place = [0u8; 4];
    file.seek(SeekFrom::Start(0x3C))?;
    file.read_exact(&mut place)?;
    let at = u64::from(u32::from_le_bytes(place)) + SUBSYSTEM_FROM_SIGNATURE;
    file.seek(SeekFrom::Start(at))?;
    file.write_all(&WINDOWS_GUI.to_le_bytes())
}

/// A program elsewhere has no icon of its own: the desktop file that starts it says what its icon is. The icon's still
/// checked, so a bad one is said to be bad on every system.
#[cfg(not(windows))]
fn set_icon(_program: &Path, icon: &[u8]) -> Result<(), String> {
    read_icon(icon).map(|_| ())
}

/// The pictures in a `.ico` file, and the list of them as a program's resources have it.
fn read_icon(icon: &[u8]) -> Result<(Vec<u8>, Vec<&[u8]>), String> {
    let bad = || "that isn't an .ico file".to_owned();
    let (Some(&[0, 0, 1, 0]), Some(&[low, high])) = (
        icon.get(..4)
            .map(|start| <&[u8; 4]>::try_from(start).expect("this is four bytes")),
        icon.get(4..6)
            .map(|count| <&[u8; 2]>::try_from(count).expect("this is two bytes")),
    ) else {
        return Err(bad());
    };
    let count = usize::from(u16::from_le_bytes([low, high]));
    let mut group = icon[..6].to_vec();
    let mut pictures = Vec::with_capacity(count);
    for index in 0..count {
        let entry = icon
            .get(6 + index * 16..6 + (index + 1) * 16)
            .ok_or_else(bad)?;
        let size = u32::from_le_bytes(entry[8..12].try_into().map_err(|_| bad())?) as usize;
        let at = u32::from_le_bytes(entry[12..16].try_into().map_err(|_| bad())?) as usize;
        pictures.push(icon.get(at..at.saturating_add(size)).ok_or_else(bad)?);
        group.extend_from_slice(&entry[..12]);
        group.extend_from_slice(&((index + 1) as u16).to_le_bytes());
    }
    Ok((group, pictures))
}

/// Gives a program an icon, from the bytes of a `.ico` file. In a program, each picture in one is a
/// resource of its own, and the list of them is another with a slightly different form.
#[cfg(windows)]
fn set_icon(program: &Path, icon: &[u8]) -> Result<(), String> {
    const ICON: usize = 3;
    const ICON_GROUP: usize = 14;
    let (group, pictures) = read_icon(icon)?;
    let path = wide(program);
    // SAFETY: the path ends in a terminator. A kind and a name can be a small number, in place of a pointer,
    // and the data lasts until the update has ended, which is what it's copied by.
    unsafe {
        let update = BeginUpdateResourceW(path.as_ptr(), 0);
        if update.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut worked = true;
        for (index, picture) in pictures.iter().enumerate() {
            worked &= UpdateResourceW(
                update,
                ICON as *const u16,
                (index + 1) as *const u16,
                0,
                picture.as_ptr().cast(),
                picture.len() as u32,
            ) != 0;
        }
        worked &= UpdateResourceW(
            update,
            ICON_GROUP as *const u16,
            std::ptr::without_provenance(1),
            0,
            group.as_ptr().cast(),
            group.len() as u32,
        ) != 0;
        if EndUpdateResourceW(update, i32::from(!worked)) == 0 || !worked {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}

/// Makes one program at `out` from the app at `app`, which runs it once to see what it loads. `icon` is
/// a `.ico` file for it to show. `code` is what its files compiled to then, if that's to go in too.
pub(crate) fn pack(
    app: &Path,
    out: &Path,
    icon: Option<&Path>,
    loaded: &[PathBuf],
    also: &[PathBuf],
    builtins: &[String],
    code: Option<&rsui_js::CompiledCode>,
    graph: &[PathBuf],
    diagnostics: bool,
) -> Result<(), String> {
    let describe = |what: &str, error: &dyn std::fmt::Display| format!("{what}: {error}");
    let this = std::env::current_exe().map_err(|error| describe("finding this program", &error))?;
    // Who the app is, from its `rsui.config.ts`, which is checked now, before anything's made.
    // An app without one packs as it always did, whatever its `package.json` is like.
    let config = match crate::config::find(app) {
        Some(_) => crate::config::load(app, crate::config::this_platform())?,
        None => crate::config::load(app, crate::config::this_platform()).unwrap_or(serde_json::Value::Null),
    };
    let mut also = also.to_vec();
    if let Some(folder) = crate::config::find(app).and_then(|file| file.parent().map(Path::to_owned)) {
        let list = |key: &str| -> Vec<String> {
            config["files"][key].as_array().into_iter().flatten().filter_map(|item| item.as_str().map(str::to_owned)).collect()
        };
        for file in crate::config::included_files(&folder, &list("include"), &list("exclude")) {
            if !also.contains(&file) {
                also.push(file);
            }
        }
    }
    let files = files_to_pack(app, loaded, &also, graph)
        .map_err(|error| describe("reading the app", &error))?;
    let entry = std::fs::canonicalize(app).map_err(|error| describe("finding the app", &error))?;
    let entry = plain_path(&entry);
    let mut links = links_to(&files);
    // A link can be further up than any file, as a workspace's packages are found from the `node_modules` at its top.
    let mut around = files.clone();
    around.extend(links.iter().map(|(link, _)| link.clone()));
    let root = folder_around(&around).ok_or("the app didn't load anything")?;
    links.retain(|(_, target)| target.starts_with(&root));
    // `--icon` first, then the config's, which can be a .png.
    let icon = match icon {
        Some(icon) => Some(std::fs::read(icon).map_err(|error| describe("reading the icon", &error))?),
        None => match config["icon"].as_str() {
            Some(path) => {
                let bytes = std::fs::read(path).map_err(|error| describe("reading the icon", &error))?;
                Some(if path.to_ascii_lowercase().ends_with(".png") {
                    ico_from_png(&bytes).map_err(|error| describe("the icon", &error))?
                } else {
                    bytes
                })
            }
            None => None,
        },
    };
    // Publish only a completed program. A failed rebuild preserves an existing
    // output, rather than truncating it and then deleting it during rollback.
    let staged = out.with_file_name(format!(
        ".{}.rsui-{}-{}.tmp",
        out.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| {
        std::fs::copy(&this, &staged).map_err(|error| describe("copying this program", &error))?;
        let opened = || OpenOptions::new().read(true).write(true).open(&staged);
        #[cfg(windows)]
        {
            let mut file = opened().map_err(|error| describe("opening what was made", &error))?;
            stop_wanting_a_console(&mut file).map_err(|error| describe("writing it", &error))?;
        }
        if let Some(icon) = icon {
            set_icon(&staged, &icon).map_err(|error| describe("adding the icon", &error))?;
        }
        // An app with no config of its own is known as it always was (`rsui.<name>`), so nothing it's pinned as changes.
        if crate::config::find(app).is_some() {
            set_config(&staged, &config).map_err(|error| describe("adding the config", &error))?;
        }
        // This goes last, as adding an icon rewrites the file, and would lose what's on its end.
        let mut file = opened().map_err(|error| describe("opening what was made", &error))?;
        rsui_js::append_packed_files_with_diagnostics(
            &mut file,
            &root,
            &entry,
            &files,
            loaded,
            builtins,
            &links,
            code,
            diagnostics,
        )
        .map_err(|error| describe("adding the app", &error))
    })()
    .and_then(|()| {
        std::fs::rename(&staged, out).map_err(|error| describe("publishing the app", &error))
    });
    if result.is_err() {
        // Half a program is worse than none, and this just made it.
        let _ = std::fs::remove_file(&staged);
    }
    result
}

/// Where a file can be found on the disk: where it says, unless it's inside this packed program, in which case it's put
/// in a folder of the program's own, for what can only use a file on the disk, as Windows does to play a sound.
pub(crate) fn on_the_disk(path: &Path) -> PathBuf {
    let Some(bytes) = rsui_js::read_packed_file(path) else {
        return path.to_owned();
    };
    let program = std::env::current_exe().unwrap_or_default();
    let stem = program
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    // Each version of the program has its own, so one never plays what another had.
    let made = std::fs::metadata(&program)
        .and_then(|found| found.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs());
    let mut name = String::new();
    for part in path.components().skip(1) {
        if !name.is_empty() {
            name.push('-');
        }
        name.push_str(&part.as_os_str().to_string_lossy());
    }
    let place = std::env::temp_dir()
        .join(format!("{stem}-{made}"))
        .join(name.replace(['\\', '/', ':'], "-"));
    let same = std::fs::metadata(&place).is_ok_and(|found| found.len() == bytes.len() as u64);
    if !same {
        let written = place
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&place, &bytes));
        if written.is_err() {
            return path.to_owned();
        }
    }
    place
}

/// A path without the `\\?\` that Windows puts on the front of a full one.
pub(crate) fn plain_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn a_package_is_the_folder_after_the_last_node_modules() {
        let of = |path: &str| package_folder(Path::new(path));
        assert_eq!(
            of(r"C:\a\node_modules\pkg\lib\x.js"),
            Some(PathBuf::from(r"C:\a\node_modules\pkg"))
        );
        assert_eq!(
            of(r"C:\a\node_modules\@scope\pkg\x.js"),
            Some(PathBuf::from(r"C:\a\node_modules\@scope\pkg"))
        );
        assert_eq!(
            of(r"C:\a\node_modules\one\node_modules\two\x.js"),
            Some(PathBuf::from(r"C:\a\node_modules\one\node_modules\two"))
        );
        assert_eq!(of(r"C:\a\src\x.js"), None);
    }

    #[test]
    fn an_app_is_packed_with_its_pictures_fonts_and_sounds_and_the_packages_it_uses() {
        let folder = std::env::temp_dir().join(format!("rsui-pack-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        for (path, text) in [
            ("app.tsx", "code"),
            ("src/view.tsx", "code"),
            ("src/unused.tsx", "code that isn't run"),
            ("assets/logo.svg", "<svg/>"),
            ("assets/font.woff2", "font"),
            ("assets/ding.wav", "sound"),
            ("notes.md", "not for the app"),
            (".git/thing.png", "not for the app"),
            ("node_modules/used/index.js", "code"),
            ("node_modules/used/other.js", "code"),
            ("node_modules/used/font.ttf", "font"),
            ("node_modules/unused/index.js", "code"),
        ] {
            let path = folder.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let folder = plain_path(&std::fs::canonicalize(&folder).unwrap());
        let loaded = [
            folder.join("app.tsx"),
            folder.join("src").join("view.tsx"),
            folder.join("node_modules").join("used").join("index.js"),
        ];
        let packed = files_to_pack(&folder.join("app.tsx"), &loaded, &[], &[]).unwrap();
        let _ = std::fs::remove_dir_all(&folder);
        let mut names: Vec<String> = packed
            .iter()
            .map(|path| {
                path.strip_prefix(&folder)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "app.tsx",
                "assets/ding.wav",
                "assets/font.woff2",
                "assets/logo.svg",
                "node_modules/used/font.ttf",
                "node_modules/used/index.js",
                "node_modules/used/other.js",
                "src/view.tsx",
            ]
        );
    }

    #[test]
    #[cfg(windows)]
    fn the_folder_around_files_is_the_nearest_one() {
        let files = [
            PathBuf::from(r"C:\a\b\c\one.tsx"),
            PathBuf::from(r"C:\a\b\two.ts"),
            PathBuf::from(r"C:\a\b\d\three.ts"),
        ];
        assert_eq!(folder_around(&files), Some(PathBuf::from(r"C:\a\b")));
    }

    #[test]
    fn something_that_is_not_an_icon_is_refused_and_the_program_stays_as_it_was() {
        let file = std::env::temp_dir().join(format!("rsui-pack-{}.exe", std::process::id()));
        std::fs::write(&file, b"not used").unwrap();
        assert!(set_icon(&file, b"this is not an icon").is_err());
        assert!(
            set_icon(&file, &[0, 0, 1, 0]).is_err(),
            "too short to say how many"
        );
        assert!(
            set_icon(&file, &[0, 0, 1, 0, 1, 0]).is_err(),
            "no room for what it says"
        );
        let mut far_off = vec![0, 0, 1, 0, 1, 0];
        far_off.extend_from_slice(&[0; 8]);
        far_off.extend_from_slice(&u32::MAX.to_le_bytes());
        far_off.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(set_icon(&file, &far_off).is_err(), "a picture past the end");
        assert_eq!(std::fs::read(&file).unwrap(), b"not used");
        let _ = std::fs::remove_file(&file);
    }
}
