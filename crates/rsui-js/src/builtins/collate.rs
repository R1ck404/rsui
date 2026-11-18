//! Collation: the order `Intl.Collator` and `localeCompare` put text in. It is Unicode's collation algorithm (UTS #10)
//! with CLDR's root data (`collate_tables.rs`), which is what Edge's and Node's ICU use for every language that doesn't
//! change the order, English among them, with ICU's options: how much difference counts (`sensitivity`), numbers
//! compared by value (`numeric`), capitals first (`caseFirst: 'upper'`) and punctuation ignored (`ignorePunctuation`).
//!
//! Two strings are compared without making their sort keys: each gives its collation elements one at a time, and the
//! first difference in primary weights (which letter), then secondary (which accent), then tertiary (which case) is the
//! answer, so most comparisons stop after a few characters, and the start two strings share is skipped. Nothing is
//! allocated unless the text has combining marks, which have to be put in order (NFD) first, as the algorithm needs.
//!
//! The order is ICU's for Latin, Greek, Cyrillic and the other alphabets, kana and Hangul, and for the common Han
//! characters, which CLDR puts in radical-stroke order (`collate_han.rs`). Han from Extension B on goes after those, by
//! code point, where ICU interleaves it by radical too. Languages that change the order (Swedish, German phonebook,
//! Chinese pinyin...) get the root order.

use std::cmp::Ordering;
use std::sync::OnceLock;

use super::collate_han::HAN_RUNS;
use super::collate_tables::{CONTRACTIONS, DIGIT_ZEROS, EXPANSIONS, RUN_STARTS, RUN_VALUES};
use super::normalize::{Form, combining_class, normalize};
use super::normalize_tables::COMBINING;
use crate::object::Kind;
use crate::text::Text;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

/// What `Intl.Collator` was asked for, in the form ICU has it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Options {
    /// How many levels count: 1 for `base` and `case`, 2 for `accent`, 3 for `variant`.
    pub strength: u8,
    /// Whether case counts after the letters, for `sensitivity: 'case'`.
    pub case_level: bool,
    /// `caseFirst: 'upper'`.
    pub upper_first: bool,
    pub numeric: bool,
    pub ignore_punctuation: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            strength: 3,
            case_level: false,
            upper_first: false,
            numeric: false,
            ignore_punctuation: false,
        }
    }
}

impl Options {
    /// The options `sensitivity` names, or `None` for one that isn't a sensitivity.
    pub(crate) fn with_sensitivity(mut self, sensitivity: &str) -> Option<Self> {
        (self.strength, self.case_level) = match sensitivity {
            "base" => (1, false),
            "accent" => (2, false),
            "case" => (1, true),
            "variant" => (3, false),
            _ => return None,
        };
        Some(self)
    }

    /// The options as a number, to keep in a compare function.
    pub(crate) fn to_bits(self) -> i32 {
        self.strength as i32
            | (self.case_level as i32) << 2
            | (self.upper_first as i32) << 3
            | (self.numeric as i32) << 4
            | (self.ignore_punctuation as i32) << 5
    }

    pub(crate) fn from_bits(bits: i32) -> Self {
        Self {
            strength: (bits & 3) as u8,
            case_level: bits & 4 != 0,
            upper_first: bits & 8 != 0,
            numeric: bits & 16 != 0,
            ignore_punctuation: bits & 32 != 0,
        }
    }
}

/// An element of punctuation, spaces or symbols, which `ignorePunctuation` ignores.
const VARIABLE: u32 = 1 << 30;
/// A value in the run table that isn't an element: see `RUN_VALUES`.
const EXPANDS: u32 = 1 << 31;

const HANGUL_FIRST: u32 = 0xAC00;
const HANGUL_COUNT: u32 = 11172;
const VOWEL_COUNT: u32 = 21;
const TRAILING_COUNT: u32 = 28;

#[inline]
fn primary(element: u32) -> u32 {
    element >> 14 & 0xFFFF
}

#[inline]
fn secondary(element: u32) -> u32 {
    element >> 5 & 0x1FF
}

#[inline]
fn tertiary(element: u32) -> u32 {
    element & 0x1F
}

const fn element(primary: u32, secondary: u32, tertiary: u32) -> u32 {
    primary << 14 | secondary << 5 | tertiary
}

/// The secondary and tertiary weights of a plain letter.
const COMMON_SECONDARY: u32 = 0x20;
const COMMON_TERTIARY: u32 = 0x02;

/// Whether a tertiary weight is a capital's. Normal-sized kana count as capitals and small ones as lowercase, as ICU
/// has them, so that `caseFirst` orders those too.
#[inline]
fn is_upper(tertiary: u32) -> bool {
    matches!(tertiary, 0x08..=0x0C | 0x0E | 0x11 | 0x12 | 0x1D)
}

/// Where in a sorted table of runs keyed by code point (shifted up) each 256 code points start, so that finding a code
/// point's run only searches the few runs near it.
struct Pages {
    starts: Vec<u16>,
    shift: u32,
}

impl Pages {
    fn new(keys: &[u32], shift: u32, last_point: u32) -> Self {
        let pages = (last_point >> 8) + 2;
        let starts = (0..pages)
            .map(|page| keys.partition_point(|&key| key >> shift < page << 8) as u16)
            .collect();
        Self { starts, shift }
    }

    /// The index of the last run in `keys` that starts at or before `point`.
    #[inline]
    fn find(&self, keys: &[u32], point: u32) -> Option<usize> {
        let page = (point >> 8) as usize;
        let low = (*self.starts.get(page)? as usize).saturating_sub(1);
        let high = *self.starts.get(page + 1)? as usize;
        let key = point << self.shift | ((1 << self.shift) - 1);
        let at = low + keys[low..high].partition_point(|&start| start <= key);
        at.checked_sub(1)
    }
}

/// What's worked out from the tables the first time text that isn't ASCII is compared, to find things faster.
struct Index {
    runs: Pages,
    han: Pages,
    /// Which 64 code points of the Basic Multilingual Plane have a combining mark among them.
    marks: [u64; 16],
    /// Which code points of the Basic Multilingual Plane start a contraction.
    starts_contraction: Box<[u64; 1024]>,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut marks = [0u64; 16];
        for (at, &run) in COMBINING.iter().enumerate() {
            if run & 0xFF == 0 || run >> 8 >= 0x10000 {
                continue;
            }
            let end = COMBINING.get(at + 1).map_or(0x110000, |next| next >> 8).min(0x10000);
            for block in (run >> 8) / 64..end.div_ceil(64) {
                marks[block as usize / 64] |= 1 << (block % 64);
            }
        }
        let mut starts_contraction = Box::new([0u64; 1024]);
        for entry in CONTRACTIONS.iter().filter(|entry| entry.0 < 0x10000) {
            starts_contraction[entry.0 as usize / 64] |= 1 << (entry.0 % 64);
        }
        Index {
            runs: Pages::new(RUN_STARTS, 11, 0x10FFFF),
            han: Pages::new(HAN_RUNS, 16, 0xFFFF),
            marks,
            starts_contraction,
        }
    })
}

/// Whether a code point might be a combining mark, which only those in a block with one can be.
#[inline]
fn maybe_mark(point: u32) -> bool {
    point > 0xFFFF || index().marks[point as usize / 4096] & 1 << (point / 64 % 64) != 0
}

/// The run-coded value of one code point: its element, or where its elements are in `EXPANSIONS`.
fn lookup(point: u32) -> Option<u32> {
    let at = index().runs.find(RUN_STARTS, point)? + 1;
    let start = RUN_STARTS[at - 1];
    let first = start >> 11;
    let length = (start & 0x7FF) + 1;
    if point >= first + length {
        return None;
    }
    let value = RUN_VALUES[at - 1];
    Some(if value & EXPANDS != 0 { value } else { value + ((point - first) << 14) })
}

/// The place of a common Han character in radical-stroke order, which is CLDR's.
fn han_rank(point: u32) -> Option<u32> {
    if !matches!(point, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xFA0E..=0xFA29) {
        return None;
    }
    let run = HAN_RUNS[index().han.find(HAN_RUNS, point)?];
    // The compatibility ideographs between the twelve unified ones aren't in a run, but they're in the main table, so
    // they don't get here.
    Some((run & 0xFFFF) + (point - (run >> 16)))
}

/// The elements of a code point the tables don't list, worked out from it, as UTS #10 says: the common Han characters
/// by radical-stroke order, then the rest of Han and a few other scripts by where they are in Unicode, after
/// everything else, and code points with no character after those.
fn implicit(point: u32) -> [u32; 2] {
    if let Some(rank) = han_rank(point) {
        return [
            element(0xFB40 + (rank >> 15), COMMON_SECONDARY, COMMON_TERTIARY),
            element(rank & 0x7FFF | 0x8000, 0, 0),
        ];
    }
    let (lead, rest) = match point {
        0x17000..=0x18AFF | 0x18D00..=0x18D8F => (0xFB00, point - 0x17000),
        0x1B170..=0x1B2FF => (0xFB01, point - 0x1B170),
        0x18B00..=0x18CFF => (0xFB02, point - 0x18B00),
        0x20000..=0x2F7FF | 0x2FA20..=0x3FFFF => (0xFB80 + (point >> 15), point & 0x7FFF),
        _ => (0xFBC0 + (point >> 15), point & 0x7FFF),
    };
    [element(lead, COMMON_SECONDARY, COMMON_TERTIARY), element(rest | 0x8000, 0, 0)]
}

/// The value of a decimal digit of any script.
#[inline]
fn digit_value(point: u32) -> Option<u32> {
    if point < 0x660 {
        // Arabic-Indic digits are the first after ASCII's.
        return (b'0' as u32..=b'9' as u32).contains(&point).then(|| point - b'0' as u32);
    }
    let at = DIGIT_ZEROS.partition_point(|&zero| zero <= point);
    let zero = *DIGIT_ZEROS.get(at.checked_sub(1)?)?;
    (point - zero < 10).then(|| point - zero)
}

/// What's looked up most: ASCII's elements, and which ASCII characters start a contraction (`l·`, in Catalan).
struct Ascii {
    elements: [u32; 128],
    starts_contraction: u128,
}

fn ascii() -> &'static Ascii {
    static TABLE: OnceLock<Ascii> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut elements = [0; 128];
        for (point, slot) in elements.iter_mut().enumerate() {
            // Every ASCII character has one element.
            *slot = lookup(point as u32).unwrap_or(0);
        }
        let starts_contraction = CONTRACTIONS
            .iter()
            .filter(|entry| entry.0 < 0x80)
            .fold(0, |mask, entry| mask | 1 << entry.0);
        Ascii {
            elements,
            starts_contraction,
        }
    })
}

/// The primary weight numbers start with when `numeric` is on: just before `0`'s, as ICU has it, so that a number goes
/// before a digit that isn't one (`²`).
fn number_primary() -> u32 {
    primary(ascii().elements[b'0' as usize]) - 1
}

/// Text, as one string gives it: ASCII, or UTF-16 already taken apart.
#[derive(Clone, Copy)]
enum Source<'a> {
    Ascii(&'a [u8]),
    Wide(&'a [u16]),
}

impl<'a> Source<'a> {
    fn len(&self) -> usize {
        match self {
            Self::Ascii(bytes) => bytes.len(),
            Self::Wide(units) => units.len(),
        }
    }

    /// The code unit at `at`, or `None` at the end.
    #[inline]
    fn unit(&self, at: usize) -> Option<u16> {
        match self {
            Self::Ascii(bytes) => bytes.get(at).map(|&byte| byte as u16),
            Self::Wide(units) => units.get(at).copied(),
        }
    }

    fn from(&self, at: usize) -> Source<'a> {
        match *self {
            Self::Ascii(bytes) => Self::Ascii(&bytes[at..]),
            Self::Wide(units) => Self::Wide(&units[at..]),
        }
    }

    /// The code point at `at`, and where the next one starts.
    #[inline]
    fn point_at(&self, at: usize) -> (u32, usize) {
        match self {
            Self::Ascii(bytes) => (bytes[at] as u32, at + 1),
            Self::Wide(units) => {
                let unit = units[at] as u32;
                if (0xD800..0xDC00).contains(&unit) {
                    if let Some(&low) = units.get(at + 1) {
                        if (0xDC00..0xE000).contains(&low) {
                            return (0x10000 + ((unit - 0xD800) << 10) + (low as u32 - 0xDC00), at + 2);
                        }
                    }
                }
                (unit, at + 1)
            }
        }
    }
}

/// The collation elements of a string, one at a time.
#[derive(Clone)]
struct Elements<'a> {
    text: Source<'a>,
    at: usize,
    /// The rest of a character's elements, after the first.
    pending: &'static [u32],
    /// One or two more elements worked out here: the second half of an implicit weight, how many digits a number has,
    /// or the rest of a Hangul syllable's letters. 0 is none.
    extra: u32,
    after_extra: u32,
    /// The digits of a number still to give, from `at`, when `numeric` is on.
    digits: usize,
    numeric: bool,
    ignore_punctuation: bool,
    /// Whether the last element with a primary weight was punctuation that's being ignored, so that accents on it are
    /// too.
    ignoring: bool,
    ascii: &'static Ascii,
    /// Whether to stop at a combining mark, in text that hasn't been taken apart; and whether it did.
    watch_marks: bool,
    found_mark: bool,
    /// Whether contractions can't start with ASCII here, as the text is all ASCII.
    all_ascii: bool,
    zero: u32,
}

impl<'a> Elements<'a> {
    fn new(text: Source<'a>, options: Options, watch_marks: bool) -> Self {
        Self {
            watch_marks,
            found_mark: false,
            text,
            at: 0,
            pending: &[],
            extra: 0,
            after_extra: 0,
            digits: 0,
            numeric: options.numeric,
            ignore_punctuation: options.ignore_punctuation,
            ignoring: false,
            ascii: ascii(),
            all_ascii: matches!(text, Source::Ascii(_)),
            zero: number_primary(),
        }
    }

    /// The next element, or `None` at the end. Elements that are ignorable at every level aren't given.
    #[inline]
    fn next(&mut self) -> Option<u32> {
        loop {
            let element = self.next_any()?;
            if element == 0 {
                continue;
            }
            if self.ignore_punctuation {
                if element & VARIABLE != 0 {
                    self.ignoring = true;
                    continue;
                }
                if primary(element) != 0 {
                    self.ignoring = false;
                } else if self.ignoring {
                    continue;
                }
            }
            return Some(element);
        }
    }

    #[inline]
    fn next_any(&mut self) -> Option<u32> {
        if self.extra != 0 {
            let extra = self.extra;
            self.extra = std::mem::take(&mut self.after_extra);
            return Some(extra);
        }
        if let Some((&first, rest)) = self.pending.split_first() {
            self.pending = rest;
            return Some(first);
        }
        if self.digits > 0 {
            self.digits -= 1;
            let (point, next) = self.text.point_at(self.at);
            self.at = next;
            let value = digit_value(point).unwrap_or(0);
            return Some(element(self.zero + 1 + value, 0, 0));
        }
        if self.at >= self.text.len() {
            return None;
        }
        let (point, next) = self.text.point_at(self.at);
        if point < 0x80
            && !(self.numeric && (0x30..=0x39).contains(&point))
            && (self.all_ascii || self.ascii.starts_contraction & 1 << point == 0)
        {
            self.at = next;
            return Some(self.ascii.elements[point as usize]);
        }
        if self.watch_marks && (is_mark(point) || next < self.text.len() && is_mark(self.text.point_at(next).0)) {
            // Marks may need putting in order, so the text has to be taken apart to be compared.
            self.found_mark = true;
            return None;
        }
        if self.numeric && digit_value(point).is_some() {
            return Some(self.number());
        }
        self.at = next;
        if (HANGUL_FIRST..HANGUL_FIRST + HANGUL_COUNT).contains(&point) {
            return Some(self.syllable(point - HANGUL_FIRST));
        }
        let value = match self.contraction(point) {
            Some(value) => value,
            None => match lookup(point) {
                Some(value) => value,
                None => {
                    let [first, second] = implicit(point);
                    self.extra = second;
                    return Some(first);
                }
            },
        };
        if value & EXPANDS != 0 {
            let offset = (value >> 8 & 0x7F_FFFF) as usize;
            let count = (value & 0xFF) as usize;
            if count == 0 {
                // It collates as a Han character, with its own tertiary weight.
                let [first, second] = implicit(value >> 8 & 0x3_FFFF);
                self.extra = second;
                return Some(first & !0x1F | value >> 26 & 0x1F);
            }
            let elements = &EXPANSIONS[offset..offset + count];
            let (&first, rest) = elements.split_first()?;
            self.pending = rest;
            return Some(first);
        }
        Some(value)
    }

    /// A Hangul syllable, which collates as the letters it's made of, as UTS #10 says: this gives the first and leaves
    /// the others to come. Each of the letters has one element.
    fn syllable(&mut self, index: u32) -> u32 {
        let letter = |point: u32| lookup(point).filter(|value| value & EXPANDS == 0).unwrap_or(0);
        let trailing = index % TRAILING_COUNT;
        self.extra = letter(0x1161 + index % (VOWEL_COUNT * TRAILING_COUNT) / TRAILING_COUNT);
        if trailing != 0 {
            self.after_extra = letter(0x11A7 + trailing);
        }
        letter(0x1100 + index / (VOWEL_COUNT * TRAILING_COUNT))
    }

    /// A run of digits, as ICU has numbers: leading zeros don't count, then a number with more digits is bigger, and
    /// then the digits decide. This gives the first element and leaves the count and the digits to come.
    fn number(&mut self) -> u32 {
        let mut at = self.at;
        // Leading zeros are skipped.
        while at < self.text.len() {
            let (point, next) = self.text.point_at(at);
            if digit_value(point) != Some(0) {
                break;
            }
            at = next;
        }
        let first = at;
        let mut count = 0;
        while at < self.text.len() {
            let (point, next) = self.text.point_at(at);
            if digit_value(point).is_none() {
                break;
            }
            count += 1;
            at = next;
        }
        self.at = first;
        self.digits = count;
        self.extra = element(self.zero + count.min(0xFFFF - self.zero as usize) as u32, 0, 0);
        element(self.zero, COMMON_SECONDARY, COMMON_TERTIARY)
    }

    /// The elements of the longest sequence that collates as one starting with `point`, if there is one; it moves past
    /// what else it takes.
    fn contraction(&mut self, point: u32) -> Option<u32> {
        let Source::Wide(_) = self.text else {
            return None;
        };
        if point < 0x10000 && index().starts_contraction[point as usize / 64] & 1 << (point % 64) == 0 {
            return None;
        }
        let from = CONTRACTIONS.partition_point(|entry| entry.0 < point);
        if CONTRACTIONS.get(from).is_none_or(|entry| entry.0 != point) || self.at >= self.text.len() {
            return None;
        }
        let (second, after_second) = self.text.point_at(self.at);
        let third = (after_second < self.text.len()).then(|| self.text.point_at(after_second));
        let mut found = None;
        for &(first, two, three, value) in &CONTRACTIONS[from..] {
            if first != point {
                break;
            }
            if two != second {
                continue;
            }
            if three == 0 {
                found = found.or(Some((value, after_second)));
            } else if let Some((third, after_third)) = third {
                if three == third {
                    found = Some((value, after_third));
                    break;
                }
            }
        }
        let (value, next) = found?;
        self.at = next;
        Some(value)
    }
}

/// Compares the weights one level gives, skipping elements with none at that level. `None` is that one of the two came
/// to a combining mark it was watching for.
#[inline]
fn compare_level(mut one: Elements, mut other: Elements, weight: impl Fn(u32) -> u32) -> Option<Ordering> {
    loop {
        let a = loop {
            match one.next() {
                Some(element) => match weight(element) {
                    0 => continue,
                    weight => break weight,
                },
                None => break 0,
            }
        };
        let b = loop {
            match other.next() {
                Some(element) => match weight(element) {
                    0 => continue,
                    weight => break weight,
                },
                None => break 0,
            }
        };
        if (a == 0 || b == 0) && (one.found_mark || other.found_mark) {
            return None;
        }
        if a != b {
            return Some(a.cmp(&b));
        }
        if a == 0 {
            return Some(Ordering::Equal);
        }
    }
}

/// Compares two texts level by level; `None` if marks were watched for and found.
fn compare_sources(one: Source, other: Source, options: Options, watch_marks: bool) -> Option<Ordering> {
    let a = Elements::new(one, options, watch_marks);
    let b = Elements::new(other, options, watch_marks);
    let order = compare_level(a.clone(), b.clone(), primary)?;
    if order != Ordering::Equal {
        return Some(order);
    }
    if options.strength >= 2 {
        let order = compare_level(a.clone(), b.clone(), secondary)?;
        if order != Ordering::Equal {
            return Some(order);
        }
    }
    let upper_first = options.upper_first;
    if options.case_level {
        // Only what has a primary weight has a case: 1 for what comes first, 2 for what comes after.
        let case = move |element: u32| match primary(element) {
            0 => 0,
            _ if is_upper(tertiary(element)) == upper_first => 1,
            _ => 2,
        };
        let order = compare_level(a.clone(), b.clone(), case)?;
        if order != Ordering::Equal {
            return Some(order);
        }
    }
    if options.strength >= 3 {
        let third = move |element: u32| match tertiary(element) {
            0 => 0,
            weight if upper_first && !is_upper(weight) => weight | 0x20,
            weight => weight,
        };
        return compare_level(a, b, third);
    }
    Some(Ordering::Equal)
}


/// Whether a code point is a combining mark that taking text apart (NFD) might put in another order.
#[inline]
fn is_mark(point: u32) -> bool {
    point >= 0x300 && maybe_mark(point) && combining_class(point) != 0
}

fn source(text: &Text) -> Source<'_> {
    match text.ascii() {
        Some(bytes) => Source::Ascii(bytes),
        None => Source::Wide(text.wide().unwrap_or(&[])),
    }
}

/// How many code units at the start of two texts can be skipped when comparing them, as they're the same and give the
/// same elements: names in a folder often start the same way. It's where they first differ, or before that, so that
/// what comes next can't change what the same part gives: so not in the middle of a number, and not just before
/// anything other than ASCII, which may be a mark, or what makes a contraction with what's before.
fn same_start(one: Source, other: Source, options: Options) -> usize {
    let mut at = match (one, other) {
        (Source::Ascii(a), Source::Ascii(b)) => a.iter().zip(b).take_while(|(x, y)| x == y).count(),
        (Source::Wide(a), Source::Wide(b)) => a.iter().zip(b).take_while(|(x, y)| x == y).count(),
        (Source::Ascii(a), Source::Wide(b)) | (Source::Wide(b), Source::Ascii(a)) => {
            a.iter().zip(b).take_while(|(x, y)| **x as u16 == **y).count()
        }
    };
    let is_digit = |unit: Option<u16>| unit.is_some_and(|unit| (b'0' as u16..=b'9' as u16).contains(&unit));
    while at > 0 {
        let before = one.unit(at - 1).unwrap_or(0);
        let (next, other_next) = (one.unit(at), other.unit(at));
        let plain = |unit: Option<u16>| unit.is_none_or(|unit| unit < 0x80);
        let safe = plain(next)
            && plain(other_next)
            && !(0xD800..0xE000).contains(&before)
            && !(before >= 0x80 && index().starts_contraction[before as usize / 64] & 1 << (before % 64) != 0)
            && !(options.numeric && digit_value(before as u32).is_some() && (is_digit(next) || is_digit(other_next)));
        if safe {
            break;
        }
        at -= 1;
    }
    at
}

/// Compares two strings as `Intl.Collator` with these options does.
///
/// Letters with accents are in the table as they are, as they are in CLDR's, so text is compared as it is until either
/// comes to a combining mark (or a letter just before one), which is rare. Only then are both taken apart (NFD), which
/// puts marks in order, and compared again. Taking text apart costs more than comparing it.
pub(crate) fn compare(one: &Text, other: &Text, options: Options) -> Ordering {
    let (a, b) = (source(one), source(other));
    let same = same_start(a, b, options);
    if let Some(order) = compare_sources(a.from(same), b.from(same), options, true) {
        return order;
    }
    let apart = |text: &Text| text.wide().and_then(|units| normalize(units, Form::Nfd));
    let (one_apart, other_apart) = (apart(one), apart(other));
    let one = one_apart.as_deref().map_or(source(one), Source::Wide);
    let other = other_apart.as_deref().map_or(source(other), Source::Wide);
    compare_sources(one, other, options, false).unwrap_or(Ordering::Equal)
}

/// What the `-u-kn` and `-u-kf` extensions of a locale ask for, as options that come in the options object override.
fn extension_options(locale: &str, mut options: Options) -> Options {
    let lower = locale.to_ascii_lowercase();
    let Some(at) = lower.find("-u-") else {
        return options;
    };
    let keys: Vec<&str> = lower[at + 3..].split('-').take_while(|key| *key != "x").collect();
    for (index, key) in keys.iter().enumerate() {
        let value = keys.get(index + 1).copied().filter(|value| value.len() > 2);
        match *key {
            "kn" => options.numeric = value != Some("false"),
            "kf" => options.upper_first = value == Some("upper"),
            _ => {}
        }
    }
    options
}

/// The options `Intl.Collator` and `localeCompare` are given, read as ECMA-402 says.
pub(crate) fn options_of(vm: &mut Vm, locales: Value, given: Value) -> Run<Options> {
    let mut options = Options::default();
    let locale = if locales.is_string() {
        Some(locales)
    } else if locales.is_object() {
        let first = vm.get_named(locales, "0")?;
        (!first.is_undefined()).then_some(first)
    } else {
        None
    };
    if let Some(locale) = locale {
        let locale = vm.to_rust_string(locale)?;
        options = extension_options(&locale, options);
    }
    if given.is_undefined() {
        return Ok(options);
    }
    if !given.is_object() {
        return Err(vm.type_error("Collator options should be an object"));
    }
    let numeric = vm.get_named(given, "numeric")?;
    if !numeric.is_undefined() {
        options.numeric = vm.to_boolean(numeric);
    }
    let case_first = vm.get_named(given, "caseFirst")?;
    if !case_first.is_undefined() {
        let case_first = vm.to_rust_string(case_first)?;
        options.upper_first = match case_first.as_str() {
            "upper" => true,
            "lower" | "false" => false,
            _ => return Err(vm.range_error(&format!("Value {case_first} out of range for collator options property caseFirst"))),
        };
    }
    let sensitivity = vm.get_named(given, "sensitivity")?;
    if !sensitivity.is_undefined() {
        let sensitivity = vm.to_rust_string(sensitivity)?;
        options = match options.with_sensitivity(&sensitivity) {
            Some(options) => options,
            None => {
                return Err(vm.range_error(&format!("Value {sensitivity} out of range for collator options property sensitivity")));
            }
        };
    }
    let ignore_punctuation = vm.get_named(given, "ignorePunctuation")?;
    if !ignore_punctuation.is_undefined() {
        options.ignore_punctuation = vm.to_boolean(ignore_punctuation);
    }
    Ok(options)
}

fn string_arg(vm: &mut Vm, args: &Args, index: usize) -> Run<Value> {
    let value = vm.arg(args, index);
    if value.is_string() { Ok(value) } else { vm.to_js_string(value) }
}

/// A collator's `compare`: the options are in the function's data.
fn compare_native(vm: &mut Vm, args: Args) -> Run<Value> {
    let options = Options::from_bits(vm.native_data_of(&args).as_int());
    let one = string_arg(vm, &args, 0)?;
    let other = string_arg(vm, &args, 1)?;
    let order = compare(vm.text_of(one), vm.text_of(other), options);
    Ok(Value::int(order as i32))
}

/// `collator(locales, options)`, which makes the `compare` of a new `Intl.Collator`: a function of two strings that
/// keeps its options with it, so that sorting calls straight into Rust.
pub(crate) fn make_compare(vm: &mut Vm, args: Args) -> Run<Value> {
    let locales = vm.arg(&args, 0);
    let given = vm.arg(&args, 1);
    let options = options_of(vm, locales, given)?;
    let name = vm.intern_str("");
    let compare = vm.new_native(name, 2, compare_native, Value::int(options.to_bits()), false);
    // Comparing two strings depends on nothing else, so what's worked out with it can be kept (see `NativeData::pure`).
    if let Kind::Native(native) = &mut vm.objects[compare.index()].kind {
        native.pure = true;
    }
    Ok(compare)
}

/// What a native compare function's options are, for `resolvedOptions`: `[numeric, sensitivity, caseFirst,
/// ignorePunctuation]`.
pub(crate) fn compare_options(vm: &mut Vm, args: Args) -> Run<Value> {
    let function = vm.arg(&args, 0);
    let bits = match function.is_object().then(|| &vm.objects[function.index()].kind) {
        Some(Kind::Native(native)) if std::ptr::fn_addr_eq(native.func, compare_native as crate::object::NativeFn) => native.data.as_int(),
        _ => return Ok(Value::UNDEFINED),
    };
    let options = Options::from_bits(bits);
    let sensitivity = match (options.strength, options.case_level) {
        (1, false) => "base",
        (1, true) => "case",
        (2, _) => "accent",
        _ => "variant",
    };
    let sensitivity = vm.new_str(sensitivity);
    let case_first = vm.new_str(if options.upper_first { "upper" } else { "false" });
    Ok(vm.new_array(vec![
        Value::bool(options.numeric),
        sensitivity,
        case_first,
        Value::bool(options.ignore_punctuation),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICU_ORDERS: &str = include_str!("../../tests/data/collation-icu.txt");

    fn text(name: &str) -> Text {
        Text::from_str(name)
    }

    fn options_named(words: &str) -> Options {
        let mut options = Options::default();
        for word in words.split(' ') {
            match word.split_once('=') {
                Some(("sensitivity", value)) => options = options.with_sensitivity(value).unwrap(),
                Some(("caseFirst", value)) => options.upper_first = value == "upper",
                None if word == "numeric" => options.numeric = true,
                None if word == "ignorePunctuation" => options.ignore_punctuation = true,
                None if word == "default" => {}
                _ => panic!("unknown option {word}"),
            }
        }
        options
    }

    /// Every name compares with the next as Node's ICU said, for each set of options; as the order is a total one,
    /// that makes sorting give ICU's order.
    #[test]
    fn orders_names_as_icu_does() {
        // After the two lines that say what this is.
        let mut lines = ICU_ORDERS.lines().skip(2);
        let count: usize = lines.next().unwrap().strip_prefix("names ").unwrap().parse().unwrap();
        let names: Vec<Text> = (0..count)
            .map(|_| text(&lines.next().unwrap().replace("\\t", "\t").replace("\\0", "\0")))
            .collect();
        let mut wrong = Vec::new();
        let mut checked = 0;
        while let Some(heading) = lines.next() {
            let options = options_named(heading.strip_prefix("order ").unwrap());
            let mut previous: Option<usize> = None;
            for item in lines.next().unwrap().split(' ') {
                let (index, sign) = match item.split_once(':') {
                    Some((index, sign)) => (index.parse::<usize>().unwrap(), sign.parse::<i32>().unwrap()),
                    None => (item.parse().unwrap(), 0),
                };
                if let Some(previous) = previous {
                    let order = compare(&names[previous], &names[index], options) as i32;
                    checked += 1;
                    if order != sign {
                        wrong.push(format!(
                            "{heading}: {:?} vs {:?} gave {order}, ICU {}",
                            names[previous].to_rust(),
                            names[index].to_rust(),
                            sign
                        ));
                    }
                }
                previous = Some(index);
            }
        }
        assert!(checked > 10_000);
        assert!(wrong.is_empty(), "{} of {checked} differ from ICU:\n{}", wrong.len(), wrong.join("\n"));
    }

    #[test]
    fn compares_by_level() {
        let order = |a: &str, b: &str, options: Options| compare(&text(a), &text(b), options);
        let variant = Options::default();
        let base = variant.with_sensitivity("base").unwrap();
        let accent = variant.with_sensitivity("accent").unwrap();
        let case = variant.with_sensitivity("case").unwrap();
        assert_eq!(order("a", "b", variant), Ordering::Less);
        assert_eq!(order("a", "A", variant), Ordering::Less);
        assert_eq!(order("a", "A", base), Ordering::Equal);
        assert_eq!(order("a", "á", base), Ordering::Equal);
        assert_eq!(order("a", "á", accent), Ordering::Less);
        assert_eq!(order("a", "A", accent), Ordering::Equal);
        assert_eq!(order("a", "á", case), Ordering::Equal);
        assert_eq!(order("a", "A", case), Ordering::Less);
        // Taken apart or not, it's the same text.
        assert_eq!(order("é", "e\u{301}", variant), Ordering::Equal);
        let upper = Options { upper_first: true, ..variant };
        assert_eq!(order("a", "A", upper), Ordering::Greater);
        let numeric = Options { numeric: true, ..variant };
        assert_eq!(order("file9", "file10", numeric), Ordering::Less);
        assert_eq!(order("file9", "file10", variant), Ordering::Greater);
        assert_eq!(order("a01", "a1", numeric), Ordering::Equal);
        assert_eq!(order("x\u{663}", "x3", numeric), Ordering::Equal);
        let quiet = Options { ignore_punctuation: true, ..variant };
        assert_eq!(order("a-b", "ab", quiet), Ordering::Equal);
        assert_eq!(order("a-b", "ab", variant), Ordering::Less);
    }

    #[test]
    fn reads_locale_extensions() {
        let options = extension_options("en-u-kn-kf-upper", Options::default());
        assert!(options.numeric && options.upper_first);
        let options = extension_options("de-u-co-phonebk-kn-false", Options::default());
        assert!(!options.numeric && !options.upper_first);
    }

    /// Sorting the kind of names a folder has, many of them, is quick: Node's ICU takes about 40 ms for these.
    #[test]
    fn sorts_many_names_quickly() {
        let names = many_names(25_000);
        let options = Options { numeric: true, ..Options::default() }.with_sensitivity("base").unwrap();
        let mut sorted: Vec<&Text> = names.iter().collect();
        let start = std::time::Instant::now();
        sorted.sort_by(|a, b| compare(a, b, options));
        let took = start.elapsed();
        eprintln!("sorting {} names took {took:?}", names.len());
        for pair in sorted.windows(2) {
            assert_ne!(compare(pair[0], pair[1], options), Ordering::Greater);
        }
        // Generous, for a debug build on a busy machine.
        assert!(took < std::time::Duration::from_secs(if cfg!(debug_assertions) { 20 } else { 2 }));
    }

    /// The shortcuts (skipping the start two strings have in common, and not taking text apart until a mark comes)
    /// give what comparing all of both, taken apart, does, for strings made to trip them up.
    #[test]
    fn shortcuts_change_nothing() {
        let pieces = [
            "a", "l", "L", "\u{b7}", "1", "0", "9", "\u{663}", "-", " ", "\u{301}", "\u{323}", "\u{302}", "\u{e9}",
            "\u{ea}", "\u{1ec7}", "\u{418}", "\u{306}", "\u{4e00}", "\u{ac00}", "\u{1f600}", "\u{f900}", "E", "z",
        ];
        let mut seed: u32 = 99;
        let mut random = |n: usize| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 8) as usize % n
        };
        let make = |random: &mut dyn FnMut(usize) -> usize| {
            (0..random(6)).map(|_| pieces[random(pieces.len())]).collect::<String>()
        };
        let sensitivities = ["base", "accent", "case", "variant"];
        for _ in 0..20_000 {
            let start = make(&mut random);
            let one = text(&(start.clone() + &make(&mut random)));
            let other = text(&(start + &make(&mut random)));
            let options = Options {
                numeric: random(2) == 0,
                upper_first: random(2) == 0,
                ignore_punctuation: random(4) == 0,
                ..Options::default()
            }
            .with_sensitivity(sensitivities[random(4)])
            .unwrap();
            let apart = |text: &Text| text.wide().and_then(|units| normalize(units, Form::Nfd));
            let (one_apart, other_apart) = (apart(&one), apart(&other));
            let a = one_apart.as_deref().map_or(source(&one), Source::Wide);
            let b = other_apart.as_deref().map_or(source(&other), Source::Wide);
            let slow = compare_sources(a, b, options, false).unwrap();
            assert_eq!(
                compare(&one, &other, options),
                slow,
                "{:?} vs {:?} with {options:?}",
                one.to_rust(),
                other.to_rust()
            );
        }
    }

    /// Names like a big folder's, mostly ASCII with accents, CJK and numbers in some.
    pub(crate) fn many_names(count: usize) -> Vec<Text> {
        let words = [
            "amd64_microsoft-windows-", "x86_", "wow64_", "Report", "photo", "IMG_", "résumé", "Café", "naïve", "Zürich",
            "東京", "日本語", "한국어", "カタカナ", "Москва", "notes", "Final", "backup", "readme", ".txt", ".png", ".dll",
            "_31bf3856ad364e35_", "10.0.26100.", "none_", " (copy)", "-", ".", "_",
        ];
        let mut seed: u32 = 7;
        let mut random = |n: usize| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 8) as usize % n
        };
        (0..count)
            .map(|index| {
                let mut name = String::new();
                for _ in 0..2 + random(5) {
                    name.push_str(words[random(words.len())]);
                    if random(3) == 0 {
                        name.push_str(&random(5000).to_string());
                    }
                }
                name.push_str(&index.to_string());
                text(&name)
            })
            .collect()
    }
}
