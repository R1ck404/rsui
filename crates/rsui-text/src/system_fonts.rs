//! Finding a font by its name, from what Windows says is installed. This is only looked at the first time something asks
//! for a font by name, so a program that doesn't has one registry key fewer to read.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::font::{Font, read_u16, read_u32};

const LOCAL_MACHINE: *mut c_void = 0x8000_0002u32 as usize as *mut c_void;
const CURRENT_USER: *mut c_void = 0x8000_0001u32 as usize as *mut c_void;
const READ: u32 = 0x2_0019;
const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts";

// Kernelbase has these too, and it's always loaded, where advapi32 would load three more libraries just to start up. The
// name is the one Windows promises to keep pointing at whichever library has them.
#[link(name = "api-ms-win-core-registry-l1-1-0", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegOpenKeyExW(
        key: *mut c_void,
        name: *const u16,
        options: u32,
        access: u32,
        result: *mut *mut c_void,
    ) -> i32;
    fn RegEnumValueW(
        key: *mut c_void,
        index: u32,
        name: *mut u16,
        name_length: *mut u32,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        data_length: *mut u32,
    ) -> i32;
    fn RegCloseKey(key: *mut c_void) -> i32;
}

fn read_key(root: *mut c_void, into: &mut HashMap<String, (PathBuf, u32)>) {
    let name: Vec<u16> = KEY.encode_utf16().chain(Some(0)).collect();
    let mut key = std::ptr::null_mut();
    // SAFETY: the name ends in a zero, each call is told how large what it writes to is, and the key is closed once.
    unsafe {
        if RegOpenKeyExW(root, name.as_ptr(), 0, READ, &raw mut key) != 0 {
            return;
        }
        let mut value_name = vec![0u16; 512];
        let mut data = vec![0u16; 1024];
        for index in 0.. {
            let mut name_length = value_name.len() as u32;
            let mut data_length = (data.len() * 2) as u32;
            let mut kind = 0;
            let result = RegEnumValueW(
                key,
                index,
                value_name.as_mut_ptr(),
                &raw mut name_length,
                std::ptr::null_mut(),
                &raw mut kind,
                data.as_mut_ptr().cast(),
                &raw mut data_length,
            );
            if result != 0 {
                break;
            }
            let shown = String::from_utf16_lossy(&value_name[..name_length as usize]);
            let file = String::from_utf16_lossy(&data[..data_length as usize / 2]);
            let file = file.trim_end_matches('\0');
            add_entry(&shown, file, into);
        }
        RegCloseKey(key);
    }
}

/// `Arial Bold (TrueType)` has one font in it, and `Cambria & Cambria Math (TrueType)` has one for each name, in that order.
fn add_entry(shown: &str, file: &str, into: &mut HashMap<String, (PathBuf, u32)>) {
    let names = shown.rsplit_once(" (").map_or(shown, |(names, _)| names);
    let path = if file.contains('\\') || file.contains('/') {
        PathBuf::from(file)
    } else {
        let folder =
            std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        folder.join("Fonts").join(file)
    };
    for (face, name) in names.split(" & ").enumerate() {
        into.insert(name.trim().to_lowercase(), (path.clone(), face as u32));
    }
}

fn all_fonts() -> &'static HashMap<String, (PathBuf, u32)> {
    static FONTS: OnceLock<HashMap<String, (PathBuf, u32)>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut fonts = HashMap::new();
        read_key(LOCAL_MACHINE, &mut fonts);
        // What a person has installed for themselves goes on top.
        read_key(CURRENT_USER, &mut fonts);
        fonts
    })
}

/// The file and which font in it, for a name like `Segoe UI Bold`. A name that's a path is that file.
pub(crate) fn find(name: &str) -> Option<(PathBuf, u32)> {
    if name.contains('/') || name.contains('\\') {
        return Some((PathBuf::from(name), 0));
    }
    all_fonts().get(&name.to_lowercase()).cloned()
}

/// One font of a family, which is one of the places a variable font names along its axes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Instance {
    pub(crate) path: PathBuf,
    pub(crate) face: u32,
    /// Its CSS weight, such as 600 for semibold.
    pub(crate) weight: f32,
    pub(crate) italic: bool,
    /// Where it is along the font's axes, by tag.
    pub(crate) settings: Vec<([u8; 4], f32)>,
}

/// What a family's name is to the variable fonts that are installed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Variable {
    /// It's the name of these places along one or more variable fonts' axes, as DirectWrite, and so a browser, names them.
    Family(Arc<[Instance]>),
    /// It's what Windows calls a variable font whose places all belong to families with longer names, as `Segoe UI
    /// Variable` is, which a browser doesn't know as a family.
    NotAFamily,
    /// It has nothing to do with variable fonts.
    No,
}

/// The weights and styles CSS names that aren't part of a family's name.
const STYLE_WORDS: &[&str] = &[
    "thin",
    "hairline",
    "extralight",
    "ultralight",
    "light",
    "semilight",
    "demilight",
    "regular",
    "normal",
    "book",
    "medium",
    "semibold",
    "demibold",
    "bold",
    "extrabold",
    "ultrabold",
    "black",
    "heavy",
    "italic",
    "oblique",
];

/// The families each place a variable font names along its axes belongs to, by the name of each, lowercased, as
/// DirectWrite names them: the font's typographic family, then what its `STAT` table calls where the place is along
/// each axis other than weight and slant, in that table's order, leaving out what it says can be. A font without
/// `STAT` goes by each place's own name, without its weight and style.
pub(crate) fn instance_families(font: &Font, path: &Path) -> Vec<(String, Instance)> {
    let Some(family) = font.name(16).or_else(|| font.name(1)) else {
        return Vec::new();
    };
    let axes = font.variation_axes();
    let stat = font.stat_table().and_then(read_stat);
    font.named_instances()
        .into_iter()
        .map(|instance| {
            let at = |tag: &[u8; 4]| {
                axes.iter()
                    .position(|axis| axis.tag == *tag)
                    .and_then(|axis| instance.coordinates.get(axis).copied())
            };
            let own_name = font.name(instance.subfamily_name).unwrap_or_default();
            let mut name = family.clone();
            match &stat {
                Some((design, values)) => {
                    for (index, tag) in design {
                        if matches!(tag, b"wght" | b"ital" | b"slnt") {
                            continue;
                        }
                        let Some(coordinate) = at(tag) else {
                            continue;
                        };
                        let near = |value: f32| (value - coordinate).abs() < 0.001;
                        let on_axis = || values.iter().filter(|value| value.axis == *index);
                        let found = on_axis().find(|value| near(value.value)).or_else(|| {
                            on_axis().find(|value| {
                                value.range.is_some_and(|(low, high)| {
                                    low <= coordinate && coordinate < high
                                })
                            })
                        });
                        if let Some(found) = found
                            && !found.elidable
                            && let Some(part) = font.name(found.name)
                        {
                            name.push(' ');
                            name.push_str(&part);
                        }
                    }
                }
                None => {
                    for word in own_name.split_whitespace() {
                        if !STYLE_WORDS.contains(&word.to_lowercase().as_str()) {
                            name.push(' ');
                            name.push_str(word);
                        }
                    }
                }
            }
            let lower = own_name.to_lowercase();
            let italic = at(b"ital").is_some_and(|ital| ital >= 0.5)
                || at(b"slnt").is_some_and(|slant| slant != 0.0)
                || lower.contains("italic")
                || lower.contains("oblique");
            let settings = axes
                .iter()
                .zip(&instance.coordinates)
                .map(|(axis, value)| (axis.tag, *value))
                .collect();
            (
                normal_name(&name),
                Instance {
                    path: path.to_owned(),
                    face: font.face_index(),
                    weight: at(b"wght").unwrap_or(400.0),
                    italic,
                    settings,
                },
            )
        })
        .collect()
}

/// A value a `STAT` table names along one of its axes.
struct AxisValue {
    axis: u16,
    value: f32,
    /// What it covers along its axis, for one that says.
    range: Option<(f32, f32)>,
    elidable: bool,
    name: u16,
}

/// The design axes a `STAT` table has, by their place in it and in their order, and the values it names along them.
fn read_stat(stat: &[u8]) -> Option<(Vec<(u16, [u8; 4])>, Vec<AxisValue>)> {
    let size = read_u16(stat, 4)? as usize;
    let count = read_u16(stat, 6)? as usize;
    let at = read_u32(stat, 8)? as usize;
    let value_count = read_u16(stat, 12)? as usize;
    let values_at = read_u32(stat, 14)? as usize;
    if size < 8 {
        return None;
    }
    let mut design = Vec::with_capacity(count.min(64));
    for index in 0..count.min(64) {
        let record = at + index * size;
        let tag = stat.get(record..record + 4)?;
        let ordering = read_u16(stat, record + 6)?;
        design.push((ordering, index as u16, [tag[0], tag[1], tag[2], tag[3]]));
    }
    design.sort_by_key(|(ordering, index, _)| (*ordering, *index));
    let fixed = |at: usize| read_u32(stat, at).map(|value| value as i32 as f32 / 65536.0);
    let mut values = Vec::new();
    for index in 0..value_count.min(1024) {
        let Some(offset) = read_u16(stat, values_at + index * 2) else {
            break;
        };
        let table = values_at + offset as usize;
        let (Some(format), Some(axis), Some(flags), Some(name)) = (
            read_u16(stat, table),
            read_u16(stat, table + 2),
            read_u16(stat, table + 4),
            read_u16(stat, table + 6),
        ) else {
            continue;
        };
        let elidable = flags & 2 != 0;
        let (value, range) = match format {
            1 | 3 => (fixed(table + 8), None),
            2 => (fixed(table + 8), fixed(table + 12).zip(fixed(table + 16))),
            _ => continue,
        };
        if let Some(value) = value {
            values.push(AxisValue {
                axis,
                value,
                range,
                elidable,
                name,
            });
        }
    }
    Some((
        design
            .into_iter()
            .map(|(_, index, tag)| (index, tag))
            .collect(),
        values,
    ))
}

/// A name as it's compared: lowercase, with single spaces.
fn normal_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

type Families = Arc<Vec<(String, Instance)>>;

/// The families of a font file's places along its axes, `None` for a font that doesn't vary, kept once worked out.
fn families_of(path: &Path, face: u32) -> Option<Families> {
    static KNOWN: OnceLock<Mutex<HashMap<(PathBuf, u32), Option<Families>>>> = OnceLock::new();
    let known = KNOWN.get_or_init(Default::default);
    let key = (path.to_owned(), face);
    if let Some(found) = known.lock().ok()?.get(&key) {
        return found.clone();
    }
    let found = Font::open_face(path, face)
        .ok()
        .filter(|font| !font.variation_axes().is_empty())
        .map(|font| Arc::new(instance_families(&font, path)));
    known.lock().ok()?.insert(key, found.clone());
    found
}

/// What `name` is to the variable fonts that are installed, kept once worked out. Only fonts whose names Windows lists
/// start the same way are looked in, so that most names open no fonts at all.
pub(crate) fn find_variable(name: &str) -> Variable {
    static KNOWN: OnceLock<Mutex<HashMap<String, Variable>>> = OnceLock::new();
    if name.contains('/') || name.contains('\\') {
        return Variable::No;
    }
    let wanted = normal_name(name);
    let known = KNOWN.get_or_init(Default::default);
    if let Some(found) = known
        .lock()
        .ok()
        .and_then(|known| known.get(&wanted).cloned())
    {
        return found;
    }
    let found = look_for_variable(&wanted, all_fonts());
    if let Ok(mut known) = known.lock() {
        known.insert(wanted, found.clone());
    }
    found
}

fn look_for_variable(wanted: &str, fonts: &HashMap<String, (PathBuf, u32)>) -> Variable {
    let exact = fonts.get(wanted);
    // A font Windows has by this name that doesn't vary is the usual kind, which is the most common case by far.
    if let Some((path, face)) = exact
        && families_of(path, *face).is_none()
    {
        return Variable::No;
    }
    let words: Vec<&str> = wanted.split(' ').collect();
    let starts_like = |listed: &str| {
        let mut listed: Vec<&str> = listed.split_whitespace().collect();
        // Leaving out the style the name ends with, as in `Sitka Text Italic`.
        while listed.len() > 1 && listed.last().is_some_and(|word| STYLE_WORDS.contains(word)) {
            listed.pop();
        }
        let Some((_, all_but_last)) = listed.split_last() else {
            return false;
        };
        // Its whole name begins this one, as `Segoe UI Variable` does `Segoe UI Variable Text`, or all but its last
        // word does, as `Sitka Text` does `Sitka Banner`.
        words.starts_with(&listed) || (!all_but_last.is_empty() && words.starts_with(all_but_last))
    };
    let mut files: Vec<&(PathBuf, u32)> = fonts
        .iter()
        .filter(|(listed, _)| starts_like(listed))
        .map(|(_, file)| file)
        .collect();
    files.sort();
    files.dedup();
    let instances: Vec<Instance> = files
        .into_iter()
        .filter_map(|(path, face)| families_of(path, *face))
        .flat_map(|families| {
            families
                .iter()
                .filter(|(family, _)| family == wanted)
                .map(|(_, instance)| instance.clone())
                .collect::<Vec<_>>()
        })
        .collect();
    if !instances.is_empty() {
        Variable::Family(instances.into())
    } else if exact.is_some() {
        Variable::NotAFamily
    } else {
        Variable::No
    }
}

/// Of a family's places along its axes, the one for this CSS weight and style, as DirectWrite and so a browser chooses:
/// the nearest weight among those in the style if there are any, the heavier of two as near for a weight from 400 up,
/// and the lighter below that.
pub(crate) fn choose_instance(
    instances: &[Instance],
    weight: f32,
    italic: bool,
) -> Option<&Instance> {
    let styled = instances.iter().any(|instance| instance.italic == italic);
    instances
        .iter()
        .filter(|instance| !styled || instance.italic == italic)
        .min_by(|a, b| {
            let distance = |instance: &Instance| (instance.weight - weight).abs();
            distance(a).total_cmp(&distance(b)).then_with(|| {
                if weight >= 400.0 {
                    b.weight.total_cmp(&a.weight)
                } else {
                    a.weight.total_cmp(&b.weight)
                }
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_entries_turn_into_names() {
        let mut fonts = HashMap::new();
        add_entry("Arial Bold (TrueType)", "arialbd.ttf", &mut fonts);
        add_entry(
            "Cambria & Cambria Math (TrueType)",
            "cambria.ttc",
            &mut fonts,
        );
        add_entry("Mine (OpenType)", r"C:\Users\me\mine.otf", &mut fonts);
        assert!(fonts["arial bold"].0.ends_with("arialbd.ttf"));
        assert_eq!(fonts["cambria"].1, 0);
        assert_eq!(fonts["cambria math"].1, 1);
        assert_eq!(fonts["mine"].0, PathBuf::from(r"C:\Users\me\mine.otf"));
    }

    #[test]
    fn a_name_that_is_a_path_is_that_file() {
        assert_eq!(find("C:/x/y.ttf"), Some((PathBuf::from("C:/x/y.ttf"), 0)));
    }

    fn instance(weight: f32, italic: bool) -> Instance {
        Instance {
            path: PathBuf::new(),
            face: 0,
            weight,
            italic,
            settings: Vec::new(),
        }
    }

    #[test]
    fn a_weight_between_two_goes_as_windows_chooses() {
        let family = [300.0, 350.0, 400.0, 600.0, 700.0].map(|weight| instance(weight, false));
        let chosen =
            |weight: f32| choose_instance(&family, weight, false).map(|instance| instance.weight);
        assert_eq!(chosen(400.0), Some(400.0));
        // 500 is as near 400 as 600, and the heavier is chosen, as Edge does.
        assert_eq!(chosen(500.0), Some(600.0));
        assert_eq!(chosen(900.0), Some(700.0));
        assert_eq!(chosen(100.0), Some(300.0));
        let gapped = [200.0, 400.0].map(|weight| instance(weight, false));
        assert_eq!(
            choose_instance(&gapped, 300.0, false).map(|instance| instance.weight),
            Some(200.0)
        );
        // Italic is chosen where there is one, and upright otherwise.
        let styled = [
            instance(400.0, false),
            instance(400.0, true),
            instance(700.0, false),
        ];
        assert!(choose_instance(&styled, 400.0, true).is_some_and(|instance| instance.italic));
        assert!(choose_instance(&styled, 700.0, true).is_some_and(|instance| instance.italic));
        assert!(choose_instance(&family, 400.0, true).is_some_and(|instance| !instance.italic));
        assert_eq!(choose_instance(&[], 400.0, false), None);
    }

    #[test]
    fn variable_fonts_name_families_as_directwrite_does() {
        let path = Path::new("C:/Windows/Fonts/SegUIVar.ttf");
        let Ok(font) = Font::open(path) else {
            return;
        };
        let families = instance_families(&font, path);
        assert_eq!(families.len(), 15);
        let weights = |name: &str| -> Vec<f32> {
            families
                .iter()
                .filter(|(family, _)| family == name)
                .map(|(_, instance)| instance.weight)
                .collect()
        };
        for name in [
            "segoe ui variable small",
            "segoe ui variable text",
            "segoe ui variable display",
        ] {
            assert_eq!(weights(name), [300.0, 350.0, 400.0, 600.0, 700.0], "{name}");
        }
        // The regular place has its own short name, but `STAT` says it's text.
        let text = families
            .iter()
            .find(|(family, instance)| {
                family == "segoe ui variable text" && instance.weight == 400.0
            })
            .expect("it's there");
        assert_eq!(text.1.settings, [(*b"wght", 400.0), (*b"opsz", 10.5)]);
    }

    #[test]
    fn variable_families_are_found_by_their_names() {
        if !Path::new("C:/Windows/Fonts/SegUIVar.ttf").exists() {
            return;
        }
        assert_eq!(find_variable("Segoe UI"), Variable::No);
        assert_eq!(find_variable("no such font at all"), Variable::No);
        assert_eq!(find_variable("Segoe UI Variable"), Variable::NotAFamily);
        let Variable::Family(display) = find_variable("segoe  UI variable DISPLAY") else {
            panic!("it's a family");
        };
        assert_eq!(display.len(), 5);
        assert!(
            display
                .iter()
                .all(|instance| instance.settings[1] == (*b"opsz", 36.0))
        );
        // Fonts whose names Windows lists differently from their families' are found too.
        if Path::new("C:/Windows/Fonts/SitkaVF.ttf").exists() {
            let Variable::Family(banner) = find_variable("Sitka Banner") else {
                panic!("it's a family");
            };
            assert!(banner.iter().any(|instance| instance.italic));
            assert!(banner.iter().any(|instance| !instance.italic));
        }
        if Path::new("C:/Windows/Fonts/bahnschrift.ttf").exists() {
            assert!(matches!(find_variable("Bahnschrift"), Variable::Family(_)));
            assert!(matches!(
                find_variable("Bahnschrift SemiCondensed"),
                Variable::Family(_)
            ));
        }
    }

    #[test]
    fn windows_has_the_font_it_comes_with() {
        assert!(find("Segoe UI").is_some());
        assert!(find("SEGOE ui bold").is_some());
        assert_eq!(find("no such font at all"), None);
    }
}
