//! What a component read of what it was given as it rendered, so that when it's given new props it can skip rendering if
//! what it read is all the same, even where something else in them changed, as when a pane is given the whole of a session
//! that's saved again whenever which pane has the keyboard changes, and only reads the views kept in it. See "Not rendering
//! what would come out the same" in rsui-tsx's DESIGN.md.
//!
//! The compiler puts a [`Op::Note`] after each read of what a component function is given (its props, or a name
//! destructured from them) in an app's own files, as `props.session.views[key]`: the chain of property reads is noted
//! whole, with what came out at its end, which is compared as a whole when the component's given new props, as
//! [`Vm::props_equivalent`] compares props. What's read as itself, as the props spread into an object or given to a call, is
//! the same as reading all of it. And it puts one after each read of a captured variable from a function around it, which
//! is what a function given in the props, as `pane={(id) => <Pane active={id === focus} />}`, reads when the component calls
//! it as it renders: those are noted through the function, so that what it would give is known to be the same when what it
//! read is, whatever else it sees has changed.
//!
//! A Note does nothing but step over what follows it unless something's rendering ([`Vm::tracking`]), which is a test of
//! one byte, in compiled code too.

use crate::object::{ACCESSOR, Kind, LAZY_FUNCTION};
use crate::op::{
    IS_ARROW, IS_ASYNC, IS_CLASS, IS_GENERATOR, NOTE_ALIASED, NOTE_ARG, NOTE_CALLED, NOTE_CALLING, NOTE_CELL, NOTE_COMPARED, NOTE_ENTER,
    NOTE_KINDS, NOTE_OWN, NOTE_PARAM, NOTE_PARAMS, NOTE_RESULT, NOTE_SAME_ITSELF, NOTE_UP, Op, SAYS_APP_FILE,
};
use crate::text::Atom;
use crate::value::Value;
use crate::vm::Vm;
use foldhash::fast::FixedState;
use std::collections::HashMap;

/// Set on a function that's been called while something rendered without what it read being noted for that, or that's
/// kept as a hook's dependency, so that it's never made to do what a new one does (see
/// [`Vm::props_equivalent_handing_on`]): what rendered with it, or the hook, would go on as it was.
/// It's `READ_ONLY_LENGTH`'s bit, which only an array has, and a function never does: the others, as the collector's `OLD`, are
/// in use on every kind of object.
pub(crate) const NOT_HANDED_ON: u8 = crate::object::READ_ONLY_LENGTH;

/// What [`Vm::start_reads`] gives for a render that isn't noted, as a library's component's.
pub const NO_READS: u32 = u32::MAX;

/// How far a read goes into what it read, at most, which is a chain of property reads after one another.
const MOST_STEPS: usize = 32;

/// How many paths, or entries, a render has before they're found by a table rather than by looking along them. A row reads a
/// few; a list that reads each of thousands of items would otherwise look along all it's read for each one it reads.
const INDEX_FROM: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Step {
    /// A property, by the key it was read with.
    Key(u64),
    /// What a function captured, as its code numbers it: a closure of another code isn't the same one.
    Capture(u32, u16),
}

#[derive(Clone, Copy)]
struct Path {
    parent: u32,
    step: Step,
    key: Value,
}

/// One thing that was read, where it was, and what it was.
#[derive(Clone, Copy)]
struct Entry {
    path: u32,
    value: Value,
    /// It has to be the same value, not one that does the same.
    itself: bool,
    /// It was read as more than one thing, so it's taken to have changed.
    changing: bool,
    /// It's what a method was called on, which is only the same as itself while it holds no functions and isn't a ref, for
    /// what didn't say `memo` (see `cautious` in [`Vm::props_equivalent`]), as a method can say something else with
    /// nothing given anew. Anything else that's the same object is the same: what's read of it is noted for itself.
    careful: bool,
    /// All that was done with it was give it to elements the app's JSX made, as a prop, so what it is only matters as far as
    /// what those read of it: see [`Vm::link_child`].
    only_given: bool,
}

/// A function read from the props whose own reads are noted through it.
enum Registered {
    /// Read at this path: what it captures is under it.
    Read(u32),
    /// Made by such a function as it ran: where each of what it captured from that one is.
    Inner(Box<[Option<u32>]>),
}

/// What one render of a component read.
pub(crate) struct Record {
    /// The host's number for what rendered, which [`Vm::take_unheld_readers`] gives back.
    owner: u32,
    function: Value,
    props: Value,
    /// Whether the same object with methods, or a ref, isn't the same as itself, as for [`Vm::props_equivalent`].
    cautious: bool,
    /// The component's own frame, once it's been called, as its depth and where its registers start.
    frame: Option<(u32, u32)>,
    gave_up: bool,
    /// Where things were read, with the props themselves first.
    paths: Vec<Path>,
    entries: Vec<Entry>,
    /// Which path is where, by what it's in and how it's reached, and which entry is each path's, once there are
    /// [`INDEX_FROM`] of them. Nothing's ever taken out of either list, so these only grow with them.
    path_index: Option<Box<HashMap<(u32, Step), u32, FixedState>>>,
    entry_index: Option<Box<HashMap<u32, u32, FixedState>>>,
    /// What was only compared with `===` or `!==`: where it was, what with, and whether it was the same.
    comparisons: Vec<(u32, Value, bool)>,
    /// Each of `comparisons`, once there are [`INDEX_FROM`]: a list whose rows each compare an item with what's picked
    /// compares thousands, and each one would otherwise look along all those before it.
    comparison_index: Option<Box<std::collections::HashSet<(u32, u64, bool), FixedState>>>,
    /// The cells the props, or what they were destructured as, are in, which functions inside can see, and where in the
    /// props each is.
    cells: Vec<(Value, u32)>,
    /// The cells captured by a function read from the props, and where they are through it.
    function_cells: Vec<(u32, u32)>,
    functions: Vec<(u32, Registered)>,
    /// What few renders have, made when one does, so that a long list's rows take no more room each than before.
    more: Option<Box<More>>,
    /// Everything above, for the collector.
    held: Vec<Value>,
}

/// What a render that gave something on, looked something up, or was given something by another has.
struct More {
    /// What was given to elements the app's JSX made, as props: where it was read, which prop it was given as, and how many
    /// times.
    given: Vec<(u32, Value, u32)>,
    /// What looking something up in what was read gave, as `selected.has(path)` does: where what it was called on was, what
    /// was called, what with, and what it gave.
    lookups: Vec<(u32, Value, Box<[Value]>, Value)>,
    /// The renders of the components what was given went to, which were linked to this one when they rendered or skipped
    /// rendering inside what this rendered: see [`Vm::link_child`].
    children: Vec<u32>,
    /// The render this one's component was given part of what that read by, or `NO_READS`, and which of its props were, from
    /// where.
    parent: u32,
    from_parent: Vec<(Value, u32)>,
}

impl Record {
    fn new(owner: u32, function: Value, props: Value, cautious: bool) -> Record {
        Record {
            owner,
            function,
            props,
            cautious,
            frame: None,
            gave_up: false,
            paths: vec![Path {
                parent: u32::MAX,
                step: Step::Key(0),
                key: Value::UNDEFINED,
            }],
            entries: Vec::new(),
            path_index: None,
            entry_index: None,
            comparisons: Vec::new(),
            comparison_index: None,
            cells: Vec::new(),
            function_cells: Vec::new(),
            functions: Vec::new(),
            more: None,
            held: vec![function, props],
        }
    }

    fn path(&mut self, parent: u32, step: Step, key: Value) -> u32 {
        let found = match &self.path_index {
            Some(index) => index.get(&(parent, step)).copied(),
            None => self
                .paths
                .iter()
                .position(|path| path.parent == parent && path.step == step)
                .map(|at| at as u32),
        };
        if let Some(at) = found {
            return at;
        }
        self.paths.push(Path { parent, step, key });
        if key.is_string() || key.is_symbol() {
            self.held.push(key);
        }
        let at = (self.paths.len() - 1) as u32;
        match &mut self.path_index {
            Some(index) => {
                index.insert((parent, step), at);
            }
            None if self.paths.len() >= INDEX_FROM => {
                let mut index = HashMap::with_capacity_and_hasher(self.paths.len() * 2, FixedState::default());
                // The first of two the same is the one looking along them finds, though there are never two.
                for (at, path) in self.paths.iter().enumerate().rev() {
                    index.insert((path.parent, path.step), at as u32);
                }
                self.path_index = Some(Box::new(index));
            }
            None => {}
        }
        at
    }

    /// Where the entry for `path` is in `entries`, if there's one.
    fn entry_at(&self, path: u32) -> Option<usize> {
        match &self.entry_index {
            Some(index) => index.get(&path).map(|&at| at as usize),
            None => self.entries.iter().position(|entry| entry.path == path),
        }
    }

    /// Notes that what's at `path` was compared with `other`, and whether it was the same, unless that's been noted.
    fn compared(&mut self, path: u32, other: Value, was: bool) {
        let key = (path, other.bits(), was);
        let known = match &self.comparison_index {
            Some(index) => index.contains(&key),
            None => self
                .comparisons
                .iter()
                .any(|&(at, with, same)| at == path && with.bits() == other.bits() && same == was),
        };
        if known {
            return;
        }
        self.comparisons.push((path, other, was));
        self.held.push(other);
        match &mut self.comparison_index {
            Some(index) => {
                index.insert(key);
            }
            None if self.comparisons.len() >= INDEX_FROM => {
                let index = self.comparisons.iter().map(|&(at, with, same)| (at, with.bits(), same)).collect();
                self.comparison_index = Some(Box::new(index));
            }
            None => {}
        }
    }

    /// Adds an entry, keeping the table of them up to date.
    fn push_entry(&mut self, entry: Entry) {
        self.entries.push(entry);
        let at = (self.entries.len() - 1) as u32;
        match &mut self.entry_index {
            Some(index) => {
                index.entry(entry.path).or_insert(at);
            }
            None if self.entries.len() >= INDEX_FROM => {
                let mut index = HashMap::with_capacity_and_hasher(self.entries.len() * 2, FixedState::default());
                for (at, one) in self.entries.iter().enumerate().rev() {
                    index.insert(one.path, at as u32);
                }
                self.entry_index = Some(Box::new(index));
            }
            None => {}
        }
    }

    fn note(&mut self, path: u32, value: Value, itself: bool, careful: bool) {
        if let Some(entry) = self.entry_at(path).map(|at| &mut self.entries[at]) {
            if entry.value != value {
                entry.changing = true;
            }
            entry.itself |= itself;
            entry.careful |= careful;
            entry.only_given = false;
            return;
        }
        self.push_entry(Entry {
            path,
            value,
            itself,
            changing: false,
            careful,
            only_given: false,
        });
        self.held.push(value);
    }

    /// Notes that what was read at `path` was given to an element the app's JSX made, as its prop `prop`.
    fn note_given(&mut self, path: u32, value: Value, prop: Value) {
        if let Some(entry) = self.entry_at(path).map(|at| &mut self.entries[at]) {
            if entry.value != value {
                entry.changing = true;
            }
        } else {
            self.push_entry(Entry {
                path,
                value,
                itself: false,
                changing: false,
                careful: false,
                only_given: true,
            });
            self.held.push(value);
        }
        let more = self.more_mut();
        let counted = match more.given.iter_mut().find(|(at, name, _)| *at == path && *name == prop) {
            Some((_, _, count)) => {
                *count += 1;
                true
            }
            None => {
                more.given.push((path, prop, 1));
                false
            }
        };
        if !counted {
            self.held.push(prop);
        }
    }

    fn more_mut(&mut self) -> &mut More {
        self.more.get_or_insert_with(|| {
            Box::new(More {
                given: Vec::new(),
                lookups: Vec::new(),
                children: Vec::new(),
                parent: NO_READS,
                from_parent: Vec::new(),
            })
        })
    }

    fn given(&self) -> &[(u32, Value, u32)] {
        self.more.as_ref().map_or(&[], |more| &more.given)
    }

    fn lookups(&self) -> &[(u32, Value, Box<[Value]>, Value)] {
        self.more.as_ref().map_or(&[], |more| &more.lookups)
    }

    fn children(&self) -> &[u32] {
        self.more.as_ref().map_or(&[], |more| &more.children)
    }

    fn parent(&self) -> u32 {
        self.more.as_ref().map_or(NO_READS, |more| more.parent)
    }

    fn from_parent(&self) -> &[(Value, u32)] {
        self.more.as_ref().map_or(&[], |more| &more.from_parent)
    }

    /// Where the prop named `prop` of what was given is, if it was read.
    fn prop_path(&self, prop: Value) -> Option<u32> {
        self.paths
            .iter()
            .position(|path| path.parent == 0 && path.step == Step::Key(prop.bits()))
            .map(|at| at as u32)
    }

    fn registered(&self, function: u32) -> Option<&Registered> {
        self.functions
            .iter()
            .find(|(one, _)| *one == function)
            .map(|(_, registered)| registered)
    }

    /// Whether what it read can be gone by: its own frame was found, and nothing it did was more than could be noted.
    fn usable(&self) -> bool {
        self.frame.is_some() && !self.gave_up
    }
}

/// Every render's reads, and what's being rendered now.
#[derive(Default)]
pub(crate) struct Reads {
    records: Vec<Option<Box<Record>>>,
    free: Vec<u32>,
    /// What's rendering, innermost last, [`NO_READS`] for what isn't noted.
    stack: Vec<u32>,
    /// Each cell a component's props are in, or a name destructured from them, by its number: whose it is, and where.
    alias_cells: HashMap<u32, (u32, u32), FixedState>,
    /// Each function read from props and called as something rendered, by its object's number: what read it.
    readers: HashMap<u32, Vec<u32>, FixedState>,
    /// The owners of what read a function that's been handed on, and now reads something else.
    unheld: Vec<u32>,
    /// The closures that hold each block of captured variables that's been shared by handing on, by where it is: see
    /// [`Vm::join_handed`].
    groups: HashMap<usize, Vec<u32>, FixedState>,
    /// What's been read to be given to a call, which is read as itself unless what it's given to notes what it reads of it:
    /// see `NOTE_ARG`.
    given: Vec<Given>,
    /// Which call what's been given was last told it's for (`NOTE_CALLING`), for what's called to take.
    calling: u32,
    /// The frames of functions that note what they read of their parameters, with where each was read from: see
    /// `NOTE_PARAMS`.
    params: Vec<ParamFrame>,
    /// A lookup that's been read to be called, as `selected.has`, until what it gives is known: see `NOTE_RESULT`.
    lookup: Option<Lookup>,
}

/// A lookup about to be called on what was read: whose read, where what it's called on is, and in which frame.
#[derive(Clone, Copy)]
struct Lookup {
    id: u32,
    path: u32,
    on: Value,
    method: Value,
    itself: bool,
    depth: u32,
}

/// Something that was read to be given to a call, which `NOTE_ARG` says, until what it's given to takes it.
#[derive(Clone, Copy)]
struct Given {
    /// Whose read it was, and where.
    id: u32,
    path: u32,
    value: Value,
    /// The frame it was read in, by its depth, and where on the stack it's given from, which is where what's called finds it.
    depth: u32,
    at: u32,
    /// The call it's been told it's for, or 0 before that's known.
    calling: u32,
}

/// A frame of a function that notes what it reads of its parameters, and where each of those was read from.
struct ParamFrame {
    depth: u32,
    base: u32,
    /// Each parameter's register, whose read it was, and where.
    params: Vec<(u16, u32, u32)>,
}

/// What a Note says was read: what it started from, the keys it went through, what came out, and how it was read.
struct Read<'k> {
    from: Value,
    keys: &'k [Key],
    base: usize,
    /// Where what was read is on the stack, for what's given to a call.
    at: usize,
    /// The prop it's given as, to an element the app's JSX made.
    prop: Option<Value>,
    leaf: Value,
    kind: u8,
    this: Option<Value>,
    compared: Option<Value>,
}

/// A key of something read, from after a Note.
#[derive(Clone, Copy)]
enum Key {
    Register(u16),
    Name(u32),
}

impl Reads {
    /// The host's number for what rendered, for the render `id`.
    pub(crate) fn owner_of(&self, id: u32) -> Option<u32> {
        self.records.get(id as usize)?.as_ref().map(|record| record.owner)
    }

    /// Whether the block of captured variables at `block` is held by more than one function, as handing on makes it.
    pub(crate) fn shares_captures(&self, block: usize) -> bool {
        !self.groups.is_empty() && self.groups.contains_key(&block)
    }

    /// Forgets the closures that shared captured variables by handing on that the collector has just freed, which `freed`
    /// says of a place in the list of objects. Nothing else holds them, so a number of one left here could be another
    /// object's once its place is used again, or be on a page given back (see `giveback.rs`), which can't be read. What's
    /// left with fewer than two shares nothing any more.
    pub(crate) fn forget_freed_closures(&mut self, freed: impl Fn(u32) -> bool) {
        if self.groups.is_empty() {
            return;
        }
        self.groups.retain(|_, members| {
            members.retain(|member| !freed(*member));
            members.len() > 1
        });
    }
}

impl Vm {
    /// Starts noting what's read while a component renders: `function` given `props`, for `owner`, or nothing but what
    /// renders without being noted when `noted` is false. Each of these has to be followed by [`end_reads`](Self::end_reads).
    /// What it gives is for [`reads_same`](Self::reads_same), and has to be let go of with [`drop_reads`](Self::drop_reads).
    pub fn start_reads(&mut self, owner: u32, function: Value, props: Value, noted: bool, cautious: bool) -> u32 {
        let id = if noted && reads_enabled() {
            let record = Box::new(Record::new(owner, function, props, cautious));
            match self.reads.free.pop() {
                Some(id) => {
                    self.reads.records[id as usize] = Some(record);
                    id
                }
                None => {
                    self.reads.records.push(Some(record));
                    (self.reads.records.len() - 1) as u32
                }
            }
        } else {
            NO_READS
        };
        self.reads.stack.push(id);
        self.tracking = true;
        if id != NO_READS {
            self.keeping.restart(id);
        }
        id
    }

    /// Starts noting what's read while `function` is called to make what something renders, as a long list's `renderRow` is
    /// for each row, for `owner`: what it reads of what it captured is noted through it, so that the same function made again
    /// seeing what reads the same, as [`reads_same`](Self::reads_same) says given the new one, would make the same. Followed by
    /// [`end_reads`](Self::end_reads), as `start_reads` is.
    pub fn start_function_reads(&mut self, owner: u32, function: Value) -> u32 {
        let id = self.start_reads(owner, function, function, self.is_app_closure(function), false);
        if let Some(record) = self.reads.records.get_mut(id as usize).and_then(Option::as_mut) {
            // No component's frame is what's noted: what's read through the function is.
            record.frame = Some((0, 0));
            self.register_function(id, function, 0);
        }
        id
    }

    /// Stops noting for what [`start_reads`](Self::start_reads) last started.
    pub fn end_reads(&mut self) {
        if let Some(id) = self.reads.stack.pop() {
            // What it gave to what didn't take it was read as itself, and what it was about to look something up in, whole.
            self.flush_given(|given| given.id == id);
            if self.reads.lookup.is_some_and(|lookup| lookup.id == id) {
                self.flush_lookup();
            }
            self.reads.params.retain(|frame| frame.params.iter().all(|&(_, owner, _)| owner != id));
        }
        self.tracking = !self.reads.stack.is_empty();
        if !self.tracking {
            self.reads.given.clear();
            self.reads.params.clear();
            self.keeping.rendered();
        }
    }

    /// Notes what a lookup that's waiting for what it gives was to be called on as read whole, as it would have been.
    fn flush_lookup(&mut self) {
        if let Some(lookup) = self.reads.lookup.take()
            && let Some(record) = self.reads.records.get_mut(lookup.id as usize).and_then(Option::as_mut)
            && (lookup.path as usize) < record.paths.len()
        {
            record.note(lookup.path, lookup.on, lookup.itself, true);
        }
    }

    /// Whether calling `method` on `on` only looks something up in it, with nothing of the app's run: a `Set`'s or a `Map`'s
    /// `has`, or a `Map`'s `get`.
    fn only_looks_up(&self, method: Value, on: Value) -> bool {
        if !method.is_object() || !on.is_object() {
            return false;
        }
        let Kind::Native(native) = &self.objects[method.index()].kind else {
            return false;
        };
        let func = native.func as usize;
        let on = &self.objects[on.index()].kind;
        (func == crate::builtins::collections::collection_has as usize && matches!(on, Kind::Set(_) | Kind::Map(_)))
            || (func == crate::builtins::collections::map_get as usize && matches!(on, Kind::Map(_)))
    }

    /// Notes what was read to be given to a call, and that `which` says wasn't taken, as read as itself.
    fn flush_given(&mut self, which: impl Fn(&Given) -> bool) {
        if self.reads.given.is_empty() {
            return;
        }
        let given = std::mem::take(&mut self.reads.given);
        let mut kept = Vec::with_capacity(given.len());
        for one in given {
            if which(&one) {
                if let Some(record) = self.reads.records.get_mut(one.id as usize).and_then(Option::as_mut)
                    && (one.path as usize) < record.paths.len()
                {
                    record.note(one.path, one.value, true, false);
                }
            } else {
                kept.push(one);
            }
        }
        self.reads.given = kept;
    }

    /// Lets go of what a render read.
    pub fn drop_reads(&mut self, id: u32) {
        let Some(record) = self.reads.records.get_mut(id as usize).and_then(Option::take) else {
            return;
        };
        self.forget_record(id, &record);
        self.reads.free.push(id);
    }

    fn forget_record(&mut self, id: u32, record: &Record) {
        if record.parent() != NO_READS
            && let Some(parent) = self.reads.records.get_mut(record.parent() as usize).and_then(Option::as_mut)
            && let Some(more) = parent.more.as_mut()
        {
            more.children.retain(|child| *child != id);
        }
        for &child in record.children() {
            if let Some(child) = self.reads.records.get_mut(child as usize).and_then(Option::as_mut)
                && let Some(more) = child.more.as_mut()
                && more.parent == id
            {
                more.parent = NO_READS;
                more.from_parent.clear();
            }
        }
        for (cell, _) in &record.cells {
            if self.reads.alias_cells.get(&(cell.index() as u32)).is_some_and(|(owner, _)| *owner == id) {
                self.reads.alias_cells.remove(&(cell.index() as u32));
            }
        }
        for (function, registered) in &record.functions {
            if let Registered::Read(_) = registered
                && let Some(readers) = self.reads.readers.get_mut(function)
            {
                readers.retain(|reader| *reader != id);
                if readers.is_empty() {
                    self.reads.readers.remove(function);
                }
            }
        }
    }

    /// What's held by every render's reads, for the collector.
    pub(crate) fn each_read_value(&self, mut each: impl FnMut(Value)) {
        for record in self.reads.records.iter().flatten() {
            for value in &record.held {
                each(*value);
            }
            for (cell, _) in &record.cells {
                each(*cell);
            }
        }
        self.keeping.each_value(&mut each);
    }

    /// The owners of renders that read a function that's since been made to do what a new one does, and now read something
    /// else, which have to render again.
    pub fn take_unheld_readers(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.reads.unheld)
    }

    /// What's rendering now, as [`start_reads`](Self::start_reads) gave it, or [`NO_READS`].
    pub fn current_reads(&self) -> u32 {
        self.reads.stack.last().copied().unwrap_or(NO_READS)
    }

    /// Links what `child` read, a component rendered with `props` inside what `parent` rendered, or kept from rendering there,
    /// to what `parent` gave it: each of its props that's what `parent` read and gave as that prop to an element the app's JSX
    /// made. When `parent` is given new props in which what it gave is different, it's the same as long as what each component
    /// it was given to read of it is, and every place it was given is one of those: see `given_hold`.
    pub fn link_child(&mut self, parent: u32, child: u32, props: Value) {
        if parent == NO_READS || child == NO_READS || parent == child {
            return;
        }
        let Some(record) = self.reads.records.get(parent as usize).and_then(Option::as_ref) else {
            return;
        };
        if record.given().is_empty() {
            return;
        }
        let wanted: Vec<(u32, Value, Value)> = record
            .given()
            .iter()
            .filter_map(|&(path, prop, _)| {
                let entry = record.entries.iter().find(|entry| entry.path == path && entry.only_given)?;
                Some((path, prop, entry.value))
            })
            .collect();
        let mut links = Vec::new();
        for (path, prop, value) in wanted {
            if self.plain_step(props, prop) == Some(value) {
                links.push((prop, path));
            }
        }
        if links.is_empty() {
            return;
        }
        let Some(record) = self.reads.records.get_mut(child as usize).and_then(Option::as_mut) else {
            return;
        };
        for &(prop, _) in &links {
            record.held.push(prop);
        }
        let more = record.more_mut();
        let before = std::mem::replace(&mut more.parent, parent);
        more.from_parent = links;
        if before != NO_READS
            && before != parent
            && let Some(old) = self.reads.records.get_mut(before as usize).and_then(Option::as_mut)
            && let Some(more) = old.more.as_mut()
        {
            more.children.retain(|one| *one != child);
        }
        if let Some(parent) = self.reads.records.get_mut(parent as usize).and_then(Option::as_mut) {
            let more = parent.more_mut();
            if !more.children.contains(&child) {
                more.children.push(child);
            }
        }
    }

    /// Has `new`, what a component that rendered again by itself read, given what it was given as `old` was, take `old`'s
    /// place among what was given what it's in read, as it's given the same props.
    pub fn move_link(&mut self, old: u32, new: u32) {
        if old == NO_READS || new == NO_READS || old == new {
            return;
        }
        let Some(record) = self.reads.records.get_mut(old as usize).and_then(Option::as_mut) else {
            return;
        };
        let Some(more) = record.more.as_mut() else {
            return;
        };
        let parent = std::mem::replace(&mut more.parent, NO_READS);
        let links = std::mem::take(&mut more.from_parent);
        if parent == NO_READS {
            return;
        }
        if let Some(record) = self.reads.records.get_mut(parent as usize).and_then(Option::as_mut)
            && let Some(more) = record.more.as_mut()
        {
            for one in &mut more.children {
                if *one == old {
                    *one = new;
                }
            }
        }
        if let Some(record) = self.reads.records.get_mut(new as usize).and_then(Option::as_mut) {
            for &(prop, _) in &links {
                record.held.push(prop);
            }
            let more = record.more_mut();
            more.parent = parent;
            more.from_parent = links;
        }
    }

    /// Whether what `record` read at `path`, and only gave on to elements as a prop, is the same as `now` for what it was given
    /// to: each of the components it went to, linked to it (see [`link_child`](Self::link_child)), read the same of it, and
    /// together they're everywhere it was given. Each is noted in `deferred`, to be given `now` once everything's the same.
    #[allow(clippy::too_many_arguments)]
    fn given_hold(
        &mut self,
        record: &Record,
        path: u32,
        now: Value,
        anew: bool,
        opaque: Value,
        handed: &mut Vec<(Value, Value)>,
        deferred: &mut Vec<(u32, Vec<(Value, Value)>)>,
    ) -> bool {
        let gives: u32 = record.given().iter().filter(|(at, ..)| *at == path).map(|(.., count)| *count).sum();
        if gives == 0 {
            return false;
        }
        let mut linked = 0;
        for &child in record.children() {
            let Some(found) = self.reads.records.get_mut(child as usize).and_then(Option::take) else {
                continue;
            };
            let over: Vec<(Value, Value)> = found
                .from_parent()
                .iter()
                .filter(|(_, at)| *at == path)
                .map(|&(prop, _)| (prop, now))
                .collect();
            if over.is_empty() {
                self.reads.records[child as usize] = Some(found);
                continue;
            }
            // What read its props whole, or was compared when they were, read what was given too.
            let whole = found.entries.iter().any(|entry| entry.path == 0) || found.comparisons.iter().any(|(at, ..)| *at == 0);
            let holds = found.usable()
                && !whole
                && self.record_holds(&found, found.props, anew, opaque, handed, &over, deferred);
            linked += over.len() as u32;
            self.reads.records[child as usize] = Some(found);
            if !holds {
                return false;
            }
            deferred.push((child, over));
        }
        linked == gives
    }

    /// Gives each component what was given on to it in place of what it was given before, as its props say from now on (see
    /// `given_hold`), with what it made that sees them through a cell.
    fn give_deferred(&mut self, deferred: Vec<(u32, Vec<(Value, Value)>)>) {
        for (child, over) in deferred {
            let Some(record) = self.reads.records.get_mut(child as usize).and_then(Option::take) else {
                continue;
            };
            for &(prop, value) in &over {
                if let Ok(atom) = self.to_atom(prop) {
                    let _ = self.set_property(record.props, atom, value);
                }
            }
            for (cell, path) in &record.cells {
                if let Some(value) = self.walk(&record, *path, record.props) {
                    self.cells[cell.index()] = value;
                }
            }
            self.reads.records[child as usize] = Some(record);
        }
    }

    /// Says a function is kept as itself, as a hook's dependency is, so that it isn't made to do what another does.
    pub fn keep_as_itself(&mut self, value: Value) {
        if value.is_object() && matches!(self.objects[value.index()].kind, Kind::Function { .. }) {
            self.objects.add_flags(value.index(), NOT_HANDED_ON);
        }
    }

    /// Whether a component that rendered with what `id` noted would show and do the same given `new`: everything it read is
    /// the same there, by the rules of [`props_equivalent_handing_on`](Self::props_equivalent_handing_on), handlers and
    /// functions read from them handed on. If so, what it made that sees its props, or a name destructured from them, now
    /// sees the new ones, as a new render's would, and what read a function that's been handed on is looked at again: what
    /// now reads something else is given by [`take_unheld_readers`](Self::take_unheld_readers), to be rendered again.
    pub fn reads_same(&mut self, id: u32, new: Value, opaque: Value) -> bool {
        let mut handed = Vec::new();
        if !self.check_record(id, new, opaque, &mut handed) {
            return false;
        }
        self.look_again_at_readers(id, handed, opaque);
        true
    }

    /// [`reads_same`](Self::reads_same) but for looking again at what read the functions handed on, which are put in
    /// `handed`. Given the props it already has, as it is when it's looked at again, it's only asked whether what was handed
    /// on changed what it read.
    fn check_record(&mut self, id: u32, new: Value, opaque: Value, handed: &mut Vec<(Value, Value)>) -> bool {
        let Some(record) = self.reads.records.get_mut(id as usize).and_then(Option::take) else {
            return false;
        };
        let mut deferred = Vec::new();
        let anew = new != record.props;
        let same = record.usable() && self.record_holds(&record, new, anew, opaque, handed, &[], &mut deferred);
        let mut record = record;
        if same {
            self.reads.records[id as usize] = Some(record);
            self.give_deferred(deferred);
            let Some(taken) = self.reads.records.get_mut(id as usize).and_then(Option::take) else {
                return true;
            };
            record = taken;
            // What the last render made that sees the props, or a name destructured from them, now sees the new ones.
            for (cell, path) in &record.cells {
                if let Some(value) = self.walk(&record, *path, new) {
                    self.cells[cell.index()] = value;
                }
            }
            record.props = new;
            record.held[1] = new;
        }
        self.reads.records[id as usize] = Some(record);
        same
    }

    /// Whether everything `record` read is the same in `new`, with each prop in `over` as it says instead, which is what was
    /// given in its place. What's only been given on to the components below, as a prop, is the same as long as what those
    /// read of it is, which, if so, is to be given to them in `deferred`. Unless it's given `anew`, the same object it read
    /// is the same even where it's cautious (see `cautious` in [`Vm::props_equivalent`]): it's only being asked whether
    /// what's been handed on reads something else, and what's read through a function that's handed on is noted through it.
    #[allow(clippy::too_many_arguments)]
    fn record_holds(
        &mut self,
        record: &Record,
        new: Value,
        anew: bool,
        opaque: Value,
        handed: &mut Vec<(Value, Value)>,
        over: &[(Value, Value)],
        deferred: &mut Vec<(u32, Vec<(Value, Value)>)>,
    ) -> bool {
        let mut looks = crate::embed::Looks::new(crate::embed::EQUIVALENT_LOOKS, opaque);
        looks.hand_on = true;
        let mut values: Vec<Option<Option<Value>>> = vec![None; record.paths.len()];
        values[0] = Some(Some(new));
        for &(prop, value) in over {
            if let Some(at) = record.prop_path(prop) {
                values[at as usize] = Some(Some(value));
            }
        }
        for entry in &record.entries {
            if entry.changing {
                return false;
            }
            let Some(now) = self.walk_into(record, entry.path, &mut values) else {
                return false;
            };
            let old = entry.value;
            let same = if entry.itself {
                old == now || self.same_value(old, now)
            } else {
                let cautious = anew && entry.careful && record.cautious && !self.under_capture(record, entry.path);
                let mark = looks.handed.len();
                let equal = self.equivalent(old, now, crate::embed::EQUIVALENT_DEPTH, &mut looks, cautious);
                if !equal {
                    looks.handed.truncate(mark);
                }
                equal || self.hands_on_value(record, entry.path, old, now, &mut looks)
            };
            if !same && !(entry.only_given && self.given_hold(record, entry.path, now, anew, opaque, handed, deferred)) {
                return false;
            }
        }
        for &(path, other, was) in &record.comparisons {
            let Some(now) = self.walk_into(record, path, &mut values) else {
                return false;
            };
            if self.strict_equals(now, other) != was {
                return false;
            }
        }
        for (path, method, given, gave) in record.lookups() {
            let Some(now) = self.walk_into(record, *path, &mut values) else {
                return false;
            };
            if !self.only_looks_up(*method, now) {
                return false;
            }
            match self.call_function(*method, now, given) {
                Ok(result) if result == *gave || self.same_value(result, *gave) => {}
                _ => return false,
            }
        }
        // Each name what it read is destructured as has to be there to be handed on.
        for (_, path) in &record.cells {
            if self.walk_into(record, *path, &mut values).is_none() {
                return false;
            }
        }
        handed.extend(self.hand_on(&mut looks));
        true
    }

    /// Looks again at each render that read a function that's been handed on, other than `checked`'s, which might now read
    /// something else: those that do are to render again, and those that don't hand on what they read in turn.
    pub(crate) fn look_again_at_readers(&mut self, checked: u32, mut handed: Vec<(Value, Value)>, opaque: Value) {
        let mut seen = vec![checked];
        while let Some((old, new)) = handed.pop() {
            // What read any of the closures that now hold what `new` sees, as each was handed on to some time.
            let block = match &self.objects[new.index()].kind {
                Kind::Function { upvalues, .. } => upvalues.block(),
                _ => 0,
            };
            let holders = if block == 0 { vec![old.index() as u32] } else { self.holding(block, old) };
            let mut readers: Vec<u32> = Vec::new();
            for holder in holders {
                for reader in self.reads.readers.get(&holder).into_iter().flatten() {
                    if !readers.contains(reader) {
                        readers.push(*reader);
                    }
                }
            }
            for reader in readers {
                if seen.contains(&reader) {
                    continue;
                }
                seen.push(reader);
                let Some(props) = self.reads.records.get(reader as usize).and_then(|record| record.as_ref().map(|r| r.props))
                else {
                    continue;
                };
                if !self.check_record(reader, props, opaque, &mut handed) {
                    let owner = self.reads.records[reader as usize].as_ref().map_or(u32::MAX, |r| r.owner);
                    if !self.reads.unheld.contains(&owner) {
                        self.reads.unheld.push(owner);
                    }
                }
            }
        }
    }

    /// Has `old`, and every closure that holds the same captured variables as it, hold `new`'s from now on, in the same
    /// block, so that what's handed on to any of them later reaches all of them. A child that skipped rendering holds the
    /// old function while what it's in holds the new one it was handed, or keeps an older one still, as a page element
    /// does, and once one of those is the one handed on again, the others have to follow. If one of them is running, as it
    /// can be while something renders because of what it did, its block stays where it is, and gets the new ones put in it
    /// instead. Whether `old` now sees anything other than it did is given back: not if the two already held the same block,
    /// as two kept apart, by an element that kept the old one, do once they've been joined, or if neither captured anything.
    pub(crate) fn join_handed(&mut self, old: Value, new: Value) -> bool {
        let block_of = |vm: &Vm, value: Value| match &vm.objects[value.index()].kind {
            Kind::Function { upvalues, .. } => Some(upvalues.block()),
            _ => None,
        };
        let (Some(old_block), Some(new_block)) = (block_of(self, old), block_of(self, new)) else {
            return false;
        };
        if old_block == new_block || old_block == 0 || new_block == 0 {
            return false;
        }
        let members = self.holding(old_block, old);
        let running = self
            .frames
            .iter()
            .any(|frame| frame.upvalues as usize == old_block + std::mem::size_of::<Value>());
        if running {
            let captures: Vec<Value> = match &self.objects[new.index()].kind {
                Kind::Function { upvalues, .. } => upvalues.to_vec(),
                _ => return false,
            };
            for &member in &members {
                // Through `objects[..]`, which is the write barrier, for each that holds the block.
                if let Kind::Function { upvalues, .. } = &mut self.objects[member as usize].kind
                    && upvalues.block() == old_block
                {
                    for (at, value) in captures.iter().enumerate() {
                        upvalues.set(at, *value);
                    }
                }
            }
            return true;
        }
        for &member in &members {
            let shared = match &self.objects[new.index()].kind {
                Kind::Function { upvalues, .. } => upvalues.share(),
                _ => return false,
            };
            if let Kind::Function { upvalues, .. } = &mut self.objects[member as usize].kind {
                *upvalues = shared;
            }
        }
        self.reads.groups.remove(&old_block);
        let group = self.reads.groups.entry(new_block).or_default();
        for member in members.into_iter().chain([new.index() as u32]) {
            if !group.contains(&member) {
                group.push(member);
            }
        }
        true
    }

    /// The closures that hold the block of captured variables at `block`: `one`, and those it's shared with that still do.
    fn holding(&mut self, block: usize, one: Value) -> Vec<u32> {
        let mut members = self.reads.groups.get(&block).cloned().unwrap_or_default();
        members.retain(|member| {
            (*member as usize) < self.objects.len()
                && matches!(&self.objects[*member as usize].kind, Kind::Function { upvalues, .. } if upvalues.block() == block)
        });
        if !members.contains(&(one.index() as u32)) {
            members.push(one.index() as u32);
        }
        members
    }

    /// Whether a path goes through what a function captured, where what's the same as itself is, as it is for what a
    /// closure sees (see `equivalent_inside`).
    fn under_capture(&self, record: &Record, mut path: u32) -> bool {
        while path != 0 && path != u32::MAX {
            let at = record.paths[path as usize];
            if let Step::Capture(..) = at.step {
                return true;
            }
            path = at.parent;
        }
        false
    }

    /// What's at `path` from `root`, as the props are.
    fn walk(&mut self, record: &Record, path: u32, root: Value) -> Option<Value> {
        let mut values: Vec<Option<Option<Value>>> = vec![None; record.paths.len()];
        values[0] = Some(Some(root));
        self.walk_into(record, path, &mut values)
    }

    fn walk_into(&mut self, record: &Record, path: u32, values: &mut [Option<Option<Value>>]) -> Option<Value> {
        if let Some(found) = values[path as usize] {
            return found;
        }
        let at = record.paths[path as usize];
        let found = self
            .walk_into(record, at.parent, values)
            .and_then(|parent| match at.step {
                Step::Key(_) => self.plain_step(parent, at.key),
                Step::Capture(proto, index) => self.captured(parent, proto, index),
            });
        values[path as usize] = Some(found);
        found
    }

    /// What a closure of code `proto` captured as its `index`th, now.
    fn captured(&self, function: Value, proto: u32, index: u16) -> Option<Value> {
        if !function.is_object() {
            return None;
        }
        let Kind::Function {
            proto: its, upvalues, ..
        } = &self.objects[function.index()].kind
        else {
            return None;
        };
        if *its != proto {
            return None;
        }
        let held = *upvalues.get(index as usize)?;
        if self.protos[proto as usize].captures.get(index as usize)?.by_value {
            return Some(held);
        }
        let value = self.cells[held.index()];
        (!value.is_empty()).then_some(value)
    }

    /// What reading `key` of `value` gives, without running anything: a property that holds a value, of an object or what it
    /// inherits from, or an element of an array. Nothing for a getter, a proxy, a string's own properties or anything else
    /// that would need more than looking.
    fn plain_step(&mut self, value: Value, key: Value) -> Option<Value> {
        if !value.is_object() {
            return None;
        }
        let index = if key.is_int() && key.as_int() >= 0 {
            Some(key.as_int() as usize)
        } else {
            None
        };
        let array = match &self.objects[value.index()].kind {
            Kind::Array(items) => Some((items.len(), index.and_then(|index| items.get(index).copied()))),
            _ => None,
        };
        if let Some((length, item)) = array {
            if index.is_some() {
                return match item {
                    Some(item) if item.is_empty() => None,
                    Some(item) => Some(item),
                    None => self.inherited(value, key),
                };
            }
            if key.is_string() && self.string_is(key, "length") {
                return Some(Value::int_or_double(length));
            }
        }
        if !(key.is_string() || key.is_symbol() || index.is_some()) {
            return None;
        }
        self.inherited(value, key)
    }

    fn inherited(&mut self, mut value: Value, key: Value) -> Option<Value> {
        let atom = if key.is_string() || key.is_symbol() || key.is_int() {
            self.to_atom(key).ok()?
        } else {
            return None;
        };
        for _ in 0..16 {
            let object = &self.objects[value.index()];
            match object.kind {
                Kind::Plain | Kind::Array(_) | Kind::Function { .. } => {}
                _ => return None,
            }
            if matches!(object.kind, Kind::Function { .. }) && object.flags & LAZY_FUNCTION != 0 {
                return None;
            }
            let shape = object.shape;
            if let Some((slot, flags)) = self.shapes.lookup(shape, atom) {
                if flags & ACCESSOR != 0 {
                    return None;
                }
                return self.objects[value.index()].slots().get(slot as usize).copied();
            }
            let proto = self.shapes.proto_of(shape);
            if proto == crate::object::NONE {
                return Some(Value::UNDEFINED);
            }
            value = Value::object(proto);
        }
        None
    }

    fn string_is(&mut self, value: Value, text: &str) -> bool {
        self.plain_text(value).is_some_and(|found| found == text)
    }

    /// Whether `old`, read at `path`, can be made to do what `new` does: a handler, if it was read as one (`onClick` and the
    /// rest), or a function that what read it only called as it rendered, whose reads are noted through it.
    fn hands_on_value(&mut self, record: &Record, path: u32, old: Value, new: Value, looks: &mut crate::embed::Looks) -> bool {
        let at = record.paths[path as usize];
        let named = matches!(at.step, Step::Key(_)) && at.key.is_string() && self.plain_text(at.key).is_some_and(|name| {
            let name = name.as_bytes();
            name.len() > 2 && name.starts_with(b"on") && name[2].is_ascii_uppercase()
        });
        if named && self.handler_pair(old, new) {
            return self.note_handed(old, new, looks);
        }
        self.function_pair(old, new) && self.note_handed(old, new, looks)
    }

    /// Notes a function to be handed on: the same one can only be made to do what one new one does.
    pub(crate) fn note_handed(&self, old: Value, new: Value, looks: &mut crate::embed::Looks) -> bool {
        match looks.handed.iter().find(|(was, _)| *was == old) {
            Some((_, now)) => *now == new,
            None => {
                looks.handed.push((old, new));
                true
            }
        }
    }

    /// Whether two functions are made from the same code in the app's own files, which can be handed on: neither has had its
    /// own properties asked for, and it isn't async or a generator, which can be waiting part of the way through.
    pub(crate) fn handler_pair(&self, old: Value, new: Value) -> bool {
        if !old.is_object() || !new.is_object() {
            return false;
        }
        let (first, second) = (&self.objects[old.index()], &self.objects[new.index()]);
        let (
            Kind::Function { proto, home, upvalues, .. },
            Kind::Function { proto: other_proto, home: other_home, upvalues: other_upvalues, .. },
        ) = (&first.kind, &second.kind)
        else {
            return false;
        };
        let untouched = |object: &crate::object::Object| object.flags & LAZY_FUNCTION != 0 && object.slots().is_empty();
        let code = &self.protos[*proto as usize];
        proto == other_proto
            && home == other_home
            && upvalues.len() == other_upvalues.len()
            && untouched(first)
            && untouched(second)
            && code.says & SAYS_APP_FILE != 0
            && code.flags & (IS_ASYNC | IS_GENERATOR | IS_CLASS) == 0
    }

    /// [`handler_pair`](Self::handler_pair) for a function that isn't named as a handler, as `renderRow` or `pane`, which
    /// is called as something renders: only one whose every call while something rendered was noted, and that no hook
    /// keeps as itself (see [`NOT_HANDED_ON`]), as what called it is looked at again once it's handed on.
    pub(crate) fn function_pair(&self, old: Value, new: Value) -> bool {
        reads_enabled() && self.handler_pair(old, new) && self.objects[old.index()].flags & NOT_HANDED_ON == 0
    }

    // Noting, as Note says.

    /// Does the Note at `at` in the code, for the frame on top.
    #[cold]
    #[allow(unsafe_code)]
    pub(crate) fn note(&mut self, at: usize) {
        let Op::Note { value, root, count, kind } = self.code[at] else {
            return;
        };
        let Some(&current) = self.reads.stack.last() else {
            return;
        };
        let Some(frame) = self.frames.last() else {
            return;
        };
        let (base, closure, upvalues, depth) =
            (frame.base as usize, frame.closure, frame.upvalues, self.frames.len() as u32);
        // Most reads while something renders are of what isn't noted: a component's own read when it isn't the one being
        // rendered, or a captured variable that's neither a destructured name's cell nor seen through a function read from
        // the props, which is only marked. Those go no further.
        match kind & NOTE_KINDS {
            NOTE_OWN | NOTE_CELL if !self.own_frame(current, depth, base) => return,
            NOTE_PARAM if self.reads.params.is_empty() => return,
            NOTE_UP if !self.up_is_noted(current, closure, upvalues, root) => {
                self.objects.add_flags(closure as usize, NOT_HANDED_ON);
                return;
            }
            _ => {}
        }
        let mut keys = [Key::Register(0); MOST_STEPS + 2];
        let count = (count as usize).min(keys.len());
        for (place, op) in keys.iter_mut().zip(&self.code[at + 1..at + 1 + count]) {
            *place = match *op {
                Op::NoteKey { reg } => Key::Register(reg),
                Op::NoteName { atom } => Key::Name(atom),
                _ => return,
            };
        }
        let mut keys = &keys[..count];
        let register = |vm: &Vm, reg: u16| vm.stack[base + reg as usize];
        let at = base + value as usize;
        // Given as a prop to an element the app's JSX made, which the last key names.
        let mut prop = None;
        if kind & NOTE_ARG != 0
            && kind & NOTE_SAME_ITSELF == 0
            && let Some((Key::Name(atom), rest)) = keys.split_last()
        {
            prop = Some(self.atom_value(Atom(*atom)));
            keys = rest;
        }
        match kind & NOTE_KINDS {
            NOTE_CALLING => {
                // What was given to a call before this one and not taken is read as itself.
                self.flush_given(|given| given.calling != 0);
                self.reads.calling = self.reads.calling.wrapping_add(1).max(1);
                let calling = self.reads.calling;
                let (start, end) = (at as u32, (at + root as usize) as u32);
                for given in &mut self.reads.given {
                    if given.depth == depth && given.at >= start && given.at < end {
                        given.calling = calling;
                    }
                }
            }
            NOTE_PARAMS => self.take_params(depth, base, value, root),
            NOTE_RESULT => {
                let Some(lookup) = self.reads.lookup else {
                    return;
                };
                if lookup.depth != depth {
                    return;
                }
                self.reads.lookup = None;
                let Some(Key::Register(argc)) = keys.first().copied() else {
                    return;
                };
                let start = base + root as usize + 2;
                let given: Box<[Value]> = self.stack[start..start + argc as usize].into();
                let result = register(self, value);
                if let Some(record) = self.reads.records.get_mut(lookup.id as usize).and_then(Option::as_mut) {
                    record.held.push(lookup.method);
                    record.held.extend(given.iter().copied());
                    record.held.push(result);
                    record.more_mut().lookups.push((lookup.path, lookup.method, given, result));
                }
            }
            NOTE_PARAM => {
                let Some((owner, start)) = self.reads.params.iter().rev().find(|frame| frame.depth == depth && frame.base == base as u32).and_then(
                    |frame| frame.params.iter().find(|(reg, ..)| *reg == root).map(|&(_, owner, path)| (owner, path)),
                ) else {
                    return;
                };
                let this = self.called_on(&mut keys, kind, base);
                let compared = self.compared_with(&mut keys, kind, base);
                let from = register(self, root);
                let leaf = register(self, value);
                let read = Read { from, keys, base, at, prop, leaf, kind, this, compared };
                self.record_read(owner, current, start, &read);
            }
            NOTE_ENTER => {
                if current == NO_READS {
                    return;
                }
                let props = register(self, value);
                let Some(record) = self.reads.records[current as usize].as_mut() else {
                    return;
                };
                if record.function == Value::object(closure) && record.props == props {
                    if record.frame.is_some() {
                        // It's being called again, as a render that started again does.
                        let owner = record.owner;
                        let cautious = record.cautious;
                        let function = record.function;
                        let old = std::mem::replace(record.as_mut(), Record::new(owner, function, props, cautious));
                        self.forget_record(current, &old);
                        // What it was about to give was where in what it read then, which is nothing now.
                        self.reads.given.retain(|given| given.id != current);
                        self.keeping.restart(current);
                        self.reads.params.retain(|frame| frame.params.iter().all(|&(_, owner, _)| owner != current));
                    }
                    if let Some(record) = self.reads.records[current as usize].as_mut() {
                        record.frame = Some((depth, base as u32));
                    }
                }
            }
            NOTE_CELL => {
                if !self.own_frame(current, depth, base) {
                    return;
                }
                let cell = register(self, value);
                if !cell.is_cell() {
                    return;
                }
                let path = match keys.first() {
                    Some(Key::Name(atom)) if kind & NOTE_ALIASED != 0 => {
                        let key = self.atom_value(Atom(*atom));
                        let record = self.reads.records[current as usize].as_mut().expect("being rendered");
                        record.path(0, Step::Key(key.bits()), key)
                    }
                    _ => 0,
                };
                let record = self.reads.records[current as usize].as_mut().expect("being rendered");
                record.cells.push((cell, path));
                self.reads.alias_cells.insert(cell.index() as u32, (current, path));
            }
            NOTE_OWN => {
                if !self.own_frame(current, depth, base) {
                    return;
                }
                let mut start = 0;
                if kind & NOTE_ALIASED != 0
                    && let Some(Key::Name(atom)) = keys.first()
                {
                    let key = self.atom_value(Atom(*atom));
                    let record = self.reads.records[current as usize].as_mut().expect("being rendered");
                    start = record.path(0, Step::Key(key.bits()), key);
                    keys = &keys[1..];
                }
                let this = self.called_on(&mut keys, kind, base);
                let compared = self.compared_with(&mut keys, kind, base);
                let mut from = register(self, root);
                if from.is_cell() {
                    from = self.cells[from.index()];
                }
                let leaf = register(self, value);
                let read = Read { from, keys, base, at, prop, leaf, kind, this, compared };
                self.record_read(current, current, start, &read);
            }
            NOTE_UP => {
                let Kind::Function { proto, .. } = self.objects[closure as usize].kind else {
                    return;
                };
                let Some(capture) = self.protos[proto as usize].captures.get(root as usize).copied() else {
                    return;
                };
                if upvalues.is_null() {
                    return;
                }
                // SAFETY: a frame's captured variables are its closure's, which is alive while it runs, and the compiler
                // only names one it has.
                let held = unsafe { *upvalues.add(root as usize) };
                let this = self.called_on(&mut keys, kind, base);
                let compared = self.compared_with(&mut keys, kind, base);
                let leaf = register(self, value);
                let mut seen = false;
                if capture.by_value {
                    if current != NO_READS {
                        let path = self.through(current, closure, proto, root);
                        if let Some(path) = path {
                            let read = Read { from: held, keys, base, at, prop, leaf, kind, this, compared };
                            self.record_read(current, current, path, &read);
                            seen = true;
                        }
                    }
                } else if held.is_cell() {
                    let from = self.cells[held.index()];
                    if from.is_empty() {
                        return;
                    }
                    let read = Read { from, keys, base, at, prop, leaf, kind, this, compared };
                    if let Some(&(owner, path)) = self.reads.alias_cells.get(&(held.index() as u32)) {
                        self.record_read(owner, current, path, &read);
                    }
                    if current != NO_READS {
                        let found = self.reads.records[current as usize].as_ref().and_then(|record| {
                            record
                                .function_cells
                                .iter()
                                .find(|(cell, _)| *cell == held.index() as u32)
                                .map(|(_, path)| *path)
                        });
                        if let Some(path) = found {
                            self.record_read(current, current, path, &read);
                            seen = true;
                        }
                    }
                }
                if !seen {
                    // What it read isn't noted for what's rendering, so it can't be handed on: that wouldn't be looked at again.
                    self.objects.add_flags(closure as usize, NOT_HANDED_ON);
                }
            }
            _ => {}
        }
    }

    /// At the start of a function that notes what it reads of the parameters in the bits of `noted`, and reads those in the bits
    /// of `whole` whole: what it was given that was read to be given to this call is taken, and each of those is noted where it
    /// was read from, rather than being read as itself.
    fn take_params(&mut self, depth: u32, base: usize, noted: u16, whole: u16) {
        self.reads.params.retain(|frame| frame.depth < depth);
        let calling = self.reads.calling;
        if calling == 0 || depth < 2 || !self.reads.given.iter().any(|given| given.calling == calling && given.depth == depth - 1) {
            return;
        }
        let mut params = Vec::new();
        for index in 0..16u16 {
            let bit = 1u16 << index;
            if (noted | whole) & bit == 0 {
                continue;
            }
            // What's called finds what it's given where the caller put it, as a frame starts where its callee's register is.
            let register = 1 + index;
            let at = (base + register as usize) as u32;
            let Some(position) = self.reads.given.iter().position(|given| {
                given.calling == calling && given.depth == depth - 1 && given.at == at && given.value == self.stack[at as usize]
            }) else {
                continue;
            };
            let given = self.reads.given.swap_remove(position);
            if noted & bit != 0 {
                params.push((register, given.id, given.path));
            } else if let Some(record) = self.reads.records.get_mut(given.id as usize).and_then(Option::as_mut) {
                // A function inside sees it, which can read anything of it: it's read whole.
                record.note(given.path, given.value, false, false);
            }
        }
        if !params.is_empty() {
            self.reads.params.push(ParamFrame { depth, base: base as u32, params });
        }
    }

    /// Whether a read of captured variable `index` of `closure`, running, is one that's noted: its cell is a destructured
    /// name's, or the closure, or its cell, was read from what's rendering's props.
    #[allow(unsafe_code)]
    fn up_is_noted(&self, current: u32, closure: u32, upvalues: *const Value, index: u16) -> bool {
        if upvalues.is_null() {
            return false;
        }
        // SAFETY: a frame's captured variables are its closure's, which is alive while it runs, and the compiler only names
        // one it has.
        let held = unsafe { *upvalues.add(index as usize) };
        if held.is_cell() && !self.reads.alias_cells.is_empty() && self.reads.alias_cells.contains_key(&(held.index() as u32)) {
            return true;
        }
        if current == NO_READS {
            return false;
        }
        let Some(record) = self.reads.records[current as usize].as_ref() else {
            return false;
        };
        if held.is_cell() && record.function_cells.iter().any(|(cell, _)| *cell == held.index() as u32) {
            return true;
        }
        record.registered(closure).is_some()
    }

    fn own_frame(&self, current: u32, depth: u32, base: usize) -> bool {
        current != NO_READS
            && self.reads.records[current as usize]
                .as_ref()
                .is_some_and(|record| record.frame == Some((depth, base as u32)))
    }

    /// For a read that's only compared with `===` or `!==`, what with, from the first key, which isn't a step.
    fn compared_with(&self, keys: &mut &[Key], kind: u8, base: usize) -> Option<Value> {
        if kind & NOTE_COMPARED == 0 {
            return None;
        }
        let (Some(Key::Register(reg)), rest) = (keys.first().copied(), &keys[1..]) else {
            return None;
        };
        *keys = rest;
        Some(self.stack[base + reg as usize])
    }

    /// For a read that's about to be called as a method, what it's called on, from the first key, which isn't a step.
    fn called_on(&self, keys: &mut &[Key], kind: u8, base: usize) -> Option<Value> {
        if kind & NOTE_CALLED == 0 {
            return None;
        }
        let (Some(Key::Register(reg)), rest) = (keys.first().copied(), &keys[1..]) else {
            return None;
        };
        *keys = rest;
        Some(self.stack[base + reg as usize])
    }

    /// Where captured variable `index` of the closure running, of code `proto`, is, for the render `current`, if that closure
    /// was read from what it was given, or made by one that was.
    fn through(&mut self, current: u32, closure: u32, proto: u32, index: u16) -> Option<u32> {
        let record = self.reads.records[current as usize].as_mut()?;
        match record.registered(closure)? {
            Registered::Read(base) => {
                let base = *base;
                Some(record.path(base, Step::Capture(proto, index), Value::UNDEFINED))
            }
            Registered::Inner(paths) => paths.get(index as usize).copied().flatten(),
        }
    }

    /// Notes, for the render `id`, a read that started at `start` from `from` and went through `keys`, giving `leaf`, as
    /// the render `current` ran.
    fn record_read(&mut self, id: u32, current: u32, start: u32, read: &Read<'_>) {
        if id == NO_READS {
            return;
        }
        let &Read { from, keys, base, at, prop, leaf, kind, this, compared } = read;
        let itself = kind & NOTE_SAME_ITSELF != 0;
        let mut path = start;
        let mut value = from;
        let mut before: Option<(u32, Value)> = None;
        let mut whole = false;
        for (at, key) in keys.iter().enumerate() {
            let key = match *key {
                Key::Register(reg) => self.stack[base + reg as usize],
                Key::Name(atom) => self.atom_value(Atom(atom)),
            };
            let Some(next) = self.plain_step(value, key) else {
                // A `Set` or a `Map` isn't looked into, but a lookup in one, as `selected.has`, is only what it gives.
                if at + 1 == keys.len() && this == Some(value) && self.only_looks_up(leaf, value) {
                    self.flush_lookup();
                    let itself = kind & NOTE_SAME_ITSELF != 0;
                    self.reads.lookup = Some(Lookup { id, path, on: value, method: leaf, itself, depth: self.frames.len() as u32 });
                    return;
                }
                whole = true;
                break;
            };
            let Some(record) = self.reads.records[id as usize].as_mut() else {
                return;
            };
            before = Some((path, value));
            path = record.path(path, Step::Key(key.bits()), key);
            value = next;
        }
        let Some(record) = self.reads.records[id as usize].as_mut() else {
            return;
        };
        if whole {
            // What can't be looked into without running something is read whole, as far as it goes.
            record.note(path, value, itself, false);
            return;
        }
        if value != leaf && !(value.is_number() && leaf.is_number() && value.as_number() == leaf.as_number()) {
            // What looking found isn't what was read, so what it was read from is read whole.
            record.note(start, from, itself, false);
            return;
        }
        if let Some(other) = compared {
            // All that's known of it is whether it's what it was compared with.
            let was = self.strict_equals(leaf, other);
            let record = self.reads.records[id as usize].as_mut().expect("just found");
            record.compared(path, other, was);
            return;
        }
        if let Some(this) = this
            && !self.is_app_arrow(leaf)
        {
            // A method can see what it's called on, and a native's goes through all of it: that's read whole. One that only
            // looks something up in it is only what it gives, once it's given it.
            let (parent, held) = before.unwrap_or((start, from));
            if held == this && self.only_looks_up(leaf, this) {
                self.flush_lookup();
                self.reads.lookup = Some(Lookup { id, path: parent, on: this, method: leaf, itself, depth: self.frames.len() as u32 });
                return;
            }
            let record = self.reads.records[id as usize].as_mut().expect("just found");
            record.note(parent, if held == this { held } else { this }, itself, true);
            return;
        }
        // A function read from an object in the props, as TanStack Table's `row.getIsSelected`, can say something else when
        // it's called with nothing given anew, as it can read what this can't see: for what didn't say `memo`, what it's
        // read from is read whole too, which is only the same as itself while it holds no functions (see `equivalent`).
        let cautious_parent = match before {
            Some((parent, held)) if parent != 0 && self.is_callable(leaf) => {
                let record = self.reads.records[id as usize].as_ref().expect("just found");
                (record.cautious && !self.under_capture(record, parent)).then_some((parent, held))
            }
            _ => None,
        };
        let record = self.reads.records[id as usize].as_mut().expect("just found");
        if kind & NOTE_ARG != 0 && itself {
            // Given to a call, it's read as itself unless what's called notes what it reads of it.
            let depth = self.frames.len() as u32;
            let at = at as u32;
            self.flush_given(|given| given.depth == depth && given.at == at);
            self.reads.given.push(Given { id, path, value: leaf, depth, at, calling: 0 });
        } else if let Some(prop) = prop {
            record.note_given(path, leaf, prop);
        } else {
            record.note(path, leaf, itself, false);
        }
        let record = self.reads.records[id as usize].as_mut().expect("just found");
        if let Some((parent, held)) = cautious_parent {
            record.note(parent, held, itself, true);
        }
        if id == current && self.is_app_closure(leaf) {
            self.register_function(id, leaf, path);
        }
    }

    fn is_app_closure(&self, value: Value) -> bool {
        if !value.is_object() {
            return false;
        }
        match &self.objects[value.index()].kind {
            Kind::Function { proto, .. } => self.protos[*proto as usize].says & SAYS_APP_FILE != 0,
            _ => false,
        }
    }

    fn is_app_arrow(&self, value: Value) -> bool {
        self.is_app_closure(value)
            && match &self.objects[value.index()].kind {
                Kind::Function { proto, .. } => self.protos[*proto as usize].flags & IS_ARROW != 0,
                _ => false,
            }
    }

    /// Notes that what a function read from the props reads of what it captured, when it's called, is read through it.
    fn register_function(&mut self, id: u32, function: Value, path: u32) {
        let handle = function.index() as u32;
        let Kind::Function { proto, upvalues, .. } = &self.objects[function.index()].kind else {
            return;
        };
        let proto = *proto;
        let held: Vec<Value> = upvalues.to_vec();
        let Some(record) = self.reads.records[id as usize].as_mut() else {
            return;
        };
        if record.registered(handle).is_some() {
            return;
        }
        record.functions.push((handle, Registered::Read(path)));
        record.held.push(function);
        let captures = &self.protos[proto as usize].captures;
        for (index, (value, capture)) in held.iter().zip(captures.iter()).enumerate() {
            if capture.by_value || !value.is_cell() {
                continue;
            }
            if record.function_cells.iter().any(|(cell, _)| *cell == value.index() as u32) {
                continue;
            }
            let at = record.path(path, Step::Capture(proto, index as u16), Value::UNDEFINED);
            record.function_cells.push((value.index() as u32, at));
            record.held.push(*value);
        }
        let readers = self.reads.readers.entry(handle).or_default();
        if !readers.contains(&id) {
            readers.push(id);
        }
    }

    /// While something's rendering, a function made by one read from the props: what it captures from that one is where
    /// that one's is. One made by a function that wasn't noted means that one can't be handed on, as what's made now
    /// would see what it sees.
    #[cold]
    pub(crate) fn note_closure(&mut self, closure: Value) {
        let Some(&current) = self.reads.stack.last() else {
            return;
        };
        let Some(parent) = self.frames.last().map(|frame| frame.closure) else {
            return;
        };
        let Kind::Function { proto, .. } = &self.objects[closure.index()].kind else {
            return;
        };
        let captures = &self.protos[*proto as usize].captures;
        if !captures.iter().any(|capture| !capture.from_register) {
            return;
        }
        let Kind::Function { proto: parent_proto, .. } = self.objects[parent as usize].kind else {
            return;
        };
        let wanted: Vec<(bool, bool, u16)> =
            captures.iter().map(|capture| (capture.from_register, capture.by_value, capture.index)).collect();
        let registered = current != NO_READS
            && self.reads.records[current as usize]
                .as_ref()
                .is_some_and(|record| record.registered(parent).is_some());
        if !registered {
            self.objects.add_flags(parent as usize, NOT_HANDED_ON);
            return;
        }
        let paths: Box<[Option<u32>]> = wanted
            .iter()
            .map(|&(from_register, by_value, index)| {
                if from_register || !by_value {
                    None
                } else {
                    self.through(current, parent, parent_proto, index)
                }
            })
            .collect();
        if paths.iter().any(Option::is_some)
            && let Some(record) = self.reads.records[current as usize].as_mut()
        {
            record.functions.push((closure.index() as u32, Registered::Inner(paths)));
            record.held.push(closure);
        }
    }
}

/// Whether noting what's read is on: `RSUI_READS=0` turns it off, for comparing.
pub(crate) fn reads_enabled() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("RSUI_READS").map_or(true, |value| value != "0"))
}
