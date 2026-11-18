//! What a web view costs once it's used: how long finding and loading the runtime takes, how long the first page takes, and
//! how much memory the browser's processes take. It opens a real window with a page from the internet in it, so it's for a
//! person to run, not for tests:
//!
//! `cargo run --release -p rsui-webview --example measure -- [https://example.com/]`
//!
//! It prints one line of JSON and closes once the page has loaded and settled for two seconds. What it keeps is in a
//! folder of its own in the temporary folder, which it deletes after.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rsui_webview::{Bounds, Event, Settings, WebView};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_DESTROY {
        // SAFETY: ending this thread's message loop.
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    // SAFETY: the arguments this was given.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// The private bytes of `root` and every process under it, in MB.
fn private_mb(root: u32) -> f64 {
    let mut total = 0usize;
    // SAFETY: a snapshot of the processes, read and closed here, and each process opened only to read its memory.
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return 0.0;
        };
        let mut all = Vec::new();
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                all.push((entry.th32ProcessID, entry.th32ParentProcessID));
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
        let mut ours = vec![root];
        let mut at = 0;
        while at < ours.len() {
            let parent = ours[at];
            ours.extend(all.iter().filter(|(_, of)| *of == parent).map(|(id, _)| *id));
            at += 1;
        }
        for id in ours {
            if let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, id) {
                let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
                if GetProcessMemoryInfo(process, (&raw mut counters).cast(), size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32).is_ok() {
                    total += counters.PrivateUsage;
                }
                let _ = CloseHandle(process);
            }
        }
    }
    total as f64 / 1_048_576.0
}

fn main() {
    let url = std::env::args().nth(1).unwrap_or_else(|| "https://example.com/".to_owned());
    let found = Instant::now();
    let runtime = rsui_webview::runtime::find();
    let find_ms = found.elapsed().as_secs_f64() * 1000.0;
    let Some(runtime) = runtime else {
        eprintln!("{}", rsui_webview::runtime::missing());
        return;
    };
    let folder = std::env::temp_dir().join(format!("rsui-webview-measure-{}", std::process::id()));
    // SAFETY: a window class and a window of this thread's, with a message loop below.
    let window = unsafe {
        let instance = GetModuleHandleW(None).expect("this program is a module");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("rsui-webview-measure"),
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("rsui-webview-measure"),
            w!("rsui-webview: what a page from elsewhere costs"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1000,
            700,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("the window can be made")
    };
    let started = Instant::now();
    let finished: Rc<RefCell<Option<Instant>>> = Rc::default();
    let view = WebView::new(Settings { data_folder: folder.clone(), ..Settings::default() }, {
        let finished = finished.clone();
        move |event| {
            if event == Event::Finished && finished.borrow().is_none() {
                *finished.borrow_mut() = Some(Instant::now());
            }
            if let Event::Unavailable(why) = &event {
                eprintln!("{why}");
            }
        }
    });
    view.navigate(&url);
    view.place(window.0 as isize, Some(&Bounds { rect: [0, 0, 980, 650], shown: [0, 0, 980, 650], ..Bounds::default() }));
    let mut message = MSG::default();
    // SAFETY: this thread's message loop.
    unsafe {
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
            let settled = finished.borrow().is_some_and(|at| at.elapsed() > Duration::from_secs(2));
            if settled || started.elapsed() > Duration::from_secs(60) {
                break;
            }
            if finished.borrow().is_some() {
                // Woken often enough to see two seconds go by.
                SetTimer(Some(window), 1, 100, None);
            }
        }
    }
    let first_page_ms = finished.borrow().map(|at| (at - started).as_secs_f64() * 1000.0);
    let browser = view.browser_process();
    let browser_mb = browser.map(private_mb);
    let own_mb = private_mb(std::process::id()) - browser_mb.unwrap_or(0.0);
    println!(
        "{{\"runtime\":\"{}\",\"find_ms\":{find_ms:.2},\"first_page_ms\":{},\"browser_private_mb\":{},\"this_process_private_mb\":{own_mb:.1}}}",
        runtime.version,
        first_page_ms.map_or("null".to_owned(), |ms| format!("{ms:.0}")),
        browser_mb.map_or("null".to_owned(), |mb| format!("{mb:.1}")),
    );
    drop(view);
    // SAFETY: the window this made.
    unsafe {
        let _ = DestroyWindow(window);
    }
    // The browser lets go of its folder once its processes have gone.
    std::thread::sleep(Duration::from_secs(2));
    let _ = std::fs::remove_dir_all(&folder);
}
