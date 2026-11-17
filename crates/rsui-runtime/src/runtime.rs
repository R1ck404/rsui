use std::any::Any;
use std::fmt;
use std::ops::{Index, IndexMut};
use std::time::{Duration, Instant};

use rsui_core::{ActionKind, Element, IntoNode, Key, Node, Patch, Path, PropValue, Text, diff_at};
use thin_vec::ThinVec;
use triomphe::Arc;

use crate::cx::{Cx, Updates};
use crate::event::Event;
use crate::remote::Inbox;
use crate::trace::{Cause, Handled, Trace, Tracer};

/// How many times one flush may render and run effects before it gives up, since
/// only effects that keep changing state would ever need this many.
const MAX_ROUNDS: usize = 64;

pub(crate) type Cleanup = Box<dyn FnOnce(&mut Updates<'_>)>;
pub(crate) type HandlerFn = Box<dyn FnMut(&mut Updates<'_>, Option<&PropValue>)>;
pub(crate) type EffectFn = Box<dyn FnOnce(&mut Updates<'_>) -> Option<Cleanup>>;

/// A component with its props, with their types erased.
pub(crate) trait AnyComponent {
    fn render(&self, cx: &mut Cx<'_>) -> Node;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

pub(crate) struct Mounted<P, R> {
    pub(crate) render: fn(&mut Cx<'_>, &P) -> R,
    pub(crate) props: P,
}

impl<P: 'static, R: IntoNode + 'static> AnyComponent for Mounted<P, R> {
    fn render(&self, cx: &mut Cx<'_>) -> Node {
        (self.render)(cx, &self.props).into_node()
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

pub(crate) struct Handler {
    pub(crate) key: Key,
    pub(crate) action: ActionKind,
    pub(crate) run: HandlerFn,
}

/// One mounted component.
///
/// An instance doesn't hold its output. Only the tree does, so the nodes on the way
/// to a changed component have one owner and can be updated in place.
pub(crate) struct Instance {
    /// Counts reuses of this slot, so a [`State`](crate::State) can tell it's stale.
    pub(crate) generation: u32,
    pub(crate) alive: bool,
    pub(crate) dirty: bool,
    pub(crate) parent: u32,
    pub(crate) depth: u32,
    /// Where this child is among its siblings: the position of the call that made it among unkeyed ones, or what the
    /// parent said with [`Cx::child_at`](crate::Cx::child_at). Keyed children are told apart by their key within it.
    pub(crate) place: u64,
    pub(crate) key: Option<Key>,
    /// Empty only while the instance is rendering, or after it's unmounted.
    pub(crate) component: Option<Box<dyn AnyComponent>>,
    pub(crate) hooks: ThinVec<Box<dyn Any>>,
    pub(crate) children: ThinVec<u32>,
    /// Whether children are in the order they appear in the output.
    pub(crate) children_in_order: bool,
    pub(crate) handlers: ThinVec<Handler>,
    pub(crate) cleanups: ThinVec<(u32, Cleanup)>,
    /// Where this instance's output is inside its parent's output.
    pub(crate) path: Path,
}

const NO_PARENT: u32 = u32::MAX;

/// How many instances are stored together. Growing never moves existing instances,
/// so there's no moment with two copies of them, and little capacity goes unused.
const SLAB_CHUNK: usize = 256;

/// Instances by id, in chunks of [`SLAB_CHUNK`].
#[derive(Default)]
pub(crate) struct Slab {
    chunks: Vec<Vec<Instance>>,
    len: usize,
}

impl Slab {
    fn push(&mut self, instance: Instance) {
        if self.len % SLAB_CHUNK == 0 {
            self.chunks.push(Vec::with_capacity(SLAB_CHUNK));
        }
        self.chunks
            .last_mut()
            .expect("a chunk was just added if needed")
            .push(instance);
        self.len += 1;
    }

    pub(crate) fn get(&self, id: usize) -> Option<&Instance> {
        (id < self.len).then(|| &self[id])
    }

    pub(crate) fn get_mut(&mut self, id: usize) -> Option<&mut Instance> {
        (id < self.len).then(|| &mut self[id])
    }
}

impl Index<usize> for Slab {
    type Output = Instance;

    #[inline]
    fn index(&self, id: usize) -> &Instance {
        &self.chunks[id / SLAB_CHUNK][id % SLAB_CHUNK]
    }
}

impl IndexMut<usize> for Slab {
    #[inline]
    fn index_mut(&mut self, id: usize) -> &mut Instance {
        &mut self.chunks[id / SLAB_CHUNK][id % SLAB_CHUNK]
    }
}

pub(crate) struct PendingEffect {
    pub(crate) instance: u32,
    pub(crate) generation: u32,
    pub(crate) hook: u32,
    pub(crate) run: EffectFn,
}

/// Everything except the tree, so rendering can't touch the tree it's replacing.
#[derive(Default)]
pub(crate) struct Store {
    /// Which runtime this is, among any others on this thread.
    pub(crate) id: u32,
    pub(crate) instances: Slab,
    free: Vec<u32>,
    pub(crate) dirty: Vec<u32>,
    pub(crate) effects: Vec<PendingEffect>,
    /// How many of the effects being run now are still to run.
    pub(crate) effects_left: usize,
    cleanups: Vec<Cleanup>,
    pub(crate) rejected: Option<Text>,
    pub(crate) inbox: std::sync::Arc<Inbox>,
    renders: u64,
    /// Components with something moving, which render again on each [`Runtime::tick`].
    pub(crate) animating: Vec<u32>,
    /// Whether things move, or go straight to where they're going.
    pub(crate) animations: bool,
    /// What time it is as far as animations go, while any are running.
    pub(crate) now: Option<Instant>,
    /// Whether `now` is a frame's, from [`Runtime::tick`], with nothing heard since: once something is (an input between
    /// frames), the next flush takes the time it really is.
    pub(crate) ticked: bool,
    /// Whether time is only ever what [`Runtime::tick`] says, as a test that keeps its own clock wants (Blink's virtual time):
    /// nothing reads the real clock for what moves, between frames or when something starts moving with nothing else moving.
    pub(crate) own_clock: bool,
    /// The last frame's time, from [`Runtime::tick`], which is the time on that clock.
    pub(crate) last_tick: Option<Instant>,
    /// What's told once each round of effects has run, which is when what's queued its own effects runs them, in its own order.
    pub(crate) on_effects_done: Option<fn(&mut Updates<'_>)>,
    /// What's told if effects never settle.
    pub(crate) on_runaway: Option<fn(&mut Updates<'_>)>,
    /// What's been asked to have the keyboard, for whatever has one to give.
    pub(crate) focus_wanted: Option<Key>,
    /// Whether that's to show that it has the keyboard, if it was said, as a page's `focus({ focusVisible })` says.
    pub(crate) focus_visible_wanted: Option<bool>,
    /// Whether the pending focus must leave every scroll position unchanged.
    pub(crate) focus_prevent_scroll_wanted: bool,
    /// What part of a field's text has been asked to be selected, as UTF-16 offsets, as a page's `setSelectionRange()` has them.
    pub(crate) select_wanted: Option<(Key, u32, u32)>,
    /// What's been asked to have the pointer (`Some(None)` to let it go), as a page's `setPointerCapture()` asks.
    pub(crate) capture_wanted: Option<Option<Key>>,
    /// Whether a handler has said what it was given is for whatever's around it too.
    pub(crate) passed_on: bool,
    /// What the window should be called, for whatever has one.
    pub(crate) title_wanted: Option<Text>,
    /// What to put on the clipboard, for whatever has one.
    pub(crate) copy_wanted: Option<Text>,
    /// Whether something wants what's on the clipboard, and what to tell.
    pub(crate) paste_wanted: bool,
    pub(crate) on_paste: Option<fn(&mut Updates<'_>, Option<&str>)>,
    /// What's been asked to be opened by whatever opens things, for whatever can do that.
    pub(crate) urls_wanted: Vec<Text>,
    /// Whether the window's title bar should be dark, if that's been said.
    pub(crate) dark_title_bar_wanted: Option<bool>,
    /// Whether text has been asked to be drawn the way the system draws it.
    pub(crate) system_glyphs: bool,
    /// What to tell when the system goes between dark and light.
    pub(crate) on_color_scheme: Option<fn(&mut Updates<'_>, bool)>,
    /// What to tell when the window's size changes.
    pub(crate) on_resize: Option<fn(&mut Updates<'_>, f32, f32)>,
    /// What to tell when the window's been laid out.
    pub(crate) on_laid_out: Option<fn(&mut Updates<'_>)>,
    /// What's rendered and why, while something's looking.
    pub(crate) tracer: Option<Box<Tracer>>,
    /// What came with what's being handled now, if anything did.
    pub(crate) event: Option<Event>,
    /// Whether a handler has said that nothing further along should hear about what it was given.
    pub(crate) stopped: bool,
    /// Whether a handler has said that what would happen by itself shouldn't.
    pub(crate) prevented: bool,
    /// What hears about things for the app as a whole, as a page's `document` does.
    pub(crate) on_document_event: Option<fn(&mut Updates<'_>, &str)>,
    /// What's to be told that something's scrolled, once the frame that shows it has been drawn, at most once each however often
    /// it moved before then, as a browser fires `scroll` (see [`Runtime::deliver_scrolled`]). Each is for a piece of state, by
    /// its instance and hook, in the order they first moved.
    pub(crate) scrolled: Vec<((u32, u32), std::rc::Rc<dyn Fn(&mut Updates<'_>)>)>,
}

impl Store {
    /// What time it is for animations.
    pub(crate) fn clock(&self) -> Instant {
        self.now.unwrap_or_else(Instant::now)
    }

    pub(crate) fn allocate(
        &mut self,
        parent: u32,
        depth: u32,
        key: Option<Key>,
        place: u64,
        component: Box<dyn AnyComponent>,
    ) -> u32 {
        let reused = self.free.pop();
        let id = reused.unwrap_or(self.instances.len as u32);
        let generation = reused.map_or(0, |id| self.instances[id as usize].generation);
        let instance = Instance {
            generation,
            alive: true,
            dirty: false,
            parent,
            depth,
            place,
            key,
            component: Some(component),
            hooks: ThinVec::new(),
            children: ThinVec::new(),
            children_in_order: true,
            handlers: ThinVec::new(),
            cleanups: ThinVec::new(),
            path: Path::from(&[][..]),
        };
        if reused.is_some() {
            self.instances[id as usize] = instance;
        } else {
            self.instances.push(instance);
        }
        id
    }

    /// Removes an instance and everything below it. Its cleanups run after the
    /// current render, once the tree no longer shows it.
    pub(crate) fn unmount(&mut self, id: u32) {
        let instance = &mut self.instances[id as usize];
        let children = std::mem::take(&mut instance.children);
        let cleanups = std::mem::take(&mut instance.cleanups);
        instance.alive = false;
        instance.dirty = false;
        instance.generation = instance.generation.wrapping_add(1);
        instance.component = None;
        instance.key = None;
        instance.hooks = ThinVec::new();
        instance.handlers = ThinVec::new();
        self.free.push(id);
        self.cleanups
            .extend(cleanups.into_iter().map(|(_, cleanup)| cleanup));
        for child in children {
            self.unmount(child);
        }
    }

    pub(crate) fn mark_dirty(&mut self, id: u32) {
        let instance = &mut self.instances[id as usize];
        if !instance.dirty {
            instance.dirty = true;
            self.dirty.push(id);
        }
    }

    /// Renders an instance, given its last output if it has one.
    pub(crate) fn render(&mut self, id: u32, old_output: Option<Node>, cause: Cause) -> Node {
        let traced = match self.tracer.as_mut() {
            None => None,
            Some(tracer) => {
                let instance = &self.instances[id as usize];
                let parent = (instance.parent != NO_PARENT).then_some(instance.parent);
                tracer.start(id, instance.generation, parent, instance.depth, cause);
                Some((Instant::now(), old_output.clone()))
            }
        };
        let instance = &mut self.instances[id as usize];
        instance.dirty = false;
        instance.handlers.clear();
        let component = instance
            .component
            .take()
            .expect("a component isn't rendered from inside itself");
        let old_children = std::mem::take(&mut instance.children);

        let mut cx = Cx::new(self, id, old_output, old_children, cause);
        let mut output = component.render(&mut cx);
        let (children, leftover) = cx.finish();

        for child in leftover {
            self.unmount(child);
        }
        let children = self.place_children(id, &output, children);
        let instance = &mut self.instances[id as usize];
        instance.component = Some(component);
        instance.children = children;
        if output.key.is_none() && instance.key.is_some() {
            Arc::make_mut(&mut output).key = instance.key.clone();
        }
        self.renders += 1;
        if let Some((started, old)) = traced {
            let changed = old.is_none_or(|old| !Arc::ptr_eq(&old, &output) && *old != *output);
            if let Some(tracer) = self.tracer.as_mut() {
                tracer.finish(started, changed);
            }
        }
        output
    }

    /// Records where each child ended up in the output, and unmounts any child that
    /// didn't end up there, since nothing could show it. This only walks the nodes
    /// this component made itself, because it stops at its children.
    ///
    /// `rendered` holds on to each child's node, so that none of them can be freed
    /// and have its address reused by a node in the output.
    fn place_children(
        &mut self,
        id: u32,
        output: &Node,
        rendered: Vec<(u32, Node)>,
    ) -> ThinVec<u32> {
        if rendered.is_empty() {
            self.instances[id as usize].children_in_order = true;
            return ThinVec::new();
        }
        // Children almost always appear in the order they rendered in, which only
        // takes one comparison per node to confirm.
        let mut in_order = InOrder {
            store: self,
            rendered: &rendered,
            next: 0,
            path: Vec::new(),
        };
        in_order.visit(output);
        let all_in_order = in_order.next == rendered.len();
        self.instances[id as usize].children_in_order = all_in_order;

        // Debug builds always take this slower path, as it also checks that no
        // child's output appears twice.
        if all_in_order && !cfg!(debug_assertions) {
            return rendered.iter().map(|(child, _)| *child).collect();
        }
        let mut roots: Vec<(*const Element, u32)> = rendered
            .iter()
            .enumerate()
            .map(|(index, (_, node))| (Arc::as_ptr(node), index as u32))
            .collect();
        roots.sort_unstable();
        let mut any_order = AnyOrder {
            store: self,
            roots: &roots,
            rendered: &rendered,
            placed: vec![false; rendered.len()],
            path: Vec::new(),
        };
        any_order.visit(output);
        let placed = any_order.placed;

        let mut children = ThinVec::with_capacity(rendered.len());
        for (index, (child, _)) in rendered.iter().enumerate() {
            if placed[index] {
                children.push(*child);
            } else {
                self.unmount(*child);
            }
        }
        children
    }

    /// Every index from the root down to `id`.
    fn absolute_path(&self, id: u32) -> Vec<u32> {
        let mut sections = Vec::new();
        let mut current = id;
        while current != NO_PARENT {
            let instance = &self.instances[current as usize];
            sections.push(&instance.path);
            current = instance.parent;
        }
        sections
            .into_iter()
            .rev()
            .flat_map(|section| section.iter().copied())
            .collect()
    }
}

/// Places children by walking the output, expecting them in the order they rendered.
struct InOrder<'a> {
    store: &'a mut Store,
    rendered: &'a [(u32, Node)],
    next: usize,
    path: Vec<u32>,
}

impl InOrder<'_> {
    fn visit(&mut self, node: &Node) {
        let Some((child, root)) = self.rendered.get(self.next) else {
            return;
        };
        if Arc::ptr_eq(root, node) {
            self.store.instances[*child as usize].path = Path::from(self.path.as_slice());
            self.next += 1;
            return;
        }
        for (position, child) in node.children.iter().enumerate() {
            self.path.push(position as u32);
            self.visit(child);
            self.path.pop();
        }
    }
}

/// Places children by walking the output and looking up each node's address.
struct AnyOrder<'a> {
    store: &'a mut Store,
    /// Each child's root node, sorted by address.
    roots: &'a [(*const Element, u32)],
    rendered: &'a [(u32, Node)],
    placed: Vec<bool>,
    path: Vec<u32>,
}

impl AnyOrder<'_> {
    fn visit(&mut self, node: &Node) {
        let address = Arc::as_ptr(node);
        if let Ok(found) = self.roots.binary_search_by_key(&address, |root| root.0) {
            let index = self.roots[found].1 as usize;
            assert!(
                !self.placed[index],
                "a component's output can only appear once in its parent's output"
            );
            self.placed[index] = true;
            let child = self.rendered[index].0;
            self.store.instances[child as usize].path = Path::from(self.path.as_slice());
            return;
        }
        for (position, child) in node.children.iter().enumerate() {
            self.path.push(position as u32);
            self.visit(child);
            self.path.pop();
        }
    }
}

/// Why an action couldn't be carried out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
    /// No node exists at that path.
    NoSuchNode,
    /// The node doesn't offer that action.
    ActionNotAvailable,
    /// The node offers that action, but nothing handles it.
    NoHandler,
    /// The handler refused the action, for this reason.
    Rejected(Text),
}

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchNode => f.write_str("no node exists at that path"),
            Self::ActionNotAvailable => f.write_str("the node doesn't offer that action"),
            Self::NoHandler => f.write_str("nothing handles that action"),
            Self::Rejected(reason) => write!(f, "the action was rejected: {reason}"),
        }
    }
}

impl std::error::Error for DispatchError {}

/// A mounted UI: the tree, and the components that produce it.
/// What changes sooner than this needs every frame there can be.
const MOTION_EVERY_FRAME: Duration = Duration::from_millis(12);

pub struct Runtime {
    store: Store,
    root_instance: u32,
    tree: Node,
    runaways: u64,
    /// Props that are moving by themselves, as CSS transitions and animations do.
    motions: crate::motion::Motions,
    /// Whether what was there to begin with has been looked at for what moves, which waits until it's known whether
    /// things move at all, as a window says so after it's made this.
    first_seen: bool,
    /// Whether this has been made, before which nothing's looked at for what moves.
    made: bool,
}

impl Runtime {
    /// Mounts a component as the root of the UI, and runs its first effects.
    pub fn new<P: 'static, R: IntoNode + 'static>(
        render: fn(&mut Cx<'_>, &P) -> R,
        props: P,
    ) -> Self {
        Self::made(render, props, false)
    }

    /// Like [`new`](Self::new), with what renders kept from the start, as [`set_tracing`](Self::set_tracing) has it.
    pub fn new_tracing<P: 'static, R: IntoNode + 'static>(
        render: fn(&mut Cx<'_>, &P) -> R,
        props: P,
    ) -> Self {
        Self::made(render, props, true)
    }

    fn made<P: 'static, R: IntoNode + 'static>(
        render: fn(&mut Cx<'_>, &P) -> R,
        props: P,
        tracing: bool,
    ) -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
        let mut store = Store {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            tracer: tracing.then(Box::default),
            ..Store::default()
        };
        let root_instance =
            store.allocate(NO_PARENT, 0, None, 0, Box::new(Mounted { render, props }));
        let tree = store.render(root_instance, None, Cause::Mounted);
        let mut runtime = Self {
            store,
            root_instance,
            tree,
            runaways: 0,
            motions: crate::motion::Motions::default(),
            first_seen: false,
            made: false,
        };
        runtime.flush();
        runtime.made = true;
        runtime
    }

    /// Looks at what was there to begin with for what moves: animations start from there, or go straight to where
    /// they'd finish if things don't move.
    fn see_first(&mut self, patches: &mut Vec<Patch>) {
        self.first_seen = true;
        let now = self.store.clock();
        let old = self.tree.clone();
        let mut tree = self.tree.clone();
        self.motions
            .rendered(&[], None, &mut tree, now, self.store.animations);
        if !Arc::ptr_eq(&old, &tree) {
            diff_at(&[], &old, &tree, patches);
            self.tree = tree;
        }
    }

    /// The current tree. Holding on to a clone is cheap, and keeps that version
    /// intact while the runtime moves on.
    pub fn root(&self) -> &Node {
        &self.tree
    }

    /// How many times any component has rendered so far.
    pub fn renders(&self) -> u64 {
        self.store.renders
    }

    /// How many places there are for components, how many are mounted, and how many effects and cleanups are waiting,
    /// for seeing that none of them grows for good.
    #[doc(hidden)]
    pub fn sizes(&self) -> [usize; 4] {
        let instances = &self.store.instances;
        let mounted = (0..instances.len)
            .filter(|id| instances[*id].alive)
            .count();
        [
            instances.len,
            mounted,
            self.store.effects.len(),
            self.store.cleanups.len(),
        ]
    }

    /// Carries out an action on the node at `path`. State changes take effect at
    /// the next [`flush`](Self::flush), so several actions can share one update.
    ///
    /// The handler comes from the component that made the node, or failing that the
    /// nearest component above it with a handler for that key and action.
    pub fn dispatch(
        &mut self,
        path: &[u32],
        action: ActionKind,
        input: Option<PropValue>,
    ) -> Result<(), DispatchError> {
        // Heard between frames, so what it changes is at the time it really is (see `flush`).
        self.store.ticked = false;
        let mut node = &self.tree;
        for &index in path {
            node = node
                .children
                .get(index as usize)
                .ok_or(DispatchError::NoSuchNode)?;
        }
        if !node.actions.contains(action) {
            return Err(DispatchError::ActionNotAvailable);
        }
        let key = node.key.clone().ok_or(DispatchError::NoHandler)?;
        let owner = self.store.owner_of(self.root_instance, path);
        let (holder, index) = self
            .store
            .find_handler(owner, &key, action)
            .ok_or(DispatchError::NoHandler)?;

        // Handlers only change state, never the handler list, so this slot stays
        // valid until the handler is put back.
        let handlers = &mut self.store.instances[holder as usize].handlers;
        let placeholder: HandlerFn = Box::new(|_, _| {});
        let mut run = std::mem::replace(&mut handlers[index].run, placeholder);
        let started = self.store.tracer.is_some().then(Instant::now);
        run(&mut Updates::new(&mut self.store), input.as_ref());
        self.store.instances[holder as usize].handlers[index].run = run;

        let rejected = self.store.rejected.take();
        if let (Some(started), Some(tracer)) = (started, self.store.tracer.as_mut()) {
            tracer.trace.handled.push(Handled {
                action,
                key,
                path: path.to_vec(),
                took: started.elapsed(),
                rejected: rejected.is_some(),
                note: tracer.handler_note.take(),
            });
        }
        match rejected {
            Some(reason) => Err(DispatchError::Rejected(reason)),
            None => Ok(()),
        }
    }

    /// Starts or stops keeping what renders, why, and how long it takes, for [`take_trace`](Self::take_trace). It's off
    /// to begin with, and costs nothing then.
    pub fn set_tracing(&mut self, on: bool) {
        if on != self.store.tracer.is_some() {
            self.store.tracer = on.then(Box::default);
        }
    }

    pub fn is_tracing(&self) -> bool {
        self.store.tracer.is_some()
    }

    /// Which runtime this is, among those on this thread, as [`Cx::runtime_id`] has it.
    pub fn id(&self) -> u32 {
        self.store.id
    }

    /// What's rendered, and what's been carried out, since this last said, while tracing. Each render says where its
    /// component's output is now, if it's still there.
    pub fn take_trace(&mut self) -> Trace {
        let Some(tracer) = self.store.tracer.as_mut() else {
            return Trace::default();
        };
        let mut trace = std::mem::take(&mut tracer.trace);
        for render in &mut trace.renders {
            render.path = self.component_path(render.component, render.generation);
        }
        trace
    }

    /// The component whose output has the node at `path` in it, nearest to it, with its generation, as [`Rendered`]
    /// has them.
    ///
    /// [`Rendered`]: crate::Rendered
    pub fn component_at(&self, path: &[u32]) -> (u32, u32) {
        let id = self.store.owner_of(self.root_instance, path);
        (id, self.store.instances[id as usize].generation)
    }

    /// Where a component's output is in the tree, if it's still mounted.
    pub fn component_path(&self, component: u32, generation: u32) -> Option<Vec<u32>> {
        let instance = self.store.instances.get(component as usize)?;
        (instance.alive && instance.generation == generation)
            .then(|| self.store.absolute_path(component))
    }

    /// The component that rendered this one, unless it's the root or gone.
    pub fn component_parent(&self, component: u32, generation: u32) -> Option<(u32, u32)> {
        let instance = self.store.instances.get(component as usize)?;
        if !instance.alive || instance.generation != generation || instance.parent == NO_PARENT {
            return None;
        }
        let parent = instance.parent;
        Some((parent, self.store.instances[parent as usize].generation))
    }

    /// [`dispatch`](Self::dispatch), with what came with it, which its handler reads with [`Updates::event`]. `event`'s
    /// `current` is set to `path`. It's given back as it was, for whatever hears about it next.
    pub fn dispatch_with(
        &mut self,
        path: &[u32],
        action: ActionKind,
        input: Option<PropValue>,
        event: &mut Event,
    ) -> Result<(), DispatchError> {
        // Heard between frames, so what it changes is at the time it really is (see `flush`).
        self.store.ticked = false;
        event.current.clear();
        event.current.extend_from_slice(path);
        self.store.event = Some(std::mem::take(event));
        let result = self.dispatch(path, action, input);
        if let Some(given) = self.store.event.take() {
            *event = given;
        }
        result
    }

    /// Tells what [`Cx::on_document_event`] gave about something that happened, as the app as a whole hears it, which `event`
    /// says the rest of. `kind` is what the web calls it, as `"pointerdown"`. Whether it should go any further is
    /// [`take_stopped`](Self::take_stopped), and whether what it would do by itself should happen is
    /// [`take_prevented`](Self::take_prevented).
    pub fn deliver_document_event(&mut self, kind: &str, event: &mut Event) {
        let Some(told) = self.store.on_document_event else {
            return;
        };
        event.current.clear();
        self.store.event = Some(std::mem::take(event));
        told(&mut Updates::new(&mut self.store), kind);
        if let Some(given) = self.store.event.take() {
            *event = given;
        }
    }

    /// Whether a handler that's run since this last said called [`Updates::stop_propagation`].
    pub fn take_stopped(&mut self) -> bool {
        std::mem::take(&mut self.store.stopped)
    }

    /// Whether a handler that's run since this last said called [`Updates::prevent_default`].
    pub fn take_prevented(&mut self) -> bool {
        std::mem::take(&mut self.store.prevented)
    }

    /// Renders everything whose state changed and runs effects, until nothing is
    /// left to do, and returns the patches for what changed. Parents render before
    /// their children, so a child they remove or update doesn't render twice.
    ///
    /// # Panics
    ///
    /// If effects keep changing state, since that would never finish.
    pub fn flush(&mut self) -> Vec<Patch> {
        // Everything one flush sets moving starts at the same time, as what one restyle sets off does on a page, so that what
        // takes as long ends together and is told in the page's order (see `advance` in motion.rs): the button the keyboard
        // left, drawn after the one it came to, would otherwise end a millisecond later.
        let pinned = self.store.now.is_none();
        if pinned {
            let clock = if self.store.own_clock { self.store.last_tick } else { None };
            self.store.now = Some(clock.unwrap_or_else(Instant::now));
        } else if !self.store.ticked && !self.store.own_clock {
            // Between frames, time goes on for what's moving, as a browser's animation clock does outside a frame (Blink's
            // `AnimationClock` updates itself then): a change before the next frame finds a transition as far along as it's
            // got by now, rather than where the last frame had it, and turns back from there.
            self.store.now = self.store.now.map(|was| was.max(Instant::now()));
            self.store.ticked = true;
        }
        let patches = if self.store.tracer.is_some() {
            let started = Instant::now();
            let patches = self.flush_untimed();
            if let Some(tracer) = self.store.tracer.as_mut() {
                tracer.trace.flushing += started.elapsed();
            }
            patches
        } else {
            self.flush_untimed()
        };
        if pinned {
            self.store.now = None;
        }
        patches
    }

    fn flush_untimed(&mut self) -> Vec<Patch> {
        let mut patches = Vec::new();
        // Only what has arrived by now. Other threads can keep sending for as long as they like,
        // which must neither hold this up nor count as effects that never settle. What arrives
        // from here on wakes the host, for the next flush.
        // What arrives can't change the tree, only say what's to change, so it's run with the tree as
        // it is: a page's timer runs here, and what it asks about the page is the tree its own code
        // last drew, rather than the last one laid out, which can be from before that, or none at
        // all before the first layout, as a timer that came due while the first frame was being
        // made found (`what_doesn_t_fit_is_a_styled` in `rsui-tsx/tests/native_elements.rs`, under
        // load). It's also this runtime's tree rather than one an app before it on this thread left.
        crate::event::set_current_tree(&self.tree);
        self.run_jobs();
        // While the tree's being changed, nothing else holds it, so that changing it doesn't copy it.
        crate::event::forget_current_tree();
        if self.made && !self.first_seen {
            self.see_first(&mut patches);
        }
        self.motions.take_kept(&mut patches);
        if !self.motions.is_empty() {
            let now = self.store.clock();
            self.motions.advance(&mut self.tree, now, &mut patches);
        }
        for _ in 0..MAX_ROUNDS {
            if !self.motions.ended.is_empty() {
                self.tell_motions_ended();
            }
            if self.store.dirty.is_empty()
                && self.store.effects.is_empty()
                && self.store.cleanups.is_empty()
            {
                if !self.motions.held.is_empty() {
                    // What's started moving, now that everything this changed has been drawn, as a page tells it.
                    self.tell_motions_started();
                    continue;
                }
                // With nothing moving, the next thing to move starts from whenever that is.
                if self.store.animating.is_empty() && self.motions.is_empty() {
                    self.store.now = None;
                }
                crate::event::set_current_tree(&self.tree);
                return patches;
            }
            let mut batch = std::mem::take(&mut self.store.dirty);
            batch.sort_unstable_by_key(|&id| (self.store.instances[id as usize].depth, id));
            for id in batch {
                let instance = &self.store.instances[id as usize];
                if instance.alive && instance.dirty {
                    self.update(id, &mut patches);
                }
            }
            crate::event::set_current_tree(&self.tree);
            self.run_effects();
            crate::event::forget_current_tree();
        }
        self.stop_runaway_effects(&mut patches);
        crate::event::set_current_tree(&self.tree);
        patches
    }

    /// Tells what's finished moving, and offers to hear about it, that it has.
    #[cold]
    fn tell_motions_ended(&mut self) {
        let action = *crate::motion::MOTION_END;
        for (path, key, what) in std::mem::take(&mut self.motions.ended) {
            // What's started or stopped waits until everything that's changing has been drawn: see `tell_motions_started`.
            if what.as_str().starts_with("transitionstart ") || what.as_str().starts_with("transitioncancel ") {
                self.motions.held.push((path, key, what));
                continue;
            }
            let same = crate::motion::find(&self.tree, &path)
                .is_some_and(|node| node.key == key && node.actions.contains(action));
            if same {
                // It's only being told, and what it does about that is up to it.
                let _ = self.dispatch(&path, action, Some(PropValue::from(what)));
            }
        }
    }

    /// Tells what's started and stopped moving once everything a change changed has been drawn, in the order of the tree, as a
    /// page tells what one restyle set off: when the keyboard goes from one button to the next, the one it left before the one it
    /// came to if that's first in the page, though rsui draws the one it came to first. Each element's own are in the order they
    /// came (see `start_order` in motion.rs).
    #[cold]
    fn tell_motions_started(&mut self) {
        let mut held = std::mem::take(&mut self.motions.held);
        held.sort_by(|a, b| a.0.cmp(&b.0));
        let action = *crate::motion::MOTION_END;
        for (path, key, what) in held {
            let same = crate::motion::find(&self.tree, &path)
                .is_some_and(|node| node.key == key && node.actions.contains(action));
            if same {
                let _ = self.dispatch(&path, action, Some(PropValue::from(what)));
            }
        }
    }

    /// Effects that keep changing state would otherwise go on for ever, and that's a mistake in one part of an
    /// app, which shouldn't take the rest of it down. What's waiting is dropped, so that what's on screen is
    /// what it had got to, and everything else still works. Whatever happens next starts afresh.
    #[cold]
    fn stop_runaway_effects(&mut self, patches: &mut Vec<Patch>) {
        self.runaways += 1;
        eprintln!(
            "Effects kept changing state after {MAX_ROUNDS} rounds, so they were stopped. An effect is \
             probably setting state that makes it run again."
        );
        self.forget_what_is_waiting();
        if let Some(told) = self.store.on_runaway {
            told(&mut Updates::new(&mut self.store));
            // Once, to show what that changed. If it's what's running away, its effects don't get to run.
            let mut batch = std::mem::take(&mut self.store.dirty);
            batch.sort_unstable_by_key(|&id| (self.store.instances[id as usize].depth, id));
            for id in batch {
                let instance = &self.store.instances[id as usize];
                if instance.alive && instance.dirty {
                    self.update(id, patches);
                }
            }
            self.forget_what_is_waiting();
        }
    }

    fn forget_what_is_waiting(&mut self) {
        for id in std::mem::take(&mut self.store.dirty) {
            self.store.instances[id as usize].dirty = false;
        }
        self.store.effects.clear();
    }

    /// What's been asked to have the keyboard, by [`Updates::focus`] or [`Cx::focus`], since this last said.
    pub fn take_focus_wanted(&mut self) -> Option<Key> {
        self.store.focus_wanted.take()
    }

    /// Whether what was last asked to have the keyboard is to show that it has, if that was said ([`Updates::focus_showing`]).
    pub fn take_focus_visible_wanted(&mut self) -> Option<bool> {
        self.store.focus_visible_wanted.take()
    }

    /// Whether the pending focus was requested without revealing its target.
    pub fn take_focus_prevent_scroll_wanted(&mut self) -> bool {
        std::mem::take(&mut self.store.focus_prevent_scroll_wanted)
    }

    /// What's been asked to be selected in a field, by [`Updates::select_text`] or [`Cx::select_text`], since this last said.
    pub fn take_select_wanted(&mut self) -> Option<(Key, u32, u32)> {
        self.store.select_wanted.take()
    }

    /// What's been asked to have the pointer since this last said, by [`Updates::capture_pointer`]: `Some(None)` if it's to be let go.
    pub fn take_capture_wanted(&mut self) -> Option<Option<Key>> {
        self.store.capture_wanted.take()
    }

    /// What the window has been asked to be called, by [`Updates::set_title`], since this last said.
    pub fn take_title_wanted(&mut self) -> Option<Text> {
        self.store.title_wanted.take()
    }

    /// What's been asked to go on the clipboard, by [`Updates::copy_text`], since this last said.
    pub fn take_copy_wanted(&mut self) -> Option<Text> {
        self.store.copy_wanted.take()
    }

    /// Whether something's asked for what's on the clipboard, by [`Updates::ask_for_clipboard`], since this last said.
    pub fn take_paste_wanted(&mut self) -> bool {
        std::mem::take(&mut self.store.paste_wanted)
    }

    /// Passes on what's on the clipboard to whatever asked for it.
    pub fn deliver_clipboard(&mut self, text: Option<&str>) {
        if let Some(told) = self.store.on_paste {
            told(&mut Updates::new(&mut self.store), text);
        }
    }

    /// What's been asked to be opened, by [`Updates::open_url`], since this last said.
    pub fn take_urls_wanted(&mut self) -> Vec<Text> {
        std::mem::take(&mut self.store.urls_wanted)
    }

    /// What the window's title bar has been asked to be, by [`Updates::set_dark_title_bar`], since this
    /// last said.
    pub fn take_dark_title_bar_wanted(&mut self) -> Option<bool> {
        self.store.dark_title_bar_wanted.take()
    }

    /// Whether text has been asked to be drawn the way the system draws it, by [`Updates::draw_text_as_the_system_does`].
    pub fn system_glyphs(&self) -> bool {
        self.store.system_glyphs
    }

    /// Passes on that the system has gone dark or light to what asked to hear about that.
    pub fn deliver_color_scheme(&mut self, dark: bool) {
        if let Some(told) = self.store.on_color_scheme {
            told(&mut Updates::new(&mut self.store), dark);
        }
    }

    /// Passes on how big the window now is, in logical pixels, to what asked to hear about that.
    pub fn deliver_resize(&mut self, width: f32, height: f32) {
        if let Some(told) = self.store.on_resize {
            told(&mut Updates::new(&mut self.store), width, height);
        }
    }

    /// Says that the window's just been laid out, to what asked to hear about that, as what watches the size of something does.
    pub fn deliver_laid_out(&mut self) {
        if let Some(told) = self.store.on_laid_out {
            told(&mut Updates::new(&mut self.store));
        }
    }

    /// Whether something's been scrolled, and what hears about that hasn't been told yet (see
    /// [`Updates::tell_scrolled_later`]).
    pub fn scrolled_waiting(&self) -> bool {
        !self.store.scrolled.is_empty()
    }

    /// Tells what hears that something's scrolled, once for each that has, however often it moved, in the order they first
    /// moved: a browser fires `scroll` once a frame, after scrolling, rather than for each step. What they change is drawn
    /// in the next frame. Says how many were told.
    pub fn deliver_scrolled(&mut self) -> usize {
        let waiting = std::mem::take(&mut self.store.scrolled);
        let told = waiting.len();
        for (_, tell) in waiting {
            tell(&mut Updates::new(&mut self.store));
        }
        told
    }

    /// Whether the handler that just ran called [`Updates::pass_on`].
    pub fn take_passed_on(&mut self) -> bool {
        std::mem::take(&mut self.store.passed_on)
    }

    /// How many times effects have had to be stopped for not settling.
    pub fn runaways(&self) -> u64 {
        self.runaways
    }

    /// Whether anything is partway through moving, from [`Cx::animate`]. While it is, call
    /// [`tick`](Self::tick) for each frame, and [`flush`](Self::flush). Nothing needs calling after
    /// that, so something that's stopped moving costs nothing.
    pub fn is_animating(&self) -> bool {
        !self.store.animating.is_empty() || !self.motions.is_empty()
    }

    /// Whether something needs every frame there can be, rather than only now and then: a component that's moving, or
    /// something moving by itself that changes faster than that.
    pub fn moves_every_frame(&self) -> bool {
        !self.store.animating.is_empty()
            || self
                .next_motion_in()
                .is_some_and(|wait| wait < MOTION_EVERY_FRAME)
    }

    /// Display scheduling with conservative viewport eligibility from the host.
    /// Only infinite visual-only keyframes may be omitted; component animation,
    /// finite completion deadlines and motion timelines remain unchanged.
    pub fn moves_every_frame_in_view(&self, visible: impl Fn(&[u32], &Node) -> bool) -> bool {
        !self.store.animating.is_empty()
            || self.next_motion_in_view(visible).is_some_and(|wait| wait < MOTION_EVERY_FRAME)
    }

    /// How long until what's moving by itself, as a CSS transition does, next looks any different, if anything is.
    /// Something that creeps along only needs drawing that often.
    pub fn next_motion_in(&self) -> Option<Duration> {
        if self.motions.is_empty() {
            return None;
        }
        Some(self.motions.next_change(&self.tree, self.store.clock()))
    }

    /// Like `next_motion_in`, omitting display wakeups for proven offscreen
    /// infinite visual motion. Sampling and original start times are retained.
    pub fn next_motion_in_view(&self, visible: impl Fn(&[u32], &Node) -> bool) -> Option<Duration> {
        self.motions.next_change_in_view(&self.tree, self.store.clock(), visible)
    }

    /// Moves everything that's moving along to this time, ready for the next
    /// [`flush`](Self::flush).
    pub fn tick(&mut self, now: Instant) {
        self.store.now = Some(now);
        self.store.ticked = true;
        self.store.last_tick = Some(now);
        for id in std::mem::take(&mut self.store.animating) {
            if self.store.instances[id as usize].alive {
                self.store.mark_dirty(id);
            }
        }
    }

    /// Whether time, for what moves, is only ever what [`tick`](Self::tick) says, as for a test that keeps its own clock and
    /// looks at each frame: what's heard between two frames is then at the last one's time, however long it really took, and
    /// what starts moving with nothing else moving starts then too. Off, as in a window, those take the time it really is.
    pub fn set_own_clock(&mut self, on: bool) {
        self.store.own_clock = on;
    }

    /// Whether things move, or go straight to where they're going. They start off, as an agent or a
    /// test wants each result straight away. What's moving when they go off goes straight there.
    pub fn set_animations(&mut self, on: bool) {
        self.store.animations = on;
        if !on {
            self.tick(Instant::now());
            self.motions.stop(&mut self.tree);
        } else if self.made && !self.first_seen {
            let mut patches = Vec::new();
            self.see_first(&mut patches);
            self.motions.keep(patches);
        }
    }

    /// Whether another thread has queued a change, or a task from
    /// [`Updates::spawn`] or [`Updates::after`] is still running.
    pub fn has_background_work(&self) -> bool {
        self.store.inbox.is_busy()
    }

    /// Asks to be told when there may be something to [`flush`](Self::flush). `waker`
    /// is called on the thread that caused it, so it should only signal the thread
    /// running this runtime, for example by posting a message to its window.
    pub fn set_waker(&mut self, waker: impl Fn() + Send + Sync + 'static) {
        self.store.inbox.set_waker(std::sync::Arc::new(waker));
    }

    /// Blocks until there may be something to flush, all background work has
    /// finished, or `timeout` passes.
    pub fn wait(&self, timeout: Duration) {
        self.store.inbox.wait(Instant::now() + timeout);
    }

    /// Keeps flushing until all background work has finished, or `timeout` passes.
    /// Returns the patches, and whether everything finished.
    ///
    /// A task that never ends, such as a repeating timer, means this always takes
    /// the whole timeout.
    pub fn settle(&mut self, timeout: Duration) -> (Vec<Patch>, bool) {
        let deadline = Instant::now() + timeout;
        let mut patches = self.flush();
        while self.has_background_work() {
            if Instant::now() >= deadline {
                return (patches, false);
            }
            self.store.inbox.wait(deadline);
            patches.extend(self.flush());
        }
        (patches, true)
    }

    fn run_jobs(&mut self) {
        for job in self.store.inbox.take_jobs() {
            job(&mut Updates::new(&mut self.store));
        }
        // Rejecting only means something for an action someone is waiting on.
        self.store.rejected = None;
    }

    /// Renders one component again and puts its new output in the tree.
    fn update(&mut self, id: u32, patches: &mut Vec<Patch>) {
        let path = self.store.absolute_path(id);
        let old = node_at(&self.tree, &path).clone();
        // What renders can look through the tree as it is, which nothing's changed yet (a component that set its state from its
        // ref renders again here, and Radix's switch asks `closest('form')` in that render), and it's let go of again before
        // it's changed, so that changing it doesn't copy it.
        crate::event::set_current_tree(&self.tree);
        let mut new = self.store.render(id, Some(old.clone()), Cause::State);
        crate::event::forget_current_tree();
        // A parent may have given the node its key, and it shouldn't lose it.
        if new.key.is_none() && old.key.is_some() {
            Arc::make_mut(&mut new).key = old.key.clone();
        }
        if self.first_seen {
            let now = self.store.clock();
            self.motions
                .rendered(&path, Some(&old), &mut new, now, self.store.animations);
        }
        diff_at(&path, &old, &new, patches);

        let mut slot = &mut self.tree;
        for &index in &path {
            slot = &mut Arc::make_mut(slot).children[index as usize];
        }
        *slot = new;
    }

    fn run_effects(&mut self) {
        for cleanup in std::mem::take(&mut self.store.cleanups) {
            cleanup(&mut Updates::new(&mut self.store));
        }
        let effects = std::mem::take(&mut self.store.effects);
        self.store.effects_left = effects.len();
        for effect in effects {
            self.store.effects_left -= 1;
            let current = |store: &Store| {
                let instance = &store.instances[effect.instance as usize];
                instance.alive && instance.generation == effect.generation
            };
            if !current(&self.store) {
                continue;
            }
            let instance = &mut self.store.instances[effect.instance as usize];
            let old_cleanup = instance
                .cleanups
                .iter()
                .position(|(hook, _)| *hook == effect.hook)
                .map(|position| instance.cleanups.remove(position).1);
            if let Some(cleanup) = old_cleanup {
                cleanup(&mut Updates::new(&mut self.store));
            }
            let cleanup = (effect.run)(&mut Updates::new(&mut self.store));
            if let Some(cleanup) = cleanup {
                self.store.instances[effect.instance as usize]
                    .cleanups
                    .push((effect.hook, cleanup));
            }
        }
        if let Some(told) = self.store.on_effects_done {
            told(&mut Updates::new(&mut self.store));
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.store.inbox.close();
        self.store.unmount(self.root_instance);
        for cleanup in std::mem::take(&mut self.store.cleanups) {
            cleanup(&mut Updates::new(&mut self.store));
        }
    }
}

pub(crate) fn node_at<'a>(root: &'a Node, path: &[u32]) -> &'a Node {
    path.iter()
        .fold(root, |node, &index| &node.children[index as usize])
}

impl Store {
    /// The deepest instance whose output contains the node at `path`.
    fn owner_of(&self, root: u32, path: &[u32]) -> u32 {
        let mut owner = root;
        let mut rest = path;
        while let Some((child, remaining)) = self.child_containing(owner, rest) {
            owner = child;
            rest = remaining;
        }
        owner
    }

    /// The child of `parent` whose output contains the node at `path`, and the rest
    /// of the path below that child.
    fn child_containing<'p>(&self, parent: u32, path: &'p [u32]) -> Option<(u32, &'p [u32])> {
        let instance = &self.instances[parent as usize];
        let contains = |child: u32| {
            let child_path = &self.instances[child as usize].path;
            path.strip_prefix(&**child_path)
                .map(|remaining| (child, remaining))
        };
        if instance.children_in_order {
            // In output order, paths are sorted, so only the last one that sorts at
            // or before this path can be a prefix of it.
            let after = instance
                .children
                .partition_point(|&child| *self.instances[child as usize].path <= *path);
            contains(*instance.children.get(after.checked_sub(1)?)?)
        } else {
            instance.children.iter().find_map(|&child| contains(child))
        }
    }

    fn find_handler(&self, from: u32, key: &Key, action: ActionKind) -> Option<(u32, usize)> {
        let mut current = from;
        while current != NO_PARENT {
            let instance = &self.instances[current as usize];
            let found = instance
                .handlers
                .iter()
                .position(|handler| handler.action == action && handler.key == *key);
            if let Some(index) = found {
                return Some((current, index));
            }
            current = instance.parent;
        }
        None
    }
}
