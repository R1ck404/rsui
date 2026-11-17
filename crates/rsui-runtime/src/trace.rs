//! What rendered, why, and how long it took, kept only while something's looking: see [`Runtime::set_tracing`].
//!
//! While nothing's looking, rendering checks one `Option` and does nothing else, so an app that isn't being looked at
//! pays nothing for this.
//!
//! [`Runtime::set_tracing`]: crate::Runtime::set_tracing

use std::time::{Duration, Instant};

use rsui_core::{ActionKind, Key, Text};

/// Why a component rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// It's new.
    Mounted,
    /// What's around it rendered and gave it different props.
    Props,
    /// Its own state changed, or it's moving.
    State,
}

/// One component's render.
#[derive(Debug, Clone)]
pub struct Rendered {
    /// Which component, as the runtime numbers them. A number can be used again once that component's gone, and
    /// `generation` tells them apart.
    pub component: u32,
    pub generation: u32,
    /// The component that rendered it, unless it's the root.
    pub parent: Option<u32>,
    pub depth: u32,
    pub cause: Cause,
    /// How long it took, with the components it rendered inside it.
    pub took: Duration,
    /// How long it took without them.
    pub own: Duration,
    /// Whether what it gave was any different from last time. A render that changed nothing could have been left out.
    pub changed: bool,
    /// What it's called, if it said with [`Cx::trace_name`](crate::Cx::trace_name).
    pub name: Option<Text>,
    /// Why it rendered, in sentences, if it said with [`Cx::trace_why`](crate::Cx::trace_why).
    pub why: Vec<String>,
    /// Where its output is in the tree, as [`Runtime::take_trace`](crate::Runtime::take_trace) found it, unless it was
    /// gone by then.
    pub path: Option<Vec<u32>>,
}

/// An action that was carried out, and how long its handler took.
#[derive(Debug, Clone)]
pub struct Handled {
    pub action: ActionKind,
    /// What it was carried out on.
    pub key: Key,
    pub path: Vec<u32>,
    pub took: Duration,
    /// Whether the handler turned it down.
    pub rejected: bool,
    /// What the handler said about itself, as where it's written, with
    /// [`Updates::trace_handler`](crate::Updates::trace_handler).
    pub note: Option<String>,
}

/// What's happened since [`Runtime::take_trace`](crate::Runtime::take_trace) last said.
#[derive(Debug, Clone, Default)]
pub struct Trace {
    /// In the order they started, so a component comes before what it rendered inside it.
    pub renders: Vec<Rendered>,
    pub handled: Vec<Handled>,
    /// How long flushing took, which is rendering and running effects.
    pub flushing: Duration,
}

impl Trace {
    pub fn is_empty(&self) -> bool {
        self.renders.is_empty() && self.handled.is_empty()
    }
}

/// What's being kept while tracing.
#[derive(Default)]
pub(crate) struct Tracer {
    pub(crate) trace: Trace,
    /// The renders going on now, innermost last, each with where it is in `trace.renders` and how long what it's rendered
    /// inside it has taken so far.
    pub(crate) open: Vec<(usize, Duration)>,
    /// What the handler that's running has said about itself.
    pub(crate) handler_note: Option<String>,
}

impl Tracer {
    pub(crate) fn start(
        &mut self,
        component: u32,
        generation: u32,
        parent: Option<u32>,
        depth: u32,
        cause: Cause,
    ) {
        self.open.push((self.trace.renders.len(), Duration::ZERO));
        self.trace.renders.push(Rendered {
            component,
            generation,
            parent,
            depth,
            cause,
            took: Duration::ZERO,
            own: Duration::ZERO,
            changed: true,
            name: None,
            why: Vec::new(),
            path: None,
        });
    }

    pub(crate) fn finish(&mut self, started: Instant, changed: bool) {
        let Some((at, inside)) = self.open.pop() else {
            return;
        };
        let took = started.elapsed();
        let render = &mut self.trace.renders[at];
        render.took = took;
        render.own = took.saturating_sub(inside);
        render.changed = changed;
        if let Some((_, outer)) = self.open.last_mut() {
            *outer += took;
        }
    }

    /// The render going on now.
    pub(crate) fn current(&mut self) -> Option<&mut Rendered> {
        let (at, _) = *self.open.last()?;
        self.trace.renders.get_mut(at)
    }
}
