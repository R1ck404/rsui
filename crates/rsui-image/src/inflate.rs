//! Unpacks the zlib data in a PNG, handing out what it finds as it goes, rather than holding it all.

use crate::ImageError;

const INVALID: ImageError = ImageError::Invalid("its compressed data doesn't make sense");

/// How many bits a code can be looked up with in one go. Most codes are shorter than this.
const FAST_BITS: u32 = 10;
/// How much earlier output a repeat can refer to.
const WINDOW: usize = 32 * 1024;

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DISTANCE_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Reads bits, lowest first, across several separate pieces of data.
struct Bits<'a> {
    pieces: &'a [&'a [u8]],
    piece: usize,
    at: usize,
    held: u64,
    count: u32,
}

impl Bits<'_> {
    /// Tops up what's held, as far as there's data to.
    #[inline]
    fn fill(&mut self) {
        while self.count <= 56 {
            let Some(data) = self.pieces.get(self.piece) else {
                return;
            };
            if let Some(&byte) = data.get(self.at) {
                self.held |= u64::from(byte) << self.count;
                self.count += 8;
                self.at += 1;
            } else {
                self.piece += 1;
                self.at = 0;
            }
        }
    }

    #[inline]
    fn take(&mut self, bits: u32) -> Result<u32, ImageError> {
        if bits == 0 {
            return Ok(0);
        }
        if self.count < bits {
            self.fill();
            if self.count < bits {
                return Err(ImageError::Invalid("its compressed data ends too soon"));
            }
        }
        let value = (self.held & ((1u64 << bits) - 1)) as u32;
        self.held >>= bits;
        self.count -= bits;
        Ok(value)
    }

    fn align_to_byte(&mut self) {
        let extra = self.count % 8;
        self.held >>= extra;
        self.count -= extra;
    }
}

/// A set of codes, ready to look up.
struct Codes {
    /// By the next `FAST_BITS` bits, the symbol and how many bits its code has, or 0 if it's longer.
    fast: Box<[u16; 1 << FAST_BITS]>,
    /// For longer codes, the canonical arrangement: how many of each length, and symbols in order.
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Codes {
    fn new(lengths: &[u8]) -> Result<Self, ImageError> {
        let mut counts = [0u16; 16];
        for &length in lengths {
            counts[length as usize] += 1;
        }
        counts[0] = 0;
        let mut left = 1i32;
        for count in &counts[1..] {
            left = left * 2 - i32::from(*count);
            if left < 0 {
                return Err(INVALID);
            }
        }
        let mut offsets = [0u16; 16];
        for length in 1..15 {
            offsets[length + 1] = offsets[length] + counts[length];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (symbol, &length) in lengths.iter().enumerate() {
            if length != 0 {
                symbols[offsets[length as usize] as usize] = symbol as u16;
                offsets[length as usize] += 1;
            }
        }
        // Each code goes into the table at every entry that starts with it. Codes are read from
        // their top bit, so they go in reversed.
        let mut fast = Box::new([0u16; 1 << FAST_BITS]);
        let mut code = 0u32;
        let mut index = 0usize;
        for length in 1..=15u32 {
            for _ in 0..counts[length as usize] {
                if length <= FAST_BITS {
                    let reversed = code.reverse_bits() >> (32 - length);
                    let entry = (symbols[index] << 4) | length as u16;
                    let mut slot = reversed as usize;
                    while slot < 1 << FAST_BITS {
                        fast[slot] = entry;
                        slot += 1 << length;
                    }
                }
                code += 1;
                index += 1;
            }
            code <<= 1;
        }
        Ok(Self {
            fast,
            counts,
            symbols,
        })
    }

    #[inline]
    fn read(&self, bits: &mut Bits<'_>) -> Result<u16, ImageError> {
        if bits.count < 15 {
            bits.fill();
        }
        let entry = self.fast[(bits.held & ((1 << FAST_BITS) - 1)) as usize];
        if entry != 0 {
            let length = u32::from(entry & 15);
            if length > bits.count {
                return Err(INVALID);
            }
            bits.held >>= length;
            bits.count -= length;
            return Ok(entry >> 4);
        }
        // One bit at a time, which long codes are rare enough to get away with.
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for length in 1..=15usize {
            code |= bits.take(1)? as i32;
            let count = i32::from(self.counts[length]);
            if code - count < first {
                return self
                    .symbols
                    .get((index + code - first) as usize)
                    .copied()
                    .ok_or(INVALID);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(INVALID)
    }
}

/// Unpacks `pieces`, which together make a zlib stream, giving `sink` what comes out in order. It stops
/// as soon as `sink` says it has enough, which is how a file saying it's enormous gets noticed.
pub(crate) fn inflate(
    pieces: &[&[u8]],
    mut sink: impl FnMut(&[u8]) -> Result<bool, ImageError>,
) -> Result<(), ImageError> {
    let mut bits = Bits {
        pieces,
        piece: 0,
        at: 0,
        held: 0,
        count: 0,
    };
    let header = bits.take(16)?;
    let (first, second) = (header & 0xFF, header >> 8);
    if first & 0x0F != 8 || (first * 256 + second) % 31 != 0 || second & 0x20 != 0 {
        return Err(ImageError::Invalid(
            "its compressed data has the wrong header",
        ));
    }
    let mut out = Output {
        window: vec![0u8; WINDOW * 2],
        filled: 0,
        handed: 0,
    };
    let result = read_blocks(&mut bits, &mut out, &mut sink);
    if result.is_err() {
        // What came before the problem can still be shown.
        let _ = out.hand_over(&mut sink);
    }
    result
}

fn read_blocks(
    bits: &mut Bits<'_>,
    out: &mut Output,
    sink: &mut impl FnMut(&[u8]) -> Result<bool, ImageError>,
) -> Result<(), ImageError> {
    loop {
        let last = bits.take(1)? == 1;
        let codes = match bits.take(2)? {
            0 => {
                bits.align_to_byte();
                let length = bits.take(16)?;
                let check = bits.take(16)?;
                if length ^ check != 0xFFFF {
                    return Err(INVALID);
                }
                for _ in 0..length {
                    out.window[out.filled] = bits.take(8)? as u8;
                    out.filled += 1;
                    if out.is_full() && out.hand_over(sink)? {
                        return Ok(());
                    }
                }
                None
            }
            1 => {
                let mut lengths = [8u8; 288];
                lengths[144..256].fill(9);
                lengths[256..280].fill(7);
                Some((Codes::new(&lengths)?, Codes::new(&[5u8; 30])?))
            }
            2 => Some(read_code_lengths(bits)?),
            _ => return Err(INVALID),
        };
        if let Some((literals, distances)) = &codes {
            loop {
                let symbol = literals.read(bits)? as usize;
                if symbol < 256 {
                    out.window[out.filled] = symbol as u8;
                    out.filled += 1;
                } else if symbol == 256 {
                    break;
                } else {
                    let index = symbol - 257;
                    let (&base, &extra) = LENGTH_BASE
                        .get(index)
                        .zip(LENGTH_EXTRA.get(index))
                        .ok_or(INVALID)?;
                    let length = base as usize + bits.take(u32::from(extra))? as usize;
                    let index = distances.read(bits)? as usize;
                    let (&base, &extra) = DISTANCE_BASE
                        .get(index)
                        .zip(DISTANCE_EXTRA.get(index))
                        .ok_or(INVALID)?;
                    let distance = base as usize + bits.take(u32::from(extra))? as usize;
                    if distance > out.filled {
                        return Err(INVALID);
                    }
                    // Copied a byte at a time, as a repeat can overlap itself on purpose.
                    for _ in 0..length {
                        out.window[out.filled] = out.window[out.filled - distance];
                        out.filled += 1;
                    }
                }
                if out.is_full() && out.hand_over(sink)? {
                    return Ok(());
                }
            }
        }
        if last {
            out.hand_over(sink)?;
            return Ok(());
        }
    }
}

/// What's been unpacked. Repeats can refer back as far as `WINDOW`, so that much stays once it's been
/// handed out, and the rest of the room is for what comes next.
struct Output {
    window: Vec<u8>,
    filled: usize,
    /// How much of `window` has been handed out.
    handed: usize,
}

impl Output {
    /// Whether another repeat, which is at most 258 long, might not fit.
    #[inline]
    fn is_full(&self) -> bool {
        self.filled + 258 >= self.window.len()
    }

    /// Hands out what's new, and slides what repeats might need down to make room. Says whether the
    /// sink has had enough.
    fn hand_over(
        &mut self,
        sink: &mut impl FnMut(&[u8]) -> Result<bool, ImageError>,
    ) -> Result<bool, ImageError> {
        if sink(&self.window[self.handed..self.filled])? {
            return Ok(true);
        }
        if self.filled > WINDOW {
            self.window
                .copy_within(self.filled - WINDOW..self.filled, 0);
            self.filled = WINDOW;
        }
        self.handed = self.filled;
        Ok(false)
    }
}

fn read_code_lengths(bits: &mut Bits<'_>) -> Result<(Codes, Codes), ImageError> {
    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let literals = bits.take(5)? as usize + 257;
    let distances = bits.take(5)? as usize + 1;
    let code_lengths = bits.take(4)? as usize + 4;
    if literals > 286 || distances > 30 {
        return Err(INVALID);
    }
    let mut lengths = [0u8; 19];
    for &position in &ORDER[..code_lengths] {
        lengths[position] = bits.take(3)? as u8;
    }
    let codes = Codes::new(&lengths)?;
    let mut all = [0u8; 320];
    let total = literals + distances;
    let mut index = 0;
    while index < total {
        let symbol = codes.read(bits)?;
        let (value, repeat) = match symbol {
            0..=15 => (symbol as u8, 1),
            16 => {
                if index == 0 {
                    return Err(INVALID);
                }
                (all[index - 1], 3 + bits.take(2)? as usize)
            }
            17 => (0, 3 + bits.take(3)? as usize),
            _ => (0, 11 + bits.take(7)? as usize),
        };
        if index + repeat > total {
            return Err(INVALID);
        }
        all[index..index + repeat].fill(value);
        index += repeat;
    }
    Ok((
        Codes::new(&all[..literals])?,
        Codes::new(&all[literals..total])?,
    ))
}
