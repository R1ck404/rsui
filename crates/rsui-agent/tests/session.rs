//! The tools and the protocol, through the messages a client would send.

use std::time::Duration;

use rsui_agent::Session;
use rsui_core::{ActionKind, Element, Node, PropValue, Role, Symbol, text};
use rsui_headless::Headless;
use rsui_runtime::{Cx, ListWindow};
use serde_json::{Value, json};

const MODERN: &str = "2026-07-28";

fn meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": MODERN,
        "io.modelcontextprotocol/clientCapabilities": {},
    })
}

fn send(session: &mut Session, message: Value) -> Value {
    let text = session
        .handle_message(&message.to_string())
        .expect("a request gets a response");
    assert!(!text.contains('\n'), "a message must fit on one line");
    serde_json::from_str(&text).expect("the response is JSON")
}

fn request(session: &mut Session, method: &str, mut params: Value) -> Value {
    params["_meta"] = meta();
    send(
        session,
        json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params }),
    )
}

/// Calls a tool, and returns its text and whether it reported an error.
fn tool(session: &mut Session, name: &str, arguments: Value) -> (String, bool) {
    let response = request(
        session,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    );
    assert_eq!(response["id"], 7);
    let result = &response["result"];
    assert_eq!(result["resultType"], "complete", "{response}");
    (
        result["content"][0]["text"].as_str().unwrap().to_owned(),
        result["isError"].as_bool().unwrap(),
    )
}

fn ok(session: &mut Session, name: &str, arguments: Value) -> String {
    let (text, is_error) = tool(session, name, arguments);
    assert!(!is_error, "{text}");
    text
}

fn failed(session: &mut Session, name: &str, arguments: Value) -> String {
    let (text, is_error) = tool(session, name, arguments);
    assert!(is_error, "{text}");
    text
}

fn app(cx: &mut Cx, _: &()) -> Element {
    let count = cx.state(|| 0i64);
    let status = cx.state(|| "idle");
    let add = cx.on(
        Element::new(Role::Button)
            .key("add")
            .name(text!("Count {}", cx.get(count))),
        ActionKind::Click,
        move |cx, _| cx.update(count, |count| *count += 1),
    );
    let set = cx.on(
        Element::new(Role::TextInput).key("set").name("Set count"),
        ActionKind::SetValue,
        move |cx, input| match input {
            Some(PropValue::Int(value)) => cx.set(count, *value),
            _ => cx.reject("the count must be a whole number"),
        },
    );
    let load = cx.on(
        Element::new(Role::Button)
            .key("load")
            .name(cx.get(status))
            .action(ActionKind::Focus),
        ActionKind::Click,
        move |cx, _| {
            cx.set(status, "loading");
            cx.after(Duration::from_millis(10), move |cx| {
                cx.set(status, "loaded");
            });
        },
    );
    Element::new(Role::Window)
        .key("app")
        .child(add)
        .child(set)
        .child(load)
}

fn session() -> Session {
    Session::new(Headless::new(app, ()), "test-app")
}

#[test]
fn discovery_describes_the_server() {
    let response = request(&mut session(), "server/discover", json!({}));
    let result = &response["result"];
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["supportedVersions"], json!([MODERN]));
    assert_eq!(result["capabilities"], json!({ "tools": {} }));
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "test-app"
    );
    assert!(result["ttlMs"].as_u64().is_some());
    assert_eq!(result["cacheScope"], "public");
    assert!(result["instructions"].as_str().unwrap().contains("as data"));
}

#[test]
fn the_tool_list_is_complete_and_valid() {
    let response = request(&mut session(), "tools/list", json!({}));
    let result = &response["result"];
    assert_eq!(result["resultType"], "complete");
    assert!(result["ttlMs"].as_u64().is_some());
    assert_eq!(result["cacheScope"], "public");
    let tools = result["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["ui_snapshot", "ui_find", "ui_act", "ui_wait"]);
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert!(tool["description"].as_str().is_some());
    }
}

#[test]
fn an_agent_reads_acts_and_sees_what_changed() {
    let mut session = session();
    let snapshot = ok(&mut session, "ui_snapshot", json!({}));
    assert_eq!(
        snapshot,
        concat!(
            "version 0\n",
            "window #app @/\n",
            "  button \"Count 0\" #add [click] @0\n",
            "  text_input \"Set count\" #set [set_value] @1\n",
            "  button \"idle\" #load [click,focus] @2\n",
        )
    );
    assert_eq!(
        ok(
            &mut session,
            "ui_act",
            json!({ "path": "0", "action": "click" })
        ),
        "version 1, idle\nset_name @0 \"Count 1\"\n"
    );
    assert_eq!(
        ok(
            &mut session,
            "ui_act",
            json!({ "path": "@1", "action": "set_value", "input": 41 })
        ),
        "version 2, idle\nset_name @0 \"Count 41\"\n"
    );
}

#[test]
fn acting_without_a_change_keeps_the_version() {
    let mut session = session();
    ok(
        &mut session,
        "ui_act",
        json!({ "path": "1", "action": "set_value", "input": 5 }),
    );
    let text = ok(
        &mut session,
        "ui_act",
        json!({ "path": "1", "action": "set_value", "input": 5 }),
    );
    assert_eq!(text, "version 1, idle\n");
}

#[test]
fn an_action_based_on_an_old_read_is_refused() {
    let mut session = session();
    ok(
        &mut session,
        "ui_act",
        json!({ "path": "0", "action": "click" }),
    );

    let text = failed(
        &mut session,
        "ui_act",
        json!({ "path": "0", "action": "click", "version": 0 }),
    );
    assert!(
        text.starts_with("stale: you read version 0, but the UI is at version 1"),
        "{text}"
    );
    assert!(
        ok(&mut session, "ui_snapshot", json!({})).contains("Count 1"),
        "nothing changed"
    );

    ok(
        &mut session,
        "ui_act",
        json!({ "path": "0", "action": "click", "version": 1 }),
    );
}

#[test]
fn background_work_finishes_within_an_action() {
    let mut session = session();
    let text = ok(
        &mut session,
        "ui_act",
        json!({ "path": "2", "action": "click" }),
    );
    assert_eq!(
        text,
        "version 1, idle\nset_name @2 \"loading\"\nset_name @2 \"loaded\"\n"
    );
}

#[test]
fn an_action_reports_when_background_work_outlasts_its_timeout() {
    let mut session = session();
    let text = ok(
        &mut session,
        "ui_act",
        json!({ "path": "2", "action": "click", "timeout_ms": 0 }),
    );
    assert_eq!(text, "version 1, still working\nset_name @2 \"loading\"\n");

    let text = ok(
        &mut session,
        "ui_wait",
        json!({ "version": 1, "timeout_ms": 10_000 }),
    );
    assert_eq!(text, "version 2, idle\nset_name @2 \"loaded\"\n");
}

/// Long enough for a 10 ms timer to have fired and queued its change.
fn let_background_work_finish() {
    std::thread::sleep(Duration::from_millis(200));
}

#[test]
fn a_change_from_background_work_makes_an_old_read_stale() {
    let mut session = session();
    ok(
        &mut session,
        "ui_act",
        json!({ "path": "2", "action": "click", "timeout_ms": 0 }),
    );
    let_background_work_finish();

    // Version 1 was current when the agent read it, but the timer has changed the UI
    // since, even though nothing has asked the server to apply that yet.
    let text = failed(
        &mut session,
        "ui_act",
        json!({ "path": "0", "action": "click", "version": 1 }),
    );
    assert!(text.contains("UI is at version 2"), "{text}");
}

#[test]
fn a_snapshot_makes_earlier_changes_old_news() {
    let mut session = session();
    ok(
        &mut session,
        "ui_act",
        json!({ "path": "2", "action": "click", "timeout_ms": 0 }),
    );
    let_background_work_finish();
    let snapshot = ok(&mut session, "ui_snapshot", json!({}));
    assert!(snapshot.contains("loaded"), "{snapshot}");

    let text = ok(
        &mut session,
        "ui_act",
        json!({ "path": "0", "action": "click" }),
    );
    assert_eq!(text, "version 3, idle\nset_name @0 \"Count 1\"\n");
}

/// Reports quickly, while other work carries on for much longer.
fn partly_slow(cx: &mut Cx, _: &()) -> Element {
    let status = cx.state(|| "started");
    cx.effect((), move |cx| {
        cx.after(Duration::from_millis(5), move |cx| {
            cx.set(status, "partly done")
        });
        cx.after(Duration::from_secs(20), |_| {});
    });
    Element::new(Role::Text).name(cx.get(status))
}

#[test]
fn waiting_returns_at_the_first_change_even_if_work_remains() {
    let mut session = Session::new(Headless::new(partly_slow, ()), "slow");
    let start = std::time::Instant::now();
    let text = ok(
        &mut session,
        "ui_wait",
        json!({ "version": 0, "timeout_ms": 15_000 }),
    );
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "{:?}",
        start.elapsed()
    );
    assert_eq!(
        text,
        "version 1, still working\nset_name @/ \"partly done\"\n"
    );
}

#[test]
fn waiting_with_nothing_running_returns_at_once() {
    let mut session = session();
    let start = std::time::Instant::now();
    let text = ok(
        &mut session,
        "ui_wait",
        json!({ "version": 0, "timeout_ms": 30_000 }),
    );
    assert_eq!(text, "version 0, idle\n");
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn waiting_for_a_version_already_passed_returns_the_news() {
    let mut session = session();
    ok(
        &mut session,
        "ui_act",
        json!({ "path": "0", "action": "click" }),
    );
    assert_eq!(
        ok(&mut session, "ui_wait", json!({ "version": 0 })),
        "version 1, idle\n"
    );
}

#[test]
fn bad_actions_explain_themselves_and_change_nothing() {
    let mut session = session();
    let mut check = |arguments: Value, expected: &str| {
        let text = failed(&mut session, "ui_act", arguments);
        assert!(text.contains(expected), "{text} should contain {expected}");
    };
    check(json!({ "path": "9", "action": "click" }), "no node exists");
    check(json!({ "path": "x", "action": "click" }), "isn't a path");
    check(json!({ "action": "click" }), "say which node");
    check(json!({ "path": "0" }), "action argument is required");
    check(json!({ "path": 0, "action": "click" }), "must be a string");
    check(
        json!({ "path": "0", "action": "explode" }),
        "no action is called",
    );
    check(
        json!({ "path": "0", "action": "scroll" }),
        "button \"Count 0\" #add [click] @0",
    );
    check(
        json!({ "path": "2", "action": "focus" }),
        "nothing handles that action",
    );
    check(
        json!({ "path": "1", "action": "set_value", "input": "lots" }),
        "must be a whole number",
    );
    check(
        json!({ "path": "1", "action": "set_value", "input": [1] }),
        "must be a string, number or boolean",
    );
    check(
        json!({ "path": "0", "action": "click", "version": -1 }),
        "whole number",
    );
    assert!(ok(&mut session, "ui_snapshot", json!({})).starts_with("version 0"));
}

#[test]
fn made_up_action_names_are_not_stored() {
    let mut session = session();
    let name = "a-name-nobody-has-ever-used";
    failed(
        &mut session,
        "ui_act",
        json!({ "path": "0", "action": name }),
    );
    assert_eq!(Symbol::existing(name), None);
    ok(
        &mut session,
        "ui_find",
        json!({ "action": name, "role": name, "key": name }),
    );
    assert_eq!(Symbol::existing(name), None);
}

#[test]
fn finding_nodes_returns_their_lines() {
    let mut session = session();
    assert_eq!(
        ok(
            &mut session,
            "ui_find",
            json!({ "action": "click", "name_contains": "COUNT" })
        ),
        "version 0\nbutton \"Count 0\" #add [click] @0\n"
    );
    assert_eq!(
        ok(&mut session, "ui_find", json!({ "role": "button" }))
            .lines()
            .count(),
        3
    );
    assert_eq!(
        ok(&mut session, "ui_find", json!({ "key": "nope" })),
        "version 0\nno node matches key nope\n"
    );
    let text = failed(&mut session, "ui_find", json!({}));
    assert!(text.contains("at least one condition"), "{text}");
}

#[test]
fn a_snapshot_can_be_limited_to_part_of_the_tree() {
    let mut session = session();
    assert_eq!(
        ok(&mut session, "ui_snapshot", json!({ "path": "1" })),
        "version 0\ntext_input \"Set count\" #set [set_value] @1\n"
    );
    assert_eq!(
        ok(&mut session, "ui_snapshot", json!({ "depth": 0 })),
        "version 0\nwindow #app @/\n  (+3 children)\n"
    );
    let text = failed(&mut session, "ui_snapshot", json!({ "path": "5/1" }));
    assert_eq!(text, "no node at path 5/1");
}

fn many(cx: &mut Cx, _: &()) -> Element {
    let rows = cx.state(|| 1_000u32);
    let items: Vec<Node> = (0..cx.get(rows))
        .map(|index| {
            Element::new(Role::ListItem)
                .key(index as usize)
                .name(text!("row {index}"))
                .into_node()
        })
        .collect();
    cx.on(
        Element::new(Role::List).key("list").children(items),
        ActionKind::Click,
        move |cx, _| cx.set(rows, 0),
    )
}

#[test]
fn a_huge_change_is_summarized() {
    let mut session = Session::new(Headless::new(many, ()), "many");
    let text = ok(
        &mut session,
        "ui_act",
        json!({ "path": "/", "action": "click" }),
    );
    assert!(text.len() < 4_200, "{} bytes", text.len());
    assert!(
        text.contains("more changes; read them with ui_snapshot"),
        "{text}"
    );
    assert_eq!(
        ok(&mut session, "ui_snapshot", json!({})),
        "version 1\nlist #list [click] @/\n"
    );
}

#[test]
fn a_huge_match_list_is_summarized() {
    let mut session = Session::new(Headless::new(many, ()), "many");
    let text = ok(&mut session, "ui_find", json!({ "role": "list_item" }));
    assert_eq!(text.lines().count(), 52);
    assert!(text.ends_with("(+950 more)\n"), "{text}");
}

fn long_list(cx: &mut Cx, _: &()) -> Element {
    cx.virtual_list(
        "rows",
        ListWindow {
            len: 1_000_000,
            visible: 2,
            overscan: 0,
            reveal: None,
        },
        |cx, index| {
            cx.keyed(
                index,
                |_, index: &usize| Element::new(Role::ListItem).name(text!("row {index}")),
                index,
            )
        },
    )
}

#[test]
fn an_agent_can_scroll_a_virtual_list() {
    let mut session = Session::new(Headless::new(long_list, ()), "long");
    assert_eq!(
        ok(&mut session, "ui_snapshot", json!({})),
        concat!(
            "version 0\n",
            "list #rows count=1000000 first_visible=0 [scroll,resize] @/\n",
            "  list_item \"row 0\" #0 @0\n",
            "  list_item \"row 1\" #1 @1\n",
        )
    );
    ok(
        &mut session,
        "ui_act",
        json!({ "path": "/", "action": "scroll", "input": 500_000 }),
    );
    let text = ok(&mut session, "ui_snapshot", json!({}));
    assert!(text.contains("first_visible=500000"), "{text}");
    assert!(text.contains("row 500001"), "{text}");
}

#[test]
fn an_unsupported_version_lists_what_is_supported() {
    let response = send(
        &mut session(),
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/list",
            "params": { "_meta": {
                "io.modelcontextprotocol/protocolVersion": "1900-01-01",
                "io.modelcontextprotocol/clientCapabilities": {},
            } }
        }),
    );
    assert_eq!(response["id"], 1);
    assert_eq!(response["error"]["code"], -32022);
    assert_eq!(response["error"]["message"], "Unsupported protocol version");
    assert_eq!(response["error"]["data"]["supported"], json!([MODERN]));
    assert_eq!(response["error"]["data"]["requested"], "1900-01-01");
}

#[test]
fn a_request_missing_required_metadata_is_rejected() {
    let code = |session: &mut Session, params: Value| {
        send(
            session,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": params }),
        )["error"]["code"]
            .clone()
    };
    let mut session = session();
    assert_eq!(code(&mut session, json!({})), -32602);
    assert_eq!(
        code(
            &mut session,
            json!({ "_meta": { "io.modelcontextprotocol/protocolVersion": MODERN } })
        ),
        -32602
    );
    let response = send(
        &mut session,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    );
    let message = response["error"]["message"].as_str().unwrap();
    assert!(
        message.contains(MODERN) && message.contains("2025-11-25"),
        "{message}"
    );
}

#[test]
fn a_legacy_client_can_use_the_handshake() {
    let mut session = session();
    let response = send(
        &mut session,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "old", "version": "1" },
            }
        }),
    );
    let result = &response["result"];
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert_eq!(result["capabilities"], json!({ "tools": {} }));
    assert_eq!(result["serverInfo"]["name"], "test-app");

    assert_eq!(
        session.handle_message(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
        None
    );
    let response = send(
        &mut session,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }),
    );
    assert_eq!(response["result"], json!({}));

    let response = send(
        &mut session,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }),
    );
    assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 4);
    assert!(response["result"].get("resultType").is_none());

    let response = send(
        &mut session,
        json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "ui_act", "arguments": { "path": "0", "action": "click" } }
        }),
    );
    assert_eq!(response["result"]["isError"], false);
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Count 1")
    );
}

#[test]
fn a_legacy_client_asking_for_an_unknown_version_gets_the_newest_legacy_one() {
    let response = send(
        &mut session(),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2020-01-01" } }),
    );
    assert_eq!(response["result"]["protocolVersion"], "2025-11-25");
}

#[test]
fn malformed_messages_get_the_standard_errors() {
    let mut session = session();
    let code = |session: &mut Session, text: &str| -> Value {
        let response: Value =
            serde_json::from_str(&session.handle_message(text).expect("an error response"))
                .unwrap();
        response["error"]["code"].clone()
    };
    assert_eq!(code(&mut session, "{not json"), -32700);
    assert_eq!(code(&mut session, "[1, 2]"), -32600);
    assert_eq!(code(&mut session, r#"{"jsonrpc":"2.0","id":1}"#), -32600);
    assert_eq!(
        code(&mut session, r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#),
        -32600
    );
    assert_eq!(
        code(
            &mut session,
            r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#
        ),
        -32600
    );
    assert_eq!(
        code(
            &mut session,
            r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":[]}"#
        ),
        -32602
    );

    let response = request(&mut session, "no/such/method", json!({}));
    assert_eq!(response["error"]["code"], -32601);
    let response = request(
        &mut session,
        "tools/call",
        json!({ "name": "no_such_tool" }),
    );
    assert_eq!(response["error"]["code"], -32602);
    assert_eq!(response["error"]["message"], "Unknown tool: no_such_tool");
    let response = request(&mut session, "tools/call", json!({}));
    assert_eq!(response["error"]["code"], -32602);
    let response = request(
        &mut session,
        "tools/call",
        json!({ "name": "ui_find", "arguments": 1 }),
    );
    assert_eq!(response["error"]["code"], -32602);
}

#[test]
fn notifications_and_stray_responses_get_no_reply() {
    let mut session = session();
    for message in [
        r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#,
        r#"{"jsonrpc":"2.0","id":5,"result":{}}"#,
        r#"{"jsonrpc":"2.0","id":5,"error":{"code":1,"message":"no"}}"#,
    ] {
        assert_eq!(session.handle_message(message), None, "{message}");
    }
}

#[test]
fn serving_answers_each_line_and_skips_blank_ones() {
    let input = concat!(
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\n\n",
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#,
        "\n",
    );
    let mut output = Vec::new();
    session().serve(input.as_bytes(), &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    let ids: Vec<i64> = text
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["id"]
                .as_i64()
                .unwrap()
        })
        .collect();
    assert_eq!(ids, [1, 2]);
    assert!(text.ends_with('\n'));
}

#[test]
fn a_node_can_be_acted_on_by_its_key() {
    let mut session = session();
    let text = ok(&mut session, "ui_snapshot", json!({}));
    let key = text
        .lines()
        .find(|line| line.contains("[click]") && line.contains('#'))
        .and_then(|line| line.split('#').nth(1))
        .and_then(|rest| rest.split(' ').next())
        .expect("something can be clicked")
        .to_owned();
    let told = ok(
        &mut session,
        "ui_act",
        json!({ "key": key, "action": "click" }),
    );
    assert!(told.starts_with("version 1"), "{told}");
    let why = failed(
        &mut session,
        "ui_act",
        json!({ "key": "not-there", "action": "click" }),
    );
    assert!(why.contains("not-there"), "{why}");
    let why = failed(
        &mut session,
        "ui_act",
        json!({ "key": key, "path": "/", "action": "click" }),
    );
    assert!(why.contains("not both"), "{why}");
    let why = failed(&mut session, "ui_act", json!({ "action": "click" }));
    assert!(why.contains("which node"), "{why}");
}

#[test]
fn changes_to_how_things_look_are_only_counted() {
    fn looks(cx: &mut Cx, _: &()) -> Element {
        let on = cx.state(|| false);
        let color = if cx.get(on) { 1 } else { 2 };
        let button = Element::new(Role::Button)
            .key("go")
            .prop(Symbol::new("text_color"), PropValue::Int(color));
        cx.on(button, ActionKind::Click, move |cx, _| {
            cx.update(on, |on| *on = !*on)
        })
    }
    let mut session = Session::new(Headless::new(looks, ()), "looks");
    let told = ok(
        &mut session,
        "ui_act",
        json!({ "key": "go", "action": "click" }),
    );
    assert!(told.starts_with("version 1"), "it's still a change: {told}");
    assert!(
        told.contains("(and 1 changes to how things look)"),
        "{told}"
    );
    assert!(!told.contains("text_color"), "{told}");
}
