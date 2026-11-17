//! Shapes Indic scripts. A syllable isn't drawn in the order it's written: a vowel can go before the
//! consonants it comes after, and a consonant can turn into a mark on another. The OpenType spec for these
//! scripts describes the steps, which depend on what the font can do.

use crate::indic_tables::{
    A, C, CM, CS, DOTTEDCIRCLE, H, INDIC_DECOMPOSITIONS, INDIC_RANGES, M, MPST, N, PLACEHOLDER, RA,
    REPHA, RS, SM, SYMBOL, V, VOWEL_RULES, X, ZWJ, ZWNJ,
};
use crate::ot::{
    MASK_ABVF, MASK_ALL, MASK_BLWF, MASK_HALF, MASK_INIT, MASK_PREF, MASK_PSTF, MASK_RPHF, Plan,
    Slot, Step, Tables, would_substitute,
};
use crate::shape::Script;

// Where something goes in a syllable, in the order they come.
const POS_START: u8 = 0;
const POS_RA_TO_BECOME_REPH: u8 = 1;
const POS_PRE_M: u8 = 2;
const POS_PRE_C: u8 = 3;
const POS_BASE_C: u8 = 4;
const POS_AFTER_MAIN: u8 = 5;
const POS_BEFORE_SUB: u8 = 7;
const POS_BELOW_C: u8 = 8;
const POS_AFTER_SUB: u8 = 9;
const POS_BEFORE_POST: u8 = 10;
const POS_POST_C: u8 = 11;
const POS_AFTER_POST: u8 = 12;
const POS_SMVD: u8 = 13;
const POS_END: u8 = 14;

/// How a script forms a reph, the shape a Ra takes on top of what follows it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RephMode {
    /// Ra and a halant make one.
    Implicit,
    /// Only if it's asked for, with a joiner after them.
    Explicit,
    /// It has a character of its own.
    Logical,
}

/// What makes each script different.
#[derive(Clone, Copy)]
pub(crate) struct Config {
    script: Script,
    virama: char,
    reph_position: u8,
    reph_mode: RephMode,
    /// Whether what's before the base can turn into forms that go below it.
    below_before_base: bool,
}

impl Config {
    pub(crate) fn script(&self) -> Script {
        self.script
    }

    pub(crate) fn virama(&self) -> char {
        self.virama
    }
}

/// The rules for a script, if it's one of these.
pub(crate) fn config_of(script: Script) -> Option<Config> {
    let make = |virama, reph_position, reph_mode, below_before_base| Config {
        script,
        virama,
        reph_position,
        reph_mode,
        below_before_base,
    };
    Some(match script {
        Script::Devanagari => make('\u{94D}', POS_BEFORE_POST, RephMode::Implicit, true),
        Script::Bengali => make('\u{9CD}', POS_AFTER_SUB, RephMode::Implicit, true),
        Script::Gurmukhi => make('\u{A4D}', POS_BEFORE_SUB, RephMode::Implicit, true),
        Script::Gujarati => make('\u{ACD}', POS_BEFORE_POST, RephMode::Implicit, true),
        Script::Oriya => make('\u{B4D}', POS_AFTER_MAIN, RephMode::Implicit, true),
        Script::Tamil => make('\u{BCD}', POS_AFTER_POST, RephMode::Implicit, true),
        Script::Telugu => make('\u{C4D}', POS_AFTER_POST, RephMode::Explicit, false),
        Script::Kannada => make('\u{CCD}', POS_AFTER_POST, RephMode::Implicit, false),
        Script::Malayalam => make('\u{D4D}', POS_AFTER_MAIN, RephMode::Logical, true),
        _ => return None,
    })
}

/// Looks up what a character's category is and which side it sits on.
fn properties_of(character: char) -> (u8, u8) {
    let code = character as u32;
    let found = INDIC_RANGES.partition_point(|range| range.1 < code);
    match INDIC_RANGES.get(found) {
        Some(&(first, _, category, side)) if first <= code => (category, side),
        _ => (X, 0),
    }
}

/// Where a matra goes, by which script it's in and which side it sits on.
fn matra_position(character: char, side: u8, script: Script) -> u8 {
    let code = character as u32;
    match side {
        1 => POS_PRE_M,
        2 => match script {
            Script::Devanagari => POS_AFTER_SUB,
            Script::Telugu => {
                if code <= 0x0C42 || code == 0x0C56 {
                    POS_BEFORE_SUB
                } else {
                    POS_AFTER_SUB
                }
            }
            Script::Kannada => {
                if !(0x0CC3..=0x0CD6).contains(&code) {
                    POS_BEFORE_SUB
                } else {
                    POS_AFTER_SUB
                }
            }
            _ => POS_AFTER_POST,
        },
        3 => match script {
            Script::Gurmukhi => POS_AFTER_POST,
            Script::Oriya => POS_AFTER_MAIN,
            Script::Telugu | Script::Kannada => POS_BEFORE_SUB,
            _ => POS_AFTER_SUB,
        },
        4 => match script {
            Script::Gurmukhi | Script::Gujarati | Script::Tamil | Script::Malayalam => {
                POS_AFTER_POST
            }
            Script::Telugu => {
                if code <= 0x0C42 || code == 0x0C56 {
                    POS_BEFORE_SUB
                } else {
                    POS_AFTER_SUB
                }
            }
            Script::Kannada => {
                if !(0x0CC3..=0x0CD6).contains(&code) {
                    POS_BEFORE_SUB
                } else {
                    POS_AFTER_SUB
                }
            }
            _ => POS_AFTER_SUB,
        },
        _ => POS_END,
    }
}

#[inline]
fn is_consonant(category: u8) -> bool {
    matches!(category, C | CS | RA | CM | V | PLACEHOLDER | DOTTEDCIRCLE)
}

#[inline]
fn is_joiner(category: u8) -> bool {
    matches!(category, ZWJ | ZWNJ)
}

/// Sets what's known about a character on its slot.
pub(crate) fn set_properties(slot: &mut Slot, character: char, script: Script) {
    let (category, side) = properties_of(character);
    let position = if is_consonant(category) {
        POS_BASE_C
    } else if matches!(category, M | MPST) {
        matra_position(character, side, script)
    } else if matches!(category, SM | A) {
        POS_SMVD
    } else {
        POS_END
    };
    slot.category = category;
    slot.place = if character == '\u{B01}' {
        POS_BEFORE_SUB
    } else {
        position
    };
}

/// Puts a dotted circle in front of what can't go on what's before it, like a sign for a vowel on a letter
/// that's already a different vowel, so that it shows as a mistake. `None` if there's nothing like that.
pub(crate) fn with_bad_vowels_shown(characters: &[(u32, char)]) -> Option<Vec<(u32, char)>> {
    let mut result: Option<Vec<(u32, char)>> = None;
    for (index, (byte, character)) in characters.iter().enumerate() {
        let before = |back: usize| index.checked_sub(back).map(|at| characters[at].1 as u32);
        let code = *character as u32;
        let shown = before(1).is_some_and(|first| {
            let at = VOWEL_RULES.partition_point(|rule| (rule.0, rule.1) < (first, code));
            VOWEL_RULES
                .get(at)
                .is_some_and(|rule| (rule.0, rule.1, rule.2) == (first, code, 0))
        }) || before(2).zip(before(1)).is_some_and(|(first, second)| {
            let at = VOWEL_RULES
                .partition_point(|rule| (rule.0, rule.1, rule.2) < (first, second, code));
            VOWEL_RULES.get(at) == Some(&(first, second, code))
        });
        if shown && result.is_none() {
            result = Some(characters[..index].to_vec());
        }
        if let Some(result) = &mut result {
            if shown {
                result.push((*byte, DOTTED_CIRCLE));
            }
            result.push((*byte, *character));
        }
    }
    result
}

/// Splits characters made of two others, which are put back together later only if that's a letter with
/// a dot, and the font has it.
pub(crate) fn decomposed(characters: &[(u32, char)]) -> Vec<(u32, char)> {
    fn push(byte: u32, character: char, out: &mut Vec<(u32, char)>) {
        let code = character as u32;
        if let Ok(found) = INDIC_DECOMPOSITIONS.binary_search_by_key(&code, |entry| entry.0) {
            let (_, first, second) = INDIC_DECOMPOSITIONS[found];
            if let (Some(first), Some(second)) = (char::from_u32(first), char::from_u32(second)) {
                push(byte, first, out);
                push(byte, second, out);
                return;
            }
        }
        out.push((byte, character));
    }
    let mut out = Vec::with_capacity(characters.len());
    for (byte, character) in characters {
        push(*byte, *character, &mut out);
    }
    out
}

/// Reads through one syllable's worth of categories, trying each thing that can be there.
struct Syllables<'a> {
    categories: &'a [u8],
}

/// What kind of thing a run of characters makes, which says what's done with it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    Consonant,
    Vowel,
    Standalone,
    Symbol,
    Broken,
    Other,
}

impl Syllables<'_> {
    #[inline]
    fn at(&self, at: usize) -> u8 {
        self.categories.get(at).copied().unwrap_or(u8::MAX)
    }

    fn is(&self, at: usize, category: u8) -> bool {
        self.at(at) == category
    }

    /// What can modify a consonant.
    fn modifiers(&self, mut at: usize) -> usize {
        // A register shifter, with a non-joiner before it if there's one.
        let plain = at;
        if self.is(at, ZWNJ) && self.is(at + 1, RS) {
            at += 2;
        } else if self.is(at, RS) {
            at += 1;
        } else {
            at = plain;
        }
        if self.is(at, N) {
            at += 1;
            if self.is(at, N) {
                at += 1;
            }
        }
        at
    }

    fn consonant_part(&self, mut at: usize) -> Option<usize> {
        if !matches!(self.at(at), C | RA) {
            return None;
        }
        at += 1;
        if self.is(at, ZWJ) {
            at += 1;
        }
        Some(self.modifiers(at))
    }

    fn halant_group(&self, mut at: usize) -> Option<usize> {
        if is_joiner(self.at(at)) && self.is(at + 1, H) {
            at += 1;
        }
        if !self.is(at, H) {
            return None;
        }
        at += 1;
        if self.is(at, ZWJ) {
            at += 1;
            if self.is(at, N) {
                at += 1;
            }
        }
        Some(at)
    }

    /// Up to four more consonants, each after a halant.
    fn more_consonants(&self, mut at: usize) -> usize {
        for _ in 0..4 {
            let Some(after) = self
                .halant_group(at)
                .and_then(|after| self.consonant_part(after))
            else {
                break;
            };
            at = after;
        }
        at
    }

    fn matra_group(&self, at: usize) -> Option<usize> {
        let mut at = at;
        while is_joiner(self.at(at)) {
            at += 1;
        }
        if self.is(at, M) || self.is(at, MPST) {
            at += 1;
        } else if self.is(at, SM) && self.is(at + 1, MPST) {
            at += 2;
        } else {
            return None;
        }
        if self.is(at, N) {
            at += 1;
        }
        if self.is(at, H) {
            at += 1;
        }
        Some(at)
    }

    /// What can come after the consonants: a medial, then a halant, or any number of matras.
    fn ending(&self, mut at: usize) -> usize {
        if self.is(at, CM) {
            at += 1;
        }
        // A halant on its own, with whatever joiner goes with it.
        if self.is(at, H) && self.is(at + 1, ZWNJ) {
            return at + 2;
        }
        if let Some(after) = self.halant_group(at) {
            return after;
        }
        while let Some(after) = self.matra_group(at) {
            at = after;
        }
        at
    }

    fn tail(&self, at: usize) -> usize {
        let mut at = at;
        let start = at;
        if is_joiner(self.at(at)) && self.is(at + 1, SM) {
            at += 1;
        }
        if self.is(at, SM) {
            at += 1;
            if self.is(at, SM) {
                at += 1;
            }
            if self.is(at, ZWNJ) {
                at += 1;
            }
        } else {
            at = start;
        }
        let mut accents = 0;
        while self.is(at, A) && accents < 3 {
            at += 1;
            accents += 1;
        }
        at
    }

    /// A Ra and a halant, or a repha of its own.
    fn reph(&self, at: usize) -> usize {
        if self.is(at, RA) && self.is(at + 1, H) {
            at + 2
        } else if self.is(at, REPHA) {
            at + 1
        } else {
            at
        }
    }

    fn consonant_syllable(&self, start: usize) -> Option<usize> {
        let at = if matches!(self.at(start), REPHA | CS) {
            start + 1
        } else {
            start
        };
        let at = self.consonant_part(at)?;
        let at = self.more_consonants(at);
        Some(self.tail(self.ending(at)))
    }

    fn vowel_syllable(&self, start: usize) -> Option<usize> {
        let at = self.reph(start);
        if !self.is(at, V) {
            return None;
        }
        let at = self.modifiers(at + 1);
        let rest = self.tail(self.ending(self.more_consonants(at)));
        // A joiner can also be all there is after it, and the longest wins.
        if self.is(at, ZWJ) {
            return Some(rest.max(at + 1));
        }
        Some(rest)
    }

    fn standalone(&self, start: usize) -> Option<usize> {
        let plain = if matches!(self.at(start), REPHA | CS) {
            start + 1
        } else {
            start
        };
        let at = if self.is(plain, PLACEHOLDER) {
            plain + 1
        } else {
            let after_reph = self.reph(start);
            if !self.is(after_reph, DOTTEDCIRCLE) {
                return None;
            }
            after_reph + 1
        };
        let at = self.modifiers(at);
        let at = self.more_consonants(at);
        Some(self.tail(self.ending(at)))
    }

    fn symbol(&self, start: usize) -> Option<usize> {
        if !self.is(start, SYMBOL) {
            return None;
        }
        let at = if self.is(start + 1, N) {
            start + 2
        } else {
            start + 1
        };
        Some(self.tail(at))
    }

    /// Marks with nothing to go on, which get a dotted circle to sit on, if they add up to something.
    fn broken(&self, start: usize) -> Option<usize> {
        let at = self.modifiers(self.reph(start));
        let at = self.tail(self.ending(self.more_consonants(at)));
        (at > start).then_some(at)
    }

    /// The next syllable from here: where it ends, and what it is.
    fn next(&self, start: usize) -> (usize, Kind) {
        let tries = [
            (self.consonant_syllable(start), Kind::Consonant),
            (self.vowel_syllable(start), Kind::Vowel),
            (self.standalone(start), Kind::Standalone),
            (self.symbol(start), Kind::Symbol),
            (self.broken(start), Kind::Broken),
        ];
        let mut best = (start + 1, Kind::Other);
        for (end, kind) in tries {
            // The longest wins, and the first of those that are the same.
            if let Some(end) = end {
                if end > best.0 || (end == best.0 && best.1 == Kind::Other) {
                    best = (end, kind);
                }
            }
        }
        best
    }
}

/// Finds the syllables, numbering them from 1. A number can wrap around in very long runs, which doesn't
/// matter, as all that counts is that neighbors differ.
pub(crate) fn find_syllables(slots: &mut [Slot]) -> Vec<Kind> {
    let categories: Vec<u8> = slots.iter().map(|slot| slot.category).collect();
    let finder = Syllables {
        categories: &categories,
    };
    let mut kinds = Vec::new();
    let mut start = 0;
    let mut number = 0u16;
    while start < slots.len() {
        let (end, kind) = finder.next(start);
        number = number.wrapping_add(1).max(1);
        for slot in &mut slots[start..end] {
            slot.syllable = number;
        }
        kinds.push(kind);
        start = end;
    }
    kinds
}

/// Gives a dotted circle to marks with nothing to go on, once fonts have had a first look at the text.
pub(crate) fn insert_dotted_circles(slots: &mut Vec<Slot>, kinds: &[Kind], dotted_circle: u16) {
    if dotted_circle == 0 || !kinds.contains(&Kind::Broken) {
        return;
    }
    let mut output = Vec::with_capacity(slots.len() + 2);
    let mut start = 0;
    for kind in kinds {
        if start >= slots.len() {
            break;
        }
        let number = slots[start].syllable;
        let mut end = start + 1;
        while end < slots.len() && slots[end].syllable == number {
            end += 1;
        }
        let mut at = start;
        if *kind == Kind::Broken {
            // It goes after any reph, for it to sit on what's left.
            while at < end && slots[at].category == REPHA {
                output.push(slots[at]);
                at += 1;
            }
            let mut circle = slots[start];
            circle.glyph = dotted_circle;
            circle.category = DOTTEDCIRCLE;
            circle.place = POS_BASE_C;
            circle.mask = MASK_ALL;
            circle.class = 0;
            output.push(circle);
        }
        output.extend_from_slice(&slots[at..end]);
        start = end;
    }
    *slots = output;
}

/// The features of a plan that tell where consonants go, with their steps.
pub(crate) struct Forms<'a> {
    pub(crate) tables: Tables<'a>,
    pub(crate) rphf: &'a [Step],
    pub(crate) pref: &'a [Step],
    pub(crate) blwf: &'a [Step],
    pub(crate) pstf: &'a [Step],
    pub(crate) vatu: &'a [Step],
    pub(crate) virama: u16,
    pub(crate) old_spec: bool,
    /// Whether a rule counts as making a form even if it depends on what's around the letter. Bengali's
    /// newer rules show it isn't for most, but Malayalam fonts count on it, in either way of being made.
    pub(crate) around_too: bool,
}

impl Forms<'_> {
    /// Where a consonant goes, by which forms the font gives it.
    fn consonant_position(&self, glyph: u16) -> u8 {
        if self.virama == 0 {
            return POS_BASE_C;
        }
        let around = [self.virama, glyph, self.virama];
        let forms = |steps: &[Step]| {
            would_substitute(self.tables, steps, &around[..2], self.around_too)
                || would_substitute(self.tables, steps, &around[1..], self.around_too)
        };
        if forms(self.blwf) || forms(self.vatu) {
            POS_BELOW_C
        } else if forms(self.pstf) || forms(self.pref) {
            POS_POST_C
        } else {
            POS_BASE_C
        }
    }
}

/// Decides for each consonant which form it's likely to take.
pub(crate) fn set_consonant_positions(slots: &mut [Slot], forms: &Forms<'_>) {
    let mut known: Vec<(u16, u8)> = Vec::new();
    for slot in slots.iter_mut() {
        if slot.place != POS_BASE_C {
            continue;
        }
        let position = match known.iter().find(|entry| entry.0 == slot.glyph) {
            Some(entry) => entry.1,
            None => {
                let position = forms.consonant_position(slot.glyph);
                known.push((slot.glyph, position));
                position
            }
        };
        slot.place = position;
    }
}

/// Puts everything in a syllable where the font expects it, before any of its forms are applied.
fn reorder_syllable(slots: &mut [Slot], config: &Config, forms: &Forms<'_>) {
    let end = slots.len();
    // In Kannada, a Ra, a halant and a joiner have to be taken the same as if the last two were swapped.
    if config.script == Script::Kannada
        && end >= 3
        && slots[0].category == RA
        && slots[1].category == H
        && slots[2].category == ZWJ
    {
        slots.swap(1, 2);
    }
    let mut base = end;
    let mut has_reph = false;
    let mut limit = 0;
    if !forms.rphf.is_empty()
        && end >= 3
        && ((config.reph_mode == RephMode::Implicit && !is_joiner(slots[2].category))
            || (config.reph_mode == RephMode::Explicit && slots[2].category == ZWJ))
    {
        let glyphs = [slots[0].glyph, slots[1].glyph, slots[2].glyph];
        if would_substitute(forms.tables, forms.rphf, &glyphs[..2], forms.around_too)
            || (config.reph_mode == RephMode::Explicit
                && would_substitute(forms.tables, forms.rphf, &glyphs, forms.around_too))
        {
            limit = 2;
            while limit < end && is_joiner(slots[limit].category) {
                limit += 1;
            }
            base = 0;
            has_reph = true;
        }
    } else if config.reph_mode == RephMode::Logical && slots[0].category == REPHA {
        limit = 1;
        while limit < end && is_joiner(slots[limit].category) {
            limit += 1;
        }
        base = 0;
        has_reph = true;
    }
    // From the end, the last consonant that doesn't have a form below or after the others.
    let mut at = end;
    let mut seen_below = false;
    while at > limit {
        at -= 1;
        if is_consonant(slots[at].category) {
            let place = slots[at].place;
            if place != POS_BELOW_C && (place != POS_POST_C || seen_below) {
                base = at;
                break;
            }
            if place == POS_BELOW_C {
                seen_below = true;
            }
            base = at;
        } else if at > 0 && slots[at].category == ZWJ && slots[at - 1].category == H {
            // A joiner after a halant asks for half a form, which ends the search.
            break;
        }
    }
    // With nothing else to sit on, a Ra doesn't become a reph.
    if has_reph && base == 0 && limit - base <= 2 {
        has_reph = false;
    }

    for slot in slots.iter_mut().take(base) {
        slot.place = slot.place.min(POS_PRE_C);
    }
    if base < end {
        slots[base].place = POS_BASE_C;
    }
    if has_reph {
        slots[0].place = POS_RA_TO_BECOME_REPH;
    }

    // Fonts in the older way want the first halant after the base to go after the last consonant.
    if forms.old_spec {
        let disallow_double = config.script == Script::Kannada;
        for at in base + 1..end {
            if slots[at].category == H {
                let mut to = end - 1;
                while to > at {
                    if is_consonant(slots[to].category)
                        || (disallow_double && slots[to].category == H)
                    {
                        break;
                    }
                    to -= 1;
                }
                if slots[to].category != H && to > at {
                    slots[at..=to].rotate_left(1);
                }
                break;
            }
        }
    }

    // What's attached to something goes where that goes.
    let mut last_place = POS_START;
    for at in 0..end {
        let category = slots[at].category;
        if matches!(category, ZWJ | ZWNJ | N | RS | CM | H) {
            slots[at].place = last_place;
            if category == H && slots[at].place == POS_PRE_M {
                // A halant doesn't move with a matra on the left.
                for before in (1..=at).rev() {
                    if slots[before - 1].place != POS_PRE_M {
                        slots[at].place = slots[before - 1].place;
                        break;
                    }
                }
            }
        } else if slots[at].place != POS_SMVD {
            if category == MPST && at > 0 && slots[at - 1].category == SM {
                slots[at - 1].place = slots[at].place;
            }
            last_place = slots[at].place;
        }
    }
    // Consonants after the base own what's before them, back to the last consonant or matra.
    let mut last = base;
    for at in base + 1..end {
        if is_consonant(slots[at].category) {
            for between in last + 1..at {
                if slots[between].place < POS_SMVD {
                    slots[between].place = slots[at].place;
                }
            }
            last = at;
        } else if matches!(slots[at].category, M | MPST) {
            last = at;
        }
    }

    slots.sort_by_key(|slot| slot.place);

    // Find the base again. Several matras on the left go in the reverse of their order.
    let (mut first_left, mut last_left) = (end, end);
    base = end;
    for (at, slot) in slots.iter().enumerate() {
        if slot.place == POS_BASE_C {
            base = at;
            break;
        } else if slot.place == POS_PRE_M {
            if first_left == end {
                first_left = at;
            }
            last_left = at;
        }
    }
    if first_left < last_left {
        slots[first_left..=last_left].reverse();
        let mut from = first_left;
        for at in first_left..=last_left {
            if matches!(slots[at].category, M | MPST) {
                slots[from..=at].reverse();
                from = at + 1;
            }
        }
    }

    // Which forms each can take.
    for slot in slots.iter_mut() {
        if slot.place != POS_RA_TO_BECOME_REPH {
            break;
        }
        slot.mask |= MASK_RPHF;
    }
    let mut before_base = MASK_HALF;
    if !forms.old_spec && config.below_before_base {
        before_base |= MASK_BLWF;
    }
    for slot in slots.iter_mut().take(base) {
        slot.mask |= before_base;
    }
    for slot in slots.iter_mut().skip(base + 1) {
        slot.mask |= MASK_BLWF | MASK_ABVF | MASK_PSTF;
    }
    // In older fonts for Devanagari, a Ra with a halant before the base goes below too, unless a joiner
    // after them asks for its other form.
    if forms.old_spec && config.script == Script::Devanagari {
        for at in 0..base.saturating_sub(1) {
            if slots[at].category == RA
                && slots[at + 1].category == H
                && (at + 2 == base || slots[at + 2].category != ZWJ)
            {
                slots[at].mask |= MASK_BLWF;
                slots[at + 1].mask |= MASK_BLWF;
            }
        }
    }
    if !forms.pref.is_empty() && base + 2 < end {
        for at in base + 1..end - 1 {
            let glyphs = [slots[at].glyph, slots[at + 1].glyph];
            if would_substitute(forms.tables, forms.pref, &glyphs, forms.around_too) {
                slots[at].mask |= MASK_PREF;
                slots[at + 1].mask |= MASK_PREF;
                break;
            }
        }
    }
    // A non-joiner stops half forms before it.
    for at in 1..end {
        if is_joiner(slots[at].category) {
            let non_joiner = slots[at].category == ZWNJ;
            let mut before = at;
            loop {
                before -= 1;
                if non_joiner {
                    slots[before].mask &= !MASK_HALF;
                }
                if before == 0 || is_consonant(slots[before].category) {
                    break;
                }
            }
        }
    }
}

/// Puts each syllable in the order the font expects, before its forms are applied.
pub(crate) fn reorder(slots: &mut [Slot], kinds: &[Kind], config: &Config, forms: &Forms<'_>) {
    let mut start = 0;
    let mut which = 0;
    while start < slots.len() {
        let number = slots[start].syllable;
        let mut end = start + 1;
        while end < slots.len() && slots[end].syllable == number {
            end += 1;
        }
        if matches!(
            kinds.get(which),
            Some(Kind::Consonant | Kind::Vowel | Kind::Standalone | Kind::Broken)
        ) {
            reorder_syllable(&mut slots[start..end], config, forms);
        }
        which += 1;
        start = end;
    }
}

/// Whether a glyph was made by joining others together, and wasn't then split.
#[inline]
fn ligated(slot: &Slot) -> bool {
    slot.is_ligated() && !slot.is_multiplied()
}

/// Whether a glyph is one of these. Something made from others isn't, whatever it started as.
#[inline]
fn is_one_of(slot: &Slot, categories: &[u8]) -> bool {
    !slot.is_ligated() && categories.contains(&slot.category)
}

#[inline]
fn is_halant(slot: &Slot) -> bool {
    is_one_of(slot, &[H])
}

/// Moves what depends on which forms were made, now that they have been.
fn finish_syllable(
    slots: &mut [Slot],
    previous_is_letter: bool,
    config: &Config,
    forms: &Forms<'_>,
) {
    let end = slots.len();
    // Having been through ligatures can lose that something was a halant.
    if forms.virama != 0 {
        for slot in slots.iter_mut() {
            if slot.glyph == forms.virama && slot.is_ligated() && slot.is_multiplied() {
                slot.category = H;
                slot.clear_ligated();
            }
        }
    }
    let mut try_pref = !forms.pref.is_empty();
    let mut base = 0;
    while base < end {
        if slots[base].place >= POS_BASE_C {
            if try_pref && base + 1 < end {
                for at in base + 1..end {
                    if slots[at].mask & MASK_PREF != 0 {
                        if !(slots[at].is_substituted() && ligated(&slots[at])) {
                            // It was a candidate, and didn't form anything, so the base is around here.
                            base = at;
                            while base < end && is_halant(&slots[base]) {
                                base += 1;
                            }
                            if base < end {
                                slots[base].place = POS_BASE_C;
                            }
                            try_pref = false;
                        }
                        break;
                    }
                }
                if base == end {
                    break;
                }
            }
            // In Malayalam, forms below that weren't made are bases of their own.
            if config.script == Script::Malayalam {
                let mut at = base + 1;
                while at < end {
                    while at < end && is_one_of(&slots[at], &[ZWJ, ZWNJ]) {
                        at += 1;
                    }
                    if at == end || !is_halant(&slots[at]) {
                        break;
                    }
                    at += 1;
                    while at < end && is_one_of(&slots[at], &[ZWJ, ZWNJ]) {
                        at += 1;
                    }
                    if at < end
                        && is_one_of(&slots[at], &[C, CS, RA, CM, V, PLACEHOLDER, DOTTEDCIRCLE])
                        && slots[at].place == POS_BELOW_C
                    {
                        base = at;
                        slots[base].place = POS_BASE_C;
                    }
                    at += 1;
                }
            }
            if base > 0 && base < end && slots[base].place > POS_BASE_C {
                base -= 1;
            }
            break;
        }
        base += 1;
    }
    if base == end && base > 0 && is_one_of(&slots[base - 1], &[ZWJ]) {
        base -= 1;
    }
    if base < end {
        while base > 0 && is_one_of(&slots[base], &[N, H]) {
            base -= 1;
        }
    }

    // Matras on the left go after whatever half forms were made.
    let no_half_forms = matches!(config.script, Script::Malayalam | Script::Tamil);
    if end > 1 && base > 0 {
        let mut new_place = if base == end {
            base.saturating_sub(2)
        } else {
            base - 1
        };
        if !no_half_forms {
            loop {
                while new_place > 0 && !is_one_of(&slots[new_place], &[M, MPST, H]) {
                    new_place -= 1;
                }
                if is_halant(&slots[new_place]) && slots[new_place].place != POS_PRE_M {
                    // A joiner after this halant means it isn't one that stands alone, so keep looking.
                    if new_place + 1 < end
                        && is_one_of(&slots[new_place + 1], &[ZWJ])
                        && new_place > 0
                    {
                        new_place -= 1;
                        continue;
                    }
                } else {
                    new_place = 0;
                }
                break;
            }
        }
        if new_place > 0 && slots[new_place].place != POS_PRE_M {
            let mut at = new_place;
            while at > 0 {
                if slots[at - 1].place == POS_PRE_M {
                    let old = at - 1;
                    if old < base && base <= new_place {
                        base -= 1;
                    }
                    slots[old..=new_place].rotate_left(1);
                    new_place = new_place.saturating_sub(1);
                }
                at -= 1;
            }
        }
    }

    // A reph goes where the script says, if it was made.
    if end > 1
        && slots[0].place == POS_RA_TO_BECOME_REPH
        && ((slots[0].category == REPHA) != ligated(&slots[0]))
    {
        let new_place = reph_target(slots, base, config);
        slots[0..=new_place].rotate_left(1);
        if base > 0 && base <= new_place {
            base -= 1;
        }
    }

    // A consonant that goes before the base, if it formed something.
    if try_pref && base + 1 < end {
        for at in base + 1..end {
            if slots[at].mask & MASK_PREF != 0 {
                if ligated(&slots[at]) {
                    let mut new_place = base;
                    if !no_half_forms {
                        while new_place > 0 && !is_one_of(&slots[new_place - 1], &[M, MPST, H]) {
                            new_place -= 1;
                        }
                    }
                    if new_place > 0
                        && is_halant(&slots[new_place - 1])
                        && new_place < end
                        && is_one_of(&slots[new_place], &[ZWJ, ZWNJ])
                    {
                        new_place += 1;
                    }
                    slots[new_place..=at].rotate_right(1);
                }
                break;
            }
        }
    }

    // A matra on the left at the start of a word can have a form for that.
    if slots[0].place == POS_PRE_M && !previous_is_letter {
        slots[0].mask |= MASK_INIT;
    }
}

/// Where a reph that's been made goes.
fn reph_target(slots: &[Slot], base: usize, config: &Config) -> usize {
    let end = slots.len();
    let after_first_halant = || -> Option<usize> {
        let mut at = 1;
        while at < base && !is_halant(&slots[at]) {
            at += 1;
        }
        if at < base && is_halant(&slots[at]) {
            if at + 1 < base && is_one_of(&slots[at + 1], &[ZWJ, ZWNJ]) {
                at += 1;
            }
            return Some(at);
        }
        None
    };
    if config.reph_position != POS_AFTER_POST {
        if let Some(found) = after_first_halant() {
            return found;
        }
        if config.reph_position == POS_AFTER_MAIN {
            let mut at = base;
            while at + 1 < end && slots[at + 1].place <= POS_AFTER_MAIN {
                at += 1;
            }
            if at < end {
                return at;
            }
        }
        if config.reph_position == POS_AFTER_SUB {
            let mut at = base;
            while at + 1 < end
                && !matches!(slots[at + 1].place, POS_POST_C | POS_AFTER_POST | POS_SMVD)
            {
                at += 1;
            }
            if at < end {
                return at;
            }
        }
    }
    if let Some(found) = after_first_halant() {
        return found;
    }
    // Otherwise it goes to the end, before any marks that go there.
    let mut at = end - 1;
    while at > 0 && slots[at].place == POS_SMVD {
        at -= 1;
    }
    if is_halant(&slots[at]) {
        // After a matra and a halant, it goes before the halant, to work with the matra.
        let mut between = base + 1;
        while between < at {
            if is_one_of(&slots[between], &[M, MPST]) {
                at -= 1;
            }
            between += 1;
        }
    }
    at
}

/// Moves things into their final places in each syllable, after its forms have been applied.
pub(crate) fn finish(slots: &mut [Slot], config: &Config, forms: &Forms<'_>) {
    let mut start = 0;
    while start < slots.len() {
        let number = slots[start].syllable;
        let mut end = start + 1;
        while end < slots.len() && slots[end].syllable == number {
            end += 1;
        }
        // A circle standing in for a missing letter doesn't count as one.
        let previous_is_letter =
            start > 0 && !matches!(slots[start - 1].category, X | DOTTEDCIRCLE);
        finish_syllable(&mut slots[start..end], previous_is_letter, config, forms);
        start = end;
    }
}

/// The glyph to use for a syllable with nothing to sit on, if the font has one.
pub(crate) const DOTTED_CIRCLE: char = '\u{25CC}';

/// The features that make up a plan, in order, and which masks they use. Those with `MASK_ALL` are for
/// every glyph.
pub(crate) fn plan_of(plan: &Plan) -> PlanStages<'_> {
    let stage = |index: usize| plan.substitutions.get(index).map_or(&[][..], Vec::as_slice);
    PlanStages {
        rphf: stage(STAGE_RPHF),
        pref: stage(STAGE_PREF),
        blwf: stage(STAGE_BLWF),
        pstf: stage(STAGE_PSTF),
        vatu: stage(STAGE_VATU),
    }
}

/// The steps for the features that decide where consonants go.
pub(crate) struct PlanStages<'a> {
    pub(crate) rphf: &'a [Step],
    pub(crate) pref: &'a [Step],
    pub(crate) blwf: &'a [Step],
    pub(crate) pstf: &'a [Step],
    pub(crate) vatu: &'a [Step],
}

// Which stage of a plan has which feature. See `plan_for`. Stages stay in place even if they're empty.
pub(crate) const STAGE_BEFORE: std::ops::Range<usize> = 0..2;
pub(crate) const STAGE_BASIC: std::ops::Range<usize> = 2..13;
const STAGE_RPHF: usize = 4;
const STAGE_PREF: usize = 6;
const STAGE_BLWF: usize = 7;
const STAGE_PSTF: usize = 10;
const STAGE_VATU: usize = 11;
pub(crate) const STAGE_AFTER: usize = 13;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telugus_length_mark_goes_before_what_goes_below() {
        // It's what HarfBuzz does, and it's the one mark past the others that does.
        assert_eq!(
            matra_position('\u{0C56}', 2, Script::Telugu),
            POS_BEFORE_SUB
        );
        assert_eq!(
            matra_position('\u{0C42}', 2, Script::Telugu),
            POS_BEFORE_SUB
        );
        assert_eq!(matra_position('\u{0C43}', 2, Script::Telugu), POS_AFTER_SUB);
    }
}
