//! Undoing DEFLATE as it comes, a piece at a time, inside a zlib or gzip wrapper or neither, the way the format's
//! specification (RFC 1951) describes it. Whatever runs out partway is read again from where it began once there's
//! more, so the pieces can be split anywhere.

use super::huffman::{Bits, Stop, bad};
use super::{Wrap, adler32_from, crc32};

pub(super) const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258,
];
pub(super) const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
pub(super) const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
pub(super) const DISTANCE_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/// The order the lengths of the code for code lengths come in.
pub(super) const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// How far back DEFLATE can look.
const WINDOW: usize = 32 * 1024;

/// What an entry of a [`Table`] is. The lowest five bits are how many bits it takes in all, its code and the extra
/// bits after it, the next four how many of those are its code, then what kind it is, and the top half its value: a
/// literal, where lengths or distances of its code start, or where a second table is.
const LITERAL: u32 = 1 << 9;
/// A length or a distance, whose extra bits are added to its value.
const SIZED: u32 = 1 << 10;
const END: u32 = 1 << 11;
/// The entry is for a second table, for codes longer than the first takes, and its lowest bits say how many bits
/// past the first table's choose an entry of it.
const LINK: u32 = 1 << 12;
/// A symbol that has a code but means nothing. An entry with none of these is for a code that isn't in the table.
const MEANINGLESS: u32 = 1 << 13;

/// A code to read: each entry is what a symbol means and how many bits it takes, found from the next bits.
#[derive(Default)]
struct Table {
    entries: Vec<u32>,
    root: u32,
}

impl Table {
    /// Makes this the table for a code of these lengths, with `meaning` giving each symbol's kind and value, and how
    /// many extra bits follow it. A code can't have more codes than there's room for, but one that has fewer is
    /// allowed, and reading what's missing fails then.
    fn build(&mut self, lengths: &[u8], root: u32, meaning: fn(usize) -> u32) -> bool {
        let mut counts = [0u32; 16];
        for length in lengths {
            let Some(count) = counts.get_mut(usize::from(*length)) else {
                return false;
            };
            *count += 1;
        }
        counts[0] = 0;
        let room: i64 = (1..16).map(|bits| i64::from(counts[bits]) << (15 - bits)).sum();
        if room > 1 << 15 {
            return false;
        }
        let mut next = [0u32; 16];
        let mut code = 0;
        for bits in 1..16 {
            code = (code + counts[bits - 1]) << 1;
            next[bits] = code;
        }
        let size = 1usize << root;
        let mask = size - 1;
        self.root = root;
        self.entries.clear();
        self.entries.resize(size, 0);
        // Codes longer than the first table takes share its entry for their first bits, which links to a second table
        // big enough for the longest of them.
        if counts[root as usize + 1..].iter().any(|count| *count > 0) {
            let mut longest = [0u8; 1 << 11];
            let mut place = next;
            for length in lengths {
                let bits = u32::from(*length);
                if bits > root {
                    let code = place[bits as usize].reverse_bits() >> (32 - bits);
                    place[bits as usize] += 1;
                    let first = code as usize & mask;
                    longest[first] = longest[first].max(*length);
                }
            }
            for (first, length) in longest[..size].iter().enumerate() {
                if *length != 0 {
                    let sub = u32::from(*length) - root;
                    self.entries[first] = (self.entries.len() as u32) << 16 | LINK | sub;
                    self.entries.resize(self.entries.len() + (1 << sub), 0);
                }
            }
        }
        for (symbol, length) in lengths.iter().enumerate() {
            let bits = u32::from(*length);
            if bits == 0 {
                continue;
            }
            let code = (next[bits as usize].reverse_bits() >> (32 - bits)) as usize;
            next[bits as usize] += 1;
            let entry = meaning(symbol) + (bits << 5) + bits;
            if bits <= root {
                let mut at = code;
                while at < size {
                    self.entries[at] = entry;
                    at += 1 << bits;
                }
            } else {
                let link = self.entries[code & mask];
                let (start, sub) = ((link >> 16) as usize, link & 31);
                let mut at = code >> root;
                while at < 1 << sub {
                    self.entries[start + at] = entry;
                    at += 1 << (bits - root);
                }
            }
        }
        true
    }

    /// The entry for what the next bits are, which may not all be there.
    #[inline(always)]
    fn find(&self, held: u64) -> u32 {
        let entry = self.entries[(held & ((1 << self.root) - 1)) as usize];
        if entry & LINK == 0 {
            return entry;
        }
        self.entries[(entry >> 16) as usize + ((held >> self.root) & ((1 << (entry & 31)) - 1)) as usize]
    }
}

/// What a literal or length symbol means.
fn literal_meaning(symbol: usize) -> u32 {
    match symbol {
        0..256 => LITERAL | (symbol as u32) << 16,
        256 => END,
        257..286 => SIZED | u32::from(LENGTH_BASE[symbol - 257]) << 16 | u32::from(LENGTH_EXTRA[symbol - 257]),
        _ => MEANINGLESS,
    }
}

/// What a symbol of the code for code lengths means, which is itself.
fn length_meaning(symbol: usize) -> u32 {
    LITERAL | (symbol as u32) << 16
}

fn distance_meaning(symbol: usize) -> u32 {
    match symbol {
        0..30 => SIZED | u32::from(DISTANCE_BASE[symbol]) << 16 | u32::from(DISTANCE_EXTRA[symbol]),
        _ => MEANINGLESS,
    }
}

/// How many bits the first tables take, which are the most most codes are.
const LITERAL_ROOT: u32 = 11;
const DISTANCE_ROOT: u32 = 9;

/// Reads a symbol, giving its entry and its value with its extra bits added, or `Short` if it isn't all there.
#[inline]
fn read(bits: &mut Bits, table: &Table, wrong: &'static str) -> Result<(u32, u32), Stop> {
    if bits.count < 56 {
        bits.refill();
    }
    let entry = table.find(bits.held);
    if entry & (LITERAL | SIZED | END) == 0 {
        return Err(if entry & MEANINGLESS == 0 && bits.count < 15 && bits.at == bits.input.len() { Stop::Short } else { bad(wrong) });
    }
    let total = entry & 31;
    if total > bits.count {
        return Err(Stop::Short);
    }
    let value = (entry >> 16) + ((bits.held & ((1 << total) - 1)) >> ((entry >> 5) & 15)) as u32;
    bits.held >>= total;
    bits.count -= total;
    Ok((entry, value))
}

enum State {
    Header,
    Block,
    Stored(usize),
    Codes,
    Trailer,
    Done,
}

fn fixed_tables() -> &'static (Table, Table) {
    static FIXED: std::sync::OnceLock<(Table, Table)> = std::sync::OnceLock::new();
    FIXED.get_or_init(|| {
        let mut lengths = [0u8; 288];
        lengths[..144].fill(8);
        lengths[144..256].fill(9);
        lengths[256..280].fill(7);
        lengths[280..].fill(8);
        let mut tables = (Table::default(), Table::default());
        tables.0.build(&lengths, LITERAL_ROOT, literal_meaning);
        tables.1.build(&[5; 30], DISTANCE_ROOT, distance_meaning);
        tables
    })
}

pub(super) struct Inflater {
    bits: Bits,
    wrap: Wrap,
    state: State,
    /// What's come out: at least the last 32 KB of it, for what refers back, and what's yet to be given out.
    window: Vec<u8>,
    /// How much of `window` has been given out.
    sent: usize,
    /// How much of `window` is counted in `check`.
    checked: usize,
    check: u32,
    /// How many bytes this gzip member has come to, for its trailer.
    length: u32,
    last: bool,
    /// The codes of the block being read, unless it has the fixed ones. They're kept from block to block so their
    /// room is too.
    fixed: bool,
    literals: Table,
    distances: Table,
    dictionary: Option<Vec<u8>>,
    /// Nothing more is tried until there's this much unread, after a part ran out, so that a long part that comes
    /// a byte at a time isn't read again from its start for every byte.
    wait: usize,
}

impl Inflater {
    pub(super) fn new(wrap: Wrap, dictionary: Option<Vec<u8>>) -> Self {
        let mut made = Self {
            bits: Bits::default(),
            wrap,
            state: State::Header,
            window: Vec::new(),
            sent: 0,
            checked: 0,
            check: 0,
            length: 0,
            last: false,
            fixed: true,
            literals: Table::default(),
            distances: Table::default(),
            dictionary,
            wait: 0,
        };
        made.begin();
        made
    }

    /// Starts on another gzip member, which can't refer back to the one before, though what that undid may not have
    /// been given out yet.
    fn begin(&mut self) {
        self.checked = self.window.len();
        self.length = 0;
        self.last = false;
        self.check = if matches!(self.wrap, Wrap::Gzip) { 0 } else { 1 };
        self.state = State::Header;
        if matches!(self.wrap, Wrap::Raw) {
            self.state = State::Block;
            if let Some(dictionary) = &self.dictionary {
                self.window.extend_from_slice(dictionary);
                self.sent = self.window.len();
                self.checked = self.window.len();
            }
        }
    }

    pub(super) fn reset(&mut self) {
        self.bits = Bits::default();
        self.wait = 0;
        self.window.clear();
        self.sent = 0;
        self.begin();
    }

    pub(super) fn ended(&self) -> bool {
        matches!(self.state, State::Done)
    }

    /// Reads what's come, giving out what that undoes. `finish` says nothing more is coming, which makes a stream that
    /// hasn't ended wrong.
    pub(super) fn push(&mut self, input: &[u8], finish: bool, out: &mut Vec<u8>) -> Result<(), Stop> {
        if !matches!(self.state, State::Done) {
            self.bits.input.extend_from_slice(input);
        }
        let unread = self.bits.input.len() - self.bits.at;
        let went = if unread >= self.wait || finish { self.run() } else { Err(Stop::Short) };
        self.give(out);
        match went {
            Ok(()) => {
                self.wait = 0;
                Ok(())
            }
            Err(Stop::Short) => {
                if finish {
                    return Err(Stop::Bad("Z_BUF_ERROR", "unexpected end of file"));
                }
                let unread = self.bits.input.len() - self.bits.at;
                if self.wait <= unread {
                    self.wait = unread.saturating_mul(2).max(64);
                }
                Ok(())
            }
            Err(stop) => Err(stop),
        }
    }

    /// Gives out what's new, and forgets what's too far back to be needed.
    fn give(&mut self, out: &mut Vec<u8>) {
        self.count_new();
        if self.sent == 0 && out.is_empty() && self.window.len() > 2 * WINDOW {
            // All of it is new, as it is when everything comes at once, so it's handed over rather than copied, and
            // only what can still be referred back to is kept.
            let keep = self.window[self.window.len() - WINDOW..].to_vec();
            *out = std::mem::replace(&mut self.window, keep);
            self.sent = WINDOW;
            self.checked = WINDOW;
        } else {
            out.extend_from_slice(&self.window[self.sent..]);
            self.sent = self.window.len();
            if self.window.len() > 4 * WINDOW {
                let drop = self.window.len() - WINDOW;
                self.window.drain(..drop);
                self.sent -= drop;
                self.checked -= drop;
            }
        }
        self.bits.forget_read();
    }

    fn count_new(&mut self) {
        let new = &self.window[self.checked..];
        self.check = match self.wrap {
            Wrap::Gzip => crc32(new, self.check),
            Wrap::Zlib => adler32_from(new, self.check),
            _ => self.check,
        };
        self.length = self.length.wrapping_add(new.len() as u32);
        self.checked = self.window.len();
    }

    /// Reads all it can, saying `Short` if what's come so far ends partway.
    fn run(&mut self) -> Result<(), Stop> {
        loop {
            let mark = self.bits.mark();
            let went = match self.state {
                State::Done => {
                    self.bits.at = self.bits.input.len();
                    return Ok(());
                }
                State::Header => self.header(),
                State::Block => self.block(),
                State::Stored(left) => {
                    self.bits.to_byte();
                    let bytes = self.bits.bytes(left);
                    if bytes.is_empty() {
                        return Err(Stop::Short);
                    }
                    let got = bytes.len();
                    self.window.extend_from_slice(bytes);
                    self.state = if got == left { self.after_block() } else { State::Stored(left - got) };
                    Ok(())
                }
                State::Codes => self.symbols(),
                State::Trailer => self.trailer(),
            };
            if let Err(stop) = went {
                if stop == Stop::Short {
                    self.bits.restore(mark);
                }
                return Err(stop);
            }
        }
    }

    fn after_block(&self) -> State {
        if !self.last {
            State::Block
        } else if matches!(self.wrap, Wrap::Raw) {
            State::Done
        } else {
            State::Trailer
        }
    }

    fn byte(&mut self) -> Result<u8, Stop> {
        self.bits.take(8).map(|byte| byte as u8)
    }

    #[inline(never)]
    fn header(&mut self) -> Result<(), Stop> {
        if matches!(self.wrap, Wrap::Auto) {
            let mark = self.bits.mark();
            let first = (self.byte()?, self.byte()?);
            self.bits.restore(mark);
            self.wrap = if first == (0x1f, 0x8b) { Wrap::Gzip } else { Wrap::Zlib };
            self.check = if matches!(self.wrap, Wrap::Gzip) { 0 } else { 1 };
        }
        if matches!(self.wrap, Wrap::Gzip) {
            if (self.byte()?, self.byte()?) != (0x1f, 0x8b) {
                return Err(bad("incorrect header check"));
            }
            if self.byte()? != 8 {
                return Err(bad("unknown compression method"));
            }
            let flags = self.byte()?;
            if flags & 0xe0 != 0 {
                return Err(bad("unknown header flags set"));
            }
            for _ in 0..6 {
                self.byte()?;
            }
            if flags & 4 != 0 {
                let size = usize::from(self.byte()?) | usize::from(self.byte()?) << 8;
                for _ in 0..size {
                    self.byte()?;
                }
            }
            for flag in [8, 16] {
                if flags & flag != 0 {
                    while self.byte()? != 0 {}
                }
            }
            if flags & 2 != 0 {
                self.byte()?;
                self.byte()?;
            }
        } else {
            let (method, flags) = (self.byte()?, self.byte()?);
            if (u32::from(method) << 8 | u32::from(flags)) % 31 != 0 {
                return Err(bad("incorrect header check"));
            }
            if method & 15 != 8 {
                return Err(bad("unknown compression method"));
            }
            if method >> 4 > 7 {
                return Err(bad("invalid window size"));
            }
            if flags & 0x20 != 0 {
                let wanted = (0..4).try_fold(0u32, |sum, _| Ok::<_, Stop>(sum << 8 | u32::from(self.byte()?)))?;
                let Some(dictionary) = &self.dictionary else {
                    return Err(Stop::Bad("Z_NEED_DICT", "Missing dictionary"));
                };
                if adler32_from(dictionary, 1) != wanted {
                    return Err(Stop::Bad("Z_NEED_DICT", "Bad dictionary"));
                }
                self.window.extend_from_slice(dictionary);
                self.sent = self.window.len();
                self.checked = self.window.len();
            }
        }
        self.state = State::Block;
        Ok(())
    }

    #[inline(never)]
    fn block(&mut self) -> Result<(), Stop> {
        let last = self.bits.take(1)? == 1;
        let kind = self.bits.take(2)?;
        match kind {
            0 => {
                self.bits.to_byte();
                let size = self.bits.take(16)?;
                let check = self.bits.take(16)?;
                if size ^ 0xffff != check {
                    return Err(bad("invalid stored block lengths"));
                }
                self.last = last;
                self.state = if size == 0 { self.after_block() } else { State::Stored(size as usize) };
            }
            1 => {
                self.fixed = true;
                self.last = last;
                self.state = State::Codes;
            }
            2 => {
                let literal_count = self.bits.take(5)? as usize + 257;
                let distance_count = self.bits.take(5)? as usize + 1;
                let order_count = self.bits.take(4)? as usize + 4;
                if literal_count > 286 || distance_count > 30 {
                    return Err(bad("too many length or distance symbols"));
                }
                let mut code_lengths = [0u8; 19];
                for place in &ORDER[..order_count] {
                    code_lengths[*place] = self.bits.take(3)? as u8;
                }
                // The code for code lengths is read through the table the distances will have, which isn't needed
                // until it's made again below.
                let length_codes = &mut self.distances;
                if !length_codes.build(&code_lengths, 7, length_meaning) {
                    return Err(bad("invalid code lengths set"));
                }
                let wanted = literal_count + distance_count;
                let mut all = [0u8; 286 + 30];
                let mut filled = 0;
                while filled < wanted {
                    let (_, symbol) = read(&mut self.bits, length_codes, "invalid code lengths set")?;
                    if symbol < 16 {
                        all[filled] = symbol as u8;
                        filled += 1;
                    } else {
                        let (value, times) = match symbol {
                            16 => {
                                if filled == 0 {
                                    return Err(bad("invalid bit length repeat"));
                                }
                                (all[filled - 1], 3 + self.bits.take(2)?)
                            }
                            17 => (0, 3 + self.bits.take(3)?),
                            _ => (0, 11 + self.bits.take(7)?),
                        };
                        let times = times as usize;
                        if filled + times > wanted {
                            return Err(bad("invalid bit length repeat"));
                        }
                        all[filled..filled + times].fill(value);
                        filled += times;
                    }
                }
                if all[256] == 0 {
                    return Err(bad("invalid code -- missing end-of-block"));
                }
                if !self.literals.build(&all[..literal_count], LITERAL_ROOT, literal_meaning) {
                    return Err(bad("invalid literal/lengths set"));
                }
                if !self.distances.build(&all[literal_count..wanted], DISTANCE_ROOT, distance_meaning) {
                    return Err(bad("invalid distances set"));
                }
                self.fixed = false;
                self.last = last;
                self.state = State::Codes;
            }
            _ => return Err(bad("invalid block type")),
        }
        Ok(())
    }

    /// Reads a block's symbols until it ends, or what's come runs out.
    fn symbols(&mut self) -> Result<(), Stop> {
        let (literals, distances) = if self.fixed {
            let fixed = fixed_tables();
            (&fixed.0, &fixed.1)
        } else {
            (&self.literals, &self.distances)
        };
        let bits = &mut self.bits;
        let window = &mut self.window;
        let before = (bits.at, window.len());
        if fast(bits, window, literals, distances)? {
            self.state = self.after_block();
            return Ok(());
        }
        let mut any = before != (bits.at, window.len());
        loop {
            let mark = bits.mark();
            let went = (|| {
                let (entry, value) = read(bits, literals, "invalid literal/length code")?;
                if entry & LITERAL != 0 {
                    window.push(value as u8);
                    return Ok(false);
                }
                if entry & END != 0 {
                    return Ok(true);
                }
                let size = value as usize;
                let (_, distance) = read(bits, distances, "invalid distance code")?;
                let distance = distance as usize;
                if distance > window.len() {
                    return Err(bad("invalid distance too far back"));
                }
                let from = window.len() - distance;
                if distance >= size {
                    window.extend_from_within(from..from + size);
                } else {
                    window.reserve(size);
                    for step in 0..size {
                        let byte = window[from + step];
                        window.push(byte);
                    }
                }
                Ok(false)
            })();
            match went {
                Ok(false) => any = true,
                Ok(true) => break,
                Err(stop) => {
                    bits.restore(mark);
                    // What's been read so far stays read, as each symbol is a part of its own, so running out after
                    // some is going on.
                    return if stop == Stop::Short && any { Ok(()) } else { Err(stop) };
                }
            }
        }
        self.state = self.after_block();
        Ok(())
    }

    #[inline(never)]
    fn trailer(&mut self) -> Result<(), Stop> {
        self.bits.to_byte();
        self.count_new();
        if matches!(self.wrap, Wrap::Gzip) {
            let check = (0..4).try_fold(0u32, |sum, at| Ok::<_, Stop>(sum | u32::from(self.byte()?) << (8 * at)))?;
            let length = (0..4).try_fold(0u32, |sum, at| Ok::<_, Stop>(sum | u32::from(self.byte()?) << (8 * at)))?;
            if check != self.check {
                return Err(bad("incorrect data check"));
            }
            if length != self.length {
                return Err(bad("incorrect length check"));
            }
            // Another member can follow, and zeros, which are often there to fill out a block, are let be.
            let next = self.bits.input.get(self.bits.at).copied();
            if next.is_some_and(|byte| byte != 0) {
                self.begin();
                return Ok(());
            }
        } else {
            let check = (0..4).try_fold(0u32, |sum, _| Ok::<_, Stop>(sum << 8 | u32::from(self.byte()?)))?;
            if check != self.check {
                return Err(bad("incorrect data check"));
            }
        }
        self.state = State::Done;
        Ok(())
    }
}

/// The most a symbol and the match after it write, with what a copy eight bytes at a time can write past its end.
const ROOM: usize = 258 + 8;

/// Reads a block's symbols for as long as there are eight bytes left to read, which is more than any symbol and its
/// distance take, so none can run out partway and nothing has to be read again. Gives whether the block ended.
///
/// What comes out is written into `window`'s spare capacity, which is kept to at least `ROOM`, and only counted in its
/// length at the end. Nothing has to be filled in first, as fresh memory would be faulted in twice then, and each
/// write needs no check. Copies go eight bytes at a time, past where they end into what's written next.
#[inline(never)]
#[allow(unsafe_code)]
fn fast(bits: &mut Bits, window: &mut Vec<u8>, literals: &Table, distances: &Table) -> Result<bool, Stop> {
    let input = &bits.input[..];
    let (mut held, mut count, mut at) = (bits.held, bits.count, bits.at);
    let mut out = window.len();
    let mut room = window.as_mut_ptr();
    let literal_mask = (1u64 << literals.root) - 1;
    let distance_mask = (1u64 << distances.root) - 1;
    let (literal_root, distance_root) = (literals.root, distances.root);
    let (literals, distances) = (&literals.entries[..], &distances.entries[..]);
    let went = loop {
        if at + 8 > input.len() {
            break Ok(false);
        }
        if out + ROOM > window.capacity() {
            // SAFETY: everything below `out` has been written.
            unsafe { window.set_len(out) };
            // Room for about as much as what's left to read will make, as most text comes to four times as much.
            window.reserve(ROOM + 4 * (input.len() - at) + 4096);
            room = window.as_mut_ptr();
        }
        let word = u64::from_le_bytes(input[at..at + 8].try_into().unwrap_or_default());
        held |= word << count;
        at += ((63 - count) >> 3) as usize;
        count |= 56;
        let mut entry = literals[(held & literal_mask) as usize];
        if entry & LINK != 0 {
            entry = literals[(entry >> 16) as usize + ((held >> literal_root) & ((1 << (entry & 31)) - 1)) as usize];
        }
        if entry & LITERAL != 0 {
            // SAFETY: `out + ROOM` is within the capacity, as was made sure above, and this writes two at most.
            unsafe { room.add(out).write((entry >> 16) as u8) };
            out += 1;
            held >>= entry & 31;
            count -= entry & 31;
            // At least 41 bits are left, enough for another literal, which often comes next.
            let entry = literals[(held & literal_mask) as usize];
            if entry & LITERAL != 0 {
                // SAFETY: as for the one before.
                unsafe { room.add(out).write((entry >> 16) as u8) };
                out += 1;
                held >>= entry & 31;
                count -= entry & 31;
            }
            continue;
        }
        if entry & SIZED == 0 {
            if entry & END != 0 {
                held >>= entry & 31;
                count -= entry & 31;
                break Ok(true);
            }
            break Err(bad("invalid literal/length code"));
        }
        // A length takes at most 20 bits and a distance 28, which the 56 there were are enough for.
        let total = entry & 31;
        let length = (entry >> 16) as usize + ((held & ((1 << total) - 1)) >> ((entry >> 5) & 15)) as usize;
        held >>= total;
        count -= total;
        let mut entry = distances[(held & distance_mask) as usize];
        if entry & LINK != 0 {
            entry = distances[(entry >> 16) as usize + ((held >> distance_root) & ((1 << (entry & 31)) - 1)) as usize];
        }
        if entry & SIZED == 0 {
            break Err(bad("invalid distance code"));
        }
        let total = entry & 31;
        let distance = (entry >> 16) as usize + ((held & ((1 << total) - 1)) >> ((entry >> 5) & 15)) as usize;
        held >>= total;
        count -= total;
        if distance > out {
            break Err(bad("invalid distance too far back"));
        }
        let end = out + length;
        let mut from = out - distance;
        // SAFETY: what's read is all below `out` when it's read, so it's been written, and what's written is below
        // `end` rounded up to eight, which is no more than `ROOM` past where the copy starts, inside the capacity.
        unsafe {
            if distance >= 8 {
                // Each eight read end no later than where the eight written start, so they don't overlap.
                while out < end {
                    std::ptr::copy_nonoverlapping(room.add(from), room.add(out), 8);
                    from += 8;
                    out += 8;
                }
            } else if distance == 1 {
                let eight = [*room.add(from); 8];
                while out < end {
                    std::ptr::copy_nonoverlapping(eight.as_ptr(), room.add(out), 8);
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
    };
    // SAFETY: everything below `out` has been written, and it's within the capacity.
    unsafe { window.set_len(out) };
    (bits.held, bits.count, bits.at) = (held, count, at);
    went
}
