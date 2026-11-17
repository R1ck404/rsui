//! The value layer of rsui: an immutable element tree, a keyed diff between two
//! trees, and deterministic snapshots of a tree.
//!
//! Nothing here knows about windows, pixels or agents. A renderer consumes
//! [`Patch`]es, and an agent reads a [`snapshot`]; both see the same tree.
//!
//! Subtrees are shared through [`Node`]. When two trees hold the same `Arc`,
//! [`diff`] skips that subtree without looking inside it, so reusing unchanged
//! subtrees makes an update cost proportional to what changed.
//!
//! ```
//! use rsui_core::{ActionKind, Element, Patch, Role, apply, diff, snapshot_json};
//!
//! let save = |name: &str| {
//!     Element::new(Role::Button).key("save").name(name).action(ActionKind::Click)
//! };
//! let old = Element::new(Role::Group).child(save("Save")).into_node();
//! let new = Element::new(Role::Group).child(save("Saved")).into_node();
//!
//! let patches = diff(&old, &new);
//! assert!(matches!(patches.as_slice(), [Patch::SetName { .. }]));
//! assert_eq!(apply(old, &patches).unwrap(), new);
//! assert_eq!(
//!     snapshot_json(&new),
//!     r#"{"role":"group","children":[{"key":"save","role":"button","name":"Saved","actions":["click"]}]}"#,
//! );
//! ```

mod diff;
mod element;
mod patch;
mod pool;
mod snapshot;
pub mod stack;
mod symbol;

pub use diff::{diff, diff_at};
pub use element::{
    ActionKind, Actions, CustomRole, Element, GENERATED_KEYS_FROM, IntoNode, Key, Node, PropValue,
    Props, Role,
};
pub use patch::{ApplyError, Patch, Path, apply};
pub use pool::NodePool;
pub use snapshot::{snapshot, snapshot_into, snapshot_json};

/// A cheap-to-clone string in 16 bytes. Up to 15 bytes live inline, without allocating.
pub use ecow::EcoString as Text;
/// Builds a [`Text`] like `format!`, without allocating if the result fits inline.
pub use ecow::eco_format as text;
pub use symbol::Symbol;
