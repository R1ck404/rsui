//! Zstandard written so it's about as small as zstd makes it at each level. Earlier copies are found through chains of
//! where the same few bytes were seen, the last three offsets are tried first as they cost the least to say, and a
//! match can wait a byte or two to see whether a better one starts there, as zstd's lazy levels do. Each block's
//! literals get a Huffman code made for them, and its sequences FSE codes that are the usual ones, one symbol, made
//! for the block or the block before's, whichever comes out smallest. A block that doesn't get smaller is stored as
//! it is.

use super::deflate::same_for;
use super::huffman::{BitWriter, Stop, code_lengths};
use super::zstd::{BLOCK_MAX, Dictionary, KINDS, LL_BASE, LL_BITS, MAGIC, ML_BASE, ML_BITS, Xxh64, read_dictionary, spread};

/// What a level does, as zstd has it for inputs over 256 KB: the window, the chain of earlier places and the table
/// of where each hash was last seen, as powers of two, how many places to look at (also a power of two), the
/// shortest match, one long enough to stop looking at, and the strategy, from 1 for the fastest to 9.
const LEVELS: [[u16; 7]; 23] = [
    [19, 12, 13, 1, 6, 1, 1],
    [19, 13, 14, 1, 7, 0, 1],
    [20, 15, 16, 1, 6, 0, 1],
    [21, 16, 17, 1, 5, 0, 2],
    [21, 18, 18, 1, 5, 0, 2],
    [21, 18, 19, 3, 5, 2, 3],
    [21, 18, 19, 3, 5, 4, 4],
    [21, 19, 20, 4, 5, 8, 4],
    [21, 19, 20, 4, 5, 16, 5],
    [22, 20, 21, 4, 5, 16, 5],
    [22, 21, 22, 5, 5, 16, 5],
    [22, 21, 22, 6, 5, 16, 5],
    [22, 22, 23, 6, 5, 32, 5],
    [22, 22, 22, 4, 5, 32, 6],
    [22, 22, 23, 5, 5, 32, 6],
    [22, 23, 23, 6, 5, 32, 6],
    [22, 22, 22, 5, 5, 48, 7],
    [23, 23, 22, 5, 4, 64, 7],
    [23, 23, 22, 6, 3, 64, 8],
    [23, 24, 22, 7, 3, 256, 9],
    [25, 25, 23, 7, 3, 256, 9],
    [26, 26, 24, 7, 3, 512, 9],
    [27, 27, 25, 9, 3, 999, 9],
];

/// The parameters zstd takes, by their numbers, that change what's written.
const LEVEL: u32 = 100;
const WINDOW_LOG: u32 = 101;
const HASH_LOG: u32 = 102;
const CHAIN_LOG: u32 = 103;
const SEARCH_LOG: u32 = 104;
const MIN_MATCH: u32 = 105;
const TARGET_LENGTH: u32 = 106;
const STRATEGY: u32 = 107;
const CONTENT_SIZE: u32 = 200;
const CHECKSUM: u32 = 201;
const DICTIONARY_ID: u32 = 202;
const FORMAT: u32 = 10;

#[derive(Clone, Copy)]
struct Effort {
    window_log: u32,
    chain_log: u32,
    hash_log: u32,
    search_log: u32,
    min_match: u32,
    target: u32,
    strategy: u32,
}

/// The way each kind of sequence code was last written, which the next block can say to use again.
#[derive(Clone)]
enum Code {
    One(u8),
    Table(Coder),
}

/// An FSE code to write, laid out as zstd's encoder lays it out: the state after each, sorted by symbol, and for
/// each symbol what finds how many bits a state gives out and where the next state is.
#[derive(Clone)]
struct Coder {
    log: u32,
    counts: Vec<i16>,
    next: Vec<u16>,
    symbols: Vec<(u32, i32)>,
}

impl Coder {
    fn new(counts: &[i16], log: u32) -> Self {
        let size = 1u32 << log;
        let spread = spread(counts, log).unwrap_or_default();
        let mut start = vec![0u32; counts.len() + 1];
        for (symbol, count) in counts.iter().enumerate() {
            start[symbol + 1] = start[symbol] + if *count == -1 { 1 } else { (*count).max(0) as u32 };
        }
        let mut next = vec![0u16; size as usize];
        let mut fill = start.clone();
        for (at, symbol) in spread.iter().enumerate() {
            next[fill[usize::from(*symbol)] as usize] = (size as usize + at) as u16;
            fill[usize::from(*symbol)] += 1;
        }
        let symbols = counts
            .iter()
            .zip(&start)
            .map(|(count, start)| match *count {
                0 => (0, 0),
                -1 | 1 => ((log << 16) - size, *start as i32 - 1),
                count => {
                    let count = count as u32;
                    let most = log - (31 - (count - 1).leading_zeros());
                    ((most << 16) - (count << most), *start as i32 - count as i32)
                }
            })
            .collect();
        Self { log, counts: counts.to_vec(), next, symbols }
    }

    fn start(&self, symbol: u8) -> u32 {
        let (delta, find) = self.symbols[usize::from(symbol)];
        let bits = (delta + (1 << 15)) >> 16;
        let value = (bits << 16) - delta;
        u32::from(self.next[((value >> bits) as i32 + find) as usize])
    }

    #[inline]
    fn put(&self, state: &mut u32, symbol: u8, writer: &mut BitWriter) {
        let (delta, find) = self.symbols[usize::from(symbol)];
        let bits = (*state + delta) >> 16;
        writer.put(*state & ((1 << bits) - 1), bits);
        *state = u32::from(self.next[((*state >> bits) as i32 + find) as usize]);
    }

    fn finish(&self, state: u32, writer: &mut BitWriter) {
        writer.put(state & ((1 << self.log) - 1), self.log);
    }

    /// About how many bits these counts of symbols would take, if all of them have a place.
    fn cost(&self, counts: &[u32]) -> Option<f32> {
        counts.iter().enumerate().filter(|(_, count)| **count > 0).try_fold(0.0, |sum, (symbol, count)| {
            let had = *self.counts.get(symbol)?;
            (had != 0).then(|| sum + *count as f32 * (self.log as f32 - f32::from(had.max(1)).log2()))
        })
    }
}

/// The place of the highest bit that's set, taking nothing as one.
fn highest_bit(value: u32) -> u32 {
    31 - value.max(1).leading_zeros()
}

/// How accurate an FSE table for this many of these symbols should be, as zstd works it out.
fn table_log(most: u32, count: usize, most_symbol: usize) -> u32 {
    let count = count.max(2) as u32;
    let from_count = highest_bit(count - 1).saturating_sub(2);
    let least = (highest_bit(count) + 1).min(highest_bit(most_symbol as u32) + 2);
    most.min(from_count).max(least).clamp(5, most)
}

/// Counts scaled to add up to the table's size, each that comes at all getting at least one place.
fn normalize(counts: &[u32], log: u32) -> Vec<i16> {
    let total: u64 = counts.iter().map(|count| u64::from(*count)).sum();
    let size = 1i32 << log;
    let mut scaled: Vec<i16> = counts
        .iter()
        .map(|count| if *count == 0 { 0 } else { ((u64::from(*count) * size as u64 + total / 2) / total).max(1) as i16 })
        .collect();
    let mut over: i32 = scaled.iter().map(|count| i32::from(*count)).sum::<i32>() - size;
    while over != 0 {
        let Some(largest) = (0..scaled.len()).max_by_key(|at| scaled[*at]) else { break };
        let change = if over < 0 { over } else { over.min(i32::from(scaled[largest]) - 1).max(1) };
        if over > 0 && scaled[largest] <= 1 {
            break;
        }
        scaled[largest] -= change as i16;
        over -= change;
    }
    scaled
}

/// Writes an FSE table's description, which `zstd::read_counts` reads.
fn write_counts(counts: &[i16], log: u32, out: &mut Vec<u8>) {
    let mut writer = BitWriter::default();
    writer.put(log - 5, 4);
    let mut remaining = 1i32 << log;
    let mut symbol = 0;
    while remaining > 0 {
        let width = 32 - (remaining as u32 + 1).leading_zeros();
        let lower = (1 << (width - 1)) - 1;
        let threshold = (1 << width) - 1 - (remaining as u32 + 1);
        let value = (counts[symbol] + 1) as u32;
        if value < threshold {
            writer.put(value, width - 1);
        } else if value <= lower {
            writer.put(value, width);
        } else {
            writer.put(value + threshold, width);
        }
        remaining -= i32::from(counts[symbol]).abs();
        symbol += 1;
        if counts[symbol - 1] == 0 {
            let mut zeros = counts[symbol..].iter().take_while(|count| **count == 0).count();
            symbol += zeros;
            while zeros >= 3 {
                writer.put(3, 2);
                zeros -= 3;
            }
            writer.put(zeros as u32, 2);
        }
    }
    writer.align();
    out.append(&mut writer.out);
}

fn close(mut writer: BitWriter, out: &mut Vec<u8>) {
    writer.put(1, 1);
    writer.align();
    out.append(&mut writer.out);
}

/// The description of a Huffman tree with these weights, the last of which isn't written: in four bits each, or
/// squeezed with FSE, whichever is smaller.
fn describe(weights: &[u8]) -> Option<Vec<u8>> {
    let mut direct = None;
    if weights.len() <= 128 {
        let mut out = vec![127 + weights.len() as u8];
        out.extend(weights.chunks(2).map(|two| two[0] << 4 | two.get(1).copied().unwrap_or(0)));
        direct = Some(out);
    }
    let mut counts = [0u32; 13];
    for weight in weights {
        counts[usize::from(*weight)] += 1;
    }
    if counts.iter().filter(|count| **count > 0).count() < 2 {
        return direct;
    }
    let most = counts.iter().rposition(|count| *count > 0).unwrap_or(0);
    let log = table_log(6, weights.len(), most);
    let normal = normalize(&counts[..=most], log);
    let coder = Coder::new(&normal, log);
    let mut squeezed = vec![0];
    write_counts(&normal, log, &mut squeezed);
    // Two states take turns, so the one the reader starts with is the first weight's.
    let mut writer = BitWriter::default();
    let mut left = weights.len();
    let (mut one, mut two);
    if left % 2 == 1 {
        one = coder.start(weights[left - 1]);
        two = coder.start(weights[left - 2]);
        coder.put(&mut one, weights[left - 3], &mut writer);
        left -= 3;
    } else {
        two = coder.start(weights[left - 1]);
        one = coder.start(weights[left - 2]);
        left -= 2;
    }
    while left > 0 {
        coder.put(&mut two, weights[left - 1], &mut writer);
        coder.put(&mut one, weights[left - 2], &mut writer);
        left -= 2;
    }
    coder.finish(two, &mut writer);
    coder.finish(one, &mut writer);
    close(writer, &mut squeezed);
    squeezed[0] = (squeezed.len() - 1) as u8;
    match direct {
        Some(direct) if direct.len() <= squeezed.len() => Some(direct),
        _ if squeezed.len() <= 128 => Some(squeezed),
        direct => direct,
    }
}

/// The literals section's header for literals as they are (kind 0) or one byte repeated (kind 1).
fn plain_header(kind: u8, size: usize, out: &mut Vec<u8>) {
    if size < 32 {
        out.push(kind | (size as u8) << 3);
    } else if size < 4096 {
        out.extend_from_slice(&[kind | 4 | (size as u8) << 4, (size >> 4) as u8]);
    } else {
        out.extend_from_slice(&[kind | 12 | (size as u8) << 4, (size >> 4) as u8, (size >> 12) as u8]);
    }
}

#[inline(never)]
fn literals_section(literals: &[u8], slow: bool, out: &mut Vec<u8>) {
    let size = literals.len();
    if size > 1 && literals.iter().all(|byte| *byte == literals[0]) {
        plain_header(1, size, out);
        out.push(literals[0]);
        return;
    }
    if size >= 64 {
        if let Some(packed) = huffman(literals) {
            let least = (size >> if slow { 7 } else { 6 }) + 2;
            if packed.len() + least < size {
                let single = size < 256;
                let header = if single { 3 } else { 3 + usize::from(size >= 1024) + usize::from(size >= 16384) };
                let format = if single { 0 } else { header as u64 - 2 };
                let width = [10, 10, 14, 18][format as usize];
                let value = 2 | format << 2 | (size as u64) << 4 | (packed.len() as u64) << (4 + width);
                out.extend_from_slice(&value.to_le_bytes()[..header]);
                out.extend_from_slice(&packed);
                return;
            }
        }
    }
    plain_header(0, size, out);
    out.extend_from_slice(literals);
}

/// Literals in a Huffman code made for them: its description, then one stream, or four with a table of where
/// they start.
#[inline(never)]
fn huffman(literals: &[u8]) -> Option<Vec<u8>> {
    let mut counts = [0u32; 256];
    for byte in literals {
        counts[usize::from(*byte)] += 1;
    }
    let lengths = code_lengths(&counts, 11);
    let bits = u32::from(*lengths.iter().max()?);
    let weights: Vec<u8> = lengths.iter().map(|length| if *length == 0 { 0 } else { (bits + 1 - u32::from(*length)) as u8 }).collect();
    let last = weights.iter().rposition(|weight| *weight > 0)?;
    let mut out = describe(&weights[..last])?;
    // The lightest symbols get the lowest codes, as the reader's table has them first.
    let mut codes = [(0u32, 0u32); 256];
    let mut place = 0u32;
    for weight in 1..=bits as u8 {
        for (symbol, _) in weights.iter().enumerate().filter(|(_, each)| **each == weight) {
            codes[symbol] = (place >> (weight - 1), bits + 1 - u32::from(weight));
            place += 1 << (weight - 1);
        }
    }
    let stream = |part: &[u8], out: &mut Vec<u8>| {
        let mut writer = BitWriter::default();
        for byte in part.iter().rev() {
            let (code, length) = codes[usize::from(*byte)];
            writer.put(code, length);
        }
        close(writer, out);
    };
    if literals.len() < 256 {
        stream(literals, &mut out);
    } else {
        let jumps = out.len();
        out.extend_from_slice(&[0; 6]);
        let segment = literals.len().div_ceil(4);
        for (number, part) in literals.chunks(segment).enumerate() {
            let from = out.len();
            stream(part, &mut out);
            if number < 3 {
                let length = u16::try_from(out.len() - from).ok()?;
                out[jumps + 2 * number..jumps + 2 * number + 2].copy_from_slice(&length.to_le_bytes());
            }
        }
    }
    Some(out)
}

/// The code a value has among codes that each start at a base.
const fn code_of(bases: &[u32], value: u32) -> u8 {
    let mut code = 0;
    while code + 1 < bases.len() && bases[code + 1] <= value {
        code += 1;
    }
    code as u8
}

/// The code of each literal length up to 63, and of each match length from 3 up to 130.
const LL_CODES: [u8; 64] = {
    let mut table = [0; 64];
    let mut length = 0;
    while length < 64 {
        table[length] = code_of(&LL_BASE, length as u32);
        length += 1;
    }
    table
};
const ML_CODES: [u8; 128] = {
    let mut table = [0; 128];
    let mut length = 0;
    while length < 128 {
        table[length] = code_of(&ML_BASE, length as u32 + 3);
        length += 1;
    }
    table
};

/// The code of a literal length, from a table up to 63 and from its highest bit past that, as zstd works it out. What
/// the slowest levels price can be longer than any block has, which gets the last code, as it would from the bases.
#[inline]
fn literal_code(length: u32) -> u8 {
    if length < 64 { LL_CODES[length as usize] } else { (highest_bit(length) as u8 + 19).min(35) }
}

#[inline]
fn match_code(length: u32) -> u8 {
    let past = length - 3;
    if past < 128 { ML_CODES[past as usize] } else { (highest_bit(past) as u8 + 36).min(52) }
}

/// The usual FSE codes, made once.
fn usual_coders() -> &'static [Coder; 3] {
    static USUAL: std::sync::OnceLock<[Coder; 3]> = std::sync::OnceLock::new();
    USUAL.get_or_init(|| KINDS.map(|(counts, log, _, _)| Coder::new(counts, log)))
}

/// A sequence: how many literals come before it, the length of its match, and its offset value, as written.
#[derive(Clone, Copy)]
struct Sequence {
    literals: u32,
    length: u32,
    value: u32,
}

/// The value that says this offset, which is one of the last three if it can be, changing them as `resolve` will:
/// the first of the values from 1 to 3 that stands for it, or the offset plus three.
#[inline]
fn offset_value(last: &mut [u32; 3], offset: u32, literals: u32) -> u32 {
    let [first, second, third] = *last;
    // With no literals before it, each value means the one after what it would, and the third is one less than the
    // last offset.
    let (value, made) = if literals > 0 {
        if offset == first {
            (1, *last)
        } else if offset == second {
            (2, [second, first, third])
        } else if offset == third {
            (3, [third, first, second])
        } else {
            (offset + 3, [offset, first, second])
        }
    } else if offset == second {
        (1, [second, first, third])
    } else if offset == third {
        (2, [third, first, second])
    } else if offset == first.wrapping_sub(1) {
        (3, [offset, first, second])
    } else {
        (offset + 3, [offset, first, second])
    };
    *last = made;
    value
}

/// Adds `data[from..to]` to `literals`, sixteen bytes at a time when there's room, as most runs of literals are short
/// and copying them that way takes no call.
#[inline]
fn add_literals(literals: &mut Vec<u8>, data: &[u8], from: usize, to: usize) {
    match data.get(from..from + 16) {
        Some(sixteen) if to - from <= 16 => {
            let keep = literals.len() + to - from;
            literals.extend_from_slice(sixteen);
            literals.truncate(keep);
        }
        _ => literals.extend_from_slice(&data[from..to]),
    }
}

/// Where each hash of the first few bytes at a place was last seen, and for each place either where its hash was
/// seen before that, or, for the slower levels, the two places under it in a tree of the places before it, sorted by
/// what comes after each, as zstd's binary tree has them. Zero is nowhere.
struct Finder {
    head: Vec<u32>,
    chain: Vec<u32>,
    tree: bool,
    hash_shift: u32,
    hash_bytes: u32,
    attempts: usize,
    window: usize,
    /// The places from here on haven't been noted yet.
    noted: usize,
}

thread_local! {
    /// The tables of encoders that are done with them, for the next to clear rather than have the system find fresh
    /// memory for. Each of more than half a megabyte comes straight from the system, and faulting it in again took
    /// about as long as squeezing 64 KB. No more than `SPARE_MOST` bytes are kept.
    static SPARE: std::cell::RefCell<Vec<Vec<u32>>> = const { std::cell::RefCell::new(Vec::new()) };
}

const SPARE_MOST: usize = 1 << 20;

/// A table of `size` zeros, from a spare one if there's one of about that size.
fn zeroed(size: usize) -> Vec<u32> {
    let spare = SPARE
        .try_with(|spare| {
            let mut spare = spare.try_borrow_mut().ok()?;
            let at = spare.iter().position(|table| table.capacity() >= size && table.capacity() <= 2 * size)?;
            Some(spare.swap_remove(at))
        })
        .ok()
        .flatten();
    match spare {
        Some(mut table) => {
            table.clear();
            table.resize(size, 0);
            table
        }
        None => vec![0; size],
    }
}

impl Drop for Finder {
    fn drop(&mut self) {
        // Codecs that streams hold can be let go of as the thread ends, after the spares are gone, which is fine.
        let _ = SPARE.try_with(|spare| {
            let Ok(mut spare) = spare.try_borrow_mut() else { return };
            for table in [std::mem::take(&mut self.head), std::mem::take(&mut self.chain)] {
                let kept: usize = spare.iter().map(|table| 4 * table.capacity()).sum();
                if table.capacity() > 0 && kept + 4 * table.capacity() <= SPARE_MOST {
                    spare.push(table);
                }
            }
        });
    }
}

impl Finder {
    #[inline]
    fn hash(&self, data: &[u8], at: usize) -> usize {
        let word = u64::from_le_bytes(data[at..at + 8].try_into().unwrap_or_default());
        ((word << (64 - 8 * self.hash_bytes)).wrapping_mul(0x9e37_79b1_85eb_ca87) >> self.hash_shift) as usize
    }

    /// Notes every place up to `at`, which can be hashed.
    fn note_to(&mut self, data: &[u8], at: usize) {
        while self.noted < at {
            let place = self.noted;
            self.noted += 1;
            if self.tree {
                self.descend(data, place, data.len() - place, 4096, None);
            } else {
                let hash = self.hash(data, place);
                let mask = self.chain.len() - 1;
                self.chain[place & mask] = self.head[hash];
                self.head[hash] = place as u32;
            }
        }
    }

    /// Notes `at`, and gives the matches for what's there up to `end`, each longer and further away than the one
    /// before, as their lengths and offsets.
    #[inline(never)]
    fn search(&mut self, data: &[u8], at: usize, end: usize, nice: usize, found: &mut Vec<(usize, usize)>) {
        self.note_to(data, at);
        self.noted = at + 1;
        if self.tree {
            self.descend(data, at, end - at, nice, Some(found));
            return;
        }
        let mask = self.chain.len() - 1;
        let low = at.saturating_sub(self.window).max(1);
        let most = end - at;
        let hash = self.hash(data, at);
        let mut candidate = self.head[hash] as usize;
        self.chain[at & mask] = candidate as u32;
        self.head[hash] = at as u32;
        let mut best = self.hash_bytes as usize - 1;
        for _ in 0..self.attempts {
            if candidate >= at || candidate < low || best >= most {
                break;
            }
            if data[candidate + best] == data[at + best] && data[candidate] == data[at] {
                let length = same_for(data, candidate, at, most);
                if length > best {
                    best = length;
                    found.push((length, at - candidate));
                    if length >= nice {
                        break;
                    }
                }
            }
            let next = self.chain[candidate & mask] as usize;
            if next >= candidate {
                break;
            }
            candidate = next;
        }
    }

    /// Puts `at` at the top of its tree, going down from where it was to split what's below between the two sides,
    /// and gives the matches met on the way if `found` is there. A match as long as `most` or `nice` ends it, and
    /// what's below that place is let go of, as which side it'd go on isn't known.
    fn descend(&mut self, data: &[u8], at: usize, most: usize, nice: usize, mut found: Option<&mut Vec<(usize, usize)>>) {
        let mask = self.chain.len() / 2 - 1;
        let hash = self.hash(data, at);
        let mut candidate = self.head[hash] as usize;
        self.head[hash] = at as u32;
        let low = at.saturating_sub(self.window).max(at.saturating_sub(mask)).max(1);
        let (mut smaller, mut larger) = (2 * (at & mask), 2 * (at & mask) + 1);
        let (mut common_smaller, mut common_larger) = (0, 0);
        let mut best = self.hash_bytes as usize - 1;
        // When places are only being noted, a match as long as `nice` ends it, so none is compared further than that,
        // or a long repeat would be compared all the way to its end for each place in it.
        let reach = if found.is_some() { most } else { most.min(nice) };
        for _ in 0..self.attempts {
            if candidate < low || candidate >= at {
                break;
            }
            let common = common_smaller.min(common_larger).min(reach);
            let length = common + same_for(data, candidate + common, at + common, reach - common);
            if length > best {
                best = length;
                if let Some(found) = found.as_deref_mut() {
                    found.push((length, at - candidate));
                }
            }
            if length >= most.min(nice) {
                break;
            }
            if data[candidate + length] < data[at + length] {
                self.chain[smaller] = candidate as u32;
                common_smaller = length;
                smaller = 2 * (candidate & mask) + 1;
                candidate = self.chain[smaller] as usize;
            } else {
                self.chain[larger] = candidate as u32;
                common_larger = length;
                larger = 2 * (candidate & mask);
                candidate = self.chain[larger] as usize;
            }
        }
        self.chain[smaller] = 0;
        self.chain[larger] = 0;
    }
}

/// How long a match there is at `at` with each of the last offsets that can be said as one, with that offset.
#[inline(never)]
fn repeats(data: &[u8], last: &[u32; 3], at: usize, end: usize, window: usize, literals: usize) -> [(usize, usize); 3] {
    let offsets = [last[0], last[1], last[2], last[0].wrapping_sub(1)];
    let tried = if literals == 0 { &offsets[1..] } else { &offsets[..3] };
    let mut out = [(0, 0); 3];
    for (place, offset) in out.iter_mut().zip(tried) {
        let offset = *offset as usize;
        if offset > 0 && offset <= at.min(window) {
            *place = (same_for(data, at - offset, at, end - at), offset);
        }
    }
    out
}

/// A place the cheapest way of writing a stretch has reached: what that costs, the match that ends there (none for
/// a literal), the literals since the last match, and the last offsets it leaves.
#[derive(Clone, Copy)]
struct Step {
    price: f32,
    length: u32,
    offset: u32,
    literals: u32,
    last: [u32; 3],
}

fn relax(steps: &mut Vec<Step>, at: usize, step: Step) {
    if at >= steps.len() {
        steps.resize(at + 1, Step { price: f32::INFINITY, ..step });
    }
    if step.price < steps[at].price {
        steps[at] = step;
    }
}

/// How many bits each symbol costs, if they come as often as these say, each once more.
fn prices(counts: &[u32]) -> Vec<f32> {
    let total: f32 = counts.iter().map(|count| *count as f32 + 1.0).sum();
    counts.iter().map(|count| (total / (*count as f32 + 1.0)).log2()).collect()
}

pub(super) struct Encoder {
    level: i32,
    params: Vec<(u32, i32)>,
    effort: Effort,
    checksum: bool,
    size_flag: bool,
    id_flag: bool,
    magic: bool,
    pledged: Option<u64>,
    dictionary: Result<Option<Dictionary>, ()>,
    /// The frame's size, once it's known.
    size: Option<u64>,
    fresh: bool,
    started: bool,
    /// Where blocks can refer back to: what's been written, the dictionary, then what's still to be.
    data: Vec<u8>,
    at: usize,
    window: usize,
    single: bool,
    finder: Option<Finder>,
    last: [u32; 3],
    codes: [Option<Code>; 3],
    sum: Xxh64,
    total: u64,
    literals: Vec<u8>,
    sequences: Vec<Sequence>,
    /// How often each code of each kind came in the last block, and each literal, for what the slowest levels think
    /// things cost.
    stats: [Vec<u32>; 4],
}

impl Encoder {
    /// An encoder at a level, changed by zstd's parameters, by number, for a frame this big if that's known, with a
    /// dictionary.
    pub(super) fn new(level: i32, params: Vec<(u32, i32)>, pledged: Option<u64>, dictionary: Option<Vec<u8>>) -> Self {
        let param = |key: u32| params.iter().rev().find(|(each, _)| *each == key).map(|(_, value)| *value);
        let level = param(LEVEL).unwrap_or(level);
        // Only the content and the last offsets of a dictionary with tables are used, as each block has its own.
        let dictionary = dictionary.map(read_dictionary).transpose().map_err(|_| ());
        let checksum = param(CHECKSUM).is_some_and(|value| value != 0);
        let size_flag = param(CONTENT_SIZE).is_none_or(|value| value != 0);
        let id_flag = param(DICTIONARY_ID).is_none_or(|value| value != 0);
        let magic = param(FORMAT).is_none_or(|value| value == 0);
        let mut made = Self {
            level: if level == 0 { 3 } else { level.clamp(-131_072, 22) },
            params,
            effort: Effort { window_log: 0, chain_log: 0, hash_log: 0, search_log: 0, min_match: 0, target: 0, strategy: 0 },
            checksum,
            size_flag,
            id_flag,
            magic,
            pledged,
            dictionary,
            size: None,
            fresh: true,
            started: false,
            data: Vec::new(),
            at: 0,
            window: 0,
            single: false,
            finder: None,
            last: [1, 4, 8],
            codes: [None, None, None],
            sum: Xxh64::new(),
            total: 0,
            literals: Vec::new(),
            sequences: Vec::new(),
            stats: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        };
        made.effort = made.effort_for(None, 0);
        made
    }

    /// What the level and parameters come to for a frame of this size, if it's known.
    fn effort_for(&self, size: Option<u64>, dictionary: usize) -> Effort {
        let row = LEVELS[self.level.max(0) as usize].map(u32::from);
        let param = |key: u32, otherwise: u32| match self.params.iter().rev().find(|(each, _)| *each == key) {
            Some((_, value)) if *value > 0 => *value as u32,
            _ => otherwise,
        };
        let mut effort = Effort {
            window_log: param(WINDOW_LOG, row[0]),
            chain_log: param(CHAIN_LOG, row[1]),
            hash_log: param(HASH_LOG, row[2]),
            search_log: param(SEARCH_LOG, row[3]),
            min_match: param(MIN_MATCH, row[4]).clamp(3, 7),
            target: param(TARGET_LENGTH, row[5]),
            strategy: param(STRATEGY, row[6]),
        };
        // A frame that's known to be smaller needs no more window than it has, nor tables bigger than that.
        if let Some(size) = size {
            let whole = size + dictionary as u64;
            let log = if whole < 64 { 6 } else { highest_bit((whole - 1).min(u64::from(u32::MAX)) as u32) + 1 };
            effort.window_log = effort.window_log.min(log).max(10);
        }
        let log = effort.window_log;
        // A tree takes two places for each place it reaches back to, where a chain takes one. Past these, the
        // tables would take more memory than they're worth here.
        effort.hash_log = effort.hash_log.min(log + 1).min(22);
        effort.chain_log = effort.chain_log.min(log + u32::from(effort.strategy >= 6)).clamp(6, 24);
        effort
    }

    /// Takes what's given, and gives out what's been written: whole blocks for `operation` 0, all of it for 1, and
    /// the end of the frame for 2.
    pub(super) fn push(&mut self, mut input: &[u8], operation: u32, out: &mut Vec<u8>) -> Result<(), Stop> {
        if self.fresh {
            self.begin(if operation == 2 { Some(input.len() as u64) } else { self.pledged })?;
        }
        self.total += input.len() as u64;
        if self.size.is_some_and(|size| self.total > size) {
            return Err(Stop::Bad("ZSTD_error_srcSize_wrong", "Src size is incorrect"));
        }
        if self.checksum {
            self.sum.update(input);
        }
        let block = BLOCK_MAX.min(self.window);
        loop {
            let room = (2 * self.window + 2 * block).saturating_sub(self.data.len());
            let (now, later) = input.split_at(room.min(input.len()));
            self.data.extend_from_slice(now);
            input = later;
            while self.data.len() - self.at > block {
                self.block(self.at + block, false, out);
            }
            if input.is_empty() {
                break;
            }
        }
        if operation == 1 && self.data.len() > self.at {
            self.block(self.data.len(), false, out);
        }
        if operation == 2 {
            if self.size.is_some_and(|size| self.total != size) {
                return Err(Stop::Bad("ZSTD_error_srcSize_wrong", "Src size is incorrect"));
            }
            if self.data.len() > self.at {
                self.block(self.data.len(), true, out);
            } else {
                self.header(out);
                out.extend_from_slice(&[1, 0, 0]);
            }
            if self.checksum {
                out.extend_from_slice(&self.sum.sum().to_le_bytes());
            }
            self.fresh = true;
        }
        Ok(())
    }

    /// Gets ready for a frame of this size, if it's known.
    fn begin(&mut self, size: Option<u64>) -> Result<(), Stop> {
        let Ok(dictionary) = &self.dictionary else {
            return Err(Stop::Bad("ZSTD_error_memory_allocation", "Allocation error : not enough memory"));
        };
        let content = dictionary.as_ref().map_or(&[][..], |found| &found.content[..]);
        self.effort = self.effort_for(size, content.len());
        self.size = size;
        self.single = self.size_flag && size.is_some_and(|size| size <= 1 << self.effort.window_log);
        self.window = 1 << self.effort.window_log;
        self.data.clear();
        self.data.extend_from_slice(content);
        self.at = content.len();
        self.last = dictionary.as_ref().map_or([1, 4, 8], |found| found.carried.last);
        self.codes = [None, None, None];
        self.stats = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        self.sum = Xxh64::new();
        self.total = 0;
        self.fresh = false;
        self.started = false;
        let effort = self.effort;
        let mut finder = Finder {
            head: zeroed(1 << effort.hash_log),
            chain: zeroed(1 << effort.chain_log),
            tree: effort.strategy >= 6,
            hash_shift: 64 - effort.hash_log,
            hash_bytes: effort.min_match.max(4),
            attempts: match effort.strategy {
                1 => 1,
                2 => 4,
                _ => 1 << effort.search_log,
            },
            window: if self.single { usize::MAX } else { self.window },
            noted: 1,
        };
        if effort.strategy <= 2 {
            // What came before, which is the dictionary, is noted the way those strategies look for it.
            let (long_bits, short_bits) = (64 - finder.hash_shift, finder.chain.len().trailing_zeros());
            let short = finder.hash_bytes;
            for place in 1..self.at.saturating_sub(7) {
                let eight = u64::from_le_bytes(self.data[place..place + 8].try_into().unwrap_or_default());
                if effort.strategy == 2 {
                    finder.head[hash_of(eight, 8, long_bits)] = place as u32;
                    finder.chain[hash_of(eight, short, short_bits)] = place as u32;
                } else {
                    finder.head[hash_of(eight, short, long_bits)] = place as u32;
                }
            }
        } else {
            finder.note_to(&self.data, self.at.saturating_sub(7));
        }
        self.finder = Some(finder);
        Ok(())
    }

    fn header(&mut self, out: &mut Vec<u8>) {
        if self.started {
            return;
        }
        self.started = true;
        if self.magic {
            out.extend_from_slice(&MAGIC.to_le_bytes());
        }
        let size = self.size.filter(|_| self.size_flag);
        let size_code = size.map_or(0, |size| u8::from(size >= 256) + u8::from(size >= 65536 + 256) + u8::from(size >= 0xffff_ffff));
        let id = self.dictionary.as_ref().ok().and_then(Option::as_ref).map_or(0, |found| found.id);
        let id = if self.id_flag { id } else { 0 };
        let id_code = if id == 0 { 0 } else if id < 256 { 1 } else if id < 65536 { 2 } else { 3 };
        out.push(size_code << 6 | u8::from(self.single) << 5 | u8::from(self.checksum) << 2 | id_code);
        if !self.single {
            out.push(((self.effort.window_log - 10) << 3) as u8);
        }
        out.extend_from_slice(&id.to_le_bytes()[..[0, 1, 2, 4][usize::from(id_code)]]);
        if let Some(size) = size {
            match size_code {
                0 if self.single => out.push(size as u8),
                0 => {}
                1 => out.extend_from_slice(&((size - 256) as u16).to_le_bytes()),
                2 => out.extend_from_slice(&(size as u32).to_le_bytes()),
                _ => out.extend_from_slice(&size.to_le_bytes()),
            }
        }
    }

    /// Writes what's from `at` to `end` as a block, then moves the window along if it's far enough on.
    fn block(&mut self, end: usize, last: bool, out: &mut Vec<u8>) {
        // zstd never makes a frame's first block one byte repeated, as readers before 1.4.4 got that wrong.
        let first = !self.started;
        self.header(out);
        let start = self.at;
        let (last_before, codes_before) = (self.last, self.codes.clone());
        self.literals.clear();
        self.sequences.clear();
        if self.effort.strategy >= 7 {
            self.find_cheapest(start, end);
            let mut counts = vec![0u32; 256];
            for byte in &self.literals {
                counts[usize::from(*byte)] += 1;
            }
            self.stats[3] = counts;
        } else if self.effort.strategy <= 2 {
            self.find_quick(start, end);
        } else {
            self.find(start, end);
        }
        let mut body = Vec::with_capacity((end - start) / 2 + 64);
        literals_section(&self.literals, self.effort.strategy >= 8, &mut body);
        self.sequences_section(&mut body);
        let raw = &self.data[start..end];
        let (kind, body) = if !first && raw.len() > 1 && raw.iter().all(|byte| *byte == raw[0]) {
            (1, vec![raw[0]])
        } else if body.len() >= raw.len() {
            (0, raw.to_vec())
        } else {
            (2, body)
        };
        if kind != 2 {
            // What's not written squeezed changes nothing the next block can take from this one.
            self.last = last_before;
            self.codes = codes_before;
        }
        let size = if kind == 1 { raw.len() } else { body.len() };
        let header = (size as u32) << 3 | kind << 1 | u32::from(last);
        out.extend_from_slice(&header.to_le_bytes()[..3]);
        out.extend_from_slice(&body);
        self.at = end;
        if !self.single && self.at >= 2 * self.window {
            let shift = self.window;
            self.data.drain(..shift);
            self.at -= shift;
            if let Some(finder) = &mut self.finder {
                finder.noted = finder.noted.saturating_sub(shift);
                for place in finder.head.iter_mut().chain(finder.chain.iter_mut()) {
                    *place = place.saturating_sub(shift as u32);
                }
            }
        }
    }

    /// Finds the sequences for what's from `start` to `end` the way zstd's two fastest strategies do, with no chains:
    /// a table of where the last few bytes with each hash were seen, and for the second strategy another of where
    /// each eight bytes were, which is tried first. The last offset is tried a byte ahead before either, a stretch
    /// with nothing found is gone through faster and faster, and after each match the next bytes are tried with the
    /// offset before last, as that's often where the next match is.
    #[inline(never)]
    fn find_quick(&mut self, start: usize, end: usize) {
        // Made once for each strategy, so that which it is isn't asked again for each place. Making one for each
        // length of the short hash too, as zstd does, was 6% faster at level 1 for 20 KB more of program.
        if self.effort.strategy >= 2 {
            self.find_quick_as::<true>(start, end);
        } else {
            self.find_quick_as::<false>(start, end);
        }
    }

    /// The same, with two tables or one.
    fn find_quick_as<const DOUBLE: bool>(&mut self, start: usize, end: usize) {
        let Some(mut finder) = self.finder.take() else { return };
        let double = DOUBLE;
        let short = finder.hash_bytes;
        let (long_bits, short_bits) = (64 - finder.hash_shift, finder.chain.len().trailing_zeros());
        let acceleration = if self.level < 0 { (-self.level) as usize } else { 0 };
        // Places this near the end of what's here can't be hashed, so what's after them is left as literals.
        let limit = end.min(self.data.len().saturating_sub(8));
        let window = if self.single { usize::MAX } else { self.window };
        let data = &self.data[..];
        let word = |at: usize| u64::from_le_bytes(data[at..at + 8].try_into().unwrap_or_default());
        let four = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap_or_default());
        let mut tables = Quick::<DOUBLE> { head: &mut finder.head, chain: &mut finder.chain, short, long_bits, short_bits };
        let mut last = self.last;
        let mut anchor = start;
        let mut at = start;
        while at < limit {
            let low = at.saturating_sub(window).max(1);
            let eight = word(at);
            let (long, near) = tables.swap(eight, at);
            let rep = last[0] as usize;
            let (mut begin, mut length, offset);
            if end - at > 4 && rep > 0 && rep <= (at + 1).min(window) && four(at + 1 - rep) == four(at + 1) {
                begin = at + 1;
                offset = rep;
                length = 4 + same_for(data, begin - rep + 4, begin + 4, end - begin - 4);
            } else if double && end - at >= 8 && long >= low && long < at && word(long) == eight {
                begin = at;
                offset = at - long;
                length = 8 + same_for(data, long + 8, at + 8, end - at - 8);
            } else if end - at >= 4 && near >= low && near < at && four(near) == four(at) {
                // A long match a byte later is better than a short one here, if there's one.
                let later = if double && at + 1 < limit && end - at > 8 { tables.long(word(at + 1)) } else { 0 };
                if later >= (at + 1).saturating_sub(window).max(1) && later <= at && word(later) == word(at + 1) {
                    begin = at + 1;
                    offset = begin - later;
                    length = 8 + same_for(data, later + 8, begin + 8, end - begin - 8);
                } else {
                    begin = at;
                    offset = at - near;
                    length = 4 + same_for(data, near + 4, at + 4, end - at - 4);
                }
            } else {
                at += ((at - anchor) >> 8) + 1 + acceleration;
                continue;
            }
            // A match can often start a little earlier than where it was found.
            while begin > anchor && begin > offset && data[begin - 1] == data[begin - 1 - offset] {
                begin -= 1;
                length += 1;
            }
            add_literals(&mut self.literals, data, anchor, begin);
            let literals = (begin - anchor) as u32;
            let value = offset_value(&mut last, offset as u32, literals);
            self.sequences.push(Sequence { literals, length: length as u32, value });
            let next = begin + length;
            // Places in what was matched are noted too: two, as zstd notes them, for the fastest, and the first and
            // last four for the next, which finds a percent or so more for a little more time, as matches often
            // start where an earlier one did.
            if next < limit {
                if double {
                    let first = (at + 5).min(next);
                    for place in (at + 1..first).chain((next - 4).max(first)..next) {
                        tables.swap(word(place), place);
                    }
                } else {
                    tables.swap(word(at + 2), at + 2);
                    tables.swap(word(next - 2), next - 2);
                }
            }
            at = next;
            anchor = next;
            // What comes straight after a match is often found again with the offset before last.
            while at < limit && end - at > 4 {
                let rep = last[1] as usize;
                if rep == 0 || rep > at.min(window) || four(at - rep) != four(at) {
                    break;
                }
                let length = 4 + same_for(data, at - rep + 4, at + 4, end - at - 4);
                let value = offset_value(&mut last, rep as u32, 0);
                self.sequences.push(Sequence { literals: 0, length: length as u32, value });
                tables.swap(word(at), at);
                at += length;
                anchor = at;
            }
        }
        self.literals.extend_from_slice(&self.data[anchor..end]);
        self.last = last;
        self.finder = Some(finder);
    }

    /// Finds the sequences for what's from `start` to `end`, as zstd's lazy search does, and how hard it looks
    /// depends on the level.
    fn find(&mut self, start: usize, end: usize) {
        let Some(mut finder) = self.finder.take() else { return };
        let effort = self.effort;
        let depth = match effort.strategy {
            0..=3 => 0,
            4 => 1,
            _ => 2,
        };
        let nice = if effort.strategy <= 2 { 32 } else { effort.target.max(16) * 4 } as usize;
        let acceleration = if self.level < 0 { (-self.level) as usize } else { 0 };
        let min = finder.hash_bytes as usize;
        // Places this near the end of what's here can't be hashed, so what's after them is left as literals.
        let limit = end.min(self.data.len().saturating_sub(8));
        let window = if self.single { usize::MAX } else { self.window };
        let data = &self.data;
        let mut found = Vec::new();
        let mut search = |finder: &mut Finder, at: usize| {
            found.clear();
            finder.search(data, at, end, nice, &mut found);
            found.last().copied().unwrap_or((0, 0))
        };
        let best_repeat = |last: &[u32; 3], at: usize, literals: usize| {
            repeats(data, last, at, end, window, literals).into_iter().max_by_key(|(length, _)| *length).unwrap_or((0, 0))
        };
        // How much a match is worth: its length, less what saying its offset costs.
        let gain = |length: usize, offset: usize, last: &[u32; 3]| -> i64 {
            let cost = if last.contains(&(offset as u32)) { 1 } else { highest_bit(offset as u32 + 3) };
            length as i64 * 4 - i64::from(cost)
        };
        let mut anchor = start;
        let mut at = start;
        let mut last = self.last;
        while at < limit {
            let (mut length, mut offset) = best_repeat(&last, at, at - anchor);
            if length < 4 {
                length = 0;
            }
            let (found, found_offset) = search(&mut finder, at);
            if found >= min && gain(found, found_offset, &last) > gain(length, offset, &last) + 1 {
                (length, offset) = (found, found_offset);
            }
            if length < min {
                at += ((at - anchor) >> 8) + 1 + acceleration;
                continue;
            }
            let mut begin = at;
            let mut next = at;
            // A match that starts a byte or two later may be better, and then one after that.
            'better: while depth > 0 && next + 1 < limit && length < nice {
                for bonus in [4, 7].into_iter().take(depth) {
                    next += 1;
                    if next >= limit {
                        break 'better;
                    }
                    let (with_last, last_offset) = best_repeat(&last, next, next - anchor);
                    if with_last >= 4 && gain(with_last, last_offset, &last) > gain(length, offset, &last) + 1 {
                        (length, offset, begin) = (with_last, last_offset, next);
                    }
                    let (found, found_offset) = search(&mut finder, next);
                    if found >= 4 && gain(found, found_offset, &last) > gain(length, offset, &last) + bonus {
                        (length, offset, begin) = (found, found_offset, next);
                        continue 'better;
                    }
                }
                break;
            }
            // A match can often start a little earlier than where it was found.
            while begin > anchor && begin > offset && data[begin - 1] == data[begin - 1 - offset] {
                begin -= 1;
                length += 1;
            }
            self.literals.extend_from_slice(&data[anchor..begin]);
            let literals = (begin - anchor) as u32;
            let value = offset_value(&mut last, offset as u32, literals);
            self.sequences.push(Sequence { literals, length: length as u32, value });
            at = begin + length;
            anchor = at;
            if effort.strategy == 1 {
                // The fastest levels don't note every place inside a match.
                finder.noted = finder.noted.max(at.saturating_sub(2).min(limit));
            }
        }
        self.literals.extend_from_slice(&data[anchor..end]);
        self.last = last;
        self.finder = Some(finder);
    }

    /// Finds the sequences for what's from `start` to `end` the way zstd's slowest levels do: for a stretch at a
    /// time, the cheapest way to each place in it, by what each literal and each match would cost to write, given
    /// how often each symbol came in the block before.
    fn find_cheapest(&mut self, start: usize, end: usize) {
        let Some(mut finder) = self.finder.take() else { return };
        let nice = (self.effort.target as usize).clamp(16, 4095);
        let min = finder.hash_bytes as usize;
        let limit = end.min(self.data.len().saturating_sub(8));
        let window = if self.single { usize::MAX } else { self.window };
        let data = &self.data;
        // The first block has only its own bytes to go by.
        let mut counts = vec![0u32; 256];
        for byte in &data[start..end] {
            counts[usize::from(*byte)] += 1;
        }
        let literal = prices(if self.stats[3].is_empty() { &counts } else { &self.stats[3] });
        let codes: [Vec<f32>; 3] = std::array::from_fn(|kind| {
            let usual: Vec<u32> = KINDS[kind].0.iter().map(|count| count.unsigned_abs().into()).collect();
            prices(if self.stats[kind].is_empty() { &usual } else { &self.stats[kind] })
        });
        let code_price = |kind: usize, code: u8| codes[kind].get(usize::from(code)).copied().unwrap_or(12.0);
        let run_price = |count: u32| {
            let code = literal_code(count);
            code_price(0, code) + f32::from(LL_BITS[usize::from(code)])
        };
        let match_price = |value: u32, length: usize| {
            let (of, ml) = (highest_bit(value) as u8, match_code(length as u32));
            code_price(1, of) + f32::from(of) + code_price(2, ml) + f32::from(ML_BITS[usize::from(ml)]) + run_price(0)
        };
        let mut steps: Vec<Step> = Vec::new();
        let mut found = Vec::new();
        let mut tries = Vec::new();
        let mut anchor = start;
        let mut at = start;
        let mut last = self.last;
        while at < limit {
            steps.clear();
            steps.push(Step { price: 0.0, length: 0, offset: 0, literals: (at - anchor) as u32, last });
            let span = (limit - at).min(4096);
            let mut stop = None;
            for step in 0..span {
                let here = steps[step];
                let place = at + step;
                let more = here.literals + 1;
                let price = here.price + literal[usize::from(data[place])] + run_price(more) - run_price(here.literals);
                relax(&mut steps, step + 1, Step { price, length: 0, offset: 0, literals: more, last: here.last });
                found.clear();
                finder.search(data, place, end, nice, &mut found);
                let repeated = repeats(data, &here.last, place, end, window, here.literals as usize);
                // Each length a match reaches could be where it ends. One long enough is taken as it is, and the
                // stretch ends with it. Each match found is longer than the one before, so it's only tried for the
                // lengths that one didn't reach.
                tries.clear();
                let long = repeated.into_iter().chain(found.last().copied()).find(|(length, _)| *length >= nice);
                if let Some((length, offset)) = long {
                    tries.push((length, length, offset));
                } else {
                    tries.extend(repeated.iter().map(|(length, offset)| (3, *length, *offset)));
                    let mut shortest = min;
                    for (length, offset) in found.iter().copied() {
                        tries.push((shortest, length, offset));
                        shortest = shortest.max(length + 1);
                    }
                }
                for (from, length, offset) in tries.iter().copied() {
                    let mut after = here.last;
                    let value = offset_value(&mut after, offset as u32, here.literals);
                    for each in from..=length {
                        let price = here.price + match_price(value, each);
                        relax(&mut steps, step + each, Step { price, length: each as u32, offset: offset as u32, literals: 0, last: after });
                    }
                }
                if let Some((length, _)) = long {
                    stop = Some(step + length);
                    break;
                }
            }
            // The cheapest way to the furthest place reached, followed back.
            let reached = stop.unwrap_or(steps.len() - 1);
            let mut step = reached;
            let mut path = Vec::new();
            while step > 0 {
                let each = steps[step];
                step -= each.length.max(1) as usize;
                if each.length > 0 {
                    path.push((at + step, each.length, each.offset));
                }
            }
            for (begin, length, offset) in path.into_iter().rev() {
                self.literals.extend_from_slice(&data[anchor..begin]);
                let literals = (begin - anchor) as u32;
                let value = offset_value(&mut last, offset, literals);
                self.sequences.push(Sequence { literals, length, value });
                anchor = begin + length as usize;
            }
            at += reached;
        }
        self.literals.extend_from_slice(&data[anchor..end]);
        self.last = last;
        self.finder = Some(finder);
    }

    #[inline(never)]
    fn sequences_section(&mut self, out: &mut Vec<u8>) {
        let count = self.sequences.len();
        if count < 128 {
            out.push(count as u8);
        } else if count < 0x7f00 {
            out.extend_from_slice(&[(count >> 8) as u8 + 128, count as u8]);
        } else {
            out.push(255);
            out.extend_from_slice(&((count - 0x7f00) as u16).to_le_bytes());
        }
        if count == 0 {
            return;
        }
        let mut symbols = [Vec::with_capacity(count), Vec::with_capacity(count), Vec::with_capacity(count)];
        for sequence in &self.sequences {
            symbols[0].push(literal_code(sequence.literals));
            symbols[1].push(highest_bit(sequence.value) as u8);
            symbols[2].push(match_code(sequence.length));
        }
        let modes_at = out.len();
        out.push(0);
        for (kind, (_, _, most_symbol, most_log)) in KINDS.into_iter().enumerate() {
            let mut counts = vec![0u32; most_symbol + 1];
            for symbol in &symbols[kind] {
                counts[usize::from(*symbol)] += 1;
            }
            let used = counts.iter().filter(|count| **count > 0).count();
            let biggest = counts.iter().rposition(|count| *count > 0).unwrap_or(0);
            let usual = &usual_coders()[kind];
            let log = table_log(most_log, count, biggest);
            let made = Coder::new(&normalize(&counts[..=biggest], log), log);
            let mut description = Vec::new();
            write_counts(&made.counts, log, &mut description);
            // The cheapest of the four ways, each with what its description costs.
            let mut choices: Vec<(f32, u8)> = Vec::new();
            if used == 1 {
                choices.push((8.0, 1));
            }
            if let Some(bits) = usual.cost(&counts) {
                choices.push((bits, 0));
            }
            if let Some(bits) = made.cost(&counts) {
                choices.push((bits + 8.0 * description.len() as f32, 2));
            }
            match &self.codes[kind] {
                Some(Code::One(symbol)) if used == 1 && counts[usize::from(*symbol)] > 0 => choices.push((0.0, 3)),
                Some(Code::Table(before)) => {
                    if let Some(bits) = before.cost(&counts) {
                        choices.push((bits, 3));
                    }
                }
                _ => {}
            }
            let mode = choices.iter().min_by(|a, b| a.0.total_cmp(&b.0)).map_or(2, |choice| choice.1);
            out[modes_at] |= mode << (6 - 2 * kind);
            match mode {
                0 => self.codes[kind] = Some(Code::Table(usual.clone())),
                1 => {
                    out.push(symbols[kind][0]);
                    self.codes[kind] = Some(Code::One(symbols[kind][0]));
                }
                2 => {
                    out.extend_from_slice(&description);
                    self.codes[kind] = Some(Code::Table(made));
                }
                _ => {}
            }
            self.stats[kind] = counts;
        }
        let coders: [Option<&Coder>; 3] = std::array::from_fn(|kind| match &self.codes[kind] {
            Some(Code::Table(coder)) => Some(coder),
            _ => None,
        });
        // Written from the last sequence back, so it's read from the first on.
        let mut writer = BitWriter::default();
        writer.out.reserve(4 * count + 16);
        #[inline(always)]
        fn extra(writer: &mut BitWriter, sequence: Sequence, [ll, of, ml]: [u8; 3]) {
            let (ll, of, ml) = (usize::from(ll), u32::from(of), usize::from(ml));
            writer.put(sequence.literals - LL_BASE[ll], u32::from(LL_BITS[ll]));
            writer.put(sequence.length - ML_BASE[ml], u32::from(ML_BITS[ml]));
            writer.put(sequence.value - (1 << of), of);
        }
        let codes_at = |at: usize| [symbols[0][at], symbols[1][at], symbols[2][at]];
        let mut states = [0u32; 3];
        for kind in 0..3 {
            if let Some(coder) = coders[kind] {
                states[kind] = coder.start(symbols[kind][count - 1]);
            }
        }
        extra(&mut writer, self.sequences[count - 1], codes_at(count - 1));
        for (at, sequence) in self.sequences[..count - 1].iter().enumerate().rev() {
            let codes = codes_at(at);
            for kind in [1, 2, 0] {
                if let Some(coder) = coders[kind] {
                    coder.put(&mut states[kind], codes[kind], &mut writer);
                }
            }
            extra(&mut writer, *sequence, codes);
        }
        for kind in [2, 1, 0] {
            if let Some(coder) = coders[kind] {
                coder.finish(states[kind], &mut writer);
            }
        }
        close(writer, out);
    }
}

/// The tables zstd's two fastest strategies find matches through: for the second, where each eight bytes were last
/// seen, by their hash, and where the first few were; and for the first only the second of those.
struct Quick<'a, const DOUBLE: bool> {
    head: &'a mut [u32],
    chain: &'a mut [u32],
    short: u32,
    long_bits: u32,
    short_bits: u32,
}

impl<const DOUBLE: bool> Quick<'_, DOUBLE> {
    /// Where what's at `at`, whose first eight bytes are `eight`, was last seen by each table, which then say `at`.
    /// A hash is less than the table's length, which is a power of two, and masking it says so for nothing.
    #[inline(always)]
    fn swap(&mut self, eight: u64, at: usize) -> (usize, usize) {
        let (head_mask, chain_mask) = (self.head.len() - 1, self.chain.len() - 1);
        if DOUBLE {
            let (long, short) = (hash_of(eight, 8, self.long_bits) & head_mask, hash_of(eight, self.short, self.short_bits) & chain_mask);
            let found = (self.head[long] as usize, self.chain[short] as usize);
            self.head[long] = at as u32;
            self.chain[short] = at as u32;
            found
        } else {
            let short = hash_of(eight, self.short, self.long_bits) & head_mask;
            let found = self.head[short] as usize;
            self.head[short] = at as u32;
            (0, found)
        }
    }

    /// Where eight bytes like these were last seen.
    #[inline(always)]
    fn long(&self, eight: u64) -> usize {
        self.head[hash_of(eight, 8, self.long_bits) & (self.head.len() - 1)] as usize
    }
}

/// A hash of `bits` bits of the first `bytes` of these eight.
#[inline(always)]
fn hash_of(eight: u64, bytes: u32, bits: u32) -> usize {
    ((eight << (64 - 8 * bytes)).wrapping_mul(0x9e37_79b1_85eb_ca87) >> (64 - bits)) as usize
}

/// A dictionary with tables of its own, as zstd's are: a Huffman code of seven bits for each of the first 128 bytes,
/// the usual FSE codes, the usual last offsets, and this content.
#[cfg(test)]
pub(super) fn dictionary_for_test(content: &[u8]) -> Vec<u8> {
    let mut out = super::zstd::DICTIONARY_MAGIC.to_le_bytes().to_vec();
    out.extend_from_slice(&7777u32.to_le_bytes());
    out.push(254);
    out.extend_from_slice(&[0x11; 63]);
    out.push(0x10);
    for kind in [1, 2, 0] {
        let (counts, log, _, _) = KINDS[kind];
        write_counts(counts, log, &mut out);
    }
    for last in [1u32, 4, 8] {
        out.extend_from_slice(&last.to_le_bytes());
    }
    out.extend_from_slice(content);
    out
}


#[cfg(test)]
mod tests {
    use super::super::zstd::resolve;
    use super::*;

    #[test]
    fn offsets_are_said_as_the_reader_will_take_them() {
        // Against trying each value with what the reader does, for offsets that are and aren't the last ones.
        let mut seed = 7u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            seed >> 16
        };
        for _ in 0..20_000 {
            let last = [1 + next() % 6, 1 + next() % 6, 1 + next() % 6];
            let (offset, literals) = (1 + next() % 8, next() % 2);
            let (mut tried, mut made) = (last, last);
            let value = (1..=3).find(|value| resolve(&mut tried.clone(), *value, literals) == offset).unwrap_or(offset + 3);
            resolve(&mut tried, value, literals);
            assert_eq!(offset_value(&mut made, offset, literals), value, "{last:?} {offset} {literals}");
            assert_eq!(made, tried);
        }
    }
}

#[cfg(test)]
mod codes {
    use super::*;

    #[test]
    fn lengths_have_the_codes_their_bases_give() {
        for length in 0..300_000 {
            assert_eq!(literal_code(length), code_of(&LL_BASE, length), "{length}");
            if length >= 3 {
                assert_eq!(match_code(length), code_of(&ML_BASE, length), "{length}");
            }
        }
    }
}
