//! Many properties at once, for Rust that makes, reads and fills JavaScript objects, as rsui-tsx's styling does for every
//! element it draws. Going through [`Vm::get_property`] and [`Vm::set_property`] a property at a time finds where each one is
//! again, and checks again what the object is; JavaScript's own code doesn't, as each place in it remembers the shape it saw
//! and where the property was in it. These remember the same, for a list of names Rust gives once ([`Keys`]) and for each pair
//! of shapes `assign` has copied between:
//!
//! - [`Vm::new_object_with`] makes an object that has just those names, in that order, as an object literal does: the shape is
//!   found once, and the values go straight into its slots.
//! - [`Vm::read_keys`] reads them all from an object, with where each is in its shape remembered for the last few shapes.
//! - [`Vm::assign`] is `Object.assign(target, source)`: for each shape of target and source it's met, what goes where, and the
//!   shape the target ends with, is worked out once, so copying is reading and writing slots.
//!
//! A shape is a number that a full collection can free and give to another, which [`Shapes::generation`] says has happened,
//! and what's remembered here is forgotten then. Adding a property is only right while no prototype has a setter or a read-only
//! one of that name, which [`Vm::epoch`] going up says may have changed, as it does for what the interpreter remembers.

use std::collections::HashMap;

use foldhash::fast::FixedState;

use crate::object::{ACCESSOR, IS_PROTO, Kind, LAZY_FUNCTION, NEVER_DICT, NONE, NOT_EXTENSIBLE, PLAIN, WRITABLE};
use crate::text::{Atom, AtomKind};
use crate::value::Value;
use crate::vm::Vm;

type Run<T> = Result<T, Value>;

/// A list of property names, found once, for [`Vm::new_object_with`], [`Vm::read_keys`] and [`Vm::write_keys`]. It's for the
/// engine that made it, and lasts as long as that does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keys(u32);

impl Keys {
    /// How many names there are, which is how many values go with them.
    pub fn len(self, vm: &Vm) -> usize {
        vm.number_strings.bulk.lists[self.0 as usize].atoms.len()
    }

    pub fn is_empty(self, vm: &Vm) -> bool {
        self.len(vm) == 0
    }
}

/// How many shapes [`Vm::read_keys`] remembers for each list.
const READ_WAYS: usize = 8;
/// How many pairs of shapes `assign` remembers before it forgets them all and starts again. A page's styles copy between a
/// few thousand: shadcn's sink meets about 6,000.
const MOST_PAIRS: usize = 16384;

struct List {
    atoms: Box<[Atom]>,
    /// Whether every name is one that's simply in an object's shape: not a symbol, nor a number, which go elsewhere or first.
    plain: bool,
    /// The shape of an object with just these, from `Object.prototype`, and the generation of shapes it's from, or `NONE`.
    made: u32,
    made_generation: u32,
    /// For the last few shapes read: the shape, and where each name is in it, `NONE` for one that isn't there. What isn't
    /// there is `undefined` only while nothing on the way up has it, which is as long as the epoch it was found in.
    reads: [(u32, u32, Box<[u32]>); READ_WAYS],
    next_read: u8,
}

/// What `assign` does from one shape into another: which slots of the source go over which of the target, which go on the
/// end, and the shape the target has then.
struct Pair {
    /// The shape the target has after, or `NONE` if nothing's added.
    to: u32,
    /// The epoch it's right for, when something's added.
    epoch: u32,
    /// Pairs of (source slot, target slot) for what's replaced, then the source slots of what's added, in order.
    moves: Box<[u32]>,
    replaced: u32,
}

#[derive(Default)]
pub(crate) struct Bulk {
    lists: Vec<List>,
    pairs: HashMap<u64, Pair, FixedState>,
    /// The generation of shapes what's remembered is from.
    generation: u32,
    /// What's read out of a source before the target's written, kept for the next time.
    values: Vec<Value>,
    entries: Vec<(Atom, u32, u8)>,
    slots: Vec<u32>,
}

impl Bulk {
    pub(crate) fn memory(&self) -> usize {
        self.pairs.capacity() * (size_of::<u64>() + size_of::<Pair>())
            + self.pairs.values().map(|pair| pair.moves.len() * 4).sum::<usize>()
            + self.lists.iter().map(|list| list.atoms.len() * 4 + list.reads.iter().map(|read| read.2.len() * 4).sum::<usize>()).sum::<usize>()
    }
}

impl Vm {
    /// Forgets what's remembered of shapes, if a collection has freed any since.
    #[inline]
    fn bulk_shapes_now(&mut self) {
        if self.number_strings.bulk.generation != self.shapes.generation {
            self.number_strings.bulk.generation = self.shapes.generation;
            self.number_strings.bulk.pairs.clear();
            for list in &mut self.number_strings.bulk.lists {
                list.made = NONE;
                for read in &mut list.reads {
                    read.0 = NONE;
                }
            }
        }
    }

    /// A list of property names for [`new_object_with`](Self::new_object_with), [`read_keys`](Self::read_keys) and
    /// [`write_keys`](Self::write_keys). Make it once, as it's kept for as long as the engine is.
    pub fn keys(&mut self, names: &[&str]) -> Keys {
        let atoms: Vec<Atom> = names.iter().map(|name| self.intern_str(name)).collect();
        self.keys_of(&atoms)
    }

    /// [`keys`](Self::keys), from atoms [`atom`](Self::atom) gave.
    pub fn keys_of(&mut self, atoms: &[Atom]) -> Keys {
        let plain = atoms.iter().enumerate().all(|(at, key)| {
            self.atoms.kind_of(*key) == AtomKind::Name && self.atoms.index_of(*key).is_none() && !atoms[..at].contains(key)
        });
        self.number_strings.bulk.lists.push(List {
            atoms: atoms.into(),
            plain,
            made: NONE,
            made_generation: 0,
            reads: std::array::from_fn(|_| (NONE, 0, Box::default())),
            next_read: 0,
        });
        Keys((self.number_strings.bulk.lists.len() - 1) as u32)
    }

    /// The atom of each of the names, in order.
    pub fn keys_atoms(&self, keys: Keys) -> &[Atom] {
        &self.number_strings.bulk.lists[keys.0 as usize].atoms
    }

    /// A new object with the names `keys` has, in that order, holding `values`, as `{ a: 1, b: 2 }` makes: there have to be as
    /// many values as names. Like anything else made for Rust, it's held until the host function gives back.
    pub fn new_object_with(&mut self, keys: Keys, values: &[Value]) -> Value {
        self.bulk_shapes_now();
        let list = &self.number_strings.bulk.lists[keys.0 as usize];
        assert_eq!(list.atoms.len(), values.len(), "a value for each name");
        let mut shape = list.made;
        if shape == NONE || list.made_generation != self.shapes.generation {
            shape = self.made_shape(keys);
        }
        if shape == NONE {
            // Names that aren't simply in a shape go the long way, as they'd be found then.
            let object = self.new_plain();
            for at in 0..values.len() {
                let key = self.number_strings.bulk.lists[keys.0 as usize].atoms[at];
                self.define(object.handle(), key, values[at], PLAIN);
            }
            return object;
        }
        let object = self.alloc_object(shape, Kind::Plain, 0);
        if values.len() > 4 {
            self.debt += values.len() * 8;
        }
        // It's new, so it's young, and needs no write barrier.
        self.objects.for_the_engine(object.index()).set_slots(values);
        object
    }

    /// The shape an object with just the names of `keys` has, found again, or `NONE` if they aren't all plain names, or are
    /// too many to share one.
    fn made_shape(&mut self, keys: Keys) -> u32 {
        let list = &self.number_strings.bulk.lists[keys.0 as usize];
        if !list.plain {
            return NONE;
        }
        let mut shape = self.realm.object_shape;
        for at in 0..self.number_strings.bulk.lists[keys.0 as usize].atoms.len() {
            if self.shapes.wants_dict(shape) {
                return NONE;
            }
            let key = self.number_strings.bulk.lists[keys.0 as usize].atoms[at];
            shape = self.shapes.add(shape, key, PLAIN);
        }
        let list = &mut self.number_strings.bulk.lists[keys.0 as usize];
        list.made = shape;
        list.made_generation = self.shapes.generation;
        shape
    }

    /// Reads each of the names of `keys` from `object` into `out`, as `object.name` would, `undefined` for what it hasn't
    /// got: `out` has to be as long as the list. Where each is in the shape of a plain object is remembered, for the last few
    /// shapes this list was read from.
    pub fn read_keys(&mut self, object: Value, keys: Keys, out: &mut [Value]) -> Run<()> {
        let count = self.number_strings.bulk.lists[keys.0 as usize].atoms.len();
        assert_eq!(count, out.len(), "a place for each name");
        if object.is_object() && self.number_strings.bulk.lists[keys.0 as usize].plain {
            self.bulk_shapes_now();
            let found = &self.objects[object.index()];
            if matches!(found.kind, Kind::Plain) && found.flags & LAZY_FUNCTION == 0 && !self.shapes.is_dict(found.shape) {
                let shape = found.shape;
                let list = &self.number_strings.bulk.lists[keys.0 as usize];
                let way = list.reads.iter().position(|read| read.0 == shape && read.1 == self.epoch);
                let way = match way {
                    Some(way) => Some(way),
                    None => self.remember_reads(keys, shape),
                };
                // What isn't remembered for this shape yet goes the long way, which remembers it, as a read in JavaScript does.
                if let Some(way) = way {
                    let slots = &self.number_strings.bulk.lists[keys.0 as usize].reads[way].2;
                    let found = &self.objects[object.index()];
                    for (at, slot) in slots.iter().enumerate() {
                        // SAFETY: the slot is one the object's shape has.
                        #[allow(unsafe_code)]
                        let value = if *slot == NONE { Value::UNDEFINED } else { unsafe { found.read_slot(*slot as usize) } };
                        out[at] = value;
                    }
                    return Ok(());
                }
            }
        }
        for at in 0..count {
            let key = self.number_strings.bulk.lists[keys.0 as usize].atoms[at];
            out[at] = self.get_by_atom(object, key)?;
        }
        Ok(())
    }

    /// Finds where each of a list's names is in a plain object's shape from what reads remember (see `Vm::remembered_value`),
    /// and remembers it for the list, if each is a plain one, or isn't there or on the way up. Says which way it's remembered
    /// in. Looking for a name in a shape with many, as a look has, can be slow, as a shape that hasn't been looked in has a
    /// table of them made, and what reads remember is found quickly.
    fn remember_reads(&mut self, keys: Keys, shape: u32) -> Option<usize> {
        let count = self.number_strings.bulk.lists[keys.0 as usize].atoms.len();
        let mut slots = std::mem::take(&mut self.number_strings.bulk.slots);
        slots.clear();
        for at in 0..count {
            let key = self.number_strings.bulk.lists[keys.0 as usize].atoms[at];
            match self.remembered_value(shape, key) {
                Some((NONE, slot)) => slots.push(slot),
                Some((holder, _)) if holder == self.absent => slots.push(NONE),
                _ => {
                    self.number_strings.bulk.slots = slots;
                    return None;
                }
            }
        }
        let epoch = self.epoch;
        let list = &mut self.number_strings.bulk.lists[keys.0 as usize];
        let way = list.next_read as usize % READ_WAYS;
        list.next_read = list.next_read.wrapping_add(1);
        // Where a way's been used before, its list is used again, as looks of many shapes are read one after another.
        let read = &mut list.reads[way];
        read.0 = shape;
        read.1 = epoch;
        if read.2.len() == count {
            read.2.copy_from_slice(&slots);
        } else {
            read.2 = slots.as_slice().into();
        }
        self.number_strings.bulk.slots = slots;
        Some(way)
    }

    /// Whether nothing on the way up from an object of this shape has a property of this name, nor could say what one does:
    /// there's no proxy or function that hasn't made its own yet.
    fn nothing_above(&mut self, shape: u32, key: Atom) -> bool {
        let mut at = self.shapes.proto_of(shape);
        while at != NONE {
            let proto = &self.objects[at as usize];
            if proto.flags & LAZY_FUNCTION != 0 || matches!(proto.kind, Kind::Proxy(_)) {
                return false;
            }
            let proto_shape = proto.shape;
            if self.shapes.lookup(proto_shape, key).is_some() {
                return false;
            }
            at = self.shapes.proto_of(proto_shape);
        }
        true
    }

    /// [`read_keys`](Self::read_keys) of an object's own properties alone: what it hasn't got itself is `undefined`, whatever
    /// its prototype has, which is nothing for what Rust and its own JavaScript make of names no one else uses, as rsui's
    /// looks are. A getter is called. Nothing is remembered, so it's as quick on objects of thousands of shapes, one after
    /// another, as on one: finding a name that isn't there is mostly a look at a bit for it in the shape.
    pub fn read_own_keys(&mut self, object: Value, keys: Keys, out: &mut [Value]) -> Run<()> {
        let count = self.number_strings.bulk.lists[keys.0 as usize].atoms.len();
        assert_eq!(count, out.len(), "a place for each name");
        if !object.is_object() || !matches!(self.objects[object.index()].kind, Kind::Plain) || !self.number_strings.bulk.lists[keys.0 as usize].plain {
            return self.read_keys(object, keys, out);
        }
        let shape = self.objects[object.index()].shape;
        for at in 0..count {
            let key = self.number_strings.bulk.lists[keys.0 as usize].atoms[at];
            out[at] = match self.shapes.lookup(shape, key) {
                None => Value::UNDEFINED,
                Some((slot, flags)) if flags & ACCESSOR == 0 => self.objects[object.index()].slots()[slot as usize],
                Some(_) => self.get_by_atom(object, key)?,
            };
        }
        Ok(())
    }

    /// Writes each of `values` to the names of `keys` on `object`, in order, as `object.name = value` does.
    pub fn write_keys(&mut self, object: Value, keys: Keys, values: &[Value]) -> Run<()> {
        let count = self.number_strings.bulk.lists[keys.0 as usize].atoms.len();
        assert_eq!(count, values.len(), "a value for each name");
        for at in 0..count {
            let key = self.number_strings.bulk.lists[keys.0 as usize].atoms[at];
            if object.is_object() {
                self.set_remembered(object.handle(), key, values[at])?;
            } else {
                self.set(object, key, values[at])?;
            }
        }
        Ok(())
    }

    /// An own property of a plain object, if it has one that's a plain value, without anything on the way up being looked
    /// at: `None` for one that isn't there, or isn't a plain value, or isn't a plain object.
    #[inline]
    pub fn own_value(&mut self, object: Value, key: Atom) -> Option<Value> {
        if !object.is_object() {
            return None;
        }
        let found = &self.objects[object.index()];
        if !matches!(found.kind, Kind::Plain) {
            return None;
        }
        let shape = found.shape;
        let (slot, flags) = self.shapes.lookup(shape, key)?;
        if flags & ACCESSOR != 0 {
            return None;
        }
        self.objects[object.index()].slots().get(slot as usize).copied()
    }

    /// Whether a plain object's shape has a name, which for one that hasn't is found without looking through them all.
    #[inline]
    pub fn has_own_name(&mut self, object: Value, key: Atom) -> bool {
        object.is_object() && {
            let shape = self.objects[object.index()].shape;
            self.shapes.lookup(shape, key).is_some()
        }
    }

    /// Which shape a plain object has, for Rust that remembers what it found out about objects of a shape, as which of their
    /// properties are of some kind: the same number is another shape's only once a collection has freed this one, which
    /// changes the number. `None` for what isn't a plain object, or is one with too many properties to share a shape.
    #[inline]
    pub fn shape_stamp(&self, object: Value) -> Option<u64> {
        if !object.is_object() {
            return None;
        }
        let found = &self.objects[object.index()];
        (matches!(found.kind, Kind::Plain) && !self.shapes.is_dict(found.shape))
            .then(|| u64::from(self.shapes.generation) << 32 | u64::from(found.shape))
    }

    /// The names of a plain object's own properties, in the order they were added, with where each is, for
    /// [`slot_value`](Self::slot_value) on an object with the same [`shape_stamp`](Self::shape_stamp), or on this one until
    /// it's changed, for one with too many to share a shape. Nothing for what isn't a plain object.
    pub fn own_slots_into(&self, object: Value, out: &mut Vec<(Atom, u32)>) {
        out.clear();
        if !object.is_object() || !matches!(self.objects[object.index()].kind, Kind::Plain) {
            return;
        }
        let shape = self.objects[object.index()].shape;
        let backwards = self.shapes.visit_entries(shape, |key, slot| out.push((key, slot)));
        if backwards {
            out.reverse();
        }
    }

    /// What's in a slot of a plain object, found by [`own_slots_into`](Self::own_slots_into) for its shape, or `undefined` if
    /// it hasn't got that many. An accessor's slot holds what has its getter and setter, not what they give.
    #[inline]
    pub fn slot_value(&self, object: Value, slot: u32) -> Value {
        if !object.is_object() {
            return Value::UNDEFINED;
        }
        self.objects[object.index()].slots().get(slot as usize).copied().unwrap_or(Value::UNDEFINED)
    }

    /// Takes these properties out of an object, as `delete object.name` does for each in strict code, keeping the rest in a shape it can
    /// share with others. Deleting from an object of more than a few properties gives it a shape of its own, a dictionary,
    /// which is right for an object used as a table that has things taken out time after time, and slow for one that's
    /// read and added to as objects that share shapes are, as an element's look is. For a plain object with only plain
    /// properties, the rest are put in a shape found from its prototype's, one property at a time as they were added, which
    /// another object with the same in the same order has too; anything else has them deleted one at a time.
    pub fn delete_keys(&mut self, object: Value, keys: &[Atom]) -> Run<()> {
        if keys.is_empty() || !object.is_object() {
            return Ok(());
        }
        let found = &self.objects[object.index()];
        let shape = found.shape;
        if !matches!(found.kind, Kind::Plain)
            || found.flags & (IS_PROTO | NEVER_DICT | LAZY_FUNCTION) != 0
            || self.shapes.is_dict(shape)
            || !self.shapes.is_all_plain(shape)
        {
            for key in keys {
                self.delete(object.handle(), *key)?;
            }
            return Ok(());
        }
        let mut entries = std::mem::take(&mut self.number_strings.bulk.entries);
        self.shapes.entries_into(shape, &mut entries);
        let mut values = std::mem::take(&mut self.number_strings.bulk.values);
        values.clear();
        let mut at = self.shapes.root(self.shapes.proto_of(shape));
        let mut any = false;
        for &(key, slot, flags) in &entries {
            if keys.contains(&key) {
                any = true;
                continue;
            }
            at = self.shapes.add(at, key, flags);
            values.push(self.objects[object.index()].slots()[slot as usize]);
        }
        if any {
            let into = &mut self.objects[object.index()];
            into.shape = at;
            into.set_slots(&values);
        }
        entries.clear();
        self.number_strings.bulk.entries = entries;
        values.clear();
        self.number_strings.bulk.values = values;
        Ok(())
    }

    /// `Object.assign(target, source)`, for one source: see the module's comment.
    pub fn assign(&mut self, target: Value, source: Value) -> Run<()> {
        if self.assign_remembered(target, source) || self.assign_into_dict(target, source) {
            return Ok(());
        }
        self.assign_slowly(target, source)
    }

    /// Copies from a plain object of plain properties into a plain object that has a shape of its own, as one that's had a
    /// property deleted has, one property at a time but without the checks the long way has for other kinds. Says whether
    /// it did, which is before anything's changed if it doesn't.
    fn assign_into_dict(&mut self, target: Value, source: Value) -> bool {
        if !target.is_object() || !source.is_object() {
            return false;
        }
        let (into, from) = (&self.objects[target.index()], &self.objects[source.index()]);
        if !matches!(into.kind, Kind::Plain)
            || !matches!(from.kind, Kind::Plain)
            || into.flags & (NOT_EXTENSIBLE | IS_PROTO | LAZY_FUNCTION) != 0
            || from.flags & LAZY_FUNCTION != 0
            || !self.shapes.is_dict(into.shape)
            || self.shapes.is_dict(from.shape)
            || !self.shapes.is_all_plain(from.shape)
        {
            return false;
        }
        let (into_shape, from_shape) = (into.shape, from.shape);
        let mut entries = std::mem::take(&mut self.number_strings.bulk.entries);
        self.shapes.entries_into(from_shape, &mut entries);
        // Everything's looked at first, so that nothing's done if any of it can't be.
        let fits = entries.iter().all(|&(key, _, _)| {
            self.atoms.kind_of(key) == AtomKind::Name && self.atoms.index_of(key).is_none()
        }) && (0..entries.len()).all(|at| {
            let key = entries[at].0;
            match self.shapes.lookup(into_shape, key) {
                Some((_, flags)) => flags & (ACCESSOR | WRITABLE) == WRITABLE,
                None => self.nothing_above(into_shape, key),
            }
        });
        if fits {
            for &(key, slot, _) in &entries {
                // SAFETY: the slot is one the source's shape has.
                #[allow(unsafe_code)]
                let value = unsafe { self.objects[source.index()].read_slot(slot as usize) };
                match self.shapes.lookup(into_shape, key) {
                    Some((at, _)) => self.objects[target.index()].slots_mut()[at as usize] = value,
                    None => {
                        self.shapes.dict_add(into_shape, key, PLAIN);
                        self.objects[target.index()].push_slot(value);
                        self.debt += 8;
                    }
                }
            }
        }
        entries.clear();
        self.number_strings.bulk.entries = entries;
        fits
    }

    /// Copies from a plain object of plain properties into a plain object, if what goes where for their two shapes is known
    /// or can be found and remembered. Says whether it did.
    #[allow(unsafe_code)]
    pub(crate) fn assign_remembered(&mut self, target: Value, source: Value) -> bool {
        if !target.is_object() || !source.is_object() {
            return false;
        }
        let (into, from) = (&self.objects[target.index()], &self.objects[source.index()]);
        if !matches!(into.kind, Kind::Plain)
            || !matches!(from.kind, Kind::Plain)
            || into.flags & (NOT_EXTENSIBLE | IS_PROTO | LAZY_FUNCTION | NEVER_DICT) != 0
            || from.flags & LAZY_FUNCTION != 0
        {
            return false;
        }
        let (into_shape, from_shape) = (into.shape, from.shape);
        self.bulk_shapes_now();
        let key = u64::from(into_shape) << 32 | u64::from(from_shape);
        let known = match self.number_strings.bulk.pairs.get(&key) {
            Some(pair) => pair.to == NONE || pair.epoch == self.epoch,
            None => false,
        };
        if !known && !self.remember_pair(into_shape, from_shape) {
            return false;
        }
        let Some(pair) = self.number_strings.bulk.pairs.get(&key) else {
            return false;
        };
        let replaced = pair.replaced as usize * 2;
        let from = &self.objects[source.index()];
        let values = &mut self.number_strings.bulk.values;
        values.clear();
        // SAFETY: each slot is one the source's shape has, as the pair is for it.
        values.extend(pair.moves.iter().step_by(2).take(pair.replaced as usize).map(|slot| unsafe { from.read_slot(*slot as usize) }));
        values.extend(pair.moves[replaced..].iter().map(|slot| unsafe { from.read_slot(*slot as usize) }));
        let to = pair.to;
        // What's written goes through the write barrier, as the target can be old.
        let into = &mut self.objects[target.index()];
        for (at, value) in values[..pair.replaced as usize].iter().enumerate() {
            // SAFETY: each slot is one the target's shape has.
            unsafe { into.write_slot(pair.moves[at * 2 + 1] as usize, *value) };
        }
        let added = values.len() - pair.replaced as usize;
        if added > 0 {
            into.shape = to;
            for value in &values[pair.replaced as usize..] {
                into.push_slot(*value);
            }
            self.debt += added * 8;
        }
        true
    }

    /// Works out what goes where from one shape into another, and remembers it, if the source's shape is only plain
    /// properties with plain names, what they replace in the target is plain and writable, and what's added isn't on the way
    /// up from the target. Says whether it could.
    fn remember_pair(&mut self, into_shape: u32, from_shape: u32) -> bool {
        if self.shapes.is_dict(into_shape) || self.shapes.is_dict(from_shape) || !self.shapes.is_all_plain(from_shape) {
            return false;
        }
        let mut entries = std::mem::take(&mut self.number_strings.bulk.entries);
        self.shapes.entries_into(from_shape, &mut entries);
        let mut moves = Vec::with_capacity(entries.len() * 2);
        let mut added = Vec::new();
        let mut to = into_shape;
        let mut fits = true;
        for &(key, slot, _) in &entries {
            if self.atoms.kind_of(key) != AtomKind::Name || self.atoms.index_of(key).is_some() {
                fits = false;
                break;
            }
            match self.shapes.lookup(into_shape, key) {
                Some((at, flags)) if flags & (ACCESSOR | WRITABLE) == WRITABLE => moves.extend([slot, at]),
                Some(_) => {
                    fits = false;
                    break;
                }
                None => {
                    if self.shapes.wants_dict(to) || !self.nothing_above(into_shape, key) {
                        fits = false;
                        break;
                    }
                    to = self.shapes.add(to, key, PLAIN);
                    added.push(slot);
                }
            }
        }
        entries.clear();
        self.number_strings.bulk.entries = entries;
        if !fits {
            return false;
        }
        if self.number_strings.bulk.pairs.len() >= MOST_PAIRS {
            self.number_strings.bulk.pairs.clear();
        }
        let replaced = (moves.len() / 2) as u32;
        let to = if added.is_empty() { NONE } else { to };
        moves.extend(added);
        self.number_strings.bulk.pairs.insert(
            u64::from(into_shape) << 32 | u64::from(from_shape),
            Pair {
                to,
                epoch: self.epoch,
                moves: moves.into(),
                replaced,
            },
        );
        true
    }

    /// `Object.assign` for one source the long way, as the built-in does it for what isn't plain.
    pub(crate) fn assign_slowly(&mut self, target: Value, source: Value) -> Run<()> {
        if source.is_nullish() {
            return Ok(());
        }
        let target = self.to_object(target)?;
        let source = self.to_object(source)?;
        for (key, flags) in self.own_keys(source.handle()) {
            if flags & crate::object::ENUMERABLE != 0 {
                let value = self.get(source, key)?;
                self.set(target, key, value)?;
            }
        }
        Ok(())
    }
}
