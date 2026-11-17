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

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

const MAGIC: &[u8; 8] = b"RSUIPACK";
const FOOTER: u64 = 8 + 8 + 8;

struct PackedFiles {
    /// The program, open for reading what's in it, which each file is read from where it is.
    program: File,
    /// Where each file is in the program, by its path as [`key`] writes it.
    files: HashMap<String, (u64, u32)>,
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

/// A file's path, and how far into the program it starts and how long it is.
type PackedFile = (String, (u64, u32));

/// A folder that was a link when the program was packed, and the folder it led to.
pub type PackedLink = (PathBuf, PathBuf);

/// What's on the end of a program: which file to run, every file with where it is, which were run when it was
/// packed, in the order they were, the modules that come with the engine that were, and the folders that were links.
struct PackedList {
    entry: String,
    files: Vec<PackedFile>,
    run_order: Vec<String>,
    builtins: Vec<String>,
    links: Vec<(String, String)>,
    code: Option<(u64, u64)>,
    diagnostics: bool,
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
        files: HashMap::with_capacity(list.files.len()),
        folders: HashSet::new(),
        run_order: list.run_order.iter().map(|path| root.join(path)).collect(),
        builtins: list.builtins,
        links: HashMap::with_capacity(list.links.len()),
        root: root.to_owned(),
        code: list.code,
        diagnostics: list.diagnostics,
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
    for (path, place) in list.files {
        let path = root.join(path);
        add_folders(&path, &mut packed.folders);
        packed.files.insert(key(&path), place);
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
    file.seek(SeekFrom::Start(at)).ok()?;
    let mut list = vec![0u8; usize::try_from(length).ok()?];
    file.read_exact(&mut list).ok()?;
    let mut rest = list.as_slice();
    let entry = read_text(&mut rest)?.to_owned();
    let count = read_u32(&mut rest)?;
    let mut files = Vec::with_capacity((count as usize).min(rest.len() / 16));
    for _ in 0..count {
        let path = read_text(&mut rest)?.to_owned();
        let place = read_u64(&mut rest)?;
        let size = read_u32(&mut rest)?;
        files.push((path, (place, size)));
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
    Some(PackedList {
        entry,
        files,
        run_order,
        builtins,
        links,
        code,
        diagnostics: read_u32(&mut rest).unwrap_or(0) & 1 != 0,
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
    for path in files {
        let bytes = crate::source_overlay::bytes(path)?;
        to.write_all(&bytes)?;
        add_text(&mut list, &inside(root, path)?);
        list.extend_from_slice(&at.to_le_bytes());
        list.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        at += bytes.len() as u64;
        if let Ok(size) = u32::try_from(bytes.len()) {
            sizes.insert(path.clone(), size);
        }
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
    to.write_all(&list)?;
    to.write_all(&at.to_le_bytes())?;
    to.write_all(&(list.len() as u64).to_le_bytes())?;
    to.write_all(MAGIC)?;
    Ok(())
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
    let &(place, size) = packed.files.get(&packed_key(packed, path))?;
    let mut bytes = vec![0; size as usize];
    read_at(&packed.program, &mut bytes, place).ok()?;
    Some(bytes)
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
        .map(|&(_, size)| size)
}

/// Whether there's a file here, inside the program.
pub(crate) fn is_packed_file(path: &Path) -> bool {
    PACKED
        .get()
        .is_some_and(|packed| packed.files.contains_key(&packed_key(packed, path)))
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
        if let Some(&(place, size)) = packed.files.get(&packed_key(packed, path)) {
            let mut bytes = vec![0; size as usize];
            read_at(&packed.program, &mut bytes, place)?;
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
    use std::os::windows::fs::FileExt;
    while !bytes.is_empty() {
        match file.seek_read(bytes, at)? {
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
        assert_eq!(list.entry, "main.tsx");
        assert_eq!(list.run_order, [r"sub\other.js", "main.tsx"]);
        assert_eq!(list.builtins, builtins);
        assert_eq!(
            list.links,
            [(
                r"node_modules\pkg".to_owned(),
                r"node_modules\.pnpm\pkg@1".to_owned()
            )]
        );
        assert_eq!(list.files.len(), 2);
        assert_eq!(list.files[1].0, r"sub\other.js");
        assert_eq!(list.code, None);
        let bytes = std::fs::read(&program).unwrap();
        let (place, size) = list.files[1].1;
        let found = &bytes[place as usize..place as usize + size as usize];
        assert_eq!(std::str::from_utf8(found).unwrap(), "\u{4F60} other");
        // A file with nothing on it, or something too short to say, is just a program.
        std::fs::write(&program, b"short").unwrap();
        assert!(read_list(&program).is_none());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
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
            files: HashMap::new(),
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
    fn a_program_with_nothing_on_it_has_nothing_to_run() {
        assert!(read_list(&std::env::current_exe().unwrap()).is_none());
    }

    #[test]
    fn tidying_a_path_works_out_the_dots() {
        assert_eq!(
            tidied(Path::new(r"C:\app\.\src\..\main.tsx")),
            Path::new(r"C:\app\main.tsx")
        );
    }
}
