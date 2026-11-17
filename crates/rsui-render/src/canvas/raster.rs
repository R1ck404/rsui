//! How much of each pixel a shape covers, from the lines that go round it.
//!
//! Each line adds the exact area it leaves to its right in each pixel it crosses, with a sign for which way it goes, and
//! adding each row up from the left gives how much of each pixel is inside, as `rsui-text` draws glyphs. Rows are worked out
//! a band at a time, so a shape as big as a large canvas never needs a buffer that big, and only the part of each row that a
//! line touched is added up, so a thin diagonal costs what it covers rather than its bounding box.

/// `x.floor()`, without calling the C library for it, as that's what it compiles to without SSE4.1, and it's done for every row of
/// every line.
#[inline(always)]
pub fn floor(x: f32) -> f32 {
    if !(x.abs() < 1.0e9) {
        return x.floor();
    }
    let t = x as i32 as f32;
    if t > x { t - 1.0 } else { t }
}

/// `x.ceil()`, the same way.
#[inline(always)]
pub fn ceil(x: f32) -> f32 {
    if !(x.abs() < 1.0e9) {
        return x.ceil();
    }
    let t = x as i32 as f32;
    if t < x { t + 1.0 } else { t }
}

/// How many cells a band has at most, which is what's kept between shapes.
const BAND_CELLS: usize = 1 << 20;
/// How many cells across each mark says a line touched, so that a row a shape crosses far apart, as a wavy line's does, is
/// only added up where lines are.
const CHUNK: usize = 32;

/// A rectangle of whole pixels, from `x0, y0` up to but not including `x1, y1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl IRect {
    pub fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn intersect(&self, other: &Self) -> Self {
        Self { x0: self.x0.max(other.x0), y0: self.y0.max(other.y0), x1: self.x1.min(other.x1), y1: self.y1.min(other.y1) }
    }

    pub fn union(&self, other: &Self) -> Self {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Self { x0: self.x0.min(other.x0), y0: self.y0.min(other.y0), x1: self.x1.max(other.x1), y1: self.y1.max(other.y1) }
    }

    /// The whole pixels these lines reach.
    pub fn around(lines: &[[f32; 4]]) -> Option<Self> {
        let first = lines.first()?;
        let (mut x0, mut y0, mut x1, mut y1) = (first[0], first[1], first[0], first[1]);
        for line in lines {
            x0 = x0.min(line[0]).min(line[2]);
            x1 = x1.max(line[0]).max(line[2]);
            y0 = y0.min(line[1]).min(line[3]);
            y1 = y1.max(line[1]).max(line[3]);
        }
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            // Something reaches forever, which only what's on the canvas can show.
            return Some(Self::new(i32::MIN / 2, i32::MIN / 2, i32::MAX / 2, i32::MAX / 2));
        }
        let clamp = |v: f32| v.clamp(-1.0e8, 1.0e8);
        Some(Self::new(clamp(x0).floor() as i32, clamp(y0).floor() as i32, clamp(x1).ceil() as i32, clamp(y1).ceil() as i32))
    }
}

/// What's kept between shapes, to work out how much of each pixel they cover.
#[derive(Default)]
pub struct Rasterizer {
    /// One band's cells, all zero between uses.
    cells: Vec<f32>,
    /// Which cells each row of the band touched, from its left.
    touched: Vec<(i32, i32)>,
    /// Which chunks of `CHUNK` cells each row of the band touched, a bit each, `words` to a row.
    marks: Vec<u64>,
    words: usize,
    /// One row of coverage, as it's handed out.
    row: Vec<f32>,
    /// Every row added up cell by cell, for tests to check the wide way against.
    #[cfg(test)]
    narrow_only: bool,
}

impl Rasterizer {
    /// Works out what `lines` cover inside `clip`, and calls `each(y, x, coverage)` for each row that has any, with how much
    /// of each pixel from `x` on is inside, from 0 to 1, by the even-odd rule or the nonzero one. Says what it reached.
    pub fn fill(
        &mut self,
        lines: &[[f32; 4]],
        clip: IRect,
        even_odd: bool,
        mut each: impl FnMut(i32, i32, &[f32]),
    ) -> Option<IRect> {
        let reach = IRect::around(lines)?.intersect(&clip);
        if reach.is_empty() {
            return None;
        }
        let width = (reach.x1 - reach.x0) as usize;
        // One extra cell, for what a line at the right edge leaves past it.
        let stride = width + 2;
        let band = (BAND_CELLS / stride).max(1);
        if self.cells.len() < stride * band.min((reach.y1 - reach.y0) as usize) {
            self.cells.resize(stride * band.min((reach.y1 - reach.y0) as usize), 0.0);
        }
        if self.row.len() < width + 1 {
            self.row.resize(width + 1, 0.0);
        }
        let (left, right) = (reach.x0 as f32, reach.x1 as f32);
        let mut top = reach.y0;
        while top < reach.y1 {
            let bottom = (top + band as i32).min(reach.y1);
            let rows = (bottom - top) as usize;
            self.touched.clear();
            self.touched.resize(rows, (i32::MAX, i32::MIN));
            self.words = stride.div_ceil(CHUNK * 64);
            self.marks.clear();
            self.marks.resize(rows * self.words, 0);
            let (band_top, band_bottom) = (top as f32, bottom as f32);
            for line in lines {
                let [mut x0, mut y0, mut x1, mut y1] = *line;
                if y0 == y1 || (y0 <= band_top && y1 <= band_top) || (y0 >= band_bottom && y1 >= band_bottom) {
                    continue;
                }
                if !(x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite()) {
                    continue;
                }
                // Into the band, top to bottom.
                let at_y = |y: f32, x0: f32, y0: f32, x1: f32, y1: f32| x0 + (x1 - x0) * ((y - y0) / (y1 - y0));
                if y0 < band_top {
                    x0 = at_y(band_top, x0, y0, x1, y1);
                    y0 = band_top;
                } else if y0 > band_bottom {
                    x0 = at_y(band_bottom, x0, y0, x1, y1);
                    y0 = band_bottom;
                }
                if y1 < band_top {
                    x1 = at_y(band_top, x0, y0, x1, y1);
                    y1 = band_top;
                } else if y1 > band_bottom {
                    x1 = at_y(band_bottom, x0, y0, x1, y1);
                    y1 = band_bottom;
                }
                // Then across: what's past an edge goes along it, which keeps how much it covers of each row.
                self.add_split(x0 - left, y0 - band_top, x1 - left, y1 - band_top, right - left, stride, rows);
            }
            for r in 0..rows {
                let (lo, hi) = self.touched[r];
                if lo > hi {
                    continue;
                }
                let lo = lo.max(0) as usize;
                let hi = (hi as usize).min(stride - 1);
                let start = r * stride;
                let cells = &mut self.cells[start..start + stride];
                let end = hi.min(width - 1);
                #[cfg(test)]
                let wide = end >= lo + 4 * CHUNK && !self.narrow_only;
                #[cfg(not(test))]
                let wide = end >= lo + 4 * CHUNK;
                if lo <= end && wide {
                    // A wide row: only the chunks lines touched are added up. In the rest every cell is 0, so the total
                    // stays what it was, and the coverage is that all along, or nothing, which isn't handed out.
                    let marks = &self.marks[r * self.words..(r + 1) * self.words];
                    let cover = |total: f32| {
                        if even_odd {
                            let folded = total.abs() % 2.0;
                            if folded > 1.0 { 2.0 - folded } else { folded }
                        } else {
                            total.abs().min(1.0)
                        }
                    };
                    let mut total = 0.0f32;
                    // Where the part being handed out starts, in the row.
                    let mut from: Option<usize> = None;
                    let mut c = lo;
                    while c <= end {
                        let chunk = c / CHUNK;
                        let last = ((chunk + 1) * CHUNK - 1).min(end);
                        if marks[chunk / 64] & (1 << (chunk % 64)) != 0 {
                            let at = *from.get_or_insert(c);
                            for (out, cell) in self.row[c - at..=last - at].iter_mut().zip(&mut cells[c..=last]) {
                                total += *cell;
                                *cell = 0.0;
                                *out = cover(total);
                            }
                        } else if total == 0.0 {
                            if let Some(at) = from.take() {
                                each(top + r as i32, reach.x0 + at as i32, &self.row[..c - at]);
                            }
                        } else {
                            let at = *from.get_or_insert(c);
                            self.row[c - at..=last - at].fill(cover(total));
                        }
                        c = last + 1;
                    }
                    if let Some(at) = from {
                        each(top + r as i32, reach.x0 + at as i32, &self.row[..=end - at]);
                    }
                } else if lo <= end {
                    let mut total = 0.0f32;
                    let row = &mut self.row[..=end - lo];
                    if even_odd {
                        for (out, cell) in row.iter_mut().zip(&mut cells[lo..=end]) {
                            total += *cell;
                            *cell = 0.0;
                            let folded = total.abs() % 2.0;
                            *out = if folded > 1.0 { 2.0 - folded } else { folded };
                        }
                    } else {
                        for (out, cell) in row.iter_mut().zip(&mut cells[lo..=end]) {
                            total += *cell;
                            *cell = 0.0;
                            *out = total.abs().min(1.0);
                        }
                    }
                    each(top + r as i32, reach.x0 + lo as i32, row);
                }
                // What lands past the right edge counts for nothing shown.
                for cell in &mut cells[(end + 1).max(lo)..=hi] {
                    *cell = 0.0;
                }
            }
            top = bottom;
        }
        // A buffer that grew for one huge shape doesn't stay that big.
        if self.cells.capacity() > 4 * BAND_CELLS {
            self.cells = Vec::new();
        }
        Some(reach)
    }

    /// Adds a line, splitting it where it leaves the band's sides.
    #[allow(clippy::too_many_arguments)]
    fn add_split(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, stride: usize, rows: usize) {
        let mut stops = [(x0, y0), (x0, y0), (x0, y0), (x1, y1)];
        let mut count = 1;
        let at_x = |x: f32| y0 + (y1 - y0) * ((x - x0) / (x1 - x0));
        let (low, high) = if x0 < x1 { (0.0, width) } else { (width, 0.0) };
        for edge in [low, high] {
            if (x0 < edge && edge < x1) || (x1 < edge && edge < x0) {
                stops[count] = (edge, at_x(edge));
                count += 1;
            }
        }
        stops[count] = (x1, y1);
        for pair in 0..count {
            let (a, b) = (stops[pair], stops[pair + 1]);
            self.add_line(a.0.clamp(0.0, width), a.1, b.0.clamp(0.0, width), b.1, stride, rows);
        }
    }

    /// Adds one line's effect on every cell it passes through, in the band's own coordinates.
    fn add_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, stride: usize, rows: usize) {
        if y0 == y1 {
            return;
        }
        let (direction, x0, y0, x1, y1) = if y0 < y1 { (1.0, x0, y0, x1, y1) } else { (-1.0, x1, y1, x0, y0) };
        let slope = (x1 - x0) / (y1 - y0);
        let first_row = (y0.max(0.0) as usize).min(rows);
        let last_row = (ceil(y1).max(0.0) as usize).min(rows);
        let mut x = x0 + slope * (first_row as f32 - y0).max(0.0);
        let cells = &mut self.cells[..rows * stride];
        let touched = &mut self.touched[..rows];
        let words = self.words;
        let marks = &mut self.marks[..rows * words];
        for row in first_row..last_row {
            let row_top = row as f32;
            let dy = (row_top + 1.0).min(y1) - row_top.max(y0);
            if dy <= 0.0 {
                continue;
            }
            let x_next = x + slope * dy;
            let signed = direction * dy;
            let (left, right) = if x < x_next { (x, x_next) } else { (x_next, x) };
            let left_floor = floor(left);
            let left_pixel = left_floor.max(0.0) as usize;
            let right_ceil = ceil(right);
            let right_pixel = right_ceil.max(0.0) as usize;
            let line = &mut cells[row * stride..row * stride + stride];
            let hi;
            if right_pixel <= left_pixel + 1 {
                // Within one pixel: part of the area lands here, and the rest on the next.
                let middle = 0.5 * (x + x_next) - left_floor;
                line[left_pixel] += signed - signed * middle;
                if left_pixel + 1 < stride {
                    line[left_pixel + 1] += signed * middle;
                }
                hi = left_pixel + 1;
            } else {
                let inverse = 1.0 / (right - left);
                let left_fraction = left - left_floor;
                let first_area = 0.5 * inverse * (1.0 - left_fraction) * (1.0 - left_fraction);
                let right_fraction = right - right_ceil + 1.0;
                let last_area = 0.5 * inverse * right_fraction * right_fraction;
                line[left_pixel] += signed * first_area;
                if right_pixel == left_pixel + 2 {
                    line[left_pixel + 1] += signed * (1.0 - first_area - last_area);
                } else {
                    let second_area = inverse * (1.5 - left_fraction);
                    line[left_pixel + 1] += signed * (second_area - first_area);
                    let each = signed * inverse;
                    for cell in &mut line[left_pixel + 2..right_pixel - 1] {
                        *cell += each;
                    }
                    let so_far = second_area + (right_pixel - left_pixel - 3) as f32 * inverse;
                    line[right_pixel - 1] += signed * (1.0 - so_far - last_area);
                }
                if right_pixel < stride {
                    line[right_pixel] += signed * last_area;
                }
                hi = right_pixel;
            }
            let reached = &mut touched[row];
            reached.0 = reached.0.min(left_pixel as i32);
            reached.1 = reached.1.max(hi as i32);
            for chunk in left_pixel / CHUNK..=hi / CHUNK {
                marks[row * words + chunk / 64] |= 1 << (chunk % 64);
            }
            x = x_next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<[f32; 4]> {
        vec![[x0, y0, x1, y0], [x1, y0, x1, y1], [x1, y1, x0, y1], [x0, y1, x0, y0]]
    }

    fn total(lines: &[[f32; 4]], clip: IRect, even_odd: bool) -> f32 {
        let mut sum = 0.0;
        Rasterizer::default().fill(lines, clip, even_odd, |_, _, row| sum += row.iter().sum::<f32>());
        sum
    }

    #[test]
    fn a_square_covers_its_area() {
        let all = IRect::new(0, 0, 100, 100);
        assert!((total(&square(10.0, 10.0, 20.0, 30.0), all, false) - 200.0).abs() < 1e-3);
        assert!((total(&square(10.25, 10.5, 20.75, 30.0), all, false) - 10.5 * 19.5).abs() < 1e-3);
    }

    #[test]
    fn what_is_outside_the_clip_is_left_out() {
        let clip = IRect::new(15, 0, 100, 100);
        assert!((total(&square(10.0, 10.0, 20.0, 30.0), clip, false) - 100.0).abs() < 1e-3);
        let narrow = IRect::new(0, 0, 12, 100);
        assert!((total(&square(10.0, 10.0, 20.0, 30.0), narrow, false) - 40.0).abs() < 1e-3);
    }

    #[test]
    fn even_odd_leaves_a_hole() {
        let mut lines = square(0.0, 0.0, 10.0, 10.0);
        lines.extend(square(2.0, 2.0, 8.0, 8.0));
        let all = IRect::new(0, 0, 100, 100);
        assert!((total(&lines, all, true) - 64.0).abs() < 1e-3);
        assert!((total(&lines, all, false) - 100.0).abs() < 1e-3);
    }

    #[test]
    fn wide_rows_hand_out_only_what_lines_touch_and_the_same_coverage() {
        // A thin zigzag across 600 pixels, and a wide square with it, by both rules.
        let mut lines = Vec::new();
        let points: Vec<(f32, f32)> = (0..=60).map(|i| (i as f32 * 10.0 + 0.3, if i % 2 == 0 { 10.2 } else { 40.7 })).collect();
        for pair in points.windows(2) {
            lines.push([pair[0].0, pair[0].1, pair[1].0, pair[1].1]);
        }
        for pair in points.windows(2).rev() {
            lines.push([pair[1].0, pair[1].1 + 1.5, pair[0].0, pair[0].1 + 1.5]);
        }
        lines.push([points[0].0, points[0].1 + 1.5, points[0].0, points[0].1]);
        let last = points[points.len() - 1];
        lines.push([last.0, last.1, last.0, last.1 + 1.5]);
        lines.extend(square(50.5, 20.25, 400.75, 30.5));
        let clip = IRect::new(0, 0, 700, 60);
        for even_odd in [false, true] {
            let mut got = vec![0.0f32; 700 * 60];
            let mut handed = 0;
            Rasterizer::default().fill(&lines, clip, even_odd, |y, x, row| {
                handed += row.len();
                got[y as usize * 700 + x as usize..][..row.len()].copy_from_slice(row);
            });
            // Every cell of every row added up.
            let mut expected = vec![0.0f32; 700 * 60];
            let mut narrow = Rasterizer { narrow_only: true, ..Rasterizer::default() };
            narrow.fill(&lines, clip, even_odd, |y, x, row| {
                expected[y as usize * 700 + x as usize..][..row.len()].copy_from_slice(row);
            });
            assert!(handed < 700 * 30, "{handed}");
            for (i, (a, b)) in got.iter().zip(&expected).enumerate() {
                assert_eq!(a.to_bits(), b.to_bits(), "{even_odd} at {i}: {a} {b}");
            }
        }
    }

    #[test]
    fn tall_shapes_go_band_by_band() {
        let clip = IRect::new(0, 0, 4000, 4000);
        let area = total(&square(0.0, 0.0, 4000.0, 4000.0), clip, false);
        assert!((area - 16.0e6).abs() / 16.0e6 < 1e-4, "{area}");
    }
}
