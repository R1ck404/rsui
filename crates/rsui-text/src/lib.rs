//! Fonts, text measuring, line breaking and glyph rendering.
//!
//! All of this is written from scratch, and reads TrueType fonts directly:
//!
//! - [`Font`] reads a font file, whose contents aren't trusted.
//! - [`TextEngine`] measures text and breaks it into lines, and gives
//!   [`rsui_layout`] the sizes it needs through [`rsui_layout::Measure`].
//! - [`Rasterizer`] turns a glyph into pixels, and [`GlyphAtlas`] arranges those
//!   pixels in a texture for the GPU.
//!
//! ```no_run
//! use rsui_text::{Font, TextEngine, TextStyle};
//!
//! let font = Font::open("C:/Windows/Fonts/segoeui.ttf").unwrap();
//! let mut text = TextEngine::new(font);
//! let layout = text.layout("Hello, world", TextStyle::new(14.0), f32::INFINITY);
//! println!("{} by {}", layout.width(), layout.height());
//! ```
//!
//! Text is shaped by the font's own rules and put in order by the Unicode Bidirectional Algorithm,
//! so Arabic joins up, Hebrew and Arabic run from the right, and ligatures, kerning and marks all
//! land where the font says. Text that doesn't need any of this skips all of it, for the cost of a
//! glance at each character.
//!
//! # What's missing
//!
//! Scripts whose letters change order, such as Devanagari and Thai's relatives, are shaped as if
//! they didn't, so they aren't right yet. Lines only break at spaces and the like, and between
//! ideographs, so text in Thai or Khmer, which needs a dictionary for that, breaks poorly. Color
//! glyphs show as outlines. Fonts must have TrueType outlines, so most `.otf` files don't load, and
//! hinting instructions are ignored.

// The shared generated-font fixture also serves integration tests.
#[cfg(test)]
extern crate self as rsui_text;

mod atlas;
mod color;
mod complex;
mod data;
mod dictionary;
mod engine;
mod fallback;
mod files;
mod font;
mod gpos;
mod indic;
mod indic_tables;
mod lines;
mod ot;
mod raster;
mod rich;
mod shape;
mod southeast;
mod system_fonts;
#[cfg(not(windows))]
mod unix_fonts;

pub mod bidi;
mod unicode_tables;
mod universal;
mod universal_tables;
mod variations;

pub use atlas::{AtlasGlyph, GlyphAtlas, GlyphKey, Upload, whole_part};
pub use color::ColorBitmap;
pub use engine::{
    FontStyle, Line, PlacedGlyph, TextEngine, TextLayout, TextStyle, max_lines, space_letters,
    space_words, wrapped_line,
};
pub use fallback::system_ui_font_files;
pub use files::{FileReader, read_by_reader, read_file, read_files_with};
pub use font::{ColorLayer, Font, FontError, FontMetrics, OutlineSink};
pub use raster::{GlyphBitmap, Rasterizer, SystemGlyphs};
pub use rich::{
    Piece, RichLayout, RichLine, RichScratch, RichText, TextRun, gather_text, is_selectable,
    paragraph_arrangement, paragraph_text, path_of_run,
};
pub use shape::is_mark;
pub use variations::VariationAxis;
