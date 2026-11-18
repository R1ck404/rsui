//! Unicode normalization, for `String.prototype.normalize`: NFD and NFKD take every character apart, with the marks
//! after each in a set order, and NFC and NFKC then put back together what composes, as UAX #15 says. The tables are
//! made from the Unicode Character Database (`normalize_tables.rs`), and Hangul syllables are
//! worked out, as the standard does.
//!
//! Most text never gets here: what's all below the first character any form changes is given back as it is.

use super::normalize_tables::{CANONICAL, COMBINING, COMPATIBLE, COMPOSITIONS, DECOMPOSED};

/// Which of the four forms.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Form {
    Nfc,
    Nfd,
    Nfkc,
    Nfkd,
}

impl Form {
    pub(crate) fn named(name: &str) -> Option<Self> {
        Some(match name {
            "NFC" => Self::Nfc,
            "NFD" => Self::Nfd,
            "NFKC" => Self::Nfkc,
            "NFKD" => Self::Nfkd,
            _ => return None,
        })
    }

    fn compatible(self) -> bool {
        matches!(self, Self::Nfkc | Self::Nfkd)
    }

    fn composes(self) -> bool {
        matches!(self, Self::Nfc | Self::Nfkc)
    }

    /// Nothing below this is changed by the form: combining marks start at U+0300, letters with marks at U+00C0, and
    /// what has a compatibility decomposition at U+00A0.
    fn first_changed(self) -> u16 {
        match self {
            Self::Nfc => 0x300,
            Self::Nfd => 0xC0,
            Self::Nfkc | Self::Nfkd => 0xA0,
        }
    }
}

const OFFSET_BITS: u32 = 14;

const HANGUL_FIRST: u32 = 0xAC00;
const HANGUL_COUNT: u32 = 11172;
const LEADING_FIRST: u32 = 0x1100;
const LEADING_COUNT: u32 = 19;
const VOWEL_FIRST: u32 = 0x1161;
const VOWEL_COUNT: u32 = 21;
const TRAILING_FIRST: u32 = 0x11A7;
const TRAILING_COUNT: u32 = 28;
const PER_LEADING: u32 = VOWEL_COUNT * TRAILING_COUNT;

/// A code point's canonical combining class, which is 0 for anything that isn't a mark that's put in order.
pub(crate) fn combining_class(code: u32) -> u8 {
    if code < 0x300 {
        return 0;
    }
    let at = COMBINING.partition_point(|entry| entry >> 8 <= code);
    (COMBINING[at - 1] & 0xFF) as u8
}

/// The UTF-16 a code point decomposes to, in a table of `CANONICAL`'s kind, if it has an entry there.
fn decomposition(table: &[u32], code: u32) -> Option<&'static [u16]> {
    let at = table.partition_point(|entry| entry >> OFFSET_BITS < code);
    let entry = *table.get(at)?;
    // The last entry only says where the last decomposition ends.
    if entry >> OFFSET_BITS != code || at + 1 == table.len() {
        return None;
    }
    let mask = (1 << OFFSET_BITS) - 1;
    let start = (entry & mask) as usize;
    let end = (table[at + 1] & mask) as usize;
    Some(&DECOMPOSED[start..end])
}

/// What two code points compose to, if anything.
fn compose(first: u32, second: u32) -> Option<u32> {
    // A Hangul syllable is a leading consonant and a vowel, and then perhaps a trailing consonant.
    if (LEADING_FIRST..LEADING_FIRST + LEADING_COUNT).contains(&first)
        && (VOWEL_FIRST..VOWEL_FIRST + VOWEL_COUNT).contains(&second)
    {
        let syllable = (first - LEADING_FIRST) * PER_LEADING + (second - VOWEL_FIRST) * TRAILING_COUNT;
        return Some(HANGUL_FIRST + syllable);
    }
    if (HANGUL_FIRST..HANGUL_FIRST + HANGUL_COUNT).contains(&first)
        && (first - HANGUL_FIRST) % TRAILING_COUNT == 0
        && (TRAILING_FIRST + 1..TRAILING_FIRST + TRAILING_COUNT).contains(&second)
    {
        return Some(first + second - TRAILING_FIRST);
    }
    let key = (u64::from(first) << 42) | (u64::from(second) << 21);
    let at = COMPOSITIONS.partition_point(|entry| entry & !0x1F_FFFF < key);
    let entry = *COMPOSITIONS.get(at)?;
    (entry & !0x1F_FFFF == key).then_some((entry & 0x1F_FFFF) as u32)
}

/// Code points from UTF-16, with a surrogate that isn't half of a pair as itself.
fn code_points(units: &[u16]) -> impl Iterator<Item = u32> + '_ {
    char::decode_utf16(units.iter().copied()).map(|found| match found {
        Ok(character) => u32::from(character),
        Err(lone) => u32::from(lone.unpaired_surrogate()),
    })
}

/// Adds what `code` decomposes to, every step taken, with each code point's class.
fn decompose(code: u32, compatible: bool, out: &mut Vec<(u32, u8)>) {
    if (HANGUL_FIRST..HANGUL_FIRST + HANGUL_COUNT).contains(&code) {
        let index = code - HANGUL_FIRST;
        out.push((LEADING_FIRST + index / PER_LEADING, 0));
        out.push((VOWEL_FIRST + index % PER_LEADING / TRAILING_COUNT, 0));
        if index % TRAILING_COUNT != 0 {
            out.push((TRAILING_FIRST + index % TRAILING_COUNT, 0));
        }
        return;
    }
    let found = if compatible { decomposition(COMPATIBLE, code) } else { None };
    match found.or_else(|| decomposition(CANONICAL, code)) {
        Some(units) => out.extend(code_points(units).map(|part| (part, combining_class(part)))),
        None => out.push((code, combining_class(code))),
    }
}

/// Puts each run of marks in the order of their classes, keeping the order of those in the same class.
fn put_in_order(codes: &mut [(u32, u8)]) {
    let mut start = 0;
    while start < codes.len() {
        if codes[start].1 == 0 {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < codes.len() && codes[end].1 != 0 {
            end += 1;
        }
        if end - start > 1 {
            codes[start..end].sort_by_key(|(_, class)| *class);
        }
        start = end;
    }
}

/// Puts back together what composes, in place, as the standard's canonical composition does: a mark composes with the
/// last starter before it unless something between them has a class as high as its own, or is a starter itself.
fn compose_all(codes: &mut Vec<(u32, u8)>) {
    let mut starter: Option<usize> = None;
    // The class of what was last kept after the starter, or -1 for nothing, when they're next to each other.
    let mut last: i16 = -1;
    let mut kept = 0;
    for index in 0..codes.len() {
        let (code, class) = codes[index];
        if let Some(at) = starter {
            if last < i16::from(class) {
                if let Some(made) = compose(codes[at].0, code) {
                    codes[at].0 = made;
                    continue;
                }
            }
        }
        if class == 0 {
            starter = Some(kept);
            last = -1;
        } else {
            last = i16::from(class);
        }
        codes[kept] = (code, class);
        kept += 1;
    }
    codes.truncate(kept);
}

/// The text in the form asked for, or `None` if it's in it already.
pub(crate) fn normalize(units: &[u16], form: Form) -> Option<Vec<u16>> {
    let first_changed = form.first_changed();
    // What's before the first character that might change stays as it is, but for the one just before it, which is a
    // starter, as everything there is, and which what comes next may compose with.
    let start = units.iter().position(|unit| *unit >= first_changed)?.saturating_sub(1);
    let mut codes: Vec<(u32, u8)> = Vec::with_capacity(units.len() - start + 8);
    for code in code_points(&units[start..]) {
        decompose(code, form.compatible(), &mut codes);
    }
    put_in_order(&mut codes);
    if form.composes() {
        compose_all(&mut codes);
    }
    let mut out = Vec::with_capacity(start + codes.len() + 8);
    out.extend_from_slice(&units[..start]);
    for (code, _) in codes {
        match char::from_u32(code) {
            Some(character) => {
                let mut pair = [0; 2];
                out.extend_from_slice(character.encode_utf16(&mut pair));
            }
            // A lone surrogate stays as it was.
            None => out.push(code as u16),
        }
    }
    if out == units { None } else { Some(out) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn form(text: &str, form: Form) -> String {
        let given = units(text);
        String::from_utf16(&normalize(&given, form).unwrap_or(given)).unwrap()
    }

    #[test]
    fn letters_and_marks_come_apart_and_go_back_together() {
        assert_eq!(form("e\u{301}", Form::Nfc), "\u{e9}");
        assert_eq!(form("\u{e9}", Form::Nfd), "e\u{301}");
        // The marks go in order of their classes, below before above.
        assert_eq!(form("a\u{301}\u{327}", Form::Nfd), "a\u{327}\u{301}");
        assert_eq!(form("\u{1e9b}\u{323}", Form::Nfc), "\u{1e9b}\u{323}");
        assert_eq!(form("\u{1e9b}\u{323}", Form::Nfd), "\u{17f}\u{323}\u{307}");
        assert_eq!(form("\u{1e9b}\u{323}", Form::Nfkc), "\u{1e69}");
        assert_eq!(form("\u{1e9b}\u{323}", Form::Nfkd), "s\u{323}\u{307}");
        // Hangul is worked out.
        assert_eq!(form("\u{d55c}", Form::Nfd), "\u{1112}\u{1161}\u{11ab}");
        assert_eq!(form("\u{1112}\u{1161}\u{11ab}", Form::Nfc), "\u{d55c}");
        // What's excluded from composing stays apart, and what's compatible only changes with K.
        assert_eq!(form("\u{915}\u{93c}", Form::Nfc), "\u{915}\u{93c}");
        assert_eq!(form("\u{fb01}", Form::Nfc), "\u{fb01}");
        assert_eq!(form("\u{fb01}", Form::Nfkc), "fi");
        assert_eq!(form("\u{2126}", Form::Nfc), "\u{3a9}");
    }

    #[test]
    fn text_already_in_the_form_is_left_alone() {
        assert_eq!(normalize(&units("plain ascii"), Form::Nfkd), None);
        assert_eq!(normalize(&units("caf\u{e9}"), Form::Nfc), None);
        assert_eq!(normalize(&[0xD800, 0x301], Form::Nfc), None);
    }
}
