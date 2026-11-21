//! `native` on rsui's own elements: `<button native>`, `<input native>`, `<checkbox native>`, `<switch native>`,
//! `<slider native>`, `<progress native>` and `<window native>` are drawn in the platform's look (`rsui::native_look`), as
//! any element is, from props. What the app says of how one looks (`className`, colors, borders, corners, shadows, type) is
//! left out, which a dev build says once for each; where it goes and how big it is stays. What's said for one platform
//! (`android={{ variant: 'tonal' }}`, `windows={{ variant: 'accent' }}`) is read for the look that platform has, and left
//! alone by the others.
//!
//! A checkbox or switch says it's changed with `onChange(checked)`, and a slider with `onChange(value)`, which this wires up:
//! a slider is dragged, clicked along, or moved with the arrow keys, Home, End and Page Up and Down.

use std::cell::RefCell;
use std::collections::HashSet;

use rsui::core::{ActionKind, Element, PropValue, Symbol, symbol};
use rsui::native_look::{self as look, Control, NativeControl, Theme};
use rsui::runtime::{Cx, Updates};
use rsui_js::{Args, HostFunction, Root, Value, Vm};

use crate::props::Gathered;

thread_local! {
    /// What's been warned about, by tag and prop, so each is said once.
    static WARNED: RefCell<HashSet<(&'static str, &'static str)>> = RefCell::new(HashSet::new());
}

/// Says once, in a dev build, that something said of a native control is left out.
fn warn(tag: &'static str, what: &'static str) {
    if rsui_js::running_packed() {
        return;
    }
    let first = WARNED.with(|warned| warned.borrow_mut().insert((tag, what)));
    if first {
        eprintln!(
            "rsui: <{tag} native> leaves out `{what}`: the platform decides how a native control looks. Where it goes and how big \
             it is (margin, width, height, flex, alignment, position) are kept; for a look of your own, leave `native` off."
        );
    }
}

/// The name of the prop a platform's extras are in, for the look that platform has.
fn extras_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Material => "android",
        Theme::Fluent => "windows",
    }
}

/// An element that says `native`, with this tag: what it's said for the look's platform is read, and its look put on its props.
/// Nothing's done for a tag that has no native look.
pub(crate) fn restyle(vm: &mut Vm, tag: &str, props: Value, gathered: &mut Gathered) -> Result<Option<NativeControl>, Value> {
    let Some(control) = Control::of_tag(tag) else {
        if !rsui_js::running_packed() {
            WARNED.with(|warned| {
                if warned.borrow_mut().insert(("", "")) {
                    eprintln!("rsui: <{tag} native> has no native look: it's button, input, checkbox, switch, slider, progress and window that do");
                }
            });
        }
        return Ok(None);
    };
    let extras = vm.atom(extras_name(look::theme()));
    let extras = if props.is_object() { vm.get_property(props, extras)? } else { Value::UNDEFINED };
    let variant = if extras.is_object() {
        let name = vm.atom("variant");
        let said = vm.get_property(extras, name)?;
        vm.string_text(said).map(|text| text.into_owned())
    } else {
        None
    };
    let tag = control.tag();
    let native = look::restyle(control, variant.as_deref(), &mut gathered.entries, |name| {
        warn(tag, if name.as_str() == "class_name" { "className" } else { name.as_str() });
    });
    Ok(Some(native))
}

/// Says once that a native control's `className` is left out.
pub(crate) fn class_name_left_out(tag: &str) {
    if let Some(control) = Control::of_tag(tag) {
        warn(control.tag(), "className");
    }
}

/// A native control's parts put in, and what it says it's changed with wired up, before what's in it is put in.
pub(crate) fn finish(cx: &mut Cx<'_>, native: &NativeControl, element: Element, handlers: &mut Vec<(ActionKind, Root)>, has_children: bool) -> Element {
    let mut element = look::finish(native, element, has_children);
    let changed = handlers
        .iter()
        .position(|(action, _)| *action == ActionKind::SetValue)
        .filter(|_| matches!(native.control, Control::Checkbox | Control::Switch | Control::Slider))
        .map(|at| handlers.remove(at).1);
    match native.control {
        Control::Checkbox | Control::Switch => {
            if let Some(changed) = changed {
                let next = !native.checked();
                element = cx.on(element, ActionKind::Toggle, move |updates, _| {
                    crate::bridge::call_with(updates, &changed, Value::bool(next));
                });
            }
        }
        Control::Slider => element = slider(cx, native, element, changed),
        _ => {}
    }
    element
}

/// How a slider's dragged, clicked and moved with keys, telling `changed` each new value.
fn slider(cx: &mut Cx<'_>, native: &NativeControl, element: Element, changed: Option<Root>) -> Element {
    let number = |name: Symbol, otherwise: f64| match element.props.get_symbol(name) {
        Some(PropValue::Float(number)) => *number,
        Some(PropValue::Int(number)) => *number as f64,
        _ => otherwise,
    };
    let (min, max) = (number(symbol!("min"), 0.0), number(symbol!("max"), 100.0));
    let step = number(symbol!("step"), 1.0);
    let value = min + f64::from(native.fraction()) * (max - min);
    // It takes the keyboard, and is pressed, as a button is.
    let element = cx.on(element, ActionKind::Click, |_, _| {});
    let Some(changed) = changed else {
        return element;
    };
    if native.disabled() {
        return element;
    }
    let settle = move |next: f64| {
        let stepped = if step > 0.0 { ((next - min) / step).round() * step + min } else { next };
        stepped.clamp(min.min(max), max.max(min))
    };
    let thumb = match look::theme() {
        Theme::Material | Theme::Fluent => 20.0,
    };
    let dragged = changed.clone();
    let element = cx.on(element, ActionKind::Custom(symbol!("drag")), move |updates, input| {
        let Some(PropValue::Str(said)) = input else { return };
        let words: Vec<&str> = said.split(' ').collect();
        let (Some(phase), Some(x), Some(width)) =
            (words.first(), words.get(1).and_then(|x| x.parse::<f64>().ok()), words.get(5).and_then(|w| w.parse::<f64>().ok()))
        else {
            return;
        };
        if !matches!(*phase, "start" | "move") || width <= thumb {
            return;
        }
        let next = settle(min + (x - thumb / 2.0) / (width - thumb) * (max - min));
        if (next - value).abs() > f64::EPSILON {
            crate::bridge::call_with(updates, &dragged, Value::number(next));
        }
    });
    cx.on(element, ActionKind::KeyPress, move |updates: &mut Updates<'_>, input: Option<&PropValue>| {
        let Some(PropValue::Str(key)) = input else {
            updates.pass_on();
            return;
        };
        let page = ((max - min) / 10.0).max(step);
        let next = match key.as_str() {
            "left" | "down" => value - step,
            "right" | "up" => value + step,
            "pageup" => value + page,
            "pagedown" => value - page,
            "home" => min,
            "end" => max,
            _ => {
                updates.pass_on();
                return;
            }
        };
        let next = settle(next);
        if (next - value).abs() > f64::EPSILON {
            crate::bridge::call_with(updates, &changed, Value::number(next));
        }
    })
}

/// `nativeLookDark(dark)`: the system's gone dark or light, which native controls follow. Says whether any have been drawn,
/// which then have to be made again.
fn host_native_look_dark(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let dark = args.get(vm, 0) == Value::TRUE;
    Ok(Value::bool(look::set_dark(dark)))
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 1] = [("nativeLookDark", host_native_look_dark)];

