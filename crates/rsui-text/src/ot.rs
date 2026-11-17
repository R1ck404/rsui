//! Runs the programs in a font's GSUB and GPOS tables, which decide what glyphs a run of text
//! becomes and where they go.
//!
//! A font comes from outside the program, so everything read from it is checked, and running it
//! has a budget, so that no font can make this hang or use all the memory.

use crate::font::{read_i16, read_u16, read_u32};
use crate::gpos::{coverage_index, glyph_class};

/// The glyph is a base, as GDEF classes them.
const CLASS_LIGATURE: u8 = 2;
const CLASS_MARK: u8 = 3;

const IGNORE_BASE: u16 = 2;
const IGNORE_LIGATURE: u16 = 4;
const IGNORE_MARK: u16 = 8;
const USE_FILTER_SET: u16 = 0x10;
const RIGHT_TO_LEFT: u16 = 1;

/// How deep one rule can call another.
const MAX_NESTING: u32 = 8;
/// How long a sequence a rule can match.
const MAX_RULE_LENGTH: usize = 64;
/// How many steps shaping one glyph can take on average. Real fonts need a few dozen.
const BUDGET_PER_GLYPH: u32 = 2048;
/// How many times longer shaping can make a run of glyphs.
const MAX_GROWTH: usize = 16;
/// How long a chain of glyphs attached to each other can be followed.
const MAX_ATTACHMENT_CHAIN: u32 = 64;

/// Which features apply to this glyph, among those in a stage.
pub(crate) const MASK_ISOLATED: u16 = 1;
pub(crate) const MASK_FINAL: u16 = 2;
pub(crate) const MASK_MEDIAL: u16 = 4;
pub(crate) const MASK_INITIAL: u16 = 8;
pub(crate) const MASK_ALL: u16 = 0x10;
/// Lookups ignore this glyph, and it shows nothing, such as a zero-width joiner.
pub(crate) const MASK_INVISIBLE: u16 = 0x20;
/// This is a zero-width non-joiner, which shows nothing but gets in the way of ligatures.
pub(crate) const MASK_NON_JOINER: u16 = 0x40;
/// This is a zero-width joiner in text that joins, which shows nothing, and also gets in the way of
/// ligatures, as well as being something a rule can look for.
pub(crate) const MASK_JOINER: u16 = 0x80;
/// Any of the masks that mean nothing is shown, and no room is taken up.
pub(crate) const MASK_HIDDEN: u16 = MASK_INVISIBLE | MASK_NON_JOINER | MASK_JOINER;
// Which forms a consonant in an Indic script can take, depending on where it is in its syllable.
pub(crate) const MASK_RPHF: u16 = 0x100;
pub(crate) const MASK_PREF: u16 = 0x200;
pub(crate) const MASK_BLWF: u16 = 0x400;
pub(crate) const MASK_ABVF: u16 = 0x800;
pub(crate) const MASK_HALF: u16 = 0x1000;
pub(crate) const MASK_PSTF: u16 = 0x2000;
pub(crate) const MASK_INIT: u16 = 0x4000;

const WAS_SUBSTITUTED: u8 = 1;
const WAS_LIGATED: u8 = 2;
const WAS_MULTIPLIED: u8 = 4;

/// One glyph being shaped.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Slot {
    pub(crate) glyph: u16,
    pub(crate) class: u8,
    pub(crate) mask: u16,
    /// For an Indic script, what kind of thing this is, where it goes, and which syllable it's in.
    pub(crate) category: u8,
    pub(crate) place: u8,
    pub(crate) syllable: u16,
    /// What's been done to this glyph, as some rules depend on that.
    history: u8,
    /// The byte in the text this came from.
    pub(crate) cluster: u32,
    /// Identifies a ligature, and marks that sit on it.
    ligature: u16,
    /// For a mark on a ligature, which of its parts it follows, counting from 1.
    component: u8,
    pub(crate) x_advance: i32,
    pub(crate) x_offset: i32,
    pub(crate) y_offset: i32,
    /// How far away the glyph this one is attached to is, in either direction. Nothing can be attached
    /// to something further away than this can say, which no font has a use for.
    attached: i16,
    attached_as_mark: bool,
}

impl Slot {
    /// `mark` says whether the character is a mark, which counts if the font doesn't say.
    pub(crate) fn new(glyph: u16, cluster: u32, mask: u16, mark: bool) -> Self {
        Self {
            glyph,
            cluster,
            mask,
            class: if mark { CLASS_MARK } else { 0 },
            ..Self::default()
        }
    }

    pub(crate) fn is_mark(&self) -> bool {
        self.class == CLASS_MARK
    }

    pub(crate) fn is_substituted(&self) -> bool {
        self.history & WAS_SUBSTITUTED != 0
    }

    pub(crate) fn is_ligated(&self) -> bool {
        self.history & WAS_LIGATED != 0
    }

    pub(crate) fn is_multiplied(&self) -> bool {
        self.history & WAS_MULTIPLIED != 0
    }

    pub(crate) fn clear_substituted(&mut self) {
        self.history &= !WAS_SUBSTITUTED;
    }

    /// Whether this is the second or a later glyph that one glyph was split into.
    pub(crate) fn is_later_part(&self) -> bool {
        self.is_multiplied() && self.component != 0
    }

    pub(crate) fn clear_ligated(&mut self) {
        self.history &= !(WAS_LIGATED | WAS_MULTIPLIED);
    }
}

/// The tables shaping needs, from one font.
#[derive(Clone, Copy)]
pub(crate) struct Tables<'a> {
    pub(crate) gsub: Option<&'a [u8]>,
    pub(crate) gpos: Option<&'a [u8]>,
    pub(crate) gdef: Option<&'a [u8]>,
}

/// A quick way to tell that a lookup has nothing to do with a glyph, as most don't. This is two
/// sets of 64 bits, each for part of a glyph's number. If either bit is unset, it isn't there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Summary([u64; 2]);

impl Summary {
    const EVERYTHING: Self = Self([u64::MAX; 2]);

    fn add(&mut self, glyph: u16) {
        self.0[0] |= 1 << (glyph & 63);
        self.0[1] |= 1 << ((glyph >> 5) & 63);
    }

    fn add_range(&mut self, first: u16, last: u16) {
        for glyph in first..=last {
            self.add(glyph);
            if glyph.wrapping_sub(first) >= 2048 {
                *self = Self::EVERYTHING;
                return;
            }
        }
    }

    #[inline]
    fn may_have(self, glyph: u16) -> bool {
        self.0[0] >> (glyph & 63) & 1 != 0 && self.0[1] >> ((glyph >> 5) & 63) & 1 != 0
    }
}

/// How many glyphs and ranges are read to make all the summaries for a table. A font that has more
/// than this is either enormous or damaged, and whatever's left just isn't summarized.
const SUMMARY_BUDGET: u32 = 1 << 20;

/// One lookup to run, and which glyphs it runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Step {
    lookup: u16,
    mask: u16,
    summary: Summary,
}

/// Finds which glyphs a lookup could start at. Anything unclear counts as being all of them.
fn summarize(table: &[u8], lookup: u16, substituting: bool, budget: &mut u32) -> Summary {
    summarize_inner(table, lookup, substituting, budget).unwrap_or(Summary::EVERYTHING)
}

fn summarize_inner(
    table: &[u8],
    lookup: u16,
    substituting: bool,
    budget: &mut u32,
) -> Option<Summary> {
    let list = read_u16(table, 8)? as usize;
    if lookup >= read_u16(table, list)? {
        return Some(Summary([0; 2]));
    }
    let at = list + read_u16(table, list + 2 + lookup as usize * 2)? as usize;
    let kind = read_u16(table, at)?;
    let count = read_u16(table, at + 4)? as usize;
    let mut summary = Summary([0; 2]);
    for index in 0..count {
        let mut subtable = at + read_u16(table, at + 6 + index * 2)? as usize;
        let mut subtable_kind = kind;
        if kind == if substituting { 7 } else { 9 } {
            subtable_kind = read_u16(table, subtable + 2)?;
            subtable += read_u32(table, subtable + 4)? as usize;
        }
        let context = if substituting { (5, 6) } else { (7, 8) };
        let format = read_u16(table, subtable)?;
        let coverage = if subtable_kind == context.0 && format == 3 {
            subtable + read_u16(table, subtable + 6)? as usize
        } else if subtable_kind == context.1 && format == 3 {
            first_chain_coverage(table, subtable)?
        } else {
            subtable + read_u16(table, subtable + 2)? as usize
        };
        add_coverage(table, coverage, &mut summary, budget)?;
    }
    Some(summary)
}

fn add_coverage(table: &[u8], at: usize, summary: &mut Summary, budget: &mut u32) -> Option<()> {
    let count = read_u16(table, at + 2)? as usize;
    *budget = budget.checked_sub(count as u32 + 1)?;
    match read_u16(table, at)? {
        1 => {
            for index in 0..count {
                summary.add(read_u16(table, at + 4 + index * 2)?);
            }
        }
        2 => {
            for index in 0..count {
                let range = at + 4 + index * 6;
                summary.add_range(read_u16(table, range)?, read_u16(table, range + 2)?);
            }
        }
        _ => return None,
    }
    Some(())
}

/// Which lookups a font runs for a script, in order.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    /// Each stage finishes before the next begins.
    pub(crate) substitutions: Vec<Vec<Step>>,
    pub(crate) positions: Vec<Step>,
    /// Whether the positions include kerning, or an older table should be used.
    pub(crate) has_kerning: bool,
    /// Whether the font only has what's for the older way of shaping an Indic script.
    pub(crate) older_way: bool,
    /// Whether the font says it has anything for this script, rather than what it has for any script.
    pub(crate) knows_script: bool,
}

fn has_script(table: &[u8], tag: &[u8; 4]) -> bool {
    let Some(list) = read_u16(table, 4).map(|at| at as usize) else {
        return false;
    };
    let count = read_u16(table, list).unwrap_or(0) as usize;
    (0..count).any(|index| table.get(list + 2 + index * 6..list + 6 + index * 6) == Some(&tag[..]))
}

/// The features of a script's default language in a table, each with its lookups.
fn features_of(table: &[u8], scripts: &[&[u8; 4]]) -> Option<Vec<([u8; 4], Vec<u16>)>> {
    let script_list = read_u16(table, 4)? as usize;
    let feature_list = read_u16(table, 6)? as usize;
    let count = read_u16(table, script_list)? as usize;
    let mut found = None;
    let usual: [&[u8; 4]; 3] = [b"DFLT", b"dflt", b"latn"];
    for wanted in scripts.iter().chain(&usual) {
        for index in 0..count {
            let record = script_list + 2 + index * 6;
            if table.get(record..record + 4)? == &wanted[..] {
                found = Some(script_list + read_u16(table, record + 4)? as usize);
                break;
            }
        }
        if found.is_some() {
            break;
        }
    }
    let script_table = found?;
    let language = read_u16(table, script_table)? as usize;
    if language == 0 {
        return None;
    }
    let language = script_table + language;
    let required = read_u16(table, language + 2)?;
    let feature_count = read_u16(table, language + 4)? as usize;
    let total_features = read_u16(table, feature_list)?;
    let mut indices = Vec::with_capacity(feature_count + 1);
    if required != 0xFFFF {
        indices.push(required);
    }
    for index in 0..feature_count {
        indices.push(read_u16(table, language + 6 + index * 2)?);
    }
    let mut features = Vec::with_capacity(indices.len());
    for index in indices {
        if index >= total_features {
            continue;
        }
        let record = feature_list + 2 + index as usize * 6;
        let tag: [u8; 4] = table.get(record..record + 4)?.try_into().ok()?;
        let feature = feature_list + read_u16(table, record + 4)? as usize;
        let lookups = read_u16(table, feature + 2)? as usize;
        let list = (0..lookups)
            .map(|at| read_u16(table, feature + 4 + at * 2))
            .collect::<Option<Vec<u16>>>()?;
        features.push((tag, list));
    }
    Some(features)
}

fn steps_of(
    table: Option<&[u8]>,
    features: &[([u8; 4], Vec<u16>)],
    wanted: &[(&[u8; 4], u16)],
    substituting: bool,
    budget: &mut u32,
) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    for (tag, mask) in wanted {
        for (feature, lookups) in features {
            if feature == *tag {
                for lookup in lookups {
                    match steps.iter_mut().find(|step| step.lookup == *lookup) {
                        Some(step) => step.mask |= mask,
                        None => steps.push(Step {
                            lookup: *lookup,
                            mask: *mask,
                            summary: table.map_or(Summary::EVERYTHING, |table| {
                                summarize(table, *lookup, substituting, budget)
                            }),
                        }),
                    }
                }
            }
        }
    }
    steps.sort_unstable_by_key(|step| step.lookup);
    steps
}

impl Plan {
    /// Works out what to run for a script. `stages` are its substitutions, in order.
    pub(crate) fn new(
        tables: Tables<'_>,
        scripts: &[&[u8; 4]],
        stages: &[&[(&[u8; 4], u16)]],
        positions: &[(&[u8; 4], u16)],
    ) -> Self {
        let substitution_features = tables
            .gsub
            .and_then(|table| features_of(table, scripts))
            .unwrap_or_default();
        let position_features = tables
            .gpos
            .and_then(|table| features_of(table, scripts))
            .unwrap_or_default();
        // The first tag is the newer way of doing things, if a script has two.
        let newer_way = scripts.len() < 2
            || tables
                .gsub
                .is_some_and(|table| has_script(table, scripts[0]));
        let mut budget = SUMMARY_BUDGET;
        Self {
            substitutions: stages
                .iter()
                .map(|stage| {
                    steps_of(
                        tables.gsub,
                        &substitution_features,
                        stage,
                        true,
                        &mut budget,
                    )
                })
                .collect(),
            older_way: !newer_way,
            // A font with nothing for any script is taken to need nothing for this one, and to mean it.
            knows_script: tables.gsub.is_none_or(|table| {
                scripts.iter().any(|tag| has_script(table, tag))
                    || ![b"DFLT", b"dflt", b"latn"]
                        .iter()
                        .any(|tag| has_script(table, tag))
            }),
            positions: steps_of(
                tables.gpos,
                &position_features,
                positions,
                false,
                &mut budget,
            ),
            has_kerning: position_features.iter().any(|(tag, _)| tag == b"kern"),
        }
    }
}

/// What a feature's single substitutions make of each of these glyphs, as `tnum` makes a digit of one of the same width as
/// every other digit. What it doesn't change, or a font without it, stays as it is.
pub(crate) fn single_substitutes(table: Option<&[u8]>, feature: &[u8; 4], glyphs: &[u16]) -> Vec<u16> {
    let mut out = glyphs.to_vec();
    let Some(table) = table else {
        return out;
    };
    let (Some(features), Some(lookup_list)) = (features_of(table, &[b"latn"]), read_u16(table, 8)) else {
        return out;
    };
    let lookup_list = lookup_list as usize;
    let lookup_count = read_u16(table, lookup_list).unwrap_or(0);
    for (tag, lookups) in &features {
        if tag != feature {
            continue;
        }
        for &lookup in lookups {
            if lookup >= lookup_count {
                continue;
            }
            let Some(at) = read_u16(table, lookup_list + 2 + lookup as usize * 2) else {
                continue;
            };
            let lookup = lookup_list + at as usize;
            let (Some(kind), Some(count)) = (read_u16(table, lookup), read_u16(table, lookup + 4)) else {
                continue;
            };
            for index in 0..count as usize {
                let Some(at) = read_u16(table, lookup + 6 + index * 2) else {
                    break;
                };
                let mut subtable = lookup + at as usize;
                let mut subtable_kind = kind;
                if kind == 7 {
                    let (Some(inner), Some(offset)) = (read_u16(table, subtable + 2), read_u32(table, subtable + 4)) else {
                        continue;
                    };
                    subtable_kind = inner;
                    subtable += offset as usize;
                }
                if subtable_kind != 1 {
                    continue;
                }
                let (Some(format), Some(coverage)) = (read_u16(table, subtable), read_u16(table, subtable + 2)) else {
                    continue;
                };
                let coverage = subtable + coverage as usize;
                for glyph in &mut out {
                    let Some(covered) = coverage_index(table, coverage, *glyph) else {
                        continue;
                    };
                    let made = match format {
                        1 => read_i16(table, subtable + 4).map(|delta| (i32::from(*glyph) + i32::from(delta)) as u16),
                        2 if covered < read_u16(table, subtable + 4).unwrap_or(0) as usize => {
                            read_u16(table, subtable + 6 + covered * 2)
                        }
                        _ => None,
                    };
                    if let Some(made) = made {
                        *glyph = made;
                    }
                }
            }
        }
    }
    out
}

/// What ASCII text can do in a font, to tell if plain text can skip shaping altogether.
#[derive(Debug, Default)]
pub(crate) struct AsciiRules {
    /// Pairs of ASCII characters that begin a ligature, in order.
    pub(crate) pairs: Vec<(u8, u8)>,
    /// Which characters begin one of those, as bits.
    pub(crate) firsts: u128,
    /// Whether rules that look at context apply to ASCII, so that any text might change.
    pub(crate) contextual: bool,
}

impl AsciiRules {
    pub(crate) fn read(table: Option<&[u8]>, ascii_glyphs: &[u16; 128]) -> Self {
        Self::read_inner(table, ascii_glyphs).unwrap_or_default()
    }

    fn read_inner(table: Option<&[u8]>, ascii_glyphs: &[u16; 128]) -> Option<Self> {
        let table = table?;
        let features = features_of(table, &[b"latn"])?;
        let wanted: &[(&[u8; 4], u16)] = &[
            (b"rlig", MASK_ALL),
            (b"liga", MASK_ALL),
            (b"clig", MASK_ALL),
            (b"calt", MASK_ALL),
            (b"rclt", MASK_ALL),
        ];
        // These get read through directly, so nothing needs summarizing.
        let steps = steps_of(None, &features, wanted, true, &mut 0);
        let mut rules = Self::default();
        let lookup_list = read_u16(table, 8)? as usize;
        let lookup_count = read_u16(table, lookup_list)?;
        for step in steps {
            if step.lookup >= lookup_count {
                continue;
            }
            let lookup =
                lookup_list + read_u16(table, lookup_list + 2 + step.lookup as usize * 2)? as usize;
            let kind = read_u16(table, lookup)?;
            let count = read_u16(table, lookup + 4)? as usize;
            for index in 0..count {
                let mut subtable = lookup + read_u16(table, lookup + 6 + index * 2)? as usize;
                let mut subtable_kind = kind;
                if kind == 7 {
                    subtable_kind = read_u16(table, subtable + 2)?;
                    subtable += read_u32(table, subtable + 4)? as usize;
                }
                match subtable_kind {
                    4 => rules.read_ligatures(table, subtable, ascii_glyphs),
                    // Anything else here changes glyphs by what's around them, so ask whether
                    // it could start from an ASCII character. Format 3 lists its first coverage
                    // differently, so it counts whatever it covers.
                    5 | 6 | 1 | 2 | 3 | 8
                        if subtable_touches_ascii(table, subtable, subtable_kind, ascii_glyphs) =>
                    {
                        rules.contextual = true;
                    }
                    _ => {}
                }
            }
        }
        rules.pairs.sort_unstable();
        rules.pairs.dedup();
        Some(rules)
    }

    fn read_ligatures(&mut self, table: &[u8], subtable: usize, ascii_glyphs: &[u16; 128]) {
        let Some(coverage) = read_u16(table, subtable + 2) else {
            return;
        };
        let coverage = subtable + coverage as usize;
        let Some(sets) = read_u16(table, subtable + 4) else {
            return;
        };
        for (first, glyph) in ascii_glyphs.iter().enumerate() {
            if *glyph == 0 {
                continue;
            }
            let Some(covered) = coverage_index(table, coverage, *glyph) else {
                continue;
            };
            if covered >= sets as usize {
                continue;
            }
            let Some(set) = read_u16(table, subtable + 6 + covered * 2) else {
                continue;
            };
            let set = subtable + set as usize;
            let Some(count) = read_u16(table, set) else {
                continue;
            };
            for index in 0..count as usize {
                let Some(offset) = read_u16(table, set + 2 + index * 2) else {
                    break;
                };
                let ligature = set + offset as usize;
                let Some(second) = read_u16(table, ligature + 4) else {
                    continue;
                };
                if let Some(second) = ascii_glyphs.iter().position(|glyph| *glyph == second) {
                    self.pairs.push((first as u8, second as u8));
                    self.firsts |= 1 << first;
                }
            }
        }
    }
}

fn subtable_touches_ascii(
    table: &[u8],
    subtable: usize,
    kind: u16,
    ascii_glyphs: &[u16; 128],
) -> bool {
    let format = read_u16(table, subtable).unwrap_or(0);
    let coverage = match (kind, format) {
        // These start with a list of coverages, rather than one.
        (5, 3) => read_u16(table, subtable + 6).map(|at| subtable + at as usize),
        (6, 3) => first_chain_coverage(table, subtable),
        _ => read_u16(table, subtable + 2).map(|at| subtable + at as usize),
    };
    let Some(coverage) = coverage else {
        return true;
    };
    ascii_glyphs
        .iter()
        .any(|glyph| *glyph != 0 && coverage_index(table, coverage, *glyph).is_some())
}

fn first_chain_coverage(table: &[u8], subtable: usize) -> Option<usize> {
    let backtrack = read_u16(table, subtable + 2)? as usize;
    let input = subtable + 4 + backtrack * 2;
    let at = read_u16(table, input + 2)?;
    Some(subtable + at as usize)
}

#[derive(Clone, Copy)]
struct Flags {
    flag: u16,
    filter: u16,
}

/// How a rule matches each glyph in a sequence.
#[derive(Clone, Copy)]
enum Matcher {
    /// The numbers are glyphs.
    Glyph,
    /// The numbers are classes, in this class table.
    Class(usize),
    /// The numbers are offsets to coverage tables, from this position.
    Coverage(usize),
}

struct Shaper<'a, 'b> {
    table: &'a [u8],
    gdef: Option<&'a [u8]>,
    slots: &'b mut Vec<Slot>,
    budget: u32,
    max_slots: usize,
    next_ligature: u16,
    right_to_left: bool,
    substituting: bool,
    /// Whether rules can only see what's in the syllable they started in.
    within_syllable: bool,
    /// Whether joiners are something a rule sees, rather than something it looks past.
    sees_joiners: bool,
    /// Which glyphs the feature being applied is for. Everything a rule changes has to be one of them.
    mask: u16,
}

/// Works out each glyph's class, for rules that go by what kind of glyph it is.
pub(crate) fn set_classes(tables: Tables<'_>, slots: &mut [Slot]) {
    if has_classes(tables.gdef) {
        for slot in slots.iter_mut() {
            slot.class = class_of_glyph(tables.gdef, slot.glyph);
        }
    }
}

/// Applies a plan's substitutions to these glyphs.
pub(crate) fn substitute(tables: Tables<'_>, plan: &Plan, slots: &mut Vec<Slot>) {
    set_classes(tables, slots);
    let stages: Vec<&[Step]> = plan.substitutions.iter().map(Vec::as_slice).collect();
    let mut next_ligature = 1;
    substitute_stages(tables, &stages, slots, false, &mut next_ligature);
}

/// Applies some stages of substitutions, one after the other, within each syllable if `in_syllables`.
pub(crate) fn substitute_stages(
    tables: Tables<'_>,
    stages: &[&[Step]],
    slots: &mut Vec<Slot>,
    in_syllables: bool,
    next_ligature: &mut u16,
) {
    let Some(table) = tables.gsub else {
        return;
    };
    let budget = (slots.len() as u32 + 8).saturating_mul(BUDGET_PER_GLYPH);
    let max_slots = (slots.len() + 8).saturating_mul(MAX_GROWTH);
    let mut shaper = Shaper {
        table,
        gdef: tables.gdef,
        slots,
        budget,
        max_slots,
        next_ligature: *next_ligature,
        right_to_left: false,
        substituting: true,
        within_syllable: in_syllables,
        sees_joiners: in_syllables,
        mask: MASK_ALL,
    };
    for stage in stages {
        for step in *stage {
            shaper.run_lookup(*step);
        }
    }
    *next_ligature = shaper.next_ligature;
}

/// Whether a feature has a rule for exactly these glyphs in this order, whatever it would make of them.
/// A rule that also depends on what's around them only counts if `around_too`.
pub(crate) fn would_substitute(
    tables: Tables<'_>,
    steps: &[Step],
    glyphs: &[u16],
    around_too: bool,
) -> bool {
    let Some(table) = tables.gsub else {
        return false;
    };
    let mut budget = WOULD_SUBSTITUTE_BUDGET;
    steps.iter().any(|step| {
        lookup_would_apply(table, step.lookup, glyphs, around_too, &mut budget) == Some(true)
    })
}

/// How many rules to look through for one question, so that a bad font can't take forever.
const WOULD_SUBSTITUTE_BUDGET: u32 = 100_000;

fn lookup_would_apply(
    table: &[u8],
    lookup: u16,
    glyphs: &[u16],
    around_too: bool,
    budget: &mut u32,
) -> Option<bool> {
    let list = read_u16(table, 8)? as usize;
    if lookup >= read_u16(table, list)? {
        return None;
    }
    let lookup = list + read_u16(table, list + 2 + lookup as usize * 2)? as usize;
    let lookup_kind = read_u16(table, lookup)?;
    for at in 0..read_u16(table, lookup + 4)? as usize {
        let mut subtable = lookup + read_u16(table, lookup + 6 + at * 2)? as usize;
        let mut kind = lookup_kind;
        if kind == 7 {
            kind = read_u16(table, subtable + 2)?;
            subtable += read_u32(table, subtable + 4)? as usize;
        }
        let found = match kind {
            4 => ligature_would_apply(table, subtable, glyphs, budget),
            5 => context_would_apply(table, subtable, glyphs, budget),
            6 => chain_would_apply(table, subtable, glyphs, around_too, budget),
            _ => None,
        };
        if found == Some(true) {
            return Some(true);
        }
    }
    Some(false)
}

/// Counts a rule looked through, and says if there's any looking left to do.
fn spend_from(budget: &mut u32) -> Option<()> {
    *budget = budget.checked_sub(1)?;
    Some(())
}

fn ligature_would_apply(
    table: &[u8],
    subtable: usize,
    glyphs: &[u16],
    budget: &mut u32,
) -> Option<bool> {
    let coverage = subtable + read_u16(table, subtable + 2)? as usize;
    let covered = coverage_index(table, coverage, *glyphs.first()?)?;
    if covered >= read_u16(table, subtable + 4)? as usize {
        return None;
    }
    let set = subtable + read_u16(table, subtable + 6 + covered * 2)? as usize;
    for at in 0..read_u16(table, set)? as usize {
        spend_from(budget)?;
        let ligature = set + read_u16(table, set + 2 + at * 2)? as usize;
        if input_would_match(
            table,
            Matcher::Glyph,
            ligature + 4,
            read_u16(table, ligature + 2)? as usize,
            glyphs,
        )? {
            return Some(true);
        }
    }
    Some(false)
}

/// Whether numbers at `items` say the same as every glyph after the first.
fn input_would_match(
    table: &[u8],
    matcher: Matcher,
    items: usize,
    count: usize,
    glyphs: &[u16],
) -> Option<bool> {
    if count != glyphs.len() {
        return Some(false);
    }
    for (item, glyph) in glyphs.iter().skip(1).enumerate() {
        if !rule_matches(table, matcher, read_u16(table, items + item * 2)?, *glyph) {
            return Some(false);
        }
    }
    Some(true)
}

fn context_would_apply(
    table: &[u8],
    subtable: usize,
    glyphs: &[u16],
    budget: &mut u32,
) -> Option<bool> {
    let first = *glyphs.first()?;
    match read_u16(table, subtable)? {
        format @ (1 | 2) => {
            let coverage = subtable + read_u16(table, subtable + 2)? as usize;
            let covered = coverage_index(table, coverage, first)?;
            let (matcher, set_list, chosen) = if format == 1 {
                (Matcher::Glyph, subtable + 4, covered)
            } else {
                let classes = subtable + read_u16(table, subtable + 4)? as usize;
                (
                    Matcher::Class(classes),
                    subtable + 6,
                    glyph_class(table, classes, first),
                )
            };
            if chosen >= read_u16(table, set_list)? as usize {
                return None;
            }
            let set = read_u16(table, set_list + 2 + chosen * 2)? as usize;
            if set == 0 {
                return None;
            }
            let set = subtable + set;
            for rule in 0..read_u16(table, set)? as usize {
                spend_from(budget)?;
                let rule = set + read_u16(table, set + 2 + rule * 2)? as usize;
                let count = read_u16(table, rule)? as usize;
                if input_would_match(table, matcher, rule + 4, count, glyphs)? {
                    return Some(true);
                }
            }
            Some(false)
        }
        3 => {
            let count = read_u16(table, subtable + 2)? as usize;
            let first_cover = subtable + read_u16(table, subtable + 6)? as usize;
            coverage_index(table, first_cover, first)?;
            input_would_match(
                table,
                Matcher::Coverage(subtable),
                subtable + 8,
                count,
                glyphs,
            )
        }
        _ => None,
    }
}

fn chain_would_apply(
    table: &[u8],
    subtable: usize,
    glyphs: &[u16],
    around_too: bool,
    budget: &mut u32,
) -> Option<bool> {
    let first = *glyphs.first()?;
    match read_u16(table, subtable)? {
        format @ (1 | 2) => {
            let coverage = subtable + read_u16(table, subtable + 2)? as usize;
            let covered = coverage_index(table, coverage, first)?;
            let (matcher, set_list, chosen) = if format == 1 {
                (Matcher::Glyph, subtable + 4, covered)
            } else {
                let offset = read_u16(table, subtable + 6)? as usize;
                let classes = if offset == 0 { 0 } else { subtable + offset };
                let chosen = if classes == 0 {
                    0
                } else {
                    glyph_class(table, classes, first)
                };
                (Matcher::Class(classes), subtable + 10, chosen)
            };
            if chosen >= read_u16(table, set_list)? as usize {
                return None;
            }
            let set = read_u16(table, set_list + 2 + chosen * 2)? as usize;
            if set == 0 {
                return None;
            }
            let set = subtable + set;
            for rule in 0..read_u16(table, set)? as usize {
                spend_from(budget)?;
                let rule = set + read_u16(table, set + 2 + rule * 2)? as usize;
                let behind = read_u16(table, rule)? as usize;
                let input_at = rule + 2 + behind * 2;
                let count = read_u16(table, input_at)? as usize;
                let ahead_at = input_at + 2 + count.saturating_sub(1) * 2;
                let ahead = read_u16(table, ahead_at)? as usize;
                if (around_too || (behind == 0 && ahead == 0))
                    && input_would_match(table, matcher, input_at + 2, count, glyphs)?
                {
                    return Some(true);
                }
            }
            Some(false)
        }
        3 => {
            let behind = read_u16(table, subtable + 2)? as usize;
            let input_at = subtable + 4 + behind * 2;
            let count = read_u16(table, input_at)? as usize;
            let ahead_at = input_at + 2 + count * 2;
            let ahead = read_u16(table, ahead_at)? as usize;
            if !around_too && (behind != 0 || ahead != 0) {
                return Some(false);
            }
            let first_cover = subtable + read_u16(table, input_at + 2)? as usize;
            coverage_index(table, first_cover, first)?;
            input_would_match(
                table,
                Matcher::Coverage(subtable),
                input_at + 4,
                count,
                glyphs,
            )
        }
        _ => None,
    }
}

/// Whether a number from a rule means this glyph.
fn rule_matches(table: &[u8], matcher: Matcher, value: u16, glyph: u16) -> bool {
    match matcher {
        Matcher::Glyph => value == glyph,
        // A rule can leave out the class table, which puts everything in class 0.
        Matcher::Class(0) => value == 0,
        Matcher::Class(classes) => glyph_class(table, classes, glyph) == value as usize,
        Matcher::Coverage(base) => coverage_index(table, base + value as usize, glyph).is_some(),
    }
}

/// Applies a plan's positioning to these glyphs, whose advances are already set.
pub(crate) fn position(
    tables: Tables<'_>,
    plan: &Plan,
    slots: &mut Vec<Slot>,
    right_to_left: bool,
    marks_take_no_room: bool,
) {
    // Marks sit on what's before them, and take up no room of their own. Fonts for Indic scripts already
    // say how much room each takes.
    if marks_take_no_room {
        for slot in slots.iter_mut() {
            if slot.class == CLASS_MARK {
                slot.x_advance = 0;
            }
        }
    }
    let Some(table) = tables.gpos else {
        return;
    };
    let budget = (slots.len() as u32 + 8).saturating_mul(BUDGET_PER_GLYPH);
    let max_slots = slots.len();
    let mut shaper = Shaper {
        table,
        gdef: tables.gdef,
        slots,
        budget,
        max_slots,
        next_ligature: 0,
        right_to_left,
        substituting: false,
        mask: MASK_ALL,
        within_syllable: false,
        sees_joiners: false,
    };
    for step in &plan.positions {
        shaper.run_lookup(*step);
    }
    resolve_attachments(slots, right_to_left);
}

/// Attached glyphs were placed relative to what they're attached to. Their offsets become relative
/// to where the pen will be when they're reached.
fn resolve_attachments(slots: &mut [Slot], right_to_left: bool) {
    for index in 0..slots.len() {
        resolve_attachment(slots, index, right_to_left, MAX_ATTACHMENT_CHAIN);
    }
}

fn resolve_attachment(slots: &mut [Slot], index: usize, right_to_left: bool, depth: u32) {
    let chain = slots[index].attached;
    if chain == 0 || depth == 0 {
        return;
    }
    slots[index].attached = 0;
    let Some(parent) = index
        .checked_add_signed(chain as isize)
        .filter(|at| *at < slots.len())
    else {
        return;
    };
    resolve_attachment(slots, parent, right_to_left, depth - 1);
    slots[index].y_offset += slots[parent].y_offset;
    if !slots[index].attached_as_mark {
        return;
    }
    slots[index].x_offset += slots[parent].x_offset;
    if parent < index {
        let between: i32 = if right_to_left {
            slots[parent + 1..=index]
                .iter()
                .map(|slot| slot.x_advance)
                .sum()
        } else {
            -slots[parent..index]
                .iter()
                .map(|slot| slot.x_advance)
                .sum::<i32>()
        };
        slots[index].x_offset += between;
    }
}

/// Whether a font says which of its glyphs are marks.
fn has_classes(gdef: Option<&[u8]>) -> bool {
    gdef.and_then(|gdef| read_u16(gdef, 4))
        .is_some_and(|at| at != 0)
}

fn class_of_glyph(gdef: Option<&[u8]>, glyph: u16) -> u8 {
    let Some(gdef) = gdef else {
        return 0;
    };
    let Some(table) = read_u16(gdef, 4).filter(|at| *at != 0) else {
        return 0;
    };
    glyph_class(gdef, table as usize, glyph) as u8
}

impl Shaper<'_, '_> {
    fn spend(&mut self) -> bool {
        match self.budget.checked_sub(1) {
            Some(left) => {
                self.budget = left;
                true
            }
            None => false,
        }
    }

    fn lookup_at(&self, lookup: u16) -> Option<usize> {
        let list = read_u16(self.table, 8)? as usize;
        if lookup >= read_u16(self.table, list)? {
            return None;
        }
        Some(list + read_u16(self.table, list + 2 + lookup as usize * 2)? as usize)
    }

    fn flags_of(&self, lookup: usize) -> Flags {
        let flag = read_u16(self.table, lookup + 2).unwrap_or(0);
        let count = read_u16(self.table, lookup + 4).unwrap_or(0) as usize;
        let filter = if flag & USE_FILTER_SET != 0 {
            read_u16(self.table, lookup + 6 + count * 2).unwrap_or(0)
        } else {
            0
        };
        Flags { flag, filter }
    }

    /// Whether a rule with these flags acts as if this glyph isn't there.
    fn skips(&self, index: usize, flags: Flags) -> bool {
        let slot = &self.slots[index];
        // A non-joiner only matters for choosing glyphs. Kerning looks straight past it.
        if slot.mask & MASK_INVISIBLE != 0 || (!self.substituting && slot.mask & MASK_HIDDEN != 0) {
            return true;
        }
        self.skips_by_kind(slot, flags)
    }

    /// Whether a rule that starts here should pass over this glyph. Rules can start on what doesn't show,
    /// as when a joiner and the emoji after it become one.
    #[inline]
    fn skips_starting(&self, index: usize, flags: Flags) -> bool {
        if self.substituting {
            self.skips_by_kind(&self.slots[index], flags)
        } else {
            self.skips(index, flags)
        }
    }

    /// Whether a rule with these flags has been told to ignore this kind of glyph.
    fn skips_by_kind(&self, slot: &Slot, flags: Flags) -> bool {
        match slot.class {
            1 if flags.flag & IGNORE_BASE != 0 => true,
            CLASS_LIGATURE if flags.flag & IGNORE_LIGATURE != 0 => true,
            CLASS_MARK => {
                if flags.flag & IGNORE_MARK != 0 {
                    return true;
                }
                if flags.flag & USE_FILTER_SET != 0 {
                    return !self.in_mark_set(flags.filter, slot.glyph);
                }
                let wanted = flags.flag >> 8;
                wanted != 0 && self.mark_attachment_class(slot.glyph) != wanted
            }
            _ => false,
        }
    }

    fn mark_attachment_class(&self, glyph: u16) -> u16 {
        let Some(gdef) = self.gdef else {
            return 0;
        };
        match read_u16(gdef, 10) {
            Some(table) if table != 0 => glyph_class(gdef, table as usize, glyph) as u16,
            _ => 0,
        }
    }

    fn in_mark_set(&self, set: u16, glyph: u16) -> bool {
        self.in_mark_set_inner(set, glyph).unwrap_or(false)
    }

    fn in_mark_set_inner(&self, set: u16, glyph: u16) -> Option<bool> {
        let gdef = self.gdef?;
        // Mark sets came in with version 1.2.
        if read_u16(gdef, 2)? < 2 {
            return None;
        }
        let sets = read_u16(gdef, 12)? as usize;
        if sets == 0 || set >= read_u16(gdef, sets + 2)? {
            return None;
        }
        let coverage = sets + read_u32(gdef, sets + 4 + set as usize * 4)? as usize;
        Some(coverage_index(gdef, coverage, glyph).is_some())
    }

    /// Whether a rule that started at `from` can see the glyph at `at`.
    #[inline]
    fn can_see(&self, from: usize, at: usize) -> bool {
        !self.within_syllable || self.slots[from].syllable == self.slots[at].syllable
    }

    /// The next glyph a rule sees, which is a glyph that's normally skipped if it's what the rule wants.
    fn next_wanted(
        &self,
        index: usize,
        flags: Flags,
        wanted: impl Fn(u16) -> bool,
    ) -> Option<usize> {
        (index + 1..self.slots.len())
            .find(|at| {
                let slot = &self.slots[*at];
                !self.skips(*at, flags) || (slot.mask & MASK_INVISIBLE != 0 && wanted(slot.glyph))
            })
            .filter(|at| self.can_see(index, *at))
    }

    fn next_after(&self, index: usize, flags: Flags) -> Option<usize> {
        (index + 1..self.slots.len())
            .find(|at| !self.skips(*at, flags))
            .filter(|at| self.can_see(index, *at))
    }

    fn previous_before(&self, index: usize, flags: Flags) -> Option<usize> {
        (0..index)
            .rev()
            .find(|at| !self.skips(*at, flags))
            .filter(|at| self.can_see(index, *at))
    }

    /// Whether what a rule only looks at, without changing it, acts as if this glyph isn't there. A joiner
    /// or anything else that doesn't show is passed by unless it's what the rule is looking for, which
    /// `matched` says, as with the parts of an emoji. A non-joiner keeps letters from joining, but isn't
    /// usually what's being looked for around them either.
    fn passes_by(&self, index: usize, flags: Flags, matched: impl FnOnce() -> bool) -> bool {
        let mask = self.slots[index].mask;
        if mask & (MASK_JOINER | MASK_INVISIBLE) != 0
            || (!self.sees_joiners && mask & MASK_NON_JOINER != 0)
        {
            return !matched();
        }
        self.skips(index, flags)
    }

    /// Runs one lookup across all the glyphs.
    fn run_lookup(&mut self, step: Step) {
        let Some(lookup) = self.lookup_at(step.lookup) else {
            return;
        };
        self.mask = step.mask;
        let kind = read_u16(self.table, lookup).unwrap_or(0);
        let flags = self.flags_of(lookup);
        let reversed = self.substituting
            && (kind == 8
                || (kind == 7
                    && read_u16(self.table, lookup + 6)
                        .and_then(|at| read_u16(self.table, lookup + at as usize + 2))
                        == Some(8)));
        if reversed {
            // This one runs from the end, so that what it's changed can't confuse it.
            let mut index = self.slots.len();
            while index > 0 {
                index -= 1;
                if !self.spend() {
                    return;
                }
                if self.slots[index].mask & step.mask != 0
                    && step.summary.may_have(self.slots[index].glyph)
                    && !self.skips_starting(index, flags)
                {
                    self.try_subtables(lookup, kind, index, flags, 0);
                }
            }
            return;
        }
        let mut index = 0;
        while index < self.slots.len() {
            if !self.spend() {
                return;
            }
            if self.slots[index].mask & step.mask != 0
                && step.summary.may_have(self.slots[index].glyph)
                && !self.skips_starting(index, flags)
            {
                if let Some(next) = self.try_subtables(lookup, kind, index, flags, 0) {
                    index = next.max(index + 1);
                    continue;
                }
            }
            index += 1;
        }
    }

    /// Tries each subtable at this glyph until one applies, and says where to go on from.
    fn try_subtables(
        &mut self,
        lookup: usize,
        kind: u16,
        index: usize,
        flags: Flags,
        depth: u32,
    ) -> Option<usize> {
        let count = read_u16(self.table, lookup + 4)? as usize;
        for at in 0..count {
            if !self.spend() {
                return None;
            }
            let mut subtable = lookup + read_u16(self.table, lookup + 6 + at * 2)? as usize;
            let mut subtable_kind = kind;
            let extension = if self.substituting { 7 } else { 9 };
            if kind == extension {
                subtable_kind = read_u16(self.table, subtable + 2)?;
                subtable += read_u32(self.table, subtable + 4)? as usize;
            }
            let result = if self.substituting {
                self.substitute_at(subtable, subtable_kind, index, flags, depth)
            } else {
                self.position_at(subtable, subtable_kind, index, flags, depth)
            };
            if result.is_some() {
                return result;
            }
        }
        None
    }

    fn set_glyph(&mut self, index: usize, glyph: u16) {
        self.slots[index].glyph = glyph;
        // A joiner the font has turned into something is meant to be seen as that.
        self.slots[index].mask &= !MASK_HIDDEN;
        self.slots[index].history |= WAS_SUBSTITUTED;
        if has_classes(self.gdef) {
            self.slots[index].class = class_of_glyph(self.gdef, glyph);
        }
    }

    fn substitute_at(
        &mut self,
        subtable: usize,
        kind: u16,
        index: usize,
        flags: Flags,
        depth: u32,
    ) -> Option<usize> {
        let table = self.table;
        match kind {
            1 => {
                let glyph = self.slots[index].glyph;
                let coverage = subtable + read_u16(table, subtable + 2)? as usize;
                let covered = coverage_index(table, coverage, glyph)?;
                let new_glyph = match read_u16(table, subtable)? {
                    1 => glyph.wrapping_add(read_u16(table, subtable + 4)?),
                    2 => {
                        if covered >= read_u16(table, subtable + 4)? as usize {
                            return None;
                        }
                        read_u16(table, subtable + 6 + covered * 2)?
                    }
                    _ => return None,
                };
                self.set_glyph(index, new_glyph);
                Some(index + 1)
            }
            2 | 3 => {
                let glyph = self.slots[index].glyph;
                let coverage = subtable + read_u16(table, subtable + 2)? as usize;
                let covered = coverage_index(table, coverage, glyph)?;
                if covered >= read_u16(table, subtable + 4)? as usize {
                    return None;
                }
                let sequence = subtable + read_u16(table, subtable + 6 + covered * 2)? as usize;
                let count = read_u16(table, sequence)? as usize;
                if kind == 3 {
                    // Alternates: the first one is the default.
                    if count == 0 {
                        return None;
                    }
                    let new_glyph = read_u16(table, sequence + 2)?;
                    self.set_glyph(index, new_glyph);
                    return Some(index + 1);
                }
                if self.slots.len() + count > self.max_slots {
                    return None;
                }
                if count == 0 {
                    // Removing a glyph entirely isn't something a font can ask for.
                    return None;
                }
                let first = self.slots[index];
                let mut replacement = Vec::with_capacity(count);
                for at in 0..count {
                    let mut slot = first;
                    slot.glyph = read_u16(table, sequence + 2 + at * 2)?;
                    slot.history |= WAS_SUBSTITUTED | WAS_MULTIPLIED;
                    slot.component = at.min(255) as u8;
                    if has_classes(self.gdef) {
                        slot.class = class_of_glyph(self.gdef, slot.glyph);
                    }
                    replacement.push(slot);
                }
                self.slots.splice(index..=index, replacement);
                Some(index + count)
            }
            4 => self.form_ligature(subtable, index, flags),
            5 => self.match_context(subtable, index, flags, depth),
            6 => self.match_chain(subtable, index, flags, depth),
            8 => self.reverse_chain(subtable, index, flags),
            _ => None,
        }
    }

    fn form_ligature(&mut self, subtable: usize, index: usize, flags: Flags) -> Option<usize> {
        let table = self.table;
        let glyph = self.slots[index].glyph;
        let coverage = subtable + read_u16(table, subtable + 2)? as usize;
        let covered = coverage_index(table, coverage, glyph)?;
        if covered >= read_u16(table, subtable + 4)? as usize {
            return None;
        }
        let set = subtable + read_u16(table, subtable + 6 + covered * 2)? as usize;
        let count = read_u16(table, set)? as usize;
        'ligatures: for at in 0..count {
            if !self.spend() {
                return None;
            }
            let ligature = set + read_u16(table, set + 2 + at * 2)? as usize;
            let new_glyph = read_u16(table, ligature)?;
            let parts = read_u16(table, ligature + 2)? as usize;
            if parts == 0 || parts > MAX_RULE_LENGTH {
                continue;
            }
            let mut positions = [0usize; MAX_RULE_LENGTH];
            positions[0] = index;
            let mut last = index;
            for (part, position) in positions.iter_mut().enumerate().take(parts).skip(1) {
                let wanted = read_u16(table, ligature + 4 + (part - 1) * 2)?;
                let Some(next) = self.next_wanted(last, flags, |glyph| glyph == wanted) else {
                    continue 'ligatures;
                };
                // A non-joiner isn't skipped, so it fails this, which is how it blocks ligatures.
                if self.slots[next].glyph != wanted || self.slots[next].mask & self.mask == 0 {
                    continue 'ligatures;
                }
                *position = next;
                last = next;
            }
            // A letter with marks, or marks together, make one thing that's still only a letter or a
            // mark, with no parts for other marks to choose between.
            let rest_are_marks = positions[1..parts]
                .iter()
                .all(|at| self.slots[*at].class == CLASS_MARK);
            let is_ligature = !rest_are_marks;
            let id = if is_ligature {
                let id = self.next_ligature;
                self.next_ligature = self.next_ligature.wrapping_add(1).max(1);
                id
            } else {
                0
            };
            if is_ligature {
                // A joiner in between is passed over, and marks in between stay, on their part.
                let mut part_number = 0u8;
                let mut remove_next = 1;
                for at in index + 1..=last {
                    if remove_next < parts && positions[remove_next] == at {
                        remove_next += 1;
                        part_number += 1;
                    } else if self.slots[at].class == CLASS_MARK {
                        self.slots[at].ligature = id;
                        self.slots[at].component = part_number + 1;
                    }
                }
            }
            for part in (1..parts).rev() {
                self.slots.remove(positions[part]);
            }
            self.set_glyph(index, new_glyph);
            let slot = &mut self.slots[index];
            slot.ligature = id;
            slot.component = 0;
            slot.history = (slot.history | WAS_LIGATED) & !WAS_MULTIPLIED;
            return Some(index + 1);
        }
        None
    }

    fn reverse_chain(&mut self, subtable: usize, index: usize, flags: Flags) -> Option<usize> {
        let table = self.table;
        let glyph = self.slots[index].glyph;
        let coverage = subtable + read_u16(table, subtable + 2)? as usize;
        let covered = coverage_index(table, coverage, glyph)?;
        let backtrack = read_u16(table, subtable + 4)? as usize;
        let mut at = index;
        for item in 0..backtrack {
            at = self.previous_before(at, flags)?;
            let cover = subtable + read_u16(table, subtable + 6 + item * 2)? as usize;
            coverage_index(table, cover, self.slots[at].glyph)?;
        }
        let lookahead_list = subtable + 6 + backtrack * 2;
        let lookahead = read_u16(table, lookahead_list)? as usize;
        let mut at = index;
        for item in 0..lookahead {
            at = self.next_after(at, flags)?;
            let cover = subtable + read_u16(table, lookahead_list + 2 + item * 2)? as usize;
            coverage_index(table, cover, self.slots[at].glyph)?;
        }
        let list = lookahead_list + 2 + lookahead * 2;
        if covered >= read_u16(table, list)? as usize {
            return None;
        }
        let new_glyph = read_u16(table, list + 2 + covered * 2)?;
        self.set_glyph(index, new_glyph);
        Some(index + 1)
    }

    fn matches(&self, matcher: Matcher, value: u16, glyph: u16) -> bool {
        rule_matches(self.table, matcher, value, glyph)
    }

    /// Matches the glyphs after `index` against `count` numbers at `items`, and fills in where
    /// they are, starting after the first.
    fn match_input(
        &mut self,
        matcher: Matcher,
        items: usize,
        count: usize,
        index: usize,
        flags: Flags,
        positions: &mut [usize; MAX_RULE_LENGTH],
    ) -> Option<usize> {
        if count == 0 || count > MAX_RULE_LENGTH {
            return None;
        }
        positions[0] = index;
        let mut last = index;
        for (item, position) in positions.iter_mut().enumerate().take(count).skip(1) {
            if !self.spend() {
                return None;
            }
            let value = read_u16(self.table, items + (item - 1) * 2)?;
            last = self.next_wanted(last, flags, |glyph| self.matches(matcher, value, glyph))?;
            if self.slots[last].mask & self.mask == 0
                || !self.matches(matcher, value, self.slots[last].glyph)
            {
                return None;
            }
            *position = last;
        }
        Some(last)
    }

    fn match_ahead(
        &mut self,
        matcher: Matcher,
        items: usize,
        count: usize,
        from: usize,
        flags: Flags,
    ) -> Option<()> {
        let mut at = from;
        for item in 0..count {
            if !self.spend() {
                return None;
            }
            let value = read_u16(self.table, items + item * 2)?;
            let wanted =
                |this: &Self, at: usize| this.matches(matcher, value, this.slots[at].glyph);
            at = (at + 1..self.slots.len())
                .find(|at| !self.passes_by(*at, flags, || wanted(self, *at)))
                .filter(|found| self.can_see(from, *found))?;
            if !wanted(self, at) {
                return None;
            }
        }
        Some(())
    }

    fn match_behind(
        &mut self,
        matcher: Matcher,
        items: usize,
        count: usize,
        from: usize,
        flags: Flags,
    ) -> Option<()> {
        let mut at = from;
        for item in 0..count {
            if !self.spend() {
                return None;
            }
            let value = read_u16(self.table, items + item * 2)?;
            let wanted =
                |this: &Self, at: usize| this.matches(matcher, value, this.slots[at].glyph);
            at = (0..at)
                .rev()
                .find(|at| !self.passes_by(*at, flags, || wanted(self, *at)))
                .filter(|found| self.can_see(from, *found))?;
            if !wanted(self, at) {
                return None;
            }
        }
        Some(())
    }

    /// Runs the lookups a matched rule calls for, each at one of the matched glyphs.
    fn run_records(
        &mut self,
        records: usize,
        count: usize,
        positions: &mut [usize; MAX_RULE_LENGTH],
        matched: usize,
        depth: u32,
    ) -> Option<usize> {
        if depth >= MAX_NESTING {
            return Some(positions[matched - 1] + 1);
        }
        let mut last = positions[matched - 1];
        // How many glyphs there are in the rule so far. Numbers in a rule go by how things stand after
        // what it's already done, not by what it started with.
        let mut matched = matched;
        for record in 0..count {
            if !self.spend() {
                break;
            }
            let sequence = read_u16(self.table, records + record * 4)? as usize;
            let lookup_index = read_u16(self.table, records + record * 4 + 2)?;
            if sequence >= matched {
                continue;
            }
            let at = positions[sequence];
            let Some(lookup) = self.lookup_at(lookup_index) else {
                continue;
            };
            let kind = read_u16(self.table, lookup).unwrap_or(0);
            let flags = self.flags_of(lookup);
            let before = self.slots.len();
            if at < before {
                self.try_subtables(lookup, kind, at, flags, depth + 1);
            }
            // What came after moves along if that changed how many glyphs there are.
            let change = self.slots.len() as isize - before as isize;
            if change == 0 {
                continue;
            }
            last = last.saturating_add_signed(change);
            let next = sequence + 1;
            if change > 0 {
                // New glyphs come right after that one, and are part of the rule now.
                let grown = change as usize;
                if matched + grown > MAX_RULE_LENGTH {
                    break;
                }
                positions.copy_within(next..matched, next + grown);
                for new in 0..grown {
                    positions[next + new] = at + new + 1;
                }
                for position in &mut positions[next + grown..matched + grown] {
                    *position += grown;
                }
                matched += grown;
            } else {
                // The glyphs that went were the ones right after that one.
                let gone = change.unsigned_abs().min(matched - next);
                positions.copy_within(next + gone..matched, next);
                matched -= gone;
                for position in &mut positions[next..matched] {
                    *position = position.saturating_sub(gone);
                }
            }
        }
        Some(last + 1)
    }

    fn match_context(
        &mut self,
        subtable: usize,
        index: usize,
        flags: Flags,
        depth: u32,
    ) -> Option<usize> {
        let table = self.table;
        let glyph = self.slots[index].glyph;
        let mut positions = [0usize; MAX_RULE_LENGTH];
        match read_u16(table, subtable)? {
            format @ (1 | 2) => {
                let coverage = subtable + read_u16(table, subtable + 2)? as usize;
                let covered = coverage_index(table, coverage, glyph)?;
                let (matcher, set_list, chosen) = if format == 1 {
                    (Matcher::Glyph, subtable + 4, covered)
                } else {
                    let classes = subtable + read_u16(table, subtable + 4)? as usize;
                    (
                        Matcher::Class(classes),
                        subtable + 6,
                        glyph_class(table, classes, glyph),
                    )
                };
                if chosen >= read_u16(table, set_list)? as usize {
                    return None;
                }
                let set = read_u16(table, set_list + 2 + chosen * 2)? as usize;
                if set == 0 {
                    return None;
                }
                let set = subtable + set;
                for rule in 0..read_u16(table, set)? as usize {
                    let rule = set + read_u16(table, set + 2 + rule * 2)? as usize;
                    let count = read_u16(table, rule)? as usize;
                    let records = read_u16(table, rule + 2)? as usize;
                    if self
                        .match_input(matcher, rule + 4, count, index, flags, &mut positions)
                        .is_some()
                    {
                        let list = rule + 4 + count.saturating_sub(1) * 2;
                        return self.run_records(list, records, &mut positions, count, depth);
                    }
                }
                None
            }
            3 => {
                let count = read_u16(table, subtable + 2)? as usize;
                let records = read_u16(table, subtable + 4)? as usize;
                if count == 0 {
                    return None;
                }
                let first = subtable + read_u16(table, subtable + 6)? as usize;
                coverage_index(table, first, glyph)?;
                self.match_input(
                    Matcher::Coverage(subtable),
                    subtable + 8,
                    count,
                    index,
                    flags,
                    &mut positions,
                )?;
                let list = subtable + 6 + count * 2;
                self.run_records(list, records, &mut positions, count, depth)
            }
            _ => None,
        }
    }

    fn match_chain(
        &mut self,
        subtable: usize,
        index: usize,
        flags: Flags,
        depth: u32,
    ) -> Option<usize> {
        let table = self.table;
        let glyph = self.slots[index].glyph;
        let mut positions = [0usize; MAX_RULE_LENGTH];
        match read_u16(table, subtable)? {
            format @ (1 | 2) => {
                let coverage = subtable + read_u16(table, subtable + 2)? as usize;
                let covered = coverage_index(table, coverage, glyph)?;
                let (behind, input, ahead, set_list, chosen) = if format == 1 {
                    (
                        Matcher::Glyph,
                        Matcher::Glyph,
                        Matcher::Glyph,
                        subtable + 4,
                        covered,
                    )
                } else {
                    let class_table = |at: usize| -> Option<usize> {
                        let offset = read_u16(table, subtable + at)? as usize;
                        Some(if offset == 0 { 0 } else { subtable + offset })
                    };
                    let classes = class_table(6)?;
                    (
                        Matcher::Class(class_table(4)?),
                        Matcher::Class(classes),
                        Matcher::Class(class_table(8)?),
                        subtable + 10,
                        if classes == 0 {
                            0
                        } else {
                            glyph_class(table, classes, glyph)
                        },
                    )
                };
                if chosen >= read_u16(table, set_list)? as usize {
                    return None;
                }
                let set = read_u16(table, set_list + 2 + chosen * 2)? as usize;
                if set == 0 {
                    return None;
                }
                let set = subtable + set;
                for rule in 0..read_u16(table, set)? as usize {
                    if !self.spend() {
                        return None;
                    }
                    let rule = set + read_u16(table, set + 2 + rule * 2)? as usize;
                    let behind_count = read_u16(table, rule)? as usize;
                    let input_at = rule + 2 + behind_count * 2;
                    let input_count = read_u16(table, input_at)? as usize;
                    let ahead_at = input_at + 2 + input_count.saturating_sub(1) * 2;
                    let ahead_count = read_u16(table, ahead_at)? as usize;
                    let records_at = ahead_at + 2 + ahead_count * 2;
                    let records = read_u16(table, records_at)? as usize;
                    let Some(last) = self.match_input(
                        input,
                        input_at + 2,
                        input_count,
                        index,
                        flags,
                        &mut positions,
                    ) else {
                        continue;
                    };
                    if self
                        .match_behind(behind, rule + 2, behind_count, index, flags)
                        .is_none()
                        || self
                            .match_ahead(ahead, ahead_at + 2, ahead_count, last, flags)
                            .is_none()
                    {
                        continue;
                    }
                    return self.run_records(
                        records_at + 2,
                        records,
                        &mut positions,
                        input_count,
                        depth,
                    );
                }
                None
            }
            3 => {
                let behind_count = read_u16(table, subtable + 2)? as usize;
                let input_at = subtable + 4 + behind_count * 2;
                let input_count = read_u16(table, input_at)? as usize;
                if input_count == 0 {
                    return None;
                }
                let first = subtable + read_u16(table, input_at + 2)? as usize;
                coverage_index(table, first, glyph)?;
                let ahead_at = input_at + 2 + input_count * 2;
                let ahead_count = read_u16(table, ahead_at)? as usize;
                let records_at = ahead_at + 2 + ahead_count * 2;
                let records = read_u16(table, records_at)? as usize;
                let cover = Matcher::Coverage(subtable);
                let last = self.match_input(
                    cover,
                    input_at + 4,
                    input_count,
                    index,
                    flags,
                    &mut positions,
                )?;
                self.match_behind(cover, subtable + 4, behind_count, index, flags)?;
                self.match_ahead(cover, ahead_at + 2, ahead_count, last, flags)?;
                self.run_records(records_at + 2, records, &mut positions, input_count, depth)
            }
            _ => None,
        }
    }

    fn position_at(
        &mut self,
        subtable: usize,
        kind: u16,
        index: usize,
        flags: Flags,
        depth: u32,
    ) -> Option<usize> {
        match kind {
            1 => self.single_position(subtable, index),
            2 => self.pair_position(subtable, index, flags),
            3 => self.cursive_position(subtable, index, flags),
            4 => self.mark_on_base(subtable, index),
            5 => self.mark_on_ligature(subtable, index),
            6 => self.mark_on_mark(subtable, index, flags),
            7 => self.match_context(subtable, index, flags, depth),
            8 => self.match_chain(subtable, index, flags, depth),
            _ => None,
        }
    }

    fn apply_value(&mut self, index: usize, record: usize, format: u16) -> Option<()> {
        let table = self.table;
        let mut at = record;
        let mut read = |bit: u16| -> Option<i32> {
            if format & bit == 0 {
                return Some(0);
            }
            let value = read_i16(table, at)?;
            at += 2;
            Some(i32::from(value))
        };
        let (x_place, y_place, x_advance) = (read(1)?, read(2)?, read(4)?);
        let slot = &mut self.slots[index];
        slot.x_offset += x_place;
        slot.y_offset += y_place;
        slot.x_advance += x_advance;
        Some(())
    }

    fn single_position(&mut self, subtable: usize, index: usize) -> Option<usize> {
        let table = self.table;
        let coverage = subtable + read_u16(table, subtable + 2)? as usize;
        let covered = coverage_index(table, coverage, self.slots[index].glyph)?;
        let format = read_u16(table, subtable + 4)?;
        let record = match read_u16(table, subtable)? {
            1 => subtable + 6,
            2 => {
                if covered >= read_u16(table, subtable + 6)? as usize {
                    return None;
                }
                subtable + 8 + covered * value_size(format)
            }
            _ => return None,
        };
        self.apply_value(index, record, format)?;
        Some(index + 1)
    }

    fn pair_position(&mut self, subtable: usize, index: usize, flags: Flags) -> Option<usize> {
        let table = self.table;
        let coverage = subtable + read_u16(table, subtable + 2)? as usize;
        let covered = coverage_index(table, coverage, self.slots[index].glyph)?;
        let second = self.next_after(index, flags)?;
        let format_1 = read_u16(table, subtable + 4)?;
        let format_2 = read_u16(table, subtable + 6)?;
        let (size_1, size_2) = (value_size(format_1), value_size(format_2));
        let second_glyph = self.slots[second].glyph;
        let records = match read_u16(table, subtable)? {
            1 => {
                if covered >= read_u16(table, subtable + 8)? as usize {
                    return None;
                }
                let set = subtable + read_u16(table, subtable + 10 + covered * 2)? as usize;
                let count = read_u16(table, set)? as usize;
                let pair_size = 2 + size_1 + size_2;
                let (mut low, mut high) = (0, count);
                let mut found = None;
                while low < high {
                    let middle = (low + high) / 2;
                    let pair = set + 2 + middle * pair_size;
                    match read_u16(table, pair)?.cmp(&second_glyph) {
                        std::cmp::Ordering::Equal => {
                            found = Some(pair + 2);
                            break;
                        }
                        std::cmp::Ordering::Less => low = middle + 1,
                        std::cmp::Ordering::Greater => high = middle,
                    }
                }
                found?
            }
            2 => {
                let first_classes = subtable + read_u16(table, subtable + 8)? as usize;
                let second_classes = subtable + read_u16(table, subtable + 10)? as usize;
                let class_1 = glyph_class(table, first_classes, self.slots[index].glyph);
                let class_2 = glyph_class(table, second_classes, second_glyph);
                let count_1 = read_u16(table, subtable + 12)? as usize;
                let count_2 = read_u16(table, subtable + 14)? as usize;
                if class_1 >= count_1 || class_2 >= count_2 {
                    return None;
                }
                subtable + 16 + (class_1 * count_2 + class_2) * (size_1 + size_2)
            }
            _ => return None,
        };
        self.apply_value(index, records, format_1)?;
        self.apply_value(second, records + size_1, format_2)?;
        Some(if format_2 != 0 { second + 1 } else { second })
    }

    fn anchor(&self, at: usize) -> Option<(i32, i32)> {
        Some((
            i32::from(read_i16(self.table, at + 2)?),
            i32::from(read_i16(self.table, at + 4)?),
        ))
    }

    fn cursive_position(&mut self, subtable: usize, index: usize, flags: Flags) -> Option<usize> {
        let table = self.table;
        let coverage = subtable + read_u16(table, subtable + 2)? as usize;
        let count = read_u16(table, subtable + 4)? as usize;
        let entry_of = |covered: usize| -> Option<(u16, u16)> {
            if covered >= count {
                return None;
            }
            Some((
                read_u16(table, subtable + 6 + covered * 4)?,
                read_u16(table, subtable + 8 + covered * 4)?,
            ))
        };
        let this = coverage_index(table, coverage, self.slots[index].glyph)?;
        let (entry, _) = entry_of(this)?;
        if entry == 0 {
            return None;
        }
        let previous = self.previous_before(index, flags)?;
        let earlier = coverage_index(table, coverage, self.slots[previous].glyph)?;
        let (_, exit) = entry_of(earlier)?;
        if exit == 0 {
            return None;
        }
        let (entry_x, entry_y) = self.anchor(subtable + entry as usize)?;
        let (exit_x, exit_y) = self.anchor(subtable + exit as usize)?;
        if self.right_to_left {
            let shift = exit_x + self.slots[previous].x_offset;
            self.slots[previous].x_advance -= shift;
            self.slots[previous].x_offset -= shift;
            self.slots[index].x_advance = entry_x + self.slots[index].x_offset;
        } else {
            self.slots[previous].x_advance = exit_x + self.slots[previous].x_offset;
            let shift = entry_x + self.slots[index].x_offset;
            self.slots[index].x_advance -= shift;
            self.slots[index].x_offset -= shift;
        }
        // Each one lines up with the other at their anchors. Which one moves depends on the flag.
        if flags.flag & RIGHT_TO_LEFT != 0 {
            self.slots[previous].y_offset = entry_y - exit_y;
            self.slots[previous].attached = i16::try_from(index - previous).unwrap_or(0);
            self.slots[previous].attached_as_mark = false;
        } else {
            self.slots[index].y_offset = exit_y - entry_y;
            self.slots[index].attached = -i16::try_from(index - previous).unwrap_or(0);
            self.slots[index].attached_as_mark = false;
        }
        Some(index + 1)
    }

    /// Whether this glyph is one a mark attachment subtable applies to at all.
    fn covered_mark(&self, subtable: usize, index: usize) -> Option<usize> {
        let coverage = subtable + read_u16(self.table, subtable + 2)? as usize;
        coverage_index(self.table, coverage, self.slots[index].glyph)
    }

    /// The nearest glyph before this one that isn't a mark.
    fn base_before(&self, index: usize) -> Option<usize> {
        (0..index).rev().find(|at| {
            let slot = &self.slots[*at];
            slot.class != CLASS_MARK && slot.mask & MASK_HIDDEN == 0
        })
    }

    fn attach_mark(
        &mut self,
        subtable: usize,
        index: usize,
        parent: usize,
        mark_array: usize,
        class_count: usize,
        parent_anchor_at: impl Fn(&Self, usize) -> Option<usize>,
    ) -> Option<usize> {
        let table = self.table;
        let mark_coverage = subtable + read_u16(table, subtable + 2)? as usize;
        let covered = coverage_index(table, mark_coverage, self.slots[index].glyph)?;
        if covered >= read_u16(table, mark_array)? as usize {
            return None;
        }
        let class = read_u16(table, mark_array + 2 + covered * 4)? as usize;
        if class >= class_count {
            return None;
        }
        let mark_anchor = mark_array + read_u16(table, mark_array + 4 + covered * 4)? as usize;
        let parent_anchor = parent_anchor_at(self, class)?;
        let (mark_x, mark_y) = self.anchor(mark_anchor)?;
        let (parent_x, parent_y) = self.anchor(parent_anchor)?;
        let slot = &mut self.slots[index];
        slot.x_offset = parent_x - mark_x;
        slot.y_offset = parent_y - mark_y;
        slot.attached = -i16::try_from(index - parent).unwrap_or(0);
        slot.attached_as_mark = true;
        Some(index + 1)
    }

    fn mark_on_base(&mut self, subtable: usize, index: usize) -> Option<usize> {
        let table = self.table;
        self.covered_mark(subtable, index)?;
        let base = self.base_before(index)?;
        let base_coverage = subtable + read_u16(table, subtable + 4)? as usize;
        let row = coverage_index(table, base_coverage, self.slots[base].glyph)?;
        let class_count = read_u16(table, subtable + 6)? as usize;
        let mark_array = subtable + read_u16(table, subtable + 8)? as usize;
        let base_array = subtable + read_u16(table, subtable + 10)? as usize;
        if row >= read_u16(table, base_array)? as usize {
            return None;
        }
        self.attach_mark(
            subtable,
            index,
            base,
            mark_array,
            class_count,
            move |this, class| {
                let at = read_u16(this.table, base_array + 2 + (row * class_count + class) * 2)?;
                (at != 0).then_some(base_array + at as usize)
            },
        )
    }

    fn mark_on_ligature(&mut self, subtable: usize, index: usize) -> Option<usize> {
        let table = self.table;
        self.covered_mark(subtable, index)?;
        let base = self.base_before(index)?;
        let ligature_coverage = subtable + read_u16(table, subtable + 4)? as usize;
        let row = coverage_index(table, ligature_coverage, self.slots[base].glyph)?;
        let class_count = read_u16(table, subtable + 6)? as usize;
        let mark_array = subtable + read_u16(table, subtable + 8)? as usize;
        let ligature_array = subtable + read_u16(table, subtable + 10)? as usize;
        if row >= read_u16(table, ligature_array)? as usize {
            return None;
        }
        let attach = ligature_array + read_u16(table, ligature_array + 2 + row * 2)? as usize;
        let components = read_u16(table, attach)? as usize;
        if components == 0 {
            return None;
        }
        let (mark, ligature) = (self.slots[index], self.slots[base]);
        let component = if mark.ligature != 0 && mark.ligature == ligature.ligature {
            usize::from(mark.component.max(1)) - 1
        } else {
            components - 1
        }
        .min(components - 1);
        self.attach_mark(
            subtable,
            index,
            base,
            mark_array,
            class_count,
            move |this, class| {
                let at = read_u16(
                    this.table,
                    attach + 2 + (component * class_count + class) * 2,
                )?;
                (at != 0).then_some(attach + at as usize)
            },
        )
    }

    fn mark_on_mark(&mut self, subtable: usize, index: usize, flags: Flags) -> Option<usize> {
        let table = self.table;
        self.covered_mark(subtable, index)?;
        let previous = self.previous_before(index, flags)?;
        if self.slots[previous].class != CLASS_MARK {
            return None;
        }
        // Marks on different parts of a ligature don't stack, though a mark can itself be one.
        let (this, other) = (self.slots[index], self.slots[previous]);
        let stacks = if this.ligature == other.ligature {
            this.ligature == 0 || this.component == other.component
        } else {
            (this.ligature != 0 && this.component == 0)
                || (other.ligature != 0 && other.component == 0)
        };
        if !stacks {
            return None;
        }
        let second_coverage = subtable + read_u16(table, subtable + 4)? as usize;
        let row = coverage_index(table, second_coverage, self.slots[previous].glyph)?;
        let class_count = read_u16(table, subtable + 6)? as usize;
        let mark_array = subtable + read_u16(table, subtable + 8)? as usize;
        let second_array = subtable + read_u16(table, subtable + 10)? as usize;
        if row >= read_u16(table, second_array)? as usize {
            return None;
        }
        self.attach_mark(
            subtable,
            index,
            previous,
            mark_array,
            class_count,
            move |this, class| {
                let at = read_u16(
                    this.table,
                    second_array + 2 + (row * class_count + class) * 2,
                )?;
                (at != 0).then_some(second_array + at as usize)
            },
        )
    }
}

fn value_size(format: u16) -> usize {
    format.count_ones() as usize * 2
}

#[cfg(test)]
mod would_substitute_tests {
    use super::*;

    #[test]
    fn a_glyphs_record_stays_small() {
        // Every shaped glyph has one of these, moved around as rules run.
        assert!(size_of::<Slot>() <= 32, "{}", size_of::<Slot>());
    }

    /// A substitution table with one lookup of this kind, with this as its one subtable.
    fn table_of(kind: u16, subtable: &[u8]) -> Vec<u8> {
        let mut table = vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 10];
        // The list of one lookup, right after the header.
        table.extend_from_slice(&[0, 1, 0, 4]);
        // The lookup, with its subtable right after it.
        table.extend_from_slice(&kind.to_be_bytes());
        table.extend_from_slice(&[0, 0, 0, 1, 0, 8]);
        table.extend_from_slice(subtable);
        table
    }

    fn numbers(values: &[u16]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_be_bytes())
            .collect()
    }

    /// A list of one glyph.
    fn covering(glyph: u16) -> Vec<u16> {
        vec![1, 1, glyph]
    }

    /// A ligature of two glyphs.
    fn ligature(first: u16, second: u16) -> Vec<u8> {
        let mut values = vec![1, 18, 1, 8, 1, 4, 99, 2, second];
        values.extend(covering(first));
        numbers(&values)
    }

    /// A chain rule that goes by lists, one glyph in each. Lists come after the rule's own numbers.
    fn chain_by_lists(behind: &[u16], input: &[u16], ahead: &[u16]) -> Vec<u8> {
        let header = 2 + 2 + behind.len() * 2 + 2 + input.len() * 2 + 2 + ahead.len() * 2 + 2;
        let mut values = vec![3];
        let mut at = header as u16;
        let mut lists = Vec::new();
        for group in [behind, input, ahead] {
            values.push(group.len() as u16);
            for glyph in group {
                values.push(at);
                at += 6;
                lists.extend(covering(*glyph));
            }
        }
        values.push(0);
        values.extend(lists);
        numbers(&values)
    }

    /// A chain rule that goes by glyphs: two as input, and any around them.
    fn chain_by_glyphs(first: u16, second: u16, behind: &[u16], ahead: &[u16]) -> Vec<u8> {
        // The header, one set, and then its one rule.
        let mut rule = vec![behind.len() as u16];
        rule.extend_from_slice(behind);
        rule.extend_from_slice(&[2, second]);
        rule.push(ahead.len() as u16);
        rule.extend_from_slice(ahead);
        rule.push(0);
        let coverage = 8 + 4 + rule.len() as u16 * 2;
        let mut values = vec![1, coverage, 1, 8, 1, 4];
        values.extend(rule);
        values.extend(covering(first));
        numbers(&values)
    }

    /// A rule with nothing around it, that goes by glyphs.
    fn context_by_glyphs(first: u16, second: u16) -> Vec<u8> {
        let mut values = vec![1, 18, 1, 8, 1, 4, 2, 0, second];
        values.extend(covering(first));
        numbers(&values)
    }

    /// A rule with nothing around it, that goes by lists.
    fn context_by_lists(first: u16, second: u16) -> Vec<u8> {
        let mut values = vec![3, 2, 0, 10, 16];
        values.extend(covering(first));
        values.extend(covering(second));
        numbers(&values)
    }

    fn would(table: &[u8], glyphs: &[u16], around_too: bool) -> bool {
        let tables = Tables {
            gsub: Some(table),
            gpos: None,
            gdef: None,
        };
        let step = Step {
            lookup: 0,
            mask: MASK_ALL,
            summary: Summary::EVERYTHING,
        };
        would_substitute(tables, &[step], glyphs, around_too)
    }

    #[test]
    fn a_ligature_counts_for_exactly_its_own_glyphs() {
        let table = table_of(4, &ligature(5, 6));
        assert!(would(&table, &[5, 6], false));
        assert!(!would(&table, &[6, 5], false));
        assert!(!would(&table, &[5, 7], false));
        assert!(!would(&table, &[5, 6, 7], false));
        assert!(!would(&table, &[5], false));
    }

    #[test]
    fn a_rule_by_lists_counts_with_nothing_around_it() {
        let table = table_of(6, &chain_by_lists(&[], &[5, 6], &[]));
        assert!(would(&table, &[5, 6], false));
        assert!(!would(&table, &[6, 6], false));
        assert!(!would(&table, &[5, 7], false));
        assert!(!would(&table, &[5, 6, 6], false));
    }

    #[test]
    fn a_rule_by_lists_that_goes_by_whats_around_it_only_counts_if_asked() {
        for table in [
            table_of(6, &chain_by_lists(&[1], &[5, 6], &[])),
            table_of(6, &chain_by_lists(&[], &[5, 6], &[1])),
        ] {
            assert!(!would(&table, &[5, 6], false));
            assert!(would(&table, &[5, 6], true));
        }
    }

    #[test]
    fn a_rule_by_glyphs_is_the_same() {
        let table = table_of(6, &chain_by_glyphs(5, 6, &[], &[]));
        assert!(would(&table, &[5, 6], false));
        assert!(!would(&table, &[5, 7], false));
        assert!(!would(&table, &[4, 6], false));
        for table in [
            table_of(6, &chain_by_glyphs(5, 6, &[1], &[])),
            table_of(6, &chain_by_glyphs(5, 6, &[], &[1])),
        ] {
            assert!(!would(&table, &[5, 6], false));
            assert!(would(&table, &[5, 6], true));
        }
    }

    #[test]
    fn a_rule_with_nothing_around_it_counts() {
        for table in [
            table_of(5, &context_by_glyphs(5, 6)),
            table_of(5, &context_by_lists(5, 6)),
        ] {
            assert!(would(&table, &[5, 6], false));
            assert!(!would(&table, &[5, 7], false));
            assert!(!would(&table, &[4, 6], false));
            assert!(!would(&table, &[5, 6, 6], false));
        }
    }

    #[test]
    fn a_rule_kept_elsewhere_in_the_table_counts_too() {
        let inner = ligature(5, 6);
        let mut extension = numbers(&[1, 4, 0, 0]);
        extension[6..8].copy_from_slice(&8u16.to_be_bytes());
        extension.extend_from_slice(&inner);
        assert!(would(&table_of(7, &extension), &[5, 6], false));
    }

    #[test]
    fn a_cut_off_table_does_no_harm() {
        let whole = table_of(6, &chain_by_lists(&[1], &[5, 6], &[2]));
        assert!(would(&whole, &[5, 6], true));
        // With the second glyph's list gone, or anything before it, nothing can be said for it.
        for length in 0..whole.len() - 6 {
            assert!(!would(&whole[..length], &[5, 6], true), "cut at {length}");
        }
    }

    /// A table of several lookups, each with one subtable.
    fn table_of_lookups(lookups: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut table = vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 10];
        table.extend_from_slice(&(lookups.len() as u16).to_be_bytes());
        let mut at = 2 + lookups.len() * 2;
        for (_, subtable) in lookups {
            table.extend_from_slice(&(at as u16).to_be_bytes());
            at += 8 + subtable.len();
        }
        for (kind, subtable) in lookups {
            table.extend_from_slice(&kind.to_be_bytes());
            table.extend_from_slice(&[0, 0, 0, 1, 0, 8]);
            table.extend_from_slice(subtable);
        }
        table
    }

    #[test]
    fn what_a_rule_adds_goes_right_after_what_it_came_from() {
        // The rule looks past marks, so its glyphs aren't next to each other. The second it knows of after
        // the first lookup is what that made, not the 2 past the mark.
        let mut table = table_of_lookups(&[
            (6, rule_for_three([1, 2, 3], &[(0, 1), (1, 2)])),
            (2, into_two(1, 7, 8)),
            (1, replacing(8, 80)),
        ]);
        // The first lookup's flags: ignore marks.
        let first = 10 + read_u16(&table, 12).unwrap() as usize;
        table[first + 3] = IGNORE_MARK as u8;
        let tables = Tables {
            gsub: Some(&table),
            gpos: None,
            gdef: None,
        };
        let step = Step {
            lookup: 0,
            mask: MASK_ALL,
            summary: Summary::EVERYTHING,
        };
        let mut slots = vec![
            Slot::new(1, 0, MASK_ALL, false),
            Slot::new(9, 1, MASK_ALL, true),
            Slot::new(2, 2, MASK_ALL, false),
            Slot::new(3, 3, MASK_ALL, false),
        ];
        let stage = [step];
        let mut next_ligature = 1;
        substitute_stages(tables, &[&stage], &mut slots, false, &mut next_ligature);
        let glyphs: Vec<u16> = slots.iter().map(|slot| slot.glyph).collect();
        assert_eq!(glyphs, [7, 80, 9, 2, 3]);
    }

    #[test]
    fn a_letter_and_marks_making_one_is_no_ligature_for_marks_to_choose_parts_of() {
        fn ligature_id(second_is_mark: bool) -> u16 {
            let table = table_of(4, &ligature(5, 6));
            let tables = Tables {
                gsub: Some(&table),
                gpos: None,
                gdef: None,
            };
            let step = Step {
                lookup: 0,
                mask: MASK_ALL,
                summary: Summary::EVERYTHING,
            };
            let mut slots = vec![
                Slot::new(5, 0, MASK_ALL, false),
                Slot::new(6, 1, MASK_ALL, second_is_mark),
            ];
            let stage = [step];
            let mut next_ligature = 1;
            substitute_stages(tables, &[&stage], &mut slots, false, &mut next_ligature);
            assert_eq!(slots.len(), 1);
            slots[0].ligature
        }
        assert_eq!(ligature_id(true), 0);
        assert_ne!(ligature_id(false), 0);
    }

    /// A rule for three glyphs in a row, which then runs these lookups at these places.
    fn rule_for_three(glyphs: [u16; 3], records: &[(u16, u16)]) -> Vec<u8> {
        let header = 2 + 2 + 2 + 3 * 2 + 2 + 2 + records.len() * 4;
        let mut values = vec![3, 0, 3];
        for index in 0..3 {
            values.push((header + index * 6) as u16);
        }
        values.push(0);
        values.push(records.len() as u16);
        for (sequence, lookup) in records {
            values.extend_from_slice(&[*sequence, *lookup]);
        }
        for glyph in glyphs {
            values.extend(covering(glyph));
        }
        numbers(&values)
    }

    fn replacing(glyph: u16, with: u16) -> Vec<u8> {
        let mut values = vec![2, 8, 1, with];
        values.extend(covering(glyph));
        numbers(&values)
    }

    fn into_two(glyph: u16, first: u16, second: u16) -> Vec<u8> {
        let mut values = vec![1, 14, 1, 8, 2, first, second];
        values.extend(covering(glyph));
        numbers(&values)
    }

    fn after_rule(table: &[u8], glyphs: &[u16]) -> Vec<u16> {
        let tables = Tables {
            gsub: Some(table),
            gpos: None,
            gdef: None,
        };
        let step = Step {
            lookup: 0,
            mask: MASK_ALL,
            summary: Summary::EVERYTHING,
        };
        let mut slots: Vec<Slot> = glyphs
            .iter()
            .enumerate()
            .map(|(at, glyph)| Slot::new(*glyph, at as u32, MASK_ALL, false))
            .collect();
        let stage = [step];
        let mut next_ligature = 1;
        substitute_stages(tables, &[&stage], &mut slots, false, &mut next_ligature);
        slots.iter().map(|slot| slot.glyph).collect()
    }

    #[test]
    fn numbers_in_a_rule_go_by_what_it_has_already_done() {
        // Three glyphs, whose first two become one, after which the second is what was the third.
        let table = table_of_lookups(&[
            (6, rule_for_three([1, 2, 3], &[(0, 1), (1, 2)])),
            (4, ligature(1, 2)),
            (1, replacing(3, 30)),
        ]);
        assert_eq!(after_rule(&table, &[1, 2, 3]), [99, 30]);
    }

    #[test]
    fn a_number_past_what_a_rule_has_left_is_nothing() {
        // There are two glyphs after the first lookup, so there's no third to change.
        let table = table_of_lookups(&[
            (6, rule_for_three([1, 2, 3], &[(0, 1), (2, 2)])),
            (4, ligature(1, 2)),
            (1, replacing(3, 30)),
        ]);
        assert_eq!(after_rule(&table, &[1, 2, 3]), [99, 3]);
    }

    #[test]
    fn numbers_count_what_a_rule_has_added_too() {
        // One glyph becomes two, so what was the third is the fourth, and the second is new.
        let table = table_of_lookups(&[
            (6, rule_for_three([1, 2, 3], &[(0, 1), (3, 2)])),
            (2, into_two(1, 7, 8)),
            (1, replacing(3, 30)),
        ]);
        assert_eq!(after_rule(&table, &[1, 2, 3]), [7, 8, 2, 30]);
        let table = table_of_lookups(&[
            (6, rule_for_three([1, 2, 3], &[(0, 1), (1, 2)])),
            (2, into_two(1, 7, 8)),
            (1, replacing(8, 80)),
        ]);
        assert_eq!(after_rule(&table, &[1, 2, 3]), [7, 80, 2, 3]);
    }

    #[test]
    fn nothing_counts_with_no_table() {
        let tables = Tables {
            gsub: None,
            gpos: None,
            gdef: None,
        };
        assert!(!would_substitute(tables, &[], &[5, 6], true));
    }
}
