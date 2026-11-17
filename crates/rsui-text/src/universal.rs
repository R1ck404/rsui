//! Shapes the rarer scripts that write things out of order, like Balinese, Javanese and Sinhala. They share
//! one set of rules, the Universal Shaping Engine, which goes by what kind of thing each character is.

use crate::ot::{
    MASK_ALL, MASK_FINAL, MASK_HIDDEN, MASK_INITIAL, MASK_ISOLATED, MASK_MEDIAL, MASK_RPHF, Slot,
};
use crate::universal_tables::{
    B, CATEGORIES, CGJ, CMABV, CMBLW, CS, DECOMPOSITIONS, FABV, FBLW, FM, FPST, GB, H, HN, HVM, IS,
    MABV, MBLW, MPRE, MPST, N, R, SCRIPT_BLOCKS, SCRIPT_TAGS, SK, SMABV, SMBLW, SUB, VABV, VBLW,
    VMABV, VMBLW, VMPRE, VMPST, VPRE, VPST, VS, ZWNJ,
};

/// Which stage each part of the work goes up to. See `universal_plan`.
pub(crate) const STAGE_BEFORE: std::ops::Range<usize> = 0..2;
pub(crate) const STAGE_RPHF: usize = 2;
pub(crate) const STAGE_PREF: usize = 3;
pub(crate) const STAGE_FORMS: usize = 4;
pub(crate) const STAGE_AFTER: usize = 5;

/// What kind of thing a run of characters makes, which says what's done with it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    /// A letter with what goes with it, or ending in a sign that stacks it on what's next.
    Syllable,
    /// Signs with no letter to go on.
    Broken,
    /// Numbers joined up, which take part in joining but aren't reordered.
    Numbers,
    /// Something and marks on it, which doesn't join.
    Symbol,
    /// Anything else.
    Other,
}

/// Which of these scripts a character is in, if any, as a number for [`tags_of`].
pub(crate) fn script_of(code: u32) -> Option<u8> {
    let found = SCRIPT_BLOCKS.partition_point(|block| block.1 < code);
    match SCRIPT_BLOCKS.get(found) {
        Some(&(first, _, script)) if first <= code => Some(script),
        _ => None,
    }
}

/// What fonts call this script.
pub(crate) fn tags_of(script: u8) -> &'static [&'static [u8; 4]] {
    std::slice::from_ref(&SCRIPT_TAGS[script as usize])
}

/// What kind of thing this character is.
pub(crate) fn category_of(character: char) -> u8 {
    let code = character as u32;
    let found = CATEGORIES.partition_point(|range| range.1 < code);
    match CATEGORIES.get(found) {
        Some(&(first, _, category)) if first <= code => category,
        _ => 0,
    }
}

/// Puts letters back together with the sign they were split from, if the font has that.
pub(crate) fn put_back(
    characters: &[(u32, char)],
    has_glyph: impl Fn(char) -> bool,
    is_mark: impl Fn(char) -> bool,
) -> Vec<(u32, char)> {
    let mut out: Vec<(u32, char)> = Vec::with_capacity(characters.len());
    for (byte, character) in characters {
        if let Some((_, last)) = out.last_mut() {
            if !is_mark(*last) {
                let made = DECOMPOSITIONS
                    .iter()
                    .find(|entry| entry.1 == *last as u32 && entry.2 == *character as u32)
                    .and_then(|entry| char::from_u32(entry.0));
                if let Some(made) = made.filter(|made| has_glyph(*made)) {
                    *last = made;
                    continue;
                }
            }
        }
        out.push((*byte, *character));
    }
    out
}

/// Splits characters made of two others, as fonts for these scripts expect.
pub(crate) fn taken_apart(characters: &[(u32, char)]) -> Vec<(u32, char)> {
    fn push(byte: u32, character: char, out: &mut Vec<(u32, char)>) {
        let code = character as u32;
        if let Ok(found) = DECOMPOSITIONS.binary_search_by_key(&code, |entry| entry.0) {
            let (_, first, second) = DECOMPOSITIONS[found];
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

struct Finder<'a> {
    categories: &'a [u8],
}

impl Finder<'_> {
    #[inline]
    fn at(&self, at: usize) -> u8 {
        self.categories.get(at).copied().unwrap_or(u8::MAX)
    }

    #[inline]
    fn is(&self, at: usize, category: u8) -> bool {
        self.at(at) == category
    }

    #[inline]
    fn maybe(&self, at: usize, category: u8) -> usize {
        at + usize::from(self.is(at, category))
    }

    #[inline]
    fn any(&self, mut at: usize, category: u8) -> usize {
        while self.is(at, category) {
            at += 1;
        }
        at
    }

    fn start(&self, at: usize) -> Option<usize> {
        let at = if matches!(self.at(at), R | CS) {
            at + 1
        } else {
            at
        };
        if !matches!(self.at(at), B | GB) {
            return None;
        }
        Some(self.maybe(at + 1, VS))
    }

    fn consonant_marks(&self, at: usize) -> usize {
        self.any(self.any(at, CMABV), CMBLW)
    }

    fn consonant_modifiers(&self, at: usize) -> usize {
        let mut at = self.consonant_marks(at);
        loop {
            if matches!(self.at(at), H | HVM | IS | SK) && self.is(at + 1, B) {
                at = self.consonant_marks(at + 2);
            } else if self.is(at, SUB) {
                at = self.consonant_marks(at + 1);
            } else {
                return at;
            }
        }
    }

    fn middle(&self, at: usize) -> usize {
        let at = self.consonant_modifiers(at);
        // Medial consonants.
        let at = self.maybe(at, MPRE);
        let at = self.maybe(at, MABV);
        let at = self.maybe(at, MBLW);
        let at = self.maybe(at, MPST);
        // Dependent vowels, or a halant instead of them.
        let at = if self.is(at, H) {
            at + 1
        } else {
            let at = self.any(at, VPRE);
            let at = self.any(at, VABV);
            let at = self.any(at, VBLW);
            self.any(at, VPST)
        };
        // Vowel modifiers.
        let at = self.maybe(at, HVM);
        let at = self.any(at, VMPRE);
        let at = self.any(at, VMABV);
        let at = self.any(at, VMBLW);
        let mut at = self.any(at, VMPST);
        while self.is(at, SK) && self.is(at + 1, B) {
            at += 2;
        }
        at
    }

    fn tail(&self, at: usize) -> usize {
        let at = self.middle(at);
        // Final consonants.
        let at = self.any(at, FABV);
        let at = self.any(at, FBLW);
        let at = self.any(at, FPST);
        // Final modifiers. There's only the one kind in the data, so this covers them all.
        let at = self.any(at, FM);
        // A non-joiner can be the last thing in a syllable.
        self.maybe(at, ZWNJ)
    }

    fn after_stacker(&self, at: usize) -> Option<usize> {
        let at = self.consonant_modifiers(at);
        self.is(at, IS).then_some(at + 1)
    }

    fn after_sakot(&self, at: usize) -> Option<usize> {
        let at = self.middle(at);
        self.is(at, SK).then_some(at + 1)
    }

    fn numbers(&self, mut at: usize) -> usize {
        while self.is(at, HN) && self.is(at + 1, N) {
            at += 2;
        }
        at
    }

    fn symbol_tail(&self, at: usize) -> usize {
        if self.is(at, SMABV) {
            self.any(self.any(at, SMABV), SMBLW)
        } else {
            self.any(at, SMBLW)
        }
    }

    /// The next thing from here: where it ends, and what it is. The longest wins, and the first of those
    /// that are the same.
    fn next(&self, start: usize) -> (usize, Kind) {
        let mut best = (start + 1, Kind::Other);
        let mut consider = |end: usize, kind: Kind| {
            if end > best.0 || (end == best.0 && best.1 == Kind::Other) {
                best = (end, kind);
            }
        };
        if self.is(start, N) {
            let end = self.numbers(start + 1);
            consider(self.maybe(end, HN), Kind::Numbers);
        }
        if matches!(self.at(start), 0 | GB) {
            consider(self.symbol_tail(start + 1), Kind::Symbol);
        }
        if let Some(at) = self.start(start) {
            if let Some(end) = self.after_stacker(at) {
                consider(end, Kind::Syllable);
            }
            if let Some(end) = self.after_sakot(at) {
                consider(end, Kind::Syllable);
            }
            consider(self.tail(at), Kind::Syllable);
        }
        // Whatever could follow a letter, with no letter.
        let at = self.maybe(start, R);
        let mut broken = self.tail(at);
        if let Some(end) = self.after_stacker(at) {
            broken = broken.max(end);
        }
        if let Some(end) = self.after_sakot(at) {
            broken = broken.max(end);
        }
        broken = broken.max(self.symbol_tail(at));
        let joined = self.numbers(at);
        broken = broken.max(joined).max(self.maybe(joined, HN));
        if broken > start {
            consider(broken, Kind::Broken);
        }
        best
    }
}

/// Finds the syllables, numbering them from 1. A grapheme joiner, and a non-joiner before a mark, don't
/// get in the way of what's around them.
pub(crate) fn find_syllables(slots: &mut [Slot], is_mark: &[bool]) -> Vec<Kind> {
    // Which characters count, and where they are.
    let mut counted: Vec<usize> = Vec::with_capacity(slots.len());
    for (at, slot) in slots.iter().enumerate() {
        match slot.category {
            CGJ => {}
            ZWNJ => {
                let next = (at + 1..slots.len()).find(|next| slots[*next].category != CGJ);
                if next.is_none_or(|next| !is_mark[next]) {
                    counted.push(at);
                }
            }
            _ => counted.push(at),
        }
    }
    let categories: Vec<u8> = counted.iter().map(|at| slots[*at].category).collect();
    let finder = Finder {
        categories: &categories,
    };
    let mut kinds = Vec::new();
    let (mut start, mut number) = (0, 0u16);
    let mut from = 0;
    while start < categories.len() {
        let (end, kind) = finder.next(start);
        number = number.wrapping_add(1).max(1);
        // It takes in anything that didn't count, right up to the next one that does.
        let to = counted.get(end).copied().unwrap_or(slots.len());
        for slot in &mut slots[from..to] {
            slot.syllable = number;
        }
        kinds.push(kind);
        from = to;
        start = end;
    }
    if from < slots.len() {
        number = number.wrapping_add(1).max(1);
        for slot in &mut slots[from..] {
            slot.syllable = number;
        }
        kinds.push(Kind::Other);
    }
    kinds
}

/// Calls this with the slots of each syllable.
fn each_syllable(slots: &mut [Slot], mut change: impl FnMut(usize, &mut [Slot])) {
    let (mut start, mut index) = (0, 0);
    while start < slots.len() {
        let number = slots[start].syllable;
        let mut end = start + 1;
        while end < slots.len() && slots[end].syllable == number {
            end += 1;
        }
        change(index, &mut slots[start..end]);
        index += 1;
        start = end;
    }
}

/// Says which glyphs can become a reph, and how each syllable joins on to what's around it.
pub(crate) fn set_masks(slots: &mut [Slot], kinds: &[Kind]) {
    let mut last = 0u16;
    let mut last_range = 0..0;
    let mut offset = 0;
    let mut ranges: Vec<(std::ops::Range<usize>, Kind)> = Vec::with_capacity(kinds.len());
    each_syllable(slots, |index, syllable| {
        ranges.push((offset..offset + syllable.len(), kinds[index]));
        offset += syllable.len();
    });
    const FORMS: u16 = MASK_ISOLATED | MASK_FINAL | MASK_MEDIAL | MASK_INITIAL;
    for (range, kind) in ranges {
        if matches!(kind, Kind::Syllable | Kind::Broken) {
            let limit = if slots[range.start].category == R {
                1
            } else {
                range.len().min(3)
            };
            for slot in &mut slots[range.start..range.start + limit] {
                slot.mask |= MASK_RPHF;
            }
        }
        if matches!(kind, Kind::Symbol | Kind::Other) {
            last = 0;
            continue;
        }
        let joins = last == MASK_FINAL || last == MASK_ISOLATED;
        if joins {
            let form = if last == MASK_FINAL {
                MASK_MEDIAL
            } else {
                MASK_INITIAL
            };
            for slot in &mut slots[last_range.clone()] {
                slot.mask = (slot.mask & !FORMS) | form;
            }
        }
        last = if joins { MASK_FINAL } else { MASK_ISOLATED };
        for slot in &mut slots[range.clone()] {
            slot.mask = (slot.mask & !FORMS) | last;
        }
        last_range = range;
    }
}

/// Gives a dotted circle to marks with nothing to go on, after any reph, once fonts have had a first look.
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
            while at < end && slots[at].category == R {
                output.push(slots[at]);
                at += 1;
            }
            let mut circle = slots[start];
            circle.glyph = dotted_circle;
            circle.category = B;
            circle.mask = (slots[start].mask & !MASK_HIDDEN) | MASK_ALL;
            circle.class = 0;
            output.push(circle);
        }
        output.extend_from_slice(&slots[at..end]);
        start = end;
    }
    *slots = output;
}

/// Forgets what's been substituted so far, as what's next only wants to know what it changes itself.
pub(crate) fn forget_substituted(slots: &mut [Slot]) {
    for slot in slots.iter_mut() {
        slot.clear_substituted();
    }
}

/// Notes which glyphs did become a reph, as they can be anything by now.
pub(crate) fn record_reph(slots: &mut [Slot]) {
    each_syllable(slots, |_, syllable| {
        for slot in syllable
            .iter_mut()
            .take_while(|slot| slot.mask & MASK_RPHF != 0)
        {
            if slot.is_substituted() {
                slot.category = R;
                break;
            }
        }
    });
}

/// Notes which glyphs became a form that goes before the rest.
pub(crate) fn record_pre_base(slots: &mut [Slot]) {
    each_syllable(slots, |_, syllable| {
        if let Some(slot) = syllable.iter_mut().find(|slot| slot.is_substituted()) {
            slot.category = VPRE;
        }
    });
}

fn is_halant(slot: &Slot) -> bool {
    matches!(slot.category, H | HVM | IS) && !slot.is_ligated()
}

const fn is_post_base(category: u8) -> bool {
    matches!(
        category,
        FABV | FBLW
            | FPST
            | MABV
            | MBLW
            | MPST
            | MPRE
            | VABV
            | VBLW
            | VPST
            | VPRE
            | VMABV
            | VMBLW
            | VMPST
            | VMPRE
    )
}

/// Puts everything where fonts expect it in each syllable.
pub(crate) fn reorder(slots: &mut [Slot], kinds: &[Kind]) {
    each_syllable(slots, |index, syllable| {
        if matches!(
            kinds.get(index),
            Some(Kind::Syllable | Kind::Symbol | Kind::Broken)
        ) {
            reorder_syllable(syllable);
        }
    });
}

fn reorder_syllable(slots: &mut [Slot]) {
    let end = slots.len();
    // A reph goes towards the end, before the first thing that comes after the letter.
    if slots[0].category == R && end > 1 {
        for at in 1..end {
            let after = is_post_base(slots[at].category) || is_halant(&slots[at]);
            if after || at == end - 1 {
                let to = if after { at - 1 } else { at };
                slots[..=to].rotate_left(1);
                break;
            }
        }
    }
    // What goes first goes to the front, or after the last halant, of what's before it.
    let mut front = 0;
    for at in 0..end {
        if is_halant(&slots[at]) {
            front = at + 1;
        } else if matches!(slots[at].category, VPRE | VMPRE)
            && !slots[at].is_later_part()
            && front < at
        {
            slots[front..=at].rotate_right(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::universal_tables::O;

    fn categories_of(text: &str) -> Vec<u8> {
        text.chars().map(category_of).collect()
    }

    fn next_of(text: &str) -> (usize, Kind) {
        let categories = categories_of(text);
        Finder {
            categories: &categories,
        }
        .next(0)
    }

    #[test]
    fn characters_are_what_unicode_says() {
        assert_eq!(category_of('\u{1B15}'), B, "a Balinese consonant");
        assert_eq!(category_of('\u{1B44}'), H, "Balinese's sign for stacking");
        assert_eq!(category_of('\u{0DCA}'), HVM, "Sinhala's sign");
        assert_eq!(category_of('\u{A9BC}'), VABV, "a Javanese vowel");
        assert_eq!(category_of('a'), O);
    }

    #[test]
    fn scripts_are_found_by_their_blocks() {
        assert_eq!(tags_of(script_of(0x1B15).unwrap()), &[b"bali"]);
        assert_eq!(tags_of(script_of(0xA98F).unwrap()), &[b"java"]);
        assert_eq!(tags_of(script_of(0x0D9A).unwrap()), &[b"sinh"]);
        assert_eq!(script_of(0x0915), None);
        assert_eq!(script_of(0x41), None);
    }

    #[test]
    fn a_letter_and_a_sign_go_back_together_if_the_font_has_what_they_make() {
        // Balinese A and its length sign make Aa.
        let parts = [(0, '\u{1B05}'), (0, '\u{1B35}'), (3, '\u{1B15}')];
        let is_mark = |character: char| character == '\u{1B35}';
        let whole = put_back(&parts, |_| true, is_mark);
        assert_eq!(whole, [(0, '\u{1B06}'), (3, '\u{1B15}')]);
        assert_eq!(put_back(&parts, |_| false, is_mark), parts);
    }

    #[test]
    fn signs_dont_go_back_together_with_each_other() {
        // These make another sign, but that sign is meant to stay in two parts, for moving the first.
        let parts = [(0, '\u{1B3E}'), (0, '\u{1B35}')];
        assert_eq!(put_back(&parts, |_| true, |_| true), parts);
    }

    #[test]
    fn a_syllable_ends_where_it_should() {
        assert_eq!(next_of("\u{1B15}\u{1B16}"), (1, Kind::Syllable));
        assert_eq!(next_of("\u{1B15}\u{1B44}\u{1B16}"), (3, Kind::Syllable));
        assert_eq!(next_of("\u{1B38}"), (1, Kind::Broken));
    }

    #[test]
    fn anything_at_all_comes_to_an_end() {
        for category in 0..=40 {
            let categories = vec![category; 8];
            let finder = Finder {
                categories: &categories,
            };
            for start in 0..8 {
                assert!(finder.next(start).0 > start, "{category} at {start}");
            }
        }
    }
}
