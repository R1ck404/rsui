//! The libraries the program loads only once something in them is called (`/DELAYLOAD` in `.cargo/config.toml`). Each has
//! one table of what's called in it, which the linker's delay-load helper looks a function up in the first time it's
//! called. A function declared by hand as `#[link(name = "shell32")]` comes from the system's SHELL32.lib, and if the
//! `windows` crate doesn't import it too, it ends up in no table, and calling it crashes. `shell.reveal` did, in a real
//! window.

use std::path::{Path, PathBuf};

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// The libraries that are loaded late, as `.cargo/config.toml` names them, without `.dll`.
fn loaded_late() -> Vec<String> {
    let config = std::fs::read_to_string(repo().join(".cargo").join("config.toml")).unwrap();
    config
        .split("/DELAYLOAD:")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .map(|name| name.trim_end_matches(".dll").to_ascii_lowercase())
        .collect()
}

fn u16_at(bytes: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> usize {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
}

/// Each delay-loaded library's name, in lower case, with the names of the functions in its table.
fn delay_tables(program: &[u8]) -> Vec<(String, Vec<String>)> {
    let pe = u32_at(program, 0x3C);
    let sections = u16_at(program, pe + 6);
    let optional = pe + 24;
    let section_table = optional + u16_at(program, pe + 20);
    // Where something the program's loaded at is in the file.
    let offset = |rva: usize| {
        (0..sections)
            .map(|at| section_table + at * 40)
            .find_map(|section| {
                let (start, size, raw) = (u32_at(program, section + 12), u32_at(program, section + 8), u32_at(program, section + 20));
                (rva >= start && rva < start + size).then(|| raw + rva - start)
            })
            .unwrap()
    };
    let text = |rva: usize| {
        let from = offset(rva);
        let to = from + program[from..].iter().position(|&byte| byte == 0).unwrap();
        String::from_utf8_lossy(&program[from..to]).into_owned()
    };
    // The 14th of the optional header's directories, after its 112 bytes of fields, is the delay-load one.
    let mut descriptor = offset(u32_at(program, optional + 112 + 13 * 8));
    let mut out = Vec::new();
    while u32_at(program, descriptor + 4) != 0 {
        let name = text(u32_at(program, descriptor + 4)).to_ascii_lowercase();
        let mut names = Vec::new();
        let mut entry = offset(u32_at(program, descriptor + 16));
        loop {
            let value = u64::from_le_bytes(program[entry..entry + 8].try_into().unwrap());
            if value == 0 {
                break;
            }
            if value >> 63 == 0 {
                // A name, after two bytes of hint.
                names.push(text(value as usize + 2));
            }
            entry += 8;
        }
        out.push((name, names));
        descriptor += 32;
    }
    out
}

#[test]
fn functions_declared_by_hand_from_a_library_that_is_loaded_late_are_in_its_table() {
    let tables = delay_tables(&std::fs::read(TOOL).unwrap());
    let has = |library: &str, function: &str| tables.iter().any(|(name, names)| name == library && names.iter().any(|one| one == function));
    // Neither is imported by the `windows` crate with the features rsui has, so these are what the hand-written
    // declarations give. Both were in no table before.
    assert!(has("shell32.dll", "ShellExecuteExW"), "{tables:?}");
    assert!(has("user32.dll", "MessageBeep"), "{tables:?}");
    // Each library has one table.
    for (at, (name, _)) in tables.iter().enumerate() {
        assert!(!tables[..at].iter().any(|(other, _)| other == name), "{name} has two tables");
    }
}

/// Every `.rs` file under a folder.
fn sources(folder: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(folder).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_library_that_is_loaded_late_is_linked_from_the_system_s_import_library() {
    let late = loaded_late();
    assert!(late.contains(&"shell32".to_owned()) && late.contains(&"user32".to_owned()), "{late:?}");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(repo().join("crates")).unwrap().flatten() {
        let src = entry.path().join("src");
        if src.is_dir() {
            sources(&src, &mut files);
        }
    }
    let mut wrong = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for library in &late {
            // ws2_32, which the standard library imports, and advapi32, which only rsui's own declarations do, come
            // only from the system's import libraries, so each is one table that way, and has to stay that way.
            if library == "ws2_32" || library == "advapi32" {
                continue;
            }
            if text.contains(&format!("#[link(name = \"{library}\")]")) {
                wrong.push(format!("{} links {library} from its import library", file.display()));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
