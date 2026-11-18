//! Lengths worked out from the size of what they're in, as CSS's percentages are: a share of it and some pixels more, and that
//! kept between a least and a most, as CSS's `min()`, `max()` and `clamp()` say.

use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::sync::{LazyLock, Mutex};

/// A share of what something's in and some pixels more, as CSS's `calc(100% - 14px)` is: `percent` of it, and `pixels`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub percent: f32,
    pub pixels: f32,
}

impl Line {
    /// No share, only pixels.
    pub const fn pixels(pixels: f32) -> Self {
        Self { percent: 0.0, pixels }
    }

    /// How long it is in something `base` pixels long.
    pub fn at(self, base: f32) -> f32 {
        base * self.percent / 100.0 + self.pixels
    }

    /// `"25%"`, `"50% + -5px"`, `"12px"` or `"12"`, as a length is written as a prop.
    fn read(text: &str) -> Option<Self> {
        let text = text.trim();
        let number = |text: &str| text.trim().parse::<f32>().ok();
        let pixels = |text: &str| number(text.trim().strip_suffix("px").unwrap_or(text));
        match text.split_once('%') {
            Some((percent, "")) => Some(Self { percent: number(percent)?, pixels: 0.0 }),
            Some((percent, more)) => Some(Self { percent: number(percent)?, pixels: pixels(more.trim().strip_prefix('+')?)? }),
            None => pixels(text).map(Self::pixels),
        }
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.percent, self.pixels) {
            (percent, _) if percent == 0.0 => write!(f, "{}px", self.pixels),
            (percent, pixels) if pixels == 0.0 => write!(f, "{percent}%"),
            (percent, pixels) => write!(f, "{percent}% + {pixels}px"),
        }
    }
}

/// A [`Line`] kept between a least and a most, as CSS's `clamp(50px, 50%, 300px)` is: the bigger of the least and the smaller of
/// the other two, so that the least wins where they cross, as CSS has it. `min()` and `max()` of two lines are one with no least,
/// or no most, which is an endless number of pixels. A plain line is one with neither.
///
/// As a prop it's written as CSS writes those, with each line as [`Line`] writes it: `"min(640px, 100% + -48px)"`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clamp {
    pub least: Line,
    pub value: Line,
    pub most: Line,
}

impl Clamp {
    /// Just a line, with no least or most.
    pub const fn line(value: Line) -> Self {
        Self { least: Line::pixels(f32::NEG_INFINITY), value, most: Line::pixels(f32::INFINITY) }
    }

    /// How long it is in something `base` pixels long.
    pub fn at(self, base: f32) -> f32 {
        self.value.at(base).min(self.most.at(base)).max(self.least.at(base))
    }

    /// This and some pixels more, which is each of its lines with them, as the least and the most of what's all that much longer
    /// are each that much longer. A least or a most that isn't there stays so.
    pub fn plus(self, more: f32) -> Self {
        let plus = |line: Line| Line { pixels: line.pixels + more, ..line };
        Self { least: plus(self.least), value: plus(self.value), most: plus(self.most) }
    }

    /// Whether any of it is a share of what it's in, which needs that to be known.
    pub fn has_share(self) -> bool {
        self.least.percent != 0.0 || self.value.percent != 0.0 || self.most.percent != 0.0
    }

    /// What a prop says, as [`Line`] reads it, or `min()`, `max()` or `clamp()` of those, as this writes them.
    pub fn read(text: &str) -> Option<Self> {
        let text = text.trim();
        let Some((name, inside)) = text.strip_suffix(')').and_then(|text| text.split_once('(')) else {
            return Line::read(text).map(Self::line);
        };
        let lines: Vec<Line> = inside.split(',').map(Line::read).collect::<Option<_>>()?;
        let none = Self::line(Line::pixels(0.0));
        match (name, lines.as_slice()) {
            ("min", [value, most]) => Some(Self { value: *value, most: *most, ..none }),
            ("max", [least, value]) => Some(Self { least: *least, value: *value, ..none }),
            ("clamp", [least, value, most]) => Some(Self { least: *least, value: *value, most: *most }),
            _ => None,
        }
    }
}

impl fmt::Display for Clamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (least, value, most) = (self.least, self.value, self.most);
        match (least.pixels == f32::NEG_INFINITY, most.pixels == f32::INFINITY) {
            (true, true) => write!(f, "{value}"),
            (true, false) => write!(f, "min({value}, {most})"),
            (false, true) => write!(f, "max({least}, {value})"),
            (false, false) => write!(f, "clamp({least}, {value}, {most})"),
        }
    }
}

/// Part of the way from one [`Clamp`] to another, as a transition between widths with `min()`, `max()` or `clamp()` in them is
/// part way through: each is worked out in what it's in, and it's `along` of the way from what the one comes to to what the
/// other does, as a browser mixes them, rather than each of their lines.
///
/// As a prop it's written `"mix(0.25, min(640px, 100% + -48px), clamp(50px, 50%, 300px))"`, each end as [`Clamp`] writes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mix {
    pub along: f32,
    pub from: Clamp,
    pub to: Clamp,
}

impl Mix {
    /// How long it is in something `base` pixels long.
    pub fn at(self, base: f32) -> f32 {
        let from = self.from.at(base);
        from + (self.to.at(base) - from) * self.along
    }

    /// Whether either end is a share of what it's in, which needs that to be known.
    pub fn has_share(self) -> bool {
        self.from.has_share() || self.to.has_share()
    }

    /// This and some pixels more, which is each end with them.
    pub fn plus(self, more: f32) -> Self {
        Self { from: self.from.plus(more), to: self.to.plus(more), ..self }
    }

    /// What a prop says, as this writes it.
    pub fn read(text: &str) -> Option<Self> {
        let inside = text.trim().strip_prefix("mix(")?.strip_suffix(')')?;
        let (along, ends) = inside.split_once(',')?;
        // The ends have commas in them too, inside their brackets: the one between them is outside every bracket.
        let mut depth = 0;
        let middle = ends.char_indices().find_map(|(at, c)| {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => return Some(at),
                _ => {}
            }
            None
        })?;
        let (from, to) = (&ends[..middle], &ends[middle + 1..]);
        Some(Self { along: along.trim().parse().ok()?, from: Clamp::read(from)?, to: Clamp::read(to)? })
    }
}

impl fmt::Display for Mix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "mix({}, {}, {})", self.along, self.from, self.to)
    }
}

/// A value kept once for everything that says it, which they only point to. A length is copied into every node's style, and
/// what `min()`, `max()` and `clamp()` say is many numbers; kept this way, a length that says one is no bigger than one that's
/// two numbers. They're only made as styles are read, and the same one only once, so there are only as many as there are
/// different ones said, which a page has few of. Two are the same when they're the same one.
pub struct Kept<T: 'static>(&'static T);

impl<T> Clone for Kept<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Kept<T> {}

impl<T> PartialEq for Kept<T> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl<T> Deref for Kept<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.0
    }
}

impl<T: fmt::Debug> fmt::Debug for Kept<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The one kept for `value`, whose numbers' bits are `key`, made the first time it's asked for.
fn keep<T: Send + Sync, const N: usize>(kept: &Mutex<HashMap<[u32; N], &'static T>>, key: [u32; N], value: T) -> Kept<T> {
    let mut kept = kept.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Kept(kept.entry(key).or_insert_with(|| Box::leak(Box::new(value))))
}

fn bits<const N: usize>(numbers: [f32; N]) -> [u32; N] {
    numbers.map(f32::to_bits)
}

impl Clamp {
    /// The one kept for this.
    pub fn kept(self) -> Kept<Clamp> {
        static KEPT: LazyLock<Mutex<HashMap<[u32; 6], &'static Clamp>>> = LazyLock::new(Mutex::default);
        keep(&KEPT, bits(self.numbers()), self)
    }

    fn numbers(&self) -> [f32; 6] {
        let Self { least, value, most } = self;
        [least.percent, least.pixels, value.percent, value.pixels, most.percent, most.pixels]
    }
}

/// The two ends of a [`Mix`], kept as one (see [`Kept`]): a mix is made each frame of a transition, with the same ends and only
/// how far along it is changing.
pub type Ends = (Clamp, Clamp);

impl Mix {
    /// Its ends, kept, and how far along it is, which is what a length holds of it.
    pub fn kept(self) -> (f32, Kept<Ends>) {
        static KEPT: LazyLock<Mutex<HashMap<[u32; 12], &'static Ends>>> = LazyLock::new(Mutex::default);
        let [a, b, c, d, e, f] = self.from.numbers();
        let [g, h, i, j, k, l] = self.to.numbers();
        (self.along, keep(&KEPT, bits([a, b, c, d, e, f, g, h, i, j, k, l]), (self.from, self.to)))
    }

    /// The mix a length holds as how far along it is and its kept ends.
    pub fn of(along: f32, ends: Kept<Ends>) -> Self {
        let (from, to) = *ends;
        Self { along, from, to }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_kept_is_kept_once_and_a_length_holding_it_stays_small() {
        let clamp = Clamp::read("clamp(50px, 50%, 300px)").unwrap();
        assert_eq!(clamp.kept(), Clamp::read("clamp(50px, 50%, 300px)").unwrap().kept());
        assert_ne!(clamp.kept(), Clamp::read("clamp(50px, 40%, 300px)").unwrap().kept());
        assert_eq!(*clamp.kept(), clamp);
        let mix = Mix::read("mix(0.25, min(640px, 100% + -48px), 20px)").unwrap();
        let (along, ends) = mix.kept();
        assert_eq!(Mix::of(along, ends), mix);
        assert_eq!(Mix { along: 0.75, ..mix }.kept().1, ends, "a mix further along has the same ends");
        assert!(std::mem::size_of::<crate::Length>() <= 16);
    }

    #[test]
    fn lines_are_read_as_props_write_them() {
        let line = |text| Clamp::read(text).map(|clamp| clamp.value);
        assert_eq!(line("25%"), Some(Line { percent: 25.0, pixels: 0.0 }));
        assert_eq!(line("50% + -5px"), Some(Line { percent: 50.0, pixels: -5.0 }));
        assert_eq!(line("12px"), Some(Line::pixels(12.0)));
        assert_eq!(line("12"), Some(Line::pixels(12.0)));
        assert_eq!(line("50% 5px"), None);
        assert_eq!(line("auto"), None);
        assert_eq!(Clamp::read("50% + -5px").map(|clamp| clamp.at(200.0)), Some(95.0));
    }

    #[test]
    fn min_max_and_clamp_are_worked_out_in_what_they_are_in() {
        let at = |text: &str, base: f32| Clamp::read(text).map(|clamp| clamp.at(base));
        // `width: min(640px, calc(100% - 48px))`, in something 800 and 400 pixels wide.
        assert_eq!(at("min(640px, 100% + -48px)", 800.0), Some(640.0));
        assert_eq!(at("min(640px, 100% + -48px)", 400.0), Some(352.0));
        assert_eq!(at("max(100px, 20%)", 1000.0), Some(200.0));
        assert_eq!(at("max(100px, 20%)", 300.0), Some(100.0));
        assert_eq!(at("clamp(50px, 50%, 300px)", 1000.0), Some(300.0));
        assert_eq!(at("clamp(50px, 50%, 300px)", 400.0), Some(200.0));
        assert_eq!(at("clamp(50px, 50%, 300px)", 40.0), Some(50.0));
        // Where the least is more than the most, it wins.
        assert_eq!(at("clamp(50%, 10px, 20px)", 100.0), Some(50.0));
        assert_eq!(at("min(1px, 2px, 3px)", 100.0), None);
        // With pixels more, as padding outside it makes it, each of its lines has them.
        let wider = Clamp::read("clamp(50px, 50%, 300px)").map(|clamp| clamp.plus(20.0));
        assert_eq!(wider.map(|clamp| clamp.at(400.0)), Some(220.0));
        assert_eq!(wider.map(|clamp| clamp.at(40.0)), Some(70.0));
        assert_eq!(wider.map(|clamp| clamp.at(1000.0)), Some(320.0));
        assert_eq!(Clamp::read("min(640px, 100%)").map(|clamp| clamp.plus(8.0).to_string()).as_deref(), Some("min(648px, 100% + 8px)"));
        assert_eq!(at("calc(1px)", 100.0), None);
    }

    #[test]
    fn a_mix_is_part_way_from_what_one_end_comes_to_to_what_the_other_does() {
        let at = |text: &str, base: f32| Mix::read(text).map(|mix| mix.at(base));
        // From `min(300px, 50%)` to `clamp(400px, 60%, 800px)`, which keep to different lines in 1000px: 300 and 600, so a
        // quarter of the way is 375, which mixing each of their lines wouldn't give.
        let text = "mix(0.25, min(300px, 50%), clamp(400px, 60%, 800px))";
        assert_eq!(at(text, 1000.0), Some(375.0));
        // In 400px, 200 and 400.
        assert_eq!(at(text, 400.0), Some(250.0));
        // An end can be plain pixels or a share.
        assert_eq!(at("mix(0.5, 20px, 100% + -20px)", 200.0), Some(100.0));
        assert_eq!(Mix::read(text).unwrap().plus(10.0).at(1000.0), 385.0);
        assert!(Mix::read("mix(0.5, 20px, 40px)").is_some_and(|mix| !mix.has_share()));
        assert_eq!(Mix::read(text).unwrap().to_string(), text);
        assert_eq!(Mix::read("mix(0.5, 20px)"), None);
        assert_eq!(Mix::read("mix(half, 20px, 40px)"), None);
        assert_eq!(Mix::read("min(20px, 40px)"), None);
    }

    #[test]
    fn what_is_written_is_read_back() {
        for text in ["min(640px, 100% + -48px)", "max(100px, 20%)", "clamp(50px, 50%, 300px)", "50% + -5px", "12px"] {
            assert_eq!(Clamp::read(text).unwrap().to_string(), text);
        }
    }
}
