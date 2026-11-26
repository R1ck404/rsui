//! `pty` in `rsui/desktop`: programs run in a pseudo terminal, which an app that draws a terminal reads and types into. Like the
//! rest of the desktop, it only really happens for a program in a real window. While one's being checked with `--do` or
//! `--agent`, a pty is pretended: nothing runs, what the app types is kept for `pty written`, and the steps `pty out` and
//! `pty exit` make the program print and end. `RSUI_PTY=real` has `--do` run them for real, for checking an app against a shell.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rsui::pty::{Pty, PtyOptions};
use rsui_js::{Args, HostFunction, Value, Vm};

use crate::bridge;
use crate::native::{failed, number_at, real, text_at};

/// How many reads of printed text (of up to 64 KB each) may be waiting for the app to take them before the program is made to wait.
const MOST_WAITING: usize = 16;

enum Running {
    Real { pty: Arc<Pty>, waiting: Arc<AtomicUsize> },
    Pretend { data: u32, exit: u32, written: Vec<String>, size: (u16, u16), ended: bool },
}

thread_local! {
    static PTYS: RefCell<BTreeMap<u32, Running>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT: Cell<u32> = const { Cell::new(1) };
}

fn really() -> bool {
    real() || std::env::var("RSUI_PTY").is_ok_and(|how| how == "real")
}

/// `ptySpawn(json, dataId, exitId)`: its number. The json is `{ command, cwd, env, cols, rows }`. `dataId` is told what it prints,
/// as text, and `exitId` its exit code when it's over.
fn host_spawn(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let json = text_at(vm, args, 0);
    let data = number_at(vm, args, 1);
    let exit = number_at(vm, args, 2);
    let spec: serde_json::Value = serde_json::from_str(&json).map_err(|error| failed(vm, &format!("pty options: {error}")))?;
    let text = |name: &str| spec.get(name).and_then(serde_json::Value::as_str).map(str::to_owned);
    let size = |name: &str, or: u64| spec.get(name).and_then(serde_json::Value::as_u64).unwrap_or(or).clamp(1, 2000) as u16;
    let (cols, rows) = (size("cols", 80), size("rows", 24));
    let id = NEXT.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    if !really() {
        PTYS.with(|ptys| ptys.borrow_mut().insert(id, Running::Pretend { data, exit, written: Vec::new(), size: (cols, rows), ended: false }));
        return Ok(Value::number(f64::from(id)));
    }
    let env = spec
        .get("env")
        .and_then(serde_json::Value::as_object)
        .map(|vars| vars.iter().filter_map(|(name, value)| value.as_str().map(|v| (name.clone(), v.to_owned()))).collect())
        .unwrap_or_default();
    let options = PtyOptions { command: text("command").unwrap_or_default(), cwd: text("cwd"), env, cols, rows };
    let waiting = Arc::new(AtomicUsize::new(0));
    let tell = bridge::desktop_events_from_anywhere();
    let tell_exit = bridge::desktop_events_from_anywhere();
    let heard = Arc::downgrade(&waiting);
    let pty = Pty::spawn(
        &options,
        Arc::new(move |text| {
            // A program that prints faster than the app reads is held at the pipe, as it is on any terminal.
            loop {
                // A disposed terminal has no app left to acknowledge its output.
                let Some(waiting) = heard.upgrade() else { return };
                if waiting.load(Ordering::Relaxed) < MOST_WAITING {
                    waiting.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                drop(waiting);
                std::thread::sleep(Duration::from_millis(2));
            }
            tell(data, Some(text));
        }),
        Box::new(move |code| tell_exit(exit, Some(code.to_string()))),
    )
    .map_err(|message| failed(vm, &message))?;
    PTYS.with(|ptys| ptys.borrow_mut().insert(id, Running::Real { pty: Arc::new(pty), waiting }));
    Ok(Value::number(f64::from(id)))
}

/// `ptyWrite(id, text)`: types this.
fn host_write(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    let value = args.get(vm, 1);
    let text = if value.is_string() {
        vm.string_text(value).unwrap()
    } else {
        vm.plain_text(value).unwrap_or_default().into()
    };
    PTYS.with(|ptys| match ptys.borrow_mut().get_mut(&id) {
        Some(Running::Real { pty, .. }) => pty.write(text.as_bytes()),
        Some(Running::Pretend { written, ended: false, .. }) => written.push(text.into_owned()),
        _ => {}
    });
    Ok(Value::UNDEFINED)
}

/// `ptyResize(id, cols, rows)`.
fn host_resize(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let (id, cols, rows) = (number_at(vm, args, 0), number_at(vm, args, 1).clamp(1, 2000) as u16, number_at(vm, args, 2).clamp(1, 2000) as u16);
    PTYS.with(|ptys| match ptys.borrow_mut().get_mut(&id) {
        Some(Running::Real { pty, .. }) => pty.resize(cols, rows),
        Some(Running::Pretend { size, .. }) => *size = (cols, rows),
        None => {}
    });
    Ok(Value::UNDEFINED)
}

/// `ptyAck(id)`: the app has taken one read of what was printed, so the program can print more.
fn host_ack(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    PTYS.with(|ptys| {
        if let Some(Running::Real { waiting, .. }) = ptys.borrow().get(&id) {
            let _ = waiting.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |now| Some(now.saturating_sub(1)));
        }
    });
    Ok(Value::UNDEFINED)
}

/// `ptyKill(id)`: ends the program, and lets go of it when its end has been told.
fn host_kill(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    PTYS.with(|ptys| match ptys.borrow().get(&id) {
        Some(Running::Real { pty, .. }) => pty.kill(),
        Some(Running::Pretend { exit, ended: false, .. }) => bridge::fire_desktop_event(*exit, Some("1".to_owned())),
        _ => {}
    });
    Ok(Value::UNDEFINED)
}

/// `ptyFree(id)`: forgets it, ending the program if it's still there.
fn host_free(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    PTYS.with(|ptys| ptys.borrow_mut().remove(&id));
    Ok(Value::UNDEFINED)
}

/// `ptyPid(id)`: the program's number, or 0.
fn host_pid(vm: &mut Vm, args: Args) -> Result<Value, Value> {
    let id = number_at(vm, args, 0);
    let pid = PTYS.with(|ptys| match ptys.borrow().get(&id) {
        Some(Running::Real { pty, .. }) => pty.pid(),
        _ => 0,
    });
    Ok(Value::number(f64::from(pid)))
}

/// A program's printed text the way a step writes it: `\e`, `\r`, `\n`, `\t`, `\\`, `\xNN` and `\u{N}`.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('e') => out.push('\u{1b}'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('a') => out.push('\u{7}'),
            Some('\\') => out.push('\\'),
            Some('x') => {
                let hex: String = (0..2).filter_map(|_| chars.next_if(char::is_ascii_hexdigit)).collect();
                out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
            Some('u') if chars.peek() == Some(&'{') => {
                chars.next();
                let hex: String = chars.by_ref().take_while(|c| *c != '}').collect();
                out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The steps: `pty out [N] TEXT` has pty N (the last one by default) print, `pty exit [N] [CODE]` ends it, `pty written [N]` says
/// what the app has typed into it, `pty size [N]` its size, and `pty list` which there are.
pub(crate) fn step(words: &[String]) -> Result<(), String> {
    let (verb, rest) = words.split_first().ok_or("pty needs out, exit, written, size or list")?;
    let last = PTYS.with(|ptys| ptys.borrow().keys().next_back().copied());
    // An optional first number says which.
    let (id, rest) = match rest.first().and_then(|word| word.parse::<u32>().ok()) {
        Some(id) if verb != "exit" || rest.len() > 1 => (Some(id), &rest[1..]),
        _ => (last, rest),
    };
    let id = id.filter(|_| verb != "list").ok_or_else(|| "no pty is running".to_owned());
    PTYS.with(|ptys| {
        let mut ptys = ptys.borrow_mut();
        match verb.as_str() {
            "list" => {
                for (id, one) in ptys.iter() {
                    match one {
                        Running::Real { pty, .. } => println!("  pty {id}: real, pid {}", pty.pid()),
                        Running::Pretend { size, written, ended, .. } => {
                            println!("  pty {id}: pretend, {}x{}, {} written{}", size.0, size.1, written.len(), if *ended { ", ended" } else { "" });
                        }
                    }
                }
                Ok(())
            }
            "out" | "exit" | "written" | "size" => {
                let id = id?;
                match ptys.get_mut(&id) {
                    Some(Running::Pretend { data, exit, written, size, ended }) => match verb.as_str() {
                        "out" => {
                            if *ended {
                                return Err(format!("pty {id} has ended"));
                            }
                            bridge::fire_desktop_event(*data, Some(unescape(&rest.join(" "))));
                            Ok(())
                        }
                        "exit" => {
                            let code = rest.first().and_then(|word| word.parse::<u32>().ok()).unwrap_or(0);
                            *ended = true;
                            bridge::fire_desktop_event(*exit, Some(code.to_string()));
                            Ok(())
                        }
                        "written" => {
                            let all = written.join("");
                            println!("  pty {id} was typed {all:?}");
                            if rest.first().is_some_and(|word| word == "clear") {
                                written.clear();
                            }
                            Ok(())
                        }
                        _ => {
                            println!("  pty {id} is {}x{}", size.0, size.1);
                            Ok(())
                        }
                    },
                    Some(Running::Real { pty, .. }) => match verb.as_str() {
                        "out" => Err("a real pty prints what its program does; this is for a pretended one".to_owned()),
                        "exit" => {
                            pty.kill();
                            Ok(())
                        }
                        _ => Err("a real pty keeps what's typed in its program".to_owned()),
                    },
                    None => Err(format!("no pty {id}")),
                }
            }
            other => Err(format!("{other:?} isn't a pty step: out, exit, written, size or list")),
        }
    })
}

/// Every program ends, for a new version of the app's code to ask for them again.
pub(crate) fn let_go_of_everything() {
    PTYS.with(|ptys| ptys.borrow_mut().clear());
}

pub(crate) const FUNCTIONS: [(&str, HostFunction); 7] = [
    ("ptySpawn", host_spawn),
    ("ptyWrite", host_write),
    ("ptyResize", host_resize),
    ("ptyAck", host_ack),
    ("ptyKill", host_kill),
    ("ptyFree", host_free),
    ("ptyPid", host_pid),
];

#[cfg(test)]
mod tests {
    use super::unescape;

    #[test]
    fn a_step_can_write_escapes() {
        assert_eq!(unescape(r"a\e[1mb\r\n\x41\u{1F600}\\"), "a\u{1b}[1mb\r\nA\u{1F600}\\");
        assert_eq!(unescape(r"\q"), r"\q");
    }
}
