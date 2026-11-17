//! Draws glyphs that are made of gradients and shapes on top of each other, from a font's `COLR` table. Each
//! is drawn once, to pixels of its own, at each size it's wanted in, and is an ordinary picture after that.

use crate::font::{Font, read_i16, read_u16, read_u32};
use crate::raster::{GlyphBitmap, Rasterizer};

/// The largest of these to draw, in pixels, so that nobody's asked for a huge one by a damaged font.
const MAX_SIDE: i32 = 512;
/// How many steps drawing one can take, and how many pixels it can touch doing them, which real fonts come
/// nowhere near. A damaged one could otherwise take minutes.
const BUDGET: u32 = 20_000;
const PIXEL_BUDGET: u64 = 400_000_000;
const MAX_DEPTH: u32 = 32;

/// A picture of a glyph, drawn where its pen would be.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ColorBitmap {
    /// Where the left edge goes, from the pen, in pixels.
    pub left: i32,
    /// Where the top edge goes, from the baseline, in pixels. Down is positive.
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// Red, green, blue and alpha, with colors multiplied by alpha.
    pub pixels: Vec<u8>,
}

/// From a glyph's own units to pixels, or to anything else: `x' = a x + c y + e`, `y' = b x + d y + f`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Matrix {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Matrix {
    const fn translate(x: f32, y: f32) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: x,
            f: y,
        }
    }

    const fn scale(x: f32, y: f32) -> Self {
        Self {
            a: x,
            b: 0.0,
            c: 0.0,
            d: y,
            e: 0.0,
            f: 0.0,
        }
    }

    /// What doing `then` after this does.
    fn then(self, then: Self) -> Self {
        Self {
            a: then.a * self.a + then.c * self.b,
            b: then.b * self.a + then.d * self.b,
            c: then.a * self.c + then.c * self.d,
            d: then.b * self.c + then.d * self.d,
            e: then.a * self.e + then.c * self.f + then.e,
            f: then.b * self.e + then.d * self.f + then.f,
        }
    }

    /// Doing this around a point, rather than around where the axes cross.
    fn around(self, x: f32, y: f32) -> Self {
        Self::translate(-x, -y)
            .then(self)
            .then(Self::translate(x, y))
    }

    fn inverse(self) -> Option<Self> {
        let determinant = self.a * self.d - self.b * self.c;
        if determinant.abs() < 1e-12 {
            return None;
        }
        let scale = 1.0 / determinant;
        Some(Self {
            a: self.d * scale,
            b: -self.b * scale,
            c: -self.c * scale,
            d: self.a * scale,
            e: (self.c * self.f - self.d * self.e) * scale,
            f: (self.b * self.e - self.a * self.f) * scale,
        })
    }

    #[inline]
    fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
}

fn read_u24(data: &[u8], at: usize) -> Option<usize> {
    Some((usize::from(*data.get(at)?) << 16) | read_u16(data, at + 1)? as usize)
}

fn read_fixed_2_14(data: &[u8], at: usize) -> Option<f32> {
    Some(f32::from(read_i16(data, at)?) / 16384.0)
}

fn read_fixed_16_16(data: &[u8], at: usize) -> Option<f32> {
    Some(read_u32(data, at)? as i32 as f32 / 65536.0)
}

fn fword(data: &[u8], at: usize) -> Option<f32> {
    Some(f32::from(read_i16(data, at)?))
}

/// Where a gradient goes past its ends.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Extend {
    Pad,
    Repeat,
    Reflect,
}

struct Gradient {
    stops: Vec<(f32, [f32; 4])>,
    extend: Extend,
}

impl Gradient {
    /// The color this far along it, multiplied by its alpha.
    fn at(&self, t: f32) -> [f32; 4] {
        let t = match self.extend {
            Extend::Pad => t,
            Extend::Repeat => t - t.floor(),
            Extend::Reflect => {
                let part = t.rem_euclid(2.0);
                if part > 1.0 { 2.0 - part } else { part }
            }
        };
        let after = self.stops.partition_point(|stop| stop.0 <= t);
        if after == 0 {
            return self.stops[0].1;
        }
        if after == self.stops.len() {
            return self.stops[after - 1].1;
        }
        let (start, from) = self.stops[after - 1];
        let (end, to) = self.stops[after];
        let part = if end > start {
            (t - start) / (end - start)
        } else {
            0.0
        };
        std::array::from_fn(|index| from[index] + (to[index] - from[index]) * part)
    }
}

/// What to fill something with, at a place in the glyph's own units.
enum Fill {
    Solid([f32; 4]),
    Linear {
        gradient: Gradient,
        start: (f32, f32),
        along: (f32, f32),
        length_squared: f32,
    },
    Radial {
        gradient: Gradient,
        start: (f32, f32),
        start_radius: f32,
        toward: (f32, f32),
        growth: f32,
    },
    Sweep {
        gradient: Gradient,
        center: (f32, f32),
        start: f32,
        end: f32,
    },
}

impl Fill {
    fn at(&self, x: f32, y: f32) -> [f32; 4] {
        match self {
            Self::Solid(color) => *color,
            Self::Linear {
                gradient,
                start,
                along,
                length_squared,
            } => {
                let t = ((x - start.0) * along.0 + (y - start.1) * along.1) / length_squared;
                gradient.at(t)
            }
            Self::Radial {
                gradient,
                start,
                start_radius,
                toward,
                growth,
            } => {
                let (px, py) = (x - start.0, y - start.1);
                let a = toward.0 * toward.0 + toward.1 * toward.1 - growth * growth;
                let b = px * toward.0 + py * toward.1 + start_radius * growth;
                let c = px * px + py * py - start_radius * start_radius;
                let usable = |t: f32| start_radius + t * growth >= 0.0;
                let t = if a.abs() < 1e-6 {
                    (b.abs() > 1e-6)
                        .then(|| c / (2.0 * b))
                        .filter(|t| usable(*t))
                } else {
                    let under = b * b - a * c;
                    if under < 0.0 {
                        None
                    } else {
                        let root = under.sqrt();
                        let (first, second) = ((b + root) / a, (b - root) / a);
                        let (high, low) = if first > second {
                            (first, second)
                        } else {
                            (second, first)
                        };
                        [high, low].into_iter().find(|t| usable(*t))
                    }
                };
                t.map_or([0.0; 4], |t| gradient.at(t))
            }
            Self::Sweep {
                gradient,
                center,
                start,
                end,
            } => {
                let angle = (y - center.1)
                    .atan2(x - center.0)
                    .to_degrees()
                    .rem_euclid(360.0);
                let span = end - start;
                if span.abs() < 1e-6 {
                    return [0.0; 4];
                }
                gradient.at((angle - start) / span)
            }
        }
    }
}

struct Painter<'a> {
    font: &'a Font,
    raster: &'a mut Rasterizer,
    table: &'a [u8],
    glyph_list: usize,
    layer_list: usize,
    text_color: [f32; 4],
    width: usize,
    height: usize,
    left: i32,
    top: i32,
    budget: u32,
    pixels_left: u64,
}

type Canvas = Vec<[f32; 4]>;

impl Painter<'_> {
    /// A color from the font's palette, multiplied by its alpha and by this one. The last number there is
    /// means the color of the text.
    fn color(&self, index: u16, alpha: f32) -> [f32; 4] {
        let [r, g, b, a] = if index == 0xFFFF {
            self.text_color
        } else {
            self.font
                .palette_color(index)
                .map_or(self.text_color, |rgba| {
                    rgba.map(|part| f32::from(part) / 255.0)
                })
        };
        let a = a * alpha.clamp(0.0, 1.0);
        [r * a, g * a, b * a, a]
    }

    fn color_line(&self, at: usize) -> Option<Gradient> {
        let table = self.table;
        let extend = match *table.get(at)? {
            1 => Extend::Repeat,
            2 => Extend::Reflect,
            _ => Extend::Pad,
        };
        let count = read_u16(table, at + 1)? as usize;
        if count == 0 {
            return None;
        }
        let mut stops = Vec::with_capacity(count);
        for stop in 0..count {
            let item = at + 3 + stop * 6;
            let offset = read_fixed_2_14(table, item)?;
            let index = read_u16(table, item + 2)?;
            let alpha = read_fixed_2_14(table, item + 4)?;
            stops.push((offset, self.color(index, alpha)));
        }
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        Some(Gradient { stops, extend })
    }

    /// Fills the canvas, as far as the clip lets it.
    fn fill(&self, fill: &Fill, matrix: Matrix, clip: Option<&[f32]>, canvas: &mut Canvas) {
        let Some(inverse) = matrix.inverse() else {
            return;
        };
        for y in 0..self.height {
            for x in 0..self.width {
                let index = y * self.width + x;
                let cover = clip.map_or(1.0, |clip| clip[index]);
                if cover <= 0.0 {
                    continue;
                }
                let (font_x, font_y) = inverse.apply(
                    (x as i32 + self.left) as f32 + 0.5,
                    (y as i32 + self.top) as f32 + 0.5,
                );
                let color = fill.at(font_x, font_y);
                let keep = 1.0 - color[3] * cover;
                let out = &mut canvas[index];
                for part in 0..4 {
                    out[part] = color[part] * cover + out[part] * keep;
                }
            }
        }
    }

    /// What a glyph covers, in the canvas, within what's already clipped.
    fn glyph_clip(&mut self, glyph: u16, matrix: Matrix, clip: Option<&[f32]>) -> Vec<f32> {
        let bitmap: GlyphBitmap = self.raster.rasterize_with(
            self.font,
            glyph,
            [matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f],
        );
        let mut mask = vec![0f32; self.width * self.height];
        for row in 0..bitmap.height as i32 {
            let y = bitmap.top + row - self.top;
            if y < 0 || y as usize >= self.height {
                continue;
            }
            for column in 0..bitmap.width as i32 {
                let x = bitmap.left + column - self.left;
                if x < 0 || x as usize >= self.width {
                    continue;
                }
                let index = y as usize * self.width + x as usize;
                let cover = f32::from(
                    bitmap.coverage[row as usize * bitmap.width as usize + column as usize],
                ) / 255.0;
                mask[index] = cover * clip.map_or(1.0, |clip| clip[index]);
            }
        }
        mask
    }

    fn draw_transformed(
        &mut self,
        paint: usize,
        transform: Matrix,
        matrix: Matrix,
        clip: Option<&[f32]>,
        canvas: &mut Canvas,
        depth: u32,
    ) -> Option<()> {
        let inner = read_u24(self.table, paint + 1)?;
        self.draw(paint + inner, transform.then(matrix), clip, canvas, depth)
    }

    /// Draws one of the font's drawing steps, and what it holds. Each odd one is the same as the even one
    /// before it, for fonts with variations, which this doesn't do anything with.
    #[allow(clippy::too_many_lines)]
    fn draw(
        &mut self,
        paint: usize,
        matrix: Matrix,
        clip: Option<&[f32]>,
        canvas: &mut Canvas,
        depth: u32,
    ) -> Option<()> {
        if depth >= MAX_DEPTH {
            return None;
        }
        self.budget = self.budget.checked_sub(1)?;
        self.pixels_left = self
            .pixels_left
            .checked_sub((self.width * self.height) as u64)?;
        let table = self.table;
        let depth = depth + 1;
        let format = *table.get(paint)?;
        let at = paint + 4;
        match format {
            1 => {
                let count = usize::from(*table.get(paint + 1)?);
                let first = read_u32(table, paint + 2)? as usize;
                for layer in first..first + count {
                    let offset = read_u32(table, self.layer_list + 4 + layer * 4)? as usize;
                    self.draw(self.layer_list + offset, matrix, clip, canvas, depth)?;
                }
            }
            2 | 3 => {
                let index = read_u16(table, paint + 1)?;
                let alpha = read_fixed_2_14(table, paint + 3)?;
                self.fill(&Fill::Solid(self.color(index, alpha)), matrix, clip, canvas);
            }
            4 | 5 => {
                let gradient = self.color_line(paint + read_u24(table, paint + 1)?)?;
                let point = |index: usize| -> Option<(f32, f32)> {
                    Some((
                        fword(table, at + index * 4)?,
                        fword(table, at + index * 4 + 2)?,
                    ))
                };
                let (start, end, rotation) = (point(0)?, point(1)?, point(2)?);
                // The direction it changes in is across the line from the start to the third point.
                let across = (rotation.0 - start.0, rotation.1 - start.1);
                let toward = (end.0 - start.0, end.1 - start.1);
                let normal = (-across.1, across.0);
                let normal_squared = normal.0 * normal.0 + normal.1 * normal.1;
                let along = if normal_squared > 0.0 {
                    let share = (toward.0 * normal.0 + toward.1 * normal.1) / normal_squared;
                    (normal.0 * share, normal.1 * share)
                } else {
                    toward
                };
                let length_squared = along.0 * along.0 + along.1 * along.1;
                if length_squared > 0.0 {
                    let fill = Fill::Linear {
                        gradient,
                        start,
                        along,
                        length_squared,
                    };
                    self.fill(&fill, matrix, clip, canvas);
                }
            }
            6 | 7 => {
                let gradient = self.color_line(paint + read_u24(table, paint + 1)?)?;
                let start = (fword(table, at)?, fword(table, at + 2)?);
                let start_radius = read_u16(table, at + 4)? as f32;
                let end = (fword(table, at + 6)?, fword(table, at + 8)?);
                let end_radius = read_u16(table, at + 10)? as f32;
                let fill = Fill::Radial {
                    gradient,
                    start,
                    start_radius,
                    toward: (end.0 - start.0, end.1 - start.1),
                    growth: end_radius - start_radius,
                };
                self.fill(&fill, matrix, clip, canvas);
            }
            8 | 9 => {
                let gradient = self.color_line(paint + read_u24(table, paint + 1)?)?;
                let center = (fword(table, at)?, fword(table, at + 2)?);
                let start = read_fixed_2_14(table, at + 4)? * 180.0;
                let end = read_fixed_2_14(table, at + 6)? * 180.0;
                let fill = Fill::Sweep {
                    gradient,
                    center,
                    start,
                    end,
                };
                self.fill(&fill, matrix, clip, canvas);
            }
            10 => {
                let inner = paint + read_u24(table, paint + 1)?;
                let glyph = read_u16(table, paint + 4)?;
                let mask = self.glyph_clip(glyph, matrix, clip);
                self.draw(inner, matrix, Some(&mask), canvas, depth)?;
            }
            11 => {
                let glyph = read_u16(table, paint + 1)?;
                let inner = self.paint_of(glyph)?;
                self.draw(inner, matrix, clip, canvas, depth)?;
            }
            12 | 13 => {
                let inner = paint + read_u24(table, paint + 1)?;
                let values = paint + read_u24(table, paint + 4)?;
                let part = |index: usize| read_fixed_16_16(table, values + index * 4);
                let transform = Matrix {
                    a: part(0)?,
                    b: part(1)?,
                    c: part(2)?,
                    d: part(3)?,
                    e: part(4)?,
                    f: part(5)?,
                };
                self.draw(inner, transform.then(matrix), clip, canvas, depth)?;
            }
            14 | 15 => {
                let transform = Matrix::translate(fword(table, at)?, fword(table, at + 2)?);
                self.draw_transformed(paint, transform, matrix, clip, canvas, depth)?;
            }
            16 | 17 => {
                let transform =
                    Matrix::scale(read_fixed_2_14(table, at)?, read_fixed_2_14(table, at + 2)?);
                self.draw_transformed(paint, transform, matrix, clip, canvas, depth)?;
            }
            18 | 19 => {
                let transform =
                    Matrix::scale(read_fixed_2_14(table, at)?, read_fixed_2_14(table, at + 2)?)
                        .around(fword(table, at + 4)?, fword(table, at + 6)?);
                self.draw_transformed(paint, transform, matrix, clip, canvas, depth)?;
            }
            20 | 21 => {
                let scale = read_fixed_2_14(table, at)?;
                self.draw_transformed(
                    paint,
                    Matrix::scale(scale, scale),
                    matrix,
                    clip,
                    canvas,
                    depth,
                )?;
            }
            22 | 23 => {
                let scale = read_fixed_2_14(table, at)?;
                let transform = Matrix::scale(scale, scale)
                    .around(fword(table, at + 2)?, fword(table, at + 4)?);
                self.draw_transformed(paint, transform, matrix, clip, canvas, depth)?;
            }
            24..=27 => {
                let angle = read_fixed_2_14(table, at)? * std::f32::consts::PI;
                let (sin, cos) = angle.sin_cos();
                let mut transform = Matrix {
                    a: cos,
                    b: sin,
                    c: -sin,
                    d: cos,
                    e: 0.0,
                    f: 0.0,
                };
                if format >= 26 {
                    transform = transform.around(fword(table, at + 2)?, fword(table, at + 4)?);
                }
                self.draw_transformed(paint, transform, matrix, clip, canvas, depth)?;
            }
            28..=31 => {
                let x = read_fixed_2_14(table, at)? * std::f32::consts::PI;
                let y = read_fixed_2_14(table, at + 2)? * std::f32::consts::PI;
                let mut transform = Matrix {
                    a: 1.0,
                    b: y.tan(),
                    c: -x.tan(),
                    d: 1.0,
                    e: 0.0,
                    f: 0.0,
                };
                if format >= 30 {
                    transform = transform.around(fword(table, at + 4)?, fword(table, at + 6)?);
                }
                self.draw_transformed(paint, transform, matrix, clip, canvas, depth)?;
            }
            32 => {
                let source = paint + read_u24(table, paint + 1)?;
                let mode = *table.get(paint + 4)?;
                let backdrop = paint + read_u24(table, paint + 5)?;
                let size = self.width * self.height;
                let mut over = vec![[0f32; 4]; size];
                let mut under = vec![[0f32; 4]; size];
                self.draw(source, matrix, None, &mut over, depth)?;
                self.draw(backdrop, matrix, None, &mut under, depth)?;
                for index in 0..size {
                    let mixed = composite(mode, over[index], under[index]);
                    let cover = clip.map_or(1.0, |clip| clip[index]);
                    let keep = 1.0 - mixed[3] * cover;
                    let out = &mut canvas[index];
                    for part in 0..4 {
                        out[part] = mixed[part] * cover + out[part] * keep;
                    }
                }
            }
            _ => {}
        }
        Some(())
    }

    /// Where a glyph's drawing starts, if it has one.
    fn paint_of(&self, glyph: u16) -> Option<usize> {
        paint_of(self.table, self.glyph_list, glyph)
    }
}

/// Where a glyph's drawing starts, in a list of them that's sorted by glyph.
fn paint_of(table: &[u8], list: usize, glyph: u16) -> Option<usize> {
    let (mut low, mut high) = (0, read_u32(table, list)? as usize);
    while low < high {
        let middle = (low + high) / 2;
        let record = list + 4 + middle * 6;
        match read_u16(table, record)?.cmp(&glyph) {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
            std::cmp::Ordering::Equal => {
                return Some(list + read_u32(table, record + 2)? as usize);
            }
        }
    }
    None
}

/// Puts one picture on another in one of the ways a font can ask for. The ones that mix colors in other ways
/// are taken as simply drawing on top.
fn composite(mode: u8, source: [f32; 4], backdrop: [f32; 4]) -> [f32; 4] {
    let (over, under) = (source[3], backdrop[3]);
    let (from_source, from_backdrop) = match mode {
        0 => (0.0, 0.0),
        1 => (1.0, 0.0),
        2 => (0.0, 1.0),
        4 => (1.0 - under, 1.0),
        5 => (under, 0.0),
        6 => (0.0, over),
        7 => (1.0 - under, 0.0),
        8 => (0.0, 1.0 - over),
        9 => (under, 1.0 - over),
        10 => (1.0 - under, over),
        11 => (1.0 - under, 1.0 - over),
        12 => (1.0, 1.0),
        _ => (1.0, 1.0 - over),
    };
    std::array::from_fn(|part| {
        (source[part] * from_source + backdrop[part] * from_backdrop).min(1.0)
    })
}

/// Where a glyph says it stays within, in its own units, as least and most across and up.
fn clip_box(table: &[u8], list: usize, glyph: u16) -> Option<[f32; 4]> {
    let count = read_u32(table, list + 1)? as usize;
    let (mut low, mut high) = (0, count);
    while low < high {
        let middle = (low + high) / 2;
        let record = list + 5 + middle * 7;
        let (first, last) = (read_u16(table, record)?, read_u16(table, record + 2)?);
        if last < glyph {
            low = middle + 1;
        } else if first > glyph {
            high = middle;
        } else {
            let at = list + read_u24(table, record + 4)?;
            return Some([
                fword(table, at + 1)?,
                fword(table, at + 3)?,
                fword(table, at + 5)?,
                fword(table, at + 7)?,
            ]);
        }
    }
    None
}

impl Rasterizer {
    /// Draws a glyph made of gradients at `size` pixels per em, if this one is. The color of the text only
    /// matters for what the font says should be that color.
    pub fn rasterize_color(
        &mut self,
        font: &Font,
        glyph: u16,
        size: f32,
        text_color: [u8; 4],
    ) -> Option<ColorBitmap> {
        let (table, glyph_list, layer_list, clip_list) = font.gradient_tables()?;
        let paint = paint_of(table, glyph_list, glyph)?;
        let metrics = font.metrics();
        let scale = size / f32::from(metrics.units_per_em);
        if !(scale.is_finite() && scale > 0.0) {
            return None;
        }
        let bounds = clip_list
            .and_then(|list| clip_box(table, list, glyph))
            .unwrap_or_else(|| {
                [
                    -f32::from(metrics.units_per_em) * 0.5,
                    f32::from(metrics.descent),
                    f32::from(font.advance(glyph)) + f32::from(metrics.units_per_em) * 0.5,
                    f32::from(metrics.ascent),
                ]
            });
        // A pixel to spare at each side, as edges fall between pixels.
        let left = (bounds[0] * scale).floor() as i32 - 1;
        let top = (-bounds[3] * scale).floor() as i32 - 1;
        let right = (bounds[2] * scale).ceil() as i32 + 1;
        let bottom = (-bounds[1] * scale).ceil() as i32 + 1;
        let (width, height) = (right - left, bottom - top);
        if width <= 0 || height <= 0 || width > MAX_SIDE || height > MAX_SIDE {
            return None;
        }
        let (width, height) = (width as usize, height as usize);
        let mut canvas = vec![[0f32; 4]; width * height];
        let [r, g, b, a] = text_color.map(|part| f32::from(part) / 255.0);
        let mut painter = Painter {
            font,
            raster: self,
            table,
            glyph_list,
            layer_list,
            text_color: [r, g, b, a],
            width,
            height,
            left,
            top,
            budget: BUDGET,
            pixels_left: PIXEL_BUDGET,
        };
        let matrix = Matrix {
            a: scale,
            b: 0.0,
            c: 0.0,
            d: -scale,
            e: 0.0,
            f: 0.0,
        };
        painter.draw(paint, matrix, None, &mut canvas, 0)?;
        let mut pixels = Vec::with_capacity(width * height * 4);
        for color in &canvas {
            for part in color {
                pixels.push((part.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            }
        }
        Some(ColorBitmap {
            left,
            top,
            width: width as u32,
            height: height as u32,
            pixels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrices_go_there_and_back() {
        let matrix = Matrix {
            a: 2.0,
            b: 1.0,
            c: -1.0,
            d: 3.0,
            e: 5.0,
            f: -7.0,
        };
        let (x, y) = matrix.apply(4.0, 9.0);
        let (back_x, back_y) = matrix.inverse().unwrap().apply(x, y);
        assert!((back_x - 4.0).abs() < 1e-4 && (back_y - 9.0).abs() < 1e-4);
        assert!(Matrix::scale(0.0, 1.0).inverse().is_none());
    }

    #[test]
    fn scaling_around_a_point_leaves_it_where_it_is() {
        let matrix = Matrix::scale(2.0, 3.0).around(10.0, 20.0);
        assert_eq!(matrix.apply(10.0, 20.0), (10.0, 20.0));
        assert_eq!(matrix.apply(11.0, 20.0), (12.0, 20.0));
    }

    #[test]
    fn gradients_mix_and_go_on_past_their_ends() {
        let stops = vec![(0.0, [0.0, 0.0, 0.0, 1.0]), (1.0, [1.0, 1.0, 1.0, 1.0])];
        let pad = Gradient {
            stops: stops.clone(),
            extend: Extend::Pad,
        };
        assert_eq!(pad.at(0.5)[0], 0.5);
        assert_eq!(pad.at(-3.0)[0], 0.0);
        assert_eq!(pad.at(3.0)[0], 1.0);
        let repeat = Gradient {
            stops: stops.clone(),
            extend: Extend::Repeat,
        };
        assert!((repeat.at(1.25)[0] - 0.25).abs() < 1e-5);
        let reflect = Gradient {
            stops,
            extend: Extend::Reflect,
        };
        assert!((reflect.at(1.25)[0] - 0.75).abs() < 1e-5);
    }

    #[test]
    fn a_radial_fill_is_empty_where_no_circle_reaches() {
        let gradient = Gradient {
            stops: vec![(0.0, [1.0, 0.0, 0.0, 1.0]), (1.0, [0.0, 0.0, 1.0, 1.0])],
            extend: Extend::Pad,
        };
        // A circle that doesn't grow, moving right, reaches nothing far above it.
        let fill = Fill::Radial {
            gradient,
            start: (0.0, 0.0),
            start_radius: 1.0,
            toward: (10.0, 0.0),
            growth: 0.0,
        };
        assert_eq!(fill.at(5.0, 50.0), [0.0; 4]);
        assert!(fill.at(5.0, 0.0)[3] > 0.0);
    }

    #[test]
    fn composites_do_what_their_names_say() {
        let source = [0.5, 0.0, 0.0, 0.5];
        let backdrop = [0.0, 1.0, 0.0, 1.0];
        assert_eq!(composite(1, source, backdrop), source);
        assert_eq!(composite(2, source, backdrop), backdrop);
        assert_eq!(composite(5, source, [0.0; 4]), [0.0; 4]);
        assert_eq!(composite(5, source, backdrop), source);
        assert_eq!(composite(3, source, backdrop), [0.5, 0.5, 0.0, 1.0]);
    }
}
