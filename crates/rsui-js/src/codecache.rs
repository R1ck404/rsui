//! Compiled code kept inside a packed program, so that it starts without compiling what it compiled when it was packed.
//!
//! Packing an app (`rsui-tsx --pack`) runs its main file, and with [`Vm::keep_compiled_code`] every file that's compiled
//! while it does is written down as the compiler made it. They go on the end of the packed program as one block, beside
//! the files themselves (packed.rs). When the program starts, a file that's in the block is loaded from it rather than
//! parsed and compiled. Its top level still runs, as it would have, so nothing an app does as it starts is decided when
//! it's packed: only what compiling its text would have made is.
//!
//! The block is for one build of the engine, [`BUILD_ID`], which build.rs makes from everything in this crate, and it's
//! only used by that build. Anything that doesn't check out, as a block made by another build or one that's damaged, means
//! the file is compiled as it would have been, as is a file that's being debugged, filtered or compiled lazily.
//!
//! What's numbered for one engine is written as what it is, and numbered again when it's loaded:
//!
//! - names (atoms) by their text, in a table the whole block shares; the well-known ones by their number, which every
//!   engine gives them; a class's `#names` as new ones each time, as compiling makes them; the engine's own hidden
//!   names (`%meta`) by their name;
//! - constants by their value: numbers by their bits, strings by their text, template objects and BigInts by what
//!   they're made from, and the two functions the compiler uses as constants by which they are;
//! - functions by where they are in the file.
//!
//! Instructions are written field by field, little-endian, by code build.rs makes from op.rs (`op_codec.rs`), so how
//! they're laid out in memory doesn't matter. The kinds of instruction that hold a number belonging to one engine are
//! [`numbered`]'s, and its test fails for a new kind it doesn't know.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::modules::{ExportFrom, ModuleInfo};
use crate::op::{Cache, Capture, Handler, Op, Proto};
use crate::source_text::{NO_SOURCE, SourceText};
use crate::text::{Atom, AtomKind, Text};
use crate::value::Value;
use crate::vm::Vm;

include!(concat!(env!("OUT_DIR"), "/build_id.rs"));

/// Compiler diagnostics must not load bytecode made with a different option.
/// Keep the default identity unchanged; the disabled pass has its own identity.
fn compiler_build_id() -> u64 {
    if crate::compiler::const_value_captures_enabled() {
        BUILD_ID
    } else {
        BUILD_ID ^ 0xc047_ca97_0000_0001
    }
}

mod codec {
    use crate::op::Op;
    include!(concat!(env!("OUT_DIR"), "/op_codec.rs"));
}
#[cfg(test)]
pub(crate) use codec::OP_KINDS;
pub(crate) use codec::{read_op, write_op};

const MAGIC: &[u8; 8] = b"RSUICODE";
/// Changes whenever how the block is written does, though [`BUILD_ID`] would too.
const FORMAT: u32 = 2;

// How an atom is written.
const ATOM_INDEX: u8 = 0;
const ATOM_FIXED: u8 = 1;
const ATOM_NAME: u8 = 2;
const ATOM_NAMED_PRIVATE: u8 = 3;
const ATOM_NEW_PRIVATE: u8 = 4;

// How a constant is written.
const CONST_BITS: u8 = 0;
const CONST_STRING: u8 = 1;
const CONST_SYMBOL: u8 = 2;
const CONST_TEMPLATE: u8 = 3;
const CONST_BIGINT: u8 = 4;
const CONST_DISPOSAL: u8 = 5;
const CONST_NAME_FUNCTION: u8 = 6;
const CONST_FILE: u8 = 7;

/// What's numbered for one engine in an instruction: a constant (with whether it has to fit 16 bits), an atom, or a
/// function. Every other field is the same in any engine: registers, jumps (which are relative), places in the function's
/// own list of property caches, captures, small numbers, and numbers the engine itself fixes, as `TYPE_` kinds and
/// `CallMath`'s place in `builtins::math::QUICK`.
pub(crate) enum Numbered<'a> {
    Const32(&'a mut u32),
    Const16(&'a mut u16),
    Atom(&'a mut u32),
    Proto(&'a mut u32),
    Nothing,
}

/// What in `op` is numbered for one engine. A new kind of instruction that holds such a number has to be added here: the
/// test `every_numbered_field_is_remapped` fails until it is.
pub(crate) fn numbered(op: &mut Op) -> Numbered<'_> {
    match op {
        Op::LoadConst { index, .. } => Numbered::Const32(index),
        Op::AddK { k, .. }
        | Op::SubK { k, .. }
        | Op::MulK { k, .. }
        | Op::DivK { k, .. }
        | Op::ModK { k, .. } => Numbered::Const16(k),
        Op::DeclareGlobalVar { name, .. }
        | Op::DeclareGlobalFunction { name, .. }
        | Op::ThrowConstAssign { name, .. }
        | Op::ThrowTdz { name, .. } => Numbered::Atom(name),
        Op::Closure { proto, .. } => Numbered::Proto(proto),
        _ => Numbered::Nothing,
    }
}

// Writing and reading.

#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    fn u16(&mut self, value: u16) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    fn count(&mut self, count: usize) -> Option<()> {
        self.u32(u32::try_from(count).ok()?);
        Some(())
    }
    fn text(&mut self, text: &str) {
        self.u32(text.len() as u32);
        self.0.extend_from_slice(text.as_bytes());
    }
}

struct In<'a>(&'a [u8]);

impl<'a> In<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let (taken, rest) = self.0.split_at_checked(count)?;
        self.0 = rest;
        Some(taken)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    /// A count of things, each at least a byte, so it can't be more than what's left.
    fn count(&mut self) -> Option<usize> {
        let count = self.u32()? as usize;
        (count <= self.0.len()).then_some(count)
    }
    fn text(&mut self) -> Option<&'a str> {
        let count = self.count()?;
        std::str::from_utf8(self.take(count)?).ok()
    }
}

/// A sample of a module that comes with the program, to tell whether it's the one that was packed: its length, and its
/// first and last 256 bytes. Its whole text isn't hashed, as that would take as long as some loads; the block's build
/// and the program it's in are the same by the time this is asked anyway.
fn sample(source: &str) -> u64 {
    let bytes = source.as_bytes();
    let head = &bytes[..bytes.len().min(256)];
    let tail = &bytes[bytes.len().saturating_sub(256)..];
    let mut hash = 0xcbf2_9ce4_8422_2325u64 ^ bytes.len() as u64;
    for byte in head.iter().chain(tail) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

// Packing: what's kept while files compile.

/// Which file a recorded file's code is for.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Named {
    File(PathBuf),
    Builtin(String),
}

/// One file's code, as it's written into the block.
struct Recorded {
    name: Named,
    is_module: bool,
    /// For a file: the length and hash of its text as it was compiled, if any of its functions has text. For a module
    /// that comes with the program: its length and [`sample`].
    text: Option<(u32, u64)>,
    body: Vec<u8>,
}

/// Everything compiled while an app was being packed, to go on the end of the program. See [`crate::Engine::take_compiled_code`].
#[derive(Default)]
pub struct CompiledCode {
    /// Every text the files' code names, each once.
    strings: Vec<Vec<u16>>,
    string_at: HashMap<Vec<u16>, u32>,
    files: Vec<Recorded>,
    /// What each name an `import` or `require` gave was found to be, from which folder, and whether it was an `import`.
    resolved: Vec<(PathBuf, String, bool, PathBuf)>,
}

impl CompiledCode {
    fn string(&mut self, units: Vec<u16>) -> u32 {
        if let Some(at) = self.string_at.get(&units) {
            return *at;
        }
        let at = self.strings.len() as u32;
        self.strings.push(units.clone());
        self.string_at.insert(units, at);
        at
    }

    /// How many files' code there is.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// What a [`Vm`] keeps for the code cache: what's being recorded, when packing, and the names it's made from the block
/// so far, when starting.
#[derive(Default)]
pub(crate) struct CodeCache {
    /// Set while a file compiles when packing.
    pub(crate) recording: bool,
    /// What's been recorded, if this engine's packing.
    pub(crate) packing: Option<CompiledCode>,
    /// The functions made since recording started, in the order they were.
    pub(crate) protos: Vec<u32>,
    /// The template objects made while recording, by their value, with their strings, as they're constants.
    pub(crate) templates: Vec<(u64, Vec<Vec<u16>>, Vec<Vec<u16>>)>,
    /// The BigInt literals made while recording, by their value, with their digits and radix.
    pub(crate) bigints: Vec<(u64, String, u32)>,
    /// The atom each of the block's strings has been, by its number, or `u32::MAX` if it hasn't been asked for yet. They're
    /// good until atoms are collected (see `Atoms::generation`).
    names: Vec<u32>,
    names_generation: u32,
    /// Which block `names` are for, by where it is.
    names_block: u64,
    /// Whether every file compiled goes through the cache and back, for testing it. See [`Vm::check_code_cache`].
    pub(crate) checking: bool,
    /// A block to load from, for tests, which have no packed program.
    #[cfg(test)]
    test_block: Option<&'static Block>,
}

/// Whether `RSUI_CODE_CACHE_CHECK` asks for every engine to check the code cache (see [`Vm::check_code_cache`]).
fn checks_from_env() -> bool {
    static CHECKS: OnceLock<bool> = OnceLock::new();
    *CHECKS
        .get_or_init(|| std::env::var_os("RSUI_CODE_CACHE_CHECK").is_some_and(|value| value != "0"))
}

impl CodeCache {
    fn recording(&mut self) -> Option<&mut Self> {
        if self.recording { Some(self) } else { None }
    }
}

// Starting: the block inside the program.

/// Where a file's code is in the block, and what it was compiled from.
#[derive(Clone, Copy, Debug)]
struct Entry {
    is_module: bool,
    /// How long the file is in the program, or for a module that comes with it, how long its source is.
    size: u32,
    /// Whether any of its functions has text, and the length and hash of the text they're in (for a module that comes
    /// with the program, its [`sample`]).
    has_text: bool,
    text_length: u32,
    text_hash: u64,
    at: u32,
    length: u32,
}

/// The block on the end of a packed program, read once.
pub(crate) struct Block {
    storage: crate::code_bytes::CodeBytes,
    /// Where each string is in `bytes`, how long it is, and whether it's in two bytes a unit or one.
    strings: Vec<(u32, u32, bool)>,
    /// Each file's code, by the key packed.rs looks the file up by.
    files: HashMap<String, Entry>,
    builtins: HashMap<String, Entry>,
    /// Where the bodies start in `bytes`.
    bodies: usize,
    /// Which block this is, as no two are, for what's kept of its names.
    id: u64,
    /// The file each name that was imported or required when packing was found to be, by the key of the folder it was
    /// asked for from, the name, and whether it was an `import`. See [`Vm::cached_resolution`].
    resolved: HashMap<(String, String, bool), PathBuf>,
}

static BLOCK: OnceLock<Option<Block>> = OnceLock::new();

/// The block in this program, if it's packed with one that's for this build and reads.
pub(crate) fn block() -> Option<&'static Block> {
    BLOCK
        .get_or_init(|| {
            if std::env::var_os("RSUI_NO_CODE_CACHE").is_some() {
                return None;
            }
            let _time = crate::startup::span(crate::startup::Phase::CacheRead, || None);
            let (bytes, root) = crate::packed::read_code_block()?;
            Block::read(bytes, &root)
        })
        .as_ref()
}

impl Block {
    fn read(bytes: impl Into<crate::code_bytes::CodeBytes>, root: &Path) -> Option<Self> {
        let bytes = bytes.into();
        let mut from = In(&bytes);
        if from.take(8)? != MAGIC || from.u64()? != compiler_build_id() || from.u32()? != FORMAT {
            return None;
        }
        // What's damaged could still read, as code that's wrong, so all of it is checked first.
        let sum = from.u64()?;
        if crate::source_text::hash_bytes(from.0) != sum {
            return None;
        }
        let mut files = HashMap::new();
        let mut builtins = HashMap::new();
        for _ in 0..from.count()? {
            let kind = from.u8()?;
            let name = from.text()?;
            let entry = Entry {
                is_module: from.u8()? != 0,
                size: from.u32()?,
                has_text: from.u8()? != 0,
                text_length: from.u32()?,
                text_hash: from.u64()?,
                at: from.u32()?,
                length: from.u32()?,
            };
            match kind {
                0 => {
                    files.insert(crate::packed::key(&root.join(name)), entry);
                }
                1 => {
                    builtins.insert(name.to_owned(), entry);
                }
                _ => return None,
            }
        }
        let mut resolved = HashMap::new();
        for _ in 0..from.count()? {
            let dir = from.text()?;
            let name = from.text()?;
            let is_import = from.u8()? != 0;
            let path = from.text()?;
            resolved.insert(
                (
                    crate::packed::key(&root.join(dir)),
                    name.to_owned(),
                    is_import,
                ),
                crate::packed::tidied(&root.join(path)),
            );
        }
        let count = from.count()?;
        let mut strings = Vec::with_capacity(count);
        for _ in 0..count {
            let wide = from.u8()? != 0;
            let length = from.u32()?;
            let at = bytes.len() - from.0.len();
            from.take(length as usize * if wide { 2 } else { 1 })?;
            strings.push((at as u32, length, wide));
        }
        let bodies = bytes.len() - from.0.len();
        let all = (bytes.len() - bodies) as u64;
        if files
            .values()
            .chain(builtins.values())
            .any(|entry| u64::from(entry.at) + u64::from(entry.length) > all)
        {
            return None;
        }
        static BLOCKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Some(Self {
            storage: bytes,
            strings,
            files,
            builtins,
            bodies,
            id: BLOCKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            resolved,
        })
    }

    /// The text of string `at`, in units.
    fn text(&self, at: u32) -> Option<Text> {
        let &(start, length, wide) = self.strings.get(at as usize)?;
        let start = start as usize;
        Some(if wide {
            let bytes = &self.storage[start..start + length as usize * 2];
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            Text::from_units(&units)
        } else {
            Text::from_ascii(&self.storage[start..start + length as usize])
        })
    }

    fn units(&self, at: u32) -> Option<Vec<u16>> {
        Some(self.text(at)?.units())
    }

    /// Whether a module that comes with the program is in the block, for what reads them ahead.
    pub(crate) fn has_builtin(&self, name: &str) -> bool {
        self.builtins.contains_key(name)
    }

    /// Whether a file is in the block, for what reads them ahead.
    pub(crate) fn has_file(&self, path: &Path) -> bool {
        crate::packed::lookup_key(path).is_some_and(|key| self.files.contains_key(&key))
    }
}

/// The block as [`crate::packed::append_packed_files`] writes it, for `code`: the files' keys are their paths from `root`
/// on, and only the files in `packed` are kept, each read again to check it's what was compiled (`sizes` says how long
/// each is in the program). What's kept is checked against what's packed here, not when the program starts.
pub(crate) fn write_block(
    code: &CompiledCode,
    root: &Path,
    packed: &HashMap<PathBuf, u32>,
) -> Vec<u8> {
    // A file's found as it would be looked up, so a path written another way is the same file.
    let packed: HashMap<String, (&PathBuf, u32)> = packed
        .iter()
        .map(|(path, size)| (crate::packed::key(path), (path, *size)))
        .collect();
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for file in &code.files {
        if !seen.insert(file.name.clone()) {
            continue;
        }
        let (kind, name, size, text) = match &file.name {
            Named::File(path) => {
                let Some(&(packed_path, size)) = packed.get(&crate::packed::key(path)) else {
                    continue;
                };
                let Ok(inside) = packed_path.strip_prefix(root) else {
                    continue;
                };
                // The file's read again, as it's packed, and only kept if it's what was compiled.
                let Ok(source) = crate::modules::read_source(path) else {
                    continue;
                };
                let now = (source.len() as u32, crate::source_text::hash(&source));
                if file.text.is_some_and(|text| text != now) {
                    continue;
                }
                (0u8, inside.to_string_lossy().into_owned(), size, file.text)
            }
            Named::Builtin(name) => {
                let Some(text) = file.text else {
                    continue;
                };
                (1u8, name.clone(), text.0, Some(text))
            }
        };
        entries.push((kind, name, file.is_module, size, text, file.body.as_slice()));
    }
    // What names were found to be, where both the folder and the file are what's packed.
    let mut resolved = Vec::new();
    for (dir, name, is_import, path) in &code.resolved {
        let Some(&(packed_path, _)) = packed.get(&crate::packed::key(path)) else {
            continue;
        };
        let (Ok(dir), Ok(path)) = (dir.strip_prefix(root), packed_path.strip_prefix(root)) else {
            continue;
        };
        resolved.push((
            dir.to_string_lossy().into_owned(),
            name.clone(),
            *is_import,
            path.to_string_lossy().into_owned(),
        ));
    }
    block_bytes(&code.strings, &entries, &resolved)
}

/// One file's code as it goes in a block: whether it's a file (0) or a module that comes with the program (1), its name,
/// whether it's a module, its size, its text's length and hash, and its body.
type Prepared<'a> = (u8, String, bool, u32, Option<(u32, u64)>, &'a [u8]);

/// A block with these strings and files' code in it, and what these names were found to be: from which folder, the name,
/// whether it was an `import`, and the file, each from the folder the program's in.
fn block_bytes(
    strings: &[Vec<u16>],
    entries: &[Prepared<'_>],
    resolved: &[(String, String, bool, String)],
) -> Vec<u8> {
    let mut index = Out::default();
    let mut bodies = Out::default();
    let mut kept = 0usize;
    for (kind, name, is_module, size, text, body) in entries {
        let (Ok(at), Ok(length)) = (u32::try_from(bodies.0.len()), u32::try_from(body.len()))
        else {
            continue;
        };
        index.u8(*kind);
        index.text(name);
        index.u8(u8::from(*is_module));
        index.u32(*size);
        index.u8(u8::from(text.is_some()));
        let (text_length, text_hash) = text.unwrap_or_default();
        index.u32(text_length);
        index.u64(text_hash);
        index.u32(at);
        index.u32(length);
        bodies.0.extend_from_slice(body);
        kept += 1;
    }
    let mut out = Out::default();
    out.u32(kept as u32);
    out.0.extend_from_slice(&index.0);
    out.u32(resolved.len() as u32);
    for (dir, name, is_import, path) in resolved {
        out.text(dir);
        out.text(name);
        out.u8(u8::from(*is_import));
        out.text(path);
    }
    out.u32(strings.len() as u32);
    for units in strings {
        let wide = units.iter().any(|unit| *unit >= 0x80);
        out.u8(u8::from(wide));
        out.u32(units.len() as u32);
        for unit in units {
            if wide {
                out.u16(*unit);
            } else {
                out.u8(*unit as u8);
            }
        }
    }
    out.0.extend_from_slice(&bodies.0);
    let mut block = Out::default();
    block.0.extend_from_slice(MAGIC);
    block.u64(compiler_build_id());
    block.u32(FORMAT);
    block.u64(crate::source_text::hash_bytes(&out.0));
    block.0.extend_from_slice(&out.0);
    block.0
}

/// Atoms and constants as one file's code numbers them, by where they are in its lists.
#[derive(Default)]
struct Tables {
    atoms: Vec<u32>,
    atom_at: HashMap<u32, u32>,
    consts: Vec<u32>,
    const_at: HashMap<u32, u32>,
}

impl Tables {
    fn atom(&mut self, atom: u32) -> u32 {
        *self.atom_at.entry(atom).or_insert_with(|| {
            self.atoms.push(atom);
            (self.atoms.len() - 1) as u32
        })
    }
    fn constant(&mut self, index: u32) -> u32 {
        *self.const_at.entry(index).or_insert_with(|| {
            self.consts.push(index);
            (self.consts.len() - 1) as u32
        })
    }
}

/// A function as it's read from the block, before it's added.
struct ReadProto {
    proto: Proto,
    code: Vec<Op>,
    caches: Vec<Cache>,
    text: Option<(u32, u32)>,
}

impl Vm {
    /// Has this engine write down what each file compiles to, for [`Vm::take_compiled_code`]: for packing.
    pub(crate) fn keep_compiled_code(&mut self) {
        self.code_cache.get_or_insert_with(Box::default).packing = Some(CompiledCode::default());
    }

    /// Everything written down since [`Vm::keep_compiled_code`].
    pub(crate) fn take_compiled_code(&mut self) -> Option<CompiledCode> {
        let mut code = self.code_cache.as_mut()?.packing.take()?;
        code.resolved = self
            .modules
            .resolved_names()
            .map(|((dir, name, is_import), path)| {
                (dir.clone(), name.clone(), *is_import, path.clone())
            })
            .collect();
        Some(code)
    }

    /// What `name`, asked for from `dir`, was found to be when the program was packed, if it was, so that it needn't be
    /// looked for again: an `import` if `is_import`, and a `require` if not. In a packed program, its files are what they
    /// were then, so what's found is too.
    pub(crate) fn cached_resolution(
        &self,
        dir: &Path,
        name: &str,
        is_import: bool,
    ) -> Option<PathBuf> {
        let block = self.cache_block()?;
        // For measuring what the table saves: `RSUI_NO_CACHED_RESOLUTION` looks every name up again.
        static LOOKS_AGAIN: OnceLock<bool> = OnceLock::new();
        if block.resolved.is_empty()
            || *LOOKS_AGAIN.get_or_init(|| std::env::var_os("RSUI_NO_CACHED_RESOLUTION").is_some())
        {
            return None;
        }
        let key = crate::packed::lookup_key(dir).unwrap_or_else(|| crate::packed::key(dir));
        let found = block
            .resolved
            .get(&(key, name.to_owned(), is_import))
            .cloned();
        if found.is_some() {
            crate::startup::count_resolved();
        }
        found
    }

    /// Has every file this engine compiles go through the code cache and back, for testing it: what's compiled is written
    /// as it would be for a block, read again, and checked to be the same, and what was read is what runs. Packing turns
    /// this off. `RSUI_CODE_CACHE_CHECK` turns it on for every engine.
    pub(crate) fn check_code_cache(&mut self, on: bool) {
        self.code_cache.get_or_insert_with(Box::default).checking = on;
    }

    /// Starts writing down what's compiled, if this engine's packing or checking the cache, and says whether it is. A file
    /// that's compiled lazily, or for a debugger or a source filter, isn't, as it doesn't compile as it would when it
    /// starts.
    pub(crate) fn start_recording(&mut self, lazily: bool) -> bool {
        if self.code_cache.is_none() && checks_from_env() {
            self.check_code_cache(true);
        }
        let Some(cache) = self
            .code_cache
            .as_mut()
            .filter(|cache| cache.packing.is_some() || cache.checking)
        else {
            return false;
        };
        if cache.recording || lazily || crate::inspect::has_source_filter() {
            return false;
        }
        cache.recording = true;
        cache.protos.clear();
        cache.templates.clear();
        cache.bigints.clear();
        true
    }

    /// Writes down what was compiled since [`Vm::start_recording`], if it worked, as `name`'s: `top` is the file's
    /// function and whether it's a module. `source` is a module's own source, if it comes with the program.
    ///
    /// When checking the cache rather than packing, it's read back at once and checked, and what was read is given back,
    /// to be run in place of what was compiled. A difference panics, as that's a mistake in the cache.
    pub(crate) fn finish_recording(
        &mut self,
        name: Named,
        top: Option<(u32, bool)>,
        source: Option<&'static str>,
    ) -> Option<(u32, bool)> {
        let cache = self
            .code_cache
            .as_mut()
            .and_then(|cache| cache.recording())?;
        cache.recording = false;
        let packing = cache.packing.take();
        let checking = packing.is_none();
        let (top, is_module) = top?;
        let mut code = packing.unwrap_or_default();
        let written = self.write_file(&mut code, &name, top, is_module);
        let mut swapped = None;
        if let Some((body, text, order)) = written {
            let text = match source {
                Some(source) => Some((source.len() as u32, sample(source))),
                None => text,
            };
            if checking {
                swapped = Some(self.check_round_trip(
                    &code,
                    &name,
                    text,
                    &body,
                    (top, &order),
                    is_module,
                    source,
                ));
            } else {
                code.files.push(Recorded {
                    name,
                    is_module,
                    text,
                    body,
                });
            }
        }
        if let Some(cache) = self.code_cache.as_mut() {
            if !checking {
                cache.packing = Some(code);
            }
            cache.protos.clear();
            cache.templates.clear();
            cache.bigints.clear();
        }
        swapped
    }

    /// Reads back what [`Vm::write_file`] wrote for a file, checks it's what was compiled, and gives its function.
    #[allow(clippy::too_many_arguments)]
    fn check_round_trip(
        &mut self,
        code: &CompiledCode,
        name: &Named,
        text: Option<(u32, u64)>,
        body: &[u8],
        (top, order): (u32, &[u32]),
        is_module: bool,
        source: Option<&'static str>,
    ) -> (u32, bool) {
        let size = text.map_or(0, |(length, _)| length);
        let bytes = block_bytes(
            &code.strings,
            &[(1, "check".to_owned(), is_module, size, text, body)],
            &[],
        );
        let block =
            Block::read(bytes, Path::new("")).expect("the code cache reads the block it wrote");
        let entry = block.builtins["check"];
        let file = self.protos[top as usize].file;
        let (ids, at, loaded_module) = self
            .read_file_code(&block, entry, file, |vm| match (name, source) {
                (_, Some(source)) => vm.add_source(SourceText::Static(source)),
                (Named::File(path), None) => {
                    let (length, hash) = text.unwrap_or_default();
                    vm.add_source(SourceText::File {
                        path: path.clone(),
                        length: length as usize,
                        hash,
                    })
                }
                (Named::Builtin(_), None) => NO_SOURCE,
            })
            .unwrap_or_else(|| {
                panic!("the code cache couldn't read back what it wrote for {name:?}")
            });
        assert_eq!(
            loaded_module, is_module,
            "the code cache read {name:?} back as another kind of file"
        );
        if let Err(difference) = self.same_code(order, &ids) {
            panic!("the code cache read {name:?} back differently: {difference}");
        }
        // What was compiled is never run now, so what it imports and exports goes.
        self.modules.infos.remove(&top);
        (ids[at], is_module)
    }

    /// Whether functions `a` and `b` are the same, one by one, as compiling and loading should make them: the same
    /// instructions, with what's numbered for this engine the same thing, and the same of everything else.
    fn same_code(&self, a: &[u32], b: &[u32]) -> Result<(), String> {
        let (x, y) = (self.describe_functions(a), self.describe_functions(b));
        if x.len() != y.len() {
            return Err(format!("{} functions, then {}", x.len(), y.len()));
        }
        match x.iter().zip(&y).position(|(one, other)| one != other) {
            Some(at) => Err(format!(
                "function {at} was
{}
and is
{}",
                x[at], y[at]
            )),
            None => Ok(()),
        }
    }

    /// Functions as they'd be in any engine, one line each with all that's known of them, for [`Vm::same_code`]: the
    /// functions they make are said by where they are in `ids`.
    pub(crate) fn describe_functions(&self, ids: &[u32]) -> Vec<String> {
        let local: HashMap<u32, usize> = ids.iter().enumerate().map(|(at, id)| (*id, at)).collect();
        ids.iter()
            .map(|&id| {
                let proto = &self.protos[id as usize];
                let (code, caches) = self.code_room.spans[id as usize];
                let places: Vec<String> = self.caches[proto.cache_base as usize..(proto.cache_base + caches) as usize]
                    .iter()
                    .map(|place| self.describe_atom(place.atom))
                    .collect();
                let ops: Vec<(Vec<u8>, String)> = self.code[proto.start as usize..(proto.start + code) as usize]
                    .iter()
                    .map(|op| self.describe_op(*op, &local))
                    .collect();
                format!(
                    "registers {} params {} length {} flags {} strict {} locals {} name {} captures {:?} {:?}                      handlers {:?} lines {:?} caches {places:?} text {:?} code {ops:?}",
                    proto.registers,
                    proto.params,
                    proto.length,
                    proto.flags,
                    proto.strict,
                    proto.locals,
                    self.describe_atom(proto.name),
                    proto.captures.iter().map(|c| (c.from_register, c.by_value, c.index)).collect::<Vec<_>>(),
                    proto.capture_names.iter().map(|name| self.describe_atom(*name)).collect::<Vec<_>>(),
                    proto.handlers.iter().map(|h| (h.start, h.end, h.target, h.reg, h.is_catch)).collect::<Vec<_>>(),
                    proto.lines,
                    self.text_of_proto(id).map(|(_, start, end)| (start, end)),
                )
            })
            .collect()
    }

    /// An instruction as it reads in any engine: its bytes, with what's numbered for this one (zeroed in them) said.
    fn describe_op(&self, mut op: Op, local: &HashMap<u32, usize>) -> (Vec<u8>, String) {
        let said = match numbered(&mut op) {
            Numbered::Const32(index) => std::mem::take(index).to_string(),
            Numbered::Const16(index) => u32::from(std::mem::take(index)).to_string(),
            Numbered::Atom(atom) => self.describe_atom(Atom(std::mem::take(atom))),
            Numbered::Proto(proto) => format!("function {:?}", local.get(&std::mem::take(proto))),
            Numbered::Nothing => String::new(),
        };
        // A constant is said by what it is, not its number.
        let said = match op {
            Op::LoadConst { .. }
            | Op::AddK { .. }
            | Op::SubK { .. }
            | Op::MulK { .. }
            | Op::DivK { .. }
            | Op::ModK { .. } => self.describe_const(said.parse().unwrap_or(u32::MAX)),
            _ => said,
        };
        let mut bytes = Vec::new();
        write_op(&mut bytes, &op);
        (bytes, said)
    }

    fn describe_atom(&self, atom: Atom) -> String {
        if let Some(index) = atom.held_index() {
            return format!("index {index}");
        }
        let text = self
            .atoms
            .string_of(atom)
            .map(|string| self.strings.items[string as usize].text.to_rust());
        match self.atoms.kind_of(atom) {
            AtomKind::Name => format!("name {text:?}"),
            _ if atom.0 < crate::text::FIXED_ATOMS => format!("well-known {}", atom.0),
            AtomKind::Hidden => {
                let named = text
                    .as_ref()
                    .is_some_and(|text| self.private_names.get(text) == Some(&atom));
                format!("hidden {text:?} {}", if named { "named" } else { "new" })
            }
            AtomKind::Symbol => format!("symbol {text:?}"),
        }
    }

    fn describe_const(&self, index: u32) -> String {
        let Some(&value) = self.consts.get(index as usize) else {
            return format!("no constant {index}");
        };
        if value.is_string() {
            format!("string {:?}", self.text_of(value).to_rust())
        } else if value.is_symbol() {
            format!("symbol {}", self.describe_atom(Atom(value.handle())))
        } else if value.is_object() {
            // A template object is made again; the compiler's two functions are the same ones.
            if value == self.realm.disposal.0 || value == self.realm.name_function.0 {
                format!("function {:x}", value.0)
            } else {
                "object".to_owned()
            }
        } else if value.is_bigint() {
            "bigint".to_owned()
        } else {
            format!("bits {:x}", value.0)
        }
    }

    /// What a file compiled to, as its body in the block, the length and hash of the text its functions are in, if any
    /// are, and its functions in the order they're written, which ends with its own. `None` if any of it is something
    /// this can't write.
    fn write_file(
        &self,
        packing: &mut CompiledCode,
        name: &Named,
        top: u32,
        is_module: bool,
    ) -> Option<(Vec<u8>, Option<(u32, u64)>, Vec<u32>)> {
        let cache = self.code_cache.as_ref()?;
        // Only what the file's function can make is kept: a file read one way and then another made more.
        let recorded: HashMap<u32, usize> = cache
            .protos
            .iter()
            .enumerate()
            .map(|(at, id)| (*id, at))
            .collect();
        let mut wanted = vec![false; cache.protos.len()];
        let mut stack = vec![top];
        while let Some(id) = stack.pop() {
            let at = *recorded.get(&id)?;
            if std::mem::replace(&mut wanted[at], true) {
                continue;
            }
            let proto = &self.protos[id as usize];
            let (length, _) = self.code_room.spans[id as usize];
            for op in &self.code[proto.start as usize..(proto.start + length) as usize] {
                if let Op::Closure { proto, .. } = op {
                    stack.push(*proto);
                }
            }
        }
        let order: Vec<u32> = cache
            .protos
            .iter()
            .zip(&wanted)
            .filter(|(_, wanted)| **wanted)
            .map(|(id, _)| *id)
            .collect();
        let local: HashMap<u32, u32> = order
            .iter()
            .enumerate()
            .map(|(at, id)| (*id, at as u32))
            .collect();
        let mut tables = Tables::default();
        let mut body = Out::default();
        let mut source = None;
        body.count(order.len())?;
        body.u32(*local.get(&top)?);
        for &id in &order {
            let proto = &self.protos[id as usize];
            let (length, cache_count) = self.code_room.spans[id as usize];
            body.u16(proto.registers);
            body.u16(proto.params);
            body.u8(proto.length);
            body.u8(proto.flags);
            body.u8(u8::from(proto.strict));
            body.u8(proto.locals);
            body.u32(tables.atom(proto.name.0));
            body.count(proto.captures.len())?;
            for (capture, name) in proto.captures.iter().zip(&proto.capture_names) {
                body.u8(u8::from(capture.from_register) | (u8::from(capture.by_value) << 1));
                body.u16(capture.index);
                body.u32(tables.atom(name.0));
            }
            body.count(proto.handlers.len())?;
            for handler in &proto.handlers {
                body.u32(handler.start);
                body.u32(handler.end);
                body.u32(handler.target);
                body.u16(handler.reg);
                body.u8(u8::from(handler.is_catch));
            }
            body.count(proto.lines.len())?;
            for (at, line) in &proto.lines {
                body.u32(*at);
                body.u32(*line);
            }
            body.u32(cache_count);
            for place in
                &self.caches[proto.cache_base as usize..(proto.cache_base + cache_count) as usize]
            {
                body.u32(tables.atom(place.atom.0));
            }
            body.u32(length);
            for op in &self.code[proto.start as usize..(proto.start + length) as usize] {
                let mut op = *op;
                match numbered(&mut op) {
                    Numbered::Const32(index) => *index = tables.constant(*index),
                    Numbered::Const16(index) => {
                        *index = u16::try_from(tables.constant(u32::from(*index))).ok()?
                    }
                    Numbered::Atom(atom) => *atom = tables.atom(*atom),
                    Numbered::Proto(proto) => *proto = *local.get(proto)?,
                    Numbered::Nothing => {}
                }
                write_op(&mut body.0, &op);
            }
            // Where its text is, which has to be in the one source the file has, and that has to be the file's own.
            match self.text_of_proto(id) {
                Some((from, start, end)) => {
                    if *source.get_or_insert(from) != from {
                        return None;
                    }
                    body.u8(1);
                    body.u32(start);
                    body.u32(end);
                }
                None => body.u8(0),
            }
        }
        let text = match source {
            None => None,
            Some(from) => match (self.source_entry(from)?, name) {
                (SourceText::File { path, length, hash }, Named::File(file)) if path == file => {
                    Some((u32::try_from(*length).ok()?, *hash))
                }
                (SourceText::Static(_), Named::Builtin(_)) => None,
                _ => return None,
            },
        };
        // What it imports and exports.
        let info = if is_module {
            self.modules.infos.get(&top)
        } else {
            None
        };
        match info {
            Some(info) => {
                body.u8(1);
                write_info(&mut body, info);
            }
            None if is_module => return None,
            None => body.u8(0),
        }
        // The constants, and then the atoms, as more atoms can come from the constants.
        let mut consts = Out::default();
        consts.count(tables.consts.len())?;
        for index in tables.consts.clone() {
            let value = self.consts[index as usize];
            if value.is_string() {
                consts.u8(CONST_STRING);
                consts.u32(packing.string(self.text_of(value).units()));
            } else if value.is_symbol() && value.handle() == self.protos[top as usize].file.0 {
                consts.u8(CONST_FILE);
            } else if value.is_symbol() {
                consts.u8(CONST_SYMBOL);
                consts.u32(tables.atom(value.handle()));
            } else if let Some((_, strings, raw)) =
                cache.templates.iter().find(|(bits, ..)| *bits == value.0)
            {
                consts.u8(CONST_TEMPLATE);
                consts.count(strings.len())?;
                for units in strings {
                    consts.u32(packing.string(units.clone()));
                }
                consts.count(raw.len())?;
                for units in raw {
                    consts.u32(packing.string(units.clone()));
                }
            } else if let Some((_, digits, radix)) =
                cache.bigints.iter().find(|(bits, ..)| *bits == value.0)
            {
                consts.u8(CONST_BIGINT);
                consts.u32(packing.string(digits.encode_utf16().collect()));
                consts.u32(*radix);
            } else if value == self.realm.disposal.0 && !value.is_undefined() {
                consts.u8(CONST_DISPOSAL);
            } else if value == self.realm.name_function.0 && !value.is_undefined() {
                consts.u8(CONST_NAME_FUNCTION);
            } else if value.is_object() || value.is_bigint() || value.is_cell() {
                return None;
            } else {
                consts.u8(CONST_BITS);
                consts.u64(value.0);
            }
        }
        let mut atoms = Out::default();
        atoms.count(tables.atoms.len())?;
        let fixed = crate::text::FIXED_ATOMS;
        let mut new_private: HashMap<u32, u32> = HashMap::new();
        for &raw in &tables.atoms {
            let atom = Atom(raw);
            if let Some(index) = atom.held_index() {
                atoms.u8(ATOM_INDEX);
                atoms.u32(index);
                continue;
            }
            match self.atoms.kind_of(atom) {
                AtomKind::Name => {
                    let string = self.atoms.string_of(atom)?;
                    atoms.u8(ATOM_NAME);
                    atoms.u32(packing.string(self.strings.items[string as usize].text.units()));
                }
                _ if raw < fixed => {
                    atoms.u8(ATOM_FIXED);
                    atoms.u32(raw);
                }
                AtomKind::Hidden => {
                    let description = self
                        .atoms
                        .string_of(atom)
                        .map(|string| self.strings.items[string as usize].text.to_rust())
                        .unwrap_or_default();
                    let units = description.encode_utf16().collect();
                    if self.private_names.get(&description) == Some(&atom) {
                        atoms.u8(ATOM_NAMED_PRIVATE);
                        atoms.u32(packing.string(units));
                    } else {
                        // A class's own `#name`, which is a new one each time the class is compiled.
                        let count = new_private.len() as u32;
                        let at = *new_private.entry(raw).or_insert(count);
                        atoms.u8(ATOM_NEW_PRIVATE);
                        atoms.u32(at);
                        atoms.u32(packing.string(units));
                    }
                }
                AtomKind::Symbol => return None,
            }
        }
        let mut out = Out::default();
        out.u8(u8::from(is_module));
        out.0.extend_from_slice(&atoms.0);
        out.0.extend_from_slice(&consts.0);
        out.0.extend_from_slice(&body.0);
        Some((out.0, text, order))
    }

    /// Loads the file at `path` from the block, if it's there and it's what's packed now, as the function for `file`.
    /// `module` says it has to be a module, as a `.mjs` file is. It says the function and whether it's a module.
    pub(crate) fn load_cached_file(
        &mut self,
        path: &Path,
        file: Atom,
        module: bool,
        lazily: bool,
    ) -> Option<(u32, bool)> {
        if lazily || crate::inspect::has_source_filter() || crate::source_overlay::contains(path) {
            return None;
        }
        let block = self.cache_block()?;
        // Outside a packed program, there's only a block in tests, whose files are on the disk.
        let key = crate::packed::lookup_key(path).unwrap_or_else(|| crate::packed::key(path));
        let entry = *block.files.get(&key)?;
        let size = match crate::packed::packed_size(path) {
            Some(size) => size,
            None => u32::try_from(std::fs::metadata(path).ok()?.len()).ok()?,
        };
        if size != entry.size || (module && !entry.is_module) {
            return None;
        }
        self.load_entry(block, entry, file, |vm| {
            if !entry.has_text {
                return NO_SOURCE;
            }
            vm.add_source(SourceText::File {
                path: path.to_owned(),
                length: entry.text_length as usize,
                hash: entry.text_hash,
            })
        })
    }

    /// Loads the module `name` that comes with the program from the block, if it's there and its source is `source`, as
    /// the function for `file`. Its functions' text is `source`'s if `shows_text`.
    pub(crate) fn load_cached_builtin(
        &mut self,
        name: &str,
        source: &'static str,
        file: Atom,
        shows_text: bool,
    ) -> Option<u32> {
        if self.compiles_lazily(None) || crate::inspect::has_source_filter() {
            return None;
        }
        let block = self.cache_block()?;
        let entry = *block.builtins.get(name)?;
        if entry.size as usize != source.len()
            || entry.text_hash != sample(source)
            || entry.is_module
        {
            return None;
        }
        let loaded = self.load_entry(block, entry, file, |vm| {
            if shows_text {
                vm.add_source(SourceText::Static(source))
            } else {
                NO_SOURCE
            }
        });
        loaded.map(|(proto, _)| proto)
    }

    /// The block files are loaded from: the program's, or in a test, the one it gave.
    fn cache_block(&self) -> Option<&'static Block> {
        #[cfg(test)]
        if let Some(block) = self.code_cache.as_ref().and_then(|cache| cache.test_block) {
            return Some(block);
        }
        block()
    }

    fn load_entry(
        &mut self,
        block: &Block,
        entry: Entry,
        file: Atom,
        text: impl FnOnce(&mut Self) -> u32,
    ) -> Option<(u32, bool)> {
        let loaded = self.read_file_code(block, entry, file, text);
        crate::startup::count_cache(loaded.is_some());
        loaded.map(|(ids, top, is_module)| (ids[top], is_module))
    }

    /// The atom the block's string `at` is, interned the first time it's asked for.
    fn block_name(&mut self, block: &Block, at: u32) -> Option<Atom> {
        let cache = self.code_cache.get_or_insert_with(Box::default);
        let which = block.id;
        if cache.names_generation != self.atoms.generation || cache.names_block != which {
            cache.names.clear();
            cache.names.resize(block.strings.len(), u32::MAX);
            cache.names_generation = self.atoms.generation;
            cache.names_block = which;
        }
        let known = *cache.names.get(at as usize)?;
        if known != u32::MAX {
            return Some(Atom(known));
        }
        let atom = self.atoms.intern(&mut self.strings, block.text(at)?);
        if let Some(cache) = self.code_cache.as_mut() {
            cache.names[at as usize] = atom.0;
        }
        Some(atom)
    }

    /// Adds the functions `entry` in `block` has, as `file`'s, and says their numbers, which of them is the file's own, and
    /// whether it's a module. Nothing's added unless all of it checks out.
    fn read_file_code(
        &mut self,
        block: &Block,
        entry: Entry,
        file: Atom,
        text: impl FnOnce(&mut Self) -> u32,
    ) -> Option<(Vec<u32>, usize, bool)> {
        let start = block.bodies + entry.at as usize;
        let mut from = In(&block.storage[start..start + entry.length as usize]);
        let is_module = from.u8()? != 0;
        let names = crate::startup::span(crate::startup::Phase::CacheNames, || None);
        // Atoms.
        let count = from.count()?;
        let mut atoms = Vec::with_capacity(count);
        let mut new_private: HashMap<u32, Atom> = HashMap::new();
        for _ in 0..count {
            let atom = match from.u8()? {
                ATOM_INDEX => Atom::from_index(from.u32()? as usize)?,
                ATOM_FIXED => {
                    let raw = from.u32()?;
                    (raw < crate::text::FIXED_ATOMS).then_some(Atom(raw))?
                }
                ATOM_NAME => self.block_name(block, from.u32()?)?,
                ATOM_NAMED_PRIVATE => {
                    let name = block.text(from.u32()?)?.to_rust();
                    self.private_atom(&name)
                }
                ATOM_NEW_PRIVATE => {
                    let at = from.u32()?;
                    let description = block.text(from.u32()?)?.to_rust();
                    match new_private.get(&at) {
                        Some(atom) => *atom,
                        None => {
                            let atom = self.new_private_atom(&description);
                            new_private.insert(at, atom);
                            atom
                        }
                    }
                }
                _ => return None,
            };
            atoms.push(atom);
        }
        let atom = |at: u32| atoms.get(at as usize).copied();
        // Constants.
        let count = from.count()?;
        let mut consts = Vec::with_capacity(count);
        for _ in 0..count {
            let value = match from.u8()? {
                CONST_BITS => {
                    let value = Value(from.u64()?);
                    // Bits that would be a handle to something are never written as bits.
                    if value.is_object()
                        || value.is_string()
                        || value.is_symbol()
                        || value.is_bigint()
                        || value.is_cell()
                    {
                        return None;
                    }
                    value
                }
                CONST_STRING => {
                    let found = self.block_name(block, from.u32()?)?;
                    self.atom_value(found)
                }
                CONST_SYMBOL => Value::symbol(atom(from.u32()?)?.0),
                CONST_FILE => Value::symbol(file.0),
                CONST_TEMPLATE => {
                    let mut strings = Vec::new();
                    for _ in 0..from.count()? {
                        strings.push(block.units(from.u32()?)?);
                    }
                    let mut raw = Vec::new();
                    for _ in 0..from.count()? {
                        raw.push(block.units(from.u32()?)?);
                    }
                    let strings: Vec<&[u16]> = strings.iter().map(Vec::as_slice).collect();
                    let raw: Vec<&[u16]> = raw.iter().map(Vec::as_slice).collect();
                    self.template_object(&strings, &raw)
                }
                CONST_BIGINT => {
                    let digits = block.text(from.u32()?)?.to_rust();
                    let radix = from.u32()?;
                    self.bigint_literal(&digits, radix)
                }
                CONST_DISPOSAL => self.disposal_function(),
                CONST_NAME_FUNCTION => self.name_function_native(),
                _ => return None,
            };
            consts.push(self.add_const(value));
        }
        drop(names);
        let _code = crate::startup::span(crate::startup::Phase::CacheCode, || None);
        // Functions: their numbers first, as one makes others by theirs.
        let count = from.count()?;
        let top = from.u32()? as usize;
        if top >= count {
            return None;
        }
        let mut read = Vec::with_capacity(count);
        for _ in 0..count {
            let registers = from.u16()?;
            let params = from.u16()?;
            let length = from.u8()?;
            let flags = from.u8()?;
            let strict = from.u8()? != 0;
            let locals = from.u8()?;
            let name = atom(from.u32()?)?;
            let captures_count = from.count()?;
            let mut captures = Vec::with_capacity(captures_count);
            let mut capture_names = Vec::with_capacity(captures_count);
            for _ in 0..captures_count {
                let bits = from.u8()?;
                let index = from.u16()?;
                captures.push(Capture {
                    from_register: bits & 1 != 0,
                    by_value: bits & 2 != 0,
                    index,
                });
                capture_names.push(atom(from.u32()?)?);
            }
            let mut handlers = Vec::with_capacity(from.count()?);
            for _ in 0..handlers.capacity() {
                handlers.push(Handler {
                    start: from.u32()?,
                    end: from.u32()?,
                    target: from.u32()?,
                    reg: from.u16()?,
                    is_catch: from.u8()? != 0,
                });
            }
            let mut lines = Vec::with_capacity(from.count()?);
            for _ in 0..lines.capacity() {
                lines.push((from.u32()?, from.u32()?));
            }
            let mut caches = Vec::with_capacity(from.count()?);
            for _ in 0..caches.capacity() {
                caches.push(Cache::new(atom(from.u32()?)?));
            }
            let code_count = from.count()?;
            let mut code = Vec::with_capacity(code_count);
            for _ in 0..code_count {
                let mut op = read_op(&mut from.0)?;
                // Every number in the code is the file's own, checked and made this engine's before anything's added.
                match numbered(&mut op) {
                    Numbered::Const32(index) => *index = *consts.get(*index as usize)?,
                    Numbered::Const16(index) => {
                        *index = u16::try_from(*consts.get(*index as usize)?).ok()?
                    }
                    Numbered::Atom(at) => *at = atom(*at)?.0,
                    Numbered::Proto(at) => {
                        if *at as usize >= count {
                            return None;
                        }
                    }
                    Numbered::Nothing => {}
                }
                code.push(op);
            }
            let text = match from.u8()? {
                0 => None,
                _ => Some((from.u32()?, from.u32()?)),
            };
            read.push(ReadProto {
                proto: Proto {
                    start: 0,
                    cache_base: 0,
                    registers,
                    params,
                    length,
                    flags,
                    strict,
                    locals,
                    name,
                    file,
                    captures: captures.into_boxed_slice(),
                    capture_names: capture_names.into_boxed_slice(),
                    handlers: handlers.into_boxed_slice(),
                    lines: lines.into_boxed_slice(),
                },
                code,
                caches,
                text,
            });
        }
        let info = match from.u8()? {
            0 => None,
            _ => Some(read_info(&mut from)?),
        };
        if !from.0.is_empty() || is_module != info.is_some() {
            return None;
        }
        let source = if read.iter().any(|one| one.text.is_some()) {
            text(self)
        } else {
            NO_SOURCE
        };
        let placeholder = || Proto {
            start: 0,
            cache_base: 0,
            registers: 1,
            params: 0,
            length: 0,
            flags: 0,
            strict: false,
            locals: 0,
            name: Atom(0),
            file,
            captures: Box::new([]),
            capture_names: Box::new([]),
            handlers: Box::new([]),
            lines: Box::new([]),
        };
        let ids: Vec<u32> = (0..count)
            .map(|_| self.add_proto(placeholder(), 0, 0))
            .collect();
        for (one, &id) in read.into_iter().zip(&ids) {
            let mut code = one.code;
            for op in &mut code {
                if let Numbered::Proto(at) = numbered(op) {
                    *at = ids[*at as usize];
                }
            }
            let mut proto = one.proto;
            proto.start = self.place_code(&code);
            proto.cache_base = self.place_caches(&one.caches);
            self.replace_proto(id, proto, code.len() as u32, one.caches.len() as u32);
            if let Some(span) = one.text {
                if source != NO_SOURCE {
                    self.set_text(id, source, span);
                }
            }
        }
        if let Some(info) = info {
            self.modules.infos.insert(ids[top], info);
        }
        Some((ids, top, is_module))
    }
}

fn write_info(out: &mut Out, info: &ModuleInfo) {
    out.u32(info.requested.len() as u32);
    for name in &info.requested {
        out.text(name);
    }
    out.u32(info.imports.len() as u32);
    for (from, name) in &info.imports {
        out.u32(*from as u32);
        write_name(out, name.as_deref());
    }
    out.u32(info.locals.len() as u32);
    for name in &info.locals {
        out.text(name);
    }
    out.u32(info.exports.len() as u32);
    for (name, from) in &info.exports {
        out.text(name);
        match from {
            ExportFrom::Local(at) => {
                out.u8(0);
                out.u32(*at as u32);
            }
            ExportFrom::Import(at, name) => {
                out.u8(1);
                out.u32(*at as u32);
                write_name(out, name.as_deref());
            }
        }
    }
    out.u32(info.stars.len() as u32);
    for star in &info.stars {
        out.u32(*star as u32);
    }
    out.u8(u8::from(info.is_async));
}

fn write_name(out: &mut Out, name: Option<&str>) {
    match name {
        Some(name) => {
            out.u8(1);
            out.text(name);
        }
        None => out.u8(0),
    }
}

fn read_name(from: &mut In<'_>) -> Option<Option<String>> {
    Some(match from.u8()? {
        0 => None,
        _ => Some(from.text()?.to_owned()),
    })
}

fn read_info(from: &mut In<'_>) -> Option<ModuleInfo> {
    let requested = (0..from.count()?)
        .map(|_| from.text().map(str::to_owned))
        .collect::<Option<_>>()?;
    let imports = (0..from.count()?)
        .map(|_| Some((from.u32()? as usize, read_name(from)?)))
        .collect::<Option<_>>()?;
    let locals = (0..from.count()?)
        .map(|_| from.text().map(str::to_owned))
        .collect::<Option<_>>()?;
    let exports = (0..from.count()?)
        .map(|_| {
            let name = from.text()?.to_owned();
            let export = match from.u8()? {
                0 => ExportFrom::Local(from.u32()? as usize),
                _ => ExportFrom::Import(from.u32()? as usize, read_name(from)?),
            };
            Some((name, export))
        })
        .collect::<Option<_>>()?;
    let stars = (0..from.count()?)
        .map(|_| Some(from.u32()? as usize))
        .collect::<Option<_>>()?;
    let is_async = from.u8()? != 0;
    Some(ModuleInfo {
        requested,
        imports,
        locals,
        exports,
        stars,
        is_async,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::Engine;

    /// Fields that mean the same in any engine: registers, jumps (which are relative), places in the function's own list
    /// of property caches, counts, small numbers in the code, and numbers the engine itself fixes (`TYPE_` kinds,
    /// `CallMath`'s place in `builtins::math::QUICK`, a debugger's line).
    const PLAIN: &[(&str, &str)] = &[
        ("dst", "u16"),
        ("src", "u16"),
        ("a", "u16"),
        ("b", "u16"),
        ("f", "u16"),
        ("object", "u16"),
        ("key", "u16"),
        ("value", "u16"),
        ("value", "i16"),
        ("value", "i32"),
        ("offset", "i16"),
        ("offset", "i32"),
        ("cache", "u16"),
        ("condition", "u16"),
        ("iter", "u16"),
        ("function", "u16"),
        ("source", "u16"),
        ("array", "u16"),
        ("argc", "u16"),
        ("this", "u16"),
        ("i", "u16"),
        ("constructor", "u16"),
        ("cell", "u16"),
        ("start", "u16"),
        ("reg", "u16"),
        ("kind", "u16"),
        ("which", "u16"),
        ("received", "u16"),
        ("parent", "u16"),
        ("methods", "u16"),
        ("line", "u32"),
        ("limit", "u16"),
        ("limit", "i16"),
        ("keys", "u16"),
        ("home", "u16"),
        ("count", "u16"),
        ("capacity", "u16"),
        ("capacity", "u32"),
    ];

    /// Fields whose name means a number for one engine in other kinds, and is plain only in these.
    const PLAIN_IN: &[(&str, &str, &[&str])] =
        &[("index", "u16", &["GetUp", "GetUpValue", "SetUp"])];

    /// The fields that hold a number for one engine, which [`numbered`] has to give.
    const NUMBERED: &[(&str, &str)] = &[
        ("LoadConst", "index"),
        ("AddK", "k"),
        ("SubK", "k"),
        ("MulK", "k"),
        ("DivK", "k"),
        ("ModK", "k"),
        ("DeclareGlobalVar", "name"),
        ("DeclareGlobalFunction", "name"),
        ("ThrowConstAssign", "name"),
        ("ThrowTdz", "name"),
        ("Closure", "proto"),
    ];

    fn size(kind: &str) -> usize {
        match kind {
            "u8" | "i8" | "bool" => 1,
            "u16" | "i16" => 2,
            "u32" | "i32" => 4,
            _ => 8,
        }
    }

    /// Every kind of instruction op.rs has is written and read back as it was, and every field in one is either known to
    /// mean the same in any engine or is what [`numbered`] gives, so it's numbered again when it's loaded. A new kind with
    /// a new field fails this until it's said which it is.
    #[test]
    fn every_numbered_field_is_remapped() {
        let mut problems = Vec::new();
        assert!(
            OP_KINDS.len() > 150,
            "op.rs was read: {} kinds",
            OP_KINDS.len()
        );
        for (number, (kind, fields)) in OP_KINDS.iter().enumerate() {
            // One of this kind with each field a different number, which fits any of them.
            let mut bytes = vec![number as u8];
            for (at, (_, field_kind)) in fields.iter().enumerate() {
                let value = (at as u64 + 1) * 3;
                bytes.extend_from_slice(&value.to_le_bytes()[..size(field_kind)]);
            }
            let mut from = bytes.as_slice();
            let mut op = read_op(&mut from).unwrap_or_else(|| panic!("{kind} reads"));
            assert!(from.is_empty(), "{kind} reads all it wrote");
            let mut again = Vec::new();
            write_op(&mut again, &op);
            assert_eq!(again, bytes, "{kind} is written as it was read");
            let handed = match numbered(&mut op) {
                Numbered::Const32(value) | Numbered::Atom(value) | Numbered::Proto(value) => {
                    *value = 0x7777;
                    true
                }
                Numbered::Const16(value) => {
                    *value = 0x7777;
                    true
                }
                Numbered::Nothing => false,
            };
            let mut after = Vec::new();
            write_op(&mut after, &op);
            let mut at = 1;
            let mut changed = None;
            for (field, field_kind) in fields.iter() {
                let size = size(field_kind);
                if after[at..at + size] != bytes[at..at + size] {
                    changed = Some(*field);
                }
                at += size;
            }
            assert_eq!(
                handed,
                changed.is_some(),
                "{kind}: what numbered() gives is one of its fields"
            );
            for (field, field_kind) in fields.iter() {
                let is_numbered = NUMBERED.iter().any(|(k, f)| k == kind && f == field);
                let plain = PLAIN.iter().any(|(f, t)| f == field && t == field_kind)
                    || PLAIN_IN
                        .iter()
                        .any(|(f, t, kinds)| f == field && t == field_kind && kinds.contains(kind));
                if is_numbered && changed != Some(*field) {
                    problems.push(format!("{kind}.{field} holds a number for one engine, and numbered() doesn't give it"));
                } else if !is_numbered && changed == Some(*field) {
                    problems.push(format!(
                        "numbered() gives {kind}.{field}, which this test doesn't list in NUMBERED"
                    ));
                } else if !is_numbered && !plain {
                    problems.push(format!(
                        "{kind}.{field}: {field_kind} is new to the code cache. If it holds a constant's, an atom's or a \
                         function's number, add {kind} to numbered() in codecache.rs and to NUMBERED here; if it means the \
                         same in any engine, add it to PLAIN here"
                    ));
                }
            }
        }
        for (kind, field) in NUMBERED {
            let there = OP_KINDS
                .iter()
                .any(|(k, fields)| k == kind && fields.iter().any(|(f, _)| f == field));
            assert!(there, "{kind}.{field} is in NUMBERED but not in op.rs");
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    /// What reading instructions makes of bytes that aren't one: nothing, rather than a panic.
    #[test]
    fn what_isnt_an_instruction_reads_as_none() {
        assert!(read_op(&mut &[][..]).is_none());
        assert!(read_op(&mut &[255u8][..]).is_none() || OP_KINDS.len() == 256);
        // LoadConst with its fields cut short.
        assert!(read_op(&mut &[0u8, 1][..]).is_none());
    }

    /// The JavaScript files in the repository that are run as files: the benchmarks, the Node modules that come with the
    /// engine and its helpers.
    fn corpus() -> Vec<PathBuf> {
        fn walk(folder: &Path, files: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(folder)
                .expect("a corpus folder")
                .flatten()
            {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, files);
                } else if path.extension().is_some_and(|extension| extension == "js") {
                    files.push(path);
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        for folder in ["benches/js", "src/node", "src/js"] {
            walk(&root.join(folder), &mut files);
        }
        files.sort();
        files
    }

    /// The names of the Node modules that come with the engine, from `node_source`.
    fn builtin_names() -> Vec<String> {
        let source = include_str!("modules.rs");
        let start = source
            .find("pub(crate) fn node_source")
            .expect("node_source is in modules.rs");
        let end = start
            + source[start..]
                .find("\n}\n")
                .or_else(|| source[start..].find("\r\n}\r\n"))
                .expect("its end");
        let mut names = Vec::new();
        for line in source[start..end].lines() {
            let line = line.trim();
            if !line.starts_with('"') || !line.contains("=>") {
                continue;
            }
            let pattern = &line[..line.find("=>").expect("checked")];
            for name in pattern.split('|') {
                if let Some(name) = name
                    .trim()
                    .strip_prefix('"')
                    .and_then(|name| name.strip_suffix('"'))
                {
                    names.push(name.to_owned());
                }
            }
        }
        names
    }

    /// Every file in the corpora compiles, goes through the cache and back, and reads back as it compiled, function by
    /// function and instruction by instruction (checking panics if not).
    #[test]
    fn every_file_in_the_corpora_reads_back_as_it_compiled() {
        let files = corpus();
        assert!(files.len() > 60, "found {} files", files.len());
        for path in &files {
            let mut engine = Engine::new();
            engine.set_printer(|_| {});
            engine.check_code_cache(true);
            if let Err(error) = engine.vm.compile_file(path) {
                let message = engine.display(error);
                panic!("{} doesn't compile: {message}", path.display());
            }
        }
    }

    /// Every Node module that comes with the engine goes through the cache and back as it loads, and loading it does
    /// what it does without the cache.
    #[test]
    fn every_bundled_module_reads_back_and_runs_the_same() {
        let names = builtin_names();
        assert!(names.len() > 40, "found {} modules", names.len());
        let folder =
            std::env::temp_dir().join(format!("rsui-codecache-modules-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        for (at, name) in names.iter().enumerate() {
            let file = folder.join(format!("m{at}.cjs"));
            std::fs::write(
                &file,
                format!("console.log(Object.keys(require({name:?})).sort().join());"),
            )
            .unwrap();
            let run = |check: bool| {
                let output = Rc::new(RefCell::new(String::new()));
                let sink = output.clone();
                let mut engine = Engine::new();
                engine.set_printer(move |text| sink.borrow_mut().push_str(text));
                engine.check_code_cache(check);
                let result = engine.run_file(&file).and_then(|_| engine.run_until_idle());
                let printed = output.borrow().clone();
                match result {
                    Ok(_) => format!("ok {printed}"),
                    Err(error) => format!("threw {} {printed}", error.message),
                }
            };
            assert_eq!(run(true), run(false), "require({name:?})");
        }
        let _ = std::fs::remove_dir_all(folder);
    }

    /// A small app that uses what the cache has to get right, as files: ES modules and CommonJS, classes with private
    /// names, template objects, BigInts, constants of every kind, errors that name variables, functions' text, lines in
    /// stack traces, and `import.meta`.
    const APP: &[(&str, &str)] = &[
        (
            "main.mjs",
            r#"import { Counter, tag, big, shared } from './lib.mjs';
import * as everything from './lib.mjs';
import cjs from './old.cjs';
import def, { again } from './again.mjs';
const counter = new Counter();
counter.add(2); counter.add(3.5);
console.log('count', counter.total, Counter.made, counter.hasSecret());
console.log('tag', tag`one ${1} two ${'x'}`, tag`one ${1} two ${'x'}` === tag`one ${1} two ${'x'}`);
const same = () => tag`same`;
console.log('same template', same() === same());
console.log('big', big, typeof big, big * 3n, 0x1fn);
console.log('consts', 0.1 + 0.2, -0 === 0, Object.is(-0, -0), 1e300 * 1e10, NaN, 'é', '\uD800'.length, 'x' + 1.5);
console.log('names', Object.keys(everything).sort().join(), shared === everything.shared);
console.log('cjs', cjs.twice(21), cjs.name, def(), again);
try { tdz(); } catch (error) { console.log('tdz', error.constructor.name, error.message); }
function tdz() { late; let late = 1; }
try { const fixed = 1; fixed = 2; } catch (error) { console.log('const', error.constructor.name, error.message); }
console.log('text', String(Counter).slice(0, 13), String(same), (function named(a, b) { return a + b; }).toString());
console.log('line', new Error('here').stack.split('\n')[1].replace(/.*[\\/]/, ''));
console.log('meta', typeof import.meta.url, import.meta.url.endsWith('main.mjs'));
async function waits() { await null; return 'awaited'; }
function* counts() { yield 1; yield* [2, 3]; }
console.log('gen', [...counts()].join());
waits().then((value) => console.log('async', value));
const { a, ...rest } = { a: 1, b: 2, c: 3 };
console.log('rest', a, JSON.stringify(rest), [..."ab"].join('|'), /b+/g.exec('abbb')[0]);
label: for (let i = 0; i < 3; i++) { for (let j = 0; j < 3; j++) { if (j === 1) continue label; if (i === 2) break label; } }
console.log('switch', ((x) => { switch (x) { case 1: return 'one'; default: return 'other'; } })(1), null ?? 'n', ({})?.x?.y);
"#,
        ),
        (
            "lib.mjs",
            r#"export class Counter {
    static made = 0;
    #secret = 42;
    #total = 0;
    static { Counter.made = 1; }
    add(n) { this.#total += n; }
    get total() { return this.#total; }
    hasSecret() { return #secret in this && this.#secret === 42; }
}
export class Other { #secret = 'other'; read(o) { try { return o.#secret; } catch (error) { return error.constructor.name; } } }
export function tag(strings, ...values) { return strings.raw.join('_') + values.length; }
export const big = 12345678901234567890n;
export const shared = { made: 'once' };
export * from './star.mjs';
"#,
        ),
        (
            "star.mjs",
            "export const fromStar = 'star';\nexport default 'not re-exported';\n",
        ),
        (
            "again.mjs",
            "import { shared } from './lib.mjs';\nexport default function () { return shared.made; }\nexport { fromStar as again } from './star.mjs';\n",
        ),
        (
            "old.cjs",
            "const path = require('path');\nexports.twice = (n) => n * 2;\nexports.name = path.basename(__filename);\n",
        ),
    ];

    /// Writes [`APP`] in a folder of its own, and gives where its main file is.
    fn write_app(name: &str) -> (PathBuf, PathBuf) {
        let folder =
            std::env::temp_dir().join(format!("rsui-codecache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        for (file, text) in APP {
            std::fs::write(folder.join(file), text).unwrap();
        }
        // Paths as the engine finds them, which is what's packed.
        let folder = crate::packed::tidied(&std::fs::canonicalize(&folder).unwrap());
        let folder = PathBuf::from(folder.to_string_lossy().trim_start_matches(r"\\?\"));
        (folder.join("main.mjs"), folder)
    }

    /// Runs the app at `main` in `engine` to its end, and gives what it printed.
    fn run_app(mut engine: Engine, main: &Path) -> (String, Engine) {
        let output = Rc::new(RefCell::new(String::new()));
        let sink = output.clone();
        engine.set_printer(move |text| sink.borrow_mut().push_str(text));
        let result = engine.run_file(main).and_then(|_| engine.run_until_idle());
        if let Err(error) = result {
            panic!(
                "the app threw: {}\nhaving printed:\n{}",
                error.message,
                output.borrow()
            );
        }
        let printed = output.borrow().clone();
        (printed, engine)
    }

    #[test]
    fn an_app_run_through_the_cache_does_what_it_does_without() {
        let (main, folder) = write_app("check");
        let (plain, _) = run_app(Engine::new(), &main);
        let mut checking = Engine::new();
        checking.check_code_cache(true);
        let (checked, _) = run_app(checking, &main);
        assert_eq!(checked, plain);
        // What it printed is what it should have, so the test is of something.
        assert!(plain.contains("count 5.5 1 true"), "{plain}");
        assert!(plain.contains("same template true"), "{plain}");
        assert!(plain.contains("tdz ReferenceError"), "{plain}");
        assert!(plain.contains("async awaited"), "{plain}");
        let _ = std::fs::remove_dir_all(folder);
    }

    /// What one engine compiled, packed as a block, loads in another that numbers its atoms, constants and functions
    /// differently, as a packed program does, and the app does the same there.
    #[test]
    fn a_block_from_one_engine_loads_in_another() {
        let (main, folder) = write_app("block");
        let mut packing = Engine::new();
        packing.keep_compiled_code();
        let (expected, mut packing) = run_app(packing, &main);
        let code = packing.take_compiled_code().expect("it was kept");
        let files: Vec<PathBuf> = APP.iter().map(|(file, _)| folder.join(file)).collect();
        let sizes: HashMap<PathBuf, u32> = files
            .iter()
            .map(|file| (file.clone(), std::fs::metadata(file).unwrap().len() as u32))
            .collect();
        let bytes = write_block(&code, &folder, &sizes);
        let block: &'static Block = Box::leak(Box::new(
            Block::read(bytes, &folder).expect("the block reads"),
        ));
        assert_eq!(
            block.files.len(),
            APP.len(),
            "every file of the app is in the block"
        );
        assert!(
            block.builtins.contains_key("path"),
            "and the module it requires"
        );

        // Another engine, which has made things of its own first, so its numbers are different.
        let starting = || {
            let mut engine = Engine::new();
            engine.set_printer(|_| {});
            let junk: Vec<String> = (0..300).map(|n| format!("'junk {n}'")).collect();
            let script = format!(
                "var junk = [{}]; function first(x) {{ return x + 0.25 + junk.length; }} first(1);",
                junk.join(",")
            );
            engine.eval(&script, "junk.js").unwrap();
            engine
        };
        let mut loading = starting();
        loading
            .vm
            .code_cache
            .get_or_insert_with(Box::default)
            .test_block = Some(block);
        let before = crate::startup::count_loaded();
        let resolved_before = crate::startup::count_resolved_so_far();
        let (printed, loading) = run_app(loading, &main);
        assert_eq!(printed, expected);
        // Each file, and `path`, was loaded rather than compiled, and what each of the four files' names the app imports
        // was found to be was known.
        assert!(crate::startup::count_loaded() >= before + APP.len() as u32 + 1);
        assert!(crate::startup::count_resolved_so_far() >= resolved_before + 4);

        // Function by function, what was loaded is what compiling there makes, though numbered differently.
        let mut compiling = starting();
        for (file, _) in APP {
            let path = folder.join(file);
            let entry = block.files[&crate::packed::key(&path)];
            let mut loading_one = starting();
            let name = loading_one.vm.intern_str(&path.to_string_lossy());
            let source = SourceText::File {
                path: path.clone(),
                length: entry.text_length as usize,
                hash: entry.text_hash,
            };
            let (ids, _, _) = loading_one
                .vm
                .read_file_code(block, entry, name, |vm| vm.add_source(source))
                .expect("it loads");
            let loaded = loading_one.vm.describe_functions(&ids);
            compiling.vm.keep_compiled_code();
            assert!(compiling.vm.start_recording(false));
            let compiled = compiling.vm.compile_file(&path).expect("it compiles");
            let top = match compiled {
                crate::modules::FileCode::Module(proto) => proto,
                crate::modules::FileCode::CommonJs(proto) => proto,
            };
            let mut scratch = CompiledCode::default();
            let named = Named::File(path.clone());
            let (_, _, order) = compiling
                .vm
                .write_file(&mut scratch, &named, top, entry.is_module)
                .expect("it writes");
            compiling.vm.code_cache.as_mut().unwrap().recording = false;
            let made = compiling.vm.describe_functions(&order);
            assert_eq!(loaded, made, "{file}");
            // And the numbers in the code are this engine's: its 300 strings came first, so the constants' are higher
            // than the file's own list goes.
            if *file == "main.mjs" {
                let ops: Vec<Op> = ids
                    .iter()
                    .flat_map(|&id| {
                        let start = loading_one.vm.protos[id as usize].start as usize;
                        loading_one.vm.code
                            [start..start + loading_one.vm.code_room.spans[id as usize].0 as usize]
                            .to_vec()
                    })
                    .collect();
                assert!(
                    ops.iter()
                        .any(|op| matches!(op, Op::LoadConst { index, .. } if *index >= 300))
                );
            }
        }
        drop(loading);
        let _ = std::fs::remove_dir_all(folder);
    }

    /// A block that's damaged, cut short or for another build isn't used, and loading from one that's damaged in its
    /// middle adds nothing that doesn't check out.
    #[test]
    fn a_block_that_doesnt_check_out_isnt_used() {
        let (main, folder) = write_app("damaged");
        let mut packing = Engine::new();
        packing.keep_compiled_code();
        let (expected, mut packing) = run_app(packing, &main);
        let code = packing.take_compiled_code().unwrap();
        let files: Vec<PathBuf> = APP.iter().map(|(file, _)| folder.join(file)).collect();
        let sizes: HashMap<PathBuf, u32> = files
            .iter()
            .map(|file| (file.clone(), std::fs::metadata(file).unwrap().len() as u32))
            .collect();
        let bytes = write_block(&code, &folder, &sizes);
        // Another build's.
        let mut other = bytes.clone();
        other[8] ^= 1;
        assert!(Block::read(other, &folder).is_none());
        // Cut short.
        assert!(Block::read(bytes[..bytes.len() - 1].to_vec(), &folder).is_none());
        assert!(Block::read(bytes[..20].to_vec(), &folder).is_none());
        // Damaged anywhere, in what's compiled as well: code that's wrong could still read, so none of it is used.
        for flip in [
            30,
            bytes.len() / 3,
            bytes.len() / 2 + 13,
            bytes.len() * 3 / 4,
            bytes.len() - 7,
        ] {
            let mut damaged = bytes.clone();
            damaged[flip] ^= 0x5a;
            assert!(
                Block::read(damaged, &folder).is_none(),
                "damaged at {flip} of {}",
                bytes.len()
            );
        }
        // A file that's changed since isn't loaded.
        let block: &'static Block = Box::leak(Box::new(Block::read(bytes, &folder).unwrap()));
        std::fs::write(
            folder.join("star.mjs"),
            "export const fromStar = 'changed!';\nexport default 'not re-exported';\n",
        )
        .unwrap();
        let mut loading = Engine::new();
        loading
            .vm
            .code_cache
            .get_or_insert_with(Box::default)
            .test_block = Some(block);
        let (printed, _) = run_app(loading, &main);
        assert!(expected.contains(" once star"), "{expected}");
        assert_eq!(printed, expected.replace(" once star", " once changed!"));
        let _ = std::fs::remove_dir_all(folder);
    }
}
