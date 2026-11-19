//! What came in to an app from outside, kept so that it can be given to the app again, as a replay does.
//!
//! A window gives everything that comes in to its app through [`App::feed`], which keeps it once
//! [`App::record_inputs`] has been asked for, with when it came by [`now_ms`]. What the app does is all from what comes
//! in, the clock, and what it reads from elsewhere, so an app given the same inputs at the same times, with the same
//! clock, does the same, apart from what it reads: a file, the network, or what another thread did.

use std::sync::OnceLock;

use rsui_runtime::DispatchError;

use crate::{App, DropPhase, KeyPress, Modifiers, PointerButton};

/// The most inputs kept, which is hours of use. Past this, nothing more's kept, and a replay only goes this far.
const MOST_KEPT: usize = 400_000;

/// Something that came in to an app, as a window gives it.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Move { x: f32, y: f32 },
    Leave,
    Down { x: f32, y: f32, clicks: u32 },
    Up { x: f32, y: f32 },
    ButtonDown { x: f32, y: f32, button: PointerButton },
    ButtonUp { x: f32, y: f32, button: PointerButton },
    CancelPress,
    Active(bool),
    /// The window was minimized (false) or brought back from that (true).
    Shown(bool),
    Modifiers(Modifiers),
    Key(KeyPress),
    KeyUp(KeyPress),
    Text(String),
    Composition { phase: String, text: String },
    Scroll { x: f32, y: f32, rows: i64, across: bool },
    /// Wheel movement in logical pixels. Precise devices track input directly;
    /// discrete notches may use the app's smooth-scroll animation.
    ScrollPixels { x: f32, y: f32, pixels: f32, across: bool, precise: bool },
    Resize { width: u32, height: u32, scale: f32 },
    Files { x: f32, y: f32, paths: Vec<String> },
    DragFromOutside { phase: DropPhase, x: f32, y: f32, files: Vec<String>, text: Option<String> },
    DarkMode(bool),
    /// What assistive technology asked of something, as a screen reader does.
    Access { path: Vec<u32>, command: crate::access::AccessCommand },
    Close,
}

/// An input, and when it came, in milliseconds since 1970 by [`now_ms`].
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded {
    pub at: f64,
    pub input: Input,
}

static CLOCK: OnceLock<fn() -> f64> = OnceLock::new();

/// Has inputs kept with the time `clock` says, rather than the system's, as the clock of the code an app runs, which
/// a replay sets. It can only be said once.
pub fn set_clock(clock: fn() -> f64) {
    let _ = CLOCK.set(clock);
}

/// The time now, in milliseconds since 1970, by the clock [`set_clock`] said, or else the system's.
pub fn now_ms() -> f64 {
    match CLOCK.get() {
        Some(clock) => clock(),
        None => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |since| since.as_secs_f64() * 1000.0),
    }
}

/// What's kept of what came in.
#[derive(Default)]
pub(crate) struct Recording {
    inputs: Vec<Recorded>,
    full: bool,
    modifiers: Modifiers,
}

impl Input {
    /// What it's called in a trace, as `buttonup`.
    pub fn kind(&self) -> &'static str {
        match self {
            Input::Move { .. } => "move",
            Input::Leave => "leave",
            Input::Down { .. } => "down",
            Input::Up { .. } => "up",
            Input::ButtonDown { .. } => "buttondown",
            Input::ButtonUp { .. } => "buttonup",
            Input::CancelPress => "cancel",
            Input::Active(_) => "active",
            Input::Shown(_) => "shown",
            Input::Modifiers(_) => "held",
            Input::Key(_) => "key",
            Input::KeyUp(_) => "keyup",
            Input::Text(_) => "text",
            Input::Composition { .. } => "composition",
            Input::Scroll { .. } => "scroll",
            Input::ScrollPixels { .. } => "scrollpixels",
            Input::Resize { .. } => "resize",
            Input::Files { .. } => "files",
            Input::DragFromOutside { .. } => "drag",
            Input::DarkMode(_) => "dark",
            Input::Access { .. } => "access",
            Input::Close => "close",
        }
    }
}

impl App {
    /// Starts keeping what comes in through [`App::feed`], from now, with nothing kept before.
    pub fn record_inputs(&mut self) {
        self.recording = Some(Box::default());
    }

    /// What's been kept of what came in, and whether more came than could be kept.
    pub fn recorded_inputs(&self) -> (&[Recorded], bool) {
        self.recording
            .as_ref()
            .map_or((&[][..], false), |recording| (&recording.inputs[..], recording.full))
    }

    /// Has the app take in something from outside, and keeps it if it's keeping what comes in. For [`Input::Close`],
    /// it says whether the app wants to stay open; for anything else it's true. What's typed into a password is kept as
    /// `*`s, so a trace never has one in it.
    pub fn feed(&mut self, input: &Input) -> Result<bool, DispatchError> {
        let _logged = crate::input_log::Span::new("phase-feed");
        if self.pill_took(input) {
            return Ok(true);
        }
        let _split = rsui_alloc::split::enter(rsui_alloc::split::INPUT);
        let _span = rsui_spans::span(rsui_spans::Kind::Input).map(|mut span| {
            span.label(input.kind());
            span
        });
        let masked;
        let kept_input = if self.recording.is_some()
            && matches!(input, Input::Key(_) | Input::KeyUp(_) | Input::Text(_) | Input::Composition { .. })
            && self.typing_a_password()
        {
            masked = masked_input(input);
            &masked
        } else {
            input
        };
        if let Some(recording) = &mut self.recording {
            // What's held is only kept when it changes, as it's said with each move.
            let kept = match input {
                Input::Modifiers(held) => {
                    let changed = *held != recording.modifiers;
                    recording.modifiers = *held;
                    changed
                }
                _ => true,
            };
            if kept {
                if recording.inputs.len() < MOST_KEPT {
                    recording.inputs.push(Recorded {
                        at: now_ms(),
                        input: kept_input.clone(),
                    });
                } else {
                    recording.full = true;
                }
            }
        }
        match input {
            Input::Move { x, y } => self.pointer_move(*x, *y),
            Input::Leave => self.pointer_leave(),
            Input::Down { x, y, clicks } => self.pointer_down(*x, *y, *clicks),
            Input::Up { x, y } => self.pointer_up(*x, *y)?,
            Input::ButtonDown { x, y, button } => self.button_down(*x, *y, *button),
            Input::ButtonUp { x, y, button } => self.button_up(*x, *y, *button)?,
            Input::CancelPress => self.cancel_press(),
            Input::Active(active) => self.set_active(*active),
            Input::Shown(shown) => self.set_shown(*shown),
            Input::Modifiers(held) => self.set_modifiers(*held),
            Input::Key(press) => self.key(*press)?,
            Input::KeyUp(press) => self.key_up(*press)?,
            Input::Text(text) => self.type_text(text)?,
            Input::Composition { phase, text } => self.composition(phase, text)?,
            Input::Scroll { x, y, rows, across } => {
                if *across {
                    self.scroll_across(*x, *y, *rows)?;
                } else {
                    self.scroll(*x, *y, *rows)?;
                }
            }
            Input::ScrollPixels { x, y, pixels, across, precise } => {
                self.scroll_pixels(*x, *y, *pixels, *across, *precise)?;
            }
            Input::Resize { width, height, scale } => self.resize((*width, *height), *scale),
            Input::Files { x, y, paths } => self.files_dropped(*x, *y, paths)?,
            Input::DragFromOutside { phase, x, y, files, text } => {
                return Ok(self.drag_from_outside(*phase, *x, *y, files, text.as_deref()));
            }
            Input::DarkMode(dark) => self.set_dark_mode(*dark),
            Input::Access { path, command } => self.access_command(path, command.clone())?,
            Input::Close => return self.asked_to_close(),
        }
        Ok(true)
    }
}
/// What's typed, as it's kept while a password has the keyboard: each character as `*`.
fn masked_input(input: &Input) -> Input {
    let stars = |text: &str| "*".repeat(text.chars().count());
    let press = |press: &KeyPress| {
        let mut press = *press;
        if let crate::KeyCode::Character(_) = press.code {
            press.code = crate::KeyCode::Character('*');
        }
        press.typed = press.typed.map(|_| '*');
        press.scan = 0;
        press
    };
    match input {
        Input::Key(one) => Input::Key(press(one)),
        Input::KeyUp(one) => Input::KeyUp(press(one)),
        Input::Text(text) => Input::Text(stars(text)),
        Input::Composition { phase, text } => Input::Composition {
            phase: phase.clone(),
            text: stars(text),
        },
        other => other.clone(),
    }
}
