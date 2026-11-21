//! Writes two things for the code cache (src/codecache.rs) into `OUT_DIR`:
//!
//! - `build_id.rs`: a number that changes whenever anything in this crate does, which a cache is made for, so a cache made
//!   by another build of the engine is never used.
//! - `op_codec.rs`: how each instruction in src/op.rs is written and read field by field, and a list of them all, made from
//!   op.rs itself so that a new instruction is written the moment it's added, and the code cache's test sees it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let here = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo says where the crate is"),
    );
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo says where to write"));
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");

    // Where the JIT has a backend (src/jit/mod.rs).
    println!("cargo:rustc-check-cfg=cfg(jit_backend)");
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if arch == "x86_64" {
        println!("cargo:rustc-cfg=jit_backend");
    }

    let mut files = Vec::new();
    list_files(&here.join("src"), &mut files);
    files.push(here.join("build.rs"));
    files.push(here.join("Cargo.toml"));
    files.sort();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for file in &files {
        let name = file
            .strip_prefix(&here)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        mix(&mut hash, name.as_bytes());
        mix(&mut hash, &[0]);
        let bytes = std::fs::read(file).unwrap_or_default();
        mix(&mut hash, &(bytes.len() as u64).to_le_bytes());
        mix(&mut hash, &bytes);
    }
    // How it's built matters too: what's compiled in and for what.
    for name in [
        "TARGET",
        "PROFILE",
        "CARGO_PKG_VERSION",
        "OPT_LEVEL",
        "DEBUG",
    ] {
        mix(
            &mut hash,
            std::env::var(name).unwrap_or_default().as_bytes(),
        );
        mix(&mut hash, &[0]);
    }
    let mut features: Vec<String> = std::env::vars()
        .map(|(name, _)| name)
        .filter(|name| name.starts_with("CARGO_FEATURE_"))
        .collect();
    features.sort();
    for feature in features {
        mix(&mut hash, feature.as_bytes());
        mix(&mut hash, &[0]);
    }
    std::fs::write(
        out.join("build_id.rs"),
        format!("/// Which build of the engine this is. See build.rs.\npub(crate) const BUILD_ID: u64 = 0x{hash:016x};\n"),
    )
    .expect("writing build_id.rs");

    let op = std::fs::read_to_string(here.join("src").join("op.rs")).expect("reading src/op.rs");
    let kinds = parse_ops(&op);
    std::fs::write(out.join("op_codec.rs"), codec(&kinds)).expect("writing op_codec.rs");
}

fn list_files(folder: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            list_files(&path, files);
        } else {
            files.push(path);
        }
    }
}

fn mix(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
}

/// An instruction: its name, and its fields' names and types, in order.
type Kind = (String, Vec<(String, String)>);

/// The instructions in op.rs's `enum Op`, which is plain: unit variants and variants with named fields of integer types.
fn parse_ops(source: &str) -> Vec<Kind> {
    let start = source
        .find("pub(crate) enum Op {")
        .expect("op.rs has `pub(crate) enum Op {`");
    let body = &source[start + "pub(crate) enum Op {".len()..];
    // Comments go first, then what's left is words and punctuation.
    let mut text = String::new();
    for line in body.lines() {
        let line = match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        };
        text.push_str(line);
        text.push('\n');
    }
    let mut tokens = Vec::new();
    let mut word = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() || character == '_' {
            word.push(character);
            continue;
        }
        if !word.is_empty() {
            tokens.push(std::mem::take(&mut word));
        }
        if !character.is_whitespace() {
            tokens.push(character.to_string());
        }
    }
    let mut kinds = Vec::new();
    let mut at = 0;
    let next = |at: &mut usize| {
        let token = tokens.get(*at).cloned().expect("op.rs's `enum Op` ends");
        *at += 1;
        token
    };
    loop {
        let token = next(&mut at);
        match token.as_str() {
            "}" => break,
            "#" => {
                // An attribute: skipped to its closing bracket.
                assert_eq!(next(&mut at), "[", "an attribute in op.rs");
                let mut depth = 1;
                while depth > 0 {
                    match next(&mut at).as_str() {
                        "[" => depth += 1,
                        "]" => depth -= 1,
                        _ => {}
                    }
                }
            }
            name => {
                assert!(
                    name.chars().next().is_some_and(char::is_uppercase),
                    "build.rs can't read op.rs's `enum Op` at {name:?}: teach it"
                );
                let mut fields = Vec::new();
                match next(&mut at).as_str() {
                    "," => {}
                    "{" => {
                        loop {
                            let field = next(&mut at);
                            if field == "}" {
                                break;
                            }
                            assert_eq!(next(&mut at), ":", "a field in op.rs's {name}");
                            let kind = next(&mut at);
                            assert!(
                                size_of_type(&kind).is_some(),
                                "op.rs's {name}.{field} is a {kind}, which build.rs doesn't know how to write: teach it"
                            );
                            fields.push((field, kind));
                            match next(&mut at).as_str() {
                                "," => {}
                                "}" => break,
                                other => panic!(
                                    "build.rs can't read op.rs's {name} at {other:?}: teach it"
                                ),
                            }
                        }
                        // The comma after a variant with fields can only be left out at the end.
                        if tokens.get(at).map(String::as_str) == Some(",") {
                            at += 1;
                        }
                    }
                    "}" => {
                        kinds.push((name.to_owned(), fields));
                        break;
                    }
                    other => panic!("build.rs can't read op.rs's {name} at {other:?}: teach it"),
                }
                kinds.push((name.to_owned(), fields));
            }
        }
    }
    assert!(
        kinds.len() <= 256,
        "there are more than 256 instructions: write their kinds in two bytes"
    );
    kinds
}

fn size_of_type(kind: &str) -> Option<usize> {
    Some(match kind {
        "u8" | "i8" | "bool" => 1,
        "u16" | "i16" => 2,
        "u32" | "i32" => 4,
        "u64" | "i64" => 8,
        _ => return None,
    })
}

fn codec(kinds: &[Kind]) -> String {
    let mut out = String::from("// Made by build.rs from src/op.rs. See build.rs.\n\n");
    out.push_str(
        "/// Every kind of instruction, in op.rs's order, with its fields' names and types.\n",
    );
    out.push_str("#[cfg_attr(not(test), allow(dead_code))]\n");
    out.push_str("pub(crate) const OP_KINDS: &[(&str, &[(&str, &str)])] = &[\n");
    for (name, fields) in kinds {
        let fields: Vec<String> = fields
            .iter()
            .map(|(field, kind)| format!("(\"{field}\", \"{kind}\")"))
            .collect();
        let _ = writeln!(out, "    (\"{name}\", &[{}]),", fields.join(", "));
    }
    out.push_str("];\n\n");
    out.push_str("/// Writes an instruction: its kind's number in [`OP_KINDS`], then each field, little-endian.\n");
    out.push_str("pub(crate) fn write_op(out: &mut Vec<u8>, op: &Op) {\n    match *op {\n");
    for (number, (name, fields)) in kinds.iter().enumerate() {
        let names: Vec<&str> = fields.iter().map(|(field, _)| field.as_str()).collect();
        let pattern = if fields.is_empty() {
            format!("Op::{name}")
        } else {
            format!("Op::{name} {{ {} }}", names.join(", "))
        };
        let _ = write!(
            out,
            "        {pattern} => {{\n            out.push({number});\n"
        );
        for (field, kind) in fields {
            if kind == "bool" {
                let _ = writeln!(out, "            out.push(u8::from({field}));");
            } else {
                let _ = writeln!(
                    out,
                    "            out.extend_from_slice(&{field}.to_le_bytes());"
                );
            }
        }
        out.push_str("        }\n");
    }
    out.push_str("    }\n}\n\n");
    out.push_str(
        "/// Reads an instruction [`write_op`] wrote, or `None` if there isn't one there.\n",
    );
    out.push_str("pub(crate) fn read_op(from: &mut &[u8]) -> Option<Op> {\n");
    out.push_str(
        "    let (&kind, rest) = from.split_first()?;\n    *from = rest;\n    Some(match kind {\n",
    );
    for (number, (name, fields)) in kinds.iter().enumerate() {
        if fields.is_empty() {
            let _ = writeln!(out, "        {number} => Op::{name},");
            continue;
        }
        let values: Vec<String> = fields
            .iter()
            .map(|(field, kind)| {
                let size = size_of_type(kind).expect("checked when read");
                if kind == "bool" {
                    format!("{field}: take::<1>(from)?[0] != 0")
                } else {
                    format!("{field}: {kind}::from_le_bytes(take::<{size}>(from)?)")
                }
            })
            .collect();
        // Struct fields are worked out in the order they're written, which is the order they were.
        let _ = writeln!(
            out,
            "        {number} => Op::{name} {{ {} }},",
            values.join(", ")
        );
    }
    out.push_str("        _ => return None,\n    })\n}\n\n");
    out.push_str("fn take<const N: usize>(from: &mut &[u8]) -> Option<[u8; N]> {\n");
    out.push_str("    let (bytes, rest) = from.split_first_chunk::<N>()?;\n    *from = rest;\n    Some(*bytes)\n}\n");
    out
}
