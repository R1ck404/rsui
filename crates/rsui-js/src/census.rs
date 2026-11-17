//! A count of what an engine is keeping, and of what keeps something, for finding out why memory grows.
//!
//! [`Engine::census`] collects all the garbage, then goes from the same roots the collector starts from, over the same
//! references it follows, and counts what it reaches by what it is: a function by its name and where it's written, an
//! object by what made it. [`Engine::retainers`] gives, for what has some text in what it's counted as, the shortest way
//! there from a root, with each step named. What's reached has to be what the collector keeps, so `each_child` in
//! `gc.rs` and [`children`] here have to agree; [`Census::unexplained`] says how many it kept that this didn't reach.
//!
//! The references in a `WeakMap`, a `WeakSet` and a `WeakRef` are only followed once there's nothing else left, and a
//! `WeakMap`'s value once its key has been reached some other way, as they would be if they were weak. What's only
//! reached through them is counted as [`weakly held`](Census::weakly_held): what's kept only because they're held as
//! strongly as anything else.
//!
//! None of this runs unless it's asked for, so it costs a program that doesn't nothing.

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;

use crate::Engine;
use crate::object::*;
use crate::text::Atom;
use crate::value::Value;
use crate::vm::{Job, Vm};

/// What an engine is keeping, once all its garbage has been collected.
#[derive(Debug, Clone, Default)]
pub struct Census {
    /// The objects reached, and what they take up, as the collector counts it.
    pub objects: usize,
    pub object_bytes: usize,
    /// The places in the list of objects, free or not, which is what the list itself takes up.
    pub object_places: usize,
    /// The free places in that list, and how many of them are on pages of memory with nothing but free places on them, which
    /// could be given back to the system without moving anything.
    pub free_places: usize,
    pub free_page_places: usize,
    pub strings: usize,
    pub string_bytes: usize,
    pub string_places: usize,
    /// The variables closures share, reached.
    pub cells: usize,
    pub cell_places: usize,
    /// The objects only reached through a `WeakMap`, a `WeakSet` or a `WeakRef`.
    pub weakly_held: usize,
    /// The objects the collector kept that weren't reached here, which should be none.
    pub unexplained: usize,
    pub shapes: usize,
    /// The property names and symbols in use, and the places for them, free or not.
    pub atoms: usize,
    pub atom_places: usize,
    /// How many values Rust is holding with a `Root`.
    pub rooted: usize,
    /// How many values were on the list of what natives are holding when this was asked for, which collecting the
    /// garbage here empties, but a collection while JavaScript runs doesn't.
    pub nursery: usize,
    /// What [`Engine::memory`] says once the garbage has been collected.
    pub memory: crate::MemoryUse,
    /// The objects reached, by what they're counted as, the most bytes first.
    pub groups: Vec<CensusGroup>,
}

/// Objects that are counted as the same thing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CensusGroup {
    pub label: String,
    pub count: usize,
    pub bytes: usize,
    /// How many of them are only reached through something weak.
    pub weakly: usize,
    /// The room past the last property value or element they have outside themselves, in bytes, which they could do without.
    pub room: usize,
}

impl Census {
    /// How many objects are counted as something with this text in it.
    pub fn count(&self, label: &str) -> usize {
        self.groups
            .iter()
            .filter(|group| group.label.contains(label))
            .map(|group| group.count)
            .sum()
    }

    /// The totals, and the `top` groups with the most bytes.
    pub fn report(&self, top: usize) -> String {
        let mut out = format!(
            "{} bytes in use, {} objects ({} bytes, {} places), {} strings ({} bytes, {} places), {} cells ({} places), {} shapes, \
             {} atoms ({} places), {} rooted, {} in the nursery, {} weakly held, {} unexplained\n\
             {} free places, {} of them on pages with nothing else on them\n",
            self.memory.total(),
            self.objects,
            self.object_bytes,
            self.object_places,
            self.strings,
            self.string_bytes,
            self.string_places,
            self.cells,
            self.cell_places,
            self.shapes,
            self.atoms,
            self.atom_places,
            self.rooted,
            self.nursery,
            self.weakly_held,
            self.unexplained,
            self.free_places,
            self.free_page_places,
        );
        for group in self.groups.iter().take(top) {
            let _ = writeln!(
                out,
                "{:>8} {:>10} B  {}{}{}",
                group.count,
                group.bytes,
                group.label,
                if group.weakly > 0 {
                    format!("  ({} weakly held)", group.weakly)
                } else {
                    String::new()
                },
                if group.room > 0 {
                    format!("  ({} B empty)", group.room)
                } else {
                    String::new()
                }
            );
        }
        out
    }

    /// The groups that have more objects now than `before` had, the most more first, with the totals' change.
    pub fn growth_since(&self, before: &Census, top: usize) -> String {
        let was: HashMap<&str, &CensusGroup> = before
            .groups
            .iter()
            .map(|group| (group.label.as_str(), group))
            .collect();
        let mut grown: Vec<(i64, i64, &CensusGroup)> = self
            .groups
            .iter()
            .map(|group| {
                let (count, bytes) = was
                    .get(group.label.as_str())
                    .map_or((0, 0), |was| (was.count, was.bytes));
                (
                    group.count as i64 - count as i64,
                    group.bytes as i64 - bytes as i64,
                    group,
                )
            })
            .filter(|(count, bytes, _)| *count > 0 || *bytes > 0)
            .collect();
        grown.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
        let mut out = format!(
            "memory {:+} bytes, objects {:+} ({:+} bytes), strings {:+} ({:+} bytes), cells {:+}, shapes {:+}, atoms {:+}, rooted {:+}, weakly held {:+}\n",
            self.memory.total() as i64 - before.memory.total() as i64,
            self.objects as i64 - before.objects as i64,
            self.object_bytes as i64 - before.object_bytes as i64,
            self.strings as i64 - before.strings as i64,
            self.string_bytes as i64 - before.string_bytes as i64,
            self.cells as i64 - before.cells as i64,
            self.shapes as i64 - before.shapes as i64,
            self.atoms as i64 - before.atoms as i64,
            self.rooted as i64 - before.rooted as i64,
            self.weakly_held as i64 - before.weakly_held as i64,
        );
        for (count, bytes, group) in grown.into_iter().take(top) {
            let _ = writeln!(out, "{count:>+8} {bytes:>+10} B  {}", group.label);
        }
        out
    }
}

impl std::fmt::Display for Census {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.report(30))
    }
}

thread_local! {
    /// Whether plain objects are counted by their properties' names too: see [`Engine::census_by_keys`].
    static BY_KEYS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl Engine {
    /// The timers waiting, grouped by what they run, how long they wait and whether they repeat, most first, as
    /// `(count, "name (file:line) every 16 ms")`: for finding timers that pile up, with the `timers` step.
    #[doc(hidden)]
    pub fn timer_summary(&self) -> Vec<(usize, String)> {
        let vm = &self.vm;
        let mut counts: HashMap<String, usize> = HashMap::new();
        for timer in &vm.timers {
            let name = if timer.callback.is_object() {
                function_name(vm, &vm.objects[timer.callback.index()])
                    .unwrap_or_else(|| "?".to_owned())
            } else {
                "?".to_owned()
            };
            let how = match timer.repeat {
                Some(every) => format!("every {every} ms"),
                None => format!("after {} ms", timer.delay),
            };
            *counts.entry(format!("{name} {how}")).or_default() += 1;
        }
        let mut list: Vec<(usize, String)> = counts
            .into_iter()
            .map(|(name, count)| (count, name))
            .collect();
        list.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        list
    }

    /// The same as [`Engine::census`], with plain objects counted by the names of their first properties as well, as
    /// `Object {value,shown,set}`, which tells apart what's made by object literals, as hooks and props are.
    #[doc(hidden)]
    pub fn census_by_keys(&mut self) -> Census {
        BY_KEYS.with(|on| on.set(true));
        let census = self.census();
        BY_KEYS.with(|on| on.set(false));
        census
    }

    /// Collects all the garbage, then counts what's left. See the top of `census.rs`.
    #[doc(hidden)]
    pub fn census(&mut self) -> Census {
        let nursery = self.vm.nursery.len();
        self.collect_garbage();
        // What follows walks every place, and a page given back has nothing to read.
        self.vm.take_back_pages();
        let constructor = self.vm.intern_str("constructor");
        let vm = &self.vm;
        let walk = Walk::new(vm);
        let mut census = Census {
            object_places: vm.objects.len(),
            string_places: vm.strings.items.len(),
            cell_places: vm.cells.len(),
            shapes: vm.shapes.live(),
            atoms: vm.atoms.live(),
            atom_places: vm.atoms.len(),
            nursery,
            memory: vm.memory(),
            ..Census::default()
        };
        vm.held.each(|_| census.rooted += 1);
        let mut groups: HashMap<String, CensusGroup> = HashMap::new();
        let mut kept = 0;
        // Each page of the list's memory: which it is, whether everything on it is free, and how many places start on it.
        let base = vm.objects.as_ptr() as usize;
        let mut page = (usize::MAX, true, 0usize);
        let finish = |page: (usize, bool, usize), census: &mut Census| {
            if page.0 != usize::MAX && page.1 {
                census.free_page_places += page.2;
            }
        };
        for (index, object) in vm.objects.iter().enumerate() {
            let at = (base + index * size_of::<Object>()) / 4096;
            if at != page.0 {
                finish(page, &mut census);
                page = (at, true, 0);
            }
            page.2 += 1;
            if matches!(object.kind, Kind::Free) {
                census.free_places += 1;
            } else {
                page.1 = false;
                kept += 1;
            }
            if !walk.objects[index].reached || matches!(object.kind, Kind::Free) {
                continue;
            }
            let bytes = object_bytes(object);
            let weakly = walk.objects[index].weakly;
            census.objects += 1;
            census.object_bytes += bytes;
            census.weakly_held += usize::from(weakly);
            let mut label = label(vm, index as u32, constructor);
            if BY_KEYS.with(std::cell::Cell::get) {
                label = with_holder(vm, &walk, index as u32, label, constructor);
            }
            let group = groups.entry(label).or_default();
            group.count += 1;
            group.bytes += bytes;
            group.weakly += usize::from(weakly);
            group.room += room_bytes(object);
        }
        finish(page, &mut census);
        census.unexplained = kept - census.objects.min(kept);
        let tally = crate::text::Tally::new();
        for (index, reached) in walk.strings.iter().enumerate() {
            if *reached && let Some(string) = vm.strings.items.get(index) {
                census.strings += 1;
                census.string_bytes +=
                    string.text.heap_bytes_in(tally) + size_of::<crate::text::StringData>();
            }
        }
        census.cells = walk.cells.iter().filter(|cell| cell.reached).count();
        census.groups = groups
            .into_iter()
            .map(|(label, mut group)| {
                group.label = label;
                group
            })
            .collect();
        census.groups.sort_by(|a, b| {
            b.bytes
                .cmp(&a.bytes)
                .then(b.count.cmp(&a.count))
                .then(a.label.cmp(&b.label))
        });
        census
    }

    /// Collects all the garbage, then gives the shortest way from a root to each of up to `limit` objects counted as
    /// something with `label` in it, the most recently made places first, one line each. See the top of `census.rs`.
    #[doc(hidden)]
    pub fn retainers(&mut self, label: &str, limit: usize) -> Vec<String> {
        self.collect_garbage();
        self.vm.take_back_pages();
        let constructor = self.vm.intern_str("constructor");
        let vm = &self.vm;
        let walk = Walk::new(vm);
        let mut out = Vec::new();
        for index in (0..vm.objects.len()).rev() {
            if out.len() >= limit {
                break;
            }
            if !walk.objects[index].reached {
                continue;
            }
            let found = self::label(vm, index as u32, constructor);
            if found.contains(label) {
                out.push(walk.path_to(vm, Node::Object(index as u32), constructor));
            }
        }
        out
    }
}

/// Something that's reached: an object (or a BigInt, which lives with them), or a cell.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Node {
    Object(u32),
    Cell(u32),
}

/// How something was reached from the thing before it.
#[derive(Clone, Copy)]
enum Step {
    /// From a root: what kind, and which of them.
    Root(&'static str, u32),
    Slot(u32),
    Item(u32),
    Upvalue(u32),
    Home,
    NativeData,
    BoundTarget,
    BoundThis,
    BoundArg(u32),
    Boxed,
    Key(u32),
    Value(u32),
    WeakTarget,
    RegistryCallback,
    RegistryHeld(u32),
    RegistryToken(u32),
    Promise(&'static str, u32),
    Generator(&'static str, u32),
    IterTarget,
    Getter,
    Setter,
    RegExp(&'static str),
    Buffer,
    ProxyTarget,
    ProxyHandler,
    CellValue,
    Prototype,
}

#[derive(Clone, Copy)]
struct Seen {
    reached: bool,
    /// Only through something weak.
    weakly: bool,
    from: Option<Node>,
    step: Step,
}

impl Default for Seen {
    fn default() -> Self {
        Self {
            reached: false,
            weakly: false,
            from: None,
            step: Step::Root("", 0),
        }
    }
}

/// A walk over everything reachable, breadth first, so that the way it finds to each thing is the shortest.
struct Walk {
    objects: Vec<Seen>,
    cells: Vec<Seen>,
    strings: Vec<bool>,
    queue: VecDeque<Node>,
    /// Whether what's being reached now is only reached through something weak.
    weakly: bool,
    /// What something weak refers to, put off until nothing else is left.
    put_off: Vec<(Node, Step, Value)>,
    /// A `WeakMap`'s entries, by the map and where they are, with their key and value.
    entries: Vec<(Node, u32, Value, Value)>,
}

impl Walk {
    fn new(vm: &Vm) -> Self {
        let mut walk = Self {
            objects: vec![Seen::default(); vm.objects.len()],
            cells: vec![Seen::default(); vm.cells.len()],
            strings: vec![false; vm.strings.items.len()],
            queue: VecDeque::new(),
            weakly: false,
            put_off: Vec::new(),
            entries: Vec::new(),
        };
        walk.roots(vm);
        walk.drain(vm);
        // A `WeakMap`'s value is held by its key, so once a key has been reached, so has its value.
        loop {
            let mut more = false;
            let entries = std::mem::take(&mut walk.entries);
            for entry in entries {
                let (map, at, key, value) = entry;
                if walk.is_reached(key) {
                    walk.reach(value, Some(map), Step::Value(at));
                    more = true;
                } else {
                    walk.entries.push(entry);
                }
            }
            if !more {
                break;
            }
            walk.drain(vm);
        }
        // Whatever's left is only held through something weak.
        walk.weakly = true;
        for (from, step, value) in std::mem::take(&mut walk.put_off) {
            walk.reach(value, Some(from), step);
        }
        for (map, at, key, value) in std::mem::take(&mut walk.entries) {
            walk.reach(key, Some(map), Step::Key(at));
            walk.reach(value, Some(map), Step::Value(at));
        }
        walk.drain(vm);
        walk
    }

    fn is_reached(&self, value: Value) -> bool {
        let tag = value.tag();
        if tag == Value::OBJECT_TAG {
            self.objects
                .get(value.index())
                .is_some_and(|seen| seen.reached)
        } else {
            // Anything that isn't an object can't be collected while something uses it as a key.
            true
        }
    }

    /// The same roots the collector starts from, in `Vm::collect`.
    fn roots(&mut self, vm: &Vm) {
        let root = |walk: &mut Self, kind: &'static str, at: usize, value: Value| {
            walk.reach(value, None, Step::Root(kind, at as u32));
        };
        for (at, value) in vm.stack[..vm.top].iter().enumerate() {
            root(self, "the stack", at, *value);
        }
        for (at, value) in vm.consts.iter().enumerate() {
            root(self, "a constant", at, *value);
        }
        for (at, value) in vm.nursery.iter().enumerate() {
            root(self, "what natives hold", at, *value);
        }
        for (at, value) in vm.roots.iter().enumerate() {
            root(self, "the engine's roots", at, *value);
        }
        let mut at = 0;
        vm.held.each(|value| {
            root(self, "a Rust Root", at, value);
            at += 1;
        });
        for (at, value) in vm.char_strings.iter().enumerate() {
            root(self, "a character's string", at, *value);
        }
        root(self, "the global object", 0, Value::object(vm.global));
        for (at, intrinsic) in vm.selector_intrinsics.iter().enumerate() {
            root(self, "selector intrinsic owner", at, intrinsic.owner);
            root(self, "selector intrinsic value", at, intrinsic.value);
        }
        root(
            self,
            "Function.prototype[Symbol.hasInstance]",
            0,
            vm.function_has_instance,
        );
        root(self, "the strict callee thrower", 0, vm.throw_type_error);
        root(self, "Array.prototype.values", 0, vm.realm.array_values.0);
        root(self, "what using calls", 0, vm.realm.disposal.0);
        root(self, "the helpers", 0, vm.helpers);
        for (at, frame) in vm.frames.iter().enumerate() {
            if frame.closure != NONE {
                root(self, "a frame's closure", at, Value::object(frame.closure));
            }
            root(self, "a frame's new.target", at, frame.new_target);
        }
        for (at, args) in vm.kept_args.iter().enumerate() {
            for value in args {
                root(self, "a frame's arguments", at, *value);
            }
        }
        for (at, job) in vm.jobs.iter().enumerate() {
            let values = match job {
                Job::Reaction {
                    handler,
                    argument,
                    derived,
                    ..
                } => vec![*handler, *argument, *derived],
                Job::Thenable {
                    promise,
                    thenable,
                    then,
                } => vec![*promise, *thenable, *then],
                Job::Call(function) => vec![*function],
            };
            for value in values {
                root(self, "a job", at, value);
            }
        }
        // What a `WeakRef` handed out during this job, and registries whose callbacks are due, as the collector has them.
        for (at, value) in vm.kept.iter().enumerate() {
            root(self, "kept by a WeakRef until the job ends", at, *value);
        }
        for (at, registry) in vm.cleanups.iter().enumerate() {
            root(self, "a registry with callbacks due", at, *registry);
        }
        for (at, timer) in vm.timers.iter().enumerate() {
            root(self, "a timer", at, timer.callback);
            root(self, "a timer's object", at, timer.object);
            for value in &timer.args {
                root(self, "a timer's arguments", at, *value);
            }
        }
        for (at, value) in vm.unhandled_rejections.iter().enumerate() {
            root(self, "an unhandled rejection", at, *value);
        }
        for (at, value) in vm.modules.values().into_iter().enumerate() {
            root(self, "the modules", at, value);
        }
        for handle in vm.atoms.strings() {
            if let Some(seen) = self.strings.get_mut(handle as usize) {
                *seen = true;
            }
        }
        let realm = &vm.realm;
        for (at, handle) in [
            realm.object_proto,
            realm.function_proto,
            realm.array_proto,
            realm.string_proto,
            realm.number_proto,
            realm.boolean_proto,
            realm.symbol_proto,
            realm.error_proto,
            realm.type_error_proto,
            realm.range_error_proto,
            realm.reference_error_proto,
            realm.syntax_error_proto,
            realm.iterator_proto,
            realm.native_iterator_proto,
            realm.generator_proto,
            realm.promise_proto,
            realm.promise_constructor,
            realm.array_constructor,
            realm.map_proto,
            realm.set_proto,
            realm.weak_map_proto,
            realm.weak_set_proto,
            realm.date_proto,
            realm.regexp_proto,
            realm.array_buffer_proto,
            realm.data_view_proto,
            realm.text_encoder_proto,
            realm.text_decoder_proto,
            realm.async_generator_proto,
            realm.bigint_proto,
            realm.generator_function_proto,
            realm.async_function_proto,
            realm.async_generator_function_proto,
            realm.weak_ref_proto,
            realm.finalization_registry_proto,
            realm.suppressed_error_proto,
            realm.disposable_stack_proto,
            realm.async_disposable_stack_proto,
        ]
        .into_iter()
        .enumerate()
        {
            if handle != NONE {
                root(self, "the realm", at, Value::object(handle));
            }
        }
    }

    fn reach(&mut self, value: Value, from: Option<Node>, step: Step) {
        let tag = value.tag();
        let (seen, node) =
            if tag == Value::OBJECT_TAG || (tag >= Value::CELL_TAG && value.is_bigint()) {
                (
                    self.objects.get_mut(value.index()),
                    Node::Object(value.handle()),
                )
            } else if tag == Value::STRING_TAG {
                if let Some(seen) = self.strings.get_mut(value.index()) {
                    *seen = true;
                }
                return;
            } else if tag >= Value::CELL_TAG {
                (
                    self.cells.get_mut(value.index()),
                    Node::Cell(value.handle()),
                )
            } else {
                return;
            };
        // A handle past the end is one to something freed long ago, which the collector leaves alone too.
        let Some(seen) = seen else { return };
        if seen.reached {
            return;
        }
        *seen = Seen {
            reached: true,
            weakly: self.weakly,
            from,
            step,
        };
        self.queue.push_back(node);
    }

    /// Something weak refers to `value`, which is only followed once there's nothing else, unless that's now.
    fn weak(&mut self, value: Value, from: Node, step: Step) {
        if self.weakly {
            self.reach(value, Some(from), step);
        } else {
            self.put_off.push((from, step, value));
        }
    }

    fn drain(&mut self, vm: &Vm) {
        while let Some(node) = self.queue.pop_front() {
            match node {
                Node::Cell(index) => {
                    let value = vm.cells[index as usize];
                    self.reach(value, Some(node), Step::CellValue);
                }
                Node::Object(index) => {
                    let object = &vm.objects[index as usize];
                    children(self, node, object);
                    // An object's prototype is kept through its shape, for as long as the object is.
                    let proto = vm.shapes.items[object.shape as usize].proto;
                    if proto != NONE && !matches!(object.kind, Kind::Free) {
                        self.reach(Value::object(proto), Some(node), Step::Prototype);
                    }
                }
            }
        }
    }

    /// The way from a root to `node`, each thing and how the next is reached from it.
    fn path_to(&self, vm: &Vm, node: Node, constructor: Atom) -> String {
        let mut steps = Vec::new();
        let mut at = Some(node);
        while let Some(node) = at {
            let seen = match node {
                Node::Object(index) => self.objects[index as usize],
                Node::Cell(index) => self.cells[index as usize],
            };
            steps.push((node, seen));
            at = seen.from;
            if steps.len() > 10_000 {
                break;
            }
        }
        steps.reverse();
        let mut out = String::new();
        for (number, (node, seen)) in steps.iter().enumerate() {
            if number == 0 {
                if let Step::Root(kind, at) = seen.step {
                    let _ = write!(out, "{kind} #{at}");
                }
            } else {
                let from = steps[number - 1].0;
                let _ = write!(out, " {}", step_name(vm, from, seen.step));
            }
            if seen.weakly && (number == 0 || !steps[number - 1].1.weakly) {
                out.push_str(" (weak)");
            }
            let _ = write!(out, " -> {}", node_name(vm, *node, constructor));
        }
        out
    }
}

/// Reaches everything an object refers to, as `each_child` in `gc.rs` gives it to the collector, saying how.
fn children(walk: &mut Walk, node: Node, object: &Object) {
    let from = Some(node);
    for (at, value) in object.slots().iter().enumerate() {
        walk.reach(*value, from, Step::Slot(at as u32));
    }
    match &object.kind {
        Kind::Free
        | Kind::Plain
        | Kind::Error
        | Kind::Date(_)
        | Kind::ArrayBuffer(_)
        | Kind::BigInt(_) => {}
        Kind::Array(items) | Kind::Arguments(items) => {
            for (at, value) in items.iter().enumerate() {
                walk.reach(*value, from, Step::Item(at as u32));
            }
        }
        Kind::Function { home, upvalues, .. } => {
            if *home != NONE {
                walk.reach(Value::object(*home), from, Step::Home);
            }
            for (at, value) in upvalues.iter().enumerate() {
                walk.reach(*value, from, Step::Upvalue(at as u32));
            }
        }
        Kind::Native(native) => walk.reach(native.data, from, Step::NativeData),
        Kind::Bound(bound) => {
            walk.reach(bound.target, from, Step::BoundTarget);
            walk.reach(bound.this, from, Step::BoundThis);
            for (at, value) in bound.args.iter().enumerate() {
                walk.reach(*value, from, Step::BoundArg(at as u32));
            }
        }
        Kind::Boxed(value) => walk.reach(*value, from, Step::Boxed),
        Kind::WeakRef(value) => walk.weak(*value, node, Step::WeakTarget),
        // As the collector has it: the callback and what it's given are held strongly, targets and tokens weakly.
        Kind::FinalizationRegistry(data) => {
            walk.reach(data.cleanup, from, Step::RegistryCallback);
            for (at, cell) in data.cells.iter().enumerate() {
                walk.reach(cell.held, from, Step::RegistryHeld(at as u32));
                walk.weak(cell.target, node, Step::WeakTarget);
                walk.weak(cell.token, node, Step::RegistryToken(at as u32));
            }
            for (at, (held, token)) in data.cleared.iter().enumerate() {
                walk.reach(*held, from, Step::RegistryHeld(at as u32));
                walk.weak(*token, node, Step::RegistryToken(at as u32));
            }
        }
        Kind::Map(data) | Kind::Set(data) => {
            for (at, (key, value)) in data.entries.iter().enumerate() {
                walk.reach(*key, from, Step::Key(at as u32));
                walk.reach(*value, from, Step::Value(at as u32));
            }
        }
        Kind::WeakSet(data) => {
            // What's in a set can be kept as the value of its entry as well as the key.
            for (at, (key, value)) in data.entries.iter().enumerate() {
                walk.weak(*key, node, Step::Key(at as u32));
                walk.weak(*value, node, Step::Value(at as u32));
            }
        }
        Kind::WeakMap(data) => {
            for (at, (key, value)) in data.entries.iter().enumerate() {
                if walk.weakly {
                    walk.reach(*key, from, Step::Key(at as u32));
                    walk.reach(*value, from, Step::Value(at as u32));
                } else if walk.is_reached(*key) {
                    walk.reach(*value, from, Step::Value(at as u32));
                } else {
                    walk.entries.push((node, at as u32, *key, *value));
                }
            }
        }
        Kind::Promise(data) => {
            walk.reach(data.value, from, Step::Promise("value", 0));
            for (at, reaction) in data.reactions.iter().enumerate() {
                let at = at as u32;
                walk.reach(
                    reaction.on_fulfilled,
                    from,
                    Step::Promise("on fulfilled", at),
                );
                walk.reach(reaction.on_rejected, from, Step::Promise("on rejected", at));
                walk.reach(reaction.derived, from, Step::Promise("derived", at));
            }
        }
        Kind::Generator(data) => {
            if data.closure != NONE {
                walk.reach(
                    Value::object(data.closure),
                    from,
                    Step::Generator("closure", 0),
                );
            }
            for (at, value) in data.registers.iter().enumerate() {
                walk.reach(*value, from, Step::Generator("register", at as u32));
            }
            if let Some(args) = &data.extra_args {
                for (at, value) in args.iter().enumerate() {
                    walk.reach(*value, from, Step::Generator("argument", at as u32));
                }
            }
            walk.reach(data.new_target, from, Step::Generator("new.target", 0));
            walk.reach(data.promise, from, Step::Generator("promise", 0));
            walk.reach(data.return_value, from, Step::Generator("return value", 0));
            for (at, (_, value, promise)) in data.queue.iter().enumerate() {
                walk.reach(*value, from, Step::Generator("queued", at as u32));
                walk.reach(*promise, from, Step::Generator("queued promise", at as u32));
            }
        }
        Kind::Iter(data) => walk.reach(data.target, from, Step::IterTarget),
        Kind::Accessor { get, set } => {
            walk.reach(*get, from, Step::Getter);
            walk.reach(*set, from, Step::Setter);
        }
        Kind::RegExp(data) => {
            walk.reach(data.source, from, Step::RegExp("source"));
            walk.reach(data.flags, from, Step::RegExp("flags"));
        }
        Kind::TypedArray(data) => walk.reach(data.buffer, from, Step::Buffer),
        Kind::DataView(data) => walk.reach(data.buffer, from, Step::Buffer),
        Kind::Proxy(data) => {
            walk.reach(data.target, from, Step::ProxyTarget);
            walk.reach(data.handler, from, Step::ProxyHandler);
        }
    }
}

/// What an object takes up, as the collector counts it, and what the table of a map takes besides.
/// The room past the last value in what an object keeps outside itself, in bytes.
fn room_bytes(object: &Object) -> usize {
    let slots = object.slots_heap_bytes().saturating_sub(object.slots().len() * 8);
    match &object.kind {
        Kind::Array(items) | Kind::Arguments(items) if items.capacity() > 0 => {
            slots + (items.capacity() - items.len()) * 8
        }
        _ => slots,
    }
}

fn object_bytes(object: &Object) -> usize {
    size_of::<Object>()
        + object.slots_heap_bytes()
        + match &object.kind {
            Kind::Array(items) | Kind::Arguments(items) => items.capacity() * 8,
            Kind::ArrayBuffer(data) => data.bytes.capacity(),
            Kind::Function { upvalues, .. } => upvalues.len() * 8,
            Kind::Native(_) => size_of::<NativeData>(),
            Kind::Map(data) | Kind::Set(data) | Kind::WeakMap(data) | Kind::WeakSet(data) => {
                data.entries.capacity() * 16
                    + data.buckets.capacity() * 4
                    + data.chain.capacity() * 4
            }
            _ => 0,
        }
}

/// The last part of a file's path.
fn file_name(vm: &Vm, file: Atom) -> String {
    let path = vm.atom_text(file);
    path.rsplit(['/', '\\']).next().unwrap_or(&path).to_owned()
}

/// A function's name and where it's written, as `name (file:line)`.
fn function_name(vm: &Vm, object: &Object) -> Option<String> {
    match &object.kind {
        Kind::Function { proto, .. } => {
            let proto = &vm.protos[*proto as usize];
            let name = vm.atom_text(proto.name);
            let line = proto.lines.first().map_or(0, |(_, line)| *line);
            Some(format!(
                "{} ({}:{line})",
                if name.is_empty() { "anonymous" } else { &name },
                file_name(vm, proto.file)
            ))
        }
        Kind::Native(native) => Some(format!("{} (native)", vm.atom_text(native.name))),
        Kind::Bound(bound) if bound.target.is_object() => {
            function_name(vm, &vm.objects[bound.target.index()]).map(|name| format!("bound {name}"))
        }
        _ => None,
    }
}

/// An object's own property, if it has one by that name that isn't a getter or setter.
fn own_value(vm: &Vm, object: &Object, name: Atom) -> Option<Value> {
    let mut found = None;
    vm.shapes.visit_entries(object.shape, |key, slot| {
        if key == name {
            found = Some(slot);
        }
    });
    let value = *object.slots().get(found? as usize)?;
    let is_accessor =
        value.is_object() && matches!(vm.objects[value.index()].kind, Kind::Accessor { .. });
    (!is_accessor).then_some(value)
}

/// The name of the function an object's `constructor` is, if it has one of its own.
fn constructor_name(vm: &Vm, object: &Object, constructor: Atom) -> Option<String> {
    let value = own_value(vm, object, constructor)?;
    if !value.is_object() {
        return None;
    }
    function_name(vm, &vm.objects[value.index()])
}

/// What an object is counted as: a function by its name and where it's written, a prototype by whose it is, and
/// anything else by its kind and what made it, as far as its prototype's `constructor` says.
fn label(vm: &Vm, handle: u32, constructor: Atom) -> String {
    let object = &vm.objects[handle as usize];
    if let Some(name) = function_name(vm, object) {
        let kind = match &object.kind {
            Kind::Native(_) => "native function",
            Kind::Bound(_) => "bound function",
            _ => "function",
        };
        return format!("{kind} {name}");
    }
    let kind = match &object.kind {
        Kind::Free => "free",
        Kind::Plain => "Object",
        Kind::Array(_) => "Array",
        Kind::Arguments(_) => "arguments",
        Kind::Function { .. } | Kind::Native(_) | Kind::Bound(_) => "function",
        Kind::Error => "Error",
        Kind::Boxed(_) => "boxed primitive",
        Kind::Date(_) => "Date",
        Kind::Map(_) => "Map",
        Kind::Set(_) => "Set",
        Kind::WeakMap(_) => "WeakMap",
        Kind::WeakSet(_) => "WeakSet",
        Kind::Promise(_) => "Promise",
        Kind::Generator(_) => "generator",
        Kind::Iter(_) => "iterator",
        Kind::Accessor { .. } => "getter and setter",
        Kind::RegExp(_) => "RegExp",
        Kind::ArrayBuffer(_) => "ArrayBuffer",
        Kind::WeakRef(_) => "WeakRef",
        Kind::FinalizationRegistry(_) => "FinalizationRegistry",
        Kind::TypedArray(_) => "typed array",
        Kind::DataView(_) => "DataView",
        Kind::Proxy(_) => "Proxy",
        Kind::BigInt(_) => "BigInt",
    };
    if !matches!(object.kind, Kind::Plain | Kind::Error) {
        return kind.to_owned();
    }
    if let Some(of) = constructor_name(vm, object, constructor) {
        return format!("prototype of {of}");
    }
    let proto = vm.shapes.items[object.shape as usize].proto;
    if proto == NONE {
        return format!("{kind} (null prototype)");
    }
    if proto == vm.realm.object_proto {
        if BY_KEYS.with(std::cell::Cell::get) && matches!(object.kind, Kind::Plain) {
            let names: Vec<String> = vm.shapes.entries(object.shape).iter().take(6).map(|(key, _, _)| vm.atom_text(*key)).collect();
            return format!("{kind} {{{}}}", names.join(","));
        }
        return kind.to_owned();
    }
    match constructor_name(vm, &vm.objects[proto as usize], constructor) {
        Some(of) => format!("{kind} made by {of}"),
        None => format!("{kind} with prototype #{proto}"),
    }
}

/// An array's, a `Set`'s or a `Map`'s label with what it was first reached from, as `Array @ Object {deps,run}.deps`, which
/// tells apart lists that are all just `Array` otherwise. Only for [`Engine::census_by_keys`].
fn with_holder(vm: &Vm, walk: &Walk, handle: u32, label: String, constructor: Atom) -> String {
    let object = &vm.objects[handle as usize];
    if !matches!(object.kind, Kind::Array(_) | Kind::Set(_) | Kind::Map(_)) {
        return label;
    }
    let seen = walk.objects[handle as usize];
    match seen.from {
        Some(Node::Object(from)) => {
            let step = match seen.step {
                Step::Item(_) => "[]".to_owned(),
                Step::Key(_) | Step::Value(_) => " entry".to_owned(),
                step => step_name(vm, Node::Object(from), step),
            };
            format!("{label} @ {}{step}", self::label(vm, from, constructor))
        }
        Some(Node::Cell(_)) => format!("{label} @ a closure's variable"),
        None => label,
    }
}

/// A thing on a way from a root, for [`Walk::path_to`].
fn node_name(vm: &Vm, node: Node, constructor: Atom) -> String {
    match node {
        Node::Cell(index) => format!("cell #{index}"),
        Node::Object(index) => format!("{} #{index}", label(vm, index, constructor)),
    }
}

/// How the next thing on a way from a root is reached from `from`.
fn step_name(vm: &Vm, from: Node, step: Step) -> String {
    let object = match from {
        Node::Object(index) => Some(&vm.objects[index as usize]),
        Node::Cell(_) => None,
    };
    match step {
        Step::Root(kind, at) => format!("{kind} #{at}"),
        Step::Slot(slot) => {
            let mut name = None;
            if let Some(object) = object {
                vm.shapes.visit_entries(object.shape, |key, at| {
                    if at == slot {
                        name = Some(key);
                    }
                });
            }
            match name {
                Some(name) => format!(".{}", vm.atom_text(name)),
                None => format!(".[slot {slot}]"),
            }
        }
        Step::Item(at) => format!("[{at}]"),
        Step::Upvalue(at) => {
            let name = match object.map(|object| &object.kind) {
                Some(Kind::Function { proto, .. }) => vm.protos[*proto as usize]
                    .capture_names
                    .get(at as usize)
                    .map(|name| vm.atom_text(*name)),
                _ => None,
            };
            match name {
                Some(name) if !name.is_empty() => format!("captures `{name}`"),
                _ => format!("captures #{at}"),
            }
        }
        Step::Home => "home object".to_owned(),
        Step::NativeData => "native data".to_owned(),
        Step::BoundTarget => "bound target".to_owned(),
        Step::BoundThis => "bound this".to_owned(),
        Step::BoundArg(at) => format!("bound argument {at}"),
        Step::Boxed => "boxed value".to_owned(),
        Step::Key(at) => format!("key #{at}"),
        Step::Value(at) => format!("value #{at}"),
        Step::WeakTarget => "target".to_owned(),
        Step::RegistryCallback => "cleanup callback".to_owned(),
        Step::RegistryHeld(at) => format!("held value {at}"),
        Step::RegistryToken(at) => format!("unregister token {at}"),
        Step::Promise(what, at) | Step::Generator(what, at) => format!("{what} #{at}"),
        Step::IterTarget => "iterating".to_owned(),
        Step::Getter => "getter".to_owned(),
        Step::Setter => "setter".to_owned(),
        Step::RegExp(what) => what.to_owned(),
        Step::Buffer => "buffer".to_owned(),
        Step::ProxyTarget => "proxy target".to_owned(),
        Step::ProxyHandler => "proxy handler".to_owned(),
        Step::CellValue => "holds".to_owned(),
        Step::Prototype => "prototype".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use crate::Engine;

    /// What's reached is everything the collector kept, and each function is counted by its name and where it is.
    #[test]
    fn it_counts_what_the_collector_keeps() {
        let mut engine = Engine::new();
        engine
            .eval(
                "function Kept() {}\n\
                 globalThis.kept = [];\n\
                 for (let at = 0; at < 50; at += 1) kept.push({ made: new Kept(), run() { return at; } });\n\
                 for (let at = 0; at < 1000; at += 1) ({ gone: [at] });",
                "kept.js",
            )
            .unwrap();
        let census = engine.census();
        assert_eq!(census.unexplained, 0, "{census}");
        assert_eq!(census.weakly_held, 0, "{census}");
        assert_eq!(census.count("Object made by Kept (kept.js:1)"), 50, "{census}");
        assert_eq!(census.count("function run (kept.js:3)"), 50, "{census}");
        assert_eq!(census.count("prototype of Kept"), 1, "{census}");
        let paths = engine.retainers("Object made by Kept", 1);
        assert_eq!(paths.len(), 1);
        assert!(
            paths[0].starts_with("the global object #0 -> Object #")
                && paths[0].contains(".kept -> Array")
                && paths[0].contains(".made -> Object made by Kept"),
            "{}",
            paths[0]
        );
    }

    /// What only a `WeakMap`, a `WeakSet` or a `WeakRef` refers to is let go of by the collection the census starts
    /// with, so none of it is counted, while a `WeakMap`'s value stays while something else has its key, and a
    /// `WeakRef`'s target stays until the job that made it ends.
    #[test]
    fn it_tells_what_is_only_weakly_held() {
        let mut engine = Engine::new();
        engine
            .eval(
                "function Key() {}\n\
                 function Held() {}\n\
                 globalThis.map = new WeakMap();\n\
                 globalThis.keys = [];\n\
                 for (let at = 0; at < 10; at += 1) { const key = new Key(); keys.push(key); map.set(key, new Held()); }\n\
                 for (let at = 0; at < 20; at += 1) map.set(new Key(), new Held());\n\
                 globalThis.set = new WeakSet([new Key()]);\n\
                 globalThis.ref = new WeakRef(new Key());",
                "weak.js",
            )
            .unwrap();
        let census = engine.census();
        assert_eq!(census.unexplained, 0, "{census}");
        let weakly = |label: &str| {
            census
                .groups
                .iter()
                .filter(|group| group.label.contains(label))
                .map(|group| group.weakly)
                .sum::<usize>()
        };
        // The 10 keys in `keys`, and the `WeakRef`'s target, kept until this job ends.
        assert_eq!(census.count("Object made by Key"), 11, "{census}");
        assert_eq!(weakly("Object made by Key"), 0, "{census}");
        // Only the values of keys that are still there.
        assert_eq!(census.count("Object made by Held"), 10, "{census}");
        assert_eq!(weakly("Object made by Held"), 0, "{census}");
        assert_eq!(census.weakly_held, 0, "{census}");
        let paths = engine.retainers("Object made by Held", 1);
        assert!(paths[0].contains("map"), "{}", paths[0]);
    }
}
