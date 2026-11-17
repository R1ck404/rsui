//! What code written for a page hears about, as the web has it: presses, moves, the wheel and keys, which go from the root
//! down to what they happened to and back up, to whatever offers to hear each on the way, and to the app as a whole, as
//! a page's `document` does. Nothing here runs unless something's offered to hear it, so an app that doesn't pays nothing.

use std::time::{Duration, Instant};

use rsui_core::{ActionKind, Key, Node, PropValue, symbol};
use rsui_runtime::{DispatchError, Event, KeyInfo, Modifiers, document_listens, listens};

use super::{App, find};
use crate::input::{KeyPress, PointerButton};

/// Something that goes down to what it happened to and back up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    Down,
    Up,
    Move,
    /// The mouse's `mousedown`, `mouseup` and `mousemove` that go with a pointer's, each sent by itself after the pointer's
    /// has been all the way down and back up, as a browser has them (see [`App::fire`]).
    MouseDown,
    MouseUp,
    MouseMove,
    Wheel,
    KeyDown,
    KeyUp,
    /// `paste`, `copy` or `cut`, which its input says.
    Clipboard,
    /// `start`, `update` or `end` of something being put together by a way of typing, which its input says.
    Composition,
    /// A click with a button other than the main one, as the web's `auxclick` is.
    AuxClick,
    /// The second click of a double click, after it, as the web's `dblclick` is.
    DoubleClick,
    /// A click, on the way down, as `onClickCapture` hears it. On the way up it's the click itself, which goes as
    /// [`App::activate_at`] has it.
    Click,
    /// Something being dragged, as the web's `dragstart`, `dragenter`, `dragover`, `dragleave`, `drop` and `dragend` are,
    /// which its input says.
    Drag,
    /// Text typed while what has the keyboard isn't a field, for an element a page can edit, as `rsuitext` with the text as its
    /// `data`.
    Text,
    /// What's selected in the field that has the keyboard changed to something, which a page hears as `select`.
    Select,
}

/// What the web calls each part of dragging.
const DRAG_NAMES: [&str; 7] = [
    "dragstart",
    "drag",
    "dragenter",
    "dragover",
    "dragleave",
    "drop",
    "dragend",
];

/// Something being dragged, from inside the app or from another program.
pub(crate) struct DragSession {
    /// What's being dragged, which is nothing for what comes from another program.
    source: Option<Vec<u32>>,
    /// What it's over, as was last said.
    over: Option<Vec<u32>>,
    /// Whether what it's over said it can be dropped there.
    accepted: bool,
    /// What's being dragged in from another program.
    files: Vec<rsui_core::Text>,
    text: Option<rsui_core::Text>,
}

/// Where a drag from another program has got to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropPhase {
    /// It's come over the window.
    Enter,
    /// It's moved, over the window.
    Over,
    /// It's gone, without being dropped.
    Leave,
    Drop,
}

/// How far the pointer has to go with the button down on something that can be dragged before it is, in logical pixels.
const DRAG_START: f32 = 4.0;

impl Kind {
    /// What hears it on the way up, and what hears it on the way down.
    fn actions(self) -> (Option<ActionKind>, ActionKind) {
        let (up, down) = match self {
            Self::Down => (symbol!("pointer_down"), symbol!("pointer_down_capture")),
            Self::Up => (symbol!("pointer_up"), symbol!("pointer_up_capture")),
            Self::Move => (symbol!("pointer_move"), symbol!("pointer_move_capture")),
            Self::MouseDown => (symbol!("mouse_down"), symbol!("mouse_down_capture")),
            Self::MouseUp => (symbol!("mouse_up"), symbol!("mouse_up_capture")),
            Self::MouseMove => (symbol!("mouse_move"), symbol!("mouse_move_capture")),
            Self::Wheel => (symbol!("wheel"), symbol!("wheel_capture")),
            Self::KeyDown => (symbol!("key_down"), symbol!("key_down_capture")),
            Self::KeyUp => (symbol!("key_up"), symbol!("key_up_capture")),
            Self::Clipboard => (symbol!("clipboard"), symbol!("clipboard_capture")),
            Self::Composition => (symbol!("composition"), symbol!("composition_capture")),
            Self::AuxClick => (symbol!("aux_click"), symbol!("aux_click_capture")),
            Self::DoubleClick => (symbol!("double_click"), symbol!("double_click_capture")),
            Self::Click => (symbol!("click"), symbol!("click_capture")),
            Self::Drag => (symbol!("web_drag"), symbol!("web_drag_capture")),
            Self::Text => (symbol!("text_typed"), symbol!("text_typed_capture")),
            Self::Select => (symbol!("field_selected"), symbol!("field_selected_capture")),
        };
        let up = (self != Self::Click).then_some(ActionKind::Custom(up));
        (up, ActionKind::Custom(down))
    }

    /// Which of [`listens`] it is, for the app as a whole.
    fn listens(self) -> u32 {
        match self {
            Self::Down | Self::MouseDown => listens::DOWN,
            Self::Up | Self::MouseUp => listens::UP,
            Self::Move | Self::MouseMove => listens::MOVE,
            Self::Wheel => listens::WHEEL,
            Self::KeyDown => listens::KEY_DOWN,
            Self::KeyUp => listens::KEY_UP,
            Self::Clipboard => listens::CLIPBOARD,
            Self::Composition => listens::COMPOSITION,
            Self::AuxClick | Self::Click => listens::CLICK,
            Self::DoubleClick => listens::DOUBLE_CLICK,
            Self::Drag => listens::DRAG,
            Self::Text => listens::TEXT,
            Self::Select => listens::SELECT,
        }
    }

    /// The mouse's event that goes with a pointer's, as `mousedown` goes with `pointerdown`.
    fn mouse(self) -> Option<Self> {
        match self {
            Self::Down => Some(Self::MouseDown),
            Self::Up => Some(Self::MouseUp),
            Self::Move => Some(Self::MouseMove),
            _ => None,
        }
    }

    /// What the web calls it, given what came with it.
    fn web_name(self, input: Option<&PropValue>) -> &'static str {
        let said = match input {
            Some(PropValue::Str(text)) => text.as_str(),
            _ => "",
        };
        match self {
            Self::Down => "pointerdown",
            Self::Up => "pointerup",
            Self::Move => "pointermove",
            Self::MouseDown => "mousedown",
            Self::MouseUp => "mouseup",
            Self::MouseMove => "mousemove",
            Self::Wheel => "wheel",
            Self::KeyDown => "keydown",
            Self::KeyUp => "keyup",
            Self::Clipboard => match said {
                "copy" => "copy",
                "cut" => "cut",
                _ => "paste",
            },
            Self::Composition => match said {
                "start" => "compositionstart",
                "end" => "compositionend",
                _ => "compositionupdate",
            },
            Self::AuxClick => "auxclick",
            Self::DoubleClick => "dblclick",
            Self::Click => "click",
            Self::Text => "rsuitext",
            Self::Select => "rsuiselect",
            Self::Drag => DRAG_NAMES
                .iter()
                .find(|name| **name == said)
                .copied()
                .unwrap_or("dragover"),
        }
    }
}

/// How something that went down and back up turned out.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Fired {
    /// Something said nothing further along should hear about it.
    pub(crate) stopped: bool,
    /// Something said what it would do by itself shouldn't happen.
    pub(crate) prevented: bool,
}

/// What's being held down with the main button, which hears the pointer move until it's let go of, wherever the pointer
/// goes, as a slider's thumb wants: where it is, and the keys of what's on the way there, to find it again if the tree
/// changes around it.
pub(crate) struct Captured {
    pub(crate) path: Vec<u32>,
    keys: Vec<Option<Key>>,
}

/// Where what has these keys is now, if it's still there.
fn follow(root: &Node, path: &[u32], keys: &[Option<Key>]) -> Option<Vec<u32>> {
    let mut node = root;
    let mut found = Vec::with_capacity(path.len());
    for (&step, key) in path.iter().zip(keys) {
        let at = match node.children.get(step as usize) {
            Some(child) if child.key == *key => step as usize,
            _ => {
                let key = key.as_ref()?;
                node.children
                    .iter()
                    .position(|child| child.key.as_ref() == Some(key))?
            }
        };
        node = &node.children[at];
        found.push(at as u32);
    }
    Some(found)
}

/// Where what was at `path`, with `keys` along it, is now, as far down as it's still there, and whether all of it is: what's put in
/// or taken out before something moves it along, which isn't it going anywhere. What has no key is taken to be what's in the same
/// place, as before.
pub(crate) fn follow_along(root: &Node, path: &[u32], keys: &[Option<Key>]) -> (Vec<u32>, bool) {
    let mut node = root;
    let mut found = Vec::with_capacity(path.len());
    if keys.len() != path.len() {
        return (path.to_vec(), false);
    }
    for (&step, key) in path.iter().zip(keys) {
        let at = match node.children.get(step as usize) {
            Some(child) if child.key == *key => Some(step as usize),
            _ => key
                .as_ref()
                .and_then(|key| node.children.iter().position(|child| child.key.as_ref() == Some(key))),
        };
        let Some(at) = at else {
            return (found, false);
        };
        node = &node.children[at];
        found.push(at as u32);
    }
    (found, true)
}

/// The keys along `path`, as far as it goes: a path into a paragraph's words can go past what's in the tree, which has fewer keys
/// than steps, and isn't followed (see `follow_along`).
pub(crate) fn keys_along(root: &Node, path: &[u32]) -> Vec<Option<Key>> {
    let mut node = root;
    let mut keys = Vec::with_capacity(path.len());
    for &step in path {
        let Some(child) = node.children.get(step as usize) else {
            break;
        };
        node = child;
        keys.push(node.key.clone());
    }
    keys
}

impl App {
    /// What's held on the keyboard, which goes along with whatever the pointer does next. A window says this as each thing
    /// happens. With no window, it's whatever was said last.
    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    /// Where the window's top left is on the screen, in physical pixels, for what wants to know where the pointer is there.
    pub fn set_screen_origin(&mut self, x: f32, y: f32) {
        self.screen_origin = (x, y);
    }

    /// How often at most code that hears the pointer move is told, which a window sets to how often it shows a frame. With
    /// no window it's told of every move.
    pub fn set_move_pace(&mut self, pace: Duration) {
        self.move_pace = pace;
    }

    /// What goes with something happening at this position, in physical pixels, to what's at `target`.
    pub(crate) fn event_at(&mut self, x: f32, y: f32, target: &[u32]) -> Event {
        self.lay_out();
        let scale = self.scale;
        let mut boxes = Vec::with_capacity(target.len() + 1);
        for depth in 0..=target.len() {
            boxes.push(
                self.layout
                    .borrow()
                    .rect(&target[..depth])
                    .map_or([0.0; 4], |rect| [rect.x, rect.y, rect.width, rect.height]),
            );
        }
        let (moved_x, moved_y) = self.last_sent_move.map_or((0.0, 0.0), |(from_x, from_y)| {
            ((x - from_x) / scale, (y - from_y) / scale)
        });
        Event {
            x: x / scale,
            y: y / scale,
            screen_x: (x + self.screen_origin.0) / scale,
            screen_y: (y + self.screen_origin.1) / scale,
            movement_x: moved_x,
            movement_y: moved_y,
            buttons: self.buttons,
            modifiers: self.modifiers,
            target: target.to_vec(),
            boxes,
            root: Some(self.runtime.root().clone()),
            serial: rsui_runtime::next_event_serial(),
            ..Event::default()
        }
    }

    /// What goes with a key, for what has the keyboard, at `target`.
    pub(crate) fn key_event(&mut self, key: KeyPress, target: &[u32]) -> Event {
        let (x, y) = self.pointer.unwrap_or((0.0, 0.0));
        let mut event = self.event_at(x, y, target);
        event.modifiers = key.modifiers();
        event.key = Some(KeyInfo {
            key: key.web_key().into(),
            code: key.web_code(),
            repeat: key.repeat,
            location: key.web_location(),
            composing: self.composing,
        });
        event
    }

    /// Whether anything along `target`, or the app as a whole, wants to hear about this.
    pub(crate) fn hears(&self, kind: Kind, target: &[u32]) -> bool {
        if document_listens() & kind.listens() != 0 {
            return true;
        }
        let (up, down) = kind.actions();
        let (mouse_up, mouse_down) = kind.mouse().map_or((None, None), |mouse| {
            let (up, down) = mouse.actions();
            (up, Some(down))
        });
        let wants = |node: &Node| {
            node.actions.contains(down)
                || up.is_some_and(|up| node.actions.contains(up))
                || mouse_down.is_some_and(|down| node.actions.contains(down))
                || mouse_up.is_some_and(|up| node.actions.contains(up))
        };
        let mut node = self.runtime.root();
        if wants(node) {
            return true;
        }
        for &step in target {
            match node.children.get(step as usize) {
                Some(child) => node = child,
                None => return false,
            }
            if wants(node) {
                return true;
            }
        }
        false
    }

    /// Tells the app as a whole, if it wants to hear about this, and says whether it said to stop there.
    pub(crate) fn tell_document(
        &mut self,
        listened: u32,
        name: &str,
        phase: u8,
        event: &mut Event,
    ) -> bool {
        if document_listens() & listened == 0 {
            return false;
        }
        event.phase = phase;
        self.runtime.deliver_document_event(name, event);
        self.runtime.take_stopped()
    }

    /// Sends what happened to `target` from the root down, to whatever offers to hear it on the way down, and back up to
    /// whatever offers to hear it on the way up, stopping where something says so, with the app as a whole first on the
    /// way down and last on the way up.
    ///
    /// A pointer's press, release or move is two events, as on a page: `pointerdown` all the way down and back up, and then
    /// `mousedown` the same, as an event of its own, with the number of clicks as its `detail` (the pointer's is 0). A
    /// `pointerdown` something said not to do what it would keeps the mouse's events from being sent until the button's back
    /// up, as a browser has it.
    pub(crate) fn fire(
        &mut self,
        kind: Kind,
        target: &[u32],
        input: Option<PropValue>,
        event: &mut Event,
    ) -> Result<Fired, DispatchError> {
        let Some(mouse) = kind.mouse() else {
            return self.fire_one(kind, target, input, event);
        };
        let detail = event.detail;
        event.detail = 0;
        let pointer = self.fire_one(kind, target, input.clone(), event)?;
        if kind == Kind::Down && pointer.prevented {
            self.mouse_held_back = true;
        }
        let held_back = self.mouse_held_back;
        if kind == Kind::Up {
            self.mouse_held_back = false;
        }
        if held_back {
            return Ok(pointer);
        }
        let mut mouse_event = self.event_like(event, target);
        mouse_event.detail = if kind == Kind::Move { 0 } else { detail };
        let fired = self.fire_one(mouse, target, input, &mut mouse_event)?;
        Ok(Fired {
            stopped: pointer.stopped || fired.stopped,
            prevented: pointer.prevented || fired.prevented,
        })
    }

    /// A new event with what `event` says about where and what, for another that's sent after it, as `mousedown` is after
    /// `pointerdown`: what that one's handlers changed in the tree doesn't change what it happened to.
    fn event_like(&self, event: &Event, target: &[u32]) -> Event {
        Event {
            x: event.x,
            y: event.y,
            screen_x: event.screen_x,
            screen_y: event.screen_y,
            movement_x: event.movement_x,
            movement_y: event.movement_y,
            button: event.button,
            buttons: event.buttons,
            modifiers: event.modifiers,
            detail: event.detail,
            target: target.to_vec(),
            boxes: event.boxes.clone(),
            root: event.root.clone(),
            serial: rsui_runtime::next_event_serial(),
            ..Event::default()
        }
    }

    fn fire_one(
        &mut self,
        kind: Kind,
        target: &[u32],
        input: Option<PropValue>,
        event: &mut Event,
    ) -> Result<Fired, DispatchError> {
        let (up, down) = kind.actions();
        let name = kind.web_name(input.as_ref());
        let listened = kind.listens();
        // Timers its handlers set wait until it's been everywhere, as on a page.
        let _dispatching = rsui_runtime::dispatch_started();
        // Whatever an earlier handler said is nothing to do with this.
        self.runtime.take_stopped();
        self.runtime.take_prevented();
        let mut fired = Fired {
            stopped: self.tell_document(listened, name, 1, event),
            prevented: false,
        };
        let mut depth = 0;
        while !fired.stopped && depth <= target.len() {
            let at = &target[..depth];
            if find(self.runtime.root(), at).is_some_and(|node| node.actions.contains(down)) {
                event.phase = if depth == target.len() { 2 } else { 1 };
                self.runtime.dispatch_with(at, down, input.clone(), event)?;
                fired.stopped = self.runtime.take_stopped();
            }
            depth += 1;
        }
        // What's listening on the elements along the way hears it once the handlers on the way down have, and before those on
        // the way up, as on a page, where React hears both where its root is, around them.
        if !fired.stopped {
            fired.stopped = self.tell_document(listened, name, 2, event);
        }
        if let Some(up) = up {
            let mut depth = target.len() + 1;
            while !fired.stopped && depth > 0 {
                depth -= 1;
                let at = &target[..depth];
                if find(self.runtime.root(), at).is_some_and(|node| node.actions.contains(up)) {
                    event.phase = if depth == target.len() { 2 } else { 3 };
                    self.runtime.dispatch_with(at, up, input.clone(), event)?;
                    fired.stopped = self.runtime.take_stopped();
                }
            }
            if !fired.stopped {
                fired.stopped = self.tell_document(listened, name, 3, event);
            } else {
                // What happens by itself, as a key typing, still does when nothing further along was to hear it, which the app
                // as a whole is told after everything else, as phase 4.
                self.tell_document(listened, name, 4, event);
                self.runtime.take_stopped();
            }
        }
        fired.prevented = self.runtime.take_prevented();
        Ok(fired)
    }

    /// Where moves go while a mouse button is down on something that hears them, if anything is.
    pub(crate) fn captured_path(&mut self) -> Option<Vec<u32>> {
        let captured = self.captured.as_ref()?;
        let path = follow(self.runtime.root(), &captured.path, &captured.keys);
        if path.is_none() {
            self.captured = None;
        }
        path
    }

    /// Sends what the pointer does to what's at `target`, as a page's `setPointerCapture()` asks.
    pub(crate) fn capture_explicitly(&mut self, target: &[u32]) {
        self.captured = Some(Captured {
            path: target.to_vec(),
            keys: keys_along(self.runtime.root(), target),
        });
    }

    /// Starts sending moves to what was pressed, if it or anything it's in hears them.
    pub(crate) fn capture(&mut self, target: &[u32]) {
        self.captured = self.hears_moves_along(target).then(|| Captured {
            path: target.to_vec(),
            keys: keys_along(self.runtime.root(), target),
        });
    }

    /// Whether anything along `target` hears the pointer move. The app as a whole hears it wherever it is, so that
    /// doesn't count.
    fn hears_moves_along(&self, target: &[u32]) -> bool {
        let mut node = self.runtime.root();
        let wants = Self::hears_moves;
        if wants(node) {
            return true;
        }
        for &step in target {
            match node.children.get(step as usize) {
                Some(child) => node = child,
                None => return false,
            }
            if wants(node) {
                return true;
            }
        }
        false
    }

    /// The pointer's moved, for whatever hears that: now, or once it's been long enough since the last time.
    pub(crate) fn pointer_moved(&mut self) {
        if !self.moves_heard && self.captured.is_none() && document_listens() & listens::MOVE == 0 {
            return;
        }
        self.move_due = true;
        let soon_enough = self
            .last_move_at
            .is_some_and(|last| last.elapsed() < self.move_pace);
        if !soon_enough {
            self.send_move();
        }
    }

    /// How long until a move that's waiting can be told.
    pub(crate) fn move_wait(&self) -> Option<Duration> {
        if !self.move_due {
            return None;
        }
        Some(self.last_move_at.map_or(Duration::ZERO, |last| {
            self.move_pace.saturating_sub(last.elapsed())
        }))
    }

    /// Services a paced move during a frame as well as a timer wake. Continuously invalidated native paint messages can
    /// starve Windows timers, so timers alone cannot be responsible for delivering input while an animation is running.
    pub(crate) fn send_due_move(&mut self) -> bool {
        if self.move_wait().is_some_and(|wait| wait.is_zero()) {
            self.send_move();
            return true;
        }
        false
    }

    /// Tells what hears the pointer move where it is now, if that's waiting to be told.
    pub(crate) fn send_move(&mut self) {
        if !std::mem::take(&mut self.move_due) {
            return;
        }
        let Some((x, y)) = self.pointer else {
            return;
        };
        let target = match self.captured_path() {
            Some(path) => path,
            // Where what it's over is now, which things put in before it since may have moved along.
            None => follow_along(self.runtime.root(), &self.pointer_paths[0], &self.pointer_keys[0]).0,
        };
        self.last_move_at = Some(Instant::now());
        let mut event = self.event_at(x, y, &target);
        self.last_sent_move = Some((x, y));
        // It's only being told.
        let _ = self.fire(Kind::Move, &target, None, &mut event);
    }

    /// Tells what's under a press, or what has the moves, that a button went down or came up. Says whether something said
    /// what it does by itself shouldn't happen, which for the main button going down is the keyboard moving to it.
    pub(crate) fn tell_button(
        &mut self,
        kind: Kind,
        x: f32,
        y: f32,
        target: &[u32],
        button: PointerButton,
        clicks: u32,
    ) -> bool {
        if !self.hears(kind, target) {
            return false;
        }
        let mut event = self.event_at(x, y, target);
        event.button = button.web_button();
        event.detail = clicks;
        self.fire(kind, target, None, &mut event)
            .is_ok_and(|fired| fired.prevented)
    }

    /// Tells what has the keyboard, and what it's in, about a key, and says whether something said what it does by itself
    /// shouldn't happen.
    pub(crate) fn tell_key(&mut self, kind: Kind, key: KeyPress) -> Result<bool, DispatchError> {
        let target = self.focused().map(<[u32]>::to_vec).unwrap_or_default();
        if !self.hears(kind, &target) {
            return Ok(false);
        }
        let mut event = self.key_event(key, &target);
        let input = Some(PropValue::from(key.describe()));
        Ok(self.fire(kind, &target, input, &mut event)?.prevented)
    }

    /// Tells what has the keyboard, and what it's in, that something's about to be pasted, copied or cut, and says whether
    /// something said it shouldn't be. What's pasted comes with it.
    pub(crate) fn tell_clipboard(&mut self, what: &'static str) -> Result<bool, DispatchError> {
        let target = self.focused().map(<[u32]>::to_vec).unwrap_or_default();
        if !self.hears(Kind::Clipboard, &target) {
            return Ok(false);
        }
        let (x, y) = self.pointer.unwrap_or((0.0, 0.0));
        let mut event = self.event_at(x, y, &target);
        if what == "paste" {
            event.data = self.clipboard.read().map(Into::into);
        }
        Ok(self
            .fire(
                Kind::Clipboard,
                &target,
                Some(PropValue::from(what)),
                &mut event,
            )?
            .prevented)
    }

    /// Something's being put together by a way of typing, as Chinese, Japanese and Korean are: `phase` is `"start"`,
    /// `"update"` with what it's come to so far, or `"end"` with what it came to, which is then typed as anything else is. A
    /// window says this as the system does, and with no window it's for trying out what an app does about it.
    pub fn composition(&mut self, phase: &str, text: &str) -> Result<(), DispatchError> {
        let phase: &'static str = match phase {
            "start" => "start",
            "end" => "end",
            _ => "update",
        };
        self.composing = phase != "end";
        self.update();
        // Nothing's put together in a field that can't be changed, as a browser has it.
        if self.focus_is_read_only() {
            self.composing = false;
            return Ok(());
        }
        let target = self.focused().map(<[u32]>::to_vec).unwrap_or_default();
        if !self.hears(Kind::Composition, &target) {
            return Ok(());
        }
        let (x, y) = self.pointer.unwrap_or((0.0, 0.0));
        let mut event = self.event_at(x, y, &target);
        event.data = Some(text.into());
        self.fire(
            Kind::Composition,
            &target,
            Some(PropValue::from(phase)),
            &mut event,
        )?;
        Ok(())
    }

    /// Text was typed while what has the keyboard isn't a field: says so to what hears it, as an element a page can edit does,
    /// which puts the text in itself.
    pub(crate) fn typed_elsewhere(&mut self, text: &str) -> Result<(), DispatchError> {
        let target = self.focused().map(<[u32]>::to_vec).unwrap_or_default();
        if !self.hears(Kind::Text, &target) {
            return Ok(());
        }
        let (x, y) = self.pointer.unwrap_or((0.0, 0.0));
        let mut event = self.event_at(x, y, &target);
        event.data = Some(text.into());
        self.fire(Kind::Text, &target, None, &mut event)?;
        Ok(())
    }

    /// Says what's selected in the field that has the keyboard, for a page's `selectionStart`, `selectionEnd` and
    /// `selectionDirection` (see [`field_selection`](crate::field_selection)), and tells what hears it whenever it's changed, as
    /// a page's `selectionchange` does, with `data` saying whether it's more than a caret (`"range"`), as `select` is for.
    pub(crate) fn publish_selection(&mut self) -> Result<(), DispatchError> {
        let Some(target) = self.focused().map(<[u32]>::to_vec) else {
            return Ok(());
        };
        let Some(edit) = self.focus.as_ref().and_then(|focus| focus.edit.as_ref()) else {
            return Ok(());
        };
        let (anchor, caret) = (edit.anchor(), edit.caret());
        let node = self.focused_node();
        // A field with a `ref` has its key as its own, rather than as `dom_key`, as input-otp's one field does.
        let Some(key) = super::page_key(&node).map(str::to_owned) else {
            return Ok(());
        };
        // What the page was last told, which isn't what was last said for its `selectionStart` while a change was offered to it.
        let told = (key.clone(), anchor, caret);
        super::say_field_selection(key, anchor, caret);
        if self.selection_told.as_ref() == Some(&told) {
            return Ok(());
        }
        self.selection_told = Some(told);
        if !self.hears(Kind::Select, &target) {
            return Ok(());
        }
        let (x, y) = self.pointer.unwrap_or((0.0, 0.0));
        let mut event = self.event_at(x, y, &target);
        event.data = Some(if anchor == caret { "caret" } else { "range" }.into());
        self.fire(Kind::Select, &target, None, &mut event)?;
        Ok(())
    }

    /// Tells what's under the pointer about the wheel, and says whether something said it shouldn't scroll.
    pub(crate) fn tell_wheel(&mut self, x: f32, y: f32, across: f32, down: f32) -> bool {
        let Some(target) = self.path_at(x, y) else {
            return false;
        };
        if !self.hears(Kind::Wheel, &target) {
            return false;
        }
        let mut event = self.event_at(x, y, &target);
        event.delta_x = across;
        event.delta_y = down;
        self.fire(Kind::Wheel, &target, None, &mut event)
            .is_ok_and(|fired| fired.prevented)
    }

    /// What hears the pointer or the mouse move, on the way up or down, for looking along where it is.
    pub(crate) fn move_actions() -> [ActionKind; 4] {
        let (up, down) = Kind::Move.actions();
        let (mouse_up, mouse_down) = Kind::MouseMove.actions();
        [up.expect("moves go up"), down, mouse_up.expect("moves go up"), mouse_down]
    }

    /// Whether this hears the pointer or the mouse move.
    pub(crate) fn hears_moves(node: &Node) -> bool {
        Self::move_actions().iter().any(|action| node.actions.contains(*action))
    }

    /// The deepest thing at this position, below `path`, for what code written for the web calls the target.
    pub(crate) fn deepest_in(&mut self, path: &[u32], x: f32, y: f32) -> Vec<u32> {
        self.path_at(x, y)
            .filter(|deepest| deepest.starts_with(path))
            .unwrap_or_else(|| path.to_vec())
    }

    /// The click event's `detail`, and its button, for one that's dispatched.
    pub(crate) fn click_event(
        &mut self,
        at: Option<(f32, f32)>,
        target: &[u32],
        clicks: u32,
    ) -> Event {
        let mut event = match at {
            Some((x, y)) => self.event_at(x, y, target),
            None => {
                // Done with the keyboard, or by an agent, it's in the middle of what's clicked, which says no clicks.
                self.lay_out();
                let middle = self
                    .layout
                    .borrow()
                    .rect(target)
                    .map_or((0.0, 0.0), |rect| {
                        (
                            (rect.x + rect.width / 2.0) * self.scale,
                            (rect.y + rect.height / 2.0) * self.scale,
                        )
                    });
                self.event_at(middle.0, middle.1, target)
            }
        };
        event.detail = clicks;
        event
    }
}

/// Whether something says it can be dragged, as a page's `draggable` does.
fn is_draggable(node: &Node) -> bool {
    match node.props.get_symbol(symbol!("draggable")) {
        Some(PropValue::Bool(on)) => *on,
        Some(PropValue::Str(text)) => text.as_str() == "true",
        _ => false,
    }
}

// Dragging, as the web has it: something with `draggable` is pressed and moved a little, which starts it, then what it's over
// hears it come, move and go, and says whether it can be dropped there, and it's dropped there or not when the button comes
// up. What's dragged in from another program goes the same way, with what it is.
impl App {
    /// Whether something's being dragged, as a page has it.
    pub fn is_dragging(&self) -> bool {
        self.html_drag.is_some()
    }

    /// Whether what's being dragged is from inside the app, which a press ending some other way ends too.
    pub(crate) fn dragging_from_here(&self) -> bool {
        self.html_drag
            .as_ref()
            .is_some_and(|session| session.source.is_some())
    }

    /// What the main button went down on that can be dragged, if anything along `under` can.
    pub(crate) fn note_draggable(&mut self, under: &[u32], x: f32, y: f32) {
        let mut node = self.runtime.root();
        let mut found = None;
        for (depth, &step) in under.iter().enumerate() {
            match node.children.get(step as usize) {
                Some(child) => node = child,
                None => break,
            }
            if is_draggable(node) {
                found = Some(depth + 1);
            }
        }
        self.drag_from = found.map(|depth| (under[..depth].to_vec(), (x, y)));
    }

    /// Tells about a part of dragging, from the root down to `target` and back up, and says whether it was stopped from
    /// doing what it would, which for `dragover` and `dragenter` means it can be dropped there.
    fn tell_drag(&mut self, kind: &'static str, target: &[u32], x: f32, y: f32) -> bool {
        if !self.hears(Kind::Drag, target) {
            return false;
        }
        let mut event = self.event_at(x, y, target);
        event.buttons = self.buttons;
        if let Some(session) = &self.html_drag {
            event.files.clone_from(&session.files);
            event.data.clone_from(&session.text);
        }
        self.fire(Kind::Drag, target, Some(PropValue::from(kind)), &mut event)
            .is_ok_and(|fired| fired.prevented)
    }

    /// The pointer's moved with the main button down. It starts a drag if it was pressed on something that can be, and has
    /// gone far enough, and says whether that's what's happening, in which case nothing else hears it move.
    pub(crate) fn drag_moved(&mut self, x: f32, y: f32) -> bool {
        if self.html_drag.is_some() {
            self.drag_over(x, y);
            return true;
        }
        let Some((source, (start_x, start_y))) = self.drag_from.clone() else {
            return false;
        };
        let far = DRAG_START * self.scale;
        if (x - start_x).abs() < far && (y - start_y).abs() < far {
            return false;
        }
        self.drag_from = None;
        if find(self.runtime.root(), &source).is_none() {
            return false;
        }
        self.html_drag = Some(DragSession {
            source: Some(source.clone()),
            over: None,
            accepted: false,
            files: Vec::new(),
            text: None,
        });
        if self.tell_drag("dragstart", &source, start_x, start_y) {
            self.html_drag = None;
            return false;
        }
        // It's not a press or a selection any more, and nothing's held for its moves.
        self.pressed = None;
        self.captured = None;
        self.dragged = None;
        self.selecting = false;
        self.selecting_text = false;
        self.invalidate_automatic();
        self.repaint = true;
        self.drag_over(x, y);
        true
    }

    /// What's being dragged is here: what it's over now hears it come if it's just got there, and what it was over hears it
    /// go, and what it's over says whether it can be dropped there.
    fn drag_over(&mut self, x: f32, y: f32) {
        let over = self.path_at(x, y).unwrap_or_default();
        let (source, before) = match &self.html_drag {
            Some(session) => (session.source.clone(), session.over.clone()),
            None => return,
        };
        if let Some(source) = &source {
            self.tell_drag("drag", source, x, y);
        }
        let mut accepted = false;
        if before.as_deref() != Some(over.as_slice()) {
            if let Some(before) = &before {
                self.tell_drag("dragleave", before, x, y);
            }
            accepted = self.tell_drag("dragenter", &over, x, y);
        }
        accepted |= self.tell_drag("dragover", &over, x, y);
        if let Some(session) = &mut self.html_drag {
            session.over = Some(over);
            session.accepted = accepted;
        }
    }

    /// The button's come up while something's being dragged, which drops it where it is, if that's allowed there.
    pub(crate) fn drag_released(&mut self, x: f32, y: f32) {
        let over = self.path_at(x, y).unwrap_or_default();
        self.end_drag(Some(over), x, y, true);
    }

    /// Ends a drag without dropping it, as escape does.
    pub(crate) fn cancel_drag(&mut self) {
        let (x, y) = self.pointer.unwrap_or((0.0, 0.0));
        self.end_drag(None, x, y, false);
    }

    /// Drops what's being dragged on what it's over, if it's been said that it can be and `dropping`, or leaves it, and
    /// tells what was dragged that that's the end of it.
    fn end_drag(&mut self, over: Option<Vec<u32>>, x: f32, y: f32, dropping: bool) {
        let Some(session) = self.html_drag.as_mut() else {
            return;
        };
        if let Some(over) = over {
            session.over = Some(over);
        }
        let (over, source, accepted) = (
            session.over.clone(),
            session.source.clone(),
            session.accepted,
        );
        if let Some(over) = &over {
            self.tell_drag(
                if dropping && accepted {
                    "drop"
                } else {
                    "dragleave"
                },
                over,
                x,
                y,
            );
        }
        if let Some(source) = &source {
            self.tell_drag("dragend", source, x, y);
        }
        self.html_drag = None;
    }

    /// Something from another program is being dragged over the window, or has been dropped on it, at this position in
    /// physical pixels, with these files or this text: what's under it hears it as a page's elements do. Says whether it can
    /// be dropped there, which it can if something said so, or if they're files and something takes them, as `onDropFiles`
    /// does. A drop that nothing heard goes to what takes files, as it always has. A window says this as the system does.
    pub fn drag_from_outside(
        &mut self,
        phase: DropPhase,
        x: f32,
        y: f32,
        files: &[String],
        text: Option<&str>,
    ) -> bool {
        self.update();
        self.lay_out();
        self.pointer = Some((x, y));
        if phase == DropPhase::Enter || self.html_drag.is_none() {
            self.html_drag = Some(DragSession {
                source: None,
                over: None,
                accepted: false,
                files: files.iter().map(|path| path.as_str().into()).collect(),
                text: text.map(Into::into),
            });
        }
        let takes_files = !files.is_empty()
            && self
                .target_for(x, y, ActionKind::Custom(symbol!("drop_files")))
                .is_some();
        match phase {
            DropPhase::Enter | DropPhase::Over => {
                self.drag_over(x, y);
                let accepted = self
                    .html_drag
                    .as_ref()
                    .is_some_and(|session| session.accepted);
                accepted || takes_files
            }
            DropPhase::Leave => {
                self.cancel_drag();
                false
            }
            DropPhase::Drop => {
                let accepted = self
                    .html_drag
                    .as_ref()
                    .is_some_and(|session| session.accepted);
                self.drag_released(x, y);
                if !accepted && takes_files {
                    let _ = self.files_dropped(x, y, files);
                }
                accepted || takes_files
            }
        }
    }
}
