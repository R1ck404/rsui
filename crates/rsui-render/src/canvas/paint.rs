//! What a canvas paints with, and how it lays that on what's there.
//!
//! Colors are kept as a page's canvas keeps them, red, green, blue and alpha in a byte each, with the colors multiplied by alpha.
//! Gradients mix their stops' colors before that, as Chromium does, and are looked up from a table of 1024 steps. Pictures are
//! sampled between pixels when smoothing is on, and at the nearest one when it's off.

use std::rc::Rc;

use super::geom::Matrix;

/// A picture to draw or repeat, with its colors multiplied by alpha.
#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    #[inline]
    fn texel(&self, x: i32, y: i32) -> [f32; 4] {
        let at = (y as usize * self.width as usize + x as usize) * 4;
        let p = &self.pixels[at..at + 4];
        [f32::from(p[0]), f32::from(p[1]), f32::from(p[2]), f32::from(p[3])]
    }
}

/// How a pattern repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Both,
    X,
    Y,
    Neither,
}

/// What a gradient goes along.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GradientKind {
    Linear { x0: f32, y0: f32, x1: f32, y1: f32 },
    Radial { x0: f32, y0: f32, r0: f32, x1: f32, y1: f32, r1: f32 },
    Conic { angle: f32, x: f32, y: f32 },
}

const STEPS: usize = 1024;

/// A gradient's colors at each of its steps, multiplied by alpha: from 0 to 255 as floating point, and packed as the canvas has
/// pixels, rounded as `Shader::row32` rounds them, so a row of it at full strength is only looked up.
#[derive(Debug)]
pub struct GradientTable {
    pub floats: Vec<[f32; 4]>,
    pub packed: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct Gradient {
    pub kind: GradientKind,
    /// Each stop's offset and color, without alpha multiplied in, in the order they were added, which is also by offset.
    stops: Vec<(f32, [u8; 4])>,
    table: Option<Rc<GradientTable>>,
}

impl Gradient {
    pub fn new(kind: GradientKind) -> Self {
        Self { kind, stops: Vec::new(), table: None }
    }

    pub fn add_stop(&mut self, offset: f32, color: [u8; 4]) {
        // Stops at the same offset keep the order they were added in, which is what makes a sharp change.
        let at = self.stops.partition_point(|(o, _)| *o <= offset);
        self.stops.insert(at, (offset, color));
        self.table = None;
    }

    pub fn has_stops(&self) -> bool {
        !self.stops.is_empty()
    }

    /// The colors along it, multiplied by alpha, from 0 to 255.
    pub fn table(&mut self) -> Rc<GradientTable> {
        if let Some(table) = &self.table {
            return table.clone();
        }
        let mut out = Vec::with_capacity(STEPS);
        for i in 0..STEPS {
            let t = (i as f32 + 0.5) / STEPS as f32;
            let c = self.color_at(t);
            out.push([c[0] * 255.0, c[1] * 255.0, c[2] * 255.0, c[3] * 255.0]);
        }
        let packed = out.iter().map(|s| pack([(s[0] + 0.5) as u32, (s[1] + 0.5) as u32, (s[2] + 0.5) as u32, (s[3] + 0.5) as u32])).collect();
        let table = Rc::new(GradientTable { floats: out, packed });
        self.table = Some(table.clone());
        table
    }

    fn color_at(&self, t: f32) -> [f32; 4] {
        let stops = &self.stops;
        let premultiply = |c: [f32; 4]| [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]];
        let unit = |c: [u8; 4]| [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, f32::from(c[3]) / 255.0];
        if t <= stops[0].0 {
            return premultiply(unit(stops[0].1));
        }
        let last = stops[stops.len() - 1];
        if t >= last.0 {
            return premultiply(unit(last.1));
        }
        // The last stop at or before t, and the one after it.
        let after = stops.partition_point(|(o, _)| *o <= t);
        let (o0, c0) = stops[after - 1];
        let (o1, c1) = stops[after];
        let f = if o1 > o0 { (t - o0) / (o1 - o0) } else { 1.0 };
        let (c0, c1) = (unit(c0), unit(c1));
        premultiply([
            c0[0] + (c1[0] - c0[0]) * f,
            c0[1] + (c1[1] - c0[1]) * f,
            c0[2] + (c1[2] - c0[2]) * f,
            c0[3] + (c1[3] - c0[3]) * f,
        ])
    }
}

#[derive(Debug, Clone)]
pub struct Pattern {
    pub image: Rc<Image>,
    pub repeat: Repeat,
    pub transform: Matrix,
}

/// What a fill or stroke is in, as the state has it: a color, or a gradient or pattern by number.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Style {
    /// Red, green, blue and alpha, not multiplied.
    Color([u8; 4]),
    Gradient(u32),
    Pattern(u32),
}

/// What's painted, ready to be sampled at each pixel.
pub enum Shader {
    /// Multiplied by alpha, from 0 to 255.
    Solid([f32; 4]),
    Linear { table: Rc<GradientTable>, to_user: Matrix, x0: f32, y0: f32, dx: f32, dy: f32, inv_len2: f32 },
    Radial { table: Rc<GradientTable>, to_user: Matrix, kind: [f32; 6] },
    Conic { table: Rc<GradientTable>, to_user: Matrix, angle: f32, x: f32, y: f32 },
    Image { image: Rc<Image>, to_image: Matrix, smooth: bool, repeat: Repeat, bounds: [i32; 4], pattern: bool },
    /// Nothing at all, as a gradient with no stops is.
    None,
}

/// A linear or radial gradient's shader as borrowed parts, which, unlike a `Shader`, can be shared by threads drawing bands of
/// one fill (see `pool`).
#[derive(Clone, Copy)]
pub enum GradientRow<'a> {
    Linear { table: &'a GradientTable, to_user: Matrix, x0: f32, y0: f32, dx: f32, dy: f32, inv_len2: f32 },
    Radial { table: &'a GradientTable, to_user: Matrix, kind: [f32; 6] },
}

impl GradientRow<'_> {
    /// A row of it from `x, y`, packed, at full strength, as `Shader::row32` gives it.
    pub fn row32(&self, x: i32, y: i32, out: &mut [u32]) {
        match *self {
            Self::Linear { table, to_user, x0, y0, dx, dy, inv_len2 } => {
                let (mut ux, mut uy) = to_user.apply(x as f32 + 0.5, y as f32 + 0.5);
                let (step_x, step_y) = (to_user.a, to_user.b);
                for px in out.iter_mut() {
                    *px = table.packed[step_of(((ux - x0) * dx + (uy - y0) * dy) * inv_len2)];
                    ux += step_x;
                    uy += step_y;
                }
            }
            Self::Radial { table, to_user, kind } => {
                let packed = &table.packed[..];
                radial_row(&kind, &to_user, x, y, out.len(), |i, t| {
                    out[i] = t.map_or(0, |t| packed[step_of(t)]);
                });
            }
        }
    }
}

impl Shader {
    /// A linear or radial gradient's parts, to be drawn by threads.
    pub fn gradient(&self) -> Option<GradientRow<'_>> {
        match self {
            Self::Linear { table, to_user, x0, y0, dx, dy, inv_len2 } => Some(GradientRow::Linear { table, to_user: *to_user, x0: *x0, y0: *y0, dx: *dx, dy: *dy, inv_len2: *inv_len2 }),
            Self::Radial { table, to_user, kind } => Some(GradientRow::Radial { table, to_user: *to_user, kind: *kind }),
            _ => None,
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Whether it's the same everywhere.
    pub fn solid(&self) -> Option<[f32; 4]> {
        match self {
            Self::Solid(color) => Some(*color),
            _ => None,
        }
    }

    /// Fills `out` with what's painted at each pixel of a row from `x`, from 0 to 255, multiplied by alpha.
    pub fn row(&self, x: i32, y: i32, out: &mut [[f32; 4]]) {
        let center = |m: &Matrix, i: usize| m.apply(x as f32 + i as f32 + 0.5, y as f32 + 0.5);
        match self {
            Self::None => out.fill([0.0; 4]),
            Self::Solid(color) => out.fill(*color),
            Self::Linear { table, to_user, x0, y0, dx, dy, inv_len2 } => {
                let (mut ux, mut uy) = center(to_user, 0);
                let (step_x, step_y) = (to_user.a, to_user.b);
                for px in out.iter_mut() {
                    let t = ((ux - x0) * dx + (uy - y0) * dy) * inv_len2;
                    *px = look_up(&table.floats, t);
                    ux += step_x;
                    uy += step_y;
                }
            }
            Self::Radial { table, to_user, kind } => {
                radial_row(kind, to_user, x, y, out.len(), |i, t| {
                    out[i] = t.map_or([0.0; 4], |t| look_up(&table.floats, t));
                });
            }
            Self::Conic { table, to_user, angle, x: cx, y: cy } => {
                for (i, px) in out.iter_mut().enumerate() {
                    let (ux, uy) = center(to_user, i);
                    let mut t = ((uy - cy).atan2(ux - cx) - angle) / std::f32::consts::TAU;
                    t -= t.floor();
                    *px = look_up(&table.floats, t);
                }
            }
            Self::Image { image, to_image, smooth, repeat, bounds, pattern } => {
                for (i, px) in out.iter_mut().enumerate() {
                    // Worked out for each pixel rather than stepped to, so that nothing adds up to land on the wrong side
                    // of a pixel when sampling the nearest.
                    let (u, v) = center(to_image, i);
                    *px = sample(image, u, v, *smooth, *repeat, bounds, *pattern);
                }
            }
        }
    }
}

impl Shader {
    /// The same as [`row`](Self::row), as pixels packed as they are on the canvas (red in the low byte, alpha in the high one),
    /// multiplied by alpha, and by `alpha8` out of 255 too. A picture that's only moved and resized gives that without going through
    /// floating point, as Skia's quick pipeline does.
    pub fn row32(&self, x: i32, y: i32, alpha8: u32, out: &mut [u32], scratch: &mut Vec<[f32; 4]>) {
        // A gradient at full strength: each pixel's place along it, and its packed color from the table, which is what packing
        // its floating point color would give.
        if alpha8 == 255
            && let Some(gradient) = self.gradient()
        {
            gradient.row32(x, y, out);
            return;
        }
        if let Self::Image { image, to_image, smooth, repeat, bounds, pattern } = self
            && to_image.b == 0.0
            && to_image.c == 0.0
        {
            image_row32(image, to_image, x, y, *smooth, *repeat, bounds, *pattern, out);
            if alpha8 < 255 {
                let by = alpha8 + (alpha8 >> 7);
                for px in out.iter_mut() {
                    *px = scale32(*px, by);
                }
            }
            return;
        }
        scratch.resize(out.len(), [0.0; 4]);
        self.row(x, y, &mut scratch[..out.len()]);
        let alpha = alpha8 as f32 / 255.0;
        for (px, s) in out.iter_mut().zip(scratch.iter()) {
            *px = pack([
                (s[0] * alpha + 0.5) as u32,
                (s[1] * alpha + 0.5) as u32,
                (s[2] * alpha + 0.5) as u32,
                (s[3] * alpha + 0.5) as u32,
            ]);
        }
    }
}

const LANES: u32 = 0x00FF_00FF;

/// A packed pixel times `by` out of 256, two parts at a time, rounded.
#[inline(always)]
pub fn scale32(p: u32, by: u32) -> u32 {
    let rb = (((p & LANES) * by + 0x0080_0080) >> 8) & LANES;
    let ag = (((p >> 8) & LANES) * by + 0x0080_0080) & !LANES;
    rb | ag
}

/// Between two packed pixels, `by` out of 256 of the way to the second.
#[inline(always)]
pub fn lerp32(a: u32, b: u32, by: u32) -> u32 {
    let rest = 256 - by;
    let rb = (((a & LANES) * rest + (b & LANES) * by + 0x0080_0080) >> 8) & LANES;
    let ag = (((a >> 8) & LANES) * rest + ((b >> 8) & LANES) * by + 0x0080_0080) & !LANES;
    rb | ag
}

/// Four parts from 0 to 255 as one packed pixel, red first.
#[inline(always)]
pub fn pack(c: [u32; 4]) -> u32 {
    c[0].min(255) | c[1].min(255) << 8 | c[2].min(255) << 16 | c[3].min(255) << 24
}

/// The four parts of a packed pixel.
#[inline(always)]
pub fn unpack(p: u32) -> [u32; 4] {
    [p & 255, (p >> 8) & 255, (p >> 16) & 255, p >> 24]
}

/// Lays a row of packed pixels on the canvas through how much of each pixel they cover, as `source-over` does, a pixel at a
/// time: what `pixels::blend_solid` and `pixels::blend_shaded` are checked against.
#[cfg(test)]
#[inline]
pub fn blend_row(out: &mut [u8], cover: &[f32], source: impl Fn(usize) -> u32) {
    for (i, (px, &k)) in out.chunks_exact_mut(4).zip(cover).enumerate() {
        // How much is covered is from 0 to 1, so this is a plain conversion, without the checks `as` adds.
        let scaled = k.max(0.0).min(1.0) * 256.0 + 0.5;
        // SAFETY: `max` leaves no NaN, so `scaled` is between 0.5 and 256.5, which an `i32` holds.
        let c = unsafe { scaled.to_int_unchecked::<i32>() } as u32;
        if c == 0 {
            continue;
        }
        let s = source(i);
        let s = if c >= 256 { s } else { scale32(s, c) };
        let sa = s >> 24;
        if sa == 255 {
            px.copy_from_slice(&s.to_le_bytes());
            continue;
        }
        if s == 0 {
            continue;
        }
        let d = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
        let out = s + scale32(d, 256 - (sa + (sa >> 7)));
        px.copy_from_slice(&out.to_le_bytes());
    }
}

/// Lays one packed pixel on the canvas through `cover` out of 256 of it, as `source-over` does, which hairlines do from a table
/// of each coverage.
#[cfg(test)]
#[inline(always)]
pub fn blend_pixel(px: &mut [u8], color: u32, cover: u32) {
    let s = if cover >= 256 { color } else { scale32(color, cover) };
    let sa = s >> 24;
    if sa == 255 {
        px.copy_from_slice(&s.to_le_bytes());
    } else if s != 0 {
        let d = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
        px.copy_from_slice(&(s + scale32(d, 256 - (sa + (sa >> 7)))).to_le_bytes());
    }
}

/// `x / 255`, rounded, for `x` up to 255 * 255, as Skia works it out.
#[inline]
pub fn div255(x: u32) -> u32 {
    let x = x + 128;
    (x + (x >> 8)) >> 8
}

/// Lays `s` on `d`, both from 0 to 255 and multiplied by alpha, in whole numbers, for the ways of laying one on the other that
/// only weigh each by the other's alpha: what `composite` does in floating point, a few times quicker, as Skia's quick pipeline
/// does them. `None` for the rest, which `composite` does.
#[inline]
pub fn composite8(op: Op, s: [u32; 4], d: [u32; 4]) -> Option<[u32; 4]> {
    let (sa, da) = (s[3], d[3]);
    // s * fs + d * fd, with both weights out of 255. Neither sum goes past 255 * 255, as no color is more than its alpha.
    let pd = |fs: u32, fd: u32| [div255(s[0] * fs + d[0] * fd), div255(s[1] * fs + d[1] * fd), div255(s[2] * fs + d[2] * fd), div255(sa * fs + da * fd)];
    Some(match op {
        Op::DestinationOut => {
            let keep = 255 - sa;
            [div255(d[0] * keep), div255(d[1] * keep), div255(d[2] * keep), div255(da * keep)]
        }
        Op::SourceAtop => pd(da, 255 - sa),
        Op::DestinationOver => {
            let k = 255 - da;
            [div255(s[0] * k) + d[0], div255(s[1] * k) + d[1], div255(s[2] * k) + d[2], div255(sa * k) + da]
        }
        Op::Xor => pd(255 - da, 255 - sa),
        Op::Lighter => [(s[0] + d[0]).min(255), (s[1] + d[1]).min(255), (s[2] + d[2]).min(255), (sa + da).min(255)],
        _ => return None,
    })
}

/// Where along an axis to read for a sample at `at`, in a picture `size` across from `start`, as `wrap`, `decal` or clamping
/// says, or `None` for nothing.
#[inline]
fn texel_index(at: i32, start: i32, end: i32, wrap: bool, decal: bool) -> Option<usize> {
    if wrap {
        Some((start + (at - start).rem_euclid(end - start)) as usize)
    } else if decal && (at < start || at >= end) {
        None
    } else {
        Some(at.clamp(start, end - 1) as usize)
    }
}

/// A row of a picture that's only moved and resized, sampled between pixels in steps of 256, or at the nearest, packed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn image_row32(image: &Image, m: &Matrix, x: i32, y: i32, smooth: bool, repeat: Repeat, bounds: &[i32; 4], pattern: bool, out: &mut [u32]) {
    let [bx0, by0, bx1, by1] = *bounds;
    let (wrap_x, decal_x, wrap_y, decal_y) = match (pattern, repeat) {
        (false, _) => (false, false, false, false),
        (true, Repeat::Both) => (true, false, true, false),
        (true, Repeat::X) => (true, false, false, true),
        (true, Repeat::Y) => (false, true, true, false),
        (true, Repeat::Neither) => (false, true, false, true),
    };
    let width = image.width as usize;
    let pixels = &image.pixels;
    let texel = |row: Option<usize>, column: Option<usize>| -> u32 {
        match (row, column) {
            (Some(r), Some(c)) => {
                let at = (r * width + c) * 4;
                u32::from_le_bytes([pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]])
            }
            _ => 0,
        }
    };
    let v = m.d * (y as f32 + 0.5) + m.f;
    if !v.is_finite() {
        out.fill(0);
        return;
    }
    if !smooth {
        let row = texel_index(super::raster::floor(v - 1.0e-4) as i32, by0, by1, wrap_y, decal_y);
        // A picture drawn once, clamped to its edges: where each sample is, in steps of 1/2^32 of a pixel, so the row is read with
        // a shift and a clamp a pixel, and nothing drifts over even the widest row.
        if !pattern
            && let Some(r) = row
            && let ([], words, []) = unsafe { pixels.align_to::<u32>() }
        {
            let start = f64::from(m.a) * (f64::from(x) + 0.5) + f64::from(m.e) - 1.0e-4;
            let step = f64::from(m.a);
            if start.is_finite() && step.is_finite() && start.abs() + step.abs() * out.len() as f64 + 1.0 < 1.0e9 {
                let line = &words[r * width..r * width + width];
                let (first, last) = (i64::from(bx0.max(0)), i64::from(bx1.min(width as i32).max(1) - 1));
                let mut at = (start * 4_294_967_296.0).floor() as i64;
                let step = (step * 4_294_967_296.0).round() as i64;
                for px in out.iter_mut() {
                    *px = line[(at >> 32).clamp(first, last) as usize];
                    at += step;
                }
                return;
            }
        }
        for (i, px) in out.iter_mut().enumerate() {
            let u = m.a * ((x + i as i32) as f32 + 0.5) + m.e;
            *px = if u.is_finite() { texel(row, texel_index(super::raster::floor(u - 1.0e-4) as i32, bx0, bx1, wrap_x, decal_x)) } else { 0 };
        }
        return;
    }
    let fy = v - 0.5;
    let y0 = super::raster::floor(fy);
    let wy = ((fy - y0) * 256.0) as u32;
    let (row0, row1) = (texel_index(y0 as i32, by0, by1, wrap_y, decal_y), texel_index(y0 as i32 + 1, by0, by1, wrap_y, decal_y));
    // A picture drawn once, which is clamped to its edges: two rows of whole pixels, read straight from where they are.
    if !pattern
        && let (Some(r0), Some(r1)) = (row0, row1)
        && let ([], words, []) = unsafe { pixels.align_to::<u32>() }
    {
        let (top, bottom) = (&words[r0 * width..r0 * width + width], &words[r1 * width..r1 * width + width]);
        let (first, last) = (bx0.max(0), (bx1.min(width as i32)).max(1) - 1);
        // Where each sample is, in steps of 1/65536 of a pixel, which needs nothing rounded for each pixel.
        let start = m.a * (x as f32 + 0.5) + m.e - 0.5;
        if (start.abs() as f64 + (m.a.abs() as f64) * out.len() as f64) < 30000.0 {
            let mut at = (start * 65536.0) as i64;
            let step = (m.a * 65536.0) as i64;
            let mut i = 0;
            // Four at a time where both pixels sampled are inside the picture, which is all of it but its edges.
            #[cfg(target_arch = "x86_64")]
            if last > first {
                // SAFETY: each sample reads pixels `c` and `c + 1` of two rows that are `width` long, with `c + 1 <= last < width`.
                i = unsafe { super::pixels::bilinear_run(top, bottom, (first as i64, last as i64), &mut at, step, wy, out) };
            }
            for px in out[i..].iter_mut() {
                let x0 = (at >> 16) as i32;
                let wx = ((at >> 8) & 255) as u32;
                let c0 = x0.clamp(first, last) as usize;
                let c1 = (x0 + 1).clamp(first, last) as usize;
                let up = lerp32(top[c0], top[c1], wx);
                let down = lerp32(bottom[c0], bottom[c1], wx);
                *px = lerp32(up, down, wy);
                at += step;
            }
            return;
        }
        for (i, px) in out.iter_mut().enumerate() {
            let u = start + m.a * i as f32;
            let x0 = super::raster::floor(u);
            let wx = ((u - x0) * 256.0) as u32;
            let x0 = x0 as i32;
            let c0 = x0.clamp(first, last) as usize;
            let c1 = (x0 + 1).clamp(first, last) as usize;
            let up = lerp32(top[c0], top[c1], wx);
            let down = lerp32(bottom[c0], bottom[c1], wx);
            *px = lerp32(up, down, wy);
        }
        return;
    }
    for (i, px) in out.iter_mut().enumerate() {
        let u = m.a * ((x + i as i32) as f32 + 0.5) + m.e - 0.5;
        if !u.is_finite() {
            *px = 0;
            continue;
        }
        let x0 = super::raster::floor(u);
        let wx = ((u - x0) * 256.0) as u32;
        let (c0, c1) = (texel_index(x0 as i32, bx0, bx1, wrap_x, decal_x), texel_index(x0 as i32 + 1, bx0, bx1, wrap_x, decal_x));
        let top = lerp32(texel(row0, c0), texel(row0, c1), wx);
        let bottom = lerp32(texel(row1, c0), texel(row1, c1), wx);
        *px = lerp32(top, bottom, wy);
    }
}

#[inline]
fn look_up(table: &[[f32; 4]], t: f32) -> [f32; 4] {
    table[step_of(t)]
}

/// Which step of a gradient's table `t` along it is.
#[inline(always)]
fn step_of(t: f32) -> usize {
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    ((t * STEPS as f32) as usize).min(STEPS - 1)
}

/// How far along a radial gradient (`[x0, y0, r0, x1, y1, r1]`) each pixel of a row from `x, y` is, `n` of them, as the canvas
/// spec has it: the bigger of the two circles through it whose radius isn't negative, or `None` where none is. Each pixel's place
/// is stepped to from the last, and what's the same along the row is worked out once, which makes it a few times quicker than
/// each pixel worked out alone; `Shader::row` and `Shader::row32` both use this, so they find the same places.
#[inline]
fn radial_row(kind: &[f32; 6], to_user: &Matrix, x: i32, y: i32, n: usize, mut each: impl FnMut(usize, Option<f32>)) {
    let [x0, y0, r0, x1, y1, r1] = *kind;
    let (cdx, cdy, dr) = (x1 - x0, y1 - y0, r1 - r0);
    let a = cdx * cdx + cdy * cdy - dr * dr;
    let (mut ux, mut uy) = to_user.apply(x as f32 + 0.5, y as f32 + 0.5);
    let (step_x, step_y) = (to_user.a, to_user.b);
    let (r0r0, r0dr) = (r0 * r0, r0 * dr);
    if cdx == 0.0 && cdy == 0.0 && dr != 0.0 {
        // Circles about the same center, as a glow's are: the circle through a point is as far from the center as it is, so
        // it's (distance - r0) / (r1 - r0) along, which is what the general way below comes to with one root and no choosing.
        let inv_dr = 1.0 / dr;
        for i in 0..n {
            let (pdx, pdy) = (ux - x0, uy - y0);
            each(i, Some(((pdx * pdx + pdy * pdy).sqrt() - r0) * inv_dr));
            ux += step_x;
            uy += step_y;
        }
        return;
    }
    if a.abs() < 1e-9 {
        // One circle's edge goes through the other's center: one root.
        for i in 0..n {
            let (pdx, pdy) = (ux - x0, uy - y0);
            let b = pdx * cdx + pdy * cdy + r0dr;
            let c = pdx * pdx + pdy * pdy - r0r0;
            let mut t = None;
            if b != 0.0 {
                let w = c / (2.0 * b);
                if r0 + w * dr >= 0.0 {
                    t = Some(w);
                }
            }
            each(i, t);
            ux += step_x;
            uy += step_y;
        }
        return;
    }
    let inv_a = 1.0 / a;
    for i in 0..n {
        let (pdx, pdy) = (ux - x0, uy - y0);
        let b = pdx * cdx + pdy * cdy + r0dr;
        let c = pdx * pdx + pdy * pdy - r0r0;
        let disc = b * b - a * c;
        let mut t = None;
        if disc >= 0.0 {
            let root = disc.sqrt();
            let (w1, w2) = ((b + root) * inv_a, (b - root) * inv_a);
            let (big, small) = if w1 > w2 { (w1, w2) } else { (w2, w1) };
            if r0 + big * dr >= 0.0 {
                t = Some(big);
            } else if r0 + small * dr >= 0.0 {
                t = Some(small);
            }
        }
        each(i, t);
        ux += step_x;
        uy += step_y;
    }
}

/// What a picture has at a place in its own pixels, from 0 to 255.
#[inline]
fn sample(image: &Image, u: f32, v: f32, smooth: bool, repeat: Repeat, bounds: &[i32; 4], pattern: bool) -> [f32; 4] {
    let [bx0, by0, bx1, by1] = *bounds;
    if !(u.is_finite() && v.is_finite()) {
        return [0.0; 4];
    }
    // `drawImage()` clamps to the part it draws; a pattern repeats, or has nothing past its picture the way it doesn't.
    let (wrap_x, decal_x, wrap_y, decal_y) = match repeat {
        Repeat::Both => (true, false, true, false),
        Repeat::X => (true, false, false, true),
        Repeat::Y => (false, true, true, false),
        Repeat::Neither => (false, true, false, true),
    };
    let clamped = !pattern;
    let (decal_x, decal_y) = if clamped { (false, false) } else { (decal_x, decal_y) };
    let fetch = |x: i32, y: i32| -> [f32; 4] {
        let x = if wrap_x && !clamped {
            bx0 + (x - bx0).rem_euclid(bx1 - bx0)
        } else if decal_x && (x < bx0 || x >= bx1) {
            return [0.0; 4];
        } else {
            x.clamp(bx0, bx1 - 1)
        };
        let y = if wrap_y && !clamped {
            by0 + (y - by0).rem_euclid(by1 - by0)
        } else if decal_y && (y < by0 || y >= by1) {
            return [0.0; 4];
        } else {
            y.clamp(by0, by1 - 1)
        };
        image.texel(x, y)
    };
    if !smooth {
        // A sample exactly between two pixels takes the first, as Skia's does.
        return fetch(super::raster::floor(u - 1.0e-4) as i32, super::raster::floor(v - 1.0e-4) as i32);
    }
    let (x, y) = (u - 0.5, v - 0.5);
    let (fx0, fy0) = (super::raster::floor(x), super::raster::floor(y));
    let (tx, ty) = (x - fx0, y - fy0);
    let (x0, y0) = (fx0 as i32, fy0 as i32);
    let a = fetch(x0, y0);
    let b = fetch(x0 + 1, y0);
    let c = fetch(x0, y0 + 1);
    let d = fetch(x0 + 1, y0 + 1);
    let mut out = [0.0; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * tx;
        let bottom = c[i] + (d[i] - c[i]) * tx;
        out[i] = top + (bottom - top) * ty;
    }
    out
}

/// How what's drawn is laid on what's there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    SourceOver,
    SourceIn,
    SourceOut,
    SourceAtop,
    DestinationOver,
    DestinationIn,
    DestinationOut,
    DestinationAtop,
    Lighter,
    Copy,
    Xor,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    /// What `clearRect()` does.
    Clear,
}

impl Op {
    pub fn from_index(index: u32) -> Self {
        use Op::*;
        const ALL: [Op; 26] = [
            SourceOver, SourceIn, SourceOut, SourceAtop, DestinationOver, DestinationIn, DestinationOut, DestinationAtop, Lighter, Copy,
            Xor, Multiply, Screen, Overlay, Darken, Lighten, ColorDodge, ColorBurn, HardLight, SoftLight, Difference, Exclusion, Hue,
            Saturation, Color, Luminosity,
        ];
        ALL.get(index as usize).copied().unwrap_or(SourceOver)
    }

    /// Whether it changes what's there even where nothing's drawn, so that the whole of what's clipped has to be gone over.
    pub fn unbounded(self) -> bool {
        matches!(self, Op::SourceIn | Op::SourceOut | Op::DestinationIn | Op::DestinationAtop | Op::Copy)
    }
}

/// Lays `s` on `d`, both from 0 to 1 and multiplied by alpha.
#[inline]
pub fn composite(op: Op, s: [f32; 4], d: [f32; 4]) -> [f32; 4] {
    let (sa, da) = (s[3], d[3]);
    let each = |f: &dyn Fn(f32, f32) -> f32| [f(s[0], d[0]), f(s[1], d[1]), f(s[2], d[2])];
    let pd = |fs: f32, fd: f32| -> [f32; 4] {
        [s[0] * fs + d[0] * fd, s[1] * fs + d[1] * fd, s[2] * fs + d[2] * fd, sa * fs + da * fd]
    };
    match op {
        Op::SourceOver => pd(1.0, 1.0 - sa),
        Op::SourceIn => pd(da, 0.0),
        Op::SourceOut => pd(1.0 - da, 0.0),
        Op::SourceAtop => pd(da, 1.0 - sa),
        Op::DestinationOver => pd(1.0 - da, 1.0),
        Op::DestinationIn => pd(0.0, sa),
        Op::DestinationOut => pd(0.0, 1.0 - sa),
        Op::DestinationAtop => pd(1.0 - da, sa),
        Op::Lighter => [(s[0] + d[0]).min(1.0), (s[1] + d[1]).min(1.0), (s[2] + d[2]).min(1.0), (sa + da).min(1.0)],
        Op::Copy => s,
        Op::Xor => pd(1.0 - da, 1.0 - sa),
        Op::Clear => [0.0; 4],
        Op::Multiply => {
            let c = each(&|sc, dc| sc * (1.0 - da) + dc * (1.0 - sa) + sc * dc);
            [c[0], c[1], c[2], sa + da - sa * da]
        }
        Op::Screen => {
            let c = each(&|sc, dc| sc + dc - sc * dc);
            [c[0], c[1], c[2], sa + da - sa * da]
        }
        _ => {
            // The rest blend what's there and what's drawn as colors without alpha, and then put alpha back.
            let un = |c: f32, a: f32| if a > 0.0 { (c / a).min(1.0) } else { 0.0 };
            let cs = [un(s[0], sa), un(s[1], sa), un(s[2], sa)];
            let cb = [un(d[0], da), un(d[1], da), un(d[2], da)];
            let blended = blend(op, cb, cs);
            let a = sa + da - sa * da;
            let mut out = [0.0; 4];
            for i in 0..3 {
                out[i] = s[i] * (1.0 - da) + d[i] * (1.0 - sa) + sa * da * blended[i];
            }
            out[3] = a;
            out
        }
    }
}

fn blend(op: Op, cb: [f32; 3], cs: [f32; 3]) -> [f32; 3] {
    let separable = |f: fn(f32, f32) -> f32| [f(cb[0], cs[0]), f(cb[1], cs[1]), f(cb[2], cs[2])];
    fn hard_light(b: f32, s: f32) -> f32 {
        if s <= 0.5 { b * 2.0 * s } else { let t = 2.0 * s - 1.0; b + t - b * t }
    }
    match op {
        Op::Overlay => separable(|b, s| hard_light(s, b)),
        Op::Darken => separable(|b, s| b.min(s)),
        Op::Lighten => separable(|b, s| b.max(s)),
        Op::ColorDodge => separable(|b, s| {
            if b == 0.0 { 0.0 } else if s >= 1.0 { 1.0 } else { (b / (1.0 - s)).min(1.0) }
        }),
        Op::ColorBurn => separable(|b, s| {
            if b >= 1.0 { 1.0 } else if s <= 0.0 { 0.0 } else { 1.0 - ((1.0 - b) / s).min(1.0) }
        }),
        Op::HardLight => separable(hard_light),
        Op::SoftLight => separable(|b, s| {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }),
        Op::Difference => separable(|b, s| (b - s).abs()),
        Op::Exclusion => separable(|b, s| b + s - 2.0 * b * s),
        Op::Hue => set_lum(set_sat(cs, sat(cb)), lum(cb)),
        Op::Saturation => set_lum(set_sat(cb, sat(cs)), lum(cb)),
        Op::Color => set_lum(cs, lum(cb)),
        Op::Luminosity => set_lum(cb, lum(cs)),
        _ => cs,
    }
}

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 {
        for v in &mut out {
            *v = l + (*v - l) * l / (l - n);
        }
    }
    if x > 1.0 {
        for v in &mut out {
            *v = l + (*v - l) * (1.0 - l) / (x - l);
        }
    }
    out
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    if max <= min {
        return [0.0; 3];
    }
    let mut out = [0.0; 3];
    for i in 0..3 {
        out[i] = if c[i] == max {
            s
        } else if c[i] == min {
            0.0
        } else {
            (c[i] - min) * s / (max - min)
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_over_covers_what_is_under() {
        let out = composite(Op::SourceOver, [0.5, 0.0, 0.0, 0.5], [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(out, [0.5, 0.0, 0.5, 1.0]);
    }

    #[test]
    fn gradients_mix_unmultiplied_colors() {
        let mut g = Gradient::new(GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 1.0, y1: 0.0 });
        g.add_stop(0.0, [255, 0, 0, 255]);
        g.add_stop(1.0, [0, 0, 255, 0]);
        let mid = g.color_at(0.5);
        assert!((mid[0] - 0.25).abs() < 1e-5 && (mid[3] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn stops_at_the_same_place_make_a_sharp_change() {
        let mut g = Gradient::new(GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 1.0, y1: 0.0 });
        g.add_stop(0.0, [255, 0, 0, 255]);
        g.add_stop(0.5, [255, 0, 0, 255]);
        g.add_stop(0.5, [0, 255, 0, 255]);
        g.add_stop(1.0, [0, 255, 0, 255]);
        assert_eq!(g.color_at(0.49)[0], 1.0);
        assert_eq!(g.color_at(0.51)[1], 1.0);
    }

    #[test]
    fn multiply_darkens() {
        let out = composite(Op::Multiply, [0.5, 0.5, 0.5, 1.0], [1.0, 0.5, 0.0, 1.0]);
        assert!((out[0] - 0.5).abs() < 1e-6 && (out[1] - 0.25).abs() < 1e-6 && out[2].abs() < 1e-6);
    }

    /// Sampling at the nearest pixel a whole row at once, in fixed point, reads the same pixels as working out each sample's
    /// place in floating point does, at many sizes and offsets.
    #[test]
    fn nearest_samples_in_fixed_point_are_the_same_as_one_at_a_time() {
        let (w, h) = (37usize, 5usize);
        let pixels: Vec<u8> = (0..w * h * 4).map(|i| (i * 7 % 251) as u8).collect();
        assert_eq!(pixels.as_ptr() as usize % 4, 0, "the quick way is only taken for pixels on a 4-byte boundary");
        let image = Image { width: w as u32, height: h as u32, pixels };
        let bounds = [0, 0, w as i32, h as i32];
        for scale in [0.1f32, 0.25, 1.0 / 6.0, 0.5, 1.0, 1.5, 3.0, 7.25] {
            for offset in [-3.0f32, 0.0, 0.3, 2.0, 11.7] {
                let m = Matrix::new([scale, 0.0, 0.0, scale, offset, 0.0]);
                for x in [-5, 0, 3, 40] {
                    let mut fast = vec![0u32; 300];
                    image_row32(&image, &m, x, 2, false, Repeat::Neither, &bounds, false, &mut fast);
                    let v = m.d * (2.0 + 0.5) + m.f;
                    let row = (super::super::raster::floor(v - 1.0e-4) as i32).clamp(0, h as i32 - 1) as usize;
                    for (i, got) in fast.iter().enumerate() {
                        let u = m.a * ((x + i as i32) as f32 + 0.5) + m.e;
                        let column = (super::super::raster::floor(u - 1.0e-4) as i32).clamp(0, w as i32 - 1) as usize;
                        let at = (row * w + column) * 4;
                        let want = u32::from_le_bytes([image.pixels[at], image.pixels[at + 1], image.pixels[at + 2], image.pixels[at + 3]]);
                        assert_eq!(*got, want, "scale {scale} offset {offset} x {x} pixel {i}");
                    }
                }
            }
        }
    }

    /// A row of a picture drawn bigger, from two of its rows sampled across and then between them, as `draw_image_rows` makes
    /// it, is the same as sampling it whole, as its shader does.
    #[test]
    fn rows_sampled_across_then_between_are_the_same_as_sampled_whole() {
        let (w, h) = (23usize, 9usize);
        let pixels: Vec<u8> = (0..w * h * 4).map(|i| (i * 13 % 253) as u8).collect();
        let image = Image { width: w as u32, height: h as u32, pixels };
        let bounds = [0, 0, w as i32, h as i32];
        for (sx, sy) in [(0.1f32, 0.15f32), (1.0 / 6.0, 1.0 / 6.0), (0.5, 0.25), (1.0, 1.0), (2.0, 0.7)] {
            for (ex, fy) in [(0.0f32, 0.0f32), (-1.3, 0.4), (2.5, -0.75)] {
                let m = Matrix::new([sx, 0.0, 0.0, sy, ex, fy]);
                for y in -3..60 {
                    let len = 90;
                    let mut whole = vec![0u32; len];
                    image_row32(&image, &m, -4, y, true, Repeat::Neither, &bounds, false, &mut whole);
                    let v = m.d * (y as f32 + 0.5) + m.f;
                    let fy0 = v - 0.5;
                    let top = super::super::raster::floor(fy0);
                    let wy = ((fy0 - top) * 256.0) as u32;
                    let row = |r: i32| {
                        let r = r.clamp(0, h as i32 - 1);
                        let mut out = vec![0u32; len];
                        image_row32(&image, &Matrix::new([m.a, 0.0, 0.0, 0.0, m.e, r as f32 + 0.5]), -4, 0, true, Repeat::Neither, &bounds, false, &mut out);
                        out
                    };
                    let (upper, lower) = (row(top as i32), row(top as i32 + 1));
                    let mut got = vec![0u32; len];
                    super::super::pixels::lerp_rows(&mut got, &upper, &lower, wy);
                    assert_eq!(got, whole, "scale {sx},{sy} offset {ex},{fy} row {y}");
                }
            }
        }
    }

    /// A gradient's row packed from its table is what packing its floating point row gives, for linear and radial ones of
    /// every shape, turned or not.
    #[test]
    fn gradient_rows_packed_from_the_table_are_the_same_as_packed_floats() {
        let kinds = [
            GradientKind::Linear { x0: 3.0, y0: 2.0, x1: 90.0, y1: 40.0 },
            GradientKind::Linear { x0: 10.0, y0: 10.0, x1: 10.0, y1: 10.0 },
            GradientKind::Radial { x0: 40.0, y0: 30.0, r0: 0.0, x1: 40.0, y1: 30.0, r1: 35.0 },
            GradientKind::Radial { x0: 20.0, y0: 25.0, r0: 5.0, x1: 60.0, y1: 30.0, r1: 30.0 },
            GradientKind::Radial { x0: 50.0, y0: 30.0, r0: 40.0, x1: 52.0, y1: 31.0, r1: 2.0 },
        ];
        for kind in kinds {
            let mut g = Gradient::new(kind);
            g.add_stop(0.0, [255, 40, 0, 255]);
            g.add_stop(0.4, [20, 200, 90, 128]);
            g.add_stop(1.0, [0, 0, 255, 0]);
            let table = g.table();
            for to_user in [Matrix::IDENTITY, Matrix::new([0.8, 0.3, -0.2, 1.1, 4.0, -3.0])] {
                let shader = match kind {
                    GradientKind::Linear { x0, y0, x1, y1 } => {
                        let (dx, dy) = (x1 - x0, y1 - y0);
                        let len2 = dx * dx + dy * dy;
                        Shader::Linear { table: table.clone(), to_user, x0, y0, dx, dy, inv_len2: if len2 > 0.0 { 1.0 / len2 } else { 0.0 } }
                    }
                    GradientKind::Radial { x0, y0, r0, x1, y1, r1 } => Shader::Radial { table: table.clone(), to_user, kind: [x0, y0, r0, x1, y1, r1] },
                    GradientKind::Conic { .. } => unreachable!(),
                };
                let mut scratch = Vec::new();
                for y in [0, 7, 29, 55] {
                    let mut fast = vec![0u32; 100];
                    shader.row32(-5, y, 255, &mut fast, &mut scratch);
                    let mut floats = vec![[0.0f32; 4]; 100];
                    shader.row(-5, y, &mut floats);
                    let slow: Vec<u32> = floats.iter().map(|s| pack([(s[0] + 0.5) as u32, (s[1] + 0.5) as u32, (s[2] + 0.5) as u32, (s[3] + 0.5) as u32])).collect();
                    assert_eq!(fast, slow, "{kind:?} row {y}");
                }
            }
        }
    }
}
