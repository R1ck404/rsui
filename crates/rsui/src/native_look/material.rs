//! Material 3, as Jetpack Compose's `material3` draws it: its baseline color scheme's roles, its type scale in Roboto, its
//! shapes, its state layers (8% under the pointer, 10% pressed, as a ripple) and its focus indicator.

use rsui_core::{PropValue, Symbol, symbol};
use rsui_render::Color;

use super::{Control, NativeControl, Out, alpha, check_mark, over, part};

/// The color roles that controls use, from Material's baseline scheme.
struct Scheme {
    primary: Color,
    on_primary: Color,
    secondary: Color,
    secondary_container: Color,
    on_secondary_container: Color,
    surface: Color,
    on_surface: Color,
    on_surface_variant: Color,
    outline: Color,
    surface_container_low: Color,
    surface_container_highest: Color,
}

const LIGHT: Scheme = Scheme {
    primary: Color::hex(0x6750A4),
    on_primary: Color::hex(0xFFFFFF),
    secondary: Color::hex(0x625B71),
    secondary_container: Color::hex(0xE8DEF8),
    on_secondary_container: Color::hex(0x1D192B),
    surface: Color::hex(0xFEF7FF),
    on_surface: Color::hex(0x1D1B20),
    on_surface_variant: Color::hex(0x49454F),
    outline: Color::hex(0x79747E),
    surface_container_low: Color::hex(0xF7F2FA),
    surface_container_highest: Color::hex(0xE6E0E9),
};

const DARK: Scheme = Scheme {
    primary: Color::hex(0xD0BCFF),
    on_primary: Color::hex(0x381E72),
    secondary: Color::hex(0xCCC2DC),
    secondary_container: Color::hex(0x4A4458),
    on_secondary_container: Color::hex(0xE8DEF8),
    surface: Color::hex(0x141218),
    on_surface: Color::hex(0xE6E0E9),
    on_surface_variant: Color::hex(0xCAC4D0),
    outline: Color::hex(0x938F99),
    surface_container_low: Color::hex(0x1D1B20),
    surface_container_highest: Color::hex(0x36343B),
};

fn scheme(native: &NativeControl) -> &'static Scheme {
    if native.dark { &DARK } else { &LIGHT }
}

/// Android's own type, which it has wherever it's running; elsewhere what's there instead.
const FONT: &str = "Roboto";

/// How strong a state layer is: under the pointer, and pressed (`StateTokens`).
const HOVERED: f32 = 0.08;
const PRESSED: f32 = 0.10;
/// How strong what can't be used is: its container, and what's on it.
const DISABLED_CONTAINER: f32 = 0.12;
const DISABLED_CONTENT: f32 = 0.38;

const BUTTON_VARIANTS: [&str; 5] = ["filled", "tonal", "outlined", "text", "elevated"];
const FIELD_VARIANTS: [&str; 2] = ["outlined", "filled"];

pub(super) fn variant(control: Control, said: Option<&str>) -> u8 {
    let list: &[&str] = match control {
        Control::Button => &BUTTON_VARIANTS,
        Control::Input => &FIELD_VARIANTS,
        _ => &[],
    };
    said.and_then(|said| list.iter().position(|one| *one == said)).unwrap_or(0) as u8
}

pub(super) fn style(native: &NativeControl, out: &mut Out<'_>) {
    let s = scheme(native);
    match native.control {
        Control::Button => button(native, s, out),
        Control::Input => field(native, s, out),
        Control::Checkbox | Control::Switch => {
            out.text(symbol!("direction"), "row");
            out.text(symbol!("align"), "center");
            out.number(symbol!("gap"), if native.control == Control::Switch { 12.0 } else { 4.0 });
            out.number(symbol!("min_height"), 48.0);
            out.color(symbol!("text_color"), content(native, s.on_surface));
            out.flag(symbol!("plain_disabled"));
            out.focus_ring(s.secondary, 3.0, 2.0, None);
            out.number(symbol!("corner_radius"), 4.0);
        }
        Control::Slider => {
            out.text(symbol!("direction"), "row");
            out.text(symbol!("align"), "center");
            out.number(symbol!("height"), 44.0);
            out.number(symbol!("min_width"), 120.0);
            out.flag(symbol!("plain_disabled"));
            out.focus_ring(s.secondary, 3.0, 2.0, None);
            out.number(symbol!("corner_radius"), 22.0);
        }
        Control::Progress => {
            out.text(symbol!("direction"), "row");
            out.number(symbol!("height"), 4.0);
            out.number(symbol!("corner_radius"), 2.0);
            out.color(symbol!("background"), s.secondary_container);
        }
        Control::Surface => {
            out.color(symbol!("background"), s.surface);
            out.color(symbol!("text_color"), s.on_surface);
            out.font(FONT, 14.0, 400, 20.0);
        }
    }
}

/// What's on a control, faded if it can't be used.
fn content(native: &NativeControl, color: Color) -> Color {
    if native.disabled { alpha(scheme(native).on_surface, DISABLED_CONTENT) } else { color }
}

/// Buttons (`ButtonTokens`): 40 high, round ends, Label Large, with a state layer of what's on them.
fn button(native: &NativeControl, s: &Scheme, out: &mut Out<'_>) {
    let none = Color::TRANSPARENT;
    let (container, label, layer, border) = match BUTTON_VARIANTS[usize::from(native.variant)] {
        "tonal" => (s.secondary_container, s.on_secondary_container, s.on_secondary_container, None),
        "outlined" => (none, s.primary, s.primary, Some(s.outline)),
        "text" => (none, s.primary, s.primary, None),
        "elevated" => (s.surface_container_low, s.primary, s.primary, None),
        _ => (s.primary, s.on_primary, s.on_primary, None),
    };
    let text_button = native.variant == 3;
    out.number(symbol!("min_height"), 40.0);
    out.number(symbol!("min_width"), 58.0);
    let across = if text_button { 12.0 } else { 24.0 };
    out.padding(across, 10.0, across, 10.0);
    out.number(symbol!("corner_radius"), 20.0);
    out.font(FONT, 14.0, 500, 20.0);
    out.number(symbol!("letter_spacing"), 0.1);
    out.text(symbol!("text_align"), "center");
    out.flag(symbol!("no_wrap"));
    out.flag(symbol!("plain_disabled"));
    out.focus_ring(s.secondary, 3.0, 2.0, None);
    if native.disabled {
        if container.a > 0 {
            out.color(symbol!("background"), alpha(s.on_surface, DISABLED_CONTAINER));
        }
        if border.is_some() {
            border_all(out, alpha(s.on_surface, DISABLED_CONTAINER), 1.0);
        }
        out.color(symbol!("text_color"), alpha(s.on_surface, DISABLED_CONTENT));
        return;
    }
    if container.a > 0 {
        out.color(symbol!("background"), container);
    }
    out.color(symbol!("text_color"), label);
    out.color(symbol!("hover_background"), over(container, layer, HOVERED));
    out.color(symbol!("pressed_background"), over(container, layer, PRESSED));
    out.color(symbol!("ripple"), alpha(layer, PRESSED));
    if let Some(border) = border {
        border_all(out, border, 1.0);
    }
    if native.variant == 4 {
        // Level 1 of Material's elevation, as a shadow.
        out.color(symbol!("shadow_color"), Color::rgba(0, 0, 0, 0x4D));
        out.number(symbol!("shadow_blur"), 3.0);
        out.number(symbol!("shadow_y"), 1.0);
    }
}

fn border_all(out: &mut Out<'_>, color: Color, width: f64) {
    out.color(symbol!("border_color"), color);
    out.number(symbol!("border_width"), width);
}

/// Text fields (`OutlinedTextFieldTokens`, `FilledTextFieldTokens`): 56 high, Body Large, a line of the primary color while
/// they're typed in, and no label above them yet: the placeholder is what's said of them.
fn field(native: &NativeControl, s: &Scheme, out: &mut Out<'_>) {
    let filled = native.variant == 1;
    out.number(symbol!("min_height"), 56.0);
    out.padding(16.0, 16.0, 16.0, 16.0);
    out.number(symbol!("corner_radius"), 4.0);
    out.font(FONT, 16.0, 400, 24.0);
    out.number(symbol!("letter_spacing"), 0.5);
    out.flag(symbol!("plain_disabled"));
    out.no_focus_ring();
    out.color(symbol!("text_color"), content(native, s.on_surface));
    out.color(symbol!("placeholder_color"), content(native, s.on_surface_variant));
    if filled {
        let container = if native.disabled { alpha(s.on_surface, 0.04) } else { s.surface_container_highest };
        out.color(symbol!("background"), container);
        out.number(symbol!("border_bottom_width"), 1.0);
        out.color(symbol!("border_bottom_color"), content(native, s.on_surface_variant));
        if !native.disabled {
            out.color(symbol!("hover_background"), over(container, s.on_surface, HOVERED));
            out.focus_line("bottom", s.primary, 2.0);
        }
    } else {
        border_all(out, if native.disabled { alpha(s.on_surface, DISABLED_CONTAINER) } else { s.outline }, 1.0);
        if !native.disabled {
            out.focus_line("all", s.primary, 2.0);
        }
    }
}

pub(super) fn label(native: &NativeControl) -> Vec<(Symbol, PropValue)> {
    let s = scheme(native);
    let mut props = Vec::with_capacity(6);
    let mut out = Out(&mut props);
    out.color(symbol!("text_color"), content(native, s.on_surface));
    out.font(FONT, 14.0, 400, 20.0);
    out.number(symbol!("letter_spacing"), 0.25);
    out.into_vec()
}

pub(super) fn parts(native: &NativeControl) -> Vec<rsui_core::Element> {
    let s = scheme(native);
    match native.control {
        Control::Checkbox => vec![checkbox(native, s)],
        Control::Switch => vec![switch(native, s)],
        Control::Slider => slider(native, s),
        Control::Progress => vec![part(native, 0, || {
            let mut props = Vec::new();
            let mut out = Out(&mut props);
            out.number(symbol!("width_percent"), f64::from(native.fraction) * 100.0);
            out.number(symbol!("height"), 4.0);
            out.number(symbol!("corner_radius"), 2.0);
            out.color(symbol!("background"), s.primary);
            out.into_vec()
        })],
        _ => Vec::new(),
    }
}

/// A checkbox (`CheckboxTokens`): an 18 box with round corners 2, its outline 2 wide until it's ticked and filled, in a circle
/// 40 across that's its state layer.
fn checkbox(native: &NativeControl, s: &Scheme) -> rsui_core::Element {
    let ticked = native.checked;
    let mut held = part(native, 1, || {
        let mut props = Vec::new();
        let mut out = Out(&mut props);
        out.size(18.0, 18.0);
        out.number(symbol!("corner_radius"), 2.0);
        out.text(symbol!("align"), "center");
        out.text(symbol!("justify"), "center");
        match (ticked, native.disabled) {
            (true, false) => out.color(symbol!("background"), s.primary),
            (true, true) => out.color(symbol!("background"), alpha(s.on_surface, DISABLED_CONTENT)),
            (false, false) => border_all(&mut out, s.on_surface_variant, 2.0),
            (false, true) => border_all(&mut out, alpha(s.on_surface, DISABLED_CONTENT), 2.0),
        }
        out.into_vec()
    });
    if ticked {
        let color = if native.disabled { s.surface } else { s.on_primary };
        let mark = rsui_core::Element::new(rsui_core::Role::Image)
            .props(super::part_props(native, 2, || check_mark(18.0, "0 0 24 24", "M4.8 12 L9.6 16.8 L19.2 7.2", 2.67, color)));
        held = held.child(mark);
    }
    part(native, 3, || {
        let mut props = Vec::new();
        let mut out = Out(&mut props);
        out.size(40.0, 40.0);
        out.number(symbol!("corner_radius"), 20.0);
        out.text(symbol!("align"), "center");
        out.text(symbol!("justify"), "center");
        if !native.disabled {
            let layer = if ticked { s.primary } else { s.on_surface };
            out.color(symbol!("hover_background"), alpha(layer, HOVERED));
        }
        out.into_vec()
    })
    .child(held)
}

/// A switch (`SwitchTokens`): a track 52 by 32, round, whose handle is 16 across when it's off and 24 when it's on, sliding
/// over and growing as it changes.
fn switch(native: &NativeControl, s: &Scheme) -> rsui_core::Element {
    let on = native.checked;
    let handle = part(native, 4, || {
        let mut props = Vec::new();
        let mut out = Out(&mut props);
        out.size(24.0, 24.0);
        out.number(symbol!("corner_radius"), 12.0);
        out.number(symbol!("margin_left"), 4.0);
        let color = match (on, native.disabled) {
            (true, false) => s.on_primary,
            (false, false) => s.outline,
            (true, true) => s.surface,
            (false, true) => alpha(s.on_surface, DISABLED_CONTENT),
        };
        out.color(symbol!("background"), color);
        out.text(symbol!("transform"), if on { "translateX(20px) scale(1)" } else { "translateX(0px) scale(0.6667)" });
        out.text(symbol!("transitions"), "transform s 0.25 0 cubic-bezier(0.2, 0, 0, 1)\nbackground c 0.25 0 linear");
        out.into_vec()
    });
    part(native, 5, || {
        let mut props = Vec::new();
        let mut out = Out(&mut props);
        out.size(52.0, 32.0);
        out.number(symbol!("corner_radius"), 16.0);
        out.text(symbol!("direction"), "row");
        out.text(symbol!("align"), "center");
        out.number(symbol!("shrink"), 0.0);
        match (on, native.disabled) {
            (true, false) => out.color(symbol!("background"), s.primary),
            (true, true) => out.color(symbol!("background"), alpha(s.on_surface, DISABLED_CONTAINER)),
            (false, disabled) => {
                out.color(symbol!("background"), if disabled { alpha(s.surface_container_highest, DISABLED_CONTAINER) } else { s.surface_container_highest });
                border_all(&mut out, if disabled { alpha(s.on_surface, DISABLED_CONTAINER) } else { s.outline }, 2.0);
            }
        }
        out.text(symbol!("transitions"), "background c 0.25 0 linear");
        out.into_vec()
    })
    .child(handle)
}

/// A slider (`SliderTokens`): a track 4 high, of the primary color up to a handle 20 across and of the secondary container
/// after it.
fn slider(native: &NativeControl, s: &Scheme) -> Vec<rsui_core::Element> {
    let along = f64::from(native.fraction);
    let (active, inactive, handle) = if native.disabled {
        (alpha(s.on_surface, DISABLED_CONTENT), alpha(s.on_surface, DISABLED_CONTAINER), alpha(s.on_surface, DISABLED_CONTENT))
    } else {
        (s.primary, s.secondary_container, s.primary)
    };
    let track = |share: f64, color: Color, number: u32| {
        part(native, number, || {
            let mut props = Vec::new();
            let mut out = Out(&mut props);
            out.number(symbol!("grow"), share);
            out.number(symbol!("shrink"), 1.0);
            out.number(symbol!("basis"), 0.0);
            out.number(symbol!("height"), 4.0);
            out.number(symbol!("corner_radius"), 2.0);
            out.color(symbol!("background"), color);
            out.into_vec()
        })
    };
    let thumb = part(native, 7, || {
        let mut props = Vec::new();
        let mut out = Out(&mut props);
        out.size(20.0, 20.0);
        out.number(symbol!("shrink"), 0.0);
        out.number(symbol!("corner_radius"), 10.0);
        out.color(symbol!("background"), handle);
        if !native.disabled {
            out.color(symbol!("shadow_color"), Color::rgba(0, 0, 0, 0x4D));
            out.number(symbol!("shadow_blur"), 2.0);
            out.number(symbol!("shadow_y"), 1.0);
        }
        out.into_vec()
    });
    vec![track(along, active, 6), thumb, track(1.0 - along, inactive, 8)]
}
