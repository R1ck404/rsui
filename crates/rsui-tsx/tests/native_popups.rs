//! `native` from `rsui`, the system's own popups. None comes up here: what's asked is said, and the `dialog` step says what
//! the next one's answered with, as someone would have.

use std::path::PathBuf;
use std::process::Command;

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

/// An app with a log of what's happened to it, and a button for each of `buttons`, by key and what it does.
fn app(name: &str, buttons: &[(&str, &str)]) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-popups-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    let buttons: String = buttons
        .iter()
        .map(|(key, run)| format!("<button key=\"{key}\" onClick={{async () => {{ try {{ {run} }} catch (error) {{ log('failed ' + error.message); }} }}}}>{key}</button>\n"))
        .collect();
    let source = format!(
        r#"
        import {{ useState, native }} from 'rsui';
        let said = '';
        let redraw = () => {{}};
        const log = (text) => {{ said += text + ';'; redraw(); }};
        function Log() {{
            const [, set] = useState(0);
            redraw = () => set((n) => n + 1);
            return <text>{{'log ' + said}}</text>;
        }}
        export default function App() {{
            return (
                <window>
                    <Log />
                    {buttons}
                </window>
            );
        }}
        "#
    );
    std::fs::write(&file, source).unwrap();
    file
}

fn works(file: &PathBuf, steps: &str) -> String {
    let output = Command::new(TOOL).arg("--do").arg(steps).arg(file).output().unwrap();
    let said = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "{said}");
    said
}

#[test]
fn a_confirmation_is_answered_by_the_steps_and_cancelled_without_them() {
    let file = app(
        "confirm",
        &[("delete", "log('delete ' + await native.confirm({ title: 'Delete this list?', confirm: 'Delete', cancel: 'Keep', destructive: true }));")],
    );
    let said = works(
        &file,
        "dialog \"Delete\"; click #delete; wait \"delete true;\"; click #delete; wait \"delete false;\"; dialog \"Keep\"; click #delete; wait \"delete false;delete false;\"; dialog cancel; click #delete; wait \"delete false;delete false;delete false;\"",
    );
    assert!(said.contains("it asked to confirm \"Delete this list?\", \"\", and got true"), "{said}");
}

#[test]
fn an_alert_is_done_and_a_toast_and_share_are_said() {
    let file = app(
        "alert",
        &[
            ("alert", "await native.alert({ title: 'Saved', message: 'All done' }); log('seen');"),
            ("toast", "native.toast('Copied', { long: true }); log('toasted');"),
            ("share", "await native.share({ text: 'My list', url: 'https://example.com/list' }); log('shared');"),
            ("nothing", "await native.share({});"),
        ],
    );
    let said = works(&file, "click #alert; wait \"seen;\"; click #toast; wait \"toasted;\"; click #share; wait \"shared;\"; click #nothing; wait \"failed native.share needs\"");
    assert!(said.contains("it showed the message \"Saved\", \"All done\""), "{said}");
    assert!(said.contains("it showed the toast \"Copied\", for a while"), "{said}");
    assert!(said.contains("it asked to share \"\", \"My list\", \"https://example.com/list\""), "{said}");
}

#[test]
fn a_sheet_is_answered_by_an_option_or_its_place_or_not_at_all() {
    let file = app(
        "sheet",
        &[("sort", "log('sort ' + await native.sheet({ title: 'Sort by', options: ['Name', 'Date', 'Size'] }));")],
    );
    let said = works(
        &file,
        "dialog \"Date\"; click #sort; wait \"sort 1;\"; dialog \"2\"; click #sort; wait \"sort 2;\"; click #sort; wait \"sort null;\"; dialog \"Colour\"; click #sort; wait \"failed the sheet\"",
    );
    assert!(said.contains("it asked \"Sort by\" with [\"Name\", \"Date\", \"Size\"], and got \"Date\""), "{said}");
}

#[test]
fn a_day_and_a_time_are_picked_by_the_steps() {
    let file = app(
        "pickers",
        &[
            (
                "day",
                "const when = await native.pickDate({ value: new Date(2026, 10, 21, 9, 30), max: new Date(2026, 11, 31) }); log(when === null ? 'no day' : 'day ' + when.getFullYear() + '/' + (when.getMonth() + 1) + '/' + when.getDate() + ' ' + when.getHours() + ':' + when.getMinutes());",
            ),
            (
                "time",
                "const time = await native.pickTime({ value: { hours: 8, minutes: 0 } }); log(time === null ? 'no time' : 'time ' + time.hours + ':' + time.minutes);",
            ),
        ],
    );
    let said = works(
        &file,
        "dialog \"2026-12-03\"; click #day; wait \"day 2026/12/3 9:30;\"; click #day; wait \"no day;\"; dialog \"2027-01-01\"; click #day; wait \"failed 2027-01-01 isn't one\"; dialog \"14:05\"; click #time; wait \"time 14:5;\"; dialog cancel; click #time; wait \"no time;\"",
    );
    assert!(said.contains("it asked for a day, and got 2026-12-03"), "{said}");
    assert!(said.contains("it asked for a time, and got 14:05"), "{said}");
}

/// The real thing, in a program running with a real window, on a desktop of its own that nobody can see, with each popup
/// answered by pressing its buttons from here.
#[cfg(windows)]
mod really {
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WAIT_OBJECT_0, WPARAM};
    use windows::Win32::System::StationsAndDesktops::{CloseDesktop, CreateDesktopW, DESKTOP_CONTROL_FLAGS, EnumDesktopWindows};
    use windows::Win32::System::Threading::{
        CREATE_NO_WINDOW, CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, IDOK, IsWindowVisible, PostMessageW, WM_COMMAND};
    use windows::core::{BOOL, PCWSTR, PWSTR};

    /// What a task dialog does when one of its buttons is pressed.
    const TDM_CLICK_BUTTON: u32 = 0x0400 + 102;

    fn dialogs(desktop: windows::Win32::System::StationsAndDesktops::HDESK) -> Vec<HWND> {
        unsafe extern "system" fn look(window: HWND, found: LPARAM) -> BOOL {
            let mut class = [0u16; 16];
            // SAFETY: this is called while the list it's given is still there, and `class` is big enough.
            unsafe {
                let length = GetClassNameW(window, &mut class);
                if IsWindowVisible(window).as_bool() && String::from_utf16_lossy(&class[..length as usize]) == "#32770" {
                    (*(found.0 as *mut Vec<HWND>)).push(window);
                }
            }
            BOOL(1)
        }
        let mut found = Vec::<HWND>::new();
        // SAFETY: `found` lasts through the call.
        unsafe {
            let _ = EnumDesktopWindows(Some(desktop), Some(look), LPARAM((&raw mut found) as isize));
        }
        found
    }

    #[test]
    fn each_popup_is_the_systems_own_and_its_answer_comes_back() {
        let folder = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("tsx-popups-really-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let out = folder.join("said.txt");
        let _ = std::fs::remove_file(&out);
        let app = folder.join("app.tsx");
        std::fs::write(
            &app,
            format!(
                r#"
                import {{ useEffect, native }} from 'rsui';
                import fs from 'fs';
                export const title = 'Popups';
                export default function App() {{
                    useEffect(() => {{
                        (async () => {{
                            const said = [];
                            try {{
                                said.push('confirm ' + await native.confirm({{ title: 'Go on?', confirm: 'Go', destructive: true }}));
                                said.push('sheet ' + await native.sheet({{ title: 'Pick', options: ['a', 'b', 'c'] }}));
                                await native.alert({{ title: 'Done' }});
                                said.push('alert');
                                const day = await native.pickDate({{ value: new Date(2026, 10, 21, 8, 15) }});
                                said.push('day ' + day.getDate() + ' ' + day.getHours());
                                native.toast('Bye');
                            }} catch (error) {{
                                said.push('failed ' + error.message);
                            }}
                            fs.writeFileSync({out:?}, said.join('\n'));
                            process.exit(0);
                        }})();
                    }}, []);
                    return <window><text>Popups</text></window>;
                }}
                "#,
                out = out.display().to_string()
            ),
        )
        .unwrap();
        let name = format!("rsui-tsx-popups-{}", std::process::id());
        let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let desktop_name = wide(&name);
        // SAFETY: the name ends in a zero, and the desktop is closed once the program on it has gone.
        let desktop = unsafe { CreateDesktopW(PCWSTR(desktop_name.as_ptr()), None, None, DESKTOP_CONTROL_FLAGS(0), 0x10_0000 | 0xFF, None) }
            .expect("a desktop can be made");
        let mut command = wide(&format!("\"{}\" \"{}\"", super::TOOL, app.display()));
        let mut on_desktop = wide(&name);
        let started = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            lpDesktop: PWSTR(on_desktop.as_mut_ptr()),
            ..Default::default()
        };
        let mut running = PROCESS_INFORMATION::default();
        // SAFETY: everything handed over lasts through the call, and the handles that come back are closed below.
        unsafe {
            CreateProcessW(
                None,
                Some(PWSTR(command.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_NO_WINDOW,
                None,
                PCWSTR(wide(&folder.display().to_string()).as_ptr()),
                &raw const started,
                &raw mut running,
            )
        }
        .expect("the program starts");
        let began = Instant::now();
        let mut finished = false;
        let mut seen: Vec<(isize, usize)> = Vec::new();
        while began.elapsed() < Duration::from_secs(120) {
            // SAFETY: the process's handle is open until it's closed below.
            if unsafe { WaitForSingleObject(running.hProcess, 300) } == WAIT_OBJECT_0 {
                finished = true;
                break;
            }
            for dialog in dialogs(desktop) {
                // Each popup, in the order they come, gets the next answer: the confirmation its OK, the sheet its third
                // option, the alert its OK, and the picker, which isn't a task dialog, its OK.
                let at = match seen.iter().position(|(window, _)| *window == dialog.0 as isize) {
                    Some(at) => at,
                    None => {
                        seen.push((dialog.0 as isize, seen.len()));
                        seen.len() - 1
                    }
                };
                let button = [IDOK.0 as usize, rsui_first_option() + 2, IDOK.0 as usize].get(seen[at].1).copied().unwrap_or(IDOK.0 as usize);
                // SAFETY: it does nothing if the window's gone.
                unsafe {
                    if seen[at].1 == 3 {
                        let _ = PostMessageW(Some(dialog), WM_COMMAND, WPARAM(IDOK.0 as usize), LPARAM(0));
                    } else {
                        let _ = PostMessageW(Some(dialog), TDM_CLICK_BUTTON, WPARAM(button), LPARAM(0));
                    }
                }
            }
        }
        // SAFETY: the handles are this test's own, and the program it started is stopped if it didn't finish.
        unsafe {
            if !finished {
                let _ = TerminateProcess(running.hProcess, 1);
                let _ = WaitForSingleObject(running.hProcess, 5000);
            }
            let _ = CloseHandle(running.hThread);
            let _ = CloseHandle(running.hProcess);
            let _ = CloseDesktop(desktop);
        }
        let said = std::fs::read_to_string(&out).unwrap_or_default();
        assert!(finished, "the program didn't finish; it said {said:?}");
        assert_eq!(said, "confirm true\nsheet 2\nalert\nday 21 8");
    }

    /// Where a sheet's options start among a task dialog's buttons, as `rsui::popups::FIRST_OPTION` has it.
    fn rsui_first_option() -> usize {
        100
    }
}
