//! What a [`Session`](crate::Session) operates: an app with no window, or anything else that can do what's here.

use std::time::{Duration, Instant};

use rsui_core::{ActionKind, Node, Patch, PropValue};
use rsui_headless::Headless;
use rsui_runtime::DispatchError;

/// One of the windows of an app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    /// What the tools call it. The main window is 0.
    pub id: u32,
    pub title: String,
    /// The modal window that has to be dealt with before this one can be used, if there is one.
    pub waiting_for: Option<u32>,
}

/// An app for a [`Session`](crate::Session) to operate. Only what's needed for reading it and acting on it has to be
/// here. The rest are for hosts that can do more, which say so with [`does_more`](Self::does_more).
pub trait Host {
    /// The app's windows, with the main one first.
    fn windows(&self) -> Vec<WindowInfo>;

    /// The tree of a window, if it's open.
    fn root(&self, window: u32) -> Option<&Node>;

    /// Applies anything that's waiting, and says what changed in each window.
    fn flush(&mut self) -> Vec<(u32, Patch)>;

    /// Carries out an action. What it changes comes from the next [`flush`](Self::flush).
    fn dispatch(
        &mut self,
        window: u32,
        path: &[u32],
        action: ActionKind,
        input: Option<PropValue>,
    ) -> Result<(), DispatchError>;

    /// Whether anything's going on right now that an action should wait for.
    fn busy(&self) -> bool;

    /// Whether something's due later that could change the tree, such as a timer, which only `ui_wait` waits for.
    fn more_to_come(&self) -> bool {
        false
    }

    /// Waits until there may be something to flush, or this much time has passed.
    fn wait(&self, timeout: Duration);

    /// Keeps flushing until nothing's going on, or the time's up. Says what changed, and whether it all finished.
    fn settle(&mut self, timeout: Duration) -> (Vec<(u32, Patch)>, bool) {
        let deadline = Instant::now() + timeout;
        let mut patches = self.flush();
        while self.busy() {
            let now = Instant::now();
            if now >= deadline {
                return (patches, false);
            }
            self.wait(deadline - now);
            patches.extend(self.flush());
        }
        (patches, true)
    }

    /// Whether this has the tools below, and windows to tell apart.
    fn does_more(&self) -> bool {
        false
    }

    /// Called at the start of every tool call, for keeping up with what's changed outside the app.
    fn begin(&mut self) {}

    /// Why nothing can be done at the moment, if that's so, such as the app having closed.
    fn problem(&self) -> Option<String> {
        None
    }

    /// What the app has logged since this was last asked, a line each.
    fn take_logs(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// A picture of a window, as a PNG file.
    fn picture(&mut self, _window: u32) -> Result<Vec<u8>, String> {
        Err("this app can't be drawn".to_owned())
    }

    /// Presses a key such as `ctrl+s`, with the keyboard on this node if one's given.
    fn press(&mut self, _window: u32, _on: Option<&[u32]>, _key: &str) -> Result<(), String> {
        Err("this app doesn't take keys".to_owned())
    }

    /// Does what a window's close button does.
    fn close(&mut self, _window: u32) -> Result<(), String> {
        Err("this app has no windows to close".to_owned())
    }

    /// Starts the app again from nothing.
    fn restart(&mut self) -> Result<(), String> {
        Err("this app can't be restarted".to_owned())
    }

    /// Says what the next dialog asking for a file or folder gets, with `None` for it being cancelled.
    fn answer_next_dialog(&mut self, _path: Option<String>) -> Result<(), String> {
        Err("this app has no dialogs".to_owned())
    }

    /// Why the component that made this node last rendered, how often it has, and what its state is.
    fn why(&mut self, _window: u32, _path: &[u32]) -> Result<String, String> {
        Err("this app doesn't keep why things render".to_owned())
    }

    /// What's happened in each frame since this was last asked: what came in, what rendered and why, and how long it took.
    fn frames(&mut self) -> Result<String, String> {
        Err("this app doesn't keep its frames".to_owned())
    }

    /// Adds a logpoint, an expression shown each time a line runs, or takes away what's at `at` with no expression. `at`
    /// is a file and a line, as `app.tsx:40`, or `all` to take them all away.
    fn logpoint(&mut self, _at: &str, _expression: Option<&str>) -> Result<String, String> {
        Err("this app doesn't have logpoints".to_owned())
    }

    /// What the frames kept come to: the slowest, which components took the most time, and why renders that changed
    /// nothing happened.
    fn profile(&mut self) -> Result<String, String> {
        Err("this app doesn't keep its frames".to_owned())
    }

    /// Every change of state that's kept, numbered, with what it was before and after.
    fn history(&mut self) -> Result<String, String> {
        Err("this app doesn't keep its changes of state".to_owned())
    }

    /// Puts the app's state back to how it was just after a change in [`history`](Self::history), or forward again.
    fn go_to(&mut self, _change: u32) -> Result<String, String> {
        Err("this app doesn't keep its changes of state".to_owned())
    }
}

impl Host for Headless {
    fn windows(&self) -> Vec<WindowInfo> {
        vec![WindowInfo {
            id: 0,
            title: String::new(),
            waiting_for: None,
        }]
    }

    fn root(&self, window: u32) -> Option<&Node> {
        (window == 0).then(|| Headless::root(self))
    }

    fn flush(&mut self) -> Vec<(u32, Patch)> {
        let patches = self.runtime_mut().flush();
        patches.into_iter().map(|patch| (0, patch)).collect()
    }

    fn dispatch(
        &mut self,
        _window: u32,
        path: &[u32],
        action: ActionKind,
        input: Option<PropValue>,
    ) -> Result<(), DispatchError> {
        self.runtime_mut().dispatch(path, action, input)
    }

    fn busy(&self) -> bool {
        self.runtime().has_background_work()
    }

    fn wait(&self, timeout: Duration) {
        self.runtime().wait(timeout);
    }
}
