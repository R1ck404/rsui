//! Serving an app with `--agent`, as a real subprocess, the way a client that's been told about it starts it.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const TOOL: &str = env!("CARGO_BIN_EXE_rsui-tsx");

fn app(name: &str, source: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("agent-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("app.tsx");
    std::fs::write(&file, source).unwrap();
    file
}

struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next: i64,
}

impl Client {
    fn start(file: &PathBuf) -> Self {
        let mut child = Command::new(TOOL)
            .arg("--agent")
            .arg(file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut client = Self {
            child,
            stdin,
            stdout,
            next: 0,
        };
        client.request("initialize", json!({ "protocolVersion": "2025-11-25" }));
        client
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        // Anything else on this stream would break a real client, as it would this.
        let reply: Value =
            serde_json::from_str(&line).unwrap_or_else(|_| panic!("that isn't JSON: {line:?}"));
        assert_eq!(reply["id"], id, "{reply}");
        reply
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let reply = self.request(
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        );
        reply["result"].clone()
    }

    /// What a tool said, which it should have without any trouble.
    fn ok(&mut self, tool: &str, arguments: Value) -> String {
        let result = self.call(tool, arguments.clone());
        let text = result["content"][0]["text"].as_str().unwrap().to_owned();
        assert_eq!(result["isError"], false, "{tool} {arguments}: {text}");
        text
    }

    /// Why a tool couldn't do that.
    fn refused(&mut self, tool: &str, arguments: Value) -> String {
        let result = self.call(tool, arguments.clone());
        let text = result["content"][0]["text"].as_str().unwrap().to_owned();
        assert_eq!(result["isError"], true, "{tool} {arguments}: {text}");
        text
    }

    /// The path of what has this key, in this window, from what an agent can read.
    fn path_of(&mut self, window: u32, key: &str) -> String {
        let found = self.ok("ui_find", json!({ "key": key, "window": window }));
        let line = found
            .lines()
            .find(|line| line.contains(&format!("#{key}")))
            .unwrap_or_else(|| panic!("no {key} in {found}"));
        line.rsplit('@').next().unwrap().to_owned()
    }

    fn click(&mut self, window: u32, key: &str) -> String {
        let path = self.path_of(window, key);
        self.ok(
            "ui_act",
            json!({ "path": path, "action": "click", "window": window }),
        )
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const COUNTER: &str = r##"
    import { useState } from 'rsui';
    export const title = 'Counter';
    export const size = [320, 200];
    export default function App() {
        const [count, setCount] = useState(0);
        const [name, setName] = useState('');
        return (
            <window background="#ffffff" color="#000000">
                <text>{'Count is ' + count}</text>
                <button key="add" height={30} onClick={() => { console.log('adding, now ' + (count + 1)); setCount(count + 1); }}>Add</button>
                <button key="off" height={30} disabled onClick={() => setCount(100)}>Off</button>
                <input key="name" width={200} height={30} value={name} onChange={setName} />
                <input key="fixed" width={200} height={30} value="fixed" readOnly onChange={setName} />
                <text>{'Name is ' + name}</text>
            </window>
        );
    }
"##;

#[test]
fn all_the_tools_are_there_and_say_what_they_take() {
    let file = app("tools", COUNTER);
    let mut client = Client::start(&file);
    let reply = client.request("tools/list", json!({}));
    let tools = reply["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "ui_snapshot",
            "ui_find",
            "ui_act",
            "ui_wait",
            "ui_screenshot",
            "ui_key",
            "ui_close",
            "ui_restart",
            "ui_why",
            "ui_logpoint",
            "ui_history",
            "ui_goto",
            "ui_profile",
            "ui_frames",
            "ui_next_dialog",
        ]
    );
    for tool in tools {
        assert!(tool["description"].as_str().unwrap().len() > 20, "{tool}");
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
    }
    assert!(tools[0]["inputSchema"]["properties"]["window"].is_object());
}

#[test]
fn an_agent_reads_acts_and_is_told_what_changed_and_what_was_logged() {
    let file = app("act", COUNTER);
    let mut client = Client::start(&file);
    let first = client.ok("ui_snapshot", json!({}));
    assert!(first.starts_with("version 0\n"), "{first}");
    assert!(first.contains("Count is 0"), "{first}");
    let told = client.click(0, "add");
    assert!(told.starts_with("version 1, idle"), "{told}");
    assert!(told.contains("Count is 1"), "{told}");
    assert!(told.contains("console \"adding, now 1\""), "{told}");
    let path = client.path_of(0, "name");
    let told = client.ok(
        "ui_act",
        json!({ "path": path, "action": "set_value", "input": "Ann" }),
    );
    assert!(told.contains("Name is Ann"), "{told}");
    let stale = client.refused(
        "ui_act",
        json!({ "path": path, "action": "set_value", "input": "Bo", "version": 1 }),
    );
    assert!(stale.contains("stale"), "{stale}");
}

#[test]
fn what_a_person_could_not_do_is_refused() {
    let file = app("refused", COUNTER);
    let mut client = Client::start(&file);
    let path = client.path_of(0, "off");
    let why = client.refused("ui_act", json!({ "path": path, "action": "click" }));
    assert!(why.contains("can't be used"), "{why}");
    let path = client.path_of(0, "fixed");
    let why = client.refused(
        "ui_act",
        json!({ "path": path, "action": "set_value", "input": "changed" }),
    );
    assert!(why.contains("can't be used"), "{why}");
    let text = client.ok("ui_snapshot", json!({}));
    assert!(text.contains("Count is 0"), "{text}");
    assert!(!text.contains("changed"), "{text}");
}

#[test]
fn set_value_is_always_given_text_as_someone_could_only_type_that() {
    let file = app(
        "typed",
        r#"
        import { useState } from 'rsui';
        export default function App() {
            const [got, setGot] = useState('nothing');
            return (
                <window>
                    <input key="box" width={200} height={30} value="" onChange={(value) => setGot(typeof value + ' ' + value.trim())} />
                    <text>{'Got ' + got}</text>
                </window>
            );
        }
        "#,
    );
    let mut client = Client::start(&file);
    let why = client.refused("ui_act", json!({ "key": "box", "action": "set_value" }));
    assert!(why.contains("needs an input"), "{why}");
    assert!(
        client.ok("ui_snapshot", json!({})).contains("Got nothing"),
        "the app didn't crash"
    );
    let told = client.ok(
        "ui_act",
        json!({ "key": "box", "action": "set_value", "input": 12 }),
    );
    assert!(told.contains("Got string 12"), "{told}");
    let told = client.ok(
        "ui_act",
        json!({ "key": "box", "action": "set_value", "input": true }),
    );
    assert!(told.contains("Got string true"), "{told}");
    let told = client.ok(
        "ui_act",
        json!({ "key": "box", "action": "set_value", "input": "" }),
    );
    assert!(told.contains("Got string "), "{told}");
}

#[test]
fn a_node_can_be_used_by_its_key_and_only_how_it_looks_changing_is_a_count() {
    let file = app(
        "keyed",
        r##"
        import { useState } from 'rsui';
        export default function App() {
            const [on, setOn] = useState(false);
            return <window onKeyPress={(key) => { setOn(key); return true; }}><input key="box" width={200} height={30} value="" onChange={() => {}} /><button key="go" color={on ? '#ff0000' : '#00ff00'} onClick={() => setOn(!on)}>Go</button></window>;
        }
        "##,
    );
    let mut client = Client::start(&file);
    let told = client.ok("ui_act", json!({ "key": "go", "action": "click" }));
    assert!(told.starts_with("version 1"), "{told}");
    assert!(told.contains("changes to how things look"), "{told}");
    assert!(!told.contains("text_color"), "{told}");
    client.ok("ui_key", json!({ "key": "tab", "on_key": "box" }));
    let why = client.refused("ui_key", json!({ "key": "tab", "on_key": "nope" }));
    assert!(why.contains("nope"), "{why}");
}

#[test]
fn nothing_the_app_writes_can_break_the_connection() {
    let file = app(
        "stdout",
        r#"
        export default function App() {
            console.log('from the log');
            process.stdout.write('from process.stdout\n');
            process.stderr.write('from process.stderr\n');
            return <window><button key="more" height={30} onClick={() => process.stdout.write('{"not":"yours"}\n')}>More</button></window>;
        }
        "#,
    );
    let mut client = Client::start(&file);
    let text = client.ok("ui_snapshot", json!({}));
    assert!(text.contains("console \"from the log\""), "{text}");
    client.click(0, "more");
    client.ok("ui_snapshot", json!({}));
}

/// The bytes in a tool's image.
fn from_base64(text: &str) -> Vec<u8> {
    let value = |byte: u8| match byte {
        b'A'..=b'Z' => u32::from(byte - b'A'),
        b'a'..=b'z' => u32::from(byte - b'a') + 26,
        b'0'..=b'9' => u32::from(byte - b'0') + 52,
        b'+' => 62,
        _ => 63,
    };
    let mut out = Vec::new();
    for four in text.as_bytes().chunks(4) {
        let all = four.iter().fold(0, |all, byte| {
            all << 6 | if *byte == b'=' { 0 } else { value(*byte) }
        });
        let bytes = all.to_be_bytes();
        let keep = four.iter().filter(|byte| **byte != b'=').count() - 1;
        out.extend_from_slice(&bytes[1..1 + keep]);
    }
    out
}

#[test]
fn a_screenshot_is_a_real_small_picture_of_what_is_there() {
    let file = app("shot", COUNTER);
    let mut client = Client::start(&file);
    let result = client.call("ui_screenshot", json!({}));
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["content"][1]["type"], "image");
    assert_eq!(result["content"][1]["mimeType"], "image/png");
    let bytes = from_base64(result["content"][1]["data"].as_str().unwrap());
    assert!(bytes.len() < 60_000, "it's {} bytes", bytes.len());
    let picture = rsui_image::decode(&bytes, (320, 200)).unwrap();
    assert_eq!((picture.width, picture.height), (320, 200));
    let pixel = |x: usize, y: usize| &picture.pixels[(y * 320 + x) * 4..(y * 320 + x) * 4 + 4];
    assert_eq!(
        pixel(319, 199),
        [255, 255, 255, 255],
        "the background it asked for"
    );
    let dark = picture.pixels.chunks(4).filter(|one| one[0] < 80).count();
    assert!(dark > 50, "and there's text on it, with {dark} dark pixels");
}

const WINDOWS: &str = r#"
    import { useState, Window } from 'rsui';
    export const title = 'Main';
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
fn every_window_can_be_read_and_used_and_is_told_apart() {
    let file = app("windows", WINDOWS);
    let mut client = Client::start(&file);
    let told = client.click(0, "open");
    assert!(told.contains("window 1 \"Settings\" opened"), "{told}");
    let list = client.ok("ui_snapshot", json!({}));
    assert!(list.contains("window 0 \"Main\""), "{list}");
    assert!(list.contains("window 1 \"Settings\""), "{list}");
    let there = client.ok("ui_snapshot", json!({ "window": 1 }));
    assert!(there.contains("Add to 0"), "{there}");
    // Looking for something looks everywhere, and says where it found it.
    let found = client.ok("ui_find", json!({ "key": "add" }));
    assert!(found.contains("window 1: button"), "{found}");
    let told = client.click(1, "add");
    assert!(
        told.contains("Main says 1"),
        "changes reach the main one: {told}"
    );
    assert!(told.contains("window 1:"), "and say where: {told}");
    let shot = client.call("ui_screenshot", json!({ "window": 1 }));
    let bytes = from_base64(shot["content"][1]["data"].as_str().unwrap());
    let picture = rsui_image::decode(&bytes, (300, 200)).unwrap();
    assert_eq!((picture.width, picture.height), (300, 200));
    let told = client.ok("ui_close", json!({ "window": 1 }));
    assert!(told.contains("\"Settings\" closed"), "{told}");
    let why = client.refused("ui_snapshot", json!({ "window": 1 }));
    assert!(why.contains("no window 1"), "{why}");
}

#[test]
fn a_modal_window_keeps_the_main_one_from_being_used() {
    let file = app("modal", WINDOWS);
    let mut client = Client::start(&file);
    let open = client.path_of(0, "open");
    client.click(0, "ask");
    let list = client.ok("ui_snapshot", json!({}));
    assert!(list.contains("waiting for window"), "{list}");
    let why = client.refused("ui_act", json!({ "path": open, "action": "click" }));
    assert!(why.contains("modal"), "{why}");
    let why = client.refused("ui_key", json!({ "key": "tab" }));
    assert!(why.contains("modal"), "{why}");
    // It can still be read, as a person can see it.
    assert!(client.ok("ui_snapshot", json!({})).contains("Main says 0"));
    client.click(1, "yes");
    client.ok("ui_act", json!({ "path": open, "action": "click" }));
}

#[test]
fn saving_a_file_changes_what_is_running_by_the_next_call_and_keeps_its_state() {
    let file = app("reload", COUNTER);
    let mut client = Client::start(&file);
    client.click(0, "add");
    std::fs::write(&file, COUNTER.replace("Count is ", "Now at ")).unwrap();
    let text = client.ok("ui_snapshot", json!({}));
    assert!(text.contains("Now at 1"), "{text}");
    assert!(text.contains("loaded again"), "{text}");
    // A mistake shows, and what's running carries on until there's a fix.
    std::fs::write(&file, "export default function App( {").unwrap();
    let text = client.ok("ui_snapshot", json!({}));
    assert!(
        text.contains("Now at 1") || text.to_lowercase().contains("error"),
        "{text}"
    );
    std::fs::write(&file, COUNTER.replace("Count is ", "Fixed at ")).unwrap();
    let text = client.ok("ui_snapshot", json!({}));
    assert!(text.contains("Fixed at 1"), "{text}");
}

#[test]
fn saving_a_file_leaves_other_windows_open_and_still_updating() {
    let source = r#"
        import { useState, Window } from 'rsui';
        export default function App() {
            const [count, setCount] = useState(0);
            return (
                <window>
                    <button key="open" height={30} onClick={() => setCount(count + 1)}>Add</button>
                    <Window title="Other" size={[200, 100]}>
                        <text key="shown">{'Before says ' + count}</text>
                        <button key="there" height={30} onClick={() => setCount(count + 10)}>There</button>
                    </Window>
                </window>
            );
        }
    "#;
    let file = app("reload-windows", source);
    let mut client = Client::start(&file);
    client.click(0, "open");
    assert!(
        client
            .ok("ui_snapshot", json!({ "window": 1 }))
            .contains("Before says 1")
    );
    std::fs::write(&file, source.replace("Before says", "After says")).unwrap();
    let text = client.ok("ui_snapshot", json!({ "window": 1 }));
    assert!(
        text.contains("After says 1"),
        "the same window, with the new code: {text}"
    );
    assert!(!text.contains("closed"), "{text}");
    let told = client.click(0, "open");
    assert!(
        told.contains("After says 2"),
        "and it still hears about changes: {told}"
    );
    let told = client.click(1, "there");
    assert!(
        told.contains("After says 12"),
        "and it can still be used: {told}"
    );
}

#[test]
fn a_file_another_one_imports_counts_too() {
    let file = app(
        "imports",
        r#"
        import { LABEL } from './label';
        export default function App() { return <window><text>{LABEL}</text></window>; }
        "#,
    );
    let other = file.with_file_name("label.ts");
    std::fs::write(&other, "export const LABEL = 'first';").unwrap();
    let mut client = Client::start(&file);
    assert!(client.ok("ui_snapshot", json!({})).contains("first"));
    std::fs::write(&other, "export const LABEL = 'second';").unwrap();
    let text = client.ok("ui_snapshot", json!({}));
    assert!(text.contains("second"), "{text}");
}

#[test]
fn an_app_that_will_not_start_says_why_until_a_fix_is_saved() {
    let file = app("broken", "export default function App( {");
    let mut client = Client::start(&file);
    let why = client.refused("ui_snapshot", json!({}));
    assert!(why.contains("couldn't start"), "{why}");
    assert!(client.request("tools/list", json!({}))["result"]["tools"].is_array());
    std::fs::write(&file, COUNTER).unwrap();
    let text = client.ok("ui_snapshot", json!({}));
    assert!(text.contains("Count is 0"), "{text}");
}

#[test]
fn restarting_forgets_everything_and_timers_still_work_afterwards() {
    let source = r#"
        import { useEffect, useState } from 'rsui';
        export default function App() {
            const [count, setCount] = useState(0);
            const [done, setDone] = useState(false);
            useEffect(() => { const timer = setTimeout(() => setDone(true), 1500); return () => clearTimeout(timer); }, []);
            return (
                <window>
                    <text>{'Count is ' + count}</text>
                    <text>{done ? 'Finished' : 'Working'}</text>
                    <button key="add" height={30} onClick={() => setCount(count + 1)}>Add</button>
                </window>
            );
        }
    "#;
    let file = app("restart", source);
    let mut client = Client::start(&file);
    client.click(0, "add");
    assert!(client.ok("ui_snapshot", json!({})).contains("Count is 1"));
    let told = client.ok("ui_restart", json!({}));
    assert!(told.contains("the app started"), "{told}");
    let text = client.ok("ui_snapshot", json!({}));
    assert!(text.contains("Count is 0"), "{text}");
    assert!(text.contains("Working"), "{text}");
    let version: u64 = text
        .lines()
        .next()
        .unwrap()
        .trim_start_matches("version ")
        .parse()
        .unwrap();
    // The timer going off is what ends this.
    let told = client.ok("ui_wait", json!({ "version": version, "timeout_ms": 8000 }));
    assert!(told.contains("Finished"), "{told}");
    client.click(0, "add");
    assert!(client.ok("ui_snapshot", json!({})).contains("Count is 1"));
}

#[test]
fn closing_the_main_window_ends_the_app_until_it_is_restarted() {
    let file = app("close", COUNTER);
    let mut client = Client::start(&file);
    let told = client.ok("ui_close", json!({}));
    assert!(told.contains("the app closed"), "{told}");
    let why = client.refused("ui_snapshot", json!({}));
    assert!(why.contains("ui_restart"), "{why}");
    client.ok("ui_restart", json!({}));
    assert!(client.ok("ui_snapshot", json!({})).contains("Count is 0"));
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
                <window onKeyPress={(key) => { setSaid(key); return true; }}>
                    <input key="box" width={200} height={30} value="" onChange={() => {}} />
                    <text>{'Pressed ' + said}</text>
                </window>
            );
        }
        "#,
    );
    let mut client = Client::start(&file);
    let path = client.path_of(0, "box");
    let told = client.ok("ui_key", json!({ "key": "ctrl+s", "path": path }));
    assert!(told.contains("Pressed ctrl+s"), "{told}");
    let why = client.refused("ui_key", json!({ "key": "banana" }));
    assert!(why.contains("isn't a key"), "{why}");
}

#[test]
fn no_real_dialog_comes_up_and_the_answer_can_be_given_first() {
    let file = app(
        "dialog",
        r#"
        import { useState } from 'rsui';
        import { dialogs } from 'rsui/desktop';
        export default function App() {
            const [got, setGot] = useState('nothing yet');
            return (
                <window>
                    <button key="open" height={30} onClick={async () => setGot(String(await dialogs.open({ title: 'Pick' })))}>Open</button>
                    <text>{'Got ' + got}</text>
                </window>
            );
        }
        "#,
    );
    let mut client = Client::start(&file);
    let told = client.click(0, "open");
    assert!(told.contains("Got null"), "{told}");
    assert!(told.contains("a dialog was asked for"), "{told}");
    client.ok("ui_next_dialog", json!({ "path": "C:/some/file.txt" }));
    let told = client.click(0, "open");
    assert!(told.contains("Got C:/some/file.txt"), "{told}");
}

#[test]
fn a_client_going_away_ends_it() {
    let file = app("ends", COUNTER);
    let mut client = Client::start(&file);
    drop(client.stdin.take());
    let started = Instant::now();
    loop {
        if let Some(status) = client.child.try_wait().unwrap() {
            assert!(status.success(), "{status}");
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "it never ended"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn an_agent_can_ask_why_something_rendered_without_turning_anything_on() {
    let file = app(
        "why",
        r#"
        import { useState } from 'rsui';
        function Row({ label, onPick }: { label: string; onPick: () => void }) {
            return <button key={label} onClick={onPick}>{label}</button>;
        }
        export default function App() {
            const [count, setCount] = useState(0);
            return (
                <window>
                    <button key="add" onClick={() => setCount(count + 1)}>Add</button>
                    <Row label="milk" onPick={() => {}} />
                </window>
            );
        }
        "#,
    );
    let mut client = Client::start(&file);
    client.ok("ui_act", json!({ "key": "add", "action": "click" }));
    let frames = client.ok("ui_frames", json!({}));
    assert!(frames.contains("click on #add"), "{frames}");
    assert!(
        frames.contains("`count` was set from 0 to 1 at app.tsx:10"),
        "{frames}"
    );
    let why = client.ok("ui_why", json!({ "key": "milk" }));
    assert!(
        why.contains("`onPick` is a new function each time. App makes it at app.tsx:11"),
        "{why}"
    );
    assert!(why.contains("changing nothing"), "{why}");
    let again = client.ok("ui_frames", json!({}));
    assert!(!again.contains("click on #add"), "only what's new: {again}");
}

#[test]
fn an_agent_can_go_back_through_state_and_log_a_line_without_stopping_it() {
    let file = app("history", COUNTER);
    let mut client = Client::start(&file);
    client.ok("ui_act", json!({ "key": "add", "action": "click" }));
    client.ok("ui_act", json!({ "key": "add", "action": "click" }));
    let history = client.ok("ui_history", json!({}));
    assert!(
        history.contains("2. App: `count` was set from 1 to 2"),
        "{history}"
    );
    let back = client.ok("ui_goto", json!({ "change": 1 }));
    assert!(back.contains("after change 1"), "{back}");
    assert!(client.ok("ui_snapshot", json!({})).contains("Count is 1"));
    let logged = client.ok(
        "ui_logpoint",
        json!({ "at": "app.tsx:8", "expression": "count * 10" }),
    );
    assert!(
        logged.contains("count * 10 = 10"),
        "it runs, with the state it had: {logged}"
    );
    let removed = client.ok("ui_logpoint", json!({ "at": "all" }));
    assert!(removed.contains("1 taken away"), "{removed}");
}
