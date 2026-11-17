//! `Object`, `Function` and `Symbol`.

use crate::object::*;
use crate::text::{self, Atom, AtomKind, Text};
use crate::value::Value;
use crate::vm::{Args, Run, Vm};

impl Vm {
    pub(crate) fn install_object(&mut self) {
        let proto = self.realm.object_proto;
        let object = self.constructor("Object", 1, object_constructor, proto);
        self.method(proto, "hasOwnProperty", 1, has_own_property);
        self.method(proto, "isPrototypeOf", 1, is_prototype_of);
        self.method(proto, "propertyIsEnumerable", 1, property_is_enumerable);
        self.method(proto, "toString", 0, object_to_string);
        self.method(proto, "toLocaleString", 0, object_to_string);
        self.method(proto, "valueOf", 0, object_value_of);
        let proto_atom = self.intern_str("__proto__");
        let get_name = self.intern_str("get __proto__");
        let set_name = self.intern_str("set __proto__");
        let getter = self.new_native(get_name, 0, get_proto_accessor, Value::UNDEFINED, false);
        let setter = self.new_native(set_name, 1, set_proto_accessor, Value::UNDEFINED, false);
        self.define_accessor(proto, proto_atom, Some(getter), Some(setter), CONFIGURABLE);

        self.method(object, "keys", 1, object_keys);
        self.method(object, "values", 1, object_values);
        self.method(object, "entries", 1, object_entries);
        self.method(object, "assign", 2, object_assign);
        self.method(object, "freeze", 1, object_freeze);
        self.method(object, "isFrozen", 1, object_is_frozen);
        self.method(object, "seal", 1, object_seal);
        self.method(object, "isSealed", 1, object_is_sealed);
        self.method(object, "preventExtensions", 1, object_prevent_extensions);
        self.method(object, "isExtensible", 1, object_is_extensible);
        self.method(object, "create", 2, object_create);
        self.method(object, "getPrototypeOf", 1, object_get_prototype_of);
        self.method(object, "setPrototypeOf", 2, object_set_prototype_of);
        self.method(object, "defineProperty", 3, object_define_property);
        self.method(object, "defineProperties", 2, object_define_properties);
        self.method(
            object,
            "getOwnPropertyDescriptor",
            2,
            object_get_own_descriptor,
        );
        self.method(
            object,
            "getOwnPropertyDescriptors",
            1,
            object_get_own_descriptors,
        );
        self.method(object, "getOwnPropertyNames", 1, object_get_own_names);
        self.method(object, "getOwnPropertySymbols", 1, object_get_own_symbols);
        self.method(object, "fromEntries", 1, object_from_entries);
        self.method(object, "is", 2, object_is);
        self.method(object, "hasOwn", 2, object_has_own);
        self.method(object, "groupBy", 2, object_group_by);
    }

    /// `call` or `apply`, marked as only calling another function (see [`NativeData::forwards`]).
    fn forwarding_method(&mut self, object: u32, name: &str, length: u8, func: NativeFn, forwards: u8) {
        let atom = self.intern_str(name);
        let function = self.new_native(atom, length, func, Value::UNDEFINED, false);
        if let Kind::Native(native) = &mut self.objects[function.index()].kind {
            native.forwards = forwards;
        }
        self.add_property(object, atom, function, HIDDEN);
    }

    pub(crate) fn install_function(&mut self) {
        let proto = self.realm.function_proto;
        self.constructor("Function", 1, function_constructor, proto);
        self.forwarding_method(proto, "call", 1, function_call, FORWARDS_CALL);
        self.forwarding_method(proto, "apply", 2, function_apply, FORWARDS_APPLY);
        self.method(proto, "bind", 1, function_bind);
        self.method(proto, "toString", 0, function_to_string);
        self.function_has_instance = self.symbol_method(
            proto,
            text::SYMBOL_HAS_INSTANCE,
            "[Symbol.hasInstance]",
            1,
            function_has_instance,
        );

        let symbol_proto = self.plain_proto();
        self.realm.symbol_proto = symbol_proto;
        let symbol = self.constructor("Symbol", 0, symbol_constructor, symbol_proto);
        for (name, atom) in [
            ("iterator", text::SYMBOL_ITERATOR),
            ("hasInstance", text::SYMBOL_HAS_INSTANCE),
            ("toPrimitive", text::SYMBOL_TO_PRIMITIVE),
            ("toStringTag", text::SYMBOL_TO_STRING_TAG),
            ("asyncIterator", text::SYMBOL_ASYNC_ITERATOR),
            ("match", text::SYMBOL_MATCH),
            ("matchAll", text::SYMBOL_MATCH_ALL),
            ("replace", text::SYMBOL_REPLACE),
            ("search", text::SYMBOL_SEARCH),
            ("split", text::SYMBOL_SPLIT),
            ("species", text::SYMBOL_SPECIES),
            ("unscopables", text::SYMBOL_UNSCOPABLES),
            ("isConcatSpreadable", text::SYMBOL_IS_CONCAT_SPREADABLE),
            ("dispose", text::SYMBOL_DISPOSE),
            ("asyncDispose", text::SYMBOL_ASYNC_DISPOSE),
        ] {
            self.constant(symbol, name, Value::symbol(atom.0));
        }
        self.method(symbol, "for", 1, symbol_for);
        self.method(symbol, "keyFor", 1, symbol_key_for);
        self.method(symbol_proto, "toString", 0, symbol_to_string);
        self.method(symbol_proto, "valueOf", 0, symbol_value_of);
        self.getter(symbol_proto, "description", symbol_description);
    }

    /// The text of what a value is, from `Object.prototype.toString`.
    pub(crate) fn class_name(&self, object: u32) -> &'static str {
        match &self.objects[object as usize].kind {
            Kind::Array(_) => "Array",
            Kind::Arguments(_) => "Arguments",
            Kind::Function { .. } | Kind::Native(_) | Kind::Bound(_) => "Function",
            Kind::Error => "Error",
            Kind::Boxed(value) => {
                if value.is_string() {
                    "String"
                } else if value.is_number() {
                    "Number"
                } else if value.is_bool() {
                    "Boolean"
                } else {
                    "Object"
                }
            }
            Kind::Date(_) => "Date",
            Kind::RegExp(_) => "RegExp",
            Kind::Proxy(data) if data.target.is_object() => self.class_name(data.target.handle()),
            _ => "Object",
        }
    }

    pub(crate) fn descriptor_checked(&mut self, object: u32, key: Atom) -> Run<Value> {
        if matches!(self.objects[object as usize].kind, Kind::Proxy(_)) {
            return self.proxy_descriptor(object, key);
        }
        self.materialize_if_lazy(object);
        let special = match &self.objects[object as usize].kind {
            Kind::Array(_) | Kind::Arguments(_) | Kind::Boxed(_) | Kind::TypedArray(_) => {
                if self.has_own_special(object, key) {
                    let target = Value::object(object);
                    let value = self.get(target, key)?;
                    let frozen = self.objects[object as usize].flags & FROZEN_ELEMENTS != 0;
                    let is_length = key == text::LENGTH;
                    Some((
                        value,
                        if is_length {
                            if frozen || self.objects[object as usize].flags & READ_ONLY_LENGTH != 0
                            {
                                0
                            } else {
                                WRITABLE
                            }
                        } else if frozen {
                            ENUMERABLE
                        } else {
                            PLAIN
                        },
                    ))
                } else {
                    None
                }
            }
            _ => None,
        };
        let result = self.new_plain();
        let handle = result.handle();
        if let Some((value, flags)) = special {
            self.define(handle, text::VALUE, value, PLAIN);
            self.define(
                handle,
                text::WRITABLE,
                Value::bool(flags & WRITABLE != 0),
                PLAIN,
            );
            self.define(
                handle,
                text::ENUMERABLE,
                Value::bool(flags & ENUMERABLE != 0),
                PLAIN,
            );
            self.define(
                handle,
                text::CONFIGURABLE,
                Value::bool(flags & CONFIGURABLE != 0),
                PLAIN,
            );
            return Ok(result);
        }
        let shape = self.objects[object as usize].shape;
        let Some((slot, flags)) = self.shapes.lookup(shape, key) else {
            return Ok(Value::UNDEFINED);
        };
        let value = self.objects[object as usize].slots()[slot as usize];
        if flags & ACCESSOR != 0 {
            let Kind::Accessor { get, set } = self.objects[value.index()].kind else {
                unreachable!("an accessor property holds an accessor");
            };
            self.define(handle, text::GET, get, PLAIN);
            self.define(handle, text::SET, set, PLAIN);
        } else {
            self.define(handle, text::VALUE, value, PLAIN);
            self.define(
                handle,
                text::WRITABLE,
                Value::bool(flags & WRITABLE != 0),
                PLAIN,
            );
        }
        self.define(
            handle,
            text::ENUMERABLE,
            Value::bool(flags & ENUMERABLE != 0),
            PLAIN,
        );
        self.define(
            handle,
            text::CONFIGURABLE,
            Value::bool(flags & CONFIGURABLE != 0),
            PLAIN,
        );
        Ok(result)
    }

    fn has_own_special(&mut self, object: u32, key: Atom) -> bool {
        let shape = self.objects[object as usize].shape;
        self.has_own(object, key) && self.shapes.lookup(shape, key).is_none()
    }

    pub(crate) fn define_from_descriptor(
        &mut self,
        object: u32,
        key: Atom,
        descriptor: Value,
    ) -> Run<()> {
        if !descriptor.is_object() {
            let shown = self.brief(descriptor);
            return Err(
                self.type_error(&format!("Property description must be an object: {shown}"))
            );
        }
        if matches!(self.objects[object as usize].kind, Kind::Proxy(_)) {
            return self.proxy_define(object, key, descriptor);
        }
        self.materialize_if_lazy(object);
        let shape = self.objects[object as usize].shape;
        let mut existing = self.shapes.lookup(shape, key);
        if let (Kind::Array(_), true) = (&self.objects[object as usize].kind, key == text::LENGTH) {
            return self.define_array_length(object, descriptor);
        }
        // An element of an array that's just a value in the array acts like a plain property here.
        let element_value = if existing.is_none() {
            self.own_element(object, key)
        } else {
            None
        };
        if element_value.is_none() && existing.is_none() {
            let is_array_add = matches!(self.objects[object as usize].kind, Kind::Array(_))
                && self.atoms.index_of(key).is_some();
            if self.objects[object as usize].flags & READ_ONLY_LENGTH != 0 && is_array_add {
                let index = self.atoms.index_of(key).unwrap_or(0);
                let Kind::Array(items) = &self.objects[object as usize].kind else {
                    unreachable!("just checked");
                };
                if index as usize >= items.len() {
                    return Err(self.type_error("Cannot add an element past a fixed length"));
                }
            }
        }
        if let Some((_, flags)) = existing {
            if flags & CONFIGURABLE == 0 {
                self.check_redefinition(descriptor, object, key, flags)?;
            }
        } else if element_value.is_none()
            && self.objects[object as usize].flags & NOT_EXTENSIBLE != 0
        {
            let name = self.atom_text(key);
            return Err(self.type_error(&format!(
                "Cannot define property {name}, object is not extensible"
            )));
        }
        let old_flags = if element_value.is_some() {
            existing = None;
            PLAIN
        } else {
            existing.map_or(0, |(_, flags)| flags & !ACCESSOR)
        };
        let d = descriptor.handle();
        let mut flags = old_flags;
        for (name, bit) in [
            (text::ENUMERABLE, ENUMERABLE),
            (text::CONFIGURABLE, CONFIGURABLE),
            (text::WRITABLE, WRITABLE),
        ] {
            if self.has_property(d, name) {
                let value = self.get(descriptor, name)?;
                if self.to_boolean(value) {
                    flags |= bit;
                } else {
                    flags &= !bit;
                }
            }
        }
        let has_get = self.has_property(d, text::GET);
        let has_set = self.has_property(d, text::SET);
        if has_get || has_set {
            let get = self.get(descriptor, text::GET)?;
            let set = self.get(descriptor, text::SET)?;
            for function in [get, set] {
                if !function.is_undefined() && !self.is_callable(function) {
                    return Err(self.type_error("A getter or setter must be a function"));
                }
            }
            if self.has_property(d, text::VALUE) || self.has_property(d, text::WRITABLE) {
                return Err(self.type_error(
                    "Invalid property descriptor. Cannot both specify accessors and a value or writable attribute",
                ));
            }
            flags &= !WRITABLE;
            self.define_accessor(
                object,
                key,
                has_get.then_some(get),
                has_set.then_some(set),
                flags,
            );
            return Ok(());
        }
        let was_accessor = matches!(existing, Some((_, found)) if found & ACCESSOR != 0);
        if self.has_property(d, text::VALUE) || !was_accessor {
            let value = if self.has_property(d, text::VALUE) {
                self.get(descriptor, text::VALUE)?
            } else if let Some(element) = element_value {
                element
            } else if let Some((slot, _)) = existing {
                self.objects[object as usize].slots()[slot as usize]
            } else {
                Value::UNDEFINED
            };
            self.define(object, key, value, flags);
        } else {
            self.change_flags(object, key, flags | ACCESSOR);
        }
        Ok(())
    }

    /// The value of an element that's stored plainly in an array.
    fn own_element(&mut self, object: u32, key: Atom) -> Option<Value> {
        let index = self.atoms.index_of(key)? as usize;
        match &self.objects[object as usize].kind {
            Kind::Array(items) | Kind::Arguments(items) => {
                items.get(index).copied().filter(|item| !item.is_empty())
            }
            _ => None,
        }
    }

    /// A property that can't be configured can only be changed in a few ways, or not changed at all.
    fn check_redefinition(
        &mut self,
        descriptor: Value,
        object: u32,
        key: Atom,
        flags: u8,
    ) -> Run<()> {
        let d = descriptor.handle();
        let refuse = |vm: &mut Vm| -> Run<()> {
            let name = vm.atom_text(key);
            Err(vm.type_error(&format!("Cannot redefine property: {name}")))
        };
        let wants_configurable = self.has_property(d, text::CONFIGURABLE);
        if wants_configurable {
            let value = self.get(descriptor, text::CONFIGURABLE)?;
            if self.to_boolean(value) {
                return refuse(self);
            }
        }
        if self.has_property(d, text::ENUMERABLE) {
            let value = self.get(descriptor, text::ENUMERABLE)?;
            if self.to_boolean(value) != (flags & ENUMERABLE != 0) {
                return refuse(self);
            }
        }
        let wants_accessor = self.has_property(d, text::GET) || self.has_property(d, text::SET);
        let wants_data = self.has_property(d, text::VALUE) || self.has_property(d, text::WRITABLE);
        if flags & ACCESSOR != 0 {
            if wants_data {
                return refuse(self);
            }
            let shape = self.objects[object as usize].shape;
            let Some((slot, _)) = self.shapes.lookup(shape, key) else {
                return Ok(());
            };
            let holder = self.objects[object as usize].slots()[slot as usize];
            let Kind::Accessor { get, set } = self.objects[holder.index()].kind else {
                return Ok(());
            };
            if self.has_property(d, text::GET) {
                let new_get = self.get(descriptor, text::GET)?;
                if !self.same_value(new_get, get) {
                    return refuse(self);
                }
            }
            if self.has_property(d, text::SET) {
                let new_set = self.get(descriptor, text::SET)?;
                if !self.same_value(new_set, set) {
                    return refuse(self);
                }
            }
            return Ok(());
        }
        if wants_accessor {
            return refuse(self);
        }
        if flags & WRITABLE == 0 {
            if self.has_property(d, text::WRITABLE) {
                let value = self.get(descriptor, text::WRITABLE)?;
                if self.to_boolean(value) {
                    return refuse(self);
                }
            }
            if self.has_property(d, text::VALUE) {
                let value = self.get(descriptor, text::VALUE)?;
                let shape = self.objects[object as usize].shape;
                if let Some((slot, _)) = self.shapes.lookup(shape, key) {
                    let old = self.objects[object as usize].slots()[slot as usize];
                    if !self.same_value(value, old) {
                        return refuse(self);
                    }
                }
            }
        }
        Ok(())
    }

    fn define_array_length(&mut self, object: u32, descriptor: Value) -> Run<()> {
        let d = descriptor.handle();
        let is_read_only = self.objects[object as usize].flags & READ_ONLY_LENGTH != 0;
        for (name, allowed) in [(text::CONFIGURABLE, false), (text::ENUMERABLE, false)] {
            if self.has_property(d, name) {
                let value = self.get(descriptor, name)?;
                if self.to_boolean(value) != allowed {
                    return Err(self.type_error("Cannot redefine property: length"));
                }
            }
        }
        if self.has_property(d, text::GET) || self.has_property(d, text::SET) {
            return Err(self.type_error("Cannot redefine property: length"));
        }
        if self.has_property(d, text::VALUE) {
            let value = self.get(descriptor, text::VALUE)?;
            let number = self.to_number(value)?;
            if number < 0.0 || number.fract() != 0.0 || number > f64::from(u32::MAX) {
                return Err(self.range_error("Invalid array length"));
            }
            let current = self.length_of(Value::object(object))?;
            if is_read_only && number as usize != current {
                return Err(self.type_error("Cannot redefine property: length"));
            }
            if !is_read_only {
                self.set_on(object, text::LENGTH, Value::number(number))
                    .inspect_err(|_| {
                        self.write_refused = false;
                    })?;
            }
        }
        if self.has_property(d, text::WRITABLE) {
            let value = self.get(descriptor, text::WRITABLE)?;
            if self.to_boolean(value) {
                if is_read_only {
                    return Err(self.type_error("Cannot redefine property: length"));
                }
            } else {
                self.objects[object as usize].flags |= READ_ONLY_LENGTH;
            }
        }
        Ok(())
    }

    fn set_integrity(&mut self, object: u32, frozen: bool) {
        // A namespace is sealed already, and can't be frozen: see `object_freeze`.
        if self.namespace_target(Value::object(object)).is_some() {
            return;
        }
        if let Some((target, _)) = self.proxy_of(object) {
            self.set_integrity(target.handle(), frozen);
            return;
        }
        self.materialize_if_lazy(object);
        // Sealing an array's elements is about as rare as freezing them's common, so it's treated the same.
        if !frozen && matches!(self.objects[object as usize].kind, Kind::Array(_)) {
            let found = &mut self.objects[object as usize];
            found.flags |= NOT_EXTENSIBLE;
            return;
        }
        let found = &mut self.objects[object as usize];
        found.flags |= NOT_EXTENSIBLE;
        if frozen && matches!(found.kind, Kind::Array(_) | Kind::Arguments(_)) {
            found.flags |= FROZEN_ELEMENTS;
        }
        if found.flags & IS_PROTO != 0 {
            self.epoch = self.epoch.wrapping_add(1);
            #[cfg(feature = "counts")]
            crate::counts::epoch("a prototype frozen or sealed", u32::MAX);
        }
        let found = &mut self.objects[object as usize];
        found.shape = self.shapes.with_flags(found.shape, |_, flags| {
            if frozen && flags & ACCESSOR == 0 {
                flags & !(CONFIGURABLE | WRITABLE)
            } else {
                flags & !CONFIGURABLE
            }
        });
    }

    fn check_integrity(&mut self, object: u32, frozen: bool) -> bool {
        // Each of a namespace's names can't be changed to something else, but is written to as its module runs.
        if let Some(target) = self.namespace_target(Value::object(object)) {
            return !frozen || self.namespace_is_empty(target);
        }
        if let Some((target, _)) = self.proxy_of(object) {
            return self.check_integrity(target.handle(), frozen);
        }
        self.materialize_if_lazy(object);
        let found = &self.objects[object as usize];
        if found.flags & NOT_EXTENSIBLE == 0 {
            return false;
        }
        if let Kind::Array(items) | Kind::Arguments(items) = &found.kind {
            if !items.is_empty() && found.flags & FROZEN_ELEMENTS == 0 {
                return false;
            }
        }
        self.shapes
            .entries(found.shape)
            .iter()
            .all(|(_, _, flags)| {
                flags & CONFIGURABLE == 0
                    && (!frozen || flags & ACCESSOR != 0 || flags & WRITABLE == 0)
            })
    }
}

fn object_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    if args.new_target.is_object() && args.new_target.handle() != args.callee {
        let proto = vm.proto_for(&args, vm.realm.object_proto)?;
        return Ok(vm.new_object_with_proto(proto, Kind::Plain));
    }
    if value.is_nullish() {
        return Ok(vm.new_plain());
    }
    vm.to_object(value)
}

fn has_own_property(vm: &mut Vm, args: Args) -> Run<Value> {
    let key = vm.arg(&args, 0);
    let atom = vm.to_atom(key)?;
    let object = vm.to_object(args.this)?;
    Ok(Value::bool(vm.has_own(object.handle(), atom)))
}

fn is_prototype_of(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    if !value.is_object() || !args.this.is_object() {
        return Ok(Value::FALSE);
    }
    let mut at = vm.proto_of(value.handle());
    while at != NONE {
        if at == args.this.handle() {
            return Ok(Value::TRUE);
        }
        at = vm.proto_of(at);
    }
    Ok(Value::FALSE)
}

fn property_is_enumerable(vm: &mut Vm, args: Args) -> Run<Value> {
    let key = vm.arg(&args, 0);
    let atom = vm.to_atom(key)?;
    let object = vm.to_object(args.this)?;
    let found = vm
        .own_keys(object.handle())
        .iter()
        .any(|(found, flags)| *found == atom && flags & ENUMERABLE != 0);
    Ok(Value::bool(found))
}

fn object_to_string(vm: &mut Vm, args: Args) -> Run<Value> {
    if args.this.is_undefined() {
        return Ok(vm.name_string(text::TAG_UNDEFINED));
    }
    if args.this.is_null() {
        return Ok(vm.name_string(text::TAG_NULL));
    }
    // A string, number or boolean is only made an object to read its tag, which is on its prototype. When that's a
    // plain value or isn't there, the object wouldn't change anything, so it isn't made. A getter would be called with
    // the object, so then it is.
    let primitive_tag = if args.this.is_object() {
        None
    } else if args.this.is_string() {
        Some((vm.realm.string_proto, text::TAG_STRING))
    } else if args.this.is_number() {
        Some((vm.realm.number_proto, text::TAG_NUMBER))
    } else if args.this.is_bool() {
        Some((vm.realm.boolean_proto, text::TAG_BOOLEAN))
    } else {
        None
    };
    if let Some((proto, builtin)) = primitive_tag
        && let Some(tag) = vm.peek_property(proto, text::SYMBOL_TO_STRING_TAG)
    {
        if tag.is_string() {
            let name = vm.text_of(tag).to_rust();
            return Ok(vm.new_str(&format!("[object {name}]")));
        }
        return Ok(vm.name_string(builtin));
    }
    let object = vm.to_object(args.this)?;
    let tag = vm.get(object, text::SYMBOL_TO_STRING_TAG)?;
    if tag.is_string() {
        let name = vm.text_of(tag).to_rust();
        return Ok(vm.new_str(&format!("[object {name}]")));
    }
    // What's given for each built-in kind is always the same, so it's the same string each time.
    let builtin = match vm.class_name(object.handle()) {
        "Object" => text::TAG_OBJECT,
        "Array" => text::TAG_ARRAY,
        "Function" => text::TAG_FUNCTION,
        "Error" => text::TAG_ERROR,
        "String" => text::TAG_STRING,
        "Number" => text::TAG_NUMBER,
        "Boolean" => text::TAG_BOOLEAN,
        "Date" => text::TAG_DATE,
        "RegExp" => text::TAG_REGEXP,
        "Arguments" => text::TAG_ARGUMENTS,
        name => return Ok(vm.new_str(&format!("[object {name}]"))),
    };
    Ok(vm.name_string(builtin))
}

fn object_value_of(vm: &mut Vm, args: Args) -> Run<Value> {
    vm.to_object(args.this)
}

fn get_proto_accessor(vm: &mut Vm, args: Args) -> Run<Value> {
    let object = vm.to_object(args.this)?;
    let proto = vm.proto_of(object.handle());
    Ok(if proto == NONE {
        Value::NULL
    } else {
        Value::object(proto)
    })
}

fn set_proto_accessor(vm: &mut Vm, args: Args) -> Run<Value> {
    let proto = vm.arg(&args, 0);
    if args.this.is_nullish() {
        return Err(vm.type_error("Object.prototype.__proto__ called on null or undefined"));
    }
    if args.this.is_object() && (proto.is_object() || proto.is_null()) {
        let new_proto = if proto.is_null() {
            NONE
        } else {
            proto.handle()
        };
        if !vm.change_proto(args.this.handle(), new_proto) {
            return Err(vm.type_error("Cannot set this prototype"));
        }
    }
    Ok(Value::UNDEFINED)
}

fn object_keys(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let object = vm.to_object(value)?;
    if let Some(keys) = vm.plain_object_keys(object.handle(), false) {
        return Ok(vm.new_array(keys));
    }
    let names = vm.own_enumerable_names(object.handle());
    let items = names.into_iter().map(|atom| vm.atom_value(atom)).collect();
    if let Some(error) = vm.deferred_error.take() {
        return Err(error);
    }
    Ok(vm.new_array(items))
}

fn object_values(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let object = vm.to_object(value)?;
    let names = vm.own_enumerable_names(object.handle());
    let mut items = Vec::with_capacity(names.len());
    for atom in names {
        let item = vm.get(object, atom)?;
        vm.nursery.push(item);
        items.push(item);
    }
    Ok(vm.new_array(items))
}

fn object_entries(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let object = vm.to_object(value)?;
    let names = vm.own_enumerable_names(object.handle());
    let mut items = Vec::with_capacity(names.len());
    for atom in names {
        let item = vm.get(object, atom)?;
        let key = vm.atom_value(atom);
        items.push(vm.new_array(vec![key, item]));
    }
    Ok(vm.new_array(items))
}

fn object_assign(vm: &mut Vm, args: Args) -> Run<Value> {
    let target = vm.arg(&args, 0);
    let target = vm.to_object(target)?;
    for index in 1..args.count {
        let source = vm.arg(&args, index);
        if source.is_nullish() {
            continue;
        }
        let source = vm.to_object(source)?;
        // An ordinary object with only plain properties can't change what it has while this is going through it.
        let found = &vm.objects[source.index()];
        if matches!(found.kind, Kind::Plain)
            && found.flags & LAZY_FUNCTION == 0
            && vm.shapes.is_all_plain(found.shape)
            && !vm.shapes.is_dict(found.shape)
        {
            let shape = found.shape;
            // The list of what's in it is made in one kept for the purpose, as making one each time was much of the
            // time this took. A setter that's called can come back here, and makes a list of its own then.
            let mut entries = std::mem::take(&mut vm.assign_entries);
            vm.shapes.entries_into(shape, &mut entries);
            if entries.iter().all(|(key, _, _)| {
                vm.atoms.index_of(*key).is_none()
                    && vm.atoms.kind_of(*key) == crate::text::AtomKind::Name
            }) {
                let mut done = Ok(());
                for &(key, slot, _) in &entries {
                    let now = vm.objects[source.index()].shape;
                    let value = if now == shape {
                        vm.objects[source.index()].slots()[slot as usize]
                    } else {
                        // A setter on the target has been changing the source.
                        match vm.shapes.lookup(now, key) {
                            Some((_, flags)) if flags & ENUMERABLE != 0 => match vm.get(source, key) {
                                Ok(value) => value,
                                Err(error) => {
                                    done = Err(error);
                                    break;
                                }
                            },
                            _ => continue,
                        }
                    };
                    if let Err(error) = vm.set_remembered(target.handle(), key, value) {
                        done = Err(error);
                        break;
                    }
                }
                entries.clear();
                if entries.capacity() <= 64 {
                    vm.assign_entries = entries;
                }
                done?;
                continue;
            }
            entries.clear();
            vm.assign_entries = entries;
        }
        for (key, flags) in vm.own_keys(source.handle()) {
            if flags & ENUMERABLE != 0 {
                let value = vm.get(source, key)?;
                vm.set(target, key, value)?;
            }
        }
    }
    Ok(target)
}

fn object_freeze(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    if let Some(target) = vm.namespace_target(value) {
        if !vm.namespace_is_empty(target) {
            return Err(vm.type_error("Cannot freeze a module's namespace, as its names can change"));
        }
    }
    if value.is_object() {
        vm.set_integrity(value.handle(), true);
    }
    Ok(value)
}

fn object_is_frozen(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    Ok(Value::bool(
        !value.is_object() || vm.check_integrity(value.handle(), true),
    ))
}

fn object_seal(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    if value.is_object() {
        vm.set_integrity(value.handle(), false);
    }
    Ok(value)
}

fn object_is_sealed(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    Ok(Value::bool(
        !value.is_object() || vm.check_integrity(value.handle(), false),
    ))
}

fn object_prevent_extensions(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    if value.is_object() {
        vm.objects[value.index()].flags |= NOT_EXTENSIBLE;
    }
    Ok(value)
}

fn object_is_extensible(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    Ok(Value::bool(
        value.is_object() && vm.objects[value.index()].flags & NOT_EXTENSIBLE == 0,
    ))
}

fn object_create(vm: &mut Vm, args: Args) -> Run<Value> {
    let proto = vm.arg(&args, 0);
    if !proto.is_object() && !proto.is_null() {
        let shown = vm.brief(proto);
        return Err(vm.type_error(&format!(
            "Object prototype may only be an Object or null: {shown}"
        )));
    }
    let handle = if proto.is_null() {
        NONE
    } else {
        proto.handle()
    };
    let object = vm.new_object_with_proto(handle, Kind::Plain);
    let properties = vm.arg(&args, 1);
    if !properties.is_undefined() {
        define_properties(vm, object, properties)?;
    }
    Ok(object)
}

fn object_get_prototype_of(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let object = vm.to_object(value)?;
    vm.prototype_checked(object)
}

fn object_set_prototype_of(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let proto = vm.arg(&args, 1);
    if !proto.is_object() && !proto.is_null() {
        return Err(vm.type_error("Object prototype may only be an Object or null"));
    }
    if value.is_object() {
        let handle = if proto.is_null() {
            NONE
        } else {
            proto.handle()
        };
        if !vm.change_proto(value.handle(), handle) {
            return Err(vm.type_error("Cannot set this prototype"));
        }
    }
    if value.is_nullish() {
        return Err(vm.type_error("Object.setPrototypeOf called on null or undefined"));
    }
    Ok(value)
}

fn object_define_property(vm: &mut Vm, args: Args) -> Run<Value> {
    let object = vm.arg(&args, 0);
    if !object.is_object() {
        return Err(vm.type_error("Object.defineProperty called on non-object"));
    }
    let key = vm.arg(&args, 1);
    let atom = vm.to_atom(key)?;
    let descriptor = vm.arg(&args, 2);
    vm.define_from_descriptor(object.handle(), atom, descriptor)?;
    Ok(object)
}

fn define_properties(vm: &mut Vm, object: Value, properties: Value) -> Run<()> {
    let properties = vm.to_object(properties)?;
    for (key, flags) in vm.own_keys(properties.handle()) {
        if flags & ENUMERABLE != 0 {
            let descriptor = vm.get(properties, key)?;
            vm.define_from_descriptor(object.handle(), key, descriptor)?;
        }
    }
    Ok(())
}

fn object_define_properties(vm: &mut Vm, args: Args) -> Run<Value> {
    let object = vm.arg(&args, 0);
    if !object.is_object() {
        return Err(vm.type_error("Object.defineProperties called on non-object"));
    }
    let properties = vm.arg(&args, 1);
    define_properties(vm, object, properties)?;
    Ok(object)
}

fn object_get_own_descriptor(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let object = vm.to_object(value)?;
    let key = vm.arg(&args, 1);
    let atom = vm.to_atom(key)?;
    vm.descriptor_checked(object.handle(), atom)
}

fn object_get_own_descriptors(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let object = vm.to_object(value)?;
    let result = vm.new_plain();
    for (key, _) in vm.own_keys_checked(object.handle())? {
        let descriptor = vm.descriptor_checked(object.handle(), key)?;
        vm.define(result.handle(), key, descriptor, PLAIN);
    }
    Ok(result)
}

fn object_get_own_names(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let object = vm.to_object(value)?;
    let keys = vm.own_keys_checked(object.handle())?;
    let mut items = Vec::with_capacity(keys.len());
    for (key, _) in keys {
        if vm.atoms.kind_of(key) == AtomKind::Name {
            items.push(vm.atom_value(key));
        }
    }
    Ok(vm.new_array(items))
}

fn object_get_own_symbols(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let object = vm.to_object(value)?;
    let keys = vm.own_keys(object.handle());
    let items = keys
        .into_iter()
        .filter(|(key, _)| vm.atoms.kind_of(*key) == AtomKind::Symbol)
        .map(|(key, _)| Value::symbol(key.0))
        .collect();
    Ok(vm.new_array(items))
}

fn object_from_entries(vm: &mut Vm, args: Args) -> Run<Value> {
    let source = vm.arg(&args, 0);
    if source.is_nullish() {
        return Err(vm.type_error("Object.fromEntries needs something to go through"));
    }
    let result = vm.new_plain();
    vm.iterate(source, |vm, item| {
        if !item.is_object() {
            let shown = vm.brief(item);
            return Err(vm.type_error(&format!("Iterator value {shown} is not an entry object")));
        }
        let key = vm.get_index(item, 0)?;
        let value = vm.get_index(item, 1)?;
        let atom = vm.to_atom(key)?;
        vm.define(result.handle(), atom, value, PLAIN);
        Ok(())
    })?;
    Ok(result)
}

fn object_is(vm: &mut Vm, args: Args) -> Run<Value> {
    let (a, b) = (vm.arg(&args, 0), vm.arg(&args, 1));
    Ok(Value::bool(vm.same_value(a, b)))
}

fn object_has_own(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    let object = vm.to_object(value)?;
    let key = vm.arg(&args, 1);
    let atom = vm.to_atom(key)?;
    Ok(Value::bool(vm.has_own(object.handle(), atom)))
}

fn object_group_by(vm: &mut Vm, args: Args) -> Run<Value> {
    let source = vm.arg(&args, 0);
    let callback = vm.arg(&args, 1);
    let items = vm.iterate_to_vec(source)?;
    let result = vm.new_object_with_proto(NONE, Kind::Plain);
    for (index, item) in items.into_iter().enumerate() {
        vm.nursery.push(item);
        let key = vm.call(
            callback,
            Value::UNDEFINED,
            &[item, Value::number(index as f64)],
        )?;
        let atom = vm.to_atom(key)?;
        let list = vm.get(result, atom)?;
        if list.is_undefined() {
            let list = vm.new_array(vec![item]);
            vm.define(result.handle(), atom, list, PLAIN);
        } else if let Kind::Array(found) = &mut vm.objects[list.index()].kind {
            found.push(item);
        }
    }
    Ok(result)
}

pub(super) fn function_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    let keyword = match &vm.objects[args.callee as usize].kind {
        Kind::Native(native) => match vm.atom_text(native.name).as_str() {
            "GeneratorFunction" => "function*",
            "AsyncFunction" => "async function",
            "AsyncGeneratorFunction" => "async function*",
            _ => "function",
        },
        _ => "function",
    };
    let mut parts = Vec::with_capacity(args.count);
    for index in 0..args.count {
        let part = vm.arg(&args, index);
        parts.push(vm.to_rust_string(part)?);
    }
    let body = parts.pop().unwrap_or_default();
    // The two parts have to make sense without each other, so they can't close what's in the other.
    let params = parts.join(",");
    let file = vm.intern_str("anonymous");
    let check_params = format!("return ({keyword} anonymous({params}\n) {{\n}})");
    let check_body = format!("return ({keyword} anonymous(\n) {{\n{body}\n}})");
    vm.compile_source(&check_params, file)?;
    vm.compile_source(&check_body, file)?;
    let source = format!("return ({keyword} anonymous({params}\n) {{\n{body}\n}})");
    let proto = vm.compile_source(&source, file)?;
    vm.run_proto(proto, &[])
}

fn function_call(vm: &mut Vm, args: Args) -> Run<Value> {
    let this = vm.arg(&args, 0);
    let count = args.count.saturating_sub(1);
    // A native is called with the same arguments where they are, one further on, as `hasOwnProperty.call(o, key)` and
    // `toString.call(value)` are, which is what library code does for a check: no list made, and no call from Rust.
    if args.this.is_object()
        && let Kind::Native(native) = &vm.objects[args.this.index()].kind
        && native.forwards == 0
    {
        let func = native.func;
        let inner = Args {
            this,
            start: args.start + 1,
            count,
            new_target: Value::UNDEFINED,
            callee: args.this.handle(),
        };
        return func(vm, inner);
    }
    if count <= 8 {
        let mut rest = [Value::UNDEFINED; 8];
        for (index, place) in rest[..count].iter_mut().enumerate() {
            *place = vm.arg(&args, index + 1);
        }
        return vm.call(args.this, this, &rest[..count]);
    }
    let rest: Vec<Value> = (1..args.count).map(|index| vm.arg(&args, index)).collect();
    vm.call(args.this, this, &rest)
}

fn function_apply(vm: &mut Vm, args: Args) -> Run<Value> {
    let this = vm.arg(&args, 0);
    let list = vm.arg(&args, 1);
    let items = if list.is_nullish() {
        Vec::new()
    } else {
        let length = vm.length_of(list)?;
        let mut items = Vec::with_capacity(length);
        for index in 0..length {
            items.push(vm.get_index(list, index)?);
        }
        items
    };
    vm.call(args.this, this, &items)
}

fn function_bind(vm: &mut Vm, args: Args) -> Run<Value> {
    if !vm.is_callable(args.this) {
        return Err(vm.type_error("Bind must be called on a function"));
    }
    let this = vm.arg(&args, 0);
    let rest: Box<[Value]> = (1..args.count).map(|index| vm.arg(&args, index)).collect();
    let proto = vm.proto_of(args.this.handle());
    let bound_count = rest.len();
    let bound = vm.new_object_with_proto(
        proto,
        Kind::Bound(Box::new(BoundData {
            target: args.this,
            this,
            args: rest,
        })),
    );
    let name = vm.get(args.this, text::NAME)?;
    let name = if name.is_string() {
        vm.text_of(name).to_rust()
    } else {
        String::new()
    };
    let name = vm.new_str(&format!("bound {name}"));
    vm.define(bound.handle(), text::NAME, name, CONFIGURABLE);
    let length = vm.get(args.this, text::LENGTH)?;
    let length = if length.is_number() {
        (length.as_number() - bound_count as f64).max(0.0)
    } else {
        0.0
    };
    vm.define(
        bound.handle(),
        text::LENGTH,
        Value::number(length),
        CONFIGURABLE,
    );
    Ok(bound)
}

impl Vm {
    /// What compiled code calls to name a function from a key it only has when it runs, as a computed one is.
    pub(crate) fn name_function_native(&mut self) -> Value {
        if self.realm.name_function.0.is_undefined() {
            let atom = self.intern_str("SetFunctionName");
            let function = self.new_native(atom, 3, name_function, Value::UNDEFINED, false);
            self.realm.name_function = crate::vm::Value0(function);
        }
        self.realm.name_function.0
    }
}

/// `(function, key, prefix)`: names `function` from `key`, a string or a symbol, as the language does for a function
/// defined under a computed key, with `get ` before it if `prefix` is 1 and `set ` if it's 2. A class that has said what
/// its name is, with a static method or field called `name`, keeps that.
fn name_function(vm: &mut Vm, args: Args) -> Run<Value> {
    let function = vm.arg(&args, 0);
    let key = vm.arg(&args, 1);
    let prefix = vm.arg(&args, 2);
    if !function.is_object() {
        return Ok(Value::UNDEFINED);
    }
    let handle = function.handle();
    vm.materialize_if_lazy(handle);
    let shape = vm.objects[handle as usize].shape;
    if let Some((slot, flags)) = vm.shapes.lookup(shape, text::NAME) {
        let current = vm.objects[handle as usize].slots()[slot as usize];
        let is_default = flags & crate::object::ACCESSOR == 0
            && current.is_string()
            && vm.text_of(current).to_rust().is_empty();
        if !is_default {
            return Ok(Value::UNDEFINED);
        }
    }
    let atom = vm.to_atom(key)?;
    let mut name = match vm.atoms.kind_of(atom) {
        crate::text::AtomKind::Symbol => match vm.atoms.string_of(atom) {
            Some(description) => {
                let description = vm.text_of(Value::string(description)).to_rust();
                format!("[{description}]")
            }
            None => String::new(),
        },
        _ => vm.atom_text(atom),
    };
    if prefix.is_int() && prefix.as_int() == 1 {
        name.insert_str(0, "get ");
    } else if prefix.is_int() && prefix.as_int() == 2 {
        name.insert_str(0, "set ");
    }
    let name = vm.new_str(&name);
    vm.define(handle, text::NAME, name, CONFIGURABLE);
    Ok(Value::UNDEFINED)
}

fn function_to_string(vm: &mut Vm, args: Args) -> Run<Value> {
    if !vm.is_callable(args.this) {
        return Err(vm.type_error("Function.prototype.toString requires that 'this' be a Function"));
    }
    // A function that was written gives back exactly what was written, from its first token to its last.
    if let Kind::Function { proto, .. } = vm.objects[args.this.index()].kind {
        if let Some(written) = vm.function_text(proto) {
            return Ok(vm.new_text(written));
        }
    }
    // A bound function has no name of its own here, as in V8.
    if let Kind::Bound(_) = vm.objects[args.this.index()].kind {
        return Ok(vm.new_str("function () { [native code] }"));
    }
    let name = vm.get(args.this, text::NAME)?;
    let name = if name.is_string() {
        vm.text_of(name).to_rust()
    } else {
        String::new()
    };
    // What's private isn't a name this form can have.
    let name = if name.starts_with('#') { "" } else { &name };
    Ok(vm.new_str(&format!("function {name}() {{ [native code] }}")))
}

fn function_has_instance(vm: &mut Vm, args: Args) -> Run<Value> {
    let value = vm.arg(&args, 0);
    if !vm.is_callable(args.this) {
        return Ok(Value::FALSE);
    }
    Ok(Value::bool(vm.ordinary_has_instance(value, args.this)?))
}

fn symbol_constructor(vm: &mut Vm, args: Args) -> Run<Value> {
    if args.new_target.is_object() {
        return Err(vm.type_error("Symbol is not a constructor"));
    }
    let description = vm.arg(&args, 0);
    let handle = if description.is_undefined() {
        None
    } else {
        Some(vm.to_js_string(description)?.handle())
    };
    let atom = vm.atoms.new_symbol(handle, AtomKind::Symbol);
    Ok(Value::symbol(atom.0))
}

fn symbol_for(vm: &mut Vm, args: Args) -> Run<Value> {
    let key = vm.arg(&args, 0);
    let key = vm.to_rust_string(key)?;
    if let Some(found) = vm.symbol_registry.get(&key) {
        return Ok(Value::symbol(found.0));
    }
    let handle = vm.strings.add(Text::from_str(&key));
    let atom = vm.atoms.new_symbol(Some(handle), AtomKind::Symbol);
    // What's in the registry can be asked for again by its key at any time, so it stays.
    vm.atoms.pin(atom);
    vm.symbol_registry.insert(key, atom);
    Ok(Value::symbol(atom.0))
}

fn symbol_key_for(vm: &mut Vm, args: Args) -> Run<Value> {
    let symbol = vm.arg(&args, 0);
    if !symbol.is_symbol() {
        return Err(vm.type_error("Symbol.keyFor needs a symbol"));
    }
    let found = vm
        .symbol_registry
        .iter()
        .find(|(_, atom)| atom.0 == symbol.handle())
        .map(|(key, _)| key.clone());
    Ok(match found {
        Some(key) => vm.new_str(&key),
        None => Value::UNDEFINED,
    })
}

fn this_symbol(vm: &mut Vm, this: Value) -> Run<Value> {
    if this.is_symbol() {
        return Ok(this);
    }
    if this.is_object() {
        if let Kind::Boxed(value) = vm.objects[this.index()].kind {
            if value.is_symbol() {
                return Ok(value);
            }
        }
    }
    Err(vm.type_error("This isn't a symbol"))
}

fn symbol_to_string(vm: &mut Vm, args: Args) -> Run<Value> {
    let symbol = this_symbol(vm, args.this)?;
    let text = vm.atom_text(Atom(symbol.handle()));
    Ok(vm.new_str(&format!("Symbol({text})")))
}

fn symbol_value_of(vm: &mut Vm, args: Args) -> Run<Value> {
    this_symbol(vm, args.this)
}

fn symbol_description(vm: &mut Vm, args: Args) -> Run<Value> {
    let symbol = this_symbol(vm, args.this)?;
    Ok(match vm.atoms.string_of(Atom(symbol.handle())) {
        Some(handle) => Value::string(handle),
        None => Value::UNDEFINED,
    })
}
