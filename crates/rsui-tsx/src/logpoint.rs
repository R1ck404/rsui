//! Logpoints: an expression on a line of the app's code, whose value is said each time that line runs, with nothing stopping.
//!
//! A call is put in front of the line as the file's read, with nothing added to the line count, and the app takes in the new
//! code as it does when a file's saved, keeping its state. Only a line that starts a statement can have one, which is checked
//! by reading the file with it before anything runs, so a logpoint can't break the app.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use rsui_js::{Args, Value, Vm};

/// How many times a logpoint says what it found before it only counts.
const MOST_SAID: u32 = 100;
/// How long a value it says can be.
const LONGEST_VALUE: usize = 120;

struct Logpoint {
    file: PathBuf,
    line: u32,
    expression: String,
    hits: u32,
}

thread_local! {
    /// By number, which is where each is in this, with what's been taken away left as nothing.
    static LOGPOINTS: RefCell<Vec<Option<Logpoint>>> = const { RefCell::new(Vec::new()) };
}

/// What a file's called, the same way however it's written.
fn same_file(path: &Path) -> String {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

/// Where a file is, from how it's written in a step: as it is, or from the app's folder.
pub(crate) fn find_file(file: &str) -> Result<PathBuf, String> {
    let given = Path::new(file);
    let from_app = crate::bridge::app_file()
        .parent()
        .map(|folder| folder.join(given))
        .filter(|path| path.is_file());
    match from_app {
        Some(path) => Ok(path),
        None if given.is_file() => Ok(given.to_owned()),
        None => Err(format!(
            "there's no file {file}, from the app's folder or from here"
        )),
    }
}

/// Puts a call to a logpoint at the start of line `line`, if it's where a statement starts. Lines don't move.
fn put_in(source: &str, line: u32, call: &str) -> Result<String, String> {
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    let at = (line as usize)
        .checked_sub(1)
        .filter(|at| *at < lines.len())
        .ok_or_else(|| format!("the file has {} lines", lines.len()))?;
    let text = lines[at];
    let first = text.trim_start();
    let starts_well = first
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$' || c == '[' || c == '(');
    // What comes before has to have ended, or this is part of something, as text inside JSX or an argument would be.
    let before_ended = lines[..at]
        .iter()
        .rev()
        .map(|line| line.trim())
        .find(|line| !line.is_empty() && !line.starts_with("//"))
        .is_none_or(|line| line.ends_with(';') || line.ends_with('{') || line.ends_with('}'));
    if !starts_well || !before_ended {
        return Err(format!(
            "line {line} doesn't start a statement, so there's nowhere for a logpoint on it. A line that does, such as one \
             starting with `const`, `return` or a call, works"
        ));
    }
    let indent = text.len() - first.len();
    let mut out = String::with_capacity(source.len() + call.len());
    for (index, part) in lines.iter().enumerate() {
        if index == at {
            out.push_str(&part[..indent]);
            out.push_str(call);
            out.push_str(&part[indent..]);
        } else {
            out.push_str(part);
        }
    }
    Ok(out)
}

fn call_for(id: usize, expression: &str) -> String {
    format!("__rsuiLogpoint({id}, () => ({expression}));")
}

/// Adds a logpoint, checking it fits where it's going, and has the app take in the code with it. Says where it is.
pub(crate) fn add(at: &str, expression: &str) -> Result<String, String> {
    let (file, line) = at
        .rsplit_once(':')
        .and_then(|(file, line)| Some((file, line.parse::<u32>().ok()?)))
        .ok_or_else(|| format!("{at:?} isn't a place. It's a file and a line, as in app.tsx:40"))?;
    if expression.trim().is_empty() {
        return Err(
            "say what to show there, as in `logpoint app.tsx:40 \"items.length\"`".to_owned(),
        );
    }
    let path = find_file(file)?;
    let source = std::fs::read_to_string(&path).map_err(|error| format!("{file}: {error}"))?;
    let id = LOGPOINTS.with(|points| points.borrow().len());
    let changed = put_in(&source, line, &call_for(id, expression))?;
    if let Err(problem) = rsui_js::check_syntax(&changed) {
        if rsui_js::check_syntax(&source).is_ok() {
            return Err(format!(
                "with that there, line {} wouldn't make sense: {}. It's either not where a statement starts, or `{expression}` isn't an expression",
                problem.line, problem.message
            ));
        }
    }
    LOGPOINTS.with(|points| {
        points.borrow_mut().push(Some(Logpoint {
            file: path.clone(),
            line,
            expression: expression.to_owned(),
            hits: 0,
        }));
    });
    take_in_new_code();
    Ok(format!("{}:{line}", short(&path)))
}

/// Takes away the logpoints on a line, or all of them for `all`. Says how many it took away.
pub(crate) fn remove(at: &str) -> Result<usize, String> {
    let wanted = if at == "all" {
        None
    } else {
        let (file, line) = at
            .rsplit_once(':')
            .and_then(|(file, line)| Some((file, line.parse::<u32>().ok()?)))
            .ok_or_else(|| {
                format!("{at:?} isn't a place. It's a file and a line, as in app.tsx:40, or all")
            })?;
        Some((same_file(&find_file(file)?), line))
    };
    let removed = LOGPOINTS.with(|points| {
        let mut removed = 0;
        for slot in points.borrow_mut().iter_mut() {
            let matches = slot.as_ref().is_some_and(|point| {
                wanted.as_ref().is_none_or(|(file, line)| {
                    same_file(&point.file) == *file && point.line == *line
                })
            });
            if matches {
                *slot = None;
                removed += 1;
            }
        }
        removed
    });
    if removed > 0 {
        take_in_new_code();
    }
    Ok(removed)
}

/// The logpoints there are: where each is, what it says, and how many times it's run.
pub(crate) fn entries() -> Vec<(String, String, u32)> {
    LOGPOINTS.with(|points| {
        points
            .borrow()
            .iter()
            .flatten()
            .map(|point| {
                (
                    format!("{}:{}", short(&point.file), point.line),
                    point.expression.clone(),
                    point.hits,
                )
            })
            .collect()
    })
}

/// The logpoints there are, a line each, with how many times each has run.
pub(crate) fn list() -> String {
    let lines: Vec<String> = LOGPOINTS.with(|points| {
        points
            .borrow()
            .iter()
            .flatten()
            .map(|point| {
                format!(
                    "{}:{} {} ({})",
                    short(&point.file),
                    point.line,
                    point.expression,
                    match point.hits {
                        0 => "hasn't run".to_owned(),
                        1 => "ran once".to_owned(),
                        hits => format!("ran {hits} times"),
                    }
                )
            })
            .collect()
    });
    if lines.is_empty() {
        "there are no logpoints. `logpoint app.tsx:40 \"items.length\"` adds one".to_owned()
    } else {
        lines.join("\n")
    }
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

/// Has files go through [`filter`] while there are logpoints, and not otherwise, as a packed program reads its files ahead
/// unless something has to change them, and has the app take in the code.
fn take_in_new_code() {
    let any = LOGPOINTS.with(|points| points.borrow().iter().any(Option::is_some));
    rsui_js::set_source_filter(any.then(|| std::rc::Rc::new(filter) as rsui_js::SourceFilter));
    crate::why::reloading_because("A logpoint was added or taken away, so its code ran again with that.");
    crate::bridge::request_reload(&crate::bridge::app_file());
}

/// What's read from a file before it runs, with its logpoints put in.
pub(crate) fn filter(path: &Path, source: &mut String) {
    let file = same_file(path);
    LOGPOINTS.with(|points| {
        for (id, point) in points.borrow().iter().enumerate() {
            let Some(point) = point else {
                continue;
            };
            if same_file(&point.file) == file {
                if let Ok(changed) = put_in(source, point.line, &call_for(id, &point.expression)) {
                    *source = changed;
                }
            }
        }
    });
}

/// `__rsuiLogpoint(id, read)`: a line with a logpoint is running, so this says what `read` gives.
pub(crate) fn host_logpoint(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = args.get(vm, 0).as_number() as usize;
    let read = args.get(vm, 1);
    let Some((place, expression, hits)) = LOGPOINTS.with(|points| {
        let mut points = points.borrow_mut();
        let point = points.get_mut(id)?.as_mut()?;
        point.hits += 1;
        Some((
            format!("{}:{}", short(&point.file), point.line),
            point.expression.clone(),
            point.hits,
        ))
    }) else {
        return Ok(Value::UNDEFINED);
    };
    if hits > MOST_SAID {
        return Ok(Value::UNDEFINED);
    }
    let shown = match vm.call_function(read, Value::UNDEFINED, &[]) {
        Ok(value) => vm.describe(value, LONGEST_VALUE),
        Err(error) => {
            let text = vm.error_text(error);
            format!("threw {}", text.lines().next().unwrap_or(""))
        }
    };
    let mut said = format!("logpoint {place}: {expression} = {shown}");
    if hits == MOST_SAID {
        said.push_str(&format!(
            " (that's {MOST_SAID}, so from here it's only counted, which `logpoints` shows)"
        ));
    }
    crate::bridge::say(said);
    Ok(Value::UNDEFINED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_logpoint_goes_in_front_of_a_statement_without_moving_lines() {
        let source = "function App() {\n    const items = load();\n    return items;\n}\n";
        let changed = put_in(source, 3, "log();").unwrap();
        assert_eq!(
            changed,
            "function App() {\n    const items = load();\n    log();return items;\n}\n"
        );
        assert_eq!(changed.lines().count(), source.lines().count());
    }

    #[test]
    fn a_line_that_is_part_of_something_has_no_room_for_one() {
        let jsx = "return (\n    <window>\n        Some text\n    </window>\n);\n";
        assert!(put_in(jsx, 3, "log();").is_err(), "text inside JSX");
        assert!(put_in(jsx, 2, "log();").is_err(), "an element");
        let call = "show(\n    first,\n    second);\n";
        assert!(put_in(call, 2, "log();").is_err(), "an argument");
        assert!(put_in(call, 9, "log();").is_err(), "past the end");
    }
}
