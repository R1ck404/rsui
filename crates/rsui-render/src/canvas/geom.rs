//! Where a canvas's shapes go: its transforms, its paths as a page's code builds them, and strokes turned into what's filled.
//!
//! A path's points are kept where they land on the canvas, as the spec has them transformed when they're added. Curves and arcs
//! are made into lines as they're added, finely enough for how big they are on the canvas. A stroke is worked out where the
//! path was drawn, in the space the transform at the time of stroking makes, so that a line's width is transformed as a page's is.

use std::f32::consts::{PI, TAU};

/// How far the lines a curve becomes may stray from it, in canvas pixels. Skia's is a quarter of a pixel; this is finer, as
/// what's drawn here goes by the exact area each line covers.
const TOLERANCE: f32 = 0.1;

/// `x' = a x + c y + e`, `y' = b x + d y + f`, as the canvas has its transforms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Matrix {
    pub const IDENTITY: Self = Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub fn new(values: [f32; 6]) -> Self {
        let [a, b, c, d, e, f] = values;
        Self { a, b, c, d, e, f }
    }

    #[inline]
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    /// What `other` does, then this.
    pub fn then(&self, other: &Self) -> Self {
        // self ∘ other: apply other first.
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    pub fn invert(&self) -> Option<Self> {
        let det = f64::from(self.a) * f64::from(self.d) - f64::from(self.b) * f64::from(self.c);
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let (a, b, c, d, e, f) = (
            f64::from(self.a),
            f64::from(self.b),
            f64::from(self.c),
            f64::from(self.d),
            f64::from(self.e),
            f64::from(self.f),
        );
        Some(Self {
            a: (d / det) as f32,
            b: (-b / det) as f32,
            c: (-c / det) as f32,
            d: (a / det) as f32,
            e: ((c * f - d * e) / det) as f32,
            f: ((b * e - a * f) / det) as f32,
        })
    }

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// Whether it only moves and resizes, keeping lines across and down as they were.
    pub fn is_axis_aligned(&self) -> bool {
        self.b == 0.0 && self.c == 0.0
    }

    pub fn is_finite(&self) -> bool {
        [self.a, self.b, self.c, self.d, self.e, self.f].iter().all(|v| v.is_finite())
    }

    /// About how much bigger it makes things, which is how finely a curve needs to be followed.
    pub fn scale(&self) -> f32 {
        let x = (self.a * self.a + self.b * self.b).sqrt();
        let y = (self.c * self.c + self.d * self.d).sqrt();
        let most = x.max(y);
        if most.is_finite() && most > 0.0 { most } else { 1.0 }
    }
}

/// One part of a path that's a line from point to point, which may come back to where it started.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Subpath {
    pub start: u32,
    pub end: u32,
    pub closed: bool,
}

/// The points between an arc's ends, which sit `out` times as far from its center as the arc does, so that filling the lines
/// between them covers as much as the arc.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArcPoints {
    pub start: u32,
    pub end: u32,
    pub center: (f32, f32),
    pub out: f32,
}

/// A path, as lines from point to point on the canvas.
#[derive(Debug, Clone, Default)]
pub struct Path {
    pub points: Vec<(f32, f32)>,
    pub subpaths: Vec<Subpath>,
    /// Where arcs' points are, for what draws them on the arc instead.
    pub arcs: Vec<ArcPoints>,
    /// Whether the last subpath is still being added to, which `moveTo` or `closePath` ends.
    open: bool,
}

impl Path {
    pub fn clear(&mut self) {
        self.points.clear();
        self.subpaths.clear();
        self.arcs.clear();
        self.open = false;
    }

    /// Its points, with those between each arc's ends put back on the arc, as a stroke a pixel wide is drawn through them.
    pub fn points_on_arcs(&self, out: &mut Vec<(f32, f32)>) {
        out.clear();
        out.extend_from_slice(&self.points);
        for arc in &self.arcs {
            let (cx, cy) = arc.center;
            for point in out.iter_mut().take(arc.end as usize).skip(arc.start as usize) {
                *point = (cx + (point.0 - cx) / arc.out, cy + (point.1 - cy) / arc.out);
            }
        }
    }

    pub fn move_to(&mut self, point: (f32, f32)) {
        if !(point.0.is_finite() && point.1.is_finite()) {
            return;
        }
        // A subpath that's only a point stays, as a stroke with round caps draws a dot for one with a line of no length.
        if self.open {
            if let Some(last) = self.subpaths.last()
                && last.end - last.start == 1
            {
                // Only a moveTo: it's replaced.
                self.points.truncate(last.start as usize);
                self.subpaths.pop();
            }
        }
        let at = self.points.len() as u32;
        self.points.push(point);
        self.subpaths.push(Subpath { start: at, end: at + 1, closed: false });
        self.open = true;
    }

    /// A line to here, starting a subpath here if there isn't one, as `lineTo` does.
    pub fn line_to(&mut self, point: (f32, f32)) {
        if !(point.0.is_finite() && point.1.is_finite()) {
            return;
        }
        if !self.open {
            // After closePath, a new subpath starts where the last one did.
            match self.subpaths.last() {
                Some(last) if last.closed => {
                    let start = self.points[last.start as usize];
                    self.move_to(start);
                }
                _ => {
                    self.move_to(point);
                    // A lineTo with nothing before it is a moveTo there, and nothing more.
                    return;
                }
            }
        }
        self.points.push(point);
        if let Some(last) = self.subpaths.last_mut() {
            last.end = self.points.len() as u32;
        }
    }

    /// Makes sure there's a subpath, starting at `point` if there wasn't, as curves need.
    fn ensure(&mut self, point: (f32, f32)) {
        if !self.open {
            match self.subpaths.last() {
                Some(last) if last.closed => {
                    let start = self.points[last.start as usize];
                    self.move_to(start);
                }
                _ => self.move_to(point),
            }
        }
    }

    pub fn close(&mut self) {
        if self.open {
            if let Some(last) = self.subpaths.last_mut() {
                last.closed = true;
            }
            self.open = false;
        }
    }

    /// Adds a quadratic curve, in user space, through `m`.
    pub fn quad_to(&mut self, m: &Matrix, control: (f32, f32), to: (f32, f32)) {
        let first = m.apply(control.0, control.1);
        self.ensure(first);
        let from = self.points[self.points.len() - 1];
        let control = first;
        let to = m.apply(to.0, to.1);
        if !(control.0.is_finite() && control.1.is_finite() && to.0.is_finite() && to.1.is_finite()) {
            return;
        }
        let bend_x = from.0 - 2.0 * control.0 + to.0;
        let bend_y = from.1 - 2.0 * control.1 + to.1;
        let bend = (bend_x * bend_x + bend_y * bend_y).sqrt();
        let steps = ((bend / (8.0 * TOLERANCE)).sqrt().ceil() as u32).clamp(1, 1000);
        for step in 1..steps {
            let t = step as f32 / steps as f32;
            let u = 1.0 - t;
            self.line_to((
                u * u * from.0 + 2.0 * u * t * control.0 + t * t * to.0,
                u * u * from.1 + 2.0 * u * t * control.1 + t * t * to.1,
            ));
        }
        self.line_to(to);
    }

    /// Adds a cubic curve, in user space, through `m`.
    pub fn cubic_to(&mut self, m: &Matrix, one: (f32, f32), two: (f32, f32), to: (f32, f32)) {
        let one = m.apply(one.0, one.1);
        self.ensure(one);
        let from = self.points[self.points.len() - 1];
        let two = m.apply(two.0, two.1);
        let to = m.apply(to.0, to.1);
        if ![one.0, one.1, two.0, two.1, to.0, to.1].iter().all(|v| v.is_finite()) {
            return;
        }
        cubic_points(from, one, two, to, |point| self.line_to(point));
    }

    /// Adds an ellipse's arc, in user space, through `m`, as `arc()` and `ellipse()` do: a line to its start, then it.
    #[allow(clippy::too_many_arguments)]
    pub fn ellipse(
        &mut self,
        m: &Matrix,
        center: (f32, f32),
        radii: (f32, f32),
        rotation: f32,
        start: f32,
        end: f32,
        anticlockwise: bool,
    ) {
        if ![center.0, center.1, radii.0, radii.1, rotation, start, end].iter().all(|v| v.is_finite()) {
            return;
        }
        let end = adjust_end_angle(start, end, anticlockwise);
        let sweep = end - start;
        let (sin_r, cos_r) = rotation.sin_cos();
        let point = |angle: f32| {
            let (sin, cos) = angle.sin_cos();
            let (x, y) = (radii.0 * cos, radii.1 * sin);
            m.apply(center.0 + x * cos_r - y * sin_r, center.1 + x * sin_r + y * cos_r)
        };
        let first = point(start);
        if self.open {
            self.line_to(first);
        } else {
            self.ensure(first);
            if self.points.last() != Some(&first) {
                self.line_to(first);
            }
        }
        let radius = radii.0.max(radii.1) * m.scale();
        let steps = arc_steps(radius, sweep.abs());
        // Points between the ends sit a little outside, so the lines cover as much as the arc does rather than less.
        let each = (sweep / steps as f32).abs();
        let out = if each > 1e-4 { (each / each.sin()).sqrt() } else { 1.0 };
        // Each point is the last turned by the same angle, which needs no sine or cosine of its own.
        let (step_sin, step_cos) = (sweep / steps as f32).sin_cos();
        let (mut sin, mut cos) = start.sin_cos();
        let first = self.points.len() as u32;
        for _ in 1..steps {
            (sin, cos) = (sin * step_cos + cos * step_sin, cos * step_cos - sin * step_sin);
            let (x, y) = (radii.0 * out * cos, radii.1 * out * sin);
            self.line_to(m.apply(center.0 + x * cos_r - y * sin_r, center.1 + x * sin_r + y * cos_r));
        }
        let end = self.points.len() as u32;
        if end > first && out != 1.0 {
            self.arcs.push(ArcPoints { start: first, end, center: m.apply(center.0, center.1), out });
        }
        self.line_to(point(start + sweep));
    }

    /// `arcTo()`: a line to where the arc starts, and the arc that turns from one line to the other, in user space.
    pub fn arc_to(&mut self, m: &Matrix, inverse: Option<&Matrix>, one: (f32, f32), two: (f32, f32), radius: f32) {
        if ![one.0, one.1, two.0, two.1, radius].iter().all(|v| v.is_finite()) {
            return;
        }
        let device_one = m.apply(one.0, one.1);
        if !self.open {
            self.ensure(device_one);
        }
        // Where the path is now, in user space.
        let Some(last) = self.points.last().copied() else {
            return;
        };
        let Some(inverse) = inverse else {
            return;
        };
        let p0 = inverse.apply(last.0, last.1);
        let (p1, p2) = (one, two);
        let same = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
        if same(p0, p1) || same(p1, p2) || radius == 0.0 {
            self.line_to(device_one);
            return;
        }
        let (v1x, v1y) = (p0.0 - p1.0, p0.1 - p1.1);
        let (v2x, v2y) = (p2.0 - p1.0, p2.1 - p1.1);
        let cross = v1x * v2y - v1y * v2x;
        let l1 = (v1x * v1x + v1y * v1y).sqrt();
        let l2 = (v2x * v2x + v2y * v2y).sqrt();
        // In a line: a line to the first point.
        if cross.abs() < 1e-6 * l1 * l2 {
            self.line_to(device_one);
            return;
        }
        let cos = ((v1x * v2x + v1y * v2y) / (l1 * l2)).clamp(-1.0, 1.0);
        let angle = cos.acos();
        let distance = radius / (angle / 2.0).tan();
        let t1 = (p1.0 + v1x / l1 * distance, p1.1 + v1y / l1 * distance);
        let t2 = (p1.0 + v2x / l2 * distance, p1.1 + v2y / l2 * distance);
        // The center is along the bisector, `radius` from both lines.
        let (nx, ny) = if cross > 0.0 { (v1y / l1, -v1x / l1) } else { (-v1y / l1, v1x / l1) };
        let center = (t1.0 + nx * radius, t1.1 + ny * radius);
        let a0 = (t1.1 - center.1).atan2(t1.0 - center.0);
        let a1 = (t2.1 - center.1).atan2(t2.0 - center.0);
        // The arc goes the short way round, which is clockwise when the turn is.
        let anticlockwise = cross > 0.0;
        self.ellipse(m, center, (radius, radius), 0.0, a0, a1, anticlockwise);
    }

    /// A rectangle, as its own closed subpath, as `rect()` adds it.
    pub fn rect(&mut self, m: &Matrix, x: f32, y: f32, w: f32, h: f32) {
        if ![x, y, w, h].iter().all(|v| v.is_finite()) {
            return;
        }
        self.move_to(m.apply(x, y));
        self.line_to(m.apply(x + w, y));
        self.line_to(m.apply(x + w, y + h));
        self.line_to(m.apply(x, y + h));
        self.close();
        // After a rect, the path is at its first corner, in a new subpath.
        self.move_to(m.apply(x, y));
    }

    /// `roundRect()`, with each corner's radii as `[x, y]`, from the top left round.
    pub fn round_rect(&mut self, m: &Matrix, x: f32, y: f32, w: f32, h: f32, radii: [[f32; 2]; 4]) {
        if ![x, y, w, h].iter().all(|v| v.is_finite()) {
            return;
        }
        // A rectangle drawn backwards keeps its corners where they're said to be, going the other way round.
        let (mut x, mut y, mut w, mut h) = (x, y, w, h);
        let mut r = radii;
        if w < 0.0 {
            x += w;
            w = -w;
            r = [r[1], r[0], r[3], r[2]];
        }
        if h < 0.0 {
            y += h;
            h = -h;
            r = [r[3], r[2], r[1], r[0]];
        }
        // Corners that'd overlap are made smaller together, as CSS's border-radius is.
        let mut scale = 1.0f32;
        for (sum, side) in [
            (r[0][0] + r[1][0], w),
            (r[3][0] + r[2][0], w),
            (r[0][1] + r[3][1], h),
            (r[1][1] + r[2][1], h),
        ] {
            if sum > side && sum > 0.0 {
                scale = scale.min(side / sum);
            }
        }
        for corner in &mut r {
            corner[0] *= scale;
            corner[1] *= scale;
        }
        let [tl, tr, br, bl] = r;
        self.move_to(m.apply(x + tl[0], y));
        self.line_to(m.apply(x + w - tr[0], y));
        self.corner(m, (x + w - tr[0], y + tr[1]), tr, -PI / 2.0);
        self.line_to(m.apply(x + w, y + h - br[1]));
        self.corner(m, (x + w - br[0], y + h - br[1]), br, 0.0);
        self.line_to(m.apply(x + bl[0], y + h));
        self.corner(m, (x + bl[0], y + h - bl[1]), bl, PI / 2.0);
        self.line_to(m.apply(x, y + tl[1]));
        self.corner(m, (x + tl[0], y + tl[1]), tl, PI);
        self.close();
        self.move_to(m.apply(x, y));
    }

    fn corner(&mut self, m: &Matrix, center: (f32, f32), radii: [f32; 2], from: f32) {
        if radii[0] <= 0.0 || radii[1] <= 0.0 {
            return;
        }
        let steps = arc_steps(radii[0].max(radii[1]) * m.scale(), PI / 2.0);
        for step in 1..=steps {
            let angle = from + PI / 2.0 * (step as f32 / steps as f32);
            let (sin, cos) = angle.sin_cos();
            self.line_to(m.apply(center.0 + radii[0] * cos, center.1 + radii[1] * sin));
        }
    }

    /// Each subpath's points, and whether it comes back to its start.
    pub fn each(&self) -> impl Iterator<Item = (&[(f32, f32)], bool)> {
        self.subpaths
            .iter()
            .map(|sub| (&self.points[sub.start as usize..sub.end as usize], sub.closed))
    }

    /// What filling it covers, as lines, each closed back to its start, as filling has every subpath.
    pub fn fill_lines(&self, lines: &mut Vec<[f32; 4]>) {
        for (points, _) in self.each() {
            if points.len() < 2 {
                continue;
            }
            for pair in points.windows(2) {
                lines.push([pair[0].0, pair[0].1, pair[1].0, pair[1].1]);
            }
            let (first, last) = (points[0], points[points.len() - 1]);
            if first != last {
                lines.push([last.0, last.1, first.0, first.1]);
            }
        }
    }
}

/// Points along a cubic curve, finely enough, after `from`.
pub fn cubic_points(from: (f32, f32), one: (f32, f32), two: (f32, f32), to: (f32, f32), mut add: impl FnMut((f32, f32))) {
    // How far it bends, which says how many lines it needs: the largest second difference.
    let dd = |a: f32, b: f32, c: f32| a - 2.0 * b + c;
    let bend = (dd(from.0, one.0, two.0).hypot(dd(from.1, one.1, two.1)))
        .max(dd(one.0, two.0, to.0).hypot(dd(one.1, two.1, to.1)));
    let steps = ((bend * 6.0 / (8.0 * TOLERANCE)).sqrt().ceil() as u32).clamp(1, 1000);
    for step in 1..steps {
        let t = step as f32 / steps as f32;
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        add((
            a * from.0 + b * one.0 + c * two.0 + d * to.0,
            a * from.1 + b * one.1 + c * two.1 + d * to.1,
        ));
    }
    add(to);
}

/// How many lines an arc of this radius, in canvas pixels, needs over this many radians.
pub fn arc_steps(radius: f32, sweep: f32) -> u32 {
    if !(radius > 0.0) || sweep == 0.0 {
        return 1;
    }
    // Each line's middle is this far inside the arc: r (1 - cos(θ/2)).
    let most = if radius > TOLERANCE {
        2.0 * (1.0 - TOLERANCE / radius).clamp(-1.0, 1.0).acos()
    } else {
        PI
    };
    ((sweep / most.max(1e-3)).ceil() as u32).clamp(1, 4096)
}

/// Where an arc ends, as Chromium works it out from its start, its end and which way it goes.
pub fn adjust_end_angle(start: f32, end: f32, anticlockwise: bool) -> f32 {
    if !anticlockwise && end - start >= TAU {
        start + TAU
    } else if anticlockwise && start - end >= TAU {
        start - TAU
    } else if !anticlockwise && start > end {
        start + (TAU - (start - end) % TAU)
    } else if anticlockwise && start < end {
        start - (TAU - (end - start) % TAU)
    } else {
        end
    }
}

/// How a stroke's ends look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    Butt,
    Round,
    Square,
}

/// How a stroke's corners look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Join {
    Miter,
    Round,
    Bevel,
}

/// What a stroke is drawn with.
#[derive(Debug, Clone, PartialEq)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: Cap,
    pub join: Join,
    pub miter_limit: f32,
    pub dash: Vec<f32>,
    pub dash_offset: f32,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self { width: 1.0, cap: Cap::Butt, join: Join::Miter, miter_limit: 10.0, dash: Vec::new(), dash_offset: 0.0 }
    }
}

/// Turns a path into what its stroke covers, as polygons filled with the nonzero rule, in canvas pixels. `m` is the transform
/// at the time of stroking, whose space the stroke's widths are in.
pub struct Stroker {
    user: Vec<(f32, f32)>,
    dashed: Vec<(Vec<(f32, f32)>, bool)>,
    left: Vec<(f32, f32)>,
    right: Vec<(f32, f32)>,
}

impl Default for Stroker {
    fn default() -> Self {
        Self { user: Vec::new(), dashed: Vec::new(), left: Vec::new(), right: Vec::new() }
    }
}

impl Stroker {
    pub fn stroke(&mut self, path: &Path, style: &StrokeStyle, m: &Matrix, lines: &mut Vec<[f32; 4]>) {
        if !(style.width > 0.0 && style.width.is_finite()) {
            return;
        }
        let Some(inverse) = m.invert() else {
            return;
        };
        let tolerance_scale = m.scale();
        let identity = m.is_identity();
        let dashes = usable_dashes(&style.dash);
        for (points, closed) in path.each() {
            // Back to where the stroke's widths are.
            self.user.clear();
            for &(x, y) in points {
                let point = if identity { (x, y) } else { inverse.apply(x, y) };
                if self.user.last() != Some(&point) {
                    self.user.push(point);
                }
            }
            if closed && self.user.len() > 1 && self.user.first() == self.user.last() {
                self.user.pop();
            }
            let user = std::mem::take(&mut self.user);
            if let Some(dashes) = &dashes {
                self.dashed.clear();
                dash(&user, closed, dashes, style.dash_offset, &mut self.dashed);
                let dashed = std::mem::take(&mut self.dashed);
                for (piece, piece_closed) in &dashed {
                    self.stroke_polyline(piece, *piece_closed, style, m, tolerance_scale, lines);
                }
                self.dashed = dashed;
            } else {
                self.stroke_polyline(&user, closed, style, m, tolerance_scale, lines);
            }
            self.user = user;
        }
    }

    fn stroke_polyline(
        &mut self,
        points: &[(f32, f32)],
        closed: bool,
        style: &StrokeStyle,
        m: &Matrix,
        scale: f32,
        lines: &mut Vec<[f32; 4]>,
    ) {
        let half = style.width / 2.0;
        if points.is_empty() {
            return;
        }
        if points.len() == 1 {
            // A line of no length: a dot with round caps, a square with square ones, nothing with butt.
            let (x, y) = points[0];
            match style.cap {
                Cap::Butt => {}
                Cap::Round => {
                    let steps = arc_steps(half * scale, TAU);
                    let circle: Vec<(f32, f32)> = (0..steps)
                        .map(|i| {
                            let a = TAU * i as f32 / steps as f32;
                            (x + half * a.cos(), y + half * a.sin())
                        })
                        .collect();
                    polygon(&circle, m, lines);
                }
                Cap::Square => {
                    polygon(&[(x - half, y - half), (x + half, y - half), (x + half, y + half), (x - half, y + half)], m, lines);
                }
            }
            return;
        }
        let closed = closed && points.len() > 2;
        let count = points.len();
        let segments = if closed { count } else { count - 1 };
        let normal = |i: usize| {
            let (a, b) = (points[i % count], points[(i + 1) % count]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let length = (dx * dx + dy * dy).sqrt();
            if length == 0.0 { (0.0, 0.0, 0.0, 0.0) } else { (dx / length, dy / length, -dy / length * half, dx / length * half) }
        };
        self.left.clear();
        self.right.clear();
        let (mut left, mut right) = (std::mem::take(&mut self.left), std::mem::take(&mut self.right));
        for i in 0..segments {
            let (_, _, nx, ny) = normal(i);
            let (a, b) = (points[i % count], points[(i + 1) % count]);
            if i > 0 || closed {
                let previous = if i == 0 { segments - 1 } else { i - 1 };
                // How long the shorter of the two segments is, which says whether their insides meet along them.
                let length = |at: usize| {
                    let (a, b) = (points[at % count], points[(at + 1) % count]);
                    (b.0 - a.0).hypot(b.1 - a.1)
                };
                let room = length(previous).min(length(i));
                join(&mut left, &mut right, points[i % count], normal(previous), normal(i), style, half, scale, room);
            } else {
                left.push((a.0 + nx, a.1 + ny));
                right.push((a.0 - nx, a.1 - ny));
            }
            left.push((b.0 + nx, b.1 + ny));
            right.push((b.0 - nx, b.1 - ny));
        }
        if closed {
            // The join at the start was put in front of the first segment, so each side goes round once.
            polygon(&left, m, lines);
            right.reverse();
            polygon(&right, m, lines);
        } else {
            let mut outline = std::mem::take(&mut left);
            let (dx, dy, nx, ny) = normal(segments - 1);
            cap(&mut outline, points[count - 1], (dx, dy), (nx, ny), style.cap, half, scale, true);
            outline.extend(right.iter().rev());
            let (dx, dy, nx, ny) = normal(0);
            cap(&mut outline, points[0], (-dx, -dy), (-nx, -ny), style.cap, half, scale, false);
            polygon(&outline, m, lines);
            left = outline;
        }
        self.left = left;
        self.right = right;
    }
}

/// Adds what goes round a corner at `at`, between the segment with `before`'s direction and normal, and `after`'s, to each side.
#[allow(clippy::too_many_arguments)]
fn join(
    left: &mut Vec<(f32, f32)>,
    right: &mut Vec<(f32, f32)>,
    at: (f32, f32),
    before: (f32, f32, f32, f32),
    after: (f32, f32, f32, f32),
    style: &StrokeStyle,
    half: f32,
    scale: f32,
    room: f32,
) {
    let (bdx, bdy, bnx, bny) = before;
    let (adx, ady, anx, any) = after;
    let cross = bdx * ady - bdy * adx;
    let dot = bdx * adx + bdy * ady;
    let left_start = (at.0 + bnx, at.1 + bny);
    let left_end = (at.0 + anx, at.1 + any);
    let right_start = (at.0 - bnx, at.1 - bny);
    let right_end = (at.0 - anx, at.1 - any);
    if cross.abs() < 1e-6 && dot > 0.0 {
        // Straight on.
        left.push(left_end);
        right.push(right_end);
        return;
    }
    // With y going down, a positive cross product turns clockwise on screen, which puts the normal's side inside.
    let left_outside = cross < 0.0;
    let (outer, outer_start, outer_end, sign) = if left_outside {
        (&mut *left, left_start, left_end, 1.0)
    } else {
        (&mut *right, right_start, right_end, -1.0)
    };
    outer.push(outer_start);
    match style.join {
        Join::Bevel => {}
        Join::Miter => {
            // 1 / sin(θ/2), where θ is the angle between the two segments.
            let cos_theta = -dot;
            let sin_half = ((1.0 - cos_theta) / 2.0).max(0.0).sqrt();
            if sin_half > 0.0 && 1.0 / sin_half <= style.miter_limit {
                // Where the two outer edges meet.
                let (mx, my) = (bnx + anx, bny + any);
                let length = (mx * mx + my * my).sqrt();
                if length > 0.0 {
                    let reach = half / sin_half;
                    outer.push((at.0 + sign * mx / length * reach, at.1 + sign * my / length * reach));
                }
            }
        }
        Join::Round => {
            let a0 = (outer_start.1 - at.1).atan2(outer_start.0 - at.0);
            let mut a1 = (outer_end.1 - at.1).atan2(outer_end.0 - at.0);
            // The short way round, on the outside.
            let mut sweep = a1 - a0;
            while sweep > PI {
                sweep -= TAU;
            }
            while sweep < -PI {
                sweep += TAU;
            }
            a1 = a0 + sweep;
            let steps = arc_steps(half * scale, sweep.abs());
            for step in 1..steps {
                let angle = a0 + (a1 - a0) * (step as f32 / steps as f32);
                outer.push((at.0 + half * angle.cos(), at.1 + half * angle.sin()));
            }
        }
    }
    outer.push(outer_end);
    let inner = if left_outside { right } else { left };
    // Where a corner is slight, as each of the many a curve is made of is, the insides of the two segments cross along both of
    // them, and that's the corner of the inside. Going through the corner itself there instead would go over what the segments
    // cover twice, right up to the inside edge, whose pixels would then be counted twice over: curves came out heavier than lines.
    let turn = dot.clamp(-1.0, 1.0).acos();
    if turn < PI / 2.0 && half * (turn / 2.0).tan() < room {
        let (mx, my) = (bnx + anx, bny + any);
        let length = (mx * mx + my * my).sqrt();
        if length > 0.0 {
            let reach = half / (turn / 2.0).cos();
            inner.push((at.0 - sign * mx / length * reach, at.1 - sign * my / length * reach));
            return;
        }
    }
    // Otherwise the inside goes through the corner itself, so nothing's left out where the sides cross.
    let (inner_start, inner_end) = if left_outside { (right_start, right_end) } else { (left_start, left_end) };
    inner.push(inner_start);
    inner.push(at);
    inner.push(inner_end);
}

/// Adds an end of a stroke at `at`, going in `direction`, from the left side to the right if `forward`.
#[allow(clippy::too_many_arguments)]
fn cap(
    outline: &mut Vec<(f32, f32)>,
    at: (f32, f32),
    direction: (f32, f32),
    normal: (f32, f32),
    kind: Cap,
    half: f32,
    scale: f32,
    _forward: bool,
) {
    let (dx, dy) = direction;
    let (nx, ny) = normal;
    match kind {
        Cap::Butt => {}
        Cap::Square => {
            outline.push((at.0 + nx + dx * half, at.1 + ny + dy * half));
            outline.push((at.0 - nx + dx * half, at.1 - ny + dy * half));
        }
        Cap::Round => {
            let a0 = ny.atan2(nx);
            let steps = arc_steps(half * scale, PI);
            // From the left side round the front to the right side.
            let turn = if dx * ny - dy * nx > 0.0 { -PI } else { PI };
            for step in 1..steps {
                let angle = a0 + turn * (step as f32 / steps as f32);
                outline.push((at.0 + half * angle.cos(), at.1 + half * angle.sin()));
            }
        }
    }
}

/// Adds a closed polygon in user space, through `m`, as lines.
fn polygon(points: &[(f32, f32)], m: &Matrix, lines: &mut Vec<[f32; 4]>) {
    if points.len() < 2 {
        return;
    }
    let identity = m.is_identity();
    let map = |p: (f32, f32)| if identity { p } else { m.apply(p.0, p.1) };
    let first = map(points[0]);
    let mut last = first;
    for &point in &points[1..] {
        let next = map(point);
        // A point again, as a join starts where the segment before it ended, makes a line that covers nothing.
        if next == last {
            continue;
        }
        lines.push([last.0, last.1, next.0, next.1]);
        last = next;
    }
    lines.push([last.0, last.1, first.0, first.1]);
}

/// The dashes to use, if there's a pattern that has any length, doubled when it's odd, as the spec has it.
fn usable_dashes(dash: &[f32]) -> Option<Vec<f32>> {
    if dash.is_empty() || dash.iter().any(|d| !(d.is_finite() && *d >= 0.0)) {
        return None;
    }
    let total: f32 = dash.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let mut out = dash.to_vec();
    if out.len() % 2 == 1 {
        out.extend_from_slice(dash);
    }
    Some(out)
}

/// Cuts a polyline into the pieces a dash pattern leaves of it.
fn dash(points: &[(f32, f32)], closed: bool, pattern: &[f32], offset: f32, out: &mut Vec<(Vec<(f32, f32)>, bool)>) {
    if points.is_empty() {
        return;
    }
    let total: f32 = pattern.iter().sum();
    let mut phase = offset % total;
    if phase < 0.0 {
        phase += total;
    }
    let mut index = 0;
    while phase >= pattern[index] {
        phase -= pattern[index];
        index = (index + 1) % pattern.len();
    }
    let mut left_in_dash = pattern[index] - phase;
    let mut on = index % 2 == 0;
    let mut current: Vec<(f32, f32)> = Vec::new();
    if on {
        current.push(points[0]);
    }
    let count = points.len();
    let segments = if closed { count } else { count - 1 };
    for i in 0..segments {
        let a = points[i];
        let b = points[(i + 1) % count];
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let length = (dx * dx + dy * dy).sqrt();
        let mut done = 0.0;
        while length - done > left_in_dash {
            done += left_in_dash;
            let t = done / length;
            let point = (a.0 + dx * t, a.1 + dy * t);
            if on {
                current.push(point);
                out.push((std::mem::take(&mut current), false));
            } else {
                current.push(point);
            }
            on = !on;
            index = (index + 1) % pattern.len();
            left_in_dash = pattern[index];
        }
        left_in_dash -= length - done;
        if on {
            current.push(b);
        }
    }
    if on && current.len() > 1 {
        out.push((current, false));
    }
}

/// Whether a point is inside what these lines go around, by the nonzero rule or the even-odd one.
pub fn contains(lines: &[[f32; 4]], x: f32, y: f32, even_odd: bool) -> bool {
    let mut winding = 0i32;
    for line in lines {
        let [x0, y0, x1, y1] = *line;
        if (y0 <= y) != (y1 <= y) {
            let t = (y - y0) / (y1 - y0);
            let cross_x = x0 + t * (x1 - x0);
            if cross_x > x {
                winding += if y1 > y0 { 1 } else { -1 };
            }
        }
    }
    if even_odd { winding % 2 != 0 } else { winding != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_compose_and_invert() {
        let m = Matrix::new([2.0, 0.0, 0.0, 3.0, 10.0, 20.0]);
        let r = Matrix::new([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);
        let both = m.then(&r);
        assert_eq!(both.apply(1.0, 0.0), (10.0, 23.0));
        let back = both.invert().unwrap();
        let (x, y) = back.apply(10.0, 23.0);
        assert!((x - 1.0).abs() < 1e-5 && y.abs() < 1e-5);
    }

    #[test]
    fn arcs_end_where_chromium_says() {
        assert_eq!(adjust_end_angle(0.0, 7.0, false), TAU);
        assert!((adjust_end_angle(1.0, 0.5, false) - (1.0 + TAU - 0.5)).abs() < 1e-5);
        assert!((adjust_end_angle(0.0, 1.0, true) - (0.0 - (TAU - 1.0))).abs() < 1e-5);
        assert_eq!(adjust_end_angle(0.0, 1.0, false), 1.0);
    }

    #[test]
    fn lines_with_nothing_before_them_start_a_subpath() {
        let mut path = Path::default();
        path.line_to((1.0, 1.0));
        path.line_to((5.0, 1.0));
        assert_eq!(path.points, vec![(1.0, 1.0), (5.0, 1.0)]);
        path.close();
        path.line_to((9.0, 9.0));
        assert_eq!(path.subpaths.len(), 2);
        assert_eq!(path.points[2], (1.0, 1.0));
    }

    #[test]
    fn a_stroke_covers_its_width() {
        let mut path = Path::default();
        path.move_to((10.0, 10.0));
        path.line_to((50.0, 10.0));
        let mut lines = Vec::new();
        let style = StrokeStyle { width: 4.0, ..StrokeStyle::default() };
        Stroker::default().stroke(&path, &style, &Matrix::IDENTITY, &mut lines);
        assert!(contains(&lines, 30.0, 11.5, false));
        assert!(!contains(&lines, 30.0, 12.5, false));
        assert!(!contains(&lines, 9.0, 10.0, false));
    }

    #[test]
    fn dashes_leave_gaps() {
        let mut out = Vec::new();
        dash(&[(0.0, 0.0), (10.0, 0.0)], false, &[2.0, 3.0], 0.0, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, vec![(0.0, 0.0), (2.0, 0.0)]);
        assert_eq!(out[1].0, vec![(5.0, 0.0), (7.0, 0.0)]);
    }

    #[test]
    fn miter_joins_reach_the_corner() {
        let mut path = Path::default();
        path.move_to((10.0, 10.0));
        path.line_to((50.0, 10.0));
        path.line_to((50.0, 50.0));
        let mut lines = Vec::new();
        let style = StrokeStyle { width: 10.0, ..StrokeStyle::default() };
        Stroker::default().stroke(&path, &style, &Matrix::IDENTITY, &mut lines);
        assert!(contains(&lines, 54.0, 6.0, false));
        let bevel = StrokeStyle { join: Join::Bevel, ..style };
        lines.clear();
        Stroker::default().stroke(&path, &bevel, &Matrix::IDENTITY, &mut lines);
        assert!(!contains(&lines, 54.5, 5.5, false));
    }
}
