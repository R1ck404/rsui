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
    /// What [`fill_by_rule`](Self::fill_by_rule) keeps between shapes: each line cut at the rows it crosses, where each row's
    /// pieces start, and what [`Slicing`] works with.
    pieces: Vec<Piece>,
    starts: Vec<u32>,
    slicing: Slicing,
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

    /// Fills what these lines go around as SVG and a canvas do, by the nonzero rule, or the even-odd one if `even_odd` says, as
    /// far as `largest` on a side. Unlike [`fill`](Self::fill), which adds up how far round each pixel the lines go, this works
    /// out what's inside first, so where parts of a shape lie over each other they're covered once, as much as either covers:
    /// two circles going the same way don't count twice where the edge of one is inside the other, and by the even-odd rule a
    /// part inside another is a hole whichever way it goes.
    ///
    /// Each row of pixels is cut into slices across, at every point where a line starts, ends or crosses another. Within a
    /// slice no lines cross, so going along it from the left, how many times the lines have gone round is the same all the
    /// way between one and the next, and the rule says whether that's inside. Only the lines where that changes are kept, as
    /// the two sides of each part that's inside, and they're added up as [`fill`](Self::fill) adds its lines, which gives the
    /// exact area inside each pixel.
    ///
    /// So that a row a shape crosses in many places, as a chart's line does, doesn't cost every slice of it all of them, a row's
    /// lines are taken in groups that are apart from each other across it, each sliced by itself, starting from how many times
    /// the groups left of it have gone round at each height.
    pub fn fill_by_rule(
        &mut self,
        lines: &[[f32; 4]],
        largest: usize,
        even_odd: bool,
    ) -> GlyphBitmap {
        let Some((left, top, width, height)) = bounds(lines, largest) else {
            return GlyphBitmap::default();
        };
        let stride = width + 1;
        self.areas.clear();
        self.areas.resize(stride * height, 0.0);
        self.pieces.clear();
        self.starts.clear();
        self.starts.resize(height + 1, 0);
        // Each line cut at the rows it crosses, sorted by row: how many there are in each row first, then each in its place.
        let rows_of = |line: &[f32; 4]| {
            let (y0, y1) = (line[1] - top, line[3] - top);
            let (low, high) = if y0 < y1 { (y0, y1) } else { (y1, y0) };
            let first = (low.max(0.0) as usize).min(height);
            let last = (high.ceil().max(0.0) as usize).min(height);
            first..last
        };
        for line in lines.iter() {
            if line[1] != line[3] {
                for row in rows_of(line) {
                    self.starts[row + 1] += 1;
                }
            }
        }
        for row in 0..height {
            self.starts[row + 1] += self.starts[row];
        }
        self.pieces.resize(self.starts[height] as usize, Piece::default());
        let mut next: Vec<u32> = self.starts[..height].to_vec();
        for line in lines.iter() {
            let [x0, y0, x1, y1] = *line;
            if y0 == y1 {
                continue;
            }
            let rows = rows_of(line);
            let (x0, y0, x1, y1) = (x0 - left, y0 - top, x1 - left, y1 - top);
            let (way, x0, y0, x1, y1) = if y0 < y1 {
                (1, x0, y0, x1, y1)
            } else {
                (-1, x1, y1, x0, y0)
            };
            let slope = (x1 - x0) / (y1 - y0);
            let x_at = |y: f32| if y <= y0 { x0 } else if y >= y1 { x1 } else { x0 + slope * (y - y0) };
            for row in rows {
                let from = (row as f32).max(y0);
                let to = (row as f32 + 1.0).min(y1);
                let at = &mut next[row];
                self.pieces[*at as usize] = Piece {
                    x0: x_at(from),
                    y0: from,
                    x1: x_at(to),
                    y1: to.max(from),
                    way,
                };
                *at += 1;
            }
        }
        let slicing = &mut self.slicing;
        slicing.even_odd = even_odd;
        for row in 0..height {
            let (start, end) = (self.starts[row] as usize, self.starts[row + 1] as usize);
            let pieces = &mut self.pieces[start..end];
            pieces.sort_unstable_by(|a, b| a.left().total_cmp(&b.left()));
            slicing.left_of.clear();
            slicing.left_of.push((f32::NEG_INFINITY, 0));
            let mut first = 0;
            while first < pieces.len() {
                let mut reach = pieces[first].right();
                let mut last = first + 1;
                while last < pieces.len() && pieces[last].left() <= reach {
                    reach = reach.max(pieces[last].right());
                    last += 1;
                }
                slicing.fill_group(&mut pieces[first..last], width, &mut self.areas, stride, height);
                first = last;
            }
        }
        if self.pieces.capacity() > 64 * 1024 {
            self.pieces = Vec::new();
        }
        let bitmap = self.coverage_of(left, top, width, height);
        if self.areas.capacity() > 256 * 1024 {
            self.areas = Vec::new();
        }
        bitmap
    }

    fn draw_lines(&mut self) -> GlyphBitmap {
        self.draw_lines_up_to(MAX_GLYPH_SIZE)
    }

    fn draw_lines_up_to(&mut self, largest: usize) -> GlyphBitmap {
        let Some((left, top, width, height)) = bounds(&self.lines, largest) else {
            return GlyphBitmap::default();
        };

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
        self.coverage_of(left, top, width, height)
    }

    /// Each pixel's coverage, from the areas the lines left, which are added up along each row.
    fn coverage_of(&self, left: f32, top: f32, width: usize, height: usize) -> GlyphBitmap {
        let stride = width + 1;
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

/// The whole pixels these lines reach, as their left and top and how many across and down, if that's no more than `largest`
/// each way and not nothing.
fn bounds(lines: &[[f32; 4]], largest: usize) -> Option<(f32, f32, usize, usize)> {
    let first = lines.first()?;
    let (mut min_x, mut min_y) = (first[0], first[1]);
    let (mut max_x, mut max_y) = (first[0], first[1]);
    for line in lines {
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
        return None;
    }
    Some((left, top, width, height))
}

/// The part of a line in one row of pixels, from its top to its bottom, and which way the line went: 1 down, -1 up.
#[derive(Debug, Clone, Copy, Default)]
struct Piece {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    way: i32,
}

impl Piece {
    fn left(&self) -> f32 {
        self.x0.min(self.x1)
    }

    fn right(&self) -> f32 {
        self.x0.max(self.x1)
    }

    fn x_at(&self, y: f32) -> f32 {
        if y <= self.y0 {
            self.x0
        } else if y >= self.y1 {
            self.x1
        } else {
            self.x0 + (self.x1 - self.x0) * ((y - self.y0) / (self.y1 - self.y0))
        }
    }
}

/// Where a piece is at the top and bottom of a slice of a row, which way it goes, and which piece of the row it is.
#[derive(Debug, Clone, Copy)]
struct Across {
    top: f32,
    bottom: f32,
    way: i32,
    index: u32,
}


/// What [`Rasterizer::fill_by_rule`] works with as it slices each group of a row's pieces, kept between rows and shapes.
#[derive(Default)]
struct Slicing {
    even_odd: bool,
    /// How many times the lines left of the group being sliced have gone round, which goes up and down the row: from each
    /// height on, until the next. The first is from the top of the row.
    left_of: Vec<(f32, i32)>,
    /// Where each slice starts and ends.
    stops: Vec<f32>,
    /// The pieces in the slice being looked at, left to right along its top.
    across: Vec<Across>,
    /// The same, left to right along its bottom.
    below: Vec<Across>,
    /// Where it's cut again, where pieces cross.
    cuts: Vec<f32>,
    /// How many times the group's own pieces go round, from each height on, where that changes.
    changes: Vec<(f32, i32)>,
    /// What `left_of` is once that's added.
    merged: Vec<(f32, i32)>,
}

impl Slicing {
    /// Adds what's inside of a group of pieces of one row, apart from the rest across it, to `areas`, from what `left_of`
    /// says, and then adds what the group goes round to that, for the next group.
    fn fill_group(&mut self, pieces: &mut [Piece], width: usize, areas: &mut [f32], stride: usize, height: usize) {
        let even_odd = self.even_odd;
        let inside = |winding: i32| if even_odd { winding & 1 != 0 } else { winding != 0 };
        pieces.sort_unstable_by(|a, b| a.y0.total_cmp(&b.y0));
        // Each slice starts and ends where a piece does.
        self.stops.clear();
        for piece in pieces.iter() {
            self.stops.push(piece.y0);
            self.stops.push(piece.y1);
        }
        self.stops.sort_unstable_by(f32::total_cmp);
        self.stops.dedup();
        self.across.clear();
        self.changes.clear();
        let mut own = 0;
        let mut added = 0;
        for slice in 1..self.stops.len() {
            let (a, b) = (self.stops[slice - 1], self.stops[slice]);
            // The pieces in this slice, in the order they were in along the bottom of the one before, which is the order they're
            // in along the top of this one but for those that start here.
            self.across.retain(|p| pieces[p.index as usize].y1 > a);
            while added < pieces.len() && pieces[added].y0 <= a {
                self.across.push(Across {
                    top: 0.0,
                    bottom: 0.0,
                    way: pieces[added].way,
                    index: added as u32,
                });
                added += 1;
            }
            // How many times the group's own pieces go round, across the slice, from here down.
            let goes_round: i32 = self.across.iter().map(|p| p.way).sum();
            if goes_round != own {
                own = goes_round;
                self.changes.push((a, own));
            }
            if self.across.is_empty() {
                continue;
            }
            for p in &mut self.across {
                let piece = &pieces[p.index as usize];
                p.top = piece.x_at(a);
                p.bottom = piece.x_at(b);
            }
            // Left to right along the top. They're nearly in that order already, so each is moved to its place.
            sort_nearly_sorted(&mut self.across, |p, q| {
                p.top < q.top || (p.top == q.top && p.bottom < q.bottom)
            });
            // Then along the bottom, where a piece that has to pass another to get to its place crosses it, and the slice is
            // cut again where it does. That only happens where parts of a shape lie over each other.
            const CROSSING: f32 = 1.0e-4;
            self.cuts.clear();
            self.cuts.push(a);
            self.below.clear();
            self.below.extend_from_slice(&self.across);
            for i in 1..self.below.len() {
                let moving = self.below[i];
                let mut j = i;
                while j > 0 && self.below[j - 1].bottom > moving.bottom {
                    // What's passed was left of what's moving along the top, and is right of it along the bottom.
                    let passed = self.below[j - 1];
                    let gap_top = moving.top - passed.top;
                    let gap_bottom = moving.bottom - passed.bottom;
                    if gap_bottom < -CROSSING {
                        let t = gap_top / (gap_top - gap_bottom);
                        if t > 0.0 && t < 1.0 {
                            self.cuts.push(a + (b - a) * t);
                        }
                    }
                    self.below[j] = passed;
                    j -= 1;
                }
                self.below[j] = moving;
            }
            // Where the slice is cut by how many times what's left of the group goes round changing, too.
            let from_left = self.left_of.partition_point(|step| step.0 <= a);
            for step in &self.left_of[from_left..] {
                if step.0 >= b {
                    break;
                }
                self.cuts.push(step.0);
            }
            let crosses = self.cuts.len() > 1;
            if crosses {
                self.cuts.sort_unstable_by(f32::total_cmp);
                self.cuts.dedup();
            }
            self.cuts.push(b);
            for cut in 1..self.cuts.len() {
                let (from, to) = (self.cuts[cut - 1], self.cuts[cut]);
                if to <= from {
                    continue;
                }
                let (share_from, share_to) = ((from - a) / (b - a), (to - a) / (b - a));
                let along = |p: &Across, share: f32| p.top + (p.bottom - p.top) * share;
                if crosses {
                    let middle = 0.5 * (share_from + share_to);
                    sort_nearly_sorted(&mut self.across, |p, q| along(p, middle) < along(q, middle));
                }
                let middle = 0.5 * (from + to);
                let mut winding = self.left_of[self.left_of.partition_point(|step| step.0 <= middle) - 1].1;
                for p in &self.across {
                    let was = inside(winding);
                    winding += p.way;
                    let is = inside(winding);
                    if was != is {
                        // Within the picture, as working out where a piece is can put it a hair past its edge, which would put
                        // what's in the pixel at the edge in the one beside it.
                        let x_from = along(p, share_from).clamp(0.0, width as f32);
                        let x_to = along(p, share_to).clamp(0.0, width as f32);
                        // Downwards on the left of what's inside and upwards on its right, which adds up to its area.
                        if is {
                            add_line(areas, stride, height, x_from, from, x_to, to);
                        } else {
                            add_line(areas, stride, height, x_to, to, x_from, from);
                        }
                    }
                }
            }
        }
        if own != 0 {
            self.changes.push((*self.stops.last().unwrap_or(&0.0), 0));
        }
        // What the group goes round, added to what's left of it, for the next group. Where a whole outline goes through the
        // row, it goes round once all the way down it, and where it comes back out the way it came in, not at all, so what a
        // group goes round is mostly the same all the way down, and often nothing, as for the two sides of a line's stroke.
        if self.changes.is_empty() {
            return;
        }
        self.merged.clear();
        let (mut base, mut own) = (self.left_of[0].1, 0);
        self.merged.push((f32::NEG_INFINITY, base));
        let (mut i, mut j) = (1, 0);
        while i < self.left_of.len() || j < self.changes.len() {
            let y = match (self.left_of.get(i), self.changes.get(j)) {
                (Some(step), Some(change)) => step.0.min(change.0),
                (Some(step), None) => step.0,
                (None, Some(change)) => change.0,
                (None, None) => break,
            };
            while i < self.left_of.len() && self.left_of[i].0 == y {
                base = self.left_of[i].1;
                i += 1;
            }
            while j < self.changes.len() && self.changes[j].0 == y {
                own = self.changes[j].1;
                j += 1;
            }
            if self.merged.last().is_some_and(|last| last.1 != base + own) {
                self.merged.push((y, base + own));
            }
        }
        std::mem::swap(&mut self.left_of, &mut self.merged);
    }
}

/// Sorts what's nearly in order by moving each to its place, which costs only as much as how far each has to go.
fn sort_nearly_sorted(items: &mut [Across], before: impl Fn(&Across, &Across) -> bool) {
    for i in 1..items.len() {
        let moving = items[i];
        let mut j = i;
        while j > 0 && before(&moving, &items[j - 1]) {
            items[j] = items[j - 1];
            j -= 1;
        }
        items[j] = moving;
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
        // Nothing is left of the bitmap, but rounding can put the end of a line that comes down to its left edge a hair past
        // it, and then all of what's in the pixel there would land in the next one.
        let x_next = (x + slope * dy).max(0.0);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(bitmap: &GlyphBitmap, x: i32, y: i32) -> f32 {
        let (x, y) = (x - bitmap.left, y - bitmap.top);
        f32::from(bitmap.coverage[y as usize * bitmap.width as usize + x as usize]) / 255.0
    }

    #[test]
    fn a_line_coming_down_to_the_left_edge_leaves_its_pixel_as_covered_as_it_is() {
        // A shape whose left side slopes down to the bitmap's left edge, from all sorts of places in the row above it. Rounding
        // once put the bottom of that side a hair past the edge, and then what it covered of the pixel there went in the next.
        for step in 0..400 {
            let x = 4.0 + 0.0003 * step as f32;
            let y = 8.01 + 0.0024 * step as f32;
            let lines = vec![[x, y, 4.0, 9.0], [4.0, 9.0, 6.0, 9.0], [6.0, 9.0, 6.0, y], [6.0, y, x, y]];
            // Of the pixel from 4 to 5, all below `y` and right of the side, which is on average half as far across as `x` is.
            let expected = (9.0 - y) * (1.0 - 0.5 * (x - 4.0));
            for even_odd in [false, true] {
                let bitmap = Rasterizer::new().fill_by_rule(&lines, 64, even_odd);
                assert!((at(&bitmap, 4, 8) - expected).abs() < 0.6 / 255.0, "{x} {y}: {}", at(&bitmap, 4, 8));
            }
            let bitmap = Rasterizer::new().fill(&mut lines.clone(), 64);
            assert!((at(&bitmap, 4, 8) - expected).abs() < 0.6 / 255.0, "{x} {y}: {}", at(&bitmap, 4, 8));
        }
    }

    /// A square from `x0, y0` to `x1, y1`, going clockwise or the other way.
    fn square(x0: f32, y0: f32, x1: f32, y1: f32, clockwise: bool, lines: &mut Vec<[f32; 4]>) {
        let mut corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
        if !clockwise {
            corners.reverse();
        }
        for i in 0..4 {
            let (a, b) = (corners[i], corners[(i + 1) % 4]);
            lines.push([a.0, a.1, b.0, b.1]);
        }
    }

    #[test]
    fn overlapping_parts_are_covered_once_and_holes_go_by_the_rule() {
        // Two squares going the same way, whose right edges are 0.3 and 0.7 of the way across the same column.
        let mut lines = Vec::new();
        square(1.0, 1.0, 6.3, 6.0, true, &mut lines);
        square(3.0, 3.0, 6.7, 9.0, true, &mut lines);
        let bitmap = Rasterizer::new().fill_by_rule(&lines, 64, false);
        assert!((at(&bitmap, 6, 4) - 0.7).abs() < 1.0 / 255.0, "{}", at(&bitmap, 6, 4));
        assert_eq!(at(&bitmap, 4, 4), 1.0);
        // Added up, that column was counted as both, all of it.
        assert_eq!(at(&Rasterizer::new().fill(&mut lines.clone(), 64), 6, 4), 1.0);
        // By the even-odd rule, where they overlap is a hole: from 6 to 6.3 of that column, and the rest is the second square's.
        let bitmap = Rasterizer::new().fill_by_rule(&lines, 64, true);
        assert_eq!(at(&bitmap, 4, 4), 0.0);
        assert!((at(&bitmap, 6, 4) - 0.4).abs() < 1.0 / 255.0, "{}", at(&bitmap, 6, 4));
        assert_eq!(at(&bitmap, 2, 2), 1.0);
        // Going opposite ways, the overlap is a hole by either rule.
        let mut lines = Vec::new();
        square(1.0, 1.0, 6.3, 6.0, true, &mut lines);
        square(3.0, 3.0, 6.7, 9.0, false, &mut lines);
        for even_odd in [false, true] {
            let bitmap = Rasterizer::new().fill_by_rule(&lines, 64, even_odd);
            assert_eq!(at(&bitmap, 4, 4), 0.0);
            assert!((at(&bitmap, 6, 4) - 0.4).abs() < 1.0 / 255.0, "{}", at(&bitmap, 6, 4));
        }
    }

    #[test]
    fn what_is_left_of_a_part_of_a_row_can_change_down_it() {
        // A wide box whose top is halfway down a row, and a little one going the same way, above that and inside it, in that
        // row. The box's top isn't in what's filled, as nothing along a row is, so the little one is on its own in that row,
        // and only has the box's side left of it below the box's top.
        let mut lines = Vec::new();
        square(2.0, 2.5, 10.0, 8.0, true, &mut lines);
        square(5.0, 2.0, 6.0, 2.8, true, &mut lines);
        let bitmap = Rasterizer::new().fill_by_rule(&lines, 64, false);
        assert_eq!(at(&bitmap, 5, 2), 1.0);
        assert!((at(&bitmap, 3, 2) - 0.5).abs() < 1.0 / 255.0);
        assert!((at(&bitmap, 7, 2) - 0.5).abs() < 1.0 / 255.0);
    }

    #[test]
    fn crossing_lines_are_cut_where_they_cross() {
        // A bow tie, whose two sides cross in the middle of a pixel, and a star drawn in one go, whose middle goes round twice.
        let mut tie = vec![[0.0, 0.0, 8.0, 8.0], [8.0, 8.0, 8.0, 0.0], [8.0, 0.0, 0.0, 8.0], [0.0, 8.0, 0.0, 0.0]];
        tie.iter_mut().for_each(|line| line.iter_mut().for_each(|v| *v += 0.5));
        let bitmap = Rasterizer::new().fill_by_rule(&tie, 64, false);
        // The pixel the crossing is in, from 4 to 5 each way, around (4.5, 4.5): the two triangles left and right of it.
        assert!((at(&bitmap, 4, 4) - 0.5).abs() < 1.0 / 255.0, "{}", at(&bitmap, 4, 4));
        assert_eq!(at(&bitmap, 1, 4), 1.0);
        assert_eq!(at(&bitmap, 4, 1), 0.0);
        let star: Vec<(f32, f32)> = (0..5)
            .map(|i| {
                let turn = std::f32::consts::TAU * (i as f32 * 2.0) / 5.0 - std::f32::consts::FRAC_PI_2;
                (10.0 + 9.0 * turn.cos(), 10.0 + 9.0 * turn.sin())
            })
            .collect();
        let lines: Vec<[f32; 4]> = (0..5)
            .map(|i| {
                let (a, b) = (star[i], star[(i + 1) % 5]);
                [a.0, a.1, b.0, b.1]
            })
            .collect();
        let middle = |even_odd| at(&Rasterizer::new().fill_by_rule(&lines, 64, even_odd), 10, 10);
        assert_eq!(middle(false), 1.0);
        assert_eq!(middle(true), 0.0);
    }
}
