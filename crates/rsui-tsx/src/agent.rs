//! Serving an app to an agent over the Model Context Protocol: `rsui-tsx --agent app.tsx`. There's no window, and nothing
//! comes up on screen. Saving a file changes what's running by the next tool call, keeping its state.

use std::cell::RefCell;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use rsui::WindowId;
use rsui::access::AccessCommand;
use rsui::replay::Input;
use rsui::core::{ActionKind, Node, Patch, PropValue};
use rsui::runtime::DispatchError;
use rsui_agent::{Host, Session, WindowInfo};

use crate::bridge::{self, Logs};
use crate::drive::Desk;

/// How long starting up, or getting ready for a picture, waits for what's going on.
const SETTLES_FOR: Duration = Duration::from_secs(1);

/// An app being served, or the reason that it couldn't start.
struct AppHost {
    desk: Option<Desk>,
    path: PathBuf,
    stack_limit: usize,
    /// The app's files and when each was last changed, to see when they do.
    files: Vec<(PathBuf, Option<SystemTime>)>,
    could_not_start: Option<String>,
    logs: Logs,
}

fn changed_at(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|file| file.modified())
        .ok()
}

impl AppHost {
    fn new(path: &Path, stack_limit: usize) -> Self {
        let logs = Logs::default();
        bridge::send_logs_to(logs.clone());
        // A save keeps what's in the app, as it does with a window.
        bridge::keep_state_across_reloads();
        let mut host = Self {
            desk: None,
            path: path.to_owned(),
            stack_limit,
            files: Vec::new(),
            could_not_start: None,
            logs,
        };
        host.start();
        host
    }

    /// Runs the app from nothing, in place of whatever's running.
    fn start(&mut self) {
        // It's all to be let go of first, as what it holds goes back to the engine it came from.
        self.desk = None;
        bridge::forget_what_ran();
        match Desk::open(&self.path, self.stack_limit, (960, 640)) {
            Ok(mut desk) => {
                if let Err(reason) = desk.settle(SETTLES_FOR) {
                    bridge::say(reason);
                }
                // What it opened with is just there, rather than a change.
                desk.take_changes();
                self.desk = Some(desk);
                self.could_not_start = None;
                // Starting up touches a lot that won't be touched again, and most of the time this is only waiting.
                rsui::render::give_back_memory();
            }
            Err(reason) => self.could_not_start = Some(reason),
        }
        self.note_files();
    }

    fn note_files(&mut self) {
        let mut files = bridge::loaded_files();
        if !files.contains(&self.path) {
            files.push(self.path.clone());
        }
        self.files = files
            .into_iter()
            .map(|file| {
                let changed = changed_at(&file);
                (file, changed)
            })
            .collect();
    }

    fn open(&self, window: u32) -> Option<&crate::drive::Open> {
        let desk = self.desk.as_ref()?;
        desk.windows.iter().find(|one| one.id == window)
    }

    /// The app, with the steps it takes about to be for this window.
    fn desk_on(&mut self, window: u32) -> Result<&mut Desk, String> {
        let desk = self.desk.as_mut().ok_or("the app isn't running")?;
        desk.current = window;
        Ok(desk)
    }
}

impl Host for AppHost {
    fn windows(&self) -> Vec<WindowInfo> {
        let Some(desk) = &self.desk else {
            return Vec::new();
        };
        let modal = desk.modal_window();
        desk.windows
            .iter()
            .map(|one| WindowInfo {
                id: one.id,
                title: one.title.clone(),
                waiting_for: modal.filter(|_| one.id == WindowId::MAIN.0),
            })
            .collect()
    }

    fn root(&self, window: u32) -> Option<&Node> {
        Some(self.open(window)?.app.runtime().root())
    }

    fn flush(&mut self) -> Vec<(u32, Patch)> {
        let Some(desk) = &mut self.desk else {
            return Vec::new();
        };
        // What one window does can change another, so it goes on until that's over, if that's soon.
        for _ in 0..16 {
            match desk.pass() {
                Ok(true) => {}
                Ok(false) => break,
                Err(reason) => {
                    bridge::say(reason);
                    break;
                }
            }
        }
        desk.take_changes()
    }

    fn dispatch(
        &mut self,
        window: u32,
        path: &[u32],
        action: ActionKind,
        input: Option<PropValue>,
    ) -> Result<(), DispatchError> {
        let desk = self.desk.as_mut().ok_or(DispatchError::NoSuchNode)?;
        let one = desk
            .windows
            .iter_mut()
            .find(|one| one.id == window)
            .ok_or(DispatchError::NoSuchNode)?;
        one.app.runtime_mut().dispatch(path, action, input)
    }

    fn busy(&self) -> bool {
        self.desk.as_ref().is_some_and(Desk::busy)
    }

    fn more_to_come(&self) -> bool {
        self.desk.is_some() && bridge::timer_is_waiting()
    }

    fn wait(&self, timeout: Duration) {
        std::thread::sleep(timeout.min(Duration::from_millis(2)));
    }

    fn does_more(&self) -> bool {
        true
    }

    fn why(&mut self, window: u32, path: &[u32]) -> Result<String, String> {
        self.desk_on(window)?.why(path)
    }

    fn frames(&mut self) -> Result<String, String> {
        self.desk.as_mut().ok_or("the app isn't running")?.frames()
    }

    fn logpoint(&mut self, at: &str, expression: Option<&str>) -> Result<String, String> {
        let said = match expression {
            Some(expression) => {
                let place = crate::logpoint::add(at, expression)?;
                format!("{place} says {expression} each time it runs, in what tools give back")
            }
            None => {
                let removed = crate::logpoint::remove(at)?;
                format!("{removed} taken away")
            }
        };
        Ok(format!("{said}\n{}", crate::logpoint::list()))
    }

    fn profile(&mut self) -> Result<String, String> {
        self.desk.as_mut().ok_or("the app isn't running")?.profile()
    }

    fn history(&mut self) -> Result<String, String> {
        self.desk.as_mut().ok_or("the app isn't running")?.history()
    }

    fn go_to(&mut self, change: u32) -> Result<String, String> {
        self.desk
            .as_mut()
            .ok_or("the app isn't running")?
            .go_to(change)
    }

    fn begin(&mut self) {
        let changed = self
            .files
            .iter()
            .any(|(file, when)| changed_at(file) != *when);
        if !changed {
            return;
        }
        if self.desk.is_none() {
            // There's nothing to bring forward. It only starts from here if it never could, and not once it's been closed.
            if self.could_not_start.is_some() {
                self.start();
            }
            return;
        }
        bridge::say("a file changed, so what's running has been loaded again".to_owned());
        bridge::request_reload(&self.path);
        self.flush_without_losing_changes();
        self.note_files();
    }

    fn problem(&self) -> Option<String> {
        if let Some(reason) = &self.could_not_start {
            return Some(format!(
                "the app couldn't start: {reason}\nSaving a fix is enough for it to try again."
            ));
        }
        match &self.desk {
            Some(desk) if !desk.windows.is_empty() => None,
            _ => Some("the app has closed. ui_restart starts it again.".to_owned()),
        }
    }

    fn take_logs(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.logs.borrow_mut())
    }

    fn picture(&mut self, window: u32) -> Result<Vec<u8>, String> {
        let desk = self.desk_on(window)?;
        desk.settle(SETTLES_FOR)?;
        let one = desk.here_mut()?;
        let pixels = one.draw()?;
        let (width, height) = one.size;
        let file = crate::png::encode_drawn(width as usize, height as usize, &pixels);
        // Starting the software GPU touches a lot, and what was made for one picture is soon used again if it's needed.
        rsui::render::give_back_memory();
        Ok(file)
    }

    fn press(&mut self, window: u32, on: Option<&[u32]>, key: &str) -> Result<(), String> {
        let desk = self.desk_on(window)?;
        if let Some(path) = on {
            let one = desk.usable()?;
            one.app
                .feed(&Input::Access { path: path.to_vec(), command: AccessCommand::Focus })
                .map_err(|error| error.to_string())?;
            desk.pass()?;
        }
        desk.press(key)
    }

    fn close(&mut self, window: u32) -> Result<(), String> {
        self.desk_on(window)?.close_here()
    }

    fn restart(&mut self) -> Result<(), String> {
        self.start();
        match &self.could_not_start {
            Some(reason) => Err(format!("the app couldn't start: {reason}")),
            None => Ok(()),
        }
    }

    fn answer_next_dialog(&mut self, path: Option<String>) -> Result<(), String> {
        bridge::answer_next_dialog(path);
        Ok(())
    }
}

impl AppHost {
    /// Lets a new version of the code go in. What it changes is still there to be found, for the tool that's being called.
    fn flush_without_losing_changes(&mut self) {
        if let Some(desk) = &mut self.desk {
            if let Err(reason) = desk.pass() {
                bridge::say(reason);
            }
        }
    }
}

const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const STD_ERROR_HANDLE: u32 = -12i32 as u32;

#[allow(unsafe_code)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetStdHandle(which: u32) -> *mut std::ffi::c_void;
    fn SetStdHandle(which: u32, handle: *mut std::ffi::c_void) -> i32;
}

/// Takes the standard output for the protocol alone, and sends anything else that's written there to the standard error.
/// A single stray line would break the connection, and an app can write there in all sorts of ways.
#[allow(unsafe_code)]
fn take_standard_output() -> File {
    use std::os::windows::io::FromRawHandle;
    // SAFETY: these only read and change which handles this process's standard streams are, and from here on nothing else
    // here uses the one that's taken, which this now owns.
    unsafe {
        let output = GetStdHandle(STD_OUTPUT_HANDLE);
        SetStdHandle(STD_OUTPUT_HANDLE, GetStdHandle(STD_ERROR_HANDLE));
        File::from_raw_handle(output)
    }
}

/// Runs an app on this thread, and answers a client on the standard streams until it closes them.
pub(crate) fn serve(path: &Path, stack_limit: usize) -> std::io::Result<()> {
    let output = take_standard_output();
    let host = AppHost::new(path, stack_limit);
    let session = Session::new(host, &crate::file_title(path));
    session.serve(std::io::stdin().lock(), output)
}

/// What an app has logged, a line each, for as long as nobody's come for it.
pub(crate) fn keep_log(logs: &Rc<RefCell<Vec<String>>>, text: &str) {
    /// An app that logs all the time and is never asked about would otherwise fill memory.
    const MOST_LINES: usize = 1_000;
    let mut logs = logs.borrow_mut();
    for line in text.lines() {
        logs.push(line.to_owned());
    }
    if logs.len() > MOST_LINES {
        let extra = logs.len() - MOST_LINES;
        logs.drain(..extra);
    }
}
