//! Turns a tree into a list of things to draw.
#[cfg(feature = "mutation-pilot")]
#[path = "mutation_pilot.rs"]
pub(crate) mod mutation_pilot;
#[cfg(feature = "mutation-pilot")]
use mutation_pilot::PilotPaint;

use rsui_core::{Element, Node, PropValue, Role, Symbol};
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::LazyLock;

use rsui_layout::{Layout, LayoutVisitor, Rect, ScrollInfo, shows_name, shows_own_text};
use rsui_text::{
    AtlasGlyph, Font, GlyphAtlas, GlyphKey, Rasterizer, RichLayout, RichScratch, RichText,
    TextEngine, TextLayout, TextStyle, Upload, gather_text, paragraph_arrangement, whole_part,
};

use crate::images::{ColorGlyph, ColorGlyphKey, ImageUpload, Images};
use crate::paint::{Arrangement, BorderPattern, Color, Look, NAMES, Names, Outline, Shadow};
use crate::transform::{self, IDENTITY, Matrix};

#[cfg(test)]
#[path = "svg_hash_tests.rs"]
mod svg_hash_tests;

/// The color of text that doesn't set one.
const DEFAULT_TEXT: Color = Color::hex(0x1F2328);

/// An ancestor being passed contributes only its inherited text color. Decode
/// exactly the same property as Look without reading its unrelated paint data.
fn passed_text_color(element: &Element) -> Option<Color> {
    element.props.get_symbol(rsui_core::symbol!("text_color")).and_then(Color::from_prop)
}
/// The color of the hint in an empty text input.
const PLACEHOLDER: Color = Color::hex(0x8C959F);

/// The color of what an empty input shows, which is its `placeholder_color` if it has one.
fn placeholder_color(element: &Element) -> Color {
    element
        .props
        .get("placeholder_color")
        .and_then(Color::from_prop)
        .unwrap_or(PLACEHOLDER)
}
/// Whether what an empty input shows instead of a value is slanted, as `::placeholder { font-style: italic }` says.
fn placeholder_italic(element: &Element) -> bool {
    element.props.get("placeholder_italic") == Some(&PropValue::Bool(true))
}

/// The color behind selected text.
const SELECTION: Color = Color::rgba(0x1F, 0x6F, 0xEB, 0x55);
/// The color of the outline around what the keyboard is on.
const FOCUS_RING: Color = Color::hex(0x1F6FEB);
/// Chromium's `-webkit-focus-ring-color` (`LayoutTheme::FocusRingColor` off the Mac), which a page's focus ring is drawn in when it
/// doesn't give an `outline-color` of its own.
const FOCUS_RING_COLOR: Color = Color::hex(0x101010);
/// How wide that outline is, in layout pixels. It sits just outside the element.
const FOCUS_RING_WIDTH: f32 = 2.0;
/// The color of a scrollbar's thumb.
const SCROLL_THUMB_OPACITY: u8 = 0x66;
/// How a tooltip looks, in layout pixels.
const TOOLTIP_BACKGROUND: Color = Color::hex(0x24292F);
const TOOLTIP_TEXT_SIZE: f32 = 12.0;
const TOOLTIP_WIDEST: f32 = 320.0;
const TOOLTIP_PADDING: f32 = 6.0;
const TOOLTIP_GAP: f32 = 20.0;
/// How thick a scrollbar's thumb is, and how far it is from the edge, in layout pixels.
const SCROLL_THUMB_WIDTH: f32 = 6.0;
/// How thick a browser's own scrollbar is, as `scrollbar_style: "classic"` draws it.
const CLASSIC_SCROLLBAR: f32 = 17.0;
const SCROLL_THUMB_INSET: f32 = 2.0;
/// How wide a band along the edge can be used to grab the scrollbar, in layout pixels.
pub const SCROLL_BAR_REACH: f32 = 12.0;
/// The shortest a thumb gets, in layout pixels, so that there's still something to grab.
const SCROLL_THUMB_SHORTEST: f32 = 24.0;

/// Where a scrollbar's thumb goes along its track, and how long it is, for content this long moved
/// this far. `None` if it all fits, so there's nothing to scroll.
pub fn scroll_thumb(track: f32, content: f32, moved: f32) -> Option<(f32, f32)> {
    if content <= track + 0.5 || track <= 0.0 {
        return None;
    }
    let length = (track * track / content).clamp(SCROLL_THUMB_SHORTEST.min(track), track);
    let travel = track - length;
    let furthest = content - track;
    Some(((moved / furthest).clamp(0.0, 1.0) * travel, length))
}

/// How far to move content for a thumb to be at this distance along its track.
pub fn scroll_for_thumb(track: f32, content: f32, thumb_start: f32) -> f32 {
    let Some((_, length)) = scroll_thumb(track, content, 0.0) else {
        return 0.0;
    };
    let travel = track - length;
    if travel <= 0.0 {
        return 0.0;
    }
    (thumb_start / travel).clamp(0.0, 1.0) * (content - track)
}

/// What the person is doing, which is drawn without changing the tree or rendering any
/// component. Elements are recognized by their address, so a node used in several places
/// is highlighted in all of them.
#[derive(Debug, Clone, Copy, Default)]
pub struct Interaction<'a> {
    /// What's under the pointer, which uses its hover background.
    pub hovered: Option<&'a Element>,
    /// How far along that background is, from 0 to 1.
    pub hover_amount: f32,
    /// What the pointer's just left, and how much of its hover background is left.
    pub leaving: Option<(&'a Element, f32)>,
    /// A text input with several lines that's been scrolled without having the keyboard, and how far,
    /// in physical pixels.
    pub resting_area: Option<(&'a Element, f32)>,
    /// What's being pressed, which uses its pressed background.
    pub pressed: Option<&'a Element>,
    /// What gets the focus ring.
    pub ring: Option<&'a Element>,
    /// The text input being edited.
    pub editing: Option<Editing<'a>>,
    /// Text that's been selected without being in an input, and which bytes of it.
    pub text_selection: Option<(&'a Element, (usize, usize))>,
    /// A little text explaining what's under the pointer, and where the pointer is, in physical pixels.
    pub tooltip: Option<(&'a str, f32, f32)>,
    /// What a debugger draws over everything, if one is.
    pub overlay: Option<&'a Overlay>,
    /// A press spreading out from where it landed, as Material's ripple does, on what says it has one (`ripple`).
    pub ripple: Option<Ripple<'a>>,
    /// Where scrollbars only show while something scrolls ([`fade_scrollbars`]), what's scrolling, and how much its
    /// scrollbar shows, from 0 to 1.
    pub scrolling: Option<(&'a Element, f32)>,
}

static FADING_SCROLLBARS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Has rsui's own thin scrollbars show only while what they're on scrolls, and fade once it's stopped, as a phone's do,
/// rather than always. Those whose size or colors are said (CSS's `scrollbar-width` and `scrollbar-color`) and the classic
/// ones with arrows show as they always do.
pub fn fade_scrollbars(on: bool) {
    FADING_SCROLLBARS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether scrollbars fade: see [`fade_scrollbars`].
pub fn scrollbars_fade() -> bool {
    FADING_SCROLLBARS.load(std::sync::atomic::Ordering::Relaxed)
}

/// A press spreading out from where it landed, on an element whose `ripple` prop is its color: what it's on, where it landed in
/// logical pixels from the window's top left, how far it's spread from 0 (a dot) to 1 (all of it), and how strongly it shows,
/// from 0 to 1, which falls once it's let go.
#[derive(Debug, Clone, Copy)]
pub struct Ripple<'a> {
    pub element: &'a Element,
    pub x: f32,
    pub y: f32,
    pub spread: f32,
    pub strength: f32,
}

/// What a debugger draws on top of everything else, in physical pixels: boxes, and a little text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    pub boxes: Vec<OverlayBox>,
    pub texts: Vec<OverlayText>,
}

impl Overlay {
    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty() && self.texts.is_empty()
    }

    pub fn clear(&mut self) {
        self.boxes.clear();
        self.texts.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayBox {
    /// Left, top, width and height.
    pub rect: [f32; 4],
    pub color: Color,
    /// How wide its outline is, or 0 to fill it.
    pub outline: f32,
    /// How round its corners are, in physical pixels.
    pub radius: f32,
}

/// Text in a box of its own color, kept inside the window.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayText {
    pub text: String,
    pub left: f32,
    pub top: f32,
    /// How big it is, and how wide it can get before it wraps, in layout pixels.
    pub size: f32,
    pub widest: f32,
    pub color: Color,
    pub background: Color,
}

/// The state of a text input being edited.
#[derive(Debug, Clone, Copy)]
pub struct Editing<'a> {
    pub element: &'a Element,
    /// What's selected across the text, in physical pixels, from left to right. Text in mixed
    /// directions can have several separate pieces of it selected.
    pub selection: &'a [(f32, f32)],
    /// Where to draw the caret across the text, in physical pixels, unless it's hidden, such as
    /// between blinks.
    pub caret: Option<f32>,
    /// How far the text is moved to the left, to keep the caret in view, in physical pixels.
    pub scroll: f32,
    /// For text that can have several lines, which line the above are on.
    pub lines: Option<EditingLines<'a>>,
}

/// Where things are among the lines of a text input with several.
#[derive(Debug, Clone, Copy)]
pub struct EditingLines<'a> {
    /// Which line each piece of the selection is on.
    pub selection: &'a [u32],
    pub caret: u32,
    /// How far the text is moved up, to keep the caret in view, in physical pixels.
    pub scroll: f32,
}

/// How far across a one-line input its text starts: on the right for right-to-left text, as `line_indent` has it, and otherwise
/// where its `text_align` puts it while it fits, as a browser's `<input style="text-align: center">` has it.
pub fn input_indent(text: &mut TextEngine, element: &Element, shown: &str, style: TextStyle, width: f32) -> f32 {
    // `text-indent` first, as Radix's fields pad their text with it, then where it's aligned in what's left.
    let room = (width - style.indent).max(0.0);
    let indent = text.line_indent(shown, style, room);
    if indent != 0.0 {
        return indent + style.indent;
    }
    let share = match element.props.get("text_align") {
        Some(PropValue::Str(how)) if how.as_str() == "center" => 0.5,
        Some(PropValue::Str(how)) if matches!(how.as_str(), "right" | "end") => 1.0,
        _ => return style.indent,
    };
    style.indent + ((room - text.line_width(shown, style)) * share).max(0.0)
}

/// The value shown by a text input.
pub fn input_value(element: &Element) -> &str {
    match element.props.get_symbol(NAMES.value) {
        Some(PropValue::Str(value)) => value.as_str(),
        _ => "",
    }
}

/// What shows in place of what's in a password input, if this is one.
pub fn password_mask(element: &Element) -> Option<String> {
    static NAME: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("password"));
    (element.props.get_symbol(*NAME) == Some(&PropValue::Bool(true)))
        .then(|| "\u{2022}".repeat(input_value(element).chars().count()))
}

/// Where this byte of what's really in a password input is in what shows instead.
pub fn byte_in_mask(value: &str, byte: usize) -> usize {
    value[..byte].chars().count() * '\u{2022}'.len_utf8()
}

/// The other way around.
pub fn byte_from_mask(value: &str, shown: usize) -> usize {
    value
        .char_indices()
        .nth(shown / '\u{2022}'.len_utf8())
        .map_or(value.len(), |(byte, _)| byte)
}

/// The area inside an element's padding, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContentBox {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

/// Where an element's text starts, and how wide it can be, given where the element is in layout
/// pixels and how many physical pixels each of those is.
pub fn content_box(element: &Element, rect: Rect, scale: f32) -> ContentBox {
    content_box_of(&Look::read(&element.props, &NAMES), rect, scale)
}

#[inline(always)]
fn content_box_of(look: &Look, rect: Rect, scale: f32) -> ContentBox {
    ContentBox {
        left: rect.x * scale + look.padding_left * scale,
        top: snap(rect.y * scale + look.padding_top * scale),
        width: ((rect.width - look.padding_left - look.padding_right) * scale).max(0.0),
        height: ((rect.height - look.padding_top - look.padding_bottom) * scale).max(0.0),
    }
}

/// One rectangle or glyph, exactly as the GPU reads it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Quad {
    /// Left, top, width and height, in physical pixels.
    pub rect: [f32; 4],
    /// For a glyph: left, top, width and height in the atlas. For a rectangle, all zero, except
    /// that the first number is its outline's width in sixteenths of a pixel, if it's only an
    /// outline.
    pub glyph: [u16; 4],
    /// Premultiplied red, green, blue and alpha.
    pub color: [u8; 4],
    /// How round the corners are, in physical pixels.
    pub radius: f32,
}

/// An area of the window, in whole physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PixelRect {
    /// As far as anything could be, for what can't tell yet what would be seen.
    const EVERYWHERE: Self = Self {
        left: i32::MIN / 2,
        top: i32::MIN / 2,
        right: i32::MAX / 2,
        bottom: i32::MAX / 2,
    };

    fn intersect(self, other: Self) -> Self {
        Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        }
    }

    #[inline(always)]
    fn overlaps(self, left: f32, top: f32, right: f32, bottom: f32) -> bool {
        (self.left as f32) < right
            && (self.top as f32) < bottom
            && left < self.right as f32
            && top < self.bottom as f32
    }
}

/// A run of quads drawn inside one area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Batch {
    pub first: u32,
    pub count: u32,
    pub clip: PixelRect,
    /// The image these quads show, or 0 if they're rectangles and glyphs.
    pub image: u32,
    /// Which of the scene's transforms these are drawn with, counting from 1, or 0 for none, which is nearly always.
    pub transform: u16,
    /// Which of the scene's filters their colors go through, counting from 1, or 0 for none, which is nearly always.
    pub filter: u16,
    /// Which of the scene's round clips cuts them off, counting from 1, or 0 for none, which is nearly always.
    pub round: u16,
}

/// Round corners what's drawn is cut off at, as CSS's `overflow: hidden` with `border-radius` has it, or a box with round
/// corners what's drawn is kept out of, as a shadow is kept out from under what it's behind. A batch says which it's drawn with,
/// and each says the one it's in, so that what's in two is cut off by both.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoundClip {
    /// Its left, top, right and bottom edges, in physical pixels, as they are before the transform it was made in.
    pub edges: [f32; 4],
    /// How far across and down each corner's curve goes, in physical pixels, from the top left going round.
    pub corners: [[f32; 2]; 4],
    /// Whether what's drawn shows only outside it, rather than only inside.
    pub outside: bool,
    /// Which of the scene's transforms it was made in, counting from 1, or 0 for none.
    pub transform: u16,
    /// The one it's in, counting from 1, or 0 for none.
    pub outer: u16,
}

/// What CSS's `filter` does to colors: red, green and blue each from those, as what each is multiplied by and what's added, and
/// what opacity is multiplied by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Filter {
    pub rows: [f32; 12],
    pub opacity: f32,
}

impl Filter {
    const NONE: Self = Self {
        rows: [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        opacity: 1.0,
    };

    /// What `inner` and then this do, together.
    fn after(self, inner: Self) -> Self {
        let (a, b) = (self.rows, inner.rows);
        let mut rows = [0f32; 12];
        for row in 0..3 {
            for column in 0..4 {
                let mut sum = if column == 3 { a[row * 4 + 3] } else { 0.0 };
                for k in 0..3 {
                    sum += a[row * 4 + k] * b[k * 4 + column];
                }
                rows[row * 4 + column] = sum;
            }
        }
        Self {
            rows,
            opacity: self.opacity * inner.opacity,
        }
    }

    /// What a list of CSS filters does to colors, as `grayscale(1) brightness(1.2)` says, each after the one before it. What
    /// isn't about colors, as `blur()` is, is left out.
    pub fn read(text: &str) -> Option<Self> {
        let mut out = Self::NONE;
        let mut any = false;
        let mut rest = text.trim();
        while let Some(open) = rest.find('(') {
            let Some(close) = rest[open..].find(')').map(|at| at + open) else {
                break;
            };
            let name = rest[..open].trim().to_ascii_lowercase();
            let inside = rest[open + 1..close].trim();
            rest = &rest[close + 1..];
            let amount = |usual: f32| -> f32 {
                if inside.is_empty() {
                    return usual;
                }
                match inside.strip_suffix('%') {
                    Some(percent) => percent
                        .trim()
                        .parse::<f32>()
                        .map_or(usual, |percent| percent / 100.0),
                    None => inside.parse::<f32>().unwrap_or(usual),
                }
            };
            let matrix = |m: [f32; 9]| Self {
                rows: [
                    m[0], m[1], m[2], 0.0, m[3], m[4], m[5], 0.0, m[6], m[7], m[8], 0.0,
                ],
                opacity: 1.0,
            };
            let step = match name.as_str() {
                "grayscale" => {
                    let a = 1.0 - amount(1.0).clamp(0.0, 1.0);
                    matrix([
                        0.2126 + 0.7874 * a,
                        0.7152 - 0.7152 * a,
                        0.0722 - 0.0722 * a,
                        0.2126 - 0.2126 * a,
                        0.7152 + 0.2848 * a,
                        0.0722 - 0.0722 * a,
                        0.2126 - 0.2126 * a,
                        0.7152 - 0.7152 * a,
                        0.0722 + 0.9278 * a,
                    ])
                }
                "sepia" => {
                    let a = 1.0 - amount(1.0).clamp(0.0, 1.0);
                    matrix([
                        0.393 + 0.607 * a,
                        0.769 - 0.769 * a,
                        0.189 - 0.189 * a,
                        0.349 - 0.349 * a,
                        0.686 + 0.314 * a,
                        0.168 - 0.168 * a,
                        0.272 - 0.272 * a,
                        0.534 - 0.534 * a,
                        0.131 + 0.869 * a,
                    ])
                }
                "saturate" => {
                    let s = amount(1.0).max(0.0);
                    matrix([
                        0.213 + 0.787 * s,
                        0.715 - 0.715 * s,
                        0.072 - 0.072 * s,
                        0.213 - 0.213 * s,
                        0.715 + 0.285 * s,
                        0.072 - 0.072 * s,
                        0.213 - 0.213 * s,
                        0.715 - 0.715 * s,
                        0.072 + 0.928 * s,
                    ])
                }
                "hue-rotate" => {
                    let degrees = crate::transform::parse_angle(inside).unwrap_or(0.0);
                    let (sin, cos) = degrees.to_radians().sin_cos();
                    matrix([
                        0.213 + cos * 0.787 - sin * 0.213,
                        0.715 - cos * 0.715 - sin * 0.715,
                        0.072 - cos * 0.072 + sin * 0.928,
                        0.213 - cos * 0.213 + sin * 0.143,
                        0.715 + cos * 0.285 + sin * 0.140,
                        0.072 - cos * 0.072 - sin * 0.283,
                        0.213 - cos * 0.213 - sin * 0.787,
                        0.715 - cos * 0.715 + sin * 0.715,
                        0.072 + cos * 0.928 + sin * 0.072,
                    ])
                }
                "invert" => {
                    let a = amount(1.0).clamp(0.0, 1.0);
                    let k = 1.0 - 2.0 * a;
                    Self {
                        rows: [k, 0.0, 0.0, a, 0.0, k, 0.0, a, 0.0, 0.0, k, a],
                        opacity: 1.0,
                    }
                }
                "brightness" => {
                    let b = amount(1.0).max(0.0);
                    matrix([b, 0.0, 0.0, 0.0, b, 0.0, 0.0, 0.0, b])
                }
                "contrast" => {
                    let c = amount(1.0).max(0.0);
                    let add = (1.0 - c) / 2.0;
                    Self {
                        rows: [c, 0.0, 0.0, add, 0.0, c, 0.0, add, 0.0, 0.0, c, add],
                        opacity: 1.0,
                    }
                }
                "opacity" => Self {
                    rows: Self::NONE.rows,
                    opacity: amount(1.0).clamp(0.0, 1.0),
                },
                _ => continue,
            };
            out = step.after(out);
            any = true;
        }
        any.then_some(out)
    }
}

/// Whether any of these quads covers part of one before it, apart from glyphs over glyphs, whose edges touching shows no more
/// faded one part at a time than as a whole. Worked out from the area around those before it, which can say they overlap when they
/// don't, which only costs drawing them as one when it isn't needed.
fn parts_overlap(quads: &[Quad]) -> bool {
    let mut all: Option<[f32; 4]> = None;
    let mut shapes: Option<[f32; 4]> = None;
    let meets = |area: Option<[f32; 4]>, [x, y, w, h]: [f32; 4]| {
        area.is_some_and(|[left, top, right, bottom]| x < right && x + w > left && y < bottom && y + h > top)
    };
    let grow = |area: &mut Option<[f32; 4]>, [x, y, w, h]: [f32; 4]| {
        *area = Some(area.map_or([x, y, x + w, y + h], |[left, top, right, bottom]| {
            [left.min(x), top.min(y), right.max(x + w), bottom.max(y + h)]
        }));
    };
    for quad in quads {
        if quad.rect[2] <= 0.0 || quad.rect[3] <= 0.0 || quad.color[3] == 0 {
            continue;
        }
        // A glyph says where it is in the atlas, and a rectangle doesn't.
        let glyph = quad.glyph[2] > 0 && quad.glyph[3] > 0 && quad.glyph[2] != u16::MAX - 1;
        if meets(if glyph { shapes } else { all }, quad.rect) {
            return true;
        }
        grow(&mut all, quad.rect);
        if !glyph {
            grow(&mut shapes, quad.rect);
        }
    }
    false
}

/// Everything to draw in a frame, in order from back to front.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub quads: Vec<Quad>,
    /// The gamma and contrast that glyphs' coverage goes through for their color, as a browser has it, or 0 for neither, which
    /// leaves it as it is.
    pub text_look: [f32; 2],
    pub batches: Vec<Batch>,
    /// New glyph pixels that the atlas texture needs.
    pub uploads: Vec<Upload>,
    /// How many pixels wide and high the atlas texture needs to be.
    pub atlas_size: u32,
    /// Goes up when the atlas starts over, after which a place in it can hold a different glyph,
    /// so looking the same doesn't mean a quad will look the same.
    pub atlas_generation: u32,
    /// Images to put on the GPU.
    pub image_uploads: Vec<ImageUpload>,
    /// Images the GPU can let go of.
    pub dropped_images: Vec<u32>,
    /// How what's turned or resized is, in physical pixels. A batch says which it's using.
    pub transforms: Vec<Matrix>,
    /// What colors go through, as CSS's `filter` says. A batch says which it's using.
    pub filters: Vec<Filter>,
    /// Round corners what's drawn is cut off at. A batch says which it's using.
    pub round_clips: Vec<RoundClip>,
    /// The color of the window itself, if it says it has one that can't be seen through. What it hasn't covered has to be that,
    /// as there's nothing behind a window to show: that's the corners, if it has a radius.
    pub window_color: Option<Color>,
    /// What's faded as a whole, as CSS's `opacity` has it, in the order their batches are in, none inside another.
    pub layers: Vec<Layer>,
}

/// A batch that samples what's already drawn instead of an uploaded image. Its quad carries the blur in sixteenths of a pixel.
pub(crate) const BACKDROP_IMAGE: u32 = u32::MAX;

/// Something see-through whose parts overlap, as a disabled button's text over its background does: its batches, `first..end`,
/// are drawn by themselves, then put over what's behind them at `opacity`, so that what's under one part doesn't show through
/// it, as a browser has it. Fading each part by itself made the button's dark background show through its text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    pub first: u32,
    pub end: u32,
    pub opacity: f32,
    /// Where on the screen what it draws is, which is all that's drawn again when it's put down.
    pub bounds: PixelRect,
}

impl Scene {
    /// Whether anything at all is drawn at this point, in physical pixels, as far as the rectangles of what's drawn say: a
    /// rectangle with any color, an image, or a glyph, with a little room around each glyph so that the gaps in text count.
    /// What's turned or resized by a transform counts all over where it's cut to, and round corners count as square. It's for
    /// a transparent window to tell whether the pointer is over something, or should go through to what's behind.
    pub fn draws_at(&self, x: f32, y: f32) -> bool {
        const AROUND_GLYPHS: f32 = 2.0;
        let (px, py) = (x.floor() as i32, y.floor() as i32);
        self.batches.iter().any(|batch| {
            let clip = batch.clip;
            if px < clip.left || px >= clip.right || py < clip.top || py >= clip.bottom {
                return false;
            }
            // A shadow isn't under what it's around.
            let round = usize::from(batch.round).checked_sub(1).and_then(|at| self.round_clips.get(at));
            if let Some(&RoundClip { edges: [left, top, right, bottom], outside: true, .. }) = round
                && x >= left && x < right && y >= top && y < bottom
            {
                return false;
            }
            let first = batch.first as usize;
            let quads = self.quads.get(first..first + batch.count as usize).unwrap_or_default();
            if batch.transform != 0 {
                return !quads.is_empty();
            }
            quads.iter().any(|quad| {
                let glyph = quad.glyph[2] > 0 && quad.glyph[3] > 0 && quad.glyph[2] != u16::MAX - 1;
                if batch.image == 0 && quad.color[3] == 0 {
                    return false;
                }
                let room = if glyph && batch.image == 0 { AROUND_GLYPHS } else { 0.0 };
                let [left, top, width, height] = quad.rect;
                x >= left - room && x < left + width + room && y >= top - room && y < top + height + room
            })
        })
    }
}

/// Whether leaving out what can't be seen is off, for comparing the two in `RSUI_CULL_CHECK`.
#[cfg(feature = "render-diagnostics")]
static CULL_OFF: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Stable for this painter's shape atlas; contains no shape text or tree data.
enum SvgHash {
    Legacy,
    Fast(foldhash::fast::RandomState),
}

impl Default for SvgHash {
    fn default() -> Self {
        static FAST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *FAST.get_or_init(|| std::env::var_os("RSUI_FAST_SVG_HASH").is_some_and(|value| value == "1")) {
            Self::Fast(foldhash::fast::RandomState::default())
        } else { Self::Legacy }
    }
}

impl SvgHash {
    fn key(&self, element: &Element, layers: &str, width: f32, height: f32, current: Color) -> u64 {
        match self {
            Self::Legacy => svg_hash_inputs(element, layers, width, height, current, std::collections::hash_map::DefaultHasher::new()),
            Self::Fast(seed) => svg_hash_inputs(element, layers, width, height, current, seed.build_hasher()),
        }
    }
}

/// One input path for both algorithms: changing the hasher cannot omit a field.
fn svg_hash_inputs<H: Hasher>(element: &Element, layers: &str, width: f32, height: f32, current: Color, mut hasher: H) -> u64 {
    layers.hash(&mut hasher);
    (width as u32, height as u32).hash(&mut hasher);
    [current.r, current.g, current.b, current.a].hash(&mut hasher);
    if let Some(PropValue::Str(view)) = element.props.get("view_box") {
        view.hash(&mut hasher);
    }
    (element.props.get("stretch") == Some(&PropValue::Bool(true))).hash(&mut hasher);
    hasher.finish()
}

/// Turns trees into scenes, keeping the glyphs it has drawn so far.
#[derive(Default)]
pub struct Painter {
    #[cfg(feature = "render-diagnostics")]
    disable_text_replay: bool,
    #[cfg(feature = "mutation-pilot")]
    pub(crate) pilot: PilotPaint,
    #[cfg(all(test, feature = "render-diagnostics"))]
    retry_for_test: bool,
    #[cfg(feature = "render-diagnostics")]
    work: crate::diagnostics::SceneWork,
    atlas: GlyphAtlas,
    rasterizer: Rasterizer,
    pub(crate) scene: Scene,
    /// Kept between frames, so that a frame doesn't allocate.
    glyphs: TextLayout,
    clips: Vec<PixelRect>,
    images: Images,
    svg_hash: SvgHash,
    fades: Vec<Fade>,
    turning: Vec<Turning>,
    /// Whether what was last worked out had transforms, as the scene's lists go to the GPU's side once it's been drawn.
    had_transforms: bool,
    /// What was last worked out, if it showed a caret and was asked to be kept, so that it can blink without the rest
    /// being worked out again. The scene's own lists go to the GPU's side once it's drawn, so this keeps them.
    with_caret: Option<Box<CaretScene>>,
    /// Whether the next scene is kept that way.
    keep_next: bool,
    text_runs: TextRuns,
    paragraph: ParagraphMemo,
    /// Whether the system's glyphs are gray, rather than ClearType's, as they have to be on what's seen through.
    gray_text: bool,
}

// Reuse the layout already held by TextEngine's scratch slot. Only the exact
// input is copied; no glyph/layout arrays or tree nodes are retained here.
const PARAGRAPH_KEY_BYTES: usize = 32 * 1024;

#[derive(Default)]
struct ParagraphMemo {
    source: RichText,
    width: f32,
    along: Option<f32>,
    stamp: Option<(u64, u64, usize)>,
    seen: u32,
    previous: u32,
}

impl ParagraphMemo {
    fn clear_key(&mut self) {
        self.stamp = None;
        self.source = RichText::default();
    }

    fn end_pass(&mut self) {
        self.previous = self.seen;
        if self.seen != 1 {
            self.clear_key();
        }
    }

    fn matches(
        &self,
        stamp: Option<(u64, u64, usize)>,
        text: &RichText,
        width: f32,
        along: Option<f32>,
    ) -> bool {
        stamp.is_some()
            && self.stamp == stamp
            && self.width.to_bits() == width.to_bits()
            && self.along == along
            && self.source == *text
    }

    fn remember(&mut self, text: &RichText, width: f32, along: Option<f32>) -> bool {
        let run_bytes = text
            .runs
            .len()
            .saturating_mul(size_of::<rsui_text::TextRun>());
        // A first paint (including idle recovery) does not need a retained key.
        // Enroll only after a preceding single-paragraph paint.
        if self.previous != 1
            || self.seen != 1
            || text.text.len() < 128
            || text.text.len().saturating_add(run_bytes) > PARAGRAPH_KEY_BYTES
            || !equals_itself(text)
            || width.is_nan()
            || !equals_itself(&along)
        {
            self.clear_key();
            return false;
        }
        let text_capacity = self.source.text.capacity().max(text.text.len());
        let run_capacity = self.source.runs.capacity().max(text.runs.len());
        if text_capacity
            .saturating_add(run_capacity.saturating_mul(size_of::<rsui_text::TextRun>()))
            > PARAGRAPH_KEY_BYTES
        {
            self.clear_key();
        }
        // Exact reservations keep combined retained capacity under the cap,
        // including alternating large text and many short styled runs.
        if self.source.text.capacity() < text.text.len() {
            self.source.text = String::with_capacity(text.text.len());
        }
        if self.source.runs.capacity() < text.runs.len() {
            self.source.runs = Vec::with_capacity(text.runs.len());
        }
        self.source.text.clear();
        self.source.text.push_str(&text.text);
        self.source.runs.clear();
        self.source.runs.extend_from_slice(&text.runs);
        self.source.inline_top = text.inline_top;
        self.width = width;
        self.along = along;
        true
    }
}

/// The glyphs of text drawn lately, by everything that decides where they go and what they look like, so that text that
/// hasn't changed since the frame before isn't laid out and placed again. That's most of the text in most frames.
#[derive(Default)]
struct TextRuns {
    runs: HashMap<TextRunKey, TextRun, foldhash::fast::RandomState>,
    frame: u32,
    /// Lists from runs that have gone, to be filled again rather than given back and asked for.
    spare: Vec<Vec<Quad>>,
    /// Requested quad capacity in `spare`, kept bounded during active editing.
    spare_bytes: usize,
    /// Complete movable runs have a separate hard capacity budget.
    translated_bytes: usize,
}

struct TextRun {
    quads: Vec<Quad>,
    origin: [f32; 2],
    /// A proof from the completed layout, not an assumption about wrapping/fonts.
    baseline: Option<f32>,
    /// The last frame it was drawn in.
    frame: u32,
}

fn moved_quads(quads: &[Quad], by: [f32; 2]) -> impl Iterator<Item = Quad> + '_ {
    quads.iter().map(move |quad| {
        let mut quad = *quad;
        quad.rect[0] += by[0];
        quad.rect[1] += by[1];
        quad
    })
}

fn quad_in_clip(quad: &Quad, clip: PixelRect) -> bool {
    let [left, top, width, height] = quad.rect;
    clip.overlaps(left, top, left + width, top + height)
}

/// Whether `value` is equal to itself, which a float that's NaN, or anything holding one, isn't.
#[allow(clippy::eq_op)]
fn equals_itself<T: PartialEq>(value: &T) -> bool {
    value == value
}

/// Everything that decides a piece of text's glyphs, in the usual case of text without shadows, gradients or a limit on
/// its lines. The same of all of these is the same glyphs, as working them out uses nothing else.
#[derive(PartialEq)]
struct TextRunKey {
    name: rsui_core::Text,
    style: TextStyle,
    keeps_words: bool,
    /// Exact origin/width for ordinary runs; fractional x and width for movable
    /// runs. Their vertical baseline proof is validated on lookup instead.
    place: [u32; 3],
    clip: PixelRect,
    translated: bool,
    color: Color,
    rounding: u32,
    zoom: Option<[u32; 3]>,
    /// Which text engine, and how many fonts it has open, as opening another can change which one a character comes
    /// from.
    engine: u64,
    fonts: usize,
    atlas_generation: u32,
}

// Keys whose style isn't equal to itself, which takes a size that isn't a number, are never kept.
impl Eq for TextRunKey {}

impl Hash for TextRunKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.as_str().hash(state);
        self.place.hash(state);
        self.style.size.to_bits().hash(state);
        self.clip.top.hash(state);
    }
}

/// How many spare lists are kept at most.
const SPARE_RUNS: usize = 64;
// Reuse a few larger runs during active edits without retaining 64 large paragraphs.
const SPARE_BYTES: usize = 256 * 1024;
const IDLE_SPARE_RUN_BYTES: usize = 4096;
const TRANSLATED_RUN_BYTES: usize = 512 * 1024;

/// Only reclaim material slack, leaving room for ordinary interaction after idle.
fn trim_slack<T>(list: &mut Vec<T>) {
    let floor = 4096 / size_of::<T>().max(1);
    let target = list.len().saturating_mul(2).max(floor);
    if list.capacity() > target.saturating_mul(2) {
        list.shrink_to(target);
    }
}

impl TextRuns {
    fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// The glyphs for this, if they were worked out in this frame or the one before.
    fn find(&mut self, key: &TextRunKey, origin: [f32; 2]) -> Option<(&[Quad], [f32; 2])> {
        let frame = self.frame;
        let run = self.runs.get_mut(key)?;
        let mut by = [origin[0] - run.origin[0], origin[1] - run.origin[1]];
        if key.translated {
            if let Some(baseline) = run.baseline {
                by[1] = (origin[1] + baseline + 0.5).floor()
                    - (run.origin[1] + baseline + 0.5).floor();
            } else if by[1].fract() != 0.0 {
                // Different lines can round differently at a fractional origin.
                return None;
            }
        }
        run.frame = frame;
        Some((&run.quads, by))
    }

    fn keep(&mut self, key: TextRunKey, quads: &[Quad], origin: [f32; 2], baseline: Option<f32>) {
        let mut kept = self.spare.pop().unwrap_or_default();
        self.spare_bytes -= kept.capacity() * size_of::<Quad>();
        kept.extend_from_slice(quads);
        let frame = self.frame;
        if let Some(old) = self.runs.remove(&key) {
            if key.translated {
                self.translated_bytes -= old.quads.capacity() * size_of::<Quad>();
            }
            self.spare_list(old.quads);
        }
        if key.translated {
            let bytes = kept.capacity() * size_of::<Quad>();
            if bytes > TRANSLATED_RUN_BYTES - self.translated_bytes {
                self.spare_list(kept);
                return;
            }
            self.translated_bytes += bytes;
        }
        if let Some(old) = self.runs.insert(key, TextRun {
            quads: kept,
            frame,
            origin,
            baseline,
        }) {
            self.spare_list(old.quads);
        }
    }

    fn spare_list(&mut self, mut list: Vec<Quad>) {
        let bytes = list.capacity() * size_of::<Quad>();
        if self.spare.len() < SPARE_RUNS && bytes <= SPARE_BYTES - self.spare_bytes {
            list.clear();
            self.spare.push(list);
            self.spare_bytes += bytes;
        }
    }

    fn trim_spares(&mut self) {
        self.spare.retain(|list| {
            let bytes = list.capacity() * size_of::<Quad>();
            if bytes > IDLE_SPARE_RUN_BYTES {
                self.spare_bytes -= bytes;
                false
            } else {
                true
            }
        });
    }

    /// Lets go of what wasn't drawn in this frame.
    fn end_frame(&mut self) {
        let frame = self.frame;
        let spare = &mut self.spare;
        let spare_bytes = &mut self.spare_bytes;
        let translated_bytes = &mut self.translated_bytes;
        self.runs.retain(|key, run| {
            if run.frame == frame {
                return true;
            }
            let bytes = run.quads.capacity() * size_of::<Quad>();
            if key.translated {
                *translated_bytes -= bytes;
            }
            if spare.len() < SPARE_RUNS && bytes <= SPARE_BYTES - *spare_bytes {
                let mut list = std::mem::take(&mut run.quads);
                list.clear();
                spare.push(list);
                *spare_bytes += bytes;
            }
            false
        });
    }
}

/// A scene with the caret showing, and where the caret is in it.
#[derive(Debug, Default)]
struct CaretScene {
    quads: Vec<Quad>,
    batches: Vec<Batch>,
    transforms: Vec<Matrix>,
    filters: Vec<Filter>,
    round_clips: Vec<RoundClip>,
    /// The caret's place among the quads, and the place among the batches of the one it's in.
    at: usize,
    batch_at: usize,
}

/// Something that's turned or resized being drawn, with what to go back to once it's done.
#[derive(Debug, Clone, Copy)]
struct Turning {
    element: usize,
    /// Whether it's being moved, which a browser does by stretching what it drew, until it's done.
    moving: bool,
    before: u16,
    matrix_before: Matrix,
    outer_clip: PixelRect,
}

/// Something see-through being drawn, whose quads are faded once they're all known.
#[derive(Debug, Clone, Copy)]
struct Fade {
    element: usize,
    first_quad: usize,
    opacity: f32,
}

impl Painter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Disable only translated short-text command reuse for paired diagnostics.
    /// Traversal, culling, shaping and resource caches otherwise remain the same.
    #[cfg(feature = "render-diagnostics")]
    #[doc(hidden)]
    pub fn set_text_command_reuse_for_diagnostics(&mut self, on: bool) {
        self.disable_text_replay = !on;
        self.text_runs = TextRuns::default();
    }

    /// Export pending image retirements without painting or accepting new loads.
    /// Cancel their undrawn uploads before the renderer releases the resources.
    pub fn flush_dropped_images(&mut self) {
        self.images.drain_dropped(
            &mut self.scene.image_uploads,
            &mut self.scene.dropped_images,
        );
    }

    /// Release unused images and high-water slack at the existing idle boundary.
    /// Current commands, pending uploads, live text runs and the caret remain usable.
    pub fn forget_caches(&mut self) {
        #[cfg(feature = "mutation-pilot")]
        {
            // Pending history must keep its lengths; idle trims only unused capacity.
            trim_slack(&mut self.pilot.history.quads);
            trim_slack(&mut self.pilot.history.batches);
            trim_slack(&mut self.pilot.history.transforms);
            trim_slack(&mut self.pilot.history.filters);
            trim_slack(&mut self.pilot.history.round_clips);
        }
        self.text_runs.trim_spares();
        self.paragraph = ParagraphMemo::default();
        self.images.let_go_of_unused();
        self.images.drain_dropped(
            &mut self.scene.image_uploads,
            &mut self.scene.dropped_images,
        );
        let scene = &mut self.scene;
        trim_slack(&mut scene.quads);
        trim_slack(&mut scene.batches);
        trim_slack(&mut scene.transforms);
        trim_slack(&mut scene.filters);
        trim_slack(&mut scene.round_clips);
        trim_slack(&mut scene.uploads);
        trim_slack(&mut scene.image_uploads);
        trim_slack(&mut scene.dropped_images);
        if let Some(kept) = &mut self.with_caret {
            trim_slack(&mut kept.quads);
            trim_slack(&mut kept.batches);
            trim_slack(&mut kept.transforms);
            trim_slack(&mut kept.filters);
            trim_slack(&mut kept.round_clips);
        }
        // Only scratch: every layout overwrites these before reading them.
        self.glyphs.glyphs.clear();
        self.glyphs.lines.clear();
        trim_slack(&mut self.glyphs.glyphs);
        trim_slack(&mut self.glyphs.lines);
        trim_slack(&mut self.clips);
        trim_slack(&mut self.fades);
        trim_slack(&mut self.turning);
        let runs = &mut self.text_runs.runs;
        let target = runs.len().saturating_mul(2).max(64);
        if runs.capacity() > target.saturating_mul(2) {
            runs.shrink_to(target);
        }
        for run in runs.values_mut() {
            trim_slack(&mut run.quads);
        }
        self.text_runs.translated_bytes = runs.iter()
            .filter(|(key, _)| key.translated)
            .map(|(_, run)| run.quads.capacity() * size_of::<Quad>())
            .sum();
    }

    /// Whether anything in the last thing drawn was turned or resized, so that what's under the pointer needs more working out.
    pub fn has_transforms(&self) -> bool {
        self.had_transforms
    }

    /// Has glyphs drawn the way the system draws them, as a browser does, or the way rsui does. Glyphs drawn already are
    /// drawn again. It stays the way rsui does if the system can't.
    pub fn draw_glyphs_as_the_system_does(&mut self, on: bool) {
        #[cfg(feature = "mutation-pilot")]
        self.pilot.invalidate();
        let system = match (on, self.gray_text) {
            (false, _) => None,
            (true, false) => crate::dwrite::system_glyphs(),
            (true, true) => crate::dwrite::gray_glyphs(),
        };
        self.rasterizer.set_system(system);
        self.atlas.start_over();
    }

    /// Whether glyphs are drawn the way the system draws them.
    pub fn draws_glyphs_as_the_system_does(&self) -> bool {
        self.rasterizer.has_system()
    }

    /// Has the system's glyphs drawn gray, or as the system's set to draw them, which is usually ClearType. ClearType
    /// covers each part of a pixel apart, which only works over something opaque, so a window that's seen through has
    /// its text gray, as a browser does on a see-through page. Glyphs drawn already are drawn again.
    pub fn draw_text_gray(&mut self, gray: bool) {
        if gray == self.gray_text {
            return;
        }
        self.gray_text = gray;
        if self.rasterizer.has_system() {
            self.draw_glyphs_as_the_system_does(true);
        }
    }

    /// Whether the system's glyphs are drawn gray. See [`draw_text_gray`](Self::draw_text_gray).
    pub fn draws_text_gray(&self) -> bool {
        self.gray_text
    }

    /// The images in use, which outlast anything to do with the GPU.
    pub fn images(&mut self) -> &mut Images {
        &mut self.images
    }

    /// Keeps a copy of the next scene worked out, if it has a caret, for [`show_caret`](Self::show_caret). Copying
    /// every scene with a caret would slow down typing for the sake of blinking, so it's only done when asked.
    pub fn keep_next_for_blinking(&mut self) {
        self.keep_next = true;
    }

    /// The scene last worked out, or last given by [`show_caret`](Self::show_caret).
    pub fn last_scene(&mut self) -> &mut Scene {
        #[cfg(feature = "mutation-pilot")]
        self.pilot.invalidate();
        &mut self.scene
    }

    /// Retire resource IDs without exporting authoritative drawing commands.
    pub fn release_unused_images_to(&mut self, renderer: &mut crate::Renderer) {
        self.flush_dropped_images();
        renderer.release_images(&mut self.scene.dropped_images);
    }

    /// Whether [`show_caret`](Self::show_caret) has a scene to give.
    pub fn can_show_caret(&self) -> bool {
        self.with_caret.is_some()
    }

    /// The scene last worked out, with the caret shown or hidden, if that's all that's different about it and it
    /// had the caret showing when it was worked out and kept. It's then what working it out again would give, without doing
    /// so, which is all a blink needs. Otherwise there's nothing, and it has to be worked out again.
    pub fn show_caret(&mut self, show: bool) -> Option<&mut Scene> {
        #[cfg(feature = "mutation-pilot")]
        self.pilot.invalidate();
        self.show_caret_internal(show)
    }

    pub(crate) fn show_caret_internal(&mut self, show: bool) -> Option<&mut Scene> {
        let kept = self.with_caret.as_deref()?;
        let scene = &mut self.scene;
        scene.quads.clone_from(&kept.quads);
        scene.batches.clone_from(&kept.batches);
        scene.transforms.clone_from(&kept.transforms);
        scene.filters.clone_from(&kept.filters);
        scene.round_clips.clone_from(&kept.round_clips);
        if !show {
            // The batches after the caret's start one earlier, and its own has one fewer. One that had only the caret
            // isn't there, as there'd be nothing in it.
            scene.quads.remove(kept.at);
            let batches = &mut scene.batches;
            let later = if batches[kept.batch_at].count == 1 {
                batches.remove(kept.batch_at);
                kept.batch_at
            } else {
                batches[kept.batch_at].count -= 1;
                kept.batch_at + 1
            };
            for batch in &mut batches[later..] {
                batch.first -= 1;
            }
        }
        #[cfg(feature = "render-diagnostics")]
        {
            self.work = crate::diagnostics::SceneWork {
                serial: self.work.serial.saturating_add(1),
                path: crate::diagnostics::ScenePath::CaretReuse,
                caret_copy_bytes: crate::diagnostics::CommandStorage::of(
                    &kept.quads,
                    &kept.batches,
                    &kept.transforms,
                    &kept.filters,
                )
                .len_bytes(),
                atlas_before: self.atlas_observation(),
                ..Default::default()
            };
            self.finish_work();
        }
        Some(&mut self.scene)
    }

    /// Everything on the GPU is gone, as there's a new renderer. This keeps what's still good.
    pub fn renderer_replaced(&mut self) {
        let images = std::mem::take(&mut self.images);
        let svg_hash = std::mem::replace(&mut self.svg_hash, SvgHash::Legacy);
        let gray_text = self.gray_text;
        *self = Self::new();
        self.images = images;
        self.svg_hash = svg_hash;
        self.gray_text = gray_text;
        self.images.gpu_replaced();
    }

    /// Works out what to draw. `scale` is the number of physical pixels per layout pixel,
    /// and `size` is the window's size in physical pixels. `layout` must be up to date
    /// with `root`, in layout pixels.
    pub fn paint(
        &mut self,
        root: &Node,
        layout: &Layout,
        text: &mut TextEngine,
        scale: f32,
        size: (u32, u32),
    ) -> &mut Scene {
        self.paint_with(root, layout, text, scale, size, Interaction::default())
    }

    /// Like [`paint`](Self::paint), showing what the person is doing.
    pub fn paint_with(
        &mut self,
        root: &Node,
        layout: &Layout,
        text: &mut TextEngine,
        scale: f32,
        size: (u32, u32),
        interaction: Interaction<'_>,
    ) -> &mut Scene {
        #[cfg(feature = "render-diagnostics")]
        if std::env::var_os("RSUI_CULL_CHECK").is_some() {
            self.check_culling(root, layout, text, scale, size, interaction);
            return self.last_scene();
        }
        self.paint_traversal::<true>(root, layout, text, scale, size, interaction);
        self.last_scene()
    }

    /// For `RSUI_CULL_CHECK`: works the scene out with nothing left out for not being seen, then as usual, and says on stderr if
    /// what they'd draw differs. What's in an area that's empty draws nothing, and runs of quads cut off the same way draw as
    /// one, so those don't count as differences. The usual one is what's kept.
    #[cfg(feature = "render-diagnostics")]
    fn check_culling(
        &mut self,
        root: &Node,
        layout: &Layout,
        text: &mut TextEngine,
        scale: f32,
        size: (u32, u32),
        interaction: Interaction<'_>,
    ) {
        use std::sync::atomic::Ordering::Relaxed;
        static CHECKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static DIFFERENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        fn drawn(scene: &Scene) -> (Vec<Quad>, Vec<(PixelRect, u32, u16, u16)>, Vec<usize>) {
            let mut quads = Vec::new();
            let mut runs: Vec<(PixelRect, u32, u16, u16)> = Vec::new();
            let mut ends = Vec::new();
            for batch in &scene.batches {
                let clip = batch.clip;
                if clip.left >= clip.right || clip.top >= clip.bottom {
                    continue;
                }
                let run = (clip, batch.image, batch.transform, batch.filter);
                if runs.last() != Some(&run) {
                    runs.push(run);
                    ends.push(quads.len());
                }
                quads.extend_from_slice(&scene.quads[batch.first as usize..(batch.first + batch.count) as usize]);
                *ends.last_mut().expect("just pushed") = quads.len();
            }
            (quads, runs, ends)
        }
        let keep = self.keep_next;
        // Twice, so that what the first one starts, as an image being read, is there for both that are compared.
        CULL_OFF.store(true, Relaxed);
        self.paint_traversal::<true>(root, layout, text, scale, size, interaction);
        self.keep_next = keep;
        self.paint_traversal::<true>(root, layout, text, scale, size, interaction);
        CULL_OFF.store(false, Relaxed);
        let everything = (
            drawn(&self.scene),
            // As text, so that a NaN in one is the same as in the other.
            format!("{:?}", self.scene.transforms),
            format!("{:?} {:?}", self.scene.filters, self.scene.round_clips),
        );
        self.keep_next = keep;
        let enters_all = self.work.node_enters;
        self.paint_traversal::<true>(root, layout, text, scale, size, interaction);
        let culled = (
            drawn(&self.scene),
            // As text, so that a NaN in one is the same as in the other.
            format!("{:?}", self.scene.transforms),
            format!("{:?} {:?}", self.scene.filters, self.scene.round_clips),
        );
        let checks = CHECKS.fetch_add(1, Relaxed) + 1;
        if everything != culled {
            let different = DIFFERENT.fetch_add(1, Relaxed) + 1;
            eprintln!(
                "RSUI_CULL_CHECK: scene {checks} differs ({different} so far): {} quads in {} runs, culled {} quads in {} runs",
                everything.0.0.len(),
                everything.0.1.len(),
                culled.0.0.len(),
                culled.0.1.len()
            );
            let (a, b) = (&everything.0, &culled.0);
            if let Some(at) = (0..a.0.len().max(b.0.len())).find(|&at| a.0.get(at) != b.0.get(at)) {
                eprintln!("  first different quad {at}: {:?} / {:?}", a.0.get(at), b.0.get(at));
            }
            if let Some(at) = (0..a.1.len().max(b.1.len())).find(|&at| a.1.get(at) != b.1.get(at) || a.2.get(at) != b.2.get(at)) {
                eprintln!("  first different run {at}: {:?} to {:?} / {:?} to {:?}", a.1.get(at), a.2.get(at), b.1.get(at), b.2.get(at));
            }
            if everything.1 != culled.1 || everything.2 != culled.2 {
                eprintln!("  transforms or filters and round clips differ: {:?} / {:?}; {:?} / {:?}", everything.1, culled.1, everything.2, culled.2);
            }
        } else if checks.is_power_of_two() {
            eprintln!(
                "RSUI_CULL_CHECK: {checks} scenes the same, {} differ; this one {} quads, {} of {} nodes entered",
                DIFFERENT.load(Relaxed),
                culled.0.0.len(),
                self.work.node_enters,
                enters_all
            );
        }
    }

    /// The unchanged traversal, for independent diagnostic paint/pixel oracles.
    /// Resource ownership and atlas retry are identical to ordinary painting.
    #[cfg(feature = "render-diagnostics")]
    pub fn paint_full_for_diagnostics(
        &mut self,
        root: &Node,
        layout: &Layout,
        text: &mut TextEngine,
        scale: f32,
        size: (u32, u32),
        interaction: Interaction<'_>,
    ) -> &mut Scene {
        self.paint_traversal::<false>(root, layout, text, scale, size, interaction);
        self.last_scene()
    }

    fn paint_traversal<const PRUNE: bool>(
        &mut self,
        root: &Node,
        layout: &Layout,
        text: &mut TextEngine,
        scale: f32,
        size: (u32, u32),
        interaction: Interaction<'_>,
    ) -> &mut Scene {
        #[cfg(feature = "render-diagnostics")]
        {
            self.work = crate::diagnostics::SceneWork {
                serial: self.work.serial.saturating_add(1),
                path: crate::diagnostics::ScenePath::FullTraversal,
                atlas_before: self.atlas_observation(),
                ..Default::default()
            };
        }
        self.images.begin_frame();
        // If the atlas fills up and starts over, glyphs placed earlier in this frame are
        // gone. Painting again finds room for all of them, unless a single frame needs
        // more than the largest atlas can hold, which is rare enough to just accept.
        let mut caret_at = None;
        self.text_runs.begin_frame();
        for _ in 0..2 {
            let generation = self.atlas.generation();
            self.paragraph.seen = 0;
            let window = PixelRect {
                left: 0,
                top: 0,
                right: size.0 as i32,
                bottom: size.1 as i32,
            };
            self.scene.quads.clear();
            self.scene.batches.clear();
            self.scene.transforms.clear();
            self.scene.filters.clear();
            self.scene.round_clips.clear();
            self.scene.layers.clear();
            self.turning.clear();
            self.clips.clear();
            self.clips.push(window);
            self.fades.clear();
            #[cfg(feature = "mutation-pilot")]
            {
                self.pilot.captures = 0;
                self.pilot.valid = false;
                self.pilot.fast = false;
            }
            let mut visitor = SceneBuilder::<PRUNE> {
                #[cfg(feature = "render-diagnostics")]
                disable_text_replay: self.disable_text_replay,
                #[cfg(feature = "mutation-pilot")]
                pilot: &mut self.pilot,
                #[cfg(feature = "render-diagnostics")]
                work: &mut self.work,
                colors: Vec::new(),
                quads: &mut self.scene.quads,
                batches: &mut self.scene.batches,
                transforms: &mut self.scene.transforms,
                turning: &mut self.turning,
                transform: 0,
                filters: &mut self.scene.filters,
                round_clips: &mut self.scene.round_clips,
                filtering: Vec::new(),
                filter: 0,
                matrix: IDENTITY,
                atlas: &mut self.atlas,
                rasterizer: &mut self.rasterizer,
                text,
                scale,
                clips: &mut self.clips,
                images: &mut self.images,
                svg_hash: &self.svg_hash,
                fades: &mut self.fades,
                layers: &mut self.scene.layers,
                fading: 0,
                shading: interaction.leaving.is_some() || interaction.hover_amount < 1.0,
                batch_start: 0,
                interaction,
                names: &NAMES,
                glyphs: &mut self.glyphs,
                look_of: usize::MAX,
                look: Look::default(),
                caret_at: None,
                text_runs: &mut self.text_runs,
                paragraph: &mut self.paragraph,
                pictured_glyphs: false,
                rounded: Vec::new(),
            };
            layout.visit(root, &mut visitor);
            if let Some((text, x, y)) = interaction.tooltip {
                visitor.paint_tooltip(text, x, y, window);
            }
            if let Some(overlay) = interaction.overlay {
                visitor.paint_overlay(overlay, window);
            }
            visitor.finish_batch();
            caret_at = visitor.caret_at;
            #[cfg(feature = "render-diagnostics")]
            {
                visitor.work.paint_passes = visitor.work.paint_passes.saturating_add(1);
                visitor.work.quads_built = visitor
                    .work
                    .quads_built
                    .saturating_add(visitor.quads.len() as u64);
                visitor.work.batches_built = visitor
                    .work
                    .batches_built
                    .saturating_add(visitor.batches.len() as u64);
            }
            #[cfg(all(test, feature = "render-diagnostics"))]
            if std::mem::take(&mut self.retry_for_test) {
                self.atlas.start_over();
            }
            self.paragraph.end_pass();
            if self.atlas.generation() == generation {
                break;
            }
        }
        #[cfg(feature = "mutation-pilot")]
        {
            self.pilot.target = 0;
            // Resources can change independently; keep the first pilot resource-free.
            self.pilot.valid &= !self.scene.batches.iter().any(|b| b.image != 0);
        }
        self.text_runs.end_frame();
        let _image_start = self.images.end_frame(
            &mut self.scene.image_uploads,
            &mut self.scene.dropped_images,
        );
        // A scene that was worked out and never drawn still has glyphs waiting to go to the GPU, which
        // won't be asked for again. If the atlas has started over, those aren't any use.
        if self.scene.atlas_generation != self.atlas.generation() {
            self.scene.uploads.clear();
        }
        #[cfg(feature = "render-diagnostics")]
        let glyph_start = self.scene.uploads.len();
        self.scene.uploads.extend(self.atlas.take_uploads());
        #[cfg(feature = "render-diagnostics")]
        {
            for upload in &self.scene.uploads[glyph_start..] {
                self.work.glyph_exports.add(upload.coverage.len() as u64);
            }
            for upload in &self.scene.image_uploads[_image_start..] {
                self.work.image_exports.add(upload.pixels.len() as u64);
            }
        }
        self.had_transforms = !self.scene.transforms.is_empty();
        self.scene.text_look = if self.rasterizer.has_system() {
            crate::dwrite::BROWSER_TEXT_LOOK
        } else {
            [0.0, 0.0]
        };
        self.scene.window_color = (root.role == Role::Window)
            .then(|| Look::read(&root.props, &NAMES).background)
            .flatten()
            .filter(|color| color.a == 255);
        self.scene.atlas_size = self.atlas.size();
        self.scene.atlas_generation = self.atlas.generation();
        let keep = std::mem::take(&mut self.keep_next);
        self.keep_caret_scene(caret_at.filter(|_| keep));
        #[cfg(feature = "render-diagnostics")]
        self.finish_work();
        &mut self.scene
    }

    /// Keeps a copy of the scene if it has a caret and it's to be kept, for [`show_caret`](Self::show_caret).
    /// Otherwise the one before is out of date, and its memory goes back.
    fn keep_caret_scene(&mut self, caret_at: Option<usize>) {
        let scene = &self.scene;
        let found = caret_at.and_then(|at| {
            let batch_at = scene.batches.iter().position(|batch| {
                batch.first as usize <= at && at < (batch.first + batch.count) as usize
            })?;
            Some((at, batch_at))
        });
        let Some((at, batch_at)) = found else {
            self.with_caret = None;
            return;
        };
        let kept = self.with_caret.get_or_insert_default();
        kept.quads.clone_from(&scene.quads);
        kept.batches.clone_from(&scene.batches);
        kept.transforms.clone_from(&scene.transforms);
        kept.filters.clone_from(&scene.filters);
        kept.round_clips.clone_from(&scene.round_clips);
        kept.at = at;
        kept.batch_at = batch_at;
    }
}

struct SceneBuilder<'a, const PRUNE: bool> {
    #[cfg(feature = "render-diagnostics")]
    disable_text_replay: bool,
    #[cfg(feature = "mutation-pilot")]
    pilot: &'a mut PilotPaint,
    #[cfg(feature = "render-diagnostics")]
    work: &'a mut crate::diagnostics::SceneWork,
    /// What text takes its color from, if it doesn't have its own: each thing being drawn that has
    /// something in it, and says a color, as its address and that color.
    colors: Vec<(usize, Color)>,
    quads: &'a mut Vec<Quad>,
    batches: &'a mut Vec<Batch>,
    transforms: &'a mut Vec<Matrix>,
    turning: &'a mut Vec<Turning>,
    /// Which transform what's being drawn has, and what it is.
    transform: u16,
    filters: &'a mut Vec<Filter>,
    round_clips: &'a mut Vec<RoundClip>,
    /// What has a filter, and which filter was in use before it.
    filtering: Vec<(usize, u16)>,
    /// Which filter what's being drawn goes through.
    filter: u16,
    matrix: Matrix,
    atlas: &'a mut GlyphAtlas,
    rasterizer: &'a mut Rasterizer,
    text: &'a mut TextEngine,
    scale: f32,
    clips: &'a mut Vec<PixelRect>,
    images: &'a mut Images,
    svg_hash: &'a SvgHash,
    fades: &'a mut Vec<Fade>,
    layers: &'a mut Vec<Layer>,
    /// How many of those there are, close at hand, as this is checked for everything drawn.
    fading: u32,
    /// Whether a hover color is partway through coming or going, which it almost never is.
    shading: bool,
    batch_start: usize,
    interaction: Interaction<'a>,
    names: &'static Names,
    glyphs: &'a mut TextLayout,
    /// The last set of props read, and what was found. Neighbors often share theirs.
    look_of: usize,
    look: Look,
    /// Which quad is the caret, if it's drawn.
    caret_at: Option<usize>,
    text_runs: &'a mut TextRuns,
    paragraph: &'a mut ParagraphMemo,
    /// Whether text being placed has used a glyph kept as a picture, whose quads can't be kept in `text_runs`.
    pictured_glyphs: bool,
    /// The round clips what's drawn now is in, innermost last: how many clips there were with each, and which it is, counting
    /// from 1.
    rounded: Vec<(usize, u16)>,
}

/// The nearest whole number. `round` is a call into a library unless the program's built for newer
/// processors than it is, and this runs four times for everything drawn, on screen or not. Numbers too
/// big for this are far off the screen, and come out as the biggest it can do, still far off the screen.
#[inline(always)]
fn snap(value: f32) -> f32 {
    (value + 0.5f32.copysign(value)) as i32 as f32
}

/// Whether `candidate` is this very element.
#[inline(always)]
fn is(candidate: Option<&Element>, element: &Element) -> bool {
    candidate.is_some_and(|candidate| std::ptr::eq(candidate, element))
}

impl<const PRUNE: bool> SceneBuilder<'_, PRUNE> {
    #[inline(always)]
    fn let_go_of_color(&mut self, element: &Element) {
        if self
            .colors
            .last()
            .is_some_and(|held| held.0 == std::ptr::from_ref(element) as usize)
        {
            self.colors.pop();
        }
    }

    #[inline(always)]
    fn around_color(&self) -> Color {
        self.colors.last().map_or(DEFAULT_TEXT, |held| held.1)
    }

    /// Which of the scene's round clips what's drawn now is cut off by, counting from 1, or 0 for none.
    #[inline(always)]
    fn round(&self) -> u16 {
        self.rounded.last().map_or(0, |one| one.1)
    }

    /// Cuts what's drawn after this off at a box's round corners, as well as wherever it's already cut off, or keeps it out of
    /// the box if `outside`, until the clip that was pushed last is popped. `edges` are its left, top, right and bottom.
    #[cold]
    #[inline(never)]
    fn push_round_clip(&mut self, edges: [f32; 4], corners: [[f32; 2]; 4], outside: bool) {
        self.finish_batch();
        let outer = self.round();
        self.round_clips.push(RoundClip { edges, corners, outside, transform: self.transform, outer });
        let at = self.round_clips.len().min(usize::from(u16::MAX)) as u16;
        self.rounded.push((self.clips.len(), at));
    }

    /// Stops cutting off what's drawn at the clip that was pushed last, and at any round corners that went with it.
    fn pop_clip(&mut self) {
        self.finish_batch();
        let depth = self.clips.len();
        self.clips.pop();
        while self.rounded.last().is_some_and(|one| one.0 >= depth) {
            self.rounded.pop();
        }
    }

    /// How far across and down each of an element's round corners goes on a box this big, in physical pixels, from the top left
    /// going round, or `None` if they're all square. It's the element whose look was read last.
    fn corners(&self, element: &Element, area: [f32; 4]) -> Option<[[f32; 2]; 4]> {
        let shape = if self.look.shaped { crate::gradient::BoxShape::of(element, area, self.scale) } else { None };
        let corners = match shape {
            Some(shape) => shape.corners(),
            None => [[fitting_radius(self.look.corner_radius * self.scale, area); 2]; 4],
        };
        corners.iter().any(|corner| corner[0] > 0.0 && corner[1] > 0.0).then_some(corners)
    }

    /// Cuts what's in an element with round corners off at them, inside its border, as CSS's `overflow: hidden` has it: the
    /// curve inside a border is the one outside it less the width of the border on each side of it. It's the element whose look
    /// was read last, at `rect`.
    #[cold]
    #[inline(never)]
    fn round_off(&mut self, element: &Element, rect: Rect) {
        let left = snap(rect.x * self.scale);
        let top = snap(rect.y * self.scale);
        let right = snap((rect.x + rect.width) * self.scale);
        let bottom = snap((rect.y + rect.height) * self.scale);
        let Some(corners) = self.corners(element, [left, top, right - left, bottom - top]) else {
            return;
        };
        let (top_width, right_width, bottom_width, left_width) = border_sides(element);
        let [top_width, right_width, bottom_width, left_width] =
            [top_width, right_width, bottom_width, left_width].map(|width| drawn_width(width, self.scale));
        let less = |[across, down]: [f32; 2], side: f32, end: f32| [(across - side).max(0.0), (down - end).max(0.0)];
        let corners = [
            less(corners[0], left_width, top_width),
            less(corners[1], right_width, top_width),
            less(corners[2], right_width, bottom_width),
            less(corners[3], left_width, bottom_width),
        ];
        let edges = [left + left_width, top + top_width, right - right_width, bottom - bottom_width];
        self.push_round_clip(edges, corners, false);
    }

    #[inline(always)]
    /// Cuts off what's drawn after this at the edges of `rect`, as well as wherever it's already cut off, until it's popped.
    fn push_clip(&mut self, rect: Rect) {
        self.finish_batch();
        let clip = self.clip().intersect(PixelRect {
            left: (rect.x * self.scale).round() as i32,
            top: (rect.y * self.scale).round() as i32,
            right: ((rect.x + rect.width) * self.scale).round() as i32,
            bottom: ((rect.y + rect.height) * self.scale).round() as i32,
        });
        self.clips.push(clip);
    }

    fn clip(&self) -> PixelRect {
        *self
            .clips
            .last()
            .expect("the window is always the outer clip")
    }

    /// Ends the current run of quads, since what comes next has a different clip.
    fn finish_batch(&mut self) {
        let count = self.quads.len() - self.batch_start;
        if count > 0 {
            self.batches.push(Batch {
                first: self.batch_start as u32,
                count: count as u32,
                clip: self.clip_on_screen(self.clip()),
                image: 0,
                transform: self.transform,
                filter: self.filter,
                round: self.round(),
            });
            self.batch_start = self.quads.len();
        }
    }

    /// What a clip comes to, as it's only ever a rectangle on the screen. For what's been turned, that's the rectangle around it.
    #[inline(always)]
    fn clip_on_screen(&self, clip: PixelRect) -> PixelRect {
        if self.transform == 0 {
            clip
        } else {
            self.turned_clip(clip)
        }
    }

    #[cold]
    #[inline(never)]
    fn turned_clip(&self, clip: PixelRect) -> PixelRect {
        let outer = self
            .turning
            .last()
            .map_or(PixelRect::EVERYWHERE, |one| one.outer_clip);
        if clip == PixelRect::EVERYWHERE {
            return outer;
        }
        let corners = [
            (clip.left, clip.top),
            (clip.right, clip.top),
            (clip.left, clip.bottom),
            (clip.right, clip.bottom),
        ]
        .map(|(x, y)| transform::apply_matrix(self.matrix, x as f32, y as f32));
        let along = |pick: fn(&(f32, f32)) -> f32| {
            corners
                .iter()
                .map(pick)
                .fold((f32::MAX, f32::MIN), |(low, high), value| {
                    (low.min(value), high.max(value))
                })
        };
        let (left, right) = along(|corner| corner.0);
        let (top, bottom) = along(|corner| corner.1);
        outer.intersect(PixelRect {
            left: left.floor() as i32,
            top: top.floor() as i32,
            right: right.ceil() as i32,
            bottom: bottom.ceil() as i32,
        })
    }

    /// Starts drawing something that's turned or resized. What's in it could end up anywhere, so nothing's left out of it for being
    /// out of sight.
    #[cold]
    #[inline(never)]
    fn begin_transform(&mut self, element: &Element, edges: [f32; 4]) {
        let [left, top, right, bottom] = edges;
        let own =
            transform::transform_of(element, left, top, right - left, bottom - top, self.scale)
                .unwrap_or(IDENTITY);
        self.finish_batch();
        let outer_clip = self.clip_on_screen(self.clip());
        self.turning.push(Turning {
            element: std::ptr::from_ref(element) as usize,
            moving: matches!(
                element.props.get("transform_moving"),
                Some(PropValue::Bool(true))
            ),
            before: self.transform,
            matrix_before: self.matrix,
            outer_clip,
        });
        self.matrix = transform::multiply(self.matrix, own);
        self.transforms.push(self.matrix);
        self.transform = self.transforms.len().min(usize::from(u16::MAX)) as u16;
        self.clips.push(PixelRect::EVERYWHERE);
    }

    /// Finishes what [`begin_transform`](Self::begin_transform) started, if this is what it was started for.
    #[cold]
    #[inline(never)]
    fn finish_transform(&mut self, element: &Element) {
        let is_this = self
            .turning
            .last()
            .is_some_and(|one| one.element == std::ptr::from_ref(element) as usize);
        if !is_this {
            return;
        }
        self.finish_batch();
        if let Some(done) = self.turning.pop() {
            self.transform = done.before;
            self.matrix = done.matrix_before;
            self.clips.pop();
        }
    }

    /// Reads what this element looks like into `self.look`. That isn't copied out, as this runs for
    /// everything that's drawn and it's big.
    #[inline]
    fn read_look(&mut self, element: &Element) {
        let identity = element.props.identity();
        if identity != self.look_of {
            self.look_of = identity;
            self.look.read_from(&element.props, self.names);
        }
    }

    #[inline(always)]
    fn paint_text(&mut self, element: &Element, rect: Rect) {
        let mut style = TextStyle::from_element(element);
        style = style.scaled(self.scale);
        let content = content_box_of(&self.look, rect, self.scale);
        let color = self.look.text_color.unwrap_or(self.around_color());
        // Text that's cut off at its edges, as CSS's `overflow: hidden` has it, doesn't show past them, however it's arranged:
        // a `<pre>` that's kept to its lines is cut off at its `max-height` too.
        let cut = rsui_layout::clips(element);
        if self.look.arranged {
            if cut {
                self.clip_to(rect, element);
            }
            self.paint_arranged_text(element, rect, style, content, color);
            if cut {
                self.unclip();
            }
            return;
        }
        if cut {
            self.clip_to(rect, element);
        }
        let clip = self.clip();
        let keeps_words = keeps_words(element);
        let max_lines = rsui_text::max_lines(element);
        // A complete short run can be moved during scrolling without laying out
        // or placing its glyphs again. Keep fractional horizontal pen positions
        // in the key (rasterization depends on them). A proven single baseline
        // can follow fractional scrolling using its exact rounded displacement;
        // other runs require a whole-pixel vertical displacement.
        // Long/partially laid-out text and transformed/color glyphs use the
        // existing exact-position cache. Current batch clips remain authoritative.
        let translated = PRUNE
            && element.name.len() <= 128
            && self.transform == 0
            && !self.text.has_color_fonts()
            && !self.look.text_shadow
            && !self.look.gradient
            && max_lines.is_none()
            && content.left.is_finite()
            && content.top.is_finite();
        #[cfg(feature = "render-diagnostics")]
        let translated = translated && !self.disable_text_replay;
        let origin = if translated { [content.left.floor(), content.top] } else { [0.0; 2] };
        let key = (!self.look.text_shadow && !self.look.gradient && max_lines.is_none())
            .then(|| TextRunKey {
                name: element.name.clone(),
                style,
                keeps_words,
                place: [content.left - origin[0], content.top - origin[1], content.width].map(f32::to_bits),
                clip: if translated { PixelRect::EVERYWHERE } else { clip },
                translated,
                color,
                rounding: self.pen_rounding().to_bits(),
                zoom: self
                    .plain_zoom()
                    .map(|(scale, left, top)| [scale, left, top].map(f32::to_bits)),
                engine: self.text.id(),
                fonts: self.text.font_count(),
                atlas_generation: self.atlas.generation(),
            })
            // A style with a NaN in it isn't equal even to itself, so it could never be found again: don't keep it.
            .filter(|key| equals_itself(&key.style));
        let first_quad = self.quads.len();
        let replayed = key.as_ref().and_then(|key| self.text_runs.find(key, origin));
        #[cfg(feature = "render-diagnostics")]
        if replayed.is_some() {
            self.work.text_run_reuses = self.work.text_run_reuses.saturating_add(1);
            if translated {
                self.work.translated_text_run_reuses = self.work.translated_text_run_reuses.saturating_add(1);
            }
        }
        // Built for testing, it works them out anyway, and checks that they come out the same.
        #[cfg(debug_assertions)]
        let replayed = replayed.map(|(quads, by)| moved_quads(quads, by).collect::<Vec<_>>());
        #[cfg(not(debug_assertions))]
        if let Some((quads, by)) = replayed {
            self.quads.extend(moved_quads(quads, by)
                .filter(|quad| !translated || quad_in_clip(quad, clip)));
            if cut {
                self.unclip();
            }
            return;
        }
        let batches_before = self.batches.len();
        self.pictured_glyphs = false;
        let limit = if keeps_words && element.name.len() < KEPT_WORDS_LONGEST {
            // A word that's wider than there's room for goes past the edge, as on a page, rather than being broken, which
            // is how it was measured.
            content
                .width
                .max(self.text.longest_word(&element.name, style))
        } else {
            content.width
        };
        self.text.layout_visible_into(
            &element.name,
            style,
            limit,
            (
                clip.top as f32 - content.top,
                clip.bottom as f32 - content.top,
            ),
            self.glyphs,
        );
        // Layout can lazily open a color fallback. Such runs may emit image
        // batches, which must retain their original clipping/batch indices.
        let cache_layout_valid = !translated || !self.text.has_color_fonts();
        let translated = translated && cache_layout_valid;
        let key = key.filter(|_| cache_layout_valid);
        #[cfg(debug_assertions)]
        let replayed = replayed.filter(|_| cache_layout_valid);
        if let Some(lines) = max_lines {
            self.clamp_lines(&element.name, style, limit, lines);
        }
        if self.look.text_shadow {
            self.place_text_shadow(element, style, content.left, content.top);
        }
        let first_glyph = self.quads.len();
        self.place_glyphs_in_clip(style, content.left, content.top, color, if translated { PixelRect::EVERYWHERE } else { clip });
        if self.look.gradient {
            self.color_glyphs_by_background(element, rect, first_glyph);
        }
        #[cfg(debug_assertions)]
        if let Some(quads) = replayed {
            assert_eq!(
                quads,
                self.quads[first_quad..],
                "kept glyphs for {:?} are what placing them again gives",
                element.name
            );
        }
        // Text with a glyph that's a picture is in batches of its own, which these don't keep.
        if let Some(key) = key {
            if self.batches.len() == batches_before && !self.pictured_glyphs {
                let baseline = self.glyphs.glyphs.first().map(|glyph| glyph.y)
                    .filter(|baseline| self.glyphs.glyphs.iter().all(|glyph| glyph.y == *baseline));
                self.text_runs.keep(key, &self.quads[first_quad..], origin, baseline);
            }
        }
        if translated {
            let mut to = first_quad;
            for from in first_quad..self.quads.len() {
                let quad = self.quads[from];
                if quad_in_clip(&quad, clip) {
                    self.quads[to] = quad;
                    to += 1;
                }
            }
            self.quads.truncate(to);
        }
        if cut {
            self.unclip();
        }
    }

    /// Keeps only the first `keep` lines of what's been laid out, the last of them ending in `â€¦` if there was more, as CSS's
    /// `line-clamp` has it.
    #[cold]
    #[inline(never)]
    fn clamp_lines(&mut self, text: &str, style: TextStyle, limit: f32, keep: usize) {
        let layout = &mut *self.glyphs;
        if keep == 0 || layout.lines.len() <= keep || layout.lines.first().is_none_or(|line| line.start != 0) {
            return;
        }
        let last = layout.lines[keep - 1];
        let shown = &text[last.start..last.end.max(last.start)];
        layout.lines.truncate(keep);
        layout.glyphs.truncate(last.first_glyph as usize);
        let mut one = rsui_text::TextLayout::default();
        self.text
            .layout_one_line_into(&format!("{shown}\u{2026}"), style, limit, true, &mut one);
        let Some(first) = one.lines.first() else {
            return;
        };
        let shift = last.baseline - first.baseline;
        let layout = &mut *self.glyphs;
        let start = layout.glyphs.len() as u32;
        layout
            .glyphs
            .extend(one.glyphs.iter().map(|glyph| rsui_text::PlacedGlyph {
                y: glyph.y + shift,
                x: glyph.x + last.x,
                ..*glyph
            }));
        if let Some(line) = layout.lines.last_mut() {
            line.first_glyph = start;
            line.glyph_count = layout.glyphs.len() as u32 - start;
            line.width = first.width;
        }
    }

    /// Draws text that's kept to one line, or put somewhere along its line, or underlined.
    #[cold]
    #[inline(never)]
    fn paint_arranged_text(
        &mut self,
        element: &Element,
        rect: Rect,
        style: TextStyle,
        content: ContentBox,
        color: Color,
    ) {
        let look = Arrangement::of(element);
        if look.ellipsis || look.no_wrap {
            self.text.layout_one_line_into(
                &element.name,
                style,
                content.width,
                look.ellipsis,
                self.glyphs,
            );
        } else {
            self.text
                .layout_into(&element.name, style, content.width, self.glyphs);
            // Only so many lines, the last ending in "…", as it was measured, before they're put along their lines: centered
            // text that's kept to two lines, as a grid's names are, is centered as it's cut.
            if let Some(lines) = rsui_text::max_lines(element) {
                self.clamp_lines(&element.name, style, content.width, lines);
            }
        }
        rsui_text::space_letters(self.glyphs, style.letter_spacing);
        rsui_text::space_words(self.glyphs, &element.name, style.word_spacing);
        if let Some(along) = look.along {
            for index in 0..self.glyphs.lines.len() {
                let line = self.glyphs.lines[index];
                let wanted = (content.width - line.width).max(0.0) * along;
                let shift = wanted - line.x;
                let (first, count) = (line.first_glyph as usize, line.glyph_count as usize);
                for glyph in &mut self.glyphs.glyphs[first..first + count] {
                    glyph.x += shift;
                }
                self.glyphs.lines[index].x = wanted;
            }
        }
        if self.look.text_shadow {
            self.place_text_shadow(element, style, content.left, content.top);
        }
        let first_glyph = self.quads.len();
        self.place_glyphs(style, content.left, content.top, color);
        // Text that shows its background through it, as gradient text does, whatever's arranged about it, as Mantine's bold
        // `Text variant="gradient"` is.
        if self.look.gradient {
            self.color_glyphs_by_background(element, rect, first_glyph);
        }
        if look.decoration.any() {
            for index in 0..self.glyphs.lines.len() {
                let line = self.glyphs.lines[index];
                self.paint_decoration(
                    look.decoration,
                    content.left + line.x,
                    content.top + line.baseline,
                    line.width,
                    style.size,
                    color,
                );
            }
        }
    }

    /// Draws the lines under, through and over a stretch of text, as `decoration` says, from `left` along `width`, for text
    /// this big, in physical pixels.
    #[cold]
    #[inline(never)]
    fn paint_decoration(
        &mut self,
        decoration: crate::paint::Decoration,
        left: f32,
        baseline: f32,
        width: f32,
        size: f32,
        color: Color,
    ) {
        use crate::paint::LineStyle;
        let color = decoration.color.unwrap_or(color);
        let thickness = decoration
            .thickness
            .map_or((size / 14.0).round(), |thickness| {
                (thickness * self.scale).round()
            })
            .max(1.0);
        let mut tops = Vec::with_capacity(3);
        if decoration.under {
            let below = decoration
                .offset
                .map_or((size * 0.085).round().max(1.0), |offset| {
                    (offset * self.scale).round()
                });
            tops.push((baseline + below).round());
        }
        if decoration.through {
            // Where the font says a line through it goes, a little over a quarter of its size up, as most fonts have it, with the line
            // in the middle of that.
            tops.push((baseline - size * 0.259 - thickness / 2.0).round());
        }
        if decoration.over {
            tops.push((baseline - (size * 0.905).round()).round());
        }
        let right = left + width;
        for top in tops {
            match decoration.style {
                LineStyle::Solid => {
                    self.quads
                        .push(solid_quad([left, top, width, thickness], color, 0.0))
                }
                LineStyle::Double => {
                    self.quads
                        .push(solid_quad([left, top, width, thickness], color, 0.0));
                    self.quads.push(solid_quad(
                        [left, top + thickness * 2.0, width, thickness],
                        color,
                        0.0,
                    ));
                }
                LineStyle::Dotted | LineStyle::Dashed => {
                    let (on, off) = if decoration.style == LineStyle::Dotted {
                        (thickness, thickness)
                    } else {
                        (thickness * 3.0, thickness * 2.0)
                    };
                    let mut x = left;
                    while x < right {
                        self.quads.push(solid_quad(
                            [x, top, on.min(right - x), thickness],
                            color,
                            0.0,
                        ));
                        x += on + off;
                    }
                }
                LineStyle::Wavy => {
                    // A smooth wave as Blink draws one, drawn as a picture that's kept, as the same one is drawn again and again.
                    let across = (right - left).ceil().max(1.0) as u32;
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    "wave".hash(&mut hasher);
                    (across, thickness.to_bits(), color.to_prop()).hash(&mut hasher);
                    let hash = hasher.finish();
                    let key = ColorGlyphKey {
                        font: u16::MAX - 3,
                        glyph: (hash >> 48) as u16,
                        size_bits: hash as u32,
                        text_color: ((hash >> 16) as u32).to_le_bytes(),
                    };
                    let made = self
                        .images
                        .color_glyph(key, || crate::gradient::draw_wave(across, thickness, color));
                    if let Some(made) = made {
                        let clip = self.clip();
                        self.place_color_glyph(made, left, top, Color::WHITE, clip);
                    }
                }
            }
        }
    }

    /// Draws text made of parts that look different. Backgrounds go first, as they can reach past
    /// the words next to them, and then the words on top.
    #[cold]
    #[inline(never)]
    fn paint_paragraph(&mut self, element: &Element, rect: Rect) {
        #[derive(Clone, Copy)]
        struct PartLook<'e> {
            element: Option<&'e Element>,
            color: Color,
            decoration: crate::paint::Decoration,
            background: Option<Color>,
            hover: Option<Color>,
            pressed: Option<Color>,
            radius: f32,
            reach: f32,
        }
        let content = content_box_of(&self.look, rect, self.scale);
        let (one_line, along) = paragraph_arrangement(element);
        let mut style = TextStyle::from_element(element);
        style = style.scaled(self.scale);
        let names = self.names;
        let scale = self.scale;
        let first = PartLook {
            element: None,
            color: self.look.text_color.unwrap_or(self.around_color()),
            decoration: if self.look.arranged {
                Arrangement::of(element).decoration
            } else {
                crate::paint::Decoration::default()
            },
            background: None,
            hover: None,
            pressed: None,
            radius: 0.0,
            reach: 0.0,
        };
        let mut looks = Vec::new();
        self.paragraph.seen = self.paragraph.seen.saturating_add(1);
        if !PRUNE || self.paragraph.seen > 1 {
            self.paragraph.clear_key();
        }
        let revision = self.text.rich_scratch_revision();
        let mut scratch = self.text.take_rich();
        scratch.text.clear();
        gather_text(
            element,
            style,
            scale,
            first,
            &mut |part, around: PartLook<'_>, is_run| {
                let look = Look::read(&part.props, names);
                let own = PartLook {
                    element: Some(part),
                    color: look.text_color.unwrap_or(around.color),
                    // Lines through text go through what's inside it too, as a browser has them.
                    decoration: if look.arranged {
                        let own = Arrangement::of(part).decoration;
                        if own.any() { own } else { around.decoration }
                    } else {
                        around.decoration
                    },
                    background: look.background.or(around.background),
                    hover: look.hover.or(around.hover),
                    pressed: look.pressed.or(around.pressed),
                    radius: if look.background.is_some() {
                        look.corner_radius * scale
                    } else {
                        around.radius
                    },
                    reach: if look.background.is_some() {
                        look.padding_left * scale
                    } else {
                        around.reach
                    },
                };
                if is_run {
                    looks.push(own);
                }
                own
            },
            &mut scratch.text,
        );
        // A paragraph whose box is only as tall as its fonts make it has what's in it as far up: see `TextStyle::inline_area`.
        scratch.text.inline_top = self.text.inline_area(style).map_or(0.0, |(top, _)| top);
        let limit = if one_line {
            f32::INFINITY
        } else {
            content.width
        };
        let RichScratch { text, layout } = &mut *scratch;
        let fonts = self.text.font_count();
        let stamp = revision.map(|revision| (self.text.id(), revision, fonts));
        let reused = PRUNE && self.paragraph.matches(stamp, text, limit, along);
        self.paragraph.stamp = None;
        if !reused {
            self.text.layout_rich_into(text, limit, along, layout);
        }
        #[cfg(feature = "render-diagnostics")]
        if reused {
            self.work.paragraph_reuses = self.work.paragraph_reuses.saturating_add(1);
        } else {
            self.work.paragraph_layouts = self.work.paragraph_layouts.saturating_add(1);
        }
        let keep = PRUNE
            && fonts == self.text.font_count()
            && (reused || self.paragraph.remember(text, limit, along));
        let clip = self.clip();
        for piece in &layout.pieces {
            let part = &looks[piece.run as usize];
            let shown = part.element.and_then(|element| {
                if is(self.interaction.pressed, element) {
                    part.pressed.or(part.hover).or(part.background)
                } else if is(self.interaction.hovered, element) {
                    part.hover.or(part.background)
                } else {
                    part.background
                }
            });
            let Some(background) = shown else { continue };
            let x = content.left + piece.x - piece.before - part.reach;
            let y = content.top + piece.top;
            let width = piece.before + piece.width + piece.after + part.reach * 2.0;
            if clip.overlaps(x, y, x + width, y + piece.height) {
                self.quads.push(solid_quad(
                    [x, y, width, piece.height],
                    background,
                    part.radius,
                ));
            }
        }
        if let Some((selected, range)) = self.interaction.text_selection {
            if std::ptr::eq(selected, element) {
                self.paint_text_selection(text, layout, range, (content.left, content.top));
            }
        }
        // A line kept to one that's too long, and says to end in `…`, as CSS's `text-overflow: ellipsis` does: what doesn't fit
        // before the `…` isn't drawn, whichever run it's in, and the `…` is in the paragraph's own look.
        let cut = (one_line
            && matches!(element.props.get("ellipsis"), Some(PropValue::Bool(true)))
            && layout.width() > content.width + 0.01)
            .then(|| {
                let (dots, width) = self.text.ellipsis_glyph(style);
                (dots, (content.width - width).max(0.0))
            });
        let mut dots_at: Option<(f32, f32)> = None;
        for at in 0..layout.pieces.len() {
            let piece = layout.pieces[at];
            let part = looks[piece.run as usize];
            let top = content.top + piece.top;
            let start = piece.first_glyph as usize;
            let all = &layout.glyphs[start..start + piece.glyph_count as usize];
            let mut kept = all.len();
            if let Some((_, room)) = cut {
                let end_of = |index: usize| all.get(index + 1).map_or(piece.x + piece.width, |next| next.x);
                kept = (0..all.len()).take_while(|&index| end_of(index) <= room).count();
                // The `…` goes where the last of what's kept ends, on its line.
                if let Some(first) = all.first() {
                    let end = if kept == 0 { 0.0 } else { end_of(kept - 1) };
                    if dots_at.is_none_or(|(was, _)| end > was) {
                        dots_at = Some((end, first.y));
                    }
                }
                if kept == 0 {
                    continue;
                }
            }
            if !clip.overlaps(
                content.left + piece.x - piece.before,
                top,
                content.left + piece.x + piece.width + piece.after,
                top + piece.height,
            ) {
                continue;
            }
            let piece_style = text.runs[piece.run as usize].style;
            self.glyphs.glyphs.clear();
            self.glyphs.glyphs.extend_from_slice(&all[..kept]);
            self.place_glyphs(piece_style, content.left, content.top, part.color);
            if part.decoration.any() {
                let baseline = layout
                    .glyphs
                    .get(start)
                    .map_or(piece.top + piece.height, |glyph| glyph.y);
                self.paint_decoration(
                    part.decoration,
                    content.left + piece.x - piece.before,
                    content.top + baseline,
                    piece.before + piece.width + piece.after,
                    piece_style.size,
                    part.color,
                );
            }
            if let Some(ringed) = part
                .element
                .filter(|part| is(self.interaction.ring, part))
            {
                self.paint_ring(ringed, [
                    content.left + piece.x,
                    top,
                    content.left + piece.x + piece.width,
                    top + piece.height,
                ]);
            }
        }
        if let (Some((mut dots, _)), Some((x, baseline))) = (cut, dots_at) {
            dots.x = x;
            dots.y = baseline;
            self.glyphs.glyphs.clear();
            self.glyphs.glyphs.push(dots);
            self.place_glyphs(style, content.left, content.top, first.color);
        }
        self.text.give_back_rich(scratch);
        if keep && fonts == self.text.font_count() {
            self.paragraph.stamp = self
                .text
                .rich_scratch_revision()
                .map(|revision| (self.text.id(), revision, fonts));
        } else {
            self.paragraph.clear_key();
        }
    }

    /// Draws what's selected in a paragraph, behind its words, as tall as the lines it's on.
    #[inline(never)]
    fn paint_text_selection(
        &mut self,
        text: &RichText,
        layout: &RichLayout,
        (from, to): (usize, usize),
        (left, top): (f32, f32),
    ) {
        let mut spans = Vec::new();
        let clip = self.clip();
        for line in &layout.lines {
            // Only untransformed selection rectangles use this screen-space clip.
            // Skip shaping selection spans for lines that cannot contribute pixels.
            if PRUNE
                && self.transform == 0
                && selection_line_outside(clip, top + line.top, line.height)
            {
                continue;
            }
            let first = line.first_piece as usize;
            for piece in &layout.pieces[first..first + line.piece_count as usize] {
                let (start, end) = (piece.start as usize, piece.end as usize);
                if to <= start || from > end {
                    continue;
                }
                spans.clear();
                if from.max(start) < to.min(end) {
                    let selected = (from.max(start) - start, to.min(end) - start);
                    let style = piece.style(text.runs[piece.run as usize].style);

                    self.text
                        .selection_spans(&text.text[start..end], style, selected, &mut spans);
                }
                for span in &mut spans {
                    span.0 += piece.x;
                    span.1 += piece.x;
                }
                // The spaces after it are selected if what's selected goes on past them.
                if to > end {
                    if piece.spaces_after > 0.0 {
                        let x = piece.x + piece.width;
                        spans.push((x, x + piece.spaces_after));
                    }
                    if piece.spaces_before > 0.0 {
                        spans.push((piece.x - piece.spaces_before, piece.x));
                    }
                }
                for &(start_x, end_x) in &spans {
                    self.push_selection([
                        left + start_x,
                        top + line.top,
                        end_x - start_x,
                        line.height,
                    ]);
                }
            }
        }
    }

    // Keep selection-only loops separate from the ordinary text painting path.
    #[inline(never)]
    fn paint_area_selection(
        &mut self,
        selection: &[(f32, f32)],
        lines: &[u32],
        left: f32,
        top: f32,
        height: f32,
    ) {
        let clip = self.clip();
        for ((from, to), line) in selection.iter().zip(lines) {
            let y = top + *line as f32 * height;
            let (from, to) = (left + from, left + to);
            if PRUNE
                && self.transform == 0
                && selection_line_outside(clip, y, height)
                && from.is_finite()
                && to.is_finite()
                && to >= from
                && (to - from).is_finite()
            {
                continue;
            }
            let (from, to) = (from.round(), to.round());
            self.push_selection([from, y, to - from, height]);
        }
    }

    #[inline(never)]
    fn paint_input_selection(
        &mut self,
        selection: &[(f32, f32)],
        left: f32,
        top: f32,
        height: f32,
    ) {
        for (from, to) in selection {
            let (from, to) = ((left + from).round(), (left + to).round());
            self.push_selection([from, top, to - from, height]);
        }
    }

    /// Selection is a plain rectangle, with no shadow or stroke outside its bounds.
    /// Transformed batches retain their original path: their screen scissor is an
    /// enclosing rectangle, so a local-space rejection would be insufficient.
    fn push_selection(&mut self, rect: [f32; 4]) {
        if PRUNE && self.transform == 0 && selection_rect_outside(self.clip(), rect) {
            return;
        }
        self.quads.push(solid_quad(rect, SELECTION, 0.0));
    }

    /// Starts on something with any of what most elements don't have, drawing its background and
    /// border if it can be seen. See [`Look::unusual`].
    #[cold]
    #[inline(never)]
    fn enter_unusual(&mut self, element: &Element, edges: [f32; 4], seen: bool) -> bool {
        let seen = if self.look.transformed {
            self.begin_transform(element, edges);
            true
        } else {
            seen
        };
        if self.look.filtered {
            self.begin_filter(element);
        }
        self.paint_unusual(element, edges, seen);
        seen
    }

    /// Starts drawing something whose colors go through a filter, as CSS's `filter` says, with what's around it, if that has
    /// one too.
    #[cold]
    #[inline(never)]
    fn begin_filter(&mut self, element: &Element) {
        let Some(PropValue::Str(text)) = element.props.get("css_filter") else {
            return;
        };
        let Some(own) = Filter::read(text) else {
            return;
        };
        self.finish_batch();
        let around = self
            .filter
            .checked_sub(1)
            .and_then(|at| self.filters.get(usize::from(at)))
            .copied()
            .unwrap_or(Filter::NONE);
        self.filtering
            .push((std::ptr::from_ref(element) as usize, self.filter));
        self.filters.push(around.after(own));
        self.filter = self.filters.len().min(usize::from(u16::MAX)) as u16;
    }

    /// Finishes what [`begin_filter`](Self::begin_filter) started, if this is what it was started for.
    #[cold]
    #[inline(never)]
    fn finish_filter(&mut self, element: &Element) {
        if self
            .filtering
            .last()
            .is_some_and(|(one, _)| *one == std::ptr::from_ref(element) as usize)
        {
            self.finish_batch();
            if let Some((_, before)) = self.filtering.pop() {
                self.filter = before;
            }
        }
    }

    fn paint_unusual(&mut self, element: &Element, edges: [f32; 4], seen: bool) {
        let look = self.look;
        if look.faded > 0.0 {
            self.fading += 1;
            self.fades.push(Fade {
                element: std::ptr::from_ref(element) as usize,
                first_quad: self.quads.len(),
                opacity: 1.0 - look.faded,
            });
        }
        if !seen {
            return;
        }
        let [left, top, right, bottom] = edges;
        let area = [left, top, right - left, bottom - top];
        let radius = fitting_radius(look.corner_radius * self.scale, area);
        if look.backdrop_blur > 0.0 && area[2] > 0.0 && area[3] > 0.0 {
            self.finish_batch();
            let clip = self.clip_on_screen(self.clip());
            let corners = look.shaped.then(|| self.corners(element, area)).flatten();
            if let Some(corners) = corners {
                self.push_round_clip(edges, corners, false);
            }
            let blur = (look.backdrop_blur * self.scale * 16.0).round().clamp(1.0, f32::from(u16::MAX)) as u16;
            self.quads.push(Quad { rect: area, glyph: [blur, 0, 0, 0], color: [0; 4], radius });
            self.batches.push(Batch {
                first: self.batch_start as u32,
                count: 1,
                clip,
                image: BACKDROP_IMAGE,
                transform: self.transform,
                filter: 0,
                round: self.round(),
            });
            self.batch_start = self.quads.len();
            if corners.is_some() {
                self.rounded.pop();
            }
        }
        let shadows = if look.shadowed {
            Shadow::all(element)
        } else {
            Vec::new()
        };
        // The first is on top, as CSS has it, so they're drawn from the last.
        for shadow in shadows.iter().rev().filter(|shadow| !shadow.inset) {
            // A blur's spread is half what it says, as CSS has it, and it reaches three times that.
            let reach = shadow.blur * self.scale * 1.5;
            let spread = shadow.spread * self.scale;
            let shadow_radius = if radius > 0.0 {
                (radius + spread).max(0.0)
            } else {
                0.0
            };
            let quad = Quad {
                rect: [
                    left - spread - reach + shadow.x * self.scale,
                    top - spread - reach + shadow.y * self.scale,
                    right - left + (spread + reach) * 2.0,
                    bottom - top + (spread + reach) * 2.0,
                ],
                // The second number's only used by shadows, as how far it reaches, in sixteenths of a pixel.
                glyph: [0, (reach * 16.0).min(f32::from(u16::MAX)) as u16, 0, 0],
                color: shadow.color.premultiplied(),
                radius: shadow_radius,
            };
            // Under what can't be seen through, the whole shadow can go, as it's covered, unless what it's under is being faded
            // (`opacity`), which a browser does to the box and its shadow together: a Menu's dropdown went grey as it faded in.
            if self.fading == 0
                && look
                    .background
                    .is_some_and(|background| background.a == 255)
            {
                self.quads.push(quad);
            } else if reach == 0.0 && shadow.x == 0.0 && shadow.y == 0.0 && spread > 0.0 {
                // A shadow that's only spread, as Tailwind's `ring` is, shows as a ring around what it's behind, its corners
                // as round as the box's and as much more as it's spread.
                self.quads
                    .push(outline_quad(quad.rect, shadow.color, shadow_radius, spread));
            } else {
                let corners = self.corners(element, area).unwrap_or_default();
                self.paint_shadow_around(quad, edges, corners);
            }
        }
        // A press that ripples shows as the ripple, over what's there otherwise, rather than as a pressed background.
        let rippling = look.native && self.interaction.ripple.is_some_and(|ripple| std::ptr::eq(ripple.element, element));
        let focused = look.native && self.has_keyboard(element);
        let pressed = look
            .pressed
            .filter(|_| is(self.interaction.pressed, element) && !rippling);
        let focus_background = if focused {
            element.props.get("focus_background").and_then(Color::from_prop)
        } else {
            None
        };
        let shade = if self.shading && pressed.is_none() && focus_background.is_none() {
            self.hover_shade(element, &look)
        } else {
            None
        };
        let hovered = look.hover.filter(|_| is(self.interaction.hovered, element));
        let background = pressed.or(focus_background).or(shade).or(hovered).or(look.background);
        // Corners that aren't all one circle, or a shape it's cut to, are drawn as a picture, background and border together.
        let shape = if look.shaped {
            crate::gradient::BoxShape::of(element, area, self.scale)
        } else {
            None
        };
        let drew_shape = match shape {
            Some(shape) => {
                let border = if look.border_pattern == BorderPattern::Line {
                    let (top, right, bottom, left) = border_sides(element);
                    let widths = [top, right, bottom, left].map(|width| drawn_width(width, self.scale));
                    let colors = ["border_top_color", "border_right_color", "border_bottom_color", "border_left_color"]
                        .map(|name| element.props.get(name).and_then(Color::from_prop).or(look.border_color));
                    (widths.iter().any(|width| *width > 0.0) && colors.iter().any(Option::is_some))
                        .then_some(crate::gradient::ShapeBorder { widths, colors })
                } else {
                    None
                };
                self.paint_shaped_box(area, &shape, background, border);
                true
            }
            None => false,
        };
        if let Some(background) = background.filter(|_| !drew_shape) {
            if background.a > 0 {
                self.quads.push(solid_quad(area, background, radius));
            }
        }
        // A background that only shows through text is drawn with the text instead.
        let through_text = matches!(element.props.get("background_clip"), Some(PropValue::Str(how)) if how.as_str() == "text");
        if look.gradient && !through_text {
            self.paint_gradient(element, area, radius);
        }
        if look.background_image {
            self.paint_background_image(element, area, &look);
        }
        // What's inside goes over its background, from the last.
        for shadow in shadows.iter().rev().filter(|shadow| shadow.inset) {
            let reach = shadow.blur * self.scale * 1.5;
            let spread = (shadow.spread * self.scale).max(0.0);
            let moved =
                |pixels: f32| ((pixels * self.scale).round().clamp(-127.0, 127.0) + 128.0) as u16;
            self.quads.push(Quad {
                rect: area,
                glyph: [
                    (spread * 16.0).min(f32::from(u16::MAX)) as u16,
                    (reach * 16.0).min(f32::from(u16::MAX)) as u16,
                    INSET_MARK,
                    (moved(shadow.x) << 8) | moved(shadow.y),
                ],
                color: shadow.color.premultiplied(),
                radius,
            });
        }
        // A border as wide all around, with a side of another color, is drawn a side at a time.
        let colored_sides = look.border_side_colors
            && !look.border_sides
            && !drew_shape
            && look.border_width > 0.0
            && look.border_pattern == BorderPattern::Line;
        if colored_sides {
            let width = (look.border_width * self.scale).round().max(1.0);
            self.paint_colored_sides(element, area, look.border_color, width, radius);
        }
        if let Some(color) = look.border_color.filter(|_| {
            look.border_width > 0.0
                && !colored_sides
                && !(drew_shape && look.border_pattern == BorderPattern::Line)
        }) {
            let width = (look.border_width * self.scale).round().max(1.0);
            if look.border_pattern == BorderPattern::Line {
                self.quads.push(outline_quad(area, color, radius, width));
            } else if !matches!(
                look.border_pattern,
                BorderPattern::Dashes | BorderPattern::Dots
            ) {
                self.paint_styled_border(area, color, width, look.border_pattern);
            } else {
                self.paint_border_pattern(area, color, width, look.border_pattern);
            }
        }
        if look.border_sides && !(drew_shape && look.border_pattern == BorderPattern::Line) {
            self.paint_border_sides(element, area, look.border_color);
        }
        if look.outlined && is_auto_outline(element) {
            let outline = Outline::of(element);
            self.paint_auto_outline(element, area, radius, outline.color, outline.offset);
        } else if look.outlined {
            let outline = Outline::of(element);
            let width = (outline.width * self.scale).round().max(1.0);
            let out = outline.offset * self.scale + width;
            let around = [
                area[0] - out,
                area[1] - out,
                area[2] + out * 2.0,
                area[3] + out * 2.0,
            ];
            let rounded = if radius > 0.0 { radius + out } else { 0.0 };
            // Dashes or dots, as `outline-style` says, are drawn as a border's are.
            match element.props.get("outline_style") {
                Some(PropValue::Str(how)) if how.as_str() == "dashed" => {
                    self.paint_border_pattern(around, outline.color, width, BorderPattern::Dashes);
                }
                Some(PropValue::Str(how)) if how.as_str() == "dotted" => {
                    self.paint_border_pattern(around, outline.color, width, BorderPattern::Dots);
                }
                _ => self
                    .quads
                    .push(outline_quad(around, outline.color, rounded, width)),
            }
        }
        if look.native {
            self.paint_native(element, area, radius, focused, rippling);
        }
    }

    /// Whether this has the keyboard: it's being typed in, or it shows the focus ring.
    fn has_keyboard(&self, element: &Element) -> bool {
        is(self.interaction.ring, element)
            || self
                .interaction
                .editing
                .is_some_and(|editing| std::ptr::eq(editing.element, element))
    }

    /// What a control in the platform's look draws over its background and border as it's used: the line a text field
    /// has while it's typed in (`focus_line`, `"bottom COLOR WIDTH"` or `"all COLOR WIDTH"`, the color as `Color::to_prop` has
    /// it), a ripple spreading from where it was pressed (`ripple`, its color at full strength), and its text's color while
    /// it's pressed (`pressed_text_color`).
    #[cold]
    #[inline(never)]
    fn paint_native(&mut self, element: &Element, area: [f32; 4], radius: f32, focused: bool, rippling: bool) {
        if focused {
            if let Some(PropValue::Str(line)) = element.props.get("focus_line") {
                let mut words = line.split_whitespace();
                let side = words.next().unwrap_or("all");
                let color = words.next().and_then(color_word);
                let width = words.next().and_then(|width| width.parse::<f32>().ok()).unwrap_or(2.0);
                if let Some(color) = color {
                    let width = (width * self.scale).round().max(1.0);
                    if side == "bottom" {
                        let strip = [area[0], area[1] + area[3] - width, area[2], width];
                        self.border_strip(strip, color, area, radius);
                    } else {
                        self.quads.push(outline_quad(area, color, radius, width));
                    }
                }
            }
        }
        if rippling {
            if let (Some(ripple), Some(color)) = (self.interaction.ripple, element.props.get("ripple").and_then(Color::from_prop)) {
                self.paint_ripple(element, area, ripple, color);
            }
        }
        if is(self.interaction.pressed, element) {
            if let Some(color) = element.props.get("pressed_text_color").and_then(Color::from_prop) {
                self.look.text_color = Some(color);
            }
        }
    }

    /// Material's ripple: a circle that grows from where it was pressed to cover all of what was pressed, moving to its middle as
    /// it does, cut off at its edges and round corners, as Compose draws it (`RippleAnimation`): it starts three tenths of the
    /// longer side across and ends 10 pixels past the corners.
    fn paint_ripple(&mut self, element: &Element, area: [f32; 4], ripple: Ripple<'_>, color: Color) {
        let [left, top, across, down] = area;
        if across <= 0.0 || down <= 0.0 {
            return;
        }
        let spread = ripple.spread.clamp(0.0, 1.0);
        let (pressed_x, pressed_y) = (ripple.x * self.scale, ripple.y * self.scale);
        let (middle_x, middle_y) = (left + across / 2.0, top + down / 2.0);
        let x = pressed_x + (middle_x - pressed_x) * spread;
        let y = pressed_y + (middle_y - pressed_y) * spread;
        let from = across.max(down) * 0.3 / 2.0;
        let to = (across * across + down * down).sqrt() / 2.0 + 10.0 * self.scale;
        let reach = from + (to - from) * spread;
        let alpha = (f32::from(color.a) * ripple.strength.clamp(0.0, 1.0)).round() as u8;
        if alpha == 0 {
            return;
        }
        let circle = solid_quad([x - reach, y - reach, reach * 2.0, reach * 2.0], Color { a: alpha, ..color }, reach);
        match self.corners(element, area) {
            Some(corners) => {
                self.push_round_clip([left, top, left + across, top + down], corners, false);
                self.quads.push(circle);
                self.finish_batch();
                self.rounded.pop();
            }
            None => {
                self.finish_batch();
                let clip = self.clip().intersect(PixelRect {
                    left: left.round() as i32,
                    top: top.round() as i32,
                    right: (left + across).round() as i32,
                    bottom: (top + down).round() as i32,
                });
                self.clips.push(clip);
                self.quads.push(circle);
                self.finish_batch();
                self.clips.pop();
            }
        }
    }

    /// A focus ring a control in the platform's look says it has (`focus_ring`): `"COLOR WIDTH OFFSET"`, a ring that far
    /// outside its edge, and then, if it says, `INNER_COLOR INNER_WIDTH`, a ring just outside its edge, as Fluent's has.
    #[cold]
    #[inline(never)]
    fn paint_native_ring(&mut self, said: &str, edges: [f32; 4]) {
        let [left, top, right, bottom] = edges;
        let area = [left, top, right - left, bottom - top];
        let radius = fitting_radius(self.look.corner_radius * self.scale, area);
        let words: Vec<&str> = said.split_whitespace().collect();
        let number = |at: usize, otherwise: f32| words.get(at).and_then(|word| word.parse::<f32>().ok()).unwrap_or(otherwise);
        let around = |out: f32| [area[0] - out, area[1] - out, area[2] + out * 2.0, area[3] + out * 2.0];
        let rounded = |out: f32| if radius > 0.0 { radius + out } else { 0.0 };
        if let Some(outer) = words.first().copied().and_then(color_word) {
            let width = (number(1, 2.0) * self.scale).round().max(1.0);
            let out = (number(2, 0.0) * self.scale).round() + width;
            self.quads.push(outline_quad(around(out), outer, rounded(out), width));
        }
        if let Some(inner) = words.get(3).copied().and_then(color_word) {
            let width = (number(4, 1.0) * self.scale).round().max(1.0);
            self.quads.push(outline_quad(around(width), inner, rounded(width), width));
        }
    }

    /// A shadow only around what it's behind, as CSS has it, for something that can be seen through: it's kept out of the box,
    /// whose edges are `edges`, with its round corners.
    #[cold]
    #[inline(never)]
    fn paint_shadow_around(&mut self, quad: Quad, edges: [f32; 4], corners: [[f32; 2]; 4]) {
        self.push_round_clip(edges, corners, true);
        self.quads.push(quad);
        self.finish_batch();
        self.rounded.pop();
    }

    /// Draws a box whose corners aren't all one circle, or that's cut to a shape, with its background and its border, as a picture
    /// that's kept.
    #[cold]
    #[inline(never)]
    fn paint_shaped_box(
        &mut self,
        area: [f32; 4],
        shape: &crate::gradient::BoxShape,
        background: Option<Color>,
        border: Option<crate::gradient::ShapeBorder>,
    ) {
        let [left, top, across, down] = area;
        let (across, down) = (across.round(), down.round());
        if across < 1.0 || down < 1.0 || (background.is_none() && border.is_none()) {
            return;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        "shaped".hash(&mut hasher);
        (across as u32, down as u32).hash(&mut hasher);
        shape.hash_into(&mut hasher);
        background.map(Color::to_prop).hash(&mut hasher);
        border
            .map(|border| (border.colors.map(|color| color.map(Color::to_prop)), border.widths.map(f32::to_bits)))
            .hash(&mut hasher);
        let hash = hasher.finish();
        let key = ColorGlyphKey {
            font: u16::MAX - 4,
            glyph: (hash >> 48) as u16,
            size_bits: hash as u32,
            text_color: ((hash >> 16) as u32).to_le_bytes(),
        };
        let made = self.images.color_glyph(key, || {
            crate::gradient::draw_shaped_box(across as u32, down as u32, shape, background, border)
        });
        if let Some(made) = made {
            let clip = self.clip();
            self.place_color_glyph(made, left, top, Color::WHITE, clip);
        }
    }

    /// Draws a border that's two lines, or two colors, as `pattern` says, as a picture that's kept, as borders rarely change.
    #[cold]
    #[inline(never)]
    fn paint_styled_border(
        &mut self,
        area: [f32; 4],
        color: Color,
        width: f32,
        pattern: BorderPattern,
    ) {
        let [left, top, across, down] = area;
        let (across, down) = (across.round(), down.round());
        if across < 1.0 || down < 1.0 {
            return;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        "border".hash(&mut hasher);
        (
            across as u32,
            down as u32,
            width.to_bits(),
            pattern as u8,
            color.to_prop(),
        )
            .hash(&mut hasher);
        let hash = hasher.finish();
        let key = ColorGlyphKey {
            font: u16::MAX - 2,
            glyph: (hash >> 48) as u16,
            size_bits: hash as u32,
            text_color: ((hash >> 16) as u32).to_le_bytes(),
        };
        let made = self.images.color_glyph(key, || {
            crate::gradient::draw_styled_border(across as u32, down as u32, width, color, pattern)
        });
        if let Some(made) = made {
            let clip = self.clip();
            self.place_color_glyph(made, left, top, Color::WHITE, clip);
        }
    }

    /// A border of dashes or dots, as a browser draws them: each side starts and ends with one at its corners, and what's between
    /// them is shared out evenly.
    #[cold]
    #[inline(never)]
    fn paint_border_pattern(
        &mut self,
        area: [f32; 4],
        color: Color,
        width: f32,
        pattern: BorderPattern,
    ) {
        self.paint_pattern_sides(area, color, width, pattern, [true; 4]);
    }

    /// Dashes or dots on the sides of `area` that `sides` says, from the top going around, as a border with only some sides has.
    fn paint_pattern_sides(
        &mut self,
        area: [f32; 4],
        color: Color,
        width: f32,
        pattern: BorderPattern,
        sides: [bool; 4],
    ) {
        let [left, top, across, down] = area;
        // A dot is round, as a browser has it, once it's big enough for that to show.
        let round = if pattern == BorderPattern::Dots && width > 3.0 {
            width / 2.0
        } else {
            0.0
        };
        let piece = if pattern == BorderPattern::Dashes {
            width * 2.0
        } else {
            width
        };
        // Along one side, from `start`, `length` long: where each piece goes, and how long it is.
        let pieces = |length: f32| -> (usize, f32) {
            // As many as fit with about as much between them as they're wide: dashes to the nearest, dots to the fewer.
            let fit = (length + width) / (piece + width);
            let count = if pattern == BorderPattern::Dashes {
                fit.round()
            } else if round == 0.0 {
                fit.floor()
            } else {
                // Dots: as many as leave room between them nearest to how big they are, as Blink has it.
                let fewer = fit.floor().max(1.0);
                let more = fewer + 1.0;
                let room = |count: f32| {
                    if count > 1.0 {
                        (length - piece * count) / (count - 1.0)
                    } else {
                        f32::INFINITY
                    }
                };
                if room(more) <= 0.0 || (room(fewer) - width).abs() < (room(more) - width).abs() {
                    fewer
                } else {
                    more
                }
            }
            .max(1.0) as usize;
            let room = if count > 1 {
                (length - piece * count as f32) / (count - 1) as f32
            } else {
                0.0
            };
            (count, room)
        };
        let (count, room) = pieces(across);
        for at in 0..count {
            // On whole pixels, as a browser has them, with what's between them a pixel more or less.
            let x = (left + at as f32 * (piece + room)).round();
            let x = if at + 1 == count && count > 1 {
                left + across - piece
            } else {
                x
            };
            if sides[0] {
                self.quads
                    .push(solid_quad([x, top, piece, width], color, round));
            }
            if sides[2] {
                self.quads.push(solid_quad(
                    [x, top + down - width, piece, width],
                    color,
                    round,
                ));
            }
        }
        let (count, room) = pieces(down);
        for at in 0..count {
            let y = (top + at as f32 * (piece + room)).round();
            let y = if at + 1 == count && count > 1 {
                top + down - piece
            } else {
                y
            };
            if sides[3] {
                self.quads
                    .push(solid_quad([left, y, width, piece], color, round));
            }
            if sides[1] {
                self.quads.push(solid_quad(
                    [left + across - width, y, width, piece],
                    color,
                    round,
                ));
            }
        }
    }

    /// Draws a background that goes from one color to another, over its plain background, as CSS has `background-image` over
    /// `background-color`. It's drawn into a picture of its size once, and kept.
    #[cold]
    #[inline(never)]
    fn paint_gradient(&mut self, element: &Element, area: [f32; 4], radius: f32) {
        let Some(PropValue::Str(spec)) = element.props.get("background_gradient") else {
            return;
        };
        let [left, top, width, height] = area;
        let (width, height) = (width.round(), height.round());
        if width < 1.0 || height < 1.0 {
            return;
        }
        // As big as `background-size` says, over and over, or as big as the box.
        let tile = match element.props.get("background_size") {
            Some(PropValue::Str(said)) if !matches!(said.trim(), "" | "auto" | "auto auto" | "cover" | "contain" | "100% 100%") => {
                // What's in `em` is the size of its text, as for a picture.
                let em = match element.props.get("font_size") {
                    Some(PropValue::Float(size)) => *size as f32,
                    Some(PropValue::Int(size)) => *size as f32,
                    _ => 16.0,
                };
                Some(background_size(said, (width, height), (width, height), (self.scale, em)))
                    .filter(|&(across, down)| across >= 1.0 && down >= 1.0 && (across < width || down < height))
            }
            _ => None,
        };
        // Where the tiles start, as `background-position` says, as a striped Progress's stripes move along (Mantine).
        let shift = match (tile, element.props.get("background_position")) {
            (Some((across, down)), Some(PropValue::Str(said))) if !said.trim().is_empty() => {
                background_position(said.as_str(), (width - across, height - down), self.scale)
            }
            _ => (0.0, 0.0),
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        "gradient".hash(&mut hasher);
        spec.as_str().hash(&mut hasher);
        (width as u32, height as u32, radius.to_bits()).hash(&mut hasher);
        tile.map(|(across, down)| (across.to_bits(), down.to_bits())).hash(&mut hasher);
        (shift.0.to_bits(), shift.1.to_bits()).hash(&mut hasher);
        let hash = hasher.finish();
        let key = ColorGlyphKey {
            font: u16::MAX - 1,
            glyph: (hash >> 48) as u16,
            size_bits: hash as u32,
            text_color: ((hash >> 16) as u32).to_le_bytes(),
        };
        let scale = self.scale;
        let made = self.images.color_glyph(key, || {
            crate::gradient::draw_gradient_tiled(
                spec.as_str(),
                width as u32,
                height as u32,
                radius,
                scale,
                tile,
                shift,
            )
        });
        if let Some(made) = made {
            let clip = self.clip();
            self.place_color_glyph(made, left, top, Color::WHITE, clip);
        }
    }

    /// Scrollbars as a browser draws them by itself on Windows: a grey track with arrows at its ends and a thumb between them, and
    /// a square where two meet. Each is only there when there's more to see that way.
    #[cold]
    #[inline(never)]
    fn paint_classic_scrollbars(&mut self, rect: Rect, info: ScrollInfo, element: &Element) {
        // A browser's own colors, unless CSS's `scrollbar-color` says its thumb's and its track's, which its arrows and its corner
        // are drawn in then, and its own size, unless `scrollbar-width` says it's thin.
        let said = |name: &str| element.props.get(name).and_then(Color::from_prop);
        let (thumb_said, track_said) = (said("scrollbar_thumb"), said("scrollbar_track"));
        let track = track_said.unwrap_or(Color::hex(0xF1F1F1));
        let thumb = thumb_said.unwrap_or(Color::hex(0xC1C1C1));
        let arrow_color = thumb_said.unwrap_or(Color::hex(0x505050));
        let corner = track_said.unwrap_or(Color::hex(0xDCDCDC));
        let thickness = match element.props.get("scrollbar_size") {
            Some(PropValue::Float(size)) => *size as f32,
            Some(PropValue::Int(size)) => *size as f32,
            _ => CLASSIC_SCROLLBAR,
        };
        if thickness <= 0.0 {
            return;
        }
        let s = self.scale;
        let size = (thickness * s).round();
        let [left, top] = [(rect.x * s).round(), (rect.y * s).round()];
        let [right, bottom] = [
            ((rect.x + rect.width) * s).round(),
            ((rect.y + rect.height) * s).round(),
        ];
        let down = info.content.height > info.view.height + 0.5;
        let across = info.content.width > info.view.width + 0.5;
        // An arrow, as rows that get wider, pointing `way`: 0 up, 1 down, 2 left, 3 right.
        let arrow = |quads: &mut Vec<Quad>, box_left: f32, box_top: f32, way: u8| {
            let (middle_x, middle_y) = (
                box_left + (size / 2.0).floor(),
                box_top + (size / 2.0).floor(),
            );
            for row in 0..4 {
                let half = row as f32 * s;
                let reach = (row as f32 - 2.0) * s;
                let rect = match way {
                    0 => [middle_x - half, middle_y + reach, half * 2.0 + s, s],
                    1 => [middle_x - half, middle_y - reach, half * 2.0 + s, s],
                    2 => [middle_x + reach, middle_y - half, s, half * 2.0 + s],
                    _ => [middle_x - reach, middle_y - half, s, half * 2.0 + s],
                };
                quads.push(solid_quad(rect, arrow_color, 0.0));
            }
        };
        let thumb_along = |view: f32, content: f32, moved: f32, track: f32| {
            let room = (track - size * 2.0).max(0.0);
            let length = (room * view / content).max(size).min(room);
            let share = if content > view {
                moved / (content - view)
            } else {
                0.0
            };
            (size + (room - length) * share.clamp(0.0, 1.0), length)
        };
        // Down the left side where text goes from right to left, as a browser has it, with the one along the bottom after it.
        let on_left = matches!(element.props.get("scrollbar_left"), Some(PropValue::Bool(true)));
        if down {
            let bar_left = if on_left { left } else { right - size };
            let length = bottom - top - if across { size } else { 0.0 };
            self.quads
                .push(solid_quad([bar_left, top, size, length], track, 0.0));
            arrow(self.quads, bar_left, top, 0);
            arrow(self.quads, bar_left, top + length - size, 1);
            let (start, along) = thumb_along(
                info.view.height * s,
                info.content.height * s,
                info.y * s,
                length,
            );
            self.quads.push(solid_quad(
                [bar_left + 2.0 * s, top + start, size - 4.0 * s, along],
                thumb,
                0.0,
            ));
        }
        if across {
            let bar_top = bottom - size;
            let length = right - left - if down { size } else { 0.0 };
            let left = if down && on_left { left + size } else { left };
            self.quads
                .push(solid_quad([left, bar_top, length, size], track, 0.0));
            arrow(self.quads, left, bar_top, 2);
            arrow(self.quads, left + length - size, bar_top, 3);
            let (start, along) = thumb_along(
                info.view.width * s,
                info.content.width * s,
                info.x * s,
                length,
            );
            self.quads.push(solid_quad(
                [left + start, bar_top + 2.0 * s, along, size - 4.0 * s],
                thumb,
                0.0,
            ));
        }
        if down && across {
            self.quads.push(solid_quad(
                [if on_left { left } else { right - size }, bottom - size, size, size],
                corner,
                0.0,
            ));
        }
    }

    /// Draws a border on each side that has one of its own, as a line along that side.
    #[cold]
    #[inline(never)]
    fn paint_border_sides(&mut self, element: &Element, area: [f32; 4], usual: Option<Color>) {
        let [left, top, width, height] = area;
        let radius = fitting_radius(self.look.corner_radius * self.scale, area);
        let right = left + width;
        let bottom = top + height;
        for (width_name, color_name, side) in [
            ("border_top_width", "border_top_color", 0),
            ("border_right_width", "border_right_color", 1),
            ("border_bottom_width", "border_bottom_color", 2),
            ("border_left_width", "border_left_color", 3),
        ] {
            let thick = match element.props.get(width_name) {
                Some(PropValue::Float(thick)) => *thick as f32,
                Some(PropValue::Int(thick)) => *thick as f32,
                _ => continue,
            };
            if thick <= 0.0 {
                continue;
            }
            let thick = (thick * self.scale).round().max(1.0);
            let color = element
                .props
                .get(color_name)
                .and_then(Color::from_prop)
                .or(usual);
            let Some(color) = color.filter(|color| color.a > 0) else {
                continue;
            };
            // A strip of the side from `from` to `to` in from its outer edge.
            let strip = |from: f32, to: f32| match side {
                0 => [left, top + from, width, to - from],
                1 => [right - to, top, to - from, height],
                2 => [left, bottom - to, width, to - from],
                _ => [left + from, top, to - from, height],
            };
            let top_or_left = side == 0 || side == 3;
            let dark = crate::gradient::darker(color);
            // How a side that's two colors or two lines is drawn, as a browser has it: its outer half and inner half.
            let (outer, inner) = match self.look.border_pattern {
                BorderPattern::Groove => (
                    Some(if top_or_left { dark } else { color }),
                    Some(if top_or_left { color } else { dark }),
                ),
                BorderPattern::Ridge => (
                    Some(if top_or_left { color } else { dark }),
                    Some(if top_or_left { dark } else { color }),
                ),
                BorderPattern::Inset => {
                    let shade = if top_or_left { dark } else { color };
                    (Some(shade), Some(shade))
                }
                BorderPattern::Outset => {
                    let shade = if top_or_left { color } else { dark };
                    (Some(shade), Some(shade))
                }
                // Dashes or dots along the side, as wide as it is.
                pattern @ (BorderPattern::Dashes | BorderPattern::Dots) => {
                    let mut only = [false; 4];
                    only[side] = true;
                    self.paint_pattern_sides(area, color, thick, pattern, only);
                    continue;
                }
                BorderPattern::Double if thick >= 3.0 => {
                    let third = ((thick + 1.0) / 3.0).floor().max(1.0);
                    self.border_strip(strip(0.0, third), color, area, radius);
                    self.border_strip(strip(thick - third, thick), color, area, radius);
                    continue;
                }
                _ => {
                    self.border_strip(strip(0.0, thick), color, area, radius);
                    continue;
                }
            };
            let half = (thick / 2.0).floor().max(1.0).min(thick);
            if let Some(outer) = outer {
                self.border_strip(strip(0.0, half), outer, area, radius);
            }
            if let Some(inner) = inner.filter(|_| half < thick) {
                self.border_strip(strip(half, thick), inner, area, radius);
            }
        }
    }

    /// Draws a border that's as wide all around with each side in its own color, as CSS has it: a side has what's between the
    /// lines from the box's middle to its corners, so a round box's sides are quarters of its ring. That's cut out of the ring in
    /// strips, as a clip is a rectangle.
    #[cold]
    #[inline(never)]
    fn paint_colored_sides(&mut self, element: &Element, area: [f32; 4], usual: Option<Color>, width: f32, radius: f32) {
        const STRIPS: usize = 12;
        let [left, top, across, down] = area;
        if across <= 0.0 || down <= 0.0 {
            return;
        }
        for (name, side) in [("border_top_color", 0), ("border_right_color", 1), ("border_bottom_color", 2), ("border_left_color", 3)] {
            let color = element.props.get(name).and_then(Color::from_prop).or(usual);
            let Some(color) = color.filter(|color| color.a > 0) else {
                continue;
            };
            self.finish_batch();
            for strip in 0..STRIPS {
                // How far in from this side the strip is, from its edge to the middle, and how much of the side it has there.
                let (near, far) = (strip as f32 / STRIPS as f32, (strip + 1) as f32 / STRIPS as f32);
                let mid = (near + far) / 2.0;
                let [x0, y0, x1, y1] = match side {
                    0 => [left + mid * across / 2.0, top + near * down / 2.0, left + across - mid * across / 2.0, top + far * down / 2.0],
                    2 => [left + mid * across / 2.0, top + down - far * down / 2.0, left + across - mid * across / 2.0, top + down - near * down / 2.0],
                    1 => [left + across - far * across / 2.0, top + mid * down / 2.0, left + across - near * across / 2.0, top + down - mid * down / 2.0],
                    _ => [left + near * across / 2.0, top + mid * down / 2.0, left + far * across / 2.0, top + down - mid * down / 2.0],
                };
                let clip = self.clip().intersect(PixelRect {
                    left: x0.round() as i32,
                    top: y0.round() as i32,
                    right: x1.round() as i32,
                    bottom: y1.round() as i32,
                });
                self.clips.push(clip);
                self.quads.push(outline_quad(area, color, radius, width));
                self.finish_batch();
                self.clips.pop();
            }
        }
    }

    /// Draws part of a border that's only on some sides. With rounded corners, its outer edge follows them, as CSS has it, which
    /// is the whole rounded box, as much of it as is in `strip`.
    fn border_strip(&mut self, strip: [f32; 4], color: Color, area: [f32; 4], radius: f32) {
        if radius <= 0.0 {
            self.quads.push(solid_quad(strip, color, 0.0));
            return;
        }
        self.finish_batch();
        let clip = self.clip().intersect(PixelRect {
            left: strip[0].round() as i32,
            top: strip[1].round() as i32,
            right: (strip[0] + strip[2]).round() as i32,
            bottom: (strip[1] + strip[3]).round() as i32,
        });
        self.clips.push(clip);
        self.quads.push(solid_quad(area, color, radius));
        self.finish_batch();
        self.clips.pop();
    }

    /// Draws the ring around what has the keyboard.
    #[cold]
    #[inline(never)]
    fn paint_ring(&mut self, element: &Element, edges: [f32; 4]) {
        if let Some(PropValue::Str(said)) = element.props.get("focus_ring") {
            self.paint_native_ring(said, edges);
            return;
        }
        let [left, top, right, bottom] = edges;
        let radius = fitting_radius(
            self.look.corner_radius * self.scale,
            [left, top, right - left, bottom - top],
        );
        let width = (FOCUS_RING_WIDTH * self.scale).round();
        if self.rasterizer.has_system() {
            // A page's, as a browser draws it: its `:focus-visible { outline: auto 1px -webkit-focus-ring-color }`, in the page's
            // own `outline-color` and `outline-offset` where it gives them (shadcn's `outline-ring/50` on everything).
            let outline = Outline::of(element);
            let color = if element.props.get("outline_color").is_some() {
                outline.color
            } else {
                FOCUS_RING_COLOR
            };
            self.paint_auto_outline(element, [left, top, right - left, bottom - top], radius, color, outline.offset);
            return;
        }
        self.quads.push(outline_quad(
            [
                left - width,
                top - width,
                right - left + width * 2.0,
                bottom - top + width * 2.0,
            ],
            FOCUS_RING,
            radius + width,
            width,
        ));
    }

    /// Draws `outline-style: auto` as Chromium does (`OutlinePainter::PaintFocusRing`): a ring two pixels wide in the outline's
    /// color, half outside the edge `outline-offset` away and half inside it (a pixel further in on a box with a border, which it
    /// then covers), with a white ring a pixel wide around that, whatever `outline-width` says. Its corners follow the box's, and
    /// are a little round on a square box.
    fn paint_auto_outline(&mut self, element: &Element, area: [f32; 4], radius: f32, color: Color, offset: f32) {
        let (top, right, bottom, left) = border_sides(element);
        let inside_border = if top.min(right).min(bottom).min(left) >= 1.0 { 1.0 } else { 0.0 };
        let scale = self.scale;
        let inner = (2.0 * scale).round().max(1.0);
        let outer = scale.round().max(1.0);
        // How far out the inner ring's outer edge is.
        let out = ((offset - inside_border) * scale).round() + inner / 2.0;
        let corner = if radius > 0.0 { (radius + out).max(0.0) } else { (3.0 * scale).round() };
        let around = |out: f32| [area[0] - out, area[1] - out, area[2] + out * 2.0, area[3] + out * 2.0];
        self.quads.push(outline_quad(around(out + outer), Color::WHITE, corner + outer, outer));
        self.quads.push(outline_quad(around(out), color, corner, inner));
    }

    /// Draws what's in a text input or an image.
    #[cold]
    #[inline(never)]
    fn paint_contents(&mut self, element: &Element, rect: Rect, edges: [f32; 4]) {
        let look = self.look;
        let [left, top, right, bottom] = edges;
        if element.role == Role::Image {
            // A picture goes inside its padding, as what's in anything does, which is inside its border too, as an `<img>` is on
            // a page: a page's border is part of its padding by the time it's here.
            let border = if look.border_color.is_some() && !look.border_sides {
                (look.border_width * self.scale).round().max(0.0)
            } else {
                0.0
            };
            let s = self.scale;
            let inner = [
                left + look.padding_left * s,
                top + look.padding_top * s,
                (right - left - (look.padding_left + look.padding_right) * s).max(0.0),
                (bottom - top - (look.padding_top + look.padding_bottom) * s).max(0.0),
            ];
            let inner_look = Look {
                corner_radius: (look.corner_radius - border / s).max(0.0),
                ..look
            };
            self.paint_image(element, inner, &inner_look);
        } else if element.props.get_symbol(self.names.rows).is_some() {
            self.paint_text_area(element, rect, &look);
        } else {
            self.paint_input(element, rect, &look, edges);
        }
    }

    /// The color of something whose hover color is partway through coming or going.
    fn hover_shade(&self, element: &Element, look: &Look) -> Option<Color> {
        let hover = look.hover?;
        let base = look.background.unwrap_or(Color { a: 0, ..hover });
        if is(self.interaction.hovered, element) {
            return Some(base.mix(hover, self.interaction.hover_amount));
        }
        match self.interaction.leaving {
            Some((leaving, amount)) if std::ptr::eq(leaving, element) => {
                Some(base.mix(hover, amount))
            }
            _ => None,
        }
    }

    /// Fades everything something see-through has drawn, including what's inside it.
    #[cold]
    #[inline(never)]
    fn finish_fade(&mut self, element: &Element) {
        let address = std::ptr::from_ref(element) as usize;
        if self
            .fades
            .last()
            .is_some_and(|fade| fade.element == address)
        {
            let fade = self.fades.pop().expect("there is one");
            self.fading -= 1;
            // What overlaps is put down as one, faded as a whole, unless it's inside something faded itself, which fades what's
            // in it part by part, as before.
            if self.fading == 0
                && fade.opacity > 0.0
                && parts_overlap(&self.quads[fade.first_quad..])
                && self.make_layer(fade.first_quad, fade.opacity)
            {
                return;
            }
            for quad in &mut self.quads[fade.first_quad..] {
                for part in &mut quad.color {
                    *part = (f32::from(*part) * fade.opacity).round() as u8;
                }
            }
        }
    }

    /// Makes the quads from `first_quad` on a layer of their own, faded as one: see `Layer`. They're a run of batches by
    /// themselves, the one they start in split there if it has quads before them.
    #[cold]
    #[inline(never)]
    fn make_layer(&mut self, first_quad: usize, opacity: f32) -> bool {
        self.finish_batch();
        let first_quad = first_quad as u32;
        let mut first = self.batches.len();
        while first > 0 && self.batches[first - 1].first + self.batches[first - 1].count > first_quad {
            first -= 1;
        }
        if first < self.batches.len() && self.batches[first].first < first_quad {
            let whole = self.batches[first];
            let before = first_quad - whole.first;
            self.batches[first].count = before;
            self.batches.insert(
                first + 1,
                Batch {
                    first: first_quad,
                    count: whole.count - before,
                    ..whole
                },
            );
            first += 1;
        }
        let end = self.batches.len();
        if first >= end {
            return false;
        }
        // Where its quads are, inside the clips they're drawn in, or the clips themselves for what's turned.
        let mut bounds: Option<PixelRect> = None;
        let mut grow = |area: PixelRect| {
            bounds = Some(bounds.map_or(area, |was| PixelRect {
                left: was.left.min(area.left),
                top: was.top.min(area.top),
                right: was.right.max(area.right),
                bottom: was.bottom.max(area.bottom),
            }));
        };
        for batch in &self.batches[first..end] {
            if batch.transform != 0 {
                grow(batch.clip);
                continue;
            }
            for quad in &self.quads[batch.first as usize..(batch.first + batch.count) as usize] {
                let [x, y, w, h] = quad.rect;
                let area = PixelRect {
                    left: (x.floor() as i32 - 1).max(batch.clip.left),
                    top: (y.floor() as i32 - 1).max(batch.clip.top),
                    right: ((x + w).ceil() as i32 + 1).min(batch.clip.right),
                    bottom: ((y + h).ceil() as i32 + 1).min(batch.clip.bottom),
                };
                if area.left < area.right && area.top < area.bottom {
                    grow(area);
                }
            }
        }
        let Some(bounds) = bounds else {
            return false;
        };
        self.layers.push(Layer {
            first: first as u32,
            end: end as u32,
            opacity,
            bounds,
        });
        true
    }

    /// Draws a little text on a dark background, below the pointer, staying inside the window.
    #[cold]
    #[inline(never)]
    fn paint_tooltip(&mut self, text: &str, x: f32, y: f32, window: PixelRect) {
        let mut style = TextStyle::new(TOOLTIP_TEXT_SIZE);
        style = style.scaled(self.scale);
        let limit = TOOLTIP_WIDEST * self.scale;
        let padding = (TOOLTIP_PADDING * self.scale).round();
        let size = self.text.measure(text, style, limit);
        let (width, height) = (
            size.width.ceil() + padding * 2.0,
            size.height.ceil() + padding * 2.0,
        );
        let (right, bottom) = (window.right as f32, window.bottom as f32);
        let left = x.min(right - width).max(0.0).round();
        // Below the pointer's arrow, or above it if there's no room for that.
        let below = y + TOOLTIP_GAP * self.scale;
        let above = y - height - TOOLTIP_GAP * self.scale / 2.0;
        let top = if below + height <= bottom {
            below
        } else {
            above.max(0.0)
        }
        .round();
        self.quads.push(solid_quad(
            [left, top, width, height],
            TOOLTIP_BACKGROUND,
            (4.0 * self.scale).round(),
        ));
        self.text
            .layout_visible_into(text, style, limit, (0.0, f32::MAX), self.glyphs);
        self.place_glyphs(style, left + padding, top + padding, Color::WHITE);
    }

    /// Draws what a debugger has asked for, on top of everything.
    #[cold]
    #[inline(never)]
    fn paint_overlay(&mut self, overlay: &Overlay, window: PixelRect) {
        for one in &overlay.boxes {
            self.quads.push(if one.outline > 0.0 {
                outline_quad(one.rect, one.color, one.radius, one.outline)
            } else {
                solid_quad(one.rect, one.color, one.radius)
            });
        }
        let (right, bottom) = (window.right as f32, window.bottom as f32);
        for text in &overlay.texts {
            let style = TextStyle::new(text.size).scaled(self.scale);
            let limit = text.widest * self.scale;
            let padding = (3.0 * self.scale).round();
            let size = self.text.measure(&text.text, style, limit);
            let (width, height) = (
                size.width.ceil() + padding * 2.0,
                size.height.ceil() + padding * 2.0,
            );
            let left = text.left.min(right - width).max(0.0).round();
            let top = text.top.min(bottom - height).max(0.0).round();
            if text.background.a > 0 {
                self.quads.push(solid_quad(
                    [left, top, width, height],
                    text.background,
                    (3.0 * self.scale).round(),
                ));
            }
            self.text
                .layout_visible_into(&text.text, style, limit, (0.0, f32::MAX), self.glyphs);
            self.place_glyphs(style, left + padding, top + padding, text.color);
        }
    }

    /// Draws a shape, which is kept as a picture at this size until nothing's shown it for a while.
    #[cold]
    #[inline(never)]
    fn paint_shape(&mut self, element: &Element, layers: &str, area: [f32; 4], look: &Look) {
        let [left, top, width, height] = area;
        let (width, height) = (width.round(), height.round());
        if width < 1.0 || height < 1.0 {
            return;
        }
        let current = look.text_color.unwrap_or(self.around_color());
        let hash = self.svg_hash.key(element, layers, width, height, current);
        let key = ColorGlyphKey {
            // No font can be this one, as there can't be that many.
            font: u16::MAX,
            glyph: (hash >> 48) as u16,
            size_bits: hash as u32,
            text_color: ((hash >> 16) as u32).to_le_bytes(),
        };
        let (rasterizer, images) = (&mut *self.rasterizer, &mut *self.images);
        let place = [
            left.round() as i32,
            top.round() as i32,
            width as i32,
            height as i32,
        ];
        let made = images.shape_glyph(key, place, || {
            crate::shape::draw_shape(element, layers, (width, height), current, rasterizer)
        });
        if let Some(made) = made {
            let clip = self.clip();
            self.place_color_glyph(made, left, top, Color::WHITE, clip);
        }
    }

    /// Draws the picture behind something, as CSS's `background-image` has it: as big as `background_size` says, where
    /// `background_position` says in what's inside its border, and again and again to fill all of it, out to its border's
    /// edge, unless `background_repeat` says not.
    #[cold]
    #[inline(never)]
    fn paint_background_image(&mut self, element: &Element, area: [f32; 4], look: &Look) {
        let text = |name: &str| match element.props.get(name) {
            Some(PropValue::Str(text)) => text.as_str(),
            _ => "",
        };
        let source = match element.props.get("background_image") {
            Some(PropValue::Str(source)) if !source.is_empty() => source,
            _ => return,
        };
        let Some(natural) = self.images.natural_size(source) else {
            return;
        };
        let scale = self.scale;
        let side = |name: &str| {
            let thick = match element.props.get(name) {
                Some(PropValue::Float(thick)) => *thick as f32,
                Some(PropValue::Int(thick)) => *thick as f32,
                _ => look.border_width,
            };
            (thick * scale).round().max(0.0)
        };
        let (top, right, bottom, left) = if look.border_sides {
            (
                side("border_top_width"),
                side("border_right_width"),
                side("border_bottom_width"),
                side("border_left_width"),
            )
        } else {
            let all = if look.border_color.is_some() {
                (look.border_width * scale).round().max(0.0)
            } else {
                0.0
            };
            (all, all, all, all)
        };
        // Where it's placed from is inside the border.
        let origin = [
            area[0] + left,
            area[1] + top,
            area[2] - left - right,
            area[3] - top - bottom,
        ];
        if origin[2] <= 0.0 || origin[3] <= 0.0 {
            return;
        }
        let (natural_width, natural_height) = (natural.0 as f32 * scale, natural.1 as f32 * scale);
        let mut sized = text("background_size");
        // What only says what shape it is is as big as fits, as CSS has it.
        if self.images.shape_only(source) && matches!(sized.trim(), "" | "auto" | "auto auto") {
            sized = "contain";
        }
        // What's in `em` is the size of its text.
        let em = match element.props.get("font_size") {
            Some(PropValue::Float(size)) => *size as f32,
            Some(PropValue::Int(size)) => *size as f32,
            _ => 16.0,
        };
        let (width, height) = background_size(
            sized,
            (natural_width, natural_height),
            (origin[2], origin[3]),
            (scale, em),
        );
        let (width, height) = (width.round().max(1.0), height.round().max(1.0));
        let (x, y) = background_position(
            text("background_position"),
            (origin[2] - width, origin[3] - height),
            scale,
        );
        let (x, y) = ((origin[0] + x).round(), (origin[1] + y).round());
        let repeat = text("background_repeat");
        let mut words = repeat.split_whitespace();
        let (across, down) = match (words.next(), words.next()) {
            (Some("repeat-x"), _) => (true, false),
            (Some("repeat-y"), _) => (false, true),
            (Some(first), Some(second)) => (first != "no-repeat", second != "no-repeat"),
            (Some("no-repeat"), None) => (false, false),
            _ => (true, true),
        };
        let edge = [area[0], area[1], area[0] + area[2], area[1] + area[3]];
        let first_x = if across {
            x - ((x - edge[0]) / width).ceil() * width
        } else {
            x
        };
        let first_y = if down {
            y - ((y - edge[1]) / height).ceil() * height
        } else {
            y
        };
        let Some(shown) = self.images.request(source, (width as u32, height as u32)) else {
            return;
        };
        self.finish_batch();
        let first = self.quads.len();
        // So many that it couldn't be seen as anything but a color is left at that.
        const MOST: usize = 16384;
        let mut tile_y = first_y;
        'rows: while tile_y < edge[3] {
            let mut tile_x = first_x;
            while tile_x < edge[2] {
                if tile_x + width > edge[0] && tile_y + height > edge[1] {
                    self.quads.push(Quad {
                        rect: [tile_x, tile_y, width, height],
                        glyph: [0, 0, 65535, 65535],
                        color: Color::WHITE.premultiplied(),
                        radius: 0.0,
                    });
                    if self.quads.len() - first >= MOST {
                        break 'rows;
                    }
                }
                if !across {
                    break;
                }
                tile_x += width;
            }
            if !down {
                break;
            }
            tile_y += height;
        }
        if self.quads.len() == first {
            return;
        }
        let clip = self.clip().intersect(PixelRect {
            left: edge[0].floor() as i32,
            top: edge[1].floor() as i32,
            right: edge[2].ceil() as i32,
            bottom: edge[3].ceil() as i32,
        });
        self.batches.push(Batch {
            first: first as u32,
            count: (self.quads.len() - first) as u32,
            clip: self.clip_on_screen(clip),
            image: shown.id,
            transform: self.transform,
            filter: self.filter,
            round: self.round(),
        });
        self.batch_start = self.quads.len();
    }

    /// Draws an image in this area, in physical pixels, once it's been read.
    #[cold]
    #[inline(never)]
    fn paint_image(&mut self, element: &Element, area: [f32; 4], look: &Look) {
        static SOURCE: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("src"));
        static FIT: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("fit"));
        if let Some(layers) = crate::shape::layers_of(element) {
            self.paint_shape(element, layers, area, look);
            return;
        }
        let Some(PropValue::Str(source)) = element.props.get_symbol(*SOURCE) else {
            return;
        };
        let Some(natural) = self.images.natural_size(source) else {
            return;
        };
        let (natural_width, natural_height) = (natural.0 as f32, natural.1 as f32);
        let [left, top, width, height] = area;
        if width < 1.0 || height < 1.0 {
            return;
        }
        let fit = match element.props.get_symbol(*FIT) {
            Some(PropValue::Str(fit)) => fit.as_str(),
            _ => "contain",
        };
        // The part of the image to show, and where to show it, as parts of each.
        let across = width / natural_width;
        let down = height / natural_height;
        let (place, part, target) = match fit {
            "fill" => (area, [0.0, 0.0, 1.0, 1.0], (width, height)),
            "cover" => {
                let scale = across.max(down);
                let (seen_x, seen_y) = (across / scale, down / scale);
                (
                    area,
                    [(1.0 - seen_x) / 2.0, (1.0 - seen_y) / 2.0, seen_x, seen_y],
                    (natural_width * scale, natural_height * scale),
                )
            }
            _ => {
                let scale = across.min(down);
                let (shown_x, shown_y) = (
                    (natural_width * scale).round().max(1.0),
                    (natural_height * scale).round().max(1.0),
                );
                (
                    [
                        left + ((width - shown_x) / 2.0).round(),
                        top + ((height - shown_y) / 2.0).round(),
                        shown_x,
                        shown_y,
                    ],
                    [0.0, 0.0, 1.0, 1.0],
                    (shown_x, shown_y),
                )
            }
        };
        let target = (target.0.ceil() as u32, target.1.ceil() as u32);
        let Some(shown) = self.images.request(source, target) else {
            return;
        };
        self.finish_batch();
        let to_part = |value: f32| (value * 65535.0).round().clamp(0.0, 65535.0) as u16;
        let radius = look.corner_radius * self.scale;
        self.quads.push(Quad {
            rect: place,
            glyph: [
                to_part(part[0]),
                to_part(part[1]),
                to_part(part[2]).max(1),
                to_part(part[3]).max(1),
            ],
            color: Color::WHITE.premultiplied(),
            radius,
        });
        self.batches.push(Batch {
            first: self.batch_start as u32,
            count: 1,
            clip: self.clip_on_screen(self.clip()),
            image: shown.id,
            transform: self.transform,
            filter: self.filter,
            round: self.round(),
        });
        self.batch_start = self.quads.len();
    }

    /// Draws a text input with several lines, cut off at its padding.
    #[cold]
    #[inline(never)]
    fn paint_text_area(&mut self, element: &Element, rect: Rect, look: &Look) {
        let mut style = TextStyle::from_element(element);
        style = style.scaled(self.scale);
        let content = content_box_of(look, rect, self.scale);
        let editing = self
            .interaction
            .editing
            .filter(|editing| std::ptr::eq(editing.element, element));
        let lines = editing.and_then(|editing| editing.lines);
        let scroll = lines.map_or_else(
            || match self.interaction.resting_area {
                Some((area, scroll)) if std::ptr::eq(area, element) => scroll,
                _ => 0.0,
            },
            |lines| lines.scroll,
        );
        let value = input_value(element);
        let hint = match element.props.get_symbol(self.names.placeholder) {
            Some(PropValue::Str(hint)) if value.is_empty() => hint.as_str(),
            _ => "",
        };
        let shown = if value.is_empty() { hint } else { value };
        if value.is_empty() && placeholder_italic(element) {
            style.italic = true;
        }
        let color = look.text_color.unwrap_or(self.around_color());
        let shown_color = if value.is_empty() {
            placeholder_color(element)
        } else {
            color
        };
        let line_height = self.text.line_height(style);
        let top = content.top - scroll;

        self.finish_batch();
        let clip = self.clip().intersect(PixelRect {
            left: content.left.floor() as i32,
            top: content.top.floor() as i32,
            right: (content.left + content.width).ceil() as i32,
            bottom: (content.top + content.height).ceil() as i32,
        });
        self.clips.push(clip);
        if let (Some(editing), Some(lines)) = (editing, lines) {
            if !editing.selection.is_empty() {
                self.paint_area_selection(
                    editing.selection,
                    lines.selection,
                    content.left,
                    top,
                    line_height,
                );
            }
        }
        self.text.layout_visible_into(
            shown,
            style,
            content.width,
            (scroll, scroll + content.height),
            self.glyphs,
        );
        self.place_glyphs(style, content.left, top, shown_color);
        if let (Some(editing), Some(lines)) = (editing, lines) {
            if let Some(caret) = editing.caret {
                // At the far right, it has to stay inside what's cut off.
                let thickness = self.scale.round().max(1.0);
                let far = (content.left + content.width - thickness).floor();
                let x = (content.left + caret).round().min(far);
                let y = top + lines.caret as f32 * line_height;
                let (down, tall) = self.text.caret_span(style);
                self.caret_at = Some(self.quads.len());
                self.quads
                    .push(solid_quad([x, y + down, thickness, tall], color, 0.0));
            }
        }
        self.finish_batch();
        self.clips.pop();
    }

    /// Draws a text input's value, with its selection and caret, cut off at its padding.
    #[cold]
    #[inline(never)]
    fn paint_input(&mut self, element: &Element, rect: Rect, look: &Look, outer: [f32; 4]) {
        let mut style = TextStyle::from_element(element);
        style = style.scaled(self.scale);
        let content = content_box_of(look, rect, self.scale);
        let editing = self
            .interaction
            .editing
            .filter(|editing| std::ptr::eq(editing.element, element));
        let mask = password_mask(element);
        let value = mask.as_deref().unwrap_or_else(|| input_value(element));
        let hint = match element.props.get_symbol(self.names.placeholder) {
            Some(PropValue::Str(hint)) if value.is_empty() => hint.as_str(),
            _ => "",
        };
        let shown_text = if value.is_empty() { hint } else { value };
        if value.is_empty() && placeholder_italic(element) {
            style.italic = true;
        }
        let scroll = match editing {
            Some(editing) => editing.scroll,
            // Too long to fit, what shows is how it starts, which is at the right if it reads that way.
            None if self.text.is_right_to_left(shown_text, style) => {
                (self.text.line_width(shown_text, style) - content.width).max(0.0)
            }
            None => 0.0,
        };
        // Right-to-left text starts from the right, for as long as it fits.
        let indent = input_indent(self.text, element, shown_text, style, content.width);
        let left = content.left + indent - scroll;
        let color = look.text_color.unwrap_or(self.around_color());
        let height = self.text.line_height(style);
        // With `justify="center"`, its line of text is in the middle of the input, up and down, as on a page. It's at the top otherwise.
        let centered = matches!(element.props.get("justify"), Some(PropValue::Str(how)) if how.as_str() == "center");
        let top = if centered {
            (content.top + ((content.height - height) / 2.0).max(0.0)).round()
        } else {
            content.top
        };

        self.finish_batch();
        let clip = self.clip().intersect(PixelRect {
            left: content.left.floor() as i32,
            top: outer[1] as i32,
            right: (content.left + content.width).ceil() as i32,
            bottom: outer[3] as i32,
        });
        self.clips.push(clip);

        if let Some(editing) = editing.filter(|editing| !editing.selection.is_empty()) {
            self.paint_input_selection(editing.selection, left, top, height);
        }
        let shown_color = if value.is_empty() {
            placeholder_color(element)
        } else {
            color
        };
        // Only what's inside the input is placed, which matters for very long values.
        self.text.layout_across_into(
            shown_text,
            style,
            (scroll - indent, scroll - indent + content.width),
            self.glyphs,
        );
        self.place_glyphs(style, left, top, shown_color);
        if let Some(caret) = editing.and_then(|editing| editing.caret) {
            let x = (left + caret).round();
            let (down, tall) = self.text.caret_span(style);
            self.caret_at = Some(self.quads.len());
            self.quads.push(solid_quad(
                [x, top + down, self.scale.round().max(1.0), tall],
                color,
                0.0,
            ));
        }

        self.finish_batch();
        self.clips.pop();
    }

    /// How far along a pen goes before which quarter of a pixel it's in is worked out. Glyphs drawn the way the system draws
    /// them go to the nearest quarter, as a browser has them, and rsui's own go to the quarter before.
    fn pen_rounding(&self) -> f32 {
        if self.rasterizer.has_system() {
            0.125
        } else {
            0.0
        }
    }

    /// Colors each glyph from `first` on as its element's gradient is where that glyph is, if its background only shows through
    /// its text, as `background-clip: text` has it.
    #[cold]
    #[inline(never)]
    fn color_glyphs_by_background(&mut self, element: &Element, rect: Rect, first: usize) {
        if !matches!(element.props.get("background_clip"), Some(PropValue::Str(how)) if how.as_str() == "text")
        {
            return;
        }
        let Some(PropValue::Str(spec)) = element.props.get("background_gradient") else {
            return;
        };
        let s = self.scale;
        let (left, top, w, h) = (
            rect.x * s,
            rect.y * s,
            (rect.width * s).round(),
            (rect.height * s).round(),
        );
        for quad in &mut self.quads[first..] {
            let [x, y, across, down] = quad.rect;
            let place = (x + across / 2.0 - left, y + down / 2.0 - top);
            if let Some(color) =
                crate::gradient::color_at_place(spec.as_str(), (w, h), s, place.0, place.1)
            {
                // What the glyph covers is in the gradient's color where it is, whatever color the text says, which is usually none.
                let to = |value: f32| (value * 255.0).round().clamp(0.0, 255.0) as u8;
                quad.color = [to(color[0]), to(color[1]), to(color[2]), to(color[3])];
            }
        }
    }

    /// Draws text's shadow, as CSS's `text-shadow` says, under where the text that's been laid out goes: the text again, moved and
    /// in the shadow's color, or for a soft one, faint copies of it spread around where it would be, which is near enough to blurred.
    #[cold]
    #[inline(never)]
    fn place_text_shadow(&mut self, element: &Element, style: TextStyle, left: f32, top: f32) {
        let Some(PropValue::Str(said)) = element.props.get("text_shadow") else {
            return;
        };
        let mut parts = said.split_whitespace();
        let mut number = || parts.next().and_then(|part| part.parse::<f32>().ok());
        let (Some(x), Some(y), Some(blur)) = (number(), number(), number()) else {
            return;
        };
        let Some(color) = parts.next().and_then(|part| part.parse::<u32>().ok()) else {
            return;
        };
        let [r, g, b, a] = color.to_be_bytes();
        let (x, y, blur) = (x * self.scale, y * self.scale, blur * self.scale);
        if blur < 1.0 {
            self.place_glyphs(style, left + x, top + y, Color::rgba(r, g, b, a));
            return;
        }
        // A ring of copies as far out as half the blur, and one in the middle, which together are about as dark as the shadow.
        const AROUND: usize = 8;
        let faint = Color::rgba(
            r,
            g,
            b,
            ((f32::from(a) * 2.0 / (AROUND + 1) as f32).round() as u8).max(1),
        );
        self.place_glyphs(style, left + x, top + y, faint);
        for step in 0..AROUND {
            let (sin, cos) = (step as f32 / AROUND as f32 * std::f32::consts::TAU).sin_cos();
            let reach = blur / 2.0;
            self.place_glyphs(style, left + x + cos * reach, top + y + sin * reach, faint);
        }
    }

    /// Turns what was laid out in `self.glyphs` into quads.
    #[inline(always)]
    fn place_glyphs(&mut self, style: TextStyle, left: f32, top: f32, color: Color) {
        self.place_glyphs_in_clip(style, left, top, color, self.clip());
    }

    #[inline(always)]
    fn place_glyphs_in_clip(&mut self, style: TextStyle, left: f32, top: f32, color: Color, clip: PixelRect) {
        if self.text.has_color_fonts() {
            self.place_glyphs_with_color(style, left, top, color);
            return;
        }
        let color = color.premultiplied();
        // How far a glyph can reach from its pen and baseline in each direction, generously.
        // Anything further than this from the clip needs no drawing, so it needs no pixels.
        let reach = style.size * 2.0;
        let rounding = self.pen_rounding();
        let zoom = self.plain_zoom();
        for glyph in &self.glyphs.glyphs {
            let pen = left + glyph.x + rounding;
            // On a whole pixel, as a browser has it, so that letters are as sharp as they are on a page.
            let baseline = (top + glyph.y + 0.5).floor();
            if !clip.overlaps(pen - reach, baseline - reach, pen + reach, baseline + reach) {
                continue;
            }
            let Some(font) = self.text.font(glyph.font) else {
                continue;
            };
            let spot = GlyphSpot::new(zoom, style.size, left + glyph.x, rounding, top + glyph.y);
            let key = GlyphKey::new(glyph.font, glyph.glyph, spot.size, spot.pen);
            let placed = self.atlas.glyph(key, font, self.rasterizer);
            if placed.width == 0 {
                continue;
            }
            let (x, y) = spot.place(placed);
            let (width, height) = (f32::from(placed.width), f32::from(placed.height));
            if clip.overlaps(x, y, x + width, y + height) {
                self.quads.push(spot.quad(placed, x, y, color));
            }
        }
    }

    /// How much what's being drawn is resized, and where to, if that's all that's done to it, as CSS's `scale()` does, as
    /// `(scale, x, y)`.
    fn plain_zoom(&self) -> Option<(f32, f32, f32)> {
        // What's being moved is drawn as it was and stretched, as a browser has it until it stops.
        if self.transform == 0 || self.turning.iter().any(|one| one.moving) {
            return None;
        }
        let [a, b, c, d, e, f] = self.matrix;
        #[allow(clippy::float_cmp)]
        let plain = b == 0.0 && c == 0.0 && a == d && a > 0.0 && a != 1.0;
        plain.then_some((a, e, f))
    }
}

/// Draws a glyph that's made of colored shapes, one over the other, if this one is.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn place_color_layers(
    atlas: &mut GlyphAtlas,
    rasterizer: &mut Rasterizer,
    quads: &mut Vec<Quad>,
    font: &Font,
    (font_number, glyph): (u16, u16),
    size: f32,
    (pen, baseline): (f32, f32),
    text_color: Color,
    clip: PixelRect,
) -> bool {
    let mut any = false;
    for layer in font.color_layers(glyph) {
        any = true;
        let color = match layer.color {
            // The text can be see-through as a whole, as when it's fading.
            Some([r, g, b, a]) => {
                let a = (u32::from(a) * u32::from(text_color.a) + 127) / 255;
                Color::rgba(r, g, b, a as u8)
            }
            None => text_color,
        };
        let key = GlyphKey::new(font_number, layer.glyph, size, pen);
        let placed = atlas.glyph(key, font, rasterizer);
        if placed.width == 0 {
            continue;
        }
        let x = whole_part(pen) + f32::from(placed.left);
        let y = baseline + f32::from(placed.top);
        let (width, height) = (f32::from(placed.width), f32::from(placed.height));
        if clip.overlaps(x, y, x + width, y + height) {
            quads.push(glyph_quad(placed, x, y, color.premultiplied()));
        }
    }
    any
}

impl<const PRUNE: bool> SceneBuilder<'_, PRUNE> {
    /// Like [`place_glyphs`](Self::place_glyphs), for when a font with colored glyphs is open, so that text
    /// with none of them never comes this way.
    #[cold]
    #[inline(never)]
    fn place_glyphs_with_color(&mut self, style: TextStyle, left: f32, top: f32, color: Color) {
        let clip = self.clip();
        let reach = style.size * 2.0;
        let rounding = self.pen_rounding();
        let zoom = self.plain_zoom();
        for index in 0..self.glyphs.glyphs.len() {
            let glyph = self.glyphs.glyphs[index];
            let pen = left + glyph.x + rounding;
            // On a whole pixel, as a browser has it, so that letters are as sharp as they are on a page.
            let baseline = (top + glyph.y + 0.5).floor();
            if !clip.overlaps(pen - reach, baseline - reach, pen + reach, baseline + reach) {
                continue;
            }
            let Some(font) = self.text.font(glyph.font) else {
                continue;
            };
            if font.has_color_layers() {
                let key = ColorGlyphKey {
                    font: glyph.font,
                    glyph: glyph.glyph,
                    size_bits: style.size.to_bits(),
                    // Fading it is done when it's drawn, so a fade doesn't draw it over and over.
                    text_color: [color.r, color.g, color.b, 255],
                };
                self.pictured_glyphs = true;
                let (rasterizer, images) = (&mut *self.rasterizer, &mut *self.images);
                let made = images.color_glyph(key, || {
                    rasterizer.rasterize_color(font, glyph.glyph, style.size, key.text_color)
                });
                if let Some(made) = made {
                    self.place_color_glyph(made, pen, baseline, color, clip);
                    continue;
                }
                if place_color_layers(
                    self.atlas,
                    self.rasterizer,
                    self.quads,
                    font,
                    (glyph.font, glyph.glyph),
                    style.size,
                    (pen, baseline),
                    color,
                    clip,
                ) {
                    continue;
                }
            }
            let spot = GlyphSpot::new(zoom, style.size, left + glyph.x, rounding, top + glyph.y);
            let key = GlyphKey::new(glyph.font, glyph.glyph, spot.size, spot.pen);
            let placed = self.atlas.glyph(key, font, self.rasterizer);
            if placed.width == 0 {
                continue;
            }
            let (x, y) = spot.place(placed);
            let (width, height) = (f32::from(placed.width), f32::from(placed.height));
            if clip.overlaps(x, y, x + width, y + height) {
                self.quads
                    .push(spot.quad(placed, x, y, color.premultiplied()));
            }
        }
    }

    /// Draws a glyph that's been kept as a picture, at a whole pixel, as pictures aren't drawn between them.
    fn place_color_glyph(
        &mut self,
        made: ColorGlyph,
        pen: f32,
        baseline: f32,
        color: Color,
        clip: PixelRect,
    ) {
        let x = (pen + 0.5).floor() + made.left as f32;
        let y = (baseline + 0.5).floor() + made.top as f32;
        let (width, height) = (made.width as f32, made.height as f32);
        if !clip.overlaps(x, y, x + width, y + height) {
            return;
        }
        self.finish_batch();
        let glass = Color::rgba(255, 255, 255, color.a);
        self.quads.push(Quad {
            rect: [x, y, width, height],
            glyph: [0, 0, u16::MAX, u16::MAX],
            color: glass.premultiplied(),
            radius: 0.0,
        });
        self.batches.push(Batch {
            first: self.batch_start as u32,
            count: 1,
            clip: self.clip_on_screen(clip),
            image: made.id,
            transform: self.transform,
            filter: self.filter,
            round: self.round(),
        });
        self.batch_start = self.quads.len();
    }
}

/// How wide a border this wide, as CSS says, is drawn, in physical pixels: on whole pixels, and never thinner than one.
fn drawn_width(width: f32, scale: f32) -> f32 {
    if width > 0.0 { (width * scale).round().max(1.0) } else { 0.0 }
}

#[inline(always)]
/// A corner as round as it can be on something this size, which is half of its shorter side, as in CSS: more than that would leave
/// nothing in the middle.
fn fitting_radius(radius: f32, rect: [f32; 4]) -> f32 {
    radius.min(rect[2].min(rect[3]) / 2.0).max(0.0)
}

/// The rectangle CSS's `clip-path: inset(top right bottom left)` cuts `rect` to, its sides as CSS's `margin` has them, in pixels
/// or parts of the box, or nothing for another shape.
fn inset_rect(said: &str, rect: Rect) -> Option<Rect> {
    let inside = said.trim().strip_prefix("inset(")?.strip_suffix(')')?;
    // What comes after `round` is the shape of its corners, which the cut leaves out.
    let inside = inside.split(" round ").next().unwrap_or(inside);
    let words: Vec<&str> = inside.split_whitespace().collect();
    let length = |word: &str, whole: f32| -> Option<f32> {
        match word.strip_suffix('%') {
            Some(share) => share.parse::<f32>().ok().map(|share| share / 100.0 * whole),
            None => word.trim_end_matches("px").parse::<f32>().ok(),
        }
    };
    let get = |at: usize| words.get(at).copied();
    let top = get(0)?;
    let right = get(1).unwrap_or(top);
    let bottom = get(2).unwrap_or(top);
    let left = get(3).unwrap_or(right);
    let (top, right) = (length(top, rect.height)?, length(right, rect.width)?);
    let (bottom, left) = (length(bottom, rect.height)?, length(left, rect.width)?);
    Some(Rect::new(
        rect.x + left,
        rect.y + top,
        (rect.width - left - right).max(0.0),
        (rect.height - top - bottom).max(0.0),
    ))
}

/// A glyph from the atlas at this place, in this premultiplied color. One with a byte for each part of each pixel is a third as wide
/// on the screen as in the atlas, and says so with a radius below 0, which nothing else has.
/// Where a glyph goes, and how big it's drawn. In what's only resized, as CSS's `scale()` does, that's at the size it ends up on
/// the screen, on whole pixels of the screen, so that it's as sharp as a browser has it, rather than stretched once it's drawn.
#[derive(Clone, Copy)]
struct GlyphSpot {
    size: f32,
    pen: f32,
    baseline: f32,
    /// How much it's resized, where to.
    zoom: Option<(f32, f32, f32)>,
}

impl GlyphSpot {
    fn new(zoom: Option<(f32, f32, f32)>, size: f32, x: f32, rounding: f32, y: f32) -> Self {
        match zoom {
            Some((scale, left, top)) => Self {
                size: size * scale,
                pen: x * scale + left + rounding,
                baseline: (y * scale + top + 0.5).floor(),
                zoom,
            },
            None => Self {
                size,
                pen: x + rounding,
                baseline: (y + 0.5).floor(),
                zoom,
            },
        }
    }

    /// Where its picture's top left corner goes, before what it's in is resized.
    fn place(self, placed: AtlasGlyph) -> (f32, f32) {
        let x = whole_part(self.pen) + f32::from(placed.left);
        let y = self.baseline + f32::from(placed.top);
        match self.zoom {
            Some((scale, left, top)) => ((x - left) / scale, (y - top) / scale),
            None => (x, y),
        }
    }

    fn quad(self, placed: AtlasGlyph, x: f32, y: f32, color: [u8; 4]) -> Quad {
        let mut quad = glyph_quad(placed, x, y, color);
        if let Some((scale, _, _)) = self.zoom {
            quad.rect[2] /= scale;
            quad.rect[3] /= scale;
        }
        quad
    }
}

fn glyph_quad(placed: AtlasGlyph, x: f32, y: f32, color: [u8; 4]) -> Quad {
    let (across, down) = (f32::from(placed.width), f32::from(placed.height));
    Quad {
        rect: [
            x,
            y,
            if placed.subpixel {
                across / 3.0
            } else {
                across
            },
            down,
        ],
        glyph: [placed.x, placed.y, placed.width, placed.height],
        color,
        radius: if placed.subpixel { -1.0 } else { 0.0 },
    }
}

/// How big a background's picture is, in pixels, as CSS's `background-size` says, from how big it is itself and how big what it's
/// placed in is.
fn background_size(said: &str, natural: (f32, f32), room: (f32, f32), (scale, em): (f32, f32)) -> (f32, f32) {
    let (width, height) = (natural.0.max(1.0), natural.1.max(1.0));
    match said.trim() {
        "cover" => {
            let times = (room.0 / width).max(room.1 / height);
            (width * times, height * times)
        }
        "contain" => {
            let times = (room.0 / width).min(room.1 / height);
            (width * times, height * times)
        }
        said => {
            let mut words = said.split_whitespace();
            let length = |word: Option<&str>, whole: f32| {
                let word = word?;
                if let Some(percent) = word.strip_suffix('%') {
                    return percent
                        .parse::<f32>()
                        .ok()
                        .map(|percent| percent / 100.0 * whole);
                }
                if let Some(ems) = word.strip_suffix("em").filter(|_| !word.ends_with("rem")) {
                    return ems.parse::<f32>().ok().map(|ems| ems * em * scale);
                }
                word.strip_suffix("px")
                    .unwrap_or(word)
                    .parse::<f32>()
                    .ok()
                    .map(|pixels| pixels * scale)
            };
            match (length(words.next(), room.0), length(words.next(), room.1)) {
                (Some(across), Some(down)) => (across, down),
                (Some(across), None) => (across, across * height / width),
                (None, Some(down)) => (down * width / height, down),
                (None, None) => (width, height),
            }
        }
    }
}

/// Where a background's picture is, in pixels from the top left of what it's placed in, as CSS's `background-position` says,
/// from how much room there is around it.
fn background_position(said: &str, spare: (f32, f32), scale: f32) -> (f32, f32) {
    let words: Vec<&str> = said.split_whitespace().collect();
    let place = |word: &str, spare: f32| match word {
        "left" | "top" => Some(0.0),
        "center" => Some(spare / 2.0),
        "right" | "bottom" => Some(spare),
        word => match word.strip_suffix('%') {
            Some(percent) => percent
                .parse::<f32>()
                .ok()
                .map(|percent| percent / 100.0 * spare),
            None => word
                .strip_suffix("px")
                .unwrap_or(word)
                .parse::<f32>()
                .ok()
                .map(|pixels| pixels * scale),
        },
    };
    let upright = |word: &str| matches!(word, "top" | "bottom");
    match words.as_slice() {
        [] => (0.0, 0.0),
        [one] if upright(one) => (spare.0 / 2.0, place(one, spare.1).unwrap_or(0.0)),
        [one] => (place(one, spare.0).unwrap_or(0.0), spare.1 / 2.0),
        // `right 10px bottom 5px` is from those edges.
        [side_x, by_x, side_y, by_y, ..] => {
            let (side_x, by_x, side_y, by_y) = if upright(side_x) {
                (side_y, by_y, side_x, by_x)
            } else {
                (side_x, by_x, side_y, by_y)
            };
            let from = |side: &str, by: &str, spare: f32| {
                let by = place(by, spare).unwrap_or(0.0);
                if matches!(side, "right" | "bottom") {
                    spare - by
                } else {
                    by
                }
            };
            (from(side_x, by_x, spare.0), from(side_y, by_y, spare.1))
        }
        // `right 12px center`, as Bootstrap places a select's arrow, is an edge and how far from it for one way, and a keyword
        // for the other, either way round.
        [first, second, third] => {
            let keyword = |word: &str| matches!(word, "left" | "right" | "top" | "bottom" | "center");
            let (side, by, other) = if keyword(second) {
                (*second, *third, *first)
            } else {
                (*first, *second, *third)
            };
            let along = |side: &str, by: &str, spare: f32| {
                let by = place(by, spare).unwrap_or(0.0);
                if matches!(side, "right" | "bottom") {
                    spare - by
                } else {
                    by
                }
            };
            if upright(side) || matches!(other, "left" | "right") {
                (place(other, spare.0).unwrap_or(0.0), along(side, by, spare.1))
            } else {
                (along(side, by, spare.0), place(other, spare.1).unwrap_or(0.0))
            }
        }
        [first, second, ..] => {
            let (x, y) = if upright(first) || matches!(*second, "left" | "right") {
                (second, first)
            } else {
                (first, second)
            };
            (
                place(x, spare.0).unwrap_or(0.0),
                place(y, spare.1).unwrap_or(0.0),
            )
        }
    }
}

/// Keep a one-pixel guard band at the scissor edge for subpixel rasterization.
/// Invalid or reversed bounds stay on the existing path, including public Editing input.
fn selection_line_outside(clip: PixelRect, top: f32, height: f32) -> bool {
    let bottom = top + height;
    top.is_finite()
        && bottom.is_finite()
        && height >= 0.0
        && (bottom < clip.top as f32 - 1.0 || top > clip.bottom as f32 + 1.0)
}

fn selection_rect_outside(clip: PixelRect, [left, top, width, height]: [f32; 4]) -> bool {
    let (right, bottom) = (left + width, top + height);
    [left, top, right, bottom]
        .iter()
        .all(|value| value.is_finite())
        && width >= 0.0
        && height >= 0.0
        && (right < clip.left as f32 - 1.0
            || bottom < clip.top as f32 - 1.0
            || left > clip.right as f32 + 1.0
            || top > clip.bottom as f32 + 1.0)
}

/// What a shadow inside something is marked with, in place of a glyph's width in the atlas, as the shader has it.
const INSET_MARK: u16 = 65534;

/// A color written in a prop's text as `Color::to_prop` has it, as a control in the platform's look writes them.
fn color_word(word: &str) -> Option<Color> {
    Color::from_prop(&PropValue::Int(word.parse().ok()?))
}

fn solid_quad(rect: [f32; 4], color: Color, radius: f32) -> Quad {
    Quad {
        rect,
        glyph: [0; 4],
        color: color.premultiplied(),
        radius,
    }
}

/// Text longer than this has its words broken to fit, whatever it says, as finding its longest word each time it's drawn would
/// take longer than drawing it.
const KEPT_WORDS_LONGEST: usize = 4096;

/// Whether text keeps each word whole, as on a page, rather than breaking one that's too wide.
fn keeps_words(element: &Element) -> bool {
    static KEEP_WORDS: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("keep_words"));
    element.props.get_symbol(*KEEP_WORDS) == Some(&PropValue::Bool(true))
}

/// Whether its outline is `outline-style: auto`, which is drawn as the browser's focus ring is.
fn is_auto_outline(element: &Element) -> bool {
    static OUTLINE_STYLE: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("outline_style"));
    matches!(element.props.get_symbol(*OUTLINE_STYLE), Some(PropValue::Str(how)) if how.as_str() == "auto")
}

/// Whether something says what outline it has, rather than saying nothing: none at all, as CSS's `outline: none` says, or its
/// own, which is drawn instead of the ring a browser draws around what has the keyboard, as an author's outline is, transparent
/// or not, as Tailwind's `outline-none` (`2px solid transparent`) is.
fn has_no_outline(element: &Element) -> bool {
    static OUTLINE_WIDTH: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("outline_width"));
    element.props.get_symbol(*OUTLINE_WIDTH).is_some()
}

/// How many lines an element's own text is drawn on, in this place, in layout pixels, as [`Scene`] lays it out to draw it. For
/// comparing with how a browser breaks the same text.
pub fn text_line_count(text: &mut TextEngine, element: &Element, rect: Rect) -> usize {
    if element.name.is_empty() {
        return 0;
    }
    let style = TextStyle::from_element(element);
    let content = content_box(element, rect, 1.0);
    let arrangement = Arrangement::of(element);
    if arrangement.ellipsis || arrangement.no_wrap {
        return element.name.split('\n').count();
    }
    let limit = if keeps_words(element) && element.name.len() < KEPT_WORDS_LONGEST {
        content.width.max(text.longest_word(&element.name, style))
    } else {
        content.width
    };
    let mut layout = TextLayout::default();
    text.layout_into(&element.name, style, limit, &mut layout);
    let lines = layout.lines.len();
    rsui_text::max_lines(element).map_or(lines, |most| lines.min(most))
}

/// How wide each of an element's borders is, as `(top, right, bottom, left)`, in logical pixels.
fn border_sides(element: &Element) -> (f32, f32, f32, f32) {
    let number = |name: &str| match element.props.get(name) {
        Some(PropValue::Float(width)) => Some(*width as f32),
        Some(PropValue::Int(width)) => Some(*width as f32),
        _ => None,
    };
    if element.props.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    // A border's only drawn with a color, which is what makes it take room.
    let all = if element.props.get("border_color").is_some() {
        number("border_width").unwrap_or(0.0)
    } else {
        0.0
    };
    let side = |name: &str| number(name).unwrap_or(all).max(0.0);
    (
        side("border_top_width"),
        side("border_right_width"),
        side("border_bottom_width"),
        side("border_left_width"),
    )
}

fn outline_quad(rect: [f32; 4], color: Color, radius: f32, width: f32) -> Quad {
    let sixteenths = (width * 16.0).round().clamp(1.0, f32::from(u16::MAX)) as u16;
    Quad {
        rect,
        glyph: [sixteenths, 0, 0, 0],
        color: color.premultiplied(),
        radius,
    }
}

impl<const PRUNE: bool> LayoutVisitor for SceneBuilder<'_, PRUNE> {
    fn clipped_children_are_invisible(&self) -> bool {
        let clip = self.clip();
        PRUNE
            && self.transform == 0
            && self.filter == 0
            && self.fading == 0
            && (clip.left >= clip.right || clip.top >= clip.bottom)
    }

    /// What [`enter`](Self::enter) and [`paragraph`](Self::paragraph) find for a box that draws only inside itself: whether its
    /// edges, on whole pixels, miss what's cut off around it. Not while something around it is turned, filtered or faded, where
    /// that isn't all there is to it, nor for the pilot, which keeps what each thing drew.
    #[inline(always)]
    fn draws_nothing_at(&self, rect: Rect) -> bool {
        #[cfg(feature = "render-diagnostics")]
        if CULL_OFF.load(std::sync::atomic::Ordering::Relaxed) {
            return false;
        }
        if !PRUNE
            || cfg!(feature = "mutation-pilot")
            || self.transform != 0
            || self.filter != 0
            || self.fading != 0
        {
            return false;
        }
        let left = snap(rect.x * self.scale);
        let top = snap(rect.y * self.scale);
        let right = snap((rect.x + rect.width) * self.scale);
        let bottom = snap((rect.y + rect.height) * self.scale);
        !self.clip().overlaps(left, top, right, bottom)
    }

    /// What's past the edges of something that scrolls mustn't show, which are inside its borders, as CSS has it, and inside
    /// its round corners, if it has them.
    fn enter_scrolled(&mut self, rect: Rect, element: &Element) {
        let (top, right, bottom, left) = border_sides(element);
        let inside = Rect::new(
            rect.x + left,
            rect.y + top,
            (rect.width - left - right).max(0.0),
            (rect.height - top - bottom).max(0.0),
        );
        self.push_clip(inside);
        if self.look.corner_radius > 0.0 || self.look.shaped {
            self.round_off(element, rect);
        }
    }

    fn clip_to(&mut self, rect: Rect, element: &Element) {
        // On the way to something placed by itself, this element wasn't entered.
        self.read_look(element);
        // What's cut to a rectangle in from its edges, as CSS's `clip-path: inset()` says, is cut off there.
        let cut = if self.look.shaped {
            match element.props.get("clip_path") {
                Some(PropValue::Str(said)) => inset_rect(said.as_str(), rect).unwrap_or(rect),
                _ => rect,
            }
        } else {
            rect
        };
        self.push_clip(cut);
        // What's in something with round corners, as an avatar's picture is, is cut off at them too.
        if self.look.corner_radius > 0.0 || self.look.shaped {
            self.round_off(element, rect);
        }
    }

    fn unclip(&mut self) {
        self.pop_clip();
    }

    fn exit_scrolled(&mut self, rect: Rect, info: ScrollInfo, element: &Element) {
        if matches!(element.props.get("scrollbar_style"), Some(PropValue::Str(style)) if style.as_str() == "classic")
        {
            self.paint_classic_scrollbars(rect, info, element);
            self.pop_clip();
            return;
        }
        // What it looks like can be said, as CSS's scrollbars do, with a size that's all of it and colors for its thumb and track.
        let said = match element.props.get("scrollbar_size") {
            Some(PropValue::Float(size)) => Some(*size as f32),
            Some(PropValue::Int(size)) => Some(*size as f32),
            _ => None,
        };
        let (thickness, inset) = match said {
            Some(size) => ((size * self.scale).round(), 0.0),
            None => (
                (SCROLL_THUMB_WIDTH * self.scale).round().max(1.0),
                (SCROLL_THUMB_INSET * self.scale).round(),
            ),
        };
        if thickness <= 0.0 {
            self.pop_clip();
            return;
        }
        // A thumb that's given a size can still be drawn as rsui's own is: rounded, and with some room round it in its track, which is
        // what CSS's `::-webkit-scrollbar-thumb` says with `border-radius` and a transparent `border` (`scrollbar_radius`,
        // `scrollbar_inset`). Without them a thumb that's given a size is a plain rectangle that fills it.
        let number = |name: &str| match element.props.get(name) {
            Some(PropValue::Float(value)) => Some(*value as f32),
            Some(PropValue::Int(value)) => Some(*value as f32),
            _ => None,
        };
        let room_round = if said.is_some() {
            (number("scrollbar_inset").unwrap_or(0.0) * self.scale).round().clamp(0.0, thickness / 2.0)
        } else {
            0.0
        };
        let radius = if said.is_some() {
            (number("scrollbar_radius").unwrap_or(0.0) * self.scale).max(0.0)
        } else {
            thickness / 2.0
        };
        // On a phone, rsui's own only shows while it's scrolling, and as much as it still does.
        let shows = match self.interaction.scrolling {
            _ if said.is_some() || !scrollbars_fade() => 1.0,
            Some((scrolling, shows)) if std::ptr::eq(scrolling, element) => shows,
            _ => 0.0,
        };
        if shows <= 0.0 {
            self.pop_clip();
            return;
        }
        // It goes with the text, so that it shows up on whatever's behind that.
        let thumb = element
            .props
            .get("scrollbar_thumb")
            .and_then(Color::from_prop)
            .unwrap_or(Color {
                a: (f32::from(SCROLL_THUMB_OPACITY) * shows).round() as u8,
                ..self.around_color()
            });
        let track = element
            .props
            .get("scrollbar_track")
            .and_then(Color::from_prop);
        let vertical = scroll_thumb(info.view.height, info.content.height, info.y);
        if let (Some(track), Some(_)) = (track.filter(|track| track.a > 0), vertical) {
            let left = ((rect.x + rect.width) * self.scale).round() - thickness;
            let top = (rect.y * self.scale).round();
            self.quads.push(solid_quad(
                [left, top, thickness, (rect.height * self.scale).round()],
                track,
                0.0,
            ));
        }
        let vertical = vertical.filter(|_| thumb.a > 0);
        if let Some((start, length)) = vertical {
            let top = ((rect.y + start) * self.scale).round() + room_round;
            let bottom = ((rect.y + start + length) * self.scale).round() - room_round;
            let left = ((rect.x + rect.width) * self.scale).round() - inset - thickness + room_round;
            let (across, down) = (thickness - room_round * 2.0, bottom - top);
            self.quads.push(solid_quad(
                [left, top, across, down],
                thumb,
                radius.min(across.min(down) / 2.0),
            ));
        }
        let across =
            scroll_thumb(info.view.width, info.content.width, info.x).filter(|_| thumb.a > 0);
        if let Some((start, length)) = across {
            let left = ((rect.x + start) * self.scale).round() + room_round;
            let right = ((rect.x + start + length) * self.scale).round() - room_round;
            let top = ((rect.y + rect.height) * self.scale).round() - inset - thickness + room_round;
            let (across, down) = (right - left, thickness - room_round * 2.0);
            self.quads.push(solid_quad(
                [left, top, across, down],
                thumb,
                radius.min(across.min(down) / 2.0),
            ));
        }
        self.pop_clip();
    }

    #[inline]
    fn enter(&mut self, element: &Element, rect: Rect) {
        #[cfg(feature = "mutation-pilot")]
        let pilot_first = self.quads.len();
        #[cfg(feature = "render-diagnostics")]
        {
            self.work.node_enters = self.work.node_enters.saturating_add(1);
        }
        // Edges land on whole pixels, or they'd come out blurred.
        let left = snap(rect.x * self.scale);
        let top = snap(rect.y * self.scale);
        let right = snap((rect.x + rect.width) * self.scale);
        let bottom = snap((rect.y + rect.height) * self.scale);
        let mut seen = self.clip().overlaps(left, top, right, bottom);
        self.read_look(element);
        if let Some(color) = self.look.text_color {
            if !element.children.is_empty() {
                self.colors
                    .push((std::ptr::from_ref(element) as usize, color));
            }
        }
        // Each of the things that branch off here is rare, and all of it costs one check.
        if self.look.unusual || self.shading {
            seen = self.enter_unusual(element, [left, top, right, bottom], seen);
        } else if seen {
            if let Some(background) = self.look.background {
                if background.a > 0 {
                    let area = [left, top, right - left, bottom - top];
                    let radius = fitting_radius(self.look.corner_radius * self.scale, area);
                    self.quads.push(solid_quad(area, background, radius));
                }
            }
        }
        #[cfg(feature = "mutation-pilot")]
        let pilot_text = self.quads.len();
        if seen {
            // What says it has no outline, as `outline: none` does in CSS, shows that it has the keyboard some other way.
            if is(self.interaction.ring, element) && !has_no_outline(element) {
                self.paint_ring(element, [left, top, right, bottom]);
            }
            if matches!(element.role, Role::TextInput | Role::Image) {
                self.paint_contents(element, rect, [left, top, right, bottom]);
            } else if shows_name(element)
                && !element.name.is_empty()
                && (element.children.is_empty() || shows_own_text(element))
            {
                self.paint_text(element, rect);
            }
        }
        #[cfg(feature = "mutation-pilot")]
        {
            let safe = seen
                && self.transform == 0
                && self.filter == 0
                && self.fading == 0
                && self.look.backdrop_blur == 0.0
                && mutation_pilot::interaction_safe(element, self.interaction);
            let inherited = self.around_color();
            self.pilot.capture(
                element,
                self.look,
                [pilot_first, pilot_text, self.quads.len()],
                inherited,
                safe,
            );
        }
    }

    fn paragraph(&mut self, element: &Element, rect: Rect) {
        #[cfg(feature = "render-diagnostics")]
        {
            self.work.paragraph_callbacks = self.work.paragraph_callbacks.saturating_add(1);
        }
        let left = snap(rect.x * self.scale);
        let top = snap(rect.y * self.scale);
        let right = snap((rect.x + rect.width) * self.scale);
        let bottom = snap((rect.y + rect.height) * self.scale);
        if self.clip().overlaps(left, top, right, bottom) {
            self.paint_paragraph(element, rect);
        }
    }

    #[inline]
    fn exit(&mut self, element: &Element) {
        if self.fading != 0 {
            self.finish_fade(element);
        }
        if !self.colors.is_empty() {
            self.let_go_of_color(element);
        }
        if self.filter != 0 {
            self.finish_filter(element);
        }
        if self.transform != 0 {
            self.finish_transform(element);
        }
    }

    fn passing(&mut self, element: &Element) {
        #[cfg(feature = "render-diagnostics")]
        {
            self.work.ancestor_passes = self.work.ancestor_passes.saturating_add(1);
        }
        if let Some(color) = passed_text_color(element) {
            self.colors
                .push((std::ptr::from_ref(element) as usize, color));
        }
    }

    fn passed(&mut self, element: &Element) {
        if !self.colors.is_empty() {
            self.let_go_of_color(element);
        }
    }
}

#[cfg(feature = "render-diagnostics")]
impl Painter {
    /// Copies the last scene record without touching resources or queues.
    pub fn scene_work(&self) -> crate::diagnostics::SceneWork {
        self.work
    }

    fn atlas_observation(&self) -> crate::diagnostics::Atlas {
        crate::diagnostics::Atlas {
            generation: self.atlas.generation(),
            size: self.atlas.size(),
            entries: self.atlas.len() as u64,
        }
    }

    fn finish_work(&mut self) {
        self.work.output = crate::diagnostics::Commands::of(&self.scene);
        self.work.atlas_after = self.atlas_observation();
        for upload in &self.scene.uploads {
            self.work.pending_glyphs.add(upload.coverage.len() as u64);
        }
        for upload in &self.scene.image_uploads {
            self.work.pending_images.add(upload.pixels.len() as u64);
        }
        self.work.pending_drops = self.scene.dropped_images.len() as u64;
    }

    /// Walks owned retained lists on request. See diagnostics::PainterStorage for exclusions.
    pub fn retained_storage(&self) -> crate::diagnostics::PainterStorage {
        use crate::diagnostics::{CommandStorage, ListStorage, PainterStorage};
        let scene = &self.scene;
        let mut storage = PainterStorage {
            #[cfg(feature = "mutation-pilot")]
            comparison_spare: CommandStorage::of(
                &self.pilot.history.quads,
                &self.pilot.history.batches,
                &self.pilot.history.transforms,
                &self.pilot.history.filters,
            ),
            scene: CommandStorage::of(
                &scene.quads,
                &scene.batches,
                &scene.transforms,
                &scene.filters,
            ),
            glyph_descriptors: ListStorage::of(&scene.uploads),
            image_descriptors: ListStorage::of(&scene.image_uploads),
            dropped_images: ListStorage::of(&scene.dropped_images),
            spare_text_descriptors: ListStorage::of(&self.text_runs.spare),
            paragraph_key_capacity_bytes: self.paragraph.source.text.capacity() as u64
                + (self.paragraph.source.runs.capacity() * size_of::<rsui_text::TextRun>()) as u64,
            text_entries: self.text_runs.runs.len() as u64,
            text_map_capacity: self.text_runs.runs.capacity() as u64,
            scratch_glyphs: ListStorage::of(&self.glyphs.glyphs),
            scratch_lines: ListStorage::of(&self.glyphs.lines),
            clips: ListStorage::of(&self.clips),
            fades: ListStorage::of(&self.fades),
            turning: ListStorage::of(&self.turning),
            atlas: self.atlas_observation(),
            image_payload_estimate: self.images.gpu_bytes() as u64,
            ..Default::default()
        };
        if let Some(kept) = &self.with_caret {
            storage.kept_caret =
                CommandStorage::of(&kept.quads, &kept.batches, &kept.transforms, &kept.filters);
        }
        for upload in &scene.uploads {
            storage.glyph_payload.add(ListStorage::of(&upload.coverage));
        }
        for upload in &scene.image_uploads {
            storage.image_payload.add(ListStorage::of(&upload.pixels));
        }
        for run in self.text_runs.runs.values() {
            storage.live_text_quads.add(ListStorage::of(&run.quads));
        }
        for quads in &self.text_runs.spare {
            storage.spare_text_quads.add(ListStorage::of(quads));
        }
        storage
    }
}

#[cfg(all(test, feature = "render-diagnostics"))]
#[path = "../../rsui-text/tests/support/mod.rs"]
mod diagnostic_font;

#[cfg(all(test, feature = "render-diagnostics"))]
mod passing_color_tests {
    use super::*;

    fn values() -> Vec<Option<PropValue>> {
        vec![
            None,
            Some(PropValue::Bool(false)),
            Some(PropValue::Float(0x112233ff_u32 as f64)),
            Some(PropValue::from("#112233")),
            Some(PropValue::Int(-1)),
            Some(PropValue::Int(i64::from(u32::MAX) + 1)),
            Some(PropValue::Int(0)),
            Some(PropValue::Int(Color::rgba(17, 34, 51, 0).to_prop())),
            Some(PropValue::Int(Color::rgba(17, 34, 51, 96).to_prop())),
            Some(PropValue::Int(Color::hex(0x112233).to_prop())),
        ]
    }

    #[test]
    fn direct_passing_color_matches_full_look_reference() {
        for value in values() {
            // These exercise unrelated parsing in the old full Look, including
            // its disabled/plain postprocessing. None changes inherited color.
            let mut ancestor = Element::new(Role::Group)
                .prop("background", Color::WHITE.to_prop())
                .prop("border_width", 2.0)
                .prop("border_style", "dashed")
                .prop("shadow_blur", 8.0)
                .prop("disabled", true)
                .prop("plain_disabled", true)
                .prop("opacity", 0.5);
            if let Some(value) = value { ancestor = ancestor.prop("text_color", value); }
            assert_eq!(passed_text_color(&ancestor), Look::read(&ancestor.props, &NAMES).text_color);
        }
    }

    #[test]
    fn absolute_text_keeps_full_reference_inheritance_and_alpha() {
        let inherited = Color::hex(0x406080);
        for value in values() {
            let mut ancestor = Element::new(Role::Group)
                .prop("width", 200.0).prop("height", 100.0);
            if let Some(value) = value { ancestor = ancestor.prop("text_color", value); }
            let expected = Look::read(&ancestor.props, &NAMES).text_color.unwrap_or(inherited);
            let text = Element::new(Role::Text).name("AB")
                .prop("font_size", 16.0).prop("position", "absolute")
                .prop("left", 12.0).prop("top", 10.0);
            let root = |leaf: Element| Element::new(Role::Window)
                .prop("text_color", inherited.to_prop())
                .child(ancestor.clone().child(leaf)).into_node();
            let actual = root(text.clone());
            let reference = root(text.prop("text_color", expected.to_prop()));
            let paint = |node: &Node| {
                let mut text = TextEngine::new(diagnostic_font::plain_font());
                let mut layout = Layout::new();
                layout.compute(node, rsui_layout::Size::new(240.0, 140.0), &mut text);
                let mut painter = Painter::new();
                let scene = painter.paint(node, &layout, &mut text, 1.0, (240, 140));
                let quads = scene.quads.iter().filter(|quad| quad.glyph != [0; 4]).copied().collect::<Vec<_>>();
                assert!(painter.scene_work().ancestor_passes >= 2, "exercise the passing path");
                quads
            };
            let actual = paint(&actual);
            assert!(!actual.is_empty(), "the text creates glyph commands even with transparent color");
            assert_eq!(actual, paint(&reference));
            assert!(actual.iter().all(|quad| quad.color == expected.premultiplied()));
        }
    }
}

#[cfg(all(test, feature = "render-diagnostics"))]
mod diagnostics_tests {
    use super::*;
    use crate::diagnostics::*;

    #[test]
    fn complete_text_command_capacity_is_bounded_and_evicted() {
        let mut runs = TextRuns::default();
        runs.begin_frame();
        let quads = vec![solid_quad([0.0, 0.0, 7.0, 7.0], Color::WHITE, 0.0); 128];
        for index in 0..1000 {
            runs.keep(TextRunKey {
                name: format!("run {index}").into(),
                style: TextStyle::new(14.0),
                keeps_words: false,
                place: [0; 3],
                clip: PixelRect::EVERYWHERE,
                translated: true,
                color: Color::WHITE,
                rounding: 0,
                zoom: None,
                engine: 0,
                fonts: 1,
                atlas_generation: 0,
            }, &quads, [0.0; 2], Some(0.0));
        }
        assert!(runs.translated_bytes <= TRANSLATED_RUN_BYTES);
        assert!(runs.runs.len() < 1000);
        runs.begin_frame();
        runs.end_frame();
        assert_eq!(runs.translated_bytes, 0);
        assert!(runs.runs.is_empty());
        assert!(runs.spare_bytes <= SPARE_BYTES);
    }

    #[cfg(not(debug_assertions))]
    #[test]
    #[ignore = "paired optimized paint-only timing; run with --profile bench --ignored --nocapture"]
    fn timing_scroll_command_replay() {
        let mut baseline = Painter::new();
        baseline.set_text_command_reuse_for_diagnostics(false);
        let mut replay = Painter::new();
        let font = || Font::open("C:\\Windows\\Fonts\\segoeui.ttf").expect("Windows UI font for timing");
        let mut baseline_text = TextEngine::new(font());
        let mut replay_text = TextEngine::new(font());
        let mut measure_text = TextEngine::new(font());
        let mut base_ns = Vec::new();
        let mut replay_ns = Vec::new();
        let mut reuses = 0;
        for step in 0..1200 {
            let offset = f64::from(step % 300) * 0.731_267;
            let mut rows = Element::new(Role::Group);
            for row in 0..200 {
                rows = rows.child(Element::new(Role::Text).name(format!("Row {row}: drawing commands stay reusable while scrolling"))
                    .prop("height", 24.0).prop("font_size", 14.0));
            }
            let root = Element::new(Role::Window).child(Element::new(Role::Group)
                .prop("scroll_y", offset).prop("height", 800.0).prop("scrollbar_size", 0.0).child(rows)).into_node();
            let mut layout = Layout::new();
            layout.compute(&root, rsui_layout::Size::new(600.0, 800.0), &mut measure_text);
            if step % 300 > 0 {
                assert!(layout.scroll_info(&[0], &root.children[0]).unwrap().y > 0.0);
            }
            let paint = |p: &mut Painter, text: &mut TextEngine| {
                let start = std::time::Instant::now();
                p.paint(&root, &layout, text, 1.0, (600, 800));
                std::hint::black_box(p.scene.quads.len());
                start.elapsed().as_nanos()
            };
            let (a, b) = if step % 2 == 0 {
                (paint(&mut baseline, &mut baseline_text), paint(&mut replay, &mut replay_text))
            } else {
                let b = paint(&mut replay, &mut replay_text);
                (paint(&mut baseline, &mut baseline_text), b)
            };
            if step >= 100 {
                base_ns.push(a);
                replay_ns.push(b);
                reuses += replay.scene_work().translated_text_run_reuses;
            }
        }
        base_ns.sort_unstable();
        replay_ns.sort_unstable();
        let middle = base_ns.len() / 2;
        eprintln!("scroll paint median ns: exact={}, replay={}, replay/exact={:.3}; translated replays={reuses}", base_ns[middle], replay_ns[middle], replay_ns[middle] as f64 / base_ns[middle] as f64);
        assert!(reuses > 0);
    }

    #[test]
    fn scrolling_replays_short_text_commands_with_current_clips() {
        fn page(down: f32) -> Node {
            let mut rows = Element::new(Role::Group);
            for row in 0..100 {
                rows = rows.child(Element::new(Role::Text)
                    .name(format!("ABCD ABCD row {row}"))
                    .prop("height", 24.0)
                    .prop("font_size", 14.0));
            }
            Element::new(Role::Window).child(Element::new(Role::Group)
                .prop("scroll_y", f64::from(down)).prop("height", 100.0).prop("scrollbar_size", 0.0).child(rows)).into_node()
        }
        for scale in [1.0, 1.25] {
        let mut painter = Painter::new();
        let mut text = TextEngine::new(diagnostic_font::plain_font());
        let mut renderer = crate::Renderer::offscreen_flipping((300, 120)).unwrap();
        let mut reference_renderer = crate::Renderer::offscreen_flipping((300, 120)).unwrap();
        let mut reused = 0;
        for down in [0.0, 1.0, 1.5, 2.5, 5.0, 11.0, 24.0, 25.0, 100.0, 95.0, 0.0] {
            let root = page(down);
            let mut layout = Layout::new();
            layout.compute(&root, rsui_layout::Size::new(300.0 / scale, 120.0 / scale), &mut text);
            if down > 0.0 {
                assert!(layout.scroll_info(&[0], &root.children[0]).unwrap().y > 0.0);
            }
            painter.paint(&root, &layout, &mut text, scale, (300, 120));
            reused += painter.scene_work().translated_text_run_reuses;
            if down == 2.5 && scale == 1.0 {
                assert!(painter.scene_work().translated_text_run_reuses > 0,
                    "fractional smooth-scroll offsets must replay proven single-baseline runs");
            }
            renderer.render(painter.last_scene(), Color::WHITE).unwrap();
            // Independent Painter and glyph atlas; full traversal also disables
            // the translated cache, so this is not replay compared with itself.
            let mut reference = Painter::new();
            let mut reference_text = TextEngine::new(diagnostic_font::plain_font());
            reference.paint_full_for_diagnostics(&root, &layout, &mut reference_text, scale, (300, 120), Interaction::default());
            reference_renderer.render(reference.last_scene(), Color::WHITE).unwrap();
            assert!(renderer.read_pixels().unwrap() == reference_renderer.read_pixels().unwrap(), "scroll offset {down}, scale {scale}");
        }
        assert!(reused > 0, "scrolling must reuse drawing commands, not just layout");
        eprintln!("translated short-text command replays across eleven scrolling frames, scale {scale}: {reused}");
        }
    }

    #[test]
    fn image_exports_count_only_the_appended_tail() {
        let mut painter = Painter::new();
        let root = Element::new(Role::Window).into_node();
        let mut text = TextEngine::new(diagnostic_font::plain_font());
        let mut layout = Layout::new();
        layout.compute(&root, rsui_layout::Size::new(20.0, 20.0), &mut text);
        painter
            .images
            .color_glyph(
                ColorGlyphKey {
                    font: 0,
                    glyph: 1,
                    size_bits: 10f32.to_bits(),
                    text_color: [255; 4],
                },
                || {
                    Some(rsui_text::ColorBitmap {
                        width: 1,
                        height: 1,
                        pixels: vec![255; 4],
                        ..Default::default()
                    })
                },
            )
            .unwrap();
        painter.paint(&root, &layout, &mut text, 1.0, (20, 20));
        assert_eq!(
            painter.scene_work().image_exports,
            Payload { count: 1, bytes: 4 }
        );
        painter.paint(&root, &layout, &mut text, 1.0, (20, 20));
        assert_eq!(painter.scene_work().image_exports, Payload::default());
        assert_eq!(
            painter.scene_work().pending_images,
            Payload { count: 1, bytes: 4 }
        );
        let census = painter.retained_storage();
        assert_eq!(census.image_payload.len_bytes, 4);
        assert!(census.image_payload.capacity_bytes >= 4);
    }

    #[test]
    fn retry_counts_discarded_work_and_caret_counts_copies() {
        let root = Element::new(Role::Window)
            .child(Element::new(Role::Text).name("ABC"))
            .into_node();
        let mut text = TextEngine::new(diagnostic_font::plain_font());
        let mut layout = Layout::new();
        layout.compute(&root, rsui_layout::Size::new(100.0, 40.0), &mut text);
        let mut painter = Painter {
            retry_for_test: true,
            ..Default::default()
        };
        painter.paint(&root, &layout, &mut text, 1.0, (100, 40));
        let work = painter.scene_work();
        assert_eq!(work.paint_passes, 2);
        assert_eq!(work.node_enters, 4);
        assert_eq!(work.quads_built, work.output.quads * 2);
        assert_eq!(work.batches_built, work.output.batches * 2);
        assert_eq!(
            work.atlas_after.generation,
            work.atlas_before.generation + 1
        );
        assert_eq!(work.pending_glyphs, work.glyph_exports);
        assert_eq!(work.glyph_exports.count, 3);
        // The discarded first pass must not leave stale atlas content in final pixels.
        let mut fresh = Painter::new();
        let mut fresh_text = TextEngine::new(diagnostic_font::plain_font());
        let mut fresh_layout = Layout::new();
        fresh_layout.compute(&root, rsui_layout::Size::new(100.0, 40.0), &mut fresh_text);
        fresh.paint(&root, &fresh_layout, &mut fresh_text, 1.0, (100, 40));
        for flipping in [false, true] {
            let make = || {
                if flipping {
                    crate::Renderer::offscreen_flipping((100, 40))
                } else {
                    crate::Renderer::offscreen((100, 40))
                }
                .unwrap()
            };
            let mut candidate = make();
            let mut reference = make();
            reference.set_always_redraw(true);
            candidate
                .render(&mut painter.scene.clone(), Color::WHITE)
                .unwrap();
            reference
                .render(&mut fresh.scene.clone(), Color::WHITE)
                .unwrap();
            assert_eq!(
                candidate.read_pixels().unwrap(),
                reference.read_pixels().unwrap()
            );
        }

        assert!(painter.show_caret(false).is_none());
        assert_eq!(painter.scene_work(), work);
        // Private fixture marks a command as the caret; exercise the real reuse path.
        painter.keep_caret_scene(Some(0));
        let bytes = painter.retained_storage().kept_caret.len_bytes();
        let scene_before = painter.scene.clone();
        painter.show_caret(false).unwrap();
        let reused = painter.scene_work();
        assert_eq!(reused.path, ScenePath::CaretReuse);
        assert_eq!(reused.serial, work.serial + 1);
        assert_eq!(
            (
                reused.paint_passes,
                reused.node_enters,
                reused.ancestor_passes,
                reused.paragraph_callbacks,
                reused.quads_built,
                reused.batches_built
            ),
            (0, 0, 0, 0, 0, 0)
        );
        assert_eq!(reused.caret_copy_bytes, bytes);
        assert_eq!(reused.output.quads, work.output.quads - 1);
        assert_eq!(reused.glyph_exports, Payload::default());
        assert_eq!(reused.pending_glyphs, work.pending_glyphs);
        painter.show_caret(true).unwrap();
        assert_eq!(painter.scene, scene_before);
        painter.renderer_replaced();
        assert_eq!(painter.scene_work(), SceneWork::default());
    }
}

#[cfg(test)]
#[test]
fn a_scene_says_where_something_is_drawn() {
    let everywhere = PixelRect { left: 0, top: 0, right: 400, bottom: 300 };
    let quad = |rect: [f32; 4], glyph: [u16; 4], alpha: u8| Quad { rect, glyph, color: [0, 0, 0, alpha], radius: 0.0 };
    let scene = Scene {
        quads: vec![
            // A box, one that can't be seen, a glyph, a picture and something turned.
            quad([10.0, 10.0, 100.0, 50.0], [0; 4], 255),
            quad([200.0, 10.0, 100.0, 50.0], [0; 4], 0),
            quad([10.0, 100.0, 8.0, 12.0], [5, 5, 8, 12], 255),
            quad([200.0, 100.0, 50.0, 50.0], [0; 4], 0),
            quad([300.0, 200.0, 20.0, 20.0], [0; 4], 255),
        ],
        batches: vec![
            Batch { first: 0, count: 3, clip: everywhere, image: 0, transform: 0, filter: 0 , round: 0},
            Batch { first: 3, count: 1, clip: everywhere, image: 7, transform: 0, filter: 0 , round: 0},
            Batch {
                first: 4,
                count: 1,
                clip: PixelRect { left: 290, top: 190, right: 340, bottom: 240 },
                image: 0,
                transform: 1,
                filter: 0,
                round: 0,
            },
        ],
        ..Scene::default()
    };
    assert!(scene.draws_at(50.0, 30.0), "in the box");
    assert!(!scene.draws_at(150.0, 30.0), "beside it");
    assert!(!scene.draws_at(250.0, 30.0), "in what can't be seen");
    assert!(scene.draws_at(14.0, 105.0), "on the glyph");
    assert!(scene.draws_at(19.0, 105.0), "just past it, as between letters");
    assert!(!scene.draws_at(25.0, 105.0), "well past it");
    assert!(scene.draws_at(220.0, 120.0), "on the picture, whatever color its quad says");
    assert!(scene.draws_at(335.0, 235.0), "what's turned counts all over where it's cut to");
    assert!(!scene.draws_at(380.0, 280.0));
    let cut = Scene {
        quads: vec![quad([0.0, 0.0, 400.0, 300.0], [0; 4], 255)],
        batches: vec![Batch { first: 0, count: 1, clip: PixelRect { left: 0, top: 0, right: 100, bottom: 100 }, image: 0, transform: 0, filter: 0 , round: 0}],
        ..Scene::default()
    };
    assert!(cut.draws_at(50.0, 50.0));
    assert!(!cut.draws_at(150.0, 150.0), "what's cut off isn't drawn");
    assert!(!Scene::default().draws_at(0.0, 0.0), "nothing's drawn before the first frame");
}

#[cfg(test)]
#[test]
fn background_position_with_an_edge_and_how_far_for_one_way() {
    // Bootstrap's select arrow, `right 12px center`, and the same the other way round.
    assert_eq!(background_position("right 12px center", (100.0, 20.0), 1.0), (88.0, 10.0));
    assert_eq!(background_position("center right 12px", (100.0, 20.0), 1.0), (88.0, 10.0));
    assert_eq!(background_position("left bottom 5px", (100.0, 20.0), 2.0), (0.0, 10.0));
    assert_eq!(background_position("right 0px bottom 0px", (100.0, 20.0), 1.0), (100.0, 20.0));
}

#[cfg(test)]
mod selection_clip_bounds {
    use super::*;
    #[test]
    fn guards_preserve_edges_and_unknown_bounds() {
        let clip = PixelRect {
            left: 0,
            top: 0,
            right: 100,
            bottom: 60,
        };
        for top in [-1.0, 0.0, 59.5, 60.0, 61.0] {
            assert!(!selection_line_outside(clip, top, 1.0));
            assert!(!selection_rect_outside(clip, [1.0, top, 4.0, 1.0]));
        }
        assert!(selection_line_outside(clip, -10.0, 2.0));
        assert!(selection_line_outside(clip, 63.0, 1.0));
        assert!(selection_rect_outside(clip, [105.0, 1.0, 4.0, 1.0]));
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(!selection_line_outside(clip, bad, 1.0));
            assert!(!selection_rect_outside(clip, [bad, 0.0, 1.0, 1.0]));
        }
        assert!(!selection_line_outside(clip, 100.0, -2.0));
        assert!(!selection_rect_outside(clip, [200.0, 0.0, -3.0, 1.0]));
    }
}

#[cfg(test)]
#[test]
fn paragraph_key_capacity_stays_bounded_when_text_and_run_sizes_alternate() {
    let mut memo = ParagraphMemo {
        seen: 1,
        previous: 1,
        ..Default::default()
    };
    let mut text = RichText::default();
    text.push(&"A".repeat(24000), TextStyle::new(12.0));
    assert!(memo.remember(&text, 140.0, None));
    text.clear();
    for _ in 0..(20000 / size_of::<rsui_text::TextRun>()) {
        text.push("AB", TextStyle::new(12.0));
    }
    assert!(memo.remember(&text, 140.0, None));
    assert!(
        memo.source.text.capacity() + memo.source.runs.capacity() * size_of::<rsui_text::TextRun>()
            <= PARAGRAPH_KEY_BYTES
    );
    text.runs[0].style.size = f32::NAN;
    assert!(!memo.remember(&text, 140.0, None));
    assert_eq!(memo.source.text.capacity(), 0);
    assert_eq!(memo.source.runs.capacity(), 0);
}
