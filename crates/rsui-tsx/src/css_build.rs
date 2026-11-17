//! CSS that the app's own tools make into what a browser gets, as its bundler did: its PostCSS settings, or Tailwind 4, which
//! makes the CSS for the classes the app's code uses. Nothing here knows what those tools do. They're run as they are, from the
//! app's `node_modules`, by `css_build.js`, in an engine of its own on a thread of its own, so the app's engine never loads them
//! and nothing they leave behind is the app's. It's all done before what asked goes on, as the CSS has to be there to draw with.
//!
//! What they make is kept in `node_modules/.cache/rsui`, with what it was made from: the files the tools read, as their sizes and
//! times, and a fingerprint of everything in the app's code that could be a class name. While those are the same, it's used as it
//! is, and making it again is only for when the app's code has something new in it. A packed app has what was made when it was
//! packed inside it, and uses that.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rsui_js::{Args, HostFunction, Value, Vm};

use crate::css::text_of;

/// What `css_build.js` is, which is written next to what it makes, to be run from there.
const BUILDER: &str = include_str!("css_build.js");

/// A fingerprint of some text: FNV-1a, which is the same on every machine and in every version, as what's kept on the disk has to be.
fn fingerprint(parts: &[&[u8]]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in *part {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        // What separates one part from the next, so that moving a byte from one to the other changes it.
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

// What could be a class name

/// What ends a piece of text that could be a class name, outside brackets: spaces, quotes, and what code puts around strings.
fn ends_candidate(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'"' | b'\'' | b'`' | b'{' | b'}' | b';' | b'<' | b'>' | b',' | b'=' | b'\\')
}

/// Where the bracket or parenthesis that starts at `open` closes, if it does before any space, as arbitrary values such as
/// `[&_svg]` and `bg-(--brand)` do. What's inside can have quotes, and other brackets.
fn closing(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (at, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'[' | b'(' => depth += 1,
            b']' | b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            byte if byte.is_ascii_whitespace() => return None,
            _ => {}
        }
    }
    None
}

/// Everything in some text that could be a class name, or a CSS variable's name, into `out`, as Tailwind's own scanner finds
/// them: it doesn't know what the text is, and what isn't a class is left out by Tailwind when it finds it isn't one.
pub(crate) fn candidates_in(text: &str, out: &mut BTreeSet<String>) {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if ends_candidate(bytes[at]) {
            at += 1;
            continue;
        }
        let start = at;
        while at < bytes.len() {
            let byte = bytes[at];
            if byte == b'[' || byte == b'(' {
                match closing(bytes, at) {
                    Some(end) => {
                        at = end + 1;
                        continue;
                    }
                    None if byte == b'(' => break,
                    None => {}
                }
            }
            if ends_candidate(byte) || (byte == b')' && start < at) {
                break;
            }
            at += 1;
        }
        let piece = &text[start..at.max(start)];
        add_candidate(piece, out);
        // What's inside parentheses that aren't a class's own, as in `cn(` or `var(--x)`, is looked at by itself too.
        if let Some(open) = piece.find('(') {
            if open > 0 && !piece[..open].ends_with('-') {
                candidates_in(&piece[open + 1..], out);
            }
        }
        if at == start {
            at += 1;
        }
    }
    // CSS variables, wherever they're named, as `var(--brand)` in a style: a theme only keeps those that are used.
    let mut from = 0;
    while let Some(found) = text[from..].find("--") {
        let begin = from + found;
        let after_word = begin > 0 && {
            let before = bytes[begin - 1];
            before.is_ascii_alphanumeric() || before == b'-' || before == b'_'
        };
        let end = text[begin + 2..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .map_or(text.len(), |length| begin + 2 + length);
        if !after_word && end > begin + 2 {
            out.insert(text[begin..end].to_owned());
        }
        from = end.max(begin + 2);
    }
}

/// A piece of text, without what's around it that a class name can't start or end with, if there's anything left.
fn add_candidate(piece: &str, out: &mut BTreeSet<String>) {
    let starts = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '!' | '[' | '@' | '*' | '_');
    let ends = |c: char| c.is_ascii_alphanumeric() || matches!(c, ']' | ')' | '%' | '!' | '_' | '-');
    let trimmed = piece.trim_start_matches(|c: char| !starts(c)).trim_end_matches(|c: char| !ends(c));
    if !trimmed.is_empty() && trimmed.len() <= 400 && trimmed.chars().any(|c| c.is_ascii_alphabetic()) {
        out.insert(trimmed.to_owned());
    }
}

/// What's been read of each file, as when it changed and how big it was, with what's in it that could be a class name.
type Scanned = (f64, u64, BTreeSet<String>);

thread_local! {
    static SCANNED: RefCell<std::collections::HashMap<PathBuf, Scanned>> = RefCell::new(std::collections::HashMap::new());
}

/// When a file last changed, in milliseconds, and how big it is.
fn stamp(path: &Path) -> Option<(f64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let time = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs_f64() * 1000.0;
    Some((time, meta.len()))
}

/// Everything in these files that could be a class name. A file that hasn't changed since it was last read isn't read again, and
/// one that isn't text has none.
fn candidates_of(files: &[PathBuf]) -> BTreeSet<String> {
    let mut all = BTreeSet::new();
    SCANNED.with(|scanned| {
        let mut scanned = scanned.borrow_mut();
        for file in files {
            let Some((time, size)) = stamp(file).or_else(|| packed_stamp(file)) else {
                continue;
            };
            let fresh = scanned.get(file).is_some_and(|(was, length, _)| *was == time && *length == size);
            if !fresh {
                let mut found = BTreeSet::new();
                if let Ok(text) = read_text(file) {
                    candidates_in(&text, &mut found);
                }
                scanned.insert(file.clone(), (time, size, found));
            }
            if let Some((_, _, found)) = scanned.get(file) {
                all.extend(found.iter().cloned());
            }
        }
    });
    all
}

/// A file's text, from inside the program if it's packed, and otherwise from the disk, if it's text at all.
fn read_text(file: &Path) -> Result<String, ()> {
    let bytes = rsui_js::read_packed_file(file).map_or_else(|| std::fs::read(file).map_err(|_| ()), Ok)?;
    String::from_utf8(bytes).map_err(|_| ())
}

fn packed_stamp(file: &Path) -> Option<(f64, u64)> {
    rsui_js::read_packed_file(file).map(|bytes| (0.0, bytes.len() as u64))
}

// Where else to look, as `@source` says

/// Whether a path, from `/` on, fits a pattern with `*`, `**`, `?` and `{a,b}` in it, as a glob.
fn glob_matches(pattern: &str, path: &str) -> bool {
    if let Some(open) = pattern.find('{') {
        if let Some(close) = pattern[open..].find('}').map(|found| open + found) {
            return pattern[open + 1..close]
                .split(',')
                .any(|choice| glob_matches(&format!("{}{choice}{}", &pattern[..open], &pattern[close + 1..]), path));
        }
    }
    fn parts_match(pattern: &[&str], path: &[&str]) -> bool {
        match pattern.split_first() {
            None => path.is_empty(),
            Some((&"**", rest)) => (0..=path.len()).any(|skip| parts_match(rest, &path[skip..])),
            Some((first, rest)) => path
                .split_first()
                .is_some_and(|(part, others)| part_matches(first.as_bytes(), part.as_bytes()) && parts_match(rest, others)),
        }
    }
    fn part_matches(pattern: &[u8], name: &[u8]) -> bool {
        match pattern.split_first() {
            None => name.is_empty(),
            Some((b'*', rest)) => (0..=name.len()).any(|skip| part_matches(rest, &name[skip..])),
            Some((b'?', rest)) => !name.is_empty() && part_matches(rest, &name[1..]),
            Some((c, rest)) => name.first().is_some_and(|n| n.eq_ignore_ascii_case(c)) && part_matches(rest, &name[1..]),
        }
    }
    let pattern: Vec<&str> = pattern.split('/').filter(|part| !part.is_empty() && *part != ".").collect();
    let path: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    parts_match(&pattern, &path)
}

/// The files that a source `@source` names: a file, everything in a folder, or what fits a pattern, from `base`.
fn files_of_source(base: &Path, pattern: &str, out: &mut Vec<PathBuf>) {
    let globbed = pattern.find(['*', '?', '{']).unwrap_or(pattern.len());
    // The part before anything that's a pattern is a folder to start from.
    let fixed = pattern[..globbed].rfind('/').map_or("", |slash| &pattern[..slash]);
    let start = base.join(fixed);
    let rest = pattern[fixed.len()..].trim_start_matches('/');
    if globbed == pattern.len() {
        let path = base.join(pattern);
        if path.is_file() {
            out.push(path);
            return;
        }
    }
    let mut stack = vec![start.clone()];
    while let Some(folder) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                stack.push(path);
            } else {
                let inside = path.strip_prefix(&start).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                if globbed == pattern.len() || glob_matches(rest, &inside) {
                    out.push(path);
                }
            }
        }
    }
}

/// The files to look in for class names: the app's own code that's been loaded, and what `@source` adds or takes away. What's
/// in a package isn't the app's own, unless `@source` says it's to be looked at.
fn files_to_scan(loaded: &[PathBuf], sources: &serde_json::Value) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = loaded
        .iter()
        .filter(|file| !file.components().any(|part| part.as_os_str().eq_ignore_ascii_case("node_modules")))
        .cloned()
        .collect();
    let mut taken_away = Vec::new();
    for source in sources.as_array().into_iter().flatten() {
        let base = Path::new(source["base"].as_str().unwrap_or("."));
        let pattern = source["pattern"].as_str().unwrap_or("**/*");
        if source["negated"].as_bool() == Some(true) {
            files_of_source(base, pattern, &mut taken_away);
        } else {
            files_of_source(base, pattern, &mut files);
        }
    }
    files.retain(|file| !taken_away.contains(file));
    files.sort();
    files.dedup();
    files
}

fn candidates_fingerprint(candidates: &BTreeSet<String>) -> String {
    let joined = candidates.iter().map(String::as_str).collect::<Vec<_>>().join("\n");
    fingerprint(&[joined.as_bytes()])
}

// What's been made, kept

/// What's at the top of a file of what's been made, as JSON in a comment, which says what it was made from.
const KEPT_START: &str = "/* rsui css build\n";
const KEPT_END: &str = "\n*/\n";

fn kept_file(project: &Path, text: &str) -> PathBuf {
    project
        .join("node_modules")
        .join(".cache")
        .join("rsui")
        .join(format!("css-{}.css", fingerprint(&[text.as_bytes()])))
}

/// What's kept for some CSS, as what it says about itself and what was made.
fn read_kept(file: &Path) -> Option<(serde_json::Value, String)> {
    let text = rsui_js::read_packed_file(file)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .or_else(|| std::fs::read_to_string(file).ok())?;
    let rest = text.strip_prefix(KEPT_START)?;
    let end = rest.find(KEPT_END)?;
    let about = serde_json::from_str(&rest[..end]).ok()?;
    Some((about, rest[end + KEPT_END.len()..].to_owned()))
}

/// Whether the files the tools read are as they were.
fn files_unchanged(about: &serde_json::Value) -> bool {
    about["files"].as_array().is_some_and(|files| {
        files.iter().all(|file| {
            let path = Path::new(file[0].as_str().unwrap_or(""));
            stamp(path).is_some_and(|(time, size)| file[1].as_f64() == Some(time) && file[2].as_u64() == Some(size))
        })
    })
}

/// What CSS comes to, for `cssBuild` to give back: what was made, or what was given if nothing makes anything of it, and what
/// went wrong, if anything did.
struct Built {
    css: String,
    error: Option<String>,
}

/// The folder an app's packages are installed in, which is the nearest one up from the CSS with a `node_modules` in it that isn't
/// itself an installed package: a package's own stylesheet is made with the app's tools, as a bundler makes it.
fn project_of(folder: &Path) -> Option<PathBuf> {
    folder
        .ancestors()
        .filter(|at| !at.components().any(|part| part.as_os_str().eq_ignore_ascii_case("node_modules")))
        .find(|at| at.join("node_modules").is_dir())
        .map(Path::to_owned)
}

fn build(vm: &mut Vm, text: &str, folder: &Path, file: &Path) -> Built {
    let given = || Built { css: text.to_owned(), error: None };
    if rsui_js::running_packed() {
        // What was made when it was packed, which is all a packed app has.
        return folder
            .ancestors()
            .find_map(|at| read_kept(&kept_file(at, text)))
            .map_or_else(given, |(about, css)| Built { css, error: about["error"].as_str().map(str::to_owned) });
    }
    let Some(project) = project_of(folder) else {
        // Nothing's installed, so there's nothing to make anything with.
        return given();
    };
    let kept = kept_file(&project, text);
    let loaded = vm.loaded_files();
    if let Some((about, css)) = read_kept(&kept) {
        let same_place = about["from"].as_str() == Some(&*file.to_string_lossy());
        let classes_same = about["candidates"].is_null()
            || about["candidates"].as_str() == Some(&*candidates_fingerprint(&candidates_of(&files_to_scan(&loaded, &about["sources"]))));
        if same_place && files_unchanged(&about) && classes_same {
            crate::bridge::pack_also(&kept);
            return Built { css, error: about["error"].as_str().map(str::to_owned) };
        }
    }
    let made = run_builder(text, folder, file, &project, &loaded);
    let (about, css, error) = match made {
        // What went wrong isn't kept, so it's tried again next time, when it may have been put right.
        Ok(made) if made["css"].as_str().is_none() => {
            return Built { css: text.to_owned(), error: made["error"].as_str().map(str::to_owned) };
        }
        Ok(made) => {
            let error = made["error"].as_str().map(str::to_owned);
            let css = made["css"].as_str().map_or_else(|| text.to_owned(), str::to_owned);
            let candidates = if made["built"].as_bool() == Some(true) {
                serde_json::Value::from(candidates_fingerprint(&candidates_of(&files_to_scan(&loaded, &made["sources"]))))
            } else {
                serde_json::Value::Null
            };
            let files: Vec<serde_json::Value> = made["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|one| {
                    let path = one.as_str()?;
                    let (time, size) = stamp(Path::new(path))?;
                    Some(serde_json::json!([path, time, size]))
                })
                .collect();
            let missing = made["missing"].as_array().cloned().unwrap_or_default();
            let error = error.or_else(|| {
                (!missing.is_empty()).then(|| {
                    let names: Vec<&str> = missing.iter().filter_map(serde_json::Value::as_str).collect();
                    format!("{} isn't installed, so what it has is left out", names.join(", "))
                })
            });
            let about = serde_json::json!({
                "from": file.to_string_lossy(),
                "files": files,
                "sources": made["sources"],
                "candidates": candidates,
                "error": error,
            });
            (about, css, error)
        }
        Err(error) => return Built { css: text.to_owned(), error: Some(error) },
    };
    let written = kept
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&kept, format!("{KEPT_START}{about}{KEPT_END}{css}")));
    if written.is_ok() {
        crate::bridge::pack_also(&kept);
    }
    Built { css, error }
}

thread_local! {
    /// The app's code, for `__rsuiScan` on the builder's thread.
    static TO_SCAN: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

/// `__rsuiScan(sources)`, in the builder's engine: everything that could be a class name in the app's code and in what the
/// sources add.
fn host_scan(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let sources = args.get(vm, 0);
    let sources: serde_json::Value = serde_json::from_str(&text_of(vm, sources)).unwrap_or_default();
    let loaded = TO_SCAN.with(|files| files.borrow().clone());
    let found = candidates_of(&files_to_scan(&loaded, &sources));
    let values = found.iter().map(|one| vm.new_string(one)).collect();
    Ok(vm.new_array(values))
}

/// `__rsuiResolveStylesheet(name, folder)`, in the builder's engine: the file `@import` means, found as `require` finds a module,
/// with a package's stylesheet being what its `exports` say for `style`, as bundlers find it. It's `null` if there's none.
fn host_resolve_stylesheet(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (name, folder) = (args.get(vm, 0), args.get(vm, 1));
    let name = text_of(vm, name).into_owned();
    let folder = PathBuf::from(text_of(vm, folder).into_owned());
    Ok(match vm.resolve_with_conditions(&folder, &name, &["style"]) {
        Some(found) => vm.new_string(&found.to_string_lossy()),
        None => Value::NULL,
    })
}

/// Runs `css_build.js` with the app's tools, in an engine of its own on a thread of its own, and gives back what it says.
fn run_builder(text: &str, folder: &Path, file: &Path, project: &Path, loaded: &[PathBuf]) -> Result<serde_json::Value, String> {
    let builder = project.join("node_modules").join(".cache").join("rsui").join("build.js");
    if std::fs::read_to_string(&builder).ok().as_deref() != Some(BUILDER) {
        builder
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&builder, BUILDER))
            .map_err(|error| format!("writing {}: {error}", builder.display()))?;
    }
    // The packages the app itself depends on, some of which have what a tool needs installed for them, as pnpm keeps it.
    let packages: Vec<String> = std::fs::read_to_string(project.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .map(|json| {
            ["dependencies", "devDependencies"]
                .iter()
                .filter_map(|part| json[part].as_object())
                .flat_map(|names| names.keys().cloned())
                .collect()
        })
        .unwrap_or_default();
    let request = serde_json::json!({
        "css": text,
        "base": folder.to_string_lossy(),
        "from": file.to_string_lossy(),
        "project": project.to_string_lossy(),
        "packages": packages,
    })
    .to_string();
    let loaded = loaded.to_vec();
    let thread = std::thread::Builder::new()
        .name("rsui css build".to_owned())
        .stack_size(crate::STACK)
        .spawn(move || -> Result<String, String> {
            TO_SCAN.with(|files| *files.borrow_mut() = loaded);
            let mut engine = rsui_js::Engine::new();
            engine.set_stack_limit(crate::STACK / 8 * 7);
            // What the tools print about themselves isn't the app's to show.
            engine.set_printer(|_| {});
            let vm = engine.vm();
            let scan = vm.host_function("__rsuiScan", 1, host_scan);
            vm.set_global("__rsuiScan", scan);
            let resolve = vm.host_function("__rsuiResolveStylesheet", 2, host_resolve_stylesheet);
            vm.set_global("__rsuiResolveStylesheet", resolve);
            let request = vm.new_string(&request);
            vm.set_global("__rsuiCssBuildRequest", request);
            engine
                .run_file(&builder)
                .and_then(|_| engine.run_until_idle())
                .map_err(|error| error.message)?;
            let vm = engine.vm();
            let result = vm.get_global("__rsuiCssBuildResult").unwrap_or(Value::UNDEFINED);
            vm.plain_text(result).ok_or_else(|| "the app's CSS tools didn't finish".to_owned())
        })
        .map_err(|error| error.to_string())?;
    let said = thread
        .join()
        .unwrap_or_else(|_| Err("the app's CSS tools stopped part way".to_owned()))?;
    serde_json::from_str(&said).map_err(|error| error.to_string())
}

/// `cssBuild(text, from)`: `{ css, error }`, for CSS that's in the file or folder `from`.
fn host_build(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (text, from) = (args.get(vm, 0), args.get(vm, 1));
    let text = text_of(vm, text).into_owned();
    let from = PathBuf::from(text_of(vm, from).into_owned());
    let (folder, file) = if from.is_dir() {
        (from.clone(), from.join("index.css"))
    } else {
        (from.parent().map_or_else(|| PathBuf::from("."), Path::to_owned), from.clone())
    };
    let built = build(vm, &text, &folder, &file);
    let out = vm.new_object();
    let css = vm.new_string(&built.css);
    let css_name = vm.atom("css");
    vm.set_property(out, css_name, css)?;
    if let Some(error) = built.error {
        let error = vm.new_string(&error);
        let error_name = vm.atom("error");
        vm.set_property(out, error_name, error)?;
    }
    Ok(out)
}

/// How many of the app's files have been loaded, for knowing when there's more of its code to look at.
fn host_loaded_count(vm: &mut Vm, _args: Args) -> Result<Value, Value> {
    Ok(Value::number(vm.loaded_file_count() as f64))
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 2] = [("cssBuild", host_build), ("cssLoadedCount", host_loaded_count)];

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<String> {
        let mut out = BTreeSet::new();
        candidates_in(text, &mut out);
        out.into_iter().collect()
    }

    #[test]
    fn class_names_are_found_wherever_code_writes_them() {
        let all = found(r#"<div className="flex min-h-svh gap-1.5" /> cn("bg-primary/90", ok && 'hover:bg-accent')"#);
        for one in ["flex", "min-h-svh", "gap-1.5", "bg-primary/90", "hover:bg-accent"] {
            assert!(all.contains(&one.to_owned()), "{one} in {all:?}");
        }
    }

    #[test]
    fn arbitrary_values_keep_what_is_inside_their_brackets() {
        let all = found(r#""[&_svg:not([class*='size-'])]:size-4 has-[>svg]:px-3 bg-(--brand) w-[calc(100%-2rem)]""#);
        for one in ["[&_svg:not([class*='size-'])]:size-4", "has-[>svg]:px-3", "bg-(--brand)", "w-[calc(100%-2rem)]", "--brand"] {
            assert!(all.contains(&one.to_owned()), "{one} in {all:?}");
        }
    }

    #[test]
    fn variables_are_found_in_styles() {
        assert!(found("style={{ color: 'var(--color-red-500)' }}").contains(&"--color-red-500".to_owned()));
    }

    #[test]
    fn globs_match_as_patterns_do() {
        assert!(glob_matches("**/*.{js,tsx}", "src/a/b.tsx"));
        assert!(glob_matches("src/*.ts", "src/x.ts"));
        assert!(!glob_matches("src/*.ts", "src/a/x.ts"));
        assert!(glob_matches("**/*", "a"));
    }

    #[test]
    fn a_fingerprint_is_the_same_everywhere() {
        assert_eq!(fingerprint(&[b"a"]), fingerprint(&[b"a"]));
        assert_ne!(fingerprint(&[b"ab", b""]), fingerprint(&[b"a", b"b"]));
    }
}
