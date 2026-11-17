//! Finding a font by its name, from what Windows says is installed. This is only looked at the first time something asks
//! for a font by name, so a program that doesn't has one registry key fewer to read.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::OnceLock;

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

    #[test]
    fn windows_has_the_font_it_comes_with() {
        assert!(find("Segoe UI").is_some());
        assert!(find("SEGOE ui bold").is_some());
        assert_eq!(find("no such font at all"), None);
    }
}
