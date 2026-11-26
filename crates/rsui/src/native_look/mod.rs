//! rsui's own controls in the platform's look: Material 3 on Android, Fluent (Windows 11's) on Windows, drawn by rsui as any
//! element is, so a control in this look costs what any other does, in a list of thousands, scrolled or animated.
//!
//! An element says it wants this with `native` (`<button native>`), which is what [`restyle`] is for: it takes out whatever
//! the app said of how the element looks, keeping where it goes and how big it is, and puts in the platform's colors,
//! shapes, type and state layers, as props any element can have. What changes as it's used is drawn from props too: the
//! background under the pointer and while it's pressed (`hover_background`, `pressed_background`), Material's ripple
//! (`ripple`), a text field's line while it's typed in (`focus_line`), and the focus ring (`focus_ring`). A control with parts,
//! as a checkbox's box or a switch's track, has them put in by [`finish`].
//!
//! Which look is chosen once, by the app's `rsui.config.ts` (`theme`), or `RSUI_THEME`, which goes over it: `system` is
//! Material on Android and Fluent elsewhere (Linux too, until it has a look of its own; `mac`'s isn't written yet, and is
//! that too). Android's build has only Material in it. Dark or light follows the system, or `RSUI_COLOR_SCHEME`, and
//! [`set_dark`] when that changes, after which what's drawn has to be made again. Fluent's accent color is the user's
//! (`AccentPalette`); Material's colors are its baseline scheme until Android says what its dynamic colors are.

use std::cell::{Cell, RefCell};

use rsui_core::{Element, PropValue, Props, Role, Symbol, symbol};
use rsui_render::Color;

#[cfg(not(target_os = "android"))]
mod fluent;
mod material;

/// A platform's look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    /// Material 3, Android's.
    Material,
    /// Fluent, Windows 11's, which an Android build doesn't have.
    #[cfg(not(target_os = "android"))]
    Fluent,
}

impl Theme {
    /// What `system` means here.
    pub fn of_system() -> Self {
        #[cfg(target_os = "android")]
        return Self::Material;
        #[cfg(not(target_os = "android"))]
        Self::Fluent
    }

    /// The look a config's `theme` names: `system`, `material`, `fluent` or `mac`. One that isn't built in here, as Fluent on
    /// Android or `mac` anywhere yet, is the system's.
    pub fn named(name: &str) -> Self {
        match name {
            "material" => Self::Material,
            #[cfg(not(target_os = "android"))]
            "fluent" => Self::Fluent,
            // TODO: macOS's look, once there's a window there to show it in.
            _ => Self::of_system(),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Material => "material",
            #[cfg(not(target_os = "android"))]
            Self::Fluent => "fluent",
        }
    }
}

/// What a control in the platform's look is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Button,
    /// A text field, of one line or several.
    Input,
    Checkbox,
    Switch,
    Slider,
    Progress,
    /// A window, whose background, text color and type are the platform's, for the controls in it to be on.
    Surface,
}

impl Control {
    /// The control an element with this tag is in the platform's look, if it's one that has one.
    pub fn of_tag(tag: &str) -> Option<Self> {
        Some(match tag {
            "button" => Self::Button,
            "input" => Self::Input,
            "checkbox" => Self::Checkbox,
            "switch" => Self::Switch,
            "slider" => Self::Slider,
            "progress" => Self::Progress,
            "window" => Self::Surface,
            _ => return None,
        })
    }

    pub fn tag(self) -> &'static str {
        match self {
            Self::Button => "button",
            Self::Input => "input",
            Self::Checkbox => "checkbox",
            Self::Switch => "switch",
            Self::Slider => "slider",
            Self::Progress => "progress",
            Self::Surface => "window",
        }
    }
}

#[derive(Clone, Copy)]
struct Settings {
    theme: Option<Theme>,
    dark: Option<bool>,
    /// Whether anything's been drawn in the look, which has to be drawn again if it changes.
    used: bool,
}

thread_local! {
    static SETTINGS: Cell<Settings> = const { Cell::new(Settings { theme: None, dark: None, used: false }) };
    /// The parts of controls made so far, as their props are the same for every control in the same state.
    static PARTS: RefCell<Vec<(u64, Props)>> = const { RefCell::new(Vec::new()) };
    /// What each look says of a control in each state, as [`restyle`] puts it on.
    static STYLES: RefCell<Vec<(u64, Vec<(Symbol, PropValue)>)>> = const { RefCell::new(Vec::new()) };
}

fn settings() -> Settings {
    SETTINGS.with(Cell::get)
}

fn change(change: impl FnOnce(&mut Settings)) {
    SETTINGS.with(|settings| {
        let mut now = settings.get();
        change(&mut now);
        settings.set(now);
    });
}

/// Which look `native` is on this thread, as the app's config says (`system`, `material`, `fluent` or `mac`), unless
/// `RSUI_THEME` says otherwise.
pub fn choose_theme(name: &str) {
    let theme = Theme::named(&std::env::var("RSUI_THEME").unwrap_or_else(|_| name.to_owned()));
    if settings().theme != Some(theme) {
        PARTS.with(|parts| parts.borrow_mut().clear());
    }
    change(|settings| settings.theme = Some(theme));
}

/// The look `native` is on this thread.
pub fn theme() -> Theme {
    if let Some(theme) = settings().theme {
        return theme;
    }
    let theme = std::env::var("RSUI_THEME").map_or_else(|_| Theme::of_system(), |name| Theme::named(&name));
    change(|settings| settings.theme = Some(theme));
    theme
}

/// Whether the look is its dark one.
pub fn dark() -> bool {
    if let Some(dark) = settings().dark {
        return dark;
    }
    let dark = match std::env::var("RSUI_COLOR_SCHEME") {
        Ok(scheme) => scheme == "dark",
        #[cfg(target_os = "android")]
        Err(_) => crate::window::night(),
        #[cfg(not(target_os = "android"))]
        Err(_) => crate::system_is_dark(),
    };
    change(|settings| settings.dark = Some(dark));
    dark
}

/// The system has gone dark or light. Says whether anything drawn in the look has to be made again for it.
pub fn set_dark(dark: bool) -> bool {
    let was = settings();
    if was.dark == Some(dark) {
        return false;
    }
    PARTS.with(|parts| parts.borrow_mut().clear());
    change(|settings| settings.dark = Some(dark));
    was.used
}

/// A control being made in the platform's look, from what [`restyle`] found out about it, for [`finish`].
#[derive(Debug, Clone, Copy)]
pub struct NativeControl {
    pub control: Control,
    pub theme: Theme,
    /// Which of its look's variants, by its number in that look's list.
    variant: u8,
    disabled: bool,
    checked: bool,
    /// How far along a slider or progress bar is, from 0 to 1.
    fraction: f32,
    dark: bool,
}

impl NativeControl {
    /// The value a slider or progress bar shows, from 0 to 1.
    pub fn fraction(&self) -> f32 {
        self.fraction
    }

    pub fn checked(&self) -> bool {
        self.checked
    }

    pub fn disabled(&self) -> bool {
        self.disabled
    }
}

/// Whether a prop is about how an element looks rather than where it goes, which a control in the platform's look decides
/// for itself.
pub fn is_visual(name: Symbol) -> bool {
    let name = name.as_str();
    matches!(
        name,
        "background"
            | "hover_background"
            | "pressed_background"
            | "text_color"
            | "corner_radius"
            | "corner_radii"
            | "shadows"
            | "placeholder_color"
            | "decoration_color"
            | "underline"
            | "line_through"
            | "overline"
            | "css_filter"
            | "backdrop_blur"
            | "text_shadow"
            | "letter_spacing"
            | "word_spacing"
            | "line_height"
            | "transitions"
            | "ripple"
            | "pressed_text_color"
            | "plain_disabled"
            | "text_align"
            | "class_name"
    ) || ["border_", "shadow_", "outline_", "font_", "padding_", "background_", "focus_", "scrollbar_"]
        .iter()
        .any(|start| name.starts_with(start))
}

fn number(value: Option<&PropValue>) -> Option<f64> {
    match value? {
        PropValue::Float(number) => Some(*number),
        PropValue::Int(number) => Some(*number as f64),
        PropValue::Str(text) => text.trim_end_matches('%').parse().ok(),
        PropValue::Bool(_) => None,
    }
}

/// Puts the platform's look on an element's props, taking out what the app said of how it looks (each of which `ignored`
/// is told, for a warning), with what's said for this look's platform (`variant`, from `android={{ variant }}` or
/// `windows={{ variant }}`). Layout the app said goes over what the look says, as a `width` or a `minHeight`.
pub fn restyle(control: Control, variant: Option<&str>, entries: &mut Vec<(Symbol, PropValue)>, mut ignored: impl FnMut(Symbol)) -> NativeControl {
    let theme = theme();
    fn get(entries: &[(Symbol, PropValue)], name: Symbol) -> Option<&PropValue> {
        entries.iter().rev().find(|(said, _)| *said == name).map(|(_, value)| value)
    }
    let flag = |entries: &[(Symbol, PropValue)], name: Symbol| get(entries, name) == Some(&PropValue::Bool(true));
    let disabled = flag(entries, symbol!("disabled"));
    let checked = flag(entries, symbol!("checked"));
    let (min, max) = match control {
        Control::Slider => (number(get(entries, symbol!("min"))).unwrap_or(0.0), number(get(entries, symbol!("max"))).unwrap_or(100.0)),
        _ => (0.0, number(get(entries, symbol!("max"))).unwrap_or(1.0)),
    };
    let fraction = match control {
        Control::Slider | Control::Progress => {
            let value = number(get(entries, symbol!("value"))).unwrap_or(min);
            if max > min { ((value - min) / (max - min)).clamp(0.0, 1.0) as f32 } else { 0.0 }
        }
        _ => 0.0,
    };
    entries.retain(|(name, _)| {
        if *name == symbol!("native") {
            return false;
        }
        // What's in a window is placed by it, so its padding is where things go rather than how it looks.
        let visual = is_visual(*name) && !(control == Control::Surface && name.as_str().starts_with("padding_"));
        if visual {
            ignored(*name);
        }
        !visual
    });
    let mut native = NativeControl { control, theme, variant: 0, disabled, checked, fraction, dark: dark() };
    native.variant = match theme {
        Theme::Material => material::variant(control, variant),
        #[cfg(not(target_os = "android"))]
        Theme::Fluent => fluent::variant(control, variant),
    };
    // What the look says is the same for every control in the same state, and is made once: making it each time was
    // a fifth of what a list of buttons took to make again.
    let key = state_key(&native, 0xFFFF);
    STYLES.with(|styles| {
        let mut styles = styles.borrow_mut();
        let at = match styles.iter().position(|(kept, _)| *kept == key) {
            Some(at) => at,
            None => {
                let mut own = Vec::with_capacity(24);
                match theme {
                    Theme::Material => material::style(&native, &mut Out(&mut own)),
                    #[cfg(not(target_os = "android"))]
                    Theme::Fluent => fluent::style(&native, &mut Out(&mut own)),
                }
                styles.push((key, own));
                styles.len() - 1
            }
        };
        // The look's first, so what the app said of where it goes comes after it, and wins.
        entries.splice(0..0, styles[at].1.iter().cloned());
    });
    change(|settings| settings.used = true);
    native
}

/// A number for a control's state, and which of its parts, for what's kept of how it looks: the part in the low 16 bits, and
/// the state above them.
fn state_key(native: &NativeControl, part: u32) -> u64 {
    let state = (native.control as u64) << 8
        | u64::from(native.variant) << 3
        | u64::from(native.theme != Theme::Material) << 2
        | u64::from(native.disabled) << 1
        | u64::from(native.checked);
    (state << 1 | u64::from(native.dark)) << 16 | u64::from(part & 0xFFFF)
}

/// Puts in the parts of a control that has them, as a checkbox's box, before what's in it, with a label from its name if
/// there's nothing else in it. Call it before what's in it is put in.
pub fn finish(native: &NativeControl, mut element: Element, has_children: bool) -> Element {
    let parts = match native.theme {
        Theme::Material => material::parts(native),
        #[cfg(not(target_os = "android"))]
        Theme::Fluent => fluent::parts(native),
    };
    if parts.is_empty() {
        return element;
    }
    let labelled = matches!(native.control, Control::Checkbox | Control::Switch);
    let label = (labelled && !has_children && element.children.is_empty() && !element.name.is_empty()).then(|| element.name.clone());
    for (at, part) in parts.into_iter().enumerate() {
        element.children.insert(at, part.into_node());
    }
    if let Some(label) = label {
        let props = part_props(native, 99, || match native.theme {
            Theme::Material => material::label(native),
            #[cfg(not(target_os = "android"))]
            Theme::Fluent => fluent::label(native),
        });
        element = element.child(Element::new(Role::Text).name(label).props(props));
    }
    element
}

/// The props of one of a control's parts, made once for each state it can be in, which is most of what they cost.
fn part_props(native: &NativeControl, part: u32, make: impl FnOnce() -> Vec<(Symbol, PropValue)>) -> Props {
    // Sliders and progress bars are as many states as there are places along them, which aren't kept.
    let along = matches!(native.control, Control::Slider | Control::Progress);
    let key = state_key(native, part);
    if !along {
        if let Some(found) = PARTS.with(|parts| parts.borrow().iter().find(|(kept, _)| *kept == key).map(|(_, props)| props.clone())) {
            return found;
        }
    }
    let props: Props = make().into_iter().collect();
    if !along {
        PARTS.with(|parts| parts.borrow_mut().push((key, props.clone())));
    }
    props
}

/// What a look writes props with.
pub(crate) struct Out<'a>(&'a mut Vec<(Symbol, PropValue)>);

impl Out<'_> {
    fn color(&mut self, name: Symbol, color: Color) {
        self.0.push((name, PropValue::Int(color.to_prop())));
    }

    fn number(&mut self, name: Symbol, number: f64) {
        self.0.push((name, PropValue::Float(number)));
    }

    fn text(&mut self, name: Symbol, text: &str) {
        self.0.push((name, PropValue::from(text)));
    }

    fn flag(&mut self, name: Symbol) {
        self.0.push((name, PropValue::Bool(true)));
    }

    fn padding(&mut self, left: f64, top: f64, right: f64, bottom: f64) {
        self.number(symbol!("padding_left"), left);
        self.number(symbol!("padding_top"), top);
        self.number(symbol!("padding_right"), right);
        self.number(symbol!("padding_bottom"), bottom);
    }

    fn size(&mut self, width: f64, height: f64) {
        self.number(symbol!("width"), width);
        self.number(symbol!("height"), height);
    }

    fn font(&mut self, family: &str, size: f64, weight: i64, line_height: f64) {
        self.text(symbol!("font_family"), family);
        self.number(symbol!("font_size"), size);
        self.0.push((symbol!("font_weight"), PropValue::Int(weight)));
        self.number(symbol!("line_height"), line_height);
    }

    /// A ring around it while it has the keyboard: `width` wide, `offset` outside its edge, and a thinner one inside that.
    fn focus_ring(&mut self, color: Color, width: f64, offset: f64, inner: Option<(Color, f64)>) {
        let mut said = format!("{} {width} {offset}", color.to_prop());
        if let Some((inner, inner_width)) = inner {
            said.push_str(&format!(" {} {inner_width}", inner.to_prop()));
        }
        self.text(symbol!("focus_ring"), &said);
    }

    /// No ring while it has the keyboard, for a field that shows it some other way.
    fn no_focus_ring(&mut self) {
        self.text(symbol!("focus_ring"), "");
    }

    fn focus_line(&mut self, side: &str, color: Color, width: f64) {
        self.text(symbol!("focus_line"), &format!("{side} {} {width}", color.to_prop()));
    }

    fn into_vec(self) -> Vec<(Symbol, PropValue)> {
        std::mem::take(self.0)
    }
}

/// A color as `#AARRGGBB` is written, as Fluent's tokens are.
#[cfg(not(target_os = "android"))]
pub(crate) const fn argb(value: u32) -> Color {
    Color::rgba((value >> 16) as u8, (value >> 8) as u8, value as u8, (value >> 24) as u8)
}

/// `color` with this much of `layer` over it, as Material's state layers are.
pub(crate) fn over(color: Color, layer: Color, opacity: f32) -> Color {
    if color.a == 0 {
        return Color { a: (opacity * 255.0).round() as u8, ..layer };
    }
    let mix = |from: u8, to: u8| (f32::from(from) + (f32::from(to) - f32::from(from)) * opacity).round() as u8;
    Color { r: mix(color.r, layer.r), g: mix(color.g, layer.g), b: mix(color.b, layer.b), a: color.a }
}

/// `color`, this see-through.
pub(crate) fn alpha(color: Color, opacity: f32) -> Color {
    Color { a: (f32::from(color.a) * opacity).round() as u8, ..color }
}

/// A check mark as a picture, in the color of `text_color`, for a checkbox.
fn check_mark(size: f64, view: &str, path: &str, stroke: f32, color: Color) -> Vec<(Symbol, PropValue)> {
    let mut props = Vec::with_capacity(6);
    let mut out = Out(&mut props);
    out.size(size, size);
    out.text(symbol!("view_box"), view);
    out.text(symbol!("shape_layers"), &rsui_render::shape_layer(path, None, Some((None, stroke)), rsui_render::LineCap::Round));
    out.color(symbol!("text_color"), color);
    out.into_vec()
}

/// A part of a control: a group with these props, and what's in it.
fn part(native: &NativeControl, number: u32, make: impl FnOnce() -> Vec<(Symbol, PropValue)>) -> Element {
    Element::new(Role::Group).props(part_props(native, number, make))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_says_how_it_looks_is_taken_out_and_where_it_goes_is_kept() {
        choose_theme("material");
        let mut entries = vec![
            (symbol!("native"), PropValue::Bool(true)),
            (symbol!("background"), PropValue::Int(0x00FF_00FF)),
            (symbol!("corner_radius"), PropValue::Float(3.0)),
            (symbol!("margin_top"), PropValue::Float(8.0)),
            (symbol!("width"), PropValue::Float(120.0)),
            (symbol!("font_size"), PropValue::Float(30.0)),
        ];
        let mut ignored = Vec::new();
        let native = restyle(Control::Button, Some("tonal"), &mut entries, |name| ignored.push(name.as_str()));
        assert_eq!(ignored, ["background", "corner_radius", "font_size"]);
        let props: Props = {
            let mut entries = entries.clone();
            entries.reverse();
            entries.dedup_by_key(|(name, _)| *name);
            entries.into_iter().collect()
        };
        assert_eq!(props.get("margin_top"), Some(&PropValue::Float(8.0)));
        assert_eq!(props.get("width"), Some(&PropValue::Float(120.0)));
        assert_eq!(props.get("corner_radius"), Some(&PropValue::Float(20.0)));
        assert!(props.get("native").is_none());
        assert_eq!(native.theme, Theme::Material);
    }

    #[test]
    fn what_is_kept_of_a_look_is_kept_apart_for_light_and_dark() {
        choose_theme("material");
        let background = |dark: bool| {
            set_dark(dark);
            let mut entries = Vec::new();
            restyle(Control::Button, None, &mut entries, |_| {});
            entries.into_iter().find(|(name, _)| *name == symbol!("background")).map(|(_, value)| value)
        };
        let light = background(false);
        assert_ne!(background(true), light);
        assert_eq!(background(false), light);
    }

    #[test]
    fn looks_are_named_and_what_is_not_here_is_the_systems() {
        assert_eq!(Theme::named("material"), Theme::Material);
        assert_eq!(Theme::named("mac"), Theme::of_system());
        assert_eq!(Theme::named("system"), Theme::of_system());
        #[cfg(not(target_os = "android"))]
        assert_eq!(Theme::named("fluent"), Theme::Fluent);
    }

    #[test]
    fn state_layers_go_over_what_they_are_on() {
        let white = Color::WHITE;
        let black = Color::BLACK;
        assert_eq!(over(white, black, 0.5), Color::rgba(128, 128, 128, 255));
        assert_eq!(over(Color::TRANSPARENT, black, 0.08).a, 20);
    }
}
