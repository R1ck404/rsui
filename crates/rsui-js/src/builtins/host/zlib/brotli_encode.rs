//! Writing Brotli: earlier copies found through chains of where each four bytes were seen before, and the last
//! distances tried first, as they cost next to nothing to say again, each match weighed the way Brotli's own encoder
//! weighs them. Each meta-block has one prefix code for each of literals, commands and distances, made for it, or is
//! stored as it is when that's smaller. The quality says how hard to look. The dictionary isn't used.

use super::brotli::{CELL_COPY, CELL_INSERT, COPY_BASE, COPY_EXTRA, INSERT_BASE, INSERT_EXTRA, LENGTH_ORDER, context};
use super::deflate::same_for;
use super::huffman::{BitWriter, code_lengths, codes_of};

/// How much is written as one meta-block at most.
const BLOCK: usize = 1 << 18;
/// What a match has to be worth to be taken, and what each part of it is worth, in Brotli's own units.
const MIN_SCORE: u32 = 2020;
const LITERAL_SCORE: u32 = 135;
const BASE_SCORE: u32 = 1920;
const DISTANCE_PENALTY: u32 = 30;
/// How much better a match a byte later has to be for the byte to be written as a literal first.
const LAZY_GAIN: u32 = 175;

struct Command {
    insert: u32,
    copy: u32,
    distance: u32,
}

pub(super) struct Encoder {
    window_bits: u32,
    window_size: usize,
    /// How many places to look at for each match, how long one has to be to stop looking, and how many bytes later
    /// to look for a better one.
    chain: u32,
    nice: usize,
    lazy: bool,
    /// Whether two contexts of literals get codes of their own.
    contexts: bool,
    window: Vec<u8>,
    /// Where what's yet to be written starts in `window`.
    from: usize,
    /// The next place to note in the hash chains.
    next_insert: usize,
    head: Vec<u32>,
    prev: Vec<u32>,
    hash_shift: u32,
    ring_mask: usize,
    distances: [u32; 4],
    writer: BitWriter,
    started: bool,
    finished: bool,
}

/// Which code an insert or copy length has, by the table of where each starts.
fn length_code(base: &[u32; 24], length: u32) -> usize {
    base.iter().rposition(|start| *start <= length).unwrap_or(0)
}

/// The distance code and its extra bits for a distance that isn't one of the last ones, with no postfix or direct
/// codes.
fn distance_code(distance: u32) -> (usize, u32, u32) {
    let x = distance + 3;
    let bits = 31 - x.leading_zeros() - 1;
    let prefix = (x >> bits) & 1;
    (16 + 2 * (bits as usize - 1) + prefix as usize, bits, x - ((2 + prefix) << bits))
}

/// The command code for these insert and copy codes, and whether it means the last distance without saying so.
fn command_code(insert: usize, copy: usize, implicit: bool) -> usize {
    let cell = if implicit && insert < 8 && copy < 16 {
        copy >> 3
    } else {
        match (insert >> 3, copy >> 3) {
            (0, 0) => 2,
            (0, 1) => 3,
            (1, 0) => 4,
            (1, 1) => 5,
            (0, _) => 6,
            (_, 0) => 7,
            (1, _) => 8,
            (_, 1) => 9,
            _ => 10,
        }
    };
    debug_assert!(usize::from(CELL_INSERT[cell]) <= insert && usize::from(CELL_COPY[cell]) <= copy);
    cell * 64 + (insert & 7) * 8 + (copy & 7)
}

/// The run-length code for a prefix code's lengths, as Brotli writes it: what's repeated said once, then how many
/// more times in digits of repeat codes, highest first.
fn length_symbols(lengths: &[u8]) -> Vec<(u8, u8)> {
    let used = lengths.iter().rposition(|length| *length != 0).map_or(0, |at| at + 1);
    let mut out = Vec::new();
    let mut previous = 8u8;
    let mut at = 0;
    while at < used {
        let value = lengths[at];
        let mut run = 1;
        while at + run < used && lengths[at + run] == value {
            run += 1;
        }
        at += run;
        let (code, shift, mut left) = if value == 0 {
            if run == 11 {
                out.push((0, 0));
                run -= 1;
            }
            (17, 3, run)
        } else {
            if previous != value {
                out.push((value, 0));
                run -= 1;
            }
            if run == 7 {
                out.push((value, 0));
                run -= 1;
            }
            previous = value;
            (16, 2, run)
        };
        if left < 3 {
            out.extend(std::iter::repeat_n((value, 0), left));
            continue;
        }
        left -= 3;
        let start = out.len();
        loop {
            out.push((code, (left & ((1 << shift) - 1)) as u8));
            left >>= shift;
            if left == 0 {
                break;
            }
            left -= 1;
        }
        out[start..].reverse();
    }
    out
}

/// Writes a prefix code for symbols of these lengths in an alphabet of `size`, the short way if four or fewer are
/// used. A code with nothing used is written as one with only the first symbol.
fn write_code(writer: &mut BitWriter, lengths: &[u8], size: usize) {
    let mut used: Vec<usize> = (0..lengths.len()).filter(|at| lengths[*at] != 0).collect();
    if used.len() <= 4 {
        let width = usize::BITS - (size - 1).leading_zeros();
        if used.is_empty() {
            used.push(0);
        }
        used.sort_by_key(|symbol| (lengths[*symbol], *symbol));
        writer.put(1, 2);
        writer.put(used.len() as u32 - 1, 2);
        for symbol in &used {
            writer.put(*symbol as u32, width);
        }
        if used.len() == 4 {
            writer.put(u32::from(lengths[used[0]] == 1), 1);
        }
        return;
    }
    let symbols = length_symbols(lengths);
    let mut counts = [0u32; 18];
    for (symbol, _) in &symbols {
        counts[usize::from(*symbol)] += 1;
    }
    let length_lengths = code_lengths(&counts, 5);
    let only_one = counts.iter().filter(|count| **count > 0).count() == 1;
    // The lengths of the code for lengths, each in a code of its own, of which these are the bits and how many.
    const STATIC: [(u32, u32); 6] = [(0, 2), (7, 4), (3, 3), (2, 2), (1, 2), (15, 4)];
    let skip = if length_lengths[LENGTH_ORDER[0]] == 0 && length_lengths[LENGTH_ORDER[1]] == 0 {
        if length_lengths[LENGTH_ORDER[2]] == 0 { 3 } else { 2 }
    } else {
        0
    };
    let last = if only_one { 17 } else { LENGTH_ORDER.iter().rposition(|place| length_lengths[*place] != 0).unwrap_or(0) };
    writer.put(skip, 2);
    for place in &LENGTH_ORDER[skip as usize..=last] {
        let (bits, count) = STATIC[usize::from(length_lengths[*place])];
        writer.put(bits, count);
    }
    let codes = codes_of(&length_lengths);
    for (symbol, extra) in symbols {
        let symbol = usize::from(symbol);
        if !only_one {
            writer.put(u32::from(codes[symbol]), u32::from(length_lengths[symbol]));
        }
        match symbol {
            16 => writer.put(u32::from(extra), 2),
            17 => writer.put(u32::from(extra), 3),
            _ => {}
        }
    }
}

/// The lengths symbols are written with, which are none at all for a code of one symbol.
fn emitted(lengths: &[u8]) -> Vec<u8> {
    if lengths.iter().filter(|length| **length != 0).count() == 1 { vec![0; lengths.len()] } else { lengths.to_vec() }
}

/// About how many bits a code of these lengths takes to write, for choosing between ways.
fn code_cost(lengths: &[u8]) -> usize {
    let used = lengths.iter().filter(|length| **length != 0).count();
    if used <= 4 { 12 + used * 10 } else { 40 + length_symbols(lengths).len() * 4 }
}

impl Encoder {
    pub(super) fn new(quality: i32, window_bits: u32, _mode: u32, size_hint: usize) -> Self {
        let quality = quality.clamp(0, 11) as u32;
        let window_bits = if window_bits == 0 { 22 } else { window_bits.clamp(10, 24) };
        // A file that's known to be small needs no more chain than it has bytes.
        let mut ring_bits = window_bits.min(if quality >= 4 { 20 } else { 18 });
        if size_hint > 0 {
            ring_bits = ring_bits.min((usize::BITS - size_hint.leading_zeros()).max(10));
        }
        let hash_bits = match quality {
            0..=2 => 14,
            3..=6 => 16,
            _ => 17,
        };
        let chain = [1, 2, 4, 8, 16, 24, 32, 64, 128, 256, 512, 1024][quality as usize];
        Self {
            window_bits,
            window_size: (1 << window_bits) - 16,
            chain,
            nice: [16, 24, 32, 48, 64, 96, 128, 160, 256, 384, 512, 1024][quality as usize],
            lazy: quality >= 4,
            contexts: quality >= 6,
            window: Vec::new(),
            from: 0,
            next_insert: 0,
            head: vec![0; 1 << hash_bits],
            prev: vec![0; 1 << ring_bits],
            hash_shift: 32 - hash_bits,
            ring_mask: (1 << ring_bits) - 1,
            distances: [16, 15, 11, 4],
            writer: BitWriter::default(),
            started: false,
            finished: false,
        }
    }

    /// Takes what's given, and gives out what's been written: on `1`, a flush, all of it, and on `2` the end too.
    pub(super) fn push(&mut self, input: &[u8], operation: u32, out: &mut Vec<u8>) {
        if self.finished {
            return;
        }
        if !self.started {
            self.started = true;
            match self.window_bits {
                16 => self.writer.put(0, 1),
                17 => self.writer.put(1, 7),
                bits if bits > 17 => self.writer.put(1 | (bits - 17) << 1, 4),
                bits => self.writer.put(1 | (bits - 8) << 4, 7),
            }
        }
        let mut input = input;
        loop {
            let room = BLOCK - (self.window.len() - self.from);
            let (now, later) = input.split_at(room.min(input.len()));
            self.window.extend_from_slice(now);
            input = later;
            if self.window.len() - self.from == BLOCK {
                self.meta_block(self.window.len());
            }
            if input.is_empty() {
                break;
            }
        }
        if operation != 0 && self.window.len() > self.from {
            self.meta_block(self.window.len());
        }
        match operation {
            2 => {
                self.writer.put(3, 2);
                self.writer.align();
                self.finished = true;
            }
            1 | 3 if !self.writer.aligned() => {
                // An empty metadata block, which only fills out the byte.
                self.writer.put(6, 6);
                self.writer.align();
            }
            _ => {}
        }
        out.append(&mut self.writer.out);
    }

    #[inline]
    fn hash(&self, place: usize) -> usize {
        let four = u32::from_le_bytes([
            self.window[place],
            self.window[place + 1],
            self.window[place + 2],
            self.window[place + 3],
        ]);
        (four.wrapping_mul(0x1e35_a7bd) >> self.hash_shift) as usize
    }

    /// Notes the places up to `until` in the chains, as far as there are four bytes to hash.
    fn insert_until(&mut self, until: usize) {
        let end = until.min(self.window.len().saturating_sub(3));
        // A long match is noted more sparsely the less hard it's looking, as each place costs a little.
        if self.chain < 16 && end > self.next_insert + 32 {
            self.next_insert = end - 16;
        }
        while self.next_insert < end {
            let place = self.next_insert;
            let hash = self.hash(place);
            self.prev[place & self.ring_mask] = self.head[hash];
            self.head[hash] = place as u32 + 1;
            self.next_insert += 1;
        }
    }

    /// The best match for what's at `at`, as its score, length and distance, looking no further than `end`.
    fn find(&mut self, at: usize, end: usize) -> (u32, usize, usize) {
        let most = end - at;
        let mut best = (MIN_SCORE, 0, 0);
        if most < 4 {
            return best;
        }
        // The last distances first, as they cost the least to say.
        let tries = if self.chain >= 4 { 4 } else { 1 };
        for which in 0..tries {
            let distance = self.distances[3 - which] as usize;
            if distance > at || distance > self.window_size {
                continue;
            }
            let length = same_for(&self.window, at - distance, at, most);
            if length >= 2 + usize::from(which > 0) * 2 {
                let score = LITERAL_SCORE * length as u32 + BASE_SCORE + 15 - 39 * which as u32;
                if score > best.0 {
                    best = (score, length, distance);
                }
            }
        }
        self.insert_until(at);
        let hash = self.hash(at);
        let mut candidate = self.head[hash] as usize;
        self.prev[at & self.ring_mask] = candidate as u32;
        self.head[hash] = at as u32 + 1;
        self.next_insert = at + 1;
        let limit = at.saturating_sub(self.window_size.min(self.ring_mask));
        let mut chain = self.chain;
        let nice = self.nice.min(most);
        // What the loop reads is held here, as what it writes would otherwise have it all read again each time.
        let (window, prev, mask) = (&self.window[..], &self.prev[..], self.ring_mask);
        let four = |at: usize| u32::from_le_bytes(window[at..at + 4].try_into().unwrap_or_default());
        let two = |at: usize| u16::from_le_bytes([window[at], window[at + 1]]);
        let start = four(at);
        while candidate > limit && chain > 0 {
            let place = candidate - 1;
            chain -= 1;
            // Only a longer match can be better than the best so far, and one has the same first four bytes and the
            // same two at the end of the best, which rule out most places.
            let reach = best.1.min(most - 1);
            if four(place) == start && (reach == 0 || two(place + reach - 1) == two(at + reach - 1)) {
                let length = same_for(window, place, at, most);
                if length >= 4 {
                    let distance = at - place;
                    let score = BASE_SCORE + LITERAL_SCORE * length as u32
                        - DISTANCE_PENALTY * (usize::BITS - 1 - distance.leading_zeros());
                    if score > best.0 {
                        best = (score, length, distance);
                        if length >= nice {
                            break;
                        }
                    }
                }
            }
            let next = prev[place & mask] as usize;
            if next >= candidate {
                break;
            }
            candidate = next;
        }
        best
    }

    /// Writes what's from `from` to `end` as a meta-block.
    fn meta_block(&mut self, end: usize) {
        let start = self.from;
        let start_ring = self.distances;
        let mut commands = Vec::new();
        let mut at = start;
        let mut literal_from = start;
        while at + 4 <= end {
            let mut found = self.find(at, end);
            if found.1 == 0 {
                // A long way with nothing found is likely more of what can't be squeezed, which is gone through
                // faster, as Brotli's own encoder does.
                let quiet = at - literal_from;
                let patience = if self.chain >= 256 { 512 } else { 64 };
                at += if quiet > 5 * patience {
                    16
                } else if quiet > patience {
                    8
                } else {
                    1
                };
                continue;
            }
            if self.lazy {
                for _ in 0..2 {
                    if at + 5 > end {
                        break;
                    }
                    let next = self.find(at + 1, end);
                    if next.0 < found.0 + LAZY_GAIN {
                        break;
                    }
                    at += 1;
                    found = next;
                }
            }
            let (_, length, distance) = found;
            commands.push(Command { insert: (at - literal_from) as u32, copy: length as u32, distance: distance as u32 });
            self.note_distance(distance as u32);
            at += length;
            literal_from = at;
            self.insert_until(at);
        }
        if literal_from < end {
            commands.push(Command { insert: (end - literal_from) as u32, copy: 0, distance: 0 });
        }
        self.write_meta(start, end, &commands, start_ring);
        self.from = end;
        // What's too far back to be referred to is forgotten, by whole turns of the chains so that where each place
        // is in them stays the same.
        let keep = self.window_size + 1;
        if self.window.len() > 2 * keep.max(BLOCK) {
            let drop = (self.window.len() - keep) & !self.ring_mask;
            if drop > 0 {
                self.window.drain(..drop);
                self.from -= drop;
                self.next_insert -= drop;
                let drop = drop as u32;
                for place in self.head.iter_mut().chain(self.prev.iter_mut()) {
                    *place = place.saturating_sub(drop);
                }
            }
        }
    }

    /// Keeps the last distances as the decoder will, which is what finding them again relies on.
    fn note_distance(&mut self, distance: u32) {
        if distance != self.distances[3] {
            self.distances = [self.distances[1], self.distances[2], self.distances[3], distance];
        }
    }

    fn write_meta(&mut self, start: usize, end: usize, commands: &[Command], start_ring: [u32; 4]) {
        let length = end - start;
        // Each command as its code and what follows it, with the distance code worked out against the last
        // distances as the decoder will have them.
        let mut ring = start_ring;
        let mut literal_counts = [[0u32; 256]; 2];
        let mut command_counts = [0u32; 704];
        let mut distance_counts = [0u32; 64];
        let mut coded = Vec::with_capacity(commands.len());
        let mut at = start;
        for (index, command) in commands.iter().enumerate() {
            let insert_code = length_code(&INSERT_BASE, command.insert);
            let last = index + 1 == commands.len() && command.copy == 0;
            let copy_code = length_code(&COPY_BASE, command.copy.max(2));
            let (distance, distance_extra) = if last {
                (None, (0, 0))
            } else if command.distance == ring[3] {
                (Some(0), (0, 0))
            } else {
                let cached = (1..4).find(|which| ring[3 - which] == command.distance);
                let found = match cached {
                    Some(which) => (which, 0, 0),
                    None => distance_code(command.distance),
                };
                ring = [ring[1], ring[2], ring[3], command.distance];
                (Some(found.0), (found.1, found.2))
            };
            let implicit = distance == Some(0) || last;
            let code = command_code(insert_code, copy_code, implicit);
            let explicit = !last && code >= 128;
            command_counts[code] += 1;
            for literal in at..at + command.insert as usize {
                literal_counts[self.literal_context(literal)][usize::from(self.window[literal])] += 1;
            }
            at += command.insert as usize + command.copy as usize;
            if explicit && let Some(code) = distance {
                distance_counts[code] += 1;
            }
            coded.push((code, insert_code, copy_code, if explicit { distance } else { None }, distance_extra));
        }
        let contexts = self.contexts && literal_counts[1].iter().any(|count| *count > 0);
        let literal_lengths: Vec<Vec<u8>> = if contexts {
            literal_counts.iter().map(|counts| code_lengths(counts, 15)).collect()
        } else {
            let all: Vec<u32> = (0..256).map(|byte| literal_counts[0][byte] + literal_counts[1][byte]).collect();
            vec![code_lengths(&all, 15)]
        };
        let command_lengths = code_lengths(&command_counts, 15);
        let distance_lengths = code_lengths(&distance_counts, 15);
        // Whether it's smaller as it is.
        let mut bits = 64 + code_cost(&command_lengths) + code_cost(&distance_lengths);
        for (lengths, counts) in literal_lengths.iter().zip(if contexts { &literal_counts[..] } else { &literal_counts[..1] }) {
            bits += code_cost(lengths);
            if contexts {
                bits += counts.iter().zip(lengths).map(|(count, length)| *count as usize * usize::from(*length)).sum::<usize>();
            }
        }
        if !contexts {
            bits += (0..256)
                .map(|byte| (literal_counts[0][byte] + literal_counts[1][byte]) as usize * usize::from(literal_lengths[0][byte]))
                .sum::<usize>();
        }
        for (code, insert_code, copy_code, distance, (extra_bits, _)) in &coded {
            bits += usize::from(command_lengths[*code])
                + usize::from(INSERT_EXTRA[*insert_code])
                + usize::from(COPY_EXTRA[*copy_code]);
            if let Some(distance) = distance {
                bits += usize::from(distance_lengths[*distance]) + *extra_bits as usize;
            }
        }
        let nibbles = (usize::BITS - (length - 1).leading_zeros()).div_ceil(4).max(4);
        self.writer.put(0, 1);
        self.writer.put(nibbles - 4, 2);
        // Up to 24 bits of length, written as two parts so neither is more than a `put` takes.
        let size = (length - 1) as u32;
        self.writer.put(size & 0xfff, 12);
        self.writer.put(size >> 12, 4 * nibbles - 12);
        if bits > length * 8 + 16 {
            self.writer.put(1, 1);
            self.writer.align();
            self.writer.out.extend_from_slice(&self.window[start..end]);
            // What's stored doesn't change the last distances, so they go back to what they were.
            self.distances = start_ring;
            return;
        }
        self.writer.put(0, 1);
        // One block type of each, no postfix or direct distance codes, and literals' context from the UTF-8 mode
        // when there are two codes for them.
        self.writer.put(0, 3);
        self.writer.put(0, 6);
        self.writer.put(2, 2);
        if contexts {
            // Two trees, for literals after a letter or digit and the rest, as a map that's run-length coded.
            self.writer.put(1, 1);
            self.writer.put(0, 3);
            self.write_literal_map();
        } else {
            self.writer.put(0, 1);
        }
        self.writer.put(0, 1);
        for lengths in &literal_lengths {
            write_code(&mut self.writer, lengths, 256);
        }
        write_code(&mut self.writer, &command_lengths, 704);
        write_code(&mut self.writer, &distance_lengths, 64);
        // A code with only one symbol takes no bits for it.
        let literal_lengths: Vec<Vec<u8>> = literal_lengths.iter().map(|lengths| emitted(lengths)).collect();
        let command_lengths = emitted(&command_lengths);
        let distance_lengths = emitted(&distance_lengths);
        let literal_codes: Vec<Vec<u16>> = literal_lengths.iter().map(|lengths| codes_of(lengths)).collect();
        let command_codes = codes_of(&command_lengths);
        let distance_codes = codes_of(&distance_lengths);
        let mut at = start;
        for (command, (code, insert_code, copy_code, distance, (extra_bits, extra))) in commands.iter().zip(&coded) {
            self.writer.put(u32::from(command_codes[*code]), u32::from(command_lengths[*code]));
            self.writer.put(command.insert - INSERT_BASE[*insert_code], u32::from(INSERT_EXTRA[*insert_code]));
            self.writer.put(command.copy.max(2) - COPY_BASE[*copy_code], u32::from(COPY_EXTRA[*copy_code]));
            for literal in at..at + command.insert as usize {
                let tree = if contexts { self.literal_context(literal) } else { 0 };
                let byte = usize::from(self.window[literal]);
                self.writer.put(u32::from(literal_codes[tree][byte]), u32::from(literal_lengths[tree][byte]));
            }
            at += command.insert as usize + command.copy as usize;
            if let Some(distance) = distance {
                self.writer.put(u32::from(distance_codes[*distance]), u32::from(distance_lengths[*distance]));
                self.writer.put(*extra, *extra_bits);
            }
        }
    }

    /// Which of the two literal codes a literal uses: one after a letter, digit or the like, and one after the rest.
    fn literal_context(&self, at: usize) -> usize {
        let last = if at > 0 { self.window[at - 1] } else { 0 };
        let before = if at > 1 { self.window[at - 2] } else { 0 };
        usize::from(context(2, last, before) >= 32)
    }

    /// The map from the 64 UTF-8 contexts to the two codes: the first 32 to the first, and the rest to the second.
    fn write_literal_map(&mut self) {
        // A run-length prefix of 5, so that 32 zeros are one symbol, then a symbol for each 1. The symbols are 0,
        // a run of 32 zeros as 5, and 1 as 6, which is two plus 4: three symbols, written the short way.
        self.writer.put(1, 1);
        self.writer.put(4, 4);
        let lengths = {
            let mut lengths = [0u8; 7];
            lengths[5] = 1;
            lengths[6] = 1;
            lengths
        };
        write_code(&mut self.writer, &lengths, 7);
        let codes = codes_of(&lengths);
        self.writer.put(u32::from(codes[5]), 1);
        self.writer.put(0, 5);
        for _ in 0..32 {
            self.writer.put(u32::from(codes[6]), 1);
        }
        self.writer.put(0, 1);
    }
}
