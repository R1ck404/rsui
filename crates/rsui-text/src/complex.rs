//! Lays out text that needs shaping or reordering, and answers questions about the result, such
//! as where a caret goes. What comes out is remembered, as this is far more work than plain text.

use std::hash::BuildHasher;
use std::ops::Range;

use foldhash::fast::FixedState;

use crate::bidi::{self, Class};
use crate::dictionary::{needs_dictionary, word_starts};
use crate::engine::{Face, Line, PlacedGlyph, TextEngine, TextLayout};
use crate::indic;
use crate::lines::{is_space, next_measured_line};
use crate::ot::{
    MASK_ALL, MASK_HIDDEN, MASK_INVISIBLE, MASK_JOINER, MASK_NON_JOINER, Plan, Slot, Step,
    position, set_classes, substitute, substitute_stages,
};
use crate::shape::{
    Script, is_complex_character, is_invisible, is_mark, joining_masks, line_levels, normalized,
    resolve_scripts, runs_of,
};
use crate::southeast::{self, Southeast};
use crate::universal;

/// How many results can be remembered, before counting how big they are.
const CACHE_SLOTS: usize = 256;
/// How many of those each result can go in. With only one, two labels on screen that land on the same
/// slot would push each other out on every frame, and both be worked out again each time.
const CACHE_WAYS: usize = 4;
/// How much memory remembered results can use. One result alone can be over this, as it takes
/// a very long line to get there, and working one out again for every frame would be worse.
const CACHE_BYTES: usize = 512 * 1024;
/// Stands in for a width, for text in a text input. No limit is given by this exact number,
/// as it isn't a number.
const INPUT_LINE: u32 = u32::MAX;

/// How much work some text needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Complexity {
    /// One glyph per character, in order.
    Plain,
    /// In order, but the font replaces some glyphs. Line breaks can be worked out as if it were
    /// plain, and then only lines that show need shaping.
    Ligatures,
    /// It has to be reordered, or shaped to know how wide it is.
    Complex,
}

/// One unbreakable group of glyphs on a line, in the order they appear from left to right.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cluster {
    /// The text it came from, as byte positions.
    start: u32,
    end: u32,
    left: f32,
    right: f32,
    right_to_left: bool,
}

impl Cluster {
    /// Where a caret goes for this position inside this cluster. Inside a ligature, its width is
    /// shared evenly between the letters that made it.
    fn x_at(&self, text: &str, at: usize) -> f32 {
        let (start, end) = (self.start as usize, self.end as usize);
        let lead = if self.right_to_left {
            self.right
        } else {
            self.left
        };
        if at <= start {
            return lead;
        }
        let (mut before, mut total) = (0u32, 0u32);
        for (offset, character) in text[start..end].char_indices() {
            if !is_mark(character) && !is_invisible(character) {
                total += 1;
                if start + offset < at {
                    before += 1;
                }
            }
        }
        if total <= 1 || before >= total {
            return self.trailing(at >= end || total > 0);
        }
        let share = (self.right - self.left) * before as f32 / total as f32;
        if self.right_to_left {
            self.right - share
        } else {
            self.left + share
        }
    }

    fn trailing(&self, past: bool) -> f32 {
        if past == self.right_to_left {
            self.left
        } else {
            self.right
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ShapedKey {
    text: u64,
    length: usize,
    size: u32,
    primary: usize,
    width: u32,
    /// Which way its paragraphs go, as `TextStyle::bidi_key` says.
    bidi: u8,
}

#[derive(Debug, Clone, PartialEq)]
struct ShapedLine {
    start: u32,
    end: u32,
    width: f32,
    x: f32,
    baseline: f32,
    glyphs: Range<u32>,
}

/// Text that's been shaped and arranged.
#[derive(Debug)]
pub(crate) struct Shaped {
    key: ShapedKey,
    lines: Vec<ShapedLine>,
    glyphs: Vec<PlacedGlyph>,
    /// Only for one line of text in an input, the only text with carets.
    clusters: Vec<Cluster>,
    right_to_left: bool,
}

impl Shaped {
    fn bytes(&self) -> usize {
        size_of::<Self>()
            + self.lines.capacity() * size_of::<ShapedLine>()
            + self.glyphs.capacity() * size_of::<PlacedGlyph>()
            + self.clusters.capacity() * size_of::<Cluster>()
    }

    /// Where a caret goes for a byte position.
    fn caret_x(&self, text: &str, at: usize) -> f32 {
        if let Some(cluster) = self
            .clusters
            .iter()
            .find(|cluster| (cluster.start as usize) <= at && at < cluster.end as usize)
        {
            return cluster.x_at(text, at);
        }
        // Past all the text, which is the far side of its last letter.
        self.clusters
            .iter()
            .max_by_key(|cluster| cluster.end)
            .map_or(0.0, |last| last.trailing(true))
    }

    /// Everywhere a caret can stop, from left to right.
    fn stops(&self, text: &str) -> Vec<(usize, f32)> {
        let mut stops = Vec::with_capacity(self.clusters.len() + 1);
        let mut end = 0;
        for cluster in &self.clusters {
            let (start, cluster_end) = (cluster.start as usize, cluster.end as usize);
            end = end.max(cluster_end);
            for (offset, character) in text[start..cluster_end].char_indices() {
                // Letters stacked together in an Indic syllable can't be pointed at one by one, and
                // neither can the parts of an emoji.
                let in_syllable = matches!(character as u32,
                    0x0900..=0x0DFF | 0x2190..=0x2BFF | 0x1F000..=0x1FAFF);
                if offset == 0 || (!is_mark(character) && !is_invisible(character) && !in_syllable)
                {
                    stops.push((start + offset, cluster.x_at(text, start + offset)));
                }
            }
        }
        stops.push((end, self.caret_x(text, end)));
        stops.sort_unstable_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        stops
    }
}

/// Recent results.
#[derive(Default)]
pub(crate) struct ShapedCache {
    slots: Vec<Option<Box<Shaped>>>,
    bytes: usize,
    /// Which of a full set's slots to replace next.
    next_way: usize,
    /// Where budget eviction resumes, so one new label doesn't empty the cache.
    next_victim: usize,
}

impl ShapedCache {
    fn insert(&mut self, slot: usize, shaped: Box<Shaped>) {
        if let Some(old) = self.slots[slot].take() {
            self.bytes -= old.bytes();
        }
        let cost = shaped.bytes();
        // Evict only enough entries to make room. An oversized current result is
        // allowed on its own, as the caller needs it to finish the current layout.
        while self.bytes > 0 && self.bytes.saturating_add(cost) > CACHE_BYTES {
            let victim = self.next_victim;
            self.next_victim = (victim + 1) % self.slots.len();
            if let Some(old) = self.slots[victim].take() {
                self.bytes -= old.bytes();
            }
        }
        self.bytes += cost;
        self.slots[slot] = Some(shaped);
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn result(bytes: usize) -> Box<Shaped> {
        Box::new(Shaped {
            key: ShapedKey {
                text: 0,
                length: 0,
                size: 0,
                primary: 0,
                width: 0,
                bidi: 0,
            },
            lines: Vec::new(),
            glyphs: Vec::with_capacity(bytes.div_ceil(size_of::<PlacedGlyph>())),
            clusters: Vec::new(),
            right_to_left: false,
        })
    }

    fn cache() -> ShapedCache {
        let mut cache = ShapedCache::default();
        cache.slots.resize_with(CACHE_SLOTS, || None);
        cache
    }

    #[test]
    fn budget_pressure_preserves_unrelated_shaped_results() {
        let mut cache = cache();
        for slot in 0..12 {
            cache.insert(slot, result(CACHE_BYTES / 8));
            assert!(cache.bytes <= CACHE_BYTES);
            assert_eq!(
                cache.bytes,
                cache
                    .slots
                    .iter()
                    .flatten()
                    .map(|entry| entry.bytes())
                    .sum::<usize>()
            );
        }
        assert!(
            cache.slots.iter().flatten().count() >= 6,
            "one new result must not flush every previously shaped label"
        );
    }

    #[test]
    fn oversized_result_does_not_keep_other_results_over_budget() {
        let mut cache = cache();
        cache.insert(0, result(100));
        cache.insert(1, result(CACHE_BYTES * 2));
        assert_eq!(cache.slots.iter().flatten().count(), 1);
        assert!(cache.slots[1].is_some());
        cache.insert(2, result(100));
        assert!(cache.bytes <= CACHE_BYTES);
        assert!(cache.slots[1].is_none());
        assert!(cache.slots[2].is_some());
    }
}

/// One paragraph, with what's known about each of its characters.
struct Paragraph {
    /// Each character and its byte position in the text, except for `\r`, which isn't shown.
    characters: Vec<(u32, char)>,
    levels: Vec<u8>,
    classes: Vec<Class>,
    scripts: Vec<Script>,
    faces: Vec<u16>,
    level: u8,
}

/// Where glyphs and clusters for a line go.
struct Output<'a> {
    glyphs: &'a mut Vec<PlacedGlyph>,
    clusters: Option<&'a mut Vec<Cluster>>,
}

/// Which characters in a run begin something a caret can stop at.
fn cluster_start(characters: &[(u32, char)], byte: u32) -> u32 {
    let found = characters.partition_point(|(at, _)| *at <= byte).max(1);
    characters[..found]
        .iter()
        .rposition(|(_, character)| !is_mark(*character) && !is_invisible(*character))
        .map_or(characters[0].0, |at| characters[at].0)
}

fn end_of(characters: &[(u32, char)]) -> u32 {
    characters
        .last()
        .map_or(0, |(byte, character)| byte + character.len_utf8() as u32)
}

/// Shapes characters that share a font, a script and a direction, in the order they appear in the
/// text. The result comes out in the same order.
fn shape_run(
    face: &Face,
    plan: &Plan,
    script: Script,
    right_to_left: bool,
    characters: &[(u32, char)],
    slots: &mut Vec<Slot>,
) {
    slots.clear();
    // A font that doesn't say it knows about a script gets no rules that are for fonts that do.
    if plan.knows_script {
        if let Some(config) = indic::config_of(script) {
            shape_indic_run(face, plan, &config, characters, slots);
            return;
        }
        if matches!(script, Script::Universal(_)) {
            shape_universal_run(face, plan, characters, slots);
            return;
        }
    }
    if let Some(kind) = Southeast::of(script) {
        if kind == Southeast::Khmer || (plan.knows_script && !plan.older_way) {
            shape_southeast_run(face, plan, kind, characters, slots);
            return;
        }
    }
    let tables = face.font.shaping_tables();
    let thai = sara_am_taken_apart(characters);
    let characters = thai.as_deref().unwrap_or(characters);
    let characters = &normalized(characters, |made| face.glyph(made) != 0)[..];
    let masks = script.joins().then(|| joining_masks(characters));
    let mut skip_selector = false;
    for (index, (byte, character)) in characters.iter().enumerate() {
        let glyph = if right_to_left {
            // A font might not have the mirror image of something that it does have.
            bidi::mirror_of(*character)
                .map(|mirror| face.glyph(mirror))
                .filter(|glyph| *glyph != 0)
                .unwrap_or_else(|| face.glyph(*character))
        } else {
            face.glyph(*character)
        };
        if skip_selector {
            skip_selector = false;
            continue;
        }
        let variant = variant_at(face, characters, index);
        skip_selector = variant.is_some();
        let glyph = variant.unwrap_or(glyph);
        let mut mask: u16 = masks.as_ref().map_or(MASK_ALL, |masks| masks[index]);
        if *character == '\u{200C}' {
            // This blocks ligatures, so lookups have to see it.
            mask |= MASK_NON_JOINER;
        } else if *character == '\u{200D}' && script.joins() {
            // Here a joiner does too: it says to join these letters, rather than to replace them
            // with one.
            mask |= MASK_JOINER;
        } else if is_invisible(*character) {
            mask |= MASK_INVISIBLE;
        }
        slots.push(Slot::new(glyph, *byte, mask, is_mark(*character)));
    }
    substitute(tables, plan, slots);
    for slot in slots.iter_mut() {
        slot.x_advance = if slot.mask & MASK_HIDDEN != 0 {
            0
        } else {
            i32::from(face.font.advance(slot.glyph))
        };
    }
    position(tables, plan, slots, right_to_left, true);
    if !plan.has_kerning {
        kern_the_old_way(face, slots, right_to_left);
    }
}

/// Shapes characters of an Indic script: finds their syllables, puts each in the order the font wants,
/// applies the forms it has, and puts what depends on those where it finally goes.
fn shape_indic_run(
    face: &Face,
    plan: &Plan,
    config: &indic::Config,
    characters: &[(u32, char)],
    slots: &mut Vec<Slot>,
) {
    let tables = face.font.shaping_tables();
    // Letters with a dot only come back together if there are marks around at all, which draws the same
    // either way, and this way matches what other programs do, which fonts are tested with.
    let any_marks = characters
        .iter()
        .skip(1)
        .any(|(_, character)| is_mark(*character));
    let shown = (face.glyph(indic::DOTTED_CIRCLE) != 0)
        .then(|| indic::with_bad_vowels_shown(characters))
        .flatten();
    let characters = shown.as_deref().unwrap_or(characters);
    let characters = indic::decomposed(characters);
    let characters = &normalized(&characters, |made| any_marks && face.glyph(made) != 0)[..];
    let mut skip_selector = false;
    for (index, (byte, character)) in characters.iter().enumerate() {
        if skip_selector {
            skip_selector = false;
            continue;
        }
        let mut mask = MASK_ALL;
        if *character == '\u{200C}' {
            mask |= MASK_NON_JOINER;
        } else if *character == '\u{200D}' {
            mask |= MASK_JOINER;
        } else if is_invisible(*character) {
            mask |= MASK_INVISIBLE;
        }
        let variant = variant_at(face, characters, index);
        skip_selector = variant.is_some();
        let glyph = variant.unwrap_or_else(|| face.glyph(*character));
        let mut slot = Slot::new(glyph, *byte, mask, is_mark(*character));
        indic::set_properties(&mut slot, *character, config_script(config));
        slots.push(slot);
    }
    set_classes(tables, slots);
    let kinds = indic::find_syllables(slots);
    let stage =
        |index: usize| -> &[Step] { plan.substitutions.get(index).map_or(&[], Vec::as_slice) };
    let mut next_ligature = 1;
    let before: Vec<&[Step]> = indic::STAGE_BEFORE.map(stage).collect();
    substitute_stages(tables, &before, slots, true, &mut next_ligature);
    indic::insert_dotted_circles(slots, &kinds, face.glyph(indic::DOTTED_CIRCLE));
    let stages = indic::plan_of(plan);
    let forms = indic::Forms {
        tables,
        rphf: stages.rphf,
        pref: stages.pref,
        blwf: stages.blwf,
        pstf: stages.pstf,
        vatu: stages.vatu,
        virama: face.glyph(config_virama(config)),
        old_spec: plan.older_way,
        around_too: plan.older_way || config.script() == Script::Malayalam,
    };
    indic::set_consonant_positions(slots, &forms);
    indic::reorder(slots, &kinds, config, &forms);
    // Each of these sees what the one before made.
    for index in indic::STAGE_BASIC {
        substitute_stages(tables, &[stage(index)], slots, true, &mut next_ligature);
    }
    indic::finish(slots, config, &forms);
    let after: Vec<&[Step]> = (indic::STAGE_AFTER..plan.substitutions.len())
        .map(stage)
        .collect();
    substitute_stages(tables, &after, slots, true, &mut next_ligature);
    finish_syllables(face, plan, slots, false);
}

/// Shapes Khmer or Burmese.
fn shape_southeast_run(
    face: &Face,
    plan: &Plan,
    kind: Southeast,
    characters: &[(u32, char)],
    slots: &mut Vec<Slot>,
) {
    let tables = face.font.shaping_tables();
    let characters = southeast::taken_apart(characters, kind);
    let characters = &normalized(&characters, |made| face.glyph(made) != 0)[..];
    let mut skip_selector = false;
    for (index, (byte, character)) in characters.iter().enumerate() {
        if skip_selector {
            skip_selector = false;
            continue;
        }
        let mut mask = MASK_ALL;
        if *character == '\u{200C}' {
            mask |= MASK_NON_JOINER;
        } else if *character == '\u{200D}' {
            mask |= MASK_JOINER;
        } else if is_invisible(*character) {
            mask |= MASK_INVISIBLE;
        }
        let variant = variant_at(face, characters, index);
        skip_selector = variant.is_some();
        let glyph = variant.unwrap_or_else(|| face.glyph(*character));
        let mut slot = Slot::new(glyph, *byte, mask, is_mark(*character));
        southeast::set_category(&mut slot, *character, kind);
        slots.push(slot);
    }
    set_classes(tables, slots);
    let broken = southeast::find_syllables(slots, kind);
    let stage =
        |index: usize| -> &[Step] { plan.substitutions.get(index).map_or(&[], Vec::as_slice) };
    let mut next_ligature = 1;
    let before: Vec<&[Step]> = (0..Southeast::STAGES_BEFORE).map(stage).collect();
    substitute_stages(tables, &before, slots, true, &mut next_ligature);
    southeast::insert_dotted_circles(slots, &broken, face.glyph(indic::DOTTED_CIRCLE));
    southeast::reorder(slots, kind);
    for index in Southeast::STAGES_BEFORE..kind.first_stage_after() {
        substitute_stages(tables, &[stage(index)], slots, true, &mut next_ligature);
    }
    let after: Vec<&[Step]> = (kind.first_stage_after()..plan.substitutions.len())
        .map(stage)
        .collect();
    substitute_stages(tables, &after, slots, true, &mut next_ligature);
    finish_syllables(face, plan, slots, kind == Southeast::Burmese);
}

/// Shapes one of the rarer scripts that share the same rules.
fn shape_universal_run(
    face: &Face,
    plan: &Plan,
    characters: &[(u32, char)],
    slots: &mut Vec<Slot>,
) {
    let tables = face.font.shaping_tables();
    let any_marks = characters
        .iter()
        .skip(1)
        .any(|(_, character)| is_mark(*character));
    let shown = (face.glyph(indic::DOTTED_CIRCLE) != 0)
        .then(|| indic::with_bad_vowels_shown(characters))
        .flatten();
    let characters = shown.as_deref().unwrap_or(characters);
    let characters = universal::taken_apart(characters);
    let characters = &normalized(&characters, |_| false)[..];
    let characters = &universal::put_back(
        characters,
        |made| any_marks && face.glyph(made) != 0,
        is_mark,
    )[..];
    let mut marks = Vec::with_capacity(characters.len());
    let mut skip_selector = false;
    for (index, (byte, character)) in characters.iter().enumerate() {
        if skip_selector {
            skip_selector = false;
            continue;
        }
        let mut mask = MASK_ALL;
        if *character == '\u{200C}' {
            mask |= MASK_NON_JOINER;
        } else if *character == '\u{200D}' {
            mask |= MASK_JOINER;
        } else if is_invisible(*character) {
            mask |= MASK_INVISIBLE;
        }
        let variant = variant_at(face, characters, index);
        skip_selector = variant.is_some();
        let glyph = variant.unwrap_or_else(|| face.glyph(*character));
        let mark = is_mark(*character);
        let mut slot = Slot::new(glyph, *byte, mask, mark);
        slot.category = universal::category_of(*character);
        marks.push(mark);
        slots.push(slot);
    }
    set_classes(tables, slots);
    let kinds = universal::find_syllables(slots, &marks);
    universal::set_masks(slots, &kinds);
    let stage =
        |index: usize| -> &[Step] { plan.substitutions.get(index).map_or(&[], Vec::as_slice) };
    let mut next_ligature = 1;
    let before: Vec<&[Step]> = universal::STAGE_BEFORE.map(stage).collect();
    substitute_stages(tables, &before, slots, true, &mut next_ligature);
    universal::forget_substituted(slots);
    substitute_stages(
        tables,
        &[stage(universal::STAGE_RPHF)],
        slots,
        true,
        &mut next_ligature,
    );
    universal::record_reph(slots);
    universal::forget_substituted(slots);
    substitute_stages(
        tables,
        &[stage(universal::STAGE_PREF)],
        slots,
        true,
        &mut next_ligature,
    );
    universal::record_pre_base(slots);
    substitute_stages(
        tables,
        &[stage(universal::STAGE_FORMS)],
        slots,
        true,
        &mut next_ligature,
    );
    universal::insert_dotted_circles(slots, &kinds, face.glyph(indic::DOTTED_CIRCLE));
    universal::reorder(slots, &kinds);
    let after: Vec<&[Step]> = (universal::STAGE_AFTER..plan.substitutions.len())
        .map(stage)
        .collect();
    substitute_stages(tables, &after, slots, false, &mut next_ligature);
    finish_syllables(face, plan, slots, true);
}

/// Treats each syllable as one thing from here on, and works out where everything goes.
fn finish_syllables(face: &Face, plan: &Plan, slots: &mut Vec<Slot>, marks_take_no_room: bool) {
    let tables = face.font.shaping_tables();
    // A syllable is one thing to a caret, however its parts have moved around.
    let mut start = 0;
    while start < slots.len() {
        let number = slots[start].syllable;
        let mut end = start + 1;
        while end < slots.len() && slots[end].syllable == number {
            end += 1;
        }
        let first = slots[start..end]
            .iter()
            .map(|slot| slot.cluster)
            .min()
            .unwrap_or(0);
        for slot in &mut slots[start..end] {
            slot.cluster = first;
        }
        start = end;
    }
    for slot in slots.iter_mut() {
        slot.x_advance = if slot.mask & MASK_HIDDEN != 0 {
            0
        } else {
            i32::from(face.font.advance(slot.glyph))
        };
    }
    position(tables, plan, slots, false, marks_take_no_room);
    if !plan.has_kerning {
        kern_the_old_way(face, slots, false);
    }
}

/// The special glyph for this character, if a variation selector after it asks for one the font has. The
/// selector has then done its job, and goes.
#[inline]
fn variant_at(face: &Face, characters: &[(u32, char)], index: usize) -> Option<u16> {
    if !face.font.has_variations() {
        return None;
    }
    let (_, selector) = characters.get(index + 1)?;
    if !matches!(*selector as u32, 0xFE00..=0xFE0F | 0xE0100..=0xE01EF) {
        return None;
    }
    face.font.variation_glyph(characters[index].1, *selector)
}

/// In Thai and Lao, the vowel Sara Am is drawn as a small circle over the letter, and a Sara Aa after it.
/// The circle goes under any tone mark, which comes first in the text. It's the way fonts expect it.
fn sara_am_taken_apart(characters: &[(u32, char)]) -> Option<Vec<(u32, char)>> {
    let is_sara_am = |character: char| matches!(character, '\u{0E33}' | '\u{0EB3}');
    if !characters
        .iter()
        .any(|(_, character)| is_sara_am(*character))
    {
        return None;
    }
    let goes_above = |character: char| {
        matches!(character as u32 & !0x80,
            0x0E31 | 0x0E34..=0x0E37 | 0x0E3B | 0x0E47..=0x0E4E)
    };
    let mut result = Vec::with_capacity(characters.len() + 4);
    for (byte, character) in characters {
        if !is_sara_am(*character) {
            result.push((*byte, *character));
            continue;
        }
        let offset = *character as u32 - 0x0E33;
        let (circle, after) = (0x0E4D + offset, 0x0E32 + offset);
        let mut at = result.len();
        while at > 0 && goes_above(result[at - 1].1) {
            at -= 1;
        }
        result.insert(at, (*byte, char::from_u32(circle).unwrap_or(*character)));
        result.push((*byte, char::from_u32(after).unwrap_or(*character)));
    }
    Some(result)
}

fn config_script(config: &indic::Config) -> Script {
    config.script()
}

fn config_virama(config: &indic::Config) -> char {
    config.virama()
}

/// Where a cluster's characters end. Glyphs can be in another order from their characters, but a cluster's
/// width still has to go somewhere.
fn characters_in_cluster(from: usize, to: usize, end: usize) -> usize {
    to.max(from + 1).min(end)
}

#[cfg(test)]
mod cluster_tests {
    use super::characters_in_cluster;

    #[test]
    fn a_cluster_always_has_a_character_to_give_its_width_to() {
        assert_eq!(characters_in_cluster(2, 5, 9), 5);
        // What's next came before it in the text.
        assert_eq!(characters_in_cluster(4, 2, 9), 5);
        assert_eq!(characters_in_cluster(4, 4, 9), 5);
        assert_eq!(characters_in_cluster(8, 3, 9), 9);
    }
}

/// For fonts that keep their kerning somewhere older than GPOS.
fn kern_the_old_way(face: &Face, slots: &mut [Slot], right_to_left: bool) {
    let mut previous: Option<usize> = None;
    for index in 0..slots.len() {
        if slots[index].is_mark() || slots[index].mask & MASK_HIDDEN != 0 {
            continue;
        }
        if let Some(previous) = previous {
            if right_to_left {
                let kerning = face
                    .font
                    .old_kerning(slots[index].glyph, slots[previous].glyph);
                slots[index].x_advance += i32::from(kerning);
            } else {
                let kerning = face
                    .font
                    .old_kerning(slots[previous].glyph, slots[index].glyph);
                slots[previous].x_advance += i32::from(kerning);
            }
        }
        previous = Some(index);
    }
}

impl TextEngine {
    /// How much work this text needs in this font. This is the only cost plain text has.
    pub(crate) fn complexity(&mut self, text: &str, primary: usize) -> Complexity {
        let found = self.complexity_in_order(text, primary);
        // In a paragraph that goes from the right, or with every letter turned around, plain letters can come out in another
        // order: digits and what's around them, punctuation at either end (`1234 5678` and `Hello!` there show in another order).
        if found != Complexity::Complex && self.bidi.get() & 6 != 0 && reorders(text, self.bidi.get()) {
            return Complexity::Complex;
        }
        found
    }

    fn complexity_in_order(&mut self, text: &str, primary: usize) -> Complexity {
        if !text.is_ascii() {
            // Text that's shaped gets its fonts opened then, so that remembered text needn't look.
            if text.chars().any(is_complex_character) {
                return Complexity::Complex;
            }
            self.open_fallbacks_for(text, primary);
        }
        let rules = self.faces[primary].ascii_rules();
        if rules.contextual {
            return Complexity::Ligatures;
        }
        // Many fonts have no ligatures for ASCII, which saves looking at each byte.
        if rules.pairs.is_empty() {
            return Complexity::Plain;
        }
        let mut previous = 0x7Fu8;
        for &byte in text.as_bytes() {
            if rules.firsts >> previous & 1 != 0
                && rules.pairs.binary_search(&(previous, byte)).is_ok()
            {
                return Complexity::Ligatures;
            }
            previous = byte & 0x7F;
        }
        Complexity::Plain
    }

    /// Chooses the font for each character. Marks stay on what they sit on if they can, and
    /// characters that show nothing don't split a run.
    fn faces_of(&self, primary: usize, characters: &[(u32, char)]) -> Vec<u16> {
        let mut faces = Vec::with_capacity(characters.len());
        let mut previous = primary;
        for (index, (_, character)) in characters.iter().enumerate() {
            let has_it = |face: usize| self.faces[face].glyph(*character) != 0;
            // What's joined on to an emoji is part of it, whatever other fonts have for it.
            let joined = index > 0
                && characters[index - 1].1 == '\u{200D}'
                && self.faces[previous].font.has_color_layers()
                && has_it(previous);
            let stays =
                is_invisible(*character) || joined || (is_mark(*character) && has_it(previous));
            let chosen = if stays {
                previous
            } else if asks_for_emoji(characters, index) {
                self.color_face_with(*character)
                    .unwrap_or_else(|| self.find_glyph(primary, *character).0)
            } else {
                self.find_glyph(primary, *character).0
            };
            faces.push(chosen as u16);
            previous = chosen;
        }
        faces
    }

    /// The first font with colored glyphs that has this character.
    fn color_face_with(&self, character: char) -> Option<usize> {
        self.search_order
            .iter()
            .find(|entry| {
                let face = &self.faces[entry.face as usize];
                // Asking for an emoji goes for a font of them, even for what it isn't otherwise for, like
                // a digit in a keycap, which a font for just flags isn't.
                (entry.is_for(character) || entry.is_for('\u{FE0F}'))
                    && face.font.has_color_layers()
                    && face.glyph(character) != 0
            })
            .map(|entry| entry.face as usize)
    }

    fn paragraph(&mut self, text: &str, at: usize, primary: usize) -> Paragraph {
        self.open_fallbacks_for(text, primary);
        let characters: Vec<(u32, char)> = text
            .char_indices()
            .filter(|(_, character)| *character != '\r')
            .map(|(byte, character)| ((at + byte) as u32, character))
            .collect();
        let only_characters: Vec<char> = characters.iter().map(|(_, c)| *c).collect();
        let key = self.bidi.get();
        let mut result = bidi::paragraph(&only_characters, level_of_key(key));
        if key & 4 != 0 {
            // `bidi-override`: every letter goes the paragraph's way, as if between an RLO or LRO and a PDF.
            result.levels.fill(result.level);
        }
        Paragraph {

            scripts: resolve_scripts(&characters),
            faces: self.faces_of(primary, &characters),
            characters,
            levels: result.levels,
            classes: result.classes,
            level: result.level,
        }
    }

    /// How wide each character in a paragraph is, which is what lines get broken by. The parts
    /// of a ligature get a share each.
    fn character_widths(&mut self, paragraph: &Paragraph, end: usize, size: f32) -> Vec<f32> {
        let mut widths = vec![0.0f32; end];
        let mut slots = Vec::new();
        let characters = &paragraph.characters[..end];
        for run in runs_of(
            &paragraph.levels[..end],
            &paragraph.scripts[..end],
            &paragraph.faces[..end],
            characters,
        ) {
            let right_to_left = paragraph.levels[run.start] % 2 == 1;
            let script = paragraph.scripts[run.start];
            let face_index = paragraph.faces[run.start] as usize;
            let plan = self.faces[face_index].plan(script, right_to_left);
            let face = &self.faces[face_index];
            let scale = size / f32::from(face.font.metrics().units_per_em);
            shape_run(
                face,
                &face.plans[plan].2,
                script,
                right_to_left,
                &characters[run.clone()],
                &mut slots,
            );
            let mut index = 0;
            while index < slots.len() {
                let cluster = slots[index].cluster;
                let mut advance = 0;
                while index < slots.len() && slots[index].cluster == cluster {
                    advance += slots[index].x_advance;
                    index += 1;
                }
                let from = run.start
                    + characters[run.clone()].partition_point(|(byte, _)| *byte < cluster);
                let to = match slots.get(index) {
                    Some(next) => {
                        run.start
                            + characters[run.clone()]
                                .partition_point(|(byte, _)| *byte < next.cluster)
                    }
                    None => run.end,
                };
                let to = characters_in_cluster(from, to, run.end);
                share_width(
                    &mut widths[from..to],
                    &characters[from..to],
                    advance as f32 * scale,
                );
            }
        }
        widths
    }

    /// Shapes and places part of a paragraph as one line, and says how wide it came out.
    fn shape_line(
        &mut self,
        paragraph: &Paragraph,
        range: Range<usize>,
        size: f32,
        baseline: f32,
        output: &mut Output<'_>,
    ) -> f32 {
        let characters = &paragraph.characters[range.clone()];
        let levels = line_levels(
            &paragraph.levels[range.clone()],
            &paragraph.classes[range.clone()],
            paragraph.level,
        );
        let scripts = &paragraph.scripts[range.clone()];
        let faces = &paragraph.faces[range];
        let runs = runs_of(&levels, scripts, faces, characters);
        let run_levels: Vec<u8> = runs.iter().map(|run| levels[run.start]).collect();
        let mut slots = Vec::new();
        let mut pen = 0.0f32;
        for position_in_line in bidi::visual_order(&run_levels) {
            let run = runs[position_in_line].clone();
            let right_to_left = levels[run.start] % 2 == 1;
            let script = scripts[run.start];
            let face_index = faces[run.start] as usize;
            let plan = self.faces[face_index].plan(script, right_to_left);
            let face = &self.faces[face_index];
            let run_characters = &characters[run];
            shape_run(
                face,
                &face.plans[plan].2,
                script,
                right_to_left,
                run_characters,
                &mut slots,
            );
            pen = place_run(
                &slots,
                run_characters,
                face_index as u16,
                size / f32::from(face.font.metrics().units_per_em),
                right_to_left,
                pen,
                baseline,
                output,
            );
        }
        pen
    }

    /// Lays out text from scratch.
    fn shape_text(
        &mut self,
        text: &str,
        primary: usize,
        size: f32,
        max_width: Option<f32>,
        key: ShapedKey,
    ) -> Shaped {
        let (line_height, ascent) = self.text_line_metrics(text, primary, size);
        let mut shaped = Shaped {
            key,
            lines: Vec::new(),
            glyphs: Vec::new(),
            clusters: Vec::new(),
            right_to_left: false,
        };
        let Some(max_width) = max_width else {
            // One line in a text input, which keeps spaces at the end, as the caret goes past them.
            let paragraph = self.paragraph(text, 0, primary);
            shaped.right_to_left = paragraph.level % 2 == 1;
            let mut output = Output {
                glyphs: &mut shaped.glyphs,
                clusters: Some(&mut shaped.clusters),
            };
            let count = paragraph.characters.len();
            let width = self.shape_line(&paragraph, 0..count, size, ascent, &mut output);
            shaped.lines.push(ShapedLine {
                start: 0,
                end: text.len() as u32,
                width,
                x: 0.0,
                baseline: ascent,
                glyphs: 0..shaped.glyphs.len() as u32,
            });
            return shaped;
        };
        let mut at = 0;
        let mut aligned_right: Vec<bool> = Vec::new();
        for piece in text.split('\n') {
            let paragraph = self.paragraph(piece, at, primary);
            shaped.right_to_left |= paragraph.level % 2 == 1;
            let right_to_left = paragraph.level % 2 == 1;
            let visible = visible_length(&paragraph.characters);
            let lines: Vec<(usize, usize)> = if max_width.is_finite() {
                let widths = self.character_widths(&paragraph, visible, size);
                break_lines(&paragraph.characters[..visible], &widths, max_width)
            } else {
                vec![(0, visible)]
            };
            for (from, to) in lines {
                let baseline = ascent + shaped.lines.len() as f32 * line_height;
                let first = shaped.glyphs.len() as u32;
                let mut output = Output {
                    glyphs: &mut shaped.glyphs,
                    clusters: None,
                };
                let width = self.shape_line(&paragraph, from..to, size, baseline, &mut output);
                // A line of nothing but spaces starts where the paragraph does, as plain text does.
                let start = if from == 0 {
                    at as u32
                } else {
                    paragraph
                        .characters
                        .get(from)
                        .map_or((at + piece.len()) as u32, |first| first.0)
                };
                let end = if from < to {
                    end_of(&paragraph.characters[from..to])
                } else {
                    start
                };
                aligned_right.push(right_to_left);
                shaped.lines.push(ShapedLine {
                    start,
                    end,
                    width,
                    x: 0.0,
                    baseline,
                    glyphs: first..shaped.glyphs.len() as u32,
                });
            }
            at += piece.len() + 1;
        }
        align_lines(&mut shaped, &aligned_right, max_width);
        shaped
    }

    /// The result for some text, from memory if possible.
    fn shaped(&mut self, text: &str, primary: usize, size: f32, max_width: Option<f32>) -> &Shaped {
        let key = ShapedKey {
            text: FixedState::default().hash_one(text),
            length: text.len(),
            size: size.to_bits(),
            primary,
            width: max_width.map_or(INPUT_LINE, f32::to_bits),
            bidi: self.bidi.get(),
        };
        let mixed = key.text
            ^ u64::from(key.bidi)
            ^ u64::from(key.size).wrapping_mul(0x9E37_79B9)
            ^ u64::from(key.width).wrapping_mul(0x85EB_CA6B)
            ^ primary as u64;
        let set = ((mixed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as usize % CACHE_SLOTS)
            & !(CACHE_WAYS - 1);
        if self.shaped_cache.slots.is_empty() {
            self.shaped_cache.slots.resize_with(CACHE_SLOTS, || None);
        }
        let hit = self.shaped_cache.slots[set..set + CACHE_WAYS]
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|shaped| shaped.key == key));
        let slot = if let Some(way) = hit {
            set + way
        } else {
            let shaped = Box::new(self.shape_text(text, primary, size, max_width, key));
            let cache = &mut self.shaped_cache;
            let empty = cache.slots[set..set + CACHE_WAYS]
                .iter()
                .position(Option::is_none);
            let way = empty.unwrap_or_else(|| {
                cache.next_way = (cache.next_way + 1) % CACHE_WAYS;
                cache.next_way
            });
            let slot = set + way;
            cache.insert(slot, shaped);
            slot
        };
        self.shaped_cache.slots[slot]
            .as_deref()
            .expect("the slot was just filled")
    }

    /// Fills in `layout` for text that isn't plain.
    pub(crate) fn layout_shaped_into(
        &mut self,
        text: &str,
        primary: usize,
        size: f32,
        max_width: f32,
        layout: &mut TextLayout,
    ) {
        let shaped = self.shaped(text, primary, size, Some(max_width));
        layout.glyphs.extend_from_slice(&shaped.glyphs);
        layout.lines.extend(shaped.lines.iter().map(|line| Line {
            start: line.start as usize,
            end: line.end as usize,
            width: line.width,
            x: line.x,
            baseline: line.baseline,
            first_glyph: line.glyphs.start,
            glyph_count: line.glyphs.end - line.glyphs.start,
        }));
    }

    /// How many lines text that isn't plain takes up, and how wide the widest is.
    pub(crate) fn measure_shaped(
        &mut self,
        text: &str,
        primary: usize,
        size: f32,
        max_width: f32,
    ) -> (u32, f32) {
        let shaped = self.shaped(text, primary, size, Some(max_width));
        let widest = shaped
            .lines
            .iter()
            .fold(0f32, |widest, line| widest.max(line.width));
        (shaped.lines.len() as u32, widest)
    }

    /// Lays out one line in a text input, placing what falls in this range across it.
    pub(crate) fn layout_shaped_across(
        &mut self,
        text: &str,
        primary: usize,
        size: f32,
        across: (f32, f32),
        layout: &mut TextLayout,
    ) {
        let shaped = self.shaped(text, primary, size, None);
        layout.glyphs.extend(
            shaped
                .glyphs
                .iter()
                .filter(|glyph| glyph.x >= across.0 && glyph.x <= across.1),
        );
    }

    pub(crate) fn shaped_caret_x(
        &mut self,
        text: &str,
        primary: usize,
        size: f32,
        at: usize,
    ) -> f32 {
        self.shaped(text, primary, size, None).caret_x(text, at)
    }

    pub(crate) fn shaped_stops(
        &mut self,
        text: &str,
        primary: usize,
        size: f32,
    ) -> Vec<(usize, f32)> {
        self.shaped(text, primary, size, None).stops(text)
    }

    pub(crate) fn shaped_spans(
        &mut self,
        text: &str,
        primary: usize,
        size: f32,
        selected: (usize, usize),
        spans: &mut Vec<(f32, f32)>,
    ) {
        let shaped = self.shaped(text, primary, size, None);
        for cluster in &shaped.clusters {
            let from = selected.0.max(cluster.start as usize);
            let to = selected.1.min(cluster.end as usize);
            if from >= to {
                continue;
            }
            let first = cluster.x_at(text, from);
            let second = if to == cluster.end as usize {
                cluster.trailing(true)
            } else {
                cluster.x_at(text, to)
            };
            let (left, right) = (first.min(second), first.max(second));
            match spans.last_mut() {
                Some(last) if (last.1 - left).abs() < 0.01 => last.1 = right,
                _ => spans.push((left, right)),
            }
        }
    }

    /// Whether one line in a text input starts from the right, and how wide it is.
    pub(crate) fn shaped_direction(
        &mut self,
        text: &str,
        primary: usize,
        size: f32,
    ) -> (bool, f32) {
        let shaped = self.shaped(text, primary, size, None);
        (
            shaped.right_to_left,
            shaped.lines.first().map_or(0.0, |line| line.width),
        )
    }

    /// Finds the lines of long text that needs shaping, without keeping any of its glyphs.
    pub(crate) fn long_shaped_lines(
        &mut self,
        text: &str,
        primary: usize,
        size: f32,
        max_width: f32,
    ) -> LongShaped {
        let mut result = LongShaped::default();
        let mut at = 0;
        for piece in text.split('\n') {
            let paragraph = self.paragraph(piece, at, primary);
            result.paragraphs.push((at as u32, paragraph.level));
            let mut previous = None;
            for (index, level) in paragraph.levels.iter().enumerate() {
                if previous != Some(*level) {
                    result.levels.push((paragraph.characters[index].0, *level));
                    previous = Some(*level);
                }
            }
            let visible = visible_length(&paragraph.characters);
            let widths = self.character_widths(&paragraph, visible, size);
            for (from, to) in break_lines(&paragraph.characters[..visible], &widths, max_width) {
                let width: f32 = widths[from..to].iter().sum();
                let start = if from == 0 {
                    at as u32
                } else {
                    paragraph.characters[from].0
                };
                let end = if from < to {
                    end_of(&paragraph.characters[from..to])
                } else {
                    start
                };
                result.widest = result.widest.max(width);
                result.lines.push((start, end, width));
            }
            at += piece.len() + 1;
        }
        result.lines.shrink_to_fit();
        result.levels.shrink_to_fit();
        result.paragraphs.shrink_to_fit();
        result
    }

    /// Places one line of long text that needs shaping, and returns how far along it starts.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn place_long_shaped_line(
        &mut self,
        text: &str,
        lines: &LongShaped,
        line: usize,
        primary: usize,
        size: f32,
        max_width: f32,
        baseline: f32,
        glyphs: &mut Vec<PlacedGlyph>,
    ) -> f32 {
        let (start, end, width) = lines.lines[line];
        let (start_byte, end_byte) = (start as usize, end as usize);
        let characters: Vec<(u32, char)> = text[start_byte..end_byte]
            .char_indices()
            .filter(|(_, character)| *character != '\r')
            .map(|(byte, character)| ((start_byte + byte) as u32, character))
            .collect();
        let found = lines
            .paragraphs
            .partition_point(|(at, _)| *at <= start)
            .max(1);
        let level = lines.paragraphs[found - 1].1;
        let mut levels = Vec::with_capacity(characters.len());
        let mut next = lines.levels.partition_point(|(at, _)| *at <= start).max(1) - 1;
        for (byte, _) in &characters {
            while next + 1 < lines.levels.len() && lines.levels[next + 1].0 <= *byte {
                next += 1;
            }
            levels.push(lines.levels.get(next).map_or(level, |run| run.1));
        }
        let paragraph = Paragraph {
            classes: characters.iter().map(|(_, c)| bidi::class_of(*c)).collect(),
            scripts: resolve_scripts(&characters),
            faces: self.faces_of(primary, &characters),
            levels,
            level,
            characters,
        };
        let first = glyphs.len();
        let count = paragraph.characters.len();
        let mut output = Output {
            glyphs,
            clusters: None,
        };
        self.shape_line(&paragraph, 0..count, size, baseline, &mut output);
        let x = if level % 2 == 1 {
            let room = if max_width.is_finite() {
                max_width
            } else {
                lines.widest
            };
            (room - width).max(0.0)
        } else {
            0.0
        };
        for glyph in &mut glyphs[first..] {
            glyph.x += x;
        }
        x
    }

    /// Shapes one line of text that's in order, such as for its ligatures.
    pub(crate) fn place_shaped_line(
        &mut self,
        text: &str,
        at: usize,
        primary: usize,
        size: f32,
        baseline: f32,
        glyphs: &mut Vec<PlacedGlyph>,
    ) {
        let paragraph = self.paragraph(text, at, primary);
        let count = paragraph.characters.len();
        let mut output = Output {
            glyphs,
            clusters: None,
        };
        self.shape_line(&paragraph, 0..count, size, baseline, &mut output);
    }

    /// Lets go of everything remembered about shaped text.
    pub(crate) fn forget_shaped(&mut self) {
        self.shaped_cache = ShapedCache::default();
    }
}

/// The lines of long text that needs shaping, and how to find out about any one of them later.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LongShaped {
    /// Start, end and width of each line.
    pub(crate) lines: Vec<(u32, u32, f32)>,
    /// Where each paragraph starts, and its level.
    paragraphs: Vec<(u32, u8)>,
    /// Where the level changes, and what it changes to.
    levels: Vec<(u32, u8)>,
    pub(crate) widest: f32,
}

impl LongShaped {
    /// Heap capacity retained for reconstructing visible lines of a long text.
    pub(crate) fn bytes(&self) -> usize {
        self.lines.capacity() * size_of::<(u32, u32, f32)>()
            + self.paragraphs.capacity() * size_of::<(u32, u8)>()
            + self.levels.capacity() * size_of::<(u32, u8)>()
    }
}

/// The paragraph level that `TextStyle::bidi_key` says, as `bidi::paragraph` takes it.
pub(crate) fn level_of_key(key: u8) -> Option<u8> {
    match key & 3 {
        0 => None,
        1 => Some(0),
        _ => Some(1),
    }
}

/// Whether text with no letters that need shaping shows in another order than it's written in, in paragraphs that go as
/// `TextStyle::bidi_key` says: in one from the right, digits and what's between them and punctuation at either end do, and
/// with every letter going from the right all of it does. Spaces at either end don't count, as they don't show.
pub(crate) fn reorders(text: &str, key: u8) -> bool {
    if level_of_key(key) != Some(1) {
        return false;
    }
    let trimmed = text.trim_matches(|character: char| character.is_whitespace());
    if key & 4 != 0 {
        return trimmed.chars().nth(1).is_some();
    }
    for piece in trimmed.split('\n') {
        let piece = piece.trim_matches(|character: char| character.is_whitespace());
        // Letters that go from the left, and spaces between them, are one run in order.
        let plain = piece.chars().all(|character| {
            character.is_ascii_alphabetic()
                || character == ' '
                || (!character.is_ascii() && bidi::class_of(character) == Class::L)
        });
        if plain {
            continue;
        }
        let characters: Vec<char> = piece.chars().filter(|character| *character != '\r').collect();
        if characters.len() < 2 {
            continue;
        }
        let found = bidi::paragraph(&characters, Some(1));
        let first = found.levels[0];
        if found.levels.iter().any(|level| *level != first) || first % 2 == 1 {
            return true;
        }
    }
    false
}

/// Gives the width of a cluster to its characters: an even share for each, and none for marks.
fn share_width(widths: &mut [f32], characters: &[(u32, char)], width: f32) {
    let count = characters
        .iter()
        .filter(|(_, c)| !is_mark(*c) && !is_invisible(*c))
        .count();
    if count == 0 {
        if let Some(first) = widths.first_mut() {
            *first += width;
        }
        return;
    }
    let share = width / count as f32;
    for (slot, (_, character)) in widths.iter_mut().zip(characters) {
        if !is_mark(*character) && !is_invisible(*character) {
            *slot += share;
        }
    }
}

/// How many characters are left after leaving off spaces at the end.
fn visible_length(characters: &[(u32, char)]) -> usize {
    characters
        .iter()
        .rposition(|(_, character)| !is_space(*character))
        .map_or(0, |at| at + 1)
}

/// Whether what comes after this character says it's to be shown as an emoji.
fn asks_for_emoji(characters: &[(u32, char)], index: usize) -> bool {
    matches!(
        characters.get(index + 1),
        Some((_, '\u{FE0F}' | '\u{20E3}'))
    )
}

/// Finds the lines in a paragraph, as ranges of its characters that leave out spaces at each end.
fn break_lines(characters: &[(u32, char)], widths: &[f32], max_width: f32) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    let mut start = 0;
    // Finding the words takes a dictionary, so it's only done for a paragraph that has to wrap.
    let words = if widths.iter().sum::<f32>() > max_width
        && characters
            .iter()
            .any(|(_, character)| needs_dictionary(*character))
    {
        word_starts(characters)
    } else {
        Vec::new()
    };
    loop {
        let (end, next) = next_measured_line(characters, widths, start, max_width, &words);
        lines.push((start, end));
        if next >= characters.len() {
            return lines;
        }
        start = next;
    }
}

/// Moves lines from paragraphs that run right to left over to the right.
fn align_lines(shaped: &mut Shaped, aligned_right: &[bool], max_width: f32) {
    let room = if max_width.is_finite() {
        max_width
    } else {
        shaped
            .lines
            .iter()
            .fold(0f32, |widest, line| widest.max(line.width))
    };
    for (line, right) in shaped.lines.iter_mut().zip(aligned_right) {
        if !right {
            continue;
        }
        line.x = (room - line.width).max(0.0);
        if line.x > 0.0 {
            let range = line.glyphs.start as usize..line.glyphs.end as usize;
            for glyph in &mut shaped.glyphs[range] {
                glyph.x += line.x;
            }
        }
    }
}

/// Places a shaped run with the pen starting here, and returns where the pen ends up.
#[allow(clippy::too_many_arguments)]
fn place_run(
    slots: &[Slot],
    characters: &[(u32, char)],
    font: u16,
    scale: f32,
    right_to_left: bool,
    mut pen: f32,
    baseline: f32,
    output: &mut Output<'_>,
) -> f32 {
    // In the order of the text: which slots each cluster has, and where it starts.
    let mut clusters: Vec<(u32, Range<usize>)> = Vec::new();
    for (index, slot) in slots.iter().enumerate() {
        let start = cluster_start(characters, slot.cluster);
        match clusters.last_mut() {
            Some((last, range)) if *last >= start => range.end = index + 1,
            _ => clusters.push((start, index..index + 1)),
        }
    }
    let run_end = end_of(characters);
    let count = clusters.len();
    for at in 0..count {
        let index = if right_to_left { count - 1 - at } else { at };
        let (start, range) = &clusters[index];
        let end = clusters.get(index + 1).map_or(run_end, |next| next.0);
        let left = pen;
        let place = |slot: &Slot, pen: f32, glyphs: &mut Vec<PlacedGlyph>| {
            if slot.mask & MASK_HIDDEN == 0 {
                glyphs.push(PlacedGlyph {
                    font,
                    glyph: slot.glyph,
                    x: pen + slot.x_offset as f32 * scale,
                    // Whole pixels up and down, as text sits on whole pixels everywhere.
                    y: baseline - (slot.y_offset as f32 * scale).round(),
                });
            }
        };
        if right_to_left {
            for slot in slots[range.clone()].iter().rev() {
                place(slot, pen, output.glyphs);
                pen += slot.x_advance as f32 * scale;
            }
        } else {
            for slot in &slots[range.clone()] {
                place(slot, pen, output.glyphs);
                pen += slot.x_advance as f32 * scale;
            }
        }
        if let Some(out) = output.clusters.as_deref_mut() {
            out.push(Cluster {
                start: *start,
                end,
                left,
                right: pen,
                right_to_left,
            });
        }
    }
    pen
}
