//! Loading files: ES modules, CommonJS, JSON, and finding packages in `node_modules` as Node does.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use foldhash::fast::FixedState;

use crate::compiler::{KeptAst, KeptSource};
use crate::generator::{Resume, Suspended};
use crate::lexer::SyntaxError;
use crate::object::*;
use crate::parser;
use crate::source_text::SourceText;
use crate::text::{self, Atom};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

const EXTENSIONS: [&str; 7] = ["ts", "tsx", "js", "jsx", "mjs", "cjs", "json"];

impl crate::Engine {
    /// Configuration bytes the resolver read, needed by relocated source loads.
    pub fn module_configuration_files(&self) -> Vec<PathBuf> {
        let mut files: Vec<_> = self.vm.modules.configuration_files.as_deref().into_iter().flat_map(|files| files.iter()).cloned().collect();
        files.sort(); files
    }
    /// Resolves a build graph edge without executing the requested module.
    /// `None` identifies an engine/host builtin, which needs no packed file.
    pub fn resolve_module_file(
        &mut self,
        from: &Path,
        name: &str,
        is_import: bool,
    ) -> Result<Option<PathBuf>, crate::JsError> {
        self.enter();
        let name = file_url_path(name.to_owned());
        let bare = name.strip_prefix("node:").unwrap_or(&name);
        if self.vm.is_builtin_name(bare) {
            return Ok(None);
        }
        match self
            .vm
            .resolve_file(from.parent().unwrap_or(Path::new(".")), bare, is_import)
        {
            Ok(path) => {
                let path = canonical(&path);
                self.vm.modules.resolved.insert(
                    (
                        from.parent().unwrap_or(Path::new(".")).to_owned(),
                        bare.to_owned(),
                        is_import,
                    ),
                    path.clone(),
                );
                Ok(Some(path))
            }
            Err(error) => self.finish(Err(error)).map(|_| None),
        }
    }
}

#[derive(Default)]
pub(crate) struct Modules {
    /// ES modules started since the modules being loaded were, because something asked for a name a module that only
    /// passes on exports gives from one that was [`Dep::Deferred`]: they're linked and run with the rest. With each, the
    /// module that named it and which of its names it is, to put back as it was if loading fails.
    late: Vec<(u32, u32, usize)>,
    /// Whether the files in each folder are in a package that says they have no side effects, by the folder.
    side_effects: HashMap<PathBuf, std::rc::Rc<SideEffects>, FixedState>,
    configuration_files: Option<Box<HashSet<PathBuf, FixedState>>>,
    /// Each file that's been started, with the `module` object it has.
    loaded: HashMap<PathBuf, Value, FixedState>,
    /// Modules that the program embedding this engine gave it, by the name they're imported by.
    pub(crate) host: Vec<(&'static str, &'static str)>,
    /// The first file that was run.
    pub(crate) main: Option<PathBuf>,
    /// Each file that's been started, in the order it was.
    order: Vec<PathBuf>,
    /// How many of `order` are files that are in `loaded`, once that's been counted since either last changed: see
    /// [`loaded_file_count`](Self::loaded_file_count).
    counted: std::cell::Cell<Option<usize>>,
    /// Each module that comes with the engine, or that the program embedding it gave it, that's been started, in the
    /// order it was.
    builtin_order: Vec<String>,
    /// Where each name a module asked for, from each folder, turned out to be, as `import` or as `require` asked for it.
    /// A package is asked for by most of the files of a program, and finding it again each time is most of what loading
    /// them all takes.
    resolved: HashMap<(PathBuf, String, bool), PathBuf, FixedState>,
    /// What each module that's just been compiled imports and exports, by its proto, until it's loaded.
    pub(crate) infos: HashMap<u32, ModuleInfo, FixedState>,
    /// Each ES module that's been started, by its number.
    records: HashMap<u32, Record, FixedState>,
    /// The number of each ES module, by its file.
    es: HashMap<PathBuf, u32, FixedState>,
    next_record: u32,
    /// CommonJS files that an ES module imports, compiled when the module was, to be run when it's their turn.
    prepared: HashMap<PathBuf, u32, FixedState>,
    /// Each module's `import.meta`, by its file.
    metas: HashMap<String, Value, FixedState>,
    /// What every module's `import.meta.env` is, if what's running the program says, as a bundler's does for an app.
    /// Node has none. See [`crate::Engine::set_import_meta_env`].
    pub(crate) meta_env: Option<Value>,
    /// A file to read as an ES module whatever's in it, as [`crate::Engine::run_module`] asks.
    module_goal: Option<PathBuf>,
    /// Whether the nearest `package.json` above each folder that's been asked about says `"type": "module"`.
    package_types: HashMap<PathBuf, bool, FixedState>,
    /// What the first file threw after it had waited at its top, and its promise, until it's been said.
    main_failure: Option<(Value, Value)>,
    /// Conditions a package's `exports` are read with as well as the usual ones, while something's looked for with them.
    pub(crate) conditions: Vec<String>,
    /// Whether packages are found as a bundler for a browser finds them: with `exports`' `browser` condition, and what a
    /// `package.json`'s `browser` field says to use in place of its own files. See [`crate::Vm::set_browser_packages`].
    pub(crate) browser: bool,
    /// The `paths` (and `baseUrl`) of the `tsconfig.json` or `jsconfig.json` nearest each folder that's been asked about, if
    /// it has any: see [`Vm::aliased_file`].
    aliases: HashMap<PathBuf, Option<std::rc::Rc<Aliases>>, FixedState>,
}

/// What a `tsconfig.json` says names that aren't relative stand for, as TypeScript and bundlers read it: each of its `paths`
/// patterns, with the places it stands for, and the folder they're from (its `baseUrl`, or its own folder).
#[derive(Debug, Default)]
pub(crate) struct Aliases {
    paths: Vec<(String, Vec<String>)>,
    base: PathBuf,
    /// Whether a `baseUrl` was given, so that any name may also be a file under it.
    has_base_url: bool,
}

/// A `tsconfig.json`'s text as JSON: without its comments, or the commas left before a `}` or `]`, which TypeScript allows.
fn strip_jsonc(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    let mut start = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'"' => {
                at += 1;
                while at < bytes.len() && bytes[at] != b'"' {
                    at += if bytes[at] == b'\\' { 2 } else { 1 };
                }
                at += 1;
            }
            b'/' if bytes.get(at + 1) == Some(&b'/') => {
                out.push_str(&text[start..at]);
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
                start = at;
            }
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                out.push_str(&text[start..at]);
                at += 2;
                while at + 1 < bytes.len() && !(bytes[at] == b'*' && bytes[at + 1] == b'/') {
                    at += 1;
                }
                at = (at + 2).min(bytes.len());
                start = at;
            }
            b',' => {
                let next = bytes[at + 1..].iter().find(|b| !b.is_ascii_whitespace());
                if matches!(next, Some(b'}' | b']')) {
                    out.push_str(&text[start..at]);
                    start = at + 1;
                }
                at += 1;
            }
            _ => at += 1,
        }
    }
    out.push_str(&text[start.min(text.len())..]);
    out
}

/// What a pattern of `paths` makes of a name: what its `*` stands for (empty if it has none), if it matches.
fn alias_match<'a>(pattern: &str, name: &'a str) -> Option<&'a str> {
    match pattern.split_once('*') {
        Some((before, after)) => (name.len() >= before.len() + after.len()
            && name.starts_with(before)
            && name.ends_with(after))
        .then(|| &name[before.len()..name.len() - after.len()]),
        None => (pattern == name).then_some(""),
    }
}

/// What a module imports and exports, as the compiler found, for loading it with the modules it imports.
#[derive(Default, Clone, Debug)]
pub(crate) struct ModuleInfo {
    /// Each module it names, each once, in the order it first does.
    pub(crate) requested: Vec<String>,
    /// What each of its imports gives a name to: which of `requested` it's from, and the name it has there, or `None`
    /// for all of it.
    pub(crate) imports: Vec<(usize, Option<String>)>,
    /// Its own variables that it exports, each once, by the numbers `%bind` is given.
    pub(crate) locals: Vec<String>,
    /// Each name it exports, and where that comes from.
    pub(crate) exports: Vec<(String, ExportFrom)>,
    /// What `export * from` gives everything of.
    pub(crate) stars: Vec<usize>,
    /// Whether it waits at its top, with `await`.
    pub(crate) is_async: bool,
    /// Whether all it does is pass on what other modules export (`export { a } from`, `export * from`, and imports it
    /// exports again), with no code of its own. What it passes on from packages that say they have no side effects is
    /// only loaded when something asks for it: see [`Dep::Deferred`].
    pub(crate) only_reexports: bool,
}

impl ModuleInfo {
    /// The number of a module this one names, from where it says so.
    pub(crate) fn request(&mut self, source: &[u16]) -> usize {
        let name = String::from_utf16_lossy(source);
        match self.requested.iter().position(|found| *found == name) {
            Some(at) => at,
            None => {
                self.requested.push(name);
                self.requested.len() - 1
            }
        }
    }
}

/// Where a name a module exports comes from.
#[derive(Clone, Debug)]
pub(crate) enum ExportFrom {
    /// One of its own variables, by its number in [`ModuleInfo::locals`].
    Local(usize),
    /// A name in a module it names, or all of that module, by its number in [`ModuleInfo::requested`].
    Import(usize, Option<String>),
}

/// An ES module, and how far it's got.
struct Record {
    path: PathBuf,
    info: ModuleInfo,
    /// Its code, stopped between its steps, until it's finished.
    generator: Value,
    /// What it exports, as everything that imports it sees it: a getter for each name, reading its cell.
    namespace: Value,
    /// What the namespace has its names in: a getter for each. See [`Vm::new_namespace`].
    target: Value,
    /// A cell holding `namespace`, for each `import * as` of it to be the same.
    namespace_cell: Value,
    /// What `require` gives for it when it has a default export: its names and `__esModule`, as Node has it.
    required: Value,
    /// The cell of each of its own exported variables, as it handed them over.
    cells: Vec<Value>,
    /// What each module it names turned out to be.
    deps: Vec<Dep>,
    state: State,
    /// That it's finished, once it has to be waited for.
    promise: Value,
    /// How many of the modules it imports that have to be waited for are still going.
    waiting: usize,
    /// What it threw, if it did, which is thrown again for whatever imports it later.
    error: Value,
    /// Cells for what's imported from a module that isn't an ES module, which can't be known before that has run: by which
    /// module and what name, or `None` for all of it.
    filled: Vec<(usize, Option<String>, Value)>,
    /// Whether some of what it names may be [`Dep::Deferred`], and its namespace not made yet, until it's needed.
    partial: bool,
}

#[derive(Clone)]
enum Dep {
    /// A module that a module that only passes on exports names, which hasn't been looked for yet, as nothing has asked
    /// for what it gives: as a bundler leaves out what nothing uses from a package that says it has no side effects
    /// (`"sideEffects": false` in its `package.json`). Importing one name from a package whose main file passes on
    /// thousands of modules loads only the module that name is from. Its namespace (`import * as`, `import()`,
    /// `require`) needs all of them, so that loads the rest.
    Deferred,
    Es(u32),
    /// A module that comes with the engine, which is run as soon as it's asked for. Its `module` object.
    Builtin(Value),
    /// A CommonJS file, or JSON, or anything else that isn't an ES module, which runs when it's its turn.
    File(PathBuf),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    /// Its first step has handed over its cells.
    Started,
    /// Its second has taken what it imports.
    Linked,
    /// Its code is running, or what it imports is.
    Running,
    /// It's waiting, for itself or for what it imports.
    Waiting,
    Done,
    Failed,
}

/// What a package's `package.json` says about its files' side effects.
enum SideEffects {
    /// Nothing, or that they may have them.
    Some,
    /// `"sideEffects": false`: none of them has any.
    None,
    /// Only the files that match these patterns have them, from the package's folder.
    Only(PathBuf, Vec<String>),
}

/// Whether a path matches a pattern of `sideEffects`, as bundlers have them: `*` is anything but a slash, `**` any
/// folders, `?` one character, and a pattern without a slash is matched against the file's name.
fn side_effect_pattern_matches(pattern: &str, root: &Path, path: &Path) -> bool {
    let pattern = pattern.strip_prefix("./").unwrap_or(pattern).replace('\\', "/");
    let relative = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
    let subject = if pattern.contains('/') {
        relative.as_str()
    } else {
        relative.rsplit('/').next().unwrap_or_default()
    };
    fn matches(pattern: &[u8], text: &[u8]) -> bool {
        match pattern.split_first() {
            None => text.is_empty(),
            Some((b'*', rest)) if rest.first() == Some(&b'*') => {
                let rest = &rest[1..];
                let rest = rest.strip_prefix(b"/").unwrap_or(rest);
                (0..=text.len()).any(|at| (at == 0 || text[at - 1] == b'/') && matches(rest, &text[at..]))
            }
            Some((b'*', rest)) => {
                (0..=text.len()).take_while(|&at| at == 0 || text[at - 1] != b'/').any(|at| matches(rest, &text[at..]))
            }
            Some((b'?', rest)) => text.first().is_some_and(|&c| c != b'/') && matches(rest, &text[1..]),
            Some((&c, rest)) => text.first() == Some(&c) && matches(rest, &text[1..]),
        }
    }
    matches(pattern.as_bytes(), subject.as_bytes())
}

/// What a name a module exports turned out to be.
enum Found {
    Cell(Value),
    Missing,
    /// Two `export *`s give it, from different places.
    Ambiguous,
}

impl Modules {
    /// What each name an `import` or `require` gave was found to be: by the folder it was asked for from, the name, and
    /// whether it was an `import`. Packing keeps these (codecache.rs).
    pub(crate) fn resolved_names(
        &self,
    ) -> impl Iterator<Item = (&(PathBuf, String, bool), &PathBuf)> {
        self.resolved.iter()
    }

    pub(crate) fn values(&self) -> Vec<Value> {
        let mut values: Vec<Value> = self.loaded.values().copied().collect();
        values.extend(self.metas.values().copied());
        values.extend(self.meta_env);
        if let Some((error, promise)) = self.main_failure {
            values.extend([error, promise]);
        }
        for record in self.records.values() {
            values.extend([
                record.generator,
                record.namespace,
                record.namespace_cell,
                record.target,
                record.required,
                record.promise,
                record.error,
            ]);
            values.extend_from_slice(&record.cells);
            values.extend(record.filled.iter().map(|(_, _, cell)| *cell));
            for dep in &record.deps {
                if let Dep::Builtin(module) = dep {
                    values.push(*module);
                }
            }
        }
        values
    }

    /// The code of each CommonJS file that's been compiled to be run later, which has to be kept until it is.
    pub(crate) fn prepared_protos(&self) -> Vec<u32> {
        self.prepared.values().copied().collect()
    }

    /// The files that have been loaded, in the order they were.
    /// Each module that comes with the engine, or that the program gave it, that's been started, in the order it was.
    pub(crate) fn loaded_builtins(&self) -> Vec<String> {
        self.builtin_order.clone()
    }

    pub(crate) fn loaded_files(&self) -> Vec<PathBuf> {
        self.order
            .iter()
            .filter(|path| !is_built_in(path) && self.loaded.contains_key(*path))
            .cloned()
            .collect()
    }

    /// How many files [`loaded_files`](Self::loaded_files) gives, without making the list, as a page asks this every time it
    /// looks anything up.
    pub(crate) fn loaded_file_count(&self) -> usize {
        if let Some(count) = self.counted.get() {
            return count;
        }
        let count = self
            .order
            .iter()
            .filter(|path| !is_built_in(path) && self.loaded.contains_key(*path))
            .count();
        self.counted.set(Some(count));
        count
    }

    /// What the first file threw after it waited at its top, if it did and that hasn't been said, with its promise.
    pub(crate) fn take_main_failure(&mut self) -> Option<(Value, Value)> {
        self.main_failure.take()
    }

    pub(crate) fn forget_files(&mut self) {
        self.counted.set(None);
        self.loaded.retain(|path, _| is_built_in(path));
        self.order.retain(|path| is_built_in(path));
        self.records.clear();
        self.es.clear();
        self.prepared.clear();
        self.metas.clear();
        self.package_types.clear();
        self.side_effects.clear();
        // What's on the disk may have changed too.
        self.resolved.clear();
        FOLDERS.with(|folders| folders.borrow_mut().clear());
        FULL_FOLDERS.with(|folders| folders.borrow_mut().clear());
    }
}

/// Whether this is what the engine calls a module that came with it, rather than a file.
fn is_built_in(path: &Path) -> bool {
    path.to_string_lossy().starts_with("node:")
}

pub(crate) fn has_module_syntax(program: &crate::ast::Program<'_>) -> bool {
    use crate::ast::Stmt;
    // An `export type` and nothing else still makes a module, as TypeScript has it, though nothing's left of it.
    program.says_module
        || program.function.body.iter().any(|statement| {
            matches!(
                statement,
                Stmt::Import { .. }
                    | Stmt::Export(_)
                    | Stmt::ExportFrom { .. }
                    | Stmt::ExportDefault(_)
            )
        })
}

/// Whether some text has the word `await` in it anywhere, which a module that's only a module for waiting at its top has.
fn mentions_await(source: &str) -> bool {
    let bytes = source.as_bytes();
    let is_name = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'.');
    (0..bytes.len()).any(|at| {
        bytes[at] == b'a'
            && bytes[at..].starts_with(b"await")
            && (at == 0 || !is_name(bytes[at - 1]))
            && !bytes.get(at + 5).is_some_and(|next| is_name(*next))
    })
}

/// Whether some text has the word `import` or `export` in it anywhere, which every module has to.
pub(crate) fn might_be_module(source: &str) -> bool {
    let bytes = source.as_bytes();
    let is_name = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'.');
    let is_word_at = |at: usize, word: &[u8]| {
        bytes[at..].starts_with(word)
            && (at == 0 || !is_name(bytes[at - 1]))
            && !bytes
                .get(at + word.len())
                .is_some_and(|next| is_name(*next))
    };
    (0..bytes.len()).any(|at| {
        (bytes[at] == b'i' && is_word_at(at, b"import"))
            || (bytes[at] == b'e' && is_word_at(at, b"export"))
    })
}

/// What's in a folder: each name in it, by its letters all the same case, as they are to Windows, with the name as it's
/// written, and whether it's a folder, or `None` if it's a link, which has to be asked about.
type Listing = HashMap<String, (std::ffi::OsString, Option<bool>), FixedState>;

thread_local! {
    /// What's in each folder that's been looked in, by its path with its letters all the same case, or `None` if it
    /// couldn't be read. Finding a module is trying a lot of names in a few folders, and one look at each folder answers
    /// them all, rather than asking the disk about each.
    static FOLDERS: std::cell::RefCell<HashMap<String, Option<std::rc::Rc<Listing>>, FixedState>> =
        std::cell::RefCell::default();
    /// Whether to ask the disk about each path, rather than going by what was in its folder when it was looked at.
    static ASK_THE_DISK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Each folder's full path, as [`canonical`] gives it, by the path it was asked about by.
    static FULL_FOLDERS: std::cell::RefCell<HashMap<PathBuf, PathBuf, FixedState>> = std::cell::RefCell::default();
}

/// What's in `folder`, from the first look at it.
fn listing(folder: &Path) -> Option<std::rc::Rc<Listing>> {
    let key = folder.to_string_lossy().to_lowercase();
    FOLDERS.with(|folders| {
        folders
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                let mut listing = Listing::default();
                let entries = match std::fs::read_dir(folder) {
                    Ok(entries) => entries,
                    // Nothing's in a folder that isn't there, which is most of the places a package is looked for.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Some(std::rc::Rc::new(listing));
                    }
                    Err(_) => return None,
                };
                for entry in entries.flatten() {
                    let kind = entry.file_type().ok().filter(|kind| !kind.is_symlink());
                    let name = entry.file_name();
                    listing.insert(
                        name.to_string_lossy().to_lowercase(),
                        (name, kind.map(|kind| kind.is_dir())),
                    );
                }
                Some(std::rc::Rc::new(listing))
            })
            .clone()
    })
}

/// Whether `path` is a file (`Some(false)`) or a folder (`Some(true)`), from a look at what's in the folder it's in, or
/// `None` if it's neither, or if that can't be said that way.
fn listed(path: &Path) -> Option<Option<bool>> {
    let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
        return None;
    };
    // A folder that came inside the program has what came with it, and what isn't in it isn't anywhere.
    if crate::packed::is_packed_folder(folder) {
        return Some(None);
    }
    if ASK_THE_DISK.with(std::cell::Cell::get) {
        return None;
    }
    let listing = listing(folder)?;
    match listing.get(&name.to_string_lossy().to_lowercase()) {
        None => Some(None),
        Some((_, Some(is_folder))) => Some(Some(*is_folder)),
        // A link is whatever it leads to.
        Some((_, None)) => None,
    }
}

fn is_file(path: &Path) -> bool {
    if crate::packed::is_packed_file(path) {
        return true;
    }
    match listed(path) {
        Some(kind) => kind == Some(false),
        None => path.metadata().is_ok_and(|found| found.is_file()),
    }
}

fn is_folder(path: &Path) -> bool {
    if crate::packed::is_packed_folder(path) {
        return true;
    }
    match listed(path) {
        Some(kind) => kind == Some(true),
        None => path.is_dir(),
    }
}

fn with_extensions(path: &Path) -> Option<PathBuf> {
    if is_file(path) {
        return Some(path.to_owned());
    }
    for extension in EXTENSIONS {
        let mut name = path.as_os_str().to_owned();
        name.push(".");
        name.push(extension);
        let candidate = PathBuf::from(name);
        if is_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// The TypeScript file a name for the JavaScript it'll be compiled to means, when there's no such JavaScript: `./App.js`
/// is `App.ts` or `App.tsx`, `.jsx` is `.tsx`, `.mjs` is `.mts` and `.cjs` is `.cts`, as TypeScript and Vite find them.
/// TypeScript only does this from its own files; here it's from any, as the name would find nothing otherwise.
fn typescript_instead(path: &Path) -> Option<PathBuf> {
    let instead: &[&str] = match path.extension()?.to_str()? {
        "js" => &["ts", "tsx"],
        "jsx" => &["tsx"],
        "mjs" => &["mts"],
        "cjs" => &["cts"],
        _ => return None,
    };
    instead
        .iter()
        .map(|extension| path.with_extension(extension))
        .find(|candidate| is_file(candidate))
}

impl Vm {
    /// Compiles source code into a function that takes no parameters.
    pub(crate) fn compile_source(&mut self, source: &str, file: Atom) -> Run<u32> {
        match self.compile_inner(source, file, false) {
            Ok((proto, _)) => Ok(proto),
            Err(error) => Err(self.report_syntax_error(&error, file)),
        }
    }

    fn compile_inner(
        &mut self,
        source: &str,
        file: Atom,
        allow_module: bool,
    ) -> Result<(u32, bool), SyntaxError> {
        let memory = crate::ast::AstMemory::new();
        let program = parser::parse(source, &memory, self.stack_room(), !allow_module)?;
        let is_module = allow_module && has_module_syntax(&program);
        let text = self.add_source(SourceText::Owned(source.into()));
        let compiler = crate::compiler::Compiler::new(self, file).with_source(text);
        let proto = compiler.program(&program, is_module)?;
        Ok((proto, is_module))
    }

    /// Compiles and runs a script. With `lazy`, its functions are compiled when they're first called, and its source
    /// is copied to be kept until then.
    pub(crate) fn run_script_source(&mut self, source: &str, file: Atom, lazy: bool) -> Run<Value> {
        let proto = if lazy {
            self.compile_script_lazily(source, file)?
        } else {
            self.compile_script(source, file)?
        };
        self.run_script(proto)
    }

    /// As [`Vm::compile_script`], with what the parser made kept for the functions that wait to be compiled.
    fn compile_script_lazily(&mut self, source: &str, file: Atom) -> Run<u32> {
        let kept = KeptAst::sharing(KeptSource::Owned(source.to_owned()), self.kept_memory());
        let program = match parser::parse(kept.text(), kept.memory(), self.stack_room(), false) {
            Ok(program) => program,
            Err(error) => return Err(self.report_syntax_error(&error, file)),
        };
        let text = self.add_source(SourceText::Owned(source.into()));
        let compiler = crate::compiler::Compiler::new(self, file)
            .keeping(&kept, true)
            .with_source(text);
        let compiled = compiler.eval_program(&program);
        kept.finished();
        match compiled {
            Ok(proto) => Ok(proto),
            Err(error) => Err(self.report_syntax_error(&error, file)),
        }
    }

    /// Everything the parser made is gone again by the time this has finished, before any of the code runs.
    fn compile_script(&mut self, source: &str, file: Atom) -> Run<u32> {
        let memory = crate::ast::AstMemory::new();
        let program = match parser::parse(source, &memory, self.stack_room(), false) {
            Ok(program) => program,
            Err(error) => return Err(self.report_syntax_error(&error, file)),
        };
        let text = self.add_source(SourceText::Owned(source.into()));
        let compiler = crate::compiler::Compiler::new(self, file).with_source(text);
        match compiler.eval_program(&program) {
            Ok(proto) => Ok(proto),
            Err(error) => Err(self.report_syntax_error(&error, file)),
        }
    }

    /// Compiles source code as the body of a function with these parameters, which the program returns, without
    /// copying it into a function first. See [`parser::parse_file`].
    /// Its functions are compiled when they're first called, if they all are ([`crate::LazyCompile::Always`]).
    /// Its functions' text is what's in `source`, which is in the program, unless `shows_text` is false, as for the
    /// engine's own standard library, whose functions say they're built in.
    pub(crate) fn compile_wrapped(
        &mut self,
        source: &'static str,
        params: &[&'static str],
        file: Atom,
        shows_text: bool,
    ) -> Run<u32> {
        let lazily = self.compiles_lazily(None);
        let kept = if lazily {
            KeptAst::sharing(KeptSource::Static(source), self.kept_memory())
        } else {
            KeptAst::new(KeptSource::Static(source))
        };
        let room = self.stack_room();
        let parsing = crate::startup::span(crate::startup::Phase::Parse, || None);
        let parsed = parser::parse_file(kept.text(), kept.memory(), room, params, false);
        drop(parsing);
        let program = match parsed {
            Ok(parser::FileProgram::Wrapped(program)) => program,
            Ok(_) => unreachable!("only a function's body is read when a module isn't allowed"),
            Err(error) => return Err(self.report_syntax_error(&error, file)),
        };
        let text = if shows_text {
            self.add_source(SourceText::Static(source))
        } else {
            crate::source_text::NO_SOURCE
        };
        let compiled = crate::compiler::Compiler::new(self, file)
            .keeping(&kept, lazily)
            .with_source(text)
            .program(&program, false);
        kept.finished();
        match compiled {
            Ok(proto) => Ok(proto),
            Err(error) => Err(self.report_syntax_error(&error, file)),
        }
    }

    /// Compiles a file that was parsed on another thread, with its functions compiled when they're first called if
    /// `lazily`.
    pub(crate) fn compile_parsed(
        &mut self,
        parsed: crate::ahead::Parsed,
        file: Atom,
        lazily: bool,
    ) -> Run<(u32, bool)> {
        let is_module = parsed.is_module;
        let (kept, program) = parsed.into_kept();
        let name = self.atom_text(file);
        let bundled = name
            .strip_prefix("node:")
            .map(|bare| (bare, node_source(bare)));
        let text = match bundled {
            Some((bare, _)) if !shows_its_text(bare) => crate::source_text::NO_SOURCE,
            Some((_, Some(bundled))) if bundled.len() == kept.text().len() => {
                self.add_source(SourceText::Static(bundled))
            }
            Some(_) => crate::source_text::NO_SOURCE,
            None => self.add_source(SourceText::file(PathBuf::from(name), kept.text())),
        };
        let compiled = crate::compiler::Compiler::new(self, file)
            .keeping(&kept, lazily)
            .with_source(text)
            .program(&program, is_module);
        kept.finished();
        match compiled {
            Ok(proto) => Ok((proto, is_module)),
            Err(error) => Err(self.report_syntax_error(&error, file)),
        }
    }

    /// Whether the functions in the file at `path` (or in a script or bundled module, with none) wait to be compiled
    /// until they're called. See [`crate::LazyCompile`].
    pub(crate) fn compiles_lazily(&self, path: Option<&Path>) -> bool {
        match self.lazy_compile {
            crate::LazyCompile::Never => false,
            crate::LazyCompile::Always => true,
            crate::LazyCompile::NodeModules => path.is_some_and(|path| {
                path.components()
                    .any(|part| part.as_os_str().eq_ignore_ascii_case("node_modules"))
            }),
        }
    }

    pub(crate) fn report_syntax_error(&mut self, error: &SyntaxError, file: Atom) -> Value {
        let file = self.atom_text(file);
        let message = format!("{} ({file}:{})", error.message, error.line);
        self.syntax_error(&message)
    }

    pub(crate) fn resolve_file(
        &mut self,
        from_dir: &Path,
        name: &str,
        is_import: bool,
    ) -> Run<PathBuf> {
        if name.starts_with("./")
            || name.starts_with("../")
            || name == "."
            || name == ".."
            || Path::new(name).is_absolute()
        {
            let path = from_dir.join(name);
            return self.resolve_path(&path, name);
        }
        if name.starts_with('#') {
            return self.package_import(from_dir, name, is_import);
        }
        if let Some(found) = self.aliased_file(from_dir, name)? {
            return Ok(found);
        }
        // A package, looked for in every `node_modules` on the way up.
        let (package, rest) = split_package(name);
        let mut at = Some(from_dir);
        while let Some(dir) = at {
            let root = dir.join("node_modules").join(package);
            if is_folder(&root) {
                let subpath = rest.map_or(".".to_owned(), |rest| format!("./{rest}"));
                let found = if let Some(path) = self.package_export(&root, &subpath, is_import)? {
                    self.resolve_path(&path, name)?
                } else {
                    let path = rest.map_or(root.clone(), |rest| root.join(rest));
                    self.resolve_path(&path, name)?
                };
                return self.browser_file(&root, found);
            }
            at = dir.parent();
        }
        Err(self.module_not_found(name))
    }

    /// What a `#name` means by the `imports` of the nearest `package.json` above the importing file, as Node's subpath imports
    /// (vfile's `#minpath`, @base-ui/react's `#prehydration/...`): the name, or a pattern with a `*`, and its conditions read
    /// as `exports`' are. A target that starts `./` is a file in that package; any other is a package, found from there.
    fn package_import(&mut self, from_dir: &Path, name: &str, is_import: bool) -> Run<PathBuf> {
        let mut at = Some(from_dir);
        while let Some(dir) = at {
            if is_file(&dir.join("package.json")) {
                break;
            }
            at = dir.parent();
        }
        let Some(root) = at.map(Path::to_path_buf) else {
            return Err(self.module_not_found(name));
        };
        let Some(package) = self.read_package_json(&root)? else {
            return Err(self.module_not_found(name));
        };
        let imports = self.get_named(package, "imports")?;
        if !imports.is_object() {
            return Err(self.module_not_found(name));
        }
        let Some((target, wildcard)) = self.subpath_target(imports, name)? else {
            return Err(self.module_not_found(name));
        };
        let Some(found) = self.export_target(target, is_import)? else {
            return Err(self.module_not_found(name));
        };
        let found = match &wildcard {
            Some(part) => found.replace('*', part),
            None => found,
        };
        if found.starts_with("./") || found.starts_with("../") {
            let path = root.join(&found);
            let found = self.resolve_path(&path, name)?;
            return self.browser_file(&root, found);
        }
        if found.starts_with('#') {
            return Err(self.module_not_found(name));
        }
        // Another package, as if this one had imported it by that name.
        self.resolve_file(&root, &found, is_import)
    }

    /// The target an `exports` or `imports` map gives a subpath: the subpath itself, or else the pattern with a `*` whose part
    /// before it is longest, with what the `*` stands for.
    fn subpath_target(&mut self, map: Value, path: &str) -> Run<Option<(Value, Option<String>)>> {
        let atom = self.intern_str(path);
        if self.has_property(map.handle(), atom) {
            return Ok(Some((self.get(map, atom)?, None)));
        }
        let mut best: Option<(usize, Atom, String)> = None;
        for name in self.own_enumerable_names(map.handle()) {
            let pattern = self.atom_text(name);
            let Some((before, after)) = pattern.split_once('*') else {
                continue;
            };
            if path.len() >= before.len() + after.len()
                && path.starts_with(before)
                && path.ends_with(after)
                && best.as_ref().is_none_or(|(had, ..)| before.len() > *had)
            {
                best = Some((
                    before.len(),
                    name,
                    path[before.len()..path.len() - after.len()].to_owned(),
                ));
            }
        }
        match best {
            Some((_, name, part)) => Ok(Some((self.get(map, name)?, Some(part)))),
            None => Ok(None),
        }
    }

    /// The file a name that isn't relative stands for by the `paths` of the app's nearest `tsconfig.json` (or `jsconfig.json`),
    /// as TypeScript, Vite's tsconfig-paths and esbuild read them: `@/components/ui/button` for shadcn/ui's `"@/*": ["./src/*"]`.
    /// The pattern with the longest part before its `*` is used, and each of its places tried in turn; then, if there's a
    /// `baseUrl`, the name as a file under it. `None` sends the name on to `node_modules`, as a bundler does. Files in
    /// `node_modules` aren't the app's, so their names never go this way.
    fn aliased_file(&mut self, from_dir: &Path, name: &str) -> Run<Option<PathBuf>> {
        let Some(aliases) = self.aliases_for(from_dir)? else {
            return Ok(None);
        };
        let mut best: Option<(usize, &str, &Vec<String>)> = None;
        for (pattern, targets) in &aliases.paths {
            if let Some(star) = alias_match(pattern, name) {
                let rank = if pattern.contains('*') {
                    pattern.find('*').unwrap_or(0)
                } else {
                    usize::MAX
                };
                if best.is_none_or(|(had, ..)| rank > had) {
                    best = Some((rank, star, targets));
                }
            }
        }
        if let Some((_, star, targets)) = best {
            for target in targets {
                let path = aliases.base.join(target.replacen('*', star, 1));
                if let Ok(found) = self.resolve_path(&path, name) {
                    return Ok(Some(found));
                }
            }
        }
        if aliases.has_base_url {
            if let Ok(found) = self.resolve_path(&aliases.base.join(name), name) {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    /// The aliases of the `tsconfig.json` or `jsconfig.json` nearest this folder, if it has any.
    fn aliases_for(&mut self, dir: &Path) -> Run<Option<std::rc::Rc<Aliases>>> {
        if let Some(found) = self.modules.aliases.get(dir) {
            return Ok(found.clone());
        }
        let found = if dir
            .components()
            .any(|part| part.as_os_str().eq_ignore_ascii_case("node_modules"))
        {
            None
        } else if let Some(config) = ["tsconfig.json", "jsconfig.json"]
            .iter()
            .map(|file| dir.join(file))
            .find(|path| is_file(path))
        {
            self.read_aliases(&config, 0)?
                .filter(|aliases| !aliases.paths.is_empty() || aliases.has_base_url)
                .map(std::rc::Rc::new)
        } else {
            match dir.parent() {
                Some(parent) => self.aliases_for(parent)?,
                None => None,
            }
        };
        self.modules
            .aliases
            .insert(dir.to_path_buf(), found.clone());
        Ok(found)
    }

    /// The `paths` and `baseUrl` a config file says, or the one it `extends` says, when it's a file of the app's own.
    fn read_aliases(&mut self, path: &Path, depth: u32) -> Run<Option<Aliases>> {
        let Ok(text) = crate::packed::read_file(path) else {
            return Ok(None);
        };
        if crate::source_overlay::is_active() {
            self.modules.configuration_files.get_or_insert_with(Default::default).insert(canonical(path));
        }
        let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
        let json = self.get_named(Value::object(self.global), "JSON")?;
        let parse = self.get_named(json, "parse")?;
        let text = self.new_str(&strip_jsonc(&text));
        // A config that isn't JSON is one TypeScript would complain of; it says nothing here.
        let Ok(config) = self.call(parse, json, &[text]) else {
            return Ok(None);
        };
        if !config.is_object() {
            return Ok(None);
        }
        let extends = self.get_named(config, "extends")?;
        let parent = if extends.is_string() && depth < 8 {
            let said = self.text_of(extends).to_rust();
            if said.starts_with('.') {
                let mut file = dir.join(&said);
                if file.extension().is_none() {
                    file.set_extension("json");
                }
                self.read_aliases(&file, depth + 1)?
            } else {
                None
            }
        } else {
            None
        };
        let options = self.get_named(config, "compilerOptions")?;
        let (base_url, paths) = if options.is_object() {
            let base_url = self.get_named(options, "baseUrl")?;
            let base_url = base_url
                .is_string()
                .then(|| self.text_of(base_url).to_rust());
            let paths = self.get_named(options, "paths")?;
            let paths = if paths.is_object() {
                let mut out = Vec::new();
                for key in self.own_enumerable_names(paths.handle()) {
                    let pattern = self.atom_text(key);
                    let list = self.get(paths, key)?;
                    let mut targets = Vec::new();
                    if list.is_object() {
                        let length = self.get_named(list, "length")?;
                        for index in 0..length.as_number() as u32 {
                            let item = self.get_named(list, &index.to_string())?;
                            if item.is_string() {
                                targets.push(self.text_of(item).to_rust());
                            }
                        }
                    }
                    out.push((pattern, targets));
                }
                Some(out)
            } else {
                None
            };
            (base_url, paths)
        } else {
            (None, None)
        };
        let mut aliases = parent.unwrap_or_default();
        if let Some(base_url) = &base_url {
            aliases.base = dir.join(base_url);
            aliases.has_base_url = true;
        }
        if let Some(paths) = paths {
            aliases.paths = paths;
            // Without a `baseUrl`, `paths` are from the file that says them.
            if base_url.is_none() && !aliases.has_base_url {
                aliases.base = dir.clone();
            }
        }
        Ok(Some(aliases))
    }

    /// What a package's `browser` field says to use in place of this file of its own, as a bundler for a browser uses it, when
    /// packages are found that way. The file itself otherwise.
    fn browser_file(&mut self, root: &Path, found: PathBuf) -> Run<PathBuf> {
        if !self.modules.browser {
            return Ok(found);
        }
        let Some(package) = self.read_package_json(root)? else {
            return Ok(found);
        };
        let browser = self.get_named(package, "browser")?;
        let Ok(inside) = found.strip_prefix(root) else {
            return Ok(found);
        };
        let inside = inside.to_string_lossy().replace('\\', "/");
        let main = self.get_named(package, "main")?;
        let main = main.is_string().then(|| self.text_of(main).to_rust());
        let same = |said: &str| {
            let said = said.trim_start_matches("./");
            said == inside || with_extensions(&root.join(said)).is_some_and(|one| one == found)
        };
        let mapped = if browser.is_string() {
            // One file in place of the package's main one.
            let instead = self.text_of(browser).to_rust();
            (main.as_deref().is_some_and(same) || main.is_none() && inside.starts_with("index."))
                .then_some(instead)
        } else if browser.is_object() {
            let mut out = None;
            for key in self.own_enumerable_names(browser.handle()) {
                let said = self.atom_text(key);
                if said.starts_with('.') && same(&said) {
                    let value = self.get(browser, key)?;
                    if value.is_string() {
                        out = Some(self.text_of(value).to_rust());
                    }
                    break;
                }
            }
            out
        } else {
            None
        };
        Ok(match mapped {
            Some(instead) => {
                with_extensions(&root.join(instead.trim_start_matches("./"))).unwrap_or(found)
            }
            None => found,
        })
    }

    fn module_not_found(&mut self, name: &str) -> Value {
        let error = self.make_error(
            self.realm.error_proto,
            &format!("Cannot find module '{name}'"),
        );
        let code = self.new_str("MODULE_NOT_FOUND");
        let atom = self.intern_str("code");
        self.define(error.handle(), atom, code, PLAIN);
        error
    }

    fn resolve_path(&mut self, path: &Path, name: &str) -> Run<PathBuf> {
        if let Some(found) = with_extensions(path) {
            return Ok(found);
        }
        if let Some(found) = typescript_instead(path) {
            return Ok(found);
        }
        if is_folder(path) {
            if let Some(main) = self.package_main(path)? {
                if let Some(found) = with_extensions(&path.join(&main)) {
                    return Ok(found);
                }
                if let Some(found) = with_extensions(&path.join(&main).join("index")) {
                    return Ok(found);
                }
            }
            if let Some(found) = with_extensions(&path.join("index")) {
                return Ok(found);
            }
        }
        Err(self.module_not_found(name))
    }

    fn read_package_json(&mut self, dir: &Path) -> Run<Option<Value>> {
        let Ok(text) = crate::packed::read_file(&dir.join("package.json")) else {
            return Ok(None);
        };
        if crate::source_overlay::is_active() {
            self.modules.configuration_files.get_or_insert_with(Default::default).insert(canonical(&dir.join("package.json")));
        }
        let json = self.get_named(Value::object(self.global), "JSON")?;
        let parse = self.get_named(json, "parse")?;
        let text = self.new_str(&text);
        Ok(Some(self.call(parse, json, &[text])?))
    }

    fn package_main(&mut self, dir: &Path) -> Run<Option<String>> {
        let Some(package) = self.read_package_json(dir)? else {
            return Ok(None);
        };
        let value = self.get_named(package, "main")?;
        if value.is_string() {
            return Ok(Some(self.text_of(value).to_rust()));
        }
        Ok(None)
    }

    /// What a package's `exports` says this path means, if it has any.
    fn package_export(&mut self, root: &Path, path: &str, is_import: bool) -> Run<Option<PathBuf>> {
        let Some(package) = self.read_package_json(root)? else {
            return Ok(None);
        };
        let exports = self.get_named(package, "exports")?;
        if exports.is_nullish() {
            return Ok(None);
        }
        let mut wildcard = None;
        let target = if exports.is_string() {
            (path == ".").then_some(exports)
        } else {
            let names = self.own_enumerable_names(exports.handle());
            let has_paths = names
                .iter()
                .any(|name| self.atom_text(*name).starts_with('.'));
            if !has_paths {
                // It's a list of conditions, with no paths at all.
                (path == ".").then_some(exports)
            } else {
                // A path with a star stands for a whole lot of paths.
                self.subpath_target(exports, path)?.map(|(target, part)| {
                    wildcard = part;
                    target
                })
            }
        };
        let Some(target) = target else {
            return Ok(None);
        };
        let found = self.export_target(target, is_import)?;
        Ok(found.map(|found| {
            let found = match &wildcard {
                Some(part) => found.replace('*', part),
                None => found,
            };
            root.join(found)
        }))
    }

    /// Goes through a package's conditions in the order it wrote them, taking the first that applies.
    fn export_target(&mut self, target: Value, is_import: bool) -> Run<Option<String>> {
        if target.is_string() {
            return Ok(Some(self.text_of(target).to_rust()));
        }
        if !target.is_object() {
            return Ok(None);
        }
        if let Kind::Array(_) = self.objects[target.index()].kind {
            for item in self.array_items(target) {
                if let Some(found) = self.export_target(item, is_import)? {
                    return Ok(Some(found));
                }
            }
            return Ok(None);
        }
        let names = self.own_enumerable_names(target.handle());
        for name in &names {
            let condition = self.atom_text(*name);
            let applies = match condition.as_str() {
                "default" | "rsui" => true,
                "import" | "module" => is_import,
                "browser" => self.modules.browser,
                "require" => !is_import,
                other => self.modules.conditions.iter().any(|one| one == other),
            };
            if applies {
                let value = self.get(target, *name)?;
                if let Some(found) = self.export_target(value, is_import)? {
                    return Ok(Some(found));
                }
            }
        }
        // Nothing fits, so anything for Node is better than nothing.
        for name in names {
            if self.atom_text(name) == "node" {
                let value = self.get(target, name)?;
                return self.export_target(value, is_import);
            }
        }
        Ok(None)
    }

    fn make_require(&mut self, dir: &Path, native: NativeFn) -> Value {
        let dir = self.new_str(&dir.to_string_lossy());
        let name = self.intern_str("require");
        let require = self.new_native(name, 1, native, dir, false);
        // `require.resolve`, from the same folder.
        let resolve_name = self.intern_str("resolve");
        let resolve = self.new_native(resolve_name, 1, resolve_function, dir, false);
        self.define(require.handle(), resolve_name, resolve, HIDDEN);
        require
    }

    /// Whether `name` is a module that comes with the engine, or that the program gave it, rather than a file.
    fn is_builtin_name(&self, name: &str) -> bool {
        self.modules
            .host
            .iter()
            .any(|(host_name, _)| *host_name == name)
            || node_source(name).is_some()
    }

    /// The modules that come with Node, for packages that use them. Each is only compiled if something asks.
    pub(crate) fn load_builtin_module(&mut self, name: &str) -> Run<Option<Value>> {
        let host = self
            .modules
            .host
            .iter()
            .find(|(host_name, _)| *host_name == name)
            .map(|(_, source)| *source);
        let Some(source) = host.or_else(|| node_source(name)) else {
            return Ok(None);
        };
        let key = PathBuf::from(format!("node:{name}"));
        if let Some(found) = self.modules.loaded.get(&key) {
            return Ok(Some(*found));
        }
        let module = self.new_plain();
        let exports = self.new_plain();
        let exports_atom = self.intern_str("exports");
        self.define(module.handle(), exports_atom, exports, PLAIN);
        self.modules.counted.set(None);
        self.modules.loaded.insert(key, module);
        let file = self.intern_str(&format!("node:{name}"));
        self.modules.builtin_order.push(name.to_owned());
        let compiling = crate::startup::span(crate::startup::Phase::Compile, || {
            Some(format!("node:{name}"))
        });
        // What it compiles to may be in the program, if it's packed (codecache.rs).
        let proto = match self.load_cached_builtin(name, source, file, shows_its_text(name)) {
            Some(proto) => proto,
            None => {
                let lazily = self.compiles_lazily(None);
                let recording = self.start_recording(lazily);
                let compiled = match crate::ahead::take(&builtin_path(name)) {
                    Some(parsed) if !parsed.is_module => self
                        .compile_parsed(parsed, file, lazily)
                        .map(|(proto, _)| proto),
                    _ => self.compile_wrapped(source, &BUILTIN_PARAMS, file, shows_its_text(name)),
                };
                let swapped = if recording {
                    let top = compiled.as_ref().ok().map(|proto| (*proto, false));
                    let named = crate::codecache::Named::Builtin(name.to_owned());
                    self.finish_recording(named, top, Some(source))
                } else {
                    None
                };
                // Checking the cache, what it read back runs in place of what was compiled.
                match swapped {
                    Some((proto, _)) => proto,
                    None => compiled?,
                }
            }
        };
        drop(compiling);
        let _running =
            crate::startup::span(crate::startup::Phase::Run, || Some(format!("node:{name}")));
        let function = self.run_proto(proto, &[])?;
        let require = self.make_require(Path::new("."), require_function);
        let inspect_name = self.intern_str("inspect");
        let inspect = self.new_native(inspect_name, 1, inspect_function, Value::UNDEFINED, false);
        let wants_files = matches!(
            name,
            "fs" | "internal/fetch"
                | "internal/http"
                | "internal/misc"
                | "internal/net"
                | "internal/websocket"
                | "worker_threads"
                | "internal/intl"
                | "child_process"
                | "internal/standard-streams"
                | "zlib"
                | "internal/buffer"
                | "path"
                | "path/posix"
                | "path/win32"
        );
        let files = if host.is_none() && wants_files {
            crate::builtins::host::file_natives(self)
        } else {
            Value::UNDEFINED
        };
        self.call(
            function,
            exports,
            &[exports, require, module, inspect, files],
        )?;
        Ok(Some(module))
    }

    /// Runs a file, or finds it's already been run, and gives its `module` object. An ES module is run with all it
    /// imports, if they haven't been: see [`Vm::load_es_module`].
    pub(crate) fn load_module(&mut self, path: &Path) -> Run<Value> {
        if let Some(found) = self.modules.loaded.get(path).copied() {
            // One that was started as part of loading another may not have been run yet, as when CommonJS that it
            // imports asks for it.
            if let Some(&id) = self.modules.es.get(path) {
                let late_from = self.modules.late.len();
                self.complete_module(id)?;
                let mut started = Vec::new();
                let linked = self.link_started(&mut started, late_from);
                self.modules.late.truncate(late_from);
                linked?;
                self.evaluate_module(id)?;
            }
            return Ok(found);
        }
        let module = self.new_plain();
        let exports = self.new_plain();
        let exports_atom = self.intern_str("exports");
        self.define(module.handle(), exports_atom, exports, PLAIN);
        self.modules.counted.set(None);
        self.modules.loaded.insert(path.to_owned(), module);
        self.modules.order.push(path.to_owned());
        if self.modules.main.is_none() {
            self.modules.main = Some(path.to_owned());
        }
        let result = self.run_file_into(path, module, exports);
        if let Err(error) = result {
            // An ES module that threw as it ran stays, to throw the same again for whatever else imports it, as the
            // language has it. Anything else can be tried again.
            if !self.modules.es.contains_key(path) {
                self.modules.counted.set(None);
                self.modules.loaded.remove(path);
            }
            return Err(error);
        }
        Ok(module)
    }

    fn run_file_into(&mut self, path: &Path, module: Value, exports: Value) -> Run<()> {
        let file_text = path.to_string_lossy().into_owned();
        let dir = path.parent().unwrap_or(Path::new(".")).to_owned();
        if let Some(proto) = self.modules.prepared.remove(path) {
            return self.run_commonjs(proto, &dir, &file_text, module, exports);
        }
        let extension = path.extension().and_then(|extension| extension.to_str());
        if matches!(extension, Some("json" | "css")) {
            let source = match crate::packed::read_file(path) {
                Ok(source) => source,
                Err(error) => {
                    return Err(self.make_error(
                        self.realm.error_proto,
                        &format!("Cannot read {file_text}: {error}"),
                    ));
                }
            };
            let text = self.new_str(&source);
            let value = if extension == Some("json") {
                let json = self.get_named(Value::object(self.global), "JSON")?;
                let parse = self.get_named(json, "parse")?;
                self.call(parse, json, &[text])?
            } else {
                // What a stylesheet is, the host says, with `__rsuiCss(text, file)`, which gives back what it exports, as
                // a bundler does for `import './app.css'`. With nothing to say so, it's its text.
                let hook = self.get_named(Value::object(self.global), "__rsuiCss")?;
                if self.is_callable(hook) {
                    let file = self.new_str(&file_text);
                    self.call(hook, Value::UNDEFINED, &[text, file])?
                } else {
                    text
                }
            };
            let exports_atom = self.intern_str("exports");
            self.define(module.handle(), exports_atom, value, PLAIN);
            return Ok(());
        }
        match self.compile_file(path)? {
            FileCode::CommonJs(proto) => {
                self.run_commonjs(proto, &dir, &file_text, module, exports)
            }
            FileCode::Module(proto) => self.load_es_module(path, proto, module),
        }
    }

    /// Whether the nearest `package.json` above a file says `"type": "module"`. What each folder on the way has is kept,
    /// as most files of a program are in a few folders.
    fn package_says_module(&mut self, path: &Path) -> bool {
        let mut at = path.parent();
        let mut passed = Vec::new();
        let found = loop {
            let Some(dir) = at else {
                break false;
            };
            if let Some(known) = self.modules.package_types.get(dir) {
                break *known;
            }
            passed.push(dir.to_owned());
            // Each folder up to the drive's is asked about one name, so the disk is asked directly: reading all of a
            // folder, as finding modules does, can take milliseconds for one as full as the temporary files'.
            let candidate = dir.join("package.json");
            let there = if crate::packed::running_packed() {
                is_file(&candidate)
            } else {
                std::fs::metadata(&candidate).is_ok_and(|found| found.is_file())
            };
            if there {
                let says = match self.read_package_json(dir) {
                    Ok(Some(json)) if json.is_object() => self
                        .get_named(json, "type")
                        .ok()
                        .filter(|kind| kind.is_string())
                        .is_some_and(|kind| self.text_of(kind).to_rust() == "module"),
                    _ => false,
                };
                break says;
            }
            at = dir.parent();
        };
        for dir in passed {
            self.modules.package_types.insert(dir, found);
        }
        found
    }

    /// Reads and compiles a file, as an ES module if it is one and as CommonJS if not, without running it.
    pub(crate) fn compile_file(&mut self, path: &Path) -> Run<FileCode> {
        let file_text = path.to_string_lossy().into_owned();
        let _time =
            crate::startup::span(crate::startup::Phase::Compile, || Some(file_text.clone()));
        let file = self.intern_str(&file_text);
        let lazily = self.compiles_lazily(Some(path));
        // A file being debugged is read with places to stop in it, so it isn't what was read ahead either.
        let debugged = crate::inspect::debugs(path);
        // What it compiles to may be in the program, if it's packed (codecache.rs), and is written down when packing.
        if !debugged {
            if let Some((proto, is_module)) =
                self.load_cached_file(path, file, self.must_be_module(path), lazily)
            {
                return Ok(FileCode::of(proto, is_module));
            }
        }
        let recording = !debugged && self.start_recording(lazily);
        let compiled = self.compile_file_here(path, &file_text, file, lazily, debugged);
        if recording {
            let top = compiled.as_ref().ok().map(|code| match code {
                FileCode::Module(proto) => (*proto, true),
                FileCode::CommonJs(proto) => (*proto, false),
            });
            // Checking the cache, what it read back runs in place of what was compiled.
            if let Some((proto, is_module)) =
                self.finish_recording(crate::codecache::Named::File(path.to_owned()), top, None)
            {
                return Ok(FileCode::of(proto, is_module));
            }
        }
        compiled
    }

    /// Whether the file at `path` is a module whatever's in it: a `.mjs` file is, as in Node, and `.mts` is TypeScript's
    /// `.mjs`, and so is a main file that's asked to be.
    fn must_be_module(&self, path: &Path) -> bool {
        let extension = path.extension().and_then(|extension| extension.to_str());
        matches!(extension, Some("mjs" | "mts"))
            || self.modules.module_goal.as_deref() == Some(path)
    }

    /// Compiles a file, as [`Vm::compile_file`] does, when it isn't in the code cache.
    fn compile_file_here(
        &mut self,
        path: &Path,
        file_text: &str,
        file: Atom,
        lazily: bool,
        debugged: bool,
    ) -> Run<FileCode> {
        let _parsing = crate::inspect::ParsingForDebugger::start(debugged);
        // A `.mjs` file is always a module, as in Node, whatever it has in it, and so is a `.js` file in a package that says
        // its files are. Anything else is told from what it says. What the package says is only looked for when what's in
        // the file doesn't say it's a module already, as finding it can mean looking in every folder up to the drive's.
        let extension = path.extension().and_then(|extension| extension.to_str());
        let package_decides = matches!(extension, Some("js" | "jsx" | "ts" | "tsx"));
        let mut goal = self.must_be_module(path);
        // What's been read ahead hasn't been through a filter, so with one it's read again.
        let ahead = if crate::inspect::has_source_filter()
            || debugged
            || crate::source_overlay::contains(path)
        {
            None
        } else {
            crate::ahead::take(path)
        };
        if let Some(parsed) = ahead {
            // What was read ahead as CommonJS is read again as a module if its package says it is one.
            if !parsed.is_module && !goal && package_decides && self.package_says_module(path) {
                goal = true;
            } else if parsed.is_module || !goal {
                let (proto, is_module) = self.compile_parsed(parsed, file, lazily)?;
                return Ok(FileCode::of(proto, is_module));
            }
        }
        let source = match read_source(path) {
            Ok(source) => source,
            Err(error) => {
                return Err(self.make_error(
                    self.realm.error_proto,
                    &format!("Cannot read {file_text}: {error}"),
                ));
            }
        };
        let says_module = might_be_module(&source) || mentions_await(&source);
        if !goal && !says_module && package_decides && self.package_says_module(path) {
            goal = true;
        }
        if goal {
            let memory = crate::ast::AstMemory::new();
            let room = self.stack_room();
            let text = self.add_source(SourceText::file(path.to_owned(), &source));
            let parsing =
                crate::startup::span(crate::startup::Phase::Parse, || Some(file_text.to_owned()));
            let parsed = parser::parse_module(&source, &memory, room);
            drop(parsing);
            let compiled = parsed.and_then(|program| {
                crate::compiler::Compiler::new(self, file)
                    .with_source(text)
                    .program(&program, true)
            });
            return match compiled {
                Ok(proto) => Ok(FileCode::Module(proto)),
                Err(error) => Err(self.report_syntax_error(&error, file)),
            };
        }
        let (proto, is_module) = self.compile_file_source(source, file, lazily)?;
        Ok(FileCode::of(proto, is_module))
    }

    /// Compiles what's in a file. The text is gone before any of the code runs, so a module that loads others isn't
    /// holding on to its own text the whole time, unless `lazily` and some of its functions wait to be compiled.
    fn compile_file_source(
        &mut self,
        source: String,
        file: Atom,
        lazily: bool,
    ) -> Run<(u32, bool)> {
        // A file is read once, as CommonJS or, if it says `import` or `export` anywhere, maybe as a module. What the
        // parser made is kept for the functions that wait to be compiled, with the source.
        let allow_module = might_be_module(&source) || mentions_await(&source);
        let path = PathBuf::from(self.atom_text(file));
        let text = self.add_source(SourceText::file(path, &source));
        let kept = if lazily {
            KeptAst::sharing(KeptSource::Owned(source), self.kept_memory())
        } else {
            KeptAst::new(KeptSource::Owned(source))
        };
        let room = self.stack_room();
        let parsing = crate::startup::span(crate::startup::Phase::Parse, || None);
        let read = parser::parse_file(
            kept.text(),
            kept.memory(),
            room,
            &COMMONJS_PARAMS,
            allow_module,
        );
        drop(parsing);
        let compiled = match read {
            Ok(parser::FileProgram::Wrapped(program)) => Some((program, false)),
            Ok(parser::FileProgram::Module(program)) => Some((program, true)),
            Ok(parser::FileProgram::Unsure) => None,
            Err(error) => return Err(self.report_syntax_error(&error, file)),
        };
        if let Some((program, is_module)) = compiled {
            let compiled = crate::compiler::Compiler::new(self, file)
                .keeping(&kept, lazily)
                .with_source(text)
                .program(&program, is_module);
            kept.finished();
            return match compiled {
                Ok(proto) => Ok((proto, is_module)),
                Err(error) => Err(self.report_syntax_error(&error, file)),
            };
        }
        let source = kept.text().to_owned();
        drop(kept);
        // What wasn't clear from one reading is read as a module, which can wait at its top, and if it isn't one, again
        // as CommonJS, which says what's wrong with it as it would have otherwise.
        let memory = crate::ast::AstMemory::new();
        let room = self.stack_room();
        let says_module = might_be_module(&source);
        match parser::parse_module(&source, &memory, room) {
            Ok(program) => {
                let syntax = has_module_syntax(&program);
                match crate::compiler::Compiler::new(self, file)
                    .with_source(text)
                    .program(&program, true)
                {
                    Ok(proto) => {
                        let waits = self
                            .modules
                            .infos
                            .get(&proto)
                            .is_some_and(|info| info.is_async);
                        if syntax || waits {
                            return Ok((proto, true));
                        }
                        self.modules.infos.remove(&proto);
                    }
                    Err(error) if syntax => return Err(self.report_syntax_error(&error, file)),
                    Err(_) => {}
                }
            }
            Err(error) => {
                // It may be a module written as only a script could be, with `await` as a name, say.
                let memory = crate::ast::AstMemory::new();
                if let Ok(program) = parser::parse(&source, &memory, room, false) {
                    if has_module_syntax(&program) {
                        let compiler = crate::compiler::Compiler::new(self, file);
                        return match compiler.program(&program, true) {
                            Ok(proto) => Ok((proto, true)),
                            Err(error) => Err(self.report_syntax_error(&error, file)),
                        };
                    }
                } else if says_module {
                    return Err(self.report_syntax_error(&error, file));
                }
            }
        }
        let wrapped = wrapped_as_commonjs(source);
        let proto = self.compile_source(&wrapped, file)?;
        Ok((proto, false))
    }

    /// Runs a CommonJS file, compiled as [`wrapped_as_commonjs`] has it.
    fn run_commonjs(
        &mut self,
        proto: u32,
        dir: &Path,
        file_text: &str,
        module: Value,
        exports: Value,
    ) -> Run<()> {
        let _time = crate::startup::span(crate::startup::Phase::Run, || Some(file_text.to_owned()));
        let function = self.run_proto(proto, &[])?;
        let require = self.make_require(dir, require_function);
        let filename = self.new_str(file_text);
        let dirname = self.new_str(&dir.to_string_lossy());
        self.call(
            function,
            exports,
            &[exports, require, module, filename, dirname],
        )?;
        Ok(())
    }

    /// Loads an ES module as the language says, with each module it imports that hasn't been loaded, and theirs, all
    /// together: each takes its first step, which hands over the cells it exports, then each its second, which takes the
    /// cells it imports, so that every import is the variable the module it's from has, not a copy, and modules can
    /// import each other. Then each runs, after what it imports has, and what waits at its top is waited for by what
    /// imports it.
    fn load_es_module(&mut self, path: &Path, proto: u32, module: Value) -> Run<()> {
        let mut started = Vec::new();
        let late_from = self.modules.late.len();
        let made = self
            .start_module(path, proto, module, &mut started)
            .and_then(|id| {
                let _time = crate::startup::span(crate::startup::Phase::Link, || None);
                // What's loaded by itself gives its namespace, which needs all it names.
                self.complete_module(id)?;
                self.link_started(&mut started, late_from)?;
                Ok(id)
            });
        let id = match made {
            Ok(id) => id,
            Err(error) => {
                for &(one, _, _) in &self.modules.late[late_from..] {
                    if !started.contains(&one) {
                        started.push(one);
                    }
                }
                // What named something started late names it as it did before, as that's gone.
                for (_, by, dep) in self.modules.late.split_off(late_from) {
                    if let Some(record) = self.modules.records.get_mut(&by) {
                        record.deps[dep] = Dep::Deferred;
                    }
                }
                // Nothing was run, so none of it happened.
                for one in started {
                    if let Some(record) = self.modules.records.remove(&one) {
                        self.modules.es.remove(&record.path);
                        if record.path != path {
                            self.modules.counted.set(None);
                            self.modules.loaded.remove(&record.path);
                        }
                    }
                }
                return Err(error);
            }
        };
        self.modules.late.truncate(late_from);
        self.evaluate_module(id)?;
        Ok(())
    }

    /// Makes the namespaces of modules that have been started and links them, with any that linking them starts in turn
    /// (see [`Dep::Deferred`]), which are added to `started`: those in `late` from `late_from` on.
    fn link_started(&mut self, started: &mut Vec<u32>, late_from: usize) -> Run<()> {
        let mut named = 0;
        let mut linked = 0;
        let mut late = late_from;
        loop {
            while late < self.modules.late.len() {
                started.push(self.modules.late[late].0);
                late += 1;
            }
            if named < started.len() {
                let one = started[named];
                named += 1;
                if !self.record(one).partial {
                    self.make_namespace(one)?;
                }
            } else if linked < started.len() {
                let one = started[linked];
                linked += 1;
                self.link_module(one)?;
            } else {
                return Ok(());
            }
        }
    }

    /// Loads all that a module that only passes on exports names and hasn't yet, and makes its namespace, which needs
    /// all of it: for `import * as`, `import()`, `require`, and `export *` of it from a module that doesn't only pass on
    /// exports. What's started is linked by whatever's loading, or here if nothing is. It does nothing for any other
    /// module.
    fn complete_module(&mut self, id: u32) -> Run<()> {
        if !self.modules.records.get(&id).is_some_and(|record| record.partial) {
            return Ok(());
        }
        self.record(id).partial = false;
        for dep in 0..self.record(id).deps.len() {
            if matches!(self.record(id).deps[dep], Dep::Deferred) {
                self.start_deferred(id, dep)?;
            }
            if let Dep::Es(other) = self.record(id).deps[dep] {
                if self.record(id).info.stars.contains(&dep) {
                    self.complete_module(other)?;
                }
            }
        }
        self.make_namespace(id)
    }

    /// Starts what a module names that was [`Dep::Deferred`], now that something asks for what it gives.
    fn start_deferred(&mut self, id: u32, dep: usize) -> Run<Dep> {
        let record = self.record(id);
        let dir = record.path.parent().unwrap_or(Path::new(".")).to_owned();
        let name = record.info.requested[dep].clone();
        let mut started = Vec::new();
        let found = self.find_dependency(&dir, &name, &mut started)?;
        self.record(id).deps[dep] = found.clone();
        self.modules.late.extend(started.into_iter().map(|one| (one, id, dep)));
        Ok(found)
    }

    /// What a module names, by its number, starting it first if it was [`Dep::Deferred`].
    fn dependency(&mut self, id: u32, dep: usize) -> Run<Dep> {
        match self.record(id).deps[dep].clone() {
            Dep::Deferred => self.start_deferred(id, dep),
            found => Ok(found),
        }
    }

    /// Whether the module `name` names from a module that only passes on exports can wait to be loaded until something
    /// asks for what it gives: when its package says it has no side effects. Then not loading it can't be seen, as all
    /// that loading it would do is make what it exports. A package that doesn't say so may have modules that do
    /// something as they're loaded (add styles, register themselves, set globals), even where what's passed on is from
    /// other modules that only pass on exports, as what those pass on can, so those are loaded as the language says.
    fn may_defer(&mut self, from: &Path, name: &str) -> Run<bool> {
        let dir = from.parent().unwrap_or(Path::new("."));
        let relative = name.starts_with("./") || name.starts_with("../");
        // A file next to this one is in its package, and needn't be looked for to know.
        if relative
            && !name.split('/').any(|part| part == "..")
            && matches!(*self.side_effects_of(dir), SideEffects::None)
        {
            return Ok(true);
        }
        let path = match self.resolve_module(dir, name, true) {
            Ok(Resolved::File(path)) => path,
            // What can't be found is looked for again as it's loaded, to say so then.
            Ok(Resolved::Builtin(_)) | Err(_) => return Ok(false),
        };
        let folder = path.parent().unwrap_or(Path::new(".")).to_owned();
        Ok(match &*self.side_effects_of(&folder) {
            SideEffects::None => true,
            SideEffects::Some => false,
            SideEffects::Only(root, patterns) => {
                !patterns.iter().any(|pattern| side_effect_pattern_matches(pattern, root, &path))
            }
        })
    }

    /// What the package of the files in a folder says about their side effects: its nearest `package.json` that has a
    /// name or `sideEffects`.
    fn side_effects_of(&mut self, dir: &Path) -> std::rc::Rc<SideEffects> {
        if let Some(known) = self.modules.side_effects.get(dir) {
            return known.clone();
        }
        let found = if is_file(&dir.join("package.json")) {
            match self.read_package_json(dir) {
                Ok(Some(json)) if json.is_object() => {
                    let said = self.get_named(json, "sideEffects").unwrap_or(Value::UNDEFINED);
                    let named = self.get_named(json, "name").is_ok_and(|name| name.is_string());
                    if said.is_bool() {
                        Some(if self.to_boolean(said) { SideEffects::Some } else { SideEffects::None })
                    } else if said.is_object() {
                        let patterns = self.iterate_to_vec(said).unwrap_or_default();
                        let patterns = patterns
                            .into_iter()
                            .filter(|pattern| pattern.is_string())
                            .map(|pattern| self.text_of(pattern).to_rust())
                            .collect();
                        Some(SideEffects::Only(dir.to_owned(), patterns))
                    } else if named {
                        Some(SideEffects::Some)
                    } else {
                        // A `package.json` with neither, as some packages have in their folders to say what kind of
                        // module the files are, is part of the package above.
                        None
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        let found = match (found, dir.parent()) {
            (Some(found), _) => std::rc::Rc::new(found),
            (None, Some(parent)) => self.side_effects_of(parent),
            (None, None) => std::rc::Rc::new(SideEffects::Some),
        };
        self.modules.side_effects.insert(dir.to_owned(), found.clone());
        found
    }

    /// A module's first step, and then the first steps of what it imports, as far as they go.
    fn start_module(
        &mut self,
        path: &Path,
        proto: u32,
        module: Value,
        started: &mut Vec<u32>,
    ) -> Run<u32> {
        let info = self.modules.infos.remove(&proto).unwrap_or_default();
        let target = self.new_object_with_proto(NONE, Kind::Plain);
        let namespace = self.new_namespace(target);
        let exports_atom = self.intern_str("exports");
        self.define(module.handle(), exports_atom, namespace, PLAIN);
        let id = self.modules.next_record;
        self.modules.next_record = self.modules.next_record.wrapping_add(1);
        let requested = info.requested.clone();
        let only_reexports = info.only_reexports;
        self.modules.records.insert(
            id,
            Record {
                path: path.to_owned(),
                cells: vec![Value::UNDEFINED; info.locals.len()],
                info,
                generator: Value::UNDEFINED,
                namespace,
                target,
                namespace_cell: Value::UNDEFINED,
                required: Value::UNDEFINED,
                deps: Vec::new(),
                state: State::Started,
                promise: Value::UNDEFINED,
                waiting: 0,
                error: Value::UNDEFINED,
                filled: Vec::new(),
                partial: false,
            },
        );
        self.modules.es.insert(path.to_owned(), id);
        started.push(id);
        let name = self.intern_str("bind");
        let bind = self.new_native(name, 2, bind_function, Value::number(f64::from(id)), false);
        let name = self.intern_str("import");
        let import = self.new_native(name, 1, import_binding, Value::number(f64::from(id)), false);
        let generator = self.run_proto(proto, &[bind, import])?;
        self.record(id).generator = generator;
        self.resume_generator(generator, Resume::Next, Value::UNDEFINED)?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_owned();
        for name in requested {
            let dep = if only_reexports && self.may_defer(path, &name)? {
                self.record(id).partial = true;
                Dep::Deferred
            } else {
                self.find_dependency(&dir, &name, started)?
            };
            self.record(id).deps.push(dep);
        }
        Ok(id)
    }

    fn record(&mut self, id: u32) -> &mut Record {
        self.modules
            .records
            .get_mut(&id)
            .expect("a module that's being loaded is there")
    }

    /// What a module named by an `import` is, starting it if it's an ES module that hasn't been.
    fn find_dependency(&mut self, dir: &Path, name: &str, started: &mut Vec<u32>) -> Run<Dep> {
        let path = match self.resolve_module(dir, name, true)? {
            Resolved::Builtin(module) => return Ok(Dep::Builtin(module)),
            Resolved::File(path) => path,
        };
        if let Some(&id) = self.modules.es.get(&path) {
            return Ok(Dep::Es(id));
        }
        let extension = path.extension().and_then(|extension| extension.to_str());
        if self.modules.loaded.contains_key(&path)
            || self.modules.prepared.contains_key(&path)
            || matches!(extension, Some("json" | "css"))
        {
            return Ok(Dep::File(path));
        }
        match self.compile_file(&path)? {
            FileCode::CommonJs(proto) => {
                self.modules.prepared.insert(path.clone(), proto);
                Ok(Dep::File(path))
            }
            FileCode::Module(proto) => {
                let module = self.new_plain();
                self.modules.counted.set(None);
                self.modules.loaded.insert(path.clone(), module);
                self.modules.order.push(path.clone());
                Ok(Dep::Es(self.start_module(&path, proto, module, started)?))
            }
        }
    }

    /// Gives a module's namespace a getter for each name it exports, in the order of their code units, as the language
    /// has it. Only names that can be known now are there: what comes from `export *` of CommonJS is added once that's run.
    fn make_namespace(&mut self, id: u32) -> Run<()> {
        let stars = self.record(id).info.stars.clone();
        for star in stars {
            if let Dep::Es(other) = self.dependency(id, star)? {
                self.complete_module(other)?;
            }
        }
        let mut names = Vec::new();
        self.export_names(id, &mut Vec::new(), &mut names);
        names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        names.dedup();
        let target = self.record(id).target;
        for name in names {
            let Found::Cell(cell) = self.resolve_export(id, &name, &mut Vec::new())? else {
                continue;
            };
            let key = self.intern_str(&name);
            let getter = self.new_native(key, 0, binding_getter, cell, false);
            self.define_accessor(target.handle(), key, Some(getter), None, ENUMERABLE);
        }
        Ok(())
    }

    /// What `require` of an ES module gives, as Node has it: what it exports as `module.exports` if it says so, or else,
    /// if it has a default export, its names with `__esModule` too, for what Babel and TypeScript write to see it's a
    /// module, or else its namespace itself.
    fn required_namespace(&mut self, namespace: Value) -> Run<Value> {
        let Some((&id, record)) = self
            .modules
            .records
            .iter()
            .find(|(_, record)| record.namespace == namespace)
        else {
            return Ok(namespace);
        };
        if !record.required.is_undefined() {
            return Ok(record.required);
        }
        let target = record.target;
        let whole = self.intern_str("module.exports");
        if self.has_property(target.handle(), whole) {
            return self.get(namespace, whole);
        }
        if !self.has_property(target.handle(), text::DEFAULT) {
            return Ok(namespace);
        }
        let copy = self.new_object_with_proto(NONE, Kind::Plain);
        let flag = self.intern_str("__esModule");
        let cell = self.new_cell(Value::TRUE);
        let getter = self.new_native(flag, 0, binding_getter, cell, false);
        self.define_accessor(copy.handle(), flag, Some(getter), None, ENUMERABLE);
        for (key, _) in self.own_keys(target.handle()) {
            let shape = self.objects[target.index()].shape;
            let Some((slot, flags)) = self.shapes.lookup(shape, key) else {
                continue;
            };
            let held = self.objects[target.index()].slots()[slot as usize];
            match (&self.objects[held.index()].kind, flags & ACCESSOR != 0) {
                (Kind::Accessor { get, .. }, true) => {
                    let get = *get;
                    self.define_accessor(copy.handle(), key, Some(get), None, ENUMERABLE);
                }
                _ => self.define(copy.handle(), key, held, ENUMERABLE | WRITABLE),
            }
        }
        let required = self.new_namespace(copy);
        self.record(id).required = required;
        Ok(required)
    }

    /// Each name a module exports, with what it has through `export *`, but not `default` from those.
    fn export_names(&self, id: u32, seen: &mut Vec<u32>, out: &mut Vec<String>) {
        if seen.contains(&id) {
            return;
        }
        seen.push(id);
        let Some(record) = self.modules.records.get(&id) else {
            return;
        };
        let first = seen.len() == 1;
        for (name, _) in &record.info.exports {
            if first || name != "default" {
                out.push(name.clone());
            }
        }
        for &star in &record.info.stars {
            if let Some(Dep::Es(other)) = record.deps.get(star) {
                self.export_names(*other, seen, out);
            }
        }
    }

    /// The cell that a name a module exports is, wherever it really comes from.
    fn resolve_export(&mut self, id: u32, name: &str, seen: &mut Vec<(u32, String)>) -> Run<Found> {
        if seen.iter().any(|(one, found)| *one == id && found == name) {
            return Ok(Found::Missing);
        }
        seen.push((id, name.to_owned()));
        let Some(record) = self.modules.records.get(&id) else {
            return Ok(Found::Missing);
        };
        let direct = record
            .info
            .exports
            .iter()
            .find(|(exported, _)| exported == name)
            .map(|(_, from)| from.clone());
        if let Some(from) = direct {
            return match from {
                ExportFrom::Local(at) => Ok(Found::Cell(self.record(id).cells[at])),
                ExportFrom::Import(dep, imported) => self.binding_from(id, dep, imported, seen),
            };
        }
        if name == "default" {
            return Ok(Found::Missing);
        }
        let stars = record.info.stars.clone();
        let mut found = None;
        let mut commonjs = None;
        for star in stars {
            let dep = self.dependency(id, star)?;
            let Dep::Es(other) = dep else {
                commonjs.get_or_insert(star);
                continue;
            };
            match self.resolve_export(other, name, seen)? {
                Found::Cell(cell) => {
                    if found.is_some_and(|before| before != cell) {
                        return Ok(Found::Ambiguous);
                    }
                    found = Some(cell);
                }
                Found::Ambiguous => return Ok(Found::Ambiguous),
                Found::Missing => {}
            }
        }
        Ok(match (found, commonjs) {
            (Some(cell), _) => Found::Cell(cell),
            // What CommonJS exports can't be known until it's run, so it's taken to have it.
            (None, Some(star)) => Found::Cell(self.filled_cell(id, star, Some(name.to_owned()))?),
            (None, None) => Found::Missing,
        })
    }

    /// The cell of `name` from the module `id` names as its `dep`th, or of all of that module.
    fn binding_from(
        &mut self,
        id: u32,
        dep: usize,
        name: Option<String>,
        seen: &mut Vec<(u32, String)>,
    ) -> Run<Found> {
        match self.dependency(id, dep)? {
            Dep::Deferred => unreachable!("a module that's asked for is started"),
            Dep::Es(other) => match name {
                Some(name) => self.resolve_export(other, &name, seen),
                None => {
                    // Its namespace needs all it names.
                    self.complete_module(other)?;
                    let record = self.record(other);
                    if record.namespace_cell.is_undefined() {
                        let namespace = record.namespace;
                        let cell = self.new_cell(namespace);
                        self.record(other).namespace_cell = cell;
                    }
                    Ok(Found::Cell(self.record(other).namespace_cell))
                }
            },
            Dep::Builtin(_) | Dep::File(_) => Ok(Found::Cell(self.filled_cell(id, dep, name)?)),
        }
    }

    /// What TypeScript imports by a name that isn't exported, which can only be a type, as the app wouldn't have compiled
    /// otherwise, and TypeScript leaves an import that's only used as a type out. It's a cell that's always undefined, kept
    /// with the module's other cells.
    fn type_cell(&mut self, id: u32, name: &str) -> Value {
        let record = self.record(id);
        if let Some((_, _, cell)) = record
            .filled
            .iter()
            .find(|(dep, found, _)| *dep == usize::MAX && found.as_deref() == Some(name))
        {
            return *cell;
        }
        let cell = self.new_cell(Value::UNDEFINED);
        self.record(id)
            .filled
            .push((usize::MAX, Some(name.to_owned()), cell));
        cell
    }

    /// A cell for what's imported from a module that isn't an ES module, which is filled in when that's been run, and
    /// straight away if it has.
    fn filled_cell(&mut self, id: u32, dep: usize, name: Option<String>) -> Run<Value> {
        let record = self.record(id);
        if let Some((_, _, cell)) = record
            .filled
            .iter()
            .find(|(one, found, _)| *one == dep && *found == name)
        {
            return Ok(*cell);
        }
        let module = match &record.deps[dep] {
            Dep::Builtin(module) => Some(*module),
            _ => None,
        };
        let cell = self.new_cell(Value::EMPTY);
        // It's kept with the module's cells before what's imported is asked for, as that runs code, which can collect
        // garbage: a cell nothing holds yet would be given to something else, and this module would read what that holds.
        self.record(id).filled.push((dep, name.clone(), cell));
        if let Some(module) = module {
            let value = self.imported_value(module, name.as_deref())?;
            self.set_cell(cell, value);
        }
        Ok(cell)
    }

    fn set_cell(&mut self, cell: Value, value: Value) {
        self.cells[cell.index()] = value;
        self.cells.write_barrier(cell.index(), value);
    }

    /// What an import of `name` from a module that isn't an ES module gets, as Node has it: its `module.exports` is its
    /// default, and its properties are names too.
    fn imported_value(&mut self, module: Value, name: Option<&str>) -> Run<Value> {
        let exports = self.exports_of(module)?;
        let namespace = self.namespace_of(exports)?;
        match name {
            Some(name) => {
                let key = self.intern_str(name);
                self.get(namespace, key)
            }
            None => Ok(namespace),
        }
    }

    /// A module's second step: it takes the cells of what it imports, and makes its functions. What it exports from
    /// elsewhere has to be there too, even if nothing imports it.
    fn link_module(&mut self, id: u32) -> Run<()> {
        let record = self.record(id);
        let indirect: Vec<(usize, String)> = record
            .info
            .exports
            .iter()
            .filter_map(|(_, from)| match from {
                // What's passed on from a module that hasn't been asked for isn't looked at until it is.
                ExportFrom::Import(dep, _) if matches!(record.deps[*dep], Dep::Deferred) => None,
                ExportFrom::Import(dep, Some(name)) => Some((*dep, name.clone())),
                _ => None,
            })
            .collect();
        for (dep, name) in indirect {
            let found = self.binding_from(id, dep, Some(name.clone()), &mut Vec::new())?;
            let from = self.record(id).info.requested[dep].clone();
            match found {
                Found::Cell(_) => {}
                Found::Missing if is_typescript(&self.record(id).path) => {}
                Found::Missing => {
                    return Err(self.syntax_error(&format!(
                        "The requested module '{from}' does not provide an export named '{name}'"
                    )));
                }
                Found::Ambiguous => {
                    return Err(self.syntax_error(&format!(
                        "The requested module '{from}' contains conflicting star exports for name '{name}'"
                    )));
                }
            }
        }
        let generator = self.record(id).generator;
        self.resume_generator(generator, Resume::Next, Value::UNDEFINED)?;
        self.record(id).state = State::Linked;
        Ok(())
    }

    /// Runs a module's code, after what it imports, if it hasn't been. Gives a promise of it finishing if that has to be
    /// waited for.
    fn evaluate_module(&mut self, id: u32) -> Run<Option<Value>> {
        let Some(record) = self.modules.records.get(&id) else {
            return Ok(None);
        };
        match record.state {
            // A module that only passes on exports may have been asked for more since it ran, which runs now. It's
            // running again meanwhile, for modules that pass on each other's.
            State::Done if record.info.only_reexports => {
                self.record(id).state = State::Running;
                let mut ran = Ok(None);
                for dep in 0..self.record(id).deps.len() {
                    if let Dep::Es(other) = self.record(id).deps[dep] {
                        ran = self.evaluate_module(other);
                        if ran.is_err() {
                            break;
                        }
                    }
                }
                self.record(id).state = State::Done;
                return ran.map(|_| None);
            }
            // What's already running is something that imports this, as modules can import each other.
            State::Done | State::Running | State::Started => return Ok(None),
            State::Waiting => return Ok(Some(record.promise)),
            State::Failed => return Err(record.error),
            State::Linked => {}
        }
        self.record(id).state = State::Running;
        let mut pending = Vec::new();
        for dep in 0..self.record(id).deps.len() {
            let ran = match self.record(id).deps[dep].clone() {
                Dep::Es(other) => match self.evaluate_module(other) {
                    Ok(Some(promise)) => {
                        pending.push(promise);
                        Ok(())
                    }
                    Ok(None) => Ok(()),
                    Err(error) => Err(error),
                },
                Dep::File(path) => self
                    .load_module(&path)
                    .and_then(|module| self.fill_cells(id, dep, module)),
                Dep::Builtin(_) | Dep::Deferred => Ok(()),
            };
            if let Err(error) = ran {
                return self.module_failed(id, error);
            }
        }
        if pending.is_empty() {
            return self.run_module(id, Resume::Next, Value::UNDEFINED);
        }
        let promise = self.module_promise(id);
        self.record(id).state = State::Waiting;
        self.record(id).waiting = pending.len();
        let name = self.intern_str("");
        let done = self.new_native(
            name,
            1,
            dependency_done,
            Value::number(f64::from(id)),
            false,
        );
        let failed = self.new_native(
            name,
            1,
            dependency_failed,
            Value::number(f64::from(id)),
            false,
        );
        for waiting in pending {
            self.promise_then(waiting, done, failed, Value::UNDEFINED);
        }
        Ok(Some(promise))
    }

    /// Fills in what a module imports from one that isn't an ES module, now that it's been run.
    fn fill_cells(&mut self, id: u32, dep: usize, module: Value) -> Run<()> {
        let filled: Vec<(Option<String>, Value)> = self
            .record(id)
            .filled
            .iter()
            .filter(|(one, _, _)| *one == dep)
            .map(|(_, name, cell)| (name.clone(), *cell))
            .collect();
        for (name, cell) in filled {
            let value = self.imported_value(module, name.as_deref())?;
            self.set_cell(cell, value);
        }
        // `export * from` CommonJS gives what it has now, as it can't be known before.
        if self.record(id).info.stars.contains(&dep) {
            let exports = self.exports_of(module)?;
            if exports.is_object() {
                let target = self.record(id).target;
                let mut excluded: Vec<Atom> = self
                    .own_keys(target.handle())
                    .into_iter()
                    .map(|(key, _)| key)
                    .collect();
                excluded.push(text::DEFAULT);
                self.copy_properties(target, exports, &excluded)?;
                let whole = self.intern_str("module.exports");
                if !self.has_property(target.handle(), whole) {
                    self.define(target.handle(), whole, exports, ENUMERABLE | WRITABLE);
                }
            }
        }
        Ok(())
    }

    /// Carries on with a module's code, from its start or from what it was waiting for.
    fn run_module(&mut self, id: u32, mode: Resume, value: Value) -> Run<Option<Value>> {
        let generator = self.record(id).generator;
        let running = crate::startup::span(crate::startup::Phase::Run, || {
            Some(self.record(id).path.to_string_lossy().into_owned())
        });
        let resumed = self.resume_generator(generator, mode, value);
        drop(running);
        match resumed {
            Ok((Suspended::Await, awaited)) => {
                let promise = self.module_promise(id);
                self.record(id).state = State::Waiting;
                let waiting = match self.promise_resolve_value(awaited) {
                    Ok(waiting) => waiting,
                    Err(error) => return self.run_module(id, Resume::Throw, error),
                };
                let name = self.intern_str("");
                let data = Value::number(f64::from(id));
                let fulfilled = self.new_native(name, 1, module_resumed, data, false);
                let rejected = self.new_native(name, 1, module_rejected, data, false);
                self.promise_then(waiting, fulfilled, rejected, Value::UNDEFINED);
                Ok(Some(promise))
            }
            Ok(_) => {
                let record = self.record(id);
                record.state = State::Done;
                record.generator = Value::UNDEFINED;
                let promise = record.promise;
                if promise.is_undefined() {
                    return Ok(None);
                }
                self.resolve_promise(promise, Value::UNDEFINED);
                Ok(Some(promise))
            }
            Err(error) => self.module_failed(id, error),
        }
    }

    /// A module threw, or something it imports did. That's thrown, or its promise is broken if something's waiting for it.
    fn module_failed(&mut self, id: u32, error: Value) -> Run<Option<Value>> {
        let record = self.record(id);
        record.state = State::Failed;
        record.error = error;
        record.generator = Value::UNDEFINED;
        let promise = record.promise;
        if promise.is_undefined() {
            return Err(error);
        }
        let is_main = self.modules.main.as_deref()
            == self
                .modules
                .records
                .get(&id)
                .map(|record| record.path.as_path());
        if is_main {
            self.modules.main_failure = Some((error, promise));
        }
        self.reject_promise(promise, error);
        Ok(Some(promise))
    }

    fn module_promise(&mut self, id: u32) -> Value {
        let promise = self.record(id).promise;
        if !promise.is_undefined() {
            return promise;
        }
        let promise = self.new_promise();
        self.record(id).promise = promise;
        promise
    }

    /// Whether the first file is an ES module that's still waiting at its top, or for what it imports.
    pub(crate) fn main_is_waiting(&self) -> bool {
        let Some(main) = &self.modules.main else {
            return false;
        };
        self.modules
            .es
            .get(main)
            .and_then(|id| self.modules.records.get(id))
            .is_some_and(|record| record.state == State::Waiting)
    }

    /// A promise of an ES module's code having run, if it hasn't finished, by its namespace.
    fn promise_of_namespace(&self, namespace: Value) -> Option<Value> {
        self.modules
            .records
            .values()
            .find(|record| record.namespace == namespace && record.state == State::Waiting)
            .map(|record| record.promise)
    }

    pub(crate) fn exports_of(&mut self, module: Value) -> Run<Value> {
        self.get_named(module, "exports")
    }

    /// Runs a file as the program, as an ES module whatever's in it.
    pub(crate) fn run_main_module(&mut self, path: &Path) -> Run<Value> {
        self.modules.module_goal = Some(canonical(path));
        self.run_main(path)
    }

    /// Runs a file as the program.
    pub(crate) fn run_main(&mut self, path: &Path) -> Run<Value> {
        let absolute = canonical(path);
        let module = self.load_module(&absolute)?;
        self.exports_of(module)
    }

    /// Runs code given as a string, as `eval` does. It can only see globals, as if it were called as `(0, eval)(x)`.
    fn run_eval(&mut self, source: &str) -> Run<Value> {
        let file = self.intern_str("eval");
        self.run_script_source(source, file, false)
    }

    /// Runs code as indirect `eval` does, for a debugger.
    pub(crate) fn run_eval_source(&mut self, source: &str) -> Run<Value> {
        self.run_eval(source)
    }

    pub(crate) fn install_modules(&mut self) {
        self.global_function("eval", 1, eval_function);
        let atom = self.private_atom("%import");
        let name = self.intern_str("import");
        let function = self.new_native(name, 2, dynamic_import, Value::UNDEFINED, false);
        self.add_property(self.global, atom, function, HIDDEN);
        let atom = self.private_atom("%meta");
        let name = self.intern_str("meta");
        let function = self.new_native(name, 1, meta_function, Value::UNDEFINED, false);
        self.add_property(self.global, atom, function, HIDDEN);
    }

    /// Makes a module's exports into what an `import` of it gets. A CommonJS module's exports are its default, with
    /// its properties as names too.
    fn namespace_of(&mut self, exports: Value) -> Run<Value> {
        if !exports.is_object() {
            let namespace = self.new_plain();
            self.define(namespace.handle(), text::DEFAULT, exports, PLAIN);
            return Ok(namespace);
        }
        let flag = self.intern_str("__esModule");
        if self.namespace_target(exports).is_some() || self.has_property(exports.handle(), flag) {
            return Ok(exports);
        }
        let namespace = self.new_plain();
        // Copying can run getters, which can collect garbage, and nothing else holds this yet.
        let _held = self.root(namespace);
        self.copy_properties(namespace, exports, &[])?;
        self.define(namespace.handle(), text::DEFAULT, exports, PLAIN);
        // Node 24 gives it under this name too.
        let whole = self.intern_str("module.exports");
        self.define(namespace.handle(), whole, exports, PLAIN);
        Ok(namespace)
    }
}

/// What a CommonJS file's code is given, as [`wrapped_as_commonjs`] has it.
pub(crate) const COMMONJS_PARAMS: [&str; 5] =
    ["exports", "require", "module", "__filename", "__dirname"];

/// What a module that comes with the engine is given. Its code is read as the body of a function that takes these,
/// which the program returns, as for CommonJS.
pub(crate) const BUILTIN_PARAMS: [&str; 5] = ["exports", "require", "module", "__inspect", "__fs"];

/// CommonJS code, in a function that's given what Node gives it, which is what running it returns.
pub(crate) fn wrapped_as_commonjs(source: String) -> String {
    let mut wrapped = source;
    wrapped.insert_str(
        0,
        "return (function (exports, require, module, __filename, __dirname) {",
    );
    wrapped.push_str("\n})");
    wrapped
}

/// A full path, without the `\\\\?\\` that Windows puts on the front, which nothing wants to see.
pub(crate) fn canonical(path: &Path) -> PathBuf {
    // What's inside the program isn't on the disk to ask about.
    if crate::packed::is_packed_file(path) {
        return crate::packed::real_path(path);
    }
    // A file that isn't a link is where its folder is, under the name it's written with, and its folder is worked out
    // once for all the files in it.
    if let (Some(folder), Some(name)) = (path.parent(), path.file_name()) {
        if !crate::packed::running_packed() {
            if let Some(listing) = listing(folder) {
                if let Some((written, Some(false))) =
                    listing.get(&name.to_string_lossy().to_lowercase())
                {
                    let full = FULL_FOLDERS.with(|full| {
                        full.borrow_mut()
                            .entry(folder.to_owned())
                            .or_insert_with(|| canonical_by_asking(folder))
                            .clone()
                    });
                    return full.join(written);
                }
            }
        }
    }
    canonical_by_asking(path)
}

/// [`canonical`], asking the disk.
fn canonical_by_asking(path: &Path) -> PathBuf {
    // What's inside the program isn't on the disk to ask about.
    let full = std::fs::canonicalize(path).unwrap_or_else(|_| crate::packed::tidied(path));
    let text = full.to_string_lossy();
    // `\\?\UNC\server\share` is `\\server\share`, said so its length isn't limited.
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{share}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => full,
    }
}

fn split_package(name: &str) -> (&str, Option<&str>) {
    let mut slashes = name
        .char_indices()
        .filter(|(_, c)| *c == '/')
        .map(|(at, _)| at);
    let split = if name.starts_with('@') {
        slashes.next();
        slashes.next()
    } else {
        slashes.next()
    };
    match split {
        Some(at) => (&name[..at], Some(&name[at + 1..])),
        None => (name, None),
    }
}

/// The path a `file:` URL is, as `import()` takes one, without what's after `?` or `#`, which only make it a different module to a
/// browser. Anything else is as it was.
fn file_url_path(name: String) -> String {
    let Some(rest) = name.strip_prefix("file://") else {
        return name;
    };
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    // `file:///C:/x` is a drive's path, and `file://host/x` is a file on another computer.
    let path = match rest.strip_prefix('/') {
        Some(local) if local.as_bytes().get(1) == Some(&b':') => local.to_owned(),
        Some(local) => format!("/{local}"),
        None => format!("//{rest}"),
    };
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = |byte: u8| char::from(byte).to_digit(16);
        match (
            bytes[at],
            bytes.get(at + 1).copied().and_then(hex),
            bytes.get(at + 2).copied().and_then(hex),
        ) {
            (b'%', Some(high), Some(low)) => {
                decoded.push((high * 16 + low) as u8);
                at += 3;
            }
            (byte, _, _) => {
                decoded.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).replace('/', "\\")
}

fn required_module(vm: &mut Vm, name: Value, dir: &Path, is_import: bool) -> Run<Value> {
    let name = vm.to_rust_string(name)?;
    match vm.resolve_module(dir, &name, is_import)? {
        Resolved::Builtin(module) => Ok(module),
        Resolved::File(path) => vm.load_module(&path),
    }
}

/// `require`, from CommonJS.
fn require_function(vm: &mut Vm, args: Args) -> Run<Value> {
    let name = vm.arg(&args, 0);
    let dir = vm.native_data_of(&args);
    let dir = PathBuf::from(vm.text_of(dir).to_rust());
    let module = required_module(vm, name, &dir, false)?;
    let exports = vm.exports_of(module)?;
    // An ES module that's still waiting at its top has nothing to give yet, which Node says this way.
    if vm.promise_of_namespace(exports).is_some() {
        let error = vm.make_error(
            vm.realm.error_proto,
            "require() cannot be used on an ESM graph with top-level await. Use import() instead.",
        );
        let code = vm.new_str("ERR_REQUIRE_ASYNC_MODULE");
        let key = vm.intern_str("code");
        vm.define(error.handle(), key, code, PLAIN);
        return Err(error);
    }
    if vm.namespace_target(exports).is_some() {
        return vm.required_namespace(exports);
    }
    Ok(exports)
}

/// `require.resolve(name, { paths })`: which file `require` would load, without loading it, looked for from each of
/// `paths` if it says, and otherwise from the module's own folder. A module that comes with the engine is its own name.
fn resolve_function(vm: &mut Vm, args: Args) -> Run<Value> {
    let given = vm.arg(&args, 0);
    let name = file_url_path(vm.to_rust_string(given)?);
    let bare = name.strip_prefix("node:").unwrap_or(&name).to_owned();
    if vm.is_builtin_name(&bare) {
        return Ok(vm.new_str(&name));
    }
    let dir = vm.native_data_of(&args);
    let mut dirs = vec![PathBuf::from(vm.text_of(dir).to_rust())];
    let options = vm.arg(&args, 1);
    if options.is_object() {
        let paths = vm.get_named(options, "paths")?;
        if paths.is_object() {
            let given = vm.iterate_to_vec(paths)?;
            dirs.clear();
            for one in given {
                dirs.push(PathBuf::from(vm.to_rust_string(one)?));
            }
        }
    }
    for dir in dirs {
        let found = vm.resolve_file(&dir, &bare, false).or_else(|_| {
            // What's in a folder may have changed since it was looked at, as `require` has it.
            ASK_THE_DISK.with(|ask| ask.set(true));
            let found = vm.resolve_file(&dir, &bare, false);
            ASK_THE_DISK.with(|ask| ask.set(false));
            found
        });
        if let Ok(found) = found {
            return Ok(vm.new_str(&canonical(&found).to_string_lossy()));
        }
    }
    Err(vm.module_not_found(&name))
}

fn import_now(vm: &mut Vm, args: &Args) -> Run<Value> {
    let name = vm.arg(args, 0);
    let file = vm.arg(args, 1);
    let file = if file.is_symbol() {
        vm.atom_text(Atom(file.handle()))
    } else {
        vm.to_rust_string(file)?
    };
    let dir = Path::new(&file)
        .parent()
        .unwrap_or(Path::new("."))
        .to_owned();
    let module = required_module(vm, name, &dir, true)?;
    let exports = vm.exports_of(module)?;
    vm.namespace_of(exports)
}

/// `import(x)`, which is a promise of the module's namespace once its code has run, which is now unless it waits at its top.
fn dynamic_import(vm: &mut Vm, args: Args) -> Run<Value> {
    let promise = vm.new_promise();
    match import_now(vm, &args) {
        Ok(namespace) => match vm.promise_of_namespace(namespace) {
            Some(running) => {
                let name = vm.intern_str("");
                let give = vm.new_native(name, 1, give_namespace, namespace, false);
                vm.promise_then(running, give, Value::UNDEFINED, promise);
            }
            None => vm.resolve_promise(promise, namespace),
        },
        Err(error) => vm.reject_promise(promise, error),
    }
    Ok(promise)
}

fn give_namespace(vm: &mut Vm, args: Args) -> Run<Value> {
    Ok(vm.native_data_of(&args))
}

fn eval_function(vm: &mut Vm, args: Args) -> Run<Value> {
    let source = vm.arg(&args, 0);
    if !source.is_string() {
        return Ok(source);
    }
    let source = vm.text_of(source).to_rust();
    vm.run_eval(&source)
}

fn inspect_function(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let text = crate::builtins::inspect_value(vm, value);
    Ok(vm.new_str(&text))
}

/// What a module name turned out to be.
enum Resolved {
    /// One that comes with the engine, already run: its `module` object.
    Builtin(Value),
    File(PathBuf),
}

/// A file's code, compiled.
pub(crate) enum FileCode {
    Module(u32),
    CommonJs(u32),
}

impl FileCode {
    fn of(proto: u32, is_module: bool) -> Self {
        if is_module {
            Self::Module(proto)
        } else {
            Self::CommonJs(proto)
        }
    }
}

impl Vm {
    /// Finds what a name in an `import` or `require` from `dir` is, running a module that comes with the engine if
    /// that's what it is.
    fn resolve_module(&mut self, dir: &Path, name: &str, is_import: bool) -> Run<Resolved> {
        // A `file:` URL is the file it names, as Node has it.
        let name = file_url_path(name.to_owned());
        let bare = name.strip_prefix("node:").unwrap_or(&name).to_owned();
        if let Some(module) = self.load_builtin_module(&bare)? {
            return Ok(Resolved::Builtin(module));
        }
        let key = (dir.to_owned(), bare, is_import);
        if let Some(path) = self.modules.resolved.get(&key) {
            return Ok(Resolved::File(path.clone()));
        }
        let _time = crate::startup::span(crate::startup::Phase::Resolve, || None);
        // A packed program knows what the names it asked for when it was packed were found to be (codecache.rs).
        if let Some(path) = self.cached_resolution(dir, &key.1, is_import) {
            self.modules.resolved.insert(key, path.clone());
            return Ok(Resolved::File(path));
        }
        let found = match self.resolve_file(dir, &key.1, is_import) {
            Ok(found) => found,
            // What's in a folder may have changed since it was looked at, as when a program writes a file and then
            // imports it, so what isn't found is looked for again on the disk itself.
            Err(_) => {
                ASK_THE_DISK.with(|ask| ask.set(true));
                let found = self.resolve_file(dir, &key.1, is_import);
                ASK_THE_DISK.with(|ask| ask.set(false));
                found?
            }
        };
        let path = canonical(&found);
        self.modules.resolved.insert(key, path.clone());
        Ok(Resolved::File(path))
    }

    /// `import.meta` for the module in `file`, the same object each time, with what Node 24 puts on it, in its order:
    /// `dirname` and `filename`, the folder and the file as paths, `main`, whether it's the program's first module,
    /// `resolve`, and `url`, the file as a `file:` URL, as `url.pathToFileURL` writes it.
    fn import_meta(&mut self, file: &str) -> Value {
        if let Some(found) = self.modules.metas.get(file) {
            return *found;
        }
        let meta = self.new_object_with_proto(NONE, Kind::Plain);
        let dir = Path::new(file).parent().unwrap_or(Path::new("."));
        let dirname = self.new_str(&dir.to_string_lossy());
        let filename = self.new_str(file);
        let is_main = self
            .modules
            .main
            .as_deref()
            .is_some_and(|main| main.as_os_str() == std::ffi::OsStr::new(file));
        let resolve_name = self.intern_str("resolve");
        let resolve = self.new_native(resolve_name, 1, meta_resolve, filename, false);
        let url = self.new_str(&file_url(file));
        let parts = [
            ("dirname", dirname),
            ("filename", filename),
            ("main", Value::bool(is_main)),
            ("resolve", resolve),
            ("url", url),
            // Only when what's running the program has said what it is, as Vite does for an app's code.
            ("env", self.modules.meta_env.unwrap_or(Value::UNDEFINED)),
        ];
        for (name, value) in parts {
            if value.is_undefined() {
                continue;
            }
            let key = self.intern_str(name);
            self.define(meta.handle(), key, value, PLAIN);
        }
        self.modules.metas.insert(file.to_owned(), meta);
        meta
    }
}

/// Whether a character in a path is written as `%` and its UTF-8 in a `file:` URL, as Node's `pathToFileURL` writes
/// it: the controls, space, what a URL would take to mean something else, and everything that isn't ASCII.
fn escaped_in_file_url(character: char) -> bool {
    !character.is_ascii()
        || character <= ' '
        || character == '\u{7f}'
        || matches!(
            character,
            '"' | '#' | '%' | '<' | '>' | '?' | '[' | ']' | '^' | '`' | '{' | '|' | '}' | '~'
        )
}

/// A file's source as it's compiled: without a byte order mark, with a first line for running it as a script made a
/// comment, and through the source filter, if there is one. It's read this way again for a function's text.
pub(crate) fn read_source(path: &Path) -> std::io::Result<String> {
    let reading = crate::startup::span(crate::startup::Phase::Read, || None);
    let mut source = match crate::source_overlay::source(path) {
        Some(source) => source,
        None => crate::packed::read_file(path)?,
    };
    drop(reading);
    if source.starts_with('\u{FEFF}') {
        source.drain(..'\u{FEFF}'.len_utf8());
    }
    // A first line for running as a script isn't JavaScript.
    if source.starts_with("#!") {
        source.insert_str(0, "//");
    }
    crate::inspect::filter_source(path, &mut source);
    Ok(source)
}

/// A path as a `file:` URL, as Node's `pathToFileURL` writes it: `file:///C:/a%20b/c.js`, or `file://server/share/c.js`
/// for a network share.
pub(crate) fn file_url(path: &str) -> String {
    let mut out = String::from("file://");
    let mut rest = path;
    if let Some(share) = path.strip_prefix(r"\\") {
        // A network share's machine is the URL's host, which is written in lower case.
        let share = share.strip_prefix(r"?\UNC\").unwrap_or(share);
        let end = share.find('\\').unwrap_or(share.len());
        out.push_str(&share[..end].to_ascii_lowercase());
        rest = &share[end..];
    } else if !path.starts_with(['/', '\\']) {
        out.push('/');
    }
    let mut bytes = [0; 4];
    for character in rest.chars() {
        if character == '\\' {
            out.push('/');
        } else if escaped_in_file_url(character) {
            for byte in character.encode_utf8(&mut bytes).bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        } else {
            out.push(character);
        }
    }
    out
}

/// The number of the module a native was made for.
fn record_of(vm: &Vm, args: &Args) -> u32 {
    vm.native_data_of(args).as_number() as u32
}

/// `%bind(cell, number)`: a module's first step handing over the cell of one of its own variables that it exports.
fn bind_function(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = record_of(vm, &args);
    let cell = vm.arg(&args, 0);
    let at = vm.arg(&args, 1).as_number() as usize;
    if let Some(record) = vm.modules.records.get_mut(&id) {
        if let Some(slot) = record.cells.get_mut(at) {
            *slot = cell;
        }
    }
    Ok(Value::UNDEFINED)
}

/// Whether a module is TypeScript, whose imports of types are left out.
fn is_typescript(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ending| ending.to_str()),
        Some("ts" | "tsx" | "mts" | "cts")
    )
}

/// `%import(number)`: the cell for one of a module's imports, in its second step.
fn import_binding(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = record_of(vm, &args);
    let entry = vm.arg(&args, 0).as_number() as usize;
    let Some(record) = vm.modules.records.get(&id) else {
        return Ok(Value::UNDEFINED);
    };
    let (dep, name) = record.info.imports[entry].clone();
    let from = record.info.requested[dep].clone();
    let typescript = is_typescript(&record.path);
    match vm.binding_from(id, dep, name.clone(), &mut Vec::new())? {
        Found::Cell(cell) => Ok(cell),
        Found::Missing if typescript => Ok(vm.type_cell(id, name.as_deref().unwrap_or_default())),
        Found::Missing => Err(vm.syntax_error(&format!(
            "The requested module '{from}' does not provide an export named '{}'",
            name.unwrap_or_default()
        ))),
        Found::Ambiguous => Err(vm.syntax_error(&format!(
            "The requested module '{from}' contains conflicting star exports for name '{}'",
            name.unwrap_or_default()
        ))),
    }
}

/// What a module's namespace gives for a name: what's in its cell now.
fn binding_getter(vm: &mut Vm, args: Args) -> Run<Value> {
    let cell = vm.native_data_of(&args);
    let value = vm.cells[cell.index()];
    if value.is_empty() {
        return Err(vm.reference_error("Cannot access a variable before initialization"));
    }
    Ok(value)
}

/// One of the modules a module was waiting for has finished.
fn dependency_done(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = record_of(vm, &args);
    let Some(record) = vm.modules.records.get_mut(&id) else {
        return Ok(Value::UNDEFINED);
    };
    record.waiting = record.waiting.saturating_sub(1);
    if record.waiting == 0 && record.state == State::Waiting {
        // What goes wrong from here breaks its promise, which what's waiting for it hears about.
        let _ = vm.run_module(id, Resume::Next, Value::UNDEFINED);
    }
    Ok(Value::UNDEFINED)
}

fn dependency_failed(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = record_of(vm, &args);
    let error = vm.arg(&args, 0);
    if vm
        .modules
        .records
        .get(&id)
        .is_some_and(|record| record.state == State::Waiting)
    {
        let _ = vm.module_failed(id, error);
    }
    Ok(Value::UNDEFINED)
}

/// What a module was waiting for at its top came.
fn module_resumed(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = record_of(vm, &args);
    let value = vm.arg(&args, 0);
    let _ = vm.run_module(id, Resume::Next, value);
    Ok(Value::UNDEFINED)
}

fn module_rejected(vm: &mut Vm, args: Args) -> Run<Value> {
    let id = record_of(vm, &args);
    let error = vm.arg(&args, 0);
    let _ = vm.run_module(id, Resume::Throw, error);
    Ok(Value::UNDEFINED)
}

/// `%meta(file)`, which is what `import.meta` is.
fn meta_function(vm: &mut Vm, args: Args) -> Run<Value> {
    let file = vm.arg(&args, 0);
    let file = if file.is_symbol() {
        vm.atom_text(Atom(file.handle()))
    } else {
        vm.to_rust_string(file)?
    };
    Ok(vm.import_meta(&file))
}

/// `import.meta.resolve(name)`: where an import of it would come from, as a URL, without loading it, as Node 24 says:
/// `node:fs` for what comes with Node, a URL or a path that starts with `/`, `./` or `../` as a URL from the module's own,
/// whether there's a file there or not, and a package's file as it's found, or an error with the code
/// `ERR_MODULE_NOT_FOUND` if it isn't.
fn meta_resolve(vm: &mut Vm, args: Args) -> Run<Value> {
    let name = vm.arg(&args, 0);
    let name = vm.to_rust_string(name)?;
    let file = vm.native_data_of(&args);
    let file = vm.text_of(file).to_rust();
    let bare = name.strip_prefix("node:").unwrap_or(&name);
    if node_source(bare).is_some() || vm.modules.host.iter().any(|(host, _)| *host == bare) {
        return Ok(vm.new_str(&format!("node:{bare}")));
    }
    if is_url_or_relative(&name) {
        let constructor = vm.get_named(Value::object(vm.global), "URL")?;
        let specifier = vm.new_str(&name);
        let base = vm.new_str(&file_url(&file));
        let url = vm.construct(constructor, &[specifier, base])?;
        return vm.get_named(url, "href");
    }
    let dir = Path::new(&file)
        .parent()
        .unwrap_or(Path::new("."))
        .to_owned();
    let found = match vm.resolve_file(&dir, bare, true) {
        Ok(found) => found,
        Err(error) => {
            if error.is_object() {
                let code = vm.new_str("ERR_MODULE_NOT_FOUND");
                let atom = vm.intern_str("code");
                vm.define(error.handle(), atom, code, PLAIN);
            }
            return Err(error);
        }
    };
    let path = canonical(&found);
    Ok(vm.new_str(&file_url(&path.to_string_lossy())))
}

/// Whether an import names a URL, or a place from the module that imports it, rather than a package.
fn is_url_or_relative(name: &str) -> bool {
    if name.starts_with('/')
        || name.starts_with("./")
        || name.starts_with("../")
        || name == "."
        || name == ".."
    {
        return true;
    }
    // A URL's scheme: a letter, then letters, digits, `+`, `-` and `.`, up to a colon.
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters
            .take_while(|character| *character != ':')
            .all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
            })
        && name.contains(':')
}

/// Whether the functions of a module that comes with the engine give their text from `toString`, as Node's own
/// JavaScript does. What's the language's own, as `Intl` is, says it's built in, as the language has it.
fn shows_its_text(name: &str) -> bool {
    name != "internal/intl"
}

/// The source of a module that comes with Node, as this engine has it, by the name it's imported by.
pub(crate) fn node_source(name: &str) -> Option<&'static str> {
    Some(match name {
        "events" => include_str!("node/events.js"),
        "path" | "path/posix" | "path/win32" => include_str!("node/path.js"),
        "internal/path-js" => include_str!("node/path_js.js"),
        "util" => include_str!("node/util.js"),
        "assert" | "assert/strict" => include_str!("node/assert.js"),
        "fs" => include_str!("node/fs.js"),
        "internal/fs-more" => include_str!("node/fs_more.js"),
        "fs/promises" => "module.exports = require('fs').promises;",
        "internal/fetch" => include_str!("node/fetch.js"),
        "internal/web" => include_str!("node/web.js"),
        "internal/url" => include_str!("node/url.js"),
        "internal/event-target" => include_str!("node/event_target.js"),
        "internal/buffer" => include_str!("node/buffer.js"),
        "internal/intl" => include_str!("node/intl.js"),
        "internal/env" => {
            "module.exports = (names) => {
                // Each name by how it's written in lower case, which is made once, as libraries ask for names that
                // aren't there a lot, and looking through them all each time would be slow.
                let lower = null;
                const byLower = () => {
                    if (lower === null) {
                        lower = new Map();
                        for (const one of Object.keys(names)) {
                            lower.set(one.toLowerCase(), one);
                        }
                    }
                    return lower;
                };
                // Whatever case it's asked for in, this is the case it's written in.
                const actual = (name) => {
                    if (typeof name !== 'string' || name in names) {
                        return name;
                    }
                    return byLower().get(name.toLowerCase()) ?? name;
                };
                return new Proxy(names, {
                    get: (target, name) => target[actual(name)],
                    has: (target, name) => actual(name) in target,
                    set: (target, name, value) => {
                        const written = actual(name);
                        if (typeof written === 'string' && lower !== null && !(written in target)) {
                            lower.set(written.toLowerCase(), written);
                        }
                        target[written] = String(value);
                        return true;
                    },
                    deleteProperty: (target, name) => {
                        const written = actual(name);
                        if (typeof written === 'string' && lower !== null) {
                            lower.delete(written.toLowerCase());
                        }
                        return delete target[written];
                    },
                    getOwnPropertyDescriptor: (target, name) => Object.getOwnPropertyDescriptor(target, actual(name)),
                });
            };"
        }
        "internal/more" => include_str!("node/more.js"),
        "internal/misc" => include_str!("node/misc.js"),
        "internal/standard-streams" => include_str!("node/standard_streams.js"),
        "stream" => include_str!("node/stream.js"),
        "stream/promises" => "module.exports = require('stream').promises;",
        "stream/web" => "const { internals, ...web } = require('internal/misc/web_streams'); module.exports = web;",
        "child_process" => include_str!("node/child_process.js"),
        "os" => "module.exports = require('internal/misc')('os');",
        "crypto" => "module.exports = require('internal/misc')('crypto');",
        "querystring" => "module.exports = require('internal/misc')('querystring');",
        "string_decoder" => "module.exports = require('internal/misc')('string_decoder');",
        "timers" => "module.exports = require('internal/misc')('timers');",
        "module" => "module.exports = require('internal/misc')('module');",
        "tty" => "module.exports = require('internal/misc')('tty');",
        "punycode" => "module.exports = require('internal/misc')('punycode');",
        "perf_hooks" => "module.exports = require('internal/misc')('perf_hooks');",
        "net" => "module.exports = require('internal/net').net;",
        "internal/net" => include_str!("node/net.js"),
        "internal/websocket" => include_str!("node/websocket.js"),
        "worker_threads" => include_str!("node/worker.js"),
        "internal/worker-global" => "module.exports = { Worker: require('worker_threads').WebWorker };",
        "dns" => "module.exports = require('internal/net').dns;",
        "dns/promises" => "module.exports = require('dns').promises;",
        "dgram" => "module.exports = require('internal/net').dgram;",
        "zlib" => include_str!("node/zlib.js"),
        "internal/http" => include_str!("node/http.js"),
        "http" => "module.exports = require('internal/http').http;",
        "https" => "module.exports = require('internal/http').https;",
        "timers/promises" => "module.exports = require('timers').promises;",
        "util/types" => "module.exports = require('util').types;",
        "process" => "module.exports = process;",
        "console" => "module.exports = console;",
        "constants" => "module.exports = require('os').constants;",
        "url" => include_str!("node/url_node.js"),
        "buffer" => {
            "const { Buffer } = require('internal/buffer');
            module.exports = { Buffer, SlowBuffer: Buffer, kMaxLength: 2 ** 53 - 1, kStringMaxLength: 2 ** 29 - 24, constants: { MAX_LENGTH: 2 ** 53 - 1, MAX_STRING_LENGTH: 2 ** 29 - 24 }, atob, btoa, get Blob() { return Blob; }, get File() { return File; } };"
        }
        _ if name.starts_with("internal/misc/") => return crate::builtins::host::misc_source(name),
        _ => return None,
})
}

/// What a module that comes with the engine is called among the files that are read ahead. See [`crate::ahead`].
pub(crate) fn builtin_path(name: &str) -> PathBuf {
    PathBuf::from(format!("{BUILTIN}{name}"))
}

/// What the names of modules that come with the engine start with, among files.
pub(crate) const BUILTIN: &str = "builtin:";

/// The modules a program gave the engine, for reading ahead before there's an engine to give them to.
static KNOWN_HOST: std::sync::Mutex<Vec<(&'static str, &'static str)>> =
    std::sync::Mutex::new(Vec::new());

/// Says what a module the program gives the engine is, before it does.
pub(crate) fn know_host_module(name: &'static str, source: &'static str) {
    if let Ok(mut known) = KNOWN_HOST.lock() {
        if !known.iter().any(|(known, _)| *known == name) {
            known.push((name, source));
        }
    }
}

/// The source of a module that comes with the engine, or that the program gives it, by the name it's imported by.
pub(crate) fn builtin_source(name: &str) -> Option<&'static str> {
    let host = KNOWN_HOST.lock().ok().and_then(|known| {
        known
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, source)| *source)
    });
    host.or_else(|| node_source(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_split_from_their_paths() {
        assert_eq!(split_package("react"), ("react", None));
        assert_eq!(split_package("react/jsx"), ("react", Some("jsx")));
        assert_eq!(split_package("@a/b"), ("@a/b", None));
        assert_eq!(split_package("@a/b/c/d"), ("@a/b", Some("c/d")));
    }

    #[test]
    fn tsconfig_text_loses_its_comments_and_trailing_commas() {
        let text = "{\n  // shadcn's alias\n  \"compilerOptions\": { /* see */ \"paths\": { \"@/*\": [\"./src/*\",], }, \"x\": \"a//b\" },\n}";
        assert_eq!(
            strip_jsonc(text),
            "{\n  \n  \"compilerOptions\": {  \"paths\": { \"@/*\": [\"./src/*\"] }, \"x\": \"a//b\" }\n}"
        );
        assert_eq!(
            alias_match("@/*", "@/components/ui/button"),
            Some("components/ui/button")
        );
        assert_eq!(alias_match("@/*", "@radix-ui/react-slot"), None);
        assert_eq!(alias_match("~lib", "~lib"), Some(""));
    }

    /// shadcn/ui's components import each other as `@/components/ui/...` and `@/lib/utils`, which the app's tsconfig.json says
    /// is its `src` folder.
    #[test]
    fn names_go_where_tsconfig_paths_say() {
        let dir =
            std::env::temp_dir().join(format!("rsui_js_tsconfig_paths_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for folder in [
            "src/lib",
            "src/components/ui",
            "node_modules/@/lib",
            "node_modules/pkg",
            "base/shared",
        ] {
            std::fs::create_dir_all(dir.join(folder)).unwrap();
        }
        let files = [
            (
                "src/lib/utils.ts",
                "export const cn = (...a: string[]) => a.join(' ');",
            ),
            (
                "src/components/ui/button.tsx",
                "import { cn } from '@/lib/utils';\nexport const button = cn('a', 'b');",
            ),
            (
                "src/main.ts",
                "import { button } from '@/components/ui/button';\nimport { cn } from 'exact';\nimport pkg from 'pkg';\nimport shared from 'shared/thing';\nconsole.log(button, cn('c'), pkg, shared);",
            ),
            // A package's own names never go by the app's paths.
            (
                "node_modules/@/lib/utils.js",
                "export const cn = () => 'wrong';",
            ),
            (
                "node_modules/pkg/package.json",
                "{ \"main\": \"index.js\" }",
            ),
            ("node_modules/pkg/index.js", "module.exports = 'pkg';"),
            (
                "base/tsconfig.json",
                "{ \"compilerOptions\": { \"baseUrl\": \".\" } }",
            ),
            ("base/shared/thing.ts", "export default 'shared';"),
        ];
        for (name, text) in files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        // As shadcn's Vite guide writes it, with comments, and `extends` bringing a `baseUrl`, from where that file is.
        std::fs::write(
            dir.join("tsconfig.json"),
            "{\n  \"files\": [],\n  // the alias\n  \"extends\": \"./base/tsconfig\",\n  \"compilerOptions\": { \"paths\": { \"@/*\": [\"../src/*\"], \"exact\": [\"../src/lib/utils.ts\"] } },\n}",
        )
        .unwrap();
        let output = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let mut engine = crate::Engine::new();
        let sink = output.clone();
        engine.set_printer(move |text| sink.borrow_mut().push_str(text));
        let result = engine
            .run_file(&dir.join("src/main.ts"))
            .map(|_| ())
            .and_then(|()| engine.run_until_idle())
            .map_err(|error| error.message);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(result, Ok(()));
        assert_eq!(output.borrow().trim_end(), "a b c pkg shared");
    }
}

#[cfg(test)]
mod reading_files_once {
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Runs `main.js` among these files, and gives what it printed or the error, and how many times code was read.
    fn run(files: &[(&str, &str)]) -> (Result<String, String>, usize) {
        let dir = std::env::temp_dir().join(format!(
            "rsui_js_read_once_{}_{}",
            std::process::id(),
            files[0].1.len()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let output = Rc::new(RefCell::new(String::new()));
        let mut engine = crate::Engine::new();
        let sink = output.clone();
        engine.set_printer(move |text| sink.borrow_mut().push_str(text));
        let before = crate::parser::PARSES.with(std::cell::Cell::get);
        let result = engine
            .run_file(&dir.join("main.js"))
            .map(|_| ())
            .and_then(|()| engine.run_until_idle())
            .map_err(|error| error.message);
        let parses = crate::parser::PARSES.with(std::cell::Cell::get) - before;
        std::fs::remove_dir_all(&dir).ok();
        let shown = output.borrow().replace("\r\n", "\n").trim_end().to_owned();
        (result.map(|()| shown), parses)
    }

    #[test]
    fn commonjs_that_says_import_is_read_once() {
        // `console` is built in, and printing doesn't read any more code.
        let (result, parses) = run(&[(
            "main.js",
            "// This doesn't import anything.\nconst value = typeof require + ' ' + typeof __dirname;\nconsole.log(value);\nif (value) return;\nconsole.log('not here');",
        )]);
        assert_eq!(result.as_deref(), Ok("function string"));
        assert_eq!(parses, 1);
    }

    #[test]
    fn a_module_that_exports_at_its_end_is_read_once() {
        let (result, parses) = run(&[(
            "main.js",
            "const seen = typeof require;\nfunction show() { console.log(seen, typeof module, this === undefined); }\nshow();\nexport default show;",
        )]);
        assert_eq!(result.as_deref(), Ok("undefined undefined true"));
        assert_eq!(parses, 1);
    }

    #[test]
    fn what_only_a_function_can_do_still_isnt_a_module() {
        // A module can't `return`, wherever its `export` is.
        let (result, _) = run(&[("main.js", "if (globalThis) return;\nexport const a = 1;")]);
        assert!(result.is_err(), "{result:?}");
        let (result, _) = run(&[("main.js", "export const a = 1;\nreturn;")]);
        assert!(result.is_err(), "{result:?}");
        // CommonJS can't declare what it's given again, but a module can.
        let (result, _) = run(&[(
            "main.js",
            "let require = 1; // import\nconsole.log(require);",
        )]);
        assert!(result.is_err(), "{result:?}");
        let (result, _) = run(&[(
            "main.js",
            "let require = 1;\nexport {};\nconsole.log(require);",
        )]);
        assert_eq!(result.as_deref(), Ok("1"));
        // An import that's only a type makes a module, as TypeScript has it, though it's gone once types are.
        let (result, _) = run(&[(
            "main.js",
            "import type { A } from './a';\nconsole.log(typeof module);",
        )]);
        assert_eq!(result.as_deref(), Ok("undefined"));
    }
}

#[cfg(test)]
mod deferred_reexports {
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A package `pkg` whose main file passes on `export *` from `all.js`, which passes on the default exports of
    /// `a.js`, `b.js` and `c.js`, each of which says when it runs. `side_effects` is what its `package.json` says.
    fn package(side_effects: &str) -> Vec<(String, String)> {
        let mut files = vec![
            (
                "node_modules/pkg/package.json".to_owned(),
                format!(r#"{{ "name": "pkg", "type": "module", "main": "index.js"{side_effects} }}"#),
            ),
            ("node_modules/pkg/index.js".to_owned(), "export * from './all.js';\n".to_owned()),
            (
                "node_modules/pkg/all.js".to_owned(),
                "// Made by a script.\nexport { default as A } from './a.js';\nexport { default as B } from './b.js';\nexport { default as C } from './c.js';\n".to_owned(),
            ),
        ];
        for name in ["a", "b", "c"] {
            files.push((
                format!("node_modules/pkg/{name}.js"),
                format!("console.log('{name} ran');\nexport default '{name}';\n"),
            ));
        }
        files
    }

    /// Runs `main.js` among these files, and gives what it printed or the error, and how many files were read.
    fn run(files: &[(String, String)]) -> (Result<String, String>, usize) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static RUNS: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rsui_js_deferred_{}_{}",
            std::process::id(),
            RUNS.fetch_add(1, Ordering::Relaxed)
        ));
        for (name, text) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let output = Rc::new(RefCell::new(String::new()));
        let mut engine = crate::Engine::new();
        let sink = output.clone();
        engine.set_printer(move |text| sink.borrow_mut().push_str(text));
        let before = crate::parser::PARSES.with(std::cell::Cell::get);
        let result = engine
            .run_file(&dir.join("main.js"))
            .map(|_| ())
            .and_then(|()| engine.run_until_idle())
            .map_err(|error| error.message);
        let parses = crate::parser::PARSES.with(std::cell::Cell::get) - before;
        std::fs::remove_dir_all(&dir).ok();
        let shown = output.borrow().replace("\r\n", "\n").trim_end().to_owned();
        (result.map(|()| shown), parses)
    }

    fn with_main(mut files: Vec<(String, String)>, main: &str) -> Vec<(String, String)> {
        files.push(("main.js".to_owned(), main.to_owned()));
        files
    }

    #[test]
    fn one_name_loads_only_its_module_from_a_package_without_side_effects() {
        let files = with_main(package(r#", "sideEffects": false"#), "import { B } from 'pkg';\nconsole.log(B);\n");
        let (result, parses) = run(&files);
        assert_eq!(result.as_deref(), Ok("b ran\nb"));
        // `main.js`, `index.js`, `all.js` and `b.js`.
        assert_eq!(parses, 4);
    }

    #[test]
    fn a_package_that_may_have_side_effects_is_all_run_in_order() {
        for said in ["", r#", "sideEffects": true"#] {
            let files = with_main(package(said), "import { B } from 'pkg';\nconsole.log(B);\n");
            let (result, parses) = run(&files);
            assert_eq!(result.as_deref(), Ok("a ran\nb ran\nc ran\nb"), "{said}");
            assert_eq!(parses, 6);
        }
    }

    #[test]
    fn files_that_side_effects_names_are_run() {
        let files = with_main(package(r#", "sideEffects": ["./c.js"]"#), "import { B } from 'pkg';\nconsole.log(B);\n");
        let (result, _) = run(&files);
        assert_eq!(result.as_deref(), Ok("b ran\nc ran\nb"));
        let files = with_main(package(r#", "sideEffects": ["*.css", "**/a.js"]"#), "import { C } from 'pkg';\nconsole.log(C);\n");
        let (result, _) = run(&files);
        assert_eq!(result.as_deref(), Ok("a ran\nc ran\nc"));
    }

    #[test]
    fn a_namespace_has_everything() {
        let files = with_main(
            package(r#", "sideEffects": false"#),
            "import * as all from 'pkg';\nconsole.log(Object.keys(all).join(), all.C);\n",
        );
        let (result, _) = run(&files);
        assert_eq!(result.as_deref(), Ok("a ran\nb ran\nc ran\nA,B,C c"));
    }

    #[test]
    fn importing_the_package_later_runs_the_rest_then() {
        let files = with_main(
            package(r#", "sideEffects": false"#),
            "import { A } from 'pkg';\nconsole.log(A);\nconst all = await import('pkg');\nconsole.log(Object.keys(all).join(), all.B, all.C);\n",
        );
        let (result, _) = run(&files);
        assert_eq!(result.as_deref(), Ok("a ran\na\nb ran\nc ran\nA,B,C b c"));
        // A module imported later asks for another name, which loads its module then.
        let mut files = with_main(
            package(r#", "sideEffects": false"#),
            "import { A } from 'pkg';\nconsole.log(A);\nconst { C } = await import('./more.js');\nconsole.log(C);\n",
        );
        files.push(("more.js".to_owned(), "export { C } from 'pkg';\n".to_owned()));
        let (result, _) = run(&files);
        assert_eq!(result.as_deref(), Ok("a ran\na\nc ran\nc"));
    }

    #[test]
    fn what_is_passed_on_stays_live() {
        let mut files = package(r#", "sideEffects": false"#);
        files.push((
            "node_modules/pkg/count.js".to_owned(),
            "export let count = 0;\nexport function bump() { count += 1; }\n".to_owned(),
        ));
        files[2].1.push_str("export { count, bump } from './count.js';\n");
        let files = with_main(files, "import { count, bump } from 'pkg';\nbump();\nbump();\nconsole.log(count);\n");
        let (result, _) = run(&files);
        assert_eq!(result.as_deref(), Ok("2"));
    }

    #[test]
    fn modules_can_import_each_other_through_the_package() {
        let mut files = package(r#", "sideEffects": false"#);
        // `b.js` uses `A` through the package's main file, which is being loaded as it's asked for.
        files[4].1 = "import { A } from './index.js';\nconsole.log('b ran');\nexport default () => 'b with ' + A;\n".to_owned();
        let files = with_main(files, "import { B } from 'pkg';\nconsole.log(B());\n");
        let (result, _) = run(&files);
        assert_eq!(result.as_deref(), Ok("a ran\nb ran\nb with a"));
    }

    #[test]
    fn names_two_stars_give_are_ambiguous_only_when_imported() {
        let files = vec![
            (
                "node_modules/two/package.json".to_owned(),
                r#"{ "name": "two", "type": "module", "main": "index.js", "sideEffects": false }"#.to_owned(),
            ),
            ("node_modules/two/index.js".to_owned(), "export * from './x.js';\nexport * from './y.js';\n".to_owned()),
            ("node_modules/two/x.js".to_owned(), "export const same = 'x';\nexport const onlyX = 'x';\n".to_owned()),
            ("node_modules/two/y.js".to_owned(), "export const same = 'y';\nexport const onlyY = 'y';\n".to_owned()),
        ];
        let (result, _) = run(&with_main(files.clone(), "import { onlyX, onlyY } from 'two';\nconsole.log(onlyX, onlyY);\n"));
        assert_eq!(result.as_deref(), Ok("x y"));
        let (result, _) = run(&with_main(files.clone(), "import { same } from 'two';\nconsole.log(same);\n"));
        assert!(result.as_ref().is_err_and(|error| error.contains("conflicting star exports")), "{result:?}");
        let (result, _) = run(&with_main(files.clone(), "import * as all from 'two';\nconsole.log(Object.keys(all).join());\n"));
        assert_eq!(result.as_deref(), Ok("onlyX,onlyY"));
        let (result, _) = run(&with_main(files, "import { nothing } from 'two';\nconsole.log(nothing);\n"));
        assert!(result.as_ref().is_err_and(|error| error.contains("does not provide an export named 'nothing'")), "{result:?}");
    }

    #[test]
    fn require_gives_all_of_it() {
        let mut files = with_main(
            package(r#", "sideEffects": false"#),
            "import { A } from 'pkg';\nimport { names } from './old.cjs';\nconsole.log(A, names);\n",
        );
        files.push((
            "old.cjs".to_owned(),
            "exports.names = Object.keys(require('pkg')).filter((name) => name !== '__esModule').join();\n".to_owned(),
        ));
        let (result, _) = run(&files);
        assert_eq!(result.as_deref(), Ok("a ran\nb ran\nc ran\na A,B,C"));
    }
}
