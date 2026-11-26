//! A pseudo terminal: a program run as if in a terminal window, which is told what's typed and says what it printed, as text
//! with the escape sequences in it. On Windows this is ConPTY, so anything that runs in a console works: shells, vim, ssh, a
//! build's progress bars. Output is decoded from UTF-8 as it arrives, with a character cut in half by a read kept for the next.
//!
//! Nothing here knows what the escape sequences mean. The program that draws the terminal does.

use std::sync::Arc;

/// What to run, and how big its terminal is.
#[derive(Clone, Debug)]
pub struct PtyOptions {
    /// A command line, as typed: `powershell.exe -NoLogo`, or `"C:\Program Files\Git\bin\bash.exe" -l`.
    pub command: String,
    /// Where it starts. The program's own folder when there isn't one.
    pub cwd: Option<String>,
    /// Added to the environment it inherits.
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

/// Run on the thread that reads, with what the program printed.
pub type OnData = Arc<dyn Fn(String) + Send + Sync>;
/// Run once, on another thread, when the program's gone and everything it printed has been given: its exit code.
pub type OnExit = Box<dyn FnOnce(u32) + Send>;

/// Whole characters of what's read so far, keeping the start of one that's been cut off for the next read.
#[cfg(any(windows, test))]
fn take_text(carry: &mut Vec<u8>) -> String {
    let mut text = String::new();
    loop {
        match std::str::from_utf8(carry) {
            Ok(valid) => {
                text.push_str(valid);
                carry.clear();
                return text;
            }
            Err(error) => {
                let good = error.valid_up_to();
                text.push_str(std::str::from_utf8(&carry[..good]).unwrap_or(""));
                match error.error_len() {
                    Some(bad) => {
                        text.push('\u{fffd}');
                        carry.drain(..good + bad);
                    }
                    None => {
                        // Cut off at the end: the rest of it's next.
                        carry.drain(..good);
                        return text;
                    }
                }
            }
        }
    }
}


#[cfg(any(windows, test))]
mod input;

#[cfg(windows)]
mod imp {
    use super::take_text;
    use super::{OnData, OnExit, PtyOptions};
    use std::io::Read as _;
    use super::input::Input;
    use std::fs::File;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::sync::{Arc, Weak};
    use std::sync::Mutex;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows::Win32::System::IO::CancelIoEx;
    use windows::Win32::System::Console::{ClosePseudoConsole, COORD, CreatePseudoConsole, HPCON, ResizePseudoConsole};
    use windows::Win32::System::Pipes::CreatePipe;
    use windows::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
        LPPROC_THREAD_ATTRIBUTE_LIST, CREATE_UNICODE_ENVIRONMENT, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTUPINFOEXW,
        STARTF_USESTDHANDLES, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
    };
    use windows::core::PWSTR;

    /// A running program and its console.
    pub struct Pty {
        console: Arc<Mutex<Option<isize>>>,
        input: Arc<Input>,
        keyboard: Weak<File>,
        process: Weak<OwnedHandle>,
        pid: u32,
    }

    fn close_console(console: &Mutex<Option<isize>>) {
        let owned = console.lock().unwrap().take();
        if let Some(value) = owned {
            // SAFETY: the waiter (or a failed startup) owns the one close, after any resize.
            unsafe { ClosePseudoConsole(HPCON(value)); }
        }
    }

    /// A pipe's end as something to read.
    struct PipeReader(HANDLE);
    // SAFETY: a handle is a number.
    unsafe impl Send for PipeReader {}
    impl std::io::Read for PipeReader {
        fn read(&mut self, into: &mut [u8]) -> std::io::Result<usize> {
            loop {
                let mut got = 0u32;
                // SAFETY: the buffer is this one's, and the handle is open until the reader is dropped.
                match unsafe { windows::Win32::Storage::FileSystem::ReadFile(self.0, Some(&mut *into), Some(&raw mut got), None) } {
                    // The console can write nothing at all, which isn't the end.
                    Ok(()) if got == 0 => {}
                    Ok(()) => return Ok(got as usize),
                    // The other end closing is the end of what's printed.
                    Err(_) => return Ok(0),
                }
            }
        }
    }
    impl Drop for PipeReader {
        fn drop(&mut self) {
            // SAFETY: opened for this, closed once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The environment block: what's inherited, then what's added, sorted as the system wants and with no name twice.
    fn environment(extra: &[(String, String)]) -> Vec<u16> {
        let mut vars: Vec<(String, String)> = std::env::vars().collect();
        for (name, value) in extra {
            vars.retain(|(have, _)| !have.eq_ignore_ascii_case(name));
            vars.push((name.clone(), value.clone()));
        }
        vars.sort_by_key(|(name, _)| name.to_uppercase());
        let mut block = Vec::new();
        for (name, value) in vars {
            block.extend(format!("{name}={value}").encode_utf16());
            block.push(0);
        }
        block.push(0);
        block
    }

    impl Pty {
        pub fn spawn(options: &PtyOptions, on_data: OnData, on_exit: OnExit) -> Result<Pty, String> {
            let cols = options.cols.max(1);
            let rows = options.rows.max(1);
            // SAFETY: each call is given what it asks for, and what's opened is closed on every way out.
            unsafe {
                let (mut in_read, mut in_write) = (HANDLE::default(), HANDLE::default());
                let (mut out_read, mut out_write) = (HANDLE::default(), HANDLE::default());
                CreatePipe(&raw mut in_read, &raw mut in_write, None, 0).map_err(|e| format!("no pipe for the keyboard: {e}"))?;
                if let Err(e) = CreatePipe(&raw mut out_read, &raw mut out_write, None, 0) {
                    let _ = CloseHandle(in_read);
                    let _ = CloseHandle(in_write);
                    return Err(format!("no pipe for the screen: {e}"));
                }
                let console = match CreatePseudoConsole(COORD { X: cols as i16, Y: rows as i16 }, in_read, out_write, 0) {
                    Ok(console) => console,
                    Err(e) => {
                        for h in [in_read, in_write, out_read, out_write] {
                            let _ = CloseHandle(h);
                        }
                        return Err(format!("the console couldn't be made: {e}"));
                    }
                };
                // The console has its own copies of these ends now.
                let _ = CloseHandle(in_read);
                let _ = CloseHandle(out_write);
                let fail = |why: String| {
                    ClosePseudoConsole(console);
                    let _ = CloseHandle(in_write);
                    let _ = CloseHandle(out_read);
                    Err(why)
                };

                let mut size = 0usize;
                let _ = InitializeProcThreadAttributeList(None, 1, None, &raw mut size);
                let mut list_memory = vec![0u8; size.max(1)];
                let list = LPPROC_THREAD_ATTRIBUTE_LIST(list_memory.as_mut_ptr().cast());
                if let Err(e) = InitializeProcThreadAttributeList(Some(list), 1, None, &raw mut size) {
                    return fail(format!("the process's attributes couldn't be made: {e}"));
                }
                if let Err(e) = UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                    Some(console.0 as *const std::ffi::c_void),
                    size_of::<HPCON>(),
                    None,
                    None,
                ) {
                    DeleteProcThreadAttributeList(list);
                    return fail(format!("the console couldn't be given to the process: {e}"));
                }

                let mut startup = STARTUPINFOEXW::default();
                startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
                startup.lpAttributeList = list;
                // Without this the program takes the standard handles this one has, which are pipes when this was started by
                // another program, and what it prints never reaches the console.
                startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
                startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
                startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
                startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
                let mut command = wide(&options.command);
                let environment = environment(&options.env);
                let cwd = options.cwd.as_deref().filter(|c| !c.is_empty()).map(wide);
                let mut info = PROCESS_INFORMATION::default();
                let made = CreateProcessW(
                    None,
                    Some(PWSTR(command.as_mut_ptr())),
                    None,
                    None,
                    false,
                    EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                    Some(environment.as_ptr().cast()),
                    match &cwd {
                        Some(cwd) => windows::core::PCWSTR(cwd.as_ptr()),
                        None => windows::core::PCWSTR::null(),
                    },
                    &startup.StartupInfo,
                    &raw mut info,
                );
                DeleteProcThreadAttributeList(list);
                if let Err(e) = made {
                    return fail(format!("{} couldn't be started: {e}", options.command));
                }
                let _ = CloseHandle(info.hThread);

                let process = Arc::new(OwnedHandle::from_raw_handle(info.hProcess.0));
                let console = Arc::new(Mutex::new(Some(console.0)));
                let keyboard = Arc::new(File::from_raw_handle(in_write.0));
                let pty = Pty {
                    console: console.clone(),
                    input: Arc::new(Input::default()),
                    keyboard: Arc::downgrade(&keyboard),
                    process: Arc::downgrade(&process),
                    pid: info.dwProcessId,
                };
                let input = pty.input.clone();
                let writer = std::thread::Builder::new().name("rsui-pty-write".into()).spawn(move || input.run(&*keyboard));
                let writer = match writer {
                    Ok(thread) => thread,
                    Err(error) => {
                        pty.kill();
                        let _ = CloseHandle(out_read);
                        close_console(&console);
                        return Err(format!("the keyboard thread couldn't start: {error}"));
                    }
                };

                let reader_pipe = PipeReader(out_read);
                let reader = std::thread::Builder::new().name("rsui-pty-read".into()).spawn(move || {
                    let mut pipe = reader_pipe;
                    let mut buffer = vec![0u8; 64 * 1024];
                    let mut carry = Vec::new();
                    loop {
                        match pipe.read(&mut buffer) {
                            Ok(0) | Err(_) => break,
                            Ok(got) => {
                                carry.extend_from_slice(&buffer[..got]);
                                let text = take_text(&mut carry);
                                if !text.is_empty() {
                                    on_data(text);
                                }
                            }
                        }
                    }
                });
                let reader = match reader {
                    Ok(thread) => thread,
                    Err(error) => {
                        pty.kill();
                        close_console(&console);
                        return Err(format!("the screen thread couldn't start: {error}"));
                    }
                };
                let input = pty.input.clone();
                let ending_console = console.clone();
                let waiting_process = process.clone();
                let waiter = std::thread::Builder::new().name("rsui-pty-wait".into()).spawn(move || {
                    let handle = HANDLE(waiting_process.as_raw_handle());
                    let _ = WaitForSingleObject(handle, u32::MAX);
                    let mut code = 0u32;
                    let _ = GetExitCodeProcess(handle, &raw mut code);
                    input.close();
                    // Closing the console ends both pipes, even if a write raced with cancellation.
                    close_console(&ending_console);
                    let _ = writer.join();
                    let _ = reader.join();
                    on_exit(code);
                });
                if let Err(error) = waiter {
                    pty.kill();
                    close_console(&console);
                    return Err(format!("the waiting thread couldn't start: {error}"));
                }
                Ok(pty)
            }
        }

        pub fn pid(&self) -> u32 {
            self.pid
        }

        /// Enqueues typed bytes in order. Above 1 MiB of pending input the producer waits for space.
        pub fn write(&self, bytes: &[u8]) {
            self.input.enqueue(bytes);
        }

        pub fn resize(&self, cols: u16, rows: u16) {
            let console = self.console.lock().unwrap();
            if let Some(value) = *console {
                // SAFETY: the guard keeps the console open until the resize completes.
                unsafe { let _ = ResizePseudoConsole(HPCON(value), COORD { X: cols.max(1) as i16, Y: rows.max(1) as i16 }); }
            }
        }

        /// Ends the program and wakes any producer waiting for input space.
        pub fn kill(&self) {
            self.input.close();
            // SAFETY: owned handles stay valid for both calls.
            unsafe {
                if let Some(keyboard) = self.keyboard.upgrade() {
                    let _ = CancelIoEx(HANDLE(keyboard.as_raw_handle()), None);
                }
                if let Some(process) = self.process.upgrade() {
                    let _ = TerminateProcess(HANDLE(process.as_raw_handle()), 1);
                }
            }
        }

        pub fn finished(&self) -> bool {
            // SAFETY: the process handle is owned by this terminal.
            self.process.upgrade().is_none_or(|process| unsafe { WaitForSingleObject(HANDLE(process.as_raw_handle()), 0) == WAIT_OBJECT_0 })
        }
    }

    impl Drop for Pty {
        fn drop(&mut self) {
            self.kill();
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{OnData, OnExit, PtyOptions};

    pub struct Pty;

    impl Pty {
        pub fn spawn(_: &PtyOptions, _: OnData, _: OnExit) -> Result<Pty, String> {
            Err("pseudo terminals are only here on Windows so far".to_owned())
        }
        pub fn pid(&self) -> u32 {
            0
        }
        pub fn write(&self, _: &[u8]) {}
        pub fn resize(&self, _: u16, _: u16) {}
        pub fn kill(&self) {}
        pub fn finished(&self) -> bool {
            true
        }
    }
}

pub use imp::Pty;

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn run(command: &str, then: impl FnOnce(&Pty)) -> (String, u32) {
        let (data_tx, data_rx) = mpsc::channel::<String>();
        let (exit_tx, exit_rx) = mpsc::channel::<u32>();
        let options = PtyOptions { command: command.to_owned(), cwd: None, env: vec![("RSUI_PTY_TEST".into(), "yes".into())], cols: 80, rows: 24 };
        let pty = Pty::spawn(&options, Arc::new(move |text| { let _ = data_tx.send(text); }), Box::new(move |code| { let _ = exit_tx.send(code); })).unwrap();
        then(&pty);
        let code = exit_rx.recv_timeout(Duration::from_secs(20)).expect("the program ended");
        let mut text = String::new();
        while let Ok(more) = data_rx.try_recv() {
            text.push_str(&more);
        }
        (text, code)
    }

    #[test]
    fn what_a_program_prints_comes_back_with_its_exit_code() {
        let (text, code) = run(r"cmd.exe /c echo hello-pty & exit 7", |_| {});
        assert!(text.contains("hello-pty"), "{text:?}");
        assert_eq!(code, 7);
    }

    #[test]
    fn what_is_typed_reaches_the_program_and_the_environment_is_added_to() {
        let (text, code) = run("cmd.exe", |pty| {
            pty.write(b"echo %RSUI_PTY_TEST%-\xc3\xa9\r");
            std::thread::sleep(Duration::from_millis(600));
            pty.write(b"exit 3\r");
        });
        assert!(text.contains("yes-\u{e9}"), "{text:?}");
        assert_eq!(code, 3);
    }

    #[test]
    #[ignore = "child fixture run inside a real ConPTY by a_slow_program_receives_ordered_input"]
    fn read_paste_fixture() {
        use std::io::Read;
        use windows::Win32::System::Console::{CONSOLE_MODE, GetStdHandle, STD_INPUT_HANDLE, SetConsoleMode};
        unsafe {
            let stdin = GetStdHandle(STD_INPUT_HANDLE).unwrap();
            SetConsoleMode(stdin, CONSOLE_MODE(0)).unwrap();
        }
        println!("READY_FOR_PASTE");
        std::thread::sleep(Duration::from_millis(150));
        let mut bytes = vec![0; 64 * 1024 + 3];
        std::io::stdin().read_exact(&mut bytes).unwrap();
        assert!(bytes[..64 * 1024].iter().all(|b| *b == b'a'));
        assert_eq!(&bytes[64 * 1024..], b"end");
        println!("PASTE_RECEIVED");
    }

    #[test]
    fn a_slow_program_receives_ordered_input() {
        let exe = std::env::current_exe().unwrap();
        let command = format!(r#""{}" --ignored --exact pty::tests::read_paste_fixture --nocapture"#, exe.display());
        let options = PtyOptions { command, cwd: None, env: vec![], cols: 100, rows: 30 };
        let (tx, rx) = mpsc::channel();
        let (done, exit) = mpsc::channel();
        let pty = Pty::spawn(&options, Arc::new(move |s| { let _ = tx.send(s); }), Box::new(move |c| { let _ = done.send(c); })).unwrap();
        let mut output = String::new();
        while !output.contains("READY_FOR_PASTE") {
            output.push_str(&rx.recv_timeout(Duration::from_secs(10)).unwrap());
        }
        pty.write(&vec![b'a'; 64 * 1024]);
        pty.write(b"end");
        assert_eq!(exit.recv_timeout(Duration::from_secs(15)).unwrap(), 0);
        output.extend(rx.try_iter());
        assert!(output.contains("PASTE_RECEIVED"), "{output:?}");
    }

    #[test]
    fn disposing_a_terminal_with_pending_input_ends_its_workers() {
        let options = PtyOptions { command: "cmd.exe".into(), cwd: None, env: vec![], cols: 80, rows: 25 };
        let (done, exit) = mpsc::channel();
        let pty = Pty::spawn(&options, Arc::new(|_| {}), Box::new(move |c| { let _ = done.send(c); })).unwrap();
        pty.write(&vec![b'x'; 1024 * 1024]);
        drop(pty);
        assert_eq!(exit.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
    }

    #[test]
    fn a_program_that_doesnt_exist_is_an_error_not_a_hang() {
        let options = PtyOptions { command: "definitely-not-a-program.exe".into(), cwd: None, env: vec![], cols: 80, rows: 24 };
        assert!(Pty::spawn(&options, Arc::new(|_| {}), Box::new(|_| {})).is_err());
    }

    #[test]
    fn a_character_cut_between_reads_is_kept_whole() {
        let mut carry = vec![0xc3];
        assert_eq!(super::take_text(&mut carry), "");
        carry.push(0xa9);
        assert_eq!(super::take_text(&mut carry), "\u{e9}");
    }
}
