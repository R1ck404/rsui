//! How elements look.

use std::sync::LazyLock;

use rsui_core::{Element, PropValue, Props, Symbol};

/// A color, with each part from 0 to 255.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// How opaque it is: 255 hides what's behind it, and 0 is invisible.
    pub a: u8,
}

impl Color {
    /// The color this far from this one to another, from 0 to 1.
    pub(crate) fn mix(self, other: Self, amount: f32) -> Self {
        let part = |from: u8, to: u8| {
            (f32::from(from) + (f32::from(to) - f32::from(from)) * amount).round() as u8
        };
        Self {
            r: part(self.r, other.r),
            g: part(self.g, other.g),
            b: part(self.b, other.b),
            a: part(self.a, other.a),
        }
    }

    pub const BLACK: Self = Self::hex(0x000000);
    pub const WHITE: Self = Self::hex(0xFFFFFF);
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// An opaque color from a number like `0x1F6FEB`, as in CSS.
    pub const fn hex(value: u32) -> Self {
        Self {
            r: (value >> 16) as u8,
            g: (value >> 8) as u8,
            b: value as u8,
            a: 255,
        }
    }

    /// This color, with each part multiplied by how opaque it is, which is what blending
    /// needs in order to get edges right.
    #[inline(always)]
    pub(crate) fn premultiplied(self) -> [u8; 4] {
        let multiply = |part: u8| ((u32::from(part) * u32::from(self.a) + 127) / 255) as u8;
        [multiply(self.r), multiply(self.g), multiply(self.b), self.a]
    }

    /// A color from text like `#1f6feb`, `#fff` or `#00000066`, or `white`, `black` or `transparent`.
    pub fn from_text(text: &str) -> Option<Self> {
        Some(match text {
            "white" => Self::WHITE,
            "black" => Self::BLACK,
            "transparent" => Self::TRANSPARENT,
            text => {
                let digits = text.strip_prefix('#')?;
                let number = u32::from_str_radix(digits, 16).ok()?;
                let part = |shift: u32| (number >> shift) as u8;
                let short = |shift: u32| ((number >> shift) & 15) as u8 * 17;
                match digits.len() {
                    3 => Self::rgba(short(8), short(4), short(0), 255),
                    4 => Self::rgba(short(12), short(8), short(4), short(0)),
                    6 => Self::hex(number),
                    8 => Self::rgba(part(24), part(16), part(8), part(0)),
                    _ => return None,
                }
            }
        })
    }

    /// This color, the way the `background` prop and the like hold it.
    pub fn to_prop(self) -> i64 {
        i64::from(u32::from_be_bytes([self.r, self.g, self.b, self.a]))
    }

    pub(crate) fn from_prop(value: &PropValue) -> Option<Self> {
        let PropValue::Int(value) = value else {
            return None;
        };
        let [r, g, b, a] = u32::try_from(*value).ok()?.to_be_bytes();
        Some(Self { r, g, b, a })
    }
}

/// How an element looks, apart from where it goes. Convert it to [`Props`], usually with
/// [`rsui_layout::style!`] alongside a layout style. An element draws nothing of its own
/// but its text unless it's given a background.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Paint {
    background: Option<Color>,
    text: Option<Color>,
    corner_radius: Option<f32>,
    opacity: Option<f32>,
    hover: Option<Color>,
    pressed: Option<Color>,
    border: Option<(Color, f32)>,
    shadow: Option<(Color, f32, f32)>,
}

impl Paint {
    pub const fn new() -> Self {
        Self {
            background: None,
            text: None,
            corner_radius: None,
            opacity: None,
            hover: None,
            pressed: None,
            border: None,
            shadow: None,
        }
    }

    /// The background while the pointer is over this element. This costs nothing until then,
    /// and changing to it doesn't render any component.
    pub const fn hover(mut self, color: Color) -> Self {
        self.hover = Some(color);
        self
    }

    /// The background while this element is being pressed.
    pub const fn pressed(mut self, color: Color) -> Self {
        self.pressed = Some(color);
        self
    }

    /// An outline just inside the element's edge, this many pixels wide.
    pub const fn border(mut self, color: Color, width: f32) -> Self {
        self.border = Some((color, width));
        self
    }

    pub const fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    pub const fn text(mut self, color: Color) -> Self {
        self.text = Some(color);
        self
    }

    /// How see-through this and everything in it is, from 0 to 1.
    ///
    /// To fade something, animate a number and give it to the element with `.prop("opacity", number)`.
    /// A number that changes can't go through `style!`, which keeps what it worked out the first time.
    pub const fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = Some(opacity);
        self
    }

    /// Rounds the corners of the background, by this many pixels.
    pub const fn corner_radius(mut self, pixels: f32) -> Self {
        self.corner_radius = Some(pixels);
        self
    }

    /// Puts a shadow behind this, `blur` pixels soft, and `down` pixels below it.
    pub const fn shadow(mut self, color: Color, blur: f32, down: f32) -> Self {
        self.shadow = Some((color, blur, down));
        self
    }
}

impl From<Paint> for Props {
    fn from(paint: Paint) -> Props {
        let mut entries: Vec<(Symbol, PropValue)> = Vec::new();
        if let Some(color) = paint.background {
            entries.push((Symbol::new("background"), PropValue::Int(color.to_prop())));
        }
        if let Some(color) = paint.text {
            entries.push((Symbol::new("text_color"), PropValue::Int(color.to_prop())));
        }
        if let Some(opacity) = paint.opacity {
            entries.push((Symbol::new("opacity"), PropValue::Float(f64::from(opacity))));
        }
        if let Some(radius) = paint.corner_radius {
            entries.push((
                Symbol::new("corner_radius"),
                PropValue::Float(f64::from(radius)),
            ));
        }
        if let Some(color) = paint.hover {
            entries.push((
                Symbol::new(HOVER_BACKGROUND),
                PropValue::Int(color.to_prop()),
            ));
        }
        if let Some(color) = paint.pressed {
            entries.push((
                Symbol::new(PRESSED_BACKGROUND),
                PropValue::Int(color.to_prop()),
            ));
        }
        if let Some((color, blur, down)) = paint.shadow {
            entries.push((Symbol::new("shadow_color"), PropValue::Int(color.to_prop())));
            entries.push((
                Symbol::new("shadow_blur"),
                PropValue::Float(f64::from(blur)),
            ));
            entries.push((Symbol::new("shadow_y"), PropValue::Float(f64::from(down))));
        }
        if let Some((color, width)) = paint.border {
            entries.push((Symbol::new("border_color"), PropValue::Int(color.to_prop())));
            entries.push((
                Symbol::new("border_width"),
                PropValue::Float(f64::from(width)),
            ));
        }
        entries.into_iter().collect()
    }
}

const HOVER_BACKGROUND: &str = "hover_background";
const PRESSED_BACKGROUND: &str = "pressed_background";

/// Names looked up once, so finding a prop is a pointer comparison.
pub(crate) struct Names {
    hover: Symbol,
    pressed: Symbol,
    pub(crate) rows: Symbol,
    pub(crate) value: Symbol,
    pub(crate) placeholder: Symbol,
    /// Finds which part of a [`Look`] a prop is for, in one step however many there are.
    parts: PartTable,
}

/// Which part of a [`Look`] a prop sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Disabled,
    /// Whether what's disabled looks as it says, rather than faded.
    PlainDisabled,
    Background,
    Hover,
    Pressed,
    BorderColor,
    BorderWidth,
    BorderSide,
    BorderSideColor,
    CornerRadius,
    Opacity,
    TextColor,
    PaddingLeft,
    PaddingRight,
    PaddingTop,
    PaddingBottom,
    /// Something that has a shadow, whose details are read when it's drawn.
    Shadow,
    /// Text that's arranged in some way, whose details are read when it's drawn.
    Arranged,
    /// Something that's turned or resized, whose details are read when it's drawn.
    Transformed,
    /// A background that goes from one color to another, whose details are read when it's drawn.
    Gradient,
    /// A line around the outside of something, whose details are read when it's drawn.
    Outline,
    /// Whether a border is dashes or dots rather than a line.
    BorderStyle,
    /// A picture behind something, whose details are read when it's drawn.
    BackgroundImage,
    /// What colors go through, whose details are read when it's drawn.
    Filter,
    /// Corners that aren't all the same circle, or a shape it's cut to, whose details are read when it's drawn.
    Shaped,
    /// A shadow under text, whose details are read when it's drawn.
    TextShadow,
    /// What a control drawn in the platform's look has that changes as it's used (`ripple`, `focus_line`, `focus_background`,
    /// `focus_ring`, `pressed_text_color`), whose details are read when it's drawn.
    Native,
}

/// How many places [`PartTable`] has, as a power of two. It's mostly empty, so that a prop that
/// isn't for drawing, which is most of them, is nearly always found out at the first look.
const PART_TABLE_BITS: u32 = 7;

/// A small hash table from symbols to [`Part`]s.
struct PartTable([(usize, Option<Part>); 1 << PART_TABLE_BITS]);

impl std::fmt::Debug for PartTable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PartTable")
    }
}

impl PartTable {
    #[inline(always)]
    fn place_of(address: usize) -> usize {
        // Symbols are 16 bytes apart at the least, so the low bits say nothing.
        (address >> 4).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (usize::BITS - PART_TABLE_BITS)
    }

    fn new(entries: &[(Symbol, Part)]) -> Self {
        let mut table = [(0, None); 1 << PART_TABLE_BITS];
        for (symbol, part) in entries {
            let address = symbol.address();
            let mut place = Self::place_of(address);
            while table[place].1.is_some() {
                place = (place + 1) & ((1 << PART_TABLE_BITS) - 1);
            }
            table[place] = (address, Some(*part));
        }
        Self(table)
    }

    #[inline(always)]
    fn get(&self, symbol: Symbol) -> Option<Part> {
        let address = symbol.address();
        let mut place = Self::place_of(address);
        loop {
            let (key, part) = self.0[place];
            if key == address || part.is_none() {
                return part;
            }
            place = (place + 1) & ((1 << PART_TABLE_BITS) - 1);
        }
    }
}

/// The props drawing reads, and what each is about.
const PARTS: &[(&str, Part)] = &[
    ("background", Part::Background),
    (HOVER_BACKGROUND, Part::Hover),
    (PRESSED_BACKGROUND, Part::Pressed),
    ("border_color", Part::BorderColor),
    ("border_width", Part::BorderWidth),
    ("border_top_width", Part::BorderSide),
    ("border_right_width", Part::BorderSide),
    ("border_bottom_width", Part::BorderSide),
    ("border_left_width", Part::BorderSide),
    ("border_top_color", Part::BorderSideColor),
    ("border_right_color", Part::BorderSideColor),
    ("border_bottom_color", Part::BorderSideColor),
    ("border_left_color", Part::BorderSideColor),
    ("corner_radius", Part::CornerRadius),
    ("opacity", Part::Opacity),
    ("disabled", Part::Disabled),
    ("plain_disabled", Part::PlainDisabled),
    ("text_color", Part::TextColor),
    ("padding_left", Part::PaddingLeft),
    ("padding_right", Part::PaddingRight),
    ("padding_top", Part::PaddingTop),
    ("padding_bottom", Part::PaddingBottom),
    ("shadow_blur", Part::Shadow),
    ("shadows", Part::Shadow),
    ("text_align", Part::Arranged),
    ("ellipsis", Part::Arranged),
    ("no_wrap", Part::Arranged),
    ("underline", Part::Arranged),
    ("line_through", Part::Arranged),
    ("overline", Part::Arranged),
    ("letter_spacing", Part::Arranged),
    ("word_spacing", Part::Arranged),
    ("scale", Part::Transformed),
    ("scale_x", Part::Transformed),
    ("scale_y", Part::Transformed),
    ("rotate", Part::Transformed),
    ("transform", Part::Transformed),
    ("background_gradient", Part::Gradient),
    ("background_image", Part::BackgroundImage),
    ("css_filter", Part::Filter),
    ("corner_radii", Part::Shaped),
    ("text_shadow", Part::TextShadow),
    ("clip_path", Part::Shaped),
    ("outline_width", Part::Outline),
    ("border_style", Part::BorderStyle),
    ("ripple", Part::Native),
    ("focus_line", Part::Native),
    ("focus_background", Part::Native),
    ("focus_ring", Part::Native),
    ("pressed_text_color", Part::Native),
];

pub(crate) static NAMES: LazyLock<Names> = LazyLock::new(|| Names {
    hover: Symbol::new(HOVER_BACKGROUND),
    pressed: Symbol::new(PRESSED_BACKGROUND),
    rows: Symbol::new("rows"),
    value: Symbol::new("value"),
    placeholder: Symbol::new("placeholder"),
    parts: PartTable::new(&PARTS.iter().map(|&(name, part)| (Symbol::new(name), part)).collect::<Vec<_>>()),
});

/// How much something that can't be used fades.
const DISABLED_FADE: f32 = 0.5;

/// Whether this element changes color under the pointer.
pub fn has_hover(element: &Element) -> bool {
    element.props.get_symbol(NAMES.hover).is_some()
}

/// Whether this element changes color while pressed.
pub fn has_pressed(element: &Element) -> bool {
    element.props.get_symbol(NAMES.pressed).is_some()
}

/// All of the props that affect how an element is drawn, read in one go.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Look {
    pub(crate) background: Option<Color>,
    pub(crate) hover: Option<Color>,
    pub(crate) pressed: Option<Color>,
    pub(crate) border_color: Option<Color>,
    pub(crate) border_width: f32,
    /// Whether there's a border on one side that isn't on the others.
    pub(crate) border_sides: bool,
    /// Whether a side of a border that's as wide all around has a color of its own, as a spinner's gap does.
    pub(crate) border_side_colors: bool,
    pub(crate) corner_radius: f32,
    /// How much this fades what's in it, which is 1 minus its opacity, so that nothing is 0.
    pub(crate) faded: f32,
    pub(crate) text_color: Option<Color>,
    pub(crate) padding_left: f32,
    pub(crate) padding_right: f32,
    pub(crate) padding_top: f32,
    pub(crate) padding_bottom: f32,
    /// Whether this has a shadow. See [`Shadow`].
    pub(crate) shadowed: bool,
    /// Whether this is text that's arranged in some way. See [`Arrangement`].
    pub(crate) arranged: bool,
    /// Whether this is turned or resized. See [`transform_of`](crate::transform::transform_of).
    pub(crate) transformed: bool,
    /// Whether its background goes from one color to another, as `background_gradient` says.
    pub(crate) gradient: bool,
    /// Whether there's a picture behind it, as `background_image` says.
    pub(crate) background_image: bool,
    /// Whether its colors, and what's in it, go through a filter, as `css_filter` says.
    pub(crate) filtered: bool,
    /// Whether its corners aren't all the same circle, or it's cut to a shape, as `corner_radii` and `clip_path` say.
    pub(crate) shaped: bool,
    /// Whether its text has a shadow, as `text_shadow` says.
    pub(crate) text_shadow: bool,
    /// Whether there's a line around the outside of it, as `outline_width` says. See [`Outline`].
    pub(crate) outlined: bool,
    /// What its border is made of: a line, dashes or dots, as `border_style` says.
    pub(crate) border_pattern: BorderPattern,
    /// Whether it's a control in the platform's look, with a ripple or a look of its own while it has the keyboard. See
    /// [`Part::Native`].
    pub(crate) native: bool,
    /// Whether this has any of the things most elements don't: changing colors, a border or fading.
    /// Drawing checks this once, instead of checking for each of them.
    pub(crate) unusual: bool,
}

impl Look {
    pub(crate) fn read(props: &Props, names: &Names) -> Self {
        let mut look = Self::default();
        look.read_from(props, names);
        look
    }

    /// Replaces this with what these props say.
    #[inline]
    pub(crate) fn read_from(&mut self, props: &Props, names: &Names) {
        *self = Self::default();
        // Whether it's faded for being disabled, and whether it says it isn't.
        let (mut faded_for_disabled, mut plain) = (false, false);
        for (name, value) in props.iter() {
            let Some(part) = names.parts.get(name) else {
                continue;
            };
            match part {
                Part::Background => self.background = Color::from_prop(value),
                Part::PaddingLeft => self.padding_left = number(value),
                Part::PaddingRight => self.padding_right = number(value),
                Part::PaddingTop => self.padding_top = number(value),
                Part::PaddingBottom => self.padding_bottom = number(value),
                Part::CornerRadius => self.corner_radius = number(value),
                Part::TextColor => self.text_color = Color::from_prop(value),
                Part::Hover => {
                    self.hover = Color::from_prop(value);
                    self.unusual = true;
                }
                Part::Pressed => {
                    self.pressed = Color::from_prop(value);
                    self.unusual = true;
                }
                Part::BorderColor => {
                    self.border_color = Color::from_prop(value);
                    self.unusual = true;
                }
                Part::BorderWidth => self.border_width = number(value),
                Part::BorderSide => {
                    self.border_sides = true;
                    self.unusual = true;
                }
                Part::BorderSideColor => {
                    self.border_side_colors = true;
                    self.unusual = true;
                }
                // Something that can't be used looks it, unless it says how it should look.
                Part::Disabled => {
                    if *value == PropValue::Bool(true) && self.faded == 0.0 {
                        self.faded = DISABLED_FADE;
                        self.unusual = true;
                        faded_for_disabled = true;
                    }
                }
                Part::PlainDisabled => plain = *value == PropValue::Bool(true),
                Part::Opacity => {
                    faded_for_disabled = false;
                    self.faded = 1.0 - number(value).clamp(0.0, 1.0);
                    self.unusual = true;
                }
                Part::Shadow => {
                    self.shadowed = true;
                    self.unusual = true;
                }
                Part::Arranged => self.arranged = true,
                Part::Transformed => {
                    self.transformed = true;
                    self.unusual = true;
                }
                Part::Gradient => {
                    self.gradient = true;
                    self.unusual = true;
                }
                Part::BackgroundImage => {
                    self.background_image = true;
                    self.unusual = true;
                }
                Part::Filter => {
                    self.filtered = true;
                    self.unusual = true;
                }
                Part::Shaped => {
                    self.shaped = true;
                    self.unusual = true;
                }
                Part::TextShadow => self.text_shadow = true,
                Part::Native => {
                    self.native = true;
                    self.unusual = true;
                }
                Part::Outline => {
                    self.outlined = number(value) > 0.0;
                    self.unusual |= self.outlined;
                }
                Part::BorderStyle => {
                    self.border_pattern = match value {
                        PropValue::Str(style) if style.as_str() == "dashed" => {
                            BorderPattern::Dashes
                        }
                        PropValue::Str(style) if style.as_str() == "dotted" => BorderPattern::Dots,
                        PropValue::Str(style) if style.as_str() == "double" => {
                            BorderPattern::Double
                        }
                        PropValue::Str(style) if style.as_str() == "groove" => {
                            BorderPattern::Groove
                        }
                        PropValue::Str(style) if style.as_str() == "ridge" => BorderPattern::Ridge,
                        PropValue::Str(style) if style.as_str() == "inset" => BorderPattern::Inset,
                        PropValue::Str(style) if style.as_str() == "outset" => {
                            BorderPattern::Outset
                        }
                        _ => BorderPattern::Line,
                    };
                }
            }
        }
        if faded_for_disabled && plain {
            self.faded = 0.0;
        }
    }
}

pub(crate) fn signed_number(value: &PropValue) -> f32 {
    match value {
        PropValue::Float(value) => *value as f32,
        PropValue::Int(value) => *value as f32,
        _ => 0.0,
    }
}

/// What a border is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum BorderPattern {
    #[default]
    Line,
    /// Dashes twice as long as the border is wide, with about as much between them.
    Dashes,
    /// Dots as big as the border is wide, with about as much between them: round, unless that's too small to see.
    Dots,
    /// Two lines, a third as wide as the border each, with a third between them.
    Double,
    /// Carved in: darker at the top and left of its outer half, and the other way around inside.
    Groove,
    /// Standing out: as a groove, the other way around.
    Ridge,
    /// Pressed in: darker at the top and left.
    Inset,
    /// Standing out: darker at the bottom and right.
    Outset,
}

/// A line around the outside of something, which takes no room, as CSS's `outline` is. Few things have one, so this is only read
/// when it's drawn.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Outline {
    pub(crate) width: f32,
    pub(crate) color: Color,
    /// How far out from the edge it starts.
    pub(crate) offset: f32,
}

impl Outline {
    pub(crate) fn of(element: &Element) -> Self {
        let read = |name: &str| element.props.get(name).map_or(0.0, signed_number);
        Self {
            width: read("outline_width"),
            color: element
                .props
                .get("outline_color")
                .and_then(Color::from_prop)
                .unwrap_or(Color::BLACK),
            offset: read("outline_offset"),
        }
    }
}

/// A shadow. Very few things have one, so this is only read when it's drawn, and a look doesn't get any bigger for it, as
/// looks are made for everything drawn.
pub(crate) struct Shadow {
    pub(crate) color: Color,
    pub(crate) blur: f32,
    pub(crate) x: f32,
    pub(crate) y: f32,
    /// How much bigger than what it's behind it is on each side, before it's blurred.
    pub(crate) spread: f32,
    /// Whether it's inside what it's on, as CSS's `inset` has it, rather than behind it.
    pub(crate) inset: bool,
}

impl Shadow {
    /// All of an element's shadows, from the one on top: those that `shadows` lists, one to a line as `x y blur spread color`
    /// with the color as a number and `inset` after it if it's inside, or the one the other shadow props say.
    pub(crate) fn all(element: &Element) -> Vec<Self> {
        static SHADOWS: LazyLock<Symbol> = LazyLock::new(|| Symbol::new("shadows"));
        let Some(PropValue::Str(text)) = element.props.get_symbol(*SHADOWS) else {
            return vec![Self::of(element)];
        };
        text.as_str()
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let mut number = || parts.next()?.parse::<f32>().ok();
                let (x, y, blur, spread) = (number()?, number()?, number()?, number()?);
                let color = parts.next()?.parse::<u32>().ok()?;
                let [r, g, b, a] = color.to_be_bytes();
                Some(Self {
                    color: Color::rgba(r, g, b, a),
                    blur: blur.max(0.0),
                    x,
                    y,
                    spread,
                    inset: parts.next() == Some("inset"),
                })
            })
            .collect()
    }

    pub(crate) fn of(element: &Element) -> Self {
        let names = &*SHADOW_NAMES;
        let number = |name| element.props.get_symbol(name).map_or(0.0, signed_number);
        Self {
            color: element
                .props
                .get_symbol(names.0)
                .and_then(Color::from_prop)
                .unwrap_or(Color::rgba(0, 0, 0, 0x40)),
            blur: number(names.1).max(0.0),
            x: number(names.2),
            y: number(names.3),
            spread: element
                .props
                .get("shadow_spread")
                .map_or(0.0, signed_number),
            inset: false,
        }
    }
}

static SHADOW_NAMES: LazyLock<(Symbol, Symbol, Symbol, Symbol)> = LazyLock::new(|| {
    (
        Symbol::new("shadow_color"),
        Symbol::new("shadow_blur"),
        Symbol::new("shadow_x"),
        Symbol::new("shadow_y"),
    )
});

/// How text is arranged. Like a shadow, this is only read for what has any of it.
pub(crate) struct Arrangement {
    /// How far across to put each line, from 0 at the left to 1 at the right, if it's been said.
    pub(crate) along: Option<f32>,
    pub(crate) ellipsis: bool,
    pub(crate) no_wrap: bool,
    /// Lines under, through and over it, as CSS's `text-decoration` has them.
    pub(crate) decoration: Decoration,
}

/// What lines go with text, and how they look, as CSS's `text-decoration` says. Everything but which lines is what the text
/// says itself when it isn't said.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Decoration {
    pub(crate) under: bool,
    pub(crate) through: bool,
    pub(crate) over: bool,
    pub(crate) color: Option<Color>,
    pub(crate) style: LineStyle,
    /// How thick, in layout pixels.
    pub(crate) thickness: Option<f32>,
    /// How far below the baseline a line under it goes, in layout pixels.
    pub(crate) offset: Option<f32>,
}

impl Decoration {
    pub(crate) fn any(&self) -> bool {
        self.under || self.through || self.over
    }
}

/// How a line under, through or over text is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LineStyle {
    #[default]
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

impl Arrangement {
    pub(crate) fn of(element: &Element) -> Self {
        let names = &*ARRANGEMENT_NAMES;
        let on = |name| element.props.get_symbol(name) == Some(&PropValue::Bool(true));
        Self {
            along: match element.props.get_symbol(names.0) {
                Some(PropValue::Str(side)) if side.as_str() == "center" => Some(0.5),
                Some(PropValue::Str(side)) if side.as_str() == "right" => Some(1.0),
                Some(PropValue::Str(side)) if side.as_str() == "left" => Some(0.0),
                _ => None,
            },
            ellipsis: on(names.1),
            no_wrap: on(names.2),
            decoration: Decoration::of(element),
        }
    }
}

impl Decoration {
    pub(crate) fn of(element: &Element) -> Self {
        static NAMES: LazyLock<[Symbol; 7]> = LazyLock::new(|| {
            [
                Symbol::new("underline"),
                Symbol::new("line_through"),
                Symbol::new("overline"),
                Symbol::new("decoration_color"),
                Symbol::new("decoration_style"),
                Symbol::new("decoration_thickness"),
                Symbol::new("underline_offset"),
            ]
        });
        let names = &*NAMES;
        let on = |name| element.props.get_symbol(name) == Some(&PropValue::Bool(true));
        let length = |name| match element.props.get_symbol(name) {
            Some(PropValue::Float(value)) => Some(*value as f32),
            Some(PropValue::Int(value)) => Some(*value as f32),
            _ => None,
        };
        Self {
            under: on(names[0]),
            through: on(names[1]),
            over: on(names[2]),
            color: element
                .props
                .get_symbol(names[3])
                .and_then(Color::from_prop),
            style: match element.props.get_symbol(names[4]) {
                Some(PropValue::Str(style)) => match style.as_str() {
                    "double" => LineStyle::Double,
                    "dotted" => LineStyle::Dotted,
                    "dashed" => LineStyle::Dashed,
                    "wavy" => LineStyle::Wavy,
                    _ => LineStyle::Solid,
                },
                _ => LineStyle::Solid,
            },
            thickness: length(names[5]),
            offset: length(names[6]),
        }
    }
}

static ARRANGEMENT_NAMES: LazyLock<(Symbol, Symbol, Symbol, Symbol)> = LazyLock::new(|| {
    (
        Symbol::new("text_align"),
        Symbol::new("ellipsis"),
        Symbol::new("no_wrap"),
        Symbol::new("underline"),
    )
});

/// A number from a prop's value, or zero if it isn't one or is negative.
fn number(value: &PropValue) -> f32 {
    match value {
        PropValue::Float(value) => (*value as f32).max(0.0),
        PropValue::Int(value) => (*value as f32).max(0.0),
        _ => 0.0,
    }
}

#[cfg(test)]
mod parts_tests {
    use super::*;

    /// Layout leaves out what can't be seen only for what draws inside its own box, which it finds from props it knows about.
    /// Each prop that makes something draw outside it, or changes how what's in it is drawn, has to be one of those.
    #[test]
    fn layout_knows_each_prop_that_draws_outside_a_box() {
        for &(name, part) in PARTS {
            let outside = matches!(
                part,
                Part::Transformed
                    | Part::Opacity
                    | Part::Disabled
                    | Part::Filter
                    | Part::Shadow
                    | Part::TextShadow
                    | Part::Outline
            ) || name == "clip_path";
            if outside {
                assert!(rsui_layout::moves_paint(Symbol::new(name)), "{name} isn't in rsui_layout::MOVES_PAINT");
            }
        }
    }
}
