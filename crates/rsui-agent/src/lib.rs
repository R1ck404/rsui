//! Lets agents read and operate a rsui app.
//!
//! A [`Session`] exposes an app as tools over the Model Context Protocol, so any
//! MCP client can use it. Every app has these:
//!
//! - `ui_snapshot` reads the tree as a compact outline.
//! - `ui_find` looks for nodes.
//! - `ui_act` carries out an action on a node, and reports what changed.
//! - `ui_wait` waits for background work to change the tree.
//!
//! An app in TSX has more, as `rsui-tsx --agent app.tsx`: `ui_screenshot`, `ui_key`,
//! `ui_close`, `ui_restart` and `ui_next_dialog`, and each tool can be given a window.
//! Anything that can do what [`Host`] asks can be operated this way.
//!
//! Agents can only do what the UI offers. An action must be one the node declares,
//! and it runs the same handler a person's action would. What a person couldn't use
//! at the moment, because it's disabled or a modal window is in the way, is refused.
//!
//! Each change to the tree makes a new version. `ui_act` can take the version the agent
//! last read, and refuses to run if the tree has moved on, since paths may then point
//! at different nodes.
//!
//! Text in the tree comes from the app, and possibly from the data it shows, so agents
//! should treat it as data rather than instructions. The outline always quotes it.
//!
//! ```
//! use rsui_agent::Session;
//! use rsui_core::{Element, Role};
//! use rsui_headless::Headless;
//! use rsui_runtime::Cx;
//!
//! fn app(_: &mut Cx, _: &()) -> Element {
//!     Element::new(Role::Text).name("Hello")
//! }
//!
//! let mut session = Session::new(Headless::new(app, ()), "hello");
//! let request = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
//!     "_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28",
//!              "io.modelcontextprotocol/clientCapabilities":{}},
//!     "name":"ui_snapshot","arguments":{}}}"#;
//! let response = session.handle_message(&request.replace('\n', " ")).unwrap();
//! assert!(response.contains(r#"text \"Hello\" @/"#));
//! ```
//!
//! To serve an app to a client that launches it as a subprocess, call
//! [`Session::serve_stdio`] from `main`. Nothing else may write to stdout.

mod host;
mod protocol;
mod session;

pub use host::{Host, WindowInfo};
pub use session::Session;
