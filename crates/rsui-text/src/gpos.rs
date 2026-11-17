//! Reads kerning from a font's GPOS table, where modern fonts keep it.
//!
//! GPOS can do far more than this. This only reads pair adjustments in the `kern`
//! feature, and only how far they move the next glyph.

use crate::font::{read_i16, read_u16, read_u32};
use crate::variations::{Store, device_delta};

/// How many pair adjustment subtables to look through. Fonts need a handful, so a font
/// with more is damaged or trying to slow things down.
const MAX_SUBTABLES: usize = 256;

/// Where to find a font's kerning, as positions in the font file.
pub(crate) struct PairKerning {
    /// Each subtable's position, in lookup order.
    subtables: Box<[u32]>,
    /// For each lookup, the index in `subtables` of its first subtable. Only the first
    /// subtable in a lookup that covers a glyph applies.
    lookups: Box<[u32]>,
}

impl PairKerning {
    /// Finds the kerning subtables, with positions relative to the table. Returns `None`
    /// if the font has none, or the table is damaged.
    pub(crate) fn read(gpos: &[u8]) -> Option<Self> {
        let script_list = read_u16(gpos, 4)? as usize;
        let feature_list = read_u16(gpos, 6)? as usize;
        let lookup_list = read_u16(gpos, 8)? as usize;

        let features = default_language_features(gpos, script_list)?;
        let feature_count = read_u16(gpos, feature_list)? as usize;

        let mut lookup_indices = Vec::new();
        for &feature in &features {
            let feature = feature as usize;
            if feature >= feature_count {
                continue;
            }
            let record = feature_list + 2 + feature * 6;
            if gpos.get(record..record + 4)? != b"kern" {
                continue;
            }
            let table = feature_list + read_u16(gpos, record + 4)? as usize;
            let count = read_u16(gpos, table + 2)? as usize;
            for index in 0..count {
                lookup_indices.push(read_u16(gpos, table + 4 + index * 2)?);
            }
        }
        lookup_indices.sort_unstable();
        lookup_indices.dedup();

        let lookup_count = read_u16(gpos, lookup_list)?;
        let mut subtables = Vec::new();
        let mut lookups = Vec::new();
        for index in lookup_indices {
            if index >= lookup_count {
                continue;
            }
            let lookup =
                lookup_list + read_u16(gpos, lookup_list + 2 + index as usize * 2)? as usize;
            let kind = read_u16(gpos, lookup)?;
            let count = read_u16(gpos, lookup + 4)? as usize;
            lookups.push(subtables.len() as u32);
            for sub in 0..count {
                if subtables.len() >= MAX_SUBTABLES {
                    break;
                }
                let offset = read_u16(gpos, lookup + 6 + sub * 2)? as usize;
                let mut subtable = lookup + offset;
                let mut subtable_kind = kind;
                // A subtable can sit behind a 32-bit offset, to reach further.
                if kind == 9 {
                    subtable_kind = read_u16(gpos, subtable + 2)?;
                    subtable += read_u32(gpos, subtable + 4)? as usize;
                }
                if subtable_kind == 2 {
                    subtables.push(u32::try_from(subtable).ok()?);
                }
            }
        }
        if subtables.is_empty() {
            return None;
        }
        Some(Self {
            subtables: subtables.into(),
            lookups: lookups.into(),
        })
    }

    /// Makes the positions relative to the font file instead of the table.
    pub(crate) fn at(mut self, table_start: usize) -> Self {
        for subtable in &mut self.subtables {
            *subtable = subtable.saturating_add(table_start as u32);
        }
        self
    }

    /// The adjustment between two glyphs, given the whole font file.
    pub(crate) fn kerning(
        &self,
        file: &[u8],
        left: u16,
        right: u16,
        varied: Option<Store<'_>>,
    ) -> i16 {
        let mut total = 0i16;
        for (index, &first) in self.lookups.iter().enumerate() {
            let end = self
                .lookups
                .get(index + 1)
                .map_or(self.subtables.len(), |&next| next as usize);
            for &subtable in &self.subtables[first as usize..end] {
                let found = pair_adjustment(file, subtable as usize, left, right, varied);
                if let Some(Some(value)) = found {
                    total = total.saturating_add(value);
                }
                if found.is_some() {
                    break;
                }
            }
        }
        total
    }
}

/// The features for a font's Latin script, or its default script, in the default language.
fn default_language_features(gpos: &[u8], script_list: usize) -> Option<Vec<u16>> {
    let count = read_u16(gpos, script_list)? as usize;
    let mut chosen = None;
    for index in 0..count {
        let record = script_list + 2 + index * 6;
        let tag = gpos.get(record..record + 4)?;
        let offset = read_u16(gpos, record + 4)? as usize;
        if tag == b"latn" {
            chosen = Some(offset);
            break;
        }
        if tag == b"DFLT" {
            chosen = Some(offset);
        }
    }
    let script = script_list + chosen?;
    let language = read_u16(gpos, script)? as usize;
    if language == 0 {
        return None;
    }
    let language = script + language;
    let count = read_u16(gpos, language + 4)? as usize;
    (0..count)
        .map(|index| read_u16(gpos, language + 6 + index * 2))
        .collect()
}

/// Looks for a pair in one subtable. The outer `None` means this subtable doesn't apply,
/// and the next one in the lookup should be tried. That's the case if it doesn't cover
/// the first glyph, or if it lists pairs and this isn't one of them. A subtable of classes
/// applies to any glyphs it covers, so it ends the lookup even with nothing to adjust,
/// which the inner `None` means.
fn pair_adjustment(
    file: &[u8],
    subtable: usize,
    left: u16,
    right: u16,
    varied: Option<Store<'_>>,
) -> Option<Option<i16>> {
    let format = read_u16(file, subtable)?;
    let coverage = subtable + read_u16(file, subtable + 2)? as usize;
    let covered = coverage_index(file, coverage, left)?;
    let value_format_1 = read_u16(file, subtable + 4)?;
    let value_format_2 = read_u16(file, subtable + 6)?;
    let size_1 = value_size(value_format_1);
    let size_2 = value_size(value_format_2);
    let record_size = size_1 + size_2;

    match format {
        1 => {
            let set_count = read_u16(file, subtable + 8)? as usize;
            if covered >= set_count {
                return None;
            }
            let set = subtable + read_u16(file, subtable + 10 + covered * 2)? as usize;
            let count = read_u16(file, set)? as usize;
            let pair_size = 2 + record_size;
            let (mut low, mut high) = (0, count);
            while low < high {
                let middle = (low + high) / 2;
                let pair = set + 2 + middle * pair_size;
                let second = read_u16(file, pair)?;
                match second.cmp(&right) {
                    std::cmp::Ordering::Equal => {
                        // A pair's values keep their changes from where its set starts.
                        return Some(x_advance(file, pair + 2, value_format_1, set, varied));
                    }
                    std::cmp::Ordering::Less => low = middle + 1,
                    std::cmp::Ordering::Greater => high = middle,
                }
            }
            None
        }
        2 => {
            let class_1 = glyph_class(
                file,
                subtable + read_u16(file, subtable + 8)? as usize,
                left,
            );
            let class_2 = glyph_class(
                file,
                subtable + read_u16(file, subtable + 10)? as usize,
                right,
            );
            let class_1_count = read_u16(file, subtable + 12)? as usize;
            let class_2_count = read_u16(file, subtable + 14)? as usize;
            if class_1 >= class_1_count || class_2 >= class_2_count {
                return Some(None);
            }
            let record = subtable + 16 + (class_1 * class_2_count + class_2) * record_size;
            Some(x_advance(file, record, value_format_1, subtable, varied))
        }
        _ => None,
    }
}

fn value_size(format: u16) -> usize {
    format.count_ones() as usize * 2
}

/// How far the second glyph moves, from a value record, with what it changes by where the font is set along its axes,
/// for a font that varies. Its changes are kept from where the subtable starts.
fn x_advance(
    file: &[u8],
    record: usize,
    format: u16,
    subtable: usize,
    varied: Option<Store<'_>>,
) -> Option<i16> {
    const X_PLACEMENT: u16 = 1 << 0;
    const Y_PLACEMENT: u16 = 1 << 1;
    const X_ADVANCE: u16 = 1 << 2;
    const X_ADVANCE_DEVICE: u16 = 1 << 6;
    if format & (X_ADVANCE | X_ADVANCE_DEVICE) == 0 {
        return None;
    }
    let skipped = (format & (X_PLACEMENT | Y_PLACEMENT)).count_ones() as usize;
    let value = if format & X_ADVANCE != 0 {
        read_i16(file, record + skipped * 2)?
    } else {
        0
    };
    if format & X_ADVANCE_DEVICE == 0 {
        return Some(value);
    }
    let Some(varied) = varied else {
        return Some(value);
    };
    let before = (format & 0x3F).count_ones() as usize;
    let device = read_u16(file, record + before * 2)? as usize;
    if device == 0 {
        return Some(value);
    }
    let delta = device_delta(file, subtable + device, varied);
    Some((i32::from(value) + delta).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16)
}

/// A glyph's position in a coverage table, if it's there.
pub(crate) fn coverage_index(file: &[u8], coverage: usize, glyph: u16) -> Option<usize> {
    let format = read_u16(file, coverage)?;
    let count = read_u16(file, coverage + 2)? as usize;
    let (mut low, mut high) = (0, count);
    match format {
        1 => {
            while low < high {
                let middle = (low + high) / 2;
                let found = read_u16(file, coverage + 4 + middle * 2)?;
                match found.cmp(&glyph) {
                    std::cmp::Ordering::Equal => return Some(middle),
                    std::cmp::Ordering::Less => low = middle + 1,
                    std::cmp::Ordering::Greater => high = middle,
                }
            }
            None
        }
        2 => {
            while low < high {
                let middle = (low + high) / 2;
                let range = coverage + 4 + middle * 6;
                if read_u16(file, range + 2)? < glyph {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            if low >= count {
                return None;
            }
            let range = coverage + 4 + low * 6;
            let start = read_u16(file, range)?;
            if glyph < start {
                return None;
            }
            let first_index = read_u16(file, range + 4)? as usize;
            Some(first_index + (glyph - start) as usize)
        }
        _ => None,
    }
}

/// A glyph's class in a class table. Any glyph not listed is in class 0.
pub(crate) fn glyph_class(file: &[u8], table: usize, glyph: u16) -> usize {
    glyph_class_inner(file, table, glyph).unwrap_or(0)
}

fn glyph_class_inner(file: &[u8], table: usize, glyph: u16) -> Option<usize> {
    match read_u16(file, table)? {
        1 => {
            let start = read_u16(file, table + 2)?;
            let count = read_u16(file, table + 4)?;
            let index = glyph.checked_sub(start)?;
            if index >= count {
                return None;
            }
            read_u16(file, table + 6 + index as usize * 2).map(usize::from)
        }
        2 => {
            let count = read_u16(file, table + 2)? as usize;
            let (mut low, mut high) = (0, count);
            while low < high {
                let middle = (low + high) / 2;
                let range = table + 4 + middle * 6;
                if read_u16(file, range + 2)? < glyph {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            if low >= count {
                return None;
            }
            let range = table + 4 + low * 6;
            if glyph < read_u16(file, range)? {
                return None;
            }
            read_u16(file, range + 4).map(usize::from)
        }
        _ => None,
    }
}
