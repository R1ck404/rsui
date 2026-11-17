//! The bitmaps Windows keeps on its clipboard, as `CF_DIB`: a `BITMAPINFOHEADER`, then the pixels. Most programs that paste a
//! picture read only this, and a picture copied from most of them, or with Print Screen, is only there this way.

use crate::MAX_PIXELS;

const HEADER: usize = 40;
const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;

/// A picture as a `CF_DIB`: 32 bits a pixel, blue first, in rows from the bottom, which is what's read most surely. `pixels` is
/// red, green, blue and alpha for each pixel, in rows from the top.
pub fn encode(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let row = width as usize * 4;
    let mut out = Vec::with_capacity(HEADER + row * height as usize);
    out.extend_from_slice(&(HEADER as u32).to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&BI_RGB.to_le_bytes());
    out.extend_from_slice(&((row * height as usize) as u32).to_le_bytes());
    // How many pixels a meter, which is 96 an inch, and no colors in a table.
    out.extend_from_slice(&3780i32.to_le_bytes());
    out.extend_from_slice(&3780i32.to_le_bytes());
    out.extend_from_slice(&[0; 8]);
    for line in pixels.chunks_exact(row.max(1)).take(height as usize).rev() {
        for pixel in line.chunks_exact(4) {
            out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    out
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

/// What a `CF_DIB` holds, as `(width, height, pixels)`, with the pixels red, green, blue and alpha in rows from the top, and
/// alpha as it was given. A bitmap of 24 bits a pixel, or of 32 with the usual order, is read; with 32 bits and no alpha in
/// any pixel, as most programs leave it, it's opaque. Anything else, or anything damaged, is `None`.
pub fn decode(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let header = u32_at(bytes, 0)? as usize;
    let width = i32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?);
    let height = i32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?);
    let bits = u16_at(bytes, 14)?;
    let compression = u32_at(bytes, 16)?;
    let colors = u32_at(bytes, 32)? as usize;
    if header < HEADER || width <= 0 || height == 0 || height == i32::MIN {
        return None;
    }
    let (width, from_top) = (width as u32, height < 0);
    let height = height.unsigned_abs();
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return None;
    }
    let mut start = header.checked_add(colors.checked_mul(4)?)?;
    match (bits, compression) {
        (24 | 32, BI_RGB) => {}
        (32, BI_BITFIELDS) => {
            // The masks come after a plain header, before the pixels, and are the start of what's after it in a longer one.
            // Either way they're at the same place. Only the usual order is read.
            if header == HEADER {
                start = start.checked_add(12)?;
            }
            let wanted = [0x00FF_0000, 0x0000_FF00, 0x0000_00FF];
            for (at, mask) in wanted.into_iter().enumerate() {
                if u32_at(bytes, HEADER + at * 4)? != mask {
                    return None;
                }
            }
        }
        _ => return None,
    }
    let step = usize::from(bits / 8);
    let row = (width as usize * step).checked_add(3)? & !3;
    let all = bytes.get(start..start.checked_add(row.checked_mul(height as usize)?)?)?;
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        let line = if from_top { y } else { height as usize - 1 - y };
        let line = &all[line * row..line * row + width as usize * step];
        for pixel in line.chunks_exact(step) {
            pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0], if step == 4 { pixel[3] } else { 255 }]);
        }
    }
    if bits == 32 && pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) {
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    }
    Some((width, height, pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two by three, each pixel different, with some alpha.
    fn picture() -> Vec<u8> {
        (0..6u8).flat_map(|at| [at * 40, 255 - at * 40, at, 255 - at]).collect()
    }

    #[test]
    fn a_picture_comes_back_as_it_went() {
        let dib = encode(2, 3, &picture());
        assert_eq!(dib.len(), 40 + 2 * 3 * 4);
        assert_eq!(decode(&dib), Some((2, 3, picture())));
    }

    #[test]
    fn rows_go_from_the_bottom_and_blue_comes_first() {
        let dib = encode(2, 3, &picture());
        // The first pixel stored is the bottom row's first, which is the fifth.
        assert_eq!(&dib[40..44], &[4, 255 - 160, 160, 251]);
    }

    #[test]
    fn a_bitmap_with_no_alpha_anywhere_is_opaque() {
        let mut dib = encode(2, 3, &picture());
        for pixel in dib[40..].chunks_exact_mut(4) {
            pixel[3] = 0;
        }
        let (_, _, pixels) = decode(&dib).unwrap();
        assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
        assert_eq!(&pixels[..3], &[0, 255, 0]);
    }

    #[test]
    fn twenty_four_bits_with_padded_rows_from_the_top_are_read() {
        // Three wide, so each row of nine bytes is padded to twelve.
        let mut dib = encode(3, 2, &[0; 24])[..40].to_vec();
        dib[8..12].copy_from_slice(&(-2i32).to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        dib.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 0, 0, 10, 11, 12, 13, 14, 15, 16, 17, 18, 0, 0, 0]);
        let (width, height, pixels) = decode(&dib).unwrap();
        assert_eq!((width, height), (3, 2));
        assert_eq!(&pixels[..8], &[3, 2, 1, 255, 6, 5, 4, 255]);
        assert_eq!(&pixels[12..16], &[12, 11, 10, 255]);
    }

    #[test]
    fn masks_after_the_header_are_read_and_odd_ones_are_not() {
        let plain = encode(2, 3, &picture());
        let mut dib = plain[..40].to_vec();
        dib[16..20].copy_from_slice(&BI_BITFIELDS.to_le_bytes());
        for mask in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF] {
            dib.extend_from_slice(&mask.to_le_bytes());
        }
        dib.extend_from_slice(&plain[40..]);
        assert_eq!(decode(&dib), Some((2, 3, picture())));
        dib[40..44].copy_from_slice(&0x0000_00FFu32.to_le_bytes());
        assert_eq!(decode(&dib), None);
    }

    #[test]
    fn what_is_damaged_or_unknown_is_nothing() {
        let dib = encode(2, 3, &picture());
        assert_eq!(decode(&dib[..50]), None);
        assert_eq!(decode(&dib[..10]), None);
        let mut eight = dib.clone();
        eight[14..16].copy_from_slice(&8u16.to_le_bytes());
        assert_eq!(decode(&eight), None);
        let mut huge = dib;
        huge[4..8].copy_from_slice(&i32::MAX.to_le_bytes());
        huge[8..12].copy_from_slice(&i32::MAX.to_le_bytes());
        assert_eq!(decode(&huge), None);
    }
}
