//! Replaying what an app was given: the inputs a window kept (see `rsui::replay`), given again to a new copy of the app,
//! from the start, with its clock saying what it said each time and `Math.random` starting where it did.
//!
//! An app being worked on picks where `Math.random` starts, and notes when it started, and its first window keeps what
//! comes in. A trace has that in `replay`. `--replay TRACE` runs the app again with it, as far as `--to N` says, and
//! then goes on as usual from there, with its clock going on from where it got to. The viewer runs one with `--pause`,
//! which stops at the first statement of the app's own that runs for input N, so what that input did can be gone
//! through a statement at a time, as many times as it takes.
//!
//! What the app reads from elsewhere isn't kept: a file, the network, or what another thread worked out. An app that
//! does the same with the same inputs at the same times does the same again.

use std::cell::RefCell;
use std::path::PathBuf;

use rsui::access::AccessCommand;
use rsui::replay::{Input, Recorded};
use rsui::{App, DropPhase, KeyCode, KeyPress, Modifiers, PointerButton};
use serde_json::{Value, json};

/// Which trace a run replays, how many of its inputs, and whether to pause at the last one.
pub(crate) const REPLAY_VAR: &str = "RSUI_REPLAY";
pub(crate) const TO_VAR: &str = "RSUI_REPLAY_TO";
pub(crate) const PAUSE_VAR: &str = "RSUI_REPLAY_PAUSE";

/// How many timers going off between two inputs a replay goes through, for a trace that doesn't say when they went off,
/// before it takes them to be going off forever.
const MOST_TIMERS_BETWEEN: usize = 100_000;
/// The most times timers going off are kept, as for inputs.
const MOST_TIMES_KEPT: usize = 400_000;

/// When the app started by its clock, and where `Math.random` started.
#[derive(Clone, Copy, Default)]
struct Start {
    at: f64,
    seed: u64,
}

thread_local! {
    static START: RefCell<Option<Start>> = const { RefCell::new(None) };
    static PLAN: RefCell<Option<Plan>> = const { RefCell::new(None) };
    /// A replay asked for here rather than by the environment, as a test does, which only the next app on this thread has.
    static WANTED: RefCell<Option<Wanted>> = const { RefCell::new(None) };
    /// When JavaScript timers went off, by the app's clock, as they go off when the app gets to them, which is a little
    /// after they're due, and an interval's next is from then.
    static TIMERS: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
    /// Whether timers can go off now: always, except in a replay, which says when.
    static TIMERS_NOW: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// Whether JavaScript's timers can go off now.
pub(crate) fn may_run_timers() -> bool {
    TIMERS_NOW.with(std::cell::Cell::get)
}

/// Notes that timers went off just now, for a replay to have them go off then too.
pub(crate) fn timers_ran() {
    if START.with(|kept| kept.borrow().is_none()) {
        return;
    }
    TIMERS.with(|kept| {
        let mut kept = kept.borrow_mut();
        if kept.len() < MOST_TIMES_KEPT {
            kept.push(rsui_js::now_ms());
        }
    });
}

/// Which trace to replay, how far, and whether to pause at the last input.
struct Wanted {
    trace: PathBuf,
    to: Option<usize>,
    pause: bool,
}

/// What the environment says to replay, as `--replay` and the viewer say it.
fn wanted() -> Option<Wanted> {
    if let Some(wanted) = WANTED.with(|kept| kept.borrow_mut().take()) {
        return Some(wanted);
    }
    Some(Wanted {
        trace: PathBuf::from(std::env::var_os(REPLAY_VAR)?),
        to: std::env::var(TO_VAR)
            .ok()
            .and_then(|to| to.parse::<usize>().ok()),
        pause: std::env::var_os(PAUSE_VAR).is_some(),
    })
}

/// Has the next app started on this thread replay a trace.
#[cfg(test)]
pub(crate) fn ask_for(trace: PathBuf, to: Option<usize>, pause: bool) {
    WANTED.with(|kept| *kept.borrow_mut() = Some(Wanted { trace, to, pause }));
}

/// What a replay gives the app, and before which of it it's to pause.
pub(crate) struct Plan {
    inputs: Vec<Recorded>,
    /// When timers went off, if the trace says, in order.
    timers: Option<Vec<f64>>,
    pause_before: Option<usize>,
}

/// Gets an app that's about to load ready: as a replay says, if one's asked for, or else with a start of its own noted.
/// It's said before anything of the app runs.
pub(crate) fn begin() -> Result<(), String> {
    rsui::replay::set_clock(rsui_js::now_ms);
    let start = match wanted() {
        Some(wanted) => {
            let (start, plan) = read(&wanted)?;
            if plan.pause_before.is_some() {
                // Its code has to be read with places to stop in it from the start, as there's no taking it in again
                // halfway through a replay.
                crate::debugger::start(false);
            }
            rsui_js::set_time(Some(start.at));
            PLAN.with(|kept| *kept.borrow_mut() = Some(plan));
            start
        }
        None => Start {
            at: rsui_js::now_ms(),
            seed: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |since| since.as_nanos() as u64),
        },
    };
    rsui_js::set_random_seed(Some(start.seed));
    START.with(|kept| *kept.borrow_mut() = Some(start));
    Ok(())
}

/// The replay asked for, if there's one, which is only given once.
pub(crate) fn take_plan() -> Option<Plan> {
    PLAN.with(|kept| kept.borrow_mut().take())
}

/// Reads what's to be replayed from a trace, as far as it's wanted.
fn read(wanted: &Wanted) -> Result<(Start, Plan), String> {
    let trace = wanted.trace.as_path();
    let text =
        std::fs::read_to_string(trace).map_err(|error| format!("{}: {error}", trace.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} isn't a trace: {error}", trace.display()))?;
    let replay = &value["replay"];
    if !replay.is_object() {
        return Err(format!(
            "{} has nothing to replay. It was saved by an app run with rsui-tsx app.tsx, with a window, or `save` with --do",
            trace.display()
        ));
    }
    let start = Start {
        at: replay["started"].as_f64().unwrap_or(0.0),
        seed: replay["seed"].as_u64().unwrap_or(1),
    };
    let mut inputs: Vec<Recorded> = replay["inputs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(input_from)
        .collect();
    if let Some(to) = wanted.to {
        inputs.truncate(to.saturating_add(1));
    }
    let pause_before = wanted.pause.then(|| inputs.len().saturating_sub(1));
    // What went off after the last input given is for the app to do by itself, once it's going on as usual.
    let last = inputs.last().map_or(f64::NEG_INFINITY, |one| one.at);
    let timers = replay["timers"].as_array().map(|timers| {
        timers
            .iter()
            .filter_map(Value::as_f64)
            .filter(|at| *at <= last)
            .collect()
    });
    Ok((start, Plan { inputs, timers, pause_before }))
}

/// Gives the app what the plan has, each at the time it came, with the timers that went off in between going off then
/// too. The clock goes on from the last of them at its usual speed afterwards. Says how many inputs it gave.
pub(crate) fn play(app: &mut App, plan: &Plan) -> usize {
    let size = app.size();
    let scale = app.scale();
    // What it does as it starts, such as setting a timer going in an effect, it did before anything came in.
    TIMERS_NOW.with(|now| now.set(false));
    app.update();
    let mut timers = plan.timers.as_deref().unwrap_or_default().iter().peekable();
    for (index, one) in plan.inputs.iter().enumerate() {
        if plan.timers.is_some() {
            // The timers that went off before this input, each when it did. One that went off at the same time as
            // the input went off first, as a replay of a replay has them.
            while let Some(at) = timers.next_if(|at| **at <= one.at) {
                rsui_js::set_time(Some(at.max(rsui_js::now_ms())));
                TIMERS_NOW.with(|now| now.set(true));
                crate::bridge::pump_soon();
                app.update();
                TIMERS_NOW.with(|now| now.set(false));
            }
        } else {
            timers_until(app, one.at);
        }
        rsui_js::set_time(Some(one.at.max(rsui_js::now_ms())));
        if plan.pause_before == Some(index) {
            crate::debugger::pause();
        }
        let _ = app.feed(&one.input);
        app.update();
    }
    TIMERS_NOW.with(|now| now.set(true));
    rsui_js::set_time(None);
    // The window's as big as it is, whatever it was when what's been replayed came in.
    if app.size() != size || app.scale() != scale {
        app.resize(size, scale);
        app.update();
    }
    plan.inputs.len()
}

/// Has the JavaScript timers due by `at` go off, each at the time it's due.
fn timers_until(app: &mut App, at: f64) {
    for _ in 0..MOST_TIMERS_BETWEEN {
        let Some(due) = crate::bridge::next_timer_at() else {
            return;
        };
        if due > at {
            return;
        }
        rsui_js::set_time(Some(due.max(rsui_js::now_ms())));
        TIMERS_NOW.with(|now| now.set(true));
        crate::bridge::pump_soon();
        app.update();
        TIMERS_NOW.with(|now| now.set(false));
    }
}

/// What a trace keeps for a replay of the app: its start, and what came in to its first window.
pub(crate) fn json(app: &App) -> Value {
    let Some(start) = START.with(|kept| *kept.borrow()) else {
        return Value::Null;
    };
    let (inputs, full) = app.recorded_inputs();
    let timers = TIMERS.with(|kept| kept.borrow().clone());
    json!({
        "started": start.at,
        "seed": start.seed,
        "full": full,
        "inputs": inputs.iter().map(input_json).collect::<Vec<_>>(),
        "timers": timers,
    })
}

/// What came in, from `after` on, each with its number, when it came after the start, and what it was, for the viewer.
pub(crate) fn listed(app: &App, after: usize) -> Value {
    let started = START.with(|kept| kept.borrow().map_or(0.0, |start| start.at));
    let (inputs, full) = app.recorded_inputs();
    let listed: Vec<Value> = inputs
        .iter()
        .enumerate()
        .skip(after)
        .map(|(number, one)| {
            json!({
                "number": number,
                "at": ((one.at - started) * 10.0).round() / 10.0,
                "kind": kind(&one.input),
                "said": describe(&one.input),
            })
        })
        .collect();
    json!({ "inputs": listed, "count": inputs.len(), "full": full })
}

/// Starts the app again in a new process, replaying what came in up to input `to`, and pausing at it if `pause` says.
/// Says the new process's number.
pub(crate) fn again(app: &App, to: usize, pause: bool) -> Result<u32, String> {
    if START.with(|kept| kept.borrow().is_none()) {
        return Err("this app doesn't keep what comes in".to_owned());
    }
    let count = app.recorded_inputs().0.len();
    if to >= count {
        return Err(format!("there are {count} inputs, numbered from 0"));
    }
    let trace = std::env::temp_dir().join(format!(
        "rsui-replay-{}-{}.rsuitrace",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis())
    ));
    let whole = crate::trace_file::trace_of(app, &crate::file_title(&crate::bridge::app_file()));
    std::fs::write(&trace, whole.to_string())
        .map_err(|error| format!("{}: {error}", trace.display()))?;
    let program = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut command = std::process::Command::new(program);
    command
        .arg("--once")
        .arg(app_path())
        .env(REPLAY_VAR, &trace)
        .env(TO_VAR, to.to_string())
        .env_remove(PAUSE_VAR);
    if pause {
        command.env(PAUSE_VAR, "1");
    }
    let child = command.spawn().map_err(|error| format!("it couldn't be started: {error}"))?;
    Ok(child.id())
}

fn app_path() -> PathBuf {
    let main = crate::bridge::app_file();
    std::path::absolute(&main).unwrap_or(main)
}

fn kind(input: &Input) -> &'static str {
    match input {
        Input::Move { .. } => "move",
        Input::Leave => "leave",
        Input::Down { .. } => "down",
        Input::Up { .. } => "up",
        Input::ButtonDown { .. } => "buttondown",
        Input::ButtonUp { .. } => "buttonup",
        Input::CancelPress => "cancel",
        Input::Active(_) => "active",
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

/// An input as a person would say it.
fn describe(input: &Input) -> String {
    let at = |x: &f32, y: &f32| format!("at {}, {}", x.round(), y.round());
    match input {
        Input::Move { x, y } => format!("moved {}", at(x, y)),
        Input::Leave => "left the window".to_owned(),
        Input::Down { x, y, clicks } => match clicks {
            1 => format!("pressed {}", at(x, y)),
            _ => format!("pressed {clicks} times {}", at(x, y)),
        },
        Input::Up { x, y } => format!("let go {}", at(x, y)),
        Input::ButtonDown { x, y, button } => format!("pressed the {} button {}", button_name(*button), at(x, y)),
        Input::ButtonUp { x, y, button } => format!("let go of the {} button {}", button_name(*button), at(x, y)),
        Input::CancelPress => "a press was called off".to_owned(),
        Input::Active(true) => "the window came to the front".to_owned(),
        Input::Active(false) => "another window came to the front".to_owned(),
        Input::Modifiers(held) => {
            let names = held_names(*held);
            if names.is_empty() {
                "nothing held".to_owned()
            } else {
                format!("holding {}", names.join("+"))
            }
        }
        Input::Key(press) => format!("key {}", press.describe()),
        Input::KeyUp(press) => format!("let go of {}", press.describe()),
        Input::Text(text) => format!("typed {text:?}"),
        Input::Composition { phase, text } => format!("composing ({phase}) {text:?}"),
        Input::Scroll { rows, across, .. } => format!(
            "scrolled {} {}",
            rows.abs(),
            match (across, *rows < 0) {
                (false, true) => "up",
                (false, false) => "down",
                (true, true) => "left",
                (true, false) => "right",
            }
        ),
        Input::ScrollPixels { pixels, across, precise, .. } => format!(
            "scrolled {pixels} pixels {}{}",
            if *across { "across" } else { "vertically" },
            if *precise { " precisely" } else { "" },
        ),
        Input::Resize { width, height, .. } => format!("made {width} by {height}"),
        Input::Files { paths, .. } => format!("dropped {}", paths.join(", ")),
        Input::DragFromOutside { phase, .. } => format!("dragged in from outside ({phase:?})"),
        Input::DarkMode(dark) => if *dark { "dark colors" } else { "light colors" }.to_owned(),
        Input::Access { command, .. } => format!("assistive technology asked for {}", command_name(command)),
        Input::Close => "asked to close".to_owned(),
    }
}

fn command_name(command: &AccessCommand) -> &'static str {
    match command {
        AccessCommand::Invoke => "invoke",
        AccessCommand::Toggle => "toggle",
        AccessCommand::SetValue(_) => "setvalue",
        AccessCommand::Focus => "focus",
        AccessCommand::ScrollTo(_) => "scrollto",
        AccessCommand::ScrollSteps(_) => "scrollsteps",
        AccessCommand::ScrollPages(_) => "scrollpages",
        AccessCommand::Reveal => "reveal",
    }
}

fn command_json(command: &AccessCommand) -> Value {
    let mut value = json!({ "command": command_name(command) });
    match command {
        AccessCommand::SetValue(text) => value["value"] = json!(text),
        AccessCommand::ScrollTo(along) => value["value"] = json!(along),
        AccessCommand::ScrollSteps(by) | AccessCommand::ScrollPages(by) => value["value"] = json!(by),
        _ => {}
    }
    value
}

fn command_from(value: &Value) -> Option<AccessCommand> {
    let by = || value["value"].as_i64().map(|by| by as i32);
    Some(match value["command"].as_str()? {
        "invoke" => AccessCommand::Invoke,
        "toggle" => AccessCommand::Toggle,
        "setvalue" => AccessCommand::SetValue(value["value"].as_str()?.to_owned()),
        "focus" => AccessCommand::Focus,
        "scrollto" => AccessCommand::ScrollTo(value["value"].as_f64()?),
        "scrollsteps" => AccessCommand::ScrollSteps(by()?),
        "scrollpages" => AccessCommand::ScrollPages(by()?),
        "reveal" => AccessCommand::Reveal,
        _ => return None,
    })
}

fn held_names(held: Modifiers) -> Vec<&'static str> {
    [(held.ctrl, "ctrl"), (held.alt, "alt"), (held.shift, "shift"), (held.meta, "win")]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect()
}

fn button_name(button: PointerButton) -> &'static str {
    match button {
        PointerButton::Main => "main",
        PointerButton::Middle => "middle",
        PointerButton::Secondary => "secondary",
        PointerButton::Back => "back",
        PointerButton::Forward => "forward",
    }
}

fn button_of(name: &str) -> Option<PointerButton> {
    Some(match name {
        "main" => PointerButton::Main,
        "middle" => PointerButton::Middle,
        "secondary" => PointerButton::Secondary,
        "back" => PointerButton::Back,
        "forward" => PointerButton::Forward,
        _ => return None,
    })
}

fn phase_name(phase: DropPhase) -> &'static str {
    match phase {
        DropPhase::Enter => "enter",
        DropPhase::Over => "over",
        DropPhase::Leave => "leave",
        DropPhase::Drop => "drop",
    }
}

fn phase_of(name: &str) -> Option<DropPhase> {
    Some(match name {
        "enter" => DropPhase::Enter,
        "over" => DropPhase::Over,
        "leave" => DropPhase::Leave,
        "drop" => DropPhase::Drop,
        _ => return None,
    })
}

/// The keys that have a name, and the names, both ways.
const NAMED_KEYS: &[(KeyCode, &str)] = &[
    (KeyCode::Tab, "tab"),
    (KeyCode::Enter, "enter"),
    (KeyCode::Escape, "escape"),
    (KeyCode::Backspace, "backspace"),
    (KeyCode::Delete, "delete"),
    (KeyCode::Space, "space"),
    (KeyCode::Left, "left"),
    (KeyCode::Right, "right"),
    (KeyCode::Up, "up"),
    (KeyCode::Down, "down"),
    (KeyCode::Home, "home"),
    (KeyCode::End, "end"),
    (KeyCode::PageUp, "pageup"),
    (KeyCode::PageDown, "pagedown"),
    (KeyCode::Insert, "insert"),
    (KeyCode::Shift, "shift"),
    (KeyCode::Control, "control"),
    (KeyCode::Alt, "alt"),
    (KeyCode::Meta, "meta"),
    (KeyCode::CapsLock, "capslock"),
    (KeyCode::NumLock, "numlock"),
    (KeyCode::ScrollLock, "scrolllock"),
    (KeyCode::Pause, "pause"),
    (KeyCode::PrintScreen, "printscreen"),
    (KeyCode::ContextMenu, "contextmenu"),
    (KeyCode::Process, "process"),
];

fn key_name(code: KeyCode) -> String {
    match code {
        KeyCode::Character(character) => format!("char:{character}"),
        KeyCode::Function(number) => format!("f{number}"),
        _ => NAMED_KEYS
            .iter()
            .find(|(named, _)| *named == code)
            .map_or_else(|| "unknown".to_owned(), |(_, name)| (*name).to_owned()),
    }
}

fn key_of(name: &str) -> Option<KeyCode> {
    if let Some(character) = name.strip_prefix("char:") {
        return character.chars().next().map(KeyCode::Character);
    }
    if let Some(found) = NAMED_KEYS.iter().find(|(_, named)| *named == name) {
        return Some(found.0);
    }
    name.strip_prefix('f')
        .and_then(|number| number.parse::<u8>().ok())
        .map(KeyCode::Function)
}

fn press_json(press: &KeyPress) -> Value {
    json!({
        "code": key_name(press.code),
        "ctrl": press.ctrl,
        "shift": press.shift,
        "alt": press.alt,
        "meta": press.meta,
        "repeat": press.repeat,
        "scan": press.scan,
        "typed": press.typed.map(String::from),
    })
}

fn press_from(value: &Value) -> Option<KeyPress> {
    let mut press = KeyPress::new(key_of(value["code"].as_str()?)?);
    press.ctrl = value["ctrl"].as_bool().unwrap_or(false);
    press.shift = value["shift"].as_bool().unwrap_or(false);
    press.alt = value["alt"].as_bool().unwrap_or(false);
    press.meta = value["meta"].as_bool().unwrap_or(false);
    press.repeat = value["repeat"].as_bool().unwrap_or(false);
    press.scan = value["scan"].as_u64().unwrap_or(0) as u16;
    press.typed = value["typed"].as_str().and_then(|typed| typed.chars().next());
    Some(press)
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|one| one.as_str().map(str::to_owned))
        .collect()
}

/// An input as a trace keeps it.
pub(crate) fn input_json(one: &Recorded) -> Value {
    let mut value = match &one.input {
        Input::Move { x, y } | Input::Up { x, y } => json!({ "x": x, "y": y }),
        Input::Down { x, y, clicks } => json!({ "x": x, "y": y, "clicks": clicks }),
        Input::ButtonDown { x, y, button } | Input::ButtonUp { x, y, button } => {
            json!({ "x": x, "y": y, "button": button_name(*button) })
        }
        Input::Leave | Input::CancelPress | Input::Close => json!({}),
        Input::Active(on) | Input::DarkMode(on) => json!({ "on": on }),
        Input::Modifiers(held) => {
            json!({ "ctrl": held.ctrl, "shift": held.shift, "alt": held.alt, "meta": held.meta })
        }
        Input::Key(press) | Input::KeyUp(press) => press_json(press),
        Input::Text(text) => json!({ "text": text }),
        Input::Composition { phase, text } => json!({ "phase": phase, "text": text }),
        Input::Scroll { x, y, rows, across } => json!({ "x": x, "y": y, "rows": rows, "across": across }),
        Input::ScrollPixels { x, y, pixels, across, precise } => {
            json!({ "x": x, "y": y, "pixels": pixels, "across": across, "precise": precise })
        }
        Input::Resize { width, height, scale } => json!({ "width": width, "height": height, "scale": scale }),
        Input::Files { x, y, paths } => json!({ "x": x, "y": y, "paths": paths }),
        Input::DragFromOutside { phase, x, y, files, text } => {
            json!({ "phase": phase_name(*phase), "x": x, "y": y, "files": files, "text": text })
        }
        Input::Access { path, command } => {
            let mut value = command_json(command);
            value["path"] = json!(path);
            value
        }
    };
    value["kind"] = json!(kind(&one.input));
    value["at"] = json!(one.at);
    value
}

/// An input from how a trace keeps it, if it's one this knows.
pub(crate) fn input_from(value: &Value) -> Option<Recorded> {
    let number = |name: &str| value[name].as_f64().map(|number| number as f32);
    let (x, y) = (number("x").unwrap_or(0.0), number("y").unwrap_or(0.0));
    let on = value["on"].as_bool().unwrap_or(false);
    let input = match value["kind"].as_str()? {
        "move" => Input::Move { x, y },
        "leave" => Input::Leave,
        "down" => Input::Down {
            x,
            y,
            clicks: value["clicks"].as_u64().unwrap_or(1) as u32,
        },
        "up" => Input::Up { x, y },
        "buttondown" => Input::ButtonDown {
            x,
            y,
            button: button_of(value["button"].as_str()?)?,
        },
        "buttonup" => Input::ButtonUp {
            x,
            y,
            button: button_of(value["button"].as_str()?)?,
        },
        "cancel" => Input::CancelPress,
        "active" => Input::Active(on),
        "held" => Input::Modifiers(Modifiers {
            ctrl: value["ctrl"].as_bool().unwrap_or(false),
            shift: value["shift"].as_bool().unwrap_or(false),
            alt: value["alt"].as_bool().unwrap_or(false),
            meta: value["meta"].as_bool().unwrap_or(false),
        }),
        "key" => Input::Key(press_from(value)?),
        "keyup" => Input::KeyUp(press_from(value)?),
        "text" => Input::Text(value["text"].as_str()?.to_owned()),
        "composition" => Input::Composition {
            phase: value["phase"].as_str()?.to_owned(),
            text: value["text"].as_str().unwrap_or_default().to_owned(),
        },
        "scroll" => Input::Scroll {
            x,
            y,
            rows: value["rows"].as_i64()?,
            across: value["across"].as_bool().unwrap_or(false),
        },
        "scrollpixels" => Input::ScrollPixels {
            x,
            y,
            pixels: number("pixels")?,
            across: value["across"].as_bool().unwrap_or(false),
            precise: value["precise"].as_bool().unwrap_or(true),
        },
        "resize" => Input::Resize {
            width: value["width"].as_u64()? as u32,
            height: value["height"].as_u64()? as u32,
            scale: number("scale").unwrap_or(1.0),
        },
        "files" => Input::Files {
            x,
            y,
            paths: strings(&value["paths"]),
        },
        "drag" => Input::DragFromOutside {
            phase: phase_of(value["phase"].as_str()?)?,
            x,
            y,
            files: strings(&value["files"]),
            text: value["text"].as_str().map(str::to_owned),
        },
        "dark" => Input::DarkMode(on),
        "access" => Input::Access {
            path: value["path"]
                .as_array()?
                .iter()
                .map(|step| step.as_u64().map(|step| step as u32))
                .collect::<Option<Vec<_>>>()?,
            command: command_from(value)?,
        },
        "close" => Input::Close,
        _ => return None,
    };
    Some(Recorded {
        at: value["at"].as_f64()?,
        input,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_typed_into_a_password_is_not_kept() {
        let path = std::env::temp_dir().join(format!("rsui-replay-password-{}.tsx", std::process::id()));
        std::fs::write(
            &path,
            "export default function App() { return <window><input key=\"secret\" password /><input key=\"name\" /></window>; }",
        )
        .unwrap();
        let typed = on_its_own({
            let path = path.clone();
            move || {
                let mut desk = crate::drive::Desk::open(&path, crate::STACK / 8 * 7, (400, 300)).unwrap();
                desk.run("type #secret hunter2; type #name ann").unwrap();
                let (inputs, _) = desk.windows[0].app.recorded_inputs();
                inputs
                    .iter()
                    .filter_map(|one| match &one.input {
                        Input::Text(text) => Some(text.clone()),
                        Input::Key(press) => press.typed.map(String::from),
                        _ => None,
                    })
                    .collect::<String>()
            }
        });
        let _ = std::fs::remove_file(&path);
        assert!(!typed.contains("hunter"), "{typed}");
        assert!(typed.contains('*') && typed.contains("ann"), "{typed}");
    }

    #[test]
    fn inputs_come_back_from_a_trace_as_they_went_in() {
        let mut press = KeyPress::new(KeyCode::Character('a')).ctrl();
        press.scan = 30;
        press.typed = Some('a');
        let inputs = [
            Input::Move { x: 1.5, y: 2.0 },
            Input::Leave,
            Input::Down { x: 3.0, y: 4.0, clicks: 2 },
            Input::Up { x: 3.0, y: 4.0 },
            Input::ButtonDown { x: 5.0, y: 6.0, button: PointerButton::Secondary },
            Input::ButtonUp { x: 5.0, y: 6.0, button: PointerButton::Secondary },
            Input::CancelPress,
            Input::Active(true),
            Input::Modifiers(Modifiers { ctrl: true, shift: false, alt: true, meta: false }),
            Input::Key(press),
            Input::KeyUp(KeyPress::new(KeyCode::Function(5))),
            Input::Key(KeyPress::new(KeyCode::PageDown)),
            Input::Text("héllo".to_owned()),
            Input::Composition { phase: "update".to_owned(), text: "日本".to_owned() },
            Input::Scroll { x: 7.0, y: 8.0, rows: -3, across: true },
            Input::ScrollPixels { x: 7.5, y: 8.25, pixels: -0.625, across: true, precise: true },
            Input::ScrollPixels { x: 7.5, y: 8.25, pixels: 100.0, across: false, precise: false },
            Input::Resize { width: 800, height: 600, scale: 1.5 },
            Input::Files { x: 9.0, y: 10.0, paths: vec!["C:\\a.txt".to_owned()] },
            Input::DragFromOutside { phase: DropPhase::Over, x: 1.0, y: 2.0, files: Vec::new(), text: Some("t".to_owned()) },
            Input::DarkMode(true),
            Input::Access { path: vec![0, 2], command: AccessCommand::SetValue("x".to_owned()) },
            Input::Access { path: vec![1], command: AccessCommand::ScrollSteps(-2) },
            Input::Close,
        ];
        for (at, input) in inputs.into_iter().enumerate() {
            let one = Recorded { at: 1000.0 + at as f64, input };
            assert_eq!(input_from(&input_json(&one)).as_ref(), Some(&one));
        }
    }

    const APP: &str = "import { useState, useEffect } from 'rsui';
export default function App() {
    const [count, setCount] = useState(0);
    const [ticks, setTicks] = useState(0);
    const [lucky] = useState(() => Math.floor(Math.random() * 1000000));
    const [seen, setSeen] = useState([]);
    useEffect(() => {
        const timer = setInterval(() => setTicks((ticks) => ticks + 1), 100);
        return () => clearInterval(timer);
    }, []);
    return (
        <window>
            <button key=\"more\" onClick={() => { setCount(count + 1); setSeen([...seen, ticks]); }}>More</button>
            <text key=\"shown\">{`count ${count} lucky ${lucky} seen ${seen.join('-')}`}</text>
        </window>
    );
}
";

    /// What the app shows: its count, the random number it picked, and how many times its timer had gone off at each click.
    fn shown(desk: &crate::drive::Desk) -> (u32, u32, String) {
        let outline = desk.windows[0].app.outline();
        let at = outline.find("count ").unwrap_or_else(|| panic!("{outline}"));
        let words: Vec<&str> = outline[at..]
            .split(|c: char| !c.is_ascii_alphanumeric() && c != ' ' && c != '-')
            .next()
            .unwrap_or_default()
            .split(' ')
            .collect();
        let number = |at: usize| words[at].parse::<u32>().unwrap_or_else(|_| panic!("{words:?}"));
        (number(1), number(3), words.get(5).copied().unwrap_or_default().to_owned())
    }

    /// Runs `run` with an app of its own, on a thread of its own, as each app has.
    fn on_its_own<T: Send + 'static>(run: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(crate::STACK)
            .spawn(run)
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn a_replay_gives_the_app_what_came_in_with_its_clock_and_random_numbers() {
        let folder = std::env::temp_dir().join(format!("rsui-replay-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("app.tsx");
        std::fs::write(&path, APP).unwrap();
        let trace = folder.join("app.rsuitrace");

        let (first, inputs) = on_its_own({
            let (path, trace) = (path.clone(), trace.clone());
            move || {
                let mut desk = crate::drive::Desk::open(&path, crate::STACK / 8 * 7, (400, 300)).unwrap();
                desk.settle(std::time::Duration::from_secs(2)).unwrap();
                desk.run("click #more; wait 450; click #more; click #more").unwrap();
                let shown = shown(&desk);
                let whole = crate::trace_file::trace_of(&desk.windows[0].app, "app");
                std::fs::write(&trace, whole.to_string()).unwrap();
                (shown, whole["replay"]["inputs"].as_array().unwrap().clone())
            }
        });
        assert_eq!(first.0, 3);
        let ticks: Vec<u32> = first.2.split('-').map(|tick| tick.parse().unwrap()).collect();
        assert!(ticks[1] >= ticks[0] + 4, "the timer went off while it waited: {first:?}");
        let clicks: Vec<usize> = inputs
            .iter()
            .enumerate()
            .filter(|(_, input)| input["kind"] == "buttonup")
            .map(|(number, _)| number)
            .collect();
        assert_eq!(clicks.len(), 3, "{inputs:?}");

        let replay = |to: Option<usize>| {
            let (path, trace) = (path.clone(), trace.clone());
            on_its_own(move || {
                ask_for(trace, to, false);
                let desk = crate::drive::Desk::open(&path, crate::STACK / 8 * 7, (400, 300)).unwrap();
                shown(&desk)
            })
        };
        let again = replay(None);
        let partway = replay(Some(clicks[0]));
        // Stopping in what an input did stops at the first statement that runs for it, which is the button's.
        let stopped = on_its_own({
            let (path, trace) = (path.clone(), trace.clone());
            let second = clicks[1];
            move || {
                ask_for(trace, Some(second), true);
                let desk = crate::drive::Desk::open(&path, crate::STACK / 8 * 7, (400, 300)).unwrap();
                (shown(&desk), crate::debugger::last_stop())
            }
        });
        let _ = std::fs::remove_dir_all(&folder);
        // The same clicks, the same random number, and the timer had gone off as many times at each click, as it goes
        // off at the times it did by the clock it had then.
        assert_eq!(again, first);
        assert_eq!(partway, (1, first.1, ticks[0].to_string()), "up to the first click");
        let (stopped_shown, stop) = stopped;
        let stop = stop.expect("it stopped");
        assert_eq!(stop["at"], "app.tsx:13", "{stop}");
        assert_eq!(stop["reason"], "paused", "{stop}");
        let locals = stop["frames"][1]["locals"].as_array().cloned().unwrap_or_default();
        assert!(
            stop["frames"][0]["locals"]
                .as_array()
                .unwrap()
                .iter()
                .chain(&locals)
                .any(|local| local["name"] == "count" && local["value"] == "1"),
            "what it had when it was clicked the second time: {stop}"
        );
        assert_eq!(stopped_shown.0, 2, "it carries on from there, with nothing looking");
    }
}
