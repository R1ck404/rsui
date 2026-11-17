//! What `toLocaleString` and `Intl` are told about a language, made into what Windows can be asked about it, and the digits
//! of other ways of writing numbers (`numberingSystem`, or `-u-nu-` in the locale), as a browser's ICU has them.

/// A locale as it's given, taken apart: the language Windows is asked about (`None` for the user's own, which is what a
/// browser falls back to for one it doesn't know, as `"default"` is), and the numbering system its `-u-nu-` asks for.
pub(crate) struct Locale {
    pub name: Option<String>,
    pub numbering: Option<String>,
}

pub(crate) fn locale_of(given: Option<&str>) -> Locale {
    let Some(given) = given else {
        return Locale {
            name: None,
            numbering: None,
        };
    };
    let lower = given.to_ascii_lowercase();
    // Extensions come after `-u-` (and `-x-` for private ones), each a key and its value.
    let (base, extension) = match lower.find("-u-") {
        Some(at) => (&given[..at], Some(&lower[at + 3..])),
        None => (given, None),
    };
    let base = match base.to_ascii_lowercase().find("-x-") {
        Some(at) => &base[..at],
        None => base,
    };
    let mut numbering = None;
    if let Some(extension) = extension {
        let keys: Vec<&str> = extension.split('-').collect();
        for (at, key) in keys.iter().enumerate() {
            if *key == "nu" {
                numbering = keys.get(at + 1).map(|value| (*value).to_owned());
            }
        }
    }
    let name = is_known(base).then(|| base.to_owned());
    Locale {
        name,
        numbering: numbering.filter(|system| zero_of(system).is_some()),
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn is_known(name: &str) -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn IsValidLocaleName(name: *const u16) -> i32;
    }
    if name.eq_ignore_ascii_case("en-US") || name.eq_ignore_ascii_case("en") {
        return true;
    }
    if name.is_empty() || name.eq_ignore_ascii_case("default") {
        return false;
    }
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    // SAFETY: the name ends in a terminator.
    unsafe { IsValidLocaleName(wide.as_ptr()) != 0 }
}

#[cfg(not(windows))]
fn is_known(name: &str) -> bool {
    !name.is_empty() && !name.eq_ignore_ascii_case("default")
}

/// The digit zero of a numbering system whose digits follow on from it, or `None` for one this doesn't know (which a browser
/// leaves as Latin digits, as `latn` is). `hanidec`'s aren't in a row, and are written by `digits_in`.
pub(crate) fn zero_of(system: &str) -> Option<char> {
    Some(match system {
        "latn" => '0',
        "arab" => '\u{0660}',
        "arabext" => '\u{06F0}',
        "bali" => '\u{1B50}',
        "beng" => '\u{09E6}',
        "deva" => '\u{0966}',
        "fullwide" => '\u{FF10}',
        "gujr" => '\u{0AE6}',
        "guru" => '\u{0A66}',
        "hanidec" => '\u{3007}',
        "khmr" => '\u{17E0}',
        "knda" => '\u{0CE6}',
        "laoo" => '\u{0ED0}',
        "limb" => '\u{1946}',
        "mlym" => '\u{0D66}',
        "mong" => '\u{1810}',
        "mymr" => '\u{1040}',
        "orya" => '\u{0B66}',
        "tamldec" => '\u{0BE6}',
        "telu" => '\u{0C66}',
        "thai" => '\u{0E50}',
        "tibt" => '\u{0F20}',
        _ => return None,
    })
}

const HAN_DIGITS: [char; 10] = [
    '\u{3007}', '一', '二', '三', '四', '五', '六', '七', '八', '九',
];

/// Text with its Latin digits written in this numbering system. In a number (`number`), Arabic's two also have their own
/// decimal point, group separator, percent sign and minus, as ICU writes them; a date keeps what's between its digits.
pub(crate) fn digits_in(text: &str, system: &str, number: bool) -> String {
    let Some(zero) = zero_of(system) else {
        return text.to_owned();
    };
    if zero == '0' {
        return text.to_owned();
    }
    let arabic = number && matches!(system, "arab" | "arabext");
    let mut out = String::with_capacity(text.len() * 2);
    for character in text.chars() {
        match character {
            '0'..='9' => {
                let digit = character as u32 - '0' as u32;
                out.push(if system == "hanidec" {
                    HAN_DIGITS[digit as usize]
                } else {
                    char::from_u32(zero as u32 + digit).unwrap_or(character)
                });
            }
            '.' if arabic => out.push('\u{066B}'),
            ',' if arabic => out.push('\u{066C}'),
            '%' if arabic => out.push('\u{066A}'),
            '-' if arabic && system == "arab" => out.push_str("\u{061C}-"),
            '-' if arabic && system == "arabext" => out.push_str("\u{200E}-\u{200E}"),
            other => out.push(other),
        }
    }
    out
}
