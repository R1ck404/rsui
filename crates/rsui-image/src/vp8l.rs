//! Reads the lossless kind of WebP, which is also how a lossy one usually stores its transparency.

use crate::{ImageError, MAX_PIXELS};

const INVALID: ImageError = ImageError::Invalid("its compressed data doesn't make sense");
const SIGNATURE: u8 = 0x2F;
/// Where a code's lengths are read in order, as the most common ones go first.
const CODE_LENGTH_ORDER: [usize; 19] = [
    17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];
const LITERALS: usize = 256;
const LENGTHS: usize = 24;
const DISTANCES: usize = 40;

/// The first distances, which are the nearest pixels in two dimensions, as across and down.
const NEARBY: [(i8, i8); 120] = [
    (0, 1),
    (1, 0),
    (1, 1),
    (-1, 1),
    (0, 2),
    (2, 0),
    (1, 2),
    (-1, 2),
    (2, 1),
    (-2, 1),
    (2, 2),
    (-2, 2),
    (0, 3),
    (3, 0),
    (1, 3),
    (-1, 3),
    (3, 1),
    (-3, 1),
    (2, 3),
    (-2, 3),
    (3, 2),
    (-3, 2),
    (0, 4),
    (4, 0),
    (1, 4),
    (-1, 4),
    (4, 1),
    (-4, 1),
    (3, 3),
    (-3, 3),
    (2, 4),
    (-2, 4),
    (4, 2),
    (-4, 2),
    (0, 5),
    (3, 4),
    (-3, 4),
    (4, 3),
    (-4, 3),
    (5, 0),
    (1, 5),
    (-1, 5),
    (5, 1),
    (-5, 1),
    (2, 5),
    (-2, 5),
    (5, 2),
    (-5, 2),
    (4, 4),
    (-4, 4),
    (3, 5),
    (-3, 5),
    (5, 3),
    (-5, 3),
    (0, 6),
    (6, 0),
    (1, 6),
    (-1, 6),
    (6, 1),
    (-6, 1),
    (2, 6),
    (-2, 6),
    (6, 2),
    (-6, 2),
    (4, 5),
    (-4, 5),
    (5, 4),
    (-5, 4),
    (3, 6),
    (-3, 6),
    (6, 3),
    (-6, 3),
    (0, 7),
    (7, 0),
    (1, 7),
    (-1, 7),
    (5, 5),
    (-5, 5),
    (7, 1),
    (-7, 1),
    (4, 6),
    (-4, 6),
    (6, 4),
    (-6, 4),
    (2, 7),
    (-2, 7),
    (7, 2),
    (-7, 2),
    (3, 7),
    (-3, 7),
    (7, 3),
    (-7, 3),
    (5, 6),
    (-5, 6),
    (6, 5),
    (-6, 5),
    (8, 0),
    (4, 7),
    (-4, 7),
    (7, 4),
    (-7, 4),
    (8, 1),
    (8, 2),
    (6, 6),
    (-6, 6),
    (8, 3),
    (5, 7),
    (-5, 7),
    (7, 5),
    (-7, 5),
    (8, 4),
    (6, 7),
    (-6, 7),
    (7, 6),
    (-7, 6),
    (8, 5),
    (7, 7),
    (-7, 7),
    (8, 6),
    (8, 7),
];

/// Reads bits, lowest first.
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    held: u64,
    count: u32,
    /// How many bits were made up past the end.
    made_up: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            at: 0,
            held: 0,
            count: 0,
            made_up: 0,
        }
    }

    #[inline]
    fn fill(&mut self) {
        while self.count <= 56 {
            let byte = match self.data.get(self.at) {
                Some(&byte) => {
                    self.at += 1;
                    byte
                }
                None => {
                    self.made_up += 8;
                    0
                }
            };
            self.held |= u64::from(byte) << self.count;
            self.count += 8;
        }
    }

    #[inline]
    fn take(&mut self, bits: u32) -> u32 {
        if bits == 0 {
            return 0;
        }
        if self.count < bits {
            self.fill();
        }
        let value = self.held & ((1u64 << bits) - 1);
        self.held >>= bits;
        self.count -= bits;
        value as u32
    }

    /// Whether this has read further past the end than anything could need.
    fn ran_out(&self) -> bool {
        self.made_up > 128
    }
}

/// A set of codes, ready to look up. Codes are read lowest bit first.
struct Codes {
    /// By the next 8 bits, the symbol and the length of its code, or 0 if it's longer.
    fast: [u16; 256],
    /// For longer ones: where each length's codes start, and where their symbols are.
    first_code: [u32; 16],
    first_index: [u16; 16],
    count: [u16; 16],
    symbols: Vec<u16>,
    /// The only symbol, if there's only one, which then takes no bits at all.
    only: Option<u16>,
}

impl Codes {
    fn new(lengths: &[u8]) -> Result<Self, ImageError> {
        let mut count = [0u16; 16];
        let mut used = 0usize;
        let mut last = 0u16;
        for (symbol, &length) in lengths.iter().enumerate() {
            if length > 0 {
                count[usize::from(length)] += 1;
                used += 1;
                last = symbol as u16;
            }
        }
        let mut codes = Self {
            fast: [0; 256],
            first_code: [0; 16],
            first_index: [0; 16],
            count,
            symbols: vec![0; used],
            only: None,
        };
        if used == 0 {
            return Err(INVALID);
        }
        if used == 1 {
            codes.only = Some(last);
            return Ok(codes);
        }
        let (mut code, mut index) = (0u32, 0u16);
        for (length, &how_many) in count.iter().enumerate().skip(1) {
            codes.first_code[length] = code;
            codes.first_index[length] = index;
            code = (code + u32::from(how_many)) << 1;
            index += how_many;
        }
        // It has to use up all the codes there are, neither too few nor too many.
        if code != 1 << 16 {
            return Err(INVALID);
        }
        let mut next_code = codes.first_code;
        let mut next_index = codes.first_index;
        for (symbol, &length) in lengths.iter().enumerate() {
            if length == 0 {
                continue;
            }
            let bits = usize::from(length);
            codes.symbols[usize::from(next_index[bits])] = symbol as u16;
            next_index[bits] += 1;
            let value = next_code[bits];
            next_code[bits] += 1;
            if bits <= 8 {
                let reversed = (value.reverse_bits() >> (32 - bits)) as usize;
                let entry = ((symbol as u16) << 4) | u16::from(length);
                let mut at = reversed;
                while at < 256 {
                    codes.fast[at] = entry;
                    at += 1 << bits;
                }
            }
        }
        Ok(codes)
    }

    #[inline]
    fn read(&self, bits: &mut Bits<'_>) -> Result<u16, ImageError> {
        if let Some(only) = self.only {
            return Ok(only);
        }
        if bits.count < 15 {
            bits.fill();
        }
        let entry = self.fast[(bits.held & 0xFF) as usize];
        if entry != 0 {
            let length = u32::from(entry & 15);
            bits.held >>= length;
            bits.count -= length;
            return Ok(entry >> 4);
        }
        let mut code = 0u32;
        for length in 1..16usize {
            code = (code << 1) | (bits.held & 1) as u32;
            bits.held >>= 1;
            bits.count -= 1;
            let offset = code.wrapping_sub(self.first_code[length]);
            if offset < u32::from(self.count[length]) {
                let index = usize::from(self.first_index[length]) + offset as usize;
                return self.symbols.get(index).copied().ok_or(INVALID);
            }
        }
        Err(INVALID)
    }
}

fn read_code(bits: &mut Bits<'_>, alphabet: usize) -> Result<Codes, ImageError> {
    let mut lengths = vec![0u8; alphabet];
    if bits.take(1) == 1 {
        // One or two symbols, written out.
        let two = bits.take(1) == 1;
        let first_is_wide = bits.take(1) == 1;
        let first = bits.take(if first_is_wide { 8 } else { 1 }) as usize;
        *lengths.get_mut(first).ok_or(INVALID)? = 1;
        if two {
            let second = bits.take(8) as usize;
            *lengths.get_mut(second).ok_or(INVALID)? = 1;
        }
        return Codes::new(&lengths);
    }
    let mut length_lengths = [0u8; 19];
    let count = bits.take(4) as usize + 4;
    for &position in &CODE_LENGTH_ORDER[..count] {
        length_lengths[position] = bits.take(3) as u8;
    }
    let length_codes = Codes::new(&length_lengths)?;
    let mut remaining = if bits.take(1) == 1 {
        let width = 2 + 2 * bits.take(3);
        let most = 2 + bits.take(width) as usize;
        if most > alphabet {
            return Err(INVALID);
        }
        most
    } else {
        alphabet
    };
    let mut symbol = 0;
    let mut previous = 8u8;
    while symbol < alphabet && remaining > 0 {
        remaining -= 1;
        let code = length_codes.read(bits)?;
        if code < 16 {
            lengths[symbol] = code as u8;
            symbol += 1;
            if code != 0 {
                previous = code as u8;
            }
            continue;
        }
        let (extra, base, value) = match code {
            16 => (2, 3, previous),
            17 => (3, 3, 0),
            _ => (7, 11, 0),
        };
        let repeat = base + bits.take(extra) as usize;
        if symbol + repeat > alphabet {
            return Err(INVALID);
        }
        lengths[symbol..symbol + repeat].fill(value);
        symbol += repeat;
    }
    Codes::new(&lengths)
}

struct Group {
    green: Codes,
    red: Codes,
    blue: Codes,
    alpha: Codes,
    distance: Codes,
}

fn read_group(bits: &mut Bits<'_>, cache_size: usize) -> Result<Group, ImageError> {
    Ok(Group {
        green: read_code(bits, LITERALS + LENGTHS + cache_size)?,
        red: read_code(bits, LITERALS)?,
        blue: read_code(bits, LITERALS)?,
        alpha: read_code(bits, LITERALS)?,
        distance: read_code(bits, DISTANCES)?,
    })
}

#[inline]
fn prefix_value(prefix: u32, bits: &mut Bits<'_>) -> u32 {
    if prefix < 4 {
        return prefix + 1;
    }
    let extra = (prefix - 2) >> 1;
    let offset = (2 + (prefix & 1)) << extra;
    offset + bits.take(extra) + 1
}

/// How far back a distance code means, in pixels along the image.
#[inline]
fn real_distance(code: u32, width: usize) -> usize {
    if code > 120 {
        return code as usize - 120;
    }
    let (across, down) = NEARBY[code as usize - 1];
    let distance = i64::from(across) + i64::from(down) * width as i64;
    distance.max(1) as usize
}

#[inline]
fn subsampled(size: usize, bits: u32) -> usize {
    (size + (1 << bits) - 1) >> bits
}

/// Reads a run of pixels. `top_level` says whether this is the image itself rather than something
/// that helps to read it, which is the only kind that can use different codes in different places.
fn read_pixels(
    bits: &mut Bits<'_>,
    width: usize,
    height: usize,
    top_level: bool,
) -> Result<Vec<u32>, ImageError> {
    let cache_bits = if bits.take(1) == 1 {
        let cache_bits = bits.take(4);
        if !(1..=11).contains(&cache_bits) {
            return Err(INVALID);
        }
        cache_bits
    } else {
        0
    };
    let cache_size = if cache_bits > 0 { 1 << cache_bits } else { 0 };
    let mut groups_of: Option<(Vec<u32>, u32, usize)> = None;
    let mut group_count = 1;
    if top_level && bits.take(1) == 1 {
        let size_bits = bits.take(3) + 2;
        let across = subsampled(width, size_bits);
        let down = subsampled(height, size_bits);
        let map = read_pixels(bits, across, down, false)?;
        let mut highest = 0;
        let map: Vec<u32> = map
            .into_iter()
            .map(|pixel| {
                let group = (pixel >> 8) & 0xFFFF;
                highest = highest.max(group);
                group
            })
            .collect();
        group_count = highest as usize + 1;
        groups_of = Some((map, size_bits, across));
    }
    let mut groups = Vec::new();
    groups
        .try_reserve_exact(group_count)
        .map_err(|_| ImageError::TooBig)?;
    for _ in 0..group_count {
        groups.push(read_group(bits, cache_size)?);
    }

    let total = width.checked_mul(height).ok_or(ImageError::TooBig)?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(total)
        .map_err(|_| ImageError::TooBig)?;
    pixels.resize(total, 0u32);
    let mut cache = vec![0u32; cache_size];
    let hash_shift = 32 - cache_bits;
    let mut at = 0;
    let mut group_index = 0usize;
    let mut last_block = usize::MAX;
    while at < total {
        if let Some((map, size_bits, across)) = &groups_of {
            let (x, y) = (at % width, at / width);
            let block = (y >> size_bits) * across + (x >> size_bits);
            if block != last_block {
                last_block = block;
                group_index = *map.get(block).ok_or(INVALID)? as usize;
            }
        }
        let group = groups.get(group_index).ok_or(INVALID)?;
        let symbol = u32::from(group.green.read(bits)?);
        if bits.ran_out() {
            return Err(INVALID);
        }
        if (symbol as usize) < LITERALS {
            let red = u32::from(group.red.read(bits)?);
            let blue = u32::from(group.blue.read(bits)?);
            let alpha = u32::from(group.alpha.read(bits)?);
            let pixel = (alpha << 24) | (red << 16) | (symbol << 8) | blue;
            pixels[at] = pixel;
            at += 1;
            if cache_size > 0 {
                cache[(pixel.wrapping_mul(0x1E35_A7BD) >> hash_shift) as usize] = pixel;
            }
        } else if (symbol as usize) < LITERALS + LENGTHS {
            let length = prefix_value(symbol - LITERALS as u32, bits) as usize;
            let code = u32::from(group.distance.read(bits)?);
            let distance_code = prefix_value(code, bits);
            let distance = real_distance(distance_code, width);
            if distance > at || at + length > total {
                return Err(INVALID);
            }
            for index in at..at + length {
                let pixel = pixels[index - distance];
                pixels[index] = pixel;
                if cache_size > 0 {
                    cache[(pixel.wrapping_mul(0x1E35_A7BD) >> hash_shift) as usize] = pixel;
                }
            }
            at += length;
        } else {
            let index = symbol as usize - LITERALS - LENGTHS;
            let pixel = *cache.get(index).ok_or(INVALID)?;
            pixels[at] = pixel;
            at += 1;
            cache[(pixel.wrapping_mul(0x1E35_A7BD) >> hash_shift) as usize] = pixel;
        }
    }
    Ok(pixels)
}

enum Transform {
    Predictor {
        bits: u32,
        data: Vec<u32>,
    },
    Color {
        bits: u32,
        data: Vec<u32>,
    },
    SubtractGreen,
    /// The table, how many bits each pixel was squeezed into, and how wide it was before that.
    ColorIndex {
        table: Vec<u32>,
        pack_bits: u32,
        width: usize,
    },
}

#[inline]
fn average(a: u32, b: u32) -> u32 {
    (((a ^ b) & 0xFEFE_FEFE) >> 1) + (a & b)
}

#[inline]
fn add_pixels(a: u32, b: u32) -> u32 {
    let high = (a & 0xFF00_FF00).wrapping_add(b & 0xFF00_FF00) & 0xFF00_FF00;
    let low = (a & 0x00FF_00FF).wrapping_add(b & 0x00FF_00FF) & 0x00FF_00FF;
    high | low
}

#[inline]
fn parts(pixel: u32) -> [i32; 4] {
    [
        (pixel >> 24) as i32,
        ((pixel >> 16) & 255) as i32,
        ((pixel >> 8) & 255) as i32,
        (pixel & 255) as i32,
    ]
}

#[inline]
fn from_parts(parts: [i32; 4]) -> u32 {
    let clamp = |value: i32| value.clamp(0, 255) as u32;
    (clamp(parts[0]) << 24) | (clamp(parts[1]) << 16) | (clamp(parts[2]) << 8) | clamp(parts[3])
}

#[inline]
fn select(left: u32, top: u32, corner: u32) -> u32 {
    let (l, t, c) = (parts(left), parts(top), parts(corner));
    let (mut to_left, mut to_top) = (0, 0);
    for index in 0..4 {
        to_left += (t[index] - c[index]).abs();
        to_top += (l[index] - c[index]).abs();
    }
    if to_left < to_top { left } else { top }
}

#[inline]
fn add_subtract_full(left: u32, top: u32, corner: u32) -> u32 {
    let (l, t, c) = (parts(left), parts(top), parts(corner));
    from_parts([
        l[0] + t[0] - c[0],
        l[1] + t[1] - c[1],
        l[2] + t[2] - c[2],
        l[3] + t[3] - c[3],
    ])
}

#[inline]
fn add_subtract_half(left: u32, top: u32, corner: u32) -> u32 {
    let (a, c) = (parts(average(left, top)), parts(corner));
    let half = |a: i32, c: i32| a + (a - c) / 2;
    from_parts([
        half(a[0], c[0]),
        half(a[1], c[1]),
        half(a[2], c[2]),
        half(a[3], c[3]),
    ])
}

/// Undoes each pixel being stored as how far it is from a guess made from those before it.
fn undo_predictor(pixels: &mut [u32], width: usize, height: usize, bits: u32, data: &[u32]) {
    let across = subsampled(width, bits);
    for y in 0..height {
        for x in 0..width {
            let at = y * width + x;
            let guess = if y == 0 && x == 0 {
                0xFF00_0000
            } else if y == 0 {
                pixels[at - 1]
            } else if x == 0 {
                pixels[at - width]
            } else {
                let block = (y >> bits) * across + (x >> bits);
                let mode = data.get(block).map_or(0, |pixel| (pixel >> 8) & 15);
                let (left, top, corner) =
                    (pixels[at - 1], pixels[at - width], pixels[at - width - 1]);
                // On the far right, what's up and to the right is the start of this row.
                let top_right = pixels[at - width + 1];
                match mode {
                    1 => left,
                    2 => top,
                    3 => top_right,
                    4 => corner,
                    5 => average(average(left, top_right), top),
                    6 => average(left, corner),
                    7 => average(left, top),
                    8 => average(corner, top),
                    9 => average(top, top_right),
                    10 => average(average(left, corner), average(top, top_right)),
                    11 => select(left, top, corner),
                    12 => add_subtract_full(left, top, corner),
                    13 => add_subtract_half(left, top, corner),
                    _ => 0xFF00_0000,
                }
            };
            pixels[at] = add_pixels(pixels[at], guess);
        }
    }
}

#[inline]
fn color_delta(transform: u32, color: u32) -> i32 {
    (i32::from((transform & 255) as u8 as i8) * i32::from((color & 255) as u8 as i8)) >> 5
}

fn undo_color(pixels: &mut [u32], width: usize, height: usize, bits: u32, data: &[u32]) {
    let across = subsampled(width, bits);
    for y in 0..height {
        for x in 0..width {
            let at = y * width + x;
            let element = data
                .get((y >> bits) * across + (x >> bits))
                .copied()
                .unwrap_or(0);
            let pixel = pixels[at];
            let green = (pixel >> 8) & 255;
            let red = ((pixel >> 16) & 255) as i32 + color_delta(element, green);
            let red = red & 255;
            let blue = (pixel & 255) as i32
                + color_delta(element >> 8, green)
                + color_delta(element >> 16, red as u32);
            pixels[at] = (pixel & 0xFF00_FF00) | ((red as u32) << 16) | ((blue & 255) as u32);
        }
    }
}

fn undo_color_index(
    pixels: Vec<u32>,
    packed_width: usize,
    height: usize,
    table: &[u32],
    pack_bits: u32,
    width: usize,
) -> Result<Vec<u32>, ImageError> {
    let total = width.checked_mul(height).ok_or(ImageError::TooBig)?;
    let mut out = Vec::new();
    out.try_reserve_exact(total)
        .map_err(|_| ImageError::TooBig)?;
    out.resize(total, 0u32);
    if pack_bits == 0 {
        for (to, from) in out.iter_mut().zip(&pixels) {
            *to = table
                .get(((from >> 8) & 255) as usize)
                .copied()
                .unwrap_or(0);
        }
        return Ok(out);
    }
    let per_pixel = 8 >> pack_bits;
    let mask = (1u32 << per_pixel) - 1;
    for y in 0..height {
        for x in 0..width {
            let packed = pixels[y * packed_width + (x >> pack_bits)];
            let green = (packed >> 8) & 255;
            let shift = (x as u32 & ((1 << pack_bits) - 1)) * per_pixel;
            let index = (green >> shift) & mask;
            out[y * width + x] = table.get(index as usize).copied().unwrap_or(0);
        }
    }
    Ok(out)
}

/// Reads a stream of transforms and pixels, which has no header of its own, into pixels as alpha, red,
/// green and blue, with alpha in the top byte.
pub(crate) fn read_stream(
    data: &[u8],
    width: usize,
    height: usize,
) -> Result<Vec<u32>, ImageError> {
    let mut bits = Bits::new(data);
    read_stream_from(&mut bits, width, height)
}

fn read_stream_from(
    bits: &mut Bits<'_>,
    width: usize,
    height: usize,
) -> Result<Vec<u32>, ImageError> {
    if (width as u64) * (height as u64) > MAX_PIXELS {
        return Err(ImageError::TooBig);
    }
    let mut transforms = Vec::new();
    let mut packed_width = width;
    let mut seen = 0u32;
    while bits.take(1) == 1 {
        let kind = bits.take(2);
        // Each can only be used once.
        if seen & (1 << kind) != 0 {
            return Err(INVALID);
        }
        seen |= 1 << kind;
        match kind {
            0 | 1 => {
                let size_bits = bits.take(3) + 2;
                let data = read_pixels(
                    bits,
                    subsampled(packed_width, size_bits),
                    subsampled(height, size_bits),
                    false,
                )?;
                transforms.push(if kind == 0 {
                    Transform::Predictor {
                        bits: size_bits,
                        data,
                    }
                } else {
                    Transform::Color {
                        bits: size_bits,
                        data,
                    }
                });
            }
            2 => transforms.push(Transform::SubtractGreen),
            _ => {
                let size = bits.take(8) as usize + 1;
                let mut table = read_pixels(bits, size, 1, false)?;
                for index in 1..table.len() {
                    table[index] = add_pixels(table[index], table[index - 1]);
                }
                let pack_bits = match size {
                    0..=2 => 3,
                    3..=4 => 2,
                    5..=16 => 1,
                    _ => 0,
                };
                transforms.push(Transform::ColorIndex {
                    table,
                    pack_bits,
                    width: packed_width,
                });
                packed_width = subsampled(packed_width, pack_bits);
            }
        }
    }
    let mut pixels = read_pixels(bits, packed_width, height, true)?;
    for transform in transforms.iter().rev() {
        match transform {
            Transform::Predictor { bits, data } => {
                undo_predictor(&mut pixels, packed_width, height, *bits, data);
            }
            Transform::Color { bits, data } => {
                undo_color(&mut pixels, packed_width, height, *bits, data);
            }
            Transform::SubtractGreen => {
                for pixel in &mut pixels {
                    let green = (*pixel >> 8) & 255;
                    let red = (((*pixel >> 16) & 255) + green) & 255;
                    let blue = ((*pixel & 255) + green) & 255;
                    *pixel = (*pixel & 0xFF00_FF00) | (red << 16) | blue;
                }
            }
            Transform::ColorIndex {
                table,
                pack_bits,
                width,
            } => {
                pixels = undo_color_index(pixels, packed_width, height, table, *pack_bits, *width)?;
                packed_width = *width;
            }
        }
    }
    Ok(pixels)
}

/// The size in a lossless chunk's header.
pub(crate) fn size_of(data: &[u8]) -> Result<(u32, u32), ImageError> {
    if data.first() != Some(&SIGNATURE) || data.len() < 5 {
        return Err(ImageError::Invalid("its header is wrong"));
    }
    let value = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
    Ok(((value & 0x3FFF) + 1, ((value >> 14) & 0x3FFF) + 1))
}

/// Reads a whole lossless chunk into pixels.
pub(crate) fn decode(data: &[u8]) -> Result<(u32, u32, Vec<u32>), ImageError> {
    let (width, height) = size_of(data)?;
    let mut bits = Bits::new(data.get(5..).ok_or(INVALID)?);
    // The version has to be zero. The bit before it only says whether alpha is worth looking at.
    if u32::from_le_bytes([data[1], data[2], data[3], data[4]]) >> 29 != 0 {
        return Err(ImageError::Unsupported("this version of lossless WebP"));
    }
    let pixels = read_stream_from(&mut bits, width as usize, height as usize)?;
    Ok((width, height, pixels))
}
