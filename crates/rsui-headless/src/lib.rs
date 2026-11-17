//! Runs a rsui app with no window, for tests and for agents.
//!
//! ```
//! use rsui_core::{ActionKind, Element, Role, text};
//! use rsui_headless::{Headless, Query};
//! use rsui_runtime::Cx;
//!
//! fn counter(cx: &mut Cx, _: &()) -> Element {
//!     let count = cx.state(|| 0);
//!     let name = text!("Clicked {} times", cx.get(count));
//!     let button = Element::new(Role::Button).key("add").name(name);
//!     cx.on(button, ActionKind::Click, move |cx, _| {
//!         cx.update(count, |count| *count += 1);
//!     })
//! }
//!
//! let mut ui = Headless::new(counter, ());
//! ui.click(&Query::new().role(Role::Button)).unwrap();
//! assert_eq!(ui.outline(), "button \"Clicked 1 times\" #add [click] @/\n");
//! ```

mod outline;
mod query;

use std::fmt;
use std::time::Duration;

use rsui_core::{ActionKind, IntoNode, Node, Patch, PropValue};
use rsui_runtime::{Cx, DispatchError, Runtime};

pub use outline::{
    describe_patch, format_path, only_changes_styling, outline, outline_at, parse_path,
};
pub use query::{FindError, Found, Query};

/// Why an action on a query didn't happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActError {
    /// The query didn't identify one node.
    Find(FindError),
    /// The node couldn't carry out the action.
    Dispatch(DispatchError),
}

impl fmt::Display for ActError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Find(error) => error.fmt(f),
            Self::Dispatch(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ActError {}

impl From<FindError> for ActError {
    fn from(error: FindError) -> Self {
        Self::Find(error)
    }
}

impl From<DispatchError> for ActError {
    fn from(error: DispatchError) -> Self {
        Self::Dispatch(error)
    }
}

/// A mounted app with no window.
pub struct Headless {
    runtime: Runtime,
}

impl Headless {
    pub fn new<P: 'static, R: IntoNode + 'static>(
        render: fn(&mut Cx<'_>, &P) -> R,
        props: P,
    ) -> Self {
        Self {
            runtime: Runtime::new(render, props),
        }
    }

    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut Runtime {
        &mut self.runtime
    }

    pub fn root(&self) -> &Node {
        self.runtime.root()
    }

    /// The tree as compact text, with one line per node. See [`outline`].
    pub fn outline(&self) -> String {
        outline(self.root())
    }

    /// Finds the one node matching `query`.
    pub fn find(&self, query: &Query) -> Result<Found, FindError> {
        query.find_one(self.root(), &[])
    }

    /// Finds every node matching `query`, in tree order.
    pub fn find_all(&self, query: &Query) -> Vec<Found> {
        query.find_all(self.root(), &[])
    }

    /// Carries out an action on the one node matching `query`, and returns what changed.
    pub fn act(
        &mut self,
        query: &Query,
        action: ActionKind,
        input: Option<PropValue>,
    ) -> Result<Vec<Patch>, ActError> {
        let found = self.find(query)?;
        self.runtime.dispatch(&found.path, action, input)?;
        Ok(self.runtime.flush())
    }

    pub fn click(&mut self, query: &Query) -> Result<Vec<Patch>, ActError> {
        self.act(query, ActionKind::Click, None)
    }

    pub fn toggle(&mut self, query: &Query) -> Result<Vec<Patch>, ActError> {
        self.act(query, ActionKind::Toggle, None)
    }

    pub fn set_value(
        &mut self,
        query: &Query,
        value: impl Into<PropValue>,
    ) -> Result<Vec<Patch>, ActError> {
        self.act(query, ActionKind::SetValue, Some(value.into()))
    }

    /// Waits for background work to finish, and returns what changed and whether it
    /// all finished in time.
    pub fn settle(&mut self, timeout: Duration) -> (Vec<Patch>, bool) {
        self.runtime.settle(timeout)
    }

    /// Scrolls a virtual list from the top until a node matching `item` is built, and
    /// returns it. Virtual lists only hold the rows in view, so a plain [`find`](Self::find)
    /// can't see the rest. The list stays scrolled to where the item was found.
    pub fn scroll_to_find(&mut self, list: &Query, item: &Query) -> Result<Found, ActError> {
        let mut top = 0;
        loop {
            let found = self.find(list)?;
            self.runtime.dispatch(
                &found.path,
                ActionKind::Scroll,
                Some(PropValue::Int(top as i64)),
            )?;
            self.runtime.flush();

            let found = self.find(list)?;
            if let Some(result) = item.find_all(&found.node, &found.path).into_iter().next() {
                return Ok(result);
            }
            let read = |name: &str| match found.node.props.get(name) {
                Some(PropValue::Int(value)) => *value as usize,
                _ => 0,
            };
            let first_built = read("first_built");
            let built = found.node.children.len();
            if first_built + built >= read("count") || built == 0 {
                return Err(FindError::NotFound {
                    query: item.to_string(),
                    similar: Vec::new(),
                }
                .into());
            }
            // Rows already built were just searched, so jump past them.
            top = first_built + built;
        }
    }
}
