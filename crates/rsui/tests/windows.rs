//! Real windows and the real message loop, on a desktop of their own that nobody can see, so that testing them
//! doesn't bring anything up on screen or take the keyboard.
// Real Win32 windows and dialogs, which only Windows has.
#![cfg(windows)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use rsui::core::Symbol;
use rsui::prelude::*;
use rsui::{WindowId, WindowOptions, close_window, open_window, set_window_title};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, DESKTOP_CONTROL_FLAGS, EnumDesktopWindows, HDESK,
    SetThreadDesktop,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GW_OWNER, GetClassNameW, GetWindow, GetWindowTextW, HCBT_ACTIVATE,
    HCBT_DESTROYWND, IsWindowVisible, PostMessageW, SetWindowsHookExW, WH_CBT, WM_CLOSE,
};
use windows::core::BOOL;

/// What's on a desktop, of the kind of window this makes, with what each is called.
fn windows_on(desktop: isize) -> Vec<(HWND, String)> {
    let mut found = Vec::<(HWND, String)>::new();
    unsafe extern "system" fn look(window: HWND, found: LPARAM) -> BOOL {
        let mut class = [0u16; 32];
        let mut title = [0u16; 64];
        // SAFETY: this is called while the list it's given is still there, on this thread, and these are big enough.
        unsafe {
            let length = GetClassNameW(window, &mut class);
            if IsWindowVisible(window).as_bool()
                && String::from_utf16_lossy(&class[..length as usize]) == "rsui-window"
            {
                let length = GetWindowTextW(window, &mut title);
                (*(found.0 as *mut Vec<(HWND, String)>))
                    .push((window, String::from_utf16_lossy(&title[..length as usize])));
            }
        }
        BOOL(1)
    }
    // SAFETY: `found` lasts through the call, and the function has the signature it's meant to.
    unsafe {
        let _ = EnumDesktopWindows(
            Some(HDESK(desktop as *mut _)),
            Some(look),
            LPARAM((&raw mut found) as isize),
        );
    }
    found
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "this never happened: {what}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn find(desktop: isize, title: &str) -> Option<HWND> {
    windows_on(desktop)
        .into_iter()
        .find(|(_, called)| called == title)
        .map(|(window, _)| window)
}

fn send_close(window: HWND) {
    // SAFETY: it does nothing if the window has gone.
    unsafe {
        let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

static LEFT_BEHIND: AtomicU32 = AtomicU32::new(0);
static STUBBORN_ID: AtomicU32 = AtomicU32::new(0);
static ASKED: AtomicU32 = AtomicU32::new(0);
/// Whether the window a modal one belongs to could be used at the moment that one started to go: 1 if not, 2 if so.
static OWNER_AS_IT_WENT: AtomicU32 = AtomicU32::new(0);
/// The windows that have been given the keyboard.
static ACTIVATED: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// Hears about a window just before it's activated or destroyed. Only the modal one here has an owner.
unsafe extern "system" fn just_before(code: i32, what: WPARAM, more: LPARAM) -> LRESULT {
    // SAFETY: this is the window it's being told about, on its own thread, and the rest is passed along as it came.
    unsafe {
        if code == HCBT_ACTIVATE as i32 {
            ACTIVATED.lock().unwrap().push(what.0);
        }
        if code == HCBT_DESTROYWND as i32 {
            if let Ok(owner) = GetWindow(HWND(what.0 as *mut _), GW_OWNER) {
                let usable = IsWindowEnabled(owner).as_bool();
                OWNER_AS_IT_WENT.store(if usable { 2 } else { 1 }, Ordering::SeqCst);
            }
        }
        CallNextHookEx(None, code, what, more)
    }
}

fn other_window(cx: &mut Cx, _: &()) -> Element {
    cx.effect_with_cleanup((), |_| {
        |_: &mut Updates<'_>| {
            LEFT_BEHIND.fetch_add(1, Ordering::SeqCst);
        }
    });
    Element::new(Role::Window)
        .props(Paint::new().background(Color::WHITE))
        .child(Element::new(Role::Text).name("In the other one"))
}

fn stubborn_window(cx: &mut Cx, _: &()) -> Element {
    let window = Element::new(Role::Window)
        .key("stubborn")
        .child(Element::new(Role::Text).name("Staying"));
    cx.on(
        window,
        ActionKind::Custom(Symbol::new("close_request")),
        |_, _| {
            // It gives in the second time.
            if ASKED.fetch_add(1, Ordering::SeqCst) == 1 {
                close_window(WindowId(STUBBORN_ID.load(Ordering::SeqCst)));
            }
        },
    )
}

fn main_window(cx: &mut Cx, _: &()) -> Element {
    cx.effect((), |_| {
        open_window(
            WindowOptions {
                title: "Other".to_owned(),
                size: Some((300, 200)),
                smallest: Some((100, 100)),
                owned: true,
                modal: true,
                ..WindowOptions::default()
            },
            other_window,
            (),
        );
        open_window(
            WindowOptions {
                title: "Second".to_owned(),
                size: Some((200, 100)),
                owned: true,
                modal: true,
                ..WindowOptions::default()
            },
            other_window,
            (),
        );
        let free = open_window(
            WindowOptions {
                title: "Free".to_owned(),
                size: Some((200, 100)),
                owned: false,
                ..WindowOptions::default()
            },
            other_window,
            (),
        );
        set_window_title(free, "Free, renamed");
        let stubborn = open_window(
            WindowOptions {
                title: "Stubborn".to_owned(),
                owned: false,
                ..WindowOptions::default()
            },
            stubborn_window,
            (),
        );
        STUBBORN_ID.store(stubborn.0, Ordering::SeqCst);
    });
    Element::new(Role::Window)
        .props(Paint::new().background(Color::WHITE))
        .child(Element::new(Role::Text).name("The main one"))
}

/// All in one test, as a thread can only be moved onto a desktop before it has windows, and each of these is only a
/// moment of what the same program would go through.
#[test]
fn real_windows_open_belong_to_each_other_and_go_away_as_they_should() {
    // Something stuck on a desktop nobody can see would otherwise stay there, with nobody knowing.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        eprintln!("this has been stuck for a minute");
        std::process::abort();
    });
    let name: Vec<u16> = "rsui-test-windows\0".encode_utf16().collect();
    // SAFETY: the name ends in a zero, and it's closed when this is over.
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
    let running = std::thread::spawn(move || {
        // SAFETY: this thread has no windows yet to stop it moving.
        unsafe { SetThreadDesktop(HDESK(handle as *mut _)) }.expect("it can move there");
        // SAFETY: this is called on the thread it watches, and goes when that does.
        unsafe { SetWindowsHookExW(WH_CBT, Some(just_before), None, GetCurrentThreadId()) }
            .expect("it can listen");
        rsui::run_sized("Main", Some((400, 300)), main_window, ())
    });

    let others = |wanted: usize| windows_on(handle).len() == wanted;
    wait_until("all five windows being open", || others(5));
    let main = find(handle, "Main").expect("the main one is there");
    let other = find(handle, "Other").expect("the other one is there");
    let second = find(handle, "Second").expect("the second modal one is there");
    wait_until("a window being renamed", || {
        find(handle, "Free, renamed").is_some()
    });
    let free = find(handle, "Free, renamed").unwrap();
    let stubborn = find(handle, "Stubborn").expect("the one that stays is there");

    // SAFETY: asking about a window only gives an answer.
    unsafe {
        assert_eq!(GetWindow(other, GW_OWNER).ok(), Some(main), "owned");
        assert!(GetWindow(free, GW_OWNER).is_err(), "not owned");
        assert!(!IsWindowEnabled(main).as_bool(), "something modal is open");
    }

    // Nothing here is in front, as nobody's using this desktop, so opening these can't take the keyboard from whatever is.
    let activated = ACTIVATED.lock().unwrap().clone();
    for (window, called) in [
        (other, "other"),
        (second, "second"),
        (free, "free"),
        (stubborn, "stubborn"),
    ] {
        assert!(
            !activated.contains(&(window.0 as usize)),
            "{called} took the keyboard"
        );
    }

    // What has an answer to being asked stays.
    send_close(stubborn);
    wait_until("it being asked", || ASKED.load(Ordering::SeqCst) == 1);
    assert!(find(handle, "Stubborn").is_some(), "it's still there");
    // Until it says otherwise, from code.
    send_close(stubborn);
    wait_until("it closing itself", || find(handle, "Stubborn").is_none());

    // What hasn't goes, and takes its components with it. Another modal one is still keeping the main one from being used.
    send_close(second);
    wait_until("the second modal one going", || {
        find(handle, "Second").is_none()
    });
    wait_until("what it left behind", || {
        LEFT_BEHIND.load(Ordering::SeqCst) == 1
    });
    // SAFETY: asking about a window only gives an answer.
    assert!(
        !unsafe { IsWindowEnabled(main) }.as_bool(),
        "one modal window is still open"
    );

    send_close(other);
    wait_until("the modal one going", || find(handle, "Other").is_none());
    assert_eq!(
        OWNER_AS_IT_WENT.load(Ordering::SeqCst),
        2,
        "its owner has to be usable before it goes, or the system puts another program in front"
    );
    wait_until("what it left behind", || {
        LEFT_BEHIND.load(Ordering::SeqCst) == 2
    });
    // SAFETY: asking about a window only gives an answer.
    assert!(
        unsafe { IsWindowEnabled(main) }.as_bool(),
        "the main one can be used again"
    );
    assert!(find(handle, "Main").is_some());

    // The main one going ends the program, with the rest going along with it.
    send_close(main);
    let result = {
        let started = Instant::now();
        while !running.is_finished() {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "the program never ended"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        running.join().expect("the program didn't panic")
    };
    assert!(result.is_ok(), "{result:?}");
    assert!(windows_on(handle).is_empty(), "{:?}", windows_on(handle));
    assert_eq!(
        LEFT_BEHIND.load(Ordering::SeqCst),
        3,
        "the free one heard too"
    );
    // SAFETY: nothing's using it any more.
    unsafe {
        let _ = CloseDesktop(desktop);
    }
}

#[test]
fn what_is_asked_of_an_open_window_waits_to_be_done_in_order() {
    let _ = rsui::take_window_requests();
    rsui::change_window(WindowId(3), rsui::WindowChange::OnTop(true));
    rsui::change_window(WindowId::MAIN, rsui::WindowChange::Minimize);
    let requests = rsui::take_window_requests();
    assert!(!requests.is_empty());
    assert_eq!(
        requests.changes,
        [
            (WindowId(3), rsui::WindowChange::OnTop(true)),
            (WindowId::MAIN, rsui::WindowChange::Minimize),
        ]
    );
    assert!(rsui::take_window_requests().is_empty());
}

fn with_a_handle(cx: &mut Cx, _: &()) -> Element {
    let _ = cx;
    let title_bar = Element::new(Role::Group)
        .prop("height", 40.0)
        .prop("drags_window", true)
        .child(
            Element::new(Role::Text)
                .prop("height", 20.0)
                .name("Words in the title bar"),
        );
    let close = cx.on(
        Element::new(Role::Button)
            .key(1u64)
            .prop("width", 30.0)
            .prop("height", 30.0)
            .name("Close"),
        rsui::prelude::ActionKind::Click,
        |_, _| {},
    );
    Element::new(Role::Window)
        .child(title_bar.child(close))
        .child(Element::new(Role::Group).prop("height", 100.0))
}

fn a_widget(cx: &mut Cx, _: &()) -> Element {
    let button = cx.on(
        Element::new(Role::Button)
            .key(1u64)
            .prop("width", 60.0)
            .prop("height", 30.0)
            .name(""),
        rsui::prelude::ActionKind::Click,
        |_, _| {},
    );
    let input = Element::new(Role::TextInput).key(2u64).prop("width", 100.0).prop("height", 24.0);
    // Nothing behind the whole window, as a transparent one has: a card with a color, a button with none, a text box, and
    // nothing under them.
    Element::new(Role::Window)
        .child(
            Element::new(Role::Group)
                .props(Paint::new().background(Color::WHITE))
                .prop("width", 120.0)
                .prop("height", 50.0),
        )
        .child(button)
        .child(input)
        .child(Element::new(Role::Group).prop("height", 100.0))
}

#[test]
fn a_transparent_window_is_over_only_what_it_draws_and_what_takes_clicks() {
    let text = rsui::default_text_engine().unwrap();
    let mut app = rsui::App::new(a_widget, (), text, (400, 300), 1.0);
    app.update();
    app.scene();
    assert!(app.drawn_at(10.0, 10.0), "on the card");
    assert!(!app.drawn_at(300.0, 10.0), "beside it");
    assert!(app.drawn_at(20.0, 60.0), "on the button, which has nothing drawn of its own");
    assert!(!app.drawn_at(200.0, 250.0), "under everything");
    assert!(app.takes_keys_at(20.0, 92.0), "the text box");
    assert!(!app.takes_keys_at(20.0, 60.0), "the button doesn't");
    assert!(!app.takes_keys_at(10.0, 10.0), "nor the card");
}

#[test]
fn a_window_with_no_frame_drags_from_where_it_says_and_nowhere_else() {
    let text = rsui::default_text_engine().unwrap();
    let mut app = rsui::App::new(with_a_handle, (), text, (400, 300), 1.0);
    app.update();
    assert!(app.drags_window_at(300.0, 5.0), "empty title bar");
    assert!(app.drags_window_at(5.0, 5.0), "words in the title bar");
    assert!(
        !app.drags_window_at(5.0, 25.0),
        "the button in the title bar"
    );
    assert!(!app.drags_window_at(200.0, 100.0), "below the title bar");
}
