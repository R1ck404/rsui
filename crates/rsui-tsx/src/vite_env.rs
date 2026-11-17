//! `import.meta.env` for an app's code, as Vite makes it, so an app written for Vite finds what it reads there:
//! `MODE`, `DEV`, `PROD`, `BASE_URL` and `SSR`, and the variables from its `.env` files whose names start with
//! `VITE_`, or with what its config's `envPrefix` says.
//!
//! The files are in the app's root, which is the nearest folder at or above its main file with a Vite config or a
//! `package.json`. They're read as Vite reads them, each overriding the one before: `.env`, `.env.local`, `.env.[mode]`
//! and `.env.[mode].local`, and then what's in the environment overrides them all. The mode is `development`, or
//! `production` for a packed app. `${NAME}` and `$NAME` in a value are other variables, as dotenv-expand has them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rsui_js::MetaEnvValue;

const CONFIGS: [&str; 6] = [
    "vite.config.ts",
    "vite.config.mts",
    "vite.config.js",
    "vite.config.mjs",
    "vite.config.cts",
    "vite.config.cjs",
];

/// What an app with this main file has as `import.meta.env`.
pub(crate) fn import_meta_env(main: &Path, packed: bool) -> Vec<(String, MetaEnvValue)> {
    let mode = if packed { "production" } else { "development" };
    let folder = std::path::absolute(main)
        .ok()
        .and_then(|file| file.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let root = app_root(&folder).unwrap_or(folder);
    let config = CONFIGS.iter().find_map(|name| read(&root.join(name)));
    let prefixes = config
        .as_deref()
        .and_then(|text| strings_after(text, "envPrefix"))
        .filter(|prefixes| !prefixes.is_empty() && prefixes.iter().all(|prefix| !prefix.is_empty()))
        .unwrap_or_else(|| vec!["VITE_".to_owned()]);
    let base = config
        .as_deref()
        .and_then(|text| strings_after(text, "base"))
        .and_then(|found| found.into_iter().next())
        .unwrap_or_else(|| "/".to_owned());
    let environment: BTreeMap<String, String> = std::env::vars().collect();
    let files = [".env".to_owned(), ".env.local".to_owned(), format!(".env.{mode}"), format!(".env.{mode}.local")];
    let mut loaded: BTreeMap<String, String> = BTreeMap::new();
    for file in files {
        if let Some(text) = read(&root.join(file)) {
            loaded.extend(parse(&text));
        }
    }
    let expanded: Vec<(String, String)> = loaded
        .iter()
        .map(|(key, value)| (key.clone(), expand(value, &loaded, &environment, 0)))
        .collect();
    let wanted = |key: &str| prefixes.iter().any(|prefix| key.starts_with(prefix.as_str()));
    let mut env: BTreeMap<String, String> = expanded.into_iter().filter(|(key, _)| wanted(key)).collect();
    // What's in the environment already wins over the files.
    for (key, value) in &environment {
        if wanted(key) {
            env.insert(key.clone(), value.clone());
        }
    }
    let production = mode == "production";
    let mut out: Vec<(String, MetaEnvValue)> =
        env.into_iter().map(|(key, value)| (key, MetaEnvValue::Text(value))).collect();
    out.extend([
        ("BASE_URL".to_owned(), MetaEnvValue::Text(base)),
        ("MODE".to_owned(), MetaEnvValue::Text(mode.to_owned())),
        ("DEV".to_owned(), MetaEnvValue::Bool(!production)),
        ("PROD".to_owned(), MetaEnvValue::Bool(production)),
        ("SSR".to_owned(), MetaEnvValue::Bool(false)),
    ]);
    out
}

/// A file's text, from inside the program for a packed app, and from the disk otherwise.
fn read(path: &Path) -> Option<String> {
    if rsui_js::running_packed() {
        if let Some(bytes) = rsui_js::read_packed_file(path) {
            return String::from_utf8(bytes).ok();
        }
    }
    std::fs::read_to_string(path).ok()
}

fn app_root(folder: &Path) -> Option<PathBuf> {
    folder
        .ancestors()
        .find(|at| {
            at.join("package.json").is_file() || CONFIGS.iter().any(|name| at.join(name).is_file())
        })
        .map(Path::to_path_buf)
}

/// The quoted text after `key:` in a config, as `envPrefix: 'APP_'` or `envPrefix: ['APP_', 'VITE_']` say, or `None`
/// if it's not written as plain text there.
fn strings_after(text: &str, key: &str) -> Option<Vec<String>> {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(found) = text[from..].find(key) {
        let at = from + found;
        from = at + key.len();
        let before_is_name = at > 0 && (bytes[at - 1].is_ascii_alphanumeric() || matches!(bytes[at - 1], b'_' | b'$' | b'.'));
        if before_is_name {
            continue;
        }
        let rest = text[at + key.len()..].trim_start();
        let Some(rest) = rest.strip_prefix(':') else {
            continue;
        };
        let rest = rest.trim_start();
        if let Some(list) = rest.strip_prefix('[') {
            let end = list.find(']')?;
            return list[..end].split(',').map(|part| quoted(part.trim())).collect();
        }
        let end = rest.find([',', '\n', '}']).unwrap_or(rest.len());
        return quoted(rest[..end].trim()).map(|one| vec![one]);
    }
    None
}

fn quoted(text: &str) -> Option<String> {
    let quote = text.chars().next().filter(|first| matches!(first, '\'' | '"' | '`'))?;
    let inside = text[1..].strip_suffix(quote)?;
    (!inside.contains(quote)).then(|| inside.to_owned())
}

/// The variables in a `.env` file, as dotenv reads them: `NAME=value`, perhaps after `export`, with `#` starting a
/// comment where a value isn't quoted, and a value in double quotes reading `\n` as a new line.
fn parse(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let line = line.trim_start();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || key.starts_with('#') || !key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) {
            continue;
        }
        let value = value.trim_start();
        let quote = value.chars().next().filter(|first| matches!(first, '\'' | '"' | '`'));
        let value = match quote {
            Some(quote) => {
                // A quoted value can go on over more lines, up to its closing quote.
                let mut whole = value[1..].to_owned();
                while !whole.contains(quote) {
                    match lines.next() {
                        Some(more) => {
                            whole.push('\n');
                            whole.push_str(more);
                        }
                        None => break,
                    }
                }
                let inside = whole.split(quote).next().unwrap_or("").to_owned();
                if quote == '"' { inside.replace("\\n", "\n").replace("\\r", "\r") } else { inside }
            }
            None => {
                let end = value.find(" #").or_else(|| value.find("\t#")).unwrap_or(value.len());
                let value = if value.starts_with('#') { "" } else { &value[..end] };
                value.trim_end().to_owned()
            }
        };
        out.push((key.to_owned(), value));
    }
    out
}

/// `${NAME}` and `$NAME` in a value, as what the environment has under that name, or else the files, as
/// dotenv-expand has it. `\$` is a dollar sign.
fn expand(value: &str, loaded: &BTreeMap<String, String>, environment: &BTreeMap<String, String>, depth: u32) -> String {
    let lookup = |name: &str| -> String {
        if let Some(found) = environment.get(name) {
            return found.clone();
        }
        match loaded.get(name) {
            Some(found) if depth < 8 => expand(found, loaded, environment, depth + 1),
            _ => String::new(),
        }
    };
    let mut out = String::new();
    let mut rest = value;
    while let Some(at) = rest.find('$') {
        if rest[..at].ends_with('\\') {
            out.push_str(&rest[..at - 1]);
            out.push('$');
            rest = &rest[at + 1..];
            continue;
        }
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if let Some(braced) = after.strip_prefix('{') {
            if let Some(end) = braced.find('}') {
                let inner = &braced[..end];
                // `${NAME:-default}` is the default when there's nothing under the name.
                let (name, default) = match inner.split_once(":-") {
                    Some((name, default)) => (name, Some(default)),
                    None => (inner, None),
                };
                let found = lookup(name);
                out.push_str(if found.is_empty() { default.unwrap_or("") } else { &found });
                rest = &braced[end + 1..];
                continue;
            }
        }
        let length = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
        if length == 0 {
            out.push('$');
            rest = after;
            continue;
        }
        out.push_str(&lookup(&after[..length]));
        rest = &after[length..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(found: &[(String, MetaEnvValue)], key: &str) -> Option<String> {
        found.iter().find(|(name, _)| name == key).map(|(_, value)| match value {
            MetaEnvValue::Text(text) => text.clone(),
            MetaEnvValue::Bool(on) => on.to_string(),
        })
    }

    #[test]
    fn an_apps_env_files_are_read_as_vite_reads_them() {
        let root = std::env::temp_dir().join(format!("rsui-tsx-vite-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("package.json"), "{}").unwrap();
        std::fs::write(root.join("vite.config.ts"), "export default defineConfig({ base: './', envPrefix: ['VITE_', 'APP_'] });").unwrap();
        std::fs::write(
            root.join(".env"),
            "# comment\nVITE_NAME=from .env\nVITE_KEPT = plain # not this\nAPP_TITLE=\"two\\nlines\"\nSECRET=hidden\nVITE_BOTH=$VITE_NAME and ${MISSING:-fallback}\n",
        )
        .unwrap();
        std::fs::write(root.join(".env.local"), "VITE_NAME=from .env.local\n").unwrap();
        std::fs::write(root.join(".env.development"), "export VITE_MODE_ONLY='single # kept'\nVITE_NAME=from mode\n").unwrap();
        std::fs::write(root.join(".env.development.local"), "VITE_LAST=mode local\n").unwrap();
        std::fs::write(root.join(".env.production"), "VITE_NAME=production\n").unwrap();
        let found = import_meta_env(&root.join("src").join("main.tsx"), false);
        let packed = import_meta_env(&root.join("src").join("main.tsx"), true);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(text(&found, "VITE_NAME").as_deref(), Some("from mode"));
        assert_eq!(text(&found, "VITE_KEPT").as_deref(), Some("plain"));
        assert_eq!(text(&found, "APP_TITLE").as_deref(), Some("two\nlines"));
        assert_eq!(text(&found, "VITE_MODE_ONLY").as_deref(), Some("single # kept"));
        assert_eq!(text(&found, "VITE_LAST").as_deref(), Some("mode local"));
        assert_eq!(text(&found, "VITE_BOTH").as_deref(), Some("from mode and fallback"));
        assert_eq!(text(&found, "SECRET"), None);
        assert_eq!(text(&found, "BASE_URL").as_deref(), Some("./"));
        assert_eq!(text(&found, "MODE").as_deref(), Some("development"));
        assert_eq!(text(&found, "DEV").as_deref(), Some("true"));
        assert_eq!(text(&found, "PROD").as_deref(), Some("false"));
        assert_eq!(text(&found, "SSR").as_deref(), Some("false"));
        assert_eq!(text(&packed, "MODE").as_deref(), Some("production"));
        assert_eq!(text(&packed, "PROD").as_deref(), Some("true"));
        assert_eq!(text(&packed, "VITE_NAME").as_deref(), Some("production"));
        assert_eq!(text(&packed, "VITE_LAST"), None);
    }

    #[test]
    fn an_app_with_no_config_has_the_defaults() {
        assert_eq!(strings_after("export default {}", "envPrefix"), None);
        assert_eq!(strings_after("{ envPrefix: 'APP_', }", "envPrefix"), Some(vec!["APP_".to_owned()]));
        assert_eq!(strings_after("{ database: 'x', base: \"/sub/\" }", "base"), Some(vec!["/sub/".to_owned()]));
        assert_eq!(strings_after("{ base: process.env.BASE }", "base"), None);
    }
}
