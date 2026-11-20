//! Where an app's time goes, as timed spans that every part of rsui writes, for the inspector to show as a flame chart.
//!
//! A span is a kind, when it started and ended, and what it was about (a label, such as a component's name, and a number
//! that goes with it). Spans aren't nested when they're kept: what's inside what is worked out from when each started and
//! ended, on each thread, which is how a flame chart's drawn anyway.
//!
//! There are three levels, and the app says which:
//!
//! - [`Level::Off`], for a program that's been handed to someone: [`span`] loads one byte and does nothing else.
//! - [`Level::Light`], for a program being worked on, from when it starts: only the kinds that say where a frame's time
//!   went (handling input, running script, laying out, painting, the GPU, collecting garbage), a few each frame, so the
//!   last [`KEPT_FOR`] is there to look at when something's been slow, without having to start recording first.
//! - [`Level::Detail`], while an inspector's looking: everything, down to each component's render, each host function and
//!   each text shaped.
//!
//! What's kept is the last [`KEPT_FOR`], up to [`MOST_KEPT`] spans, whichever's fewer. [`since`] gives what's been kept
//! after a point, for a viewer that asks again and again.

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// How long spans are kept for.
pub const KEPT_FOR_NANOS: u64 = 30_000_000_000;
/// The most spans kept, whatever their age: about 8 MB.
pub const MOST_KEPT: usize = 256 * 1024;

/// How much is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    Off = 0,
    Light = 1,
    Detail = 2,
}

static LEVEL: AtomicU8 = AtomicU8::new(0);

/// How much is being kept now.
#[inline]
pub fn level() -> Level {
    match LEVEL.load(Ordering::Relaxed) {
        0 => Level::Off,
        1 => Level::Light,
        _ => Level::Detail,
    }
}

/// Whether anything's being kept.
#[inline]
pub fn on() -> bool {
    LEVEL.load(Ordering::Relaxed) != 0
}

/// Whether everything is, as while an inspector's looking.
#[inline]
pub fn detail() -> bool {
    LEVEL.load(Ordering::Relaxed) >= Level::Detail as u8
}

/// Keeps this much from now on. Going down to [`Level::Off`] keeps what's been kept until it's old enough to go.
pub fn set_level(level: Level) {
    if level != Level::Off {
        origin();
    }
    LEVEL.store(level as u8, Ordering::Relaxed);
}

/// What a span is about, in groups the inspector colors alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Frame,
    Input,
    Script,
    Style,
    Layout,
    Paint,
    Gpu,
    Gc,
    Idle,
    Other,
}

impl Category {
    pub fn name(self) -> &'static str {
        match self {
            Category::Frame => "frame",
            Category::Input => "input",
            Category::Script => "script",
            Category::Style => "style",
            Category::Layout => "layout",
            Category::Paint => "paint",
            Category::Gpu => "gpu",
            Category::Gc => "gc",
            Category::Idle => "idle",
            Category::Other => "other",
        }
    }
}

macro_rules! kinds {
    ($($kind:ident = $name:literal, $category:ident, $light:literal, $about:literal;)*) => {
        /// What a span is.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        #[repr(u8)]
        pub enum Kind {
            $(#[doc = $about] $kind,)*
        }

        /// Each kind, in order, with its name, what it's about, and whether [`Level::Light`] keeps it.
        pub const KINDS: &[(&str, Category, bool, &str)] = &[$(($name, Category::$category, $light, $about),)*];
        const LIGHT: &[bool] = &[$($light,)*];
    };
}

kinds! {
    Frame = "Frame", Frame, true, "A frame, from when its update started to when it was handed to the screen.";
    Update = "Update", Frame, true, "Bringing the app up to date: what came in, rendering, and what that changed.";
    Input = "Input", Input, true, "Handling something that came in, before it reaches the app's code.";
    Handler = "Event handler", Script, true, "An app's handler for a click, a key, or another action.";
    Timer = "Timer", Script, true, "A timer going off: setTimeout, setInterval, or an animation frame.";
    Jobs = "Promise jobs", Script, true, "What promises had waiting, run once what was running had finished.";
    Script = "Script", Script, true, "Running JavaScript, from Rust calling into the engine.";
    Render = "Render", Script, false, "A component rendering.";
    Effects = "Effects", Script, false, "Running effects after a render.";
    Flush = "Reconcile", Script, true, "Working out what's changed in the tree, which renders what needs to.";
    Style = "Style", Style, false, "Working out which CSS applies to what changed.";
    LayoutApply = "Invalidate layout", Layout, false, "Working out what needs laying out from what changed.";
    Layout = "Layout", Layout, true, "Laying out.";
    Text = "Shape text", Layout, false, "Shaping a run of text into glyphs.";
    Geometry = "Geometry", Layout, false, "Telling what's listening where things are after a layout.";
    Paint = "Paint", Paint, true, "Working out what to draw.";
    Gpu = "GPU", Gpu, true, "Handing what's to be drawn to the GPU.";
    Present = "Present", Idle, true, "Waiting for the screen to take the frame.";
    Gc = "GC", Gc, true, "Collecting garbage.";
    Image = "Decode image", Other, false, "Decoding an image.";
    Module = "Load module", Script, false, "Reading and compiling a module.";
    Idle = "Idle", Idle, false, "Waiting for something to happen.";
    Part = "Part", Other, false, "A part of rsui's own work that's been timed, as `RSUI_SPLIT` names it.";
    Sample = "Sample", Script, false, "Where JavaScript was when it was sampled: the label is the stack.";
    Mark = "Mark", Other, true, "Something that happened at one time, such as a frame being asked for.";
}

impl Kind {
    pub fn name(self) -> &'static str {
        KINDS[self as usize].0
    }

    pub fn category(self) -> Category {
        KINDS[self as usize].1
    }
}

/// Whether a span of this kind is being kept.
#[inline]
pub fn wants(kind: Kind) -> bool {
    match LEVEL.load(Ordering::Relaxed) {
        0 => false,
        1 => LIGHT[kind as usize],
        _ => true,
    }
}

/// One span, as it's kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rec {
    /// Nanoseconds since [`origin`].
    pub start: u64,
    pub end: u64,
    /// What it's about, from [`intern`], or nothing for 0.
    pub label: u32,
    /// A number that goes with it, as a GC's kind or how many glyphs were shaped.
    pub arg: u32,
    pub kind: u8,
    /// Which thread it was on, from 0 in the order threads first kept one: see [`threads`].
    pub thread: u8,
}

/// When time starts for what's kept.
pub fn origin() -> Instant {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    *ORIGIN.get_or_init(Instant::now)
}

/// Nanoseconds since [`origin`].
#[inline]
pub fn now() -> u64 {
    nanos(Instant::now())
}

/// An instant, as nanoseconds since [`origin`].
#[inline]
pub fn nanos(at: Instant) -> u64 {
    at.saturating_duration_since(origin()).as_nanos() as u64
}

/// A span as it's kept: [`Rec`] with how long it took in place of when it ended, which is 24 bytes rather than 32. One that
/// took longer than `u32::MAX` nanoseconds (4.3 s) has `u32::MAX` here, and its end in [`Store::long`].
#[derive(Clone, Copy)]
struct Packed {
    start: u64,
    took: u32,
    label: u32,
    arg: u32,
    kind: u8,
    thread: u8,
}

/// How many spans each piece of [`Store`] holds. The spans kept are in pieces of this many, rather than in one list that
/// doubles as it grows: 30,000 spans were 2 MB as a list with room for 65,536, and are 0.7 MB in pieces.
const PIECE: usize = 4096;

struct Store {
    /// The spans, oldest first, in pieces that are full but for the last, the first starting at `head`.
    pieces: VecDeque<Vec<Packed>>,
    head: usize,
    len: usize,
    /// The number of the first span kept, counting every span ever kept.
    first: u64,
    pushed_since_trim: u32,
    /// The end of each span kept that took too long to say in [`Packed::took`], by its number.
    long: VecDeque<(u64, u64)>,
}

impl Store {
    fn push(&mut self, rec: Rec) {
        let took = rec.end.saturating_sub(rec.start);
        let packed = Packed {
            start: rec.start,
            took: u32::try_from(took).unwrap_or(u32::MAX),
            label: rec.label,
            arg: rec.arg,
            kind: rec.kind,
            thread: rec.thread,
        };
        if packed.took == u32::MAX {
            self.long.push_back((self.first + self.len as u64, rec.end));
        }
        if self.pieces.back().is_none_or(|piece| piece.len() == PIECE) {
            self.pieces.push_back(Vec::with_capacity(PIECE));
        }
        if let Some(piece) = self.pieces.back_mut() {
            piece.push(packed);
            self.len += 1;
        }
    }

    fn pop_front(&mut self) {
        if self.len == 0 {
            return;
        }
        self.head += 1;
        self.len -= 1;
        self.first += 1;
        let front = self.pieces.front().map_or(0, Vec::len);
        if self.head == front {
            if self.pieces.len() > 1 || front == PIECE {
                self.pieces.pop_front();
            } else if let Some(piece) = self.pieces.front_mut() {
                piece.clear();
            }
            self.head = 0;
        }
        while self.long.front().is_some_and(|(number, _)| *number < self.first) {
            self.long.pop_front();
        }
    }

    /// The `at`th span kept, from the oldest.
    fn get(&self, at: usize) -> Rec {
        let place = self.head + at;
        let packed = self.pieces[place / PIECE][place % PIECE];
        let end = if packed.took == u32::MAX {
            let number = self.first + at as u64;
            let found = self.long.binary_search_by_key(&number, |(number, _)| *number);
            found.map_or(u64::MAX, |found| self.long[found].1)
        } else {
            packed.start + u64::from(packed.took)
        };
        Rec {
            start: packed.start,
            end,
            label: packed.label,
            arg: packed.arg,
            kind: packed.kind,
            thread: packed.thread,
        }
    }

    fn front_end(&self) -> Option<u64> {
        (self.len > 0).then(|| self.get(0).end)
    }
}

static STORE: Mutex<Store> = Mutex::new(Store {
    pieces: VecDeque::new(),
    head: 0,
    len: 0,
    first: 0,
    pushed_since_trim: 0,
    long: VecDeque::new(),
});

/// How many spans have been dropped because there were too many at once, which says the newest are all there is.
static DROPPED: AtomicU64 = AtomicU64::new(0);

/// Keeps a span that's been timed already.
pub fn record(kind: Kind, start: u64, end: u64, label: u32, arg: u32) {
    let thread = thread_number();
    let rec = Rec {
        start,
        end,
        label,
        arg,
        kind: kind as u8,
        thread,
    };
    let Ok(mut store) = STORE.lock() else {
        return;
    };
    if store.len >= MOST_KEPT {
        store.pop_front();
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
    store.push(rec);
    store.pushed_since_trim += 1;
    if store.pushed_since_trim >= 512 {
        store.pushed_since_trim = 0;
        let oldest = end.saturating_sub(KEPT_FOR_NANOS);
        while store.front_end().is_some_and(|end| end < oldest) {
            store.pop_front();
        }
    }
}

/// Keeps something that happened at one time.
pub fn mark(kind: Kind, label: u32, arg: u32) {
    if wants(kind) {
        let at = now();
        record(kind, at, at, label, arg);
    }
}

/// What's been kept from span number `after` on, and the number to ask from next time. Asking from 0 gives all there
/// is.
pub fn since(after: u64) -> (Vec<Rec>, u64) {
    let Ok(store) = STORE.lock() else {
        return (Vec::new(), after);
    };
    let next = store.first + store.len as u64;
    let skip = (after.saturating_sub(store.first) as usize).min(store.len);
    ((skip..store.len).map(|at| store.get(at)).collect(), next)
}

/// How many spans are kept, and how many bytes the room for them takes, for measuring memory.
pub fn footprint() -> (usize, usize) {
    STORE.lock().map_or((0, 0), |store| (store.len, store.pieces.len() * PIECE * size_of::<Packed>()))
}

/// How many spans have been dropped, as there were too many to keep.
pub fn dropped() -> u64 {
    DROPPED.load(Ordering::Relaxed)
}

/// A span that's going on, which is kept when it's dropped.
#[must_use = "a span is kept when it's dropped, so it has to be held for as long as what it times"]
pub struct Span {
    start: u64,
    kind: Kind,
    label: u32,
    arg: u32,
}

impl Span {
    /// Says what it's about.
    pub fn label(&mut self, label: &str) {
        self.label = intern(label);
    }

    /// Says what it's about, with a label from [`intern`].
    pub fn label_id(&mut self, label: u32) {
        self.label = label;
    }

    pub fn arg(&mut self, arg: u32) {
        self.arg = arg;
    }

    /// When it started, in nanoseconds since [`origin`].
    pub fn start(&self) -> u64 {
        self.start
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let end = now();
        // Text that was measured from what's kept is too quick to see, and there can be thousands of them.
        if self.kind == Kind::Text && end - self.start < 2_000 {
            return;
        }
        record(self.kind, self.start, end, self.label, self.arg);
    }
}

/// Starts a span of `kind`, if that's being kept, which ends when what this gives is dropped.
#[inline]
pub fn span(kind: Kind) -> Option<Span> {
    if !wants(kind) {
        return None;
    }
    Some(start(kind))
}

/// Starts a span of `kind` about `label`, if that's being kept.
#[inline]
pub fn span_about(kind: Kind, label: &str) -> Option<Span> {
    if !wants(kind) {
        return None;
    }
    let mut span = start(kind);
    span.label(label);
    Some(span)
}

/// Starts a span of `kind` about what `label` says, which is only worked out if it's being kept.
#[inline]
pub fn span_with(kind: Kind, label: impl FnOnce() -> String) -> Option<Span> {
    if !wants(kind) {
        return None;
    }
    let mut span = start(kind);
    span.label(&label());
    Some(span)
}

#[inline(never)]
fn start(kind: Kind) -> Span {
    Span {
        start: now(),
        kind,
        label: 0,
        arg: 0,
    }
}

struct Labels {
    numbers: HashMap<Box<str>, u32>,
    list: Vec<Box<str>>,
}

static LABELS: Mutex<Option<Labels>> = Mutex::new(None);

/// A number for `label`, the same each time it's asked for, which is what a span keeps. 0 is nothing.
pub fn intern(label: &str) -> u32 {
    if label.is_empty() {
        return 0;
    }
    let Ok(mut labels) = LABELS.lock() else {
        return 0;
    };
    let labels = labels.get_or_insert_with(|| Labels {
        numbers: HashMap::new(),
        list: vec!["".into()],
    });
    if let Some(number) = labels.numbers.get(label) {
        return *number;
    }
    let number = labels.list.len() as u32;
    labels.list.push(label.into());
    labels.numbers.insert(label.into(), number);
    number
}

/// The labels from number `from` on, for a viewer that has those before it.
pub fn labels_from(from: usize) -> Vec<String> {
    let Ok(labels) = LABELS.lock() else {
        return Vec::new();
    };
    match labels.as_ref() {
        Some(labels) => labels.list.iter().skip(from).map(|label| label.to_string()).collect(),
        None if from == 0 => vec![String::new()],
        None => Vec::new(),
    }
}

/// What a label number says.
pub fn label(number: u32) -> String {
    let Ok(labels) = LABELS.lock() else {
        return String::new();
    };
    labels
        .as_ref()
        .and_then(|labels| labels.list.get(number as usize))
        .map(|label| label.to_string())
        .unwrap_or_default()
}

struct Stacks {
    numbers: HashMap<Vec<u32>, u32>,
    list: Vec<Vec<u32>>,
}

static STACKS: Mutex<Option<Stacks>> = Mutex::new(None);

/// A number for a stack of labels, outermost first, the same each time. 0 is the empty stack.
pub fn intern_stack(frames: &[u32]) -> u32 {
    if frames.is_empty() {
        return 0;
    }
    let Ok(mut stacks) = STACKS.lock() else {
        return 0;
    };
    let stacks = stacks.get_or_insert_with(|| Stacks {
        numbers: HashMap::new(),
        list: vec![Vec::new()],
    });
    if let Some(number) = stacks.numbers.get(frames) {
        return *number;
    }
    let number = stacks.list.len() as u32;
    stacks.list.push(frames.to_vec());
    stacks.numbers.insert(frames.to_vec(), number);
    number
}

/// The stacks from number `from` on.
pub fn stacks_from(from: usize) -> Vec<Vec<u32>> {
    let Ok(stacks) = STACKS.lock() else {
        return Vec::new();
    };
    match stacks.as_ref() {
        Some(stacks) => stacks.list.iter().skip(from).cloned().collect(),
        None if from == 0 => vec![Vec::new()],
        None => Vec::new(),
    }
}

static THREAD_NAMES: Mutex<Vec<String>> = Mutex::new(Vec::new());

thread_local! {
    static THREAD: Cell<u8> = const { Cell::new(u8::MAX) };
}

fn thread_number() -> u8 {
    THREAD
        .try_with(|number| {
            if number.get() == u8::MAX {
                let Ok(mut names) = THREAD_NAMES.lock() else {
                    return 0;
                };
                let current = std::thread::current();
                names.push(current.name().unwrap_or("a thread").to_owned());
                number.set((names.len() - 1).min(254) as u8);
            }
            number.get()
        })
        .unwrap_or(0)
}

/// Each thread that's kept a span, in the order of their numbers.
pub fn threads() -> Vec<String> {
    THREAD_NAMES.lock().map(|names| names.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_kept_in_pieces_come_back_as_they_went_in() {
        let mut store = Store { pieces: VecDeque::new(), head: 0, len: 0, first: 0, pushed_since_trim: 0, long: VecDeque::new() };
        let rec = |at: u64| Rec {
            start: at * 10,
            // Every thousandth took longer than the packed form says, which is kept aside.
            end: at * 10 + if at % 1000 == 7 { 5_000_000_000 } else { at % 9 },
            label: at as u32,
            arg: 3,
            kind: 1,
            thread: 2,
        };
        for at in 0..(PIECE as u64 * 3 + 5) {
            store.push(rec(at));
        }
        for _ in 0..(PIECE + 9) {
            store.pop_front();
        }
        assert_eq!(store.first, PIECE as u64 + 9);
        assert_eq!(store.len, PIECE * 2 - 4);
        for at in 0..store.len {
            assert_eq!(store.get(at), rec(store.first + at as u64));
        }
        // Emptied, it starts again where it was, with its memory.
        while store.len > 0 {
            store.pop_front();
        }
        assert!(store.long.is_empty() && store.pieces.len() <= 1);
        store.push(rec(1));
        assert_eq!(store.get(0), rec(1));
    }

    #[test]
    fn keeps_spans_by_level() {
        set_level(Level::Light);
        let (_, from) = since(u64::MAX);
        {
            let _kept = span(Kind::Layout);
            let _left_out = span(Kind::Render);
        }
        set_level(Level::Detail);
        {
            let _kept = span_about(Kind::Render, "Row");
        }
        set_level(Level::Off);
        {
            let _left_out = span(Kind::Layout);
        }
        let (recs, _) = since(from);
        let mine: Vec<&Rec> = recs.iter().filter(|rec| rec.thread == thread_number()).collect();
        assert_eq!(mine.len(), 2, "{mine:?}");
        assert_eq!(mine[0].kind, Kind::Layout as u8);
        assert_eq!(mine[1].kind, Kind::Render as u8);
        assert_eq!(label(mine[1].label), "Row");
        assert!(mine[0].start <= mine[0].end);
    }

    #[test]
    fn stacks_are_interned() {
        let a = intern_stack(&[1, 2, 3]);
        assert_eq!(intern_stack(&[1, 2, 3]), a);
        assert_ne!(intern_stack(&[1, 2]), a);
        assert_eq!(intern_stack(&[]), 0);
    }
}
