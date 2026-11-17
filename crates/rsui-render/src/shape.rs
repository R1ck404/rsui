//! Drawing shapes: icons, charts and anything else made of lines and curves, written as SVG paths.
//!
//! They're turned into pixels once and kept on the GPU as a picture, so a shape that isn't changing costs what a
//! picture costs, however complicated it is.

use std::f32::consts::TAU;
use std::sync::LazyLock;

use rsui_core::{Element, PropValue, Symbol};
use rsui_text::{ColorBitmap, Rasterizer};

use crate::paint::Color;
use crate::transform::{IDENTITY, Matrix, apply_matrix, parse_transform};

/// How far a curve may stray from the lines that approximate it, in pixels.
const CURVE_TOLERANCE: f32 = 0.05;
/// The most lines any one curve becomes.
const MOST_STEPS: u32 = 256;
/// The largest shape to draw, in pixels on a side, so that a mistake can't ask for a huge picture.
const LARGEST: f32 = 4096.0;

static NAMES: LazyLock<(Symbol, Symbol)> =
    LazyLock::new(|| (Symbol::new("shape_layers"), Symbol::new("view_box")));

/// What makes an element a shape: its layers, from the bottom up, as written by [`shape_layer`].
pub(crate) fn layers_of(element: &Element) -> Option<&rsui_core::Text> {
    match element.props.get_symbol(NAMES.0) {
        Some(PropValue::Str(layers)) => Some(layers),
        _ => None,
    }
}

/// How the lines of a stroke meet at a corner, as SVG's `stroke-linejoin` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineJoin {
    /// Out to a point, unless that's more than four times as far as the line is wide, when it's cut off.
    Miter,
    /// Rounded.
    #[default]
    Round,
    /// Cut off straight.
    Bevel,
}

/// How a line ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineCap {
    /// Stopping right at the end.
    #[default]
    Butt,
    /// Half a circle past the end.
    Round,
    /// Half its width past the end.
    Square,
}

/// One layer of a shape, to go in its `shape_layers` prop, with each layer on a line of its own. `d` is an SVG path.
/// `fill` and `stroke` can be left out, and either can be `None` to use its `text_color` instead of a color of its own.
pub fn shape_layer(
    d: &str,
    fill: Option<Option<Color>>,
    stroke: Option<(Option<Color>, f32)>,
    cap: LineCap,
) -> String {
    let color = |color: Option<Color>| match color {
        Some(color) => format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            color.r, color.g, color.b, color.a
        ),
        None => "currentColor".to_owned(),
    };
    let cap = match cap {
        LineCap::Butt => "butt",
        LineCap::Round => "round",
        LineCap::Square => "square",
    };
    format!(
        "{}\t{}\t{}\t{cap}\t{d}",
        fill.map_or(String::new(), color),
        stroke.map_or(String::new(), |(stroke, _)| color(stroke)),
        stroke.map_or(0.0, |(_, width)| width),
    )
}

/// The part of a shape's own numbers that's being shown: left, top, width and height.
fn view_box(element: &Element) -> Option<[f32; 4]> {
    let Some(PropValue::Str(text)) = element.props.get_symbol(NAMES.1) else {
        return None;
    };
    let mut parts = text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(str::parse::<f32>);
    let mut next = || parts.next()?.ok();
    let view = [next()?, next()?, next()?, next()?];
    (view[2] > 0.0 && view[3] > 0.0).then_some(view)
}

/// How big a shape is if nothing says otherwise, which is how big its numbers go.
pub fn natural_shape_size(element: &Element) -> Option<(f32, f32)> {
    layers_of(element)?;
    let [_, _, width, height] = view_box(element)?;
    Some((width, height))
}

/// One flattened run of lines, in pixels.
#[derive(Debug, Clone, PartialEq, Default)]
struct Run {
    points: Vec<(f32, f32)>,
    closed: bool,
}

/// Reads the numbers and letters in a path.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn skip_separators(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b',')
        {
            self.at += 1;
        }
    }

    /// A number, if that's what's next. Those can run together: `1.5.5` is two, and so is `1-2`.
    fn number(&mut self) -> Option<f32> {
        self.skip_separators();
        let start = self.at;
        let mut at = start;
        let is =
            |at: usize, wanted: &[u8]| self.bytes.get(at).is_some_and(|byte| wanted.contains(byte));
        if is(at, b"+-") {
            at += 1;
        }
        let digits = |mut at: usize| {
            while self.bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
            at
        };
        let whole_end = digits(at);
        let mut end = whole_end;
        if is(end, b".") {
            end = digits(end + 1);
        }
        if end == at || (end == at + 1 && is(at, b".")) {
            return None;
        }
        if is(end, b"eE") {
            let mut power = end + 1;
            if is(power, b"+-") {
                power += 1;
            }
            let power_end = digits(power);
            if power_end > power {
                end = power_end;
            }
        }
        self.at = end;
        std::str::from_utf8(&self.bytes[start..end])
            .ok()?
            .parse()
            .ok()
    }

    /// An arc's flag, which is one digit, and needs nothing after it: `a1 1 0 00.5.5`.
    fn flag(&mut self) -> Option<bool> {
        self.skip_separators();
        let flag = match self.bytes.get(self.at)? {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.at += 1;
        Some(flag)
    }

    fn command(&mut self) -> Option<u8> {
        self.skip_separators();
        let byte = *self.bytes.get(self.at)?;
        byte.is_ascii_alphabetic().then(|| {
            self.at += 1;
            byte
        })
    }
}

/// From a shape's own numbers to pixels.
#[derive(Debug, Clone, Copy)]
struct Placement {
    /// What this layer says to do to itself before anything else.
    matrix: Matrix,
    /// What lines get wider and curves get more detail by, which is the same as the others unless it's been stretched.
    scale: f32,
    scale_x: f32,
    scale_y: f32,
    x: f32,
    y: f32,
}

impl Placement {
    /// As big as fits, in the middle, the way SVG does unless told otherwise.
    fn to_fit(view: [f32; 4], width: f32, height: f32) -> Self {
        let scale = (width / view[2]).min(height / view[3]);
        Self {
            scale,
            matrix: IDENTITY,
            scale_x: scale,
            scale_y: scale,
            x: (width - view[2] * scale) / 2.0 - view[0] * scale,
            y: (height - view[3] * scale) / 2.0 - view[1] * scale,
        }
    }

    /// Filling all of it, whatever that does to its shape. Lines stay as wide as they'd be if it were only as big as
    /// it is in the smaller direction, which is what a chart wants.
    fn to_fill(view: [f32; 4], width: f32, height: f32) -> Self {
        let (scale_x, scale_y) = (width / view[2], height / view[3]);
        Self {
            matrix: IDENTITY,
            scale: scale_x.min(scale_y),
            scale_x,
            scale_y,
            x: -view[0] * scale_x,
            y: -view[1] * scale_y,
        }
    }

    #[inline]
    fn pixel(self, (x, y): (f32, f32)) -> (f32, f32) {
        let (x, y) = apply_matrix(self.matrix, x, y);
        (x * self.scale_x + self.x, y * self.scale_y + self.y)
    }
}

/// What's built up while going along a path.
struct Tracer {
    place: Placement,
    runs: Vec<Run>,
    /// In the shape's own numbers, as curves are worked out from these.
    pen: (f32, f32),
    start: (f32, f32),
}

impl Tracer {
    fn move_to(&mut self, to: (f32, f32)) {
        self.pen = to;
        self.start = to;
        self.runs.push(Run {
            points: vec![self.place.pixel(to)],
            closed: false,
        });
    }

    fn add_point(&mut self, to: (f32, f32)) {
        // What follows a closed run starts from where that started, with no move to say so.
        if self.runs.last().is_none_or(|run| run.closed) {
            let from = self.pen;
            self.runs.push(Run {
                points: vec![self.place.pixel(from)],
                closed: false,
            });
        }
        let pixel = self.place.pixel(to);
        if let Some(run) = self.runs.last_mut() {
            run.points.push(pixel);
        }
    }

    fn line_to(&mut self, to: (f32, f32)) {
        self.add_point(to);
        self.pen = to;
    }

    fn cubic_to(&mut self, one: (f32, f32), two: (f32, f32), to: (f32, f32)) {
        let from = self.pen;
        // How far it bends sets how many lines it needs.
        let bend = |a: f32, b: f32, c: f32| (a - 2.0 * b + c).abs();
        let most = bend(from.0, one.0, two.0)
            .max(bend(from.1, one.1, two.1))
            .max(bend(one.0, two.0, to.0))
            .max(bend(one.1, two.1, to.1))
            * self.place.scale_x.max(self.place.scale_y);
        let steps = ((0.75 * most / CURVE_TOLERANCE).sqrt().ceil() as u32).clamp(1, MOST_STEPS);
        for step in 1..steps {
            let t = step as f32 / steps as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.add_point((
                a * from.0 + b * one.0 + c * two.0 + d * to.0,
                a * from.1 + b * one.1 + c * two.1 + d * to.1,
            ));
        }
        self.line_to(to);
    }

    fn quad_to(&mut self, control: (f32, f32), to: (f32, f32)) {
        // A quadratic curve is a cubic with its controls two thirds of the way to this one.
        let from = self.pen;
        let along = |a: f32, b: f32| a + (b - a) * 2.0 / 3.0;
        self.cubic_to(
            (along(from.0, control.0), along(from.1, control.1)),
            (along(to.0, control.0), along(to.1, control.1)),
            to,
        );
    }

    /// An arc, written the way SVG does, by its ends. This finds its middle, as the spec's notes say to.
    fn arc_to(&mut self, radii: (f32, f32), turn: f32, large: bool, sweep: bool, to: (f32, f32)) {
        let from = self.pen;
        let (mut rx, mut ry) = (radii.0.abs(), radii.1.abs());
        if rx == 0.0 || ry == 0.0 {
            self.line_to(to);
            return;
        }
        if from == to {
            return;
        }
        let (sin, cos) = turn.to_radians().sin_cos();
        let half_x = (from.0 - to.0) / 2.0;
        let half_y = (from.1 - to.1) / 2.0;
        let x1 = cos * half_x + sin * half_y;
        let y1 = -sin * half_x + cos * half_y;
        // Radii too small to reach are made just big enough.
        let reach = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
        if reach > 1.0 {
            let grow = reach.sqrt();
            rx *= grow;
            ry *= grow;
        }
        let (rx2, ry2) = (rx * rx, ry * ry);
        let (x12, y12) = (x1 * x1, y1 * y1);
        let top = rx2 * ry2 - rx2 * y12 - ry2 * x12;
        let bottom = rx2 * y12 + ry2 * x12;
        let mut root = if bottom > 0.0 {
            (top / bottom).max(0.0).sqrt()
        } else {
            0.0
        };
        if large == sweep {
            root = -root;
        }
        let cx1 = root * rx * y1 / ry;
        let cy1 = -root * ry * x1 / rx;
        let cx = cos * cx1 - sin * cy1 + (from.0 + to.0) / 2.0;
        let cy = sin * cx1 + cos * cy1 + (from.1 + to.1) / 2.0;
        let first = ((y1 - cy1) / ry).atan2((x1 - cx1) / rx);
        let last = ((-y1 - cy1) / ry).atan2((-x1 - cx1) / rx);
        let mut across = last - first;
        if sweep && across < 0.0 {
            across += TAU;
        } else if !sweep && across > 0.0 {
            across -= TAU;
        }
        let radius = rx.max(ry) * self.place.scale_x.max(self.place.scale_y);
        // The further a line can be from the arc, the fewer it takes.
        let each = 2.0 * (1.0 - (CURVE_TOLERANCE / radius.max(CURVE_TOLERANCE)).min(1.0)).acos();
        let steps = ((across.abs() / each.max(0.001)).ceil() as u32).clamp(1, MOST_STEPS);
        for step in 1..steps {
            let angle = first + across * step as f32 / steps as f32;
            let (x, y) = (rx * angle.cos(), ry * angle.sin());
            self.add_point((cos * x - sin * y + cx, sin * x + cos * y + cy));
        }
        self.line_to(to);
    }

    fn close(&mut self) {
        if let Some(run) = self.runs.last_mut() {
            run.closed = true;
        }
        self.pen = self.start;
    }
}

/// Goes along a path, turning it all into lines. As in SVG, a mistake ends it, keeping what came before.
fn trace(d: &str, place: Placement) -> Vec<Run> {
    let mut tracer = Tracer {
        place,
        runs: Vec::new(),
        pen: (0.0, 0.0),
        start: (0.0, 0.0),
    };
    let mut reader = Reader {
        bytes: d.as_bytes(),
        at: 0,
    };
    let mut command = 0u8;
    // The last control point, for the curves that carry on from it.
    let mut cubic_control = None;
    let mut quad_control = None;
    loop {
        // A number where a command should be repeats the last one, with a move going on as a line.
        let read_command = reader.command();
        let repeating = read_command.is_none();
        if let Some(found) = read_command {
            command = found;
        } else if reader.bytes.get(reader.at).is_none() || command == 0 {
            break;
        } else if command == b'M' {
            command = b'L';
        } else if command == b'm' {
            command = b'l';
        } else if matches!(command, b'z' | b'Z') {
            break;
        }
        let relative = command.is_ascii_lowercase();
        let base = if relative { tracer.pen } else { (0.0, 0.0) };
        let (mut next_cubic, mut next_quad) = (None, None);
        let done = (|| -> Option<()> {
            let point = |reader: &mut Reader<'_>| -> Option<(f32, f32)> {
                let x = reader.number()?;
                let y = reader.number()?;
                Some((x + base.0, y + base.1))
            };
            match command.to_ascii_uppercase() {
                b'M' => {
                    let to = point(&mut reader)?;
                    tracer.move_to(to);
                }
                b'L' => {
                    let to = point(&mut reader)?;
                    tracer.line_to(to);
                }
                b'H' => {
                    let x = reader.number()? + base.0;
                    tracer.line_to((x, tracer.pen.1));
                }
                b'V' => {
                    let y = reader.number()? + base.1;
                    tracer.line_to((tracer.pen.0, y));
                }
                b'C' => {
                    let one = point(&mut reader)?;
                    let two = point(&mut reader)?;
                    let to = point(&mut reader)?;
                    tracer.cubic_to(one, two, to);
                    next_cubic = Some(two);
                }
                b'S' => {
                    let pen = tracer.pen;
                    let one = cubic_control.map_or(pen, |control: (f32, f32)| {
                        (2.0 * pen.0 - control.0, 2.0 * pen.1 - control.1)
                    });
                    let two = point(&mut reader)?;
                    let to = point(&mut reader)?;
                    tracer.cubic_to(one, two, to);
                    next_cubic = Some(two);
                }
                b'Q' => {
                    let control = point(&mut reader)?;
                    let to = point(&mut reader)?;
                    tracer.quad_to(control, to);
                    next_quad = Some(control);
                }
                b'T' => {
                    let pen = tracer.pen;
                    let control = quad_control.map_or(pen, |control: (f32, f32)| {
                        (2.0 * pen.0 - control.0, 2.0 * pen.1 - control.1)
                    });
                    let to = point(&mut reader)?;
                    tracer.quad_to(control, to);
                    next_quad = Some(control);
                }
                b'A' => {
                    let rx = reader.number()?;
                    let ry = reader.number()?;
                    let turn = reader.number()?;
                    let large = reader.flag()?;
                    let sweep = reader.flag()?;
                    let to = point(&mut reader)?;
                    tracer.arc_to((rx, ry), turn, large, sweep, to);
                }
                b'Z' if !repeating => tracer.close(),
                _ => return None,
            }
            Some(())
        })();
        if done.is_none() {
            break;
        }
        cubic_control = next_cubic;
        quad_control = next_quad;
    }
    tracer.runs
}

/// The lines around the inside of what a path fills.
fn fill_lines(runs: &[Run], lines: &mut Vec<[f32; 4]>) {
    for run in runs {
        let points = &run.points;
        if points.len() < 2 {
            continue;
        }
        for pair in points.windows(2) {
            lines.push([pair[0].0, pair[0].1, pair[1].0, pair[1].1]);
        }
        // Everything filled is closed, whether it says so or not.
        let (first, last) = (points[0], points[points.len() - 1]);
        lines.push([last.0, last.1, first.0, first.1]);
    }
}

/// The parts of `runs` that a dash pattern shows, as SVG's `stroke-dasharray` and `stroke-dashoffset` have it: `pattern`
/// is the length of each dash and each gap after it in turn, said again if there's an odd number of them, and `offset` is
/// how far into the pattern each run starts. It's `None` for a solid line, which is what a pattern that's all gaps, or that
/// can't be one, means.
fn dashed(runs: &[Run], pattern: &[f32], offset: f32) -> Option<Vec<Run>> {
    if pattern
        .iter()
        .any(|length| !(length.is_finite() && *length >= 0.0))
    {
        return None;
    }
    let pattern: Vec<f32> = if pattern.len() % 2 == 1 {
        pattern.iter().chain(pattern).copied().collect()
    } else {
        pattern.to_vec()
    };
    let whole: f32 = pattern.iter().sum();
    if !(whole > 0.0 && whole.is_finite() && offset.is_finite()) {
        return None;
    }
    let mut out = Vec::new();
    for run in runs {
        let mut points = run.points.clone();
        if run.closed && points.len() > 1 && points.first() != points.last() {
            points.push(points[0]);
        }
        let Some(&first) = points.first() else {
            continue;
        };
        // Where in the pattern this starts, and how much of that part is left.
        let mut index = 0;
        let mut into = offset.rem_euclid(whole);
        // As Skia has it: a part that's been gone past, or reached the end of, unless it has no length, which is a dot here.
        while into > pattern[index] || (into == pattern[index] && pattern[index] > 0.0) {
            into -= pattern[index];
            index = (index + 1) % pattern.len();
        }
        let mut left = pattern[index] - into;
        let mut drawn: Vec<(f32, f32)> = if index % 2 == 0 {
            vec![first]
        } else {
            Vec::new()
        };
        for pair in points.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            let length = (to.0 - from.0).hypot(to.1 - from.1);
            let mut along = 0.0;
            while length - along > left {
                along += left;
                let part = along / length;
                let point = (
                    from.0 + (to.0 - from.0) * part,
                    from.1 + (to.1 - from.1) * part,
                );
                if index % 2 == 0 {
                    drawn.push(point);
                    out.push(Run {
                        points: std::mem::take(&mut drawn),
                        closed: false,
                    });
                } else {
                    drawn = vec![point];
                }
                index = (index + 1) % pattern.len();
                left = pattern[index];
            }
            left -= length - along;
            if index % 2 == 0 {
                drawn.push(to);
            }
        }
        if index % 2 == 0 && drawn.len() > 1 {
            out.push(Run {
                points: drawn,
                closed: false,
            });
        }
    }
    Some(out)
}

/// The lines around a path's stroke, with round corners.
#[cfg(test)]
fn stroke_lines(runs: &[Run], width: f32, cap: LineCap, lines: &mut Vec<[f32; 4]>) {
    stroke_lines_joined(runs, width, cap, LineJoin::Round, lines);
}

/// The lines around a path's stroke, with its corners as `join` says: one outline for each run, its two sides with the ends and
/// corners between them, as the canvas strokes. That's what a browser fills, and filled, its edges are only ever counted once.
/// Pieces laid over each other, a box for each line and a circle at each corner, would be counted twice at their edges where they
/// overlap, which a curve does at every point along it, and that made curves heavier than straight lines, and lumpy.
fn stroke_lines_joined(
    runs: &[Run],
    width: f32,
    cap: LineCap,
    join: LineJoin,
    lines: &mut Vec<[f32; 4]>,
) {
    use crate::canvas::geom::{Cap, Join, Matrix, Path, StrokeStyle, Stroker};
    if !(width > 0.0 && width.is_finite()) {
        return;
    }
    let mut path = Path::default();
    for run in runs {
        let Some(&first) = run.points.first() else {
            continue;
        };
        path.move_to(first);
        for &point in &run.points[1..] {
            path.line_to(point);
        }
        if run.closed {
            path.close();
        }
    }
    let style = StrokeStyle {
        width,
        cap: match cap {
            LineCap::Butt => Cap::Butt,
            LineCap::Round => Cap::Round,
            LineCap::Square => Cap::Square,
        },
        join: match join {
            LineJoin::Miter => Join::Miter,
            LineJoin::Round => Join::Round,
            LineJoin::Bevel => Join::Bevel,
        },
        // SVG's `stroke-miterlimit` is 4 unless it says.
        miter_limit: 4.0,
        ..StrokeStyle::default()
    };
    Stroker::default().stroke(&path, &style, &Matrix::IDENTITY, lines);
}

/// Lines cut down to what's inside a picture of this size, which is all that's going to be seen, whatever a path says. Past the
/// left or right they still count, at that edge, as whatever's to the right of a line is inside it. Above or below,
/// they don't reach anything. Lines too short or strange to add anything to it are left out, so that nothing further
/// on has to deal with them.
fn inside_picture(lines: &mut Vec<[f32; 4]>, width: f32, height: f32) {
    const SHORTEST: f32 = 1.0 / 1024.0;
    let given = std::mem::take(lines);
    for [mut x0, mut y0, mut x1, mut y1] in given {
        if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
            continue;
        }
        if (y0 <= 0.0 && y1 <= 0.0) || (y0 >= height && y1 >= height) {
            continue;
        }
        let across = |from: f32, to: f32, y0: f32, y1: f32, at: f32| {
            from + (to - from) * ((at - y0) / (y1 - y0))
        };
        let (start_x, start_y, end_x, end_y) = (x0, y0, x1, y1);
        for (y, x) in [(&mut y0, &mut x0), (&mut y1, &mut x1)] {
            if *y < 0.0 {
                *x = across(start_x, end_x, start_y, end_y, 0.0);
                *y = 0.0;
            } else if *y > height {
                *x = across(start_x, end_x, start_y, end_y, height);
                *y = height;
            }
        }
        if (y1 - y0).abs() < SHORTEST {
            continue;
        }
        // What's past an edge goes along that edge, which takes up to three pieces.
        let at_x = |x: f32| y0 + (y1 - y0) * ((x - x0) / (x1 - x0));
        let mut stops = [(x0, y0), (x0, y0), (x1, y1), (x1, y1)];
        if x0 != x1 {
            let (low, high) = if x0 < x1 { (0.0, width) } else { (width, 0.0) };
            if (x0 < low && low < x1) || (x0 > low && low > x1) {
                stops[1] = (low, at_x(low));
            }
            if (x0 < high && high < x1) || (x0 > high && high > x1) {
                stops[2] = (high, at_x(high));
            }
        }
        for pair in stops.windows(2) {
            let ((from_x, from_y), (to_x, to_y)) = (pair[0], pair[1]);
            if (to_y - from_y).abs() >= SHORTEST / 16.0 {
                lines.push([
                    from_x.clamp(0.0, width),
                    from_y,
                    to_x.clamp(0.0, width),
                    to_y,
                ]);
            }
        }
    }
}

/// Lays `over`, a color through this much coverage, on top of a picture of premultiplied pixels.
fn blend_coverage(
    canvas: &mut [u8],
    (width, height): (usize, usize),
    bitmap: &rsui_text::GlyphBitmap,
    color: Color,
) {
    let [red, green, blue, alpha] = color.premultiplied();
    let over = [
        u32::from(red),
        u32::from(green),
        u32::from(blue),
        u32::from(alpha),
    ];
    let bitmap_width = bitmap.width as usize;
    for (row, line) in bitmap
        .coverage
        .chunks_exact(bitmap_width.max(1))
        .enumerate()
    {
        let y = bitmap.top + row as i32;
        if y < 0 || y as usize >= height {
            continue;
        }
        for (column, &amount) in line.iter().enumerate() {
            let x = bitmap.left + column as i32;
            if amount == 0 || x < 0 || x as usize >= width {
                continue;
            }
            let at = (y as usize * width + x as usize) * 4;
            let amount = u32::from(amount);
            let leaves = 255 - (over[3] * amount + 127) / 255;
            for part in 0..4 {
                let under = u32::from(canvas[at + part]);
                canvas[at + part] = ((over[part] * amount + 127) / 255
                    + (under * leaves + 127) / 255)
                    .min(255) as u8;
            }
        }
    }
}

/// Draws a shape at this size, in pixels.
pub(crate) fn draw_shape(
    element: &Element,
    layers: &str,
    (width, height): (f32, f32),
    current_color: Color,
    rasterizer: &mut Rasterizer,
) -> Option<ColorBitmap> {
    let stretch = element.props.get("stretch") == Some(&PropValue::Bool(true));
    draw_layers(
        layers,
        view_box(element),
        stretch,
        (width, height),
        current_color,
        rasterizer,
    )
}

/// Draws layers as [`shape_layer`] writes them at this size, in pixels, with `view` as the part of their own numbers that's
/// shown, and stretched to fill all of it if `stretch` says.
pub(crate) fn draw_layers(
    layers: &str,
    view: Option<[f32; 4]>,
    stretch: bool,
    (width, height): (f32, f32),
    current_color: Color,
    rasterizer: &mut Rasterizer,
) -> Option<ColorBitmap> {
    if !(width >= 1.0 && height >= 1.0 && width <= LARGEST && height <= LARGEST) {
        return None;
    }
    let view = view.unwrap_or([0.0, 0.0, width, height]);
    let place = if stretch {
        Placement::to_fill(view, width, height)
    } else {
        Placement::to_fit(view, width, height)
    };
    let (pixels_wide, pixels_high) = (width as usize, height as usize);
    let mut canvas = vec![0u8; pixels_wide * pixels_high * 4];
    let mut lines = Vec::new();
    let color_from = |text: &str| {
        if text == "currentColor" {
            Some(current_color)
        } else {
            Color::from_text(text)
        }
    };
    for layer in layers.lines() {
        let mut parts = layer.splitn(9, '\t');
        let fill = parts.next().unwrap_or("");
        let stroke = parts.next().unwrap_or("");
        let stroke_width = parts
            .next()
            .and_then(|width| width.parse().ok())
            .unwrap_or(1.0f32);
        let cap = match parts.next() {
            Some("round") => LineCap::Round,
            Some("square") => LineCap::Square,
            _ => LineCap::Butt,
        };
        let d = parts.next().unwrap_or("");
        let mut place = place;
        if let Some(transform) = parts.next().filter(|text| !text.is_empty()) {
            place.matrix = parse_transform(transform, view[2], view[3]);
        }
        let dashes: Vec<f32> = parts
            .next()
            .unwrap_or("")
            .split(',')
            .filter_map(|length| length.trim().parse::<f32>().ok())
            .map(|length| length * place.scale)
            .collect();
        let dash_offset = parts
            .next()
            .and_then(|offset| offset.trim().parse::<f32>().ok())
            .unwrap_or(0.0)
            * place.scale;
        let join = match parts.next().map(str::trim) {
            Some("miter") => LineJoin::Miter,
            Some("bevel") => LineJoin::Bevel,
            _ => LineJoin::Round,
        };
        let runs = trace(d, place);
        if let Some(color) = color_from(fill) {
            lines.clear();
            fill_lines(&runs, &mut lines);
            inside_picture(&mut lines, width, height);
            let bitmap = rasterizer.fill(&mut lines, LARGEST as usize + 1);
            blend_coverage(&mut canvas, (pixels_wide, pixels_high), &bitmap, color);
        }
        if let Some(color) = color_from(stroke) {
            lines.clear();
            let dashed = dashed(&runs, &dashes, dash_offset);
            stroke_lines_joined(
                dashed.as_deref().unwrap_or(&runs),
                stroke_width * place.scale,
                cap,
                join,
                &mut lines,
            );
            inside_picture(&mut lines, width, height);
            let bitmap = rasterizer.fill(&mut lines, LARGEST as usize + 1);
            blend_coverage(&mut canvas, (pixels_wide, pixels_high), &bitmap, color);
        }
    }
    Some(ColorBitmap {
        left: 0,
        top: 0,
        width: pixels_wide as u32,
        height: pixels_high as u32,
        pixels: canvas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const AS_IS: Placement = Placement {
        matrix: IDENTITY,
        scale: 1.0,
        scale_x: 1.0,
        scale_y: 1.0,
        x: 0.0,
        y: 0.0,
    };

    fn points_of(d: &str) -> Vec<Vec<(f32, f32)>> {
        trace(d, AS_IS).into_iter().map(|run| run.points).collect()
    }

    #[test]
    fn straight_lines_read_every_way_they_can_be_written() {
        let square = vec![vec![(1.0, 2.0), (5.0, 2.0), (5.0, 6.0), (1.0, 6.0)]];
        assert_eq!(points_of("M1 2 L5 2 L5 6 L1 6 Z"), square);
        assert_eq!(
            points_of("M1,2 5,2 5,6 1,6z"),
            square,
            "a move goes on as lines"
        );
        assert_eq!(points_of("m1 2 l4 0 l0 4 l-4 0 z"), square);
        assert_eq!(points_of("M1 2H5V6H1Z"), square);
        assert_eq!(points_of("m1 2h4v4h-4z"), square);
    }

    #[test]
    fn numbers_can_run_together() {
        assert_eq!(
            points_of("M.5.5L1-1 2.5e1,1e-1"),
            vec![vec![(0.5, 0.5), (1.0, -1.0), (25.0, 0.1)]]
        );
    }

    #[test]
    fn a_mistake_keeps_what_came_before_it() {
        assert_eq!(
            points_of("M1 1 L2 2 L3"),
            vec![vec![(1.0, 1.0), (2.0, 2.0)]]
        );
        assert_eq!(
            points_of("M1 1 L2 2 Q what"),
            vec![vec![(1.0, 1.0), (2.0, 2.0)]]
        );
        assert_eq!(points_of(""), Vec::<Vec<(f32, f32)>>::new());
        assert_eq!(points_of("nonsense"), Vec::<Vec<(f32, f32)>>::new());
    }

    #[test]
    fn what_follows_closing_starts_from_where_that_started() {
        let runs = points_of("M1 1 L5 1 L5 5 Z L9 9");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[1], vec![(1.0, 1.0), (9.0, 9.0)]);
    }

    fn ends_at(points: &[(f32, f32)], x: f32, y: f32) {
        let last = points[points.len() - 1];
        assert!(
            (last.0 - x).abs() < 0.01 && (last.1 - y).abs() < 0.01,
            "{last:?}"
        );
    }

    #[test]
    fn curves_go_through_the_middle_they_should_and_end_where_they_say() {
        let runs = points_of("M0 0 C0 100 100 100 100 0");
        ends_at(&runs[0], 100.0, 0.0);
        assert!(runs[0].len() > 8, "it's smooth: {}", runs[0].len());
        // At its halfway point, a curve like this is three quarters of the way to its controls.
        let middle = runs[0].iter().map(|point| point.1).fold(0.0, f32::max);
        assert!((middle - 75.0).abs() < 0.5, "{middle}");
        // A smooth curve carries on the same way.
        let runs = points_of("M0 0 C0 10 10 10 10 0 S20 -10 20 0");
        ends_at(&runs[0], 20.0, 0.0);
        let lowest = runs[0].iter().map(|point| point.1).fold(0.0, f32::min);
        assert!(lowest < -5.0, "it goes on up: {lowest}");
        let runs = points_of("M0 0 Q50 100 100 0");
        ends_at(&runs[0], 100.0, 0.0);
        let middle = runs[0].iter().map(|point| point.1).fold(0.0, f32::max);
        assert!((middle - 50.0).abs() < 0.5, "{middle}");
        ends_at(&points_of("M0 0 Q5 10 10 0 T20 0")[0], 20.0, 0.0);
    }

    #[test]
    fn arcs_are_round_and_go_the_way_they_are_told_to() {
        // Half a circle of radius 50, from left to right along the top of its middle.
        let runs = points_of("M0 50 A50 50 0 0 1 100 50");
        ends_at(&runs[0], 100.0, 50.0);
        for point in &runs[0] {
            let distance = ((point.0 - 50.0).powi(2) + (point.1 - 50.0).powi(2)).sqrt();
            assert!(
                (distance - 50.0).abs() < 0.1,
                "{point:?} is {distance} from the middle"
            );
            assert!(
                point.1 <= 50.01,
                "sweeping clockwise goes over the top: {point:?}"
            );
        }
        let other = points_of("M0 50 A50 50 0 0 0 100 50");
        assert!(
            other[0].iter().all(|point| point.1 >= 49.99),
            "{:?}",
            other[0]
        );
        // Flags can have nothing after them.
        ends_at(&points_of("M0 0a5 5 0 0010 0")[0], 10.0, 0.0);
        // No radius is a straight line, and radii too small to reach are made big enough.
        assert_eq!(
            points_of("M0 0 A0 5 0 0 1 10 0")[0],
            vec![(0.0, 0.0), (10.0, 0.0)]
        );
        ends_at(&points_of("M0 0 A1 1 0 0 1 10 0")[0], 10.0, 0.0);
    }

    fn drawn_in_a_picture(d: &str, stroke: bool, size: f32) -> rsui_text::GlyphBitmap {
        let mut lines = Vec::new();
        let runs = trace(d, AS_IS);
        if stroke {
            stroke_lines(&runs, 2.0, LineCap::Butt, &mut lines);
        } else {
            fill_lines(&runs, &mut lines);
        }
        inside_picture(&mut lines, size, size);
        Rasterizer::new().fill(&mut lines, 8192)
    }

    #[test]
    fn what_a_path_says_can_not_make_a_picture_any_bigger() {
        // A long way off, in every direction, which took hundreds of megabytes to draw.
        let bitmap = drawn_in_a_picture("M0 0 L8000 8000 L-8000 8000 Z", false, 16.0);
        assert!(
            bitmap.width <= 17 && bitmap.height <= 17,
            "{}x{}",
            bitmap.width,
            bitmap.height
        );
        let bitmap = drawn_in_a_picture(
            "M-3e30 -3e30 L3e30 -3e30 L3e30 3e30 L-3e30 3e30 Z",
            false,
            16.0,
        );
        assert!(
            bitmap.width <= 17 && bitmap.height <= 17,
            "{}x{}",
            bitmap.width,
            bitmap.height
        );
        assert_eq!(coverage_at(&bitmap, 8, 8), 255, "all of it is inside that");
    }

    #[test]
    fn a_shape_past_the_edges_still_fills_what_is_inside() {
        let bitmap = drawn_in_a_picture("M-100 2 H100 V6 H-100 Z", false, 16.0);
        assert_eq!(coverage_at(&bitmap, 0, 3), 255);
        assert_eq!(coverage_at(&bitmap, 15, 3), 255);
        assert_eq!(coverage_at(&bitmap, 8, 8), 0);
        let bitmap = drawn_in_a_picture("M8 -100 V100 H100 V-100 Z", false, 16.0);
        assert_eq!(coverage_at(&bitmap, 7, 8), 0);
        assert_eq!(coverage_at(&bitmap, 8, 8), 255);
        assert_eq!(coverage_at(&bitmap, 15, 15), 255);
        // A slope going through a corner still has the right slope inside.
        let bitmap = drawn_in_a_picture("M-16 -16 L32 32 L-16 32 Z", false, 16.0);
        assert_eq!(coverage_at(&bitmap, 2, 12), 255);
        assert_eq!(coverage_at(&bitmap, 12, 2), 0);
    }

    #[test]
    fn nothing_a_path_can_say_goes_wrong() {
        for d in [
            "M0 0 L1000 1e-36 Z",
            "M0 0 L1e-30 0",
            "M1e999 0 L0 0 L5 5 Z",
            "M0 0 L1e38 1e38 L-1e38 1e38 Z",
            "M0 0 C1e38 1e38 -1e38 1e38 5 5 Z",
            "M0 0 A1e30 1e30 0 1 1 5 5 Z",
            "M0 0 A1e-30 1e-30 0 1 1 5 5 Z",
            "M5 5 L5 5 L5 5 Z",
            "M0 0 H0 V0 Z",
        ] {
            drawn_in_a_picture(d, false, 16.0);
            drawn_in_a_picture(d, true, 16.0);
        }
    }

    fn drawn(lines: &mut Vec<[f32; 4]>) -> rsui_text::GlyphBitmap {
        Rasterizer::new().fill(lines, 8192)
    }

    fn coverage_at(bitmap: &rsui_text::GlyphBitmap, x: i32, y: i32) -> u8 {
        let (x, y) = (x - bitmap.left, y - bitmap.top);
        if x < 0 || y < 0 || x >= bitmap.width as i32 || y >= bitmap.height as i32 {
            return 0;
        }
        bitmap.coverage[y as usize * bitmap.width as usize + x as usize]
    }

    #[test]
    fn filling_covers_exactly_what_is_inside_and_opposite_ways_around_make_a_hole() {
        let mut lines = Vec::new();
        fill_lines(&trace("M2 2 H8 V8 H2 Z", AS_IS), &mut lines);
        let bitmap = drawn(&mut lines);
        assert_eq!(coverage_at(&bitmap, 5, 5), 255);
        assert_eq!(coverage_at(&bitmap, 1, 5), 0);
        assert_eq!(coverage_at(&bitmap, 8, 5), 0);
        lines.clear();
        fill_lines(
            &trace("M0 0 H10 V10 H0 Z M3 3 V7 H7 V3 Z", AS_IS),
            &mut lines,
        );
        let bitmap = drawn(&mut lines);
        assert_eq!(coverage_at(&bitmap, 1, 1), 255);
        assert_eq!(coverage_at(&bitmap, 5, 5), 0, "that's the hole");
    }

    #[test]
    fn a_stroke_is_as_wide_as_it_says_and_corners_stay_solid() {
        let mut lines = Vec::new();
        stroke_lines(
            &trace("M2 5 H12 V15", AS_IS),
            2.0,
            LineCap::Butt,
            &mut lines,
        );
        let bitmap = drawn(&mut lines);
        assert_eq!(coverage_at(&bitmap, 6, 4), 255);
        assert_eq!(coverage_at(&bitmap, 6, 5), 255);
        assert_eq!(coverage_at(&bitmap, 6, 3), 0);
        assert_eq!(coverage_at(&bitmap, 6, 6), 0);
        // Where it turns, the pieces overlap, and that isn't twice as much of anything.
        assert_eq!(coverage_at(&bitmap, 11, 4), 255);
        assert_eq!(coverage_at(&bitmap, 11, 5), 255);
        // Butt ends stop at the end.
        assert_eq!(coverage_at(&bitmap, 1, 4), 0);
    }

    #[test]
    fn ends_are_as_they_are_asked_to_be() {
        let end_of = |cap| {
            let mut lines = Vec::new();
            stroke_lines(&trace("M10 10 H20", AS_IS), 4.0, cap, &mut lines);
            let bitmap = drawn(&mut lines);
            (coverage_at(&bitmap, 8, 9), coverage_at(&bitmap, 8, 11 - 3))
        };
        assert_eq!(end_of(LineCap::Butt).0, 0);
        assert_eq!(end_of(LineCap::Square).0, 255);
        // A round end is there in the middle, and not at the corner.
        let round = end_of(LineCap::Round);
        assert!(round.0 > 100, "{round:?}");
        assert!(round.1 < 200, "{round:?}");
    }

    #[test]
    fn a_closed_stroke_has_no_gap_where_it_started() {
        let mut lines = Vec::new();
        stroke_lines(
            &trace("M2 2 H12 V12 H2 Z", AS_IS),
            2.0,
            LineCap::Butt,
            &mut lines,
        );
        let bitmap = drawn(&mut lines);
        assert_eq!(coverage_at(&bitmap, 2, 2), 255);
        assert_eq!(coverage_at(&bitmap, 1, 6), 255);
    }

    #[test]
    fn a_curve_is_no_heavier_than_it_is_wide() {
        // A circle 16 across, drawn 1.5 wide, as an icon's would be: its ink is its length times its width, with edges that
        // aren't counted twice where one bit of the curve meets the next.
        for width in [1.0f32, 1.5, 3.0] {
            let mut lines = Vec::new();
            stroke_lines(
                &trace("M20 12 A8 8 0 1 1 4 12 A8 8 0 1 1 20 12 Z", AS_IS),
                width,
                LineCap::Butt,
                &mut lines,
            );
            let bitmap = drawn(&mut lines);
            let ink: f32 = bitmap.coverage.iter().map(|&amount| f32::from(amount) / 255.0).sum();
            let wanted = std::f32::consts::TAU * 8.0 * width;
            assert!((ink / wanted - 1.0).abs() < 0.02, "{width}: {ink} against {wanted}");
        }
    }

    #[test]
    fn dashes_go_along_a_line_from_where_the_offset_says() {
        let ends = |pattern: &[f32], offset: f32| -> Vec<(f32, f32)> {
            dashed(&trace("M0 0 H30", AS_IS), pattern, offset)
                .unwrap()
                .iter()
                .map(|run| (run.points[0].0, run.points.last().unwrap().0))
                .collect()
        };
        assert_eq!(ends(&[10.0, 5.0], 0.0), [(0.0, 10.0), (15.0, 25.0)]);
        // Starting 4 into the pattern, the first dash has 6 left.
        assert_eq!(
            ends(&[10.0, 5.0], 4.0),
            [(0.0, 6.0), (11.0, 21.0), (26.0, 30.0)]
        );
        // A negative offset is that far from the end of the pattern: 12 in, which is in the gap, with 3 of it left.
        assert_eq!(ends(&[10.0, 5.0], -3.0), [(3.0, 13.0), (18.0, 28.0)]);
        // An odd number of lengths is said twice, so `4` is four on and four off.
        assert_eq!(
            ends(&[4.0], 0.0),
            [(0.0, 4.0), (8.0, 12.0), (16.0, 20.0), (24.0, 28.0)]
        );
        // A dash that has no length is still somewhere, for a round end to show as a dot.
        assert_eq!(ends(&[0.0, 10.0], 0.0)[..2], [(0.0, 0.0), (10.0, 10.0)]);
        // All gaps, or what can't be a length, is a solid line.
        assert!(dashed(&trace("M0 0 H30", AS_IS), &[0.0, 0.0], 0.0).is_none());
        assert!(dashed(&trace("M0 0 H30", AS_IS), &[-1.0, 2.0], 0.0).is_none());
    }

    #[test]
    fn a_dash_goes_around_a_corner() {
        let runs = dashed(&trace("M0 0 H10 V10", AS_IS), &[15.0, 100.0], 0.0).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].points, [(0.0, 0.0), (10.0, 0.0), (10.0, 5.0)]);
    }

    #[test]
    fn a_shape_goes_in_the_middle_of_what_it_is_given() {
        let place = Placement::to_fit([0.0, 0.0, 10.0, 10.0], 100.0, 50.0);
        assert_eq!(place.pixel((0.0, 0.0)), (25.0, 0.0));
        assert_eq!(place.pixel((10.0, 10.0)), (75.0, 50.0));
        let moved = Placement::to_fit([5.0, 5.0, 10.0, 10.0], 10.0, 10.0);
        assert_eq!(moved.pixel((5.0, 5.0)), (0.0, 0.0));
    }

    #[test]
    fn stretching_fills_it_both_ways_and_keeps_lines_as_thin_as_the_smaller_way() {
        let place = Placement::to_fill([0.0, 0.0, 10.0, 10.0], 100.0, 50.0);
        assert_eq!(place.pixel((10.0, 10.0)), (100.0, 50.0));
        assert_eq!(place.pixel((0.0, 0.0)), (0.0, 0.0));
        assert_eq!(place.scale, 5.0);
    }

    #[test]
    fn layers_come_out_as_they_should_be_read() {
        let layer = shape_layer(
            "M0 0",
            Some(Some(Color::hex(0xFF0000))),
            Some((None, 2.5)),
            LineCap::Round,
        );
        assert_eq!(layer, "#ff0000ff\tcurrentColor\t2.5\tround\tM0 0");
        let layer = shape_layer("M0 0", None, None, LineCap::Butt);
        assert_eq!(layer, "\t\t0\tbutt\tM0 0");
    }

    #[test]
    fn nothing_silly_is_drawn() {
        let element = Element::new(rsui_core::Role::Image);
        let mut rasterizer = Rasterizer::new();
        let black = Color::BLACK;
        assert!(draw_shape(&element, "", (0.0, 10.0), black, &mut rasterizer).is_none());
        assert!(draw_shape(&element, "", (1e6, 10.0), black, &mut rasterizer).is_none());
        assert!(draw_shape(&element, "", (f32::NAN, 10.0), black, &mut rasterizer).is_none());
        let empty = draw_shape(
            &element,
            "not\ta\tlayer",
            (4.0, 4.0),
            black,
            &mut rasterizer,
        );
        assert!(empty.unwrap().pixels.iter().all(|part| *part == 0));
    }
}
