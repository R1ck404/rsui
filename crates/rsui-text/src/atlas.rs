//! Arranges glyphs in a texture for the GPU.

use std::collections::HashMap;

use foldhash::fast::RandomState;

use crate::font::Font;
use crate::raster::Rasterizer;

/// The atlas starts this size, in pixels per side. Most UIs use few glyphs at few sizes.
const FIRST_SIZE: u32 = 256;
/// Beyond this the atlas starts over, rather than use more memory.
const MAX_SIZE: u32 = 2048;
/// Empty pixels around each glyph, so sampling a glyph never picks up its neighbour.
const PADDING: u32 = 1;
/// Shelf heights are rounded up to this, so similar glyphs can share a shelf.
const SHELF_ROUNDING: u32 = 4;
/// How many positions between two pixels a glyph can be drawn at.
const SUBPIXEL_STEPS: u32 = 4;

/// What identifies a glyph's pixels. Pass the same key to get the same placement back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub font: u16,
    pub glyph: u16,
    /// The size in 64ths of a pixel.
    size: u32,
    subpixel: u8,
}

/// The whole number at or below this. This runs for every glyph drawn, and `floor` is a call into a
/// library unless the program's built for newer processors than it is.
#[inline(always)]
pub fn whole_part(value: f32) -> f32 {
    let truncated = value as i32 as f32;
    if truncated > value {
        truncated - 1.0
    } else {
        truncated
    }
}

impl GlyphKey {
    /// A key for drawing a glyph at `size` pixels per em, with its pen at `x`. The whole
    /// pixels of `x` don't matter, only how far it is between two of them, so text only
    /// needs a few versions of each glyph.
    #[inline]
    pub fn new(font: u16, glyph: u16, size: f32, x: f32) -> Self {
        let fraction = x - whole_part(x);
        Self {
            font,
            glyph,
            size: (size * 64.0 + 0.5) as u32,
            subpixel: ((fraction * SUBPIXEL_STEPS as f32) as u32).min(SUBPIXEL_STEPS - 1) as u8,
        }
    }

    fn size(&self) -> f32 {
        self.size as f32 / 64.0
    }

    fn shift(&self) -> f32 {
        f32::from(self.subpixel) / SUBPIXEL_STEPS as f32
    }
}

/// Where a glyph's pixels are in the atlas, and how to place them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AtlasGlyph {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    /// Where the left edge goes, in whole pixels from the pen.
    pub left: i16,
    /// Where the top edge goes, in whole pixels from the baseline.
    pub top: i16,
    /// Whether it's three bytes across for each pixel, one for each part of it, as ClearType's are. `width` is in bytes.
    pub subpixel: bool,
}

/// New pixels for the GPU's copy of the atlas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub coverage: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
struct Shelf {
    y: u32,
    height: u32,
    /// Where the next glyph on this shelf goes.
    x: u32,
}

/// Decides where glyphs go in a texture, and says what to upload.
///
/// This keeps no copy of the texture's pixels, since the GPU has one, and RAM is what
/// this project is trying to save. Growing keeps every glyph where it is, so the GPU can
/// copy the old texture into the corner of a new one.
pub struct GlyphAtlas {
    size: u32,
    shelves: Vec<Shelf>,
    next_shelf_y: u32,
    glyphs: HashMap<GlyphKey, AtlasGlyph, RandomState>,
    uploads: Vec<Upload>,
    /// Goes up each time the atlas starts over.
    generation: u32,
}

impl Default for GlyphAtlas {
    fn default() -> Self {
        Self::new()
    }
}

impl GlyphAtlas {
    pub fn new() -> Self {
        Self {
            size: FIRST_SIZE,
            shelves: Vec::new(),
            next_shelf_y: 0,
            glyphs: HashMap::default(),
            uploads: Vec::new(),
            generation: 0,
        }
    }

    /// The width and height the texture needs to be.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Goes up each time the atlas runs out of room and starts over. Every glyph from
    /// before has to be requested again, so a frame in progress must start again.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// How many glyphs are stored.
    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// The new pixels since this was last called, to copy to the texture.
    pub fn take_uploads(&mut self) -> Vec<Upload> {
        std::mem::take(&mut self.uploads)
    }

    /// Finds a glyph's place, drawing it if this is the first time it's been asked for.
    /// A glyph too large for even the largest atlas has `width` and `height` of zero.
    pub fn glyph(&mut self, key: GlyphKey, font: &Font, rasterizer: &mut Rasterizer) -> AtlasGlyph {
        if let Some(placed) = self.glyphs.get(&key) {
            return *placed;
        }
        let bitmap = rasterizer.rasterize(font, key.glyph, key.size(), key.shift());
        let mut placed = AtlasGlyph {
            left: bitmap.left.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
            top: bitmap.top.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
            ..AtlasGlyph::default()
        };
        if bitmap.width > 0 {
            let across = if bitmap.subpixel {
                bitmap.width * 3
            } else {
                bitmap.width
            };
            if let Some((x, y)) = self.allocate(across, bitmap.height) {
                placed.x = x as u16;
                placed.y = y as u16;
                placed.width = across as u16;
                placed.height = bitmap.height as u16;
                placed.subpixel = bitmap.subpixel;
                self.uploads.push(Upload {
                    x,
                    y,
                    width: across,
                    height: bitmap.height,
                    coverage: bitmap.coverage,
                });
            }
        }
        self.glyphs.insert(key, placed);
        placed
    }

    /// Finds room for a glyph, growing or starting over if there is none.
    fn allocate(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        if width + PADDING > MAX_SIZE || height + PADDING > MAX_SIZE {
            return None;
        }
        loop {
            if let Some(found) = self.find_room(width, height) {
                return Some(found);
            }
            if self.size < MAX_SIZE {
                self.size *= 2;
            } else {
                self.start_over();
            }
        }
    }

    fn find_room(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        let needed_width = width + PADDING;
        let shelf_height = (height + PADDING).next_multiple_of(SHELF_ROUNDING);
        let size = self.size;
        // The shortest shelf that fits wastes the least.
        let best = self
            .shelves
            .iter_mut()
            .filter(|shelf| shelf.height >= shelf_height && shelf.x + needed_width <= size)
            .min_by_key(|shelf| shelf.height);
        if let Some(shelf) = best {
            let x = shelf.x;
            shelf.x += needed_width;
            return Some((x, shelf.y));
        }
        if self.next_shelf_y + shelf_height > size || needed_width > size {
            return None;
        }
        let y = self.next_shelf_y;
        self.shelves.push(Shelf {
            y,
            height: shelf_height,
            x: needed_width,
        });
        self.next_shelf_y += shelf_height;
        Some((0, y))
    }

    /// Forgets everything, and goes back to the smallest size.
    pub fn start_over(&mut self) {
        self.size = FIRST_SIZE;
        self.shelves.clear();
        self.next_shelf_y = 0;
        self.glyphs.clear();
        self.uploads.clear();
        self.generation += 1;
    }
}
