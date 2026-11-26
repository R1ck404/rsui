//! Files that come inside the program, so that a program can be handed to someone as one file.
//!
//! They go on the end of the program file, where Windows doesn't look. They're read from there as
//! they're needed, so they take no memory until then, and they look to everything else as if they
//! were next to the program, where anything it reads for itself would be.
//!
//! ```text
//! the program | each file's bytes | their compiled code | the list of files | where the list is | how long it is | RSUIPACK
//! ```
//!
//! What the files compiled to when the program was packed is one block, which codecache.rs writes and reads.
//!
//! A package is packed whole, as it can load more of itself later than packing saw, but most of it never is. Its code
//! that didn't run when the program was packed is kept squeezed (Zstandard, with a dictionary of a sample of such files,
//! which is kept once, as each is small), and only made whole again if it's read. What did run is as it was, so a program
//! starts as quickly, and every file is still there, so it does everything it did.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

const MAGIC: &[u8; 8] = b"RSUIPACK";
const FOOTER: u64 = 8 + 8 + 8;

/// How hard squeezing what's kept squeezed tries. It's done once, as the program's packed.
const SQUEEZE_LEVEL: i32 = 9;
/// How much of the files that are squeezed is taken for the dictionary they're squeezed with.
const DICTIONARY_SIZE: usize = 110 * 1024;

struct PackedFiles {
    /// The program, open for reading what's in it, which each file is read from where it is.
    program: File,
    /// Where each file is in the program, by its path as [`key`] writes it, how long it is, and how long it's kept,
    /// squeezed, or 0 if it's kept as it is.
    files: FileIndex,
    /// Where the dictionary that squeezed files need is, and how long it is, and the dictionary once it's been read.
    dictionary: Option<(u64, u64)>,
    dictionary_read: OnceLock<Option<Vec<u8>>>,
    /// Every folder that has any of them in it.
    folders: HashSet<String>,
    /// The files that were run when the program was packed, in the order they were.
    run_order: Vec<PathBuf>,
    /// The modules that come with the engine, or that the program gave it, that were run when it was packed, in the
    /// order they were.
    builtins: Vec<String>,
    /// The folders that were links to others when it was packed, by their path as [`key`] writes it, and the folder
    /// each led to. pnpm puts packages in `node_modules` this way, and a package is only packed where it really is.
    links: HashMap<String, PathBuf>,
    /// The folder the program is in, which the files' paths are from.
    root: PathBuf,
    /// Where the files' compiled code is in the program, and how long it is, if it's there (see codecache.rs).
    code: Option<(u64, u64)>,
    diagnostics: bool,
}

static PACKED: OnceLock<PackedFiles> = OnceLock::new();

/// Offset in the executable, uncompressed size, and compressed size (zero for a plain entry).
type FileLocation = (u64, u32, u32);
/// The name's UTF-8 byte range in the archive index and the location of its contents.
type PackedFile = (Range<usize>, FileLocation);

/// Large packages often put thousands of files in one directory. Keep its
/// normalized path once, and allocate only the filename for each entry.
#[derive(Default)]
struct FileIndex {
    directories: HashMap<Box<str>, HashMap<Box<str>, FileLocation>>,
}

impl FileIndex {
    fn parts(path: &str) -> (&str, &str) {
        path.split_at(path.rfind('\\').map_or(0, |at| at + 1))
    }

    fn insert(&mut self, path: &str, place: FileLocation) {
        let (directory, name) = Self::parts(path);
        if let Some(files) = self.directories.get_mut(directory) {
            files.insert(name.into(), place);
        } else {
            self.directories.insert(directory.into(), HashMap::from([(name.into(), place)]));
        }
    }

    fn get(&self, path: &str) -> Option<&FileLocation> {
        let (directory, name) = Self::parts(path);
        self.directories.get(directory)?.get(name)
    }
}

/// A folder that was a link when the program was packed, and the folder it led to.
pub type PackedLink = (PathBuf, PathBuf);

/// What's on the end of a program: which file to run, every file with where it is, which were run when it was
/// packed, in the order they were, the modules that come with the engine that were, and the folders that were links.
struct PackedList {
    bytes: crate::code_bytes::CodeBytes,
    entry: String,
    files: Vec<PackedFile>,
    run_order: Vec<String>,
    builtins: Vec<String>,
    links: Vec<(String, String)>,
    code: Option<(u64, u64)>,
    diagnostics: bool,
    dictionary: Option<(u64, u64)>,
}

impl PackedList {
    fn file_name<'a>(bytes: &'a [u8], range: &Range<usize>) -> &'a str {
        std::str::from_utf8(&bytes[range.clone()]).expect("filename validated by read_text")
    }
}

/// A path as files are looked up by, with what can be worked out from it worked out. Letters are all
/// the same case, as they are to Windows, so a program does the same thing packed as not.
pub(crate) fn key(path: &Path) -> String {
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    let mut start = String::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => start.push_str(&prefix.as_os_str().to_string_lossy()),
            Component::RootDir => start.push('\\'),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(part) => parts.push(part),
        }
    }
    let mut text = start;
    for (at, part) in parts.iter().enumerate() {
        if at > 0 {
            text.push('\\');
        }
        text.push_str(&part.to_string_lossy());
    }
    text.to_lowercase()
}

fn read_u32(from: &mut &[u8]) -> Option<u32> {
    let (number, rest) = from.split_first_chunk::<4>()?;
    *from = rest;
    Some(u32::from_le_bytes(*number))
}

fn read_u64(from: &mut &[u8]) -> Option<u64> {
    let (number, rest) = from.split_first_chunk::<8>()?;
    *from = rest;
    Some(u64::from_le_bytes(*number))
}

fn read_text<'a>(from: &mut &'a [u8]) -> Option<&'a str> {
    let length = read_u32(from)? as usize;
    let (text, rest) = from.split_at_checked(length)?;
    *from = rest;
    std::str::from_utf8(text).ok()
}

/// Looks for files inside this program. If there are any, this says which one to run, and importing
/// finds them from then on.
/// Whether this program is one that was packed, and has its own files inside it.
pub fn running_packed() -> bool {
    PACKED.get().is_some()
}

/// Only explicitly diagnostic builds expose test steps and the live inspector.
pub fn packed_diagnostics() -> bool {
    PACKED.get().is_some_and(|packed| packed.diagnostics)
}

pub fn open_packed_files() -> Option<PathBuf> {
    let program = std::env::current_exe().ok()?;
    let list = read_list(&program)?;
    let root = program.parent()?;
    let mut packed = PackedFiles {
        program: File::open(&program).ok()?,
        files: FileIndex::default(),
        folders: HashSet::new(),
        run_order: list.run_order.iter().map(|path| root.join(path)).collect(),
        builtins: list.builtins,
        links: HashMap::with_capacity(list.links.len()),
        root: root.to_owned(),
        code: list.code,
        diagnostics: list.diagnostics,
        dictionary: list.dictionary,
        dictionary_read: OnceLock::new(),
    };
    let add_folders = |path: &Path, folders: &mut HashSet<String>| {
        let mut folder = path.parent();
        while let Some(found) = folder {
            if !folders.insert(key(found)) {
                break;
            }
            folder = found.parent();
        }
    };
    for (name, place) in list.files {
        let path = root.join(PackedList::file_name(&list.bytes, &name));
        add_folders(&path, &mut packed.folders);
        packed.files.insert(&key(&path), place);
    }
    for (link, target) in list.links {
        let link = root.join(link);
        // The folders it's in are there, as it is, as what it leads to is.
        add_folders(&link, &mut packed.folders);
        packed.links.insert(key(&link), tidied(&root.join(target)));
    }
    let _ = PACKED.set(packed);
    Some(root.join(list.entry))
}

/// The list of files in a program, if it has one: which to run, and each one's path and place.
fn read_list(program: &Path) -> Option<PackedList> {
    let mut file = File::open(program).ok()?;
    file.seek(SeekFrom::End(-(FOOTER as i64))).ok()?;
    let mut footer = [0u8; FOOTER as usize];
    file.read_exact(&mut footer).ok()?;
    if &footer[16..] != MAGIC {
        return None;
    }
    let mut numbers = &footer[..16];
    let at = read_u64(&mut numbers)?;
    let length = read_u64(&mut numbers)?;
    // These are only numbers on the end of a file, and it may have been damaged.
    if at.checked_add(length)? > file.metadata().ok()?.len() {
        return None;
    }
    let list = crate::code_bytes::CodeBytes::read(&file, at, length).ok()?;
    let mut rest = &list[..];
    let entry = read_text(&mut rest)?.to_owned();
    let count = read_u32(&mut rest)?;
    let mut files = Vec::with_capacity((count as usize).min(rest.len() / 16));
    for _ in 0..count {
        let name_len = read_text(&mut rest)?.len();
        let end = list.len() - rest.len();
        let place = read_u64(&mut rest)?;
        let size = read_u32(&mut rest)?;
        files.push((end - name_len..end, (place, size, 0)));
    }
    // A program packed before this was kept doesn't have it.
    let mut run_order = Vec::new();
    if let Some(count) = read_u32(&mut rest) {
        for _ in 0..count {
            run_order.push(read_text(&mut rest)?.to_owned());
        }
    }
    // Nor this.
    let mut builtins = Vec::new();
    if let Some(count) = read_u32(&mut rest) {
        for _ in 0..count {
            builtins.push(read_text(&mut rest)?.to_owned());
        }
    }
    // Nor this.
    let mut links = Vec::new();
    if let Some(count) = read_u32(&mut rest) {
        for _ in 0..count {
            let link = read_text(&mut rest)?.to_owned();
            links.push((link, read_text(&mut rest)?.to_owned()));
        }
    }
    // Nor this.
    let code = match (read_u64(&mut rest), read_u64(&mut rest)) {
        (Some(place), Some(size)) if size > 0 && place.checked_add(size)? <= at => {
            Some((place, size))
        }
        _ => None,
    };
    let diagnostics = read_u32(&mut rest).unwrap_or(0) & 1 != 0;
    // Nor this: the files that are kept squeezed, by their number in the list, with how long each is kept, and the
    // dictionary they need.
    let mut dictionary = None;
    if let (Some(place), Some(size)) = (read_u64(&mut rest), read_u64(&mut rest)) {
        if place.checked_add(size)? > at {
            return None;
        }
        dictionary = (size > 0).then_some((place, size));
        let count = read_u32(&mut rest)?;
        for _ in 0..count {
            let index = read_u32(&mut rest)? as usize;
            let kept = read_u32(&mut rest)?;
            let (_, (place, _, stored)) = files.get_mut(index)?;
            if place.checked_add(u64::from(kept))? > at {
                return None;
            }
            *stored = kept;
        }
    }
    Some(PackedList {
        bytes: list,
        entry,
        files,
        run_order,
        builtins,
        links,
        code,
        diagnostics,
        dictionary,
    })
}

/// Puts files on the end of a program, which `to` is open on, at the end of it. Their paths are kept from
/// `root` on, and all of `files` have to be inside that, along with `entry`, which is what's run. `run_order` is the
/// files that running it ran, in the order it ran them, which is the order they're read ahead in when it starts. Any
/// that aren't in `files` are left out. `builtins` is the modules that come with the engine that it ran, which are read
/// ahead first. `links` is the folders inside `root` that were links, as pnpm makes, with the folder inside it that each
/// leads to, which is where what's in it is packed, so a package is found by its link and runs from where it really is.
/// `code` is what the files compiled to when the app ran to be packed, which the program loads rather than compiling them
/// again, if it's given (see codecache.rs).
#[allow(clippy::too_many_arguments)]
pub fn append_packed_files(
    to: &mut File,
    root: &Path,
    entry: &Path,
    files: &[PathBuf],
    run_order: &[PathBuf],
    builtins: &[String],
    links: &[PackedLink],
    code: Option<&crate::codecache::CompiledCode>,
) -> io::Result<()> {
    append_packed_files_with_diagnostics(
        to, root, entry, files, run_order, builtins, links, code, false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn append_packed_files_with_diagnostics(
    to: &mut File,
    root: &Path,
    entry: &Path,
    files: &[PathBuf],
    run_order: &[PathBuf],
    builtins: &[String],
    links: &[PackedLink],
    code: Option<&crate::codecache::CompiledCode>,
    diagnostics: bool,
) -> io::Result<()> {
    fn add_text(list: &mut Vec<u8>, text: &str) {
        list.extend_from_slice(&(text.len() as u32).to_le_bytes());
        list.extend_from_slice(text.as_bytes());
    }
    fn inside(root: &Path, path: &Path) -> io::Result<String> {
        path.strip_prefix(root)
            .map(|path| path.to_string_lossy().into_owned())
            .map_err(|_| {
                io::Error::other(format!("{} isn't in {}", path.display(), root.display()))
            })
    }
    let mut at = to.seek(SeekFrom::End(0))?;
    let mut list = Vec::new();
    add_text(&mut list, &inside(root, entry)?);
    list.extend_from_slice(&(files.len() as u32).to_le_bytes());
    let mut sizes = HashMap::with_capacity(files.len());
    let ran: HashSet<&PathBuf> = run_order.iter().collect();
    let squeezed_ones: Vec<bool> = files
        .iter()
        .map(|path| may_squeeze(path) && !ran.contains(path) && path != entry)
        .collect();
    let dictionary = squeeze_dictionary(files, &squeezed_ones)?;
    let dictionary_place = at;
    to.write_all(&dictionary)?;
    at += dictionary.len() as u64;
    let mut squeezed = Vec::new();
    for (index, path) in files.iter().enumerate() {
        let bytes = crate::source_overlay::bytes(path)?;
        let size = u32::try_from(bytes.len())
            .map_err(|_| io::Error::other(format!("{} is too big to pack", path.display())))?;
        // What's kept squeezed has to be worth it: a tenth smaller at least.
        let frame = (squeezed_ones[index] && !dictionary.is_empty())
            .then(|| crate::builtins::host::zlib::zstd_frame(&bytes, SQUEEZE_LEVEL, Some(&dictionary)))
            .filter(|frame| frame.len() < bytes.len() / 10 * 9);
        let kept = frame.as_deref().unwrap_or(&bytes);
        to.write_all(kept)?;
        add_text(&mut list, &inside(root, path)?);
        list.extend_from_slice(&at.to_le_bytes());
        list.extend_from_slice(&size.to_le_bytes());
        if frame.is_some() {
            squeezed.push((index as u32, kept.len() as u32));
        }
        at += kept.len() as u64;
        sizes.insert(path.clone(), size);
    }
    let code = code
        .filter(|code| !code.is_empty())
        .map(|code| crate::codecache::write_block(code, root, &sizes));
    let code_place = at;
    if let Some(block) = &code {
        to.write_all(block)?;
        at += block.len() as u64;
    }
    let ran: Vec<String> = run_order
        .iter()
        .filter(|path| files.contains(path))
        .map(|path| inside(root, path))
        .collect::<io::Result<_>>()?;
    list.extend_from_slice(&(ran.len() as u32).to_le_bytes());
    for path in &ran {
        add_text(&mut list, path);
    }
    list.extend_from_slice(&(builtins.len() as u32).to_le_bytes());
    for name in builtins {
        add_text(&mut list, name);
    }
    list.extend_from_slice(&(links.len() as u32).to_le_bytes());
    for (link, target) in links {
        add_text(&mut list, &inside(root, link)?);
        add_text(&mut list, &inside(root, target)?);
    }
    list.extend_from_slice(&code_place.to_le_bytes());
    list.extend_from_slice(&(code.as_ref().map_or(0, Vec::len) as u64).to_le_bytes());
    list.extend_from_slice(&u32::from(diagnostics).to_le_bytes());
    list.extend_from_slice(&dictionary_place.to_le_bytes());
    list.extend_from_slice(&(dictionary.len() as u64).to_le_bytes());
    list.extend_from_slice(&(squeezed.len() as u32).to_le_bytes());
    for (index, kept) in &squeezed {
        list.extend_from_slice(&index.to_le_bytes());
        list.extend_from_slice(&kept.to_le_bytes());
    }
    to.write_all(&list)?;
    to.write_all(&at.to_le_bytes())?;
    to.write_all(&(list.len() as u64).to_le_bytes())?;
    to.write_all(MAGIC)?;
    Ok(())
}

/// Whether a file may be kept squeezed, if it didn't run when the program was packed: a package's code, which it may
/// load later, but mostly never does. Its `package.json` is read to find its files, and pictures, fonts and sounds are
/// mostly squeezed already, so those are kept as they are, as is everything an app has of its own.
fn may_squeeze(path: &Path) -> bool {
    const KINDS: [&str; 9] = ["js", "mjs", "cjs", "json", "ts", "mts", "cts", "tsx", "jsx"];
    let in_package = path
        .components()
        .any(|part| part.as_os_str().eq_ignore_ascii_case("node_modules"));
    let kind = path
        .extension()
        .is_some_and(|ending| KINDS.iter().any(|kind| ending.eq_ignore_ascii_case(kind)));
    let is_manifest = path
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("package.json"));
    in_package && kind && !is_manifest
}

/// The dictionary that the files to be squeezed are squeezed with: some of each of them, from all through the list, as
/// files near each other in a package are much alike. Empty if there's nothing to squeeze.
fn squeeze_dictionary(files: &[PathBuf], squeezed: &[bool]) -> io::Result<Vec<u8>> {
    let wanted: Vec<&PathBuf> = files
        .iter()
        .zip(squeezed)
        .filter_map(|(path, &squeeze)| squeeze.then_some(path))
        .collect();
    let mut dictionary = Vec::new();
    if wanted.is_empty() {
        return Ok(dictionary);
    }
    // A file in every so many, until there's enough, so the sample is from all of them rather than the first few.
    let step = (wanted.len() / 256).max(1);
    for path in wanted.iter().step_by(step) {
        let bytes = crate::source_overlay::bytes(path)?;
        let room = DICTIONARY_SIZE - dictionary.len();
        dictionary.extend_from_slice(&bytes[..bytes.len().min(room).min(4096)]);
        if dictionary.len() >= DICTIONARY_SIZE {
            break;
        }
    }
    Ok(dictionary)
}

/// What a file in the program has in it, made whole again if it's kept squeezed.
fn packed_bytes(packed: &PackedFiles, &(place, size, kept): &(u64, u32, u32)) -> io::Result<Vec<u8>> {
    if kept == 0 {
        let mut bytes = vec![0; size as usize];
        read_at(&packed.program, &mut bytes, place)?;
        return Ok(bytes);
    }
    let broken = || io::Error::new(io::ErrorKind::InvalidData, "a file inside the program is damaged");
    let dictionary = packed
        .dictionary_read
        .get_or_init(|| {
            let (place, size) = packed.dictionary?;
            let mut bytes = vec![0; usize::try_from(size).ok()?];
            read_at(&packed.program, &mut bytes, place).ok()?;
            Some(bytes)
        })
        .as_deref()
        .ok_or_else(broken)?;
    let mut frame = vec![0; kept as usize];
    read_at(&packed.program, &mut frame, place)?;
    crate::builtins::host::zlib::zstd_unframe(&frame, size as usize, Some(dictionary)).ok_or_else(broken)
}

/// Where `path` really is, with any folder in it that was a link when the program was packed swapped for the one it led
/// to, as asking Windows for a full path would. Links are only ever what's in a `node_modules`, or in a scope in one, so
/// only those parts of a path are looked up. `None` if it goes through none.
fn through_links(packed: &PackedFiles, path: &Path) -> Option<PathBuf> {
    if packed.links.is_empty() {
        return None;
    }
    let mut now = tidied(path);
    let mut changed = false;
    // What a link leads to is where it really is, so it isn't through another, but a few more are allowed for anyway,
    // and no more, in case some are made into a loop.
    'again: for _ in 0..8 {
        let parts: Vec<Component> = now.components().collect();
        for (at, part) in parts.iter().enumerate() {
            if !part.as_os_str().eq_ignore_ascii_case("node_modules") {
                continue;
            }
            let Some(name) = parts.get(at + 1) else {
                break;
            };
            let length = if name.as_os_str().to_string_lossy().starts_with('@') {
                at + 3
            } else {
                at + 2
            };
            if length > parts.len() {
                break;
            }
            let link: PathBuf = parts[..length].iter().collect();
            if let Some(target) = packed.links.get(&key(&link)) {
                let mut led_to = target.clone();
                led_to.extend(&parts[length..]);
                now = led_to;
                changed = true;
                continue 'again;
            }
        }
        break;
    }
    changed.then_some(now)
}

/// Where `path` really is, if it's inside this program: as [`tidied`] has it, with any link it goes through followed.
pub(crate) fn real_path(path: &Path) -> PathBuf {
    match PACKED.get().and_then(|packed| through_links(packed, path)) {
        Some(path) => path,
        None => tidied(path),
    }
}

/// How this program's files are looked up for a path, with the links it goes through followed.
fn packed_key(packed: &PackedFiles, path: &Path) -> String {
    match through_links(packed, path) {
        Some(path) => key(&path),
        None => key(path),
    }
}

/// What's in a file inside this program, if it's a packed program and the file's in it.
pub fn read_packed_file(path: &Path) -> Option<Vec<u8>> {
    let packed = PACKED.get()?;
    let found = packed.files.get(&packed_key(packed, path))?;
    packed_bytes(packed, found).ok()
}

/// A bounded view of one entry. Plain entries share the executable using positioned reads;
/// dictionary-compressed source entries are inflated once, as when loading a module.
pub(crate) fn open_packed_reader(path: &Path, start: u64) -> Option<io::Result<Box<dyn Read + Send>>> {
    let packed = PACKED.get()?;
    let found = packed.files.get(&packed_key(packed, path))?;
    Some(packed_reader(packed, found, start))
}

fn packed_reader<'a>(packed: &'a PackedFiles, found: &(u64, u32, u32), start: u64) -> io::Result<Box<dyn Read + Send + 'a>> {
    let (place, size, kept) = *found;
    let start = start.min(u64::from(size));
    if start == u64::from(size) {
        return Ok(Box::new(io::empty()));
    }
    if kept != 0 {
        let mut cursor = io::Cursor::new(packed_bytes(packed, found)?);
        cursor.set_position(start);
        return Ok(Box::new(cursor));
    }
    struct Entry<'a> {
        file: &'a File,
        at: u64,
        left: u64,
    }
    impl Read for Entry<'_> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            let count = self.left.min(out.len() as u64) as usize;
            read_at(self.file, &mut out[..count], self.at)?;
            self.at += count as u64;
            self.left -= count as u64;
            Ok(count)
        }
    }
    Ok(Box::new(Entry { file: &packed.program, at: place + start, left: u64::from(size) - start }))
}

/// The block of compiled code inside this program, if it's packed with one, and the folder its files' paths are from.
pub(crate) fn read_code_block() -> Option<(crate::code_bytes::CodeBytes, PathBuf)> {
    let packed = PACKED.get()?;
    let (place, size) = packed.code?;
    let bytes = crate::code_bytes::CodeBytes::read(&packed.program, place, size).ok()?;
    Some((bytes, packed.root.clone()))
}

/// What a file inside this program is looked up by, if it's a packed program.
pub(crate) fn lookup_key(path: &Path) -> Option<String> {
    PACKED.get().map(|packed| packed_key(packed, path))
}

/// The files that were run when the program was packed, in the order they were, if it's a packed program.
pub(crate) fn run_order() -> Vec<PathBuf> {
    PACKED
        .get()
        .map_or_else(Vec::new, |packed| packed.run_order.clone())
}

/// The modules that come with the engine that were run when the program was packed, in the order they were.
pub(crate) fn builtins_run() -> Vec<String> {
    PACKED
        .get()
        .map_or_else(Vec::new, |packed| packed.builtins.clone())
}

/// How long a file inside the program is, if it's there.
pub(crate) fn packed_size(path: &Path) -> Option<u32> {
    let packed = PACKED.get()?;
    packed
        .files
        .get(&packed_key(packed, path))
        .map(|&(_, size, _)| size)
}

/// Whether there's a file here, inside the program.
pub(crate) fn is_packed_file(path: &Path) -> bool {
    PACKED
        .get()
        .is_some_and(|packed| packed.files.get(&packed_key(packed, path)).is_some())
}

/// Whether there's a folder here with files in it, inside the program.
pub(crate) fn is_packed_folder(path: &Path) -> bool {
    PACKED
        .get()
        .is_some_and(|packed| packed.folders.contains(&packed_key(packed, path)))
}

/// What's in a file, from inside the program if it's there, and otherwise from the disk.
pub(crate) fn read_file(path: &Path) -> io::Result<String> {
    if let Some(packed) = PACKED.get() {
        if let Some(found) = packed.files.get(&packed_key(packed, path)) {
            let bytes = packed_bytes(packed, found)?;
            return Ok(match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
            });
        }
    }
    // As Node does, what isn't really UTF-8 gets read as far as it can be, as some packages are in older encodings.
    Ok(match String::from_utf8(std::fs::read(path)?) {
        Ok(text) => text,
        Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
    })
}

/// Fills `bytes` from `file`, starting `at` bytes in, without moving where it's at, so threads can share it.
pub(crate) fn read_at(file: &File, mut bytes: &mut [u8], mut at: u64) -> io::Result<()> {
    #[cfg(windows)]
    use std::os::windows::fs::FileExt;
    #[cfg(unix)]
    use std::os::unix::fs::FileExt;
    while !bytes.is_empty() {
        #[cfg(windows)]
        let read = file.seek_read(bytes, at);
        #[cfg(unix)]
        let read = file.read_at(bytes, at);
        match read? {
            0 => return Err(io::ErrorKind::UnexpectedEof.into()),
            read => {
                bytes = &mut bytes[read..];
                at += read as u64;
            }
        }
    }
    Ok(())
}

/// A path with what can be worked out from it worked out, for one that can't be asked about on the disk.
pub(crate) fn tidied(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn paths_that_mean_the_same_are_the_same() {
        assert_eq!(
            key(Path::new(r"C:\App\.\src\..\Main.TSX")),
            key(Path::new(r"c:\app\main.tsx"))
        );
        assert_eq!(key(Path::new("C:/app/main.tsx")), r"c:\app\main.tsx");
    }

    #[test]
    fn files_on_the_end_of_a_program_can_be_found_again() {
        let folder = std::env::temp_dir().join(format!("rsui-packed-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("sub")).unwrap();
        std::fs::write(folder.join("main.tsx"), "the main file").unwrap();
        std::fs::write(folder.join("sub").join("other.js"), "\u{4F60} other").unwrap();
        let program = folder.join("program.exe");
        std::fs::write(&program, b"what the program is").unwrap();
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&program)
            .unwrap();
        let files = [folder.join("main.tsx"), folder.join("sub").join("other.js")];
        // What was run, in the order it was, without what wasn't packed.
        let ran = [
            files[1].clone(),
            folder.join("elsewhere.js"),
            files[0].clone(),
        ];
        let builtins = ["react".to_owned(), "events".to_owned()];
        let links = [(
            folder.join("node_modules").join("pkg"),
            folder.join("node_modules").join(".pnpm").join("pkg@1"),
        )];
        append_packed_files(
            &mut file, &folder, &files[0], &files, &ran, &builtins, &links, None,
        )
        .unwrap();
        drop(file);
        let list = read_list(&program).unwrap();
        // Paths in the list are as the system writes them.
        let native = |path: &str| path.replace('\\', std::path::MAIN_SEPARATOR_STR);
        assert_eq!(list.entry, "main.tsx");
        assert_eq!(list.run_order, [native(r"sub\other.js"), "main.tsx".to_owned()]);
        assert_eq!(list.builtins, builtins);
        assert_eq!(
            list.links,
            [(native(r"node_modules\pkg"), native(r"node_modules\.pnpm\pkg@1"))]
        );
        assert_eq!(list.files.len(), 2);
        assert_eq!(PackedList::file_name(&list.bytes, &list.files[1].0), native(r"sub\other.js"));
        assert_eq!(list.code, None);
        let bytes = std::fs::read(&program).unwrap();
        let (place, size, kept) = list.files[1].1;
        // What isn't in a package is kept as it is.
        assert_eq!(kept, 0);
        let found = &bytes[place as usize..place as usize + size as usize];
        assert_eq!(std::str::from_utf8(found).unwrap(), "\u{4F60} other");
        // A file with nothing on it, or something too short to say, is just a program.
        drop(list);
        std::fs::write(&program, b"short").unwrap();
        assert!(read_list(&program).is_none());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    #[cfg(windows)]
    fn a_path_through_a_link_in_node_modules_is_where_the_link_leads() {
        let pnpm = Path::new(r"C:\app\node_modules\.pnpm");
        let links = [
            (
                r"C:\app\node_modules\pkg",
                pnpm.join(r"pkg@1\node_modules\pkg"),
            ),
            // What a package needs, next to it, is a link too.
            (
                r"C:\app\node_modules\.pnpm\pkg@1\node_modules\dep",
                pnpm.join(r"dep@2\node_modules\dep"),
            ),
            (
                r"C:\app\node_modules\@scope\thing",
                pnpm.join(r"@scope+thing@3\node_modules\@scope\thing"),
            ),
            // A workspace's own package is somewhere else altogether.
            (
                r"C:\app\node_modules\shared",
                PathBuf::from(r"C:\app\packages\shared"),
            ),
        ];
        let packed = PackedFiles {
            program: File::open(std::env::current_exe().unwrap()).unwrap(),
            files: FileIndex::default(),
            folders: HashSet::new(),
            run_order: Vec::new(),
            builtins: Vec::new(),
            links: links
                .iter()
                .map(|(link, target)| (key(Path::new(link)), target.clone()))
                .collect(),
            root: PathBuf::new(),
            code: None,
            diagnostics: false,
            dictionary: None,
            dictionary_read: OnceLock::new(),
        };
        let through = |path: &str| through_links(&packed, Path::new(path));
        assert_eq!(
            through(r"C:\app\node_modules\pkg\lib\index.js"),
            Some(pnpm.join(r"pkg@1\node_modules\pkg\lib\index.js"))
        );
        // Letters are the same in any case, as they are to Windows, and what's worked out from the path is.
        assert_eq!(
            through(r"c:\APP\src\..\Node_Modules\PKG\.\index.js"),
            Some(pnpm.join(r"pkg@1\node_modules\pkg\index.js"))
        );
        assert_eq!(
            through(r"C:\app\node_modules\.pnpm\pkg@1\node_modules\dep\index.js"),
            Some(pnpm.join(r"dep@2\node_modules\dep\index.js"))
        );
        assert_eq!(
            through(r"C:\app\node_modules\@scope\thing\package.json"),
            Some(pnpm.join(r"@scope+thing@3\node_modules\@scope\thing\package.json"))
        );
        assert_eq!(
            through(r"C:\app\node_modules\shared\dist\index.js"),
            Some(PathBuf::from(r"C:\app\packages\shared\dist\index.js"))
        );
        assert_eq!(
            through(r"C:\app\node_modules\pkg"),
            Some(pnpm.join(r"pkg@1\node_modules\pkg"))
        );
        // What isn't through one is where it says.
        assert_eq!(through(r"C:\app\node_modules\other\index.js"), None);
        assert_eq!(through(r"C:\app\src\node_modules"), None);
        assert_eq!(through(r"C:\app\node_modules\@scope"), None);
    }

    #[test]
    fn a_packages_code_that_didnt_run_is_kept_squeezed_and_reads_the_same() {
        let folder = std::env::temp_dir().join(format!("rsui-packed-squeezed-{}", std::process::id()));
        let package = folder.join("node_modules").join("icons");
        std::fs::create_dir_all(&package).unwrap();
        let mut files = vec![folder.join("main.tsx"), package.join("package.json"), package.join("index.js")];
        std::fs::write(&files[0], "import { Star } from 'icons';").unwrap();
        std::fs::write(&files[1], r#"{ "name": "icons", "sideEffects": false, "description": "a package of icons, each its own module" }"#).unwrap();
        std::fs::write(&files[2], "export { default as Star } from './star.js';\n".repeat(30)).unwrap();
        for name in ["star", "moon", "sun", "cloud", "tiny"] {
            let path = package.join(format!("{name}.js"));
            let text = if name == "tiny" {
                "x".to_owned()
            } else {
                format!("import {{ make }} from './make.js';\nexport default make('{name}', 'M13.91 2.19c-.76-1.59-3.05-1.59-3.82 0L8.07 6.37a.11.11 0 0 1-.09.06l-4.63.6');\n").repeat(8)
            };
            std::fs::write(&path, text).unwrap();
            files.push(path);
        }
        files.sort();
        let entry = folder.join("main.tsx");
        let ran = [entry.clone(), package.join("index.js"), package.join("star.js")];
        let program = folder.join("program.exe");
        std::fs::write(&program, b"what the program is").unwrap();
        let mut file = std::fs::OpenOptions::new().read(true).write(true).open(&program).unwrap();
        append_packed_files(&mut file, &folder, &entry, &files, &ran, &[], &[], None).unwrap();
        drop(file);
        let list = read_list(&program).unwrap();
        assert!(list.dictionary.is_some());
        let mut index = FileIndex::default();
        for (name, place) in &list.files {
            let path = PackedList::file_name(&list.bytes, name);
            index.insert(&key(&folder.join(path)), *place);
        }
        let packed = PackedFiles {
            program: File::open(&program).unwrap(),
            files: index,
            folders: HashSet::new(),
            run_order: Vec::new(),
            builtins: Vec::new(),
            links: HashMap::new(),
            root: folder.clone(),
            code: None,
            diagnostics: false,
            dictionary: list.dictionary,
            dictionary_read: OnceLock::new(),
        };
        for path in &files {
            let found = *packed.files.get(&key(path)).unwrap();
            let squeezed = found.2 != 0;
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            // What ran, what isn't a package's code, its manifest, and what wouldn't get smaller are as they are.
            let expected = matches!(name.as_str(), "moon.js" | "sun.js" | "cloud.js");
            assert_eq!(squeezed, expected, "{name}");
            if squeezed {
                assert!(found.2 < found.1 / 2, "{name}: {found:?}");
            }
            let expected = std::fs::read(path).unwrap();
            assert_eq!(packed_bytes(&packed, &found).unwrap(), expected, "{name}");
            for offset in [0, 3, expected.len() + 1] {
                let mut range = Vec::new();
                packed_reader(&packed, &found, offset as u64).unwrap().take(7).read_to_end(&mut range).unwrap();
                assert_eq!(range, expected[offset.min(expected.len())..(offset + 7).min(expected.len())], "{name}");
            }
        }
        // A squeezed file that's been damaged isn't read as what it was, as one kept as it is wouldn't be. (Neither has
        // a sum of its own to be checked against.)
        let mut bytes = std::fs::read(&program).unwrap();
        let moon = *packed.files.get(&key(&package.join("moon.js"))).unwrap();
        bytes[moon.0 as usize + moon.2 as usize / 2] ^= 0x55;
        let dictionary = list.dictionary;
        drop(list);
        drop(packed);
        std::fs::write(&program, &bytes).unwrap();
        let packed = PackedFiles {
            program: File::open(&program).unwrap(),
            files: FileIndex::default(),
            folders: HashSet::new(),
            run_order: Vec::new(),
            builtins: Vec::new(),
            links: HashMap::new(),
            root: folder.clone(),
            code: None,
            diagnostics: false,
            dictionary,
            dictionary_read: OnceLock::new(),
        };
        assert!(packed_bytes(&packed, &moon).map_or(true, |read| read != std::fs::read(package.join("moon.js")).unwrap()));
        drop(packed);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_program_with_nothing_on_it_has_nothing_to_run() {
        assert!(read_list(&std::env::current_exe().unwrap()).is_none());
    }

    #[test]
    #[cfg(windows)]
    fn tidying_a_path_works_out_the_dots() {
        assert_eq!(
            tidied(Path::new(r"C:\app\.\src\..\main.tsx")),
            Path::new(r"C:\app\main.tsx")
        );
    }
}
