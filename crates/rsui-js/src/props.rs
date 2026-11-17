//! Reading and writing properties, the long way. The interpreter has short ways for what's common, and comes here
//! for the rest, which is also where it learns what to remember for next time.

use crate::object::*;
use crate::text::{self, Atom, AtomKind};
use crate::value::Value;
use crate::vm::{ConstructorMemo, Run, Vm};

/// The most elements an array can have. A bigger index would need a kind of array that isn't here yet.
const MAX_ARRAY_LENGTH: usize = 1 << 28;

/// How many places there are to remember where properties were found. Library code reads through many shapes at one
/// place: chart.js's option resolvers read each name from objects of thousands of shapes (about 4,000 at once), and at
/// 256 what one read remembered was mostly gone by the time the next of the same shape came. 2,048 places of each kind
/// are 80 KB.
const LOOKUPS: usize = 2048;

/// Diagnostic ablation of the frame-free getter path. Bytecode and descriptors are unchanged.
fn fast_getters_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("RSUI_FAST_GETTERS").is_none_or(|value| value != "0"))
}

/// Experimental shared-shape deletion for small ordinary data bags.
fn compact_delete_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("RSUI_COMPACT_DELETE").is_some_and(|value| value == "1"))
}

/// Set in a remembered slot when the property there is an accessor, whose getter has to be called.
const ACCESSOR_SLOT: u32 = 1 << 31;
/// The remembered slot of a property that isn't there at all, on the object or any prototype.
const ABSENT_SLOT: u32 = ACCESSOR_SLOT - 1;
/// Set in a remembered slot as well as `ACCESSOR_SLOT` when the getter is the built-in one for a typed array's `length`
/// and it's on a prototype, so it can't change without the epoch changing (see `define_accessor`). For a typed array,
/// the length can then be worked out without calling it.
const TYPED_LENGTH_SLOT: u32 = 1 << 30;
/// Where writes are remembered, after the reads, in the same list.
const STORES: usize = LOOKUPS;

/// Where in `Vm::lookups` what's remembered for a shape and a property can be: this place or the one after it. With only
/// one, two that landed on the same place took turns there, and which ones did changed with the shapes' numbers: a
/// bench whose reads see 24 shapes missed 4% or 12% of the time depending on how many shapes were made before them.
#[inline]
fn lookup_way(shape: u32, key: Atom) -> usize {
    (shape ^ key.0.wrapping_mul(0x9E37_79B1).rotate_left(7)) as usize & (LOOKUPS - 2)
}

impl Vm {
    // Functions make their `name`, `length` and `prototype` when they're first needed.

    #[inline]
    fn check_lazy(&mut self, object: u32, key: Atom) {
        if self.objects[object as usize].flags & LAZY_FUNCTION != 0
            && (key == text::NAME || key == text::LENGTH || key == text::PROTOTYPE)
        {
            self.materialize(object);
        }
    }

    #[inline]
    pub(crate) fn materialize_if_lazy(&mut self, object: u32) {
        if self.objects[object as usize].flags & LAZY_FUNCTION != 0 {
            self.materialize(object);
        }
    }

    fn materialize(&mut self, object: u32) {
        self.objects[object as usize].flags &= !LAZY_FUNCTION;
        let (name, length, proto_flags) = match &self.objects[object as usize].kind {
            Kind::Function { proto, .. } => {
                let proto = &self.protos[*proto as usize];
                (proto.name, proto.length, Some(proto.flags))
            }
            Kind::Native(native) => (native.name, native.length, None),
            _ => return,
        };
        self.add_property(
            object,
            text::LENGTH,
            Value::int(i32::from(length)),
            CONFIGURABLE,
        );
        let name = self.atom_name_value(name);
        self.add_property(object, text::NAME, name, CONFIGURABLE);
        if let Some(flags) = proto_flags {
            let wants_prototype = flags & crate::op::IS_CLASS == 0
                && (flags & crate::op::IS_CONSTRUCTOR != 0 || flags & crate::op::IS_GENERATOR != 0);
            if wants_prototype {
                let is_generator = flags & crate::op::IS_GENERATOR != 0;
                let parent = if !is_generator {
                    self.realm.object_proto
                } else if flags & crate::op::IS_ASYNC != 0 {
                    self.realm.async_generator_proto
                } else {
                    self.realm.generator_proto
                };
                let shape = self.root_shape(parent);
                let prototype = self.alloc_object(shape, Kind::Plain, 1);
                if !is_generator {
                    self.add_property(
                        prototype.handle(),
                        text::CONSTRUCTOR,
                        Value::object(object),
                        HIDDEN,
                    );
                }
                self.add_property(object, text::PROTOTYPE, prototype, WRITABLE);
            }
        }
    }

    /// What goes in a function's `name`, which for a symbol is its description in brackets.
    fn atom_name_value(&mut self, atom: Atom) -> Value {
        match self.atoms.kind_of(atom) {
            AtomKind::Name => self.atom_value(atom),
            _ => {
                let text = format!("[{}]", self.atom_text(atom));
                self.new_str(&text)
            }
        }
    }

    // Adding and changing.

    pub(crate) fn add_property(&mut self, object: u32, key: Atom, value: Value, flags: u8) {
        let found = &mut self.objects[object as usize];
        if found.flags & IS_PROTO != 0 {
            self.epoch = self.epoch.wrapping_add(1);
            #[cfg(feature = "counts")]
            crate::counts::epoch("a property added to a prototype", key.0);
            if key.held_index().is_some() {
                self.prototypes_have_elements = true;
            }
        }
        let mut shape = found.shape;
        if !self.shapes.is_dict(shape)
            && self.shapes.wants_dict(shape)
            && found.flags & NEVER_DICT == 0
        {
            shape = self.shapes.to_dict(shape);
            found.shape = shape;
        }
        if self.shapes.is_dict(shape) {
            self.shapes.dict_add(shape, key, flags);
        } else {
            found.shape = self.shapes.add(shape, key, flags);
        }
        found.push_slot(value);
        self.debt += 8;
    }

    /// Makes or replaces a property on an object itself, whatever its prototype has.
    pub(crate) fn define(&mut self, object: u32, key: Atom, value: Value, flags: u8) {
        self.check_lazy(object, key);
        if self.define_special(object, key, value, flags == PLAIN) {
            return;
        }
        if let Some((target, _)) = self.proxy_of(object) {
            self.define(target.handle(), key, value, flags);
            return;
        }
        let shape = self.objects[object as usize].shape;
        match self.shapes.lookup(shape, key) {
            Some((slot, old_flags)) => {
                if old_flags != flags {
                    self.change_flags(object, key, flags);
                }
                self.objects[object as usize].slots_mut()[slot as usize] = value;
                if self.objects[object as usize].flags & IS_PROTO != 0 {
                    self.epoch = self.epoch.wrapping_add(1);
                    #[cfg(feature = "counts")]
                    crate::counts::epoch("a prototype's property defined again", key.0);
                }
            }
            None => self.add_property(object, key, value, flags),
        }
    }

    fn define_special(&mut self, object: u32, key: Atom, value: Value, plain: bool) -> bool {
        let found = &mut self.objects[object as usize];
        if !matches!(found.kind, Kind::Array(_) | Kind::Arguments(_)) {
            return false;
        }
        if let Some(index) = self.atoms.index_of(key) {
            let (Kind::Array(items) | Kind::Arguments(items)) = &mut found.kind else {
                return false;
            };
            let index = index as usize;
            if index < MAX_ARRAY_LENGTH {
                if index >= items.len() {
                    items.vec().resize(index + 1, Value::EMPTY);
                }
                // One that isn't plain has to stay in the shape, and there's a hole here that leads to it.
                if !plain {
                    items[index] = Value::EMPTY;
                    found.flags |= SPECIAL_ELEMENTS;
                    return false;
                }
                items[index] = value;
                if found.flags & SPECIAL_ELEMENTS != 0 {
                    let shape = found.shape;
                    if self.shapes.lookup(shape, key).is_some() {
                        let _ = self.delete_from_shape(object, key);
                    }
                }
                return true;
            }
        }
        false
    }

    pub(crate) fn change_flags(&mut self, object: u32, key: Atom, flags: u8) {
        let found = &mut self.objects[object as usize];
        if found.flags & IS_PROTO != 0 {
            self.epoch = self.epoch.wrapping_add(1);
            #[cfg(feature = "counts")]
            crate::counts::epoch("a prototype's property's flags", key.0);
        }
        if self.shapes.is_dict(found.shape) {
            self.shapes.dict_set_flags(found.shape, key, flags);
        } else {
            found.shape =
                self.shapes.with_flags(
                    found.shape,
                    |found_key, old| if found_key == key { flags } else { old },
                );
        }
    }

    pub(crate) fn define_accessor(
        &mut self,
        object: u32,
        key: Atom,
        getter: Option<Value>,
        setter: Option<Value>,
        flags: u8,
    ) {
        self.check_lazy(object, key);
        // An element of an array that's an accessor lives in the shape, with a hole in the array.
        if self.define_special(object, key, Value::UNDEFINED, false) {
            return;
        }
        let shape = self.objects[object as usize].shape;
        let flags = flags | ACCESSOR;
        if let Some((slot, old_flags)) = self.shapes.lookup(shape, key) {
            if old_flags & ACCESSOR != 0 {
                // What's remembered can say which getter a prototype has (see `TYPED_LENGTH_SLOT`).
                if self.objects[object as usize].flags & IS_PROTO != 0 {
                    self.epoch = self.epoch.wrapping_add(1);
                    #[cfg(feature = "counts")]
                    crate::counts::epoch("a prototype's accessor defined again", key.0);
                }
                let holder = self.objects[object as usize].slots()[slot as usize];
                if let Kind::Accessor { get, set } = &mut self.objects[holder.index()].kind {
                    if let Some(getter) = getter {
                        *get = getter;
                    }
                    if let Some(setter) = setter {
                        *set = setter;
                    }
                }
                if old_flags != flags {
                    self.change_flags(object, key, flags);
                }
                return;
            }
        }
        let shape = self.shapes.root(NONE);
        let holder = self.alloc_object(
            shape,
            Kind::Accessor {
                get: getter.unwrap_or(Value::UNDEFINED),
                set: setter.unwrap_or(Value::UNDEFINED),
            },
            0,
        );
        self.define(object, key, holder, flags);
    }

    // Reading.

    /// A property that isn't in the shape: the elements of an array, say, or the length of one.
    fn own_special(&self, object: u32, key: Atom) -> Option<Value> {
        match &self.objects[object as usize].kind {
            Kind::Array(items) | Kind::Arguments(items) => {
                if key == text::LENGTH {
                    return Some(Value::number(items.len() as f64));
                }
                let index = self.atoms.index_of(key)? as usize;
                items.get(index).copied().filter(|value| !value.is_empty())
            }
            Kind::Boxed(value) if value.is_string() => {
                let string = &self.strings.items[value.index()].text;
                if key == text::LENGTH {
                    return Some(Value::int(string.len() as i32));
                }
                let index = self.atoms.index_of(key)? as usize;
                (index < string.len()).then_some(Value::UNDEFINED)
            }
            _ => None,
        }
    }

    pub(crate) fn get(&mut self, target: Value, key: Atom) -> Run<Value> {
        if target.is_object() {
            return self.get_from(target.handle(), key, target);
        }
        if target.is_string() {
            let string = &self.strings.items[target.index()].text;
            if key == text::LENGTH {
                return Ok(Value::int(string.len() as i32));
            }
            if let Some(index) = self.atoms.index_of(key) {
                return Ok(self.char_at(target, index as usize));
            }
            return self.get_from(self.realm.string_proto, key, target);
        }
        let proto = self.proto_of_primitive(target, key)?;
        self.get_from(proto, key, target)
    }

    fn proto_of_primitive(&mut self, target: Value, key: Atom) -> Run<u32> {
        if target.is_string() {
            return Ok(self.realm.string_proto);
        }
        if target.is_number() {
            return Ok(self.realm.number_proto);
        }
        if target.is_bool() {
            return Ok(self.realm.boolean_proto);
        }
        if target.is_symbol() {
            return Ok(self.realm.symbol_proto);
        }
        if target.is_bigint() {
            return Ok(self.realm.bigint_proto);
        }
        let shown = if target.is_null() {
            "null"
        } else {
            "undefined"
        };
        let key = self.atom_text(key);
        Err(self.type_error(&format!(
            "Cannot read properties of {shown} (reading '{key}')"
        )))
    }

    pub(crate) fn get_from(&mut self, start: u32, key: Atom, receiver: Value) -> Run<Value> {
        let start_shape = self.objects[start as usize].shape;
        if let Some(found) = self.remembered(start_shape, key) {
            if self.can_use_remembered(start, key) {
                return self.use_remembered(start, receiver, found);
            }
        }
        if self.atoms.kind_of(key) == AtomKind::Hidden {
            return self.get_private(start, key, receiver);
        }
        let mut at = start;
        let mut first_shape = NONE;
        loop {
            self.check_lazy(at, key);
            if matches!(self.objects[at as usize].kind, Kind::Proxy(_)) {
                return self.proxy_get(at, key, receiver);
            }
            if matches!(self.objects[at as usize].kind, Kind::TypedArray(_)) {
                // A number is always the array's own business, even one past its end.
                if let Some(index) = self.atoms.index_of(key) {
                    return Ok(self
                        .typed_get(Value::object(at), index as usize)
                        .unwrap_or(Value::UNDEFINED));
                }
            }
            if let Some(value) = self.own_special(at, key) {
                if let Kind::Boxed(boxed) = self.objects[at as usize].kind {
                    if key != text::LENGTH {
                        let index = self.atoms.index_of(key).unwrap_or(0) as usize;
                        return Ok(self.char_at(boxed, index));
                    }
                }
                return Ok(value);
            }
            let shape = self.objects[at as usize].shape;
            if at == start {
                first_shape = shape;
            }
            if let Some((slot, flags)) = self.shapes.lookup(shape, key) {
                let value = self.objects[at as usize].slots()[slot as usize];
                if flags & ACCESSOR == 0 {
                    self.remember_found(first_shape, key, start, at, slot);
                    return Ok(value);
                }
                let Kind::Accessor { get, .. } = self.objects[value.index()].kind else {
                    unreachable!("an accessor property holds an accessor");
                };
                let typed_length = if get.is_undefined() {
                    None
                } else {
                    self.typed_length_by_getter(get, receiver)
                };
                let marks = if typed_length.is_some() && at != start {
                    ACCESSOR_SLOT | TYPED_LENGTH_SLOT
                } else {
                    ACCESSOR_SLOT
                };
                self.remember_found(first_shape, key, start, at, slot | marks);
                if get.is_undefined() {
                    return Ok(Value::UNDEFINED);
                }
                if let Some(length) = typed_length {
                    return Ok(length);
                }
                return self.call_accessor(get, receiver);
            }
            let proto = self.shapes.items[shape as usize].proto;
            if proto == NONE {
                self.remember_found(first_shape, key, start, 0, ABSENT_SLOT);
                return Ok(Value::UNDEFINED);
            }
            at = proto;
        }
    }

    /// What's remembered for this property on objects of this shape, from the two places from `way` (a
    /// [`lookup_way`], after `STORES` for a write), whether it still holds or not.
    #[inline(always)]
    fn lookup_at(&self, way: usize, shape: u32, key: Atom) -> Option<&crate::vm::Lookup> {
        let pair = self.lookups.get(way..way + 2)?;
        pair.iter()
            .find(|found| found.shape == shape && found.atom == key.0)
    }

    /// What was found before for this property on objects of this shape, if it still holds.
    #[inline(always)]
    fn remembered(&self, shape: u32, key: Atom) -> Option<crate::vm::Lookup> {
        let found = self.lookup_at(lookup_way(shape, key), shape, key)?;
        (found.holder == NONE || found.epoch == self.epoch).then_some(*found)
    }

    /// Whether what was found for an object's shape goes for this object too. What the shape doesn't say has to be
    /// looked at: a function that hasn't made its `name` and the like yet, or the length of an array.
    #[inline(always)]
    fn can_use_remembered(&self, object: u32, key: Atom) -> bool {
        let found = &self.objects[object as usize];
        if found.flags & LAZY_FUNCTION != 0
            && (key == text::NAME || key == text::LENGTH || key == text::PROTOTYPE)
        {
            return false;
        }
        !(key == text::LENGTH
            && matches!(
                found.kind,
                Kind::Array(_) | Kind::Arguments(_) | Kind::Boxed(_)
            ))
    }

    /// Reads what [`remembered`](Self::remembered) said is where.
    #[inline(always)]
    fn use_remembered(
        &mut self,
        object: u32,
        receiver: Value,
        found: crate::vm::Lookup,
    ) -> Run<Value> {
        if found.slot == ABSENT_SLOT {
            return Ok(Value::UNDEFINED);
        }
        if found.slot & ACCESSOR_SLOT != 0 {
            if found.slot & TYPED_LENGTH_SLOT != 0 && receiver.is_object() {
                if let Kind::TypedArray(data) = &self.objects[receiver.index()].kind {
                    return Ok(Value::int_or_double(self.typed_length_now(data)));
                }
            }
            return self.get_through_remembered_accessor(object, receiver, found);
        }
        let holder = if found.holder == NONE {
            object
        } else {
            found.holder
        };
        Ok(self.objects[holder as usize].slots()[found.slot as usize])
    }

    /// Remembers where a property was found, or that it wasn't, for objects of the shape the search started with, if
    /// that's something to go by: a name that isn't a number or private, on an object that shares its shape.
    #[inline]
    fn remember_found(&mut self, first_shape: u32, key: Atom, start: u32, at: u32, slot: u32) {
        if first_shape == NONE
            || self.shapes.is_dict(first_shape)
            || self.atoms.kind_of(key) == AtomKind::Hidden
            || self.atoms.index_of(key).is_some()
        {
            return;
        }
        // Something that isn't there could be added to any object on the way, so it's only true until then.
        let holder = if at == start && slot != ABSENT_SLOT {
            NONE
        } else {
            at
        };
        self.remember_lookup(lookup_way(first_shape, key), first_shape, key, holder, slot);
    }

    /// Where a property that's a plain value was last found on objects of this shape, as (holder, slot), the holder
    /// being `NONE` for the object itself, if that's remembered and still true. It's what [`Vm::get_cached`] looks at
    /// first, for the interpreter to look at without the call, when a place in the code sees more than one shape.
    /// Lazy functions' own properties aren't made yet, and an array's `length` isn't in its shape, so the caller
    /// leaves those to the long way.
    /// A property that isn't there is said to be undefined on [`Vm::absent`], as `learn_absent` does.
    #[inline(always)]
    pub(crate) fn remembered_value(&self, shape: u32, key: Atom) -> Option<(u32, u32)> {
        let found = self.lookup_at(lookup_way(shape, key), shape, key)?;
        if found.holder != NONE && found.epoch != self.epoch {
            return None;
        }
        if found.slot == ABSENT_SLOT {
            return (self.absent != NONE).then_some((self.absent, 0));
        }
        (found.slot & ACCESSOR_SLOT == 0).then_some((found.holder, found.slot))
    }

    /// Where a write of this property to an object of this shape went before, as (the shape it changed to, or `NONE`
    /// if it was already there, and the slot), if that's remembered and still true. It's what [`Vm::set_cached`] looks
    /// at, for a megamorphic SetPropPoly to look at without the call. What isn't remembered (`length`, frozen and
    /// non-extensible objects' new properties, typed arrays' elements, dictionaries) never matches, but lazy
    /// functions' own properties aren't made yet, so the caller leaves those to the long way.
    #[inline(always)]
    pub(crate) fn remembered_store(&self, shape: u32, key: Atom) -> Option<(u32, u32)> {
        let found = self.lookup_at(STORES + lookup_way(shape, key), shape, key)?;
        (found.holder == NONE || found.epoch == self.epoch).then_some((found.holder, found.slot))
    }

    /// Remembers for a place in the code what it found on objects of a shape, in its first entry, which is the one
    /// GetProp, SetProp and the like look at. Once the place sees a second shape it's given [`POLY`] more entries,
    /// what was first moves to one of them (unless it was out of date), and its instruction becomes the kind that looks
    /// at them too (GetPropPoly and the like). When those are all in use the place is megamorphic: its first entry is
    /// just replaced from then on, as what reads remember has the rest. `checks_epoch` says whether what's in `other`
    /// only holds while no prototype changes, as it does for everything but a field being defined.
    ///
    /// Whatever finds what a place should have the long way has to come here, or the place would keep coming back.
    #[inline(always)]
    pub(crate) fn learn(
        &mut self,
        cache: usize,
        shape: u32,
        other: u32,
        slot: u32,
        checks_epoch: bool,
    ) {
        use crate::op::MEGAMORPHIC;
        let epoch = self.epoch;
        let entry = &mut self.caches[cache];
        let old = *entry;
        (entry.shape, entry.other, entry.slot, entry.epoch) = (shape, other, slot, epoch);
        // What's usual, and all a megamorphic place ever does.
        if old.shape == NONE
            || old.shape == shape
            || (old.more != NONE && old.more & MEGAMORPHIC != 0)
            || (checks_epoch && old.other != NONE && old.epoch != epoch)
        {
            return;
        }
        self.learn_another(cache, old, checks_epoch);
    }

    /// What [`learn`](Self::learn) does when a place sees another shape: keeps what it had first in one of its other
    /// entries, giving it those first if it hasn't any yet.
    #[inline(never)]
    fn learn_another(&mut self, cache: usize, old: crate::op::Cache, checks_epoch: bool) {
        use crate::op::{Cache, MEGAMORPHIC, POLY};
        let epoch = self.epoch;
        let shape = self.caches[cache].shape;
        let stale = |entry: &Cache| checks_epoch && entry.other != NONE && entry.epoch != epoch;
        let mut more = old.more;
        if more == NONE {
            more = (self.caches.len() - cache) as u32;
            for _ in 0..POLY {
                self.caches.push(Cache::new(old.atom));
            }
            self.caches[cache].more = more;
            self.go_polymorphic(cache);
        }
        let start = cache + (more & !MEGAMORPHIC) as usize;
        let block = &mut self.caches[start..start + POLY];
        // One for the shape that's now first goes, so a shape is only ever in one place.
        let at = block.iter().position(|entry| entry.shape == shape).or_else(|| {
            block
                .iter()
                .position(|entry| entry.shape == NONE || stale(entry))
        });
        match at {
            Some(at) => {
                let entry = &mut block[at];
                (entry.shape, entry.other, entry.slot, entry.epoch) =
                    (old.shape, old.other, old.slot, old.epoch);
            }
            None => self.caches[cache].more = more | MEGAMORPHIC,
        }
    }

    /// Makes the instruction running now, if it's the one for this place, the kind that looks at the place's other
    /// entries. The frame's `pc` is past it, as the interpreter saves it before coming here. Anything else that comes
    /// here (GetLength, DefineField, a native function's own reads) is left as it is, and looks at them the long way.
    fn go_polymorphic(&mut self, cache: usize) {
        use crate::op::Op;
        let Some(frame) = self.frames.last() else {
            return;
        };
        let at = frame.pc.wrapping_sub(1) as usize;
        let place = cache.wrapping_sub(frame.cache_base as usize);
        let Some(op) = self.code.get_mut(at) else {
            return;
        };
        *op = match *op {
            Op::GetProp { dst, object, cache } if cache as usize == place => Op::GetPropPoly {
                dst,
                object,
                cache,
            },
            Op::SetProp {
                object,
                value,
                cache,
            } if cache as usize == place => Op::SetPropPoly {
                object,
                value,
                cache,
            },
            Op::CallMethod { dst, f, cache } if cache as usize == place => {
                Op::CallMethodPoly { dst, f, cache }
            }
            other => other,
        };
    }

    /// Which of the entries a place in the code has after its first is for this shape, if it has them. The
    /// interpreter's polymorphic instructions look at them themselves; this is for the rest.
    #[inline(always)]
    fn known_more(&self, cache: usize, shape: u32) -> Option<usize> {
        let more = self.caches[cache].more;
        // A megamorphic place goes by what reads and writes remember.
        if more == NONE || more & crate::op::MEGAMORPHIC != 0 {
            return None;
        }
        let start = cache + more as usize;
        (start..start + crate::op::POLY).find(|&at| self.caches[at].shape == shape)
    }

    /// Swaps what a place in the code has first with one of the entries after it, so the interpreter finds it next.
    #[inline(always)]
    fn bring_forward(&mut self, cache: usize, at: usize) {
        let (first, other) = (self.caches[cache], self.caches[at]);
        let entry = &mut self.caches[cache];
        (entry.shape, entry.other, entry.slot, entry.epoch) =
            (other.shape, other.other, other.slot, other.epoch);
        let entry = &mut self.caches[at];
        (entry.shape, entry.other, entry.slot, entry.epoch) =
            (first.shape, first.other, first.slot, first.epoch);
    }

    /// [`Vm::get_cached`] for GetProp and GetPropPoly in the interpreter, whose answer goes to register `dst`: a proxy's
    /// `get` trap can be called in place (see [`Vm::proxy_get_here`]). A private name is the proxy's own, as ever.
    #[inline(never)]
    pub(crate) fn get_cached_here(&mut self, target: Value, cache: usize, dst: u16) -> Run<Value> {
        if target.is_object() {
            let object = target.handle();
            let key = self.caches[cache].atom;
            if matches!(self.objects[object as usize].kind, Kind::Proxy(_)) {
                if self.atoms.kind_of(key) != AtomKind::Hidden {
                    return self.proxy_get_here(object, key, dst);
                }
            } else if let Some(getter) = self.remembered_getter(object, key) {
                return self.call_getter_here(getter, target, dst);
            }
        }
        self.get_cached(target, cache)
    }

    /// The getter written in JavaScript that what reads remember says a property of this object is read through, if
    /// it is: what [`Vm::get_cached`] would come to and call. A place in the code only ever remembers plain values,
    /// so for a getter that's the first thing it looks at that could answer.
    #[inline(always)]
    fn remembered_getter(&self, object: u32, key: Atom) -> Option<(u32, u32)> {
        let shape = self.objects[object as usize].shape;
        let found = self.remembered(shape, key)?;
        if found.slot == ABSENT_SLOT
            || found.slot & ACCESSOR_SLOT == 0
            || found.slot & TYPED_LENGTH_SLOT != 0
            || !self.can_use_remembered(object, key)
        {
            return None;
        }
        let holder = if found.holder == NONE {
            object
        } else {
            found.holder
        };
        let accessor = self.objects[holder as usize].slots()[(found.slot & !ACCESSOR_SLOT) as usize];
        let Kind::Accessor { get, .. } = self.objects[accessor.index()].kind else {
            return None;
        };
        if !get.is_object() {
            return None;
        }
        match self.objects[get.index()].kind {
            Kind::Function { proto, .. } if self.protos[proto as usize].flags & crate::op::IS_CLASS == 0 => {
                Some((get.handle(), proto))
            }
            _ => None,
        }
    }

    /// Calls a getter written in JavaScript in place, as the interpreter does a call, with its answer going to `dst`,
    /// and fails with [`Value::ENTERED`].
    fn call_getter_here(&mut self, (getter, proto): (u32, u32), receiver: Value, dst: u16) -> Run<Value> {
        if let Some(value) = self.trivial_getter_value(Value::object(getter)) {
            return Ok(value);
        }
        let base = self.top;
        self.reserve_stack(base + 1 + 16)?;
        self.stack[base] = receiver;
        self.enter_function(getter, proto, base, 0, dst, 0, Value::UNDEFINED)?;
        Err(Value::ENTERED)
    }

    /// A getter that only returns a literal or captured binding needs no call frame.
    /// Keep normal calls while debugging/counting/profiling so entry instrumentation
    /// and stepping retain their existing behavior.
    fn trivial_getter_value(&self, getter: Value) -> Option<Value> {
        if cfg!(any(feature = "counts", feature = "jsprof"))
            || !fast_getters_enabled()
            || crate::inspect::DEBUG_WANTED.load(std::sync::atomic::Ordering::Relaxed)
        {
            return None;
        }
        self.simple_getter_value(getter)
    }

    /// Inspect existing bytecode rather than retaining another result/cache/root.
    /// Cells are read live; a TDZ goes through the ordinary call for its exact error.
    fn simple_getter_value(&self, getter: Value) -> Option<Value> {
        use crate::op::{IS_ASYNC, IS_CLASS, IS_GENERATOR, KEEPS_ARGS, NOT_COMPILED, Op};
        if !getter.is_object() {
            return None;
        }
        let Kind::Function { proto, upvalues, .. } = &self.objects[getter.index()].kind else {
            return None;
        };
        let id = *proto;
        let proto = &self.protos[id as usize];
        if proto.start == NOT_COMPILED
            || proto.params != 0
            || proto.flags & (IS_ASYNC | IS_CLASS | IS_GENERATOR | KEEPS_ARGS) != 0
            || !proto.handlers.is_empty()
            || (!self.debug_vars.is_empty() && self.debug_vars.contains_key(&id))
        {
            return None;
        }
        let first = *self.code.get(proto.start as usize)?;
        if first == Op::ReturnUndefined {
            return Some(Value::UNDEFINED);
        }
        let Op::Return { src } = *self.code.get(proto.start as usize + 1)? else {
            return None;
        };
        let (dst, value) = match first {
            Op::GetUpValue { dst, index } => (dst, *upvalues.get(index as usize)?),
            Op::GetUp { dst, index } => {
                let cell = *upvalues.get(index as usize)?;
                if !cell.is_cell() {
                    return None;
                }
                (dst, self.cells[cell.index()])
            }
            Op::LoadConst { dst, index } => (dst, self.consts[index as usize]),
            Op::LoadInt { dst, value } => (dst, Value::int(value)),
            Op::LoadUndefined { dst } => (dst, Value::UNDEFINED),
            Op::LoadNull { dst } => (dst, Value::NULL),
            Op::LoadTrue { dst } => (dst, Value::TRUE),
            Op::LoadFalse { dst } => (dst, Value::FALSE),
            _ => return None,
        };
        (dst == src && !value.is_empty() && !value.is_cell()).then_some(value)
    }

    fn call_accessor(&mut self, getter: Value, receiver: Value) -> Run<Value> {
        if let Some(value) = self.trivial_getter_value(getter) {
            return Ok(value);
        }
        self.call(getter, receiver, &[])
    }

    /// [`Vm::get_by_value`] for GetElem in the interpreter, as [`Vm::get_cached_here`] is for GetProp: a proxy read by
    /// a string or a symbol, which needs nothing called to say which property it is, can have its trap called in place.
    #[inline(never)]
    pub(crate) fn get_by_value_here(&mut self, target: Value, key: Value, dst: u16) -> Run<Value> {
        if target.is_object()
            && (key.is_string() || key.is_symbol())
            && matches!(self.objects[target.index()].kind, Kind::Proxy(_))
        {
            let atom = self.to_atom(key)?;
            if self.atoms.kind_of(atom) != AtomKind::Hidden {
                return self.proxy_get_here(target.handle(), atom, dst);
            }
        }
        self.get_by_value(target, key)
    }

    /// Reads a property for a place in the code, and remembers how, if it's something that can be remembered.
    #[inline(never)]
    pub(crate) fn get_cached(&mut self, target: Value, cache: usize) -> Run<Value> {
        let key = self.caches[cache].atom;
        if !target.is_object() {
            if target.is_string() && key != text::LENGTH {
                let proto = self.realm.string_proto;
                if let Some(value) = self.get_from_primitive_proto(proto, key, cache) {
                    return Ok(value);
                }
            } else if target.is_number() {
                // `x.toFixed(2)`, `n.toString(16)`: as for a string.
                let proto = self.realm.number_proto;
                if let Some(value) = self.get_from_primitive_proto(proto, key, cache) {
                    return Ok(value);
                }
            }
            return self.get(target, key);
        }
        let object = target.handle();
        // A proxy's every read is its handler's, and none of what's remembered below is about it. Compiled code
        // leaves every read of a proxy here (chart.js's options are read through 16,000 of them each update).
        if matches!(self.objects[object as usize].kind, Kind::Proxy(_))
            && self.atoms.kind_of(key) != AtomKind::Hidden
        {
            return self.proxy_get(object, key, target);
        }
        if key == text::LENGTH {
            match &self.objects[object as usize].kind {
                Kind::Array(items) | Kind::Arguments(items) => {
                    return Ok(Value::int_or_double(items.len()));
                }
                Kind::Boxed(_) => return self.get(target, key),
                _ => {}
            }
        }
        self.check_lazy(object, key);
        let first_shape = self.objects[object as usize].shape;
        // A place that's seen several shapes but whose instruction doesn't look at its other entries (GetLength).
        if let Some(at) = self.known_more(cache, first_shape) {
            let known = self.caches[at];
            if known.other == NONE || known.epoch == self.epoch {
                self.bring_forward(cache, at);
                let holder = if known.other == NONE {
                    object
                } else {
                    known.other
                };
                return Ok(self.objects[holder as usize].slots()[known.slot as usize]);
            }
        }
        let way = lookup_way(first_shape, key);
        if let Some(found) = self.lookup_at(way, first_shape, key) {
            if found.holder == NONE || found.epoch == self.epoch {
                let found = *found;
                if found.slot == ABSENT_SLOT {
                    self.learn_absent(cache, first_shape, key);
                    return Ok(Value::UNDEFINED);
                }
                if found.slot & ACCESSOR_SLOT != 0 {
                    return self.use_remembered(object, target, found);
                }
                self.learn(cache, found.shape, found.holder, found.slot, true);
                let holder = if found.holder == NONE {
                    object
                } else {
                    found.holder
                };
                return Ok(self.objects[holder as usize].slots()[found.slot as usize]);
            }
        }
        let mut at = object;
        loop {
            if at != object {
                self.check_lazy(at, key);
            }
            match self.objects[at as usize].kind {
                Kind::Proxy(_) => return self.get_from(at, key, target),
                // A typed array's numbers are its own business, and what else it has is the usual.
                Kind::TypedArray(_) if self.atoms.index_of(key).is_some() => {
                    return self.get_from(at, key, target);
                }
                // A prototype that's an array has a length of its own, which isn't in its shape.
                Kind::Array(_) | Kind::Arguments(_) | Kind::Boxed(_)
                    if at != object && key == text::LENGTH =>
                {
                    return self.get_from(at, key, target);
                }
                _ => {}
            }
            let shape = self.objects[at as usize].shape;
            if let Some((slot, flags)) = self.shapes.lookup(shape, key) {
                let value = self.objects[at as usize].slots()[slot as usize];
                if flags & ACCESSOR != 0 {
                    let Kind::Accessor { get, .. } = self.objects[value.index()].kind else {
                        unreachable!("an accessor property holds an accessor");
                    };
                    if get.is_undefined() {
                        if self.atoms.kind_of(key) == AtomKind::Hidden {
                            return self.get_private(at, key, target);
                        }
                        return Ok(Value::UNDEFINED);
                    }
                    // Where a getter is can be remembered just like where a value is, as long as it's a name.
                    let typed_length = self.typed_length_by_getter(get, target);
                    if !self.shapes.is_dict(first_shape)
                        && self.atoms.kind_of(key) == AtomKind::Name
                        && self.atoms.index_of(key).is_none()
                    {
                        let (holder, marks) = if at == object {
                            (NONE, ACCESSOR_SLOT)
                        } else if typed_length.is_some() {
                            (at, ACCESSOR_SLOT | TYPED_LENGTH_SLOT)
                        } else {
                            (at, ACCESSOR_SLOT)
                        };
                        self.remember_lookup(way, first_shape, key, holder, slot | marks);
                    }
                    if let Some(length) = typed_length {
                        return Ok(length);
                    }
                    return self.call_accessor(get, target);
                }
                if !self.shapes.is_dict(first_shape) {
                    let holder = if at == object { NONE } else { at };
                    self.learn(cache, first_shape, holder, slot, true);
                    self.remember_lookup(way, first_shape, key, holder, slot);
                }
                return Ok(value);
            }
            // A private name is only ever an object's own.
            if at == object && self.atoms.kind_of(key) == AtomKind::Hidden {
                return Err(self.private_name_error(key));
            }
            let proto = self.shapes.items[shape as usize].proto;
            if proto == NONE {
                self.remember_found(first_shape, key, object, 0, ABSENT_SLOT);
                self.learn_absent(cache, first_shape, key);
                return Ok(Value::UNDEFINED);
            }
            at = proto;
        }
    }

    /// Has a place in the code remember that objects of this shape don't have this property, and nothing on the way
    /// up from them does, as if it were on [`Vm::absent`], whose one value is undefined. That's only true until a
    /// prototype changes, which the epoch says, as for a property found on one. Not for a name a function makes when
    /// it's first needed (`name`, `length`, `prototype`), an array's `length` or an index, which a shape doesn't say,
    /// nor a private name, which isn't there to be read. A read of a property an object may not have (`props.key`)
    /// went the long way every time before.
    fn learn_absent(&mut self, cache: usize, shape: u32, key: Atom) {
        if self.absent != NONE
            && !self.shapes.is_dict(shape)
            && self.atoms.kind_of(key) != AtomKind::Hidden
            && self.atoms.index_of(key).is_none()
            && key != text::LENGTH
            && key != text::NAME
            && key != text::PROTOTYPE
        {
            self.learn(cache, shape, self.absent, 0, true);
        }
    }

    /// Calls a getter that was found before for objects of this shape, or does what the built-in one for a typed
    /// array's `length` would without calling it.
    #[inline(never)]
    fn get_through_remembered_accessor(
        &mut self,
        object: u32,
        target: Value,
        found: crate::vm::Lookup,
    ) -> Run<Value> {
        let holder = if found.holder == NONE {
            object
        } else {
            found.holder
        };
        let slot = found.slot & !(ACCESSOR_SLOT | TYPED_LENGTH_SLOT);
        let accessor = self.objects[holder as usize].slots()[slot as usize];
        let Kind::Accessor { get, .. } = self.objects[accessor.index()].kind else {
            unreachable!("an accessor property holds an accessor");
        };
        if get.is_undefined() {
            return Ok(Value::UNDEFINED);
        }
        if let Some(length) = self.typed_length_by_getter(get, target) {
            return Ok(length);
        }
        self.call_accessor(get, target)
    }

    #[inline]
    fn remember_lookup(&mut self, way: usize, shape: u32, key: Atom, holder: u32, slot: u32) {
        if self.lookups.is_empty() {
            self.lookups.resize(
                LOOKUPS + STORES,
                crate::vm::Lookup {
                    shape: NONE,
                    atom: 0,
                    holder: NONE,
                    slot: 0,
                    epoch: 0,
                },
            );
        }
        // What's newest goes in the first of the two places, and what was there moves to the second, unless it's
        // what this replaces.
        let pair = &mut self.lookups[way..way + 2];
        let same = |found: &crate::vm::Lookup| found.shape == shape && found.atom == key.0;
        let at = if same(&pair[1]) {
            1
        } else {
            if !same(&pair[0]) {
                pair[1] = pair[0];
            }
            0
        };
        pair[at] = crate::vm::Lookup {
            shape,
            atom: key.0,
            holder,
            slot,
            epoch: self.epoch,
        };
    }

    /// A method of a string or a number, on `String.prototype` or `Number.prototype` (`proto`), which it's worth
    /// remembering just like one on an object.
    fn get_from_primitive_proto(&mut self, proto: u32, key: Atom, cache: usize) -> Option<Value> {
        let shape = self.objects[proto as usize].shape;
        let entry = self.caches[cache];
        // Where it was found, on the prototype or further up, or that it's nowhere (`Vm::absent`, whose one value is
        // undefined), for as long as the prototype has its shape and no prototype has changed.
        if entry.shape == shape && entry.other != NONE && entry.epoch == self.epoch {
            return Some(self.objects[entry.other as usize].slots()[entry.slot as usize]);
        }
        // A place that's seen objects too keeps it among its other entries, as for any shape (see `learn`).
        if let Some(at) = self.known_more(cache, shape) {
            let known = self.caches[at];
            if known.other != NONE && known.epoch == self.epoch {
                self.bring_forward(cache, at);
                return Some(self.objects[known.other as usize].slots()[known.slot as usize]);
            }
        }
        if self.atoms.index_of(key).is_some() || self.atoms.kind_of(key) == AtomKind::Hidden {
            return None;
        }
        let mut at = proto;
        let (holder, slot) = loop {
            if !matches!(self.objects[at as usize].kind, Kind::Plain | Kind::Boxed(_)) {
                return None;
            }
            let here = self.objects[at as usize].shape;
            if let Some((slot, flags)) = self.shapes.lookup(here, key) {
                if flags & ACCESSOR != 0 {
                    return None;
                }
                break (at, slot);
            }
            let up = self.shapes.items[here as usize].proto;
            if up == NONE {
                // Nowhere: a string has no `key` (`child.key` of a string child), as for an object that doesn't.
                if self.absent == NONE || key == text::NAME || key == text::PROTOTYPE {
                    return None;
                }
                break (self.absent, 0);
            }
            at = up;
        };
        // A change to what it holds then has to count, as it does for anything that's a prototype. The prototype's
        // shape stands for the string's or number's in the place's entries.
        self.objects[proto as usize].flags |= IS_PROTO;
        self.learn(cache, shape, holder, slot, true);
        Some(self.objects[holder as usize].slots()[slot as usize])
    }

    /// Reads a property that was asked for by a value, which no place in the code can remember for itself.
    pub(crate) fn get_by_atom(&mut self, target: Value, key: Atom) -> Run<Value> {
        if !target.is_object() {
            return self.get(target, key);
        }
        self.get_from(target.handle(), key, target)
    }

    /// Puts a private name on an object, which can only happen once. It doesn't matter what kind of object it is, as even
    /// a frozen one or a proxy can be given them.
    pub(crate) fn add_private(
        &mut self,
        object: u32,
        key: Atom,
        value: Value,
        flags: u8,
    ) -> Run<()> {
        let shape = self.objects[object as usize].shape;
        if self.shapes.lookup(shape, key).is_some() {
            let name = self.atom_text(key);
            return Err(self.type_error(&format!(
                "Cannot initialize {name} twice on the same object"
            )));
        }
        self.add_property(object, key, value, flags);
        Ok(())
    }

    /// Reads a property without anything running, or `None` if finding out would take that, as with a getter or a proxy.
    /// Whether reading `key` from this object would call the built-in getter `func`, found as `peek_property` finds a
    /// value: a built-in getter that only reads what the object is (a regular expression's `flags`) can then be
    /// skipped, as calling it does nothing else.
    pub(crate) fn has_builtin_getter(&mut self, start: u32, key: Atom, func: NativeFn) -> bool {
        let mut at = start;
        loop {
            if matches!(self.objects[at as usize].kind, Kind::Proxy(_)) {
                return false;
            }
            self.check_lazy(at, key);
            let shape = self.objects[at as usize].shape;
            if let Some((slot, flags)) = self.shapes.lookup(shape, key) {
                if flags & ACCESSOR == 0 {
                    return false;
                }
                let accessor = self.objects[at as usize].slots()[slot as usize];
                let Kind::Accessor { get, .. } = self.objects[accessor.index()].kind else {
                    return false;
                };
                return get.is_object()
                    && matches!(&self.objects[get.index()].kind,
                        Kind::Native(native) if native.func as usize == func as usize);
            }
            let proto = self.shapes.items[shape as usize].proto;
            if proto == NONE {
                return false;
            }
            at = proto;
        }
    }

    pub(crate) fn peek_property(&mut self, start: u32, key: Atom) -> Option<Value> {
        let mut at = start;
        loop {
            if matches!(self.objects[at as usize].kind, Kind::Proxy(_)) {
                return None;
            }
            self.check_lazy(at, key);
            let shape = self.objects[at as usize].shape;
            if let Some((slot, flags)) = self.shapes.lookup(shape, key) {
                if flags & ACCESSOR != 0 {
                    return None;
                }
                return Some(self.objects[at as usize].slots()[slot as usize]);
            }
            let proto = self.shapes.items[shape as usize].proto;
            if proto == NONE {
                return Some(Value::UNDEFINED);
            }
            at = proto;
        }
    }

    /// Reads a private name, which is only ever the object's own: nothing that inherits from it gets to use it, and a
    /// proxy doesn't hear about it.
    fn get_private(&mut self, object: u32, key: Atom, receiver: Value) -> Run<Value> {
        let shape = self.objects[object as usize].shape;
        let Some((slot, flags)) = self.shapes.lookup(shape, key) else {
            return Err(self.private_name_error(key));
        };
        let value = self.objects[object as usize].slots()[slot as usize];
        if flags & ACCESSOR != 0 {
            let Kind::Accessor { get, .. } = self.objects[value.index()].kind else {
                unreachable!("an accessor property holds an accessor");
            };
            if get.is_undefined() {
                let name = self.atom_text(key);
                return Err(self.type_error(&format!("{name} was defined without a getter")));
            }
            return self.call_accessor(get, receiver);
        }
        Ok(value)
    }

    fn private_name_error(&mut self, key: Atom) -> Value {
        let name = self.atom_text(key);
        self.type_error(&format!(
            "Cannot use private member {name} on an object that doesn't have it"
        ))
    }

    #[inline(never)]
    pub(crate) fn get_global_slow(&mut self, cache: usize) -> Run<Value> {
        let key = self.caches[cache].atom;
        let global = self.global;
        let shape = self.objects[global as usize].shape;
        if let Some((slot, flags)) = self.shapes.lookup(shape, key) {
            if flags & ACCESSOR == 0 {
                let entry = &mut self.caches[cache];
                entry.shape = shape;
                entry.slot = slot;
                return Ok(self.objects[global as usize].slots()[slot as usize]);
            }
            return self.get(Value::object(global), key);
        }
        if self.has_property(global, key) {
            return self.get(Value::object(global), key);
        }
        let name = self.atom_text(key);
        Err(self.reference_error(&format!("{name} is not defined")))
    }

    #[inline(never)]
    pub(crate) fn set_global_slow(&mut self, cache: usize, value: Value) -> Run<()> {
        let key = self.caches[cache].atom;
        let global = self.global;
        if !self.has_property(global, key) {
            let name = self.atom_text(key);
            return Err(self.reference_error(&format!("{name} is not defined")));
        }
        self.set_on(global, key, value)
    }

    pub(crate) fn has_property(&mut self, start: u32, key: Atom) -> bool {
        self.has_property_checked(start, key).unwrap_or(false)
    }

    pub(crate) fn has_property_checked(&mut self, start: u32, key: Atom) -> Run<bool> {
        // What a read found, or didn't, on objects of this shape is the answer, as reads remember what they found on
        // a prototype, or that it wasn't anywhere, only for as long as no prototype changes.
        let first_shape = self.objects[start as usize].shape;
        if let Some(found) = self.remembered(first_shape, key) {
            if self.can_use_remembered(start, key) {
                return Ok(found.slot != ABSENT_SLOT);
            }
        }
        let mut at = start;
        loop {
            if matches!(self.objects[at as usize].kind, Kind::Proxy(_)) {
                return self.proxy_has(at, key);
            }
            if self.has_own(at, key) {
                return Ok(true);
            }
            let proto = self.shapes.items[self.objects[at as usize].shape as usize].proto;
            if proto == NONE {
                // Each object on the way was looked at the same way a read would have, and none of them had it.
                if self.objects[start as usize].shape == first_shape {
                    self.remember_found(first_shape, key, start, 0, ABSENT_SLOT);
                }
                return Ok(false);
            }
            at = proto;
        }
    }

    pub(crate) fn has_own(&mut self, object: u32, key: Atom) -> bool {
        let shape = self.objects[object as usize].shape;
        // Whether the shape has it is all a remembered read can say, so an array's elements and `length`, or a lazy
        // function's own, are left to the long way, as are proxies (whose shapes are never remembered).
        if let Some(found) = self.remembered(shape, key) {
            if self.can_use_remembered(object, key) {
                return found.holder == NONE && found.slot != ABSENT_SLOT;
            }
        }
        if matches!(self.objects[object as usize].kind, Kind::Proxy(_)) {
            return self
                .proxy_descriptor(object, key)
                .is_ok_and(|descriptor| !descriptor.is_undefined());
        }
        self.check_lazy(object, key);
        if matches!(self.objects[object as usize].kind, Kind::TypedArray(_)) {
            if let Some(index) = self.atoms.index_of(key) {
                return self
                    .typed_get(Value::object(object), index as usize)
                    .is_some();
            }
        }
        if self.own_special(object, key).is_some() {
            return true;
        }
        let shape = self.objects[object as usize].shape;
        self.shapes.lookup(shape, key).is_some()
    }

    pub(crate) fn has_in(&mut self, key: Value, object: Value) -> Run<bool> {
        if !object.is_object() {
            let shown = self.brief(object);
            return Err(self.type_error(&format!(
                "Cannot use 'in' operator to search for a key in {shown}"
            )));
        }
        let atom = self.to_atom(key)?;
        if self.atoms.kind_of(atom) == AtomKind::Hidden {
            let shape = self.objects[object.index()].shape;
            return Ok(self.shapes.lookup(shape, atom).is_some());
        }
        self.has_property_checked(object.handle(), atom)
    }

    // Writing.

    pub(crate) fn set(&mut self, target: Value, key: Atom, value: Value) -> Run<()> {
        if target.is_object() {
            return self.set_on(target.handle(), key, value);
        }
        if target.is_nullish() {
            let shown = if target.is_null() {
                "null"
            } else {
                "undefined"
            };
            let key = self.atom_text(key);
            return Err(self.type_error(&format!(
                "Cannot set properties of {shown} (setting '{key}')"
            )));
        }
        // A setter on the prototype still gets called, for a value that's not an object.
        let proto = self.proto_of_primitive(target, key)?;
        let mut at = proto;
        while at != NONE {
            let shape = self.objects[at as usize].shape;
            if let Some((slot, flags)) = self.shapes.lookup(shape, key) {
                if flags & ACCESSOR != 0 {
                    return self.write_found(at, slot, flags, key, value, target);
                }
                break;
            }
            at = self.proto_of(at);
        }
        self.write_refused = true;
        let key = self.atom_text(key);
        Err(self.type_error(&format!("Cannot create property '{key}' on a primitive")))
    }

    fn set_special(&mut self, object: u32, key: Atom, value: Value) -> Run<bool> {
        let found = &self.objects[object as usize];
        if matches!(found.kind, Kind::TypedArray(_)) {
            return match self.atoms.index_of(key) {
                Some(index) => {
                    self.typed_set(Value::object(object), index as usize, value)?;
                    Ok(true)
                }
                None => Ok(false),
            };
        }
        if !matches!(found.kind, Kind::Array(_) | Kind::Arguments(_)) {
            return Ok(false);
        }
        if key == text::LENGTH {
            if matches!(found.kind, Kind::Arguments(_)) {
                return Ok(false);
            }
            self.set_array_length(object, value)?;
            return Ok(true);
        }
        match self.atoms.index_of(key) {
            Some(index) => {
                self.set_element(object, index as usize, value)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub(crate) fn set_element(&mut self, object: u32, index: usize, value: Value) -> Run<()> {
        let found = &mut self.objects[object as usize];
        // A hole might have something special behind it, here or on a prototype.
        let is_hole = match &found.kind {
            Kind::Array(items) | Kind::Arguments(items) => {
                items.get(index).is_none_or(|item| item.is_empty())
            }
            _ => false,
        };
        if is_hole && (found.flags & SPECIAL_ELEMENTS != 0 || self.prototypes_have_elements) {
            let key = self.index_atom(index);
            if self.set_through_chain(object, key, value)? {
                return Ok(());
            }
        }
        let found = &mut self.objects[object as usize];
        if found.flags & FROZEN_ELEMENTS != 0 {
            self.write_refused = true;
            return Err(self.type_error(&format!(
                "Cannot assign to read only property '{index}' of object"
            )));
        }
        let not_extensible = found.flags & NOT_EXTENSIBLE != 0;
        let read_only_length = found.flags & READ_ONLY_LENGTH != 0;
        if found.flags & IS_PROTO != 0 {
            self.prototypes_have_elements = true;
        }
        let (Kind::Array(items) | Kind::Arguments(items)) = &mut found.kind else {
            unreachable!("only arrays come here");
        };
        if read_only_length && index >= items.len() {
            self.write_refused = true;
            return Err(self.type_error("Cannot add an element past a fixed length"));
        }
        if index < items.len() {
            items[index] = value;
            return Ok(());
        }
        if not_extensible {
            self.write_refused = true;
            return Err(self.type_error("Cannot add an element, object is not extensible"));
        }
        if index >= MAX_ARRAY_LENGTH {
            return Err(self.range_error("An array can't be this long yet"));
        }
        if index > items.len() {
            items.vec().resize(index, Value::EMPTY);
            self.debt += (index - items.len()) * 8;
        }
        items.push(value);
        self.debt += 8;
        Ok(())
    }

    fn set_array_length(&mut self, object: u32, value: Value) -> Run<()> {
        let number = self.to_number(value)?;
        if number < 0.0 || number.fract() != 0.0 || number > f64::from(u32::MAX) {
            return Err(self.range_error("Invalid array length"));
        }
        if number as usize > MAX_ARRAY_LENGTH {
            return Err(self.range_error("An array can't be this long yet"));
        }
        let found = &mut self.objects[object as usize];
        if found.flags & (FROZEN_ELEMENTS | READ_ONLY_LENGTH) != 0 {
            self.write_refused = true;
            return Err(self.type_error("Cannot change the length of a frozen array"));
        }
        let Kind::Array(items) = &mut found.kind else {
            unreachable!("only arrays come here");
        };
        items.vec().resize(number as usize, Value::EMPTY);
        if found.flags & SPECIAL_ELEMENTS != 0 {
            // What's past the new end goes too, unless one of them can't, and then that's the end.
            let entries = self.shapes.entries(self.objects[object as usize].shape);
            let mut indexes: Vec<(u32, Atom, u8)> = entries
                .iter()
                .filter_map(|(key, _, flags)| {
                    self.atoms.index_of(*key).map(|index| (index, *key, *flags))
                })
                .filter(|(index, _, _)| f64::from(*index) >= number)
                .collect();
            indexes.sort_by_key(|entry| std::cmp::Reverse(entry.0));
            for (index, key, flags) in indexes {
                if flags & CONFIGURABLE == 0 {
                    if let Kind::Array(items) = &mut self.objects[object as usize].kind {
                        items.vec().resize(index as usize + 1, Value::EMPTY);
                    }
                    self.write_refused = true;
                    return Err(self.type_error("Cannot delete an element that can't be deleted"));
                }
                self.delete_from_shape(object, key)?;
            }
        }
        Ok(())
    }

    /// Looks for something a write to a hole should go through instead. It says whether it found one.
    fn set_through_chain(&mut self, object: u32, key: Atom, value: Value) -> Run<bool> {
        let mut at = object;
        while at != NONE {
            if matches!(self.objects[at as usize].kind, Kind::Proxy(_)) {
                self.proxy_set(at, key, value, Value::object(object))?;
                return Ok(true);
            }
            let shape = self.objects[at as usize].shape;
            if let Some((slot, flags)) = self.shapes.lookup(shape, key) {
                if at == object || flags & ACCESSOR != 0 {
                    self.write_found(at, slot, flags, key, value, Value::object(object))?;
                    return Ok(true);
                }
                if flags & WRITABLE == 0 {
                    return Err(self.read_only_error(key));
                }
                return Ok(false);
            }
            at = self.proto_of(at);
        }
        Ok(false)
    }

    pub(crate) fn set_on(&mut self, object: u32, key: Atom, value: Value) -> Run<()> {
        if self.atoms.kind_of(key) == AtomKind::Hidden {
            let shape = self.objects[object as usize].shape;
            let Some((slot, flags)) = self.shapes.lookup(shape, key) else {
                return Err(self.private_name_error(key));
            };
            return self.write_found(object, slot, flags, key, value, Value::object(object));
        }
        if matches!(self.objects[object as usize].kind, Kind::Proxy(_)) {
            return self.proxy_set(object, key, value, Value::object(object));
        }
        self.check_lazy(object, key);
        if self.set_special(object, key, value)? {
            return Ok(());
        }
        let shape = self.objects[object as usize].shape;
        if let Some((slot, flags)) = self.shapes.lookup(shape, key) {
            return self.write_found(object, slot, flags, key, value, Value::object(object));
        }
        // Something on a prototype might say what happens.
        let mut at = self.shapes.items[shape as usize].proto;
        while at != NONE {
            self.check_lazy(at, key);
            if matches!(self.objects[at as usize].kind, Kind::Proxy(_)) {
                return self.proxy_set(at, key, value, Value::object(object));
            }
            let proto_shape = self.objects[at as usize].shape;
            if let Some((slot, flags)) = self.shapes.lookup(proto_shape, key) {
                if flags & ACCESSOR != 0 {
                    return self.write_found(at, slot, flags, key, value, Value::object(object));
                }
                if flags & WRITABLE == 0 {
                    return Err(self.read_only_error(key));
                }
                break;
            }
            at = self.shapes.items[proto_shape as usize].proto;
        }
        if self.objects[object as usize].flags & NOT_EXTENSIBLE != 0 {
            self.write_refused = true;
            let key = self.atom_text(key);
            return Err(self.type_error(&format!(
                "Cannot add property {key}, object is not extensible"
            )));
        }
        self.add_property(object, key, value, PLAIN);
        Ok(())
    }

    fn read_only_error(&mut self, key: Atom) -> Value {
        self.write_refused = true;
        let key = self.atom_text(key);
        self.type_error(&format!(
            "Cannot assign to read only property '{key}' of object"
        ))
    }

    fn write_found(
        &mut self,
        object: u32,
        slot: u32,
        flags: u8,
        key: Atom,
        value: Value,
        receiver: Value,
    ) -> Run<()> {
        if flags & ACCESSOR != 0 {
            let holder = self.objects[object as usize].slots()[slot as usize];
            let Kind::Accessor { set, .. } = self.objects[holder.index()].kind else {
                unreachable!("an accessor property holds an accessor");
            };
            if set.is_undefined() {
                self.write_refused = true;
                let key = self.atom_text(key);
                return Err(self.type_error(&format!(
                    "Cannot set property {key} of object which has only a getter"
                )));
            }
            self.call(set, receiver, &[value])?;
            return Ok(());
        }
        if flags & WRITABLE == 0 {
            return Err(self.read_only_error(key));
        }
        self.objects[object as usize].slots_mut()[slot as usize] = value;
        Ok(())
    }

    /// Writes a property for a place in the code, and remembers how, if it can.
    #[inline(never)]
    pub(crate) fn set_cached(&mut self, target: Value, value: Value, cache: usize) -> Run<()> {
        let key = self.caches[cache].atom;
        if !target.is_object() {
            return self.set(target, key, value);
        }
        let object = target.handle();
        self.check_lazy(object, key);
        if key == text::LENGTH
            || self.objects[object as usize].flags & NOT_EXTENSIBLE != 0
            || matches!(self.objects[object as usize].kind, Kind::TypedArray(_))
        {
            return self.set_on(object, key, value);
        }
        let shape = self.objects[object as usize].shape;
        if self.shapes.is_dict(shape) {
            return self.set_on(object, key, value);
        }
        // A place that's seen several shapes, which its instruction (SetProp before it becomes SetPropPoly) doesn't
        // look at.
        if let Some(at) = self.known_more(cache, shape) {
            let known = self.caches[at];
            let into = &mut self.objects[object as usize];
            if known.other == NONE {
                into.slots_mut()[known.slot as usize] = value;
                self.bring_forward(cache, at);
                return Ok(());
            }
            if known.epoch == self.epoch && into.flags & IS_PROTO == 0 {
                into.shape = known.other;
                into.push_slot(value);
                self.debt += 8;
                self.bring_forward(cache, at);
                return Ok(());
            }
        }
        // A place in the code that sees objects of many shapes still finds what it did before for each of them.
        let way = STORES + lookup_way(shape, key);
        if let Some(found) = self.lookup_at(way, shape, key) {
            let found = *found;
            if found.holder == NONE {
                self.learn(cache, shape, NONE, found.slot, true);
                self.objects[object as usize].slots_mut()[found.slot as usize] = value;
                return Ok(());
            }
            let into = &mut self.objects[object as usize];
            if found.epoch == self.epoch && into.flags & IS_PROTO == 0 {
                into.shape = found.holder;
                into.push_slot(value);
                self.debt += 8;
                self.learn(cache, shape, found.holder, found.slot, true);
                return Ok(());
            }
        }
        if let Some((slot, flags)) = self.shapes.lookup(shape, key) {
            if flags & (ACCESSOR | WRITABLE) == WRITABLE {
                self.learn(cache, shape, NONE, slot, true);
                self.objects[object as usize].slots_mut()[slot as usize] = value;
                if self.atoms.index_of(key).is_none() {
                    self.remember_lookup(way, shape, key, NONE, slot);
                }
                return Ok(());
            }
            return self.set_on(object, key, value);
        }
        // A private name can only be written to if it's already there.
        if self.atoms.kind_of(key) == AtomKind::Hidden {
            return Err(self.private_name_error(key));
        }
        // If nothing on the way up has this, it's added, and it'll go the same way next time.
        let mut at = self.shapes.items[shape as usize].proto;
        while at != NONE {
            self.check_lazy(at, key);
            let proto_shape = self.objects[at as usize].shape;
            if self.shapes.lookup(proto_shape, key).is_some()
                || matches!(self.objects[at as usize].kind, Kind::Proxy(_))
            {
                return self.set_on(object, key, value);
            }
            at = self.shapes.items[proto_shape as usize].proto;
        }
        self.add_property(object, key, value, PLAIN);
        let new_shape = self.objects[object as usize].shape;
        if !self.shapes.is_dict(new_shape) {
            let slot = self.objects[object as usize].slots().len() as u32 - 1;
            self.learn(cache, shape, new_shape, slot, true);
            if self.atoms.index_of(key).is_none() {
                // Adding it is only right for as long as no prototype gets a setter or a read-only one of these.
                self.remember_lookup(way, shape, key, new_shape, slot);
            }
        }
        Ok(())
    }

    /// [`Vm::set_on`] for a built-in that writes to plain objects over and over, as `Object.assign` does: what a write
    /// of this name to an object of this shape did before is in what writes remember, as it is for places in the code
    /// that see many shapes (see [`Vm::set_cached`]), and is done again without looking anything up. Whatever has to go
    /// the long way is remembered for next time.
    pub(crate) fn set_remembered(&mut self, object: u32, key: Atom, value: Value) -> Run<()> {
        let found = &self.objects[object as usize];
        let shape = found.shape;
        if !matches!(found.kind, Kind::Plain)
            || found.flags & (NOT_EXTENSIBLE | LAZY_FUNCTION) != 0
            || self.atoms.kind_of(key) != AtomKind::Name
            || self.atoms.index_of(key).is_some()
            || self.shapes.is_dict(shape)
        {
            return self.set_on(object, key, value);
        }
        let way = LOOKUPS + lookup_way(shape, key);
        if let Some(seen) = self.lookups.get(way) {
            if seen.shape == shape && seen.atom == key.0 {
                let seen = *seen;
                if seen.holder == NONE {
                    self.objects[object as usize].slots_mut()[seen.slot as usize] = value;
                    return Ok(());
                }
                let into = &mut self.objects[object as usize];
                if seen.epoch == self.epoch && into.flags & IS_PROTO == 0 {
                    into.shape = seen.holder;
                    into.push_slot(value);
                    self.debt += 8;
                    return Ok(());
                }
            }
        }
        if let Some((slot, flags)) = self.shapes.lookup(shape, key) {
            if flags & (ACCESSOR | WRITABLE) == WRITABLE {
                self.objects[object as usize].slots_mut()[slot as usize] = value;
                self.remember_lookup(way, shape, key, NONE, slot);
                return Ok(());
            }
            return self.set_on(object, key, value);
        }
        // Anything on the way up with this name could say what happens.
        let mut at = self.shapes.items[shape as usize].proto;
        while at != NONE {
            self.check_lazy(at, key);
            let proto_shape = self.objects[at as usize].shape;
            if self.shapes.lookup(proto_shape, key).is_some()
                || matches!(self.objects[at as usize].kind, Kind::Proxy(_))
            {
                return self.set_on(object, key, value);
            }
            at = self.shapes.items[proto_shape as usize].proto;
        }
        self.add_property(object, key, value, PLAIN);
        let new_shape = self.objects[object as usize].shape;
        if !self.shapes.is_dict(new_shape) && self.objects[object as usize].flags & IS_PROTO == 0
        {
            let slot = self.objects[object as usize].slots().len() as u32 - 1;
            // Adding it is only right for as long as no prototype gets a setter or a read-only one of these.
            self.remember_lookup(way, shape, key, new_shape, slot);
        }
        Ok(())
    }

    /// Makes a property of an object literal or a class's field for a place in the code.
    #[inline(never)]
    pub(crate) fn define_cached(&mut self, object: u32, value: Value, cache: usize) -> Run<()> {
        let key = self.caches[cache].atom;
        // A place that's seen several shapes (a field of a base class, defined on objects of its subclasses).
        if let Some(at) = self.known_more(cache, self.objects[object as usize].shape) {
            let into = &mut self.objects[object as usize];
            into.shape = self.caches[at].other;
            into.push_slot(value);
            self.bring_forward(cache, at);
            return Ok(());
        }
        if self.atoms.kind_of(key) == AtomKind::Hidden {
            // Private names never turn up in a copy, or anywhere else.
            let shape = self.objects[object as usize].shape;
            self.add_private(object, key, value, WRITABLE)?;
            // An object of this shape doesn't have it yet, so the next one can just be given it.
            let new_shape = self.objects[object as usize].shape;
            if !self.shapes.is_dict(shape) && !self.shapes.is_dict(new_shape) {
                self.learn(cache, shape, new_shape, 0, false);
            }
            return Ok(());
        }
        let shape = self.objects[object as usize].shape;
        if self.shapes.is_dict(shape) {
            self.define(object, key, value, PLAIN);
            return Ok(());
        }
        if let Some((slot, flags)) = self.shapes.lookup(shape, key) {
            // Said again, as in `{ ...state, count: 1 }`, which is just a new value for it.
            let found = &mut self.objects[object as usize];
            if flags == PLAIN && found.flags & IS_PROTO == 0 && matches!(found.kind, Kind::Plain) {
                found.slots_mut()[slot as usize] = value;
                return Ok(());
            }
            self.define(object, key, value, PLAIN);
            return Ok(());
        }
        self.check_lazy(object, key);
        if self.objects[object as usize].shape != shape {
            self.define(object, key, value, PLAIN);
            return Ok(());
        }
        self.add_property(object, key, value, PLAIN);
        let new_shape = self.objects[object as usize].shape;
        if !self.shapes.is_dict(new_shape) {
            self.learn(cache, shape, new_shape, 0, false);
        }
        Ok(())
    }

    // By a value that could be anything.

    /// The array index a value means, if it means one.
    #[inline]
    fn index_from_value(&mut self, key: Value) -> Option<usize> {
        if key.is_int() {
            return usize::try_from(key.as_int()).ok();
        }
        if key.is_double() {
            let number = key.as_double();
            if number >= 0.0 && number.fract() == 0.0 && number < 4_294_967_295.0 {
                return Some(number as usize);
            }
            return None;
        }
        if key.is_string() {
            let handle = key.handle();
            let atom = self.atoms.intern_string(&mut self.strings, handle);
            return self.atoms.index_of(atom).map(|index| index as usize);
        }
        None
    }

    #[inline(never)]
    pub(crate) fn get_by_value(&mut self, target: Value, key: Value) -> Run<Value> {
        if key.is_int() && target.is_object() {
            // A negative index turns into a huge one, which a typed array says is nothing, as it should.
            if let Some(value) = self.typed_get_quick(target.handle(), key.as_int() as u32 as usize)
            {
                return Ok(value);
            }
        }
        if let Some(index) = self.index_from_value(key) {
            if target.is_object() {
                if let Kind::Array(items) | Kind::Arguments(items) =
                    &self.objects[target.index()].kind
                {
                    if let Some(value) = items.get(index) {
                        if !value.is_empty() {
                            return Ok(*value);
                        }
                    }
                }
                if matches!(self.objects[target.index()].kind, Kind::TypedArray(_)) {
                    return Ok(self.typed_get(target, index).unwrap_or(Value::UNDEFINED));
                }
            } else if target.is_string() {
                return Ok(self.char_at(target, index));
            }
        }
        if target.is_nullish() {
            let shown = if target.is_null() {
                "null"
            } else {
                "undefined"
            };
            let key = self.brief(key);
            return Err(self.type_error(&format!(
                "Cannot read properties of {shown} (reading {key})"
            )));
        }
        let atom = self.to_atom(key)?;
        self.get_by_atom(target, atom)
    }

    #[inline(never)]
    pub(crate) fn set_by_value(&mut self, target: Value, key: Value, value: Value) -> Run<()> {
        if key.is_int()
            && key.as_int() >= 0
            && target.is_object()
            && self.typed_set_quick(target.handle(), key.as_int() as usize, value)
        {
            return Ok(());
        }
        if target.is_object()
            && matches!(
                self.objects[target.index()].kind,
                Kind::Array(_) | Kind::Arguments(_)
            )
        {
            if let Some(index) = self.index_from_value(key) {
                return self.set_element(target.handle(), index, value);
            }
        }
        if target.is_object() && matches!(self.objects[target.index()].kind, Kind::TypedArray(_)) {
            if let Some(index) = self.index_from_value(key) {
                return self.typed_set(target, index, value);
            }
        }
        if target.is_nullish() {
            let shown = if target.is_null() {
                "null"
            } else {
                "undefined"
            };
            let key = self.brief(key);
            return Err(
                self.type_error(&format!("Cannot set properties of {shown} (setting {key})"))
            );
        }
        let atom = self.to_atom(key)?;
        self.set(target, atom, value)
    }

    /// A string of one character. Those that are ASCII are all the same few strings.
    pub(crate) fn char_at(&mut self, string: Value, index: usize) -> Value {
        let text = &self.strings.items[string.index()].text;
        if index >= text.len() {
            return Value::UNDEFINED;
        }
        let unit = text.at(index);
        self.char_string(unit)
    }

    /// The string of one character. Those in ASCII are only ever made once.
    pub(crate) fn char_string(&mut self, unit: u16) -> Value {
        if let Some(known) = self.char_strings.get(unit as usize) {
            if !known.is_undefined() {
                return *known;
            }
        }
        let atom = self.intern_units(&[unit]);
        let value = self.atom_value(atom);
        if unit < 128 {
            if self.char_strings.is_empty() {
                self.char_strings.resize(128, Value::UNDEFINED);
            }
            self.char_strings[unit as usize] = value;
        }
        value
    }

    // Deleting.

    fn compact_delete_eligible(&self, object: u32, shape: u32) -> bool {
        let found = &self.objects[object as usize];
        compact_delete_enabled()
            && matches!(found.kind, Kind::Plain)
            && found.flags & IS_PROTO == 0
            && !self.shapes.is_dict(shape)
            && self.shapes.items[shape as usize].count <= 32
            && self.shapes.is_all_plain(shape)
    }

    fn compact_delete_from_shape(&mut self, object: u32, shape: u32, key: Atom) {
        let entries = self.shapes.entries(shape);
        let mut new_shape = self.shapes.root(self.shapes.items[shape as usize].proto);
        let mut slots = Vec::with_capacity(entries.len().saturating_sub(1));
        for (found_key, found_slot, found_flags) in entries {
            if found_key != key {
                new_shape = self.shapes.add(new_shape, found_key, found_flags);
                slots.push(self.objects[object as usize].slots()[found_slot as usize]);
            }
        }
        let found = &mut self.objects[object as usize];
        found.shape = new_shape;
        found.set_slots(&slots);
    }

    pub(crate) fn delete_by_value(&mut self, target: Value, key: Value) -> Run<bool> {
        if target.is_nullish() {
            let shown = if target.is_null() {
                "null"
            } else {
                "undefined"
            };
            return Err(self.type_error(&format!("Cannot convert {shown} to object")));
        }
        if !target.is_object() {
            return Ok(true);
        }
        let atom = self.to_atom(key)?;
        self.delete(target.handle(), atom)
    }

    /// Takes a property out of the shape, once it's known it can go.
    fn delete_from_shape(&mut self, object: u32, key: Atom) -> Run<bool> {
        let shape = self.objects[object as usize].shape;
        let Some((slot, _)) = self.shapes.lookup(shape, key) else {
            return Ok(true);
        };
        if self.compact_delete_eligible(object, shape) {
            self.compact_delete_from_shape(object, shape, key);
            return Ok(true);
        }
        let dict = if self.shapes.is_dict(shape) {
            shape
        } else {
            let dict = self.shapes.to_dict(shape);
            self.objects[object as usize].shape = dict;
            dict
        };
        self.shapes.dict_remove(dict, key);
        let found = &mut self.objects[object as usize];
        found.slots_mut()[slot as usize] = Value::UNDEFINED;
        Ok(true)
    }

    pub(crate) fn delete(&mut self, object: u32, key: Atom) -> Run<bool> {
        if matches!(self.objects[object as usize].kind, Kind::Proxy(_)) {
            return self.proxy_delete(object, key);
        }
        self.check_lazy(object, key);
        let found = &mut self.objects[object as usize];
        if let Kind::Array(items) | Kind::Arguments(items) = &mut found.kind {
            if key == text::LENGTH {
                self.write_refused = true;
                return Err(self.type_error("Cannot delete property 'length'"));
            }
            if let Some(index) = self.atoms.index_of(key) {
                if found.flags & FROZEN_ELEMENTS != 0 {
                    self.write_refused = true;
                    return Err(self.type_error("Cannot delete an element of a frozen array"));
                }
                if let Some(slot) = items.get_mut(index as usize) {
                    if !slot.is_empty() || found.flags & SPECIAL_ELEMENTS == 0 {
                        *slot = Value::EMPTY;
                        return Ok(true);
                    }
                } else {
                    return Ok(true);
                }
            }
        }
        let shape = found.shape;
        let Some((slot, flags)) = self.shapes.lookup(shape, key) else {
            return Ok(true);
        };
        if flags & CONFIGURABLE == 0 {
            self.write_refused = true;
            let key = self.atom_text(key);
            return Err(self.type_error(&format!("Cannot delete property '{key}' of object")));
        }
        let found = &mut self.objects[object as usize];
        if found.flags & IS_PROTO != 0 {
            self.epoch = self.epoch.wrapping_add(1);
            #[cfg(feature = "counts")]
            crate::counts::epoch("a prototype's property deleted", key.0);
        }
        if found.flags & NEVER_DICT != 0 || self.compact_delete_eligible(object, shape) {
            // Only the global object is this way, and it keeps a shape it can share with the caches.
            // Eligible ordinary bags use the same exact rebuilding path.
            self.compact_delete_from_shape(object, shape, key);
            return Ok(true);
        }
        let dict = if self.shapes.is_dict(shape) {
            shape
        } else {
            let dict = self.shapes.to_dict(shape);
            self.objects[object as usize].shape = dict;
            dict
        };
        self.shapes.dict_remove(dict, key);
        let found = &mut self.objects[object as usize];
        found.slots_mut()[slot as usize] = Value::UNDEFINED;
        self.shapes.dict_tidy(dict, found);
        Ok(true)
    }

    // Listing.

    /// The object's own property names with their flags, in the order the language says: indexes, then names,
    /// then symbols.
    pub(crate) fn own_keys(&mut self, object: u32) -> Vec<(Atom, u8)> {
        if matches!(self.objects[object as usize].kind, Kind::Proxy(_)) {
            return match self.proxy_own_keys(object) {
                Ok(keys) => keys,
                Err(error) => {
                    self.deferred_error = Some(error);
                    Vec::new()
                }
            };
        }
        self.materialize_if_lazy(object);
        let mut out = Vec::new();
        let count = match &self.objects[object as usize].kind {
            Kind::Array(items) | Kind::Arguments(items) => {
                let holes: Vec<bool> = items.iter().map(|item| item.is_empty()).collect();
                let flags = if self.objects[object as usize].flags & FROZEN_ELEMENTS != 0 {
                    ENUMERABLE
                } else {
                    PLAIN
                };
                for (index, hole) in holes.iter().enumerate() {
                    if !hole {
                        out.push((self.index_atom(index), flags));
                    }
                }
                0
            }
            Kind::Boxed(value) if value.is_string() => self.strings.items[value.index()].text.len(),
            Kind::TypedArray(data) => self.typed_length(data),
            _ => 0,
        };
        for index in 0..count {
            out.push((self.index_atom(index), ENUMERABLE));
        }
        let shape = self.objects[object as usize].shape;
        let entries = self.shapes.entries(shape);
        let mut indexes = Vec::new();
        let mut names = Vec::with_capacity(entries.len());
        let mut symbols = Vec::new();
        for (key, _, flags) in entries {
            match self.atoms.kind_of(key) {
                AtomKind::Hidden => {}
                AtomKind::Symbol => symbols.push((key, flags)),
                AtomKind::Name => match self.atoms.index_of(key) {
                    Some(index) => indexes.push((index, key, flags)),
                    None => names.push((key, flags)),
                },
            }
        }
        indexes.sort_by_key(|entry| entry.0);
        if out.is_empty() {
            out.extend(indexes.into_iter().map(|(_, key, flags)| (key, flags)));
        } else {
            // What's in the array and what's in the shape go together, in order.
            out.extend(indexes.into_iter().map(|(_, key, flags)| (key, flags)));
            out.sort_by_key(|(key, _)| self.atoms.index_of(*key));
        }
        if matches!(
            self.objects[object as usize].kind,
            Kind::Array(_) | Kind::Arguments(_)
        ) || matches!(self.objects[object as usize].kind, Kind::Boxed(value) if value.is_string())
        {
            out.push((text::LENGTH, WRITABLE));
        }
        out.extend(names);
        out.extend(symbols);
        out
    }

    /// The same as `own_keys`, for when an error from a proxy's handler should come out.
    pub(crate) fn own_keys_checked(&mut self, object: u32) -> Run<Vec<(Atom, u8)>> {
        let keys = self.own_keys(object);
        match self.deferred_error.take() {
            Some(error) => Err(error),
            None => Ok(keys),
        }
    }

    /// The names of own enumerable string properties, as `Object.keys` wants them.
    pub(crate) fn own_enumerable_names(&mut self, object: u32) -> Vec<Atom> {
        self.own_keys(object)
            .into_iter()
            .filter(|(key, flags)| {
                flags & ENUMERABLE != 0 && self.atoms.kind_of(*key) == AtomKind::Name
            })
            .map(|(key, _)| key)
            .collect()
    }

    /// The enumerable names of an ordinary object, as a list made the quick way, if this is that sort of object. With
    /// `whole_chain`, it has to have nothing that a prototype would add to the list.
    pub(crate) fn plain_object_keys(
        &mut self,
        object: u32,
        whole_chain: bool,
    ) -> Option<Vec<Value>> {
        let found = &self.objects[object as usize];
        if !matches!(found.kind, Kind::Plain) || found.flags & LAZY_FUNCTION != 0 {
            return None;
        }
        let shape = found.shape;
        if self.shapes.is_dict(shape) {
            return None;
        }
        if whole_chain {
            let mut at = self.shapes.items[shape as usize].proto;
            while at != NONE {
                let parent = &self.objects[at as usize];
                if self.shapes.has_enumerable(parent.shape)
                    || matches!(
                        parent.kind,
                        Kind::Array(_)
                            | Kind::Arguments(_)
                            | Kind::Boxed(_)
                            | Kind::Proxy(_)
                            | Kind::TypedArray(_)
                    )
                {
                    return None;
                }
                at = self.shapes.items[parent.shape as usize].proto;
            }
        }
        let way = shape as usize % self.shape_keys.len();
        if self.shape_keys[way].0 == shape {
            return Some(self.shape_keys[way].1.to_vec());
        }
        let mut keys = Vec::new();
        for (key, _, flags) in self.shapes.entries(shape) {
            // Numbers go first, and in order, and have no strings kept for them.
            if self.atoms.index_of(key).is_some() {
                return None;
            }
            if flags & ENUMERABLE != 0 && self.atoms.kind_of(key) == AtomKind::Name {
                keys.push(self.atom_value(key));
            }
        }
        self.shape_keys[way] = (shape, keys.as_slice().into());
        Some(keys)
    }

    /// Everything `for in` goes through: enumerable names here and on prototypes, once each.
    #[inline(never)]
    pub(crate) fn for_in_keys(&mut self, source: Value) -> Run<Value> {
        let mut out = Vec::new();
        if source.is_nullish() {
            return Ok(self.new_array(out));
        }
        let object = self.to_object(source)?;
        if let Some(keys) = self.plain_object_keys(object.handle(), true) {
            return Ok(self.new_array(keys));
        }
        let mut seen: Vec<Atom> = Vec::new();
        let mut at = object.handle();
        loop {
            let found = &self.objects[at as usize];
            // Prototypes are nearly always only methods, which there's no need to go through and list.
            let has_elements = matches!(
                found.kind,
                Kind::Array(_)
                    | Kind::Arguments(_)
                    | Kind::Boxed(_)
                    | Kind::Proxy(_)
                    | Kind::TypedArray(_)
            );
            if at == object.handle() || has_elements || self.shapes.has_enumerable(found.shape) {
                let is_first = at == object.handle();
                for (key, flags) in self.own_keys(at) {
                    if self.atoms.kind_of(key) == AtomKind::Name
                        && (is_first || !seen.contains(&key))
                    {
                        if flags & ENUMERABLE != 0 {
                            out.push(self.atom_value(key));
                        }
                        seen.push(key);
                    }
                }
            }
            let proto = self.shapes.items[self.objects[at as usize].shape as usize].proto;
            if proto == NONE {
                break;
            }
            at = proto;
        }
        Ok(self.new_array(out))
    }

    pub(crate) fn keys_to_atoms(&mut self, keys: Value) -> Run<Vec<Atom>> {
        let count = match &self.objects[keys.index()].kind {
            Kind::Array(items) => items.len(),
            _ => 0,
        };
        let mut out = Vec::with_capacity(count);
        for at in 0..count {
            let item = match &self.objects[keys.index()].kind {
                Kind::Array(items) => items.get(at).copied().unwrap_or(Value::UNDEFINED),
                _ => Value::UNDEFINED,
            };
            out.push(self.to_atom(item)?);
        }
        Ok(out)
    }

    /// Copies own enumerable properties, as `{...source}` does.
    pub(crate) fn copy_properties(
        &mut self,
        target: Value,
        source: Value,
        excluded: &[Atom],
    ) -> Run<()> {
        if source.is_nullish() {
            return Ok(());
        }
        // An empty object taking everything from a plain one, which is what `{...props}` mostly is.
        if source.is_object() && excluded.is_empty() {
            let from = &self.objects[source.index()];
            let to = &self.objects[target.index()];
            if matches!(from.kind, Kind::Plain)
                && to.slots().is_empty()
                && matches!(to.kind, Kind::Plain)
                && to.flags & IS_PROTO == 0
                && self.shapes.is_all_plain(from.shape)
                && self.shapes.items[from.shape as usize].proto
                    == self.shapes.items[to.shape as usize].proto
            {
                let shape = from.shape;
                let count = from.slots().len();
                if count <= 4 {
                    let mut values = [Value::UNDEFINED; 4];
                    values[..count].copy_from_slice(from.slots());
                    let to = &mut self.objects[target.index()];
                    to.shape = shape;
                    to.set_slots(&values[..count]);
                } else {
                    let values: Box<[Value]> = from.slots().into();
                    self.debt += count * 8;
                    let to = &mut self.objects[target.index()];
                    to.shape = shape;
                    to.set_slots_owned(values);
                }
                return Ok(());
            }
        }
        if source.is_object() && self.copy_plain_properties(target, source, excluded) {
            return Ok(());
        }
        let source = self.to_object(source)?;
        let handle = source.handle();
        for (key, flags) in self.own_keys(handle) {
            if flags & ENUMERABLE == 0 || excluded.contains(&key) {
                continue;
            }
            let value = self.get(source, key)?;
            self.define(target.handle(), key, value, PLAIN);
        }
        Ok(())
    }

    /// Copies from an ordinary object whose properties are all plain names, as most are, without listing its keys the
    /// long way or reading them one at a time. It says whether it could.
    fn copy_plain_properties(&mut self, target: Value, source: Value, excluded: &[Atom]) -> bool {
        let from = &self.objects[source.index()];
        if !matches!(from.kind, Kind::Plain)
            || from.flags & LAZY_FUNCTION != 0
            || self.shapes.is_dict(from.shape)
            || !self.shapes.is_all_plain(from.shape)
        {
            return false;
        }
        // The list is made in one kept for the purpose, as `Object.assign` does.
        let mut entries = std::mem::take(&mut self.assign_entries);
        self.shapes.entries_into(from.shape, &mut entries);
        // Numbers come first and symbols last, which isn't the order they were added in.
        if entries.iter().any(|(key, _, _)| {
            self.atoms.kind_of(*key) != AtomKind::Name || self.atoms.index_of(*key).is_some()
        }) {
            entries.clear();
            self.assign_entries = entries;
            return false;
        }
        // Into a plain object, which is what `{...a}` and `{...rest}` make, a property is added or replaced as
        // `define` would, without the checks it needs for other kinds.
        let to = &self.objects[target.index()];
        let plain_target = matches!(to.kind, Kind::Plain)
            && to.flags & (IS_PROTO | NOT_EXTENSIBLE | LAZY_FUNCTION | NEVER_DICT) == 0;
        for &(key, slot, _) in &entries {
            if excluded.contains(&key) {
                continue;
            }
            let value = self.objects[source.index()].slots()[slot as usize];
            if plain_target {
                let shape = self.objects[target.index()].shape;
                if !self.shapes.is_dict(shape) {
                    match self.shapes.lookup(shape, key) {
                        Some((at, PLAIN)) => {
                            self.objects[target.index()].slots_mut()[at as usize] = value;
                            continue;
                        }
                        None if !self.shapes.wants_dict(shape) => {
                            let next = self.shapes.add(shape, key, PLAIN);
                            let into = &mut self.objects[target.index()];
                            into.shape = next;
                            into.push_slot(value);
                            self.debt += 8;
                            continue;
                        }
                        _ => {}
                    }
                }
            }
            self.define(target.handle(), key, value, PLAIN);
        }
        entries.clear();
        if entries.capacity() <= 64 {
            self.assign_entries = entries;
        }
        true
    }

    /// A copy of the elements of an array, with holes as `undefined`.
    pub(crate) fn array_items(&self, array: Value) -> Vec<Value> {
        match &self.objects[array.index()].kind {
            Kind::Array(items) | Kind::Arguments(items) => items
                .iter()
                .map(|item| {
                    if item.is_empty() {
                        Value::UNDEFINED
                    } else {
                        *item
                    }
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn proto_of(&self, object: u32) -> u32 {
        self.shapes.items[self.objects[object as usize].shape as usize].proto
    }

    /// Changes a prototype as the language allows, and says if it was allowed.
    pub(crate) fn change_proto(&mut self, object: u32, proto: u32) -> bool {
        if self.proto_of(object) == proto {
            return true;
        }
        // `Object.prototype` can't get a prototype, and an object that's closed to additions is closed to this.
        if object == self.realm.object_proto
            || self.objects[object as usize].flags & NOT_EXTENSIBLE != 0
        {
            return false;
        }
        let mut at = proto;
        while at != NONE {
            if at == object {
                return false;
            }
            // A proxy has its own idea of what comes next, so the search stops there.
            if matches!(self.objects[at as usize].kind, Kind::Proxy(_)) {
                break;
            }
            at = self.proto_of(at);
        }
        self.set_proto(object, proto);
        true
    }

    pub(crate) fn set_proto(&mut self, object: u32, proto: u32) {
        if proto != NONE {
            self.objects[proto as usize].flags |= IS_PROTO;
        }
        let found = &mut self.objects[object as usize];
        if found.flags & IS_PROTO != 0 {
            self.epoch = self.epoch.wrapping_add(1);
            #[cfg(feature = "counts")]
            crate::counts::epoch("a prototype's prototype set", u32::MAX);
        }
        found.shape = self.shapes.with_proto(found.shape, proto);
        if self.shapes.is_dict(found.shape) {
            self.epoch = self.epoch.wrapping_add(1);
            #[cfg(feature = "counts")]
            crate::counts::epoch("an object whose prototype was set became a dictionary", u32::MAX);
        }
    }

    // Classes.

    pub(crate) fn new_class(&mut self, constructor: Value, parent: Value) -> Run<Value> {
        let handle = constructor.handle();
        let proto_parent = if parent.is_empty() {
            self.realm.object_proto
        } else if parent.is_null() {
            NONE
        } else {
            if !self.is_constructor(parent) {
                let shown = self.brief(parent);
                return Err(self.type_error(&format!(
                    "Class extends value {shown} is not a constructor or null"
                )));
            }
            let parent_proto = self.get(parent, text::PROTOTYPE)?;
            if parent_proto.is_object() {
                parent_proto.handle()
            } else if parent_proto.is_null() {
                NONE
            } else {
                return Err(
                    self.type_error("Class extends value does not have valid prototype property")
                );
            }
        };
        if parent.is_object() {
            self.set_proto(handle, parent.handle());
        }
        let shape = self.root_shape(proto_parent);
        let prototype = self.alloc_object(shape, Kind::Plain, 2);
        self.add_property(prototype.handle(), text::CONSTRUCTOR, constructor, HIDDEN);
        self.materialize_if_lazy(handle);
        self.define(handle, text::PROTOTYPE, prototype, 0);
        if let Kind::Function { home, .. } = &mut self.objects[handle as usize].kind {
            *home = prototype.handle();
        }
        Ok(prototype)
    }

    /// Gives a new instance of a class its private methods, and says what sets up its fields, if it has any.
    fn private_methods_and_fields(&mut self, closure: u32, this: Value) -> Run<Option<Value>> {
        let shape = self.objects[closure as usize].shape;
        // Private methods come first, for fields to be able to use them.
        if let Some((slot, _)) = self.shapes.lookup(shape, text::PRIVATE_METHODS) {
            let methods = self.objects[closure as usize].slots()[slot as usize];
            let methods_shape = self.objects[methods.index()].shape;
            for (key, slot, flags) in self.shapes.entries(methods_shape) {
                let value = self.objects[methods.index()].slots()[slot as usize];
                self.add_private(this.handle(), key, value, flags)?;
            }
        }
        Ok(self
            .shapes
            .lookup(shape, text::FIELDS)
            .map(|(slot, _)| self.objects[closure as usize].slots()[slot as usize]))
    }

    /// Sets up a new instance's private methods and fields, for compiled code.
    #[cfg(feature = "jit")]
    pub(crate) fn run_fields(&mut self, closure: u32, this: Value) -> Run<()> {
        if let Some(fields) = self.private_methods_and_fields(closure, this)? {
            self.call(fields, this, &[])?;
        }
        Ok(())
    }

    /// [`Vm::run_fields`] for RunFields in the interpreter: what sets up the fields, which is written in JavaScript, is
    /// called in place, as a call is, and fails with [`Value::ENTERED`]. It gives back the object, which goes to
    /// register `this_reg`, where the object already is.
    #[inline(never)]
    pub(crate) fn run_fields_here(&mut self, closure: u32, this: Value, this_reg: u16) -> Run<()> {
        if let Some(fields) = self.private_methods_and_fields(closure, this)? {
            if let Kind::Function { proto, .. } = self.objects[fields.index()].kind {
                let base = self.top;
                self.reserve_stack(base + 1 + 16)?;
                self.stack[base] = this;
                self.enter_function(fields.handle(), proto, base, 0, this_reg, 0, Value::UNDEFINED)?;
                return Err(Value::ENTERED);
            }
            self.call(fields, this, &[])?;
        }
        Ok(())
    }

    /// SuperCallSpread in the interpreter: `super(...args)` of a parent constructor written in JavaScript makes its
    /// frame in place, with the array's items as its arguments after `this` at `f + 1`, answering to `dst`, and fails
    /// with [`Value::ENTERED`], as SuperCall does. Anything else is constructed from here.
    #[inline(never)]
    pub(crate) fn super_call_spread_here(&mut self, f: u16, array: Value, dst: u16) -> Run<Value> {
        let frame = *self.frames.last().expect("there's a frame running");
        let (parent, new_target) = self.super_constructor(frame.closure, frame.new_target)?;
        let at = frame.base as usize + f as usize;
        self.stack[at] = parent;
        let args = self.array_items(array);
        if let Kind::Function { proto, .. } = self.objects[parent.index()].kind {
            let flags = self.protos[proto as usize].flags;
            if flags & crate::op::IS_CONSTRUCTOR != 0 {
                let this_value = if flags & crate::op::IS_DERIVED != 0 {
                    Value::EMPTY
                } else {
                    self.create_this(new_target)?
                };
                let start = at + 1;
                self.reserve_stack(start + 1 + args.len() + 16)?;
                self.stack[start] = this_value;
                self.stack[start + 1..start + 1 + args.len()].copy_from_slice(&args);
                self.enter_function(
                    parent.handle(),
                    proto,
                    start,
                    args.len(),
                    dst,
                    crate::vm::CONSTRUCTING,
                    new_target,
                )?;
                return Err(Value::ENTERED);
            }
        }
        self.construct_with_target(parent, &args, new_target)
    }

    fn home_of(&self, closure: u32) -> u32 {
        match &self.objects[closure as usize].kind {
            Kind::Function { home, .. } => *home,
            _ => NONE,
        }
    }

    pub(crate) fn super_get(&mut self, closure: u32, key: Value, this: Value) -> Run<Value> {
        let home = self.home_of(closure);
        let atom = self.to_atom(key)?;
        if home == NONE {
            return Err(self.syntax_error("'super' keyword unexpected here"));
        }
        let parent = self.proto_of(home);
        if parent == NONE {
            return Ok(Value::UNDEFINED);
        }
        self.get_from(parent, atom, this)
    }

    /// Which constructor `super()` means here, and what `new.target` is for it.
    pub(crate) fn super_constructor(
        &mut self,
        closure: u32,
        new_target: Value,
    ) -> Run<(Value, Value)> {
        let is_arrow = match &self.objects[closure as usize].kind {
            Kind::Function { proto, .. } => {
                self.protos[*proto as usize].flags & crate::op::IS_ARROW != 0
            }
            _ => false,
        };
        // An arrow's way to its constructor is through what it inherited from it.
        let (constructor, new_target) = if is_arrow {
            let home = self.home_of(closure);
            if home == NONE {
                return Err(self.syntax_error("'super' keyword unexpected here"));
            }
            let constructor = self.get(Value::object(home), text::CONSTRUCTOR)?;
            (constructor.handle(), constructor)
        } else {
            (closure, new_target)
        };
        let parent = self.proto_of(constructor);
        let parent_value = Value::object(parent);
        if parent == NONE || !self.is_constructor(parent_value) {
            return Err(self.type_error("Super constructor is not a constructor"));
        }
        Ok((parent_value, new_target))
    }

    pub(crate) fn construct_with_target(
        &mut self,
        function: Value,
        args: &[Value],
        new_target: Value,
    ) -> Run<Value> {
        let start = self.top;
        let needed = start + args.len() + 2;
        self.reserve_stack(needed + 16)?;
        self.stack[start] = function;
        self.stack[start + 1] = Value::UNDEFINED;
        self.stack[start + 2..needed].copy_from_slice(args);
        self.top = needed;
        let result = self.call_at(start, args.len(), Some(new_target));
        self.top = start;
        if let Ok(value) = result {
            self.nursery.push(value);
        }
        result
    }

    // Iterating.

    #[inline(never)]
    pub(crate) fn get_iterator(&mut self, source: Value) -> Run<(Value, Value)> {
        if source.is_object() {
            let handle = source.handle();
            let found = &self.objects[handle as usize];
            if found.shape == self.realm.array_shape && matches!(found.kind, Kind::Array(_)) {
                let proto = &self.objects[self.realm.array_proto as usize];
                let shape = proto.shape;
                let slot = if self.array_iterator_slot.0 == shape {
                    Some(self.array_iterator_slot.1)
                } else {
                    let found = self.shapes.lookup(shape, text::SYMBOL_ITERATOR);
                    if let Some((slot, _)) = found {
                        self.array_iterator_slot = (shape, slot);
                    }
                    found.map(|(slot, _)| slot)
                };
                if let Some(slot) = slot {
                    let method =
                        self.objects[self.realm.array_proto as usize].slots()[slot as usize];
                    if method == self.realm.array_values.0 {
                        return Ok((source, Value::int(0)));
                    }
                }
            }
            if matches!(found.kind, Kind::Arguments(_)) {
                let iterator = self.new_native_iter(IterKind::ArrayValues, source);
                return Ok((iterator, Value::UNDEFINED));
            }
        }
        if source.is_nullish() {
            let shown = self.brief(source);
            return Err(self.type_error(&format!("{shown} is not iterable")));
        }
        let method = self.get(source, text::SYMBOL_ITERATOR)?;
        if !self.is_callable(method) {
            let shown = self.brief(source);
            return Err(self.type_error(&format!("{shown} is not iterable")));
        }
        let iterator = self.call(method, source, &[])?;
        if !iterator.is_object() {
            return Err(self.type_error("Result of the Symbol.iterator method is not an object"));
        }
        if matches!(self.objects[iterator.index()].kind, Kind::Iter(_)) {
            return Ok((iterator, Value::UNDEFINED));
        }
        let next = self.get(iterator, text::NEXT)?;
        Ok((iterator, next))
    }

    pub(crate) fn new_native_iter(&mut self, kind: IterKind, target: Value) -> Value {
        let proto = self.realm.native_iterator_proto;
        self.new_object_with_proto(
            proto,
            Kind::Iter(Box::new(IterData {
                kind,
                target,
                at: 0,
            })),
        )
    }

    /// The next value of an iterator, or nothing when it's finished.
    #[inline(never)]
    pub(crate) fn iterator_step(&mut self, iterator: Value, next: Value) -> Run<Option<Value>> {
        if matches!(self.objects[iterator.index()].kind, Kind::Iter(_)) {
            return self.native_iter_step(iterator.handle());
        }
        if let Some(step) = self.generator_step_directly(iterator, next) {
            return step;
        }
        let result = self.call(next, iterator, &[])?;
        if !result.is_object() {
            return Err(self.type_error("Iterator result is not an object"));
        }
        let done = self.get(result, text::DONE)?;
        if self.to_boolean(done) {
            return Ok(None);
        }
        Ok(Some(self.get(result, text::VALUE)?))
    }

    pub(crate) fn iterator_close(&mut self, iterator: Value) -> Run<()> {
        if !iterator.is_object() || matches!(self.objects[iterator.index()].kind, Kind::Iter(_)) {
            return Ok(());
        }
        let method = self.get(iterator, text::RETURN)?;
        if !method.is_nullish() {
            self.call(method, iterator, &[])?;
        }
        Ok(())
    }

    /// Goes through an iterable one at a time. If the visitor has an error, the iterator is told to close.
    pub(crate) fn iterate(
        &mut self,
        source: Value,
        visitor: impl FnMut(&mut Self, Value) -> Run<()>,
    ) -> Run<()> {
        let (first, second) = self.get_iterator(source)?;
        self.iterate_from(first, second, visitor)
    }

    /// Continues an iterator already obtained from a source, without reading its iterator getter a second time.
    pub(crate) fn iterate_from(
        &mut self,
        first: Value,
        second: Value,
        mut visitor: impl FnMut(&mut Self, Value) -> Run<()>,
    ) -> Run<()> {
        if second.is_int() {
            let mut at = 0;
            loop {
                let item = match &self.objects[first.index()].kind {
                    Kind::Array(items) if at < items.len() => items[at],
                    _ => return Ok(()),
                };
                at += 1;
                let item = if item.is_empty() {
                    Value::UNDEFINED
                } else {
                    item
                };
                visitor(self, item)?;
            }
        }
        while let Some(item) = self.iterator_step(first, second)? {
            self.nursery.push(item);
            if let Err(error) = visitor(self, item) {
                let _ = self.iterator_close(first);
                return Err(error);
            }
        }
        Ok(())
    }

    /// Goes through everything an iterator has, and collects it.
    pub(crate) fn iterate_to_vec(&mut self, source: Value) -> Run<Vec<Value>> {
        let (first, second) = self.get_iterator(source)?;
        if second.is_int() {
            return Ok(self.array_items(first));
        }
        let mut out = Vec::new();
        while let Some(item) = self.iterator_step(first, second)? {
            // What's been collected isn't anywhere else, and the next step can run code.
            self.nursery.push(item);
            out.push(item);
        }
        Ok(out)
    }

    /// `...source` in an array literal. It's kept out of the interpreter's loop, which only calls it.
    #[inline(never)]
    pub(crate) fn spread_into(&mut self, array: Value, source: Value) -> Run<()> {
        // An array, as most are: its elements go straight across, a few at a time, with no list made of them.
        let (first, second) = self.get_iterator(source)?;
        if second.is_int() && first != array {
            let count = match &self.objects[first.index()].kind {
                Kind::Array(items) => items.len(),
                _ => 0,
            };
            self.debt += count * 8;
            let mut at = 0;
            while at < count {
                let mut few = [Value::UNDEFINED; 8];
                let taken = (count - at).min(few.len());
                if let Kind::Array(items) = &self.objects[first.index()].kind {
                    for (place, item) in few.iter_mut().zip(&items[at..at + taken]) {
                        *place = if item.is_empty() {
                            Value::UNDEFINED
                        } else {
                            *item
                        };
                    }
                }
                if let Kind::Array(target) = &mut self.objects[array.index()].kind {
                    for value in &few[..taken] {
                        target.push(*value);
                    }
                }
                at += taken;
            }
            return Ok(());
        }
        let mut items = if second.is_int() {
            self.array_items(first)
        } else {
            let mut out = Vec::new();
            while let Some(item) = self.iterator_step(first, second)? {
                // What's been collected isn't anywhere else, and the next step can run code.
                self.nursery.push(item);
                out.push(item);
            }
            out
        };
        self.debt += items.len() * 8;
        if let Kind::Array(target) = &mut self.objects[array.index()].kind {
            target.vec().append(&mut items);
        }
        Ok(())
    }

    pub(crate) fn native_iter_step(&mut self, iterator: u32) -> Run<Option<Value>> {
        let Kind::Iter(data) = &self.objects[iterator as usize].kind else {
            unreachable!("only native iterators come here");
        };
        let (kind, target, at) = (data.kind, data.target, data.at as usize);
        let mut next_at = at + 1;
        let result = match kind {
            IterKind::ArrayValues | IterKind::ArrayKeys | IterKind::ArrayEntries => {
                let length = self.length_of(target)?;
                if at >= length {
                    None
                } else {
                    Some(match kind {
                        IterKind::ArrayKeys => Value::number(at as f64),
                        IterKind::ArrayValues => self.get_index(target, at)?,
                        _ => {
                            let value = self.get_index(target, at)?;
                            self.new_array(vec![Value::number(at as f64), value])
                        }
                    })
                }
            }
            IterKind::StringValues => {
                let text = &self.strings.items[target.index()].text;
                if at >= text.len() {
                    None
                } else {
                    let unit = text.at(at);
                    if (0xD800..0xDC00).contains(&unit)
                        && at + 1 < text.len()
                        && (0xDC00..0xE000).contains(&text.at(at + 1))
                    {
                        let pair = [unit, text.at(at + 1)];
                        next_at = at + 2;
                        let atom = self.intern_units(&pair);
                        Some(self.atom_value(atom))
                    } else {
                        Some(self.char_string(unit))
                    }
                }
            }
            IterKind::MapEntries
            | IterKind::MapKeys
            | IterKind::MapValues
            | IterKind::SetValues
            | IterKind::SetEntries => {
                let (Kind::Map(data) | Kind::Set(data)) = &self.objects[target.index()].kind else {
                    return Err(self.type_error("This iterator's map has gone away"));
                };
                let mut position = at;
                let mut found = None;
                while position < data.entries.len() {
                    let entry = data.entries[position];
                    position += 1;
                    if !entry.0.is_empty() {
                        found = Some(entry);
                        break;
                    }
                }
                next_at = position;
                found.map(|(key, value)| match kind {
                    IterKind::MapKeys | IterKind::SetValues => key,
                    IterKind::MapValues => value,
                    IterKind::SetEntries => self.new_array(vec![key, key]),
                    _ => self.new_array(vec![key, value]),
                })
            }
        };
        if let Kind::Iter(data) = &mut self.objects[iterator as usize].kind {
            data.at = if result.is_some() {
                next_at as u32
            } else {
                u32::MAX / 2
            };
        }
        Ok(result)
    }

    /// The `length` of anything array-like.
    /// What should make the result of a method of this object: what its constructor's `Symbol.species` says, or if
    /// that's nothing, what's given. This is how a subclass gets its own kind of object back.
    pub(crate) fn species_constructor(&mut self, object: Value, default: Value) -> Run<Value> {
        let constructor = self.get(object, text::CONSTRUCTOR)?;
        self.species_from(constructor, default)
    }

    /// What a `constructor` says should make things, which is what its `Symbol.species` says.
    pub(crate) fn species_from(&mut self, constructor: Value, default: Value) -> Run<Value> {
        if constructor.is_undefined() {
            return Ok(default);
        }
        if !constructor.is_object() {
            return Err(self.type_error("The constructor property is not an object"));
        }
        let species = self.get(constructor, text::SYMBOL_SPECIES)?;
        if species.is_undefined() || species.is_null() {
            return Ok(default);
        }
        if !self.is_constructor(species) {
            return Err(self.type_error("Symbol.species is not a constructor"));
        }
        Ok(species)
    }

    /// An object's `constructor`, for an object that's expected to inherit it from this prototype. Once that's been seen
    /// for a shape, it's a couple of reads from then on, until a prototype changes. `memo` says which kind of object this
    /// is for, as an array and a promise are asked about all the time.
    pub(crate) fn constructor_of(&mut self, object: Value, proto: u32, memo: usize) -> Run<Value> {
        let shape = self.objects[object.index()].shape;
        let known = self.constructor_memos[memo];
        if known.shape == shape && known.epoch == self.epoch {
            return Ok(self.objects[proto as usize].slots()[known.slot as usize]);
        }
        let found = self.get(object, text::CONSTRUCTOR)?;
        if self.shapes.items[shape as usize].proto == proto
            && !self.shapes.is_dict(shape)
            && self.shapes.lookup(shape, text::CONSTRUCTOR).is_none()
        {
            let proto_shape = self.objects[proto as usize].shape;
            if let Some((slot, flags)) = self.shapes.lookup(proto_shape, text::CONSTRUCTOR) {
                if flags & ACCESSOR == 0 {
                    self.constructor_memos[memo] = ConstructorMemo {
                        shape,
                        epoch: self.epoch,
                        slot,
                    };
                }
            }
        }
        Ok(found)
    }

    /// The `length` IncLoopLtLength compares with when it isn't an array's, which it has no place in the code to
    /// remember: an `arguments` object's and a string's are where `get_cached` finds them, and a typed array's is worked
    /// out as `get_cached` does once what reads remember says its getter is still the built-in one. Anything else goes
    /// the long way. Going through `get` each time was 15% of a loop over a typed array.
    #[inline(never)]
    pub(crate) fn loop_length(&mut self, target: Value) -> Run<Value> {
        if let Some(length) = self.quick_loop_length(target) {
            return Ok(length);
        }
        self.get(target, text::LENGTH)
    }

    /// What [`Vm::loop_length`] finds without going the long way, if it can. (Compiled code asks for it too.)
    #[inline]
    pub(crate) fn quick_loop_length(&self, target: Value) -> Option<Value> {
        if target.is_object() {
            let object = &self.objects[target.index()];
            match &object.kind {
                Kind::Arguments(items) => return Some(Value::int_or_double(items.len())),
                Kind::TypedArray(data) => {
                    if let Some(found) = self.remembered(object.shape, text::LENGTH) {
                        if found.slot & TYPED_LENGTH_SLOT != 0 {
                            return Some(Value::int_or_double(self.typed_length_now(data)));
                        }
                    }
                }
                _ => {}
            }
        } else if target.is_string() {
            return Some(Value::int(
                self.strings.items[target.index()].text.len() as i32,
            ));
        }
        None
    }

    pub(crate) fn length_of(&mut self, target: Value) -> Run<usize> {
        if target.is_object() {
            if let Kind::Array(items) | Kind::Arguments(items) = &self.objects[target.index()].kind
            {
                return Ok(items.len());
            }
            if let Kind::TypedArray(data) = &self.objects[target.index()].kind {
                return Ok(self.typed_length(data));
            }
        }
        let length = self.get(target, text::LENGTH)?;
        let number = self.to_number(length)?;
        if number.is_nan() || number <= 0.0 {
            return Ok(0);
        }
        Ok(number.min(9_007_199_254_740_991.0) as usize)
    }

    /// The atom for a position in something array-like.
    pub(crate) fn index_atom(&mut self, index: usize) -> Atom {
        match Atom::from_index(index) {
            Some(atom) => atom,
            None => self.intern_str(&index.to_string()),
        }
    }

    pub(crate) fn get_index(&mut self, target: Value, index: usize) -> Run<Value> {
        if target.is_object() {
            if let Kind::Array(items) | Kind::Arguments(items) = &self.objects[target.index()].kind
            {
                if let Some(value) = items.get(index) {
                    if !value.is_empty() {
                        return Ok(*value);
                    }
                }
            }
        }
        if target.is_string() {
            return Ok(self.char_at(target, index));
        }
        if target.is_object() && matches!(self.objects[target.index()].kind, Kind::TypedArray(_)) {
            return Ok(self.typed_get(target, index).unwrap_or(Value::UNDEFINED));
        }
        let atom = self.index_atom(index);
        self.get(target, atom)
    }

    pub(crate) fn set_index(&mut self, target: Value, index: usize, value: Value) -> Run<()> {
        if target.is_object()
            && matches!(
                self.objects[target.index()].kind,
                Kind::Array(_) | Kind::Arguments(_)
            )
        {
            return self.set_element(target.handle(), index, value);
        }
        if target.is_object() && matches!(self.objects[target.index()].kind, Kind::TypedArray(_)) {
            return self.typed_set(target, index, value);
        }
        let atom = self.index_atom(index);
        self.set(target, atom, value)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::Engine;

    fn getter_program(source: &str) -> (Engine, Rc<RefCell<String>>) {
        let output = Rc::new(RefCell::new(String::new()));
        let sink = output.clone();
        let mut engine = Engine::new();
        #[cfg(all(feature = "jit", windows, target_arch = "x86_64"))]
        { engine.vm.jit.threshold = 2; }
        engine.set_printer(move |text| sink.borrow_mut().push_str(text));
        engine.eval(source, "simple-getters.js").unwrap();
        engine.run_until_idle().unwrap();
        (engine, output)
    }

    fn global_value(engine: &mut Engine, name: &str) -> crate::value::Value {
        let key = engine.vm.intern_str(name);
        engine.vm.get(crate::value::Value::object(engine.vm.global), key).unwrap()
    }

    #[test]
    fn simple_getters_read_live_captures_and_keep_tdz_identity_and_gc_semantics() {
        let (mut engine, output) = getter_program(r#"
            function make(value) { return {get selected(){return value}}; }
            globalThis.original={n:1}; globalThis.immutable=make(original);
            globalThis.immutableGetter=Object.getOwnPropertyDescriptor(immutable,'selected').get;
            function live(){let value={n:2};globalThis.change=v=>value=v;return {get selected(){return value}};}
            globalThis.mutable=live();
            globalThis.mutableGetter=Object.getOwnPropertyDescriptor(mutable,'selected').get;
            for(let i=0;i<10000;i++)if(immutable.selected!==original||mutable.selected.n!==2)throw Error('bad capture');
            function early(){const obj={get selected(){return later}};let before;try{obj.selected}catch(e){before=e.name+' '+e.message.includes('later')}const later={n:3};return before+' '+(obj.selected===later);}
            console.log(early(),immutableGetter===Object.getOwnPropertyDescriptor(immutable,'selected').get);
        "#);
        assert_eq!(output.borrow().trim_end(), "ReferenceError true true true");
        let getter = global_value(&mut engine, "immutableGetter");
        let expected = global_value(&mut engine, "original");
        assert_eq!(engine.vm.simple_getter_value(getter), Some(expected));
        let mutable_getter = global_value(&mut engine, "mutableGetter");
        assert!(engine.vm.simple_getter_value(mutable_getter).is_some());
        engine.eval("globalThis.replacement={n:4};change(replacement);", "change-getter.js").unwrap();
        engine.collect_garbage();
        let replacement = global_value(&mut engine, "replacement");
        assert_eq!(engine.vm.simple_getter_value(mutable_getter), Some(replacement));
        engine.eval("for(let i=0;i<10000;i++)if(mutable.selected!==replacement||immutable.selected!==original)throw Error('changed after GC');", "getter-after-gc.js").unwrap();
    }

    #[test]
    fn simple_getters_preserve_descriptors_redefinition_prototypes_receivers_and_proxy_traps() {
        let (_, output) = getter_program(r#"
            const held={n:7};const proto={get selected(){return held}};const obj=Object.create(proto);
            const first=Object.getOwnPropertyDescriptor(proto,'selected').get;
            function read(o){return o.selected}for(let i=0;i<10000;i++)if(read(obj)!==held)throw Error('identity');
            let d=Object.getOwnPropertyDescriptor(proto,'selected');
            console.log(d.get===first,!('value'in d),d.enumerable,d.configurable,Reflect.get(proto,'selected',{})===held,first.call(null)===held);
            Object.defineProperty(proto,'selected',{get(){return 12}});console.log(read(obj));
            Object.setPrototypeOf(obj,{get selected(){return 14}});console.log(read(obj));
            let traps=0;const proxy=new Proxy(proto,{get(t,k,r){traps++;return Reflect.get(t,k,r)}});
            console.log(read(proxy),read(proxy),traps);
            const own={get selected(){return this.n},n:18};console.log(Reflect.get(own,'selected',{n:19}),read(own));
            Object.defineProperty(obj,'selected',{value:20});console.log(read(obj));
        "#);
        assert_eq!(output.borrow().trim_end(), "true true true true true true\n12\n14\n12 12 2\n19 18\n20");
    }

    #[test]
    fn simple_getters_do_not_skip_effects_parameter_initializers_async_or_generator_wrappers() {
        let (_, output) = getter_program(r#"
            let effects=0;const held={n:8};const o={get selected(){effects++;return held}};
            function read(v){return v.selected}for(let i=0;i<100;i++)read(o);console.log(effects);
            const defaults={};Object.defineProperty(defaults,'selected',{get:function(a=(effects++,1)){return held}});
            console.log(read(defaults)===held,read(defaults)===held,effects);
            const generators={};Object.defineProperty(generators,'selected',{get:function*(){return held}});
            console.log(read(generators).next().value===held);
            const asyncs={};Object.defineProperty(asyncs,'selected',{get:async function(){return held}});
            const promise=read(asyncs);console.log(typeof promise.then);promise.then(v=>console.log(v===held));
            const literals={get a(){return 17},get b(){return null},get c(){return true},get d(){return 'kept'},get e(){}};
            console.log(literals.a,literals.b,literals.c,literals.d,literals.e);
        "#);
        assert_eq!(output.borrow().trim_end(), "100\ntrue true 102\ntrue\nfunction\n17 null true kept undefined\ntrue");
    }

    #[test]
    fn simple_getters_remain_visible_to_breakpoints_and_stepping() {
        // File loading installs debugger instrumentation; Engine::eval deliberately
        // runs uninstrumented scripts and would not exercise getter stepping here.
        let folder = std::env::temp_dir().join(format!("rsui-simple-getter-debug-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("getter-debug.js");
        std::fs::write(&file, "const held=7;\nfunction selectedGetter(){return held;}\nconst o={};Object.defineProperty(o,'value',{get:selectedGetter});o.value;o.value;").unwrap();
        let wanted = file.clone();
        crate::set_debug_files(Some(Rc::new(move |path| path == wanted)));
        let seen = Rc::new(RefCell::new(Vec::new()));
        let kept = seen.clone();
        crate::set_pause_handler(Some(Rc::new(move |_, pause| {
            kept.borrow_mut().push(pause.at.function.clone());
            crate::Resume::Continue
        })));
        crate::set_breakpoints(&[(file.to_string_lossy().into_owned(), 2)]);
        let mut engine = Engine::new();
        let result = engine.run_file(&file);
        crate::set_breakpoints(&[]);
        crate::set_pause_handler(None);
        crate::set_debug_files(None);
        let _ = std::fs::remove_dir_all(&folder);
        result.unwrap();
        assert_eq!(seen.borrow().iter().filter(|name| name.as_str() == "selectedGetter").count(), 2);
    }

    #[test]
    fn owned_spread_slots_keep_order_descriptors_symbols_independence_and_gc_reachability() {
        let (mut engine, output) = getter_program(r#"
            const symbol=Symbol('last');const shared={n:5};
            globalThis.spreadSource={2:'two',first:shared,second:2,third:3,fourth:4,fifth:5,[symbol]:'symbol'};
            globalThis.spreadCopy={...spreadSource};
            console.log(Reflect.ownKeys(spreadCopy).map(k=>typeof k==='symbol'?'symbol':k).join(','));
            console.log(spreadCopy.first===shared,spreadCopy[symbol]);
            const desc=Object.getOwnPropertyDescriptor(spreadCopy,'first');
            console.log(desc.writable,desc.enumerable,desc.configurable,!('get'in desc));
            spreadSource.second=20;spreadCopy.third=30;
            console.log(spreadCopy.second,spreadSource.third);
            globalThis.onlyCopy={...spreadCopy};spreadSource=null;spreadCopy=null;
        "#);
        assert_eq!(output.borrow().trim_end(), "2,first,second,third,fourth,fifth,symbol\ntrue symbol\ntrue true true true\n2 3");
        engine.collect_garbage();
        engine.eval("if(onlyCopy.first.n!==5||onlyCopy.third!==30||onlyCopy.second!==2)throw Error('lost spread values');", "spread-after-gc.js").unwrap();
        let target = engine.vm.new_plain();
        // Keep the target old through a collection before copying young references
        // into it, exercising the ordinary Objects write barrier retained by this path.
        let key = engine.vm.intern_str("oldSpreadTarget");
        engine.vm.set(crate::value::Value::object(engine.vm.global), key, target).unwrap();
        engine.collect_garbage();
        engine.eval("globalThis.youngSpreadSource={first:{n:8},second:2,third:3,fourth:4,fifth:5};", "young-spread.js").unwrap();
        let source = global_value(&mut engine, "youngSpreadSource");
        engine.vm.copy_properties(target, source, &[]).unwrap();
        engine.eval("youngSpreadSource=null;", "drop-young-spread.js").unwrap();
        engine.vm.collect_young_garbage();
        engine.eval("if(oldSpreadTarget.first.n!==8||oldSpreadTarget.third!==3)throw Error('old target lost young values');", "old-spread-after-gc.js").unwrap();
    }

    #[test]
    fn compact_delete_shares_small_plain_shapes_and_preserves_fallbacks() {
        let (mut engine, _) = getter_program(r#"
            globalThis.left={a:1,style:{x:1},b:2};globalThis.right={a:3,style:{x:2},b:4};
            delete left.style;delete right.style;
            globalThis.protoBag={a:1,style:2,b:3};Object.create(protoBag);delete protoBag.style;
            globalThis.accessorBag={a:1,style:2,get b(){return 3}};delete accessorBag.style;
            globalThis.flagsBag={a:1,style:2,b:3};Object.defineProperty(flagsBag,'a',{enumerable:false});delete flagsBag.style;
            globalThis.largeBag={};for(let i=0;i<33;i++)largeBag['k'+i]=i;delete largeBag.k1;
            globalThis.arrayBag=[1,2];arrayBag.style=3;delete arrayBag.style;
        "#);
        let left=global_value(&mut engine,"left");let right=global_value(&mut engine,"right");
        let a=engine.vm.objects[left.index()].shape;let b=engine.vm.objects[right.index()].shape;
        assert_eq!(engine.vm.shapes.is_dict(a),!super::compact_delete_enabled());
        assert_eq!(engine.vm.shapes.is_dict(b),!super::compact_delete_enabled());
        assert_eq!(a==b,super::compact_delete_enabled());
        for name in ["protoBag","accessorBag","flagsBag","largeBag","arrayBag"] {
            let value=global_value(&mut engine,name);
            assert!(engine.vm.shapes.is_dict(engine.vm.objects[value.index()].shape),"{name} must retain dictionary fallback");
        }
    }

    #[test]
    fn compact_delete_preserves_plain_symbols_readdition_descriptors_and_releases_removed_values() {
        let (mut engine, output)=getter_program(r#"
            const sym=Symbol('key');globalThis.small={a:1,drop:{n:5},b:2,[sym]:3};
            globalThis.removedWeak=new WeakRef(small.drop);delete small.drop;delete small[sym];small.drop=4;small[sym]=6;
            const d=Object.getOwnPropertyDescriptor(small,'b');
            console.log(Reflect.ownKeys(small).map(k=>typeof k==='symbol'?k.description:k).join(','),d.value,d.writable,d.enumerable,d.configurable);
            Object.preventExtensions(small);console.log(delete small.b,Object.isExtensible(small),small.a,small.drop,small[sym]);
        "#);
        assert_eq!(output.borrow().trim_end(),"a,b,drop,key 2 true true true\ntrue false 1 4 6");
        engine.vm.clear_kept_objects();engine.collect_garbage();
        let weak=global_value(&mut engine,"removedWeak");
        assert_eq!(engine.vm.data_surface_weak_target(weak),Ok(None),"removed slot must not retain its value");
        engine.eval("if(small.a!==1||small.drop!==4)throw Error('survivor lost')", "compact-after-gc.js").unwrap();
    }

    #[test]
    fn normal_delete_preserves_descriptors_symbols_key_order_and_readdition() {
        let (_, output) = getter_program(r#"
            const first=Symbol('first'),last=Symbol('last');let calls=0;
            const o={2:'two',a:1,b:2,c:3,[first]:4,[last]:5};
            Object.defineProperty(o,'hidden',{value:6,writable:false,enumerable:false,configurable:true});
            Object.defineProperty(o,'seen',{get(){calls++;return 7},enumerable:true,configurable:true});
            Object.defineProperty(o,'locked',{value:8,writable:false,enumerable:true,configurable:false});
            const getter=Object.getOwnPropertyDescriptor(o,'seen').get;
            console.log(delete o.b,delete o[first],delete o.absent,calls);
            o.b=20;o[first]=40;
            console.log(Reflect.ownKeys(o).map(k=>typeof k==='symbol'?k.description:k).join(','));
            const h=Object.getOwnPropertyDescriptor(o,'hidden'),s=Object.getOwnPropertyDescriptor(o,'seen');
            console.log(h.value,h.writable,h.enumerable,h.configurable,s.get===getter,s.enumerable,s.configurable,calls);
            console.log(delete o.locked,Reflect.deleteProperty(o,'locked'),o.locked);
            try{(function(){'use strict';delete o.locked})()}catch(e){console.log(e.name)}
            Object.preventExtensions(o);console.log(delete o.c,Object.isExtensible(o),o.a,o.b,o.seen,calls);
        "#);
        assert_eq!(output.borrow().trim_end(), "true true true 0\n2,a,c,hidden,seen,locked,b,last,first\n6 false false true true true true 0\nfalse false 8\nTypeError\ntrue false 1 20 7 1");
    }

    #[test]
    fn normal_delete_invalidates_cached_reads_and_prototype_presence_without_changing_receivers() {
        let (_, output) = getter_program(r#"
            const proto={gone:9,other:10};const o={a:1,gone:2,b:3,last:4};Object.setPrototypeOf(o,proto);
            function read(v){return [v.gone,v.last,'gone'in v,Object.hasOwn(v,'gone')].join(':')}
            for(let i=0;i<10000;i++)if(read(o)!=='2:4:true:true')throw Error('before');
            delete o.gone;console.log(read(o));delete proto.gone;console.log(read(o));
            o.gone=30;console.log(read(o));
            Object.setPrototypeOf(o,{get gone(){return this.a+40}});delete o.gone;console.log(read(o));
            const numeric={3:'three',1:'one',b:2,a:1};delete numeric[1];numeric[1]='again';
            console.log(Object.keys(numeric).join(','),numeric[1]);
            let traps=0;const proxy=new Proxy({x:1},{deleteProperty(t,k){traps++;return Reflect.deleteProperty(t,k)}});
            console.log(delete proxy.x,traps,'x'in proxy);
        "#);
        assert_eq!(output.borrow().trim_end(), "9:4:true:false\n:4:false:false\n30:4:true:true\n41:4:true:false\n1,3,b,a again\ntrue 1 false");
    }

    #[test]
    fn normal_delete_keeps_surviving_references_reachable_across_collections() {
        let (mut engine, output) = getter_program(r#"
            globalThis.large={};for(let i=0;i<40;i++)large['k'+i]=i;delete large.k1;
            if('k1'in large||large.k2!==2||large.k39!==39)throw Error('large delete');
            globalThis.list=[1,2,3];delete list[1];console.log(list.length,1 in list);
            const fn=function named(){};console.log(delete fn.name,Object.hasOwn(fn,'name'));
            globalThis.survivors=[];for(let i=0;i<500;i++){const o={a:{n:i},drop:2,b:3,c:4,d:5,e:6};delete o.drop;survivors.push(o)}
        "#);
        assert_eq!(output.borrow().trim_end(), "3 false\ntrue false");
        engine.collect_garbage();
        engine.eval("for(let i=0;i<survivors.length;i++)if(survivors[i].a.n!==i||survivors[i].b!==3||'drop'in survivors[i])throw Error('lost surviving value');globalThis.oldDeleteTarget={a:1,drop:2,b:3,c:4,d:5,e:6};", "delete-after-gc.js").unwrap();
        engine.collect_garbage();
        engine.eval("oldDeleteTarget.a={n:99};delete oldDeleteTarget.drop;", "delete-young-value.js").unwrap();
        engine.eval("0", "delete-end-job.js").unwrap();
        engine.vm.collect_young_garbage();
        engine.eval("if(oldDeleteTarget.a.n!==99||oldDeleteTarget.b!==3||'drop'in oldDeleteTarget)throw Error('old target lost young value');", "delete-after-minor.js").unwrap();
    }

    /// What's remembered about where properties are has to stop being true the moment the objects change, and the
    /// short ways for typed arrays have to leave anything unusual to the long way. The answers are what Node gives.
    #[test]
    fn what_is_remembered_follows_changes() {
        let source = r#"
        const out = [];
        // Getters found before, then changed.
        const proto = { get v() { return 1; } };
        const objects = [Object.create(proto), Object.create(proto)];
        for (const o of objects) out.push(o.v);
        Object.defineProperty(proto, 'v', { get() { return 2; }, configurable: true });
        for (const o of objects) out.push(o.v);
        delete proto.v;
        proto.v = 3;
        for (const o of objects) out.push(o.v);
        // Names that weren't anywhere, then are.
        const empty = [{}, { a: 1 }, Object.create(proto)];
        for (let r = 0; r < 2; r++) for (const o of empty) out.push(String(o.missing));
        Object.prototype.missing = 'now';
        for (const o of empty) out.push(o.missing);
        delete Object.prototype.missing;
        for (const o of empty) out.push(String(o.missing));
        const other = { missing: 'other' };
        Object.setPrototypeOf(empty[0], other);
        out.push(empty[0].missing, String(empty[1].missing));
        // Writes at one place to objects of several shapes, then a setter or a read-only one on the way.
        function Base() {}
        const kinds = [() => ({}), () => new Base(), () => ({ z: 1 })];
        function fill(o, v) { o.w = v; return o; }
        for (let r = 0; r < 3; r++) for (const make of kinds) out.push(fill(make(), r).w);
        Object.defineProperty(Base.prototype, 'w', { set(v) { this.seen = v * 10; }, configurable: true });
        const b = fill(new Base(), 4);
        out.push(b.seen, Object.keys(b).join());
        Object.defineProperty(Base.prototype, 'w', { value: 'fixed', writable: false });
        const c = fill(new Base(), 5);
        out.push(c.w, Object.keys(c).join());
        const sealed = Object.preventExtensions({});
        fill(sealed, 6);
        out.push(String(sealed.w));
        // Private fields, at a place that remembers the shape they make.
        class Holder { constructor(o) { return o; } }
        class Marked extends Holder { #mark = 1; constructor(o) { super(o); } static has(o) { return #mark in o; } }
        const target = {};
        new Marked(target);
        let twice = 'no';
        try { new Marked(target); } catch (e) { twice = e.constructor.name; }
        const fresh = {};
        new Marked(fresh);
        out.push(twice, Marked.has(target), Marked.has(fresh), Marked.has({}));
        // Typed arrays, the usual kind and the others.
        const bytes = new Uint8ClampedArray(4);
        bytes[0] = 300; bytes[1] = -3; bytes[2] = 1.5; bytes[3] = 2.5;
        out.push(Array.from(bytes).join());
        const ints = new Int16Array([1, 2, 3, 4, 5]);
        ints.set(ints.subarray(0, 3), 2);
        out.push(Array.from(ints).join());
        const floats = new Float32Array(3);
        floats.set(new Int8Array([-1, 2, -3]));
        out.push(Array.from(floats).join(), floats[5], floats[-1]);
        const buffer = new ArrayBuffer(8, { maxByteLength: 16 });
        const tracking = new Uint8Array(buffer);
        const fixed = new Uint8Array(buffer, 0, 8);
        tracking[7] = 9;
        buffer.resize(4);
        out.push(tracking.length, fixed.length, String(fixed[0]), String(tracking[7]));
        fixed[1] = 5;
        out.push(String(fixed[1]));
        buffer.transfer();
        out.push(tracking.length, String(tracking[0]));
        tracking[0] = 1;
        out.push(String(tracking[0]));
        const own = new Uint8Array(2);
        Object.defineProperty(own, 'length', { value: 'own' });
        out.push(own.length, new Uint8Array(3).length);
        console.log(out.join(' '));
"#;
        let output = Rc::new(RefCell::new(String::new()));
        let mut engine = Engine::new();
        let sink = output.clone();
        engine.set_printer(move |text| sink.borrow_mut().push_str(text));
        engine.eval(source, "test.js").unwrap();
        assert_eq!(
            output.borrow().trim_end(),
            "1 1 2 2 3 3 undefined undefined undefined undefined undefined undefined now now now undefined undefined undefined other undefined 0 0 0 1 1 1 2 2 2 40 seen fixed  undefined TypeError true true false 255,0,2,2 1,2,1,2,3 -1,2,-3   4 0 undefined undefined undefined 0 undefined undefined own 3"
        );
    }
}
