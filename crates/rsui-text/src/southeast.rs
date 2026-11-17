//! Shapes Khmer and Burmese, which work like Indic scripts but with rules of their own. Each writes some
//! vowels and consonants in a different order from how they're typed, within a syllable.

use crate::ot::{MASK_ABVF, MASK_ALL, MASK_BLWF, MASK_INIT, MASK_PREF, MASK_PSTF, Slot};
use crate::shape::Script;

// Kinds of character. These are only ever compared with each other within one script.
const X: u8 = 0;
const C: u8 = 1;
const V: u8 = 2;
const RA: u8 = 3;
const ZWNJ: u8 = 4;
const ZWJ: u8 = 5;
const PLACEHOLDER: u8 = 6;
const DOTTED_CIRCLE: u8 = 7;
const H: u8 = 8;
const VPRE: u8 = 9;
const VABV: u8 = 10;
const VBLW: u8 = 11;
const VPST: u8 = 12;
// Only in Khmer.
const ROBATIC: u8 = 13;
const XGROUP: u8 = 14;
const YGROUP: u8 = 15;
// Only in Burmese.
const AS: u8 = 16;
const MY: u8 = 17;
const MR: u8 = 18;
const MW: u8 = 19;
const MH: u8 = 20;
const A: u8 = 21;
const DB: u8 = 22;
const PT: u8 = 23;
const SM: u8 = 24;
const VS: u8 = 25;
const DIGIT: u8 = 26;
const PUNCTUATION: u8 = 27;

// Where something goes in a Burmese syllable, in the order they come.
const POS_PRE_M: u8 = 2;
const POS_PRE_C: u8 = 3;
const POS_BASE_C: u8 = 4;
const POS_AFTER_MAIN: u8 = 5;
const POS_BEFORE_SUB: u8 = 7;
const POS_BELOW_C: u8 = 8;
const POS_AFTER_SUB: u8 = 9;

/// Which of these a script is, if it's either.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Southeast {
    Khmer,
    Burmese,
}

impl Southeast {
    pub(crate) fn of(script: Script) -> Option<Self> {
        match script {
            Script::Khmer => Some(Self::Khmer),
            Script::Myanmar => Some(Self::Burmese),
            _ => None,
        }
    }

    /// The stages of features that go together, for both of these: what's done before anything's moved
    /// comes in the first two, and what's after [`first_stage_after`](Self::first_stage_after) is done
    /// once everything's in place.
    pub(crate) fn first_stage_after(self) -> usize {
        match self {
            Self::Khmer => 3,
            Self::Burmese => 6,
        }
    }

    pub(crate) const STAGES_BEFORE: usize = 2;
}

/// A mark on what comes after a certain kind of Coeng and Ro in Khmer, which fonts use to tell apart
/// two ways of stacking the same letters. It's only used here, so it can share a bit with Indic scripts.
pub(crate) const MASK_CFAR: u16 = MASK_INIT;

const fn is_joiner(category: u8) -> bool {
    matches!(category, ZWNJ | ZWJ)
}

fn khmer_category(code: u32) -> u8 {
    match code {
        0x179A => RA,
        0x1780..=0x17A2 => C,
        0x17A3..=0x17B3 => V,
        0x17B6 | 0x17C0 | 0x17C4 | 0x17C5 => VPST,
        0x17B7..=0x17BA | 0x17BE => VABV,
        0x17BB..=0x17BD | 0x17BF => VBLW,
        0x17C1..=0x17C3 => VPRE,
        0x17C6 | 0x17CB | 0x17CD..=0x17D1 => XGROUP,
        0x17C7 | 0x17C8 | 0x17D3 | 0x17DD => YGROUP,
        0x17C9 | 0x17CA | 0x17CC => ROBATIC,
        0x17D2 => H,
        0x200C => ZWNJ,
        0x200D => ZWJ,
        0x00A0 | 0x2010..=0x2015 | 0x2022 => PLACEHOLDER,
        0x25CC => DOTTED_CIRCLE,
        _ => X,
    }
}

fn burmese_category(code: u32) -> u8 {
    match code {
        0x1004 | 0x101B | 0x105A => RA,
        0x1000..=0x1021
        | 0x103F
        | 0x104E
        | 0x1050
        | 0x1051
        | 0x105B..=0x105D
        | 0x1061
        | 0x1065
        | 0x1066
        | 0x106E..=0x1070
        | 0x1075..=0x1081
        | 0x108E
        | 0xA9E0..=0xA9E4
        | 0xA9E7..=0xA9EF
        | 0xA9FA..=0xA9FE
        | 0xAA60..=0xAA76
        | 0xAA7A
        | 0xAA7E
        | 0xAA7F => C,
        0x1022..=0x102A | 0x1052..=0x1055 => V,
        0x102B | 0x102C | 0x1056 | 0x1057 | 0x1062 | 0x1067 | 0x1068 | 0x1083 | 0xA9E5 => VPST,
        0x102D | 0x102E | 0x1033..=0x1035 | 0x1071..=0x1074 | 0x1085 | 0x1086 | 0x109D => VABV,
        0x102F | 0x1030 | 0x1058 | 0x1059 => VBLW,
        0x1031 | 0x1084 => VPRE,
        0x1032 | 0x1036 => A,
        0x1037 => DB,
        0x1038 | 0x1087..=0x108D | 0x108F | 0x109A..=0x109C => SM,
        0x1039 => H,
        0x103A => AS,
        0x103B | 0x105E | 0x105F => MY,
        0x103C => MR,
        0x103D | 0x1082 => MW,
        0x103E | 0x1060 => MH,
        0x1040..=0x1049 | 0x1090..=0x1099 => DIGIT,
        0x104A | 0x104B => PUNCTUATION,
        0x1063 | 0x1064 | 0x1069..=0x106D | 0xAA7B => PT,
        0xFE00..=0xFE0F => VS,
        0x200C => ZWNJ,
        0x200D => ZWJ,
        0x002D | 0x00A0 | 0x00D7 | 0x2012..=0x2015 | 0x2022 | 0x25FB..=0x25FE => PLACEHOLDER,
        0x25CC => DOTTED_CIRCLE,
        _ => X,
    }
}

/// Says what kind of thing a character is, on its slot.
pub(crate) fn set_category(slot: &mut Slot, character: char, script: Southeast) {
    let code = character as u32;
    slot.category = match script {
        Southeast::Khmer => khmer_category(code),
        Southeast::Burmese => burmese_category(code),
    };
}

/// In Khmer, some vowels are written on both sides of a letter, and fonts want the side before it apart.
pub(crate) fn taken_apart(characters: &[(u32, char)], script: Southeast) -> Vec<(u32, char)> {
    let mut out = Vec::with_capacity(characters.len() + 4);
    for (byte, character) in characters {
        if script == Southeast::Khmer
            && matches!(*character as u32, 0x17BE..=0x17C0 | 0x17C4 | 0x17C5)
        {
            out.push((*byte, '\u{17C1}'));
        }
        out.push((*byte, *character));
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

    /// Goes over one if it's there.
    #[inline]
    fn maybe(&self, at: usize, category: u8) -> usize {
        at + usize::from(self.is(at, category))
    }

    /// Goes over as many as there are.
    #[inline]
    fn any(&self, mut at: usize, category: u8) -> usize {
        while self.is(at, category) {
            at += 1;
        }
        at
    }

    // Khmer.

    fn khmer_consonant(&self, at: usize) -> Option<usize> {
        if !matches!(self.at(at), C | RA | V) {
            return None;
        }
        let at = at + 1;
        if self.is(at, ROBATIC) {
            Some(at + 1)
        } else if is_joiner(self.at(at)) && self.is(at + 1, ROBATIC) {
            Some(at + 2)
        } else {
            Some(at)
        }
    }

    fn xgroup(&self, mut at: usize) -> usize {
        loop {
            let mut next = at;
            while is_joiner(self.at(next)) {
                next += 1;
            }
            if !self.is(next, XGROUP) {
                return at;
            }
            at = next + 1;
        }
    }

    fn khmer_tail(&self, at: usize) -> usize {
        let at = self.xgroup(at);
        let at = self.maybe(at, VPRE);
        let at = self.xgroup(at);
        let at = self.maybe(at, VBLW);
        let mut at = self.xgroup(at);
        if self.is(at, VABV) {
            at += 1;
        } else if is_joiner(self.at(at)) && self.is(at + 1, VABV) {
            at += 2;
        }
        let at = self.xgroup(at);
        let at = self.maybe(at, VPST);
        let mut at = self.xgroup(at);
        if self.is(at, H) && matches!(self.at(at + 1), C | RA | V) {
            at += 2;
        }
        self.any(at, YGROUP)
    }

    fn khmer_rest(&self, at: usize) -> usize {
        let mut at = self.maybe(at, ROBATIC);
        while self.is(at, H) {
            match self.khmer_consonant(at + 1) {
                Some(after) => at = after,
                None => return at + 1,
            }
        }
        self.khmer_tail(at)
    }

    /// Where the syllable from here ends, and whether it's missing a letter to go on.
    fn next_khmer(&self, start: usize) -> (usize, bool) {
        let base = if matches!(self.at(start), PLACEHOLDER | DOTTED_CIRCLE) {
            Some(start + 1)
        } else {
            self.khmer_consonant(start)
        };
        if let Some(at) = base {
            return (self.khmer_rest(at), false);
        }
        let end = self.khmer_rest(start);
        if end > start {
            (end, true)
        } else {
            (start + 1, false)
        }
    }

    // Burmese.

    fn medials(&self, at: usize) -> usize {
        let at = self.maybe(at, MY);
        let at = self.maybe(at, AS);
        let at = self.maybe(at, MR);
        if self.is(at, MW) {
            let at = self.maybe(at + 1, MH);
            self.maybe(at, AS)
        } else if self.is(at, MH) {
            self.maybe(at + 1, AS)
        } else {
            at
        }
    }

    fn dot_below(&self, at: usize) -> usize {
        if self.is(at, DB) {
            self.maybe(at + 1, AS)
        } else {
            at
        }
    }

    fn burmese_tail(&self, mut at: usize) -> usize {
        // More letters stacked below, each after a sign that says so.
        while self.is(at, H) && matches!(self.at(at + 1), C | RA | V) {
            at = self.maybe(at + 2, VS);
        }
        if self.is(at, H) {
            return at + 1;
        }
        let at = self.any(at, AS);
        let mut at = self.medials(at);
        while self.is(at, VPRE) {
            at = self.maybe(at + 1, VS);
        }
        let at = self.any(at, VABV);
        let at = self.any(at, VBLW);
        let at = self.any(at, A);
        let mut at = self.dot_below(at);
        while self.is(at, VPST) {
            let after = self.maybe(at + 1, MH);
            let after = self.any(after, AS);
            let after = self.any(after, VABV);
            let after = self.any(after, A);
            at = self.dot_below(after);
        }
        while self.is(at, PT) {
            let after = self.any(at + 1, A);
            let after = self.maybe(after, DB);
            at = self.maybe(after, AS);
        }
        let at = self.any(at, SM);
        if is_joiner(self.at(at)) { at + 1 } else { at }
    }

    /// Whether a letter stacked by the one before it starts here.
    fn stacks_on_ra(&self, at: usize) -> bool {
        self.is(at, RA) && self.is(at + 1, AS) && self.is(at + 2, H)
    }

    fn next_burmese(&self, start: usize) -> (usize, bool) {
        let is_base = |at: usize| {
            matches!(
                self.at(at),
                C | RA | V | DIGIT | PLACEHOLDER | DOTTED_CIRCLE
            )
        };
        let after_stack = if self.stacks_on_ra(start) {
            start + 3
        } else {
            start
        };
        // The longest of these wins, and the first of those that are the same.
        let whole = [after_stack, start]
            .iter()
            .find(|at| is_base(**at))
            .map(|at| self.burmese_tail(self.maybe(*at + 1, VS)));
        let broken = self.burmese_tail(self.maybe(after_stack, VS));
        if let Some(whole) = whole {
            if whole >= broken {
                return (whole, false);
            }
        }
        if is_joiner(self.at(start)) && broken <= start + 1 {
            return (start + 1, false);
        }
        if self.is(start, PUNCTUATION) {
            // Signs can come after punctuation just as they can after anything.
            let at = self.any(start + 1, A);
            let at = self.maybe(at, DB);
            let at = self.any(at, AS);
            return (self.any(at, SM), false);
        }
        if broken > start {
            (broken, true)
        } else {
            (start + 1, false)
        }
    }
}

/// Finds the syllables, numbering them from 1, and says which are marks with nothing to go on.
pub(crate) fn find_syllables(slots: &mut [Slot], script: Southeast) -> Vec<bool> {
    let categories: Vec<u8> = slots.iter().map(|slot| slot.category).collect();
    let finder = Finder {
        categories: &categories,
    };
    let mut broken = Vec::new();
    let mut start = 0;
    let mut number = 0u16;
    while start < slots.len() {
        let (end, is_broken) = match script {
            Southeast::Khmer => finder.next_khmer(start),
            Southeast::Burmese => finder.next_burmese(start),
        };
        number = number.wrapping_add(1).max(1);
        for slot in &mut slots[start..end] {
            slot.syllable = number;
        }
        broken.push(is_broken);
        start = end;
    }
    broken
}

/// Gives a dotted circle to marks with nothing to go on, once fonts have had a first look at the text.
pub(crate) fn insert_dotted_circles(slots: &mut Vec<Slot>, broken: &[bool], dotted_circle: u16) {
    if dotted_circle == 0 || !broken.contains(&true) {
        return;
    }
    let mut output = Vec::with_capacity(slots.len() + 2);
    let mut start = 0;
    for is_broken in broken {
        if start >= slots.len() {
            break;
        }
        let number = slots[start].syllable;
        let mut end = start + 1;
        while end < slots.len() && slots[end].syllable == number {
            end += 1;
        }
        if *is_broken {
            let mut circle = slots[start];
            circle.glyph = dotted_circle;
            circle.category = DOTTED_CIRCLE;
            circle.mask = MASK_ALL;
            circle.class = 0;
            output.push(circle);
        }
        output.extend_from_slice(&slots[start..end]);
        start = end;
    }
    *slots = output;
}

/// Calls this with the slots of each syllable.
fn each_syllable(slots: &mut [Slot], mut change: impl FnMut(&mut [Slot])) {
    let mut start = 0;
    while start < slots.len() {
        let number = slots[start].syllable;
        let mut end = start + 1;
        while end < slots.len() && slots[end].syllable == number {
            end += 1;
        }
        change(&mut slots[start..end]);
        start = end;
    }
}

/// Puts everything where fonts expect it in each syllable, before any of its forms are applied.
pub(crate) fn reorder(slots: &mut [Slot], script: Southeast) {
    match script {
        Southeast::Khmer => each_syllable(slots, reorder_khmer),
        Southeast::Burmese => each_syllable(slots, reorder_burmese),
    }
}

fn reorder_khmer(slots: &mut [Slot]) {
    let end = slots.len();
    for slot in slots.iter_mut().skip(1) {
        slot.mask |= MASK_BLWF | MASK_ABVF | MASK_PSTF;
    }
    let mut stacked = 0;
    for at in 1..end {
        if slots[at].category == H && stacked <= 2 && at + 1 < end {
            stacked += 1;
            if slots[at + 1].category == RA {
                // A Coeng and Ro go right to the start, and make a form that goes before the letter.
                slots[at].mask |= MASK_PREF;
                slots[at + 1].mask |= MASK_PREF;
                slots[..at + 2].rotate_right(2);
                for slot in &mut slots[at + 2..] {
                    slot.mask |= MASK_CFAR;
                }
                stacked = 2;
            }
        } else if slots[at].category == VPRE {
            // The part of a vowel that goes before everything.
            slots[..=at].rotate_right(1);
        }
    }
}

fn is_burmese_consonant(category: u8) -> bool {
    matches!(category, C | RA | V | PLACEHOLDER | DOTTED_CIRCLE)
}

fn reorder_burmese(slots: &mut [Slot]) {
    let end = slots.len();
    let has_stack =
        end >= 3 && slots[0].category == RA && slots[1].category == AS && slots[2].category == H;
    let limit = if has_stack { 3 } else { 0 };
    let base = (limit..end)
        .find(|at| is_burmese_consonant(slots[*at].category))
        .unwrap_or(if has_stack { 0 } else { limit });
    let mut at = 0;
    while at < limit {
        slots[at].place = POS_AFTER_MAIN;
        at += 1;
    }
    while at < base {
        slots[at].place = POS_PRE_C;
        at += 1;
    }
    if at < end {
        slots[at].place = POS_BASE_C;
        at += 1;
    }
    let mut place = POS_AFTER_MAIN;
    // Each of these goes where it goes by what's come before it in the syllable.
    while at < end {
        let category = slots[at].category;
        slots[at].place = if category == MR {
            POS_PRE_C
        } else if category == VPRE {
            POS_PRE_M
        } else if category == VS {
            slots[at - 1].place
        } else if place == POS_AFTER_MAIN && category == VBLW {
            place = POS_BELOW_C;
            place
        } else if place == POS_BELOW_C && category == A {
            POS_BEFORE_SUB
        } else if place == POS_BELOW_C && category == VBLW {
            place
        } else if place == POS_BELOW_C {
            place = POS_AFTER_SUB;
            place
        } else {
            place
        };
        at += 1;
    }
    slots.sort_by_key(|slot| slot.place);
    // Vowels that go first go in the order they were typed, each with what belongs to it.
    let first = slots.iter().position(|slot| slot.place == POS_PRE_M);
    let last = slots.iter().rposition(|slot| slot.place == POS_PRE_M);
    if let (Some(first), Some(last)) = (first, last) {
        if first < last {
            slots[first..=last].reverse();
            let mut from = first;
            for at in first..=last {
                if slots[at].category == VPRE {
                    slots[from..=at].reverse();
                    from = at + 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots_of(text: &str, script: Southeast) -> Vec<Slot> {
        let characters: Vec<(u32, char)> = text
            .char_indices()
            .map(|(byte, c)| (byte as u32, c))
            .collect();
        taken_apart(&characters, script)
            .iter()
            .map(|(byte, character)| {
                let mut slot = Slot::new(*character as u32 as u16, *byte, MASK_ALL, false);
                set_category(&mut slot, *character, script);
                slot
            })
            .collect()
    }

    fn order_after(text: &str, script: Southeast) -> Vec<u32> {
        let mut slots = slots_of(text, script);
        find_syllables(&mut slots, script);
        reorder(&mut slots, script);
        slots.iter().map(|slot| u32::from(slot.glyph)).collect()
    }

    fn syllables_of(text: &str, script: Southeast) -> usize {
        let mut slots = slots_of(text, script);
        find_syllables(&mut slots, script);
        let mut numbers: Vec<u16> = slots.iter().map(|slot| slot.syllable).collect();
        numbers.dedup();
        numbers.len()
    }

    #[test]
    fn a_khmer_vowel_goes_before_its_letter() {
        assert_eq!(
            order_after("\u{1780}\u{17C1}", Southeast::Khmer),
            [0x17C1, 0x1780]
        );
    }

    #[test]
    fn a_khmer_vowel_on_both_sides_is_taken_apart() {
        assert_eq!(
            order_after("\u{1780}\u{17C4}", Southeast::Khmer),
            [0x17C1, 0x1780, 0x17C4]
        );
    }

    #[test]
    fn a_coeng_and_ro_go_first() {
        assert_eq!(
            order_after("\u{1780}\u{17D2}\u{179A}", Southeast::Khmer),
            [0x17D2, 0x179A, 0x1780]
        );
    }

    #[test]
    fn a_burmese_vowel_and_medial_go_before_the_letter() {
        // Ka, Medial Ra, Sign E.
        assert_eq!(
            order_after("\u{1000}\u{103C}\u{1031}", Southeast::Burmese),
            [0x1031, 0x103C, 0x1000]
        );
    }

    #[test]
    fn a_burmese_kinzi_goes_after_the_letter() {
        // Nga, Asat, Virama, then Ga.
        assert_eq!(
            order_after("\u{1004}\u{103A}\u{1039}\u{1002}", Southeast::Burmese),
            [0x1002, 0x1004, 0x103A, 0x1039]
        );
    }

    #[test]
    fn syllables_end_where_they_should() {
        assert_eq!(syllables_of("\u{1780}\u{1781}", Southeast::Khmer), 2);
        assert_eq!(
            syllables_of("\u{1780}\u{17D2}\u{1781}", Southeast::Khmer),
            1
        );
        assert_eq!(syllables_of("\u{1000}\u{1001}", Southeast::Burmese), 2);
        assert_eq!(
            syllables_of("\u{1000}\u{1039}\u{1001}", Southeast::Burmese),
            1
        );
    }

    #[test]
    fn signs_with_nothing_to_go_on_get_a_circle_if_there_is_one() {
        let mut slots = slots_of("\u{17C1}", Southeast::Khmer);
        let broken = find_syllables(&mut slots, Southeast::Khmer);
        insert_dotted_circles(&mut slots, &broken, 77);
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].glyph, 77);
        let mut slots = slots_of("\u{17C1}", Southeast::Khmer);
        let broken = find_syllables(&mut slots, Southeast::Khmer);
        insert_dotted_circles(&mut slots, &broken, 0);
        assert_eq!(slots.len(), 1);
    }

    #[test]
    fn anything_at_all_comes_to_an_end() {
        for category in 0..30 {
            let categories = vec![category; 8];
            let finder = Finder {
                categories: &categories,
            };
            for start in 0..8 {
                assert!(finder.next_khmer(start).0 > start);
                assert!(finder.next_burmese(start).0 > start);
            }
        }
    }
}
