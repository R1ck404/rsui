//! Looking at an app while it runs: what rendered and why, and where each frame's time went.
//!
//! It's off until [`App::set_inspecting`](crate::App::set_inspecting) turns it on, and then it keeps the last
//! [`KEPT_FOR`] of frames, so there's no starting to record before something goes wrong. Shown in the window, components
//! flash where they render, in another color when that render changed nothing, and a meter in the corner splits each frame
//! into its parts against the time there is for one. Ctrl+shift+I shows it and hides it, where that's allowed.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use rsui_core::Text;
use rsui_layout::Layout;
use rsui_render::{Color, Overlay, OverlayBox, OverlayText};
use rsui_runtime::{Cause, Handled, Rendered, Runtime, Trace};

/// How long frames are kept for.
pub const KEPT_FOR: Duration = Duration::from_secs(30);
/// The most frames kept, whatever the time, for an app drawing every frame there is.
const MOST_FRAMES: usize = 4000;
/// How long a flash takes to fade.
const FLASH_FOR: Duration = Duration::from_millis(700);
/// What a frame has to fit in, at 60 a second.
const FRAME_BUDGET: Duration = Duration::from_micros(16_667);
/// How many frames the meter shows.
const METER_FRAMES: usize = 60;
/// More flashes than this at once don't get names, as they'd only cover each other.
const MOST_NAMED_FLASHES: usize = 12;

const CHANGED: Color = Color::hex(0x2F81F7);
const CHANGED_NOTHING: Color = Color::hex(0xE3B341);
const JS: Color = Color::hex(0xD4A72C);
const LAYOUT: Color = Color::hex(0x8957E5);
const PAINT: Color = Color::hex(0x2DA44E);
const GPU: Color = Color::hex(0x388BFD);
const PANEL: Color = Color::rgba(0x1C, 0x21, 0x28, 0xE6);
const PANEL_TEXT: Color = Color::hex(0xE6EDF3);
/// What a viewer's pointing at, in the blue a browser's inspector has.
const HIGHLIGHT: Color = Color::rgba(0x6F, 0xA8, 0xDC, 0xA8);
const HIGHLIGHT_EDGE: Color = Color::hex(0x1A73E8);

/// Whether the inspector's allowed: 0 until someone says, which is as the build is, 1 for no and 2 for yes.
static ALLOWED: AtomicU8 = AtomicU8::new(0);
static PILL_ALLOWED: AtomicBool = AtomicBool::new(true);
static OPENER: Mutex<Option<Opener>> = Mutex::new(None);

/// What opens the inspector for an app, as F12 and the pill do: rsui-tsx opens its viewer, docked to the app's window.
pub type Opener = fn(&mut crate::App);

/// Has F12, ctrl+shift+I and the pill open the inspector with this, rather than draw what it keeps over the app.
pub fn set_opener(opener: Opener) {
    *OPENER.lock().expect("lock is not poisoned") = Some(opener);
}

pub(crate) fn opener() -> Option<Opener> {
    *OPENER.lock().expect("lock is not poisoned")
}

/// Whether a window can have the pill in its corner, where the inspector's allowed. `RSUI_PILL=0` says it can't too.
pub fn allow_pill(allowed: bool) {
    PILL_ALLOWED.store(allowed, Ordering::Relaxed);
}

pub(crate) fn pill_allowed() -> bool {
    PILL_ALLOWED.load(Ordering::Relaxed)
        && inspector_allowed()
        && std::env::var_os("RSUI_PILL").is_none_or(|value| value != "0")
}

/// Starts keeping where each frame's time goes, lightly, for a program that's being worked on, unless `RSUI_SPANS=0` says
/// not to or more is being kept already.
pub fn keep_spans_lightly() {
    if rsui_spans::level() == rsui_spans::Level::Off
        && std::env::var_os("RSUI_SPANS").is_none_or(|value| value != "0")
    {
        rsui_spans::set_level(rsui_spans::Level::Light);
    }
}
static SAVER: std::sync::Mutex<Option<TraceSaver>> = std::sync::Mutex::new(None);
/// How long what ctrl+shift+S did shows.
const TOLD_FOR: Duration = Duration::from_secs(4);

/// What saves what's been kept about an app, for ctrl+shift+S, and says where it went.
pub type TraceSaver = fn(&crate::App) -> Result<std::path::PathBuf, String>;

/// Lets ctrl+shift+S save what's kept, while the inspector's showing, with this.
pub fn set_trace_saver(saver: TraceSaver) {
    *SAVER.lock().expect("lock is not poisoned") = Some(saver);
}

pub(crate) fn trace_saver() -> Option<TraceSaver> {
    *SAVER.lock().expect("lock is not poisoned")
}
static FROM_START: AtomicBool = AtomicBool::new(false);

/// Has every app made from now on keep what happens from its first render, as an agent wants, for asking why later.
pub fn inspect_from_start(on: bool) {
    FROM_START.store(on, Ordering::Relaxed);
}

/// Whether an app being made now is inspected from the start, and if so whether that's shown. `RSUI_INSPECT=1` has it
/// shown, and `RSUI_INSPECT=keep` has it only kept.
pub(crate) fn wanted_from_start() -> Option<bool> {
    if let Some(asked) = std::env::var_os("RSUI_INSPECT") {
        if asked != "0" && !asked.is_empty() {
            return Some(asked != "keep");
        }
    }
    FROM_START.load(Ordering::Relaxed).then_some(false)
}

/// Lets ctrl+shift+I show the inspector in a window, for a program that's being worked on. A program handed to someone
/// shouldn't have it.
pub fn allow_inspector(allowed: bool) {
    ALLOWED.store(if allowed { 2 } else { 1 }, Ordering::Relaxed);
    if !allowed {
        rsui_spans::set_level(rsui_spans::Level::Off);
    }
}

/// Whether the inspector's allowed: where nothing's said, in a debug build only, as one of those is being worked on.
pub(crate) fn inspector_allowed() -> bool {
    match ALLOWED.load(Ordering::Relaxed) {
        0 => cfg!(debug_assertions),
        allowed => allowed == 2,
    }
}

/// Something another thread wants done with the app being looked at, on its own thread, as a viewer in another program
/// asks.
pub type LiveJob = Box<dyn FnOnce(&mut crate::App) + Send>;

type Waker = std::sync::Arc<dyn Fn() + Send + Sync>;

/// What's waiting to be done with the app being looked at, and how to wake it for that. The first window with a way to be
/// woken, where the inspector's allowed, is the one: the main window, as that's made first.
static LIVE_JOBS: Mutex<Vec<LiveJob>> = Mutex::new(Vec::new());
static LIVE_WAKER: Mutex<Option<Waker>> = Mutex::new(None);
/// Whether anything's in [`LIVE_JOBS`], for the app to see without taking the lock at every update.
static LIVE_WAITING: AtomicBool = AtomicBool::new(false);

/// Has `job` done with the app being looked at, the next time it updates, which this wakes it for. Until a window's open
/// there's no app to do it, and it waits for one.
pub fn run_in_app(job: LiveJob) {
    LIVE_JOBS.lock().expect("lock is not poisoned").push(job);
    LIVE_WAITING.store(true, Ordering::Release);
    let waker = LIVE_WAKER.lock().expect("lock is not poisoned").clone();
    if let Some(waker) = waker {
        waker();
    }
}

/// Makes the app with this waker the one jobs go to, if none is yet. Says whether it is.
pub(crate) fn claim_live(waker: Waker) -> bool {
    if !inspector_allowed() {
        return false;
    }
    let mut claimed = LIVE_WAKER.lock().expect("lock is not poisoned");
    if claimed.is_some() {
        return false;
    }
    *claimed = Some(waker.clone());
    drop(claimed);
    // What was asked before there was a window is done now.
    if LIVE_WAITING.load(Ordering::Acquire) {
        waker();
    }
    true
}

/// What's waiting to be done, if anything is.
pub(crate) fn take_live_jobs() -> Vec<LiveJob> {
    if !LIVE_WAITING.swap(false, Ordering::Acquire) {
        return Vec::new();
    }
    std::mem::take(&mut *LIVE_JOBS.lock().expect("lock is not poisoned"))
}

/// What happened in one frame: what came in, what rendered and why, and how long each part took.
#[derive(Debug, Clone, Default)]
pub struct FrameRecord {
    /// Counting from 1 since inspecting started.
    pub number: u64,
    /// How long after inspecting started it was drawn.
    pub at: Duration,
    /// What was carried out, as clicks and keys are.
    pub handled: Vec<Handled>,
    pub renders: Vec<Rendered>,
    /// Running JavaScript or Rust for handlers, rendering, and running effects.
    pub script: Duration,
    pub layout: Duration,
    /// Working out what to draw.
    pub paint: Duration,
    /// Handing it to the GPU.
    pub gpu: Duration,
    /// Waiting for the screen to take it, which isn't counted in [`total`](Self::total), as it's the screen's time, not
    /// the app's.
    pub present: Duration,
    /// When it was painted, in `rsui_spans`'s time, which says which of the spans kept are this frame's.
    pub painted: u64,
}

impl FrameRecord {
    pub fn total(&self) -> Duration {
        self.script + self.layout + self.paint + self.gpu
    }

    fn is_empty(&self) -> bool {
        self.handled.is_empty() && self.renders.is_empty() && self.total().is_zero()
    }

    /// The frame as a story, on a few lines: what came in, what rendered and why, and where the time went.
    pub fn story(&self) -> String {
        let mut out = format!("frame {} at {:.2}s:", self.number, self.at.as_secs_f64());
        let _ = write!(
            out,
            " {} (script {}, layout {}, paint {}, gpu {})",
            ms(self.total()),
            ms(self.script),
            ms(self.layout),
            ms(self.paint),
            ms(self.gpu)
        );
        if self.total() > FRAME_BUDGET {
            out.push_str(", over the 16.7 ms there is for a frame");
        }
        if self.present >= Duration::from_micros(100) {
            let _ = write!(out, ", then {} waiting for the screen", ms(self.present));
        }
        for handled in &self.handled {
            let _ = write!(
                out,
                "\n  {} on #{} ran {}for {}{}",
                handled.action.name(),
                handled.key,
                handled
                    .note
                    .as_ref()
                    .map(|note| format!("{note} "))
                    .unwrap_or_default(),
                ms(handled.took),
                if handled.rejected {
                    ", and was turned down"
                } else {
                    ""
                }
            );
        }
        let wasted = self.renders.iter().filter(|render| !render.changed).count();
        if !self.renders.is_empty() {
            let _ = write!(
                out,
                "\n  {} rendered{}",
                count(self.renders.len(), "component"),
                if wasted > 0 {
                    format!(", {wasted} of them changing nothing")
                } else {
                    String::new()
                }
            );
        }
        // Only what started a render is worth saying why about: the rest rendered because of it.
        for render in &self.renders {
            if render.cause == Cause::Props && render.why.is_empty() && render.changed {
                continue;
            }
            let _ = write!(out, "\n    {}", describe_render(render));
        }
        out
    }
}

/// One render, in a line or two.
pub fn describe_render(render: &Rendered) -> String {
    let name = render.name.as_ref().map_or("a component", Text::as_str);
    let mut out = format!("{name} ({}", ms(render.took));
    if !render.changed {
        out.push_str(", changed nothing");
    }
    out.push(')');
    if render.why.is_empty() {
        out.push_str(match render.cause {
            Cause::Mounted => ": it's new",
            Cause::Props => ": what's around it gave it different props",
            Cause::State => ": its state changed",
        });
    } else {
        out.push_str(": ");
        out.push_str(&render.why.join(" "));
    }
    out
}

/// What the frames kept come to: the slowest, which components took the most time, and why renders that changed nothing
/// happened, which is where time is most often won back.
pub fn summary<'a>(frames: impl Iterator<Item = &'a FrameRecord>) -> String {
    let frames: Vec<&FrameRecord> = frames.collect();
    let busy: Vec<&&FrameRecord> = frames
        .iter()
        .filter(|frame| !frame.renders.is_empty() || !frame.handled.is_empty())
        .collect();
    if busy.is_empty() {
        return "nothing's rendered or been carried out in what's kept".to_owned();
    }
    let renders: Vec<&Rendered> = busy.iter().flat_map(|frame| frame.renders.iter()).collect();
    let wasted: Vec<&&Rendered> = renders.iter().filter(|render| !render.changed).collect();
    let over = frames
        .iter()
        .filter(|frame| frame.total() > FRAME_BUDGET)
        .count();
    let mut out = format!(
        "{} with something happening, {} drawn in all, {} over the 16.7 ms there is for one. {} rendered, {} of them changing nothing.",
        count(busy.len(), "frame"),
        frames.len(),
        over,
        count(renders.len(), "component"),
        wasted.len()
    );

    let mut slowest: Vec<&&FrameRecord> = frames.iter().collect();
    slowest.sort_by_key(|frame| std::cmp::Reverse(frame.total()));
    out.push_str("\n\nSlowest frames:");
    for frame in slowest.iter().take(3) {
        let what = frame
            .handled
            .first()
            .map(|handled| format!(", after {} on #{}", handled.action.name(), handled.key))
            .unwrap_or_default();
        let _ = write!(
            out,
            "\n  frame {}: {} (script {}, layout {}, paint {}, gpu {}){what}",
            frame.number,
            ms(frame.total()),
            ms(frame.script),
            ms(frame.layout),
            ms(frame.paint),
            ms(frame.gpu)
        );
    }

    // By name, as that's what's fixed in the code: every row of a list is one line here.
    let mut by_name: Vec<(String, usize, usize, Duration)> = Vec::new();
    for render in &renders {
        let name = render.name.as_ref().map_or("a component", Text::as_str);
        let at = match by_name.iter().position(|(kept, ..)| kept == name) {
            Some(at) => at,
            None => {
                by_name.push((name.to_owned(), 0, 0, Duration::ZERO));
                by_name.len() - 1
            }
        };
        let entry = &mut by_name[at];
        entry.1 += 1;
        entry.2 += usize::from(!render.changed);
        entry.3 += render.own;
    }
    by_name.sort_by_key(|(_, _, _, own)| std::cmp::Reverse(*own));
    out.push_str("\n\nComponents by the time they took themselves, leaving out what they rendered inside them:");
    for (name, renders, wasted, own) in by_name.iter().take(10) {
        let _ = write!(
            out,
            "\n  {name}: {} over {}",
            ms(*own),
            count(*renders, "render")
        );
        if *wasted > 0 {
            let _ = write!(out, ", {wasted} changing nothing");
        }
    }

    // The first thing said about each is the reason, and the same reason comes up again and again.
    let mut reasons: Vec<(String, usize)> = Vec::new();
    for render in &wasted {
        let name = render.name.as_ref().map_or("a component", Text::as_str);
        let reason = match render.why.first() {
            Some(why) => format!("{name}: {why}"),
            None => format!(
                "{name}: {}",
                describe_render(render)
                    .split_once(": ")
                    .map_or("", |(_, why)| why)
            ),
        };
        match reasons.iter_mut().find(|(kept, _)| *kept == reason) {
            Some((_, times)) => *times += 1,
            None => reasons.push((reason, 1)),
        }
    }
    if !reasons.is_empty() {
        reasons.sort_by_key(|(_, times)| std::cmp::Reverse(*times));
        out.push_str("\n\nWhy renders that changed nothing happened, most often first:");
        for (reason, times) in reasons.iter().take(5) {
            let _ = write!(out, "\n  {}× {reason}", times);
        }
    }
    out
}

fn ms(duration: Duration) -> String {
    let ms = duration.as_secs_f64() * 1000.0;
    if ms < 0.1 && !duration.is_zero() {
        "<0.1 ms".to_owned()
    } else {
        format!("{ms:.1} ms")
    }
}

fn count(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

struct Flash {
    component: (u32, u32),
    path: Vec<u32>,
    name: Option<Text>,
    changed: bool,
    at: Instant,
    /// How many times it's rendered while this one's been showing.
    times: u32,
}

pub(crate) struct Inspector {
    /// Whether it's drawn in the window, rather than only kept.
    pub(crate) showing: bool,
    started: Instant,
    frames: VecDeque<FrameRecord>,
    current: FrameRecord,
    next_number: u64,
    flashes: Vec<Flash>,
    pub(crate) overlay: Overlay,
    /// Whether it wants drawing again for itself. Frames drawn only for that aren't kept, so they don't count as the app's.
    pub(crate) redraw: bool,
    /// Whether the last frame was kept, for the GPU's time to go with it.
    kept_last: bool,
    /// What's been done that's worth saying, such as where a trace was saved, and since when.
    told: Option<(String, Instant)>,
    /// What a viewer's pointing at, and what to call it, which is shown whether or not the rest is.
    pub(crate) highlight: Option<(Vec<u32>, String)>,
    /// Where time went, as a viewer worked it out: each element, how hot it is from 0 to 1, and what to say on it.
    pub(crate) heat: Vec<(Vec<u32>, f32, String)>,
}

impl Inspector {
    /// What the frames kept take, for measuring memory: how many, their renders and handled actions, and about how many bytes.
    pub(crate) fn footprint(&self) -> String {
        let (mut renders, mut handled, mut bytes, mut why, mut paths) = (0, 0, 0, 0, 0);
        bytes += self.frames.capacity() * size_of::<FrameRecord>();
        for frame in self.frames.iter().chain([&self.current]) {
            renders += frame.renders.len();
            handled += frame.handled.len();
            bytes += frame.renders.capacity() * size_of::<Rendered>() + frame.handled.capacity() * size_of::<Handled>();
            for render in &frame.renders {
                why += render.why.capacity() * size_of::<Text>();
                paths += render.path.as_ref().map_or(0, |path| path.capacity() * 4);
                bytes += render.name.as_ref().map_or(0, |name| if name.len() > 15 { name.len() } else { 0 });
            }
        }
        format!(
            "inspector: {} frames, {renders} renders, {handled} handled, {bytes} B of records, {why} B of why, {paths} B of paths",
            self.frames.len()
        )
    }

    pub(crate) fn new() -> Self {
        Self {
            showing: false,
            started: Instant::now(),
            frames: VecDeque::new(),
            current: FrameRecord::default(),
            next_number: 1,
            flashes: Vec::new(),
            overlay: Overlay::default(),
            redraw: false,
            kept_last: false,
            told: None,
            highlight: None,
            heat: Vec::new(),
        }
    }

    pub(crate) fn frames(&self) -> &VecDeque<FrameRecord> {
        &self.frames
    }

    /// Takes in what the runtime has kept since this last asked.
    pub(crate) fn take_from(&mut self, runtime: &mut Runtime) {
        let Trace {
            renders,
            handled,
            flushing,
        } = runtime.take_trace();
        let now = Instant::now();
        for render in renders.iter().filter(|_| self.showing) {
            let Some(path) = &render.path else {
                continue;
            };
            let component = (render.component, render.generation);
            match self
                .flashes
                .iter_mut()
                .find(|flash| flash.component == component)
            {
                Some(flash) => {
                    flash.path.clone_from(path);
                    flash.changed = render.changed;
                    flash.at = now;
                    flash.times += 1;
                }
                None => self.flashes.push(Flash {
                    component,
                    path: path.clone(),
                    name: render.name.clone(),
                    changed: render.changed,
                    at: now,
                    times: 1,
                }),
            }
        }
        self.current.script +=
            flushing + handled.iter().map(|handled| handled.took).sum::<Duration>();
        // A render that changed nothing changes nothing on screen either, but its flash has to be drawn.
        if self.showing && !renders.is_empty() {
            self.redraw = true;
        }
        self.current.renders.extend(renders);
        self.current.handled.extend(handled);
    }

    pub(crate) fn layout_took(&mut self, took: Duration) {
        self.current.layout += took;
    }

    pub(crate) fn paint_took(&mut self, took: Duration) {
        self.current.paint += took;
    }

    /// Ends the frame that's been drawn, which is kept if the app wanted it, rather than only this.
    pub(crate) fn frame_done(&mut self, app_wanted: bool) {
        let now = Instant::now();
        let wanted =
            app_wanted || !self.current.handled.is_empty() || !self.current.renders.is_empty();
        self.kept_last = wanted && !self.current.is_empty();
        if !wanted {
            // What it took was this drawing for itself, which isn't what's being looked at.
            self.current = FrameRecord::default();
        } else if !self.current.is_empty() {
            let mut frame = std::mem::take(&mut self.current);
            frame.number = self.next_number;
            frame.at = now - self.started;
            frame.painted = rsui_spans::nanos(now);
            self.next_number += 1;
            self.frames.push_back(frame);
        }
        let oldest = (now - self.started).saturating_sub(KEPT_FOR);
        while self.frames.len() > MOST_FRAMES
            || self.frames.front().is_some_and(|frame| frame.at < oldest)
        {
            self.frames.pop_front();
        }
    }

    /// The GPU's time, and how long the screen took to take it, go with the frame it drew, which is over by then.
    pub(crate) fn gpu_took(&mut self, took: Duration, present: Duration) {
        if !self.kept_last {
            return;
        }
        if let Some(frame) = self.frames.back_mut() {
            frame.gpu += took;
            frame.present += present;
        }
    }

    /// Whether something's still fading, which needs frames for it to.
    pub(crate) fn flashing(&self) -> bool {
        self.showing && (!self.flashes.is_empty() || self.told.is_some())
    }

    /// Says something in the window for a few seconds.
    pub(crate) fn tell(&mut self, text: String) {
        self.told = Some((text, Instant::now()));
    }

    /// What's drawn over the app this frame.
    pub(crate) fn build_overlay(
        &mut self,
        runtime: &Runtime,
        layout: &Layout,
        scale: f32,
        size: (u32, u32),
        pointer: Option<(f32, f32)>,
    ) {
        self.overlay.clear();
        self.add_heat(layout, scale);
        self.add_highlight(layout, scale);
        if !self.showing {
            return;
        }
        let now = Instant::now();
        self.flashes
            .retain(|flash| now.duration_since(flash.at) < FLASH_FOR);
        let named = self.flashes.len() <= MOST_NAMED_FLASHES;
        for flash in &self.flashes {
            let Some(rect) = layout.rect(&flash.path) else {
                continue;
            };
            let left = 1.0 - now.duration_since(flash.at).as_secs_f32() / FLASH_FOR.as_secs_f32();
            let base = if flash.changed {
                CHANGED
            } else {
                CHANGED_NOTHING
            };
            let rect = [
                rect.x * scale,
                rect.y * scale,
                rect.width * scale,
                rect.height * scale,
            ];
            self.overlay.boxes.push(OverlayBox {
                rect,
                color: faded(base, left),
                outline: 2.0 * scale,
                radius: 0.0,
            });
            if named {
                if let Some(name) = &flash.name {
                    let mut label = name.to_string();
                    if flash.times > 1 {
                        let _ = write!(label, " ×{}", flash.times);
                    }
                    self.overlay.texts.push(OverlayText {
                        text: label,
                        left: rect[0],
                        top: (rect[1] - 16.0 * scale).max(0.0),
                        size: 10.0,
                        widest: 200.0,
                        color: Color::WHITE,
                        background: faded(base, left.max(0.4)),
                    });
                }
            }
        }
        self.add_meter(scale, size);
        if self
            .told
            .as_ref()
            .is_some_and(|(_, since)| now.duration_since(*since) >= TOLD_FOR)
        {
            self.told = None;
        }
        if let Some((text, _)) = &self.told {
            self.overlay.texts.push(OverlayText {
                text: text.clone(),
                left: 8.0 * scale,
                top: size.1 as f32 - 40.0 * scale,
                size: 12.0,
                widest: 600.0,
                color: PANEL_TEXT,
                background: PANEL,
            });
        }
        if let Some((x, y)) = pointer {
            if let Some(note) = self.note_at(runtime, layout, x / scale, y / scale) {
                self.overlay.texts.push(OverlayText {
                    text: note,
                    left: x + 16.0 * scale,
                    top: y + 20.0 * scale,
                    size: 11.0,
                    widest: 420.0,
                    color: PANEL_TEXT,
                    background: PANEL,
                });
            }
        }
    }

    /// Where time went, drawn over the app: each element a viewer said was hot, from blue for a little to red for the most,
    /// with its share written on the hottest.
    fn add_heat(&mut self, layout: &Layout, scale: f32) {
        let mut labelled = 0;
        for (path, heat, label) in &self.heat {
            let Some(rect) = layout.rect(path) else {
                continue;
            };
            let rect = [rect.x * scale, rect.y * scale, rect.width * scale, rect.height * scale];
            if rect[2] < 1.0 || rect[3] < 1.0 {
                continue;
            }
            let color = heat_color(*heat);
            self.overlay.boxes.push(OverlayBox {
                rect,
                color: faded(color, 0.18 + 0.42 * heat.clamp(0.0, 1.0)),
                outline: 0.0,
                radius: 0.0,
            });
            self.overlay.boxes.push(OverlayBox {
                rect,
                color: faded(color, 0.9),
                outline: scale.max(1.0),
                radius: 0.0,
            });
            if !label.is_empty() && labelled < MOST_NAMED_FLASHES && rect[3] >= 14.0 * scale {
                labelled += 1;
                self.overlay.texts.push(OverlayText {
                    text: label.clone(),
                    left: rect[0] + 2.0 * scale,
                    top: rect[1] + 2.0 * scale,
                    size: 10.0,
                    widest: (rect[2] / scale).max(60.0),
                    color: Color::WHITE,
                    background: faded(color, 0.92),
                });
            }
        }
    }

    /// What a viewer's pointing at, as a browser's inspector shows it: filled in, with its name and size by it.
    fn add_highlight(&mut self, layout: &Layout, scale: f32) {
        let Some((path, name)) = &self.highlight else {
            return;
        };
        let Some(rect) = layout.rect(path) else {
            return;
        };
        let rect = [rect.x * scale, rect.y * scale, rect.width * scale, rect.height * scale];
        self.overlay.boxes.push(OverlayBox {
            rect,
            color: HIGHLIGHT,
            outline: 0.0,
            radius: 0.0,
        });
        self.overlay.boxes.push(OverlayBox {
            rect,
            color: HIGHLIGHT_EDGE,
            outline: 1.0 * scale,
            radius: 0.0,
        });
        let label = format!(
            "{name}  {} \u{d7} {}",
            (rect[2] / scale).round(),
            (rect[3] / scale).round()
        );
        let below = rect[1] < 24.0 * scale;
        self.overlay.texts.push(OverlayText {
            text: label,
            left: rect[0],
            top: if below {
                rect[1] + rect[3] + 4.0 * scale
            } else {
                rect[1] - 22.0 * scale
            },
            size: 11.0,
            widest: 400.0,
            color: Color::hex(0x202124),
            background: Color::WHITE,
        });
    }

    /// The last [`METER_FRAMES`] frames as bars in the top right corner, each split into its parts, with a line where the
    /// time there is for a frame runs out.
    fn add_meter(&mut self, scale: f32, size: (u32, u32)) {
        let (bar, gap, height) = (3.0 * scale, 1.0 * scale, 40.0 * scale);
        let width = METER_FRAMES as f32 * (bar + gap) + gap;
        let left = size.0 as f32 - width - 8.0 * scale;
        let top = 8.0 * scale;
        self.overlay.boxes.push(OverlayBox {
            rect: [left, top, width, height],
            color: PANEL,
            outline: 0.0,
            radius: 0.0,
        });
        // Two frames' worth fits, so the budget is halfway up.
        let per_ms = height / (2.0 * FRAME_BUDGET.as_secs_f32() * 1000.0);
        let shown: Vec<&FrameRecord> = self.frames.iter().rev().take(METER_FRAMES).collect();
        for (at, frame) in shown.iter().rev().enumerate() {
            let x = left + gap + at as f32 * (bar + gap);
            let mut y = top + height;
            for (took, color) in [
                (frame.script, JS),
                (frame.layout, LAYOUT),
                (frame.paint, PAINT),
                (frame.gpu, GPU),
            ] {
                let tall = (took.as_secs_f32() * 1000.0 * per_ms).min(y - top);
                if tall > 0.0 {
                    y -= tall;
                    self.overlay.boxes.push(OverlayBox {
                        rect: [x, y, bar, tall.max(1.0)],
                        color,
                        outline: 0.0,
                        radius: 0.0,
                    });
                }
            }
        }
        self.overlay.boxes.push(OverlayBox {
            rect: [left, top + height / 2.0, width, scale.max(1.0)],
            color: faded(Color::WHITE, 0.5),
            outline: 0.0,
            radius: 0.0,
        });
        let text = match self.frames.back() {
            Some(frame) => format!(
                "script {}  layout {}  paint {}  gpu {}",
                ms(frame.script),
                ms(frame.layout),
                ms(frame.paint),
                ms(frame.gpu)
            ),
            None => "nothing's happened yet".to_owned(),
        };
        self.overlay.texts.push(OverlayText {
            text,
            left,
            top: top + height + 2.0 * scale,
            size: 10.0,
            widest: width / scale,
            color: PANEL_TEXT,
            background: PANEL,
        });
    }

    /// What to say about what's under the pointer: which component made it, how often that's rendered, and why it last did.
    fn note_at(&self, runtime: &Runtime, layout: &Layout, x: f32, y: f32) -> Option<String> {
        let path = layout.hit_test(x, y)?;
        let component = runtime.component_at(&path);
        Some(
            self.why(component)
                .unwrap_or_else(|| "This hasn't rendered since inspecting started.".to_owned()),
        )
    }

    /// What's known about a component's renders: how many there have been, and why the last one happened.
    pub(crate) fn why(&self, component: (u32, u32)) -> Option<String> {
        let mine = |render: &&Rendered| (render.component, render.generation) == component;
        let all: Vec<&Rendered> = self
            .frames
            .iter()
            .flat_map(|frame| frame.renders.iter())
            .filter(mine)
            .collect();
        let last = all.last()?;
        let wasted = all.iter().filter(|render| !render.changed).count();
        let mut out = format!(
            "{} rendered {} in the last {} s",
            last.name.as_ref().map_or("This component", Text::as_str),
            match all.len() {
                1 => "once".to_owned(),
                n => format!("{n} times"),
            },
            KEPT_FOR.as_secs()
        );
        if wasted > 0 {
            let _ = write!(out, ", {wasted} of them changing nothing");
        }
        out.push_str(".\nLast: ");
        out.push_str(&describe_render(last));
        Some(out)
    }
}

/// From blue for cool through yellow to red for hot.
fn heat_color(heat: f32) -> Color {
    let stops: [(f32, [f32; 3]); 4] = [
        (0.0, [0x38 as f32, 0x8B as f32, 0xFD as f32]),
        (0.45, [0x3F as f32, 0xB9 as f32, 0x50 as f32]),
        (0.7, [0xE3 as f32, 0xB3 as f32, 0x41 as f32]),
        (1.0, [0xF8 as f32, 0x51 as f32, 0x49 as f32]),
    ];
    let heat = heat.clamp(0.0, 1.0);
    let after = stops.iter().position(|(at, _)| *at >= heat).unwrap_or(stops.len() - 1).max(1);
    let (from, to) = (stops[after - 1], stops[after]);
    let along = if to.0 > from.0 { (heat - from.0) / (to.0 - from.0) } else { 1.0 };
    let mix = |at: usize| (from.1[at] + (to.1[at] - from.1[at]) * along).round() as u8;
    Color::rgba(mix(0), mix(1), mix(2), 0xFF)
}

fn faded(color: Color, amount: f32) -> Color {
    Color {
        a: (f32::from(color.a) * amount.clamp(0.0, 1.0)).round() as u8,
        ..color
    }
}
