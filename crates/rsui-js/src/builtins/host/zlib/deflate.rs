//! DEFLATE written the way zlib writes it, so that what comes out is about as small, at about the speed, for each
//! level: earlier copies are found through chains of where each four bytes were seen before, a match can wait a byte
//! to see if a longer one starts there (above level 3), and each block is written with codes made for it, the fixed
//! codes, or as it is, whichever is smallest. It takes input a piece at a time and gives out what it can.

use super::Wrap;
use super::huffman::{BitWriter, code_lengths, codes_of};
use super::inflate::{DISTANCE_BASE, DISTANCE_EXTRA, LENGTH_BASE, LENGTH_EXTRA, ORDER};

const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
/// How much is wanted ahead of where matches are looked for, so that the longest match can always be seen.
const LOOKAHEAD: usize = MAX_MATCH + MIN_MATCH + 1;
/// A match of three this far back takes more bits than three literals would.
const TOO_FAR: usize = 4096;

pub(super) const FILTERED: u8 = 1;
pub(super) const HUFFMAN_ONLY: u8 = 2;
pub(super) const RLE: u8 = 3;
pub(super) const FIXED: u8 = 4;

#[derive(Clone, Copy, PartialEq, PartialOrd)]
pub(super) enum Flush {
    None,
    Partial,
    Sync,
    Full,
    Finish,
}

/// What a level does, as zlib has it: a match this long is looked for less hard, one this long isn't waited on to
/// see if a longer one starts next, one this long ends the looking, and this many places are looked at at most.
#[derive(Clone, Copy)]
struct Config {
    good: u16,
    lazy: u16,
    nice: u16,
    chain: u16,
    slow: bool,
}

const fn config(good: u16, lazy: u16, nice: u16, chain: u16, slow: bool) -> Config {
    Config { good, lazy, nice, chain, slow }
}

const CONFIGS: [Config; 10] = [
    config(0, 0, 0, 0, false),
    config(4, 4, 8, 4, false),
    config(4, 5, 16, 8, false),
    config(4, 6, 32, 32, false),
    config(4, 4, 16, 16, true),
    config(8, 16, 32, 32, true),
    config(8, 16, 128, 128, true),
    config(8, 32, 128, 256, true),
    config(32, 128, 258, 1024, true),
    config(32, 258, 258, 4096, true),
];

/// Which length code each length from 3 is.
const LENGTH_CODE: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut code = 0;
    while code < 28 {
        let base = LENGTH_BASE[code] as usize - 3;
        let mut step = 0;
        while step < 1 << LENGTH_EXTRA[code] {
            table[base + step] = code as u8;
            step += 1;
        }
        code += 1;
    }
    table[255] = 28;
    table
};

/// Which distance code each distance is, by the distance less one up to 256, and past that by it over 128, after 256.
const DISTANCE_CODE: [u8; 512] = {
    let mut table = [0u8; 512];
    let mut code = 0;
    while code < 30 {
        let base = DISTANCE_BASE[code] as usize - 1;
        let mut step = 0;
        while step < 1 << DISTANCE_EXTRA[code] {
            let distance = base + step;
            if distance < 256 {
                table[distance] = code as u8;
            } else {
                table[256 + (distance >> 7)] = code as u8;
            }
            step += 1;
        }
        code += 1;
    }
    table
};

fn distance_code(distance: usize) -> usize {
    let less = distance - 1;
    usize::from(if less < 256 { DISTANCE_CODE[less] } else { DISTANCE_CODE[256 + (less >> 7)] })
}

/// How many of the first `most` bytes at `a` and `b` are the same, where `a` comes before `b`, eight at a time.
#[inline]
pub(super) fn same_for(window: &[u8], a: usize, b: usize, most: usize) -> usize {
    let (first, second) = (&window[a..a + most], &window[b..b + most]);
    let mut eights = first.chunks_exact(8).zip(second.chunks_exact(8));
    let mut length = 0;
    for (one, two) in &mut eights {
        let differ = u64::from_le_bytes(one.try_into().unwrap_or_default()) ^ u64::from_le_bytes(two.try_into().unwrap_or_default());
        if differ != 0 {
            return length + (differ.trailing_zeros() / 8) as usize;
        }
        length += 8;
    }
    length + first[length..].iter().zip(&second[length..]).take_while(|(one, two)| one == two).count()
}

pub(super) struct Deflater {
    wrap: Wrap,
    level: u8,
    strategy: u8,
    config: Config,
    window_bits: u32,
    size: usize,
    /// What's been seen, from up to twice the window back, and what's ahead.
    window: Vec<u8>,
    /// Where in `window` the next byte to write is.
    at: usize,
    /// Where in `window` the block being made starts, if that's still there, for writing it as it is.
    block_start: Option<usize>,
    block_bytes: usize,
    /// Where each hash of four bytes was last seen, and for each place, where its hash was seen before that. Zero
    /// is nowhere.
    /// They're places in `window`, which is never more than 64 KB, so they fit in 16 bits, as in zlib, which halves
    /// what has to be cleared for each stream.
    head: Vec<u16>,
    prev: Vec<u16>,
    hash_shift: u32,
    match_length: usize,
    match_start: usize,
    prev_length: usize,
    prev_match: usize,
    match_available: bool,
    /// The block being made: each is a byte or a match's length less three, shifted up, and the match's distance,
    /// which is zero for a byte.
    symbols: Vec<u32>,
    most_symbols: usize,
    literal_counts: [u32; 286],
    distance_counts: [u32; 30],
    writer: BitWriter,
    started: bool,
    check: u32,
    length: u32,
    dictionary: Option<Vec<u8>>,
    last_flush: Flush,
    /// For level 0, what's to be written as it is.
    stored: Vec<u8>,
    finished: bool,
}

impl Deflater {
    pub(super) fn new(wrap: Wrap, level: i32, window_bits: u32, memory: u32, strategy: u8, dictionary: Option<Vec<u8>>) -> Self {
        let level = if level < 0 { 6 } else { level.min(9) as u8 };
        let window_bits = window_bits.clamp(9, 15);
        let memory = memory.clamp(1, 9);
        let hash_bits = memory + 7;
        let size = 1 << window_bits;
        Self {
            wrap,
            level,
            strategy,
            config: CONFIGS[usize::from(level)],
            window_bits,
            size,
            window: Vec::with_capacity(2 * size),
            at: 0,
            block_start: Some(0),
            block_bytes: 0,
            head: vec![0; 1 << hash_bits],
            prev: vec![0; size],
            hash_shift: 32 - hash_bits,
            match_length: MIN_MATCH - 1,
            match_start: 0,
            prev_length: MIN_MATCH - 1,
            prev_match: 0,
            match_available: false,
            symbols: Vec::new(),
            most_symbols: 1 << (memory + 6),
            literal_counts: [0; 286],
            distance_counts: [0; 30],
            writer: BitWriter::default(),
            started: false,
            check: if matches!(wrap, Wrap::Gzip) { 0 } else { 1 },
            length: 0,
            dictionary,
            last_flush: Flush::None,
            stored: Vec::new(),
            finished: false,
        }
    }

    /// Changes the level and strategy, for `params()`, which has flushed first.
    pub(super) fn set_params(&mut self, level: i32, strategy: u8) {
        self.level = if level < 0 { 6 } else { level.min(9) as u8 };
        self.config = CONFIGS[usize::from(self.level)];
        self.strategy = strategy;
    }

    fn header(&mut self) {
        self.started = true;
        match self.wrap {
            Wrap::Gzip => {
                let extra = if self.level == 9 {
                    2
                } else if self.level < 2 || self.strategy >= HUFFMAN_ONLY {
                    4
                } else {
                    0
                };
                self.writer.out.extend_from_slice(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, extra, 10]);
            }
            Wrap::Zlib => {
                let method = (self.window_bits - 8) << 4 | 8;
                let level = if self.strategy >= HUFFMAN_ONLY || self.level < 2 {
                    0
                } else if self.level < 6 {
                    1
                } else if self.level == 6 {
                    2
                } else {
                    3
                };
                let mut flags = level << 6 | u32::from(self.dictionary.is_some()) << 5;
                flags += 31 - (method << 8 | flags) % 31;
                self.writer.out.extend_from_slice(&[method as u8, flags as u8]);
                if let Some(dictionary) = &self.dictionary {
                    self.writer.out.extend_from_slice(&super::adler32_from(dictionary, 1).to_be_bytes());
                }
            }
            _ => {}
        }
        if let Some(dictionary) = self.dictionary.take() {
            // What the dictionary is is history that what comes can refer back to.
            let from = dictionary.len().saturating_sub(self.size - LOOKAHEAD);
            self.window.extend_from_slice(&dictionary[from..]);
            for place in 0..self.window.len().saturating_sub(MIN_MATCH - 1) {
                self.insert(place);
            }
            self.at = self.window.len();
            self.block_start = Some(self.at);
        }
    }

    /// Takes what's given, and gives out what's been written, all of it if `flush` says so.
    pub(super) fn push(&mut self, mut input: &[u8], flush: Flush, out: &mut Vec<u8>) {
        if self.finished {
            return;
        }
        if !self.started {
            self.header();
        }
        // A flush that's no more than the one before, with nothing new, does nothing, as in zlib.
        let useless = input.is_empty() && flush != Flush::Finish && flush <= self.last_flush;
        self.last_flush = flush;
        self.length = self.length.wrapping_add(input.len() as u32);
        self.check = match self.wrap {
            Wrap::Gzip => super::crc32(input, self.check),
            Wrap::Zlib => super::adler32_from(input, self.check),
            _ => self.check,
        };
        if self.level == 0 {
            self.stored.extend_from_slice(input);
            while self.stored.len() >= 65535 && !(flush == Flush::Finish && self.stored.len() == 65535) {
                let rest = self.stored.split_off(65535);
                let block = std::mem::replace(&mut self.stored, rest);
                self.stored_block(&block, false);
            }
            if flush != Flush::None && !useless && (!self.stored.is_empty() || flush == Flush::Finish) {
                let block = std::mem::take(&mut self.stored);
                self.stored_block(&block, flush == Flush::Finish);
            }
        } else {
            loop {
                if self.at >= 2 * self.size - LOOKAHEAD {
                    self.slide();
                }
                let room = 2 * self.size - self.window.len();
                let (now, later) = input.split_at(room.min(input.len()));
                self.window.extend_from_slice(now);
                input = later;
                let flushing = input.is_empty() && flush != Flush::None;
                self.compress(flushing);
                if input.is_empty() {
                    break;
                }
            }
            if flush != Flush::None && !useless && (!self.symbols.is_empty() || flush == Flush::Finish) {
                self.write_block(flush == Flush::Finish);
            }
        }
        match flush {
            Flush::Partial if !useless => {
                // An empty block with the fixed codes, which is ten bits.
                self.writer.put(2, 3);
                self.writer.put(0, 7);
            }
            Flush::Sync | Flush::Full if !useless => {
                self.writer.put(0, 3);
                self.writer.align();
                self.writer.out.extend_from_slice(&[0, 0, 0xff, 0xff]);
                if flush == Flush::Full {
                    self.head.fill(0);
                }
            }
            Flush::Finish => {
                self.writer.align();
                match self.wrap {
                    Wrap::Gzip => {
                        self.writer.out.extend_from_slice(&self.check.to_le_bytes());
                        self.writer.out.extend_from_slice(&self.length.to_le_bytes());
                    }
                    Wrap::Zlib => self.writer.out.extend_from_slice(&self.check.to_be_bytes()),
                    _ => {}
                }
                self.finished = true;
            }
            _ => {}
        }
        out.append(&mut self.writer.out);
    }

    /// Moves the window down by its size, once what's being written is far enough along.
    fn slide(&mut self) {
        let size = self.size;
        self.window.drain(..size);
        self.at -= size;
        self.match_start = self.match_start.saturating_sub(size);
        self.prev_match = self.prev_match.saturating_sub(size);
        self.block_start = self.block_start.and_then(|start| start.checked_sub(size));
        let size = size as u16;
        for place in self.head.iter_mut().chain(self.prev.iter_mut()) {
            *place = place.saturating_sub(size);
        }
    }

    /// A hash of the four bytes at `place`, or the three at the end. Hashing four, as Chromium's zlib (and so Node's)
    /// does, leaves fewer places in each chain that turn out not to match, which made levels 6 and 9 a quarter faster
    /// and level 1 a few percent smaller, for 0.1% to 0.3% more at 6 and 9, as fewer matches of three are found.
    #[inline]
    fn hash(&self, place: usize) -> usize {
        let window = &self.window;
        if let Some(four) = window.get(place..place + 4) {
            let four = u32::from_le_bytes(four.try_into().unwrap_or_default());
            return (four.wrapping_mul(0x9e37_79b1) >> self.hash_shift) as usize;
        }
        let three = u32::from(window[place]) << 16 | u32::from(window[place + 1]) << 8 | u32::from(window[place + 2]);
        (three.wrapping_mul(0x9e37_79b1) >> self.hash_shift) as usize
    }

    /// Notes where the four bytes at `place` were seen, giving where they were seen before.
    #[inline]
    fn insert(&mut self, place: usize) -> usize {
        let hash = self.hash(place);
        let before = self.head[hash];
        self.prev[place & (self.size - 1)] = before;
        self.head[hash] = place as u16;
        before as usize
    }

    /// The longest match for what's at `at` through the chain from `candidate`, if it's longer than `prev_length`.
    fn longest_match(&mut self, mut candidate: usize) -> usize {
        let at = self.at;
        let ahead = self.window.len() - at;
        let mut chain = u32::from(self.config.chain);
        if self.prev_length >= usize::from(self.config.good) {
            chain >>= 2;
        }
        let most = MAX_MATCH.min(ahead);
        let nice = usize::from(self.config.nice).min(most);
        let limit = at.saturating_sub(self.size - LOOKAHEAD);
        let mut best = self.prev_length;
        if best >= most {
            return best;
        }
        // Everything the loop reads is held here, as what it writes would otherwise have it all read again each time.
        let (window, prev, mask) = (&self.window[..], &self.prev[..], self.size - 1);
        let ahead = &window[at..at + most];
        let mut found = self.match_start;
        let two = |bytes: &[u8], from: usize| u16::from_le_bytes([bytes[from], bytes[from + 1]]);
        // A longer match has the same first two bytes and the same two at the end of the best so far, which rule out
        // most places, as zlib does. `best` is at least two, as no match is shorter than three.
        let (mut end, start) = (two(ahead, best - 1), two(ahead, 0));
        loop {
            if candidate + most <= window.len() && two(window, candidate + best - 1) == end && two(window, candidate) == start {
                let length = same_for(window, candidate, at, most);
                if length > best {
                    found = candidate;
                    best = length;
                    if length >= nice {
                        break;
                    }
                    end = two(ahead, best - 1);
                }
            }
            candidate = prev[candidate & mask] as usize;
            chain -= 1;
            if candidate <= limit || chain == 0 {
                break;
            }
        }
        self.match_start = found;
        best
    }

    fn literal(&mut self, byte: u8) {
        self.symbols.push(u32::from(byte) << 16);
        self.literal_counts[usize::from(byte)] += 1;
        self.block_bytes += 1;
        if self.symbols.len() >= self.most_symbols {
            self.write_block(false);
        }
    }

    fn copy(&mut self, distance: usize, length: usize) {
        self.symbols.push(((length - MIN_MATCH) as u32) << 16 | distance as u32);
        self.literal_counts[257 + usize::from(LENGTH_CODE[length - MIN_MATCH])] += 1;
        self.distance_counts[distance_code(distance)] += 1;
        self.block_bytes += length;
        if self.symbols.len() >= self.most_symbols {
            self.write_block(false);
        }
    }

    /// Finds matches for all that's come, but what has to wait for more to come after it unless this is `flushing`.
    fn compress(&mut self, flushing: bool) {
        let strategy = self.strategy;
        loop {
            let ahead = self.window.len() - self.at;
            if ahead == 0 || (ahead < LOOKAHEAD && !flushing) {
                break;
            }
            if strategy == HUFFMAN_ONLY || (strategy == RLE && self.at == 0) {
                let byte = self.window[self.at];
                self.at += 1;
                self.literal(byte);
                continue;
            }
            if strategy == RLE {
                let at = self.at;
                let length = if ahead >= MIN_MATCH { same_for(&self.window, at - 1, at, MAX_MATCH.min(ahead)) } else { 0 };
                if length >= MIN_MATCH {
                    self.at += length;
                    self.copy(1, length);
                } else {
                    let byte = self.window[at];
                    self.at += 1;
                    self.literal(byte);
                }
                continue;
            }
            let candidate = if ahead >= MIN_MATCH { self.insert(self.at) } else { 0 };
            let reach = candidate != 0 && self.at - candidate <= self.size - LOOKAHEAD;
            if self.config.slow {
                self.prev_length = self.match_length;
                self.prev_match = self.match_start;
                self.match_length = MIN_MATCH - 1;
                if reach && self.prev_length < usize::from(self.config.lazy) {
                    self.match_length = self.longest_match(candidate);
                    if self.match_length <= 5
                        && (strategy == FILTERED || (self.match_length == MIN_MATCH && self.at - self.match_start > TOO_FAR))
                    {
                        self.match_length = MIN_MATCH - 1;
                    }
                }
                if self.prev_length >= MIN_MATCH && self.match_length <= self.prev_length {
                    // The match that began a byte ago is the one to take, and what's in it is noted as having been
                    // seen as far as there's enough after it.
                    let last_insert = self.window.len() - MIN_MATCH;
                    let (distance, length) = (self.at - 1 - self.prev_match, self.prev_length);
                    let end = self.at - 1 + length;
                    for place in self.at + 1..end {
                        if place <= last_insert {
                            self.insert(place);
                        }
                    }
                    self.at = end;
                    self.match_available = false;
                    self.match_length = MIN_MATCH - 1;
                    self.copy(distance, length);
                } else if self.match_available {
                    let byte = self.window[self.at - 1];
                    self.at += 1;
                    self.literal(byte);
                } else {
                    self.match_available = true;
                    self.at += 1;
                }
            } else {
                self.prev_length = MIN_MATCH - 1;
                self.match_length = if reach { self.longest_match(candidate) } else { 0 };
                if self.match_length >= MIN_MATCH {
                    let (distance, length) = (self.at - self.match_start, self.match_length);
                    let end = self.at + length;
                    if length <= usize::from(self.config.lazy) && self.window.len() - end >= MIN_MATCH {
                        for place in self.at + 1..end {
                            self.insert(place);
                        }
                    }
                    self.at = end;
                    self.copy(distance, length);
                } else {
                    let byte = self.window[self.at];
                    self.at += 1;
                    self.literal(byte);
                }
            }
        }
        if flushing && self.match_available {
            let byte = self.window[self.at - 1];
            self.match_available = false;
            self.literal(byte);
        }
    }

    fn stored_block(&mut self, bytes: &[u8], last: bool) {
        let mut chunks = bytes.chunks(65535).peekable();
        if bytes.is_empty() {
            self.writer.put(u32::from(last), 3);
            self.writer.align();
            self.writer.out.extend_from_slice(&[0, 0, 0xff, 0xff]);
            return;
        }
        while let Some(chunk) = chunks.next() {
            let final_one = last && chunks.peek().is_none();
            self.writer.put(u32::from(final_one), 3);
            self.writer.align();
            self.writer.out.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
            self.writer.out.extend_from_slice(&(!(chunk.len() as u16)).to_le_bytes());
            self.writer.out.extend_from_slice(chunk);
        }
    }

    /// Writes the block that's been made, the smallest way it can be.
    fn write_block(&mut self, last: bool) {
        self.literal_counts[256] += 1;
        // zlib makes sure there are two codes of each, which some readers need, and so does this.
        for counts in [&mut self.literal_counts[..], &mut self.distance_counts[..]] {
            let mut used = counts.iter().filter(|count| **count > 0).count();
            for count in counts.iter_mut() {
                if used >= 2 {
                    break;
                }
                if *count == 0 {
                    *count = 1;
                    used += 1;
                }
            }
        }
        let literal_lengths = code_lengths(&self.literal_counts, 15);
        let distance_lengths = code_lengths(&self.distance_counts, 15);
        let literal_count = 257 + literal_lengths[257..].iter().rposition(|length| *length != 0).map_or(0, |at| at + 1);
        let distance_count = 1 + distance_lengths[1..].iter().rposition(|length| *length != 0).map_or(0, |at| at + 1);
        // The lengths of both codes, as code-length symbols with what repeats said once.
        let mut all = literal_lengths[..literal_count].to_vec();
        all.extend_from_slice(&distance_lengths[..distance_count]);
        let mut runs: Vec<(u8, u8)> = Vec::new();
        let mut at = 0;
        while at < all.len() {
            let value = all[at];
            let mut run = 1;
            while at + run < all.len() && all[at + run] == value {
                run += 1;
            }
            at += run;
            if value == 0 {
                while run >= 11 {
                    let take = run.min(138);
                    runs.push((18, (take - 11) as u8));
                    run -= take;
                }
                if run >= 3 {
                    runs.push((17, (run - 3) as u8));
                    run = 0;
                }
            } else {
                runs.push((value, 0));
                run -= 1;
                while run >= 3 {
                    let take = run.min(6);
                    runs.push((16, (take - 3) as u8));
                    run -= take;
                }
            }
            for _ in 0..run {
                runs.push((value, 0));
            }
        }
        let mut length_counts = [0u32; 19];
        for (symbol, _) in &runs {
            length_counts[usize::from(*symbol)] += 1;
        }
        let length_lengths = code_lengths(&length_counts, 7);
        let order_count = 4.max(ORDER.iter().rposition(|place| length_lengths[*place] != 0).map_or(0, |at| at + 1));
        let extra_of = |symbol: u8| match symbol {
            16 => 2,
            17 => 3,
            18 => 7,
            _ => 0,
        };
        let header_bits: usize = 14
            + 3 * order_count
            + runs.iter().map(|(symbol, _)| usize::from(length_lengths[usize::from(*symbol)]) + extra_of(*symbol)).sum::<usize>();
        let fixed_length = |symbol: usize| match symbol {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
        let (mut dynamic_bits, mut fixed_bits) = (header_bits, 0usize);
        for (symbol, count) in self.literal_counts.iter().enumerate() {
            let extra = if symbol > 256 { usize::from(LENGTH_EXTRA[symbol - 257]) } else { 0 };
            let count = *count as usize;
            dynamic_bits += count * (usize::from(literal_lengths[symbol]) + extra);
            fixed_bits += count * (fixed_length(symbol) + extra);
        }
        for (symbol, count) in self.distance_counts.iter().enumerate() {
            let count = *count as usize;
            let extra = usize::from(DISTANCE_EXTRA[symbol]);
            dynamic_bits += count * (usize::from(distance_lengths[symbol]) + extra);
            fixed_bits += count * (5 + extra);
        }
        let stored_bits = self.block_start.map(|_| (self.block_bytes + 5 * self.block_bytes.div_ceil(65535).max(1)) * 8 + 7);
        let fixed = self.strategy == FIXED || fixed_bits <= dynamic_bits;
        let best = if fixed { fixed_bits } else { dynamic_bits };
        if let Some(stored) = stored_bits.filter(|stored| *stored < best + 3 && self.strategy != FIXED) {
            let _ = stored;
            let start = self.block_start.unwrap_or(0);
            let block = self.window[start..start + self.block_bytes].to_vec();
            self.stored_block(&block, last);
        } else {
            let (literal_lengths, distance_lengths) = if fixed {
                let mut literals = [8u8; 288];
                literals[144..256].fill(9);
                literals[256..280].fill(7);
                self.writer.put(u32::from(last) | 2, 3);
                // All 288 count for which code each gets, even the two that are never used: without them, every
                // nine-bit code would be four too small.
                (literals.to_vec(), vec![5u8; 30])
            } else {
                self.writer.put(u32::from(last) | 4, 3);
                self.writer.put((literal_count - 257) as u32, 5);
                self.writer.put((distance_count - 1) as u32, 5);
                self.writer.put((order_count - 4) as u32, 4);
                for place in &ORDER[..order_count] {
                    self.writer.put(u32::from(length_lengths[*place]), 3);
                }
                let length_codes = codes_of(&length_lengths);
                for (symbol, extra) in &runs {
                    let symbol = usize::from(*symbol);
                    self.writer.put(u32::from(length_codes[symbol]), u32::from(length_lengths[symbol]));
                    self.writer.put(u32::from(*extra), extra_of(symbol as u8) as u32);
                }
                (literal_lengths, distance_lengths)
            };
            let literal_codes = codes_of(&literal_lengths);
            let distance_codes = codes_of(&distance_lengths);
            for symbol in &self.symbols {
                let distance = (symbol & 0xffff) as usize;
                if distance == 0 {
                    let byte = (symbol >> 16) as usize;
                    self.writer.put(u32::from(literal_codes[byte]), u32::from(literal_lengths[byte]));
                } else {
                    let length = (symbol >> 16) as usize;
                    let code = usize::from(LENGTH_CODE[length]);
                    self.writer.put(u32::from(literal_codes[257 + code]), u32::from(literal_lengths[257 + code]));
                    self.writer.put((length + MIN_MATCH - usize::from(LENGTH_BASE[code])) as u32, u32::from(LENGTH_EXTRA[code]));
                    let code = distance_code(distance);
                    self.writer.put(u32::from(distance_codes[code]), u32::from(distance_lengths[code]));
                    self.writer.put((distance - usize::from(DISTANCE_BASE[code])) as u32, u32::from(DISTANCE_EXTRA[code]));
                }
            }
            self.writer.put(u32::from(literal_codes[256]), u32::from(literal_lengths[256]));
        }
        self.symbols.clear();
        self.literal_counts = [0; 286];
        self.distance_counts = [0; 30];
        // What's next starts where this ended, but for a byte that's waiting to see if a match starts after it.
        self.block_start = Some(self.at - usize::from(self.match_available));
        self.block_bytes = 0;
    }
}
