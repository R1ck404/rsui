//! The fonts of systems other than Windows: Linux, Android and macOS. Which font is the UI's, which ones have the
//! characters it doesn't, and what's installed by name.
//!
//! Nothing here asks fontconfig or CoreText. The fonts a system comes with are in a few well-known places, which differ
//! between Linux distributions, so each fallback is a list of the files it might be, and the text engine tries them in
//! order only once some text needs one (a file that isn't there costs one failed open, once). Finding a font by its
//! name reads the names of every installed font, which is only done the first time a name is asked for.

use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use crate::font::{read_u16, read_u32};

type Ranges = &'static [RangeInclusive<u32>];

/// The folders fonts are installed in, the system's first and then the person's own, which go on top.
pub(crate) fn font_folders() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "android") {
        folders.extend(["/system/fonts", "/product/fonts", "/system/font"].map(PathBuf::from));
    } else if cfg!(target_vendor = "apple") {
        folders.extend(
            ["/System/Library/Fonts", "/System/Library/Fonts/Supplemental", "/Library/Fonts"].map(PathBuf::from),
        );
        if let Some(home) = &home {
            folders.push(home.join("Library/Fonts"));
        }
    } else {
        let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
        let mut system: Vec<PathBuf> =
            data_dirs.split(':').filter(|dir| !dir.is_empty()).map(|dir| Path::new(dir).join("fonts")).collect();
        for usual in ["/usr/share/fonts", "/usr/local/share/fonts"] {
            if !system.iter().any(|dir| dir == Path::new(usual)) {
                system.push(PathBuf::from(usual));
            }
        }
        folders.extend(system);
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
        if let Some(data_home) = data_home {
            folders.push(data_home.join("fonts"));
        }
        if let Some(home) = &home {
            folders.push(home.join(".fonts"));
        }
    }
    folders
}

/// Where the UI font might be, in regular and bold, best first. The first that opens is used.
pub fn ui_font_candidates() -> (&'static [&'static str], &'static [&'static str]) {
    if cfg!(target_os = "android") {
        (
            &["/system/fonts/Roboto-Regular.ttf", "/system/fonts/RobotoStatic-Regular.ttf", "/system/fonts/DroidSans.ttf"],
            &["/system/fonts/Roboto-Bold.ttf", "/system/fonts/RobotoStatic-Bold.ttf", "/system/fonts/DroidSans-Bold.ttf"],
        )
    } else if cfg!(target_vendor = "apple") {
        (
            &["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Helvetica.ttc", "/Library/Fonts/Arial.ttf"],
            &["/System/Library/Fonts/Supplemental/Arial Bold.ttf", "/Library/Fonts/Arial Bold.ttf"],
        )
    } else {
        (
            &[
                "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/google-noto-vf/NotoSans[wght].ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                "/usr/share/fonts/liberation-sans/LiberationSans-Regular.ttf",
            ],
            &[
                "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
                "/usr/share/fonts/noto/NotoSans-Bold.ttf",
                "/usr/share/fonts/google-noto/NotoSans-Bold.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
                "/usr/share/fonts/liberation-sans/LiberationSans-Bold.ttf",
            ],
        )
    }
}

/// The first of these that's a file.
pub fn first_that_exists(candidates: &'static [&'static str]) -> Option<&'static str> {
    candidates.iter().copied().find(|path| Path::new(path).is_file())
}

const HAN_AND_KANA: Ranges = &[
    0x2E80..=0x2FDF,
    0x3000..=0x30FF,
    0x3100..=0x312F,
    0x31A0..=0x31FF,
    0x3200..=0x33FF,
    0x3400..=0x4DBF,
    0x4E00..=0x9FFF,
    0xF900..=0xFAFF,
    0xFE30..=0xFE4F,
    0xFF00..=0xFFEF,
    0x20000..=0x3FFFF,
];
const HANGUL: Ranges = &[0x1100..=0x11FF, 0x3130..=0x318F, 0xA960..=0xA97F, 0xAC00..=0xD7FF, 0xFFA0..=0xFFDC];
const EMOJI: Ranges = &[
    0x203C..=0x203C,
    0x2049..=0x2049,
    0x20E3..=0x20E3,
    0x2122..=0x2122,
    0x2139..=0x2139,
    0x2194..=0x21AA,
    0x231A..=0x23FF,
    0x24C2..=0x24C2,
    0x25AA..=0x25FE,
    0x2600..=0x27BF,
    0x2934..=0x2935,
    0x2B05..=0x2B55,
    0xFE0F..=0xFE0F,
    0x1F000..=0x1FAFF,
];
const DEVANAGARI: Ranges = &[0x0900..=0x097F, 0xA8E0..=0xA8FF];
const ARABIC: Ranges = &[0x0600..=0x06FF, 0x0750..=0x077F, 0x08A0..=0x08FF, 0xFB50..=0xFDFF, 0xFE70..=0xFEFF];
const HEBREW: Ranges = &[0x0590..=0x05FF, 0xFB1D..=0xFB4F];
const THAI: Ranges = &[0x0E00..=0x0E7F];
const OTHER_SCRIPTS: Ranges = &[0x0370..=0x058F, 0x0980..=0x0DFF, 0x0E80..=0x0EFF, 0x10A0..=0x10FF, 0x1780..=0x17FF];
const SYMBOLS: Ranges = &[0x2000..=0x2BFF, 0x2E00..=0x2E7F, 0x10000..=0x1FAFF];

/// What's best to look in, for what, in order: for each, the files it might be.
pub(crate) fn fallbacks(language: &str) -> Vec<(Vec<&'static str>, Ranges)> {
    let lower = language.to_ascii_lowercase();
    let mut fonts: Vec<(Vec<&'static str>, Ranges)> = Vec::new();
    if cfg!(target_os = "android") {
        fonts.push((vec!["/system/fonts/NotoSansCJK-Regular.ttc", "/system/fonts/DroidSansFallback.ttf"], HAN_AND_KANA));
        fonts.push((vec!["/system/fonts/NotoSansCJK-Regular.ttc"], HANGUL));
        fonts.push((vec!["/system/fonts/NotoColorEmoji.ttf", "/system/fonts/NotoColorEmojiLegacy.ttf"], EMOJI));
        fonts.push((vec!["/system/fonts/NotoSansDevanagari-VF.ttf", "/system/fonts/NotoSansDevanagari-Regular.otf"], DEVANAGARI));
        fonts.push((vec!["/system/fonts/NotoNaskhArabic-Regular.ttf", "/system/fonts/NotoNaskhArabicUI-Regular.ttf"], ARABIC));
        fonts.push((vec!["/system/fonts/NotoSansHebrew-Regular.ttf"], HEBREW));
        fonts.push((vec!["/system/fonts/NotoSansThai-Regular.ttf"], THAI));
        fonts.push((vec!["/system/fonts/NotoSansSymbols-Regular-Subsetted.ttf", "/system/fonts/NotoSansSymbols-Regular.ttf"], SYMBOLS));
    } else if cfg!(target_vendor = "apple") {
        // PingFang has every kind of Han in one file; Japanese looks right in Hiragino.
        let han = if lower.starts_with("ja") {
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc"
        } else {
            "/System/Library/Fonts/PingFang.ttc"
        };
        fonts.push((vec![han, "/System/Library/Fonts/STHeiti Light.ttc"], HAN_AND_KANA));
        fonts.push((vec!["/System/Library/Fonts/AppleSDGothicNeo.ttc"], HANGUL));
        fonts.push((vec!["/System/Library/Fonts/Apple Color Emoji.ttc"], EMOJI));
        fonts.push((vec!["/System/Library/Fonts/Kohinoor.ttc", "/System/Library/Fonts/Supplemental/DevanagariMT.ttc"], DEVANAGARI));
        fonts.push((vec!["/System/Library/Fonts/GeezaPro.ttc", "/System/Library/Fonts/Supplemental/Arial.ttf"], ARABIC));
        fonts.push((vec!["/System/Library/Fonts/ArialHB.ttc", "/System/Library/Fonts/Supplemental/Arial.ttf"], HEBREW));
        fonts.push((vec!["/System/Library/Fonts/Thonburi.ttc"], THAI));
        fonts.push((vec!["/System/Library/Fonts/Apple Symbols.ttf", "/System/Library/Fonts/Supplemental/Arial Unicode.ttf"], SYMBOLS));
    } else {
        // Han and kana: the TrueType ones first, then Noto's (which are CFF), in each distribution's place.
        fonts.push((
            vec![
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/google-noto-sans-cjk-vf-fonts/NotoSansCJK-VF.ttc",
                "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
                "/usr/share/fonts/wqy-microhei/wqy-microhei.ttc",
                "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
                "/usr/share/fonts/google-droid-sans-fonts/DroidSansFallbackFull.ttf",
            ],
            HAN_AND_KANA,
        ));
        fonts.push((
            vec![
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/truetype/nanum/NanumGothic.ttf",
                "/usr/share/fonts/truetype/unfonts-core/UnDotum.ttf",
            ],
            HANGUL,
        ));
        fonts.push((
            vec![
                "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
                "/usr/share/fonts/noto/NotoColorEmoji.ttf",
                "/usr/share/fonts/google-noto-color-emoji-fonts/Noto-COLRv1.ttf",
                "/usr/share/fonts/google-noto-emoji/NotoColorEmoji.ttf",
                "/usr/share/fonts/joypixels/JoyPixels.ttf",
                "/usr/share/fonts/truetype/ancient-scripts/Symbola_hint.ttf",
            ],
            EMOJI,
        ));
        for (script, ranges) in [("Devanagari", DEVANAGARI), ("Hebrew", HEBREW), ("Thai", THAI)] {
            fonts.push((noto_candidates(script), ranges));
        }
        fonts.push((
            vec![
                "/usr/share/fonts/truetype/noto/NotoNaskhArabic-Regular.ttf",
                "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
                "/usr/share/fonts/noto/NotoSansArabic-Regular.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            ],
            ARABIC,
        ));
        let dejavu = vec![
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        ];
        fonts.push((dejavu.clone(), OTHER_SCRIPTS));
        fonts.push((dejavu, SYMBOLS));
    }
    fonts
}

/// Where Noto's font for a script is in the distributions that split it into one file a script.
fn noto_candidates(script: &str) -> Vec<&'static str> {
    ["/usr/share/fonts/truetype/noto/", "/usr/share/fonts/noto/", "/usr/share/fonts/google-noto/"]
        .iter()
        .map(|folder| &*Box::leak(format!("{folder}NotoSans{script}-Regular.ttf").into_boxed_str()))
        .collect()
}

/// Every installed font, by each of its names in lowercase: its full name, as `noto sans bold`, and its family's name
/// for the regular one of a family, as `noto sans`. The person's own fonts go on top of the system's.
pub(crate) fn all_fonts() -> HashMap<String, (PathBuf, u32)> {
    let mut fonts = HashMap::new();
    for folder in font_folders() {
        let mut files = Vec::new();
        walk(&folder, 0, &mut files);
        files.sort();
        for file in files {
            add_file(&file, &mut fonts);
        }
    }
    fonts
}

fn walk(folder: &Path, depth: usize, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() || (kind.is_symlink() && path.is_dir()) {
            if depth < 6 {
                walk(&path, depth + 1, into);
            }
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc"))
        {
            into.push(path);
        }
    }
}

/// Some bytes from a file, from `at`, as many as there are up to `length`.
fn read_part(file: &std::fs::File, at: usize, length: usize) -> Option<Vec<u8>> {
    use std::os::unix::fs::FileExt;
    let mut bytes = vec![0; length];
    let mut got = 0;
    while got < length {
        match file.read_at(&mut bytes[got..], (at + got) as u64) {
            Ok(0) => break,
            Ok(read) => got += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    bytes.truncate(got);
    Some(bytes)
}

/// Reads only the start of a font file and its `name` table, which is all finding it by name needs. Fonts for Han are
/// tens of megabytes each, and most of a system's fonts are never used.
fn add_file(path: &Path, into: &mut HashMap<String, (PathBuf, u32)>) {
    let Ok(file) = std::fs::File::open(path) else { return };
    let Some(head) = read_part(&file, 0, 12 + 4 * 256) else { return };
    let faces: Vec<usize> = if head.starts_with(b"ttcf") {
        let count = read_u32(&head, 8).unwrap_or(0).min(256) as usize;
        (0..count).filter_map(|index| read_u32(&head, 12 + index * 4).map(|at| at as usize)).collect()
    } else {
        vec![0]
    };
    for (face, at) in faces.into_iter().enumerate() {
        let Some(names) = names_at(&file, at) else { continue };
        let entry = (path.to_owned(), face as u32);
        if let Some(full) = &names.full {
            into.insert(full.to_lowercase(), entry.clone());
        }
        let regular = names.subfamily.as_deref().is_none_or(|sub| sub.eq_ignore_ascii_case("regular"));
        if regular {
            for family in [&names.family, &names.typographic].into_iter().flatten() {
                into.entry(family.to_lowercase()).or_insert_with(|| entry.clone());
            }
        }
    }
}

struct Names {
    family: Option<String>,
    subfamily: Option<String>,
    full: Option<String>,
    typographic: Option<String>,
}

/// The names of the font whose table directory is at `at` in the file.
fn names_at(file: &std::fs::File, at: usize) -> Option<Names> {
    let directory = read_part(file, at, 12 + 64 * 16)?;
    let tables = read_u16(&directory, 4)? as usize;
    let mut name = None;
    for index in 0..tables.min(64) {
        let record = 12 + index * 16;
        if directory.get(record..record + 4)? == b"name" {
            let start = read_u32(&directory, record + 8)? as usize;
            let length = (read_u32(&directory, record + 12)? as usize).min(1 << 20);
            name = read_part(file, start, length);
            break;
        }
    }
    let table = name?;
    Some(Names {
        family: name_of(&table, 1),
        subfamily: name_of(&table, 2),
        full: name_of(&table, 4),
        typographic: name_of(&table, 16),
    })
}

/// A name from a `name` table, preferring Windows' English one, as `Font::name` does.
fn name_of(table: &[u8], id: u16) -> Option<String> {
    let count = read_u16(table, 2)? as usize;
    let strings = read_u16(table, 4)? as usize;
    let mut best: Option<(u8, usize)> = None;
    for index in 0..count {
        let record = 6 + index * 12;
        if read_u16(table, record + 6)? != id {
            continue;
        }
        let rank = match (read_u16(table, record)?, read_u16(table, record + 2)?, read_u16(table, record + 4)?) {
            (3, 1 | 10, 0x409) => 4,
            (3, 1 | 10, _) => 3,
            (0, _, _) => 2,
            (1, 0, 0) => 1,
            _ => continue,
        };
        if best.is_none_or(|(best, _)| rank > best) {
            best = Some((rank, record));
        }
    }
    let (rank, record) = best?;
    let length = read_u16(table, record + 8)? as usize;
    let offset = read_u16(table, record + 10)? as usize;
    let raw = table.get(strings + offset..strings + offset + length)?;
    Some(if rank == 1 {
        raw.iter().map(|&byte| byte as char).collect()
    } else {
        let units: Vec<u16> = raw.chunks_exact(2).map(|pair| u16::from_be_bytes([pair[0], pair[1]])).collect();
        String::from_utf16_lossy(&units)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fallback_has_somewhere_to_look() {
        for (candidates, ranges) in fallbacks("en-US") {
            assert!(!candidates.is_empty());
            assert!(!ranges.is_empty());
        }
    }

    #[test]
    fn installed_fonts_are_found_by_their_names() {
        let fonts = all_fonts();
        // A system with DejaVu, as nearly every Linux has, finds it by its family and by its full name.
        if let Some(path) = first_that_exists(&["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"]) {
            assert_eq!(fonts.get("dejavu sans").map(|(found, _)| found.as_path()), Some(Path::new(path)));
            assert!(fonts.contains_key("dejavu sans bold"));
        }
    }
}
