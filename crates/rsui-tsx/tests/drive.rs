//! Using an app with `--do`, as an agent would: one command, and what it prints.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn app(name: &str, source: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("drive-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    file
}

fn run(file: &PathBuf, steps: &str) -> Output {
    Command::new(TOOL)
        .arg("--do")
        .arg(steps)
        .arg(file)
        .output()
        .unwrap()
}

fn said(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn complained(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// It worked, and here's what it said.
fn works(file: &PathBuf, steps: &str) -> String {
    let output = run(file, steps);
    assert!(
        output.status.success(),
        "{steps}\n{}\n{}",
        said(&output),
        complained(&output)
    );
    said(&output)
}

/// It failed, and here's why, and what it printed before that.
fn fails(file: &PathBuf, steps: &str) -> (String, String) {
    let output = run(file, steps);
    assert!(!output.status.success(), "{steps}\n{}", said(&output));
    (said(&output), complained(&output))
}

const COUNTER: &str = r#"
    import { useState } from 'rsui';
    export default function App() {
        const [count, setCount] = useState(0);
        const [name, setName] = useState('');
        return (
            <window padding={8} gap={8}>
                <text>{'Count is ' + count}</text>
                <button key="add" paddingX={14} paddingY={8} onClick={() => { console.log('adding'); setCount(count + 1); }}>Add one</button>
                <button key="off" paddingX={14} paddingY={8} disabled onClick={() => setCount(100)}>Not now</button>
                <input key="name" width={200} height={30} value={name} onChange={setName} />
                <text>{'Name is ' + name}</text>
            </window>
        );
    }
"#;

#[test]
fn clicks_can_hold_keys_and_the_pointer_can_move_with_nothing_pressed() {
    let file = app(
        "held",
        r#"
        import React, { useState } from 'react';
        export default function App() {
            const [log, setLog] = useState([]);
            const note = (said) => setLog((was) => [...was, said]);
            return (
                <div>
                    <button key="pick" onClick={(e) => note('click' + (e.ctrlKey ? ' ctrl' : '') + (e.shiftKey ? ' shift' : '') + ' ' + e.button)}
                        onAuxClick={(e) => note('aux ' + e.button)}>Pick</button>
                    <div key="pad" style={{ width: 100, height: 40 }} onMouseMove={(e) => note('move ' + Math.round(e.offsetX) + ',' + Math.round(e.offsetY))}>Pad</div>
                    <input key="field" onCompositionEnd={(e) => note('composed ' + e.data)} onKeyUp={(e) => note('up ' + e.key)} />
                    <p>{'log ' + log.join(', ')}</p>
                </div>
            );
        }
        "#,
    );
    let out = works(
        &file,
        "click #pick ctrl shift; expect \"click ctrl shift 0\"; click #pick; expect \"click ctrl shift 0, click 0\"; \
         middleclick #pick; expect \"aux 1\"; move #pad; expect \"move 50,20\"; press f5 #field; expect \"up F5\"; \
         compose 你好; expect \"composed 你好\"",
    );
    assert!(out.contains("> move #pad"), "{out}");
    let (_, why) = fails(&file, "click #pick hyper");
    assert!(why.contains("ctrl, shift, alt or win"), "{why}");
}

#[test]
fn things_can_be_dragged_about_and_dropped_in_from_other_programs() {
    let file = app(
        "dragging",
        r#"
        import React, { useState } from 'react';
        export default function App() {
            const [log, setLog] = useState([]);
            const note = (said) => setLog((was) => [...was, said]);
            return (
                <div>
                    <div key="card" draggable style={{ width: 80, height: 30 }} onDragStart={(e) => e.dataTransfer.setData('text', 'card')}>Card</div>
                    <div key="zone" style={{ width: 200, height: 60 }} onDragOver={(e) => e.preventDefault()}
                        onDrop={(e) => note('got ' + (e.dataTransfer.getData('text') || [...e.dataTransfer.files].map((f) => f.name).join('+')))}>Zone</div>
                    <p>{'log ' + log.join(', ')}</p>
                </div>
            );
        }
        "#,
    );
    let out = works(
        &file,
        "dragto #card #zone; expect \"got card\"; dropfiles #zone C:/a/one.txt C:/b/two.png; expect \"got one.txt+two.png\"; \
         droptext #zone hello there; expect \"got hello there\"; droptext #card nope",
    );
    assert!(out.contains("nothing there takes it"), "{out}");
}

#[test]
fn clicking_a_key_changes_the_app_and_says_what_changed() {
    let file = app("click", COUNTER);
    let out = works(&file, "click #add; click #add; expect \"Count is 2\"");
    assert!(out.contains("> click #add"), "{out}");
    assert!(out.contains("Count is 1"), "says what changed: {out}");
    assert!(out.contains("adding"), "and what it logged: {out}");
}

#[test]
fn something_can_be_clicked_by_what_it_says() {
    let file = app("name", COUNTER);
    works(&file, "click \"Add one\"; expect \"Count is 1\"");
    works(&file, "click \"Add\"; expect \"Count is 1\"");
}

#[test]
fn what_is_disabled_stays_that_way() {
    let file = app("disabled", COUNTER);
    let (out, why) = fails(&file, "click #off");
    assert!(why.contains("disabled"), "{why}");
    assert!(out.contains("Count is 0"), "{out}");
    let (_, why) = fails(&file, "act #off click");
    assert!(why.contains("disabled"), "{why}");
}

#[test]
fn typing_takes_the_place_of_what_is_there() {
    let file = app("type", COUNTER);
    let out = works(
        &file,
        "type #name \"first one\"; expect \"Name is first one\"; type #name second; expect \"Name is second\"; expect no \"first\"; type #name \"\"; expect no \"second\"",
    );
    assert!(out.contains("Name is first one"), "{out}");
}

#[test]
fn a_step_that_fails_says_which_and_why_and_shows_what_was_there() {
    let file = app("fails", COUNTER);
    let (out, why) = fails(&file, "click #add; expect \"Count is 5\"; click #add");
    assert!(why.contains("step 2 failed"), "{why}");
    assert!(why.contains("Count is 5"), "{why}");
    assert!(out.contains("Count is 1"), "what was there: {out}");
    assert!(!out.contains("Count is 2"), "it stopped: {out}");
}

#[test]
fn something_can_be_clicked_by_where_it_is_when_it_has_no_key_and_its_name_is_not_its_own() {
    let file = app(
        "paths",
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [got, setGot] = useState('nothing');
            return (
                <window>
                    <button height={30} onClick={() => setGot('first')}>Same</button>
                    <button height={30} onClick={() => setGot('second')}>Same</button>
                    <text>{'Got ' + got}</text>
                </window>
            );
        }
        "#,
    );
    works(
        &file,
        "click @1; expect \"Got second\"; click @0; expect \"Got first\"",
    );
    let (_, why) = fails(&file, "click \"Same\"");
    assert!(why.contains("@path"), "what to do about it: {why}");
    let (_, why) = fails(&file, "click @9");
    assert!(why.contains("@9"), "{why}");
}

#[test]
fn what_is_not_there_says_what_is_instead() {
    let file = app("missing", COUNTER);
    let (_, why) = fails(&file, "click #nope");
    assert!(why.contains("nope"), "{why}");
    assert!(why.contains("#add"), "what could have been meant: {why}");
}

#[test]
fn nonsense_says_what_the_steps_are() {
    let file = app("nonsense", COUNTER);
    let (_, why) = fails(&file, "dance");
    assert!(why.contains("click TARGET"), "{why}");
    let (_, why) = fails(&file, "");
    assert!(why.contains("click TARGET"), "{why}");
    let (_, why) = fails(&file, "press banana");
    assert!(why.contains("isn't a key"), "{why}");
}

#[test]
fn an_app_that_will_not_load_says_why() {
    let file = app(
        "broken",
        "export default function App() { return <window>; }",
    );
    let (_, why) = fails(&file, "look");
    assert!(!why.is_empty());
}

#[test]
fn steps_can_come_from_a_file_or_the_standard_input() {
    let file = app("from", COUNTER);
    let steps = file.with_file_name("steps.txt");
    std::fs::write(
        &steps,
        "// Adding one.\nclick #add\n\nexpect \"Count is 1\"\n",
    )
    .unwrap();
    works(&file, &format!("@{}", steps.display()));

    use std::io::Write;
    let mut child = Command::new(TOOL)
        .args(["--do", "-"])
        .arg(&file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input
        .write_all(b"click #add; expect \"Count is 1\"")
        .unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", complained(&output));
}

#[test]
fn a_picture_can_be_taken_along_the_way_at_any_size() {
    let file = app("shot", COUNTER);
    let first = file.with_file_name("first.png");
    let second = file.with_file_name("second.png");
    let steps = format!(
        "shot \"{}\"; size 300 200; shot \"{}\"; expect \"Count is 0\"",
        first.display().to_string().replace('\\', "\\\\"),
        second.display().to_string().replace('\\', "\\\\"),
    );
    works(&file, &steps);
    let size_of = |path: &PathBuf| {
        let bytes = std::fs::read(path).expect("there's a picture");
        (
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        )
    };
    assert_eq!(size_of(&first), (960, 640));
    assert_eq!(size_of(&second), (300, 200));
}

const WINDOWS: &str = r#"
    import { useState, Window } from 'rsui';
    export const title = 'The main one';
    export default function App() {
        const [open, setOpen] = useState(false);
        const [asking, setAsking] = useState(false);
        const [count, setCount] = useState(0);
        return (
            <window>
                <text>{'Main says ' + count}</text>
                <button key="open" height={30} onClick={() => setOpen(true)}>Open</button>
                <button key="ask" height={30} onClick={() => setAsking(true)}>Ask</button>
                {open ? (
                    <Window title="Settings" size={[300, 200]} onClose={() => setOpen(false)}>
                        <button key="add" height={30} onClick={() => setCount(count + 1)}>{'Add to ' + count}</button>
                    </Window>
                ) : null}
                {asking ? (
                    <Window title="Sure?" size={[200, 100]} modal onClose={() => setAsking(false)}>
                        <button key="yes" height={30} onClick={() => setAsking(false)}>Yes</button>
                    </Window>
                ) : null}
            </window>
        );
    }
"#;

#[test]
fn another_window_can_be_used_and_changes_reach_the_rest() {
    let file = app("windows", WINDOWS);
    let out = works(
        &file,
        "click #open; window \"Settings\"; click #add; expect \"Add to 1\"; window 0; expect \"Main says 1\"",
    );
    assert!(out.contains("\"Settings\" opened"), "{out}");
    assert!(
        out.contains("window 1 \"Settings\":"),
        "all of it at the end: {out}"
    );
}

#[test]
fn closing_a_window_asks_it_and_closing_the_main_one_ends_the_app() {
    let file = app("close", WINDOWS);
    let out = works(&file, "click #open; window 1; close; expect \"Open\"");
    assert!(out.contains("\"Settings\" closed"), "{out}");
    let out = works(&file, "close");
    assert!(out.contains("the app closed"), "{out}");
    let (_, why) = fails(&file, "close; look");
    assert!(why.contains("has closed"), "{why}");
}

#[test]
fn a_modal_window_keeps_the_main_one_from_being_used_as_it_would_a_person() {
    let file = app("modal", WINDOWS);
    let (out, why) = fails(&file, "click #ask; click #open");
    assert!(why.contains("Sure?"), "{why}");
    assert!(out.contains("\"Sure?\""), "{out}");
    works(
        &file,
        "click #ask; window \"Sure?\"; click #yes; window 0; click #open; expect \"Open\"",
    );
}

#[test]
fn a_window_that_does_not_exist_says_which_do() {
    let file = app("no-window", WINDOWS);
    let (_, why) = fails(&file, "window 7");
    assert!(why.contains("The main one"), "{why}");
}

#[test]
fn no_real_dialog_ever_comes_up_and_the_answer_can_be_given() {
    let file = app(
        "dialog",
        r#"
        import { useState } from 'rsui';
        import { dialogs } from 'rsui/desktop';
        export default function App() {
            const [got, setGot] = useState('nothing yet');
            return (
                <window>
                    <button key="open" height={30} onClick={async () => setGot(String(await dialogs.open({ title: "Pick" })))}>Open</button>
                    <text>{'Got ' + got}</text>
                </window>
            );
        }
        "#,
    );
    // Nothing would come back from a real one, as there's nobody to answer it.
    let out = works(&file, "click #open; wait \"Got null\"");
    assert!(out.contains("a dialog was asked for"), "{out}");
    works(
        &file,
        "dialog \"C:\\\\some\\\\file.txt\"; click #open; wait \"file.txt\"; dialog cancel; click #open; wait \"Got null\"",
    );
}

#[test]
fn opening_a_link_only_says_so() {
    let file = app(
        "url",
        r#"
        import { shell } from 'rsui/desktop';
        export default function App() {
            return <window><button key="go" height={30} onClick={() => shell.open('https://example.com/')}>Go</button></window>;
        }
        "#,
    );
    let out = works(&file, "click #go");
    assert!(out.contains("https://example.com/"), "{out}");
    assert!(out.contains("wasn't done"), "{out}");
}

#[test]
fn what_finishes_later_can_be_waited_for() {
    let file = app(
        "later",
        r#"
        import { useEffect, useState } from 'rsui';
        export default function App() {
            const [done, setDone] = useState(false);
            useEffect(() => { const timer = setTimeout(() => setDone(true), 2500); return () => clearTimeout(timer); }, []);
            return <window><text>{done ? 'Finished' : 'Working'}</text></window>;
        }
        "#,
    );
    works(
        &file,
        "expect \"Working\"; wait \"Finished\"; expect no \"Working\"",
    );
    let (_, why) = fails(&file, "wait \"Finished\" 10");
    assert!(why.contains("hadn't shown up"), "{why}");
    works(&file, "wait no \"Working\"");
}

#[test]
fn a_clock_does_not_hold_up_the_steps_but_what_happens_right_away_counts() {
    let file = app(
        "clock",
        r#"
        import { useEffect, useState } from 'rsui';
        export default function App() {
            const [time, setTime] = useState(0);
            const [soon, setSoon] = useState(false);
            useEffect(() => { const timer = setInterval(() => setTime((time) => time + 1), 5000); return () => clearInterval(timer); }, []);
            return (
                <window>
                    <text>{'Soon is ' + soon}</text>
                    <button key="soon" height={30} onClick={() => setTimeout(() => setSoon(true), 0)}>Soon</button>
                </window>
            );
        }
        "#,
    );
    let started = std::time::Instant::now();
    let out = works(&file, "click #soon; expect \"Soon is true\"");
    assert!(!out.contains("still going"), "{out}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "it took {:?}",
        started.elapsed()
    );
}

#[test]
fn something_far_down_is_scrolled_to_before_it_is_clicked() {
    let file = app(
        "far",
        r#"
        import { useState } from 'rsui';
        export const size = [300, 200];
        export default function App() {
            const [count, setCount] = useState(0);
            return (
                <window>
                    <text>{'Count is ' + count}</text>
                    <scroll key="area" flex={1}>
                        {Array.from({ length: 60 }, (_, at) => (
                            <button key={'row' + at} height={30} onClick={() => setCount(at)}>{'Row ' + at}</button>
                        ))}
                    </scroll>
                </window>
            );
        }
        "#,
    );
    works(&file, "click #row50; expect \"Count is 50\"");
}

#[test]
fn keys_reach_what_has_the_keyboard() {
    let file = app(
        "keys",
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [said, setSaid] = useState('nothing');
            return (
                <window>
                    <input key="box" width={200} height={30} value="" onChange={() => {}}
                        onKeyPress={(key) => { setSaid(key); return true; }} />
                    <text>{'Pressed ' + said}</text>
                </window>
            );
        }
        "#,
    );
    works(
        &file,
        "press enter #box; expect \"Pressed enter\"; press ctrl+s; expect \"Pressed ctrl+s\"",
    );
}

#[test]
fn a_picture_of_the_first_frame_still_works_the_way_it_did() {
    let file = app("png", COUNTER);
    let picture = file.with_file_name("out.png");
    let output = Command::new(TOOL)
        .arg("--png")
        .arg(&picture)
        .arg(&file)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", complained(&output));
    assert!(said(&output).contains("Count is 0"));
    assert!(picture.exists());
}

/// One of the examples, with its files somewhere of its own.
fn example(name: &str, setting: &str, place: &PathBuf, steps: &str) -> String {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("apps")
        .join(name);
    let output = Command::new(TOOL)
        .env(setting, place)
        .arg("--do")
        .arg(steps)
        .arg(file)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{steps}\n{}\n{}",
        said(&output),
        complained(&output)
    );
    said(&output)
}

fn scratch(name: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("drive-saved-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

#[test]
fn closing_tasks_straight_after_a_change_keeps_the_change() {
    let place = scratch("tasks").join("tasks.json");
    let setting = "RSUI_TASKS_FILE";
    example(
        "tasks.tsx",
        setting,
        &place,
        "type #draft \"quick one\"; click #add; close",
    );
    example("tasks.tsx", setting, &place, "expect \"quick one\"");
}

#[test]
fn closing_notes_straight_after_typing_keeps_what_was_typed() {
    let place = scratch("notes");
    let setting = "RSUI_NOTES_FOLDER";
    example(
        "notes.tsx",
        setting,
        &place,
        "type #editor \"typed then closed\"; close",
    );
    example("notes.tsx", setting, &place, "expect \"typed then closed\"");
}

#[test]
fn closing_expenses_straight_after_adding_one_keeps_it() {
    let place = scratch("expenses").join("expenses.json");
    let setting = "RSUI_EXPENSES_FILE";
    example(
        "expenses.tsx",
        setting,
        &place,
        "type #description \"quick lunch\"; type #amount 12.50; click #add; close",
    );
    example("expenses.tsx", setting, &place, "expect \"quick lunch\"");
}

#[test]
fn the_desktop_widgets_example_asks_for_its_layers_and_bar_without_a_real_desktop() {
    // Steps never make real windows, so this only says what the clock and the bar would ask of the desktop.
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("apps").join("desktop_widgets.tsx");
    let said = works(
        &file,
        "expect \"Clock: on, desktop layer, transparent\"; click #layer-bottom; click #bar-toggle; expect \"App bar: on the top edge\"; click #bar-edge; click #see-through; click #bar-toggle; expect \"App bar: off\"",
    );
    assert!(said.contains("Layer(Bottom)"), "{said}");
    assert!(said.contains("edge: Right, size: 140"), "{said}");
    assert!(said.contains("Transparent(false)"), "{said}");
}

#[test]
fn what_is_asked_of_a_window_is_said_and_not_done() {
    let file = app(
        "window-changes",
        r#"
        import { mainWindow } from 'rsui';
        export default function App() {
            return <window><button key="go" onClick={() => mainWindow.setFullscreen()}>Go</button></window>;
        }
        "#,
    );
    let said = works(&file, "click #go");
    assert!(said.contains("Fullscreen(true)"), "{said}");
}

#[test]
fn a_socket_with_nothing_to_say_does_not_hold_up_the_steps() {
    let file = app(
        "idle-socket",
        r#"
        import { useState } from 'rsui';
        import { createServer, connect } from 'net';
        export default function App() {
            const [said, setSaid] = useState('waiting');
            useState(() => {
                const server = createServer((socket) => socket.on('data', (text) => { socket.write('heard ' + text); }));
                server.listen(0, '127.0.0.1', () => {
                    const client = connect(server.address().port, '127.0.0.1', () => setSaid('connected'));
                    globalThis.client = client;
                    client.on('data', (text) => setSaid(String(text)));
                });
            });
            return (
                <window>
                    <text>{said}</text>
                    <button key="say" onClick={() => globalThis.client.write('hello')}>Say</button>
                </window>
            );
        }
        "#,
    );
    let started = std::time::Instant::now();
    let said = works(
        &file,
        "wait \"connected\"; click #say; wait \"heard hello\"; look; look",
    );
    // A step's given a second to settle, so waiting for what's listening would take a good deal longer than this.
    assert!(
        !said.contains("something's still going"),
        "{said} after {:?}",
        started.elapsed()
    );
}

const WHY: &str = r#"
    import { useState, useRef, useContext, createContext, useCallback } from 'rsui';
    const Theme = createContext('light');
    function useTodos() {
        const [items, setItems] = useState(['milk']);
        const add = useCallback((item: string) => setItems((old) => [...old, item]), []);
        return { items, add };
    }
    function Row({ label, onPick }: { label: string; onPick: () => void }) {
        const theme = useContext(Theme);
        return <button key={label} onClick={onPick}>{label + ' ' + theme}</button>;
    }
    export default function App() {
        const [count, setCount] = useState(0);
        const [theme, setTheme] = useState('light');
        const picked = useRef(0);
        const { items, add } = useTodos();
        return (
            <Theme.Provider value={theme}>
                <window>
                    <text>{'Count is ' + count}</text>
                    <button key="add" onClick={() => setCount(count + 1)}>Add</button>
                    <button key="theme" onClick={() => setTheme('dark')}>Dark</button>
                    <button key="todo" onClick={() => add('eggs')}>Todo</button>
                    {items.map((item) => <Row key={item} label={item} onPick={() => { picked.current++; }} />)}
                </window>
            </Theme.Provider>
        );
    }
"#;

#[test]
fn why_says_what_changed_where_and_names_hooks_by_their_variables() {
    let file = app("why", WHY);
    let out = works(&file, "click #add; frames; why #milk");
    assert!(
        out.contains("click on #add ran `onClick` at app.tsx:22"),
        "the handler and its line: {out}"
    );
    assert!(
        out.contains("`count` was set from 0 to 1 at app.tsx:22, in onClick."),
        "the state, by name: {out}"
    );
    assert!(
        out.contains("`onPick` is a new function each time. App makes it at app.tsx:25. useCallback would keep the same one."),
        "the prop that's new each time, and where it's made: {out}"
    );
    assert!(
        out.contains("Row (") && out.contains("changed nothing"),
        "a render that changed nothing: {out}"
    );
    assert!(
        out.contains("theme = 'light'  (context, app.tsx:10)"),
        "a hook by its name: {out}"
    );
    assert!(out.contains("It's in the app › App › Provider."), "{out}");
}

#[test]
fn state_set_inside_a_hook_of_the_apps_own_says_both() {
    let file = app("why-hook", WHY);
    let out = works(&file, "click #todo; frames");
    assert!(
        out.contains(
            "`useTodos → items` was set from [ 'milk' ] to [ 'milk', 'eggs' ] at app.tsx:6."
        ),
        "{out}"
    );
}

#[test]
fn what_reads_a_context_renders_once_when_it_changes() {
    let file = app("why-context", WHY);
    let out = works(&file, "frames; click #theme; frames");
    let clicked = out.split("> click #theme").nth(1).unwrap();
    let rows = clicked.matches("    Row (").count();
    assert_eq!(
        rows, 1,
        "the row renders once, with its new props, rather than again for the context: {out}"
    );
    assert!(
        out.contains("The context it reads as `theme` changed."),
        "{out}"
    );
}

#[test]
fn inspecting_can_be_turned_off_and_says_so() {
    let file = app("why-off", WHY);
    let (_, why) = fails(&file, "inspect off; click #add; why #milk");
    assert!(why.contains("inspect on"), "{why}");
    works(&file, "inspect off; inspect on; click #add; why #milk");
}

#[test]
fn state_can_go_back_to_any_change_and_forward_again() {
    let file = app("history", WHY);
    let out = works(
        &file,
        "click #add; click #add; click #theme; history; goto 1; expect \"Count is 1\"; expect \"milk light\"; goto 3; expect \"Count is 2\"; expect \"milk dark\"; goto 0; expect \"Count is 0\"",
    );
    assert!(
        out.contains("1. App: `count` was set from 0 to 1 at app.tsx:22, in onClick."),
        "{out}"
    );
    assert!(
        out.contains("3. App: `theme` was set from 'light' to 'dark'"),
        "{out}"
    );
    assert!(
        out.contains("it's as it was after change 1, with 2 hooks put back"),
        "{out}"
    );
    // Changing something after going back carries on from there.
    let out = works(
        &file,
        "click #add; click #add; goto 1; click #theme; history",
    );
    assert!(out.contains("2. App: `theme` was set"), "{out}");
    assert!(!out.contains("3. App"), "{out}");
}

#[test]
fn a_logpoint_says_what_it_finds_each_time_its_line_runs_and_keeps_the_state() {
    let file = app("logpoint", WHY);
    let out = works(
        &file,
        "click #add; logpoint app.tsx:11 \"label + ' is ' + theme\"; click #theme; logpoints; logpoint off app.tsx:11; click #add",
    );
    assert!(
        out.contains("logpoint app.tsx:11: label + ' is ' + theme = 'milk is light'"),
        "{out}"
    );
    assert!(
        out.contains("logpoint app.tsx:11: label + ' is ' + theme = 'milk is dark'"),
        "{out}"
    );
    assert!(
        out.contains("app.tsx:11 label + ' is ' + theme (ran 2 times)"),
        "{out}"
    );
    assert!(
        out.contains("Count is 2"),
        "adding it kept the state: {out}"
    );
    assert_eq!(
        out.matches("logpoint app.tsx:11:").count(),
        2,
        "and taking it away stops it: {out}"
    );
    let (_, why) = fails(&file, "logpoint app.tsx:21 \"count\"");
    assert!(
        why.contains("doesn't start a statement"),
        "a line inside JSX has no room for one: {why}"
    );
}

#[test]
fn profile_says_what_took_the_time_and_why_renders_were_for_nothing() {
    let file = app("profile", WHY);
    let out = works(&file, "click #add; click #add; profile");
    assert!(out.contains("Slowest frames:"), "{out}");
    assert!(
        out.contains("Row: ") && out.contains("changing nothing"),
        "{out}"
    );
    assert!(
        out.contains("2× Row: `onPick` is a new function each time. App makes it at app.tsx:25."),
        "the reason, counted: {out}"
    );
}

#[test]
fn a_saved_trace_opens_in_the_viewer_the_same_way() {
    let file = app("trace", WHY);
    let trace = file.with_file_name("saved.rsuitrace");
    let out = works(
        &file,
        &format!(
            "click #add; save \"{}\"",
            trace.display().to_string().replace('\\', "/")
        ),
    );
    assert!(out.contains("saved in"), "{out}");
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&trace).unwrap()).unwrap();
    assert_eq!(saved["kind"], "rsui trace");
    assert!(
        saved["history"][0]["said"]
            .as_str()
            .unwrap()
            .contains("`count` was set from 0 to 1")
    );

    let viewed = Command::new(TOOL)
        .arg("--view")
        .arg(&trace)
        .arg("--do")
        .arg("expect \"click on #add ran\"; click \"Rendering\"; expect \"Row\"; click \"State\"; expect \"was set from 0 to 1\"")
        .output()
        .unwrap();
    assert!(
        viewed.status.success(),
        "{}\n{}",
        said(&viewed),
        complained(&viewed)
    );

    let not_one = file.with_file_name("not.rsuitrace");
    std::fs::write(&not_one, "{}").unwrap();
    let refused = Command::new(TOOL)
        .arg("--view")
        .arg(&not_one)
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(
        complained(&refused).contains("isn't a trace saved by rsui"),
        "{}",
        complained(&refused)
    );
}
#[test]
fn a_page_element_is_found_by_its_id_and_a_path_is_typed_as_it_is_written() {
    let file = app(
        "page-id",
        r#"
        import 'rsui/web';
        import { useState } from 'react';
        export default function App() {
            const [said, setSaid] = useState('nothing');
            const [path, setPath] = useState('');
            return (
                <div>
                    <button id="save" onClick={() => setSaid('saved')}>Save</button>
                    <input key="path" placeholder="repository path" value={path} onChange={(e) => setPath(e.target.value)} />
                    <p>{said + ' ' + path}</p>
                </div>
            );
        }
        "#,
    );
    let out = works(
        &file,
        r#"click #save; type "repository path" "C:\Users\me\new\tasks"; expect "saved C:\\Users\\me\\new\\tasks""#,
    );
    assert!(out.contains("saved"), "{out}");
}
#[test]
fn a_transition_that_starts_while_a_step_settles_finishes_while_waiting() {
    // Mantine's Transition shows a tooltip at opacity 0, sets 1 a frame later and settles 100 ms after that, all inside the step
    // that hovers. What's moving said when it next looked different from when it last moved on, which only waking moves on,
    // so `wait` never woke it, and the tooltip stayed invisible.
    let file = app(
        "fade-wakes",
        r#"
        import 'rsui/web';
        import { useEffect, useState } from 'react';
        function Fade() {
            const [state, setState] = useState('out');
            useEffect(() => {
                requestAnimationFrame(() => {
                    setState('entering');
                    setTimeout(() => setState('entered'), 100);
                });
            }, []);
            return <div style={{ transition: 'opacity 100ms', opacity: state === 'out' ? 0 : 1 }}>{'Faded ' + state}</div>;
        }
        export default function App() {
            const [shown, setShown] = useState(false);
            return (
                <div>
                    <button id="show" onClick={() => setShown(true)}>Show</button>
                    {shown ? <Fade /> : null}
                </div>
            );
        }
        "#,
    );
    let out = works(&file, r#"animations on; click #show; expect "Faded entered"; wait 400"#);
    let after = out.split("> click #show").nth(1).unwrap_or_default();
    assert!(
        after.contains("only how things look changed"),
        "it faded in while waiting: {out}"
    );
}

#[test]
fn props_a_component_is_first_seen_with_as_a_block_redraw_nothing_else() {
    // Each swatch of Mantine's ColorPicker gave a component props it hadn't been seen with, and finding out that it makes a block
    // for them drew the whole kitchen sink again, most of a second each time.
    let file = app(
        "flow-learned",
        r#"
        import 'rsui/web';
        import { useState } from 'react';
        // Seen making a line's worth and a block, so that what it makes is learned for each set of props.
        function Swatch({ color, inline }) {
            return inline ? <span>{'inline ' + color}</span> : <div>{'swatch ' + color}</div>;
        }
        export default function App() {
            const [colors, setColors] = useState(['red']);
            return (
                <div>
                    <p>Unrelated</p>
                    <Swatch color="red" inline />
                    {colors.map((color) => <Swatch key={color} color={color} />)}
                    <button id="add" onClick={() => setColors((was) => [...was, 'c' + was.length])}>Add</button>
                </div>
            );
        }
        "#,
    );
    let out = works(&file, r#"click #add; click #add; click #add; expect "swatch c3"; why "Unrelated""#);
    let why = out.split("> why").nth(1).unwrap_or_default();
    let times = why
        .split(" rendered ")
        .nth(1)
        .map(|rest| rest.split(" in the last").next().unwrap_or_default().to_owned())
        .unwrap_or_default();
    assert!(
        times == "once" || times == "2 times",
        "what's beside them isn't drawn again for each: {times:?}\n{out}"
    );
}

/// Driving a page by CSS selectors and reading its state with code run in it: `clickat css:SELECTOR`, `eval`,
/// `evalto`, `waitjs`, `wheel`, `settle`, and `shots` for frames, and `downat`/`upat` with no place go on from the pointer.
#[test]
fn a_page_can_be_driven_by_selectors_and_read_with_code() {
    let file = app(
        "selectors",
        r#"
        import { PageWindow } from 'rsui/web';
        import { useState } from 'react';
        export const size = [400, 300];
        export default function App() {
            const [said, setSaid] = useState('nothing');
            return (
                <PageWindow>
                    <div style={{ height: 120 }}>top</div>
                    <button className="go" onClick={() => setSaid('clicked')} onPointerDown={() => setSaid('down')}>Go</button>
                    <p id="said">{said}</p>
                    <div style={{ height: 1000 }}>tall</div>
                </PageWindow>
            );
        }
        "#,
    );
    let folder = file.parent().unwrap().to_path_buf();
    let state = folder.join("state.txt");
    let frames = folder.join("frames");
    let steps = format!(
        "clickat \"css:button.go\"; expect \"clicked\"; eval \"document.getElementById('said').textContent\"; \
         evalto \"{}\" \"document.getElementById('said').textContent + ' ' + innerWidth\"; \
         waitjs \"document.getElementById('said').textContent === 'clicked'\" 500; \
         wheel 10 10 0 100; waitjs \"scrollY > 50\" 2000; eval \"'at ' + scrollY\"; wheel 10 10 0 -100; \
         moveat \"css:button.go\"; downat; expect \"down\"; upat; \
         settle 0; clickat \"css:button.go\"; settle default; shots \"{}\" 300 16",
        state.display().to_string().replace('\\', "/"),
        frames.display().to_string().replace('\\', "/"),
    );
    let out = works(&file, &steps);
    assert!(out.contains("  = clicked"), "{out}");
    // Three rows, a notch, down the page.
    let scrolled: f64 = out.split("  = at ").nth(1).and_then(|rest| rest.lines().next()).and_then(|said| said.trim().parse().ok()).unwrap_or(0.0);
    assert!(scrolled > 50.0, "the wheel scrolls the page: {out}");
    assert_eq!(std::fs::read_to_string(&state).unwrap(), "clicked 400");
    let taken = std::fs::read_dir(&frames).unwrap().filter_map(Result::ok).filter(|one| one.file_name().to_string_lossy().starts_with("frame-")).count();
    assert!(taken >= 2, "pictures every 16 ms for 300 ms, however long one takes to draw: {taken}\n{out}");
    // What a selector doesn't find says so.
    let (_, why) = fails(&file, "clickat \"css:#nothing\"");
    assert!(why.contains("nothing in the page matches \"#nothing\""), "{why}");
}

/// A package's `#name` imports, as Node's subpath imports (vfile's `#minpath`, @base-ui/react's `#prehydration/...`): an exact
/// name and a `*` pattern, conditions read as `exports`' are (`node` only when nothing else fits, `browser`, `import` and
/// `require`), a target that's another package, and the app's own `package.json` imports.
#[test]
fn package_imports_with_a_hash_are_found_by_the_nearest_package_json() {
    let file = app(
        "hash-imports",
        r#"
        import React from 'react';
        import { said } from 'uses-hash';
        import local from '#local';
        export default function App() {
            return <div>{'Said ' + said + ' ' + local}</div>;
        }
        "#,
    );
    let folder = file.parent().unwrap();
    let write = |path: PathBuf, text: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        folder.join("package.json"),
        r##"{ "name": "app", "imports": { "#local": "./local.js" } }"##,
    );
    write(folder.join("local.js"), "module.exports = 'local';");
    let package = folder.join("node_modules").join("uses-hash");
    write(
        package.join("package.json"),
        r##"{
            "name": "uses-hash",
            "main": "./index.js",
            "imports": {
                "#path": { "node": "./lib/path.node.js", "default": "./lib/path.browser.js" },
                "#parts/*": { "browser": { "import": "./parts/*.mjs", "require": "./parts/*.cjs" }, "default": "./parts/none.js" },
                "#parts/special/*": "./parts/special-*.js",
                "#other": "other-package/inner",
                "#only-node": { "node": "./lib/node-only.js" }
            }
        }"##,
    );
    write(
        package.join("index.js"),
        "const path = require('#path'); const thumb = require('#parts/thumb'); const special = require('#parts/special/x'); \
         const other = require('#other'); const nodeOnly = require('#only-node'); \
         exports.said = [path, thumb, special, other, nodeOnly].join(' ');",
    );
    write(package.join("lib").join("path.node.js"), "module.exports = 'node-path';");
    write(package.join("lib").join("path.browser.js"), "module.exports = 'browser-path';");
    write(package.join("lib").join("node-only.js"), "module.exports = 'node-only';");
    write(package.join("parts").join("thumb.cjs"), "module.exports = 'thumb-cjs';");
    write(package.join("parts").join("thumb.mjs"), "export default 'thumb-mjs';");
    write(package.join("parts").join("none.js"), "module.exports = 'none';");
    write(package.join("parts").join("special-x.js"), "module.exports = 'special-x';");
    let other = package.join("node_modules").join("other-package");
    write(other.join("package.json"), r#"{ "name": "other-package" }"#);
    write(other.join("inner.js"), "module.exports = 'other-inner';");
    // rsui-tsx finds packages as a browser bundler does, so `browser` applies, and `require` within it.
    works(&file, "expect \"Said browser-path thumb-cjs special-x other-inner node-only local\"");
}
