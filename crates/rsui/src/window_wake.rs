//! Waking a window when the app next has something to do, on time. `SetTimer` only goes off on the system's clock tick, every
//! 15.6 ms unless something has asked for it to be faster, and later still when Windows gathers timers together to save power,
//! as it does more once a window's been left alone: an animation asking for a frame every 33 ms got one every 47, and fewer once
//! nobody had touched the window for a few seconds. This is one thread for the whole process, waiting on a high resolution
//! timer, which goes off within a millisecond or so, and posting [`WAKE_DUE`] to the window whose time it is.
//!
//! Where there's no such timer (before Windows 10 1803), [`start`] says so and the window uses `SetTimer` as before.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateEventW, CreateWaitableTimerExW, INFINITE, SetEvent, SetWaitableTimer,
    TIMER_ALL_ACCESS, WaitForMultipleObjects,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::PCWSTR;

/// Posted to a window when the time it asked to be woken at has come.
pub(crate) const WAKE_DUE: u32 = WM_APP + 17;

struct Waker {
    /// Each window waiting, by its handle, and when for.
    due: Mutex<Vec<(isize, Instant)>>,
    /// Set when that's changed, for the thread to look again.
    changed: isize,
}

static WAKER: OnceLock<Option<Waker>> = OnceLock::new();

/// The thread that wakes windows, started the first time it's wanted, or `None` if there's no high resolution timer here.
fn waker() -> Option<&'static Waker> {
    WAKER
        .get_or_init(|| {
            // SAFETY: plain object creation; the handles are kept for as long as the process runs.
            let (timer, changed) = unsafe {
                let timer = CreateWaitableTimerExW(None, PCWSTR::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0).ok()?;
                let changed = CreateEventW(None, false, false, PCWSTR::null()).ok()?;
                (timer.0 as isize, changed.0 as isize)
            };
            std::thread::Builder::new().name("rsui-wake".into()).spawn(move || run(timer, changed)).ok()?;
            Some(Waker { due: Mutex::new(Vec::new()), changed })
        })
        .as_ref()
}

/// Whether windows can be woken this way.
pub(crate) fn start() -> bool {
    waker().is_some()
}

/// Has `window` sent [`WAKE_DUE`] at `when`, in place of any time it asked for before, or never if that's `None`.
pub(crate) fn set(window: HWND, when: Option<Instant>) {
    let Some(waker) = waker() else {
        return;
    };
    let key = window.0 as isize;
    {
        let mut due = waker.due.lock().unwrap_or_else(|e| e.into_inner());
        due.retain(|(w, _)| *w != key);
        if let Some(when) = when {
            due.push((key, when));
        }
    }
    // SAFETY: the event is kept for as long as the process runs.
    unsafe {
        let _ = SetEvent(HANDLE(waker.changed as *mut _));
    }
}

fn run(timer: isize, changed: isize) {
    let (timer, changed) = (HANDLE(timer as *mut _), HANDLE(changed as *mut _));
    loop {
        let Some(waker) = WAKER.get().and_then(Option::as_ref) else {
            // Not stored yet: it is a moment after this starts.
            std::thread::yield_now();
            continue;
        };
        // Whoever's time has come, and when the next one's is.
        let now = Instant::now();
        let mut ready = Vec::new();
        let next = {
            let mut due = waker.due.lock().unwrap_or_else(|e| e.into_inner());
            due.retain(|&(window, when)| {
                // Within a quarter of a millisecond is close enough, and saves waiting again for almost nothing.
                if when.saturating_duration_since(now).as_micros() <= 250 {
                    ready.push(window);
                    false
                } else {
                    true
                }
            });
            due.iter().map(|(_, when)| *when).min()
        };
        for window in ready {
            // SAFETY: posting does nothing if the window has gone.
            unsafe {
                let _ = PostMessageW(Some(HWND(window as *mut _)), WAKE_DUE, WPARAM(0), LPARAM(0));
            }
        }
        // SAFETY: the handles are this thread's, kept for as long as the process runs.
        unsafe {
            match next {
                Some(when) => {
                    // Relative, in 100 ns units, as a negative number.
                    let wait = -((when.saturating_duration_since(Instant::now()).as_nanos() / 100).max(1) as i64);
                    if SetWaitableTimer(timer, &wait, 0, None, None, false).is_err() {
                        std::thread::sleep(when.saturating_duration_since(Instant::now()));
                        continue;
                    }
                    let _ = WaitForMultipleObjects(&[changed, timer], false, INFINITE);
                }
                None => {
                    if WaitForMultipleObjects(&[changed], false, INFINITE) != WAIT_OBJECT_0 {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
            }
        }
    }
}
