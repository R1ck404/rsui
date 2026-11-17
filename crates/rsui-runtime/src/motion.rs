//! Props that move from one value to another over time by themselves, as CSS's `transition` and `animation` have them.
//!
//! An element says what moves with two props. `transitions` lists props that, when they change, go to their new value
//! over a while rather than at once, one to a line: its name, `n` for a number, `c` for a color, `s` for text with
//! numbers in it or `h` for a list of shadows (as `shadows` has them), then how long it takes and how long it waits first, in
//! seconds, and how it eases, as CSS says that. After a `|`, a line can say which of CSS's properties the prop stands for,
//! with commas between, which are what's told it's started and ended, one each: `border_color` is CSS's four
//! `border-*-color` longhands.
//!
//! ```text
//! opacity n 0.2 0 ease
//! background c 0.3 0 linear|background-color
//! ```
//!
//! `animation` says how a set of keyframes plays when the element first shows, or when it changes: how long one time
//! through takes and how long it waits first, in seconds, how many times it goes through (`inf` for always), which way
//! (`normal`, `reverse`, `alternate` or `alternate-reverse`), what it leaves when it isn't playing (`none`, `forwards`,
//! `backwards` or `both`), and how it eases between keyframes. Its keyframes follow, one prop to a line: how far through
//! (0 to 1), the prop, its kind, and the value there.
//!
//! ```text
//! 0.3 0 1 normal none ease
//! 0 opacity n 0
//! 1 opacity n 1
//! ```
//!
//! What's drawn is what's moving, and the element keeps what it says it is underneath, to go back to.

use std::sync::{Arc as StdArc, LazyLock};
use std::time::{Duration, Instant};

use rsui_core::{ActionKind, Key, Node, Patch, Path, PropValue, Role, Symbol, Text};
use triomphe::Arc;

struct Names {
    transitions: Symbol,
    animation: Symbol,
}

static NAMES: LazyLock<Names> = LazyLock::new(|| Names {
    transitions: Symbol::new("transitions"),
    animation: Symbol::new("animation"),
});

/// Displayed values may differ from the last declared target. Such nodes must
/// be seen again by the motion machinery even when they look unchanged.
pub(crate) fn declared_on(props: &rsui_core::Props) -> bool {
    if props.is_empty() {
        return false;
    }
    props.get_symbol(NAMES.animation).is_some() || props.get_symbol(NAMES.transitions).is_some()
}

/// What kind of value a prop has, which says how it goes from one to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Number,
    /// Red, green, blue and alpha in one number, as `Color::to_prop` has it.
    Color,
    /// Text whose numbers move, as `translateY(10px)` does to `translateY(0px)`.
    Text,
    /// A list of shadows, one to a line as `x y blur spread color`, with `inset` after one that's inside, as `shadows` has
    /// them, which move as CSS's `box-shadow` does: each with the one in the same place in the other list.
    Shadows,
}

impl Kind {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "n" => Some(Self::Number),
            "c" => Some(Self::Color),
            "s" => Some(Self::Text),
            "h" => Some(Self::Shadows),
            _ => None,
        }
    }
}

/// How something speeds up and slows down, as CSS's timing functions say.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Easing {
    Linear,
    Bezier(f64, f64, f64, f64),
    /// So many even steps, and whether the first is at the start rather than the end of each.
    Steps(u32, bool),
}

impl Easing {
    fn parse(text: &str) -> Self {
        let text = text.trim();
        match text {
            "linear" => Self::Linear,
            "ease" => Self::Bezier(0.25, 0.1, 0.25, 1.0),
            "ease-in" => Self::Bezier(0.42, 0.0, 1.0, 1.0),
            "ease-out" => Self::Bezier(0.0, 0.0, 0.58, 1.0),
            "ease-in-out" => Self::Bezier(0.42, 0.0, 0.58, 1.0),
            "step-start" => Self::Steps(1, true),
            "step-end" => Self::Steps(1, false),
            _ => {
                let inside = |name: &str| {
                    text.strip_prefix(name)
                        .and_then(|rest| rest.trim().strip_prefix('('))
                        .and_then(|rest| rest.strip_suffix(')'))
                        .map(|rest| rest.split(',').map(str::trim).collect::<Vec<_>>())
                };
                if let Some(parts) = inside("cubic-bezier") {
                    let numbers: Vec<f64> =
                        parts.iter().filter_map(|part| part.parse().ok()).collect();
                    if let [x1, y1, x2, y2] = numbers[..] {
                        return Self::Bezier(x1.clamp(0.0, 1.0), y1, x2.clamp(0.0, 1.0), y2);
                    }
                }
                if let Some(parts) = inside("steps") {
                    if let Some(count) = parts.first().and_then(|count| count.parse::<u32>().ok()) {
                        let start = matches!(parts.get(1).copied(), Some("start" | "jump-start"));
                        return Self::Steps(count.max(1), start);
                    }
                }
                // What CSS does by itself.
                Self::Bezier(0.25, 0.1, 0.25, 1.0)
            }
        }
    }

    /// How far along something is, from 0 to 1, when it's this far through its time.
    fn at(self, time: f64) -> f64 {
        let time = time.clamp(0.0, 1.0);
        match self {
            Self::Linear => time,
            Self::Steps(count, start) => {
                let count = f64::from(count);
                let step = if start {
                    (time * count).floor() + 1.0
                } else {
                    (time * count).floor()
                };
                (step / count).min(1.0)
            }
            Self::Bezier(x1, y1, x2, y2) => {
                let curve = |a: f64, b: f64, t: f64| {
                    let u = 1.0 - t;
                    3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
                };
                // The point on the curve that's this far across, found by halving, which always gets there.
                let (mut low, mut high) = (0.0, 1.0);
                for _ in 0..40 {
                    let middle = (low + high) / 2.0;
                    if curve(x1, x2, middle) < time {
                        low = middle;
                    } else {
                        high = middle;
                    }
                }
                curve(y1, y2, (low + high) / 2.0)
            }
        }
    }
}

/// Somewhere between two values: `from` when `along` is 0, and `to` when it's 1. `None` if they can't be in between.
fn between(kind: Kind, from: &PropValue, to: &PropValue, along: f64) -> Option<PropValue> {
    let number = |value: &PropValue| match value {
        PropValue::Int(value) => Some(*value as f64),
        PropValue::Float(value) => Some(*value),
        _ => None,
    };
    match kind {
        Kind::Number => {
            if let (Some(a), Some(b)) = (number(from), number(to)) {
                return Some(PropValue::Float(a + (b - a) * along));
            }
            // A length that's a share of what it's in, as a Switch's thumb's `calc(100% - 16.5px)`, moves as CSS has it: its
            // share and its pixels each go their own way, and it's written as layout reads it, `"P% + Xpx"`.
            let ((share_a, pixels_a), (share_b, pixels_b)) = (length(from)?, length(to)?);
            let (share, pixels) = (
                share_a + (share_b - share_a) * along,
                pixels_a + (pixels_b - pixels_a) * along,
            );
            Some(PropValue::from(format!("{share}% + {pixels}px").as_str()))
        }
        Kind::Color => {
            let (PropValue::Int(a), PropValue::Int(b)) = (from, to) else {
                return None;
            };
            let (a, b) = (
                u32::try_from(*a).ok()?.to_be_bytes(),
                u32::try_from(*b).ok()?.to_be_bytes(),
            );
            // As CSS has it, colors mix with their alpha taken into account, so a clear color doesn't bring black in.
            let (alpha_a, alpha_b) = (f64::from(a[3]) / 255.0, f64::from(b[3]) / 255.0);
            let alpha = alpha_a + (alpha_b - alpha_a) * along;
            let mut mixed = [0u8; 4];
            for channel in 0..3 {
                let premultiplied = f64::from(a[channel]) * alpha_a
                    + (f64::from(b[channel]) * alpha_b - f64::from(a[channel]) * alpha_a) * along;
                mixed[channel] = if alpha > 0.0 {
                    (premultiplied / alpha).round().clamp(0.0, 255.0) as u8
                } else {
                    0
                };
            }
            mixed[3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            Some(PropValue::Int(i64::from(u32::from_be_bytes(mixed))))
        }
        Kind::Text => {
            let (PropValue::Str(a), PropValue::Str(b)) = (from, to) else {
                return None;
            };
            // What's worked out with `calc()` moves as what it comes to, as sonner's stacked toasts'
            // `translateY(calc(-1 * 14px * 1)) scale(calc(-1 * 1 * 0.05 + 1))` does.
            let (a, b) = (without_calcs(a.as_str()), without_calcs(b.as_str()));
            let (a, b) = (a.as_ref(), b.as_ref());
            // Nothing moved is the same moves with nothing in them, as CSS's `none` is for a transform.
            let (a, b) = match (a.trim() == "none", b.trim() == "none") {
                (true, false) => (still_like(b), b.to_owned()),
                (false, true) => (a.to_owned(), still_like(a)),
                _ => (a.to_owned(), b.to_owned()),
            };
            // A list of transform functions that starts the same as a longer one has the rest as moving nothing, as CSS has
            // it: a Slider's thumb goes from `translate(-50%, -50%)` to `translate(-50%, -50%) scale(1.05)`.
            text_between(&a, &b, along)
                .or_else(|| {
                    let (a, b) = padded(a, b);
                    text_between(&a, &b, along)
                })
                .map(|text| PropValue::from(text.as_str()))
        }
        Kind::Shadows => {
            let (PropValue::Str(a), PropValue::Str(b)) = (from, to) else {
                return None;
            };
            shadows_between(a.as_str(), b.as_str(), along).map(|text| PropValue::from(text.as_str()))
        }
    }
}

/// `text` with each `calc()` in it that comes to one number, with at most one unit, written as that number, as `calc(-1 * 14px)`
/// is `-14px`. One that mixes units, as `calc(100% - 16px)`, stays as it is.
fn without_calcs(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains("calc(") {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("calc(") {
        // Only `calc` itself, not the end of a longer name.
        let named = start > 0 && rest.as_bytes()[start - 1].is_ascii_alphanumeric();
        let open = start + "calc".len();
        let mut depth = 0;
        let mut end = None;
        for (at, c) in rest[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + at);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        out.push_str(&rest[..start]);
        let worked_out = if named { None } else { CalcReader { text: rest[open + 1..end].as_bytes(), at: 0 }.whole() };
        match worked_out {
            Some((value, unit)) => {
                let rounded = (value * 10_000.0).round() / 10_000.0;
                out.push_str(&format!("{rounded}{unit}"));
            }
            None => out.push_str(&rest[start..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out)
}

/// Reads what's in a `calc()`: numbers with a unit or none, `+ - * /` and brackets, where only what's alike is added and only a
/// plain number multiplies or divides.
struct CalcReader<'a> {
    text: &'a [u8],
    at: usize,
}

impl CalcReader<'_> {
    fn whole(mut self) -> Option<(f64, String)> {
        let found = self.sum()?;
        self.spaces();
        (self.at == self.text.len() && found.0.is_finite()).then_some(found)
    }

    fn spaces(&mut self) {
        while self.text.get(self.at).is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.at += 1;
        }
    }

    fn sum(&mut self) -> Option<(f64, String)> {
        let (mut value, mut unit) = self.product()?;
        loop {
            self.spaces();
            let sign = match self.text.get(self.at) {
                Some(b'+') => 1.0,
                Some(b'-') => -1.0,
                _ => return Some((value, unit)),
            };
            self.at += 1;
            let (other, other_unit) = self.product()?;
            if other_unit != unit {
                // A plain 0 added to a length is that length.
                if other == 0.0 && other_unit.is_empty() {
                    continue;
                }
                if value == 0.0 && unit.is_empty() {
                    unit = other_unit;
                } else {
                    return None;
                }
            }
            value += sign * other;
        }
    }

    fn product(&mut self) -> Option<(f64, String)> {
        let (mut value, mut unit) = self.single()?;
        loop {
            self.spaces();
            let times = match self.text.get(self.at) {
                Some(b'*') => true,
                Some(b'/') => false,
                _ => return Some((value, unit)),
            };
            self.at += 1;
            let (other, other_unit) = self.single()?;
            if times {
                match (unit.is_empty(), other_unit.is_empty()) {
                    (_, true) => {}
                    (true, false) => unit = other_unit,
                    (false, false) => return None,
                }
                value *= other;
            } else {
                if !other_unit.is_empty() || other == 0.0 {
                    return None;
                }
                value /= other;
            }
        }
    }

    fn single(&mut self) -> Option<(f64, String)> {
        self.spaces();
        // A `calc()` in a `calc()` is brackets, as sonner's `--lift-amount: calc(var(--lift) * var(--gap))` in its `--y` is.
        if self.text[self.at..].starts_with(b"calc(") {
            self.at += "calc".len();
        }
        match *self.text.get(self.at)? {
            b'(' => {
                self.at += 1;
                let found = self.sum()?;
                self.spaces();
                if self.text.get(self.at) != Some(&b')') {
                    return None;
                }
                self.at += 1;
                Some(found)
            }
            b'-' if self.text.get(self.at + 1) == Some(&b'(') => {
                self.at += 1;
                let (value, unit) = self.single()?;
                Some((-value, unit))
            }
            _ => {
                let start = self.at;
                if matches!(self.text.get(self.at), Some(b'-' | b'+')) {
                    self.at += 1;
                }
                while self.text.get(self.at).is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.') {
                    self.at += 1;
                }
                let value = std::str::from_utf8(&self.text[start..self.at]).ok()?.parse::<f64>().ok()?;
                let unit_start = self.at;
                while self.text.get(self.at).is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'%') {
                    self.at += 1;
                }
                Some((value, std::str::from_utf8(&self.text[unit_start..self.at]).ok()?.to_ascii_lowercase()))
            }
        }
    }
}

/// One shadow of a list, as `shadows` has it: x, y, blur and spread, its color as a number, and whether it's inside.
type OneShadow = ([f64; 4], i64, bool);

/// The shadows a list says, or `None` if it isn't one.
fn shadow_list(text: &str) -> Option<Vec<OneShadow>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split_whitespace();
            let mut number = || parts.next()?.parse::<f64>().ok();
            let sizes = [number()?, number()?, number()?, number()?];
            let color = parts.next()?.parse::<i64>().ok()?;
            Some((sizes, color, parts.next() == Some("inset")))
        })
        .collect()
}

/// Two lists of shadows made as long as each other, as CSS has them move: the shorter one gets clear shadows of no size at its
/// end, each inside if the other's is. `None` if they aren't lists.
fn shadow_pairs(from: &str, to: &str) -> Option<(Vec<OneShadow>, Vec<OneShadow>)> {
    let (mut a, mut b) = (shadow_list(from)?, shadow_list(to)?);
    while a.len() < b.len() {
        a.push(([0.0; 4], 0, b[a.len()].2));
    }
    while b.len() < a.len() {
        b.push(([0.0; 4], 0, a[b.len()].2));
    }
    Some((a, b))
}

/// A list of shadows partway from `from` to `to`, each shadow's sizes and color partway to the other's. A shadow inside
/// can't be partway to one that isn't, which makes the lists not move at all, as CSS has it.
fn shadows_between(from: &str, to: &str, along: f64) -> Option<String> {
    use std::fmt::Write as _;
    let (a, b) = shadow_pairs(from, to)?;
    let mut out = String::with_capacity(to.len() + 16);
    for ((sizes_a, color_a, inside), (sizes_b, color_b, inside_b)) in a.iter().zip(&b) {
        if inside != inside_b {
            return None;
        }
        let Some(PropValue::Int(color)) =
            between(Kind::Color, &PropValue::Int(*color_a), &PropValue::Int(*color_b), along)
        else {
            return None;
        };
        let size = |index: usize| {
            let value = sizes_a[index] + (sizes_b[index] - sizes_a[index]) * along;
            (value * 10_000.0).round() / 10_000.0
        };
        if !out.is_empty() {
            out.push('\n');
        }
        let _ = write!(
            out,
            "{} {} {} {} {color}{}",
            size(0),
            size(1),
            size(2).max(0.0),
            size(3),
            if *inside { " inset" } else { "" }
        );
    }
    Some(out)
}

/// The functions of a transform, as `["translate(-50%, -50%)", "scale(1.05)"]`, or `None` if it isn't only functions.
fn functions_of(text: &str) -> Option<Vec<&str>> {
    let mut out = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let open = rest.find('(')?;
        let name = &rest[..open];
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return None;
        }
        let mut depth = 0;
        let mut end = None;
        for (at, c) in rest.char_indices().skip(open) {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(at + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end?;
        out.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    Some(out)
}

/// `a` and `b` with the shorter of two lists of transform functions made as long as the other, with what it's missing moving
/// nothing, if each function it has is the same as the other's in the same place. Otherwise they're as they were.
fn padded(a: String, b: String) -> (String, String) {
    let name = |function: &str| function[..function.find('(').unwrap_or(0)].to_owned();
    let (Some(first), Some(second)) = (functions_of(&a), functions_of(&b)) else {
        return (a, b);
    };
    if first.len() == second.len() {
        return (a, b);
    }
    let (short, long) = if first.len() < second.len() { (&first, &second) } else { (&second, &first) };
    if short.iter().zip(long.iter()).any(|(x, y)| name(x) != name(y)) {
        return (a, b);
    }
    let mut grown = short.join(" ");
    for extra in &long[short.len()..] {
        grown.push(' ');
        grown.push_str(&still_like(extra));
    }
    let long = long.join(" ");
    if first.len() < second.len() { (grown, long) } else { (long, grown) }
}

/// A length as `(share, pixels)`, from a number of pixels, `"40%"`, `"12px"` or `"100% + -16.5px"`, as layout writes them.
fn length(value: &PropValue) -> Option<(f64, f64)> {
    let pixels = |text: &str| text.trim().strip_suffix("px").unwrap_or(text.trim()).trim().parse::<f64>().ok();
    match value {
        PropValue::Int(value) => Some((0.0, *value as f64)),
        PropValue::Float(value) => Some((0.0, *value)),
        PropValue::Str(text) => {
            let text = text.as_str().trim();
            if let Some((share, rest)) = text.split_once("% +") {
                Some((share.trim().parse().ok()?, pixels(rest)?))
            } else if let Some(share) = text.strip_suffix('%') {
                Some((share.trim().parse().ok()?, 0.0))
            } else {
                Some((0.0, pixels(text)?))
            }
        }
        PropValue::Bool(_) => None,
    }
}

/// The same transform functions as `text`, moving nothing: no distance, no turn, and a scale of 1.
fn still_like(text: &str) -> String {
    let (words, numbers) = parts_of(text);
    let mut out = String::with_capacity(text.len());
    let mut function = String::new();
    let mut argument = 0;
    for (index, word) in words.iter().enumerate() {
        out.push_str(word);
        if let Some(open) = word.rfind('(') {
            function = word[..open]
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            argument = 0;
        }
        if index < numbers.len() {
            let still = if function.starts_with("scale") {
                1.0
            } else if function == "matrix" {
                [1.0, 0.0, 0.0, 1.0, 0.0, 0.0][argument.min(5)]
            } else {
                0.0
            };
            out.push_str(&format!("{still}"));
            argument += 1;
        }
    }
    out
}

/// What a prop is when nothing says, as CSS has it, for moving from or to: no transform, fully there, clear, and no margin or
/// padding. A Switch's label moves its `margin-inline-end` from nothing as its `margin-inline-start` goes to 0, and the two
/// together keep the track as wide as it was; with the one that comes and goes jumping, the track grew and shrank.
fn when_unsaid(kind: Kind, prop: Symbol) -> Option<PropValue> {
    let name = prop.as_str();
    match kind {
        Kind::Text if name == "transform" => Some(PropValue::from("none")),
        // No shadows.
        Kind::Shadows => Some(PropValue::from("")),
        Kind::Color => Some(PropValue::Int(0)),
        Kind::Number if name == "opacity" => Some(PropValue::Float(1.0)),
        Kind::Number if name.starts_with("margin_") || name.starts_with("padding_") => {
            Some(PropValue::Float(0.0))
        }
        _ => None,
    }
}

/// The parts of some text: what isn't a number, and the numbers between, each with how it's written.
fn parts_of(text: &str) -> (Vec<&str>, Vec<(f64, &str)>) {
    let mut words = Vec::new();
    let mut numbers = Vec::new();
    let bytes = text.as_bytes();
    let (mut at, mut word_start) = (0, 0);
    let is_word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    while at < bytes.len() {
        // A color, as `#0000ffff`, is one word, whatever's in it.
        if bytes[at] == b'#' {
            at += 1;
            while at < bytes.len() && is_word(bytes[at]) {
                at += 1;
            }
            continue;
        }
        let digit_at = |place: usize| {
            bytes
                .get(place)
                .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
        };
        let starts = (bytes[at].is_ascii_digit()
            || (matches!(bytes[at], b'-' | b'+' | b'.') && digit_at(at + 1)))
            && match at.checked_sub(1).map(|before| bytes[before]) {
                None => true,
                // After a single letter, as SVG's `M0` and `L10` have it, but not in a longer word, as `rotate3d` is.
                Some(before) if before.is_ascii_alphabetic() => at < 2 || !is_word(bytes[at - 2]),
                Some(before) => !is_word(before),
            };
        if !starts {
            at += 1;
            continue;
        }
        let mut end = at + 1;
        while end < bytes.len() {
            let byte = bytes[end];
            let exponent = byte == b'e'
                && bytes
                    .get(end + 1)
                    .is_some_and(|next| next.is_ascii_digit() || *next == b'-' || *next == b'+');
            if byte.is_ascii_digit() || (byte == b'.' && !text[at..end].contains('.')) || exponent {
                end += if exponent && matches!(bytes[end + 1], b'-' | b'+') {
                    2
                } else {
                    1
                };
            } else {
                break;
            }
        }
        let Ok(number) = text[at..end].parse::<f64>() else {
            at = end;
            continue;
        };
        words.push(&text[word_start..at]);
        numbers.push((number, &text[at..end]));
        word_start = end;
        at = end;
    }
    words.push(&text[word_start..]);
    (words, numbers)
}

/// Text with each number partway from what it is in `from` to what it is in `to`, if the rest of them is the same.
fn text_between(from: &str, to: &str, along: f64) -> Option<String> {
    if from == to {
        return Some(to.to_owned());
    }
    let (words, a) = parts_of(from);
    let (other, b) = parts_of(to);
    if words.len() != other.len() || a.len() != b.len() {
        return None;
    }
    // A 0 with no unit is a length of 0 in CSS, so `translateY(100%)` moves to `translateY(0)` as to `translateY(0%)`, as
    // sonner's toasts slide in: the word after it takes the other's unit.
    let mut said = Vec::with_capacity(words.len());
    for (index, (mine, theirs)) in words.iter().zip(&other).enumerate() {
        if mine == theirs {
            said.push(*mine);
            continue;
        }
        let unit = |word: &str| word.len() - word.trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == '%').len();
        let (mine_unit, theirs_unit) = (unit(mine), unit(theirs));
        let zero = |numbers: &[(f64, &str)]| index > 0 && numbers.get(index - 1).is_some_and(|(value, _)| *value == 0.0);
        if mine[mine_unit..] != theirs[theirs_unit..] {
            return None;
        }
        if mine_unit == 0 && zero(&a) {
            said.push(*theirs);
        } else if theirs_unit == 0 && zero(&b) {
            said.push(*mine);
        } else {
            return None;
        }
    }
    let mut out = String::with_capacity(to.len() + 8);
    for (index, word) in said.iter().enumerate() {
        out.push_str(word);
        if let (Some((a, _)), Some((b, written))) = (a.get(index), b.get(index)) {
            if a == b {
                out.push_str(written);
            } else {
                let value = a + (b - a) * along;
                // Rounded to what can be seen, so the text doesn't grow long digits.
                let rounded = (value * 10_000.0).round() / 10_000.0;
                out.push_str(&format!("{rounded}"));
            }
        }
    }
    Some(out)
}

fn seconds(text: &str) -> Duration {
    Duration::from_secs_f64(
        text.parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(0.0),
    )
}

/// One prop that moves when it changes.
#[derive(Debug, Clone)]
struct TransitionRule<'a> {
    prop: Symbol,
    kind: Kind,
    duration: Duration,
    delay: Duration,
    easing: Easing,
    /// The CSS properties it stands for, with commas between, or nothing for the prop itself.
    names: &'a str,
}

fn transition_rules(text: &str) -> Vec<TransitionRule<'_>> {
    text.lines()
        .filter_map(|line| {
            let (line, names) = line.split_once('|').unwrap_or((line, ""));
            let mut parts = line.splitn(5, ' ');
            let prop = Symbol::new(parts.next()?.trim());
            let kind = Kind::parse(parts.next()?.trim())?;
            let duration = seconds(parts.next()?.trim());
            let delay = seconds(parts.next().unwrap_or("0").trim());
            let easing = Easing::parse(parts.next().unwrap_or("ease"));
            Some(TransitionRule {
                prop,
                kind,
                duration,
                delay,
                easing,
                names: names.trim(),
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

/// A set of keyframes and how they play.
#[derive(Debug)]
struct Plan {
    duration: Duration,
    delay: Duration,
    /// How many times it goes through, which can be a fraction, or never stop.
    times: f64,
    direction: Direction,
    /// Whether the first keyframe shows before it starts, and the last after it ends.
    fill_before: bool,
    fill_after: bool,
    easing: Easing,
    /// Each prop it moves, with its kind, and where it is at each keyframe that says.
    props: Vec<Track>,
}

/// One prop an animation moves: its name, its kind, and its value at each keyframe that says, by how far through that is.
type Track = (Symbol, Kind, Vec<(f64, PropValue)>);

impl Plan {
    fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        let mut head = lines.next()?.split_whitespace();
        let duration = seconds(head.next()?);
        let delay = seconds(head.next().unwrap_or("0"));
        let times = match head.next().unwrap_or("1") {
            "inf" | "infinite" => f64::INFINITY,
            count => count.parse::<f64>().ok().filter(|count| *count >= 0.0)?,
        };
        let direction = match head.next().unwrap_or("normal") {
            "reverse" => Direction::Reverse,
            "alternate" => Direction::Alternate,
            "alternate-reverse" => Direction::AlternateReverse,
            _ => Direction::Normal,
        };
        let fill = head.next().unwrap_or("none");
        let easing = Easing::parse(&head.collect::<Vec<_>>().join(" "));
        let mut props: Vec<Track> = Vec::new();
        for line in lines {
            let mut parts = line.splitn(4, ' ');
            let (Some(offset), Some(name), Some(kind), Some(value)) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let (Ok(offset), Some(kind)) = (offset.parse::<f64>(), Kind::parse(kind)) else {
                continue;
            };
            let value = match kind {
                Kind::Number => PropValue::Float(value.parse().ok()?),
                Kind::Color => PropValue::Int(value.parse().ok()?),
                Kind::Text => PropValue::from(value),
                // A line each, which are `;` between here.
                Kind::Shadows => PropValue::from(value.replace(';', "\n").as_str()),
            };
            let name = Symbol::new(name);
            match props.iter_mut().find(|(prop, _, _)| *prop == name) {
                Some((_, _, frames)) => frames.push((offset.clamp(0.0, 1.0), value)),
                None => props.push((name, kind, vec![(offset.clamp(0.0, 1.0), value)])),
            }
        }
        for (_, _, frames) in &mut props {
            frames.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        Some(Self {
            duration,
            delay,
            times,
            direction,
            fill_before: matches!(fill, "backwards" | "both"),
            fill_after: matches!(fill, "forwards" | "both"),
            easing,
            props,
        })
    }

    /// How far through one time it is, from 0 to 1, after `time` of playing, which way it's going included, or `None`
    /// once it's done. A plan that takes no time is done at once.
    fn progress(&self, time: Duration) -> Option<f64> {
        let length = self.duration.as_secs_f64();
        if length <= 0.0 {
            return None;
        }
        let through = time.as_secs_f64() / length;
        if through >= self.times {
            return None;
        }
        Some(self.directed(through))
    }

    /// Where it is this many times through, going the way it goes.
    fn directed(&self, through: f64) -> f64 {
        let round = through.floor();
        let part = through - round;
        if self.backwards_in(round as u64) {
            1.0 - part
        } else {
            part
        }
    }

    /// Whether this time through, counting from 0, goes from the last keyframe to the first.
    fn backwards_in(&self, round: u64) -> bool {
        match self.direction {
            Direction::Normal => false,
            Direction::Reverse => true,
            Direction::Alternate => round % 2 == 1,
            Direction::AlternateReverse => round % 2 == 0,
        }
    }

    /// Where it ends, once it's played as many times as it does: the end of the last time through, which is its start
    /// if that one went backwards.
    fn end(&self) -> f64 {
        if !self.times.is_finite() {
            return 1.0;
        }
        if self.times > 0.0 && self.times.fract() == 0.0 {
            return if self.backwards_in(self.times as u64 - 1) {
                0.0
            } else {
                1.0
            };
        }
        self.directed(self.times)
    }

    /// The value of each prop this far through, with `base` for what's at the start or end when no keyframe says.
    fn values_at(
        &self,
        along: f64,
        base: &[(Symbol, Option<PropValue>)],
    ) -> Vec<(Symbol, Option<PropValue>)> {
        self.props
            .iter()
            .map(|(name, kind, frames)| {
                let own = base
                    .iter()
                    .find(|(prop, _)| prop == name)
                    .and_then(|(_, value)| value.clone())
                    .or_else(|| when_unsaid(*kind, *name));
                // What isn't said at the start or the end is what the element says itself, as CSS has it.
                let mut all: Vec<(f64, Option<PropValue>)> = Vec::with_capacity(frames.len() + 2);
                if frames.first().is_none_or(|(offset, _)| *offset > 0.0) {
                    all.push((0.0, own.clone()));
                }
                all.extend(
                    frames
                        .iter()
                        .map(|(offset, value)| (*offset, Some(value.clone()))),
                );
                if frames.last().is_none_or(|(offset, _)| *offset < 1.0) {
                    all.push((1.0, own.clone()));
                }
                let next = all
                    .iter()
                    .position(|(offset, _)| *offset > along)
                    .unwrap_or(all.len() - 1)
                    .max(1);
                let ((start, from), (end, to)) = (&all[next - 1], &all[next]);
                let value = match (from, to) {
                    (Some(from), Some(to)) => {
                        let span = end - start;
                        let part = if span > 0.0 {
                            ((along - start) / span).clamp(0.0, 1.0)
                        } else {
                            1.0
                        };
                        let eased = self.easing.at(part);
                        between(*kind, from, to, eased).or_else(|| {
                            Some(if eased < 0.5 {
                                from.clone()
                            } else {
                                to.clone()
                            })
                        })
                    }
                    (from, to) => {
                        if along < (start + end) / 2.0 {
                            from.clone()
                        } else {
                            to.clone()
                        }
                    }
                };
                (*name, value)
            })
            .collect()
    }
}

/// Something that's moving now.
enum Moving {
    Transition {
        prop: Symbol,
        kind: Kind,
        from: PropValue,
        to: PropValue,
        started: Instant,
        delay: Duration,
        duration: Duration,
        easing: Easing,
        /// The CSS properties it stands for, as its rule says.
        names: StdArc<str>,
        /// CSS's reversing-adjusted start value (CSS Transitions 1, 3): where going back the way it came would end, which is where
        /// it started, or where what it turned back from started.
        reversing_from: PropValue,
        /// CSS's reversing shortening factor: how much of its time it takes, for one that's turned back part of the way.
        shortening: f64,
    },
    Animation {
        /// What the element said, which says whether it's the same animation when it says it again.
        said: StdArc<str>,
        plan: StdArc<Plan>,
        started: Instant,
        /// What the element says itself for each prop the animation moves, to go back to.
        base: Vec<(Symbol, Option<PropValue>)>,
    },
}

impl Place {
    /// Only unbounded, purely visual keyframes can stop requesting display work
    /// while outside the viewport. Finite motion keeps all of its deadlines.
    fn can_sleep_outside_view(&self) -> bool {
        match &self.moving {
            Moving::Animation { plan, .. } => {
                plan.times.is_infinite() && !plan.duration.is_zero() && !plan.props.is_empty()
                    && plan.props.iter().all(|(name, kind, _)| {
                        (*kind == Kind::Number && name.as_str() == "opacity")
                            || (*kind == Kind::Color && matches!(name.as_str(),
                                "background" | "color" | "border_color" | "border_top_color"
                                | "border_right_color" | "border_bottom_color" | "border_left_color"))
                    })
            }
            Moving::Transition { .. } => false,
        }
    }

    /// What this sets at `now`, and whether it's done by then, with whether what it's in is being moved, while it is, for
    /// it to be drawn as a browser draws what's being moved.
    fn values_at(&self, now: Instant) -> (Vec<(Symbol, Option<PropValue>)>, bool) {
        let (mut values, done) = self.moved_values_at(now);
        if self.moves_transform() {
            values.push((*TRANSFORM_MOVING, (!done).then_some(PropValue::Bool(true))));
        }
        (values, done)
    }

    /// How far along a transition is at `now`, as its timing function puts it (0 before it starts and 1 once it's done), with
    /// where going back would end and how much of its time it takes: what turning it back works out from.
    fn reversing_at(&self, now: Instant) -> Option<(f64, &PropValue, f64)> {
        let Moving::Transition { started, delay, duration, easing, reversing_from, shortening, .. } = &self.moving else {
            return None;
        };
        let time = now.saturating_duration_since(*started);
        let along = if time < *delay {
            0.0
        } else {
            let part = (time - *delay).as_secs_f64() / duration.as_secs_f64().max(1e-9);
            if part >= 1.0 { 1.0 } else { easing.at(part) }
        };
        Some((along, reversing_from, *shortening))
    }

    /// Whether this moves `transform`.
    fn moves_transform(&self) -> bool {
        self.kind_of(*TRANSFORM).is_some()
    }

    fn moved_values_at(&self, now: Instant) -> (Vec<(Symbol, Option<PropValue>)>, bool) {
        match &self.moving {
            Moving::Transition {
                prop,
                kind,
                from,
                to,
                started,
                delay,
                duration,
                easing,
                ..
            } => {
                let time = now.saturating_duration_since(*started);
                if time < *delay {
                    (vec![(*prop, Some(from.clone()))], false)
                } else {
                    let part = (time - *delay).as_secs_f64() / duration.as_secs_f64().max(1e-9);
                    if part >= 1.0 {
                        (vec![(*prop, Some(to.clone()))], true)
                    } else {
                        let value =
                            between(*kind, from, to, easing.at(part)).unwrap_or_else(|| to.clone());
                        (vec![(*prop, Some(value))], false)
                    }
                }
            }
            Moving::Animation {
                plan,
                started,
                base,
                ..
            } => {
                let time = now.saturating_duration_since(*started);
                if time < plan.delay {
                    if plan.fill_before {
                        (plan.values_at(plan.directed(0.0), base), false)
                    } else {
                        (base.clone(), false)
                    }
                } else {
                    match plan.progress(time - plan.delay) {
                        Some(along) => (plan.values_at(along, base), false),
                        None if plan.fill_after => (plan.values_at(plan.end(), base), true),
                        None => (base.clone(), true),
                    }
                }
            }
        }
    }

    /// What kind of value a prop that this moves has.
    fn kind_of(&self, name: Symbol) -> Option<Kind> {
        match &self.moving {
            Moving::Transition { prop, kind, .. } => (*prop == name).then_some(*kind),
            Moving::Animation { plan, .. } => plan
                .props
                .iter()
                .find(|(prop, _, _)| *prop == name)
                .map(|(_, kind, _)| *kind),
        }
    }
}

/// Whether two values of a prop would be drawn the same: a sixteenth of a pixel apart, a fiftieth of a degree, a
/// thousandth of a scale, half a step of the way to clear, or the same color.
fn looks_the_same(kind: Kind, name: Symbol, a: &PropValue, b: &PropValue) -> bool {
    let close = if name.as_str() == "opacity" {
        0.5 / 255.0
    } else {
        1.0 / 16.0
    };
    let number = |value: &PropValue| match value {
        PropValue::Int(value) => Some(*value as f64),
        PropValue::Float(value) => Some(*value),
        _ => None,
    };
    match kind {
        Kind::Number => match (number(a), number(b)) {
            (Some(a), Some(b)) => (a - b).abs() < close,
            // Lengths with a share in them are the same if they say the same: how big a share is takes knowing what it's of.
            _ => matches!((length(a), length(b)), (Some((share_a, a)), Some((share_b, b)))
                if (share_a - share_b).abs() < 0.001 && (a - b).abs() < close),
        },
        Kind::Color => a == b,
        Kind::Shadows => {
            let (PropValue::Str(a), PropValue::Str(b)) = (a, b) else {
                return false;
            };
            shadow_pairs(a.as_str(), b.as_str()).is_some_and(|(a, b)| {
                a.iter().zip(&b).all(|((sizes_a, color_a, inside_a), (sizes_b, color_b, inside_b))| {
                    color_a == color_b
                        && inside_a == inside_b
                        && sizes_a.iter().zip(sizes_b).all(|(a, b)| (a - b).abs() < close)
                })
            })
        }
        Kind::Text => {
            let (PropValue::Str(a), PropValue::Str(b)) = (a, b) else {
                return false;
            };
            let ((words, a), (other, b)) = (parts_of(a.as_str()), parts_of(b.as_str()));
            // How close is close enough goes by what each number is: what a shape's made of is in pixels, and so is
            // anything that says so, a turn is in degrees, and what's only a number, as a scale is, is a share.
            let in_pixels = name.as_str() == "shape_layers";
            words == other
                && a.len() == b.len()
                && a.iter()
                    .zip(&b)
                    .enumerate()
                    .all(|(index, ((a, _), (b, _)))| {
                        let unit = words.get(index + 1).copied().unwrap_or("");
                        let close = if in_pixels || unit.starts_with("px") {
                            1.0 / 16.0
                        } else if unit.starts_with("deg") {
                            0.02
                        } else {
                            0.001
                        };
                        (a - b).abs() < close
                    })
        }
    }
}

/// The prop that's moved by `transform`, and the one that says it's being moved.
static TRANSFORM: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("transform"));
static TRANSFORM_MOVING: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("transform_moving"));

/// Where something moving is, and what's there, to be sure it's still the same element.
struct Place {
    path: Vec<u32>,
    key: Option<Key>,
    role: Role,
    moving: Moving,
}

/// Everything that's moving, in a tree that a [`Runtime`](crate::Runtime) keeps.
#[derive(Default)]
pub(crate) struct Motions {
    places: Vec<Place>,
    /// What's changed in the tree since the last time anyone was told.
    kept: Vec<Patch>,
    /// What's finished moving, for what wants to hear about that: where it is, and `"transition"` with the prop that moved, or
    /// `"animation"`.
    pub(crate) ended: Vec<(Vec<u32>, Option<Key>, Text)>,
    /// What's started or stopped moving, as `ended` has it, held until everything that's changing has been drawn.
    pub(crate) held: Vec<(Vec<u32>, Option<Key>, Text)>,
}

/// What something offers to hear that it's finished moving, as CSS's `transitionend` and `animationend` are.
pub(crate) static MOTION_END: LazyLock<ActionKind> =
    LazyLock::new(|| ActionKind::Custom(Symbol::new("motion_end")));

impl Motions {
    pub(crate) fn is_empty(&self) -> bool {
        self.places.is_empty()
    }

    /// Holds on to what's changed, for the next time anyone's told.
    pub(crate) fn keep(&mut self, patches: Vec<Patch>) {
        self.kept.extend(patches);
    }

    /// Says what's changed since the last time.
    pub(crate) fn take_kept(&mut self, patches: &mut Vec<Patch>) {
        patches.append(&mut self.kept);
    }

    /// Sends everything that's moving straight to where it's going, as when things stop moving: what changes goes there,
    /// and an animation to where it leaves things once it's done.
    pub(crate) fn stop(&mut self, tree: &mut Node) {
        let mut patches = Vec::new();
        for place in std::mem::take(&mut self.places) {
            let mut values = match &place.moving {
                Moving::Transition { prop, to, .. } => vec![(*prop, Some(to.clone()))],
                Moving::Animation { plan, base, .. }
                    if plan.fill_after && plan.times.is_finite() =>
                {
                    plan.values_at(plan.end(), base)
                }
                Moving::Animation { base, .. } => base.clone(),
            };
            if place.moves_transform() {
                values.push((*TRANSFORM_MOVING, None));
            }
            if find(tree, &place.path)
                .is_some_and(|node| node.key == place.key && node.role == place.role)
            {
                set_at(tree, &place.path, &values, &mut patches);
            }
        }
        self.kept.extend(patches);
    }

    /// Sees to what's moving in a part of the tree that's just been made again, at `path`. `old` is what was shown there,
    /// moving props and all, and `new` is what it says now, which this changes to show what's moving.
    pub(crate) fn rendered(
        &mut self,
        path: &[u32],
        old: Option<&Node>,
        new: &mut Node,
        now: Instant,
        moving: bool,
    ) {
        if let Some(old) = old {
            self.rebase(path, old, new);
        }
        let mut at = path.to_vec();
        self.visit(&mut at, old, new, now, moving);
    }

    /// Keyed siblings can change position while the same CSS animation keeps
    /// running (Radix inserts toast focus proxies during its layout effects).
    /// Motion belongs to the matched node, not its former numeric child index.
    fn rebase(&mut self, prefix: &[u32], old: &Node, new: &Node) {
        if self.places.is_empty() && self.held.is_empty() && self.ended.is_empty() {
            return;
        }
        let mut matches = std::collections::HashMap::default();
        let mut moved = |path: &mut Vec<u32>| {
            let Some(relative) = path.strip_prefix(prefix) else { return true; };
            let Some(next) = matched_path(old, new, relative, &mut matches) else { return false; };
            path.truncate(prefix.len());
            path.extend(next);
            true
        };
        self.places.retain_mut(|place| moved(&mut place.path));
        self.held.retain_mut(|(path, _, _)| moved(path));
        self.ended.retain_mut(|(path, _, _)| moved(path));
    }

    /// Goes through `new` and everything in it that isn't `old`'s, top down, with a list of where it's got to in each
    /// level rather than a call for each, so that a tree of any depth takes no more of the stack than a shallow one.
    fn visit(
        &mut self,
        path: &mut Vec<u32>,
        old: Option<&Node>,
        new: &mut Node,
        now: Instant,
        moving: bool,
    ) {
        let Some(root) = self.enter(path, old, new, now, moving) else {
            return;
        };
        let mut levels = vec![root];
        while let Some(level) = levels.last_mut() {
            if level.next < level.node.children.len() {
                let index = level.next;
                level.next += 1;
                let child = level.node.children[index].clone();
                let old_child = level.matched[index]
                    .and_then(|from| level.old.as_ref().map(|old| old.children[from].clone()));
                if old_child.as_ref().is_some_and(|old_child| Arc::ptr_eq(old_child, &child)) {
                    continue;
                }
                let mut shown = child.clone();
                path.push(index as u32);
                match self.enter(path, old_child.as_ref(), &mut shown, now, moving) {
                    Some(inner) => {
                        // `inner` has it, so letting go of this one leaves it as shared as it was.
                        drop(shown);
                        levels.push(inner);
                    }
                    None => {
                        path.pop();
                        if !Arc::ptr_eq(&shown, &child) {
                            level.replaced.push((index, shown));
                        }
                    }
                }
                continue;
            }
            let Some(Level { mut node, replaced, .. }) = levels.pop() else {
                break;
            };
            let Some(parent) = levels.last_mut() else {
                // The top: `new` itself, which `node` was only another hold on, let go of first so that it's changed in place
                // if nothing else has it.
                drop(node);
                put_back(new, replaced);
                return;
            };
            put_back(&mut node, replaced);
            path.pop();
            let index = parent.next - 1;
            if !Arc::ptr_eq(&node, &parent.node.children[index]) {
                parent.replaced.push((index, node));
            }
        }
    }

    /// What [`visit`](Self::visit) does on getting to `new`, before what's in it: whatever in it is moving, and what its
    /// children are to be matched with, if it has any.
    fn enter(
        &mut self,
        path: &[u32],
        old: Option<&Node>,
        new: &mut Node,
        now: Instant,
        moving: bool,
    ) -> Option<Level> {
        if let Some(old) = old {
            if Arc::ptr_eq(old, new) {
                return None;
            }
        }
        let names = &*NAMES;
        let animation = new.props.get_symbol(names.animation).cloned();
        let transitions = new.props.get_symbol(names.transitions).cloned();
        if animation.is_some()
            || transitions.is_some()
            || self.places.iter().any(|place| place.path == *path)
        {
            self.element(
                path,
                old,
                new,
                animation.as_ref(),
                transitions.as_ref(),
                now,
                moving,
            );
        }
        if new.children.is_empty() {
            return None;
        }
        let matched = match old {
            Some(old) if !old.children.is_empty() => match_children(&old.children, &new.children),
            _ => vec![None; new.children.len()],
        };
        Some(Level {
            old: old.cloned(),
            node: new.clone(),
            matched,
            next: 0,
            replaced: Vec::new(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn element(
        &mut self,
        path: &[u32],
        old: Option<&Node>,
        new: &mut Node,
        animation: Option<&PropValue>,
        transitions: Option<&PropValue>,
        now: Instant,
        moving: bool,
    ) {
        // What was moving here belongs to something else if that's what's here now.
        let (key, role) = (new.key.clone(), new.role);
        self.places
            .retain(|place| place.path != path || (place.key == key && place.role == role));
        let mut animated: Vec<Symbol> = Vec::new();
        // An animation, which goes over what moves when it changes.
        let said = match animation {
            Some(PropValue::Str(text)) => Some(text.as_str()),
            _ => None,
        };
        let existing = self.places.iter().position(|place| {
            place.path == path && matches!(&place.moving, Moving::Animation { .. })
        });
        match (said, existing) {
            (Some(said), Some(index)) if matches!(&self.places[index].moving, Moving::Animation { said: was, .. } if &**was == said) =>
            {
                // Still playing: what it says now is what it goes back to, and what's shown is where it's got to.
                let Moving::Animation { plan, base, .. } = &mut self.places[index].moving else {
                    unreachable!("it was just found to be one");
                };
                for (name, value) in base.iter_mut() {
                    *value = new.props.get_symbol(*name).cloned();
                    animated.push(*name);
                }
                let element = Arc::make_mut(new);
                for (name, _, _) in &plan.props {
                    match old.and_then(|old| old.props.get_symbol(*name)) {
                        Some(value) => element.props.insert(*name, value.clone()),
                        None => {
                            element.props.remove(name.as_str());
                        }
                    }
                }
            }
            // The same animation as was said before, which has played to its end: it isn't played again because what it's on was
            // made again (a drawer re-rendered as it's pressed), as CSS has it, where only a change to the animation starts it
            // (CSS Animations 1, 3). It stays where it left things, if it says it does.
            (Some(said), None)
                if old.and_then(|old| old.props.get_symbol(NAMES.animation)).is_some_and(
                    |was| matches!(was, PropValue::Str(was) if was.as_str() == said),
                ) =>
            {
                if let Some(plan) = Plan::parse(said) {
                    let base: Vec<(Symbol, Option<PropValue>)> = plan
                        .props
                        .iter()
                        .map(|(name, _, _)| (*name, new.props.get_symbol(*name).cloned()))
                        .collect();
                    animated.extend(base.iter().map(|(name, _)| *name));
                    if plan.fill_after && plan.times.is_finite() {
                        let shown = plan.values_at(plan.end(), &base);
                        set_all(Arc::make_mut(new), &shown);
                    }
                }
            }
            (Some(said), _) => {
                if let Some(index) = existing {
                    self.places.remove(index);
                }
                if let Some(plan) = Plan::parse(said) {
                    let base: Vec<(Symbol, Option<PropValue>)> = plan
                        .props
                        .iter()
                        .map(|(name, _, _)| (*name, new.props.get_symbol(*name).cloned()))
                        .collect();
                    animated.extend(base.iter().map(|(name, _)| *name));
                    let plan = StdArc::new(plan);
                    let shown = if moving {
                        let shown = if plan.delay.is_zero() || plan.fill_before {
                            let start = plan.directed(0.0);
                            Some(plan.values_at(start, &base))
                        } else {
                            None
                        };
                        self.places.push(Place {
                            path: path.to_vec(),
                            key: key.clone(),
                            role,
                            moving: Moving::Animation {
                                said: StdArc::from(said),
                                plan: plan.clone(),
                                started: now,
                                base: base.clone(),
                            },
                        });
                        shown
                    } else if plan.fill_after {
                        // With nothing moving, it's where it would be once it had finished.
                        Some(plan.values_at(plan.end(), &base))
                    } else {
                        None
                    };
                    // With nothing moving, it's finished as soon as it starts, which what's waiting for that hears.
                    let started_now = old.and_then(|old| old.props.get_symbol(NAMES.animation))
                        != animation;
                    if !moving
                        && started_now
                        && plan.times.is_finite()
                        && new.actions.contains(*MOTION_END)
                    {
                        self.ended
                            .push((path.to_vec(), key.clone(), Text::from("animation")));
                    }
                    if let Some(shown) = shown {
                        set_all(Arc::make_mut(new), &shown);
                    }
                }
            }
            (None, Some(index)) => {
                self.places.remove(index);
            }
            (None, None) => {}
        }
        // What moves when it changes.
        let rules = match transitions {
            Some(PropValue::Str(text)) => transition_rules(text.as_str()),
            _ => Vec::new(),
        };
        let mut changes: Vec<(Symbol, PropValue)> = Vec::new();
        // What's told it's stopped and started, by the CSS properties each stands for, told once each, after this.
        let mut cancelled: Vec<String> = Vec::new();
        let mut started: Vec<String> = Vec::new();
        for rule in &rules {
            if animated.contains(&rule.prop) {
                continue;
            }
            let target = new
                .props
                .get_symbol(rule.prop)
                .cloned()
                .or_else(|| when_unsaid(rule.kind, rule.prop));
            let shown = old.and_then(|old| old.props.get_symbol(rule.prop)).cloned();
            // What wasn't there before only moves from what it is when nothing says, if that's known.
            let shown = match (&shown, old) {
                (None, Some(_)) => when_unsaid(rule.kind, rule.prop),
                _ => shown,
            };
            let existing = self.places.iter().position(|place| {
                place.path == path && matches!(&place.moving, Moving::Transition { prop, .. } if *prop == rule.prop)
            });
            // What's on its way is where it's got to by now, as CSS has a running transition's value, rather than where it was
            // last drawn: the keyboard leaving a button before a frame was drawn turns its ring back from where it had got to.
            let shown = match existing {
                Some(index) if moving => self.places[index]
                    .moved_values_at(now)
                    .0
                    .into_iter()
                    .find(|(name, _)| *name == rule.prop)
                    .and_then(|(_, value)| value)
                    .or(shown),
                _ => shown,
            };
            let (Some(target), Some(shown)) = (target, shown) else {
                if let Some(index) = existing {
                    self.places.remove(index);
                }
                continue;
            };
            if !moving || rule.duration.is_zero() {
                if let Some(index) = existing {
                    // What's on its way to where it's going still goes there: as CSS has it, only a new place to go, or the
                    // prop no longer being said to move, stops it, and not how long a new one would take. Mantine's
                    // FloatingIndicator sets `transition-duration: 0ms` as soon as it's moving.
                    if moving
                        && matches!(&self.places[index].moving, Moving::Transition { to, .. } if *to == target)
                    {
                        changes.push((rule.prop, shown));
                        continue;
                    }
                    self.places.remove(index);
                }
                // With nothing moving, what would have moved is where it's going at once, and that's the end of it.
                if !moving
                    && !rule.duration.is_zero()
                    && shown != target
                    && new.actions.contains(*MOTION_END)
                    && between(rule.kind, &shown, &target, 0.5).is_some()
                {
                    for name in css_names(rule.prop, rule.names) {
                        self.ended
                            .push((path.to_vec(), key.clone(), Text::from(format!("transition {name}"))));
                    }
                }
                continue;
            }
            let heard = new.actions.contains(*MOTION_END);
            // Turning back the way it was going takes as long as it took to get here, as CSS has it (CSS Transitions 1, 3:
            // the reversing shortening factor), and it can go back again the same way.
            let mut reversing_from = shown.clone();
            let mut shortening = 1.0;
            if let Some(index) = existing {
                if matches!(&self.places[index].moving, Moving::Transition { to, .. } if *to == target)
                {
                    // On its way there already.
                    changes.push((rule.prop, shown));
                    continue;
                }
                if let Some((along, back_to, was_shortening)) = self.places[index].reversing_at(now) {
                    if *back_to == target {
                        shortening = (along * was_shortening + 1.0 - was_shortening).abs().clamp(0.0, 1.0);
                        if let Moving::Transition { to, .. } = &self.places[index].moving {
                            reversing_from = to.clone();
                        }
                    }
                }
                let was = self.places.remove(index);
                // Somewhere else to go stops the way it was going, which what hears it is told, as a page's `transitioncancel`.
                if heard {
                    if let Moving::Transition { prop, names, .. } = &was.moving {
                        cancelled.extend(css_names(*prop, names).map(str::to_owned));
                    }
                }
            }
            if shown == target || between(rule.kind, &shown, &target, 0.5).is_none() {
                continue;
            }
            // What hears it is told it's started, as a page's `transitionrun` and `transitionstart` say, along with what ends.
            if heard {
                started.extend(css_names(rule.prop, rule.names).map(str::to_owned));
            }
            self.places.push(Place {
                path: path.to_vec(),
                key: key.clone(),
                role,
                moving: Moving::Transition {
                    prop: rule.prop,
                    kind: rule.kind,
                    from: shown.clone(),
                    to: target,
                    started: now,
                    delay: rule.delay,
                    duration: rule.duration.mul_f64(shortening),
                    easing: rule.easing,
                    names: StdArc::from(rule.names),
                    reversing_from,
                    shortening,
                },
            });
            changes.push((rule.prop, shown));
        }
        // Each CSS property once, even where two props stand for it, as `border_color` and `border_top_color` both do for
        // `border-top-color`, in the order a browser tells them: see `start_order`.
        cancelled.sort_by(|a, b| start_order(a).cmp(&start_order(b)).then_with(|| a.cmp(b)));
        cancelled.dedup();
        for name in cancelled {
            self.ended
                .push((path.to_vec(), key.clone(), Text::from(format!("transitioncancel {name}"))));
        }
        started.sort_by(|a, b| start_order(a).cmp(&start_order(b)).then_with(|| a.cmp(b)));
        started.dedup();
        for name in started {
            self.ended
                .push((path.to_vec(), key.clone(), Text::from(format!("transitionstart {name}"))));
        }
        // What isn't said to move anymore, but was moving, goes where it's going.
        self.places.retain(|place| {
            place.path != path
                || match &place.moving {
                    Moving::Transition { prop, .. } => rules.iter().any(|rule| rule.prop == *prop),
                    Moving::Animation { .. } => true,
                }
        });
        if !changes.is_empty() {
            let element = Arc::make_mut(new);
            for (name, value) in changes {
                element.props.insert(name, value);
            }
        }
    }

    /// How long until anything that's moving looks any different from how it's shown in `tree`, which is when it next
    /// needs drawing, as something that creeps along slowly only does every so often. It's at most a quarter of a second,
    /// and nothing at all for something that's done, or isn't where it was.
    pub(crate) fn next_change(&self, tree: &Node, now: Instant) -> Duration {
        self.next_change_in_view(tree, now, |_, _| true).unwrap_or(Duration::from_millis(250))
    }

    pub(crate) fn next_change_in_view(&self, tree: &Node, now: Instant, visible: impl Fn(&[u32], &Node) -> bool) -> Option<Duration> {
        let mut soonest = Duration::from_millis(250);
        let mut active = false;
        for place in &self.places {
            let Some(node) = find(tree, &place.path) else {
                return Some(Duration::ZERO);
            };
            if place.can_sleep_outside_view() && !visible(&place.path, node) {
                continue;
            }
            active = true;
            let mut wait = Duration::from_millis(1);
            while wait < soonest {
                let (values, done) = place.values_at(now + wait);
                let different = values.iter().any(|(name, value)| {
                    match (node.props.get_symbol(*name), value) {
                        (Some(shown), Some(value)) => place
                            .kind_of(*name)
                            .is_none_or(|kind| !looks_the_same(kind, *name, shown, value)),
                        (None, None) => false,
                        _ => true,
                    }
                });
                if done || different {
                    soonest = wait;
                    break;
                }
                wait *= 2;
            }
        }
        active.then_some(soonest)
    }

    /// Moves everything along to `now`, in `tree`, saying what changed in `patches`.
    pub(crate) fn advance(&mut self, tree: &mut Node, now: Instant, patches: &mut Vec<Patch>) {
        let mut finished = Vec::new();
        // What's ended, told in the order a page tells it, as CSS's composite order has it: by when each was due to end, then
        // by when it started (which change set it off), then where it is in the tree, and then by the CSS property's name.
        let mut told: Vec<(Instant, Instant, Vec<u32>, Option<Key>, Text)> = Vec::new();
        for (index, place) in self.places.iter().enumerate() {
            let Some(node) = find(tree, &place.path) else {
                finished.push(index);
                continue;
            };
            if node.key != place.key || node.role != place.role {
                finished.push(index);
                continue;
            }
            let (values, done) = place.values_at(now);
            // What hasn't moved enough to be seen yet isn't drawn again, as a ring that creeps a pixel a second would otherwise
            // be, at every frame. Where it ends is always where it ends.
            let values: Vec<(Symbol, Option<PropValue>)> = if done {
                values
            } else {
                values
                    .into_iter()
                    .filter(|(name, value)| {
                        let kind = place.kind_of(*name);
                        match (node.props.get_symbol(*name), value, kind) {
                            (Some(shown), Some(value), Some(kind)) => {
                                !looks_the_same(kind, *name, shown, value)
                            }
                            _ => true,
                        }
                    })
                    .collect()
            };
            let heard = done && node.actions.contains(*MOTION_END);
            set_at(tree, &place.path, &values, patches);
            if done {
                finished.push(index);
            }
            if heard {
                match &place.moving {
                    Moving::Transition {
                        prop,
                        names,
                        started,
                        delay,
                        duration,
                        ..
                    } => {
                        for name in css_names(*prop, names) {
                            told.push((
                                *started + *delay + *duration,
                                *started,
                                place.path.clone(),
                                place.key.clone(),
                                Text::from(format!("transition {name}")),
                            ));
                        }
                    }
                    Moving::Animation { started, plan, .. } => {
                        let length = plan.duration.as_secs_f64() * plan.times;
                        let due = Duration::try_from_secs_f64(length).map_or(now, |length| *started + plan.delay + length);
                        told.push((
                            due.min(now),
                            *started,
                            place.path.clone(),
                            place.key.clone(),
                            Text::from("animation"),
                        ));
                    }
                }
            }
        }
        for index in finished.into_iter().rev() {
            self.places.remove(index);
        }
        if told.len() > 1 {
            told.sort_by(|a, b| {
                (a.0, a.1, &a.2)
                    .cmp(&(b.0, b.1, &b.2))
                    .then_with(|| a.4.as_str().cmp(b.4.as_str()))
            });
        }
        self.ended
            .extend(told.into_iter().map(|(_, _, path, key, what)| (path, key, what)));
    }
}

/// The CSS properties a transition of `prop` stands for, as its rule says, or the prop itself. `-` is none: what moves with
/// another prop that says what's moving, as what's drawn of an outline does with CSS's `outline-width`.
fn css_names<'a>(prop: Symbol, names: &'a str) -> impl Iterator<Item = &'a str> {
    let own = if names.is_empty() { Some(prop.as_str()) } else { None };
    names
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty() && *name != "-")
        .chain(own)
}

/// Where Chromium (and so Edge) tells a CSS property's `transitionrun`, `transitionstart` and `transitioncancel` among those one change sets off on
/// one element: not CSS's composite order (by name), which it tells their ends in, but the order of its own table of new
/// transitions, which is the same every time for the same properties. These are those seen: a shadcn/ui button's
/// `transition-all` focus ring sets off `border-top-color`, `outline-width`, `border-bottom-color`, `border-left-color`,
/// `box-shadow` and `border-right-color` in that order, and its hover `background-color` and `color` in that one. What isn't
/// here comes after, by name.
fn start_order(name: &str) -> usize {
    const SEEN: [&str; 8] = [
        "background-color",
        "color",
        "border-top-color",
        "outline-width",
        "border-bottom-color",
        "border-left-color",
        "box-shadow",
        "border-right-color",
    ];
    SEEN.iter().position(|seen| *seen == name).unwrap_or(SEEN.len())
}

/// Pairs each new child with the old one it continues, as diffing does: by key, and the ones without a key in order.
/// A node [`Motions::visit`] is going through the children of.
struct Level {
    old: Option<Node>,
    node: Node,
    /// Which of `old`'s children each of `node`'s is.
    matched: Vec<Option<usize>>,
    /// The next child to go to.
    next: usize,
    /// The children that came out changed, to be put in once all of them have been seen to.
    replaced: Vec<(usize, Node)>,
}

/// Puts what's changed of a node's children back into it, copying it only if something has.
fn put_back(node: &mut Node, replaced: Vec<(usize, Node)>) {
    if !replaced.is_empty() {
        let element = Arc::make_mut(node);
        for (index, shown) in replaced {
            element.children[index] = shown;
        }
    }
}

fn match_children(old: &[Node], new: &[Node]) -> Vec<Option<usize>> {
    let mut unkeyed = old
        .iter()
        .enumerate()
        .filter(|(_, child)| child.key.is_none())
        .map(|(index, _)| index);
    // Each key's old children, in order, latest first, so that a long list is matched in one pass over it.
    let mut keyed: std::collections::HashMap<&Key, Vec<usize>, foldhash::fast::RandomState> =
        std::collections::HashMap::default();
    for (index, child) in old.iter().enumerate().rev() {
        if let Some(key) = &child.key {
            keyed.entry(key).or_default().push(index);
        }
    }
    new.iter()
        .map(|child| {
            let found = match &child.key {
                Some(key) => keyed.get_mut(key).and_then(Vec::pop),
                None => unkeyed.next(),
            };
            found.filter(|&index| old[index].role == child.role)
        })
        .collect()
}

fn matched_path(
    mut old: &Node,
    mut new: &Node,
    relative: &[u32],
    cache: &mut std::collections::HashMap<Vec<u32>, Vec<Option<u32>>, foldhash::fast::RandomState>,
) -> Option<Vec<u32>> {
    if old.key != new.key || old.role != new.role {
        return None;
    }
    let mut from = Vec::with_capacity(relative.len());
    let mut to = Vec::with_capacity(relative.len());
    for (depth, &index) in relative.iter().enumerate() {
        if Arc::ptr_eq(old, new) {
            find(old, &relative[depth..])?;
            to.extend_from_slice(&relative[depth..]);
            return Some(to);
        }
        let mapping = cache.entry(from.clone()).or_insert_with(|| {
            // The common props-only update needs no per-child mapping allocation.
            // Cache that proof once per parent even when many motions share it.
            if old.children.len() == new.children.len()
                && old.children.iter().zip(&new.children)
                    .all(|(old, new)| old.key == new.key && old.role == new.role)
            {
                return Vec::new();
            }
            let mut inverse = vec![None; old.children.len()];
            for (next, before) in match_children(&old.children, &new.children).into_iter().enumerate() {
                if let Some(before) = before { inverse[before] = Some(next as u32); }
            }
            inverse
        });
        let next = if mapping.is_empty() { index } else { mapping.get(index as usize).copied().flatten()? };
        old = old.children.get(index as usize)?;
        new = new.children.get(next as usize)?;
        from.push(index);
        to.push(next);
    }
    Some(to)
}

pub(crate) fn find<'a>(tree: &'a Node, path: &[u32]) -> Option<&'a Node> {
    let mut node = tree;
    for &index in path {
        node = node.children.get(index as usize)?;
    }
    Some(node)
}

fn set_all(element: &mut rsui_core::Element, values: &[(Symbol, Option<PropValue>)]) {
    for (name, value) in values {
        match value {
            Some(value) => element.props.insert(*name, value.clone()),
            None => {
                element.props.remove(name.as_str());
            }
        }
    }
}

/// Sets props of the element at `path`, saying so in `patches`, where they're different.
fn set_at(
    tree: &mut Node,
    path: &[u32],
    values: &[(Symbol, Option<PropValue>)],
    patches: &mut Vec<Patch>,
) {
    let Some(node) = find(tree, path) else {
        return;
    };
    let changed: Vec<&(Symbol, Option<PropValue>)> = values
        .iter()
        .filter(|(name, value)| node.props.get_symbol(*name) != value.as_ref())
        .collect();
    if changed.is_empty() {
        return;
    }
    let mut slot = tree;
    for &index in path {
        slot = &mut Arc::make_mut(slot).children[index as usize];
    }
    let element = Arc::make_mut(slot);
    for (name, value) in changed {
        match value {
            Some(value) => {
                element.props.insert(*name, value.clone());
                patches.push(Patch::SetProp {
                    path: Path::from(path),
                    name: *name,
                    value: value.clone(),
                });
            }
            None => {
                element.props.remove(name.as_str());
                patches.push(Patch::RemoveProp {
                    path: Path::from(path),
                    name: *name,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_insertions_and_reordering_keep_motion_time_and_event_targets() {
        let animated = |key: &str| rsui_core::Element::new(Role::Group).key(key)
            .prop("animation", "1 0 1 normal none linear\n0 opacity n 0\n1 opacity n 1")
            .into_node();
        let now = Instant::now();
        let mut tree = rsui_core::Element::new(Role::Group).child(animated("a")).child(animated("b")).into_node();
        let mut motions = Motions::default();
        motions.rendered(&[], None, &mut tree, now, true);
        let mut patches = Vec::new();
        motions.advance(&mut tree, now + Duration::from_millis(100), &mut patches);
        motions.held.push((vec![0], tree.children[0].key.clone(), Text::from("animationstart")));
        let old = tree.clone();
        let mut next = rsui_core::Element::new(Role::Group)
            .child(rsui_core::Element::new(Role::Button).key("focus-proxy"))
            .child(animated("b")).child(animated("a")).into_node();
        motions.rendered(&[], Some(&old), &mut next, now + Duration::from_millis(100), true);
        assert_eq!(motions.places.len(), 2);
        assert_eq!(motions.held[0].0, vec![2], "pending events follow the same keyed node");
        motions.advance(&mut next, now + Duration::from_millis(500), &mut patches);
        for index in [1, 2] {
            assert_eq!(next.children[index].props.get("opacity"), Some(&PropValue::Float(0.5)), "reordering preserves start time");
        }
        let old = next;
        let mut removed = rsui_core::Element::new(Role::Group).child(animated("b")).into_node();
        motions.rendered(&[], Some(&old), &mut removed, now + Duration::from_millis(500), true);
        assert_eq!(motions.places.len(), 1);
        assert!(motions.held.is_empty(), "events for removed nodes are discarded");
    }

    #[test]
    fn motion_paths_are_matched_within_parent_scope_not_by_global_key() {
        let parent = |key: &str| rsui_core::Element::new(Role::Group).key(key)
            .child(rsui_core::Element::new(Role::Text).key("same-local-key")).into_node();
        let old = rsui_core::Element::new(Role::Group).child(parent("left")).child(parent("right")).into_node();
        let new = rsui_core::Element::new(Role::Group).child(parent("right")).child(parent("left")).into_node();
        let mut cache = std::collections::HashMap::default();
        assert_eq!(matched_path(&old, &new, &[0, 0], &mut cache), Some(vec![1, 0]));
        assert_eq!(matched_path(&old, &new, &[1, 0], &mut cache), Some(vec![0, 0]));
    }

    /// A transition turned back part of the way takes as long as it took to get there (CSS Transitions 1, 3: the reversing
    /// shortening factor), from where it had got to by the time of the change, and turned back again it takes the whole time.
    #[test]
    fn a_transition_turned_back_takes_as_long_as_it_took_to_get_there() {
        let element = |opacity: f64| -> Node {
            let mut element = rsui_core::Element::new(Role::Group);
            element.props.insert("opacity", PropValue::Float(opacity));
            element.props.insert("transitions", PropValue::from("opacity n 1 0 linear"));
            Arc::new(element)
        };
        let duration = |motions: &Motions| match &motions.places[0].moving {
            Moving::Transition { duration, .. } => *duration,
            Moving::Animation { .. } => unreachable!("only a transition is moving"),
        };
        let mut motions = Motions::default();
        let start = Instant::now();
        let shown = element(0.0);
        let mut there = element(1.0);
        motions.rendered(&[], Some(&shown), &mut there, start, true);
        assert_eq!(motions.places.len(), 1);
        assert_eq!(duration(&motions), Duration::from_secs(1));
        let mut back = element(0.0);
        motions.rendered(&[], Some(&there), &mut back, start + Duration::from_millis(250), true);
        assert_eq!(motions.places.len(), 1);
        assert_eq!(duration(&motions), Duration::from_millis(250));
        // Where it's got to, a quarter of the way.
        assert_eq!(back.props.get("opacity"), Some(&PropValue::Float(0.25)));
        // Turned back again, half way back: as far as it had come and back is the whole way again, as CSS works it out.
        let mut again = element(1.0);
        motions.rendered(&[], Some(&back), &mut again, start + Duration::from_millis(375), true);
        assert_eq!(duration(&motions), Duration::from_secs(1).mul_f64(0.5 * 0.25 + 1.0 - 0.25));
    }

    #[test]
    fn easing_goes_from_start_to_end_as_css_has_it() {
        assert_eq!(Easing::parse("linear").at(0.3), 0.3);
        let ease = Easing::parse("ease");
        assert!(ease.at(0.0).abs() < 1e-6 && (ease.at(1.0) - 1.0).abs() < 1e-6);
        // `ease` is most of the way there by halfway.
        assert!((ease.at(0.5) - 0.8024).abs() < 0.002);
        assert!((Easing::parse("ease-in").at(0.5) - 0.3153).abs() < 0.002);
        assert!((Easing::parse("cubic-bezier(0, 0, 1, 1)").at(0.25) - 0.25).abs() < 1e-6);
        assert_eq!(Easing::parse("steps(4)").at(0.3), 0.25);
        assert_eq!(Easing::parse("steps(4, start)").at(0.3), 0.5);
    }

    #[test]
    fn what_calc_comes_to_is_what_moves() {
        assert_eq!(without_calcs("translateY(calc(-1 * 14px * 1)) scale(calc(-1 * 1 * 0.05 + 1))"), "translateY(-14px) scale(0.95)");
        assert_eq!(without_calcs("translateY(calc(-1 * -100%))"), "translateY(100%)");
        assert_eq!(without_calcs("translateX(calc(100% - 16px))"), "translateX(calc(100% - 16px))");
        assert_eq!(without_calcs("translateY(calc((2 + 1) * 4px / 2))"), "translateY(6px)");
        assert_eq!(without_calcs("translateY(calc(calc(-1 * 14px) * 2))"), "translateY(-28px)");
        let collapsed = PropValue::from("translateY(calc(-1 * 14px * 1)) scale(calc(-1 * 1 * 0.05 + 1))");
        let expanded = PropValue::from("translateY(calc(-1 * 84px))");
        assert_eq!(
            between(Kind::Text, &collapsed, &expanded, 0.5),
            Some(PropValue::from("translateY(-49px) scale(0.975)"))
        );
    }

    #[test]
    fn a_zero_with_no_unit_moves_as_a_length_of_zero() {
        // sonner's toasts: `translateY(100%)` to `translateY(0)`, which CSS reads as `translateY(0%)`.
        assert_eq!(text_between("translateY(100%)", "translateY(0)", 0.25).as_deref(), Some("translateY(75%)"));
        assert_eq!(text_between("translateY(0)", "translateY(-20px)", 0.5).as_deref(), Some("translateY(-10px)"));
        // A number that isn't 0 with no unit is no length, and stays as it was.
        assert_eq!(text_between("translateY(5)", "translateY(20px)", 0.5), None);
    }

    #[test]
    fn shadows_move_as_lists_as_css_has_them() {
        // From none, a ring grows out of a clear one: red at half its alpha, half as far out.
        let red = 0xff00_00ff_u32;
        let half = between(Kind::Shadows, &PropValue::from(""), &PropValue::from(format!("0 0 0 3 {red}").as_str()), 0.5);
        assert_eq!(half, Some(PropValue::from(format!("0 0 0 1.5 {}", 0xff00_0080_u32).as_str())));
        // A shorter list is made as long as the other with clear shadows, and what's the same stays.
        let from = PropValue::from("0 1 2 0 128");
        let to = PropValue::from("0 1 2 0 128
0 0 0 4 0");
        assert_eq!(between(Kind::Shadows, &from, &to, 0.25), Some(PropValue::from("0 1 2 0 128
0 0 0 1 0")));
        // One inside can't be partway to one that isn't, so the lists don't move.
        let inside = PropValue::from("0 0 0 2 128 inset");
        assert_eq!(between(Kind::Shadows, &from, &inside, 0.5), None);
        assert!(looks_the_same(Kind::Shadows, Symbol::new("shadows"), &from, &PropValue::from("0 1.01 2 0 128")));
        assert!(!looks_the_same(Kind::Shadows, Symbol::new("shadows"), &from, &to));
    }

    #[test]
    fn transitions_tell_the_css_properties_they_stand_for() {
        let rules = transition_rules("border_color c 0.15 0 cubic-bezier(0.4, 0, 0.2, 1)|border-top-color,border-right-color
opacity n 0.2 0 ease");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].easing, Easing::Bezier(0.4, 0.0, 0.2, 1.0));
        let names: Vec<&str> = css_names(rules[0].prop, rules[0].names).collect();
        assert_eq!(names, ["border-top-color", "border-right-color"]);
        let own: Vec<&str> = css_names(rules[1].prop, rules[1].names).collect();
        assert_eq!(own, ["opacity"]);
        assert_eq!(css_names(Symbol::new("outline_width"), "-").count(), 0);
        let mut started = vec!["border-right-color", "box-shadow", "border-top-color", "zoom", "outline-width"];
        started.sort_by_key(|name| (start_order(name), *name));
        assert_eq!(started, ["border-top-color", "outline-width", "box-shadow", "border-right-color", "zoom"]);
    }

    #[test]
    fn a_shorter_transform_moves_as_if_it_had_the_rest_moving_nothing() {
        let from = PropValue::from("translate(-50%, -50%)");
        let to = PropValue::from("translate(-50%, -50%) scale(1.05)");
        let half = between(Kind::Text, &from, &to, 0.5);
        assert_eq!(half, Some(PropValue::from("translate(-50%, -50%) scale(1.025)")));
        let back = between(Kind::Text, &to, &from, 0.5);
        assert_eq!(back, Some(PropValue::from("translate(-50%, -50%) scale(1.025)")));
        // What isn't the same functions doesn't move.
        let other = PropValue::from("rotate(10deg) scale(2)");
        assert_eq!(between(Kind::Text, &from, &other, 0.5), None);
    }

    #[test]
    fn numbers_in_text_move_and_the_rest_stays() {
        assert_eq!(
            text_between(
                "translateY(10px) rotate(0deg)",
                "translateY(0px) rotate(90deg)",
                0.5
            )
            .as_deref(),
            Some("translateY(5px) rotate(45deg)")
        );
        assert_eq!(
            text_between("M0 0 L10 -4", "M0 0 L20 -8", 0.25).as_deref(),
            Some("M0 0 L12.5 -5")
        );
        // A different kind of thing can't be partway to it.
        assert_eq!(text_between("scale(1)", "rotate(1deg)", 0.5), None);
        // A color stays as it's written, and a number that's part of a word isn't a number.
        assert_eq!(
            text_between(
                "#0000ffff	1	M217 110a107 107",
                "#0000ffff	1	M217 110a107 107",
                0.5
            )
            .as_deref(),
            Some("#0000ffff	1	M217 110a107 107")
        );
        assert_eq!(
            text_between("#00ff00ff 0.50 x", "#00ff00ff 1 x", 0.5).as_deref(),
            Some("#00ff00ff 0.75 x")
        );
        // Numbers that are part of a word aren't numbers.
        assert_eq!(
            text_between("rotate3d(1, 0, 0, 10deg)", "rotate3d(1, 0, 0, 20deg)", 0.5).as_deref(),
            Some("rotate3d(1, 0, 0, 15deg)")
        );
    }

    #[test]
    fn a_length_with_a_share_in_it_moves_its_share_and_its_pixels() {
        // A Switch's thumb, from `2.5px` to `calc(100% - 16.5px)`, which layout has as `"100% + -16.5px"`.
        let (from, to) = (PropValue::Float(2.5), PropValue::from("100% + -16.5px"));
        assert_eq!(
            between(Kind::Number, &from, &to, 0.5),
            Some(PropValue::from("50% + -7px"))
        );
        assert_eq!(
            between(Kind::Number, &to, &from, 1.0),
            Some(PropValue::from("0% + 2.5px"))
        );
        assert_eq!(
            between(Kind::Number, &PropValue::from("40%"), &PropValue::from("12px"), 0.25),
            Some(PropValue::from("30% + 3px"))
        );
        // What isn't a length still can't be partway to one.
        assert_eq!(between(Kind::Number, &PropValue::from("auto"), &to, 0.5), None);
        let left = Symbol::new("left");
        assert!(looks_the_same(Kind::Number, left, &to, &PropValue::from("100% + -16.52px")));
        assert!(!looks_the_same(Kind::Number, left, &to, &PropValue::from("99% + -16.5px")));
    }

    #[test]
    fn nothing_moved_is_the_same_moves_with_nothing_in_them() {
        assert_eq!(
            still_like("translateY(20px) scale(0.5) rotate(45deg)"),
            "translateY(0px) scale(1) rotate(0deg)"
        );
        let from = PropValue::from("none");
        let to = PropValue::from("translateX(-10px)");
        assert_eq!(
            between(Kind::Text, &from, &to, 0.5),
            Some(PropValue::from("translateX(-5px)"))
        );
        assert_eq!(
            between(Kind::Text, &to, &from, 1.0),
            Some(PropValue::from("translateX(0px)"))
        );
    }

    #[test]
    fn what_has_hardly_moved_looks_the_same() {
        let opacity = Symbol::new("opacity");
        let width = Symbol::new("width");
        assert!(looks_the_same(
            Kind::Number,
            opacity,
            &PropValue::Float(0.5),
            &PropValue::Float(0.501)
        ));
        assert!(!looks_the_same(
            Kind::Number,
            opacity,
            &PropValue::Float(0.5),
            &PropValue::Float(0.51)
        ));
        assert!(looks_the_same(
            Kind::Number,
            width,
            &PropValue::Float(10.0),
            &PropValue::Float(10.05)
        ));
        let shape = Symbol::new("shape_layers");
        let transform = Symbol::new("transform");
        assert!(looks_the_same(
            Kind::Text,
            transform,
            &PropValue::from("translateX(1px) scale(1.5)"),
            &PropValue::from("translateX(1.05px) scale(1.5)")
        ));
        assert!(!looks_the_same(
            Kind::Text,
            transform,
            &PropValue::from("scale(1.5)"),
            &PropValue::from("scale(1.51)")
        ));
        let ring = |offset: &str| {
            PropValue::from(format!("#3b82f6ff\t6\tround\tM217 110\t\t674\t{offset}").as_str())
        };
        assert!(looks_the_same(
            Kind::Text,
            shape,
            &ring("4.5"),
            &ring("4.55")
        ));
        assert!(!looks_the_same(
            Kind::Text,
            shape,
            &ring("4.5"),
            &ring("4.6")
        ));
    }

    #[test]
    fn colors_mix_with_their_alpha() {
        let red = PropValue::Int(0xFF00_00FF);
        let clear = PropValue::Int(0x0000_0000);
        // Fading to clear stays red, as CSS has it, rather than going through dark red.
        assert_eq!(
            between(Kind::Color, &red, &clear, 0.5),
            Some(PropValue::Int(0xFF00_0080))
        );
        let blue = PropValue::Int(0x0000_FFFF);
        assert_eq!(
            between(Kind::Color, &red, &blue, 0.5),
            Some(PropValue::Int(0x8000_80FF))
        );
    }

    #[test]
    fn keyframes_say_where_each_prop_is() {
        let plan =
            Plan::parse("1 0 1 normal none linear\n0 opacity n 0\n0.5 opacity n 1\n0.5 width n 10")
                .unwrap();
        let base = [
            (Symbol::new("opacity"), Some(PropValue::Float(0.5))),
            (Symbol::new("width"), Some(PropValue::Float(30.0))),
        ];
        let at = |along: f64| -> Vec<Option<f64>> {
            plan.values_at(along, &base)
                .into_iter()
                .map(|(_, value)| match value {
                    Some(PropValue::Float(value)) => Some(value),
                    _ => None,
                })
                .collect()
        };
        // Opacity goes 0 to 1 by halfway, then back to what the element says. Width starts at what the element says.
        assert_eq!(at(0.25), [Some(0.5), Some(20.0)]);
        assert_eq!(at(0.75), [Some(0.75), Some(20.0)]);
        assert_eq!(at(0.5), [Some(1.0), Some(10.0)]);
    }

    #[test]
    fn an_animation_goes_the_way_and_as_many_times_as_it_says() {
        let plan =
            |head: &str| Plan::parse(&format!("{head}\n0 opacity n 0\n1 opacity n 1")).unwrap();
        let second = |seconds: f64| Duration::from_secs_f64(seconds);
        let normal = plan("1 0 2 normal none linear");
        assert_eq!(normal.progress(second(0.25)), Some(0.25));
        assert_eq!(normal.progress(second(1.25)), Some(0.25));
        assert_eq!(normal.progress(second(2.0)), None);
        assert_eq!(normal.end(), 1.0);
        let alternate = plan("1 0 2 alternate forwards linear");
        assert_eq!(alternate.progress(second(1.25)), Some(0.75));
        // Going back the second time, it ends where it started.
        assert_eq!(alternate.end(), 0.0);
        assert_eq!(plan("1 0 1 reverse none linear").end(), 0.0);
        assert_eq!(plan("1 0 1.5 normal none linear").end(), 0.5);
        assert!(
            plan("1 0 inf normal none linear")
                .progress(second(1000.5))
                .is_some()
        );
    }
}
