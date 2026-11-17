//! What comes with something that happens, beyond the input a handler's given: where the pointer was, which button, what was
//! held, which key, and what it happened to. Handlers that want it ask [`Updates::event`](crate::Updates::event), as code
//! written for the web does, and those that don't pay nothing for it.

use std::cell::Cell;

use rsui_core::{Node, Text};

/// The keys held along with something, as the web says them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// The Windows key.
    pub meta: bool,
}

/// What a key that was pressed or let go of was, as the web says it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeyInfo {
    /// What it means as it's set out on this keyboard: `"a"`, `"A"`, `"Enter"`, `"F1"`, `"Shift"`.
    pub key: Text,
    /// Where it is on the keyboard, whatever it means there: `"KeyA"`, `"Digit1"`, `"ShiftLeft"`. Shortcuts that go by this stay
    /// in the same place whatever language the keyboard's in.
    pub code: &'static str,
    /// Whether it's being held down, rather than having just been pressed.
    pub repeat: bool,
    /// 0 for most, 1 for the one on the left of two, 2 for the right, and 3 for the number pad.
    pub location: u8,
    /// Whether something's being put together by a way of typing, as Chinese, Japanese and Korean are, which has the keys first.
    pub composing: bool,
}

/// Where something happened, and what to, as the web has it.
#[derive(Debug, Clone, Default)]
pub struct Event {
    /// Where the pointer was, from the window's top left, in logical pixels.
    pub x: f32,
    pub y: f32,
    /// The same, from the top left of the screen.
    pub screen_x: f32,
    pub screen_y: f32,
    /// How far the pointer moved since the last time it was said, in logical pixels.
    pub movement_x: f32,
    pub movement_y: f32,
    /// Which button this is about, as the web numbers them: 0 the main one, 1 the middle, 2 the other, 3 back and 4 forward.
    pub button: u8,
    /// Which buttons are down, as the web has them: 1 the main one, 2 the other, 4 the middle, 8 back and 16 forward.
    pub buttons: u8,
    pub modifiers: Modifiers,
    /// How many clicks in a row this is.
    pub detail: u32,
    /// Where in the tree it happened, which is the deepest thing there, or what has the keyboard.
    pub target: Vec<u32>,
    /// Where each thing from the root down to `target` is, in logical pixels: `[x, y, width, height]`, one for each of
    /// `target`'s steps and one for the root.
    pub boxes: Vec<[f32; 4]>,
    /// The tree as it was, to read about `target` and what it's in.
    pub root: Option<Node>,
    /// What's hearing about it now, which is somewhere along `target`.
    pub current: Vec<u32>,
    /// 1 on the way down to `target`, 2 at it, and 3 on the way back up, as the web's `eventPhase` says.
    pub phase: u8,
    pub key: Option<KeyInfo>,
    /// How far the wheel turned, in pixels.
    pub delta_x: f32,
    pub delta_y: f32,
    /// Text that came with it, as what's pasted, what's being put together by a way of typing, or what's being dragged in.
    pub data: Option<Text>,
    /// The paths of files being dragged in from another program.
    pub files: Vec<Text>,
    /// Which event this is, from [`next_event_serial`], for code written for the web to have one object for it however many
    /// handlers it goes to, as a page does. 0 says nothing.
    pub serial: u64,
}

thread_local! {
    static EVENT_SERIAL: Cell<u64> = const { Cell::new(0) };
}

/// A number no other event has had, for [`Event::serial`].
pub fn next_event_serial() -> u64 {
    EVENT_SERIAL.with(|serial| {
        let next = serial.get() + 1;
        serial.set(next);
        next
    })
}

/// Which kinds of thing are heard by the app as a whole, as the web's `document.addEventListener` and
/// `window.addEventListener` are, rather than by anything in the tree.
pub mod listens {
    /// `pointerdown` and `mousedown`.
    pub const DOWN: u32 = 1;
    /// `pointerup` and `mouseup`.
    pub const UP: u32 = 1 << 1;
    /// `pointermove` and `mousemove`.
    pub const MOVE: u32 = 1 << 2;
    pub const CLICK: u32 = 1 << 3;
    pub const KEY_DOWN: u32 = 1 << 4;
    pub const KEY_UP: u32 = 1 << 5;
    pub const WHEEL: u32 = 1 << 6;
    /// `paste`, `copy` and `cut`.
    pub const CLIPBOARD: u32 = 1 << 7;
    /// `compositionstart`, `compositionupdate` and `compositionend`.
    pub const COMPOSITION: u32 = 1 << 8;
    pub const CONTEXT_MENU: u32 = 1 << 9;
    pub const DOUBLE_CLICK: u32 = 1 << 10;
    /// What's being dragged around inside the app, as the web's `dragover` and `drop` are.
    pub const DRAG: u32 = 1 << 11;
    /// Text typed while what has the keyboard isn't a field, as an element a page can edit hears it.
    pub const TEXT: u32 = 1 << 12;
    /// What's selected in a field changed, as a page's `select` says.
    pub const SELECT: u32 = 1 << 13;
}

thread_local! {
    /// The tree as it was when it last changed, for what runs to look through: see [`current_tree`].
    static CURRENT_TREE: std::cell::RefCell<Option<rsui_core::Node>> = const { std::cell::RefCell::new(None) };
    static DOCUMENT_LISTENS: Cell<u32> = const { Cell::new(0) };
    static WANTS_DROPS: Cell<bool> = const { Cell::new(false) };
    static DISPATCHING: Cell<u32> = const { Cell::new(0) };
}

/// Says an event is on its way through the tree until what's returned is dropped: see [`dispatching`].
pub fn dispatch_started() -> Dispatching {
    DISPATCHING.with(|depth| depth.set(depth.get() + 1));
    Dispatching(())
}

/// While it's kept, an event's on its way through the tree, from [`dispatch_started`].
pub struct Dispatching(());

impl Drop for Dispatching {
    fn drop(&mut self) {
        DISPATCHING.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Whether an event is on its way through the tree, from the app as a whole down to what it happened to and back up. What its
/// handlers leave for later (a timer that's due) waits until it's been everywhere, as on a page, where timers never run in
/// the middle of an event, only what's queued as a microtask between one listener and the next.
pub fn dispatching() -> bool {
    DISPATCHING.with(Cell::get) > 0
}

/// The tree as it is now, which may not have been laid out yet, for code that looks through it while it runs, as a page's
/// `querySelector()` does, in an effect as much as in a handler. It's the one that last changed, if there's more than one.
pub fn current_tree() -> Option<rsui_core::Node> {
    CURRENT_TREE.with(|tree| tree.borrow().clone())
}

pub(crate) fn set_current_tree(tree: &rsui_core::Node) {
    CURRENT_TREE.with(|current| *current.borrow_mut() = Some(tree.clone()));
}

/// Lets go of the tree, as it's about to change, and holding it would make that copy what it changes.
pub(crate) fn forget_current_tree() {
    CURRENT_TREE.with(|current| current.borrow_mut().take());
}

/// Says that something in the app wants to hear about things dragged in from other programs and dropped, as a page's
/// `onDrop` does, so a window starts telling it. It isn't asked for until then, as it costs a little to set up.
pub fn set_wants_drops() {
    WANTS_DROPS.with(|wants| wants.set(true));
}

/// Whether [`set_wants_drops`] has been said.
pub fn wants_drops() -> bool {
    WANTS_DROPS.with(Cell::get)
}

/// Says which kinds of thing the app as a whole wants to hear about, from [`listens`], which every window on this thread is
/// for. Each is passed to what [`Cx::on_document_event`](crate::Cx::on_document_event) gave, on the way down and on the way
/// back up.
pub fn set_document_listens(kinds: u32) {
    DOCUMENT_LISTENS.with(|listens| listens.set(kinds));
}

/// What [`set_document_listens`] last said.
pub fn document_listens() -> u32 {
    DOCUMENT_LISTENS.with(Cell::get)
}
