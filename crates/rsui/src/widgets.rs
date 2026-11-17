//! Ready-made elements, built from what any program could use itself.

use std::time::Duration;

use rsui_core::{ActionKind, Element, PropValue, Props, Role, Text};
use rsui_layout::{Align, Justify, Placement, Style, style};
use rsui_render::{Color, Paint};
use rsui_runtime::{Cx, State, Updates};

/// How long a popup takes to fade in.
const POPUP_FADE: Duration = Duration::from_millis(120);
/// How long it takes to fade out. Going away has to get out of the way sooner than arriving does.
const POPUP_FADE_OUT: Duration = Duration::from_millis(90);

/// How far a popup is through coming or going, from 0 to 1.
fn popup_shown(cx: &mut Cx<'_>, is_open: bool) -> f64 {
    if is_open {
        cx.animate(1.0, POPUP_FADE)
    } else {
        cx.animate(0.0, POPUP_FADE_OUT)
    }
}

/// Makes something this see-through, unless it's not at all, which saves it being looked at.
fn faded(element: Element, opacity: f64) -> Element {
    if opacity < 0.999 {
        element.prop("opacity", opacity)
    } else {
        element
    }
}

/// What each item in a menu does when it's chosen.
pub type MenuAction = Box<dyn FnMut(&mut Updates<'_>)>;

/// Something that takes over the window while `open` is true, with the rest dimmed behind it. It has
/// the keyboard to itself, which goes back to where it was afterwards. Clicking away from it or
/// pressing escape closes it. `None` while it's closed, which fits straight into `children`. It fades
/// out, but what's behind it can be used from the moment it's closed.
///
/// `name` says what it is, to agents and screen readers.
pub fn dialog(
    cx: &mut Cx<'_>,
    key: &str,
    name: impl Into<Text>,
    open: State<bool>,
    content: Element,
) -> Option<Element> {
    let is_open = cx.get(open);
    let shown = popup_shown(cx, is_open);
    if !is_open && shown <= 0.0 {
        return None;
    }
    let panel = Element::new(Role::Group)
        .key("panel")
        .props(style!(
            Style::new().padding(20.0).gap(12.0).max_width(560.0),
            Paint::new()
                .background(Color::WHITE)
                .corner_radius(10.0)
                .shadow(Color::rgba(0, 0, 0, 0x33), 24.0, 8.0),
        ))
        .child(content);
    let backdrop = Element::new(Role::Dialog)
        .key(key)
        .name(name)
        .props(style!(
            Style::new()
                .overlay(Placement::Fill)
                .align(Align::Center)
                .justify(Justify::Center),
            Paint::new().background(Color::rgba(0, 0, 0, 0x66)),
        ))
        .child(panel);
    let backdrop = faded(backdrop, shown);
    if !is_open {
        return Some(backdrop.prop("inert", true));
    }
    Some(cx.on(backdrop, ActionKind::Dismiss, move |cx, _| {
        cx.set(open, false)
    }))
}

/// A button that opens a menu below it, or above if there's more room there. `open` says whether it's
/// open, and choosing an item closes it. The arrow keys go through the items, and pressing enter or
/// clicking away, or escape, does what you'd expect.
pub fn menu_button(
    cx: &mut Cx<'_>,
    key: &str,
    label: impl Into<Text>,
    open: State<bool>,
    items: Vec<(Text, MenuAction)>,
) -> Element {
    let button = Element::new(Role::Button)
        .key(format!("{key}-button"))
        .name(label)
        .props(style!(
            Style::new().padding_xy(12.0, 6.0),
            Paint::new()
                .background(Color::hex(0xD8DEE4))
                .hover(Color::hex(0xC9D1D9))
                .pressed(Color::hex(0xAFB8C1))
                .corner_radius(6.0),
        ));
    let button = cx.on(button, ActionKind::Click, move |cx, _| {
        cx.update(open, |open| *open = !*open);
    });
    let mut holder = Element::new(Role::Group)
        .key(key)
        .props(style!(Style::new().align(Align::Start)))
        .child(button);
    let is_open = cx.get(open);
    let shown = popup_shown(cx, is_open);
    if is_open || shown > 0.0 {
        let mut menu = Element::new(Role::Menu)
            .key(format!("{key}-menu"))
            .props(style!(
                Style::new()
                    .overlay(Placement::Below)
                    .padding(4.0)
                    .min_width(120.0),
                Paint::new()
                    .background(Color::WHITE)
                    .border(Color::hex(0xD0D7DE), 1.0)
                    .corner_radius(8.0)
                    .shadow(Color::rgba(0, 0, 0, 0x26), 14.0, 4.0),
            ));
        for (index, (name, mut action)) in items.into_iter().enumerate() {
            let item = Element::new(Role::MenuItem)
                .key(format!("{key}-item-{index}"))
                .name(name)
                .props(style!(
                    Style::new().padding_xy(10.0, 6.0),
                    Paint::new().hover(Color::hex(0xEAEEF2)).corner_radius(4.0),
                ));
            menu = menu.child(cx.on(item, ActionKind::Click, move |cx, _| {
                cx.set(open, false);
                action(cx);
            }));
        }
        let menu = faded(menu, shown);
        let menu = if is_open {
            cx.on(menu, ActionKind::Dismiss, move |cx, _| cx.set(open, false))
        } else {
            menu.prop("inert", true)
        };
        holder = holder.child(menu);
    }
    holder
}

/// A button showing which of `options` is chosen, which opens a menu of them.
pub fn select(cx: &mut Cx<'_>, key: &str, options: &[&str], chosen: State<usize>) -> Element {
    let open = cx.state(|| false);
    let label = options.get(cx.get(chosen)).copied().unwrap_or("");
    let items = options
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let action: MenuAction = Box::new(move |cx| cx.set(chosen, index));
            (Text::from(*option), action)
        })
        .collect();
    menu_button(cx, key, label, open, items)
}

/// Several lines of editable text, showing and changing `value`, `rows` lines high unless its style
/// says otherwise. Otherwise, this is what [`text_input`] is: `rows` is what gives it several lines.
/// Enter starts a new line, the arrow keys go between them, and what doesn't fit scrolls.
pub fn text_area(
    cx: &mut Cx<'_>,
    key: &str,
    name: impl Into<Text>,
    hint: &str,
    value: State<Text>,
    rows: u32,
) -> Element {
    text_input(cx, key, name, hint, value).prop("rows", i64::from(rows))
}

/// An image from a file, or from what was given to [`register_image`](rsui_render::register_image).
/// `description` says what it shows, to agents and screen readers.
///
/// It's shown at its own size unless its style says otherwise, keeping its shape. Its `fit` prop can
/// change how it fills a different size: `"contain"` shows all of it, which is the default, `"cover"`
/// fills the space, cutting off what doesn't fit, and `"fill"` stretches it. It's read on another
/// thread, at the size it's shown, and shows up when that's done.
pub fn image(source: &str, description: impl Into<Text>, style: impl Into<Props>) -> Element {
    Element::new(Role::Image)
        .name(description)
        .props(style)
        .prop("src", source)
}

/// One line of editable text, showing and changing `value`. `name` says what it's for, to agents
/// and screen readers, and `hint` shows while it's empty.
///
/// This works like any other element, so it's easy to make your own. A
/// [`Role::TextInput`] shows its `value` prop, and clicking it or pressing tab lets someone edit
/// it. Each edit reaches its [`ActionKind::SetValue`] handler, which decides what happens. An
/// agent changes it in the same way, and one without a handler can't be edited.
pub fn text_input(
    cx: &mut Cx<'_>,
    key: &str,
    name: impl Into<Text>,
    hint: &str,
    value: State<Text>,
) -> Element {
    let element = Element::new(Role::TextInput)
        .key(key)
        .name(name)
        .props(style!(
            Style::new().padding_xy(10.0, 6.0),
            Paint::new()
                .background(Color::WHITE)
                .border(Color::hex(0xD0D7DE), 1.0)
                .corner_radius(6.0),
        ))
        .prop("value", cx.get(value))
        .prop("placeholder", hint);
    cx.on(element, ActionKind::SetValue, move |cx, input| {
        if let Some(PropValue::Str(text)) = input {
            cx.set(value, Text::clone(text));
        }
    })
}
