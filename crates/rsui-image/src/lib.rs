//! Reads PNG, JPEG and WebP files, written from scratch, with no dependencies.
//!
//! An image is read at the size it'll be shown, not the size it is, and as much as possible without
//! ever holding the whole of it in memory:
//!
//! - A PNG goes row by row, each shrunk as it arrives, so a huge one needs two rows of memory.
//! - A JPEG holds eight times less of each side for each halving it can skip, as it stores its
//!   pictures in blocks that are cheap to read at a smaller size.
//! - A WebP is read whole and then shrunk, as its blocks depend on each other.
//!
//! A file comes from outside the program, so everything in it is checked, and nothing it says can
//! make this panic, or use more than a fixed amount of time and memory for each pixel.
//!
//! ```
//! # fn example(bytes: &[u8]) -> Result<(), rsui_image::ImageError> {
//! let info = rsui_image::info(bytes)?;
//! let picture = rsui_image::decode(bytes, (info.width / 2, info.height / 2))?;
//! assert_eq!(picture.pixels.len(), (picture.width * picture.height * 4) as usize);
//! # Ok(())
//! # }
//! ```
//!
//! Whatever color space a file says it's in is turned into the one screens show, and an animated PNG
//! or WebP can be read as all of its frames with [`decode_animation`].
//!
//! # What's missing
//!
//! Lossless and hierarchical JPEGs, which nothing uses for pictures, aren't read.

mod animate;
mod color;
pub mod dib;
mod inflate;
mod jpeg;
mod png;
mod scale;
mod vp8;
mod vp8_tables;
mod vp8l;
mod webp;

use std::fmt;

pub use animate::{Animation, AnimationFrame};

/// The most pixels an image can have. Something bigger is almost certainly a mistake or an attack,
/// as it would take gigabytes to show at full size.
pub(crate) const MAX_PIXELS: u64 = 1 << 28;

/// Why an image couldn't be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageError {
    /// It's not a PNG, a JPEG or a WebP.
    UnknownFormat,
    /// It breaks the rules of its format, saying what's wrong.
    Invalid(&'static str),
    /// It's a kind that isn't handled, saying which.
    Unsupported(&'static str),
    /// It's bigger than [`decode`] will take on.
    TooBig,
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFormat => f.write_str("it's not a PNG, JPEG or WebP"),
            Self::Invalid(what) => write!(f, "it's damaged: {what}"),
            Self::Unsupported(what) => write!(f, "this isn't supported: {what}"),
            Self::TooBig => f.write_str("it's too big"),
        }
    }
}

impl std::error::Error for ImageError {}

/// What a file says about the image in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    /// Its size as it should be shown, once anything a camera said about turning it is applied.
    pub width: u32,
    pub height: u32,
}

/// An image that's been read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// Red, green, blue and alpha for each pixel, in rows from the top. The colors are multiplied by
    /// the alpha already, which is what blending wants.
    pub pixels: Vec<u8>,
}

/// Unpacks what's in zlib's format, as it comes in a `.woff` font, up to `most` bytes.
pub fn inflate_zlib(bytes: &[u8], most: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    inflate::inflate(&[bytes], |piece| {
        out.extend_from_slice(piece);
        Ok(out.len() > most)
    })
    .ok()?;
    (out.len() <= most).then_some(out)
}

/// Reads an image's size, without reading the image. This only needs the start of a file, usually,
/// though a JPEG can put its size after anything it likes.
pub fn info(bytes: &[u8]) -> Result<ImageInfo, ImageError> {
    if png::is_png(bytes) {
        png::info(bytes)
    } else if jpeg::is_jpeg(bytes) {
        jpeg::info(bytes)
    } else if webp::is_webp(bytes) {
        webp::info(bytes)
    } else {
        Err(ImageError::UnknownFormat)
    }
}

/// Reads an image, shrunk to fit within `target` if it's bigger, and keeping its shape. It never comes
/// out bigger than it is. A size of 0 counts as 1.
pub fn decode(bytes: &[u8], target: (u32, u32)) -> Result<Picture, ImageError> {
    let target = (target.0.max(1), target.1.max(1));
    if png::is_png(bytes) {
        png::decode(bytes, target)
    } else if jpeg::is_jpeg(bytes) {
        jpeg::decode(bytes, target)
    } else if webp::is_webp(bytes) {
        webp::decode(bytes, target)
    } else {
        Err(ImageError::UnknownFormat)
    }
}

/// Whether a file is an animation. This only reads through the start of it.
pub fn is_animated(bytes: &[u8]) -> bool {
    if png::is_png(bytes) {
        png::is_animated(bytes)
    } else if webp::is_webp(bytes) {
        webp::is_animated(bytes)
    } else {
        false
    }
}

/// Reads every frame of an animated file, as whole pictures shrunk to fit within `target`. Something
/// that isn't animated comes out as a single frame. A very long animation stops where it would use too
/// much memory, and starts again from there.
pub fn decode_animation(bytes: &[u8], target: (u32, u32)) -> Result<Animation, ImageError> {
    let target = (target.0.max(1), target.1.max(1));
    if png::is_png(bytes) && png::is_animated(bytes) {
        png::decode_frames(bytes, target)
    } else if webp::is_webp(bytes) && webp::is_animated(bytes) {
        webp::decode_frames(bytes, target, usize::MAX)
    } else {
        let picture = decode(bytes, target)?;
        Ok(Animation {
            width: picture.width,
            height: picture.height,
            frames: vec![AnimationFrame {
                pixels: picture.pixels,
                delay: 0,
            }],
            repeats: 1,
        })
    }
}

/// The size to read an image of this size at, to fit within `target` and keep its shape.
pub fn fitted((width, height): (u32, u32), target: (u32, u32)) -> (u32, u32) {
    let (target_width, target_height) = (target.0.max(1), target.1.max(1));
    if width <= target_width && height <= target_height {
        return (width.max(1), height.max(1));
    }
    let across = u64::from(target_width) * u64::from(height);
    let down = u64::from(target_height) * u64::from(width);
    if across <= down {
        // The width is what limits it.
        let high = (u64::from(height) * u64::from(target_width) + u64::from(width) / 2)
            / u64::from(width.max(1));
        (target_width, (high as u32).max(1))
    } else {
        let wide = (u64::from(width) * u64::from(target_height) + u64::from(height) / 2)
            / u64::from(height.max(1));
        ((wide as u32).max(1), target_height)
    }
}
