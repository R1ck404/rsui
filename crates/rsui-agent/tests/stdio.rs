//! Runs the example app as a real subprocess, the way an MCP client does, and operates
//! it end to end over its standard streams.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Client {
    fn start() -> Self {
        // Cargo builds examples before running tests, next to the test's own folder.
        let tests = std::env::current_exe().unwrap();
        let name = format!("todo{}", std::env::consts::EXE_SUFFIX);
        let program = tests
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples")
            .join(name);
        let mut child = Command::new(&program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|error| panic!("can't start {}: {error}", program.display()));
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 0,
        }
    }

    fn request(&mut self, method: &str, mut params: Value) -> Value {
        self.next_id += 1;
        params["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "1" },
            "io.modelcontextprotocol/clientCapabilities": {},
        });
        let message =
            json!({ "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params });
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();

        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], self.next_id, "{response}");
        response
    }

    fn tool(&mut self, name: &str, arguments: Value) -> String {
        let response = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        let result = &response["result"];
        let text = result["content"][0]["text"].as_str().unwrap().to_owned();
        assert_eq!(result["isError"], false, "{text}");
        text
    }

    fn set_draft_and_add(&mut self, label: &str) -> String {
        self.tool(
            "ui_act",
            json!({ "path": "0", "action": "set_value", "input": label }),
        );
        self.tool("ui_act", json!({ "path": "1", "action": "click" }))
    }
}

#[test]
fn an_agent_operates_a_real_process_from_start_to_exit() {
    let mut client = Client::start();

    let discovered = client.request("server/discover", json!({}));
    assert_eq!(
        discovered["result"]["supportedVersions"],
        json!(["2026-07-28"])
    );
    let tools = client.request("tools/list", json!({}));
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 4);

    let empty = client.tool("ui_snapshot", json!({}));
    assert!(empty.contains("window \"Todos (0)\""), "{empty}");

    client.set_draft_and_add("Buy milk");
    client.set_draft_and_add("Walk the dog");

    let found = client.tool("ui_find", json!({ "name_contains": "dog" }));
    assert!(found.contains("checkbox \"Walk the dog\""), "{found}");
    assert!(found.contains("@3/1/0"), "{found}");

    client.tool("ui_act", json!({ "path": "3/1/0", "action": "toggle" }));
    let list = client.tool("ui_snapshot", json!({ "path": "3/1" }));
    assert!(list.contains("checked=true"), "{list}");

    // Deleting the first row moves the second into its place, keeping its state.
    client.tool("ui_act", json!({ "path": "3/0/1", "action": "click" }));
    let list = client.tool("ui_snapshot", json!({}));
    assert!(list.contains("window \"Todos (1)\""), "{list}");
    assert!(!list.contains("Buy milk"), "{list}");
    assert!(list.contains("checked=true"), "{list}");

    // The server finishes this background load before it answers.
    let loaded = client.tool("ui_act", json!({ "path": "2", "action": "click" }));
    assert!(loaded.contains(", idle\n"), "{loaded}");
    let list = client.tool("ui_snapshot", json!({}));
    assert!(list.contains("Water the plants"), "{list}");
    assert!(list.contains("Load samples"), "{list}");

    // Closing stdin is how a client asks a server to exit.
    drop(client.stdin.take());
    let start = Instant::now();
    loop {
        if let Some(status) = client.child.try_wait().unwrap() {
            assert!(status.success(), "{status}");
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the server didn't exit"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn an_agent_gets_a_useful_error_from_the_app() {
    let mut client = Client::start();
    let response = client.request(
        "tools/call",
        json!({ "name": "ui_act", "arguments": { "path": "1", "action": "click" } }),
    );
    assert_eq!(response["result"]["isError"], true);
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("type something first"), "{text}");
}
