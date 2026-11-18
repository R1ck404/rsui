//! Components, state and effects on top of the `rsui-core` tree.
//!
//! A component is a function from props to an element. When its state changes, only
//! that component renders again, and the runtime emits patches for its part of the
//! tree. A child whose props didn't change keeps its node without rendering, so
//! nothing here depends on the size of the rest of the UI.
//!
//! There are no windows or pixels here. A host calls [`Runtime::dispatch`] for each
//! action, by a person or an agent, then [`Runtime::flush`] to get the patches.
//!
//! ```
//! use rsui_core::{ActionKind, Element, Patch, Role, snapshot_json, text};
//! use rsui_runtime::{Cx, Runtime};
//!
//! fn counter(cx: &mut Cx, label: &&'static str) -> Element {
//!     let count = cx.state(|| 0);
//!     let name = text!("{label}: {}", cx.get(count));
//!     let button = Element::new(Role::Button).key("add").name(name);
//!     cx.on(button, ActionKind::Click, move |cx, _| {
//!         cx.update(count, |count| *count += 1);
//!     })
//! }
//!
//! let mut runtime = Runtime::new(counter, "Count");
//! runtime.dispatch(&[], ActionKind::Click, None).unwrap();
//! let patches = runtime.flush();
//!
//! assert!(matches!(patches.as_slice(), [Patch::SetName { .. }]));
//! assert_eq!(
//!     snapshot_json(runtime.root()),
//!     r#"{"key":"add","role":"button","name":"Count: 1","actions":["click"]}"#,
//! );
//! ```

mod cx;
mod event;
mod list;
mod motion;
mod remote;
mod runtime;
mod timer;
mod trace;

pub use cx::{Cx, PreviousNode, PreviousNodes, State, Updates};
pub use event::{
    Dispatching, Event, KeyInfo, Modifiers, current_tree, dispatch_started, dispatching, document_listens, listens,
    next_event_serial, set_document_listens, set_wants_drops, wants_drops,
};
pub use list::{ListOptions, ListWindow, ScrollAxis, ScrollControl};
pub use remote::Remote;
pub use runtime::{DispatchError, Runtime};
pub use trace::{Cause, Handled, Rendered, Trace};
