//! Joins the parts into one thing that can be driven by a window, or by a test.

#[cfg(feature = "mutation-pilot")]
#[path = "mutation_pilot.rs"]
pub(crate) mod mutation_pilot;

use std::fmt;
use std::time::{Duration, Instant};

use rsui_core::{ActionKind, Element, IntoNode, Key, Node, Patch, PropValue, Role, Symbol, symbol};
use rsui_layout::{HitRegion, Layout, Measure, Rect, Size, is_inert, is_paragraph, takes_pointer};
use rsui_render::{
    Color, Editing, EditingLines, Images, Interaction, Overlay, Painter, RenderError, Renderer,
    SCROLL_BAR_REACH, Scene, byte_from_mask, byte_in_mask, content_box, has_hover, has_pressed,
    input_indent, input_value, natural_shape_size, password_mask, scroll_for_thumb, scroll_thumb, text_line_count,
};
use rsui_runtime::{Cx, DispatchError, Modifiers, Runtime, document_listens, listens};
use rsui_text::{
    Font, FontError, TextEngine, TextStyle, is_selectable, paragraph_text, wrapped_line,
};

use crate::access::{self, AccessCommand, AccessTree};
use crate::edit::{Change, Direction, TextEdit, Unit};
use crate::input::{Clipboard, Cursor, KeyCode, KeyPress, MemoryClipboard, PointerButton};
use crate::inspect::{FrameRecord, Inspector};

#[path = "app_events.rs"]
mod events;

#[cfg(test)]
#[path = "app_interaction_tests.rs"]
mod interaction_tests;

#[cfg(test)]
#[path = "app_hit_cell_tests.rs"]
mod hit_cell_tests;

#[cfg(test)]
#[path = "app_pill_tests.rs"]
mod pill_tests;

pub use events::DropPhase;
use events::{Captured, Kind, follow_along, keys_along};

const REGULAR_FONT: &str = "C:/Windows/Fonts/segoeui.ttf";
#[cfg_attr(not(windows), allow(dead_code))]
const BOLD_FONT: &str = "C:/Windows/Fonts/segoeuib.ttf";
pub(crate) const BACKGROUND: Color = Color::hex(0xF6F8FA);

/// What's drawn where nothing else is: the window's own color, if it has one that can't be seen through, and otherwise
/// nothing at all in a window that's seen through, for its backdrop to show, or rsui's background.
pub(crate) fn clear_color(window_color: Option<Color>, see_through: bool) -> Color {
    match window_color {
        Some(color) => color,
        None if see_through => Color::TRANSPARENT,
        None => BACKGROUND,
    }
}
/// How many times a caret changes before it stops blinking and stays on. A blinking caret draws
/// frames, so one left alone would stop the window from ever going quiet and giving back memory.
const CARET_CHANGES: u32 = 20;
/// How far one row of the wheel moves a scrolling area, in layout pixels. A notch is three rows.
const WHEEL_ROW: f32 = 100.0 / 3.0;
/// How far an arrow key moves a scrolling area, in layout pixels.
const ARROW_STEP: f32 = 40.0;
/// How much of what showed stays showing after a page, in layout pixels, to keep your place.
const PAGE_OVERLAP: f32 = 40.0;
/// How much of what shows a page of a web page's scrolling goes, as Chromium's (and Edge's) `kMinFractionToStepWhenPaging`.
const WEB_PAGE_FRACTION: f32 = 0.875;
/// How much room to leave around something brought into view, in layout pixels.
const REVEAL_MARGIN: f32 = 8.0;
/// How long the pointer stays on something before a tooltip shows.
const TOOLTIP_DELAY: Duration = Duration::from_millis(500);
/// How long a hover color takes to come and go.
const HOVER_FADE: Duration = Duration::from_millis(100);
/// How long scrolling takes to get where it's going.
const SCROLL_GLIDE: Duration = Duration::from_millis(120);

/// Why an app couldn't start or keep running.
#[derive(Debug)]
pub enum RunError {
    Font(FontError),
    Render(RenderError),
    #[cfg(windows)]
    Window(windows::core::Error),
    /// Elsewhere, what the system said, or that there's no window system to open one with yet.
    #[cfg(not(windows))]
    Window(String),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Font(error) => write!(f, "couldn't load the default font: {error}"),
            Self::Render(error) => write!(f, "couldn't draw: {error}"),
            Self::Window(error) => write!(f, "couldn't make a window: {error}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<FontError> for RunError {
    fn from(error: FontError) -> Self {
        Self::Font(error)
    }
}

impl From<RenderError> for RunError {
    fn from(error: RenderError) -> Self {
        Self::Render(error)
    }
}

#[cfg(windows)]
impl From<windows::core::Error> for RunError {
    fn from(error: windows::core::Error) -> Self {
        Self::Window(error)
    }
}

thread_local! {
    static LAST_SCALE: std::cell::Cell<f32> = const { std::cell::Cell::new(1.0) };
}

/// How many physical pixels there are to a logical one in the window that was last given a size on this thread, as a page's
/// `devicePixelRatio` says. 1 before there's one, and with no window.
pub fn window_scale() -> f32 {
    LAST_SCALE.with(std::cell::Cell::get)
}

/// The system's UI font, in regular and bold.
pub fn default_text_engine() -> Result<TextEngine, RunError> {
    #[cfg(windows)]
    let (regular, bold) = (REGULAR_FONT, Some(BOLD_FONT));
    // Elsewhere it's whichever of the fonts the system's UI uses there is (see `rsui_text::system_ui_font_files`).
    #[cfg(not(windows))]
    let (regular, bold) = {
        let (regular, bold) = rsui_text::system_ui_font_files();
        (regular.unwrap_or(REGULAR_FONT), bold)
    };
    let mut engine = TextEngine::new(Font::open(regular)?);
    if let Some(Ok(bold)) = bold.map(Font::open) {
        engine = engine.with_bold(bold);
    }
    Ok(engine.with_system_fallbacks())
}

/// Measures text with the engine, and images by what their files say.
pub(crate) struct Measuring<'a> {
    pub(crate) text: &'a mut TextEngine,
    pub(crate) images: &'a mut Images,
}

impl Measure for Measuring<'_> {
    fn measure(&mut self, element: &Element, max_width: f32) -> Size {
        let _owner = rsui_alloc::owners::text_scope();
        if element.role != Role::Image {
            return Measure::measure(self.text, element, max_width);
        }
        let (width, height) = if let Some(size) = natural_shape_size(element) {
            size
        } else {
            let Some(PropValue::Str(source)) = element.props.get("src") else {
                return Size::new(0.0, 0.0);
            };
            let Some((width, height)) = self.images.natural_size(source) else {
                return Size::new(0.0, 0.0);
            };
            (width as f32, height as f32)
        };
        // What only says what shape it is is as wide as there's room for, as a browser has it, or as big as fits in a browser's
        // usual 300 by 150 when there's no saying how much room that is.
        if let Some(PropValue::Str(source)) = element.props.get("src")
            && self.images.shape_only(source)
            && width > 0.0
            && height > 0.0
        {
            let room = if max_width.is_finite() && max_width > 0.0 {
                max_width
            } else {
                (300.0f32).min(150.0 * width / height)
            };
            return Size::new(room, room * height / width);
        }
        // Shown at its own size, unless that's too wide, in which case it keeps its shape. Asked how narrow it can be, which is
        // what no room at all asks, it's its own size, as a picture is in CSS.
        let scale = if width > max_width && width > 0.0 && max_width > 0.0 {
            max_width / width
        } else {
            1.0
        };
        Size::new(width * scale, height * scale)
    }

    fn place_pieces(&mut self, element: &Element, width: f32, places: &mut Vec<Rect>) {
        let _owner = rsui_alloc::owners::text_scope();
        Measure::place_pieces(self.text, element, width, places);
    }

    fn line_below(&mut self, element: &Element) -> f32 {
        let _owner = rsui_alloc::owners::text_scope();
        Measure::line_below(self.text, element)
    }

    fn first_baseline(&mut self, element: &Element) -> Option<f32> {
        let _owner = rsui_alloc::owners::text_scope();
        Measure::first_baseline(self.text, element)
    }

    fn strut(&mut self, element: &Element) -> Option<(f32, f32)> {
        let _owner = rsui_alloc::owners::text_scope();
        Measure::strut(self.text, element)
    }

    fn inline_area(&mut self, element: &Element) -> Option<(f32, f32)> {
        let _owner = rsui_alloc::owners::text_scope();
        Measure::inline_area(self.text, element)
    }

    fn x_height(&mut self, element: &Element) -> Option<f32> {
        let _owner = rsui_alloc::owners::text_scope();
        Measure::x_height(self.text, element)
    }

    fn line_levels(&mut self, line: &Element, levels: &mut Vec<u8>) {
        let _owner = rsui_alloc::owners::text_scope();
        Measure::line_levels(self.text, line, levels);
    }
}


/// What says whether the tree assistive technology sees might have changed.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) type AccessFingerprint = (u64, Option<Vec<u32>>, (u32, u32), u32);

/// What a window should do after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// What's on screen is still right.
    Unchanged,
    /// Draw again, by calling [`App::render`].
    Needed,
}

/// Text selected with the pointer, outside of any input.
struct TextSelection {
    path: Vec<u32>,
    /// All of it, as it was when selected. If that changes, what's selected no longer means anything.
    text: String,
    edit: TextEdit,
}

/// What's being dragged, which only has to want to know.
struct Dragged {
    path: Vec<u32>,
    last: (f32, f32),
}

/// What the keyboard is on.
struct Focus {
    path: Vec<u32>,
    /// The key at each step of `path`, to find the same node after the tree changes.
    keys: Vec<Option<Key>>,
    /// Whether to show a ring. Someone using a mouse knows what they clicked, so buttons only
    /// show one when reached with the keyboard.
    ring: bool,
    /// Whether code said if it shows a ring, as a page's `focus({ focusVisible })` does, which a key going down then doesn't change.
    ring_said: bool,
    /// The caret and history, if it's a text input. Only the input being edited has these, so
    /// other inputs cost nothing.
    edit: Option<TextEdit>,
    /// How far the input's text is moved left, to keep the caret in view, in physical pixels.
    scroll: f32,
    /// How far it's moved up, if it has several lines.
    scroll_y: f32,
    /// Where across the caret was before going up or down lines, to come back to on longer ones.
    goal_x: Option<f32>,
    /// Where the caret is at the end of a line that wraps rather than at the start of the next, which are the same byte, as
    /// a browser has it after End or going up or down to a line shorter than where it was. Only while the caret's there.
    upstream: Option<usize>,
    /// Where the caret and selection go across the input's text. Working this out means reading
    /// all of the text, so it's kept until something moves.
    carets: Option<Carets>,
}

/// Where the caret and selection go in a text input, and what that was worked out from.
#[derive(Clone, PartialEq)]
struct Carets {
    caret: usize,
    /// Whether the caret's at the end of the line before, where that wraps: see `Focus::upstream`.
    upstream: bool,
    selection: (usize, usize),
    scale: f32,
    width: f32,
    caret_x: f32,
    /// What's selected, from left to right.
    selection_x: Vec<(f32, f32)>,
    /// If the input has several lines, which line each piece of the selection is on.
    selection_lines: Vec<u32>,
    caret_line: u32,
    line_count: u32,
}

/// How far to move a scrolling area or a list.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ScrollStep {
    /// By rows of a wheel, positive being down.
    Wheel(i64),
    /// A wheel's exact CSS-pixel delta, and whether it tracks a precision device.
    Pixels(f32, bool),
    /// By arrow key presses, positive being down.
    Arrow(i64),
    /// By pages, positive being down.
    Page(i64),
    Start,
    End,
}

/// A scrolling area on its way somewhere.
struct ScrollGlide {
    path: Vec<u32>,
    /// What to tell it where it's got to with.
    action: ActionKind,
    /// How tall each row is, for a list, whose places are in rows rather than pixels.
    row_height: Option<f32>,
    from: f32,
    to: f32,
    started: Instant,
    /// How fast it was already going when it set off, in its units a second, when another step came while it was on its way
    /// somewhere. A glide that sets off from still slows down as [`eased`] says, which starts at three times its average speed;
    /// one that's sped up again keeps the speed it had and slows to a stop at `to`, so that steps in quick succession, as a
    /// wheel being turned, move it at an even pace, where starting again from that speed each time pulsed with each notch.
    kept_speed: Option<f32>,
    /// For something scrolling across where text goes from right to left: how far it can go, as it's told where to go from its
    /// right (see `App::mirrored_across`), while `from` and `to` are from its left.
    mirror: Option<f32>,
}

impl ScrollGlide {
    /// Where it's got to by `now`, from `from` at the start to `to` at the end of [`SCROLL_GLIDE`].
    fn place(&self, now: Instant) -> f32 {
        let Some(speed) = self.kept_speed else {
            return self.from + (self.to - self.from) * eased(self.started, now, SCROLL_GLIDE);
        };
        let time = SCROLL_GLIDE.as_secs_f32();
        let s = (now.saturating_duration_since(self.started).as_secs_f32() / time).clamp(0.0, 1.0);
        // A cubic that starts at `from` going at `speed` and stops at `to`.
        self.from + speed * time * (s - 2.0 * s * s + s * s * s) + (self.to - self.from) * (3.0 * s * s - 2.0 * s * s * s)
    }

    /// How fast it's going at `now`, in its units a second.
    fn speed(&self, now: Instant) -> f32 {
        let time = SCROLL_GLIDE.as_secs_f32();
        let s = (now.saturating_duration_since(self.started).as_secs_f32() / time).clamp(0.0, 1.0);
        let speed = self.kept_speed.unwrap_or(3.0 * (self.to - self.from) / time);
        speed * (1.0 - 4.0 * s + 3.0 * s * s) + 6.0 * (self.to - self.from) / time * (s - s * s)
    }
}

/// How far something that takes `duration` has got, from 0 to 1, quick to start and slowing down.
fn eased(started: Instant, now: Instant, duration: Duration) -> f32 {
    let left =
        1.0 - (now.saturating_duration_since(started).as_secs_f32() / duration.as_secs_f32());
    let left = left.clamp(0.0, 1.0);
    1.0 - left * left * left
}

/// A scrollbar being dragged.
struct ScrollDrag {
    path: Vec<u32>,
    vertical: bool,
    /// How far along the thumb it was grabbed, in layout pixels.
    grab: f32,
}

/// What's being held down by the pointer.
struct Press {
    path: Vec<u32>,
    key: Option<Key>,
    /// The keys along `path`, for finding it again once what the press set off has put things in or taken them out before it.
    keys: Vec<Option<Key>>,
    /// How many clicks in a row this was.
    clicks: u32,
}

/// A change being offered to a text field's component, as a browser's `InputEvent` says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSaid {
    /// `insertText`, `deleteContentBackward`, `historyUndo` and the like.
    pub input_type: &'static str,
    /// The text typed, for `insertText`.
    pub data: Option<String>,
}

thread_local! {
    static EDITING: std::cell::RefCell<Option<EditSaid>> = const { std::cell::RefCell::new(None) };
}

/// The change to a text field that its component is being offered right now, while its handler runs, so that what the page
/// hears can say what kind of input it was.
pub fn edit_in_progress() -> Option<EditSaid> {
    EDITING.with(|editing| editing.borrow().clone())
}

/// What's selected in the field that has the keyboard, as [`field_selection`] says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSelection {
    /// The field's `dom_key`.
    pub key: String,
    /// Where it's held from and where the caret is, as byte positions in its value.
    pub anchor: usize,
    pub caret: usize,
    /// Goes up by one each time it changes.
    pub at: u64,
}

thread_local! {
    static FIELD_SELECTION: std::cell::RefCell<Option<FieldSelection>> = const { std::cell::RefCell::new(None) };
}

/// Says what's selected in a field, and whether that's changed.
pub(crate) fn say_field_selection(key: String, anchor: usize, caret: usize) -> bool {
    FIELD_SELECTION.with(|said| {
        let mut said = said.borrow_mut();
        let same = said
            .as_ref()
            .is_some_and(|one| one.key == key && one.anchor == anchor && one.caret == caret);
        if !same {
            let at = said.as_ref().map_or(0, |one| one.at) + 1;
            *said = Some(FieldSelection { key, anchor, caret, at });
        }
        !same
    })
}

/// What's selected in the field that last had the keyboard, as it was last said: while a change is being offered to it, where
/// that change leaves it.
pub fn field_selection() -> Option<FieldSelection> {
    FIELD_SELECTION.with(|said| said.borrow().clone())
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RawHitKey {
    root: usize,
    tree_changes: u64,
    geometry: u64,
    size: (u32, u32),
    scale: u32,
    turned: bool,
}

struct RawHitCell {
    key: RawHitKey,
    region: HitRegion,
    path: Option<Vec<u32>>,
}

struct WheelRawHit {
    key: RawHitKey,
    point: [u32; 2],
    path: Option<Vec<u32>>,
}

/// A running UI, apart from the window it's in: the components, layout and text, and what's
/// needed to draw them. Sizes here are in physical pixels, and `scale` is how many of those
/// there are per layout pixel, which depends on the screen.
pub struct App {
    runtime: Runtime,
    /// Shared with what asks where things are while JavaScript runs: see `geometry.rs`.
    layout: std::rc::Rc<std::cell::RefCell<Layout>>,
    text: TextEngine,
    painter: Painter,
    #[cfg(all(feature = "retained-frame", not(feature = "automatic-render")))]
    retained_frame: u64,
    #[cfg(feature = "mutation-pilot")]
    pilot: Option<mutation_pilot::PilotState>,
    #[cfg(feature = "automatic-render")]
    render_binding: u64,
    #[cfg(feature = "automatic-render")]
    render_resources_reset: bool,
    #[cfg(feature = "automatic-render")]
    external_runtime: Option<(u32, u64)>,
    #[cfg(feature = "idle-probe")]
    render_times: (Duration, Duration),
    scale: f32,
    size: (u32, u32),
    /// How many times the tree has changed. Its nodes can change in place, so looking at them doesn't
    /// tell.
    tree_changes: u64,
    /// Whether layout is current for the tree and window size. Paint-only changes keep it valid.
    laid_out: bool,
    /// Whether the runtime has been told how big the window is, which it is the first time it's given a size.
    size_told: bool,
    /// Whether what's on screen is out of date, even though the tree isn't.
    repaint: bool,
    /// Whether the window is seen through where nothing's drawn, as it is with a backdrop.
    see_through: bool,
    /// Whether the caret has blinked since what's on screen was worked out. If that's all that's happened, the scene
    /// only needs the caret taking out or putting back.
    blinked: bool,
    /// Where the pointer is, if it's over the window.
    pointer: Option<(f32, f32)>,
    /// Whether the tree has changed under the pointer since `hovered` was worked out.
    hover_stale: bool,
    /// A single raw hit certificate; it owns no native node or tree.
    raw_hit_cell: Option<RawHitCell>,
    hit_cells_enabled: bool,
    /// Exact-point raw hits shared only within one public wheel operation.
    wheel_hit_memo: Option<WheelRawHit>,
    wheel_hit_active: bool,
    wheel_hit_enabled: bool,
    #[cfg(test)]
    wheel_hit_counts: [u64; 2],
    #[cfg(test)]
    hit_cell_counts: [u64; 2],
    /// Whether selected paragraph content may have changed since validation.
    text_selection_stale: bool,
    /// Mutable Runtime access can consume patches before App sees them.
    interactions_exposed: bool,
    #[cfg(test)]
    interaction_counts: [u64; 4],
    #[cfg(test)]
    conservative_interactions: bool,
    /// The nearest node under the pointer with a hover background.
    hovered: Option<Vec<u32>>,
    /// Whether things move, or go straight to where they're going. See
    /// [`set_animations`](Self::set_animations).
    animations: bool,
    /// When the pointer got to what it's on, while that's still fading in.
    hover_since: Option<Instant>,
    hover_amount: f32,
    /// What the pointer's left, while that's still fading out: since when, and how much it started with.
    leaving: Option<(Vec<u32>, Instant, f32)>,
    leaving_amount: f32,
    glide: Option<ScrollGlide>,
    /// When what's being taken in now happened, if a window said: a wheel's notch glides from then (see `set_input_time`).
    input_at: Option<Instant>,
    /// How long the screen shows each frame for, if it's been said.
    frame_time: Duration,
    /// A text input with several lines that's scrolled without having the keyboard, and how far, in
    /// physical pixels. Those keep nothing about themselves, and only one can be under the pointer.
    resting_area: Option<(Vec<u32>, f32)>,
    cursor: Cursor,
    /// Where the pointer was, where it was pressed, and what had the keyboard, as was last told to whatever wanted to know.
    pointer_paths: [Vec<u32>; 3],
    /// The keys along each of those, for finding where it is once things have been put in or taken out around it.
    pointer_keys: [Vec<Option<Key>>; 3],
    title_wanted: Option<rsui_core::Text>,
    pressed: Option<Press>,
    dragged: Option<Dragged>,
    /// Whether the pointer is dragging out a selection in the focused input.
    selecting: bool,
    /// Whether the system's dark, once that's been said.
    dark_mode: Option<bool>,
    /// Text selected outside of any input, of which there's only ever one.
    text_selection: Option<TextSelection>,
    /// Whether the pointer's selecting that, with its button down.
    selecting_text: bool,
    dragging: Option<ScrollDrag>,
    /// What's on top and has the keyboard to itself, if anything is.
    trap: Option<Vec<u32>>,
    /// Where the keyboard was before that, to go back to.
    focus_before_trap: Option<Vec<u32>>,
    /// Whether what happened last came from the keyboard, as that's who needs a focus ring.
    keyboard_last: bool,
    /// For a page's `:focus-visible`, as Chromium has it (`Document::HadKeyboardEvent`): whether a key has gone down (without Ctrl,
    /// Alt or the Windows key) since the pointer last moved the keyboard.
    had_keyboard_event: bool,
    /// Whether Tab has taken the keyboard out of a page, past its last stop or before its first, as a browser gives it to its own
    /// toolbar then: the page is told (`keyboardleft`), and told again when a press brings it back (`keyboardback`).
    keyboard_left: bool,
    /// For a page's `:focus-visible` too: whether the keyboard was last given to something by the pointer, rather than by a key, of
    /// what people do (code that moves it doesn't count), as Chromium's `Document::LastFocusType` being `kMouse`.
    last_focus_by_pointer: bool,
    /// What's under the pointer that has a tooltip, and since when, once there's something to show.
    tooltip: Option<(Vec<u32>, Instant)>,
    tooltip_showing: bool,
    /// Where the pointer was when it showed, which it stays by, rather than chasing the pointer.
    tooltip_at: (f32, f32),
    /// Someone's done something since, so it stays hidden until the pointer goes somewhere else.
    tooltip_dismissed: bool,
    focus: Option<Focus>,
    /// Whether this window is the one receiving the keyboard.
    active: bool,
    /// Whether the window can be seen, which is false while it's minimized.
    shown: bool,
    caret_on: bool,
    caret_changes_left: u32,
    clipboard: Box<dyn Clipboard>,
    /// What's kept for looking at the app while it runs, while something is.
    inspector: Option<Box<Inspector>>,
    /// When the frame being made started, in `rsui_spans`'s time, while spans are kept.
    frame_began: Option<u64>,
    /// How many frames have been drawn while spans are kept, which numbers them.
    frames_drawn: u32,
    /// The pill in the corner, in a window of a program being worked on.
    pill: crate::pill::Pill,
    /// What the pill draws, when there's no inspector to draw it with.
    pill_overlay: Overlay,
    /// Whether the next click picks what it's on for a viewer, rather than reaching the app, and what it picked.
    picking: bool,
    picked: Option<Vec<u32>>,
    /// Whether the release of the press that picked is to be kept from the app too.
    pick_release: bool,
    /// Whether this is the app that what another program asks of the app being looked at is done with. See
    /// [`run_in_app`](crate::inspect::run_in_app).
    live: bool,
    /// What's come in through [`App::feed`], once it's been asked to keep it. See [`crate::replay`].
    pub(crate) recording: Option<Box<crate::replay::Recording>>,
    /// Another program's window to keep inside this one's, or none to give back what's there, once asked.
    dock_wanted: Option<Option<crate::DockRequest>>,
    /// What's held on the keyboard, as was last said.
    modifiers: Modifiers,
    /// Which buttons are down, as the web's `buttons` has them.
    buttons: u8,
    /// Where the window's top left is on the screen, in physical pixels.
    screen_origin: (f32, f32),
    /// Whether anything under the pointer hears it move, as of when that was last worked out.
    moves_heard: bool,
    /// Whether the pointer's moved since what hears that was last told.
    move_due: bool,
    /// When that was, and where the pointer was then.
    last_move_at: Option<Instant>,
    last_sent_move: Option<(f32, f32)>,
    /// How often at most that's told.
    move_pace: Duration,
    /// What's held down with the main button that hears moves, which has them until it's let go of.
    captured: Option<Captured>,
    /// The element of a page that's asked for the pointer (`setPointerCapture()`), by its key there, while the main button's
    /// down: the button coming up goes to it, and so does the click, wherever the pointer is, as in a browser.
    pointer_capture: Option<Key>,
    /// Whether what has the pointer is what it went down on, or around it, which is then where the click is, however the page changed in
    /// between: a click is on what's around both where it went down and where it came up, and with capture that's it.
    capture_pressed: bool,
    /// Whether what the last key would type isn't to be, as something said so when it went down.
    swallow_typing: bool,
    /// What Space went down on, which is clicked when it comes up there, as a browser's button is, unless the keyboard's gone
    /// somewhere else by then.
    space_on: Option<Vec<u32>>,
    /// Whether something's being put together by a way of typing.
    composing: bool,
    /// What the main button went down on, for where a click is if nothing there takes clicks.
    down_at: Option<Vec<u32>>,
    /// How many presses in a row the one at `down_at` was, for the click there.
    down_clicks: u32,
    /// The keys along `down_at`.
    down_keys: Vec<Option<Key>>,
    /// A `pointerdown` was told not to do what it would, which keeps the mouse's events that go with the pointer's from being
    /// sent until a button comes up, as a browser has it.
    pub(crate) mouse_held_back: bool,
    /// What's selected in the field that has the keyboard, as the page was last told (see `publish_selection`): its key, and where
    /// it's held from and where the caret is.
    pub(crate) selection_told: Option<(String, usize, usize)>,
    /// Another button that's down, and what it went down on.
    other_down_at: Option<(PointerButton, Vec<u32>)>,
    /// What the main button went down on that can be dragged, as a page's `draggable` says, and where.
    drag_from: Option<(Vec<u32>, (f32, f32))>,
    /// What's being dragged, as a page has it, from inside the app or another program.
    html_drag: Option<events::DragSession>,
}

impl App {
    pub fn new<P: 'static, R: IntoNode + 'static>(
        render: fn(&mut Cx<'_>, &P) -> R,
        props: P,
        text: TextEngine,
        size: (u32, u32),
        scale: f32,
    ) -> Self {
        // What kind of window the main one is can only be changed on it now, whether it's on a screen or not.
        crate::window::main_opened();
        let inspected = crate::inspect::wanted_from_start();
        LAST_SCALE.with(|last| last.set(scale));
        crate::geometry::window_size((size.0 as f32 / scale, size.1 as f32 / scale));
        Self {
            runtime: if inspected.is_some() {
                Runtime::new_tracing(render, props)
            } else {
                Runtime::new(render, props)
            },
            layout: std::rc::Rc::new(std::cell::RefCell::new(Layout::new())),
            text,
            painter: Painter::new(),
            #[cfg(all(feature = "retained-frame", not(feature = "automatic-render")))]
            retained_frame: 0,
            #[cfg(feature = "mutation-pilot")]
            pilot: {
                #[cfg(feature = "automatic-render")]
                {
                    Some(mutation_pilot::PilotState::automatic())
                }
                #[cfg(not(feature = "automatic-render"))]
                {
                    None
                }
            },
            #[cfg(feature = "automatic-render")]
            render_binding: 0,
            #[cfg(feature = "automatic-render")]
            render_resources_reset: true,
            #[cfg(feature = "automatic-render")]
            external_runtime: None,
            #[cfg(feature = "idle-probe")]
            render_times: (Duration::ZERO, Duration::ZERO),
            scale,
            size,
            tree_changes: 0,
            laid_out: false,
            size_told: false,
            repaint: true,
            see_through: false,
            blinked: false,
            pointer: None,
            hover_stale: false,
            raw_hit_cell: None,
            hit_cells_enabled: {
                static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                *ENABLED.get_or_init(|| std::env::var("RSUI_HIT_CELLS").as_deref() != Ok("0"))
            },
            wheel_hit_memo: None,
            wheel_hit_active: false,
            wheel_hit_enabled: {
                static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                *ENABLED.get_or_init(|| std::env::var("RSUI_WHEEL_HIT_MEMO").as_deref() != Ok("0"))
            },
            #[cfg(test)]
            wheel_hit_counts: [0; 2],
            #[cfg(test)]
            hit_cell_counts: [0; 2],
            text_selection_stale: true,
            interactions_exposed: false,
            #[cfg(test)]
            interaction_counts: [0; 4],
            #[cfg(test)]
            conservative_interactions: false,
            hovered: None,
            animations: false,
            hover_since: None,
            hover_amount: 1.0,
            leaving: None,
            leaving_amount: 0.0,
            glide: None,
            input_at: None,
            frame_time: Duration::ZERO,
            resting_area: None,
            cursor: Cursor::Arrow,
            pointer_paths: [Vec::new(), Vec::new(), Vec::new()],
            pointer_keys: [Vec::new(), Vec::new(), Vec::new()],
            title_wanted: None,
            pressed: None,
            dragged: None,
            selecting: false,
            dark_mode: None,
            text_selection: None,
            selecting_text: false,
            dragging: None,
            trap: None,
            focus_before_trap: None,
            keyboard_last: false,
            had_keyboard_event: false,
            keyboard_left: false,
            last_focus_by_pointer: false,
            tooltip: None,
            tooltip_showing: false,
            tooltip_at: (0.0, 0.0),
            tooltip_dismissed: false,
            focus: None,
            active: true,
            shown: true,
            caret_on: true,
            caret_changes_left: CARET_CHANGES,
            clipboard: Box::new(MemoryClipboard::default()),
            inspector: None,
            frame_began: None,
            frames_drawn: 0,
            pill: crate::pill::Pill::default(),
            pill_overlay: Overlay::default(),
            picking: false,
            picked: None,
            pick_release: false,
            live: false,
            recording: None,
            dock_wanted: None,
            modifiers: Modifiers::default(),
            buttons: 0,
            screen_origin: (0.0, 0.0),
            moves_heard: false,
            move_due: false,
            last_move_at: None,
            last_sent_move: None,
            move_pace: Duration::ZERO,
            captured: None,
            pointer_capture: None,
            capture_pressed: false,
            swallow_typing: false,
            space_on: None,
            composing: false,
            down_at: None,
            down_clicks: 1,
            down_keys: Vec::new(),
            mouse_held_back: false,
            selection_told: None,
            other_down_at: None,
            drag_from: None,
            html_drag: None,
        }
        .inspected(inspected)
    }

    /// Starts inspecting, and showing it if `show`, for an app that's to be inspected from the start.
    fn inspected(mut self, wanted: Option<bool>) -> Self {
        if let Some(show) = wanted {
            self.set_inspecting(true);
            self.show_inspector(show);
        }
        self
    }

    /// Starts or stops keeping what renders, why, and how long each frame takes, for [`frames`](Self::frames). It costs
    /// nothing while it's off, which it is to begin with.
    pub fn set_inspecting(&mut self, on: bool) {
        if on == self.inspector.is_some() {
            return;
        }
        self.runtime.set_tracing(on);
        self.inspector = on.then(|| Box::new(Inspector::new()));
        // While it's being looked at, everything's kept, down to each render; otherwise, as little as was before.
        if on {
            rsui_spans::set_level(rsui_spans::Level::Detail);
        } else if rsui_spans::detail() {
            rsui_spans::set_level(if crate::inspect::inspector_allowed() {
                rsui_spans::Level::Light
            } else {
                rsui_spans::Level::Off
            });
        }
        self.invalidate_automatic();
        self.repaint = true;
    }

    /// What the inspector's keeping takes, for measuring memory, if it's inspecting.
    #[doc(hidden)]
    pub fn inspector_footprint(&self) -> Option<String> {
        self.inspector.as_ref().map(|inspector| inspector.footprint())
    }

    pub fn inspecting(&self) -> bool {
        self.inspector.is_some()
    }

    /// For something that draws only when [`update`](Self::update) says [`Frame::Needed`], as a window does, after it's said
    /// [`Frame::Unchanged`]: what was handled and rendered since the last frame is kept as a frame of its own, as if it had
    /// been drawn, with nothing drawn, so that what the inspector keeps still says what each step rendered and why. A window
    /// leaves it for its next frame instead.
    pub fn frame_unchanged(&mut self) {
        if let Some(inspector) = self.inspector.as_mut() {
            inspector.frame_done(false);
        }
    }

    /// Shows what's being kept in the window, or stops showing it: flashes where components render, a meter of where
    /// each frame's time goes, and why what's under the pointer last rendered. Showing it starts inspecting.
    pub fn show_inspector(&mut self, show: bool) {
        if show {
            self.set_inspecting(true);
        }
        if let Some(inspector) = self.inspector.as_mut() {
            inspector.showing = show;
            inspector.redraw = true;
        }
    }

    /// Shows what's at `path` filled in, with `name` and its size by it, as a browser's inspector does what's pointed at
    /// in it, or nothing for `None`. It's shown whether or not the rest of what ctrl+shift+I shows is.
    pub fn highlight(&mut self, at: Option<(Vec<u32>, String)>) {
        if at.is_some() {
            self.set_inspecting(true);
        }
        if let Some(inspector) = self.inspector.as_mut() {
            if inspector.highlight != at {
                inspector.highlight = at;
                inspector.redraw = true;
                self.invalidate_automatic();
                self.repaint = true;
            }
        }
    }

    /// Has this window show the pill in its corner, with how long frames take, which opens the inspector. It's only
    /// shown where the inspector's allowed.
    pub fn set_pill(&mut self, on: bool) {
        let on = on && crate::inspect::pill_allowed();
        if on != self.pill.wanted {
            self.pill.wanted = on;
            self.invalidate_automatic();
            self.repaint = true;
        }
    }

    /// Opens the inspector, as F12 does: in a window of its own, docked to this one, if the program can, and otherwise
    /// what's drawn over the app.
    pub fn open_inspector(&mut self) {
        match crate::inspect::opener() {
            Some(open) => open(self),
            None => self.show_inspector(!self.inspector_showing()),
        }
    }

    /// Has the next click on the app pick what it's on, for a viewer, rather than reach the app, with what the pointer's on
    /// filled in until then, as a browser's inspector does. [`take_picked`](Self::take_picked) says what was picked.
    pub fn set_picking(&mut self, on: bool) {
        self.picking = on;
        if !on {
            self.highlight(None);
        }
    }

    pub fn picking(&self) -> bool {
        self.picking
    }

    /// What a click picked, once, since [`set_picking`](Self::set_picking).
    pub fn take_picked(&mut self) -> Option<Vec<u32>> {
        self.picked.take()
    }

    /// The deepest element at `x, y`, in physical pixels, with what it is, for picking.
    fn element_at(&mut self, x: f32, y: f32) -> Option<(Vec<u32>, String)> {
        self.lay_out();
        let path = self.layout.borrow().hit_test(x / self.scale, y / self.scale)?;
        let node = find(self.runtime.root(), &path)?;
        let name = if node.name.is_empty() {
            node.role.name().to_owned()
        } else {
            format!("{} \u{201c}{}\u{201d}", node.role.name(), node.name)
        };
        Some((path, name))
    }

    /// Whether a pointer input is for picking, which handles it if so.
    fn pick_took(&mut self, input: &crate::replay::Input) -> bool {
        use crate::replay::Input;
        if self.pick_release {
            if let Input::Up { .. } = input {
                self.pick_release = false;
                return true;
            }
        }
        if !self.picking {
            return false;
        }
        match input {
            Input::Move { x, y } => {
                let at = self.element_at(*x, *y);
                self.highlight(at);
                true
            }
            Input::Down { x, y, .. } => {
                self.picked = self.element_at(*x, *y).map(|(path, _)| path);
                self.picking = false;
                self.pick_release = true;
                self.highlight(None);
                true
            }
            Input::Up { .. } => true,
            _ => false,
        }
    }

    /// Whether a pointer input is for the pill, or for picking, rather than the app, which it handles if so.
    pub(crate) fn pill_took(&mut self, input: &crate::replay::Input) -> bool {
        use crate::pill::PillHit;
        use crate::replay::Input;
        if self.pick_took(input) {
            return true;
        }
        if !self.pill.showing() && !self.pill.pressed {
            return false;
        }
        match input {
            Input::Move { x, y } => {
                let over = self.pill.hit(*x, *y).is_some();
                if over != self.pill.hovered {
                    self.pill.hovered = over;
                    self.invalidate_automatic();
                    self.repaint = true;
                }
                // The app still hears where the pointer is, unless it's on the pill.
                over
            }
            Input::Leave => {
                if self.pill.hovered {
                    self.pill.hovered = false;
                    self.invalidate_automatic();
                    self.repaint = true;
                }
                false
            }
            Input::Down { x, y, .. } => {
                if self.pill.hit(*x, *y).is_some() {
                    self.pill.pressed = true;
                    true
                } else {
                    false
                }
            }
            Input::Up { x, y } if self.pill.pressed => {
                self.pill.pressed = false;
                match self.pill.hit(*x, *y) {
                    Some(PillHit::Open) => self.open_inspector(),
                    Some(PillHit::Close) => self.pill.dismiss(),
                    None => {}
                }
                self.invalidate_automatic();
                self.repaint = true;
                true
            }
            _ => false,
        }
    }

    /// Draws where time went over the app, as a viewer worked it out: each element, how hot it is from 0 to 1, and what to
    /// say on it. Nothing takes it away. Like [`highlight`](Self::highlight), it's shown whether or not the rest is.
    pub fn set_heat(&mut self, heat: Vec<(Vec<u32>, f32, String)>) {
        if heat.is_empty() && self.inspector.is_none() {
            return;
        }
        self.set_inspecting(true);
        if let Some(inspector) = self.inspector.as_mut() {
            inspector.heat = heat;
            inspector.redraw = true;
        }
        self.invalidate_automatic();
        self.repaint = true;
    }

    pub fn inspector_showing(&self) -> bool {
        self.inspector
            .as_ref()
            .is_some_and(|inspector| inspector.showing)
    }

    /// The frames kept while inspecting, oldest first, for the last [`KEPT_FOR`](crate::inspect::KEPT_FOR).
    pub fn frames(&self) -> impl DoubleEndedIterator<Item = &FrameRecord> {
        self.inspector
            .iter()
            .flat_map(|inspector| inspector.frames().iter())
    }

    /// The component that made what's at `path`, numbered as the runtime numbers them, and what's known about why it
    /// rendered, while inspecting.
    pub fn why_at(&self, path: &[u32]) -> ((u32, u32), Option<String>) {
        let component = self.runtime.component_at(path);
        let why = self
            .inspector
            .as_ref()
            .and_then(|inspector| inspector.why(component));
        (component, why)
    }

    /// Sets where copied text goes, and pasted text comes from.
    pub fn set_clipboard(&mut self, clipboard: impl Clipboard + 'static) {
        self.clipboard = Box::new(clipboard);
    }

    /// Whether time, for what moves, is only what [`tick`](Self::tick) says, for a test that keeps its own clock (see
    /// [`Runtime::set_own_clock`](rsui_runtime::Runtime::set_own_clock)).
    pub fn set_own_clock(&mut self, on: bool) {
        self.runtime.set_own_clock(on);
    }

    /// Turns on smooth changes: colors that fade, scrolling that glides, and whatever components do
    /// with [`Cx::animate`](rsui_runtime::Cx::animate). They start off, so that an agent or a test sees
    /// each result straight away. A window turns them on, unless someone has asked Windows not to.
    pub fn set_animations(&mut self, on: bool) {
        self.animations = on;
        self.runtime.set_animations(on);
        self.painter.images().set_animations(on);
        if !on {
            self.finish_glide();
            self.hover_since = None;
            self.hover_amount = 1.0;
            self.leaving = None;
            self.invalidate_automatic();
            self.repaint = true;
        }
    }

    /// Whether anything's partway through moving. If it is, call [`tick`](Self::tick) and draw for
    /// each frame. Once it isn't, nothing needs doing, so being still costs nothing.
    pub fn animating(&self) -> bool {
        self.app_animating()
            || self
                .inspector
                .as_ref()
                .is_some_and(|inspector| inspector.flashing())
    }

    /// Whether the app itself has something moving, apart from what the inspector draws.
    fn app_animating(&self) -> bool {
        self.runtime.moves_every_frame_in_view(|path, node| self.motion_might_show(path, node))
            || self.hover_since.is_some()
            || self.leaving.is_some()
            || self.glide.is_some()
            || self.runtime.scrolled_waiting()
    }

    /// A deliberately narrow proof that a visual animation cannot affect this
    /// viewport. Unknown layout, content overflow, transforms and effects retain
    /// ordinary scheduling. Empty group boxes cover common loading placeholders.
    fn motion_might_show(&self, path: &[u32], node: &Node) -> bool {
        if !self.laid_out || node.role != Role::Group || !node.children.is_empty()
            || rsui_layout::shows_own_text(node)
        {
            return true;
        }
        let Some(rect) = self.layout.borrow().rect(path) else {
            return true;
        };
        let width = self.size.0 as f32 / self.scale;
        let height = self.size.1 as f32 / self.scale;
        if ![rect.x, rect.y, rect.width, rect.height, width, height].iter().all(|value| value.is_finite())
            || (rect.x + rect.width > 0.0 && rect.y + rect.height > 0.0 && rect.x < width && rect.y < height)
        {
            return true;
        }
        let mut ancestor = self.runtime.root();
        for depth in 0..=path.len() {
            if ancestor.props.iter().any(|(name, value)| {
                if depth == path.len() && (name.as_str().starts_with("shadow") || name.as_str() == "text_shadow") {
                    // Even zero blur can draw an offset/spread hard shadow.
                    return true;
                }
                if depth < path.len() && matches!(name.as_str(),
                    "shadows" | "shadow_blur" | "text_shadow" | "outline_width" | "clip_path")
                {
                    // A box's own shadow/outline does not come from its children;
                    // clipping only removes pixels. Ancestor filters/transforms
                    // still retain ordinary scheduling below.
                    return false;
                }
                match name.as_str() {
                    // Neither changes the extent of an empty box.
                    "opacity" | "disabled" => false,
                    "outline_width" => !matches!(value,
                        PropValue::Int(0) | PropValue::Float(0.0)),
                    _ => rsui_layout::moves_paint(name),
                }
            }) {
                return true;
            }
            if depth < path.len() {
                let Some(child) = ancestor.children.get(path[depth] as usize) else {
                    return true;
                };
                ancestor = child;
            }
        }
        false
    }

    /// How long the screen shows each frame for, which a window sets.
    pub fn set_frame_time(&mut self, time: Duration) {
        self.frame_time = time;
    }

    /// When what's taken in next happened, as a window's message says, if that was a moment ago: a frame that took long, or a
    /// page's code running, keeps a window from getting to what's waiting for it. A notch of the wheel glides from when it was
    /// turned, up to [`SCROLL_GLIDE`] / 2 before now, so scrolling stops when it would have had it been taken in at once, as a
    /// browser's does, whose wheel is taken in apart from the page, rather than going on for as long as the page held it up.
    pub fn set_input_time(&mut self, at: Instant) {
        self.input_at = Some(at);
    }

    /// How long the screen shows each frame for, as it's been said, or nothing.
    pub fn frame_time(&self) -> Duration {
        self.frame_time
    }

    /// Tells what hears that something's scrolled, as a browser fires `scroll`: once however often it moved since it was last
    /// told, where it is by then. A frame tells it as it starts ([`tick`](Self::tick)), before it's laid out and drawn, as a
    /// browser fires `scroll` in a frame's rendering steps before its animation frames, layout and paint: what a page moves
    /// when it scrolls, as a popover placed by script next to what it's for, is drawn in the same frame as the scrolling, never a
    /// frame behind and never held until the scrolling stops. What's scrolled when no frame is ticked, as for a host that only
    /// draws, is told once it's drawn ([`render`](Self::render), [`scene`](Self::scene)). Says whether it told anything.
    pub fn tell_scrolled(&mut self) -> bool {
        if !self.runtime.scrolled_waiting() {
            return false;
        }
        self.runtime.deliver_scrolled();
        true
    }

    /// Whether the inspector wants drawing again for itself, as what it draws fades or follows the pointer.
    fn inspector_redraw(&self) -> bool {
        self.inspector
            .as_ref()
            .is_some_and(|inspector| inspector.redraw)
    }

    /// Moves everything that's moving along to this time.
    pub fn tick(&mut self, now: Instant) {
        let _logged = crate::input_log::Span::new("phase-tick");
        self.runtime.tick(now);
        self.send_due_move();
        if let Some(inspector) = self.inspector.as_mut() {
            if inspector.flashing() {
                inspector.redraw = true;
            }
        }
        if let Some(since) = self.hover_since {
            self.hover_amount = eased(since, now, HOVER_FADE);
            self.invalidate_automatic();
            self.repaint = true;
            if now.saturating_duration_since(since) >= HOVER_FADE {
                self.hover_since = None;
                self.hover_amount = 1.0;
            }
        }
        if let Some((_, since, start)) = &self.leaving {
            self.leaving_amount = start * (1.0 - eased(*since, now, HOVER_FADE));
            #[cfg(feature = "automatic-render")]
            if let Some(p) = &mut self.pilot {
                p.full = true;
            }
            self.repaint = true;
            if now.saturating_duration_since(*since) >= HOVER_FADE {
                self.leaving = None;
                self.leaving_amount = 0.0;
            }
        }
        if let Some(glide) = &self.glide {
            let done = now.saturating_duration_since(glide.started) >= SCROLL_GLIDE;
            let place = if done { glide.to } else { glide.place(now) };
            let input = self.glide_input(glide, place, done);
            let result = self
                .runtime
                .dispatch(&glide.path, glide.action, Some(input));
            // If it couldn't go through, what was scrolling isn't there any more.
            if done || result.is_err() {
                self.glide = None;
            }
        }
        // What's scrolled since the last frame is told now, where this frame shows it: laid out there first, so what the page
        // asks of where things are is read from the window's own layout, and what it changes is applied with the rest of this
        // frame's changes, by the update that comes next (see `tell_scrolled`).
        if self.animations && self.runtime.scrolled_waiting() {
            self.update_now();
            self.lay_out();
            let split = rsui_alloc::split::on().then(|| rsui_alloc::split::enter(rsui_alloc::split::named("told")));
            self.tell_scrolled();
            drop(split);
        }
    }

    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut Runtime {
        self.reconcile_external_runtime();
        self.raw_hit_cell = None;
        self.wheel_hit_memo = None;
        self.interactions_exposed = true;
        self.text_selection_stale = true;
        self.invalidate_automatic();
        #[cfg(feature = "automatic-render")]
        {
            self.external_runtime = Some((self.runtime.id(), self.runtime.renders()));
        }
        &mut self.runtime
    }

    pub fn layout(&self) -> std::cell::Ref<'_, Layout> {
        self.layout.borrow()
    }

    /// Asks to be called when another thread has something to show. `waker` runs on that
    /// thread, so it should only signal this one, such as by posting a window message.
    pub fn set_waker(&mut self, waker: impl Fn() + Send + Sync + 'static) {
        let waker = std::sync::Arc::new(waker);
        if !self.live {
            self.live = crate::inspect::claim_live(waker.clone());
            // What comes in to the app being looked at is kept, for it to be replayed.
            if self.live {
                self.record_inputs();
            }
        }
        let for_images = waker.clone();
        self.painter.images().set_waker(move || for_images());
        self.runtime.set_waker(move || waker());
    }

    /// What the window has been asked to be called, since this last said.
    pub fn take_title_wanted(&mut self) -> Option<rsui_core::Text> {
        self.title_wanted.take()
    }

    /// What's been asked to be opened, since this last said. See [`crate::open_url`].
    pub fn take_urls_wanted(&mut self) -> Vec<rsui_core::Text> {
        self.runtime.take_urls_wanted()
    }

    /// Has the window keep another program's window inside it, along an edge, as a browser does its inspector, or give
    /// back the one that's there with `None`. See [`crate::DockRequest`].
    pub fn dock(&mut self, wanted: Option<crate::DockRequest>) {
        self.dock_wanted = Some(wanted);
        self.invalidate_automatic();
        self.repaint = true;
    }

    /// What's been asked to be docked inside the window, since this last said.
    pub fn take_dock_wanted(&mut self) -> Option<Option<crate::DockRequest>> {
        self.dock_wanted.take()
    }

    /// What the window's title bar has been asked to be, since this last said.
    pub fn take_dark_title_bar_wanted(&mut self) -> Option<bool> {
        self.runtime.take_dark_title_bar_wanted()
    }

    /// Says whether the system's set to dark colors, for anything that's asked to hear about it. Saying the
    /// same again does nothing.
    pub fn set_dark_mode(&mut self, dark: bool) {
        if self.dark_mode != Some(dark) {
            self.dark_mode = Some(dark);
            self.runtime.deliver_color_scheme(dark);
        }
    }

    /// Whether the top of what's shown has said it takes files that are dropped on it. A window that
    /// doesn't shouldn't show a pointer that says it does.
    pub fn accepts_files(&self) -> bool {
        self.path_to_action_on_top(ActionKind::Custom(symbol!("drop_files")))
            .is_some()
    }

    /// Where the top of what's shown is, if that offers this. That's the `window`, which has at most a little around it.
    fn path_to_action_on_top(&self, action: ActionKind) -> Option<Vec<u32>> {
        let mut node = self.runtime.root();
        let mut path = Vec::new();
        for _ in 0..4 {
            if node.actions.contains(action) {
                return Some(path);
            }
            if node.role == Role::Window {
                return None;
            }
            node = node.children.first()?;
            path.push(0);
        }
        None
    }

    /// The window's close button was clicked, or something like it, and this says whether what's in it wants to decide
    /// what happens, which it says by having a `close_request` action on the top of what's shown. Otherwise it closes.
    pub fn asked_to_close(&mut self) -> Result<bool, DispatchError> {
        self.update();
        let action = ActionKind::Custom(symbol!("close_request"));
        let Some(path) = self.path_to_action_on_top(action) else {
            return Ok(false);
        };
        self.runtime.dispatch(&path, action, None)?;
        self.update();
        Ok(true)
    }

    /// Files were dropped at this position, in physical pixels. The nearest thing there that takes them
    /// is given them, one path to a line.
    pub fn files_dropped(&mut self, x: f32, y: f32, paths: &[String]) -> Result<(), DispatchError> {
        self.update();
        self.lay_out();
        let action = ActionKind::Custom(symbol!("drop_files"));
        let Some(path) = self.nearest_at(x, y, |node| node.actions.contains(action)) else {
            return Ok(());
        };
        let input = PropValue::from(paths.join("\n"));
        self.runtime.dispatch(&path, action, Some(input))
    }

    /// Applies any state that has changed, and says whether the screen needs drawing.
    pub fn update(&mut self) -> Frame {
        let _logged = crate::input_log::Span::new("phase-update");
        // A frame starts with the first update after the last one was drawn, for the inspector's flame chart.
        if rsui_spans::on() && self.frame_began.is_none() {
            self.frame_began = Some(rsui_spans::now());
        }
        let span = rsui_spans::span(rsui_spans::Kind::Update);
        // With nothing moving, as for an agent or a test, what's scrolled is told at once rather than after a frame, so that
        // each result is there straight away.
        if !self.animations && self.runtime.scrolled_waiting() {
            self.runtime.deliver_scrolled();
        }
        let frame = self.update_now();
        drop(span);
        if frame == Frame::Unchanged {
            // Nothing's drawn for it, so it wasn't a frame.
            self.frame_began = None;
        }
        frame
    }

    fn update_now(&mut self) -> Frame {
        self.reconcile_external_runtime();
        if self.live {
            // What a viewer asks is done first, so that what it changes is shown in this frame.
            for job in crate::inspect::take_live_jobs() {
                job(self);
            }
        }
        self.reconcile_external_runtime();
        let split = rsui_alloc::split::enter(rsui_alloc::split::GEOMETRY);
        crate::geometry::changing(&self.layout);
        drop(split);
        let tree_owner = rsui_alloc::owners::scope(rsui_alloc::owners::TREE);
        let split = rsui_alloc::split::enter(rsui_alloc::split::FLUSH);
        let span = rsui_spans::span(rsui_spans::Kind::Flush);
        // What it was before, which says more about what the patches change (see `apply_needs_layout_since`).
        let before = self.runtime.root().clone();
        let patches = self.runtime.flush();
        drop(span);
        drop(split);
        drop(tree_owner);
        #[cfg(feature = "mutation-pilot")]
        let pilot_local = self.pilot.as_mut().is_some_and(|p| {
            #[cfg(feature = "automatic-render")]
            {
                p.automatic_patches(&patches) && self.painter.has_owned_palette()
            }
            #[cfg(not(feature = "automatic-render"))]
            {
                p.accepts(&patches)
            }
        });
        #[cfg(not(feature = "mutation-pilot"))]
        let pilot_local = false;
        if let Some(inspector) = self.inspector.as_mut() {
            inspector.take_from(&mut self.runtime);
        }
        if !patches.is_empty() {
            self.tree_changes += 1;
            let split = rsui_alloc::split::enter(rsui_alloc::split::LAYOUT_APPLY);
            let span = rsui_spans::span(rsui_spans::Kind::LayoutApply);
            let needs_layout = !pilot_local
                && self
                    .layout
                    .borrow_mut()
                    .apply_needs_layout_since(&patches, Some(&before));
            drop(span);
            drop(split);
            if needs_layout {
                self.laid_out = false;
            }
            // Paint reads these properties from the current tree. Keep any pending
            // layout invalidation, and prevent caret-only scene reuse on recolors.
            self.repaint = true;
            // These props neither change hit targets nor editable/selected text.
            // Classify the whole transaction: actions, hover participation and
            // structural/text changes retain the conservative behavior.
            let preserve = self.laid_out && preserves_interactions(&patches);
            #[cfg(test)]
            let preserve = preserve && !self.conservative_interactions;
            if !preserve {
                self.invalidate_interactions();
            }
        }
        if self.laid_out && self.repaint {
            // `changing` can release the tree on every update, including an empty
            // flush before painting. Publish it while reused boxes remain current.
            let _split = rsui_alloc::split::enter(rsui_alloc::split::GEOMETRY);
            crate::geometry::republish(&self.layout, self.runtime.root());
        }
        if let Some(key) = self.runtime.take_focus_wanted() {
            self.focus_on_key(&key);
        }
        if let Some((key, start, end)) = self.runtime.take_select_wanted() {
            self.select_in_focused(&key, start, end);
        }
        if let Some(wanted) = self.runtime.take_capture_wanted() {
            self.capture_pointer(wanted);
        }
        if let Some(title) = self.runtime.take_title_wanted() {
            self.title_wanted = Some(title);
        }
        if let Some(text) = self.runtime.take_copy_wanted() {
            self.clipboard.write(&text);
        }
        if self.runtime.take_paste_wanted() {
            let text = self.clipboard.read();
            self.runtime.deliver_clipboard(text.as_deref());
            return self.update_now();
        }
        if self.painter.images().arrived() || self.painter.images().canvases_changed() {
            #[cfg(feature = "mutation-pilot")]
            if let Some(p) = &mut self.pilot {
                p.full = true;
            }
            self.invalidate_automatic();
            self.repaint = true;
        }
        // Something that's just started moving may look no different yet, and still needs the next
        // frame for it to get anywhere.
        if self.laid_out
            && !self.repaint
            && !self.blinked
            && !self.animating()
            && !self.inspector_redraw()
        {
            Frame::Unchanged
        } else {
            Frame::Needed
        }
    }

    /// How big the window is, in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// How many physical pixels there are to a logical one.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn resize(&mut self, size: (u32, u32), scale: f32) {
        LAST_SCALE.with(|last| last.set(scale));
        crate::geometry::window_size((size.0 as f32 / scale, size.1 as f32 / scale));
        if size != self.size || scale != self.scale || !self.size_told {
            self.size = size;
            self.scale = scale;
            self.laid_out = false;
            self.text_selection_stale = true;
            self.size_told = true;
            crate::geometry::resized(&self.layout);
            self.runtime
                .deliver_resize(size.0 as f32 / scale, size.1 as f32 / scale);
        }
    }

    fn lay_out(&mut self) {
        let _logged = crate::input_log::Span::new("phase-layout");
        self.reconcile_external_runtime();
        if !self.laid_out {
            let started = self.inspector.is_some().then(Instant::now);
            self.lay_out_now();
            if let (Some(started), Some(inspector)) = (started, self.inspector.as_mut()) {
                inspector.layout_took(started.elapsed());
            }
        }
    }

    fn lay_out_now(&mut self) {
        let _owner = rsui_alloc::owners::scope(rsui_alloc::owners::LAYOUT);
        let _split = rsui_alloc::split::enter(rsui_alloc::split::LAYOUT);
        let mut span = rsui_spans::span(rsui_spans::Kind::Layout);
        self.text_selection_stale = true;
        {
            let available = Size::new(
                self.size.0 as f32 / self.scale,
                self.size.1 as f32 / self.scale,
            );
            let mut measuring = Measuring {
                text: &mut self.text,
                images: self.painter.images(),
            };
            self.layout
                .borrow_mut()
                .compute(self.runtime.root(), available, &mut measuring);
            if let Some(span) = span.as_mut() {
                // How many boxes were worked out, which says how much of the window this layout was about.
                span.arg(self.layout.borrow().last_solved() as u32);
            }
            if rsui_alloc::split::on() {
                rsui_alloc::split::count(rsui_alloc::split::named("n:solved"), self.layout.borrow().last_solved() as u64);
                rsui_alloc::split::count(rsui_alloc::split::named("n:looked"), self.layout.borrow().last_looked_at() as u64);
            }
            // Lists find out how many rows they have room for from the layout, and that changes
            // which rows they build, which doesn't change how tall they are. Once is enough, and
            // the limit is for something that isn't behaving.
            for _ in 0..3 {
                if self.layout.borrow().list_sizes().is_empty() {
                    break;
                }
                self.tell_lists_their_size();
                crate::geometry::changing(&self.layout);
                let tree_owner = rsui_alloc::owners::scope(rsui_alloc::owners::TREE);
                let split = rsui_alloc::split::enter(rsui_alloc::split::FLUSH);
                let flush = rsui_spans::span(rsui_spans::Kind::Flush);
                let patches = self.runtime.flush();
                drop(flush);
                drop(split);
                drop(tree_owner);
                self.tree_changes += 1;
                self.layout.borrow_mut().apply(&patches);
                self.invalidate_interactions();
                let mut measuring = Measuring {
                    text: &mut self.text,
                    images: self.painter.images(),
                };
                self.layout
                    .borrow_mut()
                    .compute(self.runtime.root(), available, &mut measuring);
            }
            self.laid_out = true;
            let split = rsui_alloc::split::enter(rsui_alloc::split::GEOMETRY);
            let geometry = rsui_spans::span(rsui_spans::Kind::Geometry);
            crate::geometry::laid_out(&self.layout, self.runtime.root());
            drop(geometry);
            drop(split);
            // Layout's own time ends here: what's told about it next is the app's.
            drop(span.take());
            // What watches where things are hears about it now, as a page's `ResizeObserver` does after layout. What it changes is
            // drawn on the next frame.
            let split = rsui_alloc::split::on().then(|| rsui_alloc::split::enter(rsui_alloc::split::named("laid-out")));
            self.runtime.deliver_laid_out();
            drop(split);
            // Whoever asked for this layout, it hasn't been drawn yet.
            self.invalidate_automatic();
            self.repaint = true;
            if self.layout.borrow().has_overlays() || self.trap.is_some() {
                self.follow_overlays();
            }
        }
    }

    fn tell_lists_their_size(&mut self) {
        let mut found = Vec::new();
        find_lists(
            self.runtime.root(),
            self.layout.borrow().list_sizes(),
            &mut Vec::new(),
            &mut found,
        );
        for (path, rows, across) in found {
            // A list refusing this can't be helped, and it'd only be asked again. A grid is told how many fit across as well.
            let input = Some(if across > 0.0 {
                PropValue::from(format!("{rows} {across}").as_str())
            } else {
                PropValue::Float(f64::from(rows))
            });
            let _ = self.runtime.dispatch(&path, ActionKind::Resize, input);
        }
    }

    #[cfg(feature = "idle-probe")]
    pub(crate) fn probe_render_times(&self) -> (Duration, Duration) {
        self.render_times
    }

    #[cfg(feature = "idle-probe")]
    pub(crate) fn probe_metadata(&self) -> (usize, usize) {
        #[cfg(feature = "automatic-render")]
        {
            let (fixed, dynamic) = self.painter.owned_pilot_metadata_storage();
            (
                fixed
                    + size_of::<Option<mutation_pilot::PilotState>>()
                    + size_of::<(u64, bool)>()
                    + size_of::<u64>(),
                dynamic,
            )
        }
        #[cfg(not(feature = "automatic-render"))]
        {
            (0, 0)
        }
    }

    // A mutable Runtime can consume its own patches (or be replaced). Merely
    // dispatching an action still leaves App's ordered flush intact, so it should
    // not force layout for palette-only updates.
    fn reconcile_external_runtime(&mut self) {
        let exposed = std::mem::take(&mut self.interactions_exposed);
        #[cfg(feature = "automatic-render")]
        let exposed = if self
            .external_runtime
            .take()
            .is_some_and(|old| old != (self.runtime.id(), self.runtime.renders()))
        {
            *self.layout.borrow_mut() = Layout::new();
            self.laid_out = false;
            self.repaint = true;
            self.tree_changes += 1;
            self.invalidate_automatic();
            true
        } else {
            exposed
        };
        if exposed {
            self.invalidate_interactions();
        }
    }

    fn invalidate_interactions(&mut self) {
        self.raw_hit_cell = None;
        self.wheel_hit_memo = None;
        self.hover_stale = true;
        self.text_selection_stale = true;
        self.follow_focus();
    }

    fn invalidate_automatic(&mut self) {
        #[cfg(all(feature = "retained-frame", not(feature = "automatic-render")))]
        {
            self.retained_frame = 0;
        }
        #[cfg(feature = "automatic-render")]
        if let Some(p) = &mut self.pilot {
            p.full = true;
        }
    }

    /// Draws the current state, and then tells what hears that something's scrolled (see [`tell_scrolled`](Self::tell_scrolled)).
    /// Keeps where a frame's time went that was drawn from `started`, handed to the GPU by `drawn`, and taken by the
    /// screen now, for the inspector's flame chart and the corner pill.
    fn frame_drawn(&mut self, started: Instant, drawn: Instant) {
        if !rsui_spans::on() {
            return;
        }
        let (start, handed, end) = (
            rsui_spans::nanos(started),
            rsui_spans::nanos(drawn),
            rsui_spans::now(),
        );
        rsui_spans::record(rsui_spans::Kind::Gpu, start, handed, 0, 0);
        rsui_spans::record(rsui_spans::Kind::Present, handed, end, 0, 0);
        self.frames_drawn += 1;
        let began = self.frame_began.take().unwrap_or(start).min(start);
        // A frame is the app's own time, up to handing it to the GPU: waiting for the screen after is its own span.
        rsui_spans::record(rsui_spans::Kind::Frame, began, handed, 0, self.frames_drawn);
        self.pill.frame(began, start, handed, end);
    }

    pub fn render(&mut self, renderer: &mut Renderer) -> Result<(), RenderError> {
        let result = self.draw_only(renderer);
        if result.is_ok() {
            self.tell_scrolled();
        }
        result
    }

    /// [`render`](Self::render), without telling what's scrolled afterwards, for whoever does that separately.
    #[doc(hidden)]
    pub fn draw_only(&mut self, renderer: &mut Renderer) -> Result<(), RenderError> {
        let _logged = crate::input_log::Span::new("phase-frame");
        // Paint can keep a window's timer from running, including the frame where its last animation finishes.
        if self.send_due_move() {
            self.update();
        }
        #[cfg(feature = "automatic-render")]
        {
            if !renderer.bind_automatic_app(&mut self.render_binding)
                && !self.render_resources_reset
            {
                self.renderer_replaced();
            }
            self.render_resources_reset = false;
            self.pilot
                .get_or_insert_with(mutation_pilot::PilotState::automatic)
                .painting = true;
            self.prepare_scene::<false>();
            self.pilot.as_mut().expect("automatic state").painting = false;
            let started = (self.inspector.is_some()
                || cfg!(feature = "idle-probe")
                || rsui_spans::on())
            .then(Instant::now);
            let draw_log = crate::input_log::Span::new("phase-draw-api");
            #[cfg(feature = "idle-probe")]
            if std::env::var_os("RSUI_011D_GPU").is_some() {
                let (_, gpu, status) = renderer
                    .time_draw_for_diagnostics(|r| r.draw_owned_pilot(&mut self.painter))?;
                println!("gpu_ns={:?} status={status}", gpu.map(|v| v.as_nanos()));
            } else {
                renderer.draw_owned_pilot(&mut self.painter)?;
            }
            #[cfg(not(feature = "idle-probe"))]
            renderer.draw_owned_pilot(&mut self.painter)?;
            drop(draw_log);
            let drawn = started.map(|_| Instant::now());
            #[cfg(feature = "idle-probe")]
            if std::env::var_os("RSUI_011D_PIXELS").is_some() {
                println!(
                    "window_hash={:016x}",
                    renderer.current_window_hash_for_diagnostics()?
                );
            }
            let _present_log = crate::input_log::Span::new("phase-present-api");
            let result = renderer.present();
            drop(_present_log);
            if let (Some(started), Some(drawn)) = (started, drawn) {
                self.frame_drawn(started, drawn);
            }
            #[cfg(feature = "idle-probe")]
            {
                self.render_times = (
                    drawn.expect("probe timing") - started.expect("probe timing"),
                    drawn.expect("probe timing").elapsed(),
                );
            }
            if let (Some(started), Some(drawn), Some(inspector)) =
                (started, drawn, self.inspector.as_mut())
            {
                inspector.gpu_took(drawn - started, drawn.elapsed());
            }
            result
        }
        #[cfg(not(feature = "automatic-render"))]
        self.render_legacy(renderer)
    }

    #[cfg(not(feature = "automatic-render"))]
    fn render_legacy(&mut self, renderer: &mut Renderer) -> Result<(), RenderError> {
        #[cfg(feature = "retained-frame")]
        {
            self.reconcile_external_runtime();
            if self.retained_frame != 0
                && self.laid_out
                && !self.repaint
                && !self.blinked
                && !self.hover_stale
                && !self.text_selection_stale
                && self.inspector.is_none()
                && !self.animating()
                && !self.painter.images().arrived()
                && !self.painter.images().canvases_changed()
                && renderer.reuse_frame(self.retained_frame)
            {
                let _present_log = crate::input_log::Span::new("phase-present-api");
                return renderer.present();
            }
        }
        let timed = self.inspector.is_some()
            || cfg!(feature = "idle-probe")
            || crate::input_log::on()
            || rsui_spans::on();
        let see_through = self.see_through;
        let scene = self.scene_now();
        let clear = clear_color(scene.window_color, see_through);
        if !timed {
            let result = renderer.render(scene, clear);
            if result.is_ok() {
                self.clear_rendered_commands();
                #[cfg(feature = "retained-frame")]
                {
                    self.retained_frame = renderer.frame_token();
                }
            }
            return result;
        }
        // Handing the frame to the GPU, and waiting for the screen to take it, are told apart, as the wait isn't the
        // app's to spend.
        let started = Instant::now();
        let draw_log = crate::input_log::Span::new("phase-draw-api");
        #[cfg(feature = "idle-probe")]
        if std::env::var_os("RSUI_011D_GPU").is_some() {
            let (_, gpu, status) = renderer.time_draw_for_diagnostics(|r| r.draw(scene, clear))?;
            println!("gpu_ns={:?} status={status}", gpu.map(|v| v.as_nanos()));
        } else {
            renderer.draw(scene, clear)?;
        }
        #[cfg(not(feature = "idle-probe"))]
        renderer.draw(scene, clear)?;
        drop(draw_log);
        let drawn = Instant::now();
        #[cfg(feature = "idle-probe")]
        if std::env::var_os("RSUI_011D_PIXELS").is_some() {
            println!(
                "window_hash={:016x}",
                renderer.current_window_hash_for_diagnostics()?
            );
        }
        let present_log = crate::input_log::Span::new("phase-present-api");
        let result = renderer.present();
        drop(present_log);
        self.frame_drawn(started, drawn);
        #[cfg(feature = "idle-probe")]
        {
            self.render_times = (drawn - started, drawn.elapsed());
        }
        if result.is_ok() {
            self.clear_rendered_commands();
            #[cfg(feature = "retained-frame")]
            {
                self.retained_frame = renderer.frame_token();
            }
        }
        if let Some(inspector) = self.inspector.as_mut() {
            inspector.gpu_took(drawn - started, drawn.elapsed());
        }
        result
    }

    #[cfg(not(feature = "automatic-render"))]
    fn clear_rendered_commands(&mut self) {
        // Render consumed these commands (usually swapping in the previous frame).
        // App always prepares commands again before drawing, including caret reuse.
        // Clear lengths now so idle can reclaim a large previous view's capacity.
        // Resource queues are independent and must survive until consumed.
        let scene = self.painter.last_scene();
        scene.quads.clear();
        scene.batches.clear();
        scene.transforms.clear();
        scene.filters.clear();
        scene.round_clips.clear();
    }

    #[cfg(feature = "render-diagnostics")]
    #[doc(hidden)]
    pub fn prepared_scene_for_pilot_probe(&mut self) -> &mut Scene {
        self.invalidate_automatic();
        #[cfg(feature = "automatic-render")]
        {
            self.render_binding = 0;
            self.render_resources_reset = false;
        }
        self.painter.last_scene()
    }

    /// Works out what to draw for the current state, without drawing it. Normally
    /// [`render`](Self::render) does this. As a frame would, it then tells what hears that something's scrolled.
    pub fn scene(&mut self) -> &mut Scene {
        self.invalidate_automatic();
        #[cfg(feature = "automatic-render")]
        {
            self.render_binding = 0;
            self.render_resources_reset = false;
        }
        self.prepare_scene::<false>();
        self.tell_scrolled();
        self.painter.last_scene()
    }

    /// [`scene`](Self::scene), for drawing it, which tells what's scrolled once it's drawn.
    #[cfg(not(feature = "automatic-render"))]
    fn scene_now(&mut self) -> &mut Scene {
        self.invalidate_automatic();
        self.prepare_scene::<false>();
        self.painter.last_scene()
    }

    /// Full Painter traversal of the current visual state for independently replayed Apps.
    /// Shares ordinary layout, hover, selection and caret preparation, but bypasses both
    /// caret-scene reuse shortcuts. This is not a force-GPU-redraw API: configure the
    /// reference Renderer separately. Layout and text/image/atlas caches remain warm;
    /// reconstruct a fresh App when a cold-history comparison is needed.
    #[cfg(feature = "render-diagnostics")]
    pub fn scene_full_for_diagnostics(&mut self) -> &mut Scene {
        self.invalidate_automatic();
        #[cfg(feature = "automatic-render")]
        {
            self.render_binding = 0;
            self.render_resources_reset = false;
        }
        self.prepare_scene::<true>();
        self.painter.last_scene()
    }

    /// Latest Painter work record, saved before any renderer command-list swaps.
    /// Reading it does not prepare a scene, update state or consume resource queues.
    #[cfg(feature = "render-diagnostics")]
    pub fn scene_work(&self) -> rsui_render::diagnostics::SceneWork {
        self.painter.scene_work()
    }

    /// Explicit on-demand Painter storage census, not RSS or exact GPU residency.
    /// May scan retained collections; does not consume resources or prepare a scene.
    #[cfg(feature = "render-diagnostics")]
    pub fn render_storage(&self) -> rsui_render::diagnostics::PainterStorage {
        self.painter.retained_storage()
    }

    fn prepare_scene<const FULL: bool>(&mut self) {
        let _logged = crate::input_log::Span::new("phase-prepare");
        self.reconcile_external_runtime();
        let _owner = rsui_alloc::owners::scope(rsui_alloc::owners::SCENE);
        let _split = rsui_alloc::split::enter(rsui_alloc::split::PAINT);
        let _span = rsui_spans::span(rsui_spans::Kind::Paint);
        #[cfg(feature = "mutation-pilot")]
        {
            let invalid = !self.laid_out || self.animating() || self.inspector.is_some();
            if let Some(p) = &mut self.pilot {
                p.full |= invalid || !p.painting;
            }
        }
        // A blink with nothing else changed is the last scene with the caret in or out. That's what a focused input does
        // twice a second while it's left alone, which would otherwise be the whole window worked out again each time.
        // The first blink after anything else works it all out, with the caret showing, and keeps that for the rest.
        // Whether the app needs this frame, rather than only the inspector, whose frames aren't counted.
        let app_wanted = self.repaint
            || self.blinked
            || !self.laid_out
            || self.hover_stale
            || self.app_animating();
        let only_blinked = std::mem::take(&mut self.blinked)
            && !self.repaint
            && self.laid_out
            && !self.hover_stale
            && !self.animating()
            && !self.inspector_redraw();
        if !FULL && only_blinked && self.painter.can_show_caret() {
            let show = self.caret_on && self.active;
            #[cfg(feature = "mutation-pilot")]
            if self.pilot.as_ref().is_some_and(|p| p.painting) {
                assert!(self.painter.show_owned_caret(show));
                return;
            }
            self.painter
                .show_caret(show)
                .expect("there's a scene to show");
            return;
        }
        if !FULL && only_blinked {
            self.painter.keep_next_for_blinking();
        }
        self.lay_out();
        let split = rsui_alloc::split::on().then(|| rsui_alloc::split::enter(rsui_alloc::split::named("paint:hover")));
        if self.hover_stale {
            self.hover_stale = false;
            self.update_hover();
        }
        drop(split);
        let split = rsui_alloc::split::on().then(|| rsui_alloc::split::enter(rsui_alloc::split::named("paint:caret-in-view")));
        self.keep_caret_in_view();
        drop(split);
        let split = rsui_alloc::split::on().then(|| rsui_alloc::split::enter(rsui_alloc::split::named("paint:text-selection")));
        self.check_text_selection();
        drop(split);
        self.repaint = false;

        let root = self.runtime.root();
        let at = |path: &[u32]| find(root, path).map(|node| &**node);
        let focus = self.focus.as_ref().filter(|_| self.active);
        let focused = focus.and_then(|focus| Some((focus, at(&focus.path)?)));
        let interaction = Interaction {
            hovered: self.hovered.as_deref().and_then(at),
            hover_amount: self.hover_amount,
            leaving: self
                .leaving
                .as_ref()
                .and_then(|(path, _, _)| Some((at(path)?, self.leaving_amount))),
            resting_area: self
                .resting_area
                .as_ref()
                .and_then(|(path, scroll)| Some((at(path)?, *scroll))),
            pressed: self.pressed.as_ref().and_then(|press| at(&press.path)),
            ring: focused
                .filter(|(focus, _)| focus.ring || focus.edit.is_some())
                .map(|(_, element)| element),
            editing: focused.and_then(|(focus, element)| {
                focus.edit.as_ref()?;
                let carets = focus.carets.as_ref()?;
                Some(Editing {
                    element,
                    selection: &carets.selection_x,
                    // A field that can't be changed shows what's selected in it, but no caret, as a browser has it.
                    caret: ((self.caret_on || (!FULL && only_blinked)) && !is_read_only(element))
                        .then_some(carets.caret_x),
                    scroll: focus.scroll,
                    lines: is_multiline(element).then_some(EditingLines {
                        selection: &carets.selection_lines,
                        caret: carets.caret_line,
                        scroll: focus.scroll_y,
                    }),
                })
            }),
            text_selection: self
                .text_selection
                .as_ref()
                .and_then(|selection| Some((at(&selection.path)?, selection.edit.selection()))),
            tooltip: self
                .tooltip_showing
                .then(|| {
                    let (path, _) = self.tooltip.as_ref()?;
                    let text = tooltip_of(at(path)?)?;
                    Some((text, self.tooltip_at.0, self.tooltip_at.1))
                })
                .flatten(),
            overlay: None,
        };
        if self.see_through != self.painter.draws_text_gray() {
            #[cfg(feature = "mutation-pilot")]
            if let Some(p) = &mut self.pilot {
                p.full = true;
            }
            self.painter.draw_text_gray(self.see_through);
        }
        let system_glyphs = self.runtime.system_glyphs();
        if system_glyphs != self.painter.draws_glyphs_as_the_system_does() {
            #[cfg(feature = "mutation-pilot")]
            if let Some(p) = &mut self.pilot {
                p.full = true;
            }
            self.painter.draw_glyphs_as_the_system_does(system_glyphs);
        }
        let started = self.inspector.is_some().then(Instant::now);
        if let Some(inspector) = self.inspector.as_mut() {
            inspector.build_overlay(
                &self.runtime,
                &self.layout.borrow(),
                self.scale,
                self.size,
                self.pointer,
            );
        }
        // The pill goes on top of what the inspector draws, or on its own.
        self.pill_overlay.clear();
        if self.pill.showing() {
            let overlay = match self.inspector.as_mut() {
                Some(inspector) => &mut inspector.overlay,
                None => &mut self.pill_overlay,
            };
            self.pill.draw(overlay, self.scale, self.size);
        }
        let interaction = Interaction {
            overlay: self
                .inspector
                .as_ref()
                .map(|inspector| &inspector.overlay)
                .filter(|overlay| !overlay.is_empty())
                .or(Some(&self.pill_overlay).filter(|overlay| !overlay.is_empty())),
            ..interaction
        };
        #[cfg(feature = "mutation-pilot")]
        if let Some(p) = &mut self.pilot {
            if p.painting {
                let target = find(root, p.path()).map(|n| &**n);
                if !p.full
                    && self.inspector.is_none()
                    && target.is_some_and(|e| self.painter.update_owned_pilot(e, interaction))
                {
                    return;
                }
                self.painter.enroll_owned_pilot(target);
                self.painter.paint_owned_pilot(
                    root,
                    &self.layout.borrow(),
                    &mut self.text,
                    self.scale,
                    self.size,
                    interaction,
                );
                p.full = false;
                if !FULL && only_blinked && self.painter.can_show_caret() {
                    assert!(self.painter.show_owned_caret(self.caret_on));
                }
                if let (Some(started), Some(inspector)) = (started, self.inspector.as_mut()) {
                    inspector.paint_took(started.elapsed());
                    inspector.frame_done(app_wanted);
                    inspector.redraw = false;
                }
                return;
            }
        }
        self.painter.paint_with(
            root,
            &self.layout.borrow(),
            &mut self.text,
            self.scale,
            self.size,
            interaction,
        );
        if let (Some(started), Some(inspector)) = (started, self.inspector.as_mut()) {
            inspector.paint_took(started.elapsed());
            inspector.frame_done(app_wanted);
            inspector.redraw = false;
        }
        // If it was kept, it was worked out with the caret showing, whether it is or not. If there was no caret to keep,
        // it's as it was worked out.
        if !FULL && only_blinked && self.painter.can_show_caret() {
            self.painter
                .show_caret(self.caret_on)
                .expect("there's a scene to show");
        }
    }

    /// Where the caret is, in physical pixels from the top left of the window, and how tall a line is there, if
    /// something's being edited.
    pub fn caret_place(&mut self) -> Option<(f32, f32, f32)> {
        self.lay_out();
        self.keep_caret_in_view();
        let focus = self.focus.as_ref()?;
        focus.edit.as_ref()?;
        let carets = focus.carets.as_ref()?;
        let node = find(self.runtime.root(), &focus.path)?;
        let rect = self.layout.borrow().rect(&focus.path)?;
        let content = content_box(node, rect, self.scale);
        let mut style = TextStyle::from_element(node);
        style = style.scaled(self.scale);
        let height = self.text.line_height(style);
        if is_multiline(node) {
            let y = content.top + carets.caret_line as f32 * height - focus.scroll_y;
            return Some((content.left + carets.caret_x, y, height));
        }
        let indent = input_indent(&mut self.text, node, input_value(node), style, content.width);
        Some((
            content.left + indent + carets.caret_x - focus.scroll,
            content.top,
            height,
        ))
    }

    /// Moves the focused input's text sideways, if needed for the caret to be in view.
    fn keep_caret_in_view(&mut self) {
        let Some(focus) = &mut self.focus else {
            return;
        };
        let Some(edit) = &focus.edit else {
            return;
        };
        let (Some(node), Some(rect)) = (
            find(self.runtime.root(), &focus.path),
            self.layout.borrow().rect(&focus.path),
        ) else {
            return;
        };
        let width = content_box(node, rect, self.scale).width;
        let (caret, selection) = (edit.caret(), edit.selection());
        let upstream = focus.upstream == Some(caret);
        if focus.carets.as_ref().is_some_and(|carets| {
            carets.caret == caret
                && carets.upstream == upstream
                && carets.selection == selection
                && carets.scale == self.scale
                && carets.width == width
        }) {
            return;
        }
        #[cfg(test)]
        {
            self.interaction_counts[3] += 1;
        }
        let mut style = TextStyle::from_element(node);
        style = style.scaled(self.scale);
        let value = input_value(node);
        if is_multiline(node) {
            let height = content_box(node, rect, self.scale).height;
            let line_height = self.text.line_height(style);
            let starts = self.text.wrapped_starts(value, style, width);
            let (caret_line, caret_x) =
                place_in_lines(&mut self.text, value, style, width, &starts, caret, upstream);
            let mut pieces = Vec::new();
            let mut pieces_lines = Vec::new();
            if selection.0 != selection.1 {
                let first = line_of(&starts, selection.0);
                let last = line_of(&starts, selection.1);
                for line in first..=last {
                    let start = starts[line];
                    let text = wrapped_line(value, &starts, line);
                    let from = selection.0.max(start) - start;
                    let to = (selection.1.max(start) - start).min(text.len());
                    let indent = self.text.line_indent(text, style, width);
                    let before = pieces.len();
                    if from < to {
                        self.text
                            .selection_spans(text, style, (from, to), &mut pieces);
                    }
                    for piece in &mut pieces[before..] {
                        piece.0 += indent;
                        piece.1 += indent;
                    }
                    // Selecting a line's end shows as a little more, to tell that it's selected.
                    if line < last {
                        let edge = indent + self.text.line_width(text, style);
                        pieces.push((edge, edge + 4.0 * self.scale));
                    }
                    pieces_lines.resize(pieces.len(), line as u32);
                }
            }
            let total = starts.len() as f32 * line_height;
            let caret_top = caret_line as f32 * line_height;
            focus.scroll = 0.0;
            focus.scroll_y = focus
                .scroll_y
                .min(caret_top)
                .max(caret_top + line_height - height)
                .min((total - height).max(0.0))
                .max(0.0);
            focus.carets = Some(Carets {
                caret,
                upstream,
                selection,
                scale: self.scale,
                width,
                caret_x,
                selection_x: pieces,
                selection_lines: pieces_lines,
                caret_line: caret_line as u32,
                line_count: starts.len() as u32,
            });
            return;
        }
        // In a password, what's measured is what shows, with places moved to match.
        let mask = password_mask(node);
        let (shown_caret, shown_selection) = if mask.is_some() {
            (
                byte_in_mask(value, caret),
                (
                    byte_in_mask(value, selection.0),
                    byte_in_mask(value, selection.1),
                ),
            )
        } else {
            (caret, selection)
        };
        let value = mask.as_deref().unwrap_or(value);
        let mut positions = [(shown_caret, 0.0), (value.len(), 0.0)];
        self.text.caret_positions(value, style, &mut positions);
        let caret_x = positions[0].1;
        let mut selection_x = focus.carets.take().map_or_else(Vec::new, |mut carets| {
            carets.selection_x.clear();
            carets.selection_x
        });
        if selection.0 != selection.1 {
            self.text
                .selection_spans(value, style, shown_selection, &mut selection_x);
        }
        // How wide the text is, whichever side of it is the far end.
        let end = if self.text.may_reorder(value, style) {
            self.text.line_width(value, style)
        } else {
            positions[1].1
        };
        // Leave room for the caret itself at the far end.
        let room = (width - self.scale.round().max(1.0)).max(0.0);
        focus.scroll = focus
            .scroll
            .min(caret_x)
            .max(caret_x - room)
            .min((end - room).max(0.0))
            .max(0.0);
        focus.carets = Some(Carets {
            caret,
            upstream: false,
            selection,
            scale: self.scale,
            width,
            caret_x,
            selection_x,
            selection_lines: Vec::new(),
            caret_line: 0,
            line_count: 1,
        });
    }

    /// Has the window seen through where nothing's drawn, or not, as it is while it has a backdrop. Where the window's
    /// own background is transparent, or isn't said, nothing's drawn, and its text is gray rather than ClearType. What
    /// has a color is as opaque as that is. The window has to be told too: see `Renderer::set_see_through`.
    pub fn set_see_through(&mut self, on: bool) {
        if on != self.see_through {
            self.see_through = on;
            self.repaint = true;
            self.invalidate_automatic();
        }
    }

    /// Whether the window is seen through where nothing's drawn. See [`set_see_through`](Self::set_see_through).
    pub fn see_through(&self) -> bool {
        self.see_through
    }

    /// Forgets all glyphs, since a new renderer starts with an empty atlas.
    pub fn renderer_replaced(&mut self) {
        #[cfg(feature = "automatic-render")]
        {
            self.render_resources_reset = true;
        }
        self.invalidate_automatic();
        self.repaint = true;
        self.painter.renderer_replaced();
    }

    /// How much GPU memory images are using, in bytes.
    pub fn images_gpu_bytes(&mut self) -> usize {
        self.painter.images().gpu_bytes()
    }

    /// Waits for images being read to finish, up to `timeout`, and says whether they did. A window
    /// has no need for this, as it's told when each one's ready. It's for tests and screenshots.
    pub fn wait_for_images(&mut self, timeout: std::time::Duration) -> bool {
        let finished = self.painter.images().wait(timeout);
        self.update();
        finished
    }

    /// Where the middle of what's at `path` is shown, which is where a click on it lands: the middle of where it's laid out,
    /// taken through its transform and each one of what it's in, as a Radix menu, laid out at 0, 0 and moved by `transform` to
    /// where it's shown, is. In the layout's pixels, as [`Layout::rect`](rsui_layout::Layout::rect) says.
    pub fn shown_middle(&mut self, path: &[u32]) -> Option<(f32, f32)> {
        self.lay_out();
        let layout = self.layout.borrow();
        let rect = layout.rect(path)?;
        let (mut x, mut y) = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
        if !self.painter.has_transforms() {
            return Some((x, y));
        }
        let root = self.runtime.root();
        for depth in (0..=path.len()).rev() {
            let here = &path[..depth];
            let Some(around) = layout.rect(here) else {
                continue;
            };
            let mut node = root;
            for &index in here {
                node = node.children.get(index as usize)?;
            }
            if let Some([a, b, c, d, e, f]) =
                rsui_render::transform_of(node, around.x, around.y, around.width, around.height, 1.0)
            {
                (x, y) = (a * x + c * y + e, b * x + d * y + f);
            }
        }
        Some((x, y))
    }

    /// What a pointer at this position would land on, nearest the front first, then each
    /// node around it. Positions are in physical pixels.
    fn path_at(&mut self, x: f32, y: f32) -> Option<Vec<u32>> {
        self.path_at_inner(x, y, false)
    }

    fn path_at_inner(&mut self, x: f32, y: f32, idle_hover: bool) -> Option<Vec<u32>> {
        #[cfg(feature = "idle-probe")]
        let _native_cycles = crate::native_cycles::enter(crate::native_cycles::PATH_AT);
        {
            #[cfg(feature = "idle-probe")]
            let _native_cycles = crate::native_cycles::enter(crate::native_cycles::PATH_LAY_OUT);
            self.lay_out();
        }
        let turned = self.painter.has_transforms();
        let raw = self.raw_pointer_at(x / self.scale, y / self.scale, turned, idle_hover)?;
        let (mut path, cut) = refine_pointer_path(self.runtime.root(), &self.layout.borrow(), raw,
            x / self.scale, y / self.scale);
        if cut {
            return Some(path);
        }
        if self.layout.borrow().has_paragraphs() {
            let node = node_at(self.runtime.root(), &path);
            if !node.children.is_empty() && is_paragraph(node) {
                if let Some(rect) = self.layout.borrow().rect(&path) {
                    let content = content_box(node, rect, self.scale);
                    let part = {
                        #[cfg(feature = "idle-probe")]
                        let _native_cycles = crate::native_cycles::enter(crate::native_cycles::PARAGRAPH_PART);
                        self.text.paragraph_part_at(
                            node,
                            self.scale,
                            content.width,
                            (x - content.left, y - content.top),
                        )
                    };
                    path.extend(part.unwrap_or_default());
                }
            }
        }
        Some(path)
    }

    fn raw_pointer_at(&mut self, x: f32, y: f32, turned: bool, idle_hover: bool) -> Option<Vec<u32>> {
        if self.wheel_hit_active && self.wheel_hit_enabled {
            let layout = self.layout.borrow();
            let key = RawHitKey {
                root: std::ptr::from_ref::<Element>(self.runtime.root()).addr(),
                tree_changes: self.tree_changes,
                geometry: crate::geometry::generation(),
                size: self.size,
                scale: self.scale.to_bits(),
                turned: turned || layout.has_stacking() || layout.has_overlays(),
            };
            let point = [x.to_bits(), y.to_bits()];
            if let Some(memo) = &self.wheel_hit_memo
                && memo.key == key && memo.point == point
            {
                #[cfg(test)]
                { self.wheel_hit_counts[0] += 1; }
                return memo.path.clone();
            }
            #[cfg(test)]
            { self.wheel_hit_counts[1] += 1; }
            let path = raw_pointer_path(self.runtime.root(), &layout, x, y, turned);
            self.wheel_hit_memo = Some(WheelRawHit { key, point, path: path.clone() });
            return path;
        }
        #[cfg(test)]
        if self.wheel_hit_active { self.wheel_hit_counts[1] += 1; }
        let eligible = idle_hover && self.hit_cells_enabled
            && self.buttons == 0 && self.pressed.is_none() && self.down_at.is_none() && self.space_on.is_none()
            && self.captured.is_none() && self.pointer_capture.is_none()
            && self.dragged.is_none() && self.dragging.is_none()
            && self.drag_from.is_none() && self.html_drag.is_none()
            && !self.selecting && !self.selecting_text && self.text_selection.is_none()
            && !self.app_animating()
            && self.inspector.is_none();
        let layout = self.layout.borrow();
        if !eligible || !(turned || layout.has_stacking() || layout.has_overlays()) {
            self.raw_hit_cell = None;
            return raw_pointer_path(self.runtime.root(), &layout, x, y, turned);
        }
        let key = RawHitKey {
            root: std::ptr::from_ref::<Element>(self.runtime.root()).addr(),
            tree_changes: self.tree_changes,
            geometry: crate::geometry::generation(),
            size: self.size,
            scale: self.scale.to_bits(),
            turned: turned || layout.has_stacking() || layout.has_overlays(),
        };
        if let Some(cell) = &self.raw_hit_cell
            && cell.key == key && cell.region.contains(x, y)
        {
            #[cfg(test)]
            { self.hit_cell_counts[0] += 1; }
            return cell.path.clone();
        }
        #[cfg(test)]
        { self.hit_cell_counts[1] += 1; }
        let undo = |element: &Element, rect: Rect| {
            rsui_render::transform_of(element, rect.x, rect.y, rect.width, rect.height, 1.0)
                .and_then(rsui_render::invert_matrix)
        };
        let (path, region) = {
            #[cfg(feature = "idle-probe")]
            let _native_cycles = crate::native_cycles::enter(crate::native_cycles::RAW_POINTER_PATH);
            layout.hit_test_turned_certified(self.runtime.root(), x, y, &undo,
                Rect::new(0.0, 0.0, self.size.0 as f32 / self.scale, self.size.1 as f32 / self.scale))
        };
        self.raw_hit_cell = region.map(|region| RawHitCell { key, region, path: path.clone() });
        path
    }

    /// Whether anything's drawn here, in physical pixels, as far as the last frame says, or something here takes a click,
    /// as a button with no background of its own does. A transparent window lets the pointer through where this says no.
    /// What wraps the whole window doesn't count for taking clicks, as that would leave nowhere to go through.
    pub fn drawn_at(&mut self, x: f32, y: f32) -> bool {
        if self.painter.last_scene().draws_at(x, y) {
            return true;
        }
        self.nearest_at(x, y, |node| {
            node.actions.contains(ActionKind::Click)
                || node.actions.contains(ActionKind::Toggle)
                || node.actions.contains(ActionKind::SetValue)
        })
        .is_some_and(|path| !path.is_empty())
    }

    /// Whether what's here takes keys, as a text box does, for a window that doesn't take the keyboard when it's clicked
    /// otherwise.
    pub fn takes_keys_at(&mut self, x: f32, y: f32) -> bool {
        self.nearest_at(x, y, |node| node.role == Role::TextInput || node.actions.contains(ActionKind::SetValue))
            .is_some()
    }

    /// Whether taking hold of the window here should move it, for a window with no frame. Anything inside that takes
    /// input still gets it, rather than moving the window.
    pub fn drags_window_at(&mut self, x: f32, y: f32) -> bool {
        let Some(path) = self.nearest_at(x, y, |node| {
            !node.actions.is_empty() || node.props.get("drags_window").is_some()
        }) else {
            return false;
        };
        let node = node_at(self.runtime.root(), &path);
        node.actions.is_empty()
            && matches!(node.props.get("drags_window"), Some(PropValue::Bool(true)))
    }

    /// Finds the nearest node at this position, or around it, that offers this action.
    fn target_for(&mut self, x: f32, y: f32, action: ActionKind) -> Option<Vec<u32>> {
        self.nearest_at(x, y, |node| node.actions.contains(action))
    }

    /// Finds the nearest node at this position, or around it, that passes `test`. What's on top of
    /// everything is a world of its own, so this doesn't look past one to what it happens to be in.
    fn nearest_at(&mut self, x: f32, y: f32, test: impl Fn(&Element) -> bool) -> Option<Vec<u32>> {
        let mut path = self.path_at(x, y)?;
        loop {
            if test(node_at(self.runtime.root(), &path)) {
                return Some(path);
            }
            if self.layout.borrow().has_overlays() && self.layout.borrow().is_overlay(&path) {
                return None;
            }
            path.pop()?;
        }
    }

    /// Moves the keyboard into what's just come up on top, or back out of what's just gone.
    fn follow_overlays(&mut self) {
        let root = self.runtime.root();
        let top = self
            .layout
            .borrow()
            .overlays_in(root)
            .into_iter()
            .rev()
            .find(|path| takes_keyboard(node_at(root, path)));
        if top == self.trap {
            return;
        }
        let was_clear = self.trap.is_none();
        self.trap.clone_from(&top);
        let ring = self.script_ring(None);
        match top {
            Some(path) => {
                if was_clear {
                    self.focus_before_trap = self.focused().map(<[u32]>::to_vec);
                }
                let mut stops = Vec::new();
                let mut walking = path.clone();
                collect_focusable(node_at(root, &path), &mut walking, &mut stops);
                self.set_focus(stops.into_iter().next(), ring);
            }
            None => {
                let back = self.focus_before_trap.take().filter(|path| {
                    find(self.runtime.root(), path).is_some_and(|node| focusable(node))
                });
                self.set_focus(back, ring);
            }
        }
    }

    /// The topmost thing on top of everything that wants to hear about being finished with.
    fn top_dismissable(&mut self) -> Option<Vec<u32>> {
        self.lay_out();
        let root = self.runtime.root();
        self.layout
            .borrow()
            .overlays_in(root)
            .into_iter()
            .rev()
            .find(|path| node_at(root, path).actions.contains(ActionKind::Dismiss))
    }

    /// A press has happened here, which is the end of any popups it's not in. A press on what a popup
    /// came from is left to that, or pressing a menu's button would close it, and then open it.
    fn dismiss_popups_away_from(&mut self, x: f32, y: f32) {
        if !self.layout.borrow().has_overlays() {
            return;
        }
        let hit = self.path_at(x, y).unwrap_or_default();
        let stacked = self.layout.borrow().overlays_in(self.runtime.root());
        for path in stacked.into_iter().rev() {
            let root = self.runtime.root();
            let node = node_at(root, &path);
            if !node.actions.contains(ActionKind::Dismiss) {
                continue;
            }
            let inside = hit.starts_with(&path);
            let on_backdrop = inside && hit.len() == path.len() && node.role == Role::Dialog;
            if inside && !on_backdrop {
                return;
            }
            let (_, parent) = path.split_last().expect("the root isn't on top of itself");
            if !inside && !parent.is_empty() && hit.starts_with(parent) {
                return;
            }
            // It's only being told, and what it does about that is up to it.
            let _ = self.runtime.dispatch(&path, ActionKind::Dismiss, None);
            if on_backdrop {
                return;
            }
        }
    }

    /// When something's next due: a tooltip the pointer's been still long enough for, or the next frame
    /// of an animated image that's showing.
    pub fn next_wake(&mut self) -> Option<Duration> {
        let tooltip = if self.tooltip_showing || self.tooltip_dismissed {
            None
        } else {
            self.tooltip
                .as_ref()
                .map(|(_, since)| TOOLTIP_DELAY.saturating_sub(since.elapsed()))
        };
        let frame = self.painter.images().next_frame_in();
        // What moves by itself slowly enough is drawn when it next looks different, rather than at every frame.
        let moving = self.runtime.next_motion_in_view(|path, node| self.motion_might_show(path, node));
        [tooltip, frame, moving, self.move_wait()]
            .into_iter()
            .flatten()
            .min()
    }

    /// Does what [`next_wake`](Self::next_wake) said was coming.
    pub fn wake(&mut self) {
        self.wake_at(Instant::now());
    }

    /// Like [`wake`](Self::wake), as if it were this time, which is for tests.
    pub fn wake_at(&mut self, now: Instant) {
        self.send_due_move();
        if self.painter.images().play_to(now) {
            self.invalidate_automatic();
            self.repaint = true;
        }
        if self.runtime.is_animating() {
            self.runtime.tick(now);
        }
        if self.tooltip_showing || self.tooltip_dismissed {
            return;
        }
        if let Some((_, since)) = &self.tooltip {
            if now.duration_since(*since) >= TOOLTIP_DELAY {
                self.tooltip_showing = true;
                self.tooltip_at = self.pointer.unwrap_or((0.0, 0.0));
                self.invalidate_automatic();
                self.repaint = true;
            }
        }
    }

    fn dismiss_tooltip(&mut self) {
        self.tooltip_dismissed = true;
        if self.tooltip_showing {
            self.tooltip_showing = false;
            self.invalidate_automatic();
            self.repaint = true;
        }
    }

    /// A whole click at this position, in physical pixels. This doesn't move the keyboard or
    /// show anything being pressed, so it's for agents and tests. A window uses
    /// [`pointer_down`](Self::pointer_down) and [`pointer_up`](Self::pointer_up).
    pub fn click(&mut self, x: f32, y: f32) -> Result<(), DispatchError> {
        match self.nearest_at(x, y, activates) {
            Some(path) => self.activate_at(&path, Some((x, y)), 1),
            None => {
                // Nothing here takes clicks, but the app as a whole can still want to know, as "click outside to close" does.
                let target = self.path_at(x, y).unwrap_or_default();
                self.click_on_nothing(x, y, &target, 1);
                Ok(())
            }
        }
    }

    /// Tells the app as a whole about a click on something that doesn't take them, if it wants to hear about clicks.
    fn click_on_nothing(&mut self, x: f32, y: f32, target: &[u32], clicks: u32) {
        if document_listens() & listens::CLICK == 0 {
            return;
        }
        let mut event = self.event_at(x, y, target);
        event.detail = clicks;
        if let Ok(fired) = self.fire(Kind::Click, target, None, &mut event) {
            if !fired.stopped {
                self.tell_document(listens::CLICK, "click", 3, &mut event);
            }
        }
    }

    /// Does what clicking this node does, for a press that landed at this position, or with the keyboard or an agent if
    /// there isn't one.
    ///
    /// A click is told how many boxes inside the node the press landed in, if it wasn't on the node itself, and
    /// [`Updates::event`](rsui_runtime::Updates::event) says what that was, which is how code written for the web knows
    /// what was clicked. A handler that calls [`Updates::pass_on`](rsui_runtime::Updates::pass_on) passes the click to the
    /// nearest node around it that takes clicks, which is then told how far inside it the press was. Otherwise a click
    /// stops where it lands, for what's around it, though the app as a whole hears about it unless a handler called
    /// [`Updates::stop_propagation`](rsui_runtime::Updates::stop_propagation). What offers to hear clicks on the way down,
    /// as `click_capture`, hears it first.
    fn activate_at(
        &mut self,
        path: &[u32],
        at: Option<(f32, f32)>,
        clicks: u32,
    ) -> Result<(), DispatchError> {
        if !node_at(self.runtime.root(), path)
            .actions
            .contains(ActionKind::Click)
        {
            return self.activate(path);
        }
        let deepest = match at {
            Some((x, y)) => self.deepest_in(path, x, y),
            None => path.to_vec(),
        };
        let mut event = self.click_event(at, &deepest, clicks);
        if self.hears(Kind::Click, &deepest)
            && self.fire(Kind::Click, &deepest, None, &mut event)?.stopped
        {
            return Ok(());
        }
        self.runtime.take_stopped();
        let mut at = path.to_vec();
        loop {
            let within = boxes_between(self.runtime.root(), &at, &deepest);
            let input = (within > 0).then_some(PropValue::Int(within));
            event.phase = if at.len() == deepest.len() { 2 } else { 3 };
            self.runtime
                .dispatch_with(&at, ActionKind::Click, input, &mut event)?;
            let passed_on = self.runtime.take_passed_on();
            if self.runtime.take_stopped() {
                return Ok(());
            }
            if !passed_on {
                break;
            }
            // The tree hasn't changed yet, as that waits for the next update.
            let mut found = false;
            while at.pop().is_some() {
                if node_at(self.runtime.root(), &at)
                    .actions
                    .contains(ActionKind::Click)
                {
                    found = true;
                    break;
                }
            }
            if !found {
                break;
            }
        }
        self.tell_document(listens::CLICK, "click", 3, &mut event);
        Ok(())
    }

    /// Does what clicking this node does, with the keyboard or an agent, which is the same as a press in the middle of it,
    /// with nothing to say how many clicks.
    fn activate(&mut self, path: &[u32]) -> Result<(), DispatchError> {
        let node = node_at(self.runtime.root(), path);
        if node.actions.contains(ActionKind::Click) {
            return self.activate_at(path, None, 0);
        }
        self.runtime.dispatch(path, ActionKind::Toggle, None)
    }

    /// The pointer moved to this position, in physical pixels.
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        if (self.drag_from.is_some() || self.html_drag.is_some()) && self.drag_moved(x, y) {
            // What's being dragged is all that hears the pointer now.
            self.pointer = Some((x, y));
            return;
        }
        if let Some(dragged) = self.dragged.as_mut() {
            let (dx, dy) = (x - dragged.last.0, y - dragged.last.1);
            dragged.last = (x, y);
            let path = dragged.path.clone();
            if dx != 0.0 || dy != 0.0 {
                self.update();
                self.lay_out();
                let _ = self.send_drag(&path, "move", x, y, dx, dy);
            }
        }
        self.pointer = Some((x, y));
        if self.dragging.is_some() {
            self.drag_scrollbar(x, y);
            return;
        }
        if self.selecting {
            if let Some(at) = self.input_byte_at(x, y) {
                if let Some(edit) = self.focus.as_mut().and_then(|focus| focus.edit.as_mut()) {
                    if edit.caret() != at {
                        edit.extend_to(at);
                        self.invalidate_automatic();
                        self.repaint = true;
                    }
                }
            }
        }
        if self.selecting_text {
            self.extend_text_selection(x, y);
        }
        if let Some(inspector) = self
            .inspector
            .as_mut()
            .filter(|inspector| inspector.showing)
        {
            inspector.redraw = true;
        }
        self.update_hover();
        {
            #[cfg(feature = "idle-probe")]
            let _native_cycles = crate::native_cycles::enter(crate::native_cycles::POINTER_MOVED);
            self.pointer_moved();
        }
    }

    /// The pointer left the window.
    pub fn pointer_leave(&mut self) {
        // What's waiting to hear where it went last hears that first.
        self.send_move();
        self.pointer = None;
        self.update_hover();
    }

    /// What the pointer should look like.
    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    fn update_hover(&mut self) {
        #[cfg(feature = "idle-probe")]
        let _native_cycles = crate::native_cycles::enter(crate::native_cycles::HOVER);
        #[cfg(test)]
        {
            self.interaction_counts[0] += 1;
        }
        let path = self.pointer.and_then(|(x, y)| self.path_at_inner(x, y, true));
        let mut hovered = None;
        let mut explained = None;
        let mut cursor = Cursor::Arrow;
        let mut moves_heard = false;
        if let Some(path) = &path {
            let mut node = self.runtime.root();
            let mut visit = |node: &Node, depth: usize| {
                if has_hover(node) && !is_disabled(node) {
                    hovered = Some(depth);
                }
                moves_heard |= Self::hears_moves(node);
                if tooltip_of(node).is_some() {
                    explained = Some(depth);
                }
                if let Some(PropValue::Str(said)) = node.props.get_symbol(cursor_name()) {
                    cursor = match said.as_str() {
                        "pointer" => Cursor::Hand,
                        "text" => Cursor::Text,
                        _ => Cursor::Arrow,
                    };
                } else if node.role == Role::TextInput && !is_disabled(node) {
                    cursor = Cursor::Text;
                } else if activates(node) {
                    cursor = Cursor::Hand;
                } else if node.role == Role::Text
                    && !node.children.is_empty()
                    && is_selectable(node)
                {
                    cursor = Cursor::Text;
                }
            };
            visit(node, 0);
            for (depth, &index) in path.iter().enumerate() {
                node = &node.children[index as usize];
                visit(node, depth + 1);
            }
        }
        self.cursor = cursor;
        self.moves_heard = moves_heard;
        self.tell_pointer(path.as_deref().unwrap_or(&[]), hovering_name(), 0);
        let explained = explained.and_then(|depth| Some(&path.as_ref()?[..depth]));
        if explained != self.tooltip.as_ref().map(|(path, _)| path.as_slice()) {
            if self.tooltip_showing {
                self.invalidate_automatic();
                self.repaint = true;
            }
            self.tooltip_showing = false;
            self.tooltip_dismissed = false;
            self.tooltip = explained.map(|path| (path.to_vec(), Instant::now()));
        }
        let hovered = hovered.and_then(|depth| Some(path.as_ref()?[..depth].to_vec()));
        if hovered != self.hovered {
            if self.animations {
                let now = Instant::now();
                if let Some(old) = self.hovered.take() {
                    self.leaving = Some((old, now, self.hover_amount));
                    self.leaving_amount = self.hover_amount;
                }
                if hovered.is_some() {
                    self.hover_since = Some(now);
                    self.hover_amount = 0.0;
                } else {
                    self.hover_since = None;
                    self.hover_amount = 1.0;
                }
            }
            #[cfg(feature = "automatic-render")]
            if let Some(p) = &mut self.pilot {
                if let Some(path) = hovered.as_deref().or(self.hovered.as_deref()) {
                    p.select(path);
                }
            }
            self.hovered = hovered;
            self.repaint = true;
        }
    }

    /// Tells what's at `path`, and everything it's in, that wants to know, that it's now there, and what was before that it isn't. Which
    /// of these it is goes by `which`, and what was there before is kept in the slot it says.
    fn tell_pointer(&mut self, path: &[u32], name: Symbol, which: usize) {
        let was = std::mem::take(&mut self.pointer_paths[which]);
        let was_keys = std::mem::take(&mut self.pointer_keys[which]);
        // Where what it was on is now, as far as that's still there: something put in or taken out before it moves it along
        // without it having been left, as a page has it, where it's the same element. What's gone isn't told it's
        // been left, and what's now where it was isn't told either.
        let (before, whole) = follow_along(self.runtime.root(), &was, &was_keys);
        if whole && before.as_slice() == path {
            self.pointer_paths[which] = before;
            self.pointer_keys[which] = was_keys;
            return;
        }
        let shared = before.iter().zip(path).take_while(|(a, b)| a == b).count();
        let action = ActionKind::Custom(name);
        // Code written for the web hears the pointer come and go with where it is, as `pointer_hover`, which is only made
        // into an event for what offers to hear that.
        let hover = ActionKind::Custom(symbol!("pointer_hover"));
        let heard = |app: &Self, at: &[u32], action: ActionKind| {
            find(app.runtime.root(), at).is_some_and(|node| node.actions.contains(action))
        };
        let pointer = self.pointer.filter(|_| which == 0);
        let mut event = None;
        for depth in (shared + 1..=before.len()).rev() {
            let at = &before[..depth];
            if heard(self, at, action) {
                let _ = self
                    .runtime
                    .dispatch(at, action, Some(PropValue::Bool(false)));
            }
            if let Some((x, y)) = pointer.filter(|_| heard(self, at, hover)) {
                let event = event.get_or_insert_with(|| self.event_at(x, y, &before));
                let _ = self
                    .runtime
                    .dispatch_with(at, hover, Some(PropValue::Bool(false)), event);
            }
        }
        let mut event = None;
        for depth in shared + 1..=path.len() {
            let at = &path[..depth];
            if heard(self, at, action) {
                let _ = self
                    .runtime
                    .dispatch(at, action, Some(PropValue::Bool(true)));
            }
            if let Some((x, y)) = pointer.filter(|_| heard(self, at, hover)) {
                let event = event.get_or_insert_with(|| self.event_at(x, y, path));
                let _ = self
                    .runtime
                    .dispatch_with(at, hover, Some(PropValue::Bool(true)), event);
            }
        }
        self.pointer_paths[which] = path.to_vec();
        self.pointer_keys[which] = keys_along(self.runtime.root(), path);
    }

    /// The main button went down at this position, in physical pixels. `clicks` is 2 if this
    /// is the second press of a double click.
    pub fn pointer_down(&mut self, x: f32, y: f32, clicks: u32) {
        self.pointer_down_here(x, y, clicks);
        // A word picked by two clicks is heard as `select` before the button comes up.
        let _ = self.publish_selection();
    }

    fn pointer_down_here(&mut self, x: f32, y: f32, clicks: u32) {
        self.pointer_capture = None;
        self.send_move();
        let moved = self.pointer != Some((x, y));
        self.pointer = Some((x, y));
        self.keyboard_last = false;
        self.dismiss_tooltip();
        self.update();
        self.lay_out();
        // Pressed somewhere the pointer wasn't said to have got to: what it's over hears it come first, as on a page.
        if moved {
            self.update_hover();
        }
        self.dismiss_popups_away_from(x, y);
        self.buttons |= PointerButton::Main.web_bit();
        if let Some(drag) = self.scrollbar_at(x, y) {
            self.dragging = Some(drag);
            self.drag_scrollbar(x, y);
            return;
        }
        let under = self.path_at(x, y);
        if let Some(under) = &under {
            self.tell_pointer(under, pressing_name(), 1);
        }
        // What code written for the web hears first, which can keep the keyboard where it is, and what it's on hears the
        // pointer move until the button comes up.
        let under = under.unwrap_or_default();
        self.down_keys = keys_along(self.runtime.root(), &under);
        self.down_at = Some(under.clone());
        self.down_clicks = clicks.max(1);
        let kept = self.tell_button(Kind::Down, x, y, &under, PointerButton::Main, clicks);
        self.keyboard_back_to_page();
        self.capture(&under);
        // Something that can be dragged is, once it's moved far enough, unless that's been kept from happening.
        if kept {
            self.drag_from = None;
        } else {
            self.note_draggable(&under, x, y);
        }
        if kept {
            self.pressed = self.nearest_at(x, y, activates).map(|path| Press {
                key: node_at(self.runtime.root(), &path).key.clone(),
                keys: keys_along(self.runtime.root(), &path),
                path,
                clicks,
            });
            return;
        }
        self.pressed = self.nearest_at(x, y, activates).map(|path| {
            let node = node_at(self.runtime.root(), &path);
            if has_pressed(node) {
                #[cfg(feature = "automatic-render")]
                if let Some(p) = &mut self.pilot {
                    p.select(&path);
                    p.full = true;
                }
                self.repaint = true;
            }
            Press {
                key: node.key.clone(),
                keys: keys_along(self.runtime.root(), &path),
                path,
                clicks,
            }
        });
        let dragged = self.nearest_at(x, y, |node| {
            node.actions.contains(ActionKind::Custom(symbol!("drag")))
        });
        if let Some(path) = dragged {
            self.dragged = Some(Dragged {
                path: path.clone(),
                last: (x, y),
            });
            let _ = self.send_drag(&path, "start", x, y, 0.0, 0.0);
        }

        let target = self.nearest_at(x, y, focusable);
        let was_focused = target.is_some() && self.focused() == target.as_deref();
        if self.page_has_focus_order() {
            // A page's, as Chromium has it: what the pointer gives the keyboard to doesn't show a ring (a text box does, by its
            // own CSS), what already had it keeps what it showed, and a key going down before the next click shows it again.
            if !was_focused {
                if target.is_some() || self.focus.is_some() {
                    self.had_keyboard_event = false;
                }
                if target.is_some() {
                    self.last_focus_by_pointer = true;
                }
                self.set_focus(target, false);
            }
        } else {
            self.set_focus(target, false);
        }
        if self
            .focus
            .as_ref()
            .is_some_and(|focus| focus.edit.is_some())
        {
            let at = self.input_byte_at(x, y).unwrap_or(0);
            if let Some(focus) = &mut self.focus {
                focus.goal_x = None;
                focus.upstream = None;
            }
            let node = self.focused_node();
            if let Some(edit) = self.focus.as_mut().and_then(|focus| focus.edit.as_mut()) {
                if clicks >= 3 {
                    // Three clicks pick the whole line, which in a field of one line is all of it, as in a browser.
                    edit.select_all(input_value(&node));
                } else if clicks == 2 {
                    edit.select_word_at(input_value(&node), at);
                } else if self.modifiers.shift && was_focused {
                    // Shift and a click go from where it was held to there.
                    edit.extend_to(at);
                    self.selecting = true;
                } else {
                    edit.place(at);
                    self.selecting = true;
                }
            }
            self.wake_caret();
        }
        self.press_on_text(x, y, clicks);
    }

    /// Where a press goes if there's nothing it does: anywhere else clears what's selected, and on text
    /// that can be selected it starts selecting. Two clicks choose a word and three choose it all.
    fn press_on_text(&mut self, x: f32, y: f32, clicks: u32) {
        if self.text_selection.take().is_some() {
            self.invalidate_automatic();
            self.repaint = true;
        }
        if !self.layout.borrow().has_paragraphs()
            || self.pressed.is_some()
            || self.dragged.is_some()
        {
            return;
        }
        let Some(path) = self.nearest_at(x, y, is_selectable) else {
            return;
        };
        let Some(at) = self.paragraph_byte_at(&path, x, y) else {
            return;
        };
        let text = paragraph_text(node_at(self.runtime.root(), &path));
        let mut edit = TextEdit::new();
        match clicks {
            0 | 1 => {
                edit.place(at);
                self.selecting_text = true;
            }
            2 => edit.select_word_at(&text, at),
            _ => edit.select_all(&text),
        }
        self.text_selection = Some(TextSelection { path, text, edit });
        self.text_selection_stale = false;
        self.invalidate_automatic();
        self.repaint = true;
    }

    /// The nearest byte of the paragraph at `path` to this position.
    fn paragraph_byte_at(&mut self, path: &[u32], x: f32, y: f32) -> Option<usize> {
        let node = node_at(self.runtime.root(), path);
        let rect = self.layout.borrow().rect(path)?;
        let content = content_box(node, rect, self.scale);
        Some(self.text.paragraph_byte_at(
            node,
            self.scale,
            content.width,
            (x - content.left, y - content.top),
        ))
    }

    fn extend_text_selection(&mut self, x: f32, y: f32) {
        // What's there may have changed since it was drawn, and places in it mean nothing in the new one.
        self.check_text_selection();
        let Some(path) = self.text_selection.as_ref().map(|one| one.path.clone()) else {
            self.selecting_text = false;
            return;
        };
        if find(self.runtime.root(), &path).is_none() {
            self.selecting_text = false;
            self.text_selection = None;
            return;
        }
        let Some(at) = self.paragraph_byte_at(&path, x, y) else {
            return;
        };
        if let Some(selection) = &mut self.text_selection {
            if selection.edit.caret() != at {
                selection.edit.extend_to(at);
                self.invalidate_automatic();
                self.repaint = true;
            }
        }
    }

    /// What's selected outside of any input, if anything is.
    pub fn selected_text(&self) -> Option<&str> {
        let selection = self.text_selection.as_ref()?;
        let (start, end) = selection.edit.selection();
        (start < end).then(|| &selection.text[start..end])
    }

    /// The keys that do something to selected text.
    fn text_selection_key(&mut self, key: KeyPress) -> bool {
        self.check_text_selection();
        let Some(selection) = &mut self.text_selection else {
            return false;
        };
        match key.code {
            KeyCode::Character('c' | 'C') if key.ctrl && !key.alt => {
                let (start, end) = selection.edit.selection();
                if start < end {
                    let copied = selection.text[start..end].to_owned();
                    self.clipboard.write(&copied);
                }
                true
            }
            KeyCode::Character('a' | 'A') if key.ctrl && !key.alt => {
                let text = std::mem::take(&mut selection.text);
                selection.edit.select_all(&text);
                selection.text = text;
                self.invalidate_automatic();
                self.repaint = true;
                true
            }
            KeyCode::Escape => {
                self.text_selection = None;
                self.invalidate_automatic();
                self.repaint = true;
                true
            }
            _ => false,
        }
    }

    /// Forgets what's selected if what it was selected in isn't what's there any more.
    fn check_text_selection(&mut self) {
        #[cfg(test)]
        if self.conservative_interactions {
            self.text_selection_stale = true;
        }
        if !std::mem::take(&mut self.text_selection_stale) {
            return;
        }
        let Some(selection) = &self.text_selection else {
            return;
        };
        #[cfg(test)]
        {
            self.interaction_counts[2] += 1;
        }
        let same = find(self.runtime.root(), &selection.path)
            .is_some_and(|node| is_selectable(node) && paragraph_text(node) == selection.text);
        if !same {
            self.text_selection = None;
            self.selecting_text = false;
        }
    }

    /// The main button came up at this position, which is a click if it's still on the node it
    /// went down on.
    pub fn pointer_up(&mut self, x: f32, y: f32) -> Result<(), DispatchError> {
        // What dragging across a field selected is heard as `select` before the button comes up, as in a browser.
        self.publish_selection()?;
        self.send_move();
        let moved = self.pointer != Some((x, y));
        self.pointer = Some((x, y));
        if moved && self.html_drag.is_none() && self.dragging.is_none() {
            self.update_hover();
        }
        self.drag_from = None;
        if self.html_drag.is_some() {
            // It's dropped, and that's all: it isn't a click.
            self.buttons &= !PointerButton::Main.web_bit();
            self.drag_released(x, y);
            self.tell_pointer(&[], pressing_name(), 1);
            self.captured = None;
            self.pointer_capture = None;
            self.pressed = None;
            self.down_at = None;
            return Ok(());
        }
        self.tell_pointer(&[], pressing_name(), 1);
        self.selecting = false;
        self.selecting_text = false;
        self.dragging = None;
        // What had the moves hears the button come up, wherever that is, and otherwise what it's over. What's asked for the
        // pointer (`setPointerCapture()`) is where it came up, as in a browser, so the click's there too.
        let explicit = self.pointer_capture.take().and_then(|key| path_of_page_key(self.runtime.root(), &key));
        let capture_pressed = std::mem::take(&mut self.capture_pressed) && explicit.is_some();
        let over = self.path_at(x, y).unwrap_or_default();
        let over = explicit.clone().unwrap_or(over);
        let target = explicit.clone().or_else(|| self.captured_path()).unwrap_or_else(|| over.clone());
        self.buttons &= !PointerButton::Main.web_bit();
        let clicks = self.down_clicks;
        self.tell_button(Kind::Up, x, y, &target, PointerButton::Main, clicks);
        self.captured = None;
        // Where it went down, and what was pressed, are where they are now: what the press set off may have put things in or taken
        // them out before them since, which doesn't make the click anything else, as on a page, where it's the same element.
        let down_at = self
            .down_at
            .take()
            .map(|path| follow_along(self.runtime.root(), &path, &self.down_keys).0);
        let pressed = self.pressed.take().map(|mut press| {
            if let (now, true) = follow_along(self.runtime.root(), &press.path, &press.keys) {
                press.path = now;
            }
            press
        });
        if let Some(dragged) = self.dragged.take() {
            let (dx, dy) = (x - dragged.last.0, y - dragged.last.1);
            let _ = self.send_drag(&dragged.path, "end", x, y, dx, dy);
        }
        // Where it went down and where it came up are both in what's around them both, which is where the click is, and the
        // double click after it, as on a page.
        let down_at = if capture_pressed { Some(over.clone()) } else { down_at };
        let shared = down_at.map(|down_at| {
            let shared = down_at
                .iter()
                .zip(&over)
                .take_while(|(a, b)| a == b)
                .count();
            over[..shared].to_vec()
        });
        let Some(press) = pressed else {
            if let Some(shared) = shared {
                let clicks = self.down_clicks;
                self.click_on_nothing(x, y, &shared, clicks);
                self.double_click(x, y, &shared, clicks)?;
            }
            return Ok(());
        };
        if let Some(node) = find(self.runtime.root(), &press.path) {
            if has_pressed(node) {
                #[cfg(feature = "automatic-render")]
                if let Some(p) = &mut self.pilot {
                    p.select(&press.path);
                    p.full = true;
                }
                self.repaint = true;
            }
        }
        let target = match &explicit {
            Some(path) => {
                let root = self.runtime.root();
                (0..=path.len()).rev().map(|length| &path[..length]).find(|along| activates(node_at(root, along))).map(<[u32]>::to_vec)
            }
            None => self.nearest_at(x, y, activates),
        };
        let same = target.as_ref().is_some_and(|path| {
            *path == press.path && node_at(self.runtime.root(), path).key == press.key
        });
        if same {
            self.activate_at(&press.path, Some((x, y)), press.clicks)?;
        } else if let Some(shared) = &shared {
            self.click_on_nothing(x, y, shared, press.clicks);
        }
        if same || shared.is_some() {
            let target = shared.unwrap_or_else(|| press.path.clone());
            self.double_click(x, y, &target, press.clicks)?;
        }
        Ok(())
    }

    /// The second click of a double click is still a click, and `dblclick` comes after it, through what it's in on the way down
    /// and back up, as it does on the web.
    fn double_click(&mut self, x: f32, y: f32, target: &[u32], clicks: u32) -> Result<(), DispatchError> {
        // Only the second press is a double click: a third is a click with a `detail` of 3, as on the web.
        if clicks != 2 || !self.hears(Kind::DoubleClick, target) {
            return Ok(());
        }
        let mut event = self.event_at(x, y, target);
        event.detail = clicks;
        self.fire(Kind::DoubleClick, target, None, &mut event)?;
        Ok(())
    }

    /// A button other than the main one went down here, for code written for the web, which hears which. The main one is
    /// [`pointer_down`](Self::pointer_down).
    pub fn button_down(&mut self, x: f32, y: f32, button: PointerButton) {
        if button == PointerButton::Main {
            self.pointer_down(x, y, 1);
            return;
        }
        self.send_move();
        self.pointer = Some((x, y));
        self.update();
        self.buttons |= button.web_bit();
        let under = self.path_at(x, y).unwrap_or_default();
        let kept = self.tell_button(Kind::Down, x, y, &under, button, 1);
        self.other_down_at = Some((button, under));
        if button == PointerButton::Secondary && !kept {
            self.secondary_press(x, y);
        }
    }

    /// What the other button going down does by itself, as Chromium has it: what it's on that can have the keyboard gets it, and
    /// the caret goes where it was pressed, in a field or in a page's text, unless that's in what's selected, which the menu that
    /// comes up is then about.
    fn secondary_press(&mut self, x: f32, y: f32) {
        self.lay_out();
        // Where nothing there takes the keyboard, nothing has it after, as with the main button: an open Radix context menu loses
        // it to a right press outside, and the menu that press brings up isn't given it again while the first is still going away.
        let target = self.nearest_at(x, y, focusable);
        let on_target = target.is_some();
        self.set_focus(target, false);
        if on_target && self.focus.as_ref().is_some_and(|focus| focus.edit.is_some()) {
            let at = self.input_byte_at(x, y).unwrap_or(0);
            if let Some(edit) = self.focus.as_mut().and_then(|focus| focus.edit.as_mut()) {
                let (start, end) = edit.selection();
                if !(start < end && start <= at && at <= end) {
                    edit.place(at);
                }
            }
            self.wake_caret();
            return;
        }
        if !self.layout.borrow().has_paragraphs() {
            return;
        }
        let Some(path) = self.nearest_at(x, y, is_selectable) else {
            return;
        };
        let Some(at) = self.paragraph_byte_at(&path, x, y) else {
            return;
        };
        if let Some(selection) = &self.text_selection {
            let (start, end) = selection.edit.selection();
            if selection.path == path && start < end && start <= at && at <= end {
                return;
            }
        }
        let text = paragraph_text(node_at(self.runtime.root(), &path));
        let mut edit = TextEdit::new();
        edit.place(at);
        self.text_selection = Some(TextSelection { path, text, edit });
        self.text_selection_stale = false;
        self.invalidate_automatic();
        self.repaint = true;
    }

    /// A button other than the main one came up here. The other button then brings up a menu, as
    /// [`secondary_click`](Self::secondary_click) does, and the middle one, or back or forward, clicks as the web's
    /// `auxclick` has it, if it came up on what it went down on.
    pub fn button_up(
        &mut self,
        x: f32,
        y: f32,
        button: PointerButton,
    ) -> Result<(), DispatchError> {
        if button == PointerButton::Main {
            return self.pointer_up(x, y);
        }
        self.send_move();
        self.pointer = Some((x, y));
        self.update();
        self.buttons &= !button.web_bit();
        let under = self
            .pointer_capture
            .as_ref()
            .and_then(|key| path_of_page_key(self.runtime.root(), key))
            .unwrap_or_else(|| self.path_at(x, y).unwrap_or_default());
        self.tell_button(Kind::Up, x, y, &under, button, 1);
        if self.buttons == 0 {
            self.pointer_capture = None;
            self.captured = None;
            self.capture_pressed = false;
        }
        let down = self.other_down_at.take();
        // `auxclick` first, on what it went down and came up on, and then, for the other button, `contextmenu`, as a browser on
        // Windows has them.
        if let Some((pressed, down_at)) = down {
            if pressed == button {
                let shared = down_at
                    .iter()
                    .zip(&under)
                    .take_while(|(a, b)| a == b)
                    .count();
                let target = &under[..shared];
                if self.hears(Kind::AuxClick, target) {
                    let mut event = self.event_at(x, y, target);
                    event.button = button.web_button();
                    event.detail = 1;
                    self.fire(Kind::AuxClick, target, None, &mut event)?;
                }
            }
        }
        if button == PointerButton::Secondary {
            return self.secondary_click(x, y);
        }
        Ok(())
    }

    /// The other button was clicked here. The nearest thing that wants to know is told where, in logical pixels.
    pub fn secondary_click(&mut self, x: f32, y: f32) -> Result<(), DispatchError> {
        self.dismiss_tooltip();
        self.update();
        self.lay_out();
        self.dismiss_popups_away_from(x, y);
        let action = ActionKind::Custom(symbol!("context_menu"));
        let over = self.path_at(x, y).unwrap_or_default();
        let mut event = self.event_at(x, y, &over);
        event.button = PointerButton::Secondary.web_button();
        // A page's `contextmenu` says no clicks, as a browser's does.
        event.detail = 0;
        self.runtime.take_stopped();
        if self.tell_document(listens::CONTEXT_MENU, "contextmenu", 1, &mut event)
            || self.tell_document(listens::CONTEXT_MENU, "contextmenu", 2, &mut event)
        {
            return Ok(());
        }
        if let Some(path) = self.nearest_at(x, y, |node| node.actions.contains(action)) {
            let input = PropValue::from(format!("{} {}", x / self.scale, y / self.scale).as_str());
            self.runtime
                .dispatch_with(&path, action, Some(input), &mut event)?;
            if self.runtime.take_stopped() {
                return Ok(());
            }
        }
        self.tell_document(listens::CONTEXT_MENU, "contextmenu", 3, &mut event);
        Ok(())
    }

    /// Says something's happening to what's being dragged: where the pointer is from its top left, and how far it's
    /// moved since the last time, in logical pixels.
    fn send_drag(
        &mut self,
        path: &[u32],
        phase: &str,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
    ) -> Result<(), DispatchError> {
        let Some(rect) = self.layout.borrow().rect(path) else {
            return Ok(());
        };
        let scale = self.scale;
        let input = format!(
            "{phase} {} {} {} {} {} {}",
            x / scale - rect.x,
            y / scale - rect.y,
            dx / scale,
            dy / scale,
            rect.width,
            rect.height
        );
        self.runtime.dispatch(
            path,
            ActionKind::Custom(symbol!("drag")),
            Some(PropValue::from(input.as_str())),
        )
    }

    /// Forgets anything being pressed, such as when another window takes the pointer.
    pub fn cancel_press(&mut self) {
        self.tell_pointer(&[], pressing_name(), 1);
        self.selecting = false;
        self.dragging = None;
        self.captured = None;
        self.down_at = None;
        self.other_down_at = None;
        self.buttons = 0;
        self.drag_from = None;
        if self.dragging_from_here() {
            self.cancel_drag();
        }
        if let Some(dragged) = self.dragged.take() {
            let _ = self.send_drag(
                &dragged.path,
                "cancel",
                dragged.last.0,
                dragged.last.1,
                0.0,
                0.0,
            );
        }
        if self.pressed.take().is_some() {
            self.invalidate_automatic();
            self.repaint = true;
        }
    }

    /// Where the keyboard is, as a path to the focused node.
    pub fn focused(&self) -> Option<&[u32]> {
        self.focus.as_ref().map(|focus| focus.path.as_slice())
    }

    /// The window started or stopped receiving the keyboard. Focus stays where it is, but isn't
    /// shown until the window gets the keyboard back.
    pub fn set_active(&mut self, active: bool) {
        let changed = self.active != active;
        if changed {
            self.active = active;
            // Nothing else that's drawn depends on this, and drawing can mean getting the GPU back.
            if self
                .focus
                .as_ref()
                .is_some_and(|focus| focus.ring || focus.edit.is_some())
            {
                self.invalidate_automatic();
                self.repaint = true;
            }
            self.wake_caret();
        }
        if !active {
            self.cancel_press();
        }
        if changed {
            // A page hears it as `blur` and `focus` on its window, as in a browser when another window comes to the front and
            // this one comes back (see `windowblur` in web.js).
            let mut event = rsui_runtime::Event::default();
            self.runtime.deliver_document_event(if active { "windowfocus" } else { "windowblur" }, &mut event);
            self.runtime.take_stopped();
        }
    }

    /// The window was minimized, or brought back from that, which a page hears as `visibilitychange`, with `document.hidden`
    /// saying which.
    pub fn set_shown(&mut self, shown: bool) {
        if self.shown != shown {
            self.shown = shown;
            let mut event = rsui_runtime::Event::default();
            self.runtime.deliver_document_event(if shown { "windowshown" } else { "windowhidden" }, &mut event);
            self.runtime.take_stopped();
        }
    }

    /// Gives the pointer to the element of a page with this key, or lets it go, as `setPointerCapture()` and
    /// `releasePointerCapture()` do. A mouse can only be captured while a button's down (Pointer Events 3, 10.1).
    fn capture_pointer(&mut self, key: Option<Key>) {
        let Some(key) = key else {
            if self.pointer_capture.take().is_some() {
                self.captured = None;
            }
            return;
        };
        if self.buttons == 0 {
            return;
        }
        if let Some(path) = path_of_page_key(self.runtime.root(), &key) {
            let down_at = self
                .down_at
                .as_ref()
                .map(|down_at| follow_along(self.runtime.root(), down_at, &self.down_keys).0);
            // Pressed on it or on what's in it: the click's on what's around both, which is it.
            self.capture_pressed = down_at.is_some_and(|down_at| down_at.starts_with(&path));
            self.capture_explicitly(&path);
            self.pointer_capture = Some(key);
        }
    }

    /// The key that asking to focus takes the keyboard from whatever has it, with nothing given it.
    pub const NO_FOCUS: &'static str = "__rsui_no_focus";

    /// Moves the keyboard to what has this key, if anything that can have it does, or from what has it, for [`Self::NO_FOCUS`].
    fn focus_on_key(&mut self, key: &Key) {
        fn find_key(node: &Element, key: &Key, path: &mut Vec<u32>) -> bool {
            // Something can also be found by a `dom_key` prop, which is what a page's fields have, as their own key is
            // what's around them gave them.
            let named = || match (node.props.get("dom_key"), key) {
                (Some(PropValue::Str(text)), Key::Str(wanted)) => text.as_str() == wanted.as_str(),
                (Some(PropValue::Str(text)), Key::Sym(wanted)) => text.as_str() == wanted.as_str(),
                _ => false,
            };
            if (node.key.as_ref() == Some(key) || named()) && focusable(node) {
                return true;
            }
            for (index, child) in node.children.iter().enumerate() {
                path.push(index as u32);
                if find_key(child, key, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        let said = self.runtime.take_focus_visible_wanted();
        let prevent_scroll = self.runtime.take_focus_prevent_scroll_wanted();
        if matches!(key, Key::Str(text) if text.as_str() == Self::NO_FOCUS) {
            // Nothing has the keyboard, as a page's `blur()` leaves it.
            self.set_focus(None, false);
            return;
        }
        let mut path = Vec::new();
        if find_key(self.runtime.root(), key, &mut path) {
            self.lay_out();
            let ring = self.script_ring(said);
            let target = find(self.runtime.root(), &path)
                .filter(|node| focusable(node))
                .map(|_| path.clone());
            // On a page, giving the keyboard to what has it changes nothing, as in a browser (`Document::SetFocusedElement`).
            if target.is_some() && self.focused() == target.as_deref() && self.page_has_focus_order() {
                return;
            }
            self.set_focus(target, ring);
            if let Some(focus) = &mut self.focus {
                focus.ring_said = said.is_some();
            }
            if !prevent_scroll {
                self.reveal_moved_focus();
            }
        }
    }

    /// Whether what code gives the keyboard to shows a ring, as a browser's `:focus-visible` has it, or as `said` says if it does
    /// (a page's `focus({ focusVisible })`). On a page, as Chromium has it (`MatchesFocusVisiblePseudoClass`): unless the
    /// pointer gave the keyboard to something last and no key has gone down since. Text boxes always show one, which their CSS
    /// says (`typedIn` in web.js). Elsewhere, rsui's own: if the keyboard was used last, or what had it showed one.
    fn script_ring(&self, said: Option<bool>) -> bool {
        said.unwrap_or_else(|| {
            if self.page_has_focus_order() {
                !self.last_focus_by_pointer || self.had_keyboard_event
            } else {
                self.keyboard_last || self.focus.as_ref().is_some_and(|focus| focus.ring)
            }
        })
    }

    /// A key's gone down, without Ctrl, Alt or the Windows key, which on a page shows a ring on what the pointer gave the keyboard
    /// to, as Chromium's `Node::UpdateHadKeyboardEvent` has it (`:focus-visible` is worked out again then), unless code said not.
    fn saw_keyboard(&mut self) {
        if self.had_keyboard_event {
            return;
        }
        self.had_keyboard_event = true;
        if !self.page_has_focus_order() {
            return;
        }
        let shown = match &mut self.focus {
            Some(focus) if !focus.ring && !focus.ring_said => {
                focus.ring = true;
                Some(focus.path.clone())
            }
            _ => None,
        };
        if let Some(path) = shown {
            self.tell_focus_ring(&path, true);
            self.invalidate_automatic();
            self.repaint = true;
        }
    }

    /// Moves the keyboard to a node, telling both it and the node it left, if they care. Tabbing
    /// to an input selects what's in it. A node that can't have focus clears it.
    pub fn focus_on(&mut self, path: &[u32]) {
        let target = find(self.runtime.root(), path)
            .filter(|node| focusable(node))
            .map(|_| path.to_vec());
        self.set_focus(target, true);
        self.reveal_moved_focus();
    }

    fn set_focus(&mut self, target: Option<Vec<u32>>, ring: bool) {
        if self.focus.as_ref().map(|focus| focus.path.as_slice()) == target.as_deref() {
            let mut changed = None;
            if let Some(focus) = &mut self.focus {
                if focus.ring != ring {
                    changed = Some(focus.path.clone());
                }
                focus.ring = ring;
            }
            if let Some(path) = changed {
                self.tell_focus_ring(&path, ring);
            }
            self.invalidate_automatic();
            self.repaint = true;
            return;
        }
        // What had the keyboard is told it hasn't once what has it now has been told, so that a page hears it go from one to the
        // other at once, with `blur` saying where it went and `focus` where it came from, as Tab does on a page.
        let mut lost = None;
        if let Some(old) = self.focus.take() {
            // What it was scrolled to stays, rather than it jumping back when it's let go of.
            if old.scroll_y > 0.0 {
                self.resting_area = Some((old.path.clone(), old.scroll_y));
            }
            self.tell_focus_ring(&old.path, false);
            lost = Some(old.path);
        }
        self.invalidate_automatic();
        self.repaint = true;
        let Some(path) = target else {
            if let Some(old) = lost {
                self.tell_focus(&old, false);
            }
            return;
        };
        let scroll_y = match self.resting_area.take() {
            Some((area, scroll)) if area == path => scroll,
            other => {
                self.resting_area = other;
                0.0
            }
        };
        let root = self.runtime.root();
        let mut keys = Vec::with_capacity(path.len());
        let mut node = root;
        for &index in &path {
            node = &node.children[index as usize];
            keys.push(node.key.clone());
        }
        let edit = (node.role == Role::TextInput).then(|| {
            let mut edit = TextEdit::new();
            edit.select_all(input_value(node));
            edit
        });
        self.tell_focus_ring(&path, ring);
        self.tell_focus(&path, true);
        if let Some(old) = lost {
            self.tell_focus_lost(&old);
        }
        self.focus = Some(Focus {
            path,
            keys,
            ring,
            ring_said: false,
            edit,
            scroll: 0.0,
            scroll_y,
            goal_x: None,
            upstream: None,
            carets: None,
        });
        self.wake_caret();
    }

    /// Tells what has the keyboard whether it shows that it has, as it does when it's been tabbed to rather than clicked, which
    /// CSS's `:focus-visible` is about, if it offers to hear that (as `focus_ring`). It's told before it's told it has the keyboard.
    fn tell_focus_ring(&mut self, path: &[u32], ring: bool) {
        let action = ActionKind::Custom(symbol!("focus_ring"));
        if find(self.runtime.root(), path).is_some_and(|node| node.actions.contains(action)) {
            let _ = self.runtime.dispatch(path, action, Some(PropValue::Bool(ring)));
        }
    }

    /// Tells what had the keyboard that it hasn't, after what has it now has been told, which has said where focus is within.
    fn tell_focus_lost(&mut self, path: &[u32]) {
        let offered = find(self.runtime.root(), path).is_some_and(|node| node.actions.contains(ActionKind::Focus));
        if offered {
            let _ = self.runtime.dispatch(path, ActionKind::Focus, Some(PropValue::Bool(false)));
        }
    }

    fn tell_focus(&mut self, path: &[u32], focused: bool) {
        // What it's in hears about it as well, as `:focus-within` wants.
        let within = if focused { path } else { &[] };
        let within = within.to_vec();
        self.tell_pointer(&within, focus_within_name(), 2);
        let offered = find(self.runtime.root(), path)
            .is_some_and(|node| node.actions.contains(ActionKind::Focus));
        if offered {
            // It's only being told, so whether it has anything to do with that is up to it.
            let input = Some(PropValue::Bool(focused));
            let _ = self.runtime.dispatch(path, ActionKind::Focus, input);
        }
    }

    /// Finds the focused node again now that the tree has changed, or lets go of it if it's gone.
    fn follow_focus(&mut self) {
        #[cfg(test)]
        {
            self.interaction_counts[1] += 1;
        }
        let Some(focus) = &mut self.focus else {
            return;
        };
        let mut node = self.runtime.root();
        let mut found = true;
        for (step, key) in focus.path.iter_mut().zip(&focus.keys) {
            let same = node
                .children
                .get(*step as usize)
                .is_some_and(|child| child.key == *key);
            if !same {
                let moved = key.as_ref().and_then(|key| {
                    node.children
                        .iter()
                        .position(|child| child.key.as_ref() == Some(key))
                });
                let Some(moved) = moved else {
                    found = false;
                    break;
                };
                *step = moved as u32;
            }
            node = &node.children[*step as usize];
        }
        if !found || !focusable(node) {
            self.focus = None;
            self.selecting = false;
            self.invalidate_automatic();
            self.repaint = true;
            return;
        }
        if let Some(edit) = &mut focus.edit {
            edit.clamp(input_value(node));
            focus.carets = None;
        }
    }

    /// Shows the caret right away, and starts its blinking over.
    /// Selects from `start` to `end` of the focused field's text, in UTF-16 units, as a page's `setSelectionRange()` does,
    /// if it's the field with this key. It's held at `start`, and the caret goes to `end`.
    fn select_in_focused(&mut self, key: &Key, start: u32, end: u32) {
        let node = self.focused_node();
        let named = node.key.as_ref() == Some(key)
            || match (node.props.get("dom_key"), key) {
                (Some(PropValue::Str(text)), Key::Str(wanted)) => text.as_str() == wanted.as_str(),
                (Some(PropValue::Str(text)), Key::Sym(wanted)) => text.as_str() == wanted.as_str(),
                _ => false,
            };
        if !named {
            return;
        }
        let value = input_value(&node);
        let byte = |units: u32| {
            let mut count = 0;
            for (at, character) in value.char_indices() {
                if count >= units {
                    return at;
                }
                count += character.len_utf16() as u32;
            }
            value.len()
        };
        let (from, to) = (byte(start), byte(end));
        if let Some(edit) = self.focus.as_mut().and_then(|focus| focus.edit.as_mut()) {
            edit.place(from);
            edit.extend_to(to);
            self.wake_caret();
            self.repaint = true;
        }
        // Where code put it is said, but not as `select`, which the page says itself once code selects: see `selectHeard`.
        if let Some(edit) = self.focus.as_ref().and_then(|focus| focus.edit.as_ref()) {
            let (anchor, caret) = (edit.anchor(), edit.caret());
            if let Some(said) = page_key(&node) {
                say_field_selection(said.to_owned(), anchor, caret);
                // The page knows, as it's what it said.
                self.selection_told = Some((said.to_owned(), anchor, caret));
            }
        }
    }

    fn wake_caret(&mut self) {
        self.caret_on = true;
        self.caret_changes_left = CARET_CHANGES;
    }

    /// Whether the caret needs [`blink`](Self::blink) calling on a timer.
    pub fn caret_blinking(&self) -> bool {
        self.active
            && self.caret_changes_left > 0
            && !self.focus_is_read_only()
            && self
                .focus
                .as_ref()
                .and_then(|focus| focus.edit.as_ref())
                .is_some_and(|edit| !edit.has_selection())
    }

    /// Shows or hides the caret, for it to blink. It ends up shown.
    pub fn blink(&mut self) {
        self.caret_changes_left = self.caret_changes_left.saturating_sub(1);
        self.caret_on = self.caret_changes_left == 0 || !self.caret_on;
        self.blinked = true;
    }

    /// The focused node, or the root if nothing has focus. Holding on to this is one reference
    /// count, however long a value it holds, and it doesn't stop the tree from moving on.
    /// Whether what has the keyboard is a password.
    pub(crate) fn typing_a_password(&self) -> bool {
        self.focus.is_some() && password_mask(&self.focused_node()).is_some()
    }

    fn focused_node(&self) -> Node {
        self.focus
            .as_ref()
            .and_then(|focus| find(self.runtime.root(), &focus.path))
            .unwrap_or(self.runtime.root())
            .clone()
    }

    /// Whether what has the keyboard is a field that's `read_only`: it can be selected in and copied from, as in a browser,
    /// but nothing typed, pasted, cut, deleted or composed changes it.
    pub(crate) fn focus_is_read_only(&self) -> bool {
        self.focus
            .as_ref()
            .and_then(|focus| find(self.runtime.root(), &focus.path))
            .is_some_and(|node| is_read_only(node))
    }

    /// The byte nearest this position across the focused input.
    fn input_byte_at(&mut self, x: f32, y: f32) -> Option<usize> {
        let focus = self.focus.as_ref()?;
        focus.edit.as_ref()?;
        let node = find(self.runtime.root(), &focus.path)?;
        let rect = self.layout.borrow().rect(&focus.path)?;
        let mut style = TextStyle::from_element(node);
        style = style.scaled(self.scale);
        let content = content_box(node, rect, self.scale);
        let value = input_value(node);
        if is_multiline(node) {
            let line_height = self.text.line_height(style);
            let starts = self.text.wrapped_starts(value, style, content.width);
            let line = ((y - content.top + focus.scroll_y) / line_height)
                .floor()
                .max(0.0) as usize;
            let line = line.min(starts.len().saturating_sub(1));
            return Some(byte_along_line(
                &mut self.text,
                value,
                style,
                content.width,
                &starts,
                line,
                x - content.left,
            ));
        }
        let mask = password_mask(node);
        let shown = mask.as_deref().unwrap_or(value);
        let indent = input_indent(&mut self.text, node, shown, style, content.width);
        let at = self
            .text
            .byte_at_x(shown, style, x - content.left - indent + focus.scroll);
        Some(if mask.is_some() {
            byte_from_mask(value, at)
        } else {
            at
        })
    }

    /// A key was pressed.
    ///
    /// Tab moves the keyboard. A text input then takes the keys that edit it, while enter or
    /// space does what clicking does. Any other key goes to the nearest node offering
    /// [`ActionKind::KeyPress`], starting from what has the keyboard, and on to the next if its
    /// handler calls [`Updates::pass_on`](rsui_runtime::Updates::pass_on), which is how to make a
    /// shortcut for a part of the UI or, on the root, for all of it.
    ///
    /// Before any of that, what offers to hear `key_down`, from the root down to what has the keyboard and back up, as
    /// code written for the web does, hears it, and if one of them calls
    /// [`Updates::prevent_default`](rsui_runtime::Updates::prevent_default), that's all that happens, and what the key would
    /// type isn't typed. Keys that are only held along with others, like shift, go no further than that.
    pub fn key(&mut self, key: KeyPress) -> Result<(), DispatchError> {
        let done = self.key_here(key);
        // What the key selected in a field is said, and heard as `select`, before the key comes up.
        self.publish_selection()?;
        done
    }

    fn key_here(&mut self, key: KeyPress) -> Result<(), DispatchError> {
        let inspect_key = (key.ctrl
            && key.shift
            && !key.alt
            && matches!(key.code, KeyCode::Character('i' | 'I')))
            || (key.code == KeyCode::Function(12) && !key.ctrl && !key.alt && !key.shift);
        if inspect_key && crate::inspect::inspector_allowed() {
            if !key.repeat {
                self.open_inspector();
            }
            return Ok(());
        }
        // What's drawn over the app, the flashes and the meter, without the inspector.
        if key.ctrl
            && key.shift
            && !key.alt
            && matches!(key.code, KeyCode::Character('f' | 'F'))
            && crate::inspect::inspector_allowed()
        {
            if !key.repeat {
                self.show_inspector(!self.inspector_showing());
            }
            return Ok(());
        }
        if key.ctrl
            && key.shift
            && !key.alt
            && matches!(key.code, KeyCode::Character('s' | 'S'))
            && self.inspector_showing()
        {
            if let Some(save) = crate::inspect::trace_saver().filter(|_| !key.repeat) {
                let said = match save(self) {
                    Ok(path) => format!("Saved in {}. rsui-tsx --view opens it.", path.display()),
                    Err(error) => format!("It couldn't be saved: {error}"),
                };
                eprintln!("{said}");
                if let Some(inspector) = self.inspector.as_mut() {
                    inspector.tell(said);
                    inspector.redraw = true;
                }
            }
            return Ok(());
        }
        self.wake_caret();
        self.keyboard_last = true;
        if !key.ctrl && !key.alt && !key.meta {
            self.saw_keyboard();
        }
        self.dismiss_tooltip();
        self.update();
        self.lay_out();
        self.swallow_typing = false;
        if key.code == KeyCode::Escape && self.is_dragging() {
            // That's the end of dragging, with nothing dropped.
            self.cancel_drag();
            return Ok(());
        }
        if self.tell_key(Kind::KeyDown, key)? {
            // What it would type comes next, by itself, and isn't to be.
            self.swallow_typing = matches!(key.code, KeyCode::Character(_) | KeyCode::Space)
                && !key.ctrl
                && !key.alt
                && !key.meta;
            return Ok(());
        }
        if key.code.is_modifier() || key.code == KeyCode::Process {
            return Ok(());
        }
        // What the key's handlers changed is in the tree before what it does by itself, as a page's DOM is by then: Radix's
        // Shift+Tab out of a radio group takes its group out of the tab order as the key goes down.
        self.update();
        // Pasting, copying and cutting are said first, wherever the keyboard is, and can be stopped.
        if key.ctrl && !key.alt && !key.shift {
            let what = match key.code {
                KeyCode::Character('v') => Some("paste"),
                KeyCode::Character('c') => Some("copy"),
                KeyCode::Character('x') => Some("cut"),
                _ => None,
            };
            if let Some(what) = what {
                if self.tell_clipboard(what)? {
                    return Ok(());
                }
            }
        }
        if key.code == KeyCode::Tab && !key.ctrl && !key.alt {
            // From where `keydown`'s handlers put the keyboard, as Radix's NavigationMenu gives it to a stand-in after its last
            // link for Tab to go on from, as a browser does.
            if let Some(wanted) = self.runtime.take_focus_wanted() {
                self.focus_on_key(&wanted);
            }
            self.move_focus(key.shift);
            return Ok(());
        }
        if key.code == KeyCode::Escape {
            if let Some(path) = self.top_dismissable() {
                return self.runtime.dispatch(&path, ActionKind::Dismiss, None);
            }
        }
        if self.move_in_menu(key.code) {
            return Ok(());
        }
        if self.text_selection_key(key) {
            return Ok(());
        }
        if let Some(result) = self.edit_key(key) {
            return result;
        }
        let path = self.focused().map(<[u32]>::to_vec).unwrap_or_default();
        // What says Enter and Space don't click it, as a page's `<select>` (which opens its list on Space) does, hears them as keys.
        let key_clicks = node_at(self.runtime.root(), &path).props.get_symbol(symbol!("key_clicks")) != Some(&PropValue::Bool(false));
        // A link only takes Enter, as on a page, where Space scrolls instead.
        if matches!(key.code, KeyCode::Enter | KeyCode::Space)
            && !key.ctrl
            && !key.alt
            && self.focus.is_some()
            && key_clicks
            && activates(node_at(self.runtime.root(), &path))
            && (key.code == KeyCode::Enter || node_at(self.runtime.root(), &path).role != Role::Link)
        {
            // Enter clicks as it goes down, as a browser's button has it, and Space on a page's element when it comes up, which
            // is `key_up`'s to do. rsui's own click as Space goes down.
            if key.code == KeyCode::Space && page_key(node_at(self.runtime.root(), &path)).is_some() {
                if !key.repeat {
                    self.space_on = Some(path);
                }
                return Ok(());
            }
            if !key.repeat {
                return self.activate(&path);
            }
        }
        if matches!(key.code, KeyCode::Enter | KeyCode::Space)
            && !key.repeat
            && !key.ctrl
            && !key.alt
            && self.focus.is_some()
            && key_clicks
        {
            // A button that does nothing itself is still clicked, as a page's `<button>` with no handler of its own is: what
            // listens around it hears the click, with no clicks in a row, as from a keyboard.
            let node = node_at(self.runtime.root(), &path);
            if node.role == Role::Button && !is_disabled(node) {
                let mut event = self.click_event(None, &path, 0);
                if !self.fire(Kind::Click, &path, None, &mut event)?.stopped {
                    self.tell_document(listens::CLICK, "click", 3, &mut event);
                }
                return Ok(());
            }
        }
        // Ctrl+Home and Ctrl+End go to the start and the end as Home and End do, as a browser's do with nothing to edit.
        let scrolls = !key.alt && !key.shift && (!key.ctrl || matches!(key.code, KeyCode::Home | KeyCode::End));
        let mut path = path;
        loop {
            let node = node_at(self.runtime.root(), &path);
            if node.actions.contains(ActionKind::KeyPress) {
                let input = Some(PropValue::from(key.describe()));
                self.runtime.dispatch(&path, ActionKind::KeyPress, input)?;
                if !self.runtime.take_passed_on() {
                    return Ok(());
                }
                // The tree hasn't changed yet, as that waits for the next update.
            }
            let node = node_at(self.runtime.root(), &path);
            if scrolls && node.actions.contains(ActionKind::Scroll) {
                if let Some((step, sideways)) = step_for_key(key.code, node) {
                    if self.move_scroller(&path, step, sideways)? {
                        return Ok(());
                    }
                }
            }
            // What's underneath doesn't hear keys meant for what has the keyboard to itself.
            if self
                .trap
                .as_ref()
                .is_some_and(|trap| trap.as_slice() == path)
            {
                return Ok(());
            }
            if path.pop().is_none() {
                break;
            }
        }
        // With nothing on the keyboard, these go to the list under the pointer, like the wheel, or else to a web page's own
        // scrolling, as a browser's keys scroll the document when nothing on it has the keyboard.
        if scrolls && self.focus.is_none() {
            let under = self.pointer.and_then(|(x, y)| self.target_for(x, y, ActionKind::Scroll));
            let target = under.or_else(|| {
                let mut path = Vec::new();
                web_page_scroller(self.runtime.root(), &mut path).then_some(path)
            });
            if let Some(path) = target {
                let node = node_at(self.runtime.root(), &path);
                if let Some((step, sideways)) = step_for_key(key.code, node) {
                    self.scroll_chain(path, step, sideways)?;
                    return Ok(());
                }
            }
        }
        // Escape that nothing took lets go of the keyboard in rsui's own UI; a page's element keeps it, as in a browser.
        if key.code == KeyCode::Escape
            && !self
                .focused()
                .is_some_and(|path| page_key(node_at(self.runtime.root(), path)).is_some())
        {
            self.set_focus(None, false);
        }
        Ok(())
    }

    /// The arrow keys, in a menu, go through what's in it. Says if they did.
    fn move_in_menu(&mut self, code: KeyCode) -> bool {
        let Some(trap) = &self.trap else {
            return false;
        };
        if !matches!(
            code,
            KeyCode::Up | KeyCode::Down | KeyCode::Home | KeyCode::End
        ) {
            return false;
        }
        let root = self.runtime.root();
        if node_at(root, trap).role != Role::Menu {
            return false;
        }
        let mut stops = Vec::new();
        let mut walking = trap.clone();
        collect_focusable(node_at(root, trap), &mut walking, &mut stops);
        if stops.is_empty() {
            return true;
        }
        let current = self
            .focused()
            .and_then(|path| stops.iter().position(|stop| stop.as_slice() == path));
        let last = stops.len() - 1;
        let next = match (code, current) {
            (KeyCode::Home, _) | (KeyCode::Down, None) => 0,
            (KeyCode::End, _) | (KeyCode::Up, None) => last,
            (KeyCode::Down, Some(at)) => (at + 1) % stops.len(),
            (_, Some(at)) => (at + last) % stops.len(),
            _ => 0,
        };
        let target = stops.swap_remove(next);
        self.last_focus_by_pointer = false;
        self.set_focus(Some(target), true);
        true
    }

    /// Moves the keyboard to the next node that can take it, in the order they appear. While something
    /// on top has the keyboard to itself, that's all there is to go through.
    /// Takes out of `stops` what's a page's Tab stop only because it scrolls, when it has nothing to scroll, as a browser's
    /// keyboard-focusable scrollers are only those with somewhere to go: react-resizable-panels' panels are `overflow: auto`
    /// with a word in them, which Tab went to.
    fn leave_out_still_scrollers(&mut self, stops: &mut Vec<Vec<u32>>) {
        let root = self.runtime.root();
        let scroller_only = |node: &Element| {
            page_key(node).is_some()
                && node.actions.contains(ActionKind::Scroll)
                && (node.props.get_symbol(symbol!("focusable")) == Some(&PropValue::Bool(false)) || !focusable_by_itself(node))
        };
        if !stops.iter().any(|stop| scroller_only(node_at(root, stop))) {
            return;
        }
        self.lay_out();
        let root = self.runtime.root();
        let layout = self.layout.borrow();
        stops.retain(|stop| {
            let node = node_at(root, stop);
            !scroller_only(node)
                || layout.scroll_info(stop, node).is_none_or(|info| {
                    info.content.width > info.view.width + 0.5 || info.content.height > info.view.height + 0.5
                })
        });
    }

    fn move_focus(&mut self, backwards: bool) {
        let mut stops = Vec::new();
        match &self.trap {
            Some(trap) => {
                let mut walking = trap.clone();
                collect_focusable(node_at(self.runtime.root(), trap), &mut walking, &mut stops);
            }
            None => collect_focusable(self.runtime.root(), &mut Vec::new(), &mut stops),
        }
        self.leave_out_still_scrollers(&mut stops);
        let focused = self.focused().map(<[u32]>::to_vec);
        if stops.is_empty() {
            // What has the keyboard isn't a stop (a Radix group whose items are all disabled), and there are none: it goes to the
            // page, as a browser has it.
            if focused.is_some() && self.trap.is_none() && self.page_has_focus_order() {
                self.set_focus(None, false);
                self.keyboard_leaves_page();
            }
            return;
        }
        let current = focused
            .as_deref()
            .and_then(|path| stops.iter().position(|stop| stop.as_slice() == path));
        // Something that has the keyboard but isn't a stop itself, as an element with `tabindex="-1"` given it by a click, goes on
        // from where it is: to the next stop after it, or the one before it, as a browser has it, and past them to the page.
        if current.is_none() && self.trap.is_none() && self.page_has_focus_order() {
            if let Some(path) = focused.as_deref() {
                let next = if backwards {
                    stops.iter().rposition(|stop| stop.as_slice() < path)
                } else {
                    stops.iter().position(|stop| stop.as_slice() > path)
                };
                match next {
                    Some(at) => {
                        let target = stops.swap_remove(at);
                        self.set_focus(Some(target), true);
                        // In the middle if it isn't all in view, as from any other Tab: see `reveal_moved_focus`.
                        self.reveal_moved_focus();
                    }
                    None => {
                        self.set_focus(None, false);
                        self.keyboard_leaves_page();
                    }
                }
                return;
            }
        }
        // Past the last, or before the first, the keyboard goes to the page itself, as a browser has it, and on round from there
        // with the next Tab. What's trapped, as a menu, goes straight round.
        let past_end = match (current, backwards) {
            (Some(at), false) => at + 1 == stops.len(),
            (Some(at), true) => at == 0,
            (None, _) => false,
        };
        if past_end && self.trap.is_none() && self.page_has_focus_order() {
            self.set_focus(None, false);
            self.keyboard_leaves_page();
            return;
        }
        let next = match (current, backwards) {
            (Some(at), false) => (at + 1) % stops.len(),
            (Some(at), true) => (at + stops.len() - 1) % stops.len(),
            (None, false) => 0,
            (None, true) => stops.len() - 1,
        };
        let target = stops.swap_remove(next);
        self.last_focus_by_pointer = false;
        // Coming back in, the page hears it has the keyboard again as what's given it does (see `keyboardleft` in web.js).
        self.keyboard_left = false;
        self.set_focus(Some(target), true);
        self.reveal_moved_focus();
    }

    /// Tab has taken the keyboard out of the page, past its last stop or before its first: as a browser's toolbar has it then,
    /// the page hears what had it lose it, and then the window, and `document.hasFocus()` says no until it's back.
    fn keyboard_leaves_page(&mut self) {
        self.keyboard_left = true;
        let mut event = rsui_runtime::Event::default();
        self.runtime.deliver_document_event("keyboardleft", &mut event);
        self.runtime.take_stopped();
    }

    /// A press on a page the keyboard had left gives it back, which the page hears after `mousedown`, before what's pressed gets
    /// the focus, as Chromium has it.
    fn keyboard_back_to_page(&mut self) {
        if std::mem::take(&mut self.keyboard_left) {
            let mut event = rsui_runtime::Event::default();
            self.runtime.deliver_document_event("keyboardback", &mut event);
            self.runtime.take_stopped();
        }
    }

    /// Whether this is a page, whose own document is a stop between its last and first, as a browser's is: the window's root says
    /// so by being a page's (`dom_key` on its scrolling area).
    fn page_has_focus_order(&self) -> bool {
        is_page(self.runtime.root())
    }

    /// Carries out a key meant for the focused text input. `None` if it isn't one of those.
    fn edit_key(&mut self, key: KeyPress) -> Option<Result<(), DispatchError>> {
        if key.alt {
            return None;
        }
        self.focus.as_ref()?.edit.as_ref()?;
        let node = self.focused_node();
        let value = input_value(&node);
        let multiline = is_multiline(&node);
        if multiline {
            if let Some(done) = self.move_among_lines(key, &node) {
                return Some(done);
            }
        }
        if let Some(focus) = &mut self.focus {
            focus.goal_x = None;
            focus.upstream = None;
        }
        let edit = self.focus.as_ref()?.edit.as_ref()?;
        let unit = if key.ctrl {
            Unit::Word
        } else {
            Unit::Character
        };
        // Left and right are directions on screen, which right-to-left text reverses.
        let sideways = matches!(key.code, KeyCode::Left | KeyCode::Right);
        let mut style = TextStyle::from_element(&node);
        style = style.scaled(self.scale);
        if sideways && !key.ctrl {
            let (caret, selection) = (edit.caret(), edit.selection());
            let rightward = key.code == KeyCode::Right;
            if let Some(next) = self.text.caret_across(value, style, caret, rightward) {
                let target = if edit.has_selection() && !key.shift {
                    // Moving with something selected goes to that side of the selection.
                    let mut ends = [(selection.0, 0.0), (selection.1, 0.0)];
                    self.text.caret_positions(value, style, &mut ends);
                    if (ends[0].1 < ends[1].1) == rightward {
                        selection.1
                    } else {
                        selection.0
                    }
                } else {
                    next
                };
                let edit = self.focus.as_mut()?.edit.as_mut()?;
                if key.shift {
                    edit.extend_to(target);
                } else {
                    edit.place(target);
                }
                self.invalidate_automatic();
                self.repaint = true;
                return Some(Ok(()));
            }
        }
        let flipped = sideways && self.text.is_right_to_left(value, style);
        let moved = match key.code {
            KeyCode::Left if flipped => Some((Direction::Forward, unit)),
            KeyCode::Right if flipped => Some((Direction::Back, unit)),
            KeyCode::Left => Some((Direction::Back, unit)),
            KeyCode::Right => Some((Direction::Forward, unit)),
            KeyCode::Home => Some((Direction::Back, Unit::Line)),
            KeyCode::End => Some((Direction::Forward, Unit::Line)),
            _ => None,
        };
        if let Some((direction, unit)) = moved {
            let edit = self.focus.as_mut()?.edit.as_mut()?;
            edit.go(value, direction, unit, key.shift);
            self.invalidate_automatic();
            self.repaint = true;
            return Some(Ok(()));
        }
        let change = match key.code {
            KeyCode::Backspace => edit.delete(value, Direction::Back, unit),
            KeyCode::Delete => edit.delete(value, Direction::Forward, unit),
            KeyCode::Enter if multiline && !key.ctrl => edit.insert(value, "\n").map(|change| change.said_as("insertLineBreak")),
            // These come through as typing.
            KeyCode::Space if !key.ctrl => None,
            KeyCode::Character(_) if !key.ctrl => None,
            KeyCode::Character('a') => {
                let edit = self.focus.as_mut()?.edit.as_mut()?;
                edit.select_all(value);
                self.invalidate_automatic();
                self.repaint = true;
                return Some(Ok(()));
            }
            KeyCode::Character('c') => {
                self.copy_selection(value);
                return Some(Ok(()));
            }
            // Cutting from what can't be changed does nothing, not even copy, as a browser's can't cut there.
            KeyCode::Character('x') if self.focus_is_read_only() => return Some(Ok(())),
            KeyCode::Character('x') => {
                self.copy_selection(value);
                let edit = self.focus.as_ref()?.edit.as_ref()?;
                edit.has_selection()
                    .then(|| edit.delete(value, Direction::Back, Unit::Character))
                    .flatten()
                    .map(|change| change.said_as("deleteByCut"))
            }
            KeyCode::Character('v') => {
                let pasted = self.clipboard.read().unwrap_or_default();
                let edit = self.focus.as_ref()?.edit.as_ref()?;
                if pasted.is_empty() {
                    None
                } else if multiline {
                    let pasted = pasted.replace("\r\n", "\n");
                    edit.insert(value, as_much_as_fits(&node, value, edit, &pasted))
                        .map(|change| change.said_as("insertFromPaste"))
                } else {
                    let pasted = one_line(&pasted);
                    edit.insert(value, as_much_as_fits(&node, value, edit, &pasted))
                        .map(|change| change.said_as("insertFromPaste"))
                }
            }
            KeyCode::Character('z') if key.shift => edit.redo(value),
            KeyCode::Character('z') => edit.undo(value),
            KeyCode::Character('y') => edit.redo(value),
            _ => return None,
        };
        Some(self.apply(value, change))
    }

    /// Keys that go between lines in a text input with several, or to the ends of one. `None` if this
    /// is some other key.
    fn move_among_lines(
        &mut self,
        key: KeyPress,
        node: &Node,
    ) -> Option<Result<(), DispatchError>> {
        if !matches!(
            key.code,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Home
                | KeyCode::End
        ) {
            return None;
        }
        let focus = self.focus.as_ref()?;
        let caret = focus.edit.as_ref()?.caret();
        let goal = focus.goal_x;
        let upstream = focus.upstream == Some(caret);
        let rect = self.layout.borrow().rect(&focus.path)?;
        let value = input_value(node);
        let mut style = TextStyle::from_element(node);
        style = style.scaled(self.scale);
        let content = content_box(node, rect, self.scale);
        let starts = self.text.wrapped_starts(value, style, content.width);
        let (line, x) = place_in_lines(&mut self.text, value, style, content.width, &starts, caret, upstream);
        let line_height = self.text.line_height(style);
        let page = ((content.height / line_height).floor() as usize).max(2) - 1;
        let last = starts.len() - 1;
        let (target, keeps_goal) = match key.code {
            KeyCode::Home if key.ctrl => (0, false),
            KeyCode::End if key.ctrl => (value.len(), false),
            KeyCode::Home => (starts[line], false),
            KeyCode::End => (
                starts[line] + wrapped_line(value, &starts, line).len(),
                false,
            ),
            code => {
                let goal = goal.unwrap_or(x);
                let (going_up, step) = match code {
                    KeyCode::Up => (true, 1),
                    KeyCode::Down => (false, 1),
                    KeyCode::PageUp => (true, page),
                    _ => (false, page),
                };
                let target = if going_up && line == 0 {
                    0
                } else if !going_up && line == last {
                    value.len()
                } else {
                    let next = if going_up {
                        line.saturating_sub(step)
                    } else {
                        (line + step).min(last)
                    };
                    byte_along_line(
                        &mut self.text,
                        value,
                        style,
                        content.width,
                        &starts,
                        next,
                        goal,
                    )
                };
                let focus = self.focus.as_mut()?;
                focus.goal_x = Some(goal);
                (target, true)
            }
        };
        let focus = self.focus.as_mut()?;
        if !keeps_goal {
            focus.goal_x = None;
        }
        // A byte where a wrapped line ends and the next starts is the end of the line it was reached along, as a browser has it:
        // End there, or going up or down to a line that's shorter than where the caret was.
        let along_line = matches!(key.code, KeyCode::End | KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown) && !key.ctrl;
        let wraps_at = |at: usize| at > 0 && starts.binary_search(&at).is_ok() && !value[..at].ends_with('\n');
        focus.upstream = (along_line && wraps_at(target)).then_some(target);
        let edit = focus.edit.as_mut()?;
        if key.shift {
            edit.extend_to(target);
        } else {
            edit.place(target);
        }
        self.invalidate_automatic();
        self.repaint = true;
        Some(Ok(()))
    }

    /// The wheel, over a text input with several lines. Says if it went anywhere.
    fn scroll_text_area(&mut self, x: f32, y: f32, rows: i64) -> bool {
        self.scroll_text_area_pixels(x, y, rows as f32 * WHEEL_ROW)
    }

    fn scroll_text_area_pixels(&mut self, x: f32, y: f32, pixels: f32) -> bool {
        let Some(path) = self.path_at(x, y) else {
            return false;
        };
        let Some(node) = find(self.runtime.root(), &path).cloned() else {
            return false;
        };
        let Some(rect) = self.layout.borrow().rect(&path) else {
            return false;
        };
        if !is_multiline(&node) {
            return false;
        }
        let scale = self.scale;
        let mut style = TextStyle::from_element(&node);
        style.size *= scale;
        let area = content_box(&node, rect, scale);
        let focused = self.focus.as_ref().filter(|focus| focus.path == path);
        let (lines, scroll) = match focused {
            Some(focus) => {
                let Some(carets) = &focus.carets else {
                    return false;
                };
                (carets.line_count, focus.scroll_y)
            }
            None => {
                let lines = self
                    .text
                    .wrapped_starts(input_value(&node), style, area.width)
                    .len() as u32;
                let scroll = match &self.resting_area {
                    Some((resting, scroll)) if *resting == path => *scroll,
                    _ => 0.0,
                };
                (lines, scroll)
            }
        };
        let total = lines as f32 * self.text.line_height(style);
        let moved = (scroll + pixels * scale)
            .min((total - area.height).max(0.0))
            .max(0.0);
        if (moved - scroll).abs() < 0.001 {
            return false;
        }
        match self.focus.as_mut().filter(|focus| focus.path == path) {
            Some(focus) => focus.scroll_y = moved,
            None => self.resting_area = Some((path, moved)),
        }
        self.invalidate_automatic();
        self.repaint = true;
        true
    }

    fn copy_selection(&mut self, value: &str) {
        let Some(edit) = self.focus.as_ref().and_then(|focus| focus.edit.as_ref()) else {
            return;
        };
        // A password doesn't leave by way of the clipboard.
        if password_mask(&self.focused_node()).is_some() {
            return;
        }
        if edit.has_selection() {
            let (start, end) = edit.selection();
            self.clipboard.write(&value[start..end]);
        }
    }

    /// A key was let go of, which is only for what offers to hear `key_up`, from the root down to what has the keyboard
    /// and back up, as code written for the web does.
    pub fn key_up(&mut self, key: KeyPress) -> Result<(), DispatchError> {
        self.update();
        let prevented = self.tell_key(Kind::KeyUp, key)?;
        if key.code == KeyCode::Space {
            // What Space went down on is clicked, if it still has the keyboard and nothing said not to.
            if let Some(path) = self.space_on.take() {
                self.update();
                let still = self.focused().is_some_and(|now| now == path.as_slice());
                if still && !prevented && activates(node_at(self.runtime.root(), &path)) {
                    return self.activate(&path);
                }
            }
        }
        Ok(())
    }

    /// Text was typed. Control characters are left out, as they're not text. What a key typed isn't, if what heard the key
    /// go down said so.
    pub fn type_text(&mut self, text: &str) -> Result<(), DispatchError> {
        let done = self.type_text_here(text);
        self.publish_selection()?;
        done
    }

    fn type_text_here(&mut self, text: &str) -> Result<(), DispatchError> {
        if std::mem::take(&mut self.swallow_typing) {
            return Ok(());
        }
        let Some(edit) = self.focus.as_ref().and_then(|focus| focus.edit.as_ref()) else {
            // Something a page can edit puts it in itself.
            let text: String = text.chars().filter(|c| !c.is_control()).collect();
            if text.is_empty() {
                return Ok(());
            }
            return self.typed_elsewhere(&text);
        };
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        // Ctrl+A types a control character after it's selected everything: with nothing left of it, nothing's typed, rather
        // than nothing put in place of what's selected.
        if text.is_empty() {
            return Ok(());
        }
        let node = self.focused_node();
        let value = input_value(&node);
        let text = as_much_as_fits(&node, value, edit, &text);
        let change = edit.insert(value, text);
        if let Some(focus) = &mut self.focus {
            focus.goal_x = None;
            focus.upstream = None;
        }
        self.wake_caret();
        self.apply(value, change)
    }

    /// Offers a change to the focused input's component, whose answer is whatever value it then
    /// shows. If that's what it had before, nothing happened. If it's something else, the
    /// caret goes as near as it can to where the change would have put it.
    fn apply(&mut self, before: &str, change: Option<Change>) -> Result<(), DispatchError> {
        let Some(change) = change else {
            return Ok(());
        };
        let path = self.focused().map(<[u32]>::to_vec).unwrap_or_default();
        // It can have the keyboard, for selecting and copying, but nothing it does changes anything.
        if self.focus_is_read_only() {
            return Ok(());
        }
        let input = Some(PropValue::from(change.value.clone()));
        if node_at(self.runtime.root(), &path)
            .actions
            .contains(ActionKind::SetValue)
        {
            // What the change is, for a page's `beforeinput` and `input`: see `edit_in_progress`.
            let said = EditSaid {
                input_type: change.input_type(),
                data: change.data().map(str::to_owned),
            };
            EDITING.with(|editing| *editing.borrow_mut() = Some(said));
            if let Some(key) = page_key(node_at(self.runtime.root(), &path)) {
                let (anchor, caret) = change.ends();
                say_field_selection(key.to_owned(), anchor, caret);
            }
            let dispatched = self.runtime.dispatch(&path, ActionKind::SetValue, input);
            EDITING.with(|editing| *editing.borrow_mut() = None);
            dispatched?;
        }
        self.update();
        self.invalidate_automatic();
        self.repaint = true;
        // Typing shows what's being typed in, wherever it had been scrolled to.
        self.reveal_focus();
        let node = self.focused_node();
        let now = input_value(&node);
        if let Some(focus) = &mut self.focus {
            if let Some(edit) = &mut focus.edit {
                if now == before {
                    edit.clamp(now);
                } else {
                    edit.commit(&change.settled_as(before, now));
                }
                focus.carets = None;
            }
        }
        Ok(())
    }

    /// Whether someone looks to be about to use this: the pointer is over it, or they're partway
    /// through typing in it.
    pub fn attended(&self) -> bool {
        self.pointer.is_some()
            || self
                .focus
                .as_ref()
                .is_some_and(|focus| focus.edit.is_some())
    }

    /// Lets go of what's only being remembered to save time, for when things have gone quiet.
    /// Runs [`on_quiet`]'s hook for [`Quiet::Moment`] first. A window runs that hook on its own, once it's been quiet for a few
    /// seconds, so this is for an app run some other way.
    pub fn forget_caches(&mut self) {
        run_quiet_hook(Quiet::Moment);
        self.forget_own_caches();
    }

    /// [`Self::forget_caches`] without the hook.
    pub(crate) fn forget_own_caches(&mut self) {
        self.text.forget_caches();
        self.painter.forget_caches();
    }

    /// Hand idle image retirements to the renderer without painting or presenting.
    /// Call after `forget_caches` when managing a renderer outside the native window.
    pub fn release_unused_images(&mut self, renderer: &mut Renderer) {
        self.painter.release_unused_images_to(renderer);
    }

    /// The mouse wheel, at this position, by this many rows. Positive is downwards.
    pub fn scroll(&mut self, x: f32, y: f32, rows: i64) -> Result<(), DispatchError> {
        let result = self.with_wheel_hit_memo(|app| app.scroll_now(x, y, rows));
        // When it happened is only for this.
        self.input_at = None;
        result
    }

    fn scroll_now(&mut self, x: f32, y: f32, rows: i64) -> Result<(), DispatchError> {
        // Several notches can arrive before the next frame, and each has to start from the last.
        self.update();
        self.dismiss_tooltip();
        // A notch is 100 pixels, as the web has it on Windows.
        if self.tell_wheel(x, y, 0.0, rows as f32 * 100.0 / 3.0) {
            return Ok(());
        }
        if self.scroll_text_area(x, y, rows) {
            return Ok(());
        }
        let Some(path) = self.target_for(x, y, ActionKind::Scroll) else {
            return Ok(());
        };
        self.scroll_chain(path, ScrollStep::Wheel(rows), false)
    }

    /// The wheel going across, which is a mouse's sideways wheel, or holding shift. Positive is
    /// towards the right. It only moves what can go across.
    pub fn scroll_across(&mut self, x: f32, y: f32, rows: i64) -> Result<(), DispatchError> {
        let result = self.with_wheel_hit_memo(|app| app.scroll_across_now(x, y, rows));
        self.input_at = None;
        result
    }

    /// The wheel's exact CSS-pixel delta. Precision input follows the device
    /// immediately; discrete notches retain the ordinary glide. Fractional input
    /// is never rounded to rows or physical pixels before it reaches the area.
    pub fn scroll_pixels(&mut self, x: f32, y: f32, pixels: f32, across: bool, precise: bool) -> Result<(), DispatchError> {
        let result = self.with_wheel_hit_memo(|app| app.scroll_pixels_now(x, y, pixels, across, precise));
        self.input_at = None;
        result
    }

    fn with_wheel_hit_memo(&mut self, run: impl FnOnce(&mut Self) -> Result<(), DispatchError>) -> Result<(), DispatchError> {
        let previous = std::mem::replace(&mut self.wheel_hit_active, true);
        self.wheel_hit_memo = None;
        let result = run(self);
        self.wheel_hit_memo = None;
        self.wheel_hit_active = previous;
        result
    }

    fn scroll_pixels_now(&mut self, x: f32, y: f32, pixels: f32, across: bool, precise: bool) -> Result<(), DispatchError> {
        if !pixels.is_finite() || pixels == 0.0 {
            return Ok(());
        }
        self.update();
        self.dismiss_tooltip();
        if self.tell_wheel(x, y, if across { pixels } else { 0.0 }, if across { 0.0 } else { pixels }) {
            return Ok(());
        }
        if !across && self.scroll_text_area_pixels(x, y, pixels) {
            return Ok(());
        }
        let Some(path) = self.target_for(x, y, ActionKind::Scroll) else {
            return Ok(());
        };
        self.scroll_chain(path, ScrollStep::Pixels(pixels, precise), across)
    }

    fn scroll_across_now(&mut self, x: f32, y: f32, rows: i64) -> Result<(), DispatchError> {
        self.update();
        self.dismiss_tooltip();
        if self.tell_wheel(x, y, rows as f32 * 100.0 / 3.0, 0.0) {
            return Ok(());
        }
        let Some(path) = self.target_for(x, y, ActionKind::Scroll) else {
            return Ok(());
        };
        self.scroll_chain(path, ScrollStep::Wheel(rows), true)
    }

    /// Moves the nearest thing around `path` that can still go this way, so that reaching the end of
    /// an inner area carries on with the one around it.
    fn scroll_chain(
        &mut self,
        mut path: Vec<u32>,
        step: ScrollStep,
        sideways: bool,
    ) -> Result<(), DispatchError> {
        loop {
            let node = node_at(self.runtime.root(), &path);
            if node.actions.contains(ActionKind::Scroll)
                && self.move_scroller(&path, step, sideways)?
            {
                return Ok(());
            }
            // What's underneath something on top doesn't scroll because of it.
            if self.layout.borrow().has_overlays() && self.layout.borrow().is_overlay(&path) {
                return Ok(());
            }
            if path.pop().is_none() {
                return Ok(());
            }
        }
    }

    /// Moves a scrolling area or a list, and says whether that went anywhere. `sideways` says it has to
    /// be across, rather than whichever way it mainly goes.
    fn move_scroller(
        &mut self,
        path: &[u32],
        step: ScrollStep,
        sideways: bool,
    ) -> Result<bool, DispatchError> {
        self.lay_out();
        let root = self.runtime.root();
        let node = node_at(root, path);
        if sideways && node.props.get("scroll_x").is_none() {
            return Ok(false);
        }
        let Some(info) = self.layout.borrow().scroll_info(path, node) else {
            // It handles scrolling some other way, so all that can be done is to tell it.
            if let ScrollStep::Wheel(_) | ScrollStep::Pixels(..) = step {
                let input = match step {
                    ScrollStep::Wheel(rows) => PropValue::Int(first_visible(node).saturating_add(rows).max(0)),
                    ScrollStep::Pixels(pixels, _) => PropValue::Float(f64::from((list_position(node) + pixels / WHEEL_ROW).max(0.0))),
                    _ => unreachable!(),
                };
                self.runtime
                    .dispatch(path, ActionKind::Scroll, Some(input))?;
                return Ok(true);
            }
            return Ok(false);
        };
        if is_list(node) {
            let page = match node.props.get("view_rows") {
                Some(PropValue::Float(rows)) => (rows.floor() as i64 - 1).max(1),
                _ => 1,
            };
            if let ScrollStep::Pixels(pixels, precise) = step {
                let height = match node.props.get("row_height") {
                    Some(PropValue::Float(height)) if *height > 0.0 => *height as f32,
                    _ => info.view.height / (page + 1) as f32,
                }.max(1.0);
                let from = list_position(node);
                let count = match node.props.get("count") {
                    Some(PropValue::Int(count)) => *count as f32,
                    _ => node.children.len() as f32,
                };
                let rows = match node.props.get("view_rows") {
                    Some(PropValue::Float(rows)) => rows.floor().max(1.0) as f32,
                    _ => 1.0,
                };
                let base = if precise { from } else {
                    self.glide.as_ref().filter(|glide| glide.path == path).map_or(from, |glide| glide.to)
                };
                let to = (base + pixels / height).clamp(0.0, (count - rows).max(0.0));
                if self.animations && !precise {
                    return Ok(self.glide_to(path, ActionKind::Scroll, Some(height), to, base, from));
                }
                self.glide.take_if(|glide| glide.path == path);
                if (to - from).abs() < f32::EPSILON {
                    return Ok(false);
                }
                self.runtime.dispatch(path, ActionKind::Scroll, Some(PropValue::Float(f64::from(to))))?;
                return Ok(true);
            }
            if self.animations && !matches!(step, ScrollStep::Start | ScrollStep::End) {
                if let Some(PropValue::Float(row_height)) = node.props.get("row_height") {
                    return Ok(self.glide_list(path, step, page, *row_height as f32));
                }
            }
            self.glide.take_if(|glide| glide.path == path);
            let first = first_visible(node);
            let last = match step {
                ScrollStep::Wheel(rows) | ScrollStep::Arrow(rows) => first.saturating_add(rows),
                ScrollStep::Pixels(..) => unreachable!("pixel list scrolling is handled above"),
                ScrollStep::Page(pages) => first.saturating_add(pages.saturating_mul(page)),
                ScrollStep::Start => 0,
                ScrollStep::End => i64::MAX,
            }
            .max(0);
            let at_end = info.y >= info.furthest_y() - 0.5;
            if last == first || (last > first && at_end) {
                return Ok(false);
            }
            let input = PropValue::Int(last);
            self.runtime
                .dispatch(path, ActionKind::Scroll, Some(input))?;
            return Ok(true);
        }
        let along_x = sideways || !is_vertical(node);
        let (view, moved, furthest) = if along_x {
            (info.view.width, info.x, info.furthest_x())
        } else {
            (info.view.height, info.y, info.furthest_y())
        };
        let action = scroll_action(node, along_x);
        // Steps taken while it's already going somewhere add to that, not to where it's got to.
        let base = self
            .glide
            .as_ref()
            .filter(|glide| glide.path == path && glide.action == action && !matches!(step, ScrollStep::Pixels(_, true)))
            .map_or(moved, |glide| glide.to);
        let target = match step {
            ScrollStep::Wheel(rows) => base + rows as f32 * WHEEL_ROW,
            ScrollStep::Pixels(pixels, _) => base + pixels,
            ScrollStep::Arrow(presses) => base + presses as f32 * ARROW_STEP,
            ScrollStep::Page(pages) if is_web(node) => base + pages as f32 * (view * WEB_PAGE_FRACTION).max(1.0),
            ScrollStep::Page(pages) => base + pages as f32 * (view - PAGE_OVERLAP).max(ARROW_STEP),
            ScrollStep::Start => 0.0,
            ScrollStep::End => furthest,
        };
        let target = target.clamp(0.0, furthest);
        // A web page glides to its start and end too, as a browser's Home and End do.
        if self.animations && !matches!(step, ScrollStep::Pixels(_, true)) && (!matches!(step, ScrollStep::Start | ScrollStep::End) || is_web(node)) {
            return Ok(self.glide_to(path, action, None, target, base, moved));
        }
        if matches!(step, ScrollStep::Pixels(_, true)) {
            self.glide.take_if(|glide| glide.path == path);
            if (target - moved).abs() < f32::EPSILON {
                return Ok(false);
            }
            let told = self.mirrored_across(path, along_x).map_or(target, |furthest| (furthest - target).max(0.0));
            self.runtime.dispatch(path, action, Some(PropValue::Float(f64::from(told))))?;
            return Ok(true);
        }
        self.scroll_area_to(path, target, moved, along_x)
    }

    /// Starts a list gliding by this step. Its places are in rows, and it only stops on whole ones.
    fn glide_list(&mut self, path: &[u32], step: ScrollStep, page: i64, row_height: f32) -> bool {
        let node = node_at(self.runtime.root(), path);
        let position = list_position(node);
        let count = match node.props.get("count") {
            Some(PropValue::Int(count)) => *count as f32,
            _ => node.children.len() as f32,
        };
        let whole_rows = match node.props.get("view_rows") {
            Some(PropValue::Float(rows)) => rows.floor().max(1.0) as f32,
            _ => 1.0,
        };
        let base = self
            .glide
            .as_ref()
            .filter(|glide| glide.path == path)
            .map_or(position, |glide| glide.to);
        let by = match step {
            ScrollStep::Wheel(rows) | ScrollStep::Arrow(rows) => rows,
            ScrollStep::Pixels(..) => unreachable!("pixel list scrolling is handled separately"),
            ScrollStep::Page(pages) => pages.saturating_mul(page),
            ScrollStep::Start | ScrollStep::End => 0,
        };
        let target = (base + by as f32).clamp(0.0, (count - whole_rows).max(0.0));
        self.glide_to(
            path,
            ActionKind::Scroll,
            Some(row_height),
            target,
            base,
            position,
        )
    }

    /// Starts a scrolling area gliding from where it is to `to`, unless that's where it was already
    /// going. Says whether that's anywhere new, as if it isn't, whatever's around it can take over.
    fn glide_to(
        &mut self,
        path: &[u32],
        action: ActionKind,
        row_height: Option<f32>,
        to: f32,
        going_to: f32,
        from: f32,
    ) -> bool {
        if (to - going_to).abs() < 0.01 {
            return false;
        }
        // Only one glides at a time. When scrolling moves on from an area that's reached its end,
        // that area still gets there.
        if self
            .glide
            .as_ref()
            .is_some_and(|glide| glide.path != path || glide.action != action)
        {
            self.finish_glide();
        }
        let now = Instant::now();
        // From when it happened, if that was a moment ago (see `set_input_time`).
        let started = self
            .input_at
            .take()
            .filter(|at| *at <= now)
            .map_or(now, |at| at.max(now - SCROLL_GLIDE / 2));
        // Another step on the way, the same way, keeps going as fast as it's going, rather than setting off again from there. Going
        // faster than setting off would start, it sets off again, as keeping that speed would take it past `to` and back.
        let fresh = 3.0 * (to - from) / SCROLL_GLIDE.as_secs_f32();
        let kept_speed = self
            .glide
            .as_ref()
            .filter(|glide| now.saturating_duration_since(glide.started) < SCROLL_GLIDE)
            .map(|glide| glide.speed(now))
            .filter(|speed| speed * fresh > 0.0 && speed.abs() < fresh.abs());
        let along_x = action == ActionKind::Custom(symbol!("scroll_across")) || !is_vertical(node_at(self.runtime.root(), path));
        let mirror = if row_height.is_none() { self.mirrored_across(path, along_x) } else { None };
        self.glide = Some(ScrollGlide {
            path: path.to_vec(),
            action,
            row_height,
            from,
            to,
            started,
            kept_speed,
            mirror,
        });
        self.invalidate_automatic();
        self.repaint = true;
        true
    }

    /// Puts a scrolling area that's gliding where it's going.
    fn finish_glide(&mut self) {
        if let Some(glide) = self.glide.take() {
            let input = self.glide_input(&glide, glide.to, true);
            let _ = self
                .runtime
                .dispatch(&glide.path, glide.action, Some(input));
        }
    }

    /// What to tell something that's gliding, for it to go here.
    fn glide_input(&self, glide: &ScrollGlide, place: f32, done: bool) -> PropValue {
        match glide.row_height {
            // It ends on a whole row, whatever size those come out as.
            Some(_) if done => PropValue::Int(place.round() as i64),
            Some(height) => PropValue::Float(f64::from(self.snapped(place * height) / height)),
            None => {
                let place = self.snapped(place);
                PropValue::Float(f64::from(glide.mirror.map_or(place, |furthest| (furthest - place).max(0.0))))
            }
        }
    }

    /// The nearest place that's a whole number of physical pixels, so that text doesn't shift by parts
    /// of a pixel as it goes past.
    fn snapped(&self, place: f32) -> f32 {
        (place * self.scale).round() / self.scale
    }

    /// Moves a scrolling area straight to here, or as near as whole physical pixels go, instead of
    /// wherever it was gliding to. Says whether it went anywhere.
    fn scroll_area_to(
        &mut self,
        path: &[u32],
        target: f32,
        moved: f32,
        along_x: bool,
    ) -> Result<bool, DispatchError> {
        self.glide.take_if(|glide| glide.path == path);
        let snapped = self.snapped(target);
        if (snapped - moved).abs() < 0.01 {
            return Ok(false);
        }
        let told = match self.mirrored_across(path, along_x) {
            Some(furthest) => (furthest - snapped).max(0.0),
            None => snapped,
        };
        let input = PropValue::Float(f64::from(told));
        let action = scroll_action(node_at(self.runtime.root(), path), along_x);
        self.runtime.dispatch(path, action, Some(input))?;
        Ok(true)
    }

    /// For something that scrolls across where text goes from right to left: how far it can go across, as it's told where to go
    /// from its right, which is where it starts (see `solve_scroll` in rsui-layout), while everything here goes from its left.
    /// `None` for anything else.
    fn mirrored_across(&self, path: &[u32], along_x: bool) -> Option<f32> {
        if !along_x {
            return None;
        }
        let node = node_at(self.runtime.root(), path);
        if !matches!(node.props.get("text_direction"), Some(PropValue::Str(way)) if way.as_str() == "rtl") {
            return None;
        }
        self.layout.borrow().scroll_info(path, node).map(|info| info.furthest_x())
    }

    /// The scrollbar at this position, if there is one, which is the innermost that's there. Pressing

    /// on the thumb grabs it where it is. Anywhere else on the track brings the thumb there first.
    fn scrollbar_at(&mut self, x: f32, y: f32) -> Option<ScrollDrag> {
        let mut path = self.path_at(x, y)?;
        let (x, y) = (x / self.scale, y / self.scale);
        loop {
            let node = node_at(self.runtime.root(), &path);
            if let (Some(info), Some(rect)) = (
                self.layout.borrow().scroll_info(&path, node),
                self.layout.borrow().rect(&path),
            ) {
                let grab = |along: f32, thumb: (f32, f32)| {
                    if along >= thumb.0 && along <= thumb.0 + thumb.1 {
                        along - thumb.0
                    } else {
                        thumb.1 / 2.0
                    }
                };
                // Down the left side where text goes from right to left (`scrollbar_left`), as it's drawn.
                let on_left = matches!(node.props.get("scrollbar_left"), Some(rsui_core::PropValue::Bool(true)));
                let on_bar = if on_left {
                    x <= rect.x + SCROLL_BAR_REACH
                } else {
                    x >= rect.x + rect.width - SCROLL_BAR_REACH
                };
                if on_bar {
                    if let Some(thumb) = scroll_thumb(info.view.height, info.content.height, info.y)
                    {
                        return Some(ScrollDrag {
                            grab: grab(y - rect.y, thumb),
                            path,
                            vertical: true,
                        });
                    }
                }
                if y >= rect.y + rect.height - SCROLL_BAR_REACH {
                    if let Some(thumb) = scroll_thumb(info.view.width, info.content.width, info.x) {
                        return Some(ScrollDrag {
                            grab: grab(x - rect.x, thumb),
                            path,
                            vertical: false,
                        });
                    }
                }
            }
            path.pop()?;
        }
    }

    /// Moves what's being dragged to follow the pointer.
    fn drag_scrollbar(&mut self, x: f32, y: f32) {
        self.update();
        self.lay_out();
        let Some(drag) = &self.dragging else {
            return;
        };
        let (path, vertical, grab) = (drag.path.clone(), drag.vertical, drag.grab);
        let Some(node) = find(self.runtime.root(), &path).cloned() else {
            self.dragging = None;
            return;
        };
        let (Some(info), Some(rect)) = (
            self.layout.borrow().scroll_info(&path, &node),
            self.layout.borrow().rect(&path),
        ) else {
            self.dragging = None;
            return;
        };
        let (x, y) = (x / self.scale, y / self.scale);
        let (track, content, moved, start) = if vertical {
            (
                info.view.height,
                info.content.height,
                info.y,
                y - rect.y - grab,
            )
        } else {
            (
                info.view.width,
                info.content.width,
                info.x,
                x - rect.x - grab,
            )
        };
        let target = scroll_for_thumb(track, content, start);
        // Whether this works is up to the thing being dragged, and it'd only be asked again.
        if let Some(PropValue::Float(row_height)) = node.props.get("row_height") {
            self.glide.take_if(|glide| glide.path == path);
            let row = (f64::from(target) / row_height).round() as i64;
            if row != first_visible(&node) {
                let input = Some(PropValue::Int(row));
                let _ = self.runtime.dispatch(&path, ActionKind::Scroll, input);
            }
        } else {
            let _ = self.scroll_area_to(&path, target, moved, !vertical);
        }
    }

    /// What the app looks like to assistive technology, right now.
    pub fn access_tree(&mut self) -> AccessTree {
        self.update();
        self.lay_out();
        access::build(
            self.runtime.root(),
            &self.layout.borrow(),
            self.scale,
            self.size,
            self.focused(),
        )
    }

    /// Whether what [`access_tree`](Self::access_tree) would say could have changed since this was
    /// last this. Working the tree out takes time, so it's only done if it might be different.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn access_fingerprint(&self) -> AccessFingerprint {
        (
            self.tree_changes,
            self.focused().map(<[u32]>::to_vec),
            self.size,
            self.scale.to_bits(),
        )
    }

    /// Does what assistive technology asked for, to the node at this path.
    pub fn access_command(
        &mut self,
        path: &[u32],
        command: AccessCommand,
    ) -> Result<(), DispatchError> {
        self.lay_out();
        if find(self.runtime.root(), path).is_none() {
            return Ok(());
        }
        match command {
            AccessCommand::Invoke => {
                self.activate(path)?;
            }
            AccessCommand::Toggle => {
                self.runtime.dispatch(path, ActionKind::Toggle, None)?;
            }
            AccessCommand::SetValue(text) => {
                let input = Some(PropValue::from(text));
                self.runtime.dispatch(path, ActionKind::SetValue, input)?;
            }
            AccessCommand::Focus => {
                self.keyboard_last = true;
                self.last_focus_by_pointer = false;
                self.focus_on(path);
            }
            AccessCommand::Reveal => self.reveal(path),
            AccessCommand::ScrollSteps(steps) => {
                self.scroll_chain(path.to_vec(), ScrollStep::Arrow(i64::from(steps)), false)?;
            }
            AccessCommand::ScrollPages(pages) => {
                self.scroll_chain(path.to_vec(), ScrollStep::Page(i64::from(pages)), false)?;
            }
            AccessCommand::ScrollTo(part) => {
                let node = node_at(self.runtime.root(), path).clone();
                let Some(info) = self.layout.borrow().scroll_info(path, &node) else {
                    return Ok(());
                };
                if let Some(PropValue::Float(row_height)) = node.props.get("row_height") {
                    let row = (part * f64::from(info.furthest_y()) / row_height).round() as i64;
                    let input = Some(PropValue::Int(row));
                    self.runtime.dispatch(path, ActionKind::Scroll, input)?;
                } else if is_vertical(&node) {
                    self.scroll_area_to(path, part as f32 * info.furthest_y(), info.y, false)?;
                } else {
                    self.scroll_area_to(path, part as f32 * info.furthest_x(), info.x, true)?;
                }
            }
        }
        Ok(())
    }

    /// Scrolls whatever's around the focused node so that it shows.
    fn reveal_focus(&mut self) {
        let Some(path) = self.focus.as_ref().map(|focus| focus.path.clone()) else {
            return;
        };
        self.reveal(&path);
    }

    /// Scrolls whatever's around what the keyboard's just gone to so that it shows. A page's element is put in the middle
    /// if it isn't all in view, as Chromium has it, and of a text field it's the line with the caret that's put there. An
    /// app's goes as little as it can, as for anything else.
    fn reveal_moved_focus(&mut self) {
        let Some(path) = self.focus.as_ref().map(|focus| focus.path.clone()) else {
            return;
        };
        let node = node_at(self.runtime.root(), &path);
        if node.props.get("dom_key").is_none() {
            self.reveal(&path);
            return;
        }
        self.update();
        self.lay_out();
        let Some(mut rect) = self.layout.borrow().rect(&path) else {
            return;
        };
        let node = node_at(self.runtime.root(), &path).clone();
        if node.role == Role::TextInput {
            let content = content_box(&node, rect, 1.0);
            let line = self.text.line_height(TextStyle::from_element(&node)).min(content.height);
            rect = Rect::new(rect.x, content.top, rect.width, line.max(1.0));
        }
        self.reveal_rect(&path, rect, true);
    }

    /// Scrolls whatever's around this node so that it shows.
    fn reveal(&mut self, path: &[u32]) {
        self.update();
        self.lay_out();
        let Some(rect) = self.layout.borrow().rect(path) else {
            return;
        };
        self.reveal_rect(path, rect, false);
    }

    /// Scrolls whatever's around the node at `path` so that `rect` shows, moving as little as it can, or putting it in the
    /// middle of each area it isn't all in view in if `centered`.
    fn reveal_rect(&mut self, path: &[u32], rect: Rect, centered: bool) {
        // Overlays are placed in the viewport, outside their logical ancestors'
        // scrolling content. Reveal inside their own scrolling areas, but never
        // move the page in an attempt to bring a viewport-fixed box into view.
        let first = (0..=path.len())
            .rev()
            .find(|depth| self.layout.borrow().is_overlay(&path[..*depth]))
            .unwrap_or(0);
        // Moving an inner area moves what's inside it, and the outer area then needs to move by
        // less, so each one works from what the last one did.
        let (mut shift_x, mut shift_y) = (0f32, 0f32);
        for depth in (first..path.len()).rev() {
            let around = &path[..depth];
            let node = node_at(self.runtime.root(), around).clone();
            if is_list(&node) {
                continue;
            }
            let (Some(info), Some(area)) = (
                self.layout.borrow().scroll_info(around, &node),
                self.layout.borrow().rect(around),
            ) else {
                continue;
            };
            if is_vertical(&node) {
                let along = if centered { center_along } else { reveal_along };
                let target = along(
                    info.y,
                    info.furthest_y(),
                    area.y,
                    area.height,
                    rect.y + shift_y,
                    rect.height,
                );
                shift_y += info.y - target;
                let _ = self.scroll_area_to(around, target, info.y, false);
            }
            if node.props.get("scroll_x").is_some() {
                let along = if centered { center_along } else { reveal_along };
                let target = along(
                    info.x,
                    info.furthest_x(),
                    area.x,
                    area.width,
                    rect.x + shift_x,
                    rect.width,
                );
                shift_x += info.x - target;
                let _ = self.scroll_area_to(around, target, info.x, true);
            }
        }
    }

    /// Where each element with an `embed` number is, as of the last layout, for what [`crate::embed`] keeps there. It's
    /// empty, and costs nothing, while nothing's kept anywhere.
    pub fn embedded_places(&mut self) -> Vec<(u64, crate::embed::Place)> {
        if !crate::embed::any() {
            return Vec::new();
        }
        self.lay_out();
        crate::embed::places(self.runtime.root(), &self.layout.borrow(), self.scale, self.size)
    }

    /// Reads the tree as text, as in [`rsui_headless::outline`].
    pub fn outline(&self) -> String {
        rsui_headless::outline(self.runtime.root())
    }

    /// Everything that's there, with where it is, one to a line, indented by how deep it is.
    pub fn rectangles(&self) -> String {
        self.rectangles_of(None)
    }

    /// Like [`rectangles`](Self::rectangles), with how many lines each text is drawn on, as `lines=2`, for comparing with how a
    /// browser breaks the same text.
    pub fn rectangles_with_lines(&mut self) -> String {
        let layout = self.layout.borrow();
        write_rectangles(self.runtime.root(), &layout, Some(&mut self.text))
    }

    fn rectangles_of(&self, lines: Option<&mut TextEngine>) -> String {
        write_rectangles(self.runtime.root(), &self.layout.borrow(), lines)
    }
}

/// What [`App::rectangles`] says, with how many lines text is drawn on if there's a text engine to say.
fn write_rectangles(root: &Node, layout: &Layout, mut lines: Option<&mut TextEngine>) -> String {
    fn walk(node: &Node, layout: &Layout, text: &mut Option<&mut TextEngine>, path: &mut Vec<u32>, out: &mut String) {
        if let Some(rect) = layout.rect(path) {
            out.push_str(&" ".repeat(path.len()));
            out.push_str(&format!(
                "{} {},{},{},{}",
                node.role.name(),
                rect.x,
                rect.y,
                rect.width,
                rect.height
            ));
            if !node.name.is_empty() {
                out.push_str(&format!(" {:?}", node.name.as_str()));
            }
            if let Some(text) = text.as_deref_mut().filter(|_| {
                node.role == Role::Text && node.children.is_empty() && rsui_layout::shows_name(node) && !node.name.is_empty()
            }) {
                out.push_str(&format!(" lines={}", text_line_count(text, node, rect)));
            }
            // Which page element it is, for comparing a page with how a browser lays it out. A field's is its own key.
            if let Some(PropValue::Str(key)) = node.props.get("dom_key") {
                out.push_str(&format!(" dom={}", key.as_str()));
            } else if let Some(key) = node
                .key
                .as_ref()
                .map(|key| key.to_string())
                .filter(|key| key.starts_with("__dom") && key != "__dompage")
            {
                out.push_str(&format!(" dom={key}"));
            }
            // What kind of element it is, when the page has asked for that to be said (see `__rsuiDomTags` in web.js).
            if let Some(PropValue::Str(tag)) = node.props.get("dom_tag") {
                out.push_str(&format!(" tag={}", tag.as_str()));
            }
            // How it looks, as far as what's most often checked.
            for name in ["text_color", "background", "font_size", "font_family"] {
                match node.props.get(name) {
                    Some(PropValue::Int(color)) => {
                        out.push_str(&format!(" {name}=#{:08x}", *color as u32));
                    }
                    Some(PropValue::Float(value)) => out.push_str(&format!(" {name}={value}")),
                    Some(PropValue::Str(text)) => {
                        let short =
                            text.as_str().rsplit(['/', '\\']).next().unwrap_or_default();
                        out.push_str(&format!(" {name}={short}"));
                    }
                    _ => {}
                }
            }
            out.push('\n');
        }
        for (index, child) in node.children.iter().enumerate() {
            path.push(index as u32);
            walk(child, layout, text, path, out);
            path.pop();
        }
    }
    let mut out = String::new();
    walk(
        root,
        layout,
        &mut lines,
        &mut Vec::new(),
        &mut out,
    );
    out
}

/// What a pointer at this place (in the layout's own pixels) lands on, as a window has it, and whether that's short of the deepest
/// thing there, as what's `pointer-events: none` passes the pointer to what's around it. `turned` says something may have been
/// moved or turned by a transform, as a Radix popper is `position: fixed` at 0, 0 and put in place by `translate()`, which
/// `elementFromPoint` doesn't know without drawing, so it says so always.
pub(crate) fn pointer_path(root: &Node, layout: &Layout, x: f32, y: f32, turned: bool) -> Option<(Vec<u32>, bool)> {
    let path = raw_pointer_path(root, layout, x, y, turned)?;
    Some(refine_pointer_path(root, layout, path, x, y))
}

fn raw_pointer_path(root: &Node, layout: &Layout, x: f32, y: f32, turned: bool) -> Option<Vec<u32>> {
    let undo = |element: &Element, rect: Rect| {
        rsui_render::transform_of(element, rect.x, rect.y, rect.width, rect.height, 1.0).and_then(rsui_render::invert_matrix)
    };
    {
        #[cfg(feature = "idle-probe")]
        let _native_cycles = crate::native_cycles::enter(crate::native_cycles::RAW_POINTER_PATH);
        if turned || layout.has_stacking() || layout.has_overlays() {
            layout.hit_test_turned(root, x, y, &undo)
        } else {
            layout.hit_test(x, y)
        }
    }
}

fn refine_pointer_path(root: &Node, layout: &Layout, mut path: Vec<u32>, x: f32, y: f32) -> (Vec<u32>, bool) {
    let undo = |element: &Element, rect: Rect| {
        rsui_render::transform_of(element, rect.x, rect.y, rect.width, rect.height, 1.0).and_then(rsui_render::invert_matrix)
    };
    // On a page, what has rounded corners isn't hit outside them, as in a browser: a `rounded-full` button's corners are what's
    // under it. rsui's own boxes are hit all over, as the system's controls are.
    if is_page(root) {
        #[cfg(feature = "idle-probe")]
        let _native_cycles = crate::native_cycles::enter(crate::native_cycles::ROUNDED_CORNERS);
        path.truncate(layout.kept_inside_corners(root, &path, x, y, &undo));
    }
    // What's `pointer-events: none` (inert) isn't what the pointer lands on, nor is what's in it, unless that says it takes the
    // pointer again (`pointer-events: auto`, `inert` false), as CSS has it: it lands on what's around it that does, as a
    // disabled Radix menu item's click lands on its group, and a modal layer's `body { pointer-events: none }` has a click
    // outside it land on the page itself. What's placed by itself was passed over already (see `lifted` in rsui-layout).
    let cut = {
        #[cfg(feature = "idle-probe")]
        let _native_cycles = crate::native_cycles::enter(crate::native_cycles::POINTER_INERT);
        let taking = pointer_takes_up_to(root, &path);
        let cut = taking < path.len();
        path.truncate(taking);
        cut
    };
    (path, cut)
}

/// Whether this window's tree is a page's: its root says so by being a page's (`dom_key` on its scrolling area).
fn is_page(root: &Node) -> bool {
    root.children.first().is_some_and(|child| child.props.get("dom_key").is_some())
}

/// How much of `path` takes the pointer: up to the last node on it that isn't `pointer-events: none` (inert), which what's in an
/// inert node is too, unless it says it takes the pointer itself.
fn pointer_takes_up_to(root: &Node, path: &[u32]) -> usize {
    let mut muted = is_inert(root);
    let mut last = 0;
    let mut node = root;
    for (at, &index) in path.iter().enumerate() {
        let Some(child) = node.children.get(index as usize) else {
            return path.len();
        };
        node = child;
        muted = is_inert(node) || (muted && !takes_pointer(node));
        if !muted {
            last = at + 1;
        }
    }
    last
}

fn node_at<'a>(root: &'a Node, path: &[u32]) -> &'a Node {
    path.iter()
        .fold(root, |node, &index| &node.children[index as usize])
}

/// The node at this path, if there still is one.
fn find<'a>(root: &'a Node, path: &[u32]) -> Option<&'a Node> {
    let mut node = root;
    for &index in path {
        node = node.children.get(index as usize)?;
    }
    Some(node)
}

/// How many boxes the press at `deepest` was inside, below the one at `path`, which it's in. Text is part of
/// what it's in, as the words of a page are, and so is what can't be reached.
fn boxes_between(root: &Node, path: &[u32], deepest: &[u32]) -> i64 {
    if !deepest.starts_with(path) {
        return 0;
    }
    (path.len() + 1..=deepest.len())
        .filter(|&depth| {
            find(root, &deepest[..depth])
                .is_some_and(|node| node.role != Role::Text && !is_inert(node))
        })
        .count() as i64
}

/// Whether clicking this node does anything.
fn activates(node: &Element) -> bool {
    (node.actions.contains(ActionKind::Click) || node.actions.contains(ActionKind::Toggle))
        && !is_disabled(node)
}

/// Whether this can't be used at the moment.
/// Whether this is a field that can have the keyboard, be selected in and copied from, but not be changed.
fn is_read_only(node: &Element) -> bool {
    node.props.get_symbol(symbol!("read_only")) == Some(&PropValue::Bool(true))
}

fn is_disabled(node: &Element) -> bool {
    !node.props.is_empty()
        && node.props.get_symbol(symbol!("disabled")) == Some(&PropValue::Bool(true))
}

/// As much of what's being put in an input as there's room for, if it has a `max_length`, in characters.
fn as_much_as_fits<'t>(node: &Element, value: &str, edit: &TextEdit, text: &'t str) -> &'t str {
    let most = match node.props.get_symbol(symbol!("max_length")) {
        Some(PropValue::Int(most)) => *most as usize,
        Some(PropValue::Float(most)) => *most as usize,
        _ => return text,
    };
    let (start, end) = edit.selection();
    let staying = value[..start].chars().count() + value[end..].chars().count();
    let room = most.saturating_sub(staying);
    text.char_indices()
        .nth(room)
        .map_or(text, |(byte, _)| &text[..byte])
}

/// Whether this text input has several lines.
fn is_multiline(node: &Element) -> bool {
    node.props.get("rows").is_some()
}

/// Which line a byte is on, among lines starting at `starts`.
fn line_of(starts: &[usize], at: usize) -> usize {
    starts
        .partition_point(|start| *start <= at)
        .saturating_sub(1)
}

/// Which line a byte is on, and how far across it a caret there goes.
fn place_in_lines(
    text: &mut TextEngine,
    value: &str,
    style: TextStyle,
    width: f32,
    starts: &[usize],
    at: usize,
    upstream: bool,
) -> (usize, f32) {
    let line = line_of(starts, at);
    // At the end of the line before, where that wraps, rather than at the start of this one.
    let line = if upstream && line > 0 && starts[line] == at && !value[..at].ends_with('\n') {
        line - 1
    } else {
        line
    };
    let piece = wrapped_line(value, starts, line);
    let along = (at - starts[line]).min(piece.len());
    let along = if piece.is_char_boundary(along) {
        along
    } else {
        piece.len()
    };
    let indent = text.line_indent(piece, style, width);
    (line, indent + text.caret_x(piece, style, along))
}

/// The byte nearest this far across a line.
fn byte_along_line(
    text: &mut TextEngine,
    value: &str,
    style: TextStyle,
    width: f32,
    starts: &[usize],
    line: usize,
    x: f32,
) -> usize {
    let piece = wrapped_line(value, starts, line);
    let indent = text.line_indent(piece, style, width);
    starts[line] + text.byte_at_x(piece, style, x - indent)
}

/// Whether this has the keyboard to itself while it's there.
fn takes_keyboard(node: &Element) -> bool {
    matches!(node.role, Role::Dialog | Role::Menu) && !is_inert(node)
}

/// The text explaining this, if there is any.
fn tooltip_of(node: &Element) -> Option<&str> {
    match node.props.get_symbol(rsui_core::symbol!("tooltip")) {
        Some(PropValue::Str(text)) => Some(text.as_str()),
        _ => None,
    }
}

/// Whether this node can have the keyboard. The list of what a person can reach is the same as
/// the list of what they can do.
fn focusable(node: &Element) -> bool {
    // What says it can't have the keyboard, as a page's element that can be clicked but isn't a field, a button, a link or
    // something with a `tabIndex`, as a `<label>` is, can't.
    if is_disabled(node) {
        return false;
    }
    // What it is is asked first: whether something that scrolls can have the keyboard because it does looks through everything
    // in it, which for a list of 25,000 rows with a `tabIndex` of its own was most of what moving its selection cost.
    let says_not = node.props.get_symbol(symbol!("focusable")) == Some(&PropValue::Bool(false));
    if !says_not && focusable_by_itself(node) {
        return true;
    }
    // That's about what it is, not about it scrolling: a page's `<div>` that scrolls and has nothing in it that can have the
    // keyboard can, as a browser's keyboard-focusable scrollers are, even with an `onFocus` that makes it say it can't.
    node.actions.contains(ActionKind::Scroll) && scroller_takes_focus(node)
}

/// Whether something that scrolls can have the keyboard because it does: one of rsui's own can, and a page's only when
/// nothing in it can, as a browser has it, and never the page's own, which is the document's.
fn scroller_takes_focus(node: &Element) -> bool {
    match page_key(node) {
        None => true,
        Some("__dompage") => false,
        Some(_) => !node.children.iter().any(|child| holds_focusable(child)),
    }
}

/// Whether this, or anything in it, can have the keyboard by itself.
fn holds_focusable(node: &Element) -> bool {
    !is_inert(node) && ((focusable_by_itself(node) && !is_disabled(node)) || node.children.iter().any(|child| holds_focusable(child)))
}

/// Whether this can have the keyboard for what it is, rather than only because it scrolls. A button can, whether or not
/// anything listens to it, as a page's `<button>` can. An element of a page (one with a `dom_key`) can by its kind, which its
/// page says by listening for the keyboard, rather than by being clicked: a `<label>` or a `<div>` with a click handler can't.
fn focusable_by_itself(node: &Element) -> bool {
    node.role == Role::TextInput
        || node.role == Role::Button
        || (activates(node) && page_key(node).is_none())
        || node.actions.contains(ActionKind::Focus)
        || tab_index(node).is_some()
}

/// Where the element of a page with this key is, if it's there.
fn path_of_page_key(root: &Node, key: &Key) -> Option<Vec<u32>> {
    fn walk(node: &Node, wanted: &str, path: &mut Vec<u32>) -> bool {
        if page_key(node) == Some(wanted) {
            return true;
        }
        for (index, child) in node.children.iter().enumerate() {
            path.push(index as u32);
            if walk(child, wanted, path) {
                return true;
            }
            path.pop();
        }
        false
    }
    let wanted = match key {
        Key::Str(text) => text.as_str(),
        Key::Sym(symbol) => symbol.as_str(),
        Key::Int(_) => return None,
    };
    let mut path = Vec::new();
    walk(root, wanted, &mut path).then_some(path)
}

/// An element of a page's key there, as `rsui/web` gives it: its `dom_key`, or its own key when that's one of a page's.
pub(crate) fn page_key(node: &Element) -> Option<&str> {
    match (node.props.get("dom_key"), &node.key) {
        (Some(PropValue::Str(key)), _) => Some(key.as_str()),
        (None, Some(Key::Str(key))) if key.starts_with("__dom") => Some(key.as_str()),
        _ => None,
    }
}

/// Where tab goes to this in its order, as a page's `tabIndex` says: before everything else if it's more than 0, lowest
/// first, with what doesn't say in the order it comes, as 0 is, and never if it's less than 0, though it can still have
/// the keyboard by being clicked or asked for.
fn tab_index(node: &Element) -> Option<i64> {
    match node.props.get_symbol(symbol!("tab_index")) {
        Some(PropValue::Int(index)) => Some(*index),
        Some(PropValue::Float(index)) => Some(*index as i64),
        Some(PropValue::Str(text)) => text.trim().parse().ok(),
        _ => None,
    }
}

/// How far a key moves something that scrolls, if it does, and whether that has to be across.
fn step_for_key(code: KeyCode, node: &Element) -> Option<(ScrollStep, bool)> {
    let vertical = is_vertical(node);
    let across = node.props.get("scroll_x").is_some();
    let up_and_down = vertical && matches!(code, KeyCode::Up | KeyCode::Down);
    let left_and_right = across && matches!(code, KeyCode::Left | KeyCode::Right);
    Some(match code {
        KeyCode::Up if up_and_down => (ScrollStep::Arrow(-1), false),
        KeyCode::Down if up_and_down => (ScrollStep::Arrow(1), false),
        KeyCode::Left if left_and_right => (ScrollStep::Arrow(-1), true),
        KeyCode::Right if left_and_right => (ScrollStep::Arrow(1), true),
        KeyCode::PageUp => (ScrollStep::Page(-1), false),
        KeyCode::PageDown => (ScrollStep::Page(1), false),
        // A web page's Space goes down a page, as a browser's does when what has the keyboard doesn't take it.
        KeyCode::Space if is_web(node) => (ScrollStep::Page(1), false),
        KeyCode::Home => (ScrollStep::Start, false),
        KeyCode::End => (ScrollStep::End, false),
        _ => return None,
    })
}

/// The path to the first web page's scrolling area, outermost first, if there is one: where a page's keys go with nothing on the
/// keyboard.
fn web_page_scroller(node: &Element, path: &mut Vec<u32>) -> bool {
    if node.actions.contains(ActionKind::Scroll) && is_web(node) {
        return true;
    }
    // It's near the top, under the window and what holds it, so the rest of a big tree isn't gone through for each key.
    if path.len() >= 4 {
        return false;
    }
    for (index, child) in node.children.iter().enumerate() {
        path.push(index as u32);
        if web_page_scroller(child, path) {
            return true;
        }
        path.pop();
    }
    false
}

/// Whether something that scrolls is a web page's, which scrolls by its keys as a browser does: the page itself or an element of it.
fn is_web(node: &Element) -> bool {
    node.props.get("dom_key").is_some()
}

fn is_list(node: &Element) -> bool {
    node.props.get("row_height").is_some()
}

/// Whether something that scrolls goes up and down, which is what it mainly does if it goes both ways.
fn is_vertical(node: &Element) -> bool {
    node.props.get("scroll_y").is_some() || node.props.get("scroll_x").is_none()
}

/// What to tell something that scrolls where to go. Only something that goes both ways has more than
/// one way of being told.
fn scroll_action(node: &Element, along_x: bool) -> ActionKind {
    if along_x && node.props.get("scroll_y").is_some() {
        ActionKind::Custom(symbol!("scroll_across"))
    } else {
        ActionKind::Scroll
    }
}

fn first_visible(list: &Element) -> i64 {
    list_position(list) as i64
}

/// Which row is at the top of a list, with part of one if it's partway through gliding.
fn list_position(list: &Element) -> f32 {
    match list.props.get("first_visible") {
        Some(PropValue::Int(first)) => *first as f32,
        Some(PropValue::Float(first)) => *first as f32,
        _ => 0.0,
    }
}

/// Where to scroll to, in one direction, for something to show, moving as little as it can. If it
/// doesn't fit, its start is what shows. Everything is in layout pixels.
fn reveal_along(
    moved: f32,
    furthest: f32,
    area_start: f32,
    area_length: f32,
    start: f32,
    length: f32,
) -> f32 {
    let room = (area_length - REVEAL_MARGIN * 2.0).max(0.0);
    let before = area_start + REVEAL_MARGIN;
    let target = if start < before || length > room {
        moved + start - before
    } else if start + length > before + room {
        moved + start + length - before - room
    } else {
        moved
    };
    target.clamp(0.0, furthest)
}

/// Where to scroll to, in one direction, for something to show: nowhere if it's all in view already, and otherwise with its
/// middle in the middle, as Chromium has it for what gets the keyboard. Everything is in layout pixels.
fn center_along(
    moved: f32,
    furthest: f32,
    area_start: f32,
    area_length: f32,
    start: f32,
    length: f32,
) -> f32 {
    if start >= area_start && start + length <= area_start + area_length {
        return moved;
    }
    (moved + start + length / 2.0 - area_start - area_length / 2.0).clamp(0.0, furthest)
}

/// Finds where in the tree these elements are, which is only worth walking it for because this
/// only happens when a list's size has changed.
fn find_lists(
    node: &Node,
    sizes: &[(usize, f32, f32)],
    path: &mut Vec<u32>,
    found: &mut Vec<(Vec<u32>, f32, f32)>,
) {
    let address = std::ptr::from_ref::<Element>(node) as usize;
    if let Some((_, rows, across)) = sizes.iter().find(|(list, _, _)| *list == address) {
        found.push((path.clone(), *rows, *across));
    }
    for (index, child) in node.children.iter().enumerate() {
        path.push(index as u32);
        find_lists(child, sizes, path, found);
        path.pop();
    }
}

/// What tab goes through, in the order it goes, inside `node`, which is at `path`.
fn collect_focusable(node: &Node, path: &mut Vec<u32>, stops: &mut Vec<Vec<u32>>) {
    let mut found = Stops { stops: std::mem::take(stops), first: Vec::new(), portals: Vec::new() };
    gather_focusable(node, path, &mut found);
    // What's in a portal into a page's `body` is at the end of it, in the order they opened, as on a page: see `portal_order`.
    let mut at = 0;
    while at < found.portals.len() {
        found.portals[at..].sort_by(|a, b| a.0.total_cmp(&b.0));
        let (_, mut inside) = found.portals[at].clone();
        let start = inside.len();
        let portal = node_at_from(node, &inside[path.len()..]);
        for (index, child) in portal.children.iter().enumerate() {
            inside.push(index as u32);
            gather_focusable(child, &mut inside, &mut found);
            inside.truncate(start);
        }
        at += 1;
    }
    let Stops { stops: gathered, mut first, .. } = found;
    *stops = gathered;
    if !first.is_empty() {
        // Lowest first, and those that say the same in the order they come.
        first.sort_by_key(|(index, _)| *index);
        let rest = std::mem::take(stops);
        stops.extend(first.into_iter().map(|(_, path)| path));
        stops.extend(rest);
    }
}

/// What's at `path` below `node`.
fn node_at_from<'a>(node: &'a Node, path: &[u32]) -> &'a Node {
    let mut at = node;
    for &index in path {
        match at.children.get(index as usize) {
            Some(child) => at = child,
            None => break,
        }
    }
    at
}

/// The stops `gather_focusable` finds: in the order they come, those with a `tabIndex` over 0 to go first, and the portals into a
/// page's `body` it went past, to go through after everything else.
struct Stops {
    stops: Vec<Vec<u32>>,
    first: Vec<(i64, Vec<u32>)>,
    portals: Vec<(f64, Vec<u32>)>,
}

/// What's inert (`pointer-events: none`) isn't gone through, but what's in it that says it takes the pointer again is, as a modal
/// Radix dialog's content (`pointer-events: auto`) in a `body` that's `pointer-events: none` while it's open. With a stack of its
/// own rather than the thread's, as a tree can be deeper than that has room for (see edge.rs).
fn gather_focusable(node: &Node, path: &mut Vec<u32>, found: &mut Stops) {
    // Each is a node, how long `path` is above it, its index there (none for `node` itself), and whether what it's in is inert.
    let base = path.len();
    let mut stack: Vec<(&Node, usize, Option<u32>, bool)> = vec![(node, base, None, false)];
    while let Some((node, depth, index, above)) = stack.pop() {
        path.truncate(depth);
        if let Some(index) = index {
            path.push(index);
        }
        let muted = is_inert(node) || (above && !takes_pointer(node));
        if portal_passed(node, path, found) {
            continue;
        }
        if !muted && focusable(node) && !page_scroller_passed_over(node) {
            match tab_index(node) {
                Some(index) if index < 0 => {}
                Some(index) if index > 0 => found.first.push((index, path.clone())),
                _ => found.stops.push(path.clone()),
            }
        }
        let here = path.len();
        for (index, child) in node.children.iter().enumerate().rev() {
            stack.push((child, here, Some(index as u32), muted));
        }
    }
    path.truncate(base);
}

/// Whether this is a portal into a page's `body` (see `portal_order`), which is noted to be gone through last. Apart from
/// `gather_focusable`, so as to keep that small.
#[inline(never)]
fn portal_passed(node: &Node, path: &[u32], found: &mut Stops) -> bool {
    let order = match node.props.get_symbol(symbol!("portal_order")) {
        Some(PropValue::Int(order)) => *order as f64,
        Some(PropValue::Float(order)) => *order,
        _ => return false,
    };
    found.portals.push((order, path.to_vec()));
    true
}

/// Whether this is something of a page that scrolls, and is no stop of its own as something in it can have the keyboard, as
/// Chromium has it, so a page's scrolling doesn't come before its first field. An app's own scrolling areas are stops still.
/// Apart from `gather_focusable`, which goes as deep as the tree does, so as to keep that small.
#[inline(never)]
fn page_scroller_passed_over(node: &Node) -> bool {
    node.actions.contains(ActionKind::Scroll)
        && node.props.get("dom_key").is_some()
        && node.role != Role::TextInput
        && !activates(node)
        && !node.actions.contains(ActionKind::Focus)
        && tab_index(node).is_none()
        && node.children.iter().any(holds_a_stop)
}

/// Whether Tab could stop at this or anything in it.
fn holds_a_stop(node: &Node) -> bool {
    // A stack of its own rather than the thread's, as a tree can be deeper than that has room for.
    let mut waiting = vec![node];
    while let Some(node) = waiting.pop() {
        if is_inert(node) {
            continue;
        }
        if focusable(node) && tab_index(node).is_none_or(|index| index >= 0) {
            return true;
        }
        waiting.extend(node.children.iter());
    }
    false
}

/// Text fit for a single line: line breaks become spaces.
fn one_line(text: &str) -> String {
    text.replace("\r\n", " ")
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn hovering_name() -> Symbol {
    symbol!("hover_change")
}

fn pressing_name() -> Symbol {
    symbol!("press_change")
}

fn focus_within_name() -> Symbol {
    symbol!("focus_within")
}

fn cursor_name() -> Symbol {
    symbol!("cursor")
}

/// Only a fully classified transaction can preserve interaction state. Never
/// resolve intermediate structural patch paths against the final tree here.
fn preserves_interactions(patches: &[Patch]) -> bool {
    patches.iter().all(|patch| match patch {
        Patch::SetProp { name, .. } | Patch::RemoveProp { name, .. } => {
            matches!(name.as_str(), "background" | "pressed_background")
        }
        _ => false,
    })
}

thread_local! {
    /// What an app on this thread does as well when things have gone quiet (see [`on_quiet`]).
    static QUIET_HOOK: std::cell::Cell<Option<fn(Quiet) -> bool>> = const { std::cell::Cell::new(None) };
}

/// How quiet things have gone, for [`on_quiet`]'s hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quiet {
    /// A frame is up, and nothing else has come since: no input, and nothing moving. Someone using the app is usually
    /// between two things they do, so a little work done now isn't in the way of either, as it would be in the next.
    Frame,
    /// Nothing has happened for a few seconds: in a window, at least five since the last frame (and a caret stops blinking
    /// ten seconds after the last key), after it's let go of its caches. Coming back then costs a moment anyway, as the
    /// window has given its memory back to the system and has to get it again.
    Moment,
    /// A window is letting go of the GPU: nothing has happened for half a minute (ten seconds when nobody seems to be using
    /// it), or it's been minimized for ten seconds. Getting the GPU back takes a tenth of a second, which starts as the
    /// pointer comes in, so work done now is the least likely to be in someone's way.
    Long,
}

/// Has `hook` run whenever an app on this thread has gone quiet, as rsui-tsx does to collect its JavaScript's garbage then.
/// For [`Quiet::Moment`] it runs before the app lets go of its caches. It says whether it gave up enough
/// that a window should give back all of the allocator's empty chunks, rather than its usual few.
pub fn on_quiet(hook: fn(Quiet) -> bool) {
    QUIET_HOOK.with(|quiet| quiet.set(Some(hook)));
}

/// Runs [`on_quiet`]'s hook, if there is one, and says what it did.
pub(crate) fn run_quiet_hook(quiet: Quiet) -> bool {
    QUIET_HOOK
        .with(std::cell::Cell::get)
        .is_some_and(|hook| hook(quiet))
}
