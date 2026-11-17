//! Puts the frames of an animated image together, and keeps them at the size they're shown.

use std::sync::Arc;

use crate::ImageError;
use crate::color::ColorTransform;
use crate::scale::Scaler;

/// The most frames kept. Real ones have far fewer, and something with more is better off as a video.
pub(crate) const MAX_FRAMES: usize = 2000;
/// The most memory all of an animation's frames can use, at the size they're shown. An animation that
/// goes over this plays what fits, and starts again from there.
const MAX_BYTES: usize = 128 * 1024 * 1024;
/// A frame with no time of its own is shown for this long, which is what browsers do too.
const SHORTEST_DELAY: u32 = 20;
/// How many pixels can be shrunk for all of an animation's frames put together. This is what stops a
/// file with thousands of huge frames from taking minutes.
const MOST_PIXEL_WORK: u64 = 1 << 31;
const DEFAULT_DELAY: u32 = 100;

/// One whole picture of an animation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationFrame {
    /// Red, green, blue and alpha for each pixel, with colors multiplied by alpha.
    pub pixels: Vec<u8>,
    /// How long it's shown for, in milliseconds.
    pub delay: u32,
}

/// All of an animated image, at the size it's going to be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Animation {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<AnimationFrame>,
    /// How many times it plays, where 0 means forever.
    pub repeats: u32,
}

/// The whole picture so far, with colors not yet multiplied by alpha.
pub(crate) struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Canvas {
    pub(crate) fn new((width, height): (u32, u32)) -> Result<Self, ImageError> {
        let (width, height) = (width as usize, height as usize);
        let total = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(ImageError::TooBig)?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(total)
            .map_err(|_| ImageError::TooBig)?;
        pixels.resize(total, 0u8);
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Draws a frame on, at this position, either over what's there or instead of it.
    pub(crate) fn draw(
        &mut self,
        frame: &[u8],
        (left, top): (usize, usize),
        (width, height): (usize, usize),
        blend: bool,
    ) -> Result<(), ImageError> {
        if left + width > self.width || top + height > self.height {
            return Err(ImageError::Invalid("a frame goes outside the picture"));
        }
        for (row, from) in frame.chunks_exact(width * 4).take(height).enumerate() {
            let at = ((top + row) * self.width + left) * 4;
            let to = &mut self.pixels[at..at + width * 4];
            if !blend {
                to.copy_from_slice(from);
                continue;
            }
            for (to, from) in to.chunks_exact_mut(4).zip(from.chunks_exact(4)) {
                over(to, from);
            }
        }
        Ok(())
    }

    /// Makes part of it empty again.
    pub(crate) fn clear(&mut self, (left, top): (usize, usize), (width, height): (usize, usize)) {
        for row in top..(top + height).min(self.height) {
            let start = (row * self.width + left) * 4;
            let end = (row * self.width + (left + width).min(self.width)) * 4;
            self.pixels[start..end].fill(0);
        }
    }

    /// A copy of part of it, for putting back later.
    pub(crate) fn copy_of(
        &self,
        (left, top): (usize, usize),
        (width, height): (usize, usize),
    ) -> Vec<u8> {
        let mut copy = Vec::with_capacity(width * height * 4);
        for row in top..(top + height).min(self.height) {
            let start = (row * self.width + left) * 4;
            let end = (row * self.width + (left + width).min(self.width)) * 4;
            copy.extend_from_slice(&self.pixels[start..end]);
        }
        copy
    }
}

/// Puts one pixel over another, working with colors that aren't multiplied by their alpha.
#[inline]
fn over(under: &mut [u8], top: &[u8]) {
    let (top_alpha, under_alpha) = (u32::from(top[3]), u32::from(under[3]));
    if top_alpha == 255 || under_alpha == 0 {
        under.copy_from_slice(top);
        return;
    }
    if top_alpha == 0 {
        return;
    }
    let left_over = under_alpha * (255 - top_alpha);
    let total = top_alpha * 255 + left_over;
    for part in 0..3 {
        let mixed = u32::from(top[part]) * top_alpha * 255 + u32::from(under[part]) * left_over;
        under[part] = ((mixed + total / 2) / total) as u8;
    }
    under[3] = ((total + 127) / 255) as u8;
}

/// Keeps a copy of the canvas for each frame, at the size it's going to be shown.
pub(crate) struct Stored {
    source: (u32, u32),
    size: (u32, u32),
    transform: Option<Arc<ColorTransform>>,
    frames: Vec<AnimationFrame>,
    bytes: usize,
    work: u64,
}

impl Stored {
    pub(crate) fn new(
        source: (u32, u32),
        size: (u32, u32),
        transform: Option<ColorTransform>,
    ) -> Self {
        Self {
            source,
            size,
            transform: transform.map(Arc::new),
            frames: Vec::new(),
            bytes: 0,
            work: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.frames.len()
    }

    /// Keeps what's on the canvas now, and says whether there's room to keep any more.
    pub(crate) fn keep(&mut self, canvas: &Canvas, delay: u32) -> bool {
        let needed = self.size.0 as usize * self.size.1 as usize * 4;
        let work = u64::from(self.source.0) * u64::from(self.source.1);
        if !self.frames.is_empty()
            && (self.bytes + needed > MAX_BYTES || self.work + work > MOST_PIXEL_WORK)
        {
            return false;
        }
        self.work += work;
        let mut scaler = Scaler::new(self.source, self.size);
        scaler.set_shared_transform(self.transform.clone());
        for row in canvas.pixels.chunks_exact(canvas.width * 4) {
            scaler.push(row);
        }
        self.bytes += needed;
        self.frames.push(AnimationFrame {
            pixels: scaler.finish(),
            delay: match delay {
                0 => DEFAULT_DELAY,
                delay => delay.max(SHORTEST_DELAY),
            },
        });
        true
    }

    pub(crate) fn finish(self, repeats: u32) -> Animation {
        Animation {
            width: self.size.0,
            height: self.size.1,
            frames: self.frames,
            repeats,
        }
    }
}
