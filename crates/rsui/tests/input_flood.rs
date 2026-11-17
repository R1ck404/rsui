//! A native Windows input backlog must not postpone every presentation until the pointer is released.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use rsui::prelude::*;
use rsui::core::Symbol;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, CreateDesktopW, DESKTOP_CONTROL_FLAGS, EnumDesktopWindows, HDESK, SetThreadDesktop,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CWPRETSTRUCT, CallNextHookEx, GetClassNameW, IsWindowVisible, PostMessageW, SetWindowsHookExW,
    MSG, WH_CALLWNDPROCRET, WH_GETMESSAGE, WM_APP, WM_CLOSE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
};
use windows::core::{BOOL, PCWSTR};

static HELD: AtomicBool = AtomicBool::new(false);
static DRAINED: AtomicBool = AtomicBool::new(false);
static SHOWN: AtomicU32 = AtomicU32::new(0);
static PAINTED_WHILE_HELD: Mutex<Vec<u32>> = Mutex::new(Vec::new());
const BARRIER: u32 = WM_APP + 424;

fn root(cx: &mut Cx, _: &()) -> Element {
    let position = cx.state(|| 0u32);
    SHOWN.store(cx.get(position), Ordering::SeqCst);
    let track = Element::new(Role::Group)
        .key("track")
        .props(Style::new().width(350.0).height(150.0))
        .child(Element::new(Role::Text).name(text!("Position {}", cx.get(position))));
    let track = cx.on(track, ActionKind::Custom(Symbol::new("pointer_down")), |_, _| {
        HELD.store(true, Ordering::SeqCst);
    });
    let track = cx.on(track, ActionKind::Custom(Symbol::new("pointer_move")), move |cx, _| {
        if let Some(event) = cx.event() {
            if event.buttons != 0 {
                let x = event.x as u32;
                cx.set(position, x);
            }
        }
    });
    let track = cx.on(track, ActionKind::Custom(Symbol::new("pointer_up")), |_, _| {
        HELD.store(false, Ordering::SeqCst);
    });
    Element::new(Role::Window).child(track)
}

unsafe extern "system" fn completed(code: i32, what: WPARAM, more: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: WH_CALLWNDPROCRET gives a CWPRETSTRUCT valid throughout this callback.
        let message = unsafe { &*(more.0 as *const CWPRETSTRUCT) };
        if message.message == WM_PAINT && HELD.load(Ordering::SeqCst) {
            PAINTED_WHILE_HELD.lock().unwrap().push(SHOWN.load(Ordering::SeqCst));
        }
    }
    // SAFETY: the original hook arguments are passed on unchanged.
    unsafe { CallNextHookEx(None, code, what, more) }
}

unsafe extern "system" fn dequeued(code: i32, what: WPARAM, more: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: WH_GETMESSAGE gives a MSG valid throughout this callback.
        let message = unsafe { &*(more.0 as *const MSG) };
        if message.message == BARRIER {
            DRAINED.store(true, Ordering::SeqCst);
        } else if message.message == WM_MOUSEMOVE && HELD.load(Ordering::SeqCst) {
            // Simulate a nontrivial event consumer. The producer posts its entire uninterrupted input stream without any
            // sleeps; this keeps a real native queue backlog present long enough to require multiple display deadlines.
            std::thread::sleep(Duration::from_micros(500));
        }
    }
    // SAFETY: the original hook arguments are passed on unchanged.
    unsafe { CallNextHookEx(None, code, what, more) }
}

fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(start.elapsed() < Duration::from_secs(20), "timed out: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn sustained_native_moves_present_intermediate_states_before_pointer_release() {
    let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("native-input-flood-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let log = folder.join("frames.log");
    // SAFETY: this test binary has one test, and the application threads that read this setting are started afterwards.
    unsafe { std::env::set_var("RSUI_INPUT_LOG", &log) };
    let name: Vec<u16> = format!("rsui-input-flood-{}\0", std::process::id()).encode_utf16().collect();
    // SAFETY: the name is terminated, and the desktop remains alive until its application thread has exited.
    let desktop = unsafe {
        CreateDesktopW(PCWSTR(name.as_ptr()), None, None, DESKTOP_CONTROL_FLAGS(0), 0x10_0000 | 0xFF, None)
    }.expect("a hidden desktop can be created");
    let desktop_handle = desktop.0 as isize;
    let running = std::thread::spawn(move || {
        // SAFETY: this new thread has no windows and keeps the hidden desktop throughout its native message loop.
        unsafe { SetThreadDesktop(HDESK(desktop_handle as *mut _)) }.expect("the thread can enter its desktop");
        // SAFETY: the hook is restricted to this thread, and removed by Windows when the thread exits.
        unsafe { SetWindowsHookExW(WH_CALLWNDPROCRET, Some(completed), None, GetCurrentThreadId()) }
            .expect("native message completion can be observed");
        // SAFETY: this hook likewise belongs only to the application thread and its message loop.
        unsafe { SetWindowsHookExW(WH_GETMESSAGE, Some(dequeued), None, GetCurrentThreadId()) }
            .expect("the queued input backlog can be observed");
        rsui::run_sized("Input flood", Some((400, 200)), root, ())
    });
    let mut window = None;
    wait_until("native window creation", || {
        unsafe extern "system" fn find(window: HWND, found: LPARAM) -> BOOL {
            let mut name = [0u16; 32];
            // SAFETY: the name buffer and destination remain valid during enumeration.
            unsafe {
                let length = GetClassNameW(window, &mut name);
                if IsWindowVisible(window).as_bool() && String::from_utf16_lossy(&name[..length as usize]) == "rsui-window" {
                    *(found.0 as *mut Option<HWND>) = Some(window);
                }
            }
            BOOL(1)
        }
        // SAFETY: window is writable and the callback lives throughout enumeration.
        let _ = unsafe { EnumDesktopWindows(Some(desktop), Some(find), LPARAM((&raw mut window) as isize)) };
        window.is_some()
    });
    let window = window.expect("the native window was found");
    wait_until("the first actual rendered frame", || {
        std::fs::read_to_string(&log).is_ok_and(|text| text.lines().any(|line| line.starts_with("frame ")))
    });
    // SAFETY: the coordinates and button bits are ordinary client input. Posting never waits for the application.
    unsafe {
        PostMessageW(Some(window), WM_LBUTTONDOWN, WPARAM(1), LPARAM((50 << 16) | 20)).unwrap();
        for step in 0..1000 {
            let x = 20 + step % 300;
            PostMessageW(Some(window), WM_MOUSEMOVE, WPARAM(1), LPARAM((50 << 16) | x)).unwrap();
        }
        PostMessageW(Some(window), BARRIER, WPARAM(0), LPARAM(0)).unwrap();
        PostMessageW(Some(window), WM_LBUTTONUP, WPARAM(0), LPARAM((50 << 16) | 119)).unwrap();
    }
    wait_until("the uninterrupted input stream being consumed", || DRAINED.load(Ordering::SeqCst));
    wait_until("the pointer release being consumed", || !HELD.load(Ordering::SeqCst));
    // Clean up before asserting so a failing regression leaves no native window or message thread behind.
    unsafe { PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)) }.unwrap();
    running.join().unwrap().unwrap();
    // SAFETY: the application thread has exited and owns no windows on this desktop now.
    unsafe { CloseDesktop(desktop) }.unwrap();
    let frames = PAINTED_WHILE_HELD.lock().unwrap().clone();
    let mut in_drag = false;
    let mut actual_frames = 0;
    for line in std::fs::read_to_string(&log).unwrap().lines() {
        if line.starts_with("down ") { in_drag = true; }
        if line.starts_with("up ") { in_drag = false; }
        if in_drag && line.starts_with("frame ") { actual_frames += 1; }
    }
    eprintln!("native paints while held: {}; actual renderer frames before release: {actual_frames}", frames.len());
    assert!(frames.len() >= 3, "input starved paint until release: {frames:?}");
    assert!(actual_frames >= 3, "native paint never presented the renderer's intermediate frames");
    assert!(frames.windows(2).any(|pair| pair[0] != pair[1]), "no intermediate visual state was presented: {frames:?}");
}
