//! Turns glyph outlines into pixels.
//!
//! Each line of an outline adds how much it covers of each pixel it crosses to a buffer,
//! with a sign for its direction. Adding up each row from left to right then gives the
//! exact area of each pixel inside the shape. This is much faster than sampling, and more
//! accurate, as no pixel is ever approximated.

use crate::font::{Font, OutlineSink};

/// The largest glyph to draw, in pixels, so a damaged font can't ask for a huge bitmap.
const MAX_GLYPH_SIZE: usize = 1024;
/// How far a curve may stray from the lines that approximate it, in pixels. The lines sit
/// inside a curve, so a larger number makes round shapes visibly thinner.
const CURVE_TOLERANCE: f32 = 0.05;

/// A glyph's pixels, one byte of coverage for each.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GlyphBitmap {
    /// Where the bitmap's left edge goes, from the pen position, in pixels.
    pub left: i32,
    /// Where the bitmap's top edge goes, from the baseline, in pixels. Down is positive.
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// Rows from top to bottom, with 255 for fully inside the glyph.
    pub coverage: Vec<u8>,
    /// Whether each pixel has three of those, for its red, green and blue parts from left to right, as ClearType draws text, so
    /// that each part of a pixel is as covered as it is.
    pub subpixel: bool,
}

/// Something that draws glyphs the way the system does, which [`Rasterizer`] asks first, when it's been given one. What it can't
/// draw, `None`, is drawn as usual.
pub trait SystemGlyphs {
    /// A glyph at this size in pixels, `shift_x` of a pixel to the right, as [`Rasterizer::rasterize`] has it.
    fn rasterize(
        &mut self,
        font: &Font,
        glyph: u16,
        size: f32,
        shift_x: f32,
    ) -> Option<GlyphBitmap>;
}

/// Draws glyphs, keeping its buffers between them.
#[derive(Default)]
pub struct Rasterizer {
    lines: Vec<[f32; 4]>,
    areas: Vec<f32>,
    system: Option<Box<dyn SystemGlyphs>>,
}

struct Flatten<'a> {
    lines: &'a mut Vec<[f32; 4]>,
    /// From the font's units to pixels: `x' = a x + c y + e`, `y' = b x + d y + f`.
    matrix: [f32; 6],
    start: (f32, f32),
    pen: (f32, f32),
}

impl Flatten<'_> {
    /// Font units to pixels, with y going down.
    #[inline]
    fn pixel(&self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.matrix;
        (a * x + c * y + e, b * x + d * y + f)
    }

    fn line(&mut self, to: (f32, f32)) {
        self.lines.push([self.pen.0, self.pen.1, to.0, to.1]);
        self.pen = to;
    }
}

impl OutlineSink for Flatten<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pen = self.pixel(x, y);
        self.start = self.pen;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let to = self.pixel(x, y);
        self.line(to);
    }

    fn quad_to(&mut self, control_x: f32, control_y: f32, x: f32, y: f32) {
        let control = self.pixel(control_x, control_y);
        let to = self.pixel(x, y);
        let from = self.pen;
        // How far the curve bends sets how many lines it needs.
        let bend_x = from.0 - 2.0 * control.0 + to.0;
        let bend_y = from.1 - 2.0 * control.1 + to.1;
        let bend = (bend_x * bend_x + bend_y * bend_y).sqrt();
        let steps = ((bend / (8.0 * CURVE_TOLERANCE)).sqrt().ceil() as u32).clamp(1, 64);
        for step in 1..steps {
            let t = step as f32 / steps as f32;
            let u = 1.0 - t;
            let point = (
                u * u * from.0 + 2.0 * u * t * control.0 + t * t * to.0,
                u * u * from.1 + 2.0 * u * t * control.1 + t * t * to.1,
            );
            self.line(point);
        }
        self.line(to);
    }

    fn close(&mut self) {
        if self.pen != self.start {
            self.line(self.start);
        }
    }
}

impl Rasterizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Draws a glyph at `size` pixels per em, moved right by `shift_x` of a pixel so text
    /// can sit between pixels. A glyph with no outline, or one too large, has an empty
    /// bitmap.
    pub fn rasterize(&mut self, font: &Font, glyph: u16, size: f32, shift_x: f32) -> GlyphBitmap {
        if let Some(system) = &mut self.system
            && let Some(drawn) = system.rasterize(font, glyph, size, shift_x)
        {
            return drawn;
        }
        let scale = size / f32::from(font.metrics().units_per_em);
        if !(scale.is_finite() && scale > 0.0) {
            return GlyphBitmap::default();
        }
        self.rasterize_with(font, glyph, [scale, 0.0, 0.0, -scale, shift_x, 0.0])
    }

    /// Has glyphs drawn by `system` from now on, as far as it can, or by this again, with `None`.
    pub fn set_system(&mut self, system: Option<Box<dyn SystemGlyphs>>) {
        self.system = system;
    }

    /// Whether glyphs are drawn by something other than this.
    pub fn has_system(&self) -> bool {
        self.system.is_some()
    }

    /// Draws a glyph through any transformation from its own units to pixels.
    pub(crate) fn rasterize_with(
        &mut self,
        font: &Font,
        glyph: u16,
        matrix: [f32; 6],
    ) -> GlyphBitmap {
        self.lines.clear();
        let mut flatten = Flatten {
            lines: &mut self.lines,
            matrix,
            start: (0.0, 0.0),
            pen: (0.0, 0.0),
        };
        font.outline(glyph, &mut flatten);
        self.draw_lines()
    }

    /// Fills what these lines go around, each from one point to another in pixels, as far as `largest` on a side.
    /// Lines going around the other way take away from what's inside them.
    pub fn fill(&mut self, lines: &mut Vec<[f32; 4]>, largest: usize) -> GlyphBitmap {
        std::mem::swap(&mut self.lines, lines);
        let bitmap = self.draw_lines_up_to(largest);
        std::mem::swap(&mut self.lines, lines);
        // A big shape shouldn't leave its memory behind for what are mostly little ones.
        if self.areas.capacity() > 256 * 1024 {
            self.areas = Vec::new();
        }
        bitmap
    }

    fn draw_lines(&mut self) -> GlyphBitmap {
        self.draw_lines_up_to(MAX_GLYPH_SIZE)
    }

    fn draw_lines_up_to(&mut self, largest: usize) -> GlyphBitmap {
        let Some(first) = self.lines.first() else {
            return GlyphBitmap::default();
        };
        let (mut min_x, mut min_y) = (first[0], first[1]);
        let (mut max_x, mut max_y) = (first[0], first[1]);
        for line in &self.lines {
            min_x = min_x.min(line[0]).min(line[2]);
            max_x = max_x.max(line[0]).max(line[2]);
            min_y = min_y.min(line[1]).min(line[3]);
            max_y = max_y.max(line[1]).max(line[3]);
        }
        let left = min_x.floor();
        let top = min_y.floor();
        let width = (max_x.ceil() - left) as usize;
        let height = (max_y.ceil() - top) as usize;
        if width == 0 || height == 0 || width > largest || height > largest {
            return GlyphBitmap::default();
        }

        // One extra column, for the part of a line's area that lands past its pixel.
        let stride = width + 1;
        self.areas.clear();
        self.areas.resize(stride * height, 0.0);
        for line in &self.lines {
            add_line(
                &mut self.areas,
                stride,
                height,
                line[0] - left,
                line[1] - top,
                line[2] - left,
                line[3] - top,
            );
        }

        let mut coverage = Vec::with_capacity(width * height);
        for row in self.areas.chunks_exact(stride) {
            let mut total = 0.0;
            for area in &row[..width] {
                total += area;
                coverage.push((total.abs().min(1.0) * 255.0 + 0.5) as u8);
            }
        }
        GlyphBitmap {
            left: left as i32,
            top: top as i32,
            width: width as u32,
            height: height as u32,
            coverage,
            subpixel: false,
        }
    }
}

/// Adds one line's effect on every pixel it passes through.
fn add_line(areas: &mut [f32], stride: usize, height: usize, x0: f32, y0: f32, x1: f32, y1: f32) {
    if y0 == y1 {
        return;
    }
    // Always work downwards, and note which way the line really went.
    let (direction, x0, y0, x1, y1) = if y0 < y1 {
        (1.0, x0, y0, x1, y1)
    } else {
        (-1.0, x1, y1, x0, y0)
    };
    let slope = (x1 - x0) / (y1 - y0);
    let first_row = y0 as usize;
    let last_row = (y1.ceil() as usize).min(height);
    let mut x = x0;
    for row in first_row..last_row {
        let row_top = row as f32;
        let dy = (row_top + 1.0).min(y1) - row_top.max(y0);
        let x_next = x + slope * dy;
        let signed = direction * dy;
        let (left, right) = if x < x_next { (x, x_next) } else { (x_next, x) };
        let left_floor = left.floor();
        let left_pixel = left_floor as usize;
        let right_ceil = right.ceil();
        let right_pixel = right_ceil as usize;

        let start = row * stride;
        let cells = &mut areas[start..start + stride];
        let add = |cells: &mut [f32], index: usize, value: f32| {
            // Rounding can put a point a hair past the last column.
            if let Some(cell) = cells.get_mut(index) {
                *cell += value;
            }
        };
        if right_pixel <= left_pixel + 1 {
            // Within one pixel: part of the area lands here, and the rest on the next.
            let middle = 0.5 * (x + x_next) - left_floor;
            add(cells, left_pixel, signed - signed * middle);
            add(cells, left_pixel + 1, signed * middle);
        } else {
            let inverse = 1.0 / (right - left);
            let left_fraction = left - left_floor;
            let first_area = 0.5 * inverse * (1.0 - left_fraction) * (1.0 - left_fraction);
            let right_fraction = right - right_ceil + 1.0;
            let last_area = 0.5 * inverse * right_fraction * right_fraction;
            add(cells, left_pixel, signed * first_area);
            if right_pixel == left_pixel + 2 {
                add(
                    cells,
                    left_pixel + 1,
                    signed * (1.0 - first_area - last_area),
                );
            } else {
                let second_area = inverse * (1.5 - left_fraction);
                add(cells, left_pixel + 1, signed * (second_area - first_area));
                for pixel in left_pixel + 2..right_pixel - 1 {
                    add(cells, pixel, signed * inverse);
                }
                let so_far = second_area + (right_pixel - left_pixel - 3) as f32 * inverse;
                add(cells, right_pixel - 1, signed * (1.0 - so_far - last_area));
            }
            add(cells, right_pixel, signed * last_area);
        }
        x = x_next;
    }
}
