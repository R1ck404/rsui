//! A small PNG writer. Most of what's in a picture of a UI is the same as what's next to it, so even this goes from
//! megabytes to tens of kilobytes, which matters when a picture has to go through a tool's result.

/// How far back a repeat can be, in the format.
const FURTHEST: usize = 32_768;
const LONGEST: usize = 258;
/// How many places are kept for finding what's been seen before.
const HASH_BITS: u32 = 15;

/// What each length of repeat is written as: where its range starts, and how many more bits say where in it.
const LENGTHS: [(u16, u8); 29] = [
    (3, 0),
    (4, 0),
    (5, 0),
    (6, 0),
    (7, 0),
    (8, 0),
    (9, 0),
    (10, 0),
    (11, 1),
    (13, 1),
    (15, 1),
    (17, 1),
    (19, 2),
    (23, 2),
    (27, 2),
    (31, 2),
    (35, 3),
    (43, 3),
    (51, 3),
    (59, 3),
    (67, 4),
    (83, 4),
    (99, 4),
    (115, 4),
    (131, 5),
    (163, 5),
    (195, 5),
    (227, 5),
    (258, 0),
];

/// The same, for how far back it is.
const DISTANCES: [(u16, u8); 30] = [
    (1, 0),
    (2, 0),
    (3, 0),
    (4, 0),
    (5, 1),
    (7, 1),
    (9, 2),
    (13, 2),
    (17, 3),
    (25, 3),
    (33, 4),
    (49, 4),
    (65, 5),
    (97, 5),
    (129, 6),
    (193, 6),
    (257, 7),
    (385, 7),
    (513, 8),
    (769, 8),
    (1025, 9),
    (1537, 9),
    (2049, 10),
    (3073, 10),
    (4097, 11),
    (6145, 11),
    (8193, 12),
    (12289, 12),
    (16385, 13),
    (24577, 13),
];

/// Writes bits, with the first going in the lowest place of each byte.
struct Bits {
    out: Vec<u8>,
    waiting: u64,
    count: u32,
}

impl Bits {
    fn put(&mut self, value: u32, bits: u32) {
        self.waiting |= u64::from(value) << self.count;
        self.count += bits;
        while self.count >= 8 {
            self.out.push(self.waiting as u8);
            self.waiting >>= 8;
            self.count -= 8;
        }
    }

    /// The codes go the other way around from everything else.
    fn put_code(&mut self, code: u32, bits: u32) {
        self.put(code.reverse_bits() >> (32 - bits), bits);
    }

    /// A byte or length, in the codes that the format has built in.
    fn put_symbol(&mut self, symbol: u32) {
        match symbol {
            0..=143 => self.put_code(0x30 + symbol, 8),
            144..=255 => self.put_code(0x190 + symbol - 144, 9),
            256..=279 => self.put_code(symbol - 256, 7),
            _ => self.put_code(0xC0 + symbol - 280, 8),
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.out.push(self.waiting as u8);
        }
        self.out
    }
}

/// Which range of a table a value is in.
fn place_in(table: &[(u16, u8)], value: usize) -> usize {
    table
        .iter()
        .rposition(|(start, _)| usize::from(*start) <= value)
        .expect("each table starts at the least there can be")
}

/// Compresses bytes with the codes that the format has built in, which leaves nothing to write down about them.
fn deflate(data: &[u8]) -> Vec<u8> {
    let mut bits = Bits {
        out: Vec::with_capacity(data.len() / 16),
        waiting: 0,
        count: 0,
    };
    // The last block, using the built-in codes.
    bits.put(1, 1);
    bits.put(1, 2);
    // Where each hash was last seen, with one more than that so that 0 means nowhere.
    let mut seen = vec![0u32; 1 << HASH_BITS];
    let hash = |at: usize| {
        let three =
            u32::from(data[at]) | u32::from(data[at + 1]) << 8 | u32::from(data[at + 2]) << 16;
        (three.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
    };
    let mut at = 0;
    while at < data.len() {
        let mut length = 0;
        let mut distance = 0;
        if at + 3 <= data.len() {
            let place = hash(at);
            let before = seen[place] as usize;
            seen[place] = at as u32 + 1;
            if before > 0 && at - (before - 1) <= FURTHEST {
                let from = before - 1;
                let most = LONGEST.min(data.len() - at);
                let same = (0..most)
                    .take_while(|&offset| data[from + offset] == data[at + offset])
                    .count();
                if same >= 3 {
                    length = same;
                    distance = at - from;
                }
            }
        }
        if length == 0 {
            bits.put_symbol(u32::from(data[at]));
            at += 1;
            continue;
        }
        let place = place_in(&LENGTHS, length);
        let (start, extra) = LENGTHS[place];
        bits.put_symbol(257 + place as u32);
        bits.put((length - usize::from(start)) as u32, u32::from(extra));
        let place = place_in(&DISTANCES, distance);
        let (start, extra) = DISTANCES[place];
        bits.put_code(place as u32, 5);
        bits.put((distance - usize::from(start)) as u32, u32::from(extra));
        // What's inside the repeat can be repeated later as well.
        for inside in at + 1..(at + length).min(data.len().saturating_sub(2)) {
            seen[hash(inside)] = inside as u32 + 1;
        }
        at += length;
    }
    bits.put_symbol(256);
    bits.finish()
}

/// Puts each row in whichever form looks like it'll compress best: as it is, or as how far it is from what's to the left
/// or above. This is the usual way of guessing, by which comes to the smallest numbers.
fn filtered(width: usize, rgba: &[u8]) -> Vec<u8> {
    let stride = width * 4;
    let height = rgba.len().checked_div(stride).unwrap_or(0);
    let mut out = Vec::with_capacity((stride + 1) * height);
    let mut left = vec![0u8; stride];
    let mut above = vec![0u8; stride];
    let cost = |row: &[u8]| -> u64 {
        row.iter()
            .map(|byte| u64::from((*byte as i8).unsigned_abs()))
            .sum()
    };
    for (number, row) in rgba.chunks_exact(stride).enumerate() {
        for at in 0..stride {
            let before = if at >= 4 { row[at - 4] } else { 0 };
            left[at] = row[at].wrapping_sub(before);
            let over = if number > 0 {
                rgba[(number - 1) * stride + at]
            } else {
                0
            };
            above[at] = row[at].wrapping_sub(over);
        }
        let options: [(u8, &[u8]); 3] = [(0, row), (1, &left), (2, &above)];
        let (kind, best) = options
            .into_iter()
            .min_by_key(|(_, row)| cost(row))
            .expect("there are three");
        out.push(kind);
        out.extend_from_slice(best);
    }
    out
}

/// A PNG file of pixels as they're drawn, whose colors are multiplied by how opaque they are, which a PNG's aren't. Only a
/// window that's seen through, as one with a backdrop is, has pixels that aren't opaque.
pub(crate) fn encode_drawn(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    if rgba.chunks_exact(4).all(|pixel| pixel[3] == 255) {
        return encode(width, height, rgba);
    }
    let mut plain = rgba.to_vec();
    for pixel in plain.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for part in &mut pixel[..3] {
                *part = ((u32::from(*part) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    encode(width, height, &plain)
}

/// A PNG file of these pixels: red, green, blue and alpha for each, in rows from the top.
pub(crate) fn encode(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    let raw = filtered(width, rgba);
    let mut zlib = vec![0x78, 0x01];
    zlib.extend(deflate(&raw));
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in raw.chunks(5_552) {
        for byte in chunk {
            a += u32::from(*byte);
            b += a;
        }
        a %= 65_521;
        b %= 65_521;
    }
    zlib.extend(((b << 16) | a).to_be_bytes());
    let mut header = Vec::new();
    header.extend((width as u32).to_be_bytes());
    header.extend((height as u32).to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    let mut file = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    for (kind, data) in [(b"IHDR", &header), (b"IDAT", &zlib), (b"IEND", &Vec::new())] {
        file.extend((data.len() as u32).to_be_bytes());
        file.extend(kind);
        file.extend(data);
        let mut crc = u32::MAX;
        for byte in kind.iter().chain(data.iter()) {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        file.extend((!crc).to_be_bytes());
    }
    file
}

#[cfg(test)]
mod tests {
    use super::encode;

    /// Reading it back with what reads pictures in an app, which checks the sums too, as well as being written apart
    /// from this.
    fn comes_back(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
        let file = encode(width, height, rgba);
        let picture = rsui_image::decode(&file, (width as u32, height as u32)).unwrap();
        assert_eq!(
            (picture.width as usize, picture.height as usize),
            (width, height)
        );
        assert_eq!(picture.pixels, rgba, "the pixels should come back the same");
        file
    }

    #[test]
    fn something_flat_comes_back_and_is_tiny() {
        let pixels: Vec<u8> = [30, 60, 90, 255].repeat(300 * 200);
        let file = comes_back(300, 200, &pixels);
        assert!(file.len() < 3_000, "{} bytes", file.len());
    }

    #[test]
    fn noise_comes_back_though_it_can_hardly_be_compressed() {
        let mut state = 0x1234_5678u32;
        let mut pixels = Vec::new();
        for _ in 0..97 * 53 {
            for _ in 0..3 {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                pixels.push(state as u8);
            }
            pixels.push(255);
        }
        comes_back(97, 53, &pixels);
    }

    #[test]
    fn stripes_steps_and_repeats_of_every_length_come_back() {
        let mut pixels = Vec::new();
        for y in 0..128u32 {
            for x in 0..256u32 {
                let shade = match y % 4 {
                    0 => (x / (y + 1)) as u8,
                    1 => (x % (y + 2)) as u8,
                    2 => y as u8,
                    _ => (x ^ y) as u8,
                };
                pixels.extend([shade, shade.wrapping_mul(3), 255 - shade, 255]);
            }
        }
        comes_back(256, 128, &pixels);
    }

    #[test]
    fn the_smallest_picture_there_is_comes_back() {
        comes_back(1, 1, &[1, 2, 3, 255]);
    }
}
