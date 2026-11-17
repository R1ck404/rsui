//! Text that can't be laid out one character at a time: right-to-left text, joined letters,
//! ligatures and combining marks.
//!
//! Plain text never comes here, so it pays only for a quick look at each character. See
//! [`is_complex_character`].

use crate::bidi::{self, Class};
use crate::indic;
use crate::ot::{
    MASK_ABVF, MASK_ALL, MASK_BLWF, MASK_FINAL, MASK_HALF, MASK_INIT, MASK_INITIAL, MASK_ISOLATED,
    MASK_MEDIAL, MASK_PREF, MASK_PSTF, MASK_RPHF, Plan, Tables,
};
use crate::southeast::{MASK_CFAR, Southeast};
use crate::unicode_tables::{COMBINING, COMPOSITIONS, JOINING, MARKS, SCRIPTS};
use crate::universal;

/// The most characters shaped in one go. Longer text is split, at a space if one's near. Nothing
/// in a line is ever this long unless it's a single enormous line, which only needs to be right
/// in the part that shows.
pub(crate) const MAX_SHAPED_RUN: usize = 4096;

/// Which rules text needs, among what rsui treats differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum Script {
    Other,
    Common,
    Inherited,
    Latin,
    Greek,
    Cyrillic,
    Hebrew,
    Arabic,
    Syriac,
    Hangul,
    Han,
    Devanagari,
    Bengali,
    Gurmukhi,
    Gujarati,
    Oriya,
    Tamil,
    Telugu,
    Kannada,
    Malayalam,
    Thai,
    Lao,
    Khmer,
    Myanmar,
    /// One of the rarer scripts that share the same rules, by its number among them.
    Universal(u8),
}

/// The scripts that come from the table of them, by their numbers there.
const ALL_SCRIPTS: [Script; 11] = [
    Script::Other,
    Script::Common,
    Script::Inherited,
    Script::Latin,
    Script::Greek,
    Script::Cyrillic,
    Script::Hebrew,
    Script::Arabic,
    Script::Syriac,
    Script::Hangul,
    Script::Han,
];

impl Script {
    fn of(character: char) -> Self {
        match character as u32 {
            0x0900..=0x097F | 0xA8E0..=0xA8FF | 0x1CD0..=0x1CFF => Self::Devanagari,
            0x0980..=0x09FF => Self::Bengali,
            0x0A00..=0x0A7F => Self::Gurmukhi,
            0x0A80..=0x0AFF => Self::Gujarati,
            0x0B00..=0x0B7F => Self::Oriya,
            0x0B80..=0x0BFF => Self::Tamil,
            0x0C00..=0x0C7F => Self::Telugu,
            0x0C80..=0x0CFF => Self::Kannada,
            0x0D00..=0x0D7F => Self::Malayalam,
            0x0E00..=0x0E7F => Self::Thai,
            0x0E80..=0x0EFF => Self::Lao,
            0x1000..=0x109F | 0xA9E0..=0xA9FF | 0xAA60..=0xAA7F => Self::Myanmar,
            0x1780..=0x17FF | 0x19E0..=0x19FF => Self::Khmer,
            code => match universal::script_of(code) {
                Some(script) => Self::Universal(script),
                None => ALL_SCRIPTS[bidi::lookup_run(SCRIPTS, 4, code) as usize],
            },
        }
    }

    /// The tags a font can use for this script. If there are two, the first is the newer way.
    fn tags(self) -> &'static [&'static [u8; 4]] {
        match self {
            Self::Devanagari => &[b"dev2", b"deva"],
            Self::Bengali => &[b"bng2", b"beng"],
            Self::Gurmukhi => &[b"gur2", b"guru"],
            Self::Gujarati => &[b"gjr2", b"gujr"],
            Self::Oriya => &[b"ory2", b"orya"],
            Self::Tamil => &[b"tml2", b"taml"],
            Self::Telugu => &[b"tel2", b"telu"],
            Self::Kannada => &[b"knd2", b"knda"],
            Self::Malayalam => &[b"mlm2", b"mlym"],
            Self::Universal(script) => universal::tags_of(script),
            Self::Thai => &[b"thai"],
            Self::Lao => &[b"lao "],
            Self::Khmer => &[b"khmr"],
            Self::Myanmar => &[b"mym2", b"mymr"],
            Self::Latin => &[b"latn"],
            Self::Greek => &[b"grek"],
            Self::Cyrillic => &[b"cyrl"],
            Self::Hebrew => &[b"hebr"],
            Self::Arabic => &[b"arab"],
            Self::Syriac => &[b"syrc"],
            Self::Hangul => &[b"hang"],
            Self::Han => &[b"hani"],
            Self::Other | Self::Common | Self::Inherited => &[b"DFLT"],
        }
    }

    pub(crate) fn joins(self) -> bool {
        matches!(self, Self::Arabic | Self::Syriac)
    }
}

const POSITION_FEATURES: &[(&[u8; 4], u16)] = &[
    (b"abvm", MASK_ALL),
    (b"blwm", MASK_ALL),
    (b"curs", MASK_ALL),
    (b"dist", MASK_ALL),
    (b"kern", MASK_ALL),
    (b"mark", MASK_ALL),
    (b"mkmk", MASK_ALL),
];

/// What a font should run for this script and direction.
pub(crate) fn plan_for(tables: Tables<'_>, script: Script, right_to_left: bool) -> Plan {
    if indic::config_of(script).is_some() {
        return indic_plan(tables, script);
    }
    if let Some(script_kind) = Southeast::of(script) {
        return southeast_plan(tables, script, script_kind);
    }
    if matches!(script, Script::Universal(_)) {
        return universal_plan(tables, script);
    }
    // Mirroring comes from Unicode instead of `rtlm`, which would flip what's flipped already.
    let direction: &[(&[u8; 4], u16)] = if right_to_left {
        &[(b"rtla", MASK_ALL)]
    } else {
        &[(b"ltra", MASK_ALL), (b"ltrm", MASK_ALL)]
    };
    let stages: Vec<&[(&[u8; 4], u16)]> = if script.joins() {
        vec![
            &[(b"rvrn", MASK_ALL)],
            direction,
            &[(b"ccmp", MASK_ALL), (b"locl", MASK_ALL)],
            &[(b"isol", MASK_ISOLATED)],
            &[(b"fina", MASK_FINAL)],
            &[(b"fin2", MASK_FINAL)],
            &[(b"fin3", MASK_FINAL)],
            &[(b"medi", MASK_MEDIAL)],
            &[(b"med2", MASK_MEDIAL)],
            &[(b"init", MASK_INITIAL)],
            &[(b"rlig", MASK_ALL)],
            &[(b"rclt", MASK_ALL), (b"calt", MASK_ALL)],
            &[
                (b"liga", MASK_ALL),
                (b"clig", MASK_ALL),
                (b"mset", MASK_ALL),
            ],
        ]
    } else {
        vec![
            &[(b"rvrn", MASK_ALL)],
            direction,
            &[
                (b"ccmp", MASK_ALL),
                (b"locl", MASK_ALL),
                (b"rlig", MASK_ALL),
                (b"rclt", MASK_ALL),
                (b"calt", MASK_ALL),
                (b"clig", MASK_ALL),
                (b"liga", MASK_ALL),
            ],
        ]
    };
    Plan::new(tables, script.tags(), &stages, POSITION_FEATURES)
}

/// What a font runs for an Indic script, one stage at a time, each of which stays where it is for
/// [`indic`] to find, even if the font doesn't have it.
fn indic_plan(tables: Tables<'_>, script: Script) -> Plan {
    let stages: [&[(&[u8; 4], u16)]; 15] = [
        &[(b"rvrn", MASK_ALL)],
        &[(b"locl", MASK_ALL), (b"ccmp", MASK_ALL)],
        &[(b"nukt", MASK_ALL)],
        &[(b"akhn", MASK_ALL)],
        &[(b"rphf", MASK_RPHF)],
        &[(b"rkrf", MASK_ALL)],
        &[(b"pref", MASK_PREF)],
        &[(b"blwf", MASK_BLWF)],
        &[(b"abvf", MASK_ABVF)],
        &[(b"half", MASK_HALF)],
        &[(b"pstf", MASK_PSTF)],
        &[(b"vatu", MASK_ALL)],
        &[(b"cjct", MASK_ALL)],
        &[
            (b"init", MASK_INIT),
            (b"pres", MASK_ALL),
            (b"abvs", MASK_ALL),
            (b"blws", MASK_ALL),
            (b"psts", MASK_ALL),
            (b"haln", MASK_ALL),
        ],
        &[
            (b"calt", MASK_ALL),
            (b"clig", MASK_ALL),
            (b"liga", MASK_ALL),
            (b"rclt", MASK_ALL),
        ],
    ];
    Plan::new(tables, script.tags(), &stages, POSITION_FEATURES)
}

/// The features for the rarer scripts, in stages that fit around moving things into place. The numbers in
/// `universal.rs` go by these.
fn universal_plan(tables: Tables<'_>, script: Script) -> Plan {
    let stages: [&[(&[u8; 4], u16)]; 8] = [
        &[(b"rvrn", MASK_ALL)],
        &[
            (b"locl", MASK_ALL),
            (b"ccmp", MASK_ALL),
            (b"nukt", MASK_ALL),
            (b"akhn", MASK_ALL),
        ],
        &[(b"rphf", MASK_RPHF)],
        &[(b"pref", MASK_ALL)],
        &[
            (b"rkrf", MASK_ALL),
            (b"abvf", MASK_ALL),
            (b"blwf", MASK_ALL),
            (b"half", MASK_ALL),
            (b"pstf", MASK_ALL),
            (b"vatu", MASK_ALL),
            (b"cjct", MASK_ALL),
        ],
        &[
            (b"isol", MASK_ISOLATED),
            (b"init", MASK_INITIAL),
            (b"medi", MASK_MEDIAL),
            (b"fina", MASK_FINAL),
        ],
        &[
            (b"abvs", MASK_ALL),
            (b"blws", MASK_ALL),
            (b"haln", MASK_ALL),
            (b"pres", MASK_ALL),
            (b"psts", MASK_ALL),
        ],
        &[
            (b"calt", MASK_ALL),
            (b"rlig", MASK_ALL),
            (b"clig", MASK_ALL),
            (b"liga", MASK_ALL),
            (b"rclt", MASK_ALL),
        ],
    ];
    Plan::new(tables, script.tags(), &stages, POSITION_FEATURES)
}

/// The features for Khmer and Burmese, in stages that fit around moving things into place. See
/// [`Southeast::first_stage_after`].
fn southeast_plan(tables: Tables<'_>, script: Script, kind: Southeast) -> Plan {
    let after: &[(&[u8; 4], u16)] = &[
        (b"pres", MASK_ALL),
        (b"abvs", MASK_ALL),
        (b"blws", MASK_ALL),
        (b"psts", MASK_ALL),
    ];
    let common: &[(&[u8; 4], u16)] = &[
        (b"calt", MASK_ALL),
        (b"clig", MASK_ALL),
        (b"liga", MASK_ALL),
        (b"rlig", MASK_ALL),
        (b"rclt", MASK_ALL),
    ];
    let before: [&[(&[u8; 4], u16)]; 2] = [
        &[(b"rvrn", MASK_ALL)],
        &[(b"locl", MASK_ALL), (b"ccmp", MASK_ALL)],
    ];
    let stages: Vec<&[(&[u8; 4], u16)]> = match kind {
        // Khmer's forms all go at once, in the order the font has them.
        Southeast::Khmer => vec![
            before[0],
            before[1],
            &[
                (b"pref", MASK_PREF),
                (b"blwf", MASK_BLWF),
                (b"abvf", MASK_ABVF),
                (b"pstf", MASK_PSTF),
                (b"cfar", MASK_CFAR),
            ],
            after,
            common,
        ],
        // Burmese's each see what the one before made.
        Southeast::Burmese => vec![
            before[0],
            before[1],
            &[(b"rphf", MASK_ALL)],
            &[(b"pref", MASK_ALL)],
            &[(b"blwf", MASK_ALL)],
            &[(b"pstf", MASK_ALL)],
            after,
            common,
        ],
    };
    Plan::new(tables, script.tags(), &stages, POSITION_FEATURES)
}

/// Whether plain layout can't handle this character: it has to be shaped or reordered, or joins
/// what's around it. Getting this wrong towards `true` only costs time.
#[inline]
pub(crate) fn is_complex_character(character: char) -> bool {
    matches!(character as u32,
        0xAD
        | 0x300..=0x36F
        | 0x483..=0x489
        | 0x590..=0x1CFF
        | 0x1DC0..=0x1DFF
        | 0x200B..=0x200F
        | 0x202A..=0x202E
        | 0x2060..=0x206F
        | 0x20D0..=0x20FF
        | 0x25CC
        | 0x302A..=0x302F
        | 0x3099..=0x309A
        | 0xA670..=0xABFF
        | 0xFB00..=0xFDFF
        | 0xFE00..=0xFE0F
        | 0xFE20..=0xFE2F
        | 0xFE70..=0xFEFF
        | 0x10000..=0x1FFFF
        | 0xE0000..=0xEFFFF)
}

/// Whether this character shows nothing, and takes up no room.
pub(crate) fn is_invisible(character: char) -> bool {
    matches!(character as u32,
        0xAD | 0x34F | 0x61C | 0x180B..=0x180F | 0x200B..=0x200F | 0x202A..=0x202E
        | 0x2060..=0x206F | 0xFE00..=0xFE0F | 0xFEFF | 0xE0000..=0xE0FFF)
}

/// Whether this is a combining mark, which belongs with what's before it.
pub fn is_mark(character: char) -> bool {
    bidi::lookup_run(MARKS, 1, character as u32) == 1
}

fn combining_class(character: char) -> u8 {
    bidi::lookup_run(COMBINING, 8, character as u32) as u8
}

/// What a letter and a mark make together, if they have a character of their own.
fn composition(base: char, mark: char) -> Option<char> {
    let (base, mark) = (
        u16::try_from(base as u32).ok()?,
        u16::try_from(mark as u32).ok()?,
    );
    let (mut low, mut high) = (0, COMPOSITIONS.len() / 3);
    while low < high {
        let middle = (low + high) / 2;
        let at = middle * 3;
        match (COMPOSITIONS[at], COMPOSITIONS[at + 1]).cmp(&(base, mark)) {
            std::cmp::Ordering::Equal => return char::from_u32(u32::from(COMPOSITIONS[at + 2])),
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
        }
    }
    None
}

/// Puts marks in their standard order, and swaps a letter and a mark for the single character they
/// make wherever the font has one, as that's what it means to be drawn. Each character keeps the
/// byte position that was in its place, so that positions still only go up.
pub(crate) fn normalized(
    characters: &[(u32, char)],
    has_glyph: impl Fn(char) -> bool,
) -> Vec<(u32, char)> {
    let mut sorted = characters.to_vec();
    let mut start = 0;
    while start < sorted.len() {
        if combining_class(sorted[start].1) == 0 {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < sorted.len() && combining_class(sorted[end].1) != 0 {
            end += 1;
        }
        if end - start > 1 {
            let mut marks: Vec<char> = sorted[start..end].iter().map(|(_, c)| *c).collect();
            marks.sort_by_key(|mark| combining_class(*mark));
            for (slot, mark) in sorted[start..end].iter_mut().zip(marks) {
                slot.1 = mark;
            }
        }
        start = end;
    }
    let mut out: Vec<(u32, char)> = Vec::with_capacity(sorted.len());
    let mut index = 0;
    while index < sorted.len() {
        let (byte, base) = sorted[index];
        out.push((byte, base));
        index += 1;
        let marks_start = index;
        while index < sorted.len() && combining_class(sorted[index].1) != 0 {
            index += 1;
        }
        if marks_start == index {
            continue;
        }
        compose_marks(base, &sorted[marks_start..index], &has_glyph, &mut out);
    }
    out
}

/// Adds the marks on a letter just added to `out`, using a single character for as much as it can.
///
/// A mark that wouldn't compose stays apart, and then so does the rest, since a font's own marks
/// stack better than one that's built into a letter. Hebrew is the exception: its dots compose
/// across other points, as a shin's dot is part of which letter it is. So are Indic scripts, whose dots
/// make a different letter, whatever comes after them.
fn compose_marks(
    base: char,
    marks: &[(u32, char)],
    has_glyph: &impl Fn(char) -> bool,
    out: &mut Vec<(u32, char)>,
) {
    let indic =
        matches!(base as u32, 0x0900..=0x0DFF) || universal::script_of(base as u32).is_some();
    // Only a letter makes something new with a mark. Parts of a vowel are meant to stay apart there.
    if indic && is_mark(base) {
        out.extend_from_slice(marks);
        return;
    }
    let hebrew = indic || matches!(base as u32, 0x0590..=0x05FF | 0xFB1D..=0xFB4F);
    let mut composed = base;
    let mut left: Vec<(u32, char)> = Vec::new();
    let mut last_class = 0;
    for (byte, mark) in marks {
        let class = combining_class(*mark);
        let made = (last_class < class || left.is_empty())
            .then(|| composition(composed, *mark))
            .flatten()
            .filter(|made| has_glyph(*made));
        match made {
            Some(made) if left.is_empty() || hebrew => composed = made,
            _ => {
                left.push((*byte, *mark));
                last_class = class;
            }
        }
    }
    if left.is_empty() || hebrew {
        if let Some(last) = out.last_mut() {
            last.1 = composed;
        }
        out.extend(left);
    } else {
        out.extend_from_slice(marks);
    }
}

/// How a character joins its neighbors, in Arabic and similar scripts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Joining {
    None,
    Right,
    Left,
    Dual,
    Causing,
    Transparent,
}

fn joining_of(character: char) -> Joining {
    match bidi::lookup_run(JOINING, 3, character as u32) {
        1 => Joining::Right,
        2 => Joining::Left,
        3 => Joining::Dual,
        4 => Joining::Causing,
        5 => Joining::Transparent,
        _ => Joining::None,
    }
}

/// Works out each character's form in joining scripts, as a mask for its features.
pub(crate) fn joining_masks(characters: &[(u32, char)]) -> Vec<u16> {
    let types: Vec<Joining> = characters.iter().map(|(_, c)| joining_of(*c)).collect();
    let mut masks = vec![MASK_ALL; characters.len()];
    let mut previous: Option<usize> = None;
    let mut joined_before = false;
    for index in 0..characters.len() {
        let kind = types[index];
        if kind == Joining::Transparent {
            continue;
        }
        let joins_previous =
            previous.is_some_and(|at| {
                matches!(types[at], Joining::Left | Joining::Dual | Joining::Causing)
            }) && matches!(kind, Joining::Right | Joining::Dual | Joining::Causing);
        if let Some(at) = previous {
            if joins_previous {
                // The earlier one now knows it joins forward.
                masks[at] |= if joined_before {
                    MASK_MEDIAL
                } else {
                    MASK_INITIAL
                };
            } else {
                masks[at] |= if joined_before {
                    MASK_FINAL
                } else {
                    MASK_ISOLATED
                };
            }
        }
        joined_before = joins_previous;
        previous = Some(index);
    }
    if let Some(at) = previous {
        masks[at] |= if joined_before {
            MASK_FINAL
        } else {
            MASK_ISOLATED
        };
    }
    masks
}

/// Splits a line's characters into runs that can be shaped together, as ranges of positions.
pub(crate) fn runs_of(
    levels: &[u8],
    scripts: &[Script],
    faces: &[u16],
    characters: &[(u32, char)],
) -> Vec<std::ops::Range<usize>> {
    let mut runs = Vec::new();
    let mut start = 0;
    let mut index = 1;
    while index <= levels.len() {
        if index == levels.len()
            || levels[index] != levels[start]
            || scripts[index] != scripts[start]
            || faces[index] != faces[start]
        {
            runs.push(start..index);
            start = index;
        } else if index - start >= MAX_SHAPED_RUN {
            // A very long run breaks at a space, if there's one nearby.
            let end = (start + 1..index)
                .rev()
                .take(256)
                .find(|at| characters[*at].1 == ' ')
                .map_or(index, |at| at + 1);
            runs.push(start..end);
            start = end;
        }
        index += 1;
    }
    runs
}

/// Gives characters with no script of their own, like spaces and marks, the one around them.
pub(crate) fn resolve_scripts(characters: &[(u32, char)]) -> Vec<Script> {
    let mut scripts: Vec<Script> = characters.iter().map(|(_, c)| Script::of(*c)).collect();
    let mut previous = None;
    for script in &mut scripts {
        if matches!(*script, Script::Common | Script::Inherited) {
            if let Some(previous) = previous {
                *script = previous;
            }
        } else {
            previous = Some(*script);
        }
    }
    // Anything before the first letter takes on what comes after.
    let mut next = None;
    for script in scripts.iter_mut().rev() {
        if matches!(*script, Script::Common | Script::Inherited) {
            if let Some(next) = next {
                *script = next;
            }
        } else {
            next = Some(*script);
        }
    }
    // With no letters at all, what's left is all one thing, like the parts of an emoji.
    for script in &mut scripts {
        if *script == Script::Inherited {
            *script = Script::Common;
        }
    }
    scripts
}

/// The levels for one line of a paragraph, with rule L1 applied.
pub(crate) fn line_levels(levels: &[u8], classes: &[Class], paragraph_level: u8) -> Vec<u8> {
    let mut line = levels.to_vec();
    bidi::reset_line_levels(&mut line, classes, paragraph_level);
    line
}
