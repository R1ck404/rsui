//! A packed Windows app starts a second copy of its executable to report crashes.
//! The reporter has a private pipe for panic details and a handle to the app process.

use std::backtrace::Backtrace;
use std::fmt::Write as _;
use std::io::{self, BufRead, IsTerminal, Write as _};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::sync::Mutex;

use rsui::popups::{self, Answer, Popup};
use windows::Win32::Foundation::{
    DUPLICATE_HANDLE_OPTIONS, DuplicateHandle, HANDLE, HWND, WAIT_OBJECT_0,
};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, GetProcessId, INFINITE,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

const REPORTER_FLAG: &str = "--rsui-internal-crash-reporter";
const RUST_PANIC_EXIT: u32 = 101;
const ISSUE_PAGE: &str = "https://github.com/R1ck404/rsui/issues/new";

#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
}

/// Starts a monitor for a packed GUI app. If it cannot be started, use the usual error handling.
pub fn run_packed(path: &Path, options: crate::Options) -> Result<(), crate::Error> {
    let monitoring = start_reporter();
    crate::run_with_app_panic(path, options, || {
        if monitoring {
            // An unwinding panic on the app thread was joined. Tell the reporter it was fatal.
            std::process::exit(RUST_PANIC_EXIT as i32);
        }
    })
}

fn start_reporter() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Ok(mut child) = Command::new(exe)
        .arg(REPORTER_FLAG)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let Some(mut output) = child.stdin.take() else {
        return false;
    };
    let mut parent = HANDLE::default();
    // SAFETY: our pseudo-handle and the child's process handle are valid during the call. The
    // returned handle belongs to the child. Passing it through the pipe avoids a PID lookup race.
    let duplicated = unsafe {
        let current = GetCurrentProcess();
        DuplicateHandle(
            current,
            current,
            HANDLE(child.as_raw_handle()),
            &mut parent,
            (PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE).0,
            false,
            DUPLICATE_HANDLE_OPTIONS(0),
        )
    };
    if duplicated.is_err()
        || output
            .write_all(&(parent.0 as usize).to_ne_bytes())
            .is_err()
    {
        return false;
    }

    let output = Mutex::new(output);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(mut pipe) = output.try_lock() {
            let (site, details) = panic_details(info);
            if let Ok(mut record) = serde_json::to_vec(&(site, details)) {
                record.push(b'\n');
                let _ = pipe.write_all(&record);
            }
        }
        previous(info);
    }));
    true
}

fn truncate(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn panic_details(info: &std::panic::PanicHookInfo<'_>) -> (String, String) {
    let site = info
        .location()
        .and_then(|at| {
            // Only a source basename can go into the URL. The payload and full path stay local.
            let file = at.file().rsplit(['/', '\\']).next()?;
            (file.len() <= 80
                && file
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)))
            .then(|| format!("{file}:{}:{}", at.line(), at.column()))
        })
        .unwrap_or_default();
    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-text panic payload");
    let location = info
        .location()
        .map_or_else(String::new, |at| at.to_string());
    let trace = Backtrace::force_capture().to_string();
    let details = format!(
        "Rust panic at {}\n{}\n\n{}",
        truncate(&location, 512),
        truncate(payload, 1024),
        truncate(&trace, 8192),
    );
    (site, details)
}

/// Handles the private reporter invocation before running any packed app startup code.
pub fn reporter_main() -> Option<ExitCode> {
    if std::env::args_os().nth(1)? != REPORTER_FLAG {
        return None;
    }
    let stdin = io::stdin();
    if !stdin.is_terminal()
        && let Some(report) = wait_for_report(stdin.lock())
    {
        report.show();
    }
    Some(ExitCode::SUCCESS)
}

fn wait_for_report(mut input: impl BufRead) -> Option<Report> {
    let mut handle = [0; size_of::<usize>()];
    input.read_exact(&mut handle).ok()?;
    let process = HANDLE(usize::from_ne_bytes(handle) as _);
    // SAFETY: the protocol supplies a duplicated handle owned by this process. Windows checks
    // it before using it as a process. The OS closes it when the reporter exits.
    let pid = unsafe { GetProcessId(process) };
    if pid == 0 || pid == std::process::id() {
        return None;
    }
    // Drain the pipe while the app is alive so that a panic can write its details before exiting.
    let panic = input
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<(String, String)>(&line).ok())
        .last();
    let mut code = 0;
    // SAFETY: the process handle was checked above. `code` is writable for GetExitCodeProcess.
    unsafe {
        if WaitForSingleObject(process, INFINITE) != WAIT_OBJECT_0 {
            return None;
        }
        GetExitCodeProcess(process, &mut code).ok()?;
    }
    Report::from_exit(code, panic)
}

struct Report {
    summary: String,
    details: Option<String>,
}

impl Report {
    fn from_exit(code: u32, panic: Option<(String, String)>) -> Option<Self> {
        let rust_panic = code == RUST_PANIC_EXIT
            || (panic.is_some() && matches!(code, 3 | 0x4000_0015 | 0xc000_0409));
        let native_crash = code >= 0xc000_0000 && code != 0xc000_013a; // Ctrl+C is a normal shutdown.
        if !rust_panic && !native_crash && code != 0x4000_0015 {
            return None;
        }
        let (site, details) = panic.filter(|_| rust_panic).unzip();
        let mut summary = if rust_panic {
            "Rust panic".to_owned()
        } else {
            "Native crash".to_owned()
        };
        if let Some(site) = site.filter(|site| !site.is_empty()) {
            let _ = write!(summary, " at {site}");
        }
        let _ = write!(summary, " (exit code 0x{code:08X})");
        Some(Self { summary, details })
    }

    fn body(&self) -> String {
        let version = env!("CARGO_PKG_VERSION");
        let arch = std::env::consts::ARCH;
        let os = windows_version();
        format!(
            "What were you doing when the app crashed?\n\nRSUI version: {version}\nWindows: {os} ({arch})\nError: {}",
            self.summary
        )
    }

    fn show(&self) {
        use rsui::desktop;

        const QUESTION: &str = "RSUI crashed. Report this error?";
        let body = self.body();
        let mut choices = vec![
            "Report on GitHub\nOpen a prefilled issue in your browser.".to_owned(),
            "Copy details\nCopy the crash details so you can review them before sharing."
                .to_owned(),
        ];
        loop {
            let popup = Popup::Sheet {
                title: QUESTION.into(),
                options: choices.clone(),
                cancel: None,
            };
            match popups::ask_on_this_thread(&popup, HWND::default()) {
                Ok(Answer::Chosen(Some(0))) => {
                    if desktop::open_url_on_this_thread(&issue_url(&body)).is_ok() {
                        return;
                    }
                    choices[0] = "Retry opening GitHub\nThe browser couldn't be opened.".into();
                }
                Ok(Answer::Chosen(Some(1))) => {
                    let text = self
                        .details
                        .as_ref()
                        .map_or_else(|| body.clone(), |details| format!("{body}\n\n{details}"));
                    let contents = desktop::ClipboardContents {
                        text: Some(text),
                        ..Default::default()
                    };
                    choices[1] = match desktop::write_clipboard(&contents) {
                        Ok(()) => "Copy details again\nCrash details copied to the clipboard.",
                        Err(_) => "Retry copying details\nThe clipboard couldn't be written.",
                    }
                    .to_owned();
                }
                Err(_) => {
                    if desktop::message_box(
                        QUESTION,
                        &body,
                        desktop::MessageKind::Error,
                        desktop::MessageButtons::YesNo,
                    ) == "yes"
                    {
                        let _ = desktop::open_url_on_this_thread(&issue_url(&body));
                    }
                    return;
                }
                _ => return,
            }
        }
    }
}

fn windows_version() -> String {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: the structure is initialized with the size RtlGetVersion expects.
    if unsafe { RtlGetVersion(&mut version) } != 0 {
        return "unknown".to_owned();
    }
    format!(
        "{}.{}.{}",
        version.dwMajorVersion, version.dwMinorVersion, version.dwBuildNumber
    )
}

fn issue_url(body: &str) -> String {
    let mut url = format!("{ISSUE_PAGE}?title=RSUI%20crash&body=");
    for byte in body.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            url.push(char::from(byte));
        } else {
            let _ = write!(url, "%{byte:02X}");
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_exits_and_handled_errors_do_not_report() {
        for code in [0, 1, 0xc000_013a] {
            assert!(
                Report::from_exit(code, Some(("test.rs:1:2".into(), "caught panic".into())))
                    .is_none()
            );
        }
        assert!(Report::from_exit(3, None).is_none());
    }

    #[test]
    fn native_crashes_do_not_include_stale_panic_details() {
        let report = Report::from_exit(
            0xc000_0005,
            Some(("test.rs:1:2".into(), "caught panic".into())),
        )
        .unwrap();
        assert_eq!(report.summary, "Native crash (exit code 0xC0000005)");
        assert!(report.details.is_none());
    }

    #[test]
    fn fatal_panic_details_stay_out_of_the_url() {
        for code in [RUST_PANIC_EXIT, 3, 0xc000_0409] {
            let report = Report::from_exit(
                code,
                Some(("example.rs:12:4".into(), "private panic message".into())),
            )
            .unwrap();
            let url = issue_url(&report.body());
            assert!(url.starts_with(
                "https://github.com/R1ck404/rsui/issues/new?title=RSUI%20crash&body="
            ));
            assert!(url.contains("Rust%20panic%20at%20example.rs%3A12%3A4"));
            assert!(!url.contains("private"));
            assert_eq!(report.details.as_deref(), Some("private panic message"));
        }
    }

    #[test]
    fn issue_body_escapes_url_delimiters_and_utf8() {
        assert!(issue_url("a&b#c=é\n").ends_with("body=a%26b%23c%3D%C3%A9%0A"));
    }
}
