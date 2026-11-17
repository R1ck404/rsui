//! What DEFLATE and Brotli share: bits read and written lowest first, and prefix codes, made from how often each
//! symbol comes so that they take the fewest bits, and read through a table.

/// Writes bits, lowest first.
#[derive(Default)]
pub(super) struct BitWriter {
    pub(super) out: Vec<u8>,
    held: u64,
    count: u32,
}

impl BitWriter {
    /// Writes the lowest `bits` of `value`, which has nothing above them. `bits` is at most 32.
    #[inline]
    pub(super) fn put(&mut self, value: u32, bits: u32) {
        self.held |= u64::from(value) << self.count;
        self.count += bits;
        if self.count >= 32 {
            self.spill();
        }
    }

    /// Moves 32 bits to `out`. It's kept out of each place that writes, as it's only needed every few times, and
    /// inlining it everywhere made the program a few KB bigger for nothing that could be measured.
    #[inline(never)]
    fn spill(&mut self) {
        self.out.extend_from_slice(&(self.held as u32).to_le_bytes());
        self.held >>= 32;
        self.count -= 32;
    }

    /// Whether what's been written ends on a whole byte.
    pub(super) fn aligned(&self) -> bool {
        self.count % 8 == 0
    }

    /// Fills what's left of the byte being written with zeros.
    pub(super) fn align(&mut self) {
        while self.count > 0 {
            self.out.push(self.held as u8);
            self.held >>= 8;
            self.count = self.count.saturating_sub(8);
        }
        self.held = 0;
    }

}

/// Copies `length` bytes from `distance` back onto the end of `window`, sixteen or eight at a time when they're from
/// far enough back that each of those read has been written already, which nearly all are, and writes past where the
/// copy ends into room made for it, which what's written next goes over.
#[allow(unsafe_code)]
pub(super) fn copy_back(window: &mut Vec<u8>, distance: usize, length: usize) {
    let start = window.len();
    assert!((1..=start).contains(&distance));
    window.reserve(length + 16);
    let room = window.as_mut_ptr();
    let (mut from, mut out, end) = (start - distance, start, start + length);
    // SAFETY: what's written is below `end + 16`, which is within the capacity just made. What's read is below `out`
    // when it's read, so it's been written: sixteen or eight read end no later than where those written start, when
    // the copy is from at least that far back.
    unsafe {
        if distance >= 16 {
            while out < end {
                std::ptr::copy_nonoverlapping(room.add(from), room.add(out), 16);
                from += 16;
                out += 16;
            }
        } else if distance >= 8 {
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
        window.set_len(end);
    }
}

/// Why something couldn't be read: there isn't enough of it yet, or it's wrong, as zlib's code and message for it.
#[derive(Debug, PartialEq)]
pub(super) enum Stop {
    Short,
    Bad(&'static str, &'static str),
}

pub(super) const fn bad(message: &'static str) -> Stop {
    Stop::Bad("Z_DATA_ERROR", message)
}

/// Reads bits, lowest first, from what's come so far. A part that runs out partway goes back to where it began with
/// [`restore`](Self::restore), to be read again once there's more.
#[derive(Default)]
pub(super) struct Bits {
    pub(super) input: Vec<u8>,
    pub(super) at: usize,
    /// The bits read ahead, lowest first, of which `count` are counted. Those above them are the next bits of
    /// `input` or zeros, never anything else, so taking in a word over them changes nothing.
    pub(super) held: u64,
    pub(super) count: u32,
}

/// Where a [`Bits`] was.
#[derive(Clone, Copy)]
pub(super) struct Mark(usize, u64, u32);

impl Bits {
    /// Takes in as many whole bytes as there's room for, leaving from 56 to 63 bits held when there are eight more
    /// bytes to read, with no branches.
    #[inline]
    pub(super) fn refill(&mut self) {
        if let Some(word) = self.input.get(self.at..self.at + 8) {
            let word = u64::from_le_bytes(word.try_into().unwrap_or_default());
            self.held |= word << self.count;
            self.at += ((63 - self.count) >> 3) as usize;
            self.count |= 56;
        } else {
            self.refill_end();
        }
    }

    /// The same for the last few bytes, a byte at a time.
    #[cold]
    #[inline(never)]
    fn refill_end(&mut self) {
        while self.count < 56 && self.at < self.input.len() {
            self.held |= u64::from(self.input[self.at]) << self.count;
            self.at += 1;
            self.count += 8;
        }
    }

    pub(super) fn mark(&self) -> Mark {
        Mark(self.at, self.held, self.count)
    }

    pub(super) fn restore(&mut self, mark: Mark) {
        (self.at, self.held, self.count) = (mark.0, mark.1, mark.2);
    }

    #[inline]
    pub(super) fn take(&mut self, wanted: u32) -> Result<u32, Stop> {
        if self.count < wanted {
            self.refill();
            if self.count < wanted {
                return Err(Stop::Short);
            }
        }
        let value = (self.held & ((1u64 << wanted) - 1)) as u32;
        self.held >>= wanted;
        self.count -= wanted;
        Ok(value)
    }

    /// Reads one symbol of a code.
    #[inline]
    pub(super) fn decode(&mut self, code: &Decoder) -> Result<u16, Stop> {
        if self.count < 15 {
            self.refill();
        }
        let mut entry = code.table[(self.held & ROOT_MASK) as usize];
        if entry & LINK != 0 {
            let sub = entry & 0xff;
            entry = code.table[(entry >> 16) as usize + ((self.held >> ROOT) & ((1 << sub) - 1)) as usize];
        }
        let size = entry & 0xff;
        if entry & LEAF == 0 {
            return Err(if self.count < 15 && self.at == self.input.len() { Stop::Short } else { bad(code.wrong) });
        }
        if size > self.count {
            return Err(Stop::Short);
        }
        self.held >>= size;
        self.count -= size;
        Ok((entry >> 16) as u16)
    }

    /// Reads one symbol of a code when there are at least eight more bytes to read, so that none of it can be missing.
    #[inline(always)]
    pub(super) fn decode_ahead(&mut self, code: &Decoder) -> Result<u16, Stop> {
        if self.count < 15 {
            self.refill();
        }
        let mut entry = code.table[(self.held & ROOT_MASK) as usize];
        if entry & LINK != 0 {
            entry = code.table[(entry >> 16) as usize + ((self.held >> ROOT) & ((1 << (entry & 0xff)) - 1)) as usize];
        }
        if entry & LEAF == 0 {
            return Err(bad(code.wrong));
        }
        let size = entry & 0xff;
        self.held >>= size;
        self.count -= size;
        Ok((entry >> 16) as u16)
    }

    /// Lets go of the bits of a partly read byte, and of whole bytes that were read ahead, for what's stored as it is.
    pub(super) fn to_byte(&mut self) {
        self.at -= (self.count / 8) as usize;
        self.held = 0;
        self.count = 0;
    }

    /// Forgets what's been read, so what's held doesn't grow, but for whole bytes read ahead, which
    /// [`to_byte`](Self::to_byte) can give back.
    pub(super) fn forget_read(&mut self) {
        let ahead = (self.count / 8) as usize;
        if self.at > ahead {
            self.input.drain(..self.at - ahead);
            self.at = ahead;
        }
    }

    /// Lets go of what's left of a partly read byte, saying whether it was all zeros, as Brotli wants it to be.
    pub(super) fn pad(&mut self) -> bool {
        let spare = self.count % 8;
        let zero = self.held & ((1 << spare) - 1) == 0;
        self.held >>= spare;
        self.count -= spare;
        self.to_byte();
        zero
    }

    /// Whole bytes, for what's stored as it is, once [`to_byte`](Self::to_byte) has been called: as many as there are,
    /// up to `most`.
    pub(super) fn bytes(&mut self, most: usize) -> &[u8] {
        let from = self.at;
        let to = (from + most).min(self.input.len());
        self.at = to;
        &self.input[from..to]
    }
}

const ROOT: u32 = 10;
const ROOT_MASK: u64 = (1 << ROOT) - 1;
const LEAF: u32 = 0x200;
const LINK: u32 = 0x100;

/// A code to read, as a table: each entry is a symbol and how many bits it took, or where a second table is for codes
/// longer than the first takes.
pub(super) struct Decoder {
    table: Vec<u32>,
    /// What's wrong if something that isn't in the code is read.
    wrong: &'static str,
}

/// The code each symbol has, from its length, in the order DEFLATE and Brotli both give them: shorter codes first, and
/// symbols in order among codes of a length. These are the codes as they're read, from their first bit.
fn canonical(lengths: &[u8]) -> Vec<u32> {
    let mut counts = [0u32; 16];
    for length in lengths {
        counts[usize::from(*length)] += 1;
    }
    counts[0] = 0;
    let mut next = [0u32; 16];
    let mut code = 0;
    for bits in 1..16 {
        code = (code + counts[bits - 1]) << 1;
        next[bits] = code;
    }
    lengths
        .iter()
        .map(|length| {
            let length = usize::from(*length);
            if length == 0 {
                return 0;
            }
            let code = next[length];
            next[length] += 1;
            code.reverse_bits() >> (32 - length)
        })
        .collect()
}

impl Decoder {
    /// A table for a code of these lengths, which can't have more codes than there's room for. One that has fewer is
    /// allowed, and reading what's missing fails then.
    pub(super) fn new(lengths: &[u8], wrong: &'static str) -> Result<Self, Stop> {
        let mut room: i64 = 1 << 15;
        for length in lengths {
            if *length > 15 {
                return Err(bad(wrong));
            }
            if *length > 0 {
                room -= 1 << (15 - length);
            }
        }
        if room < 0 {
            return Err(bad(wrong));
        }
        let codes = canonical(lengths);
        let mut table = vec![0u32; 1 << ROOT];
        let mut longest = vec![0u8; 1 << ROOT];
        for (symbol, length) in lengths.iter().enumerate() {
            if u32::from(*length) > ROOT {
                let first = (codes[symbol] as u64 & ROOT_MASK) as usize;
                longest[first] = longest[first].max(*length);
            }
        }
        for (first, length) in longest.iter().enumerate() {
            if *length != 0 {
                let sub = u32::from(*length) - ROOT;
                table[first] = (table.len() as u32) << 16 | LINK | sub;
                table.resize(table.len() + (1 << sub), 0);
            }
        }
        for (symbol, length) in lengths.iter().enumerate() {
            let length = u32::from(*length);
            if length == 0 {
                continue;
            }
            let code = codes[symbol] as usize;
            let entry = (symbol as u32) << 16 | LEAF | length;
            if length <= ROOT {
                let mut at = code;
                while at < 1 << ROOT {
                    table[at] = entry;
                    at += 1 << length;
                }
            } else {
                let link = table[code & ROOT_MASK as usize];
                let (start, sub) = ((link >> 16) as usize, link & 0xff);
                let mut at = code >> ROOT;
                while at < 1 << sub {
                    table[start + at] = entry;
                    at += 1 << (length - ROOT);
                }
            }
        }
        Ok(Self { table, wrong })
    }

    /// A code with one symbol, which takes no bits at all, as Brotli has.
    pub(super) fn only(symbol: u16) -> Self {
        Self {
            table: vec![u32::from(symbol) << 16 | LEAF; 1 << ROOT],
            wrong: "",
        }
    }
}

/// Lengths of a prefix code for symbols that come this often, none longer than `limit`, which take the fewest bits
/// for them. A symbol that never comes has none. One that's alone has a length of one.
pub(super) fn code_lengths(counts: &[u32], limit: u32) -> Vec<u8> {
    let mut lengths = vec![0u8; counts.len()];
    let mut used: Vec<(u32, u16)> = counts
        .iter()
        .enumerate()
        .filter(|(_, count)| **count > 0)
        .map(|(symbol, count)| (*count, symbol as u16))
        .collect();
    if used.len() <= 1 {
        if let Some((_, symbol)) = used.first() {
            lengths[usize::from(*symbol)] = 1;
        }
        return lengths;
    }
    used.sort_unstable();
    // Huffman's way, with the leaves already in order and the nodes made in order, so that the two smallest are
    // always at the front of one or the other.
    let leaves = used.len();
    let mut weight: Vec<u64> = used.iter().map(|(count, _)| u64::from(*count)).collect();
    let mut parent = vec![0usize; leaves * 2 - 1];
    let (mut leaf, mut node) = (0, leaves);
    for _ in 0..leaves - 1 {
        let mut smallest = || {
            let take_leaf = leaf < leaves && (node >= weight.len() || weight[leaf] <= weight[node]);
            if take_leaf {
                leaf += 1;
                leaf - 1
            } else {
                node += 1;
                node - 1
            }
        };
        let (first, second) = (smallest(), smallest());
        let made = weight.len();
        weight.push(weight[first] + weight[second]);
        parent[first] = made;
        parent[second] = made;
    }
    let mut depth = vec![0u32; weight.len()];
    for at in (0..weight.len() - 1).rev() {
        depth[at] = depth[parent[at]] + 1;
    }
    // How many codes of each length, with any too long made shorter at the cost of others being made longer.
    let mut per_length = [0u32; 64];
    for at in 0..leaves {
        per_length[depth[at].min(63) as usize] += 1;
    }
    let limit = limit as usize;
    for length in limit + 1..64 {
        per_length[limit] += per_length[length];
        per_length[length] = 0;
    }
    let mut total: u64 = (1..=limit).map(|length| u64::from(per_length[length]) << (limit - length)).sum();
    while total > 1 << limit {
        per_length[limit] -= 1;
        if let Some(length) = (1..limit).rev().find(|length| per_length[*length] != 0) {
            per_length[length] -= 1;
            per_length[length + 1] += 2;
        }
        total -= 1;
    }
    // The rarest get the longest.
    let mut next = used.iter();
    for length in (1..=limit).rev() {
        for _ in 0..per_length[length] {
            if let Some((_, symbol)) = next.next() {
                lengths[usize::from(*symbol)] = length as u8;
            }
        }
    }
    lengths
}

/// The codes for these lengths, as they're written, lowest bit first.
pub(super) fn codes_of(lengths: &[u8]) -> Vec<u16> {
    canonical(lengths).into_iter().map(|code| code as u16).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_as_short_as_they_can_be_and_no_longer_than_allowed() {
        let lengths = code_lengths(&[10, 1, 1, 2, 0, 4], 15);
        assert_eq!(lengths, [1, 4, 4, 3, 0, 2]);
        // Counts like the Fibonacci numbers make the deepest tree there is.
        let mut counts = vec![1u32, 1];
        while counts.len() < 30 {
            counts.push(counts[counts.len() - 1] + counts[counts.len() - 2]);
        }
        let lengths = code_lengths(&counts, 7);
        assert!(lengths.iter().all(|length| (1..=7).contains(length)));
        let room: u32 = lengths.iter().map(|length| 1 << (7 - length)).sum();
        assert_eq!(room, 1 << 7);
    }

    #[test]
    fn what_is_written_reads_back() {
        let lengths = code_lengths(&[5, 9, 12, 13, 16, 45, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1], 15);
        let codes = codes_of(&lengths);
        let mut writer = BitWriter::default();
        let message: Vec<usize> = (0..500).map(|n| n * 7 % lengths.len()).collect();
        for symbol in &message {
            writer.put(u32::from(codes[*symbol]), u32::from(lengths[*symbol]));
            writer.put(5, 3);
        }
        writer.align();
        let decoder = Decoder::new(&lengths, "wrong").unwrap();
        let mut bits = Bits { input: writer.out, ..Bits::default() };
        for symbol in &message {
            assert_eq!(usize::from(bits.decode(&decoder).unwrap()), *symbol);
            assert_eq!(bits.take(3), Ok(5));
        }
    }
}
