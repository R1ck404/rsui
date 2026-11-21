//! Which of the fonts that come with Windows have the characters the main font doesn't, so that text in
//! any language shows up without an app having to choose. Each one is only opened when text first needs
//! it, and is mapped from its file, so this costs next to nothing for the ones that are never used. Other systems'
//! lists are in `unix_fonts.rs`.
#![cfg_attr(not(windows), allow(dead_code))]

use std::ops::RangeInclusive;

use crate::engine::TextEngine;

type Ranges = &'static [RangeInclusive<u32>];

const FONTS: &str = "C:/Windows/Fonts/";

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
const HANGUL: Ranges = &[
    0x1100..=0x11FF,
    0x3130..=0x318F,
    0xA960..=0xA97F,
    0xAC00..=0xD7FF,
    0xFFA0..=0xFFDC,
];
/// What only ever shows as an emoji, and the signs that ask for one.
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
const INDIC: Ranges = &[
    0x0900..=0x0DFF,
    0x1C50..=0x1C7F,
    0x1CD0..=0x1CFF,
    0xA830..=0xA83F,
    0xA8E0..=0xA8FF,
];
const OTHER_INDIC: Ranges = &[0xAAE0..=0xAAFF, 0xABC0..=0xABFF, 0x11100..=0x1114F];
const JAVANESE: Ranges = &[0xA980..=0xA9DF];
const NEW_TAI_LUE: Ranges = &[0x1980..=0x19DF];
const HISTORIC: Ranges = &[0x10000..=0x16FFF];
const SOUTHEAST_ASIAN: Ranges = &[0x0E00..=0x0EFF, 0x1780..=0x17FF, 0x19E0..=0x1A1F];
const MYANMAR: Ranges = &[0x1000..=0x109F, 0xA9E0..=0xA9FF, 0xAA60..=0xAA7F];
const TIBETAN: Ranges = &[0x0F00..=0x0FFF];
const MONGOLIAN: Ranges = &[0x1800..=0x18AF];
const AFRICAN: Ranges = &[
    0x07C0..=0x07FF,
    0x1200..=0x139F,
    0x2D30..=0x2DDF,
    0xA500..=0xA63F,
    0xAB00..=0xAB2F,
    0x10480..=0x104AF,
];
const NATIVE_AMERICAN: Ranges = &[0x13A0..=0x167F, 0x18B0..=0x18FF, 0xAB70..=0xABBF];
const SYMBOLS: Ranges = &[0x2000..=0x2BFF, 0x2E00..=0x2E7F, 0x10000..=0x1FAFF];

/// Han characters look different from one place to another, so the user's own language goes first.
fn han_font_for(language: &str) -> &'static str {
    let lower = language.to_ascii_lowercase();
    if lower.starts_with("ja") {
        "YuGothR.ttc"
    } else if lower.starts_with("ko") {
        "malgun.ttf"
    } else if ["zh-tw", "zh-hk", "zh-mo", "zh-hant"]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        "msjh.ttc"
    } else {
        "msyh.ttc"
    }
}

/// What's best to look in, for what, in order.
fn fonts_for(language: &str) -> Vec<(&'static str, Ranges)> {
    let mut fonts = vec![(han_font_for(language), HAN_AND_KANA)];
    // Kana only comes up in Japanese, and not every font for Han has it, or looks right with it.
    fonts.push(("YuGothR.ttc", HAN_AND_KANA));
    fonts.push(("malgun.ttf", HANGUL));
    fonts.push(("seguiemj.ttf", EMOJI));
    fonts.push(("Nirmala.ttc", INDIC));
    fonts.push(("Nirmala.ttc", OTHER_INDIC));
    fonts.push(("javatext.ttf", JAVANESE));
    fonts.push(("ntailu.ttf", NEW_TAI_LUE));
    fonts.push(("seguihis.ttf", HISTORIC));
    fonts.push(("LeelawUI.ttf", SOUTHEAST_ASIAN));
    fonts.push(("KhmerUI.ttf", SOUTHEAST_ASIAN));
    fonts.push(("LaoUI.ttf", SOUTHEAST_ASIAN));
    fonts.push(("mmrtext.ttf", MYANMAR));
    fonts.push(("himalaya.ttf", TIBETAN));
    fonts.push(("monbaiti.ttf", MONGOLIAN));
    fonts.push(("ebrima.ttf", AFRICAN));
    fonts.push(("gadugi.ttf", NATIVE_AMERICAN));
    fonts.push(("seguisym.ttf", SYMBOLS));
    fonts
}

/// The heavier version of a font, as its file and which font in that file, if Windows has one.
fn bold_of(file: &str) -> Option<(&'static str, u32)> {
    Some(match file {
        "msyh.ttc" => ("msyhbd.ttc", 0),
        "msjh.ttc" => ("msjhbd.ttc", 0),
        "YuGothR.ttc" => ("YuGothB.ttc", 0),
        "malgun.ttf" => ("malgunbd.ttf", 0),
        "Nirmala.ttc" => ("Nirmala.ttc", 1),
        "LeelawUI.ttf" => ("LeelaUIb.ttf", 0),
        "KhmerUI.ttf" => ("KhmerUIB.ttf", 0),
        "LaoUI.ttf" => ("LaoUIB.ttf", 0),
        "mmrtext.ttf" => ("mmrtextb.ttf", 0),
        "ebrima.ttf" => ("ebrimabd.ttf", 0),
        "gadugi.ttf" => ("gadugib.ttf", 0),
        _ => return None,
    })
}

#[cfg(windows)]
impl TextEngine {
    /// Adds the fonts that come with Windows for every other language and for emoji, each opened only
    /// once there's text that needs it, and bold ones for bold text.
    #[must_use]
    pub fn with_system_fallbacks(mut self) -> Self {
        for (file, characters) in fonts_for(&user_language()) {
            self = self.with_lazy_fallback(format!("{FONTS}{file}"), characters);
            if let Some((bold, face)) = bold_of(file) {
                self = self.with_lazy_bold_fallback(format!("{FONTS}{bold}"), face, characters);
            }
        }
        self
    }
}

#[cfg(not(windows))]
impl TextEngine {
    /// Adds the fonts the system comes with for every other language and for emoji, each opened only once there's
    /// text that needs it. Each is a list of the files it might be, which are tried in order then.
    #[must_use]
    pub fn with_system_fallbacks(mut self) -> Self {
        for (candidates, characters) in crate::unix_fonts::fallbacks(&user_language()) {
            for path in candidates {
                self = self.with_lazy_fallback(path, characters);
            }
        }
        self
    }
}

/// The system's UI font, regular and bold, as files: Segoe UI on Windows, Roboto on Android, the San Francisco font on
/// macOS, and Noto Sans or DejaVu Sans on Linux, whichever there is.
pub fn system_ui_font_files() -> (Option<&'static str>, Option<&'static str>) {
    #[cfg(windows)]
    {
        (Some("C:/Windows/Fonts/segoeui.ttf"), Some("C:/Windows/Fonts/segoeuib.ttf"))
    }
    #[cfg(not(windows))]
    {
        let (regular, bold) = crate::unix_fonts::ui_font_candidates();
        (crate::unix_fonts::first_that_exists(regular), crate::unix_fonts::first_that_exists(bold))
    }
}

/// The language this computer's user has chosen, like `ja-JP`.
#[cfg(windows)]
fn user_language() -> String {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(name: *mut u16, size: i32) -> i32;
    }
    let mut name = [0u16; 85];
    // SAFETY: it writes at most this many characters into this space.
    let length = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
    let length = usize::try_from(length).unwrap_or(0).saturating_sub(1);
    String::from_utf16_lossy(&name[..length.min(name.len())])
}

#[cfg(not(windows))]
fn user_language() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        // `ja_JP.UTF-8` is `ja-JP`.
        .map(|value| value.split(['.', '@']).next().unwrap_or_default().replace('_', "-"))
        .unwrap_or_default()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn han_comes_from_the_font_for_the_users_language() {
        assert_eq!(han_font_for("ja-JP"), "YuGothR.ttc");
        assert_eq!(han_font_for("ko-KR"), "malgun.ttf");
        assert_eq!(han_font_for("zh-TW"), "msjh.ttc");
        assert_eq!(han_font_for("zh-Hant-HK"), "msjh.ttc");
        assert_eq!(han_font_for("zh-CN"), "msyh.ttc");
        assert_eq!(han_font_for("en-US"), "msyh.ttc");
        assert_eq!(han_font_for(""), "msyh.ttc");
    }

    #[test]
    fn every_font_with_a_heavier_version_has_one_named() {
        for font in [
            "msyh.ttc",
            "msjh.ttc",
            "YuGothR.ttc",
            "malgun.ttf",
            "Nirmala.ttc",
        ] {
            assert!(bold_of(font).is_some(), "{font}");
        }
        assert_eq!(bold_of("seguiemj.ttf"), None);
        assert_eq!(bold_of("Nirmala.ttc"), Some(("Nirmala.ttc", 1)));
    }

    #[test]
    fn the_users_language_is_something_windows_would_say() {
        let language = user_language();
        assert!(language.is_ascii(), "{language:?}");
    }
}
