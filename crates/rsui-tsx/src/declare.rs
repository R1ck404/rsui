//! What `rsui/web` makes of CSS declarations: the props rsui takes, as `margin: 4px 8px` comes to `marginTop`, `marginRight`
//! and the rest. It's what `web.js` did itself, done here, as it's done for every rule, every `style` and every keyframe.
//!
//! Each one gives exactly what the JavaScript it replaces gave: the same props in the same order, the same ones left
//! `undefined` and the same ones left out, with numbers written as JavaScript writes them. What only JavaScript knows is asked of
//! it, through what `web.js` passes as `declaring`: what a `var()` stands for, what `content` says, and what's said once as
//! something that can't be done here. Which props have been said in `em` goes back to it too, as that's the page's.

use std::borrow::Cow;
use std::sync::OnceLock;

use rsui_js::{Args, HostFunction, Value, Vm};

use crate::css::{
    Size, Sizing, color, commas, is_space, length, number_end, number_or, number_text, parse_float,
    split_outside, text_of, trim, words,
};

type Run<T> = Result<T, Value>;

/// A prop's name: one of rsui's, or `__em_` and one of them, for what's a share of the size of the text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Key {
    Plain(&'static str),
    Em(&'static str),
}

/// What a prop holds, as JavaScript would.
#[derive(Clone, Debug)]
enum Val {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Text(Cow<'static, str>),
    /// What JavaScript made, which is what `content` says.
    Js(Value),
    /// `{ 'transition-duration': '1s' }` and the like, which is a new object each time it changes, as `===` tells.
    Parts(u32, Vec<(String, String)>),
    /// Which props `!important` made.
    Keys(Vec<Key>),
}

fn text(word: &'static str) -> Val {
    Val::Text(Cow::Borrowed(word))
}

fn owned(word: impl Into<String>) -> Val {
    Val::Text(Cow::Owned(word.into()))
}

fn size_val(size: Size) -> Val {
    match size {
        Size::Pixels(number) => Val::Number(number),
        Size::Text(text) => owned(text),
        Size::None => Val::Undefined,
    }
}

/// Whether two are the same, as `===` says, with a prop that isn't there being `undefined`.
fn same(vm: &Vm, a: Option<&Val>, b: Option<&Val>) -> bool {
    match (a.unwrap_or(&Val::Undefined), b.unwrap_or(&Val::Undefined)) {
        (Val::Undefined, Val::Undefined) | (Val::Null, Val::Null) => true,
        (Val::Bool(x), Val::Bool(y)) => x == y,
        (Val::Number(x), Val::Number(y)) => x == y,
        (Val::Text(x), Val::Text(y)) => x == y,
        (Val::Js(x), Val::Js(y)) => vm.same(*x, *y),
        (Val::Parts(x, _), Val::Parts(y, _)) => x == y,
        _ => false,
    }
}

/// Props as a JavaScript object has them: in the order they were first set, as what's set again stays where it was.
#[derive(Clone, Default, Debug)]
struct Props(Vec<(Key, Val)>);

impl Props {
    fn at(&self, key: Key) -> Option<usize> {
        self.0.iter().position(|(one, _)| *one == key)
    }

    fn get(&self, key: &'static str) -> Option<&Val> {
        self.get_key(Key::Plain(key))
    }

    fn get_key(&self, key: Key) -> Option<&Val> {
        self.at(key).map(|at| &self.0[at].1)
    }

    fn set(&mut self, key: &'static str, value: Val) {
        self.set_key(Key::Plain(key), value);
    }

    fn set_key(&mut self, key: Key, value: Val) {
        match self.at(key) {
            Some(at) => self.0[at].1 = value,
            None => self.0.push((key, value)),
        }
    }

    fn delete(&mut self, key: &'static str) {
        if let Some(at) = self.at(Key::Plain(key)) {
            self.0.remove(at);
        }
    }

    /// Whether it's `undefined`, which is what it is when it isn't there.
    fn is_undefined(&self, key: &'static str) -> bool {
        matches!(self.get(key), None | Some(Val::Undefined))
    }

    /// What `{ ...this, ...more }` comes to.
    fn merge(&mut self, more: &Props) {
        for (key, value) in &more.0 {
            self.set_key(*key, value.clone());
        }
    }

    fn to_js(&self, vm: &mut Vm) -> Run<Value> {
        let out = vm.new_object();
        for (key, value) in &self.0 {
            let atom = match key {
                Key::Plain(name) => vm.atom(name),
                Key::Em(name) => vm.atom(&format!("__em_{name}")),
            };
            let value = match value {
                Val::Undefined => Value::UNDEFINED,
                Val::Null => Value::NULL,
                Val::Bool(yes) => Value::bool(*yes),
                Val::Number(number) => Value::number(*number),
                Val::Text(text) => vm.new_string(text),
                Val::Js(value) => *value,
                Val::Parts(_, parts) => {
                    let object = vm.new_object();
                    for (name, value) in parts {
                        let atom = vm.atom(name);
                        let value = vm.new_string(value);
                        vm.set_property(object, atom, value)?;
                    }
                    object
                }
                Val::Keys(keys) => {
                    let names = keys
                        .iter()
                        .map(|key| match key {
                            Key::Plain(name) => vm.new_string(name),
                            Key::Em(name) => vm.new_string(&format!("__em_{name}")),
                        })
                        .collect();
                    vm.new_array(names)
                }
            };
            vm.set_property(out, atom, value)?;
        }
        Ok(out)
    }
}

// What words mean, which were tables in `web.js`. A word that's the name of something every object has, as `constructor` is,
// is just a word here, where in a table it found that something.

fn align(word: &str) -> Option<&'static str> {
    Some(match word {
        "flex-start" | "start" | "self-start" | "left" | "top" | "baseline" => "start",
        "center" => "center",
        "flex-end" | "end" | "self-end" | "right" | "bottom" => "end",
        "stretch" | "normal" => "stretch",
        _ => return None,
    })
}

fn justify(word: &str) -> Option<&'static str> {
    Some(match word {
        "flex-start" | "start" | "left" | "normal" => "start",
        "center" => "center",
        "flex-end" | "end" | "right" => "end",
        "space-between" => "space_between",
        "space-around" => "space_around",
        "space-evenly" => "space_evenly",
        _ => return None,
    })
}

/// What's not done here, and isn't worth saying so.
fn silent(name: &str) -> bool {
    matches!(
        name,
        "will-change"
            | "-webkit-font-smoothing"
            | "-moz-osx-font-smoothing"
            | "text-rendering"
            | "font-display"
            | "-moz-user-select"
            | "-ms-user-select"
            | "overflow-wrap"
            | "word-wrap"
            | "word-break"
            | "overscroll-behavior"
            | "scroll-behavior"
            | "isolation"
            | "backface-visibility"
            | "perspective"
            | "contain"
            | "tab-size"
            | "caret-color"
            | "resize"
            | "vertical-align"
            // What only touch screens, printing or a page's own scrolling use, which a window here doesn't have.
            | "touch-action"
            | "-webkit-tap-highlight-color"
            | "-webkit-touch-callout"
            | "print-color-adjust"
            | "-webkit-print-color-adjust"
            | "scrollbar-gutter"
            | "content-visibility"
            | "text-size-adjust"
            | "-webkit-text-size-adjust"
            | "color-scheme"
            | "forced-color-adjust"
    )
}

/// The props a border's or an outline's width is said in, which are whole pixels: see `Declarer::declare_one`.
const LINE_WIDTH_KEYS: [&str; 6] = [
    "borderWidth",
    "borderTopWidth",
    "borderRightWidth",
    "borderBottomWidth",
    "borderLeftWidth",
    "outlineWidth",
];

/// A border's width as a browser draws it: one pixel if it's less than that but something, and otherwise the whole pixels in it.
fn whole_line_width(width: f64) -> f64 {
    if width > 0.0 && width < 1.0 {
        1.0
    } else if width >= 1.0 {
        (width + 1e-6).floor()
    } else {
        width
    }
}

/// What a size can be that means it's as big as what's in it.
fn from_inside(value: &str) -> bool {
    matches!(
        value,
        "auto"
            | "none"
            | "unset"
            | "initial"
            | "inherit"
            | "fit-content"
            | "max-content"
            | "min-content"
    )
}

fn decoration_style(word: &str) -> bool {
    matches!(word, "solid" | "double" | "dotted" | "dashed" | "wavy")
}

/// The ways a border can be drawn, as CSS names them.
fn border_style(word: &str) -> bool {
    matches!(
        word,
        "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset"
    )
}

/// The kinds of border that are drawn other than as a line.
fn drawn_border(word: &str) -> Option<&'static str> {
    Some(match word {
        "dashed" => "dashed",
        "dotted" => "dotted",
        "double" => "double",
        "groove" => "groove",
        "ridge" => "ridge",
        "inset" => "inset",
        "outset" => "outset",
        _ => return None,
    })
}

/// The kinds of marker a list's items can have.
fn list_type(word: &str) -> Option<&'static str> {
    Some(match word {
        "none" => "none",
        "disc" => "disc",
        "circle" => "circle",
        "square" => "square",
        "decimal" => "decimal",
        "decimal-leading-zero" => "decimal-leading-zero",
        "lower-alpha" => "lower-alpha",
        "lower-latin" => "lower-latin",
        "upper-alpha" => "upper-alpha",
        "upper-latin" => "upper-latin",
        "lower-roman" => "lower-roman",
        "upper-roman" => "upper-roman",
        "disclosure-closed" => "disclosure-closed",
        "disclosure-open" => "disclosure-open",
        _ => return None,
    })
}

/// The words for a text size, as shares of the usual size, or of what's around it.
fn font_size_word(word: &str) -> Option<f64> {
    Some(match word {
        "xx-small" => 0.5625,
        "x-small" => 0.625,
        "small" => 0.8125,
        "medium" => 1.0,
        "large" => 1.125,
        "x-large" => 1.5,
        "xx-large" => 2.0,
        "smaller" => 0.8333,
        "larger" => 1.2,
        _ => return None,
    })
}

/// The `inherit` marker of the shorthand a longhand is part of, which saying the longhand goes over: see `inherit_marker`.
fn shorthand_marker(name: &str) -> Option<&'static str> {
    Some(match name {
        "text-decoration-line" | "text-decoration-style" | "text-decoration-color" | "text-decoration-thickness" => {
            "__inherit_text-decoration"
        }
        "background-color" | "background-image" => "__inherit_background",
        "outline-width" | "outline-style" | "outline-color" => "__inherit_outline",
        _ => return None,
    })
}

/// What each property that's passed down is called here, as `inherit` on one of those means having none of its own.
/// What `inherit` on something that isn't passed down by itself is marked with, for `web.js` to take the parent's value for it
/// (see `withInherited`). The marker's value is the property's name.
fn inherit_marker(name: &str) -> Option<(&'static str, &'static str)> {
    const MARKED: [&str; 47] = [
        "flex-wrap", "flex-direction", "flex-flow", "flex", "flex-grow", "flex-shrink", "flex-basis", "align-items",
        "align-self", "align-content", "justify-content", "gap", "row-gap", "column-gap", "width", "height", "min-width",
        "min-height", "max-width", "max-height", "background", "background-color", "background-image", "border",
        "border-color", "border-width", "border-style", "border-radius", "padding", "margin", "box-shadow", "opacity",
        "overflow", "overflow-x", "overflow-y", "position", "top", "right", "bottom", "left", "z-index", "text-decoration",
        "outline", "box-sizing", "vertical-align", "transition", "text-overflow",
    ];
    const KEYS: [&str; 47] = [
        "__inherit_flex-wrap", "__inherit_flex-direction", "__inherit_flex-flow", "__inherit_flex", "__inherit_flex-grow",
        "__inherit_flex-shrink", "__inherit_flex-basis", "__inherit_align-items", "__inherit_align-self",
        "__inherit_align-content", "__inherit_justify-content", "__inherit_gap", "__inherit_row-gap", "__inherit_column-gap",
        "__inherit_width", "__inherit_height", "__inherit_min-width", "__inherit_min-height", "__inherit_max-width",
        "__inherit_max-height", "__inherit_background", "__inherit_background-color", "__inherit_background-image",
        "__inherit_border", "__inherit_border-color", "__inherit_border-width", "__inherit_border-style",
        "__inherit_border-radius", "__inherit_padding", "__inherit_margin", "__inherit_box-shadow", "__inherit_opacity",
        "__inherit_overflow", "__inherit_overflow-x", "__inherit_overflow-y", "__inherit_position", "__inherit_top",
        "__inherit_right", "__inherit_bottom", "__inherit_left", "__inherit_z-index", "__inherit_text-decoration",
        "__inherit_outline", "__inherit_box-sizing", "__inherit_vertical-align", "__inherit_transition",
        "__inherit_text-overflow",
    ];
    let at = MARKED.iter().position(|one| *one == name)?;
    Some((MARKED[at], KEYS[at]))
}

fn inherited_key(name: &str) -> Option<&'static str> {
    Some(match name {
        "color" => "color",
        "font-family" => "fontFamily",
        "font-size" => "fontSize",
        "font-weight" => "fontWeight",
        "font-style" => "fontStyle",
        "line-height" => "lineHeight",
        "text-align" => "textAlign",
        "cursor" => "cursor",
        "visibility" => "__visibility",
        "text-transform" => "__case",
        _ => return None,
    })
}

/// `monospace` stays as it is, as how big it's drawn depends on it (see `monospaceSize` in `web.js`), and rsui finds Consolas for
/// it.
fn generic_font(name: &str) -> Option<&'static str> {
    // `toLowerCase()` of these only comes from ASCII, as none of them has a `k` in it, which the Kelvin sign is one of.
    const FONTS: [(&str, &str); 6] = [
        ("sans-serif", "Arial"),
        ("san-serif", "Arial"),
        ("serif", "Times New Roman"),
        ("system-ui", "Segoe UI"),
        ("cursive", "Comic Sans MS"),
        ("fantasy", "Impact"),
    ];
    FONTS
        .iter()
        .find(|(generic, _)| name.eq_ignore_ascii_case(generic))
        .map(|(_, font)| *font)
}

/// What's drawn differently from a solid outline: dashes, dots, or `auto`, the browser's focus ring (see `paint_auto_outline` in
/// rsui-render).
fn set_outline_style(props: &mut Props, style: &str) {
    match style {
        "dashed" => props.set("outlineStyle", text("dashed")),
        "dotted" => props.set("outlineStyle", text("dotted")),
        "auto" => props.set("outlineStyle", text("auto")),
        _ => props.delete("outlineStyle"),
    }
}

/// `text.toLowerCase() === 'currentcolor'`, which only ASCII can be, as nothing else lowercases to these letters.
fn is_current_color(text: &str) -> bool {
    text.eq_ignore_ascii_case("currentcolor")
}

/// What `font-feature-settings` says of `tnum`, digits all as wide as each other, if it says anything: on, unless it's `0` or
/// `off`.
fn tnum_setting(value: &str) -> Option<bool> {
    value.split(',').map(trim).find_map(|part| {
        let rest = part.strip_prefix("\"tnum\"").or_else(|| part.strip_prefix("'tnum'"))?;
        Some(!matches!(trim(rest), "0" | "off"))
    })
}

/// What's said of each side's border width, taken away, as one width for all of them says over it.
fn without_border_sides(props: &mut Props) {
    for side in 0..4 {
        props.set(side_key("width", side), Val::Undefined);
    }
}

fn side_key(kind: &str, side: usize) -> &'static str {
    match (kind, side) {
        ("margin", 0) => "marginTop",
        ("margin", 1) => "marginRight",
        ("margin", 2) => "marginBottom",
        ("margin", _) => "marginLeft",
        ("padding", 0) => "paddingTop",
        ("padding", 1) => "paddingRight",
        ("padding", 2) => "paddingBottom",
        ("padding", _) => "paddingLeft",
        ("__auto", 0) => "__autoTop",
        ("__auto", 1) => "__autoRight",
        ("__auto", 2) => "__autoBottom",
        ("__auto", _) => "__autoLeft",
        ("width", 0) => "borderTopWidth",
        ("width", 1) => "borderRightWidth",
        ("width", 2) => "borderBottomWidth",
        ("width", _) => "borderLeftWidth",
        (_, 0) => "borderTopColor",
        (_, 1) => "borderRightColor",
        (_, 2) => "borderBottomColor",
        _ => "borderLeftColor",
    }
}

/// Where a margin or padding that's a share of what it's in is kept, as `"25%"` or `"50% + -5px"`, which what lays it out works
/// out once it knows how wide that is.
fn share_key(kind: &str, side: usize) -> &'static str {
    match (kind, side) {
        ("margin", 0) => "marginTopShare",
        ("margin", 1) => "marginRightShare",
        ("margin", 2) => "marginBottomShare",
        ("margin", _) => "marginLeftShare",
        (_, 0) => "paddingTopShare",
        (_, 1) => "paddingRightShare",
        (_, 2) => "paddingBottomShare",
        _ => "paddingLeftShare",
    }
}

fn side_of(word: &str) -> Option<usize> {
    match word {
        "top" => Some(0),
        "right" => Some(1),
        "bottom" => Some(2),
        "left" => Some(3),
        _ => None,
    }
}

/// `[a, b = a, c = a, d = b]`, which takes what's left out, or `undefined`, from the side across from it.
fn four<T: Clone>(values: &[T], missing: impl Fn(&T) -> bool, none: T) -> [T; 4] {
    let given = |at: usize| values.get(at).filter(|one| !missing(one)).cloned();
    let a = given(0).unwrap_or(none);
    let b = given(1).unwrap_or_else(|| a.clone());
    let c = given(2).unwrap_or_else(|| a.clone());
    let d = given(3).unwrap_or_else(|| b.clone());
    [a, b, c, d]
}

/// JavaScript's `parseInt(text, 10)`, as the engine has it.
fn parse_int(text: &str) -> f64 {
    let text = text.trim_start_matches(is_space);
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let mut result = 0f64;
    let mut any = false;
    for c in digits.bytes() {
        if !c.is_ascii_digit() {
            break;
        }
        result = result * 10.0 + f64::from(c - b'0');
        any = true;
    }
    if !any {
        f64::NAN
    } else if negative {
        -result
    } else {
        result
    }
}

/// JavaScript's `Number(text)`, as the engine has it.
pub(crate) fn to_number(text: &str) -> f64 {
    let text = trim(text);
    if text.is_empty() {
        return 0.0;
    }
    let bytes = text.as_bytes();
    if bytes.len() > 2 && bytes[0] == b'0' {
        let radix = match bytes[1] | 0x20 {
            b'x' => 16,
            b'o' => 8,
            b'b' => 2,
            _ => 0,
        };
        if radix != 0 {
            let mut value = 0f64;
            for byte in &bytes[2..] {
                let Some(digit) = (*byte as char).to_digit(radix) else {
                    return f64::NAN;
                };
                value = value * f64::from(radix) + f64::from(digit);
            }
            return value;
        }
    }
    match text {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    if !bytes
        .iter()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        return f64::NAN;
    }
    text.parse().unwrap_or(f64::NAN)
}

/// `/^[0-9]*\.?[0-9]+$/`.
fn is_plain_number(text: &str) -> bool {
    number_end(text.as_bytes(), 0) == Some(text.len())
}

/// `/^(-?[0-9]*\.?[0-9]+)(em|%)$/`, as the share it says.
fn em_share(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let start = usize::from(bytes.first() == Some(&b'-'));
    let end = number_end(bytes, start)?;
    let divide = match &text[end..] {
        "em" => 1.0,
        "%" => 100.0,
        _ => return None,
    };
    Some(text[..end].parse::<f64>().unwrap_or(f64::NAN) / divide)
}

/// `/[0-9.](em|lh)\b/`: whether something's said in `em`, or in `lh`, which is a share of the text's size too.
fn says_em(value: &str) -> bool {
    says_unit(value, "em") || says_unit(value, "lh")
}

fn says_unit(value: &str, unit: &str) -> bool {
    let bytes = value.as_bytes();
    let mut from = 0;
    while let Some(found) = value[from..].find(unit) {
        let at = from + found;
        if at > 0 && (bytes[at - 1].is_ascii_digit() || bytes[at - 1] == b'.') {
            let after = bytes.get(at + 2);
            if !after.is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
                return true;
            }
        }
        from = at + 1;
    }
    false
}

/// `text.split(/\s+/)` of text with no space at either end.
fn spaced(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return vec![""];
    }
    text.split(is_space)
        .filter(|part| !part.is_empty())
        .collect()
}

/// What a declaration's worked out with, and what's come of it that goes back to JavaScript.
struct Declarer<'v> {
    vm: &'v mut Vm,
    rem: f64,
    /// Which props have ever been said in `em`, in the order they were, which is the page's, so there's nothing to look out for
    /// until something has.
    em: Vec<&'static str>,
    em_changed: bool,
    /// What can't be done here, in the order it was found, to be said once.
    said: Vec<String>,
    /// What `web.js` passed as `declaring`, or `undefined` when there's no page.
    declaring: Value,
    next_parts: u32,
}

impl<'v> Declarer<'v> {
    fn new(vm: &'v mut Vm, rem: f64, declaring: Value) -> Run<Self> {
        let mut em = Vec::new();
        if declaring.is_object() {
            let name = vm.atom("em");
            let seen = vm.get_property(declaring, name)?;
            if let Some(seen) = vm.string_text(seen) {
                em.extend(seen.split(' ').filter_map(prop_key));
            }
        }
        Ok(Self {
            vm,
            rem,
            em,
            em_changed: false,
            said: Vec::new(),
            declaring,
            next_parts: 0,
        })
    }

    /// Gives back to JavaScript what's changed about the page, and says what can't be done here.
    fn finish(self) -> Run<()> {
        if !self.declaring.is_object() {
            return Ok(());
        }
        if self.em_changed {
            let seen = self.vm.new_string(&self.em.join(" "));
            let name = self.vm.atom("em");
            self.vm.set_property(self.declaring, name, seen)?;
        }
        if !self.said.is_empty() {
            let name = self.vm.atom("warn");
            let warn = self.vm.get_property(self.declaring, name)?;
            for what in &self.said {
                let what = self.vm.new_string(what);
                self.vm.call_function(warn, Value::UNDEFINED, &[what])?;
            }
        }
        Ok(())
    }

    fn not_here(&mut self, what: String) {
        self.said.push(what);
    }

    fn length(&mut self, text: &str, font_size: f64) -> Size {
        length(
            text,
            &mut Sizing {
                font_size,
                rem: self.rem,
                vm: self.vm,
            },
        )
    }

    /// A `background-position` with each length in it in pixels, as `right .75rem center` is `right 12px center`, which is what
    /// drawing it takes: keywords and percentages are as they are.
    fn position_in_pixels(&mut self, said: &str, font_size: f64) -> String {
        let mut out = Vec::new();
        let mut any = false;
        // Split where there's space outside brackets, so a `calc(2.5rem * 1)` is one word, as a Progress's stripes have it.
        for word in words(said) {
            let in_units = !word.ends_with("px") && word.bytes().any(|b| b.is_ascii_alphabetic());
            match self.length(word, font_size) {
                Size::Pixels(pixels) if in_units => {
                    any = true;
                    out.push(format!("{}px", self.number(pixels)));
                }
                _ => out.push(word.to_owned()),
            }
        }
        if any { out.join(" ") } else { said.to_owned() }
    }

    fn number(&mut self, number: f64) -> String {
        number_text(self.vm, number)
    }

    /// A size as it goes into text, as `size + 'px'` makes it.
    fn size_text(&mut self, size: &Size) -> String {
        match size {
            Size::Pixels(number) => self.number(*number),
            Size::Text(text) => text.clone(),
            Size::None => "undefined".to_owned(),
        }
    }

    fn call(&mut self, function: &str, value: &str) -> Run<Value> {
        let name = self.vm.atom(function);
        let function = self.vm.get_property(self.declaring, name)?;
        let value = self.vm.new_string(value);
        self.vm.call_function(function, Value::UNDEFINED, &[value])
    }

    /// What's said, with what each `var()` in it stands for.
    fn with_variables<'t>(&mut self, given: &'t str) -> Run<Cow<'t, str>> {
        if !given.contains("var(") || !self.declaring.is_object() {
            return Ok(Cow::Borrowed(given));
        }
        let found = self.call("vars", given)?;
        Ok(Cow::Owned(text_of(self.vm, found).into_owned()))
    }

    fn new_parts(&mut self, before: Option<&Val>, name: &str, value: &str) -> Val {
        let mut parts = match before {
            Some(Val::Parts(_, parts)) => parts.clone(),
            _ => Vec::new(),
        };
        match parts.iter_mut().find(|(one, _)| one == name) {
            Some(part) => part.1 = value.to_owned(),
            None => parts.push((name.to_owned(), value.to_owned())),
        }
        self.next_parts += 1;
        Val::Parts(self.next_parts, parts)
    }

    /// Makes each declaration part of `props`, with what says `!important` after the rest, and which props that made kept in
    /// `__important`, as nothing that doesn't say it goes over it, as CSS has it.
    fn declare_all(
        &mut self,
        props: &mut Props,
        declarations: &[(String, String, bool)],
        font_size: f64,
    ) -> Run<()> {
        let mut any = false;
        for (name, value, important) in declarations {
            if *important {
                any = true;
            } else {
                self.declare(props, name, value, font_size)?;
            }
        }
        if !any {
            return Ok(());
        }
        let before = props.clone();
        for (name, value, important) in declarations {
            if *important {
                self.declare(props, name, value, font_size)?;
            }
        }
        // What it made, which is also what it set that wasn't there before, as `undefined`: `display: flex !important` takes
        // `display: grid` away, which an earlier `display: grid !important` mustn't keep.
        let kept: Vec<Key> = props
            .0
            .iter()
            .filter(|(key, value)| {
                *key != Key::Plain("__important")
                    && (before.at(*key).is_none() || !same(self.vm, Some(value), before.get_key(*key)))
            })
            .map(|(key, _)| *key)
            .collect();
        let mut all = match props.get("__important") {
            Some(Val::Keys(keys)) => keys.clone(),
            _ => Vec::new(),
        };
        all.extend(kept);
        props.set("__important", Val::Keys(all));
        Ok(())
    }

    /// Makes `name: value` part of `props`, keeping what's in `em` as a share of the element's own text size, which is only known
    /// when it's drawn: `__em_marginTop` and the like, and `__fontScale` for a text size that's a share of what's around it.
    fn declare(&mut self, props: &mut Props, name: &str, given: &str, font_size: f64) -> Run<()> {
        let value = self.with_variables(given)?;
        // A `var()` of a variable that isn't there, with nothing after a comma, makes what it's in `unset`, as Radix's card's
        // `font-weight: var(--font-weight-normal)` is: for what's passed down, that's what it's in says.
        let value = if value.trim().is_empty() && given.contains("var(") && inherited_key(name).is_some() {
            "unset"
        } else {
            &*value
        };
        if name == "font-size" {
            let trimmed = trim(value);
            props.set(
                "__fontScale",
                em_share(trimmed).map_or(Val::Null, Val::Number),
            );
            if let Some(named) = font_size_word(trimmed) {
                props.set("__fontScale", Val::Number(named));
            }
        }
        if name == "font-size" || name.starts_with("--") || !says_em(value) {
            // Nothing's been said in `em` yet, which is nearly always, so there's nothing to look out for.
            if self.em.is_empty() {
                return self.declare_one(props, name, value, font_size);
            }
            let before: Vec<Val> = self
                .em
                .iter()
                .map(|key| props.get(key).cloned().unwrap_or(Val::Undefined))
                .collect();
            self.declare_one(props, name, value, font_size)?;
            // What's said in pixels now isn't a share of anything any more, which goes over anything said before, wherever that was.
            let mut at = 0;
            while at < self.em.len() {
                let key = self.em[at];
                if !same(self.vm, props.get(key), before.get(at)) {
                    props.set_key(Key::Em(key), Val::Undefined);
                }
                at += 1;
            }
            return Ok(());
        }
        let before = props.clone();
        self.declare_one(props, name, value, font_size)?;
        let count = props.0.len();
        for at in 0..count {
            let (key, now) = (props.0[at].0, &props.0[at].1);
            let Key::Plain(key) = key else {
                continue;
            };
            if let Val::Number(now) = *now {
                if !key.starts_with("__")
                    && !same(self.vm, Some(&Val::Number(now)), before.get(key))
                    && font_size > 0.0
                {
                    props.set_key(Key::Em(key), Val::Number(now / font_size));
                    if !self.em.contains(&key) {
                        self.em.push(key);
                        self.em_changed = true;
                    }
                }
            }
        }
        Ok(())
    }

    fn set_size(
        &mut self,
        props: &mut Props,
        name: &str,
        value: &str,
        font_size: f64,
        key: &'static str,
    ) {
        let size = self.length(value, font_size);
        let fit = match key {
            "width" => Some("fitWidth"),
            "height" => Some("fitHeight"),
            _ => None,
        };
        if let Some(fit) = fit {
            props.delete(fit);
        }
        if size != Size::None {
            props.set(key, size_val(size));
        } else if fit.is_some() && matches!(value, "fit-content" | "max-content" | "min-content") {
            // As big as what's in it, and not stretched to what it's in.
            props.delete(key);
            props.set(fit.unwrap_or(key), Val::Bool(true));
        } else if from_inside(value) {
            // Its own size, over anything said before.
            props.set(key, Val::Undefined);
        } else {
            self.not_here(format!("{name} as {value}"));
        }
    }

    fn set_number(
        &mut self,
        props: &mut Props,
        name: &str,
        value: &str,
        font_size: f64,
        key: &'static str,
    ) {
        match self.length(value, font_size) {
            Size::Pixels(number) => props.set(key, Val::Number(number)),
            Size::Text(_) => self.not_here(format!("{name} as a percentage")),
            Size::None if from_inside(value) => props.set(key, Val::Undefined),
            Size::None => self.not_here(format!("{name} as {value}")),
        }
    }

    fn set_color(&mut self, props: &mut Props, name: &str, value: &str, key: &'static str) {
        if value.is_empty() {
            return;
        }
        if is_current_color(value) && key != "color" {
            // The color of its text, which is only known once it's all been worked out.
            props.set(key, text("currentcolor"));
            return;
        }
        if value == "inherit" || is_current_color(value) || value == "unset" || value == "initial" {
            // What it's in decides, which is what having none here does.
            props.set(key, Val::Undefined);
            return;
        }
        match color(value) {
            Some(paint) => props.set(key, Val::Text(paint)),
            None => self.not_here(format!("{name} as {value}")),
        }
    }

    fn set_sides(&mut self, props: &mut Props, name: &'static str, value: &str, font_size: f64) {
        let read: Vec<Size> = words(value)
            .into_iter()
            .map(|part| {
                if part == "auto" {
                    Size::Text("auto".to_owned())
                } else {
                    self.length(part, font_size)
                }
            })
            .collect();
        let sides = four(&read, |one| *one == Size::None, Size::None);
        let is_auto = |size: &Size| matches!(size, Size::Text(text) if text == "auto");
        if name == "margin" {
            // What's `auto` takes what's left over, which depends on what it's in, so it's only worked out when that's known.
            for (side, size) in sides.iter().enumerate() {
                props.set(side_key("__auto", side), Val::Bool(is_auto(size)));
                if is_auto(size) {
                    props.set(side_key("margin", side), Val::Undefined);
                }
            }
        }
        for (side, size) in sides.iter().enumerate() {
            if let Size::Pixels(number) = size {
                props.set(side_key(name, side), Val::Number(*number));
                if props.get(share_key(name, side)).is_some() {
                    props.set(share_key(name, side), Val::Undefined);
                }
            } else if let Size::Text(share) = size
                && share.contains('%')
            {
                props.set(side_key(name, side), Val::Number(0.0));
                props.set(share_key(name, side), owned(share.clone()));
            } else if !is_auto(size) {
                self.not_here(format!("{name} as {value}"));
            }
        }
    }

    /// What an `outline` says: a line around the outside of something, which takes no room. It's only there with a style that
    /// isn't `none`, as CSS has it, 3 pixels wide unless it says, in the color of the text unless it says.
    fn set_outline(&mut self, props: &mut Props, value: &str) {
        let mut style = "none";
        let mut width = 3.0;
        let mut paint = None;
        for part in words(value) {
            let size = self.length(part, 16.0);
            let found = color(part);
            if let Size::Pixels(number) = size {
                width = number;
            } else if part == "none" || part == "hidden" || border_style(part) || part == "auto" {
                style = part;
            } else if found.is_some() {
                paint = found;
            }
        }
        props.set(
            "outlineWidth",
            Val::Number(if style == "none" || style == "hidden" {
                0.0
            } else {
                width
            }),
        );
        // What CSS's `outline-width` is, whatever the style, which is what moves when it changes (see `cssOutlineWidth` in web.js).
        props.set("cssOutlineWidth", Val::Number(width));
        // A shorthand sets each of its longhands, to its initial value where it doesn't say: the color of the text, for the color.
        match paint {
            Some(paint) => props.set("outlineColor", Val::Text(paint)),
            None => props.set("outlineColor", text("currentcolor")),
        }
        set_outline_style(props, style);
    }

    fn set_border(&mut self, props: &mut Props, value: &str) {
        if value == "none" || value == "0" {
            without_border_sides(props);
            props.set("borderWidth", Val::Number(0.0));
            return;
        }
        if value.is_empty() {
            return;
        }
        let mut found = false;
        for part in words(value) {
            let size = self.length(part, 16.0);
            let paint = color(part);
            if let Size::Pixels(number) = size {
                without_border_sides(props);
                props.set("borderWidth", Val::Number(number));
                found = true;
            } else if let Some(paint) = paint {
                props.set("borderColor", Val::Text(paint));
                found = true;
            } else if part == "none" || part == "hidden" {
                without_border_sides(props);
                props.set("borderWidth", Val::Number(0.0));
                found = true;
            } else if border_style(part) {
                props.set("borderStyle", text(drawn_border(part).unwrap_or("solid")));
                found = true;
            }
        }
        if found && props.is_undefined("borderWidth") {
            // It's 3 as CSS has it if it's not said, which is more than anyone meant.
            props.set("borderWidth", Val::Number(1.0));
        }
    }

    /// The picture a `background` or `background-image` shows, from its first `url()`, and for the `background` shorthand, what it
    /// says about its color and where the picture goes. Says whether there was one.
    fn set_background_url(&mut self, props: &mut Props, value: &str, shorthand: bool) -> bool {
        let Some(found) = find_url(value) else {
            return false;
        };
        // Without quotes, a URL can't have spaces, quotes or brackets in it, and CSS leaves out all of what says one does.
        if found.bare
            && found
                .url
                .chars()
                .any(|c| is_space(c) || matches!(c, '"' | '\'' | '('))
        {
            return true;
        }
        props.set("backgroundImage", owned(found.url));
        if !shorthand {
            return true;
        }
        // What's left, without the picture, which can have anything in it.
        let rest = format!("{} {}", &value[..found.start], &value[found.end..]);
        let layers = commas(&rest);
        let first = layers.first().copied().unwrap_or("");
        let mut halves = first.split('/');
        let placed = halves.next().unwrap_or("");
        let sized = halves.next();
        let mut position = Vec::new();
        let mut repeats = Vec::new();
        for word in words(placed) {
            if matches!(
                word,
                "repeat" | "no-repeat" | "repeat-x" | "repeat-y" | "space" | "round"
            ) {
                repeats.push(word);
            } else if matches!(word, "left" | "right" | "top" | "bottom" | "center")
                || matches!(self.length(word, 16.0), Size::Pixels(_))
                || word.ends_with('%')
            {
                position.push(word);
            } else if let Some(paint) = color(word) {
                props.set("background", Val::Text(paint));
            }
        }
        if !repeats.is_empty() {
            props.set("backgroundRepeat", owned(repeats.join(" ")));
        }
        if !position.is_empty() {
            let placed = self.position_in_pixels(&position.join(" "), 16.0);
            props.set("backgroundPosition", owned(placed));
        }
        if let Some(sized) = sized {
            if !trim(sized).is_empty() {
                let kept: Vec<&str> = spaced(trim(sized))
                    .into_iter()
                    .filter(|word| color(word).is_none())
                    .collect();
                props.set("backgroundSize", owned(kept.join(" ")));
                if let Some(paint) = words(sized).into_iter().find_map(color) {
                    props.set("background", Val::Text(paint));
                }
            }
        }
        // The color goes with the last of them.
        let last = if layers.len() > 1 {
            layers[layers.len() - 1]
        } else {
            ""
        };
        if let Some(paint) = words(last).into_iter().find_map(color) {
            props.set("background", Val::Text(paint));
        }
        true
    }

    fn set_shadow(&mut self, props: &mut Props, value: &str) {
        props.delete("shadows");
        if value == "none" {
            props.set("shadowBlur", Val::Number(0.0));
            // No shadows, which is what a list moves from and to as `box-shadow` changes (see `MOVING` in web.js).
            props.set("shadows", owned(String::new()));
            return;
        }
        // Each shadow, in the order CSS lists them, which is from the one on top.
        struct Shadow {
            sizes: [f64; 4],
            paint: Option<Cow<'static, str>>,
            inset: bool,
        }
        let all: Vec<Shadow> = commas(value)
            .into_iter()
            .map(|one| {
                let mut sizes = Vec::new();
                let mut paint = None;
                let mut inset = false;
                for part in words(one) {
                    let size = self.length(part, 16.0);
                    let found = color(part);
                    if part == "inset" {
                        inset = true;
                    } else if let Size::Pixels(number) = size {
                        sizes.push(number);
                    } else if found.is_some() {
                        paint = found;
                    }
                }
                let at = |index: usize| sizes.get(index).copied().unwrap_or(0.0);
                Shadow {
                    sizes: [at(0), at(1), at(2), at(3)],
                    paint,
                    inset,
                }
            })
            .collect();
        let Some(first) = all.first() else {
            return;
        };
        props.set("shadowX", Val::Number(first.sizes[0]));
        props.set("shadowY", Val::Number(first.sizes[1]));
        props.set("shadowBlur", Val::Number(first.sizes[2]));
        props.set("shadowSpread", Val::Number(first.sizes[3]));
        if let Some(paint) = &first.paint {
            props.set("shadowColor", Val::Text(paint.clone()));
        }
        {
            // All of them as a list, which is all that says more than one, or one inside, and what moves as a whole when
            // `box-shadow` changes, as CSS has it.
            let lines: Vec<String> = all
                .iter()
                .map(|one| {
                    let [x, y, blur, spread] = one.sizes.map(|size| self.number(size));
                    let paint = color_number(one.paint.as_deref().unwrap_or("rgba(0,0,0,0.25)"))
                        .unwrap_or(1_073_741_824);
                    format!(
                        "{x} {y} {blur} {spread} {paint}{}",
                        if one.inset { " inset" } else { "" }
                    )
                })
                .collect();
            props.set("shadows", owned(lines.join("\n")));
        }
    }

    fn set_flex(&mut self, props: &mut Props, value: &str, font_size: f64) {
        let parts = words(value);
        // Each says all of how it grows, shrinks and starts, over what any rule before said: `none` is `0 0 auto`, `auto` is
        // `1 1 auto`, and `initial` is `0 1 auto`, with shrinking as it is when nothing says.
        match parts.first().copied() {
            Some("none") => {
                props.set("grow", Val::Number(0.0));
                props.set("shrink", Val::Number(0.0));
                props.set("basis", Val::Undefined);
                return;
            }
            Some("auto") => {
                props.set("grow", Val::Number(1.0));
                props.set("shrink", Val::Undefined);
                props.set("basis", Val::Undefined);
                return;
            }
            Some("initial") => {
                props.set("grow", Val::Undefined);
                props.set("shrink", Val::Undefined);
                props.set("basis", Val::Undefined);
                return;
            }
            _ => {}
        }
        // Up to two numbers, for how much it grows and shrinks, and what it starts from, in any order after them.
        let mut numbers = Vec::new();
        let mut basis = None;
        for part in parts {
            if numbers.len() < 2 && basis.is_none() && is_plain_number(part) {
                numbers.push(parse_float(part));
            } else {
                basis = Some(part);
            }
        }
        let grow = numbers.first().copied().unwrap_or(1.0);
        let shrink = numbers.get(1).copied().unwrap_or(1.0);
        match basis {
            None => {
                // With only numbers, it starts from nothing, so what each gets is only its share.
                if grow > 0.0 {
                    props.set("grow", Val::Number(grow));
                    props.set("shrink", Val::Number(1.0));
                    props.set("basis", text("0%"));
                } else {
                    props.set("grow", Val::Number(0.0));
                }
            }
            Some(basis) => {
                // It starts from how big it is, or what it says, and what's left over is shared out.
                props.set("grow", Val::Number(grow));
                self.set_basis(props, basis, font_size);
            }
        }
        if shrink == 0.0 {
            props.set("shrink", Val::Number(0.0));
        }
    }

    /// What a flex item starts from before what's left is shared out: how big it is, for `auto` and `content`, or a length.
    fn set_basis(&mut self, props: &mut Props, basis: &str, font_size: f64) {
        let word = trim(basis).to_lowercase();
        if matches!(
            word.as_str(),
            "auto" | "content" | "max-content" | "fit-content"
        ) {
            // As big as it is, over what any rule before said, as lex: none after lex-basis: 0 is.
            props.set("basis", Val::Undefined);
            return;
        }
        let size = self.length(&word, font_size);
        if let Size::Pixels(number) = size {
            props.set("basis", Val::Number(number));
        } else if let Size::Text(share) = &size
            && share.contains("% +")
        {
            // A share and some pixels more, as `calc(25% - 4px)` is.
            props.set("basis", owned(share.clone()));
        } else if let Size::Text(share) = &size
            && trim(share).ends_with('%')
            && parse_float(trim(share)).is_finite()
            && parse_float(trim(share)) >= 0.0
        {
            // A share worked out, as Tailwind 4's `basis-1/2` is `calc(1/2 * 100%)`.
            let share = trim(share);
            if parse_float(share) == 0.0 {
                props.set("basis", Val::Number(0.0));
            } else {
                props.set("basis", owned(share));
            }
        } else if word.ends_with('%') && parse_float(&word) == 0.0 {
            props.set("basis", Val::Number(0.0));
        } else if word.ends_with('%') && parse_float(&word).is_finite() && parse_float(&word) > 0.0 {
            // A share of the flex box, which the layout works out once it knows how long that is: see `basis_share` in rsui-layout.
            props.set("basis", owned(word));
        } else {
            self.not_here(format!("flex-basis as {basis}"));
        }
    }

    /// `linear-gradient(...)` as what draws it takes it: `"angle|#rrggbbaa 0%|#rrggbbaa 20px|..."`, or nothing if it isn't one, or
    /// can't be.
    fn linear_gradient(&mut self, value: &str) -> Run<Option<String>> {
        let Some((repeating, radial, inside)) = gradient_parts(value) else {
            return Ok(None);
        };
        let mut parts: Vec<&str> = commas(inside).into_iter().map(trim).collect();
        // One with nothing in it, as one whose colors are variables nothing's set comes to, is none, as Radix Themes'
        // `linear-gradient(var(--focus-a2), var(--focus-a2))` is outside what sets them.
        if parts.is_empty() {
            return Ok(None);
        }
        let kind = if radial {
            self.radial_kind(&mut parts)
        } else {
            self.linear_kind(&mut parts)?
        };
        let mut stops = Vec::new();
        for part in parts {
            let pieces = split_outside(part, |c| c == ' ');
            let Some(paint) = pieces.first().and_then(|first| color(first)) else {
                return Ok(None);
            };
            // Always all eight digits, which is what's read.
            let mut digits: String = paint[1..].to_owned();
            if digits.len() == 3 || digits.len() == 4 {
                digits = digits.chars().flat_map(|digit| [digit, digit]).collect();
            }
            let hex = if digits.len() == 6 {
                format!("#{digits}ff")
            } else {
                format!("#{digits}")
            };
            // A stop with two places is two, of the same color.
            if pieces.len() > 1 {
                for place in &pieces[1..pieces.len().min(3)] {
                    let size = self.length(place, 16.0);
                    let place = match size {
                        Size::Pixels(number) => self.number(number) + "px",
                        _ => (*place).to_owned(),
                    };
                    stops.push(format!("{hex} {place}"));
                }
            } else {
                stops.push(hex);
            }
        }
        if stops.is_empty() {
            return Ok(None);
        }
        Ok(Some(format!(
            "{}{kind}|{}",
            if repeating { "*" } else { "" },
            stops.join("|")
        )))
    }

    /// Where a radial gradient's middle is and how big it is, as `radial ellipse farthest-corner 50% 50%`, from what it says
    /// before its colors, which is taken out of `parts` if it's there.
    fn radial_kind(&mut self, parts: &mut Vec<&str>) -> String {
        let mut shape = None;
        let mut size = Vec::new();
        let mut middle = ["50%".to_owned(), "50%".to_owned()];
        let said = words(parts.first().copied().unwrap_or(""));
        if !said.is_empty() && color(said[0]).is_none() {
            parts.remove(0);
            let at = said.iter().position(|word| *word == "at");
            for word in &said[..at.unwrap_or(said.len())] {
                if *word == "circle" || *word == "ellipse" {
                    shape = Some(*word);
                } else if is_extent(word) || word.ends_with('%') {
                    size.push((*word).to_owned());
                } else {
                    let found = self.length(word, 16.0);
                    size.push(self.size_or_zero(found) + "px");
                }
            }
            if let Some(at) = at {
                middle = self.place_of(&said[at + 1..]);
            }
        }
        let shape = shape.unwrap_or(if size.len() == 1 && !is_extent(&size[0]) {
            "circle"
        } else {
            "ellipse"
        });
        let mut out = vec!["radial".to_owned(), shape.to_owned()];
        if size.is_empty() {
            out.push("farthest-corner".to_owned());
        } else {
            out.extend(size);
        }
        out.extend(middle);
        out.join(" ")
    }

    /// `length(word) ?? 0`, as text.
    fn size_or_zero(&mut self, size: Size) -> String {
        match size {
            Size::None => "0".to_owned(),
            other => self.size_text(&other),
        }
    }

    /// Where something is across and down, as `left top` or `30% 10px` says it, as a percentage or pixels each way.
    fn place_of(&mut self, said: &[&str]) -> [String; 2] {
        let mut one = |word: &str| -> String {
            match word {
                "left" | "top" => "0%".to_owned(),
                "center" => "50%".to_owned(),
                "right" | "bottom" => "100%".to_owned(),
                _ if word.ends_with('%') => word.to_owned(),
                _ => {
                    let found = self.length(word, 16.0);
                    self.size_or_zero(found) + "px"
                }
            }
        };
        let upright = |word: &str| word == "top" || word == "bottom";
        match said {
            [] => ["50%".to_owned(), "50%".to_owned()],
            [only] => {
                if upright(only) {
                    ["50%".to_owned(), one(only)]
                } else {
                    [one(only), "50%".to_owned()]
                }
            }
            [first, second, ..] => {
                if upright(first) || *second == "left" || *second == "right" {
                    [one(second), one(first)]
                } else {
                    [one(first), one(second)]
                }
            }
        }
    }

    /// The angle a linear gradient goes at, in degrees, from what it says before its colors, which is taken out of `parts` if it's
    /// there.
    fn linear_kind(&mut self, parts: &mut Vec<&str>) -> Run<String> {
        let mut angle = 180.0;
        let Some(first) = parts.first() else {
            // `parts[0].toLowerCase()`, of nothing.
            return Err(
                self.type_error("Cannot read properties of undefined (reading 'toLowerCase')")
            );
        };
        let first = first.to_lowercase();
        if let Some(rest) = first.strip_prefix("to ") {
            let place = spaced(trim(rest));
            let side =
                |word: &str| side_of(word).map_or(f64::NAN, |side| [0.0, 90.0, 180.0, 270.0][side]);
            if place.len() == 1 {
                angle = side_of(place[0]).map_or(180.0, |side| [0.0, 90.0, 180.0, 270.0][side]);
            } else {
                // A corner, which is the angle between the sides it names.
                let (a, b) = (side(place[0]), side(place[1]));
                angle = if (a == 0.0 && b == 270.0) || (b == 0.0 && a == 270.0) {
                    315.0
                } else {
                    (a + b) / 2.0
                };
            }
            parts.remove(0);
        } else if is_angle(&first) || first == "0" {
            let number = parse_float(&first);
            angle = if first.ends_with("turn") {
                number * 360.0
            } else if first.ends_with("grad") {
                number * 0.9
            } else if first.ends_with("rad") {
                (number * 180.0) / std::f64::consts::PI
            } else {
                number
            };
            parts.remove(0);
        }
        Ok(self.number(angle))
    }

    fn type_error(&mut self, message: &str) -> Value {
        let maker = self.vm.get_global("TypeError").unwrap_or(Value::UNDEFINED);
        let message = self.vm.new_string(message);
        match self.vm.call_function(maker, Value::UNDEFINED, &[message]) {
            Ok(error) | Err(error) => error,
        }
    }

    /// What a `background` or `background-image` with gradients or a picture in it says: each gradient, from the one on top, in
    /// `backgroundGradient`, a line each, the first picture, and for the `background` shorthand, the color at the end. Says
    /// whether there was a gradient or a picture.
    fn set_background_layers(
        &mut self,
        props: &mut Props,
        value: &str,
        shorthand: bool,
    ) -> Run<bool> {
        let layers = commas(value);
        let mut gradients = Vec::new();
        let mut found = false;
        let mut pictured = false;
        for (at, layer) in layers.iter().enumerate() {
            let mut rest = trim(layer);
            if let Some((_, _, after)) = gradient_start(rest) {
                let start = rest.len() - after.len();
                // Up to the bracket that closes it.
                let bytes = rest.as_bytes();
                let mut depth = 0i32;
                let mut end = rest.len() - 1;
                for (index, c) in bytes.iter().enumerate().skip(start - 1) {
                    depth += match c {
                        b'(' => 1,
                        b')' => -1,
                        _ => 0,
                    };
                    if depth == 0 {
                        end = index;
                        break;
                    }
                }
                let whole = &rest[..end + 1];
                match self.linear_gradient(whole)? {
                    Some(spec) => gradients.push(spec),
                    None => self.not_here(format!("background as {whole}")),
                }
                found = true;
                rest = &rest[end + 1..];
            } else if !pictured && contains_url(rest) {
                pictured = true;
                found = self.set_background_url(props, rest, shorthand) || found;
                rest = "";
            }
            if shorthand && at == layers.len() - 1 {
                if let Some(paint) = words(rest).into_iter().find_map(color) {
                    props.set("background", Val::Text(paint));
                }
            }
        }
        if !gradients.is_empty() {
            props.set("backgroundGradient", owned(gradients.join("\n")));
        }
        Ok(found)
    }

    /// Grid tracks as CSS writes them, with each size that's in something other than pixels in pixels, which is what a grid here
    /// takes.
    fn grid_tracks(&mut self, value: &str, font_size: f64) -> String {
        const UNITS: [&str; 7] = ["rem", "em", "vw", "vh", "vmin", "vmax", "pt"];
        let bytes = value.as_bytes();
        let mut out = String::with_capacity(value.len());
        let mut from = 0;
        let mut at = 0;
        while at < bytes.len() {
            let start = at + usize::from(bytes[at] == b'-');
            let found = number_end(bytes, start).and_then(|end| {
                UNITS.iter().find_map(|unit| {
                    let after = end + unit.len();
                    let word_after = bytes
                        .get(after)
                        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_');
                    (value[end..].starts_with(unit) && !word_after).then_some(after)
                })
            });
            match found {
                Some(end) => {
                    let whole = &value[at..end];
                    out.push_str(&value[from..at]);
                    match self.length(whole, font_size) {
                        Size::Pixels(number) => {
                            out.push_str(&self.number(number));
                            out.push_str("px");
                        }
                        _ => out.push_str(whole),
                    }
                    from = end;
                    at = end;
                }
                None => at += value[at..].chars().next().map_or(1, char::len_utf8),
            }
        }
        out.push_str(&value[from..]);
        out
    }

    /// How big a corner is one way, as pixels, or a percentage of the box as text, which is only known when it's drawn.
    fn corner_size(&mut self, value: Option<&str>, font_size: f64) -> Corner {
        let Some(value) = value else {
            return Corner::Pixels(0.0);
        };
        if value.ends_with('%') {
            let share = parse_float(value);
            return if share.is_finite() {
                Corner::Share(self.number(share) + "%")
            } else {
                Corner::Pixels(0.0)
            };
        }
        match self.length(value, font_size) {
            Size::Pixels(size) => Corner::Pixels(js_max(0.0, size)),
            _ => Corner::Pixels(0.0),
        }
    }

    /// The corners something has said, as `[across, down]` for each, from the top left going around.
    fn corners_of(&mut self, props: &Props) -> [[Corner; 2]; 4] {
        if let Some(Val::Text(radii)) = props.get("cornerRadii") {
            let parts: Vec<Corner> = radii
                .split(' ')
                .map(|part| {
                    if part.ends_with('%') {
                        Corner::Share(part.to_owned())
                    } else {
                        Corner::Pixels(to_number(part))
                    }
                })
                .collect();
            let at = |index: usize| parts.get(index).cloned().unwrap_or(Corner::Missing);
            return [0, 1, 2, 3].map(|corner| [at(corner * 2), at(corner * 2 + 1)]);
        }
        let radius = match props.get("radius") {
            Some(Val::Number(radius)) => *radius,
            _ => 0.0,
        };
        [0, 1, 2, 3].map(|_| [Corner::Pixels(radius), Corner::Pixels(radius)])
    }

    /// Says what corners something has: one `radius` if they're all the same circle in pixels, which is nearly always and is
    /// drawn most simply, or each of them in `cornerRadii` otherwise.
    fn set_corners(&mut self, props: &mut Props, corners: [[Corner; 2]; 4]) {
        let flat: Vec<Corner> = corners.into_iter().flatten().collect();
        if let Corner::Pixels(first) = flat[0] {
            if flat
                .iter()
                .all(|one| matches!(one, Corner::Pixels(size) if *size == first))
            {
                props.set("radius", Val::Number(first));
                props.delete("cornerRadii");
                return;
            }
        }
        let written: Vec<String> = flat
            .iter()
            .map(|one| match one {
                Corner::Pixels(size) => self.number(*size),
                Corner::Share(share) => share.clone(),
                Corner::Missing => String::new(),
            })
            .collect();
        props.set("cornerRadii", owned(written.join(" ")));
        // What only takes one radius, as a picture does, takes the biggest that's in pixels, or is as round as it can be if every
        // corner is half of it or more.
        let all_half = flat
            .iter()
            .all(|one| matches!(one, Corner::Share(share) if parse_float(share) >= 50.0));
        let radius = if all_half {
            9999.0
        } else {
            flat.iter()
                .filter_map(|one| match one {
                    Corner::Pixels(size) => Some(*size),
                    _ => None,
                })
                .fold(0.0, js_max)
        };
        props.set("radius", Val::Number(radius));
    }

    /// [`declare_one_said`](Self::declare_one_said), with a border's or an outline's width in whole pixels, as a browser draws
    /// them: less than one is one, and more is what it is without what's past the last whole pixel, as Pico's `0.0625rem` at
    /// 18px is 1, rather than 1.125.
    fn declare_one(
        &mut self,
        props: &mut Props,
        name: &str,
        value: &str,
        font_size: f64,
    ) -> Run<()> {
        self.declare_one_said(props, name, value, font_size)?;
        if (name.starts_with("border") || name.starts_with("outline")) && !name.contains("radius") {
            for key in LINE_WIDTH_KEYS {
                if let Some(Val::Number(width)) = props.get(key) {
                    let whole = whole_line_width(*width);
                    if whole != *width {
                        props.set(key, Val::Number(whole));
                    }
                }
            }
        }
        Ok(())
    }

    fn declare_one_said(
        &mut self,
        props: &mut Props,
        name: &str,
        value: &str,
        font_size: f64,
    ) -> Run<()> {
        // A border's or an outline's `thin`, `medium` and `thick` are 1, 3 and 5 pixels, as a browser has them, as MUI's
        // Divider's `border-bottom-width: thin` is.
        let widths_said;
        let value = if (name.starts_with("border") || name.starts_with("outline"))
            && words(value).iter().any(|word| line_width_word(word).is_some())
        {
            widths_said = words(value)
                .iter()
                .map(|word| line_width_word(word).unwrap_or(word))
                .collect::<Vec<_>>()
                .join(" ");
            widths_said.as_str()
        } else {
            value
        };
        if name.starts_with("border")
            && !name.contains("radius")
            && name != "border-collapse"
            && name != "border-spacing"
        {
            // A border that's said is what it is, rather than one drawn as the system draws a control.
            props.set("__ownBorder", Val::Bool(true));
        }
        if let Some((property, marker)) = inherit_marker(name) {
            if value == "inherit" {
                // The parent's own value, which `web.js` puts in when the element is drawn, as only it knows its parent.
                props.set(marker, text(property));
                return Ok(());
            }
            // What's said goes over an `inherit` before it, in the same block or in a rule it goes over, as a class's
            // `box-sizing: content-box` does over `* { box-sizing: inherit }`.
            props.set(marker, Val::Undefined);
        }
        if let Some(marker) = shorthand_marker(name)
            && value != "inherit"
        {
            // A part said after its shorthand's `inherit` goes over that too, as Tailwind's `.underline` (`text-decoration-line`)
            // does over its preflight's `a { text-decoration: inherit }`: the link is underlined.
            props.set(marker, Val::Undefined);
        }
        if let Some(passed_down) = inherited_key(name) {
            if value == "inherit"
                || value == "unset"
                || (name == "color" && is_current_color(value))
            {
                props.set(passed_down, Val::Undefined);
                return Ok(());
            }
        }
        match name {
            "display" if value != "none" && table_part(value).is_some() => {
                // A part of a table is a block, with what's in it laid out by the table it's in: see `table.rs` in rsui-layout.
                props.set("tablePart", text(table_part(value).unwrap_or_default()));
                props.set("__block", Val::Bool(true));
                props.set("__inline", Val::Bool(false));
                props.set("__hidden", Val::Bool(false));
            }
            "display" => {
                // Every display but `none` says it's shown, rather than nothing about it, so that it goes over a `none` from
                // another rule, as one value of `display` does over another.
                // What's said of something that's a part of a table by itself, as a `<td>` is, goes over that.
                if value != "none" {
                    props.set("tablePart", Val::Undefined);
                }
                // One display takes the place of another, so each says what the others would have said, as `undefined` where
                // it says nothing: a `display: flex` after a `display: grid` makes a flex box, as Bootstrap's `d-grid d-md-flex` is.
                match value {
                    "none" => props.set("__hidden", Val::Bool(true)),
                    "flex" | "inline-flex" => {
                        props.set("__flex", Val::Bool(true));
                        props.set("__inline", Val::Bool(value == "inline-flex"));
                        props.set("display", Val::Undefined);
                        props.set("__lineBox", Val::Undefined);
                        props.set("__listItem", Val::Bool(false));
                        if props.is_undefined("direction") {
                            props.set("direction", text("row"));
                            // Only as what `display: flex` means by itself: a `flex-direction` from another rule stays, as
                            // `hidden flex-col md:flex` is a column at md (see `mergeLook` in web.js).
                            props.set("__directionByDisplay", Val::Bool(true));
                        }
                        props.set("__hidden", Val::Bool(false));
                    }
                    "inline-block" | "inline" => {
                        props.set("__inline", Val::Bool(true));
                        // Only what's `inline-block` is a box of its own on its line, as tall as it is.
                        props.set("__lineBox", Val::Bool(value == "inline-block"));
                        props.delete("__block");
                        props.set("__flex", Val::Undefined);
                        props.set("display", Val::Undefined);
                        props.set("__listItem", Val::Bool(false));
                        props.set("__hidden", Val::Bool(false));
                    }
                    "grid" | "inline-grid" => {
                        props.set("display", text("grid"));
                        props.set("__flex", Val::Undefined);
                        props.set("__listItem", Val::Bool(false));
                        props.set("__hidden", Val::Bool(false));
                    }
                    // No box of its own: what's in it is part of what it's in.
                    "contents" => {
                        props.set("display", text("contents"));
                        props.set("__flex", Val::Undefined);
                        props.set("__listItem", Val::Bool(false));
                        props.set("__hidden", Val::Bool(false));
                    }
                    "block" | "flow-root" | "list-item" | "table" => {
                        // Not on a line with what's around it, which matters for a picture.
                        props.set("__block", Val::Bool(true));
                        props.set("__inline", Val::Bool(false));
                        props.set("__flex", Val::Undefined);
                        props.set("display", Val::Undefined);
                        props.set("__lineBox", Val::Undefined);
                        // Only a list item has a marker, as `display: block` on an `<li>` takes it away.
                        props.set("__listItem", Val::Bool(value == "list-item"));
                        props.set("__hidden", Val::Bool(false));
                        if value == "flow-root" {
                            // What's first and last in it keeps its margins in it.
                            props.set("__ownMargins", Val::Bool(true));
                        }
                    }
                    _ => props.set("__hidden", Val::Bool(false)),
                }
            }
            "flex-direction" => {
                props.set("__directionByDisplay", Val::Undefined);
                props.set(
                    "direction",
                    text(if value.starts_with("column") {
                        "column"
                    } else {
                        "row"
                    }),
                );
                // From the end to the start, which is the other way along the same line. Saying it isn't is said too, so that
                // `sm:flex-row` after `flex-col-reverse` (shadcn/ui's dialog footer) turns it back, as each rule's props are
                // put together after they're worked out.
                props.set("reverse", Val::Bool(value.ends_with("-reverse")));
            }
            "order" => {
                let order = parse_int(value);
                if !order.is_nan() {
                    props.set("__order", Val::Number(order));
                }
            }
            // What makes something a container that `@container` asks about, by how big it's laid out (see `containerMatches` in
            // web.js): what it's asked about (`inline-size`, `size` or `normal`) and the names it goes by.
            "container-type" => props.set("__containerType", owned(value.trim().to_owned())),
            "container-name" => props.set("__containerName", owned(value.trim().to_owned())),
            "container" => {
                let (names, kind) = match value.split_once('/') {
                    Some((names, kind)) => (names.trim(), kind.trim()),
                    None => (value.trim(), "normal"),
                };
                props.set("__containerName", owned(names.to_owned()));
                props.set("__containerType", owned(kind.to_owned()));
            }
            "align-items" => props.set("align", text(align(value).unwrap_or("stretch"))),
            "justify-self" => props.set("justifySelf", text(align(value).unwrap_or("stretch"))),
            "align-self" => props.set("alignSelf", text(align(value).unwrap_or("stretch"))),
            // `normal` and `stretch` are as if nothing were said, which is the start of a flex box and filling a grid, as a
            // grid's `auto` columns do then.
            "justify-content" if matches!(value, "normal" | "stretch") => props.set("justify", Val::Undefined),
            "justify-content" => props.set("justify", text(justify(value).unwrap_or("start"))),
            "justify-items" => props.set("justifyItems", text(align(value).unwrap_or("stretch"))),
            "place-items" => {
                let said = words(value);
                let across = said.first().copied();
                let along = said.get(1).copied().or(across);
                props.set("align", text(across.and_then(align).unwrap_or("stretch")));
                props.set(
                    "justifyItems",
                    text(along.and_then(align).unwrap_or("stretch")),
                );
            }
            "flex" => self.set_flex(props, value, font_size),
            "flex-grow" => props.set("grow", Val::Number(parse_float(value))),
            "flex-shrink" => props.set("shrink", Val::Number(parse_float(value))),
            "flex-basis" => self.set_basis(props, value, font_size),
            "width" => self.set_size(props, name, value, font_size, "width"),
            "height" => self.set_size(props, name, value, font_size, "height"),
            "min-width" => {
                // A share of what it's in, as `max-width`'s can be, which is only known once that's laid out.
                match self.length(value, font_size) {
                    Size::Text(size) if size.contains('%') => props.set("minWidth", owned(size)),
                    _ => self.set_number(props, name, value, font_size, "minWidth"),
                }
            }
            "max-width" => {
                // No wider than what's in it (Tailwind's `max-w-max`), which is only known as it's laid out.
                props.delete("maxWidthFit");
                if matches!(value, "max-content" | "fit-content") {
                    props.delete("maxWidth");
                    props.set("maxWidthFit", Val::Bool(true));
                    return Ok(());
                }
                // A share of what it's in, which is only known once that's laid out.
                match self.length(value, font_size) {
                    Size::Text(size) if size.contains('%') => props.set("maxWidth", owned(size)),
                    _ => self.set_number(props, name, value, font_size, "maxWidth"),
                }
            }
            "min-height" => self.set_number(props, name, value, font_size, "minHeight"),
            "max-height" => self.set_number(props, name, value, font_size, "maxHeight"),
            "margin" => self.set_sides(props, "margin", value, font_size),
            "padding" => self.set_sides(props, "padding", value, font_size),
            "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "padding-top"
            | "padding-right" | "padding-bottom" | "padding-left" => {
                let (kind, side) = name.split_once('-').unwrap_or((name, ""));
                let side = side_of(side).unwrap_or(0);
                let kind = if kind == "margin" {
                    "margin"
                } else {
                    "padding"
                };
                match self.length(value, font_size) {
                    Size::Text(size) if size.contains('%') => {
                        props.set(side_key(kind, side), Val::Number(0.0));
                        props.set(share_key(kind, side), owned(size));
                    }
                    _ => {
                        self.set_number(props, name, value, font_size, side_key(kind, side));
                        // Pixels now, over a share said before.
                        if props.get(share_key(kind, side)).is_some() {
                            props.set(share_key(kind, side), Val::Undefined);
                        }
                    }
                }
                if kind == "margin" {
                    props.set(side_key("__auto", side), Val::Bool(value == "auto"));
                }
            }
            "gap" | "grid-gap" => {
                let said = words(value);
                let rows = said.first().copied().unwrap_or("");
                let columns = said.get(1).copied().unwrap_or(rows);
                let row = self.length(rows, font_size);
                let column = self.length(columns, font_size);
                if let Size::Pixels(row) = row {
                    props.set("gap", Val::Number(row));
                    props.set("rowGap", Val::Number(row));
                }
                if let Size::Pixels(column) = column {
                    props.set("columnGap", Val::Number(column));
                }
            }
            "row-gap" | "grid-row-gap" => self.set_number(props, name, value, font_size, "rowGap"),
            "column-gap" | "grid-column-gap" => self.set_number(props, name, value, font_size, "columnGap"),
            "position" => {
                props.delete("__sticky");
                props.set("__fixed", Val::Bool(value == "fixed"));
                match value {
                    "absolute" | "fixed" => props.set("position", text("absolute")),
                    "relative" => props.set("position", text("relative")),
                    // Where it would be, and drawn with what isn't placed, which is what nothing saying anything comes to.
                    "static" => props.delete("position"),
                    "sticky" | "-webkit-sticky" => {
                        props.set("position", text("relative"));
                        props.set("__sticky", Val::Bool(true));
                    }
                    _ => {}
                }
            }
            "top" => self.set_size(props, name, value, font_size, "top"),
            "left" => self.set_size(props, name, value, font_size, "left"),
            "right" => self.set_size(props, name, value, font_size, "right"),
            "bottom" => self.set_size(props, name, value, font_size, "bottom"),
            "inset" => {
                let read: Vec<Size> = words(value)
                    .into_iter()
                    .map(|part| self.length(part, font_size))
                    .collect();
                let sides = four(&read, |one| *one == Size::None, Size::None);
                for (side, size) in ["top", "right", "bottom", "left"].into_iter().zip(sides) {
                    props.set(side, size_val(size));
                }
            }
            "overflow" | "overflow-x" | "overflow-y" => {
                let moves = |one: &str| one == "auto" || one == "scroll" || one == "overlay";
                let said = words(value);
                let first = said.first().copied().unwrap_or("");
                let second = said.get(1).copied().unwrap_or(first);
                let across = name != "overflow-y" && moves(first);
                // `overflow` is across and then down, and each of the others is only that one.
                let down =
                    name != "overflow-x" && moves(if name == "overflow" { second } else { first });
                let was = props.get("__scroll");
                let was_across =
                    matches!(was, Some(Val::Text(was)) if was == "horizontal" || was == "both");
                let was_down =
                    matches!(was, Some(Val::Text(was)) if was == "vertical" || was == "both");
                let is_across = if name == "overflow-y" {
                    was_across
                } else {
                    across
                };
                let is_down = if name == "overflow-x" { was_down } else { down };
                let scroll = match (is_across, is_down) {
                    (true, true) => text("both"),
                    (true, false) => text("horizontal"),
                    (false, true) => text("vertical"),
                    (false, false) => Val::Undefined,
                };
                let scrolls = !matches!(scroll, Val::Undefined);
                props.set("__scroll", scroll);
                // What each way says, as `getComputedStyle()` tells it (`overflowX` and `overflowY` in web.js): react-remove-scroll
                // only lets a wheel through to what says `auto` or `scroll` the way it goes.
                let word = |one: &str| match one {
                    "visible" => Some(text("visible")),
                    "hidden" => Some(text("hidden")),
                    "clip" => Some(text("clip")),
                    "scroll" => Some(text("scroll")),
                    "auto" => Some(text("auto")),
                    "overlay" => Some(text("auto")),
                    _ => None,
                };
                if name != "overflow-y" {
                    if let Some(said) = word(first) {
                        props.set("__overflowX", said);
                    }
                }
                if name != "overflow-x" {
                    if let Some(said) = word(if name == "overflow" { second } else { first }) {
                        props.set("__overflowY", said);
                    }
                }
                // `scroll` has a scrollbar whether there's more to see or not, which takes room.
                props.set("__scrollAlways", Val::Bool(said.contains(&"scroll")));
                // What doesn't scroll and is hidden is cut off at its edges, as what scrolls is.
                let cut = said.iter().any(|one| *one == "hidden" || *one == "clip");
                if !scrolls && cut {
                    props.set("clip", Val::Bool(true));
                } else if name == "overflow" {
                    props.delete("clip");
                }
            }
            "background" | "background-color" => {
                if matches!(value, "none" | "inherit" | "unset" | "initial") {
                    props.set("background", text("#00000000"));
                    return Ok(());
                }
                if is_current_color(value) {
                    props.set("background", text("currentcolor"));
                    return Ok(());
                }
                if value.is_empty() {
                    return Ok(());
                }
                if name == "background" {
                    props.delete("backgroundImage");
                    props.delete("backgroundGradient");
                    if self.set_background_layers(props, value, true)? {
                        return Ok(());
                    }
                }
                if let Some(paint) = color(value) {
                    props.set("background", Val::Text(paint));
                    return Ok(());
                }
                if let Some(gradient) = self.linear_gradient(value)? {
                    props.set("backgroundGradient", owned(gradient));
                    return Ok(());
                }
                // An image, or a gradient that can't be drawn, with the first color there is in it, if it has one, as the nearest
                // thing to that.
                self.not_here(format!("background as {value}"));
                if let Some(inside) = first_color_in(value) {
                    props.set(
                        "background",
                        color(inside).map_or(Val::Undefined, Val::Text),
                    );
                }
            }
            "background-image" => {
                props.delete("backgroundImage");
                props.delete("backgroundGradient");
                if !self.set_background_layers(props, value, false)? && value != "none" {
                    self.not_here(format!("{name} as {value}"));
                }
            }
            "grid-template-areas" => {
                if value == "none" {
                    props.delete("gridAreas");
                } else {
                    props.set("gridAreas", owned(value));
                }
            }
            "grid-area" => {
                // A name, or where it starts and ends, as `row / column / row / column`.
                let parts: Vec<&str> = value.split('/').map(trim).collect();
                if parts.len() == 1 && is_area_name(parts[0]) && parts[0] != "auto" {
                    props.set("gridArea", owned(parts[0]));
                } else {
                    let row_start = parts[0];
                    let column_start = parts.get(1).copied().unwrap_or("auto");
                    let row = match parts.get(2) {
                        None => row_start.to_owned(),
                        Some(end) => format!("{row_start} / {end}"),
                    };
                    let column = match parts.get(3) {
                        None => column_start.to_owned(),
                        Some(end) => format!("{column_start} / {end}"),
                    };
                    props.set("gridRow", owned(row));
                    props.set("gridColumn", owned(column));
                }
            }
            "-rsui-native-border" => {
                // How much more room a control's border takes than it looks like it does, as a browser's take 2 pixels and draw
                // 1, which only counts while nothing says what its border is.
                let size = self.length(value, 16.0);
                props.set("__nativeBorder", size_val(size));
                props.delete("__ownBorder");
            }
            "-rsui-natural-width" => {
                // How wide a field is if nothing says, which stretching across what it's in still changes, as a browser's does.
                let size = self.length(value, 16.0);
                props.set("naturalWidth", size_val(size));
            }
            // Whether a textarea has the grip a browser draws in its corner, which `resize: none` takes away: see `TextArea` in
            // `web.js`.
            "resize" => props.set("__resize", owned(first_of(value))),
            // `field-sizing: content`: a field as big as what's in it, as shadcn's Textarea grows a line at a time: see `measure`
            // in rsui-text.
            "field-sizing" => {
                if first_of(value) == "content" {
                    props.set("fieldSizing", owned("content"));
                } else {
                    props.delete("fieldSizing");
                }
            }
            "accent-color" => {
                if value == "auto" {
                    props.delete("accentColor");
                } else {
                    self.set_color(props, name, value, "accentColor");
                }
            }
            // A scrollbar as CSS says it, which goes over what `::-webkit-scrollbar` says, as Chromium has it: `thin`, `none`,
            // or a browser's own, and the colors of its thumb and its track.
            "scrollbar-width" => match value {
                "auto" => props.set("__scrollbarWidth", text("auto")),
                "thin" => props.set("__scrollbarWidth", text("thin")),
                "none" => props.set("__scrollbarWidth", text("none")),
                _ => self.not_here(format!("{name} as {value}")),
            },
            "scrollbar-color" => {
                if value == "auto" {
                    props.set("__scrollbarThumb", Val::Undefined);
                    props.set("__scrollbarTrack", Val::Undefined);
                    props.set("__scrollbarColor", text("auto"));
                } else if let [thumb, track] = words(value)[..] {
                    self.set_color(props, name, thumb, "__scrollbarThumb");
                    self.set_color(props, name, track, "__scrollbarTrack");
                    props.set("__scrollbarColor", text("said"));
                } else {
                    self.not_here(format!("{name} as {value}"));
                }
            }
            "background-clip" | "-webkit-background-clip" => {
                // Text that shows its background through it, as gradient text does.
                if value == "text" {
                    props.set("backgroundClip", text("text"));
                } else {
                    props.delete("backgroundClip");
                }
            }
            "clip-path" => {
                if value == "none" {
                    props.delete("clipPath");
                } else {
                    props.set("clipPath", owned(value));
                    // A rectangle in from its edges cuts off what's in it too, as Mantine's half star is cut, which is drawn
                    // as what's cut off at its edges is: see `clip_to` in rsui-render.
                    if value.starts_with("inset(") {
                        props.set("clip", Val::Bool(true));
                    }
                }
            }
            "filter" => {
                if value == "none" {
                    props.delete("cssFilter");
                } else {
                    props.set("cssFilter", owned(value));
                    // A shadow of a box is the shadow of what it shows, which is what it is if its background covers it.
                    if let Some(shadow) = drop_shadow(value) {
                        if props.is_undefined("shadowBlur") && props.is_undefined("shadows") {
                            let parts = words(shadow);
                            let mut sizes = Vec::new();
                            for part in &parts {
                                if let Size::Pixels(size) = self.length(part, 16.0) {
                                    sizes.push(size);
                                }
                            }
                            let paint = parts.iter().find_map(|part| color(part));
                            let at = |index: usize| sizes.get(index).copied().unwrap_or(0.0);
                            let (x, y, blur) = (
                                self.number(at(0)),
                                self.number(at(1)),
                                self.number(at(2) * DROP_SHADOW_BLUR),
                            );
                            let said = format!(
                                "{x}px {y}px {blur}px {}",
                                paint.as_deref().unwrap_or("black")
                            );
                            self.set_shadow(props, &said);
                        }
                    }
                    if value.contains("blur(") || value.contains("url(") {
                        self.not_here(format!("filter as {value}"));
                    }
                }
            }
            "background-size" => {
                // In pixels, as drawing it takes, as `calc(1.25rem * 1)` comes to, apart from what's in `em`, which is the size of
                // the text where it's drawn, as Pico's checkbox's `.75em auto` is, and which drawing it works out.
                let first = first_of(value);
                if first.split_whitespace().any(|word| word.ends_with("em") && !word.ends_with("rem")) {
                    props.set("backgroundSize", owned(first));
                } else {
                    let mut out = Vec::new();
                    for word in words(trim(&first)) {
                        let unitless = word.ends_with('%') || word.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-');
                        match self.length(word, font_size) {
                            Size::Pixels(pixels) if !unitless => out.push(format!("{}px", self.number(pixels))),
                            _ => out.push(word.to_owned()),
                        }
                    }
                    props.set("backgroundSize", owned(out.join(" ")));
                }
            }
            "background-position" => {
                let placed = self.position_in_pixels(&first_of(value), font_size);
                props.set("backgroundPosition", owned(placed));
            }
            "background-repeat" => props.set("backgroundRepeat", owned(first_of(value))),
            "color" => self.set_color(props, name, value, "color"),
            "fill" => self.set_color(props, name, value, "fill"),
            "stroke" => self.set_color(props, name, value, "stroke"),
            "stroke-width" => self.set_number(props, name, value, font_size, "strokeWidth"),
            "stroke-linecap" => props.set("strokeLinecap", owned(value)),
            "stroke-linejoin" => props.set("strokeLinejoin", owned(value)),
            "stroke-dasharray" => props.set("strokeDasharray", owned(value)),
            "stroke-dashoffset" => {
                self.set_number(props, name, value, font_size, "strokeDashoffset")
            }
            "opacity" => {
                let opacity = if value.ends_with('%') {
                    parse_float(value) / 100.0
                } else {
                    parse_float(value)
                };
                if opacity.is_finite() {
                    props.set("opacity", Val::Number(opacity));
                } else {
                    // What isn't a number, as a variable that isn't set comes to, is what it starts as, which is all there, as
                    // CSS has it: Mantine's scrollbar thumb is `opacity: var(--thumb-opacity)` until its code sets that.
                    props.delete("opacity");
                }
            }
            "border" => self.set_border(props, value),
            "border-top" | "border-right" | "border-bottom" | "border-left" => {
                let side = side_of(&name[7..]).unwrap_or(0);
                let mut one = Props::default();
                self.set_border(&mut one, value);
                if let Some(width) = one
                    .get("borderWidth")
                    .filter(|width| !matches!(width, Val::Undefined))
                {
                    props.set(side_key("width", side), width.clone());
                }
                if let Some(paint) = one
                    .get("borderColor")
                    .filter(|paint| !matches!(paint, Val::Undefined))
                {
                    props.set(side_key("color", side), paint.clone());
                }
                // Dashes or dots on one side, as Mantine's dashed `Divider` and `Timeline` line have on their `::before`, are how
                // its border's drawn, as one style is all an element has here.
                if let Some(Val::Text(style)) = one.get("borderStyle") {
                    if style != "solid" {
                        props.set("borderStyle", Val::Text(style.clone()));
                    }
                }
            }
            "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width" => {
                let side = side_of(name.split('-').nth(1).unwrap_or("")).unwrap_or(0);
                self.set_number(props, name, value, font_size, side_key("width", side));
            }
            "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color" => {
                let side = side_of(name.split('-').nth(1).unwrap_or("")).unwrap_or(0);
                self.set_color(props, name, value, side_key("color", side));
            }
            "border-width" => {
                // It says each side, over what was said of one before, as MUI's Divider's `border-width: 0` does over the
                // browser's `<hr>` borders, before it says its own bottom one.
                without_border_sides(props);
                let parts = words(value);
                if parts.len() < 2 {
                    self.set_number(props, name, value, font_size, "borderWidth");
                    return Ok(());
                }
                // Each side as it says, in the order CSS has them, the ones not said taking the opposite one's.
                let read: Vec<Size> = parts
                    .into_iter()
                    .map(|part| self.length(part, font_size))
                    .collect();
                let sides = four(&read, |one| *one == Size::None, Size::None);
                for (side, size) in sides.into_iter().enumerate() {
                    if let Size::Pixels(size) = size {
                        props.set(side_key("width", side), Val::Number(size));
                    }
                }
                props.set("borderWidth", Val::Number(0.0));
            }
            "border-color" => {
                let parts = words(value);
                if parts.len() < 2 {
                    self.set_color(props, name, value, "borderColor");
                    return Ok(());
                }
                // Each side's own, in the order CSS has them, the ones not said taking the opposite one's, as a spinner's
                // `border-color: blue blue blue transparent` leaves a gap on one side.
                let sides = four(&parts, |_| false, "");
                self.set_color(props, name, sides[0], "borderColor");
                for (side, paint) in sides.into_iter().enumerate() {
                    self.set_color(props, name, paint, side_key("color", side));
                }
            }
            "border-style" => {
                if value == "none" || value == "hidden" {
                    without_border_sides(props);
                    props.set("borderWidth", Val::Number(0.0));
                } else {
                    let first = words(value).first().copied().and_then(drawn_border);
                    props.set("borderStyle", text(first.unwrap_or("solid")));
                }
            }
            "outline" => self.set_outline(props, value),
            "outline-width" => {
                self.set_number(props, name, value, font_size, "outlineWidth");
                if let Size::Pixels(number) = self.length(value, font_size) {
                    props.set("cssOutlineWidth", Val::Number(number));
                }
            }
            "outline-color" => self.set_color(props, name, value, "outlineColor"),
            "outline-offset" => self.set_number(props, name, value, font_size, "outlineOffset"),
            "outline-style" => {
                if value == "none" || value == "hidden" {
                    props.set("outlineWidth", Val::Number(0.0));
                } else if props.is_undefined("outlineWidth") {
                    props.set("outlineWidth", Val::Number(3.0));
                }
                set_outline_style(props, value);
            }
            "border-radius" => {
                // Up to four across, then up to four down after a `/`, from the top left going around, as CSS has it.
                let halves: Vec<Vec<&str>> =
                    value.split('/').map(|part| words(trim(part))).collect();
                let across = four_words(&halves[0]);
                let down = halves.get(1).map_or(across, |half| four_words(half));
                let mut corners = [0, 1, 2, 3].map(|_| [Corner::Pixels(0.0), Corner::Pixels(0.0)]);
                for (at, corner) in corners.iter_mut().enumerate() {
                    *corner = [
                        self.corner_size(across[at], font_size),
                        self.corner_size(down[at], font_size),
                    ];
                }
                self.set_corners(props, corners);
            }
            "border-top-left-radius"
            | "border-top-right-radius"
            | "border-bottom-right-radius"
            | "border-bottom-left-radius" => {
                let at = match name {
                    "border-top-left-radius" => 0,
                    "border-top-right-radius" => 1,
                    "border-bottom-right-radius" => 2,
                    _ => 3,
                };
                let said = words(value);
                let first = said.first().copied();
                let second = said.get(1).copied().or(first);
                let mut corners = self.corners_of(props);
                corners[at] = [
                    self.corner_size(first, font_size),
                    self.corner_size(second, font_size),
                ];
                self.set_corners(props, corners);
            }
            "box-shadow" => self.set_shadow(props, value),
            "font-size" => {
                if let Size::Pixels(size) = self.length(value, font_size) {
                    props.set("fontSize", Val::Number(size));
                }
            }
            "font-weight" => props.set("fontWeight", owned(value)),
            "font-family" => props.set("fontFamily", font_family(value)),
            "font-style" => props.set(
                "fontStyle",
                text(if value == "italic" || value == "oblique" {
                    "italic"
                } else {
                    "normal"
                }),
            ),
            "font" if matches!(value, "inherit" | "unset") => {
                // All of the font is the parent's, as `font: inherit` on a button or an input says, over what it has by itself.
                for key in ["fontFamily", "fontSize", "fontWeight", "fontStyle", "lineHeight", "__lineScale", "__fontScale"] {
                    props.set(key, Val::Undefined);
                }
            }
            "font" => {
                // `[style] [variant] [weight] [stretch] size[/line-height] family`. Whatever of the style and weight isn't said is
                // `normal` again, as the shorthand sets every part of the font. Nothing is set unless it has a size.
                let parts = words(value);
                let mut weight = "normal";
                let mut italic = false;
                for (at, part) in parts.iter().enumerate() {
                    let lower = part.to_ascii_lowercase();
                    match lower.as_str() {
                        "normal" | "small-caps" | "ultra-condensed" | "extra-condensed" | "condensed" | "semi-condensed"
                        | "semi-expanded" | "expanded" | "extra-expanded" | "ultra-expanded" => continue,
                        "bold" | "bolder" | "lighter" => {
                            weight = *part;
                            continue;
                        }
                        "italic" | "oblique" => {
                            italic = true;
                            continue;
                        }
                        _ if is_hundreds(part) => {
                            weight = *part;
                            continue;
                        }
                        _ => {}
                    }
                    // The size, and the line height after a `/`, which can stand apart from it (`14px / 1.4`).
                    let (size_text, mut line) = match part.split_once('/') {
                        Some((size, line)) => (size, (!line.is_empty()).then_some(line)),
                        None => (*part, None),
                    };
                    let mut family_at = at + 1;
                    if line.is_none() && (part.ends_with('/') || parts.get(at + 1).is_some_and(|next| next.starts_with('/'))) {
                        let next = parts.get(at + 1).copied().unwrap_or_default();
                        if next == "/" {
                            line = parts.get(at + 2).copied();
                            family_at = at + 3;
                        } else if let Some(rest) = next.strip_prefix('/') {
                            line = Some(rest);
                            family_at = at + 2;
                        } else {
                            line = Some(next);
                            family_at = at + 2;
                        }
                    }
                    let Size::Pixels(size) = self.length(size_text, font_size) else {
                        return Ok(());
                    };
                    props.set("fontWeight", owned(weight));
                    props.set("fontStyle", text(if italic { "italic" } else { "normal" }));
                    props.set("fontSize", Val::Number(size));
                    // Its size is a share of what it's in, or not, as `font-size` says, over what said it was before, as a
                    // class's `font: 20px Arial` is over the browser's `h1 { font-size: 2em }`.
                    let trimmed = trim(size_text);
                    props.set("__fontScale", em_share(trimmed).map_or(Val::Null, Val::Number));
                    if let Some(named) = font_size_word(trimmed) {
                        props.set("__fontScale", Val::Number(named));
                    }
                    if let Some(line) = line {
                        self.declare_one(props, "line-height", line, size)?;
                    }
                    if parts.len() > family_at {
                        props.set("fontFamily", font_family(&parts[family_at..].join(" ")));
                    }
                    break;
                }
            }
            "text-align" => {
                if matches!(value, "left" | "center" | "right" | "start" | "end") {
                    // Which side `start` and `end` are depends on which way the text goes, which is only known where the text is.
                    props.set("textAlign", owned(value));
                }
            }
            "direction" => props.set(
                "textDirection",
                text(if value == "rtl" { "rtl" } else { "ltr" }),
            ),
            // How an inline box's letters take its `direction`, and whether a block's paragraphs go by their first letter.
            "unicode-bidi" => {
                if matches!(value, "normal" | "embed" | "isolate" | "bidi-override" | "isolate-override" | "plaintext") {
                    props.set("unicodeBidi", owned(value));
                }
            }

            // What's first and last in it keeps its margins in it, as `<fieldset>`'s do: for the browser's own CSS.
            "-rsui-own-margins" => props.set("__ownMargins", Val::Bool(true)),
            // Everything as if nothing had said anything about it, which is what's in it says, for how text is written, and
            // nothing otherwise, as Radix's `.rt-reset:where(button) { all: unset }` has a button's browser look gone. What
            // comes after it in the same rule says what it says. `revert` is the browser's look, which is there already.
            "all" => {
                if matches!(value, "unset" | "initial" | "inherit") {
                    for key in all_keys(self.vm) {
                        props.set_key(*key, Val::Undefined);
                    }
                }
            }
            "-webkit-app-region" | "app-region" => {
                // Taking hold of it moves the window, as a title bar does in a window with no frame of its own, apart from what
                // says it doesn't, and what can be clicked.
                if value == "drag" || value == "no-drag" {
                    props.set("dragsWindow", Val::Bool(value == "drag"));
                }
            }
            "text-decoration" | "text-decoration-line" => {
                // Which lines, and, for all of it at once, how they look, with what isn't said going back to how it is by itself.
                let spaced_out = value.replace(',', " ");
                let said = words(&spaced_out);
                props.set("underline", Val::Bool(said.contains(&"underline")));
                props.set("lineThrough", Val::Bool(said.contains(&"line-through")));
                props.set("overline", Val::Bool(said.contains(&"overline")));
                if name == "text-decoration" {
                    let style = said.iter().find(|word| decoration_style(word));
                    let paint = said.iter().find_map(|word| color(word));
                    // Each is worked out, as one in `vw` says the page has to be worked out again when the window changes size.
                    let mut thick = None;
                    for word in &said {
                        if let Size::Pixels(size) = self.length(word, font_size) {
                            thick = thick.or(Some(size));
                        }
                    }
                    props.set(
                        "decorationStyle",
                        style.map_or(text("solid"), |style| owned(*style)),
                    );
                    // What isn't said goes back to how it is by itself, as the shorthand has it, which goes over an earlier
                    // rule's, and over what a variable said before, as Pico's `nav li a { text-decoration: none }` does.
                    match paint {
                        Some(paint) => props.set("decorationColor", Val::Text(paint)),
                        None => props.set("decorationColor", Val::Undefined),
                    }
                    match thick {
                        Some(thick) => props.set("decorationThickness", Val::Number(thick)),
                        None => props.set("decorationThickness", Val::Undefined),
                    }
                }
            }
            "text-decoration-style" => props.set("decorationStyle", owned(value)),
            "text-decoration-color" => self.set_color(props, name, value, "decorationColor"),
            "text-decoration-thickness" => {
                self.set_number(props, name, value, font_size, "decorationThickness")
            }
            "text-underline-offset" => {
                self.set_number(props, name, value, font_size, "underlineOffset")
            }
            "content" => {
                // What's in `::before` and `::after`: it's only there at all if it says this. What it says is read in `web.js`.
                if self.declaring.is_object() {
                    let said = self.call("content", value)?;
                    if said != Value::UNDEFINED {
                        props.set("__content", Val::Js(said));
                    }
                }
            }
            "line-height" => {
                // `normal` is as far apart as the font's own lines are, which is what nothing said is (`null`), rather than what
                // it's in says, and `inherit` is what it's in says, whatever comes before it said, as a page's `button {
                // line-height: inherit }` says over the browser's `normal`.
                if matches!(value, "normal" | "initial" | "inherit" | "unset") {
                    let said = if matches!(value, "normal" | "initial") { Val::Null } else { Val::Undefined };
                    props.set("lineHeight", said.clone());
                    props.set("__lineScale", said);
                    return Ok(());
                }
                let plain =
                    !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit() || c == b'.');
                // A `calc()` of numbers without units is a number too, as Tailwind's `text-sm` says `calc(1.25 / .875)`.
                let unitless_calc = value
                    .strip_prefix("calc(")
                    .is_some_and(|inside| !inside.bytes().any(|c| c.is_ascii_alphabetic() || c == b'%'));
                let scale = if plain {
                    Some(parse_float(value))
                } else if unitless_calc {
                    match self.length(value, 1.0) {
                        Size::Pixels(number) => Some(number),
                        _ => None,
                    }
                } else {
                    None
                };
                let size = match scale {
                    Some(scale) => Size::Pixels(scale * font_size),
                    None => self.length(value, font_size),
                };
                // A number on its own is times the size of the text, whatever that turns out to be, here and in everything inside it.
                props.set(
                    "__lineScale",
                    match scale {
                        Some(scale) => Val::Number(scale),
                        None => Val::Null,
                    },
                );
                match size {
                    Size::Pixels(size) => props.set("lineHeight", Val::Number(size)),
                    Size::Text(size) if size.ends_with('%') => {
                        props.set(
                            "lineHeight",
                            Val::Number((parse_float(&size) * font_size) / 100.0),
                        );
                    }
                    _ => {}
                }
            }
            "list-style" | "list-style-type" => {
                let said = words(value);
                if let Some(kind) = said.iter().find_map(|word| list_type(word)) {
                    props.set("__listStyle", text(kind));
                } else if name == "list-style-type" || said.contains(&"none") {
                    props.set("__listStyle", text("none"));
                }
            }
            "vertical-align" => {
                // Above or below the text around it, which is the only way text inside text goes.
                if value == "super" || value == "sub" {
                    props.set("verticalAlign", owned(value));
                } else if value == "baseline" {
                    props.delete("verticalAlign");
                    props.delete("__lineAlign");
                }
                // A box on a line at its top, middle or bottom, rather than on the line's text.
                match value {
                    "top" | "text-top" => props.set("__lineAlign", text("start")),
                    "bottom" | "text-bottom" => props.set("__lineAlign", text("end")),
                    "middle" => props.set("__lineAlign", text("center")),
                    _ => {}
                }
            }
            "-webkit-line-clamp" | "line-clamp" => {
                // Only so many lines, the last of which ends in `…` if there's more.
                let lines = parse_int(value);
                if lines > 0.0 {
                    props.set("maxLines", Val::Number(lines));
                } else {
                    props.delete("maxLines");
                }
            }
            "-webkit-box-orient" => {}
            "overflow-wrap" | "word-wrap" => {
                // A word too wide for a line is broken, rather than going past the edge.
                props.set(
                    "keepWords",
                    Val::Bool(!(value == "break-word" || value == "anywhere")),
                );
            }
            "word-break" => {
                if value == "break-all" || value == "break-word" {
                    props.set("keepWords", Val::Bool(false));
                } else if value == "keep-all" || value == "normal" {
                    props.set("keepWords", Val::Bool(true));
                }
                // Lines end anywhere, not only where a word's too long.
                if value == "break-all" {
                    props.set("wordBreak", text("break-all"));
                } else {
                    props.delete("wordBreak");
                }
            }
            "letter-spacing" => {
                // `normal` is none, and so is anything that isn't a length.
                let spacing = match self.length(value, font_size) {
                    Size::Pixels(size) => size,
                    _ => 0.0,
                };
                props.set("letterSpacing", Val::Number(spacing));
            }
            "word-spacing" => {
                // `normal` is none, and so is anything that isn't a length.
                let spacing = match self.length(value, font_size) {
                    Size::Pixels(size) => size,
                    _ => 0.0,
                };
                props.set("wordSpacing", Val::Number(spacing));
            }
            "text-indent" => match self.length(value, font_size) {
                Size::Pixels(indent) => props.set("textIndent", Val::Number(indent)),
                _ => self.not_here(format!("{name} as {value}")),
            },
            "text-transform" => {
                if matches!(value, "uppercase" | "lowercase" | "capitalize" | "none") {
                    props.set("__case", owned(value));
                }
            }
            "text-overflow" => {
                if value == "ellipsis" {
                    props.set("ellipsis", Val::Bool(true));
                }
            }
            "white-space" => {
                props.set("__whiteSpace", owned(value));
                if value == "nowrap" || value == "pre" {
                    props.set("noWrap", Val::Bool(true));
                } else {
                    props.delete("noWrap");
                }
            }
            "visibility" => props.set("__visibility", owned(value)),
            // Kept for the page's selection to go by (see `selectsText` in web.js), and nothing else.
            "user-select" | "-webkit-user-select" => props.set("__userSelect", owned(value)),
            "appearance" | "-webkit-appearance" | "-moz-appearance" => {
                props.set("__appearance", owned(value))
            }
            "cursor" => props.set(
                "cursor",
                text(match value {
                    "pointer" => "pointer",
                    "text" => "text",
                    _ => "default",
                }),
            ),
            // What says it takes the pointer does, even in what doesn't, as input-otp's `<input>` (`pointer-events: all`) in its
            // `pointer-events: none` box: `inert` false says so, which is otherwise what nothing saying it is.
            "pointer-events" => props.set("inert", Val::Bool(value == "none")),
            "object-fit" => {
                if matches!(value, "contain" | "cover" | "fill") {
                    props.set("fit", owned(value));
                }
            }
            // `none` is said too, so that it wins over a transform an earlier rule gave, as Mantine's checked checkbox's
            // `transform: none` does over its icon's `translateY(...) scale(0.5)`. `withTransformParts` in `web.js` takes it off again.
            "transform" => {
                // Computed absolute lengths share one unit before interpolation:
                // Tailwind switches go from translate(0px, ...) to translate(1rem, ...).
                // Preserve percentages, which still resolve against the element's box.
                let value = self.grid_tracks(value, font_size);
                props.set("transform", owned(value));
            }
            "transform-origin" => props.set("transformOrigin", owned(value)),
            // Each of these is a part of the transform of its own, which `web.js` puts together with `transform`, as CSS does: moved,
            // then turned, then made bigger or smaller, then the rest.
            "translate" => {
                let value = self.grid_tracks(value, font_size);
                props.set("__translate", owned(value));
            }
            "rotate" => props.set("__rotate", owned(value)),
            "scale" => props.set("__scale", owned(value)),
            "border-collapse" => props.set("__borderCollapse", Val::Bool(value == "collapse")),
            "border-spacing" => {
                let first = words(value).first().copied().unwrap_or("");
                if let Size::Pixels(spacing) = self.length(first, font_size) {
                    props.set("__borderSpacing", Val::Number(spacing));
                }
            }
            // `inherit` is nearly always from `html { box-sizing: border-box }`, which is what it comes to.
            "box-sizing" => props.set(
                "__boxSizing",
                if value == "inherit" {
                    text("border-box")
                } else {
                    owned(value)
                },
            ),
            "aspect-ratio" => {
                let parts: Vec<&str> = value.split('/').map(trim).collect();
                let ratio = to_number(parts[0]) / to_number(parts.get(1).copied().unwrap_or("1"));
                if ratio.is_finite() && ratio > 0.0 {
                    props.set("__aspect", Val::Number(ratio));
                } else if value != "auto" {
                    self.not_here(format!("aspect-ratio as {value}"));
                }
            }
            "flex-wrap" => {
                props.set(
                    "flexWrap",
                    Val::Bool(value == "wrap" || value == "wrap-reverse"),
                );
                if value == "wrap-reverse" {
                    self.not_here(
                        "flex-wrap: wrap-reverse, which wraps the usual way instead,".to_owned(),
                    );
                }
            }
            "flex-flow" => {
                for word in words(value) {
                    if word == "wrap" || word == "wrap-reverse" || word == "nowrap" {
                        self.declare(props, "flex-wrap", word, font_size)?;
                    } else {
                        self.declare(props, "flex-direction", word, font_size)?;
                    }
                }
            }
            "grid-auto-flow" => props.set("__flow", owned(value)),
            "align-content" => props.set("__alignContent", owned(value)),
            "grid-template-columns" => {
                let tracks = self.grid_tracks(value, font_size);
                props.set("columns", owned(tracks));
            }
            "grid-template-rows" => {
                let tracks = self.grid_tracks(value, font_size);
                props.set("rows", owned(tracks));
            }
            "grid-auto-rows" => {
                let tracks = self.grid_tracks(value, font_size);
                props.set("gridAutoRows", owned(tracks));
            }
            "grid-auto-columns" => {
                let tracks = self.grid_tracks(value, font_size);
                props.set("gridAutoColumns", owned(tracks));
            }
            "grid-column" => props.set("gridColumn", owned(value)),
            "grid-row" => props.set("gridRow", owned(value)),
            // Where it starts or ends, which goes over that part of `grid-column` and `grid-row`, wherever that's said.
            "grid-column-start" => props.set("gridColumnStart", owned(value)),
            "grid-column-end" => props.set("gridColumnEnd", owned(value)),
            "grid-row-start" => props.set("gridRowStart", owned(value)),
            "grid-row-end" => props.set("gridRowEnd", owned(value)),
            "z-index" => {
                let order = parse_int(value);
                if !order.is_nan() {
                    props.set("zIndex", Val::Number(order));
                }
            }
            "transition" => {
                props.set("__transition", owned(value));
                props.delete("__transitionParts");
            }
            "transition-property"
            | "transition-duration"
            | "transition-delay"
            | "transition-timing-function" => {
                let parts = self.new_parts(props.get("__transitionParts"), name, value);
                props.set("__transitionParts", parts);
            }
            "text-shadow" => {
                // The first shadow, as `x y blur color`, with the color as a number. It goes on to what's inside, as a font does.
                let first = commas(value).first().map(|first| trim(first));
                let Some(first) = first.filter(|_| value != "none") else {
                    props.set("textShadow", text("none"));
                    return Ok(());
                };
                let mut sizes = Vec::new();
                let mut paint = None;
                for part in words(first) {
                    if let Size::Pixels(size) = self.length(part, font_size) {
                        sizes.push(size);
                    } else if let Some(found) = color(part) {
                        paint = Some(found);
                    }
                }
                let at = |index: usize| sizes.get(index).copied().unwrap_or(0.0);
                let (x, y, blur) = (self.number(at(0)), self.number(at(1)), self.number(at(2)));
                let paint = color_number(paint.as_deref().unwrap_or("currentcolor"))
                    .or_else(|| color_number("#000000"));
                let paint = paint.map_or("undefined".to_owned(), |paint| paint.to_string());
                props.set("textShadow", owned(format!("{x} {y} {blur} {paint}")));
            }
            "table-layout" => props.set("__tableLayout", owned(value)),
            // Whether a table's caption is above its rows or below them: see `below_rows` in rsui-layout's `table.rs`.
            "caption-side" => props.set("captionSide", owned(value)),
            "float" => match value {
                "left" => props.set("__float", text("left")),
                "right" => props.set("__float", text("right")),
                // Which side these are depends on which way the text goes, which is only known in the block it's in (`withFloats`).
                "inline-start" => props.set("__float", text("inline-start")),
                "inline-end" => props.set("__float", text("inline-end")),
                _ => props.delete("__float"),
            },

            "clear" => {
                if value == "none" {
                    props.delete("__clear");
                } else {
                    props.set("__clear", Val::Bool(true));
                }
            }
            "animation" => {
                props.set("__animation", owned(value));
                props.delete("__animationParts");
            }
            "animation-name"
            | "animation-duration"
            | "animation-delay"
            | "animation-iteration-count"
            | "animation-timing-function"
            | "animation-direction"
            | "animation-fill-mode"
            | "animation-play-state" => {
                let parts = self.new_parts(props.get("__animationParts"), name, value);
                props.set("__animationParts", parts);
            }
            "text-wrap" | "text-wrap-mode" | "text-wrap-style" => {
                // Whether lines wrap at all, and whether they're balanced, which what's inside gets too, as CSS has it.
                if value == "nowrap" && name != "text-wrap-style" {
                    self.declare(props, "white-space", "nowrap", font_size)?;
                }
                let words = words(value);
                if words.contains(&"balance") {
                    props.set("textWrap", text("balance"));
                } else if name != "text-wrap-mode" && words.iter().any(|word| matches!(*word, "wrap" | "auto" | "pretty" | "stable")) {
                    props.set("textWrap", text("wrap"));
                } else if matches!(value, "inherit" | "unset") {
                    props.set("textWrap", Val::Undefined);
                }
            }
            // Digits all as wide as each other, as Chakra's badges have them, which goes down to what's in it.
            "font-variant-numeric" => {
                let said = words(value);
                if said.contains(&"tabular-nums") {
                    props.set("tabularNums", Val::Bool(true));
                } else if said.iter().any(|word| matches!(*word, "proportional-nums" | "normal")) {
                    props.set("tabularNums", Val::Bool(false));
                } else if matches!(value, "inherit" | "unset") {
                    props.set("tabularNums", Val::Undefined);
                }
            }
            "font-feature-settings" if tnum_setting(value).is_some() => {
                props.set("tabularNums", Val::Bool(tnum_setting(value) == Some(true)));
            }
            // Fine adjustments to a font's own features, of which what's usual is what's here.
            "font-feature-settings" | "font-variation-settings" | "font-kerning" | "font-optical-sizing"
                if matches!(value, "normal" | "auto" | "inherit" | "initial" | "unset") => {}
            _ => {
                if let Some(physical) = logical(name) {
                    return self.declare_logical(props, physical, value, font_size);
                }
                // A browser's own name for something, as `-webkit-text-decoration` is, is the same as what it's a name for, which
                // is said as well wherever it's said. What's only that browser's isn't anything here.
                if let Some(bare) = ["-webkit-", "-moz-", "-ms-", "-o-"].iter().find_map(|prefix| name.strip_prefix(prefix)) {
                    let said = self.said.len();
                    self.declare(props, bare, value, font_size)?;
                    self.said.truncate(said);
                    return Ok(());
                }
                // What starts with `_` is only for an old browser, as `_height` is, and is nothing to any other.
                if !silent(name) && !name.starts_with("--") && !name.starts_with('_') {
                    self.not_here(name.to_owned());
                }
            }
        }
        Ok(())
    }

    /// A property said along a line and across lines, as `margin-inline` is, as the sides it means in text that goes from
    /// left to right and down the page.
    fn declare_logical(
        &mut self,
        props: &mut Props,
        physical: Logical,
        value: &str,
        font_size: f64,
    ) -> Run<()> {
        // What's along a line goes to the sides it means where text goes from left to right, and what it put there is noted
        // too, under `~` and the name, for `logicalSides` in web.js to move to the other side where text goes from right to
        // left, once that's known: which way the text goes is passed down, and only known where it's used. What goes on both
        // sides alike is the same either way.
        let along = |name: &str| {
            name.contains("left") || name.contains("right")
        };
        match physical {
            Logical::One(name) if along(name) => {
                let before = props.clone();
                self.declare(props, name, value, font_size)?;
                noted_logical(&before, props);
                Ok(())
            }
            Logical::One(name) => self.declare(props, name, value, font_size),
            Logical::Two(start, end) => {
                let parts = words(value);
                let first = parts.first().copied().unwrap_or(value);
                let second = parts.get(1).copied().unwrap_or(first);
                let before = (along(start) && first != second).then(|| props.clone());
                self.declare(props, start, first, font_size)?;
                self.declare(props, end, second, font_size)?;
                if let Some(before) = before {
                    noted_logical(&before, props);
                }
                Ok(())
            }
        }
    }
}

/// The name a side's prop is noted under when a logical property set it (see `declare_logical`), for the props that have a
/// side along a line.
fn logical_note(key: &str) -> Option<&'static str> {
    Some(match key {
        "marginLeft" => "~marginLeft",
        "marginRight" => "~marginRight",
        "marginLeftShare" => "~marginLeftShare",
        "marginRightShare" => "~marginRightShare",
        "__autoLeft" => "~__autoLeft",
        "__autoRight" => "~__autoRight",
        "paddingLeft" => "~paddingLeft",
        "paddingRight" => "~paddingRight",
        "paddingLeftShare" => "~paddingLeftShare",
        "paddingRightShare" => "~paddingRightShare",
        "left" => "~left",
        "right" => "~right",
        "borderLeftWidth" => "~borderLeftWidth",
        "borderRightWidth" => "~borderRightWidth",
        "borderLeftColor" => "~borderLeftColor",
        "borderRightColor" => "~borderRightColor",
        "cornerRadii" => "~cornerRadii",
        _ => return None,
    })
}

/// Notes what a logical property set, which is what's in `now` that isn't in `before` or is something else there.
fn noted_logical(before: &Props, now: &mut Props) {
    let mut notes = Vec::new();
    for (key, value) in &now.0 {
        let (name, em) = match key {
            Key::Plain(name) => (*name, false),
            Key::Em(name) => (*name, true),
        };
        let Some(note) = logical_note(name) else {
            continue;
        };
        let changed = match before.get_key(*key) {
            None => true,
            Some(was) => !same_val(was, value),
        };
        if changed {
            notes.push((if em { Key::Em(note) } else { Key::Plain(note) }, value.clone()));
        }
    }
    if notes.is_empty() {
        return;
    }
    for (key, value) in notes {
        now.set_key(key, value);
    }
    now.set("__logical", Val::Bool(true));
}

/// Whether two values are the same, for what's made here, without an engine to ask about what JavaScript made.
fn same_val(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Undefined, Val::Undefined) | (Val::Null, Val::Null) => true,
        (Val::Bool(x), Val::Bool(y)) => x == y,
        (Val::Number(x), Val::Number(y)) => x == y,
        (Val::Text(x), Val::Text(y)) => x == y,
        _ => false,
    }
}

/// What a logical property means as physical ones: one, or the two sides that the first and second of its values go to.
#[derive(Clone, Copy)]
enum Logical {
    One(&'static str),
    Two(&'static str, &'static str),
}

/// The physical properties that a logical one means, in text that goes from left to right and down the page.
fn logical(name: &str) -> Option<Logical> {
    use Logical::{One, Two};
    Some(match name {
        "margin-inline" => Two("margin-left", "margin-right"),
        "margin-block" => Two("margin-top", "margin-bottom"),
        "margin-inline-start" => One("margin-left"),
        "margin-inline-end" => One("margin-right"),
        "margin-block-start" => One("margin-top"),
        "margin-block-end" => One("margin-bottom"),
        "padding-inline" => Two("padding-left", "padding-right"),
        "padding-block" => Two("padding-top", "padding-bottom"),
        "padding-inline-start" => One("padding-left"),
        "padding-inline-end" => One("padding-right"),
        "padding-block-start" => One("padding-top"),
        "padding-block-end" => One("padding-bottom"),
        "inset-inline" => Two("left", "right"),
        "inset-block" => Two("top", "bottom"),
        "inset-inline-start" => One("left"),
        "inset-inline-end" => One("right"),
        "inset-block-start" => One("top"),
        "inset-block-end" => One("bottom"),
        "inline-size" => One("width"),
        "block-size" => One("height"),
        "min-inline-size" => One("min-width"),
        "max-inline-size" => One("max-width"),
        "min-block-size" => One("min-height"),
        "max-block-size" => One("max-height"),
        "border-inline-start" => One("border-left"),
        "border-inline-end" => One("border-right"),
        "border-block-start" => One("border-top"),
        "border-block-end" => One("border-bottom"),
        "border-inline-start-width" => One("border-left-width"),
        "border-inline-end-width" => One("border-right-width"),
        "border-block-start-width" => One("border-top-width"),
        "border-block-end-width" => One("border-bottom-width"),
        "border-inline-start-color" => One("border-left-color"),
        "border-inline-end-color" => One("border-right-color"),
        "border-block-start-color" => One("border-top-color"),
        "border-block-end-color" => One("border-bottom-color"),
        "border-start-start-radius" => One("border-top-left-radius"),
        "border-start-end-radius" => One("border-top-right-radius"),
        "border-end-start-radius" => One("border-bottom-left-radius"),
        "border-end-end-radius" => One("border-bottom-right-radius"),
        _ => return None,
    })
}

/// How much softer a `drop-shadow()` is than a `box-shadow` that says the same, as Chromium has them.
const DROP_SHADOW_BLUR: f64 = 2.0;

/// A corner's size one way: pixels, a percentage, or nothing, which is what a list too short to have it has.
#[derive(Clone, Debug)]
enum Corner {
    Pixels(f64),
    Share(String),
    Missing,
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

/// `fourSides(words)`: up to four words for the sides, as four, which are `undefined` when there aren't any.
fn four_words<'t>(said: &[&'t str]) -> [Option<&'t str>; 4] {
    let given: Vec<Option<&str>> = said.iter().map(|word| Some(*word)).collect();
    four(&given, Option::is_none, None)
}

/// `commas(value)[0]?.trim() ?? value`.
fn first_of(value: &str) -> String {
    commas(value)
        .first()
        .map_or(value, |first| trim(first))
        .to_owned()
}

/// What `fontFamily` comes to: each of them, without quotes, as the system calls it if it's a kind of font, with commas
/// between, of which the first there is is used, as MUI's `"Roboto", "Helvetica", "Arial"` comes to Arial where Roboto isn't
/// installed, as in a browser.
fn font_family(value: &str) -> Val {
    let names: Vec<&str> = commas(value)
        .into_iter()
        .map(|one| {
            let one = trim(one);
            // `replace(/^["']|["']$/g, '')`: one quote off the start, and one off the end.
            let unquoted = one.strip_prefix(['"', '\'']).unwrap_or(one);
            let unquoted = unquoted.strip_suffix(['"', '\'']).unwrap_or(unquoted);
            generic_font(unquoted).unwrap_or(unquoted)
        })
        .filter(|one| !one.is_empty())
        .collect();
    match names.as_slice() {
        [] => text(""),
        [one] => owned(*one),
        _ => owned(names.join(",")),
    }
}

/// How wide a border's `thin`, `medium` or `thick` is.
fn line_width_word(word: &str) -> Option<&'static str> {
    Some(match word {
        "thin" => "1px",
        "medium" => "3px",
        "thick" => "5px",
        _ => return None,
    })
}

/// `/^[1-9]00$/`.
fn is_hundreds(word: &str) -> bool {
    let bytes = word.as_bytes();
    bytes.len() == 3 && (b'1'..=b'9').contains(&bytes[0]) && bytes[1] == b'0' && bytes[2] == b'0'
}

/// `/^[A-Za-z_-][\w-]*$/`.
fn is_area_name(word: &str) -> bool {
    let mut bytes = word.bytes();
    bytes
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_' || c == b'-')
        && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

/// `/^(closest|farthest)-/`.
fn is_extent(word: &str) -> bool {
    word.starts_with("closest-") || word.starts_with("farthest-")
}

/// `/^-?[0-9.]+(deg|turn|rad|grad)$/`.
fn is_angle(word: &str) -> bool {
    let rest = word.strip_prefix('-').unwrap_or(word);
    let digits = rest
        .bytes()
        .take_while(|c| c.is_ascii_digit() || *c == b'.')
        .count();
    digits > 0 && matches!(&rest[digits..], "deg" | "turn" | "rad" | "grad")
}

/// `/(linear|radial)-gradient\(/i` at the start: whether it's radial, and what's after it.
fn gradient_kind(text: &str) -> Option<(bool, &str)> {
    if let Some(after) = strip_ascii_prefix(text, "linear-gradient(") {
        return Some((false, after));
    }
    strip_ascii_prefix(text, "radial-gradient(").map(|after| (true, after))
}

/// `/^(repeating-)?(linear|radial)-gradient\(/i`: whether it repeats, whether it's radial, and what's after it.
fn gradient_start(text: &str) -> Option<(bool, bool, &str)> {
    if let Some((radial, after)) = strip_ascii_prefix(text, "repeating-").and_then(gradient_kind) {
        return Some((true, radial, after));
    }
    gradient_kind(text).map(|(radial, after)| (false, radial, after))
}

/// `/^\s*(repeating-)?(linear|radial)-gradient\((.*)\)\s*$/is`: whether it repeats, whether it's radial, and what's inside.
fn gradient_parts(value: &str) -> Option<(bool, bool, &str)> {
    let (repeating, radial, inside) = gradient_start(value.trim_start_matches(is_space))?;
    let close = inside.rfind(')')?;
    inside[close + 1..]
        .chars()
        .all(is_space)
        .then_some((repeating, radial, &inside[..close]))
}

fn strip_ascii_prefix<'t>(text: &'t str, prefix: &str) -> Option<&'t str> {
    let head = text.as_bytes().get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix.as_bytes())
        .then(|| &text[prefix.len()..])
}

/// `/url\(/i`.
fn contains_url(text: &str) -> bool {
    text.as_bytes()
        .windows(4)
        .any(|four| four.eq_ignore_ascii_case(b"url("))
}

/// What `/url\(\s*(?:"([^"]*)"|'([^']*)'|([^)]*?))\s*\)/` finds: the URL, whether it was in quotes, and where all of it starts
/// and ends.
struct Url<'t> {
    url: &'t str,
    bare: bool,
    start: usize,
    end: usize,
}

fn find_url(value: &str) -> Option<Url<'_>> {
    // If it's not the first `url(`, it's none of them, as what closes a later one would close the first.
    let start = value.find("url(")?;
    let after = start + 4;
    let inside = after + value[after..].len() - value[after..].trim_start_matches(is_space).len();
    let rest = &value[inside..];
    if let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') {
        if let Some(close) = rest[1..].find(quote) {
            let past = inside + 1 + close + 1;
            let spaces = value[past..].len() - value[past..].trim_start_matches(is_space).len();
            if value[past + spaces..].starts_with(')') {
                return Some(Url {
                    url: &rest[1..1 + close],
                    bare: false,
                    start,
                    end: past + spaces + 1,
                });
            }
        }
    }
    let close = inside + rest.find(')')?;
    let url = value[inside..close].trim_end_matches(is_space);
    Some(Url {
        url,
        bare: true,
        start,
        end: close + 1,
    })
}

/// `/(#[0-9a-fA-F]{3,8}|rgba?\([^)]*\)|hsla?\([^)]*\))/`: the first color written in hex, `rgb()` or `hsl()`.
fn first_color_in(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    for at in 0..bytes.len() {
        if bytes[at] == b'#' {
            let digits = bytes[at + 1..]
                .iter()
                .take(8)
                .take_while(|c| c.is_ascii_hexdigit())
                .count();
            if digits >= 3 {
                return Some(&value[at..at + 1 + digits]);
            }
            continue;
        }
        for name in [b"rgb", b"hsl"] {
            if bytes[at..].starts_with(name) {
                let mut open = at + 3;
                if bytes.get(open) == Some(&b'a') && bytes.get(open + 1) == Some(&b'(') {
                    open += 1;
                }
                if bytes.get(open) == Some(&b'(') {
                    if let Some(close) = value[open..].find(')') {
                        return Some(&value[at..open + close + 1]);
                    }
                }
            }
        }
    }
    None
}

/// What's inside `drop-shadow(...)`, as `/drop-shadow\(([^()]*(?:\([^()]*\)[^()]*)*)\)/` finds it: brackets inside it, but not
/// inside those.
fn drop_shadow(value: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(found) = value[from..].find("drop-shadow(") {
        let open = from + found + "drop-shadow(".len();
        let bytes = value.as_bytes();
        let mut at = open;
        let mut inner = false;
        while at < bytes.len() {
            match bytes[at] {
                b'(' if !inner => inner = true,
                b'(' => break,
                b')' if inner => inner = false,
                b')' => return Some(&value[open..at]),
                _ => {}
            }
            at += 1;
        }
        from = from + found + 1;
    }
    None
}

/// A color as rsui keeps it: red, green, blue and alpha in one number.
fn color_number(text: &str) -> Option<u32> {
    let hex = color(text)?;
    let mut digits: String = hex[1..].to_owned();
    if digits.len() <= 4 {
        digits = digits.chars().flat_map(|digit| [digit, digit]).collect();
    }
    if digits.len() == 6 {
        digits.push_str("ff");
    }
    u32::from_str_radix(&digits, 16).ok()
}

/// The name of each prop that can be said in `em`, as a share of the text's size, which is any prop that's a number and doesn't
/// start with `__`.
fn prop_key(name: &str) -> Option<&'static str> {
    PROP_KEYS.iter().find(|key| **key == name).copied()
}

const PROP_KEYS: [&str; 49] = [
    "basis",
    "borderBottomWidth",
    "borderLeftWidth",
    "borderRightWidth",
    "borderTopWidth",
    "borderWidth",
    "bottom",
    "columnGap",
    "decorationThickness",
    "fontSize",
    "gap",
    "grow",
    "height",
    "left",
    "letterSpacing",
    "lineHeight",
    "marginBottom",
    "marginLeft",
    "marginRight",
    "marginTop",
    "maxHeight",
    "maxLines",
    "maxWidth",
    "minHeight",
    "minWidth",
    "naturalWidth",
    "opacity",
    "outlineOffset",
    "outlineWidth",
    "paddingBottom",
    "paddingLeft",
    "paddingRight",
    "paddingTop",
    "radius",
    "right",
    "rowGap",
    "shadowBlur",
    "shadowSpread",
    "shadowX",
    "shadowY",
    "shrink",
    "strokeDashoffset",
    "strokeWidth",
    "textIndent",
    "top",
    "underlineOffset",
    "width",
    "wordSpacing",
    "zIndex",
];

/// What part of a table `display` says something is, as `table.rs` in rsui-layout calls it.
fn table_part(value: &str) -> Option<&'static str> {
    Some(match value {
        "table" | "inline-table" => "table",
        "table-row-group" | "table-header-group" | "table-footer-group" => "group",
        "table-row" => "row",
        "table-cell" => "cell",
        "table-caption" => "caption",
        "table-column" => "column",
        "table-column-group" => "columns",
        _ => return None,
    })
}

// What a browser says each element looks like by itself, which a page's own CSS goes over. It's Chromium's, as far as it goes.

const BROWSER_CSS: &str = "
h1 { font-size: 2em; margin-top: 0.67em; margin-bottom: 0.67em; font-weight: bold; }
h2 { font-size: 1.5em; margin-top: 0.83em; margin-bottom: 0.83em; font-weight: bold; }
h3 { font-size: 1.17em; margin-top: 1em; margin-bottom: 1em; font-weight: bold; }
h4 { margin-top: 1.33em; margin-bottom: 1.33em; font-weight: bold; }
h5 { font-size: 0.83em; margin-top: 1.67em; margin-bottom: 1.67em; font-weight: bold; }
h6 { font-size: 0.67em; margin-top: 2.33em; margin-bottom: 2.33em; font-weight: bold; }
p, dl { margin-top: 1em; margin-bottom: 1em; }
blockquote, figure { margin: 1em 40px; }
ul { margin-top: 1em; margin-bottom: 1em; padding-left: 40px; list-style-type: disc; }
ol { margin-top: 1em; margin-bottom: 1em; padding-left: 40px; list-style-type: decimal; }
dd { margin-left: 40px; }
pre { font-family: monospace; white-space: pre; margin-top: 1em; margin-bottom: 1em; }
code, kbd, samp { font-family: monospace; }
b, strong, th { font-weight: bold; }
i, em, cite, dfn, var, address { font-style: italic; }
u, ins { text-decoration: underline; }
s, del { text-decoration: line-through; }
small, sub, sup { font-size: smaller; }
sub { vertical-align: sub; }
sup { vertical-align: super; }
samp, var { font-family: monospace; }
var, dfn { font-style: italic; }
mark { background-color: yellow; color: black; }
abbr { text-decoration: underline dotted; }
a { color: #0000ee; text-decoration: underline; cursor: pointer; }
hr { margin-top: 0.5em; margin-bottom: 0.5em; border-top: 1px solid #9a9a9a; border-bottom: 1px solid #eeeeee; border-left: 1px solid #9a9a9a; border-right: 1px solid #eeeeee; }
table { display: table; border-collapse: separate; border-spacing: 2px; }
caption { display: table-caption; text-align: center; }
thead, tbody, tfoot { display: table-row-group; }
tr { display: table-row; }
td, th { display: table-cell; }
col { display: table-column; }
dialog { position: absolute; left: 0; right: 0; width: fit-content; height: fit-content; margin: auto; border: 3px solid black; padding: 1em; background-color: white; color: black; }
colgroup { display: table-column-group; }
th { text-align: center; padding: 1px; }
td { padding: 1px; }
input:disabled, textarea:disabled, select:disabled, button:disabled { color: rgba(16, 16, 16, 0.3); }
input:disabled, textarea:disabled { background-color: rgba(239, 239, 239, 0.3); border-color: rgba(118, 118, 118, 0.3); }
button:disabled { border-color: rgba(118, 118, 118, 0.3); -rsui-native-border: 1px; }
fieldset { -rsui-own-margins: 1; margin-left: 2px; margin-right: 2px; padding: 0.35em 0.75em 0.625em; border: 2px groove #f0f0f0; }
legend { padding-left: 2px; padding-right: 2px; }
button { box-sizing: border-box; font-family: Arial; font-size: 13.3333px; line-height: normal; color: black; text-align: center; padding: 1px 6px; border: 1px solid #767676; border-radius: 2px; background-color: #efefef; -rsui-native-border: 1px; }
select { box-sizing: border-box; }
input { -rsui-natural-width: 169px; }
input, select { font-family: Arial; font-size: 13.3333px; color: black; padding: 2px 3px; border: 1px solid #767676; border-radius: 2px; background-color: white; }
select { padding: 1px 6px 1px 4px; }
checkbox { margin: 3px 3px 3px 4px; }
radio { margin: 3px 3px 0 5px; }
summary { list-style-type: disclosure-closed; }
progress { display: inline-flex; align-items: center; width: 10em; height: 1em; }
meter { display: inline-flex; align-items: center; width: 5em; height: 1em; }
textarea { font-family: monospace; font-size: 13.3333px; color: black; padding: 2px; border: 1px solid #767676; border-radius: 2px; background-color: white; }
";

/// What the browser's sheet comes to, worked out once for every page there is, as it's the same for all of them: what each
/// selector looks like, which props it said in `em`, in the order it did, and what it said that can't be done here.
struct Browser {
    looks: Vec<(String, Props)>,
    em: String,
    said: Vec<String>,
}

static BROWSER: OnceLock<Browser> = OnceLock::new();

/// What `all` says something about: every prop the browser's sheet says for anything, then how text is written, in that order.
static ALL_KEYS: OnceLock<Vec<Key>> = OnceLock::new();

/// How text is written, which `all: unset` leaves to what it's in, apart from what the browser's sheet says already.
const TEXT_KEYS: [&str; 14] = [
    "color", "fontFamily", "fontSize", "fontWeight", "fontStyle", "lineHeight", "__lineScale", "textAlign", "letterSpacing",
    "wordSpacing", "__whiteSpace", "__case", "textIndent", "__listStyle",
];

fn all_keys(vm: &mut Vm) -> &'static [Key] {
    ALL_KEYS.get_or_init(|| {
        let mut keys: Vec<Key> = Vec::new();
        for (_, look) in &browser(vm).looks {
            for (key, _) in &look.0 {
                if !keys.contains(key) {
                    keys.push(*key);
                }
            }
        }
        for name in TEXT_KEYS {
            if !keys.contains(&Key::Plain(name)) {
                keys.push(Key::Plain(name));
            }
        }
        keys
    })
}

fn browser(vm: &mut Vm) -> &'static Browser {
    BROWSER.get_or_init(|| {
        let mut declarer = Declarer {
            vm,
            rem: 16.0,
            em: Vec::new(),
            em_changed: false,
            said: Vec::new(),
            declaring: Value::UNDEFINED,
            next_parts: 0,
        };
        let mut looks: Vec<(String, Props)> = Vec::new();
        for (selector, block) in &crate::css::read_css(BROWSER_CSS).rules {
            let mut look = Props::default();
            // Nothing in it can fail, as nothing in it asks JavaScript anything.
            let _ = declarer.declare_all(&mut look, &block.declarations, 16.0);
            for name in commas(selector) {
                match looks.iter_mut().find(|(one, _)| one == name) {
                    Some((_, existing)) => existing.merge(&look),
                    None => looks.push((name.to_owned(), look.clone())),
                }
            }
        }
        Browser {
            looks,
            em: declarer.em.join(" "),
            said: declarer.said,
        }
    })
}

// What JavaScript calls.

/// A block's declarations as `web.js` has them: `[name, value]`, or `[name, value, true]` for what's `!important`.
fn read_declarations(vm: &mut Vm, list: Value) -> Vec<(String, String, bool)> {
    let Some(items) = vm.array_elements(list) else {
        return Vec::new();
    };
    let items = items.to_vec();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Some(parts) = vm.array_elements(item) else {
            continue;
        };
        let at = |index: usize| parts.get(index).copied().unwrap_or(Value::UNDEFINED);
        let (name, value, important) = (at(0), at(1), at(2) == Value::TRUE);
        let name = text_of(vm, name).into_owned();
        let value = text_of(vm, value).into_owned();
        out.push((name, value, important));
    }
    out
}

/// `cssDeclare(declarations, fontSize, remSize, declaring, one)`: what a block's declarations come to, as props, with
/// `!important` going over the rest. With `one`, each is only what it says by itself, as a keyframe's is.
fn host_declare(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (list, font_size, rem) = (args.get(vm, 0), args.get(vm, 1), args.get(vm, 2));
    let (declaring, one) = (args.get(vm, 3), args.get(vm, 4));
    let (font_size, rem) = (number_or(font_size, 16.0), number_or(rem, 16.0));
    let declarations = read_declarations(vm, list);
    let mut props = Props::default();
    let mut declarer = Declarer::new(vm, rem, declaring)?;
    let done = if one == Value::TRUE {
        declarations.iter().try_for_each(|(name, value, _)| {
            declarer.declare_one(&mut props, name, value, font_size)
        })
    } else {
        declarer.declare_all(&mut props, &declarations, font_size)
    };
    let finished = declarer.finish();
    done?;
    finished?;
    props.to_js(vm)
}

/// `cssInline(style, fontSize, remSize, declaring)`: what a `style` object says, as React has it, in props: a number is in
/// pixels, apart from what's only ever a number.
fn host_inline(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (given, font_size, rem, declaring) = (
        args.get(vm, 0),
        args.get(vm, 1),
        args.get(vm, 2),
        args.get(vm, 3),
    );
    let (font_size, rem) = (number_or(font_size, 16.0), number_or(rem, 16.0));
    let mut said = Vec::new();
    for (name, value) in vm.properties(given)? {
        if value == Value::UNDEFINED || value.is_null() || value == Value::FALSE {
            continue;
        }
        let name = vm.atom_name(name);
        let mut css = String::with_capacity(name.len() + 4);
        for c in name.chars() {
            if c.is_ascii_uppercase() {
                css.push('-');
                css.push(c.to_ascii_lowercase());
            } else {
                css.push(c);
            }
        }
        let text = if value.is_number() {
            let number = number_text(vm, value.as_number());
            let plain = matches!(
                css.as_str(),
                "opacity"
                    | "flex"
                    | "flex-grow"
                    | "flex-shrink"
                    | "z-index"
                    | "font-weight"
                    | "line-height"
                    | "zoom"
                    | "order"
            );
            if plain { number } else { number + "px" }
        } else if value.is_string() {
            text_of(vm, value).into_owned()
        } else {
            let string = vm.get_global("String")?;
            let made = vm.call_function(string, Value::UNDEFINED, &[value])?;
            text_of(vm, made).into_owned()
        };
        said.push((css, text));
    }
    let mut props = Props::default();
    let mut declarer = Declarer::new(vm, rem, declaring)?;
    let done = said
        .iter()
        .try_for_each(|(name, value)| declarer.declare(&mut props, name, value, font_size));
    let finished = declarer.finish();
    done?;
    finished?;
    props.to_js(vm)
}

/// `cssBrowserStart(declaring)`: what the browser's sheet says in `em`, as if it had just been read, which is before any of a
/// page's CSS is.
fn host_browser_start(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let declaring = args.get(vm, 0);
    let browser = browser(vm);
    let em = vm.new_string(&browser.em);
    let name = vm.atom("em");
    vm.set_property(declaring, name, em)?;
    let name = vm.atom("warn");
    let warn = vm.get_property(declaring, name)?;
    for what in &browser.said {
        let what = vm.new_string(what);
        vm.call_function(warn, Value::UNDEFINED, &[what])?;
    }
    Ok(Value::UNDEFINED)
}

/// `cssBrowserLook(selector)`: what the browser's sheet says about this, as `h1` or `input:disabled`, or `undefined`.
fn host_browser_look(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let selector = args.get(vm, 0);
    let selector = text_of(vm, selector).into_owned();
    match browser(vm).looks.iter().find(|(one, _)| *one == selector) {
        Some((_, look)) => look.to_js(vm),
        None => Ok(Value::UNDEFINED),
    }
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 4] = [
    ("cssDeclare", host_declare),
    ("cssInline", host_inline),
    ("cssBrowserStart", host_browser_start),
    ("cssBrowserLook", host_browser_look),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The JavaScript in `web.js` that's still what `content` comes to, which the comparison passes along as `web.js` does.
    fn content_of() -> &'static str {
        let web = include_str!("web.js");
        let start = web
            .find("function contentOf(")
            .expect("web.js has contentOf");
        let end = start + web[start..].find("\n}").expect("contentOf ends") + 2;
        &web[start..end]
    }

    /// What the JavaScript this replaced gives, and what this gives, compared deeply, with the props in order, what's
    /// `undefined` and what isn't there told apart, what's said to be left out, and which props have been said in `em` after
    /// each: for the browser's own sheet, every declaration in this crate's CSS, in `web.js` and in the apps that
    /// measure it, each as a block, as a `style` object and as a keyframe, and for declarations made
    /// up at random from what CSS is made of.
    #[test]
    fn everything_comes_out_as_it_did_in_javascript() {
        let mut engine = rsui_js::Engine::new();
        let vm = engine.vm();
        let natives = vm.new_object();
        for (name, function) in crate::css::FUNCTIONS.into_iter().chain(FUNCTIONS) {
            let function = vm.host_function(name, 0, function);
            let atom = vm.atom(name);
            vm.set_property(natives, atom, function).unwrap();
        }
        vm.set_global("natives", natives);
        let corpus = [
            include_str!("web.js"),
            include_str!("declare.rs"),
            include_str!("../tests/web.rs"),
            include_str!("../tests/app.rs"),
            include_str!("../tests/fixtures/declare_reference.js"),
            include_str!("../tests/apps/web_dashboard.tsx"),
            include_str!("../tests/apps/web_css.tsx"),
        ];
        let texts = corpus.iter().map(|text| vm.new_string(text)).collect();
        let texts = vm.new_array(texts);
        vm.set_global("corpus", texts);
        // In one script, as what one script declares with `const` isn't seen by the next.
        let script = [
            include_str!("../tests/fixtures/css_reference.js"),
            include_str!("../tests/fixtures/declare_reference.js"),
            content_of(),
            COMPARE,
        ]
        .join("\n");
        let failures = engine.eval(&script, "compare").unwrap();
        let failures = engine.display(failures);
        let lines: Vec<&str> = failures.lines().collect();
        assert_eq!(lines.len(), 1, "{failures}");
        assert!(
            lines[0].starts_with(|c: char| c.is_ascii_digit()),
            "{failures}"
        );
        println!("{failures}");
    }

    const COMPARE: &str = r#"
const failures = [];
let compared = 0;
/// Everything about a value, as text: the props of an object in order, `undefined` apart from what isn't there, and `-0` and
/// `NaN` as themselves.
const show = (value) => {
    if (value === undefined) return 'undefined';
    if (value === null) return 'null';
    if (typeof value === 'number') return Object.is(value, -0) ? '-0' : String(value);
    if (typeof value === 'string') return JSON.stringify(value);
    if (Array.isArray(value)) return '[' + value.map(show).join(',') + ']';
    if (typeof value === 'object') return '{' + Object.keys(value).map((key) => JSON.stringify(key) + ':' + show(value[key])).join(',') + '}';
    return typeof value + ' ' + String(value);
};
const saidNow = [];
const declaring = { em: '', vars: withVariables, content: contentOf, warn: (what) => saidNow.push(what) };
const outcome = (run) => {
    try {
        return 'gave ' + show(run());
    } catch (error) {
        return 'threw ' + (error && error.name) + ': ' + (error && error.message);
    }
};
const compare = (what, input, old, now) => {
    leftOut = [];
    const before = outcome(old) + ' saying ' + JSON.stringify(leftOut) + ' with em ' + EM_KEYS_SEEN.join(' ');
    saidNow.length = 0;
    const after = outcome(now) + ' saying ' + JSON.stringify(saidNow) + ' with em ' + declaring.em;
    compared += 1;
    if (before !== after && failures.length < 30) {
        failures.push(what + ' ' + show(input) + '\n  was ' + before + '\n  now ' + after);
    }
};

// The browser's own sheet, first, as a page has it.
const selectors = [...new Set(readCss(BROWSER_CSS).rules.flatMap((rule) => commas(rule.selector).map((one) => one.trim())))];
compare('the browser sheet', '', () => {
    browserLook('p');
    return [...selectors, 'nothing'].map((one) => [one, browserLooks.get(one)]);
}, () => {
    natives.cssBrowserStart(declaring);
    return [...selectors, 'nothing'].map((one) => [one, natives.cssBrowserLook(one)]);
});

variables.set('--a', '10px');
variables.set('--b', '2em');
variables.set('--c', 'red');
variables.set('--d', 'var(--a) var(--c)');
variables.set('--e', '1.5');
variables.set('--g', 'linear-gradient(red, blue)');
variables.set('--self', 'var(--self)');

const asBlock = (declarations, fontSize) => {
    compare('block at ' + fontSize, declarations, () => {
        const props = {};
        declareAll(props, declarations, fontSize);
        return props;
    }, () => natives.cssDeclare(declarations, fontSize, remSize, declaring));
};
const asKeyframe = (declarations) => {
    compare('keyframe', declarations, () => {
        const said = {};
        for (const [name, value] of declarations) {
            declareOne(said, name, value, 16, {});
        }
        return said;
    }, () => natives.cssDeclare(declarations, 16, remSize, declaring, true));
};
const camel = (name) => name.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
const asStyle = (style, fontSize) => {
    compare('style at ' + fontSize, style, () => inlineLook(style, fontSize), () => natives.cssInline(style, Number(style.fontSize) || fontSize, remSize, declaring));
};
const everyWay = (declarations, round) => {
    const fontSize = [16, 13, 24, 0, -5][round % 5];
    remSize = round % 7 === 3 ? 10 : 16;
    if (round % 3 === 1) {
        globalThis.__rsuiPageSize = [1000, 700];
    }
    asBlock(declarations, fontSize);
    asKeyframe(declarations);
    const style = {};
    for (const [name, value] of declarations) {
        style[camel(name)] = /^-?[0-9.]+$/.test(value) && round % 2 === 0 ? Number(value) : value;
    }
    if (round % 4 === 0) {
        style.fontSize = [14, '20px', 'large', null][round % 3];
    }
    asStyle(style, fontSize);
    delete globalThis.__rsuiPageSize;
};

// Every block of CSS there is.
let round = 0;
const blocks = (block) => {
    if (block.declarations.length > 0) {
        everyWay(block.declarations, round++);
        for (const one of block.declarations) {
            everyWay([one], round++);
        }
    }
    for (const rule of block.rules) {
        blocks(rule.block);
    }
};
for (const text of corpus) {
    blocks(readCss(text));
}

// Made up, from what CSS is made of, the same every time.
let seed = 54321;
const random = (below) => {
    seed = (seed * 1103515245 + 12345) % 2147483648;
    return seed % below;
};
const names = [
    "display","flex-direction","order","align-items","justify-self","align-self","justify-content","justify-items","place-items","flex","flex-grow","flex-shrink","flex-basis","width","height","min-width","max-width","min-height","max-height","margin","padding","gap","grid-gap","row-gap","column-gap","grid-row-gap","grid-column-gap","position","top","left","right","bottom","inset","overflow","overflow-x","overflow-y","background","background-color","background-image","grid-template-areas","grid-area","-rsui-native-border","-rsui-natural-width","-rsui-own-margins","accent-color","background-clip","-webkit-background-clip","clip-path","filter","background-size","background-position","background-repeat","color","fill","stroke","stroke-width","stroke-linecap","stroke-linejoin","stroke-dasharray","stroke-dashoffset","opacity","border","border-top","border-right","border-bottom","border-left","border-top-width","border-right-width","border-bottom-width","border-left-width","border-top-color","border-right-color","border-bottom-color","border-left-color","border-width","border-color","border-style","outline","outline-width","outline-color","outline-offset","outline-style","border-radius","border-top-left-radius","border-top-right-radius","border-bottom-right-radius","border-bottom-left-radius","box-shadow","font-size","font-weight","font-family","font-style","font","text-align","direction","-webkit-app-region","app-region","text-decoration","text-decoration-line","text-decoration-style","text-decoration-color","text-decoration-thickness","text-underline-offset","content","line-height","list-style","list-style-type","vertical-align","-webkit-line-clamp","line-clamp","-webkit-box-orient","overflow-wrap","word-wrap","word-break","letter-spacing","word-spacing","text-indent","text-transform","text-overflow","white-space","visibility","appearance","-webkit-appearance","-moz-appearance","cursor","pointer-events","object-fit","transform","transform-origin","border-collapse","border-spacing","box-sizing","aspect-ratio","flex-wrap","flex-flow","grid-auto-flow","align-content","grid-template-columns","grid-template-rows","grid-auto-rows","grid-auto-columns","grid-column","grid-row","grid-column-start","grid-column-end","grid-row-start","grid-row-end","z-index","transition","transition-property","transition-duration","transition-delay","transition-timing-function","text-shadow","table-layout","float","clear","animation","animation-name","animation-duration","animation-delay","animation-iteration-count","animation-timing-function","animation-direction","animation-fill-mode","animation-play-state",
    "will-change","user-select","mask","shape-outside","--x","constructor","toString","__proto__","",
];
const pieces = ['0', '1', '5', '12', '.5', '-', '-2', '1.5', ' ', ' ', '  ', ',', ', ', '/', ' / ', '(', ')', '%', 'px', 'em', 'rem', 'pt', 'vw', 'vh',
    'vmin', 'vmax', 'deg', 'turn', 'rad', 'grad', 'calc(', 'min(', 'max(', 'clamp(', 'var(--a)', 'var(--b)', 'var(--c)', 'var(--d)', 'var(--e)',
    'var(--g)', 'var(--self)', 'var(--nope, 3px)', '#fff', '#a0b1c2', '#12345678', '#ABC', 'rgb(1,2,3)', 'rgba(0,0,0,.5)', 'hsl(120, 50%, 50%)',
    'red', 'Red', 'transparent', 'currentColor', 'currentcolor', 'inherit', 'unset', 'initial', 'auto', 'none', 'normal', 'bold', '700', 'italic',
    'oblique', 'solid', 'dashed', 'dotted', 'double', 'groove', 'inset', 'hidden', 'scroll', 'clip', 'overlay', 'wrap', 'nowrap', 'wrap-reverse',
    'column', 'row-reverse', 'center', 'start', 'end', 'flex-end', 'left', 'right', 'top', 'bottom', 'space-between', 'stretch', 'baseline',
    'to ', 'at ', 'circle', 'ellipse', 'closest-side', 'farthest-corner', 'linear-gradient(', 'radial-gradient(', 'repeating-linear-gradient(',
    'Linear-Gradient(', 'url(', 'URL(', '"a b"', "'x'", 'img.png', 'drop-shadow(', 'blur(', '"', "'", String.fromCharCode(92), 'e3', '1e3',
    'constructor', 'toString', '__proto__', 'hasOwnProperty', 'fit-content', 'max-content', 'content', 'span 2', '1fr', 'repeat(3, 1fr)',
    'minmax(10px, 1fr)', 'x-large', 'smaller', '80%', '50%', '100%', 'counter(item)', 'attr(title)', String.fromCharCode(92) + '201C',
    'open-quote', 'pointer', 'text', 'break-all', 'break-word', 'keep-all', 'pre', 'ltr', 'rtl', 'sub', 'super', 'middle', 'text-top',
    'ease-in', '2s', '.3s', 'border-box', 'collapse', '16/9', 'a', 'b c', 'sans-serif', 'Serif', '"Segoe UI"', 'monospace', 'underline',
    'line-through', 'wavy', 'fixed', 'sticky', 'absolute', 'relative', 'static', 'grid', 'inline-flex', 'inline-block', 'block', 'flex',
    'contain', 'cover', 'drag', 'no-drag', 'uppercase', 'ellipsis', 'repeat', 'no-repeat', 'repeat-x', 'space', 'round', 'text',
    '\t', '\n', String.fromCharCode(0xa0), String.fromCharCode(0xc9), String.fromCharCode(0xd83d, 0xde00), String.fromCharCode(0x130),
    String.fromCharCode(0x212a), 'K', 'k', 'NaN', 'Infinity', '0x10', '0b1', '+5', '00', '5.', '1.2.3'];
const value = () => {
    let text = '';
    const count = 1 + random(7);
    for (let at = 0; at < count; at += 1) {
        text += pieces[random(pieces.length)];
    }
    // Most of what's read has its brackets closed.
    if (random(3) > 0) {
        let open = 0;
        for (const c of text) {
            open += c === '(' ? 1 : c === ')' ? -1 : 0;
        }
        text += ')'.repeat(Math.max(0, open));
    }
    return random(4) === 0 ? text : text.trim();
};
for (let made = 0; made < 12000; made += 1) {
    const declarations = [];
    const count = 1 + random(4);
    for (let at = 0; at < count; at += 1) {
        const one = [names[random(names.length)], value()];
        if (random(5) === 0) {
            one.push(true);
        }
        declarations.push(one);
    }
    everyWay(declarations, round++);
}
// What's not a string, in a `style` object.
asStyle({ width: true, height: 0, opacity: 0, margin: 4, zIndex: 3, padding: { toString: () => '5px' }, flex: 2, order: -0, color: false, top: null }, 16);
failures.push(compared + ' compared');
failures.join('\n')
"#;
}
