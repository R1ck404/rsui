//! Turning something and changing its size, once it's been laid out. None of this moves anything else, and only drawing and
//! finding what's under the pointer ever know about it.

use std::sync::LazyLock;

use rsui_core::{Element, PropValue, Symbol};

/// How to move a point: `x` becomes `a x + c y + e`, and `y` becomes `b x + d y + f`, for `[a, b, c, d, e, f]`.
pub type Matrix = [f32; 6];

pub(crate) const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// What `first` does, and then what `then` does.
pub(crate) fn multiply(then: Matrix, first: Matrix) -> Matrix {
    let [a, b, c, d, e, f] = then;
    let [g, h, i, j, k, l] = first;
    [
        a * g + c * h,
        b * g + d * h,
        a * i + c * j,
        b * i + d * j,
        a * k + c * l + e,
        b * k + d * l + f,
    ]
}

/// Where this matrix puts a point.
pub fn apply_matrix(matrix: Matrix, x: f32, y: f32) -> (f32, f32) {
    let [a, b, c, d, e, f] = matrix;
    (a * x + c * y + e, b * x + d * y + f)
}

/// What undoes this, unless it's squashed everything flat, when nothing can.
pub fn invert_matrix(matrix: Matrix) -> Option<Matrix> {
    let [a, b, c, d, e, f] = matrix;
    let determinant = a * d - b * c;
    if determinant.abs() < 1e-6 {
        return None;
    }
    let (a2, b2, c2, d2) = (
        d / determinant,
        -b / determinant,
        -c / determinant,
        a / determinant,
    );
    Some([a2, b2, c2, d2, -(a2 * e + c2 * f), -(b2 * e + d2 * f)])
}

struct Names {
    scale: Symbol,
    scale_x: Symbol,
    scale_y: Symbol,
    rotate: Symbol,
    transform: Symbol,
    transform_origin: Symbol,
}

static NAMES: LazyLock<Names> = LazyLock::new(|| Names {
    scale: Symbol::new("scale"),
    scale_x: Symbol::new("scale_x"),
    scale_y: Symbol::new("scale_y"),
    rotate: Symbol::new("rotate"),
    transform: Symbol::new("transform"),
    transform_origin: Symbol::new("transform_origin"),
});

fn number(value: &PropValue) -> Option<f32> {
    match value {
        PropValue::Float(value) => Some(*value as f32),
        PropValue::Int(value) => Some(*value as f32),
        _ => None,
    }
}

fn scaling(x: f32, y: f32) -> Matrix {
    [x, 0.0, 0.0, y, 0.0, 0.0]
}

fn rotating(degrees: f32) -> Matrix {
    let (sine, cosine) = degrees.to_radians().sin_cos();
    [cosine, sine, -sine, cosine, 0.0, 0.0]
}

fn moving(x: f32, y: f32) -> Matrix {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

/// A length as CSS writes it, in layout pixels, with `whole` being what a percentage's of. It can be a `calc()` of pixels and
/// percentages, as `translate(calc(-50% + 4px))` centers something on a point 4 pixels in.
fn length(text: &str, whole: f32) -> Option<f32> {
    let text = text.trim();
    if let Some(percent) = text.strip_suffix('%') {
        if let Ok(percent) = percent.trim().parse::<f32>() {
            return Some(percent / 100.0 * whole);
        }
    }
    if let Ok(pixels) = text.strip_suffix("px").unwrap_or(text).trim().parse() {
        return Some(pixels);
    }
    let mut reading = Calc { text: text.as_bytes(), at: 0 };
    let sum = reading.sum()?;
    reading.space();
    (reading.at == reading.text.len()).then(|| sum.percent / 100.0 * whole + sum.pixels)
}

/// Some percent of something and some pixels, or a plain number, as a part of a `calc()` comes to.
#[derive(Clone, Copy)]
struct Linear {
    percent: f32,
    pixels: f32,
    /// It's a number with no unit, which is what can multiply or divide.
    plain: bool,
}

/// Reads a `calc()`, or what's in one, of pixels, percentages and numbers, with `+`, `-`, `*`, `/` and brackets.
struct Calc<'a> {
    text: &'a [u8],
    at: usize,
}

impl Calc<'_> {
    fn space(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn sum(&mut self) -> Option<Linear> {
        let mut sum = self.product()?;
        loop {
            self.space();
            let sign = match self.text.get(self.at) {
                Some(b'+') => 1.0,
                Some(b'-') => -1.0,
                _ => return Some(sum),
            };
            self.at += 1;
            let next = self.product()?;
            sum = Linear {
                percent: sum.percent + sign * next.percent,
                pixels: sum.pixels + sign * next.pixels,
                plain: sum.plain && next.plain,
            };
        }
    }

    fn product(&mut self) -> Option<Linear> {
        let mut product = self.factor()?;
        loop {
            self.space();
            let times = match self.text.get(self.at) {
                Some(b'*') => true,
                Some(b'/') => false,
                _ => return Some(product),
            };
            self.at += 1;
            let next = self.factor()?;
            let (length, by) = if times && product.plain {
                (next, product.pixels)
            } else if next.plain {
                (product, if times { next.pixels } else { 1.0 / next.pixels })
            } else {
                return None;
            };
            product = Linear {
                percent: length.percent * by,
                pixels: length.pixels * by,
                plain: length.plain,
            };
        }
    }

    fn factor(&mut self) -> Option<Linear> {
        self.space();
        let rest = &self.text[self.at..];
        if rest.first() == Some(&b'(') || rest.starts_with(b"calc(") {
            self.at += if rest[0] == b'(' { 1 } else { 5 };
            let inside = self.sum()?;
            self.space();
            if self.text.get(self.at) != Some(&b')') {
                return None;
            }
            self.at += 1;
            return Some(inside);
        }
        let start = self.at;
        if matches!(self.text.get(self.at), Some(b'-' | b'+')) {
            self.at += 1;
        }
        while self
            .text
            .get(self.at)
            .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
        {
            self.at += 1;
        }
        let number: f32 = std::str::from_utf8(&self.text[start..self.at])
            .ok()?
            .parse()
            .ok()?;
        let rest = &self.text[self.at..];
        if rest.first() == Some(&b'%') {
            self.at += 1;
            Some(Linear { percent: number, pixels: 0.0, plain: false })
        } else if rest.starts_with(b"px") {
            self.at += 2;
            Some(Linear { percent: 0.0, pixels: number, plain: false })
        } else if rest.starts_with(b"rem") {
            // A page's own font size, which is 16 pixels unless it says otherwise.
            self.at += 3;
            Some(Linear { percent: 0.0, pixels: number * 16.0, plain: false })
        } else if rest.first().is_some_and(u8::is_ascii_alphabetic) {
            None
        } else {
            Some(Linear { percent: 0.0, pixels: number, plain: true })
        }
    }
}

/// What's in a function's brackets, from just after its `(` up to the `)` that closes it, brackets inside and all: the parts,
/// split by commas, or by spaces when there are none, and how far on that `)` is. `None` if it isn't closed.
fn arguments(text: &str) -> Option<(Vec<&str>, usize)> {
    let mut depth = 0usize;
    let mut close = None;
    let mut commas = Vec::new();
    for (index, byte) in text.bytes().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' if depth == 0 => {
                close = Some(index);
                break;
            }
            b')' => depth -= 1,
            b',' if depth == 0 => commas.push(index),
            _ => {}
        }
    }
    let close = close?;
    let inside = &text[..close];
    let mut parts = Vec::new();
    if commas.is_empty() {
        let (mut depth, mut start) = (0usize, 0usize);
        for (index, byte) in inside.bytes().enumerate() {
            match byte {
                b'(' => depth += 1,
                b')' => depth = depth.saturating_sub(1),
                byte if byte.is_ascii_whitespace() && depth == 0 => {
                    parts.push(&inside[start..index]);
                    start = index + 1;
                }
                _ => {}
            }
        }
        parts.push(&inside[start..]);
    } else {
        let mut start = 0;
        for comma in commas {
            parts.push(&inside[start..comma]);
            start = comma + 1;
        }
        parts.push(&inside[start..]);
    }
    let parts = parts
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    Some((parts, close))
}

/// An angle as CSS writes it, in degrees.
pub(crate) fn parse_angle(text: &str) -> Option<f32> {
    angle(text)
}

/// An angle as CSS writes it, in degrees.
fn angle(text: &str) -> Option<f32> {
    let text = text.trim();
    if let Some(value) = text.strip_suffix("deg") {
        return value.trim().parse().ok();
    }
    if let Some(value) = text.strip_suffix("grad") {
        return Some(value.trim().parse::<f32>().ok()? * 0.9);
    }
    if let Some(value) = text.strip_suffix("rad") {
        return Some(value.trim().parse::<f32>().ok()?.to_degrees());
    }
    if let Some(value) = text.strip_suffix("turn") {
        return Some(value.trim().parse::<f32>().ok()? * 360.0);
    }
    text.parse().ok()
}

/// What CSS's `transform` says, for something this big, in layout pixels. It's only for what can be done on a flat surface: what's to do
/// with depth goes by what it does to `x` and `y`, and anything this doesn't know is left out.
pub fn parse_transform(text: &str, width: f32, height: f32) -> Matrix {
    let mut matrix = IDENTITY;
    let mut rest = text.trim();
    while let Some(open) = rest.find('(') {
        let Some((parts, close)) = arguments(&rest[open + 1..]) else {
            break;
        };
        let close = close + open + 1;
        let name = rest[..open].trim();
        let at = |index: usize| parts.get(index).copied();
        let across = |index: usize| at(index).and_then(|part| length(part, width));
        let down = |index: usize| at(index).and_then(|part| length(part, height));
        let plain = |index: usize| at(index).and_then(|part| part.parse::<f32>().ok());
        let turned = |index: usize| at(index).and_then(angle);
        let step = match name {
            "translate" | "translate3d" => across(0).map(|x| moving(x, down(1).unwrap_or(0.0))),
            "translateX" => across(0).map(|x| moving(x, 0.0)),
            "translateY" => down(0).map(|y| moving(0.0, y)),
            "scale" | "scale3d" => plain(0).map(|x| scaling(x, plain(1).unwrap_or(x))),
            "scaleX" => plain(0).map(|x| scaling(x, 1.0)),
            "scaleY" => plain(0).map(|y| scaling(1.0, y)),
            "rotate" | "rotateZ" => turned(0).map(rotating),
            // Turned around an upright or a level line, seen from straight on, it's narrower or shorter, and flipped past a
            // quarter turn.
            "rotateY" => turned(0).map(|y| scaling(y.to_radians().cos(), 1.0)),
            "rotateX" => turned(0).map(|x| scaling(1.0, x.to_radians().cos())),
            "skew" => turned(0).map(|x| {
                [
                    1.0,
                    turned(1).unwrap_or(0.0).to_radians().tan(),
                    x.to_radians().tan(),
                    1.0,
                    0.0,
                    0.0,
                ]
            }),
            "skewX" => turned(0).map(|x| [1.0, 0.0, x.to_radians().tan(), 1.0, 0.0, 0.0]),
            "skewY" => turned(0).map(|y| [1.0, y.to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
            "matrix" => (|| {
                Some([
                    plain(0)?,
                    plain(1)?,
                    plain(2)?,
                    plain(3)?,
                    plain(4)?,
                    plain(5)?,
                ])
            })(),
            _ => None,
        };
        if let Some(step) = step {
            matrix = multiply(matrix, step);
        }
        rest = &rest[close + 1..];
    }
    matrix
}

/// Where `transform_origin` says to turn around, from the top left of something this big. It's the middle unless it says otherwise.
fn origin_of(text: &str, width: f32, height: f32) -> (f32, f32) {
    let mut across = None;
    let mut down = None;
    let words: Vec<&str> = text.split_whitespace().collect();
    for (index, word) in words.iter().enumerate() {
        match *word {
            "left" => across = Some(0.0),
            "right" => across = Some(width),
            "top" => down = Some(0.0),
            "bottom" => down = Some(height),
            "center" => {}
            other => {
                // One on its own, or the first, goes across, and the second goes down.
                if index == 0 {
                    across = length(other, width);
                } else {
                    down = length(other, height);
                }
            }
        }
    }
    (across.unwrap_or(width / 2.0), down.unwrap_or(height / 2.0))
}

/// How this element's turned and resized, if it is. `left`, `top`, `width` and `height` are where it was laid out, and `unit` is how many
/// of those there are to a layout pixel.
pub fn transform_of(
    element: &Element,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    unit: f32,
) -> Option<Matrix> {
    let names = &*NAMES;
    let props = &element.props;
    let read = |name: Symbol| props.get_symbol(name).and_then(number);
    let scale = read(names.scale);
    let scale_x = read(names.scale_x);
    let scale_y = read(names.scale_y);
    let rotate = read(names.rotate);
    let written = match props.get_symbol(names.transform) {
        Some(PropValue::Str(text)) => Some(text),
        _ => None,
    };
    if scale.is_none()
        && scale_x.is_none()
        && scale_y.is_none()
        && rotate.is_none()
        && written.is_none()
    {
        return None;
    }
    let (origin_x, origin_y) = match props.get_symbol(names.transform_origin) {
        Some(PropValue::Str(text)) => origin_of(text, width / unit, height / unit),
        _ => (width / unit / 2.0, height / unit / 2.0),
    };
    // In the same order CSS has them: turned, then resized, and then what `transform` says.
    let mut matrix = rotating(rotate.unwrap_or(0.0));
    let base = scale.unwrap_or(1.0);
    matrix = multiply(
        matrix,
        scaling(scale_x.unwrap_or(base), scale_y.unwrap_or(base)),
    );
    if let Some(text) = written {
        matrix = multiply(matrix, parse_transform(text, width / unit, height / unit));
    }
    // What says how far to move is in layout pixels, and the rest of it has no units.
    matrix[4] *= unit;
    matrix[5] *= unit;
    let (around_x, around_y) = (left + origin_x * unit, top + origin_y * unit);
    Some(multiply(
        moving(around_x, around_y),
        multiply(matrix, moving(-around_x, -around_y)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsui_core::Role;

    fn close(found: (f32, f32), wanted: (f32, f32)) {
        assert!(
            (found.0 - wanted.0).abs() < 0.001 && (found.1 - wanted.1).abs() < 0.001,
            "{found:?} and not {wanted:?}"
        );
    }

    #[test]
    fn turning_goes_clockwise_on_a_screen() {
        // Going down the screen is positive, so a quarter turn clockwise takes what's to the right to below.
        close(apply_matrix(rotating(90.0), 10.0, 0.0), (0.0, 10.0));
    }

    #[test]
    fn turning_over_is_seen_from_straight_on() {
        let at = |text: &str| apply_matrix(parse_transform(text, 200.0, 100.0), 1.0, 1.0);
        // Half a turn around an upright line flips it left to right, and a quarter turn leaves no width at all.
        let flipped = at("rotateY(-180deg)");
        assert!((flipped.0 + 1.0).abs() < 1e-5 && (flipped.1 - 1.0).abs() < 1e-5);
        assert!(at("rotateY(90deg)").0.abs() < 1e-5);
        let over = at("rotateX(180deg)");
        assert!((over.0 - 1.0).abs() < 1e-5 && (over.1 + 1.0).abs() < 1e-5);
    }

    #[test]
    fn transforms_apply_from_the_last_written_to_the_first() {
        // CSS's `translate(10px) scale(2)` resizes first, and moves afterwards.
        let matrix = parse_transform("translate(10px) scale(2)", 100.0, 100.0);
        close(apply_matrix(matrix, 1.0, 1.0), (12.0, 2.0));
        let other = parse_transform("scale(2) translate(10px)", 100.0, 100.0);
        close(apply_matrix(other, 1.0, 1.0), (22.0, 2.0));
    }

    #[test]
    fn what_css_writes_can_be_read() {
        let at = |text: &str| apply_matrix(parse_transform(text, 200.0, 100.0), 1.0, 1.0);
        close(at("translateX(5px)"), (6.0, 1.0));
        close(at("translateY(-50%)"), (1.0, -49.0));
        close(at("translate(50%, 10%)"), (101.0, 11.0));
        close(at("translate3d(1px, 2px, 3px)"), (2.0, 3.0));
        close(at("scale(3)"), (3.0, 3.0));
        close(at("scale(2, 4)"), (2.0, 4.0));
        close(at("scaleX(2)"), (2.0, 1.0));
        close(at("scaleY(2)"), (1.0, 2.0));
        close(at("rotate(0.25turn)"), (-1.0, 1.0));
        close(at("rotate(90deg)"), (-1.0, 1.0));
        close(at("skewX(45deg)"), (2.0, 1.0));
        close(at("skewY(45deg)"), (1.0, 2.0));
        close(at("matrix(2, 0, 0, 2, 5, 6)"), (7.0, 8.0));
        close(
            at("perspective(100px) translateZ(4px) scale(2)"),
            (2.0, 2.0),
        );
        close(at("nonsense"), (1.0, 1.0));
        close(at("rotate("), (1.0, 1.0));
        // Mantine's slider marks: back half their width and on a few pixels, with brackets inside the brackets.
        close(at("translate(calc(-50% + 4px), calc(10px / 2))"), (-95.0, 6.0));
        close(at("translate(calc(-50% + 4px), 5px) scale(2)"), (-94.0, 7.0));
        close(at("translateX(calc((100% - 20px) * -1))"), (-179.0, 1.0));
        close(at("translate(calc(10em), 2px)"), (1.0, 1.0));
        close(
            at("translate(calc(-50% + calc(0.5rem * 1) / 2), calc(calc(0.625rem * 1) / 2))"),
            (-95.0, 6.0),
        );
        close(at(""), (1.0, 1.0));
    }

    #[test]
    fn undoing_takes_a_point_back_where_it_came_from() {
        let matrix = parse_transform("translate(3px, 4px) rotate(30deg) scale(2, 5)", 0.0, 0.0);
        let back = invert_matrix(matrix).unwrap();
        let (x, y) = apply_matrix(matrix, 7.0, 11.0);
        close(apply_matrix(back, x, y), (7.0, 11.0));
        assert_eq!(invert_matrix(scaling(0.0, 1.0)), None);
    }

    fn with(name: &str, value: impl Into<PropValue>) -> Element {
        Element::new(Role::Group).prop(name, value)
    }

    #[test]
    fn something_turns_around_its_middle_unless_it_says_otherwise() {
        let turned = with("rotate", 90.0);
        let matrix = transform_of(&turned, 100.0, 100.0, 20.0, 20.0, 1.0).unwrap();
        close(apply_matrix(matrix, 110.0, 110.0), (110.0, 110.0));
        close(apply_matrix(matrix, 120.0, 110.0), (110.0, 120.0));
        let cornered = turned.clone().prop("transform_origin", "top left");
        let matrix = transform_of(&cornered, 100.0, 100.0, 20.0, 20.0, 1.0).unwrap();
        close(apply_matrix(matrix, 100.0, 100.0), (100.0, 100.0));
        close(apply_matrix(matrix, 120.0, 100.0), (100.0, 120.0));
    }

    #[test]
    fn nothing_that_says_nothing_is_transformed() {
        assert_eq!(
            transform_of(&Element::new(Role::Group), 0.0, 0.0, 10.0, 10.0, 1.0),
            None
        );
    }

    #[test]
    fn a_scale_of_the_window_does_not_change_what_a_transform_comes_to() {
        let moved = with("transform", "translate(10px, 0)");
        // At 1.5 physical pixels to a layout pixel, ten of those are fifteen.
        let matrix = transform_of(&moved, 0.0, 0.0, 30.0, 30.0, 1.5).unwrap();
        close(apply_matrix(matrix, 0.0, 0.0), (15.0, 0.0));
        let sized = with("scale", 2.0);
        // Its middle is at 15 either way.
        let matrix = transform_of(&sized, 0.0, 0.0, 30.0, 30.0, 1.5).unwrap();
        close(apply_matrix(matrix, 15.0, 15.0), (15.0, 15.0));
        close(apply_matrix(matrix, 0.0, 0.0), (-15.0, -15.0));
    }

    #[test]
    fn origins_read_as_css_writes_them() {
        let origin = |text: &str| origin_of(text, 200.0, 100.0);
        close(origin("center"), (100.0, 50.0));
        close(origin("top"), (100.0, 0.0));
        close(origin("right bottom"), (200.0, 100.0));
        close(origin("10px 20px"), (10.0, 20.0));
        close(origin("25% 75%"), (50.0, 75.0));
        close(origin("10px"), (10.0, 50.0));
    }
}
