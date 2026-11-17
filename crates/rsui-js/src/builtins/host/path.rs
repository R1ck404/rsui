//! Node's `path`, following Node's own steps for both kinds (`path_posix.rs` and `path_win32.rs`), on the text's
//! UTF-16 units, so what comes out is what Node gives, including for anything odd in it. Windows' paths have kinds of
//! root that only those steps get right, like `C:foo`, which is in the folder that drive was last in. Anything that
//! isn't text is given to the JavaScript this used to be (`node/path_js.js`), which does with it what it always did.

use crate::object::PLAIN;
use crate::text::Text;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

use super::path_posix as posix;
use super::path_win32 as win32;

// The module itself: each function a native that knows which kind it is from its data, `true` for Windows'.

fn units(vm: &Vm, value: Value) -> Vec<u16> {
    let text: &Text = vm.text_of(value);
    match (text.ascii(), text.wide()) {
        (Some(ascii), _) => ascii.iter().map(|byte| u16::from(*byte)).collect(),
        (_, wide) => wide.unwrap_or(&[]).to_vec(),
    }
}

/// The first `count` arguments (or all of them, for `None`), if they're all text and there are that many.
fn texts(vm: &Vm, args: &Args, count: Option<usize>) -> Option<Vec<Vec<u16>>> {
    let count = count.unwrap_or(args.count);
    (0..count)
        .map(|index| {
            let value = vm.arg(args, index);
            value.is_string().then(|| units(vm, value))
        })
        .collect()
}

/// Whether it's Windows' kind of path.
fn windows(vm: &Vm, args: &Args) -> bool {
    vm.native_data_of(args) == Value::TRUE
}

fn give(vm: &mut Vm, units: &[u16]) -> Value {
    vm.new_text(Text::from_units(units))
}

/// Gives what isn't text to the JavaScript this used to be, which does with it what it always did.
fn as_before(vm: &mut Vm, args: &Args, name: &str) -> Run<Value> {
    let module = vm
        .load_builtin_module("internal/path-js")?
        .expect("it comes with the engine");
    let exports = vm.exports_of(module)?;
    let flavor = if windows(vm, args) { "win32" } else { "posix" };
    let flavor = vm.get_named(exports, flavor)?;
    let function = vm.get_named(flavor, name)?;
    let given: Vec<Value> = (0..args.count).map(|index| vm.arg(args, index)).collect();
    vm.call(function, flavor, &given)
}

/// What `process.cwd()` says, as Node asks for it each time it's needed.
fn cwd(vm: &mut Vm) -> Run<Vec<u16>> {
    let process = vm.get_named(Value::object(vm.global), "process")?;
    let function = vm.get_named(process, "cwd")?;
    let found = vm.call(function, process, &[])?;
    vm.to_units(found)
}

/// What `process.env` has under a name, if it's text.
fn env(vm: &mut Vm, name: &[u16]) -> Run<Vec<u16>> {
    let process = vm.get_named(Value::object(vm.global), "process")?;
    let variables = vm.get_named(process, "env")?;
    if !variables.is_object() {
        return Ok(Vec::new());
    }
    let key = vm.intern_units(name);
    let found = vm.get(variables, key)?;
    Ok(if found.is_string() { units(vm, found) } else { Vec::new() })
}

/// Runs something that may need the working folder (`None`) or an environment variable, which are only asked for if
/// it does. See [`win32::Ask`].
fn with_ask(vm: &mut Vm, run: impl FnOnce(&mut win32::Ask<'_>) -> Vec<u16>) -> Run<Vec<u16>> {
    let mut failed = None;
    let mut found = None;
    let out = run(&mut |name: Option<&[u16]>| {
        if failed.is_some() {
            return Vec::new();
        }
        match name {
            None => {
                if found.is_none() {
                    match cwd(vm) {
                        Ok(units) => found = Some(units),
                        Err(error) => failed = Some(error),
                    }
                }
                found.clone().unwrap_or_default()
            }
            Some(name) => match env(vm, name) {
                Ok(units) => units,
                Err(error) => {
                    failed = Some(error);
                    Vec::new()
                }
            },
        }
    });
    match failed {
        Some(error) => Err(error),
        None => Ok(out),
    }
}

fn is_absolute(vm: &mut Vm, args: Args) -> Run<Value> {
    match texts(vm, &args, Some(1)) {
        Some(paths) if windows(vm, &args) => Ok(Value::bool(win32::is_absolute(&paths[0]))),
        Some(paths) => Ok(Value::bool(posix::is_absolute(&paths[0]))),
        None => as_before(vm, &args, "isAbsolute"),
    }
}

fn normalize(vm: &mut Vm, args: Args) -> Run<Value> {
    match texts(vm, &args, Some(1)) {
        Some(paths) => {
            let out = if windows(vm, &args) { win32::normalize(&paths[0]) } else { posix::normalize(&paths[0]) };
            Ok(give(vm, &out))
        }
        None => as_before(vm, &args, "normalize"),
    }
}

fn join(vm: &mut Vm, args: Args) -> Run<Value> {
    match texts(vm, &args, None) {
        Some(paths) => {
            let parts: Vec<&[u16]> = paths.iter().map(Vec::as_slice).collect();
            let out = if windows(vm, &args) { win32::join(&parts) } else { posix::join(&parts) };
            Ok(give(vm, &out))
        }
        None => as_before(vm, &args, "join"),
    }
}

fn resolve(vm: &mut Vm, args: Args) -> Run<Value> {
    match texts(vm, &args, None) {
        Some(paths) => {
            let parts: Vec<&[u16]> = paths.iter().map(Vec::as_slice).collect();
            let out = if windows(vm, &args) {
                with_ask(vm, |ask| win32::resolve(&parts, ask))?
            } else {
                with_ask(vm, |ask| posix::resolve(&parts, &mut || ask(None)))?
            };
            Ok(give(vm, &out))
        }
        None => as_before(vm, &args, "resolve"),
    }
}

fn dirname(vm: &mut Vm, args: Args) -> Run<Value> {
    match texts(vm, &args, Some(1)) {
        Some(paths) => {
            let out = if windows(vm, &args) { win32::dirname(&paths[0]) } else { posix::dirname(&paths[0]) };
            Ok(give(vm, &out))
        }
        None => as_before(vm, &args, "dirname"),
    }
}

fn basename(vm: &mut Vm, args: Args) -> Run<Value> {
    // A suffix that isn't given is the same as an empty one.
    let paths = if vm.arg(&args, 1).is_undefined() {
        texts(vm, &args, Some(1)).map(|mut paths| {
            paths.push(Vec::new());
            paths
        })
    } else {
        texts(vm, &args, Some(2))
    };
    match paths {
        Some(paths) => {
            let (path, suffix) = (&paths[0], Some(paths[1].as_slice()));
            let out = if windows(vm, &args) { win32::basename(path, suffix) } else { posix::basename(path, suffix) };
            let out = out.to_vec();
            Ok(give(vm, &out))
        }
        None => as_before(vm, &args, "basename"),
    }
}

fn extname(vm: &mut Vm, args: Args) -> Run<Value> {
    match texts(vm, &args, Some(1)) {
        Some(paths) => {
            let out = if windows(vm, &args) { win32::extname(&paths[0]) } else { posix::extname(&paths[0]) };
            let out = out.to_vec();
            Ok(give(vm, &out))
        }
        None => as_before(vm, &args, "extname"),
    }
}

fn relative(vm: &mut Vm, args: Args) -> Run<Value> {
    match texts(vm, &args, Some(2)) {
        Some(paths) => {
            let (from, to) = (&paths[0], &paths[1]);
            let out = if windows(vm, &args) {
                with_ask(vm, |ask| win32::relative(from, to, ask))?
            } else {
                with_ask(vm, |ask| posix::relative(from, to, &mut || ask(None)))?
            };
            Ok(give(vm, &out))
        }
        None => as_before(vm, &args, "relative"),
    }
}

fn parse(vm: &mut Vm, args: Args) -> Run<Value> {
    let Some(paths) = texts(vm, &args, Some(1)) else {
        return as_before(vm, &args, "parse");
    };
    let parts = if windows(vm, &args) { win32::parse(&paths[0]) } else { posix::parse(&paths[0]) };
    let out = vm.new_plain();
    for (key, part) in ["root", "dir", "base", "ext", "name"].into_iter().zip(&parts) {
        let value = give(vm, part);
        let atom = vm.intern_str(key);
        vm.define(out.handle(), atom, value, PLAIN);
    }
    Ok(out)
}

/// `format` is left to the JavaScript, as what it does with what isn't text is JavaScript's `+`, and it's rarely used.
fn format(vm: &mut Vm, args: Args) -> Run<Value> {
    as_before(vm, &args, "format")
}

/// On Windows, a whole path in the form that has no limit on its length: `\\?\C:\...`, or `\\?\UNC\server\share\...`.
/// Anything else, and everything elsewhere, is given back as it is.
fn to_namespaced_path(vm: &mut Vm, args: Args) -> Run<Value> {
    let given = vm.arg(&args, 0);
    let path = match texts(vm, &args, Some(1)) {
        Some(paths) if windows(vm, &args) && !paths[0].is_empty() => paths.into_iter().next().unwrap_or_default(),
        _ => return Ok(given),
    };
    let mut namespaced = None;
    with_ask(vm, |ask| {
        namespaced = win32::to_namespaced_path(&path, ask);
        Vec::new()
    })?;
    match namespaced {
        Some(namespaced) => Ok(give(vm, &namespaced)),
        None => Ok(given),
    }
}

/// Each function, by its key, its name (the JavaScript's `isAbsolute` was an arrow function handed to what made the
/// rest, so it had none), and how many arguments it says it takes.
const FUNCTIONS: [(&str, &str, u8, crate::object::NativeFn); 11] = [
    ("isAbsolute", "", 1, is_absolute),
    ("normalize", "normalize", 1, normalize),
    ("join", "join", 0, join),
    ("resolve", "resolve", 0, resolve),
    ("dirname", "dirname", 1, dirname),
    ("basename", "basename", 2, basename),
    ("extname", "extname", 1, extname),
    ("relative", "relative", 2, relative),
    ("parse", "parse", 1, parse),
    ("format", "format", 1, format),
    ("toNamespacedPath", "toNamespacedPath", 1, to_namespaced_path),
];

/// One kind of `path`, as the plain object the JavaScript made, with its properties in the same order.
fn flavor(vm: &mut Vm, windows: bool) -> Value {
    let out = vm.new_plain();
    let (separator, delimiter) = if windows { ("\\", ";") } else { ("/", ":") };
    for (key, text) in [("sep", separator), ("delimiter", delimiter)] {
        let value = vm.new_str(text);
        let atom = vm.intern_str(key);
        vm.define(out.handle(), atom, value, PLAIN);
    }
    for (key, name, length, function) in FUNCTIONS {
        let name = vm.intern_str(name);
        let value = vm.new_native(name, length, function, Value::bool(windows), false);
        let atom = vm.intern_str(key);
        vm.define(out.handle(), atom, value, PLAIN);
    }
    out
}

/// `pathModule(windows)`: what `require('path')` gives, the kind for this system with both kinds on it as `posix` and
/// `win32`.
fn path_module(vm: &mut Vm, args: Args) -> Run<Value> {
    let windows = vm.arg(&args, 0) == Value::TRUE;
    let posix = flavor(vm, false);
    let win32 = flavor(vm, true);
    let current = if windows { win32 } else { posix };
    let out = vm.new_plain();
    for key in ["sep", "delimiter"].into_iter().chain(FUNCTIONS.map(|(key, ..)| key)) {
        let atom = vm.intern_str(key);
        let value = vm.get(current, atom)?;
        vm.define(out.handle(), atom, value, PLAIN);
    }
    for (key, value) in [("posix", posix), ("win32", win32)] {
        let atom = vm.intern_str(key);
        vm.define(out.handle(), atom, value, PLAIN);
    }
    Ok(out)
}

pub(super) const NATIVES: [(&str, u8, crate::object::NativeFn); 1] = [("pathModule", 1, path_module)];
