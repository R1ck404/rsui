//! What `rsui/web` does to each element's look once CSS has said everything about it: where it sits in what it's in, what
//! only means something once everything's been said (`settle` in `web.js`), and what its font comes to. It's what `web.js` did
//! itself for every element it draws, done here, as one call for each.
//!
//! Each gives exactly what the JavaScript it replaces gave: the same props in the same order, the same ones left `undefined`
//! and the same ones left out, with numbers and text as JavaScript makes them. A look comes in as the object JavaScript has,
//! is read once into a list kept in the order a JavaScript object keeps its props, and goes back as a new object, as there's
//! nothing here that takes a prop off an object. Making an object here costs more than JavaScript's `{ ...look }` does, so
//! everything about one element is done in one call, which makes one object.
//!
//! The names of props are found once for each engine, when `web.js` starts, and it passes back the number it's given for
//! them each time.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;

use rsui_js::{Args, Atom, HostFunction, Root, Value, Vm};

use crate::css::{number_text, parse_float};

type Run<T> = Result<T, Value>;

macro_rules! names {
    ($($field:ident = $text:literal),* $(,)?) => {
        /// The props this reads and writes, by name.
        #[derive(Clone, Copy)]
        struct Names {
            $($field: Atom,)*
        }

        impl Names {
            fn new(vm: &mut Vm) -> Self {
                Self { $($field: vm.atom($text),)* }
            }
        }
    };
}

names! {
    inside = "inside",
    display = "display",
    direction = "direction",
    align = "align",
    flex_wrap = "flexWrap",
    line_strut = "lineStrut",
    justify = "justify",
    text_align = "textAlign",
    white_space = "__whiteSpace",
    hidden = "__hidden",
    scroll = "__scroll",
    case = "__case",
    block = "__block",
    flex = "__flex",
    inline = "__inline",
    position = "position",
    on_line = "onLine",
    width = "width",
    height = "height",
    min_width = "minWidth",
    max_width = "maxWidth",
    min_height = "minHeight",
    max_height = "maxHeight",
    fit_width = "fitWidth",
    align_self = "alignSelf",
    line_align = "__lineAlign",
    line_box = "__lineBox",
    padding_top = "paddingTop",
    padding_right = "paddingRight",
    padding_bottom = "paddingBottom",
    padding_left = "paddingLeft",
    border_width = "borderWidth",
    border_top_width = "borderTopWidth",
    border_right_width = "borderRightWidth",
    border_bottom_width = "borderBottomWidth",
    border_left_width = "borderLeftWidth",
    margin_top = "marginTop",
    margin_bottom = "marginBottom",
    margin_left = "marginLeft",
    margin_right = "marginRight",
    margin_left_share = "marginLeftShare",
    margin_right_share = "marginRightShare",
    shrink = "shrink",
    tabular_nums = "tabularNums",
    text_wrap = "textWrap",
    font_metrics = "fontMetrics",
    grow = "grow",
    basis = "basis",
    min_auto = "minAuto",
    min_width_said = "minWidthSaid",
    min_height_said = "minHeightSaid",
    auto_top = "__autoTop",
    auto_right = "__autoRight",
    auto_bottom = "__autoBottom",
    auto_left = "__autoLeft",
    fill_width = "__fillWidth",
    margin_left_auto = "marginLeftAuto",
    margin_right_auto = "marginRightAuto",
    margin_top_auto = "marginTopAuto",
    margin_bottom_auto = "marginBottomAuto",
    background = "background",
    border_color = "borderColor",
    border_top_color = "borderTopColor",
    border_right_color = "borderRightColor",
    border_bottom_color = "borderBottomColor",
    border_left_color = "borderLeftColor",
    shadow_color = "shadowColor",
    outline_color = "outlineColor",
    fill = "fill",
    stroke = "stroke",
    hover = "hover",
    pressed = "pressed",
    transition = "__transition",
    transition_parts = "__transitionParts",
    transitions = "transitions",
    animation_said = "__animation",
    animation_parts = "__animationParts",
    animation = "animation",
    font_scale = "__fontScale",
    font_size = "fontSize",
    flow = "__flow",
    align_content_said = "__alignContent",
    sticky = "__sticky",
    top = "top",
    bottom = "bottom",
    left = "left",
    right = "right",
    sticky_top = "stickyTop",
    sticky_bottom = "stickyBottom",
    visibility = "__visibility",
    opacity = "opacity",
    inert = "inert",
    native_border = "__nativeBorder",
    own_border = "__ownBorder",
    box_sizing = "__boxSizing",
    collapse_margins = "collapseMargins",
    keep_margins = "keepMargins",
    own_margins = "__ownMargins",
    gap = "gap",
    column_gap = "columnGap",
    row_gap = "rowGap",
    list_style = "__listStyle",
    fixed = "__fixed",
    overlay = "overlay",
    aspect = "__aspect",
    shape = "aspect",
    grid_flow = "gridFlow",
    columns = "columns",
    rows = "rows",
    justify_items = "justifyItems",
    align_content = "alignContent",
    line_scale = "__lineScale",
    parts = "__parts",
    own_width = "__ownWidth",
    own_height = "__ownHeight",
    picture_aspect = "aspect",
    text_direction = "textDirection",
    reverse = "reverse",
    size_said = "__sizeSaid",
    no_wrap = "noWrap",
    keep_spaces = "keepSpaces",
    line_height = "lineHeight",
    font_family = "fontFamily",
    font_weight = "fontWeight",
    font_style = "fontStyle",
    color = "color",
    keep_words = "keepWords",
    letter_spacing = "letterSpacing",
    word_spacing = "wordSpacing",
    text_shadow = "textShadow",
    text_indent = "textIndent",
    font_key = "fontKey",
    ty = "type",
    props = "props",
    kids = "kids",
    style = "style",
    class_name = "className",
    float = "float",
    clear = "clear",
    order = "order",
    web_tag = "webTag",
    children = "children",
    web_tag_said = "__webTag",
    web_inline = "__webInline",
    web_inline_of = "__webInlineOf",
    web_look = "__webLook",
    float_said = "__float",
    clear_said = "__clear",
    order_said = "__order",
    web_said = "__webSaid",
    web_order = "__webOrder",
    context = "context",
}

macro_rules! words {
    ($($field:ident = $text:literal),* $(,)?) => {
        /// Text this puts on props, made once for each engine, as the same string is as good as a new one.
        #[derive(Clone, Copy)]
        struct Words {
            $($field: Value,)*
        }

        impl Words {
            fn new(vm: &mut Vm, held: &mut Vec<Root>) -> Self {
                Self {
                    $($field: {
                        let made = vm.new_string($text);
                        held.push(vm.root(made));
                        made
                    },)*
                }
            }
        }
    };
}

words! {
    row = "row",
    line = "line",
    column = "column",
    block = "block",
    baseline = "baseline",
    center = "center",
    start = "start",
    end = "end",
    hundred = "100%",
    fill = "fill",
    inset = "inset",
    space_around = "space_around",
    right = "right",
    left = "left",
    bold = "bold",
    normal = "normal",
    square = "square",
    circle = "circle",
    inline = "inline",
    other = "other",
    inline_start = "inline-start",
    inline_end = "inline-end",
}

/// A font file that `@font-face` gave, as `web.js` keeps them, or a font the system has that it named with `local()`.
struct FontFile {
    weight: f64,
    italic: bool,
    /// The file, or the family of the system's font.
    path: Value,
    /// How much bigger than it says its text is drawn, as `size-adjust` says.
    adjust: f64,
    /// For a system font, the weight of the one it named, and whether that's italic, as `local('Segoe UI Semibold')` is 600.
    local: Option<(f64, bool)>,
    /// How far the font goes above and below the baseline, as shares of its size, where `ascent-override` and `descent-override`
    /// say, as Radix's "Segoe UI (Custom)" does: text sits that much lower or higher in its lines.
    metrics: Option<(f64, f64)>,
}

/// Fonts only a Mac has, which a family that names them first goes past, to what's after them, as a browser here does.
/// How big rsui's own text is when nothing says, as `DEFAULT_SIZE` in rsui-text has it.
const NATIVE_TEXT_SIZE: f64 = 14.0;

const MAC_ONLY: [&str; 2] = ["-apple-system", "blinkmacsystemfont"];

/// What's kept for each engine that's loaded `rsui/web`.
struct Layout {
    names: Names,
    words: Words,
    /// What the strings in `words` are kept alive by.
    _held: Vec<Root>,
    /// The font files for each family, in the order they were given.
    fonts: Vec<(String, Vec<FontFile>)>,
    /// For each name of a prop that's been seen, the prop it's a share of the size of the text for, if it's one of those, as
    /// `__em_paddingTop` is for `paddingTop`.
    em: HashMap<Atom, Option<Atom>>,
    /// The engine's [`Vm::atom_generation`] when `em` was started, as an atom that's been freed can have its number
    /// given to another name.
    em_generation: u32,
    /// Prefix classification only; keys and values are still read from the current look on every call.
    variable_names: HashMap<Atom, Option<String>>,
    variable_generation: u32,
    variable_fields: Vec<(Atom, Value)>,
    /// What's needed to work out what's in an element, once `web.js` has said it.
    kinds: Option<Kinds>,
    /// What CSS says about each kind of element, each set of classes, and each class's `order`, as far as where things go,
    /// until `web.js` forgets what it's worked out, as it does when CSS changes.
    tag_said: HashMap<String, Option<Said>>,
    classes_said: HashMap<String, Said>,
    class_orders: HashMap<String, Option<f64>>,
}

thread_local! {
    static LAYOUTS: RefCell<Vec<Layout>> = const { RefCell::new(Vec::new()) };
}

/// How much is kept here for each engine that's loaded `rsui/web` on this thread, added up, by name.
pub(crate) fn table_sizes() -> Vec<(&'static str, usize)> {
    LAYOUTS.with(|layouts| {
        let layouts = layouts.borrow();
        let sum = |size: fn(&Layout) -> usize| layouts.iter().map(size).sum();
        vec![
            ("web layouts", layouts.len()),
            ("web held", sum(|layout| layout._held.len())),
            ("web tags said", sum(|layout| layout.tag_said.len())),
            ("web classes said", sum(|layout| layout.classes_said.len())),
            ("web class orders", sum(|layout| layout.class_orders.len())),
            ("web em props", sum(|layout| layout.em.len())),
        ]
    })
}

/// Runs something with what's kept for the engine that gave this number.
fn with_layout<R>(
    vm: &mut Vm,
    token: Value,
    run: impl FnOnce(&mut Vm, &mut Layout) -> Run<R>,
) -> Run<R> {
    let at = token.as_number() as usize;
    let _owner = rsui::alloc_owners::scope(rsui::alloc_owners::WEB);
    LAYOUTS.with(|layouts| {
        // JavaScript that's asked something here could ask something more of it, which it isn't ready for.
        let Ok(mut layouts) = layouts.try_borrow_mut() else {
            return Err(vm.type_error("rsui/web's layout is busy"));
        };
        match layouts.get_mut(at) {
            Some(layout) if token.is_number() => run(vm, layout),
            _ => Err(vm.type_error("rsui/web's layout hasn't been started")),
        }
    })
}

// What JavaScript does to values, as this needs it.

fn is_text(vm: &Vm, value: Value, text: &str) -> bool {
    value.is_string() && vm.string_text(value).is_some_and(|own| own == text)
}

fn text_of(vm: &Vm, value: Value) -> Option<Cow<'_, str>> {
    vm.string_text(value)
}

/// Whether `if (value)` goes ahead.
fn truthy(vm: &Vm, value: Value) -> bool {
    if value.is_bool() {
        return value == Value::TRUE;
    }
    if value.is_nullish() {
        return false;
    }
    if value.is_number() {
        let number = value.as_number();
        return number != 0.0 && !number.is_nan();
    }
    if value.is_string() {
        return vm.string_text(value).is_some_and(|text| !text.is_empty());
    }
    true
}

/// `String(value)`.
fn string_of(vm: &mut Vm, value: Value) -> Run<String> {
    if let Some(text) = vm.plain_text(value) {
        return Ok(text);
    }
    if value.is_undefined() {
        return Ok("undefined".to_owned());
    }
    if value.is_null() {
        return Ok("null".to_owned());
    }
    if value.is_bool() {
        return Ok(if value == Value::TRUE {
            "true"
        } else {
            "false"
        }
        .to_owned());
    }
    let string = vm.get_global("String")?;
    let made = vm.call_function(string, Value::UNDEFINED, &[value])?;
    Ok(vm
        .string_text(made)
        .map(Cow::into_owned)
        .unwrap_or_default())
}

/// `+` on numbers that aren't text, as JavaScript makes them numbers.
fn number_of(value: Value) -> f64 {
    if value.is_number() {
        value.as_number()
    } else if value == Value::TRUE {
        1.0
    } else if value == Value::FALSE || value.is_null() {
        0.0
    } else {
        f64::NAN
    }
}

/// `a + b`.
fn add(vm: &mut Vm, a: Value, b: Value) -> Run<Value> {
    if a.is_number() && b.is_number() {
        return Ok(Value::number(a.as_number() + b.as_number()));
    }
    // An object is what `String` makes of it, which is what `+` comes to for any object a look holds.
    if a.is_string() || b.is_string() || a.is_object() || b.is_object() {
        let mut text = string_of(vm, a)?;
        text.push_str(&string_of(vm, b)?);
        return Ok(vm.new_string(&text));
    }
    Ok(Value::number(number_of(a) + number_of(b)))
}

/// `Number(value)`.
fn to_number(vm: &mut Vm, value: Value) -> Run<f64> {
    if value.is_string() {
        let text = text_of(vm, value).unwrap_or_default();
        return Ok(crate::declare::to_number(&text));
    }
    if value.is_object() {
        let text = string_of(vm, value)?;
        return Ok(crate::declare::to_number(&text));
    }
    Ok(number_of(value))
}

/// A look as a JavaScript object has it: its props in the order they were first set, as what's set again stays where it
/// was, and what's taken off and set again goes at the end.
#[derive(Default)]
struct Look(Vec<(Atom, Value)>);

impl Look {
    fn read(vm: &mut Vm, object: Value) -> Run<Look> {
        let mut list = Vec::with_capacity(40);
        vm.properties_into(object, &mut list)?;
        Ok(Look(list))
    }

    fn at(&self, key: Atom) -> Option<usize> {
        self.0.iter().position(|(one, _)| *one == key)
    }

    /// `look[key]`, which is `undefined` when it isn't there.
    fn get(&self, key: Atom) -> Value {
        self.0
            .iter()
            .find(|(one, _)| *one == key)
            .map_or(Value::UNDEFINED, |(_, value)| *value)
    }

    fn set(&mut self, key: Atom, value: Value) {
        match self.at(key) {
            Some(at) => self.0[at].1 = value,
            None => self.0.push((key, value)),
        }
    }

    fn delete(&mut self, key: Atom) {
        if let Some(at) = self.at(key) {
            self.0.remove(at);
        }
    }

    /// `look[key] ??= value`.
    fn set_if_nullish(&mut self, key: Atom, value: Value) {
        if self.get(key).is_nullish() {
            self.set(key, value);
        }
    }

    fn is(&self, vm: &Vm, key: Atom, text: &str) -> bool {
        is_text(vm, self.get(key), text)
    }

    fn is_true(&self, key: Atom) -> bool {
        self.get(key) == Value::TRUE
    }

    fn is_undefined(&self, key: Atom) -> bool {
        self.get(key).is_undefined()
    }

    fn number(&self, key: Atom) -> Option<f64> {
        let value = self.get(key);
        value.is_number().then(|| value.as_number())
    }

    /// `{ ...look, ...more }`.
    fn merge(&mut self, more: &[(Atom, Value)]) {
        for (key, value) in more {
            self.set(*key, *value);
        }
    }

    fn to_js(&self, vm: &mut Vm) -> Run<Value> {
        let out = vm.new_object();
        for (key, value) in &self.0 {
            vm.set_property(out, *key, *value)?;
        }
        Ok(out)
    }
}

/// What goes along a line of text with it, rather than starting a line of its own.
pub(crate) fn inline_text_tag(tag: &str) -> bool {
    matches!(
        tag,
        "span"
            | "a"
            | "b"
            | "i"
            | "em"
            | "strong"
            | "small"
            | "code"
            | "u"
            | "s"
            | "sub"
            | "sup"
            | "mark"
            | "abbr"
            | "cite"
            | "kbd"
            | "time"
            | "label"
            | "samp"
            | "var"
            | "dfn"
            | "ins"
            | "del"
            | "data"
            | "output"
            | "bdi"
            | "bdo"
    )
}

/// What sits on a line by itself in a page, as big as what's in it, rather than as wide as what it's in.
pub(crate) fn inline_box(tag: &str) -> bool {
    matches!(
        tag,
        "button" | "input" | "select" | "textarea" | "img" | "svg" | "video" | "canvas" | "iframe" | "progress" | "meter"
    )
}

/// What `align-content` comes to as `align`, for something that goes along a row.
fn align_word(word: &str) -> Option<&'static str> {
    Some(match word {
        "flex-start" | "start" | "self-start" | "left" | "top" | "baseline" => "start",
        "center" => "center",
        "flex-end" | "end" | "self-end" | "right" | "bottom" => "end",
        "stretch" | "normal" => "stretch",
        _ => return None,
    })
}

/// What `align-content` comes to as `justify`, for a grid that's only one column.
fn justify_word(word: &str) -> Option<&'static str> {
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

/// What's being worked out, with what it's worked out with.
struct Cx<'v, 'l> {
    vm: &'v mut Vm,
    n: Names,
    w: Words,
    layout: &'l mut Layout,
}

impl Cx<'_, '_> {
    fn word(&mut self, text: &str) -> Value {
        self.vm.new_string(text)
    }

    /// The prop this name is a share of the size of the text for, if it's `__em_` and one, as `resolveRelative` finds them.
    fn em_base(&mut self, key: Atom) -> Option<Atom> {
        if self.layout.em_generation != self.vm.atom_generation() {
            self.layout.em_generation = self.vm.atom_generation();
            self.layout.em.clear();
        }
        if let Some(found) = self.layout.em.get(&key) {
            return *found;
        }
        let name = self.vm.atom_name(key);
        let base = name.strip_prefix("__em_").map(|rest| self.vm.atom(rest));
        self.layout.em.insert(key, base);
        base
    }

    /// `resolveRelative`: what was said as a share of the size of the text, now that it's known, with `parent` the size of the
    /// text around it.
    fn resolve_relative(&mut self, look: &mut Look, parent: f64) {
        let n = self.n;
        if let Some(scale) = look.number(n.font_scale) {
            look.set(n.font_size, Value::number(scale * parent));
        }
        look.delete(n.font_scale);
        let own = look.number(n.font_size).unwrap_or(parent);
        let keys: Vec<Atom> = look.0.iter().map(|(key, _)| *key).collect();
        for key in keys {
            if let Some(base) = self.em_base(key) {
                let share = look.get(key);
                if share.is_number() {
                    look.set(base, Value::number(share.as_number() * own));
                }
                look.delete(key);
            }
        }
    }

    /// `withCurrentColor`: the color of the text for each color that says it's that.
    fn with_current_color(&self, look: &mut Look, current: Value) {
        let n = self.n;
        for key in [
            n.background,
            n.border_color,
            n.border_top_color,
            n.border_right_color,
            n.border_bottom_color,
            n.border_left_color,
            n.shadow_color,
            n.outline_color,
            n.fill,
            n.stroke,
            n.hover,
            n.pressed,
        ] {
            if look.is(self.vm, key, "currentcolor") {
                if current.is_undefined() {
                    look.delete(key);
                } else {
                    look.set(key, current);
                }
            }
        }
    }

    /// `sitsOnLine`: a picture sits on a line of text, unless it's been made a block or a box of its own.
    fn sits_on_line(&self, look: &mut Look) {
        let n = self.n;
        if !look.is_true(n.block)
            && !look.is_true(n.flex)
            && look.is_undefined(n.display)
            && look.is_undefined(n.direction)
            && !look.is(self.vm, n.position, "absolute")
        {
            look.set(n.on_line, Value::TRUE);
        }
    }

    /// `alongLine`: where what's on a line goes along it, as `text-align` says. Where text goes from right to left (`backwards`),
    /// a line starts at the right, and isn't turned around, so its start and end are its right and left.
    fn along_line(&self, text_align: Value, backwards: bool) -> Value {
        if is_text(self.vm, text_align, "center") {
            self.w.center
        } else if is_text(self.vm, text_align, "right")
            || (is_text(self.vm, text_align, if backwards { "start" } else { "end" }))
            || (backwards && !is_text(self.vm, text_align, "left") && !is_text(self.vm, text_align, "end"))
        {
            self.w.end
        } else {
            Value::UNDEFINED
        }
    }

    /// What's in a block, a column, a flex row or a grid where text goes from right to left is laid out from the left and turned
    /// around (`reverse` and `cross_reverse` in rsui-layout), which would turn its own margins around with it: they're swapped
    /// here, so that once it's turned around each is on the side it says, as CSS's margins are physical (`margin-left` is at the
    /// left in `dir="rtl"` too, and `margin-inline-start` is at the right, as the web code has said already). What's placed by
    /// itself isn't turned around.
    #[inline(never)]
    fn mirrored_margins(&self, look: &mut Look) {
        let n = self.n;
        if look.is(self.vm, n.position, "absolute") || look.is(self.vm, n.position, "fixed") || look.is_true(n.fixed) {
            return;
        }
        for (one, other) in [
            (n.margin_left, n.margin_right),
            (n.auto_left, n.auto_right),
            (n.margin_left_share, n.margin_right_share),
        ] {
            let (a, b) = (look.get(one), look.get(other));
            if a.is_undefined() && b.is_undefined() {
                continue;
            }
            if b.is_undefined() {
                look.delete(one);
            } else {
                look.set(one, b);
            }
            if a.is_undefined() {
                look.delete(other);
            } else {
                look.set(other, a);
            }
        }
    }

    /// `fitInFlow`: what goes on a line is as wide as it is, in a block.
    /// Gives back whether it's inline, as what says so is taken off `look` here.
    fn fit_in_flow(&self, look: &mut Look, tag: &str, flow: &str, around: Value) -> bool {
        let n = self.n;
        let said = look.get(n.inline);
        let inline = if said.is_nullish() {
            inline_box(tag)
                || (inline_text_tag(tag)
                    && !look.is_true(n.block)
                    && !look.is(self.vm, n.position, "absolute"))
        } else {
            truthy(self.vm, said)
        };
        look.delete(n.inline);
        if inline && flow == "block" && look.is_undefined(n.width) && look.is_undefined(n.display) {
            look.set(n.fit_width, Value::TRUE);
            // Which side `right` and `left` are depends on which way the text goes: a block where it goes from right to left
            // is laid out from the left and turned around, so its start is at the right.
            let backwards = look.is(self.vm, n.text_direction, "rtl");
            let side = if is_text(self.vm, around, "center") {
                self.w.center
            } else if is_text(self.vm, around, "end")
                || is_text(self.vm, around, if backwards { "left" } else { "right" })
            {
                self.w.end
            } else {
                self.w.start
            };
            look.set_if_nullish(n.align_self, side);
        }
        inline
    }

    /// `autoMargins`: margins that are `auto` put something at an end or in the middle of what it's in, across it, and
    /// take what's left over along it.
    fn auto_margins(&self, look: &mut Look, flow: &str) {
        let n = self.n;
        let [top, right, bottom, left] =
            [n.auto_top, n.auto_right, n.auto_bottom, n.auto_left].map(|key| look.is_true(key));
        for key in [n.auto_top, n.auto_right, n.auto_bottom, n.auto_left] {
            look.delete(key);
        }
        if look.is(self.vm, n.position, "absolute") || look.is(self.vm, n.position, "fixed") {
            // What's placed by itself has its margins as they are, which put it in the middle of where it's placed: see
            // `solve_absolutes` in rsui-layout.
            for (key, on) in [
                (n.margin_top_auto, top),
                (n.margin_right_auto, right),
                (n.margin_bottom_auto, bottom),
                (n.margin_left_auto, left),
            ] {
                if on {
                    look.set(key, Value::TRUE);
                }
            }
            return;
        }
        let across = if flow == "row" {
            [top, bottom]
        } else {
            [left, right]
        };
        let along = if flow == "row" {
            [(n.margin_left_auto, left), (n.margin_right_auto, right)]
        } else {
            [(n.margin_top_auto, top), (n.margin_bottom_auto, bottom)]
        };
        if across[0] || across[1] {
            let side = if across[0] && across[1] {
                self.w.center
            } else if across[0] {
                self.w.end
            } else {
                self.w.start
            };
            look.set_if_nullish(n.align_self, side);
            // As wide as what's in it, as `fit-content` says, it's in the middle, and otherwise as wide as there's room for.
            if flow == "block" && look.is_undefined(n.width) && !look.is_true(n.fit_width) {
                look.set(n.fill_width, Value::TRUE);
            } else if flow != "row" {
                let fits = Value::bool(look.is_undefined(n.width));
                look.set_if_nullish(n.fit_width, fits);
            }
        }
        if flow != "block" {
            for (key, on) in along {
                if on {
                    look.set(key, Value::TRUE);
                }
            }
        }
    }

    /// `withMore`: a size with this many more pixels, whether it's in pixels, a percentage, or both.
    fn with_more(&mut self, size: Value, more: Value) -> Run<Value> {
        if (more.is_number() && more.as_number() == 0.0) || size.is_undefined() {
            return Ok(size);
        }
        if size.is_number() {
            return add(self.vm, size, more);
        }
        if !size.is_string() {
            return Ok(size);
        }
        let text = text_of(self.vm, size).unwrap_or_default().into_owned();
        if let Some((share, pixels)) = share_and_pixels(&text) {
            let pixels = Value::number(number_text_value(pixels));
            let sum = add(self.vm, pixels, more)?;
            let sum = string_of(self.vm, sum)?;
            return Ok(self.vm.new_string(&format!("{share}% + {sum}px")));
        }
        if text.ends_with('%') {
            let share = number_text(self.vm, parse_float(&text));
            let more = string_of(self.vm, more)?;
            return Ok(self.vm.new_string(&format!("{share}% + {more}px")));
        }
        Ok(size)
    }

    /// `settle`: what only means something when everything's been said, which is just before it goes on an element.
    /// `motion` is what `motionProps` put on it, worked out by `web.js`, and `em` whether anything's ever been said in `em`.
    fn settle(&mut self, look: &mut Look, motion: [Value; 2], em: bool) -> Run<()> {
        let n = self.n;
        // What `motionProps` does, with what it worked out.
        if !motion[0].is_undefined() {
            look.set(n.transitions, motion[0]);
        }
        look.delete(n.transition);
        look.delete(n.transition_parts);
        if !motion[1].is_undefined() {
            look.set(n.animation, motion[1]);
        }
        look.delete(n.animation_said);
        look.delete(n.animation_parts);
        if !look.is_undefined(n.font_scale) || em {
            // Anything that's still a share of the size of its text has nothing better than the usual size to go by.
            self.resolve_relative(look, 16.0);
        }
        let flow = look.get(n.flow);
        let content = look.get(n.align_content_said);
        look.delete(n.flow);
        look.delete(n.align_content_said);
        if look.is_true(n.sticky) {
            if let Some(top) = look.number(n.top) {
                look.set(n.sticky_top, Value::number(top));
            }
            if let Some(bottom) = look.number(n.bottom) {
                look.set(n.sticky_bottom, Value::number(bottom));
            }
            for key in [n.top, n.bottom, n.left, n.right] {
                look.delete(key);
            }
        }
        look.delete(n.sticky);
        if !look.is_undefined(n.visibility) {
            if look.is(self.vm, n.visibility, "hidden") {
                look.set(n.opacity, Value::int(0));
                look.set(n.inert, Value::TRUE);
            }
            look.delete(n.visibility);
        }
        let room = match look.number(n.native_border) {
            Some(room) if !look.is_true(n.own_border) => Value::number(room),
            _ => Value::int(0),
        };
        look.delete(n.native_border);
        look.delete(n.own_border);
        for (border, padding) in [
            (n.border_top_width, n.padding_top),
            (n.border_right_width, n.padding_right),
            (n.border_bottom_width, n.padding_bottom),
            (n.border_left_width, n.padding_left),
        ] {
            let mut thick = look.get(border);
            if thick.is_nullish() {
                thick = look.get(n.border_width);
            }
            if thick.is_number() && thick.as_number() > 0.0 {
                let mut was = look.get(padding);
                if was.is_nullish() {
                    was = Value::int(0);
                }
                let sum = add(self.vm, was, thick)?;
                let sum = add(self.vm, sum, room)?;
                look.set(padding, sum);
            }
        }
        // A size is what's inside the padding and border, as CSS has it, unless it says it's the whole thing.
        if !look.is(self.vm, n.box_sizing, "border-box") {
            let or_zero = |value: Value| {
                if value.is_nullish() {
                    Value::int(0)
                } else {
                    value
                }
            };
            let across = add(
                self.vm,
                or_zero(look.get(n.padding_left)),
                or_zero(look.get(n.padding_right)),
            )?;
            let down = add(
                self.vm,
                or_zero(look.get(n.padding_top)),
                or_zero(look.get(n.padding_bottom)),
            )?;
            for (key, more) in [
                (n.width, across),
                (n.min_width, across),
                (n.max_width, across),
                (n.height, down),
                (n.min_height, down),
                (n.max_height, down),
            ] {
                let size = self.with_more(look.get(key), more)?;
                look.set(key, size);
            }
        }
        look.delete(n.box_sizing);
        if look.is_true(n.fill_width) {
            look.set(n.width, self.w.hundred);
        }
        look.delete(n.fill_width);
        if look.is_undefined(n.shrink) {
            look.set(n.shrink, Value::int(1));
            look.set(n.min_auto, Value::TRUE);
        } else if look.is(self.vm, n.basis, "0%") && look.is_undefined(n.min_width) && look.is_undefined(n.min_height) {
            // `flex: 1` starts from nothing, and is still no smaller than what's in it, as CSS's `min-width: auto` has it, as
            // Mantine's segmented control's labels are each as wide as they are.
            look.set(n.min_auto, Value::TRUE);
        }
        // What says how small it can be is that small along a flex box, rather than as small as what's in it, as `min-w-0` on a
        // pane with `flex-grow: 1; flex-basis: 0` beside another is half the room, however wide its toolbar.
        if look.is_true(n.min_auto) {
            if !look.is_undefined(n.min_width) {
                look.set(n.min_width_said, Value::TRUE);
            }
            if !look.is_undefined(n.min_height) {
                look.set(n.min_height_said, Value::TRUE);
            }
        }
        if look.is_undefined(n.direction) && look.is_undefined(n.display) && !look.is_true(n.flex) {
            look.set(n.collapse_margins, Value::TRUE);
            if look.is_true(n.own_margins) {
                look.set(n.keep_margins, Value::TRUE);
            }
            look.delete(n.gap);
        } else if !look.is(self.vm, n.display, "grid") {
            let along = if look.is(self.vm, n.direction, "row") {
                look.get(n.column_gap)
            } else {
                look.get(n.row_gap)
            };
            if !along.is_undefined() {
                look.set(n.gap, along);
            }
        }
        look.delete(n.own_margins);
        // A flex box's row starts on the right in right-to-left text, as one in `dir="rtl"` does: it's laid out from the left and
        // turned around, as `row-reverse` is, which goes the other way again there. Shadcn's RTL examples' buttons, menu items
        // with their shortcuts, and where a popover then has room, all follow from it.
        if look.is_true(n.flex)
            && look.is(self.vm, n.direction, "row")
            && look.is(self.vm, n.text_direction, "rtl")
        {
            let reversed = look.is_true(n.reverse);
            look.set(n.reverse, Value::bool(!reversed));
        }
        look.delete(n.flex);
        look.delete(n.block);
        look.delete(n.list_style);
        if look.is_true(n.auto_left) && look.is_true(n.auto_right) {
            look.set_if_nullish(n.align_self, self.w.center);
        }
        for key in [n.auto_top, n.auto_right, n.auto_bottom, n.auto_left] {
            look.delete(key);
        }
        if look.is_true(n.fixed) {
            self.fixed(look)?;
        }
        look.delete(n.fixed);
        if let Some(aspect) = look.number(n.aspect) {
            if let (Some(width), true) = (look.number(n.width), look.is_undefined(n.height)) {
                look.set(n.height, Value::number(width / aspect));
            } else if let (Some(height), true) = (look.number(n.height), look.is_undefined(n.width))
            {
                look.set(n.width, Value::number(height * aspect));
            } else if look.is_undefined(n.height) || look.is_undefined(n.width) {
                // A size that's only known once it's laid out, as a block's width is, gives the other then.
                look.set(n.shape, Value::number(aspect));
            }
        }
        look.delete(n.aspect);
        let grid = look.is(self.vm, n.display, "grid");
        if grid && !flow.is_undefined() {
            if !flow.is_string() {
                return Err(self.vm.type_error("flow.startsWith is not a function"));
            }
            if text_of(self.vm, flow).is_some_and(|flow| flow.starts_with("column")) {
                look.set(n.grid_flow, self.w.column);
            }
        }
        let content_text = if content.is_undefined() {
            None
        } else {
            Some(string_of(self.vm, content)?)
        };
        if grid
            && look.is_undefined(n.grid_flow)
            && look.is_undefined(n.columns)
            && look.is_undefined(n.rows)
        {
            look.delete(n.display);
            look.set(n.direction, self.w.column);
            let row_gap = look.get(n.row_gap);
            if !row_gap.is_undefined() {
                look.set(n.gap, row_gap);
            }
            let middled = look.is(self.vm, n.align, "center") && content.is_undefined();
            let items = look.get(n.justify_items);
            // Its one column is only as wide as what's in it when `justify-content` says where it goes, as Radix's Callout's
            // `flex-start` does, and that's where what's in it is.
            let across = look.get(n.justify);
            let placed = ["start", "center", "end", "space_between", "space_around", "space_evenly"]
                .into_iter()
                .find(|word| is_text(self.vm, across, word))
                .map(|word| match word {
                    "space_between" => "start",
                    "space_around" | "space_evenly" => "center",
                    other => other,
                });
            if !items.is_undefined() {
                look.set(n.align, items);
            } else if let Some(word) = placed {
                let word = self.word(word);
                look.set(n.align, word);
            } else if !look.is_undefined(n.align) {
                look.delete(n.align);
            }
            if let Some(content) = &content_text {
                let word = justify_word(content).unwrap_or("start");
                let word = self.word(word);
                look.set(n.justify, word);
            } else if middled {
                look.set(n.justify, self.w.space_around);
            } else {
                look.delete(n.justify);
            }
            look.delete(n.justify_items);
        } else if grid && !content.is_undefined() {
            look.set(n.align_content, content);
        } else if !grid && look.is_true(n.flex_wrap) && content_text.is_some() {
            // Where the lines of a flex box that wraps go, which rsui-layout spreads as CSS does: `flex-start` keeps them at the
            // top rather than stretching each to share what's left.
            look.set(n.align_content, content);
        } else if let Some(content) = &content_text {
            if !grid && look.is(self.vm, n.direction, "row") && look.is_undefined(n.align) {
                let word = align_word(content).unwrap_or("stretch");
                let word = self.word(word);
                look.set(n.align, word);
            }
        }
        Ok(())
    }

    /// What `position: fixed` comes to, as `settle` has it: on top of everything else, placed in the window by its own `top`,
    /// `right`, `bottom` and `left`, which rsui-layout reads as CSS does for what's fixed (`Placement::Inset`): both sides of a way
    /// with no size stretch it between them, a share is of the window, and `auto` margins put it in the middle.
    fn fixed(&mut self, look: &mut Look) -> Run<()> {
        let n = self.n;
        let [top, right, bottom, left] =
            [n.top, n.right, n.bottom, n.left].map(|key| look.get(key));
        let zero = |value: Value| value.is_number() && value.as_number() == 0.0;
        if zero(top)
            && zero(left)
            && ((zero(right) && zero(bottom))
                || (look.is(self.vm, n.width, "100%") && look.is(self.vm, n.height, "100%")))
        {
            look.set(n.overlay, self.w.fill);
        } else {
            // A side says where it is with a length or a share; `auto` says nothing. Saying nothing along a way, it stays where
            // it would have been, as what's placed by itself in what it's in does.
            let vm = &*self.vm;
            let said = |value: Value| {
                value.is_number()
                    || (value.is_string()
                        && text_of(vm, value).is_some_and(|text| text.trim() != "auto"))
            };
            let across = said(left) || said(right);
            let down = said(top) || said(bottom);
            if across && down {
                look.set(n.overlay, self.w.inset);
            }
        }
        if !look.is_undefined(n.overlay) {
            look.delete(n.position);
            if look.is(self.vm, n.overlay, "fill") {
                for key in [n.top, n.right, n.bottom, n.left, n.width, n.height] {
                    look.delete(key);
                }
            }
        }
        Ok(())
    }

    /// Where something goes in what it's in, as `Styled` in `web.js` works it out once what's in it has been: along a line,
    /// on one, as wide as it is, and where its auto margins put it. It gives back how it lays out what's in it.
    #[allow(clippy::too_many_arguments)]
    fn place(
        &mut self,
        look: &mut Look,
        tag: &str,
        flow: &str,
        on_line: bool,
        along: bool,
        block_inside: bool,
        around: Value,
        current: Value,
    ) -> Value {
        let n = self.n;
        look.delete(n.hidden);
        look.delete(n.scroll);
        look.delete(n.case);
        if !on_line && flow != "line" && look.is(self.vm, n.text_direction, "rtl") {
            self.mirrored_margins(look);
        }
        if look.is_undefined(n.display) && look.is_undefined(n.direction) && along {
            // One after another along a line, onto the next when there's no more room, as what's in a line of a page does.
            look.set(n.direction, self.w.row);
            look.set_if_nullish(n.align, self.w.baseline);
            let keeps =
                look.is(self.vm, n.white_space, "nowrap") || look.is(self.vm, n.white_space, "pre");
            look.set_if_nullish(n.flex_wrap, Value::bool(!keeps));
            look.set(n.line_strut, Value::TRUE);
            let backwards = look.is(self.vm, n.text_direction, "rtl");
            let side = self.along_line(look.get(n.text_align), backwards);
            look.set_if_nullish(n.justify, side);
        }
        if tag == "button" && look.is_undefined(n.display) && look.is_undefined(n.direction) {
            look.set_if_nullish(n.justify, self.w.center);
        }
        if tag == "svg" || tag == "img" {
            self.sits_on_line(look);
        }
        let inside = if look.is(self.vm, n.display, "grid")
            || look.is_true(n.flex)
            || !look.is_undefined(n.direction)
        {
            if look.is(self.vm, n.direction, "row") {
                if look.is_true(n.line_strut) {
                    self.w.line
                } else {
                    self.w.row
                }
            } else {
                self.w.column
            }
        } else {
            self.w.block
        };
        if block_inside {
            look.set(n.inline, Value::FALSE);
        }
        let inline = self.fit_in_flow(look, tag, flow, around);
        let line_align = look.get(n.line_align);
        if !line_align.is_undefined() {
            if flow == "row" {
                look.set_if_nullish(n.align_self, line_align);
            }
            look.delete(n.line_align);
        }
        // Only what's still inline: `display: block` on a link or a span makes it a block, whose padding takes room as any
        // block's does, as Bootstrap's `.nav-link` has it.
        if (on_line || flow == "block")
            && inline_text_tag(tag)
            && inline
            && !look.is_true(n.line_box)
            && !look.is_true(n.flex)
            && !look.is(self.vm, n.display, "grid")
            && !look.is(self.vm, n.position, "absolute")
        {
            // What's inline takes no more of its line up or down for its padding and borders.
            for (padding, border, margin) in [
                (n.padding_top, n.border_top_width, n.margin_top),
                (n.padding_bottom, n.border_bottom_width, n.margin_bottom),
            ] {
                let mut pad = look.get(padding);
                if pad.is_nullish() {
                    pad = Value::int(0);
                }
                let mut thick = look.get(border);
                if thick.is_nullish() {
                    thick = look.get(n.border_width);
                }
                if thick.is_nullish() {
                    thick = Value::int(0);
                }
                // Text, or an object, makes text of the sum, which nothing's done with.
                if !pad.is_string() && !thick.is_string() && !pad.is_object() && !thick.is_object()
                {
                    let more = number_of(pad) + number_of(thick);
                    if more > 0.0 {
                        let was = look.number(margin).unwrap_or(0.0);
                        look.set(margin, Value::number(was - more));
                    }
                }
            }
        }
        look.delete(n.line_box);
        if on_line && look.is_undefined(n.shrink) && look.number(n.width).is_some() {
            look.set(n.shrink, Value::int(0));
        }
        // A block's children don't shrink to fit it, as flex items do, whatever height they say: `h-full` in a grid's cell that's
        // stretched is as tall as the cell, with what's after it going past its end (shadcn/ui's Command in its card).
        if flow == "block" && look.is_undefined(n.shrink) && (look.number(n.height).is_some() || look.get(n.height).is_string()) {
            look.set(n.shrink, Value::int(0));
            look.set(n.min_auto, Value::TRUE);
        }
        if flow == "block" {
            // A block's children aren't flex items, so `flex: auto` does nothing to them, as Ant Design's `Layout` says it in
            // the page itself: it doesn't grow to fill the page, and its basis isn't its height.
            look.delete(n.grow);
            look.delete(n.basis);
        }
        self.auto_margins(look, flow);
        self.with_current_color(look, current);
        inside
    }

    /// What `{ ...settle(look), ...host }` comes to on an element made for a tag, with its picture's own size where CSS
    /// doesn't say, and then its font, as `finalFont` has it.
    fn host(
        &mut self,
        mut look: Look,
        host: Value,
        tag: &str,
        motion: [Value; 2],
        em: bool,
        shape_parts: Value,
    ) -> Run<Look> {
        let n = self.n;
        self.settle(&mut look, motion, em)?;
        let mut out = Look(look.0.clone());
        let mut given = Vec::with_capacity(16);
        self.vm.properties_into(host, &mut given)?;
        out.merge(&given);
        out.delete(n.line_scale);
        if tag == "svg" {
            out.set(n.parts, shape_parts);
        }
        // A canvas that CSS says only the width or height of keeps the shape its own size has, as a picture does.
        if tag == "canvas" {
            let (wide, high) = (out.get(n.own_width), out.get(n.own_height));
            if look.is_undefined(n.width) != look.is_undefined(n.height)
                && wide.is_number()
                && high.is_number()
                && wide.as_number() > 0.0
                && high.as_number() > 0.0
            {
                out.set(n.picture_aspect, Value::number(wide.as_number() / high.as_number()));
                out.delete(n.own_width);
                out.delete(n.own_height);
            }
        }
        for (key, said) in [(n.width, n.own_width), (n.height, n.own_height)] {
            let own = out.get(said);
            if !own.is_undefined() {
                if look.is_undefined(key) {
                    out.set(key, own);
                }
                out.delete(said);
            }
        }
        self.final_font(&mut out)?;
        Ok(out)
    }

    /// The props text gets from what it's in, which CSS does and props don't, apart from its color: `FONT_PROPS` in `web.js`.
    fn font_props(&self) -> [Atom; 20] {
        let n = self.n;
        [
            n.font_size,
            n.font_weight,
            n.font_family,
            n.font_style,
            n.text_align,
            n.line_height,
            n.line_scale,
            n.color,
            n.list_style,
            n.keep_words,
            n.white_space,
            n.size_said,
            n.letter_spacing,
            n.word_spacing,
            n.text_shadow,
            n.text_indent,
            n.text_direction,
            n.tabular_nums,
            n.case,
            n.text_wrap,
        ]
    }

    /// How text is written in an element, as `Styled` works it out before what's in it: sizes that are shares of the size of the text
    /// around it, what it gets from that, a font that's only monospace being smaller, a list in a list, and the height of its lines.
    /// It gives back what tells whether that's changed, which is each of the font props as text, as `FONT_PROPS.map(...).join('|')`.
    fn own_font(
        &mut self,
        look: &mut Look,
        above: &Look,
        has_above: bool,
        tag: &str,
        host: bool,
    ) -> Run<String> {
        let n = self.n;
        let own_size = !look.is_undefined(n.font_size) || !look.is_undefined(n.font_scale);
        let above_size = above.number(n.font_size);
        if host {
            if own_size && look.number(n.font_scale).is_none() {
                // A size in pixels, which a font that's only monospace doesn't change, here or in anything in it.
                look.set(n.size_said, Value::TRUE);
            }
            // With nothing around it saying how big its text is, as outside a `PageWindow`, that's how big rsui's own text is
            // (`DEFAULT_SIZE` in rsui-text), which is what it's drawn at: `0.8em` is a share of that.
            self.resolve_relative(look, above_size.unwrap_or(NATIVE_TEXT_SIZE));
        }
        let props = self.font_props();
        if has_above {
            for name in props {
                if look.is_undefined(name) {
                    let given = above.get(name);
                    if !given.is_undefined() {
                        look.set(name, given);
                    }
                }
            }
        }
        // `bolder` and `lighter` are a step from the weight around it, as CSS has them: Pico's `strong` is bold that way.
        let weight = look.get(n.font_weight);
        let bolder = is_text(self.vm, weight, "bolder");
        if bolder || is_text(self.vm, weight, "lighter") {
            let around = if has_above { above.get(n.font_weight) } else { Value::UNDEFINED };
            let from = if around.is_undefined() || around.is_null() || is_text(self.vm, around, "normal") {
                400.0
            } else if is_text(self.vm, around, "bold") {
                700.0
            } else {
                to_number(self.vm, around)?
            };
            let to = if bolder {
                if from < 350.0 {
                    400.0
                } else if from < 550.0 {
                    700.0
                } else {
                    900.0
                }
            } else if from < 550.0 {
                100.0
            } else if from < 750.0 {
                400.0
            } else {
                700.0
            };
            look.set(n.font_weight, Value::number(to));
        }
        if host && !own_size && !look.is_true(n.size_said) {
            let was = only_monospace(self.vm, above.get(n.font_family));
            let now = only_monospace(self.vm, look.get(n.font_family));
            if was != now {
                let size = above_size.unwrap_or(16.0) * if now { 13.0 / 16.0 } else { 16.0 / 13.0 };
                look.set(n.font_size, Value::number(size));
            }
        }
        if host && (tag == "ul" || tag == "ol") && !above.is_undefined(n.list_style) {
            // A list in a list has no room above or below it, and bullets that are hollow, or squares in one that's in one of those.
            let em = look.number(n.font_size).unwrap_or(16.0);
            for side in [n.margin_top, n.margin_bottom] {
                if look.number(side) == Some(em) {
                    look.set(side, Value::int(0));
                }
            }
            if tag == "ul" && look.is(self.vm, n.list_style, "disc") {
                let outer = above.get(n.list_style);
                let hollow = is_text(self.vm, outer, "circle") || is_text(self.vm, outer, "square");
                look.set(
                    n.list_style,
                    if hollow { self.w.square } else { self.w.circle },
                );
            }
        }
        if let (Some(scale), Some(size)) = (look.number(n.line_scale), look.number(n.font_size)) {
            look.set(n.line_height, Value::number(scale * size));
        }
        let mut key = String::new();
        for (at, name) in props.into_iter().enumerate() {
            if at > 0 {
                key.push('|');
            }
            let value = look.get(name);
            if !value.is_nullish() {
                key.push_str(&string_of(self.vm, value)?);
            }
        }
        Ok(key)
    }

    /// `finalFont`: what font props come to here, a file for a family that CSS has given files for, with the nearest weight
    /// there is, and otherwise a system font. It gives back whether anything's changed.
    fn final_font(&mut self, props: &mut Look) -> Run<bool> {
        let n = self.n;
        let mut changed = false;
        let text_align = props.get(n.text_align);
        let start = is_text(self.vm, text_align, "start");
        let end = is_text(self.vm, text_align, "end");
        if !props.is_undefined(n.text_direction) || start || end {
            changed = true;
            let backwards = props.is(self.vm, n.text_direction, "rtl");
            if text_align.is_undefined() || start {
                let side = if backwards {
                    self.w.right
                } else if text_align.is_undefined() {
                    Value::UNDEFINED
                } else {
                    self.w.left
                };
                props.set(n.text_align, side);
            } else if end {
                props.set(
                    n.text_align,
                    if backwards { self.w.left } else { self.w.right },
                );
            }
            if props.is_undefined(n.text_align) {
                props.delete(n.text_align);
            }
            // `ltr` stays too, as text that's said to go from the left does whatever its first letter is (rsui-text's
            // `direction_said`).
        }

        if !props.is_undefined(n.white_space) || !props.is_undefined(n.size_said) {
            changed = true;
            let keeps = props.is(self.vm, n.white_space, "nowrap")
                || props.is(self.vm, n.white_space, "pre");
            if keeps && props.is_undefined(n.no_wrap) {
                props.set(n.no_wrap, Value::TRUE);
            }
            // Spaces at the end of a line are part of it when they're kept as they're written, as `pre` keeps them: a `<span>`
            // that's only a space in a `pre` is as wide as a space.
            if props.is(self.vm, n.white_space, "pre")
                || props.is(self.vm, n.white_space, "pre-wrap")
                || props.is(self.vm, n.white_space, "break-spaces")
            {
                props.set(n.keep_spaces, Value::TRUE);
            }
            props.delete(n.white_space);
            props.delete(n.size_said);
        }
        if !props.is_undefined(n.line_scale) {
            changed = true;
            if let (Some(scale), Some(size)) =
                (props.number(n.line_scale), props.number(n.font_size))
            {
                props.set(n.line_height, Value::number(scale * size));
            }
            props.delete(n.line_scale);
        }
        let family = props.get(n.font_family);
        let weight = props.get(n.font_weight);
        if family.is_undefined() && weight.is_undefined() {
            return Ok(changed);
        }
        let wanted = if weight.is_undefined() || is_text(self.vm, weight, "normal") {
            400.0
        } else if is_text(self.vm, weight, "bold") {
            700.0
        } else {
            to_number(self.vm, weight)?
        };
        // The fonts CSS gave files for are found by the first of the family's names, past any that only a Mac has, with the
        // names after it for what the system's font, if that's what it is, isn't there.
        let mut rest = String::new();
        let files = if family.is_string() {
            let family = text_of(self.vm, family).unwrap_or_default().into_owned();
            let names: Vec<&str> = family.split(',').collect();
            let mut found = None;
            for (at, name) in names.iter().enumerate() {
                if let Some(index) = self.layout.fonts.iter().position(|(one, _)| one == name) {
                    rest = names[at + 1..].join(",");
                    found = Some(index);
                    break;
                }
                if !MAC_ONLY.contains(&name.to_ascii_lowercase().as_str()) {
                    break;
                }
            }
            found
        } else {
            None
        };
        if let Some(at) = files {
            let files = &self.layout.fonts[at].1;
            let italic = props.is(self.vm, n.font_style, "italic");
            let fits = |file: &&FontFile| file.italic == italic;
            let fitting: Vec<&FontFile> = if files.iter().any(|file| file.italic == italic) {
                files.iter().filter(fits).collect()
            } else {
                files.iter().collect()
            };
            let mut best = fitting[0];
            for file in &fitting {
                if (file.weight - wanted).abs() < (best.weight - wanted).abs() {
                    best = file;
                }
            }
            let (path, best_weight, best_italic) = (best.path, best.weight, best.italic);
            let (adjust, local, metrics) = (best.adjust, best.local, best.metrics);
            // Where its text sits in its lines, if the face says, as rsui-text reads `font_metrics`.
            match metrics {
                Some((ascent, descent)) => {
                    let text = format!("{} {}", number_text(self.vm, ascent), number_text(self.vm, descent));
                    let text = self.vm.new_string(&text);
                    props.set(n.font_metrics, text);
                }
                None => props.delete(n.font_metrics),
            }
            if let Some((local_weight, local_italic)) = local {
                // The system's font it named, in its own weight, or what's after it in the family if that isn't there.
                let name = text_of(self.vm, path).unwrap_or_default().into_owned();
                let family = if rest.is_empty() { name } else { format!("{name},{rest}") };
                let family = self.vm.new_string(&family);
                props.set(n.font_family, family);
                let weight = if local_weight == 700.0 {
                    self.w.bold
                } else if local_weight == 400.0 {
                    self.w.normal
                } else {
                    let text = number_text(self.vm, local_weight);
                    self.vm.new_string(&text)
                };
                props.set(n.font_weight, weight);
                if local_italic {
                    let italic = self.vm.new_string("italic");
                    props.set(n.font_style, italic);
                }
            } else {
                props.set(n.font_family, path);
                let weight = if wanted >= 600.0 && best_weight < 600.0 {
                    self.w.bold
                } else {
                    self.w.normal
                };
                props.set(n.font_weight, weight);
                if best_italic {
                    props.delete(n.font_style);
                }
            }
            if adjust != 1.0
                && let Some(size) = props.number(n.font_size)
            {
                props.set(n.font_size, Value::number(size * adjust));
            }
        } else {
            if family.is_string() {
                props.delete(n.font_metrics);
            }
            let weight = if wanted == 700.0 {
                self.w.bold
            } else if wanted == 400.0 {
                self.w.normal
            } else {
                let text = number_text(self.vm, wanted);
                self.vm.new_string(&text)
            };
            props.set(n.font_weight, weight);
        }
        Ok(true)
    }
}

/// Whether a font family is only `monospace`, which a browser draws smaller.
fn only_monospace(vm: &Vm, family: Value) -> bool {
    let Some(family) = text_of(vm, family) else {
        return false;
    };
    let family: String = family.chars().filter(|c| *c != '"' && *c != '\'').collect();
    crate::css::trim(&family).to_lowercase() == "monospace"
}

/// `/^(-?[0-9.]+)% \+ (-?[0-9.]+)px$/`, as its two numbers' text.
fn share_and_pixels(text: &str) -> Option<(&str, &str)> {
    let number = |part: &str| {
        let digits = part.strip_prefix('-').unwrap_or(part);
        !digits.is_empty()
            && digits
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.')
    };
    let (share, rest) = text.split_once("% + ")?;
    let pixels = rest.strip_suffix("px")?;
    (number(share) && number(pixels)).then_some((share, pixels))
}

/// `Number(text)` for what's only digits and points, with a sign.
fn number_text_value(text: &str) -> f64 {
    crate::declare::to_number(text)
}

fn motion_of(vm: &Vm, args: &Args, at: usize) -> [Value; 2] {
    [args.get(vm, at), args.get(vm, at + 1)]
}

fn tag_text(vm: &Vm, tag: Value) -> String {
    vm.string_text(tag).map(Cow::into_owned).unwrap_or_default()
}

// What's in an element: what each thing in it is, as CSS and its kind say, for whether they go along a line, float, or are
// blocks. Each is worked out once for each call, from what's been found out about each tag, each set of classes and each
// styled component, which is kept here until `web.js` forgets what it's worked out.

/// What `web.js` gives this, once, for working out what's in an element: its contexts' providers that only hand something down
/// (`NestedRules`, `InheritedFont`, `FlowContext` and `NthContext`), a fragment, and how it finds what CSS says about a tag, some
/// classes and one class: `tagLook`, `looksOfClasses` and `classLook`.
struct Kinds {
    providers: [Value; 4],
    fragment: Value,
    tag_look: Value,
    classes_look: Value,
    class_look: Value,
    _held: Vec<Root>,
}

/// Which way something floats, as `floatOf` says: `undefined`, `'left'`, `'right'`, or something else.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Float {
    Undefined,
    Left,
    Right,
    /// `inline-start` and `inline-end`, which are left and right as which way the text it's in goes says (`withFloats`).
    InlineStart,
    InlineEnd,
    Other,
}

/// What one look says about where something goes: for each prop, whether it's there and what it comes to. `None` is a prop
/// that isn't there, as what's spread after something goes over it only when it's there.
#[derive(Clone, Copy, Default, Debug)]
struct Said {
    /// `__inline === true`.
    inline: Option<bool>,
    /// `__block === true`.
    block: Option<bool>,
    /// `__flex === true`.
    flex: Option<bool>,
    /// `__lineBox === true`, as `display: inline-block` says.
    line_box: Option<bool>,
    /// `display === 'grid'`.
    grid: Option<bool>,
    float: Option<Float>,
    /// `__clear === true`.
    clear: bool,
    /// `position` is `'absolute'` or `'fixed'`.
    apart: bool,
    /// `__order`, when it's a number.
    order: Option<f64>,
}

impl Said {
    /// As a number, to be kept on a styled component: two bits for each of `inline`, `block`, `flex` and `grid` (0 when
    /// it isn't there, 1 for false, 2 for true), three for `float`, and one each for `clear`, `apart`, and whether there's
    /// an `order`, which is kept by itself, and two for `line_box`.
    fn packed(&self) -> u32 {
        let two = |one: Option<bool>| match one {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        };
        let float = match self.float {
            None => 0,
            Some(Float::Undefined) => 1,
            Some(Float::Left) => 2,
            Some(Float::Right) => 3,
            Some(Float::Other) => 4,
            Some(Float::InlineStart) => 5,
            Some(Float::InlineEnd) => 6,
        };
        two(self.inline)
            | two(self.block) << 2
            | two(self.flex) << 4
            | two(self.grid) << 6
            | float << 8
            | u32::from(self.clear) << 11
            | u32::from(self.apart) << 12
            | u32::from(self.order.is_some()) << 13
            | two(self.line_box) << 14
    }

    /// What [`packed`](Self::packed) made, with the `order` that was kept beside it.
    fn unpacked(bits: u32, order: Value) -> Said {
        let two = |at: u32| match (bits >> at) & 3 {
            0 => None,
            1 => Some(false),
            _ => Some(true),
        };
        Said {
            inline: two(0),
            block: two(2),
            flex: two(4),
            grid: two(6),
            float: match (bits >> 8) & 7 {
                0 => None,
                1 => Some(Float::Undefined),
                2 => Some(Float::Left),
                3 => Some(Float::Right),
                5 => Some(Float::InlineStart),
                6 => Some(Float::InlineEnd),
                _ => Some(Float::Other),
            },
            clear: bits & 1 << 11 != 0,
            apart: bits & 1 << 12 != 0,
            order: (bits & 1 << 13 != 0 && order.is_number()).then(|| order.as_number()),
            line_box: two(14),
        }
    }
}

/// Where CSS says something goes, as `saidFlow` says: on a line of its own, along one, or nothing's said.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Flow {
    Block,
    Inline,
    Unsaid,
}

/// What kind of element something is, as CSS knows it (`tagOf`): `null`, a tag, or something that isn't text.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Tag {
    Null,
    Text(String),
    Other,
}

impl Tag {
    fn is(&self, tag: &str) -> bool {
        matches!(self, Tag::Text(own) if own == tag)
    }

    fn text(&self) -> &str {
        match self {
            Tag::Text(own) => own,
            _ => "",
        }
    }
}

/// An element, as `__jsx` makes them: `{ type, key, props, kids }`.
#[derive(Clone, Copy)]
struct Node {
    ty: Value,
    props: Value,
    kids: Value,
}

/// What goes on a line of its own in a page.
fn block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "div"
            | "section"
            | "main"
            | "nav"
            | "header"
            | "footer"
            | "aside"
            | "article"
            | "form"
            | "fieldset"
            | "figure"
            | "li"
            | "td"
            | "th"
            | "dd"
            | "dt"
            | "details"
            | "summary"
            | "address"
            | "figcaption"
            | "hgroup"
            | "search"
    )
}

/// What's a block too, but isn't something `Styled` puts lines in.
fn other_block(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "table"
            | "pre"
            | "blockquote"
            | "hr"
    )
}

/// What's inside a block that's only things that go along a line, as buttons and links do, rather than one under another.
fn inline_tag(tag: &str) -> bool {
    matches!(
        tag,
        "a" | "abbr"
            | "b"
            | "button"
            | "cite"
            | "code"
            | "em"
            | "i"
            | "img"
            | "input"
            | "kbd"
            | "label"
            | "mark"
            | "select"
            | "small"
            | "span"
            | "strong"
            | "sub"
            | "sup"
            | "svg"
            | "textarea"
            | "time"
            | "u"
            | "s"
    )
}

/// Whether something is one of the kinds of values there are, which a hole in an array isn't.
fn is_value(value: Value) -> bool {
    value.is_object()
        || value.is_string()
        || value.is_number()
        || value.is_bool()
        || value.is_nullish()
        || value.is_symbol()
        || value.is_bigint()
}

/// What's in something as a list, as `Children.toArray` makes it: arrays flattened, and without `null`, `undefined` and
/// `true` or `false`.
fn flatten(vm: &Vm, value: Value, out: &mut Vec<Value>) {
    if let Some(items) = vm.array_elements(value) {
        for item in items.to_vec() {
            flatten(vm, item, out);
        }
    } else if !value.is_nullish() && !value.is_bool() && is_value(value) {
        out.push(value);
    }
}

fn is_text_or_number(value: Value) -> bool {
    value.is_string() || value.is_number()
}

/// What's being worked out about what's in an element, with what it's worked out with.
struct Kx<'v, 'l> {
    vm: &'v mut Vm,
    n: Names,
    layout: &'l mut Layout,
    /// What's on hand to be read, which is used again for each object.
    props: Vec<(Atom, Value)>,
}

impl Kx<'_, '_> {
    fn kinds(&self) -> Run<&Kinds> {
        match &self.layout.kinds {
            Some(kinds) => Ok(kinds),
            None => Err(Value::UNDEFINED),
        }
    }

    /// `object.name`, for what may not be an object, as `object?.name` does.
    fn get(&mut self, object: Value, name: Atom) -> Run<Value> {
        if object.is_object() {
            self.vm.get_property(object, name)
        } else {
            Ok(Value::UNDEFINED)
        }
    }

    /// Whether a kind of component that's made both what goes along a line and what doesn't makes what does for these props, as
    /// `__webInlineOf` in `web.js` has learned: see `learnFlowBy` there.
    fn inline_by_props(&mut self, node: &Node) -> Run<bool> {
        let n = self.n;
        let said = self.get(node.ty, n.web_inline_of)?;
        if !self.vm.is_function(said) {
            return Ok(false);
        }
        Ok(self.vm.call_function(said, Value::UNDEFINED, &[node.props])? == Value::TRUE)
    }

    /// The element this is, if it's one, as `isValidElement` says: an object, not a function, with `type` and `kids`.
    fn node(&mut self, value: Value) -> Run<Option<Node>> {
        if !value.is_object() || self.vm.is_function(value) {
            return Ok(None);
        }
        let n = self.n;
        let mut props = std::mem::take(&mut self.props);
        self.vm.properties_into(value, &mut props)?;
        let find = |name: Atom| {
            props
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| *value)
        };
        let found = match (find(n.ty), find(n.kids)) {
            (Some(ty), Some(kids)) => Some(Node {
                ty,
                props: find(n.props).unwrap_or(Value::UNDEFINED),
                kids,
            }),
            _ => None,
        };
        self.props = props;
        Ok(found)
    }

    /// What a look says about where something goes.
    fn said_by(&mut self, look: Value) -> Run<Said> {
        let mut said = Said::default();
        if !look.is_object() {
            return Ok(said);
        }
        let n = self.n;
        let mut props = std::mem::take(&mut self.props);
        self.vm.properties_into(look, &mut props)?;
        for (key, value) in &props {
            let key = *key;
            if key == n.inline {
                said.inline = Some(*value == Value::TRUE);
            } else if key == n.block {
                said.block = Some(*value == Value::TRUE);
            } else if key == n.flex {
                said.flex = Some(*value == Value::TRUE);
            } else if key == n.line_box {
                said.line_box = Some(*value == Value::TRUE);
            } else if key == n.display {
                said.grid = Some(is_text(self.vm, *value, "grid"));
            } else if key == n.float_said {
                said.float = Some(if is_text(self.vm, *value, "left") {
                    Float::Left
                } else if is_text(self.vm, *value, "right") {
                    Float::Right
                } else if is_text(self.vm, *value, "inline-start") {
                    Float::InlineStart
                } else if is_text(self.vm, *value, "inline-end") {
                    Float::InlineEnd
                } else if value.is_undefined() {
                    Float::Undefined
                } else {
                    Float::Other
                });
            } else if key == n.clear_said {
                said.clear = *value == Value::TRUE;
            } else if key == n.position {
                said.apart =
                    is_text(self.vm, *value, "absolute") || is_text(self.vm, *value, "fixed");
            } else if key == n.order_said && value.is_number() {
                said.order = Some(value.as_number());
            }
        }
        self.props = props;
        Ok(said)
    }

    /// What CSS says about a kind of element, as `tagLook` gives it, which is nothing for a tag that isn't text.
    fn tag_said(&mut self, tag: &Tag) -> Run<Option<Said>> {
        let Tag::Text(tag) = tag else {
            return Ok(None);
        };
        if let Some(found) = self.layout.tag_said.get(tag) {
            return Ok(*found);
        }
        let look = self.kinds()?.tag_look;
        let name = self.vm.new_string(tag);
        let look = self.vm.call_function(look, Value::UNDEFINED, &[name])?;
        let said = if look.is_undefined() {
            None
        } else {
            Some(self.said_by(look)?)
        };
        self.layout.tag_said.insert(tag.clone(), said);
        Ok(said)
    }

    /// What CSS says about the classes something has, as `looksOfClasses` gives it.
    fn classes_said(&mut self, class_name: Value) -> Run<Said> {
        let Some(name) = text_of(self.vm, class_name) else {
            return Ok(Said::default());
        };
        if name.is_empty() {
            return Ok(Said::default());
        }
        if let Some(found) = self.layout.classes_said.get(name.as_ref()) {
            return Ok(*found);
        }
        let name = name.into_owned();
        let look = self.kinds()?.classes_look;
        let look = self
            .vm
            .call_function(look, Value::UNDEFINED, &[class_name])?;
        let said = self.said_by(look)?;
        self.layout.classes_said.insert(name, said);
        Ok(said)
    }

    /// What a styled component's CSS says before anything's been given to it, as its `__webLook()` gives it, if it has one.
    ///
    /// It never changes, so it's kept on the component itself, as `__webSaid` (and `__webOrder`), which goes when the
    /// component does: a page that makes a styled component each time it's drawn doesn't have them all kept here.
    fn web_said(&mut self, ty: Value) -> Run<Said> {
        if !ty.is_object() {
            return Ok(Said::default());
        }
        let n = self.n;
        let kept = self.vm.get_property(ty, n.web_said)?;
        if kept.is_number() {
            let order = self.vm.get_property(ty, n.web_order)?;
            return Ok(Said::unpacked(kept.as_number() as u32, order));
        }
        let look = self.vm.get_property(ty, n.web_look)?;
        if look.is_nullish() {
            return Ok(Said::default());
        }
        let look = self.vm.call_function(look, ty, &[])?;
        let said = self.said_by(look)?;
        if let Some(order) = said.order {
            self.vm
                .set_property(ty, n.web_order, Value::number(order))?;
        }
        let packed = Value::number(f64::from(said.packed()));
        self.vm.set_property(ty, n.web_said, packed)?;
        Ok(said)
    }

    /// `tagOf`.
    fn tag_of(&mut self, node: &Node) -> Run<Tag> {
        let n = self.n;
        let tag = if self.vm.is_function(node.ty) {
            let tag = self.vm.get_property(node.ty, n.web_tag_said)?;
            if tag.is_nullish() { Value::NULL } else { tag }
        } else {
            let tag = self.get(node.props, n.web_tag)?;
            if !tag.is_nullish() {
                tag
            } else if node.ty.is_string() {
                node.ty
            } else {
                Value::NULL
            }
        };
        Ok(if tag.is_null() {
            Tag::Null
        } else if let Some(text) = text_of(self.vm, tag) {
            Tag::Text(text.into_owned())
        } else {
            Tag::Other
        })
    }

    /// `props.style`.
    fn style(&mut self, node: &Node) -> Run<Value> {
        let n = self.n;
        self.get(node.props, n.style)
    }

    /// `saidFlow`: where CSS says an element goes, from its own `style`, what's said about its kind of element, its styled
    /// component, and its classes, in that order, each going over what's before it.
    fn said_flow(&mut self, node: &Node) -> Run<Flow> {
        let n = self.n;
        let style = self.style(node)?;
        let display = self.get(style, n.display)?;
        if let Some(display) = text_of(self.vm, display) {
            return Ok(if display.starts_with("inline") {
                Flow::Inline
            } else if display == "none" || display == "contents" {
                Flow::Unsaid
            } else {
                Flow::Block
            });
        }
        let tag = self.tag_of(node)?;
        let mut merged = self.tag_said(&tag)?.unwrap_or_default();
        let web = self.web_said(node.ty)?;
        let class_name = self.get(node.props, n.class_name)?;
        let classes = self.classes_said(class_name)?;
        for said in [web, classes] {
            merged.inline = said.inline.or(merged.inline);
            merged.block = said.block.or(merged.block);
            merged.flex = said.flex.or(merged.flex);
            merged.grid = said.grid.or(merged.grid);
        }
        Ok(if merged.inline == Some(true) {
            Flow::Inline
        } else if merged.block == Some(true)
            || merged.flex == Some(true)
            || merged.grid == Some(true)
        {
            Flow::Block
        } else {
            Flow::Unsaid
        })
    }

    /// `saidLineBox`: whether CSS makes something a box of its own on a line, as `display: inline-block` and `inline-flex` do,
    /// which text it's in has beside it rather than running through it.
    fn said_line_box(&mut self, node: &Node) -> Run<bool> {
        let n = self.n;
        let style = self.style(node)?;
        let display = self.get(style, n.display)?;
        if let Some(display) = text_of(self.vm, display) {
            return Ok(matches!(&*display, "inline-block" | "inline-flex" | "inline-grid"));
        }
        let tag = self.tag_of(node)?;
        let mut merged = self.tag_said(&tag)?.unwrap_or_default();
        let web = self.web_said(node.ty)?;
        let class_name = self.get(node.props, n.class_name)?;
        let classes = self.classes_said(class_name)?;
        for said in [web, classes] {
            merged.inline = said.inline.or(merged.inline);
            merged.flex = said.flex.or(merged.flex);
            merged.line_box = said.line_box.or(merged.line_box);
        }
        Ok(merged.line_box == Some(true) || (merged.inline == Some(true) && merged.flex == Some(true)))
    }

    /// `floatOf`, for an element.
    fn float_of(&mut self, node: &Node) -> Run<Float> {
        let n = self.n;
        let style = self.style(node)?;
        let own = self.get(style, n.float)?;
        if is_text(self.vm, own, "left") {
            return Ok(Float::Left);
        }
        if is_text(self.vm, own, "right") {
            return Ok(Float::Right);
        }
        if is_text(self.vm, own, "inline-start") {
            return Ok(Float::InlineStart);
        }
        if is_text(self.vm, own, "inline-end") {
            return Ok(Float::InlineEnd);
        }
        let class_name = self.get(node.props, n.class_name)?;
        if let Some(float) = self.classes_said(class_name)?.float {
            return Ok(float);
        }
        let tag = self.tag_of(node)?;
        Ok(self
            .tag_said(&tag)?
            .and_then(|said| said.float)
            .unwrap_or(Float::Undefined))
    }

    /// `floatOf`, for anything.
    fn float_of_value(&mut self, value: Value) -> Run<Float> {
        match self.node(value)? {
            Some(node) => self.float_of(&node),
            None => Ok(Float::Undefined),
        }
    }

    /// `clearsFloats`.
    fn clears(&mut self, value: Value) -> Run<bool> {
        let Some(node) = self.node(value)? else {
            return Ok(false);
        };
        let n = self.n;
        let style = self.style(&node)?;
        let clear = self.get(style, n.clear)?;
        if !clear.is_undefined() {
            return Ok(!is_text(self.vm, clear, "none"));
        }
        let class_name = self.get(node.props, n.class_name)?;
        Ok(self.classes_said(class_name)?.clear)
    }

    /// `placedApart`: `position: absolute` or `fixed`, from its props, its `style`, its styled component's CSS, or what the
    /// page's CSS says about its classes, as Mantine's `.m_f2d85dd2 { position: absolute }` on a password field's input does.
    fn placed_apart(&mut self, node: &Node) -> Run<bool> {
        let n = self.n;
        let mut position = self.get(node.props, n.position)?;
        if position.is_nullish() {
            let style = self.style(node)?;
            position = self.get(style, n.position)?;
        }
        if position.is_nullish() {
            if self.web_said(node.ty)?.apart {
                return Ok(true);
            }
            let class_name = self.get(node.props, n.class_name)?;
            return Ok(self.classes_said(class_name)?.apart);
        }
        Ok(is_text(self.vm, position, "absolute") || is_text(self.vm, position, "fixed"))
    }

    /// Whether this element is only one of `web.js`'s own contexts handing something down, around what it holds.
    /// Or any context's provider, which has no box of its own and goes where what it holds goes.
    fn passes_down(&mut self, node: &Node) -> Run<bool> {
        let providers = self.kinds()?.providers;
        if providers.iter().any(|one| one.bits() == node.ty.bits()) {
            return Ok(true);
        }
        if !self.vm.is_function(node.ty) {
            return Ok(false);
        }
        let n = self.n;
        Ok(!self.vm.get_property(node.ty, n.context)?.is_nullish())
    }

    /// `isInline`.
    fn is_inline(&mut self, value: Value) -> Run<bool> {
        if is_text_or_number(value) || value.is_nullish() || value.is_bool() {
            return Ok(true);
        }
        if let Some(items) = self.vm.array_elements(value) {
            for item in items.to_vec() {
                // What `every` skips, as a hole is.
                if is_value(item) && !self.is_inline(item)? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        let Some(node) = self.node(value)? else {
            return Ok(false);
        };
        let n = self.n;
        if !self.placed_apart(&node)? {
            let text = is_text(self.vm, node.ty, "text") || is_text(self.vm, node.ty, "span");
            // A page's inline tag written as itself, as `<a>` is in a file whose JSX has no tag rsui has too (no `<span>`,
            // `<button>` or `<input>`), so it isn't made the page's `a` until it's drawn: it goes along a line as that does.
            let page_inline = node.ty.is_string() && inline_text_tag(self.tag_of(&node)?.text());
            if text || page_inline || self.get(node.ty, n.web_inline)? == Value::TRUE || self.inline_by_props(&node)? {
                // Unless CSS makes it a block, as `<span class="block">` is: then it's a box of its own, and what it's in lays it
                // out as one, rather than as words on a line, which would leave out a box with nothing in it.
                return Ok(self.said_flow(&node)? != Flow::Block);
            }
        }
        if self.passes_down(&node)? {
            let mut inside = node.kids;
            if inside.is_nullish() {
                inside = self.get(node.props, n.children)?;
            }
            return self.is_inline(inside);
        }
        Ok(false)
    }

    /// `holdsBlock`, for what an element has in its `props.children`.
    fn holds_block(&mut self, children: Value) -> Run<bool> {
        let mut all = Vec::new();
        flatten(self.vm, children, &mut all);
        for child in all {
            let Some(node) = self.node(child)? else {
                continue;
            };
            if self.said_flow(&node)? == Flow::Block {
                return Ok(true);
            }
            let tag = self.tag_of(&node)?;
            if block_tag(tag.text()) || other_block(tag.text()) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `goesOnLine`: whether something goes along a line with what's next to it, rather than starting a line of its own.
    fn goes_on_line(&mut self, value: Value) -> Run<bool> {
        if is_text_or_number(value) {
            return Ok(true);
        }
        let Some(node) = self.node(value)? else {
            return Ok(false);
        };
        let tag = self.tag_of(&node)?;
        if tag.is("svg") || tag.is("img") {
            return Ok(false);
        }
        match self.said_flow(&node)? {
            Flow::Inline => return Ok(true),
            Flow::Block => return Ok(false),
            Flow::Unsaid => {}
        }
        let n = self.n;
        if inline_text_tag(tag.text()) {
            let children = self.get(node.props, n.children)?;
            if self.holds_block(children)? {
                return Ok(false);
            }
        }
        Ok(self.is_inline(value)? || inline_box(tag.text()) || inline_text_tag(tag.text()))
    }

    /// `elementsIn`: the elements in something, with what's in a fragment counted as being there itself.
    fn elements_in(&mut self, children: Value, out: &mut Vec<Node>) -> Run<()> {
        let mut all = Vec::new();
        flatten(self.vm, children, &mut all);
        let fragment = self.kinds()?.fragment;
        let n = self.n;
        for one in all {
            let Some(node) = self.node(one)? else {
                continue;
            };
            if node.ty.bits() == fragment.bits() {
                let mut inside = node.kids;
                if inside.is_nullish() {
                    inside = self.get(node.props, n.children)?;
                }
                self.elements_in(inside, out)?;
            } else {
                out.push(node);
            }
        }
        Ok(())
    }

    /// `goesAlong`: whether what's in a block is only things that go along a line.
    fn goes_along(&mut self, children: Value) -> Run<bool> {
        let mut elements = Vec::new();
        self.elements_in(children, &mut elements)?;
        if elements.len() < 2 {
            return Ok(false);
        }
        for node in &elements {
            if self.float_of(node)? != Float::Undefined {
                return Ok(false);
            }
        }
        for node in &elements {
            match self.said_flow(node)? {
                Flow::Block => return Ok(false),
                Flow::Inline => continue,
                Flow::Unsaid => {}
            }
            let tag = self.tag_of(node)?;
            let kind = ["text", "span", "link", "image", "button", "input"]
                .iter()
                .any(|kind| is_text(self.vm, node.ty, kind));
            // A component that's been found to make something that goes along a line goes along one: see `learnFlow`.
            if !(inline_tag(tag.text()) || kind || self.get(node.ty, self.n.web_inline)? == Value::TRUE || self.inline_by_props(node)?) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// `textWithBoxes`: whether what's in something is text with boxes that go along a line with it.
    fn text_with_boxes(&mut self, all: &[Value]) -> Run<bool> {
        if !all.iter().any(|child| is_text_or_number(*child)) {
            return Ok(false);
        }
        let mut boxes = false;
        for child in all {
            if let Some(node) = self.node(*child)? {
                if inline_box(self.tag_of(&node)?.text()) {
                    boxes = true;
                    break;
                }
            }
        }
        if !boxes {
            return Ok(false);
        }
        for child in all {
            let Some(node) = self.node(*child)? else {
                continue;
            };
            if self.said_flow(&node)? == Flow::Block {
                return Ok(false);
            }
            if !self.is_inline(*child)? && !inline_box(self.tag_of(&node)?.text()) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Whether what's in an element made for `tag` goes along a line, as `Styled` works it out: text with boxes, or only what goes
    /// along a line in a block that isn't only text.
    fn along(&mut self, children: Value, tag: &str) -> Run<bool> {
        let mut all = Vec::new();
        flatten(self.vm, children, &mut all);
        if block_tag(tag) {
            // Text with only more text in it, as `<sup>` is, is one piece of text, which a line of boxes isn't.
            let mut only_text = all.iter().any(|child| is_text_or_number(*child));
            if only_text {
                let mut elements = Vec::new();
                self.elements_in(children, &mut elements)?;
                for node in &elements {
                    let tag = self.tag_of(node)?;
                    if !inline_text_tag(tag.text()) || self.said_flow(node)? == Flow::Inline {
                        only_text = false;
                        break;
                    }
                }
            }
            if !only_text && (self.goes_along(children)? || self.text_with_boxes(&all)?) {
                return Ok(true);
            }
        }
        Ok(inline_text_tag(tag) && self.text_with_boxes(&all)?)
    }

    /// What `intoLines` puts on lines: for each of what's in something, whether it goes along a line, or `null` if it's placed by
    /// itself, or nothing if it's left as it is.
    fn lines(&mut self, children: Value, always: bool) -> Run<Option<Vec<Option<bool>>>> {
        let mut all = Vec::new();
        flatten(self.vm, children, &mut all);
        let mut inline = Vec::with_capacity(all.len());
        let mut picture = Vec::with_capacity(all.len());
        for child in &all {
            let node = self.node(*child)?;
            let apart = match &node {
                Some(node) => self.placed_apart(node)?,
                None => false,
            };
            let on = if apart {
                None
            } else {
                Some(self.goes_on_line(*child)?)
            };
            let mut is_picture = false;
            if let (Some(false), Some(node)) = (on, &node) {
                let tag = self.tag_of(node)?;
                is_picture =
                    (tag.is("img") || tag.is("svg")) && self.said_flow(node)? == Flow::Unsaid;
            }
            inline.push(on);
            picture.push(is_picture);
        }
        // A picture next to another, or to what's on a line, goes along a line with it, as on a page.
        let beside = |at: usize, back: bool| {
            let mut next = at;
            loop {
                if back {
                    if next == 0 {
                        return false;
                    }
                    next -= 1;
                } else {
                    next += 1;
                    if next >= inline.len() {
                        return false;
                    }
                }
                if inline[next].is_none() {
                    continue;
                }
                return inline[next] == Some(true) || picture[next];
            }
        };
        let joined: Vec<bool> = (0..all.len())
            .map(|at| picture[at] && (beside(at, true) || beside(at, false)))
            .collect();
        for (at, one) in joined.into_iter().enumerate() {
            if one {
                inline[at] = Some(true);
            }
        }
        let any_on = inline.contains(&Some(true));
        let any_off = inline.contains(&Some(false));
        if !any_on || (!any_off && !always) {
            return Ok(None);
        }
        Ok(Some(inline))
    }

    /// `orderOf`: what CSS's `order` says about something in a flex box or a grid.
    fn order_of(&mut self, value: Value) -> Run<f64> {
        let Some(node) = self.node(value)? else {
            return Ok(0.0);
        };
        let n = self.n;
        let style = self.style(&node)?;
        let mut own = self.get(style, n.order)?;
        if own.is_nullish() {
            own = self.get(node.props, n.order)?;
        }
        if !own.is_undefined() {
            let number = to_number(self.vm, own)?;
            if !number.is_nan() {
                return Ok(number);
            }
        }
        if let Some(order) = self.web_said(node.ty)?.order {
            return Ok(order);
        }
        let class_name = self.get(node.props, n.class_name)?;
        let Some(names) = text_of(self.vm, class_name).map(Cow::into_owned) else {
            return Ok(0.0);
        };
        let mut order = 0.0;
        for name in names.split(crate::css::is_space) {
            if name.is_empty() {
                continue;
            }
            let found = match self.layout.class_orders.get(name) {
                Some(found) => *found,
                None => {
                    let look = self.kinds()?.class_look;
                    let text = self.vm.new_string(name);
                    let look = self.vm.call_function(look, Value::UNDEFINED, &[text])?;
                    let found = self.said_by(look)?.order;
                    self.layout.class_orders.insert(name.to_owned(), found);
                    found
                }
            };
            if let Some(found) = found {
                order = found;
            }
        }
        Ok(order)
    }
}

/// What `cssChildren` is asked.
const ALONG: i32 = 0;
const ANY_FLOAT: i32 = 1;
const LINES: i32 = 2;
const HOLDS_BLOCK: i32 = 3;
const BLOCK_CHILD: i32 = 4;
const TEXT_KINDS: i32 = 5;
const ORDERS: i32 = 6;
const LINE_BOXES: i32 = 7;

/// What `cssChild` is asked.
const SAID_FLOW: i32 = 0;
const FLOAT_OF: i32 = 1;
const CLEARS: i32 = 2;
const GOES_ON_LINE: i32 = 3;
const IS_INLINE: i32 = 4;

/// Runs something with what's needed to work out what's in an element, for the engine that gave this number.
fn with_kinds<R>(vm: &mut Vm, token: Value, run: impl FnOnce(&mut Kx<'_, '_>) -> Run<R>) -> Run<R> {
    with_layout(vm, token, |vm, layout| {
        if layout.kinds.is_none() {
            return Err(vm.type_error("rsui/web hasn't said what's in its elements"));
        }
        let mut kx = Kx {
            vm,
            n: layout.names,
            layout,
            props: Vec::with_capacity(8),
        };
        run(&mut kx)
    })
}

// What JavaScript calls.

/// `cssLayoutStart()`: the number to give back to everything here, for this engine.
fn host_start(vm: &mut Vm, _: Args) -> Run<Value> {
    let names = Names::new(vm);
    let mut held = Vec::new();
    let words = Words::new(vm, &mut held);
    let token = LAYOUTS.with(|layouts| {
        let mut layouts = layouts.borrow_mut();
        layouts.push(Layout {
            names,
            words,
            _held: held,
            fonts: Vec::new(),
            em: HashMap::new(),
            em_generation: 0,
            variable_names: HashMap::new(),
            variable_generation: 0,
            variable_fields: Vec::new(),
            kinds: None,
            tag_said: HashMap::new(),
            classes_said: HashMap::new(),
            class_orders: HashMap::new(),
        });
        layouts.len() - 1
    });
    Ok(Value::number(token as f64))
}

/// `cssFontFile(layout, family, weight, italic, path, adjust, localWeight, localItalic, ascent, descent)`: a font file
/// `@font-face` gave, for `finalFont`, or with `localWeight`, a font the system has, whose family `path` is. `ascent` and
/// `descent` are what `ascent-override` and `descent-override` say, as shares of the size, if they both do.
fn host_font_file(vm: &mut Vm, args: Args) -> Run<Value> {
    let (token, family, weight, italic, path) = (
        args.get(vm, 0),
        args.get(vm, 1),
        args.get(vm, 2),
        args.get(vm, 3),
        args.get(vm, 4),
    );
    let (adjust, local_weight, local_italic) = (args.get(vm, 5), args.get(vm, 6), args.get(vm, 7));
    let family = tag_text(vm, family);
    let weight = to_number(vm, weight)?;
    let number = |value: Value| value.is_number().then(|| value.as_number());
    let adjust = match number(adjust) {
        Some(adjust) if adjust > 0.0 && adjust.is_finite() => adjust,
        _ => 1.0,
    };
    let local = number(local_weight).map(|local_weight| (local_weight, local_italic == Value::TRUE));
    let metrics = match (number(args.get(vm, 8)), number(args.get(vm, 9))) {
        (Some(ascent), Some(descent)) if ascent.is_finite() && descent.is_finite() => Some((ascent, descent)),
        _ => None,
    };
    let held = vm.root(path);
    with_layout(vm, token, |_, layout| {
        let file = FontFile {
            weight,
            italic: italic == Value::TRUE,
            path,
            adjust,
            local,
            metrics,
        };
        layout._held.push(held);
        match layout.fonts.iter_mut().find(|(one, _)| *one == family) {
            Some((_, files)) => files.push(file),
            None => layout.fonts.push((family, vec![file])),
        }
        Ok(Value::UNDEFINED)
    })
}

/// `cssOwnFont(layout, look, fontAbove, tag, isHost, placed)`: how text is written in what `Styled` makes, before what's in it, as a
/// new look, with what tells whether that's changed on `placed.fontKey`.
fn host_own_font(vm: &mut Vm, args: Args) -> Run<Value> {
    let (token, given, above, tag) = (
        args.get(vm, 0),
        args.get(vm, 1),
        args.get(vm, 2),
        args.get(vm, 3),
    );
    let (host, placed) = (args.get(vm, 4) == Value::TRUE, args.get(vm, 5));
    let tag = tag_text(vm, tag);
    with_layout(vm, token, |vm, layout| {
        let mut look = Look::read(vm, given)?;
        let has_above = !above.is_null();
        let above = if above.is_object() {
            Look::read(vm, above)?
        } else {
            Look::default()
        };
        let mut cx = Cx {
            vm,
            n: layout.names,
            w: layout.words,
            layout,
        };
        let key = cx.own_font(&mut look, &above, has_above, &tag, host)?;
        let key = cx.vm.new_string(&key);
        let name = cx.n.font_key;
        cx.vm.set_property(placed, name, key)?;
        look.to_js(cx.vm)
    })
}

/// `cssSettle(layout, look, transitions, animation, em)`: what `settle` gives, as a new look.
fn host_settle(vm: &mut Vm, args: Args) -> Run<Value> {
    let (token, given) = (args.get(vm, 0), args.get(vm, 1));
    let motion = motion_of(vm, &args, 2);
    let em = args.get(vm, 4) == Value::TRUE;
    with_layout(vm, token, |vm, layout| {
        let mut look = Look::read(vm, given)?;
        let mut cx = Cx {
            vm,
            n: layout.names,
            w: layout.words,
            layout,
        };
        cx.settle(&mut look, motion, em)?;
        look.to_js(cx.vm)
    })
}

/// `cssFinalFont(layout, props)`: what `finalFont` gives, which is what it was given when there's nothing to change.
fn host_final_font(vm: &mut Vm, args: Args) -> Run<Value> {
    let (token, given) = (args.get(vm, 0), args.get(vm, 1));
    with_layout(vm, token, |vm, layout| {
        let mut props = Look::read(vm, given)?;
        let mut cx = Cx {
            vm,
            n: layout.names,
            w: layout.words,
            layout,
        };
        if cx.final_font(&mut props)? {
            props.to_js(cx.vm)
        } else {
            Ok(given)
        }
    })
}

/// `cssPlace(layout, look, tag, flow, onLine, along, blockInside, around, current, placed, host, transitions, animation, em,
/// shapeParts)`: where an element made for a tag goes in what it's in, as `Styled` works it out, with how it lays out what's
/// in it on `placed.inside`. With `host`, it's the props the element is made with, as `cssHost` gives them; without, it's
/// the look, for more to be done to it first.
fn host_place(vm: &mut Vm, args: Args) -> Run<Value> {
    let (token, given, tag, flow) = (
        args.get(vm, 0),
        args.get(vm, 1),
        args.get(vm, 2),
        args.get(vm, 3),
    );
    let (on_line, along, block_inside) = (args.get(vm, 4), args.get(vm, 5), args.get(vm, 6));
    let (around, current, placed, host) = (
        args.get(vm, 7),
        args.get(vm, 8),
        args.get(vm, 9),
        args.get(vm, 10),
    );
    let motion = motion_of(vm, &args, 11);
    let (em, shape_parts) = (args.get(vm, 13) == Value::TRUE, args.get(vm, 14));
    let tag = tag_text(vm, tag);
    let flow = tag_text(vm, flow);
    with_layout(vm, token, |vm, layout| {
        let mut look = Look::read(vm, given)?;
        let mut cx = Cx {
            vm,
            n: layout.names,
            w: layout.words,
            layout,
        };
        let inside = cx.place(
            &mut look,
            &tag,
            &flow,
            on_line == Value::TRUE,
            along == Value::TRUE,
            block_inside == Value::TRUE,
            around,
            current,
        );
        let name = cx.n.inside;
        cx.vm.set_property(placed, name, inside)?;
        if host.is_undefined() {
            return look.to_js(cx.vm);
        }
        let out = cx.host(look, host, &tag, motion, em, shape_parts)?;
        out.to_js(cx.vm)
    })
}

/// `cssHost(layout, look, host, tag, transitions, animation, em, shapeParts)`: the props an element made for a tag is made
/// with, from its look once it's been placed, and what's been worked out for it as an element.
fn host_host(vm: &mut Vm, args: Args) -> Run<Value> {
    let (token, given, host, tag) = (
        args.get(vm, 0),
        args.get(vm, 1),
        args.get(vm, 2),
        args.get(vm, 3),
    );
    let motion = motion_of(vm, &args, 4);
    let (em, shape_parts) = (args.get(vm, 6) == Value::TRUE, args.get(vm, 7));
    let tag = tag_text(vm, tag);
    with_layout(vm, token, |vm, layout| {
        let look = Look::read(vm, given)?;
        let mut cx = Cx {
            vm,
            n: layout.names,
            w: layout.words,
            layout,
        };
        let out = cx.host(look, host, &tag, motion, em, shape_parts)?;
        out.to_js(cx.vm)
    })
}

/// `cssKinds(layout, providers, fragment, tagLook, looksOfClasses, classLook)`: what's needed to work out what's in an element.
fn host_kinds(vm: &mut Vm, args: Args) -> Run<Value> {
    let (token, providers, fragment) = (args.get(vm, 0), args.get(vm, 1), args.get(vm, 2));
    let (tag_look, classes_look, class_look) = (args.get(vm, 3), args.get(vm, 4), args.get(vm, 5));
    let given = vm
        .array_elements(providers)
        .map(<[Value]>::to_vec)
        .unwrap_or_default();
    let providers: [Value; 4] =
        std::array::from_fn(|at| given.get(at).copied().unwrap_or(Value::NULL));
    let mut held: Vec<Root> = providers.iter().map(|one| vm.root(*one)).collect();
    for one in [fragment, tag_look, classes_look, class_look] {
        held.push(vm.root(one));
    }
    with_layout(vm, token, |_, layout| {
        layout.kinds = Some(Kinds {
            providers,
            fragment,
            tag_look,
            classes_look,
            class_look,
            _held: held,
        });
        Ok(Value::UNDEFINED)
    })
}

/// `cssForget(layout)`: what CSS says may have changed, so what's been found out from it is found out again.
fn host_forget(vm: &mut Vm, args: Args) -> Run<Value> {
    let token = args.get(vm, 0);
    with_layout(vm, token, |_, layout| {
        layout.tag_said.clear();
        layout.classes_said.clear();
        layout.class_orders.clear();
        Ok(Value::UNDEFINED)
    })
}

/// `cssChildren(layout, children, what, extra)`: something about what's in an element. `ALONG` (0, with its tag): whether it
/// goes along a line. `ANY_FLOAT` (1): whether anything floats. `LINES` (2, with `always`): what `intoLines` puts on lines, as
/// `true`, `false` or `null` (placed by itself) for each, or `undefined` if it's left as it is. `HOLDS_BLOCK` (3): whether
/// anything's a block, by its kind or by what CSS says. `BLOCK_CHILD` (4): whether CSS says anything's a block. `TEXT_KINDS` (5):
/// 1 if anything's text, and 2 if everything goes on a line, as `isInline` says. `ORDERS` (6): the `order` of each, or
/// `undefined` if they're all 0.
fn host_children(vm: &mut Vm, args: Args) -> Run<Value> {
    let (token, children, what, extra) = (
        args.get(vm, 0),
        args.get(vm, 1),
        args.get(vm, 2),
        args.get(vm, 3),
    );
    let what = if what.is_number() {
        what.as_number() as i32
    } else {
        -1
    };
    let tag = if what == ALONG {
        tag_text(vm, extra)
    } else {
        String::new()
    };
    with_kinds(vm, token, |kx| match what {
        ALONG => Ok(Value::bool(kx.along(children, &tag)?)),
        ANY_FLOAT => {
            let mut all = Vec::new();
            flatten(kx.vm, children, &mut all);
            for child in all {
                if kx.float_of_value(child)? != Float::Undefined {
                    return Ok(Value::TRUE);
                }
            }
            Ok(Value::FALSE)
        }
        LINES => match kx.lines(children, extra == Value::TRUE)? {
            None => Ok(Value::UNDEFINED),
            Some(inline) => {
                let items = inline
                    .into_iter()
                    .map(|one| one.map_or(Value::NULL, Value::bool))
                    .collect();
                Ok(kx.vm.new_array(items))
            }
        },
        HOLDS_BLOCK => Ok(Value::bool(kx.holds_block(children)?)),
        LINE_BOXES => {
            let mut all = Vec::new();
            flatten(kx.vm, children, &mut all);
            for child in all {
                if let Some(node) = kx.node(child)? {
                    if kx.said_line_box(&node)? {
                        return Ok(Value::TRUE);
                    }
                }
            }
            Ok(Value::FALSE)
        }
        BLOCK_CHILD => {
            let mut all = Vec::new();
            flatten(kx.vm, children, &mut all);
            for child in all {
                if let Some(node) = kx.node(child)? {
                    if kx.said_flow(&node)? == Flow::Block {
                        return Ok(Value::TRUE);
                    }
                }
            }
            Ok(Value::FALSE)
        }
        TEXT_KINDS => {
            let mut all = Vec::new();
            flatten(kx.vm, children, &mut all);
            let text = all.iter().any(|child| is_text_or_number(*child));
            let mut inline = true;
            for child in all {
                if !kx.is_inline(child)? {
                    inline = false;
                    break;
                }
            }
            Ok(Value::int(i32::from(text) | (i32::from(inline) << 1)))
        }
        ORDERS => {
            let mut all = Vec::new();
            flatten(kx.vm, children, &mut all);
            let mut orders = Vec::with_capacity(all.len());
            for child in all {
                orders.push(kx.order_of(child)?);
            }
            if orders.iter().all(|order| *order == 0.0) {
                return Ok(Value::UNDEFINED);
            }
            let items = orders.into_iter().map(Value::number).collect();
            Ok(kx.vm.new_array(items))
        }
        _ => Err(kx.vm.type_error("cssChildren doesn't know that")),
    })
}

/// `cssChild(layout, child, what)`: something about one thing. `SAID_FLOW` (0): `'block'`, `'inline'` or `undefined`, as
/// `saidFlow` says. `FLOAT_OF` (1): `'left'`, `'right'`, `undefined`, or `'other'` for anything else it floats. `CLEARS` (2),
/// `GOES_ON_LINE` (3) and `IS_INLINE` (4): as `clearsFloats`, `goesOnLine` and `isInline` say.
fn host_child(vm: &mut Vm, args: Args) -> Run<Value> {
    let (token, child, what) = (args.get(vm, 0), args.get(vm, 1), args.get(vm, 2));
    let what = if what.is_number() {
        what.as_number() as i32
    } else {
        -1
    };
    with_kinds(vm, token, |kx| {
        let w = kx.layout.words;
        match what {
            SAID_FLOW => Ok(match kx.node(child)? {
                Some(node) => match kx.said_flow(&node)? {
                    Flow::Block => w.block,
                    Flow::Inline => w.inline,
                    Flow::Unsaid => Value::UNDEFINED,
                },
                None => Value::UNDEFINED,
            }),
            FLOAT_OF => Ok(match kx.float_of_value(child)? {
                Float::Left => w.left,
                Float::Right => w.right,
                Float::InlineStart => w.inline_start,
                Float::InlineEnd => w.inline_end,
                Float::Other => w.other,

                Float::Undefined => Value::UNDEFINED,
            }),
            CLEARS => Ok(Value::bool(kx.clears(child)?)),
            GOES_ON_LINE => Ok(Value::bool(kx.goes_on_line(child)?)),
            IS_INLINE => Ok(Value::bool(kx.is_inline(child)?)),
            _ => Err(kx.vm.type_error("cssChild doesn't know that")),
        }
    })
}

/// Names relevant to variable resolution, in the same order as Object.keys(look). Looks are framework-owned plain data.
/// Resolving the variables, removing the fields, and applying cascade precedence remain in web.js.
fn host_variable_keys(vm: &mut Vm, args: Args) -> Run<Value> {
    let (token, given) = (args.get(vm, 0), args.get(vm, 1));
    with_layout(vm, token, |vm, layout| {
        if layout.variable_generation != vm.atom_generation() || layout.variable_names.len() > 4096 {
            layout.variable_generation = vm.atom_generation();
            layout.variable_names.clear();
        }
        let mut fields = std::mem::take(&mut layout.variable_fields);
        let result = (|| {
            vm.properties_into(given, &mut fields)?;
            let mut keys = Vec::new();
            for (key, _) in &fields {
                if layout.variable_names.len() >= 4096 {
                    layout.variable_names.clear();
                }
                let name = layout.variable_names.entry(*key).or_insert_with(|| {
                    let name = vm.atom_name(*key);
                    (name.starts_with("--") || name.starts_with("__var ")).then_some(name)
                });
                if let Some(name) = name {
                    keys.push(vm.new_string(name));
                }
            }
            Ok(vm.new_array(keys))
        })();
        fields.clear();
        if fields.capacity() <= 4096 {
            layout.variable_fields = fields;
        }
        result
    })
}

fn host_variables_same(vm: &mut Vm, args: Args) -> Run<Value> {
    let (before, after) = (args.get(vm, 0), args.get(vm, 1));
    Ok(Value::bool(vm.same_map_entries(before, after)))
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 13] = [
    ("cssKinds", host_kinds),
    ("cssForget", host_forget),
    ("cssChildren", host_children),
    ("cssChild", host_child),
    ("cssOwnFont", host_own_font),
    ("cssLayoutStart", host_start),
    ("cssFontFile", host_font_file),
    ("cssSettle", host_settle),
    ("cssFinalFont", host_final_font),
    ("cssPlace", host_place),
    ("cssHost", host_host),
    ("cssVariableKeys", host_variable_keys),
    ("cssVariablesSame", host_variables_same),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_key_discovery_preserves_order_and_observes_mutation() {
        let mut engine = rsui_js::Engine::new();
        let vm = engine.vm();
        let natives = vm.new_object();
        for (name, function) in FUNCTIONS {
            let function = vm.host_function(name, 0, function);
            let atom = vm.atom(name);
            vm.set_property(natives, atom, function).unwrap();
        }
        vm.set_global("natives", natives);
        engine.eval(r#"
            const layout = natives.cssLayoutStart();
            const look = Object.assign(Object.create({ '--inherited': 'no' }), {
                width: 20, '--local': '10px', color: 'red', '__var width': { value: 'var(--local)' }
            });
            Object.defineProperty(look, '--hidden', { value: 'no', enumerable: false });
            look[Symbol('ignored')] = true;
            function compare() {
                const want = Object.keys(look).filter(key => key.startsWith('--') || key.startsWith('__var '));
                const actual = natives.cssVariableKeys(layout, look);
                if (JSON.stringify(actual) !== JSON.stringify(want)) throw new Error('variable key order: ' + JSON.stringify(actual));
            }
            compare();
            look['__var height'] = { value: 'var(--local)' };
            delete look['__var width'];
            compare();
            // Prefix classification is bounded and must still see names added after it has filled.
            for (let i = 0; i < 4200; i++) look['ordinary' + i] = i;
            compare();
            look['--late'] = '30px';
            compare();
        "#, "variable-keys.js").unwrap();
    }

    /// How many times fewer to make up and run, which is 50 when the collector's run at every safe point
    /// (`RSUI_GC_STRESS`), as that's hundreds of times slower: what's made up is the same, only less of it.
    fn shorter() -> u32 {
        if std::env::var_os("RSUI_GC_STRESS").is_some_and(|value| value != "0") {
            50
        } else {
            1
        }
    }

    /// A piece of `web.js` as it is now, from where `start` is to the end of what it starts, for what it's compared with to be
    /// what's really there.
    fn piece(start: &str) -> &'static str {
        let web = include_str!("web.js");
        let from = web
            .find(start)
            .unwrap_or_else(|| panic!("web.js has {start}"));
        let end = if start.starts_with("function") {
            from + web[from..].find("\n}").expect("it ends") + 2
        } else {
            from + web[from..].find(';').expect("it ends") + 1
        };
        &web[from..end]
    }

    /// What `web.js` did to each element's look in JavaScript (kept in `tests/fixtures/layout_reference.js`), and what this does,
    /// compared deeply, with props in order, what's `undefined` apart from what isn't there, and `-0` and `NaN` as themselves:
    /// for every block of CSS in this crate, `web.js` and the apps that measure it, as
    /// `declare.rs` makes it into a look, and for looks made up at random from the props this reads, each for elements of many
    /// kinds, in every way of laying out what they're in.
    #[test]
    fn everything_comes_out_as_it_did_in_javascript() {
        let mut engine = rsui_js::Engine::new();
        let vm = engine.vm();
        let natives = vm.new_object();
        for (name, function) in crate::css::FUNCTIONS
            .into_iter()
            .chain(crate::declare::FUNCTIONS)
            .chain(FUNCTIONS)
        {
            let function = vm.host_function(name, 0, function);
            let atom = vm.atom(name);
            vm.set_property(natives, atom, function).unwrap();
        }
        vm.set_global("natives", natives);
        vm.set_global("short", Value::int(shorter() as i32));
        let corpus = [
            include_str!("web.js"),
            include_str!("../tests/web.rs"),
            include_str!("../tests/app.rs"),
            include_str!("../tests/fixtures/declare_reference.js"),
            include_str!("../tests/apps/web_dashboard.tsx"),
            include_str!("../tests/apps/web_css.tsx"),
        ];
        let texts = corpus.iter().map(|text| vm.new_string(text)).collect();
        let texts = vm.new_array(texts);
        vm.set_global("corpus", texts);
        let now = [
            piece("const NO_MOTION"),
            piece("function motionOf("),
            piece("function settle("),
            piece("const finalFont ="),
        ]
        .join("\n");
        let now = format!(
            "const now = (() => {{\n{now}\nreturn {{ settle, finalFont, motionOf }};\n}})();"
        );
        let script = [
            include_str!("../tests/fixtures/layout_reference.js"),
            COMPARE_START,
            &now,
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

    const COMPARE_START: &str = r#"
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
    if (typeof value === 'function') return 'function ' + value.name;
    return typeof value + ' ' + String(value);
};
const declaring = { em: '', vars: (text) => text, content: (value) => value, warn: () => {} };
/// What stands for `motionProps`, which hasn't changed: it says the same as it does, from only what it does.
function motionProps(look) {
    if (look.__transition !== undefined || look.__transitionParts !== undefined) {
        const said = String(look.__transition) + ' ' + JSON.stringify(look.__transitionParts ?? null);
        if (!said.includes('none')) {
            look.transitions = 'moves ' + said;
        }
    }
    delete look.__transition;
    delete look.__transitionParts;
    if (look.__animation !== undefined || look.__animationParts !== undefined) {
        if (!String(look.__animation).includes('none')) {
            look.animation = 'plays ' + String(look.__animation);
        }
    }
    delete look.__animation;
    delete look.__animationParts;
}
const layout = natives.cssLayoutStart();
const placing = { inside: 'block' };
"#;

    const COMPARE: &str = r#"
const outcome = (run) => {
    globalThis.__rsuiViewportUnits = undefined;
    let said;
    try {
        said = 'gave ' + show(run());
    } catch (error) {
        said = 'threw ' + (error && error.name);
    }
    return said + ' viewport ' + show(globalThis.__rsuiViewportUnits);
};
const compare = (what, input, old, now) => {
    const before = outcome(old);
    const after = outcome(now);
    compared += 1;
    if (before !== after && failures.length < 30) {
        failures.push(what + ' ' + show(input) + '\n  was ' + before + '\n  now ' + after);
    }
};

for (const [family, weight, italic, path, adjust, localWeight, localItalic, ascent, descent] of [['Brand', 400, false, 'C:/f/brand.ttf'], ['Brand', 700, false, 'C:/f/brand-bold.ttf'],
    ['Brand', 400, true, 'C:/f/brand-italic.ttf'], ['Mono', 300, false, 'C:/f/mono.ttf'], ['Mono', NaN, false, 'C:/f/mono-x.ttf'], ['Slanted', 500, true, 'C:/f/s.ttf'],
    ['Local (Custom)', 400, false, 'Segoe UI', 1.03, 400, false, 1.05, 0.35], ['Local (Custom)', 500, false, 'Segoe UI', 1.03, 600, false],
    ['Local (Custom)', 700, true, 'Segoe UI', 0.5, 700, true], ['Sized', 400, false, 'C:/f/sized.ttf', 2]]) {
    const files = layoutReference.fontFiles.get(family) ?? [];
    files.push({ weight, italic, path, adjust: typeof adjust === 'number' && adjust > 0 && Number.isFinite(adjust) ? adjust : 1,
        local: typeof localWeight === 'number' ? { weight: localWeight, italic: localItalic === true } : undefined,
        metrics: typeof ascent === 'number' && typeof descent === 'number' ? [ascent, descent] : undefined });
    layoutReference.fontFiles.set(family, files);
    natives.cssFontFile(layout, family, weight, italic, path, adjust, localWeight, localItalic, ascent, descent);
}

const INLINE_TEXT_TAGS = new Set(['span', 'a', 'b', 'i', 'em', 'strong', 'small', 'code', 'u', 's', 'sub', 'sup', 'mark', 'abbr', 'cite', 'kbd', 'time', 'label', 'samp', 'var', 'dfn', 'ins', 'del', 'data', 'output', 'bdi', 'bdo']);
/// What `Styled` does now, around `cssPlace`.
const newPlace = (look, tag, givenFlow, along, blockChild, fontAbove, host, shapeParts) => {
    const onLine = givenFlow === 'line';
    const flow = onLine ? 'row' : givenFlow;
    const blockInside = INLINE_TEXT_TAGS.has(tag) && look.__inline === undefined && blockChild;
    const current = look.color ?? fontAbove?.color;
    const [transitions, animation] = now.motionOf(look);
    const em = declaring.em !== '';
    const fieldset = tag === 'fieldset';
    let out = fieldset
        ? natives.cssPlace(layout, look, tag, flow, onLine, along, blockInside, fontAbove?.textAlign, current, placing)
        : natives.cssPlace(layout, look, tag, flow, onLine, along, blockInside, fontAbove?.textAlign, current, placing, host, transitions, animation, em, shapeParts);
    const inside = placing.inside;
    if (fieldset) {
        out = natives.cssHost(layout, out, host, tag, transitions, animation, em, shapeParts);
    }
    return { inside, props: out };
};

let seed = 24680;
const random = (below) => {
    seed = (seed * 1103515245 + 12345) % 2147483648;
    return seed % below;
};
const pick = (list) => list[random(list.length)];
const TAGS = ['ul', 'ol', 'div', 'span', 'button', 'svg', 'img', 'a', 'li', 'input', 'fieldset', 'p', 'label', 'h1', 'td', 'table', 'ul', 'textarea', 'section', 'strong', 'select', 'video'];
const FLOWS = ['block', 'row', 'column', 'line'];
const parts = function shapeParts() {};
const FONTS = [null, { color: '#111111', textAlign: 'center' }, { textAlign: 'right' }, { color: undefined, fontSize: 13 }, { textAlign: 'left', color: 'red' }];
const HOSTS = [{}, { onClick: function clicked() {}, __ownWidth: 30 }, { __ownHeight: '20', title: 'x', width: 5 }, { __id: 'a', color: 'red', __ownWidth: undefined },
    { __ownWidth: 12, __ownHeight: 0, height: undefined }, { fontFamily: 'Brand', textAlign: 'end' }, { __lineScale: 2, noWrap: false },
    { fontFamily: '-apple-system,BlinkMacSystemFont,Local (Custom),Arial', fontSize: 14, fontWeight: 500 }, { fontFamily: 'Local (Custom)', fontStyle: 'italic', fontWeight: 'bold' },
    { fontFamily: 'Sized,Arial', fontSize: 10 }, { fontFamily: 'Arial,Brand', fontSize: 10 }];
const PAGES = [undefined, [1000, 700], [500], null];
const FONTS_ABOVE = [null, {}, { fontSize: 13, fontFamily: 'monospace', __listStyle: 'disc', color: '#ffffff' },
    { fontSize: '20px', fontFamily: '"Monospace" ', textAlign: 'center', __listStyle: 'circle', keepWords: true, lineHeight: 20 },
    { __lineScale: 1.5, fontSize: 16, letterSpacing: 2, textShadow: { toString: () => '1px' }, textIndent: undefined },
    { fontFamily: 'Arial', __whiteSpace: 'pre', __sizeSaid: true, fontWeight: 700, __listStyle: 'square' }, { fontSize: -0, textDirection: 'rtl', fontStyle: null }];

const everyWay = (look, what) => {
    globalThis.__rsuiPageSize = PAGES[random(PAGES.length)];
    declaring.em = random(3) === 0 ? 'x' : '';
    compare('settle ' + what, look, () => layoutReference.settle({ ...look }), () => now.settle({ ...look }));
    compare('finalFont ' + what, look, () => {
        const given = { ...look };
        const out = layoutReference.finalFont(given);
        return [out === given, out];
    }, () => {
        const given = { ...look };
        const out = now.finalFont(given);
        return [out === given, out];
    });
    for (let one = 0; one < 2; one += 1) {
        const fontAbove = pick(FONTS_ABOVE);
        const tag = pick(TAGS);
        const isHost = random(5) > 0;
        compare('ownFont ' + [what, tag, isHost, show(fontAbove)].join(' '), look, () => {
            const done = layoutReference.ownFont({ ...look }, fontAbove, tag, isHost);
            return [done.look, done.fontKey];
        }, () => {
            const done = natives.cssOwnFont(layout, { ...look }, fontAbove, tag, isHost, placing);
            return [done, placing.fontKey];
        });
    }
    for (let one = 0; one < 3; one += 1) {
        const tag = pick(TAGS);
        const givenFlow = pick(FLOWS);
        const along = random(3) === 0;
        const blockChild = random(4) === 0;
        const fontAbove = pick(FONTS);
        const host = pick(HOSTS);
        compare('place ' + [what, tag, givenFlow, along, blockChild, show(fontAbove), show(host)].join(' '), look,
            () => layoutReference.place({ ...look }, tag, givenFlow, along, blockChild, fontAbove, { ...host }, parts),
            () => newPlace({ ...look }, tag, givenFlow, along, blockChild, fontAbove, { ...host }, parts));
    }
};

// Every block of CSS there is, as a look.
const blocks = (block) => {
    if (block.declarations.length > 0) {
        everyWay(natives.cssDeclare(block.declarations, [16, 13, 24][random(3)], 16, declaring), 'block');
        for (const one of block.declarations) {
            everyWay(natives.cssDeclare([one], 16, 16, declaring), 'declaration');
        }
    }
    for (const rule of block.rules) {
        blocks(rule.block);
    }
};
// With less to do, only the apps that measure it.
for (const text of short > 1 ? corpus.slice(4, 6) : corpus) {
    blocks(natives.cssRead(text));
}

// Made up, from the props this reads and what they can be, the same every time.
const KEYS = ['display', 'direction', 'align', 'flexWrap', 'lineStrut', 'justify', 'textAlign', '__whiteSpace', '__hidden', '__scroll', '__case', '__block', '__flex',
    '__inline', 'position', 'onLine', 'width', 'height', 'minWidth', 'maxWidth', 'minHeight', 'maxHeight', 'fitWidth', 'alignSelf', '__lineAlign', '__lineBox',
    'paddingTop', 'paddingRight', 'paddingBottom', 'paddingLeft', 'borderWidth', 'borderTopWidth', 'borderRightWidth', 'borderBottomWidth', 'borderLeftWidth',
    'marginTop', 'marginBottom', 'shrink', 'minAuto', '__autoTop', '__autoRight', '__autoBottom', '__autoLeft', '__fillWidth', 'background', 'borderColor',
    'borderTopColor', 'shadowColor', 'fill', 'stroke', 'hover', 'pressed', '__transition', '__transitionParts', '__animation', '__animationParts', '__fontScale',
    'fontSize', '__flow', '__alignContent', '__sticky', 'top', 'bottom', 'left', 'right', '__visibility', 'opacity', '__nativeBorder', '__ownBorder', '__boxSizing',
    'gap', 'columnGap', 'rowGap', '__listStyle', '__ownMargins', '__fixed', 'overlay', '__aspect', 'gridFlow', 'columns', 'rows', 'justifyItems', '__lineScale', 'textDirection',
    '__sizeSaid', 'noWrap', 'lineHeight', 'fontFamily', 'fontWeight', 'fontStyle', 'keepWords', 'letterSpacing', 'textShadow', 'textIndent', '__em_paddingTop', '__em_width', '__em_', '__em_gap', 'color', '__ownWidth',
    'transitions', 'animation', 'constructor'];
const VALUES = [undefined, null, true, false, 0, -0, 1, 2.5, -3, NaN, 16, 'grid', 'row', 'column', 'block', 'center', 'right', 'left', 'end', 'start', 'currentcolor',
    '#123456', '50%', '10% + 4px', '-5.5% + -2px', '1.2.3% + 1px', '. % + .px', '%', '100%', 'nowrap', 'pre', 'absolute', 'fixed', 'hidden', 'visible', 'border-box',
    'column dense', 'space-between', 'constructor', 'rtl', 'ltr', 'bold', 'bolder', 'lighter', 'normal', '600', ' 700 ', 'italic', 'Brand', 'Mono', 'Slanted', 'Segoe UI', {},
    { toString: () => '7px' }, [], 'baseline', 'monospace', "'monospace'", ' MonoSpace ', 'disc', 13, 1.25, '1s ease', 'none', 4, 0.1, 1e21, -0.5, '12px'];
for (let made = 0; made < 15000 / short; made += 1) {
    const look = {};
    const count = random(14);
    for (let at = 0; at < count; at += 1) {
        look[pick(KEYS)] = pick(VALUES);
    }
    if (random(4) === 0) {
        look.__fixed = true;
        look.top = pick([0, 5, undefined, 'auto']);
        look.left = pick([0, 3, undefined]);
        look.right = pick([0, 2, undefined]);
        look.bottom = pick([0, 1, undefined]);
    }
    everyWay(look, 'made up');
}
failures.push(compared + ' compared');
failures.join('\n')
"#;

    /// What `web.js` asked in JavaScript about what's in an element (kept in `tests/fixtures/layout_reference.js`), and what this
    /// answers, compared deeply, for 20,000 lists of what an element can have in it made up at random: text, numbers, nothing,
    /// arrays with holes, objects that aren't elements, and elements of every kind, made for a tag, by a styled component, a
    /// plain component, a context's provider or a fragment, with classes, `style`s, positions and orders, nested a few deep. What
    /// CSS says changes halfway through, as it does when a page's CSS does, and `cssForget` is told, as `changed` tells it.
    #[test]
    fn what_is_in_an_element_comes_out_as_it_did_in_javascript() {
        let mut engine = rsui_js::Engine::new();
        let vm = engine.vm();
        let natives = vm.new_object();
        for (name, function) in FUNCTIONS {
            let function = vm.host_function(name, 0, function);
            let atom = vm.atom(name);
            vm.set_property(natives, atom, function).unwrap();
        }
        vm.set_global("natives", natives);
        vm.set_global("short", Value::int(shorter() as i32));
        let now = format!(
            "const now = (() => {{\n{}\n{}\nreturn {{ hostType }};\n}})();",
            piece("const TEXT_KINDS"),
            piece("function hostType(")
        );
        let script = [
            include_str!("../tests/fixtures/layout_reference.js"),
            CHILDREN_START,
            &now,
            CHILDREN,
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

    const CHILDREN_START: &str = r#"
const failures = [];
let compared = 0;
const show = (value) => {
    if (value === undefined) return 'undefined';
    if (value === null) return 'null';
    if (typeof value === 'number') return Object.is(value, -0) ? '-0' : String(value);
    if (typeof value === 'string') return JSON.stringify(value);
    if (Array.isArray(value)) return '[' + Array.from(value, (one, at) => (at in value ? show(one) : 'hole')).join(',') + ']';
    if (typeof value === 'object') return '{' + Object.keys(value).map((key) => JSON.stringify(key) + ':' + show(value[key])).join(',') + '}';
    if (typeof value === 'function') return 'function ' + value.name;
    return typeof value + ' ' + String(value);
};
// React's, as react.js has them.
function isValidElement(value) {
    return typeof value === 'object' && value !== null && 'type' in value && 'kids' in value;
}
function flatten(children, out) {
    if (Array.isArray(children)) {
        for (const child of children) {
            flatten(child, out);
        }
    } else if (children !== null && children !== undefined && typeof children !== 'boolean') {
        out.push(children);
    }
    return out;
}
const Children = { toArray: (children) => flatten(children, []) };
const context = (name) => ({ Provider: function Provider() {}, name });
const contexts = { NestedRules: context('NestedRules'), InheritedFont: context('InheritedFont'), FlowContext: context('FlowContext'), NthContext: context('NthContext') };
function Fragment(props) {
    return props.children;
}
globalThis.__Fragment = Fragment;
const SVG_TAGS = { svg: 'Svg', path: 'Path', g: 'G', circle: 'Circle', rect: 'Rect', line: 'Line', polyline: 'Polyline', polygon: 'Polygon', ellipse: 'Ellipse' };
const rsui = { Checkbox: 'Checkbox', Slider: 'Slider' };
const Select = 'Select';
const Option = 'Option';
const TEXT_TAGS = new Set(['span', 'p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'label', 'small', 'strong', 'em', 'b', 'i', 'pre', 'code', 'legend', 'blockquote', 'sup', 'sub', 'u', 's', 'mark', 'abbr', 'cite', 'kbd', 'time', 'del', 'ins', 'samp', 'var', 'dfn', 'q', 'data', 'output', 'bdi', 'bdo']);
const BLOCK_TAGS = new Set(['div', 'hgroup', 'search', 'section', 'main', 'nav', 'header', 'footer', 'aside', 'article', 'form', 'fieldset', 'figure', 'li', 'td', 'th', 'dd', 'dt', 'details', 'summary', 'address', 'figcaption']);

// What CSS says, which changes halfway through.
let generation = 0;
const TAG_LOOKS = [
    { div: {}, span: { __inline: true }, p: { display: 'grid' }, li: { __float: 'left' }, img: { __block: true }, section: { __flex: true },
        a: { __inline: undefined, __float: 'right' }, label: { position: 'absolute' }, td: { __float: undefined }, sup: { __inline: false, __block: true } },
    { div: { __inline: true }, span: { display: 'block' }, p: {}, img: { __float: 'right' }, button: { __block: true }, strong: { __clear: true, __float: 'x' } },
];
const tagLook = (name) => TAG_LOOKS[generation][name];
const CLASS_LOOKS = [
    { b: { __block: true }, i: { __inline: true }, x: { __inline: false, __block: true }, fl: { __float: 'left' }, fr: { __float: 'right' }, fn: { __float: 'none' },
        fu: { __float: undefined }, c: { __clear: true }, g: { display: 'grid' }, f: { __flex: true }, o2: { __order: 2 }, on: { __order: -1 }, os: { __order: '3' },
        abs: { position: 'absolute' }, ng: { display: 'flex', __inline: undefined } },
    { b: { __inline: true }, i: { __block: true }, fl: { __float: 'right' }, c: { __clear: false }, o2: { __order: 7 }, g: {}, f: { __flex: false, __float: 'left' } },
];
const classLook = (name) => CLASS_LOOKS[generation][name];
const NO_LOOK = Object.freeze({});
function looksOfClasses(name) {
    if (typeof name !== 'string' || name === '') {
        return NO_LOOK;
    }
    const out = {};
    for (const one of name.split(/\s+/)) {
        const look = classLook(one);
        if (look !== undefined) {
            Object.assign(out, look);
        }
    }
    return out;
}
const old = childrenReference(contexts);
const layout = natives.cssLayoutStart();
natives.cssKinds(layout, [contexts.NestedRules.Provider, contexts.InheritedFont.Provider, contexts.FlowContext.Provider, contexts.NthContext.Provider],
    Fragment, tagLook, looksOfClasses, classLook);
const ALONG = 0;
const ANY_FLOAT = 1;
const LINES = 2;
const HOLDS_BLOCK = 3;
const BLOCK_CHILD = 4;
const ORDERS = 6;
const LINE_BOXES = 7;
"#;

    const CHILDREN: &str = r#"
const outcome = (run) => {
    try {
        return 'gave ' + show(run());
    } catch (error) {
        return 'threw ' + (error && error.name);
    }
};
const compare = (what, input, before, after) => {
    const was = outcome(before);
    const is = outcome(after);
    compared += 1;
    if (was !== is && failures.length < 30) {
        failures.push(what + ' ' + show(input) + '\n  was ' + was + '\n  now ' + is);
    }
};

let seed = 13579;
const random = (below) => {
    seed = (seed * 1103515245 + 12345) % 2147483648;
    return seed % below;
};
const pick = (list) => list[random(list.length)];
const component = (name, tag, look, inline) => {
    const made = { [name]: function () {} }[name];
    made.__webTag = tag;
    made.__webLook = () => look;
    made.__webInline = inline;
    return made;
};
const COMPONENTS = [component('Box', 'div', { __inline: true }, false), component('Word', 'span', null, true), component('Pinned', undefined, { position: 'fixed', __order: 5 }),
    component('Picture', 'img', { display: 'grid' }), component('Para', 'p', { __block: true, __inline: undefined }), component('Ordered', 'li', { __order: '4', __float: 'left' }),
    function Plain() {}];
const PROVIDERS = [contexts.NestedRules.Provider, contexts.InheritedFont.Provider, contexts.FlowContext.Provider, contexts.NthContext.Provider, function Other() {}];
const TYPES = ['div', 'span', 'p', 'img', 'svg', 'a', 'button', 'input', 'li', 'text', 'link', 'image', 'group', 'table', 'h1', 'label', 'section', 'sup', 'strong',
    'select', 'textarea', 'ul', 'td', 'hr'];
const CLASSES = ['', 'b', 'i', 'x', 'fl', 'fr', 'fn', 'fu', 'c', 'g', 'f', 'o2', 'on', 'os', 'abs', 'ng', ' b  i ', 'fl c', 'i b', 'o2 os', 5, undefined, null];
const STYLES = [undefined, null, 'display:block', {}, { display: 'inline-block' }, { display: 'none' }, { display: 'contents' }, { display: 'flex' }, { display: 5 },
    { float: 'left' }, { float: 'right' }, { float: 'none' }, { clear: 'both' }, { clear: 'none' }, { clear: null }, { position: 'absolute' }, { position: 'fixed' },
    { position: null }, { order: 2 }, { order: '1' }, { order: 'x' }, { order: null }, { display: 'inline', float: 'left', order: -0 }];
const thing = (depth) => {
    const r = random(20);
    if (r < 5) {
        return pick(['a', ' ', '', 'text ', 5, 0, -0]);
    }
    if (r === 5) {
        return pick([null, undefined, true, false]);
    }
    if (r === 6) {
        return pick([{}, [], [null], { type: 'div' }, function loose() {}]);
    }
    if (r === 7 && depth < 3) {
        return list(depth + 1);
    }
    return element(depth);
};
const list = (depth) => {
    const out = [];
    const count = random(6);
    for (let at = 0; at < count; at += 1) {
        out.push(thing(depth));
    }
    if (random(8) === 0) {
        // A hole.
        out[out.length + 1] = thing(depth);
    }
    return out;
};
const element = (depth) => {
    const kind = random(12);
    const type = kind < 7 ? pick(TYPES) : kind < 9 ? pick(COMPONENTS) : kind < 11 ? pick(PROVIDERS) : Fragment;
    const props = random(8) === 0 ? null : {};
    if (props !== null) {
        if (random(2) === 0) {
            props.className = pick(CLASSES);
        }
        if (random(3) === 0) {
            props.style = pick(STYLES);
        }
        if (random(8) === 0) {
            props.position = pick(['absolute', null, 'relative', 'fixed']);
        }
        if (random(8) === 0) {
            props.order = pick([3, '2', null, 'no']);
        }
        if (random(10) === 0) {
            props.webTag = pick(['p', 'img', null, 7, 'span', 'div']);
        }
    }
    const inner = depth < 3 ? (random(2) === 0 ? list(depth + 1) : thing(depth + 1)) : undefined;
    if (typeof type === 'function' && random(4) > 0) {
        // A component has what's in it as its `children`, as `__jsx` makes it.
        const given = props ?? {};
        if (inner !== undefined) {
            given.children = inner;
        }
        return { type, key: undefined, props: given, kids: null };
    }
    if (props !== null && random(4) === 0) {
        props.children = thing(depth + 1);
    }
    return { type, key: undefined, props, kids: random(5) === 0 ? pick([null, undefined]) : inner };
};
const floatSaid = (side) => (side === undefined || side === 'left' || side === 'right' ? side : 'other');

for (let made = 0; made < 20000 / short; made += 1) {
    if (made === 10000 / short) {
        generation = 1;
        natives.cssForget(layout);
    }
    const children = random(5) === 0 ? thing(0) : list(0);
    const tag = pick(['div', 'span', 'p', 'li', 'a', 'section', 'label', 'button', 'td', 'h1', 'svg', 'input', 'select', 'option', 'img', 'ul', 'textarea']);
    compare('along ' + tag, children, () => old.along(children, tag), () => natives.cssChildren(layout, children, ALONG, tag));
    compare('anyFloat', children, () => old.anyFloat(children), () => natives.cssChildren(layout, children, ANY_FLOAT));
    const always = random(2) === 0;
    compare('lines ' + always, children, () => old.lines(children, always), () => natives.cssChildren(layout, children, LINES, always));
    compare('holdsBlock', children, () => old.holdsBlock(children), () => natives.cssChildren(layout, children, HOLDS_BLOCK));
    compare('blockChild', children, () => old.blockChild(children), () => natives.cssChildren(layout, children, BLOCK_CHILD));
    compare('orders', children, () => old.orders(children), () => natives.cssChildren(layout, children, ORDERS));
    compare('lineBoxes', children, () => old.lineBoxes(children), () => natives.cssChildren(layout, children, LINE_BOXES));
    const props = pick([{}, { display: 'flex' }, { direction: 'row' }, { type: 'checkbox' }, { type: 'range' }, { type: 'radio', display: undefined }]);
    const given = random(6) === 0 ? undefined : children;
    compare('hostType ' + tag + ' ' + show(props), given, () => old.hostType(tag, props, given), () => now.hostType(tag, props, given));
    for (const one of Children.toArray(children)) {
        // It's only ever asked about an element.
        if (isValidElement(one)) {
            compare('saidFlow', one, () => old.saidFlow(one), () => natives.cssChild(layout, one, 0));
        }
        compare('floatOf', one, () => floatSaid(old.floatOf(one)), () => natives.cssChild(layout, one, 1));
        compare('clearsFloats', one, () => old.clearsFloats(one), () => natives.cssChild(layout, one, 2));
        compare('goesOnLine', one, () => old.goesOnLine(one), () => natives.cssChild(layout, one, 3));
        compare('isInline', one, () => old.isInline(one), () => natives.cssChild(layout, one, 4));
    }
}
failures.push(compared + ' compared');
failures.join('\n')
"#;

    /// What's kept on a styled component comes back as it was, for every way it can be.
    #[test]
    fn what_a_styled_component_says_comes_back_as_it_was_kept() {
        let twos = [None, Some(false), Some(true)];
        let floats = [
            None,
            Some(Float::Undefined),
            Some(Float::Left),
            Some(Float::Right),
            Some(Float::Other),
            Some(Float::InlineStart),
            Some(Float::InlineEnd),
        ];

        let mut seen = 0;
        for inline in twos {
            for block in twos {
                for flex in twos {
                    for grid in twos {
                        for float in floats {
                            for (clear, apart) in
                                [(false, false), (true, false), (false, true), (true, true)]
                            {
                                for order in [None, Some(3.0), Some(-0.5)] {
                                    let said = Said {
                                        inline,
                                        block,
                                        flex,
                                        grid,
                                        float,
                                        clear,
                                        apart,
                                        order,
                                        // Each of what it can be, along with the rest.
                                        line_box: grid,
                                    };
                                    let kept = order.map_or(Value::UNDEFINED, Value::number);
                                    let back = Said::unpacked(said.packed(), kept);
                                    assert_eq!(format!("{said:?}"), format!("{back:?}"));
                                    seen += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(seen, 81 * 7 * 4 * 3);

    }

    /// A page that makes a new styled component each time it's drawn has each go when nothing has it any more, rather than
    /// kept here: 10,000 of them, each asked about as `Styled` asks about what's in it, take no more room than 1,000 did,
    /// once garbage's been collected. Each one's own CSS is still only worked out once, however often it's asked about.
    #[test]
    fn styled_components_made_each_time_they_are_drawn_are_not_kept() {
        let mut engine = rsui_js::Engine::new();
        let vm = engine.vm();
        let natives = vm.new_object();
        for (name, function) in FUNCTIONS {
            let function = vm.host_function(name, 0, function);
            let atom = vm.atom(name);
            vm.set_property(natives, atom, function).unwrap();
        }
        vm.set_global("natives", natives);
        engine
            .eval(
                r#"
const layout = natives.cssLayoutStart();
natives.cssKinds(layout, [{}, {}, {}, {}], function Fragment() {}, () => undefined, () => ({}), () => undefined);
let worked = 0;
let made = 0;
globalThis.draw = (times) => {
    for (let at = 0; at < times; at += 1) {
        // What `styled.span` makes, with what its CSS says, which is worked out when it's first asked for.
        const Made = function Styled() {};
        Made.__webTag = 'button';
        Made.__webInline = true;
        const look = { __inline: true, __order: at, room: new Array(64).fill(at) };
        Made.__webLook = () => {
            worked += 1;
            return look;
        };
        made += 1;
        const children = ['text ', { type: Made, key: undefined, props: { children: 'x' }, kids: null }];
        // As `Styled` asks: whether it goes along a line, what's text, and its order.
        if (natives.cssChildren(layout, children, 0, 'div') !== true || natives.cssChildren(layout, children, 5) !== 3) {
            throw new Error('not inline');
        }
        const orders = natives.cssChildren(layout, children, 6);
        if (at > 0 && orders[1] !== at) {
            throw new Error('order ' + orders);
        }
    }
    return worked === made;
};
"#,
                "made",
            )
            .unwrap();
        let draw = |engine: &mut rsui_js::Engine, times: u32| {
            let done = engine.eval(&format!("draw({times})"), "draw").unwrap();
            assert_eq!(
                engine.display(done),
                "true",
                "each one's CSS is worked out once"
            );
            engine.collect_garbage();
            engine.memory().objects
        };
        let after_some = draw(&mut engine, 1000 / shorter());
        let after_more = draw(&mut engine, 10_000 / shorter());
        assert!(
            after_more <= after_some + after_some / 10,
            "{after_some} bytes of objects after 1,000, and {after_more} after 11,000"
        );
        println!("{after_some} bytes of objects after 1,000, and {after_more} after 11,000");
    }

    /// How much is kept here for the engine that's loaded `rsui/web` last on this thread: what it holds on to, and how many
    /// tags, sets of classes, classes and names of props it's found out about.
    fn kept_here() -> [usize; 5] {
        LAYOUTS.with(|layouts| {
            let layouts = layouts.borrow();
            let layout = layouts.last().expect("rsui/web has started");
            [
                layout._held.len(),
                layout.tag_said.len(),
                layout.classes_said.len(),
                layout.class_orders.len(),
                layout.em.len(),
            ]
        })
    }

    /// A page that makes a new styled component each time it's drawn, as code converted from React sometimes does, drawn
    /// 10,000 times by clicking: nothing more is kept here for it after that than after it's been drawn 100 times, as what's
    /// worked out about each styled component is kept on it, and goes when it does.
    #[test]
    fn a_page_that_makes_a_styled_component_each_time_it_is_drawn_keeps_nothing_of_them_here() {
        let folder = std::env::temp_dir().join(format!("rsui-layout-made-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("app.tsx");
        std::fs::write(
            &file,
            r#"
            import { useState } from 'react';
            import { __styled as styled, dom } from 'rsui/web';
            export default function App() {
                const [count, setCount] = useState(0);
                // A new component each time, which `rsui/web` is asked about by the `div` it's in.
                const Word = styled.span`
                    color: #c00000;
                    display: inline-block;
                    order: 2;
                `;
                return (
                    <dom.div>
                        <dom.button key="more" onClick={() => setCount((was) => was + 1)}>more</dom.button>
                        <dom.div key="line">
                            {'Drawn ' + count + ' '}
                            <Word key="word">{'word ' + count}</Word>
                        </dom.div>
                    </dom.div>
                );
            }
            "#,
        )
        .unwrap();
        let entry = crate::load(&file, 512 * 1024).unwrap();
        let text = rsui::default_text_engine().unwrap();
        let mut app = rsui::App::new(crate::root, entry, text, (500, 400), 1.0);
        app.update();
        app.scene();
        // The middle of the button, which stays where it is.
        let mut path = Vec::new();
        assert!(
            find(app.runtime().root(), "more", &mut path),
            "{}",
            app.outline()
        );
        let rect = app.layout().rect(&path).expect("it's been laid out");
        let (x, y) = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
        app.pointer_move(x, y);
        let draw = |app: &mut rsui::App, times: u32| {
            for _ in 0..times {
                app.pointer_down(x, y, 1);
                let _ = app.pointer_up(x, y);
                app.update();
            }
        };
        let (some, more) = (100 / shorter(), 9_900 / shorter());
        draw(&mut app, some);
        let after_some = kept_here();
        draw(&mut app, more);
        app.scene();
        let outline = app.outline();
        assert!(
            outline.contains(&format!("Drawn {}", some + more))
                && outline.contains(&format!("word {}", some + more)),
            "{outline}"
        );
        let after_more = kept_here();
        assert_eq!(after_some[..4], after_more[..4], "kept after 100, and after 10,000");
        // What's known about the names of props is let go of each time the engine frees names, and found again as they're
        // seen, so how much of it there is depends on when that last was. It's never more than the names there are.
        assert!(after_more[4] <= 64 && after_some[4] <= 64, "names of props known: {after_some:?} and {after_more:?}");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Where the element with this key is, as indexes from the root.
    fn find(node: &rsui::core::Node, key: &str, path: &mut Vec<u32>) -> bool {
        if node
            .key
            .as_ref()
            .is_some_and(|found| found.to_string() == key)
        {
            return true;
        }
        for (at, child) in node.children.iter().enumerate() {
            path.push(at as u32);
            if find(child, key, path) {
                return true;
            }
            path.pop();
        }
        false
    }
}
