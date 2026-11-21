//! The system's popups, on a desktop of their own that nobody can see, so that testing them doesn't get in anyone's way:
//! each is answered by pressing its buttons from another thread, as someone would.
// Real Win32 windows and dialogs, which only Windows has.
#![cfg(windows)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rsui::popups::{Answer, Day, FIRST_OPTION, Popup, TimeOfDay, ask_on_this_thread, toast_on_this_thread};
use windows::Win32::Foundation::{HWND, LPARAM, SYSTEMTIME, WPARAM};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, DESKTOP_CONTROL_FLAGS, EnumDesktopWindows, HDESK, SetThreadDesktop,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetClassNameW, IDCANCEL, IDOK, IsWindowVisible, PostMessageW, WM_COMMAND,
};
use windows::core::{BOOL, w};

/// What a task dialog does when one of its buttons is pressed.
const TDM_CLICK_BUTTON: u32 = 0x0400 + 102;
const MCM_SETCURSEL: u32 = 0x1000 + 2;

/// Each window of this class that's showing on the desktop.
fn showing(desktop: HDESK, class: &str) -> Vec<HWND> {
    struct Looking<'a> {
        class: &'a str,
        found: Vec<HWND>,
    }
    unsafe extern "system" fn look(window: HWND, looking: LPARAM) -> BOOL {
        let mut class = [0u16; 32];
        // SAFETY: this is called while what it's given is still there, on this thread, and `class` is big enough.
        unsafe {
            let looking = &mut *(looking.0 as *mut Looking<'_>);
            let length = GetClassNameW(window, &mut class);
            if IsWindowVisible(window).as_bool() && String::from_utf16_lossy(&class[..length as usize]) == looking.class {
                looking.found.push(window);
            }
        }
        BOOL(1)
    }
    let mut looking = Looking { class, found: Vec::new() };
    // SAFETY: `looking` lasts through the call, and the function has the signature it's meant to.
    unsafe {
        let _ = EnumDesktopWindows(Some(desktop), Some(look), LPARAM((&raw mut looking) as isize));
    }
    looking.found
}

/// One at a time: two at once in one program, each on a desktop of its own, made the shell slow enough to fail on a busy
/// machine.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Runs `run` on a new thread on a desktop of its own, while another thread does `press` to each dialog that's showing
/// there, every so often, until it's done. It's told about each window of `class` that shows.
fn on_hidden_desktop<T: Send + 'static>(
    name: &str,
    class: &'static str,
    press: impl Fn(HWND) + Send + 'static,
    run: impl FnOnce() -> T + Send + 'static,
) -> (T, usize) {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // A desktop's name is shared by every program, so another run of these tests at the same time has its own.
    let name = format!("{name}-{}", std::process::id());
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: the name ends in a zero, and the desktop is closed once nothing's on it.
    let desktop = unsafe {
        CreateDesktopW(windows::core::PCWSTR(name.as_ptr()), None, None, DESKTOP_CONTROL_FLAGS(0), 0x10_0000 | 0xFF, None)
    }
    .expect("a desktop can be made");
    let handle = desktop.0 as isize;
    let finished = Arc::new(AtomicBool::new(false));
    let answering = std::thread::spawn({
        let finished = finished.clone();
        move || {
            let desktop = HDESK(handle as *mut _);
            let started = Instant::now();
            let mut seen = 0;
            while !finished.load(Ordering::Acquire) {
                // A test that can't get an answer shouldn't wait for ever, on a desktop nobody can get to.
                assert!(started.elapsed() < Duration::from_secs(120), "the popup never went away");
                std::thread::sleep(Duration::from_millis(300));
                let found = showing(desktop, class);
                seen = seen.max(found.len());
                for window in found {
                    press(window);
                }
            }
            seen
        }
    });
    let result = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            // SAFETY: this thread has no windows yet, so it can move.
            unsafe { SetThreadDesktop(HDESK(handle as *mut _)) }.expect("this thread has no windows yet");
            run()
        })
        .unwrap()
        .join()
        .unwrap();
    finished.store(true, Ordering::Release);
    let seen = answering.join().unwrap();
    // SAFETY: nothing else uses it now that what was on it is gone.
    let _ = unsafe { CloseDesktop(desktop) };
    (result, seen)
}

fn ask(name: &str, popup: Popup, press: impl Fn(HWND) + Send + 'static) -> Result<Answer, String> {
    on_hidden_desktop(name, "#32770", press, move || ask_on_this_thread(&popup, HWND::default())).0
}

fn click(id: i32) -> impl Fn(HWND) + Send + 'static {
    move |window| {
        // SAFETY: it does nothing if the window's gone.
        unsafe {
            let _ = PostMessageW(Some(window), TDM_CLICK_BUTTON, WPARAM(id as usize), LPARAM(0));
        }
    }
}

fn command(id: i32) -> impl Fn(HWND) + Send + 'static {
    move |window| {
        // SAFETY: it does nothing if the window's gone.
        unsafe {
            let _ = PostMessageW(Some(window), WM_COMMAND, WPARAM(id as usize), LPARAM(0));
        }
    }
}

fn confirm(labels: bool, destructive: bool) -> Popup {
    Popup::Confirm {
        title: "Delete this list?".into(),
        message: "It can't be brought back.".into(),
        confirm: labels.then(|| "Delete".into()),
        cancel: labels.then(|| "Keep it".into()),
        destructive,
    }
}

#[test]
fn a_confirmation_says_whether_to_go_ahead() {
    assert_eq!(ask("rsui-popup-confirm-yes", confirm(true, true), click(IDOK.0)), Ok(Answer::Confirmed(true)));
    assert_eq!(ask("rsui-popup-confirm-no", confirm(true, false), click(IDCANCEL.0)), Ok(Answer::Confirmed(false)));
}

#[test]
fn a_confirmation_with_the_systems_own_buttons_says_so_too() {
    assert_eq!(ask("rsui-popup-system-yes", confirm(false, false), click(IDOK.0)), Ok(Answer::Confirmed(true)));
    assert_eq!(ask("rsui-popup-system-no", confirm(false, true), click(IDCANCEL.0)), Ok(Answer::Confirmed(false)));
}

#[test]
fn a_message_is_done_with_once_its_seen() {
    let alert = |ok: Option<&str>| Popup::Alert { title: "Saved".into(), message: String::new(), ok: ok.map(Into::into) };
    assert_eq!(ask("rsui-popup-alert", alert(Some("Got it")), click(IDOK.0)), Ok(Answer::Done));
    // Closing it is seeing it too.
    assert_eq!(ask("rsui-popup-alert-closed", alert(None), click(IDCANCEL.0)), Ok(Answer::Done));
}

#[test]
fn a_sheet_says_which_was_chosen_or_that_none_was() {
    let sheet = || Popup::Sheet {
        title: "Sort by".into(),
        options: vec!["Name".into(), "Date".into(), "Size".into()],
        cancel: None,
    };
    assert_eq!(ask("rsui-popup-sheet", sheet(), click(FIRST_OPTION + 1)), Ok(Answer::Chosen(Some(1))));
    assert_eq!(ask("rsui-popup-sheet-none", sheet(), click(IDCANCEL.0)), Ok(Answer::Chosen(None)));
    let empty = Popup::Sheet { title: String::new(), options: Vec::new(), cancel: None };
    assert!(ask_on_this_thread(&empty, HWND::default()).is_err());
}

#[test]
fn a_day_picked_is_the_one_it_started_on_or_another() {
    let start = Day { year: 2026, month: 11, day: 21 };
    let picking = || Popup::PickDate {
        value: Some(start),
        min: Some(Day { year: 2026, month: 1, day: 1 }),
        max: Some(Day { year: 2026, month: 12, day: 31 }),
    };
    assert_eq!(ask("rsui-popup-day", picking(), command(IDOK.0)), Ok(Answer::Day(Some(start))));
    assert_eq!(ask("rsui-popup-day-none", picking(), command(IDCANCEL.0)), Ok(Answer::Day(None)));
    // Someone picks the 3rd of December.
    let another = |window: HWND| {
        static DAY: SYSTEMTIME = SYSTEMTIME {
            wYear: 2026,
            wMonth: 12,
            wDayOfWeek: 0,
            wDay: 3,
            wHour: 0,
            wMinute: 0,
            wSecond: 0,
            wMilliseconds: 0,
        };
        // SAFETY: the day lasts for good, and it does nothing if the window's gone.
        unsafe {
            if let Ok(calendar) = FindWindowExW(Some(window), None, w!("SysMonthCal32"), None) {
                let _ = PostMessageW(Some(calendar), MCM_SETCURSEL, WPARAM(0), LPARAM((&raw const DAY) as isize));
            }
            let _ = PostMessageW(Some(window), WM_COMMAND, WPARAM(IDOK.0 as usize), LPARAM(0));
        }
    };
    assert_eq!(ask("rsui-popup-day-another", picking(), another), Ok(Answer::Day(Some(Day { year: 2026, month: 12, day: 3 }))));
}

#[test]
fn a_time_picked_is_the_one_it_started_on() {
    let start = TimeOfDay { hours: 14, minutes: 35 };
    let picking = || Popup::PickTime { value: Some(start) };
    assert_eq!(ask("rsui-popup-time", picking(), command(IDOK.0)), Ok(Answer::Time(Some(start))));
    assert_eq!(ask("rsui-popup-time-none", picking(), command(IDCANCEL.0)), Ok(Answer::Time(None)));
}

#[test]
fn a_toast_shows_and_goes_away_by_itself() {
    let started = Instant::now();
    let (shown, seen) = on_hidden_desktop("rsui-popup-toast", "rsui toast", |_| {}, || {
        toast_on_this_thread("Saved", Duration::from_millis(900), HWND::default())
    });
    assert_eq!(shown, Ok(()));
    assert_eq!(seen, 1, "the toast showed");
    assert!(started.elapsed() >= Duration::from_millis(900), "it was there for as long as it was meant to be");
}
