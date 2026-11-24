//! Fluent, as WinUI 3 draws Windows 11's controls: its theme resources' colors (`Common_themeresources_any.xaml`), Segoe UI
//! Variable at 14, corners of 4, the user's accent color, and the focus visual of two rings.

use rsui_core::{PropValue, Symbol, symbol};
use rsui_render::Color;

use super::{Control, NativeControl, Out, alpha, argb, check_mark, part};

/// The theme resources controls use, light or dark.
struct Tokens {
    text_primary: Color,
    text_secondary: Color,
    text_disabled: Color,
    text_on_accent: Color,
    text_on_accent_secondary: Color,
    text_on_accent_disabled: Color,
    control_fill: Color,
    control_fill_secondary: Color,
    control_fill_tertiary: Color,
    control_fill_disabled: Color,
    control_fill_input_active: Color,
    control_stroke: Color,
    control_stroke_secondary: Color,
    control_strong_stroke: Color,
    control_strong_stroke_disabled: Color,
    control_alt_fill_secondary: Color,
    control_alt_fill_tertiary: Color,
    control_strong_fill: Color,
    control_strong_fill_disabled: Color,
    control_solid_fill: Color,
    subtle_fill_secondary: Color,
    subtle_fill_tertiary: Color,
    accent_fill_disabled: Color,
    on_accent_stroke: Color,
    on_accent_stroke_secondary: Color,
    focus_outer: Color,
    focus_inner: Color,
    background: Color,
}

const LIGHT: Tokens = Tokens {
    text_primary: argb(0xE400_0000),
    text_secondary: argb(0x9E00_0000),
    text_disabled: argb(0x5C00_0000),
    text_on_accent: argb(0xFFFF_FFFF),
    text_on_accent_secondary: argb(0xB3FF_FFFF),
    text_on_accent_disabled: argb(0xFFFF_FFFF),
    control_fill: argb(0xB3FF_FFFF),
    control_fill_secondary: argb(0x80F9_F9F9),
    control_fill_tertiary: argb(0x4DF9_F9F9),
    control_fill_disabled: argb(0x4DF9_F9F9),
    control_fill_input_active: argb(0xFFFF_FFFF),
    control_stroke: argb(0x0F00_0000),
    control_stroke_secondary: argb(0x2900_0000),
    control_strong_stroke: argb(0x7200_0000),
    control_strong_stroke_disabled: argb(0x3700_0000),
    control_alt_fill_secondary: argb(0x0600_0000),
    control_alt_fill_tertiary: argb(0x0F00_0000),
    control_strong_fill: argb(0x7200_0000),
    control_strong_fill_disabled: argb(0x5100_0000),
    control_solid_fill: argb(0xFFFF_FFFF),
    subtle_fill_secondary: argb(0x0900_0000),
    subtle_fill_tertiary: argb(0x0600_0000),
    accent_fill_disabled: argb(0x3700_0000),
    on_accent_stroke: argb(0x14FF_FFFF),
    on_accent_stroke_secondary: argb(0x6600_0000),
    focus_outer: argb(0xE400_0000),
    focus_inner: argb(0xB3FF_FFFF),
    background: argb(0xFFF3_F3F3),
};

const DARK: Tokens = Tokens {
    text_primary: argb(0xFFFF_FFFF),
    text_secondary: argb(0xC5FF_FFFF),
    text_disabled: argb(0x5DFF_FFFF),
    text_on_accent: argb(0xFF00_0000),
    text_on_accent_secondary: argb(0x8000_0000),
    text_on_accent_disabled: argb(0x87FF_FFFF),
    control_fill: argb(0x0FFF_FFFF),
    control_fill_secondary: argb(0x15FF_FFFF),
    control_fill_tertiary: argb(0x08FF_FFFF),
    control_fill_disabled: argb(0x0BFF_FFFF),
    control_fill_input_active: argb(0xB31E_1E1E),
    control_stroke: argb(0x12FF_FFFF),
    control_stroke_secondary: argb(0x18FF_FFFF),
    control_strong_stroke: argb(0x8BFF_FFFF),
    control_strong_stroke_disabled: argb(0x28FF_FFFF),
    control_alt_fill_secondary: argb(0x1900_0000),
    control_alt_fill_tertiary: argb(0x0BFF_FFFF),
    control_strong_fill: argb(0x8BFF_FFFF),
    control_strong_fill_disabled: argb(0x3FFF_FFFF),
    control_solid_fill: argb(0xFF45_4545),
    subtle_fill_secondary: argb(0x0FFF_FFFF),
    subtle_fill_tertiary: argb(0x0AFF_FFFF),
    accent_fill_disabled: argb(0x28FF_FFFF),
    on_accent_stroke: argb(0x14FF_FFFF),
    on_accent_stroke_secondary: argb(0x2300_0000),
    focus_outer: argb(0xFFFF_FFFF),
    focus_inner: argb(0xB300_0000),
    background: argb(0xFF20_2020),
};

fn tokens(native: &NativeControl) -> &'static Tokens {
    if native.dark { &DARK } else { &LIGHT }
}

/// Windows 11's type for what's in controls: Segoe UI Variable at its size for text.
const FONT: &str = "Segoe UI Variable Text";

/// The accent fill controls use: the user's accent color a shade darker on a light background and two lighter on a dark
/// one, as Windows works them out (`SystemAccentColorDark1`, `SystemAccentColorLight2`), or Windows' own blue if it doesn't
/// say. `RSUI_ACCENT` says which instead, as for pictures that have to come out the same anywhere: `#rrggbb` for both, or
/// `default` for Windows' own blue.
fn accent(native: &NativeControl) -> Color {
    static PALETTE: std::sync::OnceLock<Option<[[u8; 4]; 8]>> = std::sync::OnceLock::new();
    let palette = PALETTE.get_or_init(|| match std::env::var("RSUI_ACCENT") {
        Ok(said) if said == "default" => None,
        Ok(said) => Color::from_text(&said).map(|color| [[color.r, color.g, color.b, 255]; 8]),
        Err(_) => crate::system::accent_palette(),
    });
    let at = if native.dark { 1 } else { 4 };
    match palette {
        Some(palette) => {
            let [r, g, b, _] = palette[at];
            Color::rgba(r, g, b, 255)
        }
        None if native.dark => Color::hex(0x60CDFF),
        None => Color::hex(0x005FB8),
    }
}

const BUTTON_VARIANTS: [&str; 3] = ["standard", "accent", "subtle"];

pub(super) fn variant(control: Control, said: Option<&str>) -> u8 {
    let list: &[&str] = match control {
        Control::Button => &BUTTON_VARIANTS,
        _ => &[],
    };
    said.and_then(|said| list.iter().position(|one| *one == said)).unwrap_or(0) as u8
}

fn focus_visual(t: &Tokens, out: &mut Out<'_>) {
    // `FocusVisualPrimaryThickness` 2 outside a `FocusVisualSecondaryThickness` of 1, as WinUI draws it around what has the
    // keyboard.
    out.focus_ring(t.focus_outer, 2.0, 1.0, Some((t.focus_inner, 1.0)));
}

pub(super) fn style(native: &NativeControl, out: &mut Out<'_>) {
    let t = tokens(native);
    match native.control {
        Control::Button => button(native, t, out),
        Control::Input => text_box(native, t, out),
        Control::Checkbox | Control::Switch => {
            out.text(symbol!("direction"), "row");
            out.text(symbol!("align"), "center");
            out.number(symbol!("gap"), if native.control == Control::Switch { 12.0 } else { 8.0 });
            out.number(symbol!("min_height"), 32.0);
            out.color(symbol!("text_color"), if native.disabled { t.text_disabled } else { t.text_primary });
            out.flag(symbol!("plain_disabled"));
            out.number(symbol!("corner_radius"), 4.0);
            focus_visual(t, out);
        }
        Control::Slider => {
            out.text(symbol!("direction"), "row");
            out.text(symbol!("align"), "center");
            out.number(symbol!("height"), 32.0);
            out.number(symbol!("min_width"), 120.0);
            out.flag(symbol!("plain_disabled"));
            out.number(symbol!("corner_radius"), 4.0);
            focus_visual(t, out);
        }
        Control::Progress => {
            out.text(symbol!("direction"), "row");
            out.text(symbol!("align"), "center");
            out.number(symbol!("height"), 3.0);
        }
        Control::Surface => {
            out.color(symbol!("background"), t.background);
            out.color(symbol!("text_color"), t.text_primary);
            out.font(FONT, 14.0, 400, 20.0);
        }
    }
}

fn border(out: &mut Out<'_>, color: Color, bottom: Color) {
    out.color(symbol!("border_color"), color);
    out.number(symbol!("border_width"), 1.0);
    if bottom != color {
        out.color(symbol!("border_bottom_color"), bottom);
    }
}

/// Buttons (`DefaultButtonStyle`, `AccentButtonStyle`, and a subtle one as a command bar's): 32 high, corners of 4, a
/// border darker along the bottom, as if lit from above, and text that fades while it's pressed.
fn button(native: &NativeControl, t: &Tokens, out: &mut Out<'_>) {
    out.number(symbol!("min_height"), 32.0);
    out.padding(11.0, 5.0, 11.0, 6.0);
    out.number(symbol!("corner_radius"), 4.0);
    out.font(FONT, 14.0, 400, 20.0);
    out.text(symbol!("text_align"), "center");
    out.flag(symbol!("no_wrap"));
    out.flag(symbol!("plain_disabled"));
    focus_visual(t, out);
    match (BUTTON_VARIANTS[usize::from(native.variant)], native.disabled) {
        ("accent", false) => {
            let accent = accent(native);
            out.color(symbol!("background"), accent);
            out.color(symbol!("hover_background"), alpha(accent, 0.9));
            out.color(symbol!("pressed_background"), alpha(accent, 0.8));
            out.color(symbol!("text_color"), t.text_on_accent);
            out.color(symbol!("pressed_text_color"), t.text_on_accent_secondary);
            border(out, t.on_accent_stroke, t.on_accent_stroke_secondary);
        }
        ("accent", true) => {
            out.color(symbol!("background"), t.accent_fill_disabled);
            out.color(symbol!("text_color"), t.text_on_accent_disabled);
        }
        ("subtle", disabled) => {
            out.color(symbol!("text_color"), if disabled { t.text_disabled } else { t.text_primary });
            if !disabled {
                out.color(symbol!("hover_background"), t.subtle_fill_secondary);
                out.color(symbol!("pressed_background"), t.subtle_fill_tertiary);
                out.color(symbol!("pressed_text_color"), t.text_secondary);
            }
        }
        (_, false) => {
            out.color(symbol!("background"), t.control_fill);
            out.color(symbol!("hover_background"), t.control_fill_secondary);
            out.color(symbol!("pressed_background"), t.control_fill_tertiary);
            out.color(symbol!("text_color"), t.text_primary);
            out.color(symbol!("pressed_text_color"), t.text_secondary);
            border(out, t.control_stroke, t.control_stroke_secondary);
        }
        (_, true) => {
            out.color(symbol!("background"), t.control_fill_disabled);
            out.color(symbol!("text_color"), t.text_disabled);
            border(out, t.control_stroke, t.control_stroke);
        }
    }
}

/// A text box (`DefaultTextBoxStyle`): 32 high, with a darker line along the bottom that's the accent color and 2 thick while
/// it's typed in, when it goes solid.
fn text_box(native: &NativeControl, t: &Tokens, out: &mut Out<'_>) {
    out.number(symbol!("min_height"), 32.0);
    out.padding(10.0, 5.0, 6.0, 6.0);
    out.number(symbol!("corner_radius"), 4.0);
    out.font(FONT, 14.0, 400, 20.0);
    out.flag(symbol!("plain_disabled"));
    out.no_focus_ring();
    if native.disabled {
        out.color(symbol!("background"), t.control_fill_disabled);
        out.color(symbol!("text_color"), t.text_disabled);
        out.color(symbol!("placeholder_color"), t.text_disabled);
        border(out, t.control_stroke, t.control_stroke);
        return;
    }
    out.color(symbol!("background"), t.control_fill);
    out.color(symbol!("hover_background"), t.control_fill_secondary);
    out.color(symbol!("focus_background"), t.control_fill_input_active);
    out.color(symbol!("text_color"), t.text_primary);
    out.color(symbol!("placeholder_color"), t.text_secondary);
    border(out, t.control_stroke, t.control_strong_stroke);
    out.focus_line("bottom", accent(native), 2.0);
}

pub(super) fn label(native: &NativeControl) -> Vec<(Symbol, PropValue)> {
    let t = tokens(native);
    let mut props = Vec::with_capacity(5);
    let mut out = Out(&mut props);
    out.color(symbol!("text_color"), if native.disabled { t.text_disabled } else { t.text_primary });
    out.font(FONT, 14.0, 400, 20.0);
    out.into_vec()
}

pub(super) fn parts(native: &NativeControl) -> Vec<rsui_core::Element> {
    let t = tokens(native);
    match native.control {
        Control::Checkbox => vec![check_box(native, t)],
        Control::Switch => vec![toggle_switch(native, t)],
        Control::Slider => slider(native, t),
        Control::Progress => {
            let (done, rest) = (f64::from(native.fraction), 1.0 - f64::from(native.fraction));
            let indicator = part(native, 0, || {
                let mut props = Vec::new();
                let mut out = Out(&mut props);
                out.number(symbol!("grow"), done);
                out.number(symbol!("shrink"), 1.0);
                out.number(symbol!("basis"), 0.0);
                out.number(symbol!("height"), 3.0);
                out.number(symbol!("corner_radius"), 1.5);
                out.color(symbol!("background"), accent(native));
                out.into_vec()
            });
            let track = part(native, 1, || {
                let mut props = Vec::new();
                let mut out = Out(&mut props);
                out.number(symbol!("grow"), rest);
                out.number(symbol!("shrink"), 1.0);
                out.number(symbol!("basis"), 0.0);
                out.number(symbol!("height"), 1.0);
                out.color(symbol!("background"), t.control_strong_stroke);
                out.into_vec()
            });
            vec![indicator, track]
        }
        _ => Vec::new(),
    }
}

/// A check box (`DefaultCheckBoxStyle`): a box 20 across with corners of 4, outlined until it's ticked and filled with the
/// accent color.
fn check_box(native: &NativeControl, t: &Tokens) -> rsui_core::Element {
    let ticked = native.checked;
    let held = part(native, 2, || {
        let mut props = Vec::new();
        let mut out = Out(&mut props);
        out.size(20.0, 20.0);
        out.number(symbol!("shrink"), 0.0);
        out.number(symbol!("corner_radius"), 4.0);
        out.text(symbol!("align"), "center");
        out.text(symbol!("justify"), "center");
        match (ticked, native.disabled) {
            (true, false) => {
                let accent = accent(native);
                out.color(symbol!("background"), accent);
                out.color(symbol!("hover_background"), alpha(accent, 0.9));
            }
            (true, true) => out.color(symbol!("background"), t.accent_fill_disabled),
            (false, false) => {
                out.color(symbol!("background"), t.control_alt_fill_secondary);
                out.color(symbol!("hover_background"), t.control_alt_fill_tertiary);
                out.color(symbol!("border_color"), t.control_strong_stroke);
                out.number(symbol!("border_width"), 1.0);
            }
            (false, true) => {
                out.color(symbol!("border_color"), t.control_strong_stroke_disabled);
                out.number(symbol!("border_width"), 1.0);
            }
        }
        out.into_vec()
    });
    if !ticked {
        return held;
    }
    let color = if native.disabled { t.text_on_accent_disabled } else { t.text_on_accent };
    held.child(
        rsui_core::Element::new(rsui_core::Role::Image)
            .props(super::part_props(native, 3, || check_mark(20.0, "0 0 20 20", "M5.5 10.5 L8.5 13.5 L14.5 6.5", 1.4, color))),
    )
}

/// A toggle switch (`DefaultToggleSwitchStyle`): a track 40 by 20, round, outlined when it's off and the accent color when
/// it's on, with a knob 12 across that slides from one end to the other.
fn toggle_switch(native: &NativeControl, t: &Tokens) -> rsui_core::Element {
    let on = native.checked;
    let knob = part(native, 4, || {
        let mut props = Vec::new();
        let mut out = Out(&mut props);
        out.size(12.0, 12.0);
        out.number(symbol!("corner_radius"), 6.0);
        out.number(symbol!("margin_left"), 4.0);
        let color = match (on, native.disabled) {
            (true, false) => t.text_on_accent,
            (false, false) => t.text_secondary,
            (true, true) => t.text_on_accent_disabled,
            (false, true) => t.text_disabled,
        };
        out.color(symbol!("background"), color);
        out.text(symbol!("transform"), if on { "translateX(20px)" } else { "translateX(0px)" });
        out.text(symbol!("transitions"), "transform s 0.167 0 cubic-bezier(0, 0, 0, 1)\nbackground c 0.083 0 linear");
        out.into_vec()
    });
    part(native, 5, || {
        let mut props = Vec::new();
        let mut out = Out(&mut props);
        out.size(40.0, 20.0);
        out.number(symbol!("shrink"), 0.0);
        out.number(symbol!("corner_radius"), 10.0);
        out.text(symbol!("direction"), "row");
        out.text(symbol!("align"), "center");
        match (on, native.disabled) {
            (true, false) => {
                let accent = accent(native);
                out.color(symbol!("background"), accent);
                out.color(symbol!("hover_background"), alpha(accent, 0.9));
            }
            (true, true) => out.color(symbol!("background"), t.accent_fill_disabled),
            (false, disabled) => {
                if !disabled {
                    out.color(symbol!("background"), t.control_alt_fill_secondary);
                    out.color(symbol!("hover_background"), t.control_alt_fill_tertiary);
                }
                out.color(symbol!("border_color"), if disabled { t.control_strong_stroke_disabled } else { t.control_strong_stroke });
                out.number(symbol!("border_width"), 1.0);
            }
        }
        out.into_vec()
    })
    .child(knob)
}

/// A slider (`DefaultSliderStyle`): a track 4 high, of the accent color up to a thumb 20 across, which is a white circle with a
/// border holding a smaller one of the accent color.
fn slider(native: &NativeControl, t: &Tokens) -> Vec<rsui_core::Element> {
    let along = f64::from(native.fraction);
    let (value_color, rest_color) = if native.disabled {
        (t.accent_fill_disabled, t.control_strong_fill_disabled)
    } else {
        (accent(native), t.control_strong_fill)
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
    let inner = part(native, 9, || {
        let mut props = Vec::new();
        let mut out = Out(&mut props);
        out.size(12.0, 12.0);
        out.number(symbol!("corner_radius"), 6.0);
        out.color(symbol!("background"), value_color);
        out.into_vec()
    });
    let thumb = part(native, 7, || {
        let mut props = Vec::new();
        let mut out = Out(&mut props);
        out.size(20.0, 20.0);
        out.number(symbol!("shrink"), 0.0);
        out.number(symbol!("corner_radius"), 10.0);
        out.text(symbol!("align"), "center");
        out.text(symbol!("justify"), "center");
        out.color(symbol!("background"), t.control_solid_fill);
        border(&mut out, t.control_stroke, t.control_stroke_secondary);
        out.into_vec()
    })
    .child(inner);
    vec![track(along, value_color, 6), thumb, track(1.0 - along, rest_color, 8)]
}
