//! Reads JPEG files, baseline and progressive, with 8 or 12 bits, by Huffman or arithmetic coding, at
//! whichever fraction of their size is enough.

use crate::color::{ColorTransform, ProfileKind};
use crate::scale::{Scaler, orient};
use crate::{ImageError, ImageInfo, MAX_PIXELS, Picture, fitted};

const INVALID: ImageError = ImageError::Invalid("its compressed data doesn't make sense");
/// How many scans a file can have. Real ones have about ten, and each costs time for every block.
const MAX_SCANS: usize = 64;
/// A progressive file holds numbers for each pixel until its last scan, so it gets a lower limit.
const MAX_PROGRESSIVE_PIXELS: u64 = 1 << 26;

/// Where each entry of a block goes, when read in zigzag order.
const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

pub(crate) fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8])
}

#[derive(Clone, Copy, Default)]
struct Component {
    id: u8,
    h: usize,
    v: usize,
    table: usize,
    dc_table: usize,
    ac_table: usize,
    /// Where it's got to, for working out each block's first number from the last.
    dc_predictor: i32,
}

#[derive(Default)]
struct Frame {
    progressive: bool,
    arithmetic: bool,
    twelve_bits: bool,
    /// Keeps every number exactly, with no blocks to it.
    lossless: bool,
    /// How many bits there are to each number. Only anything but 8 or 12 if it's lossless.
    precision: u32,
    width: u32,
    height: u32,
    components: Vec<Component>,
    h_max: usize,
    v_max: usize,
}

impl Frame {
    fn mcus(&self) -> (usize, usize) {
        (
            (self.width as usize).div_ceil(8 * self.h_max),
            (self.height as usize).div_ceil(8 * self.v_max),
        )
    }

    /// How many blocks across and down a component holds, counting padding to fill out each unit.
    fn blocks(&self, component: &Component) -> (usize, usize) {
        let (across, down) = self.mcus();
        (across * component.h, down * component.v)
    }

    /// How many blocks across and down a scan of only this component has, which leaves out padding.
    fn own_blocks(&self, component: &Component) -> (usize, usize) {
        let samples =
            |size: u32, part: usize, most: usize| (size as usize * part).div_ceil(most).div_ceil(8);
        (
            samples(self.width, component.h, self.h_max),
            samples(self.height, component.v, self.v_max),
        )
    }
}

/// A set of codes, ready to look up.
#[derive(Clone)]
struct Codes {
    /// By the next 8 bits, the symbol and how long its code is, or 0 if it's longer.
    fast: [u16; 256],
    max_code: [i32; 18],
    value_offset: [i32; 17],
    values: [u8; 256],
}

impl Codes {
    fn new(counts: &[u8], values: &[u8]) -> Result<Self, ImageError> {
        let mut table = Self {
            fast: [0; 256],
            max_code: [-1; 18],
            value_offset: [0; 17],
            values: [0; 256],
        };
        table.values[..values.len()].copy_from_slice(values);
        let (mut code, mut index) = (0i32, 0i32);
        for length in 1..=16usize {
            let count = i32::from(counts[length - 1]);
            if count > 0 {
                table.value_offset[length] = index - code;
                for _ in 0..count {
                    if length <= 8 {
                        let start = (code as usize) << (8 - length);
                        let entry = (u16::from(table.values[index as usize]) << 8) | length as u16;
                        for slot in table.fast.iter_mut().skip(start).take(1 << (8 - length)) {
                            *slot = entry;
                        }
                    }
                    code += 1;
                    index += 1;
                }
                if code > 1 << length {
                    return Err(INVALID);
                }
                table.max_code[length] = code - 1;
            }
            code <<= 1;
        }
        table.max_code[17] = i32::MAX;
        Ok(table)
    }
}

/// Reads bits, top first, taking out the padding JPEG adds after each 0xFF.
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    held: u32,
    count: u32,
    /// How many bits were made up past the end, which is only allowed so far.
    made_up: u32,
}

impl Bits<'_> {
    fn fill(&mut self) {
        while self.count <= 24 {
            let byte = match self.data.get(self.at) {
                Some(&0xFF) => match self.data.get(self.at + 1) {
                    Some(&0x00) => {
                        self.at += 2;
                        0xFF
                    }
                    // A marker. The scan's data has ended, so what's needed is made up.
                    _ => {
                        self.made_up += 8;
                        0
                    }
                },
                Some(&byte) => {
                    self.at += 1;
                    byte
                }
                None => {
                    self.made_up += 8;
                    0
                }
            };
            self.held |= u32::from(byte) << (24 - self.count);
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
        let value = self.held >> (32 - bits);
        self.held <<= bits;
        self.count -= bits;
        value
    }

    /// Whether this has read so far past the data that nothing's coming.
    fn ran_out(&self) -> bool {
        self.made_up > 64
    }

    #[inline]
    fn symbol(&mut self, codes: &Codes) -> Result<u8, ImageError> {
        if self.count < 16 {
            self.fill();
        }
        let entry = codes.fast[(self.held >> 24) as usize];
        if entry != 0 {
            let length = u32::from(entry & 0xFF);
            self.held <<= length;
            self.count -= length;
            return Ok((entry >> 8) as u8);
        }
        let top = (self.held >> 16) as i32;
        for length in 9..=16usize {
            if (top >> (16 - length)) <= codes.max_code[length] {
                let index = (top >> (16 - length)) + codes.value_offset[length];
                self.held <<= length;
                self.count -= length as u32;
                return codes.values.get(index as usize).copied().ok_or(INVALID);
            }
        }
        Err(INVALID)
    }

    /// Reads a number of this many bits, whose first bit says if it's negative.
    #[inline]
    fn signed(&mut self, bits: u32) -> i32 {
        if bits == 0 {
            return 0;
        }
        let value = self.take(bits) as i32;
        if value < 1 << (bits - 1) {
            value - (1 << bits) + 1
        } else {
            value
        }
    }

    /// Goes to a restart marker, and past it.
    fn restart(&mut self) {
        self.held = 0;
        self.count = 0;
        while let Some(&byte) = self.data.get(self.at) {
            if byte == 0xFF {
                match self.data.get(self.at + 1) {
                    Some(0xD0..=0xD7) => {
                        self.at += 2;
                        return;
                    }
                    Some(0x00) => self.at += 2,
                    // Some other marker: the data's over, so leave it for whoever reads that.
                    Some(_) | None => return,
                }
            } else {
                self.at += 1;
            }
        }
    }
}

/// How the odds change as things are read: odds, what's next if it's the unlikely one, what's next if
/// it's the likely one, and whether which is which swaps. From the JPEG standard's table D.3.
const STATES: [(u16, u8, u8, bool); 114] = [
    (0x5a1d, 1, 1, true),
    (0x2586, 14, 2, false),
    (0x1114, 16, 3, false),
    (0x080b, 18, 4, false),
    (0x03d8, 20, 5, false),
    (0x01da, 23, 6, false),
    (0x00e5, 25, 7, false),
    (0x006f, 28, 8, false),
    (0x0036, 30, 9, false),
    (0x001a, 33, 10, false),
    (0x000d, 35, 11, false),
    (0x0006, 9, 12, false),
    (0x0003, 10, 13, false),
    (0x0001, 12, 13, false),
    (0x5a7f, 15, 15, true),
    (0x3f25, 36, 16, false),
    (0x2cf2, 38, 17, false),
    (0x207c, 39, 18, false),
    (0x17b9, 40, 19, false),
    (0x1182, 42, 20, false),
    (0x0cef, 43, 21, false),
    (0x09a1, 45, 22, false),
    (0x072f, 46, 23, false),
    (0x055c, 48, 24, false),
    (0x0406, 49, 25, false),
    (0x0303, 51, 26, false),
    (0x0240, 52, 27, false),
    (0x01b1, 54, 28, false),
    (0x0144, 56, 29, false),
    (0x00f5, 57, 30, false),
    (0x00b7, 59, 31, false),
    (0x008a, 60, 32, false),
    (0x0068, 62, 33, false),
    (0x004e, 63, 34, false),
    (0x003b, 32, 35, false),
    (0x002c, 33, 9, false),
    (0x5ae1, 37, 37, true),
    (0x484c, 64, 38, false),
    (0x3a0d, 65, 39, false),
    (0x2ef1, 67, 40, false),
    (0x261f, 68, 41, false),
    (0x1f33, 69, 42, false),
    (0x19a8, 70, 43, false),
    (0x1518, 72, 44, false),
    (0x1177, 73, 45, false),
    (0x0e74, 74, 46, false),
    (0x0bfb, 75, 47, false),
    (0x09f8, 77, 48, false),
    (0x0861, 78, 49, false),
    (0x0706, 79, 50, false),
    (0x05cd, 48, 51, false),
    (0x04de, 50, 52, false),
    (0x040f, 50, 53, false),
    (0x0363, 51, 54, false),
    (0x02d4, 52, 55, false),
    (0x025c, 53, 56, false),
    (0x01f8, 54, 57, false),
    (0x01a4, 55, 58, false),
    (0x0160, 56, 59, false),
    (0x0125, 57, 60, false),
    (0x00f6, 58, 61, false),
    (0x00cb, 59, 62, false),
    (0x00ab, 61, 63, false),
    (0x008f, 61, 32, false),
    (0x5b12, 65, 65, true),
    (0x4d04, 80, 66, false),
    (0x412c, 81, 67, false),
    (0x37d8, 82, 68, false),
    (0x2fe8, 83, 69, false),
    (0x293c, 84, 70, false),
    (0x2379, 86, 71, false),
    (0x1edf, 87, 72, false),
    (0x1aa9, 87, 73, false),
    (0x174e, 72, 74, false),
    (0x1424, 72, 75, false),
    (0x119c, 74, 76, false),
    (0x0f6b, 74, 77, false),
    (0x0d51, 75, 78, false),
    (0x0bb6, 77, 79, false),
    (0x0a40, 77, 48, false),
    (0x5832, 80, 81, true),
    (0x4d1c, 88, 82, false),
    (0x438e, 89, 83, false),
    (0x3bdd, 90, 84, false),
    (0x34ee, 91, 85, false),
    (0x2eae, 92, 86, false),
    (0x299a, 93, 87, false),
    (0x2516, 86, 71, false),
    (0x5570, 88, 89, true),
    (0x4ca9, 95, 90, false),
    (0x44d9, 96, 91, false),
    (0x3e22, 97, 92, false),
    (0x3824, 99, 93, false),
    (0x32b4, 99, 94, false),
    (0x2e17, 93, 86, false),
    (0x56a8, 95, 96, true),
    (0x4f46, 101, 97, false),
    (0x47e5, 102, 98, false),
    (0x41cf, 103, 99, false),
    (0x3c3d, 104, 100, false),
    (0x375e, 99, 93, false),
    (0x5231, 105, 102, false),
    (0x4c0f, 106, 103, false),
    (0x4639, 107, 104, false),
    (0x415e, 103, 99, false),
    (0x5627, 105, 106, true),
    (0x50e7, 108, 107, false),
    (0x4b85, 109, 103, false),
    (0x5597, 110, 109, false),
    (0x504f, 111, 107, false),
    (0x5a10, 110, 111, true),
    (0x5522, 112, 109, false),
    (0x59eb, 112, 111, true),
    (0x5a1d, 113, 113, false),
];

/// The state that's always one half, for what's just as likely either way.
const EVEN_ODDS: u8 = 113;

/// Reads answers from arithmetic-coded data, as in annex D of the JPEG standard.
struct ArithmeticReader<'a> {
    data: &'a [u8],
    at: usize,
    code: u64,
    range: u32,
    shift: i32,
    /// Having reached a marker is allowed here, and everything after it reads as zeros.
    reached_marker: bool,
}

impl<'a> ArithmeticReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            at: 0,
            code: 0,
            range: 0,
            shift: -16,
            reached_marker: false,
        }
    }

    fn start_over(&mut self) {
        self.code = 0;
        self.range = 0;
        self.shift = -16;
        self.reached_marker = false;
    }

    fn next_byte(&mut self) -> u8 {
        if self.reached_marker {
            return 0;
        }
        let Some(&byte) = self.data.get(self.at) else {
            self.reached_marker = true;
            return 0;
        };
        if byte != 0xFF {
            self.at += 1;
            return byte;
        }
        let mut after = self.at + 1;
        while self.data.get(after) == Some(&0xFF) {
            after += 1;
        }
        if self.data.get(after) == Some(&0) {
            self.at = after + 1;
            return 0xFF;
        }
        // It stays where it is, for whatever reads that marker.
        self.reached_marker = true;
        0
    }

    #[inline]
    fn read(&mut self, state: &mut u8) -> bool {
        while self.range < 0x8000 {
            self.shift -= 1;
            if self.shift < 0 {
                let byte = self.next_byte();
                self.code = (self.code << 8) | u64::from(byte);
                self.shift += 8;
                if self.shift < 0 {
                    self.shift += 1;
                    if self.shift == 0 {
                        // The first two bytes are in.
                        self.range = 0x8000;
                    }
                }
            }
            self.range <<= 1;
        }
        let value = *state;
        let (odds, next_unlikely, next_likely, swaps) = STATES[usize::from(value & 0x7F)];
        let odds = u32::from(odds);
        let unlikely = (value & 0x80) ^ next_unlikely ^ (u8::from(swaps) << 7);
        let likely = (value & 0x80) ^ next_likely;
        let mut answer = value >> 7;
        self.range -= odds;
        let top = u64::from(self.range) << self.shift;
        if self.code >= top {
            self.code -= top;
            if self.range < odds {
                *state = likely;
            } else {
                *state = unlikely;
                answer ^= 1;
            }
            self.range = odds;
        } else if self.range < 0x8000 {
            if self.range < odds {
                *state = unlikely;
                answer ^= 1;
            } else {
                *state = likely;
            }
        }
        answer != 0
    }

    /// Goes to a restart marker, and past it.
    fn restart(&mut self) {
        self.start_over();
        while let Some(&byte) = self.data.get(self.at) {
            if byte == 0xFF {
                match self.data.get(self.at + 1) {
                    Some(0xD0..=0xD7) => {
                        self.at += 2;
                        return;
                    }
                    Some(0x00 | 0xFF) => self.at += 1,
                    Some(_) | None => {
                        self.reached_marker = true;
                        return;
                    }
                }
            } else {
                self.at += 1;
            }
        }
        self.reached_marker = true;
    }
}

/// What's been learned about the numbers so far, which is what arithmetic coding gets its savings from.
struct ArithmeticStates {
    dc: Box<[[u8; 64]; 4]>,
    ac: Box<[[u8; 256]; 4]>,
    even_odds: u8,
    /// For each component, what its last first number was like, and what it was.
    context: [usize; 4],
    predictor: [i32; 4],
}

impl ArithmeticStates {
    fn new() -> Self {
        Self {
            dc: Box::new([[0; 64]; 4]),
            ac: Box::new([[0; 256]; 4]),
            even_odds: EVEN_ODDS,
            context: [0; 4],
            predictor: [0; 4],
        }
    }
}

/// How large a magnitude there's room for.
const TOO_LARGE: i32 = 0x8000;

/// Reads the rest of a number, given where its size bits are in a set of states.
fn read_magnitude(
    reader: &mut ArithmeticReader<'_>,
    states: &mut [u8],
    mut magnitude: i32,
    at: usize,
) -> Option<i32> {
    let mut value = magnitude;
    let at = at + 14;
    loop {
        magnitude >>= 1;
        if magnitude == 0 {
            break;
        }
        if reader.read(states.get_mut(at)?) {
            value |= magnitude;
        }
    }
    Some(value + 1)
}

/// Reads how much a block's first number differs from the last, for this component.
fn read_arithmetic_dc(
    reader: &mut ArithmeticReader<'_>,
    states: &mut ArithmeticStates,
    conditioning: &ArithmeticConditioning,
    component: usize,
    table: usize,
) -> Option<i32> {
    let bins = &mut states.dc[table];
    let context = states.context[component];
    if !reader.read(&mut bins[context]) {
        states.context[component] = 0;
        return Some(states.predictor[component]);
    }
    let negative = reader.read(&mut bins[context + 1]);
    let mut at = context + 2 + usize::from(negative);
    let mut magnitude = i32::from(reader.read(&mut bins[at]));
    if magnitude != 0 {
        at = 20;
        while reader.read(bins.get_mut(at)?) {
            magnitude <<= 1;
            if magnitude == TOO_LARGE {
                return None;
            }
            at += 1;
        }
    }
    let lower = (1i32 << conditioning.dc_lower[table]) >> 1;
    let upper = (1i32 << conditioning.dc_upper[table]) >> 1;
    states.context[component] = if magnitude < lower {
        0
    } else if magnitude > upper {
        12 + usize::from(negative) * 4
    } else {
        4 + usize::from(negative) * 4
    };
    let value = read_magnitude(reader, &mut bins[..], magnitude, at)?;
    let difference = if negative { -value } else { value };
    states.predictor[component] = (states.predictor[component] + difference) & 0xFFFF;
    Some(states.predictor[component])
}

/// Reads one of a block's other numbers, whose first state is at `at`, as `position` says where it is.
fn read_arithmetic_ac(
    reader: &mut ArithmeticReader<'_>,
    bins: &mut [u8; 256],
    even_odds: &mut u8,
    conditioning_k: u8,
    position: usize,
    at: usize,
) -> Option<i32> {
    let negative = reader.read(even_odds);
    let mut at = at + 2;
    let mut magnitude = i32::from(reader.read(&mut bins[at]));
    if magnitude != 0 && reader.read(&mut bins[at]) {
        magnitude <<= 1;
        at = if position <= usize::from(conditioning_k) {
            189
        } else {
            217
        };
        while reader.read(bins.get_mut(at)?) {
            magnitude <<= 1;
            if magnitude == TOO_LARGE {
                return None;
            }
            at += 1;
        }
    }
    let value = read_magnitude(reader, &mut bins[..], magnitude, at)?;
    Some(if negative { -value } else { value })
}

/// What a file says about how numbers are grouped, for arithmetic coding.
#[derive(Clone, Copy)]
struct ArithmeticConditioning {
    dc_lower: [u8; 4],
    dc_upper: [u8; 4],
    ac_k: [u8; 4],
}

impl Default for ArithmeticConditioning {
    fn default() -> Self {
        Self {
            dc_lower: [0; 4],
            dc_upper: [1; 4],
            ac_k: [5; 4],
        }
    }
}

/// Turns the numbers in a block into 8 by 8 samples, for 12 bits. Those are too big for the usual
/// fixed-point way, and being rare, there's no need for it to be fast.
fn inverse_dct_12_bits(block: &[i32; 64], out: &mut [u8; 64]) {
    use std::f32::consts::{FRAC_1_SQRT_2, PI};
    let basis = |sample: usize, frequency: usize| {
        let scale = if frequency == 0 { FRAC_1_SQRT_2 } else { 1.0 };
        scale * ((2 * sample + 1) as f32 * frequency as f32 * PI / 16.0).cos() / 2.0
    };
    let mut work = [0f32; 64];
    for y in 0..8 {
        for v in 0..8 {
            let weight = basis(y, v);
            for x in 0..8 {
                work[y * 8 + x] += block[v * 8 + x] as f32 * weight;
            }
        }
    }
    for y in 0..8 {
        for x in 0..8 {
            let mut total = 0.0;
            for u in 0..8 {
                total += work[y * 8 + u] * basis(x, u);
            }
            out[y * 8 + x] = ((total + 2048.0) / 16.0 + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
}

/// Turns the numbers in a block into 8 by 8 samples. This is the usual fixed-point method. Each pass
/// works from the top of a 12-bit-scaled value, so the second's result has 17 bits to take off.
fn inverse_dct(block: &[i32; 64], out: &mut [u8; 64]) {
    const fn f(value: f32) -> i32 {
        (value * 4096.0 + 0.5) as i32
    }
    let mut work = [0i32; 64];
    // Columns.
    for x in 0..8 {
        let at = |y: usize| block[y * 8 + x];
        if (1..8).all(|y| at(y) == 0) {
            let dc = at(0) * 4;
            for y in 0..8 {
                work[y * 8 + x] = dc;
            }
            continue;
        }
        let parts = dct_line(
            [at(0), at(1), at(2), at(3), at(4), at(5), at(6), at(7)],
            512,
            10,
            [f(0.541_196_1), f(-1.847_759_1), f(0.765_366_9)],
        );
        for y in 0..8 {
            work[y * 8 + x] = parts[y];
        }
    }
    // Rows.
    for y in 0..8 {
        let row = &work[y * 8..y * 8 + 8];
        let parts = dct_line(
            [
                row[0], row[1], row[2], row[3], row[4], row[5], row[6], row[7],
            ],
            65536 + (128 << 17),
            17,
            [f(0.541_196_1), f(-1.847_759_1), f(0.765_366_9)],
        );
        for x in 0..8 {
            out[y * 8 + x] = parts[x].clamp(0, 255) as u8;
        }
    }
}

#[inline]
fn dct_line(values: [i32; 8], round: i32, shift: u32, even: [i32; 3]) -> [i32; 8] {
    const fn f(value: f32) -> i32 {
        (value * 4096.0 + 0.5) as i32
    }
    let [s0, s1, s2, s3, s4, s5, s6, s7] = values;
    let p1 = (s2 + s6).wrapping_mul(even[0]);
    let t2 = p1.wrapping_add(s6.wrapping_mul(even[1]));
    let t3 = p1.wrapping_add(s2.wrapping_mul(even[2]));
    let t0 = (s0 + s4) << 12;
    let t1 = (s0 - s4) << 12;
    let (x0, x3) = (t0.wrapping_add(t3), t0.wrapping_sub(t3));
    let (x1, x2) = (t1.wrapping_add(t2), t1.wrapping_sub(t2));
    let (t0, t1, t2, t3) = (s7, s5, s3, s1);
    let (p3, p4) = (t0 + t2, t1 + t3);
    let (p1, p2) = (t0 + t3, t1 + t2);
    let p5 = (p3 + p4).wrapping_mul(f(1.175_875_6));
    let t0 = t0.wrapping_mul(f(0.298_631_3));
    let t1 = t1.wrapping_mul(f(2.053_12));
    let t2 = t2.wrapping_mul(f(3.072_711));
    let t3 = t3.wrapping_mul(f(1.501_321_1));
    let p1 = p5.wrapping_add(p1.wrapping_mul(-f(0.899_976_2)));
    let p2 = p5.wrapping_add(p2.wrapping_mul(-f(2.562_915_4)));
    let p3 = p3.wrapping_mul(-f(1.961_570_6));
    let p4 = p4.wrapping_mul(-f(0.390_180_6));
    let t3 = t3.wrapping_add(p1).wrapping_add(p4);
    let t2 = t2.wrapping_add(p2).wrapping_add(p3);
    let t1 = t1.wrapping_add(p2).wrapping_add(p4);
    let t0 = t0.wrapping_add(p1).wrapping_add(p3);
    let finish = |value: i32| value.wrapping_add(round) >> shift;
    [
        finish(x0.wrapping_add(t3)),
        finish(x1.wrapping_add(t2)),
        finish(x2.wrapping_add(t1)),
        finish(x3.wrapping_add(t0)),
        finish(x3.wrapping_sub(t0)),
        finish(x2.wrapping_sub(t1)),
        finish(x1.wrapping_sub(t2)),
        finish(x0.wrapping_sub(t3)),
    ]
}

/// The samples of one component, at whichever fraction of its size is being kept.
struct Plane {
    width: usize,
    height: usize,
    samples: Vec<u8>,
}

/// What's read in a file before and between its scans.
struct Decoder<'a> {
    bytes: &'a [u8],
    frame: Frame,
    quantization: [[u16; 64]; 4],
    dc_codes: [Option<Box<Codes>>; 4],
    ac_codes: [Option<Box<Codes>>; 4],
    restart_interval: usize,
    adobe_transform: Option<u8>,
    orientation: u8,
    /// How many times smaller each side is being kept: 1, 2, 4 or 8.
    reduce: usize,
    planes: Vec<Plane>,
    /// For a progressive file, the numbers in every block until the last scan has been read.
    coefficients: Vec<Vec<i16>>,
    scans: usize,
    end_of_band_run: u32,
    /// The pieces of a color profile, with which number each one is.
    profile: Vec<(u8, &'a [u8])>,
    /// Takes print colors to what a screen shows, if the file has a profile for that.
    ink_transform: Option<ColorTransform>,
    conditioning: ArithmeticConditioning,
}

fn read_u16(bytes: &[u8], at: usize) -> Option<usize> {
    let part = bytes.get(at..at + 2)?;
    Some(usize::from(part[0]) << 8 | usize::from(part[1]))
}

fn read_exif_orientation(data: &[u8]) -> Option<u8> {
    let tiff = data.strip_prefix(b"Exif\0\0")?;
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let word = |at: usize| -> Option<u32> {
        let part = tiff.get(at..at + 2)?;
        Some(u32::from(if little {
            u16::from_le_bytes([part[0], part[1]])
        } else {
            u16::from_be_bytes([part[0], part[1]])
        }))
    };
    let long = |at: usize| -> Option<u32> {
        let part = tiff.get(at..at + 4)?;
        Some(if little {
            u32::from_le_bytes([part[0], part[1], part[2], part[3]])
        } else {
            u32::from_be_bytes([part[0], part[1], part[2], part[3]])
        })
    };
    let directory = long(4)? as usize;
    for index in 0..word(directory)? as usize {
        let entry = directory + 2 + index * 12;
        if word(entry)? == 0x0112 {
            return Some(word(entry + 8)? as u8);
        }
    }
    None
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            frame: Frame::default(),
            quantization: [[1; 64]; 4],
            dc_codes: Default::default(),
            ac_codes: Default::default(),
            restart_interval: 0,
            adobe_transform: None,
            orientation: 1,
            reduce: 1,
            planes: Vec::new(),
            coefficients: Vec::new(),
            scans: 0,
            end_of_band_run: 0,
            profile: Vec::new(),
            ink_transform: None,
            conditioning: ArithmeticConditioning::default(),
        }
    }

    /// Puts together the color profile, for what's going to be shown.
    fn color_transform(&self) -> Option<ColorTransform> {
        if self.profile.is_empty() {
            return None;
        }
        let mut pieces = self.profile.clone();
        pieces.sort_by_key(|(number, _)| *number);
        let whole: Vec<u8> = pieces
            .iter()
            .flat_map(|(_, piece)| piece.iter().copied())
            .collect();
        let kind = match self.frame.components.len() {
            1 => ProfileKind::Gray,
            3 => ProfileKind::Rgb,
            _ => ProfileKind::Cmyk,
        };
        ColorTransform::from_icc(&whole, kind)
    }

    /// Goes through the markers, stopping at the frame's description if `only_info`, and otherwise
    /// reading every scan in the file.
    fn read(&mut self, only_info: bool, target: (u32, u32)) -> Result<(), ImageError> {
        let mut at = 2;
        loop {
            // Anything between markers doesn't belong, and is passed over.
            while self.bytes.get(at).is_some_and(|byte| *byte != 0xFF) {
                at += 1;
            }
            while self.bytes.get(at + 1) == Some(&0xFF) {
                at += 1;
            }
            let Some(&marker) = self.bytes.get(at + 1) else {
                return if self.scans > 0 || only_info && self.frame.width > 0 {
                    Ok(())
                } else {
                    Err(ImageError::Invalid("it ends too soon"))
                };
            };
            at += 2;
            match marker {
                0x00 | 0xD0..=0xD8 | 0x01 => continue,
                0xD9 => return Ok(()),
                _ => {}
            }
            let length = read_u16(self.bytes, at).ok_or(ImageError::Invalid("it ends too soon"))?;
            if length < 2 {
                return Err(ImageError::Invalid("a section has an impossible length"));
            }
            let body_end = (at + length).min(self.bytes.len());
            let body = &self.bytes[(at + 2).min(body_end)..body_end];
            match marker {
                0xE1 => {
                    if let Some(orientation) = read_exif_orientation(body) {
                        self.orientation = orientation;
                    }
                }
                0xE2 => {
                    if let Some(rest) = body.strip_prefix(b"ICC_PROFILE\0") {
                        if let Some((number, piece)) = rest.split_first() {
                            self.profile.push((*number, piece.get(1..).unwrap_or(&[])));
                        }
                    }
                }
                0xEE => {
                    if body.starts_with(b"Adobe") {
                        self.adobe_transform = body.get(11).copied();
                    }
                }
                0xDB => self.read_quantization(body)?,
                0xC4 => self.read_codes(body)?,
                0xDD => {
                    self.restart_interval = read_u16(body, 0).ok_or(INVALID)?;
                }
                0xC0..=0xC3 | 0xC9 | 0xCA => {
                    self.read_frame(
                        body,
                        matches!(marker, 0xC2 | 0xCA),
                        marker >= 0xC9,
                        marker == 0xC3,
                    )?;
                    if only_info {
                        return Ok(());
                    }
                    self.prepare(target)?;
                }
                0xC5..=0xC7 | 0xCB | 0xCD..=0xCF => {
                    return Err(ImageError::Unsupported("this kind of JPEG"));
                }
                0xCC => self.read_conditioning(body)?,
                0xDA => {
                    if self.frame.width == 0 {
                        return Err(ImageError::Invalid("its pixels come before its size"));
                    }
                    self.scans += 1;
                    if self.scans > MAX_SCANS {
                        return Ok(());
                    }
                    at = self.read_scan(body, at + length)?;
                    continue;
                }
                _ => {}
            }
            at += length;
        }
    }

    fn read_quantization(&mut self, mut body: &[u8]) -> Result<(), ImageError> {
        while let Some((&info, rest)) = body.split_first() {
            let wide = info >> 4 != 0;
            let table = usize::from(info & 15);
            let size = if wide { 128 } else { 64 };
            if table >= 4 || rest.len() < size {
                return Err(INVALID);
            }
            for index in 0..64 {
                let value = if wide {
                    u16::from_be_bytes([rest[index * 2], rest[index * 2 + 1]])
                } else {
                    u16::from(rest[index])
                };
                self.quantization[table][usize::from(ZIGZAG[index])] = value;
            }
            body = &rest[size..];
        }
        Ok(())
    }

    fn read_codes(&mut self, mut body: &[u8]) -> Result<(), ImageError> {
        while body.len() >= 17 {
            let (info, counts) = (body[0], &body[1..17]);
            let total: usize = counts.iter().map(|count| usize::from(*count)).sum();
            let values = body.get(17..17 + total).ok_or(INVALID)?;
            if total > 256 {
                return Err(INVALID);
            }
            let table = usize::from(info & 15);
            if table >= 4 {
                return Err(INVALID);
            }
            let codes = Box::new(Codes::new(counts, values)?);
            if info >> 4 == 0 {
                self.dc_codes[table] = Some(codes);
            } else {
                self.ac_codes[table] = Some(codes);
            }
            body = &body[17 + total..];
        }
        Ok(())
    }

    fn read_conditioning(&mut self, body: &[u8]) -> Result<(), ImageError> {
        for pair in body.chunks_exact(2) {
            let table = usize::from(pair[0] & 15);
            if table >= 4 {
                return Err(INVALID);
            }
            if pair[0] >> 4 == 0 {
                let (lower, upper) = (pair[1] & 15, pair[1] >> 4);
                if lower > upper {
                    return Err(INVALID);
                }
                self.conditioning.dc_lower[table] = lower;
                self.conditioning.dc_upper[table] = upper;
            } else {
                if !(1..=63).contains(&pair[1]) {
                    return Err(INVALID);
                }
                self.conditioning.ac_k[table] = pair[1];
            }
        }
        Ok(())
    }

    fn read_frame(
        &mut self,
        body: &[u8],
        progressive: bool,
        arithmetic: bool,
        lossless: bool,
    ) -> Result<(), ImageError> {
        if self.frame.width != 0 {
            return Err(ImageError::Invalid("it has two sizes"));
        }
        let precision = u32::from(*body.first().ok_or(INVALID)?);
        let known = if lossless {
            (2..=16).contains(&precision)
        } else {
            matches!(precision, 8 | 12)
        };
        if !known {
            return Err(ImageError::Unsupported("this many bits per color"));
        }
        let twelve_bits = !lossless && precision == 12;
        let height = read_u16(body, 1).ok_or(INVALID)? as u32;
        let width = read_u16(body, 3).ok_or(INVALID)? as u32;
        let count = usize::from(*body.get(5).ok_or(INVALID)?);
        if width == 0 || height == 0 {
            return Err(ImageError::Invalid("it has no size"));
        }
        if !matches!(count, 1 | 3 | 4) {
            return Err(ImageError::Unsupported("this many color components"));
        }
        let limit = if progressive {
            MAX_PROGRESSIVE_PIXELS
        } else {
            MAX_PIXELS
        };
        if u64::from(width) * u64::from(height) > limit {
            return Err(ImageError::TooBig);
        }
        let mut components = Vec::with_capacity(count);
        for index in 0..count {
            let part = body.get(6 + index * 3..9 + index * 3).ok_or(INVALID)?;
            let (h, v) = (usize::from(part[1] >> 4), usize::from(part[1] & 15));
            if !(1..=4).contains(&h) || !(1..=4).contains(&v) || part[2] >= 4 {
                return Err(INVALID);
            }
            components.push(Component {
                id: part[0],
                h,
                v,
                table: usize::from(part[2]),
                ..Component::default()
            });
        }
        // A file with one component is never really subsampled, whatever it says.
        if count == 1 {
            components[0].h = 1;
            components[0].v = 1;
        }
        self.frame = Frame {
            progressive,
            arithmetic,
            twelve_bits,
            lossless,
            precision,
            width,
            height,
            h_max: components.iter().map(|c| c.h).max().unwrap_or(1),
            v_max: components.iter().map(|c| c.v).max().unwrap_or(1),
            components,
        };
        Ok(())
    }

    /// Makes room for the picture, at the smallest fraction of its size that's enough.
    fn prepare(&mut self, target: (u32, u32)) -> Result<(), ImageError> {
        let (width, height) = (self.frame.width, self.frame.height);
        let swaps = self.orientation >= 5 && self.orientation <= 8;
        let wanted = fitted(
            if swaps {
                (height, width)
            } else {
                (width, height)
            },
            target,
        );
        let wanted = if swaps { (wanted.1, wanted.0) } else { wanted };
        self.reduce = 1;
        for reduce in [2u32, 4, 8] {
            if width.div_ceil(reduce) >= wanted.0 && height.div_ceil(reduce) >= wanted.1 {
                self.reduce = reduce as usize;
            }
        }
        if self.frame.lossless {
            return self.prepare_lossless();
        }
        let side = 8 / self.reduce;
        let components: Vec<Component> = self.frame.components.clone();
        for component in &components {
            let (across, down) = self.frame.blocks(component);
            let (plane_width, plane_height) = (across * side, down * side);
            let mut samples = Vec::new();
            samples
                .try_reserve_exact(plane_width * plane_height)
                .map_err(|_| ImageError::TooBig)?;
            samples.resize(plane_width * plane_height, 128u8);
            self.planes.push(Plane {
                width: plane_width,
                height: plane_height,
                samples,
            });
            if self.frame.progressive {
                let mut numbers = Vec::new();
                numbers
                    .try_reserve_exact(across * down * 64)
                    .map_err(|_| ImageError::TooBig)?;
                numbers.resize(across * down * 64, 0i16);
                self.coefficients.push(numbers);
            }
        }
        Ok(())
    }

    /// Makes room for a lossless picture, which is always read whole, with a sample for each number.
    fn prepare_lossless(&mut self) -> Result<(), ImageError> {
        self.reduce = 1;
        let (width, height) = (self.frame.width as usize, self.frame.height as usize);
        let (across, down) = (
            width.div_ceil(self.frame.h_max),
            height.div_ceil(self.frame.v_max),
        );
        for component in self.frame.components.clone() {
            let (plane_width, plane_height) = (across * component.h, down * component.v);
            let mut samples = Vec::new();
            samples
                .try_reserve_exact(plane_width * plane_height)
                .map_err(|_| ImageError::TooBig)?;
            samples.resize(plane_width * plane_height, 128u8);
            self.planes.push(Plane {
                width: plane_width,
                height: plane_height,
                samples,
            });
        }
        Ok(())
    }

    /// Reads a scan of a lossless picture. Each number is a guess from those next to it, chosen by
    /// `predictor`, and a difference from that guess.
    fn read_lossless_scan(
        &mut self,
        scan: &[usize],
        predictor: usize,
        point: u32,
        bits: &mut Bits<'_>,
    ) -> Result<(), ImageError> {
        if !(1..=7).contains(&predictor) || point >= self.frame.precision {
            return Err(INVALID);
        }
        let first_guess = 1i32 << (self.frame.precision - point - 1);
        let (h_max, v_max) = (self.frame.h_max, self.frame.v_max);
        let (width, height) = (self.frame.width as usize, self.frame.height as usize);
        let single = scan.len() == 1;
        let mut numbers: Vec<Vec<u16>> = Vec::with_capacity(scan.len());
        for &index in scan {
            let plane = &self.planes[index];
            let mut values = Vec::new();
            values
                .try_reserve_exact(plane.width * plane.height)
                .map_err(|_| ImageError::TooBig)?;
            values.resize(plane.width * plane.height, 0u16);
            numbers.push(values);
        }
        let (units_across, units_down) = if single {
            let component = self.frame.components[scan[0]];
            (
                (width * component.h).div_ceil(h_max),
                (height * component.v).div_ceil(v_max),
            )
        } else {
            (width.div_ceil(h_max), height.div_ceil(v_max))
        };
        // For each component: the row a restart last happened on, and whether its first number is next.
        let mut guess_first = vec![true; scan.len()];
        let mut first_row = vec![0usize; scan.len()];
        let mut until_restart = self.restart_interval;
        'units: for unit_y in 0..units_down {
            for unit_x in 0..units_across {
                if self.restart_interval > 0 && until_restart == 0 {
                    bits.restart();
                    bits.made_up = 0;
                    guess_first.fill(true);
                    until_restart = self.restart_interval;
                }
                until_restart = until_restart.saturating_sub(1);
                for (slot, &index) in scan.iter().enumerate() {
                    let component = self.frame.components[index];
                    let (parts_across, parts_down) = if single {
                        (1, 1)
                    } else {
                        (component.h, component.v)
                    };
                    let codes = self.dc_codes[component.dc_table]
                        .as_deref()
                        .ok_or(INVALID)?;
                    let stride = self.planes[index].width;
                    for part_y in 0..parts_down {
                        for part_x in 0..parts_across {
                            let x = unit_x * parts_across + part_x;
                            let y = unit_y * parts_down + part_y;
                            let at = y * stride + x;
                            let values = &mut numbers[slot];
                            if guess_first[slot] {
                                first_row[slot] = y;
                            }
                            let guess = if guess_first[slot] {
                                guess_first[slot] = false;
                                first_guess
                            } else if y == first_row[slot] {
                                i32::from(values[at - 1])
                            } else if x == 0 {
                                i32::from(values[at - stride])
                            } else {
                                let left = i32::from(values[at - 1]);
                                let above = i32::from(values[at - stride]);
                                let both = i32::from(values[at - stride - 1]);
                                match predictor {
                                    1 => left,
                                    2 => above,
                                    3 => both,
                                    4 => left + above - both,
                                    5 => left + ((above - both) >> 1),
                                    6 => above + ((left - both) >> 1),
                                    _ => (left + above) >> 1,
                                }
                            };
                            let difference = match bits.symbol(codes)? {
                                0 => 0,
                                16 => 32768,
                                size @ 1..=15 => bits.signed(u32::from(size)),
                                _ => return Err(INVALID),
                            };
                            values[at] = (guess + difference) as u16;
                        }
                    }
                }
                if bits.ran_out() {
                    break 'units;
                }
            }
        }
        // Everything else works in 8 bits per color.
        let top = (1u32 << self.frame.precision) - 1;
        for (slot, &index) in scan.iter().enumerate() {
            let (keep, plane) = (&numbers[slot], &mut self.planes[index]);
            for (sample, value) in plane.samples.iter_mut().zip(keep) {
                let whole = u32::from(*value) << point;
                *sample = if top == 255 {
                    whole as u8
                } else {
                    ((whole.min(top) * 255 + top / 2) / top) as u8
                };
            }
        }
        Ok(())
    }

    /// Reads a scan, and returns where to carry on reading from.
    fn read_scan(&mut self, header: &[u8], data_start: usize) -> Result<usize, ImageError> {
        let count = usize::from(*header.first().ok_or(INVALID)?);
        if count == 0 || count > 4 || header.len() < 1 + count * 2 + 3 {
            return Err(INVALID);
        }
        let mut scan = Vec::with_capacity(count);
        for index in 0..count {
            let id = header[1 + index * 2];
            let tables = header[2 + index * 2];
            let found = self
                .frame
                .components
                .iter()
                .position(|component| component.id == id)
                .ok_or(INVALID)?;
            let component = &mut self.frame.components[found];
            component.dc_table = usize::from(tables >> 4).min(3);
            component.ac_table = usize::from(tables & 15).min(3);
            component.dc_predictor = 0;
            scan.push(found);
        }
        let spectral_start = usize::from(header[1 + count * 2]);
        let spectral_end = usize::from(header[2 + count * 2]);
        let successive = header[3 + count * 2];
        let (high, low) = (u32::from(successive >> 4), u32::from(successive & 15));
        if self.frame.progressive
            && (spectral_start > 63 || spectral_end > 63 || spectral_start > spectral_end)
        {
            return Err(INVALID);
        }
        self.end_of_band_run = 0;
        if self.frame.lossless {
            let mut bits = Bits {
                data: self.bytes.get(data_start..).unwrap_or(&[]),
                at: 0,
                held: 0,
                count: 0,
                made_up: 0,
            };
            // A scan cut short or damaged leaves what got through, which is better than nothing.
            let _ = self.read_lossless_scan(&scan, spectral_start, low, &mut bits);
            return Ok(data_start + bits.at);
        }
        if self.frame.arithmetic {
            let parameters = ScanParameters {
                spectral_start,
                spectral_end,
                high,
                low,
            };
            let data = self.bytes.get(data_start..).unwrap_or(&[]);
            return Ok(data_start + self.read_arithmetic_scan(&scan, &parameters, data));
        }
        let mut bits = Bits {
            data: self.bytes.get(data_start..).unwrap_or(&[]),
            at: 0,
            held: 0,
            count: 0,
            made_up: 0,
        };
        let parameters = ScanParameters {
            spectral_start,
            spectral_end,
            high,
            low,
        };
        // A scan cut short or damaged leaves what got through, which is better than nothing.
        let _ = self.read_units(&scan, &parameters, &mut bits);
        Ok(data_start + bits.at)
    }

    /// Reads a scan of arithmetic-coded data, and says how far through the data it got.
    fn read_arithmetic_scan(
        &mut self,
        scan: &[usize],
        parameters: &ScanParameters,
        data: &[u8],
    ) -> usize {
        let mut reader = ArithmeticReader::new(data);
        let mut states = ArithmeticStates::new();
        let (mcu_across, mcu_down) = if let [only] = scan {
            let component = self.frame.components[*only];
            self.frame.own_blocks(&component)
        } else {
            self.frame.mcus()
        };
        let mut until_restart = self.restart_interval;
        let mut block = [0i32; 64];
        let mut finished = true;
        'units: for mcu_y in 0..mcu_down {
            for mcu_x in 0..mcu_across {
                if self.restart_interval > 0 && until_restart == 0 {
                    reader.restart();
                    states = ArithmeticStates::new();
                    until_restart = self.restart_interval;
                }
                until_restart = until_restart.saturating_sub(1);
                let done = if let [only] = scan {
                    self.read_arithmetic_block(
                        *only,
                        (mcu_x, mcu_y),
                        parameters,
                        &mut reader,
                        &mut states,
                        &mut block,
                    )
                } else {
                    let mut done = Some(());
                    'blocks: for &index in scan {
                        let component = self.frame.components[index];
                        for v in 0..component.v {
                            for h in 0..component.h {
                                let place = (mcu_x * component.h + h, mcu_y * component.v + v);
                                done = self.read_arithmetic_block(
                                    index,
                                    place,
                                    parameters,
                                    &mut reader,
                                    &mut states,
                                    &mut block,
                                );
                                if done.is_none() {
                                    break 'blocks;
                                }
                            }
                        }
                    }
                    done
                };
                // What got through is better than nothing.
                if done.is_none() {
                    finished = false;
                    break 'units;
                }
            }
        }
        let _ = finished;
        reader.at
    }

    fn read_arithmetic_block(
        &mut self,
        index: usize,
        place: (usize, usize),
        parameters: &ScanParameters,
        reader: &mut ArithmeticReader<'_>,
        states: &mut ArithmeticStates,
        block: &mut [i32; 64],
    ) -> Option<()> {
        let component = self.frame.components[index];
        let conditioning = self.conditioning;
        let k = conditioning.ac_k[component.ac_table];
        if self.frame.progressive {
            let (across, down) = self.frame.blocks(&component);
            if place.0 >= across || place.1 >= down {
                return Some(());
            }
            let start = (place.1 * across + place.0) * 64;
            let numbers = &mut self.coefficients[index][start..start + 64];
            let &ScanParameters {
                spectral_start,
                spectral_end,
                high,
                low,
            } = parameters;
            if spectral_start == 0 {
                if high == 0 {
                    let value = read_arithmetic_dc(
                        reader,
                        states,
                        &conditioning,
                        index,
                        component.dc_table,
                    )?;
                    numbers[0] = (value << low) as i16;
                } else if reader.read(&mut states.even_odds) {
                    numbers[0] |= 1 << low;
                }
                return Some(());
            }
            let bins = &mut states.ac[component.ac_table];
            if high == 0 {
                let mut at = spectral_start;
                while at <= spectral_end {
                    let mut state = 3 * (at - 1);
                    if reader.read(&mut bins[state]) {
                        break;
                    }
                    while !reader.read(&mut bins[state + 1]) {
                        state += 3;
                        at += 1;
                        if at > spectral_end {
                            return None;
                        }
                    }
                    let value =
                        read_arithmetic_ac(reader, bins, &mut states.even_odds, k, at, state)?;
                    numbers[usize::from(ZIGZAG[at])] = (value << low) as i16;
                    at += 1;
                }
                return Some(());
            }
            // Refining: one more bit for what's already been seen, and any new ones.
            let bit = 1i16 << low;
            let negative_bit = (-1i16) << low;
            let last_seen = (1..=spectral_end)
                .rev()
                .find(|at| numbers[usize::from(ZIGZAG[*at])] != 0)
                .unwrap_or(0);
            let mut at = spectral_start;
            while at <= spectral_end {
                let mut state = 3 * (at - 1);
                if at > last_seen && reader.read(&mut bins[state]) {
                    break;
                }
                loop {
                    let value = &mut numbers[usize::from(ZIGZAG[at])];
                    if *value != 0 {
                        if reader.read(&mut bins[state + 2]) {
                            *value = if *value < 0 {
                                value.wrapping_add(negative_bit)
                            } else {
                                value.wrapping_add(bit)
                            };
                        }
                        break;
                    }
                    if reader.read(&mut bins[state + 1]) {
                        *value = if reader.read(&mut states.even_odds) {
                            negative_bit
                        } else {
                            bit
                        };
                        break;
                    }
                    state += 3;
                    at += 1;
                    if at > spectral_end {
                        return None;
                    }
                }
                at += 1;
            }
            return Some(());
        }
        block.fill(0);
        let quantization = self.quantization[component.table];
        let first = read_arithmetic_dc(reader, states, &conditioning, index, component.dc_table)?;
        // It's kept in 16 bits, which is what a negative number looks like there.
        block[0] = i32::from(first as u16 as i16).wrapping_mul(i32::from(quantization[0]));
        let keep_only_dc = self.reduce == 8;
        let bins = &mut states.ac[component.ac_table];
        let mut at = 1;
        while at <= 63 {
            let mut state = 3 * (at - 1);
            if reader.read(&mut bins[state]) {
                break;
            }
            while !reader.read(&mut bins[state + 1]) {
                state += 3;
                at += 1;
                if at > 63 {
                    return None;
                }
            }
            let value = read_arithmetic_ac(reader, bins, &mut states.even_odds, k, at, state)?;
            if !keep_only_dc {
                let position = usize::from(ZIGZAG[at]);
                block[position] = value.wrapping_mul(i32::from(quantization[position]));
            }
            at += 1;
        }
        self.store_block(index, place.0, place.1, block);
        Some(())
    }

    fn read_units(
        &mut self,
        scan: &[usize],
        parameters: &ScanParameters,
        bits: &mut Bits<'_>,
    ) -> Result<(), ImageError> {
        let (mcu_across, mcu_down) = if let [only] = scan {
            let component = self.frame.components[*only];
            self.frame.own_blocks(&component)
        } else {
            self.frame.mcus()
        };
        let mut until_restart = self.restart_interval;
        let mut block = [0i32; 64];
        for mcu_y in 0..mcu_down {
            for mcu_x in 0..mcu_across {
                if self.restart_interval > 0 && until_restart == 0 {
                    bits.restart();
                    bits.made_up = 0;
                    self.end_of_band_run = 0;
                    for component in &mut self.frame.components {
                        component.dc_predictor = 0;
                    }
                    until_restart = self.restart_interval;
                }
                until_restart = until_restart.saturating_sub(1);
                if let [only] = scan {
                    self.read_block(*only, mcu_x, mcu_y, parameters, bits, &mut block)?;
                } else {
                    for &index in scan {
                        let component = self.frame.components[index];
                        for v in 0..component.v {
                            for h in 0..component.h {
                                self.read_block(
                                    index,
                                    mcu_x * component.h + h,
                                    mcu_y * component.v + v,
                                    parameters,
                                    bits,
                                    &mut block,
                                )?;
                            }
                        }
                    }
                }
                if bits.ran_out() {
                    return Err(ImageError::Invalid("it ends too soon"));
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn read_block(
        &mut self,
        index: usize,
        x: usize,
        y: usize,
        parameters: &ScanParameters,
        bits: &mut Bits<'_>,
        block: &mut [i32; 64],
    ) -> Result<(), ImageError> {
        if self.frame.progressive {
            return self.read_progressive_block(index, x, y, parameters, bits);
        }
        let component = self.frame.components[index];
        let dc = self.dc_codes[component.dc_table]
            .as_deref()
            .ok_or(INVALID)?;
        let ac = self.ac_codes[component.ac_table]
            .as_deref()
            .ok_or(INVALID)?;
        block.fill(0);
        let size = u32::from(bits.symbol(dc)?);
        if size > 16 {
            return Err(INVALID);
        }
        let difference = bits.signed(size);
        let predictor = component.dc_predictor.wrapping_add(difference);
        self.frame.components[index].dc_predictor = predictor;
        let quantization = &self.quantization[component.table];
        block[0] = predictor.wrapping_mul(i32::from(quantization[0]));
        let mut at = 1;
        let keep_only_dc = self.reduce == 8;
        while at < 64 {
            let symbol = bits.symbol(ac)?;
            let (zeros, size) = (usize::from(symbol >> 4), u32::from(symbol & 15));
            if size == 0 {
                if zeros != 15 {
                    break;
                }
                at += 16;
                continue;
            }
            at += zeros;
            if at >= 64 {
                return Err(INVALID);
            }
            let value = bits.signed(size);
            if !keep_only_dc {
                let position = usize::from(ZIGZAG[at]);
                block[position] = value.wrapping_mul(i32::from(quantization[position]));
            }
            at += 1;
        }
        self.store_block(index, x, y, block);
        Ok(())
    }

    fn read_progressive_block(
        &mut self,
        index: usize,
        x: usize,
        y: usize,
        parameters: &ScanParameters,
        bits: &mut Bits<'_>,
    ) -> Result<(), ImageError> {
        let component = self.frame.components[index];
        let (across, down) = self.frame.blocks(&component);
        if x >= across || y >= down {
            return Ok(());
        }
        let start = (y * across + x) * 64;
        let &ScanParameters {
            spectral_start,
            spectral_end,
            high,
            low,
        } = parameters;
        let numbers = &mut self.coefficients[index][start..start + 64];
        if spectral_start == 0 {
            if high == 0 {
                let dc = self.dc_codes[component.dc_table]
                    .as_deref()
                    .ok_or(INVALID)?;
                let size = u32::from(bits.symbol(dc)?);
                if size > 16 {
                    return Err(INVALID);
                }
                let predictor = component.dc_predictor.wrapping_add(bits.signed(size));
                self.frame.components[index].dc_predictor = predictor;
                numbers[0] = predictor.wrapping_mul(1 << low) as i16;
            } else if bits.take(1) == 1 {
                numbers[0] |= 1 << low;
            }
            return Ok(());
        }
        let ac = self.ac_codes[component.ac_table]
            .as_deref()
            .ok_or(INVALID)?;
        if high == 0 {
            if self.end_of_band_run > 0 {
                self.end_of_band_run -= 1;
                return Ok(());
            }
            let mut at = spectral_start;
            while at <= spectral_end {
                let symbol = bits.symbol(ac)?;
                let (zeros, size) = (u32::from(symbol >> 4), u32::from(symbol & 15));
                if size == 0 {
                    if zeros < 15 {
                        self.end_of_band_run = (1 << zeros) - 1 + bits.take(zeros);
                        break;
                    }
                    at += 16;
                    continue;
                }
                at += zeros as usize;
                if at > 63 {
                    return Err(INVALID);
                }
                numbers[usize::from(ZIGZAG[at])] = (bits.signed(size) << low) as i16;
                at += 1;
            }
            return Ok(());
        }
        // Refining: one more bit for what's already been seen, and any new ones of size 1.
        let bit = 1i16 << low;
        let refine = |value: &mut i16, bits: &mut Bits<'_>| {
            if bits.take(1) == 1 && *value & bit == 0 {
                *value = if *value >= 0 {
                    value.wrapping_add(bit)
                } else {
                    value.wrapping_sub(bit)
                };
            }
        };
        let mut at = spectral_start;
        if self.end_of_band_run == 0 {
            while at <= spectral_end {
                let symbol = bits.symbol(ac)?;
                let (mut zeros, size) = (u32::from(symbol >> 4), u32::from(symbol & 15));
                let mut new_value = 0i16;
                if size == 0 {
                    if zeros < 15 {
                        self.end_of_band_run = (1 << zeros) + bits.take(zeros);
                        break;
                    }
                } else {
                    new_value = if bits.take(1) == 1 {
                        bit
                    } else {
                        bit.wrapping_neg()
                    };
                }
                while at <= spectral_end {
                    let value = &mut numbers[usize::from(ZIGZAG[at])];
                    at += 1;
                    if *value != 0 {
                        refine(value, bits);
                    } else {
                        if zeros == 0 {
                            if new_value != 0 {
                                *value = new_value;
                            }
                            break;
                        }
                        zeros -= 1;
                    }
                }
            }
        }
        if self.end_of_band_run > 0 {
            while at <= spectral_end {
                let value = &mut numbers[usize::from(ZIGZAG[at])];
                if *value != 0 {
                    refine(value, bits);
                }
                at += 1;
            }
            self.end_of_band_run -= 1;
        }
        Ok(())
    }

    /// Writes a block's samples into its plane, at the size being kept.
    fn store_block(&mut self, index: usize, x: usize, y: usize, block: &[i32; 64]) {
        let plane = &mut self.planes[index];
        let side = 8 / self.reduce;
        let (left, top) = (x * side, y * side);
        if left + side > plane.width || top + side > plane.height {
            return;
        }
        if self.reduce == 8 {
            let value = if self.frame.twelve_bits {
                (((block[0] + 4) >> 3) + 2048 + 8) >> 4
            } else {
                ((block[0] + 4) >> 3) + 128
            };
            plane.samples[top * plane.width + left] = value.clamp(0, 255) as u8;
            return;
        }
        let mut samples = [0u8; 64];
        if self.frame.twelve_bits {
            inverse_dct_12_bits(block, &mut samples);
        } else {
            inverse_dct(block, &mut samples);
        }
        if self.reduce == 1 {
            for row in 0..8 {
                let at = (top + row) * plane.width + left;
                plane.samples[at..at + 8].copy_from_slice(&samples[row * 8..row * 8 + 8]);
            }
            return;
        }
        let reduce = self.reduce;
        for row in 0..side {
            for column in 0..side {
                let mut total = 0u32;
                for down in 0..reduce {
                    for across in 0..reduce {
                        total += u32::from(
                            samples[(row * reduce + down) * 8 + column * reduce + across],
                        );
                    }
                }
                let area = (reduce * reduce) as u32;
                plane.samples[(top + row) * plane.width + left + column] =
                    ((total + area / 2) / area) as u8;
            }
        }
    }

    /// For a progressive file, turns the numbers into samples, now that they're all there.
    fn finish_progressive(&mut self) {
        let mut block = [0i32; 64];
        for index in 0..self.frame.components.len() {
            let component = self.frame.components[index];
            let (across, down) = self.frame.blocks(&component);
            let numbers = std::mem::take(&mut self.coefficients[index]);
            let quantization = self.quantization[component.table];
            for y in 0..down {
                for x in 0..across {
                    let start = (y * across + x) * 64;
                    for (at, value) in numbers[start..start + 64].iter().enumerate() {
                        block[at] = i32::from(*value).wrapping_mul(i32::from(quantization[at]));
                    }
                    self.store_block(index, x, y, &block);
                }
            }
        }
    }

    /// Whether what's in the file is red, green and blue directly, rather than brightness and color.
    fn is_rgb(&self) -> bool {
        match self.adobe_transform {
            Some(transform) => transform == 0,
            None => {
                let ids: Vec<u8> = self.frame.components.iter().map(|c| c.id).collect();
                ids == *b"RGB"
            }
        }
    }

    /// Puts the planes together, row by row, into `scaler`.
    #[allow(clippy::needless_range_loop)]
    fn write_rows(&self, scaler: &mut Scaler) {
        let (width, height) = (
            (self.frame.width as usize).div_ceil(self.reduce),
            (self.frame.height as usize).div_ceil(self.reduce),
        );
        let count = self.frame.components.len();
        let mut rows = vec![vec![0u8; width]; count];
        let mut rgba = vec![255u8; width * 4];
        // Where each output position falls in each plane, in 256ths of a sample, once for all rows.
        let across: Vec<Vec<(u32, u32)>> = self
            .frame
            .components
            .iter()
            .map(|component| sample_positions(width, component.h, self.frame.h_max))
            .collect();
        let down: Vec<Vec<(u32, u32)>> = self
            .frame
            .components
            .iter()
            .map(|component| sample_positions(height, component.v, self.frame.v_max))
            .collect();
        let rgb = self.is_rgb();
        for y in 0..height {
            for (index, row) in rows.iter_mut().enumerate() {
                let plane = &self.planes[index];
                let (first, fraction) = down[index][y];
                let first = (first as usize).min(plane.height.saturating_sub(1));
                let second = (first + 1).min(plane.height.saturating_sub(1));
                let top = &plane.samples[first * plane.width..(first + 1) * plane.width];
                let bottom = &plane.samples[second * plane.width..(second + 1) * plane.width];
                for (x, &(left, part)) in across[index].iter().enumerate() {
                    let left = (left as usize).min(plane.width.saturating_sub(1));
                    if part == 0 && fraction == 0 {
                        row[x] = top[left];
                    } else {
                        let right = (left + 1).min(plane.width.saturating_sub(1));
                        let upper =
                            u32::from(top[left]) * (256 - part) + u32::from(top[right]) * part;
                        let lower = u32::from(bottom[left]) * (256 - part)
                            + u32::from(bottom[right]) * part;
                        let both = upper * (256 - fraction) + lower * fraction;
                        row[x] = ((both + (1 << 15)) >> 16) as u8;
                    }
                }
            }
            match count {
                1 => {
                    for (pixel, &value) in rgba.chunks_exact_mut(4).zip(&rows[0]) {
                        pixel[..3].fill(value);
                    }
                }
                3 if rgb => {
                    for (x, pixel) in rgba.chunks_exact_mut(4).enumerate() {
                        pixel[0] = rows[0][x];
                        pixel[1] = rows[1][x];
                        pixel[2] = rows[2][x];
                    }
                }
                3 => {
                    for (x, pixel) in rgba.chunks_exact_mut(4).enumerate() {
                        let (r, g, b) = to_rgb(rows[0][x], rows[1][x], rows[2][x]);
                        pixel[0] = r;
                        pixel[1] = g;
                        pixel[2] = b;
                    }
                }
                _ => {
                    // Print colors, which are written inverted, so multiplying them by how much
                    // black there isn't gives red, green and blue. One kind stores the first three
                    // as brightness and color.
                    let transformed = self.adobe_transform == Some(2);
                    let ink_transform = self.ink_transform.as_ref();
                    for (x, pixel) in rgba.chunks_exact_mut(4).enumerate() {
                        let (c, m, yellow) = if transformed {
                            let (r, g, b) = to_rgb(rows[0][x], rows[1][x], rows[2][x]);
                            (255 - r, 255 - g, 255 - b)
                        } else {
                            (rows[0][x], rows[1][x], rows[2][x])
                        };
                        if let Some(transform) = ink_transform {
                            let ink = [255 - c, 255 - m, 255 - yellow, 255 - rows[3][x]];
                            pixel[..3].copy_from_slice(&transform.ink_to_screen(ink));
                            continue;
                        }
                        let k = u32::from(rows[3][x]);
                        let part = |value: u8| ((u32::from(value) * k + 127) / 255) as u8;
                        pixel[0] = part(c);
                        pixel[1] = part(m);
                        pixel[2] = part(yellow);
                    }
                }
            }
            scaler.push(&rgba);
        }
    }
}

struct ScanParameters {
    spectral_start: usize,
    spectral_end: usize,
    high: u32,
    low: u32,
}

/// Where each position of an output line falls among a plane's samples. A plane with fewer samples
/// than the image has its samples spread out evenly, with each one's center where it belongs.
fn sample_positions(length: usize, part: usize, most: usize) -> Vec<(u32, u32)> {
    (0..length)
        .map(|at| {
            if part == most {
                return (at as u32, 0);
            }
            // In 256ths of a sample, to the center of this position, then back half a sample.
            let center = (at as u64 * 2 + 1) * 256 * part as u64 / (most as u64 * 2);
            let position = center.saturating_sub(128);
            ((position / 256) as u32, (position % 256) as u32)
        })
        .collect()
}

#[inline]
fn to_rgb(y: u8, cb: u8, cr: u8) -> (u8, u8, u8) {
    let y = i32::from(y) << 16;
    let (cb, cr) = (i32::from(cb) - 128, i32::from(cr) - 128);
    let r = (y + 91881 * cr + (1 << 15)) >> 16;
    let g = (y - 22554 * cb - 46802 * cr + (1 << 15)) >> 16;
    let b = (y + 116130 * cb + (1 << 15)) >> 16;
    (
        r.clamp(0, 255) as u8,
        g.clamp(0, 255) as u8,
        b.clamp(0, 255) as u8,
    )
}

pub(crate) fn info(bytes: &[u8]) -> Result<ImageInfo, ImageError> {
    let mut decoder = Decoder::new(bytes);
    decoder.read(true, (1, 1))?;
    if decoder.frame.width == 0 {
        return Err(ImageError::Invalid("it doesn't say how big it is"));
    }
    let (width, height) = (decoder.frame.width, decoder.frame.height);
    Ok(if (5..=8).contains(&decoder.orientation) {
        ImageInfo {
            width: height,
            height: width,
        }
    } else {
        ImageInfo { width, height }
    })
}

pub(crate) fn decode(bytes: &[u8], target: (u32, u32)) -> Result<Picture, ImageError> {
    let mut decoder = Decoder::new(bytes);
    decoder.read(false, target)?;
    if decoder.frame.width == 0 || decoder.scans == 0 {
        return Err(ImageError::Invalid("it has no pixels"));
    }
    if decoder.frame.progressive {
        decoder.finish_progressive();
    }
    let source = (
        decoder.frame.width.div_ceil(decoder.reduce as u32),
        decoder.frame.height.div_ceil(decoder.reduce as u32),
    );
    let swaps = (5..=8).contains(&decoder.orientation);
    let fit = fitted(if swaps { (source.1, source.0) } else { source }, target);
    let fit = if swaps { (fit.1, fit.0) } else { fit };
    let mut scaler = Scaler::new(source, fit);
    let transform = decoder.color_transform();
    if decoder.frame.components.len() == 4 {
        decoder.ink_transform = transform;
    } else {
        scaler.set_transform(transform);
    }
    decoder.write_rows(&mut scaler);
    decoder.planes = Vec::new();
    let size = scaler.size();
    let mut pixels = scaler.finish();
    let (width, height) = orient(&mut pixels, size, decoder.orientation);
    Ok(Picture {
        width,
        height,
        pixels,
    })
}
