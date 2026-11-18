//! Measures text and lays it out.

use std::cell::{Cell, OnceCell};
use std::hash::BuildHasher;
use std::ops::RangeInclusive;
use std::sync::{Arc, LazyLock};

use foldhash::fast::FixedState;
use rsui_core::{Element, PropValue, Role, Symbol};
use rsui_layout::{Measure, Rect, Size, is_paragraph, shows_name};

use crate::complex::{Complexity, LongShaped, ShapedCache};
use crate::font::Font;
use crate::lines::{is_space, next_line};
use crate::ot::{AsciiRules, Plan};
use crate::shape::{Script, plan_for};

/// How many kerning answers to remember for each font. Looking one up in a font takes
/// several binary searches, and text uses few distinct pairs.
const KERNING_CACHE_SIZE: usize = 512;
/// The size text gets if an element doesn't set one.
const DEFAULT_SIZE: f32 = 14.0;

/// Which font of a family to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontStyle {
    #[default]
    Regular,
    Bold,
    /// Lighter than regular, as CSS's 300 and below are.
    Light,
    /// Between light and regular, as CSS's 350 is, which is a family's semilight font if it has one, and light otherwise.
    Semilight,
    /// A little heavier than regular, as CSS's 500 is, which is a family's medium or semibold font if it has one, as Windows
    /// chooses, and regular otherwise.
    Medium,
    /// Between regular and bold, as CSS's 600 is.
    Semibold,
    /// Heavier than bold, as CSS's 800 and above are.
    Black,
}

impl FontStyle {
    /// Whether this is heavier than regular, which is bold where a font only has regular and bold.
    pub const fn is_heavy(self) -> bool {
        matches!(self, Self::Bold | Self::Semibold | Self::Black)
    }

    /// The weight for a CSS `font-weight`, which is `bold`, `normal` or a number. A weight with no font of its own is the one
    /// nearest it that there's most often a font for, as Windows chooses: 500 is medium or semibold, and 600 is semibold, then bold.
    pub fn of_weight(weight: &str) -> Self {
        match weight {
            "bold" | "bolder" => Self::Bold,
            "normal" | "lighter" => Self::Regular,
            _ => match weight.parse::<u16>() {
                Ok(0..=325) => Self::Light,
                Ok(326..=374) => Self::Semilight,
                Ok(375..=450) => Self::Regular,
                Ok(451..=550) => Self::Medium,
                Ok(551..=650) => Self::Semibold,
                Ok(651..=750) => Self::Bold,
                Ok(_) => Self::Black,
                Err(_) => Self::Regular,
            },
        }
    }

    /// What follows a family's name in the names of its fonts for this weight, in the order to look for them, as CSS would
    /// choose between them. `""` is the family's regular font.
    fn names(self) -> &'static [&'static str] {
        match self {
            Self::Regular => &[""],
            Self::Medium => &["Medium", "Semibold", "SemiBold", ""],
            Self::Bold => &["Bold"],
            Self::Light => &["Light", "Semilight", ""],
            Self::Semilight => &["Semilight", "SemiLight", "Light", ""],
            Self::Semibold => &["Semibold", "SemiBold", "Bold"],
            Self::Black => &["Black", "Heavy", "Bold"],
        }
    }

    /// The CSS weight this stands for, which a variable font is set to the place nearest.
    fn css_weight(self) -> f32 {
        match self {
            Self::Light => 300.0,
            Self::Semilight => 350.0,
            Self::Regular => 400.0,
            Self::Medium => 500.0,
            Self::Semibold => 600.0,
            Self::Bold => 700.0,
            Self::Black => 900.0,
        }
    }
}

/// How text should look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// In pixels per em.
    pub size: f32,
    pub font: FontStyle,
    /// A font by name, or a path to one. The usual one if this isn't given, or it can't be found.
    pub family: Option<Symbol>,
    pub italic: bool,
    /// How far apart lines are, in pixels, if it isn't what the font says.
    pub line_height: Option<f32>,
    /// How much further along each character's next one is, in pixels, as CSS's `letter-spacing` has it.
    pub letter_spacing: f32,
    /// How much further along what's after each space is, in pixels, as CSS's `word-spacing` has it.
    pub word_spacing: f32,
    /// How far above the baseline of the text around it this text's baseline is, in pixels, as CSS's `vertical-align: super`
    /// and `sub` have it. Only text inside a paragraph moves.
    pub rise: f32,
    /// Whether a line can end between any two characters, as CSS's `word-break: break-all` has it, rather than only between words.
    pub break_all: bool,
    /// How much further along the first line starts than the rest, in pixels, as CSS's `text-indent` has it.
    pub indent: f32,
    /// How wide what floats at the left of the text is, and how far down it goes, in pixels: lines that start above that are as
    /// much shorter, and start that much further along, as CSS's `float` has text go around what floats.
    pub beside_left: (f32, f32),
    /// The same for what floats at the right, which only makes lines shorter.
    pub beside_right: (f32, f32),
    /// Whether what it's in goes right to left, as CSS's `direction: rtl` says, whatever its own letters are, which is the end that
    /// a line too long for its room is cut off at, at its left.
    pub right_to_left: bool,
    /// Whether which way it goes was said, as CSS's `direction` always is on a page, so that its paragraphs go that way whatever
    /// letter they start with (UAX #9's HL1). Otherwise, and for `unicode-bidi: plaintext`, a paragraph goes the way its first
    /// strong letter does (rules P2 and P3).
    pub direction_said: bool,
    /// Whether every letter goes the way it does, as CSS's `unicode-bidi: bidi-override` has it, whatever they are.
    pub bidi_override: bool,
    /// Whether its box is only as tall as its fonts make it, as CSS's inline box is, rather than as its lines are: the box
    /// starts where its fonts' text does on its first line and ends where it does on its last, and what's drawn in it is where
    /// it would be in the whole lines. See [`TextEngine::inline_area`].
    pub inline_area: bool,
    /// Whether its digits are all as wide as each other, as CSS's `font-variant-numeric: tabular-nums` says, with its font's
    /// `tnum` glyphs.
    pub tabular: bool,
    /// How far its font goes above and below the baseline, as shares of its size, where `@font-face`'s `ascent-override` and
    /// `descent-override` say, rather than what the font says, which is where a line's text sits in it.
    pub metrics: Option<(f32, f32)>,
    /// Whether its lines are as near the same length as they can be, as CSS's `text-wrap: balance` has them: broken as if it had
    /// less room, the least that keeps it to as many lines. See [`TextEngine::balanced_width`].
    pub balance: bool,
}

impl TextStyle {
    pub const fn new(size: f32) -> Self {
        Self {
            size,
            font: FontStyle::Regular,
            family: None,
            italic: false,
            line_height: None,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            rise: 0.0,
            break_all: false,
            indent: 0.0,
            beside_left: (0.0, 0.0),
            beside_right: (0.0, 0.0),
            right_to_left: false,
            direction_said: false,
            bidi_override: false,
            inline_area: false,
            tabular: false,
            metrics: None,
            balance: false,
        }
    }

    /// The level its paragraphs have, as the Unicode Bidirectional Algorithm has them: 1 from the right, 0 from the left, or
    /// `None` to go by their first strong letter.
    pub const fn paragraph_level(&self) -> Option<u8> {
        if self.right_to_left {
            Some(1)
        } else if self.direction_said {
            Some(0)
        } else {
            None
        }
    }

    /// The same, said as it's remembered with what's laid out: 0 for its first letter, 1 from the left, 2 from the right, and 4
    /// more where every letter goes that way.
    pub(crate) const fn bidi_key(&self) -> u8 {
        let level = match self.paragraph_level() {
            None => 0,
            Some(0) => 1,
            Some(_) => 2,
        };
        level + if self.bidi_override { 4 } else { 0 }
    }

    /// The same, in physical pixels rather than logical ones, which is everything that's a size.
    pub fn scaled(mut self, scale: f32) -> Self {
        self.size *= scale;
        self.line_height = self.line_height.map(|height| height * scale);
        self.letter_spacing *= scale;
        self.word_spacing *= scale;
        self.rise *= scale;
        self.indent *= scale;
        self.beside_left = (self.beside_left.0 * scale, self.beside_left.1 * scale);
        self.beside_right = (self.beside_right.0 * scale, self.beside_right.1 * scale);
        self
    }

    pub const fn bold(mut self) -> Self {
        self.font = FontStyle::Bold;
        self
    }

    /// Reads the style from an element's `font_size` and `font_weight` props.
    pub fn from_element(element: &Element) -> Self {
        // Most text has neither, and looking for what isn't there is the common case.
        if element.props.is_empty() {
            return Self::new(DEFAULT_SIZE);
        }
        let names = &*STYLE_NAMES;
        let size = match element.props.get_symbol(names.0) {
            Some(PropValue::Float(size)) => *size as f32,
            Some(PropValue::Int(size)) => *size as f32,
            _ => DEFAULT_SIZE,
        };
        let font = match element.props.get_symbol(names.1) {
            Some(PropValue::Str(weight)) => FontStyle::of_weight(weight.as_str()),
            Some(PropValue::Int(weight)) => FontStyle::of_weight(&weight.to_string()),
            _ => FontStyle::Regular,
        };
        let family = match element.props.get_symbol(names.2) {
            Some(PropValue::Str(family)) => Some(Symbol::new(family.as_str())),
            _ => None,
        };
        let italic = matches!(
            element.props.get_symbol(names.3),
            Some(PropValue::Str(style)) if style.as_str() == "italic"
        );
        let line_height = match element.props.get_symbol(names.4) {
            Some(PropValue::Float(height)) => Some(*height as f32),
            Some(PropValue::Int(height)) => Some(*height as f32),
            _ => None,
        };
        let letter_spacing = match element.props.get_symbol(*LETTER_SPACING) {
            Some(PropValue::Float(spacing)) => *spacing as f32,
            Some(PropValue::Int(spacing)) => *spacing as f32,
            _ => 0.0,
        };
        let word_spacing = match element.props.get_symbol(*WORD_SPACING) {
            Some(PropValue::Float(spacing)) => *spacing as f32,
            Some(PropValue::Int(spacing)) => *spacing as f32,
            _ => 0.0,
        };
        Self {
            size,
            font,
            family,
            italic,
            line_height,
            letter_spacing,
            word_spacing,
            rise: 0.0,
            break_all: matches!(
                element.props.get_symbol(*WORD_BREAK),
                Some(PropValue::Str(how)) if how.as_str() == "break-all"
            ),
            indent: match element.props.get_symbol(*TEXT_INDENT) {
                Some(PropValue::Float(indent)) => *indent as f32,
                Some(PropValue::Int(indent)) => *indent as f32,
                _ => 0.0,
            },
            beside_left: beside(element, *BESIDE_LEFT),
            beside_right: beside(element, *BESIDE_RIGHT),
            right_to_left: matches!(
                element.props.get_symbol(*TEXT_DIRECTION),
                Some(PropValue::Str(way)) if way.as_str() == "rtl"
            ) && !plaintext(element),
            direction_said: matches!(
                element.props.get_symbol(*TEXT_DIRECTION),
                Some(PropValue::Str(way)) if way.as_str() == "rtl" || way.as_str() == "ltr"
            ) && !plaintext(element),
            bidi_override: matches!(
                element.props.get_symbol(*UNICODE_BIDI),
                Some(PropValue::Str(how)) if how.as_str() == "bidi-override" || how.as_str() == "isolate-override"
            ),
            inline_area: matches!(element.props.get_symbol(*INLINE_AREA), Some(PropValue::Bool(true))),
            tabular: matches!(element.props.get_symbol(*TABULAR), Some(PropValue::Bool(true))),
            metrics: metrics_of(element),
            balance: matches!(element.props.get_symbol(*TEXT_WRAP), Some(PropValue::Str(how)) if how.as_str() == "balance"),
        }
    }
}

/// The name of the prop that says how lines are broken: `"balance"` for [`TextStyle::balance`].
static TEXT_WRAP: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("text_wrap"));

/// The most lines a balanced text has for it to be balanced, as Blink's `kMaxLinesToBalance`: more than that is broken as usual.
const MOST_BALANCED_LINES: usize = 6;

/// The name of the prop that says how far a font goes above and below the baseline: see `TextStyle::metrics`.
pub(crate) static FONT_METRICS: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("font_metrics"));

/// What `font_metrics` says, as `"1.05 0.35"`: the ascent and the descent, as shares of the size. Nothing for anything else.
pub(crate) fn metrics_of(element: &Element) -> Option<(f32, f32)> {
    let Some(PropValue::Str(said)) = element.props.get_symbol(*FONT_METRICS) else {
        return None;
    };
    let mut parts = said.as_str().split(' ').map(str::parse::<f32>);
    match (parts.next(), parts.next()) {
        (Some(Ok(ascent)), Some(Ok(descent))) if ascent.is_finite() && descent.is_finite() => {
            Some((ascent, descent))
        }
        _ => None,
    }
}

/// The name of the prop that says digits are all as wide as each other: see `TextStyle::tabular`.
pub(crate) static TABULAR: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("tabular_nums"));

/// The name of the prop that says a text's box is only as tall as its fonts make it: see `TextStyle::inline_area`.
static INLINE_AREA: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("inline_area"));

/// The name of the prop that says how far apart letters are.
static LETTER_SPACING: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("letter_spacing"));

/// The name of the prop that says how far apart words are.
pub(crate) static WORD_SPACING: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("word_spacing"));

/// Whether a character is one that `word-spacing` goes after, as CSS has it: a space, or one that doesn't break.
fn spaces_words(character: char) -> bool {
    matches!(character, ' ' | '\u{a0}' | '\u{1361}' | '\u{10100}' | '\u{10101}' | '\u{1039f}' | '\u{1091f}')
}

/// Spreads out what's been laid out from `text` by `spacing` after each space, as CSS's `word-spacing` does, which makes each
/// line that much longer for each space on it. As with `letter-spacing`, it's laid out as if it weren't there. A line whose
/// glyphs aren't one to each character, as text that's been shaped can have, is left as it is.
pub fn space_words(layout: &mut TextLayout, text: &str, spacing: f32) {
    if spacing == 0.0 {
        return;
    }
    for line in &mut layout.lines {
        let (first, count) = (line.first_glyph as usize, line.glyph_count as usize);
        let Some(piece) = text.get(line.start..line.end) else {
            continue;
        };
        let characters = piece.chars().filter(|character| *character != '\r');
        if characters.clone().count() != count {
            continue;
        }
        let mut shift = 0.0;
        for (glyph, character) in layout.glyphs[first..first + count].iter_mut().zip(characters) {
            glyph.x += shift;
            if spaces_words(character) {
                shift += spacing;
            }
        }
        line.width += shift;
    }
}

/// How many spaces that `word-spacing` goes after the line of `text` with the most of them has.
fn widest_line_spaces(text: &str) -> usize {
    text.split('\n')
        .map(|line| {
            line.chars()
                .filter(|character| spaces_words(*character))
                .count()
        })
        .max()
        .unwrap_or(0)
}

/// Spreads out what's been laid out by `spacing` after each glyph, as CSS's `letter-spacing` does, which makes each line that
/// much longer for each glyph on it. Text is laid out as if it weren't there, as only short text tends to have any.
pub fn space_letters(layout: &mut TextLayout, spacing: f32) {
    if spacing == 0.0 {
        return;
    }
    for line in &mut layout.lines {
        let (first, count) = (line.first_glyph as usize, line.glyph_count as usize);
        for (at, glyph) in layout.glyphs[first..first + count].iter_mut().enumerate() {
            glyph.x += at as f32 * spacing;
        }
        line.width += count as f32 * spacing;
    }
}

/// How many lines text shows at most, if it says, as CSS's `line-clamp` does, the last of them ending in `…` if there's more.
pub fn max_lines(element: &Element) -> Option<usize> {
    static MAX_LINES: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("max_lines"));
    if element.props.is_empty() {
        return None;
    }
    match element.props.get_symbol(*MAX_LINES)? {
        PropValue::Int(lines) if *lines > 0 => Some(*lines as usize),
        PropValue::Float(lines) if *lines >= 1.0 => Some(*lines as usize),
        _ => None,
    }
}

/// The names of the props that style text, looked up once.
/// What keeps text to one line, as `ellipsis` and `no_wrap`.
static WORD_BREAK: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("word_break"));
/// The name of the prop that says which way what text is in goes.
static TEXT_DIRECTION: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("text_direction"));
/// The name of the prop that says how its letters' directions are worked out, as CSS's `unicode-bidi`.
pub(crate) static UNICODE_BIDI: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("unicode_bidi"));

/// Whether its paragraphs go the way their first strong letter does, as CSS's `unicode-bidi: plaintext` has them.
fn plaintext(element: &Element) -> bool {
    matches!(element.props.get_symbol(*UNICODE_BIDI), Some(PropValue::Str(how)) if how.as_str() == "plaintext")
}
/// The name of the prop that says how much further along the first line starts.
static TEXT_INDENT: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("text_indent"));
/// The names of the props that say how wide what floats beside text is, and how far down it goes.
static BESIDE_LEFT: LazyLock<[Symbol; 2]> =
    LazyLock::new(|| [Symbol::new("beside_left"), Symbol::new("beside_left_below")]);
static BESIDE_RIGHT: LazyLock<[Symbol; 2]> = LazyLock::new(|| {
    [
        Symbol::new("beside_right"),
        Symbol::new("beside_right_below"),
    ]
});

/// What props say about what floats beside text, as `(width, below)`, or nothing.
fn beside(element: &Element, names: [Symbol; 2]) -> (f32, f32) {
    let number = |name| match element.props.get_symbol(name) {
        Some(PropValue::Float(number)) => *number as f32,
        Some(PropValue::Int(number)) => *number as f32,
        _ => 0.0,
    };
    (number(names[0]), number(names[1]))
}

static ONE_LINE_NAMES: LazyLock<(Symbol, Symbol, Symbol)> = LazyLock::new(|| {
    (
        Symbol::new("ellipsis"),
        Symbol::new("no_wrap"),
        Symbol::new("keep_words"),
    )
});

pub(crate) static STYLE_NAMES: LazyLock<(Symbol, Symbol, Symbol, Symbol, Symbol)> =
    LazyLock::new(|| {
        (
            Symbol::new("font_size"),
            Symbol::new("font_weight"),
            Symbol::new("font_family"),
            Symbol::new("font_style"),
            Symbol::new("line_height"),
        )
    });

/// Text at least this long remembers where its lines break, and only places what's on screen.
const LONG_TEXT: usize = 2048;
/// How many long texts to remember the lines of.
const LINE_CACHE_SIZE: usize = 64;
/// Bound remembered line breaks and shaped lines by bytes as well as entry count.
/// One active document may exceed this; keeping its breaks avoids redoing the
/// entire document on each scroll, but it must not retain other large documents.
const LINE_CACHE_BYTES: usize = 2 * 1024 * 1024;
/// How long plain text has to be for its line breaks to be worth remembering. Finding them again each
/// frame costs more than looking them up from about here.
const REMEMBERED_LINES_FROM: usize = 256;
/// How many shaped lines of a long text to remember, which is more than can fit on a screen. Going
/// from one frame to the next, they're nearly all the same ones.
const SHAPED_LINES: usize = 128;

/// One line of a long text that took shaping to place.
#[derive(Clone, PartialEq)]
struct ShapedLine {
    line: u32,
    x: f32,
    /// With their baseline at 0.
    glyphs: Box<[PlacedGlyph]>,
}

/// Where one long text's lines break, at one size and width.
#[derive(Clone, PartialEq)]
struct CachedLines {
    /// The hash of the text, as an address could be reused for something different.
    text: u64,
    length: usize,
    size: u32,
    primary: usize,
    width: u32,
    anywhere: bool,
    /// Which way its paragraphs go, as [`TextStyle::bidi_key`] says.
    bidi: u8,
    /// Start, end and width of each line. Numbering them gives the rest.
    lines: Vec<(u32, u32, f32)>,
    complexity: Complexity,
    /// For text that needs reordering, what's needed to do that for one line.
    shaped: Option<LongShaped>,
    /// Lines that took shaping to place, each in the slot its number says.
    shaped_lines: Vec<Option<ShapedLine>>,
    /// Heap capacity retained by this entry, refreshed only when its data changes.
    retained_bytes: usize,
}

impl CachedLines {
    fn update_retained_bytes(&mut self) {
        self.retained_bytes = self.lines.capacity() * size_of::<(u32, u32, f32)>()
            + self.shaped.as_ref().map_or(0, LongShaped::bytes)
            + self.shaped_lines.capacity() * size_of::<Option<ShapedLine>>()
            + self
                .shaped_lines
                .iter()
                .flatten()
                .map(|line| line.glyphs.len() * size_of::<PlacedGlyph>())
                .sum::<usize>();
    }
}

/// Keeps the current entry's index valid while discarding older derived data.
/// This runs only after inserting or extending an entry, never on a cache hit.
fn trim_line_cache(entries: &mut Vec<CachedLines>, mut keep: usize) -> usize {
    let mut bytes: usize = entries.iter().map(|entry| entry.retained_bytes).sum();
    while bytes > LINE_CACHE_BYTES && entries.len() > 1 {
        let victim = if keep == 0 { 1 } else { 0 };
        bytes -= entries[victim].retained_bytes;
        entries.remove(victim);
        if victim < keep {
            keep -= 1;
        }
    }
    keep
}

#[cfg(test)]
mod line_cache_tests {
    use super::*;

    fn entry(id: u64, bytes: usize) -> CachedLines {
        let mut entry = CachedLines {
            text: id,
            length: 0,
            size: 0,
            primary: 0,
            width: 0,
            anywhere: false,
            bidi: 0,
            lines: Vec::with_capacity(bytes.div_ceil(size_of::<(u32, u32, f32)>())),
            complexity: Complexity::Plain,
            shaped: None,
            shaped_lines: Vec::new(),
            retained_bytes: 0,
        };
        entry.update_retained_bytes();
        entry
    }

    #[test]
    fn changing_large_documents_keeps_only_a_bounded_history() {
        let mut entries = Vec::new();
        for id in 0..12 {
            entries.push(entry(id, LINE_CACHE_BYTES / 3));
            let keep = entries.len() - 1;
            let keep = trim_line_cache(&mut entries, keep);
            assert_eq!(entries[keep].text, id);
            assert!(
                entries
                    .iter()
                    .map(|entry| entry.retained_bytes)
                    .sum::<usize>()
                    <= LINE_CACHE_BYTES
            );
        }
        assert!(entries.len() > 1, "small recent documents stay warm");
    }

    #[test]
    fn one_oversized_document_is_kept_until_another_document_replaces_it() {
        let mut entries = vec![entry(1, 100), entry(2, LINE_CACHE_BYTES * 2)];
        let keep = trim_line_cache(&mut entries, 1);
        assert_eq!(keep, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[keep].text, 2);
        entries.push(entry(3, 100));
        let keep = trim_line_cache(&mut entries, 1);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[keep].text, 3);
    }

    #[test]
    fn newly_remembered_glyphs_count_toward_the_document_budget() {
        let mut entries = vec![entry(1, LINE_CACHE_BYTES / 2), entry(2, 100)];
        entries[1].shaped_lines.push(Some(ShapedLine {
            line: 0,
            x: 0.0,
            glyphs: vec![
                PlacedGlyph {
                    font: 0,
                    glyph: 0,
                    x: 0.0,
                    y: 0.0,
                };
                LINE_CACHE_BYTES / size_of::<PlacedGlyph>()
            ]
            .into_boxed_slice(),
        }));
        entries[1].update_retained_bytes();
        let keep = trim_line_cache(&mut entries, 1);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[keep].text, 2);
        assert!(entries[keep].retained_bytes > LINE_CACHE_BYTES);
    }
}

/// A font with what's needed to read it quickly.
pub(crate) struct Face {
    /// Shared with the same font with digits all as wide as each other, if that's been asked for: see `tabular_variant`.
    pub(crate) font: Arc<Font>,
    /// The glyph and advance for each ASCII character, which is nearly all text.
    ascii: Box<[(u16, f32); 128]>,
    kerning: Box<[(u32, f32); KERNING_CACHE_SIZE]>,
    /// What ASCII text could do in this font, worked out when first needed.
    ascii_rules: Option<Box<AsciiRules>>,
    /// What to run for each script and direction that's come up.
    pub(crate) plans: Vec<(Script, bool, Plan)>,
}

/// A pair that no font can ask about, since glyph 0xFFFF can't exist.
const NO_PAIR: u32 = u32::MAX;

impl Face {
    fn new(font: Font) -> Self {
        let mut ascii = Box::new([(0u16, 0f32); 128]);
        for (code, entry) in ascii.iter_mut().enumerate() {
            let glyph = font.glyph(code as u8 as char).unwrap_or(0);
            *entry = (glyph, f32::from(font.advance(glyph)));
        }
        Self {
            font: Arc::new(font),
            ascii,
            kerning: Box::new([(NO_PAIR, 0.0); KERNING_CACHE_SIZE]),
            ascii_rules: None,
            plans: Vec::new(),
        }
    }

    /// The same font with its digits as its `tnum` feature has them, all as wide as each other, if it has that.
    fn tabular_variant(&self) -> Option<Self> {
        let digits: Vec<u16> = (b'0'..=b'9').map(|digit| self.ascii[digit as usize].0).collect();
        let made = crate::ot::single_substitutes(self.font.shaping_tables().gsub, b"tnum", &digits);
        if made == digits {
            return None;
        }
        let mut ascii = self.ascii.clone();
        for (digit, glyph) in (b'0'..=b'9').zip(made) {
            ascii[digit as usize] = (glyph, f32::from(self.font.advance(glyph)));
        }
        Some(Self {
            font: Arc::clone(&self.font),
            ascii,
            kerning: Box::new([(NO_PAIR, 0.0); KERNING_CACHE_SIZE]),
            ascii_rules: None,
            plans: Vec::new(),
        })
    }

    pub(crate) fn ascii_rules(&mut self) -> &AsciiRules {
        if self.ascii_rules.is_none() {
            let mut glyphs = [0u16; 128];
            for (glyph, entry) in glyphs.iter_mut().zip(self.ascii.iter()) {
                *glyph = entry.0;
            }
            let rules = AsciiRules::read(self.font.shaping_tables().gsub, &glyphs);
            self.ascii_rules = Some(Box::new(rules));
        }
        self.ascii_rules.as_deref().expect("it was just filled in")
    }

    /// Where in `plans` the plan for this script and direction is.
    pub(crate) fn plan(&mut self, script: Script, right_to_left: bool) -> usize {
        if let Some(found) = self
            .plans
            .iter()
            .position(|(known, direction, _)| *known == script && *direction == right_to_left)
        {
            return found;
        }
        let plan = plan_for(self.font.shaping_tables(), script, right_to_left);
        self.plans.push((script, right_to_left, plan));
        self.plans.len() - 1
    }

    pub(crate) fn glyph(&self, character: char) -> u16 {
        if character.is_ascii() {
            self.ascii[character as usize].0
        } else {
            self.font.glyph(character).unwrap_or(0)
        }
    }

    fn advance_units(&self, character: char, glyph: u16) -> f32 {
        if character.is_ascii() {
            self.ascii[character as usize].1
        } else {
            f32::from(self.font.advance(glyph))
        }
    }

    fn kerning(&mut self, left: u16, right: u16) -> f32 {
        let pair = (u32::from(left) << 16) | u32::from(right);
        let slot = (pair.wrapping_mul(0x9E37_79B1) >> 23) as usize % KERNING_CACHE_SIZE;
        let entry = &mut self.kerning[slot];
        if entry.0 == pair {
            return entry.1;
        }
        let value = self.font.kerning(left, right);
        *entry = (pair, value);
        value
    }
}

/// One glyph in laid out text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedGlyph {
    /// Which of the engine's fonts this glyph is from. See [`TextEngine::font`].
    pub font: u16,
    pub glyph: u16,
    /// The pen position, from the left of the text.
    pub x: f32,
    /// The baseline, from the top of the text.
    pub y: f32,
}

/// One line in laid out text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    /// The line's visible text, as byte positions.
    pub start: usize,
    pub end: usize,
    pub width: f32,
    /// How far the line starts from the left of the text, which isn't 0 for right-to-left text.
    pub x: f32,
    /// The baseline, from the top of the text.
    pub baseline: f32,
    /// The range of [`TextLayout::glyphs`] on this line.
    pub first_glyph: u32,
    pub glyph_count: u32,
}

/// Text arranged into lines, ready to draw.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextLayout {
    pub lines: Vec<Line>,
    pub glyphs: Vec<PlacedGlyph>,
    line_height: f32,
}

impl TextLayout {
    pub fn width(&self) -> f32 {
        self.lines
            .iter()
            .fold(0.0, |widest, line| widest.max(line.width))
    }

    pub fn height(&self) -> f32 {
        self.lines.len() as f32 * self.line_height
    }
}

/// A font asked for by name, and the number it's been given, if it could be found.
struct ChosenFont {
    family: Option<Symbol>,
    weight: FontStyle,
    italic: bool,
    face: Option<usize>,
}

/// How many characters each memo in [`TextEngine`] holds.
const MEMO_SLOTS: usize = 1024;

/// Measures and arranges text in a set of fonts.
pub struct TextEngine {
    /// The regular font first, then any fallbacks.
    pub(crate) faces: Vec<Face>,
    /// The position of the bold font, if there is one.
    bold: Option<usize>,
    /// Where recent long texts break. See [`forget_caches`](Self::forget_caches).
    long_lines: Vec<CachedLines>,
    /// Whether lines can end anywhere, as the text being worked on says. See [`TextStyle::break_all`].
    anywhere: bool,
    /// How much further along the first line of the text being worked on starts. See [`TextStyle::indent`].
    indent: f32,
    /// What floats beside the text being worked on. See [`TextStyle::beside_left`].
    beside: [(f32, f32); 2],
    /// How far apart the letters of the text being worked on are, which lines break with. See [`TextStyle::letter_spacing`].
    spacing: f32,
    /// Which way the paragraphs of the text being worked on go, as [`TextStyle::bidi_key`] says.
    pub(crate) bidi: Cell<u8>,
    /// Recent balanced texts' widths: see [`Self::balanced_width`].
    balanced: Vec<(u64, f32)>,
    next_long_line: usize,
    /// Recent text that needed shaping.
    pub(crate) shaped_cache: ShapedCache,
    /// The order to look in the regular font and fallbacks for a character, with what's wanted most
    /// first, as each font's number and how wanted it is. The order fonts were opened in isn't it, as
    /// that only says what text came first.
    pub(crate) search_order: Vec<Fallback>,
    /// The same for bold text, which looks here first, and then goes on to the others.
    bold_order: Vec<Fallback>,
    next_rank: u16,
    /// Whether any font that's open has colored glyphs, which most text then needn't look for.
    any_color: bool,
    /// Fonts that haven't been needed yet, with what's wanted most first.
    lazy: Vec<LazyFallback>,
    /// Fonts asked for by name, which there usually aren't any of, and which stay open.
    chosen_fonts: Vec<ChosenFont>,
    /// For each font that's been asked for with digits all as wide as each other, the face that has them that way, which is
    /// the same face if its font doesn't say how.
    tabular_faces: Vec<(usize, usize)>,
    /// Characters known to have no font to open, for text that gets as far as asking.
    nothing_to_open: Vec<u32>,
    /// Which font and glyph characters from beyond ASCII have, for text that has any.
    glyph_memo: OnceCell<Box<[Cell<u64>]>>,
    /// Which pages of 256 characters they're for, so that looking is one step for each character.
    lazy_pages: [u64; 8],
    /// The same for ones only bold text wants, which other text needn't stop for.
    bold_lazy_pages: [u64; 8],
    /// What arranging a paragraph with parts that look different uses, once one has.
    pub(crate) rich: Option<Box<crate::rich::RichScratch>>,
    // Changes whenever the public scratch slot is handed out, replaced or discarded.
    // Saturation disables reuse instead of ever accepting an old revision.
    pub(crate) rich_revision: u64,
    /// Which engine this is, of all those made while the program runs.
    id: u64,
}

/// A font to look in: its number, how wanted it is, and the only characters it's for, if it's been told.
#[derive(Clone, Copy)]
pub(crate) struct Fallback {
    pub(crate) face: u16,
    rank: u16,
    only: Option<&'static [RangeInclusive<u32>]>,
}

impl Fallback {
    /// Whether this font is meant to be used for this character.
    #[inline]
    pub(crate) fn is_for(&self, character: char) -> bool {
        let code = character as u32;
        self.only
            .is_none_or(|ranges| ranges.iter().any(|range| range.contains(&code)))
    }
}

/// A font that's opened once there's text that needs it.
struct LazyFallback {
    rank: u16,
    path: String,
    /// Which font in the file, for a file with several.
    face: u32,
    bold: bool,
    characters: &'static [RangeInclusive<u32>],
}

/// Which page of characters this is. Everything past the last one shares it.
#[inline]
fn page_of(character: u32) -> usize {
    (character >> 8).min(511) as usize
}

/// The characters that ask for an emoji, which have to bring in a font for one even if the font being
/// used has something to show for them.
fn asks_for_emoji(character: char) -> bool {
    matches!(character, '\u{FE0F}' | '\u{20E3}')
}

impl TextEngine {
    /// Starts with one font, which everything uses unless told otherwise.
    pub fn new(regular: Font) -> Self {
        Self {
            any_color: regular.has_color_layers(),
            faces: vec![Face::new(regular)],
            bold: None,
            long_lines: Vec::new(),
            anywhere: false,
            indent: 0.0,
            beside: [(0.0, 0.0); 2],
            spacing: 0.0,
            bidi: Cell::new(0),
            balanced: Vec::new(),
            next_long_line: 0,
            shaped_cache: ShapedCache::default(),
            search_order: vec![Fallback {
                face: 0,
                rank: 0,
                only: None,
            }],
            bold_order: Vec::new(),
            next_rank: 1,
            lazy: Vec::new(),
            chosen_fonts: Vec::new(),
            tabular_faces: Vec::new(),
            nothing_to_open: Vec::new(),
            glyph_memo: OnceCell::new(),
            lazy_pages: [0; 8],
            bold_lazy_pages: [0; 8],
            rich: None,
            rich_revision: 0,
            id: {
                static MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            },
        }
    }

    /// How much room text breaks its lines in: `max_width`, or for balanced text (see [`TextStyle::balance`]) the least room that
    /// still has it on as many lines as `max_width` does, found by halving the difference, as Blink balances lines, so that
    /// they're as near the same length as they can be. Measuring and drawing both break it there, so they agree.
    pub fn balanced_width(&mut self, text: &str, style: TextStyle, max_width: f32) -> f32 {
        if !style.balance || !max_width.is_finite() || max_width <= 0.0 || text.is_empty() {
            return max_width;
        }
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            text.hash(&mut hasher);
            max_width.to_bits().hash(&mut hasher);
            format!("{style:?}").hash(&mut hasher);
            hasher.finish()
        };
        if let Some((_, width)) = self.balanced.iter().find(|(said, _)| *said == key) {
            return *width;
        }
        let plain = TextStyle { balance: false, ..style };
        let mut layout = TextLayout::default();
        let mut lines = |engine: &mut Self, width: f32| {
            engine.layout_lines_into(text, plain, width, &mut layout);
            layout.lines.len()
        };
        let wanted = lines(self, max_width);
        let mut width = max_width;
        if (2..=MOST_BALANCED_LINES).contains(&wanted) {
            // Somewhere between the share of it each line would have and all of it.
            let (mut low, mut high) = (max_width / wanted as f32 * 0.5, max_width);
            for _ in 0..14 {
                if high - low < 0.5 {
                    break;
                }
                let middle = (low + high) / 2.0;
                if lines(self, middle) > wanted {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            width = high.ceil().min(max_width);
        }
        if self.balanced.len() >= 64 {
            self.balanced.remove(0);
        }
        self.balanced.push((key, width));
        width
    }

    /// Lets go of what's being remembered to save time, for when things have gone quiet.
    pub fn forget_caches(&mut self) {
        self.long_lines = Vec::new();
        self.next_long_line = 0;
        self.rich = None;
        self.invalidate_rich_scratch();
        self.forget_shaped();
    }

    /// How far apart lines of text are, in pixels.
    pub fn line_height(&self, style: TextStyle) -> f32 {
        self.style_metrics(self.primary(style), style).0
    }

    /// Where a caret starts down a line of this style, and how tall it is: as tall as the font's own line, in the middle of one
    /// that `line-height` makes taller, as a browser draws it, rather than the whole of that line.
    pub fn caret_span(&self, style: TextStyle) -> (f32, f32) {
        let primary = self.primary(style);
        let (natural, _) = self.line_metrics(primary, style.size);
        let (height, _) = self.style_metrics(primary, style);
        if height > natural {
            (((height - natural) / 2.0).floor(), natural)
        } else {
            (0.0, height)
        }
    }

    /// Sets the font for bold text. Without one, bold text uses the regular font.
    pub fn with_bold(mut self, bold: Font) -> Self {
        self.any_color |= bold.has_color_layers();
        self.bold = Some(self.faces.len());
        self.faces.push(Face::new(bold));
        self.glyph_memo = OnceCell::new();
        self
    }

    /// Adds a font for characters the others don't have.
    pub fn with_fallback(mut self, font: Font) -> Self {
        let rank = self.next_rank;
        self.next_rank = rank.saturating_add(1);
        self.add_fallback(font, rank, false, None);
        self
    }

    fn add_fallback(
        &mut self,
        font: Font,
        rank: u16,
        bold: bool,
        only: Option<&'static [RangeInclusive<u32>]>,
    ) {
        self.any_color |= font.has_color_layers();
        let face = self.faces.len() as u16;
        self.faces.push(Face::new(font));
        self.glyph_memo = OnceCell::new();
        let order = if bold {
            &mut self.bold_order
        } else {
            &mut self.search_order
        };
        let at = order.partition_point(|entry| entry.rank < rank);
        order.insert(at, Fallback { face, rank, only });
    }

    /// Adds a font for these characters and no others, for when the fonts before it don't have them, which
    /// is only opened once there's text that needs it. If it can't be opened, it's as if it had never been
    /// added.
    #[must_use]
    pub fn with_lazy_fallback(
        mut self,
        path: impl Into<String>,
        characters: &'static [RangeInclusive<u32>],
    ) -> Self {
        self.add_lazy(path.into(), 0, false, characters);
        self
    }

    /// Like [`with_lazy_fallback`](Self::with_lazy_fallback), for bold text only, and with which font in
    /// the file, for a file with several. Other text never opens it.
    #[must_use]
    pub fn with_lazy_bold_fallback(
        mut self,
        path: impl Into<String>,
        face: u32,
        characters: &'static [RangeInclusive<u32>],
    ) -> Self {
        self.add_lazy(path.into(), face, true, characters);
        self
    }

    fn add_lazy(
        &mut self,
        path: String,
        face: u32,
        bold: bool,
        characters: &'static [RangeInclusive<u32>],
    ) {
        let rank = self.next_rank;
        self.next_rank = rank.saturating_add(1);
        self.nothing_to_open = Vec::new();
        // A newly registered fallback can change text before a face is opened.
        self.invalidate_rich_scratch();
        self.lazy.push(LazyFallback {
            rank,
            path,
            face,
            bold,
            characters,
        });
        self.set_lazy_pages();
    }

    fn set_lazy_pages(&mut self) {
        self.lazy_pages = [0; 8];
        self.bold_lazy_pages = [0; 8];
        for lazy in &self.lazy {
            let pages = if lazy.bold {
                &mut self.bold_lazy_pages
            } else {
                &mut self.lazy_pages
            };
            for range in lazy.characters {
                for page in page_of(*range.start())..=page_of(*range.end()) {
                    pages[page / 64] |= 1 << (page % 64);
                }
            }
        }
    }

    /// Opens any fonts this text is the first to need. Faces are only ever added to the end, so the
    /// numbers of the ones already in use stay the same.
    #[inline]
    pub(crate) fn open_fallbacks_for(&mut self, text: &str, primary: usize) {
        if self.lazy.is_empty() {
            return;
        }
        let is_bold = Some(primary) == self.bold;
        for character in text.chars() {
            let page = page_of(character as u32);
            let bit = 1 << (page % 64);
            let mut wanted = self.lazy_pages[page / 64] & bit != 0;
            if is_bold {
                wanted |= self.bold_lazy_pages[page / 64] & bit != 0;
            }
            if wanted {
                // A page stays wanted while any font that's still closed covers it, so most of what
                // lands here has nothing to open. What had nothing to open never will, as fonts only
                // get opened, and open ones only get better.
                let key = (character as u32) << 1 | u32::from(is_bold);
                let slot = key as usize & (MEMO_SLOTS - 1);
                if self.nothing_to_open.get(slot) == Some(&key) {
                    continue;
                }
                let closed = self.lazy.len();
                self.open_fallback_for(character, is_bold);
                if self.lazy.len() == closed && !asks_for_emoji(character) {
                    if self.nothing_to_open.is_empty() {
                        self.nothing_to_open.resize(MEMO_SLOTS, 0);
                    }
                    self.nothing_to_open[slot] = key;
                }
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn open_fallback_for(&mut self, character: char, is_bold: bool) {
        // Bold text only needs a regular font for what no bold font has.
        if is_bold {
            self.open_from(character, true);
            let has_it = |entry: &Fallback| {
                entry.is_for(character) && self.faces[entry.face as usize].glyph(character) != 0
            };
            if self.bold_order.iter().any(has_it) && !asks_for_emoji(character) {
                return;
            }
        }
        self.open_from(character, false);
    }

    fn open_from(&mut self, character: char, bold: bool) {
        let code = character as u32;
        let asks = !bold && asks_for_emoji(character);
        loop {
            // A font that's wanted more than the best one open still has to be looked in.
            let order = if bold {
                &self.bold_order
            } else {
                &self.search_order
            };
            let best = order
                .iter()
                .find(|entry| {
                    entry.is_for(character) && self.faces[entry.face as usize].glyph(character) != 0
                })
                .map(|entry| entry.rank);
            let found = self.lazy.iter().position(|lazy| {
                lazy.bold == bold
                    && (asks || best.is_none_or(|best| lazy.rank < best))
                    && lazy.characters.iter().any(|range| range.contains(&code))
            });
            let Some(found) = found else {
                return;
            };
            let lazy = self.lazy.remove(found);
            let opened = Font::open_face(&lazy.path, lazy.face);
            self.set_lazy_pages();
            if let Ok(font) = opened {
                self.add_fallback(font, lazy.rank, bold, Some(lazy.characters));
                if asks {
                    return;
                }
            }
        }
    }

    /// Whether any font that's open has glyphs made of colored shapes.
    #[inline]
    pub fn has_color_fonts(&self) -> bool {
        self.any_color
    }

    /// How many fonts are open. Fonts are only ever opened, never closed, so text laid out by the same engine with the same
    /// number open comes out the same.
    pub fn font_count(&self) -> usize {
        self.faces.len()
    }

    /// Which engine this is, different for each one made while the program runs.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// A font by the number in [`PlacedGlyph::font`].
    pub fn font(&self, index: u16) -> Option<&Font> {
        self.faces.get(index as usize).map(|face| &*face.font)
    }

    /// The font text in this style is set in first, before any it falls back to, and its number, opening it if it's asked for
    /// by name and hasn't been.
    pub fn first_font(&mut self, style: TextStyle) -> (u16, &Font) {
        self.prepare_font(style);
        let face = self.primary(style);
        (face as u16, &*self.faces[face].font)
    }

    pub(crate) fn primary(&self, style: TextStyle) -> usize {
        // Everything that measures or places text starts here, so this is where which way it goes is taken from too.
        self.bidi.set(style.bidi_key());
        let base = self.primary_face(style);
        if !style.tabular {
            return base;
        }
        self.tabular_faces
            .iter()
            .find(|(of, _)| *of == base)
            .map_or(base, |(_, tabular)| *tabular)
    }

    /// The face text in this style is in, whatever it says of its digits.
    fn primary_face(&self, style: TextStyle) -> usize {
        let usual = if style.font.is_heavy() {
            self.bold.unwrap_or(0)
        } else {
            0
        };
        if style.family.is_none() && !style.italic {
            return usual;
        }
        self.chosen_fonts
            .iter()
            .find(|font| {
                font.family == style.family
                    && font.weight == style.font
                    && font.italic == style.italic
            })
            .and_then(|font| font.face)
            .unwrap_or(usual)
    }

    /// Opens a font that's been asked for by name, if it hasn't been. Anything measuring or placing text does this first.
    #[inline]
    pub fn prepare_font(&mut self, style: TextStyle) {
        self.anywhere = style.break_all;
        self.indent = style.indent;
        self.beside = [style.beside_left, style.beside_right];
        self.spacing = style.letter_spacing;
        if style.family.is_some()
            || style.italic
            || matches!(
                style.font,
                FontStyle::Light
                    | FontStyle::Semilight
                    | FontStyle::Medium
                    | FontStyle::Semibold
                    | FontStyle::Black
            )
        {
            self.open_chosen_font(style);
        }
        if style.tabular {
            self.open_tabular(style);
        }
    }

    /// Makes the face with digits all as wide as each other for the one this style's in, if there isn't one yet.
    #[cold]
    #[inline(never)]
    fn open_tabular(&mut self, style: TextStyle) {
        let base = self.primary_face(style);
        if self.tabular_faces.iter().any(|(of, _)| *of == base) {
            return;
        }
        let tabular = match self.faces[base].tabular_variant() {
            Some(face) => {
                self.faces.push(face);
                self.glyph_memo = OnceCell::new();
                self.faces.len() - 1
            }
            None => base,
        };
        self.tabular_faces.push((base, tabular));
    }

    #[cold]
    #[inline(never)]
    fn open_chosen_font(&mut self, style: TextStyle) {
        let weight = style.font;
        let known = self.chosen_fonts.iter().any(|font| {
            font.family == style.family && font.weight == weight && font.italic == style.italic
        });
        if known {
            return;
        }
        // As in CSS, this can be several, of which the first that's there is used, as not every version of Windows has
        // the same fonts. What one is missing, it takes from the same family without that, as text in the wrong weight is
        // better than text in the wrong font.
        let families = style.family.map_or("Segoe UI", |family| family.as_str());
        let found = families.split(',').map(css_family).find_map(|family| {
            // A family that's places along a variable font's axes, such as `Segoe UI Variable Text`, is set to the place
            // for this weight, as a browser chooses it.
            match crate::system_fonts::find_variable(family) {
                crate::system_fonts::Variable::Family(instances) => {
                    let chosen = crate::system_fonts::choose_instance(
                        &instances,
                        weight.css_weight(),
                        style.italic,
                    )?;
                    let mut font = Font::open_face(&chosen.path, chosen.face).ok()?;
                    font.set_variations(&chosen.settings);
                    return Some(font);
                }
                crate::system_fonts::Variable::NotAFamily => return None,
                crate::system_fonts::Variable::No => {}
            }
            let named = |suffix: &str| {
                let mut name = family.to_owned();
                if !suffix.is_empty() {
                    name = format!("{name} {suffix}");
                }
                if style.italic {
                    name.push_str(" Italic");
                }
                crate::system_fonts::find(&name)
            };
            let (path, face) = weight
                .names()
                .iter()
                .find_map(|suffix| named(suffix))
                .or_else(|| crate::system_fonts::find(family))?;
            Font::open_face(path, face).ok()
        });
        let face = found.map(|font| {
            self.any_color |= font.has_color_layers();
            self.faces.push(Face::new(font));
            self.glyph_memo = OnceCell::new();
            self.faces.len() - 1
        });
        self.chosen_fonts.push(ChosenFont {
            family: style.family,
            weight,
            italic: style.italic,
            face,
        });
    }

    /// How wide the widest word is, which is as narrow as text can be without breaking one.
    pub fn longest_word(&mut self, text: &str, style: TextStyle) -> f32 {
        // A word ends where a line can, as the lines here are broken: at a space, after a hyphen or a dash, and between
        // ideographs. So `abc-defghijk` is two, as it is on a page, and only as wide as its longer half has to fit.
        let mut longest = 0.0f32;
        let mut start = 0;
        let mut characters = text.char_indices().peekable();
        while let Some((at, character)) = characters.next() {
            let end = at + character.len_utf8();
            if character.is_whitespace() {
                if at > start {
                    longest = longest.max(self.line_width(&text[start..at], style));
                }
                start = end;
            } else if characters.peek().is_some_and(|&(_, next)| {
                crate::lines::can_break_between(character, next) || crate::lines::is_ideograph(next)
            })
            {
                longest = longest.max(self.line_width(&text[start..end], style));
                start = end;
            }
        }
        if start < text.len() {
            longest = longest.max(self.line_width(&text[start..], style));
        }
        longest.ceil()
    }

    /// Fills in `layout` with one line of text, however long it is. Past `limit`, what doesn't fit is cut off, with an
    /// ellipsis if `ellipsis` says so.
    pub fn layout_one_line_into(
        &mut self,
        text: &str,
        style: TextStyle,
        limit: f32,
        ellipsis: bool,
        layout: &mut TextLayout,
    ) {
        self.layout_into(text, style, f32::INFINITY, layout);
        // A line break of its own starts a new line, as on a page, unless it ends in `…`, which is only ever one line.
        if !ellipsis && layout.lines.len() > 1 {
            return;
        }
        layout.lines.truncate(1);
        let Some(line) = layout.lines.first().copied() else {
            return;
        };
        layout
            .glyphs
            .truncate((line.first_glyph + line.glyph_count) as usize);
        if line.width <= limit {
            return;
        }
        let primary = self.primary(style);
        let (dots_face, dots_glyph) = self.find_glyph(primary, '\u{2026}');
        let dots = if ellipsis {
            self.character_width(primary, style.size, None, '\u{2026}')
        } else {
            0.0
        };
        let room = (limit - dots).max(0.0);
        let baseline = line.baseline;
        let backwards = style.right_to_left || self.is_right_to_left(text, style);
        let line_right = line.x + line.width;
        if backwards {
            // Right-to-left text starts at its right, so that's the part to keep.
            let keep_from = line_right - room;
            layout.glyphs.retain(|glyph| glyph.x >= keep_from);
            let shift = -keep_from + dots;
            for glyph in &mut layout.glyphs {
                glyph.x += shift;
            }
            if ellipsis {
                layout.glyphs.insert(
                    0,
                    PlacedGlyph {
                        font: dots_face as u16,
                        glyph: dots_glyph,
                        x: 0.0,
                        y: baseline,
                    },
                );
            }
        } else {
            // A glyph stays if the next one starts within the room, or it's the last and the line ends within it.
            let ends: Vec<f32> = (0..layout.glyphs.len())
                .map(|at| layout.glyphs.get(at + 1).map_or(line_right, |next| next.x))
                .collect();
            let kept = ends.iter().take_while(|&&end| end <= line.x + room).count();
            layout.glyphs.truncate(kept);
            if ellipsis {
                let at = if kept == 0 { line.x } else { ends[kept - 1] };
                layout.glyphs.push(PlacedGlyph {
                    font: dots_face as u16,
                    glyph: dots_glyph,
                    x: at,
                    y: baseline,
                });
            }
        }
        let line = &mut layout.lines[0];
        line.width = limit;
        line.x = 0.0;
        line.glyph_count = layout.glyphs.len() as u32;
    }

    /// The `…` that ends text cut off with one, in this style, and how wide it is: for a paragraph, whose runs each have
    /// their own look, but whose `…` is in the look of the paragraph itself, as CSS has it.
    pub fn ellipsis_glyph(&mut self, style: TextStyle) -> (PlacedGlyph, f32) {
        let primary = self.primary(style);
        let (face, glyph) = self.find_glyph(primary, '\u{2026}');
        let width = self.character_width(primary, style.size, None, '\u{2026}');
        (PlacedGlyph { font: face as u16, glyph, x: 0.0, y: 0.0 }, width)
    }

    /// The face and glyph for a character: from the wanted font if it has it, otherwise
    /// the first fallback that does, and otherwise the wanted font's missing glyph.
    pub(crate) fn find_glyph(&self, primary: usize, character: char) -> (usize, u16) {
        if character.is_ascii() {
            return (primary, self.faces[primary].glyph(character));
        }
        // Looking in several fonts is most of what text from beyond the regular font costs, and the
        // same characters come round again every frame.
        let memo = self
            .glyph_memo
            .get_or_init(|| vec![Cell::new(u64::MAX); MEMO_SLOTS].into_boxed_slice());
        let key = character as u32 | (primary as u32) << 21;
        let slot = &memo[(key as usize).wrapping_mul(0x9E37_79B1) >> 7 & (MEMO_SLOTS - 1)];
        let known = slot.get();
        if (known >> 32) as u32 == key {
            return ((known >> 16 & 0xFFFF) as usize, known as u16);
        }
        let (face, glyph) = self.search_for_glyph(primary, character);
        slot.set(u64::from(key) << 32 | (face as u64) << 16 | u64::from(glyph));
        (face, glyph)
    }

    fn search_for_glyph(&self, primary: usize, character: char) -> (usize, u16) {
        let glyph = self.faces[primary].glyph(character);
        if glyph != 0 {
            return (primary, glyph);
        }
        if Some(primary) == self.bold {
            for entry in &self.bold_order {
                let glyph = self.faces[entry.face as usize].glyph(character);
                if glyph != 0 && entry.is_for(character) {
                    return (entry.face as usize, glyph);
                }
            }
        }
        for entry in &self.search_order {
            let index = entry.face as usize;
            if index != primary {
                let glyph = self.faces[index].glyph(character);
                if glyph != 0 && entry.is_for(character) {
                    return (index, glyph);
                }
            }
        }
        (primary, 0)
    }

    /// How far apart lines are, and where the first baseline is, in whole pixels so that
    /// text sits the same way on every line.
    /// How far apart lines are and how far down the first baseline is, for this style. What a line has to spare goes half above it and
    /// half below, as in CSS.
    pub(crate) fn style_metrics(&self, primary: usize, style: TextStyle) -> (f32, f32) {
        let (natural, ascent) = self.own_line_metrics(primary, style);
        match style.line_height {
            Some(height) => (height, ascent + ((height - natural) / 2.0).floor()),
            None => (natural, ascent),
        }
    }

    /// How far apart lines of `text` in this style are, and where the first one's baseline is: as its style says, or with room
    /// for every font it's in where it doesn't say.
    pub(crate) fn metrics_for(
        &mut self,
        text: &str,
        primary: usize,
        style: TextStyle,
    ) -> (f32, f32) {
        if style.line_height.is_none() && !text.is_ascii() {
            return self.text_line_metrics(text, primary, style.size);
        }
        self.style_metrics(primary, style)
    }

    /// How far apart lines of `text` are, and where the first one's baseline is, with every font its characters come from: one
    /// that's in another font, as an arrow or a tick often is, makes the lines it's on room for that font too, as a browser's do.
    pub(crate) fn text_line_metrics(
        &mut self,
        text: &str,
        primary: usize,
        size: f32,
    ) -> (f32, f32) {
        let (height, ascent) = self.line_metrics(primary, size);
        if text.is_ascii() {
            return (height, ascent);
        }
        self.open_fallbacks_for(text, primary);
        let (mut above, mut below) = (ascent, height - ascent);
        let mut seen = Vec::new();
        for character in text.chars() {
            if character.is_ascii() || character.is_whitespace() {
                continue;
            }
            let (face, _) = self.find_glyph(primary, character);
            if face != primary && !seen.contains(&face) {
                seen.push(face);
                let (other_height, other_ascent) = self.line_metrics(face, size);
                above = above.max(other_ascent);
                below = below.max(other_height - other_ascent);
            }
        }
        (above + below, above)
    }

    /// [`line_metrics`](Self::line_metrics), with what the style says of how far its font goes above and below the baseline, in
    /// whole pixels, as a browser rounds them.
    pub(crate) fn own_line_metrics(&self, primary: usize, style: TextStyle) -> (f32, f32) {
        let Some((ascent, descent)) = style.metrics else {
            return self.line_metrics(primary, style.size);
        };
        let metrics = self.faces[primary].font.metrics();
        let gap = (f32::from(metrics.line_gap) * style.size / f32::from(metrics.units_per_em.max(1))).round();
        let (ascent, descent) = ((ascent * style.size).round(), (descent * style.size).round());
        (ascent + descent + gap, ascent)
    }

    pub(crate) fn line_metrics(&self, primary: usize, size: f32) -> (f32, f32) {
        let metrics = self.faces[primary].font.metrics();
        let scale = size / f32::from(metrics.units_per_em);
        let ascent = (f32::from(metrics.ascent) * scale).round();
        let descent = (f32::from(metrics.descent) * scale).round();
        let gap = (f32::from(metrics.line_gap) * scale).round();
        (ascent - descent + gap, ascent)
    }

    /// The space text needs, breaking lines to stay within `max_width` if it can. Use
    /// infinity for no limit. This is the same as the size of [`layout`](Self::layout),
    /// without keeping the glyphs.
    pub fn measure(&mut self, text: &str, style: TextStyle, max_width: f32) -> Size {
        let max_width = self.balanced_width(text, style, max_width);
        self.prepare_font(style);
        let primary = self.primary(style);
        let (line_height, _) = self.metrics_for(text, primary, style);
        let complexity = self.complexity(text, primary);
        if complexity != Complexity::Plain && text.len() < LONG_TEXT {
            let (lines, widest) = self.measure_shaped(text, primary, style.size, max_width);
            // Lines as far apart as its shaping put them, which makes room for any other font in it.
            let line_height = if style.line_height.is_none() {
                self.text_line_metrics(text, primary, style.size).0
            } else {
                line_height
            };
            return Size::new(widest, lines as f32 * line_height);
        }
        if complexity == Complexity::Complex {
            let cached = self.long_text_lines(text, primary, style.size, max_width);
            let cached = &self.long_lines[cached];
            let widest = cached.shaped.as_ref().map_or(0.0, |shaped| shaped.widest);
            return Size::new(widest, cached.lines.len() as f32 * line_height);
        }
        let mut lines = 0u32;
        let mut widest = 0f32;
        self.each_line(
            text,
            primary,
            style.size,
            line_height,
            max_width,
            |_, _, _, width| {
                lines += 1;
                widest = widest.max(width);
            },
        );
        Size::new(widest, lines as f32 * line_height)
    }

    /// Like [`measure`](Self::measure), with each line as wide as its letters spaced out by [`TextStyle::letter_spacing`] make it,
    /// after each of them as a browser has it, and lines broken where that makes them too long, as [`layout_into`](Self::layout_into)
    /// breaks them. `None` for text that needs shaping, whose lines break as if its letters weren't spaced out.
    pub fn measure_spaced(&mut self, text: &str, style: TextStyle, max_width: f32) -> Option<Size> {
        let max_width = self.balanced_width(text, style, max_width);
        self.prepare_font(style);
        let primary = self.primary(style);
        if text.len() >= REMEMBERED_LINES_FROM || self.complexity(text, primary) != Complexity::Plain {
            return None;
        }
        let (line_height, _) = self.metrics_for(text, primary, style);
        let spacing = self.spacing;
        let mut lines = 0u32;
        let mut widest = 0f32;
        self.each_line(
            text,
            primary,
            style.size,
            line_height,
            max_width,
            |_, start, end, width| {
                lines += 1;
                widest = widest.max(width + spacing * spaced_characters(&text[start..end]) as f32);
            },
        );
        Some(Size::new(widest, lines as f32 * line_height))
    }

    /// How far apart letters are when lines break, for the text being worked on: as they're drawn, for all but long text, which
    /// is remembered whatever its letters' spacing and breaks as if they weren't spaced out.
    fn short_spacing(&self, text: &str) -> f32 {
        if text.len() < REMEMBERED_LINES_FROM { self.spacing } else { 0.0 }
    }

    /// Arranges text into lines, breaking them to stay within `max_width` if it can. Use
    /// infinity for no limit.
    pub fn layout(&mut self, text: &str, style: TextStyle, max_width: f32) -> TextLayout {
        let mut layout = TextLayout::default();
        self.layout_into(text, style, max_width, &mut layout);
        layout
    }

    /// Like [`layout`](Self::layout), filling in `layout`, whose memory is reused.
    pub fn layout_into(
        &mut self,
        text: &str,
        style: TextStyle,
        max_width: f32,
        layout: &mut TextLayout,
    ) {
        self.layout_lines_into(text, style, max_width, layout);
        // Its box starts where its fonts' text does, so what's drawn in it is as much higher in it.
        if let Some((top, _)) = self.inline_area(style) {
            for glyph in &mut layout.glyphs {
                glyph.y -= top;
            }
            for line in &mut layout.lines {
                line.baseline -= top;
            }
        }
    }

    /// For text whose box is only as tall as its fonts make it (see [`TextStyle::inline_area`]): how far below the top of its
    /// first line that box starts, and how much shorter than its lines it is, which is less than nothing for lines closer
    /// together than its fonts are tall, as `line-height: 1` makes them. `None` for any other text.
    pub fn inline_area(&mut self, style: TextStyle) -> Option<(f32, f32)> {
        if !style.inline_area || style.line_height.is_none() {
            return None;
        }
        self.prepare_font(style);
        let primary = self.primary(style);
        let (line, baseline) = self.style_metrics(primary, style);
        let (natural, ascent) = self.own_line_metrics(primary, style);
        let (top, cut) = (baseline - ascent, line - natural);
        (top != 0.0 || cut != 0.0).then_some((top, cut))
    }

    /// How tall a lowercase `x` is in this style, as its font says, or half its size where it doesn't, as CSS has it.
    pub fn x_height(&mut self, style: TextStyle) -> f32 {
        self.prepare_font(style);
        let metrics = self.faces[self.primary(style)].font.metrics();
        if metrics.x_height > 0 && metrics.units_per_em > 0 {
            f32::from(metrics.x_height) * style.size / f32::from(metrics.units_per_em)
        } else {
            style.size / 2.0
        }
    }

    pub(crate) fn layout_lines_into(
        &mut self,
        text: &str,
        style: TextStyle,
        max_width: f32,
        layout: &mut TextLayout,
    ) {
        let max_width = self.balanced_width(text, style, max_width);
        self.prepare_font(style);
        let primary = self.primary(style);
        let (line_height, ascent) = self.metrics_for(text, primary, style);
        layout.lines.clear();
        layout.glyphs.clear();
        layout.line_height = line_height;
        if self.complexity(text, primary) != Complexity::Plain {
            self.layout_shaped_into(text, primary, style.size, max_width, layout);
            return;
        }
        // Nearly all text fits on one line, which only takes one pass.
        let (first_indent, beside) = (self.indent, self.beside);
        let plain = first_indent == 0.0 && beside[0].1 <= 0.0 && beside[1].1 <= 0.0;
        if plain && self.place_single_line(text, primary, style.size, max_width, ascent, layout) {
            return;
        }
        layout.lines.clear();
        layout.glyphs.clear();
        let anywhere = self.anywhere;
        let spacing = self.short_spacing(text);
        let mut start = 0;
        let mut index = 0;
        loop {
            let (indent, narrower) = line_room(index, line_height, first_indent, beside);
            let mut line = next_line(
                text,
                start,
                max_width - narrower,
                anywhere,
                &mut |previous, character| {
                    self.character_width(primary, style.size, previous, character) + spacing
                },
            );
            // Lines break with letters as far apart as they're drawn, but are as wide as they'd be without, as
            // `space_letters` spreads them out after.
            line.width -= spacing * spaced_characters(&text[start..line.end]) as f32;
            let baseline = ascent + index as f32 * line_height;
            let first_glyph = layout.glyphs.len() as u32;
            self.place_line(
                &text[start..line.end],
                primary,
                style.size,
                baseline,
                &mut layout.glyphs,
            );
            if indent != 0.0 {
                // What's before it on the line is the room it's indented by, which is part of how wide the line is.
                for glyph in &mut layout.glyphs[first_glyph as usize..] {
                    glyph.x += indent;
                }
            }
            layout.lines.push(Line {
                start,
                end: line.end,
                width: line.width + indent,
                x: 0.0,
                baseline,
                first_glyph,
                glyph_count: layout.glyphs.len() as u32 - first_glyph,
            });
            if line.last {
                return;
            }
            start = line.next;
            index += 1;
        }
    }

    /// Lays out `text` as one line if it fits on one, and says whether it did. What comes out is
    /// the same as finding the lines first, including that spaces at the end aren't part of it.
    fn place_single_line(
        &mut self,
        text: &str,
        primary: usize,
        size: f32,
        max_width: f32,
        baseline: f32,
        layout: &mut TextLayout,
    ) -> bool {
        let limit = max_width + crate::lines::WIDTH_TOLERANCE;
        let spacing = self.short_spacing(text);
        let (mut pen, mut width, mut visible_width) = (0.0f32, 0.0f32, 0.0f32);
        let (mut visible_end, mut visible_glyphs) = (0, 0);
        let mut previous = None;
        for (byte, character) in text.char_indices() {
            if character == '\n' {
                return false;
            }
            if character == '\r' {
                continue;
            }
            let (face, glyph) = self.find_glyph(primary, character);
            let upem = f32::from(self.faces[face].font.metrics().units_per_em);
            let scale = size / upem;
            let mut units = self.faces[face].advance_units(character, glyph);
            let mut x = pen;
            if let Some((previous_face, previous_glyph)) = previous {
                if previous_face == face {
                    let kerning = self.faces[face].kerning(previous_glyph, glyph);
                    x += kerning * scale;
                    units += kerning;
                }
            }
            let new_width = width + units * size / upem;
            // Letters as far apart as they'll be drawn, after each of them.
            if new_width + spacing * (layout.glyphs.len() + 1) as f32 > limit && !is_space(character) {
                return false;
            }
            layout.glyphs.push(PlacedGlyph {
                font: face as u16,
                glyph,
                x,
                y: baseline,
            });
            pen = x + self.faces[face].advance_units(character, glyph) * scale;
            width = new_width;
            previous = Some((face, glyph));
            if !is_space(character) {
                visible_width = width;
                visible_end = byte + character.len_utf8();
                visible_glyphs = layout.glyphs.len();
            }
        }
        layout.glyphs.truncate(visible_glyphs);
        layout.lines.push(Line {
            start: 0,
            end: visible_end,
            width: visible_width,
            x: 0.0,
            baseline,
            first_glyph: 0,
            glyph_count: visible_glyphs as u32,
        });
        true
    }

    /// Like [`layout_into`](Self::layout_into), for showing text between these two heights. Long
    /// text only places the lines between them, and remembers where its lines break, so a page
    /// of it costs the same each frame as a screenful. Lines keep their numbering from the top.
    pub fn layout_visible_into(
        &mut self,
        text: &str,
        style: TextStyle,
        max_width: f32,
        visible: (f32, f32),
        layout: &mut TextLayout,
    ) {
        self.prepare_font(style);
        let primary = self.primary(style);
        let shorter_than_long = text.len() < LONG_TEXT;
        if text.len() < REMEMBERED_LINES_FROM
            || (shorter_than_long && self.complexity(text, primary) != Complexity::Plain)
        {
            self.layout_into(text, style, max_width, layout);
            return;
        }
        let (line_height, ascent) = self.style_metrics(primary, style);
        layout.lines.clear();
        layout.glyphs.clear();
        layout.line_height = line_height;
        let cached = self.long_text_lines(text, primary, style.size, max_width);
        let complexity = self.long_lines[cached].complexity;
        // Taken out for now, so that placing lines is free to use the engine.
        let shaped = self.long_lines[cached].shaped.take();
        let mut extended_cache = false;
        let count = self.long_lines[cached].lines.len();
        let first = ((visible.0 / line_height).floor().max(0.0) as usize).min(count);
        let last = ((visible.1 / line_height).ceil().max(0.0) as usize).min(count);
        for index in first..last {
            let (start, end, width) = self.long_lines[cached].lines[index];
            let (start, end) = (start as usize, end as usize);
            let baseline = ascent + index as f32 * line_height;
            let first_glyph = layout.glyphs.len() as u32;
            let mut x = 0.0;
            let slot = index % SHAPED_LINES;
            let remembered = self.long_lines[cached]
                .shaped_lines
                .get(slot)
                .and_then(Option::as_ref)
                .filter(|line| line.line == index as u32);
            if let Some(line) = remembered {
                layout
                    .glyphs
                    .extend(line.glyphs.iter().map(|glyph| PlacedGlyph {
                        y: glyph.y + baseline,
                        ..*glyph
                    }));
                layout.lines.push(Line {
                    start,
                    end,
                    width,
                    x: line.x,
                    baseline,
                    first_glyph,
                    glyph_count: layout.glyphs.len() as u32 - first_glyph,
                });
                continue;
            }
            match (&shaped, complexity) {
                (Some(shaped), _) => {
                    x = self.place_long_shaped_line(
                        text,
                        shaped,
                        index,
                        primary,
                        style.size,
                        max_width,
                        baseline,
                        &mut layout.glyphs,
                    );
                }
                (None, Complexity::Plain) => self.place_line(
                    &text[start..end],
                    primary,
                    style.size,
                    baseline,
                    &mut layout.glyphs,
                ),
                (None, _) => {
                    let line = &text[start..end];
                    if self.complexity(line, primary) == Complexity::Plain {
                        self.place_line(line, primary, style.size, baseline, &mut layout.glyphs);
                    } else {
                        self.place_shaped_line(
                            line,
                            start,
                            primary,
                            style.size,
                            baseline,
                            &mut layout.glyphs,
                        );
                    }
                }
            }
            layout.lines.push(Line {
                start,
                end,
                width,
                x,
                baseline,
                first_glyph,
                glyph_count: layout.glyphs.len() as u32 - first_glyph,
            });
            if complexity != Complexity::Plain {
                extended_cache = true;
                let entry = &mut self.long_lines[cached];
                if entry.shaped_lines.is_empty() {
                    entry.shaped_lines.resize_with(SHAPED_LINES, || None);
                }
                entry.shaped_lines[slot] = Some(ShapedLine {
                    line: index as u32,
                    x,
                    glyphs: layout.glyphs[first_glyph as usize..]
                        .iter()
                        .map(|glyph| PlacedGlyph {
                            y: glyph.y - baseline,
                            ..*glyph
                        })
                        .collect(),
                });
            }
        }
        self.long_lines[cached].shaped = shaped;
        if extended_cache {
            self.long_lines[cached].update_retained_bytes();
            trim_line_cache(&mut self.long_lines, cached);
        }
    }

    /// Finds where a long text's lines break, from memory if possible.
    fn long_text_lines(&mut self, text: &str, primary: usize, size: f32, max_width: f32) -> usize {
        let hash = FixedState::default().hash_one(text);
        let (size_bits, width_bits) = (size.to_bits(), max_width.to_bits());
        let found = self.long_lines.iter().position(|cached| {
            cached.text == hash
                && cached.length == text.len()
                && cached.size == size_bits
                && cached.primary == primary
                && cached.width == width_bits
                && cached.anywhere == self.anywhere
                && cached.bidi == self.bidi.get()
        });
        if let Some(found) = found {
            return found;
        }
        let complexity = self.complexity(text, primary);
        let mut lines = Vec::new();
        let mut shaped = None;
        if complexity == Complexity::Complex {
            let long = self.long_shaped_lines(text, primary, size, max_width);
            lines.clone_from(&long.lines);
            shaped = Some(long);
        } else {
            let line_height = self.line_metrics(primary, size).0;
            self.each_line(
                text,
                primary,
                size,
                line_height,
                max_width,
                |_, start, end, width| {
                    lines.push((start as u32, end as u32, width));
                },
            );
        }
        lines.shrink_to_fit();
        let mut entry = CachedLines {
            complexity,
            shaped,
            shaped_lines: Vec::new(),
            text: hash,
            length: text.len(),
            size: size_bits,
            primary,
            width: width_bits,
            anywhere: self.anywhere,
            bidi: self.bidi.get(),
            lines,
            retained_bytes: 0,
        };
        entry.update_retained_bytes();

        let slot = if self.long_lines.len() < LINE_CACHE_SIZE {
            self.long_lines.push(entry);
            self.long_lines.len() - 1
        } else {
            let slot = self.next_long_line;
            self.next_long_line = (slot + 1) % LINE_CACHE_SIZE;
            self.long_lines[slot] = entry;
            slot
        };
        trim_line_cache(&mut self.long_lines, slot)
    }

    /// Lays out one line of text, placing only what falls between these two distances from its
    /// left. This goes straight through without looking for line breaks, and stops once it's
    /// past the end, so it stays quick however much text follows.
    pub fn layout_across_into(
        &mut self,
        text: &str,
        style: TextStyle,
        across: (f32, f32),
        layout: &mut TextLayout,
    ) {
        let primary = self.primary(style);
        let (line_height, ascent) = self.style_metrics(primary, style);
        layout.lines.clear();
        layout.glyphs.clear();
        layout.line_height = line_height;
        let reach = style.size * 2.0;
        let (from, to) = (across.0 - reach, across.1 + reach);
        if self.complexity(text, primary) != Complexity::Plain {
            self.layout_shaped_across(text, primary, style.size, (from, to), layout);
            return;
        }
        let mut pen = 0.0f32;
        let mut previous = None;
        for character in text.chars() {
            if character == '\r' {
                continue;
            }
            let (face, glyph) = self.find_glyph(primary, character);
            let scale = style.size / f32::from(self.faces[face].font.metrics().units_per_em);
            let mut x = pen;
            if let Some((previous_face, previous_glyph)) = previous {
                if previous_face == face {
                    x += self.faces[face].kerning(previous_glyph, glyph) * scale;
                }
            }
            if x > to {
                break;
            }
            if x >= from {
                layout.glyphs.push(PlacedGlyph {
                    font: face as u16,
                    glyph,
                    x,
                    y: ascent,
                });
            }
            pen = x + self.faces[face].advance_units(character, glyph) * scale;
            previous = Some((face, glyph));
        }
    }

    /// Fills in where a caret at each of these bytes would go, in one pass over the text. They
    /// must be in order, and on character boundaries.
    pub fn caret_positions(&mut self, text: &str, style: TextStyle, carets: &mut [(usize, f32)]) {
        let primary = self.primary(style);
        if self.complexity(text, primary) != Complexity::Plain {
            for caret in carets.iter_mut() {
                caret.1 = self.shaped_caret_x(text, primary, style.size, caret.0);
            }
            return;
        }
        let mut next = 0;
        let count = carets.len();
        self.each_caret(text, style, |byte, x| {
            while next < count && carets[next].0 <= byte {
                carets[next].1 = x;
                next += 1;
            }
            next < count
        });
    }

    /// How far from the left of one line of text a caret at byte `at` goes. This is where
    /// [`layout`](Self::layout) puts the glyph that starts there, including its kerning.
    /// `at` must be on a character boundary.
    pub fn caret_x(&mut self, text: &str, style: TextStyle, at: usize) -> f32 {
        let primary = self.primary(style);
        if self.complexity(text, primary) != Complexity::Plain {
            return self.shaped_caret_x(text, primary, style.size, at);
        }
        let mut found = 0.0;
        self.each_caret(text, style, |byte, x| {
            if byte > at {
                return false;
            }
            found = x;
            true
        });
        found
    }

    /// The character boundary in one line of text nearest to `x`, measured from its left.
    pub fn byte_at_x(&mut self, text: &str, style: TextStyle, x: f32) -> usize {
        let primary = self.primary(style);
        if self.complexity(text, primary) != Complexity::Plain {
            let stops = self.shaped_stops(text, primary, style.size);
            let nearest = stops
                .iter()
                .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()));
            return nearest.map_or(0, |stop| stop.0);
        }
        let mut nearest = (0, f32::INFINITY);
        self.each_caret(text, style, |byte, caret| {
            let distance = (caret - x).abs();
            if distance < nearest.1 {
                nearest = (byte, distance);
            }
            caret <= x
        });
        nearest.0
    }

    /// Where the caret goes when it's moved one place left or right on screen, for text where that
    /// isn't simply the next character. `None` means the order on screen is the order of the text.
    pub fn caret_across(
        &mut self,
        text: &str,
        style: TextStyle,
        at: usize,
        rightward: bool,
    ) -> Option<usize> {
        let primary = self.primary(style);
        if self.complexity(text, primary) != Complexity::Complex {
            return None;
        }
        let stops = self.shaped_stops(text, primary, style.size);
        let here = stops.iter().position(|stop| stop.0 == at).or_else(|| {
            let x = self.shaped_caret_x(text, primary, style.size, at);
            stops
                .iter()
                .enumerate()
                .min_by(|a, b| (a.1.1 - x).abs().total_cmp(&(b.1.1 - x).abs()))
                .map(|(index, _)| index)
        })?;
        let next = if rightward {
            stops.get(here + 1)
        } else {
            here.checked_sub(1).and_then(|before| stops.get(before))
        };
        Some(next.map_or(at, |stop| stop.0))
    }

    /// Adds what a selection between these two bytes covers on one line, from left to right, to
    /// `spans`. Text in mixed directions can make that several separate pieces.
    pub fn selection_spans(
        &mut self,
        text: &str,
        style: TextStyle,
        selected: (usize, usize),
        spans: &mut Vec<(f32, f32)>,
    ) {
        let primary = self.primary(style);
        if self.complexity(text, primary) != Complexity::Plain {
            self.shaped_spans(text, primary, style.size, selected, spans);
            return;
        }
        let mut carets = [(selected.0, 0.0), (selected.1, 0.0)];
        self.caret_positions(text, style, &mut carets);
        spans.push((carets[0].1, carets[1].1));
    }

    /// Where each line of wrapped text starts, as byte positions. Use [`wrapped_line`] to get each
    /// one's text, for working out where carets go along it.
    pub fn wrapped_starts(&mut self, text: &str, style: TextStyle, max_width: f32) -> Vec<usize> {
        if text.len() < LONG_TEXT {
            let layout = self.layout(text, style, max_width);
            return layout.lines.iter().map(|line| line.start).collect();
        }
        let primary = self.primary(style);
        let cached = self.long_text_lines(text, primary, style.size, max_width);
        self.long_lines[cached]
            .lines
            .iter()
            .map(|line| line.0 as usize)
            .collect()
    }

    /// How wide one line of text is, including any spaces at the end of it.
    pub fn line_width(&mut self, text: &str, style: TextStyle) -> f32 {
        let primary = self.primary(style);
        if self.complexity(text, primary) != Complexity::Plain {
            return self.shaped_direction(text, primary, style.size).1;
        }
        self.caret_x(text, style, text.len())
    }

    /// Whether the end of one line of text might not be at its right, or in other words, if a
    /// caret at its end says how wide it is. If not, use [`line_width`](Self::line_width).
    pub fn may_reorder(&mut self, text: &str, style: TextStyle) -> bool {
        let primary = self.primary(style);
        self.may_reorder_at(text, primary)
    }

    fn may_reorder_at(&mut self, text: &str, primary: usize) -> bool {
        self.complexity(text, primary) == Complexity::Complex
    }

    /// Whether one line of text starts from the right.
    pub fn is_right_to_left(&mut self, text: &str, style: TextStyle) -> bool {
        let primary = self.primary(style);
        self.may_reorder_at(text, primary) && self.shaped_direction(text, primary, style.size).0
    }

    /// [`line_indent`](Self::line_indent) for a field's one line, which starts as much further along as its style's `indent`
    /// says, as CSS's `text-indent` has an input's text, as Radix's text field's is.
    pub fn field_indent(&mut self, text: &str, style: TextStyle, width: f32) -> f32 {
        if style.indent == 0.0 {
            return self.line_indent(text, style, width);
        }
        self.line_indent(text, style, (width - style.indent).max(0.0)) + style.indent
    }

    /// How far across a space this wide one line of text starts, which is how right-to-left text
    /// ends up on the right of a text input. Past that it can only start at the left.
    pub fn line_indent(&mut self, text: &str, style: TextStyle, width: f32) -> f32 {
        let primary = self.primary(style);
        if !self.may_reorder_at(text, primary) {
            return 0.0;
        }
        let (right_to_left, line) = self.shaped_direction(text, primary, style.size);
        if right_to_left {
            (width - line).max(0.0)
        } else {
            0.0
        }
    }

    /// Calls `visit` with each place a caret can go on one line, from left to right, until
    /// it returns false.
    fn each_caret(
        &mut self,
        text: &str,
        style: TextStyle,
        mut visit: impl FnMut(usize, f32) -> bool,
    ) {
        let primary = self.primary(style);
        let mut pen = 0.0;
        let mut previous = None;
        for (byte, character) in text.char_indices() {
            let (face, glyph) = self.find_glyph(primary, character);
            let scale = style.size / f32::from(self.faces[face].font.metrics().units_per_em);
            let mut x = pen;
            if let Some((previous_face, previous_glyph)) = previous {
                if previous_face == face {
                    x += self.faces[face].kerning(previous_glyph, glyph) * scale;
                }
            }
            if !visit(byte, x) {
                return;
            }
            pen = x + self.faces[face].advance_units(character, glyph) * scale;
            previous = Some((face, glyph));
        }
        visit(text.len(), pen);
    }

    /// Finds each line, and calls `visit` with its number, byte range and width.
    fn each_line(
        &mut self,
        text: &str,
        primary: usize,
        size: f32,
        line_height: f32,
        max_width: f32,
        mut visit: impl FnMut(u32, usize, usize, f32),
    ) {
        let anywhere = self.anywhere;
        let (first_indent, beside) = (self.indent, self.beside);
        let spacing = self.short_spacing(text);
        let mut start = 0;
        let mut index = 0;
        loop {
            let (indent, narrower) = line_room(index, line_height, first_indent, beside);
            let line = next_line(
                text,
                start,
                max_width - narrower,
                anywhere,
                &mut |previous, character| self.character_width(primary, size, previous, character) + spacing,
            );
            // As wide as it'd be without its letters spaced out, as `space_letters` spreads them out after.
            let width = line.width - spacing * spaced_characters(&text[start..line.end]) as f32;
            visit(index, start, line.end, width + indent);
            index += 1;
            if line.last {
                return;
            }
            start = line.next;
        }
    }

    /// How far a character moves the pen, including its kerning with the one before it.
    pub(crate) fn character_width(
        &mut self,
        primary: usize,
        size: f32,
        previous: Option<char>,
        character: char,
    ) -> f32 {
        let (face, glyph) = self.find_glyph(primary, character);
        let mut units = self.faces[face].advance_units(character, glyph);
        if let Some(previous) = previous {
            let (previous_face, previous_glyph) = self.find_glyph(primary, previous);
            if previous_face == face {
                units += self.faces[face].kerning(previous_glyph, glyph);
            }
        }
        units * size / f32::from(self.faces[face].font.metrics().units_per_em)
    }

    fn place_line(
        &mut self,
        line: &str,
        primary: usize,
        size: f32,
        baseline: f32,
        glyphs: &mut Vec<PlacedGlyph>,
    ) {
        let mut pen = 0.0;
        let mut previous = None;
        for character in line.chars() {
            if character == '\r' {
                continue;
            }
            let (face, glyph) = self.find_glyph(primary, character);
            let scale = size / f32::from(self.faces[face].font.metrics().units_per_em);
            let mut x = pen;
            if let Some((previous_face, previous_glyph)) = previous {
                if previous_face == face {
                    x += self.faces[face].kerning(previous_glyph, glyph) * scale;
                }
            }
            glyphs.push(PlacedGlyph {
                font: face as u16,
                glyph,
                x,
                y: baseline,
            });
            pen = x + self.faces[face].advance_units(character, glyph) * scale;
            previous = Some((face, glyph));
        }
    }
}

/// The text on one line, among those starting at `starts`, including spaces at its end, as a caret can
/// go past those, but not what ends it.
pub fn wrapped_line<'a>(text: &'a str, starts: &[usize], line: usize) -> &'a str {
    let start = starts.get(line).copied().unwrap_or(text.len());
    let end = starts.get(line + 1).copied().unwrap_or(text.len());
    let piece = &text[start..end];
    let piece = piece.strip_suffix('\n').unwrap_or(piece);
    piece.strip_suffix('\r').unwrap_or(piece)
}

/// A family as CSS writes it, without quotes, and with the ones CSS has for any font of a kind as the ones a browser on Windows uses.
/// How many characters the longest of the lines `text` breaks into by itself has.
/// How much further along a line starts, and how much less room it has, for an indented first line and what floats beside lines
/// that start above where it ends.
fn line_room(index: u32, line_height: f32, indent: f32, beside: [(f32, f32); 2]) -> (f32, f32) {
    let top = index as f32 * line_height;
    let indent = if index == 0 { indent } else { 0.0 };
    let left = if top < beside[0].1 { beside[0].0 } else { 0.0 };
    let right = if top < beside[1].1 { beside[1].0 } else { 0.0 };
    (indent + left, indent + left + right)
}

/// How many characters of a line `letter-spacing` goes after, which is as many glyphs as plain text is placed with.
fn spaced_characters(line: &str) -> usize {
    line.chars().filter(|character| *character != '\r' && *character != '\n').count()
}

fn widest_line_characters(text: &str) -> usize {
    text.split('\n')
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
}

/// How wide a browser has text that's this wide: up to the next 64th of a pixel, which is as finely as it places anything. Text next
/// to it is placed that far along, which whole pixels would put up to a pixel further.
fn page_width(width: f32) -> f32 {
    (width * 64.0).ceil() / 64.0
}

fn css_family(family: &str) -> &str {
    let family = family.trim().trim_matches(['"', '\'']);
    match family.to_ascii_lowercase().as_str() {
        "serif" | "ui-serif" => "Times New Roman",
        "sans-serif" => "Arial",
        "monospace" | "ui-monospace" => "Consolas",
        "cursive" => "Comic Sans MS",
        "fantasy" => "Impact",
        "system-ui" | "ui-sans-serif" | "-apple-system" | "blinkmacsystemfont" => "Segoe UI",
        _ => family,
    }
}

impl Measure for TextEngine {
    fn first_baseline(&mut self, element: &Element) -> Option<f32> {
        let text = element.role == Role::TextInput
            || (!element.children.is_empty() && is_paragraph(element))
            || (shows_name(element) && !element.name.is_empty());
        // A field with more than one line sits on a line by its bottom edge, as a browser's does.
        let lines = matches!(element.props.get("rows"), Some(PropValue::Int(rows)) if *rows > 1);
        if !text || (element.role == Role::TextInput && lines) {
            return None;
        }
        let style = TextStyle::from_element(element);
        self.prepare_font(style);
        let primary = self.primary(style);
        let (_, ascent) = self.metrics_for(&element.name, primary, style);
        Some(ascent)
    }

    fn inline_area(&mut self, element: &Element) -> Option<(f32, f32)> {
        TextEngine::inline_area(self, TextStyle::from_element(element))
    }

    fn x_height(&mut self, element: &Element) -> Option<f32> {
        Some(TextEngine::x_height(self, TextStyle::from_element(element)))
    }

    fn line_levels(&mut self, line: &Element, levels: &mut Vec<u8>) {
        crate::rich::line_levels(line, levels);
    }


    fn strut(&mut self, element: &Element) -> Option<(f32, f32)> {
        let style = TextStyle::from_element(element);
        self.prepare_font(style);
        let (height, ascent) = self.style_metrics(self.primary(style), style);
        Some((ascent, height - ascent))
    }

    fn line_below(&mut self, element: &Element) -> f32 {
        let style = TextStyle::from_element(element);
        self.prepare_font(style);
        let (height, ascent) = self.style_metrics(self.primary(style), style);
        (height - ascent).max(0.0)
    }

    fn measure(&mut self, element: &Element, max_width: f32) -> Size {
        if !element.children.is_empty() && is_paragraph(element) {
            return self.measure_paragraph(element, max_width);
        }
        let style = TextStyle::from_element(element);
        if element.role == Role::TextInput {
            // Its font, which nothing may have loaded yet, as there's no text in it.
            self.prepare_font(style);
            let (line_height, _) = self.style_metrics(self.primary(style), style);
            let rows = match element.props.get("rows") {
                Some(PropValue::Int(rows)) => (*rows).max(1) as f32,
                _ => 1.0,
            };
            // With `field-sizing: content`, one with several lines is as tall as its text's lines at its width, or its
            // placeholder's when it's empty, at least one, as a browser has it, rather than `rows`.
            let fits = matches!(element.props.get("field_sizing"), Some(PropValue::Str(how)) if how.as_str() == "content");
            if fits && element.props.get("rows").is_some() {
                let said = |name: &str| match element.props.get(name) {
                    Some(PropValue::Str(text)) => text.as_str(),
                    _ => "",
                };
                let value = said("value");
                let text = if value.is_empty() { said("placeholder") } else { value };
                let lines = if text.is_empty() {
                    1
                } else {
                    self.wrapped_starts(text, style, max_width).len().max(1)
                };
                return Size::new(
                    rsui_layout::input_width(element).min(max_width),
                    line_height * lines as f32,
                );
            }
            return Size::new(
                rsui_layout::input_width(element).min(max_width),
                line_height * rows,
            );
        }
        if !shows_name(element) || element.name.is_empty() {
            return Size::new(0.0, 0.0);
        }
        // Text that's kept to one line is as wide as it is, up to what there's room for, as what's left over is cut off.
        let one_line = !element.props.is_empty()
            && (element.props.get_symbol(ONE_LINE_NAMES.0).is_some()
                || element.props.get_symbol(ONE_LINE_NAMES.1).is_some());
        if one_line {
            let mut size = TextEngine::measure(self, &element.name, style, f32::INFINITY);
            if element.props.get("keep_spaces") == Some(&PropValue::Bool(true)) {
                // Spaces it ends with are kept, as CSS's `white-space: pre` has them, and take room: where the caret would be after
                // the last of them is how wide each line is.
                for line in element.name.split('\n') {
                    if line.ends_with(char::is_whitespace) {
                        size.width = size.width.max(self.line_width(line, style));
                    }
                }
            }
            size.width += style.letter_spacing * widest_line_characters(&element.name) as f32;
            size.width += style.word_spacing * widest_line_spaces(&element.name) as f32;
            let (line_height, _) = self.style_metrics(self.primary(style), style);
            // Its own line breaks still start new lines, unless it ends in `…`.
            let line_height = if element.props.get_symbol(ONE_LINE_NAMES.0).is_none() {
                size.height.max(line_height)
            } else {
                line_height
            };
            // A page's line that doesn't wrap and doesn't end in `…` is as long as it is, whatever room there is, whether or not
            // its words can be broken, which a line that doesn't wrap doesn't do. A page's text says which they can.
            let keeps_all = element.props.get_symbol(ONE_LINE_NAMES.2).is_some()
                && element.props.get_symbol(ONE_LINE_NAMES.0).is_none();
            let width = if keeps_all {
                page_width(size.width)
            } else if element.props.get_symbol(ONE_LINE_NAMES.2).is_some() {
                // A page's text that ends in `…` is as wide as a browser has it too, where there's room: only rsui's own text is
                // whole pixels, which made a truncated `<span>` a fraction of a pixel wider than on a page.
                page_width(size.width).min(max_width)
            } else {
                size.width.ceil().min(max_width)
            };
            return Size::new(width, line_height);
        }
        // With `keep_words`, a word isn't broken to fit, as on a page: the text is as wide as its longest word, at least.
        let max_width =
            if element.props.get_symbol(ONE_LINE_NAMES.2) == Some(&PropValue::Bool(true)) {
                max_width.max(self.longest_word(&element.name, style))
            } else {
                max_width
            };
        // Spaced-out letters break lines as they're drawn, and each line is as wide as they make it.
        let spaced = if style.letter_spacing != 0.0 {
            self.measure_spaced(&element.name, style, max_width)
        } else {
            None
        };
        let mut size = match spaced {
            Some(size) => size,
            None => TextEngine::measure(self, &element.name, style, max_width),
        };
        // Only so many lines, as CSS's `line-clamp` says.
        if let Some(lines) = max_lines(element) {
            let (line_height, _) = self.style_metrics(self.primary(style), style);
            size.height = size.height.min(line_height * lines as f32);
        }
        if style.letter_spacing != 0.0 && spaced.is_none() {
            // As far as each line goes, which is near enough for text that wraps, as it's measured as if letters weren't apart.
            let lines = (size.height / self.style_metrics(self.primary(style), style).0)
                .round()
                .max(1.0);
            let characters = element.name.chars().count() as f32 / lines;
            size.width += style.letter_spacing * characters.ceil();
        }
        if style.word_spacing > 0.0 {
            // The same for spaces, near enough: as wide as the most spaces a line has, shared out among the lines.
            let lines = (size.height / self.style_metrics(self.primary(style), style).0)
                .round()
                .max(1.0);
            let spaces = element.name.chars().filter(|character| spaces_words(*character)).count() as f32;
            size.width += style.word_spacing * (spaces / lines).ceil().min(widest_line_spaces(&element.name) as f32);
        }
        // Whole pixels, so a width measured here can't round down and wrap the text when
        // it comes back as a limit. A page's text is as wide as a browser has it, which is near enough not to.
        if element.props.get_symbol(ONE_LINE_NAMES.2).is_some() {
            return Size::new(page_width(size.width), size.height);
        }
        Size::new(size.width.ceil(), size.height)
    }

    fn place_pieces(&mut self, element: &Element, width: f32, places: &mut Vec<Rect>) {
        self.places_of_runs(element, width, places);
    }
}
