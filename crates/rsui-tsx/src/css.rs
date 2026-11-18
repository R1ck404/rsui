//! What `rsui/web` reads CSS with: sizes, colors, lists of values, and a sheet's text into its rules. It's what `web.js` did itself,
//! done here, as it's done for every declaration of every element, and reading text a character at a time is slow in JavaScript.
//!
//! Each one gives exactly what the JavaScript it replaces gave, down to how numbers come out as text, which is JavaScript's own way.

use std::borrow::Cow;

use rsui_js::{Args, HostFunction, Value, Vm};

/// What JavaScript's `\s` and `trim()` take as space.
pub(crate) fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

fn is_line_end(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

pub(crate) fn trim(text: &str) -> &str {
    text.trim_matches(is_space)
}

/// JavaScript's `parseFloat`.
pub(crate) fn parse_float(text: &str) -> f64 {
    let text = text.trim_start_matches(is_space);
    let bytes = text.as_bytes();
    let mut at = 0;
    if at < bytes.len() && (bytes[at] == b'+' || bytes[at] == b'-') {
        at += 1;
    }
    if text[at..].starts_with("Infinity") {
        return if bytes.first() == Some(&b'-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let digits = |at: &mut usize| {
        let start = *at;
        while *at < bytes.len() && bytes[*at].is_ascii_digit() {
            *at += 1;
        }
        *at - start
    };
    let whole = digits(&mut at);
    let mut fraction = 0;
    if at < bytes.len() && bytes[at] == b'.' {
        let mut after = at + 1;
        fraction = digits(&mut after);
        if whole > 0 || fraction > 0 {
            at = after;
        }
    }
    if whole == 0 && fraction == 0 {
        return f64::NAN;
    }
    if at < bytes.len() && (bytes[at] == b'e' || bytes[at] == b'E') {
        let mut after = at + 1;
        if after < bytes.len() && (bytes[after] == b'+' || bytes[after] == b'-') {
            after += 1;
        }
        if digits(&mut after) > 0 {
            at = after;
        }
    }
    text[..at].parse().unwrap_or(f64::NAN)
}

/// JavaScript's `Math.min` and `Math.max` of two, which are `NaN` if either is.
fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() { a } else { b }
    } else {
        a.min(b)
    }
}

fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_positive() { a } else { b }
    } else {
        a.max(b)
    }
}

/// JavaScript's `Math.round`, which goes up from a half.
fn js_round(value: f64) -> f64 {
    let down = value.floor();
    if value - down >= 0.5 {
        down + 1.0
    } else {
        down
    }
}

/// A number as JavaScript writes it, as `5 + '%'` does.
pub(crate) fn number_text(vm: &mut Vm, number: f64) -> String {
    vm.plain_text(Value::number(number)).unwrap_or_default()
}

/// The text inside a pair of parentheses that starts at byte `open`, and the byte after the one that closes them.
pub(crate) fn inside_parens(text: &str, open: usize) -> (&str, usize) {
    let mut depth = 0i32;
    for (at, c) in text.bytes().enumerate().skip(open) {
        if c == b'(' {
            depth += 1;
        } else if c == b')' {
            depth -= 1;
            if depth == 0 {
                return (&text[open + 1..at], at + 1);
            }
        }
    }
    (text.get(open + 1..).unwrap_or(""), text.len())
}

/// Splits at what's not in parentheses or quotes, without space around each part, and without parts that are empty.
pub(crate) fn split_outside(text: &str, is_split: impl Fn(char) -> bool) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut quote = None;
    let mut start = 0;
    let mut escaped = false;
    for (at, c) in text.char_indices() {
        if escaped {
            // What's after a backslash is a character as it is, as in a class name with `\,` or `\(` in it.
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if let Some(open) = quote {
            if c == open {
                quote = None;
            }
        } else if c == '"' || c == '\'' {
            quote = Some(c);
        } else if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
        } else if depth == 0 && is_split(c) {
            parts.push(&text[start..at]);
            start = at + c.len_utf8();
        }
    }
    parts.push(&text[start..]);
    parts
        .into_iter()
        .map(trim)
        .filter(|part| !part.is_empty())
        .collect()
}

pub(crate) fn words(text: &str) -> Vec<&str> {
    split_outside(text, |c| matches!(c, ' ' | '\t' | '\n' | '\r'))
}

pub(crate) fn commas(text: &str) -> Vec<&str> {
    split_outside(text, |c| c == ',')
}

// Sizes

/// What a size comes to: pixels, a percentage, which is text, or nothing that can be worked out here.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Size {
    Pixels(f64),
    /// Text, as `50%` or `50% + 4px`.
    Text(String),
    None,
}

/// What a size is worked out with: the size of the text it's in, how big a rem is, and the engine, for how big the page is and
/// for writing numbers as JavaScript does.
pub(crate) struct Sizing<'a> {
    pub(crate) font_size: f64,
    pub(crate) rem: f64,
    pub(crate) vm: &'a mut Vm,
}

/// How `-?[0-9]*\.?[0-9]+` matches at the start of this, without the sign: where the number ends, if it does.
pub(crate) fn number_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut at = start;
    while at < bytes.len() && bytes[at].is_ascii_digit() {
        at += 1;
    }
    let whole = at - start;
    if at < bytes.len() && bytes[at] == b'.' {
        let mut after = at + 1;
        while after < bytes.len() && bytes[after].is_ascii_digit() {
            after += 1;
        }
        if after > at + 1 {
            return Some(after);
        }
    }
    (whole > 0).then_some(at)
}

/// How tall a line is, for each pixel of the text's size, which is what `lh` is a share of.
pub(crate) const LINE: f64 = 1.2;

pub(crate) fn length(text: &str, sizing: &mut Sizing<'_>) -> Size {
    let value = trim(text);
    if value == "0" {
        return Size::Pixels(0.0);
    }
    if value.starts_with("calc(") {
        return calculated(inside_parens(value, 4).0, sizing);
    }
    // These are worked out as `calc()` is, as what's in them can be anything it can.
    if ["min(", "max(", "clamp("].iter().any(|name| value.starts_with(name)) {
        return calculated(value, sizing);
    }
    let bytes = value.as_bytes();
    let start = usize::from(bytes.first() == Some(&b'-'));
    let Some(end) = number_end(bytes, start) else {
        return Size::None;
    };
    let unit = &value[end..];
    if !unit.bytes().all(|c| c.is_ascii_lowercase() || c == b'%') {
        return Size::None;
    }
    let number: f64 = value[..end].parse().unwrap_or(f64::NAN);
    match unit {
        "" | "px" => Size::Pixels(number),
        "rem" => Size::Pixels(number * sizing.rem),
        "em" => Size::Pixels(number * sizing.font_size),
        // A line, as tall as text this size is when nothing says how tall its lines are, which is what CSS's `normal` comes to.
        "lh" => Size::Pixels(number * sizing.font_size * LINE),
        "rlh" => Size::Pixels(number * sizing.rem * LINE),
        "pt" => Size::Pixels((number * 96.0) / 72.0),
        "%" => Size::Text(number_text(sizing.vm, number) + "%"),
        // A share of the window, which is worked out again when it changes size. A window has nothing that comes and goes
        // around it as a phone's browser has, so the small, large and dynamic sizes, as `svh` is, are all the window.
        "vh" | "vw" | "vmin" | "vmax" | "svh" | "lvh" | "dvh" | "svw" | "lvw" | "dvw" | "svmin" | "lvmin" | "dvmin"
        | "svmax" | "lvmax" | "dvmax" => {
            let (width, height) = page_size(sizing.vm);
            let side = match unit.trim_start_matches(['s', 'l', 'd']) {
                "vh" => height,
                "vw" => width,
                "vmin" => js_min(width, height),
                _ => js_max(width, height),
            };
            Size::Pixels((number * side) / 100.0)
        }
        _ => Size::None,
    }
}

/// What can be inside a `calc()` and works out to what it can: `calc()` again, and `min()`, `max()` and `clamp()`.
const CALC_FUNCTIONS: [&str; 4] = ["calc", "min", "max", "clamp"];

/// The pieces of a `calc()`, as `/[()*/]|[+-](?=\s)|-?[0-9]*\.?[0-9]+[a-z%]*|NaN/g` finds them, with each of `CALC_FUNCTIONS` and the
/// bracket after it, as `min(`, as one, and the commas between what's in a `min()`, `max()` or `clamp()`.
fn calc_tokens(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    // For each bracket that's open, whether what's in it is a list, as a `min()`'s is.
    let mut lists = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let c = bytes[at];
        if matches!(c, b'(' | b')' | b'*' | b'/') || (c == b',' && lists.last() == Some(&true)) {
            if c == b'(' {
                lists.push(false);
            } else if c == b')' {
                lists.pop();
            }
            tokens.push(&text[at..at + 1]);
            at += 1;
            continue;
        }
        if (c == b'+' || c == b'-') && text[at + 1..].chars().next().is_some_and(is_space) {
            tokens.push(&text[at..at + 1]);
            at += 1;
            continue;
        }
        let digits_from = at + usize::from(c == b'-');
        if let Some(mut end) = number_end(bytes, digits_from) {
            while end < bytes.len() && (bytes[end].is_ascii_lowercase() || bytes[end] == b'%') {
                end += 1;
            }
            tokens.push(&text[at..end]);
            at = end;
            continue;
        }
        // A word: CSS's constants (`infinity`, `-infinity`, `pi`, `e`, `NaN`), in any case.
        let word_from = digits_from;
        let word_end = word_from + text[word_from..].bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'-').count();
        if word_end > word_from && bytes[word_from].is_ascii_alphabetic() {
            let word = &text[word_from..word_end];
            if ["infinity", "pi", "e", "nan"].iter().any(|one| word.eq_ignore_ascii_case(one)) {
                tokens.push(&text[at..word_end]);
                at = word_end;
                continue;
            }
            if at == word_from
                && bytes.get(word_end) == Some(&b'(')
                && CALC_FUNCTIONS.iter().any(|one| word.eq_ignore_ascii_case(one))
            {
                lists.push(!word.eq_ignore_ascii_case("calc"));
                tokens.push(&text[at..=word_end]);
                at = word_end + 1;
                continue;
            }
        }
        // Not anything, so on to the next character.
        at += text[at..].chars().next().map_or(1, char::len_utf8);
    }
    tokens
}

/// A value in a `calc()`: how much of it is a plain number, pixels, and a percentage of what it's in.
#[derive(Clone, Copy)]
struct Amount {
    number: f64,
    px: f64,
    percent: f64,
    plain: bool,
}

impl Amount {
    /// Only pixels, this many.
    const fn pixels(px: f64) -> Self {
        Self {
            number: 0.0,
            px,
            percent: 0.0,
            plain: false,
        }
    }

    fn times(self, by: f64) -> Self {
        // What there's none of stays none, even times `infinity`, so `calc(infinity * 1px)` is that many pixels and no share.
        let times = |part: f64| if part == 0.0 { 0.0 } else { part * by };
        Self {
            number: times(self.number),
            px: times(self.px),
            percent: times(self.percent),
            plain: self.plain,
        }
    }

    /// A plain number with a length is taken as that many pixels, which is more than CSS lets through.
    fn as_px(self) -> Self {
        if self.plain {
            Self::pixels(self.number)
        } else {
            self
        }
    }

    /// Both added up: a plain number if both are, and otherwise pixels and a share.
    fn plus(self, other: Self) -> Self {
        if self.plain && other.plain {
            Self {
                number: self.number + other.number,
                ..self
            }
        } else {
            let (one, other) = (self.as_px(), other.as_px());
            Self {
                px: one.px + other.px,
                percent: one.percent + other.percent,
                ..one
            }
        }
    }

    /// Whether this is as far as a length goes, which is what a `min()` of two has for a least, and a `max()` of two for a most.
    fn endless(self) -> bool {
        self.px.is_infinite()
    }

    /// Whether which of this and `other` is bigger is the same however big what they're in is, as it is when they're the same
    /// share of it.
    fn comparable(self, other: Self) -> bool {
        self.as_px().percent == other.as_px().percent
    }

    /// Of this and another that's [`comparable`](Self::comparable), the one `pick` picks, as `js_min` picks the smaller.
    fn picked(self, other: Self, pick: fn(f64, f64) -> f64) -> Self {
        if self.plain && other.plain {
            Self {
                number: pick(self.number, other.number),
                ..self
            }
        } else {
            let (one, other) = (self.as_px(), other.as_px());
            Self {
                px: pick(one.px, other.px),
                ..one
            }
        }
    }
}

/// What a `min()` of two has for a least, and a `max()` of two for a most: none at all.
const NO_LEAST: Amount = Amount::pixels(f64::NEG_INFINITY);
const NO_MOST: Amount = Amount::pixels(f64::INFINITY);

/// What a part of a `calc()` comes to: an amount, or one kept between a least and a most, as `clamp()` keeps it, which is what
/// `min()`, `max()` and `clamp()` of different shares of what they're in come to, as which is smallest is only known once how big
/// that is is. A `min()` of two has [`NO_LEAST`], and a `max()` of two [`NO_MOST`]. What can't be said as one of these, as a
/// `min()` of three different shares, isn't anything.
#[derive(Clone, Copy)]
enum Term {
    Amount(Amount),
    Clamp(Amount, Amount, Amount),
}

impl Term {
    /// `clamp(least, value, most)`, said as simply as it can be: worked out where none of it is a share of anything, with what's
    /// the same share as what's next to it put together, and as the least where that's always more than the most.
    fn clamp(least: Amount, value: Amount, most: Amount) -> Self {
        let worked_out = |least: Amount, value: Amount, most: Amount| Self::Amount(value.picked(most, js_min).picked(least, js_max));
        if least.plain && value.plain && most.plain {
            return worked_out(least, value, most);
        }
        let (mut least, mut value, mut most) = (least.as_px(), value.as_px(), most.as_px());
        if value.comparable(most) {
            value = value.picked(most, js_min);
            most = NO_MOST;
        }
        if most.endless() && least.comparable(value) {
            value = value.picked(least, js_max);
            least = NO_LEAST;
        }
        let fixed = |one: Amount| one.percent == 0.0;
        if (least.endless() && most.endless()) || (fixed(least) && fixed(value) && fixed(most)) {
            return worked_out(least, value, most);
        }
        if !least.endless() && !most.endless() && least.comparable(most) && least.px >= most.px {
            return Self::Amount(least);
        }
        Self::Clamp(least, value, most)
    }

    /// The same, the other way around: `-clamp(l, v, m)` is `clamp(-m, -v, -l)` where the least is never more than the most,
    /// which is so of what [`clamp`](Self::clamp) makes unless they're different shares.
    fn negated(self) -> Option<Self> {
        match self {
            Self::Amount(amount) => Some(Self::Amount(amount.times(-1.0))),
            Self::Clamp(least, value, most) if least.endless() || most.endless() || least.comparable(most) => {
                Some(Self::clamp(most.times(-1.0), value.times(-1.0), least.times(-1.0)))
            }
            Self::Clamp(..) => None,
        }
    }

    /// Times a plain number.
    fn times(self, by: f64) -> Option<Self> {
        match self {
            Self::Amount(amount) => Some(Self::Amount(amount.times(by))),
            Self::Clamp(_, value, _) if by == 0.0 => Some(Self::Amount(value.times(by))),
            Self::Clamp(least, value, most) if by > 0.0 => Some(Self::clamp(least.times(by), value.times(by), most.times(by))),
            Self::Clamp(..) if by < 0.0 => self.negated()?.times(-by),
            Self::Clamp(..) => None,
        }
    }

    /// Both added up. An amount is added to all of what's kept between a least and a most, and two of those can't be added.
    fn plus(self, other: Self) -> Option<Self> {
        match (self, other) {
            (Self::Amount(one), Self::Amount(other)) => Some(Self::Amount(one.plus(other))),
            (Self::Clamp(least, value, most), Self::Amount(more)) | (Self::Amount(more), Self::Clamp(least, value, most)) => {
                let more = more.as_px();
                let moved = |bound: Amount| if bound.endless() { bound } else { bound.plus(more) };
                Some(Self::clamp(moved(least), value.plus(more), moved(most)))
            }
            (Self::Clamp(..), Self::Clamp(..)) => None,
        }
    }

    /// The smaller of the two, as `min()` has it, where that can be said as one.
    fn least_of(self, other: Self) -> Option<Self> {
        match (self, other) {
            (Self::Amount(one), Self::Amount(other)) if one.plain && other.plain => Some(Self::Amount(one.picked(other, js_min))),
            (Self::Amount(one), Self::Amount(other)) => Some(Self::clamp(NO_LEAST, one, other)),
            (Self::Clamp(least, value, most), Self::Amount(more)) | (Self::Amount(more), Self::Clamp(least, value, most)) => {
                let more = more.as_px();
                // What's always less than the least is what it comes to. What's never less is a most, which takes the place of the
                // one it's the same share as, as `min(clamp(l, v, m), x)` is `clamp(l, v, min(m, x))`.
                let above_least = least.endless() || (least.comparable(more) && least.px <= more.px);
                if !above_least && least.comparable(more) {
                    Some(Self::Amount(more))
                } else if !above_least {
                    None
                } else if more.comparable(value) {
                    Some(Self::clamp(least, value.picked(more, js_min), most))
                } else if most.endless() {
                    Some(Self::clamp(least, value, more))
                } else if more.comparable(most) {
                    Some(Self::clamp(least, value, most.picked(more, js_min)))
                } else {
                    None
                }
            }
            (Self::Clamp(..), Self::Clamp(..)) => None,
        }
    }

    /// The bigger of the two, as `max()` has it, which is the smaller of the two the other way around.
    fn most_of(self, other: Self) -> Option<Self> {
        self.negated()?.least_of(other.negated()?)?.negated()
    }
}

struct Calc<'t, 's, 'z> {
    tokens: Vec<&'t str>,
    at: usize,
    sizing: &'s mut Sizing<'z>,
}

impl<'t> Calc<'t, '_, '_> {
    fn token(&self) -> Option<&'t str> {
        self.tokens.get(self.at).copied()
    }

    fn atom(&mut self) -> Option<Term> {
        let token = self.token();
        self.at += 1;
        let token = token?;
        if token == "(" || token.eq_ignore_ascii_case("calc(") {
            let inner = self.sum();
            self.at += 1;
            return inner;
        }
        if let Some(name) = token.strip_suffix('(') {
            return self.function(name);
        }
        // CSS's constants (CSS Values 4, 10.7.1), as Tailwind 4's `rounded-full` is `calc(infinity * 1px)`.
        let (sign, word) = match token.strip_prefix('-') {
            Some(rest) => (-1.0, rest),
            None => (1.0, token),
        };
        if word.eq_ignore_ascii_case("NaN") {
            return None;
        }
        let constant = match word.to_ascii_lowercase().as_str() {
            "infinity" => Some(f64::INFINITY),
            "pi" => Some(std::f64::consts::PI),
            "e" => Some(std::f64::consts::E),
            _ => None,
        };
        let plain = |number| Amount {
            number,
            px: 0.0,
            percent: 0.0,
            plain: true,
        };
        if let Some(number) = constant.map(|number| sign * number) {
            return Some(Term::Amount(plain(number)));
        }
        let digits = token.strip_prefix('-').unwrap_or(token);
        if !digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit() || c == b'.') {
            return Some(Term::Amount(plain(parse_float(token))));
        }
        match length(token, self.sizing) {
            Size::None => None,
            Size::Pixels(px) => Some(Term::Amount(Amount::pixels(px))),
            Size::Text(text) => Some(Term::Amount(Amount {
                percent: parse_float(&text),
                ..Amount::pixels(0.0)
            })),
        }
    }

    /// `min()`, `max()` or `clamp()`, by `name`, once its `(` has been read: what's in it, to its `)`.
    fn function(&mut self, name: &str) -> Option<Term> {
        let mut parts = vec![self.sum()];
        while self.token() == Some(",") {
            self.at += 1;
            parts.push(self.sum());
        }
        self.at += 1;
        let parts: Vec<Term> = parts.into_iter().collect::<Option<_>>()?;
        match (name.to_ascii_lowercase().as_str(), parts.as_slice()) {
            ("min", [first, rest @ ..]) => rest.iter().try_fold(*first, |least, one| least.least_of(*one)),
            ("max", [first, rest @ ..]) => rest.iter().try_fold(*first, |most, one| most.most_of(*one)),
            ("clamp", [Term::Amount(least), Term::Amount(value), Term::Amount(most)]) => Some(Term::clamp(*least, *value, *most)),
            ("clamp", [least, value, most]) => least.most_of(value.least_of(*most)?),
            _ => None,
        }
    }

    fn product(&mut self) -> Option<Term> {
        let mut left = self.atom();
        while let Some(op @ ("*" | "/")) = self.token() {
            let divide = op == "/";
            self.at += 1;
            let right = self.atom();
            left = match (left, right) {
                (Some(left), Some(Term::Amount(right))) if divide && right.plain => left.times(1.0 / right.number),
                (Some(Term::Amount(left)), Some(right)) if !divide && left.plain => right.times(left.number),
                (Some(left), Some(Term::Amount(right))) if !divide && right.plain => left.times(right.number),
                _ => None,
            };
        }
        left
    }

    fn sum(&mut self) -> Option<Term> {
        let mut left = self.product();
        while let Some(op @ ("+" | "-")) = self.token() {
            let minus = op == "-";
            self.at += 1;
            let right = self.product();
            left = match (left, right) {
                (Some(one), Some(right)) => if minus { right.negated() } else { Some(right) }.and_then(|right| one.plus(right)),
                _ => None,
            };
        }
        left
    }
}

/// The longest a length can be, in pixels, as Chromium's `LayoutUnit` has it: what's infinite comes to this.
const MAX_LENGTH: f64 = 33_554_400.0;

/// `calc()`, as long as all of it comes to the same kind of thing. A number with no unit can times or divide what has one.
fn calculated(text: &str, sizing: &mut Sizing<'_>) -> Size {
    let mut calc = Calc {
        tokens: calc_tokens(text),
        at: 0,
        sizing,
    };
    let result = match calc.sum() {
        None => return Size::None,
        Some(Term::Amount(result)) => result,
        Some(Term::Clamp(least, value, most)) => return bounded(calc.sizing.vm, [least, value, most]),
    };
    if result.plain {
        return if result.number.is_nan() {
            Size::None
        } else {
            Size::Pixels(result.number.clamp(-MAX_LENGTH, MAX_LENGTH))
        };
    }
    if result.px.is_nan() || result.percent.is_nan() {
        return Size::None;
    }
    if result.percent == 0.0 {
        return Size::Pixels(result.px.clamp(-MAX_LENGTH, MAX_LENGTH));
    }
    Size::Text(share_text(calc.sizing.vm, result))
}

/// A share of what it's in and pixels, as what lays it out reads it: `50%`, `50% + -5px`, or `5px` with no share. What's infinite
/// is as far as a length goes, as a browser keeps it (Chromium's `LayoutUnit` reaches 33554431px, and its `getComputedStyle()` says
/// `3.35544e+07px` for `calc(infinity * 1px)`), so that what's worked out from it, as corners made to fit their box, stays a number.
fn share_text(vm: &mut Vm, amount: Amount) -> String {
    let (px, percent) = (amount.px.clamp(-MAX_LENGTH, MAX_LENGTH), amount.percent.clamp(-MAX_LENGTH, MAX_LENGTH));
    if percent == 0.0 {
        number_text(vm, px) + "px"
    } else if px == 0.0 {
        number_text(vm, percent) + "%"
    } else {
        number_text(vm, percent) + "% + " + &number_text(vm, px) + "px"
    }
}

/// What's kept between a least and a most, as what lays it out reads it (rsui-layout's `Clamp`): `min()` where there's no least,
/// `max()` where there's no most, and `clamp()` where there's both, as `min(640px, 100% + -48px)`.
fn bounded(vm: &mut Vm, [least, value, most]: [Amount; 3]) -> Size {
    if [least, value, most].iter().any(|part| part.px.is_nan() || part.percent.is_nan()) {
        return Size::None;
    }
    let value = share_text(vm, value);
    Size::Text(match (least.endless(), most.endless()) {
        (true, _) => format!("min({value}, {})", share_text(vm, most)),
        (false, true) => format!("max({}, {value})", share_text(vm, least)),
        (false, false) => format!("clamp({}, {value}, {})", share_text(vm, least), share_text(vm, most)),
    })
}

/// Whether a size is one [`bounded`] writes.
pub(crate) fn is_bounded(size: &str) -> bool {
    ["min(", "max(", "clamp("].iter().any(|name| size.starts_with(name))
}

// Colors

/// Every color CSS has a name for, in order, to be found by name.
#[rustfmt::skip]
const NAMED: &[(&str, &str)] = &[
    ("aliceblue", "#f0f8ff"), ("antiquewhite", "#faebd7"), ("aqua", "#00ffff"), ("aquamarine", "#7fffd4"), ("azure", "#f0ffff"),
    ("beige", "#f5f5dc"), ("bisque", "#ffe4c4"), ("black", "#000000"), ("blanchedalmond", "#ffebcd"), ("blue", "#0000ff"),
    ("blueviolet", "#8a2be2"), ("brown", "#a52a2a"), ("burlywood", "#deb887"), ("cadetblue", "#5f9ea0"), ("chartreuse", "#7fff00"),
    ("chocolate", "#d2691e"), ("coral", "#ff7f50"), ("cornflowerblue", "#6495ed"), ("cornsilk", "#fff8dc"), ("crimson", "#dc143c"),
    ("cyan", "#00ffff"), ("darkblue", "#00008b"), ("darkcyan", "#008b8b"), ("darkgoldenrod", "#b8860b"), ("darkgray", "#a9a9a9"),
    ("darkgreen", "#006400"), ("darkgrey", "#a9a9a9"), ("darkkhaki", "#bdb76b"), ("darkmagenta", "#8b008b"),
    ("darkolivegreen", "#556b2f"), ("darkorange", "#ff8c00"), ("darkorchid", "#9932cc"), ("darkred", "#8b0000"),
    ("darksalmon", "#e9967a"), ("darkseagreen", "#8fbc8f"), ("darkslateblue", "#483d8b"), ("darkslategray", "#2f4f4f"),
    ("darkslategrey", "#2f4f4f"), ("darkturquoise", "#00ced1"), ("darkviolet", "#9400d3"), ("deeppink", "#ff1493"),
    ("deepskyblue", "#00bfff"), ("dimgray", "#696969"), ("dimgrey", "#696969"), ("dodgerblue", "#1e90ff"), ("firebrick", "#b22222"),
    ("floralwhite", "#fffaf0"), ("forestgreen", "#228b22"), ("fuchsia", "#ff00ff"), ("gainsboro", "#dcdcdc"), ("ghostwhite", "#f8f8ff"),
    ("gold", "#ffd700"), ("goldenrod", "#daa520"), ("gray", "#808080"), ("green", "#008000"), ("greenyellow", "#adff2f"),
    ("grey", "#808080"), ("honeydew", "#f0fff0"), ("hotpink", "#ff69b4"), ("indianred", "#cd5c5c"), ("indigo", "#4b0082"),
    ("ivory", "#fffff0"), ("khaki", "#f0e68c"), ("lavender", "#e6e6fa"), ("lavenderblush", "#fff0f5"), ("lawngreen", "#7cfc00"),
    ("lemonchiffon", "#fffacd"), ("lightblue", "#add8e6"), ("lightcoral", "#f08080"), ("lightcyan", "#e0ffff"),
    ("lightgoldenrodyellow", "#fafad2"), ("lightgray", "#d3d3d3"), ("lightgreen", "#90ee90"), ("lightgrey", "#d3d3d3"),
    ("lightpink", "#ffb6c1"), ("lightsalmon", "#ffa07a"), ("lightseagreen", "#20b2aa"), ("lightskyblue", "#87cefa"),
    ("lightslategray", "#778899"), ("lightslategrey", "#778899"), ("lightsteelblue", "#b0c4de"), ("lightyellow", "#ffffe0"),
    ("lime", "#00ff00"), ("limegreen", "#32cd32"), ("linen", "#faf0e6"), ("magenta", "#ff00ff"), ("maroon", "#800000"),
    ("mediumaquamarine", "#66cdaa"), ("mediumblue", "#0000cd"), ("mediumorchid", "#ba55d3"), ("mediumpurple", "#9370db"),
    ("mediumseagreen", "#3cb371"), ("mediumslateblue", "#7b68ee"), ("mediumspringgreen", "#00fa9a"), ("mediumturquoise", "#48d1cc"),
    ("mediumvioletred", "#c71585"), ("midnightblue", "#191970"), ("mintcream", "#f5fffa"), ("mistyrose", "#ffe4e1"),
    ("moccasin", "#ffe4b5"), ("navajowhite", "#ffdead"), ("navy", "#000080"), ("oldlace", "#fdf5e6"), ("olive", "#808000"),
    ("olivedrab", "#6b8e23"), ("orange", "#ffa500"), ("orangered", "#ff4500"), ("orchid", "#da70d6"), ("palegoldenrod", "#eee8aa"),
    ("palegreen", "#98fb98"), ("paleturquoise", "#afeeee"), ("palevioletred", "#db7093"), ("papayawhip", "#ffefd5"),
    ("peachpuff", "#ffdab9"), ("peru", "#cd853f"), ("pink", "#ffc0cb"), ("plum", "#dda0dd"), ("powderblue", "#b0e0e6"),
    ("purple", "#800080"), ("rebeccapurple", "#663399"), ("red", "#ff0000"), ("rosybrown", "#bc8f8f"), ("royalblue", "#4169e1"),
    ("saddlebrown", "#8b4513"), ("salmon", "#fa8072"), ("sandybrown", "#f4a460"), ("seagreen", "#2e8b57"), ("seashell", "#fff5ee"),
    ("sienna", "#a0522d"), ("silver", "#c0c0c0"), ("skyblue", "#87ceeb"), ("slateblue", "#6a5acd"), ("slategray", "#708090"),
    ("slategrey", "#708090"), ("snow", "#fffafa"), ("springgreen", "#00ff7f"), ("steelblue", "#4682b4"), ("tan", "#d2b48c"),
    ("teal", "#008080"), ("thistle", "#d8bfd8"), ("tomato", "#ff6347"), ("transparent", "#00000000"), ("turquoise", "#40e0d0"),
    ("violet", "#ee82ee"),
    ("wheat", "#f5deb3"), ("white", "#ffffff"), ("whitesmoke", "#f5f5f5"), ("yellow", "#ffff00"), ("yellowgreen", "#9acd32"),
];

fn hex_part(out: &mut String, value: f64) {
    use std::fmt::Write;
    let value = js_round(js_max(0.0, js_min(255.0, value))) as u8;
    let _ = write!(out, "{value:02x}");
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    let a = s * js_min(l, 1.0 - l);
    let f = |n: f64| {
        let k = (n + h / 30.0) % 12.0;
        l - a * js_max(-1.0, js_min(js_min(k - 3.0, 9.0 - k), 1.0))
    };
    [f(0.0) * 255.0, f(8.0) * 255.0, f(4.0) * 255.0]
}

/// A color as red, green and blue from 0 to 255, and how much it shows from 0 to 1.
type Rgba = [f64; 4];

/// A color written as `#rrggbb`, or `#rrggbbaa` when it doesn't fully show.
fn hex_of([red, green, blue, alpha]: Rgba) -> String {
    let mut out = String::with_capacity(9);
    out.push('#');
    for one in [red, green, blue] {
        hex_part(&mut out, one.clamp(0.0, 255.0));
    }
    if alpha < 1.0 {
        hex_part(&mut out, alpha.clamp(0.0, 1.0) * 255.0);
    }
    out
}

/// What `#rrggbb(aa)` says, as [`color`] gives it.
fn rgba_of(hex: &str) -> Option<Rgba> {
    let digits = hex.strip_prefix('#')?;
    let digits: String = match digits.len() {
        3 | 4 => digits.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => digits.to_owned(),
        _ => return None,
    };
    let byte = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok().map(f64::from);
    let alpha = if digits.len() == 8 { byte(6)? / 255.0 } else { 1.0 };
    Some([byte(0)?, byte(2)?, byte(4)?, alpha])
}

/// A part of a newer color: a number, or a percentage of `whole`, or `none`, which is 0.
fn amount(text: &str, whole: f64) -> f64 {
    match text.strip_suffix('%') {
        Some(share) => parse_float(share) * whole / 100.0,
        None if text == "none" => 0.0,
        None => parse_float(text),
    }
}

/// An angle in degrees, from what it's written in.
fn degrees(text: &str) -> f64 {
    if let Some(turns) = text.strip_suffix("turn") {
        parse_float(turns) * 360.0
    } else if let Some(radians) = text.strip_suffix("rad").filter(|_| !text.ends_with("grad")) {
        parse_float(radians).to_degrees()
    } else if let Some(grads) = text.strip_suffix("grad") {
        parse_float(grads) * 0.9
    } else {
        amount(text.strip_suffix("deg").unwrap_or(text), 1.0)
    }
}

/// From linear light to how sRGB has it, from 0 to 1.
fn to_srgb(linear: f64) -> f64 {
    if linear <= 0.003_130_8 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

fn from_srgb(srgb: f64) -> f64 {
    if srgb <= 0.040_45 {
        srgb / 12.92
    } else {
        ((srgb + 0.055) / 1.055).powf(2.4)
    }
}

/// Oklab as sRGB from 0 to 255, with what doesn't fit cut off, as a browser does.
fn oklab_to_rgb(lightness: f64, a: f64, b: f64) -> [f64; 3] {
    let l = (lightness + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m = (lightness - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s = (lightness - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
    [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701 * s,
    ]
    .map(|linear| to_srgb(linear).clamp(0.0, 1.0) * 255.0)
}

fn rgb_to_oklab([red, green, blue]: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = [red, green, blue].map(|one| from_srgb(one / 255.0));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    ]
}

/// Colors written in CSS's newer ways: `oklch()`, `oklab()`, `hwb()` and `color-mix()`.
fn newer_color(value: &str) -> Option<Rgba> {
    let open = value.find('(')?;
    let name = &value[..open];
    let inside = inside_parens(value, open).0;
    if name == "color-mix" {
        return color_mix(inside);
    }
    let parts: Vec<&str> = inside
        .split(|c: char| is_space(c) || c == ',' || c == '/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() < 3 {
        return None;
    }
    let alpha = parts.get(3).map_or(1.0, |part| amount(part, 1.0));
    let [red, green, blue] = match name {
        "oklch" => {
            let lightness = amount(parts[0], 1.0);
            let chroma = amount(parts[1], 0.4);
            let hue = degrees(parts[2]).to_radians();
            oklab_to_rgb(lightness, chroma * hue.cos(), chroma * hue.sin())
        }
        "oklab" => oklab_to_rgb(amount(parts[0], 1.0), amount(parts[1], 0.4), amount(parts[2], 0.4)),
        "hwb" => {
            let (white, black) = (amount(parts[1], 1.0), amount(parts[2], 1.0));
            if white + black >= 1.0 {
                let gray = white / (white + black) * 255.0;
                [gray, gray, gray]
            } else {
                hsl_to_rgb(degrees(parts[0]), 1.0, 0.5)
                    .map(|one| one * (1.0 - white - black) + white * 255.0)
            }
        }
        _ => return None,
    };
    let rgba = [red, green, blue, alpha];
    rgba.iter().all(|one| one.is_finite()).then_some(rgba)
}

/// `color-mix(in srgb, red 30%, blue)`: the two colors mixed in that space, by those shares, as a browser does it.
fn color_mix(inside: &str) -> Option<Rgba> {
    let parts = commas(inside);
    let [space, first, second] = parts.as_slice() else {
        return None;
    };
    let space = trim(space).strip_prefix("in")?.trim();
    let side = |text: &str| -> Option<(Rgba, Option<f64>)> {
        let text = trim(text);
        // The share comes after the color, or before it, and the color can have spaces inside its brackets.
        let (paint, share) = match text.rsplit_once(|c: char| is_space(c)) {
            Some((paint, share)) if share.ends_with('%') && !paint.ends_with('(') => (paint, Some(share)),
            _ => match text.split_once(|c: char| is_space(c)) {
                Some((share, paint)) if share.ends_with('%') => (paint, Some(share)),
                _ => (text, None),
            },
        };
        let paint = rgba_of(&color(paint)?)?;
        Some((paint, share.map(|share| amount(share, 1.0))))
    };
    let (one, first_share) = side(first)?;
    let (two, second_share) = side(second)?;
    let (p1, p2) = match (first_share, second_share) {
        (None, None) => (0.5, 0.5),
        (Some(p1), None) => (p1, 1.0 - p1),
        (None, Some(p2)) => (1.0 - p2, p2),
        (Some(p1), Some(p2)) => (p1, p2),
    };
    let total = p1 + p2;
    if total <= 0.0 {
        return None;
    }
    // Less than all of it in all leaves the rest showing through.
    let scale = total.min(1.0);
    let (p1, p2) = (p1 / total, p2 / total);
    let alpha = one[3] * p1 + two[3] * p2;
    let mixed = if space.starts_with("oklab") || space.starts_with("oklch") {
        let (a, b) = (rgb_to_oklab([one[0], one[1], one[2]]), rgb_to_oklab([two[0], two[1], two[2]]));
        let weigh = |at: usize| (a[at] * one[3] * p1 + b[at] * two[3] * p2) / alpha.max(f64::EPSILON);
        oklab_to_rgb(weigh(0), weigh(1), weigh(2))
    } else {
        let linear = space.starts_with("srgb-linear");
        let weigh = |at: usize| {
            let (x, y) = if linear {
                (from_srgb(one[at] / 255.0), from_srgb(two[at] / 255.0))
            } else {
                (one[at], two[at])
            };
            let mixed = (x * one[3] * p1 + y * two[3] * p2) / alpha.max(f64::EPSILON);
            if linear { to_srgb(mixed) * 255.0 } else { mixed }
        };
        [weigh(0), weigh(1), weigh(2)]
    };
    Some([mixed[0], mixed[1], mixed[2], alpha * scale])
}

/// Whether this is a color worked out from the color of the text it's with, as `color-mix(in oklab, currentcolor 50%, transparent)`
/// is, which is kept as it's written until that's known, and then is [`with_current_color`].
pub(crate) fn waits_on_current_color(text: &str) -> bool {
    let text = trim(text).to_lowercase();
    text != "currentcolor" && text.contains("currentcolor") && with_current_color(&text, "#000000").is_some()
}

/// What a color that [waits on the color of the text](waits_on_current_color) comes to, with that color, `current`, in it.
pub(crate) fn with_current_color(text: &str, current: &str) -> Option<Cow<'static, str>> {
    color(&trim(text).to_lowercase().replace("currentcolor", current))
}

/// A color as rsui takes it, as `#rrggbb` or `#rrggbbaa`, or as it was written if it's in hex, or nothing if it isn't one.
pub(crate) fn color(text: &str) -> Option<Cow<'static, str>> {
    let value = trim(text).to_lowercase();
    if let Some(digits) = value.strip_prefix('#') {
        let fits = matches!(digits.len(), 3 | 4 | 6 | 8)
            && digits
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
        return fits.then(|| Cow::Owned(value));
    }
    if let Ok(at) = NAMED.binary_search_by(|(name, _)| name.cmp(&value.as_str())) {
        return Some(Cow::Borrowed(NAMED[at].1));
    }
    let Some(prefix) = ["rgba(", "rgb(", "hsla(", "hsl("]
        .into_iter()
        .find(|prefix| value.starts_with(prefix))
    else {
        return newer_color(&value).map(|rgba| Cow::Owned(hex_of(rgba)));
    };
    let inside = inside_parens(&value, prefix.len() - 1).0;
    let parts: Vec<&str> = inside
        .split(|c: char| is_space(c) || c == ',' || c == '/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() < 3 {
        return None;
    }
    let part = |at: usize, whole: f64| {
        let text = parts[at];
        if text.ends_with('%') {
            (parse_float(text) * whole) / 100.0
        } else {
            parse_float(text)
        }
    };
    let alpha = if parts.len() > 3 { part(3, 1.0) } else { 1.0 };
    let rgb = if prefix.starts_with("rgb") {
        [part(0, 255.0), part(1, 255.0), part(2, 255.0)]
    } else {
        hsl_to_rgb(
            parse_float(parts[0]),
            parse_float(parts[1]) / 100.0,
            parse_float(parts[2]) / 100.0,
        )
    };
    if rgb.iter().any(|one| one.is_nan()) || alpha.is_nan() {
        return None;
    }
    let mut out = String::with_capacity(9);
    out.push('#');
    for one in rgb {
        hex_part(&mut out, one);
    }
    if alpha < 1.0 {
        hex_part(&mut out, alpha * 255.0);
    }
    Some(Cow::Owned(out))
}

// CSS, as text, into rules

/// A block of CSS: its declarations, as `[name, value, important]`, and the rules in it, as `[selector, block]`.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Block {
    pub(crate) declarations: Vec<(String, String, bool)>,
    pub(crate) rules: Vec<(String, Block)>,
}

/// CSS without its comments: what's between `/*` and `*/`, and a line that starts with `//`.
pub(crate) fn without_comments(text: &str) -> Cow<'_, str> {
    let mut out: Cow<'_, str> = Cow::Borrowed(text);
    // `/\/\*[\s\S]*?\*\//g`, which leaves one that isn't closed as it is.
    if out.contains("/*") {
        let mut kept = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("/*") {
            match rest[start + 2..].find("*/") {
                Some(end) => {
                    kept.push_str(&rest[..start]);
                    rest = &rest[start + 2 + end + 2..];
                }
                None => break,
            }
        }
        kept.push_str(rest);
        out = Cow::Owned(kept);
    }
    // `/^\s*\/\/.*$/gm`: from the start of a line, any space, which can be more lines, then `//` and the rest of that line.
    if out.contains("//") {
        let text = &*out;
        let mut kept = String::with_capacity(text.len());
        let mut from = 0;
        let mut at = 0;
        let mut line_start = true;
        while at < text.len() {
            if line_start {
                let spaces = text[at..]
                    .char_indices()
                    .find(|&(_, c)| !is_space(c))
                    .map_or(text.len() - at, |(index, _)| index);
                let after = at + spaces;
                if text[after..].starts_with("//") {
                    let end = text[after..]
                        .char_indices()
                        .find(|&(_, c)| is_line_end(c))
                        .map_or(text.len(), |(index, _)| after + index);
                    kept.push_str(&text[from..at]);
                    from = end;
                    at = end;
                    line_start = false;
                    continue;
                }
            }
            let c = text[at..].chars().next().unwrap_or(' ');
            line_start = is_line_end(c);
            at += c.len_utf8();
        }
        kept.push_str(&text[from..]);
        out = Cow::Owned(kept);
    }
    out
}

fn add_declaration(block: &mut Block, text: &str) {
    // A rule with no block, as `@layer a, b;` and `@import "x";` are, which says something all the same.
    let statement = trim(text);
    if statement.starts_with('@') {
        block.rules.push((statement.to_owned(), Block::default()));
        return;
    }
    let Some(colon) = text.find(':') else {
        return;
    };
    // A custom property's name is as it's written, as `--Grid-columns` and `--gridColumns` are different ones.
    let name = trim(&text[..colon]);
    let name = if name.starts_with("--") {
        name.to_owned()
    } else {
        name.to_lowercase()
    };
    if !is_property_name(&name) {
        return;
    }
    let value = &text[colon + 1..];
    // `/!\s*important/gi`, which is only ever plain letters.
    let mut plain = String::with_capacity(value.len());
    let mut important = false;
    let mut rest = value;
    while let Some(bang) = rest.find('!') {
        let after = rest[bang + 1..].trim_start_matches(is_space);
        let word = after.get(..9);
        if word.is_some_and(|word| word.eq_ignore_ascii_case("important")) {
            important = true;
            plain.push_str(&rest[..bang]);
            rest = &after[9..];
        } else {
            plain.push_str(&rest[..=bang]);
            rest = &rest[bang + 1..];
        }
    }
    plain.push_str(rest);
    block
        .declarations
        .push((name, trim(&plain).to_owned(), important));
}

/// Whether a declaration's name is a name at all, as CSS Syntax has it, rather than something only an old browser read, as
/// `*zoom` is, which a browser leaves out without saying so.
fn is_property_name(name: &str) -> bool {
    let rest = name.strip_prefix('-').unwrap_or(name);
    let starts = rest
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '-' || c == '\\' || !c.is_ascii());
    // What's after a backslash is any character, as `--chakra-spacing-1\.5` has a full stop in its name.
    let mut escaped = false;
    starts
        && rest.chars().all(|c| {
            let fits = escaped || c.is_alphanumeric() || c == '_' || c == '-' || c == '\\' || !c.is_ascii();
            escaped = !escaped && c == '\\';
            fits
        })
}

/// Reads the inside of a block, as far as what closes it. What's before a `{` is what the rule is for, and `name: value;` is the rest.
fn read_block(text: &str, start: usize, top: bool) -> (Block, usize) {
    let bytes = text.as_bytes();
    let mut block = Block::default();
    let mut at = start;
    let mut from = start;
    let mut quote = None;
    let mut parens = 0i32;
    while at < bytes.len() {
        let c = bytes[at];
        if let Some(open) = quote {
            if c == b'\\' {
                // What's after a backslash is taken as it is, whatever it is.
                at += 1 + text[at + 1..].chars().next().map_or(0, char::len_utf8);
                continue;
            } else if c == open {
                quote = None;
            }
        } else if c == b'\\' {
            // What's after a backslash is a character as it is, as in a class name with `\'` or `\(` in it.
            at += 1 + text[at + 1..].chars().next().map_or(0, char::len_utf8);
            continue;
        } else if c == b'"' || c == b'\'' {
            quote = Some(c);
        } else if c == b'(' {
            parens += 1;
        } else if c == b')' {
            parens -= 1;
        } else if parens <= 0 {
            if c == b';' {
                add_declaration(&mut block, &text[from..at]);
                from = at + 1;
            } else if c == b'{' {
                let (inner, end) = read_block(text, at + 1, false);
                block.rules.push((trim(&text[from..at]).to_owned(), inner));
                at = end;
                from = at;
                continue;
            } else if c == b'}' {
                add_declaration(&mut block, &text[from..at]);
                if !top {
                    return (block, at + 1);
                }
                // One that closes nothing is a mistake, which a browser gets past, as this does.
                from = at + 1;
            }
        }
        at += 1;
    }
    let at = at.min(bytes.len());
    add_declaration(&mut block, &text[from.min(at)..at]);
    (block, at)
}

pub(crate) fn read_css(text: &str) -> Block {
    read_block(&without_comments(text), 0, true).0
}

// What JavaScript calls.

pub(crate) fn text_of<'v>(vm: &'v mut Vm, value: Value) -> Cow<'v, str> {
    if value.is_string() {
        return vm.string_text(value).unwrap_or_default();
    }
    Cow::Owned(vm.plain_text(value).unwrap_or_default())
}

/// What JavaScript's `fontSize = 16` comes to, as a number.
pub(crate) fn number_or(value: Value, otherwise: f64) -> f64 {
    if value == Value::UNDEFINED {
        otherwise
    } else if value.is_number() {
        value.as_number()
    } else if value.is_null() || value == Value::FALSE {
        0.0
    } else if value == Value::TRUE {
        1.0
    } else {
        f64::NAN
    }
}

fn page_size(vm: &mut Vm) -> (f64, f64) {
    vm.set_global("__rsuiViewportUnits", Value::TRUE);
    let size = vm.get_global("__rsuiPageSize").unwrap_or(Value::UNDEFINED);
    match vm.array_elements(size) {
        Some(parts) => {
            let at = |index: usize| {
                parts
                    .get(index)
                    .map_or(f64::NAN, |one| number_or(*one, f64::NAN))
            };
            (at(0), at(1))
        }
        None => (800.0, 600.0),
    }
}

fn size_value(vm: &mut Vm, size: Size) -> Value {
    match size {
        Size::Pixels(number) => Value::number(number),
        Size::Text(text) => vm.new_string(&text),
        Size::None => Value::UNDEFINED,
    }
}

/// `length(text, fontSize, remSize)`: a size as pixels, as text if it's a percentage, or `undefined`.
fn host_length(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (text, font_size, rem) = (args.get(vm, 0), args.get(vm, 1), args.get(vm, 2));
    let text = text_of(vm, text).into_owned();
    let size = length(
        &text,
        &mut Sizing {
            font_size: number_or(font_size, 16.0),
            rem: number_or(rem, 16.0),
            vm,
        },
    );
    Ok(size_value(vm, size))
}

fn host_color(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let text = args.get(vm, 0);
    let found = color(&text_of(vm, text));
    Ok(match found {
        Some(found) => vm.new_string(&found),
        None => Value::UNDEFINED,
    })
}

fn strings(vm: &mut Vm, parts: Vec<&str>) -> Value {
    let values = parts.into_iter().map(|part| vm.new_string(part)).collect();
    vm.new_array(values)
}

fn host_words(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let text = args.get(vm, 0);
    let text = text_of(vm, text).into_owned();
    Ok(strings(vm, words(&text)))
}

fn host_commas(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let text = args.get(vm, 0);
    let text = text_of(vm, text).into_owned();
    Ok(strings(vm, commas(&text)))
}

fn block_value(vm: &mut Vm, block: &Block) -> Result<Value, Value> {
    let declarations: Vec<Value> = block
        .declarations
        .iter()
        .map(|(name, value, important)| {
            let mut items = vec![vm.new_string(name), vm.new_string(value)];
            if *important {
                items.push(Value::TRUE);
            }
            vm.new_array(items)
        })
        .collect();
    let declarations = vm.new_array(declarations);
    let mut rules = Vec::with_capacity(block.rules.len());
    let (selector_name, block_name) = (vm.atom("selector"), vm.atom("block"));
    for (selector, inner) in &block.rules {
        let rule = vm.new_object();
        let selector = vm.new_string(selector);
        vm.set_property(rule, selector_name, selector)?;
        let inner = block_value(vm, inner)?;
        vm.set_property(rule, block_name, inner)?;
        rules.push(rule);
    }
    let rules = vm.new_array(rules);
    let out = vm.new_object();
    let (declarations_name, rules_name) = (vm.atom("declarations"), vm.atom("rules"));
    vm.set_property(out, declarations_name, declarations)?;
    vm.set_property(out, rules_name, rules)?;
    Ok(out)
}

/// `readCss(text)`: `{ declarations: [[name, value, true?]], rules: [{ selector, block }] }`.
fn host_read_css(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let text = args.get(vm, 0);
    let block = read_css(&text_of(vm, text));
    block_value(vm, &block)
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 5] = [
    ("cssLength", host_length),
    ("cssColor", host_color),
    ("cssWords", host_words),
    ("cssCommas", host_commas),
    ("cssRead", host_read_css),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// What the JavaScript this replaced gives, and what this gives, for every bit of CSS in this crate's tests, in `web.js`, and
    /// for text made up at random from what CSS is made of.
    #[test]
    fn everything_comes_out_as_it_did_in_javascript() {
        let mut engine = rsui_js::Engine::new();
        let vm = engine.vm();
        let natives = vm.new_object();
        for (name, function) in FUNCTIONS {
            let function = vm.host_function(name, 0, function);
            let atom = vm.atom(name);
            vm.set_property(natives, atom, function).unwrap();
        }
        vm.set_global("natives", natives);
        let corpus = [
            include_str!("web.js"),
            include_str!("../tests/web.rs"),
            include_str!("../tests/app.rs"),
        ];
        let texts = corpus.iter().map(|text| vm.new_string(text)).collect();
        let texts = vm.new_array(texts);
        vm.set_global("corpus", texts);
        // In one script, as what one script declares with `const` isn't seen by the next.
        let reference = include_str!("../tests/fixtures/css_reference.js");
        let failures = engine
            .eval(
                &(reference.to_owned()
                    + r#"
                const failures = [];
                const same = (a, b) => typeof a === typeof b && (Object.is(a, b) || JSON.stringify(a) === JSON.stringify(b));
                const check = (what, input, old, now) => {
                    if (!same(old, now) && failures.length < 40) {
                        failures.push(what + ' ' + JSON.stringify(input) + ': ' + JSON.stringify(old) + ' and now ' + JSON.stringify(now));
                    }
                };
                const values = new Set();
                const gather = (block) => {
                    for (const [name, value] of block.declarations) {
                        values.add(value);
                        values.add(name);
                    }
                    for (const rule of block.rules) {
                        values.add(rule.selector);
                        gather(rule.block);
                    }
                };
                for (const text of corpus) {
                    check('readCss', text.slice(0, 80), readCss(text), natives.cssRead(text));
                    gather(readCss(text));
                    // Pieces of it, which start and end anywhere.
                    for (let at = 0; at < text.length; at += 997) {
                        const piece = text.slice(at, at + 1500);
                        check('readCss', piece.slice(0, 80), readCss(piece), natives.cssRead(piece));
                    }
                }
                // Made up, from what CSS is made of, the same every time.
                let seed = 12345;
                const random = (below) => {
                    seed = (seed * 1103515245 + 12345) % 2147483648;
                    return seed % below;
                };
                const pieces = ['0', '1', '5', '12', '.', '-', '+', ' ', '  ', '*', '/', '(', ')', ',', '%', 'px', 'rem', 'em', 'pt', 'vw', 'vh',
                    'vmin', 'vmax', 'calc(', 'min(', 'max(', 'clamp(', '#', 'fff', 'a0b1c2', 'rgb(', 'rgba(', 'hsl(', 'hsla(', 'red', 'Red',
                    'transparent', 'NaN', 'e', 'e3', '1e3', 'Infinity', '!important', '! IMPORTANT', ';', ':', '{', '}', '"', "'", String.fromCharCode(92),
                    '\n', '\t', '/*', '*/', '//', 'url(', 'a', 'b:', 'color', String.fromCharCode(0xa0), String.fromCharCode(0x2028),
                    String.fromCharCode(0xe9), String.fromCharCode(0xd83d, 0xde00), 'constructor-'];
                for (let round = 0; round < 6000; round += 1) {
                    let text = '';
                    const count = 1 + random(9);
                    for (let at = 0; at < count; at += 1) {
                        text += pieces[random(pieces.length)];
                    }
                    values.add(text);
                    values.add(text.trim());
                    check('readCss', text, readCss(text), natives.cssRead(text));
                }
                for (const value of [...values]) {
                    for (const word of words(value)) {
                        values.add(word);
                    }
                }
                for (const value of values) {
                    check('words', value, words(value), natives.cssWords(value));
                    check('commas', value, commas(value), natives.cssCommas(value));
                    // The JavaScript never had CSS's newer colors, which `tests/web.rs` checks against a browser instead.
                    if (!/^\s*(color-mix|oklch|oklab|hwb)\(/i.test(value)) {
                        check('color', value, color(value), natives.cssColor(value));
                    }
                    // Nor `min()`, `max()` and `clamp()` of shares, or in a `calc()`, which the tests below check instead.
                    if (/(min|max|clamp)\(/i.test(value)) {
                        continue;
                    }
                    for (const [size, rem] of [[16, 16], [10, 10], [undefined, 16]]) {
                        remSize = rem;
                        check('length ' + size + '/' + rem, value, length(value, size), natives.cssLength(value, size, rem));
                    }
                    globalThis.__rsuiPageSize = [1000, 700];
                    check('length on a page', value, length(value, 13), natives.cssLength(value, 13, 16));
                    delete globalThis.__rsuiPageSize;
                }
                check('a number', 5, length(5), natives.cssLength(5, 16, 16));
                failures.push(values.size + ' values');
                failures.join('\n')
                "#),
                "compare",
            )
            .unwrap();
        let failures = engine.display(failures);
        let lines: Vec<&str> = failures.lines().collect();
        assert_eq!(lines.len(), 1, "{failures}");
        assert!(
            lines[0].starts_with(|c: char| c.is_ascii_digit()),
            "{failures}"
        );
        println!("{failures}");
    }

    /// What a size comes to, with text 10 pixels high and a rem of 16.
    fn size_of(text: &str) -> Size {
        let mut engine = rsui_js::Engine::new();
        length(
            text,
            &mut Sizing {
                font_size: 10.0,
                rem: 16.0,
                vm: engine.vm(),
            },
        )
    }

    fn text(text: &str) -> Size {
        Size::Text(text.to_owned())
    }

    #[test]
    fn min_max_and_clamp_of_shares_are_kept_for_what_lays_them_out() {
        assert_eq!(size_of("min(640px, calc(100% - 48px))"), text("min(640px, 100% + -48px)"));
        assert_eq!(size_of("max(100px, 20%)"), text("max(20%, 100px)"));
        assert_eq!(size_of("clamp(50px, 50%, 300px)"), text("clamp(50px, 50%, 300px)"));
        assert_eq!(size_of("clamp(1rem, 2.5% + 1em, 3rem)"), text("clamp(16px, 2.5% + 10px, 48px)"));
        // What's the same share comes together, so that a `min()` of three is still two.
        assert_eq!(size_of("min(640px, 100% - 48px, 40rem, 10em)"), text("min(100px, 100% + -48px)"));
        assert_eq!(size_of("max(min(10%, 50px), 20px)"), text("clamp(20px, 10%, 50px)"));
        // In a `calc()`, added to and times a number, and the other way around taken from something.
        assert_eq!(size_of("calc(min(50%, 300px) + 2px)"), text("min(50% + 2px, 302px)"));
        assert_eq!(size_of("calc(2 * min(50%, 300px))"), text("min(100%, 600px)"));
        assert_eq!(size_of("calc(100% - max(16px, 10%))"), text("min(100% + -16px, 90%)"));
        assert_eq!(size_of("calc((100% - min(1200px, 100%)) / 2)"), text("max(0px, 50% + -600px)"));
        // Two that can't be said as one aren't anything.
        assert_eq!(size_of("min(10%, 20% + 1px, 30% + 2px)"), Size::None);
        assert_eq!(size_of("calc(min(10%, 5px) + max(20%, 1px))"), Size::None);
        assert_eq!(size_of("clamp(1px, 2px)"), Size::None);
    }

    #[test]
    fn min_max_and_clamp_of_pixels_are_worked_out() {
        assert_eq!(size_of("min(10px, 2em, 3rem)"), Size::Pixels(10.0));
        assert_eq!(size_of("max(10px, 2em, 3rem)"), Size::Pixels(48.0));
        assert_eq!(size_of("clamp(1rem, 50px, 2rem)"), Size::Pixels(32.0));
        // The least wins where it's more than the most, as CSS has it.
        assert_eq!(size_of("clamp(30px, 10px, 20px)"), Size::Pixels(30.0));
        assert_eq!(size_of("calc(min(2, 3) * 4px)"), Size::Pixels(8.0));
        assert_eq!(size_of("min(100%, infinity * 1px)"), text("100%"));
        assert_eq!(size_of("calc(1px + min(10%, 10%))"), text("10% + 1px"));
    }

    #[test]
    fn named_colors_are_in_order_to_be_found() {
        assert!(NAMED.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(NAMED.len(), 149);
    }

    #[test]
    fn numbers_are_read_as_parse_float_reads_them() {
        assert_eq!(parse_float("  12.5px"), 12.5);
        assert_eq!(parse_float(".5"), 0.5);
        assert_eq!(parse_float("-1e3x"), -1000.0);
        assert_eq!(parse_float("1e"), 1.0);
        assert_eq!(parse_float("5."), 5.0);
        assert!(parse_float(".").is_nan());
        assert!(parse_float("abc").is_nan());
        assert_eq!(parse_float("-Infinity"), f64::NEG_INFINITY);
    }
}
