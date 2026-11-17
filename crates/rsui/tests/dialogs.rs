//! The file dialogs, on a desktop of their own that nobody can see, so that testing them doesn't get in anyone's way.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rsui::dialogs::{DialogKind, DialogOptions, ask_on_this_thread};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, DESKTOP_CONTROL_FLAGS, EnumDesktopWindows, HDESK,
    SetThreadDesktop,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, IDCANCEL, IDOK, IsWindowVisible, PostMessageW, WM_COMMAND,
};
use windows::core::BOOL;

/// Answers the dialog on this desktop, however many tries it takes, until it's gone.
fn answer(desktop: isize, answer: i32, finished: &AtomicBool) {
    let desktop = HDESK(desktop as *mut _);
    let started = Instant::now();
    while !finished.load(Ordering::Acquire) {
        // A test that can't get an answer shouldn't wait for ever, on a desktop nobody can get to.
        assert!(
            started.elapsed() < Duration::from_secs(180),
            "the dialog never went away"
        );
        let mut found = Vec::<HWND>::new();
        unsafe extern "system" fn look(window: HWND, found: LPARAM) -> BOOL {
            let mut class = [0u16; 16];
            // SAFETY: this is called while the list it's given is still there, on this thread, and `class` is big enough.
            unsafe {
                let length = GetClassNameW(window, &mut class);
                if IsWindowVisible(window).as_bool()
                    && String::from_utf16_lossy(&class[..length as usize]) == "#32770"
                {
                    (*(found.0 as *mut Vec<HWND>)).push(window);
                }
            }
            BOOL(1)
        }
        // SAFETY: `found` lasts through the call, and the function has the signature it's meant to.
        unsafe {
            let _ =
                EnumDesktopWindows(Some(desktop), Some(look), LPARAM((&raw mut found) as isize));
        }
        std::thread::sleep(Duration::from_millis(400));
        for window in found {
            // SAFETY: it does nothing if the window has gone.
            unsafe {
                let _ = PostMessageW(Some(window), WM_COMMAND, WPARAM(answer as usize), LPARAM(0));
            }
        }
    }
}

/// One dialog at a time: two at once in one program, each on a desktop of its own, made the shell slow enough to fail
/// on a busy machine.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn on_hidden_desktop(
    name: &str,
    answer_with: i32,
    kind: DialogKind,
    options: DialogOptions,
) -> Result<Option<String>, String> {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // A desktop's name is shared by every program, so another run of these tests at the same time has its own: on the
    // same one, each would answer the other's dialog too.
    let name = format!("{name}-{}", std::process::id());
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: the name ends in a zero, this thread has no windows yet to stop it moving, and the desktop is closed after it
    // moves off it, when it ends.
    let desktop = unsafe {
        CreateDesktopW(
            windows::core::PCWSTR(name.as_ptr()),
            None,
            None,
            DESKTOP_CONTROL_FLAGS(0),
            0x10_0000 | 0xFF,
            None,
        )
    }
    .expect("a desktop can be made");
    let handle = desktop.0 as isize;
    let finished = Arc::new(AtomicBool::new(false));
    let answering = std::thread::spawn({
        let finished = finished.clone();
        move || answer(handle, answer_with, &finished)
    });
    // SAFETY: see above.
    unsafe { SetThreadDesktop(desktop) }.expect("this thread has no windows yet");
    let result = ask_on_this_thread(kind, &options, HWND::default());
    finished.store(true, Ordering::Release);
    answering.join().unwrap();
    // SAFETY: nothing else uses this now that the dialog's gone.
    let _ = unsafe { CloseDesktop(desktop) };
    result
}

fn run_on_new_thread<T: Send + 'static>(run: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(run)
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn cancelling_says_nothing_was_chosen() {
    let result = run_on_new_thread(|| {
        on_hidden_desktop(
            "rsui-test-cancel",
            IDCANCEL.0,
            DialogKind::OpenFile,
            DialogOptions::default(),
        )
    });
    assert_eq!(result, Ok(None));
}

#[test]
fn accepting_what_it_starts_on_says_where_that_is() {
    let file = std::env::temp_dir().join(format!("rsui-dialog-test-{}.txt", std::process::id()));
    std::fs::write(&file, "hello").unwrap();
    let start = file.to_string_lossy().into_owned();
    let result = run_on_new_thread(move || {
        on_hidden_desktop(
            "rsui-test-accept",
            IDOK.0,
            DialogKind::OpenFile,
            DialogOptions {
                title: Some("Choose".to_owned()),
                start: Some(start),
                filters: vec![("Text".to_owned(), "*.txt".to_owned())],
            },
        )
    });
    let shown = result.unwrap().expect("something was chosen");
    let _ = std::fs::remove_file(&file);
    assert_eq!(
        std::fs::canonicalize(std::path::Path::new(&shown).parent().unwrap()).unwrap(),
        std::fs::canonicalize(file.parent().unwrap()).unwrap()
    );
    assert_eq!(std::path::Path::new(&shown).file_name(), file.file_name());
}
