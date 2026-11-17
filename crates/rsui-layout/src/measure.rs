use rsui_core::{Element, Role};

use crate::tree::{Rect, Size};

/// Measures what an element with no children needs to show its content, which
/// usually means its text.
pub trait Measure {
    /// The size `element` needs, given that it should fit within `max_width` if it
    /// can, by wrapping. `max_width` may be infinite, to ask for the size on one line.
    fn measure(&mut self, element: &Element, max_width: f32) -> Size;

    /// Where each part of a paragraph (see [`is_paragraph`](crate::is_paragraph)) is, for one this wide,
    /// from the top left of its content. There's one for each part that has text, in order. What's
    /// asked to measure text without drawing any needn't say.
    fn place_pieces(&mut self, _element: &Element, _width: f32, _places: &mut Vec<Rect>) {}

    /// How far a line of text written as `element` says goes below where its letters sit, which is room left under a picture
    /// that sits on a line, as one does on a page. What doesn't draw text needn't say.
    fn line_below(&mut self, _element: &Element) -> f32 {
        0.0
    }

    /// How far below the top of its content the first line of text written as `element` sits, which is what lines it up with
    /// text beside it. `None` is something with no text, and what doesn't draw text needn't say.
    fn first_baseline(&mut self, _element: &Element) -> Option<f32> {
        None
    }

    /// How far above and below its baseline a line of text written as `element` says goes, which is as little as each line of a
    /// row that's lined up by its text takes, with `line_strut`, as CSS has it. What doesn't draw text needn't say.
    fn strut(&mut self, _element: &Element) -> Option<(f32, f32)> {
        None
    }

    /// For text whose box is only as tall as its fonts make it, as CSS's inline box is (the `inline_area` prop): how far below
    /// where its lines start that box starts, and how much shorter than its lines it is. What doesn't draw text needn't say.
    fn inline_area(&mut self, _element: &Element) -> Option<(f32, f32)> {
        None
    }

    /// How tall a lowercase `x` is in text written as `element` says, which `vertical-align: middle` on a line goes by. What
    /// doesn't draw text needn't say.
    fn x_height(&mut self, _element: &Element) -> Option<f32> {
        None
    }

    /// For a line of text that goes from the right (`line_strut` and `text_direction: "rtl"`): which way each of `line`'s children
    /// goes, as a level from the Unicode Bidirectional Algorithm, one each, as its text comes to in the line's paragraph with
    /// what isn't text as one object (U+FFFC), as CSS has its inline boxes. The line is put in the order they say. Left empty,
    /// it stays in the order of its children, which is all what doesn't draw text needs.
    fn line_levels(&mut self, _line: &Element, _levels: &mut Vec<u8>) {}
}


impl<F: FnMut(&Element, f32) -> Size> Measure for F {
    fn measure(&mut self, element: &Element, max_width: f32) -> Size {
        self(element, max_width)
    }
}

/// How wide a text input is, unless its style says otherwise. A fixed size means that typing
/// never moves anything around it.
pub const TEXT_INPUT_WIDTH: f32 = 200.0;

/// How wide a text input's text is if nothing says otherwise: what its `natural_width` says, or [`TEXT_INPUT_WIDTH`].
pub fn input_width(element: &rsui_core::Element) -> f32 {
    match element.props.get("natural_width") {
        Some(rsui_core::PropValue::Float(width)) => *width as f32,
        Some(rsui_core::PropValue::Int(width)) => *width as f32,
        _ => TEXT_INPUT_WIDTH,
    }
}

/// Whether an element shows its name as text. Others only use it to tell agents and
/// screen readers what they are.
pub fn shows_name(element: &Element) -> bool {
    matches!(
        element.role,
        Role::Text | Role::Button | Role::Link | Role::Checkbox | Role::ListItem | Role::MenuItem
    ) && !quiet_name(element)
}

/// Whether its name is only what it's called, for anyone who can't see it, and isn't drawn, as a web page's `aria-label` is:
/// a Radix checkbox with `aria-label="Select all"` and nothing in it while it's unchecked drew "Select all" in its 16px box.
fn quiet_name(element: &Element) -> bool {
    static QUIET: std::sync::LazyLock<rsui_core::Symbol> = std::sync::LazyLock::new(|| rsui_core::Symbol::new("quiet_name"));
    !element.props.is_empty() && matches!(element.props.get_symbol(*QUIET), Some(rsui_core::PropValue::Bool(true)))
}

/// Measures text as if every character were the same size. Real text measuring comes
/// from a text engine. This is for tests and for running without a window, where it
/// gives results that are predictable, if not what a screen would show.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixedMetrics {
    pub char_width: f32,
    pub line_height: f32,
}

impl Default for FixedMetrics {
    fn default() -> Self {
        Self {
            char_width: 8.0,
            line_height: 16.0,
        }
    }
}

impl Measure for FixedMetrics {
    /// Three quarters of the way down a line, for text, which is near enough where a font's letters sit.
    fn first_baseline(&mut self, element: &Element) -> Option<f32> {
        let text = element.role == Role::TextInput
            || (!element.children.is_empty() && crate::is_paragraph(element))
            || (shows_name(element) && !element.name.is_empty());
        text.then_some(self.line_height * 0.75)
    }

    fn strut(&mut self, _element: &Element) -> Option<(f32, f32)> {
        Some((self.line_height * 0.75, self.line_height * 0.25))
    }

    fn measure(&mut self, element: &Element, max_width: f32) -> Size {
        if element.role == Role::TextInput {
            let rows = match element.props.get("rows") {
                Some(rsui_core::PropValue::Int(rows)) => (*rows).max(1) as f32,
                _ => 1.0,
            };
            return Size::new(input_width(element).min(max_width), self.line_height * rows);
        }
        let characters = if !element.children.is_empty() && crate::is_paragraph(element) {
            characters_inside(element)
        } else if !shows_name(element) || element.name.is_empty() {
            return Size::new(0.0, 0.0);
        } else {
            element.name.chars().count()
        };
        let per_line = ((max_width / self.char_width).floor() as usize).max(1);
        let lines = characters.div_ceil(per_line);
        Size::new(
            characters.min(per_line) as f32 * self.char_width,
            lines as f32 * self.line_height,
        )
    }
}

/// How many characters there are in all of a paragraph's parts.
fn characters_inside(element: &Element) -> usize {
    element
        .children
        .iter()
        .map(|child| {
            if child.children.is_empty() {
                child.name.chars().count()
            } else {
                characters_inside(child)
            }
        })
        .sum()
}
