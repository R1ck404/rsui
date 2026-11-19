//! `Map`, `Set`, `WeakMap` and `WeakSet`.

use crate::object::*;
use crate::text;
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

/// Tells each of the methods on a prototype which kind of collection it's for. The ones written in JavaScript keep
/// their name there instead, and check for themselves.
fn mark_methods(vm: &mut Vm, proto: u32, kind: i32) {
    let shape = vm.objects[proto as usize].shape;
    for (_, slot, flags) in vm.shapes.entries(shape) {
        let mut value = vm.objects[proto as usize].slots()[slot as usize];
        if flags & ACCESSOR != 0 {
            if let Kind::Accessor { get, .. } = vm.objects[value.index()].kind {
                value = get;
            }
        }
        if value.is_object() {
            if let Kind::Native(native) = &mut vm.objects[value.index()].kind {
                if native.data.is_undefined() {
                    native.data = Value::int(kind);
                }
            }
        }
    }
}

pub(super) fn install(vm: &mut Vm) {
    let map_proto = vm.plain_proto();
    vm.realm.map_proto = map_proto;
    let map = vm.constructor("Map", 0, map_constructor, map_proto);
    vm.species_getter(map);
    vm.method(map_proto, "get", 1, map_get);
    vm.method(map_proto, "set", 2, map_set);
    vm.method(map_proto, "has", 1, collection_has);
    vm.method(map_proto, "delete", 1, collection_delete);
    vm.method(map_proto, "clear", 0, collection_clear);
    vm.method(map_proto, "forEach", 1, collection_for_each);
    vm.method(map_proto, "keys", 0, map_keys);
    vm.method(map_proto, "values", 0, map_values);
    vm.getter(map_proto, "size", collection_size);
    let entries = vm.intern_str("entries");
    let function = vm.new_native(entries, 0, map_entries, Value::UNDEFINED, false);
    vm.add_property(map_proto, entries, function, HIDDEN);
    vm.add_property(map_proto, text::SYMBOL_ITERATOR, function, HIDDEN);
    let tag = vm.new_str("Map");
    vm.add_property(map_proto, text::SYMBOL_TO_STRING_TAG, tag, CONFIGURABLE);

    let set_proto = vm.plain_proto();
    vm.realm.set_proto = set_proto;
    let set = vm.constructor("Set", 0, set_constructor, set_proto);
    vm.species_getter(set);
    for (name, length) in [
        ("union", 1),
        ("intersection", 1),
        ("difference", 1),
        ("symmetricDifference", 1),
        ("isSubsetOf", 1),
        ("isSupersetOf", 1),
        ("isDisjointFrom", 1),
    ] {
        vm.js_helper(set_proto, name, name, length);
    }
    vm.method(set_proto, "add", 1, set_add);
    vm.method(set_proto, "has", 1, collection_has);
    vm.method(set_proto, "delete", 1, collection_delete);
    vm.method(set_proto, "clear", 0, collection_clear);
    vm.method(set_proto, "forEach", 1, collection_for_each);
    vm.method(set_proto, "entries", 0, set_entries);
    vm.getter(set_proto, "size", collection_size);
    let values = vm.intern_str("values");
    let function = vm.new_native(values, 0, set_values, Value::UNDEFINED, false);
    vm.add_property(set_proto, values, function, HIDDEN);
    let keys = vm.intern_str("keys");
    vm.add_property(set_proto, keys, function, HIDDEN);
    vm.add_property(set_proto, text::SYMBOL_ITERATOR, function, HIDDEN);
    let tag = vm.new_str("Set");
    vm.add_property(set_proto, text::SYMBOL_TO_STRING_TAG, tag, CONFIGURABLE);

    let weak_map_proto = vm.plain_proto();
    vm.realm.weak_map_proto = weak_map_proto;
    vm.constructor("WeakMap", 0, weak_map_constructor, weak_map_proto);
    vm.method(weak_map_proto, "get", 1, map_get);
    vm.method(weak_map_proto, "set", 2, map_set);
    vm.method(weak_map_proto, "has", 1, collection_has);
    vm.method(weak_map_proto, "delete", 1, collection_delete);
    let tag = vm.new_str("WeakMap");
    vm.add_property(
        weak_map_proto,
        text::SYMBOL_TO_STRING_TAG,
        tag,
        CONFIGURABLE,
    );

    let weak_set_proto = vm.plain_proto();
    vm.realm.weak_set_proto = weak_set_proto;
    vm.constructor("WeakSet", 0, weak_set_constructor, weak_set_proto);
    vm.method(weak_set_proto, "add", 1, set_add);
    vm.method(weak_set_proto, "has", 1, collection_has);
    vm.method(weak_set_proto, "delete", 1, collection_delete);
    let tag = vm.new_str("WeakSet");
    vm.add_property(
        weak_set_proto,
        text::SYMBOL_TO_STRING_TAG,
        tag,
        CONFIGURABLE,
    );

    mark_methods(vm, map_proto, MAP);
    mark_methods(vm, weak_map_proto, WEAK_MAP);
    mark_methods(vm, weak_set_proto, WEAK_SET);
    mark_methods(vm, set_proto, SET);

    let weak_ref_proto = vm.plain_proto();
    vm.realm.weak_ref_proto = weak_ref_proto;
    vm.constructor("WeakRef", 1, weak_ref_constructor, weak_ref_proto);
    vm.method(weak_ref_proto, "deref", 0, weak_ref_deref);
    let tag = vm.new_str("WeakRef");
    vm.add_property(
        weak_ref_proto,
        text::SYMBOL_TO_STRING_TAG,
        tag,
        CONFIGURABLE,
    );

    let registry_proto = vm.plain_proto();
    vm.realm.finalization_registry_proto = registry_proto;
    vm.constructor(
        "FinalizationRegistry",
        1,
        registry_constructor,
        registry_proto,
    );
    vm.method(registry_proto, "register", 2, registry_register);
    vm.method(registry_proto, "unregister", 1, registry_unregister);
    let tag = vm.new_str("FinalizationRegistry");
    vm.add_property(
        registry_proto,
        text::SYMBOL_TO_STRING_TAG,
        tag,
        CONFIGURABLE,
    );
}

/// What can be held weakly: objects, and symbols that aren't in the global registry.
fn can_be_held_weakly(vm: &Vm, value: Value) -> bool {
    value.is_object()
        || (value.is_symbol()
            && !vm
                .symbol_registry
                .values()
                .any(|atom| atom.0 == value.handle()))
}

// What these hold weakly the collector lets go of, and tidies up after (`Vm::clear_weak` in gc.rs).
fn weak_ref_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    needs_new(vm, &args, "WeakRef")?;
    let target = vm.arg(&args, 0);
    if !can_be_held_weakly(vm, target) {
        return Err(vm.type_error("WeakRef: target must be an object or a symbol"));
    }
    let proto = vm.proto_for(&args, vm.realm.weak_ref_proto)?;
    vm.keep_during_job(target);
    Ok(vm.new_object_with_proto(proto, Kind::WeakRef(target)))
}

fn weak_ref_deref(vm: &mut Vm, args: Args) -> Run<Value> {
    if args.this.is_object() {
        if let Kind::WeakRef(target) = vm.objects[args.this.index()].kind {
            if !target.is_undefined() {
                vm.keep_during_job(target);
            }
            return Ok(target);
        }
    }
    Err(vm.type_error("this is not a WeakRef."))
}

fn registry_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    needs_new(vm, &args, "FinalizationRegistry")?;
    let cleanup = vm.arg(&args, 0);
    if !vm.is_callable(cleanup) {
        return Err(vm.type_error("FinalizationRegistry: cleanup must be callable"));
    }
    let proto = vm.proto_for(&args, vm.realm.finalization_registry_proto)?;
    Ok(vm.new_object_with_proto(
        proto,
        Kind::FinalizationRegistry(Box::new(RegistryData {
            cleanup,
            cells: Vec::new(),
            cleared: Default::default(),
            queued: false,
        })),
    ))
}

fn check_registry(vm: &mut Vm, this: Value) -> Run<()> {
    if this.is_object() && matches!(vm.objects[this.index()].kind, Kind::FinalizationRegistry(_)) {
        return Ok(());
    }
    Err(vm.type_error("this is not a FinalizationRegistry."))
}

fn registry_register(vm: &mut Vm, args: Args) -> Run<Value> {
    check_registry(vm, args.this)?;
    let (target, held, token) = (vm.arg(&args, 0), vm.arg(&args, 1), vm.arg(&args, 2));
    if !can_be_held_weakly(vm, target) {
        return Err(vm.type_error("register: target must be an object or a symbol"));
    }
    if vm.same_value(target, held) {
        return Err(vm.type_error("register: target and holdings must not be the same"));
    }
    if !token.is_undefined() && !can_be_held_weakly(vm, token) {
        return Err(vm.type_error("register: the token must be an object or a symbol"));
    }
    let token = if token.is_undefined() {
        Value::EMPTY
    } else {
        token
    };
    // Through `objects[..]`, as the registry may be old and these young.
    if let Kind::FinalizationRegistry(data) = &mut vm.objects[args.this.index()].kind {
        data.cells.push(RegistryCell {
            target,
            held,
            token,
        });
    }
    vm.debt += 24;
    Ok(Value::UNDEFINED)
}

fn registry_unregister(vm: &mut Vm, args: Args) -> Run<Value> {
    check_registry(vm, args.this)?;
    let token = vm.arg(&args, 0);
    if !can_be_held_weakly(vm, token) {
        return Err(vm.type_error("unregister: the token must be an object or a symbol"));
    }
    // An object or a symbol is only ever the same as itself. A cell whose target has gone but whose callback hasn't
    // been called yet is taken off too.
    let Kind::FinalizationRegistry(data) = &mut vm.objects[args.this.index()].kind else {
        unreachable!("just checked");
    };
    let before = data.cells.len() + data.cleared.len();
    data.cells.retain(|cell| cell.token != token);
    data.cleared.retain(|(_, cell_token)| *cell_token != token);
    Ok(Value::bool(data.cells.len() + data.cleared.len() != before))
}

impl Vm {
    /// Keeps what a `WeakRef` was made with or handed out alive until nothing's running, as the language says
    /// (AddToKeptObjects): code that has just got hold of something can count on it for the rest of what it's doing.
    pub(crate) fn keep_during_job(&mut self, target: Value) {
        if self.kept.last() == Some(&target) {
            return;
        }
        self.kept.push(target);
        // A loop that keeps asking for the same few would otherwise make this long.
        if self.kept.len() >= 64 && self.kept.len().is_power_of_two() {
            self.kept.sort_unstable_by_key(|value| value.bits());
            self.kept.dedup();
        }
    }

    /// Lets what `WeakRef`s have handed out go, now that nothing's running (ClearKeptObjects).
    #[doc(hidden)]
    pub fn clear_kept_objects(&mut self) {
        self.kept.clear();
        crate::gc::shrink_if_oversized(&mut self.kept);
    }

    /// Calls a registry's callback with what each target that has gone was registered with, one at a time, as each
    /// could unregister others. The language leaves the order open (Node's has no pattern to it): this is the order they
    /// were registered in.
    pub(crate) fn clean_up_registry(&mut self, registry: Value) -> Run<()> {
        // It mustn't go while its callback runs, which nothing else may be holding it for.
        self.nursery.push(registry);
        loop {
            let Kind::FinalizationRegistry(data) = &mut self.objects[registry.index()].kind else {
                return Ok(());
            };
            data.queued = false;
            let Some((held, _)) = data.cleared.pop_front() else {
                return Ok(());
            };
            let cleanup = data.cleanup;
            self.call(cleanup, Value::UNDEFINED, &[held])?;
        }
    }
}

impl MapData {
    /// Tidies up after the collector has taken out the entries of a `WeakMap` or a `WeakSet` whose keys have gone: an
    /// empty one gives back all it had, and one that's mostly holes is laid out again, smaller. Their keys are objects
    /// and symbols, which are only ever the same as themselves, so they hash to what they are.
    pub(crate) fn tidy_after_losing_keys(&mut self) {
        if self.live == 0 {
            *self = MapData::default();
            return;
        }
        if self.entries.len() < 64 || self.live as usize * 4 >= self.entries.len() {
            return;
        }
        self.entries.retain(|entry| !entry.0.is_empty());
        self.entries.shrink_to(self.entries.len() * 2);
        let count = self.entries.len().next_power_of_two().max(8);
        self.buckets.clear();
        self.buckets.resize(count, NONE);
        self.buckets.shrink_to(count);
        self.chain.clear();
        for (at, entry) in self.entries.iter().enumerate() {
            let bucket = bucket_of(entry.0.bits(), count);
            self.chain.push(self.buckets[bucket]);
            self.buckets[bucket] = at as u32;
        }
        self.chain.shrink_to(self.entries.capacity());
    }
}

/// Which bucket a key with this hash goes in. There's always a power of two of them.
#[inline]
fn bucket_of(hash: u64, buckets: usize) -> usize {
    (hash.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as usize & (buckets - 1)
}

impl Vm {
    /// Whether two native Maps contain the same live keys and SameValue-equal values, regardless of insertion order.
    /// This reads internal entries without invoking overridden getters, iterators, or methods, and allocates nothing.
    /// Non-Map values, including proxies and weak collections, return false.
    pub fn same_map_entries(&self, first: Value, second: Value) -> bool {
        if !first.is_object() || !second.is_object() {
            return false;
        }
        let (Kind::Map(left), Kind::Map(right)) =
            (&self.objects[first.index()].kind, &self.objects[second.index()].kind)
        else {
            return false;
        };
        if left.live != right.live {
            return false;
        }
        left.entries.iter().filter(|entry| !entry.0.is_empty()).all(|&(key, value)| {
            self.find_hashed(second, key, self.key_hash(key))
                .flatten()
                .is_some_and(|at| self.same_value(value, right.entries[at].1))
        })
    }

    /// A number standing for a key. Equal keys always have the same one, and others mostly don't.
    #[inline]
    pub(crate) fn key_hash(&self, key: Value) -> u64 {
        if key.is_int() {
            // As a number would be below: an int is already the one way of writing its number, and zero is zero.
            return if key.as_int() == 0 { 0 } else { key.0 };
        }
        if key.is_string() {
            return u64::from(self.strings.items[key.index()].hash()) | (1 << 62);
        }
        if key.is_bigint() {
            let data = self.big_of(key);
            let mut hash = u64::from(data.negative);
            for digit in &data.digits {
                hash = hash.wrapping_mul(0x0100_0000_01B3) ^ u64::from(*digit);
            }
            return hash | (1 << 61);
        }
        if key.is_number() {
            let number = key.as_number();
            // Zero and negative zero are the same key, and so are an int and a double of the same number.
            return if number == 0.0 {
                0
            } else {
                Value::number(number).0
            };
        }
        key.0
    }

    fn data_of(&mut self, this: Value, wanted: &str) -> Run<&mut MapData> {
        if this.is_object() {
            if let Kind::Map(_) | Kind::Set(_) | Kind::WeakMap(_) | Kind::WeakSet(_) =
                &self.objects[this.index()].kind
            {
                let (Kind::Map(data) | Kind::Set(data) | Kind::WeakMap(data) | Kind::WeakSet(data)) =
                    &mut self.objects[this.index()].kind
                else {
                    unreachable!("just checked");
                };
                return Ok(data);
            }
        }
        Err(self.type_error(&format!("Method {wanted} called on incompatible receiver")))
    }

    fn find_entry(&mut self, this: Value, key: Value) -> Run<Option<usize>> {
        let hash = self.key_hash(key);
        match self.find_hashed(this, key, hash) {
            Some(found) => Ok(found),
            None => Err(self.type_error("Method Map.prototype.get called on incompatible receiver")),
        }
    }

    /// The entry with this key, given what it hashes to, or nothing at all if this isn't a collection.
    #[inline]
    pub(crate) fn find_hashed(&self, this: Value, key: Value, hash: u64) -> Option<Option<usize>> {
        if !this.is_object() {
            return None;
        }
        let (Kind::Map(data) | Kind::Set(data) | Kind::WeakMap(data) | Kind::WeakSet(data)) =
            &self.objects[this.index()].kind
        else {
            return None;
        };
        if data.buckets.is_empty() {
            return Some(None);
        }
        let mut at = data.buckets[bucket_of(hash, data.buckets.len())];
        while at != NONE {
            let found = data.entries[at as usize].0;
            if found == key || (!found.is_empty() && self.same_key(found, key)) {
                return Some(Some(at as usize));
            }
            at = data.chain[at as usize];
        }
        Some(None)
    }

    /// Whether two keys that aren't the very same value are the same key, as `SameValueZero` says. Two strings with
    /// different hashes can't be, and two ints can't be either, which settles most of what's in a chain.
    #[inline]
    fn same_key(&self, found: Value, key: Value) -> bool {
        if key.is_string() {
            if !found.is_string() {
                return false;
            }
            let (first, second) = (
                &self.strings.items[found.index()],
                &self.strings.items[key.index()],
            );
            return first.hash() == second.hash() && first.text == second.text;
        }
        if key.is_int() && found.is_int() {
            return false;
        }
        self.same_value_zero(found, key)
    }

    /// Lays the buckets out again, for how many entries there are now.
    fn rebuild_buckets(&mut self, this: Value) {
        let (Kind::Map(data) | Kind::Set(data) | Kind::WeakMap(data) | Kind::WeakSet(data)) =
            &self.objects[this.index()].kind
        else {
            return;
        };
        let count = data.entries.len().next_power_of_two().max(8);
        let mut buckets = vec![NONE; count];
        let mut chain = Vec::with_capacity(data.entries.capacity());
        for (at, entry) in data.entries.iter().enumerate() {
            if entry.0.is_empty() {
                chain.push(NONE);
                continue;
            }
            let bucket = bucket_of(self.key_hash(entry.0), count);
            chain.push(buckets[bucket]);
            buckets[bucket] = at as u32;
        }
        if let Kind::Map(data) | Kind::Set(data) | Kind::WeakMap(data) | Kind::WeakSet(data) =
            &mut self.objects[this.index()].kind
        {
            data.buckets = buckets;
            data.chain = chain;
        }
    }

    fn insert_entry(&mut self, this: Value, key: Value, value: Value) -> Run<()> {
        let hash = self.key_hash(key);
        if let Some(at) = self.find_entry_hashed(this, key, hash)? {
            let data = self.data_of(this, "Map.prototype.set")?;
            data.entries[at].1 = value;
            return Ok(());
        }
        // Negative zero goes in as zero, which hashes the same.
        let key = if key.is_number() && key.as_number() == 0.0 {
            Value::int(0)
        } else {
            key
        };
        let data = self.data_of(this, "Map.prototype.set")?;
        let at = data.entries.len() as u32;
        data.entries.push((key, value));
        data.chain.push(NONE);
        data.live += 1;
        if data.entries.len() > data.buckets.len() {
            self.rebuild_buckets(this);
        } else {
            let bucket = bucket_of(hash, data.buckets.len());
            data.chain[at as usize] = data.buckets[bucket];
            data.buckets[bucket] = at;
        }
        self.debt += 32;
        Ok(())
    }

    fn find_entry_hashed(&mut self, this: Value, key: Value, hash: u64) -> Run<Option<usize>> {
        match self.find_hashed(this, key, hash) {
            Some(found) => Ok(found),
            None => Err(self.type_error("Method Map.prototype.get called on incompatible receiver")),
        }
    }

    fn remove_entry(&mut self, this: Value, key: Value) -> Run<bool> {
        let Some(at) = self.find_entry(this, key)? else {
            return Ok(false);
        };
        let data = self.data_of(this, "Map.prototype.delete")?;
        data.entries[at] = (Value::EMPTY, Value::UNDEFINED);
        data.live -= 1;
        // A map used as a queue shouldn't keep growing behind the scenes.
        if data.entries.len() >= 64 && data.live as usize * 4 < data.entries.len() {
            self.compact_entries(this);
        }
        Ok(true)
    }

    fn compact_entries(&mut self, this: Value) {
        if let Kind::Map(data) | Kind::Set(data) | Kind::WeakMap(data) | Kind::WeakSet(data) =
            &mut self.objects[this.index()].kind
        {
            data.entries.retain(|entry| !entry.0.is_empty());
        }
        self.rebuild_buckets(this);
    }

    /// Fills a new map or set from what it was given to start with.
    fn fill_collection(&mut self, collection: Value, source: Value, is_map: bool) -> Run<()> {
        if source.is_nullish() {
            return Ok(());
        }
        // It goes through the method the program can see, as the language says, and that could be anything.
        let name = if is_map { "set" } else { "add" };
        let adder = self.get_named(collection, name)?;
        if !self.is_callable(adder) {
            return Err(self.type_error(&format!(
                "'{name}' returned for property of object is not a function"
            )));
        }
        let (iterator, next) = self.get_iterator(source)?;
        if is_map && self.copy_map_entries(collection, adder, iterator) {
            return Ok(());
        }
        self.iterate_from(iterator, next, |vm, item| {
            if is_map {
                if !item.is_object() {
                    let shown = vm.brief(item);
                    return Err(
                        vm.type_error(&format!("Iterator value {shown} is not an entry object"))
                    );
                }
                let key = vm.get_index(item, 0)?;
                let value = vm.get_index(item, 1)?;
                vm.call(adder, collection, &[key, value])?;
            } else {
                vm.call(adder, collection, &[item])?;
            }
            Ok(())
        })
    }

    /// The built-in Map adder has no observable work between entries. Copy its native iterator directly instead of
    /// allocating an entry array and making a native call for every key. Both getters have already run; overridden
    /// adders and other iterators keep the ordinary path, including iterator closing on exceptions.
    fn copy_map_entries(&mut self, collection: Value, adder: Value, iterator: Value) -> bool {
        if !matches!(&self.objects[collection.index()].kind, Kind::Map(data) if data.live == 0)
            || !adder.is_object()
            || !matches!(&self.objects[adder.index()].kind, Kind::Native(native)
                if std::ptr::fn_addr_eq(native.func, map_set as NativeFn)
                    && native.data == Value::int(MAP))
        {
            return false;
        }
        let Kind::Iter(iter) = &self.objects[iterator.index()].kind else {
            return false;
        };
        if !matches!(iter.kind, IterKind::MapEntries) {
            return false;
        }
        let (source, at) = (iter.target, iter.at as usize);
        let Kind::Map(data) = &self.objects[source.index()].kind else {
            return false;
        };
        let remaining = data
            .entries
            .len()
            .saturating_sub(at)
            .min(data.live as usize);
        let mut entries = Vec::with_capacity(remaining);
        entries.extend(
            data.entries
                .iter()
                .skip(at)
                .copied()
                .filter(|entry| !entry.0.is_empty()),
        );
        let live = entries.len() as u32;
        if let Kind::Map(data) = &mut self.objects[collection.index()].kind {
            data.entries = entries;
            data.live = live;
            if live == 0 {
                data.buckets.clear();
                data.chain.clear();
            }
        }
        if live != 0 {
            self.rebuild_buckets(collection);
        }
        self.debt += live as usize * 32;
        if let Kind::Iter(iter) = &mut self.objects[iterator.index()].kind {
            iter.at = u32::MAX / 2;
        }
        true
    }
}

fn needs_new(vm: &mut Vm, args: &Args, name: &str) -> Run<()> {
    if !args.new_target.is_object() {
        return Err(vm.type_error(&format!("Constructor {name} requires 'new'")));
    }
    Ok(())
}

fn map_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    needs_new(vm, &args, "Map")?;
    let proto = vm.proto_for(&args, vm.realm.map_proto)?;
    let map = vm.new_object_with_proto(proto, Kind::Map(Box::default()));
    let source = vm.arg(&args, 0);
    vm.fill_collection(map, source, true)?;
    Ok(map)
}

fn set_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    needs_new(vm, &args, "Set")?;
    let proto = vm.proto_for(&args, vm.realm.set_proto)?;
    let set = vm.new_object_with_proto(proto, Kind::Set(Box::default()));
    let source = vm.arg(&args, 0);
    vm.fill_collection(set, source, false)?;
    Ok(set)
}

fn weak_map_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    needs_new(vm, &args, "WeakMap")?;
    let proto = vm.proto_for(&args, vm.realm.weak_map_proto)?;
    let map = vm.new_object_with_proto(proto, Kind::WeakMap(Box::default()));
    let source = vm.arg(&args, 0);
    vm.fill_collection(map, source, true)?;
    Ok(map)
}

fn weak_set_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    needs_new(vm, &args, "WeakSet")?;
    let proto = vm.proto_for(&args, vm.realm.weak_set_proto)?;
    let set = vm.new_object_with_proto(proto, Kind::WeakSet(Box::default()));
    let source = vm.arg(&args, 0);
    vm.fill_collection(set, source, false)?;
    Ok(set)
}

/// Which kinds of collection a method can be used on. A method's name is how it says.
fn check_kind(vm: &mut Vm, args: &Args) -> Run<()> {
    let callee_proto = match &vm.objects[args.callee as usize].kind {
        Kind::Native(native) => native.data,
        _ => Value::UNDEFINED,
    };
    let wanted = callee_proto.is_int().then(|| callee_proto.as_int());
    let found = if args.this.is_object() {
        match &vm.objects[args.this.index()].kind {
            Kind::Map(_) => Some(MAP),
            Kind::Set(_) => Some(SET),
            Kind::WeakMap(_) => Some(WEAK_MAP),
            Kind::WeakSet(_) => Some(WEAK_SET),
            _ => None,
        }
    } else {
        None
    };
    if wanted.is_some() && wanted != found {
        return Err(vm.type_error("Method called on incompatible receiver"));
    }
    Ok(())
}

const MAP: i32 = 0;
const SET: i32 = 1;
const WEAK_MAP: i32 = 2;
const WEAK_SET: i32 = 3;

pub(crate) fn map_get(vm: &mut Vm, args: Args) -> Run<Value> {
    check_kind(vm, &args)?;
    let key = vm.arg(&args, 0);
    let hash = vm.key_hash(key);
    match vm.find_hashed(args.this, key, hash) {
        Some(Some(at)) => {
            let (Kind::Map(data) | Kind::WeakMap(data)) = &vm.objects[args.this.index()].kind
            else {
                return Ok(vm.data_of(args.this, "Map.prototype.get")?.entries[at].1);
            };
            Ok(data.entries[at].1)
        }
        Some(None) => Ok(Value::UNDEFINED),
        None => Err(vm.type_error("Method Map.prototype.get called on incompatible receiver")),
    }
}

fn map_set(vm: &mut Vm, args: Args) -> Run<Value> {
    check_kind(vm, &args)?;
    let (key, value) = (vm.arg(&args, 0), vm.arg(&args, 1));
    if matches!(vm.objects[args.this.index()].kind, Kind::WeakMap(_))
        && !can_be_held_weakly(vm, key)
    {
        return Err(vm.type_error("Invalid value used as weak map key"));
    }
    vm.insert_entry(args.this, key, value)?;
    Ok(args.this)
}

fn set_add(vm: &mut Vm, args: Args) -> Run<Value> {
    check_kind(vm, &args)?;
    let key = vm.arg(&args, 0);
    if matches!(vm.objects[args.this.index()].kind, Kind::WeakSet(_))
        && !can_be_held_weakly(vm, key)
    {
        return Err(vm.type_error("Invalid value used in weak set"));
    }
    vm.insert_entry(args.this, key, key)?;
    Ok(args.this)
}

pub(crate) fn collection_has(vm: &mut Vm, args: Args) -> Run<Value> {
    check_kind(vm, &args)?;
    let key = vm.arg(&args, 0);
    Ok(Value::bool(vm.find_entry(args.this, key)?.is_some()))
}

fn collection_delete(vm: &mut Vm, args: Args) -> Run<Value> {
    check_kind(vm, &args)?;
    let key = vm.arg(&args, 0);
    Ok(Value::bool(vm.remove_entry(args.this, key)?))
}

fn collection_clear(vm: &mut Vm, args: Args) -> Run<Value> {
    check_kind(vm, &args)?;
    let data = vm.data_of(args.this, "Map.prototype.clear")?;
    // Iterators that are going through this still see holes, and then the end.
    data.entries
        .iter_mut()
        .for_each(|entry| *entry = (Value::EMPTY, Value::UNDEFINED));
    data.buckets.fill(NONE);
    data.chain.fill(NONE);
    data.live = 0;
    Ok(Value::UNDEFINED)
}

fn collection_size(vm: &mut Vm, args: Args) -> Run<Value> {
    check_kind(vm, &args)?;
    let live = vm.data_of(args.this, "get Map.prototype.size")?.live;
    Ok(Value::number(f64::from(live)))
}

fn collection_for_each(vm: &mut Vm, args: Args) -> Run<Value> {
    check_kind(vm, &args)?;
    let function = vm.arg(&args, 0);
    if !vm.is_callable(function) {
        let shown = vm.brief(function);
        return Err(vm.type_error(&format!("{shown} is not a function")));
    }
    let this_arg = vm.arg(&args, 1);
    let callee = vm.prepare_call(function);
    let mut at = 0;
    loop {
        let data = vm.data_of(args.this, "Map.prototype.forEach")?;
        let Some((key, value)) = data.entries.get(at).copied() else {
            break;
        };
        at += 1;
        if key.is_empty() {
            continue;
        }
        vm.call_prepared(&callee, this_arg, &[value, key, args.this])?;
    }
    Ok(Value::UNDEFINED)
}

fn iterator_of(vm: &mut Vm, this: Value, kind: IterKind) -> Run<Value> {
    let is_right = this.is_object()
        && matches!(
            (&vm.objects[this.index()].kind, kind),
            (
                Kind::Map(_),
                IterKind::MapKeys | IterKind::MapValues | IterKind::MapEntries
            ) | (Kind::Set(_), IterKind::SetValues | IterKind::SetEntries)
        );
    if !is_right {
        return Err(vm.type_error("Method called on incompatible receiver"));
    }
    Ok(vm.new_native_iter(kind, this))
}

fn map_keys(vm: &mut Vm, args: Args) -> Run<Value> {
    iterator_of(vm, args.this, IterKind::MapKeys)
}

fn map_values(vm: &mut Vm, args: Args) -> Run<Value> {
    iterator_of(vm, args.this, IterKind::MapValues)
}

fn map_entries(vm: &mut Vm, args: Args) -> Run<Value> {
    iterator_of(vm, args.this, IterKind::MapEntries)
}

fn set_values(vm: &mut Vm, args: Args) -> Run<Value> {
    iterator_of(vm, args.this, IterKind::SetValues)
}

fn set_entries(vm: &mut Vm, args: Args) -> Run<Value> {
    iterator_of(vm, args.this, IterKind::SetEntries)
}

// What a host function uses to read and make maps, as `new Map(old)`, `map.get(key)` and `map.set(key, value)` do with the
// built-in methods, without going through what the program can see: rsui-tsx works out an element's variables with them.
impl Vm {
    /// Whether this is a `Map`.
    pub fn is_map(&self, value: Value) -> bool {
        value.is_object() && matches!(self.objects[value.index()].kind, Kind::Map(_))
    }

    /// What a `Map` has for this key, as `map.get(key)` says, or `None` if it isn't a `Map`.
    pub fn map_value(&self, map: Value, key: Value) -> Option<Value> {
        if !self.is_map(map) {
            return None;
        }
        let Kind::Map(data) = &self.objects[map.index()].kind else {
            return None;
        };
        Some(match self.find_hashed(map, key, self.key_hash(key)) {
            Some(Some(at)) => data.entries[at].1,
            _ => Value::UNDEFINED,
        })
    }

    /// What a `Map` has for a string key with this text, as `map.get(text)` says, without making the string for a small one,
    /// or `None` if it isn't a `Map`.
    pub fn map_text_value(&mut self, map: Value, text: &str) -> Option<Value> {
        let size = self.map_size(map)?;
        if size > 16 {
            let key = self.new_str(text);
            return self.map_value(map, key);
        }
        let Kind::Map(data) = &self.objects[map.index()].kind else {
            return None;
        };
        for &(key, value) in &data.entries {
            if key.is_string() {
                let found = self.text_of(key);
                let same = match found.ascii() {
                    Some(bytes) => bytes == text.as_bytes(),
                    None => found.to_rust() == text,
                };
                if same {
                    return Some(value);
                }
            }
        }
        Some(Value::UNDEFINED)
    }

    /// How many entries a `Map` has, as `map.size` says, or `None` if it isn't a `Map`.
    pub fn map_size(&self, map: Value) -> Option<usize> {
        match self.is_map(map).then(|| &self.objects[map.index()].kind) {
            Some(Kind::Map(data)) => Some(data.live as usize),
            _ => None,
        }
    }

    /// A new `Map` with what another has in it, in its order, as `new Map(old)` makes, or an empty one for anything that isn't
    /// a `Map`.
    pub fn map_copied(&mut self, from: Value) -> Value {
        let entries: Vec<(Value, Value)> = match self.is_map(from).then(|| &self.objects[from.index()].kind) {
            Some(Kind::Map(data)) => data.entries.iter().copied().filter(|entry| !entry.0.is_empty()).collect(),
            _ => Vec::new(),
        };
        let proto = self.realm.map_proto;
        let map = self.new_object_with_proto(proto, Kind::Map(Box::default()));
        let live = entries.len() as u32;
        if live > 0 {
            self.debt += live as usize * 32;
            if let Kind::Map(data) = &mut self.objects[map.index()].kind {
                data.entries = entries;
                data.live = live;
            }
            self.rebuild_buckets(map);
        }
        map
    }

    /// Puts an entry in a `Map`, as `map.set(key, value)` does.
    pub fn map_put(&mut self, map: Value, key: Value, value: Value) -> Run<()> {
        if !self.is_map(map) {
            return Err(self.type_error("Method Map.prototype.set called on incompatible receiver"));
        }
        self.insert_entry(map, key, value)
    }
}
