//! Objects and their shapes. A shape says which property is where, and objects built alike share one.

use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;

use foldhash::fast::FixedState;

use crate::text::Atom;
use crate::value::Value;
use crate::vm::{Args, Vm};

pub(crate) const NONE: u32 = u32::MAX;

pub(crate) const WRITABLE: u8 = 1;
pub(crate) const ENUMERABLE: u8 = 2;
pub(crate) const CONFIGURABLE: u8 = 4;
/// The slot holds an accessor object, with the getter and setter in it.
pub(crate) const ACCESSOR: u8 = 8;
pub(crate) const PLAIN: u8 = WRITABLE | ENUMERABLE | CONFIGURABLE;
/// What a built-in method or a class's method is: it doesn't turn up when listing properties.
pub(crate) const HIDDEN: u8 = WRITABLE | CONFIGURABLE;

/// An object with more properties than this stops sharing a shape.
const MAX_SHARED_PROPERTIES: u32 = 128;
/// Up to this many properties, walking the shape's chain beats a hash table.
const LINEAR_LOOKUP: u32 = 8;

type Table = HashMap<Atom, (u32, u8), FixedState>;

#[derive(Default)]
pub(crate) struct Dict {
    map: HashMap<Atom, u32, FixedState>,
    /// In order of being added. A deleted one has atom 0, and its slot is left unused until the next tidy-up.
    entries: Vec<(Atom, u8)>,
    live: u32,
}

pub(crate) struct Shape {
    parent: u32,
    key: Atom,
    flags: u8,
    pub(crate) count: u32,
    /// The prototype of every object with this shape, or `NONE` for null.
    pub(crate) proto: u32,
    transitions: Vec<(Atom, u8, u32)>,
    table: Option<Box<Table>>,
    pub(crate) dict: Option<Box<Dict>>,
    /// Whether every property is a plain one, so copying an object can be just copying its slots.
    all_plain: bool,
    /// Whether anything here would turn up in a `for in`.
    any_enumerable: bool,
    /// A bit for each property's name, so that looking for one that isn't here rarely has to go through them all, which is
    /// what asking whether something was said is.
    keys: u64,
    /// Whether this place is free to be used for another shape.
    free: bool,
}

impl Shape {
    fn new(parent: u32, proto: u32) -> Self {
        Self {
            parent,
            key: Atom(0),
            flags: 0,
            count: 0,
            proto,
            transitions: Vec::new(),
            table: None,
            dict: None,
            all_plain: true,
            any_enumerable: false,
            keys: 0,
            free: false,
        }
    }
}

/// The bit a property's name has in [`Shape::keys`].
#[inline]
fn key_bit(key: Atom) -> u64 {
    1 << (key.0.wrapping_mul(0x9E37_79B9) >> 26)
}

/// Every shape there is. A shape is kept by the collector for as long as an object has it, a shape that's kept comes
/// from it (one property more), or the engine holds on to it itself (`Vm::realm`'s); see `Vm::collect` in gc.rs. The
/// rest are freed by a full collection, which takes them out of what they came from, and their places are used again.
/// Anything that remembers a shape by its number (the caches of places in the code, what reads remember, and a few
/// more) is cleared of the freed ones then, so a place used again can't be taken for what was there.
#[derive(Default)]
pub(crate) struct Shapes {
    pub(crate) items: Vec<Shape>,
    free: Vec<u32>,
    roots: HashMap<u32, u32, FixedState>,
    /// Goes up each time a collection frees shapes other than dictionaries, for what remembers shapes for a while
    /// without the collector knowing, such as `JSON.stringify` while it runs.
    pub(crate) generation: u32,
    /// The shapes made, or given another prototype, since the collector last looked, whose prototypes a small
    /// collection has to keep (see `Vm::collect` in gc.rs). Every other shape's prototype is old by then.
    pub(crate) touched: Vec<u32>,
    /// The shapes whose numbers the JIT's code checks for, a bit each: one of them that's freed isn't given to another
    /// shape, which that code would take for it. How many of them have been freed (and so aren't in use, nor free).
    pinned: Vec<u64>,
    retired: usize,
}

impl Shapes {
    /// Puts a shape somewhere, in a free place if there is one.
    fn place(&mut self, shape: Shape) -> u32 {
        if let Some(handle) = self.free.pop() {
            self.items[handle as usize] = shape;
            self.touched.push(handle);
            return handle;
        }
        // A pinned number the end of the list gave back (see `sweep`) isn't given out again: its place comes back
        // empty.
        while self.is_pinned(self.items.len() as u32) {
            self.items.push(Shape {
                free: true,
                ..Shape::new(NONE, NONE)
            });
            self.retired += 1;
        }
        self.items.push(shape);
        let handle = (self.items.len() - 1) as u32;
        self.touched.push(handle);
        handle
    }

    /// The shape of an object with no properties and this prototype.
    pub(crate) fn root(&mut self, proto: u32) -> u32 {
        if let Some(shape) = self.roots.get(&proto) {
            return *shape;
        }
        let shape = self.place(Shape::new(NONE, proto));
        self.roots.insert(proto, shape);
        shape
    }

    /// The shape after adding a property. A shape in dictionary mode doesn't come here.
    pub(crate) fn add(&mut self, shape: u32, key: Atom, flags: u8) -> u32 {
        let from = &mut self.items[shape as usize];
        for (found_key, found_flags, target) in &from.transitions {
            if *found_key == key && *found_flags == flags {
                return *target;
            }
        }
        let count = from.count + 1;
        let proto = from.proto;
        let all_plain = from.all_plain && flags == PLAIN;
        let any_enumerable = from.any_enumerable || flags & ENUMERABLE != 0;
        let keys = from.keys | key_bit(key);
        // What's in the table is all still true, so the child can have it rather than build its own.
        let mut table = from.table.take();
        if let Some(table) = &mut table {
            table.insert(key, (count - 1, flags));
        }
        let target = self.place(Shape {
            key,
            flags,
            count,
            table,
            all_plain,
            any_enumerable,
            keys,
            ..Shape::new(shape, proto)
        });
        self.items[shape as usize]
            .transitions
            .push((key, flags, target));
        target
    }

    /// Where a property is, and its flags.
    pub(crate) fn lookup(&mut self, shape: u32, key: Atom) -> Option<(u32, u8)> {
        let item = &self.items[shape as usize];
        if let Some(dict) = &item.dict {
            let slot = *dict.map.get(&key)?;
            return Some((slot, dict.entries[slot as usize].1));
        }
        if item.keys & key_bit(key) == 0 {
            return None;
        }
        if let Some(table) = &item.table {
            return table.get(&key).copied();
        }
        if item.count > LINEAR_LOOKUP {
            return self.lookup_building_table(shape, key);
        }
        let mut at = item;
        while at.count > 0 {
            if at.key == key {
                return Some((at.count - 1, at.flags));
            }
            at = &self.items[at.parent as usize];
        }
        None
    }

    fn lookup_building_table(&mut self, shape: u32, key: Atom) -> Option<(u32, u8)> {
        let mut table = Table::default();
        let mut at = shape;
        while self.items[at as usize].count > 0 {
            let item = &self.items[at as usize];
            table.insert(item.key, (item.count - 1, item.flags));
            at = item.parent;
        }
        let found = table.get(&key).copied();
        self.items[shape as usize].table = Some(Box::new(table));
        found
    }

    /// Certifies a bounded dictionary without allocating a descriptor list.
    pub(crate) fn bounded_dictionary_has_no_accessors(&self, shape: u32, limit: usize) -> bool {
        let Some(dict) = &self.items[shape as usize].dict else { return false; };
        // Bound historical slots as well as live ones before walking. Deleted
        // descriptor flags cannot execute anything and do not belong to the bag.
        dict.entries.len() <= limit && dict.entries.iter().all(|(key, flags)| key.0 == 0 || flags & ACCESSOR == 0)
    }

    /// Every property in the order they were added: its name, where it is and its flags.
    pub(crate) fn entries(&self, shape: u32) -> Vec<(Atom, u32, u8)> {
        let item = &self.items[shape as usize];
        if let Some(dict) = &item.dict {
            return dict
                .entries
                .iter()
                .enumerate()
                .filter(|(_, (key, _))| key.0 != 0)
                .map(|(slot, (key, flags))| (*key, slot as u32, *flags))
                .collect();
        }
        let mut out = Vec::with_capacity(item.count as usize);
        let mut at = item;
        while at.count > 0 {
            out.push((at.key, at.count - 1, at.flags));
            at = &self.items[at.parent as usize];
        }
        out.reverse();
        out
    }

    /// What [`entries`](Self::entries) gives, put in a list that's given, which is cleared first.
    pub(crate) fn entries_into(&self, shape: u32, out: &mut Vec<(Atom, u32, u8)>) {
        out.clear();
        let item = &self.items[shape as usize];
        if item.dict.is_some() {
            out.extend(self.entries(shape));
            return;
        }
        let mut at = item;
        while at.count > 0 {
            out.push((at.key, at.count - 1, at.flags));
            at = &self.items[at.parent as usize];
        }
        out.reverse();
    }

    /// Goes through what [`entries`](Self::entries) would give without making a list of it. The order is
    /// backwards, and this says whether it was.
    pub(crate) fn visit_entries(&self, shape: u32, mut visit: impl FnMut(Atom, u32)) -> bool {
        let item = &self.items[shape as usize];
        if let Some(dict) = &item.dict {
            for (slot, (key, _)) in dict.entries.iter().enumerate() {
                if key.0 != 0 {
                    visit(*key, slot as u32);
                }
            }
            return false;
        }
        let mut at = item;
        while at.count > 0 {
            visit(at.key, at.count - 1);
            at = &self.items[at.parent as usize];
        }
        true
    }

    #[inline]
    pub(crate) fn is_dict(&self, shape: u32) -> bool {
        self.items[shape as usize].dict.is_some()
    }

    /// Whether an object with this shape can be copied by copying its slots.
    #[inline]
    pub(crate) fn is_all_plain(&self, shape: u32) -> bool {
        self.items[shape as usize].all_plain
    }

    /// The prototype of every object with this shape, or `NONE` for null.
    #[inline]
    pub(crate) fn proto_of(&self, shape: u32) -> u32 {
        self.items[shape as usize].proto
    }

    /// Whether an object with this shape has nothing for `for in` to find.
    #[inline]
    pub(crate) fn has_enumerable(&self, shape: u32) -> bool {
        self.items[shape as usize].any_enumerable
    }

    /// Whether adding one more property here should stop sharing the shape.
    pub(crate) fn wants_dict(&self, shape: u32) -> bool {
        self.items[shape as usize].count >= MAX_SHARED_PROPERTIES
    }

    /// A shape for one object alone, with the same properties in the same slots.
    pub(crate) fn to_dict(&mut self, shape: u32) -> u32 {
        let entries = self.entries(shape);
        let proto = self.items[shape as usize].proto;
        let mut dict = Dict::default();
        dict.entries.reserve(entries.len());
        for (key, slot, flags) in entries {
            dict.map.insert(key, slot);
            dict.entries.push((key, flags));
            dict.live += 1;
        }
        self.place(Shape {
            dict: Some(Box::new(dict)),
            all_plain: false,
            any_enumerable: true,
            keys: u64::MAX,
            ..Shape::new(NONE, proto)
        })
    }

    /// Adds to a dictionary, and says which slot it goes in.
    pub(crate) fn dict_add(&mut self, shape: u32, key: Atom, flags: u8) -> u32 {
        let dict = self.items[shape as usize]
            .dict
            .as_mut()
            .expect("only a dictionary comes here");
        let slot = dict.entries.len() as u32;
        dict.entries.push((key, flags));
        dict.map.insert(key, slot);
        dict.live += 1;
        slot
    }

    pub(crate) fn dict_remove(&mut self, shape: u32, key: Atom) {
        let dict = self.items[shape as usize]
            .dict
            .as_mut()
            .expect("only a dictionary comes here");
        if let Some(slot) = dict.map.remove(&key) {
            dict.entries[slot as usize].0 = Atom(0);
            dict.live -= 1;
        }
    }

    pub(crate) fn dict_set_flags(&mut self, shape: u32, key: Atom, flags: u8) {
        let dict = self.items[shape as usize]
            .dict
            .as_mut()
            .expect("only a dictionary comes here");
        if let Some(slot) = dict.map.get(&key) {
            dict.entries[*slot as usize].1 = flags;
        }
    }

    /// A dictionary with a lot of deleted properties takes them out, and gives what the slots should become.
    pub(crate) fn dict_tidy(&mut self, shape: u32, object: &mut Object) {
        let dict = self.items[shape as usize]
            .dict
            .as_mut()
            .expect("only a dictionary comes here");
        if dict.entries.len() < 16 || dict.live as usize * 2 > dict.entries.len() {
            return;
        }
        let mut next = 0;
        for slot in 0..dict.entries.len() {
            if dict.entries[slot].0.0 == 0 {
                continue;
            }
            dict.entries[next] = dict.entries[slot];
            let slots = object.slots_mut();
            slots[next] = slots[slot];
            dict.map.insert(dict.entries[next].0, next as u32);
            next += 1;
        }
        dict.entries.truncate(next);
        object.truncate_slots(next);
    }

    /// The same properties, with this prototype. A dictionary keeps being one, and others get a chain of their own.
    pub(crate) fn with_proto(&mut self, shape: u32, proto: u32) -> u32 {
        if let Some(dict) = &self.items[shape as usize].dict {
            let _ = dict;
            self.items[shape as usize].proto = proto;
            self.touched.push(shape);
            return shape;
        }
        let entries = self.entries(shape);
        let mut at = self.root(proto);
        for (key, _, flags) in entries {
            at = self.add(at, key, flags);
        }
        at
    }

    /// The same properties, with new flags for each. Slots stay where they are.
    pub(crate) fn with_flags(&mut self, shape: u32, change: impl Fn(Atom, u8) -> u8) -> u32 {
        if self.items[shape as usize].dict.is_some() {
            let dict = self.items[shape as usize]
                .dict
                .as_mut()
                .expect("just checked");
            for (key, flags) in &mut dict.entries {
                if key.0 != 0 {
                    *flags = change(*key, *flags);
                }
            }
            return shape;
        }
        let entries = self.entries(shape);
        let mut at = self.root(self.items[shape as usize].proto);
        for (key, _, flags) in entries {
            at = self.add(at, key, change(key, flags));
        }
        at
    }

    /// Lets a shape that was for one object be used again, after that object has gone.
    pub(crate) fn free_dict(&mut self, shape: u32) {
        let item = &mut self.items[shape as usize];
        item.dict = None;
        item.proto = NONE;
        item.free = true;
        self.free.push(shape);
    }

    #[inline]
    pub(crate) fn is_free(&self, shape: u32) -> bool {
        self.items.get(shape as usize).is_none_or(|item| item.free)
    }

    /// The shape this one is one property more than, or `NONE`.
    #[inline]
    pub(crate) fn parent_of(&self, shape: u32) -> u32 {
        self.items[shape as usize].parent
    }

    /// Every shape a transition goes to, and every root, for the heap checker.
    pub(crate) fn transition_targets(&self) -> impl Iterator<Item = u32> + '_ {
        self.items
            .iter()
            .filter(|item| !item.free)
            .flat_map(|item| item.transitions.iter().map(|(_, _, target)| *target))
            .chain(self.roots.values().copied())
    }

    /// How many shapes are in use.
    pub(crate) fn live(&self) -> usize {
        self.items.len() - self.free.len() - self.retired
    }

    /// Keeps a shape's number from being given to another once it's freed (see `pinned`). False, and nothing done, if
    /// the shape isn't in use, as its number may be given out already.
    #[cfg_attr(not(feature = "jit"), allow(dead_code))]
    pub(crate) fn pin(&mut self, shape: u32) -> bool {
        if self.items.get(shape as usize).is_none_or(|item| item.free) {
            return false;
        }
        let word = shape as usize / 64;
        if self.pinned.len() <= word {
            self.pinned.resize(word + 1, 0);
        }
        self.pinned[word] |= 1 << (shape % 64);
        true
    }

    fn is_pinned(&self, shape: u32) -> bool {
        self.pinned.get(shape as usize / 64).is_some_and(|word| word >> (shape % 64) & 1 != 0)
    }

    /// After a full collection has marked the shapes that are in use, frees the rest: each is taken out of what it came
    /// from (or out of the roots, if it's one), and its place set aside to be used again. Sets a bit in `freed` for each
    /// that was freed, and says whether any were. The free places at the end go.
    pub(crate) fn sweep(&mut self, marks: &[u64], freed: &mut Vec<u64>) -> bool {
        let is_marked = |shape: u32| {
            marks
                .get(shape as usize / 64)
                .is_some_and(|word| word >> (shape % 64) & 1 != 0)
        };
        freed.clear();
        freed.resize(self.items.len() / 64 + 1, 0);
        let mut any = false;
        for shape in 0..self.items.len() as u32 {
            let item = &self.items[shape as usize];
            if item.free || is_marked(shape) {
                continue;
            }
            let (parent, proto, is_dict) = (item.parent, item.proto, item.dict.is_some());
            if parent != NONE {
                // A parent that's going too loses all of its own anyway.
                if is_marked(parent) {
                    self.items[parent as usize]
                        .transitions
                        .retain(|(_, _, target)| *target != shape);
                }
            } else if !is_dict && self.roots.get(&proto) == Some(&shape) {
                self.roots.remove(&proto);
            }
            self.items[shape as usize] = Shape {
                free: true,
                ..Shape::new(NONE, NONE)
            };
            if self.is_pinned(shape) {
                self.retired += 1;
            } else {
                self.free.push(shape);
            }
            freed[shape as usize / 64] |= 1 << (shape % 64);
            any = true;
        }
        if any {
            self.generation = self.generation.wrapping_add(1);
            // The free places at the end go, and the list gives back what it doesn't need: a pinned one's too, which
            // comes back empty if the list grows that far again (see `place`), so that a pin holds one empty place at
            // most and never the list's length.
            while self.items.last().is_some_and(|item| item.free) {
                if self.is_pinned(self.items.len() as u32 - 1) {
                    self.retired -= 1;
                }
                self.items.pop();
            }
            let len = self.items.len() as u32;
            self.free.retain(|shape| *shape < len);
            // Lowest first, so the end of the list stays quiet and can go next time.
            self.free.sort_unstable_by(|a, b| b.cmp(a));
            if self.items.capacity() > 1024 && self.items.len() * 4 < self.items.capacity() {
                self.items.shrink_to(self.items.len() * 2);
            }
            if self.roots.capacity() > 64 && self.roots.len() * 4 < self.roots.capacity() {
                self.roots.shrink_to(self.roots.len() * 2);
            }
        }
        any
    }

    /// Calls `name` with the name of the property this shape has that the one it comes from doesn't, or with every name
    /// if it's a dictionary.
    #[inline]
    pub(crate) fn own_names(&self, shape: u32, mut name: impl FnMut(Atom)) {
        let item = &self.items[shape as usize];
        if let Some(dict) = &item.dict {
            for (key, _) in &dict.entries {
                if key.0 != 0 {
                    name(*key);
                }
            }
        } else if item.count > 0 {
            name(item.key);
        }
    }

    /// Calls `name` with each shape in use and every name it has anywhere: its own, the ones its transitions go by,
    /// and the ones in its table, for the heap checker.
    pub(crate) fn each_name(&self, mut name: impl FnMut(u32, Atom)) {
        for (shape, item) in self.items.iter().enumerate() {
            if item.free {
                continue;
            }
            self.own_names(shape as u32, |key| name(shape as u32, key));
            for (key, _, _) in &item.transitions {
                name(shape as u32, *key);
            }
            if let Some(table) = &item.table {
                for key in table.keys() {
                    name(shape as u32, *key);
                }
            }
            if let Some(dict) = &item.dict {
                for key in dict.map.keys() {
                    name(shape as u32, *key);
                }
            }
        }
    }

    pub(crate) fn memory(&self) -> usize {
        self.items.capacity() * size_of::<Shape>()
    }
}

pub(crate) type NativeFn = fn(&mut Vm, Args) -> Result<Value, Value>;

pub(crate) struct NativeData {
    pub(crate) func: NativeFn,
    /// What this function was made with, for the ones that are made as a program runs.
    pub(crate) data: Value,
    pub(crate) name: Atom,
    pub(crate) length: u8,
    pub(crate) is_constructor: bool,
    /// Whether this is `Function.prototype.call` ([`FORWARDS_CALL`]) or `apply` ([`FORWARDS_APPLY`]), which only call
    /// another function: the interpreter then calls that one itself (`Vm::forward_in_place`), rather than this
    /// starting the interpreter again from Rust.
    pub(crate) forwards: u8,
}

/// [`NativeData::forwards`] for `Function.prototype.call`.
pub(crate) const FORWARDS_CALL: u8 = 1;
/// [`NativeData::forwards`] for `Function.prototype.apply`.
pub(crate) const FORWARDS_APPLY: u8 = 2;

pub(crate) struct BoundData {
    pub(crate) target: Value,
    pub(crate) this: Value,
    pub(crate) args: Box<[Value]>,
}

/// The entries of a `Map` or a `Set`, in order, with what's deleted left as holes until a tidy-up.
#[derive(Default)]
pub(crate) struct MapData {
    /// The latest entry in each of these. Others in the same one follow it through `chain`. There's a power of two of
    /// them, and at least as many as there are entries, so each has hardly more than one.
    pub(crate) buckets: Vec<u32>,
    pub(crate) entries: Vec<(Value, Value)>,
    pub(crate) chain: Vec<u32>,
    pub(crate) live: u32,
}

/// What a `FinalizationRegistry` is watching. The registry keeps its callback and what each target's callback is to be
/// given alive, but not the targets, or the tokens that take a target off the list: the collector sees to those.
pub(crate) struct RegistryData {
    pub(crate) cleanup: Value,
    pub(crate) cells: Vec<RegistryCell>,
    /// What's left of the cells whose target has gone, `(held, token)`, in the order they went, until the callback has
    /// been given each.
    pub(crate) cleared: std::collections::VecDeque<(Value, Value)>,
    /// Whether the registry is waiting in `Vm::cleanups` already.
    pub(crate) queued: bool,
}

pub(crate) struct RegistryCell {
    pub(crate) target: Value,
    pub(crate) held: Value,
    /// What `unregister` takes this off with, or empty if there's nothing that can (or it's gone).
    pub(crate) token: Value,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PromiseState {
    Pending,
    Fulfilled,
    Rejected,
}

pub(crate) struct Reaction {
    pub(crate) on_fulfilled: Value,
    pub(crate) on_rejected: Value,
    /// The promise `then` gave back, which settles with what the handler returns.
    pub(crate) derived: Value,
}

pub(crate) struct PromiseData {
    pub(crate) state: PromiseState,
    pub(crate) value: Value,
    pub(crate) reactions: Vec<Reaction>,
    pub(crate) handled: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GeneratorState {
    Suspended,
    Running,
    Done,
}

pub(crate) struct GeneratorData {
    pub(crate) state: GeneratorState,
    pub(crate) proto: u32,
    pub(crate) closure: u32,
    pub(crate) pc: u32,
    /// Where to put the value it's resumed with.
    pub(crate) resume_register: u16,
    pub(crate) registers: Box<[Value]>,
    pub(crate) extra_args: Option<Vec<Value>>,
    pub(crate) new_target: Value,
    /// The promise an async function's caller got, if this is one.
    pub(crate) promise: Value,
    /// What `return()` was told to return, while it's running `finally` blocks.
    pub(crate) return_value: Value,
    /// For an async generator: the calls waiting their turn, each with the promise its caller got.
    pub(crate) queue: Vec<(crate::generator::Resume, Value, Value)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum IterKind {
    ArrayValues,
    ArrayKeys,
    ArrayEntries,
    StringValues,
    MapEntries,
    MapKeys,
    MapValues,
    SetValues,
    SetEntries,
}

pub(crate) struct IterData {
    pub(crate) kind: IterKind,
    pub(crate) target: Value,
    pub(crate) at: u32,
}

// With the JIT, which reads which kind an object is from machine code, the first byte says (`repr(u8)`).
#[cfg_attr(feature = "jit", repr(u8))]
pub(crate) enum Kind {
    Free,
    Plain,
    Array(Items),
    Arguments(Items),
    Function {
        proto: u32,
        home: u32,
        /// Where a call starts in the code, when it can be called the usual way (its proto is compiled, isn't a class
        /// and doesn't keep its arguments), or `NONE`. A call then needs nothing from the proto before it can go on to
        /// the function's first instruction, which was a load more on every call. See [`crate::vm::Vm::entry_of`].
        entry: u32,
        upvalues: Upvalues,
    },
    Native(Box<NativeData>),
    Bound(Box<BoundData>),
    Error,
    /// A `new Number(1)`, or anything like it.
    Boxed(Value),
    Date(f64),
    Map(Box<MapData>),
    Set(Box<MapData>),
    WeakMap(Box<MapData>),
    WeakSet(Box<MapData>),
    Promise(Box<PromiseData>),
    Generator(Box<GeneratorData>),
    Iter(Box<IterData>),
    Accessor {
        get: Value,
        set: Value,
    },
    RegExp(Box<crate::regex::RegExpData>),
    ArrayBuffer(Box<crate::builtins::BufferData>),
    /// What a `WeakRef` points at, which the collector doesn't keep alive, and sets to undefined once it's gone.
    WeakRef(Value),
    FinalizationRegistry(Box<RegistryData>),
    TypedArray(Box<crate::builtins::TypedData>),
    DataView(Box<crate::builtins::ViewData>),
    Proxy(Box<crate::builtins::ProxyData>),
    /// A BigInt is only stored here. A value that is one has a tag of its own, and is never treated as an object.
    BigInt(Box<crate::builtins::BigData>),
}

/// An array's elements. It's a `Vec` in two words rather than three, as nothing needs more than `u32` of them. A few
/// elements can be kept inside the array's own object, where there's room as arrays rarely have properties.
pub(crate) struct Items {
    start: NonNull<Value>,
    length: u32,
    capacity: u32,
}

/// Set in an `Items` capacity when the elements are inside the object.
const IN_OBJECT: u32 = 1 << 31;

/// Up to how many values a full list of an object's property values grows by half rather than doubling.
const GROWS_BY_HALF: usize = 64;

/// Makes room for one more value in a full list of an object's property values. A small one grows by half (at least 4), as most
/// objects get one or two more properties than they were made with, if any, and doubling would leave as much room as they use,
/// for each of them; a big one doubles, as `Vec` does, so that filling it stays cheap. An array's elements still double: that
/// `push` is inlined into the interpreter's loop, and arrays didn't keep much room here.
#[inline]
fn grow_for_one(vec: &mut Vec<Value>) {
    let length = vec.len();
    if length == vec.capacity() && length < GROWS_BY_HALF {
        vec.reserve_exact((length / 2).max(INLINE_SLOTS));
    }
}

#[allow(unsafe_code)]
impl Items {
    /// No elements, and nowhere yet to put them.
    #[inline]
    pub(crate) const fn empty() -> Self {
        Self {
            start: NonNull::dangling(),
            length: 0,
            capacity: 0,
        }
    }

    #[inline]
    pub(crate) fn push(&mut self, value: Value) {
        if self.length == self.capacity & !IN_OBJECT {
            self.vec().push(value);
            return;
        }
        // SAFETY: there's room, as the length is below the capacity.
        unsafe { self.start.as_ptr().add(self.length as usize).write(value) };
        self.length += 1;
    }

    #[inline]
    pub(crate) fn pop(&mut self) -> Option<Value> {
        if self.length == 0 {
            return None;
        }
        self.length -= 1;
        // SAFETY: that place held a value, which the shorter list no longer counts.
        Some(unsafe { self.start.as_ptr().add(self.length as usize).read() })
    }

    /// How much memory it has apart from the object, in elements.
    pub(crate) fn capacity(&self) -> usize {
        if self.capacity & IN_OBJECT != 0 {
            0
        } else {
            self.capacity as usize
        }
    }

    /// Moves the elements out of the object, if they're in there.
    #[inline]
    fn leave_object(&mut self) {
        if self.capacity & IN_OBJECT != 0 {
            self.move_to_heap();
        }
    }

    #[cold]
    fn move_to_heap(&mut self) {
        let mut vec = Vec::with_capacity(8);
        vec.extend_from_slice(self);
        let mut vec = ManuallyDrop::new(vec);
        self.start = NonNull::from(vec.as_mut_slice()).cast();
        self.capacity = vec.capacity() as u32;
    }

    /// This as a `Vec`, for anything more than adding to the end.
    pub(crate) fn vec(&mut self) -> ItemsVec<'_> {
        self.leave_object();
        // SAFETY: these parts came from a `Vec` in `from`, or from `ItemsVec`, which gives back another.
        let vec = unsafe {
            Vec::from_raw_parts(
                self.start.as_ptr(),
                self.length as usize,
                self.capacity as usize,
            )
        };
        ItemsVec {
            vec: ManuallyDrop::new(vec),
            items: self,
        }
    }

    #[cfg(test)]
    pub(crate) fn into_vec(mut self) -> Vec<Value> {
        self.leave_object();
        let parts = ManuallyDrop::new(self);
        // SAFETY: as in `vec`, and these parts won't be used again.
        unsafe {
            Vec::from_raw_parts(
                parts.start.as_ptr(),
                parts.length as usize,
                parts.capacity as usize,
            )
        }
    }
}

impl From<Vec<Value>> for Items {
    #[inline]
    fn from(vec: Vec<Value>) -> Self {
        assert!(
            vec.capacity() <= u32::MAX as usize,
            "that's too many elements"
        );
        let mut vec = ManuallyDrop::new(vec);
        Self {
            start: NonNull::from(vec.as_mut_slice()).cast(),
            length: vec.len() as u32,
            capacity: vec.capacity() as u32,
        }
    }
}

#[allow(unsafe_code)]
impl Deref for Items {
    type Target = [Value];

    #[inline(always)]
    fn deref(&self) -> &[Value] {
        // SAFETY: the first `length` places hold values.
        unsafe { std::slice::from_raw_parts(self.start.as_ptr(), self.length as usize) }
    }
}

#[allow(unsafe_code)]
impl DerefMut for Items {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut [Value] {
        // SAFETY: the first `length` places hold values, and `&mut self` means nothing else is looking at them.
        unsafe { std::slice::from_raw_parts_mut(self.start.as_ptr(), self.length as usize) }
    }
}

#[allow(unsafe_code)]
impl Drop for Items {
    fn drop(&mut self) {
        if self.capacity != 0 && self.capacity & IN_OBJECT == 0 {
            drop(self.vec_for_dropping());
        }
    }
}

#[allow(unsafe_code)]
impl Items {
    fn vec_for_dropping(&mut self) -> Vec<Value> {
        // SAFETY: as in `vec`, and only `drop` comes here.
        unsafe {
            Vec::from_raw_parts(
                self.start.as_ptr(),
                self.length as usize,
                self.capacity as usize,
            )
        }
    }
}

/// An array's elements for a while as a `Vec`, which go back when this has finished with them.
pub(crate) struct ItemsVec<'a> {
    vec: ManuallyDrop<Vec<Value>>,
    items: &'a mut Items,
}

impl Deref for ItemsVec<'_> {
    type Target = Vec<Value>;

    fn deref(&self) -> &Vec<Value> {
        &self.vec
    }
}

impl DerefMut for ItemsVec<'_> {
    fn deref_mut(&mut self) -> &mut Vec<Value> {
        &mut self.vec
    }
}

impl Drop for ItemsVec<'_> {
    fn drop(&mut self) {
        assert!(
            self.vec.capacity() <= u32::MAX as usize,
            "that's too many elements"
        );
        self.items.start = NonNull::from(self.vec.as_mut_slice()).cast();
        self.items.length = self.vec.len() as u32;
        self.items.capacity = self.vec.capacity() as u32;
    }
}

/// What a closure has captured, in one word. The number of them is kept in front of them.
pub(crate) struct Upvalues(Option<NonNull<Value>>);

#[allow(unsafe_code)]
impl Upvalues {
    pub(crate) const NONE: Self = Self(None);

    pub(crate) fn new(count: usize, values: impl Iterator<Item = Value>) -> Self {
        if count <= POOLED_UPVALUES {
            let first = upvalue_pool::take(count);
            // SAFETY: the block has room for the count and `count` values, and nothing else has it.
            unsafe {
                first.as_ptr().write(Value::int(count as i32));
                let mut written = 0;
                for value in values.take(count) {
                    first.as_ptr().add(1 + written).write(value);
                    written += 1;
                }
                debug_assert_eq!(written, count);
                for at in written..count {
                    first.as_ptr().add(1 + at).write(Value::UNDEFINED);
                }
            }
            return Self(Some(first));
        }
        let mut all = Vec::with_capacity(count + 1);
        all.push(Value::int(count as i32));
        all.extend(values);
        debug_assert_eq!(all.len(), count + 1);
        let all = Box::into_raw(all.into_boxed_slice());
        Self(NonNull::new(all.cast()))
    }

    /// Where the first of them is. There's nothing to read there unless there are some.
    #[inline(always)]
    pub(crate) fn as_ptr(&self) -> *const Value {
        match self.0 {
            // SAFETY: there's always the count in front of them.
            Some(first) => unsafe { first.as_ptr().add(1) },
            None => std::ptr::null(),
        }
    }
}

#[allow(unsafe_code)]
impl Deref for Upvalues {
    type Target = [Value];

    #[inline]
    fn deref(&self) -> &[Value] {
        match self.0 {
            // SAFETY: the first place holds how many more there are.
            Some(first) => unsafe {
                let count = first.as_ptr().read().as_int() as usize;
                std::slice::from_raw_parts(first.as_ptr().add(1), count)
            },
            None => &[],
        }
    }
}

#[allow(unsafe_code)]
impl Drop for Upvalues {
    fn drop(&mut self) {
        if let Some(first) = self.0 {
            // SAFETY: this is the box `new` made, which the count in it gives the size of.
            unsafe {
                let count = first.as_ptr().read().as_int() as usize;
                if count <= POOLED_UPVALUES {
                    upvalue_pool::give(count, first);
                    return;
                }
                drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                    first.as_ptr(),
                    count + 1,
                )));
            }
        }
    }
}

/// Closures that capture this many variables or fewer have their box from [`upvalue_pool`].
const POOLED_UPVALUES: usize = 8;

/// The boxes of closures' captured variables, kept for the next closure of the same size on this thread rather than
/// freed. A chart.js line update makes and frees 30,000 closures, most of them handed to a function that calls them
/// once, and each box went through the allocator, which takes a lock, both ways. Blocks are taken from and given back
/// to the allocator as `Box<[Value]>` would be, so one from here can be freed there, and the other way round. At most
/// `KEPT` of each size are kept, and they go back to the allocator when the thread ends.
#[allow(unsafe_code)]
mod upvalue_pool {
    use super::POOLED_UPVALUES;
    use crate::value::Value;
    use std::alloc::Layout;
    use std::cell::UnsafeCell;
    use std::ptr::NonNull;

    /// How many free boxes of each size a thread keeps: 2,048 of the biggest is 144 KB.
    const KEPT: usize = 2048;

    struct Pool([Vec<NonNull<Value>>; POOLED_UPVALUES + 1]);

    impl Drop for Pool {
        fn drop(&mut self) {
            for (count, list) in self.0.iter_mut().enumerate() {
                for first in list.drain(..) {
                    // SAFETY: each block was allocated with this layout and isn't in use.
                    unsafe { std::alloc::dealloc(first.as_ptr().cast(), layout(count)) };
                }
            }
        }
    }

    thread_local! {
        static POOL: UnsafeCell<Pool> = const { UnsafeCell::new(Pool([const { Vec::new() }; POOLED_UPVALUES + 1])) };
    }

    fn layout(count: usize) -> Layout {
        Layout::array::<Value>(count + 1).expect("a few values fit")
    }

    /// A block with room for a count and `count` values, with nothing in it yet.
    #[inline]
    pub(super) fn take(count: usize) -> NonNull<Value> {
        let kept = POOL
            .try_with(|pool| {
                // SAFETY: only this thread reaches its pool, and nothing here calls out while it's borrowed.
                unsafe { (*pool.get()).0[count].pop() }
            })
            .ok()
            .flatten();
        kept.unwrap_or_else(|| {
            // SAFETY: the layout isn't zero-sized.
            let block = unsafe { std::alloc::alloc(layout(count)) };
            match NonNull::new(block.cast()) {
                Some(block) => block,
                None => std::alloc::handle_alloc_error(layout(count)),
            }
        })
    }

    /// Gives back a block [`take`] or a `Box<[Value]>` of `count + 1` made, which nothing uses any more.
    ///
    /// # Safety
    /// `first` must be such a block, of that size.
    #[inline]
    pub(super) unsafe fn give(count: usize, first: NonNull<Value>) {
        let kept = POOL
            .try_with(|pool| {
                // SAFETY: only this thread reaches its pool, and nothing here calls out while it's borrowed.
                let list = unsafe { &mut (*pool.get()).0[count] };
                if list.len() < KEPT {
                    list.push(first);
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
        if !kept {
            // SAFETY: as the caller says.
            unsafe { std::alloc::dealloc(first.as_ptr().cast(), layout(count)) };
        }
    }
}

/// Whether values with room for `capacity` and `length` of them in use would take a smaller block at their length, in
/// rsui-alloc's size classes (16 bytes apart up to 128, then wider), or past its biggest class, by more than a little.
fn worth_shrinking(length: usize, capacity: usize) -> bool {
    let class = |bytes: usize| match bytes {
        0 => 0,
        1..=128 => bytes.next_multiple_of(16),
        129..=256 => bytes.next_multiple_of(32),
        257..=512 => bytes.next_multiple_of(64),
        513..=1024 => bytes.next_multiple_of(128),
        _ => bytes.next_multiple_of(16),
    };
    class(capacity * 8) > class(length * 8)
}

const INLINE_SLOTS: usize = 4;
/// What `slot_count` says when the values are on the heap, which then knows how many there are.
const ON_HEAP: u8 = u8::MAX;

/// An object's property values. Most objects only have a few, and those go inside the object.
union SlotStore {
    inline: [Value; INLINE_SLOTS],
    heap: ManuallyDrop<Vec<Value>>,
}

/// Exactly one cache line, as the list of objects starts on a page and never moves.
pub(crate) struct Object {
    pub(crate) shape: u32,
    pub(crate) flags: u8,
    slot_count: u8,
    store: SlotStore,
    pub(crate) kind: Kind,
}

// `slot_count` says which part of `store` is in use: `inline` with that many values, or `heap` if it's `ON_HEAP`.
#[allow(unsafe_code)]
impl Object {
    #[inline(always)]
    pub(crate) fn new(shape: u32, kind: Kind, capacity: usize) -> Self {
        if capacity <= INLINE_SLOTS {
            Self {
                shape,
                flags: 0,
                slot_count: 0,
                store: SlotStore {
                    inline: [Value::UNDEFINED; INLINE_SLOTS],
                },
                kind,
            }
        } else {
            Self {
                shape,
                flags: 0,
                slot_count: ON_HEAP,
                store: SlotStore {
                    heap: ManuallyDrop::new(Vec::with_capacity(capacity)),
                },
                kind,
            }
        }
    }

    /// Has an empty array keep its first few elements inside itself, which needs no memory of its own for them.
    ///
    /// The object has to be in its place in the list of objects by now, as they're found through a pointer from then on.
    /// It stays valid, as objects never move, and whatever would use the same room moves them out first.
    /// They start as these, four at most.
    #[inline]
    pub(crate) fn keep_items_inside_with(&mut self, values: &[Value]) {
        debug_assert_eq!(self.slot_count, 0);
        debug_assert!(values.len() <= INLINE_SLOTS);
        if let Kind::Array(items) = &mut self.kind {
            debug_assert_eq!(items.capacity, 0);
            // SAFETY: with no property values in it, nothing else is using this part of the store.
            let inline = unsafe { &mut self.store.inline };
            let count = values.len().min(INLINE_SLOTS);
            inline[..count].copy_from_slice(&values[..count]);
            items.start = NonNull::from(inline).cast();
            items.length = count as u32;
            items.capacity = INLINE_SLOTS as u32 | IN_OBJECT;
        }
    }

    /// Makes room for a property value in an array that was keeping its elements inside itself.
    #[inline]
    fn clear_store(&mut self) {
        if let Kind::Array(items) | Kind::Arguments(items) = &mut self.kind {
            items.leave_object();
        }
    }

    /// A place in the list with nothing in it.
    pub(crate) fn free() -> Self {
        Self::new(0, Kind::Free, 0)
    }

    /// Makes this what [`Object::free`] is, as the collector does to each object that has died. Most of those are plain
    /// objects and small arrays that own no memory, and they only have a few fields changed, which is all that's read
    /// of a free place: its slots are left as they were, with none counted, and nothing reads them before they're
    /// written again.
    #[inline(always)]
    pub(crate) fn clear(&mut self) {
        let owns_memory = self.slot_count == ON_HEAP
            || match &self.kind {
                Kind::Free | Kind::Plain | Kind::Error | Kind::Date(_) | Kind::Boxed(_) => false,
                Kind::Array(items) | Kind::Arguments(items) => items.capacity() != 0,
                _ => true,
            };
        if owns_memory {
            self.clear_slowly();
            return;
        }
        // SAFETY: what's in `kind` owns nothing, so there's nothing to drop, and no slots are counted afterwards.
        unsafe { std::ptr::write(&mut self.kind, Kind::Free) };
        self.slot_count = 0;
        self.flags = 0;
        self.shape = 0;
    }

    #[cold]
    #[inline(never)]
    fn clear_slowly(&mut self) {
        *self = Self::free();
    }

    #[inline]
    pub(crate) fn slots(&self) -> &[Value] {
        // SAFETY: `slot_count` says which of these it is.
        unsafe {
            if self.slot_count == ON_HEAP {
                &self.store.heap
            } else {
                self.store.inline.get_unchecked(..self.slot_count as usize)
            }
        }
    }

    #[inline]
    pub(crate) fn slots_mut(&mut self) -> &mut [Value] {
        // SAFETY: `slot_count` says which of these it is.
        unsafe {
            if self.slot_count == ON_HEAP {
                &mut self.store.heap
            } else {
                self.store
                    .inline
                    .get_unchecked_mut(..self.slot_count as usize)
            }
        }
    }

    /// The value in a slot, with no looking at whether it's there.
    ///
    /// # Safety
    /// There has to be a value in that slot.
    #[inline(always)]
    pub(crate) unsafe fn read_slot(&self, slot: usize) -> Value {
        // SAFETY: `slot_count` says which of these it is, and the caller knows the slot is there.
        unsafe {
            if self.slot_count == ON_HEAP {
                *self.store.heap.get_unchecked(slot)
            } else {
                *self.store.inline.get_unchecked(slot)
            }
        }
    }

    /// Changes the value in a slot, with no looking at whether it's there.
    ///
    /// # Safety
    /// There has to be a value in that slot.
    #[inline(always)]
    pub(crate) unsafe fn write_slot(&mut self, slot: usize, value: Value) {
        // SAFETY: `slot_count` says which of these it is, and the caller knows the slot is there.
        unsafe {
            if self.slot_count == ON_HEAP {
                *(*self.store.heap).get_unchecked_mut(slot) = value;
            } else {
                *self.store.inline.get_unchecked_mut(slot) = value;
            }
        }
    }

    /// Adds a value after the last. What's usual, a plain object with room inside, is done here, as the interpreter
    /// adds most properties (`this.x = x` in a constructor) and a call for each of those was 4% of raytrace; an array
    /// keeps its first elements in that room until it has a property, so only a plain object's first can go there.
    #[inline(always)]
    pub(crate) fn push_slot(&mut self, value: Value) {
        let count = self.slot_count as usize;
        if count < INLINE_SLOTS && (count != 0 || matches!(self.kind, Kind::Plain)) {
            // SAFETY: with fewer than `INLINE_SLOTS` the values are inside, and the room after them is free.
            unsafe { *self.store.inline.get_unchecked_mut(count) = value };
            self.slot_count += 1;
            return;
        }
        self.push_slot_slowly(value);
    }

    #[inline(never)]
    fn push_slot_slowly(&mut self, value: Value) {
        if self.slot_count == 0 {
            self.clear_store();
        }
        // SAFETY: `slot_count` says which of these it is, and moving to the heap changes it to say so.
        unsafe {
            if self.slot_count == ON_HEAP {
                let heap = &mut *self.store.heap;
                grow_for_one(heap);
                heap.push(value);
            } else if (self.slot_count as usize) < INLINE_SLOTS {
                *self
                    .store
                    .inline
                    .get_unchecked_mut(self.slot_count as usize) = value;
                self.slot_count += 1;
            } else {
                let mut heap = Vec::with_capacity(INLINE_SLOTS * 2);
                heap.extend_from_slice(&self.store.inline);
                heap.push(value);
                self.store.heap = ManuallyDrop::new(heap);
                self.slot_count = ON_HEAP;
            }
        }
    }

    pub(crate) fn truncate_slots(&mut self, length: usize) {
        // SAFETY: `slot_count` says which of these it is.
        unsafe {
            if self.slot_count == ON_HEAP {
                (*self.store.heap).truncate(length);
            } else {
                self.slot_count = self.slot_count.min(length as u8);
            }
        }
    }

    /// Gives back the room past the last value in this object's property values and an array's elements, where they're
    /// outside the object and the room is more than the allocator would round their length up to. Says how many bytes of
    /// room that was. For `RSUI_EXACT`, after a full collection.
    pub(crate) fn shrink_to_fit(&mut self) -> usize {
        let mut saved = 0;
        if self.slot_count == ON_HEAP {
            // SAFETY: `slot_count` says the values are on the heap.
            let heap = unsafe { &mut *self.store.heap };
            if heap.capacity() > heap.len() && worth_shrinking(heap.len(), heap.capacity()) {
                saved += (heap.capacity() - heap.len()) * 8;
                heap.shrink_to_fit();
            }
        }
        if let Kind::Array(items) | Kind::Arguments(items) = &mut self.kind {
            let capacity = items.capacity();
            if capacity > items.len() && worth_shrinking(items.len(), capacity) {
                saved += (capacity - items.len()) * 8;
                items.vec().shrink_to_fit();
            }
        }
        saved
    }

    pub(crate) fn slots_heap_bytes(&self) -> usize {
        // SAFETY: `slot_count` says which of these it is.
        unsafe {
            if self.slot_count == ON_HEAP {
                self.store.heap.capacity() * 8
            } else {
                0
            }
        }
    }

    /// Replaces all the values with these, which it has room for or moves to the heap for.
    pub(crate) fn set_slots(&mut self, values: &[Value]) {
        self.clear_store();
        // SAFETY: `slot_count` says which of these it is, and everything's left the way it says.
        unsafe {
            if self.slot_count == ON_HEAP {
                ManuallyDrop::drop(&mut self.store.heap);
            }
            if values.len() <= INLINE_SLOTS {
                self.store.inline = [Value::UNDEFINED; INLINE_SLOTS];
                (&mut self.store.inline)[..values.len()].copy_from_slice(values);
                self.slot_count = values.len() as u8;
            } else {
                self.store.heap = ManuallyDrop::new(values.to_vec());
                self.slot_count = ON_HEAP;
            }
        }
    }

    /// Takes already-owned compact storage without allocating and copying it a second
    /// time. A boxed slice ensures capacity equals length, as the caller's debt
    /// accounting assumes. Small collections still use the object's inline slots.
    pub(crate) fn set_slots_owned(&mut self, values: Box<[Value]>) {
        if values.len() <= INLINE_SLOTS {
            self.set_slots(&values);
            return;
        }
        self.clear_store();
        // SAFETY: slot_count identifies the active union member. The old heap is
        // dropped exactly once and the incoming allocation becomes the new heap.
        unsafe {
            if self.slot_count == ON_HEAP {
                ManuallyDrop::drop(&mut self.store.heap);
            }
            self.store.heap = ManuallyDrop::new(values.into_vec());
            self.slot_count = ON_HEAP;
        }
    }
}

#[allow(unsafe_code)]
impl Drop for Object {
    #[inline]
    fn drop(&mut self) {
        if self.slot_count == ON_HEAP {
            // SAFETY: `slot_count` says the heap is in use, and this is the last anyone sees of it.
            unsafe { ManuallyDrop::drop(&mut self.store.heap) };
        }
    }
}

/// The object can't be added to.
pub(crate) const NOT_EXTENSIBLE: u8 = 1;
/// Other objects have this as their prototype, so a change to its properties can change what they find.
pub(crate) const IS_PROTO: u8 = 2;
/// A function that hasn't made its `name`, `length` and `prototype` yet, as most are never asked for.
pub(crate) const LAZY_FUNCTION: u8 = 4;
/// An array's elements can't be changed or removed.
pub(crate) const FROZEN_ELEMENTS: u8 = 8;
/// A property of the global object stays findable through the shape, however many there are.
pub(crate) const NEVER_DICT: u8 = 16;
/// An array has elements that aren't plain, which are in its shape, with holes in the array that lead there.
pub(crate) const SPECIAL_ELEMENTS: u8 = 32;
/// An array's `length` can't be changed.
pub(crate) const READ_ONLY_LENGTH: u8 = 64;

/// Where the JIT finds things in an object, which Rust is free to lay out as it likes, so it's worked out here, from
/// real objects, rather than written down.
#[cfg(feature = "jit")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ObjectLayout {
    pub(crate) size: usize,
    pub(crate) shape: usize,
    pub(crate) flags: usize,
    pub(crate) slot_count: usize,
    /// What `slot_count` says when the values are on the heap.
    pub(crate) on_heap: u8,
    /// Where the values are when they're inside the object.
    pub(crate) inline_slots: usize,
    /// Where the pointer to them is when they're on the heap.
    pub(crate) heap_slots: usize,
    /// The byte that says which `Kind` it is.
    pub(crate) kind: usize,
    pub(crate) function_tag: u8,
    pub(crate) array_tag: u8,
    pub(crate) native_tag: u8,
    pub(crate) function_proto: usize,
    /// The word that's the closure's captured variables, or 0. They start one value further on, after their count.
    pub(crate) function_upvalues: usize,
    pub(crate) array_start: usize,
    pub(crate) array_length: usize,
    /// `arguments`, whose elements are where an array's are.
    pub(crate) arguments_tag: u8,
    pub(crate) typed_tag: u8,
    /// The word in a typed array's `Kind` that points at its `TypedData`.
    pub(crate) typed_data: usize,
    /// What `TypedData::element` reads as, as a byte, for each `Element`, in the order they're declared.
    pub(crate) elements: [u8; 12],
    /// The word in a native's `Kind` that points at its `NativeData`.
    pub(crate) native_data: usize,
    /// How many values fit inside an object, and the kind whose first one can go there (see `Object::push_slot`).
    pub(crate) inline_count: u8,
    pub(crate) plain_tag: u8,
}

#[cfg(feature = "jit")]
#[allow(unsafe_code)]
pub(crate) fn object_layout() -> ObjectLayout {
    use std::mem::offset_of;
    fn at<T, U>(whole: &T, part: &U) -> usize {
        std::ptr::from_ref(part) as usize - std::ptr::from_ref(whole) as usize
    }
    fn word(whole: *const u8, at: usize) -> usize {
        // SAFETY: the callers only ask for words inside the value `whole` points at.
        unsafe { whole.add(at).cast::<usize>().read_unaligned() }
    }
    // SAFETY: `Kind` is `repr(u8)` with this feature, which puts the variant's number in its first byte.
    let tag = |kind: &Kind| unsafe { *std::ptr::from_ref(kind).cast::<u8>() };
    let object = Object::new(0, Kind::Plain, INLINE_SLOTS + 4);
    assert_eq!(object.slot_count, ON_HEAP);
    let start = std::ptr::from_ref(&object).cast::<u8>();
    let store = offset_of!(Object, store);
    let data = object.slots().as_ptr() as usize;
    let heap_slots = (0..3)
        .map(|n| store + 8 * n)
        .find(|&place| word(start, place) == data)
        .expect("a Vec has its pointer in one of its three words");
    let function = Kind::Function {
        proto: 0x1234_5678,
        home: NONE,
        entry: NONE,
        upvalues: Upvalues::new(1, std::iter::once(Value::int(7))),
    };
    let Kind::Function {
        proto, upvalues, ..
    } = &function
    else {
        unreachable!()
    };
    let function_proto = at(&function, proto);
    let function_upvalues = at(&function, upvalues);
    assert_eq!(
        word(std::ptr::from_ref(&function).cast(), function_upvalues) + 8,
        upvalues.as_ptr() as usize,
        "captured variables are one pointer, to their count"
    );
    fn nothing(_: &mut Vm, _: Args) -> Result<Value, Value> {
        Ok(Value::UNDEFINED)
    }
    let native = Kind::Native(Box::new(NativeData {
        func: nothing,
        data: Value::UNDEFINED,
        name: Atom(0),
        length: 0,
        is_constructor: false,
        forwards: 0,
    }));
    let array = Kind::Array(Items::from(vec![Value::int(1), Value::int(2)]));
    let Kind::Array(items) = &array else {
        unreachable!()
    };
    let items_at = at(&array, items);
    let arguments = Kind::Arguments(Items::from(vec![Value::int(1)]));
    let Kind::Arguments(argument_items) = &arguments else {
        unreachable!()
    };
    assert_eq!(at(&arguments, argument_items), items_at, "arguments keep their elements where arrays do");
    use crate::builtins::{Element, TypedData};
    let typed_of = |element: Element| TypedData {
        element,
        buffer: Value::UNDEFINED,
        offset: 0,
        length: 0,
        follows_buffer: false,
        quick: std::ptr::null_mut(),
        quick_length: 0,
        epoch: 0,
    };
    let elements = [
        Element::Int8,
        Element::Uint8,
        Element::Uint8Clamped,
        Element::Int16,
        Element::Uint16,
        Element::Int32,
        Element::Uint32,
        Element::Float16,
        Element::Float32,
        Element::Float64,
        Element::BigInt64,
        Element::BigUint64,
    ]
    .map(|element| {
        let data = typed_of(element);
        // SAFETY: the byte is inside the value.
        unsafe { *std::ptr::from_ref(&data).cast::<u8>().add(offset_of!(TypedData, element)) }
    });
    let typed = Kind::TypedArray(Box::new(typed_of(Element::Int8)));
    let Kind::TypedArray(boxed) = &typed else {
        unreachable!()
    };
    let boxed_at = std::ptr::from_ref::<TypedData>(boxed) as usize;
    let typed_data = (0..size_of::<Kind>() / 8)
        .map(|n| 8 * n)
        .find(|&place| word(std::ptr::from_ref(&typed).cast(), place) == boxed_at)
        .expect("a typed array's data is one pointer");
    let Kind::Native(native_box) = &native else {
        unreachable!()
    };
    let native_at = std::ptr::from_ref::<NativeData>(native_box) as usize;
    let native_data = (0..size_of::<Kind>() / 8)
        .map(|n| 8 * n)
        .find(|&place| word(std::ptr::from_ref(&native).cast(), place) == native_at)
        .expect("a native's data is one pointer");
    ObjectLayout {
        size: size_of::<Object>(),
        shape: offset_of!(Object, shape),
        flags: offset_of!(Object, flags),
        slot_count: offset_of!(Object, slot_count),
        on_heap: ON_HEAP,
        inline_slots: store,
        heap_slots,
        kind: offset_of!(Object, kind),
        function_tag: tag(&function),
        array_tag: tag(&array),
        native_tag: tag(&native),
        function_proto,
        function_upvalues,
        array_start: items_at + offset_of!(Items, start),
        array_length: items_at + offset_of!(Items, length),
        arguments_tag: tag(&arguments),
        typed_tag: tag(&typed),
        typed_data,
        elements,
        native_data,
        inline_count: INLINE_SLOTS as u8,
        plain_tag: tag(&Kind::Plain),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_slots_adopt_compact_storage_and_switch_back_to_inline_safely() {
        let mut object = Object::new(0, Kind::Plain, 0);
        for length in [5, 12, 3, 7, 0, 4, 9] {
            let values: Box<[Value]> = (0..length).map(Value::int).collect();
            let pointer = values.as_ptr();
            object.set_slots_owned(values);
            assert_eq!(object.slots(), (0..length).map(Value::int).collect::<Vec<_>>());
            if length as usize > INLINE_SLOTS {
                assert_eq!(object.slots().as_ptr(), pointer, "owned storage was copied again");
                assert_eq!(object.slots_heap_bytes(), length as usize * size_of::<Value>());
            } else {
                assert_eq!(object.slots_heap_bytes(), 0);
            }
        }
    }

    fn atom(number: u32) -> Atom {
        Atom(number + 1)
    }

    #[test]
    fn objects_built_alike_share_a_shape() {
        let mut shapes = Shapes::default();
        let root = shapes.root(NONE);
        let a = shapes.add(root, atom(1), PLAIN);
        let a = shapes.add(a, atom(2), PLAIN);
        let b = shapes.add(root, atom(1), PLAIN);
        let b = shapes.add(b, atom(2), PLAIN);
        assert_eq!(a, b);
        let c = shapes.add(root, atom(2), PLAIN);
        assert_ne!(a, c);
    }

    #[test]
    fn lookups_find_every_property_on_any_length_of_chain() {
        let mut shapes = Shapes::default();
        let mut shape = shapes.root(NONE);
        for number in 0..40 {
            shape = shapes.add(shape, atom(number), PLAIN);
            for earlier in 0..=number {
                assert_eq!(shapes.lookup(shape, atom(earlier)), Some((earlier, PLAIN)));
            }
            assert_eq!(shapes.lookup(shape, atom(number + 1)), None);
        }
    }

    #[test]
    fn a_shape_that_gave_its_table_away_can_still_find_things() {
        let mut shapes = Shapes::default();
        let mut shape = shapes.root(NONE);
        for number in 0..20 {
            shape = shapes.add(shape, atom(number), PLAIN);
        }
        assert!(shapes.lookup(shape, atom(3)).is_some());
        let child = shapes.add(shape, atom(100), PLAIN);
        assert_eq!(shapes.lookup(child, atom(100)), Some((20, PLAIN)));
        assert_eq!(shapes.lookup(shape, atom(100)), None);
        assert_eq!(shapes.lookup(shape, atom(3)), Some((3, PLAIN)));
    }

    #[test]
    fn bounded_dictionary_certification_checks_live_flags_and_historical_length() {
        let mut shapes = Shapes::default();
        let root = shapes.root(NONE);
        let shape = shapes.to_dict(root);
        shapes.dict_add(shape, atom(1), PLAIN);
        shapes.dict_add(shape, atom(2), ACCESSOR);
        assert!(!shapes.bounded_dictionary_has_no_accessors(shape, 2));
        shapes.dict_remove(shape, atom(2));
        assert!(shapes.bounded_dictionary_has_no_accessors(shape, 2));
        assert!(!shapes.bounded_dictionary_has_no_accessors(shape, 1));
        shapes.dict_add(shape, atom(3), ACCESSOR | ENUMERABLE);
        assert!(!shapes.bounded_dictionary_has_no_accessors(shape, 3));
    }

    #[test]
    fn entries_come_in_the_order_they_were_added() {
        let mut shapes = Shapes::default();
        let mut shape = shapes.root(NONE);
        for number in [5, 2, 9] {
            shape = shapes.add(shape, atom(number), PLAIN);
        }
        let keys: Vec<Atom> = shapes.entries(shape).iter().map(|entry| entry.0).collect();
        assert_eq!(keys, [atom(5), atom(2), atom(9)]);
    }

    #[test]
    fn deleting_from_a_dictionary_keeps_the_other_slots_right() {
        let mut shapes = Shapes::default();
        let mut shape = shapes.root(NONE);
        let mut object = Object::new(0, Kind::Plain, 0);
        for number in 0..20 {
            shape = shapes.add(shape, atom(number), PLAIN);
            object.push_slot(Value::int(number as i32));
        }
        let dict = shapes.to_dict(shape);
        for number in 0..15 {
            shapes.dict_remove(dict, atom(number));
        }
        assert_eq!(shapes.lookup(dict, atom(3)), None);
        assert_eq!(shapes.lookup(dict, atom(17)), Some((17, PLAIN)));
        shapes.dict_tidy(dict, &mut object);
        assert_eq!(object.slots().len(), 5);
        let (slot, _) = shapes.lookup(dict, atom(17)).unwrap();
        assert_eq!(object.slots()[slot as usize], Value::int(17));
        let slot = shapes.dict_add(dict, atom(30), PLAIN);
        assert_eq!(slot, 5);
    }

    #[test]
    fn slots_move_out_of_the_object_as_they_grow() {
        let mut object = Object::new(0, Kind::Plain, 0);
        for number in 0..4 {
            object.push_slot(Value::int(number));
        }
        assert_eq!(object.slots_heap_bytes(), 0);
        object.push_slot(Value::int(4));
        assert!(object.slots_heap_bytes() > 0);
        assert_eq!(object.slots().len(), 5);
        for number in 0..5 {
            assert_eq!(object.slots()[number], Value::int(number as i32));
        }
        object.slots_mut()[2] = Value::int(9);
        assert_eq!(object.slots()[2], Value::int(9));
        object.truncate_slots(1);
        assert_eq!(object.slots().len(), 1);
        object.set_slots(&[Value::TRUE; 9]);
        assert_eq!(object.slots().len(), 9);
        object.set_slots(&[Value::TRUE]);
        assert_eq!(object.slots().len(), 1);
        assert_eq!(object.slots_heap_bytes(), 0);
    }

    #[test]
    fn an_object_is_exactly_one_cache_line() {
        assert_eq!(size_of::<Kind>(), 24);
        assert_eq!(size_of::<Object>(), 64);
    }

    #[test]
    fn array_items_act_like_a_vec() {
        let mut items = Items::from(Vec::new());
        for number in 0..100 {
            items.push(Value::int(number));
        }
        assert_eq!(items.len(), 100);
        assert_eq!(items[50], Value::int(50));
        assert_eq!(items.pop(), Some(Value::int(99)));
        items.vec().remove(0);
        assert_eq!(items[0], Value::int(1));
        items.vec().truncate(2);
        assert_eq!(items.len(), 2);
        assert_eq!(items.into_vec(), vec![Value::int(1), Value::int(2)]);
    }

    #[test]
    fn a_small_array_keeps_its_elements_inside_until_it_needs_the_room() {
        // A box stands in for the list of objects, as it never moves either.
        let mut array = Box::new(Object::new(0, Kind::Array(Items::empty()), 0));
        array.keep_items_inside_with(&[]);
        let items = |object: &Object| -> Vec<Value> {
            match &object.kind {
                Kind::Array(items) => items.to_vec(),
                _ => Vec::new(),
            }
        };
        for number in 0..4 {
            if let Kind::Array(items) = &mut array.kind {
                items.push(Value::int(number));
                assert_eq!(items.capacity(), 0);
            }
        }
        assert_eq!(items(&array).len(), 4);
        // A property value needs the room, so the elements have to go.
        array.push_slot(Value::TRUE);
        assert_eq!(array.slots(), [Value::TRUE]);
        assert_eq!(items(&array), [0, 1, 2, 3].map(Value::int));
        if let Kind::Array(items) = &mut array.kind {
            assert!(items.capacity() > 0);
            items.push(Value::int(4));
        }
        assert_eq!(items(&array).len(), 5);

        // Growing past what fits works the same way.
        let mut array = Box::new(Object::new(0, Kind::Array(Items::empty()), 0));
        array.keep_items_inside_with(&[]);
        if let Kind::Array(items) = &mut array.kind {
            for number in 0..100 {
                items.push(Value::int(number));
            }
            assert_eq!(items[99], Value::int(99));
            assert_eq!(items.pop(), Some(Value::int(99)));
        }
    }

    #[test]
    fn captured_variables_keep_their_count_in_front_of_them() {
        let none = Upvalues::NONE;
        assert_eq!(none.len(), 0);
        let some = Upvalues::new(3, [1, 2, 3].map(Value::int).into_iter());
        assert_eq!(some.len(), 3);
        assert_eq!(some[2], Value::int(3));
        #[allow(unsafe_code)]
        // SAFETY: there are three, so there's something at the start.
        let first = unsafe { *some.as_ptr() };
        assert_eq!(first, Value::int(1));
    }

    #[test]
    fn changing_the_prototype_keeps_the_properties() {
        let mut shapes = Shapes::default();
        let root = shapes.root(NONE);
        let shape = shapes.add(root, atom(1), PLAIN);
        let moved = shapes.with_proto(shape, 7);
        assert_eq!(shapes.items[moved as usize].proto, 7);
        assert_eq!(shapes.lookup(moved, atom(1)), Some((0, PLAIN)));
    }
}
