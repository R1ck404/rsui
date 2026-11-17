//! Bounded, non-executing reads for native certificates of ordinary JS data.
use crate::object::{ACCESSOR, ENUMERABLE, Kind, NONE};
use crate::{Atom, Value, Vm};

/// An own data slot; absence is distinct from a slot containing undefined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataProperty {
    pub value: Value,
    pub enumerable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataSurfaceError {
    Unsupported,
    LimitExceeded,
}

const MAX_DEPTH: usize = 64;
const MAX_STRING_UNITS: usize = 4096;
const MAX_DESCRIPTOR_PROPERTIES: u32 = 4096;
const MAX_INTRINSIC_SLOTS: usize = 72;

#[derive(Clone, Copy)]
pub(crate) struct SelectorIntrinsic {
    pub(crate) owner: Value,
    key: Atom,
    pub(crate) value: Value,
    flags: u8,
    present: bool,
}

fn primitive(value: Value) -> bool {
    value.is_undefined() || value.is_null() || value.is_bool() || value.is_number() || value.is_string()
}

impl Vm {
    /// The intrinsic prototype a fresh ordinary object inherits, without
    /// consulting a replaceable global Object constructor or running any code.
    pub fn data_surface_object_prototype(&self) -> Value {
        Value::object(self.realm.object_proto)
    }

    /// Certifies a genuine ordinary object with no own accessors, including
    /// non-enumerable ones. Property values may themselves be objects/functions.
    /// This does not certify its prototype; use data_surface_lookup to read it.
    pub fn data_surface_plain(&self, value: Value) -> bool {
        if !value.is_object() { return false; }
        let object = &self.objects[value.index()];
        if !matches!(object.kind, Kind::Plain) { return false; }
        if self.shapes.is_all_plain(object.shape) { return true; }
        // Deleting even one field gives a normal bag a dictionary shape. Inspect
        // its bounded historical list directly, without a descriptor scratch Vec.
        if self.shapes.is_dict(object.shape) {
            return self.shapes.bounded_dictionary_has_no_accessors(object.shape, MAX_DESCRIPTOR_PROPERTIES as usize);
        }
        if self.shapes.items[object.shape as usize].count > MAX_DESCRIPTOR_PROPERTIES { return false; }
        self.shapes.entries(object.shape).iter().all(|(_, _, flags)| flags & ACCESSOR == 0)
    }

    /// Interns a string key without pinning it for the engine's lifetime.
    /// Like atoms returned by properties(), it must not outlive the rooted input
    /// string or a return to JS/collection without an atom-generation check.
    pub fn data_surface_key(&mut self, value: Value) -> Result<Atom, DataSurfaceError> {
        if !value.is_string() { return Err(DataSurfaceError::Unsupported); }
        self.to_atom(value).map_err(|_| DataSurfaceError::Unsupported)
    }

    /// Reads only a genuine ordinary object's own data property. It never calls
    /// getters, proxies or conversion methods. Other unrelated accessors are not
    /// read; certify the whole object separately when emulating object spread.
    pub fn data_surface_own(&mut self, value: Value, name: Atom) -> Result<Option<DataProperty>, DataSurfaceError> {
        if !value.is_object() || !matches!(self.objects[value.index()].kind, Kind::Plain) {
            return Err(DataSurfaceError::Unsupported);
        }
        let shape = self.objects[value.index()].shape;
        let Some((slot, flags)) = self.shapes.lookup(shape, name) else { return Ok(None); };
        if flags & ACCESSOR != 0 { return Err(DataSurfaceError::Unsupported); }
        let value = self.objects[value.index()].slots()[slot as usize];
        if value.is_empty() || value.is_cell() { return Err(DataSurfaceError::Unsupported); }
        Ok(Some(DataProperty { value, enumerable: flags & ENUMERABLE != 0 }))
    }

    /// Ordinary [[Get]] without code execution: follows at most 64 ordinary
    /// objects, rejecting a selected accessor or any traversed proxy/exotic.
    /// Unrelated prototype accessors (such as Object.prototype.__proto__) do not
    /// affect a lookup and are not executed. A prototype cycle declines safely.
    pub fn data_surface_lookup(&mut self, mut value: Value, name: Atom) -> Result<Option<DataProperty>, DataSurfaceError> {
        for _ in 0..MAX_DEPTH {
            if let Some(found) = self.data_surface_own(value, name)? { return Ok(Some(found)); }
            let shape = self.objects[value.index()].shape;
            let parent = self.shapes.items[shape as usize].proto;
            if parent == NONE { return Ok(None); }
            value = Value::object(parent);
        }
        Err(DataSurfaceError::LimitExceeded)
    }

    /// Copies genuine Map entries in insertion order, ignoring deletions.
    /// Returned Values are borrowed from this VM: root anything kept across JS
    /// execution, using Root just like other embedding APIs.
    pub fn data_surface_map(&self, value: Value, max_entries: usize) -> Result<Vec<(Value, Value)>, DataSurfaceError> {
        if !value.is_object() { return Err(DataSurfaceError::Unsupported); }
        let Kind::Map(data) = &self.objects[value.index()].kind else { return Err(DataSurfaceError::Unsupported); };
        if data.live as usize > max_entries { return Err(DataSurfaceError::LimitExceeded); }
        Ok(data.entries.iter().filter(|(key, _)| !key.is_empty()).copied().collect())
    }

    /// Genuine Map.get without consulting JS methods or conversions.
    pub fn data_surface_map_get(&self, map: Value, key: Value) -> Result<Option<Value>, DataSurfaceError> {
        if !map.is_object() || !matches!(self.objects[map.index()].kind, Kind::Map(_)) {
            return Err(DataSurfaceError::Unsupported);
        }
        let position = self.find_hashed(map, key, self.key_hash(key)).expect("genuine Map");
        let Kind::Map(data) = &self.objects[map.index()].kind else { unreachable!() };
        Ok(position.map(|at| data.entries[at].1))
    }

    /// Genuine Set values in insertion order, bounded before allocating.
    pub fn data_surface_set(&self, value: Value, max_entries: usize) -> Result<Vec<Value>, DataSurfaceError> {
        if !value.is_object() { return Err(DataSurfaceError::Unsupported); }
        let Kind::Set(data) = &self.objects[value.index()].kind else { return Err(DataSurfaceError::Unsupported); };
        if data.live as usize > max_entries { return Err(DataSurfaceError::LimitExceeded); }
        Ok(data.entries.iter().filter(|(key, _)| !key.is_empty()).map(|(key, _)| *key).collect())
    }

    // Reads descriptor slots, including the canonical species accessor objects,
    // WITHOUT invoking accessors. Only the fixed intrinsic owners use this path.
    fn selector_intrinsic_slot(&mut self, mut owner: Value, key: Atom) -> Result<Option<(Value, u8)>, DataSurfaceError> {
        for _ in 0..MAX_DEPTH {
            if !owner.is_object() || !matches!(self.objects[owner.index()].kind,
                Kind::Plain | Kind::Array(_) | Kind::Boxed(_) | Kind::Native(_))
            { return Err(DataSurfaceError::Unsupported); }
            let shape = self.objects[owner.index()].shape;
            if let Some((slot, flags)) = self.shapes.lookup(shape, key) {
                return Ok(Some((self.objects[owner.index()].slots()[slot as usize], flags)));
            }
            let parent = self.shapes.items[shape as usize].proto;
            if parent == NONE { return Ok(None); }
            owner = Value::object(parent);
        }
        Err(DataSurfaceError::LimitExceeded)
    }

    fn capture_selector_slot(&mut self, owner: Value, key: Atom) -> Value {
        let found = self.selector_intrinsic_slot(owner, key).expect("bootstrap intrinsic is readable");
        let (value, flags) = found.unwrap_or((Value::UNDEFINED, 0));
        // Accessor wrappers are mutable: defineProperty can replace their
        // getter in place. Root the original getter itself, not that wrapper.
        let saved_value = if flags & ACCESSOR != 0 {
            let Kind::Accessor { get, set } = &self.objects[value.index()].kind else {
                panic!("bootstrap accessor must have an accessor wrapper");
            };
            assert!(set.is_undefined(), "guarded bootstrap accessors have no setter");
            *get
        } else { value };
        assert!(self.selector_intrinsics.len() < MAX_INTRINSIC_SLOTS);
        self.selector_intrinsics.push(SelectorIntrinsic { owner, key, value: saved_value, flags, present: found.is_some() });
        value
    }

    pub(crate) fn capture_selector_intrinsics(&mut self) {
        assert!(self.selector_intrinsics.is_empty());
        self.selector_intrinsics = Vec::with_capacity(MAX_INTRINSIC_SLOTS);
        let global = Value::object(self.global);
        let mut constructors = Vec::new();
        for name in ["Object", "String", "Array", "Map", "Set", "WeakRef", "RegExp", "JSON"] {
            let key = self.atom(name);
            constructors.push(self.capture_selector_slot(global, key));
        }
        let number = self.atom("Number"); self.capture_selector_slot(global, number);
        for &constructor in &constructors[..7] {
            self.capture_selector_slot(constructor, crate::text::PROTOTYPE);
        }
        let (object, array, json) = (constructors[0], constructors[2], constructors[7]);
        for name in ["getOwnPropertyDescriptor", "keys", "entries", "assign"] {
            let key = self.atom(name); self.capture_selector_slot(object, key);
        }
        let key = self.atom("isArray"); self.capture_selector_slot(array, key);
        for (owner, names) in [
            (self.realm.array_proto, &["includes", "some", "filter", "push", "slice", "every", "map"][..]),
            (self.realm.string_proto, &["split", "startsWith", "endsWith", "includes", "toLowerCase", "trim", "replace"][..]),
            (self.realm.map_proto, &["get", "set", "has", "values", "clear", "size"][..]),
            (self.realm.set_proto, &["add", "has", "size"][..]),
            (self.realm.weak_ref_proto, &["deref"][..]),
            (self.realm.regexp_proto, &["exec", "test"][..]),
            (self.realm.function_proto, &["call"][..]),
            (self.realm.object_proto, &["hasOwnProperty"][..]),
        ] {
            for name in names { let key = self.atom(name); self.capture_selector_slot(Value::object(owner), key); }
        }
        for owner in [self.realm.array_proto, self.realm.string_proto, self.realm.map_proto, self.realm.set_proto] {
            self.capture_selector_slot(Value::object(owner), crate::text::SYMBOL_ITERATOR);
        }
        for key in [crate::text::SYMBOL_SPLIT, crate::text::SYMBOL_MATCH, crate::text::SYMBOL_MATCH_ALL,
            crate::text::SYMBOL_REPLACE, crate::text::SYMBOL_SEARCH]
        { self.capture_selector_slot(Value::object(self.realm.regexp_proto), key); }
        let key = self.atom("stringify"); self.capture_selector_slot(json, key);
        let to_json = self.atom("toJSON");
        for owner in [self.realm.object_proto, self.realm.array_proto, self.realm.string_proto] {
            let value = self.capture_selector_slot(Value::object(owner), to_json);
            assert!(value.is_undefined(), "bootstrap toJSON must be absent");
        }
        // filter/slice and regexp split consult constructor/species protocols.
        for (prototype, constructor) in [(self.realm.array_proto, constructors[2]), (self.realm.regexp_proto, constructors[6])] {
            self.capture_selector_slot(Value::object(prototype), crate::text::CONSTRUCTOR);
            self.capture_selector_slot(constructor, crate::text::SYMBOL_SPECIES);
        }
        let next = self.atom("next");
        self.capture_selector_slot(Value::object(self.realm.native_iterator_proto), next);
        self.capture_selector_slot(Value::object(self.realm.iterator_proto), crate::text::SYMBOL_ITERATOR);
        let close = self.atom("return");
        self.capture_selector_slot(Value::object(self.realm.native_iterator_proto), close);
    }

    /// Whether the public primitives the selector path uses still have their
    /// exact bootstrap descriptors, including canonical species accessors and
    /// absence of inherited toJSON hooks. Never invokes public getters/traps.
    pub fn data_surface_intrinsics_kept(&mut self) -> bool {
        if self.selector_intrinsics.is_empty() { return false; }
        for index in 0..self.selector_intrinsics.len() {
            let expected = self.selector_intrinsics[index];
            let Ok(found) = self.selector_intrinsic_slot(expected.owner, expected.key) else { return false; };
            match (found, expected.present) {
                (None, false) => {},
                (Some((value, flags)), true) if flags == expected.flags => {
                    if flags & ACCESSOR == 0 {
                        if value != expected.value { return false; }
                    } else {
                        if !value.is_object() { return false; }
                        // Restoring an accessor descriptor creates a new internal
                        // wrapper; its original getter/setter identities suffice.
                        let Kind::Accessor { get, set } = &self.objects[value.index()].kind else { return false; };
                        if *get != expected.value || !set.is_undefined() { return false; }
                    }
                },
                _ => return false,
            }
        }
        true
    }

    /// Copies a dense genuine Array without reading prototype slots or accessors.
    /// Indexed descriptors stored outside the dense elements are unsupported.
    pub fn data_surface_array(&self, value: Value, max_len: usize) -> Result<Vec<Value>, DataSurfaceError> {
        if !value.is_object() { return Err(DataSurfaceError::Unsupported); }
        let object = &self.objects[value.index()];
        let Kind::Array(items) = &object.kind else { return Err(DataSurfaceError::Unsupported); };
        if items.len() > max_len { return Err(DataSurfaceError::LimitExceeded); }
        if self.shapes.is_dict(object.shape)
            || self.shapes.items[object.shape as usize].count > MAX_DESCRIPTOR_PROPERTIES
        { return Err(DataSurfaceError::Unsupported); }
        if items.iter().any(|value| value.is_empty())
            || self.shapes.entries(object.shape).iter().any(|(key, _, flags)| {
                flags & ACCESSOR != 0 || self.atoms.index_of(*key).is_some()
            })
        { return Err(DataSurfaceError::Unsupported); }
        Ok(items.to_vec())
    }

    /// Returns a genuine WeakRef target without invoking an overridden deref.
    /// Like JS WeakRef.deref, an alive target is kept only until this job ends.
    pub fn data_surface_weak_target(&mut self, value: Value) -> Result<Option<Value>, DataSurfaceError> {
        if !value.is_object() { return Err(DataSurfaceError::Unsupported); }
        let Kind::WeakRef(target) = self.objects[value.index()].kind else { return Err(DataSurfaceError::Unsupported); };
        if target.is_undefined() { return Ok(None); }
        self.keep_during_job(target);
        Ok(Some(target))
    }

    /// Makes an intrinsic WeakRef to an object, without consulting JS constructors.
    /// It is a newly-created native Value; root it before keeping it across JS.
    pub fn data_surface_weak_ref(&mut self, target: Value) -> Result<Value, DataSurfaceError> {
        if !target.is_object() { return Err(DataSurfaceError::Unsupported); }
        self.keep_during_job(target);
        Ok(self.new_object_with_proto(self.realm.weak_ref_proto, Kind::WeakRef(target)))
    }

    /// Object.is for supported primitive snapshot values, without user code.
    pub fn data_surface_same_primitive(&self, first: Value, second: Value) -> bool {
        primitive(first) && primitive(second) && self.same(first, second)
    }

    /// Preflights the exact conservative charge used by copy_primitive, without
    /// copying or allocating. Unsupported types/overlong strings decline here.
    pub fn data_surface_primitive_cost(&self, value: Value) -> Result<usize, DataSurfaceError> {
        if !primitive(value) { return Err(DataSurfaceError::Unsupported); }
        if !value.is_string() { return Ok(8); }
        let units = self.text_of(value).len();
        if units > MAX_STRING_UNITS { return Err(DataSurfaceError::LimitExceeded); }
        Ok(8 + 32 + units * 2)
    }

    /// Copies supported primitives into a bounded receipt. Strings are checked
    /// BEFORE copying, limited to 4096 UTF-16 units, and deep-copied to flat fresh
    /// backing (including lone surrogates). Budget charges 8 bytes per value plus
    /// a conservative 32-byte string record and two bytes per code unit.
    /// Failures leave the budget unchanged. New strings live in the normal native
    /// nursery; root a completed/partial receipt before running JS or collecting.
    pub fn data_surface_copy_primitive(&mut self, value: Value, remaining_bytes: &mut usize) -> Result<Value, DataSurfaceError> {
        let cost = self.data_surface_primitive_cost(value)?;
        if cost > *remaining_bytes { return Err(DataSurfaceError::LimitExceeded); }
        if !value.is_string() {
            *remaining_bytes -= cost;
            return Ok(value);
        }
        let text = self.text_of(value);
        // Text::clone deep-copies Ascii/Wide and flattens Growing buffers.
        let copied = text.clone();
        *remaining_bytes -= cost;
        Ok(self.new_text(copied))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Engine, Root};

    fn eval(engine: &mut Engine, code: &str) -> Root {
        let value=engine.eval(code,"data-surface-test.js").unwrap();
        engine.vm().root(value)
    }

    #[test]
    fn own_missing_undefined_enumerability_and_safe_prototypes_are_distinct() {
        let mut engine=Engine::new();
        let object=eval(&mut engine,r#"
            globalThis.calls=0;
            const proto={ inherited:7,get trap(){calls++;return 99} };
            const object=Object.create(proto);
            Object.defineProperty(object,'hidden',{value:undefined,enumerable:false});
            object.id='same';object
        "#);
        let vm=engine.vm();
        let (id,hidden,missing,inherited,trap)=(vm.atom("id"),vm.atom("hidden"),vm.atom("missing"),vm.atom("inherited"),vm.atom("trap"));
        assert!(vm.data_surface_plain(object.get()));
        let own=vm.data_surface_own(object.get(),id).unwrap().unwrap();
        assert!(own.enumerable);assert_eq!(vm.string_text(own.value).unwrap(),"same");
        assert_eq!(vm.data_surface_own(object.get(),hidden),Ok(Some(DataProperty{value:Value::UNDEFINED,enumerable:false})));
        assert_eq!(vm.data_surface_own(object.get(),missing),Ok(None));
        assert_eq!(vm.data_surface_own(object.get(),inherited),Ok(None));
        assert_eq!(vm.data_surface_lookup(object.get(),inherited).unwrap().unwrap().value,Value::int(7));
        assert_eq!(vm.data_surface_lookup(object.get(),missing),Ok(None));
        assert_eq!(vm.data_surface_lookup(object.get(),trap),Err(DataSurfaceError::Unsupported));
        let calls=eval(&mut engine,"calls");assert_eq!(calls.get(),Value::int(0));
    }

    #[test]
    fn accessors_proxies_exotics_and_deep_prototypes_decline_without_running_code() {
        let mut engine=Engine::new();
        let getter=eval(&mut engine,"globalThis.calls=0;({stable:3,get hidden(){calls++;return 1}})");
        let proxy=eval(&mut engine,"new Proxy({stable:3},{get(){calls++;return 1},ownKeys(){calls++;return []},getPrototypeOf(){calls++;return null}})");
        let deep=eval(&mut engine,"let deep=Object.create(null);for(let n=0;n<65;n++)deep=Object.create(deep);deep");
        let vm=engine.vm();let stable=vm.atom("stable");let hidden=vm.atom("hidden");
        assert!(!vm.data_surface_plain(getter.get()));assert!(!vm.data_surface_plain(proxy.get()));
        assert_eq!(vm.data_surface_own(getter.get(),hidden),Err(DataSurfaceError::Unsupported));
        assert_eq!(vm.data_surface_own(proxy.get(),stable),Err(DataSurfaceError::Unsupported));
        assert_eq!(vm.data_surface_lookup(proxy.get(),stable),Err(DataSurfaceError::Unsupported));
        assert_eq!(vm.data_surface_lookup(deep.get(),stable),Err(DataSurfaceError::LimitExceeded));
        let calls=eval(&mut engine,"calls");assert_eq!(calls.get(),Value::int(0));
    }

    #[test]
    fn ordinary_dictionary_bags_after_delete_are_certified_without_getters() {
        let mut engine=Engine::new();
        let bag=eval(&mut engine,"let bag={id:'kept',style:{},value:undefined};for(let i=0;i<40;i++)bag['f'+i]=i;delete bag.style;bag");
        let accessor=eval(&mut engine,"globalThis.calls=0;let b={style:1};delete b.style;Object.defineProperty(b,'hidden',{get(){calls++;return 1},configurable:true});b");
        let removed=eval(&mut engine,"let b={get hidden(){calls++;return 1},id:'kept'};delete b.hidden;b");
        let huge=eval(&mut engine,"let b={};for(let i=0;i<6000;i++)b['p'+i]=i;delete b.p0;b");
        let vm=engine.vm();
        assert!(vm.shapes.is_dict(vm.objects[bag.get().index()].shape));
        assert!(vm.data_surface_plain(bag.get()));
        assert!(!vm.data_surface_plain(accessor.get()));
        assert!(vm.data_surface_plain(removed.get()));
        assert!(!vm.data_surface_plain(huge.get()));
        let id=vm.atom("id");let value=vm.atom("value");let style=vm.atom("style");
        let id_value=vm.data_surface_own(bag.get(),id).unwrap().unwrap().value;
        assert_eq!(vm.string_text(id_value).unwrap(),"kept");
        assert_eq!(vm.data_surface_own(bag.get(),value).unwrap().unwrap().value,Value::UNDEFINED);
        assert_eq!(vm.data_surface_own(bag.get(),style),Ok(None));
        assert_eq!(eval(&mut engine,"calls").get(),Value::int(0));
    }

    #[test]
    fn genuine_maps_preserve_insertion_order_and_reject_overrides_and_caps() {
        let mut engine=Engine::new();
        let map=eval(&mut engine,r#"
            globalThis.calls=0;globalThis.mapping=new Map([['a',1],['b',2]]);
            mapping.delete('a');mapping.set('a',3);
            mapping.entries=()=>{calls++;throw new Error('do not call')};mapping
        "#);
        let proxy=eval(&mut engine,"new Proxy(mapping,{get(){calls++;throw new Error('do not call')}})");
        let vm=engine.vm();
        let rows=vm.data_surface_map(map.get(),2).unwrap();
        assert_eq!(rows.len(),2);
        assert_eq!(vm.string_text(rows[0].0).unwrap(),"b");assert_eq!(rows[0].1,Value::int(2));
        assert_eq!(vm.string_text(rows[1].0).unwrap(),"a");assert_eq!(rows[1].1,Value::int(3));
        assert_eq!(vm.data_surface_map(map.get(),1),Err(DataSurfaceError::LimitExceeded));
        assert_eq!(vm.data_surface_map(proxy.get(),2),Err(DataSurfaceError::Unsupported));
        let calls=eval(&mut engine,"calls");assert_eq!(calls.get(),Value::int(0));
    }

    #[test]
    fn dense_arrays_decline_holes_index_accessors_and_oversized_descriptor_surfaces() {
        let mut engine=Engine::new();
        let array=eval(&mut engine,"[1,undefined,3]");
        let hole=eval(&mut engine,"[1,,3]");
        let getter=eval(&mut engine,"globalThis.calls=0;let a=[1];Object.defineProperty(a,'0',{get(){calls++;return 1}});a");
        let huge=eval(&mut engine,"let a=[1];for(let i=0;i<6000;i++)Object.defineProperty(a,'p'+i,{value:i,configurable:true});a");
        let vm=engine.vm();
        assert_eq!(vm.data_surface_array(array.get(),3).unwrap(),[Value::int(1),Value::UNDEFINED,Value::int(3)]);
        assert_eq!(vm.data_surface_array(array.get(),2),Err(DataSurfaceError::LimitExceeded));
        for bad in [hole.get(),getter.get(),huge.get()] {assert_eq!(vm.data_surface_array(bad,3),Err(DataSurfaceError::Unsupported));}
        let calls=eval(&mut engine,"calls");assert_eq!(calls.get(),Value::int(0));
    }

    #[test]
    fn weak_targets_survive_the_current_job_but_not_a_later_collection() {
        let mut engine=Engine::new();
        let vm=engine.vm();let mark=vm.new_values_mark();
        let target=vm.new_object();let weak=vm.data_surface_weak_ref(target).unwrap();let weak=vm.root(weak);
        assert_eq!(vm.data_surface_weak_target(weak.get()),Ok(Some(target)));
        vm.release_new_values(mark);vm.collect_garbage_now();
        assert_eq!(vm.data_surface_weak_target(weak.get()),Ok(Some(target)));
        vm.clear_kept_objects();vm.collect_garbage_now();
        assert_eq!(vm.data_surface_weak_target(weak.get()),Ok(None));
        assert_eq!(vm.data_surface_weak_target(Value::int(3)),Err(DataSurfaceError::Unsupported));
    }

    #[test]
    fn primitive_snapshots_preserve_numeric_bits_and_exact_flat_utf16_with_bounds() {
        let mut engine=Engine::new();let vm=engine.vm();
        let mut budget=1<<20;
        for value in [Value::UNDEFINED,Value::NULL,Value::TRUE,Value::number(f64::NAN),Value::number(-0.0)] {
            let copy=vm.data_surface_copy_primitive(value,&mut budget).unwrap();
            assert!(vm.data_surface_same_primitive(value,copy));
        }
        assert!(!vm.data_surface_same_primitive(Value::number(-0.0),Value::number(0.0)));
        let units=[0xd800,65,0xdc00];let original=vm.new_text(crate::text::Text::from_units(&units));
        let copy=vm.data_surface_copy_primitive(original,&mut budget).unwrap();let copy=vm.root(copy);
        assert_ne!(original,copy.get());assert!(vm.data_surface_same_primitive(original,copy.get()));
        assert_eq!(vm.text_of(copy.get()).units(),units);
        let boundary=vm.new_str(&"a".repeat(4096));
        let cost=vm.data_surface_primitive_cost(boundary).unwrap();let mut exact=cost;
        assert!(vm.data_surface_copy_primitive(boundary,&mut exact).is_ok());assert_eq!(exact,0);
        let mut short=cost-1;assert_eq!(vm.data_surface_copy_primitive(boundary,&mut short),Err(DataSurfaceError::LimitExceeded));assert_eq!(short,cost-1);
        let over=vm.new_str(&"b".repeat(4097));let before=budget;
        assert_eq!(vm.data_surface_copy_primitive(over,&mut budget),Err(DataSurfaceError::LimitExceeded));assert_eq!(budget,before);
        let object=vm.new_object();assert_eq!(vm.data_surface_copy_primitive(object,&mut budget),Err(DataSurfaceError::Unsupported));
        vm.release_new_values(0);vm.collect_garbage_now();assert_eq!(vm.text_of(copy.get()).units(),units);
    }

    #[test]
    fn growing_text_is_copied_without_retaining_the_shared_buffer() {
        let mut engine=Engine::new();
        let original=eval(&mut engine,"let text='';for(let i=0;i<600;i++)text+='x';text");
        let vm=engine.vm();let mut budget=1<<20;
        let copy=vm.data_surface_copy_primitive(original.get(),&mut budget).unwrap();
        assert!(vm.data_surface_same_primitive(original.get(),copy));
        assert!(matches!(vm.text_of(copy),crate::text::Text::Ascii(_)),"copy must be flat owned backing");
        let copy=vm.root(copy);drop(original);vm.release_new_values(0);vm.collect_garbage_now();
        assert_eq!(vm.text_of(copy.get()).len(),600);
    }

    #[test]
    fn direct_map_get_uses_same_value_zero_and_sets_ignore_user_iterators() {
        let mut engine=Engine::new();
        let map=eval(&mut engine,"new Map([[NaN,1],[-0,2],['name',3]])");
        let set=eval(&mut engine,"globalThis.calls=0;globalThis.values=new Set(['a','b']);values[Symbol.iterator]=()=>{calls++;throw Error('no')};values");
        let vm=engine.vm();let key=vm.new_str("name");
        assert_eq!(vm.data_surface_map_get(map.get(),Value::number(f64::NAN)).unwrap(),Some(Value::int(1)));
        assert_eq!(vm.data_surface_map_get(map.get(),Value::number(0.0)).unwrap(),Some(Value::int(2)));
        assert_eq!(vm.data_surface_map_get(map.get(),key).unwrap(),Some(Value::int(3)));
        let missing=vm.new_str("missing");assert_eq!(vm.data_surface_map_get(map.get(),missing),Ok(None));
        let values=vm.data_surface_set(set.get(),2).unwrap();
        assert_eq!(vm.string_text(values[0]).unwrap(),"a");assert_eq!(vm.string_text(values[1]).unwrap(),"b");
        assert_eq!(vm.data_surface_set(set.get(),1),Err(DataSurfaceError::LimitExceeded));
        let calls=eval(&mut engine,"calls");assert_eq!(calls.get(),Value::int(0));
    }

    #[test]
    fn bootstrap_intrinsics_detect_overrides_before_any_host_module_and_restore_without_execution() {
        for (owner,key) in [
            ("globalThis","'String'"),("globalThis","'Number'"),
            ("Array.prototype","'some'"),("Array.prototype","Symbol.iterator"),
            ("String.prototype","'split'"),("String.prototype","'replace'"),
            ("Map.prototype","'get'"),("Map.prototype","'size'"),
            ("Set.prototype","'has'"),("Set.prototype","'size'"),
            ("WeakRef.prototype","'deref'"),("JSON","'stringify'"),
            ("Object.prototype","'toJSON'"),("Array.prototype","'toJSON'"),
            ("String.prototype","'toJSON'"),("RegExp.prototype","'exec'"),
            ("RegExp.prototype","Symbol.split"),("RegExp","Symbol.species"),
            ("Array","Symbol.species"),("Object.getPrototypeOf([][Symbol.iterator]())","'next'"),
            ("Object.getPrototypeOf([][Symbol.iterator]())","'return'"),
        ] {
            let mut engine=Engine::new();assert!(engine.vm().data_surface_intrinsics_kept());
            eval(&mut engine,&format!(r#"
                globalThis.calls=0;globalThis.owner={owner};globalThis.key={key};
                globalThis.saved=Object.getOwnPropertyDescriptor(owner,key);
                Object.defineProperty(owner,key,{{configurable:true,get(){{calls++;return undefined}}}});
            "#));
            assert!(!engine.vm().data_surface_intrinsics_kept(),"override {owner}[{key}] must decline");
            let calls=eval(&mut engine,"calls");assert_eq!(calls.get(),Value::int(0));
            engine.vm().release_new_values(0);engine.vm().collect_garbage_now();
            eval(&mut engine,"if(saved)Object.defineProperty(owner,key,saved);else delete owner[key]");
            assert!(engine.vm().data_surface_intrinsics_kept(),"restored {owner}[{key}]");
        }
    }

    #[test]
    fn canonical_intrinsic_roots_survive_global_replacement_and_gc() {
        let mut engine=Engine::new();let vm=engine.vm();
        assert!(vm.data_surface_intrinsics_kept());
        assert!(vm.selector_intrinsics.len()<=MAX_INTRINSIC_SLOTS);
        assert!(std::mem::size_of::<SelectorIntrinsic>()*MAX_INTRINSIC_SLOTS<=2048);
        let name=vm.atom("JSON");let global=Value::object(vm.global);
        // This value already has a permanent canonical-registry root. It is not
        // rooted independently by the test, so collection exercises that root.
        let original=vm.data_surface_own(global,name).unwrap().unwrap().value;
        vm.set_property(global,name,Value::NULL).unwrap();
        vm.release_new_values(0);vm.clear_kept_objects();vm.collect_garbage_now();
        assert!(!vm.data_surface_intrinsics_kept());
        assert!(matches!(vm.objects[original.index()].kind,Kind::Plain));
        vm.set_property(global,name,original).unwrap();
        assert!(vm.data_surface_intrinsics_kept());
    }
}
