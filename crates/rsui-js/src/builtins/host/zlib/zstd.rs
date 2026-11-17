//! Zstandard, read the way RFC 8878 describes it, all of it: frames and the skippable kind, blocks stored as they are,
//! as one byte repeated, or squeezed, literals as they are or in a prefix code of one stream or four (or the code the
//! block before had), and sequences in FSE codes that are the usual ones, one symbol, made for the block, or the block
//! before's, with the last three offsets kept to be used again, and the frame's own sum of what it holds. A dictionary,
//! raw or with tables of its own, is what came before. A block is only read once all of it is there, so the pieces can
//! be split anywhere. Writing it is in `zstd_encode.rs`.

use super::huffman::Stop;

pub(super) use super::zstd_encode::Encoder;

pub(super) const MAGIC: u32 = 0xfd2f_b528;
pub(super) const DICTIONARY_MAGIC: u32 = 0xec30_a437;
/// The most a block can hold, before or after.
pub(super) const BLOCK_MAX: usize = 128 * 1024;

pub(super) const fn corrupt() -> Stop {
    Stop::Bad("ZSTD_error_corruption_detected", "Data corruption detected")
}

/// How many extra bits each literal length code and match length code has. Where each starts follows from them.
pub(super) const LL_BITS: [u8; 36] =
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 4, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
pub(super) const ML_BITS: [u8; 53] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3,
    3, 4, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
];

const fn bases<const N: usize>(bits: &[u8; N], first: u32) -> [u32; N] {
    let mut table = [first; N];
    let mut code = 1;
    while code < N {
        table[code] = table[code - 1] + (1 << bits[code - 1]);
        code += 1;
    }
    table
}

pub(super) const LL_BASE: [u32; 36] = bases(&LL_BITS, 0);
pub(super) const ML_BASE: [u32; 53] = bases(&ML_BITS, 3);

/// The usual distributions, from RFC 8878, with -1 for a symbol less likely than one in the table's size.
pub(super) const LL_DEFAULT: [i16; 36] =
    [4, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 2, 1, 1, 1, 1, 1, -1, -1, -1, -1];
pub(super) const ML_DEFAULT: [i16; 53] = [
    1, 4, 3, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1, -1, -1,
];
pub(super) const OF_DEFAULT: [i16; 29] =
    [1, 1, 1, 1, 1, 1, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1];

/// What each kind of sequence code is: its usual distribution and its accuracy, and the most a symbol and an
/// accuracy can be. Literal lengths, offsets and match lengths, in the order their tables come.
pub(super) const KINDS: [(&[i16], u32, usize, u32); 3] = [(&LL_DEFAULT, 6, 35, 9), (&OF_DEFAULT, 5, 31, 8), (&ML_DEFAULT, 6, 52, 9)];

/// Which symbol each state of an FSE table is, spread through it as both ends must agree: the least likely at the
/// end, and the rest a step apart.
pub(super) fn spread(counts: &[i16], log: u32) -> Result<Vec<u8>, Stop> {
    let size = 1usize << log;
    let mut symbols = vec![0u8; size];
    let mut high = size;
    for (symbol, count) in counts.iter().enumerate() {
        if *count == -1 {
            high = high.checked_sub(1).ok_or_else(corrupt)?;
            symbols[high] = symbol as u8;
        }
    }
    let step = (size >> 1) + (size >> 3) + 3;
    let mut at = 0;
    for (symbol, count) in counts.iter().enumerate() {
        for _ in 0..(*count).max(0) {
            symbols[at] = symbol as u8;
            at = (at + step) & (size - 1);
            while at >= high {
                at = (at + step) & (size - 1);
            }
        }
    }
    if at != 0 { Err(corrupt()) } else { Ok(symbols) }
}

/// An FSE code to read: for each state, its symbol, how many bits say the next state, and what they're added to.
/// One for a sequence's codes also has, for each state, what its symbol means, so that reading a sequence needs no
/// other tables: the lowest byte is how many bits say the next state, the next how many extra bits the symbol has,
/// then what the next state's bits are added to, and the top half what the extra bits are added to.
#[derive(Clone)]
pub(super) struct Fse {
    log: u32,
    states: Vec<u32>,
    meanings: Vec<u64>,
}

impl Fse {
    pub(super) fn new(counts: &[i16], log: u32) -> Result<Self, Stop> {
        let symbols = spread(counts, log)?;
        let mut next: Vec<u32> = counts.iter().map(|count| (*count).max(1) as u32).collect();
        let states = symbols
            .iter()
            .map(|symbol| {
                let x = next[usize::from(*symbol)];
                next[usize::from(*symbol)] += 1;
                let bits = log - (31 - x.leading_zeros());
                ((x << bits) - (1 << log)) << 16 | bits << 8 | u32::from(*symbol)
            })
            .collect();
        Ok(Self { log, states, meanings: Vec::new() })
    }

    fn one(symbol: u8) -> Self {
        Self { log: 0, states: vec![u32::from(symbol)], meanings: Vec::new() }
    }

    /// The same code, for literal lengths, offsets or match lengths (0, 1 or 2), with what each state means.
    fn meaning(mut self, kind: usize) -> Self {
        self.meanings = self
            .states
            .iter()
            .map(|entry| {
                let symbol = (entry & 255) as usize;
                let (base, extra) = match kind {
                    0 => (LL_BASE[symbol], u32::from(LL_BITS[symbol])),
                    1 => (1 << symbol, symbol as u32),
                    _ => (ML_BASE[symbol], u32::from(ML_BITS[symbol])),
                };
                u64::from(base) << 32 | u64::from(entry >> 16) << 16 | u64::from(extra) << 8 | u64::from((entry >> 8) & 255)
            })
            .collect();
        self
    }

    #[inline]
    fn symbol(&self, state: usize) -> u32 {
        self.states[state] & 255
    }

    #[inline]
    fn next(&self, state: usize, bits: &mut Backward) -> usize {
        let entry = self.states[state];
        (entry >> 16) as usize + bits.take((entry >> 8) & 255) as usize
    }
}

fn part(bytes: &[u8], from: usize, length: usize) -> Result<&[u8], Stop> {
    bytes.get(from..from.checked_add(length).ok_or_else(corrupt)?).ok_or_else(corrupt)
}

/// Reads how often each symbol comes, as an FSE table's description has it: the accuracy, then each count in as few
/// bits as could still be left for it, with runs of symbols that never come said together. Gives the counts, the
/// accuracy, and how many bytes it took.
pub(super) fn read_counts(bytes: &[u8], most_symbol: usize, most_log: u32) -> Result<(Vec<i16>, u32, usize), Stop> {
    let mut at = 0u32;
    let peek = |at: u32, width: u32| {
        let from = (at / 8) as usize;
        let mut word = 0u32;
        for (place, byte) in bytes.iter().skip(from).take(3).enumerate() {
            word |= u32::from(*byte) << (8 * place);
        }
        (word >> (at % 8)) & ((1 << width) - 1)
    };
    let log = peek(0, 4) + 5;
    at += 4;
    if log > most_log {
        return Err(corrupt());
    }
    let mut remaining = 1i32 << log;
    let mut counts = Vec::with_capacity(most_symbol + 1);
    while remaining > 0 {
        if counts.len() > most_symbol {
            return Err(corrupt());
        }
        let width = 32 - (remaining as u32 + 1).leading_zeros();
        let lower = (1 << (width - 1)) - 1;
        let threshold = (1 << width) - 1 - (remaining as u32 + 1);
        let mut value = peek(at, width);
        if value & lower < threshold {
            value &= lower;
            at += width - 1;
        } else {
            if value > lower {
                value -= threshold;
            }
            at += width;
        }
        let count = value as i32 - 1;
        remaining -= count.abs();
        counts.push(count as i16);
        if count == 0 {
            loop {
                let repeat = peek(at, 2);
                at += 2;
                counts.extend(std::iter::repeat_n(0, repeat as usize));
                if repeat != 3 || counts.len() > most_symbol {
                    break;
                }
            }
        }
    }
    let used = at.div_ceil(8) as usize;
    if remaining != 0 || counts.len() > most_symbol + 1 || used > bytes.len() {
        return Err(corrupt());
    }
    Ok((counts, log, used))
}

/// Reads bits from the end of a stream back to its start, the way the stream was written, and which starts after
/// the highest bit of its last byte, which is set. It holds the eight bytes that end at `end`, of which `used` bits
/// from the top have been read, and moves back by whole bytes when it's told to `reload`, so most reads are a shift.
/// What comes before the stream reads as zeros, and reading it makes `left` less than nothing.
pub(super) struct Backward<'a> {
    bytes: &'a [u8],
    held: u64,
    end: usize,
    used: u32,
}

impl<'a> Backward<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Result<Self, Stop> {
        match bytes.last() {
            Some(last) if *last != 0 => Ok(Self { bytes, held: Self::word(bytes, bytes.len()), end: bytes.len(), used: last.leading_zeros() + 1 }),
            _ => Err(corrupt()),
        }
    }

    /// The eight bytes that end at `end`, with zeros for those before the start.
    #[inline]
    fn word(bytes: &[u8], end: usize) -> u64 {
        match bytes.get(end.wrapping_sub(8)..end) {
            Some(eight) => u64::from_le_bytes(eight.try_into().unwrap_or_default()),
            None => bytes[..end].iter().rev().fold(0, |word, byte| word << 8 | u64::from(*byte)) << (8 * (8 - end) as u32 % 64),
        }
    }

    /// Moves back past the whole bytes that have been read, so that at least 57 bits can be, unless the start is
    /// near.
    #[inline]
    pub(super) fn reload(&mut self) {
        let back = (self.used >> 3) as usize;
        match self.bytes.get(self.end.wrapping_sub(back + 8)..self.end.wrapping_sub(back)) {
            Some(eight) if back <= self.end => {
                self.end -= back;
                self.used &= 7;
                self.held = u64::from_le_bytes(eight.try_into().unwrap_or_default());
            }
            _ => self.reload_near_start(),
        }
    }

    #[cold]
    #[inline(never)]
    fn reload_near_start(&mut self) {
        let back = ((self.used >> 3) as usize).min(self.end);
        self.end -= back;
        self.used -= 8 * back as u32;
        self.held = Self::word(self.bytes, self.end);
    }

    /// How many bits are left to read.
    #[inline]
    pub(super) fn left(&self) -> isize {
        8 * self.end as isize - self.used as isize
    }

    /// The next `wanted` bits, from 1 to 64 less than were read since the last `reload`, without reading them.
    #[inline]
    fn peek(&self, wanted: u32) -> u64 {
        self.held.checked_shl(self.used).unwrap_or(0) >> (64 - wanted)
    }

    /// Reads the next `wanted` bits, up to 32, with no more than 25 read since the last `reload`.
    #[inline]
    fn read(&mut self, wanted: u32) -> u32 {
        let value = (self.held.checked_shl(self.used).unwrap_or(0) >> 1 >> (63 - wanted)) as u32;
        self.used += wanted;
        value
    }

    /// Reads the next `wanted` bits, up to 32.
    #[inline]
    pub(super) fn take(&mut self, wanted: u32) -> u32 {
        if self.used + wanted > 64 {
            self.reload();
        }
        self.read(wanted)
    }

    fn overflowed(&self) -> bool {
        self.left() < 0
    }
}

/// A Huffman code to read: a table for the next `bits` bits, of each symbol and how many of them it took.
#[derive(Clone)]
pub(super) struct Huffman {
    bits: u32,
    table: Vec<u16>,
}

/// Reads a Huffman tree's description: the weight of each symbol but the last, four bits each or squeezed with FSE,
/// and the last weight is what's left. Gives the code and how many bytes it took.
pub(super) fn read_huffman(bytes: &[u8]) -> Result<(Huffman, usize), Stop> {
    let header = usize::from(*bytes.first().ok_or_else(corrupt)?);
    let mut weights: Vec<u8> = Vec::with_capacity(256);
    let used = if header < 128 {
        let body = part(bytes, 1, header)?;
        let (counts, log, taken) = read_counts(body, 12, 6)?;
        let table = Fse::new(&counts, log)?;
        let mut stream = Backward::new(&body[taken..])?;
        let mut states = [stream.take(log) as usize, stream.take(log) as usize];
        // The two states take turns, until moving one on would need more bits than are left, and then the other's
        // symbol is the last.
        'reading: loop {
            for turn in 0..2 {
                weights.push(table.symbol(states[turn]) as u8);
                states[turn] = table.next(states[turn], &mut stream);
                if stream.overflowed() {
                    weights.push(table.symbol(states[1 - turn]) as u8);
                    break 'reading;
                }
            }
            if weights.len() > 255 {
                return Err(corrupt());
            }
        }
        1 + header
    } else {
        let count = header - 127;
        let body = part(bytes, 1, count.div_ceil(2))?;
        weights.extend((0..count).map(|at| if at % 2 == 0 { body[at / 2] >> 4 } else { body[at / 2] & 15 }));
        1 + count.div_ceil(2)
    };
    let total: u32 = weights.iter().map(|weight| (1u32 << weight) >> 1).sum();
    if weights.len() > 255 || weights.iter().any(|weight| *weight > 12) || total == 0 {
        return Err(corrupt());
    }
    let bits = 32 - total.leading_zeros();
    let left = (1 << bits) - total;
    if bits > 12 || !left.is_power_of_two() {
        return Err(corrupt());
    }
    weights.push(left.trailing_zeros() as u8 + 1);
    // The lightest come first, each taking as many places as its weight says.
    let mut table = Vec::with_capacity(1 << bits);
    for weight in 1..=bits as u8 {
        for (symbol, _) in weights.iter().enumerate().filter(|(_, each)| **each == weight) {
            let entry = (symbol as u16) << 8 | (bits as u16 + 1 - u16::from(weight));
            table.extend(std::iter::repeat_n(entry, 1 << (weight - 1)));
        }
    }
    Ok((Huffman { bits, table }, used))
}

/// Reads a stream of literals, as many as there's room for in `out`.
fn huffman_stream(code: &Huffman, bytes: &[u8], out: &mut [u8]) -> Result<(), Stop> {
    huffman_streams(code, [bytes], [out])
}

/// Reads streams of literals side by side, which the processor can do at once as none waits on another, four from each
/// after moving each back: as a code takes at most 11 bits, that's 44 of the 57 there are then. The rest of each is
/// read a literal at a time.
fn huffman_streams<const N: usize>(code: &Huffman, parts: [&[u8]; N], mut outs: [&mut [u8]; N]) -> Result<(), Stop> {
    let mut streams: [Backward; N] = {
        let mut made = [const { None }; N];
        for (slot, part) in made.iter_mut().zip(parts) {
            *slot = Some(Backward::new(part)?);
        }
        made.map(|stream| stream.unwrap_or(Backward { bytes: &[], held: 0, end: 0, used: 0 }))
    };
    let (bits, table) = (code.bits, &code.table[..]);
    let shortest = outs.iter().map(|out| out.len()).min().unwrap_or(0);
    let mut at = 0;
    while at + 4 <= shortest {
        for stream in &mut streams {
            stream.reload();
        }
        for step in at..at + 4 {
            for (stream, out) in streams.iter_mut().zip(outs.iter_mut()) {
                let entry = table[stream.peek(bits) as usize];
                out[step] = (entry >> 8) as u8;
                stream.used += u32::from(entry as u8);
            }
        }
        at += 4;
    }
    for (stream, out) in streams.iter_mut().zip(outs.iter_mut()) {
        for slot in &mut out[at..] {
            if stream.used + 11 > 64 {
                stream.reload();
            }
            let entry = table[stream.peek(bits) as usize];
            *slot = (entry >> 8) as u8;
            stream.used += u32::from(entry as u8);
        }
        if stream.left() != 0 {
            return Err(corrupt());
        }
    }
    Ok(())
}

/// The offset a sequence's offset value stands for, given the last three, which it changes as RFC 8878 says. Values
/// of three or less are one of those, and which depends on whether there are literals before it.
#[inline]
pub(super) fn resolve(last: &mut [u32; 3], value: u32, literals: u32) -> u32 {
    if value > 3 {
        *last = [value - 3, last[0], last[1]];
        return last[0];
    }
    let which = value - 1 + u32::from(literals == 0);
    if which == 0 {
        return last[0];
    }
    let offset = if which == 3 { last[0].wrapping_sub(1) } else { last[which as usize] };
    if which != 1 {
        last[2] = last[1];
    }
    last[1] = last[0];
    last[0] = offset;
    offset
}

/// What a block can take from the one before: its Huffman code, its three FSE codes, and the last offsets. A
/// dictionary with tables starts a frame with them.
#[derive(Clone)]
pub(super) struct Carried {
    huffman: Option<Huffman>,
    /// The usual codes are shared rather than copied, as a block of a small frame takes about as long as copying one.
    codes: [Option<std::borrow::Cow<'static, Fse>>; 3],
    pub(super) last: [u32; 3],
}

impl Carried {
    fn new() -> Self {
        Self { huffman: None, codes: [None, None, None], last: [1, 4, 8] }
    }
}

pub(super) struct Dictionary {
    pub(super) id: u32,
    pub(super) content: Vec<u8>,
    pub(super) carried: Carried,
}

/// Reads a dictionary: one with tables starts with its magic number, and anything else is all content.
pub(super) fn read_dictionary(bytes: Vec<u8>) -> Result<Dictionary, Stop> {
    let word = |at: usize| part(&bytes, at, 4).map(|four| u32::from_le_bytes(four.try_into().unwrap_or_default()));
    if bytes.len() < 8 || word(0)? != DICTIONARY_MAGIC {
        return Ok(Dictionary { id: 0, content: bytes, carried: Carried::new() });
    }
    let mut carried = Carried::new();
    let (huffman, mut at) = read_huffman(&bytes[8..])?;
    carried.huffman = Some(huffman);
    at += 8;
    // Offsets, match lengths and literal lengths, in that order.
    for kind in [1, 2, 0] {
        let (_, _, most_symbol, most_log) = KINDS[kind];
        let (counts, log, used) = read_counts(bytes.get(at..).ok_or_else(corrupt)?, most_symbol, most_log)?;
        carried.codes[kind] = Some(std::borrow::Cow::Owned(Fse::new(&counts, log)?.meaning(kind)));
        at += used;
    }
    for place in 0..3 {
        carried.last[place] = word(at + 4 * place)?;
    }
    let content = bytes[at + 12..].to_vec();
    if carried.last.iter().any(|last| *last == 0 || *last as usize > content.len()) {
        return Err(corrupt());
    }
    Ok(Dictionary { id: word(4)?, content, carried })
}

const PRIME: [u64; 5] =
    [0x9e37_79b1_85eb_ca87, 0xc2b2_ae3d_27d4_eb4f, 0x1656_67b1_9e37_79f9, 0x85eb_ca77_c2b2_ae63, 0x27d4_eb2f_1656_67c5];

/// XXH64, with no seed, which a frame's sum is the low 32 bits of. It takes what it sums a piece at a time.
#[derive(Clone)]
pub(super) struct Xxh64 {
    lanes: [u64; 4],
    held: [u8; 32],
    count: usize,
    total: u64,
}

fn round(lane: u64, input: u64) -> u64 {
    lane.wrapping_add(input.wrapping_mul(PRIME[1])).rotate_left(31).wrapping_mul(PRIME[0])
}

impl Xxh64 {
    pub(super) fn new() -> Self {
        let [one, two, ..] = PRIME;
        Self { lanes: [one.wrapping_add(two), two, 0, one.wrapping_neg()], held: [0; 32], count: 0, total: 0 }
    }

    fn stripe(&mut self, stripe: &[u8]) {
        for (lane, word) in self.lanes.iter_mut().zip(stripe.chunks_exact(8)) {
            *lane = round(*lane, u64::from_le_bytes(word.try_into().unwrap_or_default()));
        }
    }

    pub(super) fn update(&mut self, mut bytes: &[u8]) {
        self.total += bytes.len() as u64;
        if self.count > 0 {
            let take = (32 - self.count).min(bytes.len());
            self.held[self.count..self.count + take].copy_from_slice(&bytes[..take]);
            self.count += take;
            bytes = &bytes[take..];
            if self.count < 32 {
                return;
            }
            let held = self.held;
            self.stripe(&held);
            self.count = 0;
        }
        let mut stripes = bytes.chunks_exact(32);
        for stripe in &mut stripes {
            self.stripe(stripe);
        }
        let rest = stripes.remainder();
        self.held[..rest.len()].copy_from_slice(rest);
        self.count = rest.len();
    }

    pub(super) fn sum(&self) -> u32 {
        let [one, two, three, four, five] = PRIME;
        let mut hash = if self.total >= 32 {
            let [a, b, c, d] = self.lanes;
            let mut hash = a.rotate_left(1).wrapping_add(b.rotate_left(7)).wrapping_add(c.rotate_left(12)).wrapping_add(d.rotate_left(18));
            for lane in self.lanes {
                hash = (hash ^ round(0, lane)).wrapping_mul(one).wrapping_add(four);
            }
            hash
        } else {
            five
        };
        hash = hash.wrapping_add(self.total);
        let mut rest = &self.held[..self.count];
        while rest.len() >= 8 {
            let word = u64::from_le_bytes(rest[..8].try_into().unwrap_or_default());
            hash = (hash ^ round(0, word)).rotate_left(27).wrapping_mul(one).wrapping_add(four);
            rest = &rest[8..];
        }
        if rest.len() >= 4 {
            let word = u64::from(u32::from_le_bytes(rest[..4].try_into().unwrap_or_default()));
            hash = (hash ^ word.wrapping_mul(one)).rotate_left(23).wrapping_mul(two).wrapping_add(three);
            rest = &rest[4..];
        }
        for byte in rest {
            hash = (hash ^ u64::from(*byte).wrapping_mul(five)).rotate_left(11).wrapping_mul(one);
        }
        hash ^= hash >> 33;
        hash = hash.wrapping_mul(two);
        hash ^= hash >> 29;
        hash = hash.wrapping_mul(three);
        hash ^= hash >> 32;
        hash as u32
    }
}

enum State {
    Magic,
    Header,
    Skip(u64),
    Block,
    Sum,
    Done,
}

pub(super) struct Decoder {
    input: Vec<u8>,
    at: usize,
    state: State,
    /// What's come out: as much as can be referred back to, and what's yet to be given out.
    window: Vec<u8>,
    sent: usize,
    window_size: usize,
    most_window: u64,
    /// The frame's size, if it says, and how much of it there's been.
    size: Option<u64>,
    made: u64,
    sum: Option<Xxh64>,
    carried: Carried,
    dictionary: Option<Dictionary>,
    literals: Vec<u8>,
}

impl Decoder {
    /// A decoder with this dictionary, if any, which won't take a window bigger than `window_log` says. A dictionary
    /// with tables that are wrong can't be used.
    pub(super) fn new(dictionary: Option<Vec<u8>>, window_log: u32) -> Result<Self, Stop> {
        let dictionary = dictionary.map(read_dictionary).transpose()?;
        Ok(Self {
            input: Vec::new(),
            at: 0,
            state: State::Magic,
            window: Vec::new(),
            sent: 0,
            window_size: 0,
            most_window: 1 << if window_log == 0 { 27 } else { window_log },
            size: None,
            made: 0,
            sum: None,
            carried: Carried::new(),
            dictionary,
            literals: Vec::new(),
        })
    }

    pub(super) fn ended(&self) -> bool {
        matches!(self.state, State::Done)
    }

    /// Reads what's come, giving out what that undoes. As in Node, what's cut short isn't wrong, and a frame that
    /// ends with more after it in the same piece is the end of everything, but another frame can start a new piece.
    pub(super) fn push(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<(), Stop> {
        if !self.ended() {
            self.input.extend_from_slice(input);
        }
        let went = self.run();
        if self.sent == 0 && out.is_empty() && matches!(self.state, State::Magic | State::Done) {
            // A whole frame, with nothing before it to keep, is handed over rather than copied, as it is when
            // everything comes at once.
            *out = std::mem::take(&mut self.window);
        } else {
            out.extend_from_slice(&self.window[self.sent..]);
        }
        self.sent = self.window.len();
        let reach = self.dictionary.as_ref().filter(|_| self.made <= self.window_size as u64).map_or(0, |found| found.content.len());
        let keep = self.window_size + reach;
        if self.window.len() > 2 * keep + BLOCK_MAX {
            let drop = self.window.len() - keep;
            self.window.drain(..drop);
            self.sent -= drop;
        }
        self.input.drain(..self.at);
        self.at = 0;
        match went {
            Err(Stop::Short) => Ok(()),
            went => went,
        }
    }

    /// The next `count` bytes of what's come, if they're there, which are then read.
    fn take(&mut self, count: usize) -> Result<&[u8], Stop> {
        let from = self.at;
        let bytes = self.input.get(from..from + count).ok_or(Stop::Short)?;
        self.at += count;
        Ok(bytes)
    }

    fn run(&mut self) -> Result<(), Stop> {
        loop {
            match self.state {
                State::Done => {
                    self.at = self.input.len();
                    return Ok(());
                }
                State::Magic => {
                    let magic = self.input.get(self.at..self.at + 4).ok_or(Stop::Short)?;
                    let magic = u32::from_le_bytes(magic.try_into().unwrap_or_default());
                    if magic == MAGIC {
                        self.at += 4;
                        self.state = State::Header;
                    } else if magic & !15 == 0x184d_2a50 {
                        let size = self.input.get(self.at + 4..self.at + 8).ok_or(Stop::Short)?;
                        self.state = State::Skip(u64::from(u32::from_le_bytes(size.try_into().unwrap_or_default())));
                        self.at += 8;
                    } else {
                        return Err(Stop::Bad("ZSTD_error_prefix_unknown", "Unknown frame descriptor"));
                    }
                }
                State::Header => self.header()?,
                State::Skip(left) => {
                    let got = (self.input.len() - self.at).min(left.min(usize::MAX as u64) as usize);
                    self.at += got;
                    if got as u64 != left {
                        self.state = State::Skip(left - got as u64);
                        return Err(Stop::Short);
                    }
                    self.end_frame();
                }
                State::Block => self.block()?,
                State::Sum => {
                    let wanted = self.sum.as_ref().map_or(0, Xxh64::sum);
                    let found = self.take(4)?;
                    if u32::from_le_bytes(found.try_into().unwrap_or_default()) != wanted {
                        return Err(Stop::Bad("ZSTD_error_checksum_wrong", "Restored data doesn't match checksum"));
                    }
                    self.end_frame();
                }
            }
        }
    }

    fn end_frame(&mut self) {
        self.state = if self.at < self.input.len() { State::Done } else { State::Magic };
    }

    fn header(&mut self) -> Result<(), Stop> {
        let descriptor = *self.input.get(self.at).ok_or(Stop::Short)?;
        let size_code = descriptor >> 6;
        let single = descriptor & 32 != 0;
        let id_size = [0, 1, 2, 4][usize::from(descriptor & 3)];
        let size_size = [usize::from(single), 2, 4, 8][usize::from(size_code)];
        let header = self.take(1 + usize::from(!single) + id_size + size_size)?.to_vec();
        if descriptor & 8 != 0 {
            return Err(Stop::Bad("ZSTD_error_frameParameter_unsupported", "Unsupported frame parameter"));
        }
        let number = |from: usize, size: usize| header[from..from + size].iter().rev().fold(0u64, |sum, byte| sum << 8 | u64::from(*byte));
        let mut window = 0;
        if !single {
            let exponent = u32::from(header[1] >> 3) + 10;
            window = (1u64 << exponent) + ((1u64 << exponent) >> 3) * u64::from(header[1] & 7);
        }
        let id = number(1 + usize::from(!single), id_size) as u32;
        self.size = (single || size_code > 0).then(|| number(header.len() - size_size, size_size) + if size_code == 1 { 256 } else { 0 });
        if single {
            window = self.size.unwrap_or(0);
        }
        if window.max(1024) > self.most_window {
            return Err(Stop::Bad("ZSTD_error_frameParameter_windowTooLarge", "Frame requires too much memory for decoding"));
        }
        if id != 0 && self.dictionary.as_ref().is_none_or(|found| found.id != id) {
            return Err(Stop::Bad("ZSTD_error_dictionary_wrong", "Dictionary mismatch"));
        }
        self.window_size = window as usize;
        self.made = 0;
        self.sum = (descriptor & 4 != 0).then(Xxh64::new);
        // A frame can't refer to the one before, only to the dictionary, and all before it has been given out.
        self.window.clear();
        self.carried = Carried::new();
        if let Some(found) = &self.dictionary {
            self.window.extend_from_slice(&found.content);
            self.carried = found.carried.clone();
        }
        // Room for what the frame says it holds, so it isn't moved as it grows, up to what any sensible frame that's
        // read a piece at a time would need anyway, and for what each block makes room for past its end. What's
        // handed over keeps its room, so there's no more than that.
        if let Some(size) = self.size {
            self.window.reserve(size.min(32 << 20) as usize + SLACK);
        }
        self.sent = self.window.len();
        self.state = State::Block;
        Ok(())
    }

    fn block(&mut self) -> Result<(), Stop> {
        let header = self.input.get(self.at..self.at + 3).ok_or(Stop::Short)?;
        let header = u32::from(header[0]) | u32::from(header[1]) << 8 | u32::from(header[2]) << 16;
        let (last, kind, size) = (header & 1 != 0, (header >> 1) & 3, (header >> 3) as usize);
        let most = self.window_size.clamp(1, BLOCK_MAX);
        if kind == 3 || size > most {
            return Err(corrupt());
        }
        let body = self.input.get(self.at + 3..self.at + 3 + if kind == 1 { 1 } else { size }).ok_or(Stop::Short)?;
        // No block can make more than what's left of a frame that says how big it is.
        let most = self.size.map_or(most, |size| most.min(size.saturating_sub(self.made).min(BLOCK_MAX as u64) as usize));
        let from = self.window.len();
        match kind {
            0 => self.window.extend_from_slice(body),
            1 => self.window.resize(from + size, body[0]),
            _ => {
                let input = std::mem::take(&mut self.input);
                let went = self.compressed(&input[self.at + 3..self.at + 3 + size], most);
                self.input = input;
                went?;
            }
        }
        self.at += 3 + if kind == 1 { 1 } else { size };
        self.made += (self.window.len() - from) as u64;
        if let Some(sum) = &mut self.sum {
            sum.update(&self.window[from..]);
        }
        if self.size.is_some_and(|size| self.made > size || (last && self.made != size)) {
            return Err(corrupt());
        }
        if last {
            if self.sum.is_some() {
                self.state = State::Sum;
            } else {
                self.end_frame();
            }
        }
        Ok(())
    }

    fn compressed(&mut self, block: &[u8], most: usize) -> Result<(), Stop> {
        let (used, size) = self.read_literals(block)?;
        let body = &block[used..];
        let start = self.window.len();
        let first = *body.first().ok_or_else(corrupt)?;
        let (count, mut at) = match first {
            0..128 => (usize::from(first), 1),
            128..=254 => ((usize::from(first) - 128) << 8 | usize::from(*body.get(1).ok_or_else(corrupt)?), 2),
            _ => (usize::from(u16::from_le_bytes(part(body, 1, 2)?.try_into().unwrap_or_default())) + 0x7f00, 3),
        };
        let mut taken = 0;
        if count > 0 {
            let modes = *body.get(at).ok_or_else(corrupt)?;
            at += 1;
            if modes & 3 != 0 {
                return Err(corrupt());
            }
            for (kind, (_, _, most_symbol, most_log)) in KINDS.into_iter().enumerate() {
                let slot = &mut self.carried.codes[kind];
                match (modes >> (6 - 2 * kind)) & 3 {
                    0 => *slot = Some(std::borrow::Cow::Borrowed(&usual_codes()[kind])),
                    1 => {
                        let symbol = *body.get(at).ok_or_else(corrupt)?;
                        if usize::from(symbol) > most_symbol {
                            return Err(corrupt());
                        }
                        *slot = Some(std::borrow::Cow::Owned(Fse::one(symbol).meaning(kind)));
                        at += 1;
                    }
                    2 => {
                        let (counts, log, used) = read_counts(&body[at..], most_symbol, most_log)?;
                        *slot = Some(std::borrow::Cow::Owned(Fse::new(&counts, log)?.meaning(kind)));
                        at += used;
                    }
                    _ if slot.is_none() => return Err(corrupt()),
                    _ => {}
                }
            }
            let [Some(ll), Some(of), Some(ml)] = &self.carried.codes else {
                return Err(corrupt());
            };
            let bits = Backward::new(body.get(at..).ok_or_else(corrupt)?)?;
            let literals = Literals { bytes: &self.literals, size };
            taken = sequences(bits, [ll.as_ref(), of.as_ref(), ml.as_ref()], count, &mut self.carried.last, literals, &mut self.window, most)?;
        } else if at != body.len() {
            return Err(corrupt());
        }
        self.window.extend_from_slice(&self.literals[taken..size]);
        if self.window.len() - start > most { Err(corrupt()) } else { Ok(()) }
    }

    /// Reads a block's literals into `literals`, followed by `SLACK` more bytes that are there to be read past the
    /// end, giving how many bytes they took and how many literals there are.
    fn read_literals(&mut self, block: &[u8]) -> Result<(usize, usize), Stop> {
        let first = *block.first().ok_or_else(corrupt)?;
        let (kind, format) = (first & 3, (first >> 2) & 3);
        let header_size = match (kind, format) {
            (0 | 1, 0 | 2) => 1,
            (0 | 1, 1) | (2 | 3, 0 | 1) => 2 + usize::from(kind > 1),
            (0 | 1, _) => 3,
            _ => 2 + usize::from(format),
        };
        let header = part(block, 0, header_size)?.iter().rev().fold(0u64, |sum, byte| sum << 8 | u64::from(*byte));
        let literals = &mut self.literals;
        if kind < 2 {
            let size = (header >> if header_size == 1 { 3 } else { 4 }) as usize;
            literals.clear();
            if kind == 0 {
                literals.extend_from_slice(part(block, header_size, size)?);
                literals.resize(size + SLACK, 0);
                return Ok((header_size + size, size));
            }
            literals.resize(size, *block.get(header_size).ok_or_else(corrupt)?);
            literals.resize(size + SLACK, 0);
            return Ok((header_size + 1, size));
        }
        let width = [10, 10, 14, 18][usize::from(format)];
        let size = (header >> 4) as usize & ((1 << width) - 1);
        let packed = (header >> (4 + width)) as usize & ((1 << width) - 1);
        let body = part(block, header_size, packed)?;
        if size > BLOCK_MAX {
            return Err(corrupt());
        }
        let mut at = 0;
        if kind == 2 {
            let (code, used) = read_huffman(body)?;
            self.carried.huffman = Some(code);
            at = used;
        }
        let code = self.carried.huffman.as_ref().ok_or_else(corrupt)?;
        let streams = &body.get(at..).ok_or_else(corrupt)?;
        // What was there before is written over, so only what's added has to be filled in.
        literals.resize(size + SLACK, 0);
        if format == 0 {
            huffman_stream(code, streams, &mut literals[..size])?;
        } else {
            let jumps = part(streams, 0, 6)?;
            let sizes: [usize; 3] = std::array::from_fn(|at| usize::from(u16::from_le_bytes([jumps[2 * at], jumps[2 * at + 1]])));
            let segment = size.div_ceil(4);
            if 3 * segment > size || sizes.iter().sum::<usize>() + 6 > streams.len() {
                return Err(corrupt());
            }
            let (one, rest) = streams[6..].split_at(sizes[0]);
            let (two, rest) = rest.split_at(sizes[1]);
            let (three, four) = rest.split_at(sizes[2]);
            let (first, rest) = literals[..size].split_at_mut(segment);
            let (second, rest) = rest.split_at_mut(segment);
            let (third, fourth) = rest.split_at_mut(segment);
            huffman_streams(code, [one, two, three, four], [first, second, third, fourth])?;
        }
        Ok((header_size + packed, size))
    }
}

/// How many bytes past the last literal can be read, so that literals can be copied sixteen at a time.
const SLACK: usize = 32;

/// A block's literals, with `SLACK` bytes after the `size` there are.
struct Literals<'a> {
    bytes: &'a [u8],
    size: usize,
}

/// The usual FSE codes, made once.
fn usual_codes() -> &'static [Fse; 3] {
    static USUAL: std::sync::OnceLock<[Fse; 3]> = std::sync::OnceLock::new();
    USUAL.get_or_init(|| {
        let mut kind = 0;
        KINDS.map(|(counts, log, _, _)| {
            kind += 1;
            Fse::new(counts, log).unwrap_or_else(|_| Fse::one(0)).meaning(kind - 1)
        })
    })
}

/// Reads a block's sequences and does what each says: copies its literals, then a match from what's come out,
/// changing the last offsets as it goes. Gives how many literals were used, and the rest come after.
///
/// What comes out is written into `window`'s spare capacity, which is made room for once: the block makes no more
/// than `most`, which each sequence is checked against first. Literals are copied sixteen bytes at a time, into what
/// comes after them, and so are matches from far enough back, as each sixteen read have been written already.
#[allow(unsafe_code)]
fn sequences(
    mut bits: Backward,
    [ll, of, ml]: [&Fse; 3],
    count: usize,
    last: &mut [u32; 3],
    literals: Literals,
    window: &mut Vec<u8>,
    most: usize,
) -> Result<usize, Stop> {
    // Each table has a power of two states, which every state is less than, so masking one costs no check.
    let (ll_states, of_states, ml_states) = (&ll.meanings[..], &of.meanings[..], &ml.meanings[..]);
    if [ll_states, of_states, ml_states].iter().any(|states| !states.len().is_power_of_two()) {
        return Err(corrupt());
    }
    let (ll_mask, of_mask, ml_mask) = (ll_states.len() - 1, of_states.len() - 1, ml_states.len() - 1);
    let mut ll_state = bits.take(ll.log) as usize;
    let mut of_state = bits.take(of.log) as usize;
    let mut ml_state = bits.take(ml.log) as usize;
    let Literals { bytes: literals, size } = literals;
    assert!(literals.len() >= size + SLACK);
    let start = window.len();
    window.reserve(most + SLACK);
    let room = window.as_mut_ptr();
    let mut out = start;
    let mut taken = 0;
    let mut wrong = false;
    for left in (0..count).rev() {
        let (ll_entry, of_entry, ml_entry) = (ll_states[ll_state & ll_mask], of_states[of_state & of_mask], ml_states[ml_state & ml_mask]);
        let extra = |entry: u64| (entry >> 8) as u32 & 255;
        let next = |entry: u64| (entry >> 16) as usize & 0xffff;
        // An offset takes at most 31 bits, then a length and a run 16 each, and the states 26 in all. Moving back
        // leaves at least 57, which is usually enough for all of them, so it's only done again when it might not be.
        bits.reload();
        let value = (of_entry >> 32) as u32 + bits.read(extra(of_entry));
        if bits.used > 64 - 32 {
            bits.reload();
        }
        let length = ((ml_entry >> 32) as u32 + bits.read(extra(ml_entry))) as usize;
        let run = (ll_entry >> 32) as u32 + bits.read(extra(ll_entry));
        if left > 0 {
            if bits.used > 64 - 26 {
                bits.reload();
            }
            ll_state = next(ll_entry) + bits.read(ll_entry as u32 & 255) as usize;
            ml_state = next(ml_entry) + bits.read(ml_entry as u32 & 255) as usize;
            of_state = next(of_entry) + bits.read(of_entry as u32 & 255) as usize;
        }
        let offset = resolve(last, value, run) as usize;
        let run = run as usize;
        if run > size - taken || out + run + length - start > most {
            wrong = true;
            break;
        }
        // SAFETY: the literals read are below `size + SLACK`, the length of `literals`, as `run` is no more than
        // there are left and sixteen at a time reads 15 past it at most. What's written is below `start + most +
        // SLACK`, as `out + run + length` is no more than `start + most`, and that's within the capacity.
        unsafe {
            let mut at = 0;
            while at < run {
                std::ptr::copy_nonoverlapping(literals.as_ptr().add(taken + at), room.add(out + at), 16);
                at += 16;
            }
        }
        taken += run;
        out += run;
        if offset == 0 || offset > out {
            wrong = true;
            break;
        }
        let end = out + length;
        let mut from = out - offset;
        // SAFETY: as above for what's written. What's read is below `out` when it's read, so it's been written:
        // sixteen or eight at a time read no further than where those are written start, when the match is from
        // that far back.
        unsafe {
            if offset >= 16 {
                while out < end {
                    std::ptr::copy_nonoverlapping(room.add(from), room.add(out), 16);
                    from += 16;
                    out += 16;
                }
            } else if offset >= 8 {
                while out < end {
                    std::ptr::copy_nonoverlapping(room.add(from), room.add(out), 8);
                    from += 8;
                    out += 8;
                }
            } else {
                while out < end {
                    room.add(out).write(*room.add(from));
                    from += 1;
                    out += 1;
                }
            }
        }
        out = end;
    }
    // SAFETY: everything from `start` to `out` has been written, and it's within the capacity.
    unsafe { window.set_len(out) };
    if wrong || bits.left() != 0 {
        return Err(corrupt());
    }
    Ok(taken)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[test]
    fn sums_are_xxh64s() {
        let sum = |bytes: &[u8]| {
            let mut hasher = Xxh64::new();
            hasher.update(bytes);
            hasher.sum()
        };
        // The low half of XXH64 with no seed, as the reference has it.
        assert_eq!(sum(b""), 0x51d8_e999);
        assert_eq!(sum(b"a"), 0xa98c_6e5b);
        assert_eq!(sum(b"hello"), 0x889f_6da3);
        let long: Vec<u8> = (0..1000u32).map(|n| (n * 7) as u8).collect();
        let mut pieces = Xxh64::new();
        for piece in long.chunks(13) {
            pieces.update(piece);
        }
        assert_eq!(pieces.sum(), sum(&long));
    }

    #[test]
    fn offsets_are_used_again_as_the_rfc_says() {
        let mut last = [1, 4, 8];
        assert_eq!(resolve(&mut last, 13, 5), 10);
        assert_eq!(last, [10, 1, 4]);
        assert_eq!(resolve(&mut last, 2, 5), 1);
        assert_eq!(last, [1, 10, 4]);
        assert_eq!(resolve(&mut last, 3, 5), 4);
        assert_eq!(last, [4, 1, 10]);
        assert_eq!(resolve(&mut last, 1, 0), 1);
        assert_eq!(last, [1, 4, 10]);
        assert_eq!(resolve(&mut last, 3, 0), 0);
    }
}
