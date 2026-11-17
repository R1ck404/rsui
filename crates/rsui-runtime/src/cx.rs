use std::any::Any;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::time::{Duration, Instant};

use foldhash::fast::RandomState;
use rsui_core::{ActionKind, Element, IntoNode, Key, Node, PropValue, Text};
use thin_vec::ThinVec;

use crate::event::Event;
use crate::remote::{Remote, TaskGuard};
use crate::runtime::{Cleanup, Handler, Mounted, PendingEffect, Store, node_at};
use crate::trace::Cause;

/// Marks an old child that this render has already claimed.
const CLAIMED: u32 = u32::MAX;
/// How far past where the next child is expected to look for it, before a hash map is worth making.
const NEARBY_CHILDREN: usize = 8;

/// A handle to a piece of a component's state. It's cheap to copy into handlers
/// and effects, and only works through a [`Cx`] or [`Updates`].
pub struct State<T> {
    instance: u32,
    generation: u32,
    hook: u32,
    value: PhantomData<fn() -> T>,
}

impl<T> Clone for State<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for State<T> {}

/// Compares which state this is, not its value, so a parent can pass a `State` to a
/// child as a prop without that making the child render every time the value changes.
impl<T> PartialEq for State<T> {
    fn eq(&self, other: &Self) -> bool {
        (self.instance, self.generation, self.hook)
            == (other.instance, other.generation, other.hook)
    }
}

impl<T> Eq for State<T> {}

impl<T> std::fmt::Debug for State<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "State({}/{}/{})",
            self.instance, self.generation, self.hook
        )
    }
}

impl Store {
    /// The value of `state`, unless its component has been unmounted.
    fn value<T: 'static>(&self, state: State<T>) -> Option<&T> {
        let instance = self.instances.get(state.instance as usize)?;
        if !instance.alive || instance.generation != state.generation {
            return None;
        }
        instance.hooks[state.hook as usize].downcast_ref()
    }

    fn value_mut<T: 'static>(&mut self, state: State<T>) -> Option<&mut T> {
        let instance = self.instances.get_mut(state.instance as usize)?;
        if !instance.alive || instance.generation != state.generation {
            return None;
        }
        instance.hooks[state.hook as usize].downcast_mut()
    }
}

const GONE: &str = "this state's component has been unmounted";

macro_rules! state_access {
    ($type:ident) => {
        impl $type<'_> {
            /// A copy of the value.
            ///
            /// # Panics
            ///
            /// If the state's component has been unmounted.
            pub fn get<T: Clone + 'static>(&self, state: State<T>) -> T {
                self.store.value(state).expect(GONE).clone()
            }

            /// A copy of the value, or nothing if the state's component has been unmounted.
            pub fn get_if_mounted<T: Clone + 'static>(&self, state: State<T>) -> Option<T> {
                self.store.value(state).cloned()
            }

            /// Reads the value without copying it.
            ///
            /// # Panics
            ///
            /// If the state's component has been unmounted.
            pub fn with<T: 'static, R>(&self, state: State<T>, read: impl FnOnce(&T) -> R) -> R {
                read(self.store.value(state).expect(GONE))
            }

            /// Replaces the value, and renders the component again if it changed.
            /// Does nothing if the component has been unmounted.
            pub fn set<T: PartialEq + 'static>(&mut self, state: State<T>, value: T) {
                if let Some(current) = self.store.value_mut(state) {
                    if *current != value {
                        *current = value;
                        self.store.mark_dirty(state.instance);
                    }
                }
            }

            /// Changes the value in place, and always renders the component again.
            /// Does nothing if the component has been unmounted.
            pub fn update<T: 'static>(&mut self, state: State<T>, change: impl FnOnce(&mut T)) {
                if let Some(current) = self.store.value_mut(state) {
                    change(current);
                    self.store.mark_dirty(state.instance);
                }
            }
        }
    };
}

state_access!(Cx);
state_access!(Updates);

/// What handlers, effects and cleanups can do: change state, not render.
pub struct Updates<'a> {
    store: &'a mut Store,
}

impl<'a> Updates<'a> {
    pub(crate) fn new(store: &'a mut Store) -> Self {
        Self { store }
    }

    /// Has `tell` run once what's just scrolled has been drawn, with [`Runtime::deliver_scrolled`](crate::Runtime::deliver_scrolled),
    /// rather than now: scrolling shows without waiting for what it tells. Asking again for the same `state` before then
    /// replaces what was asked, keeping its place, so it's told once, as a browser fires `scroll` once a frame.
    pub fn tell_scrolled_later<T>(&mut self, state: State<T>, tell: std::rc::Rc<dyn Fn(&mut Updates<'_>)>) {
        let id = (state.instance, state.hook);
        match self.store.scrolled.iter_mut().find(|(waiting, _)| *waiting == id) {
            Some(waiting) => waiting.1 = tell,
            None => self.store.scrolled.push((id, tell)),
        }
    }

    /// Whether anything that's changed hasn't been rendered yet, or has and its effects haven't run: what a timer that one of those
    /// replaces is waiting for, as it mustn't run as it was.
    /// Whether what happens is being kept, as [`Runtime::set_tracing`](crate::Runtime::set_tracing) says.
    pub fn tracing(&self) -> bool {
        self.store.tracer.is_some()
    }

    /// Says something about the handler that's running, such as where it's written, for what's tracing.
    pub fn trace_handler(&mut self, note: impl Into<String>) {
        if let Some(tracer) = self.store.tracer.as_mut() {
            tracer.handler_note = Some(note.into());
        }
    }

    pub fn effects_waiting(&self) -> bool {
        self.store.effects_left > 0
            || !self.store.effects.is_empty()
            || !self.store.dirty.is_empty()
    }

    /// In a handler, refuses the action. [`Runtime::dispatch`](crate::Runtime::dispatch)
    /// returns this reason, so an agent can learn what it did wrong.
    pub fn reject(&mut self, reason: impl Into<Text>) {
        self.store.rejected = Some(reason.into());
    }

    /// Gives the keyboard to the element with this key, once this has been applied. It's for when something
    /// has to be ready to type in, as when a dialog opens. If nothing with this key can have the keyboard,
    /// nothing happens.
    pub fn focus(&mut self, key: impl Into<Key>) {
        self.focus_with_options(key, None, false);
    }

    /// [`Updates::focus`], saying whether it's to show that it has the keyboard (a focus ring, `:focus-visible`), as a page's
    /// `focus({ focusVisible })` does, rather than that following how the keyboard was last moved. `None` is [`Updates::focus`].
    pub fn focus_showing(&mut self, key: impl Into<Key>, visible: Option<bool>) {
        self.focus_with_options(key, visible, false);
    }

    /// Gives the keyboard to an element, optionally leaving its scroll ancestors where they are.
    pub fn focus_with_options(
        &mut self,
        key: impl Into<Key>,
        visible: Option<bool>,
        prevent_scroll: bool,
    ) {
        self.store.focus_wanted = Some(key.into());
        self.store.focus_visible_wanted = visible;
        self.store.focus_prevent_scroll_wanted = prevent_scroll;
    }

    /// Selects from `start` to `end` of the text in the field with this key, counted in UTF-16 units as on the web, if it has
    /// the keyboard once this has been applied. The caret goes to `end`.
    pub fn select_text(&mut self, key: impl Into<Key>, start: u32, end: u32) {
        self.store.select_wanted = Some((key.into(), start, end));
    }

    /// Gives the pointer to the element with this key, or lets it go (`None`), as a page's `setPointerCapture()` and
    /// `releasePointerCapture()` do: while the main button's down, what it does goes to that element wherever the pointer is.
    pub fn capture_pointer(&mut self, key: Option<Key>) {
        self.store.capture_wanted = Some(key);
    }

    /// Which runtime this is, among any others on this thread, for telling apart what belongs to which window.
    pub fn runtime_id(&self) -> u32 {
        self.store.id
    }

    /// Changes what the window's called.
    pub fn set_title(&mut self, title: impl Into<Text>) {
        self.store.title_wanted = Some(title.into());
    }

    /// Puts this on the clipboard.
    pub fn copy_text(&mut self, text: impl Into<Text>) {
        self.store.copy_wanted = Some(text.into());
    }

    /// Asks for what's on the clipboard, which goes to what was given to [`Cx::on_clipboard`].
    pub fn ask_for_clipboard(&mut self) {
        self.store.paste_wanted = true;
    }

    /// Opens a web page in the browser, or an address in the mail program. Nothing else can be
    /// opened this way, as what a program shows can come from anywhere.
    pub fn open_url(&mut self, url: impl Into<Text>) {
        self.store.urls_wanted.push(url.into());
    }

    /// Makes the window's title bar dark or light, to go with what's in it.
    pub fn set_dark_title_bar(&mut self, dark: bool) {
        self.store.dark_title_bar_wanted = Some(dark);
    }

    /// Has text drawn letter for letter the way the system draws it, as a browser does, rather than the way rsui does, which is
    /// for what's written for a page and should look the way it would in one.
    pub fn draw_text_as_the_system_does(&mut self) {
        self.store.system_glyphs = true;
    }

    /// In a handler for a key, lets what's around this element hear about it too. It stops here otherwise.
    pub fn pass_on(&mut self) {
        self.store.passed_on = true;
    }

    /// What came with what's being handled, if anything did: where the pointer was, which button and key, and what it
    /// happened to. See [`Runtime::dispatch_with`](crate::Runtime::dispatch_with).
    pub fn event(&self) -> Option<&Event> {
        self.store.event.as_ref()
    }

    /// Says that nothing further along should hear about what's being handled, as the web's `stopPropagation()` does.
    pub fn stop_propagation(&mut self) {
        self.store.stopped = true;
    }

    /// Says that what would happen by itself shouldn't, as the web's `preventDefault()` does: a key isn't typed, a press
    /// doesn't move the keyboard, and the wheel doesn't scroll.
    pub fn prevent_default(&mut self) {
        self.store.prevented = true;
    }

    /// Whether something that's already heard about what's being handled said [`prevent_default`](Self::prevent_default), for
    /// what hears it later, as the web's `defaultPrevented` says.
    pub fn default_prevented(&self) -> bool {
        self.store.prevented
    }

    /// A way to change state from another thread.
    pub fn remote(&self) -> Remote {
        Remote::new(self.store.inbox.clone())
    }

    /// Runs `task` on its own thread, and counts it as background work until it
    /// finishes, so [`Runtime::settle`](crate::Runtime::settle) can wait for it.
    ///
    /// ```
    /// use rsui_core::{Element, Role, text};
    /// use rsui_runtime::{Cx, Runtime};
    /// use std::time::Duration;
    ///
    /// fn app(cx: &mut Cx, _: &()) -> Element {
    ///     let status = cx.state(|| "loading");
    ///     cx.effect((), move |cx| {
    ///         cx.spawn(move |remote| {
    ///             // Do slow work here, then report back.
    ///             remote.send(move |cx| cx.set(status, "done"));
    ///         });
    ///     });
    ///     Element::new(Role::Text).name(cx.get(status))
    /// }
    ///
    /// let mut runtime = Runtime::new(app, ());
    /// let (_, settled) = runtime.settle(Duration::from_secs(5));
    /// assert!(settled);
    /// assert_eq!(runtime.root().name, "done");
    /// ```
    pub fn spawn(&mut self, task: impl FnOnce(Remote) + Send + 'static) {
        let inbox = self.store.inbox.clone();
        inbox.begin_task();
        std::thread::spawn(move || {
            let remote = Remote::new(inbox.clone());
            let _guard = TaskGuard(inbox);
            task(remote);
        });
    }

    /// Runs `job` after `delay`. If the component has unmounted by then, any changes
    /// it makes to that component's state do nothing.
    pub fn after(&mut self, delay: Duration, job: impl FnOnce(&mut Updates<'_>) + Send + 'static) {
        let inbox = self.store.inbox.clone();
        inbox.begin_task();
        // This belongs to the timer, so the task finishes even if the timer never fires.
        let guard = TaskGuard(inbox.clone());
        crate::timer::after(delay, move || {
            let _guard = guard;
            Remote::new(inbox).send(job);
        });
    }
}

/// A number on its way somewhere.
struct Motion {
    from: f64,
    to: f64,
    started: Instant,
    duration: Duration,
}

impl Motion {
    fn resting(value: f64) -> Self {
        Self {
            from: value,
            to: value,
            started: Instant::now(),
            duration: Duration::ZERO,
        }
    }

    fn value_at(&self, now: Instant) -> f64 {
        let elapsed = now.saturating_duration_since(self.started).as_secs_f64();
        let total = self.duration.as_secs_f64();
        if elapsed >= total {
            return self.to;
        }
        // Quick to start, and slowing down to arrive, which is how things feel like they respond.
        let left = 1.0 - elapsed / total;
        self.from + (self.to - self.from) * (1.0 - left * left * left)
    }
}

/// An allocation-free view of a component's previous output for rebuilding host nodes.
/// Child component outputs are excluded: their identity belongs to those components.
/// Keep this only while rendering; it holds the old tree, not another persistent cache.
pub struct PreviousNodes {
    root: Node,
    children: [*const Element; 8],
    count: usize,
}

/// A node owned by the component that captured [`PreviousNodes`].
#[derive(Clone, Copy)]
pub struct PreviousNode<'a> {
    node: &'a Node,
    previous: &'a PreviousNodes,
}

impl PreviousNodes {
    /// The old root, unless it was itself a child component's output.
    pub fn root(&self) -> Option<PreviousNode<'_>> {
        self.owned(&self.root)
    }

    fn owned<'a>(&'a self, node: &'a Node) -> Option<PreviousNode<'a>> {
        (!self.children[..self.count].contains(&triomphe::Arc::as_ptr(node))).then_some(
            PreviousNode {
                node,
                previous: self,
            },
        )
    }
}

impl PreviousNode<'_> {
    /// A child at this output position, stopping at component ownership boundaries.
    pub fn child(self, index: usize) -> Option<Self> {
        self.node
            .children
            .get(index)
            .and_then(|node| self.previous.owned(node))
    }

    /// Reuses a host node after rebuilding its observable values and registering
    /// current handlers. Children must be the exact same nodes, so replacement
    /// component outputs cannot be confused with equal-looking old host nodes.
    /// Motion declarations always use a new node: displayed values can differ
    /// from the animation target that a render must update.
    pub fn reuse(self, element: Element) -> Node {
        let old = &**self.node;
        if old.key == element.key
            && old.role == element.role
            && old.name == element.name
            && old.props == element.props
            && old.actions == element.actions
            && !crate::motion::declared_on(&element.props)
            && old.children.len() == element.children.len()
            && old
                .children
                .iter()
                .zip(element.children.iter())
                .all(|(a, b)| triomphe::Arc::ptr_eq(a, b))
        {
            self.node.clone()
        } else {
            element.into_node()
        }
    }
}

/// What a component can do while it renders.
///
/// Hooks identify their state by the order they're called in, as in React. A
/// component must call the same hooks in the same order on every render. Breaking
/// this panics, rather than silently mixing up state.
pub struct Cx<'a> {
    store: &'a mut Store,
    id: u32,
    next_hook: usize,
    first_render: bool,
    old_output: Option<Node>,
    old_children: ThinVec<u32>,
    /// The next old child to expect, since children usually render in the same order.
    cursor: usize,
    /// Whether anything before the cursor hasn't been claimed.
    skipped: bool,
    /// Finds old children when they don't. Built only if needed.
    lookup: Option<HashMap<(Option<Key>, u64), usize, RandomState>>,
    /// For each child of the last output's root, which old child's output it was, for [`keep_child`](Self::keep_child). Built only
    /// if that's used.
    outputs: Option<Vec<u32>>,
    rendered: Vec<(u32, Node)>,
    unkeyed: u64,
    cause: Cause,
}

impl<'a> Cx<'a> {
    pub(crate) fn new(
        store: &'a mut Store,
        id: u32,
        old_output: Option<Node>,
        old_children: ThinVec<u32>,
        cause: Cause,
    ) -> Self {
        Self {
            store,
            id,
            next_hook: 0,
            first_render: old_output.is_none(),
            old_output,
            old_children,
            cursor: 0,
            skipped: false,
            lookup: None,
            outputs: None,
            rendered: Vec::new(),
            unkeyed: 0,
            cause,
        }
    }

    /// Captures old host nodes before rendering any child components. This uses
    /// fixed stack storage and declines when there are more than eight old child
    /// components, no previous output, or a child has already been claimed.
    ///
    /// A host must still evaluate props/getters and register current handlers before
    /// calling [`PreviousNode::reuse`]. The view never crosses an old child
    /// component's output, even if that component is replaced by an equal host node.
    ///
    /// ```
    /// use rsui_core::{Element, Node, Role};
    /// use rsui_runtime::Cx;
    /// fn label(cx: &mut Cx<'_>, value: &String) -> Node {
    ///     let previous = cx.previous_nodes();
    ///     let element = Element::new(Role::Text).name(value.as_str());
    ///     match previous.as_ref().and_then(|old| old.root()) {
    ///         Some(old) => old.reuse(element),
    ///         None => element.into_node(),
    ///     }
    /// }
    /// ```
    pub fn previous_nodes(&self) -> Option<PreviousNodes> {
        if self.old_children.len() > 8 || self.old_children.contains(&CLAIMED) {
            return None;
        }
        let root = self.old_output.as_ref()?.clone();
        let mut children = [std::ptr::null(); 8];
        for (slot, &id) in children.iter_mut().zip(self.old_children.iter()) {
            let path = &self.store.instances[id as usize].path;
            *slot = triomphe::Arc::as_ptr(node_at(&root, path));
        }
        Some(PreviousNodes {
            root,
            children,
            count: self.old_children.len(),
        })
    }

    /// Whether what renders is being kept, as [`Runtime::set_tracing`](crate::Runtime::set_tracing) says. A component
    /// that has more to say about why it rendered says it with [`trace_why`](Self::trace_why), and should only work that
    /// out when this is on.
    pub fn tracing(&self) -> bool {
        self.store.tracer.is_some()
    }

    /// Why this is rendering.
    pub fn cause(&self) -> Cause {
        self.cause
    }

    /// This component, as [`Rendered`](crate::Rendered) and [`Runtime::component_at`](crate::Runtime::component_at)
    /// number them, with its generation.
    pub fn component(&self) -> (u32, u32) {
        (self.id, self.store.instances[self.id as usize].generation)
    }

    /// What this component is called, for what's tracing. It does nothing if nothing is.
    pub fn trace_name(&mut self, name: impl Into<Text>) {
        if let Some(render) = self
            .store
            .tracer
            .as_mut()
            .and_then(|tracer| tracer.current())
        {
            render.name = Some(name.into());
        }
    }

    /// Says why this rendered, in a sentence, for what's tracing. It does nothing if nothing is.
    pub fn trace_why(&mut self, why: impl Into<String>) {
        if let Some(render) = self
            .store
            .tracer
            .as_mut()
            .and_then(|tracer| tracer.current())
        {
            render.why.push(why.into());
        }
    }

    /// Ends the render, returning the children it rendered and the old children it
    /// didn't.
    pub(crate) fn finish(self) -> (Vec<(u32, Node)>, Vec<u32>) {
        let hooks = self.store.instances[self.id as usize].hooks.len();
        assert_eq!(
            self.next_hook, hooks,
            "a component must call the same hooks on every render"
        );
        let leftover = self
            .old_children
            .iter()
            .copied()
            .filter(|&child| child != CLAIMED)
            .collect();
        (self.rendered, leftover)
    }

    /// Adds state to this component, starting at `initial` on the first render.
    pub fn state<T: 'static>(&mut self, initial: impl FnOnce() -> T) -> State<T> {
        let hook = self.hook(initial);
        let _: &T = self.slot(hook);
        State {
            instance: self.id,
            generation: self.store.instances[self.id as usize].generation,
            hook: hook as u32,
            value: PhantomData,
        }
    }

    /// A number that moves smoothly to `target` whenever that changes, starting quickly and slowing
    /// down, taking `duration`, from wherever it had got to. It starts at the first `target`. Use it
    /// for anything that takes a number, such as a width, or `opacity`:
    ///
    /// ```
    /// # use std::time::{Duration, Instant};
    /// # use rsui_core::{Element, Role};
    /// # use rsui_runtime::{Cx, Runtime};
    /// fn panel(cx: &mut Cx, open: &bool) -> Element {
    ///     let height = cx.animate(if *open { 200.0 } else { 0.0 }, Duration::from_millis(150));
    ///     Element::new(Role::Group).prop("height", height)
    /// }
    /// ```
    ///
    /// While it's moving, this component renders again for each frame. It costs nothing once it's
    /// arrived, or if animations are off, in which case this is always `target`.
    pub fn animate(&mut self, target: f64, duration: Duration) -> f64 {
        let now = self.store.clock();
        let off = !self.store.animations;
        let hook = self.next_hook;
        let id = self.id as usize;
        let existing = self.store.instances[id].hooks.get_mut(hook);
        let (value, moving) = if let Some(slot) = existing {
            let motion = slot
                .downcast_mut::<Motion>()
                .expect("a component must call the same hooks on every render");
            self.next_hook += 1;
            if off || !target.is_finite() {
                *motion = Motion::resting(target);
                (target, false)
            } else {
                if motion.to != target {
                    motion.from = motion.value_at(now);
                    motion.to = target;
                    motion.started = now;
                    motion.duration = duration;
                }
                let value = motion.value_at(now);
                if value == target {
                    *motion = Motion::resting(target);
                }
                (value, value != target)
            }
        } else {
            self.hook(move || Motion::resting(target));
            (target, false)
        };
        if moving && !self.store.animating.contains(&self.id) {
            self.store.animating.push(self.id);
        }
        value
    }

    /// Asks for this component to render again on the next frame, for something moving that it works out itself.
    /// A [`Runtime`](crate::Runtime) with animations off still goes on rendering it for as long as it keeps asking.
    pub fn next_frame(&mut self) {
        if !self.store.animating.contains(&self.id) {
            self.store.animating.push(self.id);
        }
    }

    /// What time it is for anything that moves.
    pub fn frame_time(&self) -> Instant {
        self.store.clock()
    }

    /// Whether things move, or go straight to where they're going.
    pub fn animations_on(&self) -> bool {
        self.store.animations
    }

    /// Computes a value only when `deps` change. Pass a [`Node`] as `T` to reuse a
    /// subtree that doesn't depend on anything else, without building it again.
    pub fn memo<D, T>(&mut self, deps: D, compute: impl FnOnce() -> T) -> T
    where
        D: PartialEq + 'static,
        T: Clone + 'static,
    {
        struct Memo<D, T> {
            deps: D,
            value: T,
        }
        let hook = self.next_hook;
        let existing = self.store.instances[self.id as usize].hooks.get_mut(hook);
        if let Some(slot) = existing {
            let memo = slot
                .downcast_mut::<Memo<D, T>>()
                .expect("a component must call the same hooks on every render");
            self.next_hook += 1;
            if memo.deps != deps {
                memo.value = compute();
                memo.deps = deps;
            }
            return memo.value.clone();
        }
        let value = compute();
        let result = value.clone();
        self.hook(move || Memo { deps, value });
        result
    }

    /// Runs `run` after the tree is updated: after the first render, then whenever
    /// `deps` change. It can change state, which starts another update.
    pub fn effect<D: PartialEq + 'static>(
        &mut self,
        deps: D,
        run: impl FnOnce(&mut Updates<'_>) + 'static,
    ) {
        self.schedule(deps, move |updates| {
            run(updates);
            None
        });
    }

    /// Gives the keyboard to the element with this key. See [`Updates::focus`].
    pub fn focus(&mut self, key: impl Into<Key>) {
        self.focus_with_options(key, None, false);
    }

    /// Gives the keyboard to the element with this key, saying whether it shows that. See [`Updates::focus_showing`].
    pub fn focus_showing(&mut self, key: impl Into<Key>, visible: Option<bool>) {
        self.focus_with_options(key, visible, false);
    }

    /// Gives the keyboard to an element, optionally leaving its scroll ancestors where they are.
    pub fn focus_with_options(
        &mut self,
        key: impl Into<Key>,
        visible: Option<bool>,
        prevent_scroll: bool,
    ) {
        self.store.focus_wanted = Some(key.into());
        self.store.focus_visible_wanted = visible;
        self.store.focus_prevent_scroll_wanted = prevent_scroll;
    }

    /// Selects from `start` to `end` of the text in the field with this key, counted in UTF-16 units as on the web, if it has
    /// the keyboard once this has been applied. The caret goes to `end`.
    pub fn select_text(&mut self, key: impl Into<Key>, start: u32, end: u32) {
        self.store.select_wanted = Some((key.into(), start, end));
    }

    /// Gives the pointer to the element with this key, or lets it go (`None`), as a page's `setPointerCapture()` and
    /// `releasePointerCapture()` do: while the main button's down, what it does goes to that element wherever the pointer is.
    pub fn capture_pointer(&mut self, key: Option<Key>) {
        self.store.capture_wanted = Some(key);
    }

    /// See [`Updates::runtime_id`].
    pub fn runtime_id(&self) -> u32 {
        self.store.id
    }

    /// See [`Updates::set_title`].
    pub fn set_title(&mut self, title: impl Into<Text>) {
        self.store.title_wanted = Some(title.into());
    }

    /// See [`Updates::copy_text`].
    pub fn copy_text(&mut self, text: impl Into<Text>) {
        self.store.copy_wanted = Some(text.into());
    }

    /// See [`Updates::ask_for_clipboard`].
    pub fn ask_for_clipboard(&mut self) {
        self.store.paste_wanted = true;
    }

    /// See [`Updates::open_url`].
    pub fn open_url(&mut self, url: impl Into<Text>) {
        self.store.urls_wanted.push(url.into());
    }

    /// See [`Updates::set_dark_title_bar`].
    pub fn set_dark_title_bar(&mut self, dark: bool) {
        self.store.dark_title_bar_wanted = Some(dark);
    }

    /// See [`Updates::draw_text_as_the_system_does`].
    pub fn draw_text_as_the_system_does(&mut self) {
        self.store.system_glyphs = true;
    }

    /// Says what to do when the system goes between dark and light, which is told whether it's now dark.
    pub fn on_color_scheme(&mut self, told: fn(&mut Updates<'_>, bool)) {
        self.store.on_color_scheme = Some(told);
    }

    /// Says what to do when the window changes size, which is told how big it now is, in logical pixels. It's told once at the start too.
    pub fn on_resize(&mut self, told: fn(&mut Updates<'_>, f32, f32)) {
        self.store.on_resize = Some(told);
    }

    /// Says what to do once the window's been laid out, which is after anything's changed where things are.
    pub fn on_laid_out(&mut self, told: fn(&mut Updates<'_>)) {
        self.store.on_laid_out = Some(told);
    }

    /// Says what to do with what's on the clipboard, when it's been asked for.
    pub fn on_clipboard(&mut self, told: fn(&mut Updates<'_>, Option<&str>)) {
        self.store.on_paste = Some(told);
    }

    /// Says what hears about what happens for the app as a whole, as a page's `document` does: it's given what the web
    /// calls it, as `"keydown"`, and [`Updates::event`] says the rest, with its `phase` 1 on the way down to what it happened
    /// to and 3 on the way back up. Only the kinds [`set_document_listens`](crate::set_document_listens) says are told.
    pub fn on_document_event(&mut self, told: fn(&mut Updates<'_>, &str)) {
        self.store.on_document_event = Some(told);
    }

    /// Says what to do if effects never settle, as when one keeps setting the state that makes it run
    /// again. They're stopped either way, and this is for showing that something went wrong.
    pub fn on_runaway(&mut self, told: fn(&mut Updates<'_>)) {
        self.store.on_runaway = Some(told);
    }

    /// Says what to do once each round of effects has run, after the last of them. It's for something that keeps effects of its own
    /// to run them all together, in the order it wants, before what changed is drawn.
    pub fn on_effects_done(&mut self, told: fn(&mut Updates<'_>)) {
        self.store.on_effects_done = Some(told);
    }

    /// Like [`effect`](Self::effect), where `run` returns a function to undo it. That
    /// function runs before the effect runs again, or after the component unmounts.
    /// By then its state may be gone, which [`Updates::set`] and
    /// [`Updates::update`] allow for.
    pub fn effect_with_cleanup<D, C>(
        &mut self,
        deps: D,
        run: impl FnOnce(&mut Updates<'_>) -> C + 'static,
    ) where
        D: PartialEq + 'static,
        C: FnOnce(&mut Updates<'_>) + 'static,
    {
        self.schedule(deps, move |updates| Some(Box::new(run(updates)) as Cleanup));
    }

    fn schedule<D, F>(&mut self, deps: D, run: F)
    where
        D: PartialEq + 'static,
        F: FnOnce(&mut Updates<'_>) -> Option<Cleanup> + 'static,
    {
        let hook = self.next_hook;
        let instance = &mut self.store.instances[self.id as usize];
        let generation = instance.generation;
        let changed = if let Some(slot) = instance.hooks.get_mut(hook) {
            let old = slot
                .downcast_mut::<D>()
                .expect("a component must call the same hooks on every render");
            self.next_hook += 1;
            let changed = *old != deps;
            *old = deps;
            changed
        } else {
            self.hook(move || deps);
            true
        };
        if changed {
            self.store.effects.push(PendingEffect {
                instance: self.id,
                generation,
                hook: hook as u32,
                run: Box::new(run),
            });
        }
    }

    /// Makes `element` respond to `action`, and returns it with that action added.
    ///
    /// Handlers find their element by its key, which is also what gives an agent a
    /// stable way to refer to it. An element without a key takes the key of its
    /// component, as passed to [`keyed`](Self::keyed).
    ///
    /// # Panics
    ///
    /// If the element has no key to use. In debug builds, also if this component
    /// already handles this action for an element with the same key.
    pub fn on(
        &mut self,
        mut element: Element,
        action: ActionKind,
        handler: impl FnMut(&mut Updates<'_>, Option<&PropValue>) + 'static,
    ) -> Element {
        let instance = &mut self.store.instances[self.id as usize];
        if element.key.is_none() {
            element.key.clone_from(&instance.key);
        }
        let key = element
            .key
            .clone()
            .expect("an element with a handler needs a key");
        debug_assert!(
            !instance
                .handlers
                .iter()
                .any(|existing| existing.action == action && existing.key == key),
            "two elements in one component handle the same action with the same key"
        );
        // Most elements have one handler, so spare room would mostly go unused.
        instance.handlers.reserve_exact(1);
        instance.handlers.push(Handler {
            key,
            action,
            run: Box::new(handler),
        });
        element.action(action)
    }

    /// Renders a child component. It keeps its state from one render to the next as
    /// long as it's called in the same position among unkeyed children, and it only
    /// renders again if `props` changed or its state did.
    ///
    /// The returned node must be placed in this component's output.
    /// A child that isn't is unmounted.
    pub fn child<P, R>(&mut self, render: fn(&mut Cx<'_>, &P) -> R, props: P) -> Node
    where
        P: PartialEq + 'static,
        R: IntoNode + 'static,
    {
        let place = self.unkeyed;
        self.unkeyed += 1;
        self.mount_child(None, place, render, props, |_, _| true)
    }

    /// Like [`child`](Self::child), for one `render` that stands for many components, as when another language defines
    /// them. If `same_kind` says the new props aren't for the same kind of component as the old ones, the old one is
    /// unmounted, and this starts again with new state.
    pub fn child_of_kind<P, R>(
        &mut self,
        render: fn(&mut Cx<'_>, &P) -> R,
        props: P,
        same_kind: fn(&P, &P) -> bool,
    ) -> Node
    where
        P: PartialEq + 'static,
        R: IntoNode + 'static,
    {
        let place = self.unkeyed;
        self.unkeyed += 1;
        self.mount_child(None, place, render, props, same_kind)
    }

    /// Like [`child_of_kind`](Self::child_of_kind), at a place the caller works out, as React tells children apart: by
    /// where each is in the arrays of children around it, holes and all, rather than by how many came before it. With a
    /// key, it's that key among the children at the same `place`, which is then where the array it's in is. It keeps its
    /// state for as long as it's rendered at the same place, with the same key, and `same_kind` agrees.
    ///
    /// A component should give all of its children places this way, or none of them, as the places [`child`](Self::child)
    /// counts are small numbers that one of these could be.
    pub fn child_at<P, R>(
        &mut self,
        key: Option<Key>,
        place: u64,
        render: fn(&mut Cx<'_>, &P) -> R,
        props: P,
        same_kind: fn(&P, &P) -> bool,
    ) -> Node
    where
        P: PartialEq + 'static,
        R: IntoNode + 'static,
    {
        self.mount_child(key, place, render, props, same_kind)
    }

    /// Like [`keyed`](Self::keyed), and [`child_of_kind`](Self::child_of_kind).
    pub fn keyed_of_kind<P, R>(
        &mut self,
        key: impl Into<Key>,
        render: fn(&mut Cx<'_>, &P) -> R,
        props: P,
        same_kind: fn(&P, &P) -> bool,
    ) -> Node
    where
        P: PartialEq + 'static,
        R: IntoNode + 'static,
    {
        self.mount_child(Some(key.into()), 0, render, props, same_kind)
    }

    /// Renders a child component identified by `key` among its siblings, so that it
    /// keeps its state when its position changes. Its element takes that key too.
    pub fn keyed<P, R>(
        &mut self,
        key: impl Into<Key>,
        render: fn(&mut Cx<'_>, &P) -> R,
        props: P,
    ) -> Node
    where
        P: PartialEq + 'static,
        R: IntoNode + 'static,
    {
        self.mount_child(Some(key.into()), 0, render, props, |_, _| true)
    }

    fn mount_child<P, R>(
        &mut self,
        key: Option<Key>,
        place: u64,
        render: fn(&mut Cx<'_>, &P) -> R,
        props: P,
        same_kind: fn(&P, &P) -> bool,
    ) -> Node
    where
        P: PartialEq + 'static,
        R: IntoNode + 'static,
    {
        let mut new_props = Some(props);
        let mut result = None;
        if let Some(id) = self.claim_child(&key, place) {
            let instance = &mut self.store.instances[id as usize];
            let component = instance
                .component
                .as_mut()
                .expect("a mounted child isn't rendering")
                .as_any_mut()
                .downcast_mut::<Mounted<P, R>>()
                .filter(|mounted| {
                    std::ptr::fn_addr_eq(mounted.render, render)
                        && new_props
                            .as_ref()
                            .is_some_and(|props| same_kind(&mounted.props, props))
                });
            if let Some(mounted) = component {
                let props = new_props.take().expect("not yet used");
                let unchanged = mounted.props == props;
                if !unchanged {
                    mounted.props = props;
                }
                let old_output = self
                    .old_output
                    .as_ref()
                    .expect("a child that already exists was rendered before");
                let old = node_at(old_output, &instance.path).clone();
                let node = if unchanged {
                    old
                } else {
                    self.store.render(id, Some(old), Cause::Props)
                };
                result = Some((id, node));
            } else {
                self.store.unmount(id);
            }
        }
        let (id, node) = result.unwrap_or_else(|| {
            let props = new_props.take().expect("not yet used");
            let depth = self.store.instances[self.id as usize].depth + 1;
            let component = Box::new(Mounted { render, props });
            let id = self.store.allocate(self.id, depth, key, place, component);
            (id, self.store.render(id, None, Cause::Mounted))
        });
        self.rendered.push((id, node.clone()));
        node
    }

    /// Keeps the child component whose output was the `index`th child of the root of what this rendered last time, as it is,
    /// without rendering it or comparing its props, and gives back that output, as [`child_at`](Self::child_at) does for a child
    /// whose props are the same. It's for a caller that knows nothing it would be given has changed, as when it's the same
    /// description as last time: a long list where only a few have been put in or taken out costs what those do. It's `None`
    /// if that child of the root wasn't a child component's output, or that component has been claimed already.
    ///
    /// The kept child keeps its place and key, so the caller has to be giving it the same ones.
    pub fn keep_child(&mut self, index: usize) -> Option<Node> {
        if self.outputs.is_none() {
            let count = self.old_output.as_ref().map_or(0, |old| old.children.len());
            let mut outputs = vec![CLAIMED; count];
            for (at, &child) in self.old_children.iter().enumerate() {
                if child == CLAIMED {
                    continue;
                }
                if let [only] = self.store.instances[child as usize].path[..] {
                    if let Some(slot) = outputs.get_mut(only as usize) {
                        *slot = at as u32;
                    }
                }
            }
            self.outputs = Some(outputs);
        }
        let at = *self.outputs.as_ref()?.get(index)?;
        if at == CLAIMED {
            return None;
        }
        let child = *self.old_children.get(at as usize)?;
        if child == CLAIMED {
            return None;
        }
        self.old_children[at as usize] = CLAIMED;
        let node = self.old_output.as_ref()?.children[index].clone();
        self.rendered.push((child, node.clone()));
        Some(node)
    }

    /// Finds the child from the last render with this identity, and removes it from
    /// the list of children still to be claimed.
    fn claim_child(&mut self, key: &Option<Key>, place: u64) -> Option<u32> {
        while self.old_children.get(self.cursor) == Some(&CLAIMED) {
            self.cursor += 1;
        }
        // Children mostly stay in order. Removing one leaves it where the next is expected, so
        // look just past it, or a list would pay for a hash map because one row went away.
        if self.lookup.is_none() {
            let nearby = self.old_children.len().min(self.cursor + NEARBY_CHILDREN);
            for index in self.cursor..nearby {
                let child = self.old_children[index];
                if child == CLAIMED {
                    continue;
                }
                let candidate = &self.store.instances[child as usize];
                if candidate.place == place && candidate.key == *key {
                    self.old_children[index] = CLAIMED;
                    self.skipped |= index != self.cursor;
                    self.cursor = index + 1;
                    return Some(child);
                }
            }
        } else if self.old_children.get(self.cursor).is_some_and(|&child| {
            let candidate = &self.store.instances[child as usize];
            candidate.place == place && candidate.key == *key
        }) {
            let child = std::mem::replace(&mut self.old_children[self.cursor], CLAIMED);
            self.cursor += 1;
            return Some(child);
        }
        // Everything has been claimed, unless some were passed over on the way.
        if self.cursor >= self.old_children.len() && !self.skipped {
            return None;
        }
        if self.lookup.is_none() {
            let mut lookup =
                HashMap::with_capacity_and_hasher(self.old_children.len(), RandomState::default());
            for (index, &child) in self.old_children.iter().enumerate() {
                if child != CLAIMED {
                    let instance = &self.store.instances[child as usize];
                    lookup.insert((instance.key.clone(), instance.place), index);
                }
            }
            self.lookup = Some(lookup);
        }
        let lookup = self.lookup.as_mut().expect("just built");
        let index = lookup.remove(&(key.clone(), place))?;
        // It may have been claimed in order since this was made, and a key can be used twice by mistake.
        match std::mem::replace(&mut self.old_children[index], CLAIMED) {
            CLAIMED => None,
            child => Some(child),
        }
    }

    /// Takes the next hook's slot, creating it on the first render.
    fn hook<T: 'static>(&mut self, create: impl FnOnce() -> T) -> usize {
        let hook = self.next_hook;
        self.next_hook += 1;
        let hooks = &mut self.store.instances[self.id as usize].hooks;
        if hook == hooks.len() {
            assert!(
                self.first_render,
                "a component must call the same hooks on every render"
            );
            hooks.reserve_exact(1);
            hooks.push(Box::new(create()) as Box<dyn Any>);
        }
        hook
    }

    fn slot<T: 'static>(&self, hook: usize) -> &T {
        self.store.instances[self.id as usize].hooks[hook]
            .downcast_ref()
            .expect("a component must call the same hooks on every render")
    }
}
