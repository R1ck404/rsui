//! Reads WebP files: lossy, lossless, with transparency, and animated.

use crate::animate::{Animation, Canvas, MAX_FRAMES, Stored};
use crate::color::{ColorTransform, ProfileKind};
use crate::scale::Scaler;
use crate::{ImageError, ImageInfo, MAX_PIXELS, Picture, fitted, vp8, vp8l};

const INVALID: ImageError = ImageError::Invalid("its structure is wrong");

pub(crate) fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
}

/// Goes through the chunks in part of a file, each as a tag and what's in it.
struct Chunks<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = (&'a [u8], &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let header = self.bytes.get(self.at..self.at + 8)?;
        let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let start = self.at + 8;
        // A chunk that's cut short still has whatever got through.
        let end = start.saturating_add(length).min(self.bytes.len());
        // They're padded out to an even size.
        self.at = start.saturating_add(length).saturating_add(length & 1);
        Some((&header[..4], &self.bytes[start..end]))
    }
}

fn chunks_of(bytes: &[u8]) -> Chunks<'_> {
    Chunks { bytes, at: 12 }
}

/// Three bytes of a number, lowest first.
fn read_24(bytes: &[u8], at: usize) -> Option<u32> {
    let part = bytes.get(at..at + 3)?;
    Some(u32::from(part[0]) | u32::from(part[1]) << 8 | u32::from(part[2]) << 16)
}

fn canvas_size(bytes: &[u8]) -> Result<(u32, u32), ImageError> {
    for (tag, body) in chunks_of(bytes) {
        match tag {
            b"VP8X" => {
                let width = read_24(body, 4).ok_or(INVALID)? + 1;
                let height = read_24(body, 7).ok_or(INVALID)? + 1;
                return Ok((width, height));
            }
            b"VP8 " => return vp8::size_of(body),
            b"VP8L" => return vp8l::size_of(body),
            _ => {}
        }
    }
    Err(ImageError::Invalid("it has no pixels"))
}

pub(crate) fn info(bytes: &[u8]) -> Result<ImageInfo, ImageError> {
    let (width, height) = canvas_size(bytes)?;
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(ImageError::TooBig);
    }
    Ok(ImageInfo { width, height })
}

/// Reads transparency that's stored apart from a lossy picture, as a value for each pixel.
fn read_alpha(body: &[u8], width: usize, height: usize) -> Result<Vec<u8>, ImageError> {
    let (&header, data) = body.split_first().ok_or(INVALID)?;
    let filter = (header >> 2) & 3;
    let compression = header & 3;
    let total = width.checked_mul(height).ok_or(ImageError::TooBig)?;
    let mut alpha = match compression {
        0 => {
            let mut plain = data.get(..total.min(data.len())).unwrap_or(&[]).to_vec();
            plain.resize(total, 0);
            plain
        }
        1 => vp8l::read_stream(data, width, height)?
            .into_iter()
            .map(|pixel| ((pixel >> 8) & 255) as u8)
            .collect(),
        _ => return Err(ImageError::Unsupported("this kind of WebP transparency")),
    };
    // Each value might be stored as how far it is from those before it.
    for y in 0..height {
        for x in 0..width {
            let at = y * width + x;
            let left = if x > 0 { Some(alpha[at - 1]) } else { None };
            let above = if y > 0 { Some(alpha[at - width]) } else { None };
            let guess = match filter {
                1 => left.or(above).unwrap_or(0),
                2 => above.or(left).unwrap_or(0),
                3 => match (left, above) {
                    (Some(left), Some(above)) => {
                        let corner = i32::from(alpha[at - width - 1]);
                        (i32::from(left) + i32::from(above) - corner).clamp(0, 255) as u8
                    }
                    (Some(only), None) | (None, Some(only)) => only,
                    (None, None) => 0,
                },
                _ => 0,
            };
            alpha[at] = alpha[at].wrapping_add(guess);
        }
    }
    Ok(alpha)
}

/// The pixels in a set of chunks that make one picture, as whole rows of red, green, blue and alpha.
fn read_picture(chunks: Chunks<'_>) -> Result<(usize, usize, Vec<u8>), ImageError> {
    let mut alpha_body = None;
    for (tag, body) in chunks {
        match tag {
            b"ALPH" => alpha_body = Some(body),
            b"VP8L" => {
                let (width, height, pixels) = vp8l::decode(body)?;
                let mut out = Vec::new();
                out.try_reserve_exact(pixels.len() * 4)
                    .map_err(|_| ImageError::TooBig)?;
                for pixel in pixels {
                    out.extend_from_slice(&[
                        (pixel >> 16) as u8,
                        (pixel >> 8) as u8,
                        pixel as u8,
                        (pixel >> 24) as u8,
                    ]);
                }
                return Ok((width as usize, height as usize, out));
            }
            b"VP8 " => {
                let frame = vp8::decode(body)?;
                let (width, height) = (frame.width, frame.height);
                let alpha = alpha_body
                    .map(|body| read_alpha(body, width, height))
                    .transpose()?;
                let mut out = Vec::new();
                out.try_reserve_exact(width * height * 4)
                    .map_err(|_| ImageError::TooBig)?;
                let mut y = 0;
                vp8::write_rows(&frame, |row| {
                    if let Some(alpha) = &alpha {
                        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
                            pixel[3] = alpha[y * width + x];
                        }
                    }
                    out.extend_from_slice(row);
                    y += 1;
                });
                return Ok((width, height, out));
            }
            _ => {}
        }
    }
    Err(ImageError::Invalid("it has no pixels"))
}

pub(crate) fn is_animated(bytes: &[u8]) -> bool {
    chunks_of(bytes).any(|(tag, _)| tag == b"ANMF")
}

fn profile_of(bytes: &[u8]) -> Option<ColorTransform> {
    let (_, body) = chunks_of(bytes).find(|(tag, _)| *tag == b"ICCP")?;
    ColorTransform::from_icc(body, ProfileKind::Rgb)
}

pub(crate) fn decode(bytes: &[u8], target: (u32, u32)) -> Result<Picture, ImageError> {
    let size = canvas_size(bytes)?;
    if u64::from(size.0) * u64::from(size.1) > MAX_PIXELS {
        return Err(ImageError::TooBig);
    }
    let fit = fitted(size, target);
    let transform = profile_of(bytes);
    // An animated one shows its first frame, drawn on an empty canvas.
    if is_animated(bytes) {
        let animation = decode_frames(bytes, target, 1)?;
        let first = animation.frames.into_iter().next().ok_or(INVALID)?;
        return Ok(Picture {
            width: animation.width,
            height: animation.height,
            pixels: first.pixels,
        });
    }
    let (width, height, pixels) = read_picture(chunks_of(bytes))?;
    if (width as u32, height as u32) != size {
        return Err(ImageError::Invalid("it has two different sizes"));
    }
    let mut scaler = Scaler::new(size, fit);
    scaler.set_transform(transform);
    for row in pixels.chunks_exact(width * 4) {
        scaler.push(row);
    }
    let (width, height) = scaler.size();
    Ok(Picture {
        width,
        height,
        pixels: scaler.finish(),
    })
}

/// Every frame of an animated file, at most this many of them, each as the whole picture at that time.
pub(crate) fn decode_frames(
    bytes: &[u8],
    target: (u32, u32),
    most: usize,
) -> Result<Animation, ImageError> {
    let size = canvas_size(bytes)?;
    if u64::from(size.0) * u64::from(size.1) > MAX_PIXELS {
        return Err(ImageError::TooBig);
    }
    let fit = fitted(size, target);
    let transform = profile_of(bytes);
    let mut canvas = Canvas::new(size)?;
    let mut stored = Stored::new(size, fit, transform);
    let mut repeats = 0;
    for (tag, body) in chunks_of(bytes) {
        if tag == b"ANIM" {
            repeats = body
                .get(4..6)
                .map_or(0, |part| u32::from(u16::from_le_bytes([part[0], part[1]])));
            continue;
        }
        if tag != b"ANMF" {
            continue;
        }
        if stored.len() >= most.min(MAX_FRAMES) {
            break;
        }
        let x = read_24(body, 0).ok_or(INVALID)? as usize * 2;
        let y = read_24(body, 3).ok_or(INVALID)? as usize * 2;
        let width = read_24(body, 6).ok_or(INVALID)? as usize + 1;
        let height = read_24(body, 9).ok_or(INVALID)? as usize + 1;
        let delay = read_24(body, 12).ok_or(INVALID)?;
        let flags = *body.get(15).ok_or(INVALID)?;
        let inner = Chunks {
            bytes: body.get(16..).ok_or(INVALID)?,
            at: 0,
        };
        let (read_width, read_height, pixels) = read_picture(inner)?;
        if read_width != width || read_height != height {
            return Err(INVALID);
        }
        let blend = flags & 2 == 0;
        let dispose_to_background = flags & 1 != 0;
        canvas.draw(&pixels, (x, y), (width, height), blend)?;
        if !stored.keep(&canvas, delay) {
            break;
        }
        if dispose_to_background {
            canvas.clear((x, y), (width, height));
        }
    }
    Ok(stored.finish(repeats))
}
