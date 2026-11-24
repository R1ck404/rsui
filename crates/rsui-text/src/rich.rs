//! A paragraph with parts that look different, wrapped as one piece of text.
//!
//! The text is cut wherever a line could end, or the look or direction changes, and each part is
//! placed by what places all other text. That costs a little more than one look all the way along,
//! and only text that asks for this pays it.

use std::sync::LazyLock;

use rsui_core::{Element, PropValue, Symbol};
use rsui_layout::{Rect, Size};

use crate::bidi;
use crate::dictionary::{needs_dictionary, word_starts};
use crate::engine::{PlacedGlyph, TextEngine, TextLayout, TextStyle};
use crate::lines::{WIDTH_TOLERANCE, can_break_between, is_ideograph, is_space};

/// A stretch of a paragraph with one look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextRun {
    /// Where it ends, as a byte position in the whole text. It starts where the one before ended.
    pub end: u32,
    pub style: TextStyle,
    /// How much room comes before its text and after it, for the padding and borders of what it's in that start and end with it,
    /// which its box takes in, and their margins, which it doesn't, as CSS's inline boxes have them.
    pub before: f32,
    pub after: f32,
    pub margin_before: f32,
    pub margin_after: f32,
    /// How far its box goes above its text and below it, for the padding and borders of the element that's only it, which take
    /// no room on its line.
    pub over: f32,
    pub under: f32,
}

/// The text of a paragraph and how each part of it looks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RichText {
    pub text: String,
    pub runs: Vec<TextRun>,
    /// How far up from where its lines are its box starts, for a paragraph whose box is only as tall as its fonts make it (see
    /// `TextStyle::inline_area`), which everything in it is moved up by: what [`TextEngine::inline_area`] says, or 0.
    pub inline_top: f32,
    /// Where what's in it starts and ends an isolate, embedding or override of its own, as CSS's `unicode-bidi` has its inline
    /// boxes, as the characters UAX #9 has for them (`U+2067` RLI to `U+2069` PDI and the rest), each at a byte of `text`,
    /// in order. They're only there for working out which way each part goes.
    pub controls: Vec<(u32, char)>,
}

impl RichText {
    pub fn clear(&mut self) {
        self.text.clear();
        self.runs.clear();
        self.inline_top = 0.0;
        self.controls.clear();
    }

    pub fn push(&mut self, text: &str, style: TextStyle) {
        self.text.push_str(text);
        self.runs.push(TextRun {
            end: self.text.len() as u32,
            style,
            before: 0.0,
            after: 0.0,
            margin_before: 0.0,
            margin_after: 0.0,
            over: 0.0,
            under: 0.0,
        });
    }
}

/// Part of a run, on one line, with its glyphs next to each other.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Piece {
    /// Which of the text's runs it's from.
    pub run: u32,
    /// How far along the line it starts, from the left of the paragraph.
    pub x: f32,
    pub width: f32,
    /// How much space to its left and right belongs to the same run, so that an underline or a
    /// background carries across it.
    pub before: f32,
    pub after: f32,
    /// Where the run's own line of text would be, which can be less than the whole line.
    pub top: f32,
    pub height: f32,
    /// The range of [`RichLayout::glyphs`] it has.
    pub first_glyph: u32,
    pub glyph_count: u32,
    /// Which bytes of the whole text it shows.
    pub start: u32,
    pub end: u32,
    /// How wide the spaces that come next in the text are, whatever they belong to, on whichever
    /// side of it they go.
    pub spaces_before: f32,
    pub spaces_after: f32,
    /// Which way it goes, as a level from the bidirectional algorithm: see [`Piece::style`].
    pub level: u8,
}

impl Piece {
    /// The style to measure its text with, and find carets and selections in, from its run's: its letters all go the way its
    /// level says, in the order the paragraph's bidirectional levels put them in, as each piece has one level.
    pub fn style(&self, run: TextStyle) -> TextStyle {
        TextStyle {
            right_to_left: self.level % 2 == 1,
            direction_said: true,
            bidi_override: true,
            ..run
        }
    }
}

/// One line of a paragraph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RichLine {
    pub top: f32,
    pub height: f32,
    pub baseline: f32,
    /// How far from the left of the paragraph it starts.
    pub x: f32,
    pub width: f32,
    /// The range of [`RichLayout::pieces`] on it, from left to right.
    pub first_piece: u32,
    pub piece_count: u32,
    /// Whether it's in a right-to-left language, which starts from the other side.
    pub right_to_left: bool,
}

/// Part of the text between places a line could end, before it's found a line to be on.
#[derive(Debug, Clone, Copy)]
struct Fragment {
    run: u32,
    start: u32,
    end: u32,
    width: f32,
    /// How wide the spaces after it are, which only count if something comes after them.
    gap: f32,
    height: f32,
    ascent: f32,
    /// How tall its text is, and how far above its own baseline that goes, from its fonts alone, whatever `line-height` says,
    /// which is what a background behind it covers, as CSS has it.
    box_height: f32,
    box_ascent: f32,
    /// How far above the line's baseline its own is, as `TextStyle::rise` says.
    rise: f32,
    /// How much of its width, at each side, is the margins of what it's in, which its box doesn't take in.
    margin_before: f32,
    margin_after: f32,
    /// How far its box goes above and below its text, past what its fonts make it: see `TextRun::over`.
    over: f32,
    under: f32,
    first_glyph: u32,
    glyph_count: u32,
    /// Which way it runs, as a level from the bidirectional algorithm.
    level: u8,
    /// Which way what it's in runs, which is the same as a level for its paragraph.
    base: u8,
    /// Whether a line can't end just before this.
    glued: bool,
    /// Whether this ends its line whatever else there's room for.
    ends_line: bool,
}

/// A paragraph arranged into lines, ready to draw.
#[derive(Debug, Clone, Default)]
pub struct RichLayout {
    pub lines: Vec<RichLine>,
    pub pieces: Vec<Piece>,
    /// Pens are from the left of the paragraph, and baselines from its top.
    pub glyphs: Vec<PlacedGlyph>,
    width: f32,
    height: f32,
    fragments: Vec<Fragment>,
    placing: TextLayout,
    order: Vec<u32>,
}

impl RichLayout {
    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn height(&self) -> f32 {
        self.height
    }

    /// Which run is at this position, from the top left of the paragraph, if any is.
    pub fn run_at(&self, x: f32, y: f32) -> Option<u32> {
        let line = self
            .lines
            .iter()
            .find(|line| y >= line.top && y < line.top + line.height)?;
        let first = line.first_piece as usize;
        self.pieces[first..first + line.piece_count as usize]
            .iter()
            .find(|piece| x >= piece.x - piece.before && x < piece.x + piece.width + piece.after)
            .map(|piece| piece.run)
    }
}

/// What's kept between uses, so that its memory is reused.
#[derive(Default)]
pub struct RichScratch {
    pub text: RichText,
    pub layout: RichLayout,
}

/// What a fragment's being cut from, while it's still growing.
struct Cutting {
    base: u8,
    start: usize,
    /// Where what's visible ends, which is before any spaces.
    end: usize,
    gap: f32,
    run: usize,
    level: u8,
    glued: bool,
}

/// How a paragraph sits: whether it's kept to one line, and how far across its lines go, from 0 at
/// the left to 1 at the right, if that's been set.
pub fn paragraph_arrangement(element: &Element) -> (bool, Option<f32>) {
    static NAMES: LazyLock<[Symbol; 3]> = LazyLock::new(|| {
        [
            Symbol::new("no_wrap"),
            Symbol::new("ellipsis"),
            Symbol::new("text_align"),
        ]
    });
    if element.props.is_empty() {
        return (false, None);
    }
    let on = |name| element.props.get_symbol(name) == Some(&PropValue::Bool(true));
    let along = match element.props.get_symbol(NAMES[2]) {
        Some(PropValue::Str(side)) => match side.as_str() {
            "left" => Some(0.0),
            "center" => Some(0.5),
            "right" => Some(1.0),
            _ => None,
        },
        _ => None,
    };
    (on(NAMES[0]) || on(NAMES[1]), along)
}

/// Goes through what's in a paragraph, adding each part with its own look to `text`.
///
/// Whatever an element doesn't say about its look comes from what it's in. `look` is told about
/// each element and what the one it's in got, and whether this one is a run of the text, in
/// order, for anything else that's inherited.
pub fn gather_text<'e, P: Copy>(
    paragraph: &'e Element,
    style: TextStyle,
    scale: f32,
    inherited: P,
    look: &mut impl FnMut(&'e Element, P, bool) -> P,
    text: &mut RichText,
) {
    for child in &paragraph.children {
        let is_run = child.children.is_empty() && !child.name.is_empty();
        let own_style = style.inside(child, scale);
        let own = look(child, inherited, is_run);
        let first = text.runs.len();
        let control = bidi_control(child);
        if let Some((opens, _, overrides)) = control {
            text.controls.push((text.text.len() as u32, opens));
            if overrides {
                let way = if opens == '\u{2067}' { '\u{202E}' } else { '\u{202D}' };
                text.controls.push((text.text.len() as u32, way));
            }
        }
        if is_run {
            text.push(&child.name, own_style);
        } else {
            gather_text(child, own_style, scale, own, look, text);
        }
        if let Some((_, closes, overrides)) = control {
            if overrides {
                text.controls.push((text.text.len() as u32, '\u{202C}'));
            }
            text.controls.push((text.text.len() as u32, closes));
        }
        // Its padding, borders and margins at its sides are room before its first run and after its last.
        if text.runs.len() > first
            && let Some([pad_left, pad_right, margin_left, margin_right, pad_top, pad_bottom]) = sides_of(child)
        {
            let last = text.runs.len() - 1;
            text.runs[first].before += pad_left * scale;
            text.runs[first].margin_before += margin_left * scale;
            text.runs[last].after += pad_right * scale;
            text.runs[last].margin_after += margin_right * scale;
            if is_run {
                text.runs[first].over += pad_top * scale;
                text.runs[first].under += pad_bottom * scale;
            }
        }
    }
}

/// What an inline part of a paragraph starts and ends as far as which way its letters go, as CSS's `unicode-bidi` says, with
/// `direction` (`text_direction`) saying which way: an isolate (`dir` says that for itself), an embedding, an override, or
/// nothing (`normal`, which most are).
fn bidi_control(element: &Element) -> Option<(char, char, bool)> {
    static DIRECTION: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("text_direction"));
    if element.props.is_empty() {
        return None;
    }
    let Some(PropValue::Str(how)) = element.props.get_symbol(*crate::engine::UNICODE_BIDI) else {
        return None;
    };
    let right_to_left = matches!(element.props.get_symbol(*DIRECTION), Some(PropValue::Str(way)) if way.as_str() == "rtl");
    let opens = match (how.as_str(), right_to_left) {
        ("isolate", true) => '\u{2067}',
        ("isolate", false) => '\u{2066}',
        ("plaintext", _) => '\u{2068}',
        ("embed", true) => '\u{202B}',
        ("embed", false) => '\u{202A}',
        ("bidi-override", true) => '\u{202E}',
        ("bidi-override", false) => '\u{202D}',
        // An isolate with an override inside it.
        ("isolate-override", true) => '\u{2067}',
        ("isolate-override", false) => '\u{2066}',
        _ => return None,
    };
    let closes = if matches!(opens, '\u{2066}' | '\u{2067}' | '\u{2068}') { '\u{2069}' } else { '\u{202C}' };
    Some((opens, closes, how.as_str() == "isolate-override"))

}

/// How much padding something in a paragraph has on its left and right, which takes in its border, as what's laid out has it,
/// then its margins there, and its padding above and below, if it has any.
fn sides_of(element: &Element) -> Option<[f32; 6]> {
    static NAMES: LazyLock<[Symbol; 8]> = LazyLock::new(|| {
        [
            Symbol::new("padding_left"),
            Symbol::new("padding_right"),
            Symbol::new("margin_left"),
            Symbol::new("margin_right"),
            Symbol::new("padding_x"),
            Symbol::new("padding_top"),
            Symbol::new("padding_bottom"),
            Symbol::new("padding_y"),
        ]
    });
    if element.props.is_empty() {
        return None;
    }
    let number = |name: Symbol| match element.props.get_symbol(name) {
        Some(PropValue::Float(value)) => Some(*value as f32),
        Some(PropValue::Int(value)) => Some(*value as f32),
        _ => None,
    };
    let padding = |side: Symbol, both: Symbol| number(side).or_else(|| number(both)).unwrap_or(0.0).max(0.0);
    let sides = [
        padding(NAMES[0], NAMES[4]),
        padding(NAMES[1], NAMES[4]),
        number(NAMES[2]).unwrap_or(0.0),
        number(NAMES[3]).unwrap_or(0.0),
        padding(NAMES[5], NAMES[7]),
        padding(NAMES[6], NAMES[7]),
    ];
    sides.iter().any(|side| *side != 0.0).then_some(sides)
}

/// All of the text of a paragraph, with its bytes numbered as [`Piece`]s number them.
pub fn paragraph_text(paragraph: &Element) -> String {
    fn add(element: &Element, text: &mut String) {
        for child in &element.children {
            if child.children.is_empty() {
                text.push_str(&child.name);
            } else {
                add(child, text);
            }
        }
    }
    let mut text = String::new();
    add(paragraph, &mut text);
    text
}

/// Whether this is text that the pointer can select, to copy it.
pub fn is_selectable(element: &Element) -> bool {
    static NAME: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("selectable"));
    element.props.get_symbol(*NAME) == Some(&PropValue::Bool(true))
        && rsui_layout::is_paragraph(element)
}

/// Where in a paragraph its run with this number comes from, as a path from it.
pub fn path_of_run(paragraph: &Element, run: u32) -> Option<Vec<u32>> {
    fn find(element: &Element, left: &mut u32, path: &mut Vec<u32>) -> bool {
        for (index, child) in element.children.iter().enumerate() {
            path.push(index as u32);
            if child.children.is_empty() {
                if !child.name.is_empty() {
                    if *left == 0 {
                        return true;
                    }
                    *left -= 1;
                }
            } else if find(child, left, path) {
                return true;
            }
            path.pop();
        }
        false
    }
    let mut left = run;
    let mut path = Vec::new();
    find(paragraph, &mut left, &mut path).then_some(path)
}

impl TextStyle {
    /// The look of something inside text with this look, which only says what's different.
    fn inside(mut self, element: &Element, scale: f32) -> Self {
        if element.props.is_empty() {
            return self;
        }
        // Up or down from the text it's in, by how big that is, as Blink has `super` and `sub`.
        static VERTICAL_ALIGN: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("vertical_align"));
        match element.props.get_symbol(*VERTICAL_ALIGN) {
            Some(PropValue::Str(how)) if how.as_str() == "super" => {
                self.rise += (self.size / 3.0 + scale).floor();
            }
            Some(PropValue::Str(how)) if how.as_str() == "sub" => {
                self.rise -= (self.size / 5.0 + scale).floor();
            }
            _ => {}
        }
        let own = Self::from_element(element);
        let names = &*crate::engine::STYLE_NAMES;
        if element.props.get_symbol(names.0).is_some() {
            self.size = own.size * scale;
        }
        if element.props.get_symbol(names.1).is_some() {
            self.font = own.font;
        }
        if own.family.is_some() {
            self.family = own.family;
        }
        if element.props.get_symbol(names.3).is_some() {
            self.italic = own.italic;
        }
        // Its own line height, which only says how much room it takes on the line, as each part of a CSS line has one.
        if element.props.get_symbol(names.4).is_some() {
            self.line_height = own.line_height.map(|height| height * scale);
        }
        // How far apart its letters are, as MUI's Typography says.
        static LETTER_SPACING: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("letter_spacing"));
        if element.props.get_symbol(*LETTER_SPACING).is_some() {
            self.letter_spacing = own.letter_spacing * scale;
        }
        if element.props.get_symbol(*crate::engine::WORD_SPACING).is_some() {
            self.word_spacing = own.word_spacing * scale;
        }
        if element.props.get_symbol(*crate::engine::TABULAR).is_some() {
            self.tabular = own.tabular;
        }
        if element.props.get_symbol(*crate::engine::FONT_METRICS).is_some() {
            self.metrics = own.metrics;
        }
        self
    }
}

impl TextEngine {
    /// Identifies the current scratch contents for the renderer. Any public
    /// checkout, return, cache reset or lazy-font registration invalidates it.
    /// Also compare engine identity and open-font count before reusing layout.
    #[doc(hidden)]
    pub fn rich_scratch_revision(&self) -> Option<u64> {
        (self.rich_revision != u64::MAX).then_some(self.rich_revision)
    }

    pub(crate) fn invalidate_rich_scratch(&mut self) {
        self.rich_revision = self.rich_revision.saturating_add(1);
    }

    /// Something to arrange a paragraph with. Give it back with [`give_back_rich`](Self::give_back_rich).
    pub fn take_rich(&mut self) -> Box<RichScratch> {
        self.invalidate_rich_scratch();
        self.rich.take().unwrap_or_default()
    }

    /// Keeps this for next time, for the memory it has.
    pub fn give_back_rich(&mut self, scratch: Box<RichScratch>) {
        self.invalidate_rich_scratch();
        self.rich = Some(scratch);
    }

    /// Arranges a paragraph into lines, breaking them to stay within `max_width` if it can. `along` says
    /// how far across its lines go, from 0 at the left to 1 at the right. Otherwise they start on the side
    /// the paragraph's language starts on. Use infinity for no limit, which leaves them all at the left.
    pub fn layout_rich_into(
        &mut self,
        rich: &RichText,
        max_width: f32,
        along: Option<f32>,
        layout: &mut RichLayout,
    ) {
        layout.lines.clear();
        layout.pieces.clear();
        layout.glyphs.clear();
        layout.fragments.clear();
        layout.width = 0.0;
        layout.height = 0.0;
        if rich.runs.is_empty() {
            return;
        }
        self.cut_into_fragments(rich, max_width, layout);
        fill_lines(layout, max_width);
        if max_width.is_finite() {
            layout.move_lines_across(max_width, along);
        }
        if rich.inline_top != 0.0 {
            let up = rich.inline_top;
            for line in &mut layout.lines {
                line.top -= up;
                line.baseline -= up;
            }
            for piece in &mut layout.pieces {
                piece.top -= up;
            }
            for glyph in &mut layout.glyphs {
                glyph.y -= up;
            }
        }
    }

    /// Cuts the text at each place a line could end or the look or direction changes.
    fn cut_into_fragments(&mut self, rich: &RichText, max_width: f32, layout: &mut RichLayout) {
        let text = rich.text.as_str();
        // What English doesn't need comes at no cost to English.
        let mut levels = None;
        let mut words = Vec::new();
        // Which way the paragraph goes, as its own style says, which its runs keep.
        let key = rich.runs[0].style.bidi_key();
        let paragraph_level = crate::complex::level_of_key(key);
        let default_base = paragraph_level.unwrap_or(0);
        if !text.is_ascii() || !rich.controls.is_empty() || crate::complex::reorders(text, key) {
            // The characters, with what starts and ends each isolate or embedding where it does.
            let mut characters: Vec<char> = Vec::with_capacity(text.len() + rich.controls.len());
            let mut original = Vec::with_capacity(text.len());
            let mut next_control = 0;
            for (byte, character) in text.char_indices() {
                while let Some((at, control)) = rich.controls.get(next_control) {
                    if *at as usize > byte {
                        break;
                    }
                    characters.push(*control);
                    next_control += 1;
                }
                original.push(characters.len());
                characters.push(character);
            }
            characters.extend(rich.controls[next_control..].iter().map(|(_, control)| *control));
            // Each line break starts a new paragraph, as far as which way it runs goes.
            let mut found_levels = Vec::with_capacity(characters.len());
            let mut found_bases = Vec::with_capacity(characters.len());
            for part in characters.split_inclusive(|character| *character == '\n') {
                let mut found = bidi::paragraph(part, paragraph_level);
                if key & 4 != 0 {
                    found.levels.fill(found.level);
                }
                found_levels.extend_from_slice(&found.levels);
                found_bases.resize(found_levels.len(), found.level);
            }
            let all_levels: Vec<u8> = original.iter().map(|at| found_levels[*at]).collect();
            let all_bases: Vec<u8> = original.iter().map(|at| found_bases[*at]).collect();
            if all_levels.iter().chain(&all_bases).any(|level| *level > 0) {
                levels = Some((all_levels, all_bases));
            }
            if max_width.is_finite() && text.chars().any(needs_dictionary) {
                let with_bytes: Vec<(u32, char)> = text
                    .char_indices()
                    .map(|(byte, character)| (byte as u32, character))
                    .collect();
                words = word_starts(&with_bytes)
                    .into_iter()
                    .filter_map(|at| with_bytes.get(at).map(|(byte, _)| *byte as usize))
                    .collect();
            }
        }
        let mut cut = Cutting {
            start: 0,
            end: 0,
            gap: 0.0,
            run: 0,
            level: levels.as_ref().map_or(0, |(levels, _)| levels[0]),
            base: levels.as_ref().map_or(default_base, |(_, bases)| bases[0]),
            glued: false,
        };
        let mut previous = None;
        for (index, (byte, character)) in text.char_indices().enumerate() {
            let level = levels.as_ref().map_or(0, |(levels, _)| levels[index]);
            let base = levels.as_ref().map_or(default_base, |(_, bases)| bases[index]);

            let new_run = byte as u32 >= rich.runs[cut.run].end;
            // Spaces stay with what's before them, wherever a line ends.
            let can_end_here = !is_space(character)
                && character != '\n'
                && (cut.gap > 0.0
                    || previous.is_some_and(|previous| can_break_between(previous, character))
                    || is_ideograph(character)
                    || words.binary_search(&byte).is_ok());
            if new_run || level != cut.level || can_end_here {
                if byte > cut.start {
                    self.close_fragment(rich, &cut, false, max_width, layout);
                }
                while byte as u32 >= rich.runs[cut.run].end {
                    cut.run += 1;
                }
                cut.start = byte;
                cut.end = byte;
                cut.gap = 0.0;
                cut.level = level;
                cut.base = base;
                cut.glued = !can_end_here && previous.is_some();
            }
            if character == '\n' {
                self.close_fragment(rich, &cut, true, max_width, layout);
                cut.start = byte + 1;
                cut.end = byte + 1;
                cut.gap = 0.0;
                cut.glued = false;
                // What comes next says which way it runs itself.
                if let Some((next_levels, next_bases)) = &levels {
                    if let (Some(next_level), Some(next_base)) =
                        (next_levels.get(index + 1), next_bases.get(index + 1))
                    {
                        cut.level = *next_level;
                        cut.base = *next_base;
                    }
                }
                previous = None;
                continue;
            }
            if is_space(character) {
                let style = rich.runs[cut.run].style;
                self.prepare_font(style);
                let primary = self.primary(style);
                cut.gap += self.character_width(primary, style.size, None, character) + style.letter_spacing + style.word_spacing;
            } else if character != '\r' {
                cut.end = byte + character.len_utf8();
            }
            previous = Some(character);
        }
        if cut.start < text.len() || text.ends_with('\n') {
            self.close_fragment(rich, &cut, false, max_width, layout);
        }
    }

    /// Finishes a fragment, in several if it's too wide to fit on any line.
    fn close_fragment(
        &mut self,
        rich: &RichText,
        cut: &Cutting,
        ends_line: bool,
        max_width: f32,
        layout: &mut RichLayout,
    ) {
        let style = rich.runs[cut.run].style;
        let visible = &rich.text[cut.start..cut.end];
        // Only spaces belong with what's before them, even if they're in a different part.
        if visible.is_empty() && cut.gap > 0.0 {
            if let Some(last) = layout.fragments.last_mut() {
                if !last.ends_line && last.level == cut.level {
                    last.gap += cut.gap;
                    last.ends_line = ends_line;
                    return;
                }
            }
        }
        let width = self.place_fragment(rich, visible, cut.start, cut, style, layout);
        let too_wide = width > max_width + WIDTH_TOLERANCE && visible.chars().nth(1).is_some();
        let starts = if too_wide {
            let own = TextStyle {
                right_to_left: cut.level % 2 == 1,
                direction_said: true,
                bidi_override: true,
                ..style
            };
            self.wrapped_starts(visible, own, max_width)
        } else {
            Vec::new()
        };
        if starts.len() > 1 {
            // What was placed is for all of it, and this comes back with each part instead.
            let placed = layout.fragments.pop().expect("it was just added");
            layout.glyphs.truncate(placed.first_glyph as usize);
            for (at, start) in starts.iter().enumerate() {
                let end = starts.get(at + 1).copied().unwrap_or(visible.len());
                self.place_fragment(
                    rich,
                    &visible[*start..end],
                    cut.start + *start,
                    cut,
                    style,
                    layout,
                );
                let added = layout.fragments.last_mut().expect("it was just added");
                if at > 0 {
                    added.glued = false;
                }
                added.gap = 0.0;
            }
        }
        let added = layout.fragments.last_mut().expect("something was added");
        added.gap = cut.gap;
        added.ends_line = ends_line;
    }

    /// Adds a fragment that doesn't need cutting any further, saying how wide it is.
    fn place_fragment(
        &mut self,
        rich: &RichText,
        visible: &str,
        at: usize,
        cut: &Cutting,
        style: TextStyle,
        layout: &mut RichLayout,
    ) -> f32 {
        // Room before its run's text, if it's where that starts, and after it, if only spaces come after it in its run.
        let run = rich.runs[cut.run];
        let run_start = if cut.run == 0 { 0 } else { rich.runs[cut.run - 1].end as usize };
        let (lead, margin_before) = if at == run_start {
            (run.before + run.margin_before, run.margin_before)
        } else {
            (0.0, 0.0)
        };
        let ends = (at + visible.len()).min(run.end as usize);
        let (trail, margin_after) = if rich.text[ends..run.end as usize].chars().all(is_space) {
            (run.after + run.margin_after, run.margin_after)
        } else {
            (0.0, 0.0)
        };
        self.prepare_font(style);
        let primary = self.primary(style);
        // A part in another font, as a tick often is, makes its line room for that font, as a browser's do.
        let (height, ascent) = self.metrics_for(visible, primary, style);
        let (box_height, box_ascent) = if style.line_height.is_some() {
            self.metrics_for(
                visible,
                primary,
                TextStyle {
                    line_height: None,
                    ..style
                },
            )
        } else {
            (height, ascent)
        };
        // Something raised reaches higher, and something lowered lower, which the line makes room for.
        let (height, ascent) = (height + style.rise.abs(), ascent + style.rise.max(0.0));
        let first_glyph = layout.glyphs.len() as u32;
        let mut width = 0.0;
        if !visible.is_empty() {
            // A run's place is the paragraph's to say, whatever its own box would be on its own. It has one level, so its letters
            // go all one way.
            let own = TextStyle {
                right_to_left: cut.level % 2 == 1,
                direction_said: true,
                bidi_override: true,
                ..style
            };
            self.layout_lines_into(visible, own, f32::INFINITY, &mut layout.placing);
            crate::engine::space_letters(&mut layout.placing, style.letter_spacing);
            crate::engine::space_words(&mut layout.placing, visible, style.word_spacing);
            if let Some(line) = layout.placing.lines.first() {
                width = line.width;
                layout
                    .glyphs
                    .extend(layout.placing.glyphs.iter().map(|glyph| PlacedGlyph {
                        x: glyph.x - line.x + lead,
                        y: 0.0,
                        ..*glyph
                    }));
            }
        }
        width += lead + trail;
        layout.fragments.push(Fragment {
            run: cut.run as u32,
            start: at as u32,
            end: (at + visible.len()) as u32,
            width,
            gap: 0.0,
            height,
            ascent,
            box_height,
            box_ascent,
            rise: style.rise,
            margin_before,
            margin_after,
            over: run.over,
            under: run.under,
            first_glyph,
            glyph_count: layout.glyphs.len() as u32 - first_glyph,
            level: cut.level,
            base: cut.base,
            glued: cut.glued,
            ends_line: false,
        });
        width
    }

    /// The space a paragraph needs.
    pub(crate) fn measure_paragraph(&mut self, element: &Element, max_width: f32) -> Size {
        let (one_line, _) = paragraph_arrangement(element);
        let mut scratch = self.take_rich();
        let RichScratch { text, layout } = &mut *scratch;
        self.arrange_paragraph(element, 1.0, max_width, one_line, None, text, layout);
        let size = Size::new(
            layout
                .width()
                .ceil()
                .min(if one_line { max_width } else { f32::INFINITY }),
            layout.height(),
        );
        self.give_back_rich(scratch);
        size
    }

    /// Fills in where each run of a paragraph is, as a box around all of its parts.
    pub(crate) fn places_of_runs(&mut self, element: &Element, width: f32, places: &mut Vec<Rect>) {
        let (one_line, along) = paragraph_arrangement(element);
        let mut scratch = self.take_rich();
        let RichScratch { text, layout } = &mut *scratch;
        self.arrange_paragraph(element, 1.0, width, one_line, along, text, layout);
        places.clear();
        places.resize(text.runs.len(), Rect::new(0.0, 0.0, 0.0, 0.0));
        let mut found = vec![false; text.runs.len()];
        for piece in &layout.pieces {
            let run = piece.run as usize;
            let area = Rect::new(piece.x, piece.top, piece.width, piece.height);
            places[run] = if found[run] {
                around(places[run], area)
            } else {
                area
            };
            found[run] = true;
        }
        self.give_back_rich(scratch);
    }

    /// Which part of a paragraph is at this position, as a path from it, with everything in physical
    /// pixels and from the top left of its content. `None` if it's not on any of them.
    pub fn paragraph_part_at(
        &mut self,
        element: &Element,
        scale: f32,
        width: f32,
        (x, y): (f32, f32),
    ) -> Option<Vec<u32>> {
        let (one_line, along) = paragraph_arrangement(element);
        let mut scratch = self.take_rich();
        let RichScratch { text, layout } = &mut *scratch;
        self.arrange_paragraph(element, scale, width, one_line, along, text, layout);
        let run = layout.run_at(x, y);
        self.give_back_rich(scratch);
        path_of_run(element, run?)
    }

    /// The character boundary in a paragraph nearest this position, as a byte of [`paragraph_text`], with
    /// everything in physical pixels and from the top left of its content.
    pub fn paragraph_byte_at(
        &mut self,
        element: &Element,
        scale: f32,
        width: f32,
        (x, y): (f32, f32),
    ) -> usize {
        let (one_line, along) = paragraph_arrangement(element);
        let mut scratch = self.take_rich();
        let RichScratch { text, layout } = &mut *scratch;
        self.arrange_paragraph(element, scale, width, one_line, along, text, layout);
        let at = self.rich_byte_at(text, layout, x, y);
        self.give_back_rich(scratch);
        at
    }

    /// Where a caret just before byte `at` of [`paragraph_text`] goes: how far along it is, where the top of its line is and how
    /// tall that is, in physical pixels from the top left of its content. A place between two parts of a line goes with the
    /// later, and one past the end at the end of the last line. `None` for a paragraph with no lines.
    pub fn paragraph_caret(&mut self, element: &Element, scale: f32, width: f32, at: usize) -> Option<(f32, f32, f32)> {
        let (one_line, along) = paragraph_arrangement(element);
        let mut scratch = self.take_rich();
        let RichScratch { text, layout } = &mut *scratch;
        self.arrange_paragraph(element, scale, width, one_line, along, text, layout);
        let mut found: Option<(f32, f32, f32)> = None;
        let mut in_order: Vec<Piece> = Vec::new();
        'lines: for line in &layout.lines {
            let pieces = &layout.pieces[line.first_piece as usize..(line.first_piece + line.piece_count) as usize];
            // In the order of the text, which on a line that goes from the right isn't the order they're drawn in.
            in_order.clear();
            in_order.extend_from_slice(pieces);
            in_order.sort_by_key(|piece| piece.start);
            for piece in &in_order {
                let (start, end) = (piece.start as usize, piece.end as usize);
                let backwards = piece.level % 2 == 1;
                if at < start {
                    // In the spaces before this piece, which are drawn after the one before it.
                    if found.is_none() {
                        found = Some((if backwards { piece.x + piece.width } else { piece.x }, line.top, line.height));
                    }
                    break 'lines;
                }
                if at <= end && end <= text.text.len() && text.text.is_char_boundary(start) && text.text.is_char_boundary(at) {
                    let style = piece.style(text.runs[piece.run as usize].style);
                    let x = piece.x + self.caret_x(&text.text[start..end], style, at - start);
                    found = Some((x, line.top, line.height));
                    if at < end {
                        break 'lines;
                    }
                } else if at > end {
                    // After it, in what spaces follow it, as far as they go.
                    let style = text.runs[piece.run as usize].style;
                    let space = self.caret_x(" ", style, 1);
                    let past = (at - end) as f32 * space;
                    let x = if backwards {
                        piece.x - past.min(piece.spaces_before.max(space))
                    } else {
                        piece.x + piece.width + past.min(piece.spaces_after.max(space))
                    };
                    found = Some((x, line.top, line.height));
                }
            }

            if pieces.is_empty() && found.is_none() {
                found = Some((line.x, line.top, line.height));
            }
        }
        self.give_back_rich(scratch);
        found.or_else(|| layout_first_line(element, self, scale))
    }

    /// The text of a text element, as [`element_caret`](Self::element_caret) and [`element_byte_at`](Self::element_byte_at)
    /// number its bytes: a paragraph's, or the element's own.
    pub fn element_text(element: &Element) -> String {
        if rsui_layout::is_paragraph(element) { paragraph_text(element) } else { element.name.to_string() }
    }

    /// Where a caret just before byte `at` of a text element's text goes, laid out `width` across: how far along, where its line's
    /// top is and how tall that is, from the top left of its content, in logical pixels.
    pub fn element_caret(&mut self, element: &Element, width: f32, at: usize) -> Option<(f32, f32, f32)> {
        if rsui_layout::is_paragraph(element) {
            return self.paragraph_caret(element, 1.0, width, at);
        }
        let text = element.name.as_str();
        let style = TextStyle::from_element(element);
        let layout = self.layout(text, style, width);
        let count = layout.lines.len();
        let line_height = if count == 0 { self.line_height(style) } else { layout.height() / count as f32 };
        for (index, line) in layout.lines.iter().enumerate() {
            if at <= line.end || index == count - 1 {
                let (start, end) = (line.start.min(text.len()), line.end.min(text.len()));
                let local = at.clamp(start, end) - start;
                let x = line.x + self.caret_x(&text[start..end], style, local);
                return Some((x, index as f32 * line_height, line_height));
            }
        }
        Some((0.0, 0.0, line_height))
    }

    /// The byte of a text element's text nearest a place in it, from the top left of its content, laid out `width` across.
    pub fn element_byte_at(&mut self, element: &Element, width: f32, (x, y): (f32, f32)) -> usize {
        if rsui_layout::is_paragraph(element) {
            return self.paragraph_byte_at(element, 1.0, width, (x, y));
        }
        let text = element.name.as_str();
        let style = TextStyle::from_element(element);
        let layout = self.layout(text, style, width);
        let count = layout.lines.len();
        if count == 0 {
            return 0;
        }
        let line_height = layout.height() / count as f32;
        let index = ((y / line_height.max(1.0)).floor().max(0.0) as usize).min(count - 1);
        let line = layout.lines[index];
        let (start, end) = (line.start.min(text.len()), line.end.min(text.len()));
        start + self.byte_at_x(&text[start..end], style, x - line.x)
    }

    fn rich_byte_at(&mut self, rich: &RichText, layout: &RichLayout, x: f32, y: f32) -> usize {
        let line = layout
            .lines
            .iter()
            .find(|line| y < line.top + line.height)
            .or(layout.lines.last());
        let Some(line) = line else { return 0 };
        let first = line.first_piece as usize;
        let how_far = |piece: &&Piece| {
            if x < piece.x {
                piece.x - x
            } else {
                (x - piece.x - piece.width).max(0.0)
            }
        };
        let nearest = layout.pieces[first..first + line.piece_count as usize]
            .iter()
            .min_by(|a, b| how_far(a).total_cmp(&how_far(b)));
        let Some(piece) = nearest else { return 0 };
        let (start, end) = (piece.start as usize, piece.end as usize);
        let style = piece.style(rich.runs[piece.run as usize].style);
        start + self.byte_at_x(&rich.text[start..end], style, x - piece.x)

    }

    #[allow(clippy::too_many_arguments)]
    fn arrange_paragraph(
        &mut self,
        element: &Element,
        scale: f32,
        width: f32,
        one_line: bool,
        along: Option<f32>,
        text: &mut RichText,
        layout: &mut RichLayout,
    ) {
        let mut style = TextStyle::from_element(element);
        style.size *= scale;
        text.clear();
        gather_text(element, style, scale, (), &mut |_, (), _| (), text);
        text.inline_top = self.inline_area(TextStyle::from_element(element).scaled(scale)).map_or(0.0, |(top, _)| top);
        let limit = if one_line { f32::INFINITY } else { width };
        self.layout_rich_into(text, limit, along, layout);
    }
}

/// Where a caret goes in a paragraph with nothing in it: its first line, as tall as its text would be.
fn layout_first_line(element: &Element, engine: &mut TextEngine, scale: f32) -> Option<(f32, f32, f32)> {
    let style = TextStyle::from_element(element).scaled(scale);
    Some((0.0, 0.0, engine.line_height(style)))
}

fn around(one: Rect, other: Rect) -> Rect {
    let left = one.x.min(other.x);
    let top = one.y.min(other.y);
    let right = (one.x + one.width).max(other.x + other.width);
    let bottom = (one.y + one.height).max(other.y + other.height);
    Rect::new(left, top, right - left, bottom - top)
}

/// Puts fragments on lines, ending a line before whatever doesn't fit on it.
fn fill_lines(layout: &mut RichLayout, max_width: f32) {
    let limit = max_width + WIDTH_TOLERANCE;
    let count = layout.fragments.len();
    let mut line_start = 0;
    let mut pen = 0.0f32;
    let mut at = 0;
    while at < count {
        // What can't be parted has to fit as a whole, or go on the next line.
        let mut end = at + 1;
        let mut whole = layout.fragments[at].width;
        while end < count && layout.fragments[end].glued {
            whole += layout.fragments[end - 1].gap + layout.fragments[end].width;
            end += 1;
        }
        if at > line_start && pen + whole > limit {
            finish_line(layout, line_start, at);
            line_start = at;
            pen = 0.0;
        }
        for index in at..end {
            let fragment = layout.fragments[index];
            if index > line_start && fragment.width > 0.0 && pen + fragment.width > limit {
                finish_line(layout, line_start, index);
                line_start = index;
                pen = 0.0;
            }
            pen += fragment.width + fragment.gap;
            if fragment.ends_line {
                finish_line(layout, line_start, index + 1);
                line_start = index + 1;
                pen = 0.0;
            }
        }
        at = end;
    }
    if line_start < count {
        finish_line(layout, line_start, count);
    }
}

/// Places these fragments as one line, under the lines so far.
fn finish_line(layout: &mut RichLayout, start: usize, end: usize) {
    let RichLayout {
        lines,
        pieces,
        glyphs,
        width,
        height,
        fragments,
        order,
        ..
    } = layout;
    let on_line = &fragments[start..end];
    let (mut ascent, mut below) = (0.0f32, 0.0f32);
    for fragment in on_line {
        ascent = ascent.max(fragment.ascent);
        below = below.max(fragment.height - fragment.ascent);
    }
    let top = *height;
    let baseline = top + ascent;
    order.clear();
    order.extend(0..on_line.len() as u32);
    in_visual_order(order, on_line);
    let first_piece = pieces.len() as u32;
    let last = on_line.len() - 1;
    let mut pen = 0.0f32;
    for &index in order.iter() {
        let index = index as usize;
        let fragment = &on_line[index];
        let gap = if index == last { 0.0 } else { fragment.gap };
        let backwards = fragment.level % 2 == 1;
        if backwards {
            pen += gap;
        }
        let x = pen;
        pen += fragment.width;
        if !backwards {
            pen += gap;
        }
        let same_run_next = on_line
            .get(index + 1)
            .is_some_and(|next| next.run == fragment.run);
        let carried = if same_run_next { gap } else { 0.0 };
        let (first, count) = (fragment.first_glyph as usize, fragment.glyph_count as usize);
        for glyph in &mut glyphs[first..first + count] {
            glyph.x += x;
            glyph.y = baseline - fragment.rise;
        }
        pieces.push(Piece {
            run: fragment.run,
            x: x + fragment.margin_before,
            width: (fragment.width - fragment.margin_before - fragment.margin_after).max(0.0),
            before: if backwards { carried } else { 0.0 },
            after: if backwards { 0.0 } else { carried },
            top: baseline - fragment.rise - fragment.box_ascent - fragment.over,
            height: fragment.box_height + fragment.over + fragment.under,
            first_glyph: fragment.first_glyph,
            glyph_count: fragment.glyph_count,
            start: fragment.start,
            end: fragment.end,
            spaces_before: if backwards { gap } else { 0.0 },
            spaces_after: if backwards { 0.0 } else { gap },
            level: fragment.level,
        });
    }
    lines.push(RichLine {
        top,
        height: ascent + below,
        baseline,
        x: 0.0,
        width: pen,
        first_piece,
        piece_count: pieces.len() as u32 - first_piece,
        right_to_left: on_line[0].base % 2 == 1,
    });
    *width = width.max(pen);
    *height += ascent + below;
}

/// Puts what's on a line in the order it shows in, from left to right, as rule L2 says.
fn in_visual_order(order: &mut [u32], on_line: &[Fragment]) {
    let highest = on_line.iter().map(|one| one.level).max().unwrap_or(0);
    if highest == 0 {
        return;
    }
    let lowest_odd = on_line
        .iter()
        .map(|one| one.level)
        .filter(|level| level % 2 == 1)
        .min()
        .unwrap_or(highest + 1);
    for level in (lowest_odd..=highest).rev() {
        let mut at = 0;
        while at < order.len() {
            if on_line[order[at] as usize].level >= level {
                let mut end = at + 1;
                while end < order.len() && on_line[order[end] as usize].level >= level {
                    end += 1;
                }
                order[at..end].reverse();
                at = end;
            } else {
                at += 1;
            }
        }
    }
}

impl RichLayout {
    fn move_lines_across(&mut self, max_width: f32, along: Option<f32>) {
        for line in &mut self.lines {
            let along = along.unwrap_or(if line.right_to_left { 1.0 } else { 0.0 });
            if along == 0.0 {
                continue;
            }
            let shift = (max_width - line.width).max(0.0) * along;
            line.x += shift;
            let first = line.first_piece as usize;
            for piece in &mut self.pieces[first..first + line.piece_count as usize] {
                piece.x += shift;
                let start = piece.first_glyph as usize;
                for glyph in &mut self.glyphs[start..start + piece.glyph_count as usize] {
                    glyph.x += shift;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod revision_test_font;

#[cfg(test)]
#[test]
fn scratch_revision_saturates_and_does_not_accept_an_old_slot() {
    let mut text = TextEngine::new(revision_test_font::plain_font());
    let initial = text.rich_scratch_revision();
    let scratch = text.take_rich();
    assert_ne!(text.rich_scratch_revision(), initial);
    let taken = text.rich_scratch_revision();
    text.give_back_rich(scratch);
    assert_ne!(text.rich_scratch_revision(), taken);
    text.rich_revision = u64::MAX - 1;
    let scratch = text.take_rich();
    assert_eq!(text.rich_scratch_revision(), None);
    text.give_back_rich(scratch);
    text.forget_caches();
    assert_eq!(text.rich_scratch_revision(), None);
}

/// What [`rsui_layout::Measure::line_levels`] asks: a bidirectional level for each child of a line of text that goes from the
/// right. Text, a leaf's own or a paragraph's, is its characters; anything laid out as a box (one that says a `direction`, or
/// has boxes in it) is one object, U+FFFC, as CSS has an atomic inline. A child's level is its first visible character's, which
/// is the way it goes as a whole: a piece of text can't be split between places on its line, so `to search!` in a paragraph from
/// the right stays with the English before it, its `!` at its own left, rather than all of it going to the left of that.
pub(crate) fn line_levels(line: &Element, levels: &mut Vec<u8>) {
    static DIRECTION: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("direction"));
    levels.clear();
    let mut characters: Vec<char> = Vec::new();
    let mut spans = Vec::with_capacity(line.children.len());
    for child in &line.children {
        let start = characters.len();
        let boxed = child.props.get_symbol(*DIRECTION).is_some();
        if !boxed && child.children.is_empty() && rsui_layout::shows_own_text(child) {
            characters.extend(child.name.chars());
        } else if !boxed && rsui_layout::is_paragraph(child) {
            characters.extend(paragraph_text(child).chars());
        } else {
            characters.push('\u{FFFC}');
        }
        spans.push((start, characters.len()));
    }
    let found = bidi::paragraph(&characters, Some(1));
    for (start, end) in spans {
        let level = (start..end)
            .find(|at| !characters[*at].is_whitespace())
            .or(Some(start))
            .and_then(|at| found.levels.get(at).copied())
            .unwrap_or(1);

        levels.push(level);
    }
}
