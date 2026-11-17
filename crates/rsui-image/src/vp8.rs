//! Reads the lossy kind of WebP, which is one VP8 key frame. RFC 6386 says how.
//!
//! Predicting each block only ever uses what's been read before any smoothing, so the whole picture is
//! read first, and smoothed after.

use crate::vp8_tables::{
    AC_TABLE, COEFF_UPDATE_PROBS, DC_TABLE, DEFAULT_COEFF_PROBS, KF_BMODE_PROBS,
};
use crate::{ImageError, MAX_PIXELS};

const INVALID: ImageError = ImageError::Invalid("its compressed data doesn't make sense");

/// Reads yes-or-no answers, each of which takes less than a bit if it's the one that was expected.
struct BoolReader<'a> {
    data: &'a [u8],
    at: usize,
    value: u64,
    /// How many bits of `value` are below the eight being compared.
    bits: i32,
    range: u32,
}

impl<'a> BoolReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            at: 0,
            value: 0,
            bits: -8,
            range: 255,
        }
    }

    #[inline]
    fn load(&mut self) {
        while self.bits < 0 {
            // Past the end it carries on with zeros, which is what decoders do.
            let byte = self.data.get(self.at).copied().unwrap_or(0);
            self.at += 1;
            self.value = (self.value << 8) | u64::from(byte);
            self.bits += 8;
        }
    }

    #[inline]
    fn read(&mut self, probability: u8) -> bool {
        self.load();
        let split = 1 + (((self.range - 1) * u32::from(probability)) >> 8);
        let big = u64::from(split) << self.bits;
        let answer = self.value >= big;
        if answer {
            self.range -= split;
            self.value -= big;
        } else {
            self.range = split;
        }
        let shift = self.range.leading_zeros() as i32 - 24;
        self.range <<= shift;
        self.bits -= shift;
        answer
    }

    #[inline]
    fn bit(&mut self) -> bool {
        self.read(128)
    }

    fn literal(&mut self, bits: u32) -> u32 {
        let mut value = 0;
        for _ in 0..bits {
            value = (value << 1) | u32::from(self.bit());
        }
        value
    }

    /// A number with a sign after it, if it's there at all.
    fn optional_signed(&mut self, bits: u32) -> i32 {
        if !self.bit() {
            return 0;
        }
        let value = self.literal(bits) as i32;
        if self.bit() { -value } else { value }
    }

    /// Follows a tree to its answer. Positive entries are where to go next, and others are answers.
    #[inline]
    fn tree(&mut self, tree: &[i8], probabilities: &[u8]) -> u8 {
        let mut at = 0usize;
        loop {
            let chosen = usize::from(self.read(probabilities[at >> 1]));
            let next = tree[at + chosen];
            if next <= 0 {
                return (-next) as u8;
            }
            at = next as usize;
        }
    }
}

const DC_PRED: u8 = 0;
const V_PRED: u8 = 1;
const H_PRED: u8 = 2;
const TM_PRED: u8 = 3;
const B_PRED: u8 = 4;

const B_DC_PRED: u8 = 0;
const B_TM_PRED: u8 = 1;
const B_VE_PRED: u8 = 2;
const B_HE_PRED: u8 = 3;
const B_LD_PRED: u8 = 4;
const B_RD_PRED: u8 = 5;
const B_VR_PRED: u8 = 6;
const B_VL_PRED: u8 = 7;
const B_HD_PRED: u8 = 8;
const B_HU_PRED: u8 = 9;

const KF_YMODE_TREE: [i8; 8] = [
    -(B_PRED as i8),
    2,
    4,
    6,
    -(DC_PRED as i8),
    -(V_PRED as i8),
    -(H_PRED as i8),
    -(TM_PRED as i8),
];
const KF_YMODE_PROBS: [u8; 4] = [145, 156, 163, 128];
const UV_MODE_TREE: [i8; 6] = [
    -(DC_PRED as i8),
    2,
    -(V_PRED as i8),
    4,
    -(H_PRED as i8),
    -(TM_PRED as i8),
];
const KF_UV_MODE_PROBS: [u8; 3] = [142, 114, 183];
const BMODE_TREE: [i8; 18] = [
    -(B_DC_PRED as i8),
    2,
    -(B_TM_PRED as i8),
    4,
    -(B_VE_PRED as i8),
    6,
    8,
    12,
    -(B_HE_PRED as i8),
    10,
    -(B_RD_PRED as i8),
    -(B_VR_PRED as i8),
    -(B_LD_PRED as i8),
    14,
    -(B_VL_PRED as i8),
    16,
    -(B_HD_PRED as i8),
    -(B_HU_PRED as i8),
];
const SEGMENT_TREE: [i8; 6] = [2, 4, 0, -1, -2, -3];

const ZIGZAG: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];
/// Which set of probabilities each position in a block uses, with one extra for just past the end.
const BANDS: [usize; 17] = [0, 1, 2, 3, 6, 4, 5, 6, 6, 6, 6, 6, 6, 6, 6, 7, 0];
const LARGE_VALUE_PROBS: [&[u8]; 4] = [
    &[173, 148, 140],
    &[176, 155, 140, 135],
    &[180, 157, 141, 134, 130],
    &[254, 254, 243, 230, 196, 177, 153, 140, 133, 130, 129],
];

/// How much each kind of number gets multiplied by, as for the first number in a block and the rest.
#[derive(Clone, Copy, Default)]
struct Quantizer {
    y1: [i32; 2],
    y2: [i32; 2],
    uv: [i32; 2],
}

#[derive(Clone, Copy, Default)]
struct FilterStrength {
    limit: i32,
    interior: i32,
    high_edge_variance: i32,
}

/// One plane of the picture, with a border to predict the first row and column from.
struct Plane {
    stride: usize,
    samples: Vec<u8>,
}

impl Plane {
    fn new(width: usize, height: usize) -> Result<Self, ImageError> {
        // A row above, a column to the left, and four on the right for what's above and to the right.
        let stride = width + 5;
        let total = stride * (height + 1);
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(total)
            .map_err(|_| ImageError::TooBig)?;
        samples.resize(total, 0u8);
        samples[..stride].fill(127);
        for row in 1..=height {
            samples[row * stride] = 129;
        }
        Ok(Self { stride, samples })
    }

    /// Where a position is. The border means that -1 is allowed in both directions.
    #[inline]
    fn at(&self, x: usize, y: usize) -> usize {
        (y + 1) * self.stride + x + 1
    }
}

/// Reads as many numbers as the block has, and says how far it got.
fn read_coefficients(
    reader: &mut BoolReader<'_>,
    probabilities: &[u8],
    kind: usize,
    context: usize,
    quantizer: [i32; 2],
    first: usize,
    out: &mut [i16],
) -> usize {
    let probs = |band: usize, context: usize| -> usize { ((kind * 8 + band) * 3 + context) * 11 };
    let mut at = first;
    let mut p = probs(BANDS[at], context);
    while at < 16 {
        if !reader.read(probabilities[p]) {
            return at;
        }
        while !reader.read(probabilities[p + 1]) {
            at += 1;
            if at == 16 {
                return 16;
            }
            p = probs(BANDS[at], 0);
        }
        let value;
        let next_context;
        if !reader.read(probabilities[p + 2]) {
            value = 1;
            next_context = 1;
        } else {
            next_context = 2;
            value = if !reader.read(probabilities[p + 3]) {
                if !reader.read(probabilities[p + 4]) {
                    2
                } else {
                    3 + i32::from(reader.read(probabilities[p + 5]))
                }
            } else if !reader.read(probabilities[p + 6]) {
                if !reader.read(probabilities[p + 7]) {
                    5 + i32::from(reader.read(159))
                } else {
                    let high = i32::from(reader.read(165));
                    7 + 2 * high + i32::from(reader.read(145))
                }
            } else {
                let high = usize::from(reader.read(probabilities[p + 8]));
                let low = usize::from(reader.read(probabilities[p + 9 + high]));
                let category = 2 * high + low;
                let mut extra = 0;
                for &probability in LARGE_VALUE_PROBS[category] {
                    extra = extra * 2 + i32::from(reader.read(probability));
                }
                extra + 3 + (8 << category)
            };
        }
        let signed = if reader.bit() { -value } else { value };
        out[ZIGZAG[at]] = (signed * quantizer[usize::from(at > 0)]) as i16;
        at += 1;
        p = probs(BANDS[at], next_context);
    }
    16
}

/// Turns the first number of each of 16 blocks back from how they were packed together.
fn inverse_walsh(input: &[i16; 16], output: &mut [i16]) {
    let mut work = [0i32; 16];
    for i in 0..4 {
        let (at0, at1, at2, at3) = (
            i32::from(input[i]),
            i32::from(input[4 + i]),
            i32::from(input[8 + i]),
            i32::from(input[12 + i]),
        );
        let (a0, a1, a2, a3) = (at0 + at3, at1 + at2, at1 - at2, at0 - at3);
        work[i] = a0 + a1;
        work[8 + i] = a0 - a1;
        work[4 + i] = a3 + a2;
        work[12 + i] = a3 - a2;
    }
    for i in 0..4 {
        let row = &work[i * 4..i * 4 + 4];
        let dc = row[0] + 3;
        let (a0, a1, a2, a3) = (dc + row[3], row[1] + row[2], row[1] - row[2], dc - row[3]);
        output[i * 4 * 16] = ((a0 + a1) >> 3) as i16;
        output[(i * 4 + 1) * 16] = ((a3 + a2) >> 3) as i16;
        output[(i * 4 + 2) * 16] = ((a0 - a1) >> 3) as i16;
        output[(i * 4 + 3) * 16] = ((a3 - a2) >> 3) as i16;
    }
}

#[inline]
fn clip(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

/// Adds a block's differences to what was predicted for it.
fn add_inverse_dct(block: &[i16], samples: &mut [u8], at: usize, stride: usize) {
    const C1: i32 = 20091 + (1 << 16);
    const C2: i32 = 35468;
    let mul = |a: i32, b: i32| (a * b) >> 16;
    let mut work = [0i32; 16];
    for i in 0..4 {
        let (i0, i1, i2, i3) = (
            i32::from(block[i]),
            i32::from(block[4 + i]),
            i32::from(block[8 + i]),
            i32::from(block[12 + i]),
        );
        let a = i0 + i2;
        let b = i0 - i2;
        let c = mul(i1, C2) - mul(i3, C1);
        let d = mul(i1, C1) + mul(i3, C2);
        work[i * 4] = a + d;
        work[i * 4 + 1] = b + c;
        work[i * 4 + 2] = b - c;
        work[i * 4 + 3] = a - d;
    }
    for i in 0..4 {
        let dc = work[i] + 4;
        let a = dc + work[8 + i];
        let b = dc - work[8 + i];
        let c = mul(work[4 + i], C2) - mul(work[12 + i], C1);
        let d = mul(work[4 + i], C1) + mul(work[12 + i], C2);
        let row = at + i * stride;
        for (x, value) in [a + d, b + c, b - c, a - d].into_iter().enumerate() {
            samples[row + x] = clip(i32::from(samples[row + x]) + (value >> 3));
        }
    }
}

#[inline]
fn avg2(a: u8, b: u8) -> u8 {
    ((u32::from(a) + u32::from(b) + 1) >> 1) as u8
}

#[inline]
fn avg3(a: u8, b: u8, c: u8) -> u8 {
    ((u32::from(a) + 2 * u32::from(b) + u32::from(c) + 2) >> 2) as u8
}

/// Predicts a whole block of this size, for luma and chroma.
fn predict_block(plane: &mut Plane, x: usize, y: usize, size: usize, mode: u8, has: (bool, bool)) {
    let stride = plane.stride;
    let at = plane.at(x, y);
    let samples = &mut plane.samples;
    let above = at - stride;
    match mode {
        V_PRED => {
            for row in 0..size {
                samples.copy_within(above..above + size, at + row * stride);
            }
        }
        H_PRED => {
            for row in 0..size {
                let start = at + row * stride;
                let left = samples[start - 1];
                samples[start..start + size].fill(left);
            }
        }
        TM_PRED => {
            let corner = i32::from(samples[above - 1]);
            for row in 0..size {
                let start = at + row * stride;
                let left = i32::from(samples[start - 1]);
                for column in 0..size {
                    samples[start + column] =
                        clip(left + i32::from(samples[above + column]) - corner);
                }
            }
        }
        _ => {
            let shift = size.trailing_zeros();
            let (has_above, has_left) = has;
            let sum_above: u32 = samples[above..above + size]
                .iter()
                .map(|value| u32::from(*value))
                .sum();
            let sum_left: u32 = (0..size)
                .map(|row| u32::from(samples[at + row * stride - 1]))
                .sum();
            let value = match (has_above, has_left) {
                (true, true) => (sum_above + sum_left + (1 << shift)) >> (shift + 1),
                (true, false) => (sum_above + (1 << (shift - 1))) >> shift,
                (false, true) => (sum_left + (1 << (shift - 1))) >> shift,
                (false, false) => 128,
            } as u8;
            for row in 0..size {
                let start = at + row * stride;
                samples[start..start + size].fill(value);
            }
        }
    }
}

/// Predicts a block of 4 by 4 from what's around it. `top` has the eight above it, from its left edge.
fn predict_small(samples: &mut [u8], at: usize, stride: usize, mode: u8, top: &[u8; 8]) {
    let corner = samples[at - stride - 1];
    let left = [
        samples[at - 1],
        samples[at + stride - 1],
        samples[at + 2 * stride - 1],
        samples[at + 3 * stride - 1],
    ];
    let (a, b, c, d, e, f, g, h) = (
        top[0], top[1], top[2], top[3], top[4], top[5], top[6], top[7],
    );
    let (i, j, k, l) = (left[0], left[1], left[2], left[3]);
    let x = corner;
    let mut out = [[0u8; 4]; 4];
    match mode {
        B_DC_PRED => {
            let sum: u32 = top[..4]
                .iter()
                .chain(&left)
                .map(|value| u32::from(*value))
                .sum();
            out = [[((sum + 4) >> 3) as u8; 4]; 4];
        }
        B_TM_PRED => {
            for (row, line) in out.iter_mut().enumerate() {
                for (column, value) in line.iter_mut().enumerate() {
                    *value =
                        clip(i32::from(left[row]) + i32::from(top[column]) - i32::from(corner));
                }
            }
        }
        B_VE_PRED => {
            let row = [avg3(x, a, b), avg3(a, b, c), avg3(b, c, d), avg3(c, d, e)];
            out = [row; 4];
        }
        B_HE_PRED => {
            out[0] = [avg3(x, i, j); 4];
            out[1] = [avg3(i, j, k); 4];
            out[2] = [avg3(j, k, l); 4];
            out[3] = [avg3(k, l, l); 4];
        }
        B_LD_PRED => {
            let v = [
                avg3(a, b, c),
                avg3(b, c, d),
                avg3(c, d, e),
                avg3(d, e, f),
                avg3(e, f, g),
                avg3(f, g, h),
                avg3(g, h, h),
            ];
            for (row, line) in out.iter_mut().enumerate() {
                for (column, value) in line.iter_mut().enumerate() {
                    *value = v[row + column];
                }
            }
        }
        B_RD_PRED => {
            let v = [
                avg3(j, k, l),
                avg3(i, j, k),
                avg3(x, i, j),
                avg3(a, x, i),
                avg3(b, a, x),
                avg3(c, b, a),
                avg3(d, c, b),
            ];
            for (row, line) in out.iter_mut().enumerate() {
                for (column, value) in line.iter_mut().enumerate() {
                    *value = v[3 + column - row];
                }
            }
        }
        B_VR_PRED => {
            out[0] = [avg2(x, a), avg2(a, b), avg2(b, c), avg2(c, d)];
            out[1] = [avg3(i, x, a), avg3(x, a, b), avg3(a, b, c), avg3(b, c, d)];
            out[2] = [avg3(j, i, x), out[0][0], out[0][1], out[0][2]];
            out[3] = [avg3(k, j, i), out[1][0], out[1][1], out[1][2]];
        }
        B_VL_PRED => {
            out[0] = [avg2(a, b), avg2(b, c), avg2(c, d), avg2(d, e)];
            out[1] = [avg3(a, b, c), avg3(b, c, d), avg3(c, d, e), avg3(d, e, f)];
            out[2] = [out[0][1], out[0][2], out[0][3], avg3(e, f, g)];
            out[3] = [out[1][1], out[1][2], out[1][3], avg3(f, g, h)];
        }
        B_HD_PRED => {
            out[0] = [avg2(i, x), avg3(i, x, a), avg3(x, a, b), avg3(a, b, c)];
            out[1] = [avg2(j, i), avg3(j, i, x), out[0][0], out[0][1]];
            out[2] = [avg2(k, j), avg3(k, j, i), out[1][0], out[1][1]];
            out[3] = [avg2(l, k), avg3(l, k, j), out[2][0], out[2][1]];
        }
        _ => {
            out[0] = [avg2(i, j), avg3(i, j, k), avg2(j, k), avg3(j, k, l)];
            out[1] = [avg2(j, k), avg3(j, k, l), avg2(k, l), avg3(k, l, l)];
            out[2] = [avg2(k, l), avg3(k, l, l), l, l];
            out[3] = [l; 4];
        }
    }
    for (row, line) in out.iter().enumerate() {
        samples[at + row * stride..at + row * stride + 4].copy_from_slice(line);
    }
}

#[inline]
fn signed_clip_1(value: i32) -> i32 {
    value.clamp(-128, 127)
}

#[inline]
fn signed_clip_2(value: i32) -> i32 {
    value.clamp(-16, 15)
}

#[inline]
fn value(samples: &[u8], at: isize) -> i32 {
    i32::from(samples[at as usize])
}

/// Smooths two samples either side of an edge, and the ones next to those only a little.
#[inline]
fn filter_2(samples: &mut [u8], at: isize, step: isize) {
    let (p1, p0, q0, q1) = (
        value(samples, at - 2 * step),
        value(samples, at - step),
        value(samples, at),
        value(samples, at + step),
    );
    let a = 3 * (q0 - p0) + signed_clip_1(p1 - q1);
    let a1 = signed_clip_2((a + 4) >> 3);
    let a2 = signed_clip_2((a + 3) >> 3);
    samples[(at - step) as usize] = clip(p0 + a2);
    samples[at as usize] = clip(q0 - a1);
}

#[inline]
fn filter_4(samples: &mut [u8], at: isize, step: isize) {
    let (p1, p0, q0, q1) = (
        value(samples, at - 2 * step),
        value(samples, at - step),
        value(samples, at),
        value(samples, at + step),
    );
    let a = 3 * (q0 - p0);
    let a1 = signed_clip_2((a + 4) >> 3);
    let a2 = signed_clip_2((a + 3) >> 3);
    let a3 = (a1 + 1) >> 1;
    samples[(at - 2 * step) as usize] = clip(p1 + a3);
    samples[(at - step) as usize] = clip(p0 + a2);
    samples[at as usize] = clip(q0 - a1);
    samples[(at + step) as usize] = clip(q1 - a3);
}

#[inline]
fn filter_6(samples: &mut [u8], at: isize, step: isize) {
    let (p2, p1, p0, q0, q1, q2) = (
        value(samples, at - 3 * step),
        value(samples, at - 2 * step),
        value(samples, at - step),
        value(samples, at),
        value(samples, at + step),
        value(samples, at + 2 * step),
    );
    let a = signed_clip_1(3 * (q0 - p0) + signed_clip_1(p1 - q1));
    let a1 = (27 * a + 63) >> 7;
    let a2 = (18 * a + 63) >> 7;
    let a3 = (9 * a + 63) >> 7;
    samples[(at - 3 * step) as usize] = clip(p2 + a3);
    samples[(at - 2 * step) as usize] = clip(p1 + a2);
    samples[(at - step) as usize] = clip(p0 + a1);
    samples[at as usize] = clip(q0 - a1);
    samples[(at + step) as usize] = clip(q1 - a2);
    samples[(at + 2 * step) as usize] = clip(q2 - a3);
}

#[inline]
fn is_smooth_enough(samples: &[u8], at: isize, step: isize, limit: i32) -> bool {
    let (p1, p0, q0, q1) = (
        value(samples, at - 2 * step),
        value(samples, at - step),
        value(samples, at),
        value(samples, at + step),
    );
    4 * (p0 - q0).abs() + (p1 - q1).abs() <= limit
}

#[inline]
fn needs_filter(samples: &[u8], at: isize, step: isize, limit: i32, interior: i32) -> bool {
    if !is_smooth_enough(samples, at, step, limit) {
        return false;
    }
    let near = |a: isize, b: isize| {
        (value(samples, at + a * step) - value(samples, at + b * step)).abs() <= interior
    };
    near(-4, -3) && near(-3, -2) && near(-2, -1) && near(3, 2) && near(2, 1) && near(1, 0)
}

#[inline]
fn high_edge_variance(samples: &[u8], at: isize, step: isize, threshold: i32) -> bool {
    (value(samples, at - 2 * step) - value(samples, at - step)).abs() > threshold
        || (value(samples, at + step) - value(samples, at)).abs() > threshold
}

/// The edge between macroblocks, or `count` samples along one between blocks.
fn filter_edge(
    samples: &mut [u8],
    at: usize,
    along: isize,
    across: isize,
    count: isize,
    strength: (i32, i32, i32),
    between_macroblocks: bool,
) {
    let (limit, interior, threshold) = strength;
    let limit = 2 * limit + 1;
    for index in 0..count {
        let here = at as isize + index * along;
        if !needs_filter(samples, here, across, limit, interior) {
            continue;
        }
        if high_edge_variance(samples, here, across, threshold) {
            filter_2(samples, here, across);
        } else if between_macroblocks {
            filter_6(samples, here, across);
        } else {
            filter_4(samples, here, across);
        }
    }
}

fn simple_edge(samples: &mut [u8], at: usize, along: isize, across: isize, limit: i32) {
    let limit = 2 * limit + 1;
    for index in 0..16 {
        let here = at as isize + index * along;
        if is_smooth_enough(samples, here, across, limit) {
            filter_2(samples, here, across);
        }
    }
}

/// Everything in a frame's first partition that applies to all of it.
struct Decoder<'a> {
    first: BoolReader<'a>,
    partitions: Vec<BoolReader<'a>>,
    width: usize,
    height: usize,
    mb_width: usize,
    mb_height: usize,
    probabilities: Vec<u8>,
    update_map: bool,
    segment_probs: [u8; 3],
    segment_absolute: bool,
    segment_quant: [i32; 4],
    segment_filter: [i32; 4],
    segmentation: bool,
    simple_filter: bool,
    filter_level: i32,
    sharpness: i32,
    use_deltas: bool,
    ref_delta_0: i32,
    mode_delta_0: i32,
    use_skip: bool,
    skip_probability: u8,
    quantizers: [Quantizer; 4],
}

/// What's left to know about a macroblock to smooth it, once the frame's been read.
#[derive(Clone, Copy, Default)]
struct MacroblockFilter {
    limit: i32,
    interior: i32,
    high_edge_variance: i32,
    inner: bool,
}

/// The picture, read as brightness and color, with color at half the size each way.
pub(crate) struct Frame {
    pub(crate) width: usize,
    pub(crate) height: usize,
    y: Plane,
    u: Plane,
    v: Plane,
}

/// The size in a lossy chunk's header.
pub(crate) fn size_of(data: &[u8]) -> Result<(u32, u32), ImageError> {
    let header = data
        .get(..10)
        .ok_or(ImageError::Invalid("it ends too soon"))?;
    if header[0] & 1 != 0 {
        return Err(ImageError::Invalid("it doesn't start with a whole picture"));
    }
    if header[3..6] != [0x9D, 0x01, 0x2A] {
        return Err(ImageError::Invalid("its header is wrong"));
    }
    let width = u32::from(u16::from_le_bytes([header[6], header[7]]) & 0x3FFF);
    let height = u32::from(u16::from_le_bytes([header[8], header[9]]) & 0x3FFF);
    if width == 0 || height == 0 {
        return Err(ImageError::Invalid("it has no size"));
    }
    Ok((width, height))
}

impl<'a> Decoder<'a> {
    fn new(data: &'a [u8]) -> Result<Self, ImageError> {
        let (width, height) = size_of(data)?;
        if u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(ImageError::TooBig);
        }
        let tag = u32::from(data[0]) | u32::from(data[1]) << 8 | u32::from(data[2]) << 16;
        let first_size = (tag >> 5) as usize;
        let rest = &data[10..];
        let first_part = rest.get(..first_size.min(rest.len())).unwrap_or(rest);
        let after_first = rest.get(first_part.len()..).unwrap_or(&[]);
        let (width, height) = (width as usize, height as usize);
        let mut decoder = Self {
            first: BoolReader::new(first_part),
            partitions: Vec::new(),
            width,
            height,
            mb_width: width.div_ceil(16),
            mb_height: height.div_ceil(16),
            probabilities: DEFAULT_COEFF_PROBS.to_vec(),
            update_map: false,
            segment_probs: [255; 3],
            segment_absolute: false,
            segment_quant: [0; 4],
            segment_filter: [0; 4],
            segmentation: false,
            simple_filter: false,
            filter_level: 0,
            sharpness: 0,
            use_deltas: false,
            ref_delta_0: 0,
            mode_delta_0: 0,
            use_skip: false,
            skip_probability: 0,
            quantizers: [Quantizer::default(); 4],
        };
        decoder.read_header(after_first)?;
        Ok(decoder)
    }

    fn read_header(&mut self, after_first: &'a [u8]) -> Result<(), ImageError> {
        let reader = &mut self.first;
        // Which color space it is, and whether it needs clamping, neither of which changes anything.
        reader.literal(2);
        self.segmentation = reader.bit();
        if self.segmentation {
            self.update_map = reader.bit();
            if reader.bit() {
                self.segment_absolute = reader.bit();
                for value in &mut self.segment_quant {
                    *value = reader.optional_signed(7);
                }
                for value in &mut self.segment_filter {
                    *value = reader.optional_signed(6);
                }
            }
            if self.update_map {
                for probability in &mut self.segment_probs {
                    *probability = if reader.bit() {
                        reader.literal(8) as u8
                    } else {
                        255
                    };
                }
            }
        }
        self.simple_filter = reader.bit();
        self.filter_level = reader.literal(6) as i32;
        self.sharpness = reader.literal(3) as i32;
        self.use_deltas = reader.bit();
        if self.use_deltas && reader.bit() {
            for index in 0..4 {
                if reader.bit() {
                    let delta = reader.literal(6) as i32;
                    let delta = if reader.bit() { -delta } else { delta };
                    if index == 0 {
                        self.ref_delta_0 = delta;
                    }
                }
            }
            for index in 0..4 {
                if reader.bit() {
                    let delta = reader.literal(6) as i32;
                    let delta = if reader.bit() { -delta } else { delta };
                    if index == 0 {
                        self.mode_delta_0 = delta;
                    }
                }
            }
        }
        let partition_count = 1usize << reader.literal(2);
        let sizes_length = 3 * (partition_count - 1);
        let sizes = after_first.get(..sizes_length).ok_or(INVALID)?;
        let mut rest = &after_first[sizes_length..];
        for index in 0..partition_count {
            let size = if index + 1 < partition_count {
                let at = index * 3;
                usize::from(sizes[at])
                    | usize::from(sizes[at + 1]) << 8
                    | usize::from(sizes[at + 2]) << 16
            } else {
                rest.len()
            };
            let (part, after) = rest.split_at(size.min(rest.len()));
            self.partitions.push(BoolReader::new(part));
            rest = after;
        }
        let reader = &mut self.first;
        let base = reader.literal(7) as i32;
        let y_dc = reader.optional_signed(4);
        let y2_dc = reader.optional_signed(4);
        let y2_ac = reader.optional_signed(4);
        let uv_dc = reader.optional_signed(4);
        let uv_ac = reader.optional_signed(4);
        for segment in 0..4 {
            let q = if !self.segmentation {
                base
            } else if self.segment_absolute {
                self.segment_quant[segment]
            } else {
                base + self.segment_quant[segment]
            };
            let dc = |delta: i32| i32::from(DC_TABLE[(q + delta).clamp(0, 127) as usize]);
            let ac = |delta: i32| i32::from(AC_TABLE[(q + delta).clamp(0, 127) as usize]);
            self.quantizers[segment] = Quantizer {
                y1: [dc(y_dc), ac(0)],
                y2: [dc(y2_dc) * 2, ((ac(y2_ac) * 101_581) >> 16).max(8)],
                uv: [dc(uv_dc).min(132), ac(uv_ac)],
            };
        }
        // Whether to keep these probabilities for later frames, of which there aren't any.
        reader.bit();
        for (probability, &update) in self.probabilities.iter_mut().zip(COEFF_UPDATE_PROBS.iter()) {
            if reader.read(update) {
                *probability = reader.literal(8) as u8;
            }
        }
        self.use_skip = reader.bit();
        if self.use_skip {
            self.skip_probability = reader.literal(8) as u8;
        }
        Ok(())
    }

    fn filter_strength(&self, segment: usize, small_blocks: bool) -> FilterStrength {
        let mut level = self.filter_level;
        if self.segmentation {
            level = if self.segment_absolute {
                self.segment_filter[segment]
            } else {
                level + self.segment_filter[segment]
            };
        }
        if self.use_deltas {
            level += self.ref_delta_0;
            if small_blocks {
                level += self.mode_delta_0;
            }
        }
        let level = level.clamp(0, 63);
        if level == 0 {
            return FilterStrength::default();
        }
        let mut interior = level;
        if self.sharpness > 0 {
            interior >>= if self.sharpness > 4 { 2 } else { 1 };
            interior = interior.min(9 - self.sharpness);
        }
        let interior = interior.max(1);
        FilterStrength {
            limit: 2 * level + interior,
            interior,
            high_edge_variance: if level >= 40 {
                2
            } else if level >= 15 {
                1
            } else {
                0
            },
        }
    }

    fn read_frame(mut self) -> Result<Frame, ImageError> {
        let (mb_width, mb_height) = (self.mb_width, self.mb_height);
        let mut y = Plane::new(mb_width * 16, mb_height * 16)?;
        let mut u = Plane::new(mb_width * 8, mb_height * 8)?;
        let mut v = Plane::new(mb_width * 8, mb_height * 8)?;
        // Each column's small-block modes along the bottom of the row above, and the same on the left.
        let mut above_modes = vec![B_DC_PRED; mb_width * 4];
        // What's non-zero: four for luma, two each for the colors, then one for the luma's summary.
        let mut above_nonzero = vec![[0u8; 9]; mb_width];
        let mut filters = vec![MacroblockFilter::default(); mb_width * mb_height];
        let mut coefficients = [0i16; 25 * 16];
        let partition_mask = self.partitions.len() - 1;
        for mb_y in 0..mb_height {
            let mut left_modes = [B_DC_PRED; 4];
            let mut left_nonzero = [0u8; 9];
            for mb_x in 0..mb_width {
                let segment = if self.update_map {
                    usize::from(self.first.tree(&SEGMENT_TREE, &self.segment_probs))
                } else {
                    0
                };
                let skip_said = self.use_skip && self.first.read(self.skip_probability);
                let y_mode = self.first.tree(&KF_YMODE_TREE, &KF_YMODE_PROBS);
                let mut small_modes = [B_DC_PRED; 16];
                if y_mode == B_PRED {
                    for row in 0..4 {
                        let mut left = left_modes[row];
                        for column in 0..4 {
                            let above = above_modes[mb_x * 4 + column];
                            let offset = (usize::from(above) * 10 + usize::from(left)) * 9;
                            let mode = self
                                .first
                                .tree(&BMODE_TREE, &KF_BMODE_PROBS[offset..offset + 9]);
                            small_modes[row * 4 + column] = mode;
                            above_modes[mb_x * 4 + column] = mode;
                            left = mode;
                        }
                        left_modes[row] = left;
                    }
                } else {
                    let implied = match y_mode {
                        V_PRED => B_VE_PRED,
                        H_PRED => B_HE_PRED,
                        TM_PRED => B_TM_PRED,
                        _ => B_DC_PRED,
                    };
                    left_modes = [implied; 4];
                    above_modes[mb_x * 4..mb_x * 4 + 4].fill(implied);
                }
                let uv_mode = self.first.tree(&UV_MODE_TREE, &KF_UV_MODE_PROBS);

                coefficients.fill(0);
                let above_flags = &mut above_nonzero[mb_x];
                let mut has_coefficients = false;
                if skip_said {
                    let (keep_above, keep_left) = (above_flags[8], left_nonzero[8]);
                    *above_flags = [0; 9];
                    left_nonzero = [0; 9];
                    // Blocks that have no summary leave what the last one that did said.
                    if y_mode == B_PRED {
                        above_flags[8] = keep_above;
                        left_nonzero[8] = keep_left;
                    }
                } else {
                    let quantizer = self.quantizers[segment];
                    let reader = &mut self.partitions[mb_y & partition_mask];
                    has_coefficients = read_residuals(
                        reader,
                        &self.probabilities,
                        &quantizer,
                        y_mode == B_PRED,
                        above_flags,
                        &mut left_nonzero,
                        &mut coefficients,
                    );
                }

                reconstruct(
                    &mut y,
                    &mut u,
                    &mut v,
                    (mb_x, mb_y, mb_width),
                    y_mode,
                    uv_mode,
                    &small_modes,
                    &coefficients,
                );

                let strength = self.filter_strength(segment, y_mode == B_PRED);
                filters[mb_y * mb_width + mb_x] = MacroblockFilter {
                    limit: strength.limit,
                    interior: strength.interior,
                    high_edge_variance: strength.high_edge_variance,
                    inner: y_mode == B_PRED || has_coefficients,
                };
            }
        }
        if self.filter_level > 0 {
            smooth(
                &mut y,
                &mut u,
                &mut v,
                &filters,
                (mb_width, mb_height),
                self.simple_filter,
            );
        }
        Ok(Frame {
            width: self.width,
            height: self.height,
            y,
            u,
            v,
        })
    }
}

/// Reads all the numbers in a macroblock, and says whether there were any.
#[allow(clippy::needless_range_loop)]
fn read_residuals(
    reader: &mut BoolReader<'_>,
    probabilities: &[u8],
    quantizer: &Quantizer,
    small_blocks: bool,
    above: &mut [u8; 9],
    left: &mut [u8; 9],
    coefficients: &mut [i16; 25 * 16],
) -> bool {
    let mut any = false;
    let mut first = 0;
    let mut luma_kind = 3;
    if !small_blocks {
        let mut summary = [0i16; 16];
        let context = usize::from(above[8] + left[8]);
        let count = read_coefficients(
            reader,
            probabilities,
            1,
            context,
            quantizer.y2,
            0,
            &mut summary,
        );
        let flag = u8::from(count > 0);
        above[8] = flag;
        left[8] = flag;
        if count > 0 {
            inverse_walsh(&summary, &mut coefficients[..16 * 16]);
        }
        first = 1;
        luma_kind = 0;
    }
    for row in 0..4 {
        for column in 0..4 {
            let context = usize::from(above[column] + left[row]);
            let start = (row * 4 + column) * 16;
            let count = read_coefficients(
                reader,
                probabilities,
                luma_kind,
                context,
                quantizer.y1,
                first,
                &mut coefficients[start..start + 16],
            );
            let flag = u8::from(count > first);
            above[column] = flag;
            left[row] = flag;
            // What came from the summary above counts, if it's not zero.
            if count > 1 || coefficients[start] != 0 {
                any = true;
            }
        }
    }
    for plane in 0..2 {
        let offset = 4 + plane * 2;
        for row in 0..2 {
            for column in 0..2 {
                let context = usize::from(above[offset + column] + left[offset + row]);
                let start = (16 + plane * 4 + row * 2 + column) * 16;
                let count = read_coefficients(
                    reader,
                    probabilities,
                    2,
                    context,
                    quantizer.uv,
                    0,
                    &mut coefficients[start..start + 16],
                );
                let flag = u8::from(count > 0);
                above[offset + column] = flag;
                left[offset + row] = flag;
                if count > 1 || coefficients[start] != 0 {
                    any = true;
                }
            }
        }
    }
    any
}

#[allow(clippy::too_many_arguments)]
fn reconstruct(
    y: &mut Plane,
    u: &mut Plane,
    v: &mut Plane,
    (mb_x, mb_y, mb_width): (usize, usize, usize),
    y_mode: u8,
    uv_mode: u8,
    small_modes: &[u8; 16],
    coefficients: &[i16; 25 * 16],
) {
    let (x, top) = (mb_x * 16, mb_y * 16);
    let has = (mb_y > 0, mb_x > 0);
    let stride = y.stride;
    let is_zero = |block: &[i16]| block.iter().all(|value| *value == 0);
    if y_mode == B_PRED {
        // All of the blocks on the right use what's above and to the right of the macroblock, as what's
        // to the right of them in this row hasn't been read yet.
        let above_row = y.at(x, top) - stride;
        let mut right = [127u8; 4];
        if mb_y > 0 {
            if mb_x + 1 < mb_width {
                right.copy_from_slice(&y.samples[above_row + 16..above_row + 20]);
            } else {
                right = [y.samples[above_row + 15]; 4];
            }
        }
        for row in 0..4 {
            for column in 0..4 {
                let at = y.at(x + column * 4, top + row * 4);
                let mut above = [0u8; 8];
                above[..4].copy_from_slice(&y.samples[at - stride..at - stride + 4]);
                if column == 3 {
                    above[4..].copy_from_slice(&right);
                } else {
                    above[4..].copy_from_slice(&y.samples[at - stride + 4..at - stride + 8]);
                }
                predict_small(
                    &mut y.samples,
                    at,
                    stride,
                    small_modes[row * 4 + column],
                    &above,
                );
                let block = &coefficients[(row * 4 + column) * 16..][..16];
                if !is_zero(block) {
                    add_inverse_dct(block, &mut y.samples, at, stride);
                }
            }
        }
    } else {
        predict_block(y, x, top, 16, y_mode, has);
        for row in 0..4 {
            for column in 0..4 {
                let block = &coefficients[(row * 4 + column) * 16..][..16];
                if !is_zero(block) {
                    let at = y.at(x + column * 4, top + row * 4);
                    add_inverse_dct(block, &mut y.samples, at, stride);
                }
            }
        }
    }
    let (x, top) = (mb_x * 8, mb_y * 8);
    for (plane, first) in [(u, 16), (v, 20)] {
        predict_block(plane, x, top, 8, uv_mode, has);
        let stride = plane.stride;
        for row in 0..2 {
            for column in 0..2 {
                let block = &coefficients[(first + row * 2 + column) * 16..][..16];
                if !is_zero(block) {
                    let at = plane.at(x + column * 4, top + row * 4);
                    add_inverse_dct(block, &mut plane.samples, at, stride);
                }
            }
        }
    }
}

/// Smooths the edges of blocks, which would otherwise show.
fn smooth(
    y: &mut Plane,
    u: &mut Plane,
    v: &mut Plane,
    filters: &[MacroblockFilter],
    (mb_width, mb_height): (usize, usize),
    simple: bool,
) {
    let (y_step, uv_step) = (y.stride as isize, u.stride as isize);
    for mb_y in 0..mb_height {
        for mb_x in 0..mb_width {
            let filter = filters[mb_y * mb_width + mb_x];
            if filter.limit == 0 {
                continue;
            }
            let (limit, interior, threshold) =
                (filter.limit, filter.interior, filter.high_edge_variance);
            let at = y.at(mb_x * 16, mb_y * 16);
            if simple {
                if mb_x > 0 {
                    simple_edge(&mut y.samples, at, y_step, 1, limit + 4);
                }
                if filter.inner {
                    for offset in [4, 8, 12] {
                        simple_edge(&mut y.samples, at + offset, y_step, 1, limit);
                    }
                }
                if mb_y > 0 {
                    simple_edge(&mut y.samples, at, 1, y_step, limit + 4);
                }
                if filter.inner {
                    for offset in [4, 8, 12] {
                        let start = at + offset * y.stride;
                        simple_edge(&mut y.samples, start, 1, y_step, limit);
                    }
                }
                continue;
            }
            let (uv_at, v_at) = (u.at(mb_x * 8, mb_y * 8), v.at(mb_x * 8, mb_y * 8));
            let outer = (limit + 4, interior, threshold);
            let inner = (limit, interior, threshold);
            if mb_x > 0 {
                filter_edge(&mut y.samples, at, y_step, 1, 16, outer, true);
                filter_edge(&mut u.samples, uv_at, uv_step, 1, 8, outer, true);
                filter_edge(&mut v.samples, v_at, uv_step, 1, 8, outer, true);
            }
            if filter.inner {
                for offset in [4, 8, 12] {
                    filter_edge(&mut y.samples, at + offset, y_step, 1, 16, inner, false);
                }
                filter_edge(&mut u.samples, uv_at + 4, uv_step, 1, 8, inner, false);
                filter_edge(&mut v.samples, v_at + 4, uv_step, 1, 8, inner, false);
            }
            if mb_y > 0 {
                filter_edge(&mut y.samples, at, 1, y_step, 16, outer, true);
                filter_edge(&mut u.samples, uv_at, 1, uv_step, 8, outer, true);
                filter_edge(&mut v.samples, v_at, 1, uv_step, 8, outer, true);
            }
            if filter.inner {
                for offset in [4, 8, 12] {
                    let start = at + offset * y.stride;
                    filter_edge(&mut y.samples, start, 1, y_step, 16, inner, false);
                }
                let uv_start = uv_at + 4 * u.stride;
                filter_edge(&mut u.samples, uv_start, 1, uv_step, 8, inner, false);
                let v_start = v_at + 4 * v.stride;
                filter_edge(&mut v.samples, v_start, 1, uv_step, 8, inner, false);
            }
        }
    }
}

#[inline]
fn to_rgb(y: u8, u: u8, v: u8, out: &mut [u8]) {
    let scale = |value: u8, by: i32| (i32::from(value) * by) >> 8;
    let finish = |value: i32| (value >> 6).clamp(0, 255) as u8;
    let luma = scale(y, 19077);
    out[0] = finish(luma + scale(v, 26149) - 14234);
    out[1] = finish(luma - scale(u, 6419) - scale(v, 13320) + 8708);
    out[2] = finish(luma + scale(u, 33050) - 17685);
}

/// Writes rows of red, green and blue, with the color planes brought up to full size smoothly. Alpha is
/// left as it is.
pub(crate) fn write_rows(frame: &Frame, mut row_out: impl FnMut(&mut [u8])) {
    let (width, height) = (frame.width, frame.height);
    let half_width = width.div_ceil(2);
    let half_height = height.div_ceil(2);
    let mut first = vec![255u8; width * 4];
    let mut second = vec![255u8; width * 4];
    let u_row = |row: usize| &frame.u.samples[frame.u.at(0, row)..frame.u.at(half_width, row)];
    let v_row = |row: usize| &frame.v.samples[frame.v.at(0, row)..frame.v.at(half_width, row)];
    let y_row = |row: usize| &frame.y.samples[frame.y.at(0, row)..frame.y.at(width, row)];
    // The first row only has itself to work from, and so on the last if the height's even.
    upsample(
        y_row(0),
        None,
        (u_row(0), v_row(0)),
        (u_row(0), v_row(0)),
        &mut first,
        &mut second,
    );
    row_out(&mut first);
    let mut y = 1;
    while y + 1 < height {
        let chroma = y.div_ceil(2);
        upsample(
            y_row(y),
            Some(y_row(y + 1)),
            (u_row(chroma - 1), v_row(chroma - 1)),
            (u_row(chroma), v_row(chroma)),
            &mut first,
            &mut second,
        );
        row_out(&mut first);
        row_out(&mut second);
        y += 2;
    }
    if height > 1 && height % 2 == 0 {
        let last = half_height - 1;
        upsample(
            y_row(height - 1),
            None,
            (u_row(last), v_row(last)),
            (u_row(last), v_row(last)),
            &mut first,
            &mut second,
        );
        row_out(&mut first);
    }
}

fn upsample(
    top: &[u8],
    bottom: Option<&[u8]>,
    upper: (&[u8], &[u8]),
    lower: (&[u8], &[u8]),
    top_out: &mut [u8],
    bottom_out: &mut [u8],
) {
    let length = top.len();
    let half = length.div_ceil(2);
    let edge = |a: u8, b: u8| ((3 * u32::from(a) + u32::from(b) + 2) >> 2) as u8;
    let put = |y_row: &[u8], x: usize, u: u8, v: u8, out: &mut [u8]| {
        to_rgb(y_row[x], u, v, &mut out[x * 4..x * 4 + 3]);
    };
    let (mut up_l_u, mut up_l_v) = (upper.0[0], upper.1[0]);
    let (mut low_l_u, mut low_l_v) = (lower.0[0], lower.1[0]);
    put(
        top,
        0,
        edge(up_l_u, low_l_u),
        edge(up_l_v, low_l_v),
        top_out,
    );
    if let Some(bottom) = bottom {
        put(
            bottom,
            0,
            edge(low_l_u, up_l_u),
            edge(low_l_v, up_l_v),
            bottom_out,
        );
    }
    for x in 1..half {
        let (up_u, up_v) = (upper.0[x], upper.1[x]);
        let (low_u, low_v) = (lower.0[x], lower.1[x]);
        let mix = |tl: u8, t: u8, l: u8, c: u8| {
            let (tl, t, l, c) = (u32::from(tl), u32::from(t), u32::from(l), u32::from(c));
            let average = tl + t + l + c + 8;
            let diagonal_12 = (average + 2 * (t + l)) >> 3;
            let diagonal_03 = (average + 2 * (tl + c)) >> 3;
            (
                ((diagonal_12 + tl) >> 1) as u8,
                ((diagonal_03 + t) >> 1) as u8,
                ((diagonal_03 + l) >> 1) as u8,
                ((diagonal_12 + c) >> 1) as u8,
            )
        };
        let (u0, u1, u2, u3) = mix(up_l_u, up_u, low_l_u, low_u);
        let (v0, v1, v2, v3) = mix(up_l_v, up_v, low_l_v, low_v);
        put(top, 2 * x - 1, u0, v0, top_out);
        put(top, 2 * x, u1, v1, top_out);
        if let Some(bottom) = bottom {
            put(bottom, 2 * x - 1, u2, v2, bottom_out);
            put(bottom, 2 * x, u3, v3, bottom_out);
        }
        (up_l_u, up_l_v) = (up_u, up_v);
        (low_l_u, low_l_v) = (low_u, low_v);
    }
    if length % 2 == 0 && length > 0 {
        put(
            top,
            length - 1,
            edge(up_l_u, low_l_u),
            edge(up_l_v, low_l_v),
            top_out,
        );
        if let Some(bottom) = bottom {
            put(
                bottom,
                length - 1,
                edge(low_l_u, up_l_u),
                edge(low_l_v, up_l_v),
                bottom_out,
            );
        }
    }
}

/// Reads a lossy chunk into brightness and color.
pub(crate) fn decode(data: &[u8]) -> Result<Frame, ImageError> {
    Decoder::new(data)?.read_frame()
}
