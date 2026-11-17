//! Breakpoints: lines the app stops at, so what it has in its variables can be looked at, and it can be taken on a
//! statement at a time.
//!
//! The app's own files (those in its folder, but not in `node_modules`) are read again with somewhere to stop at each
//! statement once the first breakpoint's set, or a pause is asked for, and the app takes in that code as it does when a
//! file's saved, keeping its state. Nothing else is read again, and until then nothing is any slower. See
//! `rsui_js::inspect` for how stopping works.
//!
//! Stopping is for a viewer that's looking at the app, which says when to carry on (see `live.rs`). With nothing looking,
//! as with `--do`, a breakpoint says where it stopped and what the variables had in them, and the app carries straight on,
//! so a window is never left stopped with no way to start it again.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rsui_js::{Pause, PauseReason, Resume, Vm};
use serde_json::{Value, json};

/// How long a value of a variable can be, as a pause shows it.
const LONGEST_VALUE: usize = 200;
/// How long what an expression worked out to can be.
const LONGEST_WORKED_OUT: usize = 4000;
/// How many functions out from the innermost a pause shows.
const MOST_FRAMES: usize = 30;
/// How many lines of the file a pause shows, before and after the one it's on.
const LINES_AROUND: usize = 6;

struct Breakpoint {
    file: PathBuf,
    line: u32,
    hits: u32,
}

thread_local! {
    static BREAKPOINTS: RefCell<Vec<Breakpoint>> = const { RefCell::new(Vec::new()) };
    static DEBUGGING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Where it last stopped, and what was there, as a viewer was told.
    static LAST_STOP: RefCell<Option<Value>> = const { RefCell::new(None) };
}

/// Where the app last stopped on this thread, as a viewer was told, if it has.
#[cfg(test)]
pub(crate) fn last_stop() -> Option<Value> {
    LAST_STOP.with(|kept| kept.borrow().clone())
}

/// Whether the app's files have somewhere to stop in them.
pub(crate) fn debugging() -> bool {
    DEBUGGING.with(std::cell::Cell::get)
}

/// Whether a file is one of the app's own, which are the ones that are debugged.
fn is_apps_own(path: &Path, folder: &Path) -> bool {
    let text = rsui_js::same_file(&path.to_string_lossy());
    let folder = rsui_js::same_file(&folder.to_string_lossy());
    text.starts_with(&folder) && !text.contains("/node_modules/")
}

/// The folder the app's main file is in.
fn app_folder() -> PathBuf {
    std::path::absolute(crate::bridge::app_file())
        .ok()
        .and_then(|main| main.parent().map(Path::to_path_buf))
        .unwrap_or_default()
}

/// Has the app's own files read with somewhere to stop in them from now on, and has the app stop when it's asked to.
/// With `take_in`, the app takes in its code again now, keeping its state, so what's running has them.
pub(crate) fn start(take_in: bool) {
    if debugging() {
        return;
    }
    DEBUGGING.with(|on| on.set(true));
    let folder = app_folder();
    rsui_js::set_debug_files(Some(Rc::new(move |path: &Path| {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
        is_apps_own(&path, &folder)
    })));
    rsui_js::set_pause_handler(Some(Rc::new(stopped)));
    if take_in {
        crate::bridge::keep_state_across_reloads();
        crate::why::reloading_because(
            "The debugger was started, so the app's code ran again with places to stop in it.",
        );
        crate::bridge::request_reload(&crate::bridge::app_file());
    }
}

fn place_of(at: &str) -> Result<(PathBuf, u32), String> {
    let (file, line) = at
        .rsplit_once(':')
        .and_then(|(file, line)| Some((file, line.parse::<u32>().ok()?)))
        .ok_or_else(|| format!("{at:?} isn't a place. It's a file and a line, as in app.tsx:40"))?;
    let path = crate::logpoint::find_file(file)?;
    let path = std::path::absolute(&path).unwrap_or(path);
    Ok((path, line))
}

fn sync() {
    let at: Vec<(String, u32)> = BREAKPOINTS.with(|points| {
        points
            .borrow()
            .iter()
            .map(|point| (point.file.to_string_lossy().into_owned(), point.line))
            .collect()
    });
    rsui_js::set_breakpoints(&at);
}

/// Adds a breakpoint, and starts debugging if it hasn't started. Says where it is.
pub(crate) fn add(at: &str) -> Result<String, String> {
    let (file, line) = place_of(at)?;
    let lines = std::fs::read_to_string(&file)
        .map_err(|error| format!("{}: {error}", file.display()))?
        .lines()
        .count();
    if line == 0 || line as usize > lines {
        return Err(format!("{at} isn't in the file, which has {lines} lines"));
    }
    if !is_apps_own(&file, &app_folder()) {
        return Err(format!(
            "{at} is in a package, and only the app's own files are debugged"
        ));
    }
    let place = format!("{}:{line}", short(&file));
    let there = BREAKPOINTS.with(|points| {
        points
            .borrow()
            .iter()
            .any(|point| point.file == file && point.line == line)
    });
    if !there {
        BREAKPOINTS.with(|points| points.borrow_mut().push(Breakpoint { file, line, hits: 0 }));
    }
    sync();
    start(true);
    Ok(place)
}

/// Takes away the breakpoint on a line, or all of them for `all`. Says how many it took away.
pub(crate) fn remove(at: &str) -> Result<usize, String> {
    let wanted = if at == "all" {
        None
    } else {
        Some(place_of(at)?)
    };
    let removed = BREAKPOINTS.with(|points| {
        let mut points = points.borrow_mut();
        let before = points.len();
        points.retain(|point| {
            wanted
                .as_ref()
                .is_some_and(|(file, line)| !(point.file == *file && point.line == *line))
        });
        before - points.len()
    });
    sync();
    Ok(removed)
}

/// The breakpoints there are: where each is, and how many times it's been stopped at.
pub(crate) fn entries() -> Vec<(String, u32)> {
    BREAKPOINTS.with(|points| {
        points
            .borrow()
            .iter()
            .map(|point| (format!("{}:{}", short(&point.file), point.line), point.hits))
            .collect()
    })
}

/// The breakpoints there are, a line each.
pub(crate) fn list() -> String {
    let lines: Vec<String> = entries()
        .into_iter()
        .map(|(at, hits)| match hits {
            0 => format!("{at} (hasn't stopped)"),
            1 => format!("{at} (stopped once)"),
            hits => format!("{at} (stopped {hits} times)"),
        })
        .collect();
    if lines.is_empty() {
        "there are no breakpoints. `break app.tsx:40` adds one".to_owned()
    } else {
        lines.join("\n")
    }
}

/// Has the app stop at the next statement of its own that runs, as a viewer's Pause does.
pub(crate) fn pause() {
    start(true);
    rsui_js::pause_soon();
}

fn short(path: &Path) -> String {
    crate::why::short_place(&rsui_js::Origin {
        function: String::new(),
        file: path.to_string_lossy().into_owned(),
        line: 0,
    })
    .trim_end_matches(":0")
    .to_owned()
}

/// What a pause says it's stopped for.
fn reason_text(reason: PauseReason) -> &'static str {
    match reason {
        PauseReason::Breakpoint => "breakpoint",
        PauseReason::Step => "step",
        PauseReason::Asked => "paused",
    }
}

/// Where the program's stopped and what it has there, as a viewer's told.
pub(crate) fn pause_json(vm: &mut Vm, pause: &Pause) -> Value {
    let frames: Vec<Value> = vm
        .paused_frames(MOST_FRAMES)
        .into_iter()
        .map(|frame| {
            let locals: Vec<Value> = frame
                .locals
                .iter()
                .map(|(name, value)| json!({ "name": name, "value": vm.describe(*value, LONGEST_VALUE) }))
                .collect();
            json!({
                "at": crate::why::short_place(&frame.at),
                "function": frame.at.function,
                "debugged": frame.debugged,
                "locals": locals,
            })
        })
        .collect();
    let source = std::fs::read_to_string(&pause.at.file).unwrap_or_default();
    let line = pause.at.line as usize;
    let first = line.saturating_sub(LINES_AROUND).max(1);
    let shown: Vec<Value> = source
        .lines()
        .enumerate()
        .skip(first - 1)
        .take(LINES_AROUND * 2 + 1)
        .map(|(index, text)| json!({ "line": index + 1, "text": text }))
        .collect();
    json!({
        "reason": reason_text(pause.reason),
        "at": crate::why::short_place(&pause.at),
        "function": pause.at.function,
        "source": shown,
        "frames": frames,
    })
}

/// Works out an expression in the `frame`th function out from where the program's stopped, as a viewer asks.
pub(crate) fn evaluate(vm: &mut Vm, frame: usize, expression: &str) -> Result<String, String> {
    match vm.evaluate_in_frame(frame, expression) {
        Ok(value) => Ok(vm.describe(value, LONGEST_WORKED_OUT)),
        Err(error) => {
            let text = vm.error_text(error);
            Err(format!("threw {}", text.lines().next().unwrap_or("")))
        }
    }
}

/// How a step's asked for, as a viewer says it.
pub(crate) fn resume_of(how: &str) -> Option<Resume> {
    Some(match how {
        "continue" => Resume::Continue,
        "over" => Resume::Over,
        "into" => Resume::Into,
        "out" => Resume::Out,
        _ => return None,
    })
}

/// The program's stopped: a viewer that's looking says when to carry on, and with none, what's there is said.
fn stopped(vm: &mut Vm, pause: &Pause) -> Resume {
    if pause.reason == PauseReason::Breakpoint {
        let file = rsui_js::same_file(&pause.at.file);
        BREAKPOINTS.with(|points| {
            for point in points.borrow_mut().iter_mut() {
                if point.line == pause.at.line
                    && rsui_js::same_file(&point.file.to_string_lossy()) == file
                {
                    point.hits += 1;
                }
            }
        });
    }
    let shown = pause_json(vm, pause);
    LAST_STOP.with(|kept| *kept.borrow_mut() = Some(shown.clone()));
    if crate::live::viewer_looking() {
        return crate::live::wait_while_paused(vm, shown);
    }
    let locals: Vec<String> = shown["frames"][0]["locals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|local| local["name"] != "this")
        .map(|local| {
            format!(
                "{} = {}",
                local["name"].as_str().unwrap_or_default(),
                local["value"].as_str().unwrap_or_default()
            )
        })
        .collect();
    let function = match pause.at.function.as_str() {
        "" => String::new(),
        name => format!(" in {name}"),
    };
    crate::bridge::say(format!(
        "{} at {}{function}: {}",
        reason_text(pause.reason),
        shown["at"].as_str().unwrap_or_default(),
        if locals.is_empty() {
            "nothing in its variables".to_owned()
        } else {
            locals.join(", ")
        }
    ));
    Resume::Continue
}
