//! Reading images, including damaged ones.
//! These files are made here, so that each thing they test is plain to see.

use proptest::prelude::*;
use rsui_image::{ImageError, decode, fitted, info};

const EVERYTHING: (u32, u32) = (u32::MAX, u32::MAX);

/// A PNG file holding `raw`, which is each row with its filter byte in front, compressed as nothing
/// more than plain blocks of data.
fn png(
    width: u32,
    height: u32,
    depth: u8,
    color: u8,
    interlace: u8,
    extra: &[(&[u8; 4], &[u8])],
    raw: &[u8],
) -> Vec<u8> {
    let mut file = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |tag: &[u8; 4], body: &[u8]| {
        file.extend((body.len() as u32).to_be_bytes());
        file.extend(tag);
        file.extend(body);
        // The check on each chunk isn't read, as a mistake there can't hurt anything.
        file.extend([0; 4]);
    };
    let mut header = Vec::new();
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([depth, color, 0, 0, interlace]);
    chunk(b"IHDR", &header);
    for (tag, body) in extra {
        chunk(tag, body);
    }
    chunk(b"IDAT", &stored(raw));
    chunk(b"IEND", &[]);
    file
}

/// Data in zlib's format, without compressing it.
fn stored(raw: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut pieces = raw.chunks(65_535).peekable();
    if raw.is_empty() {
        out.extend([1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(piece) = pieces.next() {
        out.push(u8::from(pieces.peek().is_none()));
        out.extend((piece.len() as u16).to_le_bytes());
        out.extend((!(piece.len() as u16)).to_le_bytes());
        out.extend(piece);
    }
    out.extend([0; 4]);
    out
}

/// Rows with no filter, from a function giving each pixel's bytes.
fn rows(width: u32, height: u32, mut pixel: impl FnMut(u32, u32) -> Vec<u8>) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            raw.extend(pixel(x, y));
        }
    }
    raw
}

fn rgba_png(width: u32, height: u32, mut pixel: impl FnMut(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let raw = rows(width, height, |x, y| pixel(x, y).to_vec());
    png(width, height, 8, 6, 0, &[], &raw)
}

#[test]
fn reads_the_size_without_the_pixels() {
    let file = rgba_png(3, 2, |_, _| [0; 4]);
    let found = info(&file[..33]).expect("the size is at the start");
    assert_eq!((found.width, found.height), (3, 2));
    assert_eq!(info(b"not an image"), Err(ImageError::UnknownFormat));
    assert_eq!(info(b""), Err(ImageError::UnknownFormat));
}

#[test]
fn every_kind_of_png_pixel_comes_out_right() {
    let check = |file: Vec<u8>, expected: [u8; 4]| {
        let picture = decode(&file, EVERYTHING).expect("it's fine");
        assert_eq!((picture.width, picture.height), (1, 1));
        assert_eq!(picture.pixels, expected);
    };
    let solo = |depth, color, extra: &[(&[u8; 4], &[u8])], bytes: &[u8]| {
        let mut raw = vec![0];
        raw.extend(bytes);
        png(1, 1, depth, color, 0, extra, &raw)
    };
    check(solo(8, 0, &[], &[200]), [200, 200, 200, 255]);
    check(solo(16, 0, &[], &[200, 99]), [200, 200, 200, 255]);
    check(solo(1, 0, &[], &[0x80]), [255, 255, 255, 255]);
    check(solo(2, 0, &[], &[0x40]), [85, 85, 85, 255]);
    check(solo(4, 0, &[], &[0xF0]), [255, 255, 255, 255]);
    check(solo(8, 2, &[], &[10, 20, 30]), [10, 20, 30, 255]);
    check(solo(16, 2, &[], &[10, 1, 20, 2, 30, 3]), [10, 20, 30, 255]);
    check(solo(8, 4, &[], &[100, 255]), [100, 100, 100, 255]);
    check(solo(16, 4, &[], &[100, 1, 255, 2]), [100, 100, 100, 255]);
    check(solo(8, 6, &[], &[10, 20, 30, 255]), [10, 20, 30, 255]);
    check(
        solo(16, 6, &[], &[10, 0, 20, 0, 30, 0, 255, 0]),
        [10, 20, 30, 255],
    );
    // From a palette, with and without transparency.
    let palette: &[u8] = &[1, 2, 3, 40, 50, 60];
    check(solo(8, 3, &[(b"PLTE", palette)], &[1]), [40, 50, 60, 255]);
    check(
        solo(1, 3, &[(b"PLTE", palette)], &[0x80]),
        [40, 50, 60, 255],
    );
    check(
        solo(8, 3, &[(b"PLTE", palette), (b"tRNS", &[255, 0])], &[1]),
        [0, 0, 0, 0],
    );
    // A color marked as being see-through.
    check(solo(8, 0, &[(b"tRNS", &[0, 200])], &[200]), [0, 0, 0, 0]);
    check(
        solo(8, 2, &[(b"tRNS", &[0, 10, 0, 20, 0, 30])], &[10, 20, 30]),
        [0, 0, 0, 0],
    );
}

#[test]
fn colors_come_out_multiplied_by_their_alpha() {
    let picture = decode(&rgba_png(1, 1, |_, _| [200, 100, 50, 128]), EVERYTHING).unwrap();
    assert_eq!(picture.pixels, [100, 50, 25, 128]);
}

#[test]
fn each_filter_undoes_what_it_should() {
    // Two rows of three red bytes, all starting at 10 and going up by 1, filtered each way.
    let target: Vec<u8> = (10..16).collect();
    let row = |filter: u8, bytes: &[u8]| {
        let mut out = vec![filter];
        out.extend(bytes);
        out
    };
    let cases: [(u8, Vec<u8>); 5] = [
        (0, target[3..].to_vec()),
        (1, vec![13, 1, 1]),
        (2, vec![3, 3, 3]),
        (3, vec![8, 2, 2]),
        (4, vec![3, 1, 1]),
    ];
    for (filter, second) in cases {
        let mut raw = row(0, &target[..3]);
        raw.extend(row(filter, &second));
        let file = png(3, 2, 8, 0, 0, &[], &raw);
        let picture = decode(&file, EVERYTHING).expect("it's fine");
        let reds: Vec<u8> = picture.pixels.chunks(4).map(|pixel| pixel[0]).collect();
        assert_eq!(reds, target, "filter {filter}");
    }
}

#[test]
fn an_interlaced_png_is_the_same_picture() {
    let (width, height) = (13, 11);
    let pixel = |x: u32, y: u32| [(x * 19) as u8, (y * 23) as u8, (x * y) as u8, 255];
    let plain = rgba_png(width, height, pixel);
    let mut raw = Vec::new();
    for (start_x, start_y, space_x, space_y) in [
        (0, 0, 8, 8),
        (4, 0, 8, 8),
        (0, 4, 4, 8),
        (2, 0, 4, 4),
        (0, 2, 2, 4),
        (1, 0, 2, 2),
        (0, 1, 1, 2),
    ] {
        let mut y = start_y;
        while y < height {
            let mut x = start_x;
            let mut any = false;
            while x < width {
                if !any {
                    raw.push(0);
                    any = true;
                }
                raw.extend(pixel(x, y));
                x += space_x;
            }
            y += space_y;
        }
    }
    let interlaced = png(width, height, 8, 6, 1, &[], &raw);
    assert_eq!(
        decode(&plain, EVERYTHING).unwrap(),
        decode(&interlaced, EVERYTHING).unwrap()
    );
    assert_eq!(
        decode(&plain, (5, 5)).unwrap(),
        decode(&interlaced, (5, 5)).unwrap()
    );
}

#[test]
fn compressed_data_with_codes_and_repeats_is_read() {
    // Built by hand with fixed codes: the byte 0, then 7 times a repeat of it at distance 1, which is
    // a row of 8 zeros, for an image one pixel wide with a filter byte for each row.
    let mut bits: Vec<bool> = Vec::new();
    let push = |bits: &mut Vec<bool>, value: u32, count: u32, top_first: bool| {
        for index in 0..count {
            let shift = if top_first { count - 1 - index } else { index };
            bits.push(value >> shift & 1 == 1);
        }
    };
    push(&mut bits, 1, 1, false); // Last block.
    push(&mut bits, 1, 2, false); // Fixed codes.
    push(&mut bits, 0b0011_0000, 8, true); // The byte 0.
    push(&mut bits, 0b000_0101, 7, true); // Symbol 261: repeat 7 bytes.
    push(&mut bits, 0, 5, true); // Distance code 0: from 1 back.
    push(&mut bits, 0, 7, true); // The end.
    let mut packed = vec![0x78, 0x01];
    for piece in bits.chunks(8) {
        let mut byte = 0u8;
        for (index, bit) in piece.iter().enumerate() {
            byte |= u8::from(*bit) << index;
        }
        packed.push(byte);
    }
    let mut file = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |tag: &[u8; 4], body: &[u8]| {
        file.extend((body.len() as u32).to_be_bytes());
        file.extend(tag);
        file.extend(body);
        file.extend([0; 4]);
    };
    let mut header = Vec::new();
    header.extend(1u32.to_be_bytes());
    header.extend(4u32.to_be_bytes());
    header.extend([8, 0, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &packed);
    chunk(b"IEND", &[]);
    let picture = decode(&file, EVERYTHING).expect("it's fine");
    assert_eq!((picture.width, picture.height), (1, 4));
    assert!(
        picture
            .pixels
            .chunks(4)
            .all(|pixel| pixel == [0, 0, 0, 255])
    );
}

#[test]
fn a_repeat_can_reach_all_the_way_back_across_what_was_handed_out() {
    // More data than the window holds at once, in several pieces, so it gets handed out as it goes.
    let (width, height) = (300, 300);
    let pixel = |x: u32, y: u32| [(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8, 255];
    let picture = decode(&rgba_png(width, height, pixel), EVERYTHING).unwrap();
    for (index, found) in picture.pixels.chunks(4).enumerate() {
        let (x, y) = (index as u32 % width, index as u32 / width);
        assert_eq!(found, pixel(x, y), "at {x},{y}");
    }
}

#[test]
fn shrinking_averages_what_each_new_pixel_covers() {
    // Black and white in alternate columns and rows average out to grey at half the size.
    let checker = rgba_png(8, 8, |x, y| {
        let value = if (x + y) % 2 == 0 { 0 } else { 255 };
        [value, value, value, 255]
    });
    let picture = decode(&checker, (4, 4)).unwrap();
    assert_eq!((picture.width, picture.height), (4, 4));
    for pixel in picture.pixels.chunks(4) {
        assert!(pixel[0].abs_diff(128) <= 1, "{pixel:?}");
        assert_eq!(pixel[3], 255);
    }
    // At a fraction that doesn't divide evenly, a solid color stays exactly that color.
    let solid = rgba_png(7, 13, |_, _| [10, 200, 90, 255]);
    let picture = decode(&solid, (3, 5)).unwrap();
    for pixel in picture.pixels.chunks(4) {
        for (found, wanted) in pixel.iter().zip([10u8, 200, 90, 255]) {
            assert!(found.abs_diff(wanted) <= 1, "{pixel:?}");
        }
    }
}

#[test]
fn shrinking_weighs_colors_by_how_see_through_they_are() {
    // A red pixel and a see-through blue one make half-see-through red, with no blue in it.
    let file = rgba_png(2, 1, |x, _| {
        if x == 0 {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 0]
        }
    });
    let picture = decode(&file, (1, 1)).unwrap();
    assert!(picture.pixels[0].abs_diff(128) <= 1, "{:?}", picture.pixels);
    assert_eq!(picture.pixels[2], 0, "{:?}", picture.pixels);
    assert!(picture.pixels[3].abs_diff(128) <= 1, "{:?}", picture.pixels);
}

#[test]
fn it_never_comes_out_bigger_than_it_is_and_keeps_its_shape() {
    let file = rgba_png(4, 2, |_, _| [1, 2, 3, 255]);
    let picture = decode(&file, (100, 100)).unwrap();
    assert_eq!((picture.width, picture.height), (4, 2));
    let picture = decode(&file, (2, 100)).unwrap();
    assert_eq!((picture.width, picture.height), (2, 1));
    let picture = decode(&file, (100, 1)).unwrap();
    assert_eq!((picture.width, picture.height), (2, 1));
    let picture = decode(&file, (0, 0)).unwrap();
    assert_eq!((picture.width, picture.height), (1, 1));
}

#[test]
fn a_file_cut_short_shows_what_got_through() {
    let file = rgba_png(4, 100, |_, _| [9, 9, 9, 255]);
    let cut = &file[..file.len() / 2];
    let picture = decode(cut, EVERYTHING).expect("half a picture is better than none");
    assert_eq!(&picture.pixels[..4], [9, 9, 9, 255]);
    assert_eq!(picture.height, 100);
    // With nothing to show at all, that's an error.
    assert!(decode(&file[..40], EVERYTHING).is_err());
}

#[test]
fn an_impossible_size_is_refused_at_once() {
    let file = png(1 << 20, 1 << 20, 8, 6, 0, &[], &[0]);
    assert_eq!(info(&file), Err(ImageError::TooBig));
    assert_eq!(decode(&file, EVERYTHING), Err(ImageError::TooBig));
}

#[test]
fn a_tiny_file_saying_it_is_huge_uses_little_memory_and_time() {
    // It says it has a quarter of a billion pixels, with next to no data, and an image that size can
    // still be shown small. This has to find out it's cut short without making room for it all.
    let file = png(16000, 16000, 8, 6, 0, &[], &[0; 100]);
    let started = std::time::Instant::now();
    let picture = decode(&file, (10, 10)).expect("what's there can be shown");
    assert_eq!((picture.width, picture.height), (10, 10));
    assert!(started.elapsed().as_secs() < 5);
}

// JPEG. These encode a picture of blocks with one color each, which is all a block's first number
// says, and which needs very little of the format.

/// One bit at a time, with JPEG's padding after any 0xFF.
#[derive(Default)]
struct JpegBits {
    out: Vec<u8>,
    held: u32,
    count: u32,
}

impl JpegBits {
    fn put(&mut self, value: u32, bits: u32) {
        for index in (0..bits).rev() {
            self.held = (self.held << 1) | (value >> index & 1);
            self.count += 1;
            if self.count == 8 {
                self.out.push(self.held as u8);
                if self.held as u8 == 0xFF {
                    self.out.push(0);
                }
                self.held = 0;
                self.count = 0;
            }
        }
    }

    fn finish(mut self) -> Vec<u8> {
        while self.count != 0 {
            self.put(1, 1);
        }
        self.out
    }
}

/// How a block's first number is written, as its size in bits and those bits.
fn size_of_value(value: i32) -> (u32, u32) {
    if value == 0 {
        return (0, 0);
    }
    let size = 32 - value.unsigned_abs().leading_zeros();
    let bits = if value > 0 {
        value as u32
    } else {
        (value + (1 << size) - 1) as u32
    };
    (size, bits)
}

/// A table with a code for each size of number: each is 4 bits, and 12 sizes fit.
fn dc_table() -> Vec<u8> {
    let mut body = vec![0x00];
    let mut counts = [0u8; 16];
    counts[3] = 12;
    body.extend(counts);
    body.extend(0..12u8);
    section(0xC4, &body)
}

/// A table for the rest of a block, with one code, for "nothing else in this block".
fn ac_table() -> Vec<u8> {
    let mut body = vec![0x10];
    let mut counts = [0u8; 16];
    counts[0] = 1;
    body.extend(counts);
    body.push(0x00);
    section(0xC4, &body)
}

fn section(marker: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0xFF, marker];
    out.extend(((body.len() + 2) as u16).to_be_bytes());
    out.extend(body);
    out
}

/// The numbers for each block's first number, once it's divided by 1, which give these sample values.
fn dc_of(value: u8) -> i32 {
    (i32::from(value) - 128) * 8
}

fn frame(marker: u8, width: u16, height: u16) -> Vec<u8> {
    let mut body = vec![8];
    body.extend(height.to_be_bytes());
    body.extend(width.to_be_bytes());
    body.push(1);
    body.extend([1, 0x11, 0]);
    section(marker, &body)
}

fn quantization() -> Vec<u8> {
    let mut body = vec![0];
    body.extend([1; 64]);
    section(0xDB, &body)
}

/// A grey JPEG, with one value for each 8 by 8 block.
fn grey_jpeg(width: u16, height: u16, values: &[u8], progressive: bool, app: &[u8]) -> Vec<u8> {
    let mut file = vec![0xFF, 0xD8];
    file.extend(app);
    file.extend(quantization());
    file.extend(frame(if progressive { 0xC2 } else { 0xC0 }, width, height));
    file.extend(dc_table());
    file.extend(ac_table());
    let mut bits = JpegBits::default();
    let mut previous = 0;
    for value in values {
        let dc = dc_of(*value);
        let (size, extra) = size_of_value(dc - previous);
        previous = dc;
        // The 4-bit codes count up from 0, by size.
        bits.put(size, 4);
        bits.put(extra, size);
        if !progressive {
            bits.put(0, 1);
        }
    }
    let scan_bits = bits.finish();
    // The first number of every block, or all of it for the plain kind.
    let end = if progressive { 0 } else { 63 };
    file.extend(section(0xDA, &[1, 1, 0x00, 0, end, 0]));
    file.extend(scan_bits);
    if progressive {
        // The rest of each block, which is nothing, one block at a time.
        let mut bits = JpegBits::default();
        for _ in values {
            bits.put(0, 1);
        }
        file.extend(section(0xDA, &[1, 1, 0x00, 1, 63, 0]));
        file.extend(bits.finish());
    }
    file.extend([0xFF, 0xD9]);
    file
}

#[test]
fn a_plain_jpeg_comes_out_right() {
    let file = grey_jpeg(16, 8, &[40, 200], false, &[]);
    assert_eq!(
        info(&file).map(|found| (found.width, found.height)),
        Ok((16, 8))
    );
    let picture = decode(&file, EVERYTHING).expect("it's fine");
    assert_eq!((picture.width, picture.height), (16, 8));
    for y in 0..8 {
        let at = |x: usize| picture.pixels[(y * 16 + x) * 4];
        assert!(at(3).abs_diff(40) <= 1, "{}", at(3));
        assert!(at(12).abs_diff(200) <= 1, "{}", at(12));
    }
    assert!(picture.pixels.chunks(4).all(|pixel| pixel[3] == 255));
}

#[test]
fn a_progressive_jpeg_is_the_same_picture() {
    let values = [10, 250, 128, 60];
    let plain = grey_jpeg(32, 8, &values, false, &[]);
    let progressive = grey_jpeg(32, 8, &values, true, &[]);
    assert_eq!(
        decode(&plain, EVERYTHING).unwrap(),
        decode(&progressive, EVERYTHING).unwrap()
    );
}

#[test]
fn a_jpeg_at_a_fraction_of_its_size_has_the_same_colors() {
    let file = grey_jpeg(32, 8, &[10, 250, 128, 60], false, &[]);
    for (target, size) in [
        ((16, 4), (16, 4)),
        ((8, 2), (8, 2)),
        ((4, 1), (4, 1)),
        ((10, 10), (10, 3)),
    ] {
        let picture = decode(&file, target).expect("it's fine");
        assert_eq!((picture.width, picture.height), size);
    }
    let picture = decode(&file, (4, 1)).unwrap();
    let reds: Vec<u8> = picture.pixels.chunks(4).map(|pixel| pixel[0]).collect();
    for (found, wanted) in reds.iter().zip([10u8, 250, 128, 60]) {
        assert!(found.abs_diff(wanted) <= 1, "{reds:?}");
    }
    let picture = decode(&file, (16, 4)).unwrap();
    assert!(picture.pixels[0].abs_diff(10) <= 1, "{}", picture.pixels[0]);
}

fn exif_orientation(orientation: u16) -> Vec<u8> {
    let mut body = b"Exif\0\0".to_vec();
    body.extend(b"MM\0\x2A\0\0\0\x08");
    body.extend([0, 1]);
    body.extend([0x01, 0x12, 0, 3, 0, 0, 0, 1]);
    body.extend(orientation.to_be_bytes());
    body.extend([0, 0]);
    section(0xE1, &body)
}

#[test]
fn a_photo_is_turned_the_way_the_camera_says() {
    let (dark, light) = (20u8, 230u8);
    let values = [dark, light];
    // As stored: dark on the left, light on the right. Orientation 6 turns it clockwise, which puts
    // the dark end on top. 3 turns it right around. 8 turns it the other way.
    let cases = [
        (1u16, (16, 8), (0usize, 0usize), dark),
        (2, (16, 8), (0, 0), light),
        (3, (16, 8), (0, 0), light),
        (6, (8, 16), (0, 0), dark),
        (8, (8, 16), (0, 0), light),
    ];
    for (orientation, size, (x, y), wanted) in cases {
        let file = grey_jpeg(16, 8, &values, false, &exif_orientation(orientation));
        let found = info(&file).unwrap();
        assert_eq!(
            (found.width, found.height),
            size,
            "orientation {orientation}"
        );
        let picture = decode(&file, EVERYTHING).unwrap();
        assert_eq!(
            (picture.width, picture.height),
            size,
            "orientation {orientation}"
        );
        let at = (y * picture.width as usize + x) * 4;
        assert!(
            picture.pixels[at].abs_diff(wanted) <= 1,
            "orientation {orientation}: {}",
            picture.pixels[at]
        );
    }
    // It still fits what it's asked to, the way it'll be shown.
    let file = grey_jpeg(16, 8, &values, false, &exif_orientation(6));
    let picture = decode(&file, (4, 8)).unwrap();
    assert_eq!((picture.width, picture.height), (4, 8));
}

#[test]
fn a_jpeg_cut_short_shows_what_got_through() {
    let file = grey_jpeg(160, 160, &[99; 400], false, &[]);
    let cut = &file[..file.len() - 200];
    let picture = decode(cut, EVERYTHING).expect("part of a picture is better than none");
    assert!(picture.pixels[0].abs_diff(99) <= 1);
}

#[test]
fn jpegs_that_are_not_supported_say_so() {
    let mut file = vec![0xFF, 0xD8];
    let mut body = vec![16];
    body.extend([0, 8, 0, 8, 1, 1, 0x11, 0]);
    file.extend(section(0xC0, &body));
    assert!(matches!(
        decode(&file, EVERYTHING),
        Err(ImageError::Unsupported(_))
    ));
    let mut file = vec![0xFF, 0xD8];
    // Hierarchical.
    file.extend(frame(0xC5, 8, 8));
    assert!(matches!(
        decode(&file, EVERYTHING),
        Err(ImageError::Unsupported(_))
    ));
}

#[test]
fn fitting_keeps_the_shape_and_stays_inside() {
    assert_eq!(fitted((100, 50), (50, 50)), (50, 25));
    assert_eq!(fitted((50, 100), (50, 50)), (25, 50));
    assert_eq!(fitted((100, 50), (1000, 1000)), (100, 50));
    assert_eq!(fitted((1000, 1), (10, 10)), (10, 1));
    assert_eq!(fitted((1, 1000), (10, 10)), (1, 10));
    assert_eq!(fitted((0, 0), (0, 0)), (1, 1));
}

fn a_file() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        Just(rgba_png(9, 7, |x, y| [
            (x * 20) as u8,
            (y * 30) as u8,
            7,
            200
        ])),
        Just(png(
            3,
            3,
            8,
            3,
            0,
            &[(b"PLTE", &[1, 2, 3, 4, 5, 6])],
            &[0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0, 0]
        )),
        Just(png(8, 8, 8, 0, 1, &[], &[0; 80])),
        Just(grey_jpeg(32, 8, &[10, 250, 128, 60], false, &[])),
        Just(grey_jpeg(32, 8, &[10, 250, 128, 60], true, &[])),
        Just(grey_jpeg(16, 8, &[10, 250], false, &exif_orientation(6))),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    /// A file could hold anything, and none of it may panic or hang.
    #[test]
    fn damaged_files_never_panic(
        file in a_file(),
        changes in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..16),
        cut in any::<prop::sample::Index>(),
        truncate: bool,
        target in (1u32..64, 1u32..64),
    ) {
        let mut file = file;
        for (at, value) in changes {
            let at = at.index(file.len());
            file[at] = value;
        }
        if truncate {
            file.truncate(cut.index(file.len()));
        }
        let _ = info(&file);
        if let Ok(picture) = decode(&file, target) {
            prop_assert_eq!(picture.pixels.len(), picture.width as usize * picture.height as usize * 4);
            prop_assert!(picture.width >= 1 && picture.height >= 1);
        }
    }

    #[test]
    fn noise_never_panics(
        kind in 0usize..3,
        noise in proptest::collection::vec(any::<u8>(), 0..512),
    ) {
        let mut file = match kind {
            0 => Vec::new(),
            1 => vec![0xFF, 0xD8],
            _ => vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        };
        file.extend(noise);
        let _ = info(&file);
        let _ = decode(&file, (32, 32));
    }

    /// What comes out always fits what was asked, and is as big as it can be without going over.
    #[test]
    fn fitting_stays_inside_and_uses_the_room(
        size in (1u32..5000, 1u32..5000),
        target in (1u32..5000, 1u32..5000),
    ) {
        let (width, height) = fitted(size, target);
        prop_assert!(width <= size.0.max(target.0) && height <= size.1.max(target.1));
        prop_assert!(width <= target.0.max(size.0.min(target.0)));
        if size.0 > target.0 || size.1 > target.1 {
            prop_assert!(width <= target.0 && height <= target.1, "{width}x{height}");
            prop_assert!(width == target.0 || height == target.1, "{width}x{height}");
        } else {
            prop_assert_eq!((width, height), size);
        }
    }

    /// Any size a picture is read at holds a solid color as that color.
    #[test]
    fn a_solid_picture_stays_solid_at_any_size(
        size in (1u32..40, 1u32..40),
        target in (1u32..40, 1u32..40),
        color: [u8; 3],
    ) {
        let file = rgba_png(size.0, size.1, |_, _| [color[0], color[1], color[2], 255]);
        let picture = decode(&file, target).unwrap();
        for pixel in picture.pixels.chunks(4) {
            for (found, wanted) in pixel.iter().zip(color) {
                prop_assert!(found.abs_diff(wanted) <= 1, "{:?} for {:?}", pixel, color);
            }
            prop_assert!(pixel[3] >= 254);
        }
    }
}

#[test]
fn shrinking_multiplies_every_color_by_its_alpha() {
    let file = rgba_png(2, 2, |_, _| [200, 100, 50, 128]);
    let picture = decode(&file, (1, 1)).unwrap();
    for (found, wanted) in picture.pixels.iter().zip([100u8, 50, 25, 128]) {
        assert!(found.abs_diff(wanted) <= 1, "{:?}", picture.pixels);
    }
}

#[test]
fn repeats_reach_back_across_what_has_already_been_handed_out() {
    // A pattern of 300 different-ish bytes, then repeats of it from 300 back, for far more than
    // the window holds, so that every repeat after the first slide relies on what was kept.
    let mut bits: Vec<bool> = Vec::new();
    let put = |bits: &mut Vec<bool>, value: u32, count: u32| {
        for index in (0..count).rev() {
            bits.push(value >> index & 1 == 1);
        }
    };
    let put_low_first = |bits: &mut Vec<bool>, value: u32, count: u32| {
        for index in 0..count {
            bits.push(value >> index & 1 == 1);
        }
    };
    let literal = |bits: &mut Vec<bool>, byte: u8| {
        if byte < 144 {
            put(bits, 0x30 + u32::from(byte), 8);
        } else {
            put(bits, 0x190 + u32::from(byte) - 144, 9);
        }
    };
    put_low_first(&mut bits, 1, 1); // Last block.
    put_low_first(&mut bits, 1, 2); // Fixed codes.
    literal(&mut bits, 0); // The row's filter.
    let pattern: Vec<u8> = (0..300u32).map(|index| (index * 7 % 251) as u8).collect();
    for &byte in &pattern {
        literal(&mut bits, byte);
    }
    let width = 100_000usize;
    let mut written = pattern.len();
    while written + 258 <= width {
        // Symbol 285, a length of 258, whose code is 8 bits.
        put(&mut bits, 0xC0 + (285 - 280), 8);
        // Distance code 16 covers 257 to 384, with 7 more bits.
        put(&mut bits, 16, 5);
        put_low_first(&mut bits, 300 - 257, 7);
        written += 258;
    }
    while written < width {
        literal(&mut bits, pattern[written % 300]);
        written += 1;
    }
    put(&mut bits, 0, 7); // The end.
    let mut packed = vec![0x78, 0x01];
    for piece in bits.chunks(8) {
        let mut byte = 0u8;
        for (index, bit) in piece.iter().enumerate() {
            byte |= u8::from(*bit) << index;
        }
        packed.push(byte);
    }
    let mut file = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |tag: &[u8; 4], body: &[u8]| {
        file.extend((body.len() as u32).to_be_bytes());
        file.extend(tag);
        file.extend(body);
        file.extend([0; 4]);
    };
    let mut header = Vec::new();
    header.extend((width as u32).to_be_bytes());
    header.extend(1u32.to_be_bytes());
    header.extend([8, 0, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &packed);
    chunk(b"IEND", &[]);
    let picture = decode(&file, EVERYTHING).expect("it's fine");
    assert_eq!(picture.width as usize, width);
    for (index, pixel) in picture.pixels.chunks(4).enumerate() {
        assert_eq!(pixel[0], pattern[index % 300], "at {index}");
    }
}
