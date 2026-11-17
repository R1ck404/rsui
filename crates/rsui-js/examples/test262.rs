//! Runs tests from the official Test262 suite: `test262 <suite folder> <list of tests>`.
//!
//! It writes a line as each test starts and another when it's done, so whatever's driving it can tell if one hung or
//! took the whole process down, and carry on from the one after.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

const STACK: usize = 256 * 1024 * 1024;

#[derive(Default)]
struct Meta {
    includes: Vec<String>,
    only_strict: bool,
    no_strict: bool,
    raw: bool,
    is_module: bool,
    is_async: bool,
    negative_phase: Option<String>,
    negative_type: Option<String>,
    features: Vec<String>,
}

/// Reads the part of a test's header this needs. It's YAML, but only ever in a few simple forms.
fn read_meta(source: &str) -> Meta {
    let mut meta = Meta::default();
    let Some(start) = source.find("/*---") else {
        return meta;
    };
    let Some(end) = source[start..].find("---*/") else {
        return meta;
    };
    let header = &source[start + 5..start + end];
    let mut section = "";
    for line in header.lines() {
        let trimmed = line.trim();
        let list_of = |text: &str| -> Vec<String> {
            text.trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .map(|item| item.trim().to_owned())
                .filter(|item| !item.is_empty())
                .collect()
        };
        if let Some(rest) = trimmed.strip_prefix("features:") {
            section = "features";
            meta.features.extend(list_of(rest.trim()));
        } else if let (Some(item), "features") = (trimmed.strip_prefix("- "), section) {
            meta.features.push(item.trim().to_owned());
        } else if let Some(rest) = trimmed.strip_prefix("includes:") {
            section = "includes";
            meta.includes.extend(list_of(rest.trim()));
        } else if let Some(rest) = trimmed.strip_prefix("flags:") {
            section = "flags";
            for flag in list_of(rest.trim()) {
                match flag.as_str() {
                    "onlyStrict" => meta.only_strict = true,
                    "noStrict" => meta.no_strict = true,
                    "raw" => meta.raw = true,
                    "module" => meta.is_module = true,
                    "async" => meta.is_async = true,
                    _ => {}
                }
            }
        } else if trimmed.starts_with("negative:") {
            section = "negative";
        } else if let (Some(rest), "negative") = (trimmed.strip_prefix("phase:"), section) {
            meta.negative_phase = Some(rest.trim().to_owned());
        } else if let (Some(rest), "negative") = (trimmed.strip_prefix("type:"), section) {
            meta.negative_type = Some(rest.trim().to_owned());
        } else if let (Some(item), "includes") = (trimmed.strip_prefix("- "), section) {
            meta.includes.push(item.trim().to_owned());
        } else if !line.starts_with(' ') && trimmed.contains(':') {
            section = "";
        }
    }
    meta
}

/// Proposals that haven't made it into the language, and aren't in Node 24 without a flag.
const NOT_YET_IN_THE_LANGUAGE: &[&str] = &[
    "Temporal",
    "ShadowRealm",
    "uint8array-base64",
    "decorators",
    "source-phase-imports",
    "import-defer",
    "nonextensible-applies-to-private",
    "joint-iteration",
    "immutable-arraybuffer",
    "upsert",
    "tail-call-optimization",
    "canonical-tz",
    "Intl.Locale-info",
    "iterator-sequencing",
    "iterator-chunking",
    "iterator-includes",
    "await-dictionary",
    "Math.sumPrecise",
    "legacy-regexp",
];

const PRELUDE: &str = r#"
var print = function () { console.log.apply(console, arguments); };
var $262 = {
    global: globalThis,
    evalScript: function (source) { return (0, eval)(source); },
    gc: __gc,
    clearKeptObjects: __clearKeptObjects,
    IsHTMLDDA: undefined,
    createRealm: function () { throw new Test262Error('no realms here'); },
    detachArrayBuffer: function (buffer) { buffer.transfer(); },
    agent: {},
};
"#;

struct Harness {
    root: PathBuf,
    files: HashMap<String, String>,
}

impl Harness {
    fn file(&mut self, name: &str) -> String {
        if let Some(found) = self.files.get(name) {
            return found.clone();
        }
        let text =
            std::fs::read_to_string(self.root.join("harness").join(name)).unwrap_or_default();
        self.files.insert(name.to_owned(), text.clone());
        text
    }
}

/// How one run of a test went: nothing wrong, or what was. A module test is run from its own file, after `source`, which
/// is then only the harness, so that what it imports is found next to it.
fn run_once(source: &str, file: &str, meta: &Meta, module: Option<&Path>) -> Result<(), String> {
    let output = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let sink = output.clone();
    let mut engine = rsui_js::Engine::new();
    engine.set_stack_limit(STACK / 8 * 7);
    // Tests are scripts, which are compiled whole unless this asks for every function to wait until it's called.
    if std::env::var_os("RSUI_LAZY_COMPILE").is_some() {
        engine.compile_lazily(true);
    }
    engine.set_printer(move |text| sink.borrow_mut().push_str(text));
    // What `$262.gc` and `$262.clearKeptObjects` are, for tests of `WeakRef` and `FinalizationRegistry`.
    let vm = engine.vm();
    let gc = vm.host_function("gc", 0, |vm, _| {
        vm.collect_garbage_now();
        Ok(rsui_js::Value::UNDEFINED)
    });
    vm.set_global("__gc", gc);
    let clear = vm.host_function("clearKeptObjects", 0, |vm, _| {
        vm.clear_kept_objects();
        Ok(rsui_js::Value::UNDEFINED)
    });
    vm.set_global("__clearKeptObjects", clear);
    // Plenty of tests leave these behind, and the language has nothing against it.
    engine.ignore_unhandled_rejections();
    // As the file that it is, for what it imports to be found next to it.
    let ran = match module {
        Some(path) => engine
            .eval(source, "harness.js")
            .and_then(|_| engine.run_module(path)),
        None => engine.eval(source, file),
    };
    let result = ran.and_then(|_| {
        // Timers aren't part of the language, but a few tests use them through a helper.
        engine.run_jobs()?;
        let mut turns = 0;
        while let Some(wait) = engine.run_timers()? {
            turns += 1;
            if turns > 200 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs_f64(
                (wait / 1000.0).min(0.01),
            ));
        }
        Ok(())
    });
    if let Some(wanted) = &meta.negative_type {
        return match result {
            Ok(()) => Err(format!("should have thrown {wanted}")),
            Err(error) => {
                let is_right_kind = error.message.starts_with(wanted.as_str())
                    || error.message.contains(&format!("{wanted}:"))
                    || error.message.contains(&format!("{wanted} "));
                if is_right_kind {
                    Ok(())
                } else {
                    Err(format!(
                        "threw something else: {}",
                        first_line(&error.message)
                    ))
                }
            }
        };
    }
    if let Err(error) = result {
        return Err(first_line(&error.message));
    }
    if meta.is_async {
        let text = output.borrow();
        if text.contains("Test262:AsyncTestFailure") {
            return Err(first_line(&text));
        }
        if !text.contains("Test262:AsyncTestComplete") {
            return Err("it never finished".to_owned());
        }
    }
    Ok(())
}

fn first_line(text: &str) -> String {
    // An error from the suite's own `Test262Error` shows over a few lines, with what went wrong on the second.
    let joined: Vec<&str> = text.lines().map(str::trim).take(3).collect();
    joined.join(" ").chars().take(200).collect()
}

fn run_test(harness: &mut Harness, name: &str) -> Result<&'static str, String> {
    let path = harness.root.join(name.replace('/', "\\"));
    let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
    let source = String::from_utf8_lossy(&bytes).into_owned();
    let meta = read_meta(&source);
    // What isn't in the language yet, or is only in Node behind a flag, doesn't count either way.
    if meta
        .features
        .iter()
        .any(|feature| NOT_YET_IN_THE_LANGUAGE.contains(&feature.as_str()))
        || name.contains("/Temporal/")
    {
        return Ok("future");
    }
    let mut prefix = String::new();
    if !meta.raw {
        prefix.push_str(&harness.file("assert.js"));
        prefix.push('\n');
        prefix.push_str(&harness.file("sta.js"));
        prefix.push('\n');
        prefix.push_str(PRELUDE);
        if meta.is_async {
            prefix.push_str(&harness.file("doneprintHandle.js"));
            prefix.push('\n');
        }
        for include in &meta.includes {
            prefix.push_str(&harness.file(include));
            prefix.push('\n');
        }
    }
    let file = path.to_string_lossy().into_owned();
    // A module is strict anyway, and runs once.
    if meta.is_module {
        return run_once(&prefix, &file, &meta, Some(&path))
            .map(|()| "pass")
            .map_err(|error| format!("module: {error}"));
    }
    let run_sloppy = !meta.only_strict;
    let run_strict = !meta.no_strict && !meta.raw;
    if run_sloppy {
        let joined = format!("{prefix}{source}");
        run_once(&joined, &file, &meta, None).map_err(|error| format!("sloppy: {error}"))?;
    }
    if run_strict {
        let joined = format!("\"use strict\";\n{prefix}{source}");
        run_once(&joined, &file, &meta, None).map_err(|error| format!("strict: {error}"))?;
    }
    Ok("pass")
}

fn run(root: &Path, list: &Path, first: usize) {
    std::panic::set_hook(Box::new(|_| {}));
    let mut harness = Harness {
        root: root.to_owned(),
        files: HashMap::new(),
    };
    let names = std::fs::read_to_string(list).unwrap_or_default();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for (index, name) in names.lines().enumerate().skip(first) {
        let _ = writeln!(out, "S {index}");
        let _ = out.flush();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_test(&mut harness, name)
        }));
        let line = match outcome {
            Ok(Ok(kind)) => format!("{kind} {index}"),
            Ok(Err(message)) => format!("fail {index} {}", message.replace('\n', " ")),
            Err(_) => format!("fail {index} the engine panicked"),
        };
        let _ = writeln!(out, "{line}");
    }
    let _ = out.flush();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(&args[1]);
    let list = PathBuf::from(&args[2]);
    let first = args.get(3).and_then(|text| text.parse().ok()).unwrap_or(0);
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || run(&root, &list, first))
        .expect("the system has room for a thread")
        .join()
        .ok();
}
