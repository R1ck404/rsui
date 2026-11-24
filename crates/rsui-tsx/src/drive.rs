//! Using an app from the command line, with no window, the way a person would: `rsui-tsx --do "click #add; look" app.tsx`.
//!
//! Each step is real input where things really are, so what's disabled or covered up stays that way. It's one command
//! and plain text, with nothing to set up, and it fails with the reason and what was there at the time. A list of steps
//! kept in a file is a test.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rsui::access::AccessCommand;
use rsui::core::{ActionKind, Node, Patch, PropValue, diff};
use rsui::headless::{Query, describe_patch, only_changes_styling, outline, parse_path};
use rsui::render::Renderer;
use rsui::replay::Input;
use rsui::{App, DropPhase, Frame, KeyCode, KeyPress, Modifiers, PointerButton, WindowId};

use crate::bridge;

/// How long a step waits for what it started, before going on to the next one. `wait` is for anything longer.
const STEP_SETTLES_FOR: Duration = Duration::from_secs(1);

/// How long a step waits for what it started: [`STEP_SETTLES_FOR`], unless `RSUI_DO_SETTLE_MS` says, as 0 for a step that only
/// passes once, so that what a click sets moving can be watched from the start with `shot`s, rather than once it's all over
/// (Mantine's `Transition` runs on timers, which a step otherwise waits out).
fn step_settles_for() -> Duration {
    let said = SETTLE_SAID.load(std::sync::atomic::Ordering::Relaxed);
    if said != u64::MAX {
        return Duration::from_millis(said);
    }
    static FOR: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *FOR.get_or_init(|| {
        std::env::var("RSUI_DO_SETTLE_MS")
            .ok()
            .and_then(|ms| ms.parse().ok())
            .map_or(STEP_SETTLES_FOR, Duration::from_millis)
    })
}
/// What a `settle MS` step said steps wait for, in milliseconds, or `u64::MAX` for what [`step_settles_for`] says otherwise:
/// a step before `frames` can say 0, so that steps pass once and what it sets moving is watched from the start.
static SETTLE_SAID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

/// Finds a page's element by a CSS selector, in the page, and says where its middle is, as `x y width height`, or `none`, or
/// `scrolled` when its middle, which is what's clicked, was outside the window and has been scrolled into view, so that it's to be asked again once that's laid
/// out.
const POINT_OF: &str = "((selector, scroll) => { const one = document.querySelector(selector); if (!one) return 'none'; \
    const r = one.getBoundingClientRect(); \
    if (scroll && (r.top + r.height / 2 < 0 || r.left + r.width / 2 < 0 || r.top + r.height / 2 >= innerHeight || r.left + r.width / 2 >= innerWidth)) { one.scrollIntoView({ block: 'center', inline: 'center' }); return 'scrolled'; } \
    return (r.left + r.width / 2) + ' ' + (r.top + r.height / 2) + ' ' + r.width + ' ' + r.height; })";

/// How long `wait` looks for text, unless it says.
const WAIT_LOOKS_FOR: Duration = Duration::from_secs(10);
/// A timer due this soon is part of what's going on now, such as something that was put off to the next moment. One
/// that's further off is for later, and a step doesn't hold things up for it, or an app with a clock would never settle.
const TIMER_COUNTS_WITHIN: Duration = Duration::from_millis(100);
/// How many lines of changes a step shows.
const MOST_CHANGES: usize = 30;
/// How many frames `frames` tells the story of, the latest ones.
const MOST_FRAMES: usize = 20;

/// For measuring frames with no window: `RSUI_FRAME_LOG=<file>` writes a line for each pass that drew, and for
/// each step, as `what start end extra`, in microseconds from when the log was opened. `RSUI_FRAME_RENDER=1` has each of
/// those passes draw with the GPU, offscreen, as a window would, rather than only working out the scene, and
/// `RSUI_DO_HZ=144` has `wait` pass at that rate, as a screen would pace a window, rather than as soon as it can.
pub(crate) mod frame_log {
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    pub(crate) struct Log {
        file: Mutex<std::fs::File>,
        opened: Instant,
        pub(crate) render: bool,
        pub(crate) hz: Option<f64>,
    }

    pub(crate) fn get() -> Option<&'static Log> {
        static LOG: OnceLock<Option<Log>> = OnceLock::new();
        LOG.get_or_init(|| {
            let path = std::env::var_os("RSUI_FRAME_LOG")?;
            Some(Log {
                file: Mutex::new(std::fs::File::create(path).ok()?),
                opened: Instant::now(),
                render: std::env::var_os("RSUI_FRAME_RENDER").is_some_and(|value| value != "0"),
                hz: std::env::var("RSUI_DO_HZ").ok().and_then(|hz| hz.parse().ok()),
            })
        })
        .as_ref()
    }

    /// The time between frames that `RSUI_DO_HZ` asks for.
    pub(crate) fn period() -> Option<std::time::Duration> {
        get()?.hz.map(|hz| std::time::Duration::from_secs_f64(1.0 / hz))
    }

    pub(crate) fn note(what: &str, start: Instant, extra: std::fmt::Arguments) {
        if let Some(log) = get() {
            let at = |when: Instant| when.saturating_duration_since(log.opened).as_micros();
            if let Ok(mut file) = log.file.lock() {
                let _ = writeln!(file, "{what} {} {} {extra}", at(start), at(Instant::now()));
            }
        }
    }
}

/// How far everything's scrolled down, added up, for the frame log to say when scrolling stopped moving.
fn scrolled_sum(node: &Node) -> f64 {
    let own = match node.props.get("scroll_y") {
        Some(PropValue::Float(y)) => *y,
        Some(PropValue::Int(y)) => *y as f64,
        _ => 0.0,
    };
    own + node.children.iter().map(scrolled_sum).sum::<f64>()
}

/// A window of the app.
pub(crate) struct Open {
    pub(crate) id: u32,
    pub(crate) title: String,
    modal: bool,
    pub(crate) app: App,
    /// What's been reported so far, or nothing if it hasn't said it's opened yet.
    seen: Option<Node>,
    renderer: Option<Renderer>,
    pub(crate) size: (u32, u32),
    /// When what [`App::next_wake`] last said was coming is due, as a window would set a timer for it. What it says is from
    /// when things last moved on, which only waking moves on, so it says the same until then, and is only ever nothing to
    /// wait for once that time has come.
    wake_due: Option<Instant>,
}

impl Open {
    /// Draws it as it is now, without reading the pixels back.
    fn render_frame(&mut self) -> Result<(), String> {
        if self.renderer.is_none() {
            // `RSUI_DO_DRAW=hw` draws on the GPU itself, as a window does, rather than in software, for what the driver keeps.
            let hardware = std::env::var_os("RSUI_DO_DRAW").is_some_and(|value| value == "hw");
            let made = if hardware {
                Renderer::offscreen_on_hardware(self.size)
            } else {
                Renderer::offscreen(self.size)
            }
            .map_err(|error| error.to_string())?;
            self.renderer = Some(made);
        }
        let renderer = self.renderer.as_mut().expect("that's just been made");
        self.app
            .render(renderer)
            .map_err(|error| error.to_string())?;
        // Waiting for the GPU to finish, as showing a frame in a window does: offscreen, nothing else holds the frames back,
        // and the driver would queue up hundreds of them.
        renderer
            .read_pixels()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Draws it as it is now, and gives the pixels.
    pub(crate) fn draw(&mut self) -> Result<Vec<u8>, String> {
        self.app.wait_for_images(Duration::from_secs(5));
        self.app.update();
        if self.renderer.is_none() {
            let made = Renderer::offscreen(self.size).map_err(|error| error.to_string())?;
            self.renderer = Some(made);
        }
        let renderer = self.renderer.as_mut().expect("that's just been made");
        self.app
            .render(renderer)
            .map_err(|error| error.to_string())?;
        renderer.read_pixels().map_err(|error| error.to_string())
    }
}

/// How long `wait` sleeps between passes when nothing's due, in milliseconds: 16, about a 60 Hz screen's frame, unless
/// `RSUI_DO_FRAME_MS` says, as 7 for a 144 Hz one.
fn frame_nap() -> u64 {
    static NAP: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *NAP.get_or_init(|| {
        std::env::var("RSUI_DO_FRAME_MS")
            .ok()
            .and_then(|said| said.parse().ok())
            .unwrap_or(16)
    })
}

/// Whether `RSUI_DO_DRAW` asks for each frame to be drawn.
fn draws_frames() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RSUI_DO_DRAW").is_some_and(|value| value != "0"))
}

/// All the windows of an app, with what would be asked of the system done here instead.
pub(crate) struct Desk {
    /// With the main one first. Nothing's here once it's closed.
    pub(crate) windows: Vec<Open>,
    /// The one that steps are for.
    pub(crate) current: u32,
    notes: Vec<String>,
    default_size: (u32, u32),
    /// When time stopped for what's moving, if it has, as it does while waiting for everything else to finish, so that a
    /// step leaves things partway, as they'd be in a window.
    still: Option<Instant>,
    /// The last frame of each window that `frames` has told about.
    frames_told: Vec<(u32, u64)>,
    /// Where the pointer last went, for the steps that go on from there (`downat`, `upat` and `moveat` with no place).
    pointer: (f32, f32),
    /// Whether things move, as `animations on` and `off` say, in every window, those opened later too. Off unless a step says,
    /// so that what a run sees, and its shots, are the same each time.
    animations: bool,
    /// Whether something would have moved while nothing did, or the page's CSS has rules for when nothing moves, and whether
    /// things have been made to move at any point: see [`Desk::hint_motion`].
    motion_skipped: bool,
    animations_ever: bool,
}

impl Desk {
    /// Loads an app, and gets it going. Nothing it does can bring up a real dialog or open anything.
    pub(crate) fn open(
        path: &Path,
        stack_limit: usize,
        default_size: (u32, u32),
    ) -> Result<Self, String> {
        bridge::answer_dialogs_here();
        // With no window, nothing's kept of where time goes unless `RSUI_SPANS` asks, as a measurement of what keeping it
        // costs does: `light` as a window being worked on keeps, `detail` as while an inspector's looking.
        match std::env::var("RSUI_SPANS").as_deref() {
            Ok("light" | "1") => rsui_spans::set_level(rsui_spans::Level::Light),
            Ok("detail" | "2") => rsui_spans::set_level(rsui_spans::Level::Detail),
            _ => {}
        }
        // A logpoint has the code taken in again, which keeps each component's state, as `--agent` and a watched window do.
        bridge::keep_state_across_reloads();
        // What comes in is kept, with the clock and where random numbers start, so it can be replayed as a window's is.
        crate::replay::begin()?;
        let entry = crate::load(path, stack_limit)?;
        let text = rsui::default_text_engine().map_err(|error| error.to_string())?;
        // As big as it would be in a window, if it says.
        let size = bridge::wanted_size().unwrap_or(default_size);
        let title = entry.title().unwrap_or_else(|| crate::file_title(path));
        let mut app = App::new(crate::root, entry, text, size, 1.0);
        // Paced as a screen at `RSUI_DO_HZ` would pace a window, for what fits in a frame.
        if let Some(period) = frame_log::period() {
            app.set_frame_time(period);
        }
        // Told how big it is from the start, as a window on a screen is when it's made, so that a page's `innerWidth` is what
        // it's laid out in rather than 800 by 600 until a `size` step.
        let _ = app.feed(&Input::Resize { width: size.0, height: size.1, scale: 1.0 });
        app.record_inputs();
        if let Some(plan) = crate::replay::take_plan() {
            let given = crate::replay::play(&mut app, &plan);
            println!("  replayed {given} inputs");
        }
        Ok(Self {
            windows: vec![Open {
                id: WindowId::MAIN.0,
                title,
                modal: false,
                seen: Some(app.runtime().root().clone()),
                app,
                renderer: None,
                size,
                wake_due: None,
            }],
            current: WindowId::MAIN.0,
            notes: Vec::new(),
            default_size,
            still: None,
            frames_told: Vec::new(),
            pointer: (0.0, 0.0),
            animations: false,
            motion_skipped: false,
            animations_ever: false,
        })
    }

    /// Has things move, or go straight to where they're going, in every window, as `animations on` and `off` say, at any step:
    /// a run can settle the app first and then watch its transitions. The page's code is told too, as CSS that asks
    /// `prefers-reduced-motion` is worked out from it.
    pub(crate) fn set_animations(&mut self, on: bool) {
        self.animations = on;
        self.animations_ever |= on;
        for one in &mut self.windows {
            one.app.set_animations(on);
        }
        bridge::motion_changed(on);
    }

    /// Notes whether something would have moved but didn't, as animations are off, or the page's CSS has rules for when
    /// nothing moves, for [`Desk::hint_motion`]. After each step, as what's noted is only kept until it's asked.
    pub(crate) fn note_motion(&mut self) {
        let mut skipped = bridge::take_motion_held();
        for one in &mut self.windows {
            skipped |= one.app.runtime_mut().motion_skipped();
        }
        self.motion_skipped |= skipped && !self.animations;
    }

    /// Says once, on stderr, at the end of a run that never had things move, that something would have, and how to see it:
    /// a run that turns them on knows already.
    pub(crate) fn hint_motion(&mut self) {
        self.note_motion();
        if self.motion_skipped && !self.animations_ever {
            self.motion_skipped = false;
            eprintln!(
                "rsui-tsx: the app has transitions or animations, which go straight to their end in --do; a step `animations on`, at any point, plays them"
            );
        }
    }

    /// Applies what's waiting everywhere, and does what's been asked of the windows. Says whether anything happened.
    pub(crate) fn pass(&mut self) -> Result<bool, String> {
        let mut changed = false;
        let logged = frame_log::get();
        for one in &mut self.windows {
            let pass_started = Instant::now();
            let before = one.app.runtime().root().clone();
            // What's due wakes, as a window's timer for it would: a CSS transition that changes slowly enough says when it
            // next looks different, from when it last moved on, and only waking moves it on.
            let now = self.still.unwrap_or_else(Instant::now);
            match one.app.next_wake() {
                Some(wait) if wait.is_zero() || one.wake_due.is_some_and(|due| due <= now) => {
                    one.app.wake_at(now);
                    one.wake_due = None;
                }
                Some(wait) => {
                    let due = now + wait;
                    one.wake_due = Some(one.wake_due.map_or(due, |was| was.min(due)));
                }
                None => one.wake_due = None,
            }
            // What's moving goes on to where it is by now, as it would on a screen.
            if one.app.animating() {
                one.app.tick(self.still.unwrap_or_else(Instant::now));
            }
            // Drawn when the app says what's on screen isn't right any more, as a window is, and not otherwise: a key is a key
            // going down, its text and the key coming up, each with a pass, and working the same scene out again after each
            // was most of what drawing cost a key. What the inspector keeps about what rendered, and why, is still kept for
            // each pass, as a frame that drew nothing.
            let mut told_after = false;
            if one.app.update() == Frame::Needed {
                if logged.is_some_and(|log| log.render) {
                    if one.renderer.is_none() {
                        one.renderer = Some(Renderer::offscreen_on_hardware(one.size).map_err(|error| error.to_string())?);
                    }
                    let renderer = one.renderer.as_mut().expect("that's just been made");
                    // What's scrolled is told after the frame's been drawn, as `render` does, logged by itself below.
                    one.app.draw_only(renderer).map_err(|error| error.to_string())?;
                    told_after = true;
                } else if draws_frames() {
                    // With `RSUI_DO_DRAW` set, each frame is drawn as well, on the GPU offscreen, as a window draws it, for
                    // measuring what drawing costs and keeps over time.
                    one.render_frame()?;
                } else {
                    one.app.scene();
                }
                if logged.is_some() {
                    let parts: Vec<String> = rsui::split::take()
                        .into_iter()
                        .filter(|(name, _, _)| name != "outside" && name != "idle-sleep")
                        .map(|(name, nanos, times)| format!("{name}={:.3}/{times}", nanos as f64 / 1e6))
                        .collect();
                    frame_log::note("frame", pass_started, format_args!("{} sy={:.2}", parts.join(" "), scrolled_sum(one.app.runtime().root())));
                    // How much of the tree that frame changed, after the frame's time is taken.
                    if std::env::var_os("RSUI_FRAME_CHANGES").is_some() {
                        let after = one.app.runtime().root().clone();
                        if !std::ptr::eq(&*before, &*after) {
                            let changes = diff(&before, &after);
                            frame_log::note("changes", Instant::now(), format_args!("{}", changes.len()));
                            if std::env::var_os("RSUI_FRAME_CHANGES").is_some_and(|value| value == "2") {
                                for patch in &changes {
                                    let said: String = describe_patch(patch).replace('\n', " ").chars().take(300).collect();
                                    frame_log::note("patch", Instant::now(), format_args!("{said}"));
                                }
                            }
                        }
                    }
                }
                if told_after {
                    let told_started = Instant::now();
                    if one.app.tell_scrolled() && logged.is_some() {
                        let parts: Vec<String> = rsui::split::take()
                            .into_iter()
                            .filter(|(name, _, _)| name != "outside" && name != "idle-sleep")
                            .map(|(name, nanos, times)| format!("{name}={:.3}/{times}", nanos as f64 / 1e6))
                            .collect();
                        frame_log::note("told", told_started, format_args!("{}", parts.join(" ")));
                    }
                }
            } else {
                one.app.frame_unchanged();
            }
            changed |= !std::ptr::eq(&*before, &**one.app.runtime().root());
            if let Some(title) = one.app.take_title_wanted() {
                one.title = title.to_string();
            }
            for url in one.app.take_urls_wanted() {
                bridge::say(format!("it asked to open {url:?}, which wasn't done"));
            }
        }
        let requests = rsui::take_window_requests();
        for wanted in requests.open {
            let size = wanted.options.size.unwrap_or(self.default_size);
            let text = rsui::default_text_engine().map_err(|error| error.to_string())?;
            let mut app = (wanted.make)(text, size, 1.0);
            // What's drawn is all there is of a transparent window, as it would be in a real one.
            app.set_see_through(wanted.options.transparent);
            // Things move in it as they do in the others.
            if self.animations {
                app.set_animations(true);
            }
            self.windows.push(Open {
                id: wanted.id.0,
                title: wanted.options.title,
                modal: wanted.options.modal,
                app,
                seen: None,
                renderer: None,
                size,
                wake_due: None,
            });
            changed = true;
        }
        for (id, title) in requests.titles {
            if let Some(one) = self.windows.iter_mut().find(|one| one.id == id.0) {
                one.title = title;
                changed = true;
            }
        }
        for (id, change) in requests.changes {
            // A backdrop shows through where the window's seen through, which is what's drawn here, as a window's would be.
            if let rsui::WindowChange::Backdrop(kind) = &change
                && let Some(one) = self.windows.iter_mut().find(|one| one.id == id.0)
            {
                one.app.set_see_through(kind != "none");
                changed = true;
            }
            if let rsui::WindowChange::Transparent(on) = &change
                && let Some(one) = self.windows.iter_mut().find(|one| one.id == id.0)
            {
                one.app.set_see_through(*on);
                changed = true;
            }
            // There's no window for this to happen to, so it's said instead.
            let said = match &change {
                rsui::WindowChange::Icon(bytes) => format!("Icon({} bytes)", bytes.len()),
                rsui::WindowChange::Overlay(Some(bytes), words) => format!("Overlay({} bytes, {words:?})", bytes.len()),
                other => format!("{other:?}"),
            };
            self.notes.push(format!("window {} was asked to do this: {said}", id.0));
        }
        for id in requests.close {
            self.window_gone(id.0);
            changed = true;
        }
        Ok(changed)
    }

    fn window_gone(&mut self, id: u32) {
        if id == WindowId::MAIN.0 {
            // The rest go with it, and the main one last.
            while let Some(mut gone) = self.windows.pop() {
                gone.app.update();
            }
            self.notes.push("the app closed".to_owned());
        } else if let Some(at) = self.windows.iter().position(|one| one.id == id) {
            let mut gone = self.windows.remove(at);
            gone.app.update();
            self.notes
                .push(format!("window {id} {:?} closed", gone.title));
            if self.current == id {
                self.current = WindowId::MAIN.0;
            }
        }
    }

    /// Does steps written as they are for `--do`, for a test that has a desk of its own.
    #[cfg(test)]
    pub(crate) fn run(&mut self, steps: &str) -> Result<(), String> {
        for step in parse_steps(steps)? {
            self.step(&step)?;
        }
        Ok(())
    }

    pub(crate) fn busy(&self) -> bool {
        bridge::is_working() || bridge::timer_due_within(TIMER_COUNTS_WITHIN)
    }

    /// Goes until nothing's going on, or it's been this long. Says whether it all finished.
    pub(crate) fn settle(&mut self, longest: Duration) -> Result<bool, String> {
        let was = self.still;
        self.still = Some(was.unwrap_or_else(Instant::now));
        let settled = self.settle_still(longest);
        self.still = was;
        settled
    }

    fn settle_still(&mut self, longest: Duration) -> Result<bool, String> {
        let started = Instant::now();
        loop {
            let changed = self.pass()?;
            if !changed && !self.busy() {
                return Ok(true);
            }
            if started.elapsed() >= longest {
                return Ok(false);
            }
            if !changed {
                let _split = rsui::split::on().then(|| rsui::split::enter(rsui::split::named("idle-sleep")));
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    fn here(&self) -> Result<&Open, String> {
        if self.windows.is_empty() {
            return Err("the app has closed".to_owned());
        }
        self.windows
            .iter()
            .find(|one| one.id == self.current)
            .ok_or_else(|| format!("window {} has closed", self.current))
    }

    pub(crate) fn here_mut(&mut self) -> Result<&mut Open, String> {
        self.here()?;
        let current = self.current;
        Ok(self
            .windows
            .iter_mut()
            .find(|one| one.id == current)
            .expect("that was just looked at"))
    }

    /// The modal window that's keeping the main one from being used, if there is one.
    pub(crate) fn modal_window(&self) -> Option<u32> {
        self.windows
            .iter()
            .rev()
            .find(|one| one.modal)
            .map(|one| one.id)
    }

    /// What's changed in each window since this was last asked.
    pub(crate) fn take_changes(&mut self) -> Vec<(u32, Patch)> {
        self.notes.clear();
        let mut changes = Vec::new();
        for one in &mut self.windows {
            let root = one.app.runtime().root().clone();
            if let Some(seen) = one.seen.replace(root.clone()) {
                changes.extend(diff(&seen, &root).into_iter().map(|patch| (one.id, patch)));
            }
        }
        changes
    }

    /// Does what the close button does to the window that steps are for.
    pub(crate) fn close_here(&mut self) -> Result<(), String> {
        let id = self.usable()?.id;
        let one = self.here_mut()?;
        let kept = one
            .app
            .asked_to_close()
            .map_err(|error| error.to_string())?;
        if !kept {
            self.window_gone(id);
        }
        Ok(())
    }

    /// The window that steps are for, if a person could use it right now.
    pub(crate) fn usable(&mut self) -> Result<&mut Open, String> {
        let current = self.here()?.id;
        if current == WindowId::MAIN.0 {
            if let Some(modal) = self.windows.iter().rev().find(|one| one.modal) {
                return Err(format!(
                    "the main window can't be used until window {} {:?} has gone, the same as for a person",
                    modal.id, modal.title
                ));
            }
        }
        self.here_mut()
    }

    /// Finds where something is: `#key`, `@path` as an outline shows it, or text it's called, exactly or else in part. If
    /// that's more than one thing, it's whichever can do `action`, as the text in a button is called what the button is.
    fn find(&self, target: &Word, action: Option<ActionKind>) -> Result<Vec<u32>, String> {
        let root = self.here()?.app.runtime().root();
        let name = target.text.as_str();
        if !target.quoted {
            if let Some(key) = name.strip_prefix('#') {
                let found = Query::new().key(key).find_one(root, &[]);
                // A page's element is found by its `id` too, which stays the same from one run to the next, as the key
                // that's made for it needn't.
                if found.is_err() {
                    let mut path = Vec::new();
                    if path_with_dom_id(root, key, &mut path) {
                        return Ok(path);
                    }
                }
                return found
                    .map(|found| found.path)
                    .map_err(|error| format!("{error}\n{TARGETS}"));
            }
            if let Some(written) = name.strip_prefix('@') {
                let path =
                    parse_path(written).ok_or_else(|| format!("{name} isn't a path. {TARGETS}"))?;
                if node_at(root, &path).is_none() {
                    return Err(format!(
                        "nothing's at {name}. They change as things come and go"
                    ));
                }
                return Ok(path);
            }
        }
        for query in [Query::new().name(name), Query::new().name_contains(name)] {
            let found = query.find_all(root, &[]);
            if found.is_empty() {
                continue;
            }
            if let [only] = found.as_slice() {
                return Ok(only.path.clone());
            }
            // Of more than one, what a person could reach, as what reads the window out has it: not in something that's only
            // there to be seen, as what's `visibility: hidden` or on its way out is (see `rsui::access`).
            let reachable: Vec<_> = found.iter().filter(|one| reachable(root, &one.path)).collect();
            if let [only] = reachable.as_slice() {
                return Ok(only.path.clone());
            }
            if let Some(action) = action {
                let able: Vec<_> = reachable
                    .iter()
                    .filter(|one| one.node.actions.contains(action))
                    .collect();
                if let [only] = able.as_slice() {
                    return Ok(only.path.clone());
                }
            }
            let error = query
                .find_one(root, &[])
                .expect_err("there's more than one");
            return Err(format!("{error}\n{TARGETS}"));
        }
        let error = Query::new()
            .name(name)
            .find_one(root, &[])
            .expect_err("nothing's called that");
        Err(format!("{error}\n{TARGETS}"))
    }

    /// Whether this says this text anywhere, as what it's called or what's in it.
    fn says(&self, text: &str) -> Result<bool, String> {
        let text = text
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\t', "\\t");
        Ok(outline(self.here()?.app.runtime().root()).contains(&text))
    }

    /// Where the middle of this is, with it scrolled to if need be.
    fn middle_of(&mut self, path: &[u32]) -> Result<(f32, f32), String> {
        let one = self.usable()?;
        if let Some(node) = node_at(one.app.runtime().root(), path) {
            if is_on(node, "disabled") {
                return Err("it's disabled, so a person couldn't use it either".to_owned());
            }
        }
        one.app
            .feed(&Input::Access { path: path.to_vec(), command: AccessCommand::Reveal })
            .map_err(|error| error.to_string())?;
        self.pass()?;
        let one = self.usable()?;
        let rect = one
            .app
            .layout()
            .rect(path)
            .filter(|rect| rect.width > 0.0 && rect.height > 0.0)
            .ok_or("it has no size, so it isn't on screen")?;
        // Where it's shown, which a transform can move it from, as a Radix menu's is.
        let (x, y) = one
            .app
            .shown_middle(path)
            .unwrap_or((rect.x + rect.width / 2.0, rect.y + rect.height / 2.0));
        let (width, height) = one.size;
        if x < 0.0 || y < 0.0 || x >= width as f32 || y >= height as f32 {
            return Err(format!(
                "it's at {x}, {y}, which is outside the window, at {width} by {height}"
            ));
        }
        Ok((x, y))
    }

    /// A drag as a hand makes it, in real time: the main button goes down at a share of `target`'s box, moves
    /// `moves` times, `rate` times a second, in a straight line to another share of it, and comes up, with passes in between
    /// on the screen's beat (`RSUI_DO_HZ`), and moves told at most once a frame, as a window tells them. It says what each
    /// move and each pass cost, and, given `watch`, how far behind the pointer the middle of that was each time it was laid
    /// out, and how long after the last move it got there.
    fn glide(
        &mut self,
        target: &Word,
        shares: [&Word; 4],
        moves: &Word,
        rate: &Word,
        watch: Option<&Word>,
    ) -> Result<(), String> {
        let number = |word: &Word| word.text.parse::<f64>().map_err(|_| format!("{} isn't a number", word.text));
        let [x0, y0, x1, y1] = [number(shares[0])?, number(shares[1])?, number(shares[2])?, number(shares[3])?];
        let moves = (number(moves)? as u32).max(1);
        let rate = number(rate)?.max(1.0);
        let path = self.find(target, None)?;
        self.middle_of(&path)?;
        let watched = match watch {
            Some(word) => Some(self.find(word, None)?),
            None => None,
        };
        let one = self.usable()?;
        let rect = one.app.layout().rect(&path).ok_or("it has no size, so it isn't on screen")?;
        let at = |along: f64| {
            let (sx, sy) = (x0 + (x1 - x0) * along, y0 + (y1 - y0) * along);
            (rect.x + rect.width * sx as f32, rect.y + rect.height * sy as f32)
        };
        let frame = frame_log::period().unwrap_or(Duration::from_secs_f64(1.0 / 144.0));
        one.app.set_move_pace(frame);
        let (start_x, start_y) = at(0.0);
        let _ = one.app.feed(&Input::Move { x: start_x, y: start_y });
        self.pass()?;
        let _ = self.usable()?.app.feed(&Input::Down { x: start_x, y: start_y, clicks: 1 });
        self.pass()?;
        // Where the watched part's middle is, along the way the pointer goes, as a share of it.
        let horizontal = (x1 - x0).abs() * f64::from(rect.width) >= (y1 - y0).abs() * f64::from(rect.height);
        let watched_along = |this: &mut Self| -> Option<f64> {
            let path = watched.as_ref()?;
            let thumb = this.usable().ok()?.app.layout().rect(path)?;
            Some(if horizontal {
                (f64::from(thumb.x + thumb.width / 2.0 - rect.x) / f64::from(rect.width) - x0) / (x1 - x0)
            } else {
                (f64::from(thumb.y + thumb.height / 2.0 - rect.y) / f64::from(rect.height) - y0) / (y1 - y0)
            })
        };
        let spacing = Duration::from_secs_f64(1.0 / rate);
        let span = spacing.as_secs_f64() * f64::from(moves);
        let started = Instant::now();
        let (mut sent, mut feed_ms, mut pass_ms, mut lags) = (0u32, Vec::new(), Vec::new(), Vec::new());
        let mut last_frame = started;
        let mut reached: Option<Duration> = None;
        let settle_by = Duration::from_secs_f64(span) + Duration::from_millis(1000);
        loop {
            let now = Instant::now();
            if sent < moves && now >= started + spacing * sent {
                sent += 1;
                let (x, y) = at(f64::from(sent) / f64::from(moves));
                let fed = Instant::now();
                let _ = self.usable()?.app.feed(&Input::Move { x, y });
                feed_ms.push(fed.elapsed().as_secs_f64() * 1000.0);
                frame_log::note("move", fed, format_args!("{sent}"));
            }
            let passed = Instant::now();
            self.pass()?;
            pass_ms.push(passed.elapsed().as_secs_f64() * 1000.0);
            let gone = started.elapsed();
            if let Some(along) = watched_along(self) {
                // How long ago the pointer was where the watched part is now.
                let pointer = (gone.as_secs_f64() / span).min(1.0);
                if sent < moves {
                    lags.push((pointer - along).max(0.0) * span * 1000.0);
                } else if reached.is_none() && (1.0 - along).abs() < 0.01 {
                    reached = Some(gone.saturating_sub(Duration::from_secs_f64(span)));
                }
            }
            if sent >= moves && (watched.is_none() || reached.is_some() || gone >= settle_by) {
                break;
            }
            // To whichever's next: the next move, or the next time the screen would show something.
            let next_move = (sent < moves).then(|| started + spacing * sent);
            let mut next_frame = last_frame + frame;
            while next_frame <= Instant::now() {
                next_frame += frame;
            }
            last_frame = next_frame;
            let next = next_move.map_or(next_frame, |due| due.min(next_frame));
            if let Some(wait) = next.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
        let (end_x, end_y) = at(1.0);
        let one = self.usable()?;
        one.app.feed(&Input::Up { x: end_x, y: end_y }).map_err(|error| error.to_string())?;
        one.app.set_move_pace(Duration::ZERO);
        let stats = |list: &[f64]| {
            let mean = list.iter().sum::<f64>() / list.len().max(1) as f64;
            let worst = list.iter().copied().fold(0.0, f64::max);
            (mean, worst)
        };
        let (feed_mean, feed_worst) = stats(&feed_ms);
        let (pass_mean, pass_worst) = stats(&pass_ms);
        let busy: f64 = feed_ms.iter().chain(&pass_ms).sum();
        println!(
            "  {moves} moves in {:.0} ms: feeding {feed_mean:.2} ms each, worst {feed_worst:.2}; {} passes {pass_mean:.2} ms each, worst {pass_worst:.2}; busy {:.1} ms a move",
            span * 1000.0,
            pass_ms.len(),
            busy / f64::from(moves)
        );
        if watched.is_some() {
            let (lag_mean, lag_worst) = stats(&lags);
            let settled = reached.map_or("never".to_owned(), |when| format!("{:.0} ms", when.as_secs_f64() * 1000.0));
            println!("  it was {lag_mean:.1} ms behind the pointer, worst {lag_worst:.1}, and got to the end {settled} after the last move");
        }
        Ok(())
    }

    /// Why the component that made what's at `path` last rendered, how often it has, what its hooks hold, and what it's in.
    pub(crate) fn why(&mut self, path: &[u32]) -> Result<String, String> {
        self.pass()?;
        let one = self.here()?;
        if !one.app.inspecting() {
            return Err(
                "nothing's being kept about what renders. `inspect on` starts that".to_owned(),
            );
        }
        let made = made_by(&one.app, path);
        let mut out = made.why.unwrap_or_else(|| {
            "What made this hasn't rendered since inspecting started, which was before it was there if that's from the start."
                .to_owned()
        });
        if !made.around.is_empty() {
            out.push_str("\nIt's in ");
            out.push_str(&made.around.join(" › "));
            out.push('.');
        }
        if !made.hooks.is_empty() {
            out.push_str("\nIts hooks:");
            for hook in made.hooks {
                out.push_str("\n  ");
                out.push_str(&hook);
            }
        }
        Ok(out)
    }

    /// The story of each frame since this last told, in every window: what came in, what rendered and why, and where the
    /// time went. Frames where nothing happened but drawing aren't told.
    pub(crate) fn frames(&mut self) -> Result<String, String> {
        self.pass()?;
        if !self.windows.iter().any(|one| one.app.inspecting()) {
            return Err("nothing's being kept about frames. `inspect on` starts that".to_owned());
        }
        let mut out = String::new();
        let several = self.windows.len() > 1;
        for one in &self.windows {
            let told = self
                .frames_told
                .iter()
                .find(|(id, _)| *id == one.id)
                .map_or(0, |(_, told)| *told);
            let new: Vec<_> = one
                .app
                .frames()
                .filter(|frame| frame.number > told)
                .collect();
            let Some(last) = new.last() else {
                continue;
            };
            let last = last.number;
            let busy: Vec<_> = new
                .iter()
                .filter(|frame| !frame.renders.is_empty() || !frame.handled.is_empty())
                .collect();
            if several {
                out.push_str(&format!("window {} {:?}:\n", one.id, one.title));
            }
            if busy.len() > MOST_FRAMES {
                out.push_str(&format!(
                    "({} earlier frames left out)\n",
                    busy.len() - MOST_FRAMES
                ));
            }
            for frame in busy.iter().skip(busy.len().saturating_sub(MOST_FRAMES)) {
                out.push_str(&frame.story());
                out.push('\n');
            }
            let quiet = new.len() - busy.len();
            if quiet > 0 {
                out.push_str(&format!(
                    "({quiet} {} only drew)\n",
                    if quiet == 1 { "frame" } else { "frames" }
                ));
            }
            match self.frames_told.iter_mut().find(|(id, _)| *id == one.id) {
                Some(entry) => entry.1 = last,
                None => self.frames_told.push((one.id, last)),
            }
        }
        if out.is_empty() {
            out.push_str("nothing's happened since the last time\n");
        }
        Ok(out.trim_end().to_owned())
    }

    /// What the frames kept come to, in each window: the slowest, what took the most time, and why what changed nothing
    /// rendered.
    pub(crate) fn profile(&mut self) -> Result<String, String> {
        self.pass()?;
        if !self.windows.iter().any(|one| one.app.inspecting()) {
            return Err("nothing's being kept about frames. `inspect on` starts that".to_owned());
        }
        let several = self.windows.len() > 1;
        let mut out = String::new();
        for one in &self.windows {
            if several {
                out.push_str(&format!("window {} {:?}:\n", one.id, one.title));
            }
            out.push_str(&rsui::summary(one.app.frames()));
            out.push_str("\n\n");
        }
        Ok(out.trim_end().to_owned())
    }

    /// Every change of state that's kept, with what it was before and after, and where.
    pub(crate) fn history(&mut self) -> Result<String, String> {
        self.pass()?;
        if !crate::why::tracing() {
            return Err("nothing's being kept about state. `inspect on` starts that".to_owned());
        }
        Ok(crate::why::history_text())
    }

    /// Puts every component's state back to how it was just after a change in `history`, or forward again.
    pub(crate) fn go_to(&mut self, change: u32) -> Result<String, String> {
        self.pass()?;
        if !crate::why::tracing() {
            return Err("nothing's being kept about state. `inspect on` starts that".to_owned());
        }
        let put_back = bridge::go_to_change(change)?;
        self.settle(STEP_SETTLES_FOR)?;
        Ok(crate::why::went_to(change, put_back))
    }

    /// Starts or stops keeping what renders, or showing it in pictures.
    pub(crate) fn inspect(&mut self, how: &str) -> Result<(), String> {
        for one in &mut self.windows {
            match how {
                "on" => one.app.set_inspecting(true),
                "off" => one.app.set_inspecting(false),
                "show" => one.app.show_inspector(true),
                "hide" => one.app.show_inspector(false),
                _ => return Err(format!("inspect is on, off, show or hide, not {how:?}")),
            }
        }
        rsui::inspect_from_start(
            matches!(how, "on" | "show")
                || (how == "hide" && self.windows.iter().any(|one| one.app.inspecting())),
        );
        bridge::set_tracing(self.windows.iter().any(|one| one.app.inspecting()));
        Ok(())
    }

    pub(crate) fn press(&mut self, key: &str) -> Result<(), String> {
        let (press, typed) = parse_key(key)?;
        let one = self.usable()?;
        one.app.feed(&Input::Key(press)).map_err(|error| error.to_string())?;
        self.pass()?;
        if let Some(typed) = typed {
            let one = self.usable()?;
            one.app
                .feed(&Input::Text(typed.to_string()))
                .map_err(|error| error.to_string())?;
            self.pass()?;
        }
        self.held()?;
        // And it comes up, as a key does, which only what's listening for that hears.
        let one = self.usable()?;
        one.app.feed(&Input::KeyUp(press)).map_err(|error| error.to_string())?;
        Ok(())
    }

    /// A key or a button stays down for a moment, as a person's does: timers what it set off said were due at once run before
    /// it comes up, as they do on a page, where they wait for the event to be done but not for the key to come up (Radix's radio
    /// group checks what an arrow key moved the keyboard to only while the key's down).
    fn held(&mut self) -> Result<(), String> {
        for _ in 0..4 {
            if !bridge::timer_due_within(Duration::from_millis(1)) {
                break;
            }
            bridge::pump_soon();
            self.pass()?;
        }
        Ok(())
    }

    /// A click with what's held, at a place in the window, as a person does it: the pointer gets there, and a button goes down
    /// and comes up, which for the other button brings up a menu.
    fn press_at(&mut self, x: f32, y: f32, button: PointerButton, held: Modifiers) -> Result<(), String> {
        let _ = self.usable()?.app.feed(&Input::Modifiers(held));
        let _ = self.usable()?.app.feed(&Input::Move { x, y });
        self.pass()?;
        let _ = self.usable()?.app.feed(&Input::ButtonDown { x, y, button });
        self.pass()?;
        self.held()?;
        let one = self.usable()?;
        let up = Instant::now();
        let done = one.app.feed(&Input::ButtonUp { x, y, button }).map(|_| ()).map_err(|error| error.to_string());
        frame_log::note("up", up, format_args!(""));
        // What's held is let go of, for the next step.
        let _ = one.app.feed(&Input::Modifiers(Modifiers::default()));
        done
    }

    /// Where the middle of the page's element that a CSS selector finds is, as the page's own `getBoundingClientRect` says,
    /// scrolled into view first if it's outside the window, as a person would.
    fn point_of(&mut self, selector: &str) -> Result<(f32, f32), String> {
        let quoted = serde_json::to_string(selector).map_err(|error| error.to_string())?;
        for scroll in [true, false] {
            self.pass()?;
            let said = bridge::evaluate(&format!("{POINT_OF}({quoted}, {scroll})"))?;
            match said.as_str() {
                "none" => return Err(format!("nothing in the page matches {selector:?}")),
                "scrolled" => continue,
                _ => {}
            }
            let numbers: Vec<f32> = said.split(' ').filter_map(|part| part.parse().ok()).collect();
            let [x, y, width, height] = numbers[..] else {
                return Err(format!("{selector:?} gave {said:?}, which isn't a place"));
            };
            if width <= 0.0 || height <= 0.0 {
                return Err(format!("{selector:?} has no size, so it isn't on screen"));
            }
            return Ok((x, y));
        }
        Err(format!("{selector:?} is still outside the window after scrolling to it"))
    }

    /// The place a step is about, and how many of its words said it: `X Y` in the window's pixels, `css:SELECTOR` for the
    /// middle of what that finds in the page, or nothing for where the pointer is.
    fn place(&mut self, words: &[Word]) -> Result<((f32, f32), usize), String> {
        let number = |word: &Word| word.text.parse::<f32>().map_err(|_| format!("{} isn't a number", word.text));
        let point = match words {
            [first, ..] if first.text.starts_with("css:") => (self.point_of(&first.text[4..])?, 1),
            [x, y, ..] if x.text.parse::<f32>().is_ok() => ((number(x)?, number(y)?), 2),
            [] => (self.pointer, 0),
            [other, ..] => return Err(format!("{} isn't a place: that's X Y, or css:SELECTOR", other.text)),
        };
        self.pointer = point.0;
        Ok(point)
    }

    fn step(&mut self, step: &Step) -> Result<(), String> {
        let words = step.words.as_slice();
        let verb = words[0].text.as_str();
        let arguments = &words[1..];
        // Text it's said to be looking for, with whether it's that it shouldn't be there.
        let looking_for = |arguments: &[Word]| match arguments {
            [no, text, rest @ ..] if !no.quoted && no.text == "no" => {
                Ok((text.text.clone(), true, rest.to_vec()))
            }
            [text, rest @ ..] => Ok((text.text.clone(), false, rest.to_vec())),
            [] => Err(format!("{verb} needs the text to look for")),
        };
        match (verb, arguments) {
            ("look", []) => {
                self.here()?;
                self.print_all();
                return Ok(());
            }
            ("inspect", [how]) if !how.quoted => {
                self.inspect(&how.text)?;
                println!("  inspecting is {}", how.text);
                return Ok(());
            }
            ("why", [target]) => {
                let path = self.find(target, None)?;
                for line in self.why(&path)?.lines() {
                    println!("  {line}");
                }
                return Ok(());
            }
            ("frames", []) => {
                for line in self.frames()?.lines() {
                    println!("  {line}");
                }
                return Ok(());
            }
            ("logpoint", [off, at]) if !off.quoted && off.text == "off" => {
                let removed = crate::logpoint::remove(&at.text)?;
                println!(
                    "  {removed} {} taken away",
                    if removed == 1 {
                        "logpoint"
                    } else {
                        "logpoints"
                    }
                );
                self.settle(step_settles_for())?;
                return Ok(());
            }
            ("logpoint", [at, expression]) => {
                let place = crate::logpoint::add(&at.text, &expression.text)?;
                println!("  {place} says {} each time it runs", expression.text);
                // The code with it in runs, as when a file's saved, which says what it finds as it goes.
                self.settle(step_settles_for())?;
                return Ok(());
            }
            // Runs some of the page's JavaScript, as a console does, and says what it came to.
            ("eval", [code]) => {
                let said = bridge::evaluate(&code.text)?;
                println!("  = {said}");
                return Ok(());
            }
            ("logpoints", []) => {
                for line in crate::logpoint::list().lines() {
                    println!("  {line}");
                }
                return Ok(());
            }
            ("break", [off, at]) if !off.quoted && off.text == "off" => {
                let removed = crate::debugger::remove(&at.text)?;
                println!(
                    "  {removed} {} taken away",
                    if removed == 1 {
                        "breakpoint"
                    } else {
                        "breakpoints"
                    }
                );
                return Ok(());
            }
            ("break", [at]) => {
                let place = crate::debugger::add(&at.text)?;
                println!("  {place} says what's in its variables each time it gets there");
                // The app's code is taken in again with places to stop in it, as when a file's saved.
                self.settle(step_settles_for())?;
                return Ok(());
            }
            ("breakpoints", []) => {
                for line in crate::debugger::list().lines() {
                    println!("  {line}");
                }
                return Ok(());
            }
            // Sampling where JavaScript is, as the inspector's Record does, for what `save` keeps.
            ("record", [how]) if !how.quoted && (how.text == "on" || how.text == "off") => {
                let on = how.text == "on";
                crate::bridge::with_engine(|engine| {
                    if on {
                        engine.start_sampling(Duration::from_millis(1));
                    } else {
                        engine.stop_sampling();
                    }
                })
                .ok_or("the app's in the middle of running its code")?;
                return Ok(());
            }
            ("save", [file]) => {
                let one = self.here()?;
                if !one.app.inspecting() {
                    return Err("nothing's being kept to save. `inspect on` starts that".to_owned());
                }
                crate::trace_file::save(&one.app, &one.title, Path::new(&file.text))?;
                println!(
                    "  saved in {}. `rsui-tsx --view {}` shows it",
                    file.text, file.text
                );
                return Ok(());
            }
            // The measuring steps are only in a build that measures, as they bring the engine's census into the program.
            #[cfg(feature = "alloc-owners")]
            ("retainers", [label, limit]) => {
                // The shortest way from a root to each of the last `limit` objects counted as `label`, for telling what keeps them.
                self.pass()?;
                let limit = limit.text.parse().unwrap_or(50);
                for line in bridge::retainers(&label.text, limit) {
                    println!("  kept by {line}");
                }
                return Ok(());
            }
            ("memory", []) => {
                // What the process takes from the system now and at most, and what the engine's lists take.
                self.pass()?;
                let process = crate::process_memory::now();
                println!(
                    "  process: working set {} B (peak {} B), private working set {} B, private {} B (peak {} B)",
                    process.working_set, process.peak_working_set, process.private_working_set, process.private, process.peak_private
                );
                let (heap, allocated, committed) = crate::process_memory::process_heap();
                println!("  process heap: {heap}, {allocated} B allocated, {committed} B committed");
                let (memory, footprint, collections) = bridge::engine_memory();
                println!(
                    "  engine: objects {} B, strings {} B, shapes {} B, code {} B, stack {} B, {} collections",
                    memory.objects, memory.strings, memory.shapes, memory.code, memory.stack, collections
                );
                for line in footprint.to_string().lines() {
                    println!("  {line}");
                }
                println!(
                    "  allocator: {} B in chunks of small blocks, {} B of blocks in use, {} B in empty chunks",
                    rsui::UiAlloc.retained_bytes(),
                    rsui::UiAlloc.live_bytes(),
                    rsui::UiAlloc.empty_bytes()
                );
                // Each class's chunks against what's in use in them, where that's more than a chunk's worth of room.
                let mut classes = String::new();
                for (size, chunks, blocks) in rsui::UiAlloc.classes() {
                    let room = (chunks * 65536).saturating_sub(blocks * size);
                    if room > 65536 {
                        classes += &format!(" {size}: {chunks} chunks, {} KB in use, {} KB room;", blocks * size / 1024, room / 1024);
                    }
                }
                println!("  allocator classes:{classes}");
                if std::env::var_os("RSUI_FREE_PAGES").is_some() {
                    let pages = rsui::UiAlloc.free_page_bytes();
                    let (all, tail): (usize, usize) = pages.iter().fold((0, 0), |(a, t), (_, f, u)| (a + f, t + u));
                    let each: Vec<String> = pages.iter().map(|(size, f, u)| format!("{size}: {} KB ({} untouched)", f / 1024, u / 1024)).collect();
                    println!("  allocator free pages: {} KB, {} KB of it never handed out; {}", all / 1024, tail / 1024, each.join(", "));
                }
                let (spans, room) = rsui_spans::footprint();
                println!("  spans: {spans} kept in {room} B");
                for one in &self.windows {
                    if let Some(line) = one.app.inspector_footprint() {
                        println!("  {line}");
                    }
                }
                println!("  {}", bridge::tracing_footprint());
                if std::env::var_os("RSUI_CODE_FILES").is_some() {
                    for (file, protos, code, caches) in bridge::code_by_file().iter().take(60) {
                        println!("  code {code} ops, {caches} caches, {protos} functions: {file}");
                    }
                }
                if std::env::var_os("RSUI_STRING_DUPES").is_some() {
                    let (count, bytes, again) = bridge::duplicate_strings();
                    println!("  strings with text of their own: {count}, {bytes} B, {again} B of it the same as another's");
                }
                return Ok(());
            }
            ("regions", []) => {
                // Where the process's memory is, by what each reservation of addresses is for, and the heaps' blocks by size.
                self.pass()?;
                for line in crate::process_memory::regions() {
                    println!("  {line}");
                }
                return Ok(());
            }
            ("timers", []) => {
                // What timers are waiting, by what they run, for finding ones that pile up.
                self.pass()?;
                let timers = bridge::timer_summary();
                let total: usize = timers.iter().map(|(count, _)| count).sum();
                println!("  timers: {total} waiting");
                for (count, name) in timers.iter().take(20) {
                    println!("  timer {count} x {name}");
                }
                return Ok(());
            }
            ("idle", []) => {
                // What a window does once things have gone quiet (window.rs), less emptying the working set: lets go of
                // caches and collects the JavaScript's garbage (unless `RSUI_IDLE_GC=0`), all at once where a window waits
                // a few seconds more to collect, and gives back all of the allocator's empty chunks, as a window does after
                // collecting.
                self.pass()?;
                for one in &mut self.windows {
                    one.app.forget_caches();
                }
                let mut released = 0;
                loop {
                    let some = rsui::UiAlloc.reclaim(32);
                    if some == 0 {
                        break;
                    }
                    released += some;
                }
                println!("  idle: {released} B of the allocator's chunks given back");
                return Ok(());
            }
            ("collect", []) => {
                // A full collection, as an idle app could do.
                self.pass()?;
                bridge::collect_all();
                return Ok(());
            }
            #[cfg(feature = "alloc-owners")]
            ("census", rest) if rest.len() <= 1 => {
                // What the engine and each stage are holding, for measuring memory: the engine's objects by what they are, with
                // plain objects by their properties' names, and with the `alloc-owners` feature, what each stage of Rust holds.
                self.pass()?;
                let top = rest.first().and_then(|word| word.text.parse().ok()).unwrap_or(60);
                let census = bridge::census_by_keys();
                let memory = census.memory;
                println!(
                    "  engine: objects {} B, strings {} B, shapes {} B, code {} B, stack {} B",
                    memory.objects, memory.strings, memory.shapes, memory.code, memory.stack
                );
                for line in census.report(top).lines() {
                    println!("  {line}");
                }
                for (owner, bytes) in rsui::alloc_owners::live() {
                    println!("  held by {owner}: {bytes} B");
                }
                for line in rsui::alloc_owners::sites(usize::MAX, 28) {
                    println!("  made by {line}");
                }
                for (table, size) in bridge::tables() {
                    println!("  table {table}: {size}");
                }
                return Ok(());
            }
            ("profile", []) => {
                for line in self.profile()?.lines() {
                    println!("  {line}");
                }
                return Ok(());
            }
            ("history", []) => {
                for line in self.history()?.lines() {
                    println!("  {line}");
                }
                return Ok(());
            }
            ("goto", [number]) if !number.quoted => {
                let number = number.text.parse().map_err(|_| {
                    format!(
                        "{:?} isn't a change's number, as `history` gives them",
                        number.text
                    )
                })?;
                println!("  {}", self.go_to(number)?);
            }
            ("window", [which]) => {
                self.choose_window(which)?;
                println!("  window {} {:?}", self.current, self.here()?.title);
                return Ok(());
            }
            ("click" | "rightclick" | "middleclick", [target, held @ ..]) => {
                let held = parse_held(held)?;
                let path = self.find(target, Some(ActionKind::Click))?;
                let (x, y) = self.middle_of(&path)?;
                let button = match verb {
                    "rightclick" => PointerButton::Secondary,
                    "middleclick" => PointerButton::Middle,
                    _ => PointerButton::Main,
                };
                self.press_at(x, y, button, held)?;
            }
            ("dragto", [from, to, rest @ ..]) => {
                // From the middle of one to the middle of another, with the main button down, in steps, as a hand does it.
                let steps = match rest {
                    [steps] => steps
                        .text
                        .parse::<u32>()
                        .map_err(|_| format!("{} isn't a number of steps", steps.text))?,
                    [] => 10,
                    _ => return Err("dragto takes what to drag, where to, and how many steps".to_owned()),
                }
                .max(1);
                let from = self.find(from, None)?;
                let (from_x, from_y) = self.middle_of(&from)?;
                let to = self.find(to, None)?;
                let (to_x, to_y) = self.middle_of(&to)?;
                let _ = self.usable()?.app.feed(&Input::Move { x: from_x, y: from_y });
                self.pass()?;
                let _ = self.usable()?.app.feed(&Input::Down { x: from_x, y: from_y, clicks: 1 });
                self.pass()?;
                for step in 1..=steps {
                    let along = step as f32 / steps as f32;
                    let (x, y) = (from_x + (to_x - from_x) * along, from_y + (to_y - from_y) * along);
                    let _ = self.usable()?.app.feed(&Input::Move { x, y });
                    self.pass()?;
                }
                let one = self.usable()?;
                one.app
                    .feed(&Input::Up { x: to_x, y: to_y })
                    .map_err(|error| error.to_string())?;
            }
            ("dropfiles" | "droptext", [target, what @ ..]) if !what.is_empty() => {
                // As from another program: it comes over the window, gets to the middle of this, and is dropped there.
                let path = self.find(target, None)?;
                let (x, y) = self.middle_of(&path)?;
                let (files, text): (Vec<String>, Option<String>) = if verb == "dropfiles" {
                    (what.iter().map(|word| word.text.clone()).collect(), None)
                } else {
                    let words: Vec<&str> = what.iter().map(|word| word.text.as_str()).collect();
                    (Vec::new(), Some(words.join(" ")))
                };
                let one = self.usable()?;
                one.app
                    .drag_from_outside(DropPhase::Enter, x, y, &files, text.as_deref());
                self.pass()?;
                let one = self.usable()?;
                one.app
                    .drag_from_outside(DropPhase::Over, x, y, &files, text.as_deref());
                self.pass()?;
                let one = self.usable()?;
                let taken = one
                    .app
                    .drag_from_outside(DropPhase::Drop, x, y, &files, text.as_deref());
                if !taken {
                    println!("  nothing there takes it, so it isn't dropped");
                }
            }
            ("hover" | "move", [target]) => {
                let path = self.find(target, Some(ActionKind::Click))?;
                let (x, y) = self.middle_of(&path)?;
                let _ = self.usable()?.app.feed(&Input::Move { x, y });
            }
            ("compose", text) if !text.is_empty() => {
                // As a way of typing for Chinese, Japanese or Korean does it: it starts, shows what it's come to, and ends
                // with that, which is then typed.
                let text: Vec<&str> = text.iter().map(|word| word.text.as_str()).collect();
                let text = text.join(" ");
                let one = self.usable()?;
                for (phase, said) in [("start", ""), ("update", text.as_str()), ("end", text.as_str())] {
                    one.app.feed(&Input::Composition { phase: phase.to_owned(), text: said.to_owned() }).map_err(|error| error.to_string())?;
                }
                one.app.feed(&Input::Text(text.clone())).map_err(|error| error.to_string())?;
            }
            ("drag", [target, from, to, rest @ ..]) => {
                // Across something, from and to a share of its width, in steps, as a hand does it, saying how long that took.
                let share = |word: &Word| {
                    word.text
                        .parse::<f32>()
                        .map_err(|_| format!("{} isn't a number", word.text))
                };
                let (from, to) = (share(from)?, share(to)?);
                let steps = match rest {
                    [steps] => steps
                        .text
                        .parse::<u32>()
                        .map_err(|_| format!("{} isn't a number of steps", steps.text))?,
                    _ => 20,
                }
                .max(1);
                let path = self.find(target, None)?;
                self.middle_of(&path)?;
                let one = self.usable()?;
                let rect = one
                    .app
                    .layout()
                    .rect(&path)
                    .ok_or("it has no size, so it isn't on screen")?;
                let y = rect.y + rect.height / 2.0;
                let at = |share: f32| rect.x + rect.width * share;
                let _ = self.usable()?.app.feed(&Input::Move { x: at(from), y });
                self.pass()?;
                let _ = self.usable()?.app.feed(&Input::Down { x: at(from), y, clicks: 1 });
                self.pass()?;
                let started = Instant::now();
                for step in 1..=steps {
                    let x = at(from + (to - from) * step as f32 / steps as f32);
                    let _ = self.usable()?.app.feed(&Input::Move { x, y });
                    self.pass()?;
                }
                let took = started.elapsed();
                let one = self.usable()?;
                one.app
                    .feed(&Input::Up { x: at(to), y })
                    .map_err(|error| error.to_string())?;
                println!(
                    "  {steps} moves took {:.1} ms, {:.2} ms each",
                    took.as_secs_f64() * 1000.0,
                    took.as_secs_f64() * 1000.0 / f64::from(steps)
                );
            }
            ("glide", [target, x0, y0, x1, y1, moves, rate, watch @ ..]) if watch.len() <= 1 => {
                self.glide(target, [x0, y0, x1, y1], moves, rate, watch.first())?;
            }
            ("clickat" | "rightclickat" | "hoverat" | "moveat", _) if !arguments.is_empty() || matches!(verb, "hoverat" | "moveat") => {
                // A place in the window, in its own pixels, for what has nothing to find it by, or a page's element by a
                // CSS selector, as `css:SELECTOR`.
                let ((x, y), used) = self.place(arguments)?;
                let held = &arguments[used..];
                if verb == "clickat" || verb == "rightclickat" {
                    let held = parse_held(held)?;
                    let button = if verb == "clickat" { PointerButton::Main } else { PointerButton::Secondary };
                    self.press_at(x, y, button, held)?;
                } else if held.is_empty() {
                    let _ = self.usable()?.app.feed(&Input::Move { x, y });
                } else {
                    return Err(format!("{verb} only takes where"));
                }
            }
            ("downat" | "upat", _) if arguments.len() <= 2 => {
                // The main button going down, or coming up, at a place in the window, for a drag that takes as long as a hand
                // does, with `moveat` and `wait` in between. With no place, it's where the pointer is.
                let ((x, y), used) = self.place(arguments)?;
                if used != arguments.len() {
                    return Err(format!("{verb} takes X Y, css:SELECTOR, or nothing for where the pointer is"));
                }
                let one = self.usable()?;
                if verb == "downat" {
                    let _ = one.app.feed(&Input::Move { x, y });
                    let _ = one.app.feed(&Input::Down { x, y, clicks: 1 });
                } else {
                    one.app.feed(&Input::Up { x, y }).map_err(|error| error.to_string())?;
                }
            }
            ("clicksat", [place @ .., clicks]) if (1..=2).contains(&place.len()) => {
                // Two or three clicks in a row at a place, which picks a word or a paragraph out of text.
                let ((x, y), used) = self.place(place)?;
                if used != place.len() {
                    return Err("clicksat takes X Y or css:SELECTOR, and how many clicks".to_owned());
                }
                let clicks = clicks
                    .text
                    .parse::<u32>()
                    .map_err(|_| format!("{} isn't a number of clicks", clicks.text))?;
                let _ = self.usable()?.app.feed(&Input::Move { x, y });
                self.pass()?;
                // Each press and its count, as a person's come, which is what code counting presses itself hears.
                for count in 1..=clicks {
                    let _ = self.usable()?.app.feed(&Input::Down { x, y, clicks: count });
                    self.pass()?;
                    let one = self.usable()?;
                    one.app.feed(&Input::Up { x, y }).map_err(|error| error.to_string())?;
                    self.pass()?;
                }
            }
            // Into whatever has the keyboard, as someone typing does, wherever that went.
            ("write", text) if !text.is_empty() => {
                let text: Vec<&str> = text.iter().map(|word| word.text.as_str()).collect();
                let one = self.usable()?;
                one.app
                    .type_text(&text.join(" "))
                    .map_err(|error| error.to_string())?;
            }
            // The same, a key at a time, down and up, as a hand types it, for what listens to the keys as well as the text.
            ("keys", text) if !text.is_empty() => {
                let text: Vec<&str> = text.iter().map(|word| word.text.as_str()).collect();
                let timed = std::env::var_os("RSUI_STEP_TIMES").is_some();
                for character in text.join(" ").chars() {
                    let started = Instant::now();
                    if character == ' ' {
                        self.press("space")?;
                    } else if character.is_ascii_alphanumeric() {
                        let key = character.to_ascii_lowercase().to_string();
                        self.press(&if character.is_ascii_uppercase() { format!("shift+{key}") } else { key })?;
                    } else {
                        let one = self.usable()?;
                        one.app
                            .feed(&Input::Text(character.to_string()))
                            .map_err(|error| error.to_string())?;
                    }
                    self.pass()?;
                    if timed {
                        // From the key going down to what it changed laid out and drawn.
                        println!("  key {:.3} ms", started.elapsed().as_secs_f64() * 1000.0);
                    }
                    print_split();
                }
            }
            ("type", [target, text @ ..]) => {
                let path = self.find(target, Some(ActionKind::SetValue))?;
                self.middle_of(&path)?;
                let one = self.usable()?;
                one.app
                    .feed(&Input::Access { path: path.to_vec(), command: AccessCommand::Focus })
                    .map_err(|error| error.to_string())?;
                self.pass()?;
                // It takes the place of what's there, which saves counting backspaces.
                self.press("ctrl+a")?;
                self.pass()?;
                let text: Vec<&str> = text.iter().map(|word| word.text.as_str()).collect();
                let text = text.join(" ");
                if text.is_empty() {
                    self.press("backspace")?;
                }
                for character in text.chars() {
                    let one = self.usable()?;
                    one.app
                        .feed(&Input::Text(character.to_string()))
                        .map_err(|error| error.to_string())?;
                    self.pass()?;
                }
            }
            ("press", [key]) => {
                let started = Instant::now();
                self.press(&key.text)?;
                if std::env::var_os("RSUI_STEP_TIMES").is_some() {
                    // From the key going down to it coming up, with what it changed laid out and drawn, as `keys` times each
                    // key, and as a browser's is timed to its layout: timers that are due later aren't waited for.
                    println!("  pressed {:.3} ms", started.elapsed().as_secs_f64() * 1000.0);
                }
                print_split();
            }
            ("press", [key, target]) => {
                let mut path = self.find(target, Some(ActionKind::Focus))?;
                let one = self.usable()?;
                // The text in a button is found by what it says, and it's the button that gets the keyboard, as a click
                // there gives it.
                let root = one.app.runtime().root();
                let mut up = path.clone();
                while !up.is_empty() && !node_at(root, &up).is_some_and(|node| node.actions.contains(ActionKind::Focus)) {
                    up.pop();
                }
                if !up.is_empty() {
                    path = up;
                }
                one.app
                    .feed(&Input::Access { path: path.to_vec(), command: AccessCommand::Focus })
                    .map_err(|error| error.to_string())?;
                self.pass()?;
                self.press(&key.text)?;
            }
            // Several things the pointer does one after another with no frame in between, as a window gets through what's waiting
            // for it after a frame or the page's code has held it up: `move X Y`, `down X Y` (no move to there first, as the
            // system can send a press where the pointer's got to without one), `up X Y`, `dblclick X Y` (the second press of a
            // double click, down and up), and `wheel X Y ROWS`. After each, what it changed is applied, as a window does.
            ("burst", said) if !said.is_empty() => {
                let words: Vec<&str> = said.iter().map(|word| word.text.as_str()).collect();
                let number = |text: &str| text.parse::<f32>().map_err(|_| format!("{text:?} isn't a number"));
                let mut at = 0;
                while at < words.len() {
                    let (what, x, y) = match &words[at..] {
                        [what, x, y, ..] => (*what, number(x)?, number(y)?),
                        _ => return Err(format!("burst: {:?} needs where", words[at])),
                    };
                    at += 3;
                    let one = self.usable()?;
                    match what {
                        "move" => {
                            let _ = one.app.feed(&Input::Move { x, y });
                        }
                        "down" => {
                            let _ = one.app.feed(&Input::Down { x, y, clicks: 1 });
                        }
                        "up" => {
                            one.app.feed(&Input::Up { x, y }).map_err(|error| error.to_string())?;
                        }
                        "dblclick" => {
                            let _ = one.app.feed(&Input::Down { x, y, clicks: 2 });
                            one.app.feed(&Input::Up { x, y }).map_err(|error| error.to_string())?;
                        }
                        "wheel" => {
                            let rows = words.get(at).ok_or("burst: wheel needs rows")?;
                            let rows: i64 = rows.parse().map_err(|_| format!("{rows:?} isn't a number of rows"))?;
                            at += 1;
                            one.app.scroll(x, y, rows).map_err(|error| error.to_string())?;
                        }
                        other => return Err(format!("burst doesn't know {other:?}")),
                    }
                    // What it changed is applied, as a window does after each thing it's told, with no frame drawn.
                    let _ = self.usable()?.app.update();
                }
            }
            // A wheel turned COUNT notches of ROWS at X Y, one every EVERY ms in real time, as a window gets them: whatever's
            // come by the time a frame's done is taken in together, from when the first of those was turned, before the next
            // frame. Then it waits until the scrolling's stopped, for measuring how long after the last notch that is.
            ("spin", [x, y, rows, count, every]) => {
                let number = |word: &Word| word.text.parse::<f64>().map_err(|_| format!("{:?} isn't a number", word.text));
                let (x, y) = (number(x)? as f32, number(y)? as f32);
                let (rows, count) = (number(rows)? as i64, number(count)? as u64);
                let every = Duration::from_secs_f64(number(every)? / 1000.0);
                let started = Instant::now();
                let mut fed = 0;
                let mut still_since: Option<(Instant, f64)> = None;
                loop {
                    let now = Instant::now();
                    let mut due = 0;
                    let mut first = None;
                    while fed < count && started + every * fed as u32 <= now {
                        let at = started + every * fed as u32;
                        frame_log::note("notch", at, format_args!("{rows}"));
                        first.get_or_insert(at);
                        due += rows;
                        fed += 1;
                    }
                    if let Some(first) = first {
                        let one = self.usable()?;
                        one.app.set_input_time(first);
                        one.app.scroll(x, y, due).map_err(|error| error.to_string())?;
                    }
                    self.pass()?;
                    if fed == count {
                        let down = scrolled_sum(self.usable()?.app.runtime().root());
                        match still_since {
                            Some((since, was)) if was == down => {
                                if since.elapsed() > Duration::from_millis(300) {
                                    break;
                                }
                            }
                            _ => still_since = Some((Instant::now(), down)),
                        }
                    }
                    let period = frame_log::period().unwrap_or(Duration::from_millis(7));
                    let gone = started.elapsed().as_secs_f64();
                    let next = ((gone / period.as_secs_f64()).floor() + 1.0) * period.as_secs_f64();
                    std::thread::sleep(Duration::from_secs_f64(next - gone));
                }
            }
            // The wheel turned here, wherever that is, without bringing anything into view first, for measuring scrolling.
            ("wheelat", [x, y, rows]) => {
                let number = |word: &Word| word.text.parse::<f32>().map_err(|_| format!("{:?} isn't a number", word.text));
                let (x, y) = (number(x)?, number(y)?);
                let rows: i64 = rows.text.parse().map_err(|_| format!("{:?} isn't a number of rows", rows.text))?;
                frame_log::note("notch", Instant::now(), format_args!("{rows}"));
                let one = self.usable()?;
                one.app.scroll(x, y, rows).map_err(|error| error.to_string())?;
            }
            ("scroll", [target, rows]) => {
                let rows: i64 = rows
                    .text
                    .parse()
                    .map_err(|_| format!("{:?} isn't a number of rows", rows.text))?;
                let path = self.find(target, Some(ActionKind::Scroll))?;
                let (x, y) = self.middle_of(&path)?;
                let one = self.usable()?;
                one.app
                    .scroll(x, y, rows)
                    .map_err(|error| error.to_string())?;
            }
            ("act", [target, action, input @ ..]) if input.len() < 2 => {
                let kind = ActionKind::from_name(&action.text).ok_or_else(|| {
                    format!(
                        "no action is called {:?}; what a node can do is in its [brackets]",
                        action.text
                    )
                })?;
                let path = self.find(target, Some(kind))?;
                let one = self.usable()?;
                if let Some(node) = node_at(one.app.runtime().root(), &path) {
                    if is_on(node, "disabled") {
                        return Err("it's disabled, so a person couldn't use it either".to_owned());
                    }
                }
                one.app
                    .runtime_mut()
                    .dispatch(&path, kind, input.first().map(Word::as_prop))
                    .map_err(|error| error.to_string())?;
            }
            ("close", []) => self.close_here()?,
            ("global", [keys]) => crate::native::press_global(&keys.text)?,
            ("tray", []) => crate::native::click_tray(None)?,
            ("tray", [item]) => crate::native::click_tray(Some(&item.text))?,
            ("menubar", [item]) => crate::native::click_menu_bar(&item.text)?,
            ("popup", [item]) => crate::native::click_popup(&item.text)?,
            ("thumb", [button]) => crate::native_more::click_thumbnail_button(&button.text)?,
            ("media", [key]) => crate::native_more::press_media_key(&key.text)?,
            ("notification", []) => crate::native::click_notification()?,
            ("soundends", []) => crate::native::end_sounds()?,
            ("secondinstance", words) => {
                let words: Vec<String> = words.iter().map(|word| word.text.clone()).collect();
                crate::native::second_instance(&words)?;
            }
            ("power", [what]) => crate::native::power(&what.text)?,
            ("webview", words) if !words.is_empty() => {
                let words: Vec<String> = words.iter().map(|word| word.text.clone()).collect();
                crate::webview::step(&words)?;
            }
            ("dialog", [answer]) => {
                let cancelled = !answer.quoted && answer.text == "cancel";
                let answer = (!cancelled).then(|| answer.text.clone());
                println!("  the next dialog gets {answer:?}");
                bridge::answer_next_dialog(answer);
                return Ok(());
            }
            ("animations", [on]) if !on.quoted && matches!(on.text.as_str(), "on" | "off") => {
                // Things move as they would in a window, or go straight to where they're going, as they do here otherwise.
                self.set_animations(on.text == "on");
            }
            ("active" | "shown", [on]) if !on.quoted && matches!(on.text.as_str(), "on" | "off") => {
                // Another window coming to the front (`active off`) and this one coming back, as Alt-Tab does, or the window being
                // minimized (`shown off`) and brought back, which a page hears as `blur`, `focus` and `visibilitychange`.
                let on = on.text == "on";
                let input = if verb == "active" { Input::Active(on) } else { Input::Shown(on) };
                self.usable()?.app.feed(&input).map_err(|error| error.to_string())?;
            }
            ("size", [width, height]) => {
                let number = |word: &Word| {
                    word.text
                        .parse::<u32>()
                        .ok()
                        .filter(|size| *size > 0)
                        .ok_or_else(|| format!("{:?} isn't a size", word.text))
                };
                let size = (number(width)?, number(height)?);
                let one = self.here_mut()?;
                one.size = size;
                let _ = one.app.feed(&Input::Resize { width: size.0, height: size.1, scale: 1.0 });
                // It's a new size of picture to draw to.
                if one.renderer.take().is_some() {
                    one.app.renderer_replaced();
                }
            }
            ("rects", [file]) => {
                let one = self.here_mut()?;
                one.draw()?;
                std::fs::write(&file.text, one.app.rectangles_with_lines())
                    .map_err(|error| format!("{}: {error}", file.text))?;
                return Ok(());
            }
            ("shot", [file]) => {
                let one = self.here_mut()?;
                let pixels = one.draw()?;
                let (width, height) = one.size;
                crate::write_png(
                    &PathBuf::from(&file.text),
                    width as usize,
                    height as usize,
                    &pixels,
                )
                .map_err(|error| format!("{}: {error}", file.text))?;
                println!("  {width} by {height}, in {}", file.text);
                return Ok(());
            }
            ("evalto", [file, code]) => {
                // What code run in the page comes to, written to a file as it is (a string's own text), for what reads the
                // page's state afterwards.
                self.pass()?;
                let said = bridge::evaluate(&code.text)?;
                std::fs::write(&file.text, said).map_err(|error| format!("{}: {error}", file.text))?;
                return Ok(());
            }
            ("waitjs", [code, rest @ ..]) if rest.len() <= 1 => {
                // Passes in real time until code run in the page comes to `true`, or the time's up, and says which. Running
                // out of time isn't a failure: it's for waiting until things have finished moving before looking.
                let longest = match rest {
                    [time] => Duration::from_millis(time.text.parse().map_err(|_| format!("{:?} isn't a time", time.text))?),
                    _ => WAIT_LOOKS_FOR,
                };
                let started = Instant::now();
                loop {
                    self.pass()?;
                    if bridge::evaluate(&code.text)? == "true" {
                        break;
                    }
                    if started.elapsed() >= longest {
                        println!("  it still wasn't after {} ms", longest.as_millis());
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(frame_nap().min(4)));
                }
                return Ok(());
            }
            ("shots", [folder, time, every @ ..]) if every.len() <= 1 => {
                // Pictures of the window as it goes on in real time, one every EVERY ms (16, a 60 Hz screen's frame) for MS
                // ms, as `frame-NNN-<ms>.png` in FOLDER, the ms from the first: how something moves, frame by frame.
                let number = |word: &Word| word.text.parse::<u64>().map_err(|_| format!("{:?} isn't a time", word.text));
                let longest = Duration::from_millis(number(time)?);
                let every = Duration::from_millis(every.first().map(number).transpose()?.unwrap_or(16).max(1));
                let folder = PathBuf::from(&folder.text);
                std::fs::create_dir_all(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;
                let started = Instant::now();
                let mut taken = 0u32;
                while started.elapsed() <= longest {
                    self.pass()?;
                    let at = started.elapsed().as_millis();
                    let one = self.here_mut()?;
                    let pixels = one.draw()?;
                    let (width, height) = one.size;
                    let file = folder.join(format!("frame-{taken:03}-{at}.png"));
                    crate::write_png(&file, width as usize, height as usize, &pixels)
                        .map_err(|error| format!("{}: {error}", file.display()))?;
                    taken += 1;
                    let next = every * taken;
                    if let Some(wait) = next.checked_sub(started.elapsed()) {
                        std::thread::sleep(wait);
                    }
                }
                println!("  {taken} pictures in {}", folder.display());
                return Ok(());
            }
            ("settle", [time]) => {
                // How long the steps after this wait for what they start, or `default` for as long as they otherwise do.
                let said = if time.text == "default" {
                    u64::MAX
                } else {
                    time.text.parse::<u64>().map_err(|_| format!("{:?} isn't a time or default", time.text))?
                };
                SETTLE_SAID.store(said, std::sync::atomic::Ordering::Relaxed);
                return Ok(());
            }
            ("wheel", _) if (2..=4).contains(&arguments.len()) => {
                // The wheel turned by DX DY pixels, as a browser's wheel event has it, at a place or where the pointer is.
                // The wheel goes in notches of 100/3 pixels, as Windows' rows are, so that's what it's rounded to.
                let (place, deltas) = arguments.split_at(arguments.len() - 2);
                let ((x, y), used) = self.place(place)?;
                if used != place.len() {
                    return Err("wheel takes [X Y | css:SELECTOR] DX DY".to_owned());
                }
                let number = |word: &Word| word.text.parse::<f32>().map_err(|_| format!("{:?} isn't a number", word.text));
                let rows = |pixels: f32| (pixels * 3.0 / 100.0).round() as i64;
                let (across, down) = (rows(number(&deltas[0])?), rows(number(&deltas[1])?));
                let one = self.usable()?;
                if down != 0 {
                    one.app.scroll(x, y, down).map_err(|error| error.to_string())?;
                }
                if across != 0 {
                    one.app.scroll_across(x, y, across).map_err(|error| error.to_string())?;
                }
            }
            ("expect", _) => {
                let (text, absent, rest) = looking_for(arguments)?;
                if !rest.is_empty() {
                    return Err("expect only takes the text to look for".to_owned());
                }
                // What's there right now is what this is about, and may not be there once things have settled. It's
                // only if it isn't that what's going on gets a chance to bring it about.
                self.pass()?;
                if self.says(&text)? == absent {
                    self.settle(step_settles_for())?;
                }
                if self.says(&text)? == absent {
                    return Err(if absent {
                        format!("{text:?} is there, and it shouldn't be")
                    } else {
                        format!("{text:?} isn't there")
                    });
                }
                self.print_changes(false);
                return Ok(());
            }
            ("wait", [time]) if !time.quoted && time.text.parse::<u64>().is_ok() => {
                let wanted = Duration::from_millis(time.text.parse().expect("that was checked"));
                let started = Instant::now();
                while started.elapsed() < wanted {
                    self.pass()?;
                    // Often while something's going on, and otherwise only as often as a screen would show anything, so that
                    // waiting doesn't take more of the processor than the app would in a window.
                    let nap = if self.busy() { 2 } else { frame_nap() };
                    let nap = match frame_log::period() {
                        // To the next time the screen would show something.
                        Some(period) => {
                            let gone = started.elapsed().as_secs_f64();
                            let next = ((gone / period.as_secs_f64()).floor() + 1.0) * period.as_secs_f64();
                            Duration::from_secs_f64(next - gone)
                        }
                        None => Duration::from_millis(nap),
                    };
                    std::thread::sleep(nap.min(wanted.saturating_sub(started.elapsed())));
                }
            }
            ("wait", _) => {
                let (text, absent, rest) = looking_for(arguments)?;
                let longest = match rest.as_slice() {
                    [] => WAIT_LOOKS_FOR,
                    [time] => Duration::from_millis(
                        time.text
                            .parse()
                            .map_err(|_| format!("{:?} isn't a time", time.text))?,
                    ),
                    _ => return Err("wait takes text to look for, and how long".to_owned()),
                };
                let started = Instant::now();
                loop {
                    self.pass()?;
                    if self.says(&text)? != absent {
                        break;
                    }
                    if started.elapsed() >= longest {
                        return Err(format!(
                            "{text:?} {} after {} ms",
                            if absent {
                                "was still there"
                            } else {
                                "hadn't shown up"
                            },
                            longest.as_millis()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            _ => return Err(format!("that's not a step. {STEPS}")),
        }
        if !self.settle(step_settles_for())? && step_settles_for() >= STEP_SETTLES_FOR {
            println!(
                "  (something's still going after a second, such as a timer. `wait` is for that)"
            );
        }
        self.print_changes(true);
        Ok(())
    }

    fn choose_window(&mut self, which: &Word) -> Result<(), String> {
        self.pass()?;
        let found = if which.quoted {
            self.windows.iter().find(|one| one.title == which.text)
        } else {
            let id = which.text.parse::<u32>().ok();
            self.windows.iter().find(|one| Some(one.id) == id)
        };
        let Some(found) = found else {
            let open: Vec<String> = self
                .windows
                .iter()
                .map(|one| format!("{} {:?}", one.id, one.title))
                .collect();
            return Err(format!(
                "there's no such window. These are open: {}",
                open.join(", ")
            ));
        };
        self.current = found.id;
        Ok(())
    }

    /// Says what's happened since this last said, one line at a time, and if nothing has, says that if it's asked to.
    fn print_changes(&mut self, even_if_nothing: bool) {
        let mut lines = std::mem::take(&mut self.notes);
        let mut styling = 0;
        for one in &mut self.windows {
            let root = one.app.runtime().root().clone();
            let Some(seen) = one.seen.replace(root.clone()) else {
                lines.push(format!("window {} {:?} opened", one.id, one.title));
                continue;
            };
            for patch in diff(&seen, &root) {
                if only_changes_styling(&patch) {
                    styling += 1;
                    continue;
                }
                for line in describe_patch(&patch).lines() {
                    lines.push(if one.id == WindowId::MAIN.0 {
                        line.to_owned()
                    } else {
                        format!("window {}: {line}", one.id)
                    });
                }
            }
        }
        if lines.is_empty() && styling > 0 {
            println!("  only how things look changed, in {styling} ways. `shot` shows that");
        } else if lines.is_empty() && even_if_nothing {
            println!("  nothing changed");
        }
        for line in lines.iter().take(MOST_CHANGES) {
            println!("  {line}");
        }
        if lines.len() > MOST_CHANGES {
            println!(
                "  (+{} more lines of changes. `look` shows it all)",
                lines.len() - MOST_CHANGES
            );
        }
    }

    /// Prints what's in every window, which is everything there is to know about it.
    fn print_all(&mut self) {
        // That's seen, now.
        for one in &mut self.windows {
            one.seen = Some(one.app.runtime().root().clone());
        }
        if self.windows.is_empty() {
            println!("the app has closed");
        }
        for one in &self.windows {
            if one.id != WindowId::MAIN.0 {
                println!("\nwindow {} {:?}:", one.id, one.title);
            }
            println!("{}", one.app.outline());
        }
    }
}

fn node_at<'a>(root: &'a Node, path: &[u32]) -> Option<&'a Node> {
    path.iter()
        .try_fold(root, |node, &index| node.children.get(index as usize))
}

/// Whether what's at `path` can be reached by a person: neither it nor anything it's in is inert, which is what's only there to
/// be seen, as what's on its way out, `visibility: hidden` and `pointer-events: none` are, and which isn't read out either.
fn reachable(root: &Node, path: &[u32]) -> bool {
    let mut node = root;
    for &index in path {
        let Some(child) = node.children.get(index as usize) else {
            return false;
        };
        if is_on(child, "inert") {
            return false;
        }
        node = child;
    }
    true
}

fn is_on(node: &Node, prop: &str) -> bool {
    node.props.get(prop) == Some(&PropValue::Bool(true))
}

/// What's held with a click: any of `ctrl`, `shift`, `alt` and `win`.
fn parse_held(words: &[Word]) -> Result<Modifiers, String> {
    let mut held = Modifiers::default();
    for word in words {
        match word.text.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => held.ctrl = true,
            "shift" => held.shift = true,
            "alt" => held.alt = true,
            "win" | "meta" => held.meta = true,
            other => {
                return Err(format!(
                    "{other:?} isn't something that can be held. That's ctrl, shift, alt or win"
                ));
            }
        }
    }
    Ok(held)
}

/// What a key's called: `enter`, `a`, `f5`, `shift`, `ctrl+shift+s`. It gives the key, and any character that typing it makes.
fn parse_key(key: &str) -> Result<(KeyPress, Option<char>), String> {
    let lower = key.to_ascii_lowercase();
    let mut parts: Vec<&str> = lower.split('+').collect();
    let name = parts.pop().unwrap_or_default();
    let code = match name {
        "tab" => KeyCode::Tab,
        "enter" => KeyCode::Enter,
        "escape" | "esc" => KeyCode::Escape,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "space" => KeyCode::Space,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "insert" => KeyCode::Insert,
        "shift" => KeyCode::Shift,
        "ctrl" | "control" => KeyCode::Control,
        "alt" => KeyCode::Alt,
        "win" | "meta" => KeyCode::Meta,
        "capslock" => KeyCode::CapsLock,
        "menu" | "contextmenu" => KeyCode::ContextMenu,
        _ => {
            let mut characters = name.chars();
            let function = name
                .strip_prefix('f')
                .and_then(|number| number.parse::<u8>().ok())
                .filter(|number| (1..=24).contains(number));
            match (characters.next(), characters.next(), function) {
                (_, _, Some(number)) => KeyCode::Function(number),
                (Some(character), None, _) => KeyCode::Character(character),
                _ => {
                    return Err(format!(
                        "{key:?} isn't a key. They look like enter, escape, tab, left, pagedown, f5, shift, a or ctrl+shift+s"
                    ));
                }
            }
        }
    };
    let mut press = KeyPress::new(code);
    for part in parts {
        press = match part {
            "ctrl" | "control" => press.ctrl(),
            "shift" => press.shift(),
            "alt" => press.alt(),
            "win" | "meta" => press.meta(),
            _ => {
                return Err(format!(
                    "{key:?} has {part:?} in it, which isn't ctrl, shift, alt or win"
                ));
            }
        };
    }
    let typed = match code {
        _ if press.ctrl || press.alt || press.meta => None,
        KeyCode::Space => Some(' '),
        KeyCode::Character(character) if press.shift => character.to_uppercase().next(),
        KeyCode::Character(character) => Some(character),
        _ => None,
    };
    Ok((press, typed))
}

/// What can be said to mean something.
const TARGETS: &str = "A target is #key, @path, or \"text\" it's called. Those are all in what `look` prints. A page's \
     element can be #id too, as its `id` says.";

/// Where the first of a page's elements with this `id` is, which it has as `dom_id`.
fn path_with_dom_id(node: &Node, id: &str, path: &mut Vec<u32>) -> bool {
    static DOM_ID: std::sync::LazyLock<rsui::core::Symbol> =
        std::sync::LazyLock::new(|| rsui::core::Symbol::new("dom_id"));
    if matches!(node.props.get_symbol(*DOM_ID), Some(PropValue::Str(text)) if text.as_str() == id) {
        return true;
    }
    for (index, child) in node.children.iter().enumerate() {
        path.push(index as u32);
        if path_with_dom_id(child, id, path) {
            return true;
        }
        path.pop();
    }
    false
}

/// The component that made something, as far as what's kept says.
pub(crate) struct MadeBy {
    /// What it's called, if it's rendered since inspecting started.
    pub(crate) name: Option<String>,
    /// The components it's in, outermost first.
    pub(crate) around: Vec<String>,
    /// Why it last rendered.
    pub(crate) why: Option<String>,
    /// Its hooks, each with what it holds.
    pub(crate) hooks: Vec<String>,
}

/// Which component made what's at `path`, what it's in, why it rendered and what its hooks hold.
pub(crate) fn made_by(app: &App, path: &[u32]) -> MadeBy {
    let runtime = app.runtime();
    let (component, why) = app.why_at(path);
    let name_of = |component: (u32, u32)| {
        app.frames()
            .flat_map(|frame| frame.renders.iter())
            .rev()
            .find(|render| (render.component, render.generation) == component)
            .and_then(|render| render.name.as_ref().map(ToString::to_string))
    };
    let mut around = Vec::new();
    let mut at = component;
    while let Some(parent) = runtime.component_parent(at.0, at.1) {
        if let Some(name) = name_of(parent) {
            around.push(name);
        }
        at = parent;
    }
    around.reverse();
    MadeBy {
        name: name_of(component),
        around,
        why,
        hooks: bridge::hooks_of(runtime.id(), component).unwrap_or_default(),
    }
}

/// What's said when a step makes no sense, and what `--help` says about them.
/// With `RSUI_SPLIT` set, where the time went since this was last said, part by part, each as milliseconds and times
/// entered: see `split.rs` in rsui-alloc.
fn print_split() {
    if !rsui::split::on() {
        return;
    }
    let parts: Vec<String> = rsui::split::take()
        .into_iter()
        .map(|(name, nanos, times)| format!("{name}={:.3}/{times}", nanos as f64 / 1e6))
        .collect();
    println!("  split {}", parts.join(" "));
}

pub(crate) const STEPS: &str = "The steps are: look, click TARGET [ctrl] [shift] [alt] [win], drag TARGET FROM TO [STEPS], dragto TARGET TARGET [STEPS], glide TARGET X0 Y0 X1 Y1 MOVES PER_SECOND [WATCH], dropfiles TARGET PATH..., droptext TARGET TEXT, rightclick TARGET, middleclick TARGET, hover TARGET, move TARGET, type TARGET TEXT, write TEXT, compose TEXT, press KEY [TARGET], scroll TARGET ROWS, wheelat X Y ROWS, spin X Y ROWS COUNT EVERY_MS, burst move|down|up|dblclick X Y [wheel X Y ROWS]..., act TARGET ACTION [INPUT], expect [no] \"TEXT\", wait MILLISECONDS, wait [no] \"TEXT\" [MILLISECONDS], window ID, window \"TITLE\", close, dialog \"PATH\", dialog cancel, size WIDTH HEIGHT, animations on|off (transitions and CSS animations play from that step on; off unless a step says, so shots come out the same), active on|off, shown on|off, shot FILE, rects FILE, clickat X Y [ctrl] [shift] [alt] [win], rightclickat X Y, hoverat X Y, moveat X Y, downat X Y, upat X Y, clicksat X Y CLICKS, wheel [X Y] DX DY (where X Y can be css:SELECTOR for a page's element, or left out for where the pointer is), eval \"CODE\", evalto FILE \"CODE\", waitjs \"CODE\" [MILLISECONDS], shots FOLDER MILLISECONDS [EVERY], settle MILLISECONDS|default. To see why things render: why TARGET, frames, profile, save FILE.rsuitrace, history, goto CHANGE, logpoint FILE:LINE \"EXPRESSION\", logpoint off FILE:LINE|all, logpoints, break FILE:LINE, break off FILE:LINE|all, breakpoints, inspect on|off|show|hide, record on|off (sample where JavaScript is, for save). A target is #key, @path, or \"text\" it's called. For what the desktop would do: global KEYS, tray [ITEM], menubar ITEM, popup ITEM, thumb BUTTON, media KEY, notification, soundends, secondinstance [ARGUMENT...], power WHAT. For what a web page in a <WebView> would do: webview load URL, webview inpage URL, webview fail URL [CODE], webview title TEXT, webview popup URL, webview key KEYS, webview fullscreen on|off, webview message JSON, webview close, webview crash, webview unavailable";

/// A word of a step, with whether it was in quotes, which is what makes `"#1"` some text rather than a key.
#[derive(Debug, Clone)]
struct Word {
    text: String,
    quoted: bool,
}

impl Word {
    /// As an action's input.
    fn as_prop(&self) -> PropValue {
        if !self.quoted {
            match self.text.as_str() {
                "true" => return PropValue::Bool(true),
                "false" => return PropValue::Bool(false),
                _ => {}
            }
            if let Ok(number) = self.text.parse::<i64>() {
                return PropValue::Int(number);
            }
            if let Ok(number) = self.text.parse::<f64>() {
                return PropValue::Float(number);
            }
        }
        self.text.as_str().into()
    }
}

/// One thing to do, and how it was written, to say which went wrong.
#[derive(Debug)]
struct Step {
    written: String,
    words: Vec<Word>,
}

/// Reads steps, which end at a new line or a `;` outside quotes. A line starting with `//` says nothing.
fn parse_steps(text: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    let mut words = Vec::new();
    let mut written = String::new();
    let mut characters = text.chars().peekable();
    let mut word: Option<Word> = None;
    let end_word = |word: &mut Option<Word>, words: &mut Vec<Word>| {
        words.extend(word.take());
    };
    while let Some(character) = characters.next() {
        // A comment takes the rest of the line, but only where a step could start.
        if character == '/' && words.is_empty() && word.is_none() && characters.peek() == Some(&'/')
        {
            for skipped in characters.by_ref() {
                if skipped == '\n' {
                    break;
                }
            }
            continue;
        }
        if matches!(character, ';' | '\n' | '\r') {
            end_word(&mut word, &mut words);
            if !words.is_empty() {
                steps.push(Step {
                    written: written.trim().to_owned(),
                    words: std::mem::take(&mut words),
                });
            }
            written.clear();
            continue;
        }
        written.push(character);
        if character == '"' && word.is_none() {
            let mut inside = String::new();
            loop {
                let Some(next) = characters.next() else {
                    return Err(format!("{written} has a quote that never ends"));
                };
                written.push(next);
                match next {
                    '"' => break,
                    // Only a quote or a backslash is written with one before it, so that a path like `C:\new\tasks` is
                    // typed as it's written.
                    '\\' if matches!(characters.peek(), Some('"' | '\\')) => {
                        let escaped = characters.next().unwrap_or('\\');
                        written.push(escaped);
                        inside.push(escaped);
                    }
                    other => inside.push(other),
                }
            }
            words.push(Word {
                text: inside,
                quoted: true,
            });
        } else if character.is_whitespace() {
            end_word(&mut word, &mut words);
        } else {
            word.get_or_insert_with(|| Word {
                text: String::new(),
                quoted: false,
            })
            .text
            .push(character);
        }
    }
    end_word(&mut word, &mut words);
    if !words.is_empty() {
        steps.push(Step {
            written: written.trim().to_owned(),
            words,
        });
    }
    Ok(steps)
}

/// Runs an app on this thread, and does these steps to it, printing what happens. Afterwards it prints what's there,
/// and the same if a step fails, as that's what it takes to work out why.
pub(crate) fn run_steps(path: &Path, stack_limit: usize, steps: &str) -> Result<(), String> {
    let steps = parse_steps(steps)?;
    if steps.is_empty() {
        return Err(format!("there are no steps. {STEPS}"));
    }
    // `RSUI_SAMPLE=<file>` samples the steps as it does a window, in a build with `sampling-profile`, from before the app's first
    // render, which is most of what a long page costs to start. What finishes it is made after the app, so it's dropped, and
    // writes what it found, while the engine and its compiled code are still there.
    #[cfg(all(windows, feature = "sampling-profile"))]
    let sampling = crate::sampling_profile::NativeProfile::start();
    let mut desk = Desk::open(path, stack_limit, (960, 640))?;
    #[cfg(all(windows, feature = "sampling-profile"))]
    let _profile = sampling.map(crate::sampling_profile::FinishOnDrop::new);
    rsui::mark("app made");
    rsui_js::write_startup_times("app made");
    // Things that move as the app starts only do if they're moving by then, as they are in a window.
    let animate_first = steps.first().is_some_and(|step| {
        step.words.len() == 2 && step.words[0].text == "animations" && step.words[1].text == "on"
    });
    if animate_first {
        desk.set_animations(true);
    }
    desk.settle(STEP_SETTLES_FOR)?;
    desk.note_motion();
    // Whatever opens straight away is just there, rather than something a step did.
    for one in &mut desk.windows {
        one.seen = Some(one.app.runtime().root().clone());
    }
    // How long each step took, with what it changed laid out and drawn, for measuring.
    let timed = std::env::var_os("RSUI_STEP_TIMES").is_some();
    // What starting took isn't any step's.
    let _ = rsui::split::take();
    // With `RSUI_DO_KEEP_GOING` set, a step that fails is said, and the next one is done anyway, for going over many things
    // at once, such as every component of a kit.
    let keep_going = std::env::var_os("RSUI_DO_KEEP_GOING").is_some();
    let mut failures = Vec::new();
    for (number, step) in steps.iter().enumerate() {
        println!("> {}", step.written);
        // What the engine runs is counted by step, with the `counts` feature.
        #[cfg(feature = "counts")]
        crate::bridge::count_mark(&step.written);
        let started = Instant::now();
        let done = desk.step(step);
        frame_log::note("step", started, format_args!("{}", step.written));
        desk.note_motion();
        if number == 0 {
            rsui::mark("first step done");
            rsui_js::write_startup_times("first step done");
        }
        if timed {
            println!("  took {:.3} ms", started.elapsed().as_secs_f64() * 1000.0);
        }
        print_split();
        // Between one thing someone does and the next, as a window is once its frame is up (`rsui::Quiet::Frame`).
        crate::bridge::between_frames();
        if let Err(reason) = done {
            if keep_going {
                println!("  FAILED step {}: {reason}", number + 1);
                failures.push(format!("step {}: {}: {reason}", number + 1, step.written));
                continue;
            }
            println!();
            desk.print_all();
            desk.hint_motion();
            return Err(format!(
                "step {} failed: {}: {reason}",
                number + 1,
                step.written
            ));
        }
    }
    #[cfg(feature = "counts")]
    crate::bridge::count_mark("after the steps");
    desk.hint_motion();
    if steps.last().is_none_or(|step| step.words[0].text != "look") && !keep_going {
        println!();
        desk.print_all();
    }
    if !failures.is_empty() {
        return Err(format!("{} steps failed:\n{}", failures.len(), failures.join("\n")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Vec<Vec<(String, bool)>> {
        parse_steps(text)
            .unwrap()
            .into_iter()
            .map(|step| {
                step.words
                    .into_iter()
                    .map(|word| (word.text, word.quoted))
                    .collect()
            })
            .collect()
    }

    fn plain(word: &str) -> (String, bool) {
        (word.to_owned(), false)
    }

    fn quoted(word: &str) -> (String, bool) {
        (word.to_owned(), true)
    }

    #[test]
    fn steps_end_at_a_semicolon_or_a_new_line_but_not_in_quotes() {
        assert_eq!(
            read("click #add; type #name \"a; b\"\r\n\n  look  "),
            [
                vec![plain("click"), plain("#add")],
                vec![plain("type"), plain("#name"), quoted("a; b")],
                vec![plain("look")],
            ]
        );
    }

    #[test]
    fn quotes_can_have_quotes_and_backslashes_in_them() {
        assert_eq!(
            read(r#"expect "say \"hi\" there\\""#),
            [vec![plain("expect"), quoted("say \"hi\" there\\")]]
        );
        // Any other backslash is as it's written, as in a Windows path.
        assert_eq!(
            read(r#"type "path" "C:\Users\me\new\tasks""#),
            [vec![
                plain("type"),
                quoted("path"),
                quoted(r"C:\Users\me\new\tasks")
            ]]
        );
    }

    #[test]
    fn comments_say_nothing_but_a_slash_in_a_step_does() {
        assert_eq!(
            read("// first\nshot a//b.png\n// last"),
            [vec![plain("shot"), plain("a//b.png")]]
        );
    }

    #[test]
    fn a_quote_that_never_ends_says_where() {
        let error = parse_steps("look; click \"oops").unwrap_err();
        assert!(error.contains("click \"oops"), "{error}");
    }

    #[test]
    fn keys_are_read_as_they_are_pressed() {
        let (press, typed) = parse_key("ctrl+shift+S").unwrap();
        assert_eq!(
            press.describe(),
            KeyPress::new(KeyCode::Character('s'))
                .ctrl()
                .shift()
                .describe()
        );
        assert_eq!(typed, None);
        assert_eq!(parse_key("a").unwrap().1, Some('a'));
        assert_eq!(parse_key("shift+a").unwrap().1, Some('A'));
        assert_eq!(parse_key("space").unwrap().1, Some(' '));
        assert_eq!(parse_key("enter").unwrap().1, None);
        assert_eq!(parse_key("alt+a").unwrap().0.describe(), "alt+a");
        assert_eq!(parse_key("alt+a").unwrap().1, None);
        assert_eq!(parse_key("f5").unwrap().0.code, KeyCode::Function(5));
        assert_eq!(parse_key("shift").unwrap().0.code, KeyCode::Shift);
        assert_eq!(parse_key("f").unwrap().0.code, KeyCode::Character('f'));
        assert!(parse_key("f25").is_err());
        assert!(parse_key("hyper+a").is_err());
        assert!(parse_key("banana").is_err());
    }
}
