//! The popups on Android, which rsui's activity shows: the app's library (rsui-android) says how to reach it with
//! [`set_activity`], and tells [`answered`] what each was answered with, from Android's main thread.

use std::collections::HashMap;
use std::sync::Mutex;

use super::{Answer, Answered, Popup};

/// How the activity is reached: `show(id, kind, json)` brings up a popup, which is answered with `id`, and
/// `toast(text, long)` shows a toast. Both come back straight away.
#[derive(Clone, Copy)]
pub struct Activity {
    pub show: fn(u32, &str, &str),
    pub toast: fn(&str, bool),
}

struct Waiting {
    activity: Option<Activity>,
    next: u32,
    asked: HashMap<u32, (Popup, Answered)>,
}

static WAITING: Mutex<Waiting> = Mutex::new(Waiting { activity: None, next: 1, asked: HashMap::new() });

fn waiting() -> std::sync::MutexGuard<'static, Waiting> {
    WAITING.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Says how to reach the activity. The app's library calls this when the activity's made.
pub fn set_activity(activity: Activity) {
    waiting().activity = Some(activity);
}

pub(super) fn ask(popup: Popup, answered: Answered) {
    let (kind, json) = super::to_json(&popup);
    let (activity, id) = {
        let mut waiting = waiting();
        let Some(activity) = waiting.activity else {
            drop(waiting);
            answered(Err("there's no activity to show a popup in".to_owned()));
            return;
        };
        let id = waiting.next;
        waiting.next = waiting.next.wrapping_add(1).max(1);
        waiting.asked.insert(id, (popup, answered));
        (activity, id)
    };
    (activity.show)(id, kind, &json);
}

pub(super) fn toast(text: &str, long: bool) -> Result<(), String> {
    let activity = waiting().activity.ok_or("there's no activity to show a toast in")?;
    (activity.toast)(text, long);
    Ok(())
}

/// What the popup `id` was answered with, as the activity says it: `true` or `false`, a place in a sheet's list, a day as
/// `2026-11-21` or a time as `14:05`, nothing for none, or `!` and why it couldn't be shown.
pub fn answered(id: u32, said: Option<String>) {
    let Some((popup, answered)) = waiting().asked.remove(&id) else { return };
    let answer: Result<Answer, String> = match said.as_deref() {
        Some(failed) if failed.starts_with('!') => Err(failed[1..].to_owned()),
        said => super::answer_from(&popup, said),
    };
    answered(answer);
}
