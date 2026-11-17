//! Reads PNG files, row by row, and animated ones frame by frame.

use crate::animate::{Animation, Canvas, MAX_FRAMES, Stored};
use crate::color::{ColorTransform, ProfileKind};
use crate::inflate::inflate;
use crate::scale::Scaler;
use crate::{ImageError, ImageInfo, MAX_PIXELS, Picture, fitted};

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

pub(crate) fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&SIGNATURE)
}

#[derive(Clone, Copy)]
struct Header {
    width: u32,
    height: u32,
    depth: u8,
    color: u8,
    interlaced: bool,
}

impl Header {
    fn channels(self) -> usize {
        match self.color {
            2 => 3,
            4 => 2,
            6 => 4,
            _ => 1,
        }
    }

    /// How many bytes a row this wide holds, leaving out its filter byte.
    fn row_bytes(self, width: u32) -> usize {
        (width as usize * self.channels() * self.depth as usize).div_ceil(8)
    }
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let part = bytes.get(at..at + 4)?;
    Some(u32::from_be_bytes([part[0], part[1], part[2], part[3]]))
}

fn read_header(bytes: &[u8]) -> Result<Header, ImageError> {
    let invalid = ImageError::Invalid("its header is wrong");
    if bytes.get(12..16) != Some(b"IHDR") || read_u32(bytes, 8) != Some(13) {
        return Err(invalid);
    }
    let width = read_u32(bytes, 16).ok_or(invalid)?;
    let height = read_u32(bytes, 20).ok_or(invalid)?;
    let field = |at: usize| bytes.get(at).copied().ok_or(invalid);
    let (depth, color) = (field(24)?, field(25)?);
    if field(26)? != 0 || field(27)? != 0 {
        return Err(invalid);
    }
    let interlaced = match field(28)? {
        0 => false,
        1 => true,
        _ => return Err(invalid),
    };
    let allowed = match color {
        0 => matches!(depth, 1 | 2 | 4 | 8 | 16),
        3 => matches!(depth, 1 | 2 | 4 | 8),
        2 | 4 | 6 => matches!(depth, 8 | 16),
        _ => false,
    };
    if !allowed || width == 0 || height == 0 {
        return Err(invalid);
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(ImageError::TooBig);
    }
    Ok(Header {
        width,
        height,
        depth,
        color,
        interlaced,
    })
}

pub(crate) fn info(bytes: &[u8]) -> Result<ImageInfo, ImageError> {
    let header = read_header(bytes)?;
    Ok(ImageInfo {
        width: header.width,
        height: header.height,
    })
}

/// What a file says about colors, in the chunks before its pixels.
#[derive(Default)]
struct Colors<'a> {
    palette: &'a [u8],
    transparency: &'a [u8],
    /// Whether it says it's in the color space screens show.
    is_screen_colors: bool,
    profile: &'a [u8],
    gamma: Option<f32>,
    primaries: Option<[f32; 8]>,
}

impl Colors<'_> {
    /// Takes what's in a chunk, if it's something about colors.
    fn read<'a>(colors: &mut Colors<'a>, tag: &[u8], body: &'a [u8]) {
        match tag {
            b"PLTE" => colors.palette = body,
            b"tRNS" => colors.transparency = body,
            b"sRGB" => colors.is_screen_colors = true,
            b"iCCP" => colors.profile = body,
            b"gAMA" => colors.gamma = read_u32(body, 0).map(|value| value as f32 / 100_000.0),
            b"cHRM" => {
                let mut values = [0.0; 8];
                let mut whole = true;
                for (index, value) in values.iter_mut().enumerate() {
                    match read_u32(body, index * 4) {
                        Some(found) => *value = found as f32 / 100_000.0,
                        None => whole = false,
                    }
                }
                if whole {
                    colors.primaries = Some(values);
                }
            }
            _ => {}
        }
    }

    /// How to turn this file's colors into what a screen shows. What's most specific comes first.
    fn transform(&self, header: Header) -> Option<ColorTransform> {
        if self.is_screen_colors {
            return None;
        }
        if !self.profile.is_empty() {
            let kind = if matches!(header.color, 0 | 4) {
                ProfileKind::Gray
            } else {
                ProfileKind::Rgb
            };
            return unpack_profile(self.profile)
                .and_then(|profile| ColorTransform::from_icc(&profile, kind));
        }
        ColorTransform::from_gamma_and_primaries(self.gamma, self.primaries)
    }
}

/// The most a color profile can unpack to. Real ones are a few megabytes at the very most.
const MOST_PROFILE_BYTES: usize = 32 * 1024 * 1024;

/// A profile is stored after its name, packed the same way as the pixels.
fn unpack_profile(body: &[u8]) -> Option<Vec<u8>> {
    let name_end = body.iter().position(|byte| *byte == 0)?;
    let packed = body.get(name_end + 2..)?;
    let mut out = Vec::new();
    inflate(&[packed], |chunk| {
        if out.len() + chunk.len() > MOST_PROFILE_BYTES {
            return Err(ImageError::TooBig);
        }
        out.extend_from_slice(chunk);
        Ok(false)
    })
    .ok()?;
    Some(out)
}

/// Turns rows as the file stores them into red, green, blue and alpha.
fn to_rgba(header: Header, colors: &Colors<'_>, raw: &[u8], width: usize, out: &mut [u8]) {
    let depth = header.depth as usize;
    // The value at a position among the row's samples, for depths below a byte.
    let sample = |index: usize| -> u8 {
        let bit = index * depth;
        let byte = raw[bit / 8];
        let shift = 8 - depth - bit % 8;
        (byte >> shift) & ((1u16 << depth) - 1) as u8
    };
    let key = |at: usize| -> Option<u16> {
        let part = colors.transparency.get(at..at + 2)?;
        Some(u16::from_be_bytes([part[0], part[1]]))
    };
    match header.color {
        0 => {
            let wanted = key(0);
            for (x, pixel) in out.chunks_exact_mut(4).take(width).enumerate() {
                let (full, value) = if depth == 16 {
                    (u16::from_be_bytes([raw[x * 2], raw[x * 2 + 1]]), raw[x * 2])
                } else if depth == 8 {
                    (u16::from(raw[x]), raw[x])
                } else {
                    let found = sample(x);
                    let top = (1u16 << depth) - 1;
                    (u16::from(found), (u16::from(found) * 255 / top) as u8)
                };
                pixel.copy_from_slice(&[
                    value,
                    value,
                    value,
                    if wanted == Some(full) { 0 } else { 255 },
                ]);
            }
        }
        2 => {
            let wanted = (key(0), key(2), key(4));
            let step = if depth == 16 { 6 } else { 3 };
            for (from, pixel) in raw
                .chunks_exact(step)
                .zip(out.chunks_exact_mut(4))
                .take(width)
            {
                let (r, g, b) = if depth == 16 {
                    (from[0], from[2], from[4])
                } else {
                    (from[0], from[1], from[2])
                };
                let is_key = wanted.0.is_some()
                    && if depth == 16 {
                        wanted
                            == (
                                Some(u16::from_be_bytes([from[0], from[1]])),
                                Some(u16::from_be_bytes([from[2], from[3]])),
                                Some(u16::from_be_bytes([from[4], from[5]])),
                            )
                    } else {
                        wanted
                            == (
                                Some(u16::from(from[0])),
                                Some(u16::from(from[1])),
                                Some(u16::from(from[2])),
                            )
                    };
                pixel.copy_from_slice(&[r, g, b, if is_key { 0 } else { 255 }]);
            }
        }
        3 => {
            for (x, pixel) in out.chunks_exact_mut(4).take(width).enumerate() {
                let index = if depth == 8 { raw[x] } else { sample(x) } as usize;
                let color = colors.palette.get(index * 3..index * 3 + 3);
                let alpha = colors.transparency.get(index).copied().unwrap_or(255);
                match color {
                    Some(color) => pixel.copy_from_slice(&[color[0], color[1], color[2], alpha]),
                    None => pixel.copy_from_slice(&[0, 0, 0, 255]),
                }
            }
        }
        4 => {
            let step = if depth == 16 { 4 } else { 2 };
            for (from, pixel) in raw
                .chunks_exact(step)
                .zip(out.chunks_exact_mut(4))
                .take(width)
            {
                let alpha = if depth == 16 { from[2] } else { from[1] };
                pixel.copy_from_slice(&[from[0], from[0], from[0], alpha]);
            }
        }
        _ => {
            if depth == 16 {
                for (from, pixel) in raw.chunks_exact(8).zip(out.chunks_exact_mut(4)).take(width) {
                    pixel.copy_from_slice(&[from[0], from[2], from[4], from[6]]);
                }
            } else {
                out[..width * 4].copy_from_slice(&raw[..width * 4]);
            }
        }
    }
}

/// Undoes a row's filter, given the row before it. `row` has the filter byte first.
fn unfilter(row: &mut [u8], previous: &[u8], step: usize) -> Result<(), ImageError> {
    let (kind, row) = row
        .split_first_mut()
        .ok_or(ImageError::Invalid("a row is empty"))?;
    match *kind {
        0 => {}
        1 => {
            for index in step..row.len() {
                row[index] = row[index].wrapping_add(row[index - step]);
            }
        }
        2 => {
            for (value, above) in row.iter_mut().zip(previous) {
                *value = value.wrapping_add(*above);
            }
        }
        3 => {
            for index in 0..row.len() {
                let left = if index >= step { row[index - step] } else { 0 };
                let average = ((u16::from(left) + u16::from(previous[index])) / 2) as u8;
                row[index] = row[index].wrapping_add(average);
            }
        }
        4 => {
            for index in 0..row.len() {
                let (left, corner) = if index >= step {
                    (row[index - step], previous[index - step])
                } else {
                    (0, 0)
                };
                row[index] = row[index].wrapping_add(paeth(left, previous[index], corner));
            }
        }
        _ => return Err(ImageError::Invalid("a row has an unknown filter")),
    }
    Ok(())
}

#[inline]
fn paeth(left: u8, above: u8, corner: u8) -> u8 {
    let (a, b, c) = (i32::from(left), i32::from(above), i32::from(corner));
    let estimate = a + b - c;
    let (to_a, to_b, to_c) = (
        (estimate - a).abs(),
        (estimate - b).abs(),
        (estimate - c).abs(),
    );
    if to_a <= to_b && to_a <= to_c {
        left
    } else if to_b <= to_c {
        above
    } else {
        corner
    }
}

/// The start and spacing of each pass of an interlaced image, across and down.
const PASSES: [(u32, u32, u32, u32); 7] = [
    (0, 0, 8, 8),
    (4, 0, 8, 8),
    (0, 4, 4, 8),
    (2, 0, 4, 4),
    (0, 2, 2, 4),
    (1, 0, 2, 2),
    (0, 1, 1, 2),
];

pub(crate) fn decode(bytes: &[u8], target: (u32, u32)) -> Result<Picture, ImageError> {
    let header = read_header(bytes)?;
    let mut colors = Colors::default();
    let mut data: Vec<&[u8]> = Vec::new();
    let mut at = SIGNATURE.len();
    while let Some(length) = read_u32(bytes, at) {
        let length = length as usize;
        let Some(tag) = bytes.get(at + 4..at + 8) else {
            break;
        };
        // A damaged length still leaves whatever's there to try reading.
        let body_end = (at + 8).saturating_add(length).min(bytes.len());
        let body = &bytes[(at + 8).min(body_end)..body_end];
        match tag {
            b"IDAT" => data.push(body),
            b"IEND" => break,
            tag => Colors::read(&mut colors, tag, body),
        }
        at = (at + 12).saturating_add(length);
    }
    if data.is_empty() {
        return Err(ImageError::Invalid("it has no pixels"));
    }
    if header.color == 3 && colors.palette.is_empty() {
        return Err(ImageError::Invalid("it has no palette"));
    }
    let size = fitted((header.width, header.height), target);
    let mut scaler = Scaler::new((header.width, header.height), size);
    scaler.set_transform(colors.transform(header));
    if header.interlaced {
        decode_interlaced(header, &colors, &data, &mut |row| scaler.push(row))?;
    } else {
        decode_rows(header, &colors, &data, &mut |row| scaler.push(row))?;
    }
    let (width, height) = scaler.size();
    Ok(Picture {
        width,
        height,
        pixels: scaler.finish(),
    })
}

/// Goes row by row, handing each on, with only two rows in memory.
fn decode_rows(
    header: Header,
    colors: &Colors<'_>,
    data: &[&[u8]],
    out: &mut dyn FnMut(&[u8]),
) -> Result<(), ImageError> {
    let row_bytes = header.row_bytes(header.width);
    let step = (header.channels() * header.depth as usize).div_ceil(8);
    let mut previous = vec![0u8; row_bytes + 1];
    let mut row = vec![0u8; row_bytes + 1];
    let mut filled = 0usize;
    let mut rgba = vec![0u8; header.width as usize * 4];
    let mut rows_done = 0u32;
    let result = inflate(data, |mut chunk| {
        while !chunk.is_empty() {
            if rows_done >= header.height {
                return Ok(true);
            }
            let take = (row.len() - filled).min(chunk.len());
            row[filled..filled + take].copy_from_slice(&chunk[..take]);
            filled += take;
            chunk = &chunk[take..];
            if filled == row.len() {
                filled = 0;
                unfilter(&mut row, &previous[1..], step)?;
                to_rgba(header, colors, &row[1..], header.width as usize, &mut rgba);
                out(&rgba);
                std::mem::swap(&mut row, &mut previous);
                rows_done += 1;
            }
        }
        Ok(rows_done >= header.height)
    });
    finish(result, rows_done)
}

/// A file that's cut short still shows what got through, as there's nothing better to do with it.
fn finish(result: Result<(), ImageError>, rows_done: u32) -> Result<(), ImageError> {
    match result {
        Err(error) if rows_done == 0 => Err(error),
        _ => Ok(()),
    }
}

/// Each pass only has some of the pixels, so an interlaced image has to be put together in full first.
fn decode_interlaced(
    header: Header,
    colors: &Colors<'_>,
    data: &[&[u8]],
    out: &mut dyn FnMut(&[u8]),
) -> Result<(), ImageError> {
    let (width, height) = (header.width as usize, header.height as usize);
    let mut whole = Vec::new();
    whole
        .try_reserve_exact(width * height * 4)
        .map_err(|_| ImageError::TooBig)?;
    whole.resize(width * height * 4, 0u8);
    let step = (header.channels() * header.depth as usize).div_ceil(8);
    // Which pass and row comes next, skipping passes with nothing in them.
    let pass_size = |pass: usize| {
        let (start_x, start_y, space_x, space_y) = PASSES[pass];
        (
            (header.width + space_x - 1).saturating_sub(start_x) / space_x,
            (header.height + space_y - 1).saturating_sub(start_y) / space_y,
        )
    };
    let mut pass = 0;
    let mut row_in_pass = 0u32;
    while pass < 7 && (pass_size(pass).0 == 0 || pass_size(pass).1 == 0) {
        pass += 1;
    }
    let mut row = Vec::new();
    let mut previous = Vec::new();
    let mut rgba = vec![0u8; width * 4];
    let mut filled = 0usize;
    let mut rows_done = 0u32;
    if pass < 7 {
        let bytes = header.row_bytes(pass_size(pass).0) + 1;
        row.resize(bytes, 0);
        previous.resize(bytes, 0);
    }
    let result = inflate(data, |mut chunk| {
        while !chunk.is_empty() && pass < 7 {
            let take = (row.len() - filled).min(chunk.len());
            row[filled..filled + take].copy_from_slice(&chunk[..take]);
            filled += take;
            chunk = &chunk[take..];
            if filled < row.len() {
                continue;
            }
            filled = 0;
            unfilter(&mut row, &previous[1..], step)?;
            let (start_x, start_y, space_x, space_y) = PASSES[pass];
            let (pass_width, pass_height) = pass_size(pass);
            to_rgba(header, colors, &row[1..], pass_width as usize, &mut rgba);
            let y = (start_y + row_in_pass * space_y) as usize;
            for x in 0..pass_width as usize {
                let to = (y * width + start_x as usize + x * space_x as usize) * 4;
                whole[to..to + 4].copy_from_slice(&rgba[x * 4..x * 4 + 4]);
            }
            std::mem::swap(&mut row, &mut previous);
            rows_done += 1;
            row_in_pass += 1;
            if row_in_pass >= pass_height {
                row_in_pass = 0;
                pass += 1;
                while pass < 7 && (pass_size(pass).0 == 0 || pass_size(pass).1 == 0) {
                    pass += 1;
                }
                if pass < 7 {
                    let bytes = header.row_bytes(pass_size(pass).0) + 1;
                    row.clear();
                    row.resize(bytes, 0);
                    previous.clear();
                    previous.resize(bytes, 0);
                }
            }
        }
        Ok(pass >= 7)
    });
    finish(result, rows_done)?;
    for row in whole.chunks_exact(width * 4) {
        out(row);
    }
    Ok(())
}

/// Whether this says it's animated, which it has to before its pixels.
pub(crate) fn is_animated(bytes: &[u8]) -> bool {
    let mut at = SIGNATURE.len();
    while let (Some(length), Some(tag)) = (read_u32(bytes, at), bytes.get(at + 4..at + 8)) {
        match tag {
            b"acTL" => return true,
            b"IDAT" | b"IEND" => return false,
            _ => {}
        }
        at = (at + 12).saturating_add(length as usize);
    }
    false
}

/// One frame of an animated file, as it's described.
struct FrameInfo<'a> {
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    delay: u32,
    dispose: u8,
    blend_over: bool,
    data: Vec<&'a [u8]>,
}

fn read_frame_info(body: &[u8], header: Header) -> Result<FrameInfo<'_>, ImageError> {
    let invalid = ImageError::Invalid("a frame's description is wrong");
    let number = |at: usize| read_u32(body, at).ok_or(invalid);
    let (width, height, x, y) = (number(4)?, number(8)?, number(12)?, number(16)?);
    let short = |at: usize| -> Result<u32, ImageError> {
        let part = body.get(at..at + 2).ok_or(invalid)?;
        Ok(u32::from(u16::from_be_bytes([part[0], part[1]])))
    };
    let (top, bottom) = (short(20)?, short(22)?);
    let dispose = *body.get(24).ok_or(invalid)?;
    let blend = *body.get(25).ok_or(invalid)?;
    if width == 0
        || height == 0
        || u64::from(x) + u64::from(width) > u64::from(header.width)
        || u64::from(y) + u64::from(height) > u64::from(header.height)
    {
        return Err(invalid);
    }
    Ok(FrameInfo {
        width,
        height,
        x,
        y,
        // A bottom of zero means hundredths of a second.
        delay: top * 1000 / if bottom == 0 { 100 } else { bottom },
        dispose,
        blend_over: blend == 1,
        data: Vec::new(),
    })
}

/// Every frame of an animated file, as a whole picture at that time.
pub(crate) fn decode_frames(bytes: &[u8], target: (u32, u32)) -> Result<Animation, ImageError> {
    let header = read_header(bytes)?;
    let mut colors = Colors::default();
    let mut frames: Vec<FrameInfo<'_>> = Vec::new();
    let mut repeats = 0;
    let mut at = SIGNATURE.len();
    while let Some(length) = read_u32(bytes, at) {
        let length = length as usize;
        let Some(tag) = bytes.get(at + 4..at + 8) else {
            break;
        };
        let body_end = (at + 8).saturating_add(length).min(bytes.len());
        let body = &bytes[(at + 8).min(body_end)..body_end];
        match tag {
            b"acTL" => repeats = read_u32(body, 4).unwrap_or(0),
            b"fcTL" => {
                if frames.len() >= MAX_FRAMES {
                    break;
                }
                frames.push(read_frame_info(body, header)?);
            }
            // The usual pixels are the first frame only if a frame's been described before them.
            b"IDAT" => {
                if let Some(frame) = frames.last_mut() {
                    frame.data.push(body);
                }
            }
            b"fdAT" => {
                if let (Some(frame), Some(data)) = (frames.last_mut(), body.get(4..)) {
                    frame.data.push(data);
                }
            }
            b"IEND" => break,
            tag => Colors::read(&mut colors, tag, body),
        }
        at = (at + 12).saturating_add(length);
    }
    if frames.is_empty() {
        return Err(ImageError::Invalid("it has no frames"));
    }
    if header.color == 3 && colors.palette.is_empty() {
        return Err(ImageError::Invalid("it has no palette"));
    }
    let source = (header.width, header.height);
    let mut canvas = Canvas::new(source)?;
    let mut stored = Stored::new(source, fitted(source, target), colors.transform(header));
    for (index, frame) in frames.iter().enumerate() {
        let (width, height) = (frame.width as usize, frame.height as usize);
        let place = (frame.x as usize, frame.y as usize);
        let this = Header {
            width: frame.width,
            height: frame.height,
            ..header
        };
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(width * height * 4)
            .map_err(|_| ImageError::TooBig)?;
        let mut read_row = |row: &[u8]| pixels.extend_from_slice(row);
        if !frame.data.is_empty() {
            if header.interlaced {
                decode_interlaced(this, &colors, &frame.data, &mut read_row)?;
            } else {
                decode_rows(this, &colors, &frame.data, &mut read_row)?;
            }
        }
        // What got cut short is left empty.
        pixels.resize(width * height * 4, 0);
        // Going back to what was there before only makes sense if something was.
        let dispose = if frame.dispose == 2 && index == 0 {
            1
        } else {
            frame.dispose
        };
        let before = (dispose == 2).then(|| canvas.copy_of(place, (width, height)));
        canvas.draw(&pixels, place, (width, height), frame.blend_over)?;
        if !stored.keep(&canvas, frame.delay) {
            break;
        }
        match (dispose, before) {
            (1, _) => canvas.clear(place, (width, height)),
            (2, Some(before)) => canvas.draw(&before, place, (width, height), false)?,
            _ => {}
        }
    }
    Ok(stored.finish(repeats))
}
