//! An app's `rsui.config.ts`: who the app is and how it's packed, as Tauri's `tauri.conf.json` has it, in one typed file
//! beside its `package.json`.
//!
//! ```ts
//! import { defineConfig } from 'rsui/config';
//! export default defineConfig(({ platform }) => ({ id: 'nl.example.todo', name: 'To-do', icon: 'icon.png' }));
//! ```
//!
//! It's read with rsui's own engine when the app's packed, and by `rsui-tsx config`, which prints it for scripts that can't
//! run TypeScript (`tools/android/build-apk.ps1`). A packed app never reads it: what it needs of it is written into the
//! program when it's packed (see `pack::set_config`). What's left out is filled in from `package.json`, or the folder's
//! name, and everything's checked, so a wrong id or a misspelt key is said when packing, not found out on a phone.
//!
//! What a window is like (`title`, `size`, `frameless`, ...) stays as exports of the app's main file: that's code, which can
//! depend on things and change while the app runs.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// The names the file can have, in the order they're looked for.
const NAMES: [&str; 4] = ["rsui.config.ts", "rsui.config.mts", "rsui.config.js", "rsui.config.mjs"];

/// The systems a config can say something for, by the name it uses for them.
pub(crate) const PLATFORMS: [&str; 4] = ["windows", "android", "macos", "linux"];

/// What an app may ask the system for, and what each is on Android, which is the one that has to be told up front.
const PERMISSIONS: [(&str, &[&str]); 6] = [
    ("network", &["android.permission.INTERNET", "android.permission.ACCESS_NETWORK_STATE"]),
    ("notifications", &["android.permission.POST_NOTIFICATIONS"]),
    ("camera", &["android.permission.CAMERA"]),
    ("microphone", &["android.permission.RECORD_AUDIO"]),
    ("location", &["android.permission.ACCESS_COARSE_LOCATION", "android.permission.ACCESS_FINE_LOCATION"]),
    ("vibrate", &["android.permission.VIBRATE"]),
];

const THEMES: [&str; 4] = ["system", "material", "fluent", "mac"];

/// This system, by the name a config uses for it.
pub(crate) fn this_platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_vendor = "apple") {
        "macos"
    } else {
        "linux"
    }
}

/// The folder an app's config is in: its main file's, or the folder itself if that's what was given.
fn folder_of(app: &Path) -> PathBuf {
    if app.is_dir() {
        app.to_owned()
    } else {
        app.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_owned()
    }
}

/// The app's config file, if it has one. Only its own folder is looked in, never the folders above it.
pub(crate) fn find(app: &Path) -> Option<PathBuf> {
    let folder = folder_of(app);
    NAMES.iter().map(|name| folder.join(name)).find(|path| path.is_file())
}

/// The app's config for `platform`, with what it leaves out filled in, checked, as JSON. An app with no config file has
/// one made of its `package.json` and its folder's name.
pub(crate) fn load(app: &Path, platform: &str) -> Result<Value, String> {
    if !PLATFORMS.contains(&platform) {
        return Err(format!("there's no platform called {platform}: it's one of {}", PLATFORMS.join(", ")));
    }
    let folder = folder_of(app);
    let said = match find(app) {
        Some(file) => evaluate(&file, platform)?,
        None => Value::Object(Map::new()),
    };
    resolve(&folder, said, platform)
}

/// Runs the config file in an engine of its own, on a thread of its own, so that it's never beside an app's engine, and
/// gives what it exports as its default: an object, or a function of `{ platform, mode }` that gives one.
fn evaluate(file: &Path, platform: &str) -> Result<Value, String> {
    let (file, platform) = (file.to_owned(), platform.to_owned());
    std::thread::Builder::new()
        .name("rsui config".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || evaluate_here(&file, &platform))
        .map_err(|error| error.to_string())?
        .join()
        .unwrap_or_else(|_| Err("reading the config panicked".to_owned()))
}

fn evaluate_here(file: &Path, platform: &str) -> Result<Value, String> {
    let shown = file.display();
    let mut engine = rsui_js::Engine::new();
    engine.add_module("rsui/config", "exports.defineConfig = (config) => config;");
    let exports = engine.run_file(file).map_err(|error| format!("{shown}: {}", error.message))?;
    engine.vm().set_global("__rsuiConfigExports", exports);
    let mode = if rsui_js::running_packed() { "production" } else { "development" };
    let script = format!(
        "(() => {{ const made = __rsuiConfigExports.default; \
         const config = typeof made === 'function' ? made({{ platform: '{platform}', mode: '{mode}' }}) : made; \
         if (config && typeof config.then === 'function') throw new Error('the config has to be made at once, not as a promise'); \
         return JSON.stringify(config === undefined ? {{}} : config); }})()"
    );
    let text = engine.eval(&script, "rsui.config").map_err(|error| format!("{shown}: {}", error.message))?;
    let text = engine.vm().string_text(text).map(|text| text.into_owned()).ok_or_else(|| format!("{shown}: the config isn't an object"))?;
    let value: Value = serde_json::from_str(&text).map_err(|error| format!("{shown}: {error}"))?;
    if !value.is_object() {
        return Err(format!("{shown}: its default export has to be an object, or a function that gives one"));
    }
    Ok(value)
}

/// What `package.json` says, if there is one.
fn package_json(folder: &Path) -> Map<String, Value> {
    std::fs::read_to_string(folder.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

/// A name made of letters and digits only, starting with a letter, as part of an id.
pub(crate) fn id_part(name: &str) -> String {
    let part: String = name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match part.chars().next() {
        Some(first) if first.is_ascii_alphabetic() => part,
        Some(_) => format!("app{part}"),
        None => "app".to_owned(),
    }
}

/// Whether `id` is reverse-domain, as Android's package names, Apple's bundle ids and Windows' app ids all take: two or more
/// parts, each a letter and then letters, digits or `_`.
fn valid_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split('.').collect();
    parts.len() >= 2
        && id.len() <= 128
        && parts.iter().all(|part| {
            part.chars().next().is_some_and(|first| first.is_ascii_alphabetic())
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// `1`, `1.2` or `1.2.3`, with anything after a `-` or `+` (as `1.2.3-beta`) left as it is.
fn version_numbers(version: &str) -> Option<[u32; 3]> {
    let core = version.split(['-', '+']).next()?;
    let mut numbers = [0u32; 3];
    let parts: Vec<&str> = core.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    for (slot, part) in numbers.iter_mut().zip(&parts) {
        *slot = part.parse().ok()?;
    }
    Some(numbers)
}

fn text(value: Option<&Value>, key: &str) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!("`{key}` has to be a string")),
    }
}

fn texts(value: Option<&Value>, key: &str) -> Result<Vec<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned).ok_or_else(|| format!("`{key}` has to be a list of strings")))
            .collect(),
        Some(_) => Err(format!("`{key}` has to be a list of strings")),
    }
}

/// Says which keys aren't known, with the nearest known one, as a misspelling is the likeliest reason.
fn only_known(object: &Map<String, Value>, known: &[&str], within: &str) -> Result<(), String> {
    for key in object.keys() {
        if !known.contains(&key.as_str()) {
            let near = known.iter().min_by_key(|candidate| distance(key, candidate)).filter(|near| distance(key, near) <= 3);
            let place = if within.is_empty() { String::new() } else { format!(" in `{within}`") };
            return Err(match near {
                Some(near) => format!("there's no `{key}`{place}: did you mean `{near}`?"),
                None => format!("there's no `{key}`{place}: it has {}", known.join(", ")),
            });
        }
    }
    Ok(())
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let here = row[j + 1];
            row[j + 1] = (previous + usize::from(ca != *cb)).min(row[j] + 1).min(here + 1);
            previous = here;
        }
    }
    row[b.len()]
}

/// Fills in what the config leaves out, checks it, and gives it back with every key there is.
fn resolve(folder: &Path, said: Value, platform: &str) -> Result<Value, String> {
    let said = said.as_object().cloned().unwrap_or_default();
    only_known(
        &said,
        &["id", "name", "version", "icon", "files", "permissions", "theme", "windows", "android", "macos", "linux"],
        "",
    )?;
    let package = package_json(folder);
    let folder_name = std::fs::canonicalize(folder)
        .ok()
        .and_then(|whole| whole.file_name().map(|name| name.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "app".to_owned());
    let package_name = package.get("name").and_then(Value::as_str).map(|name| name.rsplit('/').next().unwrap_or(name).to_owned());

    let name = text(said.get("name"), "name")?
        .or_else(|| package.get("productName").and_then(Value::as_str).map(str::to_owned))
        .or_else(|| package_name.clone())
        .unwrap_or_else(|| folder_name.clone());
    if name.trim().is_empty() {
        return Err("`name` can't be empty".to_owned());
    }
    let id = text(said.get("id"), "id")?
        .unwrap_or_else(|| format!("dev.rsui.{}", id_part(package_name.as_deref().unwrap_or(&folder_name))));
    if !valid_id(&id) {
        return Err(format!(
            "`id` is {id:?}, which isn't one: it's written the other way round from a web address, as `com.example.todo`: two \
             or more parts, with dots between, each a letter and then letters, digits or `_`"
        ));
    }
    // A version the config gives has to be numbers; one from `package.json`, which wasn't written for this, is taken as it
    // is, with Android's number for it 1 if it isn't.
    let given = text(said.get("version"), "version")?;
    let [major, minor, patch] = match &given {
        Some(version) => version_numbers(version)
            .ok_or_else(|| format!("`version` is {version:?}: it has to be numbers, as `1.2.3`"))?,
        None => [0, 0, 0],
    };
    let version = given
        .or_else(|| package.get("version").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| "0.1.0".to_owned());
    let [major, minor, patch] = if said.contains_key("version") {
        [major, minor, patch]
    } else {
        version_numbers(&version).unwrap_or([0, 0, 1])
    };

    let icon = match text(said.get("icon"), "icon")? {
        Some(icon) => {
            let path = folder.join(&icon);
            if !path.is_file() {
                return Err(format!("`icon` is {icon}, which isn't there ({})", path.display()));
            }
            let kind = path.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase);
            if !matches!(kind.as_deref(), Some("png" | "ico")) {
                return Err(format!("`icon` is {icon}: it has to be a .png (256 by 256 is best) or a .ico"));
            }
            Some(Value::String(path.to_string_lossy().into_owned()))
        }
        None => None,
    };

    let files = match said.get("files") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(files)) => {
            only_known(files, &["include", "exclude"], "files")?;
            files.clone()
        }
        Some(_) => return Err("`files` has to be `{ include: [...], exclude: [...] }`".to_owned()),
    };
    let include = texts(files.get("include"), "files.include")?;
    let exclude = texts(files.get("exclude"), "files.exclude")?;

    let permissions = texts(said.get("permissions"), "permissions")?;
    let known: Vec<&str> = PERMISSIONS.iter().map(|(name, _)| *name).collect();
    for permission in &permissions {
        if !known.contains(&permission.as_str()) {
            return Err(format!("there's no permission called {permission}: it's one of {}", known.join(", ")));
        }
    }

    let theme = text(said.get("theme"), "theme")?.unwrap_or_else(|| "system".to_owned());
    if !THEMES.contains(&theme.as_str()) {
        return Err(format!("`theme` is {theme}: it's one of {}", THEMES.join(", ")));
    }

    let section = |name: &str, known: &[&str]| -> Result<Map<String, Value>, String> {
        match said.get(name) {
            None | Some(Value::Null) => Ok(Map::new()),
            Some(Value::Object(section)) => {
                only_known(section, known, name)?;
                Ok(section.clone())
            }
            Some(_) => Err(format!("`{name}` has to be an object")),
        }
    };
    let android = section("android", &["minSdk", "targetSdk", "orientation", "versionCode"])?;
    let min_sdk = android.get("minSdk").map_or(Some(29), Value::as_u64).ok_or("`android.minSdk` has to be a number")?;
    if min_sdk < 29 {
        return Err(format!("`android.minSdk` is {min_sdk}: rsui needs Android 10 (29) or later"));
    }
    let target_sdk = android.get("targetSdk").map_or(Some(34), Value::as_u64).ok_or("`android.targetSdk` has to be a number")?;
    let orientation = text(android.get("orientation"), "android.orientation")?.unwrap_or_else(|| "any".to_owned());
    if !["any", "portrait", "landscape"].contains(&orientation.as_str()) {
        return Err(format!("`android.orientation` is {orientation}: it's any, portrait or landscape"));
    }
    // Each version has to have a bigger number than the one before for Android to install it over it, which this is, as
    // long as minor and patch stay under 100.
    let version_code = android
        .get("versionCode")
        .map_or(Some(u64::from(major) * 10_000 + u64::from(minor.min(99)) * 100 + u64::from(patch.min(99))), Value::as_u64)
        .ok_or("`android.versionCode` has to be a number")?
        .max(1);
    let android_permissions: Vec<&str> = PERMISSIONS
        .iter()
        .filter(|(name, _)| permissions.iter().any(|asked| asked == name))
        .flat_map(|(_, names)| names.iter().copied())
        .collect();

    let macos = section("macos", &["category", "minimumSystemVersion"])?;
    let category = text(macos.get("category"), "macos.category")?.unwrap_or_else(|| "public.app-category.utilities".to_owned());
    let minimum_macos = text(macos.get("minimumSystemVersion"), "macos.minimumSystemVersion")?.unwrap_or_else(|| "14.0".to_owned());
    let _windows = section("windows", &[])?;
    let linux = section("linux", &["categories"])?;
    let categories = texts(linux.get("categories"), "linux.categories")?;

    Ok(json!({
        "platform": platform,
        "id": id,
        "name": name,
        "version": version,
        "icon": icon,
        "files": { "include": include, "exclude": exclude },
        "permissions": permissions,
        "theme": theme,
        "android": {
            "minSdk": min_sdk,
            "targetSdk": target_sdk,
            "orientation": orientation,
            "versionCode": version_code,
            "permissions": android_permissions,
        },
        "macos": { "category": category, "minimumSystemVersion": minimum_macos },
        "windows": {},
        "linux": { "categories": if categories.is_empty() { vec!["Utility".to_owned()] } else { categories } },
    }))
}

/// Whether `path` (with `/` between its parts) matches `pattern`, where `*` is anything within a part, `**` any number of
/// whole parts, and `?` one character.
pub(crate) fn glob_matches(pattern: &str, path: &str) -> bool {
    fn parts(pattern: &[&str], path: &[&str]) -> bool {
        match pattern.split_first() {
            None => path.is_empty(),
            Some((&"**", rest)) => (0..=path.len()).any(|skip| parts(rest, &path[skip..])),
            Some((first, rest)) => path.split_first().is_some_and(|(part, after)| one(first.as_bytes(), part.as_bytes()) && parts(rest, after)),
        }
    }
    fn one(pattern: &[u8], text: &[u8]) -> bool {
        match pattern.split_first() {
            None => text.is_empty(),
            Some((b'*', rest)) => (0..=text.len()).any(|skip| one(rest, &text[skip..])),
            Some((b'?', rest)) => !text.is_empty() && one(rest, &text[1..]),
            Some((c, rest)) => text.first() == Some(c) && one(rest, &text[1..]),
        }
    }
    let pattern: Vec<&str> = pattern.trim_start_matches("./").split('/').filter(|part| !part.is_empty()).collect();
    let path: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    parts(&pattern, &path)
}

/// The files under `folder` that `include` asks for and `exclude` doesn't take out, for packing beside what the app loads.
pub(crate) fn included_files(folder: &Path, include: &[String], exclude: &[String]) -> Vec<PathBuf> {
    fn walk(folder: &Path, at: &Path, out: &mut Vec<(PathBuf, String)>) {
        let Ok(entries) = std::fs::read_dir(at) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if name == "node_modules" || name == ".git" || name == "target" {
                continue;
            }
            if path.is_dir() {
                walk(folder, &path, out);
            } else if let Ok(relative) = path.strip_prefix(folder) {
                let relative = relative.to_string_lossy().replace('\\', "/");
                out.push((path, relative));
            }
        }
    }
    if include.is_empty() {
        return Vec::new();
    }
    let mut all = Vec::new();
    walk(folder, folder, &mut all);
    all.into_iter()
        .filter(|(_, relative)| include.iter().any(|pattern| glob_matches(pattern, relative)))
        .filter(|(_, relative)| !exclude.iter().any(|pattern| glob_matches(pattern, relative)))
        .map(|(path, _)| path)
        .collect()
}

/// What a running app needs of its config: its id, for Windows to know it by, and which look `native` draws in (`theme`).
#[derive(Default)]
pub(crate) struct Running {
    pub(crate) id: Option<String>,
    pub(crate) theme: Option<String>,
}

/// What a running app needs of its config, as cheaply as it can be had: a packed app has it written in (see
/// `pack::packed_config`), and one that isn't packed has its config read, if it has one.
pub(crate) fn running(app: &Path) -> Running {
    let config = if rsui_js::running_packed() {
        crate::pack::packed_config()
    } else if find(app).is_some() {
        load(app, this_platform()).map_err(|error| eprintln!("rsui-tsx: {error}")).ok()
    } else {
        None
    };
    let said = |key: &str| config.as_ref().and_then(|config| config.get(key)?.as_str().map(str::to_owned));
    Running { id: said("id"), theme: said("theme") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rsui_config_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            std::fs::write(dir.join(file), text).unwrap();
        }
        dir
    }

    #[test]
    fn what_a_config_leaves_out_comes_from_package_json_and_the_folder() {
        let dir = folder("defaults", &[("package.json", r#"{ "name": "@me/To-Do", "version": "2.3.4" }"#), ("app.tsx", "")]);
        let config = load(&dir.join("app.tsx"), "android").unwrap();
        assert_eq!(config["id"], "dev.rsui.todo");
        assert_eq!(config["name"], "To-Do");
        assert_eq!(config["version"], "2.3.4");
        assert_eq!(config["android"]["versionCode"], 20304);
        assert_eq!(config["android"]["minSdk"], 29);
        assert_eq!(config["theme"], "system");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_typescript_config_is_run_and_its_function_is_given_the_platform() {
        let dir = folder(
            "ts",
            &[(
                "rsui.config.ts",
                "import { defineConfig } from 'rsui/config';\n\
                 type Id = string;\n\
                 const id: Id = 'nl.example.todo';\n\
                 export default defineConfig(({ platform }: { platform: string }) => ({\n\
                   id, name: 'To-do', version: '1.2.0', permissions: platform === 'android' ? ['network'] : [],\n\
                   android: { orientation: 'portrait' },\n\
                 }));\n",
            )],
        );
        let android = load(&dir, "android").unwrap();
        assert_eq!(android["id"], "nl.example.todo");
        assert_eq!(android["android"]["orientation"], "portrait");
        assert_eq!(android["android"]["permissions"], json!(["android.permission.INTERNET", "android.permission.ACCESS_NETWORK_STATE"]));
        let windows = load(&dir, "windows").unwrap();
        assert_eq!(windows["permissions"], json!([]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn mistakes_are_said_with_what_was_meant() {
        let cases = [
            ("export default { idd: 'a.b' };", "did you mean `id`?"),
            ("export default { id: 'todo' };", "isn't one"),
            ("export default { id: 'com.1x.todo' };", "isn't one"),
            ("export default { version: 'one' };", "has to be numbers"),
            ("export default { permissions: ['camra'] };", "no permission called camra"),
            ("export default { android: { minSdk: 21 } };", "Android 10"),
            ("export default { android: { orientaton: 'portrait' } };", "did you mean `orientation`?"),
            ("export default { icon: 'missing.png' };", "isn't there"),
            ("export default { theme: 'neon' };", "`theme` is neon"),
        ];
        for (index, (source, said)) in cases.iter().enumerate() {
            let dir = folder(&format!("bad{index}"), &[("rsui.config.js", source)]);
            let error = load(&dir, "windows").unwrap_err();
            assert!(error.contains(said), "{source}: {error}");
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn globs_match_parts_and_whole_folders() {
        assert!(glob_matches("assets/**", "assets/a/b.png"));
        assert!(glob_matches("assets/**", "assets/b.png"));
        assert!(!glob_matches("assets/**", "src/b.png"));
        assert!(glob_matches("**/*.psd", "assets/deep/x.psd"));
        assert!(glob_matches("*.txt", "notes.txt"));
        assert!(!glob_matches("*.txt", "a/notes.txt"));
        assert!(glob_matches("./data/??.json", "data/en.json"));
    }

    #[test]
    fn ids_are_reverse_domain() {
        assert!(valid_id("com.example.todo"));
        assert!(valid_id("nl.rick.my_app2"));
        assert!(!valid_id("todo"));
        assert!(!valid_id("com..todo"));
        assert!(!valid_id("com.example.to-do"));
        assert_eq!(id_part("My Cool-App!"), "mycoolapp");
        assert_eq!(id_part("2048"), "app2048");
    }
}
