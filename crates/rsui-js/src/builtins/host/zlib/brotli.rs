//! Brotli, read the way RFC 7932 describes it, all of it: block types and counts, context modelling, the last
//! distances, and words from the dictionary with each of the 121 ways they can be changed. Like DEFLATE, it's read
//! a piece at a time, and a part that runs out partway is read again from its start once there's more. Writing it is
//! in `brotli_encode.rs`.

use super::huffman::{Bits, Decoder as Code, Stop, copy_back};

pub(super) use super::brotli_encode::Encoder;

/// The dictionary, squeezed with DEFLATE by this engine's own `zlib.deflateRawSync`, and only undone the first time
/// a stream refers to a word in it.
const DICTIONARY_PACKED: &[u8] = include_bytes!("dictionary.bin.deflate");

/// How many words of each length there are, as a power of two.
const WORD_BITS: [u8; 25] = [0, 0, 0, 0, 10, 10, 11, 11, 10, 10, 10, 10, 10, 9, 9, 8, 7, 7, 8, 7, 7, 6, 6, 5, 5];

fn dictionary() -> &'static [u8] {
    static WORDS: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    WORDS.get_or_init(|| {
        let mut inflater = super::inflate::Inflater::new(super::Wrap::Raw, None);
        let mut out = Vec::with_capacity(122_784);
        let _ = inflater.push(DICTIONARY_PACKED, true, &mut out);
        out
    })
}

#[cfg(test)]
pub(super) fn dictionary_for_test() -> &'static [u8] {
    dictionary()
}

/// Where the words of each length start in the dictionary.
fn word_offset(length: usize) -> usize {
    (4..length).map(|shorter| shorter << WORD_BITS[shorter]).sum()
}

/// Each transform's prefix, as where it starts in `AFFIXES`, what's done to the word, and its suffix.
const TRANSFORMS: [u8; 363] = [
    0, 0, 0, 0, 0, 1, 1, 0, 1, 0, 12, 0, 0, 10, 1, 0, 0, 9, 1, 0, 0, 20, 0, 1, 0, 0, 15, 0, 10, 0, 0, 0, 25, 0, 13, 0,
    0, 1, 0, 3, 0, 1, 0, 0, 3, 1, 10, 1, 0, 0, 31, 0, 0, 38, 12, 0, 1, 0, 0, 36, 0, 0, 23, 0, 0, 43, 0, 0, 46, 0, 3, 0,
    0, 0, 51, 0, 0, 53, 0, 14, 0, 0, 2, 0, 0, 0, 59, 0, 0, 63, 1, 10, 0, 0, 0, 48, 23, 0, 0, 1, 0, 3, 0, 15, 0, 0, 0,
    72, 0, 0, 70, 0, 0, 79, 0, 0, 86, 0, 16, 0, 0, 17, 0, 9, 0, 0, 0, 4, 0, 0, 0, 93, 0, 11, 0, 0, 0, 100, 0, 0, 105,
    0, 0, 110, 0, 7, 0, 0, 1, 115, 0, 0, 120, 0, 0, 123, 1, 0, 48, 0, 0, 125, 0, 20, 0, 0, 18, 0, 0, 6, 0, 0, 0, 91, 0,
    10, 3, 0, 8, 0, 0, 0, 132, 0, 0, 137, 9, 0, 15, 0, 5, 0, 0, 9, 0, 1, 10, 3, 0, 10, 36, 23, 0, 91, 0, 11, 1, 0, 10,
    43, 0, 0, 129, 1, 0, 23, 146, 0, 0, 9, 0, 6, 0, 10, 70, 0, 0, 152, 0, 0, 141, 23, 0, 1, 0, 10, 91, 0, 10, 23, 0, 0,
    160, 1, 0, 129, 0, 0, 166, 1, 11, 1, 0, 0, 170, 1, 11, 0, 0, 0, 143, 0, 11, 36, 0, 10, 48, 1, 0, 91, 0, 0, 174, 1,
    10, 48, 0, 0, 179, 0, 0, 184, 0, 11, 70, 0, 0, 190, 1, 10, 23, 0, 11, 43, 1, 0, 143, 0, 10, 141, 0, 0, 195, 0, 11,
    23, 200, 0, 0, 1, 0, 141, 0, 10, 129, 0, 11, 129, 0, 0, 203, 0, 11, 3, 0, 10, 143, 1, 10, 141, 1, 11, 129, 1, 11,
    3, 0, 11, 141, 0, 11, 91, 0, 11, 48, 1, 11, 23, 0, 11, 143, 1, 11, 48, 1, 10, 129, 1, 11, 143, 1, 10, 143,
];

/// The prefixes and suffixes, each ending in a zero.
const AFFIXES: &[u8; 208] = b"\x00 \x00, \x00 of the \x00 of \x00s \x00.\x00 and \x00 in \x00\x22\x00 to \x00\x22>\x00\x0a\x00. \x00]\x00 for \x00 a \x00 that \x00'\x00 with \x00 from \x00 by \x00(\x00. The \x00 on \x00 as \x00 is \x00ing \x00\x0a\x09\x00:\x00ed \x00=\x22\x00 at \x00ly \x00,\x00='\x00.com/\x00. This \x00 not \x00er \x00al \x00ful \x00ive \x00less \x00est \x00ize \x00\xc2\xa0\x00ous \x00";

fn affix(at: u8) -> &'static [u8] {
    let rest = &AFFIXES[usize::from(at)..];
    &rest[..rest.iter().position(|byte| *byte == 0).unwrap_or(rest.len())]
}

/// Makes the character at the start of `word` upper case, the simple way Brotli does, saying how long it was.
fn upper(word: &mut [u8]) -> usize {
    let Some(first) = word.first().copied() else {
        return 1;
    };
    if first < 0xc0 {
        if first.is_ascii_lowercase() {
            word[0] ^= 32;
        }
        return 1;
    }
    if first < 0xe0 {
        if let Some(second) = word.get_mut(1) {
            *second ^= 32;
        }
        return 2;
    }
    if let Some(third) = word.get_mut(2) {
        *third ^= 5;
    }
    3
}

/// A word of the dictionary, changed the way transform `which` says.
pub(super) fn transformed(word: &[u8], which: usize, out: &mut Vec<u8>) {
    let (prefix, kind, suffix) = (TRANSFORMS[which * 3], TRANSFORMS[which * 3 + 1], TRANSFORMS[which * 3 + 2]);
    out.extend_from_slice(affix(prefix));
    let mut word = word;
    if (1..=9).contains(&kind) {
        word = &word[..word.len().saturating_sub(usize::from(kind))];
    } else if kind >= 12 {
        word = &word[usize::from(kind - 11).min(word.len())..];
    }
    let start = out.len();
    out.extend_from_slice(word);
    if kind == 10 {
        upper(&mut out[start..]);
    } else if kind == 11 {
        let mut at = start;
        while at < out.len() {
            at += upper(&mut out[at..]);
        }
    }
    out.extend_from_slice(affix(suffix));
}

/// The context of a literal from the two before it, for UTF-8: the first 128 of each byte before last and before that.
const UTF8_LOW: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 8, 12, 16, 12, 12,
    20, 12, 16, 24, 28, 12, 12, 32, 12, 36, 12, 44, 44, 44, 44, 44, 44, 44, 44, 44, 44, 32, 32, 24, 40, 28, 12, 12, 48,
    52, 52, 52, 48, 52, 52, 52, 48, 52, 52, 52, 52, 52, 48, 52, 52, 52, 52, 52, 48, 52, 52, 52, 52, 52, 24, 12, 28, 12,
    12, 12, 56, 60, 60, 60, 56, 60, 60, 60, 56, 60, 60, 60, 60, 60, 56, 60, 60, 60, 60, 60, 56, 60, 60, 60, 60, 60, 24,
    12, 28, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 1, 1, 1, 1, 0,
];

fn signed(byte: u8) -> u8 {
    match byte {
        0 => 0,
        1..=15 => 1,
        16..=63 => 2,
        64..=127 => 3,
        128..=191 => 4,
        192..=239 => 5,
        240..=254 => 6,
        255 => 7,
    }
}

/// Which of 64 contexts a literal is in, from the two bytes before it, in each of the four modes.
#[inline]
pub(super) fn context(mode: u8, last: u8, before: u8) -> usize {
    usize::from(match mode {
        0 => last & 0x3f,
        1 => last >> 2,
        2 => {
            let first = if last < 128 { UTF8_LOW[usize::from(last)] } else if last < 192 { last & 1 } else { 2 + (last & 1) };
            let second = if before < 128 { UTF8_LOW[128 + usize::from(before)] } else if before < 224 { 0 } else { 2 };
            first | second
        }
        _ => signed(last) << 3 | signed(before),
    })
}

/// Where insert lengths start for each code, and how many bits follow, and the same for copy lengths.
pub(super) const INSERT_BASE: [u32; 24] =
    [0, 1, 2, 3, 4, 5, 6, 8, 10, 14, 18, 26, 34, 50, 66, 98, 130, 194, 322, 578, 1090, 2114, 6210, 22594];
pub(super) const INSERT_EXTRA: [u8; 24] = [0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 7, 8, 9, 10, 12, 14, 24];
pub(super) const COPY_BASE: [u32; 24] =
    [2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 14, 18, 22, 30, 38, 54, 70, 102, 134, 198, 326, 582, 1094, 2118];
pub(super) const COPY_EXTRA: [u8; 24] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 7, 8, 9, 10, 24];
/// For each group of 64 insert-and-copy codes, where its insert codes and copy codes start.
pub(super) const CELL_INSERT: [u8; 11] = [0, 0, 0, 0, 8, 8, 0, 16, 8, 16, 16];
pub(super) const CELL_COPY: [u8; 11] = [0, 8, 0, 8, 0, 8, 16, 0, 16, 8, 16];
/// Where block counts start for each code, and how many bits follow.
pub(super) const COUNT_BASE: [u32; 26] = [
    1, 5, 9, 13, 17, 25, 33, 41, 49, 65, 81, 97, 113, 145, 177, 209, 241, 305, 369, 497, 753, 1265, 2289, 4337, 8433,
    16625,
];
pub(super) const COUNT_EXTRA: [u8; 26] = [2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 7, 8, 9, 10, 11, 12, 13, 24];
/// The order the lengths of the code for code lengths come in.
pub(super) const LENGTH_ORDER: [usize; 18] = [1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15];

fn wrong(what: &'static str) -> Stop {
    Stop::Bad(what, "Decompression failed")
}

/// A category's block types: how many, the codes for switching, which it's on, and for how much longer.
struct Blocks {
    types: u32,
    type_code: Option<Code>,
    count_code: Option<Code>,
    /// The type before last, and the last one, which is the one it's on.
    ring: [u32; 2],
    left: u32,
}

impl Blocks {
    fn one() -> Self {
        Self { types: 1, type_code: None, count_code: None, ring: [1, 0], left: u32::MAX }
    }
}

/// Reads a symbol, through the inlined way when none of it can be missing.
#[inline(always)]
fn decode(bits: &mut Bits, code: &Code) -> Result<u16, Stop> {
    if bits.at + 8 <= bits.input.len() { bits.decode_ahead(code) } else { bits.decode(code) }
}

fn read_count(bits: &mut Bits, code: &Code) -> Result<u32, Stop> {
    let symbol = usize::from(bits.decode(code)?);
    Ok(COUNT_BASE[symbol] + bits.take(u32::from(COUNT_EXTRA[symbol]))?)
}

/// What a meta-block says about how to read its commands.
struct Meta {
    blocks: [Blocks; 3],
    postfix: u32,
    direct: u32,
    modes: Vec<u8>,
    literal_map: Vec<u8>,
    distance_map: Vec<u8>,
    literals: Vec<Code>,
    commands: Vec<Code>,
    distances: Vec<Code>,
}

enum State {
    Start,
    Header,
    Stored(usize),
    Skip(usize),
    Command,
    Literals,
    Distance,
    Done,
}

pub(super) struct Decoder {
    bits: Bits,
    state: State,
    window_size: usize,
    /// What's come out: as much as can be referred back to, and what's yet to be given out.
    window: Vec<u8>,
    sent: usize,
    /// How much has come out in all, for how far back can be referred to.
    total: u64,
    last: bool,
    /// What's left of this meta-block.
    left: usize,
    meta: Option<Box<Meta>>,
    distances: [u32; 4],
    insert: u32,
    copy: u32,
    /// The command says to use the last distance, without a distance code.
    implicit: bool,
    wait: usize,
}

impl Decoder {
    pub(super) fn new() -> Self {
        Self {
            bits: Bits::default(),
            state: State::Start,
            window_size: 0,
            window: Vec::new(),
            sent: 0,
            total: 0,
            last: false,
            left: 0,
            meta: None,
            distances: [16, 15, 11, 4],
            insert: 0,
            copy: 0,
            implicit: false,
            wait: 0,
        }
    }

    pub(super) fn ended(&self) -> bool {
        matches!(self.state, State::Done)
    }

    /// Reads what's come, giving out what that undoes. `finish` says nothing more is coming.
    pub(super) fn push(&mut self, input: &[u8], finish: bool, out: &mut Vec<u8>) -> Result<(), Stop> {
        if !matches!(self.state, State::Done) {
            self.bits.input.extend_from_slice(input);
        }
        let unread = self.bits.input.len() - self.bits.at;
        let went = if unread >= self.wait || finish { self.run() } else { Err(Stop::Short) };
        out.extend_from_slice(&self.window[self.sent..]);
        self.sent = self.window.len();
        let keep = self.window_size.max(1 << 16);
        if self.window.len() > 2 * keep {
            let drop = self.window.len() - keep;
            self.window.drain(..drop);
            self.sent -= drop;
        }
        self.bits.forget_read();
        match went {
            Ok(()) => {
                self.wait = 0;
                Ok(())
            }
            Err(Stop::Short) if finish => Err(Stop::Bad("Z_BUF_ERROR", "unexpected end of file")),
            Err(Stop::Short) => {
                let unread = self.bits.input.len() - self.bits.at;
                if self.wait <= unread {
                    self.wait = unread.saturating_mul(2).max(64);
                }
                Ok(())
            }
            Err(stop) => Err(stop),
        }
    }

    fn run(&mut self) -> Result<(), Stop> {
        loop {
            let mark = self.bits.mark();
            let went = match self.state {
                State::Done => {
                    self.bits.at = self.bits.input.len();
                    return Ok(());
                }
                State::Start => self.start(),
                State::Header => self.header(),
                State::Stored(left) => {
                    self.bits.to_byte();
                    let bytes = self.bits.bytes(left);
                    if bytes.is_empty() {
                        Err(Stop::Short)
                    } else {
                        let got = bytes.len();
                        self.window.extend_from_slice(bytes);
                        self.total += got as u64;
                        self.state = if got == left { self.after_meta() } else { State::Stored(left - got) };
                        Ok(())
                    }
                }
                State::Skip(left) => {
                    self.bits.to_byte();
                    let got = self.bits.bytes(left).len();
                    if got == 0 && left > 0 {
                        Err(Stop::Short)
                    } else {
                        self.state = if got == left { self.after_meta() } else { State::Skip(left - got) };
                        Ok(())
                    }
                }
                State::Command => self.command(),
                State::Literals => self.literals(),
                State::Distance => self.distance(),
            };
            match went {
                Ok(()) => {}
                Err(Stop::Short) => {
                    self.bits.restore(mark);
                    return Err(Stop::Short);
                }
                Err(stop) => return Err(stop),
            }
        }
    }

    fn after_meta(&self) -> State {
        if self.last { State::Done } else { State::Header }
    }

    /// The size of the window, which the stream starts with.
    fn start(&mut self) -> Result<(), Stop> {
        let bits = if self.bits.take(1)? == 0 {
            16
        } else {
            match self.bits.take(3)? {
                0 => match self.bits.take(3)? {
                    1 => return Err(wrong("ERR__ERROR_FORMAT_WINDOW_BITS")),
                    0 => 17,
                    more => 8 + more,
                },
                more => 17 + more,
            }
        };
        self.window_size = (1 << bits) - 16;
        self.state = State::Header;
        Ok(())
    }

    /// The bits that end the stream, or come before what's stored as it is, which have to be zeros.
    fn padding(&mut self, what: &'static str) -> Result<(), Stop> {
        if self.bits.pad() { Ok(()) } else { Err(wrong(what)) }
    }

    #[inline(never)]
    fn header(&mut self) -> Result<(), Stop> {
        let bits = &mut self.bits;
        let last = bits.take(1)? == 1;
        if last && bits.take(1)? == 1 {
            self.last = true;
            self.padding("ERR__ERROR_FORMAT_PADDING_2")?;
            self.state = State::Done;
            return Ok(());
        }
        let nibbles = match bits.take(2)? {
            3 => 0,
            more => more + 4,
        };
        if nibbles == 0 {
            if bits.take(1)? != 0 {
                return Err(wrong("ERR__ERROR_FORMAT_RESERVED"));
            }
            let size_bytes = bits.take(2)?;
            let mut skip = 0usize;
            for at in 0..size_bytes {
                let byte = bits.take(8)? as usize;
                if at + 1 == size_bytes && size_bytes > 1 && byte == 0 {
                    return Err(wrong("ERR__ERROR_FORMAT_EXUBERANT_META_NIBBLE"));
                }
                skip |= byte << (8 * at);
            }
            let skip = if size_bytes == 0 { 0 } else { skip + 1 };
            self.padding("ERR__ERROR_FORMAT_PADDING_1")?;
            self.last = last;
            self.state = State::Skip(skip);
            return Ok(());
        }
        let mut length = 0usize;
        for at in 0..nibbles {
            let nibble = self.bits.take(4)? as usize;
            if at + 1 == nibbles && nibbles > 4 && nibble == 0 {
                return Err(wrong("ERR__ERROR_FORMAT_EXUBERANT_NIBBLE"));
            }
            length |= nibble << (4 * at);
        }
        let length = length + 1;
        self.last = last;
        self.left = length;
        if !last && self.bits.take(1)? == 1 {
            self.padding("ERR__ERROR_FORMAT_PADDING_1")?;
            self.state = State::Stored(length);
            return Ok(());
        }
        self.meta = Some(Box::new(self.meta_header()?));
        self.state = State::Command;
        Ok(())
    }

    fn meta_header(&mut self) -> Result<Meta, Stop> {
        let bits = &mut self.bits;
        let mut blocks = [Blocks::one(), Blocks::one(), Blocks::one()];
        for category in &mut blocks {
            let types = var_length(bits)? + 1;
            category.types = types;
            if types >= 2 {
                let type_code = prefix_code(bits, types as usize + 2)?;
                let count_code = prefix_code(bits, 26)?;
                category.left = read_count(bits, &count_code)?;
                category.type_code = Some(type_code);
                category.count_code = Some(count_code);
            }
        }
        let postfix = bits.take(2)?;
        let direct = bits.take(4)? << postfix;
        let mut modes = Vec::with_capacity(blocks[0].types as usize);
        for _ in 0..blocks[0].types {
            modes.push(bits.take(2)? as u8);
        }
        let literal_trees = var_length(bits)? + 1;
        let literal_map = context_map(bits, 64 * blocks[0].types as usize, literal_trees)?;
        let distance_trees = var_length(bits)? + 1;
        let distance_map = context_map(bits, 4 * blocks[2].types as usize, distance_trees)?;
        let literals = (0..literal_trees).map(|_| prefix_code(bits, 256)).collect::<Result<Vec<_>, _>>()?;
        let commands = (0..blocks[1].types).map(|_| prefix_code(bits, 704)).collect::<Result<Vec<_>, _>>()?;
        let distance_size = 16 + direct as usize + (48 << postfix);
        let distances = (0..distance_trees).map(|_| prefix_code(bits, distance_size)).collect::<Result<Vec<_>, _>>()?;
        Ok(Meta { blocks, postfix, direct, modes, literal_map, distance_map, literals, commands, distances })
    }

    /// What a category's block type and count are to be, which changes when it's used up. Nothing's changed until
    /// what it's for has been read too.
    fn switched(bits: &mut Bits, blocks: &Blocks) -> Result<([u32; 2], u32), Stop> {
        if blocks.left > 0 {
            return Ok((blocks.ring, blocks.left));
        }
        let (Some(type_code), Some(count_code)) = (&blocks.type_code, &blocks.count_code) else {
            return Ok((blocks.ring, blocks.left));
        };
        let symbol = u32::from(bits.decode(type_code)?);
        let mut kind = match symbol {
            0 => blocks.ring[0],
            1 => blocks.ring[1] + 1,
            more => more - 2,
        };
        if kind >= blocks.types {
            kind -= blocks.types;
        }
        let count = read_count(bits, count_code)?;
        Ok(([blocks.ring[1], kind], count))
    }

    fn command(&mut self) -> Result<(), Stop> {
        if self.left == 0 {
            self.state = self.after_meta();
            return Ok(());
        }
        let Some(meta) = self.meta.as_deref_mut() else {
            return Err(wrong("ERR__ERROR_FORMAT_UNREACHABLE"));
        };
        let bits = &mut self.bits;
        let (ring, left) = Self::switched(bits, &meta.blocks[1])?;
        let code = usize::from(decode(bits, &meta.commands[ring[1] as usize])?);
        let cell = code >> 6;
        let insert_code = usize::from(CELL_INSERT[cell]) + ((code >> 3) & 7);
        let copy_code = usize::from(CELL_COPY[cell]) + (code & 7);
        let insert = INSERT_BASE[insert_code] + bits.take(u32::from(INSERT_EXTRA[insert_code]))?;
        let copy = COPY_BASE[copy_code] + bits.take(u32::from(COPY_EXTRA[copy_code]))?;
        meta.blocks[1].ring = ring;
        meta.blocks[1].left = left.wrapping_sub(1);
        self.insert = insert;
        self.copy = copy;
        self.implicit = cell < 2;
        self.state = State::Literals;
        Ok(())
    }

    fn literals(&mut self) -> Result<(), Stop> {
        let Some(meta) = self.meta.as_deref_mut() else {
            return Err(wrong("ERR__ERROR_FORMAT_UNREACHABLE"));
        };
        let bits = &mut self.bits;
        let mut any = false;
        // While there are eight more bytes to read, none of a literal can be missing, so nothing has to be read again,
        // and while its block lasts, its code and context don't change.
        let blocks = &mut meta.blocks[0];
        let run = (self.insert as usize).min(self.left).min(blocks.left as usize);
        if run > 0 && bits.at + 8 <= bits.input.len() {
            let kind = blocks.ring[1] as usize;
            let (mode, map) = (meta.modes[kind], &meta.literal_map[64 * kind..64 * kind + 64]);
            let length = self.window.len();
            let mut last = if length > 0 { self.window[length - 1] } else { 0 };
            let mut before = if length > 1 { self.window[length - 2] } else { 0 };
            self.window.reserve(run);
            let mut done = 0;
            let mut went = Ok(());
            while done < run && bits.at + 8 <= bits.input.len() {
                match bits.decode_ahead(&meta.literals[usize::from(map[context(mode, last, before)])]) {
                    Ok(byte) => {
                        (before, last) = (last, byte as u8);
                        self.window.push(last);
                        done += 1;
                    }
                    Err(stop) => {
                        went = Err(stop);
                        break;
                    }
                }
            }
            blocks.left -= done as u32;
            self.insert -= done as u32;
            self.left -= done;
            self.total += done as u64;
            any = done > 0;
            match went {
                Err(Stop::Short) if any => return Ok(()),
                Err(stop) => return Err(stop),
                Ok(()) => {}
            }
        }
        while self.insert > 0 {
            if self.left == 0 {
                return Err(wrong("ERR__ERROR_FORMAT_BLOCK_LENGTH_1"));
            }
            let mark = bits.mark();
            let went = (|| {
                let (ring, left) = Self::switched(bits, &meta.blocks[0])?;
                let kind = ring[1] as usize;
                let length = self.window.len();
                let last = if length > 0 { self.window[length - 1] } else { 0 };
                let before = if length > 1 { self.window[length - 2] } else { 0 };
                let tree = meta.literal_map[64 * kind + context(meta.modes[kind], last, before)];
                Ok((bits.decode(&meta.literals[usize::from(tree)])?, ring, left))
            })();
            match went {
                Ok((byte, ring, left)) => {
                    self.window.push(byte as u8);
                    meta.blocks[0].ring = ring;
                    meta.blocks[0].left = left.wrapping_sub(1);
                    self.insert -= 1;
                    self.left -= 1;
                    self.total += 1;
                    any = true;
                }
                Err(stop) => {
                    bits.restore(mark);
                    // What came before stays read, as each literal is a part of its own, so running out after some
                    // is going on.
                    return if stop == Stop::Short && any { Ok(()) } else { Err(stop) };
                }
            }
        }
        self.state = if self.left == 0 { State::Command } else { State::Distance };
        Ok(())
    }

    fn distance(&mut self) -> Result<(), Stop> {
        let Some(meta) = self.meta.as_deref_mut() else {
            return Err(wrong("ERR__ERROR_FORMAT_UNREACHABLE"));
        };
        let bits = &mut self.bits;
        let (distance, push) = if self.implicit {
            (self.distances[3], false)
        } else {
            let (ring, left) = Self::switched(bits, &meta.blocks[2])?;
            let context = if self.copy > 4 { 3 } else { self.copy as usize - 2 };
            let tree = meta.distance_map[4 * ring[1] as usize + context];
            let code = u32::from(decode(bits, &meta.distances[usize::from(tree)])?);
            let found = if code < 16 {
                let [_, _, second, last] = self.distances;
                let (base, offset): (u32, i64) = match code {
                    0..=3 => (self.distances[3 - code as usize], 0),
                    4..=9 => (last, [-1, 1, -2, 2, -3, 3][code as usize - 4]),
                    _ => (second, [-1, 1, -2, 2, -3, 3][code as usize - 10]),
                };
                let made = i64::from(base) + offset;
                if made <= 0 {
                    return Err(wrong("ERR__ERROR_FORMAT_DISTANCE"));
                }
                made as u32
            } else if code < 16 + meta.direct {
                code - 15
            } else {
                let rest = code - meta.direct - 16;
                let extra_bits = 1 + (rest >> (meta.postfix + 1));
                let extra = bits.take(extra_bits)?;
                let high = rest >> meta.postfix;
                let low = rest & ((1 << meta.postfix) - 1);
                let offset = ((2 + (high & 1)) << extra_bits) - 4;
                ((offset + extra) << meta.postfix) + low + meta.direct + 1
            };
            meta.blocks[2].ring = ring;
            meta.blocks[2].left = left.wrapping_sub(1);
            (found, code != 0)
        };
        let copy = self.copy as usize;
        let reach = (self.window_size as u64).min(self.total) as usize;
        if distance as usize <= reach {
            if copy > self.left {
                return Err(wrong("ERR__ERROR_FORMAT_BLOCK_LENGTH_2"));
            }
            if push {
                self.distances = [self.distances[1], self.distances[2], self.distances[3], distance];
            }
            copy_back(&mut self.window, distance as usize, copy);
            self.left -= copy;
            self.total += copy as u64;
        } else {
            // Further back than there is refers to a word in the dictionary.
            if !(4..=24).contains(&copy) {
                return Err(wrong("ERR__ERROR_FORMAT_DICTIONARY"));
            }
            let id = distance as usize - reach - 1;
            let word_bits = WORD_BITS[copy];
            let (index, transform) = (id & ((1 << word_bits) - 1), id >> word_bits);
            if transform >= 121 {
                return Err(wrong("ERR__ERROR_FORMAT_TRANSFORM"));
            }
            let start = word_offset(copy) + index * copy;
            let before = self.window.len();
            transformed(&dictionary()[start..start + copy], transform, &mut self.window);
            let made = self.window.len() - before;
            if made > self.left {
                return Err(wrong("ERR__ERROR_FORMAT_BLOCK_LENGTH_2"));
            }
            self.left -= made;
            self.total += made as u64;
        }
        self.state = State::Command;
        Ok(())
    }
}

/// A number from 0 to 255 written in as few bits as it needs, as block types and tree counts are.
fn var_length(bits: &mut Bits) -> Result<u32, Stop> {
    if bits.take(1)? == 0 {
        return Ok(0);
    }
    let size = bits.take(3)?;
    if size == 0 { Ok(1) } else { Ok((1 << size) + bits.take(size)?) }
}

/// Reads a prefix code for an alphabet of this size, either of the two ways it can be written.
fn prefix_code(bits: &mut Bits, size: usize) -> Result<Code, Stop> {
    let mut lengths = vec![0u8; size];
    let skip = bits.take(2)?;
    if skip == 1 {
        let width = usize::BITS - (size - 1).leading_zeros();
        let count = bits.take(2)? as usize + 1;
        let mut symbols = [0usize; 4];
        for at in 0..count {
            symbols[at] = bits.take(width)? as usize;
            if symbols[at] >= size {
                return Err(wrong("ERR__ERROR_FORMAT_SIMPLE_HUFFMAN_ALPHABET"));
            }
            if symbols[..at].contains(&symbols[at]) {
                return Err(wrong("ERR__ERROR_FORMAT_SIMPLE_HUFFMAN_SAME"));
            }
        }
        let given: &[u8] = match count {
            1 => return Ok(Code::only(symbols[0] as u16)),
            2 => &[1, 1],
            3 => &[1, 2, 2],
            _ if bits.take(1)? == 1 => &[1, 2, 3, 3],
            _ => &[2, 2, 2, 2],
        };
        for (symbol, length) in symbols.iter().zip(given) {
            lengths[*symbol] = *length;
        }
        return Code::new(&lengths, "").map_err(|_| wrong("ERR__ERROR_FORMAT_HUFFMAN_SPACE"));
    }
    // The lengths of the code for code lengths come first, each in a code of its own, read from the right.
    const PREFIX_LENGTH: [u8; 16] = [2, 2, 2, 3, 2, 2, 2, 4, 2, 2, 2, 3, 2, 2, 2, 4];
    const PREFIX_VALUE: [u8; 16] = [0, 4, 3, 2, 0, 4, 3, 1, 0, 4, 3, 2, 0, 4, 3, 5];
    let mut code_lengths = [0u8; 18];
    let mut space = 32i32;
    let mut used = 0;
    for place in &LENGTH_ORDER[skip as usize..] {
        let peek = peek4(bits)?;
        bits.take(u32::from(PREFIX_LENGTH[peek]))?;
        let length = PREFIX_VALUE[peek];
        code_lengths[*place] = length;
        if length != 0 {
            space -= 32 >> length;
            used += 1;
            if space <= 0 {
                break;
            }
        }
    }
    if !(used == 1 || space == 0) {
        return Err(wrong("ERR__ERROR_FORMAT_CL_SPACE"));
    }
    let length_code = if used == 1 {
        Code::only(code_lengths.iter().position(|length| *length != 0).unwrap_or(0) as u16)
    } else {
        Code::new(&code_lengths, "").map_err(|_| wrong("ERR__ERROR_FORMAT_CL_SPACE"))?
    };
    let (mut symbol, mut previous, mut repeat, mut repeat_length) = (0usize, 8u8, 0usize, 0u8);
    let mut space = 32768i32;
    while symbol < size && space > 0 {
        let found = bits.decode(&length_code)?;
        if found < 16 {
            repeat = 0;
            lengths[symbol] = found as u8;
            if found != 0 {
                previous = found as u8;
                space -= 32768 >> found;
            }
            symbol += 1;
        } else {
            let (extra, length) = if found == 16 { (2, previous) } else { (3, 0) };
            if repeat_length != length {
                repeat = 0;
                repeat_length = length;
            }
            let before = repeat;
            if repeat > 0 {
                repeat = (repeat - 2) << extra;
            }
            repeat += bits.take(extra)? as usize + 3;
            let more = repeat - before;
            if symbol + more > size {
                return Err(wrong("ERR__ERROR_FORMAT_HUFFMAN_SPACE"));
            }
            lengths[symbol..symbol + more].fill(length);
            symbol += more;
            if length != 0 {
                space -= (more as i32) << (15 - length);
            }
        }
    }
    if space != 0 {
        return Err(wrong("ERR__ERROR_FORMAT_HUFFMAN_SPACE"));
    }
    Code::new(&lengths, "").map_err(|_| wrong("ERR__ERROR_FORMAT_HUFFMAN_SPACE"))
}

/// The next four bits without reading them, as zeros past what's come, which the code they're for may not need.
fn peek4(bits: &mut Bits) -> Result<usize, Stop> {
    let mark = bits.mark();
    let mut value = 0;
    for at in 0..4 {
        match bits.take(1) {
            Ok(bit) => value |= (bit as usize) << at,
            Err(_) if at >= 2 => break,
            Err(stop) => {
                bits.restore(mark);
                return Err(stop);
            }
        }
    }
    bits.restore(mark);
    Ok(value)
}

fn context_map(bits: &mut Bits, size: usize, trees: u32) -> Result<Vec<u8>, Stop> {
    if trees < 2 {
        return Ok(vec![0; size]);
    }
    let most_run = if bits.take(1)? == 1 { bits.take(4)? + 1 } else { 0 };
    let code = prefix_code(bits, (trees + most_run) as usize)?;
    let mut map = Vec::with_capacity(size);
    while map.len() < size {
        let symbol = u32::from(bits.decode(&code)?);
        if symbol == 0 {
            map.push(0);
        } else if symbol <= most_run {
            let run = (1usize << symbol) + bits.take(symbol)? as usize;
            if map.len() + run > size {
                return Err(wrong("ERR__ERROR_FORMAT_CONTEXT_MAP_REPEAT"));
            }
            map.resize(map.len() + run, 0);
        } else {
            map.push((symbol - most_run) as u8);
        }
    }
    if bits.take(1)? == 1 {
        // Each was where it is in a list that moves what's used to the front.
        let mut order: Vec<u8> = (0..=255).collect();
        for value in &mut map {
            let at = usize::from(*value);
            let found = order[at];
            order.copy_within(0..at, 1);
            order[0] = found;
            *value = found;
        }
    }
    Ok(map)
}
