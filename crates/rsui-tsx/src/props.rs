//! Turns what's written on a JSX element into the props rsui's layout and renderer read.

use std::sync::LazyLock;

use rsui::core::{ActionKind, PropValue, Symbol};
use rsui::render::Color;
use rsui_js::{Atom, Root, Value, Vm};

/// What one prop's name means. Each name is only worked out once, however many elements use it.
#[derive(Clone, Copy)]
pub(crate) enum Meaning {
    /// Dealt with somewhere else, or not there at all as far as the tree goes.
    Skip,
    Style,
    Name,
    /// Calls this function when this happens.
    Handler(ActionKind),
    /// A handler for an event as the web has it, as `onPointerDown` and `onKeyDown` are, which the layer turns into
    /// handlers of rsui's: see `rsuiEvents` in `layer.js`.
    Event,
    Number(Symbol),
    /// A number, or a string like `"50%"`, which goes in the second prop.
    Size(Symbol, Symbol),
    Padding,
    PaddingX,
    PaddingY,
    Margin,
    MarginX,
    MarginY,
    /// Takes a share of the space, which is really three props.
    Flex,
    Color(Symbol),
    Plain(Symbol),
    /// Drawn in the platform's look: see `native_look.rs`.
    Native,
}

/// `fontSize` as `font_size`, which is how rsui's own props are written.
fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    for character in name.chars() {
        if character.is_ascii_uppercase() {
            out.push('_');
            out.push(character.to_ascii_lowercase());
        } else {
            out.push(character);
        }
    }
    out
}

fn meaning_of(name: &str) -> Meaning {
    let symbol = |text: &str| Meaning::Number(Symbol::new(text));
    match name {
        "key" | "children" | "ref" | "tag" => Meaning::Skip,
        "style" => Meaning::Style,
        "native" => Meaning::Native,
        "name" => Meaning::Name,
        "onClick" => Meaning::Handler(ActionKind::Click),
        "onFocus" => Meaning::Handler(ActionKind::Focus),
        "onHoverChange" => Meaning::Handler(ActionKind::Custom(Symbol::new("hover_change"))),
        "onPressChange" => Meaning::Handler(ActionKind::Custom(Symbol::new("press_change"))),
        "onFocusWithinChange" => Meaning::Handler(ActionKind::Custom(Symbol::new("focus_within"))),
        "onFocusRingChange" => Meaning::Handler(ActionKind::Custom(Symbol::new("focus_ring"))),
        "onToggle" => Meaning::Handler(ActionKind::Toggle),
        "onChange" => Meaning::Handler(ActionKind::SetValue),
        // What a field's typed into, as a page's `onInput` is, which rsui's own fields say with `onChange`.
        "onInput" => Meaning::Handler(ActionKind::SetValue),
        "onScroll" => Meaning::Handler(ActionKind::Scroll),
        "onKeyPress" => Meaning::Handler(ActionKind::KeyPress),
        "onResize" => Meaning::Handler(ActionKind::Resize),
        "onDismiss" => Meaning::Handler(ActionKind::Dismiss),
        "onDoubleClick" => Meaning::Handler(ActionKind::Custom(Symbol::new("double_click"))),
        "onContextMenu" => Meaning::Handler(ActionKind::Custom(Symbol::new("context_menu"))),
        "onDrag" => Meaning::Handler(ActionKind::Custom(Symbol::new("drag"))),
        "onDropFiles" => Meaning::Handler(ActionKind::Custom(Symbol::new("drop_files"))),
        "onClose" => Meaning::Handler(ActionKind::Custom(Symbol::new("close_request"))),
        "onMouseEnter" | "onMouseLeave" | "onMouseOver" | "onMouseOut" | "onPointerEnter" | "onPointerLeave"
        | "onPointerOver" | "onPointerOut" | "onPointerDown" | "onPointerDownCapture" | "onPointerUp"
        | "onPointerUpCapture" | "onPointerMove" | "onPointerMoveCapture" | "onMouseDown" | "onMouseDownCapture"
        | "onMouseUp" | "onMouseUpCapture" | "onMouseMove" | "onMouseMoveCapture" | "onWheel" | "onWheelCapture"
        | "onKeyDown" | "onKeyDownCapture" | "onKeyUp" | "onKeyUpCapture" | "onAuxClick" | "onClickCapture"
        | "onPaste" | "onCopy" | "onCut" | "onCompositionStart" | "onCompositionUpdate" | "onCompositionEnd"
        | "onBlur" | "onDragStart" | "onDragEnter" | "onDragOver" | "onDragLeave" | "onDrop"
        | "onDragEnd" => Meaning::Event,
        // What `rsui/web` gives a page's elements, for events as the web has them, which go from the root down to what they
        // happened to and back up. They're given what happened, and what's after the name says when they hear it.
        other if other.starts_with("__on") => match web_event_action(&other[4..]) {
            Some(action) => Meaning::Handler(ActionKind::Custom(Symbol::new(action))),
            None => Meaning::Skip,
        },
        "width" => Meaning::Size(Symbol::new("width"), Symbol::new("width_percent")),
        "height" => Meaning::Size(Symbol::new("height"), Symbol::new("height_percent")),
        "padding" => Meaning::Padding,
        "paddingX" => Meaning::PaddingX,
        "paddingY" => Meaning::PaddingY,
        "margin" => Meaning::Margin,
        "marginX" => Meaning::MarginX,
        "marginY" => Meaning::MarginY,
        "flex" => Meaning::Flex,
        "radius" => symbol("corner_radius"),
        "background" => Meaning::Color(Symbol::new("background")),
        "hover" => Meaning::Color(Symbol::new("hover_background")),
        "pressed" => Meaning::Color(Symbol::new("pressed_background")),
        "color" => Meaning::Color(Symbol::new("text_color")),
        "borderColor" => Meaning::Color(Symbol::new("border_color")),
        "shadowColor" => Meaning::Color(Symbol::new("shadow_color")),
        "borderTopColor" => Meaning::Color(Symbol::new("border_top_color")),
        "scrollbarThumb" => Meaning::Color(Symbol::new("scrollbar_thumb")),
        "scrollbarTrack" => Meaning::Color(Symbol::new("scrollbar_track")),
        "placeholderColor" => Meaning::Color(Symbol::new("placeholder_color")),
        "outlineColor" => Meaning::Color(Symbol::new("outline_color")),
        "decorationColor" => Meaning::Color(Symbol::new("decoration_color")),
        "borderRightColor" => Meaning::Color(Symbol::new("border_right_color")),
        "borderBottomColor" => Meaning::Color(Symbol::new("border_bottom_color")),
        "borderLeftColor" => Meaning::Color(Symbol::new("border_left_color")),
        // A share of what it's in, as `100%`, is for the layout to work out, as `maxWidth`'s is, so it's passed on as it is.
        "minWidth" => Meaning::Plain(Symbol::new("min_width")),
        "minHeight" | "maxHeight" | "grow" | "shrink" | "marginTop"
        | "marginRight" | "marginBottom" | "marginLeft" | "gap" | "paddingTop" | "paddingRight"
        | "paddingBottom" | "paddingLeft" | "rowHeight" | "scrollX" | "scrollY" | "overlayX"
        | "overlayY" | "borderWidth" | "cornerRadius" | "opacity" | "fontSize" | "lineHeight"
        | "borderTopWidth" | "borderRightWidth" | "borderBottomWidth" | "borderLeftWidth"
        | "scrollbarSize" | "scrollbarRadius" | "scrollbarInset" | "stickyTop" | "stickyBottom" => symbol(&snake_case(name)),
        other => Meaning::Plain(Symbol::new(&snake_case(other))),
    }
}

/// The action for an event as the web has it, by what comes after `__on` in the name `rsui/web` gives its handler.
fn web_event_action(name: &str) -> Option<&'static str> {
    Some(match name {
        "PointerDown" => "pointer_down",
        "PointerDownCapture" => "pointer_down_capture",
        "PointerUp" => "pointer_up",
        "PointerUpCapture" => "pointer_up_capture",
        "PointerMove" => "pointer_move",
        "PointerMoveCapture" => "pointer_move_capture",
        "MouseDown" => "mouse_down",
        "MouseDownCapture" => "mouse_down_capture",
        "MouseUp" => "mouse_up",
        "MouseUpCapture" => "mouse_up_capture",
        "MouseMove" => "mouse_move",
        "MouseMoveCapture" => "mouse_move_capture",
        "Wheel" => "wheel",
        "WheelCapture" => "wheel_capture",
        "KeyDown" => "key_down",
        "KeyDownCapture" => "key_down_capture",
        "KeyUp" => "key_up",
        "KeyUpCapture" => "key_up_capture",
        "Clipboard" => "clipboard",
        "ClipboardCapture" => "clipboard_capture",
        "Composition" => "composition",
        "CompositionCapture" => "composition_capture",
        "AuxClick" => "aux_click",
        "AuxClickCapture" => "aux_click_capture",
        "ClickCapture" => "click_capture",
        "MotionEnd" => "motion_end",
        "PointerHover" => "pointer_hover",
        "Drag" => "web_drag",
        "DragCapture" => "web_drag_capture",
        _ => return None,
    })
}

/// What each name means, by its atom. An atom's number can be given to another name once nothing uses it, so what's
/// known goes whenever the engine has freed atoms.
#[derive(Default)]
pub(crate) struct Meanings {
    known: Vec<Option<Meaning>>,
    /// The engine's [`Vm::atom_generation`] when these were worked out.
    generation: u32,
}

impl Meanings {
    /// How many names this has room for.
    pub(crate) fn len(&self) -> usize {
        self.known.len()
    }

    pub(crate) fn of(&mut self, vm: &Vm, atom: Atom) -> Meaning {
        let number = atom.number() as usize;
        // A whole number as a name means nothing to an element.
        if number >= 1 << 24 {
            return Meaning::Skip;
        }
        if vm.atom_generation() != self.generation {
            self.generation = vm.atom_generation();
            self.known.clear();
        }
        if number >= self.known.len() {
            self.known.resize(number + 64, None);
        }
        *self.known[number].get_or_insert_with(|| meaning_of(&vm.atom_name(atom)))
    }
}

/// A color, from a number like `0x1F6FEB`, or text like `#1f6feb`, `#fff` or `#00000066`.
pub(crate) fn parse_color(vm: &Vm, value: Value) -> Option<i64> {
    if value.is_number() {
        return Some(Color::hex(value.as_number() as u32).to_prop());
    }
    let text = vm.string_text(value)?;
    let color = Color::from_text(&text)?;
    Some(color.to_prop())
}

/// Names that are looked up by text in `gather`, which happens to everything drawn, so it's worth keeping them.
struct Sides {
    margin_left: Symbol,
    margin_right: Symbol,
    margin_top: Symbol,
    margin_bottom: Symbol,
    left: Symbol,
    right: Symbol,
    top: Symbol,
    bottom: Symbol,
    grow: Symbol,
    shrink: Symbol,
    basis: Symbol,
}

static SIDES: LazyLock<Sides> = LazyLock::new(|| Sides {
    margin_left: Symbol::new("margin_left"),
    margin_right: Symbol::new("margin_right"),
    margin_top: Symbol::new("margin_top"),
    margin_bottom: Symbol::new("margin_bottom"),
    left: Symbol::new("padding_left"),
    right: Symbol::new("padding_right"),
    top: Symbol::new("padding_top"),
    bottom: Symbol::new("padding_bottom"),
    grow: Symbol::new("grow"),
    shrink: Symbol::new("shrink"),
    basis: Symbol::new("basis"),
});

/// What an element's props have come to.
#[derive(Default)]
pub(crate) struct Gathered {
    pub(crate) entries: Vec<(Symbol, PropValue)>,
    pub(crate) name: Option<Value>,
    pub(crate) handlers: Vec<(ActionKind, Root)>,
    /// Whether there's a handler for an event as the web has it, which the layer is asked to turn into rsui's.
    pub(crate) events: bool,
    /// Whether it says `native`, for the platform's look.
    pub(crate) native: bool,
}

fn number_of(value: Value) -> Option<f64> {
    value.is_number().then(|| value.as_number())
}

/// Adds what one prop means.
pub(crate) fn gather(
    vm: &mut Vm,
    meanings: &mut Meanings,
    out: &mut Gathered,
    atom: Atom,
    value: Value,
) -> Result<(), Value> {
    // A prop that's left out by being `undefined` or `null` is how JSX says it isn't there.
    if value.is_nullish() {
        return Ok(());
    }
    let meaning = meanings.of(vm, atom);
    match meaning {
        Meaning::Skip => {}
        Meaning::Style => {
            for (inner, inner_value) in vm.properties(value)? {
                gather(vm, meanings, out, inner, inner_value)?;
            }
        }
        Meaning::Name => out.name = Some(value),
        Meaning::Handler(action) => {
            if vm.is_function(value) {
                out.handlers.push((action, vm.root(value)));
            }
        }
        Meaning::Event => out.events |= vm.is_function(value),
        Meaning::Native => out.native |= value == Value::TRUE,
        Meaning::Number(name) => {
            if let Some(number) = number_of(value) {
                out.entries.push((name, PropValue::Float(number)));
            }
        }
        Meaning::Size(pixels, percent) => {
            if let Some(number) = number_of(value) {
                out.entries.push((pixels, PropValue::Float(number)));
            } else if let Some(text) = vm.string_text(value) {
                if let Some(Ok(number)) = text.strip_suffix('%').map(str::parse::<f64>) {
                    out.entries.push((percent, PropValue::Float(number)));
                } else if text.contains("% +") || crate::css::is_bounded(&text) {
                    // A share of what it's in and some pixels more, as CSS's `calc()` comes to, or those kept between a least and a
                    // most, as its `min()`, `max()` and `clamp()` do, which the layout reads as it is.
                    out.entries.push((pixels, PropValue::from(text.as_ref())));
                }
            }
        }
        Meaning::Padding | Meaning::PaddingX | Meaning::PaddingY => {
            if let Some(number) = number_of(value) {
                let names = &*SIDES;
                let mut side = |name: Symbol| {
                    out.entries.push((name, PropValue::Float(number)));
                };
                if !matches!(meaning, Meaning::PaddingY) {
                    side(names.left);
                    side(names.right);
                }
                if !matches!(meaning, Meaning::PaddingX) {
                    side(names.top);
                    side(names.bottom);
                }
            }
        }
        Meaning::Margin | Meaning::MarginX | Meaning::MarginY => {
            if let Some(number) = number_of(value) {
                let names = &*SIDES;
                let mut side = |name: Symbol| {
                    out.entries.push((name, PropValue::Float(number)));
                };
                if !matches!(meaning, Meaning::MarginY) {
                    side(names.margin_left);
                    side(names.margin_right);
                }
                if !matches!(meaning, Meaning::MarginX) {
                    side(names.margin_top);
                    side(names.margin_bottom);
                }
            }
        }
        Meaning::Flex => {
            if let Some(number) = number_of(value) {
                let names = &*SIDES;
                out.entries.push((names.grow, PropValue::Float(number)));
                out.entries.push((names.shrink, PropValue::Float(1.0)));
                out.entries.push((names.basis, PropValue::Float(0.0)));
            }
        }
        Meaning::Color(name) => {
            if let Some(color) = parse_color(vm, value) {
                out.entries.push((name, PropValue::Int(color)));
            }
        }
        Meaning::Plain(name) => {
            if let Some(plain) = plain_value(vm, value) {
                out.entries.push((name, plain));
            } else if vm.is_function(value) {
                // A list's `renderRow` is a function too, and that's read elsewhere.
                let name = vm.atom_name(atom);
                let mut after = name.strip_prefix("on").unwrap_or("").chars();
                if after.next().is_some_and(char::is_uppercase) {
                    return Err(no_such_event(vm, atom));
                }
            }
        }
    }
    Ok(())
}

/// What it says when a function's given to something that doesn't call it. Otherwise, nothing would
/// happen when that was meant to, with nothing to say why.
#[cold]
#[inline(never)]
fn no_such_event(vm: &mut Vm, atom: Atom) -> Value {
    let name = vm.atom_name(atom);
    vm.type_error(&format!(
        "{name} isn't something that happens, so this function would never be called. These are: \
         onClick, onDoubleClick, onContextMenu, onDrag, onFocus, onBlur, onToggle, onChange, onScroll, \
         onKeyPress, onResize, onDropFiles, onClose and onDismiss, and these, which are given events as \
         the web has them: onMouseEnter, onMouseLeave, onPointerEnter, onPointerLeave, onPointerDown, \
         onPointerUp, onPointerMove, onMouseDown, onMouseUp, onMouseMove, onWheel, onKeyDown, onKeyUp, \
         onAuxClick, onPaste, onCopy, onCut, onCompositionStart, onCompositionUpdate, onCompositionEnd, \
         onDragStart, onDragEnter, onDragOver, onDragLeave, onDrop, onDragEnd, \
         and onClickCapture and the other Capture forms."
    ))
}

/// A value as it can be held in a prop, or as it reaches a handler.
pub(crate) fn plain_value(vm: &Vm, value: Value) -> Option<PropValue> {
    if value.is_bool() {
        return Some(PropValue::Bool(value == Value::TRUE));
    }
    if value.is_int() {
        return Some(PropValue::Int(i64::from(value.as_int())));
    }
    if value.is_number() {
        return Some(PropValue::Float(value.as_number()));
    }
    vm.string_text(value).map(|text| shared_text(text.as_ref()))
}

/// A string prop's value, as one shared with the last element given the same text while it's short: each row of a long list
/// is given the same class names, which were a block for each and the text of them again (2-3 MB of the long list's).
fn shared_text(text: &str) -> PropValue {
    /// What's kept of texts given before: as many as this, after which it starts again, so a text given once (an element's
    /// key) is only kept until then.
    const MOST: usize = 8192;
    /// Longer text is rarely given again, and isn't kept.
    const LONGEST: usize = 256;
    struct Shared(PropValue);
    impl Shared {
        fn text(&self) -> &str {
            match &self.0 {
                PropValue::Str(text) => text.as_str(),
                _ => "",
            }
        }
    }
    impl std::hash::Hash for Shared {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.text().hash(state);
        }
    }
    impl PartialEq for Shared {
        fn eq(&self, other: &Self) -> bool {
            self.text() == other.text()
        }
    }
    impl Eq for Shared {}
    impl std::borrow::Borrow<str> for Shared {
        fn borrow(&self) -> &str {
            self.text()
        }
    }
    thread_local! {
        static SHARED: std::cell::RefCell<std::collections::HashSet<Shared, foldhash::fast::FixedState>> =
            std::cell::RefCell::new(std::collections::HashSet::with_hasher(foldhash::fast::FixedState::default()));
    }
    if text.len() > LONGEST {
        return PropValue::from(text);
    }
    SHARED.with(|shared| {
        let mut shared = shared.borrow_mut();
        if let Some(found) = shared.get(text) {
            return found.0.clone();
        }
        if shared.len() >= MOST {
            shared.clear();
        }
        let value = PropValue::from(text);
        shared.insert(Shared(value.clone()));
        value
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_turn_into_what_rsui_calls_them() {
        assert_eq!(snake_case("fontSize"), "font_size");
        assert_eq!(snake_case("rows"), "rows");
        assert_eq!(snake_case("alignSelf"), "align_self");
    }

    #[test]
    fn colors_are_read_in_all_their_forms() {
        let mut engine = rsui_js::Engine::new();
        let vm = engine.vm();
        let mut color = |text: &str| {
            let value = vm.new_string(text);
            parse_color(vm, value)
        };
        assert_eq!(color("#1f6feb"), Some(Color::hex(0x1F6FEB).to_prop()));
        assert_eq!(color("#fff"), Some(Color::WHITE.to_prop()));
        assert_eq!(
            color("#00000066"),
            Some(Color::rgba(0, 0, 0, 0x66).to_prop())
        );
        assert_eq!(color("white"), Some(Color::WHITE.to_prop()));
        assert_eq!(color("nonsense"), None);
        assert_eq!(color("#12"), None);
    }
}
