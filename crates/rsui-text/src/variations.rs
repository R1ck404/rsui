//! Fonts that vary, such as by weight or by the size they're drawn at, along axes their `fvar` table lists.
//!
//! Where a font is set along its axes is first turned into numbers from -1 to 1 for each, through `avar` if it has one,
//! as 2.14 fixed-point numbers, as the OpenType specification does it. Everything else goes by those:
//!
//! - outlines move by what `gvar` says for each point, worked out for the points it doesn't name;
//! - advances change by what `HVAR` says, or by how `gvar` moves a glyph's last two points if there's no `HVAR`;
//! - measurements of the whole font, such as how high a lowercase `x` is, change by what `MVAR` says;
//! - kerning and mark positions change by what `GDEF`'s store says, for the values in `GPOS` that point to it.
//!
//! Like the rest of reading a font, everything here is checked, and damaged data gives no change rather than a panic.

use std::ops::Range;

use crate::font::{read_i16, read_u16, read_u32};

/// One in 2.14 fixed point.
const ONE: i32 = 1 << 14;
/// The same in 16.16 fixed point, which a coordinate is worked out in before it's rounded to 2.14.
const FIXED_ONE: i32 = 1 << 16;

/// How many points a glyph can have for its variations to be worked out. A glyph can't have more than 65 535 points, and
/// this leaves room for the four that only say where it starts and ends.
const MAX_POINTS: usize = 0x1_0004;

/// One way a font can vary, as its `fvar` table lists it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VariationAxis {
    /// Such as `wght` for weight, or `opsz` for the size it's meant to be drawn at.
    pub tag: [u8; 4],
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

/// A place along a font's axes that it gives a name, such as "Semibold Text".
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NamedInstance {
    /// The `name` table's number for its name, such as "Semibold Text".
    pub(crate) subfamily_name: u16,
    /// Where it is along each axis, in the order `fvar` has them.
    pub(crate) coordinates: Vec<f32>,
}

fn read_fixed(data: &[u8], at: usize) -> Option<f32> {
    read_u32(data, at).map(|value| value as i32 as f32 / 65536.0)
}

fn read_i32(data: &[u8], at: usize) -> Option<i32> {
    read_u32(data, at).map(|value| value as i32)
}

/// The axes `fvar` lists, in order.
pub(crate) fn axes(fvar: &[u8]) -> Vec<VariationAxis> {
    (|| {
        let at = read_u16(fvar, 4)? as usize;
        let count = read_u16(fvar, 8)? as usize;
        let size = read_u16(fvar, 10)? as usize;
        if size < 20 {
            return None;
        }
        let mut axes = Vec::with_capacity(count.min(64));
        for index in 0..count.min(64) {
            let record = at + index * size;
            let tag = fvar.get(record..record + 4)?;
            let (min, default, max) = (
                read_fixed(fvar, record + 4)?,
                read_fixed(fvar, record + 8)?,
                read_fixed(fvar, record + 12)?,
            );
            axes.push(VariationAxis {
                tag: [tag[0], tag[1], tag[2], tag[3]],
                // A damaged axis that's backwards is read as only having its default.
                min: min.min(default),
                default,
                max: max.max(default),
            });
        }
        Some(axes)
    })()
    .unwrap_or_default()
}

/// The places along its axes that `fvar` names.
pub(crate) fn named_instances(fvar: &[u8]) -> Vec<NamedInstance> {
    (|| {
        let at = read_u16(fvar, 4)? as usize;
        let count = read_u16(fvar, 8)? as usize;
        let axis_size = read_u16(fvar, 10)? as usize;
        let instances = read_u16(fvar, 12)? as usize;
        let size = read_u16(fvar, 14)? as usize;
        if size < 4 + count * 4 {
            return None;
        }
        let first = at + count * axis_size;
        let mut found = Vec::with_capacity(instances.min(256));
        for index in 0..instances.min(256) {
            let record = first + index * size;
            let subfamily_name = read_u16(fvar, record)?;
            let coordinates = (0..count)
                .map(|axis| read_fixed(fvar, record + 4 + axis * 4))
                .collect::<Option<Vec<f32>>>()?;
            found.push(NamedInstance {
                subfamily_name,
                coordinates,
            });
        }
        Some(found)
    })()
    .unwrap_or_default()
}

/// Where `value` is along an axis, from -1 at its least through 0 at its default to 1 at its most, in 16.16 fixed point.
/// The specification works it out in that, and moves it through `avar` in that too, before it's rounded to the 2.14 that
/// regions and tuples are in, as [`to_f2dot14`] does. HarfBuzz does, and so Edge: rounded to 2.14 first, a weight of 650 in
/// Segoe UI Variable came out a 16384th short, which moved its kerning by a 200th of a unit.
pub(crate) fn normalize(axis: &VariationAxis, value: f32) -> i32 {
    let value = if value.is_nan() {
        axis.default
    } else {
        value.clamp(axis.min, axis.max)
    };
    let normal = if value < axis.default {
        (value - axis.default) / (axis.default - axis.min)
    } else if value > axis.default {
        (value - axis.default) / (axis.max - axis.default)
    } else {
        0.0
    };
    (normal * FIXED_ONE as f32).round() as i32
}

/// A coordinate in 16.16 fixed point rounded to 2.14, as the specification has it.
pub(crate) fn to_f2dot14(coordinate: i32) -> i32 {
    ((coordinate + 2) >> 2).clamp(-ONE, ONE)
}

/// Moves a coordinate along one axis as `avar` says, if it has anything for that axis, both in 16.16 fixed point.
pub(crate) fn map_through_avar(avar: &[u8], axis: usize, coordinate: i32) -> i32 {
    (|| {
        let count = read_u16(avar, 6)? as usize;
        if axis >= count {
            return None;
        }
        let mut at = 8;
        for _ in 0..axis {
            at += 2 + read_u16(avar, at)? as usize * 4;
        }
        let maps = read_u16(avar, at)? as usize;
        // Its points are in 2.14, which is a quarter of as many as 16.16.
        let pair = |index: usize| -> Option<(i32, i32)> {
            let record = at + 2 + index * 4;
            Some((
                i32::from(read_i16(avar, record)?) << 2,
                i32::from(read_i16(avar, record + 2)?) << 2,
            ))
        };
        if maps == 0 {
            return None;
        }
        let (first_from, first_to) = pair(0)?;
        if maps == 1 || coordinate <= first_from {
            return Some(coordinate - first_from + first_to);
        }
        let mut index = 1;
        while index < maps - 1 && coordinate > pair(index)?.0 {
            index += 1;
        }
        let (from, to) = pair(index)?;
        if coordinate >= from {
            return Some(coordinate - from + to);
        }
        let (before_from, before_to) = pair(index - 1)?;
        if before_from == from {
            return Some(before_to);
        }
        let moved = before_to as f64
            + (to - before_to) as f64 * (coordinate - before_from) as f64
                / (from - before_from) as f64;
        Some(moved.round() as i32)
    })()
    .unwrap_or(coordinate)
    .clamp(-FIXED_ONE, FIXED_ONE)
}

/// How much of what one region or tuple says applies, along one axis, as an item variation store reckons it.
fn region_axis_scalar(coordinate: i32, start: i32, peak: i32, end: i32) -> f32 {
    if peak == 0 || coordinate == peak {
        return 1.0;
    }
    if coordinate == 0 {
        return 0.0;
    }
    // A region that's damaged applies everywhere along this axis, as the specification says.
    if start > peak || peak > end || (start < 0 && end > 0) {
        return 1.0;
    }
    if coordinate <= start || end <= coordinate {
        return 0.0;
    }
    if coordinate < peak {
        (coordinate - start) as f32 / (peak - start) as f32
    } else {
        (end - coordinate) as f32 / (end - peak) as f32
    }
}

/// How much of each region in an item variation store applies at these coordinates, in the store's order.
pub(crate) fn region_scalars(store: &[u8], coordinates: &[i32]) -> Vec<f32> {
    (|| {
        let regions = read_u32(store, 2)? as usize;
        let axis_count = read_u16(store, regions)? as usize;
        let count = read_u16(store, regions + 2)? as usize;
        let mut scalars = Vec::with_capacity(count);
        for region in 0..count {
            let mut scalar = 1.0;
            for axis in 0..axis_count {
                let record = regions + 4 + (region * axis_count + axis) * 6;
                let start = i32::from(read_i16(store, record)?);
                let peak = i32::from(read_i16(store, record + 2)?);
                let end = i32::from(read_i16(store, record + 4)?);
                let coordinate = coordinates.get(axis).copied().unwrap_or(0);
                scalar *= region_axis_scalar(coordinate, start, peak, end);
                if scalar == 0.0 {
                    break;
                }
            }
            scalars.push(scalar);
        }
        Some(scalars)
    })()
    .unwrap_or_default()
}

/// An item variation store, with how much of each of its regions applies where the font is set.
#[derive(Clone, Copy)]
pub(crate) struct Store<'a> {
    /// From where the store starts.
    pub(crate) data: &'a [u8],
    pub(crate) scalars: &'a [f32],
}

impl Store<'_> {
    /// How much one value changes, by its place in the store.
    pub(crate) fn delta(&self, outer: u16, inner: u16) -> f32 {
        self.try_delta(outer, inner).unwrap_or(0.0)
    }

    fn try_delta(&self, outer: u16, inner: u16) -> Option<f32> {
        let data = self.data;
        let count = read_u16(data, 6)?;
        if outer >= count {
            return None;
        }
        let at = read_u32(data, 8 + outer as usize * 4)? as usize;
        let items = read_u16(data, at)?;
        if inner >= items {
            return None;
        }
        let words = read_u16(data, at + 2)?;
        let long = words & 0x8000 != 0;
        let word_count = (words & 0x7FFF) as usize;
        let region_count = read_u16(data, at + 4)? as usize;
        if word_count > region_count {
            return None;
        }
        let (wide, narrow) = if long { (4, 2) } else { (2, 1) };
        let row_size = word_count * wide + (region_count - word_count) * narrow;
        let row = at + 6 + region_count * 2 + inner as usize * row_size;
        let mut total = 0.0;
        let mut position = row;
        for index in 0..region_count {
            let region = read_u16(data, at + 6 + index * 2)? as usize;
            let delta = match (index < word_count, long) {
                (true, true) => read_i32(data, position)?,
                (true, false) | (false, true) => i32::from(read_i16(data, position)?),
                (false, false) => i32::from(*data.get(position)? as i8),
            };
            position += if index < word_count { wide } else { narrow };
            let scalar = self.scalars.get(region).copied().unwrap_or(0.0);
            if scalar != 0.0 {
                total += scalar * delta as f32;
            }
        }
        Some(total)
    }
}

/// What one of `GPOS`'s device tables says its value changes by, if it's one that points into `GDEF`'s store rather than
/// one for hinting. It's kept as the fraction of a unit it can be, as HarfBuzz keeps it, and so Edge: rounded to whole units,
/// kerning was up to half of one off, which is 0.024 pixels a pair at 100 pixels in a font of 2048 units.
pub(crate) fn device_delta(table: &[u8], device: usize, store: Store<'_>) -> f32 {
    match (
        read_u16(table, device),
        read_u16(table, device + 2),
        read_u16(table, device + 4),
    ) {
        (Some(outer), Some(inner), Some(0x8000)) => store.delta(outer, inner),
        _ => 0.0,
    }
}

/// Where a delta-set index map sends `index`: which of a store's lists, and which item in it. A map with nothing in it
/// sends each index to that item of the first list.
pub(crate) fn map_index(map: &[u8], index: u32) -> Option<(u16, u16)> {
    let format = *map.first()?;
    let entry_format = *map.get(1)?;
    let (count, first) = match format {
        0 => (u32::from(read_u16(map, 2)?), 4),
        1 => (read_u32(map, 2)?, 6),
        _ => return None,
    };
    if count == 0 {
        return Some(((index >> 16) as u16, index as u16));
    }
    let index = index.min(count - 1) as usize;
    let size = usize::from((entry_format >> 4) & 3) + 1;
    let inner_bits = u32::from(entry_format & 0xF) + 1;
    let entry = map.get(first + index * size..first + index * size + size)?;
    let value = entry
        .iter()
        .fold(0u32, |value, byte| value << 8 | u32::from(*byte));
    Some((
        (value >> inner_bits) as u16,
        (value & ((1 << inner_bits) - 1)) as u16,
    ))
}

/// How much a glyph's advance changes, as `HVAR` says.
pub(crate) fn advance_delta(hvar: &[u8], scalars: &[f32], glyph: u16) -> f32 {
    (|| {
        let store = read_u32(hvar, 4)? as usize;
        let map = read_u32(hvar, 8)? as usize;
        let (outer, inner) = if map == 0 {
            (0, glyph)
        } else {
            map_index(hvar.get(map..)?, u32::from(glyph))?
        };
        Some(
            Store {
                data: hvar.get(store..)?,
                scalars,
            }
            .delta(outer, inner),
        )
    })()
    .unwrap_or(0.0)
}

/// Where `MVAR`'s store is, from the start of the table.
pub(crate) fn metrics_store(mvar: &[u8]) -> Option<usize> {
    let at = read_u16(mvar, 10)? as usize;
    (at != 0).then_some(at)
}

/// How much one of the font's measurements changes, as `MVAR` says, by its tag, such as `xhgt` for how high a lowercase
/// `x` is.
pub(crate) fn metric_delta(mvar: &[u8], scalars: &[f32], tag: &[u8; 4]) -> f32 {
    (|| {
        let size = read_u16(mvar, 6)? as usize;
        let count = read_u16(mvar, 8)? as usize;
        let store = metrics_store(mvar)?;
        if size < 8 {
            return None;
        }
        let (mut low, mut high) = (0, count);
        while low < high {
            let middle = (low + high) / 2;
            let record = 12 + middle * size;
            match mvar.get(record..record + 4)?.cmp(&tag[..]) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => {
                    let outer = read_u16(mvar, record + 4)?;
                    let inner = read_u16(mvar, record + 6)?;
                    return Some(
                        Store {
                            data: mvar.get(store..)?,
                            scalars,
                        }
                        .delta(outer, inner),
                    );
                }
            }
        }
        None
    })()
    .unwrap_or(0.0)
}

/// How much one tuple of `gvar` applies at these coordinates.
fn tuple_scalar(
    coordinates: &[i32],
    peak: &[u8],
    intermediate: Option<(&[u8], &[u8])>,
    axis_count: usize,
) -> Option<f32> {
    let mut scalar = 1.0;
    for axis in 0..axis_count {
        let coordinate = coordinates.get(axis).copied().unwrap_or(0);
        let top = i32::from(read_i16(peak, axis * 2)?);
        if top == 0 || coordinate == top {
            continue;
        }
        if let Some((starts, ends)) = intermediate {
            let start = i32::from(read_i16(starts, axis * 2)?);
            let end = i32::from(read_i16(ends, axis * 2)?);
            if start > top || top > end || (start < 0 && end > 0) {
                continue;
            }
            if coordinate < start || coordinate > end {
                return Some(0.0);
            }
            if coordinate < top {
                if top != start {
                    scalar *= (coordinate - start) as f32 / (top - start) as f32;
                }
            } else if top != end {
                scalar *= (end - coordinate) as f32 / (end - top) as f32;
            }
        } else if coordinate == 0 || coordinate < top.min(0) || coordinate > top.max(0) {
            return Some(0.0);
        } else {
            scalar *= coordinate as f32 / top as f32;
        }
    }
    Some(scalar)
}

/// Reads a list of point numbers, as `gvar` packs them. `None` in what's returned is every point.
fn packed_points(data: &[u8], mut at: usize) -> Option<(Option<Vec<u16>>, usize)> {
    let first = *data.get(at)?;
    at += 1;
    if first == 0 {
        return Some((None, at));
    }
    let count = if first & 0x80 != 0 {
        let second = *data.get(at)?;
        at += 1;
        usize::from(first & 0x7F) << 8 | usize::from(second)
    } else {
        usize::from(first)
    };
    let mut points = Vec::with_capacity(count);
    let mut point = 0u16;
    while points.len() < count {
        let control = *data.get(at)?;
        at += 1;
        let run = usize::from(control & 0x7F) + 1;
        for _ in 0..run.min(count - points.len()) {
            let step = if control & 0x80 != 0 {
                let step = read_u16(data, at)?;
                at += 2;
                step
            } else {
                let step = u16::from(*data.get(at)?);
                at += 1;
                step
            };
            point = point.wrapping_add(step);
            points.push(point);
        }
    }
    Some((Some(points), at))
}

/// Reads `count` deltas, as `gvar` packs them, adding them to `into`.
fn packed_deltas(data: &[u8], mut at: usize, count: usize, into: &mut Vec<i32>) -> Option<usize> {
    into.clear();
    while into.len() < count {
        let control = *data.get(at)?;
        at += 1;
        let run = (usize::from(control & 0x3F) + 1).min(count - into.len());
        match control & 0xC0 {
            0x80 => into.extend(std::iter::repeat_n(0, run)),
            0x40 => {
                for _ in 0..run {
                    into.push(i32::from(read_i16(data, at)?));
                    at += 2;
                }
            }
            0xC0 => {
                for _ in 0..run {
                    into.push(read_i32(data, at)?);
                    at += 4;
                }
            }
            _ => {
                for _ in 0..run {
                    into.push(i32::from(*data.get(at)? as i8));
                    at += 1;
                }
            }
        }
    }
    Some(at)
}

/// The part of `gvar` that's about one glyph, if it varies.
fn glyph_variations(gvar: &[u8], glyph: u16) -> Option<&[u8]> {
    let glyph_count = read_u16(gvar, 12)?;
    if glyph >= glyph_count {
        return None;
    }
    let long = read_u16(gvar, 14)? & 1 != 0;
    let data = read_u32(gvar, 16)? as usize;
    let index = glyph as usize;
    let (start, end) = if long {
        (
            read_u32(gvar, 20 + index * 4)? as usize,
            read_u32(gvar, 24 + index * 4)? as usize,
        )
    } else {
        (
            read_u16(gvar, 20 + index * 2)? as usize * 2,
            read_u16(gvar, 22 + index * 2)? as usize * 2,
        )
    };
    if end <= start {
        return None;
    }
    gvar.get(data.checked_add(start)?..data.checked_add(end)?)
}

/// Works out how far each point of a glyph moves at these coordinates, as `gvar` says, adding that to `deltas`.
///
/// `points` are the glyph's points as they are by default, followed by the four that say where it starts and ends;
/// `deltas` has one for each. For a glyph with an outline, `ends` is the last point of each contour, and points `gvar`
/// doesn't name move along with those around them; a glyph made of others has none, and only the parts it names move.
pub(crate) fn glyph_deltas(
    gvar: &[u8],
    glyph: u16,
    coordinates: &[i32],
    points: &[(f32, f32)],
    ends: &[usize],
    deltas: &mut [(f32, f32)],
) {
    if coordinates.iter().all(|&coordinate| coordinate == 0) || points.len() > MAX_POINTS {
        return;
    }
    let _ = apply_tuples(gvar, glyph, coordinates, points, ends, deltas);
}

fn apply_tuples(
    gvar: &[u8],
    glyph: u16,
    coordinates: &[i32],
    points: &[(f32, f32)],
    ends: &[usize],
    deltas: &mut [(f32, f32)],
) -> Option<()> {
    let axis_count = read_u16(gvar, 4)? as usize;
    let shared_count = read_u16(gvar, 6)? as usize;
    let shared_tuples = read_u32(gvar, 8)? as usize;
    let data = glyph_variations(gvar, glyph)?;
    let header = read_u16(data, 0)?;
    let tuple_count = usize::from(header & 0x0FFF);
    let mut serialized = read_u16(data, 2)? as usize;
    // Tuples without their own points use these, and a glyph that has none for them to share is read as naming every
    // point, as HarfBuzz reads it.
    let mut shared_points = None;
    if header & 0x8000 != 0 {
        let (points, after) = packed_points(data, serialized)?;
        shared_points = points;
        serialized = after;
    }
    let count = points.len();
    let mut at = 4;
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    let mut moved: Vec<(f32, f32)> = Vec::new();
    let mut touched: Vec<bool> = Vec::new();
    for _ in 0..tuple_count {
        let size = read_u16(data, at)? as usize;
        let index = read_u16(data, at + 2)?;
        at += 4;
        let tuple_bytes = axis_count * 2;
        let peak = if index & 0x8000 != 0 {
            let peak = data.get(at..at + tuple_bytes)?;
            at += tuple_bytes;
            peak
        } else {
            let shared = usize::from(index & 0x0FFF);
            if shared >= shared_count {
                return None;
            }
            let from = shared_tuples + shared * tuple_bytes;
            gvar.get(from..from + tuple_bytes)?
        };
        let intermediate = if index & 0x4000 != 0 {
            let starts = data.get(at..at + tuple_bytes)?;
            let ends = data.get(at + tuple_bytes..at + 2 * tuple_bytes)?;
            at += 2 * tuple_bytes;
            Some((starts, ends))
        } else {
            None
        };
        let tuple = serialized;
        serialized += size;
        let scalar = tuple_scalar(coordinates, peak, intermediate, axis_count)?;
        if scalar == 0.0 {
            continue;
        }
        let tuple_data = data.get(tuple..serialized)?;
        let mut position = 0;
        let private;
        let named: Option<&Vec<u16>> = if index & 0x2000 != 0 {
            let (points, after) = packed_points(tuple_data, position)?;
            position = after;
            private = points;
            private.as_ref()
        } else {
            shared_points.as_ref()
        };
        let named_count = named.map_or(count, Vec::len);
        position = packed_deltas(tuple_data, position, named_count, &mut xs)?;
        packed_deltas(tuple_data, position, named_count, &mut ys)?;
        match named {
            None => {
                for ((delta, x), y) in deltas.iter_mut().zip(&xs).zip(&ys) {
                    delta.0 += scalar * *x as f32;
                    delta.1 += scalar * *y as f32;
                }
            }
            Some(named) => {
                moved.clear();
                moved.resize(count, (0.0, 0.0));
                touched.clear();
                touched.resize(count, false);
                for ((&point, &x), &y) in named.iter().zip(&xs).zip(&ys) {
                    let point = point as usize;
                    if point < count {
                        moved[point] = (x as f32, y as f32);
                        touched[point] = true;
                    }
                }
                if !ends.is_empty() {
                    infer_untouched(points, ends, &mut moved, &touched);
                }
                for (delta, moved) in deltas.iter_mut().zip(&moved) {
                    delta.0 += scalar * moved.0;
                    delta.1 += scalar * moved.1;
                }
            }
        }
    }
    Some(())
}

/// Moves the points `gvar` didn't name along with the named ones on each side of them in their contour, as the
/// specification says: in proportion between them, or as the nearer one if they're outside them.
pub(crate) fn infer_untouched(
    points: &[(f32, f32)],
    ends: &[usize],
    moved: &mut [(f32, f32)],
    touched: &[bool],
) {
    let mut start = 0;
    for &end in ends {
        if end < start || end >= points.len() {
            return;
        }
        let contour = start..=end;
        start = end + 1;
        let named: Vec<usize> = contour.clone().filter(|&point| touched[point]).collect();
        if named.is_empty() || named.len() == end + 1 - *contour.start() {
            continue;
        }
        if let [only] = named[..] {
            let shift = moved[only];
            for point in contour {
                if !touched[point] {
                    moved[point] = shift;
                }
            }
            continue;
        }
        let first = *contour.start();
        let length = end + 1 - first;
        for (index, &before) in named.iter().enumerate() {
            let after = named[(index + 1) % named.len()];
            // The points from just after `before` to just before `after`, around the contour.
            let mut point = before;
            loop {
                point = first + (point - first + 1) % length;
                if point == after {
                    break;
                }
                moved[point] = (
                    infer(
                        points[point].0,
                        points[before].0,
                        points[after].0,
                        moved[before].0,
                        moved[after].0,
                    ),
                    infer(
                        points[point].1,
                        points[before].1,
                        points[after].1,
                        moved[before].1,
                        moved[after].1,
                    ),
                );
            }
        }
    }
}

/// How far a point at `at` moves, along one direction, between two that are at `a` and `b` and move by `move_a` and
/// `move_b`.
fn infer(at: f32, a: f32, b: f32, move_a: f32, move_b: f32) -> f32 {
    if a == b {
        return if move_a == move_b { move_a } else { 0.0 };
    }
    let (low, high, move_low, move_high) = if a < b {
        (a, b, move_a, move_b)
    } else {
        (b, a, move_b, move_a)
    };
    if at <= low {
        move_low
    } else if at >= high {
        move_high
    } else {
        move_low + (at - low) * (move_high - move_low) / (high - low)
    }
}

/// A font's variation tables, and where it's set along its axes.
pub(crate) struct Variation {
    pub(crate) axes: Vec<VariationAxis>,
    /// Where it's set along each axis, as asked for, in the order of `axes`.
    pub(crate) values: Vec<f32>,
    /// The same, from -1 to 1, in 2.14 fixed point, after `avar`.
    pub(crate) coordinates: Vec<i32>,
    pub(crate) fvar: Range<usize>,
    pub(crate) avar: Option<Range<usize>>,
    pub(crate) gvar: Option<Range<usize>>,
    pub(crate) hvar: Option<Range<usize>>,
    pub(crate) mvar: Option<Range<usize>>,
    /// `GDEF`'s item variation store, which `GPOS`'s values point into, from the start of the file to the end of `GDEF`.
    pub(crate) gdef_store: Option<Range<usize>>,
    pub(crate) hvar_scalars: Vec<f32>,
    pub(crate) mvar_scalars: Vec<f32>,
    pub(crate) gdef_scalars: Vec<f32>,
}

impl Variation {
    /// Whether the font is anywhere but its default, which is where the rest of it already is.
    #[inline]
    pub(crate) fn is_moved(&self) -> bool {
        self.coordinates.iter().any(|&coordinate| coordinate != 0)
    }

    /// Sets the font along its axes, by tag, leaving axes not given at their defaults, and works out what follows.
    pub(crate) fn set(&mut self, bytes: &[u8], settings: &[([u8; 4], f32)]) {
        self.values = self
            .axes
            .iter()
            .map(|axis| {
                settings
                    .iter()
                    .rev()
                    .find(|(tag, _)| *tag == axis.tag)
                    .map_or(axis.default, |(_, value)| value.clamp(axis.min, axis.max))
            })
            .collect();
        let avar = self.avar.clone().and_then(|range| bytes.get(range));
        self.coordinates = self
            .axes
            .iter()
            .zip(&self.values)
            .enumerate()
            .map(|(index, (axis, &value))| {
                let normal = normalize(axis, value);
                to_f2dot14(avar.map_or(normal, |avar| map_through_avar(avar, index, normal)))
            })
            .collect();
        let scalars = |store: Option<&[u8]>| -> Vec<f32> {
            store.map_or_else(Vec::new, |store| region_scalars(store, &self.coordinates))
        };
        let hvar = self
            .hvar
            .clone()
            .and_then(|range| bytes.get(range))
            .and_then(|hvar| hvar.get(read_u32(hvar, 4)? as usize..));
        let mvar = self
            .mvar
            .clone()
            .and_then(|range| bytes.get(range))
            .and_then(|mvar| mvar.get(metrics_store(mvar)?..));
        let gdef = self.gdef_store.clone().and_then(|range| bytes.get(range));
        self.hvar_scalars = scalars(hvar);
        self.mvar_scalars = scalars(mvar);
        self.gdef_scalars = scalars(gdef);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(min: f32, default: f32, max: f32) -> VariationAxis {
        VariationAxis {
            tag: *b"wght",
            min,
            default,
            max,
        }
    }

    #[test]
    fn values_normalize_to_either_side_of_the_default() {
        let weight = axis(300.0, 400.0, 700.0);
        assert_eq!(normalize(&weight, 400.0), 0);
        assert_eq!(normalize(&weight, 700.0), FIXED_ONE);
        assert_eq!(normalize(&weight, 300.0), -FIXED_ONE);
        assert_eq!(normalize(&weight, 350.0), -FIXED_ONE / 2);
        // 600 is two thirds of the way, rounded to the nearest 16.16 number, and then to the nearest 2.14 one.
        assert_eq!(normalize(&weight, 600.0), 43691);
        assert_eq!(to_f2dot14(normalize(&weight, 600.0)), 10923);
        assert_eq!(normalize(&weight, 1000.0), FIXED_ONE);
        assert_eq!(normalize(&weight, f32::NAN), 0);
        let size = axis(5.0, 10.5, 36.0);
        assert_eq!(
            to_f2dot14(normalize(&size, 8.0)),
            (-2.5f32 / 5.5 * 16384.0).round() as i32
        );
        assert_eq!(to_f2dot14(-FIXED_ONE), -ONE);
        assert_eq!(to_f2dot14(2), 1, "halfway rounds up");
    }

    /// An `avar` with one axis, mapping 0 to 0, two thirds to a half, and the ends to themselves, as Segoe UI
    /// Variable's weight is.
    fn avar() -> Vec<u8> {
        let mut avar = vec![0, 1, 0, 0, 0, 0, 0, 2];
        let maps: [(i16, i16); 5] = [
            (-16384, -16384),
            (-8192, -8192),
            (0, 0),
            (10923, 8192),
            (16384, 16384),
        ];
        for axis in 0..2 {
            let maps: &[(i16, i16)] = if axis == 0 { &maps } else { &maps[..0] };
            avar.extend_from_slice(&(maps.len() as u16).to_be_bytes());
            for (from, to) in maps {
                avar.extend_from_slice(&from.to_be_bytes());
                avar.extend_from_slice(&to.to_be_bytes());
            }
        }
        avar
    }

    #[test]
    fn avar_maps_between_its_points() {
        let avar = avar();
        let map = |coordinate: i32| map_through_avar(&avar, 0, coordinate);
        assert_eq!(map(0), 0);
        assert_eq!(map(10923 << 2), 8192 << 2);
        assert_eq!(map(FIXED_ONE), FIXED_ONE);
        assert_eq!(map(-4096 << 2), -4096 << 2);
        // Halfway from two thirds to one is halfway from a half to one.
        let between = ((10923 << 2) + FIXED_ONE) / 2;
        assert!((map(between) - ((8192 << 2) + FIXED_ONE) / 2).abs() <= 1);
        // A third of the way to two thirds is a third of the way to a half.
        assert_eq!(to_f2dot14(map(3641 << 2)), (8192.0f32 / 3.0).round() as i32);
        // An axis with no points, or past the ones it lists, stays as it is.
        assert_eq!(map_through_avar(&avar, 1, 5000), 5000);
        assert_eq!(map_through_avar(&avar, 7, 5000), 5000);
        assert_eq!(map_through_avar(&[0, 1], 0, 5000), 5000);
    }

    #[test]
    fn a_coordinate_is_rounded_to_2_14_only_after_avar() {
        // A weight of 650 in Segoe UI Variable is 0.8333 of the way to its most, 13653 in 2.14, which `avar` maps to
        // 12287.25, so 12287. In 16.16 all the way it's 12287.75, so 12288, as HarfBuzz has it.
        let weight = axis(300.0, 400.0, 700.0);
        let avar = avar();
        assert_eq!(to_f2dot14(map_through_avar(&avar, 0, normalize(&weight, 650.0))), 12288);
    }

    #[test]
    fn a_region_applies_most_at_its_peak() {
        assert_eq!(region_axis_scalar(ONE, 0, ONE, ONE), 1.0);
        assert_eq!(region_axis_scalar(ONE / 2, 0, ONE, ONE), 0.5);
        assert_eq!(region_axis_scalar(-ONE / 2, 0, ONE, ONE), 0.0);
        assert_eq!(region_axis_scalar(0, 0, ONE, ONE), 0.0);
        // A region that doesn't use an axis applies all along it.
        assert_eq!(region_axis_scalar(1234, 0, 0, 0), 1.0);
        // Between peaks: 0.25 of the way from the start at 0 to the peak at 0.5.
        assert_eq!(region_axis_scalar(ONE / 8, 0, ONE / 2, ONE), 0.25);
        assert_eq!(region_axis_scalar(ONE * 3 / 4, 0, ONE / 2, ONE), 0.5);
        // A damaged region, crossing zero, applies everywhere.
        assert_eq!(region_axis_scalar(ONE / 2, -ONE, ONE, ONE), 1.0);
    }

    /// An item variation store with two regions along one axis, at its most and least, and one list of two items: the
    /// first with a word and a byte, the second with two bytes, so that each size is read.
    fn store() -> Vec<u8> {
        let mut store = vec![0, 1];
        store.extend_from_slice(&12u32.to_be_bytes()); // where the regions are
        store.extend_from_slice(&1u16.to_be_bytes()); // one list
        store.extend_from_slice(&28u32.to_be_bytes()); // at 28
        // Regions, at 12: one axis, two regions.
        store.extend_from_slice(&[0, 1, 0, 2]);
        for (start, peak, end) in [(0i16, 16384i16, 16384i16), (-16384, -16384, 0)] {
            store.extend_from_slice(&start.to_be_bytes());
            store.extend_from_slice(&peak.to_be_bytes());
            store.extend_from_slice(&end.to_be_bytes());
        }
        assert_eq!(store.len(), 28);
        // Two items, the first region's delta as a word, two regions, in order.
        store.extend_from_slice(&[0, 2, 0, 1, 0, 2, 0, 0, 0, 1]);
        store.extend_from_slice(&300i16.to_be_bytes());
        store.push((-20i8) as u8);
        store.extend_from_slice(&(-2i16).to_be_bytes());
        store.push(7);
        store
    }

    #[test]
    fn a_store_adds_up_each_region_s_share() {
        let store = store();
        let at_most = region_scalars(&store, &[ONE]);
        assert_eq!(at_most, [1.0, 0.0]);
        let found = Store {
            data: &store,
            scalars: &at_most,
        };
        assert_eq!(found.delta(0, 0), 300.0);
        assert_eq!(found.delta(0, 1), -2.0);
        let halfway_down = region_scalars(&store, &[-ONE / 2]);
        let found = Store {
            data: &store,
            scalars: &halfway_down,
        };
        assert_eq!(found.delta(0, 0), -10.0);
        assert_eq!(found.delta(0, 1), 3.5);
        // Past what it has is no change.
        assert_eq!(found.delta(0, 2), 0.0);
        assert_eq!(found.delta(1, 0), 0.0);
    }

    #[test]
    fn index_maps_split_each_entry() {
        // Format 0, two-byte entries with 4 bits for the inner index: 0x0123 is list 0x12, item 3.
        let map = [0, 0x13, 0, 2, 0x01, 0x23, 0x00, 0x05];
        assert_eq!(map_index(&map, 0), Some((0x12, 3)));
        assert_eq!(map_index(&map, 1), Some((0, 5)));
        // Past the end is the last entry.
        assert_eq!(map_index(&map, 9), Some((0, 5)));
        let empty = [0, 0, 0, 0];
        assert_eq!(map_index(&empty, 0x1_0003), Some((1, 3)));
    }

    /// An `HVAR` with no index map, so glyph numbers are items in the store's first list.
    #[test]
    fn hvar_changes_advances_by_glyph() {
        let mut hvar = vec![0, 1, 0, 0];
        hvar.extend_from_slice(&20u32.to_be_bytes());
        hvar.extend_from_slice(&[0; 12]);
        hvar.extend(store());
        let scalars = region_scalars(&store(), &[ONE / 2]);
        assert_eq!(advance_delta(&hvar, &scalars, 0), 150.0);
        assert_eq!(advance_delta(&hvar, &scalars, 1), -1.0);
        assert_eq!(advance_delta(&hvar, &scalars, 2), 0.0);
    }

    #[test]
    fn points_and_deltas_unpack() {
        // Three points as bytes, 1, 1+2, 3+250, and one more as a word, 253+1000.
        let data = [4, 2, 1, 2, 250, 0x80, 0x03, 0xE8];
        let (points, after) = packed_points(&data, 0).expect("they read");
        assert_eq!(points, Some(vec![1, 3, 253, 1253]));
        assert_eq!(after, data.len());
        assert_eq!(packed_points(&[0], 0), Some((None, 1)));
        // Two zeros, then a word, then a byte.
        let data = [0x81, 0x40, 0x01, 0x00, 0x00, 0xFF];
        let mut deltas = Vec::new();
        let after = packed_deltas(&data, 0, 4, &mut deltas).expect("they read");
        assert_eq!(deltas, [0, 0, 256, -1]);
        assert_eq!(after, data.len());
        assert_eq!(packed_deltas(&data, 0, 9, &mut deltas), None);
    }

    #[test]
    fn untouched_points_follow_their_neighbours() {
        // A square's corners and the middle of each side, going around: the bottom left and top right corners move.
        let points = [
            (0.0, 0.0),
            (50.0, 0.0),
            (100.0, 0.0),
            (100.0, 50.0),
            (100.0, 100.0),
            (50.0, 100.0),
            (0.0, 100.0),
            (0.0, 50.0),
        ];
        let mut moved = [(0.0, 0.0); 8];
        let mut touched = [false; 8];
        moved[0] = (-10.0, -10.0);
        moved[4] = (10.0, 20.0);
        touched[0] = true;
        touched[4] = true;
        infer_untouched(&points, &[7], &mut moved, &touched);
        // Halfway along the bottom moves halfway between the two in x, and with the lower one in y, which it's level
        // with.
        assert_eq!(moved[1], (0.0, -10.0));
        // The bottom right corner is as far right as the top right one, and as low as the bottom left.
        assert_eq!(moved[2], (10.0, -10.0));
        assert_eq!(moved[3], (10.0, 5.0));
        assert_eq!(moved[7], (-10.0, 5.0));
        // One point named moves the whole contour with it.
        let mut moved = [(0.0, 0.0); 8];
        let mut touched = [false; 8];
        moved[5] = (3.0, 4.0);
        touched[5] = true;
        infer_untouched(&points, &[3, 7], &mut moved, &touched);
        assert_eq!(moved[0], (0.0, 0.0));
        assert_eq!(moved[6], (3.0, 4.0));
        // Two named with the same x but moving differently leave the rest where they are in x.
        let mut moved = [(0.0, 0.0); 8];
        let mut touched = [false; 8];
        moved[0] = (5.0, 0.0);
        moved[6] = (-5.0, 0.0);
        touched[0] = true;
        touched[6] = true;
        infer_untouched(&points, &[7], &mut moved, &touched);
        assert_eq!(moved[7].0, 0.0);
    }

    /// A `gvar` for one axis and three glyphs, with one shared tuple at the axis's most: glyph 0 has nothing, glyph 1
    /// moves every point, and glyph 2 has a tuple of its own that's halfway, between two others, naming only one point.
    fn gvar() -> Vec<u8> {
        let mut glyph_1 = Vec::new();
        // One tuple, its data right after the header, which is 8 bytes.
        glyph_1.extend_from_slice(&[0, 1, 0, 8]);
        // Its size and the shared tuple 0, then with no points of its own or shared ones, so every point: x deltas 10 and
        // 20 as two bytes, and y deltas 1 and -1.
        let data_1 = [0x01, 10, 20, 0x01, 1, 0xFF];
        glyph_1.extend_from_slice(&(data_1.len() as u16).to_be_bytes());
        glyph_1.extend_from_slice(&[0, 0]);
        glyph_1.extend_from_slice(&data_1);
        let mut glyph_2 = Vec::new();
        glyph_2.extend_from_slice(&[0, 1, 0, 14]);
        let data_2 = [1, 0, 0, 0, 100, 0, 0];
        glyph_2.extend_from_slice(&(data_2.len() as u16).to_be_bytes());
        // Its own peak, between two others, and its own points.
        glyph_2.extend_from_slice(&(0x8000u16 | 0x4000 | 0x2000).to_be_bytes());
        glyph_2.extend_from_slice(&8192i16.to_be_bytes());
        glyph_2.extend_from_slice(&0i16.to_be_bytes());
        glyph_2.extend_from_slice(&16384i16.to_be_bytes());
        glyph_2.extend_from_slice(&data_2);
        glyph_1.resize(glyph_1.len().next_multiple_of(2), 0);
        glyph_2.resize(glyph_2.len().next_multiple_of(2), 0);
        let mut gvar = vec![0, 1, 0, 0, 0, 1, 0, 1];
        let shared_at = 20 + 4 * 2;
        gvar.extend_from_slice(&(shared_at as u32).to_be_bytes());
        gvar.extend_from_slice(&3u16.to_be_bytes());
        gvar.extend_from_slice(&0u16.to_be_bytes());
        let data_at = shared_at + 2;
        gvar.extend_from_slice(&(data_at as u32).to_be_bytes());
        for offset in [0, 0, glyph_1.len(), glyph_1.len() + glyph_2.len()] {
            gvar.extend_from_slice(&((offset / 2) as u16).to_be_bytes());
        }
        gvar.extend_from_slice(&16384i16.to_be_bytes());
        gvar.extend(glyph_1);
        gvar.extend(glyph_2);
        gvar
    }

    #[test]
    fn shared_tuples_move_every_point() {
        let gvar = gvar();
        let points = [(0.0, 0.0), (100.0, 0.0)];
        let mut deltas = [(0.0, 0.0); 2];
        glyph_deltas(&gvar, 1, &[ONE], &points, &[], &mut deltas);
        assert_eq!(deltas, [(10.0, 1.0), (20.0, -1.0)]);
        let mut deltas = [(0.0, 0.0); 2];
        glyph_deltas(&gvar, 1, &[ONE / 4], &points, &[], &mut deltas);
        assert_eq!(deltas, [(2.5, 0.25), (5.0, -0.25)]);
        // The other way along the axis is outside the tuple.
        let mut deltas = [(0.0, 0.0); 2];
        glyph_deltas(&gvar, 1, &[-ONE], &points, &[], &mut deltas);
        assert_eq!(deltas, [(0.0, 0.0); 2]);
        // A glyph with nothing doesn't move.
        glyph_deltas(&gvar, 0, &[ONE], &points, &[], &mut deltas);
        assert_eq!(deltas, [(0.0, 0.0); 2]);
    }

    #[test]
    fn intermediate_tuples_peak_between_their_ends() {
        let gvar = gvar();
        // Three points in one contour, and the four that say where the glyph starts and ends.
        let points = [
            (0.0, 0.0),
            (100.0, 0.0),
            (50.0, 100.0),
            (0.0, 0.0),
            (100.0, 0.0),
            (0.0, 0.0),
            (0.0, 0.0),
        ];
        let at = |coordinate: i32, ends: &[usize]| {
            let mut deltas = [(0.0, 0.0); 7];
            glyph_deltas(&gvar, 2, &[coordinate], &points, ends, &mut deltas);
            deltas
        };
        // At its peak, a half, the named point moves 100 along x, and the rest of its contour with it.
        let peak = at(ONE / 2, &[2]);
        assert_eq!(peak[0], (100.0, 0.0));
        assert_eq!(peak[1], (100.0, 0.0));
        assert_eq!(peak[2], (100.0, 0.0));
        // Not the points after the contour.
        assert_eq!(peak[3], (0.0, 0.0));
        // Halfway up from its start at 0, and halfway down to its end at 1.
        assert_eq!(at(ONE / 4, &[2])[0], (50.0, 0.0));
        assert_eq!(at(ONE * 3 / 4, &[2])[0], (50.0, 0.0));
        // In a glyph made of others, what isn't named doesn't move.
        let parts = at(ONE / 2, &[]);
        assert_eq!(parts[0], (100.0, 0.0));
        assert_eq!(parts[1], (0.0, 0.0));
    }
}
